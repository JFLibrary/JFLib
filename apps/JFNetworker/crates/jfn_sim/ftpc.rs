// ftpc -- the FTP client of the consoles: curl with ftp:// URLs in the Linux shell and "copy
// ftp: ..." at the IOS-like console; a console program (os.rs) that logs in, opens a data
// connection and moves one file or a listing, then ends
//
//   FtpProgram::ios_get(host, file, user, pass, sink) / ios_put(host, file, user, pass, data)
//   Engine::sh_curl_ftp(ni, args)       curl [-v] [-s] [-S] [-u USER:PASS] [-T FILE] [-o FILE |
//                                       -O] [-l | --list-only] [-P ADDR | --ftp-port ADDR]
//                                       [--disable-epsv] [-4 | -6] ftp://[USER:PASS@]HOST[:PORT]/PATH
//
// curl's steps (as curl 8 does them): the greeting, USER and PASS (anonymous with the password
// ftp@example.com when no -u), PWD, one CWD per directory of the path, EPSV (PASV when the
// server refuses it, or EPRT / PORT with -P), TYPE I (TYPE A for a listing), SIZE and RETR, STOR
// for -T, LIST or NLST (-l) for a path that ends in "/", QUIT.  It prints the file or the
// listing (or writes it with -o, -O), with -v every command ("> ...") and reply ("< ...");
// errors as curl's with its exit codes: 67 a refused login, 9 a refused CWD, 78 a missing file,
// 25 a refused upload, 7 no connection, 6 an unknown host, 8 a strange reply.  The IOS-like
// console logs in (the URL's user, "ip ftp username", or anonymous), says TYPE I and PASV, sends
// RETR or STOR with the path and words what it does in its own lines, as for TFTP.  The data of
// a passive connection goes to the address of the control connection (curl's default).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::ftp::{self, Command, Reply, ReplyReader};
use netpkt::IpAddr;

use crate::kernel::{Engine, Ns, MS};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::tcp::SockOpts;
use crate::tftpc::Sink;

/// what the program wants from the server
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Want {
    Get,
    Put(Vec<u8>),
    List { names: bool },
}

/// where it is in the dialogue
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Resolve,
    Connect,
    Greet,
    User,
    Pass,
    Pwd,
    Cwd,
    Epsv,
    Pasv,
    Port,
    Type,
    Size,
    Transfer,
    Quit,
    Done,
}

/// the client
#[derive(Clone, Debug)]
pub struct FtpProgram {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    /// the directories to change into first (curl), then the file
    pub dirs: Vec<String>,
    pub file: String,
    pub want: Want,
    pub sink: Sink,
    /// the IOS-like console's words (else curl's)
    pub ios: bool,
    pub verbose: bool,
    pub silent: bool,
    pub show_error: bool,
    /// active mode: Some(the address to name; None: the control connection's)
    pub active: Option<Option<IpAddr>>,
    pub epsv: bool,
    pub family: Family,
    pub url: String,
    step: Step,
    addr: Option<IpAddr>,
    lookup: Option<u32>,
    ctl: Option<u32>,
    reader: ReplyReader,
    data: Option<u32>,
    listener: Option<u32>,
    got: Vec<u8>,
    data_eof: bool,
    done226: bool,
    cwd: usize,
    started: Ns,
    put_off: usize,
    marks: usize,
    pub exit: Option<i32>,
}

impl FtpProgram {
    fn base(host: &str, file: &str, user: &str, pass: &str, want: Want, sink: Sink, ios: bool) -> FtpProgram {
        FtpProgram {
            host: String::from(host),
            port: ftp::PORT_FTP,
            user: String::from(user),
            pass: String::from(pass),
            dirs: Vec::new(),
            file: String::from(file),
            want,
            sink,
            ios,
            verbose: false,
            silent: false,
            show_error: false,
            active: None,
            epsv: !ios,
            family: Family::Any,
            url: format!("ftp://{}/{}", host, file),
            step: Step::Resolve,
            addr: None,
            lookup: None,
            ctl: None,
            reader: ReplyReader::new(),
            data: None,
            listener: None,
            got: Vec::new(),
            data_eof: false,
            done226: false,
            cwd: 0,
            started: 0,
            put_off: 0,
            marks: 0,
            exit: None,
        }
    }
    /// "copy ftp://host/file ..." of the IOS-like console
    pub fn ios_get(host: &str, file: &str, user: &str, pass: &str, sink: Sink) -> FtpProgram {
        FtpProgram::base(host, file, user, pass, Want::Get, sink, true)
    }
    /// "copy ... ftp://host/file"
    pub fn ios_put(host: &str, file: &str, user: &str, pass: &str, data: Vec<u8>) -> FtpProgram {
        FtpProgram::base(host, file, user, pass, Want::Put(data), Sink::Stdout, true)
    }

    fn v(&self, os: &mut OsApi, s: &str) {
        if self.verbose {
            os.print(s);
        }
    }

    fn end(&mut self, os: &mut OsApi, code: i32) {
        for s in [self.data.take(), self.listener.take(), self.ctl.take()].into_iter().flatten() {
            os.tcp_close(s);
        }
        self.step = Step::Done;
        self.exit = Some(code);
        os.exit(code);
    }

    /// an error: curl's code and message, the IOS-like reason
    fn fail(&mut self, os: &mut OsApi, code: i32, curl: &str, ios: &str) {
        if self.ios {
            os.print(&format!("\n%Error {} {} ({})\n", if matches!(self.want, Want::Put(_)) { "writing" } else { "opening" }, self.url, ios));
        } else if !self.silent || self.show_error {
            os.print(&format!("curl: ({}) {}\n", code, curl));
        }
        self.end(os, code);
    }

    fn send(&mut self, os: &mut OsApi, verb: &str, arg: &str) {
        let Some(c) = self.ctl else { return };
        let cmd = Command::new(verb, arg);
        let line = cmd.to_line();
        self.v(os, &format!("> {}", line.replace("\r\n", "\r\n")));
        os.tcp_send(c, line.as_bytes());
    }

    fn connect(&mut self, os: &mut OsApi, addr: IpAddr) {
        self.addr = Some(addr);
        self.step = Step::Connect;
        let ep = if addr.is_v4() { format!("{}:{}", addr, self.port) } else { format!("[{}]:{}", addr, self.port) };
        self.v(os, &format!("*   Trying {}...\n", ep));
        if self.ios {
            os.print(&match &self.want {
                Want::Put(d) => format!("Sending {} bytes to {} ...\n", d.len(), self.url),
                _ => format!("Fetching {} ...\n", self.url),
            });
        }
        match os.tcp_connect(addr, self.port, SockOpts::default()) {
            Ok(s) => self.ctl = Some(s),
            Err(_) => {
                let (h, p) = (self.host.clone(), self.port);
                self.fail(os, 7, &format!("Failed to connect to {} port {} after 0 ms: Couldn't connect to server", h, p), "Connection refused");
            }
        }
    }

    /// after the login: curl asks where it is, the IOS-like console goes on
    fn logged_in(&mut self, os: &mut OsApi) {
        if self.ios {
            self.step = Step::Type;
            let t = if matches!(self.want, Want::List { .. }) { "A" } else { "I" };
            return self.send(os, "TYPE", t);
        }
        self.step = Step::Pwd;
        self.send(os, "PWD", "");
    }

    fn next_dir(&mut self, os: &mut OsApi) {
        if self.cwd < self.dirs.len() {
            let d = self.dirs[self.cwd].clone();
            self.cwd += 1;
            self.step = Step::Cwd;
            return self.send(os, "CWD", &d);
        }
        self.data_setup(os);
    }

    /// the data connection: passive (EPSV, PASV) or active (EPRT, PORT)
    fn data_setup(&mut self, os: &mut OsApi) {
        if let Some(a) = self.active {
            let local = self.ctl.and_then(|c| os.tcp_info(c)).map(|r| r.local.0);
            let Some(me) = a.or(local) else { return self.fail(os, 30, "FTP: command PORT failed", "No address") };
            let Ok(l) = os.tcp_listen_on(me, 0, 1, SockOpts::default()) else { return self.fail(os, 30, "FTP: command PORT failed", "No free port") };
            let port = os.tcp_info(l).map(|r| r.local.1).unwrap_or(0);
            self.listener = Some(l);
            self.step = Step::Port;
            return self.send(os, "EPRT", &ftp::fmt_eprt(me, port));
        }
        if self.epsv {
            self.step = Step::Epsv;
            return self.send(os, "EPSV", "");
        }
        self.step = Step::Pasv;
        self.send(os, "PASV", "");
    }

    fn data_connect(&mut self, os: &mut OsApi, port: u16) {
        let Some(a) = self.addr else { return };
        self.v(os, &format!("* Connecting to {} ({}) port {}\n", self.host, a, port));
        match os.tcp_connect(a, port, SockOpts::default()) {
            Ok(d) => self.data = Some(d),
            Err(_) => {
                let h = self.host.clone();
                return self.fail(os, 7, &format!("Failed to connect to {} port {} after 0 ms: Couldn't connect to server", h, port), "Connection refused");
            }
        }
        if self.ios {
            // the IOS-like console said TYPE before PASV: the command follows
            return self.command(os);
        }
        self.step = Step::Type;
        let t = if matches!(self.want, Want::List { .. }) { "A" } else { "I" };
        self.send(os, "TYPE", t);
    }

    /// the command of the transfer
    fn command(&mut self, os: &mut OsApi) {
        let f = if self.ios { self.file.clone() } else { String::from(self.file.rsplit('/').next().unwrap_or(&self.file)) };
        match self.want.clone() {
            Want::Get if !self.ios => {
                self.step = Step::Size;
                self.send(os, "SIZE", &f);
            }
            Want::Get => {
                self.step = Step::Transfer;
                self.send(os, "RETR", &f);
            }
            Want::Put(_) => {
                self.step = Step::Transfer;
                self.send(os, "STOR", &f);
            }
            Want::List { names } => {
                self.step = Step::Transfer;
                self.send(os, if names { "NLST" } else { "LIST" }, "");
            }
        }
    }

    /// upload: what the data connection takes
    fn push(&mut self, os: &mut OsApi) {
        let (Some(d), Want::Put(data)) = (self.data, &self.want) else { return };
        while self.put_off < data.len() {
            let n = os.tcp_send(d, &data[self.put_off..]);
            if n == 0 {
                return;
            }
            self.put_off += n;
        }
        if self.put_off == data.len() {
            os.tcp_shutdown(d);
            self.put_off += 1;
        }
    }

    /// the transfer is over when 226 came and (for a download) the data connection ended
    fn maybe_done(&mut self, os: &mut OsApi) {
        let need_eof = !matches!(self.want, Want::Put(_));
        if !self.done226 || (need_eof && !self.data_eof) {
            return;
        }
        if let Some(d) = self.data.take() {
            os.tcp_close(d);
        }
        self.step = Step::Quit;
        self.send(os, "QUIT", "");
    }

    /// the end of a good transfer: the file where it goes
    fn finish(&mut self, os: &mut OsApi) {
        let ms = (os.now() - self.started) / MS;
        let data = core::mem::take(&mut self.got);
        let n = match &self.want {
            Want::Put(d) => d.len(),
            _ => data.len(),
        };
        if !matches!(self.want, Want::Put(_)) {
            match self.sink.clone() {
                Sink::File(path) => {
                    if let Err(e) = os.file_write(&path, &data) {
                        if self.ios {
                            return self.fail(os, 23, "", e.ios_text());
                        }
                        let m = format!("Failure writing output to destination, passed {} returned 0", n);
                        return self.fail(os, 23, &m, e.ios_text());
                    }
                }
                Sink::Stdout if !self.ios => {
                    let text = String::from_utf8_lossy(&data).replace("\r\n", "\n");
                    os.print(&text);
                }
                _ => {}
            }
        }
        if self.ios {
            let rate = if ms == 0 { n as u64 * 1000 } else { n as u64 * 1000 / ms };
            os.print(&format!("\nDone: {} bytes in {} ms ({} bytes/s)\n", n, ms, rate));
            if !matches!(self.want, Want::Put(_)) {
                let text = String::from_utf8_lossy(&data).into_owned();
                let (ni, url) = (os.ni, self.url.clone());
                match self.sink {
                    Sink::Running => os.eng.ios_merge(ni, &text, &url),
                    Sink::Startup => os.eng.ios_set_startup(ni, &text),
                    _ => {}
                }
            }
        }
        self.v(os, &format!("* Connection #0 to host {} left intact\n", self.host));
        self.end(os, 0);
    }

    fn on_reply(&mut self, os: &mut OsApi, r: Reply) {
        if self.verbose {
            let mut s = String::new();
            let n = r.lines.len();
            for (i, l) in r.lines.iter().enumerate() {
                let sep = if i + 1 < n { '-' } else { ' ' };
                if i == 0 || i + 1 == n {
                    s.push_str(&format!("< {}{}{}\r\n", r.code, sep, l));
                } else {
                    s.push_str(&format!("< {}\r\n", l));
                }
            }
            os.print(&s);
        }
        let c = r.code;
        match self.step {
            Step::Greet if c == 220 => {
                self.step = Step::User;
                let u = self.user.clone();
                self.send(os, "USER", &u);
            }
            Step::Greet if r.is_preliminary() => {}
            Step::Greet => self.fail(os, 8, "Weird server reply", "Unexpected reply"),
            Step::User if c == 331 || c == 332 => {
                self.step = Step::Pass;
                let p = self.pass.clone();
                self.send(os, "PASS", &p);
            }
            Step::User | Step::Pass if c == 230 || c == 202 => self.logged_in(os),
            Step::User | Step::Pass => self.fail(os, 67, &format!("Access denied: {}", c), "Permission denied"),
            Step::Pwd => self.next_dir(os),
            Step::Cwd if r.is_ok() => self.next_dir(os),
            Step::Cwd => self.fail(os, 9, "Server denied you to change to the given directory", "No such file or directory"),
            Step::Epsv if c == 229 => match ftp::epsv_port(r.text()) {
                Some(p) => self.data_connect(os, p),
                None => self.fail(os, 13, "Weird EPSV reply", "Unexpected reply"),
            },
            Step::Epsv => {
                // the server does not do EPSV: PASV instead
                self.step = Step::Pasv;
                self.send(os, "PASV", "");
            }
            Step::Pasv if c == 227 => match ftp::host_port(r.text()) {
                Some((_, p)) => self.data_connect(os, p),
                None => self.fail(os, 14, "Weird PASV reply", "Unexpected reply"),
            },
            Step::Pasv => self.fail(os, 13, "Weird PASV reply", "Passive mode refused"),
            Step::Port if r.is_ok() => {
                self.step = Step::Type;
                let t = if matches!(self.want, Want::List { .. }) { "A" } else { "I" };
                self.send(os, "TYPE", t);
            }
            Step::Port => self.fail(os, 30, "FTP: command PORT failed", "Active mode refused"),
            Step::Type if r.is_ok() => {
                if self.ios {
                    // IOS-like: PASV after TYPE
                    self.step = Step::Pasv;
                    return self.send(os, "PASV", "");
                }
                self.command(os);
            }
            Step::Type => self.fail(os, 17, "Couldn't set desired mode", "Type refused"),
            Step::Size => {
                self.step = Step::Transfer;
                let f = String::from(self.file.rsplit('/').next().unwrap_or(&self.file));
                self.send(os, "RETR", &f);
            }
            Step::Transfer if r.is_preliminary() => {
                if matches!(self.want, Want::Put(_)) {
                    self.push(os);
                }
            }
            Step::Transfer if r.is_ok() => {
                self.done226 = true;
                self.maybe_done(os);
            }
            Step::Transfer => match self.want {
                Want::Put(_) => self.fail(os, 25, &format!("Failed FTP upload: {}", c), if c == 552 || c == 452 { "No space left on device" } else { "Permission denied" }),
                Want::Get => self.fail(os, 78, &format!("RETR response: {}", c), "No such file or directory"),
                Want::List { .. } => self.fail(os, 19, &format!("LIST response: {}", c), "No such file or directory"),
            },
            Step::Quit => self.finish(os),
            _ => {}
        }
    }

    fn mark(&mut self, os: &mut OsApi) {
        if !self.ios {
            return;
        }
        let n = self.got.len() / 4096;
        while self.marks < n {
            os.print("#");
            self.marks += 1;
        }
    }
}

impl Service for FtpProgram {
    fn kind(&self) -> &'static str {
        if self.ios { "ftp" } else { "curl" }
    }
    fn start(&mut self, os: &mut OsApi) {
        self.started = os.now();
        match IpAddr::parse(&self.host) {
            Some(a) => self.connect(os, a),
            None => self.lookup = Some(os.resolve_host(&self.host, self.family)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.exit.is_some() {
            return;
        }
        match ev {
            SvcEvent::Resolved(id) if Some(id) == self.lookup => {
                let first = os.lookup(id).and_then(|l| l.result.as_ref()).and_then(|r| r.as_ref().ok()).and_then(|a| a.first());
                match first {
                    Some(a) => self.connect(os, a),
                    None => {
                        let h = self.host.clone();
                        self.fail(os, 6, &format!("Could not resolve host: {}", h), "Unknown host");
                    }
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.ctl => {
                self.step = Step::Greet;
                let a = self.addr.map(|a| format!("{}", a)).unwrap_or_default();
                self.v(os, &format!("* Connected to {} ({}) port {}\n", self.host, a, self.port));
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.data => {
                if self.step == Step::Transfer && matches!(self.want, Want::Put(_)) {
                    self.push(os);
                }
            }
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                if let Some(d) = os.tcp_accept(listener) {
                    self.data = Some(d);
                    os.tcp_close(listener);
                    self.listener = None;
                    if self.step == Step::Transfer && matches!(self.want, Want::Put(_)) {
                        self.push(os);
                    }
                }
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.ctl => {
                let b = os.tcp_recv(sock, usize::MAX);
                for r in self.reader.push(&b) {
                    self.on_reply(os, r);
                    if self.exit.is_some() {
                        return;
                    }
                }
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.data => {
                let b = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&b);
                self.mark(os);
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.data => self.push(os),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } if Some(sock) == self.data => {
                let rest = os.tcp_recv(sock, usize::MAX);
                self.got.extend_from_slice(&rest);
                self.data_eof = true;
                if let Some(d) = self.data.take() {
                    os.tcp_close(d);
                }
                self.maybe_done(os);
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.ctl => {
                self.ctl = None;
                os.tcp_close(sock);
                if self.step == Step::Quit {
                    return self.finish(os);
                }
                let h = self.host.clone();
                if self.step == Step::Connect {
                    let p = self.port;
                    let ms = (os.now() - self.started) / MS;
                    return self.fail(os, 7, &format!("Failed to connect to {} port {} after {} ms: Couldn't connect to server", h, p, ms), end.errno().unwrap_or("Connection refused"));
                }
                self.fail(os, 56, "Recv failure: Connection reset by peer", "Connection closed");
            }
            SvcEvent::TcpEof { sock, .. } if Some(sock) == self.ctl => {
                if self.step == Step::Quit {
                    return self.finish(os);
                }
                self.fail(os, 56, "Recv failure: Connection reset by peer", "Connection closed");
            }
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("bytes", self.got.len() as i64)]
    }
    fn status(&self) -> String {
        format!("ftp {}", self.url)
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

/// a message of curl before it starts: on the console, the first line as the error
fn refuse(e: &mut Engine, ni: usize, text: &str) -> Result<(), String> {
    e.out(ni, text);
    Err(String::from(text.lines().next().unwrap_or("")))
}

impl Engine {
    /// curl with an ftp:// URL
    pub(crate) fn sh_curl_ftp(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut p = FtpProgram::base("", "", "anonymous", "ftp@example.com", Want::Get, Sink::Stdout, false);
        let mut url: Option<String> = None;
        let (mut upload, mut output, mut remote_name, mut list) = (None::<String>, None::<String>, false, false);
        let mut auth: Option<String> = None;
        let mut i = 0;
        let usage = |s: &mut Engine, m: &str| refuse(s, ni, &format!("curl: {}\ncurl: try 'curl --help' for more information\n", m));
        while i < args.len() {
            let a = args[i];
            let val = |i: usize| args.get(i + 1).map(|s| String::from(*s));
            match a {
                "-v" | "--verbose" => p.verbose = true,
                "-s" | "--silent" => p.silent = true,
                "-S" | "--show-error" => p.show_error = true,
                "-sS" | "-Ss" => {
                    p.silent = true;
                    p.show_error = true;
                }
                "-l" | "--list-only" => list = true,
                "-O" | "--remote-name" => remote_name = true,
                "--ftp-pasv" => p.active = None,
                "--disable-epsv" => p.epsv = false,
                "-4" | "--ipv4" => p.family = Family::V4,
                "-6" | "--ipv6" => p.family = Family::V6,
                "-u" | "--user" | "-T" | "--upload-file" | "-o" | "--output" | "-P" | "--ftp-port" => {
                    let Some(v) = val(i) else { return usage(self, &format!("option {}: requires parameter", a)) };
                    match a {
                        "-u" | "--user" => auth = Some(v),
                        "-T" | "--upload-file" => upload = Some(v),
                        "-o" | "--output" => output = Some(v),
                        _ => p.active = Some(if v == "-" { None } else { IpAddr::parse(&v) }),
                    }
                    i += 1;
                }
                o if o.starts_with('-') => return usage(self, &format!("option {}: is unknown", o)),
                u => url = Some(String::from(u)),
            }
            i += 1;
        }
        let Some(u) = url else { return usage(self, "no URL specified") };
        let Some(rest) = u.strip_prefix("ftp://").or_else(|| u.strip_prefix("FTP://")) else { return usage(self, "no URL specified") };
        let (hostpart, path) = match rest.find('/') {
            Some(k) => (&rest[..k], &rest[k + 1..]),
            None => (rest, ""),
        };
        let (creds, hostport) = match hostpart.rfind('@') {
            Some(k) => (Some(&hostpart[..k]), &hostpart[k + 1..]),
            None => (None, hostpart),
        };
        if let Some(c) = auth.as_deref().or(creds) {
            let (us, pw) = c.split_once(':').unwrap_or((c, ""));
            p.user = String::from(us);
            p.pass = String::from(pw);
        }
        let (host, port) = match hostport.rsplit_once(':').filter(|(h, _)| !h.contains(':') || h.starts_with('[')) {
            Some((h, pt)) => match pt.parse::<u16>() {
                Ok(n) => (h, n),
                Err(_) => return refuse(self, ni, "curl: (3) URL rejected: Port number was not a decimal number between 0 and 65535\n"),
            },
            None => (hostport, ftp::PORT_FTP),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return refuse(self, ni, "curl: (3) URL rejected: No host part in the URL\n");
        }
        p.host = String::from(host);
        p.port = port;
        p.url = u.clone();
        let mut parts: Vec<String> = path.split('/').map(String::from).collect();
        let last = parts.pop().unwrap_or_default();
        p.dirs = parts.into_iter().filter(|d| !d.is_empty()).collect();
        if let Some(local) = upload {
            let data = match self.store(ni).map(|s| s.read(&local).map(Vec::from)) {
                Some(Ok(d)) => d,
                _ => return refuse(self, ni, &format!("curl: Can't open '{}'!\ncurl: try 'curl --help' for more information\ncurl: (26) Failed to open/read local data from file/application\n", local)),
            };
            p.file = if last.is_empty() { String::from(local.rsplit('/').next().unwrap_or(&local)) } else { last };
            p.want = Want::Put(data);
        } else if last.is_empty() {
            p.want = Want::List { names: list };
        } else {
            p.file = last.clone();
            p.want = Want::Get;
            let out = if remote_name { Some(last.clone()) } else { output.filter(|o| o != "-") };
            if let Some(o) = out {
                let ok = self.store(ni).is_some_and(|s| s.norm(&o).is_ok_and(|q| s.is_dir(&s.parent(&q)) && !s.is_dir(&q)));
                if !ok {
                    return refuse(self, ni, &format!("curl: (23) Failed to open the file {}: No such file or directory\n", o));
                }
                p.sink = Sink::File(o);
            }
        }
        self.run_program(ni, "curl", Box::new(p), false)
    }
}
