// ip6 -- the IPv6 part of a device: per interface its addresses (link-local, configured,
// autoconfigured) with their states and lifetimes and the settings of Neighbor Discovery and of
// router advertisements; the neighbor cache, the default router list, static routes and the
// routing table they make; what the views and the command lines read; Ip6Set (Action::Ip6)
//
//   Ip6                    IpStack::v6: ifs (one If6 per IPv4 interface number), routing
//                          ("ipv6 unicast-routing", net.ipv6.conf.all.forwarding), statics,
//                          neigh, routers, the ICMPv6 limiter, fragments and path MTUs
//   If6                    enabled, the configured addresses (AddrCfg) and the link-local one,
//                          autoconfiguration, RaCfg; at run time the addresses (Addr6 with
//                          AddrState and lifetimes) and what routers advertised (on-link
//                          prefixes, hop limit, MTU)
//   Neigh / NState         the neighbor cache of RFC 4861 7.3.2: INCOMPLETE, REACHABLE, STALE,
//                          DELAY, PROBE (static entries: fixed)
//   Ip6::rib(up, now)      connected and local routes of the interfaces that are up, on-link
//                          prefixes and default routers from advertisements (hosts), static
//                          routes (next hop on a connected network, a link-local one with its
//                          interface, one level of recursion)
//   Ip6::lookup(..)        longest prefix, then distance, then metric; a default router that is
//                          not INCOMPLETE before one that is (RFC 4861 6.3.6); a link-local
//                          destination is on the link of the interface it is given with
//   Ip6::joined(i, group)  the groups an interface listens to: all-nodes, all-routers on
//                          routers, the solicited-node group of every address (RFC 4291 2.8)
//   Ip6Set                 one change of the configuration (Engine::ip6_apply in nd.rs)
//
// The default (a decision for existing projects): IPv6 is off until a device is configured
// for it -- IOS-like devices per interface ("ipv6 enable", "ipv6 address ..."), Linux devices
// with net.ipv6.conf.all.disable_ipv6 = 0 (or per interface); so projects without IPv6 send no
// IPv6 frame and keep every report as before.  Linux turns every interface on with "all"
// (lo with ::1, the others with their EUI-64 link-local address, router solicitations and
// SLAAC unless forwarding is on); interfaces made later follow "all".  Turning it off again
// takes every IPv6 address of the interface away, as the kernel does.
//
// Engine side: l3v6.rs (input, delivery, forwarding, output, ICMPv6 errors), nd.rs (Neighbor
// Discovery, duplicate address detection, SLAAC, router advertisements, the timers),
// jobs6.rs (ping and traceroute), trace6.rs (the notes).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use base::rng::Pcg32;
use netpkt::ipv6::Reassembly6;
use netpkt::{Ipv6, Mac, Prefix6};

use crate::ip::Origin;
use crate::kernel::{Ns, MS, SEC};
use crate::trace::HopId;

/// node timer slots of the IPv6 part: the look at the interfaces (same time), the neighbor
/// cache, addresses (DAD and lifetimes) with prefixes and routers, router solicitations and
/// advertisements, fragments and path MTUs; 11 is kept free
pub const SLOT_V6_KICK: u32 = 6;
pub const SLOT_V6_ND: u32 = 7;
pub const SLOT_V6_ADDR: u32 = 8;
pub const SLOT_V6_RA: u32 = 9;
pub const SLOT_V6_REASM: u32 = 10;
pub const SLOT_V6_LAST: u32 = 11;

/// RFC 4861 10: protocol constants
pub const MAX_RTR_SOLICITATION_DELAY: Ns = SEC;
pub const RTR_SOLICITATION_INTERVAL: Ns = 4 * SEC;
pub const MAX_RTR_SOLICITATIONS: u8 = 3;
pub const MAX_INITIAL_RTR_ADVERT_INTERVAL: Ns = 16 * SEC;
pub const MAX_INITIAL_RTR_ADVERTISEMENTS: u8 = 3;
pub const MIN_DELAY_BETWEEN_RAS: Ns = 3 * SEC;
pub const MAX_RA_DELAY_TIME: Ns = 500 * MS;
pub const MAX_MULTICAST_SOLICIT: u8 = 3;
pub const MAX_UNICAST_SOLICIT: u8 = 3;
pub const REACHABLE_TIME: Ns = 30 * SEC;
pub const RETRANS_TIMER: Ns = SEC;
pub const DELAY_FIRST_PROBE_TIME: Ns = 5 * SEC;
/// RFC 4862 5.5.3 (e): the two hours a valid lifetime is not cut below
pub const TWO_HOURS: Ns = 7200 * SEC;
/// RFC 8200 4.5: fragments are given up after 60 s
pub const REASM_TIME6: Ns = 60 * SEC;
/// RFC 8201 4: a learned path MTU is tried again after 10 minutes
pub const PMTU_AGE: Ns = 600 * SEC;
/// the lifetimes IOS advertises for a prefix (30 and 7 days)
pub const VALID_DEFAULT: u32 = 2_592_000;
pub const PREFERRED_DEFAULT: u32 = 604_800;

/// how an address came to its interface
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin6 {
    /// fe80::/64 with the interface's EUI-64 (RFC 4291 2.5.6, appendix A)
    LinkLocal,
    /// "ipv6 address FE80::1 link-local"
    LinkLocalManual,
    /// configured with its interface identifier
    Manual,
    /// configured prefix + EUI-64 ("ipv6 address 2001:DB8::/64 eui-64")
    Eui64,
    /// stateless autoconfiguration from a router advertisement (RFC 4862)
    Slaac,
    /// ::1 of a Linux lo
    Loopback,
}

/// where an address stands (RFC 4862 2)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddrState {
    /// duplicate address detection runs: not used yet
    Tentative,
    Preferred,
    /// its preferred lifetime ended: kept for what uses it, not for new traffic
    Deprecated,
    /// DAD found another node with it: never used
    Duplicate,
}

impl AddrState {
    /// may be used as a source or accepted as a destination
    pub fn usable(self) -> bool {
        matches!(self, AddrState::Preferred | AddrState::Deprecated)
    }
}

/// an address of an interface at run time
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addr6 {
    pub addr: Ipv6,
    pub len: u8,
    pub origin: Origin6,
    pub state: AddrState,
    /// SLAAC: when it stops being preferred / valid (None: for ever)
    pub preferred_until: Option<Ns>,
    pub valid_until: Option<Ns>,
    /// an anycast address ("ipv6 address ... anycast"): no DAD, override 0 in advertisements
    pub anycast: bool,
    /// DAD: when the next step is due (a probe, or the end), probes still to send
    pub(crate) dad_at: Option<Ns>,
    pub(crate) dad_left: u8,
    /// who answered DAD (a duplicate)
    pub dup_mac: Option<Mac>,
}

impl Addr6 {
    pub fn prefix(&self) -> Prefix6 {
        Prefix6 { addr: self.addr, len: self.len }
    }
}

/// a configured address ("ipv6 address", "ip -6 addr add")
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddrCfg {
    /// the address (with eui64: the prefix) and its prefix length
    pub prefix: Prefix6,
    pub eui64: bool,
    pub anycast: bool,
}

/// a prefix as a router advertises it ("ipv6 nd prefix")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixCfg {
    pub prefix: Prefix6,
    /// seconds (netpkt::icmpv6::INFINITE for ever)
    pub valid: u32,
    pub preferred: u32,
    pub on_link: bool,
    pub auto: bool,
    /// "no-advertise": left out of the advertisements
    pub hidden: bool,
}

/// what a router advertises on an interface (RFC 4861 6.2.1; the defaults of IOS)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaCfg {
    /// "ipv6 nd ra suppress": no unsolicited advertisements; `all`: no solicited ones either
    pub suppress: bool,
    pub suppress_all: bool,
    pub max_interval: Ns,
    /// None: three quarters of the maximum (IOS)
    pub min_interval: Option<Ns>,
    /// seconds (0: not a default router)
    pub lifetime: u16,
    pub managed: bool,
    pub other: bool,
    /// the MTU option (IOS sends it)
    pub mtu: bool,
    /// advertised reachable time in ms (0: unspecified)
    pub reachable: u32,
    pub prefixes: Vec<PrefixCfg>,
}

impl Default for RaCfg {
    fn default() -> RaCfg {
        RaCfg { suppress: false, suppress_all: false, max_interval: 200 * SEC, min_interval: None, lifetime: 1800, managed: false, other: false, mtu: true, reachable: 0, prefixes: Vec::new() }
    }
}

impl RaCfg {
    pub fn min(&self) -> Ns {
        self.min_interval.unwrap_or(self.max_interval / 4 * 3)
    }
}

/// a prefix on the link, learned from a router advertisement (RFC 4861 5.1, prefix list)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnLink {
    pub prefix: Prefix6,
    /// None: for ever
    pub until: Option<Ns>,
}

/// the IPv6 side of an interface
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct If6 {
    /// IPv6 is on ("ipv6 enable", an address, autoconfig; Linux disable_ipv6 = 0)
    pub enabled: bool,
    /// "ipv6 enable" itself (IOS writes it back)
    pub explicit: bool,
    /// "ipv6 address X link-local" (else the EUI-64 one)
    pub ll_manual: Option<Ipv6>,
    pub cfg: Vec<AddrCfg>,
    /// SLAAC from advertisements ("ipv6 address autoconfig", Linux autoconf)
    pub autoconf: bool,
    /// a default route from advertisements ("autoconfig default"; Linux accept_ra)
    pub default_route: bool,
    /// advertisements are taken (Linux accept_ra = 1 when not forwarding; IOS with autoconfig)
    pub accept_ra: bool,
    pub ra: RaCfg,
    /// "ipv6 mtu" (None: the link's)
    pub mtu: Option<u16>,
    /// DAD probes per address (RFC 4862 DupAddrDetectTransmits; "ipv6 nd dad attempts")
    pub dad_transmits: u8,
    /// BaseReachableTime and RetransTimer (RFC 4861 6.3.2; "ipv6 nd reachable-time",
    /// "ipv6 nd ns-interval")
    pub base_reachable: Ns,
    pub retrans: Ns,
    // ---- run time
    pub addrs: Vec<Addr6>,
    /// IPv6 runs on the interface (enabled and the interface up), as last seen
    pub(crate) up: bool,
    /// ReachableTime: the base times a random 0.5 to 1.5
    pub(crate) reachable: Ns,
    /// router solicitations still to send and when the next goes; they started since IPv6 came
    /// up
    pub(crate) rs_left: u8,
    pub(crate) rs_at: Option<Ns>,
    pub(crate) rs_started: bool,
    /// routers: the next multicast advertisement, initial ones left, the last one sent
    pub(crate) ra_at: Option<Ns>,
    pub(crate) ra_initial: u8,
    pub(crate) ra_last: Option<Ns>,
    /// the next advertisement answers a solicitation
    pub(crate) ra_solicited: bool,
    /// the interface advertises (routing, IPv6 up, link-local ready, not suppressed)
    pub(crate) advertising: bool,
    /// learned from advertisements: the hop limit, the link MTU, on-link prefixes
    pub hop_limit: Option<u8>,
    pub link_mtu: Option<u16>,
    pub prefixes: Vec<OnLink>,
    /// the advertisements this interface heard (for "show ipv6 routers"): (router, when,
    /// lifetime, prefixes)
    pub heard: Vec<(Ipv6, Ns, u16, Vec<netpkt::icmpv6::PrefixInfo>)>,
}

impl If6 {
    pub fn new(linux: bool) -> If6 {
        If6 {
            enabled: false,
            explicit: false,
            ll_manual: None,
            cfg: Vec::new(),
            autoconf: linux,
            default_route: linux,
            accept_ra: linux,
            ra: RaCfg::default(),
            mtu: None,
            dad_transmits: 1,
            base_reachable: REACHABLE_TIME,
            retrans: RETRANS_TIMER,
            addrs: Vec::new(),
            up: false,
            reachable: REACHABLE_TIME,
            rs_left: 0,
            rs_at: None,
            rs_started: false,
            ra_at: None,
            ra_solicited: false,
            ra_initial: 0,
            ra_last: None,
            advertising: false,
            hop_limit: None,
            link_mtu: None,
            prefixes: Vec::new(),
            heard: Vec::new(),
        }
    }
    /// the link-local address that is usable
    pub fn link_local(&self) -> Option<Ipv6> {
        self.addrs.iter().find(|a| a.addr.is_link_local() && a.state.usable()).map(|a| a.addr)
    }
    /// any address, also tentative or duplicate ones
    pub fn has(&self, ip: Ipv6) -> bool {
        self.addrs.iter().any(|a| a.addr == ip)
    }
    /// an address that may be used (not tentative, not duplicate)
    pub fn owns(&self, ip: Ipv6) -> bool {
        self.addrs.iter().any(|a| a.addr == ip && a.state.usable())
    }
    pub fn addr(&self, ip: Ipv6) -> Option<&Addr6> {
        self.addrs.iter().find(|a| a.addr == ip)
    }
    /// `ip` is on the link: link-local, in the prefix of an address, or an on-link prefix
    pub fn on_link(&self, ip: Ipv6) -> bool {
        ip.is_link_local() || self.addrs.iter().any(|a| a.state != AddrState::Duplicate && a.len < 128 && a.prefix().contains(ip)) || self.prefixes.iter().any(|p| p.prefix.contains(ip))
    }
}

/// the reachability of a neighbor (RFC 4861 7.3.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NState {
    Incomplete,
    Reachable,
    Stale,
    Delay,
    Probe,
}

impl NState {
    /// iproute2's word
    pub fn linux(self) -> &'static str {
        match self {
            NState::Incomplete => "INCOMPLETE",
            NState::Reachable => "REACHABLE",
            NState::Stale => "STALE",
            NState::Delay => "DELAY",
            NState::Probe => "PROBE",
        }
    }
    /// the IOS-like abbreviation of "show ipv6 neighbors"
    pub fn ios(self) -> &'static str {
        match self {
            NState::Incomplete => "INCMP",
            NState::Reachable => "REACH",
            NState::Stale => "STALE",
            NState::Delay => "DELAY",
            NState::Probe => "PROBE",
        }
    }
}

/// a packet waiting for address resolution
#[derive(Clone, Debug)]
pub struct Pending6 {
    pub pkt: Vec<u8>,
    pub origin: Origin,
    pub cause: Option<HopId>,
}

/// an entry of the neighbor cache
#[derive(Clone, Debug)]
pub struct Neigh {
    pub mac: Option<Mac>,
    pub state: NState,
    /// the neighbor is a router (the R flag of its advertisements)
    pub router: bool,
    /// "ipv6 neighbor", "ip -6 neigh add": never ages
    pub fixed: bool,
    /// when it entered its state (Reachable: the last confirmation)
    pub at: Ns,
    pub(crate) probes: u8,
    /// the next timer of the entry (a retransmission, the end of DELAY or REACHABLE)
    pub(crate) next: Option<Ns>,
    pub(crate) queue: Vec<Pending6>,
}

/// a router of the default router list (RFC 4861 6.3.4)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefRouter {
    pub iface: u16,
    pub addr: Ipv6,
    pub until: Ns,
    /// default router preference (RFC 4191: 0 medium, 1 high, 3 low)
    pub prf: u8,
}

/// where an IPv6 route came from
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Src6 {
    Connected,
    Local,
    Static,
    /// an on-link prefix of a router advertisement (IOS "NDp")
    RaPrefix,
    /// a default router of advertisements (IOS "ND")
    RaDefault,
}

impl Src6 {
    /// the code of "show ipv6 route"
    pub fn code(self) -> &'static str {
        match self {
            Src6::Connected => "C",
            Src6::Local => "L",
            Src6::Static => "S",
            Src6::RaPrefix => "NDp",
            Src6::RaDefault => "ND",
        }
    }
}

/// an IPv6 route
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route6 {
    /// the destination (network form)
    pub prefix: Prefix6,
    pub via: Option<Ipv6>,
    pub iface: Option<u16>,
    pub src: Src6,
    pub ad: u8,
    /// the metric Linux shows (256 for the kernel's routes, 1024 for the others)
    pub metric: u32,
    /// routes from advertisements end
    pub expires: Option<Ns>,
}

/// the next step of an IPv6 packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextHop6 {
    pub iface: u16,
    /// the neighbor (the next router, or the destination itself)
    pub gw: Ipv6,
    pub src: Src6,
}

/// how a device limits its ICMPv6 errors (RFC 4443 2.4 (f))
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit6 {
    /// a token bucket of 10, one more every 100 ms ("ipv6 icmp error-interval 100 10")
    Ios,
    /// 50 at once and 1000 a second for the device (icmp_msgs_burst / per_sec) and per
    /// destination one every 1 s shifted by the route's prefix length (net.ipv6.icmp.ratelimit:
    /// 250 ms behind a /64, 62 ms behind the default route), six saved up; Packet Too Big only
    /// counts for the device
    Linux,
}

pub const IOS_ERR_INTERVAL: Ns = 100 * MS;
pub const IOS_ERR_BUCKET: u32 = 10;

/// the state of the limiter
#[derive(Clone, Debug, Default)]
pub(crate) struct Rate6 {
    pub(crate) tokens: u32,
    pub(crate) at: Ns,
    pub(crate) started: bool,
    /// Linux: destination -> (credit, when it was counted)
    pub(crate) peers: BTreeMap<Ipv6, (Ns, Ns)>,
}

/// the IPv6 part of a device
#[derive(Clone, Debug)]
pub struct Ip6 {
    pub ifs: Vec<If6>,
    /// forwards between interfaces and acts as a router ("ipv6 unicast-routing";
    /// net.ipv6.conf.all.forwarding = 1)
    pub routing: bool,
    pub statics: Vec<Route6>,
    pub neigh: BTreeMap<(u16, Ipv6), Neigh>,
    pub routers: Vec<DefRouter>,
    /// the hop limit of packets it makes (64 on both)
    pub hop_limit: u8,
    /// a Linux device: net.ipv6.conf.all.disable_ipv6 = 0 (interfaces made later too)
    pub all_on: bool,
    /// Linux conventions (accept_ra, the ICMPv6 limiter, queues while ND asks, fe80::/64 in
    /// the table); else IOS-like
    pub linux: bool,
    pub limit: Limit6,
    /// packets that may wait for one neighbor (Linux 101; IOS 1, RFC 4861 7.2.2 asks for one)
    pub queue: usize,
    pub(crate) rate: Rate6,
    /// fragments: (source, destination, identification) -> (parts, when they are given up)
    pub(crate) reasm: BTreeMap<(Ipv6, Ipv6, u32), (Reassembly6, Ns)>,
    /// path MTUs learned from Packet Too Big: destination -> (MTU, when it is forgotten)
    pub pmtu: BTreeMap<Ipv6, (u16, Ns)>,
    /// the device's own random numbers for IPv6 (DAD delays, reachable times, RA intervals),
    /// apart from its other ones so IPv6 changes nothing else
    pub(crate) rng: Option<Pcg32>,
    pub(crate) frag_id: u32,
    /// the time each timer slot is set for (no second event for the same time)
    pub(crate) armed: [Option<Ns>; 6],
}

impl Ip6 {
    pub fn new(linux: bool) -> Ip6 {
        Ip6 {
            ifs: Vec::new(),
            routing: false,
            statics: Vec::new(),
            neigh: BTreeMap::new(),
            routers: Vec::new(),
            hop_limit: 64,
            all_on: false,
            linux,
            limit: if linux { Limit6::Linux } else { Limit6::Ios },
            queue: if linux { 101 } else { 1 },
            rate: Rate6::default(),
            reasm: BTreeMap::new(),
            pmtu: BTreeMap::new(),
            rng: None,
            frag_id: 0,
            armed: [None; 6],
        }
    }
    /// something about IPv6 is configured on the device (else it does nothing at all)
    pub fn used(&self) -> bool {
        self.all_on || self.routing || !self.statics.is_empty() || self.ifs.iter().any(|f| f.enabled)
    }
    /// the IPv6 side of interface `i` (made when missing)
    pub fn if_mut(&mut self, i: u16) -> &mut If6 {
        let linux = self.linux;
        while self.ifs.len() <= i as usize {
            let mut f = If6::new(linux);
            f.enabled = self.all_on;
            self.ifs.push(f);
        }
        &mut self.ifs[i as usize]
    }
    pub fn iface(&self, i: u16) -> Option<&If6> {
        self.ifs.get(i as usize)
    }
    /// IPv6 is configured on interface `i`
    pub fn on(&self, i: u16) -> bool {
        self.ifs.get(i as usize).is_some_and(|f| f.enabled)
    }
    /// IPv6 runs on interface `i` now
    pub fn running(&self, i: u16) -> bool {
        self.ifs.get(i as usize).is_some_and(|f| f.up)
    }
    /// an address of the device that may be used, and its interface
    pub fn find(&self, ip: Ipv6) -> Option<u16> {
        self.ifs.iter().position(|f| f.up && f.owns(ip)).map(|i| i as u16)
    }
    /// the interface has the group (all-nodes, all-routers on routers, solicited-node groups
    /// of its addresses, tentative ones included: DAD listens there)
    pub fn joined(&self, i: u16, group: Ipv6) -> bool {
        let Some(f) = self.ifs.get(i as usize).filter(|f| f.up) else {
            return false;
        };
        group == Ipv6::ALL_NODES || (group == Ipv6::ALL_ROUTERS && self.routing) || (group.is_solicited_node() && f.addrs.iter().any(|a| a.state != AddrState::Duplicate && a.addr.solicited_node() == group))
    }
    /// the groups an interface listens to, in order (for "show ipv6 interface")
    pub fn groups(&self, i: u16) -> Vec<Ipv6> {
        let mut v = alloc::vec![Ipv6::ALL_NODES];
        if self.routing {
            v.push(Ipv6::ALL_ROUTERS);
        }
        if let Some(f) = self.ifs.get(i as usize) {
            for a in &f.addrs {
                let g = a.addr.solicited_node();
                if a.state != AddrState::Duplicate && a.origin != Origin6::Loopback && !v.contains(&g) {
                    v.push(g);
                }
            }
        }
        v
    }
    /// forgets what was learned (power off): neighbors, routers, prefixes, autoconfigured
    /// addresses, fragments, path MTUs; the configuration stays
    pub fn reset(&mut self) {
        self.neigh.retain(|_, n| n.fixed);
        self.routers.clear();
        self.reasm.clear();
        self.pmtu.clear();
        self.rate = Rate6::default();
        self.armed = [None; 6];
        for f in self.ifs.iter_mut() {
            f.up = false;
            f.addrs.retain(|a| a.origin != Origin6::Slaac);
            f.prefixes.clear();
            f.heard.clear();
            f.hop_limit = None;
            f.link_mtu = None;
            f.rs_at = None;
            f.ra_at = None;
            f.ra_last = None;
            f.advertising = false;
        }
    }
    /// when the next fragment buffer is given up or path MTU forgotten
    pub(crate) fn reasm_next(&self) -> Option<Ns> {
        self.reasm.values().map(|e| e.1).chain(self.pmtu.values().map(|p| p.1)).min()
    }
    /// Linux disable_ipv6 = 1: the interface loses every IPv6 address, the added ones too, and
    /// what advertisements taught it (the kernel's addrconf_ifdown)
    pub fn disable_linux(&mut self, i: u16) {
        if let Some(f) = self.ifs.get_mut(i as usize) {
            f.cfg.clear();
            f.addrs.clear();
            f.prefixes.clear();
            f.heard.clear();
            f.hop_limit = None;
            f.link_mtu = None;
        }
        self.routers.retain(|r| r.iface != i);
    }
    /// an interface that goes away takes its IPv6 configuration and state along
    pub fn forget(&mut self, i: u16) {
        if let Some(f) = self.ifs.get_mut(i as usize) {
            let linux = self.linux;
            *f = If6::new(linux);
        }
        self.neigh.retain(|(j, _), _| *j != i);
        self.routers.retain(|r| r.iface != i);
        self.statics.retain(|r| r.iface != Some(i));
    }

    /// the routing table: connected and local routes of the interfaces that run IPv6, the
    /// on-link prefixes and default routers learned from advertisements, the usable static
    /// routes
    pub fn rib(&self, up: &dyn Fn(u16) -> bool, now: Ns) -> Vec<Route6> {
        let mut v: Vec<Route6> = Vec::new();
        for (i, f) in self.ifs.iter().enumerate() {
            let i = i as u16;
            if !f.up || !up(i) {
                continue;
            }
            if self.linux && f.addrs.iter().any(|a| a.addr.is_link_local()) {
                let ll = Prefix6 { addr: Ipv6::from_u128(0xFE80u128 << 112), len: 64 };
                v.push(Route6 { prefix: ll, via: None, iface: Some(i), src: Src6::Connected, ad: 0, metric: 256, expires: None });
            }
            for a in &f.addrs {
                if a.state == AddrState::Duplicate || a.addr.is_link_local() {
                    continue;
                }
                let p = a.prefix().trunc();
                if a.len < 128 && !v.iter().any(|r| r.prefix == p && r.iface == Some(i) && r.src == Src6::Connected) && a.origin != Origin6::Slaac {
                    v.push(Route6 { prefix: p, via: None, iface: Some(i), src: Src6::Connected, ad: 0, metric: 256, expires: None });
                }
                if a.state.usable() {
                    v.push(Route6 { prefix: Prefix6 { addr: a.addr, len: 128 }, via: None, iface: Some(i), src: Src6::Local, ad: 0, metric: 0, expires: None });
                }
            }
            for p in &f.prefixes {
                if p.until.is_none_or(|t| t > now) && !v.iter().any(|r| r.prefix == p.prefix && r.iface == Some(i) && r.src == Src6::Connected) {
                    v.push(Route6 { prefix: p.prefix, via: None, iface: Some(i), src: Src6::RaPrefix, ad: if self.linux { 0 } else { 2 }, metric: 256, expires: p.until });
                }
            }
        }
        for r in &self.routers {
            let f = self.ifs.get(r.iface as usize);
            if r.until > now && f.is_some_and(|f| f.up && f.default_route) && up(r.iface) {
                let any = Prefix6 { addr: Ipv6::UNSPECIFIED, len: 0 };
                v.push(Route6 { prefix: any, via: Some(r.addr), iface: Some(r.iface), src: Src6::RaDefault, ad: 2, metric: 1024, expires: Some(r.until) });
            }
        }
        let direct = v.clone();
        let mut pending: Vec<&Route6> = Vec::new();
        for s in &self.statics {
            match (s.iface, s.via) {
                (Some(i), _) if self.running(i) && up(i) => v.push(*s),
                (Some(_), _) => {}
                (None, Some(gw)) => match best(&direct, gw) {
                    Some(c) if c.src != Src6::Local && c.via.is_none() => v.push(Route6 { iface: c.iface, ..*s }),
                    _ => pending.push(s),
                },
                (None, None) => {}
            }
        }
        // a next hop behind another route (one level)
        let one = v.clone();
        for s in pending {
            if let Some(gw) = s.via
                && let Some(r) = best(&one, gw)
                && r.src != Src6::Local
            {
                v.push(Route6 { iface: r.iface, ..*s });
            }
        }
        v
    }

    /// where a packet to `dst` goes; `oif`: the interface a link-local destination is on (or
    /// one that must be taken)
    pub fn lookup(&self, up: &dyn Fn(u16) -> bool, now: Ns, dst: Ipv6, oif: Option<u16>) -> Option<NextHop6> {
        if dst.is_link_local() || (dst.is_multicast() && dst.mc_scope() <= 2) {
            let i = oif?;
            return if self.running(i) && up(i) { Some(NextHop6 { iface: i, gw: dst, src: Src6::Connected }) } else { None };
        }
        let rib = self.rib(up, now);
        let cands: Vec<&Route6> = rib.iter().filter(|r| r.prefix.contains(dst) && oif.is_none_or(|o| r.iface == Some(o))).collect();
        let top = cands.iter().map(|r| (r.prefix.len, core::cmp::Reverse(r.ad), core::cmp::Reverse(r.metric))).max()?;
        let same: Vec<&Route6> = cands.into_iter().filter(|r| (r.prefix.len, core::cmp::Reverse(r.ad), core::cmp::Reverse(r.metric)) == top).collect();
        // default routers: one that is (probably) reachable first (RFC 4861 6.3.6)
        let r = same
            .iter()
            .find(|r| r.src != Src6::RaDefault || r.via.is_some_and(|g| r.iface.is_some_and(|i| self.neigh.get(&(i, g)).is_none_or(|n| n.state != NState::Incomplete))))
            .or(same.first())?;
        let iface = r.iface?;
        let gw = match r.via {
            None => dst,
            Some(via) if via.is_link_local() || self.ifs.get(iface as usize).is_some_and(|f| f.on_link(via)) => via,
            Some(via) => match best(&rib, via) {
                Some(rr) if rr.src != Src6::Local => rr.via.unwrap_or(via),
                _ => via,
            },
        };
        Some(NextHop6 { iface, gw, src: r.src })
    }
}

/// the best route for an address: longest prefix, then distance, then metric
fn best(routes: &[Route6], ip: Ipv6) -> Option<&Route6> {
    let mut b: Option<&Route6> = None;
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

/// one change of the IPv6 configuration (Action::Ip6; the command lines go through it)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ip6Set {
    /// "ipv6 unicast-routing", net.ipv6.conf.all.forwarding
    Routing(bool),
    /// "ipv6 enable" (IOS) / disable_ipv6 = 0 of one interface (Linux)
    Enable { iface: String, on: bool },
    /// Linux net.ipv6.conf.all.disable_ipv6 = 0: every interface, and the ones made later
    All(bool),
    /// a configured address; `add: false` removes it
    Addr { iface: String, cfg: AddrCfg, add: bool },
    /// "ipv6 address FE80::1 link-local" (None: back to the EUI-64 one)
    LinkLocal { iface: String, addr: Option<Ipv6> },
    /// "ipv6 address autoconfig [default]" (Linux autoconf / accept_ra)
    Autoconf { iface: String, on: bool, default_route: bool },
    /// "no ipv6 address": every configured address, the link-local one and autoconfig go
    ClearAddrs { iface: String },
    /// the advertisements of an interface ("ipv6 nd ra ...", "ipv6 nd prefix ...")
    Ra { iface: String, ra: RaCfg },
    /// "ipv6 mtu"
    Mtu { iface: String, mtu: Option<u16> },
    /// "ipv6 nd dad attempts", "ipv6 nd reachable-time", "ipv6 nd ns-interval"
    Nd { iface: String, dad: Option<u8>, reachable: Option<Ns>, retrans: Option<Ns> },
    /// a static route; `add: false` removes the matching ones
    Route { route: Route6, add: bool },
    /// a static neighbor entry ("ipv6 neighbor", "ip -6 neigh add"); None removes it
    Neighbor { iface: String, ip: Ipv6, mac: Option<Mac> },
    /// forgets the learned neighbors ("clear ipv6 neighbors", "ip -6 neigh flush")
    ClearNeighbors { iface: Option<String> },
    /// the hop limit of the device's own packets
    HopLimit(u8),
}

