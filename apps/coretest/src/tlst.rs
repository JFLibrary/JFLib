// tlst -- netio's TLS 1.3 subset and X.509: the key schedule against RFC 8448, certificates
// built, parsed and checked, mutual TLS over loopback (large data both ways), refusals (other
// CA, no client certificate, wrong name, wrong pin, expired), HTTPS through netio
//
// Also the --tls-* file modes for tools/tls_interop.py (OpenSSL in WSL).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crypto::ed25519::Keypair;
use netio::tls::{self, ClientConfig, Identity, ServerConfig, TlsError, Trust};
use netio::x509::{verify_chain, Cert, CertBuilder, Usage};
use netio::Stream;
use plat::net::{IpAddr, SocketAddr, TcpListener, TcpStream};

use crate::T;

const NOW: i64 = 1_791_000_000;

fn hx(s: &str) -> Vec<u8> {
    crypto::unhex(s).unwrap_or_default()
}

pub struct Pki {
    pub ca: Cert,
    pub server: Identity,
    pub client: Identity,
}

/// a CA, a server certificate (localhost, 127.0.0.1) and a client certificate
pub fn pki(seed: u8) -> Pki {
    let ca_key = Keypair::from_seed(&[seed; 32]);
    let ca_der = CertBuilder {
        serial: [seed; 16],
        cn: format!("Test CA {}", seed),
        org: String::from("JFLib"),
        not_before: NOW - 86_400,
        not_after: NOW + 3650 * 86_400,
        public_key: ca_key.public,
        is_ca: true,
        ..CertBuilder::default()
    }
    .sign(None, &ca_key);
    let ca = Cert::parse(&ca_der).unwrap_or_default();
    let issue = |s: u8, cn: &str, server: bool| {
        let key = Keypair::from_seed(&[s; 32]);
        let der = CertBuilder {
            serial: [s; 16],
            cn: String::from(cn),
            not_before: NOW - 3600,
            not_after: NOW + 365 * 86_400,
            public_key: key.public,
            dns: if server { alloc::vec![String::from("localhost")] } else { Vec::new() },
            ips: if server { alloc::vec![netio::x509::IpAddr::LOCALHOST_V4] } else { Vec::new() },
            server_auth: server,
            client_auth: !server,
            ..CertBuilder::default()
        }
        .sign(Some(&ca), &ca_key);
        Identity { chain: alloc::vec![der], key }
    };
    Pki { server: issue(seed + 1, "localhost", true), client: issue(seed + 2, "agent-1", false), ca }
}

pub fn run(t: &mut T) {
    schedule(t);
    certs(t);
    restricted_cas(t);
    loopback(t);
    empty_records(t);
    server_records(t);
    client_extensions(t);
    https(t);
}

/// a CA certificate below `root` (None: its own root) with further extensions
fn ca_with(seed: u8, root: Option<(&Cert, &Keypair)>, extra: &[Vec<u8>]) -> (Cert, Keypair) {
    let key = Keypair::from_seed(&[seed; 32]);
    let b = CertBuilder {
        serial: [seed; 16],
        cn: format!("Restricted CA {}", seed),
        not_before: NOW - 86_400,
        not_after: NOW + 3650 * 86_400,
        public_key: key.public,
        is_ca: true,
        ..CertBuilder::default()
    };
    let der = match root {
        Some((c, k)) => b.sign_ext(Some(c), k, extra),
        None => b.sign_ext(None, &key, extra),
    };
    (Cert::parse(&der).unwrap_or_default(), key)
}

/// a server certificate for `dns`, issued by `ca`
fn leaf_of(seed: u8, dns: &str, ca: &Cert, ca_key: &Keypair) -> Cert {
    let key = Keypair::from_seed(&[seed; 32]);
    let der = CertBuilder {
        serial: [seed; 16],
        cn: String::from(dns),
        not_before: NOW - 3600,
        not_after: NOW + 365 * 86_400,
        public_key: key.public,
        dns: alloc::vec![String::from(dns)],
        server_auth: true,
        ..CertBuilder::default()
    }
    .sign(Some(ca), ca_key);
    Cert::parse(&der).unwrap_or_default()
}

/// name constraints with wildcards (a wildcard stands for every name one label below it, so
/// an excluded name among them refuses it) and the purposes of CAs on the path
fn restricted_cas(t: &mut T) {
    use netio::der;
    use netio::x509::extension;
    const NAME_CONSTRAINTS: &[u64] = &[2, 5, 29, 30];
    const EKU: &[u64] = &[2, 5, 29, 37];
    let subtrees = |tag: u8, names: &[&str]| {
        let parts: Vec<Vec<u8>> = names.iter().map(|n| der::seq(&[&der::tlv(0x82, n.as_bytes())])).collect();
        der::tlv(tag, &parts.concat())
    };
    let ok = |leaf: &Cert, ca: &Cert, host: &str| verify_chain(core::slice::from_ref(leaf), core::slice::from_ref(ca), NOW, Some(host), Usage::Server).is_ok();
    // excluded: the host secret.example.com and every subdomain of example.net
    let nc = extension(NAME_CONSTRAINTS, true, &der::seq(&[&subtrees(0xa1, &["secret.example.com", ".example.net"])]));
    let (ca, ck) = ca_with(60, None, &[nc]);
    t.check("constraints: an excluded host is refused", !ok(&leaf_of(61, "secret.example.com", &ca, &ck), &ca, "secret.example.com"));
    t.check("constraints: another host passes", ok(&leaf_of(62, "www.example.com", &ca, &ck), &ca, "www.example.com"));
    t.check("constraints: a wildcard that stands for an excluded host is refused", !ok(&leaf_of(63, "*.example.com", &ca, &ck), &ca, "www.example.com"));
    t.check("constraints: a wildcard below excluded subdomains is refused", !ok(&leaf_of(64, "*.example.net", &ca, &ck), &ca, "www.example.net"));
    t.check("constraints: a wildcard elsewhere passes", ok(&leaf_of(65, "*.example.org", &ca, &ck), &ca, "www.example.org"));
    // permitted: example.com and its subdomains
    let nc = extension(NAME_CONSTRAINTS, true, &der::seq(&[&subtrees(0xa0, &["example.com"])]));
    let (pca, pk) = ca_with(66, None, &[nc]);
    t.check("constraints: a wildcard inside a permitted tree passes", ok(&leaf_of(67, "*.example.com", &pca, &pk), &pca, "a.example.com"));
    t.check("constraints: a wildcard outside it is refused", !ok(&leaf_of(68, "*.example.org", &pca, &pk), &pca, "a.example.org"));
    // an intermediate CA for client certificates only cannot certify a server (the root's
    // basic constraints without a path length: the builder's CAs have path length 0)
    let unbounded = extension(&[2, 5, 29, 19], true, &der::seq(&[&der::boolean(true)]));
    let (root, rk) = ca_with(70, None, &[unbounded]);
    let eku = |oids: &[&[u64]]| {
        let parts: Vec<Vec<u8>> = oids.iter().map(|o| der::oid(o)).collect();
        let refs: Vec<&[u8]> = parts.iter().map(|v| v.as_slice()).collect();
        extension(EKU, false, &der::seq(&refs))
    };
    let path_ok = |inter: &(Cert, Keypair), seed: u8| {
        let leaf = leaf_of(seed, "srv.lab", &inter.0, &inter.1);
        verify_chain(&[leaf, inter.0.clone()], core::slice::from_ref(&root), NOW, Some("srv.lab"), Usage::Server).is_ok()
    };
    let client_only = ca_with(71, Some((&root, &rk)), &[eku(&[&[1, 3, 6, 1, 5, 5, 7, 3, 2]])]);
    t.check("purpose: an intermediate for client certificates cannot certify a server", !path_ok(&client_only, 72));
    let server = ca_with(73, Some((&root, &rk)), &[eku(&[&[1, 3, 6, 1, 5, 5, 7, 3, 1]])]);
    t.check("purpose: an intermediate for servers can", path_ok(&server, 74));
    let any = ca_with(75, Some((&root, &rk)), &[eku(&[&[2, 5, 29, 37, 0]])]);
    t.check("purpose: an intermediate for any purpose can", path_ok(&any, 76));
    let plain = ca_with(77, Some((&root, &rk)), &[]);
    t.check("purpose: an intermediate without the extension can", path_ok(&plain, 78));
    // a trusted CA restricted to clients does not anchor a server
    let (anchor, ak) = ca_with(79, None, &[eku(&[&[1, 3, 6, 1, 5, 5, 7, 3, 2]])]);
    t.check("purpose: a trusted CA for client certificates does not anchor a server", !ok(&leaf_of(80, "srv.lab", &anchor, &ak), &anchor, "srv.lab"));
}

/// records without data (empty application data records) do not keep a read going past the
/// stream's read timeout (an HTTP deadline is checked between reads)
fn empty_records(t: &mut T) {
    let p = pki(50);
    let scfg = ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false };
    let (addr, th) = server(scfg, |r| {
        if let Ok(s) = r {
            for _ in 0..60 {
                if s.test_send_empty_record().is_err() {
                    break;
                }
                plat::sleep_ms(50);
            }
            s.shutdown();
        }
    });
    let ccfg = ClientConfig { trust: Trust::Anchors(alloc::vec![p.ca.clone()]), identity: None, server_name: String::from("localhost") };
    match client(&addr, ccfg) {
        Ok(c) => {
            c.set_timeouts(Some(300), None);
            let t0 = plat::time_us();
            let mut b = [0u8; 16];
            let r = c.read(&mut b);
            let ms = (plat::time_us() - t0) / 1000;
            t.check(&format!("empty records: a read ends with its timeout ({ms} ms)"),
                    r.is_err_and(|e| e.kind == plat::net::NetErrorKind::TimedOut) && ms < 1500);
        }
        Err(e) => t.check(&format!("empty records: handshake: {}", e), false),
    }
    th.join();
}

/// netio's server fed hand-made records around a real ClientHello (RFC 8446, what tlsfuzzer
/// checks): records between the parts of a handshake message, pre_shared_key's place
fn server_records(t: &mut T) {
    let p = pki(51);
    let ccfg = ClientConfig { trust: Trust::Anchors(alloc::vec![p.ca.clone()]), identity: None, server_name: String::from("localhost") };
    let ch_rec = tls::Conn::client(ccfg).take_out();
    let ch = ch_rec[5..].to_vec();
    let record = |ty: u8, b: &[u8]| {
        let mut r = alloc::vec![ty, 3, 1, (b.len() >> 8) as u8, b.len() as u8];
        r.extend_from_slice(b);
        r
    };
    let feed = |bytes: &[u8]| {
        let mut s = tls::Conn::server(ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false });
        let r = s.feed(bytes);
        (r, s.take_out())
    };
    let alert = |r: &Result<(), TlsError>| match r {
        Err(TlsError::Alert(a, _)) => Some(*a),
        _ => None,
    };
    let whole = feed(&record(22, &ch));
    t.check("server records: the ClientHello as it is gets a ServerHello", whole.0.is_ok() && whole.1.first() == Some(&22));
    let mut split = record(22, &ch[..2]);
    split.extend(record(22, &ch[2..]));
    t.check("server records: a ClientHello in two records is taken", feed(&split).0.is_ok());
    // an empty application_data record between its parts is no 0-RTT data (there is no
    // ClientHello yet) and is not skipped: unexpected_message (RFC 8446 5.1)
    let mut between = record(22, &ch[..2]);
    between.extend(record(23, &[]));
    between.extend(record(22, &ch[2..]));
    t.check("server records: an empty application_data record inside the ClientHello is refused", alert(&feed(&between).0) == Some(tls::conn::AL_UNEXPECTED_MESSAGE));
    // pre_shared_key (an identity, a binder) must be the last extension (RFC 8446 4.2.11)
    let mut psk = alloc::vec![0, 41, 0, 45, 0, 8, 0, 2, b'i', b'd', 0, 0, 0, 0, 0, 33, 32];
    psk.extend_from_slice(&[7u8; 32]);
    let with_psk = |first: bool| {
        // legacy_version, random, session id, suites, compression, then the extensions
        let sid = 4 + 2 + 32;
        let suites = sid + 1 + ch[sid] as usize;
        let comp = suites + 2 + ((ch[suites] as usize) << 8 | ch[suites + 1] as usize);
        let exts = comp + 1 + ch[comp] as usize;
        let mut m = ch[..exts + 2].to_vec();
        if first {
            m.extend_from_slice(&psk);
            m.extend_from_slice(&ch[exts + 2..]);
        } else {
            m.extend_from_slice(&ch[exts + 2..]);
            m.extend_from_slice(&psk);
        }
        let ext_len = m.len() - exts - 2;
        m[exts..exts + 2].copy_from_slice(&(ext_len as u16).to_be_bytes());
        let body = m.len() - 4;
        m[1..4].copy_from_slice(&(body as u32).to_be_bytes()[1..]);
        record(22, &m)
    };
    t.check("server records: pre_shared_key as the last extension is taken (and ignored)", feed(&with_psk(false)).0.is_ok());
    t.check("server records: pre_shared_key before another extension is refused", alert(&feed(&with_psk(true)).0) == Some(tls::conn::AL_ILLEGAL_PARAMETER));
}

/// a server's first flight with one extension more in its ServerHello (the first record, put
/// before the others; the lengths of the extensions, the message and the record follow)
pub fn sh_with_ext(flight: &[u8], extension: &[u8]) -> Vec<u8> {
    let n = 5 + ((flight[3] as usize) << 8 | flight[4] as usize);
    let sh = &flight[5..n];
    // type and length, legacy_version, random, session id, suite, compression, extensions
    let at = 4 + 2 + 32 + 1 + sh[38] as usize + 2 + 1;
    let mut m = sh[..at].to_vec();
    let old = (sh[at] as usize) << 8 | sh[at + 1] as usize;
    m.extend_from_slice(&((old + extension.len()) as u16).to_be_bytes());
    m.extend_from_slice(extension);
    m.extend_from_slice(&sh[at + 2..]);
    let body = m.len() - 4;
    m[1..4].copy_from_slice(&(body as u32).to_be_bytes()[1..]);
    let mut out = alloc::vec![flight[0], flight[1], flight[2]];
    out.extend_from_slice(&(m.len() as u16).to_be_bytes());
    out.extend_from_slice(&m);
    out.extend_from_slice(&flight[n..]);
    out
}

/// netio's client takes in a ServerHello only the extensions it offered that belong there (RFC
/// 8446 4.2): a GREASE value (RFC 8701 4: TLS-Anvil's finding) is unsupported_extension,
/// server_name (offered, but EncryptedExtensions answers it) illegal_parameter
fn client_extensions(t: &mut T) {
    let p = pki(52);
    let answer = |extension: &[u8]| {
        let ccfg = ClientConfig { trust: Trust::Anchors(alloc::vec![p.ca.clone()]), identity: None, server_name: String::from("localhost") };
        let mut c = tls::Conn::client(ccfg);
        let mut s = tls::Conn::server(ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false });
        let _ = s.feed(&c.take_out());
        let flight = s.take_out();
        let r = c.feed(&if extension.is_empty() { flight } else { sh_with_ext(&flight, extension) });
        match r {
            Ok(()) => Some(0),
            Err(TlsError::Alert(a, _)) => Some(a),
            Err(_) => None,
        }
    };
    t.check("client extensions: the server's flight as it is is taken", answer(&[]) == Some(0));
    t.check("client extensions: a GREASE extension in the ServerHello is refused", answer(&[0x0a, 0x0a, 0, 1, 0]) == Some(tls::conn::AL_UNSUPPORTED_EXTENSION));
    t.check("client extensions: server_name in the ServerHello is refused", answer(&[0, 0, 0, 0]) == Some(tls::conn::AL_ILLEGAL_PARAMETER));
    // the ServerHello in records of one byte each (TLS-Scanner's fragmentation probe could not
    // tell; tlslite-ng's server with records of 1, 50 and 111 bytes was taken)
    let ccfg = ClientConfig { trust: Trust::Anchors(alloc::vec![p.ca.clone()]), identity: None, server_name: String::from("localhost") };
    let mut c = tls::Conn::client(ccfg);
    let mut s = tls::Conn::server(ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false });
    let _ = s.feed(&c.take_out());
    let flight = s.take_out();
    let n = 5 + ((flight[3] as usize) << 8 | flight[4] as usize);
    let mut bytes = Vec::new();
    for b in &flight[5..n] {
        bytes.extend_from_slice(&[22, 3, 3, 0, 1, *b]);
    }
    bytes.extend_from_slice(&flight[n..]);
    let r = c.feed(&bytes);
    let _ = s.feed(&c.take_out());
    t.check("client records: a ServerHello in records of one byte is taken", r.is_ok() && c.established() && s.established());
}

fn schedule(t: &mut T) {
    // RFC 8448, "Simple 1-RTT Handshake"
    let mut ecdhe = [0u8; 32];
    ecdhe.copy_from_slice(&hx("8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d"));
    let s = tls::keysched::Schedule::new(&ecdhe);
    t.eq("handshake secret", crypto::hex(&s.handshake), String::from("1dc826e93606aa6fdc0aadc12f741b01046aa6b99f691ed221a9f0ca043fbeac"));
    t.eq("master secret", crypto::hex(&s.master), String::from("18df06843d13a08bf2a449844c5f8a478001bc4d4c627984d5a41da8d0402919"));
    let mut h = [0u8; 32];
    h.copy_from_slice(&hx("860c06edc07858ee8e78f0e7428c58edd6b43f2ca3e6e95f02ed063cf0e1cad8"));
    t.eq("c hs traffic", crypto::hex(&tls::keysched::derive(&s.handshake, "c hs traffic", &h)), String::from("b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21"));
    t.eq("s hs traffic", crypto::hex(&tls::keysched::derive(&s.handshake, "s hs traffic", &h)), String::from("b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38"));
}

fn certs(t: &mut T) {
    let p = pki(10);
    t.check("CA parses", p.ca.is_ca && p.ca.subject == p.ca.issuer && p.ca.signed_by(&p.ca));
    let leaf = Cert::parse(&p.server.chain[0]).unwrap_or_default();
    t.check("leaf parses", leaf.subject_cn == "localhost" && leaf.names("LOCALHOST") && leaf.names("127.0.0.1") && !leaf.names("example.org") && leaf.signed_by(&p.ca));
    t.check("chain ok", verify_chain(&[leaf.clone()], &[p.ca.clone()], NOW, Some("localhost"), Usage::Server).is_ok());
    t.check("wrong name", verify_chain(&[leaf.clone()], &[p.ca.clone()], NOW, Some("other"), Usage::Server).is_err());
    t.check("wrong usage", verify_chain(&[leaf.clone()], &[p.ca.clone()], NOW, None, Usage::Client).is_err());
    t.check("expired", verify_chain(&[leaf.clone()], &[p.ca.clone()], NOW + 400 * 86_400, None, Usage::Server).is_err());
    let other = pki(20);
    t.check("other CA", verify_chain(&[leaf.clone()], &[other.ca.clone()], NOW, None, Usage::Server).is_err());
    let mut bad = p.server.chain[0].clone();
    let n = bad.len();
    bad[n - 10] ^= 1; // the signature
    t.check("bad signature", Cert::parse(&bad).is_some_and(|c| !c.signed_by(&p.ca)));
    t.check("garbage refused", Cert::parse(b"\x30\x03\x02\x01\x00").is_none() && Cert::parse(&p.server.chain[0][..100]).is_none());
    let pem = netio::pem::encode("CERTIFICATE", &p.server.chain[0]);
    t.eq("pem round trip", netio::pem::decode(&pem, "CERTIFICATE"), Some(p.server.chain[0].clone()));
    let k8 = netio::x509::ed25519_pkcs8(&[7u8; 32]);
    t.eq("pkcs8 round trip", netio::x509::ed25519_from_pkcs8(&k8), Some([7u8; 32]));
}

/// one TLS server connection on a free port; `serve` gets the stream
fn server(cfg: ServerConfig, serve: impl FnOnce(Result<tls::TlsStream, TlsError>) + Send + 'static) -> (SocketAddr, plat::Thread) {
    let l = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)).ok().expect("bind");
    let addr = l.local_addr().unwrap_or(SocketAddr::new(IpAddr::LOCALHOST_V4, 0));
    let th = plat::thread_spawn(0, Box::new(move || {
        if let Ok(Some((s, _))) = l.accept_timeout(5000) {
            s.set_read_timeout(Some(5000));
            serve(tls::accept(Box::new(s), cfg));
        }
    }))
    .expect("thread");
    (addr, th)
}

fn client(addr: &SocketAddr, cfg: ClientConfig) -> Result<tls::TlsStream, TlsError> {
    let s = TcpStream::connect(addr, 2000).map_err(TlsError::Net)?;
    s.set_read_timeout(Some(5000));
    tls::connect(Box::new(s), cfg)
}

fn loopback(t: &mut T) {
    let p = pki(30);
    let anchors = Trust::Anchors(alloc::vec![p.ca.clone()]);
    // mutual TLS, 1 MB each way, an echo
    let scfg = ServerConfig { identity: p.server.clone(), client_auth: Some(anchors.clone()), client_auth_optional: false };
    let seen = Arc::new(plat::sync::Mutex::new(String::new()));
    let seen2 = seen.clone();
    let (addr, th) = server(scfg, move |r| {
        if let Ok(s) = r {
            *seen2.lock() = s.peer_certs().first().map(|c| c.subject_cn.clone()).unwrap_or_default();
            let mut buf = alloc::vec![0u8; 65536];
            let mut total = 0;
            while total < 1_000_000 {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        total += n;
                        let _ = s.write_all(&buf[..n]);
                    }
                }
            }
            s.shutdown();
        }
    });
    let ccfg = ClientConfig { trust: anchors.clone(), identity: Some(p.client.clone()), server_name: String::from("localhost") };
    match client(&addr, ccfg) {
        Ok(c) => {
            let data: Vec<u8> = (0..1_000_000u32).map(|i| (i * 13) as u8).collect();
            let c = Arc::new(c);
            let w = c.clone();
            let d2 = data.clone();
            let writer = plat::thread_spawn(0, Box::new(move || {
                let _ = w.write_all(&d2);
            }));
            let mut got = Vec::new();
            let mut buf = alloc::vec![0u8; 65536];
            while got.len() < data.len() {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got.extend_from_slice(&buf[..n]),
                }
            }
            if let Some(w) = writer {
                w.join();
            }
            t.check("mTLS echo 1 MB", got == data);
            t.check("server certificate seen", c.peer_certs().first().is_some_and(|x| x.subject_cn == "localhost"));
        }
        Err(e) => t.check(&format!("mTLS handshake: {}", e), false),
    }
    th.join();
    t.eq("client certificate seen", seen.lock().clone(), String::from("agent-1"));

    // refusals
    let refuse = |t: &mut T, what: &str, scfg: ServerConfig, ccfg: ClientConfig| {
        let (addr, th) = server(scfg, |r| {
            if let Ok(s) = r {
                let mut b = [0u8; 16];
                let _ = s.read(&mut b);
            }
        });
        let r: Result<(), TlsError> = client(&addr, ccfg).and_then(|c| {
            // a server refusal of the client certificate arrives with the first read
            c.write_all(b"x").map_err(TlsError::Net)?;
            let mut b = [0u8; 4];
            match c.read(&mut b) {
                Ok(_) => Err(c.last_error().unwrap_or(TlsError::Closed)),
                Err(_) => Err(c.last_error().unwrap_or(TlsError::Closed)),
            }
        });
        th.join();
        t.check(what, r.is_err());
    };
    let other = pki(40);
    refuse(t, "unknown CA", ServerConfig { identity: other.server.clone(), client_auth: None, client_auth_optional: false },
           ClientConfig { trust: anchors.clone(), identity: None, server_name: String::from("localhost") });
    refuse(t, "client certificate required", ServerConfig { identity: p.server.clone(), client_auth: Some(anchors.clone()), client_auth_optional: false },
           ClientConfig { trust: anchors.clone(), identity: None, server_name: String::from("localhost") });
    refuse(t, "client certificate of another CA", ServerConfig { identity: p.server.clone(), client_auth: Some(anchors.clone()), client_auth_optional: true },
           ClientConfig { trust: anchors.clone(), identity: Some(other.client.clone()), server_name: String::from("localhost") });
    refuse(t, "wrong name", ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false },
           ClientConfig { trust: anchors.clone(), identity: None, server_name: String::from("backup.example") });
    refuse(t, "wrong pin", ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false },
           ClientConfig { trust: Trust::Pinned(alloc::vec![[0u8; 32]]), identity: None, server_name: String::new() });
    // the right pin
    let pin = crypto::sha256(&p.server.chain[0]);
    let (addr, th) = server(ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false }, |r| {
        if let Ok(s) = r {
            let _ = s.write_all(b"pinned ok");
            s.shutdown();
        }
    });
    let ok = client(&addr, ClientConfig { trust: Trust::Pinned(alloc::vec![pin]), identity: None, server_name: String::new() }).map(|c| {
        let mut b = [0u8; 16];
        let n = c.read(&mut b).unwrap_or(0);
        Vec::from(&b[..n])
    });
    th.join();
    t.check("right pin", ok.as_deref() == Ok(&b"pinned ok"[..]));
}

fn https(t: &mut T) {
    use netio::http::{Headers, Request, Response};
    let p = pki(50);
    let scfg = ServerConfig { identity: p.server.clone(), client_auth: Some(Trust::Anchors(alloc::vec![p.ca.clone()])), client_auth_optional: true };
    let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| {
        let s = tls::accept(Box::new(tcp), scfg.clone()).ok()?;
        Some((Box::new(s) as Box<dyn Stream>, None))
    });
    let srv = netio::Server::start_with(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0), netio::ServerConfig { threads: 2, ..netio::ServerConfig::default() },
                                        Arc::new(|r: &Request| Response::text(200, &format!("über TLS: {}", r.path()))), Some(wrap));
    let Ok(srv) = srv else {
        t.check("https server", false);
        return;
    };
    let ca = p.ca.clone();
    let connector: netio::client::Connector = Arc::new(move |url: &netio::Url, tcp: TcpStream| {
        let cfg = ClientConfig { trust: Trust::Anchors(alloc::vec![ca.clone()]), identity: None, server_name: url.host.clone() };
        tls::connect(Box::new(tcp), cfg).map(|s| Box::new(s) as Box<dyn Stream>).map_err(|e| netio::HttpError::Tls(format!("{}", e)))
    });
    let c = netio::Client::new().with_connector(connector);
    let url = netio::Url::parse(&format!("https://localhost:{}/hallo", srv.local_addr().port));
    let r = url.map(|u| c.get(&u, &Headers::new()));
    t.check("https GET", matches!(r, Some(Ok(ref x)) if x.status == 200 && x.body_bytes() == "über TLS: /hallo".as_bytes()));
    srv.stop();
}

// ------------------------------------------------------------------ interop modes

/// --tls-pki DIR: write ca.pem, server.pem / server.key, client.pem / client.key (PKCS#8)
pub fn write_pki(dir: &str) -> i32 {
    let p = pki(60);
    let w = |name: &str, s: String| plat::write_file(&plat::fs::join(dir, name), s.as_bytes());
    let key = |id: &Identity| netio::pem::encode("PRIVATE KEY", &netio::x509::ed25519_pkcs8(id.key.seed()));
    let ok = w("ca.pem", netio::pem::encode("CERTIFICATE", &p.ca.der))
        && w("server.pem", netio::pem::encode("CERTIFICATE", &p.server.chain[0]))
        && w("server.key", key(&p.server))
        && w("client.pem", netio::pem::encode("CERTIFICATE", &p.client.chain[0]))
        && w("client.key", key(&p.client));
    if ok { 0 } else { 1 }
}

/// --tls-serve PORT [--mtls]: answer one HTTPS request per connection (for curl / openssl)
pub fn serve(port: u16, mtls: bool) -> i32 {
    let p = pki(60);
    let scfg = ServerConfig { identity: p.server.clone(), client_auth: if mtls { Some(Trust::Anchors(alloc::vec![p.ca.clone()])) } else { None }, client_auth_optional: false };
    let wrap: netio::server::Wrap = Arc::new(move |tcp: TcpStream| match tls::accept(Box::new(tcp), scfg.clone()) {
        Ok(s) => {
            let who = s.peer_certs().first().map(|c| c.subject_cn.clone());
            Some((Box::new(s) as Box<dyn Stream>, who))
        }
        Err(e) => {
            plat::write_stderr(format!("handshake: {}\n", e).as_bytes());
            None
        }
    });
    let h = |r: &netio::http::Request| netio::http::Response::text(200, &format!("JFLib tls ok {}\n", r.client_id.clone().unwrap_or_default()));
    match netio::Server::start_with(&SocketAddr::new(IpAddr::LOCALHOST_V4, port), netio::ServerConfig { threads: 2, ..netio::ServerConfig::default() }, Arc::new(h), Some(wrap)) {
        Ok(_srv) => loop {
            plat::sleep_ms(1000);
        },
        Err(_) => 1,
    }
}

/// --tls-echo PORT: netio's TLS server sending every line back as it came, in one write (one
/// record up to 2^14 bytes); tlsfuzzer's test-tls13-lengths expects that of its server (it names
/// `openssl s_server -rev`, which keeps the length too).  Four threads take the connections
pub fn echo(port: u16) -> i32 {
    let p = pki(60);
    let scfg = ServerConfig { identity: p.server.clone(), client_auth: None, client_auth_optional: false };
    let Ok(l) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, port)) else { return 1 };
    let l = Arc::new(l);
    let mut threads = Vec::new();
    for _ in 0..4 {
        let (l, scfg) = (l.clone(), scfg.clone());
        threads.extend(plat::thread_spawn(256 * 1024, Box::new(move || loop {
            let Ok(Some((tcp, _))) = l.accept_timeout(1000) else { continue };
            tcp.set_read_timeout(Some(10_000));
            let Ok(s) = tls::accept(Box::new(tcp), scfg.clone()) else { continue };
            let (mut line, mut b) = (Vec::new(), [0u8; 4096]);
            while let Ok(n) = s.read(&mut b) {
                if n == 0 {
                    break;
                }
                for &c in &b[..n] {
                    line.push(c);
                    if c == b'\n' {
                        let _ = s.write_all(&line);
                        line.clear();
                    }
                }
            }
        })));
    }
    for t in threads {
        t.join();
    }
    1
}

/// --tls-get HOST PORT [--mtls]: one HTTPS GET against an OpenSSL server -> the body on stdout
pub fn get(host: &str, port: u16, mtls: bool) -> i32 {
    let p = pki(60);
    let cfg = ClientConfig { trust: Trust::Anchors(alloc::vec![p.ca.clone()]), identity: if mtls { Some(p.client.clone()) } else { None }, server_name: String::from(host) };
    let Ok(tcp) = TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, port), 3000) else { return 3 };
    tcp.set_read_timeout(Some(5000));
    match tls::connect(Box::new(tcp), cfg) {
        Ok(s) => {
            let _ = s.write_all(format!("GET / HTTP/1.0\r\nHost: {}\r\n\r\n", host).as_bytes());
            let mut out = Vec::new();
            let mut b = [0u8; 4096];
            while let Ok(n) = s.read(&mut b) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&b[..n]);
            }
            plat::write_stdout(&out);
            0
        }
        Err(e) => {
            plat::write_stderr(format!("tls: {}\n", e).as_bytes());
            1
        }
    }
}

/// --tls-anvil-client PORT: one connection of netio's web client (TLS 1.3 and the TLS 1.2
/// fallback, the public suites) to TLS-Anvil on 127.0.0.1:PORT, the trigger of
/// tools/tlsanvil_check.py.  TLS-Anvil makes its certificates up for every test, so the check is
/// a Trust::Custom that takes every chain (in this test program only); TLS-Anvil judges the
/// client by what it sends.  An HTTP GET, read until the end or 5 s, then close_notify as a
/// program closes a connection (RFC 8446 6.1); how it ended goes to stderr, one line
pub fn anvil_client(port: u16) -> i32 {
    let accept_all: netio::tls::Verifier = Arc::new(|_, _, _| Ok(()));
    let cfg = ClientConfig { trust: Trust::Custom(accept_all), identity: None, server_name: String::from("localhost") };
    let Ok(tcp) = TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, port), 3000) else { return 3 };
    tcp.set_read_timeout(Some(5000));
    match tls::connect(Box::new(tcp), cfg) {
        Ok(s) => {
            let _ = s.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
            let (mut b, mut got) = ([0u8; 4096], 0);
            let end = loop {
                match s.read(&mut b) {
                    Ok(0) => break String::from(if s.end_is_clean() { "close_notify" } else { "end without close_notify" }),
                    Ok(n) => got += n,
                    Err(e) => break format!("read: {}", e),
                }
            };
            s.shutdown();
            plat::write_stderr(format!("{}: {} bytes, {}\n", s.version().0, got, end).as_bytes());
            0
        }
        Err(e) => {
            plat::write_stderr(format!("handshake: {}\n", e).as_bytes());
            1
        }
    }
}

/// --tls-web URL [--ca FILE]: an HTTPS GET with the system's CAs (and FILE's): prints
/// "VERSION SUITE STATUS BYTES" (tools/tls_web_check.py) or the error
pub fn web(url: &str, ca: Option<&str>) -> i32 {
    let extra: Vec<netio::x509::Cert> = ca
        .and_then(plat::read_file)
        .map(|b| netio::pem::decode_all(&String::from_utf8_lossy(&b), "CERTIFICATE").iter().filter_map(|d| netio::x509::Cert::parse(d)).collect())
        .unwrap_or_default();
    let Some(u) = netio::Url::parse(url) else { return 2 };
    let seen: Arc<plat::sync::Mutex<(&'static str, u16)>> = Arc::new(plat::sync::Mutex::new(("", 0)));
    let s2 = seen.clone();
    let connector: netio::client::Connector = Arc::new(move |url: &netio::Url, tcp: TcpStream| {
        let cfg = ClientConfig { trust: Trust::System(extra.clone()), identity: None, server_name: url.host.clone() };
        let s = tls::connect(Box::new(tcp), cfg).map_err(|e| netio::HttpError::Tls(format!("{}", e)))?;
        *s2.lock() = s.version();
        Ok(Box::new(s) as Box<dyn Stream>)
    });
    let c = netio::Client::new().with_connector(connector);
    match c.get(&u, &netio::http::Headers::new()) {
        Ok(r) => {
            let (v, suite) = *seen.lock();
            plat::write_stdout(format!("{} 0x{:04X} {} {}\n", v, suite, r.status, r.body_bytes().len()).as_bytes());
            0
        }
        Err(e) => {
            plat::write_stdout(format!("error {}\n", e).as_bytes());
            1
        }
    }
}
