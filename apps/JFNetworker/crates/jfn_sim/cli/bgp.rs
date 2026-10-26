// bgp -- BGP at the IOS-like command line: "router bgp ASN" with its own tree mode behind the
// prompt "(config-router)#", show ip bgp [A.B.C.D | summary | neighbors [X [routes |
// advertised-routes]]], the B routes of show ip route, the BGP part of show ip protocols, debug ip
// bgp [events | updates], clear ip bgp * | X [soft [in | out]], and the lines of the running
// configuration; help texts, messages, the texts above tables and the debug lines are our own
// words, the command words, configuration lines and the columns of the tables follow the
// originals
//
//   build_bgp(tree, modes, router)   the commands in an IOS command tree (ios.rs) where the device
//                                    routes; the router mode of BGP is a tree mode of its own
//                                    (MODE_KEY, as OSPF's)
//   Engine::ios_bgp(ni, parsed)      every BGP command
//   Engine::ios_bgp_config(ni)       the "router bgp" block of the running configuration (after
//                                    "router ospf")
//   Engine::bgp_route_age(ni, p)     how long a B route of show ip route is as it is
//   Engine::bgp_protocols(ni)        its part of show ip protocols
//   Engine::bgp_debug(ni, note)      "debug ip bgp": a decision of BGP as a console line
//   DB_EVENTS, DB_UPDATES            what "debug ip bgp events|updates" shows (CliDev::bgp_debug;
//                                    none set while D_BGP is on: both)
//
// The block lists what differs from the defaults in this order: bgp router-id, no bgp
// log-neighbor-changes, bgp default local-preference, timers bgp, network (classful networks
// without their mask, as IOS), redistribute, then per neighbour (by address) remote-as,
// description, shutdown, ebgp-multihop, update-source, next-hop-self, weight.  Route maps and
// prefix lists do not exist yet: nothing here filters or changes paths.  "router bgp" needs
// routing ("% IP routing not enabled" on an L3 switch without "ip routing"); one process per
// device.  Commands about a neighbour need its remote-as first.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};
use netpkt::bgp as wire;
use netpkt::{Ipv4, Prefix4};

use super::ios::K;
use super::{Mode, Proc, D_BGP};
use crate::action::Action;
use crate::bgp::{BgpSet, Cand, Dest, How, Peer, Redist, Soft, Src, State, AD_EBGP, AD_IBGP, MRAI_EBGP, MRAI_IBGP};
use crate::bgptrace::BNote;
use crate::kernel::{Engine, Ns, SEC};
use crate::trace::Note;

/// the key the tree knows BGP's router mode by (its prompt is "(config-router)#")
pub const MODE_KEY: &str = "(config-router)# bgp";

/// what "debug ip bgp events|updates" shows
pub const DB_EVENTS: u8 = 1;
pub const DB_UPDATES: u8 = 2;

/// "00:00:12": hours, minutes, seconds
fn hms(ns: Ns) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// the length of the classful network of an address
fn classful(a: Ipv4) -> u8 {
    match a.0[0] {
        0..=127 => 8,
        128..=191 => 16,
        192..=223 => 24,
        _ => 32,
    }
}

/// a network as the BGP table writes it: classful ones without their length
fn net_str(p: &Prefix4) -> String {
    if p.len == classful(p.addr) && p.len > 0 { format!("{}", p.addr) } else { format!("{}", p) }
}

/// the BGP commands in an IOS tree; `m`: user, privileged and global configuration mode
pub(crate) fn build_bgp(t: &mut Tree<K>, m: [ModeId; 3], router: bool) {
    let [user, prv, conf] = m;
    if !router {
        return;
    }
    for x in [user, prv] {
        t.add(x, "show ip bgp", "The BGP table: every path to every network", Some(K::ShowBgp), 0);
        t.add(x, "show ip bgp A.B.C.D=net", "The paths to one network in detail", Some(K::ShowBgp), 0);
        t.add(x, "show ip bgp summary", "One line per neighbour: state, messages, prefixes", Some(K::ShowBgpSum), 0);
        t.add(x, "show ip bgp neighbors", "The neighbours and their sessions in detail", Some(K::ShowBgpNbr), 0);
        t.add(x, "show ip bgp neighbors A.B.C.D=nbr", "One neighbour", Some(K::ShowBgpNbr), 0);
        t.add(x, "show ip bgp neighbors A.B.C.D=nbr routes=what", "The paths it sent that were taken", Some(K::ShowBgpNbr), 0);
        t.add(x, "show ip bgp neighbors A.B.C.D=nbr advertised-routes=what", "The networks sent to it", Some(K::ShowBgpNbr), 0);
        t.add(x, "show ip route bgp=only", "Only the routes BGP chose", Some(K::ShowIpRoute), 0);
    }
    t.add(prv, "debug ip bgp", "BGP sessions, messages and decisions", Some(K::DebugBgp), 0);
    t.add(prv, "undebug ip bgp", "BGP sessions, messages and decisions", Some(K::DebugBgp), 0);
    for (w, h) in [("events", "Sessions, connections, OPEN and NOTIFICATION, timers"), ("updates", "UPDATEs, the routes taken or dropped, the best paths")] {
        t.add(prv, &format!("debug ip bgp {}=what", w), h, Some(K::DebugBgp), 0);
        t.add(prv, &format!("undebug ip bgp {}=what", w), h, Some(K::DebugBgp), 0);
    }
    t.add(prv, "clear ip bgp", "Start BGP sessions anew", None, 0);
    for x in ["*=all", "A.B.C.D=nbr"] {
        let b = format!("clear ip bgp {}", x);
        t.add(prv, &b, if x.starts_with('*') { "Every neighbour" } else { "One neighbour" }, Some(K::ClearBgp), 0);
        t.add(prv, &format!("{} soft", b), "Keep the session: exchange the routes again", Some(K::ClearBgp), 0);
        t.add(prv, &format!("{} soft in=dir", b), "Ask the neighbour for its routes again", Some(K::ClearBgp), 0);
        t.add(prv, &format!("{} soft out=dir", b), "Send the own routes again", Some(K::ClearBgp), 0);
    }

    // global configuration ("router" itself is there, with its "no", from RIP)
    t.add(conf, "router", "Start a routing process", None, F_NO);
    t.add(conf, "router bgp", "BGP: paths between autonomous systems (RFC 4271)", None, 0);
    t.add(conf, "router bgp <1-4294967295>=asn", "The number of the own autonomous system", Some(K::RouterBgp), 0);

    // the router mode of BGP: like every configuration mode, then its own commands
    let r = t.mode(MODE_KEY, "Commands for this BGP process:", Some(conf), Some("Undo a setting or restore its default"));
    t.add(r, "do", "Run an EXEC command from here", None, 0);
    t.add(r, "do LINE=cmd", "The command", Some(K::Do), 0);
    t.add(r, "exit", "Back to the device configuration", Some(K::Exit), 0);
    t.add(r, "end", "Finish configuring", Some(K::End), F_HIDDEN);
    t.add(r, "bgp", "Settings of the whole process", None, F_NO);
    t.add(r, "bgp router-id", "The 32-bit name of this router in BGP", None, 0);
    t.add(r, "bgp router-id A.B.C.D=rid", "Written like an address", Some(K::BgpRouterId), 0);
    t.add_no(r, "bgp router-id", K::BgpRouterId);
    t.add(r, "bgp log-neighbor-changes", "Say on the console when a session comes or goes", Some(K::BgpLog), 0);
    t.add(r, "bgp default", "Values paths get when they bring none", None, 0);
    t.add(r, "bgp default local-preference", "The local preference inside the AS", None, 0);
    t.add(r, "bgp default local-preference <0-4294967295>=lp", "Higher wins (default 100)", Some(K::BgpLocalPref), 0);
    t.add_no(r, "bgp default local-preference", K::BgpLocalPref);
    t.add(r, "timers", "Timers of the protocol", None, F_NO);
    t.add(r, "timers bgp", "Keepalive interval and hold time of new sessions", None, 0);
    t.add(r, "timers bgp <0-65535>=ka", "Seconds between keepalives", None, 0);
    t.add(r, "timers bgp <0-65535>=ka <0-65535>=hold", "Seconds of silence until a session ends (0: never)", Some(K::BgpTimers), 0);
    t.add_no(r, "timers bgp", K::BgpTimers);
    t.add(r, "network", "Originate a network that is in the routing table", None, F_NO);
    t.add(r, "network A.B.C.D=net", "The network (classful without a mask)", Some(K::BgpNetwork), 0);
    t.add(r, "network A.B.C.D=net mask", "Its mask", None, 0);
    t.add(r, "network A.B.C.D=net mask MASK=mask", "The mask, as in the routing table", Some(K::BgpNetwork), 0);
    t.add(r, "redistribute", "Originate the routes of another source", None, F_NO);
    for (w, h) in [("connected", "Networks of the interfaces"), ("static", "Static routes"), ("rip", "Routes RIP learned")] {
        let b = format!("redistribute {}=what", w);
        t.add(r, &b, h, Some(K::BgpRedist), 0);
        t.add(r, &format!("{} metric", b), "The MED they get", None, 0);
        t.add(r, &format!("{} metric <0-4294967295>=metric", b), "Instead of their metric", Some(K::BgpRedist), 0);
    }
    t.add(r, "redistribute ospf=what", "Routes OSPF found", None, 0);
    t.add(r, "redistribute ospf=what <1-65535>=pid", "The OSPF process", Some(K::BgpRedist), 0);
    t.add(r, "redistribute ospf=what <1-65535>=pid metric", "The MED they get", None, 0);
    t.add(r, "redistribute ospf=what <1-65535>=pid metric <0-4294967295>=metric", "Instead of their metric", Some(K::BgpRedist), 0);
    let n = "neighbor A.B.C.D=nbr";
    t.add(r, "neighbor", "A neighbour and its settings", None, F_NO);
    t.add(r, n, "The neighbour's address", None, 0);
    t.add_no(r, n, K::BgpNbr);
    t.add(r, &format!("{} remote-as", n), "The neighbour's AS: the own one for iBGP, another for eBGP", None, 0);
    t.add(r, &format!("{} remote-as <1-4294967295>=asn", n), "AS number", Some(K::BgpNbr), 0);
    t.add(r, &format!("{} description", n), "A note about the neighbour", None, 0);
    t.add(r, &format!("{} description LINE=text", n), "The note, up to 80 characters", Some(K::BgpNbr), 0);
    t.add_no(r, &format!("{} description", n), K::BgpNbr);
    t.add(r, &format!("{} update-source", n), "Use the address of this interface for the session", None, 0);
    t.add(r, &format!("{} update-source NAME1=if", n), "", Some(K::BgpNbr), 0);
    t.add_no(r, &format!("{} update-source", n), K::BgpNbr);
    t.add(r, &format!("{} next-hop-self", n), "Send own address as next hop", Some(K::BgpNbr), 0);
    t.add(r, &format!("{} ebgp-multihop", n), "Allow an external neighbour that is not directly connected", Some(K::BgpNbr), 0);
    t.add(r, &format!("{} ebgp-multihop <1-255>=ttl", n), "The TTL of the session's packets (default 255)", Some(K::BgpNbr), 0);
    t.add(r, &format!("{} shutdown", n), "Keep the session down", Some(K::BgpNbr), 0);
    t.add(r, &format!("{} weight", n), "Preference of the neighbour's paths on this router", None, 0);
    t.add(r, &format!("{} weight <0-65535>=w", n), "Higher wins (default 0)", Some(K::BgpNbr), 0);
    t.add_no(r, &format!("{} weight", n), K::BgpNbr);
}

impl Engine {
    /// every BGP command
    pub(crate) fn ios_bgp(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let ip = |k: &str| p.ip(k).map(Ipv4);
        let set = match p.cmd {
            K::RouterBgp => {
                let asn = p.num("asn").unwrap_or(1) as u32;
                let cur = self.bgp_of(ni).filter(|b| b.cfg.on).map(|b| b.cfg.asn);
                if p.no {
                    if cur != Some(asn) {
                        return self.ios_err(ni, &format!("% There is no BGP process for AS {} here\n", asn));
                    }
                    let _ = self.apply(Action::Bgp { dev, set: BgpSet::Process { asn, on: false } });
                    return Ok(());
                }
                if !self.stack(ni).is_some_and(|s| s.forwarding) {
                    return self.ios_err(ni, "% IP routing not enabled\n");
                }
                if let Some(c) = cur.filter(|c| *c != asn) {
                    return self.ios_err(ni, &format!("% BGP is already running; AS is {}\n", c));
                }
                if let Err(e) = self.apply(Action::Bgp { dev, set: BgpSet::Process { asn, on: true } }) {
                    return self.ios_err(ni, &format!("% {}\n", e));
                }
                if let Some(c) = self.cdev_mut(ni) {
                    c.mode = Mode::Router(Proc::Bgp(asn));
                    c.ifs.clear();
                    c.vlans.clear();
                    c.pool.clear();
                }
                return Ok(());
            }
            K::ShowBgp | K::ShowBgpSum | K::ShowBgpNbr => {
                let text = match p.cmd {
                    K::ShowBgp => match ip("net") {
                        Some(a) => self.show_bgp_net(ni, a),
                        None => self.show_bgp(ni, None),
                    },
                    K::ShowBgpSum => self.show_bgp_sum(ni),
                    _ => match (ip("nbr"), p.text("what")) {
                        (Some(a), Some(w)) => {
                            if !self.bgp_of(ni).is_some_and(|b| b.cfg.nbrs.contains_key(&a)) {
                                return self.ios_err(ni, &format!("% No such neighbor or address family\n{}", ""));
                            }
                            self.show_bgp(ni, Some((a, w == "advertised-routes")))
                        }
                        (a, _) => self.show_bgp_nbrs(ni, a),
                    },
                };
                self.out(ni, &text);
                return Ok(());
            }
            K::DebugBgp => {
                let kind = match p.text("what") {
                    Some("events") => DB_EVENTS,
                    Some("updates") => DB_UPDATES,
                    _ => DB_EVENTS | DB_UPDATES,
                };
                let off = p.has("undebug") || p.no;
                let (before, kinds) = self.cdev(ni).map(|c| (c.debug, if c.debug & D_BGP != 0 { if c.bgp_debug == 0 { DB_EVENTS | DB_UPDATES } else { c.bgp_debug } } else { 0 })).unwrap_or((0, 0));
                let kinds = if off { kinds & !kind } else { kinds | kind };
                let after = if kinds != 0 { before | D_BGP } else { before & !D_BGP };
                if let Some(c) = self.cdev_mut(ni) {
                    c.debug = after;
                    c.bgp_debug = kinds;
                }
                if before == 0 && after != 0 {
                    self.cli.debug += 1;
                } else if before != 0 && after == 0 {
                    self.cli.debug = self.cli.debug.saturating_sub(1);
                }
                let what = match kind {
                    DB_EVENTS => "BGP events",
                    DB_UPDATES => "BGP updates",
                    _ => "BGP",
                };
                self.out(ni, &format!("Debug output for {}: {}\n", what, if off { "off" } else { "on" }));
                return Ok(());
            }
            K::ClearBgp => {
                if !self.bgp_on(ni) {
                    return self.ios_err(ni, "% No BGP process runs here\n");
                }
                let addr = ip("nbr");
                if let Some(a) = addr
                    && !self.bgp_of(ni).is_some_and(|b| b.cfg.nbrs.contains_key(&a))
                {
                    return self.ios_err(ni, &format!("% No such neighbor {}\n", a));
                }
                let soft = match (p.has("soft"), p.text("dir")) {
                    (false, _) => Soft::Hard,
                    (true, Some("in")) => Soft::In,
                    (true, Some("out")) => Soft::Out,
                    (true, _) => Soft::Both,
                };
                BgpSet::Clear { addr, soft }
            }
            K::BgpRouterId => {
                let rid = if p.no { None } else { ip("rid") };
                if let Some(a) = rid
                    && (a.is_unspecified() || a.is_multicast() || a.is_broadcast())
                {
                    return self.ios_err(ni, &format!("% {} cannot be a router ID\n", a));
                }
                BgpSet::RouterId(rid)
            }
            K::BgpLog => BgpSet::LogChanges(!p.no),
            K::BgpLocalPref => BgpSet::LocalPref(if p.no { None } else { p.num("lp").map(|v| v as u32) }),
            K::BgpTimers => {
                if p.no {
                    BgpSet::Timers(None)
                } else {
                    let (k, h) = (p.num("ka").unwrap_or(60) as u16, p.num("hold").unwrap_or(180) as u16);
                    if h != 0 && h < 3 {
                        return self.ios_err(ni, "% Hold time must be 0 or at least 3 seconds\n");
                    }
                    if !self.cdev(ni).is_some_and(|c| c.booting) && self.bgp_of(ni).is_some_and(|b| !b.peers().is_empty()) {
                        self.out(ni, "The new timers count for sessions that start from now on (clear ip bgp starts them anew).\n");
                    }
                    BgpSet::Timers(Some((k, h)))
                }
            }
            K::BgpNetwork => {
                let a = ip("net").unwrap_or_default();
                let len = match p.mask("mask") {
                    Some(l) => l,
                    None => classful(a),
                };
                let net = Prefix4 { addr: a, len };
                if net.trunc() != net {
                    return self.ios_err(ni, &format!("% {} is not the network of mask {}\n", a, net.mask()));
                }
                BgpSet::Network { net, add: !p.no }
            }
            K::BgpRedist => {
                let what = match p.text("what") {
                    Some("static") => How::Static,
                    Some("rip") => How::Rip,
                    Some("ospf") => How::Ospf,
                    _ => How::Connected,
                };
                let r = Redist { what, pid: p.num("pid").unwrap_or(0) as u16, metric: p.num("metric").map(|v| v as u32) };
                BgpSet::Redistribute { r, add: !p.no }
            }
            K::BgpNbr => {
                let addr = ip("nbr").unwrap_or_default();
                let known = self.bgp_of(ni).is_some_and(|b| b.cfg.nbrs.contains_key(&addr));
                if !p.has("remote-as") && !known {
                    if p.no {
                        return Ok(());
                    }
                    return self.ios_err(ni, "% Specify remote-as or peer-group commands first\n");
                }
                if p.has("remote-as") {
                    BgpSet::RemoteAs { addr, asn: if p.no { None } else { p.num("asn").map(|v| v as u32) } }
                } else if p.has("description") {
                    BgpSet::Descr { addr, text: if p.no { None } else { p.text("text").map(|t| String::from(&t[..t.len().min(80)])) } }
                } else if p.has("update-source") {
                    BgpSet::UpdateSource { addr, iface: if p.no { None } else { p.text("if").map(String::from) } }
                } else if p.has("next-hop-self") {
                    BgpSet::NextHopSelf { addr, on: !p.no }
                } else if p.has("ebgp-multihop") {
                    BgpSet::Multihop { addr, ttl: if p.no { None } else { Some(p.num("ttl").unwrap_or(255) as u8) } }
                } else if p.has("shutdown") {
                    BgpSet::Shutdown { addr, on: !p.no }
                } else if p.has("weight") {
                    BgpSet::Weight { addr, weight: if p.no { None } else { p.num("w").map(|v| v as u32) } }
                } else {
                    // "no neighbor X": the neighbour goes
                    BgpSet::RemoteAs { addr, asn: None }
                }
            }
            _ => return Ok(()),
        };
        match self.apply(Action::Bgp { dev, set }) {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    // ---------------------------------------------------------------- configuration

    /// the "router bgp" block of the running configuration ("" without one)
    pub(crate) fn ios_bgp_config(&self, ni: usize) -> String {
        let Some(c) = self.bgp_of(ni).map(|b| &b.cfg).filter(|c| c.on) else {
            return String::new();
        };
        let mut s = format!("router bgp {}\n", c.asn);
        if let Some(r) = c.router_id {
            s.push_str(&format!(" bgp router-id {}\n", r));
        }
        if !c.log_changes {
            s.push_str(" no bgp log-neighbor-changes\n");
        }
        if c.local_pref != crate::bgp::LOCAL_PREF {
            s.push_str(&format!(" bgp default local-preference {}\n", c.local_pref));
        }
        if (c.keepalive, c.hold) != (crate::bgp::KEEPALIVE, crate::bgp::HOLD) {
            s.push_str(&format!(" timers bgp {} {}\n", c.keepalive, c.hold));
        }
        for n in &c.networks {
            if n.len == classful(n.addr) {
                s.push_str(&format!(" network {}\n", n.addr));
            } else {
                s.push_str(&format!(" network {} mask {}\n", n.addr, n.mask()));
            }
        }
        for r in &c.redistribute {
            let mut l = match r.what {
                How::Ospf => format!(" redistribute ospf {}", r.pid),
                h => format!(" {}", h.word()),
            };
            if let Some(m) = r.metric {
                l.push_str(&format!(" metric {}", m));
            }
            s.push_str(&l);
            s.push('\n');
        }
        for (a, n) in &c.nbrs {
            s.push_str(&format!(" neighbor {} remote-as {}\n", a, n.remote_as));
            if !n.descr.is_empty() {
                s.push_str(&format!(" neighbor {} description {}\n", a, n.descr));
            }
            if n.shutdown {
                s.push_str(&format!(" neighbor {} shutdown\n", a));
            }
            match n.multihop {
                Some(255) => s.push_str(&format!(" neighbor {} ebgp-multihop\n", a)),
                Some(t) => s.push_str(&format!(" neighbor {} ebgp-multihop {}\n", a, t)),
                None => {}
            }
            if let Some(u) = &n.update_source {
                s.push_str(&format!(" neighbor {} update-source {}\n", a, u));
            }
            if n.next_hop_self {
                s.push_str(&format!(" neighbor {} next-hop-self\n", a));
            }
            if let Some(w) = n.weight {
                s.push_str(&format!(" neighbor {} weight {}\n", a, w));
            }
        }
        s.push_str("!\n");
        s
    }

    // ---------------------------------------------------------------- show

    /// the B route's age in show ip route
    pub(crate) fn bgp_route_age(&self, ni: usize, p: Prefix4) -> Ns {
        self.bgp_of(ni).and_then(|b| b.loc_rib().get(&p)).map(|d| self.now.saturating_sub(d.since)).unwrap_or(0)
    }

    /// a path's row of the BGP table: status, network (only on the first row), next hop,
    /// metric, local preference, weight, path
    fn bgp_row(&self, ni: usize, p: &Prefix4, c: &Cand, best: bool, first: bool) -> String {
        // a better route of another source keeps the path out of the forwarding (RIB-failure)
        let st = self.stack(ni);
        let ad = if c.path.ebgp { AD_EBGP } else { AD_IBGP };
        let fail = best
            && matches!(c.path.from, Src::Peer(_))
            && st.is_some_and(|s| s.rib(&|i| self.iface_up(ni, i)).iter().any(|r| r.prefix == *p && r.src != crate::ip::RouteSrc::Bgp && r.ad < ad));
        let valid = if fail {
            'r'
        } else if c.ok {
            '*'
        } else {
            ' '
        };
        let status = format!(" {}{}{} ", valid, if best { '>' } else { ' ' }, if !c.path.ebgp && matches!(c.path.from, Src::Peer(_)) { 'i' } else { ' ' });
        let net = if first { net_str(p) } else { String::new() };
        let a = &c.path.attrs;
        let metric = a.med.map(|m| format!("{}", m)).unwrap_or_default();
        let lp = a.local_pref.map(|m| format!("{}", m)).unwrap_or_default();
        let path = wire::path_text(&a.as_path);
        let tail = if path.is_empty() { String::from(wire::origin_code(a.origin)) } else { format!("{} {}", path, wire::origin_code(a.origin)) };
        let mut l = format!("{}{:<17}", status, net);
        if net.len() > 16 {
            // a long network has a line of its own
            l = format!("{}{}\n{}{:<17}", status, net, " ".repeat(5), "");
        }
        format!("{}{:<19}{:>7}{:>7}{:>7} {}\n", l, format!("{}", a.next_hop), metric, lp, c.weight, tail)
    }

    /// show ip bgp; `only`: the paths of one neighbour (Adj-RIB-In taken) or those advertised to
    /// it (the table of the original)
    fn show_bgp(&self, ni: usize, only: Option<(Ipv4, bool)>) -> String {
        let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on) else {
            return String::new();
        };
        let Some(rid) = b.router_id() else {
            return format!("BGP in AS {}: no router ID yet (no interface with an address is up)\n", b.cfg.asn);
        };
        let mut s = format!("BGP table version {}, router ID {}, local AS {}\n", b.version(), rid, b.cfg.asn);
        s.push_str("Status: * usable, > best, i from inside the AS, r not used: a better route of another source\n");
        s.push_str("Origin: i IGP, e EGP, ? incomplete (redistributed)\n\n");
        s.push_str("     Network          Next Hop            Metric LocPrf Weight Path\n");
        let mut count = 0;
        for (p, d) in b.loc_rib() {
            let rows: Vec<(usize, &Cand)> = match only {
                None => {
                    let mut v: Vec<(usize, &Cand)> = d.cands.iter().enumerate().collect();
                    // the best one first
                    v.sort_by_key(|(i, _)| Some(*i) != d.best);
                    v
                }
                Some((a, false)) => d.cands.iter().enumerate().filter(|(_, c)| c.path.from == Src::Peer(a)).collect(),
                Some((a, true)) => {
                    if !b.peer(a).is_some_and(|x| x.adj_out.contains_key(p)) {
                        continue;
                    }
                    d.best.and_then(|i| d.cands.get(i).map(|c| (i, c))).into_iter().collect()
                }
            };
            for (k, (i, c)) in rows.iter().enumerate() {
                s.push_str(&self.bgp_row(ni, p, c, Some(*i) == d.best, k == 0));
                count += 1;
            }
        }
        if only.is_some() {
            s.push_str(&format!("\nNetworks listed: {}\n", count));
        }
        s
    }

    /// show ip bgp A.B.C.D: the paths to the network that holds the address
    fn show_bgp_net(&self, ni: usize, a: Ipv4) -> String {
        let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on) else {
            return String::new();
        };
        let found = b.loc_rib().iter().filter(|(p, _)| p.contains(a)).max_by_key(|(p, _)| p.len);
        let Some((p, d)) = found else {
            return String::from("% Network not in table\n");
        };
        let d: &Dest = d;
        let mut s = format!("Network {} in the BGP table, version {}\n", p, d.ver);
        match d.best() {
            Some(_) => s.push_str(&format!("  Paths: {}, the best is #{} ({})\n", d.cands.len(), d.best.unwrap_or(0) + 1, d.step.en())),
            None => s.push_str(&format!("  Paths: {}, none of them usable\n", d.cands.len())),
        }
        let adv: Vec<String> = b.peers().iter().filter(|(_, x)| x.adj_out.contains_key(p)).map(|(k, _)| format!("{}", k)).collect();
        if adv.is_empty() {
            s.push_str("  Advertised to: nobody\n");
        } else {
            s.push_str(&format!("  Advertised to: {}\n", adv.join(" ")));
        }
        for (i, c) in d.cands.iter().enumerate() {
            let at = &c.path.attrs;
            let path = wire::path_text(&at.as_path);
            let from = match c.path.from {
                Src::Local(h) => format!("originated here ({})", h.word()),
                Src::Peer(x) => format!("from {} (BGP identifier {}), {}", x, c.path.peer_id, if c.path.ebgp { "external" } else { "internal" }),
            };
            s.push_str(&format!("  #{}: AS path {}, {}\n", i + 1, if path.is_empty() { String::from("empty (this AS)") } else { path }, from));
            let mut l = format!("      next hop {}", at.next_hop);
            if matches!(c.path.from, Src::Peer(_)) {
                if c.ok {
                    l.push_str(&format!(" (IGP metric {})", c.igp));
                } else {
                    l.push_str(" (not reachable)");
                }
            }
            l.push_str(&format!(", origin {}", wire::origin_name(at.origin)));
            if let Some(m) = at.med {
                l.push_str(&format!(", MED {}", m));
            }
            l.push_str(&format!(", local preference {}, weight {}", c.lp, c.weight));
            if Some(i) == d.best {
                l.push_str(", best");
            } else if c.ok {
                l.push_str(", usable");
            }
            s.push_str(&l);
            s.push('\n');
            if !at.communities.is_empty() {
                let v: Vec<String> = at.communities.iter().map(|x| wire::community_text(*x)).collect();
                s.push_str(&format!("      communities {}\n", v.join(" ")));
            }
            s.push_str(&format!("      since {} ago\n", hms(self.now.saturating_sub(c.path.since))));
        }
        s
    }

    /// what a neighbour shows in the State/PfxRcd column
    fn bgp_state_col(b: &crate::bgp::Bgp, a: Ipv4, p: &Peer) -> String {
        match p.state() {
            State::Established => format!("{:>8}", p.adj_in.len()),
            State::Idle if b.cfg.nbrs.get(&a).is_some_and(|n| n.shutdown) => String::from("Idle (Admin)"),
            st => String::from(st.name()),
        }
    }

    /// show ip bgp summary (the table of the original)
    fn show_bgp_sum(&self, ni: usize) -> String {
        let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on) else {
            return String::new();
        };
        let now = self.now;
        let rid = b.router_id().map(|r| format!("{}", r)).unwrap_or_else(|| String::from("not chosen yet"));
        let nets = b.loc_rib().values().filter(|d| d.best.is_some()).count();
        let mut s = format!("Router ID {}, local AS {}\n", rid, b.cfg.asn);
        s.push_str(&format!("Table version {}; networks: {}, paths: {}\n\n", b.version(), nets, b.path_count()));
        if b.cfg.nbrs.is_empty() {
            return s;
        }
        s.push_str("Neighbor        V           AS MsgRcvd MsgSent   TblVer  InQ OutQ Up/Down  State/PfxRcd\n");
        for (a, p) in b.peers() {
            let asn = b.cfg.nbrs.get(a).map(|n| n.remote_as).unwrap_or(0);
            let ud = match p.since {
                Some(t) => hms(now.saturating_sub(t)),
                None => String::from("never"),
            };
            let outq = p.conns.iter().map(|c| c.st).filter(|_| false).count();
            s.push_str(&format!("{:<16}4{:>13}{:>8}{:>8}{:>9}{:>5}{:>5} {:<8} {}\n", format!("{}", a), asn, p.n.msgs_in, p.n.msgs_out, p.tbl_ver, 0, outq, ud, Self::bgp_state_col(b, *a, p)));
        }
        s
    }

    /// show ip bgp neighbors [X]: each session in a few lines of our own
    fn show_bgp_nbrs(&self, ni: usize, only: Option<Ipv4>) -> String {
        let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on) else {
            return String::new();
        };
        if let Some(a) = only
            && !b.cfg.nbrs.contains_key(&a)
        {
            return String::from("% No such neighbor\n");
        }
        let now = self.now;
        let mut s = String::new();
        for (a, p) in b.peers() {
            if only.is_some_and(|x| x != *a) {
                continue;
            }
            let Some(n) = b.cfg.nbrs.get(a) else {
                continue;
            };
            let ebgp = n.remote_as != b.cfg.asn;
            s.push_str(&format!("BGP neighbour {}, AS {}, {}", a, n.remote_as, if ebgp { "external (eBGP)" } else { "internal (iBGP)" }));
            if !n.descr.is_empty() {
                s.push_str(&format!(", \"{}\"", n.descr));
            }
            s.push('\n');
            let st = p.state();
            match (st, p.since) {
                (State::Established, Some(t)) => s.push_str(&format!("  State Established for {}, BGP identifier {}\n", hms(now.saturating_sub(t)), p.peer_id)),
                (st, Some(t)) => s.push_str(&format!("  State {}, down for {}\n", st.name(), hms(now.saturating_sub(t)))),
                (st, None) => s.push_str(&format!("  State {}, never up\n", st.name())),
            }
            if let Some(w) = p.idle.filter(|_| st == State::Idle) {
                s.push_str(&format!("  Idle: {}\n", w.en()));
            }
            if let Some(c) = p.session() {
                let (hold_at, ka_at) = p.timers();
                let mut l = format!("  Hold time {} s, keepalive every {} s (configured {} / {})", p.hold, p.keepalive, b.cfg.hold, b.cfg.keepalive);
                if let Some(t) = ka_at {
                    l.push_str(&format!("; next keepalive in {}", hms(t.saturating_sub(now))));
                }
                if let Some(t) = hold_at {
                    l.push_str(&format!(", hold timer runs out in {}", hms(t.saturating_sub(now))));
                }
                s.push_str(&l);
                s.push('\n');
                let mut caps: Vec<&str> = alloc::vec!["IPv4 unicast"];
                if p.as4 {
                    caps.push("4-octet AS numbers");
                }
                if p.refresh {
                    caps.push("route refresh");
                }
                s.push_str(&format!("  Both offered: {}\n", caps.join(", ")));
                let ttl = match n.multihop {
                    Some(t) if ebgp => format!("TTL {}", t),
                    _ if ebgp => String::from("TTL 1"),
                    _ => String::from("TTL 255"),
                };
                s.push_str(&format!("  Connection {}:{} to {}:{}, {}, opened by {}\n", p.local, p.lport, a, p.rport, ttl, if c.out { "this router" } else { "the neighbour" }));
            } else if matches!(st, State::Connect | State::Active) {
                let mut l = format!("  Connection attempts: {}", p.n.attempts);
                if let Some(t) = p.retry_at() {
                    l.push_str(&format!(", the next in {}", hms(t.saturating_sub(now))));
                }
                if let Some(f) = p.fail {
                    l.push_str(&format!(" (the last: {})", f.errno().unwrap_or("closed")));
                }
                s.push_str(&l);
                s.push('\n');
            }
            if let Some(u) = &n.update_source {
                s.push_str(&format!("  Update source {}\n", u));
            }
            if n.next_hop_self {
                s.push_str("  Next hop to this neighbour: this router (next-hop-self)\n");
            }
            if let Some(w) = n.weight {
                s.push_str(&format!("  Weight of its paths {}\n", w));
            }
            let c = &p.n;
            s.push_str("  Messages            sent  received\n");
            for (name, o, i) in [("Opens", c.opens_out, c.opens_in), ("Updates", c.updates_out, c.updates_in), ("Keepalives", c.keepalives_out, c.keepalives_in), ("Notifications", c.notifications_out, c.notifications_in), ("Route refresh", c.refresh_out, c.refresh_in), ("Total", c.msgs_out, c.msgs_in)] {
                s.push_str(&format!("    {:<14}{:>8}{:>10}\n", name, o, i));
            }
            s.push_str(&format!("  Networks: {} taken from it, {} sent to it\n", p.adj_in.len(), p.adj_out.len()));
            let mrai = if ebgp { MRAI_EBGP } else { MRAI_IBGP };
            if mrai > 0 {
                s.push_str(&format!("  New paths go to it at most every {} s\n", mrai / SEC));
            } else {
                s.push_str("  New paths go to it at once\n");
            }
            match p.last_down {
                Some((t, why)) => s.push_str(&format!("  Sessions: {} up, {} ended; the last ended {} ago: {}\n", c.established, c.dropped, hms(now.saturating_sub(t)), why.en())),
                None => s.push_str(&format!("  Sessions: {} up, {} ended\n", c.established, c.dropped)),
            }
            if let Some((t, sent, code, sub)) = p.last_error {
                s.push_str(&format!("  Last NOTIFICATION {} {} ago: {}/{} ({})\n", if sent { "sent" } else { "received" }, hms(now.saturating_sub(t)), code, sub, wire::error_name(code, sub)));
            }
            s.push('\n');
        }
        s
    }

    /// the BGP process in show ip protocols ("" without one)
    pub(crate) fn bgp_protocols(&self, ni: usize) -> String {
        let Some(b) = self.bgp_of(ni).filter(|b| b.cfg.on) else {
            return String::new();
        };
        let c = &b.cfg;
        let now = self.now;
        let rid = b.router_id().map(|r| format!("{}", r)).unwrap_or_else(|| String::from("not chosen yet"));
        let mut s = format!("Routing process \"bgp {}\" with router ID {} (RFC 4271)\n", c.asn, rid);
        s.push_str(&format!("  Timers: keepalive {} s, hold time {} s; distances: external {}, internal {}\n", c.keepalive, c.hold, AD_EBGP, AD_IBGP));
        if !c.networks.is_empty() {
            let v: Vec<String> = c.networks.iter().map(|n| format!("{}", n)).collect();
            s.push_str(&format!("  Networks: {}\n", v.join(", ")));
        }
        if !c.redistribute.is_empty() {
            let v: Vec<String> = c
                .redistribute
                .iter()
                .map(|r| match r.what {
                    How::Ospf => format!("ospf {}", r.pid),
                    h => String::from(&h.word()["redistribute ".len()..]),
                })
                .collect();
            s.push_str(&format!("  Redistributing: {}\n", v.join(", ")));
        }
        s.push_str("  Neighbours:\n    Address          AS          State        Up/Down\n");
        for (a, p) in b.peers() {
            let asn = c.nbrs.get(a).map(|n| n.remote_as).unwrap_or(0);
            let ud = match p.since {
                Some(t) => hms(now.saturating_sub(t)),
                None => String::from("never"),
            };
            s.push_str(&format!("    {:<17}{:<12}{:<13}{}\n", format!("{}", a), asn, p.state().name(), ud));
        }
        s
    }

    // ---------------------------------------------------------------- debug

    /// "debug ip bgp": a decision of BGP as a console line, if its kind is switched on
    pub(crate) fn bgp_debug(&self, ni: usize, note: &Note) -> String {
        let Note::Bgp(n) = note else {
            return String::new();
        };
        let kinds = match self.cdev(ni).map(|c| c.bgp_debug).unwrap_or(0) {
            0 => DB_EVENTS | DB_UPDATES,
            k => k,
        };
        let of = if n.is_update() {
            DB_UPDATES
        } else if n.is_event() {
            DB_EVENTS
        } else {
            0
        };
        if kinds & of == 0 {
            return String::new();
        }
        let e = |c: u8, s: u8| wire::error_name(c, s);
        match n {
            BNote::RouterId { id } => format!("BGP: router ID {}\n", id),
            BNote::State { peer, from, to } => format!("BGP: neighbour {} went from {} to {}\n", peer, from.name(), to.name()),
            BNote::Connect { peer, src: Some(s) } => format!("BGP: opening a connection to {} from {}\n", peer, s),
            BNote::Connect { peer, src: None } => format!("BGP: opening a connection to {}\n", peer),
            BNote::NoRoute { peer } => format!("BGP: no route to neighbour {}\n", peer),
            BNote::ConnFailed { peer, end } => format!("BGP: the connection to {} failed ({})\n", peer, end.errno().unwrap_or("closed")),
            BNote::Accepted { peer, local } => format!("BGP: took a connection from {} at {}\n", peer, local),
            BNote::Refused { from, why } => {
                let w = match why {
                    crate::bgp::RefuseWhy::Unknown => "no such neighbour",
                    crate::bgp::RefuseWhy::Shutdown => "the neighbour is shut down",
                    crate::bgp::RefuseWhy::Established => "its session is up already",
                };
                format!("BGP: reset a connection from {}: {}\n", from, w)
            }
            BNote::Idle { peer, why } => format!("BGP: neighbour {} stays idle: {}\n", peer, why.en()),
            BNote::OpenSent { peer, asn, hold, id } => format!("BGP: OPEN to {}: AS {}, hold time {}, ID {}\n", peer, asn, hold, id),
            BNote::OpenRcvd { peer, asn, hold, id } => format!("BGP: OPEN from {}: AS {}, hold time {}, ID {}\n", peer, asn, hold, id),
            BNote::Negotiated { peer, hold, keepalive } => format!("BGP: with {}: hold time {}, keepalive {}\n", peer, hold, keepalive),
            BNote::Keepalive { .. } => String::new(),
            BNote::Update { peer, out: true, adv, wd } => format!("BGP: UPDATE to {}: {} networks, {} withdrawn\n", peer, adv, wd),
            BNote::Update { peer, out: false, adv, wd } => format!("BGP: UPDATE from {}: {} networks, {} withdrawn\n", peer, adv, wd),
            BNote::Refresh { peer, out: true } => format!("BGP: ROUTE-REFRESH to {}\n", peer),
            BNote::Refresh { peer, out: false } => format!("BGP: ROUTE-REFRESH from {}\n", peer),
            BNote::Notification { peer, out: true, code, sub } => format!("BGP: NOTIFICATION to {}: {}/{} ({})\n", peer, code, sub, e(*code, *sub)),
            BNote::Notification { peer, out: false, code, sub } => format!("BGP: NOTIFICATION from {}: {}/{} ({})\n", peer, code, sub, e(*code, *sub)),
            BNote::Collision { peer, keep_ours } => format!("BGP: two connections with {}: keeping the one opened by {}\n", peer, if *keep_ours { "this router" } else { "the neighbour" }),
            BNote::HoldExpired { peer } => format!("BGP: nothing from {} within the hold time\n", peer),
            BNote::Down { peer, why } => format!("BGP: the session with {} ended: {}\n", peer, why.en()),
            BNote::Dropped { peer, prefix, why } => {
                let w = match why {
                    crate::bgp::DropWhy::Loop(a) => format!("own AS {} in the path", a),
                    crate::bgp::DropWhy::OwnNextHop(x) => format!("next hop {} is this router", x),
                    crate::bgp::DropWhy::BadNextHop(x) => format!("next hop {} is no unicast address", x),
                };
                format!("BGP: dropped {} from {}: {}\n", prefix, peer, w)
            }
            BNote::Best { prefix, via, from: Some(f), step } => format!("BGP: best path to {} via {} from {} ({})\n", prefix, via, f, step.en()),
            BNote::Best { prefix, from: None, step, .. } => format!("BGP: best path to {} is originated here ({})\n", prefix, step.en()),
            BNote::Lost { prefix } => format!("BGP: no path to {} any more\n", prefix),
            BNote::Local { prefix, how } => format!("BGP: originating {} ({})\n", prefix, how.word()),
            BNote::LocalGone { prefix } => format!("BGP: no longer originating {}\n", prefix),
            BNote::Mrai { peer, wait } => format!("BGP: new paths for {} wait {} ms\n", peer, wait / 1_000_000),
        }
    }
}
