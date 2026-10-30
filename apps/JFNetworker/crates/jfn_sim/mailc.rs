// mailc -- the clients of mail: the SMTP conversation the SMTP server's relay, the E-Mail
// program, curl and mail share, a connection to a server by name (Dial), the POP3 conversation
// of the E-Mail program, its account as a file, the mail folders of a device and the E-Mail
// program's side in the engine
//
//   SmtpConv        one SMTP transaction as a client: greeting, EHLO (HELO when the server does
//                   not know it), AUTH PLAIN or LOGIN when the server offers AUTH and the client
//                   has a login, MAIL FROM with SIZE and BODY=8BITMIME where offered, RCPT TO for
//                   every recipient, DATA with the message dot-stuffed, QUIT; with `pipelining`
//                   MAIL, the RCPTs and DATA go in one segment when the server offers PIPELINING
//                   (RFC 2920); on_bytes(bytes) -> steps (lines to send, replies that came),
//                   outcome(k) for each recipient
//   Dial            the server's addresses from the resolver (getaddrinfo order), one connection
//                   after the other with a connect timeout, as curl and Postfix try them
//   Fetch           the E-Mail program's POP3 session: USER, PASS, UIDL, RETR of the messages not
//                   fetched before, DELE unless they stay on the server, QUIT
//   Account         the E-Mail program's settings, a file of "key = value" lines (ACCOUNT:
//                   /root/Mail/account): name, address, smtp, smtp_port, pop3, pop3_port, user,
//                   password, keep
//   folder(store, dir) -> Vec<MailEntry>   the messages of a folder (INBOX /root/Mail/inbox,
//                   SENT /root/Mail/sent): sender, recipients, subject, date, size, by date
//   MailClient      the service "mail-client" of the E-Mail program (hidden: it is a program, no
//                   server): Action::MailSend { dev, draft } and Action::MailFetch { dev } queue
//                   a job, which runs when the ones before it are done; Engine::mail_client(dev)
//                   gives the last send and fetch with their outcome for the status line
//
// A sent message is kept as SENT/<unique>.eml, a fetched one as INBOX/<UIDL>.eml (line ends
// "\n"), the UIDLs fetched in /root/Mail/.uidl (so with "keep" a message is fetched once); the
// files come as one step of the project per send and per fetch.  The message is the one
// netpkt::imf::Draft writes: UTF-8 text in 8bit when the server offers 8BITMIME, else
// quoted-printable; its Date and Message-ID come from the device clock and random numbers.

use alloc::boxed::Box;
use alloc::collections::{BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use jfn_model::files::Store;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::imf::{self, Draft, Mailbox, Message};
use netpkt::pop3;
use netpkt::smtp::{self, Command, Ehlo, Reply, ReplyReader};
use netpkt::IpAddr;

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Ctl, Metric, OsApi, Service, SvcEvent};
use crate::resolver::{Family, LookupErr};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;
use crate::mailtrace::MNote;

// ---------------------------------------------------------------- the SMTP conversation

/// where an SMTP conversation stands (what the last reply answered)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Greet,
    Ehlo,
    Helo,
    Auth,
    Mail,
    Rcpt,
    Data,
    Dot,
    Quit,
}

impl Stage {
    /// the command of the stage as curl and logs name it
    pub fn word(self) -> &'static str {
        match self {
            Stage::Greet => "greeting",
            Stage::Ehlo => "EHLO",
            Stage::Helo => "HELO",
            Stage::Auth => "AUTH",
            Stage::Mail => "MAIL",
            Stage::Rcpt => "RCPT",
            Stage::Data => "DATA",
            Stage::Dot => "end of data",
            Stage::Quit => "QUIT",
        }
    }
}

/// what a reply answers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Greet,
    Ehlo,
    Helo,
    Auth,
    AuthUser,
    AuthPass,
    Mail,
    Rcpt(usize),
    Data,
    Dot,
    Quit,
}

/// what the owner of a conversation does next
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// send these bytes (the command lines as the -v trace shows them: "EHLO pc1"; empty for
    /// the message)
    Send(Vec<String>, Vec<u8>),
    /// a reply came (for the trace)
    Got(Reply),
    /// the conversation is over: close the connection
    End,
}

/// one SMTP transaction as a client
#[derive(Clone, Debug)]
pub(crate) struct SmtpConv {
    pub helo: String,
    pub auth: Option<(String, String)>,
    pub from: String,
    pub rcpts: Vec<String>,
    /// the message with CRLF line ends (7-bit safe)
    pub data: Vec<u8>,
    /// the same message in 8bit, sent instead when the server offers 8BITMIME
    pub data8: Option<Vec<u8>>,
    /// MAIL, RCPT and DATA in one segment when the server offers PIPELINING
    pub pipelining: bool,
    pub ehlo: Option<Ehlo>,
    reader: ReplyReader,
    wait: VecDeque<Want>,
    /// per recipient: the refusal (None: accepted)
    pub refused: Vec<Option<Reply>>,
    pub accepted: usize,
    /// the reply after the message
    pub final_reply: Option<Reply>,
    /// the reply that ended the transaction early, and to what
    pub fail: Option<(Stage, Reply)>,
    tried_helo: bool,
    pub done: bool,
}

impl SmtpConv {
    pub fn new(helo: &str, from: &str, rcpts: Vec<String>, data: Vec<u8>) -> SmtpConv {
        let n = rcpts.len();
        SmtpConv {
            helo: String::from(helo),
            auth: None,
            from: String::from(from),
            rcpts,
            data,
            data8: None,
            pipelining: false,
            ehlo: None,
            reader: ReplyReader::new(),
            wait: VecDeque::from([Want::Greet]),
            refused: alloc::vec![None; n],
            accepted: 0,
            final_reply: None,
            fail: None,
            tried_helo: false,
            done: false,
        }
    }

    fn cmd(out: &mut Vec<Step>, verb: &str, arg: &str) {
        let c = Command::new(verb, arg);
        let line = c.to_line();
        // the trace shows AUTH's secrets as they go, like curl -v does
        out.push(Step::Send(alloc::vec![String::from(line.trim_end())], line.into_bytes()));
    }

    /// the message that goes: the 8-bit one where the server takes it
    fn message(&self) -> &[u8] {
        match (&self.data8, &self.ehlo) {
            (Some(d), Some(e)) if e.has("8BITMIME") => d,
            _ => &self.data,
        }
    }

    /// MAIL FROM (with the RCPTs and DATA when pipelining)
    fn mail(&mut self, out: &mut Vec<Step>) {
        let size = format!("{}", self.message().len());
        // BODY=8BITMIME only for a message with 8-bit bytes (RFC 6152)
        let eight = self.data8.as_ref().is_some_and(|d| d.iter().any(|&c| c >= 0x80)) && self.ehlo.as_ref().is_some_and(|e| e.has("8BITMIME"));
        let mut ps: Vec<(&str, &str)> = Vec::new();
        if self.ehlo.as_ref().is_some_and(|e| e.has("SIZE")) {
            ps.push(("SIZE", &size));
        }
        if eight {
            ps.push(("BODY", "8BITMIME"));
        }
        let mail = smtp::fmt_path("FROM", &self.from, &ps);
        let pipe = self.pipelining && self.ehlo.as_ref().is_some_and(|e| e.has("PIPELINING"));
        if !pipe {
            Self::cmd(out, "MAIL", &mail);
            self.wait.push_back(Want::Mail);
            return;
        }
        let mut shown = Vec::new();
        let mut bytes = Vec::new();
        let mut add = |verb: &str, arg: &str| {
            let l = Command::new(verb, arg).to_line();
            shown.push(String::from(l.trim_end()));
            bytes.extend_from_slice(l.as_bytes());
        };
        add("MAIL", &mail);
        self.wait.push_back(Want::Mail);
        for (k, r) in self.rcpts.iter().enumerate() {
            add("RCPT", &smtp::fmt_path("TO", r, &[]));
            self.wait.push_back(Want::Rcpt(k));
        }
        add("DATA", "");
        self.wait.push_back(Want::Data);
        out.push(Step::Send(shown, bytes));
    }

    fn quit(&mut self, out: &mut Vec<Step>) {
        if self.wait.contains(&Want::Quit) {
            return;
        }
        Self::cmd(out, "QUIT", "");
        self.wait.push_back(Want::Quit);
    }

    /// ends the transaction early with a reply (the replies still due are read, then QUIT)
    fn failed(&mut self, out: &mut Vec<Step>, st: Stage, r: Reply) {
        if self.fail.is_none() {
            self.fail = Some((st, r));
        }
        if self.wait.is_empty() {
            self.quit(out);
        }
    }

    /// after EHLO / HELO / AUTH: the transaction
    fn go(&mut self, out: &mut Vec<Step>) {
        self.mail(out);
    }

    /// bytes of the server: what to do
    pub fn on_bytes(&mut self, b: &[u8]) -> Vec<Step> {
        let mut out = Vec::new();
        for r in self.reader.push(b) {
            out.push(Step::Got(r.clone()));
            let Some(w) = self.wait.pop_front() else { continue };
            self.on_reply(&mut out, w, r);
        }
        out
    }

    fn on_reply(&mut self, out: &mut Vec<Step>, w: Want, r: Reply) {
        if self.fail.is_some() && w != Want::Quit {
            // replies to commands of a pipeline that failed already
            if w == Want::Data && r.code == 354 {
                // the server wants a message nobody accepted: an empty one (RFC 2920 3.1)
                out.push(Step::Send(Vec::new(), b".\r\n".to_vec()));
                self.wait.push_front(Want::Dot);
                return;
            }
            if self.wait.is_empty() {
                self.quit(out);
            }
            return;
        }
        match w {
            Want::Greet if r.code == 220 => {
                let h = self.helo.clone();
                Self::cmd(out, "EHLO", &h);
                self.wait.push_back(Want::Ehlo);
            }
            Want::Greet => self.failed(out, Stage::Greet, r),
            Want::Ehlo if r.code == 250 => {
                let e = Ehlo::of_reply(&r);
                let auth = self.auth.clone();
                let offered = e.has("AUTH");
                self.ehlo = Some(e);
                match auth {
                    Some((u, p)) if offered && self.ehlo.as_ref().is_some_and(|e| e.auth("PLAIN")) => {
                        Self::cmd(out, "AUTH", &format!("PLAIN {}", smtp::auth_plain(&u, &p)));
                        self.wait.push_back(Want::Auth);
                    }
                    Some(_) if offered && self.ehlo.as_ref().is_some_and(|e| e.auth("LOGIN")) => {
                        Self::cmd(out, "AUTH", "LOGIN");
                        self.wait.push_back(Want::AuthUser);
                    }
                    _ => self.go(out),
                }
            }
            Want::Ehlo if !self.tried_helo && r.is_permanent() => {
                // a server without the extensions: HELO
                self.tried_helo = true;
                let h = self.helo.clone();
                Self::cmd(out, "HELO", &h);
                self.wait.push_back(Want::Helo);
            }
            Want::Ehlo => self.failed(out, Stage::Ehlo, r),
            Want::Helo if r.code == 250 => self.go(out),
            Want::Helo => self.failed(out, Stage::Helo, r),
            Want::AuthUser if r.code == 334 => {
                let u = self.auth.as_ref().map(|a| imf::b64_encode(a.0.as_bytes())).unwrap_or_default();
                out.push(Step::Send(alloc::vec![u.clone()], format!("{}\r\n", u).into_bytes()));
                self.wait.push_back(Want::AuthPass);
            }
            Want::AuthPass if r.code == 334 => {
                let p = self.auth.as_ref().map(|a| imf::b64_encode(a.1.as_bytes())).unwrap_or_default();
                out.push(Step::Send(alloc::vec![p.clone()], format!("{}\r\n", p).into_bytes()));
                self.wait.push_back(Want::Auth);
            }
            Want::Auth if r.code == 235 => self.go(out),
            Want::Auth | Want::AuthUser | Want::AuthPass => self.failed(out, Stage::Auth, r),
            Want::Mail if r.is_positive() => {
                if !self.wait.iter().any(|x| matches!(x, Want::Rcpt(_))) {
                    if self.rcpts.is_empty() {
                        return self.failed(out, Stage::Rcpt, Reply::new(554, "no recipients"));
                    }
                    let to = smtp::fmt_path("TO", &self.rcpts[0], &[]);
                    Self::cmd(out, "RCPT", &to);
                    self.wait.push_back(Want::Rcpt(0));
                }
            }
            Want::Mail => self.failed(out, Stage::Mail, r),
            Want::Rcpt(k) => {
                if r.is_positive() {
                    self.accepted += 1;
                } else if let Some(x) = self.refused.get_mut(k) {
                    *x = Some(r.clone());
                }
                let pipelined = self.wait.contains(&Want::Data);
                if pipelined {
                    return;
                }
                if k + 1 < self.rcpts.len() {
                    let to = smtp::fmt_path("TO", &self.rcpts[k + 1], &[]);
                    Self::cmd(out, "RCPT", &to);
                    self.wait.push_back(Want::Rcpt(k + 1));
                } else if self.accepted > 0 {
                    Self::cmd(out, "DATA", "");
                    self.wait.push_back(Want::Data);
                } else {
                    self.failed(out, Stage::Rcpt, r);
                }
            }
            Want::Data if r.code == 354 => {
                if self.accepted == 0 {
                    out.push(Step::Send(Vec::new(), b".\r\n".to_vec()));
                    self.wait.push_back(Want::Dot);
                    let last = self.refused.iter().rev().flatten().next().cloned().unwrap_or_else(|| Reply::new(554, "no recipients"));
                    self.fail = Some((Stage::Rcpt, last));
                    return;
                }
                let m = smtp::stuff(self.message());
                out.push(Step::Send(Vec::new(), m));
                self.wait.push_back(Want::Dot);
            }
            Want::Data => {
                // after a pipeline: no recipient may be the reason
                if self.accepted == 0 {
                    let last = self.refused.iter().rev().flatten().next().cloned().unwrap_or(r);
                    return self.failed(out, Stage::Rcpt, last);
                }
                self.failed(out, Stage::Data, r)
            }
            Want::Dot if r.is_positive() => {
                self.final_reply = Some(r);
                self.quit(out);
            }
            Want::Dot => self.failed(out, Stage::Dot, r),
            Want::Quit => {
                self.done = true;
                out.push(Step::End);
            }
        }
    }

    /// the message was accepted for at least one recipient
    pub fn ok(&self) -> bool {
        self.fail.is_none() && self.final_reply.is_some()
    }

    /// what became of recipient k: Ok(the reply after the message) or Err(the refusal); None
    /// while it is open (the connection broke before)
    pub fn outcome(&self, k: usize) -> Option<Result<Reply, Reply>> {
        if let Some(Some(r)) = self.refused.get(k) {
            return Some(Err(r.clone()));
        }
        if let Some((_, r)) = &self.fail {
            return Some(Err(r.clone()));
        }
        self.final_reply.clone().map(Ok)
    }
}

// ---------------------------------------------------------------- a connection by name

/// why a connection could not be made
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialErr {
    Lookup(LookupErr),
    Connect(TcpEnd),
    Timeout,
}

impl DialErr {
    /// German words for notes and status lines
    pub fn msg(&self, host: &str) -> Msg {
        match self {
            DialErr::Lookup(LookupErr::NotFound) | DialErr::Lookup(LookupErr::NoData) => msg!("den Namen {} kennt das DNS nicht", host),
            DialErr::Lookup(e) => msg!("{} ist nicht aufzulösen: {}", host, phrase_of(e.text())),
            DialErr::Connect(e) => msg!("keine Verbindung zu {}: {}", host, phrase_of(e.text())),
            DialErr::Timeout => msg!("{} antwortet nicht", host),
        }
    }
}

/// a fixed German term as a message (the app's catalogue has it)
fn phrase_of(s: &'static str) -> Msg {
    jfn_model::msg::phrase(s)
}

/// what a Dial reports
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialEv {
    /// connected on this socket
    Up(u32),
    Fail(DialErr),
}

/// a connection to a server by name or address: its addresses one after the other
#[derive(Clone, Debug)]
pub(crate) struct Dial {
    pub host: String,
    pub port: u16,
    pub family: Family,
    /// the service's timer key for the connect timeout
    key: u32,
    pub timeout: Ns,
    lookup: Option<u32>,
    pub addrs: Vec<IpAddr>,
    at: usize,
    pub sock: Option<u32>,
    pub addr: Option<IpAddr>,
    pub up: bool,
    last: Option<DialErr>,
    /// "Trying ..." lines for curl -v
    pub trace: Vec<String>,
}

impl Dial {
    pub fn new(host: &str, port: u16, key: u32, timeout: Ns) -> Dial {
        Dial { host: String::from(host), port, family: Family::Any, key, timeout, lookup: None, addrs: Vec::new(), at: 0, sock: None, addr: None, up: false, last: None, trace: Vec::new() }
    }
    /// looks the name up (or connects to an address at once)
    pub fn start(&mut self, os: &mut OsApi) -> Option<DialEv> {
        match IpAddr::parse(self.host.trim_start_matches('[').trim_end_matches(']')) {
            Some(a) => {
                self.addrs = alloc::vec![a];
                self.next(os)
            }
            None => {
                self.lookup = Some(os.resolve_host(&self.host, self.family));
                None
            }
        }
    }
    /// the next address
    fn next(&mut self, os: &mut OsApi) -> Option<DialEv> {
        while self.at < self.addrs.len() {
            let a = self.addrs[self.at];
            self.at += 1;
            let ep = if a.is_v4() { format!("{}:{}", a, self.port) } else { format!("[{}]:{}", a, self.port) };
            self.trace.push(format!("*   Trying {}...", ep));
            match os.tcp_connect(a, self.port, SockOpts::default()) {
                Ok(s) => {
                    self.sock = Some(s);
                    self.addr = Some(a);
                    os.timer(self.key, os.now() + self.timeout);
                    return None;
                }
                Err(_) => self.last = Some(DialErr::Connect(TcpEnd::Unreachable)),
            }
        }
        Some(DialEv::Fail(self.last.clone().unwrap_or(DialErr::Connect(TcpEnd::Refused))))
    }
    /// an event of the service: Some when the connection is up or cannot be made
    pub fn on(&mut self, ev: &SvcEvent, os: &mut OsApi) -> Option<DialEv> {
        match ev {
            SvcEvent::Resolved(id) if Some(*id) == self.lookup => {
                self.lookup = None;
                let res = os.lookup(*id).and_then(|l| l.result.clone());
                match res {
                    Some(Ok(a)) if !a.sorted.is_empty() => {
                        self.addrs = a.sorted.clone();
                        self.next(os)
                    }
                    Some(Ok(_)) => Some(DialEv::Fail(DialErr::Lookup(LookupErr::NoData))),
                    Some(Err(e)) => Some(DialEv::Fail(DialErr::Lookup(e))),
                    None => Some(DialEv::Fail(DialErr::Lookup(LookupErr::Timeout))),
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(*sock) == self.sock && !self.up => {
                self.up = true;
                os.cancel(self.key);
                Some(DialEv::Up(*sock))
            }
            SvcEvent::TcpClosed { sock, end } if Some(*sock) == self.sock && !self.up => {
                os.tcp_close(*sock);
                self.sock = None;
                os.cancel(self.key);
                self.last = Some(DialErr::Connect(*end));
                self.next(os)
            }
            _ => None,
        }
    }
    /// the connect timeout fired: the next address
    pub fn timer(&mut self, os: &mut OsApi) -> Option<DialEv> {
        if self.up {
            return None;
        }
        if let Some(s) = self.sock.take() {
            os.tcp_abort(s);
            os.tcp_close(s);
        }
        self.last = Some(DialErr::Timeout);
        self.next(os)
    }
    /// the connection goes (the owner is done with it)
    pub fn close(&mut self, os: &mut OsApi) {
        if let Some(s) = self.sock.take() {
            os.tcp_close(s);
        }
        os.cancel(self.key);
    }
}

// ---------------------------------------------------------------- the E-Mail program's POP3 session

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FStep {
    Greet,
    User,
    Pass,
    Uidl,
    Retr(usize),
    Dele(usize),
    Quit,
    Done,
}

/// the POP3 session of the E-Mail program: the messages not fetched before
#[derive(Clone, Debug)]
pub(crate) struct Fetch {
    user: String,
    pass: String,
    keep: bool,
    known: BTreeSet<String>,
    reader: pop3::Reader,
    step: FStep,
    /// (number, UIDL) of the new messages
    todo: Vec<(u32, String)>,
    /// the messages fetched: (UIDL, bytes with CRLF)
    pub got: Vec<(String, Vec<u8>)>,
    /// the -ERR that ended it, and to which command
    pub fail: Option<(&'static str, String)>,
    pub done: bool,
}

impl Fetch {
    fn new(user: &str, pass: &str, keep: bool, known: BTreeSet<String>) -> Fetch {
        Fetch { user: String::from(user), pass: String::from(pass), keep, known, reader: pop3::Reader::new(), step: FStep::Greet, todo: Vec::new(), got: Vec::new(), fail: None, done: false }
    }
    fn send(&mut self, out: &mut Vec<Vec<u8>>, verb: &str, args: &[&str], step: FStep) {
        let c = pop3::Command::new(verb, args);
        self.reader.expect(pop3::multi_line(&c));
        out.push(c.to_line().into_bytes());
        self.step = step;
    }
    fn quit(&mut self, out: &mut Vec<Vec<u8>>) {
        self.send(out, "QUIT", &[], FStep::Quit);
    }
    /// the next RETR, the DELEs, QUIT
    fn next(&mut self, out: &mut Vec<Vec<u8>>, k: usize) {
        if k < self.todo.len() {
            let n = format!("{}", self.todo[k].0);
            return self.send(out, "RETR", &[&n], FStep::Retr(k));
        }
        if !self.keep {
            return self.dele(out, 0);
        }
        self.quit(out);
    }
    fn dele(&mut self, out: &mut Vec<Vec<u8>>, k: usize) {
        if k < self.todo.len() && !self.todo.is_empty() {
            let n = format!("{}", self.todo[k].0);
            return self.send(out, "DELE", &[&n], FStep::Dele(k));
        }
        self.quit(out);
    }
    /// bytes of the server: the lines to send
    fn on_bytes(&mut self, b: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for r in self.reader.push(b) {
            match self.step {
                FStep::Greet if r.ok => {
                    let u = self.user.clone();
                    self.send(&mut out, "USER", &[&u], FStep::User);
                }
                FStep::User if r.ok => {
                    let p = self.pass.clone();
                    self.send(&mut out, "PASS", &[&p], FStep::Pass);
                }
                FStep::Pass if r.ok => self.send(&mut out, "UIDL", &[], FStep::Uidl),
                FStep::Uidl if r.ok => {
                    let body = r.body.clone().unwrap_or_default();
                    for l in String::from_utf8_lossy(&body).lines() {
                        let mut w = l.split_whitespace();
                        if let (Some(n), Some(u)) = (w.next().and_then(|x| x.parse::<u32>().ok()), w.next())
                            && !self.known.contains(u)
                        {
                            self.todo.push((n, String::from(u)));
                        }
                    }
                    self.next(&mut out, 0);
                }
                FStep::Retr(k) if r.ok => {
                    let uid = self.todo[k].1.clone();
                    self.got.push((uid, r.body.clone().unwrap_or_default()));
                    self.next(&mut out, k + 1);
                }
                FStep::Dele(k) => self.dele(&mut out, k + 1),
                FStep::Quit => {
                    self.step = FStep::Done;
                    self.done = true;
                }
                FStep::Done => {}
                st => {
                    let what = match st {
                        FStep::Greet => "greeting",
                        FStep::User => "USER",
                        FStep::Pass => "PASS",
                        FStep::Uidl => "UIDL",
                        _ => "RETR",
                    };
                    self.fail = Some((what, r.text.clone()));
                    self.quit(&mut out);
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------- the account and the folders

/// the E-Mail program's settings
pub const ACCOUNT: &str = "/root/Mail/account";
/// the messages fetched
pub const INBOX: &str = "/root/Mail/inbox";
/// the messages sent
pub const SENT: &str = "/root/Mail/sent";
/// the UIDLs fetched before (one per line)
pub const UIDLS: &str = "/root/Mail/.uidl";

/// the E-Mail program's account
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub address: String,
    pub smtp: String,
    pub smtp_port: u16,
    pub pop3: String,
    pub pop3_port: u16,
    pub user: String,
    pub password: String,
    /// messages stay on the server after they were fetched
    pub keep: bool,
}

impl Account {
    /// "key = value" lines (# comments, unknown keys left out)
    pub fn parse(text: &str) -> Account {
        let mut a = Account { smtp_port: smtp::PORT_SMTP, pop3_port: pop3::PORT_POP3, ..Account::default() };
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let v = String::from(v.trim());
            match k.trim() {
                "name" => a.name = v,
                "address" => a.address = v,
                "smtp" => a.smtp = v,
                "smtp_port" => a.smtp_port = v.parse().unwrap_or(smtp::PORT_SMTP),
                "pop3" => a.pop3 = v,
                "pop3_port" => a.pop3_port = v.parse().unwrap_or(pop3::PORT_POP3),
                "user" => a.user = v,
                "password" => a.password = v,
                "keep" => a.keep = matches!(v.as_str(), "yes" | "true" | "1"),
                _ => {}
            }
        }
        a
    }
    /// the file's text
    pub fn to_text(&self) -> String {
        format!(
            "# E-Mail: the account of the program on the desktop\nname = {}\naddress = {}\nsmtp = {}\nsmtp_port = {}\npop3 = {}\npop3_port = {}\nuser = {}\npassword = {}\nkeep = {}\n",
            self.name.trim(),
            self.address.trim(),
            self.smtp.trim(),
            self.smtp_port,
            self.pop3.trim(),
            self.pop3_port,
            self.user.trim(),
            self.password,
            if self.keep { "yes" } else { "no" }
        )
    }
    /// the account of a device's files (None: there is none)
    pub fn of(st: &Store) -> Option<Account> {
        let b = st.read(ACCOUNT).ok()?;
        Some(Account::parse(&String::from_utf8_lossy(b)))
    }
    /// the sender as a mailbox
    pub fn mailbox(&self) -> Mailbox {
        Mailbox::new(self.name.trim(), self.address.trim())
    }
}

/// a message of a folder as a list shows it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailEntry {
    pub path: String,
    /// the sender's name or address
    pub from: String,
    /// the recipients' names or addresses
    pub to: String,
    pub subject: String,
    /// seconds since 1970 (None: no Date that can be read)
    pub date: Option<i64>,
    pub size: u64,
    pub id: String,
}

/// the messages of a folder (*.eml), oldest first (by their Date, then by name)
pub fn folder(st: &Store, dir: &str) -> Vec<MailEntry> {
    let mut v: Vec<MailEntry> = Vec::new();
    for (name, s) in st.list(dir).unwrap_or_default() {
        if s.dir || !name.ends_with(".eml") {
            continue;
        }
        let path = format!("{}/{}", dir, name);
        let Ok(b) = st.read(&path) else { continue };
        let m = Message::parse(b);
        let to: Vec<String> = m.to().iter().chain(m.cc().iter()).map(|x| String::from(x.shown())).collect();
        v.push(MailEntry {
            path,
            from: m.from().map(|x| String::from(x.shown())).unwrap_or_default(),
            to: to.join(", "),
            subject: m.subject(),
            date: m.date(),
            size: s.size,
            id: String::from(m.get("Message-ID").unwrap_or("")),
        });
    }
    v.sort_by(|a, b| a.date.unwrap_or(0).cmp(&b.date.unwrap_or(0)).then(a.path.cmp(&b.path)));
    v
}

/// a message file of a folder
pub fn read_mail(st: &Store, path: &str) -> Option<Message> {
    st.read(path).ok().map(Message::parse)
}

/// a UIDL as a file name: letters, digits, '.', '-' and '_' stay, the rest becomes '_'
pub fn file_of_uid(uid: &str) -> String {
    let s: String = uid.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    format!("{}.eml", s)
}

// ---------------------------------------------------------------- the E-Mail program in the engine

/// a message the E-Mail program sends: the fields as typed
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailDraft {
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub text: String,
    /// the Message-ID answered (a reply), with its References
    pub in_reply_to: String,
    pub references: String,
}

/// what the program does
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailJob {
    Send(MailDraft),
    Fetch,
}

/// the outcome of a send or fetch
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailOutcome {
    /// when it ended (virtual time)
    pub at: Ns,
    pub ok: bool,
    /// what happened, for the status line
    pub msg: Msg,
    /// messages fetched
    pub count: u32,
}

/// the E-Mail program as the window shows it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailClientView {
    /// a send or fetch runs ("send", "fetch")
    pub busy: Option<&'static str>,
    /// jobs waiting after it
    pub waiting: usize,
    pub last_send: Option<MailOutcome>,
    pub last_fetch: Option<MailOutcome>,
}

/// the running job
#[derive(Clone, Debug)]
enum Run {
    Send { dial: Dial, conv: SmtpConv, to: String, raw: Vec<u8> },
    Fetch { dial: Dial, fetch: Fetch },
}

const K_DIAL: u32 = 0;
const K_IDLE: u32 = 1;
/// connect timeout and how long a conversation may stay silent
const DIAL_TIMEOUT: Ns = 30 * SEC;
const IDLE_TIMEOUT: Ns = 120 * SEC;
/// the name of the service on a device
pub const CLIENT_SVC: &str = "e-mail";

/// the service of the E-Mail program
#[derive(Clone, Debug, Default)]
pub struct MailClient {
    queue: VecDeque<MailJob>,
    run: Option<Run>,
    pub last_send: Option<MailOutcome>,
    pub last_fetch: Option<MailOutcome>,
    pub sent: u64,
    pub fetched: u64,
    pub failed: u64,
    seq: u32,
}

impl MailClient {
    pub fn new() -> MailClient {
        MailClient::default()
    }

    /// the view for the window
    pub fn view(&self) -> MailClientView {
        MailClientView {
            busy: self.run.as_ref().map(|r| match r {
                Run::Send { .. } => "send",
                Run::Fetch { .. } => "fetch",
            }),
            waiting: self.queue.len(),
            last_send: self.last_send.clone(),
            last_fetch: self.last_fetch.clone(),
        }
    }

    fn outcome(&mut self, os: &mut OsApi, send: bool, ok: bool, m: Msg, count: u32) {
        let o = MailOutcome { at: os.now(), ok, msg: m.clone(), count };
        if !ok {
            self.failed += 1;
            os.note(Note::Mail(MNote::ClientFailed { what: m.clone() }));
            os.log(4, "e-mail", &m.text());
        }
        if send {
            self.last_send = Some(o);
        } else {
            self.last_fetch = Some(o);
        }
    }

    /// the next job, when none runs
    fn kick(&mut self, os: &mut OsApi) {
        while self.run.is_none() {
            let Some(job) = self.queue.pop_front() else { return };
            match job {
                MailJob::Send(d) => self.begin_send(os, d),
                MailJob::Fetch => self.begin_fetch(os),
            }
        }
    }

    fn account(os: &OsApi) -> Option<Account> {
        os.files().and_then(Account::of)
    }

    fn begin_send(&mut self, os: &mut OsApi, d: MailDraft) {
        let Some(a) = Self::account(os).filter(|a| !a.address.trim().is_empty() && !a.smtp.trim().is_empty()) else {
            return self.outcome(os, true, false, msg!("Nicht gesendet: das Konto nennt keine Adresse oder keinen SMTP-Server."), 0);
        };
        if !imf::addr_ok(a.address.trim()) {
            return self.outcome(os, true, false, msg!("Nicht gesendet: {} ist keine E-Mail-Adresse.", a.address.trim()), 0);
        }
        let to = imf::parse_list(&d.to);
        let cc = imf::parse_list(&d.cc);
        let bad: Vec<&str> = imf::split_list(&d.to).into_iter().chain(imf::split_list(&d.cc)).map(str::trim).filter(|x| !x.is_empty() && imf::parse_mailbox(x).is_none()).collect();
        if let Some(b) = bad.first() {
            return self.outcome(os, true, false, msg!("Nicht gesendet: {} ist keine E-Mail-Adresse.", b), 0);
        }
        if to.is_empty() && cc.is_empty() {
            return self.outcome(os, true, false, msg!("Nicht gesendet: kein Empfänger."), 0);
        }
        self.seq += 1;
        let clock = os.clock();
        let secs = clock.div_euclid(1_000_000_000);
        let r = os.rand();
        let domain = imf::split_addr(a.address.trim()).map(|x| String::from(x.1)).unwrap_or_default();
        let draft = Draft {
            from: a.mailbox(),
            to: to.clone(),
            cc: cc.clone(),
            subject: d.subject.clone(),
            date: secs,
            id: imf::message_id(&format!("{}.{:08X}.{}", secs, r, self.seq), &domain),
            in_reply_to: d.in_reply_to.clone(),
            references: d.references.clone(),
            text: d.text.clone(),
        };
        let raw = draft.to_bytes(false);
        let rcpts: Vec<String> = to.iter().chain(cc.iter()).map(|m| m.addr.clone()).collect();
        let shown: Vec<String> = to.iter().chain(cc.iter()).map(|m| m.addr.clone()).collect();
        let mut conv = SmtpConv::new(&os.hostname(), a.address.trim(), rcpts, raw.clone());
        conv.data8 = Some(draft.to_bytes(true));
        if !a.user.trim().is_empty() {
            conv.auth = Some((String::from(a.user.trim()), a.password.clone()));
        }
        let mut dial = Dial::new(a.smtp.trim(), a.smtp_port, K_DIAL, DIAL_TIMEOUT);
        let ev = dial.start(os);
        self.run = Some(Run::Send { dial, conv, to: shown.join(", "), raw });
        os.timer(K_IDLE, os.now() + IDLE_TIMEOUT);
        if let Some(e) = ev {
            self.dialed(os, e);
        }
    }

    fn begin_fetch(&mut self, os: &mut OsApi) {
        let Some(a) = Self::account(os).filter(|a| !a.pop3.trim().is_empty() && !a.user.trim().is_empty()) else {
            return self.outcome(os, false, false, msg!("Nicht abgerufen: das Konto nennt keinen POP3-Server oder keinen Benutzer."), 0);
        };
        let known: BTreeSet<String> = os.file_read(UIDLS).map(|b| String::from_utf8_lossy(&b).lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
        let fetch = Fetch::new(a.user.trim(), &a.password, a.keep, known);
        let mut dial = Dial::new(a.pop3.trim(), a.pop3_port, K_DIAL, DIAL_TIMEOUT);
        let ev = dial.start(os);
        self.run = Some(Run::Fetch { dial, fetch });
        os.timer(K_IDLE, os.now() + IDLE_TIMEOUT);
        if let Some(e) = ev {
            self.dialed(os, e);
        }
    }

    /// the connection is up or cannot be made
    fn dialed(&mut self, os: &mut OsApi, e: DialEv) {
        match e {
            DialEv::Up(_) => {}
            DialEv::Fail(why) => {
                let (send, host) = match &self.run {
                    Some(Run::Send { dial, .. }) => (true, dial.host.clone()),
                    Some(Run::Fetch { dial, .. }) => (false, dial.host.clone()),
                    None => return,
                };
                self.run = None;
                os.cancel(K_IDLE);
                let m = if send { msg!("Nicht gesendet: {}.", why.msg(&host)) } else { msg!("Nicht abgerufen: {}.", why.msg(&host)) };
                self.outcome(os, send, false, m, 0);
                self.kick(os);
            }
        }
    }

    /// bytes of the server
    fn data(&mut self, os: &mut OsApi, sock: u32) {
        let b = os.tcp_recv(sock, usize::MAX);
        os.timer(K_IDLE, os.now() + IDLE_TIMEOUT);
        let mut end = false;
        match self.run.as_mut() {
            Some(Run::Send { conv, .. }) => {
                for st in conv.on_bytes(&b) {
                    match st {
                        Step::Send(_, bytes) => {
                            os.tcp_send(sock, &bytes);
                        }
                        Step::End => end = true,
                        Step::Got(_) => {}
                    }
                }
            }
            Some(Run::Fetch { fetch, .. }) => {
                for l in fetch.on_bytes(&b) {
                    os.tcp_send(sock, &l);
                }
                end = fetch.done;
            }
            None => {}
        }
        if end {
            self.finish(os);
        }
    }

    /// the job is over (the conversation ended, or the connection broke)
    fn finish(&mut self, os: &mut OsApi) {
        let Some(run) = self.run.take() else { return };
        os.cancel(K_IDLE);
        match run {
            Run::Send { mut dial, conv, to, raw } => {
                dial.close(os);
                let host = dial.host.clone();
                if conv.ok() {
                    self.sent += 1;
                    // the copy in SENT
                    let clock = os.clock();
                    let name = crate::mailbox::unique(clock.div_euclid(1_000_000_000), (clock.rem_euclid(1_000_000_000) / 1000) as u32, self.seq, &os.hostname());
                    let path = format!("{}/{}.eml", SENT, name);
                    let _ = os.store_put(&path, &imf::lf(&raw));
                    os.store_report(&path);
                    let reply = conv.final_reply.as_ref().map(|r| r.line()).unwrap_or_default();
                    let refused: Vec<String> = conv.rcpts.iter().zip(conv.refused.iter()).filter(|(_, r)| r.is_some()).map(|(a, _)| a.clone()).collect();
                    os.note(Note::Mail(MNote::ClientSent { to: to.clone(), server: host.clone() }));
                    os.log(6, "e-mail", &format!("sent to {} via {}: {}", to, host, reply));
                    let m = if refused.is_empty() { msg!("Gesendet an {} über {}.", to, host) } else { msg!("Gesendet über {}, abgelehnt: {}.", host, refused.join(", ")) };
                    self.outcome(os, true, true, m, 0);
                } else {
                    let m = match &conv.fail {
                        Some((Stage::Auth, r)) => msg!("Nicht gesendet: {} lehnt die Anmeldung ab ({}).", host, r.line()),
                        Some((Stage::Rcpt, r)) => msg!("Nicht gesendet: {} nimmt keinen Empfänger an ({}).", host, r.line()),
                        Some((st, r)) => msg!("Nicht gesendet: {} lehnt {} ab ({}).", host, st.word(), r.line()),
                        None => msg!("Nicht gesendet: die Verbindung zu {} brach ab.", host),
                    };
                    self.outcome(os, true, false, m, 0);
                }
            }
            Run::Fetch { mut dial, fetch } => {
                dial.close(os);
                let host = dial.host.clone();
                let n = fetch.got.len() as u32;
                if n > 0 {
                    let mut last = String::new();
                    let mut uids = os.file_read(UIDLS).unwrap_or_default();
                    for (uid, body) in &fetch.got {
                        let path = format!("{}/{}", INBOX, file_of_uid(uid));
                        let _ = os.store_put(&path, &imf::lf(body));
                        uids.extend_from_slice(uid.as_bytes());
                        uids.push(b'\n');
                        last = path;
                    }
                    let _ = os.store_put(UIDLS, &uids);
                    os.store_report(&last);
                    self.fetched += n as u64;
                }
                match &fetch.fail {
                    Some(("PASS", t)) | Some(("USER", t)) => {
                        let m = msg!("Nicht abgerufen: {} lehnt die Anmeldung ab (-ERR {}).", host, t);
                        self.outcome(os, false, false, m, n);
                    }
                    Some((what, t)) => {
                        let m = msg!("Nicht abgerufen: {} lehnt {} ab (-ERR {}).", host, what, t);
                        self.outcome(os, false, false, m, n);
                    }
                    None if !fetch.done => {
                        let m = msg!("Abruf von {} abgebrochen: die Verbindung brach ab ({} Nachrichten geholt).", host, n);
                        self.outcome(os, false, false, m, n);
                    }
                    None => {
                        os.note(Note::Mail(MNote::ClientFetched { n, server: host.clone() }));
                        os.log(6, "e-mail", &format!("fetched {} messages from {}", n, host));
                        let m = match n {
                            0 => msg!("Keine neuen Nachrichten auf {}.", host),
                            1 => msg!("Eine neue Nachricht von {} abgerufen.", host),
                            _ => msg!("{} neue Nachrichten von {} abgerufen.", n, host),
                        };
                        self.outcome(os, false, true, m, n);
                    }
                }
            }
        }
        self.kick(os);
    }

    fn dial_of(&mut self) -> Option<&mut Dial> {
        match self.run.as_mut()? {
            Run::Send { dial, .. } | Run::Fetch { dial, .. } => Some(dial),
        }
    }
}

impl Service for MailClient {
    fn kind(&self) -> &'static str {
        "mail-client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.kick(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Ctl(Ctl::Renew) => self.kick(os),
            SvcEvent::Timer(K_DIAL) => {
                if let Some(e) = self.dial_of().and_then(|d| d.timer(os)) {
                    self.dialed(os, e);
                }
            }
            SvcEvent::Timer(K_IDLE) => {
                // nothing came for too long: the job ends
                if let Some(s) = self.dial_of().and_then(|d| d.sock) {
                    os.tcp_abort(s);
                }
                self.finish(os);
            }
            SvcEvent::TcpData { sock, .. } if self.dial_of().is_some_and(|d| d.sock == Some(sock) && d.up) => self.data(os, sock),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } if self.dial_of().is_some_and(|d| d.sock == Some(sock) && d.up) => {
                let rest = os.tcp_recv(sock, usize::MAX);
                if !rest.is_empty() {
                    // the last reply came with the end
                    match self.run.as_mut() {
                        Some(Run::Send { conv, .. }) => {
                            let _ = conv.on_bytes(&rest);
                        }
                        Some(Run::Fetch { fetch, .. }) => {
                            let _ = fetch.on_bytes(&rest);
                        }
                        None => {}
                    }
                }
                self.finish(os);
            }
            ev => {
                if let Some(e) = self.dial_of().and_then(|d| d.on(&ev, os)) {
                    self.dialed(os, e);
                }
            }
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        // a job that ran is lost with the power, the waiting ones stay
        self.run = None;
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("sent", self.sent as i64), Metric::new("fetched", self.fetched as i64), Metric::new("failed", self.failed as i64), Metric::new("queued", self.queue.len() as i64)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("{} gesendet, {} abgerufen, {} gescheitert", self.sent, self.fetched, self.failed)
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
        let run = match &self.run {
            Some(Run::Send { conv, raw, .. }) => conv.data.len() + conv.data8.as_ref().map_or(0, Vec::len) + raw.len(),
            Some(Run::Fetch { fetch, .. }) => fetch.got.iter().map(|g| g.1.len() + g.0.len()).sum(),
            None => 0,
        };
        512 + run + self.queue.iter().map(|j| if let MailJob::Send(d) = j { d.text.len() + d.subject.len() + d.to.len() + 64 } else { 16 }).sum::<usize>()
    }
}

impl Engine {
    /// Action::MailSend / MailFetch: the job goes to the device's E-Mail program (made on the
    /// first one), which takes it when the jobs before it are done
    pub(crate) fn mail_job(&mut self, dev: u32, job: MailJob) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if !matches!(self.nodes.get(ni), Some(Some(n)) if n.power) {
            return Err(ActErr::Refused(msg!("Das Gerät ist ausgeschaltet.")));
        }
        let have = self.os(ni).and_then(|o| o.svc_id(CLIENT_SVC));
        let id = match have {
            Some(id) => id,
            None => {
                self.add_service(dev, CLIENT_SVC, Box::new(MailClient::new()), true)?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                let s = os.svcs.iter_mut().find(|s| s.name == CLIENT_SVC).ok_or(ActErr::NoIp(dev))?;
                // a program, not a server: not among the device's services
                s.console = true;
                s.id
            }
        };
        let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
        let slot = os.svcs.iter_mut().find(|s| s.id == id).ok_or(ActErr::NoIp(dev))?;
        if !slot.enabled {
            slot.enabled = true;
        }
        let c = slot.svc.as_mut().and_then(|x| x.as_any_mut().downcast_mut::<MailClient>()).ok_or_else(|| ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", CLIENT_SVC)))?;
        c.queue.push_back(job);
        let running = slot.running;
        if running {
            self.svc_ctl(ni, id, Ctl::Renew);
        }
        Ok(())
    }

    /// the E-Mail program of a device (None: it never ran)
    pub fn mail_client(&self, dev: u32) -> Option<MailClientView> {
        let os = self.os(self.node_index(dev)?)?;
        os.svc(CLIENT_SVC)?.svc.as_ref()?.as_any().downcast_ref::<MailClient>().map(MailClient::view)
    }
}
