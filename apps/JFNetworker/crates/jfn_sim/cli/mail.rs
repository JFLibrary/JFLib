// mail -- mail at the Linux shell: curl with smtp:// and pop3:// URLs (the steps, -v trace and
// messages of curl 8) and mail, one-shot: a message from a pipe goes out, -H lists the inbox
//
//   Engine::sh_curl_mail(ni, args)   curl [-v] [-s] [-S] [-u USER:PASS] [--mail-from ADDR]
//                                    [--mail-rcpt ADDR].. -T FILE smtp://HOST[:PORT][/DOMAIN]
//                                    curl [-v] [-s] [-S] [-u USER:PASS] [-X CMD] [-l] [-o FILE]
//                                    pop3://[USER:PASS@]HOST[:PORT][/N]
//   Engine::sh_mail(ni, args, text)  mail [-s SUBJECT] [-c ADDR] [-r FROM] ADDR..: the text a
//                                    pipe gives (echo TEXT | mail ...; without one the message
//                                    has no text); mail -H (or mail alone): the headers of the
//                                    E-Mail program's inbox
//
// curl smtp://: the greeting, EHLO with the URL's path or the device's name, AUTH when -u is
// given and the server offers it, MAIL FROM (--mail-from, else the null path), RCPT TO for each
// --mail-rcpt, DATA with the file of -T (its line ends become CRLF, as with --crlf), QUIT; it
// prints nothing unless something fails or -v shows each command ("> ...") and reply ("< ...").
// Its errors are curl's with their exit codes: 6 an unknown host, 7 no connection, 8 a strange
// reply, 9 EHLO refused, 67 a refused login, 55 a refused MAIL, RCPT or DATA.  curl pop3://:
// the greeting, CAPA, USER and PASS, then LIST (no number in the URL, or -l), RETR N or the
// command of -X (with the number), QUIT; the body of a multi-line answer goes to the console
// or the file of -o; 67 a refused login, 8 a refused command.
//
// mail hands the message to the SMTP server of the E-Mail program's account (/root/Mail/account,
// with its login on port 587), else to an smtp-server running on the device itself (127.0.0.1):
// From is -r, the account's address or root@HOSTNAME; the message is the E-Mail program's (UTF-8
// text, quoted-printable or 8bit).  It prints only what failed and keeps no copy.  An
// interactive mail program (reading, answering in the console) is not part of the shell: the
// E-Mail program on the desktop does that.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::imf::{self, Draft, Mailbox};
use netpkt::pop3;
use netpkt::smtp;

use crate::kernel::{Engine, Ns, MS, SEC};
use crate::mailc::{self, Account, Dial, DialErr, DialEv, SmtpConv, Stage, Step};
use crate::mailtrace::MNote;
use crate::os::{ConsoleEv, Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::tcp::TcpEnd;
use crate::trace::Note;

const K_DIAL: u32 = 0;
const DIAL_TIMEOUT: Ns = 30 * SEC;

/// "curl: (6) Could not resolve host: x" and the others of a failed connection
fn dial_fail(e: &DialErr, host: &str, port: u16, ms: u64) -> (i32, String) {
    match e {
        DialErr::Lookup(_) => (6, format!("Could not resolve host: {}", host)),
        DialErr::Connect(TcpEnd::Timeout) | DialErr::Timeout => (28, format!("Failed to connect to {} port {} after {} ms: Timeout was reached", host, port, ms)),
        DialErr::Connect(_) => (7, format!("Failed to connect to {} port {} after {} ms: Couldn't connect to server", host, port, ms)),
    }
}

/// the parts of a mail URL: (user:pass, host, port, path)
fn url_parts<'a>(rest: &'a str, default_port: u16) -> Result<(Option<&'a str>, &'a str, u16, &'a str), String> {
    let (hostpart, path) = match rest.find('/') {
        Some(k) => (&rest[..k], &rest[k + 1..]),
        None => (rest, ""),
    };
    let (creds, hostport) = match hostpart.rfind('@') {
        Some(k) => (Some(&hostpart[..k]), &hostpart[k + 1..]),
        None => (None, hostpart),
    };
    let (host, port) = match hostport.rsplit_once(':').filter(|(h, _)| !h.contains(':') || h.starts_with('[')) {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (h, n),
            Err(_) => return Err(String::from("curl: (3) URL rejected: Port number was not a decimal number between 0 and 65535\n")),
        },
        None => (hostport, default_port),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return Err(String::from("curl: (3) URL rejected: No host part in the URL\n"));
    }
    Ok((creds, host, port, path))
}

// ---------------------------------------------------------------- curl smtp://

/// curl with an smtp:// URL
#[derive(Clone, Debug)]
pub struct SmtpProgram {
    dial: Dial,
    conv: SmtpConv,
    verbose: bool,
    quiet: bool,
    started: Ns,
    exit: Option<i32>,
}

impl SmtpProgram {
    fn v(&self, os: &mut OsApi, s: &str) {
        if self.verbose {
            os.print(s);
        }
    }
    fn end(&mut self, os: &mut OsApi, code: i32, err: Option<String>) {
        if let Some(e) = err
            && !self.quiet
        {
            os.print(&format!("curl: ({}) {}\n", code, e));
        }
        if code == 0 {
            let h = self.dial.host.clone();
            self.v(os, &format!("* Closing connection to {}\n", h));
        }
        self.dial.close(os);
        self.exit = Some(code);
        os.exit(code);
    }
    fn trace(&mut self, os: &mut OsApi) {
        for l in core::mem::take(&mut self.dial.trace) {
            self.v(os, &format!("{}\n", l));
        }
    }
    fn dialed(&mut self, os: &mut OsApi, e: DialEv) {
        self.trace(os);
        match e {
            DialEv::Up(_) => {
                let (h, a, p) = (self.dial.host.clone(), self.dial.addr.map(|x| format!("{}", x)).unwrap_or_default(), self.dial.port);
                self.v(os, &format!("* Connected to {} ({}) port {}\n", h, a, p));
            }
            DialEv::Fail(why) => {
                let ms = (os.now() - self.started) / MS;
                let (c, m) = dial_fail(&why, &self.dial.host, self.dial.port, ms);
                self.end(os, c, Some(m));
            }
        }
    }
    fn data(&mut self, os: &mut OsApi, sock: u32) {
        let b = os.tcp_recv(sock, usize::MAX);
        let mut done = false;
        for st in self.conv.on_bytes(&b) {
            match st {
                Step::Got(r) => {
                    if self.verbose {
                        let n = r.lines.len();
                        let mut s = String::new();
                        for (i, l) in r.lines.iter().enumerate() {
                            s.push_str(&format!("< {}{}{}\n", r.code, if i + 1 < n { '-' } else { ' ' }, l));
                        }
                        os.print(&s);
                    }
                }
                Step::Send(lines, bytes) => {
                    for l in &lines {
                        self.v(os, &format!("> {}\n", l));
                    }
                    os.tcp_send(sock, &bytes);
                }
                Step::End => done = true,
            }
        }
        if done {
            self.finish(os);
        }
    }
    /// curl's code for the way the transaction ended
    fn finish(&mut self, os: &mut OsApi) {
        if self.conv.ok() {
            return self.end(os, 0, None);
        }
        let (code, m) = match &self.conv.fail {
            Some((Stage::Greet, r)) => (8, format!("Got unexpected smtp-server response: {}", r.code)),
            Some((Stage::Ehlo | Stage::Helo, r)) => (9, format!("Remote access denied: {}", r.code)),
            Some((Stage::Auth, r)) => (67, format!("Authentication failed: {}", r.code)),
            Some((Stage::Mail, r)) => (55, format!("MAIL failed: {}", r.code)),
            Some((Stage::Rcpt, r)) => (55, format!("RCPT failed: {}", r.code)),
            Some((Stage::Data, r)) => (55, format!("DATA failed: {}", r.code)),
            Some((_, _)) => (8, String::from("Weird server reply")),
            None => (56, String::from("Recv failure: Connection reset by peer")),
        };
        self.end(os, code, Some(m));
    }
}

impl Service for SmtpProgram {
    fn kind(&self) -> &'static str {
        "curl"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.started = os.now();
        if let Some(e) = self.dial.start(os) {
            self.dialed(os, e);
        } else {
            self.trace(os);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.exit.is_some() {
            return;
        }
        let up = self.dial.up;
        match ev {
            SvcEvent::Timer(K_DIAL) => {
                if let Some(e) = self.dial.timer(os) {
                    self.dialed(os, e);
                } else {
                    self.trace(os);
                }
            }
            SvcEvent::TcpData { sock, .. } if up && Some(sock) == self.dial.sock => self.data(os, sock),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } if up && Some(sock) == self.dial.sock => {
                let rest = os.tcp_recv(sock, usize::MAX);
                if !rest.is_empty() {
                    let _ = self.conv.on_bytes(&rest);
                }
                self.finish(os);
            }
            SvcEvent::Console(ConsoleEv::Eof) | SvcEvent::Console(ConsoleEv::Line(_)) => {}
            ev => {
                if let Some(e) = self.dial.on(&ev, os) {
                    self.dialed(os, e);
                } else {
                    self.trace(os);
                }
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("accepted", self.conv.accepted as i64)]
    }
    fn status(&self) -> String {
        format!("smtp {}", self.dial.host)
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
}

// ---------------------------------------------------------------- curl pop3://

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PStep {
    Greet,
    Capa,
    User,
    Pass,
    Cmd,
    Quit,
}

/// curl with a pop3:// URL
#[derive(Clone, Debug)]
pub struct Pop3Program {
    dial: Dial,
    user: Option<(String, String)>,
    /// the command after the login ("LIST", "RETR 1", "UIDL", "DELE 2")
    cmd: String,
    out: Option<String>,
    verbose: bool,
    quiet: bool,
    reader: pop3::Reader,
    step: PStep,
    got: Option<Vec<u8>>,
    /// a refused command: curl's code and message
    fail: Option<(i32, String)>,
    started: Ns,
    exit: Option<i32>,
}

impl Pop3Program {
    fn v(&self, os: &mut OsApi, s: &str) {
        if self.verbose {
            os.print(s);
        }
    }
    fn send(&mut self, os: &mut OsApi, sock: u32, line: &str, step: PStep) {
        let c = pop3::Command::parse(line).unwrap_or_default();
        self.reader.expect(pop3::multi_line(&c));
        self.v(os, &format!("> {}\n", line));
        os.tcp_send(sock, format!("{}\r\n", line).as_bytes());
        self.step = step;
    }
    fn end(&mut self, os: &mut OsApi, code: i32, err: Option<String>) {
        if let Some(e) = err
            && !self.quiet
        {
            os.print(&format!("curl: ({}) {}\n", code, e));
        }
        self.dial.close(os);
        self.exit = Some(code);
        os.exit(code);
    }
    fn trace(&mut self, os: &mut OsApi) {
        for l in core::mem::take(&mut self.dial.trace) {
            self.v(os, &format!("{}\n", l));
        }
    }
    fn dialed(&mut self, os: &mut OsApi, e: DialEv) {
        self.trace(os);
        match e {
            DialEv::Up(_) => {
                let (h, a, p) = (self.dial.host.clone(), self.dial.addr.map(|x| format!("{}", x)).unwrap_or_default(), self.dial.port);
                self.v(os, &format!("* Connected to {} ({}) port {}\n", h, a, p));
            }
            DialEv::Fail(why) => {
                let ms = (os.now() - self.started) / MS;
                let (c, m) = dial_fail(&why, &self.dial.host, self.dial.port, ms);
                self.end(os, c, Some(m));
            }
        }
    }
    fn data(&mut self, os: &mut OsApi, sock: u32) {
        let b = os.tcp_recv(sock, usize::MAX);
        for r in self.reader.push(&b) {
            if self.verbose {
                os.print(&format!("< {}\n", r.status_line()));
                // the capabilities line by line (a message's lines go to the output instead)
                if self.step == PStep::Capa
                    && let Some(body) = &r.body
                {
                    let lines: String = String::from_utf8_lossy(body).lines().map(|l| format!("< {}\n", l)).collect();
                    os.print(&lines);
                }
            }
            match self.step {
                PStep::Greet if r.ok => self.send(os, sock, "CAPA", PStep::Capa),
                PStep::Greet => {
                    self.fail = Some((8, String::from("Got unexpected pop3-server response")));
                    self.send(os, sock, "QUIT", PStep::Quit);
                }
                PStep::Capa => match self.user.clone() {
                    Some((u, _)) => self.send(os, sock, &format!("USER {}", u), PStep::User),
                    None => {
                        let c = self.cmd.clone();
                        self.send(os, sock, &c, PStep::Cmd);
                    }
                },
                PStep::User if r.ok => {
                    let p = self.user.clone().map(|x| x.1).unwrap_or_default();
                    self.send(os, sock, &format!("PASS {}", p), PStep::Pass);
                }
                PStep::User | PStep::Pass if !r.ok => {
                    self.fail = Some((67, String::from("Access denied. -")));
                    self.send(os, sock, "QUIT", PStep::Quit);
                }
                PStep::Pass => {
                    let c = self.cmd.clone();
                    self.send(os, sock, &c, PStep::Cmd);
                }
                PStep::Cmd => {
                    if r.ok {
                        self.got = Some(r.body.clone().unwrap_or_default());
                    } else {
                        self.fail = Some((8, String::from("Weird server reply")));
                    }
                    self.send(os, sock, "QUIT", PStep::Quit);
                }
                PStep::Quit => return self.finish(os),
                _ => {}
            }
        }
    }
    fn finish(&mut self, os: &mut OsApi) {
        if let Some((c, m)) = self.fail.clone() {
            return self.end(os, c, Some(m));
        }
        let body = self.got.take().unwrap_or_default();
        if let Some(path) = self.out.clone() {
            if let Err(e) = os.file_write(&path, &imf::lf(&body)) {
                let m = format!("Failure writing output to destination, passed {} returned 0", body.len());
                let _ = e;
                return self.end(os, 23, Some(m));
            }
        } else if !body.is_empty() {
            os.print(&String::from_utf8_lossy(&imf::lf(&body)));
        }
        let h = self.dial.host.clone();
        self.v(os, &format!("* Closing connection to {}\n", h));
        self.end(os, 0, None);
    }
}

impl Service for Pop3Program {
    fn kind(&self) -> &'static str {
        "curl"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.started = os.now();
        if let Some(e) = self.dial.start(os) {
            self.dialed(os, e);
        } else {
            self.trace(os);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.exit.is_some() {
            return;
        }
        let up = self.dial.up;
        match ev {
            SvcEvent::Timer(K_DIAL) => {
                if let Some(e) = self.dial.timer(os) {
                    self.dialed(os, e);
                } else {
                    self.trace(os);
                }
            }
            SvcEvent::TcpData { sock, .. } if up && Some(sock) == self.dial.sock => self.data(os, sock),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } if up && Some(sock) == self.dial.sock => {
                let _ = os.tcp_recv(sock, usize::MAX);
                if self.step == PStep::Quit {
                    return self.finish(os);
                }
                self.end(os, 56, Some(String::from("Recv failure: Connection reset by peer")));
            }
            SvcEvent::Console(_) => {}
            ev => {
                if let Some(e) = self.dial.on(&ev, os) {
                    self.dialed(os, e);
                } else {
                    self.trace(os);
                }
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("bytes", self.got.as_ref().map_or(0, |g| g.len()) as i64)]
    }
    fn status(&self) -> String {
        format!("pop3 {}", self.dial.host)
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
}

// ---------------------------------------------------------------- mail

/// mail: one message to the SMTP server of the account or the device itself
#[derive(Clone, Debug)]
pub struct MailProgram {
    dial: Dial,
    conv: SmtpConv,
    to: String,
    exit: Option<i32>,
}

impl MailProgram {
    fn end(&mut self, os: &mut OsApi, code: i32, err: Option<String>) {
        if let Some(e) = err {
            os.print(&format!("mail: {}\n", e));
        }
        self.dial.close(os);
        self.exit = Some(code);
        os.exit(code);
    }
    fn finish(&mut self, os: &mut OsApi) {
        let h = self.dial.host.clone();
        if self.conv.ok() {
            let to = self.to.clone();
            os.note(Note::Mail(MNote::ClientSent { to, server: h }));
            return self.end(os, 0, None);
        }
        let m = match &self.conv.fail {
            Some((st, r)) => format!("{} refused {}: {}", h, st.word(), r.line()),
            None => format!("the connection to {} broke off", h),
        };
        self.end(os, 1, Some(m));
    }
}

impl Service for MailProgram {
    fn kind(&self) -> &'static str {
        "mail"
    }
    fn start(&mut self, os: &mut OsApi) {
        if let Some(DialEv::Fail(e)) = self.dial.start(os) {
            let h = self.dial.host.clone();
            self.end(os, 1, Some(format!("cannot reach the mail server {}: {}", h, dial_words(&e))));
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.exit.is_some() {
            return;
        }
        let up = self.dial.up;
        let r = match ev {
            SvcEvent::Timer(K_DIAL) => self.dial.timer(os),
            SvcEvent::TcpData { sock, .. } if up && Some(sock) == self.dial.sock => {
                let b = os.tcp_recv(sock, usize::MAX);
                let mut done = false;
                for st in self.conv.on_bytes(&b) {
                    match st {
                        Step::Send(_, bytes) => {
                            os.tcp_send(sock, &bytes);
                        }
                        Step::End => done = true,
                        Step::Got(_) => {}
                    }
                }
                if done {
                    self.finish(os);
                }
                None
            }
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } if up && Some(sock) == self.dial.sock => {
                let rest = os.tcp_recv(sock, usize::MAX);
                if !rest.is_empty() {
                    let _ = self.conv.on_bytes(&rest);
                }
                self.finish(os);
                None
            }
            SvcEvent::Console(_) => None,
            ev => self.dial.on(&ev, os),
        };
        if let Some(DialEv::Fail(e)) = r {
            let h = self.dial.host.clone();
            self.end(os, 1, Some(format!("cannot reach the mail server {}: {}", h, dial_words(&e))));
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("accepted", self.conv.accepted as i64)]
    }
    fn status(&self) -> String {
        format!("mail {}", self.to)
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
}

/// a failed connection in mail's words
fn dial_words(e: &DialErr) -> &'static str {
    match e {
        DialErr::Lookup(_) => "the name is unknown",
        DialErr::Connect(e) => e.errno().unwrap_or("connection refused"),
        DialErr::Timeout => "no answer",
    }
}

/// a message of curl before it starts: on the console, the first line as the error
fn refuse(e: &mut Engine, ni: usize, text: &str) -> Result<(), String> {
    e.out(ni, text);
    Err(String::from(text.lines().next().unwrap_or("")))
}

impl Engine {
    /// curl with an smtp:// or pop3:// URL (cli/tools.rs sends them here)
    pub(crate) fn sh_curl_mail(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let usage = |s: &mut Engine, m: &str| refuse(s, ni, &format!("curl: {}\ncurl: try 'curl --help' for more information\n", m));
        let (mut verbose, mut silent, mut show_err, mut list) = (false, false, false, false);
        let (mut auth, mut from, mut upload, mut output, mut custom) = (None::<String>, None::<String>, None::<String>, None::<String>, None::<String>);
        let mut rcpts: Vec<String> = Vec::new();
        let mut url: Option<String> = None;
        let mut family = Family::Any;
        let mut i = 0;
        while i < args.len() {
            let a = args[i];
            let val = |i: usize| args.get(i + 1).map(|s| String::from(*s));
            match a {
                "-v" | "--verbose" => verbose = true,
                "-s" | "--silent" => silent = true,
                "-S" | "--show-error" => show_err = true,
                "-sS" | "-Ss" => {
                    silent = true;
                    show_err = true;
                }
                "-l" | "--list-only" => list = true,
                "-4" | "--ipv4" => family = Family::V4,
                "-6" | "--ipv6" => family = Family::V6,
                "--crlf" => {}
                "-u" | "--user" | "--mail-from" | "--mail-rcpt" | "-T" | "--upload-file" | "-o" | "--output" | "-X" | "--request" => {
                    let Some(v) = val(i) else { return usage(self, &format!("option {}: requires parameter", a)) };
                    match a {
                        "-u" | "--user" => auth = Some(v),
                        "--mail-from" => from = Some(v),
                        "--mail-rcpt" => rcpts.push(v),
                        "-T" | "--upload-file" => upload = Some(v),
                        "-o" | "--output" => output = Some(v),
                        _ => custom = Some(v),
                    }
                    i += 1;
                }
                o if o.starts_with('-') => return usage(self, &format!("option {}: is unknown", o)),
                u => url = Some(String::from(u)),
            }
            i += 1;
        }
        let Some(u) = url else { return usage(self, "no URL specified") };
        let lower = u.to_ascii_lowercase();
        let smtp_url = lower.starts_with("smtp://");
        let Some(rest) = u.get(7..).filter(|_| smtp_url || lower.starts_with("pop3://")) else { return usage(self, "no URL specified") };
        let (creds, host, port, path) = match url_parts(rest, if smtp_url { smtp::PORT_SMTP } else { pop3::PORT_POP3 }) {
            Ok(x) => x,
            Err(m) => return refuse(self, ni, &m),
        };
        let login = auth.as_deref().or(creds).map(|c| {
            let (us, pw) = c.split_once(':').unwrap_or((c, ""));
            (String::from(us), String::from(pw))
        });
        let mut dial = Dial::new(host, port, K_DIAL, DIAL_TIMEOUT);
        dial.family = family;
        let quiet = silent && !show_err;
        if smtp_url {
            let Some(file) = upload else { return usage(self, "smtp:// needs the message as -T FILE") };
            if rcpts.is_empty() {
                return refuse(self, ni, "curl: (55) RCPT failed: no recipient given (--mail-rcpt)\n");
            }
            let data = match self.store(ni).map(|s| s.read(&file).map(Vec::from)) {
                Some(Ok(d)) => d,
                _ => return refuse(self, ni, &format!("curl: Can't open '{}'!\ncurl: try 'curl --help' for more information\ncurl: (26) Failed to open/read local data from file/application\n", file)),
            };
            let helo = if path.is_empty() { self.host(ni) } else { String::from(path.trim_end_matches('/')) };
            let mut conv = SmtpConv::new(&helo, from.as_deref().unwrap_or(""), rcpts, imf::crlf(&data));
            conv.auth = login;
            let p = SmtpProgram { dial, conv, verbose, quiet, started: 0, exit: None };
            return self.run_program(ni, "curl", Box::new(p), false);
        }
        let id = path.trim_matches('/');
        let cmd = match custom {
            Some(c) if id.is_empty() => c,
            Some(c) => format!("{} {}", c, id),
            None if id.is_empty() || list => if id.is_empty() { String::from("LIST") } else { format!("LIST {}", id) },
            None => format!("RETR {}", id),
        };
        if let Some(o) = &output {
            let ok = self.store(ni).is_some_and(|s| s.norm(o).is_ok_and(|q| s.is_dir(&s.parent(&q)) && !s.is_dir(&q)));
            if !ok {
                return refuse(self, ni, &format!("curl: (23) Failed to open the file {}: No such file or directory\n", o));
            }
        }
        let p = Pop3Program { dial, user: login, cmd, out: output.filter(|o| o != "-"), verbose, quiet, reader: pop3::Reader::new(), step: PStep::Greet, got: None, fail: None, started: 0, exit: None };
        self.run_program(ni, "curl", Box::new(p), false)
    }

    /// mail: -H lists the inbox, else one message goes out (its text from a pipe)
    pub(crate) fn sh_mail(&mut self, ni: usize, args: &[&str], text: Option<Vec<u8>>) -> Result<(), String> {
        const USAGE: &str = "Usage: mail [-s SUBJECT] [-c ADDRESS] [-r FROM] ADDRESS...\n       mail -H\n  Sends one message: its text comes from a pipe (echo TEXT | mail -s SUBJECT ADDRESS).\n  -H lists the messages the E-Mail program fetched.\n";
        let (mut subject, mut cc, mut from) = (String::new(), String::new(), None::<String>);
        let mut to: Vec<String> = Vec::new();
        let mut headers = false;
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-H" => headers = true,
                "-h" | "--help" => {
                    self.out(ni, USAGE);
                    return Ok(());
                }
                "-s" | "-c" | "-r" => {
                    let Some(v) = args.get(i + 1) else { return self.sh_err(ni, &format!("mail: option requires an argument -- '{}'\n{}", &args[i][1..], USAGE)) };
                    match args[i] {
                        "-s" => subject = String::from(*v),
                        "-c" => cc = String::from(*v),
                        _ => from = Some(String::from(*v)),
                    }
                    i += 1;
                }
                o if o.starts_with('-') => return self.sh_err(ni, &format!("mail: invalid option -- '{}'\n{}", o.trim_start_matches('-'), USAGE)),
                a => to.push(String::from(a)),
            }
            i += 1;
        }
        if headers || to.is_empty() {
            return self.mail_headers(ni);
        }
        for a in to.iter().chain(imf::parse_list(&cc).iter().map(|m| &m.addr)) {
            if !imf::addr_ok(a) {
                return self.sh_err(ni, &format!("mail: {}: not an e-mail address\n", a));
            }
        }
        let acct = self.store(ni).and_then(Account::of);
        let local = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.running && s.kind == "smtp-server").and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<crate::smtpd::SmtpServer>()).map(|x| x.port));
        let host_name = self.host(ni);
        let (server, port, login, sender) = match (&acct, local) {
            (Some(a), _) if !a.smtp.trim().is_empty() => {
                // the login goes along where the server offers AUTH (the submission port)
                let login = (!a.user.trim().is_empty()).then(|| (String::from(a.user.trim()), a.password.clone()));
                (String::from(a.smtp.trim()), a.smtp_port, login, Some(a.mailbox()))
            }
            (_, Some(p)) => (String::from("127.0.0.1"), p, None, None),
            _ => return self.sh_err(ni, "mail: no mail server to hand the message to: set up the E-Mail program's account or run an smtp-server here\n"),
        };
        let from_box = match from {
            Some(f) => imf::parse_mailbox(&f).unwrap_or_else(|| Mailbox::new("", &f)),
            None => sender.filter(|m| imf::addr_ok(&m.addr)).unwrap_or_else(|| Mailbox::new("", &format!("root@{}", host_name.to_ascii_lowercase()))),
        };
        let body = String::from_utf8_lossy(&text.unwrap_or_default()).into_owned();
        if body.is_empty() {
            self.out(ni, "mail: (no text: the message goes out empty)\n");
        }
        let now = self.now;
        let clock = self.os(ni).map(|o| o.clock.read(now)).unwrap_or(0);
        let secs = clock.div_euclid(1_000_000_000);
        let r = match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.rng.next_u32(),
            _ => 0,
        };
        let domain = imf::split_addr(&from_box.addr).map(|x| String::from(x.1)).unwrap_or_default();
        let to_boxes: Vec<Mailbox> = to.iter().map(|a| Mailbox::new("", a)).collect();
        let cc_boxes = imf::parse_list(&cc);
        let d = Draft { from: from_box.clone(), to: to_boxes, cc: cc_boxes.clone(), subject, date: secs, id: imf::message_id(&format!("{}.{:08X}.mail", secs, r), &domain), in_reply_to: String::new(), references: String::new(), text: body };
        let rcpts: Vec<String> = to.iter().cloned().chain(cc_boxes.iter().map(|m| m.addr.clone())).collect();
        let mut conv = SmtpConv::new(&host_name, &from_box.addr, rcpts, d.to_bytes(false));
        conv.data8 = Some(d.to_bytes(true));
        conv.auth = login;
        let p = MailProgram { dial: Dial::new(&server, port, K_DIAL, DIAL_TIMEOUT), conv, to: to.join(", "), exit: None };
        self.run_program(ni, "mail", Box::new(p), false)
    }

    /// mail -H: the inbox's messages as mailx lists them
    fn mail_headers(&mut self, ni: usize) -> Result<(), String> {
        let list = self.store(ni).map(|s| mailc::folder(s, mailc::INBOX)).unwrap_or_default();
        if list.is_empty() {
            let u = "root";
            return self.sh_err(ni, &format!("No mail for {}\n", u));
        }
        let mut s = String::new();
        for (k, m) in list.iter().enumerate() {
            let when = m.date.map(|t| {
                let d = imf::date(t);
                // "Thu, 01 Jan 2026 00:00:05 +0000" -> "Thu Jan  1 00:00"
                let w: Vec<&str> = d.split_whitespace().collect();
                format!("{} {} {:>2} {}", w[0].trim_end_matches(','), w[2], w[1].trim_start_matches('0'), &w[4][..5])
            });
            let lines = self.store(ni).and_then(|st| st.read(&m.path).ok().map(|b| b.iter().filter(|&&c| c == b'\n').count())).unwrap_or(0);
            let from: String = m.from.chars().take(18).collect();
            s.push_str(&format!(" {:>3} {:<18} {:<16} {:>3}/{:<5} {}\n", k + 1, from, when.unwrap_or_default(), lines, m.size, m.subject));
        }
        self.out(ni, &s);
        Ok(())
    }
}
