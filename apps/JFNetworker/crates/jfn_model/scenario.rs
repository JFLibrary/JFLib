// scenario -- .scn scripts: what happens in a headless run and what must come out
//
//   parse(text) -> Result<Scenario, Vec<ScnErr>>   every error with its line number
//   parse_duration("1.5s") -> ns   parse_rate("100M") -> bit/s   parse_ppm("0.5%") -> ppm
//   parse_milli("99.5") -> 99500 (thousandths)
//   parse_expectation("LB1 svc lb metric requests >= 10 over 5s") -> Expect   one
//                                                expectation alone (the checks of a lab's goals;
//                                                a leading "expect" is allowed); Expect::dev
//   parse_fault_text("link R1 Gi0/1 for 5s") -> (FaultSpec, Option<dur>)   one fault alone (a lab's
//                                                start_faults; a leading "fault" is allowed);
//                                                spec.devices() / spec.cable()
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
//   fault cpu SRV1 factor 3                      the CPU of a device with hardware takes that much
//   fault mem SRV1 12G                           longer; something holds that much of its memory
//   heal 2 | heal all | heal power KV2 | heal link R1 Gi0/1 | heal impair R1 Gi0/1 |
//   heal stop WEB2 web | heal disk KV1 | heal partition | heal cpu SRV1 | heal mem SRV1
//   resize SRV1 model server-l                   other hardware for a device: a model's figures
//   resize SRV1 cores 8 speed 1.5 ram 32G        and / or single ones (speed: of the reference
//     [swap 4G] [disk nvme] [disk_gb 960]        core), a reboot of 30 s unless [down 10s] or
//     [down 10s] [live]                          [live] (no reboot) say otherwise
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
//   place PC1 "Raum 2"                           a device to another place of the physical
//   place SW1 "Rack 1" 12                        view (P1, phys.rs): a room, a desk, a rack at
//                                                a unit (none: the lowest free one); the
//                                                automatic lengths of its cables follow
//   cable SW1 Gi1/0/1 length 132 | length auto   a cable's length by hand (metres), automatic
//   expect cable SW1 Gi1/0/1 length <= 100       the length of the cable at a port (no
//   expect cable SW1 Gi1/0/1 up | down           comparison: ==), whether it carries
//   deploy DEV1 GROUP|H1,H2 unit web             a rolling deployment (A2, jfn_sim deploy.rs)
//     [files /root/app[,PATH]]                   from DEV1 to a group of the project or a
//     [health active|tcp PORT|http PORT PATH [STATUS]]  list of hosts: the files, the unit,
//     [wait 2s] [timeout 10s] [rollback]         its check, the wait before the next host,
//                                                what a failure does (default: it stops)
//   load set GEN load rps 200 | users 40         changes under load (SA2; rewinding replays them):
//   lb add LB1 lb web3 10.0.2.13 [port 80]       the rate of a running load generator or its users;
//     [weight 2]                                 a backend of a load balancer that comes, drains,
//   lb drain|remove|enable LB1 lb web3           goes, is taken back; its weight or algorithm;
//   lb weight LB1 lb web3 3                      the consistency and quorums of a KV node;
//   lb algo LB1 lb least-conn                    settings of a service by their config's paths,
//   kv set KV1 kv consistency eventual [w 1]     taken without a restart (Service::reconfigure, a
//     [r 1]                                      step's line says why one cannot); a cache forgets
//   svc set WEB1 web workers 8 [time_ms 2.5]     what it holds
//   cache clear CACHE1 cache
//   expect WEB1 unit web active|inactive|failed|enabled|disabled   a unit of a host (A2)
//   expect DEV1 deploy done|running|failed|rolledback|aborted      its newest deployment
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
    /// a device goes to another place of the physical view ("place DEV PLACE [UNIT]"; unit 0:
    /// the lowest free one of a rack)
    Place { dev: String, place: String, unit: u16 },
    /// the length of the cable at a port set by hand ("cable DEV PORT length M|auto"; 0: auto)
    CableLen { dev: String, port: String, m: u32 },
    /// a rolling deployment ("deploy ..."; the names are resolved when it runs)
    Deploy(DeployStep),
    /// other hardware for a device ("resize DEV ..."); the downtime when it says one (0: live)
    Resize { dev: String, hw: HwSpec, down: Option<u64> },
    /// the load of a running generator ("load set DEV SVC rps R | users U"); thousandths of a
    /// request per second
    LoadSet { dev: String, svc: String, rate_m: Option<u64>, users: Option<u32> },
    /// a change of a running load balancer ("lb add|drain|remove|enable|weight|algo DEV SVC ...")
    Lb { dev: String, svc: String, op: LbStep },
    /// the consistency and quorums of a KV node ("kv set DEV SVC ...")
    KvSet { dev: String, svc: String, consistency: Option<String>, w: Option<u8>, r: Option<u8> },
    /// settings of a service taken without a restart ("svc set DEV SVC PATH VALUE ...")
    SvcSet { dev: String, svc: String, sets: Vec<(String, String)> },
    /// a cache forgets what it holds ("cache clear DEV SVC")
    CacheClear { dev: String, svc: String },
}

/// what "lb" does
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LbStep {
    Add { name: String, addr: String, port: u16, weight: u32 },
    Drain(String),
    Remove(String),
    Enable(String),
    Weight(String, u32),
    Algo(String),
}

/// what "resize" says of the hardware: a model's figures and single ones on top; what it does not
/// name stays as the device has it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HwSpec {
    pub model: Option<String>,
    pub cores: Option<u32>,
    pub speed_pm: Option<u32>,
    pub ram_mb: Option<u64>,
    pub swap_mb: Option<u64>,
    pub disk: Option<crate::catalog::DiskClass>,
    pub disk_gb: Option<u64>,
}

/// a deployment of a scenario: the devices by name, a group or a list
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployStep {
    pub src: String,
    /// a group's name, or device names separated by commas
    pub hosts: String,
    pub unit: String,
    /// files and folders of the source (empty: the unit file alone)
    pub files: Vec<String>,
    pub health: crate::unit::Health,
    pub wait: u64,
    pub timeout: u64,
    pub on_fail: crate::unit::OnFail,
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
    /// the CPU of a device with hardware: its costs in per mille
    Cpu { dev: String, factor_pm: u32 },
    /// something holds that many bytes of its memory
    Mem { dev: String, bytes: u64 },
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
    /// the cable at a port: its length in metres compared, or whether it carries (P1)
    Cable { dev: String, port: String, check: CableCheck },
    /// a unit of a host: active, inactive, failed, enabled, disabled (A2)
    Unit { dev: String, unit: String, state: String },
    /// the newest deployment of a device: done, running, failed, rolledback, aborted (A2)
    Deploy { dev: String, state: String },
}

/// what "expect cable DEV PORT ..." looks at
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CableCheck {
    Len(Cmp, u32),
    Up(bool),
}

impl Expect {
    /// the device it looks at (None: the frames of every cable)
    pub fn dev(&self) -> Option<&str> {
        match self {
            Expect::Output { dev, .. } | Expect::Route { dev, .. } | Expect::NoRoute { dev, .. } | Expect::Metric { dev, .. } | Expect::Series { dev, .. } => Some(dev),
            Expect::Reach { dev, .. } => Some(dev),
            Expect::Cable { dev, .. } => Some(dev),
            Expect::Unit { dev, .. } | Expect::Deploy { dev, .. } => Some(dev),
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
pub(crate) fn scaled(num: &str, unit: u64) -> Option<u64> {
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

const KEYWORDS: &[&str] = &["seed", "run", "link", "impair", "power", "expect", "pcap", "print", "at", "fault", "heal", "series", "move", "place", "cable", "deploy", "resize"];

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
            let (fault, dur) = fault_words(rest)?;
            StepKind::Fault { fault, dur }
        }
        ["heal", rest @ ..] => StepKind::Heal(parse_heal(rest)?),
        ["move", dev, x, y] => {
            let num = |s: &str| s.parse::<i32>().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", s));
            StepKind::Move { dev: String::from(*dev), x: num(x)?, y: num(y)? }
        }
        ["place", dev, place] => StepKind::Place { dev: String::from(*dev), place: String::from(*place), unit: 0 },
        ["place", dev, place, unit] => {
            let u = unit.parse::<u16>().ok().filter(|&u| u > 0).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Höheneinheit (1, 2, ...).", unit))?;
            StepKind::Place { dev: String::from(*dev), place: String::from(*place), unit: u }
        }
        ["cable", dev, port, "length", m] => StepKind::CableLen { dev: String::from(*dev), port: String::from(*port), m: metres(m)? },
        ["deploy", src, hosts, rest @ ..] => StepKind::Deploy(parse_deploy(src, hosts, rest)?),
        ["resize", dev, rest @ ..] => parse_resize(dev, rest)?,
        ["load", "set", dev, svc, what, v] => {
            let (dev, svc) = (String::from(*dev), String::from(*svc));
            match *what {
                "rps" => StepKind::LoadSet { dev, svc, rate_m: Some(scaled(v, 1000).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Rate.", v))?), users: None },
                "users" => StepKind::LoadSet { dev, svc, rate_m: None, users: Some(v.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", v))?) },
                w => return Err(format!("\u{201e}{}\u{201c} versteht load set nicht (rps, users).", w)),
            }
        }
        ["lb", "add", dev, svc, name, addr, rest @ ..] => parse_lb_add(dev, svc, name, addr, rest)?,
        ["lb", "drain", dev, svc, name] => StepKind::Lb { dev: String::from(*dev), svc: String::from(*svc), op: LbStep::Drain(String::from(*name)) },
        ["lb", "remove", dev, svc, name] => StepKind::Lb { dev: String::from(*dev), svc: String::from(*svc), op: LbStep::Remove(String::from(*name)) },
        ["lb", "enable", dev, svc, name] => StepKind::Lb { dev: String::from(*dev), svc: String::from(*svc), op: LbStep::Enable(String::from(*name)) },
        ["lb", "weight", dev, svc, name, w] => StepKind::Lb { dev: String::from(*dev), svc: String::from(*svc), op: LbStep::Weight(String::from(*name), w.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", w))?) },
        ["lb", "algo", dev, svc, algo] => StepKind::Lb { dev: String::from(*dev), svc: String::from(*svc), op: LbStep::Algo(String::from(*algo)) },
        ["kv", "set", dev, svc, rest @ ..] => parse_kv_set(dev, svc, rest)?,
        ["svc", "set", dev, svc, rest @ ..] => {
            if rest.is_empty() || rest.len() % 2 != 0 {
                return Err(String::from("svc set braucht Paare wie \u{201e}workers 8\u{201c}."));
            }
            StepKind::SvcSet { dev: String::from(*dev), svc: String::from(*svc), sets: rest.chunks(2).map(|p| (String::from(p[0]), String::from(p[1]))).collect() }
        }
        ["cache", "clear", dev, svc] => StepKind::CacheClear { dev: String::from(*dev), svc: String::from(*svc) },
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

/// the words of a deployment after its source and hosts
fn parse_deploy(src: &str, hosts: &str, w: &[&str]) -> Result<DeployStep, String> {
    use crate::unit::{Health, OnFail};
    let mut d = DeployStep { src: String::from(src), hosts: String::from(hosts), unit: String::new(), files: Vec::new(), health: Health::Active, wait: 1_000_000_000, timeout: 10_000_000_000, on_fail: OnFail::Stop };
    let dur = |s: &str| parse_duration(s).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", s));
    let port = |s: &str| s.parse::<u16>().map_err(|_| format!("\u{201e}{}\u{201c} ist kein Port.", s));
    let mut r = w;
    loop {
        r = match r {
            [] => break,
            ["unit", n, t @ ..] => {
                d.unit = String::from(*n);
                t
            }
            ["files", f, t @ ..] => {
                d.files = f.split(',').map(String::from).collect();
                t
            }
            ["health", "active", t @ ..] => {
                d.health = Health::Active;
                t
            }
            ["health", "tcp", p, t @ ..] => {
                d.health = Health::Tcp { port: port(p)? };
                t
            }
            ["health", "http", p, path, t @ ..] => {
                let (status, t) = match t {
                    [s, x @ ..] if s.bytes().all(|b| b.is_ascii_digit()) => (s.parse::<u16>().unwrap_or(200), x),
                    x => (200, x),
                };
                d.health = Health::Http { port: port(p)?, path: String::from(*path), status };
                t
            }
            ["wait", x, t @ ..] => {
                d.wait = dur(x)?;
                t
            }
            ["timeout", x, t @ ..] => {
                d.timeout = dur(x)?;
                t
            }
            ["rollback", t @ ..] => {
                d.on_fail = OnFail::Rollback;
                t
            }
            [k, ..] => return Err(format!("\u{201e}{}\u{201c} versteht deploy nicht.", k)),
        };
    }
    if d.unit.is_empty() {
        return Err(String::from("deploy braucht \u{201e}unit NAME\u{201c}."));
    }
    Ok(d)
}

/// "16G", "512M", "64k", "4096" -> bytes (a power of 1024; an optional "iB" or "B")
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.strip_suffix("iB").or_else(|| s.strip_suffix('B')).unwrap_or(s);
    let (num, unit) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1u64 << 10),
        'M' => (&s[..s.len() - 1], 1 << 20),
        'G' => (&s[..s.len() - 1], 1 << 30),
        'T' => (&s[..s.len() - 1], 1 << 40),
        _ => (s, 1),
    };
    scaled(num, unit)
}

/// the words of a resize after its device
fn parse_resize(dev: &str, w: &[&str]) -> Result<StepKind, String> {
    let (mut hw, mut down) = (HwSpec::default(), None);
    let mb = |k: &str, s: &str| parse_size(s).map(|b| b >> 20).ok_or_else(|| format!("\u{201e}{} {}\u{201c} versteht resize nicht.", k, s));
    let mut r = w;
    loop {
        r = match r {
            [] => break,
            ["live", t @ ..] => {
                down = Some(0);
                t
            }
            ["down", x, t @ ..] => {
                down = Some(parse_duration(x).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", x))?);
                t
            }
            ["model", m, t @ ..] => {
                hw.model = Some(String::from(*m));
                t
            }
            ["cores", n, t @ ..] => {
                hw.cores = Some(n.parse::<u32>().ok().filter(|&c| c > 0).ok_or_else(|| format!("\u{201e}cores {}\u{201c} versteht resize nicht.", n))?);
                t
            }
            ["speed", x, t @ ..] => {
                hw.speed_pm = Some(scaled(x, 1000).filter(|&v| v > 0 && v <= u32::MAX as u64).ok_or_else(|| format!("\u{201e}speed {}\u{201c} versteht resize nicht.", x))? as u32);
                t
            }
            ["ram", x, t @ ..] => {
                hw.ram_mb = Some(mb("ram", x)?);
                t
            }
            ["swap", x, t @ ..] => {
                hw.swap_mb = Some(mb("swap", x)?);
                t
            }
            ["disk", x, t @ ..] => {
                hw.disk = Some(crate::catalog::DiskClass::parse(x).ok_or_else(|| format!("\u{201e}disk {}\u{201c} versteht resize nicht (sata, nvme, nvme-fast, hdd).", x))?);
                t
            }
            ["disk_gb", x, t @ ..] => {
                hw.disk_gb = Some(x.parse::<u64>().map_err(|_| format!("\u{201e}disk_gb {}\u{201c} versteht resize nicht.", x))?);
                t
            }
            [k, ..] => return Err(format!("\u{201e}{}\u{201c} versteht resize nicht.", k)),
        };
    }
    if hw == HwSpec::default() {
        return Err(String::from("resize braucht \u{201e}model NAME\u{201c} oder Werte wie \u{201e}cores 8\u{201c}, \u{201e}ram 32G\u{201c}."));
    }
    Ok(StepKind::Resize { dev: String::from(dev), hw, down })
}

/// the words after "fault", with or without "for DURATION"
fn fault_words(w: &[&str]) -> Result<(FaultSpec, Option<u64>), String> {
    let (rest, dur) = match w {
        [r @ .., "for", d] => (r, Some(parse_duration(d).filter(|&x| x > 0).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Zeitangabe (z. B. 250ms, 1.5s).", d))?)),
        r => (r, None),
    };
    Ok((parse_fault(rest)?, dur))
}

/// one fault as a scenario writes it, with or without the "fault" in front (a lab's start_faults); the
/// German reason when it does not parse
pub fn parse_fault_text(s: &str) -> Result<(FaultSpec, Option<u64>), String> {
    let w = words(s.trim())?;
    let ws: Vec<&str> = w.iter().map(String::as_str).collect();
    match ws.as_slice() {
        [] | ["fault"] => Err(String::from("Der Fehler ist leer.")),
        ["fault", rest @ ..] => fault_words(rest),
        rest => fault_words(rest),
    }
}

impl FaultSpec {
    /// the devices it names
    pub fn devices(&self) -> Vec<&str> {
        match self {
            FaultSpec::Link { dev, .. } | FaultSpec::Impair { dev, .. } | FaultSpec::Power { dev } | FaultSpec::Stop { dev, .. } => alloc::vec![dev.as_str()],
            FaultSpec::Disk { dev, .. } | FaultSpec::Cpu { dev, .. } | FaultSpec::Mem { dev, .. } => alloc::vec![dev.as_str()],
            FaultSpec::Partition { a, b } => a.iter().chain(b.iter()).map(String::as_str).collect(),
        }
    }
    /// the port whose cable it is about (a device and a port)
    pub fn cable(&self) -> Option<(&str, &str)> {
        match self {
            FaultSpec::Link { dev, port } | FaultSpec::Impair { dev, port, .. } => Some((dev, port)),
            _ => None,
        }
    }
}

/// the words of "lb add" after its address
fn parse_lb_add(dev: &str, svc: &str, name: &str, addr: &str, w: &[&str]) -> Result<StepKind, String> {
    let (mut port, mut weight) = (80u16, 1u32);
    let mut r = w;
    loop {
        r = match r {
            [] => break,
            ["port", x, t @ ..] => {
                port = x.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist kein Port.", x))?;
                t
            }
            ["weight", x, t @ ..] => {
                weight = x.parse().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Zahl.", x))?;
                t
            }
            [k, ..] => return Err(format!("\u{201e}{}\u{201c} versteht lb add nicht (port, weight).", k)),
        };
    }
    Ok(StepKind::Lb { dev: String::from(dev), svc: String::from(svc), op: LbStep::Add { name: String::from(name), addr: String::from(addr), port, weight } })
}

/// the words of "kv set" after its service
fn parse_kv_set(dev: &str, svc: &str, w: &[&str]) -> Result<StepKind, String> {
    if w.is_empty() || w.len() % 2 != 0 {
        return Err(String::from("kv set braucht Paare wie \u{201e}consistency eventual\u{201c}, \u{201e}w 2\u{201c}, \u{201e}r 1\u{201c}."));
    }
    let (mut consistency, mut wq, mut rq) = (None, None, None);
    for p in w.chunks(2) {
        let bad = || format!("\u{201e}{} {}\u{201c} versteht kv set nicht.", p[0], p[1]);
        match p[0] {
            "consistency" => consistency = Some(String::from(p[1])),
            "w" => wq = Some(p[1].parse::<u8>().map_err(|_| bad())?),
            "r" => rq = Some(p[1].parse::<u8>().map_err(|_| bad())?),
            _ => return Err(bad()),
        }
    }
    Ok(StepKind::KvSet { dev: String::from(dev), svc: String::from(svc), consistency, w: wq, r: rq })
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
        ["cpu", dev, "factor", x] => FaultSpec::Cpu { dev: s(dev), factor_pm: scaled(x, 1000).filter(|&f| f > 0 && f <= u32::MAX as u64).ok_or_else(|| format!("\u{201e}factor {}\u{201c} versteht fault cpu nicht.", x))? as u32 },
        ["mem", dev, x] => FaultSpec::Mem { dev: s(dev), bytes: parse_size(x).ok_or_else(|| format!("\u{201e}{}\u{201c} ist keine Größe (z. B. 512M, 12G).", x))? },
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
        _ => return Err(format!("Diesen Fehler kenne ich nicht: fault {} (link, impair, power, stop, disk, partition, cpu, mem).", w.join(" "))),
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
        ["cpu", dev] => HealSpec::Like(FaultSpec::Cpu { dev: s(dev), factor_pm: 1000 }),
        ["mem", dev] => HealSpec::Like(FaultSpec::Mem { dev: s(dev), bytes: 0 }),
        ["partition"] => HealSpec::Like(FaultSpec::Partition { a: Vec::new(), b: Vec::new() }),
        _ => return Err(format!("Was soll geheilt werden? heal {} (all, NUMMER, link, impair, power, stop, disk, partition, cpu, mem).", w.join(" "))),
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
        [dev, "unit", unit, st] if matches!(*st, "active" | "inactive" | "failed" | "enabled" | "disabled") => Expect::Unit { dev: String::from(*dev), unit: String::from(*unit), state: String::from(*st) },
        [dev, "deploy", st] if matches!(*st, "done" | "running" | "failed" | "rolledback" | "aborted") => Expect::Deploy { dev: String::from(*dev), state: String::from(*st) },
        [dev, "reaches", host] => Expect::Reach { dev: String::from(*dev), host: String::from(*host), negate: false },
        [dev, "not", "reaches", host] => Expect::Reach { dev: String::from(*dev), host: String::from(*host), negate: true },
        ["cable", dev, port, "length", c, v] => Expect::Cable { dev: String::from(*dev), port: String::from(*port), check: CableCheck::Len(cmp(c)?, metres(v)?) },
        ["cable", dev, port, "length", v] => Expect::Cable { dev: String::from(*dev), port: String::from(*port), check: CableCheck::Len(Cmp::Eq, metres(v)?) },
        ["cable", dev, port, st] if *st == "up" || *st == "down" => Expect::Cable { dev: String::from(*dev), port: String::from(*port), check: CableCheck::Up(*st == "up") },
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

/// "132", "132m" -> 132; "auto" -> 0 (a cable's length)
fn metres(s: &str) -> Result<u32, String> {
    if s == "auto" {
        return Ok(0);
    }
    s.strip_suffix('m').unwrap_or(s).parse::<u32>().map_err(|_| format!("\u{201e}{}\u{201c} ist keine Länge in Metern.", s))
}
