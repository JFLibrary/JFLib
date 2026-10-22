// nat -- address translation at the IOS-like command line: "ip nat inside|outside" on
// interfaces, "ip nat pool", "ip nat inside source list|static", "ip nat translation ...
// timeout", "show ip nat translations [verbose]", "show ip nat statistics", "clear ip nat
// translation *", "clear ip nat statistics", "debug ip nat", and the lines of the running
// configuration; help texts, statistics and debug lines are our own words, the syntax, the
// translation table and the configuration lines follow the original
//
//   build_nat(tree, modes)            the commands in an IOS command tree (ios.rs; devices that
//                                     route)
//   Engine::ios_nat(ni, p)            the global commands, show and clear
//   Engine::ios_nat_side(ni, iface, p)    "ip nat inside|outside" of an interface
//   Engine::ios_nat_config(ni)        the global lines (before the static routes, as the
//                                     original writes them)
//   Engine::ios_nat_if_config(ni, iface)  " ip nat inside|outside"
//   Engine::nat_debug(note)           the "debug ip nat" lines of a decision (log.rs)
//
// The table of "show ip nat translations" lists the flows (protocol, inside global, inside
// local, outside local, outside global; the outside addresses stay as they are) and the
// addresses alone ("---"); "verbose" adds when each was made and used, when it ends and what
// kind it is.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::Ipv4;

use super::fmt::mask_str;
use super::ios::K;
use crate::action::{ActErr, Action};
use crate::kernel::{Engine, Ns, SEC};
use crate::nat::{Global, NatAct, Pool, Rule, Side, Target, Tmo, XlateRow, NEVER};
use crate::trace::Note;

/// the commands of NAT in an IOS tree; `m`: user, privileged, global configuration, interface
/// and subinterface mode
pub(crate) fn build_nat(t: &mut Tree<K>, m: [ModeId; 5]) {
    let [user, prv, conf, ifm, sub] = m;
    for x in [user, prv] {
        t.add(x, "show ip nat", "Address translation", None, 0);
        t.add(x, "show ip nat translations", "The translations there are now", Some(K::ShowNatTrans), 0);
        t.add(x, "show ip nat translations verbose", "With their times and kinds", Some(K::ShowNatTrans), 0);
        t.add(x, "show ip nat statistics", "Counters, interfaces, rules and pools", Some(K::ShowNatStats), 0);
    }
    t.add(prv, "clear ip nat", "Forget state of address translation", None, 0);
    t.add(prv, "clear ip nat translation", "Dynamic translations", None, 0);
    t.add(prv, "clear ip nat translation *=all", "All of them", Some(K::ClearNat), 0);
    t.add(prv, "clear ip nat statistics", "Set the counters back to 0", Some(K::ClearNat), 0);
    t.add(prv, "debug ip nat", "Translations as they happen", Some(K::Debug), 0);
    t.add(prv, "undebug ip nat", "Translations as they happen", Some(K::Undebug), 0);

    // ---- interfaces
    for x in [ifm, sub] {
        t.add(x, "ip nat", "The side of this interface for address translation", None, 0);
        t.add(x, "ip nat inside=side", "Toward the hosts whose addresses are translated", Some(K::NatSide), 0);
        t.add(x, "ip nat outside=side", "Toward the networks that see the global addresses", Some(K::NatSide), 0);
    }

    // ---- global configuration ("ip" is there with its "no")
    t.add(conf, "ip nat", "Address translation (NAT)", None, 0);
    t.add(conf, "ip nat pool", "A range of global addresses", None, 0);
    t.add(conf, "ip nat pool WORD=pool", "Name of the pool", None, 0);
    t.add_no(conf, "ip nat pool WORD=pool", K::NatPool);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start", "First address", None, 0);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start A.B.C.D=end", "Last address", None, 0);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start A.B.C.D=end netmask", "The network of the pool as a mask", None, 0);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start A.B.C.D=end netmask MASK=mask", "Network mask", Some(K::NatPool), 0);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start A.B.C.D=end prefix-length", "The network of the pool as a length", None, 0);
    t.add(conf, "ip nat pool WORD=pool A.B.C.D=start A.B.C.D=end prefix-length <1-32>=plen", "Prefix length", Some(K::NatPool), 0);
    t.add(conf, "ip nat inside", "Translate the addresses of inside hosts", None, 0);
    t.add(conf, "ip nat inside source", "Their source addresses", None, 0);
    // dynamic: hosts an access list selects
    t.add(conf, "ip nat inside source list", "Hosts that an access list permits", None, 0);
    for (acl, h) in [("<1-2699>=acl", "Number of the access list"), ("WORD=acl", "Name of the access list")] {
        let pre = format!("ip nat inside source list {}", acl);
        t.add(conf, &pre, h, None, 0);
        t.add_no(conf, &pre, K::NatList);
        t.add(conf, &format!("{} pool", pre), "Global addresses from a pool", None, 0);
        t.add(conf, &format!("{} pool WORD=pool", pre), "Name of the pool", Some(K::NatList), 0);
        t.add(conf, &format!("{} pool WORD=pool overload", pre), "Share the addresses, hosts told apart by their ports", Some(K::NatList), 0);
        t.add(conf, &format!("{} interface", pre), "The address of an interface", None, 0);
        t.add(conf, &format!("{} interface NAME1=gif", pre), "", None, 0);
        t.add(conf, &format!("{} interface NAME1=gif overload", pre), "Share the address, hosts told apart by their ports", Some(K::NatList), 0);
    }
    // static: addresses and ports
    t.add(conf, "ip nat inside source static", "A fixed translation", None, 0);
    t.add(conf, "ip nat inside source static A.B.C.D=local", "Inside address", None, 0);
    t.add_no(conf, "ip nat inside source static A.B.C.D=local", K::NatStatic);
    t.add(conf, "ip nat inside source static A.B.C.D=local A.B.C.D=global", "Global address", Some(K::NatStatic), 0);
    t.add(conf, "ip nat inside source static A.B.C.D=local A.B.C.D=global extendable", "Allow more translations of the same address", Some(K::NatStatic), 0);
    t.add(conf, "ip nat inside source static A.B.C.D=local interface", "The address of an interface", None, 0);
    t.add(conf, "ip nat inside source static A.B.C.D=local interface NAME1=gif", "", Some(K::NatStatic), 0);
    let tcp = "ip nat inside source static tcp=sproto";
    t.add(conf, tcp, "One TCP port", None, 0);
    t.add(conf, "ip nat inside source static udp=sproto", "One UDP port", None, 0);
    t.also(conf, "ip nat inside source static udp=sproto", tcp);
    t.add(conf, &format!("{} A.B.C.D=local", tcp), "Inside address", None, 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport", tcp), "Inside port", None, 0);
    t.add_no(conf, &format!("{} A.B.C.D=local <1-65535>=lport", tcp), K::NatStatic);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport A.B.C.D=global", tcp), "Global address", None, 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport A.B.C.D=global <1-65535>=gport", tcp), "Global port", Some(K::NatStatic), 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport A.B.C.D=global <1-65535>=gport extendable", tcp), "Allow more translations of the same address", Some(K::NatStatic), 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport interface", tcp), "The address of an interface", None, 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport interface NAME1=gif", tcp), "", None, 0);
    t.add(conf, &format!("{} A.B.C.D=local <1-65535>=lport interface NAME1=gif <1-65535>=gport", tcp), "Global port", Some(K::NatStatic), 0);
    // timeouts
    t.add(conf, "ip nat translation", "How long translations live without traffic", None, 0);
    for (kw, h) in [
        ("timeout", "Addresses translated without ports"),
        ("udp-timeout", "UDP flows"),
        ("dns-timeout", "UDP flows to port 53 (DNS)"),
        ("tcp-timeout", "TCP connections after the handshake"),
        ("finrst-timeout", "TCP connections after a RST or a FIN from both ends"),
        ("syn-timeout", "TCP connections before the handshake is complete"),
        ("icmp-timeout", "ICMP queries"),
    ] {
        let pre = format!("ip nat translation {}=which", kw);
        t.add(conf, &pre, h, None, 0);
        t.add_no(conf, &pre, K::NatTimeout);
        t.add(conf, &format!("{} <0-536870>=secs", pre), "Seconds", Some(K::NatTimeout), 0);
        t.add(conf, &format!("{} never", pre), "No timeout", Some(K::NatTimeout), 0);
    }
}

/// a timeout keyword of the configuration
fn tmo_of(w: &str) -> Tmo {
    Tmo::ALL.iter().copied().find(|t| t.word() == w).unwrap_or(Tmo::Generic)
}

/// "hh:mm:ss" of a span
fn hms(ns: Ns) -> String {
    let s = ns / SEC;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// an endpoint of the translation table: "10.0.0.1:80", the address alone without a port
fn ep(a: Ipv4, p: Option<u16>) -> String {
    match p {
        Some(p) => format!("{}:{}", a, p),
        None => format!("{}", a),
    }
}

/// the protocol column of the table
fn pro(p: u8) -> String {
    match p {
        0 => String::from("---"),
        PROTO_TCP => String::from("tcp"),
        PROTO_UDP => String::from("udp"),
        PROTO_ICMP => String::from("icmp"),
        n => format!("{}", n),
    }
}

/// an endpoint of a debug line ("10.0.0.1:80"; the address alone when the port is 0)
fn dep(e: (Ipv4, u16)) -> String {
    if e.1 == 0 { format!("{}", e.0) } else { format!("{}:{}", e.0, e.1) }
}

impl Engine {
    /// "ip nat inside|outside" (and its "no") of one interface
    pub(crate) fn ios_nat_side(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let side = if p.text("side") == Some("outside") { Side::Outside } else { Side::Inside };
        let cur = self.stack(ni).and_then(|s| s.iface_by_name(name).and_then(|i| s.nat.side(i)));
        let set = if p.no {
            if cur != Some(side) {
                return Ok(());
            }
            None
        } else {
            Some(side)
        };
        let _ = self.apply(Action::Nat { dev, act: NatAct::Iface { iface: String::from(name), side: set } });
        Ok(())
    }

    /// the global commands of NAT, show and clear
    pub(crate) fn ios_nat(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match p.cmd {
            K::ShowNatTrans => {
                let text = self.show_nat_translations(ni, p.has("verbose"));
                self.out(ni, &text);
                Ok(())
            }
            K::ShowNatStats => {
                let text = self.show_nat_statistics(ni);
                self.out(ni, &text);
                Ok(())
            }
            K::ClearNat => {
                let _ = self.apply(Action::Nat { dev, act: NatAct::Clear { stats: p.has("statistics") } });
                Ok(())
            }
            K::NatPool => {
                let name = String::from(p.text("pool").unwrap_or(""));
                let exists = self.nat_part(ni).is_some_and(|n| n.pools.contains_key(&name));
                if p.no {
                    return match self.apply(Action::Nat { dev, act: NatAct::Pool { name: name.clone(), pool: None } }) {
                        Ok(()) => Ok(()),
                        Err(_) if !exists => self.ios_err(ni, &format!("% Pool {} does not exist\n", name)),
                        Err(_) => self.ios_err(ni, &format!("% Pool {} is in use\n", name)),
                    };
                }
                let len = p.mask("mask").or(p.num("plen").map(|n| n as u8)).unwrap_or(32);
                let pool = Pool { start: Ipv4(p.ip("start").unwrap_or([0; 4])), end: Ipv4(p.ip("end").unwrap_or([0; 4])), len };
                match self.apply(Action::Nat { dev, act: NatAct::Pool { name: name.clone(), pool: Some(pool) } }) {
                    Ok(()) => Ok(()),
                    Err(_) => {
                        let net = netpkt::Prefix4 { addr: pool.start, len: pool.len }.trunc();
                        if pool.start.to_u32() > pool.end.to_u32() || !net.contains(pool.end) {
                            self.ios_err(ni, &format!("% Start and end of pool {} lie in different networks\n", name))
                        } else {
                            self.ios_err(ni, &format!("% Pool {} is in use\n", name))
                        }
                    }
                }
            }
            K::NatList => {
                let acl = p.num("acl").map(|n| format!("{}", n)).or_else(|| p.text("acl").map(String::from)).unwrap_or_default();
                let to = match (p.text("pool"), p.text("gif")) {
                    (Some(pool), _) => Target::Pool(String::from(pool)),
                    (None, Some(i)) => Target::Iface(String::from(i)),
                    _ => Target::Pool(String::new()),
                };
                let overload = p.has("overload");
                let rule = Rule::List { acl, to, overload };
                let r = self.apply(Action::Nat { dev, act: NatAct::Rule { rule, add: !p.no } });
                match r {
                    Err(_) if p.no => self.ios_err(ni, "% No such rule of translation\n"),
                    _ => Ok(()),
                }
            }
            K::NatStatic => {
                let proto = match p.text("sproto") {
                    Some("tcp") => PROTO_TCP,
                    Some("udp") => PROTO_UDP,
                    _ => 0,
                };
                let local = Ipv4(p.ip("local").unwrap_or([0; 4]));
                let global = match (p.ip("global"), p.text("gif")) {
                    (Some(g), _) => Global::Addr(Ipv4(g)),
                    (None, Some(i)) => Global::Iface(String::from(i)),
                    _ => Global::Addr(Ipv4::UNSPECIFIED),
                };
                let rule = Rule::Static { proto, local, lport: p.num("lport").unwrap_or(0) as u16, global, gport: p.num("gport").unwrap_or(0) as u16, extendable: p.has("extendable") };
                match self.apply(Action::Nat { dev, act: NatAct::Rule { rule, add: !p.no } }) {
                    Ok(()) => Ok(()),
                    Err(ActErr::Refused(_)) if p.no => self.ios_err(ni, "% No such static translation\n"),
                    Err(_) => self.ios_err(ni, &format!("% {} already has a static translation\n", local)),
                }
            }
            K::NatTimeout => {
                let kind = tmo_of(p.text("which").unwrap_or("timeout"));
                let secs = if p.no {
                    None
                } else if p.has("never") {
                    Some(NEVER)
                } else {
                    p.num("secs").map(|s| s as u32)
                };
                let _ = self.apply(Action::Nat { dev, act: NatAct::Timeout { kind, secs } });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// the table of "show ip nat translations"
    fn show_nat_translations(&self, ni: usize, verbose: bool) -> String {
        let dev = self.dev_of(ni);
        let now = self.now;
        let rows = self.nat_translations(dev);
        let mut s = format!("{} {:<21} {:<21} {:<21} {}\n", "Pro", "Inside global", "Inside local", "Outside local", "Outside global");
        for r in &rows {
            let (ol, og) = match r.outside {
                Some(o) => (ep(o.0, Some(o.1)), ep(o.0, Some(o.1))),
                None => (String::from("---"), String::from("---")),
            };
            s.push_str(&format!("{} {:<21} {:<21} {:<21} {}\n", pro(r.proto), ep(r.inside_global.0, r.inside_global.1), ep(r.inside_local.0, r.inside_local.1), ol, og));
            if verbose {
                s.push_str(&verbose_line(r, now));
            }
        }
        s
    }

    /// "show ip nat statistics" in our words
    fn show_nat_statistics(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let (Some(st), Some(n)) = (self.stack(ni), self.nat_part(ni)) else {
            return String::new();
        };
        let rows = self.nat_translations(dev);
        let statics = rows.iter().filter(|r| !r.dynamic && r.outside.is_none()).count();
        let dynamics = rows.len() - statics;
        let extended = rows.iter().filter(|r| r.extended).count();
        let mut s = format!("Active translations: {} ({} static, {} dynamic; {} with ports)\n", rows.len(), statics, dynamics, extended);
        for (side, title) in [(Side::Outside, "Outside interfaces:"), (Side::Inside, "Inside interfaces:")] {
            s.push_str(title);
            s.push('\n');
            let names: Vec<&str> = n.sides.iter().filter(|(_, x)| **x == side).filter_map(|(i, _)| st.ifaces.get(*i as usize).filter(|f| !f.hidden).map(|f| f.name.as_str())).collect();
            if names.is_empty() {
                s.push_str("  none\n");
            }
            for name in names {
                s.push_str(&format!("  {}\n", name));
            }
        }
        s.push_str(&format!("Hits: {}  Misses: {}  Expired: {}  Refused: {}\n", n.stats.hits, n.stats.misses, n.stats.expired, n.stats.failed));
        let dynamic: Vec<&(u32, Rule)> = n.rules.iter().filter(|(_, r)| matches!(r, Rule::List { .. })).collect();
        if !dynamic.is_empty() {
            s.push_str("Dynamic rules:\n");
        }
        for (rid, r) in dynamic {
            let Rule::List { acl, to, overload } = r else {
                continue;
            };
            let used = n.mappings().filter(|m| m.rule == *rid).map(|m| if m.proto == 0 { 1 } else { m.sessions as usize }).sum::<usize>();
            let target = match to {
                Target::Pool(p) => format!("pool {}", p),
                Target::Iface(i) => format!("interface {}", i),
            };
            s.push_str(&format!("  list {} {}{}: {} in use\n", acl, target, if *overload { " overload" } else { "" }, used));
        }
        for (name, p) in &n.pools {
            let used = (p.start.to_u32()..=p.end.to_u32()).filter(|a| n.mappings().any(|m| m.global.0.to_u32() == *a)).count() as u32;
            s.push_str(&format!("Pool {}: {} to {}, netmask {}\n", name, p.start, p.end, mask_str(p.len)));
            s.push_str(&format!("  {} addresses, {} in use ({}%)\n", p.size(), used, used * 100 / p.size().max(1)));
        }
        let t = &n.timeouts;
        let secs = |k: Tmo| {
            let v = t.get(k);
            if v == NEVER { String::from("never") } else { format!("{} s", v) }
        };
        s.push_str(&format!(
            "Timeouts: {} (addresses), udp {}, dns {}, tcp {}, finrst {}, syn {}, icmp {}\n",
            secs(Tmo::Generic),
            secs(Tmo::Udp),
            secs(Tmo::Dns),
            secs(Tmo::Tcp),
            secs(Tmo::Finrst),
            secs(Tmo::Syn),
            secs(Tmo::Icmp)
        ));
        s
    }

    /// the global lines of the running configuration: timeouts, pools, rules
    pub(crate) fn ios_nat_config(&self, ni: usize) -> String {
        let Some(n) = self.nat_part(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for k in Tmo::ALL {
            let v = n.timeouts.get(k);
            if v != k.default_secs() {
                let val = if v == NEVER { String::from("never") } else { format!("{}", v) };
                s.push_str(&format!("ip nat translation {} {}\n", k.word(), val));
            }
        }
        for (name, p) in &n.pools {
            s.push_str(&format!("ip nat pool {} {} {} netmask {}\n", name, p.start, p.end, mask_str(p.len)));
        }
        for (_, r) in n.rules.iter().filter(|(_, r)| matches!(r, Rule::List { .. })) {
            if let Rule::List { acl, to, overload } = r {
                let target = match to {
                    Target::Pool(p) => format!("pool {}", p),
                    Target::Iface(i) => format!("interface {}", i),
                };
                s.push_str(&format!("ip nat inside source list {} {}{}\n", acl, target, if *overload { " overload" } else { "" }));
            }
        }
        for (_, r) in &n.rules {
            if let Rule::Static { proto, local, lport, global, gport, extendable } = r {
                let g = match global {
                    Global::Addr(a) => format!("{}", a),
                    Global::Iface(i) => format!("interface {}", i),
                };
                let ext = if *extendable { " extendable" } else { "" };
                if *proto == 0 {
                    s.push_str(&format!("ip nat inside source static {} {}{}\n", local, g, ext));
                } else {
                    let pn = if *proto == PROTO_TCP { "tcp" } else { "udp" };
                    s.push_str(&format!("ip nat inside source static {} {} {} {} {}{}\n", pn, local, lport, g, gport, ext));
                }
            }
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// " ip nat inside" / " ip nat outside" of an interface
    pub(crate) fn ios_nat_if_config(&self, ni: usize, iface: u16) -> String {
        match self.nat_part(ni).and_then(|n| n.side(iface)) {
            Some(Side::Inside) => String::from(" ip nat inside\n"),
            Some(Side::Outside) => String::from(" ip nat outside\n"),
            None => String::new(),
        }
    }

    /// the "debug ip nat" lines of a decision (our words)
    pub(crate) fn nat_debug(&self, note: &Note) -> String {
        let pw = |p: u8| match p {
            0 => String::from("ip"),
            PROTO_TCP => String::from("tcp"),
            PROTO_UDP => String::from("udp"),
            PROTO_ICMP => String::from("icmp"),
            n => format!("protocol {}", n),
        };
        match note {
            Note::NatTranslate { proto, inbound: false, from, to } => format!("NAT: {} source {} -> {}\n", pw(*proto), dep(*from), dep(*to)),
            Note::NatTranslate { proto, inbound: true, from, to } => format!("NAT: {} destination {} -> {}\n", pw(*proto), dep(*from), dep(*to)),
            Note::NatCreate { proto, local, global, remote } => match remote {
                Some(r) => format!("NAT: new translation {} {} = {}, peer {}\n", pw(*proto), dep(*local), dep(*global), dep(*r)),
                None => format!("NAT: new mapping {} {} = {}\n", pw(*proto), dep(*local), dep(*global)),
            },
            Note::NatExpire { proto, local, global, .. } => format!("NAT: translation {} {} = {} timed out\n", pw(*proto), dep(*local), dep(*global)),
            Note::NatQuote { inbound, from, to } => format!("NAT: ICMP error, quoted {} {} -> {}\n", if *inbound { "source" } else { "destination" }, dep(*from), dep(*to)),
            Note::NatNoEntry { dst, .. } => format!("NAT: no translation for {}, passed on\n", dep(*dst)),
            Note::NatNoRule { src } => format!("NAT: no rule for {}, passed on unchanged\n", src),
            Note::NatExhausted { what } => format!("NAT: no free address or port ({}), packet dropped\n", what),
            Note::NatErrorDropped => String::from("NAT: ICMP error about no translation, dropped\n"),
            Note::NatFragWait { id } => format!("NAT: fragment of datagram {} waits for its first part\n", id),
            Note::NatFragDropped { id, parts } => format!("NAT: {} fragments of datagram {} dropped, the first never came\n", parts, id),
            _ => String::new(),
        }
    }
}

/// the extra line of "show ip nat translations verbose" (our words)
fn verbose_line(r: &XlateRow, now: Ns) -> String {
    if !r.dynamic && r.outside.is_none() {
        return String::from("    static, never ends\n");
    }
    let ends = match r.ends {
        Some(t) => format!("ends in {}", hms(t.saturating_sub(now))),
        None => String::from("no timeout"),
    };
    let kind = if r.dynamic { "dynamic" } else { "static" };
    let tcp = match r.tcp {
        Some(t) if t.rst || (t.fin[0] && t.fin[1]) => ", tcp closing",
        Some(t) if t.established => ", tcp established",
        Some(_) => ", tcp opening",
        None => "",
    };
    format!("    created {} ago, used {} ago, {}, {}{}{}\n", hms(now.saturating_sub(r.created)), hms(now.saturating_sub(r.used)), ends, kind, if r.extended { ", with ports" } else { "" }, tcp)
}
