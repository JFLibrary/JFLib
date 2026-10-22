// coretest -- self-checks of the foundation crates (datetime, crypto, compress, datastore,
// cas, netio incl. TLS and SSH, netpkt incl. DHCP, DNS, NTP, syslog and RIP, base::rng,
// base::metrics, base::cmdtree, base::hashring, base::lru, agent_core::jmap, script, its VM and
// its standard library, the BPDUs of netpkt::stp): known-answer vectors, round trips, edge
// cases.  Console program; exit code 1 when a check fails.
//
//   python build_rs.py app coretest
//   build/win/coretest.exe [--only NAME] [--bench] [--vectors FILE]  (FILE: tools/crypto_check.py)
//   build/win/coretest.exe --math FILE  (FILE: tools/script_math_check.py)
//   (build_rs.py test runs it through tests/all.uis; --script is ignored)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

mod cast;
mod crypto;
mod dhcpt;
mod dnsmsgt;
mod dnst;
mod dt;
mod httpt;
mod jmapt;
mod lz;
mod metricst;
mod netpktt;
mod ntpt;
mod platx;
mod rngt;
mod scriptt;
mod sshd;
mod store;
mod tlst;
mod cmdtreet;
mod scriptvmt;
mod httpmsgt;
mod scriptlibt;
mod hashringt;
mod lrut;
mod stpt;
mod ript;

/// the result collector of a check group
pub struct T {
    pub group: &'static str,
    pub ok: u32,
    pub failed: u32,
}

impl T {
    pub fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}\n", self.group, name).as_bytes());
        }
    }
    pub fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
}

fn run() -> i32 {
    let args = plat::args();
    let only = plat::arg_value("--only").map(String::from);
    if let Some(f) = plat::arg_value("--vectors") {
        return crypto::vectors(&f);
    }
    if let Some(f) = plat::arg_value("--math") {
        return scriptlibt::math_file(&f);
    }
    if args.iter().any(|a| a == "--bench") {
        crypto::bench();
        return 0;
    }
    if let Some(op) = plat::arg_value("--lz4") {
        let rest: alloc::vec::Vec<String> = args.iter().skip_while(|a| a.as_str() != "--lz4").skip(2).cloned().collect();
        return match rest.as_slice() {
            [i, o, ..] => lz::file_mode(&op, i, o),
            _ => 2,
        };
    }
    if let Some(op) = plat::arg_value("--zstd") {
        let rest: alloc::vec::Vec<String> = args.iter().skip_while(|a| a.as_str() != "--zstd").skip(2).cloned().collect();
        return match rest.as_slice() {
            [i, o, l, ..] => lz::zstd_file_mode(&op, i, o, l.parse().unwrap_or(3)),
            [i, o] => lz::zstd_file_mode(&op, i, o, 3),
            _ => 2,
        };
    }
    if let Some(d) = plat::arg_value("--tls-pki") {
        return tlst::write_pki(&d);
    }
    if let Some(p) = plat::arg_value("--tls-serve").and_then(|p| p.parse().ok()) {
        return tlst::serve(p, args.iter().any(|a| a == "--mtls"));
    }
    if let Some(p) = plat::arg_value("--tls-get").and_then(|p| p.parse().ok()) {
        return tlst::get(&plat::arg_value("--host").unwrap_or_else(|| String::from("localhost")), p, args.iter().any(|a| a == "--mtls"));
    }
    if let Some(u) = plat::arg_value("--tls-web") {
        return tlst::web(&u, plat::arg_value("--ca").as_deref());
    }
    if let Some(p) = plat::arg_value("--ssh-serve").and_then(|p| p.parse().ok()) {
        return sshd::serve(p);
    }
    if let Some(f) = plat::arg_value("--ssh-key") {
        return sshd::key_line(&f);
    }
    let groups: [(&'static str, fn(&mut T)); 26] = [
        ("datetime", dt::run),
        ("crypto", crypto::run),
        ("compress", lz::run),
        ("plat", platx::run),
        ("datastore", store::run),
        ("http", httpt::run),
        ("cas", cast::run),
        ("tls", tlst::run),
        ("ssh", sshd::run),
        ("dns", dnst::run),
        ("netpkt", netpktt::run),
        ("rng", rngt::run),
        ("jmap", jmapt::run),
        ("script", scriptt::run),
        ("dhcp", dhcpt::run),
        ("dnsmsg", dnsmsgt::run),
        ("ntp", ntpt::run),
        ("metrics", metricst::run),
        ("cmdtree", cmdtreet::run),
        ("scriptvm", scriptvmt::run),
        ("httpmsg", httpmsgt::run),
        ("scriptlib", scriptlibt::run),
        ("hashring", hashringt::run),
        ("lru", lrut::run),
        ("stp", stpt::run),
        ("rip", ript::run),
    ];
    let mut failed = 0;
    let mut total = 0;
    for (name, f) in groups {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed).as_bytes());
        failed += t.failed;
        total += t.ok + t.failed;
    }
    plat::write_stdout(format!("coretest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
