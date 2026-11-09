// dnst -- checks of netio::dns: hosts files, resolv.conf, the message format, and a small name
// server on loopback (UDP, truncation and TCP, search domains, unknown names); IPv6 zones
// (plat::net::ip_scope_parse, SocketAddr::scope) and the same name server on a link-local
// address of this computer, named by its zone in resolv.conf (skipped, with a note, when the
// computer has none or it can not be bound)

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use netio::dns::{self, Answer, Config};
use plat::net::{IpAddr, NetErrorKind, SocketAddr, TcpListener, UdpSocket};

use crate::T;

/// the answer of the test server to a query
fn answer(q: &[u8], tcp: bool) -> Vec<u8> {
    // the name of the question
    let mut p = 12;
    let mut labels = Vec::new();
    while p < q.len() && q[p] != 0 {
        let l = q[p] as usize;
        labels.push(String::from_utf8_lossy(&q[p + 1..p + 1 + l]).to_lowercase());
        p += 1 + l;
    }
    let name = labels.join(".");
    let qtype = u16::from_be_bytes([q[p + 1], q[p + 2]]);
    let qend = p + 5;
    let mut m = q[..qend].to_vec();
    m[2] = 0x81; // answer, recursion desired
    m[3] = 0x80;
    let rr = |m: &mut Vec<u8>, t: u16, data: &[u8]| {
        m.extend_from_slice(&[0xC0, 12]); // the question's name
        m.extend_from_slice(&t.to_be_bytes());
        m.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
        m.extend_from_slice(&(data.len() as u16).to_be_bytes());
        m.extend_from_slice(data);
    };
    let mut count = 0u16;
    match (name.as_str(), qtype) {
        ("fake.test", 1) => {
            rr(&mut m, 1, &[10, 1, 2, 3]);
            count = 1;
        }
        ("fake.test", 28) => {
            rr(&mut m, 28, &[0xfd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]);
            count = 1;
        }
        ("big.test", 1) if !tcp => m[2] |= 0x02, // truncated: ask over TCP
        ("big.test", 1) => {
            for i in 0..40u8 {
                rr(&mut m, 1, &[10, 9, 0, i]);
            }
            count = 40;
        }
        ("big.test", 28) | ("nodata.test", _) => {}
        _ => m[3] |= 3, // no such name
    }
    m[6..8].copy_from_slice(&count.to_be_bytes());
    m
}

/// a TCP listener and a UDP socket on one free port of `at` (port 0). The system's free TCP
/// port is mostly free for UDP too, but another program may hold it, and Windows hands out
/// ports in sequence and excludes whole ranges per protocol (Hyper-V, WSL), so the next ones
/// fail alike: then random ports of the dynamic range, a bounded number of times
fn bind_same_port(at: SocketAddr) -> Option<(TcpListener, UdpSocket)> {
    for i in 0..64 {
        let port = if i == 0 { 0 } else { 49152 + (plat::random_u64() % 16384) as u16 };
        let Ok(tcp) = TcpListener::bind(&SocketAddr { port, ..at }) else { continue };
        let Ok(addr) = tcp.local_addr() else { continue };
        if let Ok(udp) = UdpSocket::bind(&addr) {
            return Some((tcp, udp));
        }
    }
    None
}

/// a name server on a free loopback port (UDP and TCP on the same port)
fn server() -> Option<SocketAddr> {
    server_at(SocketAddr::new(IpAddr::LOCALHOST_V4, 0))
}

/// the name server on a free port of an address of this computer
fn server_at(at: SocketAddr) -> Option<SocketAddr> {
    let (tcp, udp) = bind_same_port(at)?;
    let addr = tcp.local_addr().ok()?;
    plat::thread_spawn(64 * 1024, Box::new(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = udp.recv_from(&mut buf, 20_000) {
            let _ = udp.send_to(&answer(&buf[..n], false), &from);
        }
    }))?;
    plat::thread_spawn(64 * 1024, Box::new(move || {
        while let Ok(Some((s, _))) = tcp.accept_timeout(20_000) {
            s.set_read_timeout(Some(2000));
            let mut len = [0u8; 2];
            if s.read_exact(&mut len).is_err() {
                continue;
            }
            let mut q = alloc::vec![0u8; u16::from_be_bytes(len) as usize];
            if s.read_exact(&mut q).is_err() {
                continue;
            }
            let a = answer(&q, true);
            let mut out = (a.len() as u16).to_be_bytes().to_vec();
            out.extend_from_slice(&a);
            let _ = s.write_all(&out);
        }
    }))?;
    Some(addr)
}

pub fn run(t: &mut T) {
    // hosts files
    let hosts = "# comment\n127.0.0.1 localhost\n127.0.1.1\tBox.Example.lan  box # this machine\n::1 ip6-localhost\nfd00::5 box\nbroken line\n";
    t.eq("hosts: case, aliases, both families", dns::hosts_lookup(hosts, "BOX"), alloc::vec![IpAddr::V4([127, 0, 1, 1]), IpAddr::parse("fd00::5").unwrap_or(IpAddr::LOCALHOST_V6)]);
    t.eq("hosts: full name", dns::hosts_lookup(hosts, "box.example.lan."), alloc::vec![IpAddr::V4([127, 0, 1, 1])]);
    t.check("hosts: unknown", dns::hosts_lookup(hosts, "other").is_empty());
    // resolv.conf
    let c = Config::from_resolv_conf("# x\nnameserver 10.0.0.53\nnameserver fe80::1%2\nsearch corp.example lab.example.\noptions edns0 ndots:2\n");
    t.eq("resolv.conf: name servers", c.nameservers.len(), 2);
    let ll = IpAddr::parse("fe80::1").unwrap_or(IpAddr::LOCALHOST_V6);
    t.eq("resolv.conf: a link-local server keeps its zone (%2)", c.nameservers.get(1).copied(), Some(SocketAddr::with_scope(ll, 53, 2)));
    t.eq("resolv.conf: a zone naming no interface: the server is left out",
        Config::from_resolv_conf("nameserver fe80::1%nosuchif9\nnameserver 10.0.0.53\n").nameservers, alloc::vec![SocketAddr::new(IpAddr::V4([10, 0, 0, 53]), 53)]);
    scopes(t, ll);
    t.eq("resolv.conf: search", c.search.clone(), alloc::vec![String::from("corp.example"), String::from("lab.example")]);
    t.eq("resolv.conf: ndots", c.ndots, 2);
    t.eq("resolv.conf: empty -> local stub", Config::from_resolv_conf("").nameservers, alloc::vec![SocketAddr::new(IpAddr::LOCALHOST_V4, 53)]);
    // the message format
    let q = dns::query(0x1234, "example.com", 1).unwrap_or_default();
    t.eq("query bytes", q.clone(), b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com\x00\x00\x01\x00\x01".to_vec());
    t.check("label too long refused", dns::query(1, &"a".repeat(64), 1).is_none());
    let qf = dns::query(7, "fake.test", 1).unwrap_or_default();
    t.eq("answer read (compressed name)", dns::parse_answer(&answer(&qf, false), &qf, 1), Answer::Found(alloc::vec![IpAddr::V4([10, 1, 2, 3])]));
    let mut wrong = answer(&qf, false);
    wrong[0] ^= 1;
    t.eq("another id", dns::parse_answer(&wrong, &qf, 1), Answer::Bad);
    let qo = dns::query(7, "fake.tesx", 1).unwrap_or_default();
    t.eq("another question", dns::parse_answer(&answer(&qf, false), &qo, 1), Answer::Bad);
    let qn = dns::query(8, "none.test", 1).unwrap_or_default();
    t.eq("no such name", dns::parse_answer(&answer(&qn, false), &qn, 1), Answer::NoName);
    t.eq("cut answer", dns::parse_answer(&answer(&qf, false)[..30], &qf, 1), Answer::Bad);

    // a name server on loopback
    let Some(ns) = server() else {
        t.check("test name server", false);
        return;
    };
    let cfg = Config { hosts: String::from("192.168.7.7 printer.lan\n"), nameservers: alloc::vec![ns], search: alloc::vec![String::from("test")], ndots: 1 };
    let ips = |r: plat::net::NetResult<Vec<SocketAddr>>| r.map(|v| v.into_iter().map(|a| a.ip).collect::<Vec<_>>());
    t.eq("UDP: A first, then AAAA", ips(dns::resolve_with("fake.test", 443, &cfg)), Ok(alloc::vec![IpAddr::V4([10, 1, 2, 3]), IpAddr::parse("fd00::7").unwrap_or(IpAddr::LOCALHOST_V6)]));
    t.eq("port kept", dns::resolve_with("fake.test", 443, &cfg).map(|v| v[0].port), Ok(443));
    t.eq("search domain", ips(dns::resolve_with("fake", 1, &cfg)).map(|v| v.len()), Ok(2));
    t.eq("truncated -> TCP", ips(dns::resolve_with("big.test", 1, &cfg)).map(|v| v.len()), Ok(40));
    t.eq("hosts before DNS", ips(dns::resolve_with("PRINTER.lan", 1, &cfg)), Ok(alloc::vec![IpAddr::V4([192, 168, 7, 7])]));
    t.eq("unknown name", dns::resolve_with("nope.test", 1, &cfg).map_err(|e| e.kind), Err(NetErrorKind::NotFound));
    t.eq("name without addresses", dns::resolve_with("nodata.test", 1, &cfg).map_err(|e| e.kind), Err(NetErrorKind::NotFound));
    t.eq("numeric", ips(dns::resolve_with("[::1]", 1, &cfg)), Ok(alloc::vec![IpAddr::LOCALHOST_V6]));
    t.check("invalid characters refused", dns::resolve_with("a b", 1, &cfg).is_err());
    let silent = Config { nameservers: alloc::vec![SocketAddr::new(IpAddr::LOCALHOST_V4, 9)], ..cfg.clone() };
    let t0 = plat::time_us();
    t.eq("no server answers -> timeout", dns::resolve_with("fake.test", 1, &silent).map_err(|e| e.kind), Err(NetErrorKind::TimedOut));
    t.check("within the time limits", plat::time_us() - t0 < 20_000_000);
    // this machine: localhost always; its own name through the system's hosts file (Linux)
    t.check("localhost", dns::resolve("localhost", 80).is_ok_and(|v| v[0].ip.is_loopback()));
    let sys = Config::system();
    t.check("Config::system: name servers (link-local ones with their zone)",
        !sys.nameservers.is_empty() && sys.nameservers.iter().all(|a| !link_local(&a.ip) || a.scope != 0));
    link_local_server(t);
}

fn link_local(ip: &IpAddr) -> bool {
    matches!(ip, IpAddr::V6(a) if a[0] == 0xfe && a[1] & 0xc0 == 0x80)
}

// IPv6 zones: parsing, printing, the interface names of this system
fn scopes(t: &mut T, ll: IpAddr) {
    use plat::net::{if_index, ip_scope_parse};
    t.eq("zone by number", ip_scope_parse("fe80::1%3"), Some((ll, 3)));
    t.eq("no zone", ip_scope_parse("fe80::1"), Some((ll, 0)));
    t.eq("zone of an IPv4 address refused", ip_scope_parse("10.0.0.1%2"), None);
    t.eq("unknown interface refused", ip_scope_parse("fe80::1%nosuchif9"), None);
    t.eq("SocketAddr::parse with a zone", SocketAddr::parse("[fe80::1%3]:53"), Some(SocketAddr::with_scope(ll, 53, 3)));
    t.eq("SocketAddr shows its zone", alloc::format!("{}", SocketAddr::with_scope(ll, 53, 3)), String::from("[fe80::1%3]:53"));
    t.eq("numeric host with a zone", dns::resolve_with("fe80::1%3", 80, &Config::default()).map(|v| v[0]), Ok(SocketAddr::with_scope(ll, 80, 3)));
    t.check("if_index: no such interface", if_index("nosuchif9").is_none() && if_index("../x").is_none() && if_index("").is_none());
    // the loopback interface by its name: lo (Linux), lo0 (macOS, iOS), loopback_0 (Windows' NDIS name)
    let lo = if cfg!(windows) { "loopback_0" } else if cfg!(target_vendor = "apple") { "lo0" } else { "lo" };
    t.eq("if_index of the loopback interface", if_index(lo), Some(1));
    t.eq("zone by name", ip_scope_parse(&alloc::format!("fe80::1%{lo}")), Some((ll, 1)));
}

// a name server on a link-local address of this computer, asked through a resolv.conf line with
// the zone (Linux: the interface's name; Windows: its number)
fn link_local_server(t: &mut T) {
    let mine = plat::net::local_addrs();
    t.check("local_addrs: loopback or an address of this computer", !mine.is_empty());
    let Some(at) = mine.iter().copied().find(|a| link_local(&a.ip) && a.scope != 0) else {
        plat::write_stdout(b"  dns: no link-local address on this computer: the link-local name server is not tried\n");
        return;
    };
    let Some(ns) = server_at(at) else {
        plat::write_stdout(alloc::format!("  dns: the name server can not be bound to {}: not tried\n", at).as_bytes());
        return;
    };
    let zone = zone_name(at.scope).unwrap_or_else(|| alloc::format!("{}", at.scope));
    let conf = alloc::format!("nameserver {}%{}\n", at.ip, zone);
    let mut cfg = Config::from_resolv_conf(&conf);
    t.eq("link-local: the zone of the resolv.conf line", cfg.nameservers.first().map(|a| (a.ip, a.scope)), Some((at.ip, at.scope)));
    for a in cfg.nameservers.iter_mut() {
        a.port = ns.port;
    }
    cfg.search = alloc::vec![String::from("test")];
    let got = dns::resolve_with("fake.test", 443, &cfg).map(|v| v.into_iter().map(|a| a.ip).collect::<Vec<_>>());
    t.eq(&alloc::format!("link-local: a name server on {} (nameserver ...%{})", ns, zone), got,
        Ok(alloc::vec![IpAddr::V4([10, 1, 2, 3]), IpAddr::parse("fd00::7").unwrap_or(IpAddr::LOCALHOST_V6)]));
    t.eq("link-local: truncated -> TCP with the zone", dns::resolve_with("big.test", 1, &cfg).map(|v| v.len()), Ok(40));
    // (Linux refuses a link-local destination without a zone at once; Windows may pick one)
    if cfg!(target_os = "linux") {
        let lost = Config { nameservers: cfg.nameservers.iter().map(|a| SocketAddr { scope: 0, ..*a }).collect(), ..cfg.clone() };
        t.check("link-local: without the zone the server can not be reached", dns::resolve_with("fake.test", 1, &lost).is_err());
    }
}

// the name of interface `index` (Linux: /sys/class/net; Windows: the number is the zone)
fn zone_name(index: u32) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let mut found = None;
    plat::dir_list("/sys/class/net", &mut |name, _, _| {
        if found.is_none() && plat::net::if_index(name) == Some(index) {
            found = Some(String::from(name));
        }
    });
    found
}
