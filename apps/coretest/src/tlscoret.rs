// tlscoret -- checks of netpkt::tls and netpkt::x509 (group tlscore; the group tls is netio's): the key
// schedule against RFC 8448, certificates written, read and checked with every fault, a client and a
// server core talking in memory (bytes in pieces, data both ways, close_notify, the same seeds giving
// the same bytes), the alert for each fault and what excuses it, a changed byte refused, what is no
// TLS; the core against netio::tls's Conn both ways with each other's certificates (RFC 8446 from
// another implementation, the one tools/tls_interop.py checks against OpenSSL); dissect and
// decode::layers of what they send.  --tls-core-pki DIR, --tls-core-serve PORT, --tls-core-get PORT are
// the core on sockets for tools/tls_interop.py --core, where OpenSSL itself is the other side

use alloc::string::String;
use alloc::vec::Vec;

use crypto::ed25519::Keypair;
use netpkt::decode;
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::tls::{self, ClientCfg, Core, Event, Failure, Identity, Part};
use netpkt::x509::{check_chain, Cert, CertSpec, F_BAD_SIGNATURE, F_EXPIRED, F_NAME, F_NOT_YET, F_SELF_SIGNED, F_UNKNOWN_ISSUER};
use netpkt::{IpAddr, Ipv4, Mac};
use plat::net::{IpAddr as NetAddr, SocketAddr, TcpListener, TcpStream};

use crate::T;

// a time inside what the certificates here say
const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn key(n: u8) -> Keypair {
    Keypair::from_seed(&[n; 32])
}

fn ca(n: u8, cn: &str) -> (Cert, Keypair) {
    let k = key(n);
    let der = CertSpec { serial: [n; 16], cn: String::from(cn), org: String::from("Labor"), not_before: NOW - 10 * DAY, not_after: NOW + 3650 * DAY, public_key: k.public, is_ca: true, ..CertSpec::default() }.sign(None, &k);
    (Cert::parse(&der).unwrap_or_default(), k)
}

fn spec(k: &Keypair, names: &[&str]) -> CertSpec {
    CertSpec { serial: [9; 16], cn: String::from(names.first().copied().unwrap_or("x")), org: String::new(), not_before: NOW - DAY, not_after: NOW + DAY, public_key: k.public, is_ca: false, dns: names.iter().map(|n| String::from(*n)).collect(), ips: Vec::new() }
}

fn identity(names: &[&str]) -> (Identity, Cert) {
    let (c, ck) = ca(1, "Labor-CA");
    let k = key(2);
    let der = spec(&k, names).sign(Some(&c), &ck);
    (Identity { chain: alloc::vec![der], key: k }, c)
}

fn cfg(name: &str, anchor: &Cert) -> ClientCfg {
    ClientCfg { name: String::from(name), anchors: alloc::vec![anchor.clone()], now: NOW, insecure: false, accept: Vec::new() }
}

// both sides until nothing moves; `piece`: bytes at a time
fn shuttle(c: &mut Core, s: &mut Core, piece: usize) {
    loop {
        let a = c.take_out();
        let b = s.take_out();
        if a.is_empty() && b.is_empty() {
            return;
        }
        for p in a.chunks(piece.max(1)) {
            s.push(p);
        }
        for p in b.chunks(piece.max(1)) {
            c.push(p);
        }
    }
}

fn failure(c: &mut Core) -> Option<Failure> {
    c.take_events().into_iter().find_map(|e| match e {
        Event::Failed(f) => Some(f),
        _ => None,
    })
}

pub fn run(t: &mut T) {
    schedule(t);
    certs(t);
    talking(t);
    faults(t);
    refused(t);
    with_netio(t);
    dissector(t);
}

fn schedule(t: &mut T) {
    // RFC 8448, "Simple 1-RTT Handshake"
    let mut ecdhe = [0u8; 32];
    ecdhe.copy_from_slice(&crypto::unhex("8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d").unwrap_or_default());
    let (handshake, master) = tls::schedule(&ecdhe);
    t.eq("handshake secret", crypto::hex(&handshake), String::from("1dc826e93606aa6fdc0aadc12f741b01046aa6b99f691ed221a9f0ca043fbeac"));
    t.eq("master secret", crypto::hex(&master), String::from("18df06843d13a08bf2a449844c5f8a478001bc4d4c627984d5a41da8d0402919"));
    let mut h = [0u8; 32];
    h.copy_from_slice(&crypto::unhex("860c06edc07858ee8e78f0e7428c58edd6b43f2ca3e6e95f02ed063cf0e1cad8").unwrap_or_default());
    t.eq("c hs traffic", crypto::hex(&tls::derive(&handshake, "c hs traffic", &h)), String::from("b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21"));
    let s_hs = tls::derive(&handshake, "s hs traffic", &h);
    t.eq("s hs traffic", crypto::hex(&s_hs), String::from("b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38"));
    let mut b = [0u8; 32];
    tls::expand_label(&s_hs, "finished", &[], &mut b);
    t.eq("finished key", crypto::hex(&b), String::from("008d3b66f816ea559f96b537e885c31fc068bf492c652f01f288a1d8cdc19fc8"));
    t.eq("names", (tls::suite_name(tls::SUITE), tls::alert_name(tls::AL_UNKNOWN_CA), tls::hs_name(tls::HS_FINISHED), tls::ct_name(tls::CT_DATA)), ("TLS_CHACHA20_POLY1305_SHA256", "unknown_ca", "Finished", "Application Data"));
}

fn certs(t: &mut T) {
    let (root, _) = ca(1, "Labor-CA");
    t.check("a CA reads", root.is_ca && root.subject_cn == "Labor-CA" && root.subject_org == "Labor" && root.issuer_cn == "Labor-CA" && root.self_signed());
    let (id, _) = identity(&["web.lab", "*.shop.lab", "1.2.3.4"]);
    let leaf = Cert::parse(&id.chain[0]).unwrap_or_default();
    t.check("a leaf reads", !leaf.is_ca && leaf.issuer_cn == "Labor-CA" && leaf.signed_by(&root) && !leaf.self_signed());
    t.eq("its times", (leaf.not_before, leaf.not_after), (NOW - DAY, NOW + DAY));
    t.eq("its key", leaf.key, Some(key(2).public));
    t.check("a name", leaf.names("web.lab") && leaf.names("WEB.lab."));
    t.check("a wildcard", leaf.names("a.shop.lab") && !leaf.names("shop.lab") && !leaf.names("a.b.shop.lab"));
    t.check("another name", !leaf.names("other.lab"));
    let k = key(3);
    let with_ip = CertSpec { ips: alloc::vec![IpAddr::V4(Ipv4([10, 0, 0, 1]))], ..spec(&k, &["ip.lab"]) }.sign(None, &k);
    let c = Cert::parse(&with_ip).unwrap_or_default();
    t.check("an address", c.names("10.0.0.1") && !c.names("10.0.0.2"));
    t.eq("names for a dialog", c.name_list(), alloc::vec![String::from("ip.lab"), String::from("10.0.0.1")]);
    t.check("the common name is no name", !Cert::parse(&CertSpec { dns: Vec::new(), ..spec(&k, &["ip.lab"]) }.sign(None, &k)).unwrap_or_default().names("ip.lab"));
    t.eq("a fingerprint", leaf.fingerprint_text().len(), 32 * 3 - 1);
    t.check("garbage", Cert::parse(b"\x30\x03\x02\x01\x00").is_none() && Cert::parse(&id.chain[0][..100]).is_none());
    let mut bad = id.chain[0].clone();
    let n = bad.len();
    bad[n - 10] ^= 1;
    t.check("a changed signature", Cert::parse(&bad).is_some_and(|c| !c.signed_by(&root)));
    // dates over the two forms of a time: UTCTime to 2049, GeneralizedTime from 2050
    let far = CertSpec { not_before: 2_524_608_000, not_after: 4_102_444_800, ..spec(&k, &["x.lab"]) }.sign(None, &k);
    t.eq("a far future", Cert::parse(&far).map(|c| (c.not_before, c.not_after)), Some((2_524_608_000, 4_102_444_800)));
    // the faults
    let anchors = alloc::vec![root.clone()];
    let chain = alloc::vec![leaf.clone()];
    t.eq("nothing wrong", check_chain(&chain, &anchors, "web.lab", NOW), 0);
    t.eq("another name", check_chain(&chain, &anchors, "other.lab", NOW), F_NAME);
    t.eq("expired", check_chain(&chain, &anchors, "web.lab", NOW + 2 * DAY), F_EXPIRED);
    t.eq("at its very end", check_chain(&chain, &anchors, "web.lab", NOW + DAY), F_EXPIRED);
    t.eq("not yet", check_chain(&chain, &anchors, "web.lab", NOW - 2 * DAY), F_NOT_YET);
    let own = Cert::parse(&spec(&k, &["web.lab"]).sign(None, &k)).unwrap_or_default();
    t.eq("self-signed", check_chain(core::slice::from_ref(&own), &anchors, "web.lab", NOW), F_SELF_SIGNED);
    t.eq("self-signed, trusted as itself", check_chain(core::slice::from_ref(&own), core::slice::from_ref(&own), "web.lab", NOW), 0);
    let (other, ok) = ca(5, "Andere CA");
    let foreign = Cert::parse(&spec(&k, &["web.lab"]).sign(Some(&other), &ok)).unwrap_or_default();
    t.eq("an issuer nobody knows", check_chain(core::slice::from_ref(&foreign), &anchors, "web.lab", NOW), F_UNKNOWN_ISSUER);
    let mut cheat = spec(&k, &["web.lab"]).sign(Some(&root), &k);
    let n = cheat.len();
    cheat[n - 1] ^= 1;
    let cheat = Cert::parse(&cheat).unwrap_or_default();
    t.eq("a wrong signature", check_chain(core::slice::from_ref(&cheat), &anchors, "web.lab", NOW), F_UNKNOWN_ISSUER);
    t.eq("several at once", check_chain(core::slice::from_ref(&own), &anchors, "x.lab", NOW + 3 * DAY), F_NAME | F_EXPIRED | F_SELF_SIGNED);
    t.eq("no chain", check_chain(&[], &anchors, "x.lab", NOW), F_UNKNOWN_ISSUER);
    // through an intermediate
    let (ic, ik) = ca(6, "Zwischen-CA");
    let inter_der = CertSpec { serial: [6; 16], cn: String::from("Zwischen-CA"), org: String::from("Labor"), not_before: NOW - DAY, not_after: NOW + DAY, public_key: ik.public, is_ca: true, ..CertSpec::default() }.sign(Some(&root), &key(1));
    let inter = Cert::parse(&inter_der).unwrap_or_default();
    let via = Cert::parse(&spec(&k, &["web.lab"]).sign(Some(&ic), &ik)).unwrap_or_default();
    t.eq("through an intermediate", check_chain(&[via.clone(), inter], &anchors, "web.lab", NOW), 0);
    t.eq("without it", check_chain(core::slice::from_ref(&via), &anchors, "web.lab", NOW), F_UNKNOWN_ISSUER);
    let other_key = CertSpec { public_key: [7; 32], ..spec(&k, &["web.lab"]) };
    let _ = F_BAD_SIGNATURE;
    t.check("a key that is not its signer's", Cert::parse(&other_key.sign(None, &k)).is_some_and(|c| !c.self_signed()));
}

fn talking(t: &mut T) {
    let (id, root) = identity(&["web.lab"]);
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    let mut s = Core::server(id.clone(), &seed(2));
    t.check("no plain text before the handshake's end", !c.send(b"x"));
    shuttle(&mut c, &mut s, 1 << 20);
    t.check("established", c.established() && s.established());
    t.eq("no faults", c.faults(), 0);
    t.check("not excused", !c.excused());
    t.eq("the SNI", s.name(), "web.lab");
    t.eq("the server's chain", c.peer_certs().first().map(|x| x.subject_cn.clone()), Some(String::from("web.lab")));
    let ev = c.take_events();
    t.eq("the client's events", ev, alloc::vec![Event::Certificate { faults: 0 }, Event::Established]);
    t.eq("the server's events", s.take_events(), alloc::vec![Event::Hello { name: String::from("web.lab") }, Event::Established]);
    // data both ways, a lot of it, in records
    let big: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    t.check("send", c.send(&big));
    shuttle(&mut c, &mut s, 1000);
    t.eq("client to server", s.read(), big.clone());
    t.check("send back", s.send(b"HTTP/1.1 200 OK\r\n\r\n"));
    shuttle(&mut c, &mut s, 7);
    t.eq("server to client", c.read(), Vec::from(&b"HTTP/1.1 200 OK\r\n\r\n"[..]));
    t.check("an empty send", c.send(b""));
    // close_notify
    c.close();
    shuttle(&mut c, &mut s, 1 << 20);
    t.check("the peer heard it", s.peer_closed() && s.take_events().contains(&Event::PeerClosed));
    t.check("the one who closed is done", c.dead() && !c.send(b"x"));
    t.check("no failure", c.failure().is_none() && s.failure().is_none());
    // in single bytes
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    let mut s = Core::server(id.clone(), &seed(2));
    shuttle(&mut c, &mut s, 1);
    t.check("byte by byte", c.established() && s.established());
    // the same seeds: the same bytes
    let run = |a: u8, b: u8| {
        let mut c = Core::client(cfg("web.lab", &root), &seed(a));
        let mut s = Core::server(id.clone(), &seed(b));
        let mut wire = Vec::new();
        for _ in 0..4 {
            let x = c.take_out();
            let y = s.take_out();
            wire.extend_from_slice(&x);
            wire.extend_from_slice(&y);
            s.push(&x);
            c.push(&y);
        }
        c.send(b"abc");
        wire.extend_from_slice(&c.take_out());
        wire
    };
    t.check("the same seeds, the same bytes", run(1, 2) == run(1, 2));
    t.check("other seeds, other bytes", run(1, 2) != run(3, 2) && run(1, 2) != run(1, 3));
    // an address: no SNI, the address in the certificate
    let k = key(4);
    let der = CertSpec { ips: alloc::vec![IpAddr::V4(Ipv4([10, 0, 0, 1]))], dns: Vec::new(), ..spec(&k, &["x"]) }.sign(Some(&root), &key(1));
    let mut c = Core::client(cfg("10.0.0.1", &root), &seed(1));
    let mut s = Core::server(Identity { chain: alloc::vec![der], key: k }, &seed(2));
    shuttle(&mut c, &mut s, 1 << 20);
    t.check("an address: established", c.established());
    t.eq("an address: no SNI", s.name(), "");
    // a KeyUpdate the peer sends is taken (the side here only receives one): netio's below
    t.check("a footprint", c.footprint() > 1024);
}

fn faults(t: &mut T) {
    let (root, rk) = ca(1, "Labor-CA");
    let k = key(2);
    // a chain with this spec, by this issuer
    let id = |sp: CertSpec, issuer: Option<&Cert>, signer: &Keypair| Identity { chain: alloc::vec![sp.sign(issuer, signer)], key: key(2) };
    let cases: [(&str, Identity, &str, u32, u8); 5] = [
        ("self-signed", id(spec(&k, &["web.lab"]), None, &k), "web.lab", F_SELF_SIGNED, tls::AL_UNKNOWN_CA),
        ("expired", id(CertSpec { not_before: NOW - 3 * DAY, not_after: NOW - DAY, ..spec(&k, &["web.lab"]) }, Some(&root), &rk), "web.lab", F_EXPIRED, tls::AL_CERTIFICATE_EXPIRED),
        ("not yet", id(CertSpec { not_before: NOW + DAY, not_after: NOW + 3 * DAY, ..spec(&k, &["web.lab"]) }, Some(&root), &rk), "web.lab", F_NOT_YET, tls::AL_CERTIFICATE_EXPIRED),
        ("another name", id(spec(&k, &["other.lab"]), Some(&root), &rk), "web.lab", F_NAME, tls::AL_BAD_CERTIFICATE),
        ("an unknown issuer", id(spec(&k, &["web.lab"]), Some(&ca(9, "Andere").0), &key(9)), "web.lab", F_UNKNOWN_ISSUER, tls::AL_UNKNOWN_CA),
    ];
    for (name, ident, host, fault, alert) in cases {
        let mut c = Core::client(cfg(host, &root), &seed(1));
        let mut s = Core::server(ident.clone(), &seed(2));
        shuttle(&mut c, &mut s, 1 << 20);
        t.check(&alloc::format!("{}: no connection", name), !c.established() && c.dead());
        t.eq(&alloc::format!("{}: its fault", name), c.faults(), fault);
        t.eq(&alloc::format!("{}: the client's alert", name), failure(&mut c).map(|f| (f.alert, f.ours)), Some((alert, true)));
        t.eq(&alloc::format!("{}: the server hears it", name), failure(&mut s).map(|f| (f.alert, f.ours)), Some((alert, false)));
        t.check(&alloc::format!("{}: the chain is there to show", name), c.peer_certs().len() == 1);
        // insecure and an accepted fingerprint go on
        let mut c = Core::client(ClientCfg { insecure: true, ..cfg(host, &root) }, &seed(1));
        let mut s = Core::server(ident.clone(), &seed(2));
        shuttle(&mut c, &mut s, 1 << 20);
        t.check(&alloc::format!("{}: insecure goes on", name), c.established() && s.established() && c.excused() && c.faults() == fault);
        let fp = Cert::parse(&ident.chain[0]).map(|x| x.fingerprint()).unwrap_or([0; 32]);
        let mut c = Core::client(ClientCfg { accept: alloc::vec![fp], ..cfg(host, &root) }, &seed(1));
        let mut s = Core::server(ident.clone(), &seed(2));
        shuttle(&mut c, &mut s, 1 << 20);
        t.check(&alloc::format!("{}: an accepted fingerprint goes on", name), c.established() && c.excused());
        let mut c = Core::client(ClientCfg { accept: alloc::vec![[1; 32]], ..cfg(host, &root) }, &seed(1));
        let mut s = Core::server(ident, &seed(2));
        shuttle(&mut c, &mut s, 1 << 20);
        t.check(&alloc::format!("{}: another fingerprint does not", name), !c.established());
    }
    // the proof of the key counts also when the certificate is taken as it is: another key than the
    // certificate's fails on its CertificateVerify
    let bad = Identity { chain: alloc::vec![spec(&k, &["web.lab"]).sign(None, &k)], key: key(3) };
    let mut c = Core::client(ClientCfg { insecure: true, ..cfg("web.lab", &root) }, &seed(1));
    let mut s = Core::server(bad, &seed(2));
    shuttle(&mut c, &mut s, 1 << 20);
    t.eq("insecure still checks the signature", failure(&mut c).map(|f| f.alert), Some(tls::AL_DECRYPT_ERROR));
}

fn refused(t: &mut T) {
    let (id, root) = identity(&["web.lab"]);
    // no TLS at all: an HTTP answer
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    c.push(b"HTTP/1.1 400 Bad Request\r\n\r\n");
    t.eq("an HTTP answer", failure(&mut c).map(|f| f.alert), Some(tls::AL_PROTOCOL_VERSION));
    let mut s = Core::server(id.clone(), &seed(2));
    s.push(b"GET / HTTP/1.1\r\n\r\n");
    t.eq("an HTTP request", failure(&mut s).map(|f| f.alert), Some(tls::AL_PROTOCOL_VERSION));
    t.check("the alert goes out", !s.take_out().is_empty());
    // a changed byte in a protected record
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    let mut s = Core::server(id.clone(), &seed(2));
    shuttle(&mut c, &mut s, 1 << 20);
    let _ = (c.take_events(), s.take_events());
    s.send(b"geheim");
    let mut w = s.take_out();
    let n = w.len();
    w[n - 20] ^= 1;
    c.push(&w);
    t.eq("a changed byte", failure(&mut c).map(|f| f.alert), Some(tls::AL_BAD_RECORD_MAC));
    t.check("nothing of it to read", c.read().is_empty());
    // a record too long, handshake messages out of order
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    c.push(&[22, 3, 3, 0xFF, 0xFF]);
    t.eq("a record too long", failure(&mut c).map(|f| f.alert), Some(tls::AL_RECORD_OVERFLOW));
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    c.push(&[22, 3, 3, 0, 4, tls::HS_FINISHED, 0, 0, 0]);
    t.eq("out of order", failure(&mut c).map(|f| f.alert), Some(tls::AL_UNEXPECTED_MESSAGE));
    // a ClientHello of another make that lacks what this server needs
    let mut s = Core::server(id, &seed(2));
    let mut b = alloc::vec![3u8, 3];
    b.extend_from_slice(&[0u8; 32]);
    b.extend_from_slice(&[0, 0, 2, 0x13, 0x03, 1, 0, 0, 0]);
    let mut m = alloc::vec![tls::HS_CLIENT_HELLO, 0, 0, b.len() as u8];
    m.extend_from_slice(&b);
    let mut rec = alloc::vec![22u8, 3, 1, 0, m.len() as u8];
    rec.extend_from_slice(&m);
    s.push(&rec);
    t.eq("TLS 1.3 required", failure(&mut s).map(|f| f.alert), Some(tls::AL_PROTOCOL_VERSION));
}

// netio's Conn is plat's: its randomness and its clock are real, so a certificate here spans decades
fn with_netio(t: &mut T) {
    use netio::tls::{ClientConfig, Conn, Identity as NIdentity, ServerConfig, Trust};
    use netio::x509::{Cert as NCert, CertBuilder};
    let (a, z) = (946_684_800, 4_102_444_800); // 2000 to 2100
    // our certificates through netio
    let rk = key(11);
    let root_der = CertSpec { serial: [11; 16], cn: String::from("Labor-CA"), org: String::from("Labor"), not_before: a, not_after: z, public_key: rk.public, is_ca: true, ..CertSpec::default() }.sign(None, &rk);
    let root = Cert::parse(&root_der).unwrap_or_default();
    let nroot = NCert::parse(&root_der);
    t.check("netio reads our CA", nroot.as_ref().is_some_and(|c| c.is_ca && c.subject_cn == "Labor-CA" && c.signed_by(c)));
    let nroot = nroot.unwrap_or_default();
    let k = key(12);
    let leaf = CertSpec { not_before: a, not_after: z, ..spec(&k, &["web.lab"]) }.sign(Some(&root), &rk);
    let nleaf = NCert::parse(&leaf);
    t.check("netio reads our leaf", nleaf.as_ref().is_some_and(|c| c.names("web.lab") && c.signed_by(&nroot)));
    t.check("netio's chain check takes it", nleaf.is_some_and(|c| netio::x509::verify_chain(&[c], core::slice::from_ref(&nroot), NOW, Some("web.lab"), netio::x509::Usage::Server).is_ok()));
    // netio's client, our server
    let mut c = Conn::client(ClientConfig { trust: Trust::Anchors(alloc::vec![nroot.clone()]), identity: None, server_name: String::from("web.lab") });
    let mut s = Core::server(Identity { chain: alloc::vec![leaf.clone()], key: k.clone() }, &seed(2));
    for _ in 0..8 {
        let x = c.take_out();
        let y = s.take_out();
        s.push(&x);
        let _ = c.feed(&y);
    }
    t.check("netio's client: established", c.established() && s.established());
    t.eq("netio's client: its SNI", s.name(), "web.lab");
    let _ = c.write_plain(b"GET / HTTP/1.1\r\n\r\n");
    s.push(&c.take_out());
    t.eq("netio's client: its data", s.read(), Vec::from(&b"GET / HTTP/1.1\r\n\r\n"[..]));
    s.send(b"HTTP/1.1 204 No Content\r\n\r\n");
    let _ = c.feed(&s.take_out());
    let mut buf = [0u8; 64];
    let n = c.read_plain(&mut buf);
    t.eq("netio's client: our data", &buf[..n], &b"HTTP/1.1 204 No Content\r\n\r\n"[..]);
    c.close();
    s.push(&c.take_out());
    t.check("netio's client: close_notify", s.peer_closed());
    // our client, netio's server, netio's certificates
    let nk = key(13);
    let nca = CertBuilder { serial: [13; 16], cn: String::from("Netio-CA"), org: String::from("JFLib"), not_before: a, not_after: z, public_key: nk.public, is_ca: true, ..CertBuilder::default() }.sign(None, &nk);
    let sk = key(14);
    let nca_cert = NCert::parse(&nca).unwrap_or_default();
    let nleaf = CertBuilder { serial: [14; 16], cn: String::from("web.lab"), not_before: a, not_after: z, public_key: sk.public, dns: alloc::vec![String::from("web.lab")], server_auth: true, ..CertBuilder::default() }.sign(Some(&nca_cert), &nk);
    let our_root = Cert::parse(&nca);
    t.check("we read netio's CA", our_root.as_ref().is_some_and(|c| c.is_ca && c.subject_cn == "Netio-CA" && c.subject_org == "JFLib" && c.self_signed()));
    let our_root = our_root.unwrap_or_default();
    let mut s = Conn::server(ServerConfig { identity: NIdentity { chain: alloc::vec![nleaf], key: sk }, client_auth: None, client_auth_optional: false });
    let mut c = Core::client(ClientCfg { name: String::from("web.lab"), anchors: alloc::vec![our_root], now: NOW, insecure: false, accept: Vec::new() }, &seed(1));
    for _ in 0..8 {
        let x = c.take_out();
        let y = s.take_out();
        let _ = s.feed(&x);
        c.push(&y);
    }
    t.check("netio's server: established", c.established() && s.established());
    t.eq("netio's server: no faults", c.faults(), 0);
    c.send(b"GET / HTTP/1.1\r\n\r\n");
    let _ = s.feed(&c.take_out());
    let n = s.read_plain(&mut buf);
    t.eq("netio's server: our data", &buf[..n], &b"GET / HTTP/1.1\r\n\r\n"[..]);
    let _ = s.write_plain(b"HTTP/1.1 204 No Content\r\n\r\n");
    c.push(&s.take_out());
    t.eq("netio's server: its data", c.read(), Vec::from(&b"HTTP/1.1 204 No Content\r\n\r\n"[..]));
    s.close();
    c.push(&s.take_out());
    t.check("netio's server: close_notify", c.peer_closed());
    t.eq("no failures", c.failure(), None);
    let _ = root;
}

// the certificates of --tls-core-*: a CA and web.lab, valid for a century (real sockets, a real clock)
fn interop_pki() -> (Cert, Identity) {
    let (a, z) = (946_684_800, 4_102_444_800);
    let rk = key(31);
    let root = CertSpec { serial: [31; 16], cn: String::from("Interop CA"), org: String::from("JFLib"), not_before: a, not_after: z, public_key: rk.public, is_ca: true, ..CertSpec::default() }.sign(None, &rk);
    let root = Cert::parse(&root).unwrap_or_default();
    let k = key(32);
    let leaf = CertSpec { not_before: a, not_after: z, ..spec(&k, &["web.lab"]) }.sign(Some(&root), &rk);
    (root, Identity { chain: alloc::vec![leaf], key: k })
}

/// --tls-core-pki DIR: ca.pem, leaf.pem, leaf.key for the other side
pub fn write_pki(dir: &str) -> i32 {
    let (root, id) = interop_pki();
    let w = |name: &str, s: String| plat::write_file(&plat::fs::join(dir, name), s.as_bytes());
    let ok = w("ca.pem", netio::pem::encode("CERTIFICATE", &root.der)) && w("leaf.pem", netio::pem::encode("CERTIFICATE", &id.chain[0])) && w("leaf.key", netio::pem::encode("PRIVATE KEY", &netio::x509::ed25519_pkcs8(id.key.seed())));
    if ok { 0 } else { 1 }
}

// bytes to the socket, what came to the core, until `done` or the end -> it got there
fn pump_socket(c: &mut Core, s: &TcpStream, done: &mut dyn FnMut(&mut Core) -> bool) -> bool {
    let mut buf = alloc::vec![0u8; 1 << 16];
    s.set_read_timeout(Some(5000));
    for _ in 0..1000 {
        let out = c.take_out();
        if !out.is_empty() && s.write_all(&out).is_err() {
            return false;
        }
        if done(c) {
            let out = c.take_out();
            return out.is_empty() || s.write_all(&out).is_ok();
        }
        if c.dead() {
            return false;
        }
        match s.read(&mut buf) {
            Ok(n) if n > 0 => c.push(&buf[..n]),
            _ => return false,
        }
    }
    false
}

/// --tls-core-serve PORT: the core as a server: each connection's first bytes come back in an answer
pub fn serve(port: u16) -> i32 {
    let (_, id) = interop_pki();
    let Ok(l) = TcpListener::bind(&SocketAddr::new(NetAddr::LOCALHOST_V4, port)) else { return 1 };
    plat::write_stdout(b"ready\n");
    let mut n = 0u8;
    loop {
        let Ok(Some((s, _))) = l.accept_timeout(20_000) else { return 0 };
        n = n.wrapping_add(1);
        let mut c = Core::server(id.clone(), &[n; 32]);
        pump_socket(&mut c, &s, &mut |c| {
            let got = c.read();
            if got.is_empty() {
                return false;
            }
            let mut a = String::from("sni=");
            a.push_str(c.name());
            a.push_str(" got=");
            a.push_str(&String::from_utf8_lossy(&got));
            c.send(a.as_bytes());
            c.close();
            true
        });
    }
}

/// --tls-core-get PORT: the core as a client of web.lab: exit 0 when the answer came, 2 when the server's
/// certificate was refused
pub fn get(port: u16) -> i32 {
    let (root, _) = interop_pki();
    let Ok(s) = TcpStream::connect(&SocketAddr::new(NetAddr::LOCALHOST_V4, port), 5000) else { return 1 };
    let cfg = ClientCfg { name: plat::arg_value("--host").unwrap_or_else(|| String::from("web.lab")), anchors: alloc::vec![root], now: plat::unix_time_ns() / 1_000_000_000, insecure: false, accept: Vec::new() };
    let mut c = Core::client(cfg, &crypto::sha256(b"tls core get"));
    let mut sent = false;
    let mut answer = Vec::new();
    let ok = pump_socket(&mut c, &s, &mut |c| {
        if c.established() && !sent {
            sent = true;
            c.send(b"hallo vom Kern\n");
        }
        answer.extend_from_slice(&c.read());
        !answer.is_empty()
    });
    plat::write_stdout(&answer);
    if c.faults() != 0 {
        return 2;
    }
    if ok { 0 } else { 1 }
}

fn frame(tcp: &[u8]) -> Vec<u8> {
    let (a, b) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]));
    let mut p = Pkt::new(HEADROOM, tcp);
    TcpHdr { src_port: 49152, dst_port: tls::PORT, seq: 1, ack: 1, flags: ACK | PSH, window: 8192, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, a, b);
    Ipv4Hdr::new(a, b, PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 1]), src: Mac([2, 0, 0, 0, 0, 2]), ethertype: ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

fn field<'a>(l: &'a decode::Layer, name: &str) -> Option<&'a str> {
    l.fields.iter().find(|f| f.name == name).map(|f| f.value.as_str())
}

fn dissector(t: &mut T) {
    let (id, root) = identity(&["web.lab"]);
    let mut c = Core::client(cfg("web.lab", &root), &seed(1));
    let mut s = Core::server(id, &seed(2));
    let hello = c.take_out();
    t.eq("dissect: one record", tls::dissect(&hello), alloc::vec![Part::Record { at: 0, len: hello.len(), ty: tls::CT_HANDSHAKE, version: 0x0301, length: hello.len() - 5 }]);
    let f = frame(&hello);
    let (p, info) = decode::info(&f);
    t.eq("the protocol", p, decode::Proto::Tls);
    t.eq("the line of a ClientHello", info.as_str(), "Client Hello (web.lab)");
    t.check("the mask", decode::proto_mask(&f) & decode::P_TLS != 0);
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Tls);
    t.check("a layer", l.is_some());
    if let Some(l) = l {
        t.eq("its type", field(l, "Handshake Type"), Some("Client Hello (1)"));
        t.eq("its version", field(l, "Version"), Some("TLS 1.2 (0x0303)"));
        t.eq("its suites", field(l, "Cipher Suites"), Some("TLS_CHACHA20_POLY1305_SHA256 (0x1303)"));
        t.eq("its name", field(l, "Extension server_name"), Some("web.lab"));
        t.eq("its versions", field(l, "Extension supported_versions"), Some("TLS 1.3"));
        t.eq("its groups", field(l, "Extension supported_groups"), Some("x25519 (0x001d)"));
        t.eq("its signatures", field(l, "Extension signature_algorithms"), Some("ed25519 (0x0807)"));
        t.check("its share", field(l, "Extension key_share").is_some_and(|v| v.starts_with("x25519: ")));
        t.check("no problems", l.problems.is_empty());
    }
    s.push(&hello);
    let flight = s.take_out();
    let f = frame(&flight);
    let (_, info) = decode::info(&f);
    t.check("the line of the server's flight", info.starts_with("Server Hello, verschlüsselt ("));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == decode::Proto::Tls);
    if let Some(l) = l {
        t.eq("the server's suite", field(l, "Cipher Suite"), Some("TLS_CHACHA20_POLY1305_SHA256 (0x1303)"));
        t.eq("the server's version", field(l, "Extension supported_versions"), Some("TLS 1.3"));
        t.eq("what goes protected", field(l, "Encrypted Record"), Some("verschlüsselt"));
        t.check("the certificate does not show", !l.fields.iter().any(|f| f.name == "Certificate"));
    }
    // a segment that ends inside a record, and the rest of it
    let cut = &flight[..flight.len() - 10];
    let parts = tls::dissect(cut);
    t.check("a record cut", matches!(parts.last(), Some(Part::Record { at, len, length, .. }) if *at + *len == cut.len() && *len < 5 + *length));
    let ls = decode::layers(&frame(cut));
    t.check("it says so", ls.iter().any(|l| l.proto == decode::Proto::Tls && field(l, "Continuation").is_some()));
    t.eq("bytes that start no record", tls::dissect(&[1, 2, 3, 4, 5, 6]), alloc::vec![Part::Rest { at: 0, len: 6 }]);
    // an alert in the clear
    let mut c2 = Core::client(cfg("web.lab", &root), &seed(1));
    c2.push(b"HTTP/1.1 400 Bad Request\r\n\r\n");
    let w = c2.take_out();
    let alert = &w[w.len() - 7..];
    let (_, info) = decode::info(&frame(alert));
    t.eq("an alert's line", info.as_str(), "Alert (protocol_version)");
    let ls = decode::layers(&frame(alert));
    t.check("an alert's fields", ls.iter().any(|l| l.proto == decode::Proto::Tls && field(l, "Alert Level") == Some("fatal (2)") && field(l, "Alert Description") == Some("protocol_version (70)")));
}
