// net_fuzz -- a mutation fuzzer (no coverage feedback) over JFLib's parsers of untrusted bytes:
// zstd, LZ4, both X.509 parsers, both DNS parsers, both HTTP parsers, a TLS server fed a mutated
// ClientHello, TLS clients fed a mutated first server flight, netpkt's dissector and its
// application-layer parsers (tools/net_fuzz.py builds and runs it).
//
//   fuzz SECONDS SEEDS OUT [TARGET...]
//
// Every target runs on its own thread for SECONDS, from its seeds (built here; SEEDS/ca.pem: the
// system's certificates, SEEDS/*.zst: frames of the reference zstd) and mutations of them.  A
// panic is caught (the build has panic = "unwind", overflow checks and debug assertions), its
// location counted and the first input per location saved to OUT.  A call that takes longer than
// HANG_S is a hang: its input is saved and the program ends with exit code 2.  The report is one
// line per target, "target NAME RUNS SEEDS SLOWEST_US SLOWEST_BYTES PANIC_SITES", followed by
// "panic NAME COUNT LOCATION -- MESSAGE" lines.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// the longest a single call may take before it counts as a hang
const HANG_S: u64 = 10;

thread_local! {
    static LOC: RefCell<Option<String>> = const { RefCell::new(None) };
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

fn mutate(r: &mut Rng, seeds: &[Vec<u8>], d: &[u8]) -> Vec<u8> {
    let mut v = d.to_vec();
    let ops = 1 + r.below(6);
    for _ in 0..ops {
        let len = v.len();
        match r.below(11) {
            0 if len > 0 => {
                let i = r.below(len);
                v[i] ^= 1 << r.below(8);
            }
            1 if len > 0 => {
                let i = r.below(len);
                v[i] = r.next() as u8;
            }
            2 if len > 0 => {
                let i = r.below(len);
                v[i] = [0u8, 1, 0x7f, 0x80, 0xff, 0x40, 0x3f, 0xc0][r.below(8)];
            }
            3 if len > 1 => {
                let i = r.below(len - 1);
                let x: u16 = [0u16, 0xffff, 0x8000, 0x7fff, 1, len as u16, (len as u16).wrapping_add(1)][r.below(7)];
                v[i..i + 2].copy_from_slice(&x.to_be_bytes());
            }
            4 if len > 3 => {
                let i = r.below(len - 3);
                let x: u32 = [0u32, 0xffff_ffff, 0x8000_0000, 0x7fff_ffff, 0x0100_0000][r.below(5)];
                v[i..i + 4].copy_from_slice(&x.to_be_bytes());
            }
            5 if len > 0 => {
                let a = r.below(len);
                let b = (a + 1 + r.below(64)).min(len);
                v.drain(a..b);
            }
            6 if len > 0 => {
                let a = r.below(len);
                let b = (a + 1 + r.below(128)).min(len);
                let chunk = v[a..b].to_vec();
                let at = r.below(len + 1);
                for _ in 0..1 + r.below(4) {
                    v.splice(at..at, chunk.iter().copied());
                }
            }
            7 => {
                let at = r.below(len + 1);
                let n = 1 + r.below(16);
                let ins: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
                v.splice(at..at, ins);
            }
            8 if len > 0 => {
                v.truncate(r.below(len));
            }
            9 => {
                let o = &seeds[r.below(seeds.len())];
                if !o.is_empty() {
                    let a = r.below(o.len());
                    let b = (a + 1 + r.below(256)).min(o.len());
                    let at = r.below(len + 1);
                    v.splice(at..at, o[a..b].iter().copied());
                }
            }
            10 if len > 0 => {
                let i = r.below(len);
                v[i] = if r.next() & 1 == 0 { v[i].wrapping_add(1) } else { v[i].wrapping_sub(1) };
            }
            _ => {}
        }
    }
    if v.len() > 1 << 20 {
        v.truncate(1 << 20);
    }
    v
}

// ------------------------------------------------------------------ seeds

fn zstd_seeds(dir: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut r = Rng(99);
    let text: Vec<u8> = b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. ".iter().copied().cycle().take(20000).collect();
    let rnd: Vec<u8> = (0..5000).map(|_| r.next() as u8).collect();
    let mut mixed = text[..3000].to_vec();
    mixed.extend_from_slice(&rnd[..2000]);
    mixed.extend(std::iter::repeat_n(0u8, 3000));
    for d in [&text, &rnd, &mixed, &b"a".to_vec(), &Vec::new()] {
        for lv in [1, 3, 9, 19] {
            out.push(compress::zstd::compress(d, lv));
        }
    }
    // frames of the reference implementation (other block types and modes)
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "zst") {
                if let Ok(b) = std::fs::read(e.path()) {
                    out.push(b);
                }
            }
        }
    }
    out
}

fn lz4_seeds() -> Vec<Vec<u8>> {
    let text: Vec<u8> = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ".iter().copied().cycle().take(9000).collect();
    let mut r = Rng(5);
    let rnd: Vec<u8> = (0..3000).map(|_| r.next() as u8).collect();
    let mut out = Vec::new();
    for d in [&text, &rnd] {
        out.push(compress::lz4::frame_compress(d));
        out.push(compress::lz4::compress(d));
    }
    out
}

fn pem_certs(dir: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    if let Ok(t) = std::fs::read_to_string(format!("{}/ca.pem", dir)) {
        out = netio::pem::decode_all(&t, "CERTIFICATE");
    }
    let k = crypto::ed25519::Keypair::from_seed(&[7; 32]);
    let ca = netio::x509::CertBuilder { serial: [1; 16], cn: "CA".into(), org: "O".into(), not_before: 0, not_after: 4_000_000_000, public_key: k.public, is_ca: true, ..Default::default() }.sign(None, &k);
    let cac = netio::x509::Cert::parse(&ca).unwrap();
    let leaf = netio::x509::CertBuilder { serial: [2; 16], cn: "a.lab".into(), not_before: 0, not_after: 4_000_000_000, public_key: k.public, dns: vec!["a.lab".into(), "*.b.lab".into()], ips: vec![], server_auth: true, ..Default::default() }.sign(Some(&cac), &k);
    out.push(ca);
    out.push(leaf);
    out
}

fn dns_seed() -> (Vec<u8>, Vec<u8>) {
    let q = netio::dns::query(0x1234, "www.example.org", 1).unwrap();
    let mut m = q.clone();
    m[2] = 0x81;
    m[3] = 0x80;
    m[6] = 0;
    m[7] = 3; // three answers: CNAME, A, A
    m.extend_from_slice(&[0xc0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6, 3, b'w', b'e', b'b', 0xc0, 16]);
    m.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 93, 184, 216, 34]);
    m.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
    (q, m)
}

fn http_seeds() -> Vec<Vec<u8>> {
    [
        &b"GET /a/b?x=1&y=%20 HTTP/1.1\r\nHost: example.org\r\nUser-Agent: x\r\nAccept: */*\r\n\r\n"[..],
        b"POST /upload HTTP/1.1\r\nHost: a\r\nContent-Length: 11\r\nContent-Type: text/plain\r\n\r\nhello world",
        b"POST /c HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nTrailer: x\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: a\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nX-A: b\r\n\r\nabc",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect()
}

struct Mock(Mutex<(Vec<u8>, usize)>);
impl netio::Stream for Mock {
    fn read(&self, buf: &mut [u8]) -> plat::net::NetResult<usize> {
        let mut g = self.0.lock().unwrap();
        let (d, p) = &mut *g;
        // hand the bytes out in small pieces to exercise the buffering
        let n = (d.len() - *p).min(buf.len()).min(7);
        buf[..n].copy_from_slice(&d[*p..*p + n]);
        *p += n;
        Ok(n)
    }
    fn write_all(&self, _buf: &[u8]) -> plat::net::NetResult<()> {
        Ok(())
    }
    fn set_timeouts(&self, _r: Option<u32>, _w: Option<u32>) {}
}

fn tls_ids() -> netio::tls::Identity {
    let k = crypto::ed25519::Keypair::from_seed(&[9; 32]);
    let der = netio::x509::CertBuilder { serial: [3; 16], cn: "a.lab".into(), not_before: 0, not_after: 4_000_000_000, public_key: k.public, dns: vec!["a.lab".into()], server_auth: true, ..Default::default() }.sign(None, &k);
    netio::tls::Identity { chain: vec![der], key: k }
}

fn client_hellos() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig { trust: netio::tls::Trust::Any, identity: None, server_name: "a.lab".into() });
    out.push(c.take_out());
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig { trust: netio::tls::Trust::System(vec![]), identity: None, server_name: "a.lab".into() });
    out.push(c.take_out());
    out
}

fn server_flight() -> Vec<Vec<u8>> {
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig { trust: netio::tls::Trust::Any, identity: None, server_name: "a.lab".into() });
    let mut s = netio::tls::Conn::server(netio::tls::ServerConfig { identity: tls_ids(), client_auth: None, client_auth_optional: false });
    let _ = s.feed(&c.take_out());
    vec![s.take_out()]
}

fn frame(eth_type: u16, l3: &[u8]) -> Vec<u8> {
    let mut f = vec![0x02, 0, 0, 0, 0, 1, 0x02, 0, 0, 0, 0, 2];
    f.extend_from_slice(&eth_type.to_be_bytes());
    f.extend_from_slice(l3);
    f
}

fn ipv4(proto: u8, l4: &[u8]) -> Vec<u8> {
    let tl = (20 + l4.len()) as u16;
    let mut h = vec![0x45, 0, 0, 0, 0, 1, 0x40, 0, 64, proto, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2];
    h[2..4].copy_from_slice(&tl.to_be_bytes());
    h.extend_from_slice(l4);
    h
}

fn ipv6(nh: u8, l4: &[u8]) -> Vec<u8> {
    let mut h = vec![0x60, 0, 0, 0, 0, 0, nh, 64];
    h[4..6].copy_from_slice(&(l4.len() as u16).to_be_bytes());
    h.extend_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    h.extend_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
    h.extend_from_slice(l4);
    h
}

fn tcp(sp: u16, dp: u16, payload: &[u8]) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&sp.to_be_bytes());
    h.extend_from_slice(&dp.to_be_bytes());
    h.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0x50, 0x18, 0xff, 0xff, 0, 0, 0, 0]);
    h.extend_from_slice(payload);
    h
}

fn udp(sp: u16, dp: u16, payload: &[u8]) -> Vec<u8> {
    let mut h = Vec::new();
    h.extend_from_slice(&sp.to_be_bytes());
    h.extend_from_slice(&dp.to_be_bytes());
    h.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    h.extend_from_slice(&[0, 0]);
    h.extend_from_slice(payload);
    h
}

fn packet_seeds() -> Vec<Vec<u8>> {
    let (_, dnsr) = dns_seed();
    let ch = client_hellos();
    let http = http_seeds();
    let snmp_get: Vec<u8> = vec![0x30, 0x26, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xa0, 0x19, 0x02, 0x01, 0x01, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0e, 0x30, 0x0c, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00];
    let radius: Vec<u8> = {
        let mut p = vec![1, 7, 0, 0];
        p.extend_from_slice(&[0x11; 16]);
        p.extend_from_slice(&[1, 6, b'u', b's', b'e', b'r', 2, 18]);
        p.extend_from_slice(&[0x22; 16]);
        let l = p.len() as u16;
        p[2..4].copy_from_slice(&l.to_be_bytes());
        p
    };
    let bgp_open: Vec<u8> = {
        let mut p = vec![0xff; 16];
        p.extend_from_slice(&[0, 0, 1, 4, 0xfd, 0xe8, 0, 180, 10, 0, 0, 1, 8, 2, 6, 1, 4, 0, 1, 0, 1]);
        let l = p.len() as u16;
        p[16..18].copy_from_slice(&l.to_be_bytes());
        p
    };
    vec![
        frame(0x0800, &ipv4(6, &tcp(40000, 80, &http[0]))),
        frame(0x0800, &ipv4(6, &tcp(80, 40000, &http[4]))),
        frame(0x0800, &ipv4(17, &udp(53, 40000, &dnsr))),
        frame(0x86dd, &ipv6(17, &udp(53, 40000, &dnsr))),
        frame(0x0800, &ipv4(6, &tcp(40001, 443, &ch[1]))),
        frame(0x0800, &ipv4(6, &tcp(40002, 22, b"SSH-2.0-OpenSSH_9.6\r\n"))),
        frame(0x0800, &ipv4(17, &udp(40003, 161, &snmp_get))),
        frame(0x0800, &ipv4(17, &udp(40004, 1812, &radius))),
        frame(0x0800, &ipv4(6, &tcp(40005, 179, &bgp_open))),
        frame(0x0800, &ipv4(17, &udp(68, 67, &[1u8; 300]))),
        frame(0x0800, &ipv4(1, &[8, 0, 0, 0, 0, 1, 0, 1, b'p', b'i', b'n', b'g'])),
        frame(0x86dd, &ipv6(58, &[135, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 1, 1, 2, 0, 0, 0, 0, 1])),
        frame(0x0806, &[0, 1, 8, 0, 6, 4, 0, 1, 2, 0, 0, 0, 0, 1, 10, 0, 0, 1, 0, 0, 0, 0, 0, 0, 10, 0, 0, 2]),
        frame(0x0800, &ipv4(89, &[2, 1, 0, 44, 10, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 0, 0, 10, 2, 1, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 0])),
        frame(0x0800, &ipv4(17, &udp(69, 40006, &[0, 1, b'f', b'i', b'l', b'e', 0, b'o', b'c', b't', b'e', b't', 0]))),
        frame(0x0800, &ipv4(17, &udp(123, 123, &[0x23; 48]))),
        frame(0x0800, &ipv4(17, &udp(514, 514, b"<34>Oct 11 22:14:15 host su: 'su root' failed"))),
    ]
}

// ------------------------------------------------------------------ targets

type Fun = Box<dyn Fn(&[u8]) + Send + Sync>;

fn targets(seed_dir: &str) -> Vec<(&'static str, Vec<Vec<u8>>, Fun)> {
    let (dq, dm) = dns_seed();
    let certs = pem_certs(seed_dir);
    let anchors: Vec<netio::x509::Cert> = certs.iter().filter_map(|c| netio::x509::Cert::parse(c)).take(20).collect();
    let id = tls_ids();
    vec![
        // a deliberate panic on inputs that start with 0xEE: the run must find it (net_fuzz.py)
        ("selftest_panic", vec![b"seed".to_vec()], Box::new(|d: &[u8]| {
            if d.first() == Some(&0xEE) {
                panic!("the self-test's deliberate panic");
            }
        }) as Fun),
        ("zstd", zstd_seeds(seed_dir), Box::new(|d: &[u8]| {
            let _ = compress::zstd::decompress(d, 1 << 22);
            let _ = compress::zstd::content_size(d);
        })),
        ("lz4", lz4_seeds(), Box::new(|d: &[u8]| {
            let _ = compress::lz4::frame_decompress(d, 1 << 20);
            let _ = compress::lz4::decompress(d, 1 << 20);
        })),
        ("x509", certs.clone(), Box::new(move |d: &[u8]| {
            if let Some(c) = netio::x509::Cert::parse(d) {
                let _ = c.names("a.lab");
                let _ = c.fingerprint();
                let _ = netio::x509::verify_chain(&[c.clone()], &[c.clone()], 1_800_000_000, Some("a.lab"), netio::x509::Usage::Server);
                let _ = netio::x509::verify_chain(&[c.clone(), c.clone()], &anchors, 1_800_000_000, Some("www.example.org"), netio::x509::Usage::Server);
                let _ = c.signed_by(&c);
            }
        })),
        ("x509_netpkt", certs, Box::new(|d: &[u8]| {
            let _ = netpkt::x509::Cert::parse(d);
        })),
        ("dns", vec![dm.clone()], Box::new(move |d: &[u8]| {
            let _ = netio::dns::parse_answer(d, &dq, 1);
        })),
        ("dns_netpkt", vec![dm], Box::new(|d: &[u8]| {
            let _ = netpkt::dns::Message::parse(d);
        })),
        ("http_netio", http_seeds(), Box::new(|d: &[u8]| {
            let m = Mock(Mutex::new((d.to_vec(), 0)));
            let mut r = netio::http::Reader::new(&m);
            if let Ok((_line, h)) = r.head() {
                let _ = r.body(&h, 1 << 20, false);
                let _ = r.head();
            }
        })),
        ("http_netpkt", http_seeds(), Box::new(|d: &[u8]| {
            let _ = netpkt::http::parse_head(d, true);
            let _ = netpkt::http::parse_head(d, false);
        })),
        ("tls_server", client_hellos(), Box::new(move |d: &[u8]| {
            let mut s = netio::tls::Conn::server(netio::tls::ServerConfig { identity: id.clone(), client_auth: None, client_auth_optional: false });
            let _ = s.feed(d);
            let _ = s.take_out();
        })),
        ("tls_client", server_flight(), Box::new(|d: &[u8]| {
            let mut c = netio::tls::Conn::client(netio::tls::ClientConfig { trust: netio::tls::Trust::Any, identity: None, server_name: "a.lab".into() });
            let _ = c.take_out();
            let _ = c.feed(d);
        })),
        ("tls_webclient", server_flight(), Box::new(|d: &[u8]| {
            let mut c = netio::tls::Conn::client(netio::tls::ClientConfig { trust: netio::tls::Trust::System(vec![]), identity: None, server_name: "a.lab".into() });
            let _ = c.take_out();
            let _ = c.feed(d);
        })),
        ("netpkt_decode", packet_seeds(), Box::new(|d: &[u8]| {
            let _ = netpkt::decode::layers(d);
            let _ = netpkt::decode::info(d);
            let _ = netpkt::decode::stack(d);
        })),
        ("netpkt_app", packet_seeds(), Box::new(|d: &[u8]| {
            // the same bytes straight into the application-layer parsers
            for off in [0usize, 42, 54, 62] {
                let b = d.get(off..).unwrap_or(&[]);
                let _ = netpkt::snmp::Message::parse(b);
                let _ = netpkt::radius::Packet::parse(b);
                let _ = netpkt::bgp::Message::read(b, true);
                let _ = netpkt::bgp::Message::read(b, false);
                let _ = netpkt::imf::Message::parse(b);
                let _ = netpkt::ssh::KexInit::parse(b);
            }
        })),
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: fuzz SECONDS SEEDS OUT [TARGET...]");
        std::process::exit(64);
    }
    let secs: u64 = args[1].parse().unwrap_or(30);
    let seeds_dir = args[2].clone();
    let out = args[3].clone();
    let only: Vec<String> = args.iter().skip(4).cloned().collect();
    std::fs::create_dir_all(&out).unwrap();
    panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() { s.to_string() } else if let Some(s) = info.payload().downcast_ref::<String>() { s.clone() } else { String::new() };
        LOC.with(|c| *c.borrow_mut() = Some(format!("{} -- {}", loc, msg.replace('\n', " "))));
    }));
    let ts = targets(&seeds_dir);
    for o in &only {
        if !ts.iter().any(|t| t.0 == o) {
            eprintln!("unknown target {} ({})", o, ts.iter().map(|t| t.0).collect::<Vec<_>>().join(", "));
            std::process::exit(64);
        }
    }
    let done = AtomicBool::new(false);
    let report = Mutex::new(Vec::<String>::new());
    // per target: when its current call began (us since start, 0: none) and its input
    let begun: Vec<AtomicU64> = ts.iter().map(|_| AtomicU64::new(0)).collect();
    let current: Vec<Mutex<Vec<u8>>> = ts.iter().map(|_| Mutex::new(Vec::new())).collect();
    let t0 = Instant::now();
    std::thread::scope(|sc| {
        // the watchdog: a call past HANG_S saves its input and ends the program
        sc.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let now = t0.elapsed().as_micros() as u64;
                for (i, b) in begun.iter().enumerate() {
                    let at = b.load(Ordering::Relaxed);
                    if at != 0 && now.saturating_sub(at) > HANG_S * 1_000_000 {
                        let input = current[i].lock().map(|g| g.clone()).unwrap_or_default();
                        let _ = std::fs::write(format!("{}/{}-hang.bin", out, ts[i].0), &input);
                        println!("hang {} {} bytes, more than {} s", ts[i].0, input.len(), HANG_S);
                        std::process::exit(2);
                    }
                }
            }
        });
        let workers: Vec<_> = ts.iter().enumerate().filter(|(_, t)| only.is_empty() || only.iter().any(|o| o == t.0)).map(|(ti, (name, seeds, f))| {
            let (out, report, begun, current) = (out.clone(), &report, &begun[ti], &current[ti]);
            std::thread::Builder::new().stack_size(64 << 20).spawn_scoped(sc, move || {
                let mut r = Rng(0x9E37_79B9_7F4A_7C15 ^ (ti as u64 + 1).wrapping_mul(0x1234_5678_9abc));
                let end = Instant::now() + Duration::from_secs(secs);
                let mut n = 0u64;
                let mut panics: BTreeMap<String, usize> = BTreeMap::new();
                let mut slow = (Duration::ZERO, 0usize);
                // the seeds as they are first
                let mut queue: Vec<Vec<u8>> = seeds.clone();
                while Instant::now() < end {
                    let input = if let Some(s) = queue.pop() {
                        s
                    } else {
                        let base = &seeds[r.below(seeds.len())];
                        let mut m = mutate(&mut r, seeds, base);
                        if r.below(4) == 0 {
                            m = mutate(&mut r, seeds, &m);
                        }
                        m
                    };
                    if let Ok(mut g) = current.lock() {
                        g.clear();
                        g.extend_from_slice(&input);
                    }
                    let started = Instant::now();
                    begun.store((t0.elapsed().as_micros() as u64).max(1), Ordering::Relaxed);
                    let res = panic::catch_unwind(AssertUnwindSafe(|| f(&input)));
                    begun.store(0, Ordering::Relaxed);
                    let dt = started.elapsed();
                    if dt > slow.0 {
                        slow = (dt, input.len());
                        let _ = std::fs::write(format!("{}/{}-slowest.bin", out, name), &input);
                    }
                    if res.is_err() {
                        let loc = LOC.with(|c| c.borrow_mut().take()).unwrap_or_default();
                        if !panics.contains_key(&loc) {
                            let _ = std::fs::write(format!("{}/{}-panic-{}.bin", out, name, panics.len() + 1), &input);
                        }
                        *panics.entry(loc).or_insert(0) += 1;
                    }
                    n += 1;
                }
                let mut s = format!("target {} {} {} {} {} {}\n", name, n, seeds.len(), slow.0.as_micros(), slow.1, panics.len());
                for (k, v) in &panics {
                    s += &format!("panic {} {} {}\n", name, v, k);
                }
                report.lock().unwrap().push(s);
            }).unwrap()
        }).collect();
        for w in workers {
            let _ = w.join();
        }
        done.store(true, Ordering::Relaxed);
    });
    let mut rep = report.into_inner().unwrap();
    rep.sort();
    for s in rep {
        print!("{}", s);
    }
}
