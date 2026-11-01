// sshd -- SSH on a device (netpkt::ssh is the protocol): the client "ssh" and "scp" as console
// programs, the servers (the vty lines of IOS-like devices, "sshd" on Linux hosts), each connection
// a command-line session of its own (cli/ses.rs), as telnetd.rs does it for telnet
//
//   SshClient                  the console program of ssh and scp; hides(): the far end asks for
//                              a password, so the console hides what is typed
//   SshServer                  the service of both servers (kind "sshd"): the hidden service
//                              "vty-ssh" of an IOS-like device (SshServer::vty; it listens on TCP 22
//                              while the device has a host key and a vty line takes SSH:
//                              Engine::ssh_sync) or a Linux host's "sshd" of the project:
//                              {"port": 22, "password": "", "users": [{"name": "deploy",
//                              "password": "..."}], "password_auth": true, "key_auth": true}
//   SshCfg                     "ip domain-name" and "ip ssh ..." of an IOS-like device
//   SshConn                    the SSH side of a session's connection (Conn::ssh in cli/ses.rs)
//   Engine::ssh_accept         a connection came to a server: a session for it, or it closes
//   Engine::ssh_events         what the connection brought, with its session in place
//   Engine::ssh_host_keys      the keys of a device: its files (IOS-like nvram:ssh_host_*_key,
//                              Linux /etc/ssh/ssh_host_*_key)
//
// A session over SSH is an X1e session: SesKind::Telnet is "from the network", Conn holds the core
// instead of a telnet reader.  What the session writes before its shell (a banner) waits in
// SshConn::early, because there is no channel for it yet; the login of the session runs when the
// client's SSH_MSG_USERAUTH_REQUEST comes.  IOS-like: through the login gate of the vty line
// (cli/aaa.rs: the line password, local users, AAA with RADIUS, which answers later), "ip ssh
// authentication-retries" tries (3), "ip ssh time-out" seconds (120); password only.  Linux: the
// passwords of the service, ~/.ssh/authorized_keys (/root for root, else /home/NAME) by key; six
// tries, two minutes.  A session ends with the channel's exit status (the shell's $?, "exit N"),
// EOF and close; its connection stays with the server until the client answers and closes (or two
// seconds pass, SshServer::linger).
//
// Lines: our client sends whole lines (the console has its own line editor, as with telnet) and
// shows what comes; a terminal's control bytes are not interpreted.  A line ending in a password
// prompt ("assword: ") hides the next line typed.  Ctrl+C travels to the far end (byte 3 stops the
// command that runs there); a line "~." ends the connection from our side, as in OpenSSH.  The
// host key check of the Linux client: ~/.ssh/known_hosts (OpenSSH's format; [host]:port for
// another port); -o StrictHostKeyChecking=yes|no|accept-new|ask.  The IOS-like client takes any
// key, as the device's defaults do.
//
// Keys, nonces and paddings come from the device's random stream: a server's core gets 32 bytes of
// it per connection, a client's too, sshd its first host key when it starts.  No virtual CPU time
// is charged for the key exchange.  Scp is the old rcp protocol of "scp -t" and "scp -f" over an
// exec channel, one file at a time; the Linux server runs it itself.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::catalog::CliStyle;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::ssh::{self, AuthMethod, Config, Core, Event, KeyPair, Request};
use netpkt::{IpAddr, Ipv4};

use crate::aaa::Requester;
use crate::cli::print::JobOut;
use crate::cli::ses::{Conn, Login, SesKind, W_SSH};
use crate::cli::{CliJob, Mode};
use crate::driver::Report;
use crate::kernel::{Engine, Ev, Ns, SEC};
use crate::os::{ConsoleEv, Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::sshtrace::SshNote;
use crate::tcp::{SockOpts, TcpEnd};
use crate::telnetd::Proto;
use crate::trace::Note;

/// the hidden service of an IOS-like device
pub const IOS_NAME: &str = "vty-ssh";
/// the identification line: our own software, no real product
pub const IDENT: &str = "SSH-2.0-JFNetworker_1";
/// the host key files
pub const IOS_RSA: &str = "nvram:/ssh_host_rsa_key";
pub const IOS_ED: &str = "nvram:/ssh_host_ed25519_key";
pub const LINUX_ED: &str = "/etc/ssh/ssh_host_ed25519_key";
pub const LINUX_RSA: &str = "/etc/ssh/ssh_host_rsa_key";
/// logins before a Linux server gives up
const LINUX_TRIES: u8 = 6;
/// seconds for a login on Linux, the default of "ip ssh time-out"
const LOGIN_SECS: u32 = 120;
/// how long a closed session's connection waits for the client's answer
const LINGER: Ns = 2 * SEC;
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;

/// 32 bytes of the device's random stream
fn seed_of(os: &mut OsApi) -> [u8; 32] {
    let mut s = [0u8; 32];
    for c in s.chunks_mut(4) {
        c.copy_from_slice(&os.rand().to_be_bytes());
    }
    s
}

/// what the IOS-like "ip domain-name" and "ip ssh ..." say
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCfg {
    pub domain: String,
    /// "ip ssh version 2" was given (we speak nothing else)
    pub version2: bool,
    /// seconds for a login
    pub timeout: u32,
    /// tries of a login
    pub retries: u32,
}

impl Default for SshCfg {
    fn default() -> SshCfg {
        SshCfg { domain: String::new(), version2: false, timeout: LOGIN_SECS, retries: 3 }
    }
}

impl SshCfg {
    /// the bytes it holds (keyframe::node_bytes)
    pub fn footprint(&self) -> usize {
        self.domain.len() + 32
    }
}

/// the SSH side of a session's connection
#[derive(Clone, Debug)]
pub struct SshConn {
    pub core: Core,
    pub channel: Option<u32>,
    /// its shell or command runs
    pub started: bool,
    /// a command: the session ends with it
    pub exec: bool,
    /// the client asked for a terminal: line ends go out as CR LF
    pub pty: bool,
    pub authed: bool,
    pub tries: u8,
    /// the user who logged in
    pub user: String,
    /// what the core reported and the session did not see yet
    pub events: VecDeque<Event>,
    /// text written before the shell
    pub early: Vec<u8>,
    /// a CR came last: the LF after it is the same line end
    pub last_cr: bool,
    /// the peer is gone: no last words
    pub gone: bool,
    pub scp: Option<ScpSrv>,
}

impl SshConn {
    fn new(core: Core) -> SshConn {
        SshConn { core, channel: None, started: false, exec: false, pty: false, authed: false, tries: 0, user: String::new(), events: VecDeque::new(), early: Vec::new(), last_cr: false, gone: false, scp: None }
    }
    /// what the session writes
    pub fn write(&mut self, text: &[u8]) {
        let (true, Some(ch)) = (self.started, self.channel) else {
            self.early.extend_from_slice(text);
            return;
        };
        if self.pty {
            let mut b = Vec::with_capacity(text.len() + 8);
            for &c in text {
                if c == b'\n' {
                    b.push(b'\r');
                }
                b.push(c);
            }
            self.core.send(ch, &b);
        } else {
            self.core.send(ch, text);
        }
    }
    /// what the connection brought goes into the core
    pub fn feed(&mut self, data: &[u8], tx: &mut Vec<u8>) {
        self.core.push(data);
        tx.extend_from_slice(&self.core.take_out());
        self.events.extend(self.core.take_events());
    }
    /// the bytes it holds
    pub fn footprint(&self) -> usize {
        self.core.footprint() + self.early.len() + self.user.len() + self.events.len() * 64 + 96
    }
}

// ---------------------------------------------------------------- the server

/// a finished session's connection, until the client closes it
#[derive(Clone, Debug)]
struct Lingering {
    sock: u32,
    core: Core,
    tx: Vec<u8>,
    until: Ns,
}

/// the SSH server of a device: the vty lines (IOS-like) or sshd (Linux)
#[derive(Clone, Debug)]
pub struct SshServer {
    ios: bool,
    pub port: u16,
    /// root's password (Linux; "": none, only keys)
    pub password: String,
    /// more users of a Linux host
    pub users: Vec<(String, String)>,
    pub password_auth: bool,
    pub key_auth: bool,
    listener: Option<u32>,
    linger: Vec<Lingering>,
    accepted: u64,
    refused: u64,
    logins: u64,
    bad_logins: u64,
}

impl SshServer {
    /// the vty lines' server of an IOS-like device
    pub fn vty() -> SshServer {
        SshServer { ios: true, port: ssh::PORT, password: String::new(), users: Vec::new(), password_auth: true, key_auth: false, listener: None, linger: Vec::new(), accepted: 0, refused: 0, logins: 0, bad_logins: 0 }
    }
    /// sshd of a Linux host from its settings
    pub fn from_config(c: &Value) -> Result<SshServer, Msg> {
        let mut s = SshServer::vty();
        s.ios = false;
        s.key_auth = true;
        if let Some(p) = c.get("port") {
            s.port = p.as_u64().filter(|&n| (1..=65535).contains(&n)).ok_or_else(|| msg!("\u{201e}port\u{201c} ist eine Portnummer von 1 bis 65535."))? as u16;
        }
        if let Some(p) = c.get("password") {
            s.password = String::from(p.as_str().ok_or_else(|| msg!("\u{201e}password\u{201c} ist ein Text."))?);
        }
        if let Some(p) = c.get("password_auth") {
            s.password_auth = p.as_bool().ok_or_else(|| msg!("\u{201e}password_auth\u{201c} ist true oder false."))?;
        }
        if let Some(p) = c.get("key_auth") {
            s.key_auth = p.as_bool().ok_or_else(|| msg!("\u{201e}key_auth\u{201c} ist true oder false."))?;
        }
        if let Some(u) = c.get("users") {
            let list = u.as_array().ok_or_else(|| msg!("\u{201e}users\u{201c} ist eine Liste."))?;
            for x in list {
                let name = x.get("name").and_then(|n| n.as_str()).ok_or_else(|| msg!("Ein Benutzer in \u{201e}users\u{201c} braucht \u{201e}name\u{201c}."))?;
                let pw = x.get("password").and_then(|n| n.as_str()).unwrap_or("");
                s.users.push((String::from(name), String::from(pw)));
            }
        }
        Ok(s)
    }
    /// a password fits a user who may log in with one
    fn password_ok(&self, user: &str, pw: &str) -> bool {
        if pw.is_empty() {
            return false;
        }
        (user == "root" && self.password == pw) || self.users.iter().any(|(n, p)| n == user && p == pw)
    }
    /// a user this server knows
    fn knows(&self, user: &str) -> bool {
        user == "root" || self.users.iter().any(|(n, _)| n == user)
    }
    /// what a failed login names
    fn methods(&self) -> String {
        let mut v = Vec::new();
        if self.key_auth {
            v.push("publickey");
        }
        if self.password_auth {
            v.push("password");
        }
        v.join(",")
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
    /// a Linux host's first host key
    fn make_host_key(&self, os: &mut OsApi) {
        if os.file_stat(LINUX_ED).is_some() {
            return;
        }
        let key = KeyPair::ed25519(&seed_of(os));
        let host = os.hostname();
        let comment = format!("root@{}", host);
        let _ = os.file_mkdir("/etc/ssh", true);
        let _ = os.file_write(LINUX_ED, key.private_file(&comment).as_bytes());
        let _ = os.file_write(&format!("{}.pub", LINUX_ED), format!("{}\n", key.public_line(&comment)).as_bytes());
        os.note(Note::Ssh(SshNote::KeyMade { kind: String::from("ed25519"), bits: 256, fingerprint: ssh::fingerprint(&key.blob()) }));
    }
    fn flush_linger(&mut self, os: &mut OsApi, i: usize) {
        let l = &mut self.linger[i];
        let out = l.core.take_out();
        l.tx.extend_from_slice(&out);
        if !l.tx.is_empty() {
            let n = os.tcp_send(l.sock, &l.tx);
            l.tx.drain(..n);
        }
    }
}

impl Service for SshServer {
    fn kind(&self) -> &'static str {
        "sshd"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.listener = None;
        self.linger.clear();
        if !self.ios {
            self.make_host_key(os);
        }
        let on = !self.ios || os.eng.ssh_wanted(os.ni);
        self.listen(os, on);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        let (ni, id) = (os.ni, os.id);
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    if os.eng.ssh_accept(ni, id, s, self.ios, &self.methods()) {
                        self.accepted += 1;
                    } else {
                        self.refused += 1;
                    }
                }
            }
            SvcEvent::TcpData { sock, .. } => match self.linger.iter().position(|l| l.sock == sock) {
                Some(i) => {
                    let d = os.tcp_recv(sock, usize::MAX);
                    self.linger[i].core.push(&d);
                    self.linger[i].core.take_events();
                    self.flush_linger(os, i);
                }
                None => os.eng.ses_rx(ni, sock),
            },
            SvcEvent::TcpWritable { sock } => match self.linger.iter().position(|l| l.sock == sock) {
                Some(i) => self.flush_linger(os, i),
                None => os.eng.ses_flush_sock(ni, sock),
            },
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } => match self.linger.iter().position(|l| l.sock == sock) {
                Some(i) => {
                    self.linger.remove(i);
                    os.tcp_close(sock);
                }
                None => os.eng.ses_hangup(ni, sock),
            },
            SvcEvent::Timer(0) => {
                let now = os.now();
                let mut keep = Vec::new();
                for l in core::mem::take(&mut self.linger) {
                    if l.until <= now {
                        os.tcp_close(l.sock);
                    } else {
                        keep.push(l);
                    }
                }
                self.linger = keep;
                if let Some(t) = self.linger.iter().map(|l| l.until).min() {
                    os.timer(0, t);
                }
            }
            _ => {}
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.listener = None;
        self.linger.clear();
        let (ni, id) = (os.ni, os.id);
        os.eng.ses_server_gone(ni, id);
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("accepted", self.accepted as i64), Metric::new("refused", self.refused as i64), Metric::new("logins", self.logins as i64), Metric::new("bad_logins", self.bad_logins as i64)]
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
    fn footprint(&self) -> usize {
        self.password.len() + self.users.iter().map(|(a, b)| a.len() + b.len() + 48).sum::<usize>() + self.linger.iter().map(|l| l.core.footprint() + l.tx.len() + 64).sum::<usize>() + 128
    }
}

impl Engine {
    /// the host keys of a device (its files)
    pub(crate) fn ssh_host_keys(&self, ni: usize) -> Vec<KeyPair> {
        let Some(store) = self.store(ni) else {
            return Vec::new();
        };
        let ios = self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios);
        let paths = if ios { [IOS_ED, IOS_RSA] } else { [LINUX_ED, LINUX_RSA] };
        paths.iter().filter_map(|p| store.read(p).ok()).filter_map(|b| KeyPair::parse_private(&String::from_utf8_lossy(b))).map(|k| k.0).collect()
    }

    /// the vty lines' SSH server should listen: the device has a host key and a line takes SSH
    pub(crate) fn ssh_wanted(&self, ni: usize) -> bool {
        self.vty_wanted_for(ni, Proto::Ssh) && self.store(ni).is_some_and(|s| s.exists(IOS_RSA) || s.exists(IOS_ED))
    }

    /// the SSH server of an IOS-like device follows its keys and vty lines: made (hidden) when it
    /// is first wanted, then listening or not (after every configuration line)
    pub(crate) fn ssh_sync(&mut self, ni: usize) {
        if self.cdev(ni).is_none_or(|c| c.style != CliStyle::Ios) || self.os(ni).is_none() {
            return;
        }
        let want = self.ssh_wanted(ni);
        let id = self.os(ni).and_then(|o| o.svc_id(IOS_NAME));
        match id {
            None if !want => {}
            None => {
                let dev = self.dev_of(ni);
                if self.add_service(dev, IOS_NAME, Box::new(SshServer::vty()), true).is_ok()
                    && let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.name == IOS_NAME))
                {
                    s.console = true;
                }
            }
            Some(id) => {
                let listening = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<SshServer>()).map(|t| t.listening());
                if listening.is_some_and(|l| l != want) {
                    self.svc_call(ni, id, None, |svc, os| {
                        if let Some(t) = svc.as_any_mut().downcast_mut::<SshServer>() {
                            t.listen(os, want);
                        }
                    });
                }
            }
        }
    }

    /// a note and a German log line of SSH
    pub(crate) fn ssh_note(&mut self, ni: usize, n: SshNote) {
        let dev = self.dev_of(ni);
        let at = self.now;
        let sev = if n.is_drop() { SEV_WARN } else { SEV_INFO };
        let text = n.message().text();
        self.cli.reports.push(Report::Log { dev, at, sev, tag: String::from("ssh"), text });
        self.note(dev, None, Note::Ssh(n));
    }

    /// a connection came to an SSH server (its service `svc`): a session for it, or it closes
    /// -> it got a session
    pub(crate) fn ssh_accept(&mut self, ni: usize, svc: u32, sock: u32, ios: bool, methods: &str) -> bool {
        let Some(row) = self.tcp_row(ni, sock) else {
            return false;
        };
        let (from, rport) = row.remote;
        let no = if ios {
            match self.vty_free(ni, from, row.local.0, rport, Proto::Ssh) {
                Ok(n) => Some(n),
                Err((why, _)) => {
                    self.tcp_close(ni, svc, sock, None);
                    self.ssh_note(ni, SshNote::Refused { from, why });
                    return false;
                }
            }
        } else {
            None
        };
        let keys = self.ssh_host_keys(ni);
        if keys.is_empty() {
            self.tcp_close(ni, svc, sock, None);
            self.ssh_note(ni, SshNote::Refused { from, why: String::from("kein Schlüssel") });
            return false;
        }
        let mut seed = [0u8; 32];
        if let Some(Some(n)) = self.nodes.get_mut(ni) {
            n.rng.fill(&mut seed);
        }
        let cfg = Config { ident: String::from(IDENT), methods: String::from(methods), ..Config::default() };
        let mut core = Core::server(cfg, keys, &seed);
        let tx = core.take_out();
        let conn = Conn {
            svc,
            sock,
            from,
            port: rport,
            reader: netpkt::telnet::Reader::new(true),
            opts: netpkt::telnet::Options::new(&[], &[]),
            tx,
            rx: Vec::new(),
            inq: VecDeque::new(),
            intr: false,
            hide: false,
            woken: false,
            ssh: Some(Box::new(SshConn::new(core))),
        };
        let Some(s) = self.ses_new(ni, SesKind::Telnet, Some(conn), no) else {
            self.tcp_close(ni, svc, sock, None);
            return false;
        };
        self.ssh_note(ni, SshNote::Accept { from, line: no });
        self.ses_flush(ni, s);
        self.ses_start_later(ni, s);
        true
    }

    pub(crate) fn ssh_ref(&self, ni: usize) -> Option<&SshConn> {
        self.ses_here(ni)?.conn.as_ref()?.ssh.as_deref()
    }
    pub(crate) fn ssh_mut(&mut self, ni: usize) -> Option<&mut SshConn> {
        self.ses_here_mut(ni)?.conn.as_mut()?.ssh.as_deref_mut()
    }
    fn ssh_from(&self, ni: usize) -> IpAddr {
        self.ses_here(ni).and_then(|i| i.conn.as_ref()).map_or(IpAddr::V4(Ipv4::UNSPECIFIED), |c| c.from)
    }

    /// what the core wants to send goes to the connection
    pub(crate) fn ssh_flush(&mut self, ni: usize) {
        let Some(s) = self.ses_here(ni).map(|i| i.id) else {
            return;
        };
        let Some(c) = self.ses_here_mut(ni).and_then(|i| i.conn.as_mut()) else {
            return;
        };
        if let Some(x) = c.ssh.as_mut() {
            let out = x.core.take_out();
            c.tx.extend_from_slice(&out);
        }
        self.ses_flush(ni, s);
    }

    /// the session in place of an SSH connection starts: the vty line's login waits for the
    /// client, Linux's session is hidden from who until it logged in
    pub(crate) fn ssh_start(&mut self, ni: usize) {
        let Some((s, no)) = self.ses_here(ni).map(|i| (i.id, i.no)) else {
            return;
        };
        let mut secs = LOGIN_SECS;
        if self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) {
            if let Some(c) = self.cdev_mut(ni) {
                c.mode = Mode::User;
                c.priv_level = 1;
                secs = c.cfg.ssh.timeout;
            }
            self.line_login(ni, &format!("vty {}", no), Requester::Vty(no));
            self.ses_idle_arm(ni);
        } else if let Some(i) = self.ses_here_mut(ni) {
            i.login = Some(Login { user: None, tries: 0 });
        }
        let at = self.now + secs as Ns * SEC;
        self.schedule(at, Ev::Ses { node: ni, ses: s, epoch: W_SSH });
    }

    /// the time for a login is up
    pub(crate) fn ssh_timeout(&mut self, ni: usize) {
        if self.ssh_ref(ni).is_none_or(|x| x.authed) || self.ses_here(ni).is_some_and(|i| i.ending) {
            return;
        }
        let from = self.ssh_from(ni);
        self.ssh_note(ni, SshNote::LoginTimeout { from });
        if let Some(x) = self.ssh_mut(ni) {
            x.core.disconnect(ssh::DISC_BY_APPLICATION, "the time for a login is up");
        }
        self.ssh_flush(ni);
        self.ssh_end(ni);
    }

    // the session in place ends: a vty line with its note, a Linux session
    fn ssh_end(&mut self, ni: usize) {
        let Some((s, no)) = self.ses_here(ni).map(|i| (i.id, i.no)) else {
            return;
        };
        if self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) {
            self.vty_close(ni, no, false);
        } else {
            let from = self.ssh_from(ni);
            if self.ssh_ref(ni).is_some_and(|x| x.authed) {
                self.ssh_note(ni, SshNote::Closed { from, idle: false });
            }
            self.ses_end(ni, s);
        }
    }

    /// what the connection brought, with its session in place
    pub(crate) fn ssh_events(&mut self, ni: usize) {
        while let Some(e) = self.ssh_mut(ni).and_then(|x| x.events.pop_front()) {
            self.ssh_event(ni, e);
            self.ssh_flush(ni);
        }
    }

    fn ssh_event(&mut self, ni: usize, e: Event) {
        let from = self.ssh_from(ni);
        match e {
            Event::Encrypted => {
                if let Some(a) = self.ssh_ref(ni).map(|x| x.core.algos().clone()) {
                    self.ssh_note(ni, SshNote::Keyed { peer: from, cipher: a.enc_sc, host_key: a.host_key });
                }
            }
            Event::AuthRequest { user, method } => self.ssh_auth(ni, user, method),
            Event::ChannelOpen { channel, kind } => {
                let Some(x) = self.ssh_mut(ni) else {
                    return;
                };
                if kind != "session" {
                    x.core.refuse_channel(channel, 3, "only session channels here");
                } else if x.channel.is_some() {
                    x.core.refuse_channel(channel, 1, "one session per connection");
                } else {
                    x.channel = Some(channel);
                    x.core.accept_channel(channel);
                }
            }
            Event::ChannelRequest { channel, request, want_reply } => self.ssh_request(ni, channel, request, want_reply),
            Event::Data { data, .. } => self.ssh_data(ni, &data),
            Event::Eof { .. } => {
                // the end of a shell's input ends it, scp is done; a command goes on
                if self.ssh_ref(ni).is_some_and(|x| x.scp.is_some() || (x.started && !x.exec)) {
                    self.ssh_end(ni);
                }
            }
            Event::Closed { .. } => {
                if let Some(x) = self.ssh_mut(ni) {
                    x.channel = None;
                }
                self.ssh_end(ni);
            }
            Event::Disconnected { reason, text } => {
                if let Some(x) = self.ssh_mut(ni) {
                    x.gone = true;
                }
                if reason != ssh::DISC_BY_APPLICATION {
                    self.ssh_note(ni, SshNote::Broken { peer: from, why: format!("{}: {}", ssh::reason_name(reason), text) });
                }
                self.ssh_end(ni);
            }
            Event::Failed { reason, text } => {
                self.ssh_note(ni, SshNote::Broken { peer: from, why: format!("{}: {}", ssh::reason_name(reason), text) });
                self.ssh_end(ni);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ logins

    fn ssh_auth(&mut self, ni: usize, user: String, method: AuthMethod) {
        if self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) {
            self.ssh_auth_ios(ni, user, method);
        } else {
            self.ssh_auth_linux(ni, user, method);
        }
    }

    // through the login gate of the vty line
    fn ssh_auth_ios(&mut self, ni: usize, user: String, method: AuthMethod) {
        let Some(no) = self.ses_here(ni).map(|i| i.no) else {
            return;
        };
        let plan = self.line_login_plan(ni, &format!("vty {}", no));
        match method {
            // a line without "login" lets everyone in
            AuthMethod::None if plan.is_none() => self.ssh_auth_open(ni, user),
            AuthMethod::Password(_) if plan.is_none() => self.ssh_auth_open(ni, user),
            AuthMethod::Password(pass) => {
                let via = Requester::Vty(no);
                let methods = plan.unwrap_or_default();
                if let Some(c) = self.cdev_mut(ni)
                    && let Some(g) = c.gate.as_mut()
                {
                    g.user = user.clone();
                    g.kind = crate::cli::aaa::GKind::Password;
                }
                self.login_run(ni, via, methods, user, pass);
            }
            _ => {
                if let Some(x) = self.ssh_mut(ni) {
                    x.core.auth_reject();
                }
            }
        }
    }

    fn ssh_auth_open(&mut self, ni: usize, user: String) {
        if let Some(c) = self.cdev_mut(ni) {
            c.gate = None;
            c.user = user.clone();
        }
        self.ssh_login_ok(ni, &user, "ohne Passwort");
    }

    /// the login gate let the session in place in (cli/aaa.rs)
    pub(crate) fn ssh_gate_ok(&mut self, ni: usize, user: &str, how: &str) {
        self.ssh_login_ok(ni, user, how);
    }

    /// the login gate refused (cli/aaa.rs) -> the session in place was an SSH one
    pub(crate) fn ssh_gate_bad(&mut self, ni: usize, user: &str) -> bool {
        if self.ssh_ref(ni).is_none() {
            return false;
        }
        self.ssh_login_bad(ni, user, "Passwort");
        true
    }

    fn ssh_auth_linux(&mut self, ni: usize, user: String, method: AuthMethod) {
        let Some(svc) = self.ses_here(ni).and_then(|i| i.conn.as_ref()).map(|c| c.svc) else {
            return;
        };
        let Some(sv) = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == svc)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<SshServer>()).cloned() else {
            return;
        };
        match method {
            AuthMethod::None => {
                if let Some(x) = self.ssh_mut(ni) {
                    x.core.auth_reject();
                }
            }
            AuthMethod::Password(p) => {
                if sv.password_auth && sv.password_ok(&user, &p) {
                    self.ssh_login_ok(ni, &user, "Passwort");
                } else {
                    self.ssh_login_bad(ni, &user, "Passwort");
                }
            }
            AuthMethod::PublicKey { alg, blob, signed, valid } => {
                let known = sv.key_auth && sv.knows(&user) && self.ssh_authorized(ni, &user, &blob);
                match (signed, known) {
                    (false, true) => {
                        if let Some(x) = self.ssh_mut(ni) {
                            x.core.auth_pk_ok(&alg, &blob);
                        }
                    }
                    (false, false) => {
                        if let Some(x) = self.ssh_mut(ni) {
                            x.core.auth_reject();
                        }
                    }
                    (true, true) if valid => self.ssh_login_ok(ni, &user, "Schlüssel"),
                    _ => self.ssh_login_bad(ni, &user, "Schlüssel"),
                }
            }
        }
    }

    // the key is in the user's ~/.ssh/authorized_keys
    fn ssh_authorized(&self, ni: usize, user: &str, blob: &[u8]) -> bool {
        let path = format!("{}/.ssh/authorized_keys", home_of(user));
        let Some(text) = self.store(ni).and_then(|s| s.read(&path).ok()).map(|b| String::from_utf8_lossy(b).into_owned()) else {
            return false;
        };
        text.lines().filter_map(ssh::parse_public).any(|p| p.blob == blob)
    }

    fn ssh_count(&mut self, ni: usize, ok: bool) {
        let Some(svc) = self.ses_here(ni).and_then(|i| i.conn.as_ref()).map(|c| c.svc) else {
            return;
        };
        if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == svc)).and_then(|s| s.svc.as_mut()).and_then(|x| x.as_any_mut().downcast_mut::<SshServer>()) {
            if ok {
                s.logins += 1;
            } else {
                s.bad_logins += 1;
            }
        }
    }

    fn ssh_login_ok(&mut self, ni: usize, user: &str, how: &str) {
        let from = self.ssh_from(ni);
        self.ssh_count(ni, true);
        if let Some(x) = self.ssh_mut(ni) {
            x.authed = true;
            x.user = String::from(user);
            x.core.auth_accept();
        }
        let linux = self.cdev(ni).is_some_and(|c| c.style == CliStyle::Linux);
        if linux {
            if let Some(i) = self.ses_here_mut(ni) {
                i.login = None;
                if user != "root" {
                    i.account = String::from(user);
                }
            }
            if user != "root" {
                let home = home_of(user);
                let _ = self.file_mkdir(ni, &home, true);
                if let Some(sh) = self.sh_mut(ni) {
                    sh.cwd = home.clone();
                    sh.set_var("HOME", &home);
                    sh.set_var("PWD", &home);
                    sh.set_var("USER", user);
                    sh.set_var("LOGNAME", user);
                }
            }
        }
        self.ssh_note(ni, SshNote::LoginOk { from, user: String::from(user), how: String::from(how) });
        self.ssh_flush(ni);
    }

    fn ssh_login_bad(&mut self, ni: usize, user: &str, how: &str) {
        let from = self.ssh_from(ni);
        self.ssh_count(ni, false);
        self.ssh_note(ni, SshNote::LoginBad { from, user: String::from(user), how: String::from(how) });
        let linux = self.cdev(ni).is_some_and(|c| c.style == CliStyle::Linux);
        let most = if linux { LINUX_TRIES as u32 } else { self.cdev(ni).map_or(3, |c| c.cfg.ssh.retries.max(1)) };
        let Some(x) = self.ssh_mut(ni) else {
            return;
        };
        x.tries = x.tries.saturating_add(1);
        if x.tries as u32 >= most {
            x.core.disconnect(ssh::DISC_NO_MORE_AUTH_METHODS, "too many logins that failed");
            self.ssh_note(ni, SshNote::TooManyTries { from, user: String::from(user) });
            self.ssh_flush(ni);
            self.ssh_end(ni);
        } else {
            x.core.auth_reject();
            self.ssh_flush(ni);
        }
    }

    // ------------------------------------------------------------ channels

    fn ssh_request(&mut self, ni: usize, channel: u32, request: Request, want_reply: bool) {
        let reply = |e: &mut Engine, ok: bool| {
            if want_reply && let Some(x) = e.ssh_mut(ni) {
                x.core.reply(channel, ok);
            }
        };
        let busy = self.ssh_ref(ni).is_none_or(|x| x.started);
        match request {
            Request::Pty { .. } => {
                if let Some(x) = self.ssh_mut(ni) {
                    x.pty = true;
                }
                reply(self, true);
            }
            Request::Env { .. } | Request::WindowChange { .. } => reply(self, true),
            Request::Shell if !busy => {
                reply(self, true);
                self.ssh_begin(ni, None);
            }
            Request::Exec(cmd) if !busy => {
                reply(self, true);
                self.ssh_begin(ni, Some(cmd));
            }
            _ => reply(self, false),
        }
    }

    // the shell or the command of the session in place begins
    fn ssh_begin(&mut self, ni: usize, cmd: Option<String>) {
        let from = self.ssh_from(ni);
        let what = cmd.clone().unwrap_or_else(|| String::from("Shell"));
        self.ssh_note(ni, SshNote::Session { from, what });
        let Some(x) = self.ssh_mut(ni) else {
            return;
        };
        x.started = true;
        x.exec = cmd.is_some();
        let early = core::mem::take(&mut x.early);
        match cmd {
            None => {
                if let Some(c) = self.ses_here_mut(ni).and_then(|i| i.conn.as_mut()) {
                    c.write_text(&String::from_utf8_lossy(&early));
                }
                if let Some(i) = self.ses_here_mut(ni) {
                    i.prompted = false;
                }
                self.cli_state(ni);
            }
            Some(line) => {
                if self.cdev(ni).is_some_and(|c| c.style == CliStyle::Linux)
                    && let Some(srv) = ScpSrv::of_command(&line)
                {
                    self.ssh_scp_begin(ni, srv);
                    return;
                }
                self.ses_took_line(ni);
                self.cli_line_at(ni, &line);
            }
        }
    }

    // what the client typed: whole lines for the session
    fn ssh_data(&mut self, ni: usize, data: &[u8]) {
        if self.ssh_ref(ni).is_some_and(|x| x.scp.is_some()) {
            self.ssh_scp_data(ni, data);
            return;
        }
        if self.ssh_ref(ni).is_none_or(|x| x.exec || !x.started) {
            return;
        }
        let Some(c) = self.ses_here_mut(ni).and_then(|i| i.conn.as_mut()) else {
            return;
        };
        let Some(x) = c.ssh.as_mut() else {
            return;
        };
        for &b in data {
            let was_cr = core::mem::take(&mut x.last_cr);
            match b {
                b'\n' if was_cr => {}
                b'\n' | b'\r' => {
                    x.last_cr = b == b'\r';
                    let l = String::from_utf8_lossy(&c.rx).into_owned();
                    c.rx.clear();
                    c.inq.push_back(l);
                }
                0 => {}
                3 => c.intr = true,
                8 | 127 => {
                    c.rx.pop();
                }
                b => c.rx.push(b),
            }
        }
    }

    /// the connection of a session goes (cli/ses.rs: ses_drop): the shell's $? as its exit
    /// status, EOF and close; the server keeps the connection until the client closes it
    pub(crate) fn ssh_drop(&mut self, ni: usize, conn: Conn, status: i32) {
        let Conn { svc, sock, mut tx, ssh: Some(mut x), .. } = conn else {
            return;
        };
        if !x.gone && !x.core.dead() {
            match x.channel.filter(|_| x.authed) {
                Some(ch) => {
                    x.core.request(ch, &Request::ExitStatus(status as u32), false);
                    x.core.eof(ch);
                    x.core.close(ch);
                }
                None => x.core.disconnect(ssh::DISC_BY_APPLICATION, "bye"),
            }
        }
        tx.extend_from_slice(&x.core.take_out());
        let n = if tx.is_empty() { 0 } else { self.tcp_write(ni, svc, sock, &tx, None) };
        tx.drain(..n);
        if x.gone || x.core.dead() {
            self.tcp_close(ni, svc, sock, None);
            return;
        }
        let until = self.now + LINGER;
        let core = x.core.clone();
        self.svc_call(ni, svc, None, |s, os| {
            if let Some(t) = s.as_any_mut().downcast_mut::<SshServer>() {
                t.linger.push(Lingering { sock, core, tx, until });
                os.timer(0, until);
            }
        });
    }

    /// "show users": the location of a session says it came over SSH
    pub(crate) fn ssh_marker(i: &crate::cli::ses::SesInfo) -> &'static str {
        if i.conn.as_ref().is_some_and(|c| c.ssh.is_some()) { " (ssh)" } else { "" }
    }

    // ------------------------------------------------------------ scp, the server's side

    fn ssh_scp_begin(&mut self, ni: usize, mut srv: ScpSrv) {
        let Some(ch) = self.ssh_ref(ni).and_then(|x| x.channel) else {
            return;
        };
        if srv.sink {
            if let Some(x) = self.ssh_mut(ni) {
                x.core.send(ch, &[0]);
                x.scp = Some(srv);
            }
            return;
        }
        // a source: it waits for the client's first 0
        srv.data = self.store(ni).and_then(|s| s.read(&srv.path).ok()).map(Vec::from);
        if let Some(x) = self.ssh_mut(ni) {
            x.scp = Some(srv);
        }
    }

    fn ssh_scp_status(&mut self, ni: usize, code: i32) {
        if let Some(sh) = self.sh_mut(ni) {
            sh.status = code;
        }
        if let Some(s) = self.ses_here(ni).map(|i| i.id) {
            self.ses_end(ni, s);
        }
    }

    fn ssh_scp_data(&mut self, ni: usize, data: &[u8]) {
        let from = self.ssh_from(ni);
        let Some(ch) = self.ssh_ref(ni).and_then(|x| x.channel) else {
            return;
        };
        let Some(mut srv) = self.ssh_mut(ni).and_then(|x| x.scp.take()) else {
            return;
        };
        let mut send: Vec<u8> = Vec::new();
        let mut end = None;
        srv.buf.extend_from_slice(data);
        if srv.sink {
            loop {
                match srv.size {
                    None => {
                        let Some(nl) = srv.buf.iter().position(|&b| b == b'\n') else {
                            break;
                        };
                        let line: Vec<u8> = srv.buf.drain(..=nl).collect();
                        let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                        let mut w = text.split_whitespace();
                        let (mode, size, name) = (w.next().unwrap_or(""), w.next().and_then(|n| n.parse::<usize>().ok()), w.next().unwrap_or(""));
                        match (mode.starts_with('C'), size) {
                            (true, Some(n)) => {
                                srv.size = Some(n);
                                srv.name = String::from(name);
                                send.push(0);
                            }
                            _ => {
                                send.extend_from_slice(b"\x01scp: cannot read that\n");
                                end = Some(1);
                                break;
                            }
                        }
                    }
                    Some(n) => {
                        if srv.buf.len() < n + 1 {
                            break;
                        }
                        let bytes: Vec<u8> = srv.buf.drain(..n).collect();
                        srv.buf.remove(0);
                        srv.size = None;
                        let is_dir = self.store(ni).is_some_and(|s| s.is_dir(&srv.path));
                        let target = if is_dir { format!("{}/{}", srv.path.trim_end_matches('/'), srv.name) } else { srv.path.clone() };
                        match self.file_put(ni, &target, &bytes, false) {
                            Ok(_) => {
                                send.push(0);
                                self.ssh_note(ni, SshNote::Scp { peer: from, file: target, bytes: n, upload: false });
                            }
                            Err(e) => {
                                send.extend_from_slice(format!("\x01scp: {}: {}\n", target, e.errno_text()).as_bytes());
                                end = Some(1);
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            // the client's 0s: "go on"
            while let Some(_b) = srv.buf.first().copied() {
                srv.buf.remove(0);
                match srv.stage {
                    0 => match &srv.data {
                        Some(d) => {
                            let name = srv.path.rsplit('/').next().unwrap_or("file");
                            send.extend_from_slice(format!("C0644 {} {}\n", d.len(), name).as_bytes());
                            srv.stage = 1;
                        }
                        None => {
                            send.extend_from_slice(format!("\x01scp: {}: No such file or directory\n", srv.path).as_bytes());
                            end = Some(1);
                        }
                    },
                    1 => {
                        let n = srv.data.as_ref().map_or(0, |d| d.len());
                        if let Some(d) = &srv.data {
                            send.extend_from_slice(d);
                        }
                        send.push(0);
                        srv.stage = 2;
                        self.ssh_note(ni, SshNote::Scp { peer: from, file: srv.path.clone(), bytes: n, upload: true });
                    }
                    _ => end = Some(0),
                }
            }
        }
        if let Some(x) = self.ssh_mut(ni) {
            if !send.is_empty() {
                x.core.send(ch, &send);
            }
            x.scp = Some(srv);
        }
        if let Some(code) = end {
            self.ssh_scp_status(ni, code);
        }
    }
}

/// a user's home directory
pub(crate) fn home_of(user: &str) -> String {
    if user == "root" { String::from("/root") } else { format!("/home/{}", user) }
}

/// the server's side of an scp
#[derive(Clone, Debug)]
pub struct ScpSrv {
    /// "scp -t": it takes a file; else "scp -f": it gives one
    sink: bool,
    path: String,
    buf: Vec<u8>,
    size: Option<usize>,
    name: String,
    data: Option<Vec<u8>>,
    stage: u8,
}

impl ScpSrv {
    /// from the command of an exec request: "scp -t PATH", "scp -f PATH"
    fn of_command(cmd: &str) -> Option<ScpSrv> {
        let mut w = cmd.split_whitespace();
        if w.next()? != "scp" {
            return None;
        }
        let mode = w.next()?;
        if mode != "-t" && mode != "-f" {
            return None;
        }
        let path = String::from(w.next()?);
        Some(ScpSrv { sink: mode == "-t", path, buf: Vec::new(), size: None, name: String::new(), data: None, stage: 0 })
    }
}

// ---------------------------------------------------------------- the client

/// what the Linux client does about a host key it does not know
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strict {
    Ask,
    Yes,
    AcceptNew,
    No,
}

/// how the program was started
#[derive(Clone, Debug)]
pub struct ClientOpts {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub cmd: Option<String>,
    /// the IOS-like console's client
    pub ios: bool,
    /// the home directory for ~/.ssh
    pub home: String,
    /// -i FILE (else ~/.ssh/id_ed25519, ~/.ssh/id_rsa)
    pub identity: Option<String>,
    /// BatchMode=yes: no questions
    pub batch: bool,
    pub strict: Strict,
    /// UserKnownHostsFile
    pub known: Option<String>,
    /// -c
    pub cipher: Option<String>,
    /// -q
    pub quiet: bool,
    pub scp: Option<ScpJob>,
}

/// what an scp does over its connection
#[derive(Clone, Debug)]
pub struct ScpJob {
    /// the file on this device
    pub local: String,
    pub remote: String,
    pub upload: bool,
    stage: u8,
    buf: Vec<u8>,
    size: usize,
}

impl ScpJob {
    pub fn new(local: String, remote: String, upload: bool) -> ScpJob {
        ScpJob { local, remote, upload, stage: 0, buf: Vec::new(), size: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Start,
    Transport,
    /// the question about a new host key waits for a line
    AskHost,
    Auth,
    /// a password is typed
    Password,
    Session,
    Done,
}

/// the console program of ssh and scp
#[derive(Clone, Debug)]
pub struct SshClient {
    o: ClientOpts,
    addr: Option<IpAddr>,
    lookup: Option<u32>,
    sock: Option<u32>,
    core: Option<Core>,
    stage: Stage,
    channel: Option<u32>,
    status: Option<i32>,
    pending: Vec<u8>,
    tries: u8,
    /// the keys it may still try
    keys: Vec<KeyPair>,
    /// the methods the server named
    methods: Vec<String>,
    /// the far end asks for a password
    hide: bool,
    skip_nl: bool,
    /// the host key that waits for an answer
    asked: Option<(String, Vec<u8>)>,
    tail: Vec<u8>,
    how: &'static str,
}

impl SshClient {
    pub fn new(o: ClientOpts) -> SshClient {
        SshClient { o, addr: None, lookup: None, sock: None, core: None, stage: Stage::Start, channel: None, status: None, pending: Vec::new(), tries: 0, keys: Vec::new(), methods: Vec::new(), hide: false, skip_nl: false, asked: None, tail: Vec::new(), how: "" }
    }

    /// the far end hides what is typed (a password)
    pub fn hides(&self) -> bool {
        self.stage == Stage::Password || self.hide
    }
    /// a session runs: Ctrl+C travels to the far end
    pub fn in_session(&self) -> bool {
        self.stage == Stage::Session
    }

    fn peer(&self) -> IpAddr {
        self.addr.unwrap_or(IpAddr::V4(Ipv4::UNSPECIFIED))
    }

    fn connect(&mut self, os: &mut OsApi) {
        let Some(addr) = self.addr else {
            return;
        };
        match os.tcp_connect(addr, self.o.port, SockOpts { nodelay: true, ..SockOpts::default() }) {
            Ok(s) => self.sock = Some(s),
            Err(_) => self.fail_connect(os, "Network is unreachable"),
        }
    }

    fn fail_connect(&mut self, os: &mut OsApi, why: &str) {
        let (to, port) = (self.peer(), self.o.port);
        os.note(Note::Ssh(SshNote::Unreachable { to, port, why: String::from(why) }));
        if self.o.ios {
            os.print(&format!("% {}\n", why));
        } else {
            os.print_err(&format!("ssh: connect to host {} port {}: {}\n", self.o.host, port, why));
        }
        self.stage = Stage::Done;
        os.exit(255);
    }

    /// ends the program: its connection closes
    fn finish(&mut self, os: &mut OsApi, code: i32) {
        if self.stage == Stage::Done {
            return;
        }
        self.flush(os);
        if let Some(s) = self.sock.take() {
            os.tcp_close(s);
        }
        self.stage = Stage::Done;
        os.note(Note::Ssh(SshNote::ClientEnd { to: self.peer(), status: code }));
        os.exit(code);
    }

    fn flush(&mut self, os: &mut OsApi) {
        let (Some(s), Some(core)) = (self.sock, self.core.as_mut()) else {
            return;
        };
        let out = core.take_out();
        self.pending.extend_from_slice(&out);
        if !self.pending.is_empty() {
            let n = os.tcp_send(s, &self.pending);
            self.pending.drain(..n);
        }
    }

    /// what the core reported
    fn pump(&mut self, os: &mut OsApi) {
        loop {
            let Some(core) = self.core.as_mut() else {
                return;
            };
            let evs = core.take_events();
            self.flush(os);
            if evs.is_empty() {
                return;
            }
            for e in evs {
                self.on_core(e, os);
            }
        }
    }

    fn with_core(&mut self, f: impl FnOnce(&mut Core)) {
        if let Some(c) = self.core.as_mut() {
            f(c);
        }
    }

    fn known_path(&self) -> String {
        self.o.known.clone().unwrap_or_else(|| format!("{}/.ssh/known_hosts", self.o.home))
    }

    /// the name under which known_hosts keeps the host
    fn known_name(&self) -> String {
        if self.o.port == ssh::PORT { self.o.host.clone() } else { format!("[{}]:{}", self.o.host, self.o.port) }
    }

    /// the verdict over a host key: known_hosts of the Linux client, nothing on an IOS-like one
    fn host_key(&mut self, alg: String, blob: Vec<u8>, os: &mut OsApi) {
        if self.o.ios || self.o.strict == Strict::No {
            self.with_core(|c| c.host_key_verdict(true));
            return;
        }
        let name = self.known_name();
        let key_type = String::from(ssh::blob_type(&blob));
        let fp = ssh::fingerprint(&blob);
        let text = os.file_read(&self.known_path()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut other = None;
        for (i, l) in text.lines().enumerate() {
            let mut w = l.splitn(2, char::is_whitespace);
            let (Some(names), Some(rest)) = (w.next(), w.next()) else {
                continue;
            };
            if !names.split(',').any(|n| n == name) {
                continue;
            }
            let Some(p) = ssh::parse_public(rest) else {
                continue;
            };
            if p.blob == blob {
                self.with_core(|c| c.host_key_verdict(true));
                return;
            }
            if p.key_type == key_type || other.is_none() {
                other = Some(i + 1);
            }
        }
        if let Some(line) = other {
            os.print_err(&format!(
                "The key of {} is not the one remembered in {} (line {}).\nThe host may have a new key, or someone may be listening in. Its key now: {}.\nAfter checking, ssh-keygen -R {} forgets the old one.\nRefused: the host key changed.\n",
                name,
                self.known_path(),
                line,
                fp,
                name
            ));
            os.note(Note::Ssh(SshNote::HostKeyChanged { host: name, fingerprint: fp }));
            self.with_core(|c| c.host_key_verdict(false));
            self.flush(os);
            self.finish(os, 255);
            return;
        }
        match self.o.strict {
            Strict::AcceptNew | Strict::No => {
                self.remember(&name, &key_type, &blob, os);
                self.with_core(|c| c.host_key_verdict(true));
            }
            Strict::Ask if !self.o.batch => {
                let _ = alg;
                os.print(&format!("{} is new here. Its {} key has the fingerprint {}.\nRemember it and go on? Type yes or no: ", name, key_type, fp));
                self.asked = Some((key_type, blob));
                self.stage = Stage::AskHost;
            }
            _ => {
                os.print_err(&format!("Refused: no key of {} is remembered in {}.\n", name, self.known_path()));
                os.note(Note::Ssh(SshNote::HostKeyRefused { host: name, fingerprint: fp }));
                self.with_core(|c| c.host_key_verdict(false));
                self.flush(os);
                self.finish(os, 255);
            }
        }
    }

    fn remember(&mut self, name: &str, key_type: &str, blob: &[u8], os: &mut OsApi) {
        let path = self.known_path();
        if let Some(dir) = path.rsplit_once('/').map(|x| x.0) {
            let _ = os.file_mkdir(dir, true);
        }
        let _ = os.file_append(&path, format!("{} {}\n", name, ssh::public_line(key_type, blob, "")).as_bytes());
        os.note(Note::Ssh(SshNote::HostKeyNew { host: String::from(name), fingerprint: ssh::fingerprint(blob) }));
        if !self.o.quiet {
            os.print_err(&format!("Remembered {} in {}.\n", name, path));
        }
    }

    /// the user's keys: -i FILE, else the usual files
    fn load_keys(&mut self, os: &mut OsApi) {
        let paths = match &self.o.identity {
            Some(f) => alloc::vec![f.clone()],
            None => alloc::vec![format!("{}/.ssh/id_ed25519", self.o.home), format!("{}/.ssh/id_rsa", self.o.home)],
        };
        for p in paths {
            if let Ok(b) = os.file_read(&p)
                && let Some((k, _)) = KeyPair::parse_private(&String::from_utf8_lossy(&b))
            {
                self.keys.push(k);
            }
        }
    }

    /// the next login to try after the server named its methods
    fn next_login(&mut self, os: &mut OsApi) {
        let user = self.o.user.clone();
        if self.methods.iter().any(|m| m == "publickey") && !self.keys.is_empty() {
            let k = self.keys.remove(0);
            self.how = "publickey";
            self.with_core(|c| c.auth_key(&user, &k));
            return;
        }
        if self.methods.iter().any(|m| m == "password") && !self.o.batch && self.tries < 3 {
            self.ask_password(os);
            return;
        }
        let (to, host) = (self.peer(), self.o.host.clone());
        os.note(Note::Ssh(SshNote::ClientDenied { to, user: user.clone() }));
        if self.o.ios {
            os.print("% Authentication failed\n");
        } else {
            os.print_err(&format!("ssh: the login as {} at {} was refused\n", user, host));
        }
        self.with_core(|c| c.disconnect(ssh::DISC_NO_MORE_AUTH_METHODS, "no more ways to log in"));
        self.finish(os, 255);
    }

    fn ask_password(&mut self, os: &mut OsApi) {
        self.stage = Stage::Password;
        if self.o.ios {
            os.print("Password: ");
        } else {
            os.print(&format!("Password for {} at {}: ", self.o.user, self.o.host));
        }
    }

    fn on_core(&mut self, e: Event, os: &mut OsApi) {
        match e {
            Event::HostKey { alg, blob } => {
                self.host_key(alg, blob, os);
                self.flush(os);
            }
            Event::AuthReady => {
                self.stage = Stage::Auth;
                self.load_keys(os);
                let user = self.o.user.clone();
                self.how = "none";
                self.with_core(|c| c.auth_none(&user));
            }
            Event::Banner(t) => {
                if !self.o.quiet {
                    os.print(&t);
                }
            }
            Event::AuthFailure { methods, .. } => {
                self.methods = methods;
                if self.how == "password" {
                    self.tries += 1;
                    if self.tries < 3 {
                        os.print_err(if self.o.ios { "% Access denied\n" } else { "That password was refused.\n" });
                    }
                }
                self.next_login(os);
            }
            Event::AuthSuccess => {
                let (to, user, how) = (self.peer(), self.o.user.clone(), String::from(self.how));
                os.note(Note::Ssh(SshNote::ClientLogin { to, user, how }));
                self.stage = Stage::Session;
                let id = self.core.as_mut().map(|c| c.open_session());
                self.channel = id;
            }
            Event::ChannelOpened { channel } => self.channel_opened(channel),
            Event::ChannelRefused { text, .. } => {
                os.print_err(&format!("ssh: the server refused a session: {}\n", text));
                self.with_core(|c| c.disconnect(ssh::DISC_BY_APPLICATION, "no session"));
                self.finish(os, 255);
            }
            Event::Data { ext: Some(_), data, .. } => {
                let t = String::from_utf8_lossy(&data).into_owned();
                os.print_err(&t.replace("\r\n", "\n"));
            }
            Event::Data { data, .. } => self.on_data(&data, os),
            Event::ChannelRequest { request: Request::ExitStatus(n), .. } => self.status = Some(n as i32),
            Event::ChannelRequest { want_reply: true, channel, .. } => self.with_core(|c| c.reply(channel, false)),
            Event::Closed { .. } => {
                self.with_core(|c| c.disconnect(ssh::DISC_BY_APPLICATION, "disconnected by user"));
                if self.o.cmd.is_none() && self.o.scp.is_none() && !self.o.quiet {
                    if self.o.ios {
                        os.print(&format!("\n% Session with {} ended\n", self.o.host));
                    } else {
                        os.print_err(&format!("Session with {} ended.\n", self.o.host));
                    }
                }
                let code = self.status.unwrap_or(255);
                self.finish(os, code);
            }
            Event::Disconnected { text, .. } => {
                if self.stage != Stage::Done {
                    let t = if self.o.ios { format!("% The server closed the connection: {}\n", text) } else { format!("ssh: {} closed the connection: {}\n", self.o.host, text) };
                    os.print_err(&t);
                    let code = self.status.unwrap_or(255);
                    self.finish(os, code);
                }
            }
            Event::Failed { text, .. } => {
                os.print_err(&format!("ssh: the connection broke: {}\n", text));
                self.finish(os, 255);
            }
            _ => {}
        }
    }

    fn channel_opened(&mut self, ch: u32) {
        let (cmd, scp) = (self.o.cmd.clone(), self.o.scp.as_ref().map(|s| (s.upload, s.remote.clone())));
        self.with_core(|c| {
            if let Some((up, remote)) = scp {
                c.request(ch, &Request::Exec(format!("scp {} {}", if up { "-t" } else { "-f" }, remote)), true);
                if !up {
                    c.send(ch, &[0]);
                }
            } else if let Some(cmd) = cmd {
                c.request(ch, &Request::Exec(cmd), true);
            } else {
                c.request(ch, &Request::Pty { term: String::from("xterm"), cols: 80, rows: 24 }, true);
                c.request(ch, &Request::Shell, true);
            }
        });
    }

    /// what the remote printed
    fn on_data(&mut self, data: &[u8], os: &mut OsApi) {
        if self.o.scp.is_some() {
            self.scp_data(data, os);
            return;
        }
        let mut b: &[u8] = data;
        if self.skip_nl && !b.is_empty() {
            self.skip_nl = false;
            if b[0] == b'\n' || b.starts_with(b"\r\n") {
                b = &b[if b[0] == b'\n' { 1 } else { 2 }..];
            }
        }
        // line ends of the terminal: a CR LF split between two pieces waits
        let mut out = core::mem::take(&mut self.tail);
        out.extend_from_slice(b);
        if out.last() == Some(&b'\r') {
            out.pop();
            self.tail.push(b'\r');
        }
        let mut text = String::from_utf8_lossy(&out).into_owned();
        if text.contains('\r') {
            text = text.replace("\r\n", "\n");
        }
        if !text.is_empty() {
            let last = text.rsplit('\n').next().unwrap_or("");
            self.hide = last.ends_with("assword: ");
            os.print(&text);
        }
    }

    /// a line typed while it runs
    fn on_line(&mut self, line: &str, os: &mut OsApi) {
        match self.stage {
            Stage::AskHost => match line.trim() {
                "yes" => {
                    let Some((kt, blob)) = self.asked.take() else {
                        return;
                    };
                    let name = self.known_name();
                    self.remember(&name, &kt, &blob, os);
                    self.stage = Stage::Transport;
                    self.with_core(|c| c.host_key_verdict(true));
                }
                "no" => {
                    let (name, fp) = (self.known_name(), self.asked.take().map_or_else(String::new, |x| ssh::fingerprint(&x.1)));
                    os.print_err("Refused: the host key was not taken.\n");
                    os.note(Note::Ssh(SshNote::HostKeyRefused { host: name, fingerprint: fp }));
                    self.stage = Stage::Transport;
                    self.with_core(|c| c.host_key_verdict(false));
                    self.finish(os, 255);
                }
                _ => os.print("Type yes or no: "),
            },
            Stage::Password => {
                let (user, pw) = (self.o.user.clone(), String::from(line));
                self.stage = Stage::Auth;
                self.how = "password";
                self.with_core(|c| c.auth_password(&user, &pw));
            }
            Stage::Session => {
                if line.trim() == "~." {
                    self.with_core(|c| c.disconnect(ssh::DISC_BY_APPLICATION, "disconnected by user"));
                    os.print_err("Closed from this side.\n");
                    self.finish(os, 255);
                    return;
                }
                if self.o.cmd.is_some() || self.o.scp.is_some() {
                    return;
                }
                if line == "\u{3}" {
                    if let Some(ch) = self.channel {
                        self.with_core(|c| c.send(ch, &[3]));
                    }
                    return;
                }
                self.skip_nl = core::mem::take(&mut self.hide);
                if let Some(ch) = self.channel {
                    let mut b = Vec::from(line.as_bytes());
                    b.push(b'\n');
                    self.with_core(|c| c.send(ch, &b));
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ scp, the client's side

    fn scp_data(&mut self, data: &[u8], os: &mut OsApi) {
        let Some(mut job) = self.o.scp.take() else {
            return;
        };
        job.buf.extend_from_slice(data);
        let ch = self.channel;
        let mut send: Vec<u8> = Vec::new();
        let mut fail = None;
        let mut done = false;
        let local_name = String::from(job.local.rsplit('/').next().unwrap_or("file"));
        if job.upload {
            // the far end's 0s: "go on"
            while let Some(&b) = job.buf.first() {
                if b != 0 {
                    fail = Some(String::from(String::from_utf8_lossy(&job.buf).trim_start_matches('\u{1}').trim_end()));
                    job.buf.clear();
                    break;
                }
                job.buf.remove(0);
                match job.stage {
                    0 => match os.file_read(&job.local) {
                        Ok(d) => {
                            send.extend_from_slice(format!("C0644 {} {}\n", d.len(), local_name).as_bytes());
                            job.size = d.len();
                            job.stage = 1;
                        }
                        Err(e) => fail = Some(format!("{}: {}", job.local, e.errno_text())),
                    },
                    1 => {
                        if let Ok(d) = os.file_read(&job.local) {
                            send.extend_from_slice(&d);
                        }
                        send.push(0);
                        job.stage = 2;
                    }
                    _ => done = true,
                }
            }
        } else {
            loop {
                if job.stage == 0 {
                    if job.buf.first() == Some(&1) {
                        fail = Some(String::from(String::from_utf8_lossy(&job.buf).trim_start_matches('\u{1}').trim_end()));
                        job.buf.clear();
                        break;
                    }
                    let Some(nl) = job.buf.iter().position(|&b| b == b'\n') else {
                        break;
                    };
                    let line: Vec<u8> = job.buf.drain(..=nl).collect();
                    let text = String::from_utf8_lossy(&line).into_owned();
                    job.size = text.split_whitespace().nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
                    job.stage = 1;
                    send.push(0);
                } else if job.stage == 1 {
                    if job.buf.len() < job.size + 1 {
                        break;
                    }
                    let bytes: Vec<u8> = job.buf.drain(..job.size).collect();
                    job.buf.remove(0);
                    let is_dir = os.files().is_some_and(|s| s.is_dir(&job.local));
                    let name = job.remote.rsplit('/').next().unwrap_or("file");
                    let target = if is_dir { format!("{}/{}", job.local.trim_end_matches('/'), name) } else { job.local.clone() };
                    match os.file_write(&target, &bytes) {
                        Ok(_) => {
                            send.push(0);
                            done = true;
                            job.stage = 2;
                        }
                        Err(e) => fail = Some(format!("{}: {}", target, e.errno_text())),
                    }
                    break;
                } else {
                    break;
                }
            }
        }
        if let Some(ch) = ch {
            self.with_core(|c| {
                if !send.is_empty() {
                    c.send(ch, &send);
                }
                if done || fail.is_some() {
                    c.eof(ch);
                }
            });
        }
        let (size, upload, file) = (job.size, job.upload, job.local.clone());
        self.o.scp = Some(job);
        if let Some(why) = fail {
            os.print_err(&format!("scp: {}\n", why));
            self.status = Some(1);
        } else if done {
            let peer = self.peer();
            os.note(Note::Ssh(SshNote::Scp { peer, file, bytes: size, upload }));
            if !self.o.quiet {
                os.print(&format!("{}: {} bytes copied\n", local_name, size));
            }
        }
    }
}

impl Service for SshClient {
    fn kind(&self) -> &'static str {
        "ssh"
    }
    fn start(&mut self, os: &mut OsApi) {
        match IpAddr::parse(self.o.host.trim_start_matches('[').trim_end_matches(']')) {
            Some(a) => {
                self.addr = Some(a);
                self.connect(os);
            }
            None => self.lookup = Some(os.resolve_host(&self.o.host, Family::Any)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(l) if Some(l) == self.lookup => {
                let all = os.lookup(l).and_then(|x| x.result.clone()).and_then(|r| r.ok()).map(|r| r.sorted).unwrap_or_default();
                match all.first() {
                    Some(&a) => {
                        self.addr = Some(a);
                        self.connect(os);
                    }
                    None => {
                        if self.o.ios {
                            os.print(&format!("% Bad IP address or host name: {}\n", self.o.host));
                        } else {
                            os.print_err(&format!("ssh: Could not resolve hostname {}\n", self.o.host));
                        }
                        self.stage = Stage::Done;
                        os.exit(255);
                    }
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.sock => {
                let (to, port) = (self.peer(), self.o.port);
                os.note(Note::Ssh(SshNote::Connected { to, port }));
                let mut cfg = Config { ident: String::from(IDENT), ..Config::default() };
                if let Some(c) = &self.o.cipher {
                    cfg.ciphers = alloc::vec![c.clone()];
                }
                self.core = Some(Core::client(cfg, &seed_of(os)));
                self.stage = Stage::Transport;
                self.pump(os);
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.with_core(|c| c.push(&d));
                self.pump(os);
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.sock => self.flush(os),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, end: TcpEnd::Closed } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.with_core(|c| c.push(&d));
                self.pump(os);
                if self.stage != Stage::Done {
                    if !self.o.quiet {
                        os.print_err(&if self.o.ios { format!("\n% Connection closed by {}\n", self.o.host) } else { format!("ssh: connection to {} closed\n", self.o.host) });
                    }
                    let code = self.status.unwrap_or(255);
                    self.finish(os, code);
                }
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.sock => {
                self.sock = None;
                if self.stage == Stage::Start {
                    let why = match end {
                        TcpEnd::Refused => "Connection refused",
                        TcpEnd::Unreachable => "No route to host",
                        _ => "Connection timed out",
                    };
                    self.fail_connect(os, why);
                } else if self.stage != Stage::Done {
                    os.print_err(&format!("ssh: connection to {} lost\n", self.o.host));
                    self.stage = Stage::Done;
                    os.exit(255);
                }
            }
            SvcEvent::Console(ConsoleEv::Line(l)) => {
                self.on_line(&l, os);
                self.pump(os);
            }
            SvcEvent::Console(ConsoleEv::Eof) => {
                if let Some(ch) = self.channel {
                    self.with_core(|c| c.eof(ch));
                    self.pump(os);
                }
            }
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        format!("ssh {} {}", self.o.host, self.o.port)
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
        self.core.as_ref().map_or(0, |c| c.footprint()) + self.pending.len() + self.o.home.len() + 512
    }
}

impl Engine {
    /// the program of a console job is an SSH client whose far end hides what is typed
    pub(crate) fn ssh_prog_hides(&self, ni: usize, job: &Option<CliJob>) -> bool {
        let Some(CliJob { out: JobOut::Proc { svc, stdin: true }, .. }) = job else {
            return false;
        };
        self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == *svc)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<SshClient>()).is_some_and(|t| t.hides())
    }

    /// Ctrl+C with an SSH session in the foreground: it travels to the far end -> it did
    pub(crate) fn ssh_interrupt(&mut self, ni: usize) -> bool {
        let Some(CliJob { out: JobOut::Proc { svc, stdin: true }, .. }) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return false;
        };
        let running = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == svc)).and_then(|s| s.svc.as_ref()).and_then(|x| x.as_any().downcast_ref::<SshClient>()).is_some_and(|t| t.in_session());
        if running {
            self.console_input(ni, svc, ConsoleEv::Line(String::from("\u{3}")));
        }
        running
    }
}
