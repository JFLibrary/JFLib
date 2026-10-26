// keyframe -- rewinding (K1): keyframes, snapshots of the whole engine at moments of a run, and
// the inputs from outside between them, so that the state at any recorded moment comes back
// exactly: the nearest keyframe at or before it, then a deterministic replay
//
//   Budget                          the policy: bytes and keyframes at most, their spacing
//   Rewind                          the keyframes and inputs of a run (Inline::rewind_on keeps
//                                   one): keyframes() / bytes() / inputs() / input_count() /
//                                   earliest() / made / thinned
//   Pos { t, events, inputs }       a position of a run: the virtual time, the events run, the
//                                   inputs taken (Inline::pos)
//   Input { at, events, kind }      what came from outside and where: InKind::Apply (an Action:
//                                   edits, power, cables, faults, moves, the steps of a
//                                   scenario), Cli (a console line), Interrupt, Call (a change
//                                   that is no command: a ping, a PDU's service, a lookup;
//                                   Inline::call), Clock (a run brought the clock to `at` and
//                                   the time series took samples there)
//   Target                          At(t): everything at or before moment t; Pos(p): exactly
//                                   that position of the run
//   Restore                         a replay on its way (Inline::restore / restore_pos):
//                                   run(&Rewind, budget) -> done, progress() in per mille,
//                                   finish() -> Past
//   Past { pos, eng }               the engine at that moment; Inline::resume makes it the
//                                   present ("Von hier fortsetzen")
//   footprint(&Engine)              the bytes a keyframe of the engine owns (an estimate);
//                                   Footprint::of: the same by part (devices with their logs,
//                                   services and TCP; cables, time series, the event queue ...)
//   digest(&Engine)                 the whole state as a number; trace_digest(&Trace, hops,
//                                   events) a stretch of the recording: a replay equals the run
//
// A keyframe is a clone of the engine without its recording.  It shares what never changes --
// frames are Arc<[u8]> in the cable queues and the event queue, the catalog's models, the
// command trees and the scripts' compiled code are shared too -- and keeps of the trace only its
// counters and the frames that may still be on a cable (Trace::stub), so it costs what the
// devices, cables, jobs, time series and the event queue hold.  The recording stays with the
// run (the live engine); a restored engine's trace holds the keyframe's stub and what its replay
// recorded.  Other parts of the engine (services, BGP sessions, files on devices) come along
// through Clone without code of their own; Service::footprint tells the budget what a service
// holds (the default guesses 1 KiB).
//
// The inputs are the only way the engine changes besides its own events.  Each is recorded
// with its position (the events run so far and the clock), and a replay over a clone of the
// keyframe runs the same events up to that count, sets the clock and takes the input again:
// the same engine byte for byte, the same frames and decisions in its recording (digest and
// trace_digest of both are equal; keyframetest.rs checks it).  Positions count events, not
// time, as a command in the simulation mode may come between two events of the same moment.
// The time series take a sample when run_until brings the clock to a moment (series.rs); where
// that happens in a run is an input too (Clock), since a long quiet stretch is sampled
// differently in one piece than in many.
//
// Policy (Budget): spacing is counted in events, which is what a replay costs (a quiet stretch
// of virtual time replays in no time, a busy lab runs thousands of events a second).  A
// keyframe is due when max(gap_events, events run so far / divisor) events ran since the
// newest one: the replay to any recent moment stays short, and a long run makes keyframes
// less often the longer it runs.  Together the keyframes hold at most `max_bytes` (footprint)
// and number at most `max_count`; beyond that the keyframe whose removal leaves the smallest
// gap relative to its age goes (gap and age in events; a tie takes the older one), so the
// recent past stays dense, the oldest are thinned out first and the spacing grows with the
// age.  The first keyframe (the start of the recording) and the newest stay while another one
// can go; when only those two are left and still too many bytes, the first goes, and moments
// before the new first keyframe cannot be restored any more (earliest(); the inputs before it
// go with it).  The newest always stays.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Write;
use core::mem::size_of;

use crate::action::Action;
use crate::kernel::{Engine, Ns};
use crate::trace::{HopId, Trace};

/// how many keyframes, how big and how far apart
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// bytes the keyframes hold together at most (footprint)
    pub max_bytes: usize,
    /// keyframes at most
    pub max_count: usize,
    /// a keyframe after this many events
    pub gap_events: u64,
    /// the gap grows with the run: at least the events run so far divided by this
    pub divisor: u64,
}

impl Default for Budget {
    fn default() -> Budget {
        Budget { max_bytes: 64 << 20, max_count: 256, gap_events: 20_000, divisor: 64 }
    }
}

/// a position of a run
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    /// the virtual time
    pub t: Ns,
    /// events run since the start (Engine::events_done)
    pub events: u64,
    /// inputs recorded before it (their number: Rewind::input_count)
    pub inputs: usize,
}

impl Pos {
    /// this position comes at or before `p` in the same run
    pub fn before(&self, p: &Pos) -> bool {
        self.t <= p.t && self.events <= p.events && self.inputs <= p.inputs
    }
}

/// what came from outside
#[derive(Clone)]
pub enum InKind {
    /// an action (Cmd::Apply): edits, power, cables, faults, moves, scenario steps
    Apply(Action),
    /// a line typed on a device's console (Cmd::Cli)
    Cli { dev: u32, line: String },
    /// Ctrl+C or Ctrl+Shift+6 on a device's console (Cmd::Interrupt)
    Interrupt { dev: u32 },
    /// a change of the engine that is no command (Inline::call): `what` names it
    Call { what: &'static str, f: Arc<dyn Fn(&mut Engine) + Send + Sync> },
    /// run_until brought the clock to the moment and the time series took samples there
    Clock,
}

impl fmt::Debug for InKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            InKind::Apply(a) => write!(f, "Apply({:?})", a),
            InKind::Cli { dev, line } => write!(f, "Cli({}, {:?})", dev, line),
            InKind::Interrupt { dev } => write!(f, "Interrupt({})", dev),
            InKind::Call { what, .. } => write!(f, "Call({})", what),
            InKind::Clock => write!(f, "Clock"),
        }
    }
}

/// an input with its position: the clock and the events run when it came
#[derive(Clone, Debug)]
pub struct Input {
    pub at: Ns,
    pub events: u64,
    pub kind: InKind,
}

/// a snapshot of the engine at a position
#[derive(Clone)]
struct Keyframe {
    pos: Pos,
    bytes: usize,
    eng: Engine,
}

/// where a replay goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// every event and input at or before moment t (the view of the simulation mode)
    At(Ns),
    /// exactly this position of the run
    Pos(Pos),
}

/// the engine at a past position
#[derive(Clone)]
pub struct Past {
    pub pos: Pos,
    pub eng: Engine,
}

/// the keyframes and inputs of a run
pub struct Rewind {
    pub cfg: Budget,
    /// where the recording began
    start: Pos,
    /// the inputs from number `base` on (those before the first keyframe went with it)
    inputs: Vec<Input>,
    base: usize,
    frames: Vec<Keyframe>,
    bytes: usize,
    /// keyframes made and thinned out since the start
    pub made: u64,
    pub thinned: u64,
}

impl Rewind {
    /// a recording from the engine's present on: its first keyframe now
    pub(crate) fn new(cfg: Budget, e: &mut Engine) -> Rewind {
        let start = Pos { t: e.now(), events: e.events_done(), inputs: 0 };
        let mut rw = Rewind { cfg, start, inputs: Vec::new(), base: 0, frames: Vec::new(), bytes: 0, made: 0, thinned: 0 };
        rw.push(e);
        rw
    }
    /// the number the next input gets (inputs recorded since the start)
    pub fn input_count(&self) -> usize {
        self.base + self.inputs.len()
    }
    /// the inputs still kept, with the number of the first
    pub fn inputs(&self) -> (usize, &[Input]) {
        (self.base, &self.inputs)
    }
    fn input(&self, k: usize) -> Option<&Input> {
        self.inputs.get(k.checked_sub(self.base)?)
    }
    /// the keyframes: position and footprint, oldest first
    pub fn keyframes(&self) -> Vec<(Pos, usize)> {
        self.frames.iter().map(|k| (k.pos, k.bytes)).collect()
    }
    /// the bytes the keyframes hold together
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    /// the earliest moment a replay can reach (the first keyframe)
    pub fn earliest(&self) -> Option<Ns> {
        self.frames.first().map(|k| k.pos.t)
    }
    /// where the recording began
    pub fn start(&self) -> Pos {
        self.start
    }

    /// the position of the engine with this recording
    pub(crate) fn pos_of(&self, e: &Engine) -> Pos {
        Pos { t: e.now(), events: e.events_done(), inputs: self.input_count() }
    }

    /// an input at the engine's position
    pub(crate) fn record(&mut self, e: &Engine, kind: InKind) {
        self.inputs.push(Input { at: e.now(), events: e.events_done(), kind });
    }

    /// a keyframe is due (see the policy at the top)
    pub(crate) fn due(&self, e: &Engine) -> bool {
        let Some(last) = self.frames.last() else { return true };
        let p = self.pos_of(e);
        if p == last.pos {
            return false;
        }
        let run = p.events.saturating_sub(self.start.events);
        p.events.saturating_sub(last.pos.events) >= self.cfg.gap_events.max(1).max(run / self.cfg.divisor.max(1))
    }

    /// a keyframe of the engine now, then the budget
    pub(crate) fn push(&mut self, e: &mut Engine) {
        let pos = self.pos_of(e);
        if self.frames.last().is_some_and(|k| k.pos == pos) {
            return;
        }
        let tr = core::mem::take(&mut e.trace);
        let mut kf = e.clone();
        kf.trace = tr.stub(e.now);
        e.trace = tr;
        // output nobody took yet is no state (a replay drops its own)
        kf.cli.reports.clear();
        let bytes = footprint(&kf);
        self.bytes += bytes;
        self.made += 1;
        self.frames.push(Keyframe { pos, bytes, eng: kf });
        self.thin(pos.events);
    }

    // removes keyframes while the budget is exceeded (see the policy at the top)
    fn thin(&mut self, now: u64) {
        while self.frames.len() > 1 && (self.bytes > self.cfg.max_bytes || self.frames.len() > self.cfg.max_count) {
            let n = self.frames.len();
            let mut k = 0;
            if n > 2 {
                // the smallest gap relative to its age: gap_k / age_k < gap_j / age_j
                let score = |k: usize| {
                    let (a, b) = (self.frames[k - 1].pos.events, self.frames[k + 1].pos.events);
                    ((b - a) as u128, (now.saturating_sub(a) + 1) as u128)
                };
                k = 1;
                let mut best = score(1);
                for j in 2..n - 1 {
                    let s = score(j);
                    if s.0 * best.1 < best.0 * s.1 {
                        (k, best) = (j, s);
                    }
                }
            }
            let gone = self.frames.remove(k);
            self.bytes -= gone.bytes;
            self.thinned += 1;
            if k == 0 {
                // the inputs before the new first keyframe are no use any more
                let from = self.frames[0].pos.inputs;
                let drop = from.saturating_sub(self.base).min(self.inputs.len());
                self.inputs.drain(..drop);
                self.base += drop;
            }
        }
    }

    /// the run goes on from `p` (Inline::resume): the inputs after it and the keyframes after
    /// it go
    pub(crate) fn cut(&mut self, p: Pos) {
        self.inputs.truncate(p.inputs.saturating_sub(self.base));
        while self.frames.last().is_some_and(|k| !k.pos.before(&p)) {
            if let Some(k) = self.frames.pop() {
                self.bytes -= k.bytes;
            }
        }
    }

    /// a replay to `target`: from the nearest keyframe at or before it, or from `from` (a past
    /// state shown before) when that lies between them; None before the earliest keyframe
    pub(crate) fn restore(&self, target: Target, from: Option<&Past>) -> Option<Restore> {
        let ok = |p: &Pos| match target {
            Target::At(t) => p.t <= t,
            Target::Pos(q) => p.before(&q),
        };
        let kf = self.frames.iter().rev().find(|k| ok(&k.pos))?;
        let (pos, eng) = match from {
            Some(p) if ok(&p.pos) && kf.pos.before(&p.pos) && p.pos.inputs >= self.base => (p.pos, p.eng.clone()),
            _ => (kf.pos, kf.eng.clone()),
        };
        Some(Restore { target, eng, next: pos.inputs, from: pos, done: false })
    }
}

/// a replay on its way to a past moment
pub struct Restore {
    target: Target,
    eng: Engine,
    /// the number of the next input to take
    next: usize,
    /// where it began
    from: Pos,
    done: bool,
}

impl Restore {
    /// replays at most `budget` events -> it arrived
    pub fn run(&mut self, rw: &Rewind, budget: usize) -> bool {
        let mut left = budget;
        while !self.done {
            let next = rw.input(self.next).filter(|i| match self.target {
                Target::At(t) => i.at <= t,
                Target::Pos(p) => self.next < p.inputs,
            });
            match next {
                Some(i) => {
                    if !self.step_to(i.events, &mut left) {
                        return false;
                    }
                    take(&mut self.eng, i);
                    self.next += 1;
                    let _ = self.eng.take_reports();
                }
                None => {
                    match self.target {
                        Target::At(t) => {
                            while self.eng.next_time().is_some_and(|x| x <= t) {
                                if left == 0 {
                                    return false;
                                }
                                self.eng.step();
                                left -= 1;
                            }
                            self.eng.now = self.eng.now.max(t);
                        }
                        Target::Pos(p) => {
                            if !self.step_to(p.events, &mut left) {
                                return false;
                            }
                            self.eng.now = self.eng.now.max(p.t);
                        }
                    }
                    let _ = self.eng.take_reports();
                    self.done = true;
                }
            }
        }
        true
    }

    // the events up to number `n`
    fn step_to(&mut self, n: u64, left: &mut usize) -> bool {
        while self.eng.events_done() < n {
            if *left == 0 {
                return false;
            }
            if self.eng.step().is_none() {
                break;
            }
            *left -= 1;
            if *left % 4096 == 0 {
                let _ = self.eng.take_reports();
            }
        }
        true
    }

    pub fn done(&self) -> bool {
        self.done
    }
    pub fn target(&self) -> Target {
        self.target
    }
    /// how far it is, in per mille of the virtual time to go (1000: arrived)
    pub fn progress(&self) -> u32 {
        if self.done {
            return 1000;
        }
        let to = match self.target {
            Target::At(t) => t,
            Target::Pos(p) => p.t,
        };
        let (a, now) = (self.from.t, self.eng.now());
        if to <= a {
            return 0;
        }
        ((now.saturating_sub(a) as u128 * 1000 / (to - a) as u128) as u32).min(999)
    }
    /// the engine at the target (call once run gave true; before, where the replay stands)
    pub fn finish(self) -> Past {
        let pos = Pos { t: self.eng.now(), events: self.eng.events_done(), inputs: self.next };
        Past { pos, eng: self.eng }
    }
}

// one input again, at its moment
fn take(e: &mut Engine, i: &Input) {
    if let InKind::Clock = i.kind {
        e.run_until(i.at, usize::MAX);
        return;
    }
    e.now = e.now.max(i.at);
    match &i.kind {
        InKind::Apply(a) => {
            let _ = e.apply(a.clone());
        }
        InKind::Cli { dev, line } => {
            let _ = e.cli_line(*dev, line);
        }
        InKind::Interrupt { dev } => {
            let _ = e.cli_interrupt(*dev);
        }
        InKind::Call { f, .. } => f(e),
        InKind::Clock => {}
    }
}

// ---------------------------------------------------------------- footprint

/// the bytes a keyframe of engine `e` owns besides what it shares (frames, models, command
/// trees, compiled scripts): an estimate from the sizes of its parts and the entries of its
/// tables, buffers and logs; Service::footprint for what a service holds
pub fn footprint(e: &Engine) -> usize {
    Footprint::of(e).total()
}

/// what an engine's parts hold (estimates in bytes; footprint is their sum)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    /// the devices: ports, tables, sockets, command lines (logs, services and TCP below)
    pub devices: usize,
    /// the devices' logs
    pub logs: usize,
    /// the devices' services (Service::footprint and their own size)
    pub services: usize,
    /// the devices' TCP connections with their buffers
    pub tcp: usize,
    /// the cables with their queues
    pub cables: usize,
    /// the time series
    pub series: usize,
    /// the event queue
    pub queue: usize,
    /// jobs, faults, WLAN, the recording (all of it for a running engine; a keyframe keeps the
    /// frames still on a cable), the engine itself
    pub rest: usize,
}

impl Footprint {
    pub fn of(e: &Engine) -> Footprint {
        use crate::link::{LinkRt, Queued};
        let mut f = Footprint::default();
        let (heap, slab) = e.queue_len();
        f.queue = heap * 24 + slab * size_of::<Option<crate::kernel::Ev>>();
        for n in e.nodes.iter().flatten() {
            node_bytes(n, &mut f);
        }
        f.devices += e.nodes.len() * size_of::<Option<crate::node::Node>>() + e.dev_idx.len() * 48;
        for l in e.links.iter().flatten() {
            f.cables += size_of::<LinkRt>() + l.queued() * size_of::<Queued>();
        }
        f.cables += e.links.len() * size_of::<Option<LinkRt>>() + e.link_idx.len() * 48;
        f.series = e.series.footprint();
        f.rest = size_of::<Engine>() + e.jobs.len() * (size_of::<crate::jobs::Job>() + 256) + e.cli.jobs.len() * 48;
        f.rest += e.faults().len() * size_of::<crate::fault::FaultMark>() + 256;
        f.rest += (e.wlan.devs.len() + e.wlan.stas.len() + e.wlan.assocs.len()) * 256;
        f.rest += e.trace.hops().len() * (size_of::<crate::trace::Hop>() + 16);
        f
    }
    pub fn total(&self) -> usize {
        self.devices + self.logs + self.services + self.tcp + self.cables + self.series + self.queue + self.rest
    }
}

// a device: its ports and timers, its bridge, IPv4 / IPv6 tables, its OS with sockets,
// services, log and TCP, its command line
fn node_bytes(n: &crate::node::Node, f: &mut Footprint) {
    use crate::bridge::{BridgePort, FdbEntry};
    use crate::ip::{ArpEntry, Iface, Route};
    use crate::os::{LogLine, Sock, SvcSlot};
    let mut b = size_of::<crate::node::Node>() + n.name.len() + n.ports.len() * size_of::<crate::node::Port>() + n.timers.len() * 4;
    // the command line with its configuration text and settings
    let cfg = &n.cli.cfg;
    b += size_of::<crate::cli::CliDev>() + 256 + cfg.startup.len() + (cfg.methods.len() + cfg.descr.len()) * 64 + n.cli.queue.iter().map(|l| l.len() + 24).sum::<usize>();
    if let Some(br) = &n.bridge {
        b += size_of::<crate::bridge::Bridge>() + br.ports.len() * size_of::<BridgePort>() + br.fdb.len() * (size_of::<FdbEntry>() + 40) + br.vlans.len() * 64 + 512;
    }
    if let Some(ip) = &n.ip {
        b += size_of::<crate::ip::IpStack>() + ip.ifaces.len() * (size_of::<Iface>() + 64);
        b += ip.arp.values().map(|a| size_of::<ArpEntry>() + 40 + a.queue.len() * 256).sum::<usize>();
        b += (ip.statics.len() + ip.dynamic.len()) * size_of::<Route>() + ip.reasm.len() * 2048 + ip.groups.len() * 8;
        // access lists, translations, IPv6 and the routing table of the last lookup
        b += 1024 + ip.dynamic.len() * size_of::<Route>();
    }
    if let Some(os) = &n.os {
        b += size_of::<crate::os::Os>() + os.socks.len() * size_of::<Sock>() + os.inbox.len() * 256 + os.hosts.len() * 64;
        f.logs += os.log.lines.iter().map(|l| size_of::<LogLine>() + l.tag.len() + l.text.len()).sum::<usize>();
        f.services += os.svcs.iter().map(|s| size_of::<SvcSlot>() + s.name.len() + s.svc.as_ref().map_or(0, |x| core::mem::size_of_val(&**x) + x.footprint())).sum::<usize>();
        f.tcp += os.tcp.footprint();
        // the resolver's cache and lookups, its latency window, the disk
        b += os.resolver.cache.len() * 128 + os.resolver.lookups.len() * 256 + os.resolver.latency.bytes() + 1024;
    }
    f.devices += b;
}

// ---------------------------------------------------------------- digests

/// FNV-1a over what is written
struct Fnv(u64);

impl Write for Fnv {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01B3);
        }
        Ok(())
    }
}

/// the whole state of engine `e` as a number: the clock and the event queue, every device (as
/// its Debug text, with each service's status line and numbers), cable, job, the command lines'
/// shared part, faults, time series, WLAN and the counters of the recording.  Output nobody
/// took yet (reports) and the routing table of the last lookup (a cache: equal inputs give the
/// same table) are left out.  Two engines with the same digest behave the same.
pub fn digest(e: &Engine) -> u64 {
    let mut w = Fnv(0xCBF2_9CE4_8422_2325);
    let _ = e.queue_text(&mut w);
    for (i, n) in e.nodes.iter().enumerate() {
        let Some(n) = n else {
            let _ = write!(w, "|{} -", i);
            continue;
        };
        let cache = n.ip.as_ref().map(|ip| ip.rib_cache.take());
        let _ = write!(w, "|{} {:?}", i, n);
        if let (Some(ip), Some(c)) = (n.ip.as_ref(), cache) {
            ip.rib_cache.replace(c);
        }
        for s in n.os.iter().flat_map(|os| os.svcs.iter()) {
            if let Some(x) = &s.svc {
                let _ = write!(w, "|{} {:?}", s.name, x.metrics_at(e.now()));
            }
        }
    }
    let _ = write!(w, "|{:?}|{:?} {:?}|{:?} {:?}|{} {}", e.links, e.dev_idx, e.link_idx, e.jobs, e.next_job, e.seed, e.strict_mdix);
    let _ = write!(w, "|{} {:?}|{:?}|{:?}|{:?}|", e.cli.debug, e.cli.jobs, e.faults, e.series, e.wlan);
    let _ = e.trace.counters(&mut w);
    w.0
}

/// the frames with ids in `hops` and the decisions with numbers in `events` of a recording as
/// a number (one that left the ring counts as missing)
pub fn trace_digest(tr: &Trace, hops: core::ops::Range<HopId>, events: core::ops::Range<u64>) -> u64 {
    let mut w = Fnv(0xCBF2_9CE4_8422_2325);
    for id in hops {
        let _ = match tr.hop(id) {
            Some(h) => write!(w, "|{:?}", h),
            None => write!(w, "|{} -", id),
        };
    }
    for seq in events {
        let _ = match tr.event(seq) {
            Some(x) => write!(w, "|{:?}", x),
            None => write!(w, "|{} -", seq),
        };
    }
    w.0
}
