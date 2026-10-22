// acl -- IPv4 access lists of IOS-like devices: standard lists (the source address) and
// extended ones (protocol, addresses, ports, ICMP types, "established"), numbered or named,
// with sequence numbers, remarks and hit counters, bound to interfaces in either direction
//
//   Acl { kind, numbered, lines }    a list; Line { seq, item: Rule(Ace) | Remark(text), hits }
//   Ace { permit, proto, src, sport, dst, dport, established, icmp, log }   one rule; Wild is an
//                                    address with a wildcard mask (any, host, or both), Ports an
//                                    operator (eq, neq, gt, lt, range)
//   Acl::check(&Flow) -> Verdict     the first rule that matches decides; none: the implicit
//                                    deny at the end (seq 0)
//   Acls { lists, bound }            the lists of a device by name ("10", "101", "WEB") and
//                                    which list an interface uses in which direction
//   AclOp                            what Action::Acl does: create, add (at a sequence number
//                                    or after the last), remove a line by number or content,
//                                    remove the list, resequence
//   Ace::config(kind) / Ace::show(kind)   the rule as a configuration line and as "show
//                                    access-lists" lists it (port and ICMP names as IOS writes
//                                    them: "eq www", "echo-reply")
//   Engine::acl_pass(..)             an interface's list decides about a packet (l3.rs); a
//                                    denied one is dropped and its sender answered
//   Engine::acl_match(ni, list, &Flow)   a list as the selector of NAT (nat.rs): permit, deny
//                                    or no such list
//   Engine::access_lists(dev) / access_groups(dev)   views for the user interface and the
//                                    show commands
//
// Where the lists act (with NAT: nat.rs has the whole order): a packet that arrives on an
// interface with an inbound list meets it first, before the device decides whether the
// packet is its own or to be forwarded, so a list also guards the device itself; a packet the
// device forwards meets the outbound list of the interface it leaves through, after routing
// and after NAT.  Packets the device makes itself pass every outbound list, as on the
// original.  A denied packet is dropped with a note naming the list and the line (seq 0: the
// implicit deny), and its sender gets "administratively prohibited" (ICMP type 3, code 13)
// from the interface the packet came in on, under the limit of the other unreachables (one
// every 500 ms).  A list without rules, or one that does not exist, lets everything through.
//
// Fragments: only the first fragment of a datagram carries the ports.  A later fragment meets
// a rule with ports, flags or an ICMP type like this: when protocol and addresses match, a
// permit rule lets it through and a deny rule is skipped (the next line decides), so the
// fragments of a datagram whose first part a list let through are not stopped.  Hit counters
// count the packets a line decided; "clear access-list counters" and switching the device off
// reset them.  A rule with "log" writes a console message the first time it matches and then
// at most every five minutes with the number of packets since (cli/acl.rs).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::icmp::{Icmp, UNREACH_ADMIN};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP};
use netpkt::nat::Flow;
use netpkt::tcp::{ACK, RST};
use netpkt::{wildcard_match, Ipv4};

use crate::kernel::{Engine, Ns};
use crate::trace::{HopId, Note};

/// a standard list looks at the source address only, an extended one at everything
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AclKind {
    Standard,
    Extended,
}

impl AclKind {
    /// the kind a list number belongs to (1-99, 1300-1999 standard; 100-199, 2000-2699
    /// extended); None: no IP access list number
    pub fn of_number(n: u32) -> Option<AclKind> {
        match n {
            1..=99 | 1300..=1999 => Some(AclKind::Standard),
            100..=199 | 2000..=2699 => Some(AclKind::Extended),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            AclKind::Standard => "standard",
            AclKind::Extended => "extended",
        }
    }
}

/// an address with a wildcard mask: the bits set in `wild` may be anything
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wild {
    pub addr: Ipv4,
    pub wild: Ipv4,
}

impl Wild {
    pub const ANY: Wild = Wild { addr: Ipv4::UNSPECIFIED, wild: Ipv4::BROADCAST };
    /// one address
    pub fn host(a: Ipv4) -> Wild {
        Wild { addr: a, wild: Ipv4::UNSPECIFIED }
    }
    /// an address and a wildcard mask; the address loses the bits the mask frees (as IOS
    /// stores it: "10.1.2.3 0.0.0.255" is 10.1.2.0 0.0.0.255)
    pub fn new(a: Ipv4, wild: Ipv4) -> Wild {
        Wild { addr: Ipv4::from_u32(a.to_u32() & !wild.to_u32()), wild }
    }
    pub fn is_any(&self) -> bool {
        self.wild == Ipv4::BROADCAST
    }
    pub fn is_host(&self) -> bool {
        self.wild == Ipv4::UNSPECIFIED
    }
    pub fn matches(&self, ip: Ipv4) -> bool {
        wildcard_match(ip, self.addr, self.wild)
    }
    /// "any", "host 10.0.0.1", "10.0.0.0 0.0.0.255" (extended lists and configurations)
    pub fn text(&self) -> String {
        if self.is_any() {
            String::from("any")
        } else if self.is_host() {
            format!("host {}", self.addr)
        } else {
            format!("{} {}", self.addr, self.wild)
        }
    }
}

/// a port condition of an extended rule
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ports {
    Any,
    Eq(u16),
    Neq(u16),
    Gt(u16),
    Lt(u16),
    /// both ends included
    Range(u16, u16),
}

impl Ports {
    pub fn matches(&self, p: u16) -> bool {
        match *self {
            Ports::Any => true,
            Ports::Eq(x) => p == x,
            Ports::Neq(x) => p != x,
            Ports::Gt(x) => p > x,
            Ports::Lt(x) => p < x,
            Ports::Range(a, b) => a <= p && p <= b,
        }
    }
    /// " eq www", " range 1024 65535", "" (with the protocol's port names)
    pub fn text(&self, proto: u8) -> String {
        let n = |p: u16| port_text(proto, p);
        match *self {
            Ports::Any => String::new(),
            Ports::Eq(x) => format!(" eq {}", n(x)),
            Ports::Neq(x) => format!(" neq {}", n(x)),
            Ports::Gt(x) => format!(" gt {}", n(x)),
            Ports::Lt(x) => format!(" lt {}", n(x)),
            Ports::Range(a, b) => format!(" range {} {}", n(a), n(b)),
        }
    }
}

/// the TCP port names an IOS-like list knows, with the help text of the console
pub const TCP_PORTS: &[(&str, u16, &str)] = &[
    ("bgp", 179, "BGP routing (179)"),
    ("chargen", 19, "Character generator (19)"),
    ("cmd", 514, "Remote commands, rcmd (514)"),
    ("daytime", 13, "Time of day as text (13)"),
    ("discard", 9, "Discard service (9)"),
    ("domain", 53, "DNS (53)"),
    ("echo", 7, "Echo service (7)"),
    ("exec", 512, "Remote execution, exec (512)"),
    ("finger", 79, "Finger (79)"),
    ("ftp", 21, "FTP control (21)"),
    ("ftp-data", 20, "FTP data (20)"),
    ("gopher", 70, "Gopher (70)"),
    ("hostname", 101, "Host name server (101)"),
    ("ident", 113, "Ident protocol (113)"),
    ("irc", 194, "Internet Relay Chat (194)"),
    ("klogin", 543, "Kerberos login (543)"),
    ("kshell", 544, "Kerberos shell (544)"),
    ("login", 513, "Remote login, rlogin (513)"),
    ("lpd", 515, "Printer daemon (515)"),
    ("nntp", 119, "Network news (119)"),
    ("pop2", 109, "Mail retrieval, POP2 (109)"),
    ("pop3", 110, "Mail retrieval, POP3 (110)"),
    ("smtp", 25, "Mail transport, SMTP (25)"),
    ("sunrpc", 111, "Sun remote procedure calls (111)"),
    ("tacacs", 49, "TACACS logins (49)"),
    ("talk", 517, "Talk (517)"),
    ("telnet", 23, "Telnet (23)"),
    ("time", 37, "Time protocol (37)"),
    ("uucp", 540, "Unix-to-Unix copy (540)"),
    ("whois", 43, "Whois directory (43)"),
    ("www", 80, "Web, HTTP (80)"),
];

/// the UDP port names, with their help texts
pub const UDP_PORTS: &[(&str, u16, &str)] = &[
    ("biff", 512, "Mail notification, biff (512)"),
    ("bootpc", 68, "DHCP and BOOTP client (68)"),
    ("bootps", 67, "DHCP and BOOTP server (67)"),
    ("discard", 9, "Discard service (9)"),
    ("dnsix", 195, "DNSIX security log (195)"),
    ("domain", 53, "DNS (53)"),
    ("echo", 7, "Echo service (7)"),
    ("isakmp", 500, "IPsec key exchange (500)"),
    ("mobile-ip", 434, "Mobile IP registration (434)"),
    ("nameserver", 42, "Old host name server (42)"),
    ("netbios-dgm", 138, "NetBIOS datagrams (138)"),
    ("netbios-ns", 137, "NetBIOS names (137)"),
    ("netbios-ss", 139, "NetBIOS sessions (139)"),
    ("non500-isakmp", 4500, "IPsec key exchange behind NAT (4500)"),
    ("ntp", 123, "Network time, NTP (123)"),
    ("rip", 520, "RIP routing (520)"),
    ("snmp", 161, "SNMP (161)"),
    ("snmptrap", 162, "SNMP traps (162)"),
    ("sunrpc", 111, "Sun remote procedure calls (111)"),
    ("syslog", 514, "Syslog (514)"),
    ("tacacs", 49, "TACACS logins (49)"),
    ("talk", 517, "Talk (517)"),
    ("tftp", 69, "Trivial file transfer, TFTP (69)"),
    ("time", 37, "Time protocol (37)"),
    ("who", 513, "Who is logged in, rwho (513)"),
    ("xdmcp", 177, "X display manager (177)"),
];

/// the ICMP message names of extended rules: (name, type, code), with their help texts
pub const ICMP_TYPES: &[(&str, u8, Option<u8>, &str)] = &[
    ("administratively-prohibited", 3, Some(13), "Forbidden by a filter"),
    ("echo", 8, None, "Echo request (ping)"),
    ("echo-reply", 0, None, "Echo reply"),
    ("host-unreachable", 3, Some(1), "The host cannot be reached"),
    ("information-reply", 16, None, "Information reply"),
    ("information-request", 15, None, "Information request"),
    ("mask-reply", 18, None, "Address mask reply"),
    ("mask-request", 17, None, "Address mask request"),
    ("net-unreachable", 3, Some(0), "The network cannot be reached"),
    ("packet-too-big", 3, Some(4), "Fragmentation needed but forbidden"),
    ("parameter-problem", 12, None, "Bad header field"),
    ("port-unreachable", 3, Some(3), "Nobody listens on the port"),
    ("protocol-unreachable", 3, Some(2), "The protocol is not supported"),
    ("reassembly-timeout", 11, Some(1), "Fragments did not come complete in time"),
    ("redirect", 5, None, "A better router for the destination"),
    ("router-advertisement", 9, None, "A router announces itself"),
    ("router-solicitation", 10, None, "A host looks for routers"),
    ("source-quench", 4, None, "Slow down (obsolete)"),
    ("time-exceeded", 11, None, "Any time exceeded message"),
    ("timestamp-reply", 14, None, "Timestamp reply"),
    ("timestamp-request", 13, None, "Timestamp request"),
    ("traceroute", 30, None, "Traceroute message"),
    ("ttl-exceeded", 11, Some(0), "The TTL ran out on the way"),
    ("unreachable", 3, None, "Any destination unreachable message"),
];

/// the IP protocols an extended rule names (besides "ip", every protocol, and a number)
pub const PROTOCOLS: &[(&str, u8, &str)] = &[
    ("ahp", 51, "IPsec authentication header"),
    ("eigrp", 88, "EIGRP routing"),
    ("esp", 50, "IPsec encrypted payload"),
    ("gre", 47, "GRE tunnels"),
    ("icmp", 1, "ICMP messages"),
    ("igmp", 2, "Multicast group membership, IGMP"),
    ("ipinip", 4, "IP in IP tunnels"),
    ("ospf", 89, "OSPF routing"),
    ("pim", 103, "Multicast routing, PIM"),
    ("tcp", 6, "TCP connections"),
    ("udp", 17, "UDP datagrams"),
];

/// a port as IOS writes it in a rule: its name when it has one ("www"), else the number
pub fn port_text(proto: u8, p: u16) -> String {
    let names = if proto == PROTO_TCP { TCP_PORTS } else { UDP_PORTS };
    match names.iter().find(|n| n.1 == p) {
        Some(n) => String::from(n.0),
        None => format!("{}", p),
    }
}

/// a port name of a protocol ("www" for TCP) as its number
pub fn port_number(proto: u8, name: &str) -> Option<u16> {
    let names = if proto == PROTO_TCP { TCP_PORTS } else { UDP_PORTS };
    names.iter().find(|n| n.0.eq_ignore_ascii_case(name)).map(|n| n.1)
}

/// a protocol as a rule writes it ("ip" for every protocol, "tcp", "gre", "112")
pub fn proto_text(p: Option<u8>) -> String {
    match p {
        None => String::from("ip"),
        Some(n) => match PROTOCOLS.iter().find(|x| x.1 == n) {
            Some(x) => String::from(x.0),
            None => format!("{}", n),
        },
    }
}

/// an ICMP condition as a rule writes it ("echo", "administratively-prohibited", "3 9")
pub fn icmp_text(t: u8, c: Option<u8>) -> String {
    if let Some(x) = ICMP_TYPES.iter().find(|x| x.1 == t && x.2 == c) {
        return String::from(x.0);
    }
    match c {
        Some(c) => format!("{} {}", t, c),
        None => format!("{}", t),
    }
}

/// one rule of a list
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    pub permit: bool,
    /// None: every protocol ("ip"; standard rules)
    pub proto: Option<u8>,
    pub src: Wild,
    pub sport: Ports,
    pub dst: Wild,
    pub dport: Ports,
    /// TCP segments with ACK or RST: no new connection
    pub established: bool,
    /// ICMP type and (optionally) code
    pub icmp: Option<(u8, Option<u8>)>,
    /// a console message when the rule matches
    pub log: bool,
}

impl Ace {
    /// a standard rule: the source alone
    pub fn standard(permit: bool, src: Wild) -> Ace {
        Ace { permit, proto: None, src, sport: Ports::Any, dst: Wild::ANY, dport: Ports::Any, established: false, icmp: None, log: false }
    }
    /// it looks at more than addresses and protocol
    fn layer4(&self) -> bool {
        self.sport != Ports::Any || self.dport != Ports::Any || self.established || self.icmp.is_some()
    }
    /// Some(permit) when the rule decides about the flow
    pub fn matches(&self, f: &Flow) -> Option<bool> {
        if self.proto.is_some_and(|p| p != f.proto) || !self.src.matches(f.src) || !self.dst.matches(f.dst) {
            return None;
        }
        if !self.layer4() {
            return Some(self.permit);
        }
        let header = if f.proto == PROTO_ICMP { f.icmp.is_some() } else { f.ports };
        if !header {
            // a later fragment: what a permit rule let through with the first goes on
            return if self.permit { Some(true) } else { None };
        }
        if !self.sport.matches(f.sport) || !self.dport.matches(f.dport) {
            return None;
        }
        if self.established && !(f.proto == PROTO_TCP && f.flags & (ACK | RST) != 0) {
            return None;
        }
        if let Some((t, c)) = self.icmp {
            match f.icmp {
                Some((ft, fc)) if ft == t && c.is_none_or(|c| c == fc) => {}
                _ => return None,
            }
        }
        Some(self.permit)
    }
    /// the rule as a configuration line after "access-list N " or in a list's own mode
    /// ("permit tcp any host 10.0.0.5 eq www"; standard: "deny 10.0.0.0 0.0.0.255")
    pub fn config(&self, kind: AclKind) -> String {
        let act = if self.permit { "permit" } else { "deny" };
        let mut s = match kind {
            AclKind::Standard => {
                let src = if self.src.is_any() {
                    String::from("any")
                } else if self.src.is_host() {
                    format!("{}", self.src.addr)
                } else {
                    format!("{} {}", self.src.addr, self.src.wild)
                };
                format!("{} {}", act, src)
            }
            AclKind::Extended => {
                let p = self.proto.unwrap_or(0);
                let mut s = format!("{} {} {}{} {}{}", act, proto_text(self.proto), self.src.text(), self.sport.text(p), self.dst.text(), self.dport.text(p));
                if let Some((t, c)) = self.icmp {
                    s.push(' ');
                    s.push_str(&icmp_text(t, c));
                }
                if self.established {
                    s.push_str(" established");
                }
                s
            }
        };
        if self.log {
            s.push_str(" log");
        }
        s
    }
    /// the rule as "show access-lists" writes it (standard: "permit 10.0.0.0, wildcard bits
    /// 0.0.0.255")
    pub fn show(&self, kind: AclKind) -> String {
        if kind == AclKind::Extended || self.src.is_any() || self.src.is_host() {
            return self.config(kind);
        }
        let act = if self.permit { "permit" } else { "deny" };
        format!("{} {}, wildcard bits {}{}", act, self.src.addr, self.src.wild, if self.log { " log" } else { "" })
    }
}

/// a line of a list
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Rule(Ace),
    Remark(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// the sequence number of a rule (0 for a remark)
    pub seq: u32,
    pub item: Item,
    /// packets the rule decided
    pub hits: u64,
    /// "log": when the last message went, and the packets since
    pub(crate) logged: Option<Ns>,
    pub(crate) unlogged: u64,
}

/// how a list decided
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub permit: bool,
    /// the deciding line's sequence number; 0: the implicit deny at the end
    pub seq: u32,
}

/// an access list
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acl {
    pub kind: AclKind,
    /// made as "access-list N" or named with its number: written as access-list lines
    pub numbered: bool,
    pub lines: Vec<Line>,
}

/// the largest sequence number
pub const SEQ_MAX: u32 = 2_147_483_647;

impl Acl {
    pub fn new(kind: AclKind, numbered: bool) -> Acl {
        Acl { kind, numbered, lines: Vec::new() }
    }
    /// it has a rule (a list of remarks alone lets everything through)
    pub fn has_rules(&self) -> bool {
        self.lines.iter().any(|l| matches!(l.item, Item::Rule(_)))
    }
    fn last_seq(&self) -> u32 {
        self.lines.iter().map(|l| l.seq).max().unwrap_or(0)
    }
    /// the first rule that matches decides; the line index too (None: the implicit deny)
    pub fn check(&self, f: &Flow) -> (Verdict, Option<usize>) {
        for (i, l) in self.lines.iter().enumerate() {
            if let Item::Rule(r) = &l.item
                && let Some(p) = r.matches(f)
            {
                return (Verdict { permit: p, seq: l.seq }, Some(i));
            }
        }
        (Verdict { permit: false, seq: 0 }, None)
    }
    /// adds a rule (at `seq`, or 10 after the last) or a remark (at the end); Err with the
    /// reason; an equal rule already there is not added again
    pub fn add(&mut self, seq: Option<u32>, item: Item) -> Result<(), AclErr> {
        match item {
            Item::Remark(t) => {
                self.lines.push(Line { seq: 0, item: Item::Remark(t), hits: 0, logged: None, unlogged: 0 });
                Ok(())
            }
            Item::Rule(r) => {
                if seq.is_none() && self.lines.iter().any(|l| l.item == Item::Rule(r.clone())) {
                    return Ok(());
                }
                let s = match seq {
                    Some(s) => {
                        if self.lines.iter().any(|l| l.seq == s && matches!(l.item, Item::Rule(_))) {
                            return Err(AclErr::DuplicateSeq);
                        }
                        s
                    }
                    None => {
                        let s = self.last_seq() as u64 + 10;
                        if s > SEQ_MAX as u64 {
                            return Err(AclErr::SeqExhausted);
                        }
                        s as u32
                    }
                };
                // before the first rule with a higher number and the remarks just above it
                let mut at = self.lines.iter().position(|l| matches!(l.item, Item::Rule(_)) && l.seq > s).unwrap_or(self.lines.len());
                if at < self.lines.len() {
                    while at > 0 && matches!(self.lines[at - 1].item, Item::Remark(_)) {
                        at -= 1;
                    }
                }
                self.lines.insert(at, Line { seq: s, item: Item::Rule(r), hits: 0, logged: None, unlogged: 0 });
                Ok(())
            }
        }
    }
    /// removes the rule with this number
    pub fn remove_seq(&mut self, seq: u32) -> bool {
        let n = self.lines.len();
        self.lines.retain(|l| !(l.seq == seq && matches!(l.item, Item::Rule(_))));
        self.lines.len() != n
    }
    /// removes the first line equal to `item`
    pub fn remove_item(&mut self, item: &Item) -> bool {
        match self.lines.iter().position(|l| l.item == *item) {
            Some(i) => {
                self.lines.remove(i);
                true
            }
            None => false,
        }
    }
    /// numbers the rules start, start + step, ...
    pub fn resequence(&mut self, start: u32, step: u32) -> bool {
        let n = self.lines.iter().filter(|l| matches!(l.item, Item::Rule(_))).count() as u64;
        if start == 0 || step == 0 || start as u64 + step as u64 * n.saturating_sub(1) > SEQ_MAX as u64 {
            return false;
        }
        let mut s = start;
        for l in self.lines.iter_mut().filter(|l| matches!(l.item, Item::Rule(_))) {
            l.seq = s;
            s = s.saturating_add(step);
        }
        true
    }
}

/// why a change of a list was refused
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AclErr {
    DuplicateSeq,
    SeqExhausted,
    /// the list exists with the other kind
    WrongKind(AclKind),
    NoList,
    NoLine,
    BadSequence,
}

impl AclErr {
    /// the refusal as a message for the app's catalogue
    pub fn msg(&self, list: &str) -> Msg {
        match self {
            AclErr::DuplicateSeq => msg!("Zugriffsliste {}: diese Zeilennummer gibt es schon.", list),
            AclErr::SeqExhausted => msg!("Zugriffsliste {}: keine Zeilennummer mehr frei.", list),
            AclErr::WrongKind(AclKind::Standard) => msg!("Zugriffsliste {} ist eine Standardliste.", list),
            AclErr::WrongKind(AclKind::Extended) => msg!("Zugriffsliste {} ist eine erweiterte Liste.", list),
            AclErr::NoList => msg!("Die Zugriffsliste {} gibt es nicht.", list),
            AclErr::NoLine => msg!("Zugriffsliste {}: keine solche Zeile.", list),
            AclErr::BadSequence => msg!("Zugriffsliste {}: so lassen sich die Zeilen nicht nummerieren.", list),
        }
    }
}

/// what Action::Acl does to a list
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AclOp {
    /// the list exists afterwards (empty when new)
    Create { kind: AclKind, numbered: bool },
    /// a rule at `seq` (None: 10 after the last) or a remark at the end
    Add { kind: AclKind, numbered: bool, seq: Option<u32>, item: Item },
    RemoveSeq(u32),
    RemoveItem(Item),
    /// the whole list
    Remove,
    Resequence { start: u32, step: u32 },
}

/// the access lists of a device and where they are used
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Acls {
    pub lists: BTreeMap<String, Acl>,
    /// (interface, outbound) -> list name ("ip access-group L in|out")
    pub bound: BTreeMap<(u16, bool), String>,
}

/// the order lists are shown and written in: numbered ones by number, then the named ones
pub fn list_order(a: &str, b: &str) -> core::cmp::Ordering {
    let num = |s: &str| s.parse::<u32>().ok();
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => core::cmp::Ordering::Less,
        (None, Some(_)) => core::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

impl Acls {
    /// changes a list
    pub fn apply(&mut self, name: &str, op: AclOp) -> Result<(), AclErr> {
        match op {
            AclOp::Create { kind, numbered } => {
                let l = self.lists.entry(String::from(name)).or_insert_with(|| Acl::new(kind, numbered));
                if l.kind != kind { Err(AclErr::WrongKind(l.kind)) } else { Ok(()) }
            }
            AclOp::Add { kind, numbered, seq, item } => {
                let l = self.lists.entry(String::from(name)).or_insert_with(|| Acl::new(kind, numbered));
                if l.kind != kind {
                    return Err(AclErr::WrongKind(l.kind));
                }
                l.add(seq, item)
            }
            AclOp::RemoveSeq(s) => {
                let l = self.lists.get_mut(name).ok_or(AclErr::NoList)?;
                if l.remove_seq(s) { Ok(()) } else { Err(AclErr::NoLine) }
            }
            AclOp::RemoveItem(it) => {
                let l = self.lists.get_mut(name).ok_or(AclErr::NoList)?;
                if l.remove_item(&it) { Ok(()) } else { Err(AclErr::NoLine) }
            }
            AclOp::Remove => {
                if self.lists.remove(name).is_some() { Ok(()) } else { Err(AclErr::NoList) }
            }
            AclOp::Resequence { start, step } => {
                let l = self.lists.get_mut(name).ok_or(AclErr::NoList)?;
                if l.resequence(start, step) { Ok(()) } else { Err(AclErr::BadSequence) }
            }
        }
    }
    /// the lists' names in their order
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.lists.keys().cloned().collect();
        v.sort_by(|a, b| list_order(a, b));
        v
    }
    /// every hit counter (of one list) back to 0
    pub fn clear_counters(&mut self, name: Option<&str>) {
        for (n, l) in self.lists.iter_mut() {
            if name.is_none_or(|x| x == n) {
                for line in l.lines.iter_mut() {
                    line.hits = 0;
                    line.logged = None;
                    line.unlogged = 0;
                }
            }
        }
    }
    /// an interface went away: its lists are no longer bound
    pub fn unbind(&mut self, iface: u16) {
        self.bound.retain(|(i, _), _| *i != iface);
    }
}

/// a list for the user interface ("show access-lists")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AclView {
    pub name: String,
    pub kind: AclKind,
    pub numbered: bool,
    /// (sequence number, the rule as "show access-lists" writes it, hits); remarks are left out
    pub rules: Vec<(u32, String, u64)>,
    pub remarks: Vec<String>,
}

impl Engine {
    /// the list bound to an interface decides about a packet; false: denied (noted, and the
    /// sender gets "administratively prohibited" about `quote`, which came in on `from`)
    pub(crate) fn acl_pass(&mut self, ni: usize, iface: u16, out: bool, pkt: &[u8], quote: &[u8], from: u16, cause: Option<HopId>) -> bool {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(st) = self.stack_mut(ni) else {
            return true;
        };
        let Some(name) = st.acls.bound.get(&(iface, out)).cloned() else {
            return true;
        };
        let Some(flow) = Flow::of(pkt) else {
            return true;
        };
        let Some(list) = st.acls.lists.get_mut(&name).filter(|l| l.has_rules()) else {
            return true;
        };
        let (v, at) = list.check(&flow);
        let mut log: Option<(Ace, u64)> = None;
        if let Some(i) = at {
            let l = &mut list.lines[i];
            l.hits += 1;
            if let Item::Rule(r) = &l.item
                && r.log
            {
                l.unlogged += 1;
                if l.logged.is_none_or(|t| now >= t + crate::cli::acl::LOG_EVERY) {
                    log = Some((r.clone(), l.unlogged));
                    l.logged = Some(now);
                    l.unlogged = 0;
                }
            }
        }
        if v.permit {
            self.note(dev, cause, Note::AclPermit { list: name.clone(), seq: v.seq, iface, out });
        } else {
            self.note(dev, cause, Note::AclDeny { list: name.clone(), seq: v.seq, iface, out });
        }
        if let Some((r, n)) = log {
            self.acl_log(ni, &name, &r, &flow, n);
        }
        if !v.permit {
            self.icmp_error(ni, from, quote, Icmp::Unreachable { code: UNREACH_ADMIN, mtu: 0 }, cause);
        }
        v.permit
    }

    /// a list as the selector of NAT: Some(permit) (the deciding line counts the packet), None
    /// when it does not exist or has no rules
    pub(crate) fn acl_match(&mut self, ni: usize, name: &str, f: &Flow) -> Option<bool> {
        let st = self.stack_mut(ni)?;
        let list = st.acls.lists.get_mut(name).filter(|l| l.has_rules())?;
        let (v, at) = list.check(f);
        if let Some(i) = at {
            list.lines[i].hits += 1;
        }
        Some(v.permit)
    }

    /// the access lists of a device in their order
    pub fn access_lists(&self, dev: u32) -> Vec<AclView> {
        let Some(st) = self.node_index(dev).and_then(|ni| self.stack(ni)) else {
            return Vec::new();
        };
        st.acls
            .names()
            .into_iter()
            .filter_map(|n| {
                let l = st.acls.lists.get(&n)?;
                let rules = l
                    .lines
                    .iter()
                    .filter_map(|x| match &x.item {
                        Item::Rule(r) => Some((x.seq, r.show(l.kind), x.hits)),
                        Item::Remark(_) => None,
                    })
                    .collect();
                let remarks = l
                    .lines
                    .iter()
                    .filter_map(|x| match &x.item {
                        Item::Remark(t) => Some(t.clone()),
                        Item::Rule(_) => None,
                    })
                    .collect();
                Some(AclView { name: n, kind: l.kind, numbered: l.numbered, rules, remarks })
            })
            .collect()
    }

    /// which interface uses which list: (interface name, outbound, list)
    pub fn access_groups(&self, dev: u32) -> Vec<(String, bool, String)> {
        let Some(st) = self.node_index(dev).and_then(|ni| self.stack(ni)) else {
            return Vec::new();
        };
        st.acls.bound.iter().filter_map(|((i, out), l)| st.ifaces.get(*i as usize).filter(|f| !f.hidden).map(|f| (f.name.clone(), *out, l.clone()))).collect()
    }
}
