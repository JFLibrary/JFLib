// ftpd -- the FTP server (RFC 959; EPSV and EPRT of RFC 2428, FEAT of RFC 2389, SIZE and MDTM of
// RFC 3659): "ftp-server" on TCP port 21 serves the files below a root directory of the
// device's store (files.rs), as vsftpd with its users shut into their home directories
//
//   config (JSON, every member may be left out)
//     {"port": 21, "root": "/srv/ftp", "anonymous": true, "anon_write": false,
//      "users": [{"name": "anna", "password": "geheim", "home": "/srv/ftp/anna", "write": true}],
//      "active": true, "quota_kb": 0, "fail_delay_ms": 1000, "max_fails": 3, "idle_ms": 300000,
//      "banner": "JFNetworker FTP server ready."}
//   FtpServer::from_config(&config)
//
// A session starts with 220 and the banner; USER and PASS log in (anonymous and ftp take any
// password when "anonymous" is on, a user without "home" lives in the root).  A wrong password
// is answered after fail_delay_ms with 530, and the connection ends after max_fails of them.
// Paths are the client's view: "/" is the home, ".." stops there.  Commands: USER, PASS, QUIT,
// SYST, FEAT, OPTS UTF8, NOOP, PWD (XPWD), CWD (XCWD), CDUP (XCUP), TYPE A|I, MODE S, STRU F,
// PASV, EPSV, PORT, EPRT, LIST, NLST, RETR, STOR, APPE, DELE, MKD, RMD, SIZE, MDTM, ABOR; others
// get 502, unknown ones 500, everything but the login before it 530.
//
// Data connections: PASV (IPv4) and EPSV listen on a free port of the device for one
// connection; PORT and EPRT (to the client's own address only) make the server connect from
// port 20 when a transfer starts ("active": false refuses them with 502).  150 goes out when
// the data connection is there, 226 when it is closed after the last byte (or, for an upload,
// after the file is on the disk).  RETR reads the file from the disk first (OsApi::disk_read),
// STOR writes it when the client closed the data connection (disk_write); an upload that grows
// over the quota of the root or the device's space ends with 552 and no file.  LIST answers
// in the lines of ls -l, NLST with the names.  The reply texts are our own words with the codes
// and formats of the RFCs.
//
// Metrics: sessions, active, logins, failed_logins, downloads, uploads, listings, errors,
// bytes_sent, bytes_received.  Notes (xfertrace.rs) and log lines (tag "ftpd").

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::files::Stat;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::ftp::{self, Command, Lines, Reply};
use netpkt::IpAddr;

use crate::http::Wheel;
use crate::kernel::{Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::tcp::SockOpts;
use crate::trace::Note;
use crate::xfertrace::XNote;

const K_WHEEL: u32 = 0;
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;
/// what goes to tcp_send at once
const CHUNK: usize = 65536;

/// a user of the server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FtpUser {
    pub name: String,
    pub password: String,
    /// the directory the user lives in (empty: the root)
    pub home: String,
    pub write: bool,
}

/// who is logged in
#[derive(Clone, Debug)]
struct Login {
    name: String,
    home: String,
    write: bool,
}

/// what waits for its data connection
#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    /// send these bytes (RETR, LIST, NLST: the command for the notes and the 150)
    Send { cmd: String, what: String, data: Vec<u8> },
    /// take a file (STOR, APPE) to this path
    Take { cmd: String, path: String, name: String, append: bool },
}

/// a deadline
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// the answer to a wrong password
    Refuse(u32),
    /// a file to send was read from the disk
    Read(u32),
    /// an uploaded file is on the disk
    Stored(u32),
    /// a session said nothing for too long
    Idle(u32),
}

/// one control connection
#[derive(Clone, Debug)]
struct Session {
    peer: IpAddr,
    local: IpAddr,
    lines: Lines,
    user: Option<String>,
    login: Option<Login>,
    fails: u8,
    /// the client's directory (its view: "/" is the home)
    cwd: String,
    binary: bool,
    /// a passive listener and its port
    pasv: Option<(u32, u16)>,
    /// where an active data connection goes
    port_to: Option<(IpAddr, u16)>,
    /// the data connection and whether it is up
    data: Option<u32>,
    data_up: bool,
    job: Option<Job>,
    /// the job runs: bytes still to send, or the bytes that came
    running: bool,
    out: Vec<u8>,
    got: Vec<u8>,
    sent: u64,
    /// the FIN after the last byte of a download went out
    fin: bool,
    /// reply bytes the control connection did not take yet
    ctl: Vec<u8>,
    /// waiting for a wrong password's answer: lines wait meanwhile
    busy: bool,
    queue: VecDeque<String>,
    idle: Option<u64>,
}

/// the FTP server
#[derive(Clone, Debug)]
pub struct FtpServer {
    pub port: u16,
    pub root: String,
    pub anonymous: bool,
    pub anon_write: bool,
    pub users: Vec<FtpUser>,
    pub active_ok: bool,
    pub quota: u64,
    pub fail_delay: Ns,
    pub max_fails: u8,
    pub idle: Ns,
    pub banner: String,
    listener: Option<u32>,
    sessions: BTreeMap<u32, Session>,
    /// data and passive sockets -> their control connection
    owner: BTreeMap<u32, u32>,
    wheel: Wheel<Due>,
    pub root_ok: bool,
    pub sessions_n: u64,
    pub logins: u64,
    pub failed_logins: u64,
    pub downloads: u64,
    pub uploads: u64,
    pub listings: u64,
    pub errors: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

impl Default for FtpServer {
    fn default() -> FtpServer {
        FtpServer::new()
    }
}

/// the client's view of a path: absolute, "." and ".." resolved, never above "/"
fn view_path(cwd: &str, arg: &str) -> String {
    let full = if arg.starts_with('/') { String::from(arg) } else { format!("{}/{}", cwd, arg) };
    let mut parts: Vec<&str> = Vec::new();
    for c in full.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    format!("/{}", parts.join("/"))
}

/// "Jan 01 00:05" of LIST (vsftpd's form)
fn list_date(t: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let c = datetime::Civil::from_unix(if t == 0 { crate::os::EPOCH_UNIX_NS / 1_000_000_000 } else { t });
    format!("{} {:02} {:02}:{:02}", MONTHS[(c.month.clamp(1, 12) - 1) as usize], c.day, c.hour, c.minute)
}

/// one LIST line
fn list_line(name: &str, s: &Stat) -> String {
    let (mode, size) = if s.dir { ("drwxr-xr-x", 4096) } else { ("-rw-r--r--", s.size) };
    format!("{}    1 ftp      ftp      {:>8} {} {}\r\n", mode, size, list_date(s.mtime), name)
}

impl FtpServer {
    pub fn new() -> FtpServer {
        FtpServer {
            port: ftp::PORT_FTP,
            root: String::from("/srv/ftp"),
            anonymous: true,
            anon_write: false,
            users: Vec::new(),
            active_ok: true,
            quota: 0,
            fail_delay: SEC,
            max_fails: 3,
            idle: 300 * SEC,
            banner: String::from("JFNetworker FTP server ready."),
            listener: None,
            sessions: BTreeMap::new(),
            owner: BTreeMap::new(),
            wheel: Wheel::new(K_WHEEL),
            root_ok: true,
            sessions_n: 0,
            logins: 0,
            failed_logins: 0,
            downloads: 0,
            uploads: 0,
            listings: 0,
            errors: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    /// the server of a project's definition (svcact.rs)
    pub fn from_config(c: &Value) -> Result<FtpServer, Msg> {
        let mut s = FtpServer::new();
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        let flag = |k: &str| c.get(k).and_then(|v| v.as_bool());
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(|x| String::from(x.trim()));
        if let Some(p) = num("port") {
            s.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        if let Some(r) = text("root") {
            if r.is_empty() {
                return Err(msg!("\u{201e}root\u{201c} ist kein Verzeichnis."));
            }
            s.root = r;
        }
        s.anonymous = flag("anonymous").unwrap_or(true);
        s.anon_write = flag("anon_write").unwrap_or(false);
        s.active_ok = flag("active").unwrap_or(true);
        if let Some(q) = c.get("quota_kb").and_then(|v| crate::httpsvc::decimal(v, 1024)) {
            s.quota = q;
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
        for u in c.get("users").and_then(|u| u.as_array()).unwrap_or(&[]) {
            let name = u.get("name").and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(msg!("Ein Benutzer braucht \u{201e}name\u{201c} ohne Leerzeichen."));
            }
            s.users.push(FtpUser {
                name: String::from(name),
                password: String::from(u.get("password").and_then(|v| v.as_str()).unwrap_or("")),
                home: String::from(u.get("home").and_then(|v| v.as_str()).unwrap_or("").trim()),
                write: u.get("write").and_then(|v| v.as_bool()).unwrap_or(true),
            });
        }
        Ok(s)
    }

    fn root_path(&self, os: &OsApi) -> Option<String> {
        os.file_norm(&self.root).ok()
    }

    /// the device's path of a client's path
    fn real(&self, os: &OsApi, sock: u32, view: &str) -> Option<String> {
        let s = self.sessions.get(&sock)?;
        let home = &s.login.as_ref()?.home;
        let joined = if view == "/" { home.clone() } else { format!("{}{}", home.trim_end_matches('/'), view) };
        os.file_norm(&joined).ok()
    }

    /// a reply on a control connection
    fn reply(&mut self, os: &mut OsApi, sock: u32, r: Reply) {
        let b = r.to_bytes();
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if !s.ctl.is_empty() {
            // behind what waits already
            s.ctl.extend_from_slice(&b);
            return;
        }
        let n = os.tcp_send(sock, &b);
        // the rest after TcpWritable (a control connection rarely fills its buffer)
        s.ctl.extend_from_slice(&b[n..]);
    }

    fn say(&mut self, os: &mut OsApi, sock: u32, code: u16, text: &str) {
        self.reply(os, sock, Reply::new(code, text));
    }

    /// the idle deadline of a session starts again
    fn touch(&mut self, os: &mut OsApi, sock: u32) {
        let at = os.now() + self.idle;
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if let Some(id) = s.idle.take() {
            self.wheel.cancel(id);
        }
        s.idle = Some(self.wheel.add(at, Due::Idle(sock)));
    }

    /// a session ends: its sockets go
    fn end(&mut self, os: &mut OsApi, sock: u32) {
        if let Some(s) = self.sessions.remove(&sock) {
            if let Some(id) = s.idle {
                self.wheel.cancel(id);
            }
            if let Some((l, _)) = s.pasv {
                self.owner.remove(&l);
                os.tcp_close(l);
            }
            if let Some(d) = s.data {
                self.owner.remove(&d);
                os.tcp_close(d);
            }
        }
        os.tcp_close(sock);
    }

    fn close_data(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if let Some(d) = s.data.take() {
            self.owner.remove(&d);
            os.tcp_close(d);
        }
        if let Some((l, _)) = s.pasv.take() {
            self.owner.remove(&l);
            os.tcp_close(l);
        }
        s.data_up = false;
        s.running = false;
        s.job = None;
        s.out.clear();
        s.got.clear();
        s.sent = 0;
        s.fin = false;
    }

    // ---------------------------------------------------------------- commands

    fn lines(&mut self, os: &mut OsApi, sock: u32, data: &[u8]) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        let ls = s.lines.push(data);
        for l in ls {
            let busy = self.sessions.get(&sock).is_some_and(|s| s.busy);
            if busy {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.queue.push_back(l);
                }
                continue;
            }
            self.command(os, sock, &l);
            if !self.sessions.contains_key(&sock) {
                return;
            }
        }
    }

    fn command(&mut self, os: &mut OsApi, sock: u32, line: &str) {
        let Some(c) = Command::parse(line) else { return };
        self.touch(os, sock);
        let logged = self.sessions.get(&sock).is_some_and(|s| s.login.is_some());
        let v = c.verb.as_str();
        match v {
            "USER" => self.user(os, sock, &c.arg),
            "PASS" => self.pass(os, sock, &c.arg),
            "QUIT" => {
                self.say(os, sock, 221, "Goodbye.");
                os.tcp_shutdown(sock);
            }
            "SYST" => self.say(os, sock, 215, "UNIX Type: L8"),
            "FEAT" => {
                let r = Reply { code: 211, lines: ["Extensions:", " EPRT", " EPSV", " MDTM", " PASV", " SIZE", " UTF8", "End"].iter().map(|x| String::from(*x)).collect() };
                self.reply(os, sock, r);
            }
            "OPTS" if c.arg.to_ascii_uppercase().starts_with("UTF8") => self.say(os, sock, 200, "UTF-8 is always on."),
            "NOOP" => self.say(os, sock, 200, "Still here."),
            _ if !logged => self.say(os, sock, 530, "Log in with USER and PASS first."),
            "PWD" | "XPWD" => {
                let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
                self.say(os, sock, 257, &format!("\"{}\" is the current directory.", cwd));
            }
            "CWD" | "XCWD" | "CDUP" | "XCUP" => {
                let arg = if v.ends_with("UP") { String::from("..") } else { c.arg.clone() };
                let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
                let view = view_path(&cwd, &arg);
                let ok = self.real(os, sock, &view).is_some_and(|p| os.file_stat(&p).is_some_and(|s| s.dir));
                if ok {
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.cwd = view;
                    }
                    self.say(os, sock, 250, "Directory changed.");
                } else {
                    self.refused(os, sock, &format!("{} {}", v, c.arg), 550, "No such directory.");
                }
            }
            "TYPE" => {
                let t = c.arg.trim().to_ascii_uppercase();
                let bin = match t.as_str() {
                    "I" | "L 8" => Some(true),
                    "A" | "A N" => Some(false),
                    _ => None,
                };
                match bin {
                    Some(b) => {
                        if let Some(s) = self.sessions.get_mut(&sock) {
                            s.binary = b;
                        }
                        self.say(os, sock, 200, if b { "Binary mode (type I)." } else { "Text mode (type A)." });
                    }
                    None => self.say(os, sock, 504, "That type is not supported."),
                }
            }
            "MODE" | "STRU" => {
                let ok = (v == "MODE" && c.arg.eq_ignore_ascii_case("S")) || (v == "STRU" && c.arg.eq_ignore_ascii_case("F"));
                if ok { self.say(os, sock, 200, "Fine.") } else { self.say(os, sock, 504, "Only stream mode and file structure.") }
            }
            "PASV" | "EPSV" => self.passive(os, sock, v == "EPSV", &c.arg),
            "PORT" | "EPRT" => self.active(os, sock, v == "EPRT", &c.arg),
            "LIST" | "NLST" => self.list(os, sock, v == "NLST", &c.arg),
            "RETR" => self.retr(os, sock, &c.arg),
            "STOR" | "APPE" => self.stor(os, sock, v == "APPE", &c.arg),
            "DELE" | "RMD" | "XRMD" | "MKD" | "XMKD" => self.change(os, sock, v, &c.arg),
            "SIZE" | "MDTM" => {
                let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
                let st = self.real(os, sock, &view_path(&cwd, &c.arg)).and_then(|p| os.file_stat(&p)).filter(|s| !s.dir);
                match st {
                    Some(s) if v == "SIZE" => self.say(os, sock, 213, &format!("{}", s.size)),
                    Some(s) => {
                        let t = datetime::Civil::from_unix(if s.mtime == 0 { crate::os::EPOCH_UNIX_NS / 1_000_000_000 } else { s.mtime });
                        self.say(os, sock, 213, &format!("{:04}{:02}{:02}{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second));
                    }
                    None => self.refused(os, sock, &format!("{} {}", v, c.arg), 550, "No such file."),
                }
            }
            "ABOR" => {
                let running = self.sessions.get(&sock).is_some_and(|s| s.job.is_some());
                if running {
                    self.close_data(os, sock);
                    self.say(os, sock, 426, "Transfer aborted.");
                }
                self.say(os, sock, 226, "Nothing is running now.");
            }
            "ALLO" => self.say(os, sock, 202, "No need to reserve space here."),
            v if ftp::verb_known(v) => self.say(os, sock, 502, "Not implemented here."),
            _ => self.say(os, sock, 500, "Unknown command."),
        }
    }

    fn refused(&mut self, os: &mut OsApi, sock: u32, cmd: &str, code: u16, text: &str) {
        self.errors += 1;
        os.note(Note::Xfer(XNote::FtpRefused { cmd: String::from(cmd), code }));
        self.say(os, sock, code, text);
    }

    fn user(&mut self, os: &mut OsApi, sock: u32, name: &str) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if s.login.is_some() {
            return self.say(os, sock, 530, "Already logged in.");
        }
        s.user = Some(String::from(name));
        let anon = name.eq_ignore_ascii_case("anonymous") || name.eq_ignore_ascii_case("ftp");
        if anon && self.anonymous {
            self.say(os, sock, 331, "Anonymous login: any password will do.");
        } else {
            self.say(os, sock, 331, "Password, please.");
        }
    }

    fn pass(&mut self, os: &mut OsApi, sock: u32, pw: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        if s.login.is_some() {
            return self.say(os, sock, 230, "Already logged in.");
        }
        let Some(name) = s.user.clone() else {
            return self.say(os, sock, 503, "USER first, then PASS.");
        };
        let anon = name.eq_ignore_ascii_case("anonymous") || name.eq_ignore_ascii_case("ftp");
        let root = self.root_path(os).unwrap_or_else(|| self.root.clone());
        let login = if anon && self.anonymous {
            Some(Login { name: String::from("anonymous"), home: root, write: self.anon_write })
        } else {
            self.users.iter().find(|u| u.name == name && u.password == pw).map(|u| Login {
                name: u.name.clone(),
                home: if u.home.is_empty() { root.clone() } else { os.file_norm(&u.home).unwrap_or_else(|_| u.home.clone()) },
                write: u.write,
            })
        };
        let peer = s.peer;
        match login {
            Some(l) => {
                // the home must be there
                if !os.file_stat(&l.home).is_some_and(|st| st.dir) {
                    self.errors += 1;
                    os.log(SEV_WARN, "ftpd", &format!("{} from {}: the home {} is missing", l.name, peer, l.home));
                    return self.say(os, sock, 530, "Your home directory is missing.");
                }
                self.logins += 1;
                os.note(Note::Xfer(XNote::FtpLogin { user: l.name.clone() }));
                os.log(SEV_INFO, "ftpd", &format!("{} logged in from {}", l.name, peer));
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.login = Some(l);
                    s.cwd = String::from("/");
                }
                self.say(os, sock, 230, "Logged in.");
            }
            None => {
                self.failed_logins += 1;
                os.note(Note::Xfer(XNote::FtpLoginFailed { user: name.clone() }));
                os.log(SEV_WARN, "ftpd", &format!("login of {} from {} refused", name, peer));
                let at = os.now() + self.fail_delay;
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.busy = true;
                    s.user = None;
                    s.fails += 1;
                }
                self.wheel.add(at, Due::Refuse(sock));
                self.wheel.arm(os);
            }
        }
    }

    /// the answer to a wrong password, after the delay; the lines that came meanwhile follow
    fn refuse_now(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        s.busy = false;
        let last = s.fails >= self.max_fails;
        self.say(os, sock, 530, "Wrong user name or password.");
        if last {
            self.say(os, sock, 421, "Too many failed logins, goodbye.");
            os.tcp_shutdown(sock);
            return;
        }
        while let Some(l) = self.sessions.get_mut(&sock).and_then(|s| if s.busy { None } else { s.queue.pop_front() }) {
            self.command(os, sock, &l);
            if !self.sessions.contains_key(&sock) {
                return;
            }
        }
    }

    fn passive(&mut self, os: &mut OsApi, sock: u32, ext: bool, arg: &str) {
        let Some(s) = self.sessions.get(&sock) else { return };
        let local = s.local;
        if !ext && !local.is_v4() {
            return self.say(os, sock, 502, "Use EPSV over IPv6.");
        }
        if ext && !arg.is_empty() && !arg.eq_ignore_ascii_case("ALL") && arg != "1" && arg != "2" {
            return self.say(os, sock, 522, "Network protocol not supported, use (1,2).");
        }
        // one listener at a time: a new one replaces the old
        if let Some((l, _)) = self.sessions.get_mut(&sock).and_then(|s| s.pasv.take()) {
            self.owner.remove(&l);
            os.tcp_close(l);
        }
        let Ok(l) = os.tcp_listen_on(local, 0, 1, SockOpts::default()) else {
            return self.say(os, sock, 425, "No free port for a data connection.");
        };
        let port = os.tcp_info(l).map(|r| r.local.1).unwrap_or(0);
        self.owner.insert(l, sock);
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.pasv = Some((l, port));
            s.port_to = None;
        }
        os.note(Note::Xfer(XNote::FtpPassive { port }));
        if ext {
            self.say(os, sock, 229, &format!("Extended passive mode on {}.", ftp::fmt_epsv(port)));
        } else {
            let ip = local.v4().unwrap_or(netpkt::Ipv4::UNSPECIFIED);
            self.say(os, sock, 227, &format!("Passive mode on ({}).", ftp::fmt_host_port(ip, port)));
        }
    }

    fn active(&mut self, os: &mut OsApi, sock: u32, ext: bool, arg: &str) {
        if !self.active_ok {
            return self.say(os, sock, 502, "Active mode is off: use PASV or EPSV.");
        }
        let Some(s) = self.sessions.get(&sock) else { return };
        let peer = s.peer;
        let to = if ext { ftp::eprt(arg) } else { ftp::host_port(arg).map(|(a, p)| (IpAddr::V4(a), p)) };
        match to {
            // the data goes to the client itself (no third party: RFC 2577)
            Some((a, p)) if a == peer && p > 0 => {
                if let Some((l, _)) = self.sessions.get_mut(&sock).and_then(|s| s.pasv.take()) {
                    self.owner.remove(&l);
                    os.tcp_close(l);
                }
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.port_to = Some((a, p));
                }
                self.say(os, sock, 200, "Active mode on.");
            }
            Some(_) => self.refused(os, sock, if ext { "EPRT" } else { "PORT" }, 500, "The data goes to your own address only."),
            None => self.say(os, sock, 501, "That address cannot be read."),
        }
    }

    /// a job waits for its data connection, or starts at once
    fn begin(&mut self, os: &mut OsApi, sock: u32, job: Job) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if s.job.is_some() {
            return self.say(os, sock, 425, "A transfer is running already.");
        }
        if s.pasv.is_none() && s.port_to.is_none() && s.data.is_none() {
            return self.say(os, sock, 425, "Use PASV, EPSV, PORT or EPRT first.");
        }
        s.job = Some(job);
        if s.data_up {
            return self.start(os, sock);
        }
        if let (Some(to), None) = (s.port_to, s.data) {
            // active: the server connects from port 20
            match os.tcp_connect_from(ftp::PORT_FTP_DATA, to.0, to.1, SockOpts::default()) {
                Ok(d) => {
                    self.owner.insert(d, sock);
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.data = Some(d);
                    }
                    os.note(Note::Xfer(XNote::FtpActive { to: to.0, port: to.1 }));
                }
                Err(_) => {
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.job = None;
                    }
                    self.say(os, sock, 425, "The data connection cannot be opened.");
                }
            }
        }
    }

    /// the data connection is up and a job waits: 150 and the bytes (a file is read first)
    fn start(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if s.running || !s.data_up {
            return;
        }
        let Some(job) = s.job.clone() else { return };
        s.running = true;
        match job {
            Job::Send { cmd, what, data } => {
                let n = data.len();
                let mode = if s.binary { "binary" } else { "text" };
                s.out = data;
                let text = format!("Here comes {} ({} bytes, {} mode).", what, n, mode);
                self.say(os, sock, 150, &text);
                if cmd == "RETR" {
                    let at = os.disk_read(n as u64);
                    self.wheel.add(at, Due::Read(sock));
                    self.wheel.arm(os);
                } else {
                    self.push(os, sock);
                }
            }
            Job::Take { name, .. } => {
                let text = format!("Ready for {}.", name);
                self.say(os, sock, 150, &text);
            }
        }
    }

    /// sends what is left of a download on the data connection; its end closes it
    fn push(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        let Some(d) = s.data else { return };
        if !s.running || s.fin {
            return;
        }
        while !s.out.is_empty() {
            let n = os.tcp_send(d, &s.out[..s.out.len().min(CHUNK)]);
            if n == 0 {
                return;
            }
            s.sent += n as u64;
            s.out.drain(..n);
        }
        // everything is queued: FIN after it; 226 when the client closed its side too
        s.fin = true;
        os.tcp_shutdown(d);
    }

    fn list(&mut self, os: &mut OsApi, sock: u32, names: bool, arg: &str) {
        let a = arg.split_whitespace().filter(|w| !w.starts_with('-')).collect::<Vec<_>>().join(" ");
        let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
        let view = view_path(&cwd, &a);
        let Some(p) = self.real(os, sock, &view) else {
            return self.refused(os, sock, "LIST", 550, "No such directory.");
        };
        let entries = match os.file_stat(&p) {
            Some(s) if s.dir => os.file_list(&p).unwrap_or_default(),
            Some(s) => alloc::vec![(String::from(view.rsplit('/').next().unwrap_or("")), s)],
            None => return self.refused(os, sock, if names { "NLST" } else { "LIST" }, 550, "No such file or directory."),
        };
        let mut text = String::new();
        for (n, s) in &entries {
            if names {
                text.push_str(&format!("{}\r\n", n));
            } else {
                text.push_str(&list_line(n, s));
            }
        }
        let cmd = String::from(if names { "NLST" } else { "LIST" });
        self.begin(os, sock, Job::Send { cmd, what: String::from(if names { "the names" } else { "the listing" }), data: text.into_bytes() });
    }

    fn retr(&mut self, os: &mut OsApi, sock: u32, arg: &str) {
        let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
        let view = view_path(&cwd, arg);
        let data = self.real(os, sock, &view).and_then(|p| os.file_read(&p).ok());
        let Some(mut data) = data else {
            return self.refused(os, sock, &format!("RETR {}", arg), 550, "No such file.");
        };
        if self.sessions.get(&sock).is_some_and(|s| !s.binary) {
            data = netpkt::tftp::to_netascii(&data);
        }
        let name = String::from(view.rsplit('/').next().unwrap_or(""));
        self.begin(os, sock, Job::Send { cmd: String::from("RETR"), what: name, data });
    }

    fn stor(&mut self, os: &mut OsApi, sock: u32, append: bool, arg: &str) {
        let write = self.sessions.get(&sock).and_then(|s| s.login.as_ref()).is_some_and(|l| l.write);
        let cmd = format!("{} {}", if append { "APPE" } else { "STOR" }, arg);
        if !write {
            return self.refused(os, sock, &cmd, 550, "Permission denied.");
        }
        let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
        let view = view_path(&cwd, arg);
        let Some(path) = self.real(os, sock, &view).filter(|_| view != "/") else {
            return self.refused(os, sock, &cmd, 553, "That name is not allowed.");
        };
        let parent_ok = os.files().is_some_and(|f| f.is_dir(&f.parent(&path)));
        if !parent_ok || os.file_stat(&path).is_some_and(|s| s.dir || s.builtin) {
            return self.refused(os, sock, &cmd, 553, "That name is not allowed.");
        }
        let name = String::from(view.rsplit('/').next().unwrap_or(""));
        self.begin(os, sock, Job::Take { cmd: String::from(if append { "APPE" } else { "STOR" }), path, name, append });
    }

    fn change(&mut self, os: &mut OsApi, sock: u32, v: &str, arg: &str) {
        let write = self.sessions.get(&sock).and_then(|s| s.login.as_ref()).is_some_and(|l| l.write);
        let cmd = format!("{} {}", v, arg);
        if !write {
            return self.refused(os, sock, &cmd, 550, "Permission denied.");
        }
        let cwd = self.sessions.get(&sock).map(|s| s.cwd.clone()).unwrap_or_default();
        let view = view_path(&cwd, arg);
        let Some(path) = self.real(os, sock, &view).filter(|_| view != "/") else {
            return self.refused(os, sock, &cmd, 550, "Permission denied.");
        };
        let r = match v {
            "DELE" => os.file_remove(&path),
            "MKD" | "XMKD" => os.eng.file_mkdir(os.ni, &path, false),
            _ => os.eng.file_rmdir(os.ni, &path),
        };
        match (r, v) {
            (Ok(_), "DELE") => self.say(os, sock, 250, "Deleted."),
            (Ok(_), "MKD" | "XMKD") => self.say(os, sock, 257, &format!("\"{}\" made.", view)),
            (Ok(_), _) => self.say(os, sock, 250, "Directory removed."),
            (Err(e), _) => self.refused(os, sock, &cmd, 550, e.errno_text()),
        }
    }

    // ---------------------------------------------------------------- data connections

    /// bytes of an upload: they must fit, else 552 and the end of the data connection
    fn take(&mut self, os: &mut OsApi, sock: u32, bytes: Vec<u8>) {
        let root = self.root_path(os);
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        let Some(Job::Take { path, name, append, .. }) = s.job.clone() else { return };
        s.got.extend_from_slice(&bytes);
        let total = s.got.len() as u64;
        self.bytes_received += bytes.len() as u64;
        let fits = os.files().is_some_and(|st| {
            let old = st.stat(&path).map_or(0, |x| x.size);
            let size = if append { old + total } else { total };
            let quota_ok = self.quota == 0 || root.as_ref().is_none_or(|r| st.under(r).saturating_sub(old) + size <= self.quota);
            quota_ok && size.saturating_sub(old) <= st.free()
        });
        if !fits {
            self.errors += 1;
            os.note(Note::Xfer(XNote::FtpFull { file: name.clone() }));
            os.log(SEV_WARN, "ftpd", &format!("{} does not fit: {} bytes", name, total));
            if let Some(d) = self.sessions.get_mut(&sock).and_then(|s| s.data.take()) {
                self.owner.remove(&d);
                os.tcp_abort(d);
            }
            self.close_data(os, sock);
            self.say(os, sock, 552, "Quota or space exceeded: the file is not stored.");
        }
    }

    /// the client closed the data connection of an upload: the file goes to the disk
    fn upload_done(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get(&sock) else { return };
        let n = s.got.len() as u64;
        let at = os.disk_write(n, 0).unwrap_or_else(|| os.now());
        self.wheel.add(at, Due::Stored(sock));
        self.wheel.arm(os);
    }

    fn stored(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        let Some(Job::Take { cmd, path, name, append }) = s.job.clone() else { return };
        let mut data = core::mem::take(&mut s.got);
        if !s.binary {
            data = netpkt::tftp::from_netascii(&data);
        }
        let r = if append { os.file_append(&path, &data) } else { os.file_write(&path, &data) };
        self.close_data(os, sock);
        match r {
            Ok(_) => {
                self.uploads += 1;
                os.note(Note::Xfer(XNote::FtpTransfer { cmd: format!("{} {}", cmd, name), bytes: data.len() as u64 }));
                let peer = self.sessions.get(&sock).map(|s| s.peer);
                os.log(SEV_INFO, "ftpd", &format!("received {} from {}: {} bytes", name, peer.map(|p| format!("{}", p)).unwrap_or_default(), data.len()));
                self.say(os, sock, 226, "File stored.");
            }
            Err(e) => {
                self.errors += 1;
                let code = if e == jfn_model::files::FsErr::NoSpace { 452 } else { 451 };
                self.say(os, sock, code, &format!("The file cannot be stored: {}.", e.errno_text()));
            }
        }
    }

    /// a download's data connection is closed: 226
    fn sent(&mut self, os: &mut OsApi, sock: u32) {
        let Some(s) = self.sessions.get(&sock) else { return };
        let Some(Job::Send { cmd, what, .. }) = s.job.clone() else { return };
        let bytes = s.sent;
        self.bytes_sent += bytes;
        match cmd.as_str() {
            "RETR" => self.downloads += 1,
            _ => self.listings += 1,
        }
        os.note(Note::Xfer(XNote::FtpTransfer { cmd: if cmd == "RETR" { format!("RETR {}", what) } else { cmd.clone() }, bytes }));
        if cmd == "RETR" {
            let peer = s.peer;
            os.log(SEV_INFO, "ftpd", &format!("sent {} to {}: {} bytes", what, peer, bytes));
        }
        self.close_data(os, sock);
        self.say(os, sock, 226, "Transfer complete.");
    }

    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (id, d) in self.wheel.due(now) {
            match d {
                Due::Refuse(s) => self.refuse_now(os, s),
                Due::Read(s) => self.push(os, s),
                Due::Stored(s) => self.stored(os, s),
                Due::Idle(s) => {
                    if self.sessions.get(&s).is_some_and(|x| x.idle == Some(id)) {
                        self.say(os, s, 421, "Nothing happened for too long, goodbye.");
                        self.end(os, s);
                    }
                }
            }
        }
        self.wheel.arm(os);
    }
}

impl Service for FtpServer {
    fn kind(&self) -> &'static str {
        "ftp-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.root_ok = self.root_path(os).is_some_and(|r| os.file_stat(&r).is_some_and(|s| s.dir));
        match os.tcp_listen(self.port, 0, SockOpts::default()) {
            Ok(l) => self.listener = Some(l),
            Err(_) => os.note(Note::PortInUse { port: self.port, svc: String::from("ftp-server") }),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.handle(ev, os);
        // the deadlines as the event left them
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.listener = None;
        self.sessions.clear();
        self.owner.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sessions", self.sessions_n as i64),
            Metric::new("active", self.sessions.len() as i64),
            Metric::new("logins", self.logins as i64),
            Metric::new("failed_logins", self.failed_logins as i64),
            Metric::new("downloads", self.downloads as i64),
            Metric::new("uploads", self.uploads as i64),
            Metric::new("listings", self.listings as i64),
            Metric::new("errors", self.errors as i64),
            Metric::new("bytes_sent", self.bytes_sent as i64),
            Metric::new("bytes_received", self.bytes_received as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.root_ok {
            return msg!("{}: das Verzeichnis fehlt", self.root);
        }
        msg!("Port {}, {}: {} Sitzungen, {} Anmeldungen, {} abgelehnt, {} Downloads, {} Uploads", self.port, self.root, self.sessions_n, self.logins, self.failed_logins, self.downloads, self.uploads)
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

impl FtpServer {
    /// one event of the service
    fn handle(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(c) = os.tcp_accept(listener) {
                    let row = os.tcp_info(c);
                    let (local, peer) = row.map(|r| (r.local.0, r.remote.0)).unwrap_or((crate::os::ANY4, crate::os::ANY4));
                    self.sessions_n += 1;
                    self.root_ok = self.root_path(os).is_some_and(|r| os.file_stat(&r).is_some_and(|s| s.dir));
                    self.sessions.insert(
                        c,
                        Session {
                            peer,
                            local,
                            lines: Lines::new(),
                            user: None,
                            login: None,
                            fails: 0,
                            cwd: String::from("/"),
                            binary: false,
                            pasv: None,
                            port_to: None,
                            data: None,
                            data_up: false,
                            job: None,
                            running: false,
                            out: Vec::new(),
                            got: Vec::new(),
                            sent: 0,
                            fin: false,
                            ctl: Vec::new(),
                            busy: false,
                            queue: VecDeque::new(),
                            idle: None,
                        },
                    );
                    let b = self.banner.clone();
                    self.say(os, c, 220, &b);
                    self.touch(os, c);
                }
                self.wheel.arm(os);
            }
            SvcEvent::TcpAccept { listener, .. } => {
                // a passive data connection
                let Some(&sock) = self.owner.get(&listener) else { return };
                while let Some(d) = os.tcp_accept(listener) {
                    let busy = self.sessions.get(&sock).is_some_and(|s| s.data.is_some());
                    if busy {
                        os.tcp_abort(d);
                        continue;
                    }
                    self.owner.insert(d, sock);
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.data = Some(d);
                        s.data_up = true;
                    }
                }
                // one connection per listener
                self.owner.remove(&listener);
                os.tcp_close(listener);
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.pasv = None;
                }
                self.start(os, sock);
            }
            SvcEvent::TcpConnected { sock: d, .. } => {
                let Some(&sock) = self.owner.get(&d) else { return };
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.data_up = true;
                }
                self.start(os, sock);
            }
            SvcEvent::TcpData { sock, .. } => {
                let bytes = os.tcp_recv(sock, usize::MAX);
                if self.sessions.contains_key(&sock) {
                    self.lines(os, sock, &bytes);
                } else if let Some(&ctrl) = self.owner.get(&sock) {
                    self.take(os, ctrl, bytes);
                }
            }
            SvcEvent::TcpWritable { sock } => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    let n = os.tcp_send(sock, &s.ctl);
                    s.ctl.drain(..n);
                } else if let Some(&ctrl) = self.owner.get(&sock) {
                    self.push(os, ctrl);
                }
            }
            SvcEvent::TcpEof { sock, .. } => {
                if self.sessions.contains_key(&sock) {
                    // the client closed the control connection
                    self.end(os, sock);
                    return;
                }
                let Some(&ctrl) = self.owner.get(&sock) else { return };
                match self.sessions.get(&ctrl).and_then(|s| s.job.clone()) {
                    Some(Job::Take { .. }) => self.upload_done(os, ctrl),
                    Some(Job::Send { .. }) if self.sessions.get(&ctrl).is_some_and(|s| s.fin) => self.sent(os, ctrl),
                    _ => {}
                }
            }
            SvcEvent::TcpClosed { sock, end } => {
                if self.sessions.contains_key(&sock) {
                    self.end(os, sock);
                    return;
                }
                let Some(ctrl) = self.owner.remove(&sock) else {
                    os.tcp_close(sock);
                    return;
                };
                os.tcp_close(sock);
                let job = self.sessions.get(&ctrl).and_then(|s| s.job.clone());
                if let Some(s) = self.sessions.get_mut(&ctrl) {
                    if s.data == Some(sock) {
                        s.data = None;
                    }
                    s.data_up = false;
                }
                match job {
                    Some(Job::Send { .. }) if end == crate::tcp::TcpEnd::Closed && self.sessions.get(&ctrl).is_some_and(|s| s.fin) => self.sent(os, ctrl),
                    Some(Job::Take { .. }) if end == crate::tcp::TcpEnd::Closed && self.sessions.get(&ctrl).is_some_and(|s| s.running) => self.upload_done(os, ctrl),
                    Some(_) => {
                        self.errors += 1;
                        self.close_data(os, ctrl);
                        self.say(os, ctrl, 426, "The data connection broke off.");
                    }
                    None => {}
                }
            }
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            _ => {}
        }
    }
}

impl crate::kernel::Engine {
    /// the root directories (and homes) of a device's FTP servers that are missing
    /// (problems.rs): (service, directory)
    pub fn ftp_missing_dirs(&self, dev: u32) -> Vec<(String, String)> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        let Some(os) = self.os(ni) else { return Vec::new() };
        let mut v = Vec::new();
        for s in os.svcs.iter().filter(|s| s.enabled && !s.console) {
            let Some(f) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<FtpServer>()) else { continue };
            let missing = |d: &str| !os.files.norm(d).ok().is_some_and(|r| os.files.stat(&r).is_some_and(|st| st.dir));
            if missing(&f.root) {
                v.push((s.name.clone(), f.root.clone()));
            }
            for u in f.users.iter().filter(|u| !u.home.is_empty()) {
                if missing(&u.home) {
                    v.push((s.name.clone(), u.home.clone()));
                }
            }
        }
        v
    }
}
