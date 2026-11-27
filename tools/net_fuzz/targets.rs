// net_fuzz targets -- the parsers of untrusted bytes, their seeds and the code that feeds them
// (compress, formats, netchan, netio, netpkt, plat).
// Shared by tools/net_fuzz/fuzz.rs (the mutation fuzzer, tools/net_fuzz.py) and the libFuzzer
// targets of tools/cov_fuzz.py, which include this file with #[path].
//
//   targets(seed_dir) -> [(name, seeds, run)]    all of them, built once
//   get(name) -> the run function of one, from a set built on first use (SEEDS: environment
//                variable NET_FUZZ_SEEDS, the folder with ca.pem and *.zst)
#![allow(dead_code)]

use std::sync::{Mutex, OnceLock};

/// xorshift64 (the mutations and the random parts of seeds)
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
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
    netio::tls::Identity::new(vec![der], k)
}

fn client_hellos() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig::new(netio::tls::Trust::Any, "a.lab"));
    out.push(c.take_out());
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig::new(netio::tls::Trust::System(vec![]), "a.lab"));
    out.push(c.take_out());
    out
}

/// the tickets of tls_server_psk: fixed keys, so the seeds' tickets open in every run
fn psk_tickets() -> std::sync::Arc<netio::tls::Tickets> {
    let tk = std::sync::Arc::new(netio::tls::Tickets::new());
    tk.set_keys(&[[3; 32]]);
    tk
}

fn psk_server(tk: &std::sync::Arc<netio::tls::Tickets>) -> netio::tls::ServerConfig {
    netio::tls::ServerConfig { tickets: Some(tk.clone()), early_data: 4096, ..netio::tls::ServerConfig::new(tls_ids()) }
}

/// ClientHellos that offer a ticket of psk_tickets, one with 0-RTT data behind it
fn psk_hellos(tk: &std::sync::Arc<netio::tls::Tickets>) -> Vec<Vec<u8>> {
    let ss = std::sync::Arc::new(netio::tls::Sessions::default());
    let ccfg = || netio::tls::ClientConfig { sessions: Some(ss.clone()), ..netio::tls::ClientConfig::new(netio::tls::Trust::Any, "a.lab") };
    let mut c = netio::tls::Conn::client(ccfg());
    let mut s = netio::tls::Conn::server(psk_server(tk));
    for _ in 0..10 {
        let a = c.take_out();
        let _ = s.feed(&a);
        let b = s.take_out();
        let _ = c.feed(&b);
    }
    let mut out = vec![netio::tls::Conn::client(ccfg()).take_out()];
    out.push(netio::tls::Conn::client_early(ccfg(), b"GET / HTTP/1.1\r\nHost: a.lab\r\n\r\n").take_out());
    out
}

fn server_flight() -> Vec<Vec<u8>> {
    let mut c = netio::tls::Conn::client(netio::tls::ClientConfig::new(netio::tls::Trust::Any, "a.lab"));
    let mut s = netio::tls::Conn::server(netio::tls::ServerConfig::new(tls_ids()));
    let _ = s.feed(&c.take_out());
    vec![s.take_out()]
}

/// a CA, a leaf of it, and OCSP responses about the leaf (good, revoked with a reason, by a
/// delegated responder, with a nonce, SHA-256 ids) and requests for it
fn ocsp_seeds() -> (netio::x509::Cert, netio::x509::Cert, Vec<Vec<u8>>) {
    use netio::x509::ocsp::{IdHash, ResponseBuilder, Status};
    use netio::x509::{Cert, CertBuilder, PrivateKey};
    let ck = crypto::ed25519::Keypair::from_seed(&[11; 32]);
    let ca_der = CertBuilder { serial: [1; 16], cn: "OCSP CA".into(), not_before: 0, not_after: 4_000_000_000, public_key: ck.public, is_ca: true, ..Default::default() }.sign(None, &ck);
    let ca = Cert::parse(&ca_der).unwrap_or_default();
    let lk = crypto::ed25519::Keypair::from_seed(&[12; 32]);
    let leaf_der = CertBuilder { serial: [2; 16], cn: "a.lab".into(), not_before: 0, not_after: 4_000_000_000, public_key: lk.public, dns: vec!["a.lab".into()], server_auth: true, ocsp_urls: vec!["http://ocsp.lab/".into()], ..Default::default() }.sign(Some(&ca), &ck);
    let leaf = Cert::parse(&leaf_der).unwrap_or_default();
    let rk = crypto::ed25519::Keypair::from_seed(&[13; 32]);
    let resp_der = CertBuilder { serial: [3; 16], cn: "responder".into(), not_before: 0, not_after: 4_000_000_000, public_key: rk.public, ocsp_signing: true, ..Default::default() }.sign(Some(&ca), &ck);
    let responder = Cert::parse(&resp_der).unwrap_or_default();
    let (cak, rkk) = (PrivateKey::from(ck), PrivateKey::from(rk));
    let t = 1_800_000_000 - 3600;
    let good = ResponseBuilder::good(t);
    let mut out: Vec<Vec<u8>> = Vec::new();
    out.extend(good.sign(&leaf, &ca, None, &cak));
    out.extend(ResponseBuilder { status: Status::Revoked { time: t - 60, reason: Some(1) }, by_key: false, ..good.clone() }.sign(&leaf, &ca, None, &cak));
    out.extend(ResponseBuilder { nonce: Some(vec![7; 16]), hash: IdHash::Sha256, next_update: None, ..good.clone() }.sign(&leaf, &ca, None, &cak));
    out.extend(good.sign(&leaf, &ca, Some(&responder), &rkk));
    out.push(netio::x509::ocsp::error_response(netio::x509::ocsp::TRY_LATER));
    if let Some(id) = netio::x509::ocsp::CertId::new(&leaf, &ca) {
        out.push(netio::x509::ocsp::request(std::slice::from_ref(&id), Some(&[5; 20])));
        out.push(netio::x509::ocsp::request(&[id.clone(), id], None));
    }
    (leaf, ca, out)
}

/// certificate requests of each key kind, with names, addresses and must-staple
fn csr_seeds() -> Vec<Vec<u8>> {
    use netio::x509::{CsrBuilder, IpAddr, PrivateKey};
    let mut r = Rng(0x5eed);
    let mut rand = |b: &mut [u8]| b.iter_mut().for_each(|x| *x = r.next() as u8);
    let keys = vec![
        PrivateKey::from(crypto::ed25519::Keypair::from_seed(&[14; 32])),
        PrivateKey::generate_ecdsa(crypto::ecdsa::Curve::P256, &mut rand),
        PrivateKey::generate_ecdsa(crypto::ecdsa::Curve::P384, &mut rand),
    ];
    let mut out = Vec::new();
    for k in &keys {
        out.extend(CsrBuilder::for_names(&["a.lab", "*.b.lab"]).sign(k));
        out.extend(CsrBuilder { ips: vec![IpAddr::LOCALHOST_V4], must_staple: true, ..CsrBuilder::default() }.sign(k));
    }
    out
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

/// fragments of a datagram, each with a 2-byte length before it (the reassembly targets read
/// a sequence of fragments from one input)
fn fragment_seeds(v6: bool) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (len, mtu) in [(3000usize, 1280usize), (1500, 576), (9000, 1500), (200, 68)] {
        let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
        let parts = if v6 {
            netpkt::ipv6::fragment(&ipv6(17, &udp(4000, 5000, &payload)), mtu.max(1280), 0x1234_5678)
        } else {
            // without DF (ipv4() sets it), or there is nothing to fragment
            let mut p = ipv4(17, &udp(4000, 5000, &payload));
            p[6] &= !0x40;
            netpkt::ipv4::fragment(&p, mtu)
        };
        let Some(mut parts) = parts else { continue };
        for order in 0..3 {
            if order == 1 {
                parts.reverse();
            }
            if order == 2 && parts.len() > 2 {
                parts.swap(0, 1);
                parts.push(parts[1].clone()); // a duplicate
            }
            let mut seq = Vec::new();
            for f in &parts {
                seq.extend_from_slice(&(f.len() as u16).to_be_bytes());
                seq.extend_from_slice(f);
            }
            out.push(seq);
        }
    }
    out
}

/// the fragments of an input made by fragment_seeds (mutated: any lengths)
fn split_fragments(d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 2 <= d.len() && out.len() < 10_000 {
        let n = u16::from_be_bytes([d[p], d[p + 1]]) as usize;
        let end = (p + 2 + n).min(d.len());
        out.push(&d[p + 2..end]);
        p = end;
    }
    out
}


// coretest's Brotli streams (apps/coretest/src/lz.rs: the reference encoder's output at several
// qualities and modes; JFLib has no Brotli encoder)
const BROTLI_STREAMS: &[&[u8]] = &[
    &[0x06],
    &[0x0b, 0x02, 0x80, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x03],
    &[0x8b, 0x9b, 0x00, 0x00, 0x80, 0xaa, 0xaa, 0xaa, 0xea, 0xff, 0x74, 0x65, 0xb8, 0x1b, 0xd8, 0xe5, 0x60, 0x00, 0x76, 0x38, 0xdb, 0x59, 0xc1, 0x84, 0xcd, 0x4c, 0x40, 0x15, 0xd8, 0x0c, 0x44, 0x45, 0xc0, 0x60, 0x03, 0x6e, 0x19, 0xd8, 0xc7, 0x81, 0x31, 0x1c, 0xcc, 0x87, 0x8d, 0x31, 0xcf, 0xd0, 0x68, 0xd8, 0x5d, 0xd8, 0xda, 0xd9, 0xeb, 0x83, 0xcf, 0x04, 0x63, 0x26, 0xac, 0x49, 0x60, 0x34, 0x58, 0xf0, 0x53, 0x82, 0x39, 0x71, 0x38, 0x8d, 0xf1, 0x2c, 0xc5, 0x38, 0x46, 0xe9, 0xc6, 0x55, 0xea, 0xe9, 0xaf, 0x9c, 0x91, 0x2b, 0x1e, 0x2f, 0x62,],
    &[0x1b, 0x37, 0x01, 0xf8, 0xc5, 0x5c, 0x5b, 0xaa, 0x47, 0x17, 0x4b, 0x10, 0x0c, 0xd9, 0xfc, 0xb8, 0x53, 0xc7, 0x3f, 0xa7, 0x6c, 0x7b, 0xac, 0x8f, 0x46, 0x2c, 0x14, 0xba, 0x80, 0xdd, 0x85, 0xad, 0x9d, 0x7d, 0x7e, 0xf0, 0x2c, 0x30, 0xb2, 0x60, 0x4d, 0x02, 0x4d, 0xd2, 0x82, 0x9f, 0x12, 0xcc, 0x89, 0xc3, 0x69, 0x8c, 0x87, 0x91, 0xc5, 0x31, 0xa6, 0x6e, 0x5c, 0x53, 0xbd, 0xfc, 0x95, 0x33, 0x6a, 0x45,],
    &[0x1b, 0x58, 0x00, 0x88, 0x1c, 0x09, 0x76, 0xec, 0xc8, 0x3a, 0x87, 0x8c, 0x1e, 0xa9, 0xa8, 0xd0, 0x40, 0x0f, 0x96, 0xea, 0x1e, 0x8a, 0x9a, 0xa0, 0x68, 0x0a, 0x2a, 0xec, 0x0d, 0x36, 0xe0, 0xc0, 0x21, 0x0b, 0x64, 0x7f, 0x21, 0xbf, 0x41, 0xa7, 0x15, 0xce, 0x0c, 0x3c, 0x4e, 0xd4, 0xbc, 0x14, 0x32, 0x08, 0x77, 0xda, 0x1f, 0xbe, 0x0c, 0xb1, 0x02,],
    &[0x1b, 0x45, 0x00, 0x00, 0x44, 0x27, 0x87, 0x78, 0x90, 0xef, 0x18, 0xb4, 0x84, 0x8a, 0x0e, 0x95, 0x07, 0x5c, 0xfc, 0xb0, 0x46, 0xa4, 0x54, 0xa3, 0x12, 0x6c, 0x8c, 0x9d, 0x63, 0x20, 0x8d, 0x27, 0x2a, 0x09, 0x52, 0xfb, 0x8d, 0xdb, 0x65, 0x94, 0x3b, 0x0f, 0xb6, 0x14, 0x46, 0xf3, 0xfb, 0xa2, 0xfe, 0x23, 0xa8, 0x09, 0x07,]
];

/// texts of several kinds for the compressors' seeds
fn plain_texts() -> Vec<Vec<u8>> {
    let mut r = Rng(17);
    let text: Vec<u8> = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ".iter().copied().cycle().take(12000).collect();
    let rnd: Vec<u8> = (0..3000).map(|_| r.next() as u8).collect();
    let mut mixed = text[..2000].to_vec();
    mixed.extend_from_slice(&rnd[..1000]);
    mixed.extend(std::iter::repeat_n(0u8, 40000)); // long matches, a window past 32 KiB
    vec![text, rnd, mixed, b"a".to_vec(), Vec::new()]
}

/// the reference encoder's Brotli streams of coretest, and google/brotli's test streams the
/// seed folder holds (*.br: tools/net_fuzz.py copies them when tools/brotli_check.py --vectors
/// fetched them)
fn brotli_seeds(dir: &str) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = BROTLI_STREAMS.iter().map(|s| s.to_vec()).collect();
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "br")).collect();
        files.sort();
        out.extend(files.iter().filter_map(|p| std::fs::read(p).ok()));
    }
    out
}

/// DEFLATE, zlib or gzip streams at the levels of each strategy (gzip: also two members)
fn deflate_seeds(kind: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for d in plain_texts() {
        for lv in [0, 1, 6, 9] {
            out.push(match kind {
                "deflate" => compress::deflate::compress(&d, lv),
                "zlib" => compress::zlib::compress(&d, lv),
                _ => compress::gzip::compress(&d, lv),
            });
        }
    }
    if kind == "gzip" {
        let mut two = compress::gzip::compress(b"first member ", 6);
        two.extend(compress::gzip::compress(b"second member", 1));
        out.push(two);
    }
    out
}

fn json_seeds() -> Vec<Vec<u8>> {
    [
        &br#"{"a":1,"b":[true,false,null],"c":{"d":"e\u00fc\ud83d\ude00\n","f":-12.5e-3},"g":[]}"#[..],
        br#"[1e308,-0,0.1,123456789012345678901234567890,"\"\\\/\b\f\r\t",[[[[[[[[[[1]]]]]]]]]]]"#,
        br#"  {"repeated":1,"repeated":2,"long":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}  "#,
        b"\xef\xbb\xbf{}",
        b"\"text\"",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect()
}

fn xml_seeds() -> Vec<Vec<u8>> {
    [
        &br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a xmlns="urn:x" xmlns:p="urn:p" p:k='v&amp;w'><b>t&lt;&#x41;&#66;</b><![CDATA[<raw>]]><!-- c --><?pi data?><e/></a>"#[..],
        br#"<!DOCTYPE svg [<!ENTITY e "&f;&f;"><!ENTITY f "xyz">]><svg width="10" height="10"><text>&e;</text><path d="M0 0L10 10"/></svg>"#,
        br#"<VisioDocument xmlns="http://schemas.microsoft.com/office/visio/2012/main"><Shapes><Shape ID="1" Type="Shape"><Cell N="PinX" V="1.5"/><Text>Box</Text></Shape></Shapes></VisioDocument>"#,
        br#"<?xml version="1.0"?><fontconfig><dir>/usr/share/fonts</dir><match target="pattern"><test name="family"><string>mono</string></test></match></fontconfig>"#,
        b"<?>",
        b"<a><b></a>",
        b"\xef\xbb\xbf<r/>",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect()
}

/// a charset label's index, then text in that charset (BOMs for UTF-16 and UTF-8)
const CHARSET_LABELS: &[&str] = &["utf-8", "utf-16le", "utf-16be", "iso-8859-1", "windows-1252", "iso-8859-2", "iso-8859-15",
    "koi8-r", "koi8-u", "windows-1251", "macintosh", "ibm866", "x-user-defined", "shift_jis", "gbk", "latin2", "", "bogus"];

fn charset_seeds() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (i, text) in [(0u8, &"Grüße, Ünïcödé €".as_bytes()[..]), (3, b"Gr\xfc\xdfe \xa4"), (4, b"\x80\x9f quotes \x93\x94"), (7, b"\xf0\xd2\xc9\xd7\xc5\xd4"),
                      (1, b"\xff\xfeG\x00r\x00\x3d\xd8\x00\xde"), (2, b"\xfe\xff\x00G\xd8\x3d\xde\x00"), (0, b"\xef\xbb\xbfBOM \xc3")] {
        let mut s = vec![i];
        s.extend_from_slice(text);
        out.push(s);
    }
    out
}

// netchan: a UDP handshake with fixed secrets and time, so that the sealed packets of the seeds
// open in a run as well (the targets build the same server and client)
const NC_NOW: u64 = 1_000_000;

fn nc_from() -> plat::net::SocketAddr {
    plat::net::SocketAddr::new(plat::net::IpAddr::V4([10, 2, 0, 1]), 4000)
}

fn nc_server() -> netchan::endpoint::ServerCore {
    netchan::endpoint::ServerCore::new(netchan::ServerConfig::default(), [9; 32], NC_NOW)
}

fn nc_client() -> netchan::endpoint::ClientCore {
    netchan::endpoint::ClientCore::new(netchan::ClientConfig::default(), [42; 32], false)
}

/// what the client sent and what the server sent in a real handshake and a few packets each way
fn nc_flights() -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    use netchan::endpoint::{Out, Route};
    let (mut s, mut cl, from) = (nc_server(), nc_client(), nc_from());
    let (mut up, mut down) = (Vec::new(), Vec::new());
    cl.start(NC_NOW);
    for round in 0..6 {
        while let Some(b) = cl.take_out() {
            s.datagram(from, &b, NC_NOW);
            up.push(b);
        }
        while let Some(o) = s.take_out() {
            if let Out::Send(Route::Udp(a), b) = o {
                if a == from {
                    cl.packet(&b, NC_NOW);
                    down.push(b);
                }
            }
        }
        while s.poll_event().is_some() {}
        while cl.poll_event().is_some() {}
        if round >= 2 && cl.state() == netchan::State::Connected {
            let _ = cl.send(b"from the client", NC_NOW);
            for id in s.ids() {
                let _ = s.send(id, b"from the server", NC_NOW);
            }
        }
    }
    (up, down)
}

/// datagrams one after the other, each with its length (u16, big-endian): split_fragments' format
fn joined(v: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for b in v {
        out.extend_from_slice(&(b.len() as u16).to_be_bytes());
        out.extend_from_slice(b);
    }
    out
}

/// every prefix of a flight (the first datagram alone, the first two, ...)
fn prefixes(v: &[Vec<u8>]) -> Vec<Vec<u8>> {
    (1..=v.len()).map(|n| joined(&v[..n])).collect()
}

fn nc_conns() -> (netchan::conn::Conn, netchan::conn::Conn) {
    let set = netchan::conn::Settings { timeout_ms: 5000, keepalive_ms: 100, packets_per_s: 1 << 20, bytes_per_s: 1 << 30 };
    let (ka, kb) = ([3u8; 32], [4u8; 32]);
    (netchan::conn::Conn::new(7, false, set, 0).with_keys(netchan::seal::Keys::new(ka, kb)),
     netchan::conn::Conn::new(7, false, set, 0).with_keys(netchan::seal::Keys::new(kb, ka)))
}

/// sealed DATA and DISCONNECT packets, behind a byte that says which (0: DATA, 1: DISCONNECT)
fn nc_sealed() -> Vec<Vec<u8>> {
    let (mut a, _) = nc_conns();
    let mut out = Vec::new();
    for (i, payload) in [&b""[..], b"x", &[7u8; 300]].iter().enumerate() {
        let mut w = Vec::new();
        a.write_data(payload, i as u64, &mut w);
        out.push([&[0u8][..], &w].concat());
    }
    let mut w = Vec::new();
    a.write_disconnect(3, &mut w);
    out.push([&[1u8][..], &w].concat());
    out
}


// plat: other programs' data (docs/PLAT_INPUTS.md), the seeds of plattest --test wirefuzz
fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// a DROPFILES block (Windows' drops): offset 20, wide or ANSI names, a double NUL
fn dropfiles(paths: &[&str], wide_names: bool) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [20u32, 0, 0, 0, wide_names as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for p in paths {
        if wide_names {
            b.extend(wide(p));
            b.extend([0, 0]);
        } else {
            b.extend(p.bytes());
            b.push(0);
        }
    }
    b.extend(if wide_names { &[0u8, 0][..] } else { &[0u8][..] });
    b
}

/// a 2 x 2 DIB, 32 bits, BI_RGB
fn dib() -> Vec<u8> {
    let mut b = Vec::new();
    for v in [40u32, 2, 2] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, 16, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 128, 255, 0, 0, 255, 255, 255, 255, 0]);
    b
}

/// CF_HTML: the header with its offsets, the fragment between the markers
fn cf_html(fragment: &str) -> Vec<u8> {
    let (pre, post) = ("<html><body><!--StartFragment-->", "<!--EndFragment--></body></html>");
    let head = |a: usize, b: usize, c: usize, d: usize| format!("Version:0.9\r\nStartHTML:{a:08}\r\nEndHTML:{b:08}\r\nStartFragment:{c:08}\r\nEndFragment:{d:08}\r\n");
    let n = head(0, 0, 0, 0).len();
    let (start, end) = (n + pre.len(), n + pre.len() + fragment.len());
    let mut b = head(n, end + post.len(), start, end).into_bytes();
    b.extend(pre.bytes());
    b.extend(fragment.bytes());
    b.extend(post.bytes());
    b
}

fn clipboard_seeds() -> Vec<Vec<u8>> {
    let mut text = wide("Hallo\r\nWelt");
    text.extend([0, 0]);
    vec![text, dropfiles(&["C:\\a", "C:\\b c"], true), dropfiles(&["/x", "/y"], false), dib(), cf_html("<b>bold</b> &amp;"),
         b"file:///tmp/a%20b\r\n# comment\r\nfile:///home/x\r\n".to_vec()]
}

/// a D-Bus signal (little endian): path "/a", interface "x.y", member "M", signature "s", body "hi"
fn dbus_signal() -> Vec<u8> {
    let mut f: Vec<u8> = Vec::new();
    let mut field = |code: u8, sig: u8, val: &[u8]| {
        while f.len() % 8 != 0 {
            f.push(0);
        }
        f.extend([code, 1, sig, 0]);
        if sig == b'g' {
            f.push(val.len() as u8);
        } else {
            f.extend((val.len() as u32).to_le_bytes());
        }
        f.extend(val);
        f.push(0);
    };
    field(1, b'o', b"/a");
    field(2, b's', b"x.y");
    field(3, b's', b"M");
    field(8, b'g', b"s");
    let mut body = Vec::new();
    body.extend(2u32.to_le_bytes());
    body.extend(b"hi\0");
    let mut m = vec![b'l', 4, 0, 1];
    m.extend((body.len() as u32).to_le_bytes());
    m.extend(1u32.to_le_bytes());
    m.extend((f.len() as u32).to_le_bytes());
    m.extend(&f);
    while m.len() % 8 != 0 {
        m.push(0);
    }
    m.extend(&body);
    m
}

/// a Wayland event: object 3, opcode 1, the words of its body
fn wl_event(words: &[u32]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend(3u32.to_le_bytes());
    m.extend((1u32 | ((8 + 4 * words.len() as u32) << 16)).to_le_bytes());
    for w in words {
        m.extend(w.to_le_bytes());
    }
    m
}

/// the targets that need Linux (D-Bus, Wayland)
#[cfg(target_os = "linux")]
fn linux_targets() -> Vec<(&'static str, Vec<Vec<u8>>, Fun)> {
    vec![
        ("plat_dbus", vec![dbus_signal()], Box::new(|d: &[u8]| {
            // one message after the other, as the bus reader does (Err(n): skip n bytes)
            let mut at = 0;
            while at < d.len() {
                match plat::fuzz::dbus(&d[at..]) {
                    Ok(Some(n)) if n > 0 => at += n,
                    Err(n) if n > 0 => at += n,
                    _ => break,
                }
            }
        }) as Fun),
        ("plat_wayland", vec![wl_event(&[]), wl_event(&[7, 0x0001_0000]), wl_event(&[5, 0x6c6c_6568, 0x006f, 9, 3])], Box::new(|d: &[u8]| {
            let _ = plat::fuzz::wl_args(d);
        })),
    ]
}

#[cfg(not(target_os = "linux"))]
fn linux_targets() -> Vec<(&'static str, Vec<Vec<u8>>, Fun)> {
    Vec::new()
}

// ------------------------------------------------------------------ compress in pieces

/// the output limit of the decoders (one-shot and in pieces alike)
const DECODE_LIMIT: usize = 1 << 22;

/// the sizes of a piece: 1 byte, a few, a block, more (the schedule's rng picks)
fn piece(r: &mut Rng) -> usize {
    let k = [1, 8, 300, 5000][r.below(4)];
    1 + r.below(k)
}

/// a stream decoder fed in pieces of the schedule's sizes, with output buffers of 1 byte to 4
/// KiB, the way stream::decode_all pumps it (input after a stream goes on with the next
/// member or frame; a format of one stream refuses it) -> the output, or the error
fn pump_pieces(dec: &mut dyn compress::stream::Decode, d: &[u8], r: &mut Rng) -> Result<Vec<u8>, compress::stream::Error> {
    use compress::stream::{Error, Status};
    let (mut out, mut buf, mut off) = (Vec::new(), vec![0u8; 4096], 0usize);
    for _ in 0..(16 * d.len() + (1 << 16)) {
        let n = piece(r).min(d.len() - off);
        let ob = piece(r).min(buf.len());
        let s = dec.decode(&d[off..off + n], &mut buf[..ob])?;
        assert!(s.read <= n && s.written <= ob, "a decoder took or wrote more than it was given: {s:?} of {n} / {ob}");
        off += s.read;
        out.extend_from_slice(&buf[..s.written]);
        if out.len() > DECODE_LIMIT {
            return Err(Error::Limit);
        }
        let eof = off == d.len();
        match s.status {
            Status::Output => assert!(s.written == ob, "Status::Output with room left: {s:?} of {ob}"),
            Status::Input => {
                assert!(s.read == n, "Status::Input with input left: {s:?} of {n}");
                if eof {
                    dec.finish()?;
                    return Ok(out);
                }
            }
            Status::End if eof => return Ok(out),
            Status::End if s.read == 0 && s.written == 0 => return Err(Error::Corrupt),
            Status::End => {}
        }
    }
    panic!("a stream decoder makes no progress ({} of {} bytes taken)", off, d.len());
}

/// a fuzz target: the stream decoder in two schedules of pieces against the one-shot decoder
/// -- the same output where one-shot decodes the input (or, in pieces, an error), and the two
/// schedules alike.  `window_limit`: the stream decoder refuses a window above its memory
/// limit (zstd: Error::Limit, as the zstd tool does), which one-shot, holding the whole output
/// anyway, does not need to
fn stream_target(new: fn() -> Box<dyn compress::stream::Decode>, oneshot: fn(&[u8], usize) -> Option<Vec<u8>>, window_limit: bool) -> Fun {
    Box::new(move |d: &[u8]| {
        let seed = 0x9E37_79B9_7F4A_7C15 ^ (d.len() as u64) ^ d.iter().take(16).fold(0u64, |h, &b| h.rotate_left(8) ^ b as u64);
        let one = oneshot(d, DECODE_LIMIT);
        let a = pump_pieces(&mut *new(), d, &mut Rng(seed | 1));
        let b = pump_pieces(&mut *new(), d, &mut Rng(seed.rotate_left(17) | 1));
        if window_limit && one.is_some() && a == Err(compress::stream::Error::Limit) {
            assert!(b == Err(compress::stream::Error::Limit), "the window limit in one schedule only");
            return;
        }
        assert!(a.as_ref().ok() == one.as_ref(), "in pieces {:?} bytes, one-shot {:?} bytes", a.as_ref().map(|v| v.len()), one.as_ref().map(|v| v.len()));
        assert!(a.as_ref().ok() == b.as_ref().ok(), "two schedules of pieces differ: {:?} / {:?}", a.as_ref().map(|v| v.len()), b.as_ref().map(|v| v.len()));
    })
}

/// a stream encoder fed in pieces (Flush::Sync at some of them when `sync`), ended with
/// Flush::Finish, output buffers of 1 byte to 4 KiB -> the compressed bytes
fn encode_pieces(enc: &mut dyn compress::stream::Encode, data: &[u8], r: &mut Rng, sync: bool) -> Vec<u8> {
    use compress::stream::{Flush, Status};
    let (mut out, mut buf, mut off) = (Vec::new(), vec![0u8; 4096], 0usize);
    let mut rounds = 0usize;
    loop {
        let n = piece(r).min(data.len() - off);
        let last = off + n == data.len();
        let flush = if last { Flush::Finish } else if sync && r.below(6) == 0 { Flush::Sync } else { Flush::None };
        let mut input = &data[off..off + n];
        loop {
            rounds += 1;
            assert!(rounds < 64 * data.len() + (1 << 16), "a stream encoder makes no progress ({off} of {} bytes)", data.len());
            let ob = piece(r).min(buf.len());
            let s = enc.encode(input, &mut buf[..ob], flush).unwrap_or_else(|e| panic!("encode: {e:?}"));
            assert!(s.read <= input.len() && s.written <= ob, "an encoder took or wrote more than it was given: {s:?}");
            out.extend_from_slice(&buf[..s.written]);
            input = &input[s.read..];
            match s.status {
                Status::Output => {}
                Status::Input if flush == Flush::Finish => {}
                Status::Input => {
                    assert!(input.is_empty(), "Status::Input with input left");
                    break;
                }
                Status::End => {
                    assert!(last && input.is_empty() && flush == Flush::Finish, "Status::End before the end");
                    return out;
                }
            }
        }
        off += n;
    }
}

/// a fuzz target: the first byte picks a format, a level and Sync flushes, the rest is the data
/// -> compress() and the stream encoder (in pieces) give streams the one-shot decoder turns back
/// into the data; without Sync the encoder writes compress()'s bytes where the format says so
/// (DEFLATE, zlib, gzip up to 256 KiB; Brotli, zstd and LZ4 frames given the size)
fn roundtrip(d: &[u8]) {
    let Some((&sel, data)) = d.split_first() else { return };
    let (fmt, lv, sync) = (sel % 6, (sel / 6) as i32 % 21, sel & 0x80 != 0);
    let data = &data[..data.len().min(if fmt == 3 && lv > 9 { 4096 } else { 32 << 10 })];
    let mut r = Rng(0x5DEE_CE66_D ^ data.len() as u64 ^ (sel as u64) << 32);
    let back = |c: &[u8]| -> Option<Vec<u8>> {
        match fmt {
            0 => compress::deflate::decompress(c, data.len()),
            1 => compress::zlib::decompress(c, data.len()),
            2 => compress::gzip::decompress(c, data.len()),
            3 => compress::brotli::decompress(c, data.len()),
            4 => compress::zstd::decompress(c, data.len()),
            _ => compress::lz4::frame_decompress(c, data.len()),
        }
    };
    let (once, mut enc): (Vec<u8>, Box<dyn compress::stream::Encode>) = match fmt {
        0 => (compress::deflate::compress(data, lv % 10), Box::new(compress::deflate::Encoder::new(lv % 10))),
        1 => (compress::zlib::compress(data, lv % 10), Box::new(compress::zlib::Encoder::new(lv % 10))),
        2 => (compress::gzip::compress(data, lv % 10), Box::new(compress::gzip::Encoder::new(lv % 10))),
        3 => (compress::brotli::compress(data, lv % 12), Box::new(compress::brotli::Encoder::new(lv % 12).with_size(data.len() as u64))),
        4 => (compress::zstd::compress(data, 1 + lv % 19), Box::new(compress::zstd::Encoder::new(1 + lv % 19).with_size(data.len() as u64))),
        _ => (compress::lz4::frame_compress(data), Box::new(compress::lz4::FrameEncoder::new().with_size(data.len() as u64))),
    };
    assert!(back(&once).as_deref() == Some(data), "format {fmt} level {lv}: compress() does not come back");
    let pieces = encode_pieces(&mut *enc, data, &mut r, sync);
    assert!(back(&pieces).as_deref() == Some(data), "format {fmt} level {lv} sync {sync}: the stream encoder's bytes do not come back");
    if !sync {
        assert!(pieces == once, "format {fmt} level {lv}: the stream encoder writes other bytes than compress() ({} / {})", pieces.len(), once.len());
    }
}

// ------------------------------------------------------------------ targets

pub type Fun = Box<dyn Fn(&[u8]) + Send + Sync>;

pub fn targets(seed_dir: &str) -> Vec<(&'static str, Vec<Vec<u8>>, Fun)> {
    let (dq, dm) = dns_seed();
    let certs = pem_certs(seed_dir);
    let anchors: Vec<netio::x509::Cert> = certs.iter().filter_map(|c| netio::x509::Cert::parse(c)).take(20).collect();
    let id = tls_ids();
    let tk = psk_tickets();
    let pcfg = psk_server(&tk);
    let (ocsp_leaf, ocsp_ca, ocsp) = ocsp_seeds();
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
        // netpkt's certificates are the crate jflib-x509 as well: what netpkt adds is check_chain
        ("x509_netpkt", certs, Box::new(|d: &[u8]| {
            if let Some(c) = netpkt::x509::Cert::parse(d) {
                let _ = netpkt::x509::check_chain(&[c.clone()], &[c.clone()], "a.lab", 1_800_000_000);
                let _ = netpkt::x509::check_chain(&[c.clone(), c.clone()], &[], "www.example.org", 1_800_000_000);
            }
        })),
        // OCSP responses checked against a certificate and its CA; requests read as a responder does
        ("ocsp", ocsp, Box::new(move |d: &[u8]| {
            let _ = netio::x509::ocsp::check(d, &ocsp_leaf, &ocsp_ca, 1_800_000_000);
            let _ = netio::x509::ocsp::Request::parse(d);
        })),
        ("csr", csr_seeds(), Box::new(|d: &[u8]| {
            if let Some(c) = netio::x509::Csr::parse(d) {
                let _ = (c.dns.len(), c.ips.len(), c.tls_features.len());
            }
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
            let mut s = netio::tls::Conn::server(netio::tls::ServerConfig::new(id.clone()));
            let _ = s.feed(d);
            let _ = s.take_out();
        })),
        // with tickets and 0-RTT: the pre_shared_key, its binders, the tickets, the early data
        // (each input with a replay register of its own, the same keys: a seed's 0-RTT is
        // taken again in every run)
        ("tls_server_psk", psk_hellos(&tk), Box::new(move |d: &[u8]| {
            let mut s = netio::tls::Conn::server(netio::tls::ServerConfig { tickets: Some(psk_tickets()), ..pcfg.clone() });
            let _ = s.feed(d);
            let _ = s.take_out();
            let mut b = [0u8; 64];
            let _ = s.read_early(&mut b);
        })),
        ("tls_client", server_flight(), Box::new(|d: &[u8]| {
            let mut c = netio::tls::Conn::client(netio::tls::ClientConfig::new(netio::tls::Trust::Any, "a.lab"));
            let _ = c.take_out();
            let _ = c.feed(d);
        })),
        ("tls_webclient", server_flight(), Box::new(|d: &[u8]| {
            let mut c = netio::tls::Conn::client(netio::tls::ClientConfig::new(netio::tls::Trust::System(vec![]), "a.lab"));
            let _ = c.take_out();
            let _ = c.feed(d);
        })),
        ("ipv4_reassembly", fragment_seeds(false), Box::new(|d: &[u8]| {
            let mut r = netpkt::ipv4::Reassembly::new();
            for f in split_fragments(d) {
                if let Some(v) = netpkt::ipv4::Ipv4View::parse(f) {
                    let _ = r.add(&v);
                    let _ = r.collected();
                }
            }
        })),
        ("ipv6_reassembly", fragment_seeds(true), Box::new(|d: &[u8]| {
            let mut r = netpkt::ipv6::Reassembly6::new();
            for f in split_fragments(d) {
                if let Some(v) = netpkt::ipv6::Ipv6View::parse(f) {
                    if let Some((fh, at)) = netpkt::ipv6::frag(&v) {
                        let _ = r.add(&v, &fh, at);
                    }
                }
            }
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
        ("deflate", deflate_seeds("deflate"), {
            // a stream with Huffman tables, the part of InflateState a call leaves for the next
            let text: Vec<u8> = (0..2000u32).map(|i| b"the quick brown fox "[(i % 20) as usize] ^ (i / 97) as u8).collect();
            let known = compress::deflate::compress(&text, 9);
            Box::new(move |d: &[u8]| {
                let _ = compress::deflate::decompress(d, 1 << 22);
                // every call is a stream of its own (a small buffer: Overflow too); after any
                // input, decoded or refused, the same state decodes the known stream as a fresh
                // one does
                let mut st = compress::deflate::InflateState::new();
                let mut dst = [0u8; 700];
                let _ = compress::deflate::decompress_into(Some(&mut st), d, &mut dst);
                let (mut again, mut fresh) = (vec![0u8; 4096], vec![0u8; 4096]);
                let a = compress::deflate::decompress_into(Some(&mut st), &known, &mut again);
                let b = compress::deflate::decompress_into(None, &known, &mut fresh);
                assert!(a == b && a == Ok((text.len(), known.len())) && again == fresh, "InflateState after an input: {a:?}, fresh: {b:?}");
            })
        }),
        ("zlib", deflate_seeds("zlib"), Box::new(|d: &[u8]| {
            let _ = compress::zlib::decompress(d, 1 << 22);
            let mut dst = [0u8; 4096];
            let _ = compress::zlib::decompress_into(None, d, &mut dst);
        })),
        ("gzip", deflate_seeds("gzip"), Box::new(|d: &[u8]| {
            let _ = compress::gzip::decompress(d, 1 << 22);
            let mut dst = [0u8; 4096];
            let _ = compress::gzip::decompress_member_into(None, d, &mut dst);
        })),
        ("brotli", brotli_seeds(seed_dir), Box::new(|d: &[u8]| {
            let _ = compress::brotli::decompress(d, 1 << 22);
            let _ = compress::brotli::decompress_prefix(d, 1000);
        })),
        // the stream decoders in pieces against the one-shot ones (stream_target)
        ("stream_deflate", deflate_seeds("deflate"), stream_target(|| Box::new(compress::deflate::Decoder::new()), compress::deflate::decompress, false)),
        ("stream_zlib", deflate_seeds("zlib"), stream_target(|| Box::new(compress::zlib::Decoder::new()), compress::zlib::decompress, false)),
        ("stream_gzip", deflate_seeds("gzip"), stream_target(|| Box::new(compress::gzip::Decoder::new()), compress::gzip::decompress, false)),
        ("stream_brotli", brotli_seeds(seed_dir), stream_target(|| Box::new(compress::brotli::Decoder::new()), compress::brotli::decompress, false)),
        ("stream_zstd", zstd_seeds(seed_dir), stream_target(|| Box::new(compress::zstd::Decoder::new()), compress::zstd::decompress, true)),
        ("stream_lz4", plain_texts().iter().map(|t| compress::lz4::frame_compress(t)).collect(), stream_target(|| Box::new(compress::lz4::FrameDecoder::new()), compress::lz4::frame_decompress, false)),
        // compress() and the stream encoders: back to the data, the same bytes (roundtrip)
        ("roundtrip", plain_texts().iter().enumerate().flat_map(|(i, t)| (0u8..6).map(move |f| {
            let mut v = vec![f + 6 * (3 * i as u8) + if i % 2 == 1 { 0x80 } else { 0 }];
            v.extend_from_slice(&t[..t.len().min(6000)]);
            v
        })).collect(), Box::new(roundtrip)),
        ("json", json_seeds(), Box::new(|d: &[u8]| {
            let _ = formats::json::parse_with_limits(d, formats::json::Limits::default());
            let tight = formats::json::Limits { bytes: 4096, depth: 8, values: 64, string_bytes: 256, members: 16 };
            let _ = formats::json::parse_with_limits(d, tight);
        })),
        ("xml", xml_seeds(), Box::new(|d: &[u8]| {
            // the bytes as text (invalid UTF-8 replaced, so mutations still reach the readers)
            let s = String::from_utf8_lossy(d);
            let _ = formats::xml::parse_with(&s, formats::xml::Limits::default());
            let mut r = formats::xml::Reader::new(&s, 32, 64);
            for _ in 0..100_000 {
                match r.next() {
                    Ok(formats::xml::Event::Eof) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let mut l = formats::xml::Lenient::new(&s);
            let mut out = String::new();
            for _ in 0..100_000 {
                match l.next() {
                    formats::xml::Token::Eof => break,
                    formats::xml::Token::Start { attrs, .. } => {
                        for (_, v) in attrs {
                            out.clear();
                            l.decode(v, &mut out);
                        }
                    }
                    formats::xml::Token::Text(raw) => {
                        out.clear();
                        l.decode(raw, &mut out);
                    }
                    _ => {}
                }
            }
        })),
        ("charsets", charset_seeds(), Box::new(|d: &[u8]| {
            let Some((&i, text)) = d.split_first() else { return };
            let label = CHARSET_LABELS[i as usize % CHARSET_LABELS.len()];
            let _ = formats::encoding::decode(text, 1 << 20);
            let _ = formats::encoding::decode_label(label, text, 1 << 20);
            let _ = formats::encoding::decode_label(label, text, 7);
            let _ = formats::encoding::label(label);
        })),
        ("netchan_packet", nc_flights().0.into_iter().chain(nc_flights().1).collect(), Box::new(|d: &[u8]| {
            if let Some(p) = netchan::packet::parse(d) {
                for pad in [false, true] {
                    let _ = netchan::packet::parse(&p.to_vec(pad));
                }
            }
        })),
        // datagrams from one address to a fresh server (the client's half of a real handshake as seeds)
        ("netchan_server", prefixes(&nc_flights().0), Box::new(|d: &[u8]| {
            let mut s = nc_server();
            for (k, b) in split_fragments(d).into_iter().enumerate() {
                s.datagram(nc_from(), b, NC_NOW + k as u64);
                while s.take_out().is_some() {}
                while s.poll_event().is_some() {}
            }
            s.tick(NC_NOW + 10_000_000);
        })),
        // the server's packets to a client that started (the server's half as seeds)
        ("netchan_client", prefixes(&nc_flights().1), Box::new(|d: &[u8]| {
            let mut cl = nc_client();
            cl.start(NC_NOW);
            while cl.take_out().is_some() {}
            for (k, b) in split_fragments(d).into_iter().enumerate() {
                cl.packet(b, NC_NOW + k as u64);
                while cl.take_out().is_some() {}
                while cl.poll_event().is_some() {}
            }
            cl.tick(NC_NOW + 10_000_000);
        })),
        ("netchan_open", nc_sealed(), Box::new(|d: &[u8]| {
            let Some((&kind, wire)) = d.split_first() else { return };
            let (_, mut b) = nc_conns();
            let head = if kind & 1 == 1 { netchan::packet::DISCONNECT_HEADER } else { netchan::packet::DATA_HEADER };
            let seq = if wire.len() >= 7 { u16::from_le_bytes([wire[5], wire[6]]) } else { 0 };
            let _ = b.open(wire, head, seq);
        })),
        ("plat_clipboard", clipboard_seeds(), Box::new(|d: &[u8]| {
            let _ = plat::fuzz::clip_wide_text(d);
            let _ = plat::fuzz::hdrop(d);
            let _ = plat::clip_html_decode(d);
            let _ = plat::clip_dib_to_bmp(d);
            let _ = plat::clip_bmp_fix(d);
            let _ = plat::clipimg_split(d);
            let _ = plat::drop_uri_list(d);
        })),
    ]
    .into_iter()
    .chain(linux_targets())
    .collect()
}


static SET: OnceLock<Vec<(&'static str, Vec<Vec<u8>>, Fun)>> = OnceLock::new();

/// the run function of a target (the set is built on the first call)
pub fn get(name: &str) -> &'static Fun {
    let set = SET.get_or_init(|| targets(&std::env::var("NET_FUZZ_SEEDS").unwrap_or_default()));
    &set.iter().find(|t| t.0 == name).unwrap_or_else(|| panic!("no target {}", name)).2
}
