// acl -- access lists at the IOS-like command line: numbered lists ("access-list N ..."), named
// and numbered lists in their own modes ("ip access-list standard|extended NAME": permit, deny,
// remark, sequence numbers, "no" of a line), "ip access-list resequence", "ip access-group"
// on interfaces, "show access-lists", "show ip access-lists", "clear access-list counters", the
// lines of the running configuration and the console messages of "log"; help texts and
// messages are our own words, the syntax, the tables and the configuration lines follow the
// original
//
//   build_acl(tree, modes)               the commands in an IOS command tree (ios.rs) with the
//                                        modes "(config-std-nacl)#" and "(config-ext-nacl)#"
//   Engine::ios_acl(ni, mode, p, line)   the commands of the lists
//   Engine::ios_if_filter(ni, iface, p)  "ip access-group" (and "ip nat inside|outside",
//                                        cli/nat.rs) in interface mode
//   Engine::ios_acl_config(ni)           named lists, then the numbered ones (after the routes)
//   Engine::ios_access_group_config(ni, iface)   the interface's "ip access-group" lines
//   Engine::acl_log(..)                  the console message of a rule with "log"
//
// The tree: a rule of an extended list is built once per mode below "permit"; "deny", the
// other protocols of a kind (ip, gre, a number ...), the second range of list numbers and the
// lines with a sequence number reach the same words through Tree::also, and so do the source
// and destination forms (any, host A, A W) and the port operators (eq, neq, gt, lt, range with
// numbers or the names of the protocol).  A numbered list made in the global configuration
// grows line by line; "no access-list N" with anything after it removes the whole list, as on
// the original.  In a list's mode a line takes the number 10 after the last unless it starts
// with one; "no N", and "no" with the words of a line, remove that line.  The running
// configuration writes no sequence numbers: a list read back is numbered 10, 20, ... again.

use alloc::format;
use alloc::string::{String, ToString};

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::Flow;
use netpkt::Ipv4;

use super::ios::K;
use super::Mode;
use crate::acl::{port_number, proto_text, Ace, AclKind, AclOp, Item, Ports, Wild, ICMP_TYPES, PROTOCOLS, TCP_PORTS, UDP_PORTS};
use crate::action::{ActErr, Action};
use crate::kernel::{Engine, Ns, SEC};

/// a rule with "log" reports again after this long at the earliest
pub const LOG_EVERY: Ns = 300 * SEC;
/// the longest remark
const REMARK_MAX: usize = 100;

const NEGATE: &str = "Undo a setting or restore its default";
const LOG: &str = "Report matches on the console";

/// the kinds of rule words after the protocol
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fam {
    Tcp,
    Udp,
    Icmp,
    Plain,
}

/// `pre` and `rest` as one path
fn sp(pre: &str, rest: &str) -> String {
    if pre.is_empty() { String::from(rest) } else { format!("{} {}", pre, rest) }
}

/// the commands of access lists in an IOS tree; `m`: user, privileged, global configuration,
/// interface and subinterface mode
pub(crate) fn build_acl(t: &mut Tree<K>, m: [ModeId; 5]) {
    let [user, prv, conf, ifm, sub] = m;
    let std = t.mode("(config-std-nacl)#", "Commands for this standard access list:", Some(conf), Some(NEGATE));
    let ext = t.mode("(config-ext-nacl)#", "Commands for this extended access list:", Some(conf), Some(NEGATE));

    // ---- EXEC
    for x in [user, prv] {
        for pre in ["show access-lists", "show ip access-lists"] {
            t.add(x, pre, "The access lists with their hit counters", Some(K::ShowAcl), 0);
            t.add(x, &format!("{} <1-2699>=acl", pre), "Only the list with this number", Some(K::ShowAcl), 0);
            t.add(x, &format!("{} WORD=acl", pre), "Only the list with this name", Some(K::ShowAcl), 0);
        }
    }
    t.add(prv, "clear access-list", "Counters of access lists", None, 0);
    t.add(prv, "clear access-list counters", "Set the hit counters back to 0", Some(K::ClearAcl), 0);
    t.add(prv, "clear access-list counters <1-2699>=acl", "Only the list with this number", Some(K::ClearAcl), 0);
    t.add(prv, "clear access-list counters WORD=acl", "Only the list with this name", Some(K::ClearAcl), 0);

    // ---- numbered lists in the global configuration
    t.add(conf, "access-list", "Add a line to a numbered access list", None, F_NO);
    let ranges: [(&str, &str, bool, Option<&str>); 4] = [
        ("<1-99>", "Standard list: source addresses", false, None),
        ("<100-199>", "Extended list: protocols, addresses, ports", true, None),
        ("<1300-1999>", "Standard list, second range of numbers", false, Some("access-list <1-99>=num")),
        ("<2000-2699>", "Extended list, second range of numbers", true, Some("access-list <100-199>=num")),
    ];
    for (range, help, extended, base) in ranges {
        let pre = format!("access-list {}=num", range);
        t.add(conf, &pre, help, None, 0);
        t.add_no(conf, &pre, if extended { K::AclExt } else { K::AclStd });
        t.add(conf, &sp(&pre, "remark"), "A comment line in the list", None, 0);
        t.add(conf, &sp(&pre, "remark LINE=rem"), "The comment, up to 100 characters", Some(K::AclRemark), 0);
        actions(t, conf, &pre, extended, base);
    }

    // ---- lists in their own modes
    t.add(conf, "ip access-list", "Set up an access list in its own mode", None, 0);
    for (kw, h, nums) in [
        ("standard", "A list of source addresses", [("<1-99>=name", "Number of a standard list"), ("<1300-1999>=name", "Number of a standard list, second range")]),
        ("extended", "A list of protocols, addresses and ports", [("<100-199>=name", "Number of an extended list"), ("<2000-2699>=name", "Number of an extended list, second range")]),
    ] {
        let pre = format!("ip access-list {}", kw);
        t.add(conf, &pre, h, None, 0);
        for (n, nh) in nums {
            t.add(conf, &sp(&pre, n), nh, Some(K::AclNamed), 0);
        }
        t.add(conf, &sp(&pre, "WORD=name"), "Name of the list", Some(K::AclNamed), 0);
    }
    t.add(conf, "ip access-list resequence", "Number the lines of a list anew", None, 0);
    for n in ["<1-2699>=name", "WORD=name"] {
        let pre = sp("ip access-list resequence", n);
        t.add(conf, &pre, if n.starts_with('<') { "Number of the list" } else { "Name of the list" }, None, 0);
        t.add(conf, &sp(&pre, "<1-2147483647>=start"), "Number of the first line", None, 0);
        t.add(conf, &sp(&pre, "<1-2147483647>=start <1-2147483647>=step"), "Step between two lines", Some(K::AclReseq), 0);
    }
    for (mm, extended) in [(std, false), (ext, true)] {
        t.add(mm, "do", "Run an EXEC command from here", None, 0);
        t.add(mm, "do LINE=cmd", "The command", Some(K::Do), 0);
        t.add(mm, "exit", "Back to the device configuration", Some(K::Exit), 0);
        t.add(mm, "end", "Finish configuring", Some(K::End), F_HIDDEN);
        t.add(mm, "remark", "A comment line in the list", None, F_NO);
        t.add(mm, "remark LINE=rem", "The comment, up to 100 characters", Some(K::AclRemark), 0);
        actions(t, mm, "", extended, None);
        let seq = "<1-2147483647>=seq";
        t.add(mm, seq, "Number of the line (its place in the list)", None, F_NO);
        t.add_no(mm, seq, K::AclSeqNo);
        actions(t, mm, seq, extended, Some(""));
    }

    // ---- interfaces
    for x in [ifm, sub] {
        t.add(x, "ip access-group", "Filter the packets of this interface with an access list", None, 0);
        for (acl, h) in [("<1-199>=acl", "Number of the list"), ("<1300-2699>=acl", "Number of the list, second range"), ("WORD=acl", "Name of the list")] {
            let pre = sp("ip access-group", acl);
            t.add(x, &pre, h, None, 0);
            t.add(x, &sp(&pre, "in=dir"), "Packets that come in", Some(K::AccessGroup), 0);
            t.add(x, &sp(&pre, "out=dir"), "Packets that go out", Some(K::AccessGroup), 0);
        }
    }
}

/// "permit" and "deny" after `pre`: the words of a rule below "permit", or (`base`) the ones
/// below the "permit" after `base`
fn actions(t: &mut Tree<K>, m: ModeId, pre: &str, extended: bool, base: Option<&str>) {
    let cmd = if extended { K::AclExt } else { K::AclStd };
    // in a list's mode they are first words and have a "no"
    let flag = if pre.is_empty() { F_NO } else { 0 };
    let permit = sp(pre, "permit=act");
    let deny = sp(pre, "deny=act");
    t.add(m, &permit, "Let matching packets through", None, flag);
    t.add(m, &deny, "Drop matching packets", None, flag);
    match base {
        Some(b) => {
            let target = sp(b, "permit=act");
            t.also(m, &permit, &target);
            t.also(m, &deny, &target);
        }
        None => {
            if extended {
                ext_body(t, m, &permit, cmd);
            } else {
                std_body(t, m, &permit, cmd);
            }
            t.also(m, &deny, &permit);
        }
    }
}

/// a standard rule: the source, "log"
fn std_body(t: &mut Tree<K>, m: ModeId, p: &str, cmd: K) {
    let a = |t: &mut Tree<K>, path: &str, h: &str, c: Option<K>| {
        t.add(m, &sp(p, path), h, c, 0);
    };
    a(t, "any=s", "Every source address", Some(cmd));
    a(t, "any=s log", LOG, Some(cmd));
    a(t, "host=s", "One source address", None);
    a(t, "host=s A.B.C.D=sh", "The address", Some(cmd));
    a(t, "host=s A.B.C.D=sh log", LOG, Some(cmd));
    a(t, "A.B.C.D=sa", "Source address, or network with wildcard bits", Some(cmd));
    a(t, "A.B.C.D=sa log", LOG, Some(cmd));
    a(t, "A.B.C.D=sa A.B.C.D=sw", "Wildcard bits (set bits may differ)", Some(cmd));
    a(t, "A.B.C.D=sa A.B.C.D=sw log", LOG, Some(cmd));
}

/// an extended rule: the protocol, then its kind of words
fn ext_body(t: &mut Tree<K>, m: ModeId, p: &str, cmd: K) {
    let num = sp(p, "<0-255>=proto");
    t.add(m, &num, "IP protocol number", None, 0);
    fam(t, m, &num, Fam::Plain, cmd);
    t.add(m, &sp(p, "ip=proto"), "Every IP protocol", None, 0);
    t.also(m, &sp(p, "ip=proto"), &num);
    for (name, n, h) in PROTOCOLS {
        let path = sp(p, &format!("{}=proto", name));
        t.add(m, &path, h, None, 0);
        match *n {
            PROTO_TCP => fam(t, m, &path, Fam::Tcp, cmd),
            PROTO_UDP => fam(t, m, &path, Fam::Udp, cmd),
            PROTO_ICMP => fam(t, m, &path, Fam::Icmp, cmd),
            _ => t.also(m, &path, &num),
        }
    }
}

/// the words after a protocol: source, (ports), destination, (ports), options
fn fam(t: &mut Tree<K>, m: ModeId, pf: &str, f: Fam, cmd: K) {
    let s1 = sp(pf, "any=s");
    t.add(m, &s1, "Every source address", None, 0);
    t.add(m, &sp(pf, "host=s"), "One source address", None, 0);
    let s2 = sp(pf, "host=s A.B.C.D=sh");
    t.add(m, &s2, "The source address", None, 0);
    t.add(m, &sp(pf, "A.B.C.D=sa"), "Source network", None, 0);
    let s3 = sp(pf, "A.B.C.D=sa A.B.C.D=sw");
    t.add(m, &s3, "Wildcard bits of the source (set bits may differ)", None, 0);
    t.also(m, &s2, &s1);
    t.also(m, &s3, &s1);
    let d1 = dst_forms(t, m, &s1, None, cmd);
    below_dst(t, m, &d1, f, cmd);
    if matches!(f, Fam::Tcp | Fam::Udp) {
        // the source port, then the same destinations
        let spc = port_ops(t, m, &s1, "s", f, None);
        dst_forms(t, m, &spc, Some(&d1), cmd);
    }
}

/// the destination forms after `at`; with `link` they lead to the words after that one (the
/// first destination path); the path of the canonical "any"
fn dst_forms(t: &mut Tree<K>, m: ModeId, at: &str, link: Option<&str>, cmd: K) -> String {
    let d1 = sp(at, "any=d");
    t.add(m, &d1, "Every destination address", Some(cmd), 0);
    t.add(m, &sp(at, "host=d"), "One destination address", None, 0);
    let d2 = sp(at, "host=d A.B.C.D=dh");
    t.add(m, &d2, "The destination address", Some(cmd), 0);
    t.add(m, &sp(at, "A.B.C.D=da"), "Destination network", None, 0);
    let d3 = sp(at, "A.B.C.D=da A.B.C.D=dw");
    t.add(m, &d3, "Wildcard bits of the destination (set bits may differ)", Some(cmd), 0);
    let target = String::from(link.unwrap_or(&d1));
    if link.is_some() {
        t.also(m, &d1, &target);
    }
    t.also(m, &d2, &target);
    t.also(m, &d3, &target);
    target
}

/// what may follow the destination
fn below_dst(t: &mut Tree<K>, m: ModeId, d1: &str, f: Fam, cmd: K) {
    match f {
        Fam::Tcp | Fam::Udp => {
            let dpc = port_ops(t, m, d1, "d", f, Some(cmd));
            options(t, m, d1, f, cmd);
            options(t, m, &dpc, f, cmd);
        }
        Fam::Icmp => {
            let ty = sp(d1, "<0-255>=it");
            t.add(m, &ty, "ICMP message type", Some(cmd), 0);
            t.add(m, &sp(&ty, "<0-255>=ic"), "ICMP message code", Some(cmd), 0);
            t.add(m, &sp(&ty, "<0-255>=ic log"), LOG, Some(cmd), 0);
            t.add(m, &sp(&ty, "log"), LOG, Some(cmd), 0);
            for (name, _, _, h) in ICMP_TYPES {
                let n = sp(d1, &format!("{}=in", name));
                t.add(m, &n, h, Some(cmd), 0);
                t.add(m, &sp(&n, "log"), LOG, Some(cmd), 0);
            }
            t.add(m, &sp(d1, "log"), LOG, Some(cmd), 0);
        }
        Fam::Plain => {
            t.add(m, &sp(d1, "log"), LOG, Some(cmd), 0);
        }
    }
}

/// "established" and "log" after `at`
fn options(t: &mut Tree<K>, m: ModeId, at: &str, f: Fam, cmd: K) {
    if f == Fam::Tcp {
        t.add(m, &sp(at, "established=est"), "Only segments of connections that exist (ACK or RST set)", Some(cmd), 0);
        t.add(m, &sp(at, "established=est log"), LOG, Some(cmd), 0);
    }
    t.add(m, &sp(at, "log"), LOG, Some(cmd), 0);
}

/// the port operators after `at` (`side` "s" or "d" for the keys); every port leads to the
/// words after the canonical one ("eq" with a number), whose path is returned
fn port_ops(t: &mut Tree<K>, m: ModeId, at: &str, side: &str, f: Fam, end: Option<K>) -> String {
    let names = if f == Fam::Tcp { TCP_PORTS } else { UDP_PORTS };
    let (ok, pk, pk2) = (format!("{}o", side), format!("{}p", side), format!("{}p2", side));
    let canon = sp(at, &format!("eq={} <0-65535>={}", ok, pk));
    for (op, h) in [("eq", "This port only"), ("neq", "Every port but this one"), ("gt", "Ports above this one"), ("lt", "Ports below this one")] {
        let opn = sp(at, &format!("{}={}", op, ok));
        t.add(m, &opn, h, None, 0);
        let num = sp(&opn, &format!("<0-65535>={}", pk));
        t.add(m, &num, "Port number", end, 0);
        if num != canon {
            t.also(m, &num, &canon);
        }
        for (name, _, nh) in names {
            let n = sp(&opn, &format!("{}={}", name, pk));
            t.add(m, &n, nh, end, 0);
            t.also(m, &n, &canon);
        }
    }
    let rn = sp(at, &format!("range={}", ok));
    t.add(m, &rn, "A range of ports, both ends included", None, 0);
    let first = sp(&rn, &format!("<0-65535>={}", pk));
    t.add(m, &first, "First port", None, 0);
    for (name, _, nh) in names {
        let n = sp(&rn, &format!("{}={}", name, pk));
        t.add(m, &n, nh, None, 0);
        t.also(m, &n, &first);
    }
    let last = sp(&first, &format!("<0-65535>={}", pk2));
    t.add(m, &last, "Last port", end, 0);
    t.also(m, &last, &canon);
    for (name, _, nh) in names {
        let n = sp(&first, &format!("{}={}", name, pk2));
        t.add(m, &n, nh, end, 0);
        t.also(m, &n, &canon);
    }
    canon
}

/// the list a command names ("show access-lists 101", "... WEB")
fn list_arg(p: &Parsed<K>, key: &str) -> Option<String> {
    p.num(key).map(|n| n.to_string()).or_else(|| p.text(key).map(String::from))
}

/// a port of a rule: a number or a name of the protocol
fn port_of(p: &Parsed<K>, key: &str, proto: u8) -> Option<u16> {
    match p.num(key) {
        Some(n) => Some(n as u16),
        None => port_number(proto, p.text(key)?),
    }
}

/// the port condition of one side
fn ports_of(p: &Parsed<K>, side: &str, proto: u8) -> Ports {
    let (ok, pk, pk2) = (format!("{}o", side), format!("{}p", side), format!("{}p2", side));
    let a = port_of(p, &pk, proto).unwrap_or(0);
    match p.text(&ok) {
        Some("eq") => Ports::Eq(a),
        Some("neq") => Ports::Neq(a),
        Some("gt") => Ports::Gt(a),
        Some("lt") => Ports::Lt(a),
        Some("range") => {
            let b = port_of(p, &pk2, proto).unwrap_or(a);
            Ports::Range(a.min(b), a.max(b))
        }
        _ => Ports::Any,
    }
}

/// an address condition: "any", "host A" or "A W" (`side` "s" or "d")
fn wild_of(p: &Parsed<K>, side: &str) -> Wild {
    match p.text(side) {
        Some("any") => Wild::ANY,
        Some("host") => Wild::host(Ipv4(p.ip(&format!("{}h", side)).unwrap_or([0; 4]))),
        _ => {
            let a = Ipv4(p.ip(&format!("{}a", side)).unwrap_or([0; 4]));
            match p.ip(&format!("{}w", side)) {
                Some(w) => Wild::new(a, Ipv4(w)),
                None => Wild::host(a),
            }
        }
    }
}

/// the rule of a parsed line
fn rule_of(p: &Parsed<K>, kind: AclKind) -> Ace {
    let permit = p.text("act") == Some("permit");
    if kind == AclKind::Standard {
        let mut a = Ace::standard(permit, wild_of(p, "s"));
        a.log = p.has("log");
        return a;
    }
    let proto = match p.text("proto") {
        Some("ip") => None,
        Some(name) => PROTOCOLS.iter().find(|x| x.0 == name).map(|x| x.1),
        None => p.num("proto").map(|n| n as u8),
    };
    let l4 = proto.unwrap_or(0);
    let icmp = match (p.num("it"), p.text("in")) {
        (Some(ty), _) => Some((ty as u8, p.num("ic").map(|c| c as u8))),
        (None, Some(name)) => ICMP_TYPES.iter().find(|x| x.0 == name).map(|x| (x.1, x.2)),
        _ => None,
    };
    Ace { permit, proto, src: wild_of(p, "s"), sport: ports_of(p, "s", l4), dst: wild_of(p, "d"), dport: ports_of(p, "d", l4), established: p.has("est"), icmp, log: p.has("log") }
}

impl Engine {
    /// the commands of access lists
    pub(crate) fn ios_acl(&mut self, ni: usize, mode: Mode, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match p.cmd {
            K::ShowAcl => {
                let text = self.show_access_lists(ni, list_arg(p, "acl").as_deref());
                self.out(ni, &text);
                Ok(())
            }
            K::ClearAcl => {
                let _ = self.apply(Action::AclCounters { dev, list: list_arg(p, "acl") });
                Ok(())
            }
            K::AclNamed => {
                let kind = if p.has("standard") { AclKind::Standard } else { AclKind::Extended };
                let (name, numbered) = match p.num("name") {
                    Some(n) => (n.to_string(), true),
                    None => {
                        let w = p.text("name").unwrap_or("");
                        if w.bytes().all(|b| b.is_ascii_digit()) {
                            return self.ios_err(ni, &format!("% {} is no number of a {} list\n", w, kind.word()));
                        }
                        (String::from(w), false)
                    }
                };
                if p.no {
                    let _ = self.apply(Action::Acl { dev, list: name, op: AclOp::Remove });
                    return Ok(());
                }
                match self.apply(Action::Acl { dev, list: name.clone(), op: AclOp::Create { kind, numbered } }) {
                    Ok(()) => {
                        if let Some(c) = self.cdev_mut(ni) {
                            c.mode = if kind == AclKind::Standard { Mode::AclStd } else { Mode::AclExt };
                            c.acl = name;
                        }
                        Ok(())
                    }
                    Err(_) => {
                        let other = if kind == AclKind::Standard { "an extended" } else { "a standard" };
                        self.ios_err(ni, &format!("% {} is already {} access list\n", name, other))
                    }
                }
            }
            K::AclReseq => {
                let name = list_arg(p, "name").unwrap_or_default();
                let (start, step) = (p.num("start").unwrap_or(10) as u32, p.num("step").unwrap_or(10) as u32);
                match self.apply(Action::Acl { dev, list: name.clone(), op: AclOp::Resequence { start, step } }) {
                    Ok(()) => Ok(()),
                    Err(ActErr::Refused(_)) if !self.stack(ni).is_some_and(|s| s.acls.lists.contains_key(&name)) => self.ios_err(ni, &format!("% Access list {} does not exist\n", name)),
                    Err(_) => self.ios_err(ni, "% The numbers would grow too large\n"),
                }
            }
            _ => self.ios_acl_line(ni, mode, p, line),
        }
    }

    /// a line of a list: a rule, a remark, "no" of one; in the global configuration or in a
    /// list's mode
    fn ios_acl_line(&mut self, ni: usize, mode: Mode, p: &Parsed<K>, _line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let (name, kind, numbered, global) = match p.num("num") {
            Some(n) => {
                let Some(k) = AclKind::of_number(n as u32) else {
                    return Ok(());
                };
                (n.to_string(), k, true, true)
            }
            None => {
                let name = self.cdev(ni).map(|c| c.acl.clone()).unwrap_or_default();
                let kind = if mode == Mode::AclStd { AclKind::Standard } else { AclKind::Extended };
                let numbered = self.stack(ni).and_then(|s| s.acls.lists.get(&name)).is_some_and(|l| l.numbered);
                (name, kind, numbered, false)
            }
        };
        // "no access-list N ..." removes the whole numbered list, whatever follows
        if global && p.no {
            let _ = self.apply(Action::Acl { dev, list: name, op: AclOp::Remove });
            return Ok(());
        }
        let item = match p.cmd {
            K::AclRemark => {
                let text = String::from(p.text("rem").unwrap_or("").trim());
                if text.chars().count() > REMARK_MAX {
                    return self.ios_err(ni, "% A remark has at most 100 characters\n");
                }
                Some(Item::Remark(text))
            }
            K::AclStd | K::AclExt => Some(Item::Rule(rule_of(p, kind))),
            _ => None,
        };
        let seq = p.num("seq").map(|s| s as u32);
        if p.no {
            // a line's number or its words
            let op = match (seq, item) {
                (Some(s), _) => AclOp::RemoveSeq(s),
                (None, Some(it)) => AclOp::RemoveItem(it),
                (None, None) => return Ok(()),
            };
            let _ = self.apply(Action::Acl { dev, list: name, op });
            return Ok(());
        }
        let Some(item) = item else {
            return Ok(());
        };
        // what the list refuses, in the console's words
        if let Some(l) = self.stack(ni).and_then(|s| s.acls.lists.get(&name)) {
            if l.kind != kind {
                let other = if kind == AclKind::Standard { "an extended" } else { "a standard" };
                return self.ios_err(ni, &format!("% {} is already {} access list\n", name, other));
            }
            let rules = || l.lines.iter().filter(|x| matches!(x.item, Item::Rule(_)));
            if let Some(s) = seq
                && matches!(item, Item::Rule(_))
                && rules().any(|x| x.seq == s)
            {
                return self.ios_err(ni, "% Duplicate sequence number\n");
            }
            if seq.is_none() && matches!(item, Item::Rule(_)) && rules().map(|x| x.seq as u64).max().unwrap_or(0) + 10 > crate::acl::SEQ_MAX as u64 {
                return self.ios_err(ni, "% No sequence number left after the last line\n");
            }
        }
        let _ = self.apply(Action::Acl { dev, list: name, op: AclOp::Add { kind, numbered, seq, item } });
        Ok(())
    }

    /// "ip access-group" and "ip nat inside|outside" for one interface
    pub(crate) fn ios_if_filter(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        if self.stack(ni).and_then(|s| s.iface_by_name(name)).is_none() {
            let pending = self.cdev(ni).is_some_and(|c| c.cfg.pending.iter().any(|x| x == name));
            return self.ios_err(ni, if pending { "% Give the subinterface its VLAN first (encapsulation dot1Q)\n" } else { "% Only routed interfaces take this, not a switch port\n" });
        }
        match p.cmd {
            K::AccessGroup => {
                let out = p.text("dir") == Some("out");
                let list = if p.no { None } else { list_arg(p, "acl") };
                let _ = self.apply(Action::AccessGroup { dev, iface: String::from(name), out, list });
                Ok(())
            }
            _ => self.ios_nat_side(ni, name, p),
        }
    }

    /// what "show access-lists" prints (all lists, or one)
    pub(crate) fn show_access_lists(&self, ni: usize, only: Option<&str>) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        for l in self.access_lists(dev) {
            if only.is_some_and(|o| o != l.name) {
                continue;
            }
            let kind = if l.kind == AclKind::Standard { "Standard" } else { "Extended" };
            s.push_str(&format!("{} IP access list {}\n", kind, l.name));
            for (seq, text, hits) in &l.rules {
                let n = match hits {
                    0 => String::new(),
                    1 => String::from(" (1 match)"),
                    h => format!(" ({} matches)", h),
                };
                s.push_str(&format!("    {} {}{}\n", seq, text, n));
            }
        }
        s
    }

    /// the access lists of the running configuration: named lists with their lines, then the
    /// lines of the numbered ones
    pub(crate) fn ios_acl_config(&self, ni: usize) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let mut named = String::new();
        let mut numbered = String::new();
        for name in st.acls.names() {
            let Some(l) = st.acls.lists.get(&name) else {
                continue;
            };
            let text = |it: &Item| match it {
                Item::Rule(r) => r.config(l.kind),
                Item::Remark(t) => format!("remark {}", t),
            };
            if l.numbered {
                for line in &l.lines {
                    numbered.push_str(&format!("access-list {} {}\n", name, text(&line.item)));
                }
            } else {
                named.push_str(&format!("ip access-list {} {}\n", l.kind.word(), name));
                for line in &l.lines {
                    named.push_str(&format!(" {}\n", text(&line.item)));
                }
                named.push_str("!\n");
            }
        }
        if !numbered.is_empty() {
            numbered.push_str("!\n");
        }
        named + &numbered
    }

    /// an interface's "ip access-group" lines
    pub(crate) fn ios_access_group_config(&self, ni: usize, iface: u16) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for out in [false, true] {
            if let Some(l) = st.acls.bound.get(&(iface, out)) {
                s.push_str(&format!(" ip access-group {} {}\n", l, if out { "out" } else { "in" }));
            }
        }
        s
    }

    /// the console message of a rule with "log" (`n` packets since the last one)
    pub(crate) fn acl_log(&mut self, ni: usize, list: &str, r: &Ace, f: &Flow, n: u64) {
        if self.cdev(ni).is_none_or(|c| c.style != jfn_model::catalog::CliStyle::Ios) {
            return;
        }
        let verb = if r.permit { "lets through" } else { "drops" };
        let pk = if n == 1 { String::from("1 packet") } else { format!("{} packets", n) };
        let standard = r.proto.is_none() && r.dst.is_any() && r.sport == Ports::Any && r.dport == Ports::Any && !r.established && r.icmp.is_none();
        let text = if standard && self.stack(ni).and_then(|s| s.acls.lists.get(list)).is_some_and(|l| l.kind == AclKind::Standard) {
            format!("%SEC-6-IPACCESSLOGS: list {} {} {} ({})", list, verb, f.src, pk)
        } else if f.ports && matches!(f.proto, PROTO_TCP | PROTO_UDP) {
            format!("%SEC-6-IPACCESSLOGP: list {} {} {} from {} port {} to {} port {} ({})", list, verb, proto_text(Some(f.proto)), f.src, f.sport, f.dst, f.dport, pk)
        } else if let Some((ty, code)) = f.icmp {
            format!("%SEC-6-IPACCESSLOGDP: list {} {} icmp from {} to {}, type {} code {} ({})", list, verb, f.src, f.dst, ty, code, pk)
        } else {
            format!("%SEC-6-IPACCESSLOGNP: list {} {} {} from {} to {} ({})", list, verb, proto_text(Some(f.proto)), f.src, f.dst, pk)
        };
        self.ios_log(ni, &text);
    }
}
