// stp -- spanning tree at the IOS-like command line: "spanning-tree mode | priority | hello-time
// | forward-time | max-age" (also as "spanning-tree vlan LIST ...") in the global
// configuration, "spanning-tree cost | port-priority | portfast | bpduguard | link-type" on
// switch ports, "show spanning-tree [interface IF] [detail] | root | vlan LIST", "debug
// spanning-tree events", "clear spanning-tree detected-protocols", the lines of the running
// configuration and the debug lines; help texts and messages are our own words, the tables and
// configuration lines follow the originals
//
//   build_stp(tree, modes)               the commands in an IOS command tree (ios.rs; switches,
//                                        no access points)
//   Engine::ios_stp(ni, p)               the global and EXEC commands
//   Engine::ios_stp_if(ni, if, p, line)  a command of interface mode for one port
//   Engine::ios_stp_config(ni)           the global lines of the running configuration
//   Engine::ios_stp_if_config(ni, port)  the lines of an interface
//   Engine::stp_debug(ni, note)          a decision as a debug line (debug spanning-tree events)
//
// One tree per bridge (stp.rs): the per-VLAN spellings are taken for recognition and set the
// bridge's tree (the VLAN list must parse); the running configuration writes the global forms.
// "pvst" and "rapid-pvst" run IEEE 802.1D and 802.1w, and show spanning-tree says so.

use alloc::format;
use alloc::string::String;

use base::cmdtree::{ModeId, Parsed, Tree, F_NO};
use jfn_model::ifname;

use super::ios::K;
use super::D_STP;
use crate::action::{ActErr, Action};
use crate::bridge::VlanSet;
use crate::kernel::{Engine, SEC};
use crate::stp::{LinkType, ModeWord, PortRole, Proto, StpPortView, StpSet, TcWhat, TreeView, PORT_PRIORITY};
use crate::trace::{stp_id, Note};

/// the commands of spanning tree in an IOS tree; `m`: user, privileged, global configuration
/// and interface mode
pub(crate) fn build_stp(t: &mut Tree<K>, m: [ModeId; 4]) {
    let [user, prv, conf, ifm] = m;
    for x in [user, prv] {
        t.add(x, "show spanning-tree", "The spanning tree: root, roles and states of the ports", Some(K::ShowStp), 0);
        t.add(x, "show spanning-tree detail", "Every port with its timers and counters", Some(K::ShowStp), 0);
        t.add(x, "show spanning-tree interface", "One port", None, 0);
        t.add(x, "show spanning-tree interface NAME1=if", "", Some(K::ShowStp), 0);
        t.add(x, "show spanning-tree interface NAME1=if detail", "With its timers and counters", Some(K::ShowStp), 0);
        t.add(x, "show spanning-tree root", "The root bridge and the way to it", Some(K::ShowStp), 0);
        t.add(x, "show spanning-tree vlan", "Taken for recognition: every VLAN shares the one tree", None, 0);
        t.add(x, "show spanning-tree vlan WORD=vlans", "VLAN list such as 1,10-20", Some(K::ShowStp), 0);
    }
    t.add(prv, "debug spanning-tree", "Decisions of the spanning tree", None, 0);
    t.add(prv, "debug spanning-tree events", "Roles, states, root and topology changes", Some(K::DebugStp), 0);
    t.add(prv, "undebug spanning-tree", "Decisions of the spanning tree", None, 0);
    t.add(prv, "undebug spanning-tree events", "Roles, states, root and topology changes", Some(K::DebugStp), 0);
    t.add(prv, "clear spanning-tree", "State of the spanning tree", None, 0);
    t.add(prv, "clear spanning-tree detected-protocols", "Try RSTP again where a neighbour spoke 802.1D", Some(K::ClearStp), 0);
    t.add(prv, "clear spanning-tree detected-protocols interface", "Only on one port", None, 0);
    t.add(prv, "clear spanning-tree detected-protocols interface NAME1=if", "", Some(K::ClearStp), 0);

    // global configuration
    t.add(conf, "spanning-tree", "Settings of the spanning tree", None, F_NO);
    t.add(conf, "spanning-tree mode", "Which spanning tree protocol runs (off without it)", None, 0);
    t.add(conf, "spanning-tree mode stp=mode", "IEEE 802.1D, the classic spanning tree", Some(K::StpMode), 0);
    t.add(conf, "spanning-tree mode rstp=mode", "IEEE 802.1w, the rapid spanning tree", Some(K::StpMode), 0);
    t.add(conf, "spanning-tree mode pvst=mode", "Taken for recognition: IEEE 802.1D, one tree for all VLANs", Some(K::StpMode), 0);
    t.add(conf, "spanning-tree mode rapid-pvst=mode", "Taken for recognition: IEEE 802.1w, one tree for all VLANs", Some(K::StpMode), 0);
    t.add_no(conf, "spanning-tree mode", K::StpMode);
    for pre in ["spanning-tree", "spanning-tree vlan WORD=vlans"] {
        t.add(conf, &format!("{} priority", pre), "Bridge priority: the lowest becomes the root", None, 0);
        t.add(conf, &format!("{} priority <0-61440>=prio", pre), "In steps of 4096 (32768 if not set)", Some(K::StpPriority), 0);
        t.add_no(conf, &format!("{} priority", pre), K::StpPriority);
        for (kw, h, range, h2) in [
            ("hello-time", "How often the root sends its BPDUs", "<1-10>", "Seconds (2 if not set)"),
            ("forward-time", "How long a port listens and how long it learns", "<4-30>", "Seconds (15 if not set)"),
            ("max-age", "How long the information of a BPDU lasts", "<6-40>", "Seconds (20 if not set)"),
        ] {
            t.add(conf, &format!("{} {}", pre, kw), h, None, 0);
            t.add(conf, &format!("{} {} {}=secs", pre, kw, range), h2, Some(K::StpTimer), 0);
            t.add_no(conf, &format!("{} {}", pre, kw), K::StpTimer);
        }
    }
    t.add(conf, "spanning-tree vlan", "Taken for recognition: every VLAN shares the bridge's one tree", None, 0);
    t.add(conf, "spanning-tree vlan WORD=vlans", "VLAN list such as 1,10-20", None, 0);

    // interfaces (switch ports)
    t.add(ifm, "spanning-tree", "Spanning tree settings of this port", None, F_NO);
    t.add(ifm, "spanning-tree cost", "Path cost of this port (lower is preferred)", None, 0);
    t.add(ifm, "spanning-tree cost <1-200000000>=cost", "The cost (by the speed of the link if not set)", Some(K::StpIf), 0);
    t.add_no(ifm, "spanning-tree cost", K::StpIf);
    t.add(ifm, "spanning-tree port-priority", "Priority of this port among ports of equal cost", None, 0);
    t.add(ifm, "spanning-tree port-priority <0-240>=prio", "In steps of 16, lower wins (128 if not set)", Some(K::StpIf), 0);
    t.add_no(ifm, "spanning-tree port-priority", K::StpIf);
    t.add(ifm, "spanning-tree portfast", "Edge port for a host: forwards at once", Some(K::StpIf), 0);
    t.add(ifm, "spanning-tree portfast disable", "Not an edge port", Some(K::StpIf), 0);
    t.add(ifm, "spanning-tree bpduguard", "Shut the port off when a BPDU arrives", None, 0);
    t.add(ifm, "spanning-tree bpduguard enable", "On", Some(K::StpIf), 0);
    t.add(ifm, "spanning-tree bpduguard disable", "Off", Some(K::StpIf), 0);
    t.add_no(ifm, "spanning-tree bpduguard", K::StpIf);
    t.add(ifm, "spanning-tree link-type", "One neighbour or a shared LAN (found from the cable if not set)", None, 0);
    t.add(ifm, "spanning-tree link-type point-to-point=lt", "One bridge at the other end: rapid agreement", Some(K::StpIf), 0);
    t.add(ifm, "spanning-tree link-type shared=lt", "Several bridges on the LAN: timers only", Some(K::StpIf), 0);
    t.add_no(ifm, "spanning-tree link-type", K::StpIf);
}

/// "Gi1/0/1"
fn short(n: &str) -> String {
    ifname::short(n)
}

/// seconds as "00:01:23"
fn hms(ns: u64) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// the first line of show spanning-tree: the protocol in our words
fn proto_line(v: &TreeView) -> String {
    let (std, name) = match v.proto {
        Proto::Stp => ("IEEE 802.1D", "STP"),
        Proto::Rstp => ("IEEE 802.1w", "RSTP"),
    };
    let note = match v.mode {
        ModeWord::Pvst | ModeWord::RapidPvst => format!(" (\"{}\" runs as {})", v.mode.word(), name),
        _ => String::new(),
    };
    format!("Spanning tree: {} ({}), one tree for every VLAN{}\n", std, name, note)
}

/// the column "Type" of the port table: "P2p", "Shr", with " Edge" or " Peer(STP)"
fn port_type(p: &StpPortView, proto: Proto) -> String {
    let mut s = String::from(if p.p2p { "P2p" } else { "Shr" });
    if p.edge {
        s.push_str(" Edge");
    }
    if proto == Proto::Rstp && p.stp_peer {
        s.push_str(" Peer(STP)");
    }
    s
}

impl Engine {
    fn stp_port_name(&self, ni: usize, p: u16) -> String {
        match self.nodes.get(ni) {
            Some(Some(n)) => String::from(n.port_name(p)),
            _ => format!("{}", p),
        }
    }

    /// the global and EXEC commands of spanning tree
    pub(crate) fn ios_stp(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let r = match p.cmd {
            K::ShowStp => {
                if let Some(v) = p.text("vlans")
                    && VlanSet::parse(v).is_none()
                {
                    return self.ios_err(ni, "% Bad VLAN list: use numbers and ranges such as 10,20-30\n");
                }
                let text = self.show_stp(ni, p);
                self.out(ni, &text);
                Ok(())
            }
            K::DebugStp => {
                let off = p.no || p.has("undebug");
                self.stp_debug_set(ni, !off);
                self.out(ni, &format!("Debug output for spanning tree events: {}\n", if off { "off" } else { "on" }));
                Ok(())
            }
            K::ClearStp => {
                let port = p.text("if").map(String::from);
                self.apply(Action::Stp { dev, set: StpSet::Mcheck { port } })
            }
            K::StpMode => {
                let m = if p.no { None } else { p.text("mode").and_then(ModeWord::parse) };
                self.apply(Action::Stp { dev, set: StpSet::Mode(m) })
            }
            K::StpPriority => {
                if let Some(v) = p.text("vlans")
                    && VlanSet::parse(v).is_none()
                {
                    return self.ios_err(ni, "% Bad VLAN list: use numbers and ranges such as 10,20-30\n");
                }
                let prio = if p.no { None } else { p.num("prio").map(|n| n as u16) };
                if prio.is_some_and(|x| x % 4096 != 0) {
                    return self.ios_err(ni, "% Bridge Priority must be in increments of 4096.\n% Allowed are 0, 4096, 8192 ... 61440.\n");
                }
                self.apply(Action::Stp { dev, set: StpSet::Priority(prio) })
            }
            K::StpTimer => {
                if let Some(v) = p.text("vlans")
                    && VlanSet::parse(v).is_none()
                {
                    return self.ios_err(ni, "% Bad VLAN list: use numbers and ranges such as 10,20-30\n");
                }
                let v = if p.no { None } else { p.num("secs").map(|n| n as u16) };
                let set = if p.has("hello-time") {
                    StpSet::Hello(v)
                } else if p.has("forward-time") {
                    StpSet::FwdDelay(v)
                } else {
                    StpSet::MaxAge(v)
                };
                match self.apply(Action::Stp { dev, set }) {
                    Err(ActErr::Refused(_)) => return self.ios_err(ni, "% The timers do not fit together: 2 x (forward-time - 1) >= max-age >= 2 x (hello-time + 1)\n"),
                    r => r,
                }
            }
            _ => Ok(()),
        };
        match r {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    /// a command of interface mode for one interface
    pub(crate) fn ios_stp_if(&mut self, ni: usize, name: &str, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let port = if name.contains('.') { None } else { self.port_index(dev, name) };
        let switch_port = port.is_some_and(|pi| self.bridge(dev).is_some_and(|b| b.ports.get(pi as usize).is_some_and(|bp| bp.member)));
        if !switch_port {
            // spanning tree is a matter of switch ports
            let at = line.len() - line.trim_start().len();
            return self.ios_marker(ni, line, at);
        }
        let port = String::from(name);
        let set = if p.has("cost") {
            StpSet::Cost { port, cost: if p.no { None } else { p.num("cost").map(|c| c as u32) } }
        } else if p.has("port-priority") {
            let prio = if p.no { None } else { p.num("prio").map(|n| n as u8) };
            if prio.is_some_and(|x| x % 16 != 0) {
                return self.ios_err(ni, "% Port Priority in increments of 16 is required\n");
            }
            StpSet::PortPriority { port, priority: prio }
        } else if p.has("portfast") {
            let on = !p.no && !p.has("disable");
            if on && !self.booting_ni(ni) {
                self.out(ni, "% Note: an edge port forwards at once. Connect hosts only: a bridge behind it makes a loop until its first BPDU arrives.\n");
            }
            StpSet::Edge { port, on }
        } else if p.has("bpduguard") {
            StpSet::Guard { port, on: !p.no && p.has("enable") }
        } else {
            let kind = match (p.no, p.text("lt")) {
                (true, _) => LinkType::Auto,
                (false, Some("point-to-point")) => LinkType::P2p,
                _ => LinkType::Shared,
            };
            StpSet::Link { port, kind }
        };
        match self.apply(Action::Stp { dev, set }) {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    fn booting_ni(&self, ni: usize) -> bool {
        self.cdev(ni).is_some_and(|c| c.booting)
    }

    /// "debug spanning-tree events" on or off
    fn stp_debug_set(&mut self, ni: usize, on: bool) {
        let before = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
        let after = if on { before | D_STP } else { before & !D_STP };
        if let Some(c) = self.cdev_mut(ni) {
            c.debug = after;
        }
        if before == 0 && after != 0 {
            self.cli.debug += 1;
        } else if before != 0 && after == 0 {
            self.cli.debug = self.cli.debug.saturating_sub(1);
        }
    }

    // ---------------------------------------------------------------- show

    fn show_stp(&self, ni: usize, p: &Parsed<K>) -> String {
        let dev = self.dev_of(ni);
        let Some(v) = self.stp_view(dev) else {
            return String::from("Spanning tree is off on this switch; \"spanning-tree mode rstp\" (or stp) turns it on.\n");
        };
        if p.has("root") {
            return self.show_stp_root(ni, &v);
        }
        if let Some(name) = p.text("if") {
            let Some(pi) = self.port_index(dev, name) else {
                return String::new();
            };
            let Some(pv) = v.port(pi) else {
                return format!("No spanning tree on {}: the port is down or no switch port.\n", name);
            };
            if p.has("detail") {
                return self.stp_port_detail(ni, &v, pv);
            }
            let mut s = String::from("Tree                Role Sts Cost      Prio.Nbr Type\n------------------- ---- --- --------- -------- --------------------------------\n");
            s.push_str(&format!("{:<19} {:<4} {:<3} {:<9} {:<8} {}\n", "all VLANs", pv.role.short(), pv.state.short(), pv.cost, format!("{}", pv.id), port_type(pv, v.proto)));
            return s;
        }
        if p.has("detail") {
            let mut s = self.stp_bridge_detail(ni, &v);
            for pv in &v.ports {
                s.push('\n');
                s.push_str(&self.stp_port_detail(ni, &v, pv));
            }
            return s;
        }
        let mut s = proto_line(&v);
        let mac = |id: netpkt::stp::BridgeId| id.mac().cisco();
        s.push_str(&format!("  Root ID    Priority    {}\n             Address     {}\n", v.root.priority(), mac(v.root)));
        match v.root_port {
            Some(rp) if !v.is_root() => {
                let id = v.port(rp).map(|x| x.id.number()).unwrap_or(rp + 1);
                s.push_str(&format!("             Cost        {}\n             Port        {} ({})\n", v.root_cost, id, self.stp_port_name(ni, rp)));
            }
            _ => s.push_str("             This bridge itself is the root\n"),
        }
        s.push_str(&format!("             Hello Time  {:>2} sec  Max Age {:>2} sec  Forward Delay {:>2} sec\n\n", v.hello, v.max_age, v.fwd_delay));
        s.push_str(&format!("  Bridge ID  Priority    {}  (priority {} sys-id-ext {})\n             Address     {}\n", v.bridge.priority(), v.bridge.priority() & 0xF000, v.bridge.ext(), mac(v.bridge)));
        s.push_str(&format!("             Hello Time  {:>2} sec  Max Age {:>2} sec  Forward Delay {:>2} sec\n", v.b_hello, v.b_max_age, v.b_fwd_delay));
        let aging = self.bridge(dev).map(|b| b.age_limit() / SEC).unwrap_or(300);
        s.push_str(&format!("             Aging Time  {} sec\n\n", aging));
        s.push_str("Interface           Role Sts Cost      Prio.Nbr Type\n------------------- ---- --- --------- -------- --------------------------------\n");
        for pv in &v.ports {
            s.push_str(&format!("{:<19} {:<4} {:<3} {:<9} {:<8} {}\n", short(&self.stp_port_name(ni, pv.port)), pv.role.short(), pv.state.short(), pv.cost, format!("{}", pv.id), port_type(pv, v.proto)));
        }
        s
    }

    fn show_stp_root(&self, ni: usize, v: &TreeView) -> String {
        let row = |a: &str, b: &str, c: &str, d: &str, e: &str, f: &str, g: &str| format!("{}\n", format!("{:<16} {:<20} {:>9} {:>5} {:>3} {:>3}  {}", a, b, c, d, e, f, g).trim_end());
        let mut s = row("", "", "Root", "Hello", "Max", "Fwd", "");
        s.push_str(&row("Tree", "Root ID", "Cost", "Time", "Age", "Dly", "Root Port"));
        s.push_str("---------------- -------------------- --------- ----- --- ---  ------------\n");
        let port = match v.root_port {
            Some(rp) if !v.is_root() => short(&self.stp_port_name(ni, rp)),
            _ => String::new(),
        };
        let id = format!("{} {}", v.root.priority(), v.root.mac().cisco());
        s.push_str(&row("all VLANs", &id, &format!("{}", v.root_cost), &format!("{}", v.hello), &format!("{}", v.max_age), &format!("{}", v.fwd_delay), &port));
        s
    }

    fn stp_bridge_detail(&self, ni: usize, v: &TreeView) -> String {
        let mut s = proto_line(v);
        s.push_str(&format!("  Bridge {}, priority {}\n", stp_id(v.bridge), v.bridge.priority()));
        match v.root_port {
            Some(rp) if !v.is_root() => s.push_str(&format!("  Root {} at cost {} over {}\n", stp_id(v.root), v.root_cost, self.stp_port_name(ni, rp))),
            _ => s.push_str("  This bridge itself is the root\n"),
        }
        s.push_str(&format!("  Times in use: hello {}, max age {}, forward delay {}; set here: hello {}, max age {}, forward delay {}\n", v.hello, v.max_age, v.fwd_delay, v.b_hello, v.b_max_age, v.b_fwd_delay));
        match v.tc_last {
            Some(t) => s.push_str(&format!("  Topology changes: {}, the last {} ago{}\n", v.tc_count, hms(self.now.saturating_sub(t)), if v.tc { ", still going on" } else { "" })),
            None => s.push_str("  Topology changes: none yet\n"),
        }
        s
    }

    fn stp_port_detail(&self, ni: usize, v: &TreeView, p: &StpPortView) -> String {
        let dev = self.dev_of(ni);
        let name = self.stp_port_name(ni, p.port);
        let role = match p.role {
            PortRole::Root => "the root port",
            PortRole::Designated => "a designated port",
            PortRole::Alternate => "an alternate port",
            PortRole::Backup => "a backup port",
            PortRole::Disabled => "disabled",
        };
        let cfg = self.bridge(dev).map(|b| b.stp.cfg.port(p.port)).unwrap_or_default();
        let link = match cfg.link {
            LinkType::Auto => "(found from the cable)",
            _ => "(set)",
        };
        let mut s = format!("{} (port {}) is {}, {}\n", name, p.id, role, p.state.word());
        s.push_str(&format!("   Path cost {}{}, port priority {}, link type {} {}\n", p.cost, if cfg.cost.is_some() { " (set)" } else { "" }, p.id.priority(), if p.p2p { "point-to-point" } else { "shared" }, link));
        if p.edge || cfg.edge {
            s.push_str(&format!("   Edge port: {}\n", if p.edge { "yes" } else { "set, but a BPDU arrived" }));
        }
        if cfg.guard {
            s.push_str("   BPDU guard is on\n");
        }
        if v.proto == Proto::Rstp && p.stp_peer {
            s.push_str("   Its neighbour speaks 802.1D: the port sends Configuration BPDUs\n");
        }
        s.push_str(&format!("   Designated root {}, root path cost {}\n", stp_id(p.d_root), p.d_cost));
        s.push_str(&format!("   Designated bridge {}, port {}\n", stp_id(p.d_bridge), p.d_port));
        let info = match v.proto {
            Proto::Stp => format!("information age {}", p.info_age),
            Proto::Rstp => format!("information left {}", p.info_age),
        };
        s.push_str(&format!("   Timers: forward delay {}, {}\n", p.fwd_timer, info));
        s.push_str(&format!("   To forwarding {} times; BPDUs sent {}, received {}\n", p.to_fwd, p.tx, p.rx));
        s
    }

    // ---------------------------------------------------------------- running configuration

    /// the global lines (before the VLANs)
    pub(crate) fn ios_stp_config(&self, ni: usize) -> String {
        let Some(b) = self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()) else {
            return String::new();
        };
        let c = &b.stp.cfg;
        let mut s = String::new();
        if let Some(m) = c.mode {
            s.push_str(&format!("spanning-tree mode {}\n", m.word()));
        }
        if c.priority != crate::stp::PRIORITY {
            s.push_str(&format!("spanning-tree priority {}\n", c.priority));
        }
        if c.hello != crate::stp::HELLO {
            s.push_str(&format!("spanning-tree hello-time {}\n", c.hello));
        }
        if c.fwd_delay != crate::stp::FWD_DELAY {
            s.push_str(&format!("spanning-tree forward-time {}\n", c.fwd_delay));
        }
        if c.max_age != crate::stp::MAX_AGE {
            s.push_str(&format!("spanning-tree max-age {}\n", c.max_age));
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// the lines of a switch port
    pub(crate) fn ios_stp_if_config(&self, ni: usize, port: u16) -> String {
        let Some(b) = self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()) else {
            return String::new();
        };
        if !b.ports.get(port as usize).is_some_and(|bp| bp.member) {
            return String::new();
        }
        let c = b.stp.cfg.port(port);
        let mut s = String::new();
        if c.edge {
            s.push_str(" spanning-tree portfast\n");
        }
        if c.guard {
            s.push_str(" spanning-tree bpduguard enable\n");
        }
        match c.link {
            LinkType::P2p => s.push_str(" spanning-tree link-type point-to-point\n"),
            LinkType::Shared => s.push_str(" spanning-tree link-type shared\n"),
            LinkType::Auto => {}
        }
        if let Some(cost) = c.cost {
            s.push_str(&format!(" spanning-tree cost {}\n", cost));
        }
        if c.priority != PORT_PRIORITY {
            s.push_str(&format!(" spanning-tree port-priority {}\n", c.priority));
        }
        s
    }

    // ---------------------------------------------------------------- debug

    /// a decision of the spanning tree as a debug line (without the time stamp)
    pub(crate) fn stp_debug(&self, ni: usize, n: &Note) -> String {
        let pn = |p: u16| short(&self.stp_port_name(ni, p));
        match n {
            Note::StpRole { port, from, to } => format!("STP: {} is now {} (was {})\n", pn(*port), to.word(), from.word()),
            Note::StpState { port, from, to } => format!("STP: {} goes from {} to {}\n", pn(*port), from.word(), to.word()),
            Note::StpRoot { root, port: Some(p), cost } => format!("STP: the root is {}, over {} at cost {}\n", stp_id(*root), pn(*p), cost),
            Note::StpRoot { root, port: None, .. } => format!("STP: this bridge is the root ({})\n", stp_id(*root)),
            Note::StpTopology { port, what } => {
                let on = port.map(|p| format!(" on {}", pn(p))).unwrap_or_default();
                match what {
                    TcWhat::Detected => format!("STP: topology change{}\n", on),
                    TcWhat::TcnSent => format!("STP: TCN to the root{}\n", on),
                    TcWhat::TcnReceived => format!("STP: TCN received{}\n", on),
                    TcWhat::Acked => format!("STP: TCN acknowledged{}\n", on),
                    TcWhat::Received => format!("STP: topology change flag received{}\n", on),
                    TcWhat::RootFlag(true) => String::from("STP: as the root, sending the topology change flag\n"),
                    TcWhat::RootFlag(false) => String::from("STP: as the root, the topology change flag ends\n"),
                }
            }
            Note::StpFlush { port, entries: 1 } => format!("STP: 1 MAC entry of {} forgotten\n", pn(*port)),
            Note::StpFlush { port, entries } => format!("STP: {} MAC entries of {} forgotten\n", entries, pn(*port)),
            Note::StpAging { secs, fast } => format!("STP: MAC entries age after {} s{}\n", secs, if *fast { " during the topology change" } else { " again" }),
            Note::StpAged { port } => format!("STP: the information on {} aged out\n", pn(*port)),
            Note::StpEdge { port, edge: true } => format!("STP: {} is an edge port\n", pn(*port)),
            Note::StpEdge { port, edge: false } => format!("STP: {} heard a BPDU and is no edge port now\n", pn(*port)),
            Note::StpMigrate { port, rstp: false } => format!("STP: {} speaks 802.1D with its neighbour\n", pn(*port)),
            Note::StpMigrate { port, rstp: true } => format!("STP: {} tries RSTP again\n", pn(*port)),
            Note::StpErrDisable { port } => format!("STP: BPDU guard shut {} off\n", pn(*port)),
            Note::StpDropped { port, .. } => format!("STP: a BPDU on {} was dropped\n", pn(*port)),
            _ => String::new(),
        }
    }
}
