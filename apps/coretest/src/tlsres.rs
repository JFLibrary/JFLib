// tlsres -- TLS 1.3 resumption and 0-RTT (RFC 8446 2.2, 2.3, 4.6.1, 8): tickets a server sends
// and takes back, a client's cache of them (single use, kept per server, trust and identity),
// key rotation and shared keys, client certificates across a resumption, the limits (lifetime,
// 7 days from the authenticating handshake, the server name, the client trust), a binder that
// does not fit; 0-RTT taken, refused (replayed, stale, another ALPN protocol, off), its limit,
// its data before and after the handshake, and the same over TCP (connect_early, read_early)

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use netio::tls::{self, ClientConfig, Conn, ServerConfig, Sessions, Tickets, TlsError, Trust};
use netio::Stream;
use plat::net::{IpAddr, SocketAddr, TcpListener, TcpStream};

use crate::tlsweb::{pki, pump, Kind, Pki};
use crate::T;

const DAY: i64 = 86_400_000;

pub fn run(t: &mut T) {
    let p = pki(90, Kind::P256);
    resumption(t, &p);
    limits(t, &p);
    keys(t, &p);
    client_certs(t, &p);
    cache(t, &p);
    early(t, &p);
    early_refused(t, &p);
    retry(t, &p);
    over_tcp(t, &p);
}

/// a handshake with the server's suites and groups set (Conn::test_server_prefs)
fn go_prefs(c: ClientConfig, s: ServerConfig, suites: &[u16], groups: &[u16]) -> Result<(Conn, Conn), TlsError> {
    let mut c = Conn::client(c);
    let mut s = Conn::server(s);
    s.test_server_prefs(suites.to_vec(), groups.to_vec());
    pump(&mut c, &mut s)?;
    Ok((c, s))
}

/// resumption after a HelloRetryRequest, sessions of SHA-384
fn retry(t: &mut T, p: &Pki) {
    use netio::tls::record::{TLS_AES_256_GCM_SHA384, TLS_CHACHA20_POLY1305_SHA256};
    let p256 = [tls::kx::SECP256R1];
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    let r = go_prefs(ccfg(p, &ss), scfg(p, &tk), &[], &p256);
    t.check("retry: resumed after a HelloRetryRequest (P-256)", r.as_ref().is_ok_and(|(c, s)| c.resumed() && s.resumed() && c.group() == tls::kx::SECP256R1));
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go_prefs(ccfg(p, &ss), scfg(p, &tk), &[TLS_AES_256_GCM_SHA384], &[]);
    let r = go_prefs(ccfg(p, &ss), scfg(p, &tk), &[TLS_AES_256_GCM_SHA384], &[]);
    t.check("retry: a session of TLS_AES_256_GCM_SHA384 resumes", r.as_ref().is_ok_and(|(c, s)| c.resumed() && s.resumed() && c.suite() == TLS_AES_256_GCM_SHA384));
    // a HelloRetryRequest for a suite of SHA-256: the SHA-384 ticket goes, psk_key_exchange_modes
    // stays (the server checks that the second ClientHello keeps it)
    let r = go_prefs(ccfg(p, &ss), scfg(p, &tk), &[TLS_CHACHA20_POLY1305_SHA256], &p256);
    t.check("retry: ... a HelloRetryRequest to SHA-256 drops its ticket: a full handshake", r.as_ref().is_ok_and(|(c, s)| !c.resumed() && !s.resumed() && c.suite() == TLS_CHACHA20_POLY1305_SHA256));
}

fn anchors(p: &Pki) -> Trust {
    Trust::Anchors(alloc::vec![p.ca.clone()])
}

fn ccfg(p: &Pki, sessions: &Arc<Sessions>) -> ClientConfig {
    ClientConfig { sessions: Some(sessions.clone()), ..ClientConfig::new(anchors(p), "localhost") }
}

fn scfg(p: &Pki, tickets: &Arc<Tickets>) -> ServerConfig {
    ServerConfig { tickets: Some(tickets.clone()), ..ServerConfig::new(p.server.clone()) }
}

/// a handshake in memory (and the tickets after it), then a message each way
fn go(c: ClientConfig, s: ServerConfig) -> Result<(Conn, Conn), TlsError> {
    let mut c = Conn::client(c);
    let mut s = Conn::server(s);
    pump(&mut c, &mut s)?;
    c.write_plain(b"ping")?;
    s.feed(&c.take_out())?;
    s.write_plain(b"pong")?;
    c.feed(&s.take_out())?;
    let mut b = [0u8; 8];
    if s.read_plain(&mut b) != 4 || c.read_plain(&mut b) != 4 {
        return Err(TlsError::Closed);
    }
    Ok((c, s))
}

fn resumed(r: &Result<(Conn, Conn), TlsError>) -> Option<bool> {
    r.as_ref().ok().map(|(c, s)| c.resumed() && s.resumed())
}

fn resumption(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.check("resumption: a full handshake", r.as_ref().is_ok_and(|(c, s)| !c.resumed() && !s.resumed()));
    t.eq("resumption: it leaves two tickets", ss.len(), 2);
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("resumption: the second connection resumes", resumed(&r), Some(true));
    t.check("resumption: ... and shows the server's certificate of the first", r.as_ref().is_ok_and(|(c, _)| c.peer_certs().first().is_some_and(|x| x.key == p.server.key.public_key())));
    t.check("resumption: ... with a fresh key exchange", r.as_ref().is_ok_and(|(c, s)| c.group() == tls::kx::X25519_MLKEM768 && s.group() == c.group()));
    t.eq("resumption: a ticket is used once, the server sends two new", ss.len(), 3);
    // a server without tickets: a full handshake, the ticket spent
    let r = go(ccfg(p, &ss), ServerConfig::new(p.server.clone()));
    t.eq("resumption: a server without tickets does a full handshake", resumed(&r), Some(false));
    // a server of other tickets: the same
    let r = go(ccfg(p, &ss), scfg(p, &Arc::new(Tickets::new())));
    t.eq("resumption: tickets of another server are not taken", resumed(&r), Some(false));
    // one ticket per handshake
    let (ss1, tk1) = (Arc::new(Sessions::default()), Arc::new(Tickets::new().per_handshake(1)));
    let _ = go(ccfg(p, &ss1), scfg(p, &tk1));
    t.eq("resumption: per_handshake(1)", ss1.len(), 1);
    // a client without a cache ignores the tickets
    let r = go(ClientConfig::new(anchors(p), "localhost"), scfg(p, &tk));
    t.eq("resumption: a client without sessions ignores tickets", resumed(&r), Some(false));
    // a changed binder ends the handshake (decrypt_error)
    let ss = Arc::new(Sessions::default());
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    let mut c = Conn::client(ccfg(p, &ss));
    let mut ch = c.take_out();
    let n = ch.len();
    ch[n - 1] ^= 1;
    let mut s = Conn::server(scfg(p, &tk));
    t.check("resumption: a binder that does not fit is decrypt_error", matches!(s.feed(&ch), Err(TlsError::Alert(51, _))));
}

fn limits(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new().lifetime(3600)));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    // the server's lifetime
    tk.test_shift(3601_000);
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("limits: a ticket past its lifetime is not taken", resumed(&r), Some(false));
    // the client's: the tickets of that handshake are fresh for the server, past their
    // lifetime for the client, which offers none
    ss.test_shift(3601_000);
    let kept = ss.len();
    let mut c = Conn::client(ccfg(p, &ss));
    let mut s = Conn::server(scfg(p, &tk));
    let _ = pump(&mut c, &mut s);
    t.check("limits: ... nor offered by the client past its lifetime", kept > 0 && !s.resumed() && c.established());
    // 7 days from the handshake that authenticated the server, however often renewed
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new().lifetime(7 * 86_400)));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.test_shift(6 * DAY);
    ss.test_shift(6 * DAY);
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("limits: resumed after 6 days", resumed(&r), Some(true));
    tk.test_shift(2 * DAY);
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("limits: the renewed ticket is refused 8 days after the first handshake (server)", resumed(&r), Some(false));
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new().lifetime(7 * 86_400)));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.test_shift(6 * DAY);
    ss.test_shift(6 * DAY);
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.test_shift(2 * DAY);
    ss.test_shift(2 * DAY);
    let n = ss.len();
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.check("limits: ... and not offered (client)", resumed(&r) == Some(false) && n > 0);
    // another server name: another cache key, no ticket offered
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    let r = go(ClientConfig { sessions: Some(ss.clone()), ..ClientConfig::new(anchors(p), "127.0.0.1") }, scfg(p, &tk));
    t.eq("limits: a ticket of localhost is not offered to 127.0.0.1", resumed(&r), Some(false));
    // a connection of Trust::Any does not hand its tickets to one that checks
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ClientConfig { sessions: Some(ss.clone()), ..ClientConfig::new(Trust::Any, "localhost") }, scfg(p, &tk));
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("limits: tickets of Trust::Any are not offered with Trust::Anchors", resumed(&r), Some(false));
    let r = go(ClientConfig { sessions: Some(ss.clone()), ..ClientConfig::new(Trust::Any, "localhost") }, scfg(p, &tk));
    t.eq("limits: ... but with Trust::Any", resumed(&r), Some(true));
}

fn keys(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.rotate();
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("keys: after a rotation the previous key's tickets are taken", resumed(&r), Some(true));
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.rotate();
    tk.rotate();
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("keys: ... after two no longer", resumed(&r), Some(false));
    // keys rotate by themselves each lifetime
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new().lifetime(3600)));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.test_shift(3000_000);
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    tk.test_shift(700_000);
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("keys: a key of its own lives two lifetimes (a ticket of 50 min under the previous one)", resumed(&r), Some(true));
    // servers that share keys take each other's tickets
    let (a, b) = (Arc::new(Tickets::new()), Arc::new(Tickets::new()));
    a.set_keys(&[[7; 32]]);
    b.set_keys(&[[8; 32], [7; 32]]);
    let ss = Arc::new(Sessions::default());
    let _ = go(ccfg(p, &ss), scfg(p, &a));
    let r = go(ccfg(p, &ss), scfg(p, &b));
    t.eq("keys: set_keys: a ticket of one server resumes at another", resumed(&r), Some(true));
    let r = go(ccfg(p, &ss), scfg(p, &a));
    t.eq("keys: ... and one of the new key not at a server without it", resumed(&r), Some(false));
}

fn client_certs(t: &mut T, p: &Pki) {
    let tk = Arc::new(Tickets::new());
    let mtls = |trust: Trust| ServerConfig { client_auth: Some(trust), ..scfg(p, &tk) };
    let ss = Arc::new(Sessions::default());
    let with_id = || ClientConfig { identity: Some(p.client.clone()), ..ccfg(p, &ss) };
    let _ = go(with_id(), mtls(anchors(p)));
    let r = go(with_id(), mtls(anchors(p)));
    t.eq("client certs: mutual TLS resumes", resumed(&r), Some(true));
    t.check("client certs: ... the server keeps the client's certificate", r.as_ref().is_ok_and(|(_, s)| s.peer_certs().first().is_some_and(|c| c.subject_cn == "agent-1")));
    // another client trust (another list of CAs): the certificate was not checked by it
    let other = pki(95, Kind::P256);
    let r = go(with_id(), mtls(Trust::Anchors(alloc::vec![p.ca.clone(), other.ca.clone()])));
    t.eq("client certs: a server of another client trust does a full handshake", resumed(&r), Some(false));
    // the same cache without the client identity: no ticket of it
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("client certs: a connection without the identity does not offer its tickets", resumed(&r), Some(false));
    // a ticket without a client certificate (a server that asked for none) at a server that
    // requires one: the same client config, so the ticket is offered
    let ss2 = Arc::new(Sessions::default());
    let id2 = || ClientConfig { identity: Some(p.client.clone()), ..ccfg(p, &ss2) };
    let _ = go(id2(), scfg(p, &tk));
    let offered = ss2.len();
    let r = go(id2(), mtls(anchors(p)));
    t.check("client certs: a ticket without a certificate is not taken where one is required", offered > 0 && resumed(&r) == Some(false) && ss2.len() == offered + 1);
    // the certificates are checked again: a client the trust refuses since is not resumed
    let revoked = Arc::new(core::sync::atomic::AtomicBool::new(false));
    let rv = revoked.clone();
    let ca = p.ca.clone();
    let check: tls::Verifier = Arc::new(move |chain, _, now| {
        if rv.load(core::sync::atomic::Ordering::Relaxed) {
            return Err(String::from("revoked"));
        }
        netio::x509::verify_chain(chain, core::slice::from_ref(&ca), now, None, netio::x509::Usage::Client)
    });
    let custom = || ServerConfig { client_auth: Some(Trust::Custom(check.clone())), ..scfg(p, &tk) };
    let ss4 = Arc::new(Sessions::default());
    let id4 = || ClientConfig { identity: Some(p.client.clone()), ..ccfg(p, &ss4) };
    let _ = go(id4(), custom());
    let r = go(id4(), custom());
    t.eq("client certs: resumed while the client's check passes", resumed(&r), Some(true));
    revoked.store(true, core::sync::atomic::Ordering::Relaxed);
    let r = go(id4(), custom());
    t.check("client certs: ... not once it refuses the client (a full handshake, refused)", matches!(r, Err(TlsError::Alert(42, _)) | Err(TlsError::PeerAlert(42))));
    // a client chain too long for a ticket (over 16 KiB) gets none
    let names: Vec<String> = (0..900).map(|i| format!("agent-{:04}.web.lab", i)).collect();
    let big = netio::x509::CertBuilder {
        serial: [99; 16],
        cn: String::from("agent-big"),
        not_before: 1_791_000_000 - 3600,
        not_after: 1_791_000_000 + 365 * 86_400,
        client_auth: true,
        dns: names,
        spki: p.client.key.spki(),
        ..netio::x509::CertBuilder::default()
    }
    .sign_key(Some(&p.ca), &p.ca_key, &[])
    .unwrap_or_default();
    let ss5 = Arc::new(Sessions::default());
    let r = go(ClientConfig { identity: Some(tls::Identity::new(alloc::vec![big.clone()], p.client.key.clone())), ..ccfg(p, &ss5) }, mtls(anchors(p)));
    t.check("client certs: a chain of over 16 KiB: no ticket (a ticket holds all of it or none)", big.len() > 16 * 1024 && r.is_ok() && ss5.is_empty());
    let ss3 = Arc::new(Sessions::default());
    let _ = go(ccfg(p, &ss3), ServerConfig { client_auth_optional: true, ..mtls(anchors(p)) });
    let r = go(ccfg(p, &ss3), ServerConfig { client_auth_optional: true, ..mtls(anchors(p)) });
    t.check("client certs: ... but where it is optional", resumed(&r) == Some(true) && r.as_ref().is_ok_and(|(_, s)| s.peer_certs().is_empty()));
}

fn cache(t: &mut T, p: &Pki) {
    let tk = Arc::new(Tickets::new());
    let ss = Arc::new(Sessions::new(1));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    let _ = go(ClientConfig { sessions: Some(ss.clone()), ..ClientConfig::new(anchors(p), "127.0.0.1") }, scfg(p, &tk));
    let r = go(ccfg(p, &ss), scfg(p, &tk));
    t.eq("cache: Sessions::new(1) keeps one server", resumed(&r), Some(false));
    let ss = Arc::new(Sessions::default());
    for _ in 0..12 {
        let _ = go(ccfg(p, &ss), scfg(p, &tk));
    }
    t.eq("cache: at most 8 tickets per server", ss.len(), 8);
    ss.clear();
    t.check("cache: clear", ss.is_empty());
}

/// a client's ClientHello with 0-RTT data, fed to a server: (client, server) with the server's
/// flight sent
fn early_start(p: &Pki, ss: &Arc<Sessions>, s: ServerConfig, data: &[u8]) -> Result<(Conn, Conn, Vec<u8>), TlsError> {
    let mut c = Conn::client_early(ccfg(p, ss), data);
    let first = c.take_out();
    let mut s = Conn::server(s);
    s.feed(&first)?;
    Ok((c, s, first))
}

fn early(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let ecfg = |n: u32| ServerConfig { early_data: n, ..scfg(p, &tk) };
    let _ = go(ccfg(p, &ss), ecfg(4096));
    let Ok((mut c, mut s, _)) = early_start(p, &ss, ecfg(4096), b"GET /early") else {
        t.check("early: the first flight", false);
        return;
    };
    t.check("early: the server takes 0-RTT and is ready before the client's Finished", s.in_early() && s.ready() && !s.established() && s.resumed());
    let mut b = [0u8; 64];
    let n = s.read_early(&mut b);
    t.eq("early: read_early gives the data", &b[..n], &b"GET /early"[..]);
    t.check("early: ... read_plain nothing yet", s.read_plain(&mut b) == 0);
    // 0.5-RTT: an answer before the client's Finished
    t.check("early: the server writes before the client's Finished", s.write_plain(b"early answer").is_ok());
    let r = c.feed(&s.take_out());
    t.check("early: the client completes", r.is_ok() && c.established() && c.resumed());
    t.eq("early: ... the server took all 10 bytes", c.early_taken(), 10);
    let n = c.read_plain(&mut b);
    t.eq("early: ... the answer came", &b[..n], &b"early answer"[..]);
    let r = s.feed(&c.take_out());
    t.check("early: EndOfEarlyData, the client's Finished: established", r.is_ok() && s.established() && !s.in_early());
    // data not read during the handshake comes first from read_plain
    let _ = go(ccfg(p, &ss), ecfg(4096));
    if let Some((mut c, mut s, _)) = started(t, "early: unread data", early_start(p, &ss, ecfg(4096), b"first ")) {
        let _ = c.feed(&s.take_out());
        let _ = c.write_plain(b"then");
        let _ = s.feed(&c.take_out());
        let n = s.read_plain(&mut b);
        t.eq("early: unread early data comes before the rest", &b[..n], &b"first then"[..]);
    }
    // more data than the ticket allows: the client sends what it takes
    let (ss2, tk2) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss2), ServerConfig { early_data: 16, ..scfg(p, &tk2) });
    if let Some((mut c, mut s, _)) = started(t, "early: the ticket's limit", early_start(p, &ss2, ServerConfig { early_data: 16, ..scfg(p, &tk2) }, &[b'x'; 100])) {
        let _ = c.feed(&s.take_out());
        t.eq("early: a client sends at most the ticket's max_early_data_size", c.early_taken(), 16);
    }
    // a server that now takes less than its tickets allow: resumed without 0-RTT (the client
    // may send what the ticket says)
    let (ss3, tk3) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss3), ServerConfig { early_data: 1000, ..scfg(p, &tk3) });
    if let Some((mut c, mut s, _)) = started(t, "early: a lower limit", early_start(p, &ss3, ServerConfig { early_data: 8, ..scfg(p, &tk3) }, &[b'x'; 100])) {
        let r = c.feed(&s.take_out()).and_then(|_| s.feed(&c.take_out()));
        t.check("early: a ticket allowing more than the server takes now: resumed, no 0-RTT", r.is_ok() && s.established() && s.resumed() && c.early_taken() == 0 && !s.has_early());
    }
    // refused 0-RTT in records of one byte each (8 KiB of them the ticket allows): all skipped,
    // the budget counts the data, not the records' type and tag
    let (ss5, tk5) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss5), ServerConfig { early_data: 8192, ..scfg(p, &tk5) });
    if let Some((mut c, mut s, _)) = started(t, "early: small records", early_start(p, &ss5, ServerConfig { early_data: 4096, ..scfg(p, &tk5) }, b"GET /")) {
        let mut small = Vec::new();
        for i in 0..4000u32 {
            small.extend_from_slice(&[23, 3, 3, 0, 18]);
            small.extend((0..18u32).map(|j| (i.wrapping_mul(31) ^ j) as u8));
        }
        let r = s.feed(&small).and_then(|_| c.feed(&s.take_out())).and_then(|_| s.feed(&c.take_out()));
        t.check("early: refused 0-RTT in 4000 records of one byte each is skipped", r.is_ok() && s.established() && s.resumed() && c.early_taken() == 0);
    }
    // a client that sends more than its ticket allows: unexpected_message
    let (ss4, tk4) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss4), ServerConfig { early_data: 8, ..scfg(p, &tk4) });
    ss4.test_max_early(1000);
    let r = early_start(p, &ss4, ServerConfig { early_data: 8, ..scfg(p, &tk4) }, &[b'x'; 100]);
    t.check("early: more than the ticket allows is unexpected_message", matches!(r, Err(TlsError::Alert(10, _))));
}

/// the first flight of early_start, or a failed check
fn started(t: &mut T, what: &str, r: Result<(Conn, Conn, Vec<u8>), TlsError>) -> Option<(Conn, Conn, Vec<u8>)> {
    match r {
        Ok(x) => Some(x),
        Err(e) => {
            t.check(&format!("{}: the first flight: {}", what, e), false);
            None
        }
    }
}

fn early_refused(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let ecfg = |n: u32| ServerConfig { early_data: n, ..scfg(p, &tk) };
    let mut b = [0u8; 64];
    // the same ClientHello again (a replay): resumption, no 0-RTT
    let _ = go(ccfg(p, &ss), ecfg(4096));
    if let Some((_, s1, first)) = started(t, "early refused: replay", early_start(p, &ss, ecfg(4096), b"POST /pay")) {
        let mut s2 = Conn::server(ecfg(4096));
        let r = s2.feed(&first);
        t.check("early refused: a replayed ClientHello: no 0-RTT", r.is_ok() && s1.in_early() && !s2.in_early() && !s2.has_early() && s2.read_early(&mut b) == 0);
        t.check("early refused: ... its handshake goes on (the replayer cannot finish it)", !s2.take_out().is_empty());
    }
    // stale: the server's age of the ticket 11 s off the client's
    let _ = go(ccfg(p, &ss), ecfg(4096));
    tk.test_shift(11_000);
    if let Some((mut c, mut s, _)) = started(t, "early refused: stale", early_start(p, &ss, ecfg(4096), b"GET /")) {
        let _ = c.feed(&s.take_out());
        let r = s.feed(&c.take_out());
        t.check("early refused: a ticket age 11 s off: resumed without 0-RTT", r.is_ok() && s.established() && s.resumed() && !s.has_early() && c.early_taken() == 0);
        t.check("early refused: ... the data reaches nobody", s.read_plain(&mut b) == 0);
    }
    // another ALPN protocol than the ticket's session
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let alpn = |l: &[&str]| l.iter().map(|x| String::from(*x)).collect::<Vec<_>>();
    let srv = |l: &[&str]| ServerConfig { early_data: 4096, alpn: alpn(l), ..scfg(p, &tk) };
    let cli = || ClientConfig { alpn: alpn(&["h2", "http/1.1"]), ..ccfg(p, &ss) };
    let _ = go(cli(), srv(&["h2"]));
    let mut c = Conn::client_early(cli(), b"GET /");
    let mut s = Conn::server(srv(&["http/1.1", "h2"]));
    let r = s.feed(&c.take_out()).and_then(|_| c.feed(&s.take_out())).and_then(|_| s.feed(&c.take_out()));
    t.check("early refused: another ALPN protocol: resumed, no 0-RTT", r.is_ok() && s.established() && s.resumed() && c.early_taken() == 0 && c.alpn() == Some("http/1.1"));
    // a server that takes no 0-RTT now
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), ServerConfig { early_data: 4096, ..scfg(p, &tk) });
    if let Some((mut c, mut s, _)) = started(t, "early refused: off", early_start(p, &ss, scfg(p, &tk), b"GET /")) {
        let r = c.feed(&s.take_out()).and_then(|_| s.feed(&c.take_out()));
        t.check("early refused: early_data 0 at the server: resumed, no 0-RTT", r.is_ok() && s.resumed() && c.early_taken() == 0 && !s.has_early());
    }
    // tickets of a server without 0-RTT: the client sends none
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let _ = go(ccfg(p, &ss), scfg(p, &tk));
    let mut c = Conn::client_early(ccfg(p, &ss), b"GET /");
    let first = c.take_out();
    let mut s = Conn::server(ServerConfig { early_data: 4096, ..scfg(p, &tk) });
    let r = s.feed(&first).and_then(|_| c.feed(&s.take_out())).and_then(|_| s.feed(&c.take_out()));
    t.check("early refused: a ticket without early_data: no 0-RTT sent", r.is_ok() && s.resumed() && c.early_taken() == 0 && !s.has_early());
}

/// connect_early and read_early over TCP: a request in 0-RTT, the answer before the client's
/// Finished, then the rest
fn over_tcp(t: &mut T, p: &Pki) {
    let (ss, tk) = (Arc::new(Sessions::default()), Arc::new(Tickets::new()));
    let scfg = ServerConfig { early_data: 4096, ..scfg(p, &tk) };
    let l = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)).ok().expect("bind");
    let addr = l.local_addr().unwrap_or(SocketAddr::new(IpAddr::LOCALHOST_V4, 0));
    let seen = Arc::new(plat::sync::Mutex::new(Vec::<(bool, String, String)>::new()));
    let seen2 = seen.clone();
    let th = plat::thread_spawn(0, Box::new(move || {
        for _ in 0..2 {
            let Ok(Some((tcp, _))) = l.accept_timeout(5000) else { return };
            tcp.set_read_timeout(Some(5000));
            let Ok(s) = tls::accept(Box::new(tcp), scfg.clone()) else { return };
            let mut b = [0u8; 256];
            let mut early = Vec::new();
            while let Ok(n) = s.read_early(&mut b) {
                if n == 0 {
                    break;
                }
                early.extend_from_slice(&b[..n]);
            }
            if !early.is_empty() {
                let _ = s.write_all(b"answer to early\n");
            }
            let n = s.read(&mut b).unwrap_or(0);
            seen2.lock().push((s.resumed(), String::from_utf8_lossy(&early).into(), String::from_utf8_lossy(&b[..n]).into()));
            let _ = s.write_all(b"bye\n");
            s.shutdown();
        }
    }))
    .expect("thread");
    let talk = |early: &[u8]| -> Option<(usize, Vec<u8>)> {
        let tcp = TcpStream::connect(&addr, 2000).ok()?;
        tcp.set_read_timeout(Some(5000));
        let (s, n) = tls::connect_early(Box::new(tcp), ccfg(p, &ss), early).ok()?;
        let _ = s.write_all(b"after");
        let mut got = Vec::new();
        let mut b = [0u8; 256];
        while let Ok(k) = s.read(&mut b) {
            if k == 0 {
                break;
            }
            got.extend_from_slice(&b[..k]);
        }
        Some((n, got))
    };
    let first = talk(b"GET /");
    let second = talk(b"GET /");
    th.join();
    let seen = seen.lock().clone();
    t.check("tcp: the first connection sends no 0-RTT (no ticket yet)", first.as_ref().is_some_and(|(n, got)| *n == 0 && got == b"bye\n"));
    t.check("tcp: the second sends its request in 0-RTT, the server takes it", second.as_ref().is_some_and(|(n, got)| *n == 5 && got == b"answer to early\nbye\n"));
    t.check("tcp: the server: read_early, then read", seen.len() == 2 && seen[0] == (false, String::new(), String::from("after")) && seen[1] == (true, String::from("GET /"), String::from("after")));
}
