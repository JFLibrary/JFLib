// coretest -- self-checks of the foundation crates (datetime, crypto, compress, datastore,
// cas, netio incl. TLS and SSH, netpkt incl. DHCP, DNS, NTP, syslog, RIP and OSPF, base::rng,
// base::metrics, base::cmdtree, base::hashring, base::lru, agent_core::jmap, the BPDUs of netpkt::stp, ICMPv6 and Neighbor Discovery of
// netpkt::icmpv6, the address selection of netpkt::addrsel, the BGP messages of netpkt::bgp,
// TFTP and FTP of netpkt::tftp and netpkt::ftp, BER, SNMP and the MIB of netpkt::ber, snmp and
// mib, RADIUS and Telnet of netpkt::radius and netpkt::telnet): known-answer vectors, round
// trips, edge cases.
// The group mail checks netpkt::smtp, netpkt::pop3 and netpkt::imf (mailt.rs).
// The group sshcore checks netpkt::ssh and netpkt::snmpv3 (sshcoret.rs, snmpv3t.rs).
// The group eap checks netpkt::eap, the EAP parts of netpkt::eapol and netpkt::radius, crypto::eappwd
// and the crate eapm (eapt.rs), and netpkt::imap (imapt.rs).
// The group cookie checks netpkt::cookie (cookiet.rs), the group tlscore netpkt::tls and netpkt::x509
// against netio::tls (tlscoret.rs).
// The group pattern checks base::pattern: wildcards and regular expressions (patternt.rs).
// JFScript's checks (script, its VM, standard library, language services, modules) are its program
// scripttest (https://github.com/JFLibrary/JFScript).
// Console program; exit code 1 when a check fails.
//
//   python build_rs.py app coretest
//   build/win/coretest.exe [--only NAME] [--bench] [--vectors FILE]  (FILE: tools/crypto_check.py)
//   (build_rs.py test runs it through tests/all.uis; --script is ignored)
//   Android (docs/ANDROID.md, AN0): a program without a UI, python tools/android_console.py coretest;
//   the groups and parts that need what plat lacks there so far (fs, net, sync, threads: AN1 to
//   AN3) say "not on Android yet" instead
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

#[cfg(not(target_os = "android"))]
mod cast;
mod crypto;
mod dhcpt;
mod dnsmsgt;
#[cfg(not(target_os = "android"))]
mod dnst;
mod dt;
#[cfg(not(target_os = "android"))]
mod httpt;
mod jmapt;
mod lz;
mod metricst;
mod netpktt;
mod ntpt;
#[cfg(not(target_os = "android"))]
mod platx;
mod rngt;
#[cfg(not(target_os = "android"))]
mod sshd;
#[cfg(not(target_os = "android"))]
mod store;
#[cfg(not(target_os = "android"))]
mod tlst;
mod sshcoret;
mod cookiet;
#[cfg(not(target_os = "android"))]
mod tlscoret;
mod snmpv3t;
mod cmdtreet;
mod httpmsgt;
mod hashringt;
mod lrut;
mod stpt;
mod ript;
mod icmpv6t;
mod ospft;
mod addrselt;
mod bgpt;
mod tftpt;
mod ftpt;
mod mailt;
mod snmpt;
mod radiust;
mod patternt;
mod eapt;
mod imapt;

/// a group that needs what plat lacks on Android so far: fs, net, sync, threads (docs/ANDROID.md,
/// AN1 to AN3) -> (name, None) there
#[cfg(not(target_os = "android"))]
macro_rules! native {
    ($name:literal, $f:path) => {
        ($name, Some($f as fn(&mut T)))
    };
}
#[cfg(target_os = "android")]
macro_rules! native {
    ($name:literal, $f:path) => {
        ($name, None)
    };
}

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
    }    /// (Android) a part of the group that needs what plat lacks there so far (docs/ANDROID.md)
    #[cfg(target_os = "android")]
    pub fn later(&self, what: &str) {
        plat::write_stdout(format!("SKIP {}: {}: not on Android yet\n", self.group, what).as_bytes());
    }
}

fn run() -> i32 {
    let args = plat::args();
    let only = plat::arg_value("--only").map(String::from);
    if let Some(f) = plat::arg_value("--vectors") {
        return crypto::vectors(&f);
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
    #[cfg(not(target_os = "android"))]
    if let Some(code) = server_modes(&args) {
        return code;
    }
    let groups: &[(&'static str, Option<fn(&mut T)>)] = &[
        ("datetime", Some(dt::run)),
        ("crypto", Some(crypto::run)),
        ("compress", Some(lz::run)),
        native!("plat", platx::run),
        native!("datastore", store::run),
        native!("http", httpt::run),
        native!("cas", cast::run),
        native!("tls", tlst::run),
        native!("ssh", sshd::run),
        native!("dns", dnst::run),
        ("netpkt", Some(netpktt::run)),
        ("rng", Some(rngt::run)),
        ("jmap", Some(jmapt::run)),
        ("dhcp", Some(dhcpt::run)),
        ("dnsmsg", Some(dnsmsgt::run)),
        ("ntp", Some(ntpt::run)),
        ("metrics", Some(metricst::run)),
        ("cmdtree", Some(cmdtreet::run)),
        ("httpmsg", Some(httpmsgt::run)),
        ("hashring", Some(hashringt::run)),
        ("lru", Some(lrut::run)),
        ("stp", Some(stpt::run)),
        ("rip", Some(ript::run)),
        ("icmpv6", Some(icmpv6t::run)),
        ("ospf", Some(ospft::run)),
        ("addrsel", Some(addrselt::run)),
        ("bgp", Some(bgpt::run)),
        ("tftp", Some(tftpt::run)),
        ("ftp", Some(ftpt::run)),
        ("mail", Some(mailt::run)),
        ("snmp", Some(snmpt::run)),
        ("radius", Some(radiust::run)),
        ("pattern", Some(patternt::run)),
        ("sshcore", Some(sshcoret::run)),
        ("eap", Some(eapt::run)),
        ("cookie", Some(cookiet::run)),
        native!("tlscore", tlscoret::run),
    ];
    let mut failed = 0;
    let mut total = 0;
    for &(name, f) in groups {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let Some(f) = f else {
            plat::write_stdout(format!("{:<10} not on Android yet\n", name).as_bytes());
            continue;
        };
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed).as_bytes());
        failed += t.failed;
        total += t.ok + t.failed;
    }
    plat::write_stdout(format!("coretest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 { 0 } else { 1 }
}

/// the servers and clients of other programs' tests (tools/tls_check.py, tools/ssh_interop.py ...)
#[cfg(not(target_os = "android"))]
fn server_modes(args: &[String]) -> Option<i32> {
    if let Some(d) = plat::arg_value("--tls-pki") {
        return Some(tlst::write_pki(&d));
    }
    if let Some(p) = plat::arg_value("--tls-serve").and_then(|p| p.parse().ok()) {
        return Some(tlst::serve(p, args.iter().any(|a| a == "--mtls")));
    }
    if let Some(p) = plat::arg_value("--tls-get").and_then(|p| p.parse().ok()) {
        return Some(tlst::get(&plat::arg_value("--host").unwrap_or_else(|| String::from("localhost")), p, args.iter().any(|a| a == "--mtls")));
    }
    if let Some(u) = plat::arg_value("--tls-web") {
        return Some(tlst::web(&u, plat::arg_value("--ca").as_deref()));
    }
    if let Some(d) = plat::arg_value("--tls-core-pki") {
        return Some(tlscoret::write_pki(&d));
    }
    if let Some(p) = plat::arg_value("--tls-core-serve").and_then(|p| p.parse().ok()) {
        return Some(tlscoret::serve(p));
    }
    if let Some(p) = plat::arg_value("--tls-core-get").and_then(|p| p.parse().ok()) {
        return Some(tlscoret::get(p));
    }
    if let Some(p) = plat::arg_value("--ssh-serve").and_then(|p| p.parse().ok()) {
        return Some(sshd::serve(p));
    }
    if let Some(p) = plat::arg_value("--ssh-core-serve").and_then(|p| p.parse().ok()) {
        return Some(sshcoret::serve(p));
    }
    if let Some(f) = plat::arg_value("--ssh-key") {
        return Some(sshd::key_line(&f));
    }
    None
}

rt::main!(run());
