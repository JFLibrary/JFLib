// fw -- the zone firewall at the IOS-like command line (firewall models only): zones, zone pairs,
// class maps and policy maps with their modes, "zone-member security" on interfaces, the timers of
// sessions, the show and clear commands, "debug policy-firewall" and the lines of the running
// configuration; help texts, tables and messages are our own words, the command words and
// configuration lines follow the original
//
//   build_fw(tree, modes)            the commands in an IOS command tree (ios.rs; F_FIREWALL) with
//                                    the modes "(config-sec-zone)#", "(config-sec-zone-pair)#",
//                                    "(config-cmap)#", "(config-pmap)#", "(config-pmap-c)#"
//   Engine::ios_fw(ni, p)            the commands
//   Engine::ios_fw_member(ni, iface, p)   "zone-member security" of one interface
//   Engine::ios_fw_config(ni)        the global lines (before the interfaces, which name the zones)
//   Engine::ios_fw_if_config(ni, iface)   " zone-member security NAME"
//   Engine::fw_debug(note)           the "debug policy-firewall" line of a decision (log.rs)
//   Engine::ios_fw_problem(..)       a refusal in the console's words
//
// CliDev::fw holds what a mode edits: the zone, pair, class or policy, and the policy's class.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};

use super::ios::K;
use super::Mode;
use crate::action::Action;
use crate::fw::{FwAction, FwOp, FwRow, FwTimer, Match, PolicyClass, CLASS_DEFAULT, FW_PROTOCOLS, HALF_OPEN_MAX, SELF_ZONE};
use crate::fwtrace::FwNote;
use crate::kernel::{Engine, Ns, SEC};
use crate::trace::Note;

const NEGATE: &str = "Undo a setting or restore its default";
const LOG: &str = "Report what it decides on the console and in the log";

/// the commands of the zone firewall in an IOS tree; `m`: user, privileged, global configuration,
/// interface and subinterface mode
pub(crate) fn build_fw(t: &mut Tree<K>, m: [ModeId; 5]) {
    let [user, prv, conf, ifm, sub] = m;
    let zm = t.mode("(config-sec-zone)#", "Commands for this zone:", Some(conf), Some(NEGATE));
    let zp = t.mode("(config-sec-zone-pair)#", "Commands for this zone pair:", Some(conf), Some(NEGATE));
    let cm = t.mode("(config-cmap)#", "Commands for this class of traffic:", Some(conf), Some(NEGATE));
    let pm = t.mode("(config-pmap)#", "Commands for this policy:", Some(conf), Some(NEGATE));
    let pc = t.mode("(config-pmap-c)#", "What this policy does with this class:", Some(conf), Some(NEGATE));

    // ---- EXEC
    for x in [user, prv] {
        t.add(x, "show zone", "The zones of the firewall", None, 0);
        t.add(x, "show zone security", "Zones and their interfaces", Some(K::ShowZone), 0);
        t.add(x, "show zone security WORD=zname", "Only this zone", Some(K::ShowZone), 0);
        t.add(x, "show zone-pair", "The directions between zones", None, 0);
        t.add(x, "show zone-pair security", "Zone pairs, their policies and the firewall's counters", Some(K::ShowZonePair), 0);
        t.add(x, "show zone-pair security source", "Only the pair from this zone", None, 0);
        t.add(x, "show zone-pair security source WORD=src", "The zone it starts from", None, 0);
        t.add(x, "show zone-pair security source WORD=src destination", "To this zone", None, 0);
        t.add(x, "show zone-pair security source WORD=src destination WORD=dst", "The zone it leads to", Some(K::ShowZonePair), 0);
        t.add(x, "show policy-map", "Policies", None, 0);
        t.add(x, "show policy-map type", "Policies of a kind", None, 0);
        t.add(x, "show policy-map type inspect", "Policies of the firewall", None, 0);
        t.add(x, "show policy-map type inspect zone-pair", "Each pair with its policy, classes and counters", Some(K::ShowPolicyZone), 0);
        t.add(x, "show policy-map type inspect zone-pair WORD=pair", "Only this pair", Some(K::ShowPolicyZone), 0);
        t.add(x, "show policy-map type inspect zone-pair WORD=pair sessions", "The sessions of this pair", Some(K::ShowPolicyZone), 0);
        t.add(x, "show policy-map type inspect zone-pair sessions", "The sessions of every pair", Some(K::ShowPolicyZone), 0);
    }
    t.add(prv, "clear zone-pair", "Forget state of the firewall", None, 0);
    t.add(prv, "clear zone-pair inspect", "What inspection keeps", None, 0);
    t.add(prv, "clear zone-pair inspect sessions", "Forget every session", Some(K::ClearFw), 0);
    t.add(prv, "clear zone-pair inspect sessions WORD=pair", "Only the sessions of this pair", Some(K::ClearFw), 0);
    t.add(prv, "clear zone-pair counter", "Set the counters of the firewall back to 0", Some(K::ClearFw), 0);
    t.add(prv, "debug policy-firewall=fw", "Decisions of the firewall as they happen", Some(K::Debug), 0);
    t.add(prv, "undebug policy-firewall=fw", "Decisions of the firewall as they happen", Some(K::Undebug), 0);

    // ---- global configuration
    t.add(conf, "zone", "A zone of the firewall", None, F_NO);
    t.add(conf, "zone security", "A security zone", None, 0);
    t.add(conf, "zone security WORD=zone", "Name of the zone", Some(K::FwZone), 0);
    t.add(conf, "zone-pair", "A direction between two zones", None, F_NO);
    t.add(conf, "zone-pair security", "Traffic from one zone to another", None, 0);
    let pre = "zone-pair security WORD=name";
    t.add(conf, pre, "Name of the pair", None, 0);
    t.add_no(conf, pre, K::FwPair);
    t.add(conf, &format!("{} source", pre), "The zone it starts from", None, 0);
    for (src, sh) in [("self=src", "The device itself"), ("WORD=src", "Name of the zone")] {
        t.add(conf, &format!("{} source {}", pre, src), sh, None, 0);
        t.add(conf, &format!("{} source {} destination", pre, src), "The zone it leads to", None, 0);
        for (dst, dh) in [("self=dst", "The device itself"), ("WORD=dst", "Name of the zone")] {
            t.add(conf, &format!("{} source {} destination {}", pre, src, dst), dh, Some(K::FwPair), 0);
        }
    }
    t.add(conf, "class-map", "A class of traffic", None, F_NO);
    t.add(conf, "class-map type", "A class of a kind", None, 0);
    t.add(conf, "class-map type inspect", "A class for the firewall", None, 0);
    t.add(conf, "class-map type inspect WORD=name", "Name of the class", Some(K::FwClass), 0);
    t.add(conf, "class-map type inspect match-any=how", "One condition is enough", None, 0);
    t.add(conf, "class-map type inspect match-any=how WORD=name", "Name of the class", Some(K::FwClass), 0);
    t.add(conf, "class-map type inspect match-all=how", "All conditions must hold (without this word too)", None, 0);
    t.add(conf, "class-map type inspect match-all=how WORD=name", "Name of the class", Some(K::FwClass), 0);
    t.add(conf, "policy-map", "What happens to classes of traffic", None, F_NO);
    t.add(conf, "policy-map type", "A policy of a kind", None, 0);
    t.add(conf, "policy-map type inspect", "A policy for the firewall", None, 0);
    t.add(conf, "policy-map type inspect WORD=name", "Name of the policy", Some(K::FwPolicy), 0);
    // ("ip" is there with its "no")
    t.add(conf, "ip inspect", "Timers and limits of the firewall's sessions", None, 0);
    for (path, h) in [
        ("tcp synwait-time", "A TCP session before its handshake is done"),
        ("tcp finwait-time", "A TCP session after a FIN"),
        ("tcp idle-time", "An established TCP session without traffic"),
        ("udp idle-time", "A UDP session without traffic"),
        ("icmp idle-time", "An ICMP session without an answer"),
        ("dns-timeout", "A UDP session to port 53"),
    ] {
        let mut at = String::from("ip inspect");
        for w in path.split(' ') {
            at.push(' ');
            at.push_str(w);
            t.add(conf, &at, h, None, 0);
        }
        t.add_no(conf, &at, K::FwTimer);
        t.add(conf, &format!("{} <1-2147483>=secs", at), "Seconds", Some(K::FwTimer), 0);
    }
    t.add(conf, "ip inspect max-incomplete", "Half-open TCP sessions", None, 0);
    t.add(conf, "ip inspect max-incomplete high", "A new connection beyond this many is dropped", None, 0);
    t.add_no(conf, "ip inspect max-incomplete high", K::FwMax);
    t.add(conf, "ip inspect max-incomplete high <1-2147483647>=n", "Sessions", Some(K::FwMax), 0);

    // ---- interfaces
    for x in [ifm, sub] {
        t.add(x, "zone-member", "The zone of this interface", None, F_NO);
        t.add(x, "zone-member security", "A security zone", None, 0);
        t.add_no(x, "zone-member security", K::FwMember);
        t.add(x, "zone-member security WORD=zone", "Name of the zone", Some(K::FwMember), 0);
        // (a whole word wins over an abbreviation: "zone security X" is no zone-member)
        t.add(x, "zone", "", None, F_NO | F_HIDDEN);
        t.add(x, "zone security", "", None, 0);
        t.add(x, "zone security WORD=zone", "", Some(K::FwZone), 0);
    }

    // ---- the modes
    for x in [zm, zp, cm, pm, pc] {
        t.add(x, "do", "Run an EXEC command from here", None, 0);
        t.add(x, "do LINE=cmd", "The command", Some(K::Do), 0);
        t.add(x, "end", "Finish configuring", Some(K::End), F_HIDDEN);
    }
    for (x, h) in [(zm, "Back to the device configuration"), (zp, "Back to the device configuration"), (cm, "Back to the device configuration"), (pm, "Back to the device configuration")] {
        t.add(x, "exit", h, Some(K::Exit), 0);
    }
    t.add(pc, "exit", "Back to the policy", Some(K::FwExit), 0);
    t.add(zm, "description", "A note on the zone", None, F_NO);
    t.add_no(zm, "description", K::FwZoneDescr);
    t.add(zm, "description LINE=text", "The note", Some(K::FwZoneDescr), 0);
    t.add(zp, "service-policy", "The policy of this pair", None, F_NO);
    t.add(zp, "service-policy type", "A policy of a kind", None, 0);
    t.add(zp, "service-policy type inspect", "A policy for the firewall", None, 0);
    t.add_no(zp, "service-policy type inspect", K::FwService);
    t.add(zp, "service-policy type inspect WORD=policy", "Name of the policy", Some(K::FwService), 0);
    t.add(cm, "match", "A condition of this class", None, F_NO);
    t.add(cm, "match protocol", "A protocol, or a well-known port of TCP or UDP", None, 0);
    for (name, _, _, h) in FW_PROTOCOLS {
        t.add(cm, &format!("match protocol {}=proto", name), h, Some(K::FwMatch), 0);
    }
    t.add(cm, "match access-group", "Packets an access list permits", None, 0);
    t.add(cm, "match access-group <1-199>=acl", "Number of the list", Some(K::FwMatch), 0);
    t.add(cm, "match access-group <1300-2699>=acl", "Number of the list, second range", Some(K::FwMatch), 0);
    t.add(cm, "match access-group name", "A list by its name", None, 0);
    t.add(cm, "match access-group name WORD=acl", "Name of the list", Some(K::FwMatch), 0);
    for x in [pm, pc] {
        t.add(x, "class", "A class in this policy", None, F_NO);
        t.add(x, "class type", "A class of a kind", None, 0);
        t.add(x, "class type inspect", "A class for the firewall", None, 0);
        t.add(x, "class type inspect WORD=class", "Name of the class", Some(K::FwPolicyClass), 0);
        t.add(x, "class class-default=class", "What no other class matches", Some(K::FwPolicyClass), 0);
    }
    for (word, h) in [("inspect", "Let it through and follow the connection: the answers come back by themselves"), ("pass", "Let this packet through, remember nothing"), ("drop", "Drop it silently"), ("reject", "Drop it and tell the sender")] {
        t.add(pc, &format!("{}=act", word), h, Some(K::FwAct), F_NO);
        t.add(pc, &format!("{}=act log", word), LOG, Some(K::FwAct), 0);
    }
}

/// "hh:mm:ss" of a span
fn hms(ns: Ns) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// the timer the parsed words name
fn timer_of(p: &Parsed<K>) -> FwTimer {
    if p.has("dns-timeout") {
        FwTimer::Dns
    } else if p.has("udp") {
        FwTimer::Udp
    } else if p.has("icmp") {
        FwTimer::Icmp
    } else if p.has("synwait-time") {
        FwTimer::Syn
    } else if p.has("finwait-time") {
        FwTimer::Fin
    } else {
        FwTimer::Tcp
    }
}

/// a class in a policy as a line of its action: "inspect log"; the action may be missing
fn action_line(c: &PolicyClass) -> Option<String> {
    c.action.map(|a| format!("{}{}", a.word(), if c.log { " log" } else { "" }))
}

impl Engine {
    /// the object a mode edits
    fn fw_ctx(&self, ni: usize) -> [String; 2] {
        self.cdev(ni).map(|c| c.fw.clone()).unwrap_or_default()
    }

    /// the mode and what it edits
    fn fw_enter(&mut self, ni: usize, mode: Mode, a: &str, b: &str) {
        self.set_mode(ni, mode);
        if let Some(c) = self.cdev_mut(ni) {
            c.fw = [String::from(a), String::from(b)];
        }
    }

    /// the commands of the zone firewall
    pub(crate) fn ios_fw(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let [ctx, ctx2] = self.fw_ctx(ni);
        match p.cmd {
            K::ShowZone => {
                let text = self.show_zones(ni, p.text("zname"));
                self.out(ni, &text);
                Ok(())
            }
            K::ShowZonePair => {
                let text = self.show_zone_pairs(ni, p.text("src"), p.text("dst"));
                self.out(ni, &text);
                Ok(())
            }
            K::ShowPolicyZone => {
                let text = if p.has("sessions") { self.show_fw_sessions(ni, p.text("pair")) } else { self.show_fw_policy(ni, p.text("pair")) };
                self.out(ni, &text);
                Ok(())
            }
            K::ClearFw => {
                let op = if p.has("counter") { FwOp::ClearCounters } else { FwOp::ClearSessions { pair: p.text("pair").map(String::from) } };
                let _ = self.apply(Action::Fw { dev, op });
                Ok(())
            }
            K::FwZone => {
                let name = String::from(p.text("zone").unwrap_or(""));
                if p.no {
                    let (members, paired) = self.stack(ni).map_or((false, false), |s| (s.fw.members.values().any(|z| *z == name), s.fw.pairs.values().any(|x| x.src == name || x.dst == name)));
                    if members {
                        return self.ios_err(ni, &format!("% Zone {} still has interfaces\n", name));
                    }
                    if paired {
                        return self.ios_err(ni, &format!("% Zone {} is still part of a zone pair\n", name));
                    }
                    return match self.apply(Action::Fw { dev, op: FwOp::Zone { name: name.clone(), add: false } }) {
                        Ok(()) => Ok(()),
                        Err(_) => self.ios_err(ni, &format!("% Zone {} does not exist\n", name)),
                    };
                }
                if name.eq_ignore_ascii_case(SELF_ZONE) {
                    return self.ios_err(ni, "% The name self belongs to the device itself\n");
                }
                let _ = self.apply(Action::Fw { dev, op: FwOp::Zone { name: name.clone(), add: true } });
                self.fw_enter(ni, Mode::FwZone, &name, "");
                Ok(())
            }
            K::FwZoneDescr => {
                let text = if p.no { None } else { p.text("text").map(|t| String::from(t.trim())) };
                let _ = self.apply(Action::Fw { dev, op: FwOp::ZoneDescr { name: ctx, text } });
                Ok(())
            }
            K::FwPair => {
                let name = String::from(p.text("name").unwrap_or(""));
                if p.no {
                    return match self.apply(Action::Fw { dev, op: FwOp::PairGone { name: name.clone() } }) {
                        Ok(()) => Ok(()),
                        Err(_) => self.ios_err(ni, &format!("% Zone pair {} does not exist\n", name)),
                    };
                }
                let src = String::from(p.text("src").unwrap_or(""));
                let dst = String::from(p.text("dst").unwrap_or(""));
                if src == dst {
                    return self.ios_err(ni, "% A zone pair joins two different zones\n");
                }
                let known = |e: &Engine, z: &str| z == SELF_ZONE || e.stack(ni).is_some_and(|s| s.fw.zones.contains_key(z));
                for z in [&src, &dst] {
                    if !known(self, z) {
                        return self.ios_err(ni, &format!("% Zone {} does not exist\n", z));
                    }
                }
                match self.apply(Action::Fw { dev, op: FwOp::Pair { name: name.clone(), src: src.clone(), dst: dst.clone() } }) {
                    Ok(()) => {
                        self.fw_enter(ni, Mode::FwPair, &name, "");
                        Ok(())
                    }
                    Err(_) => self.ios_err(ni, &format!("% There is already a zone pair from {} to {}\n", src, dst)),
                }
            }
            K::FwService => {
                let policy = if p.no { None } else { p.text("policy").map(String::from) };
                if let Some(x) = &policy
                    && !self.stack(ni).is_some_and(|s| s.fw.policies.contains_key(x))
                {
                    return self.ios_err(ni, &format!("% Policy {} does not exist\n", x));
                }
                let _ = self.apply(Action::Fw { dev, op: FwOp::PairPolicy { name: ctx, policy } });
                Ok(())
            }
            K::FwClass => {
                let name = String::from(p.text("name").unwrap_or(""));
                if p.no {
                    return match self.apply(Action::Fw { dev, op: FwOp::Class { name: name.clone(), any: false, add: false } }) {
                        Ok(()) => Ok(()),
                        Err(_) if self.stack(ni).is_some_and(|s| s.fw.classes.contains_key(&name)) => self.ios_err(ni, &format!("% Class {} is still part of a policy\n", name)),
                        Err(_) => self.ios_err(ni, &format!("% Class {} does not exist\n", name)),
                    };
                }
                let _ = self.apply(Action::Fw { dev, op: FwOp::Class { name: name.clone(), any: p.text("how") == Some("match-any"), add: true } });
                self.fw_enter(ni, Mode::FwClass, &name, "");
                Ok(())
            }
            K::FwMatch => {
                let m = match (p.text("proto"), p.num("acl"), p.text("acl")) {
                    (Some(x), _, _) => Match::Protocol(String::from(x)),
                    (None, Some(n), _) => Match::Access(format!("{}", n)),
                    (None, None, Some(x)) => Match::Access(String::from(x)),
                    _ => return Ok(()),
                };
                let _ = self.apply(Action::Fw { dev, op: FwOp::ClassMatch { class: ctx, m, add: !p.no } });
                Ok(())
            }
            K::FwPolicy => {
                let name = String::from(p.text("name").unwrap_or(""));
                if p.no {
                    return match self.apply(Action::Fw { dev, op: FwOp::Policy { name: name.clone(), add: false } }) {
                        Ok(()) => Ok(()),
                        Err(_) if self.stack(ni).is_some_and(|s| s.fw.policies.contains_key(&name)) => self.ios_err(ni, &format!("% Policy {} still applies to a zone pair\n", name)),
                        Err(_) => self.ios_err(ni, &format!("% Policy {} does not exist\n", name)),
                    };
                }
                let _ = self.apply(Action::Fw { dev, op: FwOp::Policy { name: name.clone(), add: true } });
                self.fw_enter(ni, Mode::FwPolicy, &name, "");
                Ok(())
            }
            K::FwPolicyClass => {
                let class = String::from(p.text("class").unwrap_or(""));
                if p.no {
                    let _ = self.apply(Action::Fw { dev, op: FwOp::PolicyClass { policy: ctx.clone(), class: class.clone(), add: false } });
                    if class == CLASS_DEFAULT {
                        let _ = self.apply(Action::Fw { dev, op: FwOp::PolicyAction { policy: ctx.clone(), class, action: None, log: false } });
                    }
                    self.fw_enter(ni, Mode::FwPolicy, &ctx, "");
                    return Ok(());
                }
                if self.apply(Action::Fw { dev, op: FwOp::PolicyClass { policy: ctx.clone(), class: class.clone(), add: true } }).is_err() {
                    return self.ios_err(ni, &format!("% Class {} does not exist\n", class));
                }
                self.fw_enter(ni, Mode::FwPolicyClass, &ctx, &class);
                Ok(())
            }
            K::FwAct => {
                let action = if p.no { None } else { FwAction::ALL.iter().copied().find(|a| Some(a.word()) == p.text("act")) };
                let _ = self.apply(Action::Fw { dev, op: FwOp::PolicyAction { policy: ctx, class: ctx2, action, log: !p.no && p.has("log") } });
                Ok(())
            }
            K::FwExit => {
                self.fw_enter(ni, Mode::FwPolicy, &ctx, "");
                Ok(())
            }
            K::FwTimer => {
                let secs = if p.no { None } else { p.num("secs").map(|s| s as u32) };
                let _ = self.apply(Action::Fw { dev, op: FwOp::Timer { which: timer_of(p), secs } });
                Ok(())
            }
            K::FwMax => {
                let n = if p.no { None } else { p.num("n").map(|s| s as u32) };
                let _ = self.apply(Action::Fw { dev, op: FwOp::MaxHalfOpen(n) });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// "zone-member security NAME" (and its "no") of one interface
    pub(crate) fn ios_fw_member(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        if self.stack(ni).and_then(|s| s.iface_by_name(name)).is_none() {
            let pending = self.cdev(ni).is_some_and(|c| c.cfg.pending.iter().any(|x| x == name));
            return self.ios_err(ni, if pending { "% Give the subinterface its VLAN first (encapsulation dot1Q)\n" } else { "% Only routed interfaces take this\n" });
        }
        let zone = if p.no { None } else { p.text("zone").map(String::from) };
        if let Some(z) = &zone
            && !self.stack(ni).is_some_and(|s| s.fw.zones.contains_key(z))
        {
            return self.ios_err(ni, &format!("% Zone {} does not exist\n", z));
        }
        let _ = self.apply(Action::Fw { dev, op: FwOp::Member { iface: String::from(name), zone } });
        Ok(())
    }

    /// "show zone security" in our words
    fn show_zones(&self, ni: usize, only: Option<&str>) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for (name, descr) in &st.fw.zones {
            if only.is_some_and(|o| o != name) {
                continue;
            }
            s.push_str(&format!("zone {}\n", name));
            if !descr.is_empty() {
                s.push_str(&format!("  description: {}\n", descr));
            }
            let members: Vec<&str> = st.fw.members.iter().filter(|(_, z)| *z == name).filter_map(|(i, _)| st.ifaces.get(*i as usize).map(|f| f.name.as_str())).collect();
            if members.is_empty() {
                s.push_str("  interfaces: none\n");
            } else {
                s.push_str(&format!("  interfaces: {}\n", members.join(", ")));
            }
        }
        if only.is_none_or(|o| o == SELF_ZONE) {
            s.push_str("zone self (the device itself; every interface reaches it)\n");
        }
        s
    }

    /// "show zone-pair security" in our words, with the firewall's counters
    fn show_zone_pairs(&self, ni: usize, src: Option<&str>, dst: Option<&str>) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let fw = &st.fw;
        let mut s = String::new();
        for (name, p) in fw.pairs.iter().filter(|(_, p)| src.is_none_or(|x| x == p.src) && dst.is_none_or(|x| x == p.dst)) {
            match &p.policy {
                Some(x) => s.push_str(&format!("pair {}: {} -> {}, policy {}\n", name, p.src, p.dst, x)),
                None if p.src == SELF_ZONE || p.dst == SELF_ZONE => s.push_str(&format!("pair {}: {} -> {}, no policy: everything passes\n", name, p.src, p.dst)),
                None => s.push_str(&format!("pair {}: {} -> {}, no policy: everything is dropped\n", name, p.src, p.dst)),
            }
        }
        if src.is_none() {
            let t = fw.stats;
            let live = self.fw_sessions(self.dev_of(ni));
            s.push_str(&format!("Sessions: {} now ({} half-open, limit {}); {} began, {} timed out, {} reset\n", live.len(), fw.half_open(self.now), fw.max_half_open, t.created, t.expired, t.reset));
            s.push_str(&format!("Packets: {} by a session; dropped: {} without a rule, {} beginning nothing, {} over the limit\n", t.session_hits, t.no_rule, t.not_new, t.limited));
        }
        s
    }

    /// "show policy-map type inspect zone-pair" in our words
    fn show_fw_policy(&self, ni: usize, only: Option<&str>) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let fw = &st.fw;
        let live = self.fw_sessions(self.dev_of(ni));
        let mut s = String::new();
        for (name, pair) in fw.pairs.iter().filter(|(n, _)| only.is_none_or(|o| o == n.as_str())) {
            s.push_str(&format!("pair {}: {} -> {}\n", name, pair.src, pair.dst));
            let Some(pname) = &pair.policy else {
                s.push_str("  no policy\n");
                continue;
            };
            s.push_str(&format!("  policy {}\n", pname));
            let Some(policy) = fw.policies.get(pname) else {
                continue;
            };
            for c in policy.classes.iter().chain(core::iter::once(&policy.default)) {
                let how = match fw.classes.get(&c.class) {
                    Some(k) if k.any => " (match-any)",
                    Some(_) => " (match-all)",
                    None => "",
                };
                s.push_str(&format!("    class {}{}\n", c.class, how));
                if let Some(k) = fw.classes.get(&c.class) {
                    for m in &k.matches {
                        s.push_str(&format!("      {}\n", m.config()));
                    }
                }
                let act = action_line(c).unwrap_or_else(|| String::from("drop (no action named)"));
                s.push_str(&format!("      action: {}; {} packets\n", act, c.hits));
                if c.action == Some(FwAction::Inspect) {
                    let n = live.iter().filter(|r| r.session.pair == *name && r.session.class == c.class).count();
                    s.push_str(&format!("      sessions now: {}\n", n));
                }
            }
        }
        s
    }

    /// "show policy-map type inspect zone-pair sessions" in our words
    fn show_fw_sessions(&self, ni: usize, only: Option<&str>) -> String {
        let now = self.now;
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let live = self.fw_sessions(self.dev_of(ni));
        let mut s = String::new();
        for name in st.fw.pairs.keys().filter(|n| only.is_none_or(|o| o == n.as_str())) {
            let rows: Vec<&FwRow> = live.iter().filter(|r| r.session.pair == *name).collect();
            s.push_str(&format!("pair {}: {} sessions\n", name, rows.len()));
            for r in rows {
                let x = &r.session;
                let state = x.tcp.map_or("open", |t| t.word());
                s.push_str(&format!("  {} {} {}, class {}\n", x.id, x.text(), state, x.class));
                s.push_str(&format!("    age {}, ends in {}; packets {} there, {} back; bytes {} there, {} back\n", hms(now.saturating_sub(x.created)), hms(r.ends.saturating_sub(now)), x.packets[0], x.packets[1], x.bytes[0], x.bytes[1]));
            }
        }
        s
    }

    /// the global lines of the running configuration: classes, policies, zones, pairs, timers
    pub(crate) fn ios_fw_config(&self, ni: usize) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let fw = &st.fw;
        let mut s = String::new();
        for (name, c) in &fw.classes {
            s.push_str(&format!("class-map type inspect {}{}\n", if c.any { "match-any " } else { "" }, name));
            for m in &c.matches {
                s.push_str(&format!(" {}\n", m.config()));
            }
            s.push_str("!\n");
        }
        for (name, p) in &fw.policies {
            s.push_str(&format!("policy-map type inspect {}\n", name));
            for c in &p.classes {
                s.push_str(&format!(" class type inspect {}\n", c.class));
                if let Some(a) = action_line(c) {
                    s.push_str(&format!("  {}\n", a));
                }
            }
            if p.default.action != Some(FwAction::Drop) || p.default.log {
                s.push_str(" class class-default\n");
                if let Some(a) = action_line(&p.default) {
                    s.push_str(&format!("  {}\n", a));
                }
            }
            s.push_str("!\n");
        }
        for (name, descr) in &fw.zones {
            s.push_str(&format!("zone security {}\n", name));
            if !descr.is_empty() {
                s.push_str(&format!(" description {}\n", descr));
            }
            s.push_str("!\n");
        }
        for (name, p) in &fw.pairs {
            s.push_str(&format!("zone-pair security {} source {} destination {}\n", name, p.src, p.dst));
            if let Some(x) = &p.policy {
                s.push_str(&format!(" service-policy type inspect {}\n", x));
            }
            s.push_str("!\n");
        }
        let mut tail = String::new();
        for k in FwTimer::ALL {
            let v = fw.timers.get(k);
            if v != k.default_secs() {
                tail.push_str(&format!("ip inspect {} {}\n", k.words(), v));
            }
        }
        if fw.max_half_open != HALF_OPEN_MAX {
            tail.push_str(&format!("ip inspect max-incomplete high {}\n", fw.max_half_open));
        }
        if !tail.is_empty() {
            s.push_str(&tail);
            s.push_str("!\n");
        }
        s
    }

    /// " zone-member security NAME" of an interface
    pub(crate) fn ios_fw_if_config(&self, ni: usize, iface: u16) -> String {
        match self.stack(ni).and_then(|s| s.fw.members.get(&iface)) {
            Some(z) => format!(" zone-member security {}\n", z),
            None => String::new(),
        }
    }

    /// the "debug policy-firewall" line of a decision (our words)
    pub(crate) fn fw_debug(&self, note: &Note) -> String {
        let Note::Fw(n) = note else {
            return String::new();
        };
        match n {
            FwNote::SameZone { zone, what, .. } => format!("FW: {} stays in zone {}, passes\n", what, zone),
            FwNote::SelfPass { from, to, what, .. } => format!("FW: {} from {} to {}, the device itself without a policy, passes\n", what, from, to),
            FwNote::Unzoned { zone, what, .. } => format!("FW: {} between zone {} and an interface without a zone, dropped\n", what, zone),
            FwNote::NoRule { from, to, what, .. } => format!("FW: {} from {} to {}, no rule between the zones, dropped\n", what, from, to),
            FwNote::NewSession { id, pair, class, what, .. } => format!("FW: {} begins session {} ({}, class {})\n", what, id, pair, class),
            FwNote::Session { id, back, what, .. } => format!("FW: {} belongs to session {} ({})\n", what, id, if *back { "back" } else { "there" }),
            FwNote::Related { id, what, .. } => format!("FW: {} is an ICMP error about session {}, passes\n", what, id),
            FwNote::Pass { pair, class, what, .. } => format!("FW: {} passes without a session ({}, class {})\n", what, pair, class),
            FwNote::Drop { pair, class, what, .. } => format!("FW: {} dropped ({}, class {})\n", what, pair, class),
            FwNote::Reject { pair, class, what, .. } => format!("FW: {} rejected, the sender is told ({}, class {})\n", what, pair, class),
            FwNote::NotNew { pair, what, .. } => format!("FW: {} begins nothing and belongs to no session, dropped ({})\n", what, pair),
            FwNote::Limit { pair, max, what, .. } => format!("FW: {} dropped, already {} half-open sessions ({})\n", what, max, pair),
            FwNote::Reset { id, what, .. } => format!("FW: {} ends session {} (RST)\n", what, id),
            FwNote::Expired { id, what, .. } => format!("FW: session {} ({}) timed out\n", id, what),
        }
    }
}
