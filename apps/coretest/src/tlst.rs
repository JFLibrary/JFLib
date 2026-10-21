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
            ips: if server { alloc::vec![IpAddr::LOCALHOST_V4] } else { Vec::new() },
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
    loopback(t);
    https(t);
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
