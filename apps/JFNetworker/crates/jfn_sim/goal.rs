// goal -- an expectation of a scenario or of a lab's goal against an engine at its current
// time, the same way for both: whether it holds, what it found, the line of a scenario's report
// and the reason the window shows; the goals of a lab over a run (which are reached, and when)
//
//   judge(&project, &engine, &expect, out) -> Verdict   out(dev): what the device printed (the
//                                         runner: since its last command line of the scenario;
//                                         the window: its console); devices by the project's
//                                         name, else by their host name
//   Verdict { held, text, found, x }      text: the expectation as a scenario writes it
//                                         ("expect LB1 svc lb metric requests >= 5"); found: the
//                                         value it measured or why it could not (Found)
//   verdict.report()                      the report's line ("expect ... (gemessen: 3)", German)
//   verdict.reason() -> Msg               what it found for the app's catalogue ("gemessen:
//                                         245300 µs", "keine Route"); empty for a check that
//                                         held without a value
//   check_text(&expect)                   the check as a lab writes it (the text without
//                                         "expect")
//   LabRun::new(&lab)                     the checks of every goal parsed (jfn_model::lab)
//   run.eval(&project, &engine, out) -> Vec<usize>   looks at the open goals now; the ones that
//                                         are reached by it (a goal is reached when all its
//                                         checks hold at one moment; it stays reached)
//   run.open(i) / reached() / done() / goals[i] (checks, reached, last verdicts)
//
// Judging only reads the engine and is deterministic: the same run judges the same at the same
// virtual time.  The texts of report() are the runner's, which golden reports compare byte for
// byte; reason() words the same findings as messages (jfn_model::msg) for the app's languages.
//
//   "expect cable SW1 Gi1/0/1 length <= 100" / "... up" (P1): the length the engine runs the
//                                         cable at the port with (phys.rs), whether it carries
//
//   probe(&engine, dev, host) -> Found::Ping     "expect PC1 reaches 10.0.2.10" (A1): a ping on
//                                         a copy of the engine (Engine::clone) -- up to PROBES
//                                         echoes, the next when the last is answered or a second
//                                         passed, until the first answer; a name is looked up by
//                                         the device's resolver first.  The engine itself stays
//                                         as it was, so judging still only reads.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::ifname;
use jfn_model::lab::Lab;
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::Project;
use jfn_model::scenario::{Agg, CableCheck, Expect, Span, Via};
use netpkt::decode;

use crate::ip::RouteSrc;
use crate::jobs::{IcmpKind, JobKind, PingEv, PingOpts};
use crate::kernel::{Engine, Ns, MS, SEC, US};
use crate::series::SeriesErr;

/// what a check found besides holding or not
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// nothing to add (an output that is there or not, a route that is there, no route)
    Nothing,
    /// the device it names does not exist
    NoDevice,
    /// a number it measured as the report writes it ("12", "99.5") and the unit of a series
    /// ("µs", "/s"; "" for a service's number)
    Value { text: String, unit: &'static str },
    /// the frames it counted (expect hops)
    Count(u64),
    /// the best route there is instead ("S via 10.0.1.2 dev Gi0/1"), None: no route at all
    Route(Option<String>),
    /// routes there are to a prefix that should have none
    Routes(usize),
    /// expect hops of a protocol it does not know
    UnknownProto,
    /// the device has no such service or the service no such number
    NoMetric,
    /// the series gives no value over the span
    Series(SeriesErr),
    /// the span ends after now
    Later,
    /// the pings of a probe (Expect::Reach): sent, answered, and why none was answered when
    /// something said so (the device is off or has no address, the name is unknown, an ICMP
    /// error came back)
    Ping { sent: u32, received: u32, why: Option<Msg> },
    /// no cable at the port (expect cable)
    NoCable,
    /// the cable carries or not (expect cable ... up | down)
    Carries(bool),
}

/// whether an expectation holds and what it found
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub held: bool,
    /// the expectation as a scenario writes it ("expect PC1 output contains \"x\"")
    pub text: String,
    pub found: Found,
    /// the expectation itself
    pub x: Expect,
}

/// "1.5s", "250ms" as a scenario writes a time
pub(crate) fn dur(d: Ns) -> String {
    if d == 0 {
        String::from("0s")
    } else if d % SEC == 0 {
        format!("{}s", d / SEC)
    } else if d % MS == 0 {
        format!("{}ms", d / MS)
    } else if d % US == 0 {
        format!("{}us", d / US)
    } else {
        format!("{}ns", d)
    }
}

/// a value in thousandths as a decimal ("99.5", "100", "-0.25")
pub(crate) fn milli_text(v: i64) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    if a % 1000 == 0 {
        return format!("{}{}", sign, a / 1000);
    }
    let frac = format!("{:03}", a % 1000);
    format!("{}{}.{}", sign, a / 1000, frac.trim_end_matches('0'))
}

/// a string as a scenario quotes it
pub(crate) fn quote(s: &str) -> String {
    let mut q = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\t' => q.push_str("\\t"),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

/// the P_* bit of a protocol name of "expect hops" (0: every frame)
pub(crate) fn proto_bit(p: &str) -> Option<u32> {
    Some(match p {
        "any" | "all" | "frame" | "frames" => 0,
        "eth" | "ethernet" => decode::P_ETH,
        "vlan" | "802.1q" | "dot1q" => decode::P_VLAN,
        "arp" => decode::P_ARP,
        "ip" | "ipv4" => decode::P_IPV4,
        "ipv6" => decode::P_IPV6,
        "icmp" => decode::P_ICMP,
        "icmpv6" => decode::P_ICMPV6,
        "udp" => decode::P_UDP,
        "tcp" => decode::P_TCP,
        "dhcp" => decode::P_DHCP,
        "dns" => decode::P_DNS,
        "http" => decode::P_HTTP,
        "stp" => decode::P_STP,
        "rip" => decode::P_RIP,
        "ospf" => decode::P_OSPF,
        _ => return None,
    })
}

/// the device a scenario or a check names: by the project's name, else by its host name now
pub fn resolve(p: &Project, e: &Engine, name: &str) -> Option<u32> {
    if let Some(d) = p.find(name) {
        return Some(d.id);
    }
    e.devices().find(|&id| e.node(id).is_some_and(|n| n.name.eq_ignore_ascii_case(name)))
}

/// the expectation as a scenario line writes it ("expect R1 route 10.0.2.0/24 via 10.0.1.2")
pub fn expect_text(x: &Expect) -> String {
    match x {
        Expect::Output { dev, needle, negate } => format!("expect {} output {}contains {}", dev, if *negate { "not " } else { "" }, quote(needle)),
        Expect::Route { dev, prefix, via } => {
            let what = match via {
                Via::Addr(a) => format!(" via {}", a),
                Via::Connected => String::from(" via connected"),
                Via::Iface(i) => format!(" dev {}", i),
                Via::Any => String::new(),
            };
            format!("expect {} route {}{}", dev, prefix, what)
        }
        Expect::NoRoute { dev, prefix } => format!("expect {} no route {}", dev, prefix),
        Expect::Hops { proto, cmp, n } => format!("expect hops {} {} {}", proto, cmp.as_str(), n),
        Expect::Metric { dev, svc, name, cmp, value } => format!("expect {} svc {} metric {} {} {}", dev, svc, name, cmp.as_str(), value),
        Expect::Series { dev, svc, name, agg, cmp, milli, span } => {
            let what = match span {
                Span::Last(x) => format!("over {}", dur(*x)),
                Span::Between(a, b) => format!("between {} and {}", dur(*a), dur(*b)),
            };
            let how = if *agg == Agg::Natural { String::new() } else { format!("{} ", agg.as_str()) };
            format!("expect {} svc {} metric {}{} {} {} {}", dev, svc, how, name, cmp.as_str(), milli_text(*milli), what)
        }
        Expect::Reach { dev, host, negate } => format!("expect {} {}reaches {}", dev, if *negate { "not " } else { "" }, host),
        Expect::Cable { dev, port, check: CableCheck::Len(c, m) } => format!("expect cable {} {} length {} {}", dev, port, c.as_str(), m),
        Expect::Cable { dev, port, check: CableCheck::Up(up) } => format!("expect cable {} {} {}", dev, port, if *up { "up" } else { "down" }),
    }
}

/// the check as a lab writes it: the expectation without "expect"
pub fn check_text(x: &Expect) -> String {
    let t = expect_text(x);
    match t.strip_prefix("expect ") {
        Some(r) => String::from(r),
        None => t,
    }
}

/// whether an expectation holds now; out(dev): what the device printed
pub fn judge(p: &Project, e: &Engine, x: &Expect, out: &dyn Fn(u32) -> String) -> Verdict {
    let (held, found) = judge_found(p, e, x, out);
    Verdict { held, text: expect_text(x), found, x: x.clone() }
}

fn judge_found(p: &Project, e: &Engine, x: &Expect, out: &dyn Fn(u32) -> String) -> (bool, Found) {
    if let Some(name) = x.dev()
        && resolve(p, e, name).is_none()
    {
        return (false, Found::NoDevice);
    }
    let id = x.dev().and_then(|n| resolve(p, e, n)).unwrap_or(0);
    match x {
        Expect::Output { needle, negate, .. } => (out(id).contains(needle.as_str()) != *negate, Found::Nothing),
        Expect::Route { prefix, via, .. } => {
            let rs: Vec<_> = e.routes(id).into_iter().filter(|r| r.prefix == *prefix).collect();
            let best = rs.iter().map(|r| r.ad).min();
            let rs: Vec<_> = rs.into_iter().filter(|r| Some(r.ad) == best).collect();
            let st = e.node(id).and_then(|n| n.ip.as_ref());
            let ifn = |i: Option<u16>| i.and_then(|i| st.and_then(|s| s.ifaces.get(i as usize))).map(|f| f.name.clone()).unwrap_or_default();
            let held = match via {
                Via::Any => !rs.is_empty(),
                Via::Addr(a) => rs.iter().any(|r| r.via == Some(*a)),
                Via::Connected => rs.iter().any(|r| r.src == RouteSrc::Connected),
                Via::Iface(name) => rs.iter().any(|r| ifname::same(&ifn(r.iface), name)),
            };
            if held {
                return (true, Found::Nothing);
            }
            let have = rs.first().map(|r| match r.via {
                Some(v) => format!("{} via {} dev {}", r.src.code(), v, ifn(r.iface)),
                None => format!("{} dev {}", r.src.code(), ifn(r.iface)),
            });
            (false, Found::Route(have))
        }
        Expect::NoRoute { prefix, .. } => {
            let n = e.routes(id).iter().filter(|r| r.prefix == *prefix).count();
            if n == 0 { (true, Found::Nothing) } else { (false, Found::Routes(n)) }
        }
        Expect::Hops { proto, cmp, n } => {
            let Some(bit) = proto_bit(proto) else {
                return (false, Found::UnknownProto);
            };
            let count = e.trace.hops().filter(|h| bit == 0 || h.mask & bit != 0).count() as u64;
            (cmp.holds(count, *n), Found::Count(count))
        }
        Expect::Metric { svc, name, cmp, value, .. } => match e.metric(id, svc, name) {
            Some(v) => (cmp.holds(v, *value), Found::Value { text: format!("{}", v), unit: "" }),
            None => (false, Found::NoMetric),
        },
        Expect::Series { svc, name, agg, cmp, milli, span, .. } => {
            let now = e.now();
            let (from, to) = match span {
                Span::Last(x) => (now.saturating_sub(*x), now),
                Span::Between(a, b) => (*a, *b),
            };
            if to > now {
                return (false, Found::Later);
            }
            match e.series_value(id, svc, name, *agg, from, to) {
                Ok(v) => {
                    let unit = e.series_one(id, svc, name).map_or("", |s| s.unit.as_str());
                    (cmp.holds(v.milli, *milli), Found::Value { text: milli_text(v.milli), unit })
                }
                Err(err) => (false, Found::Series(err)),
            }
        }
        Expect::Reach { host, negate, .. } => {
            let f = probe(e, id, host);
            let answered = matches!(f, Found::Ping { received, .. } if received > 0);
            (answered != *negate, f)
        }
        Expect::Cable { port, check, .. } => {
            let Some(l) = p.link_at(id, port) else {
                return (false, Found::NoCable);
            };
            match check {
                CableCheck::Len(c, m) => {
                    let len = e.cable_phys(l.id).map_or(0, |x| x.len_m);
                    (c.holds(len, *m), Found::Value { text: format!("{}", len), unit: "m" })
                }
                CableCheck::Up(up) => {
                    let now = e.link_rt(l.id).is_some() && e.link_idx_up(l.id);
                    (now == *up, if now == *up { Found::Nothing } else { Found::Carries(now) })
                }
            }
        }
    }
}

/// echoes a probe sends at most (one at a time: the next when the last is answered or timed out)
pub const PROBES: u32 = 5;
/// how long a probe runs at most (virtual time of its copy: a lookup and the echoes) and the
/// events it may take
const PROBE_SPAN: Ns = 15 * SEC;
const PROBE_EVENTS: usize = 2_000_000;

/// what an ICMP error says, as a reason
fn icmp_why(from: netpkt::Ipv4, k: IcmpKind) -> Msg {
    match k {
        IcmpKind::NoRoute => msg!("keine Route zum Ziel"),
        IcmpKind::NetUnreachable => msg!("{} meldet: Netz nicht erreichbar", from),
        IcmpKind::HostUnreachable => msg!("{} meldet: Host nicht erreichbar", from),
        IcmpKind::PortUnreachable | IcmpKind::ProtoUnreachable => msg!("{} meldet: Ziel nicht erreichbar", from),
        IcmpKind::Prohibited => msg!("{} meldet: durch eine Zugriffsliste verboten", from),
        IcmpKind::TtlExceeded | IcmpKind::ReasmExceeded => msg!("{} meldet: TTL abgelaufen", from),
        IcmpKind::FragNeeded(m) => msg!("{} meldet: Fragmentierung nötig (MTU {})", from, m),
        IcmpKind::Other(t, c) => msg!("{} meldet: ICMP Typ {} Code {}", from, t, c),
    }
}

/// a ping from `dev` to `host` (an address or a name) on a copy of the engine: up to PROBES
/// echoes until the first answer (Found::Ping); the engine itself is left as it was
pub fn probe(e: &Engine, dev: u32, host: &str) -> Found {
    let none = |why: Msg| Found::Ping { sent: 0, received: 0, why: Some(why) };
    match e.node(dev) {
        None => return Found::NoDevice,
        Some(n) if !n.power => return none(msg!("das Gerät ist ausgeschaltet")),
        Some(_) => {}
    }
    let mut c = e.clone();
    let opts = PingOpts { count: PROBES, interval: 0, timeout: SEC, ..PingOpts::default() };
    let job = match c.ping_host(dev, host.trim(), opts) {
        Ok(j) => j,
        Err(x) => return none(x.msg()),
    };
    let end = c.now() + PROBE_SPAN;
    let mut budget = PROBE_EVENTS;
    loop {
        let (done, received) = match c.job(job).map(|j| (j.done, &j.kind)) {
            Some((done, JobKind::Ping(p))) => (done, p.received),
            _ => (true, 0),
        };
        if received > 0 || done || c.now() >= end || budget == 0 {
            break;
        }
        let before = c.events_done();
        c.run_until((c.now() + 100 * MS).min(end), budget);
        budget = budget.saturating_sub((c.events_done() - before) as usize);
    }
    let Some(JobKind::Ping(p)) = c.job(job).map(|j| &j.kind) else {
        return none(msg!("keine Antwort"));
    };
    let why = if p.received > 0 {
        None
    } else if p.sent == 0 && p.host.is_some() {
        Some(msg!("der Name {} ist unbekannt", host.trim()))
    } else {
        p.events.iter().find_map(|ev| match ev {
            PingEv::Error { from, kind, .. } => Some(icmp_why(*from, *kind)),
            _ => None,
        })
    };
    Found::Ping { sent: p.sent, received: p.received, why }
}

/// the words of the report for a series without a value
fn series_why(e: SeriesErr) -> &'static str {
    match e {
        SeriesErr::NoService => "keinen solchen Dienst",
        SeriesErr::NoSeries => "keine solche Reihe",
        SeriesErr::NoValues => "keine Werte im Zeitraum",
        SeriesErr::Gone => "das Fenster der Latenzen hält den Zeitraum nicht mehr",
    }
}

impl Verdict {
    /// the line of a scenario's report (German, as golden reports have it)
    pub fn report(&self) -> String {
        let t = &self.text;
        match &self.found {
            Found::Nothing => t.clone(),
            Found::NoDevice => format!("{} (das Gerät gibt es nicht)", t),
            Found::Value { text, .. } if self.held => format!("{} ({})", t, text),
            Found::Value { text, .. } => format!("{} (gemessen: {})", t, text),
            Found::Count(n) if self.held => format!("{} ({})", t, n),
            Found::Count(n) => format!("{} (gezählt: {})", t, n),
            Found::Route(r) => format!("{} (gefunden: {})", t, r.as_deref().unwrap_or("keine Route")),
            Found::Routes(n) => format!("{} (es gibt {})", t, n),
            Found::UnknownProto => format!("{} (unbekanntes Protokoll)", t),
            Found::NoMetric => format!("{} (keinen solchen Dienst oder Wert)", t),
            Found::Series(e) => format!("{} ({})", t, series_why(*e)),
            Found::Later => format!("{} (der Zeitraum endet erst nach jetzt)", t),
            Found::Ping { why: Some(w), received: 0, .. } => format!("{} (keine Antwort: {})", t, w.text()),
            Found::Ping { sent, received, .. } => format!("{} ({} von {} beantwortet)", t, received, sent),
            Found::NoCable => format!("{} (an dem Port steckt kein Kabel)", t),
            Found::Carries(true) => format!("{} (die Leitung ist aktiv)", t),
            Found::Carries(false) => format!("{} (die Leitung ist unten)", t),
        }
    }

    /// what it found, for the user (empty when it held and has no value)
    pub fn reason(&self) -> Msg {
        match &self.found {
            Found::Nothing => match &self.x {
                Expect::Output { negate: false, .. } if !self.held => msg!("nicht in der Ausgabe"),
                Expect::Output { negate: true, .. } if !self.held => msg!("steht in der Ausgabe"),
                _ => Msg::default(),
            },
            Found::NoDevice => msg!("das Gerät gibt es nicht"),
            Found::Value { text, unit } if unit.is_empty() => msg!("gemessen: {}", text),
            Found::Value { text, unit } if unit.starts_with('/') => msg!("gemessen: {}{}", text, unit),
            Found::Value { text, unit } => msg!("gemessen: {} {}", text, unit),
            Found::Count(n) => msg!("gezählt: {}", n),
            Found::Route(None) => msg!("keine Route"),
            Found::Route(Some(r)) => msg!("gefunden: {}", r),
            Found::Routes(n) => msg!("es gibt {} Routen dorthin", n),
            Found::UnknownProto => msg!("unbekanntes Protokoll"),
            Found::NoMetric => msg!("keinen solchen Dienst oder Wert"),
            Found::Series(SeriesErr::NoService) => msg!("keinen solchen Dienst"),
            Found::Series(SeriesErr::NoSeries) => msg!("keine solche Reihe"),
            Found::Series(SeriesErr::NoValues) => msg!("keine Werte im Zeitraum"),
            Found::Series(SeriesErr::Gone) => msg!("das Fenster der Latenzen hält den Zeitraum nicht mehr"),
            Found::Later => msg!("der Zeitraum endet erst nach jetzt"),
            Found::Ping { why: Some(w), received: 0, .. } => msg!("keine Antwort: {}", w.clone()),
            Found::Ping { sent, received: 0, .. } => msg!("keine Antwort auf {} Pings", sent),
            Found::Ping { .. } if self.held => Msg::default(),
            Found::Ping { sent, received, .. } => msg!("{} von {} Pings beantwortet", received, sent),
            Found::NoCable => msg!("an dem Port steckt kein Kabel"),
            Found::Carries(true) => msg!("die Leitung ist aktiv"),
            Found::Carries(false) => msg!("die Leitung ist unten"),
        }
    }
}

// ---------------------------------------------------------------- the goals of a lab

/// a goal of a lab over a run
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoalRun {
    /// its checks; Err: the index of the first one that does not parse and why (German)
    pub checks: Result<Vec<Expect>, (usize, String)>,
    /// the moment all its checks held (a goal stays reached)
    pub reached: Option<Ns>,
    /// what each check found when it was last looked at (empty before)
    pub last: Vec<Verdict>,
}

/// the goals of a lab against a running engine
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabRun {
    pub goals: Vec<GoalRun>,
    /// a goal is looked at only once the goals before it are reached
    pub ordered: bool,
    /// the moment of the last evaluation
    pub at: Option<Ns>,
}

impl LabRun {
    pub fn new(lab: &Lab) -> LabRun {
        let goals = lab.goals.iter().map(|g| GoalRun { checks: g.parse_checks(), reached: None, last: Vec::new() }).collect();
        LabRun { goals, ordered: lab.ordered, at: None }
    }

    /// goal i is looked at: not reached yet, and in an ordered lab every goal before it is
    pub fn open(&self, i: usize) -> bool {
        self.goals.get(i).is_some_and(|g| g.reached.is_none()) && (!self.ordered || self.goals[..i].iter().all(|g| g.reached.is_some()))
    }

    /// the open goals against the engine now; the goals it found reached (in order)
    pub fn eval(&mut self, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String) -> Vec<usize> {
        let now = e.now();
        self.at = Some(now);
        let mut hit = Vec::new();
        for i in 0..self.goals.len() {
            if !self.open(i) {
                continue;
            }
            let g = &mut self.goals[i];
            let Ok(checks) = &g.checks else { continue };
            g.last = checks.iter().map(|x| judge(p, e, x, out)).collect();
            if !g.last.is_empty() && g.last.iter().all(|v| v.held) {
                g.reached = Some(now);
                hit.push(i);
            }
        }
        hit
    }

    /// how many goals are reached
    pub fn reached(&self) -> usize {
        self.goals.iter().filter(|g| g.reached.is_some()).count()
    }

    /// every goal is reached
    pub fn done(&self) -> bool {
        !self.goals.is_empty() && self.goals.iter().all(|g| g.reached.is_some())
    }
}
