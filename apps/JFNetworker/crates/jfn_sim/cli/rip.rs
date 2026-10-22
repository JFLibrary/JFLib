// rip -- RIP at the IOS-like command line: "router rip" with its mode "(config-router)#",
// "ip rip send|receive version" on interfaces, show ip rip database, show ip protocols, debug ip
// rip, clear ip route *, and the lines of the running configuration; help texts, messages and
// debug lines are our own words, the command words, configuration lines and table headers
// follow the originals
//
//   build_rip(tree, modes, router)   the commands in an IOS command tree (ios.rs); only where
//                                    the device routes
//   Engine::ios_rip(ni, parsed)      router rip and its commands, ip rip ... version, show ip rip
//                                    database, show ip protocols, clear ip route *
//   Engine::ios_rip_config(ni)       the "router rip" block of the running configuration (after
//                                    the interfaces); ios_rip_if_config(ni, iface) the "ip rip"
//                                    lines of an interface
//   Engine::rip_debug(ni, note)      "debug ip rip": a decision of RIP as console lines
//   hms(ns)                          "00:00:12" (ages of routes and updates)
//
// The block lists only what differs from the defaults, in this order: version, timers basic,
// redistribute, passive-interface, network, default-information originate, distance,
// maximum-paths, and always auto-summary or no auto-summary (off is the default here, as on
// IOS 15).  "router rip" needs routing ("% IP routing not enabled" on an L3 switch without "ip
// routing"); "no ip routing" removes the process.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};
use netpkt::Ipv4;

use super::ios::K;
use super::Mode;
use crate::action::Action;
use crate::kernel::{Engine, Ns, SEC};
use crate::rip::{classful, classful_len, Origin, Redist, RipOut, RipSet, RipTimers, DISTANCE, INFINITY, MAX_PATHS, V1, V2};
use crate::trace::Note;

/// "00:00:12": hours, minutes, seconds
pub(crate) fn hms(ns: Ns) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// whole seconds, rounded up
fn secs_up(ns: Ns) -> u64 {
    ns.div_ceil(SEC)
}

/// "1", "2", "1 2"
fn versions(v: u8) -> String {
    let mut p: Vec<&str> = Vec::new();
    if v & V1 != 0 {
        p.push("1");
    }
    if v & V2 != 0 {
        p.push("2");
    }
    if p.is_empty() { String::from("-") } else { p.join(" ") }
}

/// the RIP commands in an IOS tree; `m`: user, privileged, global configuration, interface,
/// subinterface and router mode
pub(crate) fn build_rip(t: &mut Tree<K>, m: [ModeId; 6], router: bool) {
    let [user, prv, conf, ifm, sub, rtr] = m;
    if !router {
        return;
    }
    for x in [user, prv] {
        t.add(x, "show ip rip", "State of RIP", None, 0);
        t.add(x, "show ip rip database", "The destinations RIP knows, with their paths and timers", Some(K::ShowRipDb), 0);
        t.add(x, "show ip protocols", "The routing processes and their settings", Some(K::ShowProtocols), 0);
        for (w, h) in [("connected", "Only the networks of the interfaces"), ("rip", "Only the routes RIP learned"), ("static", "Only the static routes")] {
            t.add(x, &format!("show ip route {}=only", w), h, Some(K::ShowIpRoute), 0);
        }
    }
    t.add(prv, "debug ip rip", "RIP messages and what they change", Some(K::Debug), 0);
    t.add(prv, "undebug ip rip", "RIP messages and what they change", Some(K::Undebug), 0);
    t.add(prv, "clear ip route", "Routes of the routing processes", None, 0);
    t.add(prv, "clear ip route *=all", "Forget the learned routes and ask the neighbours again", Some(K::ClearRoute), 0);

    // global configuration
    t.add(conf, "router", "Start a routing process", None, F_NO);
    t.add(conf, "router rip", "RIP: routes by hop count from the neighbours (RFC 2453)", Some(K::RouterRip), 0);

    // the router mode: like every configuration mode, then its own commands
    t.add(rtr, "do", "Run an EXEC command from here", None, 0);
    t.add(rtr, "do LINE=cmd", "The command", Some(K::Do), 0);
    t.add(rtr, "exit", "Back to the device configuration", Some(K::Exit), 0);
    t.add(rtr, "end", "Finish configuring", Some(K::End), F_HIDDEN);
    t.add(rtr, "version", "RIP version to send and receive", None, F_NO);
    t.add(rtr, "version <1-2>=ver", "1: broadcasts without masks, 2: multicasts with masks", Some(K::RipVersion), 0);
    t.add_no(rtr, "version", K::RipVersion);
    t.add(rtr, "network", "Run RIP on the interfaces of a network", None, F_NO);
    t.add(rtr, "network A.B.C.D=net", "An address of the network (its class A, B or C network counts)", Some(K::RipNetwork), 0);
    t.add(rtr, "passive-interface", "Send no updates on an interface; it still listens", None, F_NO);
    t.add(rtr, "passive-interface default", "Every interface not named with no passive-interface", Some(K::RipPassive), 0);
    t.add(rtr, "passive-interface NAME1=if", "", Some(K::RipPassive), 0);
    t.add(rtr, "auto-summary", "Summarise subnets where class A, B and C networks meet", Some(K::RipAutoSummary), F_NO);
    t.add(rtr, "default-information", "The default route", None, F_NO);
    t.add(rtr, "default-information originate", "Advertise a default route to the neighbours", Some(K::RipDefault), 0);
    t.add(rtr, "redistribute", "Advertise routes of another source", None, F_NO);
    for (w, h) in [("connected", "Networks of interfaces RIP does not run on"), ("static", "Static routes")] {
        t.add(rtr, &format!("redistribute {}=what", w), h, Some(K::RipRedist), 0);
        t.add(rtr, &format!("redistribute {}=what metric", w), "The metric they get", None, 0);
        t.add(rtr, &format!("redistribute {}=what metric <1-16>=metric", w), "1 to 15; 16 is unreachable", Some(K::RipRedist), 0);
    }
    t.add(rtr, "timers", "Timers of the protocol", None, F_NO);
    t.add(rtr, "timers basic", "Update, invalid, holddown and flush times", None, 0);
    t.add(rtr, "timers basic <1-3600>=upd", "Seconds between updates", None, 0);
    t.add(rtr, "timers basic <1-3600>=upd <1-86400>=inv", "Seconds until a route nobody confirms is invalid", None, 0);
    t.add(rtr, "timers basic <1-3600>=upd <1-86400>=inv <0-86400>=hold", "Seconds a lost route ignores worse offers", None, 0);
    t.add(rtr, "timers basic <1-3600>=upd <1-86400>=inv <0-86400>=hold <1-86400>=flush", "Seconds after its last update until a route is removed", Some(K::RipTimers), 0);
    t.add_no(rtr, "timers basic", K::RipTimers);
    t.add(rtr, "distance", "Administrative distance of the routes RIP learns", None, F_NO);
    t.add(rtr, "distance <1-255>=ad", "Lower wins against other sources (default 120)", Some(K::RipDistance), 0);
    t.add_no(rtr, "distance", K::RipDistance);
    t.add(rtr, "maximum-paths", "Equal-cost paths to use", None, F_NO);
    t.add(rtr, "maximum-paths <1-32>=paths", "How many (default 4)", Some(K::RipMaxPaths), 0);
    t.add_no(rtr, "maximum-paths", K::RipMaxPaths);

    // interfaces ("ip" itself is there, with its "no")
    for x in [ifm, sub] {
        t.add(x, "ip rip", "RIP on this interface", None, 0);
        for (w, h) in [("send", "Versions sent here"), ("receive", "Versions taken here")] {
            t.add(x, &format!("ip rip {}=dir", w), h, None, 0);
            t.add(x, &format!("ip rip {}=dir version", w), "Instead of the version of router rip", None, 0);
            t.add(x, &format!("ip rip {}=dir version 1=v1", w), "Version 1", Some(K::RipIfVersion), 0);
            t.add(x, &format!("ip rip {}=dir version 2=v2", w), "Version 2", Some(K::RipIfVersion), 0);
            t.add(x, &format!("ip rip {}=dir version 1=v1 2=v2", w), "Version 2 as well", Some(K::RipIfVersion), 0);
            t.add_no(x, &format!("ip rip {}=dir version", w), K::RipIfVersion);
        }
    }
}

impl Engine {
    /// the RIP commands of every mode
    pub(crate) fn ios_rip(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let set = match p.cmd {
            K::RouterRip => {
                if p.no {
                    let _ = self.apply(Action::Rip { dev, set: RipSet::Process(false) });
                    return Ok(());
                }
                if !self.stack(ni).is_some_and(|s| s.forwarding) {
                    return self.ios_err(ni, "% IP routing not enabled\n");
                }
                if let Err(e) = self.apply(Action::Rip { dev, set: RipSet::Process(true) }) {
                    return self.ios_err(ni, &format!("% {}\n", e));
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.mode = Mode::Router(super::Proc::Rip);
                    c.ifs.clear();
                    c.vlans.clear();
                    c.pool.clear();
                }
                return Ok(());
            }
            K::ShowRipDb => {
                let text = self.show_rip_db(ni);
                self.out(ni, &text);
                return Ok(());
            }
            K::ShowProtocols => {
                // RIP's process, then OSPF's (cli/ospf.rs)
                let text = format!("{}{}", self.show_protocols(ni), self.ospf_protocols(ni));
                self.out(ni, &text);
                return Ok(());
            }
            K::ClearRoute => {
                let _ = self.apply(Action::Rip { dev, set: RipSet::Clear });
                return Ok(());
            }
            K::RipIfVersion => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let receive = p.text("dir") == Some("receive");
                let v = (if p.has("v1") { V1 } else { 0 }) | (if p.has("v2") { V2 } else { 0 });
                let mut r = Ok(());
                for f in ifs {
                    if self.stack(ni).and_then(|s| s.iface_by_name(&f)).is_none() {
                        r = self.ios_err(ni, "% IP addresses may not be configured on L2 links.\n");
                        continue;
                    }
                    let versions = if p.no { None } else { Some(v) };
                    if let Err(e) = self.apply(Action::Rip { dev, set: RipSet::IfVersions { iface: f, receive, versions } }) {
                        r = self.ios_err(ni, &format!("% {}\n", e));
                    }
                }
                return r;
            }
            K::RipVersion => RipSet::Version(if p.no { None } else { p.num("ver").map(|v| v as u8) }),
            K::RipNetwork => {
                let net = Ipv4(p.ip("net").unwrap_or([0; 4]));
                if classful_len(net) == 32 || net.0[0] == 0 || net.is_loopback() {
                    return self.ios_err(ni, &format!("% {} is no network RIP can run on\n", net));
                }
                RipSet::Network { net, add: !p.no }
            }
            K::RipPassive => RipSet::Passive { iface: p.text("if").map(String::from), on: !p.no },
            K::RipAutoSummary => RipSet::AutoSummary(!p.no),
            K::RipDefault => RipSet::DefaultOriginate(!p.no),
            K::RipRedist => {
                let what = if p.text("what") == Some("static") { Redist::Static } else { Redist::Connected };
                RipSet::Redistribute { what, metric: p.num("metric").map(|m| m as u8), add: !p.no }
            }
            K::RipTimers => {
                if p.no {
                    RipSet::Timers(None)
                } else {
                    let n = |k: &str| p.num(k).unwrap_or(0) as u32;
                    let t = RipTimers { update: n("upd"), invalid: n("inv"), holddown: n("hold"), flush: n("flush") };
                    if t.flush < t.invalid {
                        return self.ios_err(ni, "% The flush time must not be shorter than the invalid time\n");
                    }
                    RipSet::Timers(Some(t))
                }
            }
            K::RipDistance => RipSet::Distance(if p.no { None } else { p.num("ad").map(|d| d as u8) }),
            K::RipMaxPaths => RipSet::MaxPaths(if p.no { None } else { p.num("paths").map(|d| d as u8) }),
            _ => return Ok(()),
        };
        match self.apply(Action::Rip { dev, set }) {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    /// the "router rip" block of the running configuration ("" without one)
    pub(crate) fn ios_rip_config(&self, ni: usize) -> String {
        let Some(c) = self.rip_of(ni).map(|r| &r.cfg).filter(|c| c.on) else {
            return String::new();
        };
        let mut s = String::from("router rip\n");
        if let Some(v) = c.version {
            s.push_str(&format!(" version {}\n", v));
        }
        let t = c.timers;
        if t != RipTimers::default() {
            s.push_str(&format!(" timers basic {} {} {} {}\n", t.update, t.invalid, t.holddown, t.flush));
        }
        for (what, metric) in &c.redistribute {
            match metric {
                Some(m) => s.push_str(&format!(" redistribute {} metric {}\n", what.word(), m)),
                None => s.push_str(&format!(" redistribute {}\n", what.word())),
            }
        }
        if c.passive_default {
            s.push_str(" passive-interface default\n");
        }
        for p in &c.passive {
            s.push_str(&format!(" {}passive-interface {}\n", if c.passive_default { "no " } else { "" }, p));
        }
        for n in &c.networks {
            s.push_str(&format!(" network {}\n", n));
        }
        if c.default_originate {
            s.push_str(" default-information originate\n");
        }
        if c.distance != DISTANCE {
            s.push_str(&format!(" distance {}\n", c.distance));
        }
        if c.max_paths != MAX_PATHS {
            s.push_str(&format!(" maximum-paths {}\n", c.max_paths));
        }
        s.push_str(if c.auto_summary { " auto-summary\n" } else { " no auto-summary\n" });
        s.push_str("!\n");
        s
    }

    /// the "ip rip ... version" lines of an interface
    pub(crate) fn ios_rip_if_config(&self, ni: usize, iface: &str) -> String {
        let Some(r) = self.rip_of(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for (word, map) in [("send", &r.cfg.send), ("receive", &r.cfg.recv)] {
            if let Some(v) = map.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
                s.push_str(&format!(" ip rip {} version {}\n", word, versions(v)));
            }
        }
        s
    }

    fn rip_ifname(&self, ni: usize, i: u16) -> String {
        self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default()
    }

    /// show ip rip database: every destination with its paths, own networks, unreachable ones
    fn show_rip_db(&self, ni: usize) -> String {
        let Some(r) = self.rip_of(ni).filter(|r| r.running()) else {
            return String::new();
        };
        let now = self.now;
        let mut lines: Vec<(netpkt::Prefix4, String)> = Vec::new();
        if r.cfg.auto_summary {
            // the classful networks whose subnets go out as one entry
            let mut majors: Vec<netpkt::Prefix4> = r.table().keys().filter(|k| k.len > classful_len(k.addr) && classful_len(k.addr) < 32).map(|k| classful(k.addr)).collect();
            majors.dedup();
            for m in majors {
                if !r.table().contains_key(&m) {
                    lines.push((m, format!("{}    auto-summary\n", m)));
                }
            }
        }
        for (k, e) in r.table() {
            let text = if !e.reachable() {
                let gone = e.gc_at.map(|t| secs_up(t.saturating_sub(now))).unwrap_or(0);
                match e.hold_until.filter(|&t| t > now) {
                    Some(h) => format!("{}    unreachable (metric {}): held down {} s more, removed in {} s\n", k, INFINITY, secs_up(h - now), gone),
                    None => format!("{}    unreachable (metric {}): removed in {} s\n", k, INFINITY, gone),
                }
            } else {
                match e.origin {
                    Origin::Connected(i) => format!("{}    directly connected, {}\n", k, self.rip_ifname(ni, i)),
                    Origin::Redist(what) => format!("{}    redistributed {}, metric {}\n", k, what.word(), e.metric),
                    Origin::Default => format!("{}    default route originated here, metric {}\n", k, e.metric),
                    Origin::Learned => {
                        let mut t = format!("{}\n", k);
                        for p in &e.paths {
                            t.push_str(&format!("    [{}] via {}, {}, {}\n", e.metric, p.via, hms(now.saturating_sub(p.heard)), self.rip_ifname(ni, p.iface)));
                        }
                        t
                    }
                }
            };
            lines.push((*k, text));
        }
        lines.sort_by_key(|l| (l.0.addr.to_u32(), l.0.len));
        lines.into_iter().map(|l| l.1).collect()
    }

    /// show ip protocols: the RIP process in a few lines ("" when there is none)
    fn show_protocols(&self, ni: usize) -> String {
        let Some(r) = self.rip_of(ni).filter(|r| r.cfg.on) else {
            return String::new();
        };
        let (c, now, t) = (&r.cfg, self.now, r.cfg.timers);
        let mut s = String::from("Routing process \"rip\" (RFC 2453)\n");
        match r.next_update().filter(|&n| n > now) {
            Some(n) => s.push_str(&format!("  Updates every {} seconds, the next one in {} seconds\n", t.update, secs_up(n - now))),
            None => s.push_str(&format!("  Updates every {} seconds\n", t.update)),
        }
        s.push_str(&format!("  Routes are invalid after {} seconds without an update, held down for {}, flushed after {}\n", t.invalid, t.holddown, t.flush));
        let mut from = String::from("rip");
        for (what, _) in &c.redistribute {
            from.push_str(", ");
            from.push_str(what.word());
        }
        s.push_str(&format!("  Redistributing: {}\n", from));
        let (send, recv) = match c.version {
            Some(1) => ("1", "1"),
            Some(_) => ("2", "2"),
            None => ("2", "1 and 2"),
        };
        s.push_str(&format!("  Versions: send {}, receive {}\n", send, recv));
        s.push_str("    Interface                  Send  Recv  Passive\n");
        for x in r.ifaces().values() {
            s.push_str(&format!("    {:<26} {:<5} {:<5} {}\n", x.name, versions(x.send), versions(x.recv), if x.passive { "yes" } else { "no" }));
        }
        s.push_str(&format!("  Summarising at classful borders: {}\n", if c.auto_summary { "on" } else { "off" }));
        s.push_str(&format!("  Equal-cost paths: up to {}\n", c.max_paths));
        s.push_str("  Routing for networks:\n");
        for n in &c.networks {
            s.push_str(&format!("    {}\n", n));
        }
        let passive: Vec<&str> = r.ifaces().values().filter(|x| x.passive).map(|x| x.name.as_str()).collect();
        if !passive.is_empty() {
            s.push_str("  Passive interfaces:\n");
            for p in passive {
                s.push_str(&format!("    {}\n", p));
            }
        }
        s.push_str("  Routing information sources:\n    Gateway         Distance      Last Update\n");
        for (a, at) in r.sources() {
            s.push_str(&format!("    {:<15} {:>8}      {}\n", format!("{}", a), c.distance, hms(now.saturating_sub(*at))));
        }
        s.push_str(&format!("  Distance: {} (default {})\n", c.distance, DISTANCE));
        s
    }

    /// "debug ip rip": a decision of RIP as lines (indented ones without a time stamp)
    pub(crate) fn rip_debug(&self, ni: usize, note: &Note) -> String {
        let ifn = |i: u16| self.rip_ifname(ni, i);
        let routes = |ver: u8, rs: &[(netpkt::Prefix4, u8)]| -> String {
            let mut t = String::new();
            for (p, m) in rs {
                if ver == 1 {
                    t.push_str(&format!("      {} metric {}\n", p.addr, m));
                } else {
                    t.push_str(&format!("      {} metric {}\n", p, m));
                }
            }
            t
        };
        match note {
            Note::RipRequest { iface, ver } => format!("RIP: asking for the whole table on {} (version {})\n", ifn(*iface), ver),
            Note::RipAsked { from, iface, whole } => format!("RIP: {} on {} asks for {}\n", from, ifn(*iface), if *whole { "the whole table" } else { "single routes" }),
            Note::RipSent { iface, ver, to, kind, routes: rs, .. } => {
                let what = match kind {
                    RipOut::Regular => "update",
                    RipOut::Triggered => "triggered update",
                    RipOut::Answer => "answer",
                };
                format!("RIP: {} v{} out of {} to {}, {} route{}\n{}", what, ver, ifn(*iface), to, rs.len(), if rs.len() == 1 { "" } else { "s" }, routes(*ver, rs))
            }
            Note::RipHeard { from, iface, ver, routes: rs } => format!("RIP: update v{} from {} on {}, {} route{}\n{}", ver, from, ifn(*iface), rs.len(), if rs.len() == 1 { "" } else { "s" }, routes(*ver, rs)),
            Note::RipIgnored { from, iface, why } => format!("RIP: ignored a message from {} on {}: {}\n", from, ifn(*iface), why.en()),
            Note::RipBadEntry { from, addr, metric } => format!("RIP: ignored the entry {} metric {} from {}\n", addr, metric, from),
            Note::RipLearned { prefix, via, metric } => format!("RIP: new route {} via {}, metric {}\n", prefix, via, metric),
            Note::RipChanged { prefix, via, metric, old } => format!("RIP: {} now via {}, metric {} (was {})\n", prefix, via, metric, old),
            Note::RipPathAdded { prefix, via, metric } => format!("RIP: one more path to {} via {}, metric {}\n", prefix, via, metric),
            Note::RipDown { prefix, via, why } => format!("RIP: {} via {} lost ({}): unreachable, advertised with metric 16\n", prefix, via, why.en()),
            Note::RipPathGone { prefix, via, why } => format!("RIP: path to {} via {} dropped ({})\n", prefix, via, why.en()),
            Note::RipHeld { prefix, from, metric } => format!("RIP: {} is held down: ignoring {} with metric {}\n", prefix, from, metric),
            Note::RipFlushed { prefix } => format!("RIP: {} removed from the table\n", prefix),
            Note::RipLocal { prefix, how } => format!("RIP: advertising {} ({})\n", prefix, how.en()),
            Note::RipLocalGone { prefix } => format!("RIP: {} is gone here: advertised with metric 16\n", prefix),
            _ => String::new(),
        }
    }
}
