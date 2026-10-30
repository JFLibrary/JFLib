// pop3d -- the POP3 server (RFC 1939 with UIDL and TOP, CAPA and the response codes of RFC 2449):
// "pop3-server" on TCP port 110 lets its users fetch the messages of their mailboxes, which the
// SMTP server delivers into the device's files (mailbox.rs)
//
//   config (JSON, every member may be left out)
//     {"port": 110, "users": [{"name": "bob", "password": "geheim"}], "mailbox": "/var/mail",
//      "fail_delay_ms": 1000, "max_fails": 3, "idle_ms": 600000, "banner": "POP3 ready"}
//   Pop3Server::from_config(&config)
//
// A session starts with +OK and the banner (AUTHORIZATION).  USER and PASS log in: a wrong
// password is answered after fail_delay_ms with "-ERR [AUTH] ...", and after max_fails of them
// the connection ends; a mailbox that another session holds is refused with [IN-USE].  The
// login takes the messages of the mailbox as they are then (TRANSACTION): STAT, LIST [n], UIDL
// [n] (the unique name of the Maildir file), RETR n and TOP n m (after the time of a read on the
// device's disk: OsApi::disk_read), DELE n (marks it), RSET (unmarks all), NOOP, CAPA, QUIT.
// QUIT in TRANSACTION removes the marked messages (UPDATE, one step of the project) and says
// how many; a connection that ends without QUIT removes nothing (RFC 1939 6).  Sizes count the
// message with CRLF line ends, as it travels.  A session silent for idle_ms (at least ten
// minutes by the RFC, shorter here only when set) ends without UPDATE.  The texts after +OK and
// -ERR are our own words.
//
// Metrics: sessions, active, logins, failed_logins, retrieved, deleted, bytes_sent.  Notes
// (mailtrace.rs) and log lines (tag "pop3d").

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::ftp::Lines;
use netpkt::imf;
use netpkt::pop3::{self, Command, Response};
use netpkt::IpAddr;

use crate::http::Wheel;
use crate::kernel::{Ns, MS, SEC};
use crate::mailbox;
use crate::mailtrace::MNote;
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::smtpd::{users_of, MailUser};
use crate::tcp::SockOpts;
use crate::trace::Note;

const K_WHEEL: u32 = 0;
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;

/// a message of a maildrop
#[derive(Clone, Debug, PartialEq, Eq)]
struct PMsg {
    uid: String,
    path: String,
    size: u64,
    deleted: bool,
}

/// a deadline
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// the answer to a wrong password
    Refuse(u32),
    /// a message to send was read from the disk: (session, message number, lines of TOP)
    Read(u32, u32, Option<u32>),
    /// the marked messages are removed: +OK and the end
    Updated(u32),
    Idle(u32),
}

/// one connection of a client
#[derive(Clone, Debug)]
struct Session {
    peer: IpAddr,
    lines: Lines,
    user: Option<String>,
    /// logged in: the maildrop's user and its messages
    login: Option<(String, Vec<PMsg>)>,
    fails: u8,
    /// waiting (a wrong password, a read): lines wait meanwhile
    busy: bool,
    queue: VecDeque<String>,
    out: Vec<u8>,
    idle: Option<u64>,
    closing: bool,
}

/// the POP3 server
#[derive(Clone, Debug)]
pub struct Pop3Server {
    pub port: u16,
    pub users: Vec<MailUser>,
    pub mailbox: String,
    pub fail_delay: Ns,
    pub max_fails: u8,
    pub idle: Ns,
    pub banner: String,
    listener: Option<u32>,
    sessions: BTreeMap<u32, Session>,
    /// the users whose maildrop a session holds
    locked: BTreeSet<String>,
    wheel: Wheel<Due>,
    pub sessions_n: u64,
    pub logins: u64,
    pub failed_logins: u64,
    pub retrieved: u64,
    pub deleted: u64,
    pub bytes_sent: u64,
}

impl Default for Pop3Server {
    fn default() -> Pop3Server {
        Pop3Server::new()
    }
}

impl Pop3Server {
    pub fn new() -> Pop3Server {
        Pop3Server {
            port: pop3::PORT_POP3,
            users: Vec::new(),
            mailbox: String::from(mailbox::SPOOL),
            fail_delay: SEC,
            max_fails: 3,
            idle: 600 * SEC,
            banner: String::from("POP3 ready"),
            listener: None,
            sessions: BTreeMap::new(),
            locked: BTreeSet::new(),
            wheel: Wheel::new(K_WHEEL),
            sessions_n: 0,
            logins: 0,
            failed_logins: 0,
            retrieved: 0,
            deleted: 0,
            bytes_sent: 0,
        }
    }

    /// the server of a project's definition (svcact.rs)
    pub fn from_config(c: &Value) -> Result<Pop3Server, Msg> {
        let mut s = Pop3Server::new();
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(|x| String::from(x.trim()));
        if let Some(p) = num("port") {
            s.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        s.users = users_of(c.get("users"))?;
        if let Some(m) = text("mailbox") {
            if !m.starts_with('/') {
                return Err(msg!("\u{201e}mailbox\u{201c} ist kein Verzeichnis."));
            }
            s.mailbox = m;
        }
        if let Some(t) = num("fail_delay_ms") {
            s.fail_delay = t.min(60_000) * MS;
        }
        if let Some(n) = num("max_fails") {
            s.max_fails = n.clamp(1, 100) as u8;
        }
        if let Some(t) = num("idle_ms") {
            s.idle = t.max(1000) * MS;
        }
        if let Some(b) = text("banner").filter(|b| !b.is_empty()) {
            s.banner = b;
        }
        Ok(s)
    }

    fn send(&mut self, os: &mut OsApi, sock: u32, r: &Response) {
        let b = r.to_bytes();
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if !s.out.is_empty() {
            s.out.extend_from_slice(&b);
            return;
        }
        let n = os.tcp_send(sock, &b);
        s.out.extend_from_slice(&b[n..]);
    }

    fn ok(&mut self, os: &mut OsApi, sock: u32, text: &str) {
        self.send(os, sock, &Response::ok(text));
    }

    fn err(&mut self, os: &mut OsApi, sock: u32, cmd: &str, text: &str) {
        os.note(Note::Mail(MNote::Pop3Refused { cmd: String::from(cmd) }));
        self.send(os, sock, &Response::err(text));
    }

    fn touch(&mut self, os: &mut OsApi, sock: u32) {
        let at = os.now() + self.idle;
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if let Some(id) = s.idle.take() {
            self.wheel.cancel(id);
        }
        s.idle = Some(self.wheel.add(at, Due::Idle(sock)));
    }

    /// a session ends (its maildrop free again)
    fn end(&mut self, os: &mut OsApi, sock: u32) {
        if let Some(s) = self.sessions.remove(&sock) {
            if let Some(id) = s.idle {
                self.wheel.cancel(id);
            }
            if let Some((u, _)) = s.login {
                self.locked.remove(&u.to_ascii_lowercase());
            }
        }
        os.tcp_close(sock);
    }

    fn lines(&mut self, os: &mut OsApi, sock: u32, data: &[u8]) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if s.closing {
            return;
        }
        let ls = s.lines.push(data);
        s.queue.extend(ls);
        self.drain(os, sock);
    }

    /// the lines that wait, while the session is not busy
    fn drain(&mut self, os: &mut OsApi, sock: u32) {
        while let Some(l) = self.sessions.get_mut(&sock).and_then(|s| if s.busy || s.closing { None } else { s.queue.pop_front() }) {
            self.command(os, sock, &l);
        }
    }

    /// the message of number n (from 1) of a session that is not deleted
    fn msg_of(&self, sock: u32, n: Option<u32>) -> Result<(u32, PMsg), &'static str> {
        let Some((_, msgs)) = self.sessions.get(&sock).and_then(|s| s.login.as_ref()) else { return Err("Log in first") };
        let Some(n) = n else { return Err("Which message? A number is missing") };
        match msgs.get((n as usize).wrapping_sub(1)) {
            Some(m) if m.deleted => Err("That message is marked as deleted"),
            Some(m) => Ok((n, m.clone())),
            None => Err("There is no such message"),
        }
    }

    fn command(&mut self, os: &mut OsApi, sock: u32, line: &str) {
        let Some(c) = Command::parse(line) else { return };
        self.touch(os, sock);
        let logged = self.sessions.get(&sock).is_some_and(|s| s.login.is_some());
        let v = c.verb.clone();
        match v.as_str() {
            "CAPA" => {
                let caps = "USER\r\nUIDL\r\nTOP\r\nPIPELINING\r\nRESP-CODES\r\nAUTH-RESP-CODE\r\n";
                self.send(os, sock, &Response::ok("What this server can do:").with_body(caps.as_bytes()));
            }
            "QUIT" => self.quit(os, sock),
            "NOOP" if logged => self.ok(os, sock, "Still here"),
            "USER" if !logged => {
                let name = c.args.first().cloned().unwrap_or_default();
                if name.is_empty() {
                    return self.err(os, sock, "USER", "USER needs a name");
                }
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.user = Some(name);
                }
                self.ok(os, sock, "Now the password with PASS");
            }
            "PASS" if !logged => self.pass(os, sock, c.args.first().map(String::as_str).unwrap_or("")),
            "USER" | "PASS" | "APOP" | "AUTH" => self.err(os, sock, &v, "Logged in already"),
            _ if !logged => {
                if pop3::verb_known(&v) {
                    self.err(os, sock, &v, "Log in with USER and PASS first")
                } else {
                    self.err(os, sock, &v, "Unknown command")
                }
            }
            "STAT" => {
                let (n, size) = self.sessions.get(&sock).and_then(|s| s.login.as_ref()).map(|(_, m)| m.iter().filter(|x| !x.deleted).fold((0u32, 0u64), |a, x| (a.0 + 1, a.1 + x.size))).unwrap_or((0, 0));
                self.ok(os, sock, &format!("{} {}", n, size));
            }
            "LIST" | "UIDL" => {
                let uidl = v == "UIDL";
                if c.args.is_empty() {
                    let mut body = String::new();
                    let mut n = 0;
                    if let Some((_, msgs)) = self.sessions.get(&sock).and_then(|s| s.login.as_ref()) {
                        for (k, m) in msgs.iter().enumerate().filter(|(_, m)| !m.deleted) {
                            n += 1;
                            if uidl {
                                body.push_str(&format!("{} {}\r\n", k + 1, m.uid));
                            } else {
                                body.push_str(&format!("{} {}\r\n", k + 1, m.size));
                            }
                        }
                    }
                    let head = match (uidl, n) {
                        (true, _) => String::from("Unique names follow"),
                        (false, 1) => String::from("1 message follows"),
                        (false, n) => format!("{} messages follow", n),
                    };
                    return self.send(os, sock, &Response::ok(&head).with_body(body.as_bytes()));
                }
                match self.msg_of(sock, c.num(0)) {
                    Ok((n, m)) if uidl => self.ok(os, sock, &format!("{} {}", n, m.uid)),
                    Ok((n, m)) => self.ok(os, sock, &format!("{} {}", n, m.size)),
                    Err(e) => self.err(os, sock, &v, e),
                }
            }
            "RETR" | "TOP" => {
                let lines = if v == "TOP" { c.num(1) } else { None };
                if v == "TOP" && lines.is_none() {
                    return self.err(os, sock, "TOP", "TOP takes a message number and a count of lines");
                }
                match self.msg_of(sock, c.num(0)) {
                    Ok((n, m)) => {
                        // the file from the disk first
                        let at = os.disk_read(m.size);
                        if let Some(s) = self.sessions.get_mut(&sock) {
                            s.busy = true;
                        }
                        self.wheel.add(at, Due::Read(sock, n, lines));
                        self.wheel.arm(os);
                    }
                    Err(e) => self.err(os, sock, &v, e),
                }
            }
            "DELE" => match self.msg_of(sock, c.num(0)) {
                Ok((n, _)) => {
                    if let Some((_, msgs)) = self.sessions.get_mut(&sock).and_then(|s| s.login.as_mut()) {
                        msgs[n as usize - 1].deleted = true;
                    }
                    self.ok(os, sock, &format!("Message {} marked for deletion", n));
                }
                Err(e) => self.err(os, sock, "DELE", e),
            },
            "RSET" => {
                let mut n = 0;
                if let Some((_, msgs)) = self.sessions.get_mut(&sock).and_then(|s| s.login.as_mut()) {
                    for m in msgs.iter_mut() {
                        m.deleted = false;
                        n += 1;
                    }
                }
                self.ok(os, sock, &format!("All {} messages kept", n));
            }
            v if pop3::verb_known(v) => self.err(os, sock, v, "Not available here"),
            _ => self.err(os, sock, &v, "Unknown command"),
        }
    }

    fn pass(&mut self, os: &mut OsApi, sock: u32, pw: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        let Some(name) = s.user.clone() else {
            return self.err(os, sock, "PASS", "USER first, then PASS");
        };
        let peer = s.peer;
        let user = self.users.iter().find(|u| u.name.eq_ignore_ascii_case(&name) && !u.password.is_empty() && u.password == pw).map(|u| u.name.clone());
        let Some(user) = user else {
            self.failed_logins += 1;
            os.note(Note::Mail(MNote::Pop3LoginFailed { user: name.clone() }));
            os.log(SEV_WARN, "pop3d", &format!("login of {} from {} refused", name, peer));
            let at = os.now() + self.fail_delay;
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.busy = true;
                s.user = None;
                s.fails += 1;
            }
            self.wheel.add(at, Due::Refuse(sock));
            self.wheel.arm(os);
            return;
        };
        let key = user.to_ascii_lowercase();
        if self.locked.contains(&key) {
            return self.err(os, sock, "PASS", "[IN-USE] Another session holds this mailbox");
        }
        let msgs: Vec<PMsg> = os.files().map(|st| mailbox::messages(st, &self.mailbox, &user)).unwrap_or_default().into_iter().map(|(uid, path, size)| PMsg { uid, path, size, deleted: false }).collect();
        let (n, size) = (msgs.len() as u32, msgs.iter().map(|m| m.size).sum::<u64>());
        self.locked.insert(key);
        self.logins += 1;
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.login = Some((user.clone(), msgs));
        }
        os.note(Note::Mail(MNote::Pop3Login { user: user.clone(), msgs: n, bytes: size }));
        os.log(SEV_INFO, "pop3d", &format!("{} logged in from {}: {} messages, {} bytes", user, peer, n, size));
        let what = if n == 1 { String::from("1 message") } else { format!("{} messages", n) };
        self.ok(os, sock, &format!("Logged in, {} ({} octets)", what, size));
    }

    /// the answer to a wrong password, after the delay; the lines that came meanwhile follow
    fn refuse_now(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        s.busy = false;
        let last = s.fails >= self.max_fails;
        self.send(os, sock, &Response::err("[AUTH] Wrong user name or password"));
        if last {
            self.send(os, sock, &Response::err("Too many failed logins, goodbye"));
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.closing = true;
            }
            os.tcp_shutdown(sock);
            return;
        }
        self.drain(os, sock);
    }

    /// a message was read from the disk: RETR or TOP answers
    fn read_done(&mut self, os: &mut OsApi, sock: u32, n: u32, top: Option<u32>) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        s.busy = false;
        let Some(m) = s.login.as_ref().and_then(|(_, msgs)| msgs.get(n as usize - 1)).cloned() else { return };
        let data = match os.file_read(&m.path) {
            Ok(d) => imf::crlf(&d),
            Err(_) => {
                self.err(os, sock, "RETR", "The message is gone from the disk");
                return self.drain(os, sock);
            }
        };
        let body: Vec<u8> = match top {
            None => data,
            Some(k) => {
                // the header, the empty line and k lines of the body
                let mut out = Vec::new();
                let mut in_body = false;
                let mut left = k;
                for l in data.split_inclusive(|&c| c == b'\n') {
                    if in_body {
                        if left == 0 {
                            break;
                        }
                        left -= 1;
                    } else if l == b"\r\n" {
                        in_body = true;
                    }
                    out.extend_from_slice(l);
                }
                out
            }
        };
        let size = body.len() as u64;
        if top.is_none() {
            self.retrieved += 1;
        }
        self.bytes_sent += size;
        os.note(Note::Mail(MNote::Pop3Retr { n, bytes: size }));
        let head = if top.is_none() { format!("{} octets follow", m.size) } else { String::from("The top of the message follows") };
        self.send(os, sock, &Response::ok(&head).with_body(&body));
        self.drain(os, sock);
    }

    /// QUIT: in TRANSACTION the marked messages go first (UPDATE)
    fn quit(&mut self, os: &mut OsApi, sock: u32) {
        let gone: Vec<String> = self.sessions.get(&sock).and_then(|s| s.login.as_ref()).map(|(_, m)| m.iter().filter(|x| x.deleted).map(|x| x.path.clone()).collect()).unwrap_or_default();
        if gone.is_empty() {
            self.ok(os, sock, "Goodbye");
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.closing = true;
            }
            os.tcp_shutdown(sock);
            return;
        }
        // the removals take the disk's time
        let at = os.disk_write(gone.len() as u64 * 512, 0).unwrap_or_else(|| os.now());
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.busy = true;
        }
        self.wheel.add(at, Due::Updated(sock));
        self.wheel.arm(os);
    }

    /// UPDATE: the marked messages are removed, +OK, the end
    fn updated(&mut self, os: &mut OsApi, sock: u32) {
        let Some((user, msgs)) = self.sessions.get(&sock).and_then(|s| s.login.clone()) else { return };
        let mut n = 0u32;
        let mut failed = false;
        let mut last = String::new();
        for m in msgs.iter().filter(|m| m.deleted) {
            match os.store_remove(&m.path) {
                Ok(()) => {
                    n += 1;
                    last = m.path.clone();
                }
                Err(_) => failed = true,
            }
        }
        if n > 0 {
            os.store_report(&last);
        }
        self.deleted += n as u64;
        os.note(Note::Mail(MNote::Pop3Deleted { user: user.clone(), n }));
        os.log(SEV_INFO, "pop3d", &format!("{} removed {} messages", user, n));
        if failed {
            self.send(os, sock, &Response::err("Some deleted messages could not be removed"));
        } else {
            let left = msgs.len() as u32 - n;
            self.ok(os, sock, &format!("Goodbye, {} messages removed, {} left", n, left));
        }
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.busy = false;
            s.closing = true;
        }
        os.tcp_shutdown(sock);
    }

    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (id, d) in self.wheel.due(now) {
            match d {
                Due::Refuse(s) => self.refuse_now(os, s),
                Due::Read(s, n, top) => self.read_done(os, s, n, top),
                Due::Updated(s) => self.updated(os, s),
                Due::Idle(s) => {
                    if self.sessions.get(&s).is_some_and(|x| x.idle == Some(id)) {
                        self.send(os, s, &Response::err("Nothing happened for too long, goodbye"));
                        self.end(os, s);
                    }
                }
            }
        }
        self.wheel.arm(os);
    }
}

impl Service for Pop3Server {
    fn kind(&self) -> &'static str {
        "pop3-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        match os.tcp_listen(self.port, 0, SockOpts::default()) {
            Ok(l) => self.listener = Some(l),
            Err(_) => os.note(Note::PortInUse { port: self.port, svc: String::from("pop3-server") }),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(c) = os.tcp_accept(listener) {
                    let peer = os.tcp_info(c).map(|r| r.remote.0).unwrap_or(crate::os::ANY4);
                    self.sessions_n += 1;
                    self.sessions.insert(c, Session { peer, lines: Lines::new(), user: None, login: None, fails: 0, busy: false, queue: VecDeque::new(), out: Vec::new(), idle: None, closing: false });
                    os.note(Note::Mail(MNote::Pop3Session { from: peer }));
                    let b = self.banner.clone();
                    self.ok(os, c, &b);
                    self.touch(os, c);
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                let b = os.tcp_recv(sock, usize::MAX);
                if self.sessions.contains_key(&sock) {
                    self.lines(os, sock, &b);
                }
            }
            SvcEvent::TcpWritable { sock } => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    let n = os.tcp_send(sock, &s.out);
                    s.out.drain(..n);
                }
            }
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } => {
                if self.sessions.contains_key(&sock) {
                    self.end(os, sock);
                } else {
                    os.tcp_close(sock);
                }
            }
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            _ => {}
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.listener = None;
        self.sessions.clear();
        self.locked.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sessions", self.sessions_n as i64),
            Metric::new("active", self.sessions.len() as i64),
            Metric::new("logins", self.logins as i64),
            Metric::new("failed_logins", self.failed_logins as i64),
            Metric::new("retrieved", self.retrieved as i64),
            Metric::new("deleted", self.deleted as i64),
            Metric::new("bytes_sent", self.bytes_sent as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Port {}, {} Benutzer: {} Sitzungen, {} Anmeldungen, {} abgelehnt, {} Nachrichten abgeholt, {} gelöscht", self.port, self.users.len(), self.sessions_n, self.logins, self.failed_logins, self.retrieved, self.deleted)
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
        1024 + self.users.len() * 64 + self.sessions.values().map(|s| 512 + s.out.len() + s.login.as_ref().map_or(0, |l| l.1.len() * 160)).sum::<usize>()
    }
}
