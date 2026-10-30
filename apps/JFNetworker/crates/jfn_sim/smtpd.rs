// smtpd -- the SMTP server (RFC 5321 with SIZE, 8BITMIME, PIPELINING and ENHANCEDSTATUSCODES; on
// the submission port 587 of RFC 6409 AUTH PLAIN and LOGIN of RFC 4954): "smtp-server" takes
// mail for its domains into the mailboxes of its users (mailbox.rs) and relays the rest to the
// mail servers the device's resolver finds for the recipient's domain, with a queue, retries,
// a time to give up and a bounce to the sender (RFC 3464)
//
//   config (JSON, every member may be left out)
//     {"hostname": "mail.a.lab", "domains": ["a.lab"], "users": [{"name": "anna", "password":
//      "geheim"}, "bob"], "postmaster": "anna", "networks": ["10.0.1.0/24"], "open_relay": false,
//      "submission": true, "max_size_kb": 10240, "retry_ms": 60000, "retry_max_ms": 240000,
//      "give_up_ms": 600000, "connect_timeout_ms": 30000, "timeout_ms": 300000, "mailbox":
//      "/var/mail", "port": 25, "banner": "ESMTP ready for mail"}
//   SmtpServer::from_config(&config)
//
// A session starts with 220 and the banner after the host name; EHLO answers with the keywords
// SIZE (the limit), 8BITMIME, PIPELINING, ENHANCEDSTATUSCODES and on port 587 AUTH PLAIN LOGIN
// (the enhanced codes of RFC 3463 stand before the texts after an EHLO, not after HELO).  MAIL
// FROM needs a greeting first, on port 587 a login (530), and a SIZE above the limit is refused
// at once (552); RCPT TO takes a user of a local domain (an unknown one: 550 5.1.1; postmaster
// always, delivered to "postmaster" or the first user) and any other address only from a client
// that may relay: from one of "networks" (empty: the networks of the device's interfaces and
// the loopback; an IPv6 client only from ::1), logged in, or everybody with "open_relay" (554
// 5.7.1 else, a problem in the list); at most 100 recipients.  DATA (354) reads the message until the line with the dot; one
// over the limit is refused after it (552), else it goes onto the disk of the device as the
// queue (OsApi::disk_write) with a Received field on top (RFC 5321 4.4; port 587 also adds Date
// and Message-ID when they are missing) and 250 names its queue id.  RSET, NOOP, VRFY (252),
// HELP, QUIT; STARTTLS, EXPN, ETRN, BDAT and TURN get 502, unknown commands 500.  A session that
// stays silent for timeout_ms ends with 421.  Commands that came together (PIPELINING) are
// answered in their order.
//
// Delivery: a local recipient's copy goes into its Maildir (new/) with Return-Path on top
// after the disk write.  The other recipients are grouped by domain: the MX records of the
// domain (an absolute name: no search domains; none: the domain itself as implicit MX, RFC 5321
// 5.1), in the order of their preference, each host's addresses, a connection to port 25 with a
// connect timeout, the next address and the next MX when one fails; the transaction itself is
// mailc.rs's SmtpConv with PIPELINING when the next server offers it.  A 2xx is delivered, a 5xx
// (an unknown user there) or a domain DNS does not know is permanent, everything else
// (no answer, a 4xx, DNS without an answer) is deferred: the next attempt after retry_ms,
// doubling up to retry_max_ms, until give_up_ms after the message came; then it is permanent.
// An MX that points at the device itself (a loop) is permanent too.  When every recipient of a
// message is done, the sender gets one bounce (multipart/report with a text, the delivery
// status of each failed recipient and the original header) from MAILER-DAEMON with the null
// reverse path; a bounce that fails is dropped.  The queue lives in the service's memory (it
// is lost with the power, as with a disk that was not synced; the times above are shortened
// from RFC 5321 4.5.4.1 and can be changed).  Reply texts, the greeting, the bounce's text and
// the log lines are our own words with the codes and formats of the RFCs.
//
// Metrics: sessions, active, logins, failed_logins, received, delivered, relayed, deferred,
// bounced, relay_denied, rejected, too_big, queued, bytes_received.  Notes (mailtrace.rs) and log
// lines (tag "smtpd").

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::dns::{RData, TYPE_MX};
use netpkt::imf::{self, Message};
use netpkt::smtp::{self, Command, Ehlo, Reply, Stream};
use netpkt::{IpAddr, Prefix4};

use crate::http::Wheel;
use crate::kernel::{Ns, MS, SEC};
use crate::mailbox;
use crate::mailc::{SmtpConv, Step};
use crate::mailtrace::MNote;
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::resolver::{Family, LookupErr};
use crate::tcp::SockOpts;
use crate::trace::Note;

const K_WHEEL: u32 = 0;
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;
/// recipients of one message at most (RFC 5321 4.5.3.1.8)
const RCPT_MAX: usize = 100;
/// a command line longer than this ends the session (RFC 5321 4.5.3.1.4: 512 with CRLF)
const LINE_MAX: usize = 4096;
/// relays at once
const RELAYS_MAX: usize = 8;

/// a user of the server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailUser {
    pub name: String,
    /// for AUTH (empty: the user cannot log in)
    pub password: String,
}

/// what an AUTH waits for
#[derive(Clone, Debug, PartialEq, Eq)]
enum AuthWait {
    Plain,
    LoginUser,
    LoginPass(String),
}

/// a deadline
#[derive(Clone, Debug, PartialEq, Eq)]
enum Due {
    /// a session said nothing for too long
    Idle(u32),
    /// the message of a session is on the disk: 250
    Stored(u32),
    /// the copy of recipient k of a message is written
    Local(String, usize),
    /// the next attempt of a message's deferred recipients
    Retry(String),
    /// a relay's connection does not come up / its server says nothing
    Connect(u64),
    Silent(u64),
}

/// one SMTP connection of a client
#[derive(Clone, Debug)]
struct Session {
    peer: IpAddr,
    port: u16,
    input: Stream,
    helo: Option<String>,
    esmtp: bool,
    user: Option<String>,
    auth: Option<AuthWait>,
    from: Option<String>,
    body8: bool,
    /// (address, local)
    rcpts: Vec<(String, bool)>,
    /// reply bytes the connection did not take yet
    out: Vec<u8>,
    idle: Option<u64>,
    /// waiting for the disk: lines wait meanwhile
    busy: bool,
    /// the message waiting for the disk, with its queue id
    pending: Option<(String, Vec<u8>)>,
    fails: u8,
    closing: bool,
}

/// what became of a recipient of the queue
#[derive(Clone, Debug, PartialEq, Eq)]
enum RState {
    Pending,
    /// a relay or a local write is on it
    Busy,
    Done,
    Failed(Failure),
}

/// why a recipient failed (for the bounce)
#[derive(Clone, Debug, PartialEq, Eq)]
struct Failure {
    /// the enhanced status code ("5.1.1")
    status: String,
    /// the reason in the bounce's words
    text: String,
    /// the reason for notes
    why: Msg,
    /// the reply of the next server ("smtp; 550 5.1.1 ...")
    diag: Option<String>,
    /// the next server's name
    remote: Option<String>,
}

#[derive(Clone, Debug)]
struct QRcpt {
    addr: String,
    local: bool,
    state: RState,
}

/// a message of the queue
#[derive(Clone, Debug)]
struct QItem {
    from: String,
    /// with CRLF line ends, the Received field on top
    data: Vec<u8>,
    rcpts: Vec<QRcpt>,
    arrived: Ns,
    /// the device clock then (for the bounce)
    arrived_clock: i64,
    tries: u32,
    /// the deadline of the next attempt
    retry: Option<u64>,
    /// the reason of the last deferral (the bounce after giving up names it)
    last: Option<Failure>,
    /// a bounce itself (its sender is the null path)
    bounce: bool,
    /// 8-bit text (BODY=8BITMIME came, or bytes above 127): the next server hears BODY=8BITMIME
    /// when it offers it
    eight: bool,
}

/// where a relay stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RStep {
    Mx,
    Addr,
    Connect,
    Talk,
}

/// the delivery of a message's recipients of one domain to the next server
#[derive(Clone, Debug)]
struct Relay {
    item: String,
    domain: String,
    rcpts: Vec<usize>,
    step: RStep,
    lookup: Option<u32>,
    mx: Vec<(u16, String)>,
    mx_at: usize,
    host: String,
    addrs: Vec<IpAddr>,
    addr_at: usize,
    addr: Option<IpAddr>,
    sock: Option<u32>,
    conv: Option<SmtpConv>,
    deadline: Option<u64>,
    /// the last temporary reason, and whether every MX host failed for lack of an address
    why: Option<Failure>,
    no_addr: usize,
}

/// the SMTP server
#[derive(Clone, Debug)]
pub struct SmtpServer {
    pub port: u16,
    pub submission: bool,
    pub hostname: String,
    pub domains: Vec<String>,
    pub users: Vec<MailUser>,
    pub postmaster: String,
    pub networks: Vec<Prefix4>,
    pub open_relay: bool,
    pub max_size: u64,
    pub retry: Ns,
    pub retry_max: Ns,
    pub give_up: Ns,
    pub connect_timeout: Ns,
    pub timeout: Ns,
    pub mailbox: String,
    pub banner: String,
    listeners: Vec<u32>,
    sessions: BTreeMap<u32, Session>,
    queue: BTreeMap<String, QItem>,
    relays: BTreeMap<u64, Relay>,
    next_relay: u64,
    /// sockets and lookups of the relays
    relay_socks: BTreeMap<u32, u64>,
    relay_lookups: BTreeMap<u32, u64>,
    wheel: Wheel<Due>,
    /// deliveries of this run (unique file names)
    seq: u32,
    pub sessions_n: u64,
    pub logins: u64,
    pub failed_logins: u64,
    pub received: u64,
    pub delivered: u64,
    pub relayed: u64,
    pub deferred: u64,
    pub bounced: u64,
    pub relay_denied: u64,
    pub rejected: u64,
    pub too_big: u64,
    pub bytes_received: u64,
}

impl Default for SmtpServer {
    fn default() -> SmtpServer {
        SmtpServer::new()
    }
}

/// a list of strings in a configuration (one string alone counts as a list)
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(x) if x.as_str().is_some() => alloc::vec![String::from(x.as_str().unwrap_or("").trim())],
        Some(x) => x.as_array().map(|a| a.iter().filter_map(|s| s.as_str()).map(|s| String::from(s.trim())).collect()).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// a domain name as a configuration may give it: labels of letters, digits and hyphens
pub(crate) fn domain_ok(d: &str) -> bool {
    !d.is_empty() && d.len() <= 253 && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// the users of a mail service's configuration: [{"name": .., "password": ..}] or ["name"]
pub(crate) fn users_of(v: Option<&Value>) -> Result<Vec<MailUser>, Msg> {
    let mut out = Vec::new();
    for u in v.and_then(|x| x.as_array()).unwrap_or(&[]) {
        let (name, pass) = match u.as_str() {
            Some(n) => (n.trim(), ""),
            None => (u.get("name").and_then(|x| x.as_str()).map(str::trim).unwrap_or(""), u.get("password").and_then(|x| x.as_str()).unwrap_or("")),
        };
        if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c == '@' || c == '/') {
            return Err(msg!("Ein Benutzer braucht \u{201e}name\u{201c} ohne Leerzeichen, @ und /."));
        }
        if out.iter().any(|x: &MailUser| x.name.eq_ignore_ascii_case(name)) {
            return Err(msg!("Den Benutzer {} gibt es zweimal.", name));
        }
        out.push(MailUser { name: String::from(name), password: String::from(pass) });
    }
    Ok(out)
}

impl SmtpServer {
    pub fn new() -> SmtpServer {
        SmtpServer {
            port: smtp::PORT_SMTP,
            submission: true,
            hostname: String::new(),
            domains: Vec::new(),
            users: Vec::new(),
            postmaster: String::new(),
            networks: Vec::new(),
            open_relay: false,
            max_size: 10 << 20,
            retry: 60 * SEC,
            retry_max: 240 * SEC,
            give_up: 600 * SEC,
            connect_timeout: 30 * SEC,
            timeout: 300 * SEC,
            mailbox: String::from(mailbox::SPOOL),
            banner: String::from("ESMTP ready for mail"),
            listeners: Vec::new(),
            sessions: BTreeMap::new(),
            queue: BTreeMap::new(),
            relays: BTreeMap::new(),
            next_relay: 0,
            relay_socks: BTreeMap::new(),
            relay_lookups: BTreeMap::new(),
            wheel: Wheel::new(K_WHEEL),
            seq: 0,
            sessions_n: 0,
            logins: 0,
            failed_logins: 0,
            received: 0,
            delivered: 0,
            relayed: 0,
            deferred: 0,
            bounced: 0,
            relay_denied: 0,
            rejected: 0,
            too_big: 0,
            bytes_received: 0,
        }
    }

    /// the server of a project's definition (svcact.rs)
    pub fn from_config(c: &Value) -> Result<SmtpServer, Msg> {
        let mut s = SmtpServer::new();
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        let flag = |k: &str| c.get(k).and_then(|v| v.as_bool());
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(|x| String::from(x.trim()));
        if let Some(p) = num("port") {
            s.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        s.submission = flag("submission").unwrap_or(true);
        if let Some(h) = text("hostname").filter(|h| !h.is_empty()) {
            if !domain_ok(&h) {
                return Err(msg!("\u{201e}{}\u{201c} taugt nicht als Name des Servers.", h));
            }
            s.hostname = h.to_ascii_lowercase();
        }
        for d in strings(c.get("domains")) {
            let d = String::from(d.trim_end_matches('.')).to_ascii_lowercase();
            if !domain_ok(&d) {
                return Err(msg!("\u{201e}{}\u{201c} ist keine Domäne.", d));
            }
            if !s.domains.contains(&d) {
                s.domains.push(d);
            }
        }
        s.users = users_of(c.get("users"))?;
        if let Some(p) = text("postmaster").filter(|p| !p.is_empty()) {
            if !s.users.iter().any(|u| u.name.eq_ignore_ascii_case(&p)) {
                return Err(msg!("Den Benutzer {} für postmaster gibt es nicht.", p));
            }
            s.postmaster = p;
        }
        for n in strings(c.get("networks")) {
            let p = Prefix4::parse(&n).or_else(|| netpkt::Ipv4::parse(&n).and_then(|a| Prefix4::new(a, 32))).ok_or_else(|| msg!("\u{201e}{}\u{201c} ist kein Netz.", n))?;
            s.networks.push(p);
        }
        s.open_relay = flag("open_relay").unwrap_or(false);
        if let Some(k) = c.get("max_size_kb").and_then(|v| crate::httpsvc::decimal(v, 1024)) {
            s.max_size = k.max(1024);
        }
        if let Some(t) = num("retry_ms") {
            s.retry = t.max(100) * MS;
        }
        s.retry_max = num("retry_max_ms").map_or(s.retry * 4, |t| t * MS).max(s.retry);
        if let Some(t) = num("give_up_ms") {
            s.give_up = t * MS;
        }
        if let Some(t) = num("connect_timeout_ms") {
            s.connect_timeout = t.max(100) * MS;
        }
        if let Some(t) = num("timeout_ms") {
            s.timeout = t.max(1000) * MS;
        }
        if let Some(m) = text("mailbox") {
            if !m.starts_with('/') {
                return Err(msg!("\u{201e}mailbox\u{201c} ist kein Verzeichnis."));
            }
            s.mailbox = m;
        }
        if let Some(b) = text("banner").filter(|b| !b.is_empty()) {
            s.banner = b;
        }
        Ok(s)
    }

    /// a domain of this server
    pub fn is_local(&self, domain: &str) -> bool {
        let d = domain.trim_end_matches('.');
        self.domains.iter().any(|x| x.eq_ignore_ascii_case(d)) || (!self.hostname.is_empty() && self.hostname.eq_ignore_ascii_case(d))
    }

    /// the mailbox user of a local part (postmaster: the postmaster or the first user)
    pub fn user_of(&self, local: &str) -> Option<&str> {
        if let Some(u) = self.users.iter().find(|u| u.name.eq_ignore_ascii_case(local)) {
            return Some(&u.name);
        }
        if local.eq_ignore_ascii_case("postmaster") {
            if !self.postmaster.is_empty() {
                return Some(&self.postmaster);
            }
            return self.users.first().map(|u| u.name.as_str());
        }
        None
    }

    /// the name the server gives itself
    fn host(&self, os: &OsApi) -> String {
        if !self.hostname.is_empty() {
            return self.hostname.clone();
        }
        let dev = os.hostname().to_ascii_lowercase();
        match self.domains.first() {
            Some(d) => format!("{}.{}", dev, d),
            None => dev,
        }
    }

    // ---------------------------------------------------------------- replies

    fn send(&mut self, os: &mut OsApi, sock: u32, r: Reply) {
        let b = r.to_bytes();
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if !s.out.is_empty() {
            s.out.extend_from_slice(&b);
            return;
        }
        let n = os.tcp_send(sock, &b);
        s.out.extend_from_slice(&b[n..]);
    }

    /// a reply with its enhanced code when the client said EHLO
    fn say(&mut self, os: &mut OsApi, sock: u32, code: u16, status: &str, text: &str) {
        let esmtp = self.sessions.get(&sock).is_some_and(|s| s.esmtp);
        let r = if esmtp && !status.is_empty() { Reply::enh(code, status, text) } else { Reply::new(code, text) };
        self.send(os, sock, r);
    }

    /// a refused command: the note, the number, the reply
    fn refuse(&mut self, os: &mut OsApi, sock: u32, cmd: &str, code: u16, status: &str, text: &str) {
        self.rejected += 1;
        os.note(Note::Mail(MNote::SmtpRefused { cmd: String::from(cmd), code }));
        self.say(os, sock, code, status, text);
    }

    fn touch(&mut self, os: &mut OsApi, sock: u32) {
        let at = os.now() + self.timeout;
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if let Some(id) = s.idle.take() {
            self.wheel.cancel(id);
        }
        s.idle = Some(self.wheel.add(at, Due::Idle(sock)));
    }

    /// a session ends
    fn end(&mut self, os: &mut OsApi, sock: u32) {
        if let Some(s) = self.sessions.remove(&sock)
            && let Some(id) = s.idle
        {
            self.wheel.cancel(id);
        }
        os.tcp_close(sock);
    }

    fn reset(&mut self, sock: u32) {
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.from = None;
            s.rcpts.clear();
            s.body8 = false;
        }
    }

    // ---------------------------------------------------------------- commands

    /// the lines and the message that came, in order, while the session is not waiting
    fn feed(&mut self, os: &mut OsApi, sock: u32) {
        loop {
            let Some(s) = self.sessions.get_mut(&sock) else { return };
            if s.busy || s.closing {
                return;
            }
            if s.input.in_data() {
                match s.input.data() {
                    Some(d) => self.end_data(os, sock, d),
                    None => return,
                }
                continue;
            }
            match s.input.line() {
                Some(l) => self.command(os, sock, &l),
                None => {
                    if s.input.pending() > LINE_MAX {
                        self.say(os, sock, 500, "5.5.2", "Line too long, goodbye");
                        if let Some(s) = self.sessions.get_mut(&sock) {
                            s.closing = true;
                        }
                        os.tcp_shutdown(sock);
                    }
                    return;
                }
            }
        }
    }

    fn command(&mut self, os: &mut OsApi, sock: u32, line: &str) {
        self.touch(os, sock);
        if self.sessions.get(&sock).is_some_and(|s| s.auth.is_some()) {
            return self.auth_line(os, sock, line);
        }
        let Some(c) = Command::parse(line) else {
            return self.say(os, sock, 500, "5.5.2", "Empty line");
        };
        let host = self.host(os);
        match c.verb.as_str() {
            "EHLO" | "HELO" => {
                let name = c.arg.trim();
                if name.is_empty() {
                    return self.refuse(os, sock, &c.verb, 501, "5.5.4", "Syntax: EHLO hostname");
                }
                let esmtp = c.verb == "EHLO";
                let sub = self.sessions.get(&sock).is_some_and(|s| s.port != self.port);
                self.reset(sock);
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.helo = Some(String::from(name));
                    s.esmtp = esmtp;
                }
                os.note(Note::Mail(MNote::SmtpHelo { name: String::from(name), esmtp }));
                if esmtp {
                    let mut kw = alloc::vec![format!("SIZE {}", self.max_size), String::from("8BITMIME"), String::from("PIPELINING"), String::from("ENHANCEDSTATUSCODES")];
                    if sub {
                        kw.push(String::from("AUTH PLAIN LOGIN"));
                    }
                    let e = Ehlo { domain: host, keywords: kw };
                    let r = e.to_reply(&format!("welcomes {}", name));
                    self.send(os, sock, r);
                } else {
                    self.send(os, sock, Reply::new(250, &format!("{} welcomes {}", host, name)));
                }
            }
            "MAIL" => self.mail(os, sock, &c.arg),
            "RCPT" => self.rcpt(os, sock, &c.arg),
            "DATA" => self.data(os, sock, &c.arg),
            "RSET" => {
                self.reset(sock);
                self.say(os, sock, 250, "2.0.0", "Reset, start again");
            }
            "NOOP" => self.say(os, sock, 250, "2.0.0", "Still here"),
            "QUIT" => {
                self.say(os, sock, 221, "2.0.0", &format!("{} closes the connection", host));
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.closing = true;
                }
                os.tcp_shutdown(sock);
            }
            "VRFY" => self.say(os, sock, 252, "2.1.5", "That address is not checked here; send the mail and see"),
            "HELP" => self.say(os, sock, 214, "2.0.0", "Commands: EHLO HELO MAIL RCPT DATA RSET NOOP VRFY HELP QUIT and AUTH on the submission port"),
            "AUTH" => self.auth(os, sock, &c.arg),
            v if smtp::verb_known(v) => self.say(os, sock, 502, "5.5.1", "Not available here"),
            _ => self.refuse(os, sock, &c.verb, 500, "5.5.2", "Unknown command"),
        }
    }

    fn mail(&mut self, os: &mut OsApi, sock: u32, arg: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        if s.helo.is_none() {
            return self.refuse(os, sock, "MAIL", 503, "5.5.1", "Say EHLO first");
        }
        if s.from.is_some() {
            return self.refuse(os, sock, "MAIL", 503, "5.5.1", "MAIL was given already");
        }
        if s.port != self.port && s.user.is_none() {
            return self.refuse(os, sock, "MAIL", 530, "5.7.0", "Log in with AUTH first");
        }
        let Some(p) = smtp::parse_path(arg, "FROM") else {
            return self.refuse(os, sock, "MAIL", 501, "5.5.4", "Syntax: MAIL FROM:<address>");
        };
        if !p.addr.is_empty() && !imf::addr_ok(&p.addr) {
            return self.refuse(os, sock, "MAIL", 553, "5.1.7", &format!("<{}> is no valid address", p.addr));
        }
        let mut body8 = false;
        for (k, v) in &p.params {
            match k.as_str() {
                "SIZE" => {}
                "BODY" if v.eq_ignore_ascii_case("8BITMIME") => body8 = true,
                "BODY" if v.eq_ignore_ascii_case("7BIT") => {}
                "AUTH" => {}
                _ => return self.refuse(os, sock, "MAIL", 555, "5.5.4", &format!("Parameter {} is not known here", k)),
            }
        }
        let size = p.size().unwrap_or(0);
        if size > self.max_size {
            self.too_big += 1;
            os.note(Note::Mail(MNote::TooBig { size, max: self.max_size }));
            let m = self.max_size;
            return self.refuse(os, sock, "MAIL", 552, "5.3.4", &format!("Message too big, at most {} bytes", m));
        }
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.from = Some(p.addr.clone());
            s.body8 = body8;
        }
        os.note(Note::Mail(MNote::SmtpMail { from: p.addr, size }));
        self.say(os, sock, 250, "2.1.0", "Sender noted");
    }

    /// the client may hand in mail for other domains
    fn may_relay(&self, os: &OsApi, s: &Session) -> bool {
        if self.open_relay || s.user.is_some() {
            return true;
        }
        match s.peer {
            IpAddr::V4(a) => {
                if a.is_loopback() {
                    return true;
                }
                if !self.networks.is_empty() {
                    return self.networks.iter().any(|p| p.contains(a));
                }
                os.ifaces().iter().any(|f| !f.hidden && f.addrs.iter().any(|p| p.contains(a)))
            }
            IpAddr::V6(a) => a.is_loopback(),
        }
    }

    fn rcpt(&mut self, os: &mut OsApi, sock: u32, arg: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        if s.from.is_none() {
            return self.refuse(os, sock, "RCPT", 503, "5.5.1", "Send MAIL first");
        }
        if s.rcpts.len() >= RCPT_MAX {
            return self.refuse(os, sock, "RCPT", 452, "4.5.3", "Too many recipients");
        }
        let Some(p) = smtp::parse_path(arg, "TO") else {
            return self.refuse(os, sock, "RCPT", 501, "5.5.4", "Syntax: RCPT TO:<address>");
        };
        let Some((local, domain)) = imf::split_addr(&p.addr).filter(|_| imf::addr_ok(&p.addr)) else {
            return self.refuse(os, sock, "RCPT", 553, "5.1.3", &format!("<{}> is no valid address", p.addr));
        };
        let cmd = format!("RCPT TO:<{}>", p.addr);
        if self.is_local(domain) {
            if self.user_of(local).is_none() {
                return self.refuse(os, sock, &cmd, 550, "5.1.1", &format!("<{}>: no such user here", p.addr));
            }
        } else if !self.may_relay(os, s) {
            self.relay_denied += 1;
            let peer = s.peer;
            os.note(Note::Mail(MNote::RelayDenied { rcpt: p.addr.clone(), from: peer }));
            os.log(SEV_WARN, "smtpd", &format!("refused to relay for {} to <{}>", peer, p.addr));
            return self.refuse(os, sock, &cmd, 554, "5.7.1", &format!("<{}>: relaying is not allowed for you", p.addr));
        }
        let is_local = self.is_local(domain);
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.rcpts.push((p.addr.clone(), is_local));
        }
        os.note(Note::Mail(MNote::SmtpRcpt { rcpt: p.addr, local: is_local }));
        self.say(os, sock, 250, "2.1.5", "Recipient noted");
    }

    fn data(&mut self, os: &mut OsApi, sock: u32, arg: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        if !arg.trim().is_empty() {
            return self.refuse(os, sock, "DATA", 501, "5.5.4", "DATA takes no argument");
        }
        if s.from.is_none() {
            return self.refuse(os, sock, "DATA", 503, "5.5.1", "Send MAIL first");
        }
        if s.rcpts.is_empty() {
            return self.refuse(os, sock, "DATA", 554, "5.5.1", "No valid recipients");
        }
        let max = self.max_size;
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.input.begin_data(max);
        }
        self.send(os, sock, Reply::new(354, "Go ahead, end the message with a line holding only a dot"));
    }

    /// the message after DATA came: refused when too big, else onto the disk
    fn end_data(&mut self, os: &mut OsApi, sock: u32, d: smtp::Data) {
        self.touch(os, sock);
        self.bytes_received += d.size;
        if d.over {
            self.too_big += 1;
            self.reset(sock);
            os.note(Note::Mail(MNote::TooBig { size: d.size, max: self.max_size }));
            let m = self.max_size;
            return self.refuse(os, sock, "DATA", 552, "5.3.4", &format!("Message too big, at most {} bytes", m));
        }
        let Some(s) = self.sessions.get(&sock) else { return };
        let with = format!("{}{}", if s.esmtp { "ESMTP" } else { "SMTP" }, if s.user.is_some() { "A" } else { "" });
        let peer = match s.peer {
            IpAddr::V4(a) => format!("[{}]", a),
            IpAddr::V6(a) => format!("[IPv6:{}]", a),
        };
        let one = if s.rcpts.len() == 1 { s.rcpts.first().map(|r| r.0.clone()) } else { None };
        let submission = s.port != self.port;
        let helo = s.helo.clone().unwrap_or_default();
        let host = self.host(os);
        let id = self.queue_id(os);
        let mut msg = Message::parse(&d.body);
        let date = imf::date(os.clock().div_euclid(1_000_000_000));
        // submission: Date and Message-ID when the client gave none (RFC 6409 8)
        if submission {
            if msg.get("Date").is_none() {
                msg.add("Date", &date);
            }
            if msg.get("Message-ID").is_none() {
                msg.add("Message-ID", &imf::message_id(&id, &host));
            }
        }
        msg.prepend("Received", &smtp::received(&helo, &peer, &host, &with, &id, one.as_deref(), &date));
        let data = msg.to_bytes();
        let n = data.len() as u64;
        match os.disk_write(n, n as i64) {
            Some(at) => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.busy = true;
                    s.pending = Some((id, data));
                }
                self.wheel.add(at, Due::Stored(sock));
                self.wheel.arm(os);
            }
            None => {
                self.reset(sock);
                self.refuse(os, sock, "DATA", 452, "4.3.1", "No room for the message now, try later");
            }
        }
    }

    /// a new queue id: ten hexadecimal digits from the device's random numbers
    fn queue_id(&self, os: &mut OsApi) -> String {
        loop {
            let id = format!("{:05X}{:05X}", os.rand() & 0xF_FFFF, os.rand() & 0xF_FFFF);
            let taken = self.queue.contains_key(&id) || self.sessions.values().any(|s| s.pending.as_ref().is_some_and(|p| p.0 == id));
            if !taken {
                return id;
            }
        }
    }

    /// the message of a session is on the disk: 250 and into the queue
    fn stored(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        s.busy = false;
        let Some((id, data)) = s.pending.take() else { return };
        let from = s.from.clone().unwrap_or_default();
        let rcpts = s.rcpts.clone();
        let peer = s.peer;
        let eight = s.body8 || data.iter().any(|&c| c >= 0x80);
        self.reset(sock);
        self.received += 1;
        let n = data.len() as u64;
        os.note(Note::Mail(MNote::Queued { id: id.clone(), rcpts: rcpts.len() as u32, bytes: n }));
        os.log(SEV_INFO, "smtpd", &format!("{}: from=<{}> client={} size={} rcpts={}", id, from, peer, n, rcpts.len()));
        self.say(os, sock, 250, "2.0.0", &format!("Queued as {}", id));
        let item = QItem {
            from,
            data,
            rcpts: rcpts.into_iter().map(|(addr, local)| QRcpt { addr, local, state: RState::Pending }).collect(),
            arrived: os.now(),
            arrived_clock: os.clock(),
            tries: 0,
            retry: None,
            last: None,
            bounce: false,
            eight,
        };
        self.queue.insert(id.clone(), item);
        self.deliver(os, &id);
        // the lines that came meanwhile
        self.feed(os, sock);
    }

    // ---------------------------------------------------------------- AUTH (port 587)

    fn auth(&mut self, os: &mut OsApi, sock: u32, arg: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        if s.port == self.port || !self.submission {
            return self.say(os, sock, 502, "5.5.1", "AUTH is offered on the submission port only");
        }
        if s.helo.is_none() || !s.esmtp {
            return self.refuse(os, sock, "AUTH", 503, "5.5.1", "Say EHLO first");
        }
        if s.user.is_some() {
            return self.refuse(os, sock, "AUTH", 503, "5.5.1", "Logged in already");
        }
        let mut w = arg.split_whitespace();
        let mech = w.next().unwrap_or("").to_ascii_uppercase();
        let init = w.next().map(String::from);
        match (mech.as_str(), init) {
            ("PLAIN", Some(i)) => self.auth_plain(os, sock, &i),
            ("PLAIN", None) => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.auth = Some(AuthWait::Plain);
                }
                self.send(os, sock, Reply::new(334, ""));
            }
            ("LOGIN", Some(i)) => {
                let user = imf::b64_decode(i.as_bytes()).map(|b| String::from_utf8_lossy(&b).into_owned());
                match user {
                    Some(u) => {
                        if let Some(s) = self.sessions.get_mut(&sock) {
                            s.auth = Some(AuthWait::LoginPass(u));
                        }
                        self.send(os, sock, Reply::new(334, smtp::LOGIN_PASS));
                    }
                    None => self.refuse(os, sock, "AUTH", 501, "5.5.2", "That is no base64"),
                }
            }
            ("LOGIN", None) => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.auth = Some(AuthWait::LoginUser);
                }
                self.send(os, sock, Reply::new(334, smtp::LOGIN_USER));
            }
            _ => self.refuse(os, sock, "AUTH", 504, "5.5.4", "That mechanism is not offered"),
        }
    }

    /// a line of an AUTH dialogue
    fn auth_line(&mut self, os: &mut OsApi, sock: u32, line: &str) {
        let Some(wait) = self.sessions.get_mut(&sock).and_then(|s| s.auth.take()) else { return };
        let l = line.trim();
        if l == "*" {
            return self.refuse(os, sock, "AUTH", 501, "5.0.0", "AUTH cancelled");
        }
        match wait {
            AuthWait::Plain => self.auth_plain(os, sock, l),
            AuthWait::LoginUser => match imf::b64_decode(l.as_bytes()) {
                Some(b) => {
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.auth = Some(AuthWait::LoginPass(String::from_utf8_lossy(&b).into_owned()));
                    }
                    self.send(os, sock, Reply::new(334, smtp::LOGIN_PASS));
                }
                None => self.refuse(os, sock, "AUTH", 501, "5.5.2", "That is no base64"),
            },
            AuthWait::LoginPass(u) => match imf::b64_decode(l.as_bytes()) {
                Some(b) => {
                    let p = String::from_utf8_lossy(&b).into_owned();
                    self.login(os, sock, &u, &p);
                }
                None => self.refuse(os, sock, "AUTH", 501, "5.5.2", "That is no base64"),
            },
        }
    }

    fn auth_plain(&mut self, os: &mut OsApi, sock: u32, b64: &str) {
        match smtp::plain_of(b64) {
            Some((u, p)) => self.login(os, sock, &u, &p),
            None => self.refuse(os, sock, "AUTH", 501, "5.5.2", "That is no PLAIN response"),
        }
    }

    fn login(&mut self, os: &mut OsApi, sock: u32, user: &str, pass: &str) {
        let ok = self.users.iter().any(|u| u.name.eq_ignore_ascii_case(user) && !u.password.is_empty() && u.password == pass);
        let peer = self.sessions.get(&sock).map(|s| s.peer);
        os.note(Note::Mail(MNote::SmtpAuth { user: String::from(user), ok }));
        if ok {
            self.logins += 1;
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.user = Some(String::from(user));
            }
            os.log(SEV_INFO, "smtpd", &format!("{} logged in from {}", user, peer.map(|p| format!("{}", p)).unwrap_or_default()));
            return self.say(os, sock, 235, "2.7.0", "Logged in");
        }
        self.failed_logins += 1;
        os.log(SEV_WARN, "smtpd", &format!("login of {} from {} refused", user, peer.map(|p| format!("{}", p)).unwrap_or_default()));
        let last = self.sessions.get_mut(&sock).map(|s| {
            s.fails += 1;
            s.fails >= 3
        });
        self.refuse(os, sock, "AUTH", 535, "5.7.8", "Wrong user name or password");
        if last == Some(true) {
            self.say(os, sock, 421, "4.7.0", "Too many failed logins, goodbye");
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.closing = true;
            }
            os.tcp_shutdown(sock);
        }
    }
}

/// what ended a relay
#[derive(Clone, Debug)]
enum ROut {
    /// the conversation with the next server (its outcome per recipient)
    Conv(SmtpConv),
    /// no server took part: the same failure for every recipient
    Fail(Failure),
}

impl Failure {
    fn new(status: &str, text: String, why: Msg) -> Failure {
        Failure { status: String::from(status), text, why, diag: None, remote: None }
    }
    /// a 5.x.x status: no further attempt
    fn permanent(&self) -> bool {
        self.status.starts_with('5')
    }
    /// what the next server answered
    fn of_reply(r: &Reply, host: &str) -> Failure {
        let status = String::from(r.status().unwrap_or(if r.is_permanent() { "5.0.0" } else { "4.0.0" }));
        Failure { status, text: format!("{} said: {}", host, r.line()), why: msg!("{} antwortet: {}", host, r.line()), diag: Some(format!("smtp; {}", r.line())), remote: Some(String::from(host)) }
    }
    /// the reason as a message for notes
    fn msg(&self) -> Msg {
        self.why.clone()
    }
}

/// the header of a message (up to the empty line, without it) and the rest; a message whose
/// first line is no header field has none
fn split_head(data: &[u8]) -> (&[u8], &[u8]) {
    let first = data.iter().position(|&c| c == b'\n').map_or(data, |i| &data[..i]);
    let field = first.iter().position(|&c| c == b':').is_some_and(|i| i > 0 && first[..i].iter().all(|&c| c > 32 && c < 127));
    if !field {
        return (&[], data);
    }
    match data.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(i) => (&data[..i + 2], &data[i + 2..]),
        None => (data, &[]),
    }
}

/// the message with fields on top and at the end of its header, the bytes of both as they are
fn with_fields(data: &[u8], top: &str, bottom: &str) -> Vec<u8> {
    let (head, rest) = split_head(data);
    let mut v = Vec::with_capacity(data.len() + top.len() + bottom.len() + 2);
    v.extend_from_slice(top.as_bytes());
    v.extend_from_slice(head);
    v.extend_from_slice(bottom.as_bytes());
    if head.is_empty() {
        v.extend_from_slice(b"\r\n");
    }
    v.extend_from_slice(rest);
    v
}

impl SmtpServer {
    // ---------------------------------------------------------------- delivery

    /// starts what a message's pending recipients need: a write per local one, a relay per
    /// domain of the others
    fn deliver(&mut self, os: &mut OsApi, id: &str) {
        let Some(item) = self.queue.get_mut(id) else { return };
        let n = item.data.len() as u64;
        let mut locals = Vec::new();
        let mut domains: Vec<(String, Vec<usize>)> = Vec::new();
        for (k, r) in item.rcpts.iter_mut().enumerate() {
            if r.state != RState::Pending {
                continue;
            }
            if r.local {
                r.state = RState::Busy;
                locals.push(k);
                continue;
            }
            let d = imf::split_addr(&r.addr).map(|x| x.1.to_ascii_lowercase()).unwrap_or_default();
            match domains.iter_mut().find(|x| x.0 == d) {
                Some(x) => x.1.push(k),
                None => domains.push((d, alloc::vec![k])),
            }
        }
        for k in locals {
            let at = os.disk_write(n, 0).unwrap_or_else(|| os.now());
            self.wheel.add(at, Due::Local(String::from(id), k));
        }
        for (d, ks) in domains {
            if self.relays.len() >= RELAYS_MAX {
                // the others stay: a relay that ends starts them
                break;
            }
            if let Some(item) = self.queue.get_mut(id) {
                for &k in &ks {
                    item.rcpts[k].state = RState::Busy;
                }
            }
            self.next_relay += 1;
            let rid = self.next_relay;
            // the domain as an absolute name: no search domains (as res_query)
            let lid = os.resolve(&format!("{}.", d), TYPE_MX);
            self.relay_lookups.insert(lid, rid);
            self.relays.insert(
                rid,
                Relay { item: String::from(id), domain: d, rcpts: ks, step: RStep::Mx, lookup: Some(lid), mx: Vec::new(), mx_at: 0, host: String::new(), addrs: Vec::new(), addr_at: 0, addr: None, sock: None, conv: None, deadline: None, why: None, no_addr: 0 },
            );
        }
        self.wheel.arm(os);
        self.settle(os, id);
    }

    /// messages whose recipients wait without a retry (the relays were busy)
    fn kick_waiting(&mut self, os: &mut OsApi) {
        let ids: Vec<String> = self.queue.iter().filter(|(_, q)| q.retry.is_none() && q.rcpts.iter().any(|r| r.state == RState::Pending)).map(|(k, _)| k.clone()).collect();
        for id in ids {
            if self.relays.len() >= RELAYS_MAX {
                return;
            }
            self.deliver(os, &id);
        }
    }

    /// the copy of local recipient k is written: into its Maildir
    fn local(&mut self, os: &mut OsApi, id: &str, k: usize) {
        let Some(item) = self.queue.get(id) else { return };
        let Some(r) = item.rcpts.get(k) else { return };
        let addr = r.addr.clone();
        let local = imf::split_addr(&addr).map(|x| String::from(x.0)).unwrap_or_default();
        let Some(user) = self.user_of(&local).map(String::from) else {
            let f = Failure::new("5.1.1", format!("<{}>: no such user here", addr), msg!("<{}> hat hier kein Postfach", addr));
            return self.rcpt_failed(os, id, k, f);
        };
        self.seq += 1;
        let clock = os.clock();
        let name = mailbox::unique(clock.div_euclid(1_000_000_000), (clock.rem_euclid(1_000_000_000) / 1000) as u32, self.seq, &os.hostname());
        let md = mailbox::maildir(&self.mailbox, &user);
        let path = format!("{}/new/{}", md, name);
        let top = format!("Return-Path: <{}>\r\n", item.from);
        let bytes = imf::lf(&with_fields(&item.data, &top, ""));
        let r = os.store_put(&path, &bytes);
        let cur = format!("{}/cur", md);
        if r.is_ok() && !os.files().is_some_and(|f| f.is_dir(&cur)) {
            let _ = os.store_mkdir(&cur);
        }
        match r {
            Ok(p) => {
                os.store_report(&p);
                self.delivered += 1;
                if let Some(q) = self.queue.get_mut(id) {
                    q.rcpts[k].state = RState::Done;
                }
                os.note(Note::Mail(MNote::Delivered { id: String::from(id), rcpt: addr.clone(), path: p.clone() }));
                os.log(SEV_INFO, "smtpd", &format!("{}: to=<{}> status=delivered ({})", id, addr, p));
                self.settle(os, id);
            }
            Err(e) => {
                let f = Failure::new("4.2.2", format!("the mailbox of <{}> cannot take it: {}", addr, e.errno_text()), msg!("das Postfach von <{}> nimmt die Nachricht nicht: {}", addr, e.errno_text()));
                self.rcpt_deferred(os, id, k, f);
                self.settle(os, id);
            }
        }
    }

    /// a recipient failed for good
    fn rcpt_failed(&mut self, os: &mut OsApi, id: &str, k: usize, f: Failure) {
        let Some(q) = self.queue.get_mut(id) else { return };
        let addr = q.rcpts[k].addr.clone();
        q.rcpts[k].state = RState::Failed(f.clone());
        self.bounced += 1;
        os.note(Note::Mail(MNote::Bounced { id: String::from(id), rcpt: addr.clone(), why: f.msg() }));
        os.log(SEV_WARN, "smtpd", &format!("{}: to=<{}> status=bounced ({})", id, addr, f.text));
        self.settle(os, id);
    }

    /// the wait before the next attempt of a message
    fn backoff(&self, q: &QItem) -> Ns {
        let mut b = self.retry;
        for _ in 0..q.tries.min(30) {
            b = b.saturating_mul(2).min(self.retry_max);
        }
        b
    }

    /// a recipient waits for the next attempt
    fn rcpt_deferred(&mut self, os: &mut OsApi, id: &str, k: usize, f: Failure) {
        let now = os.now();
        let give_up = self.give_up;
        let wait = self.queue.get(id).map(|q| self.backoff(q)).unwrap_or(self.retry);
        let Some(q) = self.queue.get_mut(id) else { return };
        let addr = q.rcpts[k].addr.clone();
        q.rcpts[k].state = RState::Pending;
        q.last = Some(f.clone());
        // the next attempt, at the latest when it is time to give up
        let at = (now + wait).min(q.arrived + give_up).max(now);
        if q.retry.is_none() {
            q.tries += 1;
            q.retry = Some(self.wheel.add(at, Due::Retry(String::from(id))));
        }
        self.deferred += 1;
        os.note(Note::Mail(MNote::Deferred { id: String::from(id), rcpt: addr.clone(), why: f.msg(), again: at - now }));
        os.log(SEV_INFO, "smtpd", &format!("{}: to=<{}> status=deferred ({})", id, addr, f.text));
        self.wheel.arm(os);
    }

    /// the next attempt of a message, or giving up on it
    fn retry(&mut self, os: &mut OsApi, id: &str, wid: u64) {
        let now = os.now();
        let Some(q) = self.queue.get_mut(id) else { return };
        if q.retry != Some(wid) {
            return;
        }
        q.retry = None;
        if now < q.arrived + self.give_up {
            return self.deliver(os, id);
        }
        // given up: what was deferred fails with its last reason
        let last = q.last.clone().unwrap_or_else(|| Failure::new("4.4.7", String::from("no attempt worked"), msg!("kein Versuch gelang")));
        let age = crate::trace::fmt_ns((now - q.arrived) as i64);
        let age_m = jfn_model::msg::dur((now - q.arrived) as i64);
        let ks: Vec<usize> = q.rcpts.iter().enumerate().filter(|(_, r)| r.state == RState::Pending).map(|(k, _)| k).collect();
        for k in ks {
            let f = Failure { status: String::from("4.4.7"), text: format!("given up after {}: {}", age, last.text), why: msg!("aufgegeben nach {}: {}", age_m.clone(), last.why.clone()), diag: last.diag.clone(), remote: last.remote.clone() };
            self.rcpt_failed(os, id, k, f);
        }
    }

    /// a message whose recipients are all done leaves the queue; the sender hears of those
    /// that failed (a bounce of a bounce is dropped)
    fn settle(&mut self, os: &mut OsApi, id: &str) {
        let Some(q) = self.queue.get(id) else { return };
        if q.rcpts.iter().any(|r| matches!(r.state, RState::Pending | RState::Busy)) {
            return;
        }
        let Some(q) = self.queue.remove(id) else { return };
        os.disk_free(q.data.len() as u64);
        if let Some(w) = q.retry {
            self.wheel.cancel(w);
        }
        let failed: Vec<(String, Failure)> = q.rcpts.iter().filter_map(|r| if let RState::Failed(f) = &r.state { Some((r.addr.clone(), f.clone())) } else { None }).collect();
        if failed.is_empty() {
            return;
        }
        if q.bounce || q.from.is_empty() {
            os.note(Note::Mail(MNote::BounceDropped { id: String::from(id) }));
            os.log(SEV_WARN, "smtpd", &format!("{}: the bounce cannot be delivered: dropped", id));
            return;
        }
        // the bounce: from the null path to the sender
        let to = q.from.clone();
        let (lp, dom) = imf::split_addr(&to).map(|x| (String::from(x.0), String::from(x.1))).unwrap_or_default();
        let local = self.is_local(&dom);
        if local && self.user_of(&lp).is_none() {
            os.note(Note::Mail(MNote::BounceDropped { id: String::from(id) }));
            return;
        }
        let data = self.bounce_message(os, id, &q, &failed);
        let nid = self.queue_id(os);
        let n = data.len() as u64;
        let _ = os.disk_write(n, n as i64);
        os.note(Note::Mail(MNote::BounceSent { id: String::from(id), to: to.clone() }));
        os.log(SEV_INFO, "smtpd", &format!("{}: bounce {} to <{}>", id, nid, to));
        let eight = data.iter().any(|&c| c >= 0x80);
        let item = QItem { from: String::new(), data, rcpts: alloc::vec![QRcpt { addr: to, local, state: RState::Pending }], arrived: os.now(), arrived_clock: os.clock(), tries: 0, retry: None, last: None, bounce: true, eight };
        self.queue.insert(nid.clone(), item);
        self.deliver(os, &nid);
    }

    /// the bounce (RFC 3464 with RFC 6522): a text, the status of each recipient, the original
    /// header
    fn bounce_message(&self, os: &mut OsApi, id: &str, q: &QItem, failed: &[(String, Failure)]) -> Vec<u8> {
        let host = self.host(os);
        let secs = os.clock().div_euclid(1_000_000_000);
        let boundary = format!("{}.{}/{}", id, secs, host);
        let (head, _) = split_head(&q.data);
        let subject = Message::parse(&q.data).subject();
        let mut s = String::new();
        s.push_str(&imf::fold("From", &format!("Mail delivery <MAILER-DAEMON@{}>", host)));
        s.push_str(&imf::fold("To", &format!("<{}>", q.from)));
        s.push_str(&imf::fold("Subject", &imf::encode_word(&format!("Undelivered mail: {}", subject))));
        s.push_str(&imf::fold("Date", &imf::date(secs)));
        s.push_str(&imf::fold("Message-ID", &imf::message_id(&format!("{}.{}.bounce", id, secs), &host)));
        s.push_str("Auto-Submitted: auto-replied\r\nMIME-Version: 1.0\r\n");
        s.push_str(&imf::fold("Content-Type", &format!("multipart/report; report-type=delivery-status; boundary=\"{}\"", boundary)));
        s.push_str("\r\nThis is a delivery status notification in MIME format (RFC 3464).\r\n\r\n");
        s.push_str(&format!("--{}\r\nContent-Type: text/plain; charset=us-ascii\r\nContent-Description: Notification\r\n\r\n", boundary));
        s.push_str(&format!("The mail server {} could not deliver your message to\r\nthe recipients below; it will not try again.\r\n\r\n", host));
        for (a, f) in failed {
            s.push_str(&format!("<{}>: {}\r\n", a, f.text));
        }
        s.push_str(&format!("\r\n--{}\r\nContent-Type: message/delivery-status\r\nContent-Description: Delivery report\r\n\r\n", boundary));
        s.push_str(&format!("Reporting-MTA: dns; {}\r\nArrival-Date: {}\r\n", host, imf::date(q.arrived_clock.div_euclid(1_000_000_000))));
        for (a, f) in failed {
            s.push_str(&format!("\r\nFinal-Recipient: rfc822; {}\r\nAction: failed\r\nStatus: {}\r\n", a, f.status));
            if let Some(r) = &f.remote {
                s.push_str(&format!("Remote-MTA: dns; {}\r\n", r));
            }
            if let Some(d) = &f.diag {
                s.push_str(&imf::fold("Diagnostic-Code", d));
            }
        }
        s.push_str(&format!("\r\n--{}\r\nContent-Type: text/rfc822-headers\r\nContent-Description: Header of the message\r\n\r\n", boundary));
        let mut v = s.into_bytes();
        v.extend_from_slice(head);
        v.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
        v
    }

    // ---------------------------------------------------------------- relays

    /// a lookup of a relay is done
    fn resolved(&mut self, os: &mut OsApi, lid: u32) {
        let Some(rid) = self.relay_lookups.remove(&lid) else { return };
        let res = os.lookup(lid).and_then(|l| l.result.clone());
        let Some(r) = self.relays.get_mut(&rid) else { return };
        r.lookup = None;
        let d = r.domain.clone();
        match r.step {
            RStep::Mx => {
                match res {
                    Some(Ok(a)) => {
                        let mut mx: Vec<(u16, String)> = a.records.iter().filter_map(|rr| if let RData::Mx { pref, host } = &rr.data { Some((*pref, String::from(host.trim_end_matches('.')).to_ascii_lowercase())) } else { None }).collect();
                        mx.sort();
                        mx.dedup();
                        if mx.len() == 1 && mx[0].1.is_empty() {
                            // a null MX (RFC 7505): the domain takes no mail
                            let f = Failure::new("5.1.10", format!("the domain {} takes no mail (null MX)", d), msg!("die Domäne {} nimmt keine Mail an (Null-MX)", d));
                            return self.relay_done(os, rid, ROut::Fail(f));
                        }
                        if mx.is_empty() {
                            mx.push((0, d.clone()));
                        }
                        let hosts: Vec<String> = mx.iter().map(|(p, h)| format!("{} {}", p, h)).collect();
                        os.note(Note::Mail(MNote::MxFound { domain: d.clone(), hosts: hosts.join(", ") }));
                        r.mx = mx;
                    }
                    Some(Err(LookupErr::NoData)) => {
                        // no MX: the domain itself (RFC 5321 5.1)
                        os.note(Note::Mail(MNote::MxFound { domain: d.clone(), hosts: format!("0 {}", d) }));
                        r.mx = alloc::vec![(0, d.clone())];
                    }
                    Some(Err(LookupErr::NotFound)) | Some(Err(LookupErr::BadName)) => {
                        let f = Failure::new("5.1.2", format!("the domain {} does not exist (DNS)", d), msg!("die Domäne {} gibt es nicht (DNS)", d));
                        return self.relay_done(os, rid, ROut::Fail(f));
                    }
                    other => {
                        let why = match other {
                            Some(Err(e)) => e.text(),
                            _ => LookupErr::Timeout.text(),
                        };
                        let f = Failure::new("4.4.3", format!("no DNS answer for the mail servers of {}: {}", d, why), msg!("das DNS nennt die Mailserver von {} nicht: {}", d, jfn_model::msg::phrase(why)));
                        return self.relay_done(os, rid, ROut::Fail(f));
                    }
                }
                self.next_mx(os, rid);
            }
            RStep::Addr => {
                let host = r.host.clone();
                match res {
                    Some(Ok(a)) if !a.sorted.is_empty() => {
                        let all = a.sorted.clone();
                        let others: Vec<IpAddr> = all.iter().copied().filter(|x| !os.is_mine(*x)).collect();
                        if others.is_empty() {
                            // the MX is this server: a loop
                            r.why = Some(Failure::new("5.4.6", format!("mail for {} loops back to this server ({})", d, host), msg!("Mail für {} käme zu diesem Server zurück ({})", d, host)));
                            return self.next_mx(os, rid);
                        }
                        r.addrs = others;
                        r.addr_at = 0;
                        self.next_addr(os, rid);
                    }
                    Some(Ok(_)) | Some(Err(LookupErr::NotFound)) | Some(Err(LookupErr::NoData)) => {
                        r.no_addr += 1;
                        if r.why.is_none() {
                            r.why = Some(Failure::new("4.4.4", format!("the mail server {} of {} has no address", host, d), msg!("der Mailserver {} von {} hat keine Adresse", host, d)));
                        }
                        self.next_mx(os, rid);
                    }
                    other => {
                        let why = match other {
                            Some(Err(e)) => e.text(),
                            _ => LookupErr::Timeout.text(),
                        };
                        r.why = Some(Failure::new("4.4.3", format!("no DNS answer for {}: {}", host, why), msg!("das DNS nennt {} nicht: {}", host, jfn_model::msg::phrase(why))));
                        self.next_mx(os, rid);
                    }
                }
            }
            _ => {}
        }
    }

    /// the next MX host of a relay (its addresses), or the end when none is left
    fn next_mx(&mut self, os: &mut OsApi, rid: u64) {
        let Some(r) = self.relays.get_mut(&rid) else { return };
        if r.mx_at >= r.mx.len() {
            let f = if r.no_addr > 0 && r.no_addr == r.mx.len() {
                let hosts: Vec<String> = r.mx.iter().map(|m| m.1.clone()).collect();
                Failure::new("5.4.4", format!("the mail servers of {} have no address: {}", r.domain, hosts.join(", ")), msg!("die Mailserver von {} haben keine Adresse: {}", r.domain, hosts.join(", ")))
            } else {
                r.why.clone().unwrap_or_else(|| Failure::new("4.4.1", format!("no mail server of {} could be reached", r.domain), msg!("kein Mailserver von {} erreichbar", r.domain)))
            };
            return self.relay_done(os, rid, ROut::Fail(f));
        }
        let host = r.mx[r.mx_at].1.clone();
        r.mx_at += 1;
        r.host = host.clone();
        r.step = RStep::Addr;
        r.addrs.clear();
        r.addr_at = 0;
        let lid = os.resolve_host(&format!("{}.", host), Family::Any);
        r.lookup = Some(lid);
        self.relay_lookups.insert(lid, rid);
    }

    /// a connection to the next address of the MX host, or the next MX host
    fn next_addr(&mut self, os: &mut OsApi, rid: u64) {
        let ct = self.connect_timeout;
        loop {
            let Some(r) = self.relays.get_mut(&rid) else { return };
            if r.addr_at >= r.addrs.len() {
                return self.next_mx(os, rid);
            }
            let a = r.addrs[r.addr_at];
            r.addr_at += 1;
            match os.tcp_connect(a, smtp::PORT_SMTP, SockOpts::default()) {
                Ok(s) => {
                    r.sock = Some(s);
                    r.addr = Some(a);
                    r.step = RStep::Connect;
                    let at = os.now() + ct;
                    r.deadline = Some(self.wheel.add(at, Due::Connect(rid)));
                    self.relay_socks.insert(s, rid);
                    self.wheel.arm(os);
                    return;
                }
                Err(e) => r.why = Some(Failure::new("4.4.1", format!("no connection to {} [{}]: {}", r.host, a, e), msg!("keine Verbindung zu {} [{}]: {}", r.host, a, e))),
            }
        }
    }

    /// the socket of a relay goes
    fn relay_drop_sock(&mut self, os: &mut OsApi, rid: u64, abort: bool) {
        let Some(r) = self.relays.get_mut(&rid) else { return };
        if let Some(s) = r.sock.take() {
            self.relay_socks.remove(&s);
            if abort {
                os.tcp_abort(s);
            }
            os.tcp_close(s);
        }
        if let Some(d) = r.deadline.take() {
            self.wheel.cancel(d);
        }
    }

    /// a relay's connection came up: the conversation
    fn relay_up(&mut self, os: &mut OsApi, rid: u64) {
        let host = self.host(os);
        let timeout = self.timeout;
        let Some(r) = self.relays.get_mut(&rid) else { return };
        let Some(q) = self.queue.get(&r.item) else { return };
        if let Some(d) = r.deadline.take() {
            self.wheel.cancel(d);
        }
        let rcpts: Vec<String> = r.rcpts.iter().map(|&k| q.rcpts[k].addr.clone()).collect();
        let mut conv = SmtpConv::new(&host, &q.from, rcpts, q.data.clone());
        conv.pipelining = true;
        if q.eight {
            // the same bytes, announced as 8BITMIME where the next server takes it
            conv.data8 = Some(q.data.clone());
        }
        r.conv = Some(conv);
        r.step = RStep::Talk;
        let at = os.now() + timeout;
        r.deadline = Some(self.wheel.add(at, Due::Silent(rid)));
        self.wheel.arm(os);
    }

    /// bytes from the next server
    fn relay_data(&mut self, os: &mut OsApi, rid: u64, sock: u32) {
        let b = os.tcp_recv(sock, usize::MAX);
        let timeout = self.timeout;
        let Some(r) = self.relays.get_mut(&rid) else { return };
        let Some(conv) = r.conv.as_mut() else { return };
        let mut end = false;
        for st in conv.on_bytes(&b) {
            match st {
                Step::Send(_, bytes) => {
                    os.tcp_send(sock, &bytes);
                }
                Step::End => end = true,
                Step::Got(_) => {}
            }
        }
        if let Some(d) = r.deadline.take() {
            self.wheel.cancel(d);
        }
        if end {
            let conv = r.conv.take();
            self.relay_drop_sock(os, rid, false);
            if let Some(c) = conv {
                self.relay_done(os, rid, ROut::Conv(c));
            }
            return;
        }
        let at = os.now() + timeout;
        r.deadline = Some(self.wheel.add(at, Due::Silent(rid)));
        self.wheel.arm(os);
    }

    /// the connection of a relay ended (or could not be made)
    fn relay_closed(&mut self, os: &mut OsApi, rid: u64, end: Option<crate::tcp::TcpEnd>) {
        let Some(r) = self.relays.get_mut(&rid) else { return };
        let (host, addr) = (r.host.clone(), r.addr);
        let a = addr.map(|x| format!("{}", x)).unwrap_or_default();
        match r.step {
            RStep::Connect => {
                let why = end.and_then(|e| e.errno()).unwrap_or("Connection closed");
                r.why = Some(Failure::new("4.4.1", format!("no connection to {} [{}]: {}", host, a, why), msg!("keine Verbindung zu {} [{}]: {}", host, a, why)));
                self.relay_drop_sock(os, rid, false);
                self.next_addr(os, rid);
            }
            RStep::Talk => {
                let conv = r.conv.take();
                self.relay_drop_sock(os, rid, false);
                match conv {
                    Some(c) if c.done || c.ok() => self.relay_done(os, rid, ROut::Conv(c)),
                    _ => {
                        let f = Failure::new("4.4.2", format!("the connection to {} [{}] broke off", host, a), msg!("die Verbindung zu {} [{}] brach ab", host, a));
                        self.relay_done(os, rid, ROut::Fail(f));
                    }
                }
            }
            _ => {}
        }
    }

    /// a relay is over: each of its recipients delivered, failed or deferred
    fn relay_done(&mut self, os: &mut OsApi, rid: u64, out: ROut) {
        self.relay_drop_sock(os, rid, true);
        let Some(r) = self.relays.remove(&rid) else { return };
        if let Some(l) = r.lookup {
            self.relay_lookups.remove(&l);
        }
        let id = r.item.clone();
        let host = r.host.clone();
        let addr = r.addr.map(|a| format!("{}", a)).unwrap_or_default();
        for (j, &k) in r.rcpts.iter().enumerate() {
            let Some(rcpt) = self.queue.get(&id).and_then(|q| q.rcpts.get(k)).map(|x| x.addr.clone()) else { continue };
            let res = match &out {
                ROut::Conv(c) => match c.outcome(j) {
                    Some(Ok(reply)) => Ok(reply),
                    Some(Err(reply)) => Err(Failure::of_reply(&reply, &host)),
                    None => Err(Failure::new("4.4.2", format!("the connection to {} [{}] broke off", host, addr), msg!("die Verbindung zu {} [{}] brach ab", host, addr))),
                },
                ROut::Fail(f) => Err(f.clone()),
            };
            match res {
                Ok(reply) => {
                    if let Some(q) = self.queue.get_mut(&id) {
                        q.rcpts[k].state = RState::Done;
                    }
                    self.relayed += 1;
                    os.note(Note::Mail(MNote::Relayed { id: id.clone(), rcpt: rcpt.clone(), host: host.clone(), reply: reply.line() }));
                    os.log(SEV_INFO, "smtpd", &format!("{}: to=<{}> relay={}[{}]:25 status=sent ({})", id, rcpt, host, addr, reply.line()));
                }
                Err(f) if f.permanent() => {
                    if let Some(q) = self.queue.get_mut(&id) {
                        q.rcpts[k].state = RState::Busy;
                    }
                    self.rcpt_failed(os, &id, k, f);
                }
                Err(f) => self.rcpt_deferred(os, &id, k, f),
            }
        }
        self.settle(os, &id);
        self.kick_waiting(os);
    }

    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (wid, d) in self.wheel.due(now) {
            match d {
                Due::Idle(s) => {
                    if self.sessions.get(&s).is_some_and(|x| x.idle == Some(wid)) {
                        let host = self.host(os);
                        self.say(os, s, 421, "4.4.2", &format!("{} heard nothing for too long, closing", host));
                        self.end(os, s);
                    }
                }
                Due::Stored(s) => self.stored(os, s),
                Due::Local(id, k) => self.local(os, &id, k),
                Due::Retry(id) => self.retry(os, &id, wid),
                Due::Connect(rid) => {
                    let ours = self.relays.get(&rid).is_some_and(|r| r.deadline == Some(wid));
                    if ours {
                        let (host, a) = self.relays.get(&rid).map(|r| (r.host.clone(), r.addr.map(|x| format!("{}", x)).unwrap_or_default())).unwrap_or_default();
                        if let Some(r) = self.relays.get_mut(&rid) {
                            r.deadline = None;
                            r.why = Some(Failure::new("4.4.1", format!("{} [{}] does not answer", host, a), msg!("{} [{}] antwortet nicht", host, a)));
                        }
                        self.relay_drop_sock(os, rid, true);
                        self.next_addr(os, rid);
                    }
                }
                Due::Silent(rid) => {
                    let ours = self.relays.get(&rid).is_some_and(|r| r.deadline == Some(wid));
                    if ours {
                        let host = self.relays.get(&rid).map(|r| r.host.clone()).unwrap_or_default();
                        if let Some(r) = self.relays.get_mut(&rid) {
                            r.deadline = None;
                        }
                        let f = Failure::new("4.4.2", format!("{} said nothing for too long", host), msg!("{} schweigt zu lange", host));
                        self.relay_done(os, rid, ROut::Fail(f));
                    }
                }
            }
        }
        self.wheel.arm(os);
    }

    /// one event of the service
    fn handle(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if self.listeners.contains(&listener) => {
                while let Some(c) = os.tcp_accept(listener) {
                    let row = os.tcp_info(c);
                    let (peer, port) = row.map(|r| (r.remote.0, r.local.1)).unwrap_or((crate::os::ANY4, self.port));
                    self.sessions_n += 1;
                    self.sessions.insert(
                        c,
                        Session { peer, port, input: Stream::new(), helo: None, esmtp: false, user: None, auth: None, from: None, body8: false, rcpts: Vec::new(), out: Vec::new(), idle: None, busy: false, pending: None, fails: 0, closing: false },
                    );
                    os.note(Note::Mail(MNote::SmtpSession { from: peer, port }));
                    os.log(SEV_INFO, "smtpd", &format!("connection from {} on port {}", peer, port));
                    let greet = format!("{} {}", self.host(os), self.banner);
                    self.send(os, c, Reply::new(220, &greet));
                    self.touch(os, c);
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                if self.sessions.contains_key(&sock) {
                    let b = os.tcp_recv(sock, usize::MAX);
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        if s.closing {
                            return;
                        }
                        s.input.push(&b);
                    }
                    self.feed(os, sock);
                } else if let Some(&rid) = self.relay_socks.get(&sock) {
                    self.relay_data(os, rid, sock);
                }
            }
            SvcEvent::TcpConnected { sock, .. } => {
                if let Some(&rid) = self.relay_socks.get(&sock) {
                    self.relay_up(os, rid);
                }
            }
            SvcEvent::TcpWritable { sock } => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    let n = os.tcp_send(sock, &s.out);
                    s.out.drain(..n);
                }
            }
            SvcEvent::TcpEof { sock, .. } => {
                if self.sessions.contains_key(&sock) {
                    self.end(os, sock);
                } else if let Some(&rid) = self.relay_socks.get(&sock) {
                    let rest = os.tcp_recv(sock, usize::MAX);
                    if !rest.is_empty()
                        && let Some(c) = self.relays.get_mut(&rid).and_then(|r| r.conv.as_mut())
                    {
                        let _ = c.on_bytes(&rest);
                    }
                    self.relay_closed(os, rid, None);
                }
            }
            SvcEvent::TcpClosed { sock, end } => {
                if self.sessions.contains_key(&sock) {
                    self.end(os, sock);
                } else if let Some(&rid) = self.relay_socks.get(&sock) {
                    self.relay_closed(os, rid, Some(end));
                } else {
                    os.tcp_close(sock);
                }
            }
            SvcEvent::Resolved(id) => self.resolved(os, id),
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            _ => {}
        }
    }
}

impl Service for SmtpServer {
    fn kind(&self) -> &'static str {
        "smtp-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        let mut ports = alloc::vec![self.port];
        if self.submission && self.port != smtp::PORT_SUBMISSION {
            ports.push(smtp::PORT_SUBMISSION);
        }
        for p in ports {
            match os.tcp_listen(p, 0, SockOpts::default()) {
                Ok(l) => self.listeners.push(l),
                Err(_) => os.note(Note::PortInUse { port: p, svc: String::from("smtp-server") }),
            }
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.handle(ev, os);
        self.wheel.arm(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        // the queue is lost with the power (it lived in memory)
        let held: u64 = self.queue.values().map(|q| q.data.len() as u64).sum();
        os.disk_free(held);
        self.listeners.clear();
        self.sessions.clear();
        self.queue.clear();
        self.relays.clear();
        self.relay_socks.clear();
        self.relay_lookups.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sessions", self.sessions_n as i64),
            Metric::new("active", self.sessions.len() as i64),
            Metric::new("logins", self.logins as i64),
            Metric::new("failed_logins", self.failed_logins as i64),
            Metric::new("received", self.received as i64),
            Metric::new("delivered", self.delivered as i64),
            Metric::new("relayed", self.relayed as i64),
            Metric::new("deferred", self.deferred as i64),
            Metric::new("bounced", self.bounced as i64),
            Metric::new("relay_denied", self.relay_denied as i64),
            Metric::new("rejected", self.rejected as i64),
            Metric::new("too_big", self.too_big as i64),
            Metric::new("queued", self.queue.len() as i64),
            Metric::new("bytes_received", self.bytes_received as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        let ports = if self.submission && self.port != smtp::PORT_SUBMISSION { format!("{}, {}", self.port, smtp::PORT_SUBMISSION) } else { format!("{}", self.port) };
        let doms = self.domains.join(", ");
        let (r, d, f, b, q) = (self.received, self.delivered, self.relayed, self.bounced, self.queue.len());
        match (self.domains.is_empty(), self.open_relay) {
            (true, false) => msg!("Port {}, ohne Domäne: {} empfangen, {} weitergeleitet, {} unzustellbar, {} in der Warteschlange", ports, r, f, b, q),
            (true, true) => msg!("Port {}, ohne Domäne, offener Relay: {} empfangen, {} weitergeleitet, {} unzustellbar, {} in der Warteschlange", ports, r, f, b, q),
            (false, false) => msg!("Port {}, {}: {} empfangen, {} zugestellt, {} weitergeleitet, {} unzustellbar, {} in der Warteschlange", ports, doms, r, d, f, b, q),
            (false, true) => msg!("Port {}, {}, offener Relay: {} empfangen, {} zugestellt, {} weitergeleitet, {} unzustellbar, {} in der Warteschlange", ports, doms, r, d, f, b, q),
        }
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
        let q: usize = self.queue.values().map(|x| x.data.len() + x.rcpts.len() * 96 + 256).sum();
        let s: usize = self.sessions.values().map(|x| x.input.pending() + x.out.len() + x.pending.as_ref().map_or(0, |p| p.1.len()) + 512).sum();
        let r: usize = self.relays.values().map(|x| x.conv.as_ref().map_or(0, |c| c.data.len()) + 512).sum();
        1024 + q + s + r + self.users.len() * 64
    }
}

impl crate::kernel::Engine {
    /// the problems of mail (problems.rs): MX records whose host has no address or no SMTP
    /// server at it, open relays, POP3 users whose mail cannot arrive
    pub(crate) fn mail_problems(&self, out: &mut Vec<crate::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        let name_of = |dev: u32| self.node(dev).map(|n| n.name.clone()).unwrap_or_default();
        // the addresses of the zones, the MX records (device, service, owner, host)
        let mut addrs: Vec<(String, IpAddr)> = Vec::new();
        let mut mxs: Vec<(u32, String, String, String)> = Vec::new();
        for dev in self.devices() {
            let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else { continue };
            for s in os.svcs.iter().filter(|s| s.enabled && !s.console) {
                let Some(d) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<crate::dnsd::DnsServer>()) else { continue };
                for z in &d.zones {
                    for rr in &z.records {
                        match &rr.data {
                            RData::A(a) => addrs.push((rr.name.clone(), IpAddr::V4(*a))),
                            RData::Aaaa(a) => addrs.push((rr.name.clone(), IpAddr::V6(*a))),
                            RData::Mx { host, .. } => mxs.push((dev, s.name.clone(), rr.name.clone(), host.clone())),
                            _ => {}
                        }
                    }
                }
            }
        }
        // the devices with a running SMTP server
        let smtp_at = |a: IpAddr| self.devices().any(|d| self.node_index(d).is_some_and(|ni| self.is_mine_ip(ni, a) && self.os(ni).is_some_and(|o| o.svcs.iter().any(|s| s.enabled && s.kind == "smtp-server"))));
        for (dev, svc, owner, host) in mxs {
            let h = host.trim_end_matches('.');
            if h.is_empty() {
                // a null MX: no mail on purpose
                continue;
            }
            let targets: Vec<IpAddr> = IpAddr::parse(h).into_iter().chain(addrs.iter().filter(|(n, _)| netpkt::dns::name_eq(n, h)).map(|x| x.1)).collect();
            let name = name_of(dev);
            if targets.is_empty() {
                let msg = msg!("Der MX-Eintrag von {} (Dienst {} auf {}) zeigt auf {}, das keine Adresse hat.", owner, svc, name, h);
                out.push(Problem { kind: ProblemKind::MailMxNowhere, severity: Severity::Error, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
                continue;
            }
            if !targets.iter().any(|&a| smtp_at(a)) {
                let msg = msg!("Der MX-Eintrag von {} zeigt auf {} ({}), dort läuft kein SMTP-Server.", owner, h, targets[0]);
                out.push(Problem { kind: ProblemKind::MailMxNowhere, severity: Severity::Warning, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
            }
        }
        for dev in self.devices() {
            let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else { continue };
            let name = name_of(dev);
            let smtps: Vec<&SmtpServer> = os.svcs.iter().filter(|s| s.enabled && !s.console).filter_map(|s| s.svc.as_ref()?.as_any().downcast_ref::<SmtpServer>()).collect();
            for s in os.svcs.iter().filter(|s| s.enabled && !s.console) {
                if let Some(x) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<SmtpServer>())
                    && x.open_relay
                {
                    let msg = msg!("Dienst {} auf {} ist ein offener Relay: er leitet Mail jedes Absenders weiter.", s.name, name);
                    out.push(Problem { kind: ProblemKind::MailOpenRelay, severity: Severity::Warning, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
                }
                let Some(p) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<crate::pop3d::Pop3Server>()) else { continue };
                for u in &p.users {
                    let boxed = mailbox::has_mailbox(&os.files, &p.mailbox, &u.name);
                    let taken = smtps.iter().any(|x| x.mailbox == p.mailbox && x.users.iter().any(|y| y.name.eq_ignore_ascii_case(&u.name)));
                    if !boxed && !taken {
                        let msg = msg!("POP3-Benutzer {} auf {} hat kein Postfach, und kein SMTP-Server dort nimmt Mail für ihn an.", u.name, name);
                        out.push(Problem { kind: ProblemKind::MailNoMailbox, severity: Severity::Warning, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
                    }
                }
            }
        }
    }
}
