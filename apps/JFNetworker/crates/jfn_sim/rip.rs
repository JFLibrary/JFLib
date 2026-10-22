// rip -- RIP on IOS-like routers and L3 switches: version 2 after RFC 2453, version 1 after RFC
// 1058 where an interface is set to it; a service of the device's OS ("rip") with its
// configuration ("router rip"), its table, the routes it gives the RIB, and what the command
// lines show of it
//
//   RipCfg                     version, networks, passive interfaces, auto-summary, default route,
//                              redistribution, timers, distance, maximum-paths; per interface the
//                              versions it sends and receives ("ip rip send|receive version")
//   RipSet / Action::Rip       changes of the configuration (Engine::rip_apply); Process(false)
//                              is "no router rip" (the interface settings stay), Clear is
//                              "clear ip route *" (the learned routes are forgotten and asked
//                              for again)
//   Rip                        the service: table(), ifaces(), sources(), next_update(), age(),
//                              counters `n`; metrics routes, learned, neighbours (gauges) and the
//                              counted messages
//   Engine::rip(dev)           the service of a device (None before the first RIP command)
//   Engine::rip_kick(ni)       interfaces, addresses or routes of the device may have changed
//                              (link.rs, Engine::apply): the service looks again at once
//   classful(a) / classful_len(a)   the class A, B or C network of an address
//
// Metrics (RFC 2453 3.5, 3.9.2): a directly connected network has the cost of its interface, 1;
// a received metric gets that 1 added; 16 is unreachable.  So a router shows its neighbour's LAN
// as [120/2] (as FRR does; IOS adds the 1 when it sends and shows 1 -- both put the same metrics
// on the wire, only a route that arrives with 15 is unreachable here and not there).
//
// Timers ("timers basic U I H F", default 30 180 180 300 s): updates every U seconds, each time
// U plus or minus a random sixth of it (RFC 2453 3.8: 30 s +/- 5 s, from the device's own random
// numbers); a route nobody refreshed for I seconds times out: metric 16, advertised so until F
// seconds after its last update, then removed (F - I = 120 s is the RFC's garbage collection; a
// route its next hop reports with 16 is removed F - I seconds after that report).  The holddown
// is what IOS-like devices add (RFC 2453 has none): for H seconds after a route became
// unreachable -- timed out, poisoned by its next hop, or its interface went down -- offers from
// other routers with a metric no better than its last one are ignored, the router it came from
// may bring it back; a removed route is new again.  So a route lost in a ring comes back over the
// other way only after the garbage collection.  Triggered updates (3.10.1) carry the changed
// routes at once, then wait a random 1 to 5 s for the next ones; one that a regular update would
// overtake is left out.
//
// Equal-cost paths: up to maximum-paths (default 4) routers that report the same best metric
// each give the RIB a route; forwarding picks one per destination address (ip.rs).
//
// Output: version 2 to 224.0.0.9, version 1 to 255.255.255.255, both with TTL 1, from port 520
// and the interface's primary address; answers to requests go to the asker's address and port
// (unicast, so an IOS-like device may lose the first one to ARP).  At most 25 entries per
// message.  Split horizon with poisoned reverse (3.4.3): what was learned over an interface goes
// back on it with metric 16; the interface's own networks are not advertised on it.  Version 1
// carries no masks: subnets of the outgoing interface's major network go out only when they have
// that interface's mask, other major networks as their classful network; version 2 sends
// prefixes and summarises at classful boundaries only with auto-summary (off by default, as on
// IOS 15).  When RIP starts, and when an interface comes up, it asks for the whole table and
// sends its own on that interface.
//
// Input (3.9): responses from port 520, from a neighbour on the receiving interface's subnets,
// not from an own address; version 1 with a must-be-zero field set is ignored, and so is an
// authenticated version 2 message (no authentication is configured: 5.2); an entry needs a
// unicast destination and a metric of 1 to 16.  A request for the whole table is answered like
// an update of that interface (split horizon included), one for entries with their metrics;
// passive interfaces answer nothing.  Version 1 masks are inferred: within the major network of
// the receiving interface its mask (a host route where host bits are set), else the classful
// mask.
//
// Interfaces: "network A.B.C.D" (stored classful) enables RIP on every interface whose primary
// address lies in that network; passive interfaces listen but send nothing.  RIP needs routing
// ("ip routing" on an L3 switch).  Power off forgets the table; the configuration stays.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use jfn_model::catalog::F_ROUTER;
use jfn_model::msg;
use jfn_model::msg::{dur, phrase, Msg};
use netpkt::eth::ET_IPV4;
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::rip as wire;
use netpkt::rip::Rte;
use netpkt::udp::{UdpHdr, PORT_RIP};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::action::{ActErr, Action};
use crate::ip::{Origin as IpOrigin, Route, RouteSrc};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Ctl, Dgram, Job, Metric, OsApi, Service, SvcEvent, SLOT_SVC, SVC_TIMERS};
use crate::series::MetricKind;
use crate::trace::Note;

/// the kind and the name of the service
pub const KIND: &str = "rip";
/// the metric of an unreachable destination
pub const INFINITY: u8 = 16;
/// what an interface costs: the metric of a directly connected network, added to every metric
/// received (RFC 2453 3.5, 3.9.2)
pub const COST: u8 = 1;
/// version bits: what an interface sends or receives
pub const V1: u8 = 1;
pub const V2: u8 = 2;
/// the administrative distance of RIP routes
pub const DISTANCE: u8 = 120;
/// equal-cost paths by default
pub const MAX_PATHS: u8 = 4;

// the service's timer keys
const T_UPDATE: u32 = 0;
const T_HOLD: u32 = 1;
const T_ROUTES: u32 = 2;
const T_LOOK: u32 = 3;

/// the length of the classful network of an address (A /8, B /16, C /24; D and E 32)
pub fn classful_len(a: Ipv4) -> u8 {
    match a.0[0] {
        0..=127 => 8,
        128..=191 => 16,
        192..=223 => 24,
        _ => 32,
    }
}

/// the classful network of an address
pub fn classful(a: Ipv4) -> Prefix4 {
    Prefix4 { addr: a, len: classful_len(a) }.trunc()
}

/// a destination RIP may carry: unicast (net 0 only as the default route, no 127/8)
fn usable(p: Prefix4) -> bool {
    let a = p.addr;
    if p.len == 0 {
        return a.is_unspecified();
    }
    !(a.0[0] == 0 || a.is_loopback() || a.is_multicast() || a.0[0] >= 240)
}

// ---------------------------------------------------------------- configuration

/// "timers basic UPDATE INVALID HOLDDOWN FLUSH" in seconds
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RipTimers {
    pub update: u32,
    pub invalid: u32,
    pub holddown: u32,
    pub flush: u32,
}

impl Default for RipTimers {
    fn default() -> RipTimers {
        RipTimers { update: 30, invalid: 180, holddown: 180, flush: 300 }
    }
}

impl RipTimers {
    /// how long an unreachable route stays (flush - invalid; RFC 2453 3.8: 120 s)
    pub fn gc(&self) -> Ns {
        self.flush.saturating_sub(self.invalid) as Ns * SEC
    }
}

/// what "redistribute" takes in
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Redist {
    Connected,
    Static,
}

impl Redist {
    pub fn word(self) -> &'static str {
        match self {
            Redist::Connected => "connected",
            Redist::Static => "static",
        }
    }
}

/// the configuration of RIP on a device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RipCfg {
    /// "router rip" is there
    pub on: bool,
    /// "version 1|2"; None: send 2, receive 1 and 2
    pub version: Option<u8>,
    /// classful networks of "network", sorted
    pub networks: Vec<Ipv4>,
    /// "passive-interface default"; `passive` then lists the exceptions
    pub passive_default: bool,
    pub passive: Vec<String>,
    pub auto_summary: bool,
    /// "default-information originate"
    pub default_originate: bool,
    /// "redistribute connected|static [metric N]"
    pub redistribute: Vec<(Redist, Option<u8>)>,
    pub timers: RipTimers,
    pub distance: u8,
    pub max_paths: u8,
    /// "ip rip send version" / "ip rip receive version" by interface name (V1 | V2)
    pub send: BTreeMap<String, u8>,
    pub recv: BTreeMap<String, u8>,
}

impl Default for RipCfg {
    fn default() -> RipCfg {
        RipCfg {
            on: false,
            version: None,
            networks: Vec::new(),
            passive_default: false,
            passive: Vec::new(),
            auto_summary: false,
            default_originate: false,
            redistribute: Vec::new(),
            timers: RipTimers::default(),
            distance: DISTANCE,
            max_paths: MAX_PATHS,
            send: BTreeMap::new(),
            recv: BTreeMap::new(),
        }
    }
}

impl RipCfg {
    /// the versions an interface sends
    pub fn sends(&self, ifname: &str) -> u8 {
        match self.send.get(ifname) {
            Some(v) => *v,
            None => match self.version {
                Some(1) => V1,
                _ => V2,
            },
        }
    }
    /// the versions an interface takes
    pub fn receives(&self, ifname: &str) -> u8 {
        match self.recv.get(ifname) {
            Some(v) => *v,
            None => match self.version {
                Some(1) => V1,
                Some(_) => V2,
                None => V1 | V2,
            },
        }
    }
    pub fn is_passive(&self, ifname: &str) -> bool {
        self.passive_default != self.passive.iter().any(|p| p.eq_ignore_ascii_case(ifname))
    }
    /// an address lies in one of the networks
    pub fn covers(&self, a: Ipv4) -> bool {
        self.networks.contains(&classful(a).addr)
    }
    /// "no router rip": the process settings back to their defaults; the interfaces keep theirs
    fn reset(&mut self) {
        let (send, recv) = (core::mem::take(&mut self.send), core::mem::take(&mut self.recv));
        *self = RipCfg { send, recv, ..RipCfg::default() };
    }
}

/// a change of the configuration (Action::Rip)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RipSet {
    /// "router rip" (true) / "no router rip"
    Process(bool),
    /// "version 1|2" (None: "no version")
    Version(Option<u8>),
    Network { net: Ipv4, add: bool },
    /// "passive-interface IF" (Some) or "passive-interface default" (None), `on: false` for the
    /// no-form
    Passive { iface: Option<String>, on: bool },
    AutoSummary(bool),
    DefaultOriginate(bool),
    Redistribute { what: Redist, metric: Option<u8>, add: bool },
    /// None: the defaults
    Timers(Option<RipTimers>),
    Distance(Option<u8>),
    MaxPaths(Option<u8>),
    /// "ip rip send version" (`receive: false`) or "ip rip receive version" of an interface
    /// (None: as the process says)
    IfVersions { iface: String, receive: bool, versions: Option<u8> },
    /// "clear ip route *"
    Clear,
}

// ---------------------------------------------------------------- the reasons of notes

/// what kind of update went out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RipOut {
    /// the periodic update (and the whole table when an interface comes up)
    Regular,
    /// the changed routes, at once (RFC 2453 3.10.1)
    Triggered,
    /// to the asker of a request
    Answer,
}

/// why a message was ignored
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RipDrop {
    /// the interface does not take this version
    Version,
    /// a response not from port 520
    Port,
    /// the sender is on none of the interface's subnets
    Neighbour,
    /// from an address of this device
    Own,
    /// authenticated, but no authentication is configured
    Auth,
    /// RIP does not run on the interface
    Disabled,
    /// version 1 with a must-be-zero field set
    Mbz,
    /// neither request nor response
    Command,
    /// a request on a passive interface
    Passive,
    /// a request in a version the interface does not send (RFC 2453 4.6)
    NoAnswer,
    /// shorter than a header
    Short,
}

impl RipDrop {
    /// the reason in German (a phrase of the catalogue)
    pub fn text(self) -> &'static str {
        match self {
            RipDrop::Version => "diese Version nimmt das Interface nicht an",
            RipDrop::Port => "sie kommt nicht vom RIP-Port 520",
            RipDrop::Neighbour => "der Absender liegt in keinem Netz des Interfaces",
            RipDrop::Own => "sie kommt von einer eigenen Adresse",
            RipDrop::Auth => "sie ist authentifiziert, hier ist keine Authentifizierung eingerichtet",
            RipDrop::Disabled => "RIP läuft auf diesem Interface nicht",
            RipDrop::Mbz => "Version 1 mit einem Feld, das null sein muss",
            RipDrop::Command => "unbekanntes Kommando",
            RipDrop::Passive => "das Interface ist passiv",
            RipDrop::NoAnswer => "das Interface sendet diese Version nicht",
            RipDrop::Short => "unlesbare Nachricht",
        }
    }
    /// the reason for the console
    pub fn en(self) -> &'static str {
        match self {
            RipDrop::Version => "the interface does not take this version",
            RipDrop::Port => "not from port 520",
            RipDrop::Neighbour => "the sender is on none of the interface's subnets",
            RipDrop::Own => "from an address of this device",
            RipDrop::Auth => "authenticated, but no authentication is set up here",
            RipDrop::Disabled => "RIP does not run on this interface",
            RipDrop::Mbz => "version 1 with a must-be-zero field set",
            RipDrop::Command => "unknown command",
            RipDrop::Passive => "the interface is passive",
            RipDrop::NoAnswer => "the interface does not send this version",
            RipDrop::Short => "unreadable message",
        }
    }
}

/// why a route or one of its paths was lost
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RipLoss {
    /// the router it came from reports metric 16
    Poisoned,
    /// no update for this many seconds
    Timeout(u32),
    /// its interface went down
    Iface,
    /// the router it came from reports a worse metric (other paths are better)
    Worse,
}

impl RipLoss {
    pub fn msg(self) -> Msg {
        match self {
            RipLoss::Poisoned => phrase("der Nachbar meldet Metrik 16"),
            RipLoss::Timeout(s) => msg!("{} s ohne Update", s),
            RipLoss::Iface => phrase("das Interface ist ausgefallen"),
            RipLoss::Worse => phrase("der Nachbar meldet eine schlechtere Metrik"),
        }
    }
    pub fn en(self) -> String {
        match self {
            RipLoss::Poisoned => String::from("the neighbour reports metric 16"),
            RipLoss::Timeout(s) => format!("no update for {} s", s),
            RipLoss::Iface => String::from("the interface went down"),
            RipLoss::Worse => String::from("the neighbour reports a worse metric"),
        }
    }
}

/// why the device advertises a network of its own
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RipHow {
    /// an interface RIP runs on
    Connected,
    /// "redistribute connected"
    RedistConnected,
    /// "redistribute static"
    RedistStatic,
    /// "default-information originate"
    Default,
}

impl RipHow {
    pub fn text(self) -> &'static str {
        match self {
            RipHow::Connected => "direkt angeschlossen",
            RipHow::RedistConnected => "angeschlossen, über redistribute connected",
            RipHow::RedistStatic => "statische Route, über redistribute static",
            RipHow::Default => "Standardroute, über default-information originate",
        }
    }
    pub fn en(self) -> &'static str {
        match self {
            RipHow::Connected => "directly connected",
            RipHow::RedistConnected => "connected, redistributed",
            RipHow::RedistStatic => "static, redistributed",
            RipHow::Default => "default route, originated",
        }
    }
}

// ---------------------------------------------------------------- the table

/// where a route of the table comes from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// a network of interface n, which RIP runs on
    Connected(u16),
    Redist(Redist),
    /// "default-information originate"
    Default,
    /// heard from neighbours
    Learned,
}

/// one way to a learned destination
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Path {
    /// where packets go: the sender, or the next hop it named
    pub via: Ipv4,
    /// the router whose update it is
    pub from: Ipv4,
    pub iface: u16,
    /// the last update that confirmed it
    pub heard: Ns,
}

/// a destination in the table
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub origin: Origin,
    /// 1 to 15; INFINITY while it is unreachable (until gc_at)
    pub metric: u8,
    pub tag: u16,
    /// the equal-cost paths of a learned route (none while unreachable)
    pub paths: Vec<Path>,
    /// changed since the last update went out (the route change flag of RFC 2453 3.10.1)
    pub changed: bool,
    /// unreachable: removed at gc_at, held down until hold_until
    pub gc_at: Option<Ns>,
    pub hold_until: Option<Ns>,
    /// the metric it had before it became unreachable, and the routers it came from
    pub held_metric: u8,
    pub held_from: Vec<Ipv4>,
}

impl Entry {
    fn local(origin: Origin, metric: u8) -> Entry {
        Entry { origin, metric, tag: 0, paths: Vec::new(), changed: true, gc_at: None, hold_until: None, held_metric: metric, held_from: Vec::new() }
    }
    fn learned(metric: u8, tag: u16, p: Path) -> Entry {
        Entry { origin: Origin::Learned, metric, tag, paths: alloc::vec![p], changed: true, gc_at: None, hold_until: None, held_metric: metric, held_from: Vec::new() }
    }
    pub fn reachable(&self) -> bool {
        self.metric < INFINITY
    }
}

/// an interface RIP runs on
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RipIf {
    pub name: String,
    /// its primary address
    pub addr: Prefix4,
    pub up: bool,
    pub passive: bool,
    pub loopback: bool,
    /// V1 | V2
    pub send: u8,
    pub recv: u8,
}

impl RipIf {
    /// it sends updates
    pub fn speaks(&self) -> bool {
        self.up && !self.passive && !self.loopback
    }
}

/// what RIP counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub updates_sent: u64,
    pub triggered: u64,
    pub answers: u64,
    pub requests_sent: u64,
    pub updates_rcvd: u64,
    pub requests_rcvd: u64,
    pub ignored: u64,
    pub bad_entries: u64,
    pub timeouts: u64,
    pub flushed: u64,
}

// ---------------------------------------------------------------- the service

/// RIP on a device
#[derive(Clone, Debug)]
pub struct Rip {
    pub cfg: RipCfg,
    table: BTreeMap<Prefix4, Entry>,
    ifs: BTreeMap<u16, RipIf>,
    /// routers heard from and when
    sources: BTreeMap<Ipv4, Ns>,
    next_update: Option<Ns>,
    /// the hold after a triggered update, and changes waiting for its end
    hold: Option<Ns>,
    pending: bool,
    /// the timers in effect
    used: RipTimers,
    /// interfaces that got the whole table in this event (no triggered update for them)
    fresh: Vec<u16>,
    /// the route timer as armed
    armed: Option<Ns>,
    running: bool,
    pub n: Counters,
}

impl Default for Rip {
    fn default() -> Rip {
        Rip::new()
    }
}

impl Rip {
    pub fn new() -> Rip {
        Rip {
            cfg: RipCfg::default(),
            table: BTreeMap::new(),
            ifs: BTreeMap::new(),
            sources: BTreeMap::new(),
            next_update: None,
            hold: None,
            pending: false,
            used: RipTimers::default(),
            fresh: Vec::new(),
            armed: None,
            running: false,
            n: Counters::default(),
        }
    }
    /// the table by destination
    pub fn table(&self) -> &BTreeMap<Prefix4, Entry> {
        &self.table
    }
    /// the interfaces RIP runs on, by IPv4 interface
    pub fn ifaces(&self) -> &BTreeMap<u16, RipIf> {
        &self.ifs
    }
    /// the routers heard from, with the time of their last update
    pub fn sources(&self) -> &BTreeMap<Ipv4, Ns> {
        &self.sources
    }
    pub fn next_update(&self) -> Option<Ns> {
        self.next_update
    }
    pub fn running(&self) -> bool {
        self.running
    }
    /// how long ago the path over `via` to `prefix` was confirmed
    pub fn age(&self, prefix: Prefix4, via: Ipv4, now: Ns) -> Option<Ns> {
        let e = self.table.get(&prefix)?;
        e.paths.iter().find(|p| p.via == via).map(|p| now.saturating_sub(p.heard))
    }
    /// the version it speaks by default
    pub fn version(&self) -> u8 {
        self.cfg.version.unwrap_or(2)
    }

    /// a random time around the update period (+/- a sixth)
    fn period(&self, os: &mut OsApi) -> Ns {
        let u = self.used.update.max(1) as Ns * 1000;
        let span = u / 3;
        (u - span / 2 + os.rand() as Ns % (span + 1)) * MS
    }

    // ---------------------------------------------------------------- interfaces, own routes

    /// looks at the interfaces and the own routes again: interfaces that went down lose their
    /// paths, ones that start to speak (up, not passive, or in new versions) are greeted with a
    /// request and the whole table, own networks come and go
    fn sync(&mut self, os: &mut OsApi) {
        if !self.running {
            return;
        }
        let ni = os.ni;
        let forwarding = os.eng.stack(ni).is_some_and(|s| s.forwarding);
        let mut want: BTreeMap<u16, RipIf> = BTreeMap::new();
        if self.cfg.on && forwarding {
            for (i, f) in os.ifaces().iter().enumerate() {
                let Some(p) = f.addrs.first().copied() else {
                    continue;
                };
                if f.hidden || !self.cfg.covers(p.addr) {
                    continue;
                }
                let up = os.eng.iface_up(ni, i as u16);
                want.insert(i as u16, RipIf { name: f.name.clone(), addr: p, up, passive: self.cfg.is_passive(&f.name), loopback: f.is_loopback(), send: self.cfg.sends(&f.name), recv: self.cfg.receives(&f.name) });
            }
        }
        let old = core::mem::replace(&mut self.ifs, want);
        for (i, o) in &old {
            let still = self.ifs.get(i).is_some_and(|n| n.up && n.addr == o.addr);
            if o.up && !still {
                self.iface_lost(os, *i);
                // the routers behind it are no sources any more
                self.sources.retain(|a, _| !o.addr.contains(*a));
            }
        }
        // nor are routers not heard for the flush time
        let (now, flush) = (os.now(), self.used.flush as Ns * SEC);
        self.sources.retain(|_, t| *t + flush > now);
        // greeted: interfaces that speak now and did not before (or in other versions)
        let came: Vec<u16> = self.ifs.iter().filter(|(i, n)| n.speaks() && !old.get(i).is_some_and(|o| o.speaks() && o.addr == n.addr && o.send == n.send)).map(|(i, _)| *i).collect();
        // the group of version 2 on every interface that takes it
        if let Some(st) = os.eng.stack_mut(ni) {
            st.groups.retain(|g| g.1 != wire::GROUP);
            for (i, r) in &self.ifs {
                if r.recv & V2 != 0 {
                    st.groups.push((*i, wire::GROUP));
                }
            }
        }
        self.sync_local(os);
        // fewer equal-cost paths allowed
        let max = self.cfg.max_paths.max(1) as usize;
        for e in self.table.values_mut() {
            e.paths.truncate(max);
        }
        // new timers: the next update counts from now
        if self.cfg.timers != self.used {
            self.used = self.cfg.timers;
            let at = os.now() + self.period(os);
            self.next_update = Some(at);
            os.timer(T_UPDATE, at);
        }
        for i in came {
            self.greet(os, i);
        }
    }

    /// the paths over an interface that went down are gone
    fn iface_lost(&mut self, os: &mut OsApi, i: u16) {
        let keys: Vec<Prefix4> = self.table.iter().filter(|(_, e)| e.origin == Origin::Learned && e.paths.iter().any(|p| p.iface == i)).map(|(k, _)| *k).collect();
        for k in keys {
            self.lose(os, k, &|p: &Path| p.iface == i, RipLoss::Iface);
        }
    }

    /// removes the paths of a learned route that `gone` picks; without paths left the route is
    /// unreachable: metric 16, garbage collection, holddown
    fn lose(&mut self, os: &mut OsApi, k: Prefix4, gone: &dyn Fn(&Path) -> bool, why: RipLoss) {
        let now = os.now();
        let (gc, hold) = (self.used.gc(), self.used.holddown as Ns * SEC);
        let Some(e) = self.table.get_mut(&k) else {
            return;
        };
        let lost: Vec<Path> = e.paths.iter().filter(|p| gone(p)).copied().collect();
        if lost.is_empty() {
            return;
        }
        e.paths.retain(|p| !gone(p));
        if !e.paths.is_empty() {
            for p in lost {
                os.note(Note::RipPathGone { prefix: k, via: p.via, why });
            }
            return;
        }
        e.held_metric = e.metric;
        e.held_from = lost.iter().map(|p| p.from).collect();
        e.metric = INFINITY;
        e.changed = true;
        e.gc_at = Some(now + gc);
        e.hold_until = if hold > 0 { Some(now + hold) } else { None };
        os.note(Note::RipDown { prefix: k, via: lost[0].via, why });
    }

    /// the device's own routes: the networks of the RIP interfaces that are up, what is
    /// redistributed, the default route
    fn sync_local(&mut self, os: &mut OsApi) {
        let now = os.now();
        let ni = os.ni;
        let mut want: BTreeMap<Prefix4, (Origin, u8, RipHow)> = BTreeMap::new();
        if self.cfg.on && !self.ifs.is_empty() {
            if self.cfg.default_originate {
                want.insert(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 }, (Origin::Default, COST, RipHow::Default));
            }
            for &(what, metric) in &self.cfg.redistribute {
                let m = metric.unwrap_or(COST).clamp(1, INFINITY);
                match what {
                    Redist::Static => {
                        // the static routes the RIB uses (a floating one only once it wins)
                        let rib = os.eng.stack(ni).map(|s| s.rib(&|i| os.eng.iface_up(ni, i))).unwrap_or_default();
                        for r in rib.iter().filter(|r| r.src == RouteSrc::Static) {
                            let best = rib.iter().filter(|x| x.prefix == r.prefix).map(|x| x.ad).min();
                            if best == Some(r.ad) {
                                want.insert(r.prefix.trunc(), (Origin::Redist(Redist::Static), m, RipHow::RedistStatic));
                            }
                        }
                    }
                    Redist::Connected => {
                        for (i, f) in os.ifaces().iter().enumerate() {
                            if f.hidden || !os.eng.iface_up(ni, i as u16) {
                                continue;
                            }
                            for p in &f.addrs {
                                if !p.addr.is_loopback() {
                                    want.insert(p.trunc(), (Origin::Redist(Redist::Connected), m, RipHow::RedistConnected));
                                }
                            }
                        }
                    }
                }
            }
            for (i, r) in &self.ifs {
                if !r.up {
                    continue;
                }
                if let Some(f) = os.ifaces().get(*i as usize) {
                    for p in f.addrs.iter().filter(|p| self.cfg.covers(p.addr)) {
                        want.insert(p.trunc(), (Origin::Connected(*i), COST, RipHow::Connected));
                    }
                }
            }
        }
        let gc = self.used.gc();
        let gone: Vec<Prefix4> = self.table.iter().filter(|(k, e)| e.origin != Origin::Learned && e.reachable() && !want.contains_key(k)).map(|(k, _)| *k).collect();
        for k in gone {
            if let Some(e) = self.table.get_mut(&k) {
                e.metric = INFINITY;
                e.changed = true;
                e.gc_at = Some(now + gc);
                e.hold_until = None;
                os.note(Note::RipLocalGone { prefix: k });
            }
        }
        for (k, (origin, metric, how)) in want {
            if self.table.get(&k).is_some_and(|e| e.origin == origin && e.metric == metric) {
                continue;
            }
            self.table.insert(k, Entry::local(origin, metric));
            os.note(Note::RipLocal { prefix: k, how });
        }
    }

    /// the learned routes for the RIB (one per path)
    fn install(&self, os: &mut OsApi) {
        let mut v: Vec<Route> = Vec::new();
        for (k, e) in &self.table {
            if e.origin != Origin::Learned || !e.reachable() {
                continue;
            }
            for p in &e.paths {
                v.push(Route { prefix: *k, via: Some(p.via), iface: Some(p.iface), src: RouteSrc::Rip, ad: self.cfg.distance, metric: e.metric as u32 });
            }
        }
        if let Some(st) = os.eng.stack_mut(os.ni) {
            st.set_dynamic(RouteSrc::Rip, v);
        }
    }

    // ---------------------------------------------------------------- input

    fn ignore(&mut self, os: &mut OsApi, d: &Dgram, why: RipDrop) {
        self.n.ignored += 1;
        os.note(Note::RipIgnored { from: d.src, iface: d.iface, why });
    }

    fn input(&mut self, os: &mut OsApi, d: Dgram) {
        let Some(v) = wire::RipView::parse(&d.data) else {
            return self.ignore(os, &d, RipDrop::Short);
        };
        let Some(r) = self.ifs.get(&d.iface).cloned() else {
            return self.ignore(os, &d, RipDrop::Disabled);
        };
        let ver = v.version();
        let bit = match ver {
            0 => 0,
            1 => V1,
            _ => V2,
        };
        if r.recv & bit == 0 {
            return self.ignore(os, &d, RipDrop::Version);
        }
        if ver == 1 && (v.mbz() != 0 || v.entries().any(|e| !e.mbz_clear())) {
            return self.ignore(os, &d, RipDrop::Mbz);
        }
        if ver >= 2 && v.auth().is_some() {
            return self.ignore(os, &d, RipDrop::Auth);
        }
        match v.command() {
            wire::CMD_REQUEST => self.asked(os, &d, &r, &wire::Rip::read(&v), ver.min(2)),
            wire::CMD_RESPONSE => self.heard(os, &d, &v, ver.min(2)),
            _ => self.ignore(os, &d, RipDrop::Command),
        }
    }

    /// a request: the whole table as an update of that interface would say it, or the metrics
    /// of the entries asked for
    fn asked(&mut self, os: &mut OsApi, d: &Dgram, r: &RipIf, m: &wire::Rip, ver: u8) {
        self.n.requests_rcvd += 1;
        if r.passive || r.loopback {
            return self.ignore(os, d, RipDrop::Passive);
        }
        if r.send & (if ver == 1 { V1 } else { V2 }) == 0 {
            return self.ignore(os, d, RipDrop::NoAnswer);
        }
        let to = Some((d.src, d.sport));
        if m.is_request_all() {
            os.note(Note::RipAsked { from: d.src, iface: d.iface, whole: true });
            let a = self.advert(d.iface, r, ver, false);
            self.send(os, d.iface, ver, RipOut::Answer, to, a);
            return;
        }
        os.note(Note::RipAsked { from: d.src, iface: d.iface, whole: false });
        let mut a = Advert::default();
        for e in m.entries.iter().filter(|e| e.afi == wire::AFI_INET) {
            let p = if ver == 1 { self.infer(os, d.iface, e.addr) } else { e.prefix().map(|p| p.trunc()).or_else(|| self.infer(os, d.iface, e.addr)) };
            let metric = p.and_then(|p| self.table.get(&p)).map(|x| x.metric).unwrap_or(INFINITY);
            a.entries.push(Rte { metric: metric as u32, ..*e });
            a.shown.push((p.unwrap_or(Prefix4 { addr: e.addr, len: 32 }), metric));
            a.poisoned.push(false);
        }
        self.send(os, d.iface, ver, RipOut::Answer, to, a);
    }

    /// the mask of a version 1 entry (or a version 2 one without): within the major network of
    /// the receiving interface its mask (a host route where host bits are set), else the
    /// classful one (RFC 1058 3.7)
    fn infer(&self, os: &OsApi, iface: u16, a: Ipv4) -> Option<Prefix4> {
        if a.is_unspecified() {
            return Some(Prefix4 { addr: a, len: 0 });
        }
        let major = classful(a);
        let own = os.iface(iface).and_then(|f| f.addrs.iter().find(|q| classful(q.addr) == major).copied());
        let len = match own {
            Some(q) => q.len,
            None => major.len,
        };
        let p = Prefix4 { addr: a, len };
        Some(if p.network() == a { p } else { Prefix4 { addr: a, len: 32 } })
    }

    /// a response: checked (RFC 2453 3.9.2), then every entry offered to the table
    fn heard(&mut self, os: &mut OsApi, d: &Dgram, v: &wire::RipView, ver: u8) {
        if d.sport != PORT_RIP {
            return self.ignore(os, d, RipDrop::Port);
        }
        if !os.iface(d.iface).is_some_and(|f| f.on_link(d.src)) {
            return self.ignore(os, d, RipDrop::Neighbour);
        }
        if os.is_mine(d.src) {
            return self.ignore(os, d, RipDrop::Own);
        }
        let now = os.now();
        self.n.updates_rcvd += 1;
        self.sources.insert(d.src, now);
        let mut shown: Vec<(Prefix4, u8)> = Vec::new();
        let mut bad: Vec<Rte> = Vec::new();
        let mut offers: Vec<(Prefix4, u8, Ipv4, u16)> = Vec::new();
        for e in v.routes().filter(|e| e.afi == wire::AFI_INET) {
            let p = if ver == 1 {
                self.infer(os, d.iface, e.addr)
            } else {
                match e.prefix() {
                    Some(p) => Some(p.trunc()),
                    None if e.mask.is_unspecified() => self.infer(os, d.iface, e.addr),
                    None => None,
                }
            };
            let Some(p) = p.filter(|p| usable(*p) && (1..=INFINITY as u32).contains(&e.metric)) else {
                bad.push(e);
                continue;
            };
            shown.push((p, e.metric as u8));
            // a next hop on the interface's subnet is taken; otherwise the sender (4.4)
            let via = if ver >= 2 && !e.next_hop.is_unspecified() && !os.is_mine(e.next_hop) && os.iface(d.iface).is_some_and(|f| f.on_link(e.next_hop)) { e.next_hop } else { d.src };
            offers.push((p, (e.metric as u8).saturating_add(COST).min(INFINITY), via, if ver >= 2 { e.tag } else { 0 }));
        }
        os.note(Note::RipHeard { from: d.src, iface: d.iface, ver, routes: shown });
        for e in bad {
            self.n.bad_entries += 1;
            os.note(Note::RipBadEntry { from: d.src, addr: e.addr, metric: e.metric });
        }
        for (p, m, via, tag) in offers {
            self.offer(os, p, m, d.src, via, d.iface, tag);
        }
    }

    /// one route of a response, its metric already with the cost added
    fn offer(&mut self, os: &mut OsApi, k: Prefix4, m: u8, from: Ipv4, via: Ipv4, iface: u16, tag: u16) {
        let now = os.now();
        let (gc, hold) = (self.used.gc(), self.used.holddown as Ns * SEC);
        let max = self.cfg.max_paths.max(1) as usize;
        let path = Path { via, from, iface, heard: now };
        let Some(e) = self.table.get_mut(&k) else {
            if m < INFINITY {
                self.table.insert(k, Entry::learned(m, tag, path));
                os.note(Note::RipLearned { prefix: k, via, metric: m });
            }
            return;
        };
        if e.origin != Origin::Learned && e.reachable() {
            // an own network: what others say about it does not matter
            return;
        }
        if !e.reachable() {
            if m >= INFINITY {
                return;
            }
            let held = e.origin == Origin::Learned && e.hold_until.is_some_and(|t| now < t);
            if held && !e.held_from.contains(&from) && m >= e.held_metric {
                os.note(Note::RipHeld { prefix: k, from, metric: m });
                return;
            }
            *e = Entry::learned(m, tag, path);
            os.note(Note::RipLearned { prefix: k, via, metric: m });
            return;
        }
        match e.paths.iter().position(|p| p.from == from) {
            // the router it came from: its word counts, better or worse
            Some(i) => {
                if m == e.metric {
                    e.paths[i] = path;
                    e.tag = tag;
                } else if m < e.metric || (e.paths.len() == 1 && m < INFINITY) {
                    let old = e.metric;
                    e.paths = alloc::vec![path];
                    e.metric = m;
                    e.tag = tag;
                    e.changed = true;
                    os.note(Note::RipChanged { prefix: k, via, metric: m, old });
                } else if e.paths.len() > 1 {
                    let gone = e.paths.remove(i);
                    os.note(Note::RipPathGone { prefix: k, via: gone.via, why: if m >= INFINITY { RipLoss::Poisoned } else { RipLoss::Worse } });
                } else {
                    let gone = e.paths[0].via;
                    e.held_metric = e.metric;
                    e.held_from = alloc::vec![from];
                    e.metric = INFINITY;
                    e.paths.clear();
                    e.changed = true;
                    e.gc_at = Some(now + gc);
                    e.hold_until = if hold > 0 { Some(now + hold) } else { None };
                    os.note(Note::RipDown { prefix: k, via: gone, why: RipLoss::Poisoned });
                }
            }
            // another router: only a better metric, or one more equal-cost path
            None => {
                if m < e.metric {
                    let old = e.metric;
                    e.paths = alloc::vec![path];
                    e.metric = m;
                    e.tag = tag;
                    e.changed = true;
                    os.note(Note::RipChanged { prefix: k, via, metric: m, old });
                } else if m == e.metric && e.paths.len() < max && !e.paths.iter().any(|p| p.via == via) {
                    e.paths.push(path);
                    e.paths.sort_by_key(|p| p.via);
                    os.note(Note::RipPathAdded { prefix: k, via, metric: m });
                }
            }
        }
    }

    // ---------------------------------------------------------------- timers

    /// paths that timed out, unreachable routes whose garbage collection is over
    fn expire(&mut self, os: &mut OsApi) {
        let now = os.now();
        let inv = self.used.invalid as Ns * SEC;
        let why = RipLoss::Timeout(self.used.invalid);
        let keys: Vec<Prefix4> = self.table.keys().copied().collect();
        for k in keys {
            let Some(e) = self.table.get(&k) else {
                continue;
            };
            if !e.reachable() {
                if e.gc_at.is_some_and(|t| t <= now) {
                    self.table.remove(&k);
                    self.n.flushed += 1;
                    os.note(Note::RipFlushed { prefix: k });
                }
                continue;
            }
            if e.origin == Origin::Learned && e.paths.iter().any(|p| p.heard + inv <= now) {
                self.n.timeouts += 1;
                self.lose(os, k, &|p: &Path| p.heard + inv <= now, why);
            }
        }
    }

    /// the route timer at the next timeout or end of a garbage collection
    fn arm(&mut self, os: &mut OsApi) {
        let inv = self.used.invalid as Ns * SEC;
        let next = self
            .table
            .values()
            .filter_map(|e| if !e.reachable() { e.gc_at } else if e.origin == Origin::Learned { e.paths.iter().map(|p| p.heard + inv).min() } else { None })
            .min();
        if next != self.armed {
            self.armed = next;
            match next {
                Some(t) => os.timer(T_ROUTES, t),
                None => os.cancel(T_ROUTES),
            }
        }
    }

    // ---------------------------------------------------------------- output

    /// what an update on interface `i` in version `ver` says (all routes, or the changed ones)
    fn advert(&self, i: u16, r: &RipIf, ver: u8, only_changed: bool) -> Advert {
        let major = classful(r.addr.addr);
        let summarise = ver == 1 || self.cfg.auto_summary;
        // destination as sent -> (metric, tag, changed, split horizon poisoned it)
        let mut items: BTreeMap<Prefix4, (u8, u16, bool, bool)> = BTreeMap::new();
        for (k, e) in &self.table {
            // the interface's own networks stay off it
            if e.origin == Origin::Connected(i) {
                continue;
            }
            let behind = e.origin == Origin::Learned && e.paths.iter().any(|p| p.iface == i);
            let metric = if behind { INFINITY } else { e.metric };
            let other = classful(k.addr) != major;
            let dest = if k.len == 0 {
                Some(*k)
            } else if other && k.len >= classful_len(k.addr) && summarise {
                Some(classful(k.addr))
            } else if ver == 1 {
                // no masks: only the subnets with this interface's mask, and host routes
                if !other && (k.len == r.addr.len || k.len == 32) { Some(*k) } else { None }
            } else {
                Some(*k)
            };
            let Some(dest) = dest else {
                continue;
            };
            let poisoned = behind && e.reachable();
            let tag = if dest == *k { e.tag } else { 0 };
            let x = items.entry(dest).or_insert((metric, tag, false, false));
            if metric < x.0 {
                x.0 = metric;
                x.1 = tag;
            }
            x.2 |= e.changed;
            x.3 |= poisoned;
        }
        let mut a = Advert::default();
        for (dest, (metric, tag, changed, poisoned)) in items {
            if only_changed && !changed {
                continue;
            }
            a.entries.push(if ver == 1 { Rte::v1(dest.addr, metric as u32) } else { Rte::v2(dest, Ipv4::UNSPECIFIED, metric as u32, tag) });
            a.shown.push((dest, metric));
            a.poisoned.push(poisoned && metric == INFINITY);
        }
        a
    }

    /// sends what `a` holds on interface `i`, in messages of at most 25 entries; `to`: the
    /// asker (else the group or the broadcast address); the number of messages
    fn send(&mut self, os: &mut OsApi, i: u16, ver: u8, kind: RipOut, to: Option<(Ipv4, u16)>, a: Advert) -> u32 {
        let Some(src) = self.ifs.get(&i).map(|r| r.addr.addr) else {
            return 0;
        };
        let (dst, dport) = to.unwrap_or((if ver == 1 { Ipv4::BROADCAST } else { wire::GROUP }, PORT_RIP));
        let mut sent = 0;
        for k in (0..a.entries.len()).step_by(wire::MAX_ENTRIES) {
            let end = (k + wire::MAX_ENTRIES).min(a.entries.len());
            let msg = wire::Rip::response(ver, a.entries[k..end].to_vec()).to_bytes();
            let poisoned = a.poisoned[k..end].iter().filter(|p| **p).count() as u16;
            os.note(Note::RipSent { iface: i, ver, to: dst, kind, routes: a.shown[k..end].to_vec(), poisoned });
            match kind {
                RipOut::Regular => self.n.updates_sent += 1,
                RipOut::Triggered => {
                    self.n.updates_sent += 1;
                    self.n.triggered += 1;
                }
                RipOut::Answer => self.n.answers += 1,
            }
            emit(os, i, src, dst, dport, &msg);
            sent += 1;
        }
        sent
    }

    /// a request for the whole table on interface `i`
    fn request(&mut self, os: &mut OsApi, i: u16) {
        let Some(r) = self.ifs.get(&i) else {
            return;
        };
        let ver = if r.send & V2 != 0 { 2 } else { 1 };
        let (src, dst) = (r.addr.addr, if ver == 1 { Ipv4::BROADCAST } else { wire::GROUP });
        os.note(Note::RipRequest { iface: i, ver });
        self.n.requests_sent += 1;
        emit(os, i, src, dst, PORT_RIP, &wire::Rip::request_all(ver).to_bytes());
    }

    /// an interface came up: ask for the whole table there and send the own one
    fn greet(&mut self, os: &mut OsApi, i: u16) {
        let Some(r) = self.ifs.get(&i).cloned() else {
            return;
        };
        if !r.speaks() {
            return;
        }
        self.request(os, i);
        for ver in [1u8, 2] {
            if r.send & (if ver == 1 { V1 } else { V2 }) != 0 {
                let a = self.advert(i, &r, ver, false);
                self.send(os, i, ver, RipOut::Regular, None, a);
            }
        }
        self.fresh.push(i);
    }

    /// an update on every interface that sends; the number of messages
    fn update_all(&mut self, os: &mut OsApi, kind: RipOut) -> u32 {
        let ifs: Vec<(u16, RipIf)> = self.ifs.iter().filter(|(_, r)| r.speaks()).map(|(i, r)| (*i, r.clone())).collect();
        let mut sent = 0;
        for (i, r) in ifs {
            if kind == RipOut::Triggered && self.fresh.contains(&i) {
                continue;
            }
            for ver in [1u8, 2] {
                if r.send & (if ver == 1 { V1 } else { V2 }) != 0 {
                    let a = self.advert(i, &r, ver, kind == RipOut::Triggered);
                    sent += self.send(os, i, ver, kind, None, a);
                }
            }
        }
        sent
    }

    /// the periodic update
    fn regular(&mut self, os: &mut OsApi) {
        self.update_all(os, RipOut::Regular);
        for e in self.table.values_mut() {
            e.changed = false;
        }
        self.pending = false;
        let at = os.now() + self.period(os);
        self.next_update = Some(at);
        os.timer(T_UPDATE, at);
    }

    /// changed routes go out at once, unless the hold of the last triggered update runs or a
    /// regular update is due
    fn trigger(&mut self, os: &mut OsApi) {
        if !self.table.values().any(|e| e.changed) {
            return;
        }
        let now = os.now();
        if self.hold.is_some_and(|t| t > now) {
            self.pending = true;
            return;
        }
        if self.next_update.is_some_and(|t| t <= now) {
            return;
        }
        let sent = self.update_all(os, RipOut::Triggered);
        for e in self.table.values_mut() {
            e.changed = false;
        }
        self.pending = false;
        if sent > 0 {
            let h = now + (1000 + os.rand() as Ns % 4001) * MS;
            self.hold = Some(h);
            os.timer(T_HOLD, h);
        }
    }

    /// the hold after a triggered update is over: what changed meanwhile goes out now
    fn hold_over(&mut self, os: &mut OsApi) {
        self.hold = None;
        if core::mem::take(&mut self.pending) {
            self.trigger(os);
        }
    }

    /// after every event: triggered updates, the RIB, the route timer
    fn after(&mut self, os: &mut OsApi) {
        if !self.running {
            return;
        }
        self.trigger(os);
        self.fresh.clear();
        self.install(os);
        self.arm(os);
    }

    /// "clear ip route *": the learned routes are forgotten and asked for again
    fn clear(&mut self, os: &mut OsApi) {
        self.table.retain(|_, e| e.origin != Origin::Learned);
        self.sources.clear();
        let ifs: Vec<u16> = self.ifs.iter().filter(|(_, r)| r.speaks()).map(|(i, _)| *i).collect();
        for i in ifs {
            self.request(os, i);
        }
    }
}

/// the entries of an update: as sent, as shown (destination and metric), poisoned by split
/// horizon
#[derive(Clone, Debug, Default)]
struct Advert {
    entries: Vec<Rte>,
    shown: Vec<(Prefix4, u8)>,
    poisoned: Vec<bool>,
}

/// a RIP datagram out of interface `i`: to a group or the broadcast address straight on the link
/// (TTL 1), to an asker over the routing table
fn emit(os: &mut OsApi, i: u16, src: Ipv4, dst: Ipv4, dport: u16, data: &[u8]) {
    let (ni, cause) = (os.ni, os.cause);
    let link = dst.is_multicast() || dst.is_broadcast();
    let Some(st) = os.eng.stack_mut(ni) else {
        return;
    };
    let ttl = if link { 1 } else { st.ttl };
    let id = st.next_id();
    let mut p = Pkt::new(HEADROOM, data);
    UdpHdr::new(PORT_RIP, dport).push_v4(&mut p, src, dst);
    let mut h = Ipv4Hdr::new(src, dst, PROTO_UDP, ttl);
    h.id = id;
    h.push_onto(&mut p);
    let pkt = p.into_vec();
    if link {
        let mac = if dst.is_broadcast() { Mac::BROADCAST } else { Mac::of_ipv4_multicast(dst) };
        os.eng.frame_out(ni, i, mac, ET_IPV4, &pkt, cause);
    } else {
        os.eng.ip_output(ni, pkt, IpOrigin::Local { job: None }, cause);
    }
}

impl Service for Rip {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn start(&mut self, os: &mut OsApi) {
        self.running = true;
        self.table.clear();
        self.ifs.clear();
        self.sources.clear();
        self.hold = None;
        self.pending = false;
        self.fresh.clear();
        self.armed = None;
        self.used = self.cfg.timers;
        if os.bind(PORT_RIP, None).is_err() {
            os.note(Note::PortInUse { port: PORT_RIP, svc: String::from(KIND) });
        }
        let at = os.now() + self.period(os);
        self.next_update = Some(at);
        os.timer(T_UPDATE, at);
        self.sync(os);
        self.after(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.sync(os);
        match ev {
            SvcEvent::Udp(d) => self.input(os, d),
            SvcEvent::Timer(T_UPDATE) => self.regular(os),
            SvcEvent::Timer(T_HOLD) => self.hold_over(os),
            SvcEvent::Timer(T_ROUTES) => {
                self.armed = None;
                self.expire(os);
            }
            SvcEvent::Ctl(Ctl::Renew) => self.clear(os),
            _ => {}
        }
        self.after(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        self.running = false;
        self.table.clear();
        self.ifs.clear();
        self.sources.clear();
        self.next_update = None;
        self.hold = None;
        self.pending = false;
        self.fresh.clear();
        self.armed = None;
        if let Some(st) = os.eng.stack_mut(os.ni) {
            st.set_dynamic(RouteSrc::Rip, Vec::new());
            st.groups.retain(|g| g.1 != wire::GROUP);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        let routes = self.table.values().filter(|e| e.reachable()).count() as i64;
        let learned = self.table.values().filter(|e| e.reachable() && e.origin == Origin::Learned).count() as i64;
        let n = &self.n;
        alloc::vec![
            Metric::new("routes", routes),
            Metric::new("learned", learned),
            Metric::new("neighbours", self.sources.len() as i64),
            Metric::new("updates_sent", n.updates_sent as i64),
            Metric::new("triggered", n.triggered as i64),
            Metric::new("answers", n.answers as i64),
            Metric::new("requests_sent", n.requests_sent as i64),
            Metric::new("updates_rcvd", n.updates_rcvd as i64),
            Metric::new("requests_rcvd", n.requests_rcvd as i64),
            Metric::new("ignored", n.ignored as i64),
            Metric::new("bad_entries", n.bad_entries as i64),
            Metric::new("timeouts", n.timeouts as i64),
            Metric::new("flushed", n.flushed as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        match name {
            "routes" | "learned" | "neighbours" => MetricKind::Gauge,
            _ => MetricKind::Counter,
        }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.cfg.on {
            return msg!("RIP ist nicht eingerichtet (kein router rip).");
        }
        let routes = self.table.values().filter(|e| e.reachable()).count();
        let learned = self.table.values().filter(|e| e.reachable() && e.origin == Origin::Learned).count();
        match self.next_update {
            Some(t) if self.running => msg!("RIP Version {}: Routen: {}, davon gelernt: {}, Nachbarn: {}; nächstes Update bei {}.", self.version(), routes, learned, self.sources.len(), dur(t as i64)),
            _ => msg!("RIP Version {}: wartet auf das Gerät.", self.version()),
        }
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

// ---------------------------------------------------------------- the engine side

/// the device whose interfaces or routes an action may change (its RIP looks again)
pub(crate) fn touches(a: &Action) -> Option<u32> {
    match a {
        Action::IfAddr { dev, .. }
        | Action::IfUp { dev, .. }
        | Action::Svi { dev, .. }
        | Action::SubIf { dev, .. }
        | Action::Loopback { dev, .. }
        | Action::RoutedPort { dev, .. }
        | Action::Route { dev, .. }
        | Action::Forwarding { dev, .. }
        | Action::Switchport { dev, .. }
        | Action::Access { dev, .. }
        | Action::Trunk { dev, .. }
        | Action::Vlan { dev, .. }
        | Action::PortAdmin { dev, .. } => Some(*dev),
        _ => None,
    }
}

impl Engine {
    /// the RIP service of a device (None before the first RIP command)
    pub fn rip(&self, dev: u32) -> Option<&Rip> {
        self.rip_of(self.node_index(dev)?)
    }
    pub(crate) fn rip_of(&self, ni: usize) -> Option<&Rip> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND)?.svc.as_ref()?.as_any().downcast_ref::<Rip>()
    }
    fn rip_mut(&mut self, ni: usize) -> Option<&mut Rip> {
        self.os_mut(ni)?.svcs.iter_mut().find(|s| s.kind == KIND)?.svc.as_mut()?.as_any_mut().downcast_mut::<Rip>()
    }
    /// (service id, running) of the RIP service of node `ni`
    fn rip_slot(&self, ni: usize) -> Option<(u32, bool)> {
        self.os(ni)?.svcs.iter().find(|s| s.kind == KIND).map(|s| (s.id, s.running))
    }
    /// "router rip" is configured on node `ni`
    pub(crate) fn rip_on(&self, ni: usize) -> bool {
        self.rip_of(ni).is_some_and(|r| r.cfg.on)
    }
    /// something about node `ni`'s interfaces or routes may have changed: its RIP looks again
    /// at the same virtual time (a timer of its own, so it never runs inside another call)
    pub(crate) fn rip_kick(&mut self, ni: usize) {
        if let Some((id, true)) = self.rip_slot(ni) {
            let now = self.now;
            self.set_timer(ni, SLOT_SVC + id * SVC_TIMERS + T_LOOK, now);
        }
    }
    pub(crate) fn rip_kick_dev(&mut self, dev: u32) {
        if let Some(ni) = self.node_index(dev) {
            self.rip_kick(ni);
        }
    }

    /// Action::Rip: a change of the configuration; the service comes with the first one
    /// (stopped until "router rip")
    pub(crate) fn rip_apply(&mut self, dev: u32, set: RipSet) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let routes = matches!(self.nodes.get(ni), Some(Some(n)) if n.model.has(F_ROUTER));
        if !routes {
            return Err(ActErr::Refused(msg!("{} routet nicht: hier gibt es kein RIP.", self.host(ni))));
        }
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if self.rip_slot(ni).is_none() {
            // undoing what was never set needs no service
            if matches!(set, RipSet::Process(false) | RipSet::Clear | RipSet::IfVersions { versions: None, .. }) {
                return Ok(());
            }
            self.add_service(dev, KIND, Box::new(Rip::new()), false)?;
        }
        let (id, running) = self.rip_slot(ni).ok_or(ActErr::NoIp(dev))?;
        let canon = |e: &Engine, name: &str| -> Result<String, ActErr> {
            let st = e.stack(ni).ok_or(ActErr::NoIp(dev))?;
            let i = st.iface_by_name(name).ok_or_else(|| ActErr::UnknownIface { dev, iface: String::from(name) })?;
            Ok(st.ifaces[i as usize].name.clone())
        };
        let set = match set {
            RipSet::Process(on) => {
                if let Some(r) = self.rip_mut(ni) {
                    if on {
                        r.cfg.on = true;
                    } else {
                        r.cfg.reset();
                    }
                }
                if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                    s.enabled = on;
                }
                if !on {
                    self.svc_stop(ni, id);
                } else if !running {
                    self.os_push(ni, Job::Start(id));
                }
                return Ok(());
            }
            RipSet::Clear => {
                if running {
                    self.svc_ctl(ni, id, Ctl::Renew);
                }
                return Ok(());
            }
            RipSet::Passive { iface: Some(name), on } => RipSet::Passive { iface: Some(canon(self, &name)?), on },
            RipSet::IfVersions { iface, receive, versions } => RipSet::IfVersions { iface: canon(self, &iface)?, receive, versions },
            other => other,
        };
        let r = self.rip_mut(ni).ok_or(ActErr::NoIp(dev))?;
        match set {
            RipSet::Version(v) => {
                if v.is_some_and(|x| !(1..=2).contains(&x)) {
                    return Err(ActErr::Refused(msg!("RIP kennt die Versionen 1 und 2.")));
                }
                r.cfg.version = v;
            }
            RipSet::Network { net, add } => {
                if classful_len(net) == 32 || net.0[0] == 0 || net.is_loopback() {
                    return Err(ActErr::Refused(msg!("{} ist kein Netz für RIP.", net)));
                }
                let c = classful(net).addr;
                r.cfg.networks.retain(|x| *x != c);
                if add {
                    r.cfg.networks.push(c);
                    r.cfg.networks.sort();
                }
            }
            RipSet::Passive { iface: None, on } => {
                r.cfg.passive_default = on;
                r.cfg.passive.clear();
            }
            RipSet::Passive { iface: Some(name), on } => {
                r.cfg.passive.retain(|x| !x.eq_ignore_ascii_case(&name));
                if on != r.cfg.passive_default {
                    r.cfg.passive.push(name);
                }
            }
            RipSet::AutoSummary(on) => r.cfg.auto_summary = on,
            RipSet::DefaultOriginate(on) => r.cfg.default_originate = on,
            RipSet::Redistribute { what, metric, add } => {
                r.cfg.redistribute.retain(|x| x.0 != what);
                if add {
                    r.cfg.redistribute.push((what, metric.map(|m| m.clamp(1, INFINITY))));
                    r.cfg.redistribute.sort();
                }
            }
            RipSet::Timers(t) => {
                let t = t.unwrap_or_default();
                if t.update == 0 || t.invalid == 0 || t.flush < t.invalid {
                    return Err(ActErr::Refused(msg!("Die Zeiten passen nicht: Update und Invalid größer als 0, Flush nicht kürzer als Invalid.")));
                }
                r.cfg.timers = t;
            }
            RipSet::Distance(d) => r.cfg.distance = d.unwrap_or(DISTANCE).max(1),
            RipSet::MaxPaths(n) => r.cfg.max_paths = n.unwrap_or(MAX_PATHS).clamp(1, 32),
            RipSet::IfVersions { iface, receive, versions } => {
                let map = if receive { &mut r.cfg.recv } else { &mut r.cfg.send };
                match versions.filter(|v| *v & (V1 | V2) != 0) {
                    Some(v) => {
                        map.insert(iface, v & (V1 | V2));
                    }
                    None => {
                        map.remove(&iface);
                    }
                }
            }
            RipSet::Process(_) | RipSet::Clear => {}
        }
        if running {
            self.rip_kick(ni);
        }
        Ok(())
    }
}
