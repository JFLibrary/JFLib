// cli -- the command lines of the devices: an IOS-like console for routers, switches,
// firewalls and access points, a Linux-like shell for hosts and servers
//
//   Engine::cli_line(dev, line)        a line typed at the console (Enter); while a command
//                                      runs, lines wait and run after it (with their echo); an
//                                      IOS line ending in "?" is help, as if "?" was pressed
//   Engine::cli_interrupt(dev)         Ctrl+C (Linux) / Ctrl+Shift+6 (IOS)
//   Engine::cli_help(dev, line)        the text of "?" typed after `line` (IOS)
//   Engine::cli_complete(dev, line)    Tab: the completed line and the choices
//   Engine::cli_prompt(dev) / cli_busy(dev)
//   Engine::running_config(dev)        IOS configuration, or shell commands for Linux devices
//   Engine::startup_config(dev)        what the device boots with
//   Engine::take_reports()             console output and state changes (driver::Report)
//   Engine::boot_errors(dev)           lines of the startup configuration that failed
//
//   ios      the IOS command tree (base::cmdtree) and what its commands do
//   iosshow  show commands and the running configuration of IOS devices
//   linux    the shell: ip addr / link / route / neigh, ping, traceroute, hostname, sysctl
//   print    ping and traceroute as console commands, IOS-like and iputils output
//   log      IOS console messages (%LINK, %LINEPROTO, %SYS) and debug ip icmp / debug arp
//   fmt      dotted MACs, log time stamps, the numbers of iputils
//   tools    curl, nc, ss, top, printf and pipes into nc on the shell; show tcp brief, show tcp
//            statistics, show control-plane host open-ports on IOS (curl and nc are console
//            programs: services that print to the console and read typed lines)
//   lab      lbctl, kvctl, cachectl, mqctl, workerctl: the lab's services on the shell
//   scripts  script: the device's scripts on the shell (scriptsvc.rs)
//   netsvc   DHCP, NTP and the resolver at the command lines: IOS "ip dhcp pool" (its own
//            mode), "ip dhcp excluded-address", "ip helper-address", show ip dhcp binding|
//            pool|conflict, clear ip dhcp, "ntp server", "ntp master"; dhclient and cat
//            /etc/resolv.conf on the shell
//   stp      spanning tree on IOS switches: "spanning-tree ...", show spanning-tree, debug
//            spanning-tree events, clear spanning-tree detected-protocols
//   rip      RIP at the IOS-like command line: "router rip" (its own mode), "ip rip send|receive
//            version", show ip rip database, show ip protocols, the R routes of show ip route,
//            debug ip rip, clear ip route *, the lines of the running configuration
//   acl      IOS access lists: access-list, ip access-list standard|extended (their own
//            modes), ip access-group, show access-lists, clear access-list counters
//   nat      IOS NAT: ip nat inside|outside, ip nat pool, ip nat inside source, translation
//            timeouts, show ip nat translations|statistics, clear ip nat, debug ip nat
//   ip6      IPv6 at the IOS-like console: ipv6 unicast-routing, route, neighbor, the interface
//            commands (ipv6 enable, address, autoconfig, nd ...), show ipv6 interface|route|
//            neighbors|routers, ping / traceroute ipv6, debug ipv6 icmp|nd, clear ipv6
//            neighbors, the running configuration
//   linux6   IPv6 in the shell: ip -6 addr|route|neigh, ping6 / ping -6, traceroute6 /
//            traceroute -6, sysctl net.ipv6.conf.*, the device's configuration
//   print6   ping and traceroute over IPv6 as console commands
//   ospf     OSPF at the IOS-like command line: "router ospf N" (its own tree mode behind
//            "(config-router)#"), "ip ospf ...", show ip ospf [neighbor|interface|database],
//            the O routes of show ip route, debug ip ospf adj|events|hello, clear ip ospf
//            process, the lines of the running configuration
//   names    dig, host and nslookup on the shell (console programs with their own questions
//            to a name server); ip host, ipv6 host, ip name-server and ping by name on IOS
//   wlan     WLAN: iw in the shell (dev, info, link, scan, connect, disconnect, station dump),
//            show dot11 associations on the IOS-like access point
//   bgp      BGP at the IOS-like command line: "router bgp ASN" (its own tree mode behind
//            "(config-router)#"), show ip bgp [summary|neighbors], the B routes of show ip
//            route, debug ip bgp [events|updates], clear ip bgp, the running configuration
//   files    the files of a device: dir, show flash:, more, delete, copy with its questions,
//            ip ftp username / password (IOS-like); ls, cat, rm, cp, mkdir, rmdir, touch, echo
//            > FILE, tftp (Linux); curl ftp:// is ftpc.rs
//
// A device's configuration lives in its parts (ports, bridge, IPv4); the running
// configuration is written from them and the startup configuration runs through the same
// parser at load time (load.rs), so there is one source of truth.  What only the command
// line knows (descriptions, console lines, subinterfaces without a VLAN yet, how an address
// was set) is in CliDev.  Every change goes through Engine::apply.  Output carries the
// virtual time it was made at; nothing here reads a clock.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use base::cmdtree::Tree;
use jfn_model::catalog::{CliStyle, Model};

use crate::action::ActErr;
use crate::driver::Report;
use crate::jobs::JobId;
use crate::kernel::{Engine, Ev, Ns};
use crate::trace::{HopId, Note};

pub mod fmt;
pub mod ios;
pub mod iosshow;
pub mod linux;
pub mod log;
pub mod netsvc;
pub mod print;
pub mod tools;
pub mod lab;
pub mod scripts;
pub mod stp;
pub mod rip;
pub mod acl;
pub mod nat;
pub mod ip6;
pub mod linux6;
pub mod print6;
pub mod ospf;
pub mod names;
pub mod wlan;
pub mod bgp;
pub mod files;

/// debugging switched on ("debug ip icmp", "debug arp")
pub const D_ICMP: u32 = 1;
pub const D_ARP: u32 = 2;
/// "debug spanning-tree events"
pub const D_STP: u32 = 4;
/// "debug ip rip" (a bit of its own, away from the low ones other protocols take)
pub const D_RIP: u32 = 0x100;
/// "debug ip nat" (a bit of its own, away from the others' next ones)
pub const D_NAT: u32 = 0x200;
/// "debug ipv6 icmp", "debug ipv6 nd" (cli/ip6.rs)
pub const D_ICMP6: u32 = 0x800;
pub const D_ND: u32 = 0x1000;
/// "debug ip ospf adj|events|hello" (which of them: CliDev::ospf_debug)
pub const D_OSPF: u32 = 0x400;
/// "debug ip bgp [events|updates]" (which of them: CliDev::bgp_debug)
pub const D_BGP: u32 = 0x8000;

/// the routing process a "(config-router)#" console sets up
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proc {
    /// "router rip"
    Rip,
    /// "router ospf N"
    Ospf(u16),
    /// "router bgp ASN"
    Bgp(u32),
}

/// where an IOS console is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    User,
    Priv,
    Config,
    If,
    SubIf,
    Vlan,
    Line,
    /// logged out: "Console ready, press Enter to log in."
    Out,
    /// "ip dhcp pool NAME": "(dhcp-config)#"
    DhcpPool,
    /// "router rip", "router ospf N": "(config-router)#" for the process it names
    Router(Proc),
    /// "ip access-list standard|extended NAME": "(config-std-nacl)#", "(config-ext-nacl)#"
    AclStd,
    AclExt,
}

/// how an interface's address was set ("show ip interface brief", column Method)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Unset,
    Nvram,
    Manual,
}

/// a console line block of an IOS configuration ("line con 0", "line vty 0 4")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineCfg {
    pub name: String,
    pub opts: Vec<String>,
}

/// a line of the startup configuration that failed at boot
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootErr {
    /// 1-based
    pub line: u32,
    pub text: String,
    /// the first line of the console's answer ("% Invalid input detected at '^' marker.")
    pub msg: String,
}

/// what only the command line keeps of a device's configuration
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevCfg {
    /// the configuration the device boots with
    pub startup: String,
    /// by full interface name
    pub methods: BTreeMap<String, Method>,
    /// "description" by full interface name
    pub descr: BTreeMap<String, String>,
    /// IOS subinterfaces that have no "encapsulation dot1Q" yet
    pub pending: Vec<String>,
    pub lines: Vec<LineCfg>,
    /// "ip domain-lookup" (IOS looks unknown EXEC words up as host names)
    pub domain_lookup: bool,
    pub boot_errors: Vec<BootErr>,
    /// DHCP pools without a network yet (the DHCP server takes them with "network")
    pub pools: Vec<(String, crate::dhcpd::PoolCfg)>,
}

impl Default for DevCfg {
    fn default() -> DevCfg {
        DevCfg {
            startup: String::new(),
            methods: BTreeMap::new(),
            descr: BTreeMap::new(),
            pending: Vec::new(),
            lines: alloc::vec![LineCfg { name: String::from("con 0"), opts: Vec::new() }, LineCfg { name: String::from("vty 0 4"), opts: alloc::vec![String::from("login")] }],
            domain_lookup: true,
            boot_errors: Vec::new(),
            pools: Vec::new(),
        }
    }
}

/// a question the console waits for ("Save to which file [startup-config]? ")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub prompt: String,
    pub kind: AskKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    CopyRunStart,
    /// "Output Interface: " of a ping to a link-local IPv6 address (cli/ip6.rs)
    Ping6Out { dst: netpkt::Ipv6, count: u32, bytes: u16, timeout: Ns },
    /// "clear ip ospf process"
    ClearOspf,
    /// a question of copy or delete (files.rs: CliDev::files holds the dialog)
    Files,
}

/// a console command that runs over virtual time
#[derive(Clone, Debug)]
pub struct CliJob {
    pub id: JobId,
    pub out: print::JobOut,
    pub start: Ns,
}

/// the state of an interface as IOS logs it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfSnap {
    pub name: String,
    pub admin: bool,
    pub link: bool,
    pub proto: bool,
}

/// the command line of a device
#[derive(Clone, Debug)]
pub struct CliDev {
    pub style: CliStyle,
    pub mode: Mode,
    /// the interfaces of interface mode
    pub ifs: Vec<String>,
    /// the VLANs of VLAN mode
    pub vlans: Vec<u16>,
    /// the line block of line mode
    pub line: usize,
    /// the pool of DHCP pool mode
    pub pool: String,
    pub job: Option<CliJob>,
    /// lines typed while a command runs
    pub queue: VecDeque<String>,
    pub ask: Option<Ask>,
    pub debug: u32,
    /// wake-ups of an older epoch are stale
    pub(crate) epoch: u32,
    pub cfg: DevCfg,
    /// the prompt and busy state last reported
    pub(crate) shown: Option<(String, bool)>,
    pub(crate) snap: Vec<IfSnap>,
    /// debug arp: the received request already printed
    pub(crate) rcvd_hop: HopId,
    pub(crate) booting: bool,
    /// the list of an access list's mode
    pub acl: String,
    /// what "debug ip ospf" shows (cli/ospf.rs DO_*; none: everything)
    pub ospf_debug: u8,
    /// what "debug ip bgp" shows (cli/bgp.rs DB_*; none: everything)
    pub bgp_debug: u8,
    /// copy's dialog, the last host, the FTP login (files.rs)
    pub files: files::FilesCli,
}

impl CliDev {
    pub fn new(m: &Model) -> CliDev {
        CliDev {
            style: m.cli,
            mode: Mode::User,
            ifs: Vec::new(),
            vlans: Vec::new(),
            line: 0,
            pool: String::new(),
            job: None,
            queue: VecDeque::new(),
            ask: None,
            debug: 0,
            epoch: 0,
            cfg: DevCfg::default(),
            shown: None,
            snap: Vec::new(),
            rcvd_hop: 0,
            booting: false,
            acl: String::new(),
            ospf_debug: 0,
            bgp_debug: 0,
            files: files::FilesCli::default(),
        }
    }
}

/// the command lines' part of the engine
#[derive(Clone, Debug, Default)]
pub struct EngineCli {
    pub(crate) reports: Vec<Report>,
    /// devices with debugging on
    pub(crate) debug: u32,
    /// IOS command trees by capability bits
    pub(crate) trees: Vec<(u32, Arc<Tree<ios::K>>)>,
    /// console jobs: job -> node slot
    pub(crate) jobs: BTreeMap<JobId, usize>,
}

/// does the command line want to see this decision?  (Link changes for the IOS logs, the
/// rest for debugging.)
pub(crate) fn wants(c: &EngineCli, n: &Note) -> bool {
    match n {
        Note::LinkUp { .. } | Note::LinkDown { .. } => true,
        Note::ArpRequest { .. } | Note::ArpReply { .. } | Note::ArpLearned { .. } | Note::ArpProxied { .. } | Note::EchoAnswered { .. } | Note::IcmpSent { .. } | Note::IpLocal { .. } => c.debug > 0,
        n if n.is_stp_event() => c.debug > 0,
        n if n.is_rip() => c.debug > 0,
        n if n.is_nat() => c.debug > 0,
        Note::V6(_) => c.debug > 0,
        n if n.is_ospf() => c.debug > 0,
        Note::Bgp(_) => c.debug > 0,
        _ => false,
    }
}

impl Engine {
    pub(crate) fn cdev(&self, ni: usize) -> Option<&CliDev> {
        Some(&self.nodes.get(ni)?.as_ref()?.cli)
    }
    pub(crate) fn cdev_mut(&mut self, ni: usize) -> Option<&mut CliDev> {
        Some(&mut self.nodes.get_mut(ni)?.as_mut()?.cli)
    }
    pub(crate) fn host(&self, ni: usize) -> String {
        match self.nodes.get(ni) {
            Some(Some(n)) => n.name.clone(),
            _ => String::new(),
        }
    }

    /// console output of node `ni` now
    pub(crate) fn out(&mut self, ni: usize, text: &str) {
        if text.is_empty() {
            return;
        }
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

    /// reports the prompt and busy state when they changed
    pub(crate) fn cli_state(&mut self, ni: usize) {
        let dev = self.dev_of(ni);
        let now = (self.prompt_of(ni), self.cdev(ni).is_some_and(|c| c.job.is_some()));
        let Some(c) = self.cdev_mut(ni) else {
            return;
        };
        if c.shown.as_ref() == Some(&now) {
            return;
        }
        c.shown = Some(now.clone());
        self.cli.reports.push(Report::CliState { dev, prompt: now.0, busy: now.1 });
    }

    fn prompt_of(&self, ni: usize) -> String {
        let Some(c) = self.cdev(ni) else {
            return String::new();
        };
        if let Some(a) = &c.ask {
            return a.prompt.clone();
        }
        let host = self.host(ni);
        match c.style {
            CliStyle::Ios => match c.mode {
                Mode::Out => String::new(),
                m => format!("{}{}", host, ios::prompt_suffix(m)),
            },
            CliStyle::Linux => format!("root@{}:~# ", host),
            CliStyle::None => String::new(),
        }
    }

    /// the console's node slot, or why there is none
    fn console(&self, dev: u32) -> Result<usize, ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let Some(Some(n)) = self.nodes.get(ni) else {
            return Err(ActErr::UnknownDevice(dev));
        };
        if n.cli.style == CliStyle::None {
            return Err(ActErr::Refused(jfn_model::msg!("{} hat keine Befehlszeile.", n.name)));
        }
        if !n.power {
            return Err(ActErr::Refused(jfn_model::msg!("{} ist ausgeschaltet.", n.name)));
        }
        Ok(ni)
    }

    // ---------------------------------------------------------------- the console API

    /// a line typed at a device's console
    pub fn cli_line(&mut self, dev: u32, line: &str) -> Result<(), ActErr> {
        let ni = self.console(dev)?;
        let line = line.trim_end_matches(['\r', '\n']);
        if self.cdev(ni).is_some_and(|c| c.job.is_some()) {
            // a console program that reads the console (nc) takes the line
            if self.proc_line(ni, line) {
                return Ok(());
            }
            if let Some(c) = self.cdev_mut(ni) {
                c.queue.push_back(String::from(line));
            }
            return Ok(());
        }
        let ios = self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios);
        if ios && line.ends_with('?') && self.cdev(ni).is_some_and(|c| c.ask.is_none() && c.mode != Mode::Out) {
            let h = self.ios_help(ni, &line[..line.len() - 1]);
            self.out(ni, &h);
            return Ok(());
        }
        let _ = self.cli_run(ni, line);
        self.cli_state(ni);
        Ok(())
    }

    /// runs one line now; Err with the first line of the console's complaint
    pub(crate) fn cli_run(&mut self, ni: usize, line: &str) -> Result<(), String> {
        let Some(c) = self.cdev(ni) else {
            return Ok(());
        };
        if let Some(a) = c.ask.clone() {
            if let Some(c) = self.cdev_mut(ni) {
                c.ask = None;
            }
            self.ios_answer(ni, a, line);
            return Ok(());
        }
        match c.style {
            CliStyle::Ios => self.ios_line(ni, line),
            CliStyle::Linux => self.linux_line(ni, line),
            CliStyle::None => Ok(()),
        }
    }

    /// Ctrl+C / Ctrl+Shift+6
    pub fn cli_interrupt(&mut self, dev: u32) -> Result<(), ActErr> {
        let ni = self.console(dev)?;
        let style = self.cdev(ni).map(|c| c.style).unwrap_or(CliStyle::None);
        let job = self.cdev_mut(ni).and_then(|c| {
            c.queue.clear();
            c.job.take()
        });
        match job {
            Some(j) => {
                self.cancel_job(j.id);
                self.cli.jobs.remove(&j.id);
                self.job_interrupted(ni, j);
            }
            None => {
                let asked = self.cdev_mut(ni).and_then(|c| c.ask.take()).is_some();
                if style == CliStyle::Linux || asked {
                    self.out(ni, "\n");
                }
            }
        }
        self.cli_state(ni);
        Ok(())
    }

    /// the text of "?" typed after `line` (IOS; Linux shells have no such help)
    pub fn cli_help(&self, dev: u32, line: &str) -> String {
        match self.node_index(dev) {
            Some(ni) if self.cdev(ni).is_some_and(|c| c.style == CliStyle::Ios) => self.ios_help(ni, line),
            _ => String::new(),
        }
    }

    /// Tab: the completed line (None: nothing to complete) and what the last word may become
    pub fn cli_complete(&self, dev: u32, line: &str) -> (Option<String>, Vec<String>) {
        match self.node_index(dev) {
            Some(ni) => match self.cdev(ni).map(|c| c.style) {
                Some(CliStyle::Ios) => self.ios_complete(ni, line),
                Some(CliStyle::Linux) => self.linux_complete(ni, line),
                _ => (None, Vec::new()),
            },
            None => (None, Vec::new()),
        }
    }

    /// the prompt the console shows now ("R1(config-if)#", "root@PC1:~# ")
    pub fn cli_prompt(&self, dev: u32) -> String {
        self.node_index(dev).map(|ni| self.prompt_of(ni)).unwrap_or_default()
    }
    /// a command runs (typed lines wait)
    pub fn cli_busy(&self, dev: u32) -> bool {
        self.node_index(dev).and_then(|ni| self.cdev(ni)).is_some_and(|c| c.job.is_some())
    }
    /// the running configuration (IOS: as "show running-config" lists it from "!" to "end";
    /// Linux: the shell commands that set the device up as it is)
    pub fn running_config(&self, dev: u32) -> String {
        match self.node_index(dev) {
            Some(ni) => match self.cdev(ni).map(|c| c.style) {
                Some(CliStyle::Ios) => self.ios_config(ni),
                Some(CliStyle::Linux) => self.linux_config(ni),
                _ => String::new(),
            },
            None => String::new(),
        }
    }
    /// the configuration the device boots with
    pub fn startup_config(&self, dev: u32) -> String {
        self.node_index(dev).and_then(|ni| self.cdev(ni)).map(|c| c.cfg.startup.clone()).unwrap_or_default()
    }
    /// the lines of the startup configuration the device did not take
    pub fn boot_errors(&self, dev: u32) -> Vec<BootErr> {
        self.node_index(dev).and_then(|ni| self.cdev(ni)).map(|c| c.cfg.boot_errors.clone()).unwrap_or_default()
    }
    /// console output and console changes since the last call
    pub fn take_reports(&mut self) -> Vec<Report> {
        core::mem::take(&mut self.cli.reports)
    }

    // ---------------------------------------------------------------- boot

    /// runs a device's startup configuration through its command line (load time)
    pub(crate) fn cli_boot(&mut self, dev: u32, text: &str) {
        let Some(ni) = self.node_index(dev) else {
            return;
        };
        let Some(c) = self.cdev_mut(ni) else {
            return;
        };
        c.cfg.startup = String::from(text);
        let style = c.style;
        if text.trim().is_empty() || style == CliStyle::None {
            return;
        }
        c.booting = true;
        if style == CliStyle::Ios {
            c.mode = Mode::Config;
        }
        for (i, raw) in text.lines().enumerate() {
            let t = raw.trim();
            if t.is_empty() || t.starts_with('!') || t.starts_with('#') {
                continue;
            }
            if style == CliStyle::Ios && t.eq_ignore_ascii_case("end") {
                break;
            }
            if let Err(msg) = self.cli_run(ni, t)
                && let Some(c) = self.cdev_mut(ni)
            {
                c.cfg.boot_errors.push(BootErr { line: i as u32 + 1, text: String::from(t), msg });
            }
            // a ping in a startup configuration would wait: it is not for booting
            if let Some(j) = self.cdev_mut(ni).and_then(|c| c.job.take()) {
                self.cancel_job(j.id);
                self.cli.jobs.remove(&j.id);
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.booting = false;
            c.mode = Mode::User;
            c.ifs.clear();
            c.vlans.clear();
        }
        // spanning tree starts with the whole configuration read (stp.rs)
        self.stp_booted(ni);
        if style == CliStyle::Ios {
            self.ios_snap(ni);
        }
    }

    // ---------------------------------------------------------------- waiting lines, timers

    /// the console goes on at `at` (a waiting line, a timer of a command)
    pub(crate) fn cli_wake_at(&mut self, ni: usize, at: Ns) {
        let Some(c) = self.cdev_mut(ni) else {
            return;
        };
        c.epoch = c.epoch.wrapping_add(1);
        let epoch = c.epoch;
        self.schedule(at, Ev::Cli { node: ni, epoch });
    }

    pub(crate) fn cli_wake(&mut self, ni: usize, epoch: u32) {
        let live = match self.nodes.get(ni) {
            Some(Some(n)) => n.power && n.cli.epoch == epoch,
            _ => false,
        };
        if !live {
            return;
        }
        if self.cdev(ni).is_some_and(|c| c.job.is_some()) {
            // a command that waits on its own (IOS ping without a route)
            self.job_timer(ni);
            return;
        }
        let Some(line) = self.cdev_mut(ni).and_then(|c| c.queue.pop_front()) else {
            return;
        };
        // a line typed while the previous command ran: the console shows it now
        let echo = format!("{}{}\n", self.prompt_of(ni), line);
        self.out(ni, &echo);
        let _ = self.cli_run(ni, &line);
        self.cli_state(ni);
        self.cli_next(ni);
    }

    /// after a command: the next waiting line, if any
    pub(crate) fn cli_next(&mut self, ni: usize) {
        let more = self.cdev(ni).is_some_and(|c| c.job.is_none() && !c.queue.is_empty());
        if more {
            let now = self.now;
            self.cli_wake_at(ni, now);
        }
    }

    // ---------------------------------------------------------------- jobs

    /// a job of the engine moved on (called by jobs.rs)
    pub(crate) fn cli_job(&mut self, id: JobId) {
        let Some(&ni) = self.cli.jobs.get(&id) else {
            return;
        };
        self.job_progress(ni);
    }

    /// starts printing a job for node `ni`
    pub(crate) fn cli_start_job(&mut self, ni: usize, id: JobId, out: print::JobOut) {
        let start = self.now;
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(CliJob { id, out, start });
        }
        self.cli.jobs.insert(id, ni);
        self.job_progress(ni);
    }

    /// the job of node `ni` ended: the console takes lines again
    pub(crate) fn cli_job_done(&mut self, ni: usize) {
        if let Some(j) = self.cdev_mut(ni).and_then(|c| c.job.take()) {
            self.cli.jobs.remove(&j.id);
        }
        self.cli_state(ni);
        self.cli_next(ni);
    }

    /// a decision of a device (called by link.rs for what `wants` picks)
    pub(crate) fn cli_note(&mut self, dev: u32, hop: Option<HopId>, note: &Note) {
        let Some(ni) = self.node_index(dev) else {
            return;
        };
        let Some(c) = self.cdev(ni) else {
            return;
        };
        if c.style != CliStyle::Ios {
            return;
        }
        match note {
            Note::LinkUp { .. } | Note::LinkDown { .. } => {
                // the port changed now; its state is read after the change is complete
                self.ios_snap(ni);
            }
            _ => {
                if c.debug != 0 {
                    self.ios_debug(ni, hop.filter(|&h| h != 0), note);
                }
            }
        }
    }
}
