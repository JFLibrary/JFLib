// ses -- the command-line sessions of a device: the console (session 0, always there), local
// terminals the app opens (the Terminal panel's further tabs, the desktop's terminal windows,
// the IDE's terminal pane) and sessions from the network: telnet (telnetd.rs: the vty lines of
// IOS-like devices, telnetd on Linux hosts) and SSH (sshd.rs: the same lines, sshd on Linux hosts)
//
//   SesKind                       Console | Local | Telnet (from the network; Conn::ssh tells SSH)
//   SesInfo                       what the device knows of a session: its id, kind, line name
//                                 ("con 0", "tty 1", "vty 0"; Linux "tty1", "pts/0"), since,
//                                 the last line typed, "terminal monitor", the shell's PID, its
//                                 telnet connection (Conn) and the Linux login of telnetd (Login)
//   SesCli                        a session's command line while another one is in place: mode
//                                 and its context (interfaces, VLANs, line block, pool, access
//                                 list, RADIUS group and server), the running job and the lines
//                                 typed meanwhile, a question, the privilege level, the login
//                                 gate and user, copy's dialog, the shell (directory, variables,
//                                 history, jobs), the wake-up epoch and the prompt last reported
//   SesView                       a session as the app and the tests see it
//   Engine::cli_open(dev) -> Option<u32>   a local session (None: no command line, off)
//   Engine::cli_close(dev, s)              ends session s (the console stays)
//   Engine::cli_sessions(dev)              every session, the console first
//   Engine::cli_ses_line / cli_ses_interrupt / cli_ses_help / cli_ses_complete /
//     cli_ses_prompt / cli_ses_busy / cli_ses_secret / cli_ses_reads / cli_ses_cwd /
//     cli_ses_pager / cli_ses_mode         the console API for session s (help and complete take
//                                          &mut self: the session is put in place for them and
//                                          back, nothing changes)
//   Engine::cli_reads(dev)                 the console's program reads typed lines (telnet, nc):
//                                          the terminal keeps its input line, without a prompt
//   Engine::in_ses(ni, s, f)               f with session s in place (crate)
//   Engine::log_out(ni, text)              console messages and debug output: to the console
//                                          and to every session with "terminal monitor" (crate)
//
// What stays per device (CliDev): the kind of command line, the configuration (cfg), the debug
// flags (debug, ospf_debug, bgp_debug: IOS's debugging is the device's; "terminal monitor"
// decides which other sessions see it), the interface states the console messages compare
// (snap, rcvd_hop), booting, and the sessions with their infos.  The console's messages (%LINK,
// %LINEPROTO, %SYS, debug output) go to the console always and to the sessions that monitor.
//
// One session's command line is in place in CliDev (the fields from `mode` to `sh`, as before
// sessions existed); the others wait in CliDev::parked.  Outside the engine's work session 0 is
// in place, so the console API and every existing caller see the console as they always did.
// Whatever the engine does for another session -- a line typed there, its job's progress, its
// program's output or exit, a wake-up, a RADIUS answer for its login -- runs with that session
// put in place (in_ses: a swap of the fields there and back), so every command works unchanged
// in every session.  Output follows the session in place: the console's Report::CliOut, a local
// session's Report::SesOut, a telnet session's connection.  Wake-ups of a session are
// Ev::Ses { node, ses, epoch }; the SES_MARK bit marks the session's own ones (its start, the
// telnet input, the idle timeout, the time for an SSH login, its end).  Ending a session from
// inside it (exit, an idle timeout, a closed connection) only marks it: it goes at the same
// moment in an event of its own, after the engine left it.  Switching a device off ends every session but the console,
// which starts afresh.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::CliStyle;
use netpkt::telnet::{Options, Reader};
use netpkt::IpAddr;

use super::print::JobOut;
use super::{aaa, files, sh, Ask, CliDev, CliJob, Mode};
use crate::action::ActErr;
use crate::driver::Report;
use crate::kernel::{Engine, Ev, Ns};

/// Ev::Ses epochs with this bit are the session's own wake-ups
pub const SES_MARK: u32 = 0x4000_0000;
const W_KIND: u32 = 0x0F00_0000;
const W_START: u32 = SES_MARK | 0x0100_0000;
const W_END: u32 = SES_MARK | 0x0200_0000;
pub(crate) const W_INPUT: u32 = SES_MARK | 0x0300_0000;
/// | the idle timer's generation (24 bits)
pub(crate) const W_IDLE: u32 = SES_MARK | 0x0400_0000;
/// the time for an SSH login is up (sshd.rs)
pub(crate) const W_SSH: u32 = SES_MARK | 0x0500_0000;

/// what a session is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SesKind {
    /// session 0: IOS "con 0", Linux "tty1"
    Console,
    /// a terminal the app opened: IOS "tty N" (it logs in as line con 0 says), Linux "pts/N"
    Local,
    /// a connection from the network: IOS "vty N", Linux "pts/N" (telnetd.rs; Conn::ssh tells an
    /// SSH one, sshd.rs)
    Telnet,
}

/// the connection of a session from the network: telnet (telnetd.rs) or SSH (sshd.rs)
#[derive(Clone, Debug)]
pub struct Conn {
    /// the server's service and the connection's socket
    pub svc: u32,
    pub sock: u32,
    pub from: IpAddr,
    pub port: u16,
    pub reader: Reader,
    pub opts: Options,
    /// bytes the connection did not take yet
    pub tx: Vec<u8>,
    /// the line typed so far
    pub rx: Vec<u8>,
    /// lines that came and wait for the session
    pub inq: VecDeque<String>,
    /// an Interrupt Process came
    pub intr: bool,
    /// the server asked the client to stop its echo (a password)
    pub hide: bool,
    /// an input wake-up is scheduled
    pub woken: bool,
    /// an SSH connection: the core and its session state (sshd.rs)
    pub ssh: Option<alloc::boxed::Box<crate::sshd::SshConn>>,
}

impl Conn {
    /// what the session writes goes into the connection's bytes
    pub(crate) fn write_text(&mut self, text: &str) {
        match self.ssh.as_mut() {
            Some(x) => {
                x.write(text.as_bytes());
                let out = x.core.take_out();
                self.tx.extend_from_slice(&out);
            }
            None => self.tx.extend_from_slice(&netpkt::telnet::to_nvt(text.as_bytes())),
        }
    }
}

/// the login of a Linux telnetd session ("PC1 login: ", "Password: ")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Login {
    pub user: Option<String>,
    pub tries: u8,
}

/// what the device knows of a session
#[derive(Clone, Debug)]
pub struct SesInfo {
    pub id: u32,
    pub kind: SesKind,
    /// "con 0", "tty 1", "vty 0"; Linux "tty1", "pts/0"
    pub line: String,
    /// the number of its line (vty N, tty N, pts/N; 0 for the console)
    pub no: u16,
    pub since: Ns,
    /// the last line typed
    pub last: Ns,
    /// "terminal monitor": console messages and debug output reach it
    pub monitor: bool,
    /// its shell's PID (ps; the console's is sh::SHELL_PID, IOS-like devices have none)
    pub pid: u32,
    pub conn: Option<Conn>,
    pub login: Option<Login>,
    /// a telnet session's prompt is in the connection already
    pub prompted: bool,
    /// the idle timer's generation
    pub idle_gen: u32,
    /// it ends as soon as the engine left it
    pub ending: bool,
    /// who a Linux session belongs to when it is not root (sshd.rs: another user of sshd)
    pub account: String,
}

impl SesInfo {
    fn new(id: u32, kind: SesKind, line: String, no: u16, now: Ns) -> SesInfo {
        SesInfo { id, kind, line, no, since: now, last: now, monitor: false, pid: 0, conn: None, login: None, prompted: false, idle_gen: 0, ending: false, account: String::new() }
    }
    /// the console's (session 0)
    pub fn console(style: CliStyle) -> SesInfo {
        let line = if style == CliStyle::Ios { "con 0" } else { "tty1" };
        let mut i = SesInfo::new(0, SesKind::Console, String::from(line), 0, 0);
        i.pid = sh::SHELL_PID;
        i
    }
    fn footprint(&self) -> usize {
        core::mem::size_of::<SesInfo>() + self.line.len() + self.account.len() + self.conn.as_ref().map_or(0, |c| c.tx.len() + c.rx.len() + c.inq.iter().map(|l| l.len() + 24).sum::<usize>() + 256 + c.ssh.as_ref().map_or(0, |x| x.footprint()))
    }
    /// who the session of a Linux shell belongs to
    pub fn user_name(&self) -> &str {
        if self.account.is_empty() { "root" } else { &self.account }
    }
}

/// a session's command line while another one is in place (CliDev holds the one in place)
#[derive(Clone, Debug)]
pub struct SesCli {
    pub mode: Mode,
    pub ifs: Vec<String>,
    pub vlans: Vec<u16>,
    pub line: usize,
    pub pool: String,
    pub job: Option<CliJob>,
    pub queue: VecDeque<String>,
    pub ask: Option<Ask>,
    pub(crate) epoch: u32,
    pub(crate) shown: Option<(String, bool)>,
    pub acl: String,
    pub files: files::FilesCli,
    pub priv_level: u8,
    pub gate: Option<aaa::Gate>,
    pub user: String,
    pub sg_name: String,
    pub rs_name: String,
    pub sh: sh::Shell,
    pub fw: [String; 2],
}

impl SesCli {
    /// a fresh session's command line (IOS at the user level, a shell in its home)
    pub fn new(style: CliStyle) -> SesCli {
        SesCli {
            mode: Mode::User,
            ifs: Vec::new(),
            vlans: Vec::new(),
            line: 0,
            pool: String::new(),
            job: None,
            queue: VecDeque::new(),
            ask: None,
            epoch: 0,
            shown: None,
            acl: String::new(),
            files: files::FilesCli::default(),
            priv_level: 1,
            gate: None,
            user: String::new(),
            sg_name: String::new(),
            rs_name: String::new(),
            sh: sh::Shell::new(style),
            fw: Default::default(),
        }
    }
    fn footprint(&self) -> usize {
        core::mem::size_of::<SesCli>() + self.queue.iter().map(|l| l.len() + 24).sum::<usize>() + self.gate.as_ref().map_or(0, |g| g.user.len() + g.methods.len() * 16 + 48) + self.sh.footprint()
    }
}

/// a session as the app and the tests see it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SesView {
    pub id: u32,
    pub kind: SesKind,
    pub line: String,
    /// who logged in ("" none; Linux sessions are root's)
    pub user: String,
    pub since: Ns,
    /// since the last line typed
    pub idle: Ns,
    /// a telnet session's peer
    pub from: Option<IpAddr>,
    pub monitor: bool,
}

impl CliDev {
    /// swaps the command line in place with `st`
    pub(crate) fn swap_state(&mut self, st: &mut SesCli) {
        use core::mem::swap;
        swap(&mut self.mode, &mut st.mode);
        swap(&mut self.ifs, &mut st.ifs);
        swap(&mut self.vlans, &mut st.vlans);
        swap(&mut self.line, &mut st.line);
        swap(&mut self.pool, &mut st.pool);
        swap(&mut self.job, &mut st.job);
        swap(&mut self.queue, &mut st.queue);
        swap(&mut self.ask, &mut st.ask);
        swap(&mut self.epoch, &mut st.epoch);
        swap(&mut self.shown, &mut st.shown);
        swap(&mut self.acl, &mut st.acl);
        swap(&mut self.files, &mut st.files);
        swap(&mut self.priv_level, &mut st.priv_level);
        swap(&mut self.gate, &mut st.gate);
        swap(&mut self.user, &mut st.user);
        swap(&mut self.sg_name, &mut st.sg_name);
        swap(&mut self.rs_name, &mut st.rs_name);
        swap(&mut self.sh, &mut st.sh);
        swap(&mut self.fw, &mut st.fw);
    }
    /// the infos of session `s`
    pub fn ses(&self, s: u32) -> Option<&SesInfo> {
        self.sessions.iter().find(|x| x.id == s)
    }
    pub(crate) fn ses_mut(&mut self, s: u32) -> Option<&mut SesInfo> {
        self.sessions.iter_mut().find(|x| x.id == s)
    }
    /// what the sessions besides the console's command line hold (keyframe.rs node_bytes)
    pub fn ses_footprint(&self) -> usize {
        self.sessions.iter().map(|i| i.footprint()).sum::<usize>() + self.parked.values().map(|p| p.footprint() + 32).sum::<usize>()
    }
}

/// "hh:mm:ss" of a span (show users)
pub(crate) fn hms(ns: Ns) -> String {
    let s = ns / 1_000_000_000;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

impl Engine {
    // ---------------------------------------------------------------- the session in place

    /// the session whose command line is in place at node `ni` (0: the console)
    pub(crate) fn ses_cur(&self, ni: usize) -> u32 {
        self.cdev(ni).map_or(0, |c| c.cur)
    }
    pub(crate) fn ses_info(&self, ni: usize, s: u32) -> Option<&SesInfo> {
        self.cdev(ni)?.ses(s)
    }
    pub(crate) fn ses_info_mut(&mut self, ni: usize, s: u32) -> Option<&mut SesInfo> {
        self.cdev_mut(ni)?.ses_mut(s)
    }
    /// the infos of the session in place
    pub(crate) fn ses_here(&self, ni: usize) -> Option<&SesInfo> {
        let c = self.cdev(ni)?;
        c.ses(c.cur)
    }
    pub(crate) fn ses_here_mut(&mut self, ni: usize) -> Option<&mut SesInfo> {
        let c = self.cdev_mut(ni)?;
        let s = c.cur;
        c.ses_mut(s)
    }
    /// the session in place is a telnet session
    pub(crate) fn ses_remote(&self, ni: usize) -> bool {
        self.ses_here(ni).is_some_and(|i| i.conn.is_some())
    }

    // puts session s's command line in place -> the session that was in place (None: there is
    // no session s)
    fn ses_put(&mut self, ni: usize, s: u32) -> Option<u32> {
        let c = self.cdev_mut(ni)?;
        if c.cur == s {
            return Some(s);
        }
        let mut st = c.parked.remove(&s)?;
        c.swap_state(&mut st);
        let was = c.cur;
        c.parked.insert(was, st);
        c.cur = s;
        Some(was)
    }

    /// runs `f` with session `s` in place; the session before comes back after (None: there is
    /// no session s)
    pub(crate) fn in_ses<R>(&mut self, ni: usize, s: u32, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        let was = self.ses_put(ni, s)?;
        let r = f(self);
        self.ses_put(ni, was);
        Some(r)
    }

    /// a wake-up of the session in place at `at` (cli/mod.rs: cli_wake_at, shrun.rs: the
    /// background jobs): the console's as Ev::Cli, another session's as Ev::Ses
    pub(crate) fn cli_sched(&mut self, ni: usize, at: Ns, epoch: u32) {
        match self.ses_cur(ni) {
            0 => self.schedule(at, Ev::Cli { node: ni, epoch }),
            s => self.schedule(at, Ev::Ses { node: ni, ses: s, epoch }),
        }
    }

    /// Ev::Ses: a wake-up of session `s`
    pub(crate) fn ses_wake(&mut self, ni: usize, s: u32, epoch: u32) {
        if !self.nodes.get(ni).is_some_and(|n| n.as_ref().is_some_and(|n| n.power)) {
            return;
        }
        if epoch & SES_MARK == 0 {
            self.in_ses(ni, s, |e| e.cli_wake(ni, epoch));
            return;
        }
        match epoch & (SES_MARK | W_KIND) {
            W_START => {
                self.in_ses(ni, s, |e| e.ses_start(ni));
            }
            W_INPUT => {
                self.in_ses(ni, s, |e| e.ses_input(ni));
            }
            W_IDLE => {
                self.in_ses(ni, s, |e| e.ses_idle(ni, epoch & 0x00FF_FFFF));
            }
            W_SSH => {
                self.in_ses(ni, s, |e| e.ssh_timeout(ni));
            }
            W_END => self.ses_drop(ni, s),
            _ => {}
        }
    }

    /// the session that waits (not the one in place) whose console job or background job is
    /// job `id`
    pub(crate) fn ses_of_job(&self, ni: usize, id: crate::jobs::JobId) -> Option<u32> {
        let c = self.cdev(ni)?;
        c.parked.iter().find(|(_, p)| p.job.as_ref().is_some_and(|j| j.id == id) || p.sh.bg.iter().any(|b| b.cj.as_ref().is_some_and(|j| j.id == id))).map(|(s, _)| *s)
    }

    /// the session that waits whose program (foreground or background) is service `svc`
    pub(crate) fn ses_of_prog(&self, ni: usize, svc: u32) -> Option<u32> {
        let is = |j: &Option<CliJob>| matches!(j, Some(CliJob { out: JobOut::Proc { svc: x, .. }, .. }) if *x == svc);
        let c = self.cdev(ni)?;
        c.parked.iter().find(|(_, p)| is(&p.job) || p.sh.bg.iter().any(|b| is(&b.cj))).map(|(s, _)| *s)
    }

    // ---------------------------------------------------------------- output

    /// out_raw for another session than the console: true when it took the text
    pub(crate) fn ses_out(&mut self, ni: usize, text: &str) -> bool {
        match self.ses_cur(ni) {
            0 => false,
            s => {
                self.ses_write(ni, s, text);
                true
            }
        }
    }

    /// output of session `s`: the console's report, a local session's, a telnet connection
    pub(crate) fn ses_write(&mut self, ni: usize, s: u32, text: &str) {
        if text.is_empty() {
            return;
        }
        if s == 0 {
            self.con_write(ni, text);
            return;
        }
        let remote = match self.ses_info(ni, s) {
            Some(i) => i.conn.is_some(),
            None => return,
        };
        if remote {
            if let Some(c) = self.ses_info_mut(ni, s).and_then(|i| i.conn.as_mut()) {
                c.write_text(text);
            }
            self.ses_flush(ni, s);
            return;
        }
        let dev = self.dev_of(ni);
        let at = self.now;
        if let Some(Report::SesOut { dev: d, ses, at: a, bytes }) = self.cli.reports.last_mut()
            && *d == dev
            && *ses == s
            && *a == at
        {
            bytes.extend_from_slice(text.as_bytes());
            return;
        }
        self.cli.reports.push(Report::SesOut { dev, ses: s, at, bytes: Vec::from(text.as_bytes()) });
    }

    // the console's output while another session is in place (the same report out_raw makes)
    fn con_write(&mut self, ni: usize, text: &str) {
        let dev = self.dev_of(ni);
        let at = self.now;
        if let Some(Report::CliOut { dev: d, at: a, bytes }) = self.cli.reports.last_mut()
            && *d == dev
            && *a == at
        {
            bytes.extend_from_slice(text.as_bytes());
            return;
        }
        self.cli.reports.push(Report::CliOut { dev, at, bytes: Vec::from(text.as_bytes()) });
    }

    /// a console message or debug output: the console has it (as before sessions), every
    /// other session with "terminal monitor" too
    pub(crate) fn log_out(&mut self, ni: usize, text: &str) {
        match self.ses_cur(ni) {
            0 => self.out(ni, text),
            _ => self.con_write(ni, text),
        }
        let mon: Vec<u32> = self.cdev(ni).map(|c| c.sessions.iter().filter(|i| i.id != 0 && i.monitor && !i.ending).map(|i| i.id).collect()).unwrap_or_default();
        for s in mon {
            self.ses_write(ni, s, text);
        }
    }

    /// cli_state for another session than the console: a local session reports its prompt,
    /// a telnet session writes it into the connection when it is due -> true when it was one
    pub(crate) fn ses_state(&mut self, ni: usize) -> bool {
        let s = self.ses_cur(ni);
        if s == 0 {
            return false;
        }
        if self.ses_remote(ni) {
            self.ses_prompt_due(ni);
            return true;
        }
        let dev = self.dev_of(ni);
        let now = (self.prompt_of(ni), self.cdev(ni).is_some_and(|c| c.job.is_some()));
        let Some(c) = self.cdev_mut(ni) else {
            return true;
        };
        if c.shown.as_ref() == Some(&now) {
            return true;
        }
        c.shown = Some(now.clone());
        self.cli.reports.push(Report::SesPrompt { dev, ses: s, prompt: now.0, busy: now.1 });
        true
    }

    /// a line of a session was taken: the idle time starts again, a telnet session's next
    /// prompt is due
    pub(crate) fn ses_took_line(&mut self, ni: usize) {
        let now = self.now;
        if let Some(i) = self.ses_here_mut(ni) {
            i.last = now;
            i.prompted = false;
        }
    }

    // ---------------------------------------------------------------- open and close

    /// a new session of node `ni` (its command line waits in `parked`) -> its id
    pub(crate) fn ses_new(&mut self, ni: usize, kind: SesKind, conn: Option<Conn>, no: Option<u16>) -> Option<u32> {
        let now = self.now;
        let style = self.cdev(ni)?.style;
        if style == CliStyle::None {
            return None;
        }
        // its line: the lowest free number of its kind
        let no = match no {
            Some(n) => n,
            None => {
                let c = self.cdev(ni)?;
                let pts = style == CliStyle::Linux;
                let first: u16 = if pts { 0 } else { 1 };
                (first..u16::MAX).find(|n| !c.sessions.iter().any(|i| i.id != 0 && i.no == *n && (pts || i.kind == kind))).unwrap_or(0)
            }
        };
        let line = match (style, kind) {
            (CliStyle::Ios, SesKind::Telnet) => format!("vty {}", no),
            (CliStyle::Ios, _) => format!("tty {}", no),
            _ => format!("pts/{}", no),
        };
        let pid = if style == CliStyle::Linux { self.os_mut(ni).map_or(0, |o| 100 + o.take_id()) } else { 0 };
        let c = self.cdev_mut(ni)?;
        let id = c.next_ses.max(1);
        c.next_ses = id + 1;
        let mut info = SesInfo::new(id, kind, line, no, now);
        info.pid = pid;
        info.conn = conn;
        c.sessions.push(info);
        c.parked.insert(id, SesCli::new(style));
        Some(id)
    }

    /// session `s` starts: a telnet session at its start event, a local one at once (with
    /// it in place)
    pub(crate) fn ses_start_later(&mut self, ni: usize, s: u32) {
        let now = self.now;
        self.schedule(now, Ev::Ses { node: ni, ses: s, epoch: W_START });
    }

    // the session in place starts: a login, its first prompt
    fn ses_start(&mut self, ni: usize) {
        let Some((kind, id)) = self.ses_here(ni).map(|i| (i.kind, i.id)) else {
            return;
        };
        let style = self.cdev(ni).map_or(CliStyle::None, |c| c.style);
        match (style, kind) {
            // a local terminal of an IOS-like device logs in as line con 0 says
            (CliStyle::Ios, SesKind::Local) => self.line_login(ni, "con 0", crate::aaa::Requester::Local(id)),
            (_, SesKind::Telnet) => self.telnet_start(ni),
            _ => {}
        }
        self.cli_state(ni);
    }

    /// session `s` ends once the engine left it (exit, an idle timeout, a closed connection)
    pub(crate) fn ses_end(&mut self, ni: usize, s: u32) {
        if s == 0 {
            return;
        }
        let Some(i) = self.ses_info_mut(ni, s) else {
            return;
        };
        if i.ending {
            return;
        }
        i.ending = true;
        let now = self.now;
        self.schedule(now, Ev::Ses { node: ni, ses: s, epoch: W_END });
    }

    /// session `s` goes now: what runs in it ends, its connection closes
    pub(crate) fn ses_drop(&mut self, ni: usize, s: u32) {
        if s == 0 || self.ses_info(ni, s).is_none() {
            return;
        }
        self.in_ses(ni, s, |e| e.ses_cleanup(ni));
        let dev = self.dev_of(ni);
        let Some(c) = self.cdev_mut(ni) else {
            return;
        };
        let status = c.parked.remove(&s).map_or(0, |p| p.sh.status);
        let Some(k) = c.sessions.iter().position(|i| i.id == s) else {
            return;
        };
        let info = c.sessions.remove(k);
        if info.kind == SesKind::Local {
            self.cli.reports.push(Report::SesClosed { dev, ses: s });
        }
        if let Some(conn) = info.conn {
            // SSH has last words: the exit status and the channel's end (sshd.rs)
            if conn.ssh.is_some() {
                self.ssh_drop(ni, conn, status);
                return;
            }
            if !conn.tx.is_empty() {
                let _ = self.tcp_write(ni, conn.svc, conn.sock, &conn.tx, None);
            }
            self.tcp_close(ni, conn.svc, conn.sock, None);
        }
    }

    // what runs in the session in place ends (its console job, its program, its background
    // jobs)
    fn ses_cleanup(&mut self, ni: usize) {
        if let Some(j) = self.cdev_mut(ni).and_then(|c| {
            c.queue.clear();
            c.job.take()
        }) {
            self.cancel_job(j.id);
            self.cli.jobs.remove(&j.id);
            if let JobOut::Proc { svc, .. } = j.out {
                self.console_kill(ni, svc);
            }
        }
        self.sh_abort(ni);
    }

    /// the power went off (every session but the console ends; the console starts afresh) or
    /// came back (action.rs)
    pub(crate) fn ses_power(&mut self, ni: usize, on: bool) {
        let now = self.now;
        if !on {
            let all: Vec<u32> = self.cdev(ni).map(|c| c.sessions.iter().map(|i| i.id).filter(|&s| s != 0).collect()).unwrap_or_default();
            for s in all {
                // the connections went with the power: nothing is sent any more
                if let Some(i) = self.ses_info_mut(ni, s) {
                    i.conn = None;
                }
                self.ses_drop(ni, s);
            }
        }
        if let Some(i) = self.ses_info_mut(ni, 0) {
            i.since = now;
            i.last = now;
            i.monitor = false;
            i.login = None;
        }
    }

    // ---------------------------------------------------------------- the API

    /// a new local session of a device (the Terminal panel's further tabs, a desktop's
    /// terminal window, the IDE's terminal pane) -> its id; None when the device has no command
    /// line or is off
    pub fn cli_open(&mut self, dev: u32) -> Option<u32> {
        let ni = self.console(dev).ok()?;
        let s = self.ses_new(ni, SesKind::Local, None, None)?;
        self.in_ses(ni, s, |e| e.ses_start(ni));
        Some(s)
    }

    /// ends session `s` of a device (its command, its programs and jobs end; a telnet
    /// session's connection closes); the console (0) stays
    pub fn cli_close(&mut self, dev: u32, s: u32) {
        if let Some(ni) = self.node_index(dev) {
            self.ses_drop(ni, s);
        }
    }

    /// every session of a device, the console first
    pub fn cli_sessions(&self, dev: u32) -> Vec<SesView> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        let Some(c) = self.cdev(ni) else {
            return Vec::new();
        };
        let now = self.now;
        let linux = c.style == CliStyle::Linux;
        c.sessions
            .iter()
            .map(|i| {
                let user = if linux {
                    String::from(if i.login.is_some() { "" } else { i.user_name() })
                } else if i.id == c.cur {
                    c.user.clone()
                } else {
                    c.parked.get(&i.id).map(|p| p.user.clone()).unwrap_or_default()
                };
                SesView { id: i.id, kind: i.kind, line: i.line.clone(), user, since: i.since, idle: now.saturating_sub(i.last), from: i.conn.as_ref().map(|c| c.from), monitor: i.monitor }
            })
            .collect()
    }

    // the node of a device whose session `s` exists
    fn ses_node(&self, dev: u32, s: u32) -> Result<usize, ActErr> {
        let ni = self.console(dev)?;
        if self.ses_info(ni, s).is_none() {
            return Err(ActErr::Refused(jfn_model::msg!("Die Sitzung {} gibt es nicht.", s)));
        }
        Ok(ni)
    }

    // the command line of session s while it waits (None: the console or no such session)
    fn parked_of(&self, dev: u32, s: u32) -> Option<(usize, &SesCli)> {
        let ni = self.node_index(dev)?;
        let c = self.cdev(ni)?;
        if s == c.cur {
            return None;
        }
        Some((ni, c.parked.get(&s)?))
    }

    /// a line typed in session `s` of a device (0: the console, as cli_line)
    pub fn cli_ses_line(&mut self, dev: u32, s: u32, line: &str) -> Result<(), ActErr> {
        if s == 0 {
            return self.cli_line(dev, line);
        }
        let ni = self.ses_node(dev, s)?;
        self.in_ses(ni, s, |e| {
            e.ses_took_line(ni);
            e.cli_line_at(ni, line)
        });
        Ok(())
    }

    /// Ctrl+C / Ctrl+Shift+6 in session `s`
    pub fn cli_ses_interrupt(&mut self, dev: u32, s: u32) -> Result<(), ActErr> {
        if s == 0 {
            return self.cli_interrupt(dev);
        }
        let ni = self.ses_node(dev, s)?;
        self.in_ses(ni, s, |e| e.cli_interrupt_at(ni));
        Ok(())
    }

    /// "?" after `line` in session `s` (IOS; the session is put in place for it and back)
    pub fn cli_ses_help(&mut self, dev: u32, s: u32, line: &str) -> String {
        if s == 0 {
            return self.cli_help(dev, line);
        }
        let Some(ni) = self.node_index(dev) else {
            return String::new();
        };
        self.in_ses(ni, s, |e| match e.cdev(ni) {
            Some(c) if c.gate.is_some() || c.style != CliStyle::Ios => String::new(),
            Some(_) => e.ios_help(ni, line),
            None => String::new(),
        })
        .unwrap_or_default()
    }

    /// Tab in session `s`
    pub fn cli_ses_complete(&mut self, dev: u32, s: u32, line: &str) -> (Option<String>, Vec<String>) {
        if s == 0 {
            return self.cli_complete(dev, line);
        }
        let Some(ni) = self.node_index(dev) else {
            return (None, Vec::new());
        };
        self.in_ses(ni, s, |e| match e.cdev(ni).map(|c| (c.style, c.gate.is_some())) {
            Some((_, true)) => (None, Vec::new()),
            Some((CliStyle::Ios, _)) => e.ios_complete(ni, line),
            Some((CliStyle::Linux, _)) => e.linux_complete(ni, line),
            _ => (None, Vec::new()),
        })
        .unwrap_or((None, Vec::new()))
    }

    /// the prompt session `s` shows now
    pub fn cli_ses_prompt(&self, dev: u32, s: u32) -> String {
        let Some((ni, p)) = self.parked_of(dev, s) else {
            return if s == 0 { self.cli_prompt(dev) } else { String::new() };
        };
        if let Some(t) = self.ses_info(ni, s).and_then(|i| self.login_prompt(ni, i)) {
            return t;
        }
        if let Some(g) = &p.gate {
            return g.prompt();
        }
        if let Some(a) = &p.ask {
            return a.prompt.clone();
        }
        let host = self.host(ni);
        match self.cdev(ni).map_or(CliStyle::None, |c| c.style) {
            CliStyle::Ios => format!("{}{}", host, super::ios::prompt_suffix(p.mode)),
            CliStyle::Linux => match self.ses_info(ni, s).map(|i| i.user_name()) {
                Some(u) if u != "root" => format!("{}@{}:{}$ ", u, host, p.sh.prompt_dir()),
                _ => format!("root@{}:{}# ", host, p.sh.prompt_dir()),
            },
            CliStyle::None => String::new(),
        }
    }
    /// a command runs in session `s` (typed lines wait)
    pub fn cli_ses_busy(&self, dev: u32, s: u32) -> bool {
        match self.parked_of(dev, s) {
            Some((_, p)) => p.job.is_some() || p.gate.as_ref().is_some_and(|g| g.waiting),
            None => s == 0 && self.cli_busy(dev),
        }
    }
    /// session `s` waits for a line typed without echo (a password)
    pub fn cli_ses_secret(&self, dev: u32, s: u32) -> bool {
        match self.parked_of(dev, s) {
            Some((ni, p)) => p.gate.as_ref().is_some_and(|g| g.secret()) || self.ses_info(ni, s).is_some_and(|i| i.login.as_ref().is_some_and(|l| l.user.is_some())) || self.prog_hides(ni, &p.job),
            None => s == 0 && self.cli_secret(dev),
        }
    }
    /// the program of session `s` reads typed lines (telnet, nc)
    pub fn cli_ses_reads(&self, dev: u32, s: u32) -> bool {
        match self.parked_of(dev, s) {
            Some((_, p)) => matches!(p.job, Some(CliJob { out: JobOut::Proc { stdin: true, .. }, .. })),
            None => s == 0 && self.cli_reads(dev),
        }
    }
    /// the working directory of session `s`
    pub fn cli_ses_cwd(&self, dev: u32, s: u32) -> String {
        match self.parked_of(dev, s) {
            Some((_, p)) => p.sh.cwd.clone(),
            None if s == 0 => self.cli_cwd(dev),
            None => String::new(),
        }
    }
    /// session `s`'s last line asks the terminal to page (less, more, man)
    pub fn cli_ses_pager(&self, dev: u32, s: u32) -> bool {
        match self.parked_of(dev, s) {
            Some((_, p)) => p.sh.pager,
            None => s == 0 && self.cli_pager(dev),
        }
    }
    /// the IOS mode of session `s` (Ctrl+Z leaves the configuration modes)
    pub fn cli_ses_mode(&self, dev: u32, s: u32) -> Option<Mode> {
        match self.parked_of(dev, s) {
            Some((_, p)) => Some(p.mode),
            None if s == 0 => self.node(dev).map(|n| n.cli.mode),
            None => None,
        }
    }
    /// the console's program reads typed lines (telnet, nc): the terminal keeps its input line,
    /// without a prompt of its own
    pub fn cli_reads(&self, dev: u32) -> bool {
        self.node_index(dev).and_then(|ni| self.cdev(ni)).is_some_and(|c| matches!(c.job, Some(CliJob { out: JobOut::Proc { stdin: true, .. }, .. })))
    }

    // ---------------------------------------------------------------- show users, who, ps

    /// "show users" of an IOS-like device: every line in use, the one that asks marked
    pub(crate) fn ios_users(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        let now = self.now;
        let mut s = String::from("    Line       User       Privilege  Idle      Location\n");
        for i in &c.sessions {
            let (user, level) = if i.id == c.cur { (c.user.clone(), c.priv_level) } else { c.parked.get(&i.id).map(|p| (p.user.clone(), p.priv_level)).unwrap_or_default() };
            let who = if user.is_empty() { String::from("-") } else { user };
            let mark = if i.id == c.cur { '*' } else { ' ' };
            let loc = i.conn.as_ref().map(|x| format!("{}{}", x.from, Engine::ssh_marker(i))).unwrap_or_default();
            s.push_str(&format!("{}   {:<10} {:<10} {:<10} {:<9} {}", mark, i.line, who, level, hms(now.saturating_sub(i.last)), loc).trim_end());
            s.push('\n');
        }
        s
    }

    /// the processes of the sessions (procs.rs, ps): per session its id, its terminal, its
    /// shell's PID, its program in the foreground (a service id) and its live background jobs
    /// (PID, service, text)
    pub(crate) fn ses_procs(&self, ni: usize) -> Vec<(u32, String, u32, Option<u32>, Vec<(u32, Option<u32>, String)>)> {
        let Some(c) = self.cdev(ni) else {
            return Vec::new();
        };
        let svc_of = |j: &Option<CliJob>| match j {
            Some(CliJob { out: JobOut::Proc { svc, .. }, .. }) => Some(*svc),
            _ => None,
        };
        c.sessions
            .iter()
            .filter_map(|i| {
                let (job, shell) = if i.id == c.cur { (&c.job, &c.sh) } else { c.parked.get(&i.id).map(|p| (&p.job, &p.sh))? };
                let jobs = shell.bg.iter().filter(|j| j.ended.is_none()).map(|j| (j.pid, svc_of(&j.cj), j.text.clone())).collect();
                Some((i.id, i.line.clone(), i.pid, svc_of(job), jobs))
            })
            .collect()
    }

    /// the PID of the shell in place ($$)
    pub(crate) fn ses_pid(&self, ni: usize) -> u32 {
        self.ses_here(ni).map_or(sh::SHELL_PID, |i| if i.pid == 0 { sh::SHELL_PID } else { i.pid })
    }
}
