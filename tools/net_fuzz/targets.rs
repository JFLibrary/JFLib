// net_fuzz targets -- the parsers of untrusted bytes, their seeds and the code that feeds them
// (compress, formats, img_jp2, img_jbig2, netchan, netio, netpkt, plat).
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

/// why pump_pieces stopped without the stream: the decoder refused it (or, for a format of one
/// stream, the input after it), or the output grew past DECODE_LIMIT -- the pump's own limit,
/// kept apart from a decoder's Error::Limit (zstd's window)
#[derive(Debug, PartialEq)]
enum Stop {
    Dec(compress::stream::Error),
    Over,
}

/// a stream decoder fed in pieces of the schedule's sizes, with output buffers of 1 byte to 4
/// KiB, the way stream::decode_all pumps it (input after a stream goes on with the next
/// member or frame; a format of one stream refuses it) -> the output, or why it stopped
fn pump_pieces(dec: &mut dyn compress::stream::Decode, d: &[u8], r: &mut Rng) -> Result<Vec<u8>, Stop> {
    use compress::stream::{Error, Status};
    let (mut out, mut buf, mut off) = (Vec::new(), vec![0u8; 4096], 0usize);
    for _ in 0..(16 * d.len() + (1 << 16)) {
        let n = piece(r).min(d.len() - off);
        let ob = piece(r).min(buf.len());
        let s = dec.decode(&d[off..off + n], &mut buf[..ob]).map_err(Stop::Dec)?;
        assert!(s.read <= n && s.written <= ob, "a decoder took or wrote more than it was given: {s:?} of {n} / {ob}");
        off += s.read;
        out.extend_from_slice(&buf[..s.written]);
        if out.len() > DECODE_LIMIT {
            return Err(Stop::Over);
        }
        let eof = off == d.len();
        match s.status {
            Status::Output => assert!(s.written == ob, "Status::Output with room left: {s:?} of {ob}"),
            Status::Input => {
                assert!(s.read == n, "Status::Input with input left: {s:?} of {n}");
                if eof {
                    dec.finish().map_err(Stop::Dec)?;
                    return Ok(out);
                }
            }
            Status::End if eof => return Ok(out),
            Status::End if s.read == 0 && s.written == 0 => return Err(Stop::Dec(Error::Corrupt)),
            Status::End => {}
        }
    }
    panic!("a stream decoder makes no progress ({} of {} bytes taken)", off, d.len());
}

/// a fuzz target: the stream decoder in two schedules of pieces against the one-shot decoder
/// -- the same output where one-shot decodes the input (or, in pieces, an error), and the two
/// schedules alike.  `window_limit`: the stream decoder refuses a window above its memory
/// limit (zstd: Error::Limit, as the zstd tool does), which one-shot, holding the whole output
/// anyway, does not need to.  Output past DECODE_LIMIT in pieces where one-shot decodes within
/// it is a finding, like any other difference
fn stream_target(new: fn() -> Box<dyn compress::stream::Decode>, oneshot: fn(&[u8], usize) -> Option<Vec<u8>>, window_limit: bool) -> Fun {
    Box::new(move |d: &[u8]| {
        let seed = 0x9E37_79B9_7F4A_7C15 ^ (d.len() as u64) ^ d.iter().take(16).fold(0u64, |h, &b| h.rotate_left(8) ^ b as u64);
        let one = oneshot(d, DECODE_LIMIT);
        let a = pump_pieces(&mut *new(), d, &mut Rng(seed | 1));
        let b = pump_pieces(&mut *new(), d, &mut Rng(seed.rotate_left(17) | 1));
        let window = Err(Stop::Dec(compress::stream::Error::Limit));
        if window_limit && one.is_some() && a == window {
            assert!(b == window, "the window limit in one schedule only");
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
                Status::Output => assert!(s.written == ob, "Status::Output with room left: {s:?} of {ob}"),
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

// ------------------------------------------------------------------ the HTTP client, URLs, cookies

/// answers as servers send them: lengths, chunks, interim answers, cookies, redirects, bodies in
/// gzip, br and zstd
fn client_seeds() -> Vec<Vec<u8>> {
    let with = |head: &str, body: &[u8]| {
        let mut v = head.as_bytes().to_vec();
        v.extend_from_slice(body);
        v
    };
    let gz = compress::gzip::compress(b"hello hello hello, a body in gzip", 6);
    let br = compress::brotli::compress(b"hello hello hello, a body in br", 5);
    let zs = compress::zstd::compress(b"hello hello hello, a body in zstd", 3);
    vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nSet-Cookie: a=1; Path=/; Domain=example.org; Secure\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nX: y\r\n\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: ../x?y#z\r\nSet-Cookie: b=2; Max-Age=60; SameSite=Lax\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.0 200 OK\r\n\r\nto the end".to_vec(),
        with(&format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n", gz.len()), &gz),
        with(&format!("HTTP/1.1 200 OK\r\nContent-Encoding: br\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", br.len()), &[&br[..], b"\r\n0\r\n\r\n"].concat()),
        with("HTTP/1.1 200 OK\r\nContent-Encoding: zstd\r\n\r\n", &zs),
    ]
}

/// a URL on the first line, a reference or a NO_PROXY list on the second
fn url_seeds() -> Vec<Vec<u8>> {
    [
        "http://a.example/b/c?d=1\n../x?y",
        "https://[2001:db8::1]:8443/p;q\n//other.example:81/x",
        "http://[fe80::1%25eth0]/\n?only",
        "http://u:p%40w@proxy.lan:3128\nlocalhost, .corp.example, 10.0.0.0/8, [::1]:80, *",
        "http://web.corp.example:8080/x\nlocalhost, .corp.example:8080, 10.0.0.0/8, [2001:db8::1]:80, [::2]:x",
        "HTTP://H:/%7e?%41#f\nhttps://x/./a/../b",
        "http://10.0.0.1/ä\"<>`{}\n#frag",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

/// Set-Cookie lines (one per line), which are also taken as host names for the suffix list
fn cookie_seeds() -> Vec<Vec<u8>> {
    [
        "a=1; Path=/; Domain=example.co.uk; Secure; HttpOnly; SameSite=Strict\nb=2; Max-Age=0\nc=3; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
        "x=1; Domain=github.io\ny=2; Domain=.www.ck\nz=3; Domain=city.kobe.jp.\nshop.xn--fiqs8s",
        "__Host-id=1; Path=/; Secure\n=bare\nname=\"quoted\"; SameSite=None; Secure\na.b.example.co.uk",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect()
}

// ------------------------------------------------------------------ images (img_jp2, img_jbig2)

/// the pixel cap of img_jbig2: a page or region of at most DECODE_LIMIT pixels, a stream's work at
/// most four times that plus 2^22 pixels (a byte each); its slowest inputs, regions that spend
/// all of that, take some 50 ms here and 0.7 s under cov_fuzz's instrumentation
const JBIG2_PIXELS: u64 = DECODE_LIMIT as u64;

/// the pixel cap of img_jp2: an image of at most 2^20 pixels, four times that in samples (16 MB
/// of i32).  A pixel costs about a hundred times as much under cov_fuzz's instrumentation as here
/// (a 520-byte file of 33 x 65052 pixels in 8133 empty tiles: 30 ms here, 3 to 20 s there, past
/// libFuzzer's 10 s), so DECODE_LIMIT pixels would time out on files of no fault
const JP2_PIXELS: u64 = 1 << 20;

/// a format's codec as img_all calls it: the probe, then the decode into an ARGB surface under the
/// pixel cap `cap`; a decoded surface holds its rows and keeps to the cap
fn img_codec(c: &img_core::Codec, d: &[u8], cap: u64) {
    let (Some(probe), Some(decode)) = (c.probe, c.decode) else { return };
    if !probe(d, &mut img_core::ImgMeta::default()) {
        return;
    }
    let mut sf = img_core::Surface::default();
    let dec = img_core::ImgDec { maxpx: cap, ..Default::default() };
    if decode(d, &mut sf, &dec, &mut img_core::ImgMeta::default()) == 0 {
        assert!(sf.w >= 0 && sf.h >= 0 && sf.stride >= sf.w.saturating_mul(4), "a surface of {} x {}, stride {}", sf.w, sf.h, sf.stride);
        assert!(sf.px.len() >= sf.stride as usize * sf.h as usize, "a surface shorter than its rows");
        assert!(sf.w as u64 * sf.h as u64 <= cap, "a surface of {} x {} past the cap", sf.w, sf.h);
    }
}

// JPEG 2000 images of apps/pdftest/src/rdraw.rs (`python tools/jp2_check.py --fixtures`: Pillow
// with OpenJPEG 2.5): 8 x 8 (the lossy one 16 x 16) RGB, grey and RGBA, a 4-bit codestream and
// 2-bit grey under 8-bit opacity
const J2K_RGB: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5D, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xFF, 0xD9];
const JP2_RGB: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x03, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0xD8, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5D, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xFF, 0xD9];
const JP2_GRAY: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x9B, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x26, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x30, 0x11, 0x11, 0xFA, 0xAE, 0xD9, 0xCA, 0xF1, 0xBB, 0xB6, 0xA8, 0xE2, 0xC1, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xFF, 0xD9];
const JP2_RGBA: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x4F, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x04, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x22, 0x63, 0x64, 0x65, 0x66, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF5, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x77, 0x00, 0x01, 0xFF, 0x93, 0xCF, 0xB4, 0x3C, 0x10, 0xCC, 0x4E, 0x0C, 0x04, 0xEC, 0x3D, 0x61, 0xE6, 0x13, 0x9C, 0x5C, 0x52, 0x33, 0x0F, 0xCF, 0xB4, 0x38, 0x06, 0x31, 0x3A, 0xE1, 0xF6, 0xBC, 0x75, 0x91, 0x87, 0x01, 0xE9, 0x44, 0x92, 0x7F, 0xCF, 0xB4, 0x38, 0x13, 0x8C, 0x45, 0x30, 0x14, 0x4D, 0x30, 0x11, 0x36, 0x09, 0x0C, 0x00, 0x02, 0x9F, 0xDF, 0x80, 0x88, 0x0E, 0xCA, 0x2E, 0x01, 0xF9, 0x74, 0xBC, 0x2C, 0xF8, 0x7C, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xA3, 0xED, 0x05, 0x00, 0x0C, 0x78, 0x3B, 0x2E, 0x9F, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0x97, 0x3C, 0x5B, 0xA1, 0xF5, 0x02, 0x80, 0x0F, 0x27, 0xA7, 0x14, 0x7F, 0xA7, 0xE0, 0x06, 0x0F, 0x28, 0x5F, 0xFF, 0xD9];
const JP2_LOSSY: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x2D, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x03, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0xAF, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x04, 0x04, 0x00, 0x00, 0xFF, 0x5C, 0x00, 0x11, 0x42, 0x5F, 0x52, 0x50, 0x05, 0x50, 0x05, 0x50, 0x47, 0x57, 0xD3, 0x57, 0xD3, 0x57, 0x62, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00, 0x01, 0xFF, 0x93, 0xC4, 0xA0, 0x10, 0xCC, 0x4E, 0x0A, 0xDF, 0xC7, 0x85, 0x00, 0xC8, 0xAA, 0x22, 0xB2, 0xC7, 0x46, 0x13, 0x8C, 0x45, 0x30, 0x28, 0x2C, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xFF, 0xD9];
const J2K_GRAY4: &[u8] = &[0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x03, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1B, 0x00, 0x01, 0xFF, 0x93, 0xC0, 0xF8, 0x44, 0x10, 0xDC, 0x38, 0x3D, 0xA0, 0x1D, 0x0C, 0x0C, 0x78, 0x3F, 0xFF, 0xD9];
const JP2_GRAY2_ALPHA: &[u8] = &[0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00, 0x00, 0x14, 0x66, 0x74, 0x79, 0x70, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x00, 0x6A, 0x70, 0x32, 0x20, 0x00, 0x00, 0x00, 0x43, 0x6A, 0x70, 0x32, 0x68, 0x00, 0x00, 0x00, 0x16, 0x69, 0x68, 0x64, 0x72, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x02, 0x07, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x63, 0x6F, 0x6C, 0x72, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x16, 0x63, 0x64, 0x65, 0x66, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x94, 0x6A, 0x70, 0x32, 0x63, 0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x01, 0x01, 0x07, 0x01, 0x01, 0xFF, 0x52, 0x00, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xFF, 0x5C, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xFF, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61, 0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4F, 0x70, 0x65, 0x6E, 0x4A, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6F, 0x6E, 0x20, 0x32, 0x2E, 0x35, 0x2E, 0x34, 0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1C, 0x00, 0x01, 0xFF, 0x93, 0xC0, 0x23, 0x11, 0x50, 0x3F, 0xCF, 0xB4, 0x10, 0x14, 0x00, 0x51, 0x9F, 0x80, 0x80, 0xFF, 0xD9];

/// a box of the JP2 file format (T.800 I.4)
fn jp2_box(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(ty);
    v.extend_from_slice(body);
    v
}

/// a JP2 file around codestream `cs`: the signature, the file type, the header (an image header
/// of the codestream's SIZ, then the boxes `head`) and the codestream
fn jp2_file(cs: &[u8], head: &[Vec<u8>]) -> Vec<u8> {
    let be = |o: usize| u32::from_be_bytes([cs[o], cs[o + 1], cs[o + 2], cs[o + 3]]);
    let mut ihdr = (be(12) - be(20)).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&(be(8) - be(16)).to_be_bytes());
    ihdr.extend_from_slice(&cs[40..43]);
    ihdr.extend_from_slice(&[7, 0, 0]);
    let mut jp2h = jp2_box(b"ihdr", &ihdr);
    for b in head {
        jp2h.extend_from_slice(b);
    }
    let sig = [0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];
    [&sig[..], &jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 "), &jp2_box(b"jp2h", &jp2h), &jp2_box(b"jp2c", cs)].concat()
}

/// an enumerated colour space box (16 sRGB, 17 grey, 18 sYCC, 12 CMYK, 14 CIELab)
fn jp2_colr(cs: u32) -> Vec<u8> {
    let mut b = vec![1, 0, 0];
    b.extend_from_slice(&cs.to_be_bytes());
    if cs == 14 {
        // CIELab's parameters (JPX): ranges, offsets, illuminant
        b.extend_from_slice(&[0, 0, 0, 100, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 128, 0, 0, 0, 128, 0x00, 0x44, 0x35, 0x30]);
    }
    jp2_box(b"colr", &b)
}

/// JPEG 2000: the images of rdraw.rs, JP2 headers made here around them (a palette with its
/// component mapping, an ICC profile, channel definitions in another order and with opacity,
/// sYCC, CIELab, boxes of 64-bit and of open length), and the files tools/net_fuzz.py writes into
/// the seed folder with Pillow (OpenJPEG) where it is there: tiles, precincts, code-block sizes,
/// layers, the progressions, the 9/7 wavelet, 16 bits, PLT
fn jp2_seeds(dir: &str) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = [J2K_RGB, JP2_RGB, JP2_GRAY, JP2_RGBA, JP2_LOSSY, J2K_GRAY4, JP2_GRAY2_ALPHA].iter().map(|s| s.to_vec()).collect();
    // a palette of 16 entries (three 8-bit columns) that the 4-bit component goes through
    let mut pclr = vec![0, 16, 3, 7, 7, 7];
    for i in 0..16u8 {
        pclr.extend_from_slice(&[i * 16, 255 - i * 16, i * 5]);
    }
    let cmap = [0u8, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 2];
    out.push(jp2_file(J2K_GRAY4, &[jp2_colr(16), jp2_box(b"pclr", &pclr), jp2_box(b"cmap", &cmap)]));
    // an ICC profile (only carried along), the channels as blue, green, red
    let mut icc = vec![2, 0, 0];
    icc.extend_from_slice(&132u32.to_be_bytes());
    icc.extend_from_slice(&[0u8; 32]);
    icc.extend_from_slice(b"acsp");
    icc.extend(std::iter::repeat_n(0u8, 92));
    let bgr = [0u8, 3, 0, 0, 0, 0, 0, 3, 0, 1, 0, 0, 0, 2, 0, 2, 0, 0, 0, 1];
    out.push(jp2_file(J2K_RGB, &[jp2_box(b"colr", &icc), jp2_box(b"cdef", &bgr)]));
    // the third channel as opacity, premultiplied (type 2), of the whole image (association 0)
    let alpha = [0u8, 3, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 2, 0, 2, 0, 2, 0, 0];
    out.push(jp2_file(J2K_RGB, &[jp2_colr(16), jp2_box(b"cdef", &alpha)]));
    for cs in [18, 14, 12, 17] {
        out.push(jp2_file(J2K_RGB, &[jp2_colr(cs)]));
    }
    // the codestream box with a 64-bit length, and one of open length (0: to the end)
    let plain = jp2_file(J2K_RGB, &[jp2_colr(16)]);
    let at = plain.len() - J2K_RGB.len() - 8;
    let mut xl = plain[..at].to_vec();
    xl.extend_from_slice(&1u32.to_be_bytes());
    xl.extend_from_slice(b"jp2c");
    xl.extend_from_slice(&(J2K_RGB.len() as u64 + 16).to_be_bytes());
    xl.extend_from_slice(J2K_RGB);
    out.push(xl);
    let mut open = plain.clone();
    open[at..at + 4].copy_from_slice(&[0; 4]);
    out.push(open);
    // Pillow's files (tools/net_fuzz.py)
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jp2" || x == "j2k")).collect();
        files.sort();
        for p in files {
            if let Ok(b) = std::fs::read(p) {
                out.push(b);
            }
        }
    }
    out
}

/// a JBIG2 segment (T.88 7.2) as (header, data): its number (below 256), type (0x40 set: a
/// four-byte page association), page association, the segments it refers to; `len` None: the
/// data's length (Some(0xFFFFFFFF): unknown, 7.2.7)
fn jb2_seg(num: u32, ty: u8, page: u32, refs: &[u8], data: Vec<u8>, len: Option<u32>) -> (Vec<u8>, Vec<u8>) {
    let mut h = num.to_be_bytes().to_vec();
    h.push(ty);
    h.push((refs.len() as u8) << 5);
    h.extend_from_slice(refs);
    if ty & 0x40 != 0 {
        h.extend_from_slice(&page.to_be_bytes());
    } else {
        h.push(page as u8);
    }
    h.extend_from_slice(&len.unwrap_or(data.len() as u32).to_be_bytes());
    (h, data)
}

/// the input of the target for PDF's embedded form: the length of the JBIG2Globals (two bytes),
/// the globals' segments, the page's
fn jb2_pdf(globals: &[(Vec<u8>, Vec<u8>)], page: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let g: Vec<u8> = globals.iter().flat_map(|(h, d)| h.iter().chain(d)).copied().collect();
    let mut v = (g.len() as u16).to_be_bytes().to_vec();
    v.extend_from_slice(&g);
    v.extend(page.iter().flat_map(|(h, d)| h.iter().chain(d)));
    v
}

/// a JBIG2 file (D.4): sequential (each header and its data) or random-access (the headers, then
/// the data; an end of file segment ends the headers), with the number of pages
fn jb2_file(segs: &[(Vec<u8>, Vec<u8>)], random: bool) -> Vec<u8> {
    let mut v = vec![0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A, if random { 0 } else { 1 }];
    let pages = segs.iter().filter(|(h, _)| h[4] & 0x3F == 48).count() as u32;
    v.extend_from_slice(&pages.to_be_bytes());
    if random {
        v.extend(segs.iter().flat_map(|(h, _)| h.iter()));
        v.extend(segs.iter().flat_map(|(_, d)| d.iter()));
    } else {
        v.extend(segs.iter().flat_map(|(h, d)| h.iter().chain(d)));
    }
    v
}

/// page information (7.4.8): w x h (0xFFFFFFFF: unknown), its flags and striping
fn jb2_info(w: u32, h: u32, flags: u8, striping: u16) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [w, h, 0, 0] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.push(flags);
    v.extend_from_slice(&striping.to_be_bytes());
    v
}

/// region segment information (7.4.1): w x h at (x, y), the combination operator; then `rest`
fn jb2_region(w: u32, h: u32, x: i32, y: i32, op: u8, rest: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [w, h, x as u32, y as u32] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.push(op);
    for r in rest {
        v.extend_from_slice(r);
    }
    v
}

/// the nominal adaptive pixels of a generic region's template (6.2.5.3)
fn jb2_at(template: u8) -> &'static [u8] {
    match template {
        0 => &[3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE],
        1 => &[3, 0xFF],
        _ => &[2, 0xFF],
    }
}

/// a code table segment (B.2) of lines of (prefix length, range length) from 0, a lower and an
/// upper range line, 3-bit prefix and 4-bit range lengths
fn jb2_table(lines: &[(u32, u32)], lower: u32, upper: u32) -> Vec<u8> {
    let mut v = vec![0x34];
    let high: u32 = lines.iter().map(|l| 1 << l.1).sum();
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&high.to_be_bytes());
    let mut bits: Vec<bool> = Vec::new();
    let mut put = |x: u32, n: u32| bits.extend((0..n).rev().map(|i| x >> i & 1 == 1));
    for &(p, r) in lines {
        put(p, 3);
        put(r, 4);
    }
    put(lower, 3);
    put(upper, 3);
    v.extend(bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |b, (i, &x)| b | (x as u8) << (7 - i))));
    v
}

// JBIG2 encoding for the seeds: symbol dictionaries, text regions and halftones whose arithmetic
// (or Huffman) data decodes, where random bytes almost never do (one dictionary in 10^4)

/// T.88 Table E.1: Qe, the next index after an MPS and after an LPS, whether an LPS swaps the MPS
const MQ_QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true), (0x3401, 2, 6, false), (0x1801, 3, 9, false), (0x0AC1, 4, 12, false), (0x0521, 5, 29, false),
    (0x0221, 38, 33, false), (0x5601, 7, 6, true), (0x5401, 8, 14, false), (0x4801, 9, 14, false), (0x3801, 10, 14, false),
    (0x3001, 11, 17, false), (0x2401, 12, 18, false), (0x1C01, 13, 20, false), (0x1601, 29, 21, false), (0x5601, 15, 14, true),
    (0x5401, 16, 14, false), (0x5101, 17, 15, false), (0x4801, 18, 16, false), (0x3801, 19, 17, false), (0x3401, 20, 18, false),
    (0x3001, 21, 19, false), (0x2801, 22, 19, false), (0x2401, 23, 20, false), (0x2201, 24, 21, false), (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false), (0x1601, 27, 24, false), (0x1401, 28, 25, false), (0x1201, 29, 26, false), (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false), (0x09C1, 32, 29, false), (0x08A1, 33, 30, false), (0x0521, 34, 31, false), (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false), (0x0221, 37, 34, false), (0x0141, 38, 35, false), (0x0111, 39, 36, false), (0x0085, 40, 37, false),
    (0x0049, 41, 38, false), (0x0025, 42, 39, false), (0x0015, 43, 40, false), (0x0009, 44, 41, false), (0x0005, 45, 42, false),
    (0x0001, 45, 43, false), (0x5601, 46, 46, false),
];

/// a context of the coder: its index into MQ_QE and its more probable symbol
type MqCx = (u8, u8);

/// the MQ encoder (T.88 E.2); out[0] is the byte before the data
struct MqEnc {
    a: u32,
    c: u32,
    ct: u32,
    out: Vec<u8>,
}

impl MqEnc {
    fn new() -> MqEnc {
        MqEnc { a: 0x8000, c: 0, ct: 12, out: vec![0] }
    }

    fn encode(&mut self, cx: &mut MqCx, d: u8) {
        let (qe, nmps, nlps, switch) = MQ_QE[cx.0 as usize];
        self.a -= qe;
        if d == cx.1 {
            if self.a & 0x8000 != 0 {
                self.c += qe;
                return;
            }
            if self.a < qe { self.a = qe } else { self.c += qe }
            cx.0 = nmps;
        } else {
            if self.a < qe { self.c += qe } else { self.a = qe }
            if switch {
                cx.1 ^= 1;
            }
            cx.0 = nlps;
        }
        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
            }
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    fn byte_out(&mut self) {
        let last = self.out.len() - 1;
        if self.out[last] != 0xFF && self.c >= 0x800_0000 {
            self.out[last] += 1;
            self.c &= 0x7FF_FFFF;
        }
        if self.out[last] == 0xFF {
            self.out.push((self.c >> 20) as u8);
            self.c &= 0xF_FFFF;
            self.ct = 7;
        } else {
            self.out.push((self.c >> 19) as u8);
            self.c &= 0x7_FFFF;
            self.ct = 8;
        }
    }

    /// FLUSH (E.2.9) and the marker 0xFF 0xAC: the data
    fn finish(mut self) -> Vec<u8> {
        let t = self.c + self.a;
        self.c |= 0xFFFF;
        if self.c >= t {
            self.c -= 0x8000;
        }
        self.c <<= self.ct;
        self.byte_out();
        self.c <<= self.ct;
        self.byte_out();
        if self.out.last() != Some(&0xFF) {
            self.out.push(0xFF);
        }
        self.out.push(0xAC);
        self.out.split_off(1)
    }
}

/// A.2: an integer (None: OOB) in the 512 contexts of one procedure (IADH, IADW, ...)
fn mq_int(e: &mut MqEnc, cx: &mut [MqCx], v: Option<i32>) {
    let mut prev = 1usize;
    let mut bit = |e: &mut MqEnc, d: u32| {
        e.encode(&mut cx[prev], d as u8);
        prev = if prev < 256 { (prev << 1) | d as usize } else { (((prev << 1) | d as usize) & 511) | 256 };
    };
    let (s, a) = v.map_or((1, 0), |v| ((v < 0) as u32, v.unsigned_abs()));
    let ranges: [(u32, u32, u32, &[u32]); 6] = [(4, 2, 0, &[0]), (20, 4, 4, &[1, 0]), (84, 6, 20, &[1, 1, 0]), (340, 8, 84, &[1, 1, 1, 0]), (4436, 12, 340, &[1, 1, 1, 1, 0]), (u32::MAX, 32, 4436, &[1, 1, 1, 1, 1])];
    let &(_, n, off, prefix) = ranges.iter().find(|r| a < r.0).unwrap();
    bit(e, s);
    for &p in prefix {
        bit(e, p);
    }
    for i in (0..n).rev() {
        bit(e, (a - off) >> i & 1);
    }
}

/// A.3: a symbol id of `len` bits (IAID, 2^(len+1) contexts)
fn mq_id(e: &mut MqEnc, cx: &mut [MqCx], id: u32, len: u32) {
    let mut prev = 1usize;
    for i in (0..len).rev() {
        let d = id >> i & 1;
        e.encode(&mut cx[prev], d as u8);
        prev = (prev << 1) | d as usize;
    }
}

/// 6.2.5.3: the context of pixel (x, y) of a w-wide bitmap under template `t` with the adaptive
/// pixels `at` -- the numbering of img_jbig2's generic.rs (the standard's figures: bit 0 the pixel
/// to the left, the rows above as windows, the adaptive pixels at their fixed bits)
fn gb_context(b: &[u8], w: usize, x: usize, y: usize, t: u8, at: &[(i32, i32)]) -> usize {
    let p = |dx: i64, dy: i64| -> usize {
        let (xx, yy) = (x as i64 + dx, y as i64 + dy);
        if xx < 0 || yy < 0 || xx >= w as i64 || dy > 0 || (dy == 0 && dx >= 0) { 0 } else { b[yy as usize * w + xx as usize] as usize }
    };
    let (up2, up1, own, bits): (Option<(i64, i64, u32)>, (i64, i64, u32), u32, &[u32]) = match t {
        0 => (Some((-1, 1, 12)), (-2, 2, 5), 4, &[4, 10, 11, 15]),
        1 => (Some((-1, 2, 9)), (-2, 2, 4), 3, &[3]),
        2 => (Some((-1, 1, 7)), (-2, 1, 3), 2, &[2]),
        _ => (None, (-3, 1, 5), 4, &[4]),
    };
    let mut c = 0usize;
    for k in 0..own {
        c |= p(-(k as i64) - 1, 0) << k;
    }
    for (dy, (lo, hi, s)) in [(-1, Some(up1)), (-2, up2)].into_iter().filter_map(|(dy, r)| r.map(|r| (dy, r))) {
        for dx in lo..=hi {
            c |= p(dx, dy) << (s + (hi - dx) as u32);
        }
    }
    for (&(ax, ay), &bit) in at.iter().zip(bits) {
        c |= p(ax as i64, ay as i64) << bit;
    }
    c
}

/// 6.2.5: a generic region (no TPGDON); the pixels `skip` marks are not coded (they are 0)
fn mq_generic(e: &mut MqEnc, cx: &mut [MqCx], b: &[u8], w: usize, h: usize, t: u8, at: &[(i32, i32)], skip: Option<&[u8]>) {
    for y in 0..h {
        for x in 0..w {
            if skip.is_some_and(|s| s[y * w + x] == 1) {
                continue;
            }
            let c = gb_context(b, w, x, y, t, at);
            e.encode(&mut cx[c], b[y * w + x]);
        }
    }
}

/// the nominal adaptive pixels of template 0
const AT0: [(i32, i32); 4] = [(3, -1), (-3, -1), (2, -2), (-2, -2)];

/// a symbol of w x h: a frame on the left and the bottom, a diagonal pattern inside (0 / 1)
fn glyph(w: usize, h: usize, k: usize) -> Vec<u8> {
    (0..w * h).map(|i| (i % w == 0 || i / w == h - 1 || (i % w * 3 + i / w * 5 + k) % 4 == 0) as u8).collect()
}

/// symbols in height classes: (height, the widths of its symbols); arithmetic coding takes a
/// width smaller than the one before, B.2 (Huffman) only larger ones
type Classes = [(usize, &'static [usize]); 2];
const CLASSES_MQ: Classes = [(5, &[4, 6]), (8, &[3, 7, 5])];
const CLASSES_HUFF: Classes = [(5, &[4, 6]), (8, &[3, 5, 7])];

/// the sizes of the glyphs of `classes`, in their order
fn class_sizes(classes: &Classes) -> Vec<(usize, usize)> {
    classes.iter().flat_map(|&(h, ws)| ws.iter().map(move |&w| (w, h))).collect()
}

/// 6.5 (arithmetic, template 0, no refinement): a symbol dictionary of the glyphs of `classes`
/// after `inputs` symbols of the dictionaries it refers to, the new ones exported; the generic
/// contexts start from `gb` and end there (flags 0x100: contexts used, 0x200: retained)
fn jb2_symbols_mq(classes: &Classes, flags: u16, inputs: usize, gb: &mut [MqCx]) -> Vec<u8> {
    let n: usize = classes.iter().map(|c| c.1.len()).sum();
    let mut d = flags.to_be_bytes().to_vec();
    d.extend_from_slice(jb2_at(0));
    d.extend_from_slice(&(n as u32).to_be_bytes());
    d.extend_from_slice(&(n as u32).to_be_bytes());
    let mut e = MqEnc::new();
    let (mut iadh, mut iadw, mut iaex) = ([(0u8, 0u8); 512], [(0u8, 0u8); 512], [(0u8, 0u8); 512]);
    let (mut height, mut k) = (0, 0);
    for &(h, widths) in classes {
        mq_int(&mut e, &mut iadh, Some(h as i32 - height));
        height = h as i32;
        let mut width = 0;
        for &w in widths {
            mq_int(&mut e, &mut iadw, Some(w as i32 - width));
            width = w as i32;
            mq_generic(&mut e, gb, &glyph(w, h, k), w, h, 0, &AT0, None);
            k += 1;
        }
        mq_int(&mut e, &mut iadw, None);
    }
    // the export flags: a run of the input not exported, then the new symbols
    mq_int(&mut e, &mut iaex, Some(inputs as i32));
    mq_int(&mut e, &mut iaex, Some(n as i32));
    d.extend_from_slice(&e.finish());
    d
}

/// bits, most significant first (Huffman-coded data)
struct BitW(Vec<u8>, usize);

impl BitW {
    fn put(&mut self, x: u32, n: u32) {
        for i in (0..n).rev() {
            if self.1 % 8 == 0 {
                self.0.push(0);
            }
            if x >> i & 1 == 1 {
                *self.0.last_mut().unwrap() |= 0x80 >> (self.1 % 8);
            }
            self.1 += 1;
        }
    }

    /// on to the next byte
    fn align(&mut self) {
        self.1 = self.0.len() * 8;
    }
}

/// a value of the standard Huffman tables B.1 (k 1), B.2 (k 2, None: OOB) and B.4 (k 4): (its
/// lowest value, prefix, prefix length, range bits) per line, the last one the upper range
fn huff_std(b: &mut BitW, k: u8, v: Option<u32>) {
    let lines: &[(u32, u32, u32, u32)] = match k {
        1 => &[(0, 0, 1, 4), (16, 2, 2, 8), (272, 6, 3, 16), (65808, 7, 3, 32)],
        2 => &[(0, 0, 1, 0), (1, 2, 2, 0), (2, 6, 3, 0), (3, 14, 4, 3), (11, 30, 5, 6), (75, 62, 6, 32)],
        _ => &[(1, 0, 1, 0), (2, 2, 2, 0), (3, 6, 3, 0), (4, 14, 4, 3), (12, 30, 5, 6), (76, 31, 5, 32)],
    };
    let Some(v) = v else { return b.put(63, 6) };
    let &(low, code, len, bits) = lines.iter().rev().find(|l| v >= l.0).unwrap();
    b.put(code, len);
    b.put(v - low, bits);
}

/// 6.5 (Huffman: tables B.4, B.2 and B.1, collective bitmaps uncompressed): a symbol dictionary
/// of the glyphs of `classes` (widths growing in each class), all exported
fn jb2_symbols_huff(classes: &Classes) -> Vec<u8> {
    let n: usize = classes.iter().map(|c| c.1.len()).sum();
    let mut d = 1u16.to_be_bytes().to_vec();
    d.extend_from_slice(&(n as u32).to_be_bytes());
    d.extend_from_slice(&(n as u32).to_be_bytes());
    let mut b = BitW(Vec::new(), 0);
    let (mut height, mut k) = (0, 0);
    for &(h, widths) in classes {
        huff_std(&mut b, 4, Some((h - height) as u32));
        height = h;
        let mut width = 0;
        for &w in widths {
            huff_std(&mut b, 2, Some((w - width) as u32));
            width = w;
        }
        huff_std(&mut b, 2, None);
        // BMSIZE 0: the collective bitmap uncompressed from the next byte, its rows padded
        huff_std(&mut b, 1, Some(0));
        let glyphs: Vec<Vec<u8>> = widths.iter().enumerate().map(|(i, &w)| glyph(w, h, k + i)).collect();
        k += widths.len();
        for y in 0..h {
            b.align();
            for (g, &w) in glyphs.iter().zip(widths) {
                for x in 0..w {
                    b.put(g[y * w + x] as u32, 1);
                }
            }
        }
        b.align();
    }
    huff_std(&mut b, 1, Some(0));
    huff_std(&mut b, 1, Some(n as u32));
    d.extend_from_slice(&b.0);
    d
}

/// 6.4 (arithmetic, top left, no transposition): a text region of w x h of the instances
/// (symbol id, x, y) in strips of `strips` rows, with `syms` symbols of these sizes; `refine`: the
/// refinement flag on, every instance unrefined (RI 0)
fn jb2_text_mq(w: u32, h: u32, op: u8, strips: u32, refine: bool, sizes: &[(usize, usize)], inst: &[(u32, i32, i32)]) -> Vec<u8> {
    let codelen = if sizes.len() <= 1 { 0 } else { 64 - (sizes.len() as u64 - 1).leading_zeros() };
    let flags: u16 = (refine as u16) << 1 | (strips.trailing_zeros() as u16) << 2 | 1 << 4 | (refine as u16) << 15;
    let mut d = jb2_region(w, h, 0, 0, op, &[&flags.to_be_bytes(), &(inst.len() as u32).to_be_bytes()]);
    let mut e = MqEnc::new();
    let new = || [(0u8, 0u8); 512];
    let (mut iadt, mut iafs, mut iads, mut iait, mut iari) = (new(), new(), new(), new(), new());
    let mut iaid = vec![(0u8, 0u8); 2 << codelen];
    let strips = strips as i32;
    let mut sorted = inst.to_vec();
    sorted.sort_by_key(|&(_, x, y)| (y - y.rem_euclid(strips), x));
    mq_int(&mut e, &mut iadt, Some(0));
    let (mut stript, mut firsts) = (0i32, 0i32);
    let mut i = 0;
    while i < sorted.len() {
        let base = sorted[i].2 - sorted[i].2.rem_euclid(strips);
        mq_int(&mut e, &mut iadt, Some((base - stript) / strips));
        stript = base;
        mq_int(&mut e, &mut iafs, Some(sorted[i].1 - firsts));
        firsts = sorted[i].1;
        let mut curs = firsts;
        let mut first = true;
        while i < sorted.len() && sorted[i].2 - sorted[i].2.rem_euclid(strips) == base {
            let (id, x, y) = sorted[i];
            if !first {
                mq_int(&mut e, &mut iads, Some(x - curs));
            }
            first = false;
            if strips > 1 {
                mq_int(&mut e, &mut iait, Some(y - base));
            }
            mq_id(&mut e, &mut iaid, id, codelen);
            if refine {
                mq_int(&mut e, &mut iari, Some(0));
            }
            curs = x + sizes[id as usize].0 as i32 - 1;
            i += 1;
        }
        mq_int(&mut e, &mut iads, None);
    }
    d.extend_from_slice(&e.finish());
    d
}

/// 6.7 (arithmetic, template 0): a pattern dictionary of four patterns of 4 x 4 (0, 4, 8, 12
/// pixels black)
fn jb2_patterns_mq() -> Vec<u8> {
    let (pw, ph, n) = (4usize, 4usize, 4usize);
    let mut d = vec![0, pw as u8, ph as u8];
    d.extend_from_slice(&(n as u32 - 1).to_be_bytes());
    let w = pw * n;
    let coll: Vec<u8> = (0..w * ph).map(|i| ((i % w % pw) + (i / w) * pw < (i % w / pw) * 4) as u8).collect();
    let mut e = MqEnc::new();
    let mut cx = vec![(0u8, 0u8); 1 << 16];
    mq_generic(&mut e, &mut cx, &coll, w, ph, 0, &[(-(pw as i32), 0), (-3, -1), (2, -2), (-2, -2)], None);
    d.extend_from_slice(&e.finish());
    d
}

/// 6.6 (arithmetic, template `t`): a halftone region of w x h over a grid of gw x gh cells 4
/// pixels apart (four patterns: grey values (m + n) mod 4, two planes Gray-coded); `skip`: the
/// cells outside the region left out
fn jb2_halftone_mq(w: u32, h: u32, gw: usize, gh: usize, t: u8, skip: bool, op: u8) -> Vec<u8> {
    let flags = t << 1 | (skip as u8) << 3 | op << 4;
    let grid = [&(gw as u32).to_be_bytes()[..], &(gh as u32).to_be_bytes(), &0u32.to_be_bytes(), &0u32.to_be_bytes(), &(4u16 << 8).to_be_bytes(), &0u16.to_be_bytes()].concat();
    let mut d = jb2_region(w, h, 0, 0, 0, &[&[flags], &grid]);
    let skipped: Vec<u8> = (0..gw * gh).map(|i| (skip && ((i % gw) * 4 >= w as usize || (i / gw) * 4 >= h as usize)) as u8).collect();
    let gray: Vec<usize> = (0..gw * gh).map(|i| if skipped[i] == 1 { 0 } else { (i % gw + i / gw) % 4 }).collect();
    let at: &[(i32, i32)] = match t {
        0 => &AT0,
        1 => &[(3, -1)],
        _ => &[(2, -1)],
    };
    let mut e = MqEnc::new();
    let mut cx = vec![(0u8, 0u8); 1 << 16];
    for j in 0..2 {
        // plane j (the most significant first), Gray-coded: its bit XOR the bit above it
        let plane: Vec<u8> = gray.iter().map(|&g| ((g >> (1 - j) & 1) ^ if j == 0 { 0 } else { g >> 1 & 1 }) as u8).collect();
        mq_generic(&mut e, &mut cx, &plane, gw, gh, t, at, skip.then_some(&skipped[..]));
    }
    d.extend_from_slice(&e.finish());
    d
}

/// JBIG2: PDF's embedded form (its input: the length of the globals, the globals, the page's
/// segments) and files, sequential and random-access.  Every kind of segment: generic regions in
/// MMR and arithmetic coding (each template, TPGDON, data of unknown length), refinement regions
/// (both templates, TPGRON, on the page and on an intermediate region), symbol dictionaries and
/// text regions (arithmetic and Huffman, refinement and aggregate coding, retained contexts,
/// transposed, a custom code table), pattern dictionaries and halftone regions (skip, a grid at an
/// angle), striped pages of unknown height, a default pixel and operator, a four-byte page
/// association; the files of tools/jbig2_check.py's layout suite.  Symbol dictionaries, text
/// regions, pattern dictionaries and halftones are encoded above so that they decode, except
/// aggregate coding and text regions in Huffman coding; their data is random, as is that of the
/// generic and refinement regions (which decode from any bytes)
fn jbig2_seeds() -> Vec<Vec<u8>> {
    let mut r = Rng(0x0B16_2000);
    let mut rnd = |n: usize| -> Vec<u8> { (0..n).map(|_| r.next() as u8).collect() };
    let mut out = Vec::new();
    let page = |num: u32, pg: u32, w: u32, h: u32| jb2_seg(num, 48, pg, &[], jb2_info(w, h, 0, 0), None);
    let eop = |num: u32, pg: u32| jb2_seg(num, 49, pg, &[], Vec::new(), None);
    let ff_ac = |mut v: Vec<u8>| {
        v.extend_from_slice(&[0xFF, 0xAC]);
        v
    };
    // MMR: a page of 8 x 2, four white and four black in both rows (pdftest's)
    let mmr = jb2_region(8, 2, 0, 0, 0, &[&[1], &[0x36, 0xF0]]);
    out.push(jb2_pdf(&[], &[page(0, 1, 8, 2), jb2_seg(1, 39, 1, &[], mmr.clone(), None), eop(2, 1)]));
    // arithmetic generic regions: each template, TPGDON on the odd ones; one of unknown length
    for t in 0..4u8 {
        let g = jb2_region(32, 16, 0, 0, 0, &[&[t << 1 | (t & 1) << 3], jb2_at(t), &ff_ac(rnd(48))]);
        out.push(jb2_pdf(&[], &[page(0, 1, 32, 16), jb2_seg(1, 39, 1, &[], g, None), eop(2, 1)]));
    }
    let mut g = jb2_region(32, 16, 0, 0, 0, &[&[0], jb2_at(0), &ff_ac(rnd(40))]);
    g.extend_from_slice(&16u32.to_be_bytes());
    out.push(jb2_pdf(&[], &[page(0, 1, 32, 16), jb2_seg(1, 38, 1, &[], g, Some(0xFFFF_FFFF)), eop(2, 1)]));
    // refinement on the page (template 0, its adaptive pixels) and on intermediate regions
    // (template 1 with TPGRON, a refinement of a refinement)
    let g1 = jb2_region(16, 16, 0, 0, 0, &[&[2], jb2_at(1), &ff_ac(rnd(24))]);
    let r0 = jb2_region(16, 16, 0, 0, 2, &[&[0], &[0xFF, 0xFF, 0xFF, 0xFF], &ff_ac(rnd(32))]);
    out.push(jb2_pdf(&[], &[page(0, 1, 16, 16), jb2_seg(1, 39, 1, &[], g1.clone(), None), jb2_seg(2, 43, 1, &[], r0, None), eop(3, 1)]));
    let r1 = jb2_region(16, 16, 0, 0, 0, &[&[3], &ff_ac(rnd(32))]);
    let r2 = jb2_region(16, 16, 4, 2, 0, &[&[1], &ff_ac(rnd(32))]);
    out.push(jb2_file(&[page(0, 1, 24, 24), jb2_seg(1, 36, 1, &[], g1, None), jb2_seg(2, 40, 1, &[1], r1, None), jb2_seg(3, 42, 1, &[2], r2, None), eop(4, 1)], false));
    // symbol dictionaries and text regions (encoded here): a global dictionary in arithmetic
    // coding, a text region of its five glyphs, eight instances on six strips (top left)
    let mq_sizes = class_sizes(&CLASSES_MQ);
    let inst: &[(u32, i32, i32)] = &[(0, 1, 1), (1, 7, 1), (2, 15, 2), (3, 2, 10), (4, 8, 10), (0, 20, 11), (1, 30, 10), (2, 40, 12)];
    let mut gb = vec![(0u8, 0u8); 1 << 16];
    let dict = jb2_symbols_mq(&CLASSES_MQ, 0, 0, &mut gb);
    let text = jb2_text_mq(48, 20, 0, 1, false, &mq_sizes, inst);
    out.push(jb2_pdf(&[jb2_seg(0, 0, 0, &[], dict, None)], &[page(1, 1, 48, 20), jb2_seg(2, 6, 1, &[0], text, None), eop(3, 1)]));
    // ... a dictionary that retains its contexts and one that goes on with them (after the first
    // one's five symbols); a text region of both (ten symbols) in strips of two rows with the
    // refinement flag on (no instance refined), XOR; an intermediate one under a refinement
    let mut gb = vec![(0u8, 0u8); 1 << 16];
    let kept = jb2_symbols_mq(&CLASSES_MQ, 0x200, 0, &mut gb);
    let more = jb2_symbols_mq(&CLASSES_MQ, 0x100, 5, &mut gb);
    let both: Vec<(usize, usize)> = mq_sizes.iter().chain(&mq_sizes).copied().collect();
    let inst2: Vec<(u32, i32, i32)> = inst.iter().enumerate().map(|(i, &(id, x, y))| (id + 5 * (i as u32 % 2), x, y)).collect();
    let text = jb2_text_mq(48, 20, 2, 2, true, &both, &inst2);
    let inter = jb2_text_mq(48, 20, 0, 1, false, &both, &inst2);
    let refined = jb2_region(48, 20, 0, 0, 0, &[&[1], &ff_ac(rnd(48))]);
    out.push(jb2_file(&[
        page(0, 1, 48, 20),
        jb2_seg(1, 0, 1, &[], kept, None),
        jb2_seg(2, 0, 1, &[1], more, None),
        jb2_seg(3, 7, 1, &[1, 2], text, None),
        jb2_seg(4, 4, 1, &[1, 2], inter, None),
        jb2_seg(5, 42, 1, &[4], refined, None),
        eop(6, 1),
    ], false));
    // random data where no encoder is here: refinement / aggregate coding with retained contexts
    // and a dictionary that uses them, a text region with refinement (template 0), two strips,
    // transposed, bottom right, XOR
    let mut agg = (2u16 | 0x200).to_be_bytes().to_vec();
    agg.extend_from_slice(jb2_at(0));
    agg.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 2, 0, 0, 0, 2]);
    agg.extend_from_slice(&rnd(64));
    let mut used = (2u16 | 0x100 | 0x200).to_be_bytes().to_vec();
    used.extend_from_slice(jb2_at(0));
    used.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 4, 0, 0, 0, 2]);
    used.extend_from_slice(&rnd(64));
    let tflags: u16 = 2 | 1 << 2 | 3 << 4 | 0x40 | 2 << 7;
    let text = jb2_region(40, 24, 0, 0, 0, &[&tflags.to_be_bytes(), &[0xFF, 0xFF, 0xFF, 0xFF], &5u32.to_be_bytes(), &rnd(64)]);
    out.push(jb2_file(&[page(0, 1, 40, 24), jb2_seg(1, 0, 1, &[], agg, None), jb2_seg(2, 0, 1, &[1], used, None), jb2_seg(3, 7, 1, &[1, 2], text, None), eop(4, 1)], false));
    // Huffman: a dictionary of the standard tables (encoded here) under an arithmetic text region
    // in strips of four rows; text regions in Huffman coding (random data) of the standard tables
    // and of a custom table for the first S coordinates
    let hd = jb2_symbols_huff(&CLASSES_HUFF);
    let text = jb2_text_mq(48, 20, 0, 4, false, &class_sizes(&CLASSES_HUFF), inst);
    out.push(jb2_pdf(&[jb2_seg(0, 0, 0, &[], hd.clone(), None)], &[page(1, 1, 48, 20), jb2_seg(2, 6, 1, &[0], text, None), eop(3, 1)]));
    let ht = jb2_region(32, 16, 0, 0, 0, &[&(1u16 | 1 << 4).to_be_bytes(), &0u16.to_be_bytes(), &3u32.to_be_bytes(), &rnd(40)]);
    out.push(jb2_pdf(&[jb2_seg(0, 0, 0, &[], hd.clone(), None)], &[page(1, 1, 32, 16), jb2_seg(2, 6, 1, &[0], ht, None), eop(3, 1)]));
    let table = jb2_table(&[(1, 4), (2, 5), (3, 4)], 4, 4);
    let hc = jb2_region(32, 16, 0, 0, 0, &[&(1u16 | 1 << 4).to_be_bytes(), &3u16.to_be_bytes(), &3u32.to_be_bytes(), &rnd(40)]);
    out.push(jb2_file(&[jb2_seg(0, 53, 0, &[], table, None), jb2_seg(1, 0, 0, &[], hd, None), page(2, 1, 32, 16), jb2_seg(3, 6, 1, &[1, 0], hc, None), eop(4, 1)], false));
    // a pattern dictionary of four 4 x 4 patterns and halftone regions on it (encoded here): a
    // grid of 10 x 6 with skip (its last column and row off the region), template 1 without, XOR;
    // random data: template 1 on a grid at an angle, partly off the region, AND on a black region
    let pats = jb2_patterns_mq();
    out.push(jb2_pdf(&[jb2_seg(0, 16, 0, &[], pats.clone(), None)], &[page(1, 1, 36, 20), jb2_seg(2, 22, 1, &[0], jb2_halftone_mq(36, 20, 10, 6, 0, true, 0), None), eop(3, 1)]));
    out.push(jb2_file(&[page(0, 1, 40, 24), jb2_seg(1, 16, 1, &[], pats.clone(), None), jb2_seg(2, 23, 1, &[1], jb2_halftone_mq(40, 24, 10, 6, 1, false, 2), None), eop(3, 1)], false));
    let grid = |gw: u32, gh: u32, gx: i32, gy: i32, rx: u16, ry: u16| [&gw.to_be_bytes()[..], &gh.to_be_bytes(), &gx.to_be_bytes(), &gy.to_be_bytes(), &rx.to_be_bytes(), &ry.to_be_bytes()].concat();
    let h2 = jb2_region(32, 32, 0, 0, 0, &[&[2 | 1 << 4 | 0x80], &grid(10, 10, -1024, 512, 0x2D4, 0x2D4), &ff_ac(rnd(64))]);
    out.push(jb2_file(&[jb2_seg(0, 16, 1, &[], pats, None), page(1, 1, 32, 32), jb2_seg(2, 23, 1, &[0], h2, None), eop(3, 1), jb2_seg(4, 51, 0, &[], Vec::new(), None)], true));
    // a striped page of unknown height (stripes of at most 8 rows): an MMR region, the end of the
    // stripe, an arithmetic region (template 2) below it, the end of that stripe
    let s1 = jb2_region(16, 8, 0, 0, 0, &[&[1], &rnd(12)]);
    let s2 = jb2_region(16, 8, 0, 8, 0, &[&[4], jb2_at(2), &ff_ac(rnd(20))]);
    out.push(jb2_file(&[
        jb2_seg(0, 48, 1, &[], jb2_info(16, 0xFFFF_FFFF, 0, 0x8008), None),
        jb2_seg(1, 39, 1, &[], s1, None),
        jb2_seg(2, 50, 1, &[], 7u32.to_be_bytes().to_vec(), None),
        jb2_seg(3, 38, 1, &[], s2, None),
        jb2_seg(4, 50, 1, &[], 15u32.to_be_bytes().to_vec(), None),
        eop(5, 1),
        jb2_seg(6, 51, 0, &[], Vec::new(), None),
    ], false));
    // default pixel 1, the operator overridden (XNOR, template 3), a region partly off the page;
    // a four-byte page association
    let g3 = jb2_region(24, 12, -5, 10, 3, &[&[6], jb2_at(3), &ff_ac(rnd(32))]);
    out.push(jb2_file(&[jb2_seg(0, 48 | 0x40, 7, &[], jb2_info(32, 16, 0x04 | 3 << 3 | 0x40, 0), None), jb2_seg(1, 39 | 0x40, 7, &[], g3, None), jb2_seg(2, 49 | 0x40, 7, &[], Vec::new(), None)], false));
    // the same generic region of template 0 in a random-access file
    let g0 = jb2_region(32, 16, 0, 0, 0, &[&[0], jb2_at(0), &ff_ac(rnd(48))]);
    out.push(jb2_file(&[page(0, 1, 32, 16), jb2_seg(1, 39, 1, &[], g0, None), eop(2, 1), jb2_seg(3, 51, 0, &[], Vec::new(), None)], true));
    // tools/jbig2_check.py's layout suite: two pages interleaved, two pages without an end of
    // page, a global pattern dictionary after a damaged page (MMR)
    let region = jb2_region(8, 2, 0, 0, 0, &[&[1], &[0x36, 0xF0]]);
    let patterns = [&[1u8, 4, 2][..], &1u32.to_be_bytes(), &[0x36, 0xF0]].concat();
    let halftone = jb2_region(8, 2, 0, 0, 0, &[&[1], &grid(2, 1, 0, 0, 4 << 8, 0), &[0x23, 0xA0]]);
    out.push(jb2_file(&[page(0, 1, 8, 2), page(1, 2, 8, 2), jb2_seg(2, 39, 1, &[], region.clone(), None), eop(3, 1), eop(4, 2), jb2_seg(5, 51, 0, &[], Vec::new(), None)], false));
    out.push(jb2_file(&[page(0, 1, 8, 2), jb2_seg(1, 39, 1, &[], region, None), page(2, 2, 8, 2)], false));
    out.push(jb2_file(&[page(0, 1, 8, 2), jb2_seg(1, 38, 1, &[], vec![0; 5], None), jb2_seg(2, 16, 0, &[], patterns, None), eop(3, 1), page(4, 2, 8, 2), jb2_seg(5, 23, 2, &[2], halftone, None), eop(6, 2)], false));
    out
}

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
        // netio's client on a response stream: status lines, interim answers, framing, the
        // decoders of Content-Encoding, Set-Cookie into a jar, a Location resolved
        ("http_client", client_seeds(), Box::new(|d: &[u8]| {
            let jar = std::sync::Arc::new(plat::sync::Mutex::new(netpkt::cookie::Jar::new()));
            let mut c = netio::Client::new().with_cookies(jar.clone());
            c.max_body = 1 << 20;
            let url = netio::Url::parse("https://www.example.org/a/b?c").expect("the target URL");
            for method in ["GET", "HEAD"] {
                let m = Mock(Mutex::new((d.to_vec(), 0)));
                if let Ok(r) = c.request_on(&m, method, &url, &netio::Headers::new(), &[]) {
                    assert!(r.body_bytes().len() <= 1 << 20, "a body over max_body");
                    assert!(method == "GET" || r.body_bytes().is_empty(), "a body for HEAD");
                    if let Some(loc) = r.headers.get("location") {
                        let _ = url.join(loc);
                    }
                }
            }
            let a = netpkt::cookie::Asks { host: "www.example.org", path: "/a/b", secure: true, http: true, site: None, top: true, safe: true };
            let _ = jar.lock().header(&a, 0);
        })),
        // URLs: parse, the text written again parses to the same URL, references joined
        // against them and against RFC 3986's base; proxy URLs and NO_PROXY lists
        ("url", url_seeds(), Box::new(|d: &[u8]| {
            let s = String::from_utf8_lossy(d);
            let (first, rest) = s.split_once('\n').unwrap_or((&s, ""));
            let same = |u: &netio::Url, what: &str| {
                let again = netio::Url::parse(&u.to_string());
                assert!(again.as_ref() == Some(u), "{what} does not come back from its text: {u:?} -> {u} -> {again:?}");
            };
            let base = netio::Url::parse("http://a/b/c/d;p?q").expect("the base");
            if let Some(u) = netio::Url::parse(first) {
                same(&u, "a URL");
                if let Some(j) = u.join(rest) {
                    same(&j, "a joined URL");
                }
                let _ = netio::proxy::Proxy::none().no_proxy(rest).bypass(&u);
            }
            if let Some(j) = base.join(first) {
                same(&j, "a reference joined to the base");
            }
            let _ = netio::proxy::Endpoint::parse(first);
        })),
        // cookies: Set-Cookie lines into a jar from hosts on both sides of public suffixes, the
        // Cookie header for a cross-site request; each line as a host name for the suffix list
        ("cookie", cookie_seeds(), Box::new(|d: &[u8]| {
            let s = String::from_utf8_lossy(d);
            let hosts = ["www.example.co.uk", "evil.github.io", "github.io", "192.168.1.2", "a.b.c.kobe.jp", "localhost"];
            let mut jar = netpkt::cookie::Jar::new();
            for (i, line) in s.lines().enumerate() {
                let o = netpkt::cookie::Origin { host: hosts[i % hosts.len()], path: "/a/b", secure: i % 2 == 0, http: true };
                let _ = jar.store(line, o, 1_700_000_000);
                let h = line.trim().to_ascii_lowercase();
                let suf = netpkt::psl::public_suffix(&h);
                assert!(h.ends_with(suf), "a public suffix that is not the name's end: {h:?} -> {suf:?}");
                if let Some(r) = netpkt::psl::registrable_domain(&h) {
                    assert!(r.len() > suf.len() && r.ends_with(suf) && h.ends_with(r), "a registrable domain beside its suffix: {h:?} -> {r:?} / {suf:?}");
                    assert!(!netpkt::psl::is_public_suffix(r), "a registrable domain that is a public suffix: {r:?}");
                }
            }
            for host in hosts {
                let a = netpkt::cookie::Asks { host, path: "/a/b/c", secure: true, http: true, site: Some("example.org"), top: false, safe: true };
                let _ = jar.header(&a, 1_700_000_000);
            }
            assert!(jar.cookies().len() <= netpkt::cookie::MAX_COOKIES, "a jar over its limit");
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
        ("img_jp2", jp2_seeds(seed_dir), Box::new(|d: &[u8]| {
            let _ = img_jp2::probe(d);
            // img_all's way: the codec at full resolution, into ARGB
            img_codec(&img_jp2::CODEC, d, JP2_PIXELS);
            // a lower resolution, the components as they are coded: each holds its samples
            let opt = img_jp2::Options { reduce: 1 + (d.len() % 3) as u8, max_pixels: JP2_PIXELS, convert_sycc: false, ..img_jp2::Options::default() };
            if let Ok(img) = img_jp2::decode(d, &opt) {
                for c in &img.components {
                    assert_eq!(c.data.len() as u64, c.width as u64 * c.height as u64, "a component of {} x {}", c.width, c.height);
                }
            }
        })),
        ("img_jbig2", jbig2_seeds(), Box::new(|d: &[u8]| {
            let opt = img_jbig2::Options { max_pixels: JBIG2_PIXELS };
            let check = |b: &img_jbig2::Bitmap| {
                assert_eq!(b.stride, (b.width as usize).div_ceil(8), "the stride of a page {} wide", b.width);
                let px = b.width as u64 * b.height as u64;
                assert!(px <= JBIG2_PIXELS, "a page of {} x {} past the cap", b.width, b.height);
                assert_eq!(b.bits.len() as u64, if px == 0 { 0 } else { b.stride as u64 * b.height as u64 }, "the bits of a page of {} x {}", b.width, b.height);
            };
            if img_jbig2::probe(d) {
                // a file: every page; img_all's codec: the first one, into ARGB
                if let Ok(pages) = img_jbig2::decode_file(d, &opt) {
                    pages.iter().for_each(check);
                }
                img_codec(&img_jbig2::CODEC, d, JBIG2_PIXELS);
            } else if d.len() >= 2 {
                // PDF's embedded form: the length of the JBIG2Globals (0: none), them, the page's segments
                let g = (u16::from_be_bytes([d[0], d[1]]) as usize).min(d.len() - 2);
                let (globals, page) = d[2..].split_at(g);
                if let Ok(b) = img_jbig2::decode_embedded(page, (g > 0).then_some(globals), &opt) {
                    check(&b);
                }
            }
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
