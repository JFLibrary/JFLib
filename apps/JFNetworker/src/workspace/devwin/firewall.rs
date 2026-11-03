// firewall -- the Firewall tab of a device window (IOS-like devices that route: firewalls,
// routers, L3 switches): a surface over the engine's access lists and NAT.  Forms that type
// command lines on the device's console (doc/console.rs run_lines_running: they show there and
// land in the startup configuration as one undo step); what it shows is read back from the
// engine, so a rule typed on the console appears at once
//
//   Form::new()                  nothing chosen
//   tab(n, ui, k, dev)           the parts Regeln (what protects which interface, a list, its rules
//                                in a grid, the rule form), NAT (sides, pools, dynamic and static
//                                rules, the running translations), Test (which rule meets a
//                                packet); on firewall models also Zonen and Sitzungen (fwzones.rs);
//                                below them the refusal and Befehle, the lines typed last
//
// What the forms make (pure, for the tests in fwguitest.rs):
//   wild_of(text) / wild_text(&Wild)     an address field: any, 10.0.0.5, 10.0.0.0/24, A W
//   port_of(proto, text)                 a port field: a number or a name of the protocol
//   rule_of_form(kind, &RuleForm, dev)   the fields as a rule of the engine (Err: why not); a raw
//                                        line is read by a scratch console (parse_line)
//   form_of(&Ace, kind) / expressible(&Ace)   a rule back in the fields; a rule the fields cannot
//                                        hold (a protocol number, an ICMP type and code without
//                                        a name, a net mask as wildcard) stays a raw line
//   rows_of(&Acl)                        the grid's rows, with the implicit deny as the last one
//   add_lines / edit_lines / move_lines / delete_lines   what a list's change types (change_lines)
//   new_list_lines / del_list_lines / bind_lines         lists and "ip access-group"
//   side_lines / pool_lines / pool_del_lines / dyn_lines / static_lines / rule_del_line   NAT
//   flow_of(&TestForm) / told(&Acls, iface, name, out, &Flow)   the Test part: a packet as the engine
//                                        reads one and what Acls::probe says about it, the
//                                        function the engine's own filter acts on
//   refusal(routes, locked, dev)         why the device's filters cannot be changed now
//
// A change of a list keeps what it can (change_lines): lines only added at the end need no
// numbers; lines only removed go by their words; one rule that goes to another place or changes
// goes "no N", "M rule" with M between its neighbours (ip access-list resequence N 10 10 first
// unless the rules are numbered 10, 20, ..., because a scratch engine reads the running
// configuration, which has no numbers) -- only that rule's hit counter starts over, as it would
// on the command line.  A new rule has no way to the place below a remark, and a remark no way
// into the middle of a list, except by typing what follows again ("no" and the line); then these
// rules' counters start over as well.  A rule moves with the remarks just above it.  Undo restarts
// the device with the old configuration, so all counters and translations start over.
//
// While the view stands in the past (K1), and on a device without power, everything shows but
// nothing changes.  A device without routing ("no ip routing", an L3 switch before "ip routing")
// refuses a change with the reason and offers to switch routing on.
//
// Ids: ID_FW0 + 0x800 k (window k): + 0 the parts (+ 1 + i each), + 4 .. 6 their scroll views, + 7 the
// console, + 8 the lines' scroll view; Regeln: + 0x10 the list, + 0x11 Neue Liste, + 0x12 Liste
// löschen, + 0x13 Zähler löschen, + 0x14 alle Schnittstellen, + 0x15 name and + 0x16 kind of a new
// list (+ 0x17 Anlegen, + 0x18 Abbrechen), + 0x19 Routing einschalten, + 0x1A Neu, + 0x1B Bearbeiten,
// + 0x1C Duplizieren, + 0x1D nach oben, + 0x1E nach unten, + 0x1F Löschen; the rule form + 0x20
// Aktion, + 0x21 Protokoll, + 0x22 Quelle, + 0x23 .. 0x25 its port, + 0x26 Ziel, + 0x27 .. 0x29 its
// port, + 0x2A established, + 0x2B ICMP, + 0x2C Log, + 0x2D Bemerkung, + 0x2E als Zeile, + 0x2F the
// line, + 0x30 Übernehmen, + 0x31 Abbrechen; NAT: + 0x40 .. 0x44 a pool, + 0x45 .. 0x4A a dynamic
// rule, + 0x4B .. 0x52 a static one, + 0x53 clear; Test: + 0x60 Schnittstelle, + 0x61 Richtung, + 0x62
// Protokoll, + 0x63 Quelle, + 0x64 Ziel, + 0x65 Quellport, + 0x66 Zielport, + 0x67 ACK, + 0x68 ICMP,
// + 0x69 Prüfen; + 0x70 the parts of a firewall model (+ 1 + i each), + 0x78, + 0x79 the scroll views of
// Zonen and Sitzungen; + 0x100 + 2 i the lists of the i-th interface shown (+ 1 outgoing), + 0x180 + i its NAT
// side, + 0x1C0 + i, + 0x1E0 + i, + 0x200 + i what removes a pool, a dynamic rule, a static rule; + 0x400 the
// rules' grid and + 0x600 the translations' grid (rows + 1 + i, at most ROWS_MAX).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::F_FIREWALL;
use jfn_model::ifname;
use jfn_model::Project;
use jfn_sim::acl::{icmp_text, port_number, proto_text, Ace, Acl, AclKind, Acls, Item, Ports, Probe, Wild, ICMP_TYPES, PROTOCOLS};
use jfn_sim::nat::{Global, Pool, Rule, Side, Target};
use jfn_sim::fw::{Firewall, FwRow, FwTest};
use jfn_sim::{Action, Engine};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::{is_query, Flow};
use netpkt::tcp::{ACK, SYN};
use netpkt::{Ipv4, Prefix4};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use super::fwzones::{self, ZForm};
use crate::doc::console;

/// rows a grid here shows at most (its row ids must stay in the tab's block)
pub(super) const ROWS_MAX: usize = 500;
/// the key of the implicit deny's row
const KEY_IMPLICIT: u64 = u64::MAX;
/// the key of a remark's row: this bit, its line (a rule's key is its sequence number)
const KEY_REMARK: u64 = 1 << 40;

// i18n
const PORT_OPS: [&str; 6] = ["alle", "eq (gleich)", "neq (ungleich)", "gt (größer)", "lt (kleiner)", "range (Bereich)"];
// i18n
const ACTIONS: [&str; 2] = ["erlauben (permit)", "verwerfen (deny)"];
// i18n
const DIRECTIONS: [&str; 2] = ["eingehend", "ausgehend"];

// ---------------------------------------------------------------- the fields

/// the fields of the rule form, as typed
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuleForm {
    /// 0 permit, 1 deny
    pub act: i32,
    /// an index into protos()
    pub proto: i32,
    pub src: String,
    /// an index into PORT_OPS and the port or the range's two ends
    pub sop: i32,
    pub sp1: String,
    pub sp2: String,
    pub dst: String,
    pub dop: i32,
    pub dp1: String,
    pub dp2: String,
    pub established: i32,
    /// 0 every message, 1 + i ICMP_TYPES[i]
    pub icmp: i32,
    pub log: i32,
    pub remark: String,
    /// the rule is the raw line, not the fields
    pub use_raw: i32,
    pub raw: String,
}

impl RuleForm {
    pub(crate) fn new() -> RuleForm {
        RuleForm { act: 0, proto: 0, src: String::from("any"), sop: 0, sp1: String::new(), sp2: String::new(), dst: String::from("any"), dop: 0, dp1: String::new(), dp2: String::new(), established: 0, icmp: 0, log: 0, remark: String::new(), use_raw: 0, raw: String::new() }
    }
}

/// the rule form is open: for what
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleMode {
    /// a new rule, after this rule (None: at the end)
    Add(Option<u32>),
    /// this rule changes
    Edit(u32),
}

/// the fields of the Test part
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestForm {
    /// an index into test_protos()
    pub proto: i32,
    pub src: String,
    pub dst: String,
    pub sport: String,
    pub dport: String,
    /// a segment of a connection that exists (ACK), else a new one (SYN)
    pub ack: i32,
    /// an index into ICMP_TYPES
    pub icmp: i32,
}

impl TestForm {
    pub(crate) fn new() -> TestForm {
        // (echo: the ping of the table)
        let icmp = ICMP_TYPES.iter().position(|x| x.1 == 8).unwrap_or(0) as i32;
        TestForm { proto: 0, src: String::new(), dst: String::new(), sport: String::new(), dport: String::new(), ack: 0, icmp }
    }
}

/// the fields of a static translation
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StaticForm {
    /// 0 the address, 1 one TCP port, 2 one UDP port
    pub kind: i32,
    pub local: String,
    pub lport: String,
    /// 0 an address, 1 an interface's
    pub gtype: i32,
    pub global: String,
    pub gif: i32,
    pub gport: String,
}

/// the state of the tab
pub(crate) struct Form {
    /// the fields of Zonen and Sitzungen (fwzones.rs)
    pub(super) z: ZForm,
    /// the device the fields belong to
    dev: u32,
    /// the part shown: 0 Regeln, 1 NAT, 2 Test, 3 Zonen, 4 Sitzungen
    part: i32,
    /// the chosen list
    list: String,
    grid: GridState,
    /// the rule typed last gets chosen when it is there (its configuration line)
    pick: Option<String>,
    all_ifs: i32,
    /// the fields of a new list
    new_open: bool,
    new_name: String,
    new_kind: i32,
    rule: Option<RuleMode>,
    fields: RuleForm,
    /// the form just opened: passes until it is brought into view
    reveal: u8,
    pool: [String; 4],
    dyn_list: i32,
    dyn_to: i32,
    dyn_if: i32,
    dyn_pool: i32,
    dyn_over: i32,
    stat: StaticForm,
    xlate: GridState,
    test_if: i32,
    test_out: i32,
    test: TestForm,
    /// what was asked last: interface, direction and the packet
    asked: Option<(u16, bool, Flow)>,
    /// why the last form was refused
    err: String,
    /// the lines the last form typed
    ran: Vec<String>,
}

impl Form {
    pub(crate) fn new() -> Form {
        Form {
            z: ZForm::new(),
            dev: 0,
            part: 0,
            list: String::new(),
            grid: GridState::default(),
            pick: None,
            all_ifs: 0,
            new_open: false,
            new_name: String::new(),
            new_kind: 1,
            rule: None,
            fields: RuleForm::new(),
            reveal: 0,
            pool: Default::default(),
            dyn_list: 0,
            dyn_to: 0,
            dyn_if: 0,
            dyn_pool: 0,
            dyn_over: 1,
            stat: StaticForm { kind: 1, local: String::new(), lport: String::new(), gtype: 1, global: String::new(), gif: 0, gport: String::new() },
            xlate: GridState::default(),
            test_if: 0,
            test_out: 0,
            test: TestForm::new(),
            asked: None,
            err: String::new(),
            ran: Vec::new(),
        }
    }
}

/// what a button asks for
pub(super) struct Plan {
    /// the undo step's name
    pub label: String,
    pub lines: Vec<String>,
    /// the rule to choose afterwards (its configuration line)
    pub pick: Option<String>,
    /// the list to show afterwards
    pub list: Option<String>,
    /// the rule form or the new list's fields close
    pub close: bool,
}

/// the protocols of the rule form: every protocol, the three with their own fields, the others by
/// name
pub(crate) fn protos() -> Vec<(&'static str, Option<u8>)> {
    let mut v: Vec<(&'static str, Option<u8>)> = alloc::vec![("ip", None), ("tcp", Some(PROTO_TCP)), ("udp", Some(PROTO_UDP)), ("icmp", Some(PROTO_ICMP))];
    v.extend(PROTOCOLS.iter().filter(|p| !matches!(p.1, PROTO_TCP | PROTO_UDP | PROTO_ICMP)).map(|p| (p.0, Some(p.1))));
    v
}

/// the protocols a packet of the Test part can have
pub(crate) fn test_protos() -> Vec<(&'static str, u8)> {
    protos().into_iter().filter_map(|(n, p)| p.map(|p| (n, p))).collect()
}

// a mask's bits from the top: what a net mask looks like (no wildcard one writes on purpose)
fn mask_like(w: Ipv4) -> bool {
    let b = w.to_u32();
    b != 0 && b != u32::MAX && (!b).wrapping_add(1) & !b == 0
}

/// an address field: "any", "10.0.0.5", "host 10.0.0.5", "10.0.0.0/24", "10.0.0.0 0.0.0.255"
pub(crate) fn wild_of(text: &str) -> Result<Wild, String> {
    let s = text.trim();
    let bad = || tf("„{}“ ist keine Adresse: any, 10.0.0.5, 10.0.0.0/24 oder 10.0.0.0 0.0.0.255.", &[s]);
    if s.eq_ignore_ascii_case("any") {
        return Ok(Wild::ANY);
    }
    let body = s.strip_prefix("host ").map_or(s, str::trim);
    if let Some((a, w)) = body.split_once(char::is_whitespace) {
        let (a, w) = (Ipv4::parse(a.trim()).ok_or_else(bad)?, Ipv4::parse(w.trim()).ok_or_else(bad)?);
        if mask_like(w) {
            return Err(tf("{} ist eine Netzmaske. Hier gehören Wildcard-Bits hin ({}) oder die Form {}/{}.", &[&w.to_string(), &Ipv4::from_u32(!w.to_u32()).to_string(), &a.to_string(), &w.to_u32().leading_ones().to_string()]));
        }
        return Ok(Wild::new(a, w));
    }
    if body.contains('/') {
        let p = Prefix4::parse(body).ok_or_else(bad)?;
        return Ok(if p.len == 0 { Wild::ANY } else { Wild::new(p.addr, Ipv4::from_u32(!p.mask().to_u32())) });
    }
    Ok(Wild::host(Ipv4::parse(body).ok_or_else(bad)?))
}

/// what an address field shows of a condition: "any", "10.0.0.5", "10.0.0.0/24", "10.0.0.0 0.255.0.255"
pub(crate) fn wild_text(w: &Wild) -> String {
    if w.is_any() {
        String::from("any")
    } else if w.is_host() {
        w.addr.to_string()
    } else if Ipv4::from_u32(!w.wild.to_u32()).mask_len().is_some() {
        format!("{}/{}", w.addr, (!w.wild.to_u32()).leading_ones())
    } else {
        format!("{} {}", w.addr, w.wild)
    }
}

/// a port field: a number or a name of the protocol ("www" for TCP)
pub(crate) fn port_of(proto: u8, text: &str) -> Result<u16, String> {
    let s = text.trim();
    s.parse::<u16>().ok().or_else(|| port_number(proto, s)).ok_or_else(|| tf("„{}“ ist kein Port: eine Zahl von 0 bis 65535 oder ein Name wie www.", &[s]))
}

// a port condition from its fields
fn ports_of(proto: u8, op: i32, a: &str, b: &str) -> Result<Ports, String> {
    Ok(match op {
        1 => Ports::Eq(port_of(proto, a)?),
        2 => Ports::Neq(port_of(proto, a)?),
        3 => Ports::Gt(port_of(proto, a)?),
        4 => Ports::Lt(port_of(proto, a)?),
        5 => {
            let (x, y) = (port_of(proto, a)?, port_of(proto, b)?);
            Ports::Range(x.min(y), x.max(y))
        }
        _ => Ports::Any,
    })
}

// the fields of a port condition
fn port_fields(p: Ports) -> (i32, String, String) {
    let n = |x: u16| x.to_string();
    match p {
        Ports::Any => (0, String::new(), String::new()),
        Ports::Eq(x) => (1, n(x), String::new()),
        Ports::Neq(x) => (2, n(x), String::new()),
        Ports::Gt(x) => (3, n(x), String::new()),
        Ports::Lt(x) => (4, n(x), String::new()),
        Ports::Range(a, b) => (5, n(a), n(b)),
    }
}

/// a rule typed as a line, read by a scratch console as the device's own would read it (Err: why
/// not, in the device's words)
pub(crate) fn parse_line(kind: AclKind, line: &str, dev_name: &str) -> Result<Ace, String> {
    let l = line.trim();
    if !matches!(l.split_whitespace().next(), Some("permit" | "deny")) {
        return Err(tf("„{}“ ist keine Regel: eine Zeile beginnt mit permit oder deny.", &[l]));
    }
    let mut p = Project::new("", 0);
    let dev = p.add_device("jf-r4", 0, 0).map_err(|_| String::new())?;
    let d = p.device(dev).cloned().ok_or_else(String::new)?;
    let mut e = Engine::new(0);
    e.apply(Action::AddDevice(d)).map_err(|x| x.to_string())?;
    for x in ["enable", "configure terminal", &mode_line("X", kind)] {
        e.cli_line(dev, x).map_err(|x| x.to_string())?;
    }
    let _ = console::scratch_out(&mut e, dev);
    e.cli_line(dev, l).map_err(|x| x.to_string())?;
    let out = console::scratch_out(&mut e, dev);
    if let Some(why) = console::refused(true, &out) {
        return Err(tf("{} lehnt „{}“ ab: {}", &[dev_name, l, why.trim()]));
    }
    let last = e.node(dev).and_then(|x| x.ip.as_ref()).and_then(|st| st.acls.lists.get("X")).and_then(|a| a.lines.last().cloned());
    match last.map(|x| x.item) {
        Some(Item::Rule(r)) => Ok(r),
        _ => Err(tf("„{}“ ist keine Regel: eine Zeile beginnt mit permit oder deny.", &[l])),
    }
}

/// the fields as a rule of the engine; Err: why not
pub(crate) fn rule_of_form(kind: AclKind, f: &RuleForm, dev_name: &str) -> Result<Ace, String> {
    if f.use_raw != 0 {
        return parse_line(kind, &f.raw, dev_name);
    }
    let permit = f.act == 0;
    let src = wild_of(&f.src)?;
    if kind == AclKind::Standard {
        let mut a = Ace::standard(permit, src);
        a.log = f.log != 0;
        return Ok(a);
    }
    let dst = wild_of(&f.dst)?;
    let list = protos();
    let proto = list.get(f.proto as usize).map_or(None, |p| p.1);
    let l4 = proto.unwrap_or(0);
    let ported = matches!(proto, Some(PROTO_TCP | PROTO_UDP));
    let sport = if ported { ports_of(l4, f.sop, &f.sp1, &f.sp2)? } else { Ports::Any };
    let dport = if ported { ports_of(l4, f.dop, &f.dp1, &f.dp2)? } else { Ports::Any };
    let icmp = if proto == Some(PROTO_ICMP) && f.icmp > 0 { ICMP_TYPES.get(f.icmp as usize - 1).map(|x| (x.1, x.2)) } else { None };
    Ok(Ace { permit, proto, src, sport, dst, dport, established: proto == Some(PROTO_TCP) && f.established != 0, icmp, log: f.log != 0 })
}

/// the fields can hold this rule (else it stays its raw line)
pub(crate) fn expressible(a: &Ace) -> bool {
    !mask_like(a.src.wild) && !mask_like(a.dst.wild) && protos().iter().any(|p| p.1 == a.proto) && a.icmp.is_none_or(|(t, c)| ICMP_TYPES.iter().any(|x| x.1 == t && x.2 == c))
}

/// a rule back in the fields; one they cannot hold as its raw line
pub(crate) fn form_of(a: &Ace, kind: AclKind) -> RuleForm {
    let mut f = RuleForm::new();
    f.act = if a.permit { 0 } else { 1 };
    f.log = a.log as i32;
    if !expressible(a) {
        f.use_raw = 1;
        f.raw = a.config(kind);
        return f;
    }
    f.src = wild_text(&a.src);
    if kind == AclKind::Standard {
        return f;
    }
    f.dst = wild_text(&a.dst);
    f.proto = protos().iter().position(|p| p.1 == a.proto).unwrap_or(0) as i32;
    (f.sop, f.sp1, f.sp2) = port_fields(a.sport);
    (f.dop, f.dp1, f.dp2) = port_fields(a.dport);
    f.established = a.established as i32;
    f.icmp = a.icmp.and_then(|(t, c)| ICMP_TYPES.iter().position(|x| x.1 == t && x.2 == c)).map_or(0, |i| i as i32 + 1);
    f
}

// ---------------------------------------------------------------- the rows

/// one row of a list's grid
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub key: u64,
    /// the rule's sequence number (0: a remark at the end, the implicit deny)
    pub seq: u32,
    pub act: String,
    pub proto: String,
    pub src: String,
    pub dst: String,
    pub port: String,
    pub log: bool,
    /// packets it decided (None: the row counts nothing)
    pub hits: Option<u64>,
    /// the remarks just above it
    pub remark: String,
    /// the line of the list's configuration
    pub line: String,
    /// the fields cannot hold it: it shows and changes as its raw line
    pub raw: bool,
    pub implicit: bool,
}

// the Port column: the ports, the ICMP message, established
fn port_cell(a: &Ace) -> String {
    let p = a.proto.unwrap_or(0);
    let mut v: Vec<String> = Vec::new();
    if a.sport != Ports::Any {
        v.push(tf("von {}", &[a.sport.text(p).trim()]));
    }
    if a.dport != Ports::Any {
        v.push(String::from(a.dport.text(p).trim()));
    }
    if let Some((ty, c)) = a.icmp {
        v.push(icmp_text(ty, c));
    }
    if a.established {
        v.push(String::from("established"));
    }
    if v.is_empty() { String::from("–") } else { v.join(", ") }
}

/// the rows of a list: its rules with the remarks above them in their column, remarks after the
/// last rule on rows of their own, the implicit deny at the end of a list with rules
pub(crate) fn rows_of(a: &Acl) -> Vec<Row> {
    let mut v = Vec::new();
    let mut above: Vec<&str> = Vec::new();
    for l in &a.lines {
        match &l.item {
            Item::Remark(t) => above.push(t),
            Item::Rule(r) => {
                let raw = !expressible(r);
                let line = r.config(a.kind);
                v.push(Row {
                    key: l.seq as u64,
                    seq: l.seq,
                    act: String::from(if r.permit { "permit" } else { "deny" }),
                    proto: proto_text(r.proto),
                    src: if raw { line.clone() } else { wild_text(&r.src) },
                    dst: if raw { String::new() } else { wild_text(&r.dst) },
                    port: if raw { String::new() } else { port_cell(r) },
                    log: r.log,
                    hits: Some(l.hits),
                    remark: above.join(" · "),
                    line,
                    raw,
                    implicit: false,
                });
                above.clear();
            }
        }
    }
    for (i, r) in above.iter().enumerate() {
        v.push(Row { key: KEY_REMARK + i as u64, seq: 0, act: String::new(), proto: String::new(), src: String::new(), dst: String::new(), port: String::new(), log: false, hits: None, remark: String::from(*r), line: format!("remark {}", r), raw: false, implicit: false });
    }
    if a.has_rules() {
        v.push(Row { key: KEY_IMPLICIT, seq: 0, act: String::from("deny"), proto: String::from("ip"), src: String::from("any"), dst: String::from("any"), port: String::from("–"), log: false, hits: None, remark: String::from(t("implizit: was keine Regel trifft")), line: String::from("deny ip any any"), raw: false, implicit: true });
    }
    v
}

// ---------------------------------------------------------------- what a change types

fn mode_line(name: &str, kind: AclKind) -> String {
    format!("ip access-list {} {}", kind.word(), name)
}

fn item_line(kind: AclKind, it: &Item) -> String {
    match it {
        Item::Rule(r) => r.config(kind),
        Item::Remark(x) => format!("remark {}", x),
    }
}

fn items_of(a: &Acl) -> Vec<Item> {
    a.lines.iter().map(|l| l.item.clone()).collect()
}

// the rule with this number: the first line of its block (the remarks just above it), its line
fn block_of(a: &Acl, seq: u32) -> Option<(usize, usize)> {
    let at = a.lines.iter().position(|l| l.seq == seq && matches!(l.item, Item::Rule(_)))?;
    let mut first = at;
    while first > 0 && matches!(a.lines[first - 1].item, Item::Remark(_)) {
        first -= 1;
    }
    Some((first, at))
}

/// the rule's own number after "ip access-list resequence N 10 10"
fn canon_seq(a: &Acl, seq: u32) -> Option<u32> {
    let i = a.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).position(|l| l.seq == seq)?;
    Some(10 * (i as u32 + 1))
}

/// the command lines that make list `a` hold `want`, keeping every counter it can (see the top of
/// the file); `ones`: rules that may go or come by number, each (its number now, the rule)
fn change_lines(name: &str, a: &Acl, want: &[Item], ones: &[(Option<u32>, Ace)]) -> Vec<String> {
    let kind = a.kind;
    let have = items_of(a);
    if have == want {
        return Vec::new();
    }
    // only added at the end: no numbers
    if want.starts_with(&have) {
        let tail = want[have.len()..].iter().map(|it| item_line(kind, it));
        if a.numbered {
            return tail.map(|l| format!("access-list {} {}", name, l)).collect();
        }
        let mut v = alloc::vec![mode_line(name, kind)];
        v.extend(tail);
        return v;
    }
    // only removed: by their words
    let mut c = a.clone();
    let mut gone: Vec<String> = Vec::new();
    let mut left = want.iter().peekable();
    for it in &have {
        if left.peek() == Some(&it) {
            left.next();
        } else {
            c.remove_item(it);
            gone.push(format!("no {}", item_line(kind, it)));
        }
    }
    if items_of(&c) == want {
        let mut v = alloc::vec![mode_line(name, kind)];
        v.extend(gone);
        return v;
    }
    // one rule goes to its new place by number: the others keep their counters
    let mut c = a.clone();
    let canon = c.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).enumerate().all(|(i, l)| l.seq == 10 * (i as u32 + 1));
    let _ = c.resequence(10, 10);
    for (old, y) in ones {
        let mut x = c.clone();
        let old_c = old.and_then(|s| canon_seq(a, s));
        if let Some(s) = old_c {
            x.remove_seq(s);
        }
        let Some(at) = want.iter().position(|i| matches!(i, Item::Rule(r) if r == y)) else { continue };
        let seq_of = |it: &Item| x.lines.iter().find(|l| l.item == *it).map(|l| l.seq);
        let lo = want[..at].iter().rev().find(|i| matches!(i, Item::Rule(_))).and_then(seq_of).unwrap_or(0);
        let hi = want[at + 1..].iter().find(|i| matches!(i, Item::Rule(_))).and_then(seq_of);
        let seq = hi.map_or(lo + 10, |h| (lo + h) / 2);
        if x.add(Some(seq), Item::Rule(y.clone())).is_err() || items_of(&x) != want {
            continue;
        }
        let mut v = Vec::new();
        if !canon {
            v.push(format!("ip access-list resequence {} 10 10", name));
        }
        v.push(mode_line(name, kind));
        if let Some(s) = old_c {
            v.push(format!("no {}", s));
        }
        v.push(format!("{} {}", seq, y.config(kind)));
        return v;
    }
    // what follows the first difference goes and comes again (a line that is also further up: from
    // there, because "no" takes the first one with its words)
    let mut i = have.iter().zip(want).take_while(|(x, y)| x == y).count();
    while let Some(j) = (0..i).find(|&j| have[i..].contains(&have[j])) {
        i = j;
    }
    let mut v = alloc::vec![mode_line(name, kind)];
    v.extend(have[i..].iter().map(|it| format!("no {}", item_line(kind, it))));
    v.extend(want[i..].iter().map(|it| item_line(kind, it)));
    v
}

// the number of a rule equal to this one, other than `not`
fn twin_of(a: &Acl, r: &Ace, not: Option<u32>) -> Option<u32> {
    a.lines.iter().find(|l| matches!(&l.item, Item::Rule(x) if x == r) && Some(l.seq) != not).map(|l| l.seq)
}

/// a new rule in list `name` (at the end, or after rule `after`), with a remark above it
pub(crate) fn add_lines(name: &str, a: &Acl, r: &Ace, remark: &str, after: Option<u32>) -> Result<Vec<String>, String> {
    if let Some(s) = twin_of(a, r, None) {
        return Err(tf("Diese Regel steht schon in der Liste (Zeile {}). Ändern Sie mindestens ein Feld.", &[&s.to_string()]));
    }
    let remark = remark.trim();
    let mut want = items_of(a);
    let at = after.and_then(|s| block_of(a, s)).map_or(want.len(), |(_, at)| at + 1);
    let mut new: Vec<Item> = Vec::new();
    if !remark.is_empty() {
        new.push(Item::Remark(String::from(remark)));
    }
    new.push(Item::Rule(r.clone()));
    want.splice(at..at, new);
    Ok(change_lines(name, a, &want, &[(None, r.clone())]))
}

/// rule `seq` changes; the remark above it too when the text differs
pub(crate) fn edit_lines(name: &str, a: &Acl, seq: u32, r: &Ace, remark: &str) -> Result<Vec<String>, String> {
    if let Some(s) = twin_of(a, r, Some(seq)) {
        return Err(tf("Diese Regel steht schon in der Liste (Zeile {}). Ändern Sie mindestens ein Feld.", &[&s.to_string()]));
    }
    let Some((first, at)) = block_of(a, seq) else { return Ok(Vec::new()) };
    let have = items_of(a);
    let old: Vec<Item> = have[first..at].to_vec();
    let old_text = old.iter().filter_map(|i| if let Item::Remark(x) = i { Some(x.as_str()) } else { None }).collect::<Vec<_>>().join(" · ");
    let remark = remark.trim();
    let mut new: Vec<Item> = if remark == old_text {
        old
    } else if remark.is_empty() {
        Vec::new()
    } else {
        alloc::vec![Item::Remark(String::from(remark))]
    };
    new.push(Item::Rule(r.clone()));
    let mut want = have;
    want.splice(first..=at, new);
    Ok(change_lines(name, a, &want, &[(Some(seq), r.clone())]))
}

/// rule `seq` a place up or down, with the remarks above it; None: it stands at that end
pub(crate) fn move_lines(name: &str, a: &Acl, seq: u32, up: bool) -> Option<Vec<String>> {
    let (first, at) = block_of(a, seq)?;
    let rules: Vec<u32> = a.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).map(|l| l.seq).collect();
    let i = rules.iter().position(|&s| s == seq)?;
    let other = *rules.get(if up { i.checked_sub(1)? } else { i + 1 })?;
    let (ofirst, oat) = block_of(a, other)?;
    let have = items_of(a);
    let (top, low) = if up { ((ofirst, oat), (first, at)) } else { ((first, at), (ofirst, oat)) };
    let mut want: Vec<Item> = have[..top.0].to_vec();
    want.extend_from_slice(&have[low.0..=low.1]);
    want.extend_from_slice(&have[top.0..=top.1]);
    want.extend_from_slice(&have[low.1 + 1..]);
    let rule = |k: usize| if let Item::Rule(r) = &have[k] { Some(r.clone()) } else { None };
    let ones = [(Some(seq), rule(at)?), (Some(other), rule(oat)?)];
    Some(change_lines(name, a, &want, &ones))
}

/// a row goes: a rule with the remarks above it, or a remark after the last rule
pub(crate) fn delete_lines(name: &str, a: &Acl, key: u64) -> Vec<String> {
    let mut want = items_of(a);
    if key >= KEY_REMARK {
        let trailing = a.lines.iter().rposition(|l| matches!(l.item, Item::Rule(_))).map_or(0, |i| i + 1);
        let k = trailing + (key - KEY_REMARK) as usize;
        if k < want.len() {
            want.remove(k);
        }
    } else if let Some((first, at)) = block_of(a, key as u32) {
        want.drain(first..=at);
    }
    change_lines(name, a, &want, &[])
}

/// a new list from the name's field and its kind (0 standard, 1 extended) -> its name and lines
pub(crate) fn new_list_lines(text: &str, kind: i32, acls: &Acls) -> Result<(String, Vec<String>), String> {
    let name = text.trim();
    let kind = if kind == 0 { AclKind::Standard } else { AclKind::Extended };
    if name.is_empty() {
        return Err(String::from(t("Eine Liste braucht einen Namen oder eine Nummer.")));
    }
    if name.bytes().all(|b| b.is_ascii_digit()) {
        if name.parse::<u32>().ok().and_then(AclKind::of_number) != Some(kind) {
            return Err(tf("{} passt nicht: 1 bis 99 und 1300 bis 1999 sind Standardlisten, 100 bis 199 und 2000 bis 2699 erweiterte.", &[name]));
        }
    } else if !name.as_bytes()[0].is_ascii_alphabetic() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(tf("„{}“ ist kein Listenname: Buchstaben, Ziffern, - und _, vorn ein Buchstabe.", &[name]));
    }
    if acls.lists.contains_key(name) {
        return Err(tf("Die Liste {} gibt es schon.", &[name]));
    }
    Ok((String::from(name), alloc::vec![mode_line(name, kind)]))
}

/// list `name` goes; refused while an interface or NAT uses it
pub(crate) fn del_list_lines(name: &str, acls: &Acls, rules: &[Rule], if_name: &dyn Fn(u16) -> String) -> Result<Vec<String>, String> {
    let Some(a) = acls.lists.get(name) else { return Err(tf("Die Liste {} gibt es nicht mehr.", &[name])) };
    if let Some(((i, out), _)) = acls.bound.iter().find(|(_, l)| l.as_str() == name) {
        return Err(tf("Liste {} filtert noch {} ({}): erst die Bindung lösen.", &[name, &if_name(*i), t(DIRECTIONS[*out as usize])]));
    }
    if rules.iter().any(|r| matches!(r, Rule::List { acl, .. } if acl == name)) {
        return Err(tf("NAT wählt noch mit Liste {} aus: erst die NAT-Regel entfernen.", &[name]));
    }
    Ok(alloc::vec![if a.numbered { format!("no access-list {}", name) } else { format!("no {}", mode_line(name, a.kind)) }])
}

/// the list of an interface in a direction becomes `new` (None: no list; `old`: the one there)
pub(crate) fn bind_lines(iface: &str, out: bool, old: Option<&str>, new: Option<&str>) -> Vec<String> {
    let dir = if out { "out" } else { "in" };
    let mut v = alloc::vec![format!("interface {}", iface)];
    match (new, old) {
        (Some(l), _) => v.push(format!("ip access-group {} {}", l, dir)),
        (None, Some(o)) => v.push(format!("no ip access-group {} {}", o, dir)),
        (None, None) => return Vec::new(),
    }
    v
}

// ---------------------------------------------------------------- what NAT types

/// the side of an interface becomes `new` (`old`: the one it has)
pub(crate) fn side_lines(iface: &str, old: Option<Side>, new: Option<Side>) -> Vec<String> {
    let word = |s: Side| if s == Side::Inside { "inside" } else { "outside" };
    let mut v = alloc::vec![format!("interface {}", iface)];
    match (new, old) {
        (Some(s), _) => v.push(format!("ip nat {}", word(s))),
        (None, Some(o)) => v.push(format!("no ip nat {}", word(o))),
        (None, None) => return Vec::new(),
    }
    v
}

/// a pool: name, first and last address, the network as 24, /24 or 255.255.255.0
pub(crate) fn pool_lines(name: &str, start: &str, end: &str, mask: &str) -> Result<Vec<String>, String> {
    let name = name.trim();
    if name.is_empty() || name.split_whitespace().count() != 1 {
        return Err(tf("„{}“ ist kein Name für einen Pool: ein Wort.", &[name]));
    }
    let addr = |s: &str| Ipv4::parse(s.trim()).ok_or_else(|| tf("„{}“ ist keine IPv4-Adresse.", &[s.trim()]));
    let (a, b) = (addr(start)?, addr(end)?);
    let m = mask.trim();
    let len = m.trim_start_matches('/').parse::<u8>().ok().filter(|l| (1..=32).contains(l)).or_else(|| Ipv4::parse(m).and_then(|x| x.mask_len()).filter(|l| *l > 0));
    let Some(len) = len else {
        return Err(tf("„{}“ ist keine Netzmaske: 24, /24 oder 255.255.255.0.", &[m]));
    };
    if a.to_u32() > b.to_u32() || !(Prefix4 { addr: a, len }.trunc().contains(b)) {
        return Err(tf("{} bis {} liegt nicht in einem Netz der Länge /{}.", &[&a.to_string(), &b.to_string(), &len.to_string()]));
    }
    Ok(alloc::vec![format!("ip nat pool {} {} {} netmask {}", name, a, b, Ipv4::mask_of(len))])
}

/// pool `name` goes; refused while a rule reads it (the rule would find no addresses)
pub(crate) fn pool_del_lines(name: &str, rules: &[Rule]) -> Result<Vec<String>, String> {
    match rules.iter().find(|r| matches!(r, Rule::List { to: Target::Pool(p), .. } if p == name)) {
        Some(r) => Err(tf("„{}“ nimmt die Adressen aus Pool {}: erst die Regel entfernen.", &[&rule_text(r), name])),
        None => Ok(alloc::vec![format!("no ip nat pool {}", name)]),
    }
}

/// a dynamic rule: the hosts list `acl` permits, from a pool or behind an interface's address
pub(crate) fn dyn_lines(acl: &str, to: &Target, overload: bool) -> Vec<String> {
    alloc::vec![match to {
        Target::Pool(p) => format!("ip nat inside source list {} pool {}{}", acl, p, if overload { " overload" } else { "" }),
        Target::Iface(i) => format!("ip nat inside source list {} interface {} overload", acl, i),
    }]
}

// a port of a static translation
fn nat_port(text: &str) -> Result<u16, String> {
    let s = text.trim();
    s.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| tf("„{}“ ist kein Port von 1 bis 65535.", &[s]))
}

/// a static translation or port forwarding from its fields (`ifs`: the interfaces' names)
pub(crate) fn static_lines(f: &StaticForm, ifs: &[String]) -> Result<Vec<String>, String> {
    let local = Ipv4::parse(f.local.trim()).ok_or_else(|| tf("„{}“ ist keine IPv4-Adresse.", &[f.local.trim()]))?;
    let global = if f.gtype == 1 {
        match ifs.get(f.gif as usize) {
            Some(n) => format!("interface {}", n),
            None => return Err(String::from(t("Das Gerät hat keine Schnittstelle dafür."))),
        }
    } else {
        Ipv4::parse(f.global.trim()).ok_or_else(|| tf("„{}“ ist keine IPv4-Adresse.", &[f.global.trim()]))?.to_string()
    };
    if f.kind == 0 {
        return Ok(alloc::vec![format!("ip nat inside source static {} {}", local, global)]);
    }
    let (lp, gp) = (nat_port(&f.lport)?, nat_port(&f.gport)?);
    Ok(alloc::vec![format!("ip nat inside source static {} {} {} {} {}", if f.kind == 1 { "tcp" } else { "udp" }, local, lp, global, gp)])
}

/// the line that removes a rule of NAT
pub(crate) fn rule_del_line(r: &Rule) -> String {
    match r {
        Rule::List { acl, .. } => format!("no ip nat inside source list {}", acl),
        Rule::Static { proto: 0, local, .. } => format!("no ip nat inside source static {}", local),
        Rule::Static { proto, local, lport, .. } => format!("no ip nat inside source static {} {} {}", if *proto == PROTO_TCP { "tcp" } else { "udp" }, local, lport),
    }
}

// how a rule of NAT reads
fn rule_text(r: &Rule) -> String {
    match r {
        Rule::List { acl, to: Target::Pool(p), overload } => tf("Liste {} → Pool {}{}", &[acl, p, if *overload { " (PAT)" } else { "" }]),
        Rule::List { acl, to: Target::Iface(i), .. } => tf("Liste {} → Adresse von {} (PAT)", &[acl, &ifname::short(i)]),
        Rule::Static { proto, local, lport, global, gport, .. } => {
            let g = match global {
                Global::Addr(a) => a.to_string(),
                Global::Iface(i) => ifname::short(i),
            };
            match *proto {
                0 => format!("{} = {}", local, g),
                p => format!("{} {}:{} = {}:{}", if p == PROTO_TCP { "tcp" } else { "udp" }, local, lport, g, gport),
            }
        }
    }
}

// ---------------------------------------------------------------- the Test part

/// a packet from the fields, as the engine reads a real one (netpkt::nat::Flow::of)
pub(crate) fn flow_of(f: &TestForm) -> Result<Flow, String> {
    let list = test_protos();
    let proto = list.get(f.proto as usize).map_or(PROTO_TCP, |p| p.1);
    let addr = |s: &str| Ipv4::parse(s.trim()).ok_or_else(|| tf("„{}“ ist keine IPv4-Adresse.", &[s.trim()]));
    let (src, dst) = (addr(&f.src)?, addr(&f.dst)?);
    let mut fl = Flow { proto, src, dst, sport: 0, dport: 0, ports: false, icmp: None, flags: 0, later: false, mf: false, id: 0 };
    match proto {
        PROTO_TCP | PROTO_UDP => {
            fl.dport = port_of(proto, &f.dport)?;
            // (a source port nobody asked for: one the systems hand out)
            fl.sport = if f.sport.trim().is_empty() { 49152 } else { port_of(proto, &f.sport)? };
            fl.ports = true;
            if proto == PROTO_TCP {
                fl.flags = if f.ack != 0 { ACK } else { SYN };
            }
        }
        PROTO_ICMP => {
            let m = ICMP_TYPES.get(f.icmp as usize).unwrap_or(&ICMP_TYPES[0]);
            fl.icmp = Some((m.1, m.2.unwrap_or(0)));
            fl.ports = is_query(m.1);
        }
        _ => {}
    }
    Ok(fl)
}

/// what the Test part says: the packet passes, the words for it, the rule's line if a rule decides
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Told {
    pub pass: bool,
    pub seq: u32,
    pub list: String,
    pub text: String,
}

/// what the list on an interface says about a packet, by the function the engine's filter acts on
pub(crate) fn told(acls: &Acls, iface: u16, if_name: &str, out: bool, f: &Flow) -> Told {
    let dir = t(DIRECTIONS[out as usize]);
    match acls.probe(iface, out, f) {
        Probe::NoList => Told { pass: true, seq: 0, list: String::new(), text: tf("{} hat {} keine Liste: das Paket passiert.", &[if_name, dir]) },
        Probe::Empty(l) => {
            let what = if acls.lists.contains_key(&l) { tf("Liste {} hat keine Regeln: das Paket passiert.", &[&l]) } else { tf("Liste {} gibt es nicht: das Paket passiert.", &[&l]) };
            Told { pass: true, seq: 0, list: l, text: what }
        }
        Probe::Decided { list, verdict, line } => {
            let rule = line.and_then(|i| acls.lists.get(&list).and_then(|a| a.lines.get(i).map(|x| item_line(a.kind, &x.item))));
            let text = match rule {
                Some(r) => tf("Liste {}, Zeile {}: {}", &[&list, &verdict.seq.to_string(), &r]),
                None => tf("Liste {}: keine Regel passt, das implizite deny am Ende verwirft das Paket.", &[&list]),
            };
            Told { pass: verdict.permit, seq: verdict.seq, list, text }
        }
    }
}

/// why the device's filters cannot be changed now (`locked`: a reason from outside them)
pub(crate) fn refusal(routes: bool, locked: Option<&str>, dev: &str) -> Option<String> {
    if let Some(l) = locked {
        return Some(String::from(l));
    }
    (!routes).then(|| tf("{} routet nicht (ip routing ist aus): eine Regel träfe nur Pakete an das Gerät selbst, weitergeleitet wird nichts. Erst Routing einschalten.", &[dev]))
}

// ---------------------------------------------------------------- the window

/// an interface as the tab lists it
pub(super) struct If {
    pub index: u16,
    pub name: String,
    pub short: String,
    pub addr: String,
}

/// what the tab shows of the device, at the moment the view stands at
pub(super) struct Snap {
    pub name: String,
    /// nothing can change: the reason
    pub locked: Option<String>,
    pub routes: bool,
    pub acls: Acls,
    pub ifs: Vec<If>,
    /// a firewall model: it has Zonen and Sitzungen
    pub is_fw: bool,
    pub fw: Firewall,
    sides: BTreeMap<u16, Side>,
    pools: Vec<(String, Pool)>,
    rules: Vec<Rule>,
}

fn snap(n: &Net, dev: u32) -> Option<Snap> {
    let d = n.doc.proj.device(dev)?;
    let e = n.doc.sim.state();
    let st = e.node(dev)?.ip.as_ref()?;
    let ifs = st.ifaces.iter().enumerate().filter(|(_, f)| !f.hidden).map(|(i, f)| If { index: i as u16, name: f.name.clone(), short: ifname::short(&f.name), addr: f.addrs.first().map(|a| a.to_string()).unwrap_or_default() }).collect();
    let locked = if n.doc.sim.view_at().is_some() {
        Some(String::from(t("Die Ansicht steht in der Vergangenheit: hier lässt sich nur lesen, was damals galt.")))
    } else if d.off {
        Some(tf("{} ist ausgeschaltet: Regeln und NAT lassen sich ändern, wenn es läuft.", &[&d.name]))
    } else {
        None
    };
    let is_fw = n.doc.proj.model_of(dev).is_some_and(|m| m.has(F_FIREWALL));
    Some(Snap { name: d.name.clone(), locked, routes: st.forwarding, acls: st.acls.clone(), ifs, is_fw, fw: st.fw.without_sessions(), sides: st.nat.sides.clone(), pools: st.nat.pools.iter().map(|(a, b)| (a.clone(), *b)).collect(), rules: st.nat.rules.iter().map(|(_, r)| r.clone()).collect() })
}

// a "label  field" row (the fields follow, then box_end); it breaks when it is narrow
pub(super) fn field(ui: &mut Ui, label: &'static str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
    ui.next_size(120, 0);
    ui.text(TS_BODY | TX_SECONDARY, t(label));
}

// a section's heading
pub(super) fn heading(ui: &mut Ui, s: &'static str) {
    ui.text(TS_BODY_STRONG, t(s));
}

// a caption that wraps
pub(super) fn note(ui: &mut Ui, s: &str) {
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, s);
}

// a small button with an icon
pub(super) fn icon_button(ui: &mut Ui, id: u32, icon: u32, label: &str, on: bool) -> bool {
    ui.next_flags(if on { 0 } else { ST_DISABLED });
    ui.next_icon(icon);
    ui.button(id, label) & EV_ACT != 0
}

/// a row's remove button
pub(super) fn remove_button(ui: &mut Ui, id: u32, label: &str) -> bool {
    ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
    ui.next_icon(IC_DELETE);
    ui.button(id, label) & EV_ACT != 0
}

/// a change of the tab, unless the device cannot be changed now
pub(super) fn offer(s: &Snap, p: Result<Plan, String>) -> Result<Plan, String> {
    match refusal(s.routes, s.locked.as_deref(), &s.name) {
        Some(why) => Err(why),
        None => p,
    }
}

/// the Firewall tab of device window k
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = ID_FW0 + 0x800 * k as u32;
    let Some(s) = snap(n, dev) else {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.empty_state(IC_INFO, t("Keine Schnittstellen"), t("Dieses Gerät hat keine Ports oder läuft nicht im Simulator."));
        ui.box_end();
        return;
    };
    // (the table of translations only while it is shown: it can be long)
    let translations = if n.devwin.peek(k).is_some_and(|w| w.fw.part == 1 && w.fw.dev == dev) { n.doc.sim.state().nat_translations(dev) } else { Vec::new() };
    // (the sessions and the zones' answer likewise: both are read here, the parts only show them)
    let sessions: Vec<FwRow> = if n.devwin.peek(k).is_some_and(|w| w.fw.part == 4 && w.fw.dev == dev) { n.doc.sim.state().fw_sessions(dev) } else { Vec::new() };
    let zone_test: Option<FwTest> = n.devwin.peek(k).filter(|w| w.fw.part == 2 && w.fw.dev == dev).and_then(|w| w.fw.asked.as_ref()).filter(|_| s.fw.active()).and_then(|(i, out, fl)| n.doc.sim.state().fw_test(dev, *i, *out, fl));
    let now = n.doc.sim.state().now();
    let Some(w) = n.devwin.get(k) else { return };
    let f = &mut w.fw;
    if f.dev != dev {
        *f = Form::new();
        f.dev = dev;
    }
    let mut plan: Option<Result<Plan, String>> = None;
    let mut console_tab = false;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 6);
    // the parts
    if s.is_fw {
        // (the order on the screen; f.part keeps its number)
        const ORDER: [i32; 5] = [0, 1, 3, 4, 2];
        let parts = [t("Regeln"), t("NAT"), t("Zonen"), t("Sitzungen"), t("Test")];
        let mut pos = ORDER.iter().position(|p| *p == f.part).unwrap_or(0) as i32;
        if ui.segmented(id + 0x70, &mut pos, &parts) & EV_ACT != 0 {
            f.part = ORDER.get(pos as usize).copied().unwrap_or(0);
            f.err.clear();
            ui.repass();
        }
    } else {
        f.part = f.part.clamp(0, 2);
        let parts = [t("Regeln"), t("NAT"), t("Test")];
        if ui.segmented(id, &mut f.part, &parts) & EV_ACT != 0 {
            f.err.clear();
            ui.repass();
        }
    }
    if let Some(why) = &s.locked {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.icon(IC_INFO, 14);
        note(ui, why);
        ui.box_end();
    } else if !s.routes {
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        ui.icon(IC_WARNING, 14);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &tf("{} routet nicht: Regeln und NAT träfen nur Pakete an das Gerät selbst.", &[&s.name]));
        if ui.button(id + 0x19, t("Routing einschalten")) & EV_ACT != 0 {
            plan = Some(Ok(Plan { label: tf("Routing auf {} einschalten", &[&s.name]), lines: alloc::vec![String::from("ip routing")], pick: None, list: None, close: false }));
        }
        ui.box_end();
    }
    let lock = if s.locked.is_some() { BOX_DISABLED } else { 0 };
    ui.next_size(-1, -1);
    ui.scroll_begin(if f.part > 2 { id + 0x78 + (f.part - 3) as u32 } else { id + 4 + f.part.clamp(0, 2) as u32 }, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 10);
    match f.part {
        0 => part_rules(ui, id, f, &s, lock, &mut plan),
        1 => part_nat(ui, id, f, &s, lock, &translations, now, &mut plan),
        3 => fwzones::part_zones(ui, id, k, f, &s, lock, &mut plan),
        4 => fwzones::part_sessions(ui, id, k, f, &s, lock, &sessions, now, &mut plan),
        _ => part_test(ui, id, f, &s, zone_test.as_ref()),
    }
    ui.box_end();
    ui.scroll_end();
    // below the parts: the refusal, the lines
    if !f.err.is_empty() {
        let err = f.err.clone();
        crate::workspace::error_text(ui, &err, 0);
    }
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 2);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY_STRONG, t("Befehle"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, t("im Konfigurationsmodus (configure terminal)"));
    ui.next_flags(ST_COMPACT);
    ui.next_icon(crate::workspace::icons::JN_TERMINAL);
    if ui.button(id + 7, t("Konsole")) & EV_ACT != 0 {
        console_tab = true;
    }
    ui.box_end();
    if f.ran.is_empty() {
        note(ui, t("Was die Formulare tun, steht hier als die Zeilen, die in der Konsole des Geräts laufen."));
    } else {
        ui.next_size(-1, 18 * f.ran.len().clamp(1, 4) as i32);
        ui.scroll_begin(id + 8, SC_V);
        ui.box_begin(0, BOX_V, 0);
        for l in &f.ran {
            ui.text(TS_CAPTION, l);
        }
        ui.box_end();
        ui.scroll_end();
    }
    ui.box_end();
    ui.box_end();
    // the work, after the widgets
    let Some(p) = plan else {
        if console_tab {
            w.tab = super::Tab::Cli;
            n.cons.focus = Some(dev);
            ui.repass();
        }
        return;
    };
    let result = match p {
        Err(e) => Err(e),
        Ok(p) if p.lines.is_empty() => Ok(p),
        Ok(p) => console::run_lines_running(n, dev, &p.label, &p.lines).map(|_| p),
    };
    if let Some(w) = n.devwin.get(k) {
        let f = &mut w.fw;
        match result {
            Ok(p) => {
                f.err.clear();
                if !p.lines.is_empty() {
                    f.ran = p.lines;
                }
                if p.pick.is_some() {
                    f.pick = p.pick;
                }
                if let Some(l) = p.list {
                    f.list = l;
                }
                if p.close {
                    f.rule = None;
                    f.new_open = false;
                }
            }
            Err(e) => f.err = e,
        }
    }
    ui.repass();
}

// the columns of a list's grid: all of them, or (narrow) the rule as one line
fn rule_cols(wide: bool) -> Vec<GridCol<'static>> {
    if wide {
        alloc::vec![
            GridCol::new(t("Pos."), 52).flags(GC_NOSORT | GC_RIGHT),
            GridCol::new(t("Aktion"), 62).flags(GC_NOSORT),
            GridCol::new(t("Protokoll"), 72).flags(GC_NOSORT),
            GridCol::new(t("Quelle"), 130).flags(GC_NOSORT),
            GridCol::new(t("Ziel"), 130).flags(GC_NOSORT),
            GridCol::new(t("Port"), 110).flags(GC_NOSORT),
            GridCol::new(t("Log"), 40).flags(GC_NOSORT),
            GridCol::new(t("Treffer"), 64).flags(GC_NOSORT | GC_RIGHT),
            GridCol::new(t("Bemerkung"), 120).flags(GC_NOSORT | GC_FILL),
        ]
    } else {
        alloc::vec![GridCol::new(t("Pos."), 52).flags(GC_NOSORT | GC_RIGHT), GridCol::new(t("Regel"), 180).flags(GC_NOSORT | GC_FILL), GridCol::new(t("Treffer"), 64).flags(GC_NOSORT | GC_RIGHT)]
    }
}

// the combobox of an interface's list in a direction: the lists; Some(new) when it changed
fn bind_box(ui: &mut Ui, id: u32, cur: Option<&str>, names: &[String]) -> Option<Option<String>> {
    let mut items: Vec<&str> = alloc::vec![t("keine")];
    items.extend(names.iter().map(String::as_str));
    // (bound by name, the list is not there)
    if let Some(c) = cur
        && !names.iter().any(|x| x == c)
    {
        items.push(c);
    }
    let mut sel = cur.and_then(|c| items.iter().skip(1).position(|x| *x == c)).map_or(0, |i| i as i32 + 1);
    ui.next_size(110, 0);
    if ui.combobox(id, &mut sel, &items) & EV_ACT != 0 {
        return Some(if sel == 0 { None } else { items.get(sel as usize).map(|x| String::from(*x)) });
    }
    None
}

// what protects which interface
fn bindings(ui: &mut Ui, id: u32, f: &mut Form, s: &Snap, lock: u32, plan: &mut Option<Result<Plan, String>>) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
    heading(ui, "Was schützt welche Schnittstelle?");
    ui.checkbox(id + 0x14, t("alle zeigen"), &mut f.all_ifs);
    ui.box_end();
    let names = s.acls.names();
    let bound = |i: u16, out: bool| s.acls.bound.get(&(i, out)).map(String::as_str);
    let shown: Vec<&If> = s.ifs.iter().filter(|x| f.all_ifs != 0 || !x.addr.is_empty() || bound(x.index, false).is_some() || bound(x.index, true).is_some()).collect();
    if shown.is_empty() {
        note(ui, t("Noch hat keine Schnittstelle eine Adresse."));
    }
    ui.box_begin(0, BOX_V | lock, 4);
    for (i, x) in shown.iter().enumerate().take(64) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        ui.next_size(170, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &if x.addr.is_empty() { x.short.clone() } else { format!("{}  {}", x.short, x.addr) });
        for out in [false, true] {
            let cur = bound(x.index, out);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.next_size(72, 0);
            ui.text(TS_CAPTION | TX_SECONDARY, t(DIRECTIONS[out as usize]));
            if let Some(new) = bind_box(ui, id + 0x100 + 2 * i as u32 + out as u32, cur, &names) {
                let lines = bind_lines(&x.name, out, cur, new.as_deref());
                *plan = Some(match refusal(s.routes, s.locked.as_deref(), &s.name) {
                    Some(why) => Err(why),
                    None => Ok(Plan { label: tf("Liste an {} binden", &[&x.short]), lines, pick: None, list: None, close: false }),
                });
            }
            // the state as a dot and, for screen readers, in words
            let (dot, text) = match cur {
                None => (BADGE_NEUTRAL, tf("{} {}: keine Liste, alles passiert", &[&x.short, t(DIRECTIONS[out as usize])])),
                Some(l) if s.acls.lists.get(l).is_some_and(|a| a.has_rules()) => (BADGE_OK, tf("{} {}: Liste {}", &[&x.short, t(DIRECTIONS[out as usize]), l])),
                Some(l) => (BADGE_WARN, tf("{} {}: Liste {} ohne Regeln, alles passiert", &[&x.short, t(DIRECTIONS[out as usize]), l])),
            };
            ui.status_dot(dot);
            ui.tooltip(&text);
            ui.sem(K_TEXT, 0, Some(&text), 0);
            ui.box_end();
        }
        ui.box_end();
    }
    ui.box_end();
}

// Regeln: what protects what, the chosen list, its rules, the form
fn part_rules(ui: &mut Ui, id: u32, f: &mut Form, s: &Snap, lock: u32, plan: &mut Option<Result<Plan, String>>) {
    let offer = |p: Result<Plan, String>| -> Result<Plan, String> {
        match refusal(s.routes, s.locked.as_deref(), &s.name) {
            Some(why) => Err(why),
            None => p,
        }
    };
    let top = ui.box_rect()[1];
    bindings(ui, id, f, s, lock, plan);
    // the list
    heading(ui, "Zugriffslisten");
    let names = s.acls.names();
    if !names.iter().any(|x| *x == f.list) {
        f.list = names.first().cloned().unwrap_or_default();
    }
    let labels: Vec<String> = names
        .iter()
        .map(|x| {
            let a = s.acls.lists.get(x);
            let rules = a.map_or(0, |a| a.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).count());
            let kind = if a.is_some_and(|a| a.kind == AclKind::Standard) { t("Standard") } else { t("erweitert") };
            format!("{} ({}, {})", x, kind, if rules == 1 { String::from(t("1 Regel")) } else { tf("{} Regeln", &[&rules.to_string()]) })
        })
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
    if refs.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Noch keine Liste: ohne Liste passiert alles."));
    } else {
        let mut sel = names.iter().position(|x| *x == f.list).unwrap_or(0) as i32;
        ui.next_size(240, 0);
        if ui.combobox(id + 0x10, &mut sel, &refs) & EV_ACT != 0 {
            f.list = names.get(sel as usize).cloned().unwrap_or_default();
            f.rule = None;
            f.err.clear();
            f.grid.select_only(None);
            ui.repass();
        }
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    if icon_button(ui, id + 0x11, IC_ADD, t("Neue Liste"), true) {
        f.new_open = !f.new_open;
        f.err.clear();
        ui.repass();
    }
    let chosen = s.acls.lists.get(&f.list);
    if icon_button(ui, id + 0x12, IC_DELETE, t("Liste löschen"), chosen.is_some()) {
        let if_name = |i: u16| s.ifs.iter().find(|x| x.index == i).map_or_else(String::new, |x| x.short.clone());
        let lines = del_list_lines(&f.list, &s.acls, &s.rules, &if_name);
        *plan = Some(offer(lines.map(|lines| Plan { label: tf("Liste {} löschen", &[&f.list]), lines, pick: None, list: None, close: true })));
    }
    if icon_button(ui, id + 0x13, IC_REFRESH, t("Zähler löschen"), chosen.is_some()) {
        *plan = Some(offer(Ok(Plan { label: tf("Zähler von Liste {} löschen", &[&f.list]), lines: alloc::vec![format!("do clear access-list counters {}", f.list)], pick: None, list: None, close: false })));
    }
    ui.box_end();
    ui.box_end();
    // a new list
    if f.new_open {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10) | lock, 6);
        field(ui, "Name oder Nummer");
        ui.next_size(180, 0);
        ui.textbox(id + 0x15, Some(t("z. B. 110 oder WEB")), &mut f.new_name, 40);
        ui.box_end();
        field(ui, "Art");
        ui.next_size(180, 0);
        let kinds = [t("Standard (nur die Quelle)"), t("erweitert (alles)")];
        ui.combobox(id + 0x16, &mut f.new_kind, &kinds);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.button(id + 0x18, t("Abbrechen")) & EV_ACT != 0 {
            f.new_open = false;
            f.err.clear();
            ui.repass();
        }
        if ui.accent_button(id + 0x17, t("Anlegen")) & EV_ACT != 0 {
            let made = new_list_lines(&f.new_name, f.new_kind, &s.acls);
            *plan = Some(offer(made.map(|(name, lines)| Plan { label: tf("Liste {} anlegen", &[&name]), lines, pick: None, list: Some(name), close: true })));
        }
        ui.box_end();
        ui.box_end();
    }
    let Some(a) = chosen else {
        return;
    };
    // what the list means now
    let where_: Vec<String> = s.acls.bound.iter().filter(|(_, l)| **l == f.list).map(|((i, out), _)| format!("{} {}", s.ifs.iter().find(|x| x.index == *i).map_or("?", |x| x.short.as_str()), t(DIRECTIONS[*out as usize]))).collect();
    let by_nat = s.rules.iter().any(|r| matches!(r, Rule::List { acl, .. } if *acl == f.list));
    let meaning = if !a.has_rules() {
        String::from(t("Diese Liste hat keine Regeln: gebunden oder nicht, alles passiert."))
    } else if where_.is_empty() && by_nat {
        String::from(t("Diese Liste filtert nichts: NAT wählt mit ihr die Hosts aus, die es übersetzt."))
    } else if where_.is_empty() {
        String::from(t("Diese Liste ist an keine Schnittstelle gebunden: sie filtert nichts, alles passiert."))
    } else {
        tf("Gebunden an {}. Was keine Regel erlaubt, verwirft das implizite deny am Ende.", &[&where_.join(", ")])
    };
    note(ui, &meaning);
    // the rows
    let mut rows = rows_of(a);
    rows.truncate(ROWS_MAX);
    if let Some(l) = f.pick.take() {
        if let Some(r) = rows.iter().find(|r| r.line == l) {
            f.grid.select_only(Some(r.key));
        }
    }
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    f.grid.retain(&keys);
    let wide = ui.px2dip(ui.box_rect()[2]) >= 710;
    let cols = rule_cols(wide);
    ui.next_size(-1, 34 + 28 * rows.len().clamp(3, 8) as i32);
    let grid = id + 0x400;
    let (first, end) = ui.grid_begin(grid, &mut f.grid, &cols, &keys);
    let mut del_key = false;
    for i in first..end {
        let r = &rows[i as usize];
        ui.grid_row(&mut f.grid, i);
        let pos = if r.seq == 0 { String::from("–") } else { r.seq.to_string() };
        let hits = r.hits.map_or_else(|| String::from("–"), |h| h.to_string());
        let ts = if r.implicit { TS_BODY | TX_SECONDARY } else { TS_BODY };
        ui.grid_text_ts(&f.grid, 0, ts, &pos);
        if wide {
            ui.grid_text_ts(&f.grid, 1, ts, &r.act);
            ui.grid_text_ts(&f.grid, 2, ts, &r.proto);
            ui.grid_text_ts(&f.grid, 3, ts, &r.src);
            ui.grid_text_ts(&f.grid, 4, ts, &r.dst);
            ui.grid_text_ts(&f.grid, 5, ts, &r.port);
            ui.grid_text_ts(&f.grid, 6, ts, if r.log { "log" } else { "" });
            ui.grid_text_ts(&f.grid, 7, ts, &hits);
            ui.grid_text_ts(&f.grid, 8, ts, &r.remark);
        } else {
            ui.grid_text_ts(&f.grid, 1, ts, &if r.implicit { tf("{} (implizit)", &[&r.line]) } else { r.line.clone() });
            ui.grid_text_ts(&f.grid, 2, ts, &hits);
        }
        // the row in one sentence, for screen readers
        let say = if r.implicit {
            String::from(t("Am Ende: implizites deny"))
        } else if r.seq == 0 {
            tf("Bemerkung: {}", &[&r.remark])
        } else {
            tf("Zeile {}: {}, {} Treffer", &[&pos, &r.line, &hits])
        };
        ui.sem(K_TEXT, 0, Some(&say), 0);
    }
    let ev = ui.grid_end(&mut f.grid);
    if ui.focused() == grid && ui.key(VK_DELETE) {
        del_key = true;
    }
    let cur = f.grid.cur.and_then(|c| rows.iter().find(|r| r.key == c)).cloned();
    let is_rule = cur.as_ref().is_some_and(|r| r.seq != 0);
    // the row chosen, in full
    if let Some(r) = &cur {
        let mut text = if r.implicit { tf("{} (implizit)", &[&r.line]) } else { r.line.clone() };
        if !r.remark.is_empty() && !r.implicit {
            text = format!("{}  ·  {}", text, r.remark);
        }
        note(ui, &text);
    }
    // the buttons of the rows
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 6);
    let mut open_edit = ev.ev & EV_DOUBLE != 0 && is_rule;
    let mut open_dup = false;
    if icon_button(ui, id + 0x1A, IC_ADD, t("Neu"), true) {
        f.fields = RuleForm::new();
        // (after the rule that is chosen, else at the end)
        let after = cur.as_ref().filter(|r| r.seq != 0).map(|r| r.seq);
        f.rule = Some(RuleMode::Add(after));
        f.reveal = 2;
        f.err.clear();
        ui.repass();
    }
    if icon_button(ui, id + 0x1B, IC_EDIT, t("Bearbeiten"), is_rule) {
        open_edit = true;
    }
    if icon_button(ui, id + 0x1C, IC_COPY, t("Duplizieren"), is_rule) {
        open_dup = true;
    }
    let seq = cur.as_ref().map_or(0, |r| r.seq);
    let rule_at = a.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).position(|l| l.seq == seq);
    let count = a.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).count();
    let mut moved: Option<bool> = None;
    ui.next_flags(ST_ICON_ONLY | if rule_at.is_some_and(|i| i > 0) { 0 } else { ST_DISABLED });
    ui.next_icon(IC_UP);
    if ui.button(id + 0x1D, t("Nach oben")) & EV_ACT != 0 {
        moved = Some(true);
    }
    ui.next_flags(ST_ICON_ONLY | if rule_at.is_some_and(|i| i + 1 < count) { 0 } else { ST_DISABLED });
    ui.next_icon(IC_DOWN);
    if ui.button(id + 0x1E, t("Nach unten")) & EV_ACT != 0 {
        moved = Some(false);
    }
    let can_del = cur.as_ref().is_some_and(|r| !r.implicit);
    if icon_button(ui, id + 0x1F, IC_DELETE, t("Löschen"), can_del) || (del_key && can_del && lock == 0) {
        if let Some(r) = &cur {
            let lines = delete_lines(&f.list, a, r.key);
            *plan = Some(offer(Ok(Plan { label: tf("Zeile aus Liste {} löschen", &[&f.list]), lines, pick: None, list: None, close: true })));
        }
    }
    ui.box_end();
    if let Some(up) = moved
        && let Some(lines) = move_lines(&f.list, a, seq, up)
    {
        let pick = cur.as_ref().map(|r| r.line.clone());
        *plan = Some(offer(Ok(Plan { label: tf("Regel in Liste {} verschieben", &[&f.list]), lines, pick, list: None, close: false })));
    }
    let chosen_rule = || a.lines.iter().find(|l| l.seq == seq && matches!(l.item, Item::Rule(_))).and_then(|l| if let Item::Rule(r) = &l.item { Some(r.clone()) } else { None });
    if (open_edit || open_dup)
        && let Some(r) = chosen_rule()
    {
        f.fields = form_of(&r, a.kind);
        if open_edit {
            f.fields.remark = cur.as_ref().map(|c| c.remark.clone()).unwrap_or_default();
            f.rule = Some(RuleMode::Edit(seq));
        } else {
            f.rule = Some(RuleMode::Add(Some(seq)));
        }
        f.reveal = 2;
        f.err.clear();
        ui.repass();
    }
    if f.rule.is_some() {
        rule_form(ui, id, f, s, a, lock, top, plan);
    }
    note(ui, t("Jeder Schritt läuft in der Konsole und wird zusammen mit der laufenden Konfiguration zur Startkonfiguration. Eine verschobene oder geänderte Regel beginnt bei 0 Treffern; steht ihr eine Bemerkung im Weg, gilt das auch für die Regeln darunter. Rückgängig startet das Gerät neu: dann beginnen alle Zähler und Übersetzungen bei 0."));
}

// a label and its field in a row of the rule form (the field follows, then box_end)
fn group(ui: &mut Ui, label: &'static str, w: i32) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(w, 0);
    ui.text(TS_BODY | TX_SECONDARY, t(label));
}

// the form of a rule (extended lists: everything, standard lists: the source); rows of labelled fields
// that break when the window is narrow.  `top`: where the scroll view's content begins, to bring the
// form into view when it opens
fn rule_form(ui: &mut Ui, id: u32, f: &mut Form, s: &Snap, a: &Acl, lock: u32, top: i32, plan: &mut Option<Result<Plan, String>>) {
    let Some(mode) = f.rule else { return };
    let name = f.list.clone();
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10) | lock, 6);
    if f.reveal > 0 {
        f.reveal -= 1;
        if f.reveal == 0 {
            let y = ui.px2dip(ui.box_rect()[1] - top);
            ui.scroll_to(id + 4, -1, (y - 8).max(0));
            ui.focus(id + if f.fields.use_raw != 0 { 0x2F } else { 0x22 });
        }
        ui.repass();
    }
    let fl = &mut f.fields;
    let ext = a.kind == AclKind::Extended;
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
    ui.text(
        TS_BODY_STRONG,
        &match mode {
            RuleMode::Add(None) => tf("Neue Regel am Ende von Liste {}", &[&name]),
            RuleMode::Add(Some(q)) => tf("Neue Regel nach Zeile {} von Liste {}", &[&q.to_string(), &name]),
            RuleMode::Edit(q) => tf("Zeile {} von Liste {} ändern", &[&q.to_string(), &name]),
        },
    );
    if ui.checkbox(id + 0x2E, t("als Zeile eingeben"), &mut fl.use_raw) & EV_ACT != 0 {
        // (the line starts from what the fields say)
        if fl.use_raw != 0 && fl.raw.trim().is_empty() {
            let fields = RuleForm { use_raw: 0, ..fl.clone() };
            fl.raw = rule_of_form(a.kind, &fields, &s.name).map_or_else(|_| String::new(), |r| r.config(a.kind));
        }
        ui.repass();
    }
    ui.box_end();
    if fl.use_raw != 0 {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(70, 0);
        ui.text(TS_BODY | TX_SECONDARY, t("Zeile"));
        ui.next_size(-1, 0);
        ui.textbox(id + 0x2F, Some(t("z. B. permit 112 any any")), &mut fl.raw, 200);
        ui.box_end();
    } else {
        let plist = protos();
        let proto = plist.get(fl.proto as usize).map_or(None, |p| p.1);
        let ported = ext && matches!(proto, Some(PROTO_TCP | PROTO_UDP));
        let ops: Vec<&str> = PORT_OPS.iter().map(|x| t(x)).collect();
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
        group(ui, "Aktion", 70);
        ui.next_size(170, 0);
        let items = [t(ACTIONS[0]), t(ACTIONS[1])];
        ui.combobox(id + 0x20, &mut fl.act, &items);
        ui.box_end();
        if ext {
            group(ui, "Protokoll", 70);
            ui.next_size(100, 0);
            let items: Vec<&str> = plist.iter().map(|p| p.0).collect();
            if ui.combobox(id + 0x21, &mut fl.proto, &items) & EV_ACT != 0 {
                ui.repass();
            }
            ui.box_end();
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
        group(ui, "Quelle", 70);
        ui.next_size(190, 0);
        ui.textbox(id + 0x22, Some(t("any, 10.0.0.5, 10.0.0.0/24")), &mut fl.src, 40);
        ui.box_end();
        if ported {
            group(ui, "Port", 34);
            port_row(ui, id + 0x23, &ops, &mut fl.sop, &mut fl.sp1, &mut fl.sp2);
            ui.box_end();
        }
        ui.box_end();
        if ext {
            ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
            group(ui, "Ziel", 70);
            ui.next_size(190, 0);
            ui.textbox(id + 0x26, Some(t("any, 10.0.0.5, 10.0.0.0/24")), &mut fl.dst, 40);
            ui.box_end();
            if ported {
                group(ui, "Port", 34);
                port_row(ui, id + 0x27, &ops, &mut fl.dop, &mut fl.dp1, &mut fl.dp2);
                ui.box_end();
            }
            ui.box_end();
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
        if ext && proto == Some(PROTO_ICMP) {
            group(ui, "ICMP-Nachricht", 110);
            ui.next_size(200, 0);
            let mut items: Vec<&str> = alloc::vec![t("jede")];
            items.extend(ICMP_TYPES.iter().map(|x| x.0));
            ui.combobox(id + 0x2B, &mut fl.icmp, &items);
            ui.box_end();
        }
        if ext && proto == Some(PROTO_TCP) {
            ui.checkbox(id + 0x2A, t("nur bestehende Verbindungen (established)"), &mut fl.established);
        }
        ui.checkbox(id + 0x2C, t("Treffer in der Konsole melden (log)"), &mut fl.log);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(70, 0);
    ui.text(TS_BODY | TX_SECONDARY, t("Bemerkung"));
    ui.next_size(-1, 0);
    ui.textbox(id + 0x2D, Some(t("steht über der Regel")), &mut fl.remark, 100);
    ui.box_end();
    // the line it will be
    let preview = if fl.use_raw != 0 { Some(String::from(fl.raw.trim())) } else { rule_of_form(a.kind, fl, &s.name).ok().map(|r| r.config(a.kind)) };
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &preview.filter(|x| !x.is_empty()).map_or_else(String::new, |l| tf("Wird zu: {}", &[&l])));
    if ui.button(id + 0x31, t("Abbrechen")) & EV_ACT != 0 {
        f.rule = None;
        f.err.clear();
        ui.repass();
    } else if ui.accent_button(id + 0x30, t("Übernehmen")) & EV_ACT != 0 {
        let made = rule_of_form(a.kind, fl, &s.name).and_then(|r| {
            let lines = match mode {
                RuleMode::Add(after) => add_lines(&name, a, &r, &fl.remark, after)?,
                RuleMode::Edit(q) => edit_lines(&name, a, q, &r, &fl.remark)?,
            };
            Ok(Plan { label: tf("Regel in Liste {} ändern", &[&name]), lines, pick: Some(r.config(a.kind)), list: None, close: true })
        });
        *plan = Some(match refusal(s.routes, s.locked.as_deref(), &s.name) {
            Some(why) => Err(why),
            None => made,
        });
    }
    ui.box_end();
    ui.box_end();
}

// a port condition's fields: the operator, one port or a range's two ends
fn port_row(ui: &mut Ui, id: u32, ops: &[&str], op: &mut i32, a: &mut String, b: &mut String) {
    ui.next_size(120, 0);
    if ui.combobox(id, op, ops) & EV_ACT != 0 {
        ui.repass();
    }
    if *op > 0 {
        ui.next_size(64, 0);
        ui.textbox(id + 1, Some(t("Port")), a, 16);
    }
    if *op == 5 {
        ui.next_size(64, 0);
        ui.textbox(id + 2, Some(t("bis")), b, 16);
    }
}

// the columns of the translations (narrow: without the outside and the kind)
fn xlate_cols(wide: bool) -> Vec<GridCol<'static>> {
    let mut v = alloc::vec![GridCol::new(t("Protokoll"), 72).flags(GC_NOSORT), GridCol::new(t("Innen global"), 150).flags(GC_NOSORT), GridCol::new(t("Innen lokal"), 150).flags(GC_NOSORT | GC_FILL)];
    if wide {
        v.push(GridCol::new(t("Außen"), 150).flags(GC_NOSORT));
        v.push(GridCol::new(t("Art"), 84).flags(GC_NOSORT));
        v.push(GridCol::new(t("Endet in"), 84).flags(GC_NOSORT | GC_RIGHT));
    }
    v
}

// an endpoint of the table
fn endpoint(e: (Ipv4, Option<u16>)) -> String {
    match e.1 {
        Some(p) => format!("{}:{}", e.0, p),
        None => e.0.to_string(),
    }
}

// a key for a row (FNV-1a)
pub(super) fn key_of(s: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

// NAT: the sides, pools, dynamic and static rules, the translations
#[allow(clippy::too_many_arguments)]
fn part_nat(ui: &mut Ui, id: u32, f: &mut Form, s: &Snap, lock: u32, rows: &[jfn_sim::XlateRow], now: jfn_sim::Ns, plan: &mut Option<Result<Plan, String>>) {
    let offer = |p: Result<Plan, String>| -> Result<Plan, String> {
        match refusal(s.routes, s.locked.as_deref(), &s.name) {
            Some(why) => Err(why),
            None => p,
        }
    };
    let if_names: Vec<String> = s.ifs.iter().map(|x| x.name.clone()).collect();
    let if_short: Vec<&str> = s.ifs.iter().map(|x| x.short.as_str()).collect();
    // (the lists of the comboboxes change elsewhere)
    let last = |n: usize| (n as i32 - 1).max(0);
    f.dyn_list = f.dyn_list.clamp(0, last(s.acls.lists.len()));
    f.dyn_pool = f.dyn_pool.clamp(0, last(s.pools.len()));
    f.dyn_if = f.dyn_if.clamp(0, last(s.ifs.len()));
    f.stat.gif = f.stat.gif.clamp(0, last(s.ifs.len()));
    let label = tf("NAT auf {} ändern", &[&s.name]);
    let run = |lines: Vec<String>| Plan { label: label.clone(), lines, pick: None, list: None, close: false };
    // the sides
    heading(ui, "Schnittstellen");
    note(ui, t("Innen stehen die Hosts, deren Adressen übersetzt werden; außen das Netz, das die globalen Adressen sieht. Erst wenn eine Schnittstelle eine Seite hat, übersetzt das Gerät."));
    let sides = [t("keine Seite"), t("innen"), t("außen")];
    ui.box_begin(0, BOX_V | lock, 4);
    for (i, x) in s.ifs.iter().filter(|x| f.all_ifs != 0 || !x.addr.is_empty() || s.sides.contains_key(&x.index)).enumerate().take(64) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        ui.next_size(170, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &if x.addr.is_empty() { x.short.clone() } else { format!("{}  {}", x.short, x.addr) });
        let cur = s.sides.get(&x.index).copied();
        let mut sel = match cur {
            None => 0,
            Some(Side::Inside) => 1,
            Some(Side::Outside) => 2,
        };
        ui.next_size(130, 0);
        if ui.combobox(id + 0x180 + i as u32, &mut sel, &sides) & EV_ACT != 0 {
            let new = match sel {
                1 => Some(Side::Inside),
                2 => Some(Side::Outside),
                _ => None,
            };
            *plan = Some(offer(Ok(run(side_lines(&x.name, cur, new)))));
        }
        ui.box_end();
    }
    ui.box_end();
    // pools
    heading(ui, "Adresspools");
    for (i, (name, p)) in s.pools.iter().enumerate().take(32) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{}: {} – {} /{}", name, p.start, p.end, p.len));
        if remove_button(ui, id + 0x1C0 + i as u32, &tf("Pool {} entfernen", &[name])) {
            // (an address in use: the device would refuse it, a scratch copy has no translations)
            let used = rows.iter().any(|r| r.dynamic && p.contains(r.inside_global.0));
            *plan = Some(offer(if used { Err(tf("Pool {} ist in Gebrauch: erst die laufenden Übersetzungen löschen.", &[name])) } else { pool_del_lines(name, &s.rules).map(&run) }));
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.next_size(120, 0);
    ui.textbox(id + 0x40, Some(t("Name")), &mut f.pool[0], 32);
    ui.next_size(130, 0);
    ui.textbox(id + 0x41, Some(t("erste Adresse")), &mut f.pool[1], 16);
    ui.next_size(130, 0);
    ui.textbox(id + 0x42, Some(t("letzte Adresse")), &mut f.pool[2], 16);
    ui.next_size(120, 0);
    ui.textbox(id + 0x43, Some(t("Netz, z. B. /24")), &mut f.pool[3], 16);
    if ui.button(id + 0x44, t("Pool anlegen")) & EV_ACT != 0 {
        *plan = Some(offer(pool_lines(&f.pool[0], &f.pool[1], &f.pool[2], &f.pool[3]).map(&run)));
    }
    ui.box_end();
    // dynamic rules
    heading(ui, "Viele Hosts: dynamisch und PAT");
    let dynamic: Vec<&Rule> = s.rules.iter().filter(|r| matches!(r, Rule::List { .. })).collect();
    for (i, r) in dynamic.iter().enumerate().take(32) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &rule_text(r));
        if remove_button(ui, id + 0x1E0 + i as u32, &tf("{} entfernen", &[&rule_text(r)])) {
            *plan = Some(offer(Ok(run(alloc::vec![rule_del_line(r)]))));
        }
        ui.box_end();
    }
    let names = s.acls.names();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.text(TS_BODY | TX_SECONDARY, t("Liste"));
    if refs.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("(noch keine)"));
    } else {
        ui.next_size(110, 0);
        ui.combobox(id + 0x45, &mut f.dyn_list, &refs);
    }
    ui.text(TS_BODY | TX_SECONDARY, t("über"));
    let targets = [t("die Adresse von"), t("den Pool")];
    ui.next_size(150, 0);
    if ui.combobox(id + 0x46, &mut f.dyn_to, &targets) & EV_ACT != 0 {
        ui.repass();
    }
    let pools: Vec<&str> = s.pools.iter().map(|p| p.0.as_str()).collect();
    if f.dyn_to == 0 {
        ui.next_size(110, 0);
        ui.combobox(id + 0x47, &mut f.dyn_if, &if_short);
    } else if pools.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("(noch keiner)"));
    } else {
        ui.next_size(110, 0);
        ui.combobox(id + 0x48, &mut f.dyn_pool, &pools);
        ui.checkbox(id + 0x49, t("PAT (overload)"), &mut f.dyn_over);
    }
    if ui.button(id + 0x4A, t("Regel anlegen")) & EV_ACT != 0 {
        let acl = names.get(f.dyn_list as usize);
        let to = if f.dyn_to == 0 { if_names.get(f.dyn_if as usize).map(|i| Target::Iface(i.clone())) } else { pools.get(f.dyn_pool as usize).map(|p| Target::Pool(String::from(*p))) };
        let made = match (acl, to) {
            (None, _) => Err(String::from(t("NAT wählt die Hosts mit einer Zugriffsliste aus: erst eine anlegen (Regeln)."))),
            (_, None) => Err(String::from(t("Es gibt noch keinen Pool: erst einen anlegen."))),
            (Some(l), Some(to)) => Ok(run(dyn_lines(l, &to, f.dyn_to == 0 || f.dyn_over != 0))),
        };
        *plan = Some(offer(made));
    }
    ui.box_end();
    // static rules and port forwarding
    heading(ui, "Fest: statische Übersetzung und Portweiterleitung");
    let statics: Vec<&Rule> = s.rules.iter().filter(|r| matches!(r, Rule::Static { .. })).collect();
    for (i, r) in statics.iter().enumerate().take(128) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &rule_text(r));
        if remove_button(ui, id + 0x200 + i as u32, &tf("{} entfernen", &[&rule_text(r)])) {
            *plan = Some(offer(Ok(run(alloc::vec![rule_del_line(r)]))));
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    let kinds = [t("die ganze Adresse"), t("einen TCP-Port"), t("einen UDP-Port")];
    ui.next_size(140, 0);
    if ui.combobox(id + 0x4B, &mut f.stat.kind, &kinds) & EV_ACT != 0 {
        ui.repass();
    }
    ui.next_size(130, 0);
    ui.textbox(id + 0x4C, Some(t("Adresse innen")), &mut f.stat.local, 16);
    if f.stat.kind != 0 {
        ui.next_size(96, 0);
        ui.textbox(id + 0x4D, Some(t("Port innen")), &mut f.stat.lport, 6);
    }
    ui.text(TS_BODY | TX_SECONDARY, "=");
    let gtypes = [t("Adresse außen"), t("Adresse von")];
    ui.next_size(130, 0);
    if ui.combobox(id + 0x4E, &mut f.stat.gtype, &gtypes) & EV_ACT != 0 {
        ui.repass();
    }
    if f.stat.gtype == 0 {
        ui.next_size(130, 0);
        ui.textbox(id + 0x4F, Some(t("Adresse außen")), &mut f.stat.global, 16);
    } else {
        ui.next_size(110, 0);
        ui.combobox(id + 0x50, &mut f.stat.gif, &if_short);
    }
    if f.stat.kind != 0 {
        ui.next_size(96, 0);
        ui.textbox(id + 0x51, Some(t("Port außen")), &mut f.stat.gport, 6);
    }
    if ui.button(id + 0x52, t("Übersetzung anlegen")) & EV_ACT != 0 {
        *plan = Some(offer(static_lines(&f.stat, &if_names).map(&run)));
    }
    ui.box_end();
    // the translations
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
    heading(ui, "Laufende Übersetzungen");
    ui.box_begin(0, BOX_H | lock, 0);
    if icon_button(ui, id + 0x53, IC_CLEAR, t("Übersetzungen löschen"), true) {
        *plan = Some(offer(Ok(run(alloc::vec![String::from("do clear ip nat translation *")]))));
    }
    ui.box_end();
    ui.box_end();
    let shown = &rows[..rows.len().min(ROWS_MAX)];
    let texts: Vec<[String; 6]> = shown
        .iter()
        .map(|r| {
            let pro = match r.proto {
                0 => String::from("–"),
                p => proto_text(Some(p)),
            };
            let out = r.outside.map_or_else(|| String::from("–"), |o| format!("{}:{}", o.0, o.1));
            let kind = String::from(t(if r.dynamic { "dynamisch" } else { "statisch" }));
            let ends = r.ends.map_or_else(|| String::from("–"), |e| format!("{} s", e.saturating_sub(now) / jfn_sim::SEC));
            [pro, endpoint(r.inside_global), endpoint(r.inside_local), out, kind, ends]
        })
        .collect();
    if texts.is_empty() {
        note(ui, t("Gerade läuft keine Übersetzung."));
    } else {
        let keys: Vec<u64> = texts.iter().map(|x| key_of(&format!("{}|{}|{}|{}", x[0], x[1], x[2], x[3]))).collect();
        f.xlate.retain(&keys);
        let wide = ui.px2dip(ui.box_rect()[2]) >= 640;
        let cols = xlate_cols(wide);
        ui.next_size(-1, 34 + 28 * texts.len().clamp(2, 8) as i32);
        let (first, end) = ui.grid_begin(id + 0x600, &mut f.xlate, &cols, &keys);
        for i in first..end {
            ui.grid_row(&mut f.xlate, i);
            for c in 0..cols.len() {
                ui.grid_text(&f.xlate, c, &texts[i as usize][c]);
            }
        }
        ui.grid_end(&mut f.xlate);
        if rows.len() > ROWS_MAX {
            note(ui, &tf("Die ersten {} von {}; alle zeigt „show ip nat translations“.", &[&ROWS_MAX.to_string(), &rows.len().to_string()]));
        }
    }
}

// Test: which rule meets this packet
fn part_test(ui: &mut Ui, id: u32, f: &mut Form, s: &Snap, zones: Option<&FwTest>) {
    heading(ui, "Welche Regel trifft dieses Paket?");
    note(ui, t("Hier fragt das Fenster die Listen des Geräts, ohne ein Paket zu senden; die Zähler bleiben, wie sie sind. Eine eingehende Liste sieht das Paket, wie es ankommt, eine ausgehende nach dem Routing und nach NAT: dort zählt die übersetzte Quelle."));
    let if_short: Vec<&str> = s.ifs.iter().map(|x| x.short.as_str()).collect();
    f.test_if = f.test_if.clamp(0, (if_short.len() as i32 - 1).max(0));
    field(ui, "Schnittstelle");
    if if_short.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("(keine)"));
    } else {
        ui.next_size(150, 0);
        ui.combobox(id + 0x60, &mut f.test_if, &if_short);
    }
    let dirs = [t(DIRECTIONS[0]), t(DIRECTIONS[1])];
    ui.next_size(120, 0);
    ui.combobox(id + 0x61, &mut f.test_out, &dirs);
    ui.box_end();
    let plist = test_protos();
    let proto = plist.get(f.test.proto as usize).map_or(PROTO_TCP, |p| p.1);
    field(ui, "Protokoll");
    ui.next_size(150, 0);
    let items: Vec<&str> = plist.iter().map(|p| p.0).collect();
    if ui.combobox(id + 0x62, &mut f.test.proto, &items) & EV_ACT != 0 {
        ui.repass();
    }
    ui.box_end();
    field(ui, "Quelle");
    ui.next_size(180, 0);
    ui.textbox(id + 0x63, Some(t("z. B. 192.168.1.10")), &mut f.test.src, 16);
    ui.box_end();
    field(ui, "Ziel");
    ui.next_size(180, 0);
    ui.textbox(id + 0x64, Some(t("z. B. 198.51.100.10")), &mut f.test.dst, 16);
    ui.box_end();
    if matches!(proto, PROTO_TCP | PROTO_UDP) {
        field(ui, "Zielport");
        ui.next_size(100, 0);
        ui.textbox(id + 0x66, Some(t("z. B. 80 oder www")), &mut f.test.dport, 16);
        ui.box_end();
        field(ui, "Quellport");
        ui.next_size(100, 0);
        ui.textbox(id + 0x65, Some(t("frei")), &mut f.test.sport, 16);
        ui.box_end();
    }
    if proto == PROTO_TCP {
        ui.checkbox(id + 0x67, t("gehört zu einer bestehenden Verbindung (ACK)"), &mut f.test.ack);
    }
    if proto == PROTO_ICMP {
        field(ui, "ICMP-Nachricht");
        ui.next_size(220, 0);
        let items: Vec<&str> = ICMP_TYPES.iter().map(|x| x.0).collect();
        ui.combobox(id + 0x68, &mut f.test.icmp, &items);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.accent_button(id + 0x69, t("Prüfen")) & EV_ACT != 0 {
        f.err.clear();
        f.asked = None;
        match (s.ifs.get(f.test_if as usize), flow_of(&f.test)) {
            (None, _) => f.err = String::from(t("Das Gerät hat keine Schnittstelle dafür.")),
            (_, Err(e)) => f.err = e,
            (Some(x), Ok(fl)) => f.asked = Some((x.index, f.test_out != 0, fl)),
        }
        ui.repass();
    }
    ui.box_end();
    // the answer, with the lists as they are now
    if let Some((iface, out, fl)) = &f.asked {
        let name = s.ifs.iter().find(|x| x.index == *iface).map_or("?", |x| x.short.as_str());
        let a = told(&s.acls, *iface, name, *out, fl);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(if a.pass { BADGE_OK } else { BADGE_CRIT });
        ui.text(TS_BODY_STRONG, t(if a.pass { "Passiert" } else { "Wird verworfen" }));
        ui.box_end();
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &a.text);
        if !a.pass {
            note(ui, t("Der Absender bekommt „administratively prohibited“ (ICMP Typ 3, Code 13)."));
        }
        ui.box_end();
        // after the list come the zones and sessions (they see the inside addresses: fw.rs)
        if s.fw.active() && a.pass {
            if *out && s.sides.get(iface) == Some(&Side::Outside) {
                note(ui, t("Die Zonen sehen ein Paket vor der Übersetzung der Quelle: fragen Sie nach der inneren Quelle an der Schnittstelle, auf der es ankommt."));
            } else if let Some(x) = zones {
                let z = fwzones::told_zones(x);
                ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
                ui.status_dot(if z.pass { BADGE_OK } else { BADGE_CRIT });
                ui.text(TS_BODY_STRONG, t(if z.pass { "Zonen: passiert" } else { "Zonen: wird verworfen" }));
                ui.box_end();
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_WRAP, &z.text);
                ui.box_end();
            }
        }
    }
}
