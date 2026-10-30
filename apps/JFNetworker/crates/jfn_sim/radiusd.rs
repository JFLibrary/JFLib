// radiusd -- RADIUS on a device: the server "radius-server" on hosts (RFC 2865 authentication
// on UDP 1812, RFC 2866 accounting on 1813) and the NAS side of an IOS-like device (the hidden
// service "radius-nas", which sends Access-Requests, retransmits, tries the next server and
// hands the outcome back to the login gate in cli/aaa.rs)
//
//   RadiusServerSvc           the server: clients (an address or a prefix and a shared secret),
//                             users (password, a privilege level or Service-Type in the reply,
//                             an optional reply message), the accounting file; from_config
//   NasService                the NAS: it drives one RADIUS method list for a login (the
//                             servers of each group in turn, retransmission, the next method on
//                             a timeout); created by Engine::nas_begin
//   Engine::nas_begin         start a RADIUS login for a session (cli/aaa.rs hands it a user, a
//                             password and the remaining methods)
//
// A server drops a request from an unknown client or with a wrong Message-Authenticator
// silently and logs a line (RFC 2865 says so).  The NAS hides User-Password with the server's
// shared secret and the Request Authenticator (RFC 2865 5.2), checks the Response
// Authenticator and the Message-Authenticator of every answer, and ignores an answer that does
// not fit.  The crypto is lent in (crypto::md5_parts, crypto::hmac_md5); the Request
// Authenticator comes from the device's random stream, so a run is the same byte for byte.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::radius::{self, Attr, Packet, RadiusView};
use netpkt::{Ipv4, Prefix4};

use crate::aaa::{AuthResult, Method, Requester};
use crate::aaatrace::ANote;
use crate::http::Wheel;
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::trace::Note;

/// our vendor number for the Vendor-Specific attribute that carries a privilege level
pub const JFN_VENDOR: u32 = 55355;
const SEV_INFO: u8 = 6;
const SEV_WARN: u8 = 4;

/// MD5 and HMAC-MD5 lent to netpkt::radius
fn md5(parts: &[&[u8]]) -> [u8; 16] {
    crypto::md5_parts(parts)
}
fn hmac(key: &[u8], data: &[u8]) -> [u8; 16] {
    crypto::hmac_md5(key, data)
}

// ================================================================ the server

/// a client the server answers (an address or a prefix) with its shared secret
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadiusClient {
    pub net: Prefix4,
    pub secret: String,
}

/// a user the server knows
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadiusUser {
    pub name: String,
    pub password: String,
    /// the privilege level the device should give (0 to 15)
    pub privilege: u8,
    /// a Reply-Message attribute in the answer
    pub reply: Option<String>,
}

/// the "radius-server" service
#[derive(Clone, Debug)]
pub struct RadiusServerSvc {
    pub auth_port: u16,
    pub acct_port: u16,
    pub clients: Vec<RadiusClient>,
    pub users: Vec<RadiusUser>,
    /// where accounting records are written (empty: no accounting file)
    pub acct_log: String,
    // metrics
    requests: u64,
    accepts: u64,
    rejects: u64,
    acct: u64,
    dropped: u64,
    bad_auth: u64,
}

impl Default for RadiusServerSvc {
    fn default() -> RadiusServerSvc {
        RadiusServerSvc { auth_port: radius::PORT_AUTH, acct_port: radius::PORT_ACCT, clients: Vec::new(), users: Vec::new(), acct_log: String::from("/var/log/radius/accounting"), requests: 0, accepts: 0, rejects: 0, acct: 0, dropped: 0, bad_auth: 0 }
    }
}

impl RadiusServerSvc {
    pub fn new() -> RadiusServerSvc {
        RadiusServerSvc::default()
    }

    pub fn from_config(c: &Value) -> Result<RadiusServerSvc, Msg> {
        let mut s = RadiusServerSvc::new();
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        if let Some(p) = num("auth_port") {
            s.auth_port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        if let Some(p) = num("acct_port") {
            s.acct_port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        if let Some(l) = c.get("acct_log").and_then(|v| v.as_str()) {
            s.acct_log = String::from(l.trim());
        }
        for cl in c.get("clients").and_then(|v| v.as_array()).unwrap_or(&[]) {
            let a = cl.get("address").and_then(|v| v.as_str()).unwrap_or("").trim();
            let net = Prefix4::parse(a).or_else(|| Ipv4::parse(a).map(|ip| Prefix4 { addr: ip, len: 32 })).ok_or_else(|| msg!("\u{201e}{}\u{201c} ist keine Adresse oder kein Pr\u{00e4}fix.", a))?;
            let secret = String::from(cl.get("secret").and_then(|v| v.as_str()).unwrap_or(""));
            if secret.is_empty() {
                return Err(msg!("Ein Client braucht ein \u{201e}secret\u{201c}."));
            }
            s.clients.push(RadiusClient { net: net.trunc(), secret });
        }
        for u in c.get("users").and_then(|v| v.as_array()).unwrap_or(&[]) {
            let name = u.get("name").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(msg!("Ein Benutzer braucht \u{201e}name\u{201c} ohne Leerzeichen."));
            }
            let privilege = u.get("privilege").and_then(|v| v.as_u64()).map(|p| p.min(15) as u8).unwrap_or(1);
            s.users.push(RadiusUser {
                name: String::from(name),
                password: String::from(u.get("password").and_then(|v| v.as_str()).unwrap_or("")),
                privilege,
                reply: u.get("reply").and_then(|v| v.as_str()).map(String::from),
            });
        }
        Ok(s)
    }

    /// the secret of the client that owns `from` (the most specific prefix that contains it)
    fn client_secret(&self, from: Ipv4) -> Option<&str> {
        self.clients.iter().filter(|c| c.net.contains(from)).max_by_key(|c| c.net.len).map(|c| c.secret.as_str())
    }
}

impl Service for RadiusServerSvc {
    fn kind(&self) -> &'static str {
        "radius-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        for port in [self.auth_port, self.acct_port] {
            if os.bind(port, None).is_err() {
                os.note(Note::PortInUse { port, svc: String::from("radius-server") });
            }
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        let SvcEvent::Udp(d) = ev else {
            return;
        };
        let from = d.src4();
        let Ok(v) = RadiusView::parse(&d.data) else {
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("fehlerhaftes Paket") }));
            return;
        };
        // an unknown client is dropped silently (RFC 2865 3)
        let Some(secret) = self.client_secret(from).map(String::from) else {
            self.dropped += 1;
            os.log(SEV_WARN, "radiusd", &format!("request from unknown client {} dropped", from));
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("unbekannter Client") }));
            return;
        };
        // a wrong Message-Authenticator is dropped (RFC 3579 3.2)
        if radius::check_message_auth(&d.data, None, secret.as_bytes(), hmac) == Some(false) {
            self.bad_auth += 1;
            self.dropped += 1;
            os.log(SEV_WARN, "radiusd", &format!("request from {} with a wrong Message-Authenticator dropped", from));
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Message-Authenticator") }));
            return;
        }
        let p = Packet::read(&v);
        match p.code {
            radius::ACCESS_REQUEST if d.dport == self.auth_port => self.access(os, from, d.sport, &secret, &p),
            radius::ACCOUNTING_REQUEST if d.dport == self.acct_port => self.accounting(os, from, d.sport, &secret, &p),
            _ => {
                self.dropped += 1;
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("requests", self.requests as i64),
            Metric::new("accepts", self.accepts as i64),
            Metric::new("rejects", self.rejects as i64),
            Metric::new("accounting", self.acct as i64),
            Metric::new("dropped", self.dropped as i64),
            Metric::new("bad_auth", self.bad_auth as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Ports {} / {}, {} Clients, {} Benutzer: {} angenommen, {} abgelehnt", self.auth_port, self.acct_port, self.clients.len(), self.users.len(), self.accepts, self.rejects)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn footprint(&self) -> usize {
        self.clients.iter().map(|c| c.secret.len() + 24).sum::<usize>() + self.users.iter().map(|u| u.name.len() + u.password.len() + u.reply.as_ref().map_or(0, |r| r.len()) + 24).sum::<usize>() + 512
    }
}

impl RadiusServerSvc {
    /// an Access-Request: reveal the password, look the user up, answer Accept or Reject
    fn access(&mut self, os: &mut OsApi, from: Ipv4, sport: u16, secret: &str, p: &Packet) {
        self.requests += 1;
        let user = String::from(p.text(radius::A_USER_NAME).unwrap_or_default());
        let hidden = p.attr(radius::A_USER_PASSWORD).map(|a| a.value.clone()).unwrap_or_default();
        let pw = radius::reveal_password(&hidden, secret.as_bytes(), &p.auth, md5).and_then(|b| String::from_utf8(b).ok());
        let acct = self.users.iter().find(|u| u.name == user).cloned();
        let ok = matches!((&acct, &pw), (Some(u), Some(pw)) if &u.password == pw);
        let had_ma = p.attr(radius::A_MESSAGE_AUTHENTICATOR).is_some();
        if ok {
            self.accepts += 1;
            let u = acct.unwrap();
            let mut reply = Packet::new(radius::ACCESS_ACCEPT, p.id, [0; 16]);
            // the privilege level as a Service-Type and a vendor-specific attribute of our own
            let st = if u.privilege >= 15 { radius::SERVICE_ADMINISTRATIVE } else { radius::SERVICE_NAS_PROMPT };
            reply.push(Attr::int(radius::A_SERVICE_TYPE, st));
            reply.push(Attr::vsa(JFN_VENDOR, format!("priv-lvl={}", u.privilege).as_bytes()));
            if let Some(m) = &u.reply {
                reply.push(Attr::text(radius::A_REPLY_MESSAGE, m));
            }
            self.send_reply(os, from, sport, secret, &p.auth, reply, had_ma);
            os.note(Note::Aaa(ANote::RadiusAccept { from, user: user.clone(), level: Some(u.privilege) }));
            os.log(SEV_INFO, "radiusd", &format!("{} accepted ({})", user, from));
        } else {
            self.rejects += 1;
            let mut reply = Packet::new(radius::ACCESS_REJECT, p.id, [0; 16]);
            reply.push(Attr::text(radius::A_REPLY_MESSAGE, "Zugang verweigert."));
            self.send_reply(os, from, sport, secret, &p.auth, reply, had_ma);
            os.note(Note::Aaa(ANote::RadiusReject { from, user: user.clone(), }));
            os.log(SEV_WARN, "radiusd", &format!("{} rejected ({})", user, from));
        }
    }

    /// writes the Response Authenticator (and a Message-Authenticator when the request had one)
    /// and sends the answer
    fn send_reply(&mut self, os: &mut OsApi, to: Ipv4, dport: u16, secret: &str, req_auth: &[u8; 16], reply: Packet, message_auth: bool) {
        let mut reply = reply;
        if message_auth {
            reply.push(Attr::message_auth());
        }
        let mut b = reply.to_bytes();
        if message_auth {
            radius::set_message_auth(&mut b, Some(req_auth), secret.as_bytes(), hmac);
        }
        radius::set_response_auth(&mut b, req_auth, secret.as_bytes(), md5);
        os.send_from(self.local_for(os, to), self.listen_port(os), to, dport, &b);
    }

    /// the address of the interface that reaches `to` (so the answer comes from the right one)
    fn local_for(&self, os: &OsApi, to: Ipv4) -> Ipv4 {
        os.source_for(to).unwrap_or(Ipv4::UNSPECIFIED)
    }
    fn listen_port(&self, _os: &OsApi) -> u16 {
        self.auth_port
    }

    /// an Accounting-Request: check its Request Authenticator, write a record, answer
    fn accounting(&mut self, os: &mut OsApi, from: Ipv4, sport: u16, secret: &str, p: &Packet) {
        if !radius::check_accounting_auth(&p_bytes(p), secret.as_bytes(), md5) {
            self.bad_auth += 1;
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Request-Authenticator") }));
            return;
        }
        self.acct += 1;
        let user = p.text(radius::A_USER_NAME).unwrap_or_default();
        let status = p.int(radius::A_ACCT_STATUS_TYPE).unwrap_or(0);
        let sid = p.text(radius::A_ACCT_SESSION_ID).unwrap_or_default();
        let what = radius::value_name(radius::A_ACCT_STATUS_TYPE, status).unwrap_or("?");
        if !self.acct_log.is_empty() {
            let dir = os.files().map(|s| s.parent(&self.acct_log)).unwrap_or_default();
            if !dir.is_empty() {
                let _ = os.file_mkdir(&dir, true);
            }
            let line = format!("{} session={} user={} from={}\n", what, sid, user, from);
            let _ = os.file_append(&self.acct_log, line.as_bytes());
        }
        // the Response Authenticator of an Accounting-Response is over the request's
        let reply = Packet::new(radius::ACCOUNTING_RESPONSE, p.id, [0; 16]);
        let mut b = reply.to_bytes();
        radius::set_response_auth(&mut b, &p.auth, secret.as_bytes(), md5);
        os.send_from(self.local_for(os, from), self.acct_port, from, sport, &b);
        os.log(SEV_INFO, "radiusd", &format!("accounting {} for {} ({})", what, user, from));
    }
}

/// a packet's bytes with its authenticator in place (for check_accounting_auth)
fn p_bytes(p: &Packet) -> Vec<u8> {
    p.to_bytes()
}


// ================================================================ the NAS side

const K_WHEEL: u32 = 0;

/// one login the NAS is driving: the remaining methods, and the current RADIUS exchange
#[derive(Clone, Debug)]
struct Req {
    via: Requester,
    user: String,
    pass: String,
    /// the methods still to try (methods[mi] is the one being tried)
    methods: Vec<Method>,
    mi: usize,
    want_author: bool,
    /// the servers of the current RADIUS group and which one is being asked
    servers: Vec<crate::aaa::RadiusServer>,
    si: usize,
    /// sends to the current server so far (1 = the first)
    attempt: u32,
    id: u8,
    auth: [u8; 16],
    /// its deadline in the wheel (to cancel it)
    wid: u64,
}

/// the NAS: the hidden service "radius-nas", one UDP socket, the logins it is driving
#[derive(Clone, Debug)]
pub struct NasService {
    sock: Option<u16>,
    reqs: BTreeMap<u64, Req>,
    next: u64,
    wheel: Wheel<u64>,
    // metrics
    requests: u64,
    accepts: u64,
    rejects: u64,
    timeouts: u64,
    dropped: u64,
}

impl Default for NasService {
    fn default() -> NasService {
        NasService::new()
    }
}

impl NasService {
    fn new() -> NasService {
        NasService { sock: None, reqs: BTreeMap::new(), next: 0, wheel: Wheel::new(K_WHEEL), requests: 0, accepts: 0, rejects: 0, timeouts: 0, dropped: 0 }
    }
}

impl Service for NasService {
    fn kind(&self) -> &'static str {
        "radius-nas"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.ensure_sock(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => self.answer(os, d),
            SvcEvent::Timer(K_WHEEL) => {
                self.wheel.fired();
                for (_, rid) in self.wheel.due(os.now()) {
                    self.timeout(os, rid);
                }
                self.wheel.arm(os);
            }
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("requests", self.requests as i64),
            Metric::new("accepts", self.accepts as i64),
            Metric::new("rejects", self.rejects as i64),
            Metric::new("timeouts", self.timeouts as i64),
            Metric::new("dropped", self.dropped as i64),
            Metric::new("pending", self.reqs.len() as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("{} Anfragen, {} angenommen, {} abgelehnt, {} Zeit\u{00fc}berschreitungen", self.requests, self.accepts, self.rejects, self.timeouts)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn footprint(&self) -> usize {
        self.reqs.values().map(|r| r.user.len() + r.pass.len() + r.methods.len() * 16 + r.servers.len() * 48 + 64).sum::<usize>() + 256
    }
}

impl NasService {
    /// binds the UDP socket if it is not bound yet (begin may run before start())
    fn ensure_sock(&mut self, os: &mut OsApi) {
        if self.sock.is_none() {
            self.sock = os.bind(0, None).ok();
        }
    }

    /// a new login: drive its method list (methods[0] is a RADIUS group)
    fn begin(&mut self, os: &mut OsApi, req: Req) {
        self.ensure_sock(os);
        self.next += 1;
        let rid = self.next;
        self.reqs.insert(rid, req);
        self.start_group(os, rid);
    }

    /// starts asking the servers of the group at methods[mi]
    fn start_group(&mut self, os: &mut OsApi, rid: u64) {
        let (servers, mi) = {
            let Some(r) = self.reqs.get(&rid) else {
                return;
            };
            (r.servers.clone(), r.mi)
        };
        let _ = mi;
        if servers.is_empty() {
            // an empty group: no server answered, go on to the next method
            self.advance(os, rid);
            return;
        }
        if let Some(r) = self.reqs.get_mut(&rid) {
            r.si = 0;
            r.attempt = 1;
        }
        self.send(os, rid);
    }

    /// sends the Access-Request of req `rid` to its current server
    fn send(&mut self, os: &mut OsApi, rid: u64) {
        let Some(port) = self.sock else {
            self.finish(os, rid, AuthResult::NoServer, "RADIUS");
            return;
        };
        let (server, user, pass, attempt, want_author) = {
            let Some(r) = self.reqs.get(&rid) else {
                return;
            };
            let Some(server) = r.servers.get(r.si).cloned() else {
                self.advance(os, rid);
                return;
            };
            (server, r.user.clone(), r.pass.clone(), r.attempt, r.want_author)
        };
        let _ = want_author;
        // a fresh Request Authenticator from the device's random stream
        let mut auth = [0u8; 16];
        fill16(os, &mut auth);
        let id = (os.rand() & 0xFF) as u8;
        let src = os.source_for(server.addr).unwrap_or(Ipv4::UNSPECIFIED);
        let mut p = Packet::new(radius::ACCESS_REQUEST, id, auth);
        p.push(Attr::text(radius::A_USER_NAME, &user));
        let hidden = radius::hide_password(pass.as_bytes(), server.key.as_bytes(), &auth, md5);
        p.push(Attr::bytes(radius::A_USER_PASSWORD, &hidden));
        if !src.is_unspecified() {
            p.push(Attr::addr(radius::A_NAS_IP_ADDRESS, src));
        }
        p.push(Attr::int(radius::A_NAS_PORT, 0));
        p.push(Attr::int(radius::A_SERVICE_TYPE, radius::SERVICE_NAS_PROMPT));
        p.push(Attr::int(radius::A_NAS_PORT_TYPE, radius::PORT_TYPE_VIRTUAL));
        p.push(Attr::message_auth());
        let mut b = p.to_bytes();
        radius::set_message_auth(&mut b, None, server.key.as_bytes(), hmac);
        if let Some(r) = self.reqs.get_mut(&rid) {
            r.id = id;
            r.auth = auth;
        }
        self.requests += 1;
        let sent = os.send_from(src, port, server.addr, server.auth_port, &b);
        if sent {
            os.note(Note::Aaa(ANote::RadiusRequest { to: server.addr, user, attempt }));
        } else {
            // no route to the server: no answer will come, go to the next at once
            os.note(Note::Aaa(ANote::RadiusTimeout { to: server.addr }));
            self.timeouts += 1;
            self.next_server(os, rid);
            return;
        }
        // the answer deadline
        let at = os.now() + server.timeout.max(1) as Ns * SEC;
        let wid = self.wheel.add(at, rid);
        if let Some(r) = self.reqs.get_mut(&rid) {
            r.wid = wid;
        }
        self.wheel.arm(os);
    }

    /// an answer to the socket
    fn answer(&mut self, os: &mut OsApi, d: crate::os::Dgram) {
        let from = d.src4();
        let Ok(v) = RadiusView::parse(&d.data) else {
            return;
        };
        // the request this answers: its id, its current server's address and key
        let rid = self.reqs.iter().find(|(_, r)| r.id == v.id() && r.servers.get(r.si).is_some_and(|s| s.addr == from)).map(|(id, _)| *id);
        let Some(rid) = rid else {
            return;
        };
        let (key, user) = {
            let r = &self.reqs[&rid];
            (r.servers[r.si].key.clone(), r.user.clone())
        };
        let req_auth = self.reqs[&rid].auth;
        // the Response Authenticator and the Message-Authenticator must fit the secret
        if !radius::check_response_auth(&d.data, &req_auth, key.as_bytes(), md5) {
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Response-Authenticator") }));
            return;
        }
        if radius::check_message_auth(&d.data, Some(&req_auth), key.as_bytes(), hmac) == Some(false) {
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Message-Authenticator") }));
            return;
        }
        let p = Packet::read(&v);
        self.cancel(rid);
        match p.code {
            radius::ACCESS_ACCEPT => {
                self.accepts += 1;
                let want = self.reqs[&rid].want_author;
                let level = if want { author_level(&p) } else { 1 };
                os.note(Note::Aaa(ANote::RadiusAccept { from, user, level: if want { Some(level) } else { None } }));
                self.finish(os, rid, AuthResult::Pass(level), "RADIUS");
            }
            radius::ACCESS_REJECT => {
                self.rejects += 1;
                os.note(Note::Aaa(ANote::RadiusReject { from, user }));
                self.finish(os, rid, AuthResult::Fail, "RADIUS");
            }
            radius::ACCESS_CHALLENGE => {
                os.note(Note::Aaa(ANote::RadiusChallenge { from, user }));
                // we do not carry a challenge through PAP login: treated as a refusal
                self.finish(os, rid, AuthResult::Fail, "RADIUS");
            }
            _ => self.finish(os, rid, AuthResult::Fail, "RADIUS"),
        }
    }

    /// no answer came in time: retransmit, try the next server, or move on
    fn timeout(&mut self, os: &mut OsApi, rid: u64) {
        let (to, attempt, retransmit) = {
            let Some(r) = self.reqs.get(&rid) else {
                return;
            };
            let Some(s) = r.servers.get(r.si) else {
                return;
            };
            (s.addr, r.attempt, s.retransmit)
        };
        if attempt <= retransmit {
            if let Some(r) = self.reqs.get_mut(&rid) {
                r.attempt += 1;
            }
            os.note(Note::Aaa(ANote::RadiusRetransmit { to, attempt: attempt + 1 }));
            self.send(os, rid);
        } else {
            self.timeouts += 1;
            os.note(Note::Aaa(ANote::RadiusTimeout { to }));
            self.next_server(os, rid);
        }
    }

    /// the current server gave up; try the next one of the group
    fn next_server(&mut self, os: &mut OsApi, rid: u64) {
        let more = {
            let Some(r) = self.reqs.get_mut(&rid) else {
                return;
            };
            r.si += 1;
            r.attempt = 1;
            r.si < r.servers.len()
        };
        if more {
            self.send(os, rid);
        } else {
            self.advance(os, rid);
        }
    }

    /// the current RADIUS group is exhausted: go on to the next method of the list
    fn advance(&mut self, os: &mut OsApi, rid: u64) {
        let (via, user, pass) = {
            let Some(r) = self.reqs.get_mut(&rid) else {
                return;
            };
            r.mi += 1;
            (r.via, r.user.clone(), r.pass.clone())
        };
        loop {
            let m = self.reqs.get(&rid).and_then(|r| r.methods.get(r.mi).cloned());
            let Some(m) = m else {
                // nothing left: no server answered and no local fallback
                self.finish(os, rid, AuthResult::NoServer, "RADIUS");
                return;
            };
            match m {
                Method::GroupRadius | Method::Group(_) => {
                    let servers = os.eng.aaa_servers_of(os.ni, &m);
                    if let Some(r) = self.reqs.get_mut(&rid) {
                        r.servers = servers;
                    }
                    self.start_group(os, rid);
                    return;
                }
                other => {
                    // a local method decides here (local, line, enable, none)
                    let r = os.eng.aaa_sync_method(os.ni, via, &other, &user, &pass);
                    self.finish(os, rid, r, how_of(&other));
                    return;
                }
            }
        }
    }

    fn cancel(&mut self, rid: u64) {
        if let Some(r) = self.reqs.get(&rid) {
            self.wheel.cancel(r.wid);
        }
    }

    /// the login is decided: tell the gate and forget the request (`how` is for the note)
    fn finish(&mut self, os: &mut OsApi, rid: u64, outcome: AuthResult, how: &'static str) {
        self.cancel(rid);
        let Some(r) = self.reqs.remove(&rid) else {
            return;
        };
        self.wheel.arm(os);
        os.aaa_result(r.via, outcome, how);
    }
}

/// how a method authenticated, for the login note
fn how_of(m: &Method) -> &'static str {
    match m {
        Method::Local => "lokal",
        Method::Line => "Leitungspasswort",
        Method::Enable => "enable-Passwort",
        Method::None => "ohne",
        _ => "RADIUS",
    }
}

/// sixteen bytes from the device's random stream
fn fill16(os: &mut OsApi, out: &mut [u8; 16]) {
    for c in out.chunks_mut(4) {
        c.copy_from_slice(&os.rand().to_be_bytes());
    }
}

/// the privilege level an Access-Accept grants: our vendor-specific "priv-lvl=N" first, then
/// the Service-Type (Administrative-User is 15, anything else 1)
fn author_level(p: &Packet) -> u8 {
    for a in p.all(radius::A_VENDOR_SPECIFIC) {
        if let Some((JFN_VENDOR, data)) = a.vendor()
            && let Some(n) = core::str::from_utf8(data).ok().and_then(|s| s.strip_prefix("priv-lvl=")).and_then(|s| s.parse::<u8>().ok())
        {
            return n.min(15);
        }
    }
    match p.int(radius::A_SERVICE_TYPE) {
        Some(radius::SERVICE_ADMINISTRATIVE) => 15,
        _ => 1,
    }
}

impl Engine {
    /// the "radius-nas" service of a device (made the first time, hidden from the inspector)
    fn nas_service(&mut self, ni: usize) -> Option<u32> {
        let dev = self.dev_of(ni);
        if let Some(id) = self.os(ni).and_then(|o| o.svc_id("radius-nas")) {
            return Some(id);
        }
        self.add_service(dev, "radius-nas", Box::new(NasService::new()), true).ok()?;
        let id = self.os(ni)?.svc_id("radius-nas")?;
        if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
            s.console = true;
        }
        Some(id)
    }

    /// starts a RADIUS login for a session: the NAS drives `methods` (which begin at a RADIUS
    /// group) with the user and the password, and later calls aaa_result with the outcome
    pub(crate) fn nas_begin(&mut self, ni: usize, via: Requester, user: &str, pass: &str, methods: Vec<Method>, want_author: bool) {
        let servers = methods.first().map(|m| self.aaa_servers_of(ni, m)).unwrap_or_default();
        let req = Req { via, user: String::from(user), pass: String::from(pass), methods, mi: 0, want_author, servers, si: 0, attempt: 1, id: 0, auth: [0; 16], wid: 0 };
        let Some(id) = self.nas_service(ni) else {
            self.aaa_result(ni, via, AuthResult::NoServer, "RADIUS");
            return;
        };
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(nas) = svc.as_any_mut().downcast_mut::<NasService>() {
                nas.begin(os, req);
            }
        });
    }
}
