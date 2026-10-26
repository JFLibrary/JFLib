// ip6 -- IPv6 at the IOS-like command line: "ipv6 unicast-routing", "ipv6 route", "ipv6
// neighbor", "ipv6 hop-limit"; per interface "ipv6 enable", "ipv6 address X:X::X/N [eui-64 |
// anycast]", "ipv6 address FE80::X link-local", "ipv6 address autoconfig [default]", "ipv6 mtu",
// "ipv6 nd ra suppress [all] | ra interval | ra lifetime | ra mtu suppress | prefix ... |
// managed-config-flag | other-config-flag | dad attempts | reachable-time | ns-interval"; "show
// ipv6 interface [brief | IF]", "show ipv6 route [connected | local | static | nd]", "show ipv6
// neighbors [IF]", "show ipv6 routers", "ping ipv6 X" and "ping X:X::X" (a link-local target
// asks for the output interface), "traceroute ipv6 X", "debug ipv6 icmp | nd", "clear ipv6
// neighbors"; the lines of the running configuration; the debug lines
//
// Help texts, the route legend and the debug lines are our own words; the tables and
// configuration lines follow the IOS layout, with addresses in capitals.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_NO};
use netpkt::{Ipv6, Mac, Prefix6};

use super::ios::K;
use super::print::JobOut;
use super::print6::up6;
use super::{Ask, AskKind, D_ICMP6, D_ND};
use crate::action::Action;
use crate::ip6::{AddrCfg, AddrState, Ip6Set, Origin6, PrefixCfg, Route6, Src6};
use crate::jobs6::Ping6Opts;
use crate::jobs::TraceOpts;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::trace6::{Note6, RaWhy};

const ROUTE_CODES6: &str = "Codes: C - connected network, L - address of this device, S - static route,\n       ND - default router from advertisements, NDp - prefix from advertisements\n\n";

/// a prefix as IOS prints it ("2001:DB8:1::/64")
fn upp(p: Prefix6) -> String {
    format!("{}/{}", up6(p.addr), p.len)
}

/// the IPv6 commands of the IOS-like tree (modes: user, privileged, global, interface,
/// subinterface); `router`: the device may route ("ipv6 unicast-routing")
pub(crate) fn build_ip6(t: &mut Tree<K>, m: [ModeId; 5], router: bool) {
    let [user, prv, conf, ifm, sub] = m;
    for x in [user, prv] {
        let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>| {
            t.add(x, p, h, c, 0);
        };
        a(t, "ping ipv6", "Ping over IPv6", None);
        let tgt = "ping ipv6 WORD=target";
        a(t, tgt, "IPv6 address of the host", Some(K::Ping6));
        let opts: [(&str, &str, Option<&str>); 5] = [
            ("repeat", "How many echo requests to send", Some("<1-2147483647>=count|Number of echo requests")),
            ("size", "Size of each packet", Some("<48-18024>=bytes|Packet size in bytes")),
            ("timeout", "How long to wait for each answer", Some("<0-3600>=secs|Seconds to wait")),
            ("source", "Send from this address or interface", Some("X:X:X:X::X=src6|Address to send from")),
            ("source", "Send from this address or interface", Some("NAME1=srcif|")),
        ];
        for (kw, h, val) in opts {
            let kp = format!("{} {}", tgt, kw);
            if let Some(v) = val {
                let (pp, ph) = v.split_once('|').unwrap_or((v, ""));
                a(t, &kp, h, None);
                let full = format!("{} {}", kp, pp);
                a(t, &full, ph, Some(K::Ping6));
                t.also(x, &full, tgt);
            }
        }
        a(t, "traceroute ipv6", "Traceroute over IPv6", None);
        let tt = "traceroute ipv6 WORD=target";
        a(t, tt, "IPv6 address of the destination", Some(K::Trace6));
        a(t, &format!("{} probe", tt), "Probes per hop", None);
        a(t, &format!("{} probe <1-65535>=probes", tt), "How many probes", Some(K::Trace6));
        a(t, &format!("{} timeout", tt), "How long to wait for each probe", None);
        a(t, &format!("{} timeout <0-3600>=secs", tt), "Seconds to wait", Some(K::Trace6));
        a(t, &format!("{} ttl", tt), "First and last hop to probe", None);
        a(t, &format!("{} ttl <0-255>=min", tt), "First hop", None);
        a(t, &format!("{} ttl <0-255>=min <0-255>=max", tt), "Last hop", Some(K::Trace6));
        for end in ["probe <1-65535>=probes", "timeout <0-3600>=secs", "ttl <0-255>=min <0-255>=max"] {
            t.also(x, &format!("{} {}", tt, end), tt);
        }
        a(t, "show ipv6", "IPv6 state", None);
        a(t, "show ipv6 interface", "IPv6 settings of the interfaces", Some(K::ShowIpv6));
        a(t, "show ipv6 interface brief", "One block per interface: state and addresses", Some(K::ShowIpv6));
        a(t, "show ipv6 interface NAME1=if", "", Some(K::ShowIpv6));
        a(t, "show ipv6 route", "IPv6 routes this device knows", Some(K::ShowIpv6));
        for (k, h) in [("connected", "Networks of the interfaces"), ("local", "Addresses of this device"), ("static", "Configured routes"), ("nd", "Learned from router advertisements")] {
            a(t, &format!("show ipv6 route {}=only", k), h, Some(K::ShowIpv6));
        }
        a(t, "show ipv6 neighbors", "Neighbors found by Neighbor Discovery", Some(K::ShowIpv6));
        a(t, "show ipv6 neighbors NAME1=if", "", Some(K::ShowIpv6));
        a(t, "show ipv6 routers", "Routers heard in advertisements", Some(K::ShowIpv6));
    }
    t.add(prv, "debug ipv6", "IPv6 events", None, 0);
    t.add(prv, "debug ipv6 icmp", "ICMPv6 echo and errors sent and received", Some(K::Debug6), 0);
    t.add(prv, "debug ipv6 nd", "Neighbor Discovery and address configuration", Some(K::Debug6), 0);
    t.add(prv, "undebug ipv6", "IPv6 events", None, 0);
    t.add(prv, "undebug ipv6 icmp", "ICMPv6 echo and errors sent and received", Some(K::Debug6), 0);
    t.add(prv, "undebug ipv6 nd", "Neighbor Discovery and address configuration", Some(K::Debug6), 0);
    t.add(prv, "clear ipv6", "Empty an IPv6 table", None, 0);
    t.add(prv, "clear ipv6 neighbors", "Forget the learned IPv6 neighbors", Some(K::ClearIpv6), 0);

    // ---- global configuration
    t.add(conf, "ipv6", "IPv6 settings of the device", None, F_NO);
    if router {
        t.add(conf, "ipv6 unicast-routing", "Forward IPv6 packets and act as an IPv6 router", Some(K::Ipv6Global), 0);
    }
    t.add(conf, "ipv6 hop-limit", "Hop limit of the device's own packets", None, 0);
    t.add(conf, "ipv6 hop-limit <1-255>=hops", "Hops", Some(K::Ipv6Global), 0);
    t.add_no(conf, "ipv6 hop-limit", K::Ipv6Global);
    t.add(conf, "ipv6 route", "Add a static IPv6 route", None, 0);
    t.add(conf, "ipv6 route X:X:X:X::X/<0-128>=net", "Destination prefix", None, 0);
    t.add_no(conf, "ipv6 route X:X:X:X::X/<0-128>=net", K::Ipv6Route);
    for via in ["X:X:X:X::X=gw", "NAME1=out", "NAME1=out X:X:X:X::X=gw"] {
        let p = format!("ipv6 route X:X:X:X::X/<0-128>=net {}", via);
        t.add(conf, &p, if via.ends_with("gw") { "Next router on the way" } else { "" }, Some(K::Ipv6Route), 0);
        t.add(conf, &format!("{} <1-254>=ad", p), "Preference of this route, lower wins", Some(K::Ipv6Route), 0);
    }
    t.add(conf, "ipv6 neighbor", "Add a fixed IPv6 neighbor entry", None, 0);
    t.add(conf, "ipv6 neighbor X:X:X:X::X=ip", "IPv6 address of the neighbor", None, 0);
    t.add(conf, "ipv6 neighbor X:X:X:X::X=ip NAME1=if", "", None, 0);
    t.add(conf, "ipv6 neighbor X:X:X:X::X=ip NAME1=if H.H.H=mac", "MAC address (H.H.H)", Some(K::Ipv6Neighbor), 0);
    t.add_no(conf, "ipv6 neighbor X:X:X:X::X=ip NAME1=if", K::Ipv6Neighbor);

    // ---- interfaces
    for x in [ifm, sub] {
        let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>| {
            t.add(x, p, h, c, 0);
        };
        t.add(x, "ipv6", "IPv6 settings of this interface", None, F_NO);
        a(t, "ipv6 enable", "Use IPv6 here, with a link-local address only", Some(K::Ipv6If));
        a(t, "ipv6 address", "Give the interface an IPv6 address", None);
        a(t, "ipv6 address X:X:X:X::X/<0-128>=pfx", "Address and prefix length", Some(K::Ipv6If));
        a(t, "ipv6 address X:X:X:X::X/<0-128>=pfx eui-64", "Take the interface identifier from the MAC", Some(K::Ipv6If));
        a(t, "ipv6 address X:X:X:X::X/<0-128>=pfx anycast", "An address several devices share", Some(K::Ipv6If));
        a(t, "ipv6 address X:X:X:X::X=ll", "A link-local address", None);
        a(t, "ipv6 address X:X:X:X::X=ll link-local", "Use it instead of the one from the MAC", Some(K::Ipv6If));
        a(t, "ipv6 address autoconfig", "Form addresses from router advertisements", Some(K::Ipv6If));
        a(t, "ipv6 address autoconfig default", "Take the advertising router as the default route too", Some(K::Ipv6If));
        t.add_no(x, "ipv6 address", K::Ipv6If);
        a(t, "ipv6 mtu", "Largest IPv6 packet on this interface", None);
        a(t, "ipv6 mtu <1280-9216>=mtu", "Bytes", Some(K::Ipv6If));
        t.add_no(x, "ipv6 mtu", K::Ipv6If);
        a(t, "ipv6 nd", "Neighbor Discovery on this interface", None);
        a(t, "ipv6 nd ra", "Router advertisements", None);
        a(t, "ipv6 nd ra suppress", "Send no periodic advertisements", Some(K::Ipv6If));
        a(t, "ipv6 nd ra suppress all", "Answer no solicitations either", Some(K::Ipv6If));
        a(t, "ipv6 nd ra interval", "Time between advertisements", None);
        a(t, "ipv6 nd ra interval <4-1800>=max", "Longest time in seconds", Some(K::Ipv6If));
        a(t, "ipv6 nd ra interval <4-1800>=max <3-1800>=min", "Shortest time in seconds", Some(K::Ipv6If));
        t.add_no(x, "ipv6 nd ra interval", K::Ipv6If);
        a(t, "ipv6 nd ra lifetime", "How long hosts may use this router", None);
        a(t, "ipv6 nd ra lifetime <0-9000>=life", "Seconds (0: not a default router)", Some(K::Ipv6If));
        t.add_no(x, "ipv6 nd ra lifetime", K::Ipv6If);
        a(t, "ipv6 nd ra mtu", "The MTU option of the advertisements", None);
        a(t, "ipv6 nd ra mtu suppress", "Leave the MTU out", Some(K::Ipv6If));
        let np = "ipv6 nd prefix X:X:X:X::X/<0-128>=np";
        a(t, "ipv6 nd prefix", "A prefix of the advertisements", None);
        a(t, np, "The prefix with its length", Some(K::Ipv6If));
        a(t, &format!("{} no-advertise", np), "Leave it out of the advertisements", Some(K::Ipv6If));
        t.add_no(x, np, K::Ipv6If);
        for v in ["<0-4294967295>=valid", "infinite=vinf"] {
            let pv = format!("{} {}", np, v);
            a(t, &pv, if v.starts_with('<') { "Valid lifetime in seconds" } else { "Valid for ever" }, None);
            for p in ["<0-4294967295>=pref", "infinite=pinf"] {
                let pp = format!("{} {}", pv, p);
                a(t, &pp, if p.starts_with('<') { "Preferred lifetime in seconds" } else { "Preferred for ever" }, Some(K::Ipv6If));
                a(t, &format!("{} no-autoconfig", pp), "Hosts form no address from it", Some(K::Ipv6If));
                a(t, &format!("{} off-link", pp), "Hosts do not take it as on the link", Some(K::Ipv6If));
                t.also(x, &format!("{} no-autoconfig", pp), &pp);
                t.also(x, &format!("{} off-link", pp), &pp);
            }
        }
        a(t, "ipv6 nd managed-config-flag", "Hosts take their addresses from DHCPv6 (M flag)", Some(K::Ipv6If));
        a(t, "ipv6 nd other-config-flag", "Hosts take other settings from DHCPv6 (O flag)", Some(K::Ipv6If));
        a(t, "ipv6 nd dad", "Duplicate address detection", None);
        a(t, "ipv6 nd dad attempts", "Probes per address", None);
        a(t, "ipv6 nd dad attempts <0-600>=dad", "Probes (0: no detection)", Some(K::Ipv6If));
        t.add_no(x, "ipv6 nd dad attempts", K::Ipv6If);
        a(t, "ipv6 nd reachable-time", "How long a confirmed neighbor counts as reachable", None);
        a(t, "ipv6 nd reachable-time <0-3600000>=reach", "Milliseconds (0: the default)", Some(K::Ipv6If));
        t.add_no(x, "ipv6 nd reachable-time", K::Ipv6If);
        a(t, "ipv6 nd ns-interval", "Time between neighbor solicitations", None);
        a(t, "ipv6 nd ns-interval <1000-3600000>=nsi", "Milliseconds", Some(K::Ipv6If));
        t.add_no(x, "ipv6 nd ns-interval", K::Ipv6If);
    }
}

impl Engine {
    /// an IPv6 command of the IOS-like console
    pub(crate) fn ios_ip6(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match p.cmd {
            K::Ipv6Global => {
                let set = if p.has("unicast-routing") {
                    Ip6Set::Routing(!p.no)
                } else {
                    Ip6Set::HopLimit(if p.no { 64 } else { p.num("hops").unwrap_or(64) as u8 })
                };
                // neither can be refused
                let _ = self.apply(Action::Ip6 { dev, set });
                Ok(())
            }
            K::Ipv6Route => {
                let Some((a, len)) = p.prefix6("net") else {
                    return Ok(());
                };
                let prefix = Prefix6 { addr: Ipv6(a), len };
                if prefix.trunc().addr != prefix.addr {
                    return self.ios_err(ni, "% Invalid prefix: the address has bits beyond its length\n");
                }
                let via = p.ip6("gw").map(Ipv6);
                let iface = p.text("out").and_then(|n| self.stack(ni).and_then(|s| s.iface_by_name(n)));
                if p.no {
                    let r = Route6 { prefix, via, iface, src: Src6::Static, ad: 1, metric: 1024, expires: None };
                    return match self.apply(Action::Ip6 { dev, set: Ip6Set::Route { route: r, add: false } }) {
                        Ok(()) => Ok(()),
                        Err(_) => self.ios_err(ni, "%No matching route to delete\n"),
                    };
                }
                if p.text("out").is_some() && iface.is_none() {
                    return self.ios_err(ni, "% Invalid interface\n");
                }
                if let Some(g) = via
                    && g.is_link_local()
                    && iface.is_none()
                {
                    return self.ios_err(ni, "% Interface has to be specified for a link-local nexthop\n");
                }
                if let Some(g) = via
                    && self.v6(ni).is_some_and(|v| v.ifs.iter().any(|f| f.has(g)))
                {
                    return self.ios_err(ni, "% Invalid next hop address (it's this router)\n");
                }
                let ad = p.num("ad").unwrap_or(1) as u8;
                let r = Route6 { prefix, via, iface, src: Src6::Static, ad, metric: 1024, expires: None };
                match self.apply(Action::Ip6 { dev, set: Ip6Set::Route { route: r, add: true } }) {
                    Ok(()) => Ok(()),
                    Err(_) => self.ios_err(ni, "% Invalid next hop address\n"),
                }
            }
            K::Ipv6Neighbor => {
                let ip = Ipv6(p.ip6("ip").unwrap_or([0; 16]));
                let name = String::from(p.text("if").unwrap_or(""));
                let mac = if p.no { None } else { p.mac("mac").map(Mac) };
                match self.apply(Action::Ip6 { dev, set: Ip6Set::Neighbor { iface: name, ip, mac } }) {
                    Ok(()) => Ok(()),
                    Err(_) if p.no => self.ios_err(ni, &format!("% No neighbor entry for {} found\n", up6(ip))),
                    Err(_) => self.ios_err(ni, "% Invalid neighbor entry\n"),
                }
            }
            K::Ipv6If => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if let Err(e) = self.ios_ip6_if(ni, &f, p) {
                        r = Err(e);
                    }
                }
                r
            }
            K::ShowIpv6 => {
                let text = if p.has("interface") {
                    if p.has("brief") { self.show_ipv6_brief(ni) } else { self.show_ipv6_interface(ni, p.text("if")) }
                } else if p.has("route") {
                    self.show_ipv6_route(ni, p.text("only"))
                } else if p.has("neighbors") {
                    self.show_ipv6_neighbors(ni, p.text("if"))
                } else {
                    self.show_ipv6_routers(ni)
                };
                self.out(ni, &text);
                Ok(())
            }
            K::Ping6 => self.ios_ping6(ni, p),
            K::Trace6 => self.ios_trace6(ni, p),
            K::Debug6 => {
                let off = p.no || p.has("undebug");
                let bits = if p.has("icmp") { D_ICMP6 } else { D_ND };
                let before = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
                let after = if off { before & !bits } else { before | bits };
                if let Some(c) = self.cdev_mut(ni) {
                    c.debug = after;
                }
                if before == 0 && after != 0 {
                    self.cli.debug += 1;
                } else if before != 0 && after == 0 {
                    self.cli.debug = self.cli.debug.saturating_sub(1);
                }
                let what = if bits == D_ICMP6 { "ICMPv6" } else { "IPv6 Neighbor Discovery" };
                self.out(ni, &format!("Debug output for {}: {}\n", what, if off { "off" } else { "on" }));
                Ok(())
            }
            K::ClearIpv6 => {
                let _ = self.apply(Action::Ip6 { dev, set: Ip6Set::ClearNeighbors { iface: None } });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// an IPv6 command of interface mode for one interface
    fn ios_ip6_if(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let Some(i) = self.stack(ni).and_then(|s| s.iface_by_name(name)) else {
            let pending = self.cdev(ni).is_some_and(|c| c.cfg.pending.iter().any(|x| x == name));
            let text = if pending { "% Give the subinterface its VLAN first (encapsulation dot1Q)\n" } else { "% IPv6 addresses may not be configured on L2 links.\n" };
            return self.ios_err(ni, text);
        };
        let ifn = String::from(name);
        let cur = self.v6(ni).and_then(|v| v.iface(i)).cloned();
        let set = if p.has("enable") {
            Ip6Set::Enable { iface: ifn, on: !p.no }
        } else if p.has("address") {
            if p.has("autoconfig") {
                Ip6Set::Autoconf { iface: ifn, on: !p.no, default_route: !p.no && p.has("default") }
            } else if let Some(ll) = p.ip6("ll") {
                let a = Ipv6(ll);
                if !a.is_link_local() {
                    return self.ios_err(ni, &format!("% {} is not a link-local address\n", up6(a)));
                }
                if p.no && cur.as_ref().and_then(|f| f.ll_manual) != Some(a) {
                    return self.ios_err(ni, "% Address not found\n");
                }
                Ip6Set::LinkLocal { iface: ifn, addr: if p.no { None } else { Some(a) } }
            } else if let Some((a, len)) = p.prefix6("pfx") {
                let addr = Ipv6(a);
                let cfg = AddrCfg { prefix: Prefix6 { addr, len }, eui64: p.has("eui-64"), anycast: p.has("anycast") };
                if !p.no && (addr.is_multicast() || addr.is_loopback() || addr.is_link_local() || (addr.is_unspecified() && !cfg.eui64) || len == 0) {
                    return self.ios_err(ni, &format!("% {} is not a valid IPv6 address for an interface\n", upp(cfg.prefix)));
                }
                if cfg.eui64 && len > 64 {
                    return self.ios_err(ni, "% Prefix length too long for an EUI-64 interface identifier\n");
                }
                if !p.no
                    && let Some(other) = self.v6_overlap(ni, i, cfg.prefix)
                {
                    return self.ios_err(ni, &format!("% {} overlaps with {}\n", upp(cfg.prefix.trunc()), other));
                }
                Ip6Set::Addr { iface: ifn, cfg, add: !p.no }
            } else {
                // "no ipv6 address": all of them
                Ip6Set::ClearAddrs { iface: ifn }
            }
        } else if p.has("mtu") && !p.has("ra") {
            Ip6Set::Mtu { iface: ifn, mtu: if p.no { None } else { p.num("mtu").map(|m| m as u16) } }
        } else if p.has("dad") {
            Ip6Set::Nd { iface: ifn, dad: Some(if p.no { 1 } else { p.num("dad").unwrap_or(1) as u8 }), reachable: None, retrans: None }
        } else if p.has("reachable-time") {
            Ip6Set::Nd { iface: ifn, dad: None, reachable: Some(if p.no { 0 } else { p.num("reach").unwrap_or(0) as Ns * MS }), retrans: None }
        } else if p.has("ns-interval") {
            Ip6Set::Nd { iface: ifn, dad: None, reachable: None, retrans: Some(if p.no { SEC } else { p.num("nsi").unwrap_or(1000) as Ns * MS }) }
        } else {
            // the advertisements: a changed copy of the interface's settings
            let mut ra = cur.map(|f| f.ra).unwrap_or_default();
            if p.has("suppress") && p.has("mtu") {
                ra.mtu = p.no;
            } else if p.has("suppress") {
                if p.no {
                    ra.suppress = false;
                    ra.suppress_all = false;
                } else {
                    ra.suppress = true;
                    ra.suppress_all = p.has("all");
                }
            } else if p.has("interval") {
                if p.no {
                    ra.max_interval = 200 * SEC;
                    ra.min_interval = None;
                } else {
                    let max = p.num("max").unwrap_or(200) as Ns * SEC;
                    let min = p.num("min").map(|m| m as Ns * SEC);
                    if min.is_some_and(|m| m * 4 > max * 3) {
                        return self.ios_err(ni, "% The shortest interval may be at most three quarters of the longest\n");
                    }
                    ra.max_interval = max;
                    ra.min_interval = min;
                }
            } else if p.has("lifetime") {
                ra.lifetime = if p.no { 1800 } else { p.num("life").unwrap_or(1800) as u16 };
            } else if p.has("managed-config-flag") {
                ra.managed = !p.no;
            } else if p.has("other-config-flag") {
                ra.other = !p.no;
            } else if let Some((a, len)) = p.prefix6("np") {
                let prefix = Prefix6 { addr: Ipv6(a), len }.trunc();
                ra.prefixes.retain(|c| c.prefix.trunc() != prefix);
                if !p.no {
                    let life = |num: &str, inf: &str, def: u32| if p.has(inf) { netpkt::icmpv6::INFINITE } else { p.num(num).map(|v| v as u32).unwrap_or(def) };
                    let valid = life("valid", "vinf", crate::ip6::VALID_DEFAULT);
                    let preferred = life("pref", "pinf", crate::ip6::PREFERRED_DEFAULT);
                    if preferred != netpkt::icmpv6::INFINITE && (valid != netpkt::icmpv6::INFINITE && preferred > valid) {
                        return self.ios_err(ni, "% The preferred lifetime may not exceed the valid lifetime\n");
                    }
                    ra.prefixes.push(PrefixCfg { prefix, valid, preferred, on_link: !p.has("off-link"), auto: !p.has("no-autoconfig"), hidden: p.has("no-advertise") });
                }
            } else {
                return Ok(());
            }
            Ip6Set::Ra { iface: ifn, ra }
        };
        match self.apply(Action::Ip6 { dev, set }) {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    /// another interface whose configured prefix overlaps `p` (IOS refuses it)
    fn v6_overlap(&self, ni: usize, i: u16, p: Prefix6) -> Option<String> {
        let v = self.v6(ni)?;
        let st = self.stack(ni)?;
        let p = p.trunc();
        v.ifs.iter().enumerate().find(|(j, f)| *j != i as usize && f.cfg.iter().any(|c| c.prefix.trunc().overlaps(&p))).and_then(|(j, _)| st.ifaces.get(j).map(|f| f.name.clone()))
    }

    // ---------------------------------------------------------------- ping, traceroute

    /// "ping ipv6 X", "ping X:X::X"
    pub(crate) fn ios_ping6(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let target = p.text("target").unwrap_or("");
        let Some(dst) = Ipv6::parse(target) else {
            // a name: its IPv6 address from the host table or the name servers (cli/names.rs)
            if netpkt::Ipv4::parse(target).is_some() {
                return self.ios_err(ni, "% Unrecognized host or address, or protocol not running.\n\n");
            }
            return self.ios_ping_name(ni, p, target, true);
        };
        let count = p.num("count").unwrap_or(5) as u32;
        let bytes = p.num("bytes").unwrap_or(100).max(48) as u16;
        let timeout = p.num("secs").unwrap_or(2) as Ns * SEC;
        let mut oif: Option<u16> = None;
        let mut src: Option<Ipv6> = p.ip6("src6").map(Ipv6);
        if let Some(n) = p.text("srcif") {
            let Some(i) = self.stack(ni).and_then(|s| s.iface_by_name(n)) else {
                return self.ios_err(ni, &format!("% Invalid source interface - {}\n", n));
            };
            oif = Some(i);
            src = self.ip6_source(ni, dst, Some(i));
        }
        if let Some(s) = src
            && self.v6(ni).is_none_or(|v| v.find(s).is_none())
        {
            return self.ios_err(ni, "% Invalid source address - not an address of an interface that is up\n");
        }
        if dst.is_link_local() && oif.is_none() {
            // IOS asks for the interface of a link-local destination
            if let Some(c) = self.cdev_mut(ni) {
                c.ask = Some(Ask { prompt: String::from("Output Interface: "), kind: AskKind::Ping6Out { dst, count, bytes, timeout } });
            }
            return Ok(());
        }
        self.ios_ping6_start(ni, dst, count, bytes, timeout, src, oif)
    }

    /// the answer to "Output Interface: " of a link-local ping
    pub(crate) fn ios_ping6_answer(&mut self, ni: usize, dst: Ipv6, count: u32, bytes: u16, timeout: Ns, line: &str) {
        let name = line.trim();
        let i = self.stack(ni).and_then(|s| s.iface_by_name(name));
        match i {
            Some(i) => {
                let _ = self.ios_ping6_start(ni, dst, count, bytes, timeout, None, Some(i));
            }
            None => self.out(ni, "% Invalid interface. Use full interface name without spaces (e.g. Serial0/1)\n"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ios_ping6_start(&mut self, ni: usize, dst: Ipv6, count: u32, bytes: u16, timeout: Ns, src: Option<Ipv6>, oif: Option<u16>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let routed = dst.is_multicast() || self.ip6_route(ni, dst, oif).is_some() || self.v6(ni).is_some_and(|v| v.find(dst).is_some());
        if !routed {
            return self.ios_err(ni, "% No valid route for destination\n");
        }
        let mut head = format!("Ctrl+Shift+6 stops the ping.\nPinging {} with {} echo request{} of {} bytes, {} s timeout each:\n", up6(dst), count, if count == 1 { "" } else { "s" }, bytes, timeout / SEC);
        if let Some(s) = src {
            head.push_str(&format!("Source address: {}\n", up6(s)));
        }
        if let Some(i) = oif.filter(|_| dst.is_link_local()) {
            let n = self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default();
            head.push_str(&format!("Output interface: {}\n", n));
        }
        self.out(ni, &head);
        let o = Ping6Opts { count, size: bytes - 48, hops: None, no_frag: false, interval: 0, timeout, src, oif };
        let Ok(id) = self.ping6(dev, dst, o) else {
            return Ok(());
        };
        self.cli_start_job(ni, id, JobOut::Ping6 { ios: true, col: 0, seen: 0, sent: 0, last_send: 0, head: true, t0: self.now, shown: String::new(), from: None });
        Ok(())
    }

    /// "traceroute ipv6 X", "traceroute X:X::X"
    pub(crate) fn ios_trace6(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let target = p.text("target").unwrap_or("");
        let Some(dst) = Ipv6::parse(target) else {
            return self.ios_err(ni, "% Unrecognized host or address, or protocol not running.\n\n");
        };
        if self.ip6_route(ni, dst, None).is_none() && self.v6(ni).is_none_or(|v| v.find(dst).is_none()) {
            return self.ios_err(ni, "% No valid route for destination\n");
        }
        let probes = p.num("probes").unwrap_or(3).clamp(1, 255) as u8;
        let timeout = p.num("secs").unwrap_or(3) as Ns * SEC;
        let max = p.num("max").unwrap_or(30).clamp(1, 255) as u8;
        self.out(ni, &format!("Ctrl+Shift+6 stops the traceroute.\nRoute to {}, up to {} hops:\n", up6(dst), max));
        let o = TraceOpts { max_hops: max, probes, timeout, port: 33434, size: 0 };
        let Ok(id) = self.traceroute6(dev, dst, o, None) else {
            return Ok(());
        };
        self.cli_start_job(ni, id, JobOut::Trace6 { ios: true, hop: 0, probe: 0, last: None, wrote: false, numeric: true, head: true, shown: String::new() });
        Ok(())
    }

    // ---------------------------------------------------------------- show

    /// "[up/up]" of an interface
    fn ios_if6_state(&self, ni: usize, f: Option<u16>, port: Option<u16>) -> String {
        let (admin, link, proto) = self.if_state(ni, f, port);
        let st = if !admin {
            "administratively down"
        } else if link {
            "up"
        } else {
            "down"
        };
        format!("[{}/{}]", st, if proto { "up" } else { "down" })
    }

    /// " [TEN]", " [DUP]" after an address
    fn addr_flags(a: &crate::nd::Addr6View) -> &'static str {
        match a.state {
            AddrState::Tentative => " [TEN]",
            AddrState::Duplicate => " [DUP]",
            AddrState::Deprecated => " [DEP]",
            AddrState::Preferred => "",
        }
    }

    fn show_ipv6_brief(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        for (name, f, port) in self.ios_ifaces(ni) {
            s.push_str(&format!("{:<22} {}\n", name, self.ios_if6_state(ni, f, port)));
            let addrs = f.map(|i| self.addrs6(dev, i)).unwrap_or_default();
            if addrs.is_empty() {
                s.push_str("    unassigned\n");
            }
            for a in &addrs {
                s.push_str(&format!("    {}{}\n", up6(a.addr), Self::addr_flags(a)));
            }
        }
        s
    }

    fn show_ipv6_interface(&self, ni: usize, only: Option<&str>) -> String {
        let dev = self.dev_of(ni);
        let Some(v) = self.v6(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for (name, f, port) in self.ios_ifaces(ni) {
            if only.is_some_and(|o| !o.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let (admin, link, proto) = self.if_state(ni, f, port);
            let state = if !admin {
                "administratively down"
            } else if link {
                "up"
            } else {
                "down"
            };
            let on = f.is_some_and(|i| v.on(i));
            if !on {
                if only.is_some() {
                    s.push_str(&format!("{} is {}, line protocol is {}\n  IPv6 is not enabled on this interface\n", name, state, if proto { "up" } else { "down" }));
                }
                continue;
            }
            let Some(i) = f else {
                continue;
            };
            let Some(x) = v.iface(i) else {
                continue;
            };
            let addrs = self.addrs6(dev, i);
            s.push_str(&format!("{} is {}, line protocol is {}\n", name, state, if proto { "up" } else { "down" }));
            match addrs.iter().find(|a| a.addr.is_link_local()) {
                Some(ll) => s.push_str(&format!("  IPv6 is enabled, link-local address is {}{}\n", up6(ll.addr), Self::addr_flags(ll))),
                None => s.push_str("  IPv6 is enabled, link-local address is not set up yet\n"),
            }
            s.push_str("  No Virtual link-local address(es):\n");
            let globals: Vec<&crate::nd::Addr6View> = addrs.iter().filter(|a| !a.addr.is_link_local()).collect();
            if globals.is_empty() {
                s.push_str("  No global unicast address is configured\n");
            } else {
                s.push_str("  Global unicast address(es):\n");
                for a in globals {
                    let mut fl: Vec<&str> = Vec::new();
                    if a.anycast {
                        fl.push("ANY");
                    }
                    if matches!(a.origin, Origin6::Eui64 | Origin6::Slaac) {
                        fl.push("EUI");
                    }
                    if a.origin == Origin6::Slaac {
                        fl.push("CAL");
                    }
                    match a.state {
                        AddrState::Tentative => fl.push("TEN"),
                        AddrState::Duplicate => fl.push("DUP"),
                        AddrState::Deprecated => fl.push("DEP"),
                        AddrState::Preferred if a.origin == Origin6::Slaac => fl.push("PRE"),
                        AddrState::Preferred => {}
                    }
                    let flags = if fl.is_empty() { String::new() } else { format!(" [{}]", fl.join("/")) };
                    s.push_str(&format!("    {}, subnet is {}{}\n", up6(a.addr), upp(Prefix6 { addr: a.addr, len: a.len }.trunc()), flags));
                    if a.origin == Origin6::Slaac {
                        let left = |x: Option<Ns>| x.map_or(String::from("infinite"), |n| format!("{}", n / SEC));
                        s.push_str(&format!("      valid lifetime {} preferred lifetime {}\n", left(a.valid), left(a.preferred)));
                    }
                }
            }
            s.push_str("  Joined group address(es):\n");
            for g in v.groups(i) {
                s.push_str(&format!("    {}\n", up6(g)));
            }
            s.push_str(&format!("  MTU is {} bytes\n", self.ip6_mtu(ni, i)));
            s.push_str("  ICMP error messages limited to one every 100 milliseconds\n  ICMP redirects are disabled\n  ICMP unreachables are sent\n");
            if x.dad_transmits == 0 {
                s.push_str("  ND DAD is disabled\n");
            } else {
                s.push_str(&format!("  ND DAD is enabled, number of DAD attempts: {}\n", x.dad_transmits));
            }
            s.push_str(&format!("  ND reachable time is {} milliseconds (using {})\n", x.base_reachable / MS, x.reachable / MS));
            if v.routing {
                let reach = if x.ra.reachable == 0 { String::from("0 (unspecified)") } else { format!("{} milliseconds", x.ra.reachable) };
                s.push_str(&format!("  ND advertised reachable time is {}\n  ND advertised retransmit interval is 0 (unspecified)\n", reach));
                if x.ra.suppress_all {
                    s.push_str("  ND router advertisements are suppressed, solicitations are not answered\n");
                } else if x.ra.suppress {
                    s.push_str("  ND router advertisements are only sent when solicited\n");
                } else if x.ra.min_interval.is_some() {
                    s.push_str(&format!("  ND router advertisements are sent every {} to {} seconds\n", x.ra.min() / SEC, x.ra.max_interval / SEC));
                } else {
                    s.push_str(&format!("  ND router advertisements are sent every {} seconds\n", x.ra.max_interval / SEC));
                }
                s.push_str(&format!("  ND router advertisements live for {} seconds\n  ND advertised default router preference is Medium\n", x.ra.lifetime));
                s.push_str(if x.ra.managed { "  Hosts use DHCP to obtain routable addresses.\n" } else { "  Hosts use stateless autoconfig for addresses.\n" });
            } else if let Some(r) = v.routers.iter().find(|r| r.iface == i) {
                s.push_str(&format!("  Default router is {} on {}\n", up6(r.addr), name));
            }
        }
        s
    }

    fn show_ipv6_route(&self, ni: usize, only: Option<&str>) -> String {
        let dev = self.dev_of(ni);
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let name = |i: Option<u16>| i.and_then(|i| st.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default();
        let mut routes: Vec<Route6> = self.ip6_routes(dev).into_iter().filter(|r| !(r.src == Src6::Connected && r.prefix.addr.is_link_local())).collect();
        // per prefix only the routes of the best distance are in the table
        let best = |r: &Route6, all: &[Route6]| all.iter().filter(|x| x.prefix == r.prefix).map(|x| x.ad).min() == Some(r.ad);
        let all = routes.clone();
        routes.retain(|r| best(r, &all));
        let keep = |r: &Route6| match only {
            Some("connected") => r.src == Src6::Connected,
            Some("local") => r.src == Src6::Local,
            Some("static") => r.src == Src6::Static,
            Some("nd") => matches!(r.src, Src6::RaDefault | Src6::RaPrefix),
            _ => true,
        };
        routes.retain(keep);
        // IOS keeps the multicast range in every table
        let mcast = only.is_none_or(|o| o == "local");
        let n = routes.len() + mcast as usize;
        let mut s = format!("IPv6 Routing Table - default - {} entries\n{}", n, ROUTE_CODES6);
        let mut items: Vec<(u128, u8, String)> = Vec::new();
        for r in &routes {
            let head = format!("{:<4}{} [{}/0]\n", r.src.code(), upp(r.prefix), r.ad);
            let via = match (r.src, r.via) {
                (Src6::Local, _) => format!("     via {}, receive\n", name(r.iface)),
                (_, None) => format!("     via {}, directly connected\n", name(r.iface)),
                (Src6::Static, Some(g)) if !st.v6.statics.iter().any(|x| x.prefix == r.prefix && x.via == Some(g) && x.iface.is_some()) => format!("     via {}\n", up6(g)),
                (_, Some(g)) => format!("     via {}, {}\n", up6(g), name(r.iface)),
            };
            items.push((r.prefix.addr.to_u128(), r.prefix.len, format!("{}{}", head, via)));
        }
        if mcast {
            items.push((0xFF00u128 << 112, 8, String::from("L   FF00::/8 [0/0]\n     via Null0, receive\n")));
        }
        items.sort_by_key(|x| (x.0, x.1));
        for (_, _, t) in items {
            s.push_str(&t);
        }
        s
    }

    fn show_ipv6_neighbors(&self, ni: usize, only: Option<&str>) -> String {
        let dev = self.dev_of(ni);
        let mut s = format!("{:<40} {:>4} {:<15} {:<5} {}\n", "IPv6 Address", "Age", "Link-layer Addr", "State", "Interface");
        let Some(st) = self.stack(ni) else {
            return s;
        };
        for r in self.ip6_neighbors(dev) {
            let ifn = st.ifaces.get(r.iface as usize).map(|f| f.name.clone()).unwrap_or_default();
            if only.is_some_and(|o| !o.eq_ignore_ascii_case(&ifn)) {
                continue;
            }
            let age = if r.fixed { String::from("-") } else { format!("{}", r.age / (60 * SEC)) };
            let mac = r.mac.map_or(String::from("-"), |m| m.cisco());
            let state = if r.fixed { "REACH" } else { r.state.ios() };
            s.push_str(&format!("{:<40} {:>4} {:<15} {:<5} {}\n", up6(r.ip), age, mac, state, jfn_model::ifname::short(&ifn)));
        }
        s
    }

    fn show_ipv6_routers(&self, ni: usize) -> String {
        let Some(v) = self.v6(ni) else {
            return String::new();
        };
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for (i, f) in v.ifs.iter().enumerate() {
            let name = st.ifaces.get(i).map(|x| x.name.clone()).unwrap_or_default();
            for (r, at, life, prefixes) in &f.heard {
                s.push_str(&format!("Router {} on {}, last update {} min\n", up6(*r), name, self.now.saturating_sub(*at) / (60 * SEC)));
                s.push_str(&format!("  Hops {}, Lifetime {} sec, AddrFlag=0, OtherFlag=0, MTU={}\n", f.hop_limit.unwrap_or(64), life, f.link_mtu.unwrap_or(1500)));
                for p in prefixes {
                    let fl = match (p.on_link, p.auto) {
                        (true, true) => " onlink autoconfig",
                        (true, false) => " onlink",
                        (false, true) => " autoconfig",
                        (false, false) => "",
                    };
                    let life = |x: u32| if x == netpkt::icmpv6::INFINITE { String::from("infinite") } else { format!("{}", x) };
                    s.push_str(&format!("  Prefix {}{}\n    Valid lifetime {}, preferred lifetime {}\n", upp(Prefix6 { addr: p.prefix, len: p.len }), fl, life(p.valid), life(p.preferred)));
                }
            }
        }
        s
    }

    // ---------------------------------------------------------------- running configuration

    /// the global lines ("ipv6 hop-limit", "ipv6 unicast-routing")
    pub(crate) fn ios_ip6_global_config(&self, ni: usize) -> String {
        let Some(v) = self.v6(ni) else {
            return String::new();
        };
        let mut s = String::new();
        if v.hop_limit != 64 {
            s.push_str(&format!("ipv6 hop-limit {}\n", v.hop_limit));
        }
        if v.routing {
            s.push_str("ipv6 unicast-routing\n");
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// the IPv6 lines of an interface block
    pub(crate) fn ios_ip6_if_config(&self, ni: usize, i: u16) -> String {
        let Some(f) = self.v6(ni).and_then(|v| v.iface(i)) else {
            return String::new();
        };
        let mut s = String::new();
        if let Some(ll) = f.ll_manual {
            s.push_str(&format!(" ipv6 address {} link-local\n", up6(ll)));
        }
        for c in &f.cfg {
            let tail = if c.eui64 {
                " eui-64"
            } else if c.anycast {
                " anycast"
            } else {
                ""
            };
            let p = if c.eui64 { upp(c.prefix.trunc()) } else { upp(c.prefix) };
            s.push_str(&format!(" ipv6 address {}{}\n", p, tail));
        }
        if f.autoconf {
            s.push_str(if f.default_route { " ipv6 address autoconfig default\n" } else { " ipv6 address autoconfig\n" });
        }
        if f.explicit {
            s.push_str(" ipv6 enable\n");
        }
        if let Some(m) = f.mtu {
            s.push_str(&format!(" ipv6 mtu {}\n", m));
        }
        let life = |x: u32| if x == netpkt::icmpv6::INFINITE { String::from("infinite") } else { format!("{}", x) };
        for c in &f.ra.prefixes {
            if c.hidden {
                s.push_str(&format!(" ipv6 nd prefix {} no-advertise\n", upp(c.prefix)));
                continue;
            }
            let mut l = format!(" ipv6 nd prefix {} {} {}", upp(c.prefix), life(c.valid), life(c.preferred));
            if !c.on_link {
                l.push_str(" off-link");
            }
            if !c.auto {
                l.push_str(" no-autoconfig");
            }
            s.push_str(&l);
            s.push('\n');
        }
        if f.ra.managed {
            s.push_str(" ipv6 nd managed-config-flag\n");
        }
        if f.ra.other {
            s.push_str(" ipv6 nd other-config-flag\n");
        }
        if f.ra.lifetime != 1800 {
            s.push_str(&format!(" ipv6 nd ra lifetime {}\n", f.ra.lifetime));
        }
        if f.ra.max_interval != 200 * SEC || f.ra.min_interval.is_some() {
            match f.ra.min_interval {
                Some(m) => s.push_str(&format!(" ipv6 nd ra interval {} {}\n", f.ra.max_interval / SEC, m / SEC)),
                None => s.push_str(&format!(" ipv6 nd ra interval {}\n", f.ra.max_interval / SEC)),
            }
        }
        if !f.ra.mtu {
            s.push_str(" ipv6 nd ra mtu suppress\n");
        }
        if f.ra.suppress_all {
            s.push_str(" ipv6 nd ra suppress all\n");
        } else if f.ra.suppress {
            s.push_str(" ipv6 nd ra suppress\n");
        }
        if f.dad_transmits != 1 {
            s.push_str(&format!(" ipv6 nd dad attempts {}\n", f.dad_transmits));
        }
        if f.base_reachable != crate::ip6::REACHABLE_TIME {
            s.push_str(&format!(" ipv6 nd reachable-time {}\n", f.base_reachable / MS));
        }
        if f.retrans != crate::ip6::RETRANS_TIMER {
            s.push_str(&format!(" ipv6 nd ns-interval {}\n", f.retrans / MS));
        }
        s
    }

    /// "ipv6 route" and "ipv6 neighbor" lines
    pub(crate) fn ios_ip6_route_config(&self, ni: usize) -> String {
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        let v = &st.v6;
        let mut s = String::new();
        let mut routes: Vec<&Route6> = v.statics.iter().collect();
        routes.sort_by_key(|r| (r.prefix.addr.to_u128(), r.prefix.len, r.via.map(|g| g.to_u128()).unwrap_or(0)));
        for r in &routes {
            let mut l = format!("ipv6 route {}", upp(r.prefix));
            if let Some(i) = r.iface {
                l.push(' ');
                l.push_str(&st.ifaces[i as usize].name);
            }
            if let Some(g) = r.via {
                l.push_str(&format!(" {}", up6(g)));
            }
            if r.ad != 1 {
                l.push_str(&format!(" {}", r.ad));
            }
            s.push_str(&l);
            s.push('\n');
        }
        let mut n = 0;
        for ((i, ip), e) in &v.neigh {
            if e.fixed
                && let Some(m) = e.mac
            {
                s.push_str(&format!("ipv6 neighbor {} {} {}\n", up6(*ip), st.ifaces[*i as usize].name, m.cisco()));
                n += 1;
            }
        }
        if !routes.is_empty() || n > 0 {
            s.push_str("!\n");
        }
        s
    }

    /// the lines of "show debugging" for IPv6
    pub(crate) fn ios_debug6_lines(d: u32) -> String {
        let mut s = String::new();
        if d & D_ICMP6 != 0 {
            s.push_str("Debug output for ICMPv6: on\n");
        }
        if d & D_ND != 0 {
            s.push_str("Debug output for IPv6 Neighbor Discovery: on\n");
        }
        s
    }

    // ---------------------------------------------------------------- debug

    /// the debug lines of an IPv6 decision ("debug ipv6 icmp", "debug ipv6 nd"; our own words)
    pub(crate) fn ip6_debug(&self, ni: usize, hop: Option<crate::trace::HopId>, n: &Note6, d: u32) -> String {
        let ifn = |i: u16| self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default();
        if d & D_ICMP6 != 0 && n.is_icmp() {
            return match n {
                Note6::EchoAnswered { to } => format!("ICMPv6: echo reply sent to {}\n", up6(*to)),
                Note6::IcmpSent { typ, code, to } => format!("ICMPv6: sent {} to {}\n", netpkt::icmpv6::name(*typ, *code), up6(*to)),
                Note6::IcmpLimited => String::from("ICMPv6: an error held back by the rate limit\n"),
                Note6::Local { .. } => {
                    let frame = hop.and_then(|h| self.trace.hop(h)).map(|h| h.bytes.clone());
                    let Some(f) = frame else {
                        return String::new();
                    };
                    let st = netpkt::decode::stack(&f);
                    let (Some((o6, _)), Some((oi, _))) = (st.find(netpkt::decode::Proto::Ipv6), st.find(netpkt::decode::Proto::Icmpv6)) else {
                        return String::new();
                    };
                    let Some(v) = netpkt::ipv6::Ipv6View::parse(&f[o6..]) else {
                        return String::new();
                    };
                    let t = f.get(oi).copied().unwrap_or(0);
                    let c = f.get(oi + 1).copied().unwrap_or(0);
                    match t {
                        netpkt::icmpv6::T_ECHO_REQUEST => format!("ICMPv6: echo request from {}\n", up6(v.src())),
                        netpkt::icmpv6::T_ECHO_REPLY => format!("ICMPv6: echo reply from {}\n", up6(v.src())),
                        x if x < 128 => format!("ICMPv6: received {} from {}\n", netpkt::icmpv6::name(x, c), up6(v.src())),
                        _ => String::new(),
                    }
                }
                _ => String::new(),
            };
        }
        if d & D_ND == 0 || !n.is_nd() {
            return String::new();
        }
        let p = |a: Ipv6| up6(a);
        match n {
            Note6::DadStart { iface, addr, delay } => format!("ICMPv6-ND: DAD for {} on {} starts in {} ms\n", p(*addr), ifn(*iface), delay / MS),
            Note6::DadProbe { iface, addr } => format!("ICMPv6-ND: DAD probe for {} on {}\n", p(*addr), ifn(*iface)),
            Note6::DadOk { iface, addr } => format!("ICMPv6-ND: {} on {} is unique, in use now\n", p(*addr), ifn(*iface)),
            Note6::DadDup { iface, addr, .. } => format!("ICMPv6-ND: {} on {} belongs to another node\n", p(*addr), ifn(*iface)),
            Note6::AddrNew { iface, addr, .. } => format!("ICMPv6-ND: address {} on {}\n", upp(*addr), ifn(*iface)),
            Note6::AddrLifetime { iface, addr, valid, preferred } => format!("ICMPv6-ND: {} on {} valid {} s, preferred {} s\n", p(*addr), ifn(*iface), valid, preferred),
            Note6::AddrDeprecated { iface, addr } => format!("ICMPv6-ND: {} on {} is deprecated\n", p(*addr), ifn(*iface)),
            Note6::AddrExpired { iface, addr } => format!("ICMPv6-ND: {} on {} expired\n", p(*addr), ifn(*iface)),
            Note6::AddrRemoved { iface, addr } => format!("ICMPv6-ND: {} on {} removed\n", p(*addr), ifn(*iface)),
            Note6::NsSent { iface, target, unicast } => format!("ICMPv6-ND: NS for {} on {}{}\n", p(*target), ifn(*iface), if *unicast { " (probe)" } else { "" }),
            Note6::NsHeard { iface, target, from } => format!("ICMPv6-ND: NS for {} from {} on {}\n", p(*target), p(*from), ifn(*iface)),
            Note6::NaSent { iface, target, to, .. } => format!("ICMPv6-ND: NA for {} to {} on {}\n", p(*target), p(*to), ifn(*iface)),
            Note6::NaHeard { iface, target, mac, .. } => match mac {
                Some(m) => format!("ICMPv6-ND: NA for {} on {}: at {}\n", p(*target), ifn(*iface), m.cisco()),
                None => format!("ICMPv6-ND: NA for {} on {}\n", p(*target), ifn(*iface)),
            },
            Note6::Neigh { iface, addr, from: None, to, .. } => format!("ICMPv6-ND: new entry {} on {} ({})\n", p(*addr), ifn(*iface), to.ios()),
            Note6::Neigh { iface, addr, from: Some(f), to, .. } => format!("ICMPv6-ND: {} on {}: {} -> {}\n", p(*addr), ifn(*iface), f.ios(), to.ios()),
            Note6::NeighFailed { addr } => format!("ICMPv6-ND: {} does not answer, entry deleted\n", p(*addr)),
            Note6::NdBad { why } => format!("ICMPv6-ND: message dropped: {}\n", why.text()),
            Note6::RsSent { iface } => format!("ICMPv6-ND: RS on {}\n", ifn(*iface)),
            Note6::RsHeard { iface, from } => format!("ICMPv6-ND: RS from {} on {}\n", p(*from), ifn(*iface)),
            Note6::RaSent { iface, lifetime, prefixes, solicited, .. } => format!("ICMPv6-ND: RA on {}, lifetime {} s, {} prefix{}{}\n", ifn(*iface), lifetime, prefixes, if *prefixes == 1 { "" } else { "es" }, if *solicited { " (solicited)" } else { "" }),
            Note6::RaHeard { iface, from, lifetime, prefixes } => format!("ICMPv6-ND: RA from {} on {}, lifetime {} s, {} prefix{}\n", p(*from), ifn(*iface), lifetime, prefixes, if *prefixes == 1 { "" } else { "es" }),
            Note6::RaIgnored { iface, why } => format!("ICMPv6-ND: RA on {} ignored ({})\n", ifn(*iface), if *why == RaWhy::Router { "this device routes" } else { "no autoconfig here" }),
            Note6::RouterAdded { iface, router, lifetime } => format!("ICMPv6-ND: default router {} on {} for {} s\n", p(*router), ifn(*iface), lifetime),
            Note6::RouterGone { iface, router } => format!("ICMPv6-ND: default router {} on {} gone\n", p(*router), ifn(*iface)),
            Note6::PrefixOnLink { iface, prefix, valid } => format!("ICMPv6-ND: prefix {} on {} for {} s\n", upp(*prefix), ifn(*iface), valid),
            Note6::PrefixGone { iface, prefix } => format!("ICMPv6-ND: prefix {} on {} expired\n", upp(*prefix), ifn(*iface)),
            _ => String::new(),
        }
    }
}
