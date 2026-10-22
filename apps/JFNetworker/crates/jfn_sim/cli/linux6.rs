// linux6 -- IPv6 in the shell of hosts and servers, as iproute2, iputils and procps show it
//
//   ip -6 addr [show [dev IF]] | add|del ADDR[/LEN] dev IF | flush dev IF
//   ip -6 route [show] | add|del|replace PREFIX|default [via ADDR] [dev IF] [metric N] | get ADDR
//   ip -6 neigh [show] | add|replace ADDR lladdr MAC dev IF | del ADDR dev IF | flush all|dev IF
//   ip addr / ip -br addr / ip neigh   also list the IPv6 addresses and neighbors (inet6_lines,
//                          brief6, neigh6_lines)
//   ping -6 / ping6 [-c N] [-i S] [-s N] [-t HOPS] [-W S] [-M do] [-I IF] ADDR[%IF]
//   traceroute -6 / traceroute6 [-n] [-m MAX] [-q N] [-w S] [-i IF] ADDR[%IF]
//   sysctl [-w] net.ipv6.conf.all|default|IF.disable_ipv6|forwarding|accept_ra|autoconf|
//          hop_limit[=V]
//   Engine::linux_config6   the IPv6 part of the device's configuration as such commands
//
// The interfaces start with IPv6 off (as with net.ipv6.conf.all.disable_ipv6 = 1): sysctl turns
// it on.  Names are not looked up for IPv6 yet (no AAAA lookups in the resolver): ping and
// traceroute say so as glibc does for a family a name has no address in.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::{Ipv6, Mac, Prefix6};

use super::linux::{is, secs};
use super::print::JobOut;
use crate::action::Action;
use crate::ip6::{AddrCfg, AddrState, Ip6Set, Origin6, Route6, Src6};
use crate::jobs::TraceOpts;
use crate::jobs6::Ping6Opts;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::nd::Addr6View;

/// "fe80::1%eth0" -> (address text, interface name)
fn zone(s: &str) -> (&str, Option<&str>) {
    match s.split_once('%') {
        Some((a, z)) => (a, Some(z)),
        None => (s, None),
    }
}

/// the scope word of iproute2
fn scope(a: Ipv6) -> &'static str {
    if a.is_loopback() {
        "host"
    } else if a.is_link_local() {
        "link"
    } else {
        "global"
    }
}

/// the flags iproute2 prints after the scope (its order: dadfailed, deprecated, tentative,
/// dynamic)
fn flags(a: &Addr6View) -> String {
    let mut s = String::new();
    if a.state == AddrState::Duplicate {
        s.push_str("dadfailed ");
    }
    if a.state == AddrState::Deprecated {
        s.push_str("deprecated ");
    }
    if matches!(a.state, AddrState::Tentative | AddrState::Duplicate) {
        s.push_str("tentative ");
    }
    if a.origin == Origin6::Slaac {
        s.push_str("dynamic ");
    }
    s
}

fn lft(x: Option<Ns>) -> String {
    match x {
        Some(n) => format!("{}sec", n / SEC),
        None => String::from("forever"),
    }
}

impl Engine {
    /// the addresses of an interface in the kernel's order: global ones before link-local ones,
    /// the newest first within a scope (ipv6_link_dev_addr)
    fn sorted6(&self, ni: usize, i: u16) -> Vec<Addr6View> {
        let mut v = self.addrs6_of(ni, i);
        v.reverse();
        v.sort_by_key(|a| a.addr.is_link_local());
        v
    }

    /// the inet6 lines of an interface for "ip addr"
    pub(crate) fn inet6_lines(&self, ni: usize, i: u16) -> String {
        let mut s = String::new();
        for a in self.sorted6(ni, i) {
            let pref = if a.state == AddrState::Deprecated { Some(0) } else { a.preferred };
            s.push_str(&format!("    inet6 {}/{} scope {} {}\n       valid_lft {} preferred_lft {}\n", a.addr, a.len, scope(a.addr), flags(&a), lft(a.valid), lft(pref)));
        }
        s
    }

    /// the IPv6 addresses of an interface for "ip -br addr"
    pub(crate) fn brief6(&self, ni: usize, i: u16) -> Vec<String> {
        self.sorted6(ni, i).iter().map(|a| format!("{}/{}", a.addr, a.len)).collect()
    }

    /// "ip -6 addr ..."
    pub(crate) fn sh_addr6(&mut self, ni: usize, args: &[&str], brief: bool) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "delete") {
            let add = is(cmd, "add");
            let Some(p) = args.get(1) else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            let Some(pfx) = Prefix6::parse(p) else {
                return self.sh_err(ni, &format!("Error: inet6 prefix is expected rather than \"{}\".\n", p));
            };
            let Some(name) = args.iter().position(|a| *a == "dev").and_then(|i| args.get(i + 1)) else {
                return self.sh_err(ni, "Not enough information: \"dev\" argument is required.\n");
            };
            let Some(i) = self.sh_iface(ni, name) else {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            };
            if !self.v6(ni).is_some_and(|v| v.on(i)) {
                return self.sh_err(ni, "RTNETLINK answers: Permission denied\n");
            }
            let have = self.v6(ni).and_then(|v| v.iface(i)).is_some_and(|f| f.has(pfx.addr));
            if add && have {
                return self.sh_err(ni, "RTNETLINK answers: File exists\n");
            }
            let cfg = AddrCfg { prefix: pfx, eui64: false, anycast: false };
            return match self.apply(Action::Ip6 { dev, set: Ip6Set::Addr { iface: String::from(*name), cfg, add } }) {
                Ok(()) => Ok(()),
                Err(_) if !add => self.sh_err(ni, "RTNETLINK answers: Cannot assign requested address\n"),
                Err(_) => self.sh_err(ni, "RTNETLINK answers: Invalid argument\n"),
            };
        }
        if is(cmd, "flush") {
            let Some(name) = args.get(1..).and_then(|r| r.iter().position(|a| *a == "dev").and_then(|k| r.get(k + 1)).or(r.iter().find(|a| !a.starts_with('-')))).copied() else {
                return self.sh_err(ni, "Flush requires arguments.\n");
            };
            if self.sh_iface(ni, name).is_none() {
                return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", name));
            }
            let _ = self.apply(Action::Ip6 { dev, set: Ip6Set::ClearAddrs { iface: String::from(name) } });
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip address help\".\n", cmd));
        }
        let only = if args.is_empty() { None } else { args[1..].iter().position(|a| *a == "dev").and_then(|k| args.get(k + 2)).or(args[1..].iter().find(|a| !a.starts_with('-'))).copied() };
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
            let lines = self.inet6_lines(ni, i);
            if lines.is_empty() {
                continue;
            }
            let (head, _) = self.link_line(ni, i);
            if brief {
                let state = head.rsplit_once("state ").map(|x| x.1).unwrap_or("UNKNOWN");
                let shown = head.split(": ").nth(1).unwrap_or("");
                s.push_str(format!("{:<16} {:<14} {}", shown, state, self.brief6(ni, i).join(" ")).trim_end());
                s.push('\n');
                continue;
            }
            // the link line as the inet6 family reports it: no qdisc, no group
            let before = head.split(" qdisc ").next().unwrap_or(&head);
            let state = head.rsplit_once(" state ").map(|x| x.1).unwrap_or("UNKNOWN");
            s.push_str(&format!("{} state {} qlen 1000\n{}", before, state, lines));
        }
        self.out(ni, &s);
        Ok(())
    }

    /// "ip -6 route ..."
    pub(crate) fn sh_route6(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "delete") || is(cmd, "replace") {
            let add = !is(cmd, "delete");
            let Some(target) = args.get(1) else {
                return self.sh_err(ni, "Error: inet6 prefix is expected rather than \"\".\n");
            };
            let prefix = if *target == "default" {
                Prefix6 { addr: Ipv6::UNSPECIFIED, len: 0 }
            } else {
                match Prefix6::parse(target) {
                    Some(p) => p.trunc(),
                    None => return self.sh_err(ni, &format!("Error: inet6 prefix is expected rather than \"{}\".\n", target)),
                }
            };
            let mut via: Option<Ipv6> = None;
            let mut ifn: Option<u16> = None;
            let mut metric: u32 = 1024;
            let mut k = 2;
            while k < args.len() {
                match args[k] {
                    "via" => {
                        let a = args.get(k + 1).copied().unwrap_or("");
                        match Ipv6::parse(a) {
                            Some(x) => via = Some(x),
                            None => return self.sh_err(ni, &format!("Error: inet6 address is expected rather than \"{}\".\n", a)),
                        }
                        k += 1;
                    }
                    "dev" => {
                        let n = args.get(k + 1).copied().unwrap_or("");
                        let Some(i) = self.sh_iface(ni, n) else {
                            return self.sh_err(ni, &format!("Cannot find device \"{}\"\n", n));
                        };
                        ifn = Some(i);
                        k += 1;
                    }
                    "metric" => {
                        metric = args.get(k + 1).and_then(|m| m.parse().ok()).unwrap_or(1024);
                        k += 1;
                    }
                    "proto" | "src" | "table" | "pref" => k += 1,
                    "onlink" => {}
                    o => return self.sh_err(ni, &format!("Error: either \"to\" is duplicate, or \"{}\" is a garbage.\n", o)),
                }
                k += 1;
            }
            let route = Route6 { prefix, via, iface: ifn, src: Src6::Static, ad: 1, metric, expires: None };
            if !add {
                return match self.apply(Action::Ip6 { dev, set: Ip6Set::Route { route, add: false } }) {
                    Ok(()) => Ok(()),
                    Err(_) => self.sh_err(ni, "RTNETLINK answers: No such process\n"),
                };
            }
            if via.is_none() && ifn.is_none() {
                return self.sh_err(ni, "Error: Device for nexthop is not up.\n");
            }
            if let Some(gw) = via {
                // a link-local next hop needs its device; another one an interface it is on
                let ok = match ifn {
                    Some(i) => gw.is_link_local() || self.v6(ni).and_then(|v| v.iface(i)).is_some_and(|f| f.on_link(gw)),
                    None => !gw.is_link_local() && self.v6(ni).is_some_and(|v| v.ifs.iter().any(|f| f.enabled && f.on_link(gw))),
                };
                if !ok {
                    return self.sh_err(ni, "Error: Nexthop has invalid gateway.\n");
                }
            }
            let exists = self.v6(ni).is_some_and(|v| v.statics.iter().any(|r| r.prefix == prefix && r.via == via));
            if exists && !is(cmd, "replace") {
                return self.sh_err(ni, "RTNETLINK answers: File exists\n");
            }
            return match self.apply(Action::Ip6 { dev, set: Ip6Set::Route { route, add: true } }) {
                Ok(()) => Ok(()),
                Err(_) => self.sh_err(ni, "Error: Nexthop has invalid gateway.\n"),
            };
        }
        let st = self.stack(ni);
        let name = |i: Option<u16>| i.and_then(|i| st.and_then(|s| s.ifaces.get(i as usize))).map(|f| f.name.clone()).unwrap_or_default();
        if is(cmd, "get") {
            let a = args.get(1).copied().unwrap_or("");
            let Some(dst) = Ipv6::parse(zone(a).0) else {
                return self.sh_err(ni, &format!("Error: inet6 prefix is expected rather than \"{}\".\n", a));
            };
            let Some(nh) = self.ip6_route(ni, dst, None) else {
                return self.sh_err(ni, "RTNETLINK answers: Network is unreachable\n");
            };
            let src = self.ip6_source(ni, dst, Some(nh.iface)).unwrap_or(Ipv6::UNSPECIFIED);
            let via = if nh.gw == dst { String::new() } else { format!(" via {}", nh.gw) };
            let proto = if nh.src == Src6::RaDefault { " proto ra" } else { "" };
            let text = format!("{} from ::{} dev {}{} src {} metric 1024 pref medium\n", dst, via, name(Some(nh.iface)), proto, src);
            self.out(ni, &text);
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip route help\".\n", cmd));
        }
        let now = self.now;
        let mut lines: Vec<(bool, u128, u8, String)> = Vec::new();
        // the loopback address (the kernel lists it in the main table)
        if let Some(i) = st.and_then(|s| s.ifaces.iter().position(|f| f.name == "lo")).map(|i| i as u16)
            && self.v6(ni).is_some_and(|v| v.running(i))
        {
            lines.push((false, 1, 128, String::from("::1 dev lo proto kernel metric 256 pref medium")));
        }
        for r in self.ip6_routes(dev) {
            if r.src == Src6::Local {
                continue;
            }
            let net = if r.prefix.len == 0 { String::from("default") } else if r.prefix.len == 128 { format!("{}", r.prefix.addr) } else { format!("{}", r.prefix) };
            let exp = r.expires.map(|t| format!(" expires {}sec", t.saturating_sub(now) / SEC)).unwrap_or_default();
            let l = match (r.src, r.via) {
                (Src6::Connected | Src6::RaPrefix, _) => format!("{} dev {} proto kernel metric {}{} pref medium", net, name(r.iface), r.metric, exp),
                (Src6::RaDefault, Some(g)) => {
                    let hops = r.iface.and_then(|i| self.v6(ni).and_then(|v| v.iface(i)).and_then(|f| f.hop_limit)).map(|h| format!(" hoplimit {}", h)).unwrap_or_default();
                    format!("{} via {} dev {} proto ra metric {}{}{} pref medium", net, g, name(r.iface), r.metric, exp, hops)
                }
                (_, Some(g)) => format!("{} via {} dev {} metric {} pref medium", net, g, name(r.iface), r.metric),
                (_, None) => format!("{} dev {} metric {} pref medium", net, name(r.iface), r.metric),
            };
            lines.push((r.prefix.len == 0, r.prefix.addr.to_u128(), r.prefix.len, l));
        }
        lines.sort_by_key(|l| (l.0, l.1, l.2));
        let mut s = String::new();
        for (_, _, _, l) in lines {
            s.push_str(&l);
            s.push('\n');
        }
        self.out(ni, &s);
        Ok(())
    }

    /// the IPv6 lines of "ip neigh"
    pub(crate) fn neigh6_lines(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let st = self.stack(ni);
        let mut s = String::new();
        for r in self.ip6_neighbors(dev) {
            let name = st.and_then(|s| s.ifaces.get(r.iface as usize)).map(|f| f.name.clone()).unwrap_or_default();
            match r.mac {
                Some(m) => {
                    let state = if r.fixed { "PERMANENT" } else { r.state.linux() };
                    s.push_str(&format!("{} dev {} lladdr {} {}{}\n", r.ip, name, m, if r.router { "router " } else { "" }, state));
                }
                None => s.push_str(&format!("{} dev {} INCOMPLETE\n", r.ip, name)),
            }
        }
        s
    }

    /// "ip -6 neigh ..."
    pub(crate) fn sh_neigh6(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let cmd = args.first().copied().unwrap_or("show");
        if is(cmd, "add") || is(cmd, "replace") || is(cmd, "delete") {
            let add = !is(cmd, "delete");
            let a = args.get(1).copied().unwrap_or("");
            let Some(ip) = Ipv6::parse(a) else {
                return self.sh_err(ni, &format!("Error: inet6 address is expected rather than \"{}\".\n", a));
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
            return match self.apply(Action::Ip6 { dev, set: Ip6Set::Neighbor { iface: String::from(name), ip, mac } }) {
                Ok(()) => Ok(()),
                Err(_) if add => self.sh_err(ni, "RTNETLINK answers: Invalid argument\n"),
                Err(_) => self.sh_err(ni, "RTNETLINK answers: No such file or directory\n"),
            };
        }
        if is(cmd, "flush") {
            let only = args.iter().position(|x| *x == "dev").and_then(|i| args.get(i + 1)).map(|n| String::from(*n));
            let _ = self.apply(Action::Ip6 { dev, set: Ip6Set::ClearNeighbors { iface: only } });
            return Ok(());
        }
        if !(is(cmd, "show") || is(cmd, "list")) && !args.is_empty() {
            return self.sh_err(ni, &format!("Command \"{}\" is unknown, try \"ip neigh help\".\n", cmd));
        }
        let s = self.neigh6_lines(ni);
        self.out(ni, &s);
        Ok(())
    }

    // ---------------------------------------------------------------- ping, traceroute

    /// an interface for "-I" or "%IF"; the console's complaint when there is none
    fn sh_oif(&mut self, ni: usize, name: &str, prog: &str) -> Result<u16, String> {
        match self.sh_iface(ni, name) {
            Some(i) => Ok(i),
            None => {
                let text = if prog == "ping" { format!("ping: SO_BINDTODEVICE {}: No such device\n", name) } else { format!("Cannot find device \"{}\"\n", name) };
                self.sh_err(ni, &text).map(|_| 0)
            }
        }
    }

    /// "ping -6", "ping6", "ping X:X::X"
    pub(crate) fn sh_ping6(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let mut o = Ping6Opts { count: 0, size: 56, hops: None, no_frag: false, interval: SEC, timeout: 10 * SEC, src: None, oif: None };
        let mut dst: Option<&str> = None;
        let mut via: Option<String> = None;
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
                    "s" => match val.and_then(|v| v.parse::<u16>().ok()).filter(|&s| s <= 65487) {
                        Some(s) => o.size = s,
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "t" => match val.and_then(|v| v.parse::<u8>().ok()).filter(|&t| t > 0) {
                        Some(t) => o.hops = Some(t),
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "W" => match val.and_then(secs) {
                        Some(t) => o.timeout = t.max(MS),
                        None => return self.sh_err(ni, &bad(val)),
                    },
                    "M" => match val {
                        Some("do") => o.no_frag = true,
                        Some("dont" | "want") => o.no_frag = false,
                        _ => return self.sh_err(ni, &format!("ping: invalid -M argument: {}\n", val.unwrap_or(""))),
                    },
                    "I" => {
                        let n = val.unwrap_or("");
                        match Ipv6::parse(n) {
                            Some(a) => o.src = Some(a),
                            None => {
                                o.oif = Some(self.sh_oif(ni, n, "ping")?);
                                via = Some(String::from(n));
                            }
                        }
                    }
                    "n" | "q" | "6" | "w" => {}
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
        let (addr, z) = zone(d);
        if let Some(z) = z {
            o.oif = Some(self.sh_oif(ni, z, "ping")?);
        }
        let Some(a) = Ipv6::parse(addr) else {
            return self.sh_err(ni, &format!("ping: {}: Address family for hostname not supported\n", d));
        };
        if (a.is_link_local() || (a.is_multicast() && a.mc_scope() <= 2)) && o.oif.is_none() {
            return self.sh_err(ni, "ping: connect: Invalid argument\n");
        }
        if via.is_some() {
            o.src = self.ip6_source(ni, a, o.oif);
        }
        let Ok(id) = self.ping6(dev, a, o) else {
            return self.sh_err(ni, "ping: connect: Network is unreachable\n");
        };
        let now = self.now;
        self.cli_start_job(ni, id, JobOut::Ping6 { ios: false, col: 0, seen: 0, sent: 0, last_send: now, head: false, t0: now, shown: String::from(d), from: via });
        Ok(())
    }

    /// "traceroute -6", "traceroute6", "traceroute X:X::X"
    pub(crate) fn sh_traceroute6(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let mut o = TraceOpts { max_hops: 30, probes: 3, timeout: 5 * SEC, port: 33434, size: 32 };
        let mut numeric = false;
        let mut dst: Option<&str> = None;
        let mut oif: Option<u16> = None;
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            match a {
                "-n" => numeric = true,
                "-6" => {}
                "-m" | "-q" | "-w" | "-i" => {
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
                        "-i" => oif = Some(self.sh_oif(ni, v, "traceroute")?),
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
        let (addr, z) = zone(d);
        if let Some(z) = z {
            oif = Some(self.sh_oif(ni, z, "traceroute")?);
        }
        let Some(a) = Ipv6::parse(addr) else {
            return self.sh_err(ni, &format!("{}: Address family for hostname not supported\nCannot handle \"host\" cmdline arg `{}' on position 1 (argc {})\n", d, d, args.len()));
        };
        let Ok(id) = self.traceroute6(dev, a, o, oif) else {
            return Ok(());
        };
        self.cli_start_job(ni, id, JobOut::Trace6 { ios: false, hop: 0, probe: 0, last: None, wrote: false, numeric, head: false, shown: String::from(d) });
        Ok(())
    }

    // ---------------------------------------------------------------- sysctl

    /// a key of net.ipv6 (read, or set with `val`): the line sysctl prints, or its complaint
    pub(crate) fn sysctl6(&mut self, ni: usize, key: &str, val: Option<&str>) -> Result<String, String> {
        let dev = self.dev_of(ni);
        let k = key.replace('/', ".");
        let missing = || format!("sysctl: cannot stat /proc/sys/{}: No such file or directory\n", k.replace('.', "/"));
        let Some(rest) = k.strip_prefix("net.ipv6.conf.") else {
            return Err(missing());
        };
        let Some((who, what)) = rest.rsplit_once('.') else {
            return Err(missing());
        };
        let iface = if who == "all" || who == "default" { None } else { Some(self.sh_iface(ni, who).ok_or_else(missing)?) };
        if !matches!(what, "disable_ipv6" | "forwarding" | "accept_ra" | "autoconf" | "hop_limit") {
            return Err(missing());
        }
        let num = |v: &str| v.parse::<u32>().ok();
        if let Some(v) = val {
            let bad = || format!("sysctl: setting key \"{}\": Invalid argument\n", k);
            let n = num(v).ok_or_else(bad)?;
            let set = match (what, iface) {
                ("disable_ipv6", None) if n <= 1 => Ip6Set::All(n == 0),
                ("disable_ipv6", Some(_)) if n <= 1 => Ip6Set::Enable { iface: String::from(who), on: n == 0 },
                ("forwarding", _) if n <= 1 => Ip6Set::Routing(n == 1),
                ("hop_limit", _) if (1..=255).contains(&n) => Ip6Set::HopLimit(n as u8),
                ("accept_ra" | "autoconf", _) if n <= 2 => {
                    let names: Vec<String> = match iface {
                        Some(_) => alloc::vec![String::from(who)],
                        None => self.stack(ni).map(|s| s.ifaces.iter().filter(|f| !f.hidden).map(|f| f.name.clone()).collect()).unwrap_or_default(),
                    };
                    for nm in names {
                        let i = self.stack(ni).and_then(|s| s.iface_by_name(&nm)).unwrap_or(0);
                        let (mut on, mut dr) = self.v6(ni).and_then(|x| x.iface(i)).map(|f| (f.autoconf, f.accept_ra)).unwrap_or((true, true));
                        if what == "autoconf" {
                            on = n > 0;
                        } else {
                            dr = n > 0;
                        }
                        let _ = self.apply(Action::Ip6 { dev, set: Ip6Set::Autoconf { iface: nm, on, default_route: dr } });
                    }
                    return Ok(format!("{} = {}\n", k, v));
                }
                _ => return Err(bad()),
            };
            self.apply(Action::Ip6 { dev, set }).map_err(|_| bad())?;
            return Ok(format!("{} = {}\n", k, v));
        }
        let Some(v6) = self.v6(ni) else {
            return Err(missing());
        };
        let f = iface.and_then(|i| v6.iface(i));
        let n = match what {
            "disable_ipv6" => match iface {
                Some(i) => !v6.on(i) as u32,
                None => !v6.all_on as u32,
            },
            "forwarding" => v6.routing as u32,
            "hop_limit" => v6.hop_limit as u32,
            "accept_ra" => f.map_or(1, |f| f.accept_ra as u32),
            _ => f.map_or(1, |f| f.autoconf as u32),
        };
        Ok(format!("{} = {}\n", k, n))
    }

    // ---------------------------------------------------------------- configuration

    /// the IPv6 part of a Linux device's configuration as shell commands
    pub(crate) fn linux_config6(&self, ni: usize) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let v = &st.v6;
        if !v.used() {
            return String::new();
        }
        let mut s = String::new();
        if v.all_on {
            s.push_str("sysctl -w net.ipv6.conf.all.disable_ipv6=0\n");
        }
        for (i, f) in st.ifaces.iter().enumerate() {
            if f.hidden {
                continue;
            }
            let on = v.on(i as u16);
            if on != v.all_on {
                s.push_str(&format!("sysctl -w net.ipv6.conf.{}.disable_ipv6={}\n", f.name, if on { 0 } else { 1 }));
            }
        }
        if v.routing {
            s.push_str("sysctl -w net.ipv6.conf.all.forwarding=1\n");
        }
        if v.hop_limit != 64 {
            s.push_str(&format!("sysctl -w net.ipv6.conf.all.hop_limit={}\n", v.hop_limit));
        }
        for (i, f) in st.ifaces.iter().enumerate() {
            if f.hidden {
                continue;
            }
            let Some(x) = v.iface(i as u16) else {
                continue;
            };
            if !x.accept_ra {
                s.push_str(&format!("sysctl -w net.ipv6.conf.{}.accept_ra=0\n", f.name));
            }
            if !x.autoconf {
                s.push_str(&format!("sysctl -w net.ipv6.conf.{}.autoconf=0\n", f.name));
            }
            for c in &x.cfg {
                s.push_str(&format!("ip -6 addr add {} dev {}\n", c.prefix, f.name));
            }
        }
        for r in &v.statics {
            let net = if r.prefix.len == 0 { String::from("default") } else { format!("{}", r.prefix) };
            let mut l = format!("ip -6 route add {}", net);
            if let Some(g) = r.via {
                l.push_str(&format!(" via {}", g));
            }
            if let Some(i) = r.iface {
                l.push_str(&format!(" dev {}", st.ifaces[i as usize].name));
            }
            if r.metric != 1024 {
                l.push_str(&format!(" metric {}", r.metric));
            }
            s.push_str(&l);
            s.push('\n');
        }
        for ((i, ip), e) in &v.neigh {
            if e.fixed
                && let Some(m) = e.mac
            {
                s.push_str(&format!("ip -6 neigh add {} lladdr {} dev {}\n", ip, m, st.ifaces[*i as usize].name));
            }
        }
        s
    }
}
