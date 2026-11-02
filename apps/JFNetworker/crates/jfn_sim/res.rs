// res -- the hardware of a device (Os::res, SA1): CPU cores with a run queue, RAM with swap;
// the foundation of the software-architecture lab.  Shaped like disk.rs: a job is computed when
// it is submitted and the caller gets its completion, so nothing here adds an event
//
//   Res::new(hw, own)                  cores, speed, RAM from jfn_model's Hw
//   Res::submit(svc, now, cost, usl)   a job of `cost` at a reference core: (its start, its end)
//   Res::turn(now) / charge(svc, core, now, took)   a place in the queue for work whose size is not
//                                      known yet (a script's slice), what it took when its turn came
//   Res::done_at(t) / running(t) / waiting(t)   core time spent up to t, cores busy, jobs waiting
//   Res::set_mem(svc, bytes) / forget(svc)      what a service holds (Service::mem)
//   Res::metric(name, t)               the numbers of `expect DEV svc _host metric NAME`
//   Res::svc_busy(svc) / svc_recent(svc, t, secs)   a service's CPU time (top)
//   Usl::of(config) / SvcMem::of(config, base_mb, conn_kb)   what a service's config says
//   CpuGate<T>                         requests that wait for the CPU before a service takes them
//                                      (lb.rs, kv.rs: the service's wheel says when)
//
//   OsApi::cpu(cost)                   a job now: when it is done (no hardware: now, nothing else)
//   OsApi::cpu_usl(cost, usl)          the same, slower under contention (below)
//   OsApi::cpu_or_timer(cost)          a job now (no hardware: now + cost, as a plain timer)
//   OsApi::cpu_slice(&mut left)        the next quantum of a long job (QUANTUM): when it is done
//   OsApi::cpu_slice_usl(&mut left, usl)   the same with a service's factor
//   OsApi::metered() / cpu_turn() / cpu_charge(core, took) / cpu_ps(ps)   a script's way (scriptsvc.rs)
//   Service::mem()                     the bytes a service holds; the OS asks after each of its
//                                      calls on a device with hardware (Engine::svc_call)
//   Action::Resize { dev, hw, downtime } / Action::Metered { on }   other hardware (0: hot-plugged),
//                                      the project's "Hardware simulieren"
//   Engine::res_view(dev)              the figures for the window; HOST ("_host"): the name of a
//                                      device's own time series (series.rs)
//
// Without hardware there is no state at all: every call above returns on its first branch, no
// event, no random number, no series.  A project without Device::hw and without Project::metered
// runs byte for byte as it ever did.
//
// CPU.  First come, first served across the cores: a job takes the core that is free first, and
// its completion is known at once.  That is exact for one central FIFO queue (M/G/c) and costs
// O(cores).  Costs are given at a reference core: speed_pm 1000 means a cost of 1 ms takes 1 ms,
// and a script's instruction its 5 ns.  A long job should come in QUANTUM pieces (cpu_slice), so
// that a scan does not make short requests wait for its whole length.  Processor sharing would
// have the same saturation throughput, but completions would move with every arrival and callers
// would need events for them.
//
// What this does not model: a reservation cannot be withdrawn.  The queued work of a service that
// was killed or stopped keeps its cores busy until it would have ended (a power off does end it).
//
// Contention.  Where the thing is modelled it is a real single-server queue (a route's serial_us,
// the disk).  On top a service may name a factor after the Universal Scalability Law,
// 1 + sigma (N - 1) + kappa N (N - 1), N being the cores busy with this job, on the job's cost: at
// saturation throughput is N / (1 + sigma (N - 1) + kappa N (N - 1)) times a core's, with its peak
// at N = sqrt((1 - sigma) / kappa) and falling beyond that.  It is the formula, not a measurement
// of locks and cache lines.
//
// Memory.  What the services hold (Service::mem: a base, per connection, their data) and a
// "memory hog" fault.  Beyond the RAM, costs rise linearly with the overcommit up to 20 times at
// the end of the swap, and page-ins use the real disk (Disk::access, so they wait for it and show
// in its numbers): one page per TOUCH of CPU work is touched, of which the part not in RAM is
// read; an accumulator, not random numbers, decides which job takes the next ones.  Beyond RAM and
// swap the largest service is killed: Job::Oom (it runs after the current call, as svc_call has
// the caller out of its slot), exit status 137, which a unit's restart policy sees.  The operating
// system itself holds nothing here, and caches are not freed under pressure.
//
// Network: no CPU per frame.  The line rates and queues are the bottleneck there, and that path is
// the hottest of the goldens.
//
// Other hardware (Action::Resize): by default a reboot with a downtime (a mark on the timeline,
// Fault::Resize); what the disk holds stays, the cores and the memory start empty (os_power).  With
// no downtime the hardware changes in the running system: new jobs pay the new speed, cores that go
// hand their waiting work to the others.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::catalog::{hw_of, DiskClass, Hw};
use jfn_model::msg::Msg;

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, MS, SEC, US};
use crate::os::{Job, Os, OsApi};
use crate::trace::Note;

/// a long job's piece
pub const QUANTUM: Ns = 4 * MS;
/// the time series of a device with hardware go by this name (expect DEV svc _host metric ...)
pub const HOST: &str = "_host";
/// the downtime of a resize unless it says otherwise
pub const RESIZE_DOWN: Ns = 30 * SEC;
/// the exit status of a service the memory killed
pub const OOM_EXIT: i32 = 137;
/// the unit of paging
pub const PAGE: u64 = 4096;
/// one page is touched per this much CPU work at a reference core
const TOUCH: Ns = 10 * US;
/// the most the overcommit multiplies costs, per mille (at the end of the swap)
const THRASH_MAX_PM: u64 = 20_000;
/// seconds of a service's CPU time kept (top -d)
const SECS_KEEP: usize = 128;
/// the shortest job with a cost (time goes on)
const MIN_JOB: Ns = 1;
/// the place in the queue a turn holds until its work is known (20 instructions of a script)
const TURN_HOLD: Ns = 100;

/// the factor of the Universal Scalability Law, in parts per million
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usl {
    pub sigma_ppm: u32,
    pub kappa_ppm: u32,
}

impl Usl {
    /// {"usl": {"sigma": 0.02, "kappa": 0.0005}} of a service's config
    pub fn of(c: &Value) -> Usl {
        let Some(u) = c.get("usl") else {
            return Usl::default();
        };
        let ppm = |k: &str| u.get(k).and_then(|v| crate::httpsvc::decimal(v, 1_000_000)).unwrap_or(0).min(u32::MAX as u64) as u32;
        Usl { sigma_ppm: ppm("sigma"), kappa_ppm: ppm("kappa") }
    }
    /// 1 + sigma (n - 1) + kappa n (n - 1), in parts per million
    pub fn factor_ppm(&self, n: u64) -> u64 {
        let m = n.saturating_sub(1);
        1_000_000 + self.sigma_ppm as u64 * m + self.kappa_ppm as u64 * n * m
    }
}

/// what a service holds in memory: a base, per connection, data of its own
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SvcMem {
    pub base: u64,
    pub per_conn: u64,
    pub data: u64,
}

impl SvcMem {
    /// {"mem_mb": 64, "conn_kb": 64, "data_mb": 0} of a service's config, with its kind's defaults
    pub fn of(c: &Value, base_mb: u64, conn_kb: u64) -> SvcMem {
        let get = |k: &str, scale: u64, d: u64| c.get(k).and_then(|v| crate::httpsvc::decimal(v, scale)).unwrap_or(d);
        SvcMem { base: get("mem_mb", 1 << 20, base_mb << 20), per_conn: get("conn_kb", 1 << 10, conn_kb << 10), data: get("data_mb", 1 << 20, 0) }
    }
    /// the bytes with `conns` connections and `held` bytes that come and go
    pub fn bytes(&self, conns: u64, held: u64) -> u64 {
        self.base + self.per_conn * conns + self.data + held
    }
}

/// what waits for the CPU before a service takes it: a job's completion on the service's own wheel
/// says when (a service with no hardware, where cpu() answers now, never holds anything here)
#[derive(Clone, Debug)]
pub struct CpuGate<T> {
    next: u64,
    held: BTreeMap<u64, T>,
}

impl<T> Default for CpuGate<T> {
    fn default() -> CpuGate<T> {
        CpuGate { next: 0, held: BTreeMap::new() }
    }
}

impl<T> CpuGate<T> {
    /// it waits: its number
    pub fn hold(&mut self, v: T) -> u64 {
        self.next += 1;
        self.held.insert(self.next, v);
        self.next
    }
    /// its time came
    pub fn take(&mut self, id: u64) -> Option<T> {
        self.held.remove(&id)
    }
    /// what waited goes with the service
    pub fn clear(&mut self) {
        self.held.clear();
    }
}

/// a service's part of the hardware
#[derive(Clone, Debug, Default)]
struct SvcRes {
    bytes: u64,
    busy_ns: u64,
    /// its CPU time by second of virtual time, the last SECS_KEEP
    secs: BTreeMap<u64, u64>,
}

/// the hardware of a device at run time
#[derive(Clone, Debug)]
pub struct Res {
    pub hw: Hw,
    /// the device's own figures (Device::hw), else its model's in a metered project
    pub own: bool,
    /// when each core is free
    free_at: Vec<Ns>,
    /// when the jobs that wait for a core start, in that order
    queue: VecDeque<Ns>,
    svcs: BTreeMap<u32, SvcRes>,
    /// what the services hold together
    used: u64,
    /// a memory hog fault
    pub hog: u64,
    /// a slow CPU fault: costs in per mille
    pub slow_pm: u32,
    /// pages touched and not read yet, in thousandths of a page
    page_acc: u64,
    /// an Oom job is on its way
    pub(crate) oom_due: bool,
    /// jobs submitted, core time reserved (ns), the time jobs waited for a core (ns)
    pub jobs: u64,
    pub busy_ns: u64,
    pub wait_ns: u64,
    pub max_wait: Ns,
    pub pageins: u64,
    pub oom_kills: u64,
}

impl Res {
    pub fn new(hw: Hw, own: bool) -> Res {
        let hw = hw.sane();
        Res { free_at: alloc::vec![0; hw.cores as usize], hw, own, queue: VecDeque::new(), svcs: BTreeMap::new(), used: 0, hog: 0, slow_pm: 1000, page_acc: 0, oom_due: false, jobs: 0, busy_ns: 0, wait_ns: 0, max_wait: 0, pageins: 0, oom_kills: 0 }
    }
    /// the bytes of the RAM and the swap
    pub fn ram(&self) -> u64 {
        self.hw.ram_mb << 20
    }
    pub fn swap(&self) -> u64 {
        self.hw.swap_mb << 20
    }
    /// what is held now: by the services and a memory hog
    pub fn used(&self) -> u64 {
        self.used + self.hog
    }
    /// the bytes beyond the RAM
    pub fn over(&self) -> u64 {
        self.used().saturating_sub(self.ram())
    }
    /// more is held than RAM and swap take
    pub fn exhausted(&self) -> bool {
        self.over() > self.swap()
    }
    /// what a service holds
    pub fn svc_bytes(&self, svc: u32) -> u64 {
        self.svcs.get(&svc).map_or(0, |s| s.bytes)
    }
    /// the CPU time a service reserved so far
    pub fn svc_busy(&self, svc: u32) -> u64 {
        self.svcs.get(&svc).map_or(0, |s| s.busy_ns)
    }
    /// its CPU time in the `secs` seconds up to t
    pub fn svc_recent(&self, svc: u32, t: Ns, secs: u64) -> u64 {
        let now = t / SEC;
        let from = (now + 1).saturating_sub(secs);
        self.svcs.get(&svc).map_or(0, |s| s.secs.range(from..=now).map(|x| *x.1).sum())
    }
    /// costs per mille by the overcommit: 1000 up to the RAM, THRASH_MAX_PM at the end of the swap
    fn thrash_pm(&self) -> u64 {
        let over = self.over();
        if over == 0 {
            return 1000;
        }
        let swap = self.swap().max(1);
        (1000 + (THRASH_MAX_PM - 1000) as u128 * over as u128 / swap as u128).min(THRASH_MAX_PM as u128) as u64
    }
    /// a cost at a reference core as this device takes it: speed, a slow CPU fault, the overcommit
    pub fn scale(&self, cost: Ns) -> Ns {
        let d = cost as u128 * 1000 * self.slow_pm as u128 * self.thrash_pm() as u128 / (self.hw.speed_pm.max(1) as u128 * 1_000_000);
        (d.min(u64::MAX as u128) as Ns).max(if cost > 0 { MIN_JOB } else { 0 })
    }
    /// a script's time per instruction as this device takes it
    pub fn scale_ps(&self, ps: u64) -> u64 {
        (ps as u128 * 1000 * self.slow_pm as u128 * self.thrash_pm() as u128 / (self.hw.speed_pm.max(1) as u128 * 1_000_000)).min(u64::MAX as u128) as u64
    }
    /// the core that is free first (the lowest of equals)
    fn earliest(&self) -> usize {
        let mut best = 0;
        for (i, t) in self.free_at.iter().enumerate() {
            if *t < self.free_at[best] {
                best = i;
            }
        }
        best
    }
    /// when a core is free
    pub fn next_free(&self) -> Ns {
        self.free_at.get(self.earliest()).copied().unwrap_or(0)
    }
    /// cores busy at t
    pub fn running(&self, t: Ns) -> u64 {
        self.free_at.iter().filter(|f| **f > t).count() as u64
    }
    /// jobs waiting for a core at t
    pub fn waiting(&self, t: Ns) -> u64 {
        (self.queue.len() - self.queue.partition_point(|s| *s <= t)) as u64
    }
    /// core time spent up to t, for a t not before the last submission: what was reserved, less
    /// what the cores still have to do after t (their reservations are contiguous)
    pub fn done_at(&self, t: Ns) -> u64 {
        self.busy_ns.saturating_sub(self.free_at.iter().map(|f| f.saturating_sub(t)).sum::<u64>())
    }
    fn book(&mut self, svc: u32, start: Ns, end: Ns) {
        let s = self.svcs.entry(svc).or_default();
        s.busy_ns += end - start;
        let (mut t, mut sec) = (start, start / SEC);
        while t < end {
            let cut = ((sec + 1) * SEC).min(end);
            *s.secs.entry(sec).or_insert(0) += cut - t;
            t = cut;
            sec += 1;
        }
        while s.secs.len() > SECS_KEEP {
            s.secs.pop_first();
        }
    }
    fn wait_for(&mut self, now: Ns, start: Ns) {
        while self.queue.front().is_some_and(|s| *s <= now) {
            self.queue.pop_front();
        }
        if start > now {
            self.queue.push_back(start);
        }
        self.wait_ns += start - now;
        self.max_wait = self.max_wait.max(start - now);
    }
    /// a job of `cost` at a reference core that arrives at `now`: its start and its end
    pub fn submit(&mut self, svc: u32, now: Ns, cost: Ns, usl: Usl) -> (Ns, Ns) {
        let c = self.earliest();
        let start = self.free_at[c].max(now);
        // the cores busy with it, this job's among them
        let cost = if usl == Usl::default() {
            cost
        } else {
            let n = (self.running(now) + 1).min(self.hw.cores as u64);
            (cost as u128 * usl.factor_ppm(n) as u128 / 1_000_000).min(u64::MAX as u128) as Ns
        };
        let end = start + self.scale(cost);
        self.free_at[c] = end;
        self.jobs += 1;
        self.busy_ns += end - start;
        self.wait_for(now, start);
        self.book(svc, start, end);
        (start, end)
    }
    /// a place in the queue for work whose size shows only when it runs: when its turn comes and
    /// on which core
    pub fn turn(&mut self, now: Ns) -> (Ns, usize) {
        let c = self.earliest();
        let start = self.free_at[c].max(now);
        self.free_at[c] = start + TURN_HOLD;
        self.busy_ns += TURN_HOLD;
        self.wait_for(now, start);
        (start, c)
    }
    /// the work of a turn that came at `now`: its core owes that time.  What was promised to the
    /// jobs queued behind it stays as it was, so the total work is exact and only its order slips
    /// by this little
    pub fn charge(&mut self, svc: u32, core: usize, now: Ns, took: Ns) {
        let c = core.min(self.free_at.len().saturating_sub(1));
        let extra = took.saturating_sub(TURN_HOLD);
        self.free_at[c] = self.free_at[c].max(now + TURN_HOLD) + extra;
        self.busy_ns += extra;
        self.jobs += 1;
        self.book(svc, now, now + took.max(1));
    }
    /// how many pages a job of `cost` has to read now (the accumulator keeps the rest)
    fn pages(&mut self, cost: Ns) -> u64 {
        let (over, used) = (self.over(), self.used());
        if over == 0 || used == 0 {
            return 0;
        }
        // thousandths of a page: touched pages times the part of the memory not in RAM
        self.page_acc += (cost as u128 * 1000 / TOUCH as u128 * over as u128 / used as u128).min(u64::MAX as u128 / 2) as u64;
        let n = self.page_acc / 1000;
        self.page_acc %= 1000;
        self.pageins += n;
        n
    }
    /// what a service holds now
    pub fn set_mem(&mut self, svc: u32, bytes: u64) {
        let s = self.svcs.entry(svc).or_default();
        self.used = self.used - s.bytes + bytes;
        s.bytes = bytes;
    }
    /// a service stopped or went: it holds nothing; its CPU time stays counted
    pub fn forget(&mut self, svc: u32) {
        self.set_mem(svc, 0);
    }
    /// the service that holds the most (the newest of equals)
    pub fn largest(&self) -> Option<u32> {
        let mut best: Option<(u64, u32)> = None;
        for (id, s) in &self.svcs {
            if s.bytes > 0 && best.is_none_or(|b| s.bytes >= b.0) {
                best = Some((s.bytes, *id));
            }
        }
        best.map(|b| b.1)
    }
    /// the power went off: the cores and the memory start empty, what ran never ends
    pub fn power_off(&mut self, now: Ns) {
        let left: u64 = self.free_at.iter().map(|f| f.saturating_sub(now)).sum();
        self.busy_ns = self.busy_ns.saturating_sub(left);
        for f in self.free_at.iter_mut() {
            *f = 0;
        }
        self.queue.clear();
        for s in self.svcs.values_mut() {
            s.bytes = 0;
        }
        self.used = 0;
        self.page_acc = 0;
        self.oom_due = false;
    }
    /// other hardware in the running system: cores that go hand their work to the others
    pub fn resize(&mut self, now: Ns, hw: Hw) {
        let hw = hw.sane();
        self.free_at.sort_unstable();
        while self.free_at.len() > hw.cores as usize {
            let gone = self.free_at.remove(0).saturating_sub(now);
            if let Some(f) = self.free_at.first_mut() {
                *f = (*f).max(now) + gone;
            }
            self.free_at.sort_unstable();
        }
        while self.free_at.len() < hw.cores as usize {
            self.free_at.push(now);
        }
        self.hw = hw;
    }
    /// a number of `expect DEV svc _host metric NAME` at t (cpu_pm comes from the series: series.rs)
    pub fn metric(&self, name: &str, t: Ns) -> Option<i64> {
        let v = |x: u64| Some(x.min(i64::MAX as u64) as i64);
        match name {
            "cores" => v(self.hw.cores as u64),
            "speed_pm" => v(self.hw.speed_pm as u64),
            "running" => v(self.running(t)),
            "runq" => v(self.waiting(t)),
            "busy_ms" => v(self.done_at(t) / MS),
            "jobs" => v(self.jobs),
            "wait_ms" => v(self.wait_ns / MS),
            "ram_bytes" => v(self.ram()),
            "mem_bytes" => v(self.used().min(self.ram())),
            "swap_bytes" => v(self.over()),
            "pageins" => v(self.pageins),
            "oom_kills" => v(self.oom_kills),
            _ => None,
        }
    }
    /// the bytes it adds to a keyframe (keyframe.rs)
    pub(crate) fn footprint(&self) -> usize {
        core::mem::size_of::<Res>() + self.free_at.len() * 8 + self.queue.len() * 8 + self.svcs.values().map(|s| 64 + s.secs.len() * 48).sum::<usize>()
    }
}

/// a device's figures for the window (Engine::res_view)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResView {
    pub hw: Hw,
    /// its own figures, not its model's
    pub own: bool,
    pub running: u64,
    pub waiting: u64,
    /// held in RAM and in the swap
    pub mem: u64,
    pub swap_used: u64,
    pub pageins: u64,
    pub oom_kills: u64,
    /// a slow CPU fault acts on it, a memory hog holds this much
    pub slow_pm: u32,
    pub hog: u64,
}

/// a disk of a class holding what it holds (jfn_model's table)
pub(crate) fn disk_as(o: &mut Os, class: DiskClass, gb: u64) {
    let (latency, bps, iops) = class.spec();
    o.disk.latency = latency;
    o.disk.bps = bps;
    o.disk.iops = iops;
    if gb > 0 {
        o.disk.capacity = gb << 30;
    }
}

/// the disk of a device without hardware again (what it holds stays)
fn disk_plain(o: &mut Os) {
    let d = crate::disk::Disk::default();
    (o.disk.latency, o.disk.bps, o.disk.iops, o.disk.capacity) = (d.latency, d.bps, d.iops, d.capacity);
}

/// one job on what `o` has: its completion, a page-in on its disk included
fn run_job(o: &mut Os, svc: u32, now: Ns, cost: Ns, usl: Usl) -> Ns {
    let Some(r) = o.res.as_deref_mut() else {
        return now;
    };
    let (_, end) = r.submit(svc, now, cost, usl);
    let pages = r.pages(cost);
    if pages == 0 {
        return end;
    }
    o.disk.reads += 1;
    o.disk.bytes_read += pages * PAGE;
    // the job waits for them, its core does not
    let read = o.disk.access(now, pages * PAGE);
    end + (read - now)
}

impl OsApi<'_> {
    /// the device has hardware
    pub fn metered(&self) -> bool {
        self.eng.os(self.ni).is_some_and(|o| o.res.is_some())
    }
    /// CPU work of `cost` at a reference core, from now: when it is done (no hardware: now)
    pub fn cpu(&mut self, cost: Ns) -> Ns {
        self.cpu_usl(cost, Usl::default())
    }
    /// cpu() with a service's factor of the Universal Scalability Law
    pub fn cpu_usl(&mut self, cost: Ns, usl: Usl) -> Ns {
        let (now, id) = (self.eng.now, self.id);
        match self.eng.os_mut(self.ni) {
            Some(o) if o.res.is_some() && cost > 0 => run_job(o, id, now, cost, usl),
            _ => now,
        }
    }
    /// cpu(), but without hardware it still takes its time, as a plain timer
    pub fn cpu_or_timer(&mut self, cost: Ns) -> Ns {
        if self.metered() { self.cpu(cost) } else { self.eng.now + cost }
    }
    /// the next piece of a long job (`left` goes down by it): when it is done
    pub fn cpu_slice(&mut self, left: &mut Ns) -> Ns {
        self.cpu_slice_usl(left, Usl::default())
    }
    /// cpu_slice() with a service's factor of the Universal Scalability Law
    pub fn cpu_slice_usl(&mut self, left: &mut Ns, usl: Usl) -> Ns {
        let q = (*left).min(QUANTUM);
        *left -= q;
        self.cpu_usl(q, usl)
    }
    /// a place in the CPU's queue for work whose size shows when it runs: when its turn comes and
    /// on which core (no hardware: None)
    pub fn cpu_turn(&mut self) -> Option<(Ns, usize)> {
        let now = self.eng.now;
        self.eng.os_mut(self.ni)?.res.as_deref_mut().map(|r| r.turn(now))
    }
    /// what the work of a turn that came now on that core took
    pub fn cpu_charge(&mut self, core: usize, took: Ns) {
        let (now, id) = (self.eng.now, self.id);
        if let Some(r) = self.eng.os_mut(self.ni).and_then(|o| o.res.as_deref_mut()) {
            r.charge(id, core, now, took);
        }
    }
    /// a script's time per instruction on this device (no hardware: as it says)
    pub fn cpu_ps(&self, ps: u64) -> u64 {
        self.eng.os(self.ni).and_then(|o| o.res.as_deref()).map_or(ps, |r| r.scale_ps(ps))
    }
}

impl Engine {
    /// a device that starts running gets its hardware: its own, else its model's when the project
    /// simulates hardware (load.rs, Action::AddDevice)
    pub(crate) fn res_boot(&mut self, ni: usize, own: Option<&Hw>, model: &str) {
        let hw = match own {
            Some(h) => Some((h.clone(), true)),
            None if self.metered => hw_of(model).map(|h| (h, false)),
            None => None,
        };
        if let (Some((hw, own)), Some(o)) = (hw, self.os_mut(ni)) {
            disk_as(o, hw.disk, hw.disk_gb);
            o.res = Some(Box::new(Res::new(hw, own)));
        }
    }

    /// Action::Metered: "Hardware simulieren" of the project; devices with their own figures stay
    /// as they are, the others get their model's or lose theirs, without a restart
    pub(crate) fn res_metered(&mut self, on: bool) {
        self.metered = on;
        for ni in 0..self.nodes.len() {
            let Some(Some(n)) = self.nodes.get_mut(ni) else {
                continue;
            };
            let model = n.model.id;
            let Some(o) = n.os.as_mut() else {
                continue;
            };
            if o.res.as_ref().is_some_and(|r| r.own) {
                continue;
            }
            match (on, hw_of(model)) {
                (true, Some(hw)) if o.res.is_none() => {
                    disk_as(o, hw.disk, hw.disk_gb);
                    o.res = Some(Box::new(Res::new(hw, false)));
                    // what the services hold counts from now on
                    for s in o.svcs.iter().filter(|s| s.running) {
                        let b = s.svc.as_ref().map_or(0, |x| x.mem());
                        if let Some(r) = o.res.as_deref_mut() {
                            r.set_mem(s.id, b);
                        }
                    }
                }
                (false, _) if o.res.is_some() => {
                    o.res = None;
                    disk_plain(o);
                }
                _ => {}
            }
            self.res_check(ni);
        }
    }

    /// Action::Resize: other hardware for a device (None: its model's in a metered project, else
    /// none); with a downtime it reboots (a mark on the timeline: Fault::Resize), with none the
    /// running system takes it
    pub(crate) fn res_resize(&mut self, dev: u32, hw: Option<Hw>, downtime: Ns) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let own = hw.is_some();
        let model = self.node(dev).map_or("", |n| n.model.id);
        let hw = hw.or_else(|| if self.metered { hw_of(model) } else { None }).map(|h| h.sane());
        let powered = self.node(dev).is_some_and(|n| n.power);
        if downtime > 0 && powered {
            self.fault_start(crate::fault::Fault::Resize { dev }, Some(downtime))?;
        }
        let now = self.now;
        if let Some(o) = self.os_mut(ni) {
            match hw {
                Some(hw) => {
                    disk_as(o, hw.disk, hw.disk_gb);
                    match o.res.as_deref_mut() {
                        Some(r) => {
                            r.resize(now, hw);
                            r.own = own;
                        }
                        None => {
                            o.res = Some(Box::new(Res::new(hw, own)));
                            // what the services hold counts from now on
                            for s in o.svcs.iter().filter(|s| s.running) {
                                let b = s.svc.as_ref().map_or(0, |x| x.mem());
                                if let Some(r) = o.res.as_deref_mut() {
                                    r.set_mem(s.id, b);
                                }
                            }
                        }
                    }
                }
                None => {
                    o.res = None;
                    disk_plain(o);
                }
            }
        }
        self.res_check(ni);
        Ok(())
    }

    /// the OS's look at a service after its call (svc_call): what it holds now
    pub(crate) fn res_after_call(&mut self, ni: usize, id: u32, bytes: u64) {
        let Some(r) = self.os_mut(ni).and_then(|o| o.res.as_deref_mut()) else {
            return;
        };
        if r.svc_bytes(id) != bytes {
            r.set_mem(id, bytes);
            self.res_check(ni);
        }
    }

    /// more is held than fits: the kill runs after the current call
    pub(crate) fn res_check(&mut self, ni: usize) {
        let due = match self.os_mut(ni).and_then(|o| o.res.as_deref_mut()) {
            Some(r) if r.exhausted() && !r.oom_due => {
                r.oom_due = true;
                true
            }
            _ => false,
        };
        if due {
            self.os_push(ni, Job::Oom);
        }
    }

    /// Job::Oom: the largest services go until what is held fits (or none is left to kill)
    pub(crate) fn res_oom(&mut self, ni: usize) {
        if let Some(r) = self.os_mut(ni).and_then(|o| o.res.as_deref_mut()) {
            r.oom_due = false;
        }
        loop {
            let Some(o) = self.os(ni) else {
                return;
            };
            let Some(r) = o.res.as_deref() else {
                return;
            };
            if !r.exhausted() {
                return;
            }
            let Some(id) = r.largest() else {
                return;
            };
            let bytes = r.svc_bytes(id);
            let name = o.svcs.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_default();
            let dev = self.dev_of(ni);
            self.note(dev, None, Note::OomKill { svc: name.clone(), bytes });
            self.os_log(ni, 3, "kernel", &format!("Speicher voll: Dienst {} beendet ({}, Status {})", name, crate::disk::size_text(bytes), OOM_EXIT), None);
            if let Some(r) = self.os_mut(ni).and_then(|o| o.res.as_deref_mut()) {
                r.oom_kills += 1;
                r.forget(id);
            }
            self.svc_stop(ni, id);
            if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                s.exit = Some(OOM_EXIT);
            }
            self.units_halted(ni, id, OOM_EXIT);
        }
    }

    /// a device's hardware now (None: it has none)
    pub fn res_view(&self, dev: u32) -> Option<ResView> {
        let r = self.os(self.node_index(dev)?)?.res.as_deref()?;
        let now = self.now;
        Some(ResView { hw: r.hw.clone(), own: r.own, running: r.running(now), waiting: r.waiting(now), mem: r.used().min(r.ram()), swap_used: r.over(), pageins: r.pageins, oom_kills: r.oom_kills, slow_pm: r.slow_pm, hog: r.hog })
    }

    /// the hardware of a device (top, the tests)
    pub fn res_of(&self, dev: u32) -> Option<&Res> {
        self.os(self.node_index(dev)?)?.res.as_deref()
    }
}

/// a device's figures as a status line: "8 Kerne, 32 GiB RAM, NVMe-SSD 960 GiB"
pub fn status_msg(r: &Res) -> Msg {
    r.hw.summary_msg()
}
