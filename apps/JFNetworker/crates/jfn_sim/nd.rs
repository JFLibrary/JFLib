// nd -- Neighbor Discovery (RFC 4861), duplicate address detection and stateless address
// autoconfiguration (RFC 4862) on the engine: IPv6 starting and stopping on an interface, the
// neighbor cache, router solicitations and advertisements, the IPv6 timers, Action::Ip6
//
//   ip6_kick(ni)               the interfaces of a device may have changed (a cable, an action,
//                              the power): a look at the same virtual time (SLOT_V6_KICK);
//                              nothing at all for a device without IPv6
//   ip6_check(ni)              per interface: IPv6 starts (the link-local address and the
//                              configured ones tentative, DAD after a random delay of up to 1 s,
//                              RFC 4862 5.4.2), stops (neighbors forgotten) or changes its role
//                              (advertisements on routers, solicitations on hosts once the
//                              link-local address is unique)
//   nd_input(..)               RS, RA, NS, NA after Nd::parse and Nd::check_ip; redirects are
//                              not taken (no device here sends them)
//   nd_send(ni, iface, gw, pkt, origin, cause)   address resolution and the states of the
//                              neighbor cache (RFC 4861 7.2.2, 7.3.3): INCOMPLETE with up to 3
//                              multicast solicitations a second apart (packets wait: Linux 101,
//                              IOS 1; then "address unreachable"), REACHABLE for the randomised
//                              ReachableTime (15 to 45 s), STALE until the next packet, DELAY
//                              for 5 s, PROBE with 3 unicast solicitations, then the entry goes
//   ip6_timer(ni, slot)        the look, the neighbor cache, DAD and lifetimes, RS and RA,
//                              fragments and path MTUs
//   ip6_apply(dev, Ip6Set)     Action::Ip6 (the command lines go through it)
//   ip6(dev) / ip6_routes(dev) / ip6_neighbors(dev) / addrs6(dev, iface)   views
//
// IOS-like routers (IPv6 routing on; a Linux device that forwards has no radvd and stays
// quiet) advertise on Ethernet interfaces whose link-local address is unique, unless "ipv6 nd
// ra suppress": the first three after at most 16 s each, then every MinRtrAdvInterval to
// MaxRtrAdvInterval (IOS: 150 to 200 s), with the device's hop limit, the router lifetime
// (1800 s), their MAC, the MTU and the prefix of every global address (30 days
// valid, 7 preferred, on-link and autonomous; "ipv6 nd prefix" changes or adds them).  A
// solicitation is answered to all-nodes after a random 0 to 500 ms, never sooner than 3 s
// after the last advertisement (RFC 4861 6.2.6).  Turning routing off sends one last
// advertisement with lifetime 0.  Hosts send up to three solicitations 4 s apart once the
// link-local address is unique, take advertisements (Linux unless forwarding; IOS with "ipv6
// address autoconfig"), keep the default routers (IOS only with "autoconfig default"), the
// on-link prefixes and the hop limit and MTU, and form an address from every autonomous /64
// prefix with the EUI-64 of their MAC; its lifetimes follow later advertisements with the
// two-hour rule of RFC 4862 5.5.3 (e).  Randomness comes from the device's own IPv6 stream,
// in whole milliseconds.  MLD reports are not sent (switches flood multicast anyway).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::rng::Pcg32;
use jfn_model::catalog::{CliStyle, Media};
use jfn_model::msg;
use netpkt::eth::ET_IPV6;
use netpkt::icmpv6::{self, Icmp6, Icmp6View, Nd, NdMsg, NdOpt, PrefixInfo, Ra};
use netpkt::ipv6::{Ipv6Hdr, Ipv6View, NH_ICMPV6};
use netpkt::{Ipv6, Mac, Prefix6};

use crate::action::ActErr;
use crate::ip::{IfKind, Origin};
use crate::ip6::*;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::trace::HopId;
use crate::trace6::{Note6, RaWhy};

/// a row of the neighbor cache for the views
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neigh6Row {
    pub iface: u16,
    pub ip: Ipv6,
    pub mac: Option<Mac>,
    pub state: NState,
    pub router: bool,
    pub fixed: bool,
    /// since it entered its state
    pub age: Ns,
}

/// an address of an interface for the views (the inspector, the command lines)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addr6View {
    pub addr: Ipv6,
    pub len: u8,
    pub origin: Origin6,
    pub state: AddrState,
    pub anycast: bool,
    /// what is left of the lifetimes (None: for ever)
    pub valid: Option<Ns>,
    pub preferred: Option<Ns>,
}

/// what a timer of the neighbor cache makes an entry do
enum NdStep {
    /// one more multicast solicitation (with the source of the first waiting packet)
    Ask(Option<Ipv6>),
    /// resolution failed: the entry and what waited go
    Fail,
    /// a new state (and a unicast probe with it)
    Moved(NState, NState, Option<Mac>, bool),
    /// one more unicast probe
    Probe(Option<Mac>),
    /// NUD gave up
    Gone,
}

/// seconds of a lifetime field as virtual time from now (None: infinite)
fn lifetime(now: Ns, s: u32) -> Option<Ns> {
    if s == icmpv6::INFINITE { None } else { Some(now + s as Ns * SEC) }
}

impl Engine {
    // ---------------------------------------------------------------- helpers

    /// a random number below `n` from the device's own IPv6 stream
    pub(crate) fn v6_rand(&mut self, ni: usize, n: u32) -> u32 {
        let (seed, dev) = (self.seed, self.dev_of(ni));
        match self.v6_mut(ni) {
            Some(v) => v.rng.get_or_insert_with(|| Pcg32::derive(seed ^ 0x6970_7636, dev as u64)).below(n.max(1)),
            None => 0,
        }
    }

    /// the MAC an interface's identifier comes from (loopbacks and serial lines take the first
    /// Ethernet port's, as IOS does)
    pub(crate) fn v6_mac(&self, ni: usize, i: u16) -> Mac {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return Mac::ZERO;
        };
        let first = || n.ports.iter().find(|p| p.spec.media.is_ethernet()).map(|p| p.mac).unwrap_or(n.base_mac);
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(i as usize)) else {
            return first();
        };
        match f.kind {
            IfKind::Port(p) if n.ports[p as usize].spec.media == Media::Serial => first(),
            IfKind::Loopback(_) => first(),
            _ if f.mac.is_zero() => first(),
            _ => f.mac,
        }
    }

    /// the interface is a serial line or a loopback (no link-layer addresses, no ND)
    fn v6_kind(&self, ni: usize, i: u16) -> (bool, bool, bool) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return (false, false, false);
        };
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(i as usize)) else {
            return (false, false, false);
        };
        let serial = matches!(f.kind, IfKind::Port(p) if n.ports[p as usize].spec.media == Media::Serial);
        let lo = matches!(f.kind, IfKind::Loopback(_));
        let linux_lo = lo && n.model.cli == CliStyle::Linux && f.name == "lo";
        (serial, lo, linux_lo)
    }

    fn if_name(&self, ni: usize, i: u16) -> String {
        self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default()
    }

    /// sets an IPv6 timer slot (None: none); the same time twice makes no second event
    pub(crate) fn arm6(&mut self, ni: usize, slot: u32, at: Option<Ns>) {
        let now = self.now;
        let k = (slot - SLOT_V6_KICK) as usize;
        let at = at.map(|t| t.max(now));
        let Some(v) = self.v6_mut(ni) else {
            return;
        };
        if v.armed[k] == at {
            return;
        }
        v.armed[k] = at;
        match at {
            Some(t) => self.set_timer(ni, slot, t),
            None => self.cancel_timer(ni, slot),
        }
    }

    /// the interfaces of node `ni` may have changed: IPv6 looks again at this moment
    pub(crate) fn ip6_kick(&mut self, ni: usize) {
        let active = self.v6(ni).is_some_and(|v| v.used() || v.ifs.iter().any(|f| f.up));
        if active {
            let now = self.now;
            self.arm6(ni, SLOT_V6_KICK, Some(now));
        }
    }
    pub(crate) fn ip6_kick_dev(&mut self, dev: u32) {
        if let Some(ni) = self.node_index(dev) {
            self.ip6_kick(ni);
        }
    }

    /// node timers SLOT_V6_KICK ..= SLOT_V6_LAST
    pub(crate) fn ip6_timer(&mut self, ni: usize, slot: u32) {
        if let Some(v) = self.v6_mut(ni) {
            v.armed[(slot - SLOT_V6_KICK) as usize] = None;
        }
        match slot {
            SLOT_V6_KICK => self.ip6_check(ni),
            SLOT_V6_ND => self.nd_timer(ni),
            SLOT_V6_ADDR => self.addr_timer(ni),
            SLOT_V6_RA => self.ra_timer(ni),
            SLOT_V6_REASM => self.reasm6_timer(ni),
            _ => {}
        }
    }

    // ---------------------------------------------------------------- interfaces

    /// starts and stops IPv6 on the interfaces as they are now
    pub(crate) fn ip6_check(&mut self, ni: usize) {
        let Some(st) = self.stack(ni) else {
            return;
        };
        let hidden: Vec<bool> = st.ifaces.iter().map(|f| f.hidden).collect();
        for (i, h) in hidden.iter().enumerate() {
            let i = i as u16;
            if let Some(v) = self.v6_mut(ni) {
                v.if_mut(i);
            }
            let want = !h && self.v6(ni).is_some_and(|v| v.on(i)) && self.iface_up(ni, i);
            let was = self.v6(ni).is_some_and(|v| v.running(i));
            match (was, want) {
                (false, true) => self.if6_start(ni, i),
                (true, false) => self.if6_stop(ni, i),
                (true, true) => self.if6_refresh(ni, i),
                _ => {}
            }
        }
        self.arm_all6(ni);
    }

    fn arm_all6(&mut self, ni: usize) {
        self.arm_nd(ni);
        self.arm_addr(ni);
        self.arm_ra(ni);
    }

    fn if6_start(&mut self, ni: usize, i: u16) {
        let r = self.v6_rand(ni, 1000) as Ns;
        let ll = {
            let mac = self.v6_mac(ni, i);
            let (_, _, linux_lo) = self.v6_kind(ni, i);
            let Some(v) = self.v6_mut(ni) else {
                return;
            };
            let f = v.if_mut(i);
            f.up = true;
            f.reachable = f.base_reachable / 1000 * (500 + r);
            f.rs_started = false;
            f.rs_left = 0;
            f.rs_at = None;
            f.ra_at = None;
            f.ra_solicited = false;
            f.advertising = false;
            if linux_lo { None } else { Some(f.ll_manual.unwrap_or(Ipv6::link_local_of(mac))) }
        };
        self.note6(ni, None, Note6::Up { iface: i, ll });
        self.sync_addrs(ni, i, true);
        self.if6_refresh(ni, i);
    }

    fn if6_stop(&mut self, ni: usize, i: u16) {
        let Some(v) = self.v6_mut(ni) else {
            return;
        };
        let f = v.if_mut(i);
        f.up = false;
        f.advertising = false;
        f.rs_at = None;
        f.rs_left = 0;
        f.ra_at = None;
        for a in f.addrs.iter_mut() {
            a.dad_at = None;
        }
        // IPv6 turned off (not just the link down): the addresses go, the link-local one too
        if !f.enabled {
            f.addrs.clear();
        }
        v.neigh.retain(|(j, _), n| *j != i || n.fixed);
        self.note6(ni, None, Note6::Down { iface: i });
    }

    /// the roles of a running interface: advertisements on a router, solicitations on a host
    fn if6_refresh(&mut self, ni: usize, i: u16) {
        let now = self.now;
        let (serial, lo, _) = self.v6_kind(ni, i);
        let Some(v) = self.v6(ni) else {
            return;
        };
        let (routing, linux) = (v.routing, v.linux);
        let Some(f) = v.iface(i) else {
            return;
        };
        let ll_ok = f.link_local().is_some();
        let ether = !serial && !lo;
        // a Linux device that forwards sends no advertisements (that would take radvd)
        let periodic = routing && !linux && ll_ok && ether && !f.ra.suppress && !f.ra.suppress_all;
        let was = f.advertising;
        let host_rs = !routing && f.accept_ra && ll_ok && ether && !f.rs_started;
        let (min, max) = (f.ra.min(), f.ra.max_interval);
        let first = if periodic && !was {
            let span = max.saturating_sub(min) / MS;
            let x = self.v6_rand(ni, span as u32) as Ns * MS;
            Some((min + x).min(MAX_INITIAL_RTR_ADVERT_INTERVAL))
        } else {
            None
        };
        let Some(v) = self.v6_mut(ni) else {
            return;
        };
        let f = v.if_mut(i);
        if let Some(d) = first {
            f.advertising = true;
            f.ra_initial = MAX_INITIAL_RTR_ADVERTISEMENTS - 1;
            f.ra_at = Some(f.ra_at.map_or(now + d, |t| t.min(now + d)));
        }
        let final_ra = !periodic && was;
        if final_ra {
            f.advertising = false;
            f.ra_at = None;
        }
        if host_rs {
            f.rs_started = true;
            f.rs_left = MAX_RTR_SOLICITATIONS;
            f.rs_at = Some(now);
        }
        if routing {
            f.rs_at = None;
            f.rs_left = 0;
        }
        // RFC 4861 6.2.5: an interface that stops being a router's says so once
        if final_ra && !routing && ll_ok && ether {
            self.ra_send(ni, i, false, true);
        }
        self.arm_ra(ni);
    }

    /// brings the addresses of an interface in line with its configuration (`restart`: every
    /// address goes through DAD again, as after the interface came up)
    pub(crate) fn sync_addrs(&mut self, ni: usize, i: u16, restart: bool) {
        let mac = self.v6_mac(ni, i);
        let (_, _, linux_lo) = self.v6_kind(ni, i);
        let Some(v) = self.v6_mut(ni) else {
            return;
        };
        let f = v.if_mut(i);
        let mut want: Vec<(Ipv6, u8, Origin6, bool)> = Vec::new();
        if linux_lo {
            want.push((Ipv6::LOCALHOST, 128, Origin6::Loopback, false));
        } else {
            match f.ll_manual {
                Some(a) => want.push((a, 64, Origin6::LinkLocalManual, false)),
                None => want.push((Ipv6::link_local_of(mac), 64, Origin6::LinkLocal, false)),
            }
            for c in &f.cfg {
                let a = if c.eui64 { Ipv6::eui64_in(c.prefix.trunc().addr, mac) } else { c.prefix.addr };
                want.push((a, c.prefix.len, if c.eui64 { Origin6::Eui64 } else { Origin6::Manual }, c.anycast));
            }
        }
        let mut gone: Vec<Ipv6> = Vec::new();
        f.addrs.retain(|a| {
            let keep = a.origin == Origin6::Slaac || want.iter().any(|w| w.0 == a.addr && w.2 == a.origin && w.1 == a.len);
            if !keep {
                gone.push(a.addr);
            }
            keep
        });
        let mut new: Vec<Ipv6> = Vec::new();
        for (a, len, origin, anycast) in want {
            if f.addrs.iter().any(|x| x.addr == a) {
                continue;
            }
            // the link-local address first
            let at = if a.is_link_local() { 0 } else { f.addrs.len() };
            f.addrs.insert(at, Addr6 { addr: a, len, origin, state: AddrState::Tentative, preferred_until: None, valid_until: None, anycast, dad_at: None, dad_left: 0, dup_mac: None });
            new.push(a);
        }
        let all: Vec<Ipv6> = f.addrs.iter().map(|a| a.addr).collect();
        for a in gone {
            self.note6(ni, None, Note6::AddrRemoved { iface: i, addr: a });
        }
        let running = self.ip6_running(ni, i) || self.v6(ni).is_some_and(|v| v.running(i));
        for a in all {
            let fresh = new.contains(&a);
            if fresh {
                let len = self.v6(ni).and_then(|v| v.iface(i)).and_then(|f| f.addr(a)).map(|x| x.len).unwrap_or(64);
                let origin = self.v6(ni).and_then(|v| v.iface(i)).and_then(|f| f.addr(a)).map(|x| x.origin).unwrap_or(Origin6::Manual);
                if origin != Origin6::LinkLocal {
                    self.note6(ni, None, Note6::AddrNew { iface: i, addr: Prefix6 { addr: a, len }, origin });
                }
            }
            if running && (restart || fresh) {
                self.start_dad(ni, i, a);
            }
        }
        self.arm_addr(ni);
    }

    /// an address becomes tentative and DAD starts after a random delay (no DAD for anycast
    /// addresses, loopbacks and "ipv6 nd dad attempts 0")
    fn start_dad(&mut self, ni: usize, i: u16, a: Ipv6) {
        let now = self.now;
        let (_, lo, _) = self.v6_kind(ni, i);
        let delay = self.v6_rand(ni, 1000) as Ns * MS;
        let Some(v) = self.v6_mut(ni) else {
            return;
        };
        let f = v.if_mut(i);
        let transmits = f.dad_transmits;
        let Some(x) = f.addrs.iter_mut().find(|x| x.addr == a) else {
            return;
        };
        if x.anycast || transmits == 0 || lo {
            x.state = if x.preferred_until.is_some_and(|t| t <= now) { AddrState::Deprecated } else { AddrState::Preferred };
            x.dad_at = None;
            return;
        }
        x.state = AddrState::Tentative;
        x.dad_left = transmits;
        x.dad_at = Some(now + delay);
        x.dup_mac = None;
        self.note6(ni, None, Note6::DadStart { iface: i, addr: a, delay });
    }

    // ---------------------------------------------------------------- addresses and lifetimes

    fn arm_addr(&mut self, ni: usize) {
        let Some(v) = self.v6(ni) else {
            return;
        };
        let mut next: Option<Ns> = None;
        let mut at = |t: Option<Ns>| {
            if let Some(t) = t {
                next = Some(next.map_or(t, |n| n.min(t)));
            }
        };
        for f in v.ifs.iter().filter(|f| f.up) {
            for a in &f.addrs {
                at(a.dad_at);
                if a.state == AddrState::Preferred {
                    at(a.preferred_until);
                }
                at(a.valid_until);
            }
            for p in &f.prefixes {
                at(p.until);
            }
        }
        for r in &v.routers {
            at(Some(r.until));
        }
        self.arm6(ni, SLOT_V6_ADDR, next);
    }

    fn addr_timer(&mut self, ni: usize) {
        let now = self.now;
        let n = self.v6(ni).map(|v| v.ifs.len()).unwrap_or(0);
        for i in 0..n as u16 {
            if !self.v6(ni).is_some_and(|v| v.running(i)) {
                continue;
            }
            // DAD steps
            let due: Vec<(Ipv6, u8)> = self.v6(ni).and_then(|v| v.iface(i)).map(|f| f.addrs.iter().filter(|a| a.state == AddrState::Tentative && a.dad_at.is_some_and(|t| t <= now)).map(|a| (a.addr, a.dad_left)).collect()).unwrap_or_default();
            for (a, left) in due {
                if left > 0 {
                    let retrans = self.v6(ni).and_then(|v| v.iface(i)).map(|f| f.retrans).unwrap_or(RETRANS_TIMER);
                    if let Some(x) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)).and_then(|f| f.addrs.iter_mut().find(|x| x.addr == a)) {
                        x.dad_left -= 1;
                        x.dad_at = Some(now + retrans);
                    }
                    self.dad_send(ni, i, a);
                } else {
                    let mut ll = false;
                    if let Some(x) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)).and_then(|f| f.addrs.iter_mut().find(|x| x.addr == a)) {
                        x.state = if x.preferred_until.is_some_and(|t| t <= now) { AddrState::Deprecated } else { AddrState::Preferred };
                        x.dad_at = None;
                        ll = x.addr.is_link_local();
                    }
                    self.note6(ni, None, Note6::DadOk { iface: i, addr: a });
                    if ll {
                        self.if6_refresh(ni, i);
                    }
                }
            }
            // lifetimes of autoconfigured addresses
            let Some(f) = self.v6(ni).and_then(|v| v.iface(i)) else {
                continue;
            };
            let deprecated: Vec<Ipv6> = f.addrs.iter().filter(|a| a.state == AddrState::Preferred && a.preferred_until.is_some_and(|t| t <= now)).map(|a| a.addr).collect();
            let expired: Vec<Ipv6> = f.addrs.iter().filter(|a| a.valid_until.is_some_and(|t| t <= now)).map(|a| a.addr).collect();
            let prefixes: Vec<Prefix6> = f.prefixes.iter().filter(|p| p.until.is_some_and(|t| t <= now)).map(|p| p.prefix).collect();
            if let Some(f) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)) {
                for x in f.addrs.iter_mut().filter(|x| deprecated.contains(&x.addr)) {
                    x.state = AddrState::Deprecated;
                }
                f.addrs.retain(|x| !expired.contains(&x.addr));
                f.prefixes.retain(|p| !prefixes.contains(&p.prefix));
            }
            for a in deprecated {
                self.note6(ni, None, Note6::AddrDeprecated { iface: i, addr: a });
            }
            for a in expired {
                self.note6(ni, None, Note6::AddrExpired { iface: i, addr: a });
            }
            for p in prefixes {
                self.note6(ni, None, Note6::PrefixGone { iface: i, prefix: p });
            }
        }
        // default routers whose lifetime ended
        let gone: Vec<DefRouter> = self.v6(ni).map(|v| v.routers.iter().filter(|r| r.until <= now).copied().collect()).unwrap_or_default();
        if let Some(v) = self.v6_mut(ni) {
            v.routers.retain(|r| r.until > now);
        }
        for r in gone {
            self.note6(ni, None, Note6::RouterGone { iface: r.iface, router: r.addr });
        }
        self.arm_addr(ni);
    }

    /// a DAD probe: a solicitation for the tentative address from the unspecified address to
    /// its solicited-node group, without a link-layer address
    fn dad_send(&mut self, ni: usize, i: u16, a: Ipv6) {
        let dst = a.solicited_node();
        let msg = Nd::ns(a, None).encode(Ipv6::UNSPECIFIED, dst);
        let pkt = Ipv6Hdr::new(Ipv6::UNSPECIFIED, dst, NH_ICMPV6, 255).packet(&msg);
        self.note6(ni, None, Note6::DadProbe { iface: i, addr: a });
        self.frame_out(ni, i, Mac::of_ipv6_multicast(dst), ET_IPV6, &pkt, None);
    }

    /// another node has a tentative address of interface `i`
    fn dad_dup(&mut self, ni: usize, i: u16, a: Ipv6, mac: Option<Mac>, cause: Option<HopId>) {
        if let Some(x) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)).and_then(|f| f.addrs.iter_mut().find(|x| x.addr == a)) {
            x.state = AddrState::Duplicate;
            x.dad_at = None;
            x.dup_mac = mac;
        } else {
            return;
        }
        self.note6(ni, cause, Note6::DadDup { iface: i, addr: a, mac });
        let name = self.if_name(ni, i);
        let ios = self.nodes.get(ni).and_then(|n| n.as_ref()).is_some_and(|n| n.model.cli == CliStyle::Ios);
        if ios {
            let up = format!("{}", a).to_uppercase();
            self.ios_log(ni, &format!("%IPV6_ND-4-DUPLICATE: Duplicate address {} on {}", up, name));
        } else {
            let by = mac.map(|m| format!(" used by {}", m)).unwrap_or_default();
            self.os_log(ni, 3, "kernel", &format!("IPv6: {}: IPv6 duplicate address {}{} detected!", name, a, by), cause);
        }
        self.arm_addr(ni);
    }

    // ---------------------------------------------------------------- input

    /// a Neighbor Discovery message for this device
    pub(crate) fn nd_input(&mut self, ni: usize, iface: u16, v: &Ipv6View, ic: &Icmp6View, hop: HopId) {
        let nd = match Nd::parse(ic) {
            Ok(n) => n,
            Err(e) => {
                self.note6(ni, Some(hop), Note6::NdBad { why: e });
                return;
            }
        };
        if let Some(e) = nd.check_ip(v.src(), v.dst(), v.hop_limit()) {
            self.note6(ni, Some(hop), Note6::NdBad { why: e });
            return;
        }
        match nd.msg {
            NdMsg::NeighborSolicit { target } => self.ns_input(ni, iface, v, &nd, target, hop),
            NdMsg::NeighborAdvert { router, solicited, override_, target } => self.na_input(ni, iface, &nd, (router, solicited, override_), target, hop),
            NdMsg::RouterSolicit => self.rs_input(ni, iface, v, &nd, hop),
            NdMsg::RouterAdvert(ra) => self.ra_input(ni, iface, v, &nd, ra, hop),
            NdMsg::Redirect { .. } => {}
        }
    }

    fn ns_input(&mut self, ni: usize, iface: u16, v: &Ipv6View, nd: &Nd, target: Ipv6, hop: HopId) {
        let (src, dst) = (v.src(), v.dst());
        let Some(v6) = self.v6(ni) else {
            return;
        };
        let routing = v6.routing;
        let Some(a) = v6.iface(iface).and_then(|f| f.addr(target)).cloned() else {
            self.note6(ni, Some(hop), Note6::NsIgnored { target });
            return;
        };
        match a.state {
            // RFC 4862 5.4.3: another node probes for the same tentative address
            AddrState::Tentative => {
                if src.is_unspecified() {
                    self.dad_dup(ni, iface, target, None, Some(hop));
                }
                return;
            }
            AddrState::Duplicate => return,
            _ => {}
        }
        self.note6(ni, Some(hop), Note6::NsHeard { iface, target, from: src });
        if !src.is_unspecified()
            && let Some(m) = nd.src_lla()
        {
            self.neigh_learn(ni, iface, src, m, None, Some(hop));
        }
        let (serial, _, _) = self.v6_kind(ni, iface);
        let mac = if serial { None } else { Some(self.v6_mac(ni, iface)) };
        if src.is_unspecified() {
            // a DAD probe for an address in use: the answer goes to all nodes (RFC 4861 7.2.4)
            self.na_send(ni, iface, target, Ipv6::ALL_NODES, routing, false, !a.anycast, mac, Some(hop));
        } else {
            let group = dst.is_multicast();
            self.na_send(ni, iface, target, src, routing, true, group && !a.anycast, if group { mac } else { None }, Some(hop));
        }
    }

    fn na_input(&mut self, ni: usize, iface: u16, nd: &Nd, flags: (bool, bool, bool), target: Ipv6, hop: HopId) {
        let now = self.now;
        let (router, solicited, override_) = flags;
        let tlla = nd.tgt_lla();
        let Some(v6) = self.v6(ni) else {
            return;
        };
        let own = v6.iface(iface).and_then(|f| f.addr(target)).map(|a| a.state);
        match own {
            // RFC 4862 5.4.4: an advertisement for a tentative address
            Some(AddrState::Tentative) => {
                self.dad_dup(ni, iface, target, tlla, Some(hop));
                return;
            }
            Some(_) => return,
            None => {}
        }
        let reachable = v6.iface(iface).map(|f| f.reachable).unwrap_or(REACHABLE_TIME);
        self.note6(ni, Some(hop), Note6::NaHeard { iface, target, mac: tlla, router, solicited });
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let Some(e) = v6.neigh.get_mut(&(iface, target)) else {
            self.note6(ni, Some(hop), Note6::NaIgnored { target });
            return;
        };
        if e.fixed {
            return;
        }
        let old = e.state;
        let mut lost_router = false;
        let mut queue: Vec<Pending6> = Vec::new();
        if old == NState::Incomplete {
            let Some(m) = tlla else {
                return;
            };
            e.mac = Some(m);
            e.router = router;
            e.state = if solicited { NState::Reachable } else { NState::Stale };
            e.at = now;
            e.probes = 0;
            e.next = if solicited { Some(now + reachable) } else { None };
            queue = core::mem::take(&mut e.queue);
        } else {
            let differs = tlla.is_some_and(|m| Some(m) != e.mac);
            if !override_ && differs {
                if old == NState::Reachable {
                    e.state = NState::Stale;
                    e.next = None;
                    let mac = e.mac;
                    self.note6(ni, Some(hop), Note6::Neigh { iface, addr: target, from: Some(old), to: NState::Stale, mac });
                }
                return;
            }
            if let Some(m) = tlla {
                e.mac = Some(m);
            }
            if solicited {
                e.state = NState::Reachable;
                e.at = now;
                e.probes = 0;
                e.next = Some(now + reachable);
            } else if differs {
                e.state = NState::Stale;
                e.at = now;
                e.next = None;
            }
            lost_router = e.router && !router;
            e.router = router;
        }
        let (new, mac) = (e.state, e.mac);
        if lost_router {
            v6.routers.retain(|r| !(r.iface == iface && r.addr == target));
            self.note6(ni, Some(hop), Note6::RouterGone { iface, router: target });
        }
        if new != old {
            self.note6(ni, Some(hop), Note6::Neigh { iface, addr: target, from: Some(old), to: new, mac });
        }
        self.flush6(ni, iface, target, queue);
        self.arm_nd(ni);
    }

    /// sends what waited for a neighbor; a STALE entry goes to DELAY with the first packet
    fn flush6(&mut self, ni: usize, iface: u16, ip: Ipv6, queue: Vec<Pending6>) {
        if queue.is_empty() {
            return;
        }
        let now = self.now;
        let Some(mac) = self.v6(ni).and_then(|v| v.neigh.get(&(iface, ip))).and_then(|e| e.mac) else {
            return;
        };
        let stale = self.v6_mut(ni).and_then(|v| v.neigh.get_mut(&(iface, ip))).is_some_and(|e| {
            if e.state == NState::Stale && !e.fixed {
                e.state = NState::Delay;
                e.at = now;
                e.next = Some(now + DELAY_FIRST_PROBE_TIME);
                true
            } else {
                false
            }
        });
        if stale {
            self.note6(ni, None, Note6::Neigh { iface, addr: ip, from: Some(NState::Stale), to: NState::Delay, mac: Some(mac) });
        }
        for p in queue {
            self.frame_out(ni, iface, mac, ET_IPV6, &p.pkt, p.cause);
        }
    }

    /// a link-layer address heard in a solicitation or advertisement (RFC 4861 6.3.4, 7.2.3):
    /// a new entry or a changed address is STALE; one that was INCOMPLETE sends what waited
    fn neigh_learn(&mut self, ni: usize, iface: u16, ip: Ipv6, mac: Mac, router: Option<bool>, cause: Option<HopId>) {
        let now = self.now;
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let mut note: Option<Note6> = None;
        let mut queue: Vec<Pending6> = Vec::new();
        match v6.neigh.get_mut(&(iface, ip)) {
            None => {
                v6.neigh.insert((iface, ip), Neigh { mac: Some(mac), state: NState::Stale, router: router.unwrap_or(false), fixed: false, at: now, probes: 0, next: None, queue: Vec::new() });
                note = Some(Note6::Neigh { iface, addr: ip, from: None, to: NState::Stale, mac: Some(mac) });
            }
            Some(e) if e.fixed => {}
            Some(e) => {
                if let Some(r) = router {
                    e.router = r;
                }
                if e.state == NState::Incomplete || e.mac != Some(mac) {
                    let old = e.state;
                    e.mac = Some(mac);
                    e.state = NState::Stale;
                    e.at = now;
                    e.next = None;
                    e.probes = 0;
                    queue = core::mem::take(&mut e.queue);
                    note = Some(Note6::Neigh { iface, addr: ip, from: Some(old), to: NState::Stale, mac: Some(mac) });
                }
            }
        }
        if let Some(n) = note {
            self.note6(ni, cause, n);
        }
        self.flush6(ni, iface, ip, queue);
        self.arm_nd(ni);
    }

    fn rs_input(&mut self, ni: usize, iface: u16, v: &Ipv6View, nd: &Nd, hop: HopId) {
        let now = self.now;
        let (serial, lo, _) = self.v6_kind(ni, iface);
        let Some(v6) = self.v6(ni) else {
            return;
        };
        // hosts drop solicitations (RFC 4861 6.2.6), and so does Linux without radvd
        let Some(f) = v6.iface(iface).filter(|_| v6.routing && !v6.linux) else {
            return;
        };
        if f.ra.suppress_all || f.link_local().is_none() || serial || lo {
            return;
        }
        let (last, next) = (f.ra_last, f.ra_at);
        self.note6(ni, Some(hop), Note6::RsHeard { iface, from: v.src() });
        if !v.src().is_unspecified()
            && let Some(m) = nd.src_lla()
        {
            self.neigh_learn(ni, iface, v.src(), m, None, Some(hop));
        }
        let delay = self.v6_rand(ni, (MAX_RA_DELAY_TIME / MS) as u32) as Ns * MS;
        let mut t = now + delay;
        if let Some(l) = last
            && now < l + MIN_DELAY_BETWEEN_RAS
        {
            t = l + MIN_DELAY_BETWEEN_RAS + delay;
        }
        if let Some(n) = next
            && n <= t
        {
            // the next advertisement is due earlier anyway
            return;
        }
        if let Some(f) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(iface as usize)) {
            f.ra_at = Some(t);
            f.ra_solicited = true;
        }
        self.arm_ra(ni);
    }

    fn ra_input(&mut self, ni: usize, iface: u16, v: &Ipv6View, nd: &Nd, ra: Ra, hop: HopId) {
        let now = self.now;
        let src = v.src();
        let link = self.stack(ni).and_then(|s| s.ifaces.get(iface as usize)).map(|f| f.mtu as u32).unwrap_or(1500);
        let Some(v6) = self.v6(ni) else {
            return;
        };
        if v6.routing {
            self.note6(ni, Some(hop), Note6::RaIgnored { iface, why: RaWhy::Router });
            return;
        }
        let Some(f) = v6.iface(iface) else {
            return;
        };
        if !f.accept_ra {
            self.note6(ni, Some(hop), Note6::RaIgnored { iface, why: RaWhy::Off });
            return;
        }
        let (autoconf, default_route, base) = (f.autoconf, f.default_route, f.base_reachable);
        let prefixes: Vec<PrefixInfo> = nd.prefixes().copied().collect();
        self.note6(ni, Some(hop), Note6::RaHeard { iface, from: src, lifetime: ra.lifetime, prefixes: prefixes.len() as u16 });
        let new_base = (ra.reachable != 0 && ra.reachable as Ns * MS != base).then_some(ra.reachable as Ns * MS);
        let r = if new_base.is_some() { self.v6_rand(ni, 1000) as Ns } else { 0 };
        let mut notes: Vec<Note6> = Vec::new();
        if let Some(v6) = self.v6_mut(ni) {
            let f = v6.if_mut(iface);
            f.rs_at = None;
            f.rs_left = 0;
            f.heard.retain(|h| h.0 != src);
            f.heard.push((src, now, ra.lifetime, prefixes.clone()));
            if ra.hop_limit != 0 {
                f.hop_limit = Some(ra.hop_limit);
            }
            if let Some(b) = new_base {
                f.base_reachable = b;
                f.reachable = b / 1000 * (500 + r);
            }
            if ra.retrans != 0 {
                f.retrans = ra.retrans as Ns * MS;
            }
            if let Some(m) = nd.mtu()
                && (1280..=link).contains(&m)
            {
                f.link_mtu = Some(m as u16);
            }
            // on-link prefixes (RFC 4861 6.3.4)
            for p in &prefixes {
                if !p.on_link || p.prefix.is_link_local() {
                    continue;
                }
                let pf = Prefix6 { addr: p.prefix, len: p.len.min(128) }.trunc();
                let have = f.prefixes.iter().position(|x| x.prefix == pf);
                match (have, p.valid) {
                    (Some(k), 0) => {
                        f.prefixes.remove(k);
                        notes.push(Note6::PrefixGone { iface, prefix: pf });
                    }
                    (Some(k), s) => f.prefixes[k].until = lifetime(now, s),
                    (None, 0) => {}
                    (None, s) => {
                        f.prefixes.push(OnLink { prefix: pf, until: lifetime(now, s) });
                        notes.push(Note6::PrefixOnLink { iface, prefix: pf, valid: s });
                    }
                }
            }
            // the default router list
            if default_route {
                let have = v6.routers.iter().position(|x| x.iface == iface && x.addr == src);
                match (have, ra.lifetime) {
                    (Some(k), 0) => {
                        v6.routers.remove(k);
                        notes.push(Note6::RouterGone { iface, router: src });
                    }
                    (Some(k), l) => {
                        v6.routers[k].until = now + l as Ns * SEC;
                        v6.routers[k].prf = ra.prf;
                    }
                    (None, 0) => {}
                    (None, l) => {
                        v6.routers.push(DefRouter { iface, addr: src, until: now + l as Ns * SEC, prf: ra.prf });
                        notes.push(Note6::RouterAdded { iface, router: src, lifetime: l });
                    }
                }
            }
        }
        for n in notes {
            self.note6(ni, Some(hop), n);
        }
        if let Some(m) = nd.src_lla() {
            self.neigh_learn(ni, iface, src, m, Some(true), Some(hop));
        }
        // addresses from autonomous /64 prefixes (RFC 4862 5.5.3)
        if autoconf {
            for p in &prefixes {
                let bad = p.prefix.is_link_local() || (p.preferred != icmpv6::INFINITE && p.valid != icmpv6::INFINITE && p.preferred > p.valid);
                if p.auto && !bad && p.len == 64 {
                    self.slaac(ni, iface, Prefix6 { addr: p.prefix, len: 64 }.trunc(), p.valid, p.preferred, hop);
                }
            }
        }
        self.arm_addr(ni);
        self.arm_ra(ni);
    }

    /// an address from a prefix of an advertisement, or new lifetimes for it
    fn slaac(&mut self, ni: usize, i: u16, pf: Prefix6, valid: u32, preferred: u32, hop: HopId) {
        let now = self.now;
        let addr = Ipv6::eui64_in(pf.addr, self.v6_mac(ni, i));
        let Some(f) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)) else {
            return;
        };
        if let Some(a) = f.addrs.iter_mut().find(|a| a.origin == Origin6::Slaac && a.addr == addr) {
            a.preferred_until = lifetime(now, preferred);
            // RFC 4862 5.5.3 (e): a short valid lifetime does not cut a long one below two hours
            let remaining = a.valid_until.map(|t| t.saturating_sub(now));
            let adv = if valid == icmpv6::INFINITE { Ns::MAX } else { valid as Ns * SEC };
            if adv > TWO_HOURS || remaining.is_some_and(|r| adv > r) {
                a.valid_until = lifetime(now, valid);
            } else if remaining.is_some_and(|r| r <= TWO_HOURS) {
            } else {
                a.valid_until = Some(now + TWO_HOURS);
            }
            if a.state == AddrState::Deprecated && preferred > 0 {
                a.state = AddrState::Preferred;
            }
            let v = a.valid_until.map_or(icmpv6::INFINITE, |t| ((t - now) / SEC) as u32);
            self.note6(ni, Some(hop), Note6::AddrLifetime { iface: i, addr, valid: v, preferred });
            self.arm_addr(ni);
            return;
        }
        if valid == 0 || f.has(addr) {
            return;
        }
        f.addrs.push(Addr6 {
            addr,
            len: 64,
            origin: Origin6::Slaac,
            state: AddrState::Tentative,
            preferred_until: lifetime(now, preferred),
            valid_until: lifetime(now, valid),
            anycast: false,
            dad_at: None,
            dad_left: 0,
            dup_mac: None,
        });
        self.note6(ni, Some(hop), Note6::AddrNew { iface: i, addr: Prefix6 { addr, len: 64 }, origin: Origin6::Slaac });
        self.start_dad(ni, i, addr);
        self.arm_addr(ni);
    }

    // ---------------------------------------------------------------- output

    /// a neighbor solicitation: to the target's solicited-node group, or to its MAC (NUD)
    fn ns_send(&mut self, ni: usize, iface: u16, target: Ipv6, unicast: Option<Mac>, hint: Option<Ipv6>, cause: Option<HopId>) {
        let (serial, _, _) = self.v6_kind(ni, iface);
        let mac = self.v6_mac(ni, iface);
        let Some(f) = self.v6(ni).and_then(|v| v.iface(iface)) else {
            return;
        };
        let src = hint.filter(|h| f.owns(*h)).or_else(|| f.link_local()).or_else(|| f.addrs.iter().find(|a| a.state.usable()).map(|a| a.addr));
        let Some(src) = src else {
            return;
        };
        let dst = if unicast.is_some() { target } else { target.solicited_node() };
        let msg = Nd::ns(target, if serial { None } else { Some(mac) }).encode(src, dst);
        let pkt = Ipv6Hdr::new(src, dst, NH_ICMPV6, 255).packet(&msg);
        self.note6(ni, cause, Note6::NsSent { iface, target, unicast: unicast.is_some() });
        self.frame_out(ni, iface, unicast.unwrap_or(Mac::of_ipv6_multicast(dst)), ET_IPV6, &pkt, cause);
    }

    /// a neighbor advertisement for `target` to `to`
    #[allow(clippy::too_many_arguments)]
    fn na_send(&mut self, ni: usize, iface: u16, target: Ipv6, to: Ipv6, router: bool, solicited: bool, override_: bool, tlla: Option<Mac>, cause: Option<HopId>) {
        let Some(f) = self.v6(ni).and_then(|v| v.iface(iface)) else {
            return;
        };
        let own = f.addr(target).is_some_and(|a| !a.anycast && a.state.usable());
        let Some(src) = (if own { Some(target) } else { f.link_local() }) else {
            return;
        };
        let msg = Nd::na(target, router, solicited, override_, tlla).encode(src, to);
        let pkt = Ipv6Hdr::new(src, to, NH_ICMPV6, 255).packet(&msg);
        self.note6(ni, cause, Note6::NaSent { iface, target, to, solicited });
        self.ip6_l2(ni, NextHop6 { iface, gw: to, src: Src6::Connected }, pkt, Origin::Local { job: None }, cause);
    }

    /// a packet for neighbor `gw` on `iface` (address resolution and the neighbor cache)
    pub(crate) fn nd_send(&mut self, ni: usize, iface: u16, gw: Ipv6, pkt: Vec<u8>, origin: Origin, cause: Option<HopId>) {
        let now = self.now;
        let hint = Ipv6View::parse(&pkt).map(|v| v.src());
        let Some(v6) = self.v6(ni) else {
            return;
        };
        let (qmax, retrans) = (v6.queue, v6.iface(iface).map(|f| f.retrans).unwrap_or(RETRANS_TIMER));
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let mut notes: Vec<Note6> = Vec::new();
        let mut send: Option<Mac> = None;
        let mut ask = false;
        match v6.neigh.get_mut(&(iface, gw)) {
            Some(e) if e.mac.is_some() => {
                if !e.fixed && e.state == NState::Stale {
                    e.state = NState::Delay;
                    e.at = now;
                    e.next = Some(now + DELAY_FIRST_PROBE_TIME);
                    notes.push(Note6::Neigh { iface, addr: gw, from: Some(NState::Stale), to: NState::Delay, mac: e.mac });
                }
                send = e.mac;
            }
            Some(e) => {
                if qmax == 0 {
                    notes.push(Note6::NeighDrop { addr: gw });
                } else {
                    if e.queue.len() >= qmax {
                        e.queue.remove(0);
                        notes.push(Note6::NeighDrop { addr: gw });
                    }
                    e.queue.push(Pending6 { pkt: pkt.clone(), origin, cause });
                    notes.push(Note6::NeighWait { addr: gw });
                }
            }
            None => {
                let queue = if qmax > 0 { alloc::vec![Pending6 { pkt: pkt.clone(), origin, cause }] } else { Vec::new() };
                v6.neigh.insert((iface, gw), Neigh { mac: None, state: NState::Incomplete, router: false, fixed: false, at: now, probes: 1, next: Some(now + retrans), queue });
                notes.push(Note6::Neigh { iface, addr: gw, from: None, to: NState::Incomplete, mac: None });
                ask = true;
            }
        }
        for n in notes {
            self.note6(ni, cause, n);
        }
        if let Some(m) = send {
            self.frame_out(ni, iface, m, ET_IPV6, &pkt, cause);
        }
        if ask {
            self.ns_send(ni, iface, gw, None, hint, cause);
            self.note6(ni, cause, Note6::NeighWait { addr: gw });
        }
        self.arm_nd(ni);
    }

    fn arm_nd(&mut self, ni: usize) {
        let next = self.v6(ni).and_then(|v| v.neigh.values().filter_map(|e| e.next).min());
        self.arm6(ni, SLOT_V6_ND, next);
    }

    fn nd_timer(&mut self, ni: usize) {
        let now = self.now;
        let due: Vec<(u16, Ipv6)> = self.v6(ni).map(|v| v.neigh.iter().filter(|(_, e)| e.next.is_some_and(|t| t <= now)).map(|(k, _)| *k).collect()).unwrap_or_default();
        for (iface, ip) in due {
            let retrans = self.v6(ni).and_then(|v| v.iface(iface)).map(|f| f.retrans).unwrap_or(RETRANS_TIMER);
            // the entry changes first, then the device acts
            let step = {
                let Some(e) = self.v6_mut(ni).and_then(|v| v.neigh.get_mut(&(iface, ip))) else {
                    continue;
                };
                match e.state {
                    NState::Incomplete if e.probes < MAX_MULTICAST_SOLICIT => {
                        e.probes += 1;
                        e.next = Some(now + retrans);
                        NdStep::Ask(e.queue.first().and_then(|p| Ipv6View::parse(&p.pkt).map(|v| v.src())))
                    }
                    NState::Incomplete => NdStep::Fail,
                    NState::Reachable => {
                        e.state = NState::Stale;
                        e.next = None;
                        NdStep::Moved(NState::Reachable, NState::Stale, e.mac, false)
                    }
                    NState::Delay => {
                        e.state = NState::Probe;
                        e.probes = 1;
                        e.at = now;
                        e.next = Some(now + retrans);
                        NdStep::Moved(NState::Delay, NState::Probe, e.mac, true)
                    }
                    NState::Probe if e.probes < MAX_UNICAST_SOLICIT => {
                        e.probes += 1;
                        e.next = Some(now + retrans);
                        NdStep::Probe(e.mac)
                    }
                    NState::Probe => NdStep::Gone,
                    NState::Stale => {
                        e.next = None;
                        continue;
                    }
                }
            };
            match step {
                NdStep::Ask(hint) => self.ns_send(ni, iface, ip, None, hint, None),
                NdStep::Fail => {
                    let queue = self.v6_mut(ni).and_then(|v| v.neigh.remove(&(iface, ip))).map(|e| e.queue).unwrap_or_default();
                    self.note6(ni, None, Note6::NeighFailed { addr: ip });
                    for p in queue {
                        self.nd_failed(ni, p);
                    }
                }
                NdStep::Moved(from, to, mac, probe) => {
                    self.note6(ni, None, Note6::Neigh { iface, addr: ip, from: Some(from), to, mac });
                    if probe {
                        self.ns_send(ni, iface, ip, mac, None, None);
                    }
                }
                NdStep::Probe(mac) => self.ns_send(ni, iface, ip, mac, None, None),
                NdStep::Gone => {
                    if let Some(v) = self.v6_mut(ni) {
                        v.neigh.remove(&(iface, ip));
                    }
                    self.note6(ni, None, Note6::NeighFailed { addr: ip });
                }
            }
        }
        self.arm_nd(ni);
    }

    /// a packet whose neighbor never answered: "address unreachable" (RFC 4861 7.2.2)
    fn nd_failed(&mut self, ni: usize, p: Pending6) {
        match p.origin {
            Origin::Local { job: Some((j, seq)) } => {
                let from = Ipv6View::parse(&p.pkt).map(|v| v.src()).unwrap_or(Ipv6::UNSPECIFIED);
                self.job6_error(j, seq, from, crate::jobs6::Icmp6Kind::AddrUnreachable);
            }
            Origin::Local { job: None } => {
                // the device's own TCP hears of it as of "address unreachable": a connection
                // attempt ends with "No route to host" (Linux: ndisc_error_report)
                if let Some(v) = Ipv6View::parse(&p.pkt) {
                    self.tcp_icmp6_error(ni, icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_ADDR, 0, &v);
                }
            }
            Origin::Forwarded { iface: inif } => {
                self.icmp6_error(ni, inif, &p.pkt, Icmp6::DestUnreach { code: icmpv6::UNREACH_ADDR }, p.cause);
            }
        }
    }

    // ---------------------------------------------------------------- RS and RA

    fn arm_ra(&mut self, ni: usize) {
        let next = self.v6(ni).and_then(|v| v.ifs.iter().filter(|f| f.up).flat_map(|f| [f.rs_at, f.ra_at]).flatten().min());
        self.arm6(ni, SLOT_V6_RA, next);
    }

    fn ra_timer(&mut self, ni: usize) {
        let now = self.now;
        let n = self.v6(ni).map(|v| v.ifs.len()).unwrap_or(0);
        for i in 0..n as u16 {
            let Some(f) = self.v6(ni).and_then(|v| v.iface(i)).filter(|f| f.up) else {
                continue;
            };
            let (rs, ra, solicited) = (f.rs_at.is_some_and(|t| t <= now), f.ra_at.is_some_and(|t| t <= now), f.ra_solicited);
            if rs {
                self.rs_send(ni, i);
                if let Some(f) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)) {
                    f.rs_left = f.rs_left.saturating_sub(1);
                    f.rs_at = if f.rs_left > 0 { Some(now + RTR_SOLICITATION_INTERVAL) } else { None };
                }
            }
            if ra {
                self.ra_send(ni, i, solicited, false);
                let (min, max, periodic, initial) = match self.v6(ni).and_then(|v| v.iface(i)) {
                    Some(f) => (f.ra.min(), f.ra.max_interval, f.advertising, f.ra_initial),
                    None => continue,
                };
                let span = max.saturating_sub(min) / MS;
                let mut d = min + self.v6_rand(ni, span as u32) as Ns * MS;
                if initial > 0 {
                    d = d.min(MAX_INITIAL_RTR_ADVERT_INTERVAL);
                }
                if let Some(f) = self.v6_mut(ni).and_then(|v| v.ifs.get_mut(i as usize)) {
                    f.ra_last = Some(now);
                    f.ra_solicited = false;
                    f.ra_initial = f.ra_initial.saturating_sub(1);
                    f.ra_at = if periodic { Some(now + d) } else { None };
                }
            }
        }
        self.arm_ra(ni);
    }

    /// a router solicitation from the link-local address
    fn rs_send(&mut self, ni: usize, i: u16) {
        let mac = self.v6_mac(ni, i);
        let Some(src) = self.v6(ni).and_then(|v| v.iface(i)).and_then(|f| f.link_local()) else {
            return;
        };
        let msg = Nd::rs(Some(mac)).encode(src, Ipv6::ALL_ROUTERS);
        let pkt = Ipv6Hdr::new(src, Ipv6::ALL_ROUTERS, NH_ICMPV6, 255).packet(&msg);
        self.note6(ni, None, Note6::RsSent { iface: i });
        self.frame_out(ni, i, Mac::of_ipv6_multicast(Ipv6::ALL_ROUTERS), ET_IPV6, &pkt, None);
    }

    /// the prefixes an interface advertises: those of its global addresses (as "ipv6 nd
    /// prefix" sets them, unless hidden), then the ones only "ipv6 nd prefix" names
    pub(crate) fn ra_prefixes(&self, ni: usize, i: u16) -> Vec<PrefixInfo> {
        let Some(f) = self.v6(ni).and_then(|v| v.iface(i)) else {
            return Vec::new();
        };
        let mut out: Vec<PrefixInfo> = Vec::new();
        let mut seen: Vec<Prefix6> = Vec::new();
        let pi = |p: Prefix6, c: Option<&PrefixCfg>| PrefixInfo {
            len: p.len,
            on_link: c.is_none_or(|c| c.on_link),
            auto: c.is_none_or(|c| c.auto),
            valid: c.map_or(VALID_DEFAULT, |c| c.valid),
            preferred: c.map_or(PREFERRED_DEFAULT, |c| c.preferred),
            prefix: p.addr,
        };
        for a in &f.addrs {
            if !matches!(a.origin, Origin6::Manual | Origin6::Eui64) || a.anycast || a.len == 0 || a.len >= 128 || a.state == AddrState::Duplicate {
                continue;
            }
            let p = a.prefix().trunc();
            if seen.contains(&p) {
                continue;
            }
            seen.push(p);
            let c = f.ra.prefixes.iter().find(|c| c.prefix.trunc() == p);
            if c.is_some_and(|c| c.hidden) {
                continue;
            }
            out.push(pi(p, c));
        }
        for c in &f.ra.prefixes {
            let p = c.prefix.trunc();
            if !seen.contains(&p) && !c.hidden {
                seen.push(p);
                out.push(pi(p, Some(c)));
            }
        }
        out
    }

    /// a router advertisement to all nodes (`last`: lifetime 0, the interface stops advertising)
    fn ra_send(&mut self, ni: usize, i: u16, solicited: bool, last: bool) {
        let mac = self.v6_mac(ni, i);
        let mtu = self.ip6_mtu(ni, i) as u32;
        let prefixes = self.ra_prefixes(ni, i);
        let Some(v6) = self.v6(ni) else {
            return;
        };
        let Some(f) = v6.iface(i) else {
            return;
        };
        let Some(src) = f.link_local() else {
            return;
        };
        let lifetime = if last { 0 } else { f.ra.lifetime };
        let ra = Ra { hop_limit: v6.hop_limit, managed: f.ra.managed, other: f.ra.other, prf: 0, lifetime, reachable: f.ra.reachable, retrans: 0 };
        let mut opts = alloc::vec![NdOpt::SrcLla(mac)];
        if f.ra.mtu {
            opts.push(NdOpt::Mtu(mtu));
        }
        let n = prefixes.len() as u16;
        opts.extend(prefixes.into_iter().map(NdOpt::Prefix));
        let msg = Nd::new(NdMsg::RouterAdvert(ra), opts).encode(src, Ipv6::ALL_NODES);
        let pkt = Ipv6Hdr::new(src, Ipv6::ALL_NODES, NH_ICMPV6, 255).packet(&msg);
        self.note6(ni, None, Note6::RaSent { iface: i, to: Ipv6::ALL_NODES, prefixes: n, lifetime, solicited });
        self.frame_out(ni, i, Mac::of_ipv6_multicast(Ipv6::ALL_NODES), ET_IPV6, &pkt, None);
    }

    // ---------------------------------------------------------------- Action::Ip6

    /// the IPv6 interface of a device by name
    fn v6_iface(&self, dev: u32, name: &str) -> Result<(usize, u16), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let st = self.stack(ni).ok_or(ActErr::NoIp(dev))?;
        let i = st.iface_by_name(name).ok_or_else(|| ActErr::UnknownIface { dev, iface: String::from(name) })?;
        Ok((ni, i))
    }

    /// a change of the IPv6 configuration
    pub(crate) fn ip6_apply(&mut self, dev: u32, set: Ip6Set) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let mut sync: Option<u16> = None;
        match set {
            Ip6Set::Routing(on) => {
                if let Some(v) = self.v6_mut(ni) {
                    v.routing = on;
                }
            }
            Ip6Set::All(on) => {
                let n = self.stack(ni).map(|s| s.ifaces.len()).unwrap_or(0);
                if let Some(v) = self.v6_mut(ni) {
                    v.all_on = on;
                    for i in 0..n as u16 {
                        v.if_mut(i).enabled = on;
                        if !on {
                            v.disable_linux(i);
                        }
                    }
                }
            }
            Ip6Set::Enable { iface, on } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(v) = self.v6_mut(ni) {
                    let linux = v.linux;
                    let f = v.if_mut(i);
                    f.explicit = on;
                    f.enabled = on || (!linux && (!f.cfg.is_empty() || f.ll_manual.is_some() || f.autoconf));
                    if linux && !on {
                        v.disable_linux(i);
                    }
                }
            }
            Ip6Set::Addr { iface, cfg, add } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                let a = cfg.prefix.addr;
                // Linux takes more link-local addresses with "ip -6 addr add", IOS has one
                let linux = self.v6(ni).is_some_and(|v| v.linux);
                if cfg.prefix.len > 128 || a.is_multicast() || (a.is_unspecified() && !cfg.eui64) || a.is_loopback() || (a.is_link_local() && !linux) || (cfg.eui64 && cfg.prefix.len > 64) {
                    return Err(ActErr::Refused(msg!("{} taugt nicht als IPv6-Adresse.", cfg.prefix)));
                }
                let v = self.v6(ni).ok_or(ActErr::NoIp(dev))?;
                if v.linux && !v.on(i) {
                    return Err(ActErr::Refused(msg!("IPv6 ist an {} aus (disable_ipv6).", iface)));
                }
                if add && !v.linux {
                    let p = cfg.prefix.trunc();
                    let names: Vec<String> = self.stack(ni).map(|s| s.ifaces.iter().map(|f| f.name.clone()).collect()).unwrap_or_default();
                    for (j, f) in v.ifs.iter().enumerate() {
                        if j != i as usize && f.cfg.iter().any(|c| c.prefix.trunc().overlaps(&p)) {
                            return Err(ActErr::Refused(msg!("{} überschneidet sich mit {}.", p, names.get(j).cloned().unwrap_or_default())));
                        }
                    }
                }
                let v = self.v6_mut(ni).ok_or(ActErr::NoIp(dev))?;
                let f = v.if_mut(i);
                let have = f.cfg.iter().position(|c| *c == cfg || (c.prefix == cfg.prefix && c.eui64 == cfg.eui64));
                match (add, have) {
                    (true, Some(k)) => f.cfg[k] = cfg,
                    (true, None) => f.cfg.push(cfg),
                    (false, Some(k)) => {
                        f.cfg.remove(k);
                    }
                    (false, None) => return Err(ActErr::Refused(msg!("{} ist an {} nicht eingerichtet.", cfg.prefix, iface))),
                }
                if !v.linux {
                    let f = v.if_mut(i);
                    f.enabled = f.explicit || !f.cfg.is_empty() || f.ll_manual.is_some() || f.autoconf;
                }
                sync = Some(i);
            }
            Ip6Set::LinkLocal { iface, addr } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(a) = addr
                    && !a.is_link_local()
                {
                    return Err(ActErr::Refused(msg!("{} ist keine Link-Local-Adresse (fe80::/10).", a)));
                }
                if let Some(v) = self.v6_mut(ni) {
                    let linux = v.linux;
                    let f = v.if_mut(i);
                    f.ll_manual = addr;
                    if !linux {
                        f.enabled = f.explicit || !f.cfg.is_empty() || f.ll_manual.is_some() || f.autoconf;
                    }
                }
                sync = Some(i);
            }
            Ip6Set::Autoconf { iface, on, default_route } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(v) = self.v6_mut(ni) {
                    let linux = v.linux;
                    let f = v.if_mut(i);
                    f.autoconf = on;
                    f.default_route = default_route;
                    f.accept_ra = on || default_route;
                    f.rs_started = false;
                    if !on {
                        f.addrs.retain(|a| a.origin != Origin6::Slaac);
                    }
                    if !linux {
                        f.enabled = f.explicit || !f.cfg.is_empty() || f.ll_manual.is_some() || f.autoconf;
                    }
                }
            }
            Ip6Set::ClearAddrs { iface } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(v) = self.v6_mut(ni) {
                    let linux = v.linux;
                    let f = v.if_mut(i);
                    f.cfg.clear();
                    f.ll_manual = None;
                    if !linux {
                        f.autoconf = false;
                        f.accept_ra = false;
                        f.default_route = false;
                        f.enabled = f.explicit;
                    }
                    f.addrs.retain(|a| a.origin != Origin6::Slaac);
                }
                sync = Some(i);
            }
            Ip6Set::Ra { iface, ra } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(v) = self.v6_mut(ni) {
                    v.if_mut(i).ra = ra;
                }
            }
            Ip6Set::Mtu { iface, mtu } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if mtu.is_some_and(|m| (m as usize) < netpkt::ipv6::MIN_MTU) {
                    return Err(ActErr::Refused(msg!("Die IPv6-MTU muss mindestens 1280 sein.")));
                }
                if let Some(v) = self.v6_mut(ni) {
                    v.if_mut(i).mtu = mtu;
                }
            }
            Ip6Set::Nd { iface, dad, reachable, retrans } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                if let Some(v) = self.v6_mut(ni) {
                    let f = v.if_mut(i);
                    if let Some(d) = dad {
                        f.dad_transmits = d;
                    }
                    if let Some(r) = reachable {
                        f.base_reachable = if r == 0 { REACHABLE_TIME } else { r };
                        f.reachable = f.base_reachable;
                    }
                    if let Some(r) = retrans {
                        f.retrans = r.max(MS);
                    }
                }
            }
            Ip6Set::Route { route, add } => {
                let r = Route6 { prefix: route.prefix.trunc(), ..route };
                if let Some(gw) = r.via
                    && (gw.is_multicast() || gw.is_unspecified() || gw.is_loopback() || (gw.is_link_local() && r.iface.is_none()))
                {
                    return Err(ActErr::Refused(msg!("{} taugt nicht als nächster Hop.", gw)));
                }
                // removing names the prefix alone or with what narrows it
                if add && r.via.is_none() && r.iface.is_none() {
                    return Err(ActErr::Refused(msg!("Eine Route braucht einen nächsten Hop oder ein Interface.")));
                }
                let v = self.v6_mut(ni).ok_or(ActErr::NoIp(dev))?;
                if add {
                    v.statics.retain(|s| !(s.prefix == r.prefix && s.via == r.via && s.iface == r.iface));
                    v.statics.push(r);
                } else {
                    let n = v.statics.len();
                    v.statics.retain(|s| !(s.prefix == r.prefix && (r.via.is_none() || s.via == r.via) && (r.iface.is_none() || s.iface == r.iface)));
                    if v.statics.len() == n {
                        return Err(ActErr::Refused(msg!("Keine passende Route zu {}.", r.prefix)));
                    }
                }
            }
            Ip6Set::Neighbor { iface, ip, mac } => {
                let (_, i) = self.v6_iface(dev, &iface)?;
                let now = self.now;
                let v = self.v6_mut(ni).ok_or(ActErr::NoIp(dev))?;
                match mac {
                    Some(m) => {
                        if !m.is_unicast() || m.is_zero() || ip.is_multicast() || ip.is_unspecified() {
                            return Err(ActErr::Refused(msg!("{} taugt nicht als Eintrag für {}.", m, ip)));
                        }
                        let old = v.neigh.insert((i, ip), Neigh { mac: Some(m), state: NState::Reachable, router: false, fixed: true, at: now, probes: 0, next: None, queue: Vec::new() });
                        let queue = old.map(|e| e.queue).unwrap_or_default();
                        for p in queue {
                            self.frame_out(ni, i, m, ET_IPV6, &p.pkt, p.cause);
                        }
                    }
                    None => match v.neigh.get(&(i, ip)) {
                        Some(e) if e.fixed => {
                            v.neigh.remove(&(i, ip));
                        }
                        _ => return Err(ActErr::Refused(msg!("Für {} gibt es keinen statischen Eintrag.", ip))),
                    },
                }
            }
            Ip6Set::ClearNeighbors { iface } => {
                let only = match iface {
                    Some(n) => Some(self.v6_iface(dev, &n)?.1),
                    None => None,
                };
                if let Some(v) = self.v6_mut(ni) {
                    v.neigh.retain(|(i, _), e| e.fixed || only.is_some_and(|o| o != *i));
                }
            }
            Ip6Set::HopLimit(h) => {
                if let Some(v) = self.v6_mut(ni) {
                    v.hop_limit = h.max(1);
                }
            }
        }
        // (an interface IPv6 is turned off on stops at the look below)
        if let Some(i) = sync
            && self.v6(ni).is_some_and(|v| v.running(i) && v.on(i))
        {
            self.sync_addrs(ni, i, false);
        }
        self.ip6_kick(ni);
        Ok(())
    }

    // ---------------------------------------------------------------- views

    /// the IPv6 part of a device (None: no IP at all)
    pub fn ip6(&self, dev: u32) -> Option<&Ip6> {
        self.v6(self.node_index(dev)?)
    }
    /// the IPv6 routing table as it is used now, ordered by prefix
    pub fn ip6_routes(&self, dev: u32) -> Vec<Route6> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        let Some(v) = self.v6(ni) else {
            return Vec::new();
        };
        let mut r = v.rib(&|i| self.iface_up(ni, i), self.now);
        r.sort_by_key(|x| (x.prefix.addr.to_u128(), x.prefix.len, x.ad, x.src, x.via.map(|g| g.to_u128())));
        r
    }
    /// the neighbor cache of a device
    pub fn ip6_neighbors(&self, dev: u32) -> Vec<Neigh6Row> {
        let Some(v) = self.ip6(dev) else {
            return Vec::new();
        };
        let now = self.now;
        v.neigh.iter().map(|(&(iface, ip), e)| Neigh6Row { iface, ip, mac: e.mac, state: e.state, router: e.router, fixed: e.fixed, age: now.saturating_sub(e.at) }).collect()
    }
    /// the IPv6 addresses of an interface (IPv4 interface number), the link-local one first
    pub fn addrs6(&self, dev: u32, iface: u16) -> Vec<Addr6View> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        self.addrs6_of(ni, iface)
    }
    pub(crate) fn addrs6_of(&self, ni: usize, iface: u16) -> Vec<Addr6View> {
        let now = self.now;
        let Some(f) = self.v6(ni).and_then(|v| v.iface(iface)).filter(|f| f.enabled || f.up || !f.addrs.is_empty()) else {
            return Vec::new();
        };
        f.addrs
            .iter()
            .map(|a| Addr6View {
                addr: a.addr,
                len: a.len,
                origin: a.origin,
                state: a.state,
                anycast: a.anycast,
                valid: a.valid_until.map(|t| t.saturating_sub(now)),
                preferred: a.preferred_until.map(|t| t.saturating_sub(now)),
            })
            .collect()
    }
}
