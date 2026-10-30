// snmp -- SNMP at the command lines: the agent of IOS-like devices ("snmp-server ..."), its show
// and debug commands and its lines in the running configuration; the managers of the Linux
// shell (net-snmp's tools as console programs)
//
//   IOS-like   snmp-server community NAME [ro|rw [ACL]], snmp-server location|contact TEXT,
//              snmp-server host ADDR [traps|informs] [version 1|2c] COMMUNITY,
//              snmp-server enable traps [snmp [authentication] [linkdown] [linkup] [coldstart]],
//              their no-forms and "no snmp-server" (Action::Snmp, snmpd.rs); show snmp, show snmp
//              community; debug snmp packets (D_SNMP: the agent's notes as console lines)
//   Linux      snmpget, snmpgetnext, snmpwalk, snmpbulkwalk, snmpset [-v 1|2c] -c COMMUNITY
//              [-r RETRIES] [-t SECONDS] [-On|-Oq|-Ov|-Os] [-Cr N] [-Cn N] AGENT [OID ...]
//              (snmpset: OID TYPE VALUE ..., the types i u t a o s x d n and = for the MIB's)
//
// The managers are console programs (services on the console, as dig): they send from an
// ephemeral port, wait -t seconds (1) for the answer with the request's id, try -r more times (5)
// and print the bindings in net-snmp's form ("SNMPv2-MIB::sysName.0 = STRING: R1", names from
// netpkt::mib, numbers with -On).  The agent is an address or a name (the device's resolver), a
// port after a colon, "udp:" or "udp6:" before.  snmpwalk asks GetNext from the root (mib-2
// without one) until an answer leaves it, version 2c's endOfMibView or version 1's noSuchName
// end it; a root with nothing below is asked with Get, as net-snmp does.  snmpbulkwalk asks
// GetBulk with -Cr repetitions (10).  The version is 2c unless -v says 1.  Help, usage and
// error texts are our own words, so are the exceptions' texts (mib::show); the IOS-like show
// output and the debug lines too.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::cmdtree::{ModeId, Parsed, Tree};
use netpkt::ber::{Oid, Value};
use netpkt::mib::{self, Syntax};
use netpkt::snmp::{self, Kind, Message, Pdu, VarBind, Version};
use netpkt::{IpAddr, Ipv4};

use super::ios::K;
use super::D_SNMP;
use crate::action::Action;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent, ANY, ANY4};
use crate::resolver::Family;
use crate::snmpd::{notify_words, SnmpSet, TrapHost, N_ALL, N_AUTH, N_COLDSTART, N_LINKDOWN, N_LINKUP};
use crate::snmptrace::SNote;
use crate::trace::Note;

/// the timer of a manager: the wait for an answer
const K_WAIT: u32 = 0;

// ---------------------------------------------------------------- the IOS-like commands

/// the IOS commands of SNMP
pub(super) fn build_snmp(t: &mut Tree<K>, m: [ModeId; 3]) {
    let [user, prv, conf] = m;
    for md in [user, prv] {
        t.add(md, "show snmp", "The SNMP agent: its counters and receivers", Some(K::ShowSnmp), 0);
        t.add(md, "show snmp community", "The communities and who may use them", Some(K::ShowSnmp), 0);
    }
    t.add(prv, "debug snmp", "SNMP messages", None, 0);
    t.add(prv, "debug snmp packets", "Each request, answer and notification of the agent", Some(K::DebugSnmp), 0);
    t.add(prv, "undebug snmp", "SNMP messages", None, 0);
    t.add(prv, "undebug snmp packets", "Each request, answer and notification of the agent", Some(K::DebugSnmp), 0);
    let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>, f: u8| {
        t.add(conf, p, h, c, f);
    };
    a(t, "snmp-server", "Settings of the SNMP agent", None, base::cmdtree::F_NO);
    t.add_no(conf, "snmp-server", K::SnmpOff);
    a(t, "snmp-server community", "A community: the shared word of SNMPv1 and v2c", None, 0);
    a(t, "snmp-server community WORD=comm", "Its name (read-only unless rw follows)", Some(K::SnmpCommunity), 0);
    a(t, "snmp-server community WORD=comm ro=access", "May only read", Some(K::SnmpCommunity), 0);
    a(t, "snmp-server community WORD=comm rw=access", "May read and write", Some(K::SnmpCommunity), 0);
    a(t, "snmp-server community WORD=comm ro=access WORD=acl", "Number or name of the standard access list that may use it", Some(K::SnmpCommunity), 0);
    a(t, "snmp-server community WORD=comm rw=access WORD=acl", "Number or name of the standard access list that may use it", Some(K::SnmpCommunity), 0);
    t.add_no(conf, "snmp-server community WORD=comm", K::SnmpCommunity);
    a(t, "snmp-server location", "Where the device stands (sysLocation)", None, 0);
    a(t, "snmp-server location LINE=text", "The place", Some(K::SnmpLocation), 0);
    t.add_no(conf, "snmp-server location", K::SnmpLocation);
    a(t, "snmp-server contact", "Who looks after the device (sysContact)", None, 0);
    a(t, "snmp-server contact LINE=text", "A name, a mail address", Some(K::SnmpContact), 0);
    t.add_no(conf, "snmp-server contact", K::SnmpContact);
    a(t, "snmp-server host", "A receiver of notifications", None, 0);
    a(t, "snmp-server host A.B.C.D=host", "Its address", None, 0);
    for pre in ["", " traps=kind", " informs=kind"] {
        let at = format!("snmp-server host A.B.C.D=host{}", pre);
        if pre.contains("traps") {
            a(t, &at, "Notifications without an acknowledgement", None, 0);
        } else if pre.contains("informs") {
            a(t, &at, "Notifications the receiver acknowledges (version 2c)", None, 0);
        }
        a(t, &format!("{} WORD=comm", at), "The community the notifications carry", Some(K::SnmpHost), 0);
        a(t, &format!("{} version", at), "The SNMP version of the notifications", None, 0);
        a(t, &format!("{} version 1=ver", at), "SNMPv1 (traps only)", None, 0);
        a(t, &format!("{} version 2c=ver", at), "SNMPv2c", None, 0);
        a(t, &format!("{} version 1=ver WORD=comm", at), "The community the notifications carry", Some(K::SnmpHost), 0);
        a(t, &format!("{} version 2c=ver WORD=comm", at), "The community the notifications carry", Some(K::SnmpHost), 0);
    }
    t.add_no(conf, "snmp-server host A.B.C.D=host", K::SnmpHost);
    a(t, "snmp-server enable", "Switch notifications on", None, 0);
    a(t, "snmp-server enable traps", "Notifications (every kind below without more)", Some(K::SnmpTraps), 0);
    a(t, "snmp-server enable traps snmp", "Those of the agent and the ports (all four without more)", Some(K::SnmpTraps), 0);
    let base = "snmp-server enable traps snmp";
    let opts = [("authentication", "A request with a community that was refused"), ("linkdown", "A port's link went down"), ("linkup", "A port's link came up"), ("coldstart", "The agent started")];
    for (w, h) in opts {
        a(t, &format!("{} {}", base, w), h, Some(K::SnmpTraps), 0);
    }
    for (w, _) in opts {
        t.also(conf, &format!("{} {}", base, w), base);
    }
}

impl Engine {
    /// the commands of SNMP
    pub(crate) fn ios_snmp(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let set = match p.cmd {
            K::SnmpOff => SnmpSet::Off,
            K::SnmpCommunity => {
                let name = String::from(p.text("comm").unwrap_or(""));
                if p.no {
                    SnmpSet::NoCommunity { name }
                } else {
                    let acl = p.text("acl").map(String::from);
                    if let Some(n) = acl.as_deref().and_then(|a| a.parse::<u32>().ok())
                        && crate::acl::AclKind::of_number(n) != Some(crate::acl::AclKind::Standard)
                    {
                        return self.ios_err(ni, &format!("% Access list {} is no standard list: SNMP takes 1-99 and 1300-1999\n", n));
                    }
                    SnmpSet::Community { name, rw: p.text("access") == Some("rw"), acl }
                }
            }
            K::SnmpLocation => SnmpSet::Location(if p.no { None } else { p.text("text").map(String::from) }),
            K::SnmpContact => SnmpSet::Contact(if p.no { None } else { p.text("text").map(String::from) }),
            K::SnmpHost => {
                let addr = Ipv4(p.ip("host").unwrap_or([0; 4]));
                let community = p.text("comm").map(String::from);
                if p.no {
                    SnmpSet::NoHost { addr, community }
                } else {
                    let inform = p.text("kind") == Some("informs");
                    let version = if p.text("ver") == Some("2c") { Version::V2c } else { Version::V1 };
                    if inform && version == Version::V1 {
                        return self.ios_err(ni, "% Informs need version 2c\n");
                    }
                    if addr.is_unspecified() || addr.is_multicast() || addr.is_broadcast() {
                        return self.ios_err(ni, &format!("% {} cannot receive notifications\n", addr));
                    }
                    SnmpSet::Host(TrapHost { addr, port: snmp::PORT_TRAP, inform, version, community: community.unwrap_or_default() })
                }
            }
            K::SnmpTraps => {
                let mut bits = 0;
                for (w, b) in [("authentication", N_AUTH), ("linkdown", N_LINKDOWN), ("linkup", N_LINKUP), ("coldstart", N_COLDSTART)] {
                    if p.has(w) {
                        bits |= b;
                    }
                }
                SnmpSet::Notify { bits: if bits == 0 { N_ALL } else { bits }, on: !p.no }
            }
            K::ShowSnmp => {
                let text = if p.has("community") { self.show_snmp_community(ni) } else { self.show_snmp(ni) };
                self.out(ni, &text);
                return Ok(());
            }
            K::DebugSnmp => {
                let off = p.has("undebug") || p.no;
                let before = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
                let after = if off { before & !D_SNMP } else { before | D_SNMP };
                if let Some(c) = self.cdev_mut(ni) {
                    c.debug = after;
                }
                if before == 0 && after != 0 {
                    self.cli.debug += 1;
                } else if before != 0 && after == 0 {
                    self.cli.debug = self.cli.debug.saturating_sub(1);
                }
                self.out(ni, &format!("Debug output for SNMP packets: {}\n", if off { "off" } else { "on" }));
                return Ok(());
            }
            _ => return Ok(()),
        };
        let what = match &set {
            SnmpSet::NoCommunity { name } => format!("% No community {} here\n", name),
            SnmpSet::NoHost { addr, .. } => format!("% {} receives no notifications here\n", addr),
            _ => String::from("% SNMP is not possible on this device\n"),
        };
        match self.apply(Action::Snmp { dev, set }) {
            Ok(()) => Ok(()),
            Err(_) => self.ios_err(ni, &what),
        }
    }

    /// the lines of the running configuration ("snmp-server ...")
    pub(crate) fn ios_snmp_config(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let Some(a) = self.snmp_agent(dev) else { return String::new() };
        let c = &a.cfg;
        let mut s = String::new();
        for k in &c.communities {
            s.push_str(&format!("snmp-server community {} {}", k.name, if k.rw { "RW" } else { "RO" }));
            if let Some(acl) = &k.acl {
                s.push(' ');
                s.push_str(acl);
            }
            s.push('\n');
        }
        if !c.location.is_empty() {
            s.push_str(&format!("snmp-server location {}\n", c.location));
        }
        if !c.contact.is_empty() {
            s.push_str(&format!("snmp-server contact {}\n", c.contact));
        }
        if c.notify != 0 {
            s.push_str(&format!("snmp-server enable traps snmp {}\n", notify_words(c.notify).join(" ")));
        }
        for h in &c.hosts {
            let kind = if h.inform { "informs " } else { "" };
            let ver = if h.version == Version::V2c { "version 2c " } else { "" };
            s.push_str(&format!("snmp-server host {} {}{}{}\n", h.addr, kind, ver, h.community));
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// show snmp: the agent's counters and receivers (our own layout and words)
    fn show_snmp(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let Some(a) = self.snmp_agent(dev) else {
            return String::from("%SNMP agent not enabled\n");
        };
        let c = &a.counters;
        let up = mib::ticks_text(crate::snmpmib::ticks(a.started, self.now));
        let up = up.split_once(") ").map_or(up.as_str(), |x| x.1);
        let mut s = format!("SNMP agent of {} (versions 1 and 2c, UDP port {}), up for {}\n", self.host(ni), a.cfg.port, up);
        s.push_str(&format!("Contact:  {}\nLocation: {}\n", if a.cfg.contact.is_empty() { "-" } else { &a.cfg.contact }, if a.cfg.location.is_empty() { "-" } else { &a.cfg.location }));
        s.push_str(&format!("Received {} messages\n", c[1]));
        s.push_str(&format!("  {} with a version it does not speak, {} with an unknown community or from a sender kept out,\n", c[3], c[4]));
        s.push_str(&format!("  {} not allowed for their community, {} unreadable\n", c[5], c[6]));
        s.push_str(&format!("  {} Get, {} GetNext, {} Set; {} objects read, {} changed\n", c[15], c[16], c[17], c[13], c[14]));
        s.push_str(&format!("Sent {} messages\n", c[2]));
        s.push_str(&format!("  {} responses, {} with tooBig (more than {} bytes), {} noSuchName, {} badValue, {} genErr\n", c[28], c[20], crate::snmpd::MAX_MSG, c[21], c[22], c[24]));
        s.push_str(&format!("  {} notifications: {} traps, {} informs ({} acknowledged, {} given up, {} waiting)\n", c[29], a.traps, a.informs, a.informs_acked, a.informs_lost, a.pending()));
        let words = notify_words(a.cfg.notify);
        s.push_str(&format!("Notifications: {}\n", if words.is_empty() { String::from("none switched on") } else { words.join(" ") }));
        for h in &a.cfg.hosts {
            s.push_str(&format!("  to {} port {}: {}, version {}, community {}\n", h.addr, h.port, if h.inform { "informs" } else { "traps" }, h.version.word(), h.community));
        }
        s
    }

    /// show snmp community
    fn show_snmp_community(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let Some(a) = self.snmp_agent(dev) else {
            return String::from("%SNMP agent not enabled\n");
        };
        let mut s = String::from("Community         Access      Access list\n");
        for k in &a.cfg.communities {
            s.push_str(&format!("{:<17} {:<11} {}\n", k.name, if k.rw { "read-write" } else { "read-only" }, k.acl.as_deref().unwrap_or("-")));
        }
        s
    }

    /// "debug snmp packets": a decision of SNMP as a console line
    pub(crate) fn snmp_debug(&self, note: &Note) -> String {
        let Note::Snmp(n) = note else {
            return String::new();
        };
        match n {
            SNote::Request { from, kind, community, id, vars } => format!("SNMP: {} from {}, community {}, id {}, {} binding{}\n", kind.name(), from, community, id, vars, if *vars == 1 { "" } else { "s" }),
            SNote::Response { to, id, status: 0, vars, .. } => format!("SNMP: Response to {}, id {}, {} binding{}\n", to, id, vars, if *vars == 1 { "" } else { "s" }),
            SNote::Response { to, id, status, index, .. } => format!("SNMP: Response to {}, id {}: {} at binding {}\n", to, id, snmp::error_name(*status), index),
            SNote::BadCommunity { from, community } => format!("SNMP: no answer to {}: unknown community {}\n", from, community),
            SNote::Denied { from, community, .. } => format!("SNMP: no answer to {}: it may not use community {}\n", from, community),
            SNote::BadVersion { from, version } => format!("SNMP: dropped a message of version {} from {}\n", version, from),
            SNote::Malformed { from } => format!("SNMP: dropped an unreadable message from {}\n", from),
            SNote::ReadOnly { from, community } => format!("SNMP: Set from {} refused: community {} may only read\n", from, community),
            SNote::Set { var, value } => format!("SNMP: {} = {}\n", var, value),
            SNote::Notify { to, what, inform, version } => format!("SNMP: {} {} to {}, version {}\n", if *inform { "inform" } else { "trap" }, what, to, version.word()),
            SNote::InformAgain { to, what, n } => format!("SNMP: inform {} to {} again (attempt {})\n", what, to, n),
            SNote::InformAcked { to, what } => format!("SNMP: {} acknowledged inform {}\n", to, what),
            SNote::InformLost { to, what } => format!("SNMP: inform {} to {} given up\n", what, to),
            SNote::NoRoute { to } => format!("SNMP: no route to receiver {}\n", to),
            _ => String::new(),
        }
    }
}

// ---------------------------------------------------------------- the managers of the shell

/// which tool
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prog {
    Get,
    GetNext,
    Walk,
    BulkWalk,
    Set,
}

impl Prog {
    fn name(self) -> &'static str {
        match self {
            Prog::Get => "snmpget",
            Prog::GetNext => "snmpgetnext",
            Prog::Walk => "snmpwalk",
            Prog::BulkWalk => "snmpbulkwalk",
            Prog::Set => "snmpset",
        }
    }
    fn usage(self) -> String {
        let what = match self {
            Prog::Get | Prog::GetNext => "OID [OID ...]",
            Prog::Walk | Prog::BulkWalk => "[OID]",
            Prog::Set => "OID TYPE VALUE [OID TYPE VALUE ...]",
        };
        let extra = if self == Prog::BulkWalk { " [-Cr N] [-Cn N]" } else { "" };
        format!("Usage: {} [-v 1|2c] -c COMMUNITY [-r RETRIES] [-t SECONDS] [-On|-Oq|-Ov|-Os]{} AGENT {}\n", self.name(), extra, what)
    }
    fn help(self) -> String {
        let what = match self {
            Prog::Get => "asks the agent for the value of each object",
            Prog::GetNext => "asks the agent for the object after each one named",
            Prog::Walk => "goes through every object below OID (mib-2 without one), one GetNext after the other",
            Prog::BulkWalk => "goes through every object below OID (mib-2 without one) with GetBulk, many at a time",
            Prog::Set => "gives objects new values; TYPE is i (integer), u (unsigned), t (time ticks), a (IPv4 address), o (object identifier), s (text), x (hex bytes), d (decimal bytes), n (null) or = (the type the MIB names)",
        };
        let mut s = self.usage();
        s.push_str(&format!("  {}\n", what));
        s.push_str("  AGENT    an address or a name, :PORT after it (161), udp: or udp6: before it\n");
        s.push_str("  -v       the version: 1 or 2c (2c when not given)\n");
        s.push_str("  -c       the community (no default)\n");
        s.push_str("  -r, -t   tries after the first (5) and seconds to wait for each (1)\n");
        s.push_str("  -On      names as numbers; -Oq without = and the type; -Ov values only; -Os without the MIB module\n");
        if self == Prog::BulkWalk {
            s.push_str("  -Cr, -Cn max-repetitions (10) and non-repeaters (0) of each GetBulk\n");
        }
        s
    }
}

/// how bindings are printed (-O)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Fmt {
    numeric: bool,
    quick: bool,
    value: bool,
    short: bool,
}

impl Fmt {
    fn line(&self, vb: &VarBind) -> String {
        let a = vb.oid.arcs();
        let name = if self.numeric {
            mib::name_of(a, true)
        } else if self.short {
            mib::short_name(a)
        } else {
            mib::name_of(a, false)
        };
        let val = if self.quick { mib::bare(a, &vb.value, self.numeric) } else { mib::show(a, &vb.value, self.numeric) };
        if self.value {
            val
        } else if self.quick {
            format!("{} {}", name, val)
        } else {
            format!("{} = {}", name, val)
        }
    }
}

/// a manager on the console: its requests one after the other to one agent
#[derive(Clone, Debug)]
pub struct SnmpTool {
    prog: Prog,
    version: Version,
    community: String,
    /// the agent as typed, its address once known, its port
    host: String,
    agent: Option<IpAddr>,
    port: u16,
    timeout: Ns,
    retries: u8,
    fmt: Fmt,
    oids: Vec<Oid>,
    sets: Vec<VarBind>,
    non_rep: i32,
    max_rep: i32,
    /// a walk: its root and the last name it printed
    root: Oid,
    cur: Option<Oid>,
    got: usize,
    /// a walk that found nothing asks the root with Get
    root_get: bool,
    sock: u16,
    id: i32,
    kind: Kind,
    bytes: Vec<u8>,
    tries: u8,
    lookup: Option<u32>,
}

/// the agent of a command line: (host, port)
fn agent_of(s: &str) -> Option<(String, u16)> {
    let s = s.strip_prefix("udp6:").or_else(|| s.strip_prefix("udp:")).unwrap_or(s);
    if let Some(rest) = s.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse::<u16>().ok().filter(|&p| p > 0)?,
            None if tail.is_empty() => snmp::PORT,
            None => return None,
        };
        return Some((String::from(h), port));
    }
    if s.matches(':').count() > 1 {
        // a bare IPv6 address
        return Some((String::from(s), snmp::PORT));
    }
    match s.split_once(':') {
        Some((h, p)) => Some((String::from(h), p.parse::<u16>().ok().filter(|&p| p > 0)?)),
        None => Some((String::from(s), snmp::PORT)),
    }
}

/// hex bytes ("02 4a", "024A", "02:4a")
fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let d: String = s.chars().filter(|c| !matches!(c, ' ' | ':' | '-')).collect();
    if d.len() % 2 != 0 || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..d.len() / 2).map(|i| u8::from_str_radix(&d[2 * i..2 * i + 2], 16).ok()).collect()
}

/// the value of a type letter of snmpset (the object names the enumerations of "i" and the
/// type of "=")
fn set_value(oid: &Oid, t: &str, v: &str) -> Option<Value> {
    let col = mib::column_of(oid.arcs());
    let t = if t == "=" {
        match col.map(|c| c.syntax) {
            Some(Syntax::Integer) => "i",
            Some(Syntax::Oid) => "o",
            Some(Syntax::IpAddress) => "a",
            Some(Syntax::Gauge32) => "u",
            Some(Syntax::TimeTicks) => "t",
            Some(Syntax::PhysAddress) => "x",
            _ => "s",
        }
    } else {
        t
    };
    Some(match t {
        "i" => match v.parse::<i64>() {
            Ok(n) if (i32::MIN as i64..=i32::MAX as i64).contains(&n) => Value::Int(n),
            Ok(_) => return None,
            Err(_) => Value::Int(col.and_then(|c| c.enums.iter().find(|e| e.1 == v)).map(|e| e.0)?),
        },
        "u" => Value::Gauge32(v.parse::<u32>().ok()?),
        "t" => Value::TimeTicks(v.parse::<u32>().ok()?),
        "a" => Value::Ip(Ipv4::parse(v)?),
        "o" => Value::Oid(mib::lookup(v)?),
        "s" => Value::text(v),
        "x" => Value::Str(hex_bytes(v)?),
        "d" => Value::Str(v.split(|c: char| c == '.' || c == ' ').filter(|x| !x.is_empty()).map(|x| x.parse::<u8>().ok()).collect::<Option<Vec<u8>>>()?),
        "n" => Value::Null,
        _ => return None,
    })
}

impl Engine {
    /// snmpget, snmpgetnext, snmpwalk, snmpbulkwalk, snmpset
    pub(crate) fn sh_snmp(&mut self, ni: usize, prog: Prog, args: &[&str]) -> Result<(), String> {
        let name = prog.name();
        let mut t = SnmpTool {
            prog,
            version: Version::V2c,
            community: String::new(),
            host: String::new(),
            agent: None,
            port: snmp::PORT,
            timeout: SEC,
            retries: 5,
            fmt: Fmt::default(),
            oids: Vec::new(),
            sets: Vec::new(),
            non_rep: 0,
            max_rep: 10,
            root: Oid::new(&[1, 3, 6, 1, 2, 1]),
            cur: None,
            got: 0,
            root_get: false,
            sock: 0,
            id: 0,
            kind: Kind::Get,
            bytes: Vec::new(),
            tries: 0,
            lookup: None,
        };
        let mut community: Option<String> = None;
        let mut pos: Vec<&str> = Vec::new();
        let mut k = 0;
        // an option's value: glued to it ("-v2c") or the next word
        let value = |k: &mut usize, a: &str, flag: &str| -> Option<String> {
            if a.len() > flag.len() {
                return Some(String::from(&a[flag.len()..]));
            }
            *k += 1;
            args.get(*k).map(|x| String::from(*x))
        };
        while k < args.len() {
            let a = args[k];
            if a == "-h" || a == "--help" {
                self.out(ni, &prog.help());
                return Ok(());
            }
            if a == "--" {
                pos.extend(args[k + 1..].iter().copied());
                break;
            }
            if a.starts_with('-') && a.len() > 1 && !(prog == Prog::Set && pos.len() >= 2) {
                let flag = &a[..2];
                match flag {
                    "-v" => {
                        let v = value(&mut k, a, flag).unwrap_or_default();
                        t.version = match Version::parse(&v) {
                            Some(x) => x,
                            None => return self.sh_err(ni, &format!("{}: version '{}' is none of 1 and 2c (SNMPv3 is not here)\n", name, v)),
                        };
                    }
                    "-c" => community = value(&mut k, a, flag),
                    "-r" => match value(&mut k, a, flag).and_then(|v| v.parse::<u8>().ok()) {
                        Some(r) => t.retries = r.min(20),
                        None => return self.sh_err(ni, &format!("{}: -r wants a number of tries\n", name)),
                    },
                    "-t" => match value(&mut k, a, flag).and_then(|v| super::linux::secs(&v)) {
                        Some(s) if s >= MS => t.timeout = s,
                        _ => return self.sh_err(ni, &format!("{}: -t wants seconds (at least 0.001)\n", name)),
                    },
                    "-O" => {
                        let f = value(&mut k, a, flag).unwrap_or_default();
                        for c in f.chars() {
                            match c {
                                'n' => t.fmt.numeric = true,
                                'q' => t.fmt.quick = true,
                                'v' => t.fmt.value = true,
                                's' => t.fmt.short = true,
                                _ => return self.sh_err(ni, &format!("{}: -O{} is no output form here (n, q, v, s)\n", name, c)),
                            }
                        }
                    }
                    "-C" if prog == Prog::BulkWalk => {
                        let f = value(&mut k, a, flag).unwrap_or_default();
                        let (c, rest) = f.split_at(f.len().min(1));
                        let n = if rest.is_empty() {
                            k += 1;
                            args.get(k).and_then(|x| x.parse::<i32>().ok())
                        } else {
                            rest.parse::<i32>().ok()
                        };
                        match (c, n) {
                            ("r", Some(n)) if n > 0 => t.max_rep = n.min(1000),
                            ("n", Some(n)) if n >= 0 => t.non_rep = n.min(1000),
                            _ => return self.sh_err(ni, &format!("{}: -C{} is no setting here (-Cr N, -Cn N)\n", name, f)),
                        }
                    }
                    _ => return self.sh_err(ni, &format!("{}: unknown option {}\n{}", name, a, prog.usage())),
                }
                k += 1;
                continue;
            }
            pos.push(a);
            k += 1;
        }
        let Some(c) = community.filter(|c| !c.is_empty()) else {
            return self.sh_err(ni, &format!("{}: no community given (-c NAME)\n{}", name, prog.usage()));
        };
        t.community = c;
        let Some(&agent) = pos.first() else {
            return self.sh_err(ni, &format!("{}: no agent given\n{}", name, prog.usage()));
        };
        let Some((host, port)) = agent_of(agent) else {
            return self.sh_err(ni, &format!("{}: '{}' is no agent (ADDRESS[:PORT])\n", name, agent));
        };
        t.host = host.clone();
        t.port = port;
        t.agent = IpAddr::parse(&host);
        let rest = &pos[1..];
        let oid = |s: &str| mib::lookup(s);
        match prog {
            Prog::Set => {
                if rest.is_empty() || rest.len() % 3 != 0 {
                    return self.sh_err(ni, &format!("{}: each object needs a type and a value\n{}", name, prog.usage()));
                }
                for tr in rest.chunks(3) {
                    let Some(o) = oid(tr[0]) else {
                        return self.sh_err(ni, &format!("{}: {}: no object of that name here (dotted numbers always work)\n", name, tr[0]));
                    };
                    if !["i", "u", "t", "a", "o", "s", "x", "d", "n", "="].contains(&tr[1]) {
                        return self.sh_err(ni, &format!("{}: '{}' is no type (i u t a o s x d n =)\n", name, tr[1]));
                    }
                    let Some(v) = set_value(&o, tr[1], tr[2]) else {
                        return self.sh_err(ni, &format!("{}: '{}' is no value of type {}\n", name, tr[2], tr[1]));
                    };
                    t.sets.push(VarBind::new(o, v));
                }
            }
            Prog::Walk | Prog::BulkWalk => {
                if let Some(r) = rest.first() {
                    match oid(r) {
                        Some(o) => t.root = o,
                        None => return self.sh_err(ni, &format!("{}: {}: no object of that name here (dotted numbers always work)\n", name, r)),
                    }
                }
                if prog == Prog::BulkWalk && t.version == Version::V1 {
                    return self.sh_err(ni, &format!("{}: GetBulk needs version 2c\n", name));
                }
            }
            _ => {
                if rest.is_empty() {
                    return self.sh_err(ni, &format!("{}: no object given\n{}", name, prog.usage()));
                }
                for r in rest {
                    match oid(r) {
                        Some(o) => t.oids.push(o),
                        None => return self.sh_err(ni, &format!("{}: {}: no object of that name here (dotted numbers always work)\n", name, r)),
                    }
                }
            }
        }
        self.run_program(ni, name, Box::new(t), false)
    }
}

impl SnmpTool {
    fn name(&self) -> &'static str {
        self.prog.name()
    }

    fn end(&mut self, os: &mut OsApi, code: i32) {
        os.cancel(K_WAIT);
        os.exit(code);
    }

    /// a socket of the agent's family and the first request
    fn begin(&mut self, os: &mut OsApi) {
        let Some(agent) = self.agent else { return };
        let bind = if agent.is_v4() { ANY4 } else { ANY };
        match os.bind_on(bind, 0, None) {
            Ok(p) => self.sock = p,
            Err(_) => {
                os.print(&format!("{}: no free port\n", self.name()));
                self.end(os, 1);
                return;
            }
        }
        match self.prog {
            Prog::Get => {
                let vbs = self.oids.iter().map(|o| VarBind::null(o.clone())).collect();
                self.ask(os, Kind::Get, vbs);
            }
            Prog::GetNext => {
                let vbs = self.oids.iter().map(|o| VarBind::null(o.clone())).collect();
                self.ask(os, Kind::GetNext, vbs);
            }
            Prog::Set => {
                let vbs = self.sets.clone();
                self.ask(os, Kind::Set, vbs);
            }
            Prog::Walk | Prog::BulkWalk => self.walk_on(os),
        }
    }

    /// the next request of a walk
    fn walk_on(&mut self, os: &mut OsApi) {
        let from = self.cur.clone().unwrap_or_else(|| self.root.clone());
        if self.prog == Prog::BulkWalk {
            self.ask(os, Kind::GetBulk, alloc::vec![VarBind::null(from)]);
        } else {
            self.ask(os, Kind::GetNext, alloc::vec![VarBind::null(from)]);
        }
    }

    /// a request with a new id
    fn ask(&mut self, os: &mut OsApi, kind: Kind, vbs: Vec<VarBind>) {
        self.id = (os.rand() & 0x7FFF_FFFF) as i32;
        self.kind = kind;
        let pdu = if kind == Kind::GetBulk { Pdu::bulk(self.id, self.non_rep, self.max_rep, vbs) } else { Pdu::new(kind, self.id, vbs) };
        self.bytes = Message::new(self.version, &self.community, pdu).to_bytes();
        self.tries = 0;
        self.send(os);
    }

    /// sends the request (again)
    fn send(&mut self, os: &mut OsApi) {
        let Some(agent) = self.agent else { return };
        self.tries += 1;
        if self.tries == 1 {
            os.note(Note::Snmp(SNote::Ask { to: agent, kind: self.kind, id: self.id }));
        } else {
            os.note(Note::Snmp(SNote::AskAgain { to: agent, n: self.tries }));
        }
        let bytes = self.bytes.clone();
        if !os.send(self.sock, agent, self.port, &bytes) {
            os.print(&format!("{}: no route to {}\n", self.name(), self.host));
            self.end(os, 1);
            return;
        }
        let at = os.now() + self.timeout;
        os.timer(K_WAIT, at);
    }

    /// no answer in time
    fn timed_out(&mut self, os: &mut OsApi) {
        if self.tries <= self.retries {
            self.send(os);
            return;
        }
        if let Some(a) = self.agent {
            os.note(Note::Snmp(SNote::NoAnswer { to: a }));
        }
        let t = secs_text(self.timeout);
        let how = if self.tries == 1 { format!("1 try of {} s", t) } else { format!("{} tries of {} s each", self.tries, t) };
        os.print(&format!("{}: {} did not answer ({})\n", self.name(), self.host, how));
        self.end(os, 1);
    }

    /// the answer to the request that runs
    fn answer(&mut self, os: &mut OsApi, p: Pdu) {
        os.cancel(K_WAIT);
        let fmt = self.fmt;
        if p.error_status != 0 {
            // version 1's end of the MIB ends a walk as well
            if matches!(self.prog, Prog::Walk | Prog::BulkWalk) && p.error_status == snmp::E_NO_SUCH_NAME && self.version == Version::V1 {
                return self.walk_done(os);
            }
            let at = (p.error_index.max(1) as usize).saturating_sub(1);
            let failed = p.varbinds.get(at).map(|v| mib::name_of(v.oid.arcs(), fmt.numeric)).unwrap_or_default();
            let text = if p.error_index > 0 && !failed.is_empty() { format!("{}: the agent answered {} at binding {} ({})\n", self.name(), snmp::error_name(p.error_status), p.error_index, failed) } else { format!("{}: the agent answered {}\n", self.name(), snmp::error_name(p.error_status)) };
            os.print(&text);
            return self.end(os, 1);
        }
        match self.prog {
            Prog::Get | Prog::GetNext | Prog::Set => {
                let mut s = String::new();
                for vb in &p.varbinds {
                    s.push_str(&fmt.line(vb));
                    s.push('\n');
                }
                os.print(&s);
                self.end(os, 0);
            }
            Prog::Walk | Prog::BulkWalk if self.root_get => {
                let mut s = String::new();
                for vb in &p.varbinds {
                    s.push_str(&fmt.line(vb));
                    s.push('\n');
                }
                os.print(&s);
                self.end(os, 0);
            }
            Prog::Walk | Prog::BulkWalk => {
                let mut s = String::new();
                let mut done = p.varbinds.is_empty();
                for vb in &p.varbinds {
                    if vb.value == Value::EndOfMibView || !vb.oid.starts_with(self.root.arcs()) {
                        done = true;
                        break;
                    }
                    if self.cur.as_ref().is_some_and(|c| vb.oid <= *c) {
                        os.print(&s);
                        os.print(&format!("{}: the agent answered {}, which does not come after {}: the walk ends\n", self.name(), mib::name_of(vb.oid.arcs(), fmt.numeric), self.cur.as_ref().map(|c| mib::name_of(c.arcs(), fmt.numeric)).unwrap_or_default()));
                        return self.end(os, 1);
                    }
                    s.push_str(&fmt.line(vb));
                    s.push('\n');
                    self.got += 1;
                    self.cur = Some(vb.oid.clone());
                }
                os.print(&s);
                if done {
                    return self.walk_done(os);
                }
                self.walk_on(os);
            }
        }
    }

    /// the walk left its root: a root with nothing below is asked with Get, as net-snmp does
    fn walk_done(&mut self, os: &mut OsApi) {
        if self.got == 0 && !self.root_get {
            self.root_get = true;
            let root = self.root.clone();
            self.ask(os, Kind::Get, alloc::vec![VarBind::null(root)]);
            return;
        }
        self.end(os, 0);
    }
}

impl Service for SnmpTool {
    fn kind(&self) -> &'static str {
        self.prog.name()
    }
    fn start(&mut self, os: &mut OsApi) {
        if self.agent.is_some() {
            self.begin(os);
            return;
        }
        let host = self.host.clone();
        self.lookup = Some(os.resolve_host(&host, Family::Any));
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(l) if Some(l) == self.lookup => {
                self.lookup = None;
                let all = os.lookup(l).and_then(|x| x.result.clone()).and_then(|r| r.ok()).map(|r| r.sorted).unwrap_or_default();
                match all.first() {
                    Some(a) => {
                        self.agent = Some(*a);
                        self.begin(os);
                    }
                    None => {
                        os.print(&format!("{}: no address for '{}'\n", self.name(), self.host));
                        self.end(os, 1);
                    }
                }
            }
            SvcEvent::Udp(d) if d.dport == self.sock && Some(d.src) == self.agent && d.sport == self.port => {
                let Ok(m) = Message::parse(&d.data) else { return };
                let Some(p) = m.pdu() else { return };
                if p.kind == Kind::Response && p.request_id == self.id && m.community == self.community.as_bytes() {
                    let p = p.clone();
                    self.answer(os, p);
                }
            }
            SvcEvent::Timer(K_WAIT) => self.timed_out(os),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::from(self.prog.name())
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// seconds as -t takes them ("1", "0.25")
fn secs_text(ns: Ns) -> String {
    let ms = ns / MS;
    if ms % 1000 == 0 {
        return format!("{}", ms / 1000);
    }
    let f = format!("{:03}", ms % 1000);
    format!("{}.{}", ms / 1000, f.trim_end_matches('0'))
}
