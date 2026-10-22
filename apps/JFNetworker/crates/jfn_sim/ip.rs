// ip -- the IPv4 part of a device: interfaces with addresses, the ARP cache, static routes
// and the routing table they make
//
//   Iface { name, kind: Port(p) | Sub { port, vlan } | Svi(vlan) | Loopback(n), mac, addrs,
//           mtu, admin_up }
//   IpStack::for_model(..)        the interfaces a device starts with (hosts: one per port,
//                                 routers: one per port, switches: Vlan1 shut down, Linux
//                                 bridges: br0, access points: BVI1; Linux devices also lo
//                                 with 127.0.0.1/8)
//   iface_by_name(name)           "Gi0/0/0", "gi0/0/0", "Vlan10", "Loopback0", "eth0", "br0"
//   add_addr / remove_addr / add_route / remove_route / flush_arp / reset
//   icmp_allow(now, type, code, to)   the ICMP error limiter of Linux or IOS
//   rib(up) -> Vec<Route>         connected and local routes of the interfaces that are up,
//                                 static routes whose interface is up or whose next hop is
//                                 reachable (one level of recursion)
//   lookup(up, dst) -> NextHop    longest prefix, then administrative distance, then metric;
//                                 the table of the last lookup is kept with what it was made of
//                                 (the interfaces' states and addresses, the static routes) and
//                                 made again only when that changed
//   set_dynamic(src, routes)      the routes of a routing protocol (RIP: rip.rs) in the table,
//                                 next to connected and static ones; equal-cost routes of a
//                                 protocol share the destinations: one per destination address
//   join(iface, group, on) / joined   IPv4 multicast groups an interface listens to (224.0.0.9
//                                 for RIPv2); packets to other groups are not for the device
//
// The engine decides which interfaces are up (iface_up in l3.rs); this module only keeps the
// data and answers questions about it.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use jfn_model::catalog::{CliStyle, Media, Model, F_AP, F_BRIDGE, F_HOST, F_ROUTER};
use jfn_model::ifname;
use netpkt::icmp;
use netpkt::ipv4::Reassembly;
use netpkt::{Ipv4, Mac, Prefix4};

use crate::kernel::{Ns, MS, SEC};
use crate::node::Port;
use crate::trace::HopId;

/// node timer slots of the IPv4 part
pub const SLOT_ARP: u32 = 1;
pub const SLOT_REASM: u32 = 2;

/// what an interface is bound to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfKind {
    /// a port that is no bridge member (host NIC, router interface, routed switch port)
    Port(u16),
    /// an 802.1Q subinterface of a routed port ("GigabitEthernet0/0/0.10", "eth0.10"): frames
    /// with this VLAN tag
    Sub { port: u16, vlan: u16 },
    /// the bridge's own interface in a VLAN (IOS "interface Vlan10", Linux "br0")
    Svi(u16),
    /// a loopback interface
    Loopback(u16),
}

/// an IPv4 interface
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub kind: IfKind,
    pub mac: Mac,
    /// the first address is the primary one (the source of what the device sends)
    pub addrs: Vec<Prefix4>,
    pub mtu: u16,
    /// "no shutdown" for SVIs and loopbacks (ports use the port's admin state)
    pub admin_up: bool,
    /// removed ("no interface Vlan10", "switchport" again); kept so indexes stay valid
    pub hidden: bool,
    /// answers ARP for addresses it routes elsewhere (IOS "ip proxy-arp", on by default;
    /// Linux net.ipv4.conf.*.proxy_arp, off)
    pub proxy_arp: bool,
}

impl Iface {
    pub fn primary(&self) -> Option<Ipv4> {
        self.addrs.first().map(|p| p.addr)
    }
    pub fn has(&self, ip: Ipv4) -> bool {
        self.addrs.iter().any(|p| p.addr == ip)
    }
    /// `ip` lies in one of the interface's subnets
    pub fn on_link(&self, ip: Ipv4) -> bool {
        self.addrs.iter().any(|p| p.contains(ip))
    }
    /// `ip` is the directed broadcast of one of the subnets
    pub fn is_broadcast(&self, ip: Ipv4) -> bool {
        self.addrs.iter().any(|p| p.len <= 30 && p.broadcast() == ip)
    }
    pub fn is_loopback(&self) -> bool {
        matches!(self.kind, IfKind::Loopback(_))
    }
}

/// where a route came from
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteSrc {
    Connected,
    Local,
    Static,
    Rip,
    Ospf,
}

impl RouteSrc {
    /// the code IOS shows in "show ip route"
    pub fn code(self) -> &'static str {
        match self {
            RouteSrc::Connected => "C",
            RouteSrc::Local => "L",
            RouteSrc::Static => "S",
            RouteSrc::Rip => "R",
            RouteSrc::Ospf => "O",
        }
    }
}

/// a route
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// the destination (network form)
    pub prefix: Prefix4,
    /// the next hop (None: directly connected)
    pub via: Option<Ipv4>,
    /// the outgoing interface (resolved for the table)
    pub iface: Option<u16>,
    pub src: RouteSrc,
    pub ad: u8,
    pub metric: u32,
}

/// the next step of a packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextHop {
    pub iface: u16,
    /// the neighbour to deliver to (the next hop, or the destination itself)
    pub gw: Ipv4,
    pub src: RouteSrc,
}

/// who sent a packet that waits or fails
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// made by this device (a job's probe and its sequence number, or a reply)
    Local { job: Option<(u32, u16)> },
    /// forwarded; arrived on this interface
    Forwarded { iface: u16 },
}

/// a packet waiting for ARP
#[derive(Clone, Debug)]
pub struct Pending {
    pub pkt: Vec<u8>,
    pub origin: Origin,
    pub cause: Option<HopId>,
}

/// an ARP cache entry
#[derive(Clone, Debug)]
pub struct ArpEntry {
    /// None while the request is unanswered
    pub mac: Option<Mac>,
    /// learned, or the last request sent
    pub at: Ns,
    pub tries: u8,
    pub queue: Vec<Pending>,
    pub fixed: bool,
}

/// at most this many packets wait for one ARP answer on Linux (unres_qlen; the oldest go);
/// IOS keeps none: the packet that needs the answer is dropped (the "." of ".!!!!")
pub const ARP_QUEUE: usize = 101;
/// ARP requests before giving up, one per second
pub const ARP_TRIES: u8 = 3;
/// a fragmented datagram is given up after this long
pub const REASM_TIME: Ns = 30 * SEC;

/// the IPv4 part of a device
#[derive(Clone, Debug)]
pub struct IpStack {
    pub ifaces: Vec<Iface>,
    pub arp: BTreeMap<(u16, Ipv4), ArpEntry>,
    pub statics: Vec<Route>,
    /// routes between interfaces (routers; "ip routing" on L3 switches)
    pub forwarding: bool,
    /// the TTL of packets it makes (IOS 255, Linux 64)
    pub ttl: u8,
    pub arp_timeout: Ns,
    pub(crate) ip_id: u16,
    /// how ICMP errors are limited
    pub icmp_limit: IcmpLimit,
    pub(crate) icmp_rate: IcmpRate,
    pub(crate) reasm: BTreeMap<(Ipv4, Ipv4, u8, u16), (Reassembly, Ns)>,
    /// refuse a subnet that overlaps one of another interface (IOS; Linux allows it)
    pub overlap_check: bool,
    /// ICMP errors quote as much of the packet as fits into 576 bytes (Linux, RFC 1812
    /// 4.3.2.3); otherwise the header and 8 bytes (IOS, RFC 792)
    pub long_quote: bool,
    /// packets that may wait for one ARP answer (Linux ARP_QUEUE, IOS 0)
    pub arp_queue: usize,
    /// the routing table of the last lookup (every forwarded packet looks one up)
    pub(crate) rib_cache: RefCell<RibCache>,
    /// the routes of routing protocols (set_dynamic); their interface is resolved
    pub dynamic: Vec<Route>,
    /// counts the changes of `dynamic` (the table of the last lookup compares it)
    pub(crate) dyn_gen: u64,
    /// multicast groups the device listens to: (interface, group)
    pub groups: Vec<(u16, Ipv4)>,
}

/// a routing table with what it was made of: per interface its state and addresses, the static
/// routes, the generation of the protocols' routes (equal inputs give the same table)
#[derive(Clone, Debug, Default)]
pub(crate) struct RibCache {
    up: Vec<bool>,
    addrs: Vec<Vec<Prefix4>>,
    statics: Vec<Route>,
    dyn_gen: u64,
    rib: Vec<Route>,
}

/// how a device limits the ICMP errors it sends
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcmpLimit {
    /// every error takes a token from a shared bucket (50, refilled with 1000 a second:
    /// icmp_msgs_burst, icmp_msgs_per_sec) and a second of credit kept per destination (at
    /// most 6: icmp_ratelimit with icmp_ratemask covering every error type)
    Linux,
    /// an unreachable at most every 500 ms ("ip icmp rate-limit unreachable"; "fragmentation
    /// needed" counted apart); time exceeded is not limited
    Ios,
}

pub const LINUX_ICMP_BURST: u32 = 50;
pub const LINUX_ICMP_PER_SEC: u64 = 1000;
/// the most credit a destination can save up (six errors at once)
pub const LINUX_PEER_CREDIT: Ns = 6 * SEC;
pub const IOS_UNREACH_EVERY: Ns = 500 * MS;

/// the state of the ICMP limiter
#[derive(Clone, Debug)]
pub(crate) struct IcmpRate {
    tokens: u32,
    at: Ns,
    /// destination -> (credit, when it was counted)
    peers: BTreeMap<Ipv4, (Ns, Ns)>,
    unreach: Option<Ns>,
    frag: Option<Ns>,
}

impl IcmpRate {
    pub(crate) fn new() -> IcmpRate {
        IcmpRate { tokens: LINUX_ICMP_BURST, at: 0, peers: BTreeMap::new(), unreach: None, frag: None }
    }
}

impl IpStack {
    /// the interfaces a device of `model` starts with; None for devices without IP (hub,
    /// modem, cloud)
    pub fn for_model(model: &Model, ports: &[Port], member: &dyn Fn(u16) -> bool, base_mac: Mac) -> Option<IpStack> {
        let ip = model.has(F_HOST) || model.has(F_ROUTER) || model.has(F_AP) || (model.has(F_BRIDGE) && model.cli != CliStyle::None);
        if !ip {
            return None;
        }
        let ios = model.cli == CliStyle::Ios;
        let mut ifaces = Vec::new();
        if model.cli == CliStyle::Linux {
            let lo = Prefix4 { addr: Ipv4::LOCALHOST, len: 8 };
            ifaces.push(Iface { name: String::from("lo"), kind: IfKind::Loopback(0), mac: Mac::ZERO, addrs: alloc::vec![lo], mtu: 65535, admin_up: true, hidden: false, proxy_arp: false });
        }
        for (i, p) in ports.iter().enumerate() {
            let routed = matches!(p.spec.media, Media::Rj45 | Media::Sfp | Media::SfpPlus | Media::Sfp28 | Media::Qsfp28 | Media::RadioClient | Media::RadioAp | Media::Serial);
            if routed && !member(i as u16) {
                ifaces.push(Iface { name: p.spec.name.clone(), kind: IfKind::Port(i as u16), mac: p.mac, addrs: Vec::new(), mtu: 1500, admin_up: true, hidden: false, proxy_arp: ios });
            }
        }
        if model.has(F_BRIDGE) {
            let (name, up) = match model.cli {
                CliStyle::Ios if model.has(F_AP) => (String::from("BVI1"), true),
                CliStyle::Ios => (String::from("Vlan1"), false),
                _ => (String::from("br0"), true),
            };
            ifaces.push(Iface { name, kind: IfKind::Svi(1), mac: base_mac, addrs: Vec::new(), mtu: 1500, admin_up: up, hidden: false, proxy_arp: ios });
        }
        Some(IpStack {
            ifaces,
            arp: BTreeMap::new(),
            statics: Vec::new(),
            forwarding: model.has(F_ROUTER) && (!model.has(F_BRIDGE) || model.has(F_HOST)),
            ttl: if ios { 255 } else { 64 },
            arp_timeout: if ios { 4 * 3600 * SEC } else { 60 * SEC },
            ip_id: 0,
            icmp_limit: if ios { IcmpLimit::Ios } else { IcmpLimit::Linux },
            icmp_rate: IcmpRate::new(),
            reasm: BTreeMap::new(),
            overlap_check: ios,
            long_quote: !ios,
            arp_queue: if ios { 0 } else { ARP_QUEUE },
            rib_cache: RefCell::new(RibCache::default()),
            dynamic: Vec::new(),
            dyn_gen: 0,
            groups: Vec::new(),
        })
    }

    /// an interface by any spelling of its name (removed ones are not found)
    pub fn iface_by_name(&self, name: &str) -> Option<u16> {
        self.find_name(name).filter(|&i| !self.ifaces[i as usize].hidden)
    }
    /// an interface by name, removed ones included
    pub(crate) fn find_name(&self, name: &str) -> Option<u16> {
        let n = name.trim();
        if let Some(i) = self.ifaces.iter().position(|f| f.name.eq_ignore_ascii_case(n)) {
            return Some(i as u16);
        }
        let full = ifname::expand(n)?;
        self.ifaces.iter().position(|f| f.name.eq_ignore_ascii_case(&full)).map(|i| i as u16)
    }
    /// the interface of a port
    pub fn iface_of_port(&self, port: u16) -> Option<u16> {
        self.ifaces.iter().position(|f| f.kind == IfKind::Port(port)).map(|i| i as u16)
    }
    /// the subinterface of a port for a VLAN tag
    pub fn iface_of_sub(&self, port: u16, vlan: u16) -> Option<u16> {
        self.ifaces.iter().position(|f| !f.hidden && f.kind == IfKind::Sub { port, vlan }).map(|i| i as u16)
    }
    /// the interface of a VLAN
    pub fn iface_of_vlan(&self, vlan: u16) -> Option<u16> {
        self.ifaces.iter().position(|f| f.kind == IfKind::Svi(vlan)).map(|i| i as u16)
    }
    /// any interface has this address
    pub fn is_mine(&self, ip: Ipv4) -> bool {
        self.ifaces.iter().any(|f| !f.hidden && f.has(ip))
    }
    /// the next IP identification number
    pub(crate) fn next_id(&mut self) -> u16 {
        self.ip_id = self.ip_id.wrapping_add(1);
        self.ip_id
    }

    /// adds an address (the first one of an interface becomes its primary address)
    pub fn add_addr(&mut self, iface: u16, p: Prefix4) -> Result<(), String> {
        let lo = self.ifaces.get(iface as usize).is_some_and(|f| f.is_loopback());
        let class_e = p.addr.0[0] >= 240;
        if p.len > 32 || p.addr.is_unspecified() || p.addr.is_multicast() || p.addr.is_broadcast() || class_e || p.is_reserved_host(p.addr) || (p.addr.is_loopback() && !lo) {
            return Err(format!("{} ist keine gültige Interface-Adresse.", p));
        }
        if self.overlap_check
            && let Some(f) = self.ifaces.iter().enumerate().find(|(i, f)| *i != iface as usize && !f.hidden && f.addrs.iter().any(|q| q.trunc().overlaps(&p.trunc())))
        {
            return Err(format!("{} überschneidet sich mit {}.", p.trunc(), f.1.name));
        }
        let f = self.ifaces.get_mut(iface as usize).ok_or_else(|| String::from("Unbekanntes Interface."))?;
        if !f.addrs.contains(&p) {
            f.addrs.push(p);
        }
        Ok(())
    }
    pub fn remove_addr(&mut self, iface: u16, p: Prefix4) -> bool {
        let Some(f) = self.ifaces.get_mut(iface as usize) else {
            return false;
        };
        let n = f.addrs.len();
        f.addrs.retain(|q| *q != p);
        let changed = f.addrs.len() != n;
        if changed {
            self.flush_arp(iface);
        }
        changed
    }
    /// forgets what ARP learned on an interface (static entries stay)
    pub fn flush_arp(&mut self, iface: u16) {
        self.arp.retain(|(i, _), e| *i != iface || e.fixed);
    }
    /// takes an interface away ("no interface Vlan10", "switchport"): no addresses, no
    /// neighbours; the slot stays so that interface numbers keep their meaning
    pub fn hide(&mut self, iface: u16) {
        if let Some(f) = self.ifaces.get_mut(iface as usize) {
            f.hidden = true;
            f.addrs.clear();
            self.statics.retain(|r| r.iface != Some(iface));
        }
        self.arp.retain(|(i, _), _| *i != iface);
    }
    /// what is forgotten when the device is switched off: learned neighbours, fragments
    pub fn reset(&mut self) {
        self.arp.retain(|_, e| e.fixed);
        self.reasm.clear();
        self.icmp_rate = IcmpRate::new();
    }

    /// may an ICMP error of `typ` / `code` go to `to` now?  (Takes what it costs.)
    pub(crate) fn icmp_allow(&mut self, now: Ns, typ: u8, code: u8, to: Ipv4) -> bool {
        let r = &mut self.icmp_rate;
        match self.icmp_limit {
            IcmpLimit::Ios => {
                if typ != icmp::T_UNREACH {
                    return true;
                }
                let last = if code == icmp::UNREACH_FRAG_NEEDED { &mut r.frag } else { &mut r.unreach };
                match *last {
                    Some(t) if now < t + IOS_UNREACH_EVERY => false,
                    _ => {
                        *last = Some(now);
                        true
                    }
                }
            }
            IcmpLimit::Linux => {
                let add = now.saturating_sub(r.at) * LINUX_ICMP_PER_SEC / SEC;
                if add > 0 {
                    r.tokens = (r.tokens as u64 + add).min(LINUX_ICMP_BURST as u64) as u32;
                    // the fraction of a token stays for the next time
                    r.at += add * SEC / LINUX_ICMP_PER_SEC;
                }
                if r.tokens == 0 {
                    return false;
                }
                if r.peers.len() >= 4096 {
                    r.peers.retain(|_, (c, t)| *c + now.saturating_sub(*t) < LINUX_PEER_CREDIT);
                }
                let (credit, at) = r.peers.get(&to).copied().unwrap_or((LINUX_PEER_CREDIT, now));
                let credit = (credit + now.saturating_sub(at)).min(LINUX_PEER_CREDIT);
                if credit < SEC {
                    r.peers.insert(to, (credit, now));
                    return false;
                }
                r.peers.insert(to, (credit - SEC, now));
                r.tokens -= 1;
                true
            }
        }
    }
    pub fn add_route(&mut self, r: Route) {
        let r = Route { prefix: r.prefix.trunc(), ..r };
        self.statics.retain(|s| !(s.prefix == r.prefix && s.via == r.via && s.iface == r.iface));
        self.statics.push(r);
    }
    pub fn remove_route(&mut self, prefix: Prefix4, via: Option<Ipv4>) -> bool {
        let p = prefix.trunc();
        let n = self.statics.len();
        self.statics.retain(|s| !(s.prefix == p && (via.is_none() || s.via == via)));
        self.statics.len() != n
    }

    /// the routes of a routing protocol replace the ones it gave before (their interfaces
    /// resolved); false when nothing changed
    pub fn set_dynamic(&mut self, src: RouteSrc, routes: Vec<Route>) -> bool {
        let mine: Vec<&Route> = self.dynamic.iter().filter(|r| r.src == src).collect();
        if mine.len() == routes.len() && mine.iter().zip(routes.iter()).all(|(a, b)| *a == b) {
            return false;
        }
        self.dynamic.retain(|r| r.src != src);
        self.dynamic.extend(routes);
        self.dyn_gen += 1;
        true
    }
    /// the interface listens to a multicast group (or no longer)
    pub fn join(&mut self, iface: u16, group: Ipv4, on: bool) {
        self.groups.retain(|g| *g != (iface, group));
        if on {
            self.groups.push((iface, group));
        }
    }
    /// the interface listens to this group
    pub fn joined(&self, iface: u16, group: Ipv4) -> bool {
        self.groups.contains(&(iface, group))
    }

    /// the routing table: connected and local routes of interfaces that are up, then the
    /// usable static routes, then the routes of routing protocols over interfaces that are up
    pub fn rib(&self, up: &dyn Fn(u16) -> bool) -> Vec<Route> {
        let mut v: Vec<Route> = Vec::new();
        for (i, f) in self.ifaces.iter().enumerate() {
            if !up(i as u16) {
                continue;
            }
            for p in &f.addrs {
                v.push(Route { prefix: p.trunc(), via: None, iface: Some(i as u16), src: RouteSrc::Connected, ad: 0, metric: 0 });
                if p.len < 32 {
                    v.push(Route { prefix: Prefix4 { addr: p.addr, len: 32 }, via: None, iface: Some(i as u16), src: RouteSrc::Local, ad: 0, metric: 0 });
                }
            }
        }
        let connected = v.clone();
        let mut pending: Vec<&Route> = Vec::new();
        for s in &self.statics {
            match (s.iface, s.via) {
                (Some(i), _) if up(i) => v.push(s.clone()),
                (Some(_), _) => {}
                (None, Some(gw)) => match best(&connected, gw) {
                    Some(c) => v.push(Route { iface: c.iface, ..s.clone() }),
                    None => pending.push(s),
                },
                (None, None) => {}
            }
        }
        // the routing protocols' routes (a static next hop may lie behind one of them)
        for r in &self.dynamic {
            if r.iface.is_some_and(up) {
                v.push(r.clone());
            }
        }
        // a next hop behind another static route (one level)
        let direct = v.clone();
        for s in pending {
            if let Some(gw) = s.via
                && let Some(r) = best(&direct, gw)
                && r.src != RouteSrc::Local
            {
                v.push(Route { iface: r.iface, ..s.clone() });
            }
        }
        v
    }

    /// where a packet to `dst` goes
    pub fn lookup(&self, up: &dyn Fn(u16) -> bool, dst: Ipv4) -> Option<NextHop> {
        let mut c = self.rib_cache.borrow_mut();
        let same = c.up.len() == self.ifaces.len()
            && c.statics == self.statics
            && c.dyn_gen == self.dyn_gen
            && self.ifaces.iter().enumerate().all(|(i, f)| c.up[i] == up(i as u16) && c.addrs[i] == f.addrs);
        if !same {
            c.up = (0..self.ifaces.len()).map(|i| up(i as u16)).collect();
            c.addrs = self.ifaces.iter().map(|f| f.addrs.clone()).collect();
            c.statics = self.statics.clone();
            c.dyn_gen = self.dyn_gen;
            c.rib = self.rib(up);
        }
        let rib = &c.rib;
        let r = best(rib, dst)?;
        // equal-cost routes of a routing protocol: one per destination address
        let r = if matches!(r.src, RouteSrc::Rip | RouteSrc::Ospf) {
            let same: Vec<&Route> = rib.iter().filter(|x| x.prefix == r.prefix && x.src == r.src && x.ad == r.ad && x.metric == r.metric).collect();
            same.get(ecmp_pick(dst, same.len())).copied().unwrap_or(r)
        } else {
            r
        };
        let iface = r.iface?;
        let gw = match r.via {
            None => dst,
            Some(via) => {
                // a recursive static route: the next hop of the next hop
                match self.ifaces.get(iface as usize) {
                    Some(f) if f.on_link(via) => via,
                    _ => match best(rib, via) {
                        Some(rr) if rr.src != RouteSrc::Local => rr.via.unwrap_or(via),
                        _ => via,
                    },
                }
            }
        };
        Some(NextHop { iface, gw, src: r.src })
    }
}

/// which of `n` equal-cost routes a destination takes: the same one every time
pub fn ecmp_pick(dst: Ipv4, n: usize) -> usize {
    if n < 2 {
        return 0;
    }
    let x = dst.to_u32();
    let h = x ^ (x >> 16);
    ((h ^ (h >> 8)) as usize) % n
}

/// the best route for an address: longest prefix, then distance, then metric, then order
fn best(routes: &[Route], ip: Ipv4) -> Option<&Route> {
    let mut b: Option<&Route> = None;
    for r in routes {
        if !r.prefix.contains(ip) {
            continue;
        }
        let better = match b {
            None => true,
            Some(c) => (r.prefix.len, core::cmp::Reverse(r.ad), core::cmp::Reverse(r.metric)) > (c.prefix.len, core::cmp::Reverse(c.ad), core::cmp::Reverse(c.metric)),
        };
        if better {
            b = Some(r);
        }
    }
    b
}
