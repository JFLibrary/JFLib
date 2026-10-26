// kernel -- the event engine: virtual time, the event queue and the device / cable tables
//
//   Engine::new(seed)                      empty; Engine::from_project in load.rs
//   now() / pending() / next_time()
//   step() -> Option<Ns>                   runs one event
//   run_until(t, budget) -> RunOut         every event up to t (then the clock stands at t)
//   run_idle(budget) -> RunOut             until nothing is left
//   set_timer / cancel_timer (crate)       per node and slot; a newer setting cancels the older
//                                          (slots: ip::SLOT_ARP, ip::SLOT_REASM; os::SLOT_OS
//                                          and up: the inbox, the resolver, tcp::SLOT_TCP,
//                                          services; stp::SLOT_STP the spanning tree's tick,
//                                          nat::SLOT_NAT; ip6::SLOT_V6_KICK ..= SLOT_V6_LAST
//                                          for IPv6, nd.rs; wlan::SLOT_WLAN_STA and
//                                          SLOT_WLAN_AP for WLAN)
//
// Events are plain data in a slab and run in (time, sequence) order: two events at the same
// time run in the order they were scheduled.  Nothing here reads a clock.  Before an event,
// and where run_until leaves the clock, the time series take the samples that are due
// (series.rs); they only read.

use alloc::collections::{BTreeMap, BinaryHeap};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Reverse;

use crate::link::LinkRt;
use crate::node::Node;
use crate::trace::Trace;

/// virtual time in nanoseconds since the start of the simulation
pub type Ns = u64;
pub const US: Ns = 1_000;
pub const MS: Ns = 1_000_000;
pub const SEC: Ns = 1_000_000_000;

/// a frame as it travels: shared, never changed
pub type Frame = Arc<[u8]>;

/// one scheduled event
#[derive(Clone, Debug)]
pub(crate) enum Ev {
    /// a frame reaches the far end of a cable (`dir` 0: from end 0 to end 1)
    Arrive { link: usize, epoch: u32, dir: u8, frame: Frame, hop: u64 },
    /// the sender finished serialising: the next queued frame may go
    TxDone { link: usize, epoch: u32, dir: u8 },
    /// a node timer; stale when the slot's generation moved on
    Timer { node: usize, slot: u32, epoch: u32 },
    /// the next step of a job (ping, traceroute); stale when the job's epoch moved on
    Job { job: u32, epoch: u32 },
    /// the console of a node goes on (a typed-ahead line, a timer of a command); stale when
    /// the console's epoch moved on
    Cli { node: usize, epoch: u32 },
    /// a fault with a duration heals (fault.rs); nothing when it was healed before
    Heal { id: u32 },
}

/// how a run ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOut {
    /// the target time was reached (the clock stands there)
    Reached,
    /// the event budget ran out first
    Budget,
    /// no events left
    Idle,
}

/// the simulator
#[derive(Clone)]
pub struct Engine {
    pub(crate) now: Ns,
    seq: u64,
    heap: BinaryHeap<Reverse<(Ns, u64, u32)>>,
    slab: Vec<Option<Ev>>,
    free: Vec<u32>,
    pub(crate) nodes: Vec<Option<Node>>,
    pub(crate) links: Vec<Option<LinkRt>>,
    pub(crate) dev_idx: BTreeMap<u32, usize>,
    pub(crate) link_idx: BTreeMap<u32, usize>,
    pub trace: Trace,
    pub(crate) seed: u64,
    /// copper links with the wrong cable stay down (no auto-MDI-X)
    pub strict_mdix: bool,
    events_done: u64,
    pub(crate) jobs: BTreeMap<crate::jobs::JobId, crate::jobs::Job>,
    pub(crate) next_job: crate::jobs::JobId,
    /// the command lines' shared part: reports, command trees, debugging
    pub(crate) cli: crate::cli::EngineCli,
    /// injected faults: their marks and what they do to cables, devices and disks (fault.rs)
    pub(crate) faults: crate::fault::Faults,
    /// the time series of the services and cables (series.rs)
    pub(crate) series: crate::series::Sampler,
    /// WLAN: positions, settings, stations and associations (wlan.rs)
    pub(crate) wlan: crate::wlan::WlanState,
    /// the cables' lengths and the ones too long or with optics that do not fit (phys.rs)
    pub(crate) phys: crate::phys::PhysState,
}

impl Engine {
    pub fn new(seed: u64) -> Engine {
        Engine {
            now: 0,
            seq: 0,
            heap: BinaryHeap::new(),
            slab: Vec::new(),
            free: Vec::new(),
            nodes: Vec::new(),
            links: Vec::new(),
            dev_idx: BTreeMap::new(),
            link_idx: BTreeMap::new(),
            trace: Trace::new(),
            seed,
            strict_mdix: false,
            events_done: 0,
            jobs: BTreeMap::new(),
            next_job: 0,
            cli: crate::cli::EngineCli::default(),
            faults: crate::fault::Faults::default(),
            series: crate::series::Sampler::default(),
            wlan: crate::wlan::WlanState::default(),
            phys: crate::phys::PhysState::default(),
        }
    }
    /// the virtual time
    pub fn now(&self) -> Ns {
        self.now
    }
    pub fn seed(&self) -> u64 {
        self.seed
    }
    /// scheduled events (cancelled timers included until their time comes)
    pub fn pending(&self) -> usize {
        self.heap.len()
    }
    /// events run since the start
    pub fn events_done(&self) -> u64 {
        self.events_done
    }
    /// the time of the next event
    pub fn next_time(&self) -> Option<Ns> {
        self.heap.peek().map(|r| r.0.0)
    }

    pub(crate) fn schedule(&mut self, at: Ns, ev: Ev) {
        let at = at.max(self.now);
        let slot = match self.free.pop() {
            Some(i) => {
                self.slab[i as usize] = Some(ev);
                i
            }
            None => {
                self.slab.push(Some(ev));
                (self.slab.len() - 1) as u32
            }
        };
        self.seq += 1;
        self.heap.push(Reverse((at, self.seq, slot)));
    }

    /// runs the next event; its time, or None when nothing is scheduled
    pub fn step(&mut self) -> Option<Ns> {
        let Reverse((t, _, slot)) = self.heap.pop()?;
        if t >= self.series.next {
            // the samples of the time series that are due first; they only read (series.rs)
            self.sample_until(t);
        }
        let ev = self.slab[slot as usize].take();
        self.free.push(slot);
        self.now = t;
        self.events_done += 1;
        if let Some(ev) = ev {
            self.trace.begin_batch(t);
            self.dispatch(ev);
            self.trace.end_batch();
        }
        Some(t)
    }

    /// runs every event up to time `t` (at most `budget`); the clock then stands at `t`
    pub fn run_until(&mut self, t: Ns, budget: usize) -> RunOut {
        let mut n = 0;
        while let Some(next) = self.next_time() {
            if next > t {
                break;
            }
            if n == budget {
                return RunOut::Budget;
            }
            self.step();
            n += 1;
        }
        if t > self.now {
            self.now = t;
        }
        if self.now >= self.series.next {
            self.sample_until(self.now);
        }
        RunOut::Reached
    }

    /// runs until no event is left (at most `budget` events)
    pub fn run_idle(&mut self, budget: usize) -> RunOut {
        for _ in 0..budget {
            if self.step().is_none() {
                return RunOut::Idle;
            }
        }
        if self.heap.is_empty() { RunOut::Idle } else { RunOut::Budget }
    }

    fn dispatch(&mut self, ev: Ev) {
        match ev {
            Ev::Arrive { link, epoch, dir, frame, hop } => self.arrive(link, epoch, dir, frame, hop),
            Ev::TxDone { link, epoch, dir } => self.tx_done(link, epoch, dir),
            Ev::Timer { node, slot, epoch } => self.fire_timer(node, slot, epoch),
            Ev::Job { job, epoch } => self.job_tick(job, epoch),
            Ev::Cli { node, epoch } => self.cli_wake(node, epoch),
            Ev::Heal { id } => self.fault_due(id),
        }
    }

    // ---------------------------------------------------------------- timers

    /// (re)arms timer `slot` of a node to fire at `at`; an earlier setting no longer fires
    pub(crate) fn set_timer(&mut self, node: usize, slot: u32, at: Ns) {
        let Some(Some(n)) = self.nodes.get_mut(node) else {
            return;
        };
        let s = slot as usize;
        if n.timers.len() <= s {
            n.timers.resize(s + 1, 0);
        }
        n.timers[s] = n.timers[s].wrapping_add(1);
        let epoch = n.timers[s];
        self.schedule(at, Ev::Timer { node, slot, epoch });
    }

    /// stops timer `slot` of a node
    pub(crate) fn cancel_timer(&mut self, node: usize, slot: u32) {
        if let Some(Some(n)) = self.nodes.get_mut(node)
            && let Some(g) = n.timers.get_mut(slot as usize)
        {
            *g = g.wrapping_add(1);
        }
    }

    fn fire_timer(&mut self, node: usize, slot: u32, epoch: u32) {
        let live = match self.nodes.get(node) {
            Some(Some(n)) => n.power && n.timers.get(slot as usize) == Some(&epoch),
            _ => false,
        };
        if live {
            self.on_timer(node, slot);
        }
    }

    /// a node timer fired
    fn on_timer(&mut self, node: usize, slot: u32) {
        if slot == crate::stp::SLOT_STP {
            self.stp_tick(node);
        } else if slot == crate::wlan::SLOT_WLAN_STA || slot == crate::wlan::SLOT_WLAN_AP {
            self.wlan_timer(node, slot);
        } else if slot >= crate::os::SLOT_OS {
            self.os_timer(node, slot);
        } else {
            self.l3_timer(node, slot);
        }
    }

    // ---------------------------------------------------------------- lookups

    /// the slot of a device
    pub fn node_index(&self, dev: u32) -> Option<usize> {
        self.dev_idx.get(&dev).copied()
    }
    pub fn node(&self, dev: u32) -> Option<&Node> {
        self.nodes.get(self.node_index(dev)?)?.as_ref()
    }
    #[allow(dead_code)]
    pub(crate) fn node_mut(&mut self, dev: u32) -> Option<&mut Node> {
        let i = self.node_index(dev)?;
        self.nodes.get_mut(i)?.as_mut()
    }
    /// every device id, ascending
    pub fn devices(&self) -> impl Iterator<Item = u32> + '_ {
        self.dev_idx.keys().copied()
    }
    /// the index of a port by any spelling ("gi1/0/1")
    pub fn port_index(&self, dev: u32, port: &str) -> Option<u16> {
        let n = self.node(dev)?;
        n.model.port(port).map(|p| p.index)
    }
    pub fn link_rt(&self, id: u32) -> Option<&LinkRt> {
        self.links.get(*self.link_idx.get(&id)?)?.as_ref()
    }
}

impl Engine {
    /// the cable with this project id carries traffic now (link_up of its slot)
    pub fn link_idx_up(&self, link: u32) -> bool {
        self.link_idx.get(&link).is_some_and(|&li| self.link_up(li))
    }
}
