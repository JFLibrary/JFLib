// dnst -- checks of netio::dns: hosts files, resolv.conf, the message format, and a small name
// server on loopback (UDP, truncation and TCP, search domains, unknown names)

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

/// a TCP listener and a UDP socket on one free loopback port. The system's free TCP port is
/// mostly free for UDP too, but another program may hold it, and Windows hands out ports in
/// sequence and excludes whole ranges per protocol (Hyper-V, WSL), so the next ones fail alike:
/// then random ports of the dynamic range, a bounded number of times
fn bind_same_port() -> Option<(TcpListener, UdpSocket)> {
    for i in 0..64 {
        let port = if i == 0 { 0 } else { 49152 + (plat::random_u64() % 16384) as u16 };
        let Ok(tcp) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, port)) else { continue };
        let Ok(addr) = tcp.local_addr() else { continue };
        if let Ok(udp) = UdpSocket::bind(&addr) {
            return Some((tcp, udp));
        }
    }
    None
}

/// a name server on a free loopback port (UDP and TCP on the same port)
fn server() -> Option<SocketAddr> {
    let (tcp, udp) = bind_same_port()?;
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
    let c = Config::from_resolv_conf("# x\nnameserver 10.0.0.53\nnameserver fe80::1%eth0\nsearch corp.example lab.example.\noptions edns0 ndots:2\n");
    t.eq("resolv.conf: name servers", c.nameservers.len(), 2);
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
}
