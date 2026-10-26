// scenario -- .scn scripts: what happens in a headless run and what must come out
//
//   parse(text) -> Result<Scenario, Vec<ScnErr>>   every error with its line number
//   parse_duration("1.5s") -> ns   parse_rate("100M") -> bit/s   parse_ppm("0.5%") -> ppm
//   parse_milli("99.5") -> 99500 (thousandths)
//   parse_expectation("LB1 svc lb metric requests >= 10 over 5s") -> Expect   one
//                                                expectation alone (the checks of a lab's goals;
//                                                a leading "expect" is allowed); Expect::dev
//
// One step per line, "#" starts a comment.  A step may start with "at TIME" (virtual time
// since the start); without it, it happens when the previous steps are done.
//
//   seed 42                                      the random numbers (else the project's seed)
//   at 1s PC1: ping -c 4 10.0.0.2                a command line typed on a device
//   PC1: ip addr                                 the same, now
//   run 5s | run until idle                      advance the virtual clock
//   link down R1 Gi0/0 | link up R1 Gi0/0        unplug or replug a cable
//   impair R1 Gi0/1 loss 5% delay 20ms jitter 2ms rate 100M
//   power off SRV2 | power on SRV2
//   expect PC1 output contains "4 received"      | output not contains "..."
//   expect R1 route 10.0.2.0/24 via 10.0.1.2     | via connected | dev Gi0/1 | expect R1 no route P
//   expect hops icmp >= 8                        frames of a protocol seen so far
//   expect SRV1 svc web metric requests == 4     a service's metric
//   pcap build/run.pcapng                        write the capture at the end
//   print "text"                                 a line in the report
//   fault link R1 Gi0/1 [for 5s]                 faults of the lab (a mark on the timeline,
//   fault impair R1 Gi0/1 delay 20ms loss 5%      healed after "for", or by "heal"):
//   fault power KV2                              a cable pulled or worse, a device off, a
//   fault stop WEB2 web                          service stopped, a slow disk (delay per
//   fault disk KV1 delay 20ms factor 10          access, transfer time times factor), a
//   fault partition KV1 KV2 | KV3                partition (one side alone: against all others)
//   heal 2 | heal all | heal power KV2 | heal link R1 Gi0/1 | heal impair R1 Gi0/1 |
//   heal stop WEB2 web | heal disk KV1 | heal partition
//   expect LB1 svc lb metric requests >= 99.5 over 5s       a metric's time series over a span
//   expect LB1 svc lb metric max p99_us < 5000 between 14s and 19s   (jfn_sim series.rs): the
//                                                samples of the last 5 s, or those taken after
//                                                14 s up to 19 s; without min, max or mean the
//                                                series' own (a counter's rate per second, a
//                                                gauge's mean, a latency's quantile of the span);
//                                                the value may have a fraction
//   series build/run.csv                         write the time series as CSV at the end
//   expect PC1 reaches 10.0.2.10                 a ping would get an answer now (A1: judged on
//   expect PC1 not reaches server.lab            a copy of the engine, goal.rs; a name is looked
//                                                up by the device's resolver)
//   move Laptop1 900 300                         put a device's box at x, y (world units, its
//                                                top left corner, as in the project): WLAN
//                                                stations and access points look again
//
// Device and port names are resolved when the scenario runs; this module only checks syntax.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::{Ipv4, Prefix4};

/// a parsed scenario
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scenario {
    pub seed: Option<u64>,
    pub steps: Vec<Step>,
}

/// one line of a scenario
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub line: u32,
    /// virtual time in ns ("at ..."), None = after the previous step
    pub at: Option<u64>,
    pub kind: StepKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    Cli { dev: String, cmd: String },
    Run(u64),
    RunIdle,
    Link { dev: String, port: String, up: bool },
    Impair { dev: String, port: String, imp: Impair },
    Power { dev: String, on: bool },
    Expect(Expect),
    Pcap(String),
    Print(String),
    /// a fault of the lab; with a duration it heals by itself
    Fault { fault: FaultSpec, dur: Option<u64> },
    Heal(HealSpec),
    /// the time series as CSV at the end ("series FILE")
    Series(String),
    /// a device moves on the canvas ("move DEV X Y"): its new top left corner
    Move { dev: String, x: i32, y: i32 },
}

/// a fault of a scenario (the names are resolved when it runs)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultSpec {
    /// the cable at a port is pulled
    Link { dev: String, port: String },
    /// the cable at a port gets worse (delay and jitter on top, loss, a rate cap)
    Impair { dev: String, port: String, imp: Impair },
    /// the device goes off
    Power { dev: String },
    /// a service of the device stops
    Stop { dev: String, svc: String },
    /// the disk: delay per access, transfer time times factor (per mille)
    Disk { dev: String, delay_ns: u64, factor_pm: u32 },
    /// the devices of `a` and of `b` no longer hear each other (b empty: all others)
    Partition { a: Vec<String>, b: Vec<String> },
}

/// what "heal" heals
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealSpec {
    All,
    /// a fault by its number (the order the scenario started them, from 1)
    Id(u32),
    /// the active faults of this kind on this target (values of impair and disk are ignored,
    /// "heal partition" heals every partition)
    Like(FaultSpec),
}

/// link impairments; None leaves a value as it is
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Impair {
    pub loss_ppm: Option<u32>,
    pub delay_ns: Option<u64>,
    pub jitter_ns: Option<u64>,
    pub bps: Option<u64>,
}

/// where a route has to point
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    Addr(Ipv4),
    Connected,
    Iface(String),
    /// any next hop
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    Output { dev: String, needle: String, negate: bool },
    Route { dev: String, prefix: Prefix4, via: Via },
    NoRoute { dev: String, prefix: Prefix4 },
    Hops { proto: String, cmp: Cmp, n: u64 },
    Metric { dev: String, svc: String, name: String, cmp: Cmp, value: i64 },
    /// a metric's time series over a span; the value in thousandths
    Series { dev: String, svc: String, name: String, agg: Agg, cmp: Cmp, milli: i64, span: Span },
    /// a ping from the device to an address or a name gets an answer (negate: none does)
    Reach { dev: String, host: String, negate: bool },
}

impl Expect {
    /// the device it looks at (None: the frames of every cable)
    pub fn dev(&self) -> Option<&str> {
        match self {
            Expect::Output { dev, .. } | Expect::Route { dev, .. } | Expect::NoRoute { dev, .. } | Expect::Metric { dev, .. } | Expect::Series { dev, .. } => Some(dev),
            Expect::Reach { dev, .. } => Some(dev),
            Expect::Hops { .. } => None,
        }
    }
}

/// how a windowed expectation sums up the samples of a series
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Agg {
    /// the series' own: a counter's rate over the span, a gauge's mean, a latency's quantile
    /// (or mean, maximum) of everything in the span
    #[default]
    Natural,
    Min,
    Max,
    /// the mean of the samples' values
    Mean,
}

impl Agg {
    pub fn parse(s: &str) -> Option<Agg> {
        Some(match s {
            "min" => Agg::Min,
            "max" => Agg::Max,
            "mean" => Agg::Mean,
            _ => return None,
        })
    }
    /// the word of the scenario ("" for the series' own)
    pub fn as_str(self) -> &'static str {
        match self {
            Agg::Natural => "",
            Agg::Min => "min",
            Agg::Max => "max",
            Agg::Mean => "mean",
        }
    }
}

/// the samples a windowed expectation looks at
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// "over D": the samples taken in the last D (ns)
    Last(u64),
    /// "between A and B": the samples taken after A up to B (ns)
    Between(u64, u64),
}

/// a comparison
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub fn parse(s: &str) -> Option<Cmp> {
        Some(match s {
            "==" | "=" => Cmp::Eq,
            "!=" => Cmp::Ne,
            "<" => Cmp::Lt,
            "<=" => Cmp::Le,
            ">" => Cmp::Gt,
            ">=" => Cmp::Ge,
            _ => return None,
        })
    }
    pub fn holds<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }
}

/// a syntax error with its line (1-based)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScnErr {
    pub line: u32,
    pub text: String,
}

/// a decimal number with an optional fraction, scaled by `unit`: ("1.5", 1000) -> 1500
fn scaled(num: &str, unit: u64) -> Option<u64> {
    let (int, frac) = match num.split_once('.') {
        Some((i, f)) => (i, f),
        None => (num, ""),
    };
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    if !int.bytes().all(|c| c.is_ascii_digit()) || !frac.bytes().all(|c| c.is_ascii_digit()) || frac.len() > 9 {
        return None;
    }
    let i: u64 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let mut v = i.checked_mul(unit)?;
    if !frac.is_empty() {
        let f: u64 = frac.parse().ok()?;
        let div = 10u64.pow(frac.len() as u32);
        v = v.checked_add(f.checked_mul(unit)? / div)?;
    }
    Some(v)
}

/// "250ms", "1.5s", "10us", "2m", "1h", "500ns" -> nanoseconds
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let u = match unit {
        "ns" => 1,
        "us" | "\u{b5}s" => 1_000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" | "min" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => return None,
    };
    scaled(num, u)
}

/// "100M", "1G", "2.5G", "64k", "2048000" (bit/s; an optional "bit/s" or "bps" suffix)
pub fn parse_rate(s: &str) -> Option<u64> {
    let mut s = s.trim();
    for suf in ["bit/s", "bps", "b/s"] {
        if let Some(r) = s.strip_suffix(suf) {
            s = r.trim_end();
            break;
        }
    }
    let (num, mul) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1_000),
        'M' => (&s[..s.len() - 1], 1_000_000),
        'G' => (&s[..s.len() - 1], 1_000_000_000),
        _ => (s, 1),
    };
    scaled(num, mul)
}

/// "5%", "0.1%", "5000ppm" -> parts per million (at most 1 000 000)
pub fn parse_ppm(s: &str) -> Option<u32> {
    let s = s.trim();
    let v = if let Some(p) = s.strip_suffix('%') {
        scaled(p, 10_000)?
    } else if let Some(p) = s.strip_suffix("ppm") {
        scaled(p, 1)?
    } else {
        return None;
    };
    if v <= 1_000_000 { Some(v as u32) } else { None }
}

/// splits a line into words; "quoted strings" with \" \\ \n \t escapes are one word
fn words(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_whitespace() {
            it.next();
            continue;
        }
        let mut w = String::new();
        if c == '"' {
            it.next();
            let mut closed = false;
            while let Some(c) = it.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match it.next() {
                        Some('n') => w.push('\n'),
                        Some('t') => w.push('\t'),
                        Some(e) => w.push(e),
                        None => break,
                    },
                    c => w.push(c),
                }
            }
            if !closed {
                return Err(String::from("Ein Anführungszeichen ist nicht geschlossen."));
            }
        } else {
            while let Some(&c) = it.peek() {
                if c.is_whitespace() {
                    break;
                }
                w.push(c);
                it.next();
            }
        }
        out.push(w);
    }
    Ok(out)
}

const KEYWORDS: &[&str] = &["seed", "run", "link", "impair", "power", "expect", "pcap", "print", "at", "fault", "heal", "series", "move"];

/// parses a scenario; all syntax errors are collected
pub fn parse(text: &str) -> Result<Scenario, Vec<ScnErr>> {
    let mut sc = Scenario::default();
    let mut errs = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = (i + 1) as u32;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        match parse_line(t, &mut sc) {
            Ok(Some(step)) => sc.steps.push(Step { line, at: step.0, kind: step.1 }),
            Ok(None) => {}
            Err(text) => errs.push(ScnErr { line, text }),
        }
    }
    if errs.is_empty() { Ok(sc) } else { Err(errs) }
}

fn parse_line(t: &str, sc: &mut Scenario) -> Result<Option<(Option<u64>, StepKind)>, String> {
    let mut at = None;
    let mut rest = t;
    if let Some(r) = t.strip_prefix("at ") {
        let r = r.trim_start();
        let end = r.find(char::is_whitespace).ok_or_else(|| String::from("Nach der Zeit fehlt der Schritt."))?;
        at = Some(parse_duration(&r[..end]).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", &r[..end]))?);
        rest = r[end..].trim_start();
    }
    // "DEV: command"
    let first = rest.split(char::is_whitespace).next().unwrap_or("");
    if let Some(dev) = first.strip_suffix(':')
        && !dev.is_empty()
        && !KEYWORDS.contains(&dev)
    {
        let cmd = rest[first.len()..].trim();
        if cmd.is_empty() {
            return Err(format!("Für {} fehlt der Befehl.", dev));
        }
        return Ok(Some((at, StepKind::Cli { dev: String::from(dev), cmd: String::from(cmd) })));
    }
    if let Some((dev, cmd)) = rest.split_once(':')
        && !dev.contains(char::is_whitespace)
        && !dev.is_empty()
        && !KEYWORDS.contains(&dev)
    {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            return Err(format!("Für {} fehlt der Befehl.", dev));
        }
        return Ok(Some((at, StepKind::Cli { dev: String::from(dev), cmd: String::from(cmd) })));
    }
    let w = words(rest)?;
    let ws: Vec<&str> = w.iter().map(String::as_str).collect();
    let kind = match ws.as_slice() {
        ["seed", n] => {
            if at.is_some() {
                return Err(String::from("\u{201e}seed\u{201c} steht ohne Zeitangabe."));
            }
            sc.seed = Some(n.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", n))?);
            return Ok(None);
        }
        ["run", "until", "idle"] | ["run", "idle"] => StepKind::RunIdle,
        ["run", d] => StepKind::Run(parse_duration(d).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", d))?),
        ["link", st, dev, port] if *st == "down" || *st == "up" => StepKind::Link { dev: String::from(*dev), port: String::from(*port), up: *st == "up" },
        ["impair", dev, port, kv @ ..] => StepKind::Impair { dev: String::from(*dev), port: String::from(*port), imp: parse_impair(kv)? },
        ["power", st, dev] if *st == "on" || *st == "off" => StepKind::Power { dev: String::from(*dev), on: *st == "on" },
        ["expect", rest @ ..] => StepKind::Expect(parse_expect(rest)?),
        ["pcap", file] => StepKind::Pcap(String::from(*file)),
        ["series", file] => StepKind::Series(String::from(*file)),
        ["print", text @ ..] => StepKind::Print(text.join(" ")),
        ["fault", rest @ ..] => {
            let (rest, dur) = match rest {
                [r @ .., "for", d] => (r, Some(parse_duration(d).filter(|&x| x > 0).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", d))?)),
                r => (r, None),
            };
            StepKind::Fault { fault: parse_fault(rest)?, dur }
        }
        ["heal", rest @ ..] => StepKind::Heal(parse_heal(rest)?),
        ["move", dev, x, y] => {
            let num = |s: &str| s.parse::<i32>().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", s));
            StepKind::Move { dev: String::from(*dev), x: num(x)?, y: num(y)? }
        }
        [k, ..] => return Err(format!("Unbekannter Schritt \u{201e}{}\u{201c}.", k)),
        [] => return Ok(None),
    };
    Ok(Some((at, kind)))
}

/// "loss 5% delay 20ms jitter 2ms rate 100M" (pairs, at least one)
fn parse_impair(kv: &[&str]) -> Result<Impair, String> {
    if kv.is_empty() || kv.len() % 2 != 0 {
        return Err(String::from("impair braucht Paare wie \u{201e}loss 5%\u{201c}, \u{201e}delay 20ms\u{201c}, \u{201e}jitter 2ms\u{201c}, \u{201e}rate 100M\u{201c}."));
    }
    let mut imp = Impair::default();
    for p in kv.chunks(2) {
        let bad = || format!("\u{201e}{} {}\u{201c} versteht impair nicht.", p[0], p[1]);
        match p[0] {
            "loss" => imp.loss_ppm = Some(parse_ppm(p[1]).ok_or_else(bad)?),
            "delay" => imp.delay_ns = Some(parse_duration(p[1]).ok_or_else(bad)?),
            "jitter" => imp.jitter_ns = Some(parse_duration(p[1]).ok_or_else(bad)?),
            "rate" => imp.bps = Some(parse_rate(p[1]).ok_or_else(bad)?),
            _ => return Err(bad()),
        }
    }
    Ok(imp)
}

/// the words after "fault" (without "for DURATION")
fn parse_fault(w: &[&str]) -> Result<FaultSpec, String> {
    let s = |x: &&str| String::from(*x);
    Ok(match w {
        ["link", dev, port] => FaultSpec::Link { dev: s(dev), port: s(port) },
        ["impair", dev, port, kv @ ..] => FaultSpec::Impair { dev: s(dev), port: s(port), imp: parse_impair(kv)? },
        ["power", dev] | ["power", "off", dev] => FaultSpec::Power { dev: s(dev) },
        ["stop", dev, svc] => FaultSpec::Stop { dev: s(dev), svc: s(svc) },
        ["disk", dev, kv @ ..] => {
            if kv.is_empty() || kv.len() % 2 != 0 {
                return Err(String::from("fault disk braucht Paare wie \u{201e}delay 20ms\u{201c} oder \u{201e}factor 10\u{201c}."));
            }
            let (mut delay_ns, mut factor_pm) = (0, 1000);
            for p in kv.chunks(2) {
                let bad = || format!("\u{201e}{} {}\u{201c} versteht fault disk nicht.", p[0], p[1]);
                match p[0] {
                    "delay" => delay_ns = parse_duration(p[1]).ok_or_else(bad)?,
                    "factor" => factor_pm = scaled(p[1], 1000).filter(|&f| f > 0 && f <= u32::MAX as u64).ok_or_else(bad)? as u32,
                    _ => return Err(bad()),
                }
            }
            FaultSpec::Disk { dev: s(dev), delay_ns, factor_pm }
        }
        ["partition", rest @ ..] if !rest.is_empty() => {
            let (a, b) = match rest.iter().position(|x| *x == "|") {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, &[][..]),
            };
            if a.is_empty() || (rest.contains(&"|") && b.is_empty()) {
                return Err(String::from("Eine Partition braucht Geräte auf beiden Seiten von \u{201e}|\u{201c}."));
            }
            FaultSpec::Partition { a: a.iter().map(s).collect(), b: b.iter().map(s).collect() }
        }
        _ => return Err(format!("Diesen Fehler kenne ich nicht: fault {} (link, impair, power, stop, disk, partition).", w.join(" "))),
    })
}

/// the words after "heal"
fn parse_heal(w: &[&str]) -> Result<HealSpec, String> {
    let s = |x: &&str| String::from(*x);
    Ok(match w {
        ["all"] => HealSpec::All,
        [n] if n.bytes().all(|b| b.is_ascii_digit()) => HealSpec::Id(n.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", n))?),
        ["link", dev, port] => HealSpec::Like(FaultSpec::Link { dev: s(dev), port: s(port) }),
        ["impair", dev, port] => HealSpec::Like(FaultSpec::Impair { dev: s(dev), port: s(port), imp: Impair::default() }),
        ["power", dev] => HealSpec::Like(FaultSpec::Power { dev: s(dev) }),
        ["stop", dev, svc] => HealSpec::Like(FaultSpec::Stop { dev: s(dev), svc: s(svc) }),
        ["disk", dev] => HealSpec::Like(FaultSpec::Disk { dev: s(dev), delay_ns: 0, factor_pm: 1000 }),
        ["partition"] => HealSpec::Like(FaultSpec::Partition { a: Vec::new(), b: Vec::new() }),
        _ => return Err(format!("Was soll geheilt werden? heal {} (all, NUMMER, link, impair, power, stop, disk, partition).", w.join(" "))),
    })
}

/// one expectation as a scenario writes it, with or without the "expect" in front (the checks
/// of a lab's goals); the German reason when it does not parse
pub fn parse_expectation(s: &str) -> Result<Expect, String> {
    let w = words(s.trim())?;
    let ws: Vec<&str> = w.iter().map(String::as_str).collect();
    match ws.as_slice() {
        [] | ["expect"] => Err(String::from("Die Prüfung ist leer.")),
        ["expect", rest @ ..] => parse_expect(rest),
        rest => parse_expect(rest),
    }
}

/// a decimal number in thousandths ("99.5": 99500, "-2": -2000; three decimals at most)
pub fn parse_milli(s: &str) -> Option<i64> {
    let (neg, num) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if num.split_once('.').is_some_and(|(_, f)| f.len() > 3) {
        return None;
    }
    let v = i64::try_from(scaled(num, 1000)?).ok()?;
    Some(if neg { -v } else { v })
}

/// the words after "expect DEV svc NAME metric" of an expectation over a span:
/// "[min|max|mean] METRIC CMP VALUE over D" or "... between A and B"
fn parse_series_expect(dev: &str, svc: &str, w: &[&str]) -> Result<Expect, String> {
    let dur = |s: &str| parse_duration(s).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", s));
    let (head, span) = match w {
        [h @ .., "over", d] => match dur(d)? {
            0 => return Err(String::from("\u{201e}over\u{201c} braucht eine Dauer größer als 0.")),
            x => (h, Span::Last(x)),
        },
        [h @ .., "between", a, "and", b] => {
            let (x, y) = (dur(a)?, dur(b)?);
            if x >= y {
                return Err(format!("Der Zeitraum von {} bis {} ist leer.", a, b));
            }
            (h, Span::Between(x, y))
        }
        _ => return Err(format!("Diese Erwartung kenne ich nicht: expect {} svc {} metric {}", dev, svc, w.join(" "))),
    };
    let (agg, name, c, v) = match head {
        [name, c, v] => (Agg::Natural, name, c, v),
        [a, name, c, v] => (Agg::parse(a).ok_or_else(|| format!("\u{201e}{}\u{201c} fasst keine Reihe zusammen (min, max, mean).", a))?, name, c, v),
        _ => return Err(format!("Diese Erwartung kenne ich nicht: expect {} svc {} metric {}", dev, svc, w.join(" "))),
    };
    let cmp = Cmp::parse(c).ok_or_else(|| format!("\u{201e}{}\u{201c} ist kein Vergleich (==, !=, <, <=, >, >=).", c))?;
    let milli = parse_milli(v).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zahl.", v))?;
    Ok(Expect::Series { dev: String::from(dev), svc: String::from(svc), name: String::from(*name), agg, cmp, milli, span })
}

fn parse_expect(w: &[&str]) -> Result<Expect, String> {
    let prefix = |s: &str| Prefix4::parse(s).map(|p| p.trunc()).ok_or_else(|| format!("\u{201e}{}\u{201c} ist kein IPv4-Präfix (z. B. 10.0.2.0/24).", s));
    let cmp = |s: &str| Cmp::parse(s).ok_or_else(|| format!("\u{201e}{}\u{201c} ist kein Vergleich (==, !=, <, <=, >, >=).", s));
    if let [dev, "svc", svc, "metric", rest @ ..] = w
        && (rest.contains(&"over") || rest.contains(&"between"))
    {
        return parse_series_expect(dev, svc, rest);
    }
    Ok(match w {
        [dev, "output", "contains", needle] => Expect::Output { dev: String::from(*dev), needle: String::from(*needle), negate: false },
        [dev, "output", "not", "contains", needle] => Expect::Output { dev: String::from(*dev), needle: String::from(*needle), negate: true },
        [dev, "route", p] => Expect::Route { dev: String::from(*dev), prefix: prefix(p)?, via: Via::Any },
        [dev, "route", p, "via", "connected"] => Expect::Route { dev: String::from(*dev), prefix: prefix(p)?, via: Via::Connected },
        [dev, "route", p, "via", a] => {
            let ip = Ipv4::parse(a).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine IPv4-Adresse.", a))?;
            Expect::Route { dev: String::from(*dev), prefix: prefix(p)?, via: Via::Addr(ip) }
        }
        [dev, "route", p, "dev", iface] => Expect::Route { dev: String::from(*dev), prefix: prefix(p)?, via: Via::Iface(String::from(*iface)) },
        [dev, "no", "route", p] => Expect::NoRoute { dev: String::from(*dev), prefix: prefix(p)? },
        [dev, "reaches", host] => Expect::Reach { dev: String::from(*dev), host: String::from(*host), negate: false },
        [dev, "not", "reaches", host] => Expect::Reach { dev: String::from(*dev), host: String::from(*host), negate: true },
        ["hops", proto, c, n] => Expect::Hops {
            proto: proto.to_ascii_lowercase(),
            cmp: cmp(c)?,
            n: n.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", n))?,
        },
        [dev, "svc", svc, "metric", name, c, v] => Expect::Metric {
            dev: String::from(*dev),
            svc: String::from(*svc),
            name: String::from(*name),
            cmp: cmp(c)?,
            value: v.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", v))?,
        },
        _ => return Err(format!("Diese Erwartung kenne ich nicht: expect {}", w.join(" "))),
    })
}
