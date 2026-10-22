// ospf -- OSPF at the IOS-like command line: "router ospf N" with its own tree mode behind the
// prompt "(config-router)#", "ip ospf ..." on interfaces, show ip ospf [neighbor | interface
// [brief] | database], the O routes of show ip route, the OSPF part of show ip protocols, debug
// ip ospf adj|events|hello, clear ip ospf process, and the lines of the running configuration;
// help texts, messages, the texts above tables and the debug lines are our own words, the command
// words, configuration lines and the tables follow the originals
//
//   build_ospf(tree, modes, router)  the commands in an IOS command tree (ios.rs) where the
//                                    device routes; the router mode of OSPF is a tree mode of its
//                                    own (MODE_KEY: ios.rs finds it by that key, the console shows
//                                    "(config-router)#" as for RIP)
//   Engine::ios_ospf(ni, parsed)     every OSPF command
//   Engine::ios_ospf_config(ni)      the "router ospf" block of the running configuration (after
//                                    "router rip"); ios_ospf_if_config(ni, iface) the "ip ospf"
//                                    lines of an interface
//   Engine::ospf_route_code(ni, r)   "O", "O IA", "O E1", "O E2" (with "*" for the default route)
//   Engine::ospf_protocols(ni)       its part of show ip protocols
//   Engine::ospf_debug(ni, note)     "debug ip ospf": a decision of OSPF as console lines
//   DO_ADJ, DO_EVENTS, DO_HELLO      what "debug ip ospf adj|events|hello" shows (CliDev::ospf_debug;
//                                    none set while D_OSPF is on: everything, as after "debug all")
//
// The block lists only what differs from the defaults, in this order: router-id,
// log-adjacency-changes, auto-cost, area ... stub, redistribute, passive-interface, network,
// default-information originate, distance, maximum-paths.  "router ospf" needs routing ("% IP
// routing not enabled" on an L3 switch without "ip routing"); one process per device.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};
use jfn_model::ifname;
use netpkt::ospf as wire;
use netpkt::{Ipv4, Prefix4};

use super::ios::K;
use super::{Ask, AskKind, Mode, Proc, D_OSPF};
use crate::action::Action;
use crate::ip::Route;
use crate::kernel::{Engine, Ns, SEC};
use crate::ospf::{area_str, DefaultOrig, IfState, LsuWhy, NbrState, NetArea, NetType, OspfSet, Redist, RtChange, RtKind, Source, DISTANCE, MAX_PATHS, REF_BW, RXMT, SPF_DELAY};
use crate::trace::Note;

/// the key the tree knows OSPF's router mode by (its prompt on the console is "(config-router)#")
pub const MODE_KEY: &str = "(config-router)# ospf";

/// what "debug ip ospf adj|events|hello" shows
pub const DO_ADJ: u8 = 1;
pub const DO_EVENTS: u8 = 2;
pub const DO_HELLO: u8 = 4;

/// "00:00:12": hours, minutes, seconds
fn hms(ns: Ns) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// the OSPF commands in an IOS tree; `m`: user, privileged, global configuration, interface and
/// subinterface mode
pub(crate) fn build_ospf(t: &mut Tree<K>, m: [ModeId; 5], router: bool) {
    let [user, prv, conf, ifm, sub] = m;
    if !router {
        return;
    }
    for x in [user, prv] {
        t.add(x, "show ip ospf", "State of the OSPF process", Some(K::ShowOspf), 0);
        t.add(x, "show ip ospf neighbor", "The neighbours with their state", Some(K::ShowOspfNbr), 0);
        t.add(x, "show ip ospf interface", "The interfaces OSPF runs on, in detail", Some(K::ShowOspfIf), 0);
        t.add(x, "show ip ospf interface brief", "One line per interface", Some(K::ShowOspfIf), 0);
        t.add(x, "show ip ospf interface NAME1=if", "", Some(K::ShowOspfIf), 0);
        t.add(x, "show ip ospf database", "The LSAs of every area", Some(K::ShowOspfDb), 0);
        t.add(x, "show ip route ospf=only", "Only the routes OSPF found", Some(K::ShowIpRoute), 0);
    }
    for (w, h) in [("adj", "Neighbour states, elections and the database exchange"), ("events", "Interfaces, neighbours, LSAs, SPF runs and routes"), ("hello", "Hello packets sent, taken and ignored")] {
        t.add(prv, "debug ip ospf", "OSPF decisions", None, 0);
        t.add(prv, &format!("debug ip ospf {}=what", w), h, Some(K::DebugOspf), 0);
        t.add(prv, "undebug ip ospf", "OSPF decisions", None, 0);
        t.add(prv, &format!("undebug ip ospf {}=what", w), h, Some(K::DebugOspf), 0);
    }
    t.add(prv, "clear ip ospf", "Start the OSPF process again", None, 0);
    t.add(prv, "clear ip ospf process", "Forget neighbours, LSAs and routes and start anew", Some(K::ClearOspf), 0);

    // global configuration ("router" itself is there, with its "no", from RIP)
    t.add(conf, "router", "Start a routing process", None, F_NO);
    t.add(conf, "router ospf", "OSPF: link states and shortest paths (RFC 2328)", None, 0);
    t.add(conf, "router ospf <1-65535>=pid", "Process number, only for this device", Some(K::RouterOspf), 0);

    // the router mode of OSPF: like every configuration mode, then its own commands
    let r = t.mode(MODE_KEY, "Commands for this OSPF process:", Some(conf), Some("Undo a setting or restore its default"));
    t.add(r, "do", "Run an EXEC command from here", None, 0);
    t.add(r, "do LINE=cmd", "The command", Some(K::Do), 0);
    t.add(r, "exit", "Back to the device configuration", Some(K::Exit), 0);
    t.add(r, "end", "Finish configuring", Some(K::End), F_HIDDEN);
    t.add(r, "router-id", "The 32-bit name of this router in OSPF", None, F_NO);
    t.add(r, "router-id A.B.C.D=rid", "Written like an address", Some(K::OspfRouterId), 0);
    t.add_no(r, "router-id", K::OspfRouterId);
    t.add(r, "network", "Run OSPF on the interfaces of these addresses", None, F_NO);
    t.add(r, "network A.B.C.D=net", "An address", None, 0);
    t.add(r, "network A.B.C.D=net A.B.C.D=wild", "Wildcard: the bits that need not match", None, 0);
    t.add(r, "network A.B.C.D=net A.B.C.D=wild area", "The area of these interfaces", None, 0);
    t.add(r, "network A.B.C.D=net A.B.C.D=wild area <0-4294967295>=area", "Area number (0: the backbone)", Some(K::OspfNetwork), 0);
    t.add(r, "network A.B.C.D=net A.B.C.D=wild area A.B.C.D=areaip", "Area number written like an address", Some(K::OspfNetwork), 0);
    t.add(r, "passive-interface", "Send and take no OSPF packets on an interface; its network stays", None, F_NO);
    t.add(r, "passive-interface default", "Every interface not named with no passive-interface", Some(K::OspfPassive), 0);
    t.add(r, "passive-interface NAME1=if", "", Some(K::OspfPassive), 0);
    t.add(r, "auto-cost", "Interface costs from the bandwidth", None, F_NO);
    t.add(r, "auto-cost reference-bandwidth", "The speed that costs 1", None, 0);
    t.add(r, "auto-cost reference-bandwidth <1-4294967>=bw", "Mbit/s (default 100)", Some(K::OspfAutoCost), 0);
    t.add_no(r, "auto-cost reference-bandwidth", K::OspfAutoCost);
    let d = "default-information originate";
    t.add(r, "default-information", "The default route", None, F_NO);
    t.add(r, d, "Advertise a default route as AS boundary router", Some(K::OspfDefault), 0);
    t.add(r, &format!("{} always", d), "Even without a default route of its own", Some(K::OspfDefault), 0);
    t.add(r, &format!("{} metric", d), "The metric it gets", None, 0);
    t.add(r, &format!("{} metric <0-16777214>=metric", d), "Default 1", Some(K::OspfDefault), 0);
    t.add(r, &format!("{} metric-type", d), "External type 1 or 2", None, 0);
    t.add(r, &format!("{} metric-type <1-2>=mtype", d), "1: the costs inside add up, 2: they do not (default)", Some(K::OspfDefault), 0);
    for end in ["always", "metric <0-16777214>=metric", "metric-type <1-2>=mtype"] {
        t.also(r, &format!("{} {}", d, end), d);
    }
    t.add(r, "redistribute", "Advertise routes of another source as AS-external", None, F_NO);
    for (w, h) in [("connected", "Networks of interfaces OSPF does not run on"), ("static", "Static routes"), ("rip", "Routes RIP learned")] {
        let b = format!("redistribute {}=what", w);
        t.add(r, &b, h, Some(K::OspfRedist), 0);
        t.add(r, &format!("{} subnets", b), "Subnets too, not only classful networks", Some(K::OspfRedist), 0);
        t.add(r, &format!("{} metric", b), "The metric they get", None, 0);
        t.add(r, &format!("{} metric <0-16777214>=metric", b), "Default 20", Some(K::OspfRedist), 0);
        t.add(r, &format!("{} metric-type", b), "External type 1 or 2", None, 0);
        t.add(r, &format!("{} metric-type <1-2>=mtype", b), "1: the costs inside add up, 2: they do not (default)", Some(K::OspfRedist), 0);
        for end in ["subnets", "metric <0-16777214>=metric", "metric-type <1-2>=mtype"] {
            t.also(r, &format!("{} {}", b, end), &b);
        }
    }
    t.add(r, "area", "Settings of an area", None, F_NO);
    for a in ["<0-4294967295>=area", "A.B.C.D=areaip"] {
        t.add(r, &format!("area {}", a), "The area", None, 0);
        t.add(r, &format!("area {} stub", a), "No AS-external LSAs here, a default route instead", Some(K::OspfStub), 0);
        t.add(r, &format!("area {} stub no-summary", a), "Nor summaries of other areas", Some(K::OspfStub), 0);
    }
    t.add(r, "distance", "Administrative distance of the routes OSPF finds", None, F_NO);
    t.add(r, "distance <1-255>=ad", "Lower wins against other sources (default 110)", Some(K::OspfDistance), 0);
    t.add_no(r, "distance", K::OspfDistance);
    t.add(r, "maximum-paths", "Equal-cost paths to use", None, F_NO);
    t.add(r, "maximum-paths <1-32>=paths", "How many (default 4)", Some(K::OspfMaxPaths), 0);
    t.add_no(r, "maximum-paths", K::OspfMaxPaths);
    t.add(r, "log-adjacency-changes", "Say on the console when a neighbour comes or goes", Some(K::OspfLogAdj), F_NO);
    t.add(r, "log-adjacency-changes detail", "Every change of a neighbour's state", Some(K::OspfLogAdj), 0);

    // interfaces ("ip" itself is there, with its "no")
    for x in [ifm, sub] {
        t.add(x, "ip ospf", "OSPF on this interface", None, 0);
        t.add(x, "ip ospf cost", "What a path out of this interface costs", None, 0);
        t.add(x, "ip ospf cost <1-65535>=c", "Instead of the cost from the bandwidth", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf cost", K::OspfIf);
        t.add(x, "ip ospf priority", "How much this router wants to be DR here", None, 0);
        t.add(x, "ip ospf priority <0-255>=prio", "0: never DR or BDR (default 1)", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf priority", K::OspfIf);
        t.add(x, "ip ospf hello-interval", "Seconds between hellos", None, 0);
        t.add(x, "ip ospf hello-interval <1-65535>=hi", "Must match the neighbours (default 10)", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf hello-interval", K::OspfIf);
        t.add(x, "ip ospf dead-interval", "Seconds without a hello until a neighbour is gone", None, 0);
        t.add(x, "ip ospf dead-interval <1-65535>=di", "Must match the neighbours (default four hellos)", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf dead-interval", K::OspfIf);
        t.add(x, "ip ospf network", "The kind of network behind this interface", None, 0);
        t.add(x, "ip ospf network point-to-point=nt", "One neighbour, no DR election", Some(K::OspfIf), 0);
        t.add(x, "ip ospf network broadcast=nt", "A LAN with a DR and a BDR", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf network", K::OspfIf);
        t.add(x, "ip ospf <1-65535>=pid", "The OSPF process", None, 0);
        t.add(x, "ip ospf <1-65535>=pid area", "Run it here in an area, without a network statement", None, 0);
        t.add(x, "ip ospf <1-65535>=pid area <0-4294967295>=area", "Area number", Some(K::OspfIf), 0);
        t.add(x, "ip ospf <1-65535>=pid area A.B.C.D=areaip", "Area number written like an address", Some(K::OspfIf), 0);
        t.add_no(x, "ip ospf <1-65535>=pid area", K::OspfIf);
    }
}

/// the area of a command: "area 1" or "area 0.0.0.1" (dotted)
fn area_of(p: &Parsed<K>) -> (u32, bool) {
    match p.ip("areaip") {
        Some(a) => (Ipv4(a).to_u32(), true),
        None => (p.num("area").unwrap_or(0) as u32, false),
    }
}

/// "Area 0 (backbone)", "Area 1 (stub)"
fn area_line(e: &Engine, ni: usize, a: u32) -> String {
    let o = e.ospf_of(ni);
    let text = o.map(|o| o.cfg.area_text(a)).unwrap_or_else(|| area_str(a, false));
    let what = match o.map(|o| o.cfg.stubs.get(&a).copied()) {
        _ if a == 0 => " (backbone)",
        Some(Some(true)) => " (stub, without summaries)",
        Some(Some(false)) => " (stub)",
        _ => "",
    };
    format!("Area {}{}", text, what)
}

impl Engine {
    /// every OSPF command
    pub(crate) fn ios_ospf(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let set = match p.cmd {
            K::RouterOspf => {
                let pid = p.num("pid").unwrap_or(1) as u16;
                let cur = self.ospf_of(ni).filter(|o| o.cfg.on).map(|o| o.cfg.pid);
                if p.no {
                    if cur != Some(pid) {
                        return self.ios_err(ni, &format!("% There is no OSPF process {} here\n", pid));
                    }
                    let _ = self.apply(Action::Ospf { dev, set: OspfSet::Process { pid, on: false } });
                    return Ok(());
                }
                if !self.stack(ni).is_some_and(|s| s.forwarding) {
                    return self.ios_err(ni, "% IP routing not enabled\n");
                }
                if let Some(c) = cur.filter(|c| *c != pid) {
                    return self.ios_err(ni, &format!("% Only one OSPF process per device here: process {} runs\n", c));
                }
                if let Err(e) = self.apply(Action::Ospf { dev, set: OspfSet::Process { pid, on: true } }) {
                    return self.ios_err(ni, &format!("% {}\n", e));
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.mode = Mode::Router(Proc::Ospf(pid));
                    c.ifs.clear();
                    c.vlans.clear();
                    c.pool.clear();
                }
                return Ok(());
            }
            K::ShowOspf | K::ShowOspfNbr | K::ShowOspfIf | K::ShowOspfDb => {
                let text = match p.cmd {
                    K::ShowOspf => self.show_ospf(ni),
                    K::ShowOspfNbr => self.show_ospf_nbr(ni),
                    K::ShowOspfIf => self.show_ospf_if(ni, p.has("brief"), p.text("if")),
                    _ => self.show_ospf_db(ni),
                };
                self.out(ni, &text);
                return Ok(());
            }
            K::DebugOspf => {
                let kind = match p.text("what") {
                    Some("adj") => DO_ADJ,
                    Some("events") => DO_EVENTS,
                    _ => DO_HELLO,
                };
                let off = p.has("undebug") || p.no;
                let (before, kinds) = self.cdev(ni).map(|c| (c.debug, if c.debug & D_OSPF != 0 { c.ospf_debug } else { 0 })).unwrap_or((0, 0));
                let kinds = if off { kinds & !kind } else { kinds | kind };
                let after = if kinds != 0 { before | D_OSPF } else { before & !D_OSPF };
                if let Some(c) = self.cdev_mut(ni) {
                    c.debug = after;
                    c.ospf_debug = kinds;
                }
                if before == 0 && after != 0 {
                    self.cli.debug += 1;
                } else if before != 0 && after == 0 {
                    self.cli.debug = self.cli.debug.saturating_sub(1);
                }
                let what = match kind {
                    DO_ADJ => "adjacencies",
                    DO_EVENTS => "events",
                    _ => "hellos",
                };
                self.out(ni, &format!("Debug output for OSPF {}: {}\n", what, if off { "off" } else { "on" }));
                return Ok(());
            }
            K::ClearOspf => {
                if !self.ospf_on(ni) {
                    return self.ios_err(ni, "% No OSPF process runs here\n");
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.ask = Some(Ask { prompt: String::from("Start every OSPF process anew? [no]: "), kind: AskKind::ClearOspf });
                }
                return Ok(());
            }
            K::OspfIf => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if self.stack(ni).and_then(|s| s.iface_by_name(&f)).is_none() {
                        r = self.ios_err(ni, "% IP addresses may not be configured on L2 links.\n");
                        continue;
                    }
                    let iface = f.clone();
                    let set = if p.has("cost") {
                        OspfSet::IfCost { iface, cost: if p.no { None } else { p.num("c").map(|v| v as u16) } }
                    } else if p.has("priority") {
                        OspfSet::IfPriority { iface, priority: if p.no { None } else { p.num("prio").map(|v| v as u8) } }
                    } else if p.has("hello-interval") {
                        OspfSet::IfHello { iface, secs: if p.no { None } else { p.num("hi").map(|v| v as u16) } }
                    } else if p.has("dead-interval") {
                        OspfSet::IfDead { iface, secs: if p.no { None } else { p.num("di").map(|v| v as u32) } }
                    } else if p.has("network") {
                        let kind = match p.text("nt") {
                            Some("point-to-point") => Some(NetType::PointToPoint),
                            Some(_) => Some(NetType::Broadcast),
                            None => None,
                        };
                        OspfSet::IfNetwork { iface, kind: if p.no { None } else { kind } }
                    } else {
                        let pid = p.num("pid").unwrap_or(1) as u16;
                        if !p.no
                            && let Some(c) = self.ospf_of(ni).filter(|o| o.cfg.on && o.cfg.pid != pid).map(|o| o.cfg.pid)
                        {
                            r = self.ios_err(ni, &format!("% Only one OSPF process per device here: process {} runs\n", c));
                            continue;
                        }
                        let (a, dotted) = area_of(p);
                        OspfSet::IfArea { iface, area: if p.no { None } else { Some((pid, a, dotted)) } }
                    };
                    if let Err(e) = self.apply(Action::Ospf { dev, set }) {
                        r = self.ios_err(ni, &format!("% {}\n", e));
                    }
                }
                return r;
            }
            K::OspfRouterId => {
                let rid = if p.no { None } else { p.ip("rid").map(Ipv4) };
                if let Some(a) = rid
                    && (a.is_unspecified() || a.is_multicast() || a.is_broadcast())
                {
                    return self.ios_err(ni, &format!("% {} cannot be a router ID\n", a));
                }
                let running = self.ospf_of(ni).and_then(|o| o.router_id());
                if let Err(e) = self.apply(Action::Ospf { dev, set: OspfSet::RouterId(rid) }) {
                    return self.ios_err(ni, &format!("% {}\n", e));
                }
                if running.is_some() && running != rid && !self.cdev(ni).is_some_and(|c| c.booting) {
                    self.out(ni, "The router ID in use changes with clear ip ospf process or a restart.\n");
                }
                return Ok(());
            }
            K::OspfNetwork => {
                let (area, dotted) = area_of(p);
                let net = NetArea { addr: Ipv4(p.ip("net").unwrap_or([0; 4])), wild: Ipv4(p.ip("wild").unwrap_or([0; 4])), area, dotted };
                OspfSet::Network { net, add: !p.no }
            }
            K::OspfPassive => OspfSet::Passive { iface: p.text("if").map(String::from), on: !p.no },
            K::OspfAutoCost => {
                let bw = if p.no { None } else { p.num("bw").map(|v| v as u32) };
                if !self.cdev(ni).is_some_and(|c| c.booting) {
                    self.out(ni, "Interface costs follow the new reference bandwidth: give every router the same one.\n");
                }
                OspfSet::RefBw(bw)
            }
            K::OspfDefault => OspfSet::DefaultOrig(if p.no { None } else { Some(DefaultOrig { always: p.has("always"), metric: p.num("metric").map(|v| v as u32), e1: p.num("mtype") == Some(1) }) }),
            K::OspfRedist => {
                let what = match p.text("what") {
                    Some("static") => Source::Static,
                    Some("rip") => Source::Rip,
                    _ => Source::Connected,
                };
                let subnets = p.has("subnets");
                if !p.no && !subnets && !self.cdev(ni).is_some_and(|c| c.booting) {
                    self.out(ni, "% Only classful networks will be redistributed\n");
                }
                OspfSet::Redistribute { r: Redist { what, subnets, metric: p.num("metric").map(|v| v as u32), e1: p.num("mtype") == Some(1) }, add: !p.no }
            }
            K::OspfStub => {
                let (area, _) = area_of(p);
                if area == 0 {
                    return self.ios_err(ni, "% The backbone cannot be a stub area\n");
                }
                let ns = p.has("no-summary");
                if p.no && ns {
                    OspfSet::Stub { area, no_summary: false, add: true }
                } else {
                    OspfSet::Stub { area, no_summary: ns, add: !p.no }
                }
            }
            K::OspfDistance => OspfSet::Distance(if p.no { None } else { p.num("ad").map(|d| d as u8) }),
            K::OspfMaxPaths => OspfSet::MaxPaths(if p.no { None } else { p.num("paths").map(|d| d as u8) }),
            K::OspfLogAdj => OspfSet::LogAdj { on: !p.no, detail: p.has("detail") },
            _ => return Ok(()),
        };
        match self.apply(Action::Ospf { dev, set }) {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    /// the answer to "Start every OSPF process anew? [no]:"
    pub(crate) fn ios_ospf_clear(&mut self, ni: usize, line: &str) {
        let t = line.trim();
        if t.eq_ignore_ascii_case("yes") || t.eq_ignore_ascii_case("y") {
            let dev = self.dev_of(ni);
            let _ = self.apply(Action::Ospf { dev, set: OspfSet::Clear });
        }
    }

    // ---------------------------------------------------------------- configuration

    /// the "router ospf" block of the running configuration ("" without one)
    pub(crate) fn ios_ospf_config(&self, ni: usize) -> String {
        let Some(c) = self.ospf_of(ni).map(|o| &o.cfg).filter(|c| c.on) else {
            return String::new();
        };
        let mut s = format!("router ospf {}\n", c.pid);
        if let Some(r) = c.router_id {
            s.push_str(&format!(" router-id {}\n", r));
        }
        if !c.log_adj {
            s.push_str(" no log-adjacency-changes\n");
        } else if c.log_detail {
            s.push_str(" log-adjacency-changes detail\n");
        }
        if c.ref_bw != REF_BW {
            s.push_str(&format!(" auto-cost reference-bandwidth {}\n", c.ref_bw));
        }
        for (a, ns) in &c.stubs {
            s.push_str(&format!(" area {} stub{}\n", c.area_text(*a), if *ns { " no-summary" } else { "" }));
        }
        for r in &c.redistribute {
            let mut l = format!(" redistribute {}", r.what.word());
            if let Some(m) = r.metric {
                l.push_str(&format!(" metric {}", m));
            }
            if r.e1 {
                l.push_str(" metric-type 1");
            }
            if r.subnets {
                l.push_str(" subnets");
            }
            s.push_str(&l);
            s.push('\n');
        }
        if c.passive_default {
            s.push_str(" passive-interface default\n");
        }
        for p in &c.passive {
            s.push_str(&format!(" {}passive-interface {}\n", if c.passive_default { "no " } else { "" }, p));
        }
        for n in &c.networks {
            s.push_str(&format!(" network {} {} area {}\n", n.addr, n.wild, area_str(n.area, n.dotted)));
        }
        if let Some(d) = c.default_orig {
            let mut l = String::from(" default-information originate");
            if d.always {
                l.push_str(" always");
            }
            if let Some(m) = d.metric {
                l.push_str(&format!(" metric {}", m));
            }
            if d.e1 {
                l.push_str(" metric-type 1");
            }
            s.push_str(&l);
            s.push('\n');
        }
        if c.distance != DISTANCE {
            s.push_str(&format!(" distance {}\n", c.distance));
        }
        if c.max_paths != MAX_PATHS {
            s.push_str(&format!(" maximum-paths {}\n", c.max_paths));
        }
        s.push_str("!\n");
        s
    }

    /// the "ip ospf ..." lines of an interface
    pub(crate) fn ios_ospf_if_config(&self, ni: usize, iface: &str) -> String {
        let Some(c) = self.ospf_of(ni).map(|o| &o.cfg) else {
            return String::new();
        };
        let get = |m: &alloc::collections::BTreeMap<String, u32>| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v);
        let mut s = String::new();
        if let Some(k) = c.net_type.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
            s.push_str(match k {
                NetType::PointToPoint => " ip ospf network point-to-point\n",
                _ => " ip ospf network broadcast\n",
            });
        }
        if let Some(h) = c.hello.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
            s.push_str(&format!(" ip ospf hello-interval {}\n", h));
        }
        if let Some(d) = get(&c.dead) {
            s.push_str(&format!(" ip ospf dead-interval {}\n", d));
        }
        if let Some(p) = c.priority.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
            s.push_str(&format!(" ip ospf priority {}\n", p));
        }
        if let Some(x) = c.cost.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
            s.push_str(&format!(" ip ospf cost {}\n", x));
        }
        if let Some((pid, a, dotted)) = c.if_area.iter().find(|(k, _)| k.eq_ignore_ascii_case(iface)).map(|(_, v)| *v) {
            s.push_str(&format!(" ip ospf {} area {}\n", pid, area_str(a, dotted)));
        }
        s
    }

    // ---------------------------------------------------------------- show

    fn ospf_ifname(&self, ni: usize, i: u16) -> String {
        self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default()
    }

    /// show ip ospf: the process in a few lines of our own
    fn show_ospf(&self, ni: usize) -> String {
        let Some(o) = self.ospf_of(ni).filter(|o| o.cfg.on) else {
            return String::new();
        };
        let now = self.now;
        let c = &o.cfg;
        let Some(rid) = o.router_id() else {
            return format!("OSPF process {}: no router ID yet (no interface with an address is up)\n", c.pid);
        };
        let yes = |b: bool| if b { "yes" } else { "no" };
        let mut s = format!("OSPF process {} with router ID {} (RFC 2328)\n", c.pid, rid);
        s.push_str(&format!("  Running for {}; area border router: {}; AS boundary router: {}\n", hms(now.saturating_sub(o.started())), yes(o.is_abr()), yes(o.is_asbr())));
        match o.last_spf() {
            Some(t) => s.push_str(&format!("  SPF runs {} ms after a change; runs so far: {}, the last {} ago\n", SPF_DELAY / 1_000_000, o.spf_runs(), hms(now.saturating_sub(t)))),
            None => s.push_str(&format!("  SPF runs {} ms after a change; it has not run yet\n", SPF_DELAY / 1_000_000)),
        }
        s.push_str(&format!("  LSAs are renewed every {} s; a new instance waits at least {} s, one that comes within {} s of the last is not taken\n", wire::LS_REFRESH_TIME, wire::MIN_LS_INTERVAL, wire::MIN_LS_ARRIVAL));
        s.push_str(&format!("  Interface costs: reference bandwidth {} Mbit/s\n", c.ref_bw));
        let log = match (c.log_adj, c.log_detail) {
            (false, _) => "not reported",
            (true, false) => "reported",
            (true, true) => "reported in detail",
        };
        s.push_str(&format!("  Distance {}, up to {} equal-cost paths; adjacency changes are {}\n", c.distance, c.max_paths, log));
        s.push_str(&format!("  AS-external LSAs: {}\n", o.externals().len()));
        let areas = o.attached();
        let stubs = areas.iter().filter(|a| c.is_stub(**a)).count();
        s.push_str(&format!("  Areas: {} ({} normal, {} stub)\n", areas.len(), areas.len() - stubs, stubs));
        for a in areas {
            s.push_str(&format!("    {}\n", area_line(self, ni, a)));
            let ifs: Vec<_> = o.ifaces().values().filter(|f| f.area == a && f.state != IfState::Down).collect();
            let passive = ifs.iter().filter(|f| f.passive).count();
            let full: usize = ifs.iter().map(|f| f.nbrs.values().filter(|n| n.state == NbrState::Full).count()).sum();
            s.push_str(&format!("      Interfaces: {} ({} passive), full adjacencies: {}\n", ifs.len(), passive, full));
            s.push_str(&format!("      LSAs: {}\n", o.database(a).map(|d| d.len()).unwrap_or(0)));
        }
        s
    }

    /// show ip ospf neighbor (the table of the original)
    fn show_ospf_nbr(&self, ni: usize) -> String {
        let Some(o) = self.ospf_of(ni).filter(|o| o.cfg.on) else {
            return String::new();
        };
        let now = self.now;
        let mut s = String::from("\nNeighbor ID     Pri   State           Dead Time   Address         Interface\n");
        for (i, f) in o.ifaces() {
            for n in f.nbrs.values() {
                let state = format!("{}/{}", n.state.word(), f.role(n.addr));
                s.push_str(&format!("{:<16}{:>3}   {:<16}{:<12}{:<16}{}\n", format!("{}", n.id), n.priority, state, hms(n.dead_at.saturating_sub(now)), format!("{}", n.addr), self.ospf_ifname(ni, *i)));
            }
        }
        s
    }

    /// show ip ospf interface [brief] [NAME]
    fn show_ospf_if(&self, ni: usize, brief: bool, only: Option<&str>) -> String {
        let Some(o) = self.ospf_of(ni).filter(|o| o.cfg.on) else {
            return String::new();
        };
        let now = self.now;
        let rid = o.router_id().unwrap_or_default();
        let mut s = String::new();
        if brief {
            s.push_str("Interface    PID   Area            IP Address/Mask    Cost  State Nbrs F/C\n");
        }
        for (i, f) in o.ifaces() {
            if only.is_some_and(|x| !x.eq_ignore_ascii_case(&f.name) && self.stack(ni).and_then(|st| st.iface_by_name(x)) != Some(*i)) {
                continue;
            }
            let full = f.nbrs.values().filter(|n| n.state == NbrState::Full).count();
            let cost = if f.kind == NetType::Loopback { 0 } else { f.cost };
            if brief {
                s.push_str(&format!("{:<13}{:<6}{:<16}{:<19}{:<6}{:<6}{}/{}\n", ifname::short(&f.name), o.cfg.pid, o.cfg.area_text(f.area), format!("{}", f.addr), cost, f.state.word(), full, f.nbrs.len()));
                continue;
            }
            let up = f.state != IfState::Down;
            s.push_str(&format!("{} is {}, line protocol is {}\n", f.name, if up { "up" } else { "down" }, if up { "up" } else { "down" }));
            s.push_str(&format!("  Internet address {}, area {}, process {}, router ID {}\n", f.addr, o.cfg.area_text(f.area), o.cfg.pid, rid));
            s.push_str(&format!("  Network type {}, cost {}, transmit delay 1 s, state {}, priority {}\n", f.kind.word(), cost, f.state.word(), f.priority));
            if f.kind == NetType::Loopback {
                s.push_str("  Loopback: advertised as a host route (/32) with cost 0\n");
                continue;
            }
            if f.passive {
                s.push_str("  Passive: no hellos are sent or taken here, its network is advertised\n");
                continue;
            }
            if f.kind == NetType::Broadcast {
                let who = |a: Ipv4| if a.is_unspecified() { String::from("none") } else { format!("{} at {}", f.rid_of(a, rid), a) };
                s.push_str(&format!("  Designated router {}, backup {}\n", who(f.dr), who(f.bdr)));
            }
            s.push_str(&format!("  Timers: hello {} s, dead {} s, wait {} s, retransmit {} s", f.hello, f.dead, f.dead, RXMT));
            if let Some(t) = f.hello_at {
                s.push_str(&format!("; next hello in {}", hms(t.saturating_sub(now))));
            }
            s.push('\n');
            let adj: Vec<String> = f.nbrs.values().filter(|n| n.state == NbrState::Full).map(|n| format!("{}", n.id)).collect();
            if adj.is_empty() {
                s.push_str(&format!("  Neighbours: {}, adjacent: 0\n", f.nbrs.len()));
            } else {
                s.push_str(&format!("  Neighbours: {}, adjacent: {} ({})\n", f.nbrs.len(), adj.len(), adj.join(", ")));
            }
        }
        s
    }

    /// show ip ospf database: the LSAs of every area and the AS-external ones (the tables of the
    /// original)
    fn show_ospf_db(&self, ni: usize) -> String {
        let Some(o) = self.ospf_of(ni).filter(|o| o.cfg.on) else {
            return String::new();
        };
        let Some(rid) = o.router_id() else {
            return String::new();
        };
        let now = self.now;
        let mut s = format!("\n            OSPF router {}, process {}\n", rid, o.cfg.pid);
        let row = |e: &crate::ospf::Entry, tail: String| -> String {
            let h = e.hdr(now);
            format!("{:<16}{:<16}{:<12}0x{:08X} 0x{:06X} {}", format!("{}", h.id), format!("{}", h.adv), h.age, h.seq, h.checksum, tail).trim_end().into()
        };
        for (a, db) in o.areas() {
            let at = o.cfg.area_text(*a);
            for (kind, title, head) in [
                (wire::LSA_ROUTER, "Router Link States", "Link ID         ADV Router      Age         Seq#       Checksum Link count"),
                (wire::LSA_NETWORK, "Net Link States", "Link ID         ADV Router      Age         Seq#       Checksum"),
                (wire::LSA_SUMMARY, "Summary Net Link States", "Link ID         ADV Router      Age         Seq#       Checksum"),
                (wire::LSA_ASBR, "Summary ASB Link States", "Link ID         ADV Router      Age         Seq#       Checksum"),
            ] {
                let rows: Vec<&crate::ospf::Entry> = db.iter().filter(|(k, _)| k.kind == kind).map(|(_, e)| e).collect();
                if rows.is_empty() {
                    continue;
                }
                s.push_str(&format!("\n                {} (Area {})\n\n{}\n", title, at, head));
                for e in rows {
                    let tail = match &e.lsa.body {
                        wire::LsaBody::Router(r) => format!("{}", r.links.len()),
                        _ => String::new(),
                    };
                    s.push_str(&row(e, tail));
                    s.push('\n');
                }
            }
        }
        if !o.externals().is_empty() {
            s.push_str("\n                Type-5 AS External Link States\n\nLink ID         ADV Router      Age         Seq#       Checksum Tag\n");
            for e in o.externals().values() {
                let tag = match &e.lsa.body {
                    wire::LsaBody::External(x) => format!("{}", x.tag),
                    _ => String::new(),
                };
                s.push_str(&row(e, tag));
                s.push('\n');
            }
        }
        s
    }

    /// what show ip route writes before an OSPF route: "O", "O IA", "O E2", "O*E2"
    pub(crate) fn ospf_route_code(&self, ni: usize, r: &Route) -> String {
        let kind = self.ospf_of(ni).and_then(|o| o.route_of(r.prefix)).map(|x| x.kind).unwrap_or(RtKind::Intra);
        let star = r.prefix.len == 0;
        match (kind.code(), star) {
            ("", false) => String::from("O"),
            ("", true) => String::from("O*"),
            (c, false) => format!("O {}", c),
            (c, true) => format!("O*{}", c),
        }
    }

    /// how long an OSPF route is as it is (show ip route)
    pub(crate) fn ospf_route_age(&self, ni: usize, p: Prefix4) -> Ns {
        self.ospf_of(ni).and_then(|o| o.route_of(p)).map(|x| self.now.saturating_sub(x.since)).unwrap_or(0)
    }

    /// the OSPF process in show ip protocols ("" without one)
    pub(crate) fn ospf_protocols(&self, ni: usize) -> String {
        let Some(o) = self.ospf_of(ni).filter(|o| o.cfg.on) else {
            return String::new();
        };
        let c = &o.cfg;
        let now = self.now;
        let rid = o.router_id().map(|r| format!("{}", r)).unwrap_or_else(|| String::from("not chosen yet"));
        let yes = |b: bool| if b { "yes" } else { "no" };
        let mut s = format!("Routing process \"ospf {}\" with router ID {} (RFC 2328)\n", c.pid, rid);
        s.push_str(&format!("  Area border router: {}; AS boundary router: {}\n", yes(o.is_abr()), yes(o.is_asbr())));
        let mut from = format!("ospf {}", c.pid);
        for r in &c.redistribute {
            from.push_str(", ");
            from.push_str(r.what.word());
            if r.subnets {
                from.push_str(" subnets");
            }
        }
        s.push_str(&format!("  Redistributing: {}\n", from));
        if let Some(d) = c.default_orig {
            s.push_str(&format!("  Default route: advertised{}\n", if d.always { " always" } else { " while the RIB has one" }));
        }
        s.push_str(&format!("  Equal-cost paths: up to {}\n", c.max_paths));
        let areas = o.attached();
        let stubs = areas.iter().filter(|a| c.is_stub(**a)).count();
        s.push_str(&format!("  Areas: {} normal, {} stub\n", areas.len() - stubs, stubs));
        s.push_str("  Routing for networks:\n");
        for n in &c.networks {
            s.push_str(&format!("    {} {} area {}\n", n.addr, n.wild, area_str(n.area, n.dotted)));
        }
        if !c.if_area.is_empty() {
            s.push_str("  Interfaces with an area of their own:\n");
            for (name, (_, a, dotted)) in &c.if_area {
                s.push_str(&format!("    {} (area {})\n", name, area_str(*a, *dotted)));
            }
        }
        let passive: Vec<&str> = o.ifaces().values().filter(|f| f.passive).map(|f| f.name.as_str()).collect();
        if !passive.is_empty() {
            s.push_str("  Passive interfaces:\n");
            for p in passive {
                s.push_str(&format!("    {}\n", p));
            }
        }
        // the routers whose router-LSAs this one holds, with the time since they came
        s.push_str("  Routing information sources:\n    Gateway         Distance      Last Update\n");
        let mut src: alloc::collections::BTreeMap<Ipv4, Ns> = alloc::collections::BTreeMap::new();
        for db in o.areas().values() {
            for (k, e) in db {
                if k.kind == wire::LSA_ROUTER && Some(k.adv) != o.router_id() && !e.maxage(now) {
                    let t = src.entry(k.adv).or_insert(e.at);
                    *t = (*t).max(e.at);
                }
            }
        }
        for (a, at) in src {
            s.push_str(&format!("    {:<15} {:>8}      {}\n", format!("{}", a), c.distance, hms(now.saturating_sub(at))));
        }
        s.push_str(&format!("  Distance: {} (default {})\n", c.distance, DISTANCE));
        s
    }

    // ---------------------------------------------------------------- debug

    /// "debug ip ospf": a decision of OSPF as console lines, if its kind is switched on
    pub(crate) fn ospf_debug(&self, ni: usize, note: &Note) -> String {
        let kinds = match self.cdev(ni).map(|c| c.ospf_debug).unwrap_or(0) {
            0 => DO_ADJ | DO_EVENTS | DO_HELLO,
            k => k,
        };
        let of = match note {
            Note::OspfHelloSent { .. } | Note::OspfHelloRcvd { .. } => DO_HELLO,
            Note::OspfIgnored { .. } => DO_HELLO | DO_EVENTS,
            Note::OspfNbr { .. } | Note::OspfDr { .. } | Note::OspfIfState { .. } => DO_ADJ | DO_EVENTS,
            Note::OspfMaster { .. } | Note::OspfDd { .. } | Note::OspfLsr { .. } => DO_ADJ,
            _ => DO_EVENTS,
        };
        if kinds & of == 0 {
            return String::new();
        }
        let ifn = |i: u16| self.ospf_ifname(ni, i);
        let who = |a: Ipv4| if a.is_unspecified() { String::from("none") } else { format!("{}", a) };
        let n = |k: u16, one: &str, many: &str| format!("{} {}", k, if k == 1 { one } else { many });
        match note {
            Note::OspfRouterId { id } => format!("OSPF: the process takes the router ID {}\n", id),
            Note::OspfHelloSent { iface, nbrs } => format!("OSPF: hello out of {} to 224.0.0.5, {} named\n", ifn(*iface), n(*nbrs, "neighbour", "neighbours")),
            Note::OspfHelloRcvd { from, iface } => format!("OSPF: hello from {} on {}\n", from, ifn(*iface)),
            Note::OspfIgnored { from, iface, why } => format!("OSPF: ignored a packet from {} on {}: {}\n", from, ifn(*iface), why.en()),
            Note::OspfIfState { iface, from, to } => format!("OSPF: {} goes from {} to {}\n", ifn(*iface), from.word(), to.word()),
            Note::OspfDr { iface, dr, bdr } => format!("OSPF: election on {}: DR {}, BDR {}\n", ifn(*iface), who(*dr), who(*bdr)),
            Note::OspfNbr { nbr, iface, from, to, ev } => format!("OSPF: neighbour {} on {}: {} -> {} ({})\n", nbr, ifn(*iface), from.word(), to.word(), ev.name()),
            Note::OspfMaster { nbr, iface, master } => format!("OSPF: exchange with {} on {}: this router is {}\n", nbr, ifn(*iface), if *master { "master" } else { "slave" }),
            Note::OspfDd { nbr, iface, out, flags, seq, lsas } => {
                let dir = if *out { format!("out of {} to {}", ifn(*iface), nbr) } else { format!("from {} on {}", nbr, ifn(*iface)) };
                format!("OSPF: DD {}: flags {}, seq 0x{:08x}, {}\n", dir, wire::dd_flags_text(*flags), seq, n(*lsas, "LSA header", "LSA headers"))
            }
            Note::OspfLsr { nbr, iface, out: true, lsas } => format!("OSPF: asking {} on {} for {}\n", nbr, ifn(*iface), n(*lsas, "LSA", "LSAs")),
            Note::OspfLsr { nbr, iface, out: false, lsas } => format!("OSPF: {} on {} asks for {}\n", nbr, ifn(*iface), n(*lsas, "LSA", "LSAs")),
            Note::OspfLsu { nbr, iface, out: true, lsas, why } => {
                let to = nbr.map(|a| format!("{}", a)).unwrap_or_else(|| String::from("the group"));
                let w = match why {
                    LsuWhy::Flood => "flooding",
                    LsuWhy::Answer => "the answer to a request",
                    LsuWhy::Rxmt => "again, not acknowledged",
                    LsuWhy::Back => "the newer copy of the database",
                };
                format!("OSPF: update out of {} to {}, {} ({})\n", ifn(*iface), to, n(*lsas, "LSA", "LSAs"), w)
            }
            Note::OspfLsu { nbr, iface, lsas, .. } => format!("OSPF: update from {} on {}, {}\n", who(nbr.unwrap_or_default()), ifn(*iface), n(*lsas, "LSA", "LSAs")),
            Note::OspfAck { nbr, iface, out: true, lsas, delayed } => {
                let to = if *delayed { String::from("the group (delayed)") } else { who(nbr.unwrap_or_default()) };
                format!("OSPF: acknowledgement out of {} to {}, {}\n", ifn(*iface), to, n(*lsas, "LSA", "LSAs"))
            }
            Note::OspfAck { nbr, iface, lsas, .. } => format!("OSPF: acknowledgement from {} on {}, {}\n", who(nbr.unwrap_or_default()), ifn(*iface), n(*lsas, "LSA", "LSAs")),
            Note::OspfLsaNew { lsa, from } => format!("OSPF: took {} from {}\n", lsa.en(), from),
            Note::OspfLsaOrig { lsa, why } => format!("OSPF: own {} ({})\n", lsa.en(), why.en()),
            Note::OspfLsaFlooded { lsa, ifaces } => format!("OSPF: flooded {} out of {}\n", lsa.en(), n(*ifaces, "interface", "interfaces")),
            Note::OspfLsaAcked { lsa, nbr } => format!("OSPF: {} acknowledged {}\n", nbr, lsa.en()),
            Note::OspfLsaFlush { lsa } => format!("OSPF: withdrawing own {} with MaxAge\n", lsa.en()),
            Note::OspfLsaAged { lsa } => format!("OSPF: {} reached MaxAge\n", lsa.en()),
            Note::OspfLsaGone { lsa } => format!("OSPF: removed {} from the database\n", lsa.en()),
            Note::OspfLsaOlder { lsa, nbr } => format!("OSPF: {} sent an older {}, it gets the newer one\n", nbr, lsa.en()),
            Note::OspfLsaDropped { lsa, why } => format!("OSPF: dropped {}: {}\n", lsa.en(), why.en()),
            Note::OspfSpf { run, routes, added, changed, removed } => format!("OSPF: SPF run {}: {}, {} new, {} changed, {} gone\n", run, n(*routes, "route", "routes"), added, changed, removed),
            Note::OspfRoute { prefix, kind, cost, via, what } => match what {
                RtChange::Added => format!("OSPF: new {} route {} via {}, cost {}\n", kind.en(), prefix, who(*via), cost),
                RtChange::Changed => format!("OSPF: {} route {} now via {}, cost {}\n", kind.en(), prefix, who(*via), cost),
                RtChange::Removed => format!("OSPF: {} route {} gone\n", kind.en(), prefix),
            },
            _ => String::new(),
        }
    }
}
