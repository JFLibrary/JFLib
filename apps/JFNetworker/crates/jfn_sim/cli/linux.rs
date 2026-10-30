// linux -- the shell of hosts and servers ("root@PC1:~# ")
//
//   ip [-br] addr [show [dev] IF] | add|del PREFIX dev IF | flush dev IF
//   ip [-br] link [show [dev] IF] | set [dev] IF up|down|mtu N
//            | add link IF name IF.N type vlan id N | add NAME type dummy | del IF
//   ip route [show] | add|del PREFIX|default [via ADDR] [dev IF] [metric N] | get ADDR
//   ip neigh [show] | add ADDR lladdr MAC dev IF | del ADDR dev IF | flush all|dev IF
//   ping [-c N] [-i S] [-s N] [-t TTL] [-W S] [-M do|dont|want] DEST
//   traceroute [-n] [-m MAX] [-q N] [-w S] DEST       (DEST: an address or a name)
//   hostname [NAME], sysctl [-w] net.ipv4.ip_forward[=0|1], echo, clear, true, false
//   curl, nc, ss, top, printf and "echo|printf ... | nc|cat ..." (tools.rs)
//   lbctl, kvctl, cachectl, mqctl, workerctl (lab.rs: the services of the lab)
//   script [list|status|start|stop|restart|errors|output] [NAME] (scripts.rs: the scripts)
//   dhclient [-v] [-r] [IF], cat /etc/resolv.conf|/etc/hosts|/etc/hostname (netsvc.rs)
//   ip -6 addr|route|neigh, ping6 / ping -6, traceroute6 / traceroute -6, sysctl net.ipv6.*
//   (linux6.rs; ip addr and ip neigh show both families, an IPv6 target picks IPv6 by itself)
//   dig, host, nslookup (names.rs: their own questions to a name server)
//   iw dev [IF info|link|scan|connect SSID [psk PASS]|disconnect|station dump] (cli/wlan.rs:
//   WLAN; wlan0 shows in ip link as any port, NO-CARRIER until the handshake is done)
//   ls, cat FILE, rm, cp, mkdir, rmdir, touch, echo ... > FILE, tftp -g|-p (files.rs: the
//   device's files); curl ftp://... (ftpc.rs)
//
// Objects and commands of ip take any prefix, the first one of iproute2's order winning ("ip
// a" is address, "ip r" route, "ip n" neighbour); output and error messages are those of
// iproute2 6 and iputils (the usage texts are our own).  Engine::linux_config writes the device's state as these commands
// (the startup configuration of a Linux device is such a script).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::{F_BRIDGE, F_HOST, F_ROUTER};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::action::Action;
use crate::driver::Report;
use crate::ip::{IfKind, RouteSrc};
use crate::jobs::{PingOpts, TraceOpts};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::resolver::Family;

/// the words of a shell line: spaces separate, '...' and "..." quote
fn words(line: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                any = true;
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    v.push(core::mem::take(&mut cur));
                }
                any = false;
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        v.push(cur);
    }
    v
}

/// iproute2's matches(): `w` is a prefix of `full`
pub(super) fn is(w: &str, full: &str) -> bool {
    !w.is_empty() && full.starts_with(w)
}

/// "0.2" seconds -> ns (at most microseconds)
pub(super) fn secs(s: &str) -> Option<Ns> {
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    if (i.is_empty() && f.is_empty()) || !i.bytes().all(|b| b.is_ascii_digit()) || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ns = if i.is_empty() { 0 } else { i.parse::<u64>().ok()?.checked_mul(SEC)? };
    let f = &f[..f.len().min(6)];
    if !f.is_empty() {
        ns += f.parse::<u64>().ok()? * 10u64.pow(6 - f.len() as u32) * 1000;
    }
    Some(ns)
}

const IP_USAGE: &str = "Usage: ip [-4] [-br] OBJECT [COMMAND ...]\n  OBJECT   address, link, neighbour or route (a unique prefix is enough)\n  COMMAND  show (the default), add, del and more: ip OBJECT help\n";
const ADDR_USAGE: &str = "Usage: ip address [show [dev] IF]\n       ip address add|del PREFIX dev IF\n       ip address flush dev IF\n";
const LINK_USAGE: &str = "Usage: ip link [show [dev] IF]\n       ip link set [dev] IF up|down|mtu N\n       ip link add link IF name NAME type vlan id N\n       ip link add NAME type dummy\n       ip link del IF\n";
const ROUTE_USAGE: &str = "Usage: ip route [show]\n       ip route add|del PREFIX|default [via ADDR] [dev IF] [metric N]\n       ip route get ADDR\n";
const NEIGH_USAGE: &str = "Usage: ip neigh [show]\n       ip neigh add ADDR lladdr MAC dev IF\n       ip neigh del ADDR dev IF\n       ip neigh flush all|dev IF\n";

impl Engine {
    pub(super) fn sh_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.out(ni, text);
        Err(String::from(text.lines().next().unwrap_or("")))
    }

    /// a line typed into the shell
    pub(crate) fn linux_line(&mut self, ni: usize, line: &str) -> Result<(), String> {
        // "printf ... | nc host port" (tools.rs)
        if let Some((left, right)) = super::tools::split_pipe(line) {
            return self.sh_pipe(ni, left, right);
        }
        let mut w = words(line);
        if w.first().is_some_and(|x| x == "sudo") {
            w.remove(0);
        }
        let Some(cmd) = w.first().cloned() else {
            return Ok(());
        };
        if cmd.starts_with('#') {
            return Ok(());
        }
        let args: Vec<&str> = w[1..].iter().map(String::as_str).collect();
        match cmd.as_str() {
            "ip" => self.sh_ip(ni, &args),
            "ping" => self.sh_ping(ni, &args),
            "traceroute" => self.sh_traceroute(ni, &args),
            "ping6" => self.sh_ping6(ni, &args),
            "traceroute6" => self.sh_traceroute6(ni, &args),
            "hostname" => self.sh_hostname(ni, &args),
            "sysctl" => self.sh_sysctl(ni, &args),
            "curl" => self.sh_curl(ni, &args),
            "nc" | "netcat" => self.sh_nc(ni, &args, None),
            "ss" => self.sh_ss(ni, &args),
            "top" => self.sh_top(ni, &args),
            "dig" => self.sh_dig(ni, &args),
            "host" => self.sh_host(ni, &args),
            "nslookup" => self.sh_nslookup(ni, &args),
            "printf" => self.sh_printf(ni, line),
            "lbctl" => self.sh_lbctl(ni, &args),
            "kvctl" => self.sh_kvctl(ni, &args),
            "cachectl" => self.sh_cachectl(ni, &args),
            "mqctl" => self.sh_mqctl(ni, &args),
            "workerctl" => self.sh_workerctl(ni, &args),
            "script" => self.sh_script(ni, &args),
            "dhclient" => self.sh_dhclient(ni, &args),
            "cat" => self.sh_cat(ni, &args),
            "iw" => self.sh_iw(ni, &args),
            // with "> FILE" into a file (files.rs)
            "echo" => self.sh_echo(ni, &args),
            "ls" => self.sh_ls(ni, &args),
            "rm" => self.sh_rm(ni, &args),
            "cp" => self.sh_cp(ni, &args),
            "mkdir" => self.sh_mkdir(ni, &args),
            "rmdir" => self.sh_rmdir(ni, &args),
            "touch" => self.sh_touch(ni, &args),
            "tftp" => self.sh_tftp(ni, &args),
            "telnet" => self.sh_telnet(ni, &args),
            "clear" => {
                self.out(ni, "\x1b[H\x1b[2J");
                Ok(())
            }
            "true" | "exit" | "logout" => Ok(()),
            "false" => Err(String::new()),
            _ => self.sh_err(ni, &format!("{}: command not found\n", cmd)),
        }
    }

    /// Tab: command names and the objects of ip
    pub(crate) fn linux_complete(&self, _ni: usize, line: &str) -> (Option<String>, Vec<String>) {
        let w = words(line);
        let partial = !line.is_empty() && !line.ends_with(char::is_whitespace);
        let (list, done): (&[&str], usize) = match w.len() {
            1 if partial => (&["cachectl", "cat", "clear", "curl", "dhclient", "dig", "echo", "host", "hostname", "ip", "iw", "kvctl", "lbctl", "mqctl", "nc", "nslookup", "ping", "printf", "script", "ss", "sysctl", "telnet", "top", "traceroute", "workerctl", "cp", "ls", "mkdir", "rm", "rmdir", "tftp", "touch"], 0),
            2 if partial && w[0] == "ip" => (&["address", "link", "neighbour", "route"], 1),
            3 if partial && w[0] == "ip" => (&["add", "del", "flush", "get", "set", "show"], 2),
            _ => return (None, Vec::new()),
        };
        let last = &w[done];
        let mut choices: Vec<String> = list.iter().filter(|c| c.starts_with(last.as_str())).map(|c| String::from(*c)).collect();
        choices.sort();
        let head_len = line.rfind(last.as_str()).unwrap_or(0);
        let out = if choices.len() == 1 { Some(format!("{}{} ", &line[..head_len], choices[0])) } else { None };
        (out, choices)
    }

    // ---------------------------------------------------------------- ip

    fn sh_ip(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut brief = false;
        // the address family: 0 both, 4, 6 (linux6.rs)
        let mut family = 0u8;
        let mut i = 0;
        while let Some(a) = args.get(i) {
            match *a {
                "-4" => family = 4,
                "-6" => family = 6,
                "-br" | "-brief" => brief = true,
                "-help" => {
                    self.out(ni, IP_USAGE);
                    return Ok(());
                }
                o if o.starts_with('-') => return self.sh_err(ni, &format!("Option \"{}\" is unknown, try \"ip -help\".\n", o)),
                _ => break,
            }
            i += 1;
        }
        let rest = &args[i..];
        let Some(obj) = rest.first() else {
            return self.sh_err(ni, IP_USAGE);
        };
        let sub = &rest[1..];
        let usage = if is(obj, "address") {
            ADDR_USAGE
        } else if is(obj, "route") {
            ROUTE_USAGE
        } else if is(obj, "neighbor") || is(obj, "neighbour") {
            NEIGH_USAGE
        } else if is(obj, "link") {
            LINK_USAGE
        } else {
            ""
        };
        if !usage.is_empty() && sub.first().is_some_and(|c| is(c, "help")) {
            self.out(ni, usage);
            return Ok(());
        }
        if is(obj, "address") {
            if family == 6 {
                return self.sh_addr6(ni, sub, brief);
            }
            self.sh_addr(ni, sub, brief, family == 0)
        } else if is(obj, "route") {
            if family == 6 {
                return self.sh_route6(ni, sub);
            }
            self.sh_route(ni, sub)
        } else if is(obj, "neighbor") || is(obj, "neighbour") {
            if family == 6 {
                return self.sh_neigh6(ni, sub);
            }
            let r = self.sh_neigh(ni, sub);
            if family == 0 && r.is_ok() {
                // both families: the IPv6 entries after the IPv4 ones, a flush takes both
                let cmd = sub.first().copied().unwrap_or("show");
                if is(cmd, "flush") {
                    let dev = self.dev_of(ni);
                    let _ = self.apply(Action::Ip6 { dev, set: crate::ip6::Ip6Set::ClearNeighbors { iface: None } });
                } else if is(cmd, "show") || is(cmd, "list") || sub.is_empty() {
                    let s = self.neigh6_lines(ni);
                    self.out(ni, &s);
                }
            }
            r
        } else if is(obj, "link") {
            self.sh_link(ni, sub, brief)
        } else if is(obj, "help") {
            self.out(ni, IP_USAGE);
            Ok(())
        } else {
            self.sh_err(ni, &format!("Object \"{}\" is unknown, try \"ip help\".\n", obj))
        }
    }

    /// an interface by its Linux name (removed ones are not found)
    pub(super) fn sh_iface(&self, ni: usize, name: &str) -> Option<u16> {
        let st = self.stack(ni)?;
        st.ifaces.iter().position(|f| !f.hidden && f.name == name).map(|i| i as u16)
    }

    /// "dev IF" or a bare IF
    fn sh_dev<'a>(args: &[&'a str]) -> Option<&'a str> {
        let mut i = 0;
        while i < args.len() {
            if args[i] == "dev" {
                return args.get(i + 1).copied();
            }
            i += 1;
        }
        args.iter().copied().find(|a| !a.starts_with('-'))
    }

    /// the two lines of `ip link` for an interface: "2: eth0: <...> mtu .. state UP" and
    /// "link/ether ..."
    pub(super) fn link_line(&self, ni: usize, i: u16) -> (String, String) {
        let Some(st) = self.stack(ni) else {
            return (String::new(), String::new());
        };
        let f = &st.ifaces[i as usize];
        let up = self.iface_up(ni, i);
        let admin = self.ifaces_admin(ni, i);
        let carrier = match f.kind {
            IfKind::Port(p) | IfKind::Sub { port: p, .. } => self.nodes[ni].as_ref().is_some_and(|n| n.ports[p as usize].link.is_some_and(|li| self.link_up(li))),
            _ => up,
        };
        let name = match f.kind {
            IfKind::Sub { port, .. } => format!("{}@{}", f.name, self.nodes[ni].as_ref().map(|n| n.port_name(port)).unwrap_or("?")),
            _ => f.name.clone(),
        };
        let (flags, qdisc, state, mtu, link) = match f.kind {
            IfKind::Loopback(0) if f.name == "lo" => (String::from(if admin { "LOOPBACK,UP,LOWER_UP" } else { "LOOPBACK" }), "noqueue", if admin { "UNKNOWN" } else { "DOWN" }, 65536u32, String::from("link/loopback 00:00:00:00:00:00 brd 00:00:00:00:00:00")),
            IfKind::Loopback(_) => (String::from(if admin { "BROADCAST,NOARP,UP,LOWER_UP" } else { "BROADCAST,NOARP" }), "noqueue", if admin { "UNKNOWN" } else { "DOWN" }, f.mtu as u32, format!("link/ether {} brd ff:ff:ff:ff:ff:ff", f.mac)),
            _ => {
                let fl = match (admin, carrier) {
                    (true, true) => "BROADCAST,MULTICAST,UP,LOWER_UP",
                    (true, false) => "NO-CARRIER,BROADCAST,MULTICAST,UP",
                    _ => "BROADCAST,MULTICAST",
                };
                let q = if matches!(f.kind, IfKind::Port(_)) { "fq_codel" } else { "noqueue" };
                (String::from(fl), q, if up { "UP" } else { "DOWN" }, f.mtu as u32, format!("link/ether {} brd ff:ff:ff:ff:ff:ff", f.mac))
            }
        };
        (format!("{}: {}: <{}> mtu {} qdisc {} state {}", i + 1, name, flags, mtu, qdisc, state), link)
    }

    /// not shut down
    fn ifaces_admin(&self, ni: usize, i: u16) -> bool {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return false;
        };
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(i as usize)) else {
            return false;
        };
        match f.kind {
            IfKind::Port(p) => n.ports.get(p as usize).is_some_and(|x| x.admin_up),
            _ => f.admin_up,
        }
    }

    pub(super) fn visible_ifaces(&self, ni: usize) -> Vec<u16> {
        self.stack(ni).map(|s| (0..s.ifaces.len() as u16).filter(|&i| !s.ifaces[i as usize].hidden).collect()).unwrap_or_default()
    }

    fn sh_addr(&mut self, ni: usize, args: &[&str], brief: bool, v6: bool) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "delete") {
            let add = is(cmd, "add");
            let Some(p) = args.get(1) else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            let pfx = match Prefix4::parse(p).or_else(|| Ipv4::parse(p).map(|a| Prefix4 { addr: a, len: 32 })) {
                Some(x) => x,
                None => return self.sh_err(ni, &format!("Error: any valid prefix is expected rather than \"{}\".\n", p)),
            };
            let Some(name) = args.iter().position(|a| *a == "dev").and_then(|i| args.get(i + 1)) else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            let Some(i) = self.sh_iface(ni, name) else {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            };
            let have = self.stack(ni).is_some_and(|s| s.ifaces[i as usize].addrs.contains(&pfx));
            if add && have {
                return self.sh_err(ni, "RTNETLINK answers: File exists\n");
            }
            if !add && !have {
                return self.sh_err(ni, "RTNETLINK answers: Cannot assign requested address\n");
            }
            return match self.apply(Action::IfAddr { dev, iface: String::from(*name), prefix: pfx, add }) {
                Ok(()) => Ok(()),
                Err(_) => self.sh_err(ni, "RTNETLINK answers: Invalid argument\n"),
            };
        }
        if is(cmd, "flush") {
            let Some(name) = Self::sh_dev(&args[1..]) else {
                return self.sh_err(ni, "Flush requires arguments.\n");
            };
            let Some(i) = self.sh_iface(ni, name) else {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            };
            let addrs = self.stack(ni).map(|s| s.ifaces[i as usize].addrs.clone()).unwrap_or_default();
            for a in addrs {
                let _ = self.apply(Action::IfAddr { dev, iface: String::from(name), prefix: a, add: false });
            }
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list") || is(cmd, "lst")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip address help\".\n", cmd));
        }
        let only = if args.is_empty() { None } else { Self::sh_dev(&args[1..]) };
        if let Some(n) = only
            && self.sh_iface(ni, n).is_none()
        {
            return self.sh_err(ni, &format!("Device \"{}\" does not exist.\n", n));
        }
        let mut s = String::new();
        for i in self.visible_ifaces(ni) {
            let Some(st) = self.stack(ni) else {
                break;
            };
            let f = &st.ifaces[i as usize];
            if only.is_some_and(|n| n != f.name) {
                continue;
            }
            let (head, link) = self.link_line(ni, i);
            if brief {
                let state = head.rsplit_once("state ").map(|x| x.1).unwrap_or("UNKNOWN");
                let name = head.split(": ").nth(1).unwrap_or("");
                let mut addrs: Vec<String> = f.addrs.iter().map(|a| format!("{}", a)).collect();
                if v6 {
                    addrs.extend(self.brief6(ni, i));
                }
                s.push_str(format!("{:<16} {:<14} {}", name, state, addrs.join(" ")).trim_end());
                s.push('\n');
                continue;
            }
            s.push_str(&format!("{} group default qlen 1000\n    {}\n", head, link));
            for (k, a) in f.addrs.iter().enumerate() {
                let scope = if f.name == "lo" || a.addr.is_loopback() { "host" } else { "global" };
                let brd = if a.len <= 30 && scope == "global" { format!(" brd {}", a.broadcast()) } else { String::new() };
                let sec = if k > 0 { " secondary" } else { "" };
                s.push_str(&format!("    inet {}{} scope {}{} {}\n       valid_lft forever preferred_lft forever\n", a, brd, scope, sec, f.name));
            }
            if v6 {
                s.push_str(&self.inet6_lines(ni, i));
            }
        }
        self.out(ni, &s);
        Ok(())
    }

    fn sh_link(&mut self, ni: usize, args: &[&str], brief: bool) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "set") {
            let rest = &args[1..];
            let (name, ops) = match rest.first() {
                Some(&"dev") => (rest.get(1).copied(), &rest[rest.len().min(2)..]),
                Some(n) => (Some(*n), &rest[1..]),
                None => (None, rest),
            };
            let Some(name) = name else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            if self.sh_iface(ni, name).is_none() {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            }
            let mut k = 0;
            while k < ops.len() {
                match ops[k] {
                    "up" | "down" => {
                        let _ = self.apply(Action::IfUp { dev, iface: String::from(name), up: ops[k] == "up" });
                    }
                    "mtu" => {
                        let m = ops.get(k + 1).and_then(|m| m.parse::<u16>().ok());
                        match m {
                            Some(m) if m >= 68 => {
                                let _ = self.apply(Action::Mtu { dev, iface: String::from(name), mtu: m });
                            }
                            _ => return self.sh_err(ni, &format!("Error: argument \"{}\" is wrong: Invalid \"mtu\" value\n\n", ops.get(k + 1).copied().unwrap_or(""))),
                        }
                        k += 1;
                    }
                    o => return self.sh_err(ni, &format!("Error: either \"dev\" is duplicate, or \"{}\" is a garbage.\n", o)),
                }
                k += 1;
            }
            return Ok(());
        }
        if is(cmd, "add") {
            // ip link add link eth0 name eth0.10 type vlan id 10 | ip link add dummy0 type dummy
            let get = |k: &str| args.iter().position(|a| *a == k).and_then(|i| args.get(i + 1)).copied();
            return match (get("type"), get("link"), get("name"), get("id")) {
                (Some("vlan"), Some(base), name, Some(id)) => {
                    let id: u16 = id.parse().unwrap_or(0);
                    let name = name.map(String::from).unwrap_or_else(|| format!("{}.{}", base, id));
                    let want = format!("{}.{}", base, name.rsplit_once('.').map(|x| x.1).unwrap_or(""));
                    if name != want {
                        return self.sh_err(ni, "RTNETLINK answers: Operation not supported\n");
                    }
                    match self.apply(Action::SubIf { dev, name, vlan: id, add: true }) {
                        Ok(()) => Ok(()),
                        Err(_) => self.sh_err(ni, "RTNETLINK answers: File exists\n"),
                    }
                }
                (Some("dummy"), _, _, _) => {
                    let name = args.get(1).copied().unwrap_or("dummy0");
                    let n: u16 = name.strip_prefix("dummy").and_then(|x| x.parse().ok()).unwrap_or(u16::MAX);
                    if n == u16::MAX {
                        return self.sh_err(ni, "RTNETLINK answers: Operation not supported\n");
                    }
                    match self.apply(Action::Loopback { dev, n, add: true }) {
                        Ok(()) => Ok(()),
                        Err(_) => self.sh_err(ni, "RTNETLINK answers: File exists\n"),
                    }
                }
                _ => self.sh_err(ni, "Not enough information: \"type\" argument is required.\n"),
            };
        }
        if is(cmd, "delete") {
            let Some(name) = Self::sh_dev(&args[1..]) else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            let r = if let Some(n) = name.strip_prefix("dummy").and_then(|x| x.parse::<u16>().ok()) {
                self.apply(Action::Loopback { dev, n, add: false })
            } else if name.contains('.') {
                self.apply(Action::SubIf { dev, name: String::from(name), vlan: 0, add: false })
            } else {
                return self.sh_err(ni, "RTNETLINK answers: Operation not supported\n");
            };
            return match r {
                Ok(()) => Ok(()),
                Err(_) => self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name)),
            };
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip link help\".\n", cmd));
        }
        let only = if args.is_empty() { None } else { Self::sh_dev(&args[1..]) };
        if let Some(n) = only
            && self.sh_iface(ni, n).is_none()
        {
            return self.sh_err(ni, &format!("Device \"{}\" does not exist.\n", n));
        }
        let mut s = String::new();
        for i in self.visible_ifaces(ni) {
            let name = self.stack(ni).map(|st| st.ifaces[i as usize].name.clone()).unwrap_or_default();
            if only.is_some_and(|n| n != name) {
                continue;
            }
            let (head, link) = self.link_line(ni, i);
            if brief {
                let state = head.rsplit_once("state ").map(|x| x.1).unwrap_or("UNKNOWN");
                let shown = head.split(": ").nth(1).unwrap_or("");
                let mac = link.split_whitespace().nth(1).unwrap_or("");
                let flags = head.split_once('<').and_then(|x| x.1.split_once('>')).map(|x| x.0).unwrap_or("");
                s.push_str(&format!("{:<16} {:<14} {} <{}>\n", shown, state, mac, flags));
                continue;
            }
            s.push_str(&format!("{} mode DEFAULT group default qlen 1000\n    {}\n", head, link));
        }
        self.out(ni, &s);
        Ok(())
    }

    fn sh_route(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "delete") || is(cmd, "replace") {
            let add = !is(cmd, "delete");
            let Some(target) = args.get(1) else {
                return self.sh_err(ni, "Error: inet prefix is expected rather than \"\".\n");
            };
            let prefix = if *target == "default" {
                Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 }
            } else {
                match Prefix4::parse(target).or_else(|| Ipv4::parse(target).map(|a| Prefix4 { addr: a, len: 32 })) {
                    Some(p) => p.trunc(),
                    None => return self.sh_err(ni, &format!("Error: inet prefix is expected rather than \"{}\".\n", target)),
                }
            };
            let mut via = None;
            let mut ifn: Option<String> = None;
            let mut k = 2;
            while k < args.len() {
                match args[k] {
                    "via" => {
                        let a = args.get(k + 1).copied().unwrap_or("");
                        match Ipv4::parse(a) {
                            Some(x) => via = Some(x),
                            None => return self.sh_err(ni, &format!("Error: inet address is expected rather than \"{}\".\n", a)),
                        }
                        k += 1;
                    }
                    "dev" => {
                        let n = args.get(k + 1).copied().unwrap_or("");
                        if self.sh_iface(ni, n).is_none() {
                            return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", n));
                        }
                        ifn = Some(String::from(n));
                        k += 1;
                    }
                    "metric" | "proto" | "src" | "scope" | "table" => k += 1,
                    "onlink" => {}
                    o => return self.sh_err(ni, &format!("Error: either \"to\" is duplicate, or \"{}\" is a garbage.\n", o)),
                }
                k += 1;
            }
            if !add {
                return match self.apply(Action::Route { dev, prefix, via, iface: None, ad: 1, add: false }) {
                    Ok(()) => Ok(()),
                    Err(_) => self.sh_err(ni, "RTNETLINK answers: No such process\n"),
                };
            }
            if via.is_none() && ifn.is_none() {
                return self.sh_err(ni, "Error: Device for nexthop is not up.\n");
            }
            if let Some(gw) = via {
                // the next hop must be on a network of an interface that is not down (without
                // a carrier is fine: the route waits, "linkdown")
                let ok = self.stack(ni).is_some_and(|s| {
                    s.ifaces.iter().enumerate().any(|(i, f)| !f.hidden && f.on_link(gw) && ifn.as_ref().is_none_or(|n| *n == f.name) && self.ifaces_admin(ni, i as u16))
                });
                if !ok {
                    return self.sh_err(ni, "Error: Nexthop has invalid gateway.\n");
                }
            }
            let exists = self.stack(ni).is_some_and(|s| s.statics.iter().any(|r| r.prefix == prefix && r.via == via));
            if exists && !is(cmd, "replace") {
                return self.sh_err(ni, "RTNETLINK answers: File exists\n");
            }
            return match self.apply(Action::Route { dev, prefix, via, iface: ifn, ad: 1, add: true }) {
                Ok(()) => Ok(()),
                Err(_) => self.sh_err(ni, "Error: Nexthop has invalid gateway.\n"),
            };
        }
        if is(cmd, "get") {
            let a = args.get(1).copied().unwrap_or("");
            let Some(dst) = Ipv4::parse(a) else {
                return self.sh_err(ni, &format!("Error: inet prefix is expected rather than \"{}\".\n", a));
            };
            let text = match self.route(ni, dst) {
                Some(nh) => {
                    let st = self.stack(ni);
                    let f = st.and_then(|s| s.ifaces.get(nh.iface as usize));
                    let name = f.map(|f| f.name.clone()).unwrap_or_default();
                    let src = self.source_for(ni, dst).unwrap_or(Ipv4::UNSPECIFIED);
                    if self.stack(ni).is_some_and(|s| s.is_mine(dst)) {
                        format!("local {} dev lo src {} uid 0 \n    cache <local> \n", dst, src)
                    } else if nh.gw == dst {
                        format!("{} dev {} src {} uid 0 \n    cache \n", dst, name, src)
                    } else {
                        format!("{} via {} dev {} src {} uid 0 \n    cache \n", dst, nh.gw, name, src)
                    }
                }
                None => return self.sh_err(ni, "RTNETLINK answers: Network is unreachable\n"),
            };
            self.out(ni, &text);
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip route help\".\n", cmd));
        }
        let mut s = String::new();
        let routes = self.routes(dev);
        let st = self.stack(ni);
        let name = |i: Option<u16>| i.and_then(|i| st.and_then(|s| s.ifaces.get(i as usize))).map(|f| f.name.clone()).unwrap_or_default();
        // default routes first, then by network (the kernel's main table order)
        let mut lines: Vec<(u32, u8, String)> = Vec::new();
        for r in &routes {
            let net = if r.prefix.len == 0 { String::from("default") } else if r.prefix.len == 32 { format!("{}", r.prefix.addr) } else { format!("{}", r.prefix) };
            let l = match r.src {
                RouteSrc::Local => continue,
                RouteSrc::Connected => {
                    let f = r.iface.and_then(|i| st.and_then(|s| s.ifaces.get(i as usize)));
                    if f.is_some_and(|f| f.name == "lo" || f.is_loopback() && f.addrs.iter().any(|a| a.addr.is_loopback())) {
                        continue;
                    }
                    let src = f.and_then(|f| f.addrs.iter().find(|a| a.trunc() == r.prefix).map(|a| a.addr)).unwrap_or(Ipv4::UNSPECIFIED);
                    format!("{} dev {} proto kernel scope link src {}", net, name(r.iface), src)
                }
                _ => match r.via {
                    Some(v) => format!("{} via {} dev {}", net, v, name(r.iface)),
                    None => format!("{} dev {} scope link", net, name(r.iface)),
                },
            };
            lines.push((if r.prefix.len == 0 { 0 } else { 1 }, 0, l));
        }
        lines.sort_by_key(|l| (l.0, l.1));
        for (_, _, l) in lines {
            s.push_str(&l);
            s.push('\n');
        }
        self.out(ni, &s);
        Ok(())
    }

    fn sh_neigh(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "replace") || is(cmd, "delete") {
            let add = !is(cmd, "delete");
            let a = args.get(1).copied().unwrap_or("");
            let Some(ip) = Ipv4::parse(a) else {
                return self.sh_err(ni, &format!("Error: inet address is expected rather than \"{}\".\n", a));
            };
            let get = |k: &str| args.iter().position(|x| *x == k).and_then(|i| args.get(i + 1)).copied();
            let Some(name) = get("dev") else {
                return self.sh_err(ni, "Device and destination are required arguments.\n");
            };
            if self.sh_iface(ni, name).is_none() {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            }
            let mac = if add {
                let m = get("lladdr").unwrap_or("");
                match Mac::parse(m) {
                    Some(x) => Some(x),
                    None => return self.sh_err(ni, &format!("\"{}\" is invalid lladdr.\n", m)),
                }
            } else {
                None
            };
            return match self.apply(Action::StaticArp { dev, ip, mac }) {
                Ok(()) => Ok(()),
                Err(_) if add => self.sh_err(ni, "Error: Nexthop has invalid gateway.\n"),
                Err(_) => self.sh_err(ni, "RTNETLINK answers: No such file or directory\n"),
            };
        }
        if is(cmd, "flush") {
            let _ = self.apply(Action::ClearArp { dev });
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip neigh help\".\n", cmd));
        }
        let mut s = String::new();
        let st = self.stack(ni);
        for r in self.arp_table(dev) {
            let name = st.and_then(|s| s.ifaces.get(r.iface as usize)).map(|f| f.name.clone()).unwrap_or_default();
            match r.mac {
                Some(m) => {
                    let state = if r.fixed {
                        "PERMANENT"
                    } else if r.age < 30 * SEC {
                        "REACHABLE"
                    } else {
                        "STALE"
                    };
                    s.push_str(&format!("{} dev {} lladdr {} {}\n", r.ip, name, m, state));
                }
                None => s.push_str(&format!("{} dev {} INCOMPLETE\n", r.ip, name)),
            }
        }
        self.out(ni, &s);
        Ok(())
    }

    // ---------------------------------------------------------------- ping, traceroute

    fn sh_ping(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        // "-6" or an IPv6 address: iputils pings over IPv6 (linux6.rs)
        if args.contains(&"-6") || args.iter().any(|a| !a.starts_with('-') && a.contains(':')) {
            return self.sh_ping6(ni, args);
        }
        let mut o = PingOpts { count: 0, size: 56, ttl: None, df: false, interval: SEC, timeout: 10 * SEC, src: None };
        let mut dst: Option<&str> = None;
        // "-4": a name is looked up for IPv4 only; else for what the device can use
        // (getaddrinfo: an IPv6 address first makes it a ping over IPv6)
        let mut fam = Family::Any;
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            if let Some(opt) = a.strip_prefix('-').filter(|x| !x.is_empty()) {
                let (flag, inline) = opt.split_at(1);
                let val = if inline.is_empty() && "cistWwMI".contains(flag) {
                    k += 1;
                    args.get(k).copied()
                } else {
                    Some(inline)
                };
                let bad = |v: Option<&str>| format!("ping: invalid argument: '{}'\n", v.unwrap_or(""));
                match flag {
                    "c" => match val.and_then(|v| v.parse::<u32>().ok()).filter(|&c| c > 0) {
                        Some(c) => o.count = c,
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "i" => match val.and_then(secs) {
                        Some(t) if t >= 2 * MS => o.interval = t,
                        Some(_) => return self.sh_err(ni, "ping: cannot flood, minimal interval for user must be >= 2 ms, use -i 0.002 (or higher)\n"),
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "s" => match val.and_then(|v| v.parse::<u16>().ok()).filter(|&s| s <= 65507) {
                        Some(s) => o.size = s,
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "t" => match val.and_then(|v| v.parse::<u8>().ok()).filter(|&t| t > 0) {
                        Some(t) => o.ttl = Some(t),
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "W" => match val.and_then(secs) {
                        Some(t) => o.timeout = t.max(MS),
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "M" => match val {
                        Some("do") => o.df = true,
                        Some("dont" | "want") => o.df = false,
                        _ => return self.sh_err(ni, &format!("ping: invalid -M argument: {}\n", val.unwrap_or(""))),
                    },
                    "4" => fam = Family::V4,
                    "n" | "q" => {}
                    "w" | "I" => {}
                    f => return self.sh_err(ni, &format!("ping: invalid option -- '{}'\n", f)),
                }
            } else {
                dst = Some(a);
            }
            k += 1;
        }
        let Some(d) = dst else {
            return self.sh_err(ni, "ping: usage error: Destination address required\n");
        };
        // a name goes to the device's resolver first (print.rs)
        self.linux_ping_start(ni, d, o, fam);
        Ok(())
    }

    fn sh_traceroute(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        if args.contains(&"-6") || args.iter().any(|a| !a.starts_with('-') && a.contains(':')) {
            return self.sh_traceroute6(ni, args);
        }
        let mut o = TraceOpts { max_hops: 30, probes: 3, timeout: 5 * SEC, port: 33434, size: 32 };
        let mut numeric = false;
        let mut dst: Option<&str> = None;
        let mut fam = Family::Any;
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            match a {
                "-n" => numeric = true,
                "-4" => fam = Family::V4,
                "-m" | "-q" | "-w" => {
                    k += 1;
                    let v = args.get(k).copied().unwrap_or("");
                    match a {
                        "-m" => match v.parse::<u8>() {
                            Ok(m) if m > 0 => o.max_hops = m,
                            _ => return self.sh_err(ni, &format!("Cannot handle `-m' option with arg `{}' (argc {})\n", v, k + 1)),
                        },
                        "-q" => match v.parse::<u8>() {
                            Ok(q) if (1..=10).contains(&q) => o.probes = q,
                            _ => return self.sh_err(ni, &format!("Cannot handle `-q' option with arg `{}' (argc {})\n", v, k + 1)),
                        },
                        _ => match secs(v) {
                            Some(t) => o.timeout = t.max(MS),
                            None => return self.sh_err(ni, &format!("Cannot handle `-w' option with arg `{}' (argc {})\n", v, k + 1)),
                        },
                    }
                }
                x if x.starts_with('-') => return self.sh_err(ni, &format!("Bad option `{}' (argc {})\n", x, k + 1)),
                x => dst = Some(x),
            }
            k += 1;
        }
        let Some(d) = dst else {
            return self.sh_err(ni, "Specify \"host\" missing argument.\n");
        };
        // a name goes to the device's resolver first (print.rs)
        self.linux_trace_start(ni, d, o, numeric, fam);
        Ok(())
    }

    fn sh_hostname(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match args.first() {
            None => {
                let h = format!("{}\n", self.host(ni));
                self.out(ni, &h);
                Ok(())
            }
            Some(name) => {
                if !jfn_model::project::valid_name(name) {
                    return self.sh_err(ni, "hostname: the specified hostname is invalid\n");
                }
                let name = String::from(*name);
                // (a startup configuration that boots renames nothing: the project keeps its name)
                let booting = self.cdev(ni).is_some_and(|c| c.booting);
                if name != self.host(ni) && self.apply(Action::Hostname { dev, name: name.clone() }).is_ok() && !booting {
                    self.cli.reports.push(Report::Hostname { dev, name });
                }
                Ok(())
            }
        }
    }

    fn sh_sysctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        for a in args.iter().filter(|a| !a.starts_with('-')) {
            let (key, val) = match a.split_once('=') {
                Some((k, v)) => (k.trim(), Some(v.trim())),
                None => (a.trim(), None),
            };
            if key.starts_with("net.ipv6.") || key.starts_with("net/ipv6/") {
                match self.sysctl6(ni, key, val) {
                    Ok(l) => s.push_str(&l),
                    Err(e) => return self.sh_err(ni, &e),
                }
                continue;
            }
            if key != "net.ipv4.ip_forward" && key != "net/ipv4/ip_forward" {
                let path = key.replace('.', "/");
                return self.sh_err(ni, &format!("sysctl: cannot stat /proc/sys/{}: No such file or directory\n", path));
            }
            match val {
                Some(v) => {
                    let on = match v {
                        "1" => true,
                        "0" => false,
                        _ => return self.sh_err(ni, &format!("sysctl: setting key \"net.ipv4.ip_forward\": Invalid argument\n{}", "")),
                    };
                    let _ = self.apply(Action::Forwarding { dev, on });
                    s.push_str(&format!("net.ipv4.ip_forward = {}\n", v));
                }
                None => {
                    let on = self.stack(ni).is_some_and(|st| st.forwarding);
                    s.push_str(&format!("net.ipv4.ip_forward = {}\n", if on { 1 } else { 0 }));
                }
            }
        }
        self.out(ni, &s);
        Ok(())
    }

    // ---------------------------------------------------------------- configuration

    /// the device's state as shell commands (its "running configuration")
    pub(crate) fn linux_config(&self, ni: usize) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return String::new();
        };
        let mut s = format!("hostname {}\n", n.name);
        let Some(st) = n.ip.as_ref() else {
            return s;
        };
        let default = n.model.has(F_ROUTER) && (!n.model.has(F_BRIDGE) || n.model.has(F_HOST));
        if st.forwarding != default {
            s.push_str(&format!("sysctl -w net.ipv4.ip_forward={}\n", if st.forwarding { 1 } else { 0 }));
        }
        for (i, f) in st.ifaces.iter().enumerate() {
            if f.hidden {
                continue;
            }
            match f.kind {
                IfKind::Sub { port, vlan } => s.push_str(&format!("ip link add link {} name {} type vlan id {}\n", n.port_name(port), f.name, vlan)),
                IfKind::Loopback(_) if f.name != "lo" => s.push_str(&format!("ip link add {} type dummy\n", f.name)),
                _ => {}
            }
            let admin = self.ifaces_admin(ni, i as u16);
            let default_up = !matches!(f.kind, IfKind::Sub { .. });
            if admin != default_up {
                s.push_str(&format!("ip link set dev {} {}\n", f.name, if admin { "up" } else { "down" }));
            }
            let def_mtu = if f.name == "lo" { 65535 } else { 1500 };
            if f.mtu != def_mtu {
                s.push_str(&format!("ip link set dev {} mtu {}\n", f.name, f.mtu));
            }
            for a in &f.addrs {
                if f.name == "lo" && a.addr == Ipv4::LOCALHOST && a.len == 8 {
                    continue;
                }
                s.push_str(&format!("ip addr add {} dev {}\n", a, f.name));
            }
        }
        for r in &st.statics {
            let net = if r.prefix.len == 0 { String::from("default") } else { format!("{}", r.prefix) };
            let mut l = format!("ip route add {}", net);
            if let Some(v) = r.via {
                l.push_str(&format!(" via {}", v));
            }
            if let Some(i) = r.iface {
                l.push_str(&format!(" dev {}", st.ifaces[i as usize].name));
            }
            s.push_str(&l);
            s.push('\n');
        }
        for ((i, ip), e) in &st.arp {
            if e.fixed
                && let Some(m) = e.mac
            {
                s.push_str(&format!("ip neigh add {} lladdr {} dev {}\n", ip, m, st.ifaces[*i as usize].name));
            }
        }
        // IPv6 (linux6.rs)
        s.push_str(&self.linux_config6(ni));
        s
    }
}
