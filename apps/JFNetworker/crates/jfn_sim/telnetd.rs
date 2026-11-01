// telnetd -- Telnet on a device: the client "telnet HOST [PORT]" of the IOS-like console and
// the Linux shell (a console program that opens a TCP connection, negotiates as a client,
// relays the terminal and the remote's output, and ends with a "%" line on a refusal, a
// timeout or a close), and the servers: the vty lines of IOS-like devices and telnetd on Linux
// hosts, each connection a command-line session of its own (cli/ses.rs)
//
//   TelnetClient              the "telnet" console program (cli/tools.rs-style); remote_echo():
//                             the far end hides what is typed (a password), so the console does
//   Engine::sh_telnet         the Linux shell's "telnet"
//   Engine::ios_telnet        the IOS-like "telnet"
//   TelnetServer              the service of both servers (kind "telnetd"): the hidden service
//                             "vty" of an IOS-like device (TelnetServer::vty, it listens on
//                             TCP 23 while a vty line takes logins: Engine::vty_sync) or a
//                             Linux host's "telnetd" of the project ({"port": 23, "password":
//                             ""}: the login root with that password)
//   vty_opts(lines, n)        what the "line vty" blocks that hold line n say (a later block wins
//                             per setting)
//   Proto                     Telnet | Ssh: what vty_free and the "transport input" of a line ask
//                             about (sshd.rs takes the same lines)
//   build_vty / Engine::ios_vty   "transport input telnet|ssh|all|none" and "access-class ACL
//                             in" in line mode, terminal [no] monitor, show line, clear line
//                             [vty N | console 0 | tty N | N]
//   Engine::sh_who / sh_w / sh_tty    who, w and tty in the shell
//
// The vty lines (IOS-like): every "line vty A B" block adds lines A to B (the default "vty 0 4"
// five).  A connection takes the lowest free line whose "transport input" allows telnet (the
// default is all, as the classic labs expect; "ssh" alone or "none" refuses telnet) and whose
// "access-class" (a standard access list, checked against the peer's address) lets it in, else
// a short refusal and the connection closes.  The server listens only while some vty line can
// log someone in -- "login" with a "password", "login local", "login authentication LIST" or no
// "login" at all -- so a device with the default "line vty 0 4 / login" and no password refuses
// telnet at once (IOS would answer "Password required, but none set" first).  The login is the
// console's gate (cli/aaa.rs) with Requester::Vty(N): the line password, local users, AAA with
// RADIUS; three wrong tries close the connection.  "exec-timeout M [S]" of the line ends a
// session idle that long (10 minutes by default, 0 0 never; not while a command runs).
// "exit" / "logout" ends a session, "clear line" ends another one; "show users" lists them all
// (cli/ses.rs).  Console messages and debug output reach a vty session with "terminal monitor".
//
// The protocol: the server offers WILL SUPPRESS-GO-AHEAD at the start and WILL ECHO while it
// asks for a password (the client stops echoing, the server echoes nothing, then WONT ECHO and
// the client echoes again), as login programs do with a client in line mode; our client sends
// whole lines (the console has its own line editor) and swallows the line end the server sends
// after a hidden line (the console ended it already).  Prompts and output travel as NVT text
// (netpkt::telnet::to_nvt); an Interrupt Process from the client stops the command that runs,
// Are You There gets "[Yes]".  The client's escape is Ctrl+Shift+6 (Ctrl+C in the shell), which
// closes the session and returns to the local command line.
//
// Linux: telnetd takes logins as root (our shells are root's) with the password of its
// configuration ("" lets Enter do); the session is a shell of its own on "pts/N" ("who", "w",
// "tty", ps's TTY column).  Its idle time does not end it.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::cmdtree::{ModeId, Parsed, Tree, F_NO};
use jfn_model::catalog::CliStyle;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::telnet::{self, Item, Options, Reader, OPT_ECHO, OPT_SGA};
use netpkt::{IpAddr, Ipv4};

use crate::aaa::Requester;
use crate::aaatrace::ANote;
use crate::cli::ios::K;
use crate::cli::ses::{Conn, Login, SesKind, W_IDLE, W_INPUT};
use crate::sshd::SshClient;
use crate::cli::{CliJob, LineCfg, Mode};
use crate::cli::print::JobOut;
use crate::kernel::{Engine, Ev, Ns, SEC};
use crate::os::{ConsoleEv, Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

/// the "telnet" console program
#[derive(Clone, Debug)]
pub struct TelnetClient {
    host: String,
    port: u16,
    addr: Option<IpAddr>,
    lookup: Option<u32>,
    sock: Option<u32>,
    connected: bool,
    reader: Reader,
    opts: Options,
    /// bytes typed but not yet sent
    pending: Vec<u8>,
    /// a line went out while the far end echoed: the line end it sends back is the console's
    skip_nl: bool,
}

impl TelnetClient {
    fn new(host: &str, port: u16) -> TelnetClient {
        // the server may echo and suppress go-ahead; we suppress go-ahead too
        let opts = Options::new(&[OPT_SGA], &[OPT_ECHO, OPT_SGA]);
        TelnetClient { host: String::from(host), port, addr: None, lookup: None, sock: None, connected: false, reader: Reader::new(true), opts, pending: Vec::new(), skip_nl: false }
    }

    /// the far end echoes (it asks for a password): what is typed stays hidden
    pub fn remote_echo(&self) -> bool {
        self.connected && self.opts.him(OPT_ECHO)
    }

    fn connect(&mut self, os: &mut OsApi) {
        let Some(addr) = self.addr else {
            return;
        };
        match os.tcp_connect(addr, self.port, SockOpts { nodelay: true, ..SockOpts::default() }) {
            Ok(s) => self.sock = Some(s),
            Err(_) => {
                os.print("% Destination unreachable; gateway or host down\n");
                os.exit(1);
            }
        }
    }

    /// send a line to the remote as network-virtual-terminal text
    fn send_line(&mut self, os: &mut OsApi, line: &str) {
        if let Some(s) = self.sock.filter(|_| self.connected) {
            if self.opts.him(OPT_ECHO) {
                self.skip_nl = true;
            }
            let mut b = telnet::to_nvt(line.as_bytes());
            b.extend_from_slice(b"\r\n");
            let n = os.tcp_send(s, &b);
            if n < b.len() {
                self.pending.extend_from_slice(&b[n..]);
            }
        }
    }

    /// the remote's bytes: answer the negotiations, print the text
    fn receive(&mut self, os: &mut OsApi, data: &[u8]) {
        let mut text = String::new();
        for item in self.reader.push(data) {
            match item {
                Item::Data(d) => {
                    let mut d: &[u8] = &d;
                    if self.skip_nl && !d.is_empty() {
                        self.skip_nl = false;
                        if d[0] == b'\n' {
                            d = &d[1..];
                        }
                    }
                    text.push_str(&String::from_utf8_lossy(d));
                }
                Item::Opt { verb, opt } => {
                    if let Some(ans) = self.opts.received(verb, opt)
                        && let Some(s) = self.sock
                    {
                        os.tcp_send(s, &ans);
                    }
                }
                _ => {}
            }
        }
        if !text.is_empty() {
            os.print(&text);
        }
    }
}

impl Service for TelnetClient {
    fn kind(&self) -> &'static str {
        "telnet"
    }
    fn start(&mut self, os: &mut OsApi) {
        match IpAddr::parse(self.host.trim_start_matches('[').trim_end_matches(']')) {
            Some(a) => {
                self.addr = Some(a);
                os.print(&format!("Trying {} ...\n", self.host));
                self.connect(os);
            }
            None => self.lookup = Some(os.resolve_host(&self.host, Family::Any)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(l) if Some(l) == self.lookup => {
                let all = os.lookup(l).and_then(|x| x.result.clone()).and_then(|r| r.ok()).map(|r| r.sorted).unwrap_or_default();
                match all.first() {
                    Some(&a) => {
                        self.addr = Some(a);
                        os.print(&format!("Trying {} ...\n", a));
                        self.connect(os);
                    }
                    None => {
                        os.print(&format!("% Bad IP address or host name: {}\n", self.host));
                        os.exit(1);
                    }
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.sock => {
                self.connected = true;
                os.print("% Connected. The escape is Ctrl+Shift+6.\n");
                os.note(Note::Aaa(ANote::TelnetOut { to: self.addr.and_then(|a| a.v4()).unwrap_or(Ipv4::UNSPECIFIED), port: self.port }));
                if !self.pending.is_empty() {
                    let d = core::mem::take(&mut self.pending);
                    os.tcp_send(sock, &d);
                }
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.receive(os, &d);
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.sock => {
                if !self.pending.is_empty() {
                    let d = core::mem::take(&mut self.pending);
                    let n = os.tcp_send(sock, &d);
                    self.pending.extend_from_slice(&d[n..]);
                }
            }
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, end: TcpEnd::Closed } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.receive(os, &d);
                os.print("\n% Connection closed by foreign host\n");
                os.tcp_close(sock);
                self.sock = None;
                os.exit(0);
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.sock => {
                self.sock = None;
                let to = self.addr.and_then(|a| a.v4()).unwrap_or(Ipv4::UNSPECIFIED);
                if !self.connected {
                    let why = match end {
                        TcpEnd::Refused => "Connection refused by remote host",
                        TcpEnd::Unreachable => "Destination unreachable; gateway or host down",
                        _ => "Connection timed out; remote host not responding",
                    };
                    os.print(&format!("% {}\n", why));
                    os.note(Note::Aaa(ANote::TelnetRefused { to, port: self.port, why: String::from(why) }));
                } else {
                    os.print("\n% Connection closed by foreign host\n");
                }
                os.exit(1);
            }
            SvcEvent::Console(ConsoleEv::Line(l)) => self.send_line(os, &l),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        format!("telnet {} {}", self.host, self.port)
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

impl Engine {
    /// "telnet HOST [PORT]" in the Linux shell
    pub(crate) fn sh_telnet(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let pos: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        let Some(host) = pos.first() else {
            self.out(ni, "usage: telnet HOST [PORT]\n");
            return Ok(());
        };
        let port = pos.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(telnet::PORT);
        self.run_program(ni, "telnet", Box::new(TelnetClient::new(host, port)), true)
    }

    /// "telnet HOST [PORT]" in the IOS-like EXEC
    pub(crate) fn ios_telnet(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let host = String::from(p.text("host").unwrap_or(""));
        let port = p.num("port").map(|n| n as u16).unwrap_or(telnet::PORT);
        if host.is_empty() {
            return self.ios_err(ni, "% A host is needed\n");
        }
        self.run_program(ni, "telnet", Box::new(TelnetClient::new(&host, port)), true)
    }

    /// the program of a console job is a telnet client whose far end hides what is typed
    pub(crate) fn prog_hides(&self, ni: usize, job: &Option<CliJob>) -> bool {
        let Some(CliJob { out: JobOut::Proc { svc, stdin: true }, .. }) = job else {
            return false;
        };
        if self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == *svc)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<SshClient>()).is_some() {
            return self.ssh_prog_hides(ni, job);
        }
        self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == *svc)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<TelnetClient>()).is_some_and(|t| t.remote_echo())
    }
}

// ---------------------------------------------------------------- the server

/// the telnet server of a device: the vty lines (IOS-like) or telnetd (Linux)
#[derive(Clone, Debug)]
pub struct TelnetServer {
    ios: bool,
    pub port: u16,
    /// root's password (Linux; "": Enter is enough)
    pub password: String,
    listener: Option<u32>,
    accepted: u64,
    refused: u64,
}

impl TelnetServer {
    /// the vty lines' server of an IOS-like device
    pub fn vty() -> TelnetServer {
        TelnetServer { ios: true, port: telnet::PORT, password: String::new(), listener: None, accepted: 0, refused: 0 }
    }
    /// telnetd of a Linux host from its settings: {"port": 23, "password": ""}
    pub fn from_config(c: &Value) -> Result<TelnetServer, Msg> {
        let mut s = TelnetServer { ios: false, port: telnet::PORT, password: String::new(), listener: None, accepted: 0, refused: 0 };
        if let Some(p) = c.get("port") {
            s.port = p.as_u64().filter(|&n| (1..=65535).contains(&n)).ok_or_else(|| msg!("\u{201e}port\u{201c} ist eine Portnummer von 1 bis 65535."))? as u16;
        }
        if let Some(p) = c.get("password") {
            s.password = String::from(p.as_str().ok_or_else(|| msg!("\u{201e}password\u{201c} ist ein Text."))?);
        }
        Ok(s)
    }
    /// listens on its port, or no more
    fn listen(&mut self, os: &mut OsApi, on: bool) {
        match (on, self.listener) {
            (true, None) => self.listener = os.tcp_listen(self.port, 0, SockOpts { nodelay: true, ..SockOpts::default() }).ok(),
            (false, Some(l)) => {
                os.tcp_close(l);
                self.listener = None;
            }
            _ => {}
        }
    }
    /// it listens now
    pub fn listening(&self) -> bool {
        self.listener.is_some()
    }
}

impl Service for TelnetServer {
    fn kind(&self) -> &'static str {
        "telnetd"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.listener = None;
        let on = !self.ios || os.eng.vty_wanted(os.ni);
        self.listen(os, on);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        let (ni, id) = (os.ni, os.id);
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    if os.eng.ses_accept(ni, id, s, self.ios) {
                        self.accepted += 1;
                    } else {
                        self.refused += 1;
                    }
                }
            }
            SvcEvent::TcpData { sock, .. } => os.eng.ses_rx(ni, sock),
            SvcEvent::TcpWritable { sock } => os.eng.ses_flush_sock(ni, sock),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } => os.eng.ses_hangup(ni, sock),
            _ => {}
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.listener = None;
        let (ni, id) = (os.ni, os.id);
        os.eng.ses_server_gone(ni, id);
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("accepted", self.accepted as i64), Metric::new("refused", self.refused as i64)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Port {}: {} Sitzungen angenommen, {} abgewiesen", self.port, self.accepted, self.refused)
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

// ---------------------------------------------------------------- the vty lines

/// the protocol of a session from the network
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Telnet,
    Ssh,
}

/// the first and last line of a "line vty A [B]" block
fn vty_range(name: &str) -> Option<(u16, u16)> {
    let rest = name.strip_prefix("vty ")?;
    let mut w = rest.split_whitespace();
    let a: u16 = w.next()?.parse().ok()?;
    let b: u16 = w.next().and_then(|x| x.parse().ok()).unwrap_or(a);
    Some((a, b.max(a)))
}

/// the key of a line option (a later one replaces an earlier one of the same key)
fn opt_key(o: &str) -> &str {
    if o.starts_with("login") {
        "login"
    } else if o.starts_with("transport input") {
        "transport input"
    } else {
        o.split_whitespace().next().unwrap_or("")
    }
}

/// what the "line vty" blocks that hold line `n` say, a later block after an earlier one (it
/// wins per setting); None: no block holds line n
pub fn vty_opts(lines: &[LineCfg], n: u16) -> Option<Vec<String>> {
    let mut out: Option<Vec<String>> = None;
    for l in lines {
        let Some((a, b)) = vty_range(&l.name) else { continue };
        if n < a || n > b {
            continue;
        }
        let v = out.get_or_insert_with(Vec::new);
        for o in &l.opts {
            let k = opt_key(o);
            v.retain(|x| opt_key(x) != k);
            v.push(o.clone());
        }
    }
    out
}

/// every vty line of a configuration, ascending
fn vty_lines(lines: &[LineCfg]) -> Vec<u16> {
    let mut v: Vec<u16> = lines.iter().filter_map(|l| vty_range(&l.name)).flat_map(|(a, b)| a..=b).collect();
    v.sort();
    v.dedup();
    v
}

/// the line takes the protocol ("transport input": none of it means all)
fn takes(opts: &[String], proto: Proto) -> bool {
    let word = if proto == Proto::Ssh { "ssh" } else { "telnet" };
    match opts.iter().find(|o| o.starts_with("transport input")) {
        Some(o) => o.split_whitespace().skip(2).any(|w| w == word || w == "all"),
        None => true,
    }
}

/// "exec-timeout M S" of a line (None: never; the default is 10 minutes)
fn exec_timeout(opts: &[String]) -> Option<Ns> {
    let Some(o) = opts.iter().find(|o| o.starts_with("exec-timeout")) else {
        return Some(600 * SEC);
    };
    let mut w = o.split_whitespace().skip(1);
    let m: u64 = w.next().and_then(|x| x.parse().ok()).unwrap_or(10);
    let s: u64 = w.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let t = (m * 60 + s) * SEC;
    if t == 0 { None } else { Some(t) }
}

/// the commands of the vty lines in an IOS command tree: user, privileged, line mode
pub(crate) fn build_vty(t: &mut Tree<K>, m: [ModeId; 3]) {
    let [user, prv, line] = m;
    t.add(line, "transport", "Which protocols may open a session on this line", None, F_NO);
    t.add(line, "transport input", "The protocols of sessions from the network", None, 0);
    t.add(line, "transport input telnet", "Telnet", Some(K::TransportInput), 0);
    t.add(line, "transport input telnet ssh", "Telnet and SSH", Some(K::TransportInput), 0);
    t.add(line, "transport input ssh", "SSH only (telnet is refused)", Some(K::TransportInput), 0);
    t.add(line, "transport input ssh telnet", "SSH and Telnet", Some(K::TransportInput), 0);
    t.add(line, "transport input all", "Every protocol (the default)", Some(K::TransportInput), 0);
    t.add(line, "transport input none", "No sessions from the network", Some(K::TransportInput), 0);
    t.add_no(line, "transport input", K::TransportInput);
    t.add(line, "access-class", "Who may open a session on this line", None, F_NO);
    t.add(line, "access-class WORD=acl", "A standard access list (its number or name)", None, 0);
    t.add(line, "access-class WORD=acl in", "Checked against where a session comes from", Some(K::AccessClass), 0);
    t.add_no(line, "access-class WORD=acl in", K::AccessClass);
    for x in [user, prv] {
        t.add(x, "show line", "The console and the vty lines: in use, settings, from where", Some(K::ShowLine), 0);
    }
    t.add(prv, "terminal monitor", "Show console messages and debug output in this session too", Some(K::TermMonitor), 0);
    t.add(prv, "terminal no", "Undo a setting of this session", None, 0);
    t.add(prv, "terminal no monitor", "No console messages in this session", Some(K::TermMonitor), 0);
    t.add(prv, "clear line", "End the session of a line", None, 0);
    t.add(prv, "clear line <0-16>=n", "The line by its number (0 the console, 1 to 16 vty 0 to 15)", Some(K::ClearLine), 0);
    t.add(prv, "clear line vty", "A vty line", None, 0);
    t.add(prv, "clear line vty <0-15>=v", "Its number", Some(K::ClearLine), 0);
    t.add(prv, "clear line console", "The console", None, 0);
    t.add(prv, "clear line console <0-0>=c", "Its number", Some(K::ClearLine), 0);
    t.add(prv, "clear line tty", "A local terminal", None, 0);
    t.add(prv, "clear line tty <1-99>=tt", "Its number", Some(K::ClearLine), 0);
}

impl Engine {
    /// the vty lines' telnet server should listen: some vty line takes telnet and can log
    /// someone in
    pub(crate) fn vty_wanted(&self, ni: usize) -> bool {
        self.vty_wanted_for(ni, Proto::Telnet)
    }

    /// some vty line takes the protocol and can log someone in
    pub(crate) fn vty_wanted_for(&self, ni: usize, proto: Proto) -> bool {
        let Some(c) = self.cdev(ni) else {
            return false;
        };
        if c.style != CliStyle::Ios {
            return false;
        }
        vty_lines(&c.cfg.lines).into_iter().any(|n| self.vty_usable(ni, n, proto))
    }

    // line n takes the protocol and a login can succeed there
    fn vty_usable(&self, ni: usize, n: u16, proto: Proto) -> bool {
        let Some(opts) = self.cdev(ni).and_then(|c| vty_opts(&c.cfg.lines, n)) else {
            return false;
        };
        if !takes(&opts, proto) {
            return false;
        }
        match self.line_login_plan(ni, &format!("vty {}", n)) {
            Some(m) if m == [crate::aaa::Method::Line] => self.line_password(ni, Requester::Vty(n)).is_some(),
            _ => true,
        }
    }

    /// the telnet server of an IOS-like device follows its vty lines: made (hidden) when a line
    /// takes logins the first time, then listening or not (after every configuration line)
    pub(crate) fn vty_sync(&mut self, ni: usize) {
        // the SSH server follows them too (sshd.rs)
        self.ssh_sync(ni);
        if self.cdev(ni).is_none_or(|c| c.style != CliStyle::Ios) || self.os(ni).is_none() {
            return;
        }
        let want = self.vty_wanted(ni);
        let id = self.os(ni).and_then(|o| o.svc_id("vty"));
        match id {
            None if !want => {}
            None => {
                let dev = self.dev_of(ni);
                if self.add_service(dev, "vty", Box::new(TelnetServer::vty()), true).is_ok()
                    && let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.name == "vty"))
                {
                    s.console = true;
                }
            }
            Some(id) => {
                let listening = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<TelnetServer>()).map(|t| t.listening());
                if listening.is_some_and(|l| l != want) {
                    self.svc_call(ni, id, None, |svc, os| {
                        if let Some(t) = svc.as_any_mut().downcast_mut::<TelnetServer>() {
                            t.listen(os, want);
                        }
                    });
                }
            }
        }
    }

    // the lowest free vty line a connection from `from` may take -> Err(why, text for the
    // client)
    pub(crate) fn vty_free(&mut self, ni: usize, from: IpAddr, local: IpAddr, sport: u16, proto: Proto) -> Result<u16, (String, &'static str)> {
        let lines = self.cdev(ni).map(|c| vty_lines(&c.cfg.lines)).unwrap_or_default();
        let usable: Vec<u16> = lines.into_iter().filter(|&n| self.vty_usable(ni, n, proto)).collect();
        let dport = if proto == Proto::Ssh { netpkt::ssh::PORT } else { telnet::PORT };
        let mut allowed = Vec::new();
        let mut class = None;
        for n in usable {
            let acl = self.cdev(ni).and_then(|c| vty_opts(&c.cfg.lines, n)).and_then(|o| o.iter().find_map(|x| x.strip_prefix("access-class ").map(|r| String::from(r.trim_end_matches(" in").trim()))));
            if let Some(name) = acl {
                let flow = netpkt::nat::Flow { proto: 6, src: from.v4().unwrap_or(Ipv4::UNSPECIFIED), dst: local.v4().unwrap_or(Ipv4::UNSPECIFIED), sport, dport, ports: true, icmp: None, flags: 0x02, later: false, mf: false, id: 0 };
                if self.acl_match(ni, &name, &flow) == Some(false) {
                    class = Some(name);
                    continue;
                }
            }
            allowed.push(n);
        }
        let busy = |e: &Engine, n: u16| e.cdev(ni).is_some_and(|c| c.sessions.iter().any(|i| i.kind == SesKind::Telnet && i.no == n));
        if let Some(&n) = allowed.iter().find(|&&n| !busy(self, n)) {
            return Ok(n);
        }
        if allowed.is_empty()
            && let Some(name) = class
        {
            return Err((format!("access-class {}", name), ""));
        }
        Err((String::from("keine freie vty-Leitung"), "% No free vty line, try again later\n"))
    }

    // the session of a telnet connection
    fn ses_by_sock(&self, ni: usize, sock: u32) -> Option<u32> {
        self.cdev(ni)?.sessions.iter().find(|i| i.conn.as_ref().is_some_and(|c| c.sock == sock)).map(|i| i.id)
    }

    /// a connection came to a telnet server (its service `svc`): a session for it, or a short
    /// refusal and the connection closes -> it got a session
    pub(crate) fn ses_accept(&mut self, ni: usize, svc: u32, sock: u32, ios: bool) -> bool {
        let Some(row) = self.tcp_row(ni, sock) else {
            return false;
        };
        let (from, port) = row.remote;
        let no = if ios {
            match self.vty_free(ni, from, row.local.0, port, Proto::Telnet) {
                Ok(n) => Some(n),
                Err((why, text)) => {
                    if !text.is_empty() {
                        let _ = self.tcp_write(ni, svc, sock, &telnet::to_nvt(text.as_bytes()), None);
                    }
                    self.tcp_close(ni, svc, sock, None);
                    self.aaa_note(ni, ANote::VtyRefused { from: from.v4().unwrap_or(Ipv4::UNSPECIFIED), why });
                    return false;
                }
            }
        } else {
            None
        };
        let mut opts = Options::new(&[OPT_ECHO, OPT_SGA], &[OPT_SGA]);
        let mut tx = Vec::new();
        if let Some(b) = opts.enable_us(OPT_SGA) {
            tx.extend_from_slice(&b);
        }
        let conn = Conn { svc, sock, from, port, reader: Reader::new(true), opts, tx, rx: Vec::new(), inq: alloc::collections::VecDeque::new(), intr: false, hide: false, woken: false, ssh: None };
        let Some(s) = self.ses_new(ni, SesKind::Telnet, Some(conn), no) else {
            self.tcp_close(ni, svc, sock, None);
            return false;
        };
        if let Some(n) = no {
            self.aaa_note(ni, ANote::VtyOpen { line: n, from: from.v4().unwrap_or(Ipv4::UNSPECIFIED) });
        }
        self.ses_flush(ni, s);
        self.ses_start_later(ni, s);
        true
    }

    /// the bytes a session's connection did not take yet go on
    pub(crate) fn ses_flush(&mut self, ni: usize, s: u32) {
        let Some((svc, sock, data)) = self.ses_info_mut(ni, s).and_then(|i| i.conn.as_mut()).map(|c| (c.svc, c.sock, core::mem::take(&mut c.tx))) else {
            return;
        };
        if data.is_empty() {
            return;
        }
        let n = self.tcp_write(ni, svc, sock, &data, None);
        if n < data.len()
            && let Some(c) = self.ses_info_mut(ni, s).and_then(|i| i.conn.as_mut())
        {
            let mut rest = Vec::from(&data[n..]);
            rest.extend_from_slice(&c.tx);
            c.tx = rest;
        }
    }

    /// TcpWritable: the session of the socket sends what waits
    pub(crate) fn ses_flush_sock(&mut self, ni: usize, sock: u32) {
        if let Some(s) = self.ses_by_sock(ni, sock) {
            self.ses_flush(ni, s);
        }
    }

    /// bytes came on a telnet connection: negotiations are answered, lines wait for the session
    /// (it takes them at the same moment, outside the server's call)
    pub(crate) fn ses_rx(&mut self, ni: usize, sock: u32) {
        let Some(s) = self.ses_by_sock(ni, sock) else {
            return;
        };
        let Some(svc) = self.ses_info(ni, s).and_then(|i| i.conn.as_ref()).map(|c| c.svc) else {
            return;
        };
        let data = self.tcp_read(ni, svc, sock, usize::MAX, None);
        let Some(info) = self.ses_info_mut(ni, s) else {
            return;
        };
        let ending = info.ending;
        let Some(c) = info.conn.as_mut() else {
            return;
        };
        // an SSH connection: the core takes the bytes (sshd.rs)
        if let Some(x) = c.ssh.as_mut() {
            x.feed(&data, &mut c.tx);
        } else {
            for item in c.reader.push(&data) {
                match item {
                    Item::Data(d) => {
                        for b in d {
                            match b {
                                b'\n' => {
                                    let l = String::from_utf8_lossy(&c.rx).into_owned();
                                    c.rx.clear();
                                    c.inq.push_back(l);
                                }
                                0 | b'\r' => {}
                                8 | 127 => {
                                    c.rx.pop();
                                }
                                b => c.rx.push(b),
                            }
                        }
                    }
                    Item::Opt { verb, opt } => {
                        if let Some(a) = c.opts.received(verb, opt) {
                            c.tx.extend_from_slice(&a);
                        }
                    }
                    Item::Cmd(telnet::IP) => c.intr = true,
                    Item::Cmd(telnet::AYT) => c.tx.extend_from_slice(b"\r\n[Yes]\r\n"),
                    _ => {}
                }
            }
        }
        let wake = !ending && !c.woken && (!c.inq.is_empty() || c.intr || c.ssh.as_ref().is_some_and(|x| !x.events.is_empty()));
        if wake {
            c.woken = true;
        }
        self.ses_flush(ni, s);
        if wake {
            let now = self.now;
            self.schedule(now, Ev::Ses { node: ni, ses: s, epoch: W_INPUT });
        }
    }

    /// the session in place takes the lines its connection brought (an Interrupt Process
    /// first)
    pub(crate) fn ses_input(&mut self, ni: usize) {
        let Some(s) = self.ses_here(ni).map(|i| i.id) else {
            return;
        };
        // what an SSH connection brought: logins, channels, lines (sshd.rs)
        self.ssh_events(ni);
        let ssh = self.ssh_ref(ni).is_some();
        let intr = match self.ses_here_mut(ni).and_then(|i| i.conn.as_mut()) {
            Some(c) => {
                c.woken = false;
                core::mem::take(&mut c.intr)
            }
            None => return,
        };
        if intr {
            self.cli_interrupt_at(ni);
            // (over SSH Ctrl+C at a prompt asks for the prompt again)
            if ssh {
                if let Some(i) = self.ses_here_mut(ni) {
                    i.prompted = false;
                }
                self.cli_state(ni);
            }
        }
        loop {
            if self.ses_here(ni).is_none_or(|i| i.ending) {
                return;
            }
            let Some((line, hide)) = self.ses_here_mut(ni).and_then(|i| i.conn.as_mut()).and_then(|c| c.inq.pop_front().map(|l| (l, c.hide))) else {
                return;
            };
            // a line typed without echo: the server ends it for the client
            if hide {
                self.ses_write(ni, s, "\n");
            }
            self.ses_took_line(ni);
            self.cli_line_at(ni, &line);
        }
    }

    /// cli_state of a telnet session: its prompt into the connection when the session waits
    /// for a line and it is not there yet (WILL ECHO before a password, WONT ECHO after)
    pub(crate) fn ses_prompt_due(&mut self, ni: usize) {
        let Some(c) = self.cdev(ni) else {
            return;
        };
        let busy = c.job.is_some() || c.gate.as_ref().is_some_and(|g| g.waiting);
        let secret = c.gate.as_ref().is_some_and(|g| g.secret()) || c.ses(c.cur).is_some_and(|i| i.login.as_ref().is_some_and(|l| l.user.is_some()));
        let Some(info) = c.ses(c.cur) else {
            return;
        };
        // over SSH: no prompt before the shell; a command ends the session when it is done
        if let Some(x) = info.conn.as_ref().and_then(|k| k.ssh.as_ref()) {
            if x.exec && x.started && x.scp.is_none() && !busy && c.sh.fg.is_none() && !info.ending {
                let s = info.id;
                self.ses_end(ni, s);
                return;
            }
            if !x.started || x.exec {
                return;
            }
        }
        if busy || info.prompted || info.ending {
            return;
        }
        let s = info.id;
        let prompt = self.prompt_of(ni);
        let Some(info) = self.ses_here_mut(ni) else {
            return;
        };
        info.prompted = true;
        if let Some(conn) = info.conn.as_mut()
            && conn.hide != secret
        {
            conn.hide = secret;
            // (SSH has no such option: the client knows a password prompt by its words)
            if conn.ssh.is_none() {
                let b = if secret { conn.opts.enable_us(OPT_ECHO) } else { conn.opts.disable_us(OPT_ECHO) };
                if let Some(b) = b {
                    conn.tx.extend_from_slice(&b);
                }
            }
        }
        if prompt.is_empty() {
            self.ses_flush(ni, s);
        } else {
            self.ses_write(ni, s, &prompt);
        }
    }

    // the telnet session in place starts: the vty line's login (or none), Linux's login; the
    // idle timer of a vty line
    pub(crate) fn telnet_start(&mut self, ni: usize) {
        let Some((no, kind)) = self.ses_here(ni).map(|i| (i.no, i.kind)) else {
            return;
        };
        if kind != SesKind::Telnet {
            return;
        }
        if self.ssh_ref(ni).is_some() {
            self.ssh_start(ni);
            return;
        }
        match self.cdev(ni).map(|c| c.style) {
            Some(CliStyle::Ios) => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.mode = Mode::User;
                    c.priv_level = 1;
                }
                self.out(ni, "\n");
                self.line_login(ni, &format!("vty {}", no), Requester::Vty(no));
                self.ses_idle_arm(ni);
            }
            _ => {
                if let Some(i) = self.ses_here_mut(ni) {
                    i.login = Some(Login { user: None, tries: 0 });
                }
                self.out(ni, "\n");
            }
        }
    }

    // the idle timer of the session in place: when its line's exec-timeout is up after the
    // last line typed
    pub(crate) fn ses_idle_arm(&mut self, ni: usize) {
        let Some((s, no, last)) = self.ses_here(ni).map(|i| (i.id, i.no, i.last)) else {
            return;
        };
        let Some(t) = self.cdev(ni).and_then(|c| vty_opts(&c.cfg.lines, no)).and_then(|o| exec_timeout(&o)) else {
            return;
        };
        let now = self.now;
        let Some(i) = self.ses_here_mut(ni) else {
            return;
        };
        i.idle_gen = (i.idle_gen + 1) & 0x00FF_FFFF;
        let g = i.idle_gen;
        self.schedule((last + t).max(now), Ev::Ses { node: ni, ses: s, epoch: W_IDLE | g });
    }

    /// the idle timer of the session in place fired: it ends when nothing was typed for the
    /// line's exec-timeout (a command that runs keeps it)
    pub(crate) fn ses_idle(&mut self, ni: usize, generation: u32) {
        let Some((no, last, g, ending)) = self.ses_here(ni).map(|i| (i.no, i.last, i.idle_gen, i.ending)) else {
            return;
        };
        if g != generation || ending {
            return;
        }
        let Some(t) = self.cdev(ni).and_then(|c| vty_opts(&c.cfg.lines, no)).and_then(|o| exec_timeout(&o)) else {
            return;
        };
        let busy = self.cdev(ni).is_some_and(|c| c.job.is_some() || c.gate.as_ref().is_some_and(|g| g.waiting));
        if busy {
            let now = self.now;
            if let Some(i) = self.ses_here_mut(ni) {
                i.last = now;
            }
        } else if self.now >= last + t {
            self.out(ni, &format!("\n% No input for {}: the session ends\n", crate::cli::ses::hms(t)));
            self.vty_close(ni, no, true);
            return;
        }
        self.ses_idle_arm(ni);
    }

    /// exit / logout in a session other than the console: an accounting stop for its user, a
    /// note for a vty line, then it ends
    pub(crate) fn ses_logout(&mut self, ni: usize) {
        let Some((s, no, kind)) = self.ses_here(ni).map(|i| (i.id, i.no, i.kind)) else {
            return;
        };
        if kind == SesKind::Telnet && self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) {
            self.vty_close(ni, no, false);
            return;
        }
        let user = self.cdev(ni).map(|c| c.user.clone()).unwrap_or_default();
        if !user.is_empty() {
            let via = self.ses_req(ni);
            self.aaa_acct(ni, via, &user, false);
        }
        self.ses_end(ni, s);
    }

    /// the session on vty line `n` ends (exit, three wrong passwords, an idle timeout, clear
    /// line): an accounting stop, the note
    pub(crate) fn vty_close(&mut self, ni: usize, n: u16, idle: bool) {
        let Some(s) = self.cdev(ni).and_then(|c| c.sessions.iter().find(|i| i.kind == SesKind::Telnet && i.no == n && !i.ending).map(|i| i.id)) else {
            return;
        };
        let user = self.in_ses(ni, s, |e| e.cdev(ni).map(|c| c.user.clone()).unwrap_or_default()).unwrap_or_default();
        if !user.is_empty() {
            self.in_ses(ni, s, |e| e.aaa_acct(ni, Requester::Vty(n), &user, false));
        }
        // (an SSH session says so with its own note: sshd.rs)
        let ssh_from = self.ses_info(ni, s).and_then(|i| i.conn.as_ref()).filter(|c| c.ssh.is_some()).map(|c| c.from);
        match ssh_from {
            Some(from) => self.ssh_note(ni, crate::sshtrace::SshNote::Closed { from, idle }),
            None => self.aaa_note(ni, ANote::VtyClosed { line: n, idle }),
        }
        self.ses_end(ni, s);
    }

    /// the far end closed a telnet connection: its session ends
    pub(crate) fn ses_hangup(&mut self, ni: usize, sock: u32) {
        let Some(s) = self.ses_by_sock(ni, sock) else {
            return;
        };
        let Some((kind, no, ending)) = self.ses_info(ni, s).map(|i| (i.kind, i.no, i.ending)) else {
            return;
        };
        if ending {
            return;
        }
        // an SSH connection that is gone gets no last words (sshd.rs)
        if let Some(x) = self.ses_info_mut(ni, s).and_then(|i| i.conn.as_mut()).and_then(|c| c.ssh.as_mut()) {
            x.gone = true;
        }
        if kind == SesKind::Telnet && self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) {
            self.vty_close(ni, no, false);
        } else {
            self.ses_end(ni, s);
        }
    }

    /// a telnet server stopped (its connections went with it): its sessions end
    pub(crate) fn ses_server_gone(&mut self, ni: usize, svc: u32) {
        let mine: Vec<u32> = self.cdev(ni).map(|c| c.sessions.iter().filter(|i| i.conn.as_ref().is_some_and(|x| x.svc == svc)).map(|i| i.id).collect()).unwrap_or_default();
        for s in mine {
            if let Some(i) = self.ses_info_mut(ni, s) {
                i.conn = None;
            }
            self.ses_end(ni, s);
        }
    }

    /// where the configuration of the session in place came from ("%SYS-5-CONFIG_I")
    pub(crate) fn ses_config_from(&self, ni: usize) -> String {
        match self.ses_here(ni) {
            Some(i) if i.id != 0 => match &i.conn {
                Some(c) => format!("from {} ({})", i.line, c.from),
                None => format!("at {}", i.line),
            },
            _ => String::from("at the console"),
        }
    }

    // ---------------------------------------------------------------- Linux's login

    /// the prompt of a Linux telnetd login (None: no login waits)
    pub(crate) fn login_prompt(&self, ni: usize, i: &crate::cli::ses::SesInfo) -> Option<String> {
        let l = i.login.as_ref()?;
        Some(match l.user {
            None => format!("{} login: ", self.host(ni)),
            Some(_) => String::from("Password: "),
        })
    }

    /// a line for the login of the session in place -> true when it took it
    pub(crate) fn login_line(&mut self, ni: usize, line: &str) -> bool {
        let Some(l) = self.ses_here(ni).and_then(|i| i.login.clone()) else {
            return false;
        };
        let Some(user) = l.user else {
            let u = String::from(line.trim());
            if !u.is_empty()
                && let Some(x) = self.ses_here_mut(ni).and_then(|i| i.login.as_mut())
            {
                x.user = Some(u);
            }
            return true;
        };
        // the password of the server this session came through
        let svc = self.ses_here(ni).and_then(|i| i.conn.as_ref()).map(|c| c.svc);
        let pw = svc.and_then(|id| self.os(ni)?.svcs.iter().find(|s| s.id == id)?.svc.as_ref()?.as_any().downcast_ref::<TelnetServer>().map(|t| t.password.clone()));
        let ok = user == "root" && pw.is_some_and(|p| p == line);
        if ok {
            if let Some(i) = self.ses_here_mut(ni) {
                i.login = None;
            }
            return true;
        }
        let tries = l.tries + 1;
        self.out(ni, "\nLogin incorrect\n");
        if tries >= 3 {
            let s = self.ses_cur(ni);
            self.ses_end(ni, s);
        } else if let Some(x) = self.ses_here_mut(ni).and_then(|i| i.login.as_mut()) {
            x.user = None;
            x.tries = tries;
        }
        true
    }

    // ---------------------------------------------------------------- IOS commands

    /// transport input, access-class, terminal monitor, show line, clear line
    pub(crate) fn ios_vty(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        match p.cmd {
            K::TransportInput | K::AccessClass => {
                let (key, val) = if p.cmd == K::TransportInput {
                    // the protocols in the order typed
                    let w: Vec<&str> = p.vals.iter().map(|v| v.0.as_str()).filter(|k| matches!(*k, "telnet" | "ssh" | "all" | "none")).collect();
                    ("transport input", format!("transport input {}", w.join(" ")))
                } else {
                    ("access-class", format!("access-class {} in", p.text("acl").unwrap_or("")))
                };
                if let Some(c) = self.cdev_mut(ni) {
                    let i = c.line;
                    if let Some(l) = c.cfg.lines.get_mut(i) {
                        l.opts.retain(|o| !o.starts_with(key));
                        if !p.no {
                            l.opts.push(val);
                        }
                    }
                }
                Ok(())
            }
            K::TermMonitor => {
                let on = !p.has("no");
                if let Some(i) = self.ses_here_mut(ni)
                    && i.id != 0
                {
                    i.monitor = on;
                }
                Ok(())
            }
            K::ShowLine => {
                let t = self.show_line(ni);
                self.out(ni, &t);
                Ok(())
            }
            _ => self.clear_line(ni, p),
        }
    }

    // "show line": the console, the vty lines (in use or free, transport, idle timeout, from
    // where) and the local terminals
    fn show_line(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let mut s = String::from("    Line       State    Transport  Timeout    Location\n");
        let row = |mark: char, line: &str, state: &str, tr: &str, to: &str, loc: &str| String::from(format!("{}   {:<10} {:<8} {:<10} {:<10} {}", mark, line, state, tr, to, loc).trim_end()) + "\n";
        let here = c.cur;
        s.push_str(&row(if here == 0 { '*' } else { ' ' }, "con 0", "in use", "-", "-", ""));
        for n in vty_lines(&c.cfg.lines) {
            let opts = vty_opts(&c.cfg.lines, n).unwrap_or_default();
            let tr = match opts.iter().find(|o| o.starts_with("transport input")) {
                Some(o) => String::from(o.trim_start_matches("transport input").trim()),
                None => String::from("all"),
            };
            let to = exec_timeout(&opts).map_or_else(|| String::from("never"), crate::cli::ses::hms);
            let ses = c.sessions.iter().find(|i| i.kind == SesKind::Telnet && i.no == n);
            let loc = ses.and_then(|i| i.conn.as_ref()).map(|x| format!("{}", x.from)).unwrap_or_default();
            let mark = if ses.is_some_and(|i| i.id == here) { '*' } else { ' ' };
            s.push_str(&row(mark, &format!("vty {}", n), if ses.is_some() { "in use" } else { "free" }, &tr, &to, &loc));
        }
        for i in c.sessions.iter().filter(|i| i.kind == SesKind::Local) {
            s.push_str(&row(if i.id == here { '*' } else { ' ' }, &i.line, "in use", "-", "-", ""));
        }
        s
    }

    // "clear line N | vty N | console 0 | tty N": the session on that line ends (the console
    // logs out)
    fn clear_line(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        enum L {
            Con,
            Vty(u16),
            Tty(u16),
        }
        let which = if let Some(v) = p.num("v") {
            L::Vty(v as u16)
        } else if p.has("c") || p.has("console") {
            L::Con
        } else if let Some(t) = p.num("tt") {
            L::Tty(t as u16)
        } else {
            match p.num("n").unwrap_or(0) {
                0 => L::Con,
                n => L::Vty(n as u16 - 1),
            }
        };
        let target = match which {
            L::Con => Some(0),
            L::Vty(n) => self.cdev(ni).and_then(|c| c.sessions.iter().find(|i| i.kind == SesKind::Telnet && i.no == n).map(|i| i.id)),
            L::Tty(n) => self.cdev(ni).and_then(|c| c.sessions.iter().find(|i| i.kind == SesKind::Local && i.no == n).map(|i| i.id)),
        };
        let Some(s) = target else {
            return self.ios_err(ni, "% Nobody uses that line\n");
        };
        self.out(ni, "[OK]\n");
        if s == 0 {
            // the console logs out: its login again, or "press Enter"
            self.in_ses(ni, 0, |e| {
                if e.cdev(ni).is_some_and(|c| c.job.is_some()) {
                    e.cli_interrupt_at(ni);
                }
                e.out(ni, "\n% This line was cleared from another session.\n");
                e.set_mode(ni, Mode::User);
                if e.line_login_plan(ni, "con 0").is_some() {
                    e.console_login(ni);
                } else {
                    e.set_mode(ni, Mode::Out);
                    e.out(ni, "\nConsole ready, press Enter to log in.\n\n");
                }
                e.cli_state(ni);
            });
            return Ok(());
        }
        let here = self.ses_cur(ni);
        let (kind, no) = self.ses_info(ni, s).map(|i| (i.kind, i.no)).unwrap_or((SesKind::Local, 0));
        if s != here {
            self.ses_write(ni, s, "\n% This line was cleared from another session.\n");
        }
        if kind == SesKind::Telnet {
            self.vty_close(ni, no, false);
        } else {
            self.ses_end(ni, s);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- who, w, tty

    // the device clock at virtual time `t` as "2026-01-01 00:05" and "00:05"
    fn login_time(&self, ni: usize, t: Ns) -> (String, String) {
        let unix = self.os(ni).map_or(crate::os::EPOCH_UNIX_NS, |o| o.clock.read(t)) / 1_000_000_000;
        let c = datetime::Civil::from_unix(unix);
        (format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute), format!("{:02}:{:02}", c.hour, c.minute))
    }

    /// who: every session of the device with its terminal, the time it began and where from
    pub(crate) fn sh_who(&mut self, ni: usize, _args: &[&str]) -> Result<(), String> {
        let rows: Vec<(String, Ns, Option<IpAddr>, String)> = self.cdev(ni).map(|c| c.sessions.iter().filter(|i| i.login.is_none()).map(|i| (i.line.clone(), i.since, i.conn.as_ref().map(|x| x.from), String::from(i.user_name()))).collect()).unwrap_or_default();
        let mut s = String::new();
        for (line, since, from, user) in rows {
            let (when, _) = self.login_time(ni, since);
            s.push_str(&format!("{:<8} {:<12} {}", user, line, when));
            if let Some(f) = from {
                s.push_str(&format!(" ({})", f));
            }
            s.push('\n');
        }
        self.out(ni, &s);
        Ok(())
    }

    /// w: the time, how long the device runs, the sessions with their idle time and what runs
    /// in them
    pub(crate) fn sh_w(&mut self, ni: usize, _args: &[&str]) -> Result<(), String> {
        let now = self.now;
        let (_, clock) = self.login_time(ni, now);
        let unix = self.os(ni).map_or(crate::os::EPOCH_UNIX_NS, |o| o.clock.read(now)) / 1_000_000_000;
        let sec = datetime::Civil::from_unix(unix).second;
        let Some(c) = self.cdev(ni) else {
            return Ok(());
        };
        let here = c.cur;
        let what = |job: &Option<CliJob>, sh: &crate::cli::sh::Shell, id: u32| -> String {
            if id == here {
                return String::from("w");
            }
            match (job, sh.fg.as_ref()) {
                (Some(_), Some(r)) => r.items.get(r.item).map(|x| x.text.clone()).unwrap_or_else(|| String::from("-sh")),
                _ => String::from("-sh"),
            }
        };
        let rows: Vec<(String, String, String, Ns, Ns, String)> = c
            .sessions
            .iter()
            .filter(|i| i.login.is_none())
            .map(|i| {
                let w = if i.id == here { what(&c.job, &c.sh, i.id) } else { c.parked.get(&i.id).map(|p| what(&p.job, &p.sh, i.id)).unwrap_or_default() };
                (String::from(i.user_name()), i.line.clone(), i.conn.as_ref().map_or_else(|| String::from("-"), |x| format!("{}", x.from)), i.since, now.saturating_sub(i.last), w)
            })
            .collect();
        let up = now / SEC;
        let up_text = if up < 3600 { format!("{} min", up / 60) } else { format!("{}:{:02}", up / 3600, (up / 60) % 60) };
        let n = rows.len();
        let mut s = format!(" {}:{:02} up {},  {} user{},  load average: 0.00, 0.00, 0.00\n", clock, sec, up_text, n, if n == 1 { "" } else { "s" });
        s.push_str("USER     TTY      FROM             LOGIN@   IDLE   WHAT\n");
        for (user, line, from, since, idle, w) in rows {
            let (_, at) = self.login_time(ni, since);
            let idle_s = idle / SEC;
            let idle_text = if idle_s < 60 { format!("{}.00s", idle_s) } else { format!("{}:{:02}", idle_s / 60, idle_s % 60) };
            s.push_str(&format!("{:<8} {:<8} {:<16} {:<8} {:<6} {}\n", user, line, from, at, idle_text, w));
        }
        self.out(ni, &s);
        Ok(())
    }

    /// tty: the terminal of the session ("/dev/tty1", "/dev/pts/0")
    pub(crate) fn sh_tty(&mut self, ni: usize, _args: &[&str]) -> Result<(), String> {
        let line = self.ses_here(ni).map_or_else(|| String::from("tty1"), |i| i.line.clone());
        self.out(ni, &format!("/dev/{}\n", line));
        Ok(())
    }

    /// exit / logout in the shell: a further session ends (the console's shell stays)
    pub(crate) fn sh_exit(&mut self, ni: usize) -> Result<(), String> {
        let s = self.ses_cur(ni);
        if s != 0 {
            self.out(ni, "logout\n");
            self.ses_logout(ni);
        }
        Ok(())
    }
}
