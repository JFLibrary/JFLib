// iosshow -- what the show commands of an IOS-like device print, and its running configuration
//
//   Engine::ios_show(ni, cmd)    show running-config | startup-config | ip interface brief |
//                                ip route | arp | mac address-table | vlan [brief] |
//                                interfaces [NAME] | interfaces trunk | debugging | version
//   Engine::ios_config(ni)       the configuration from "!" to "end" (what copy run start
//                                stores and the startup replay reads back; the DHCP lines
//                                come from netsvc.rs, the spanning tree's from stp.rs, "router
//                                rip" and "ip rip" from rip.rs, those of NAT and access lists
//                                from nat.rs and acl.rs, the IPv6 lines from ip6.rs)
//   show ip route [connected|static|rip]   RIP routes with the age of their last update and
//                                their interface; the legend names R only where RIP runs
//   show ip route ospf           OSPF routes as O, O IA, O E1 and O E2 (cli/ospf.rs) with the
//                                time since they are as they are and their interface; the legend
//                                names them only where OSPF runs
//   show ip route bgp            BGP routes as B with the time since they are as they are, without
//                                an interface (the next hop may be further away); the legend
//                                names B only where BGP runs (cli/bgp.rs)
//   show interfaces              a port whose cable is too long or whose optics do not fit
//                                (phys.rs, P1) gets a line in our own words that says so
//
// The layouts follow IOS 15 / IOS-XE: column widths of "show ip interface brief" (22, 15, 3,
// 6, 21), "show arp" (address 15, age right-aligned in 9), "show mac address-table" and "show
// vlan brief" (ports wrapped at 31 characters), the codes and classful groups of "show ip
// route" ("is variably subnetted, 4 subnets, 2 masks"; entries of a group with one mask
// without their length).  The route legend, the headers of the configurations and show
// version are our own words.  Values come from the engine (View, routes, arp_table, bridge).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::Parsed;
use jfn_model::catalog::{speed_str, Media, F_AP, F_BRIDGE, F_HOST, F_ROUTER};
use jfn_model::ifname;
use netpkt::{Ipv4, Prefix4};

use super::fmt::{ios_hw, mask_str, pad};
use super::ios::K;
use super::{Method, D_ARP, D_ICMP, D_RIP};
use crate::bridge::{PortMode, SwMode, VlanSet};
use crate::ip::{IfKind, Route, RouteSrc};
use crate::kernel::{Engine, SEC};

const ROUTE_CODES: &str = "Codes: C - connected network, L - address of this device, S - static route,\n       * - default route\n\n";
/// the codes on a device that runs RIP
const ROUTE_CODES_RIP: &str = "Codes: C - connected network, L - address of this device, S - static route,\n       R - learned by RIP, * - default route\n\n";
/// the codes on a device that runs OSPF, and on one that runs both
const ROUTE_CODES_OSPF: &str = "Codes: C - connected network, L - address of this device, S - static route,\n       O - learned by OSPF, IA - from another area, E1 / E2 - external of type 1 / 2,\n       * - default route\n\n";
const ROUTE_CODES_BOTH: &str = "Codes: C - connected network, L - address of this device, S - static route,\n       R - learned by RIP, O - learned by OSPF, IA - from another area,\n       E1 / E2 - external of type 1 / 2, * - default route\n\n";

/// the codes on a device that runs BGP (and RIP or OSPF)
fn route_codes_bgp(rip: bool, ospf: bool) -> String {
    let mut s = String::from("Codes: C - connected network, L - address of this device, S - static route,\n");
    s.push_str(match (rip, ospf) {
        (false, false) => "       B - learned by BGP, * - default route\n\n",
        (true, false) => "       R - learned by RIP, B - learned by BGP, * - default route\n\n",
        (false, true) => "       O - learned by OSPF, IA - from another area, E1 / E2 - external of type 1 / 2,\n       B - learned by BGP, * - default route\n\n",
        (true, true) => "       R - learned by RIP, O - learned by OSPF, IA - from another area,\n       E1 / E2 - external of type 1 / 2, B - learned by BGP, * - default route\n\n",
    });
    s
}

/// the length of the classful network of an address
fn classful(a: Ipv4) -> u8 {
    match a.0[0] {
        0..=127 => 8,
        128..=191 => 16,
        192..=223 => 24,
        _ => 32,
    }
}

/// VLAN ports in the IOS way: "Gi1/0/1, Gi1/0/2" wrapped at 31 characters
fn port_lines(ports: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for p in ports {
        if !cur.is_empty() && cur.len() + 2 + p.len() > 31 {
            lines.push(core::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str(", ");
        }
        cur.push_str(p);
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

impl Engine {
    /// the output of a show command
    pub(crate) fn ios_show(&self, ni: usize, p: &Parsed<K>) -> String {
        match p.cmd {
            K::ShowRun => {
                let body = self.ios_config(ni);
                format!("Collecting the configuration ...\n\n! configuration in effect, {} bytes\n{}", body.len(), body)
            }
            K::ShowStart => {
                let s = self.cdev(ni).map(|c| c.cfg.startup.clone()).unwrap_or_default();
                if s.trim().is_empty() { String::from("No startup configuration saved yet\n") } else { format!("! startup configuration, {} bytes\n{}{}", s.len(), s, if s.ends_with('\n') { "" } else { "\n" }) }
            }
            K::ShowIpBrief => self.show_ip_brief(ni),
            K::ShowIpRoute => self.show_ip_route(ni, p.text("only")),
            K::ShowArp => self.show_arp(ni),
            K::ShowMac => self.show_mac(ni),
            K::ShowVlan => self.show_vlan(ni, p.has("brief")),
            K::ShowIf => self.show_interfaces(ni, p.text("if")),
            K::ShowIfTrunk => self.show_trunks(ni),
            K::ShowDebug => {
                let d = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
                let mut s = String::new();
                if d & D_ICMP != 0 {
                    s.push_str("Debug output for ICMP: on\n");
                }
                if d & D_ARP != 0 {
                    s.push_str("Debug output for ARP: on\n");
                }
                if d & super::D_STP != 0 {
                    s.push_str("Debug output for spanning tree events: on\n");
                }
                if d & D_RIP != 0 {
                    s.push_str("Debug output for RIP: on\n");
                }
                if d & super::D_NAT != 0 {
                    s.push_str("Debug output for NAT: on\n");
                }
                if d & super::D_AAA != 0 {
                    s.push_str("Debug output for AAA authentication: on\n");
                }
                if d & super::D_RADIUS != 0 {
                    s.push_str("Debug output for RADIUS: on\n");
                }
                s.push_str(&Self::ios_debug6_lines(d));
                if d & super::D_OSPF != 0 {
                    let k = match self.cdev(ni).map(|c| c.ospf_debug).unwrap_or(0) {
                        0 => super::ospf::DO_ADJ | super::ospf::DO_EVENTS | super::ospf::DO_HELLO,
                        k => k,
                    };
                    for (bit, what) in [(super::ospf::DO_ADJ, "adjacencies"), (super::ospf::DO_EVENTS, "events"), (super::ospf::DO_HELLO, "hellos")] {
                        if k & bit != 0 {
                            s.push_str(&format!("Debug output for OSPF {}: on\n", what));
                        }
                    }
                }
                if d & super::D_BGP != 0 {
                    let k = match self.cdev(ni).map(|c| c.bgp_debug).unwrap_or(0) {
                        0 => super::bgp::DB_EVENTS | super::bgp::DB_UPDATES,
                        k => k,
                    };
                    for (bit, what) in [(super::bgp::DB_EVENTS, "events"), (super::bgp::DB_UPDATES, "updates")] {
                        if k & bit != 0 {
                            s.push_str(&format!("Debug output for BGP {}: on\n", what));
                        }
                    }
                }
                s
            }
            K::ShowVersion => self.show_version(ni),
            _ => String::new(),
        }
    }

    // ---------------------------------------------------------------- running configuration

    /// the configuration from "!" to "end"
    pub(crate) fn ios_config(&self, ni: usize) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return String::new();
        };
        let cfg = &n.cli.cfg;
        let mut s = String::from("!\nversion 1.0\n!\n");
        s.push_str(&format!("hostname {}\n!\n", n.name));
        if !cfg.domain_lookup {
            s.push_str("no ip domain lookup\n!\n");
        }
        // static names and name servers (cli/names.rs)
        s.push_str(&self.ios_names_config(ni));
        // the FTP client's login (cli/files.rs)
        s.push_str(&self.ios_files_config(ni));
        // logins, AAA and RADIUS (cli/aaa.rs)
        s.push_str(&self.ios_aaa_config(ni));
        if let Some(st) = &n.ip {
            let default = n.model.has(F_ROUTER) && (!n.model.has(F_BRIDGE) || n.model.has(F_HOST));
            if st.forwarding != default {
                s.push_str(if st.forwarding { "ip routing\n!\n" } else { "no ip routing\n!\n" });
            }
        }
        // IPv6: "ipv6 unicast-routing" with the global lines (cli/ip6.rs)
        s.push_str(&self.ios_ip6_global_config(ni));
        // excluded addresses and pools of the DHCP server (netsvc.rs)
        s.push_str(&self.ios_dhcp_config(ni));
        // spanning tree (stp.rs)
        s.push_str(&self.ios_stp_config(ni));
        if let Some(b) = &n.bridge {
            for (vid, name) in &b.vlans {
                if *vid == 1 {
                    continue;
                }
                s.push_str(&format!("vlan {}\n", vid));
                if *name != format!("VLAN{:04}", vid) {
                    s.push_str(&format!(" name {}\n", name));
                }
                s.push_str("!\n");
            }
        }
        // interfaces: loopbacks, ports with their subinterfaces, the bridge's interfaces
        let list = self.ios_ifaces(ni);
        let (los, rest): (Vec<_>, Vec<_>) = list.into_iter().partition(|(name, _, _)| name.starts_with("Loopback"));
        let mut items: Vec<(String, Option<u16>, Option<u16>, bool)> = los.into_iter().chain(rest).map(|(n, f, p)| (n, f, p, false)).collect();
        // subinterfaces still waiting for their VLAN go between the others of their port, by number
        let num = |n: &str| n.rsplit_once('.').and_then(|x| x.1.parse::<u32>().ok()).unwrap_or(0);
        for pend in &cfg.pending {
            let Some((base, _)) = pend.rsplit_once('.') else {
                continue;
            };
            let mut at = None;
            for (i, it) in items.iter().enumerate() {
                let same_port = it.0.eq_ignore_ascii_case(base) || it.0.rsplit_once('.').is_some_and(|(b, _)| b.eq_ignore_ascii_case(base) && num(&it.0) < num(pend));
                if same_port {
                    at = Some(i + 1);
                }
            }
            if let Some(i) = at {
                items.insert(i, (pend.clone(), None, None, true));
            }
        }
        for (name, f, port, pending) in &items {
            if *pending {
                s.push_str(&format!("interface {}\n", name));
                if let Some(d) = cfg.descr.get(name) {
                    s.push_str(&format!(" description {}\n", d));
                }
                s.push_str("!\n");
            } else {
                s.push_str(&self.if_config(ni, name, *f, *port));
            }
        }
        // "router rip" (cli/rip.rs)
        s.push_str(&self.ios_rip_config(ni));
        // "router ospf" (cli/ospf.rs)
        s.push_str(&self.ios_ospf_config(ni));
        // "router bgp" (cli/bgp.rs)
        s.push_str(&self.ios_bgp_config(ni));
        // the global lines of NAT come before the static routes (cli/nat.rs)
        s.push_str(&self.ios_nat_config(ni));
        if let Some(st) = &n.ip {
            let mut routes: Vec<&Route> = st.statics.iter().collect();
            routes.sort_by_key(|r| (r.prefix.addr.to_u32(), r.prefix.len, r.via.map(|v| v.to_u32()).unwrap_or(0)));
            for r in &routes {
                let mut l = format!("ip route {} {}", r.prefix.addr, mask_str(r.prefix.len));
                if let Some(i) = r.iface {
                    l.push(' ');
                    l.push_str(&st.ifaces[i as usize].name);
                }
                if let Some(v) = r.via {
                    l.push_str(&format!(" {}", v));
                }
                if r.ad != 1 {
                    l.push_str(&format!(" {}", r.ad));
                }
                s.push_str(&l);
                s.push('\n');
            }
            if !routes.is_empty() {
                s.push_str("!\n");
            }
            let fixed: Vec<_> = st.arp.iter().filter(|(_, e)| e.fixed).collect();
            for ((_, ip), e) in &fixed {
                if let Some(m) = e.mac {
                    s.push_str(&format!("arp {} {} ARPA\n", ip, m.cisco()));
                }
            }
            if !fixed.is_empty() {
                s.push_str("!\n");
            }
        }
        // "ipv6 route" and "ipv6 neighbor" (cli/ip6.rs)
        s.push_str(&self.ios_ip6_route_config(ni));
        // the access lists: named ones, then the numbered lines (cli/acl.rs)
        s.push_str(&self.ios_acl_config(ni));
        for l in &cfg.lines {
            s.push_str(&format!("line {}\n", l.name));
            for o in &l.opts {
                s.push_str(&format!(" {}\n", o));
            }
        }
        // "ntp master", "ntp server" (netsvc.rs)
        s.push_str(&self.ios_ntp_config(ni));
        s.push_str("!\nend\n");
        s
    }

    /// one "interface ..." block
    fn if_config(&self, ni: usize, name: &str, f: Option<u16>, port: Option<u16>) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return String::new();
        };
        let mut s = format!("interface {}\n", name);
        if let Some(d) = n.cli.cfg.descr.get(name) {
            s.push_str(&format!(" description {}\n", d));
        }
        let bp = port.and_then(|p| n.bridge.as_ref().and_then(|b| b.ports.get(p as usize)));
        let switch_port = bp.is_some_and(|b| b.member);
        if let Some(b) = bp.filter(|b| b.member && !n.model.has(F_AP)) {
            if b.access_vlan != 1 {
                s.push_str(&format!(" switchport access vlan {}\n", b.access_vlan));
            }
            if b.native_vlan != 1 {
                s.push_str(&format!(" switchport trunk native vlan {}\n", b.native_vlan));
            }
            if b.allowed != VlanSet::all() {
                s.push_str(&format!(" switchport trunk allowed vlan {}\n", b.allowed.to_text()));
            }
            match b.admin {
                SwMode::Access => s.push_str(" switchport mode access\n"),
                SwMode::Trunk => s.push_str(" switchport mode trunk\n"),
                SwMode::Dynamic => {}
            }
        } else if bp.is_some() && !switch_port && n.model.has(F_BRIDGE) && n.model.has(F_ROUTER) && !n.model.has(F_HOST) {
            s.push_str(" no switchport\n");
        }
        if let (Some(i), Some(st)) = (f, n.ip.as_ref()) {
            let x = &st.ifaces[i as usize];
            if let IfKind::Sub { vlan, .. } = x.kind {
                s.push_str(&format!(" encapsulation dot1Q {}\n", vlan));
            }
            if x.addrs.is_empty() {
                s.push_str(" no ip address\n");
            }
            for (k, a) in x.addrs.iter().enumerate() {
                s.push_str(&format!(" ip address {} {}{}\n", a.addr, mask_str(a.len), if k > 0 { " secondary" } else { "" }));
            }
            // access lists of the interface (cli/acl.rs)
            s.push_str(&self.ios_access_group_config(ni, i));
            s.push_str(&self.ios_helper_config(ni, name));
            s.push_str(&self.ios_rip_if_config(ni, name));
            s.push_str(&self.ios_ospf_if_config(ni, name));
            if x.mtu != 1500 && !x.is_loopback() {
                s.push_str(&format!(" ip mtu {}\n", x.mtu));
            }
            if !x.proxy_arp && !x.is_loopback() {
                s.push_str(" no ip proxy-arp\n");
            }
            // its side of NAT (cli/nat.rs)
            s.push_str(&self.ios_nat_if_config(ni, i));
        }
        let admin = match (port, f) {
            (Some(p), None) | (Some(p), Some(_)) if !name.contains('.') => n.ports.get(p as usize).is_some_and(|x| x.admin_up),
            (_, Some(i)) => n.ip.as_ref().is_some_and(|st| st.ifaces[i as usize].admin_up),
            _ => true,
        };
        if !admin {
            s.push_str(" shutdown\n");
        }
        // its IPv6 lines (cli/ip6.rs)
        if let Some(i) = f {
            s.push_str(&self.ios_ip6_if_config(ni, i));
        }
        // spanning tree of a switch port (stp.rs)
        if let Some(p) = port {
            s.push_str(&self.ios_stp_if_config(ni, p));
        }
        s.push_str("!\n");
        s
    }

    // ---------------------------------------------------------------- interfaces

    fn show_ip_brief(&self, ni: usize) -> String {
        let mut s = format!("{:<22} {:<15} {:<3} {:<6} {:<21} {}\n", "Interface", "IP-Address", "OK?", "Method", "Status", "Protocol");
        let Some(Some(n)) = self.nodes.get(ni) else {
            return s;
        };
        let mut list = self.ios_ifaces(ni);
        if n.bridge.is_some() {
            // switches list their VLAN interfaces first
            let (svi, other): (Vec<_>, Vec<_>) = list.into_iter().partition(|(_, f, _)| f.is_some_and(|f| n.ip.as_ref().is_some_and(|st| matches!(st.ifaces[f as usize].kind, IfKind::Svi(_)))));
            list = svi.into_iter().chain(other).collect();
        }
        for (name, f, port) in list {
            let ip = f.and_then(|f| n.ip.as_ref().and_then(|st| st.ifaces[f as usize].primary()));
            let method = match n.cli.cfg.methods.get(&name) {
                Some(Method::Nvram) => "NVRAM",
                Some(Method::Manual) => "manual",
                _ if ip.is_some() => "manual",
                _ => "unset",
            };
            let (admin, link, proto) = self.if_state(ni, f, port);
            let status = if !admin {
                "administratively down"
            } else if link {
                "up"
            } else {
                "down"
            };
            let ip = ip.map(|a| format!("{}", a)).unwrap_or_else(|| String::from("unassigned"));
            s.push_str(&format!("{:<22} {:<15} YES {:<6} {:<21} {}\n", name, ip, method, status, if proto { "up" } else { "down" }));
        }
        s
    }

    fn show_interfaces(&self, ni: usize, only: Option<&str>) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
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
            let x = f.and_then(|f| n.ip.as_ref().map(|st| &st.ifaces[f as usize]));
            let switch = port.is_some_and(|p| n.bridge.as_ref().is_some_and(|b| b.ports.get(p as usize).is_some_and(|bp| bp.member)));
            let err = port.is_some_and(|p| n.ports.get(p as usize).is_some_and(|x| x.err_disabled));
            let tail = if switch {
                if !admin {
                    " (disabled)"
                } else if err {
                    " (err-disabled)"
                } else if proto {
                    " (connected)"
                } else {
                    " (notconnect)"
                }
            } else {
                ""
            };
            s.push_str(&format!("{} is {}, line protocol is {}{}\n", name, state, if proto { "up" } else { "down" }, tail));
            let pv = port.and_then(|p| n.ports.get(p as usize));
            let mac = x.map(|x| x.mac).or(pv.map(|p| p.mac)).unwrap_or(netpkt::Mac::ZERO);
            let serial = pv.is_some_and(|p| p.spec.media == Media::Serial);
            let kind = x.map(|x| x.kind);
            let hw = match (kind, pv) {
                (Some(IfKind::Loopback(_)), _) => "Loopback",
                (Some(IfKind::Svi(_)), _) => "EtherSVI",
                (_, Some(_)) if serial => "Serial",
                (_, Some(p)) => ios_hw(p.spec.kbps),
                _ => "Ethernet",
            };
            if matches!(kind, Some(IfKind::Loopback(_))) || serial {
                s.push_str(&format!("  Hardware is {}\n", hw));
            } else {
                s.push_str(&format!("  Hardware is {}, address is {} (bia {})\n", hw, mac.cisco(), mac.cisco()));
            }
            if let Some(d) = n.cli.cfg.descr.get(&name) {
                s.push_str(&format!("  Description: {}\n", d));
            }
            if let Some(x) = x {
                for (k, a) in x.addrs.iter().enumerate() {
                    if k == 0 {
                        s.push_str(&format!("  Internet address is {}\n", a));
                    } else {
                        s.push_str(&format!("  Secondary address {}\n", a));
                    }
                }
            }
            let kbps = match (kind, pv) {
                (Some(IfKind::Loopback(_)), _) => 8_000_000,
                (Some(IfKind::Svi(_)), _) => 1_000_000,
                (_, Some(p)) => p.link.map(|li| self.link_kbps(li)).unwrap_or(p.spec.kbps),
                _ => 1_000_000,
            };
            let mtu = match kind {
                Some(IfKind::Loopback(_)) => 1514,
                _ => 1500,
            };
            let dly = match (kind, serial) {
                (Some(IfKind::Loopback(_)), _) => 5000,
                (_, true) => 20000,
                _ => 10,
            };
            s.push_str(&format!("  MTU {} bytes, BW {} Kbit/sec, DLY {} usec,\n     reliability 255/255, txload 1/255, rxload 1/255\n", mtu, kbps, dly));
            match kind {
                Some(IfKind::Loopback(_)) => s.push_str("  Encapsulation LOOPBACK, loopback not set\n"),
                Some(IfKind::Sub { vlan, .. }) => s.push_str(&format!("  Encapsulation 802.1Q Virtual LAN, Vlan ID  {}.\n", vlan)),
                _ if serial => s.push_str("  Encapsulation HDLC, crc 16, loopback not set\n  Keepalive set (10 sec)\n"),
                _ => s.push_str("  Encapsulation ARPA, loopback not set\n  Keepalive set (10 sec)\n"),
            }
            if let Some(p) = pv
                && !serial
                && kind.is_none_or(|k| matches!(k, IfKind::Port(_)))
            {
                let media = if p.spec.media == Media::Rj45 { String::from("RJ45") } else { format!("{} {}", speed_str(p.spec.kbps), p.spec.media.name()) };
                if link {
                    s.push_str(&format!("  Full Duplex, {}Mbps, link type is auto, media type is {}\n", kbps / 1000, media));
                } else {
                    s.push_str(&format!("  Auto Duplex, Auto Speed, link type is auto, media type is {}\n", media));
                }
            }
            // a cable that stays down for a physical reason says why (phys.rs, P1)
            if kind.is_none_or(|k| matches!(k, IfKind::Port(_)))
                && let Some(f) = pv.and_then(|p| p.link).and_then(|li| self.phys.bad.get(&li))
            {
                match *f {
                    crate::phys::PhysFault::TooLong { len_m, max_m, std } => s.push_str(&format!("  Cable is {} m long, {} allows {} m: no link\n", len_m, std, max_m)),
                    crate::phys::PhysFault::Optics(_) => s.push_str("  Optics do not fit the far end or the fibre: no link\n"),
                }
            }
            if x.is_some() && !serial && !matches!(kind, Some(IfKind::Loopback(_))) {
                s.push_str("  ARP type: ARPA, ARP Timeout 04:00:00\n");
            }
            let c = pv.map(|p| p.counters).unwrap_or_default();
            let last = |n: u64| if n > 0 { "00:00:00" } else { "never" };
            s.push_str(&format!("  Last input {}, output {}, output hang never\n", last(c.rx_frames), last(c.tx_frames)));
            s.push_str("  Last clearing of \"show interface\" counters never\n");
            s.push_str(&format!("  Input queue: 0/375/0/0 (size/max/drops/flushes); Total output drops: {}\n", c.drops));
            s.push_str("  Queueing strategy: fifo\n  Output queue: 0/40 (size/max)\n");
            s.push_str("  5 minute input rate 0 bits/sec, 0 packets/sec\n  5 minute output rate 0 bits/sec, 0 packets/sec\n");
            s.push_str(&format!("     {} packets input, {} bytes, 0 no buffer\n", c.rx_frames, c.rx_bytes));
            s.push_str("     Received 0 broadcasts (0 IP multicasts)\n     0 runts, 0 giants, 0 throttles\n     0 input errors, 0 CRC, 0 frame, 0 overrun, 0 ignored\n");
            s.push_str(&format!("     {} packets output, {} bytes, 0 underruns\n", c.tx_frames, c.tx_bytes));
            s.push_str("     0 output errors, 0 collisions, 0 interface resets\n     0 unknown protocol drops\n");
            if !serial {
                s.push_str("     0 babbles, 0 late collision, 0 deferred\n     0 lost carrier, 0 no carrier\n");
            }
            s.push_str("     0 output buffer failures, 0 output buffers swapped out\n");
        }
        s
    }

    fn show_trunks(&self, ni: usize) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return String::new();
        };
        let Some(b) = n.bridge.as_ref() else {
            return String::new();
        };
        let mut rows: Vec<(String, u16, VlanSet)> = Vec::new();
        for (pi, bp) in b.ports.iter().enumerate() {
            if let PortMode::Trunk { native, allowed } = &bp.mode
                && bp.member
                && n.ports[pi].link.is_some_and(|li| self.link_up(li))
            {
                rows.push((ifname::short(&n.ports[pi].spec.name), *native, allowed.clone()));
            }
        }
        if rows.is_empty() {
            return String::new();
        }
        let mut s = String::from("\nPort        Mode             Encapsulation  Status        Native vlan\n");
        for (p, native, _) in &rows {
            s.push_str(&format!("{}{}{}{}{}\n", pad(p, 12), pad("on", 17), pad("802.1q", 15), pad("trunking", 14), native));
        }
        s.push_str("\nPort        Vlans allowed on trunk\n");
        for (p, _, a) in &rows {
            s.push_str(&format!("{}{}\n", pad(p, 12), a.to_text()));
        }
        let active = |a: &VlanSet| {
            let mut v = VlanSet::none();
            for vid in b.vlans.keys() {
                if a.contains(*vid) {
                    v.insert(*vid);
                }
            }
            v.to_text()
        };
        s.push_str("\nPort        Vlans allowed and active in management domain\n");
        for (p, _, a) in &rows {
            s.push_str(&format!("{}{}\n", pad(p, 12), active(a)));
        }
        s.push_str("\nPort        Vlans in spanning tree forwarding state and not pruned\n");
        for (p, _, a) in &rows {
            s.push_str(&format!("{}{}\n", pad(p, 12), active(a)));
        }
        s
    }

    // ---------------------------------------------------------------- routes, ARP, MAC, VLANs

    /// "show ip route [connected|static|rip|ospf]"
    fn show_ip_route(&self, ni: usize, only: Option<&str>) -> String {
        let dev = self.dev_of(ni);
        let Some(st) = self.stack(ni) else {
            return String::new();
        };
        if !st.forwarding {
            return String::from("Default gateway is not set\n\nHost               Gateway           Last Use    Total Uses  Interface\nICMP redirect cache is empty\n");
        }
        let mut all = self.routes(dev);
        let rip = self.rip_on(ni);
        let rip_db = self.rip_of(ni);
        let ospf = self.ospf_on(ni);
        let bgp = self.bgp_on(ni);
        // per prefix only the routes of the best distance are in the table
        let mut best: BTreeMap<(u32, u8), u8> = BTreeMap::new();
        for r in &all {
            let k = (r.prefix.addr.to_u32(), r.prefix.len);
            let e = best.entry(k).or_insert(r.ad);
            *e = (*e).min(r.ad);
        }
        // "show ip route rip" and the like: only the routes of that source
        let keep = |r: &Route| match only {
            Some("rip") => r.src == RouteSrc::Rip,
            Some("ospf") => r.src == RouteSrc::Ospf,
            Some("bgp") => r.src == RouteSrc::Bgp,
            Some("static") => r.src == RouteSrc::Static,
            Some("connected") => matches!(r.src, RouteSrc::Connected | RouteSrc::Local),
            _ => true,
        };
        all.retain(|r| best.get(&(r.prefix.addr.to_u32(), r.prefix.len)) == Some(&r.ad) && keep(r));
        let routes: Vec<&Route> = all.iter().collect();
        let mut s = if bgp {
            route_codes_bgp(rip, ospf)
        } else {
            String::from(match (rip, ospf) {
                (true, true) => ROUTE_CODES_BOTH,
                (true, false) => ROUTE_CODES_RIP,
                (false, true) => ROUTE_CODES_OSPF,
                (false, false) => ROUTE_CODES,
            })
        };
        let default = routes.iter().find(|r| r.prefix.len == 0);
        match default {
            Some(r) => s.push_str(&format!("Gateway of last resort is {} to network 0.0.0.0\n\n", r.via.unwrap_or(Ipv4::UNSPECIFIED))),
            None => s.push_str("Gateway of last resort is not set\n\n"),
        }
        // top-level entries and classful groups in address order
        let mut groups: BTreeMap<u32, Vec<&Route>> = BTreeMap::new();
        let mut items: Vec<(u32, u8, Option<u32>)> = Vec::new();
        for r in &routes {
            let c = classful(r.prefix.addr);
            if r.prefix.len < c {
                items.push((r.prefix.addr.to_u32(), r.prefix.len, None));
            } else {
                let major = Prefix4 { addr: r.prefix.addr, len: c }.trunc().addr.to_u32();
                if !groups.contains_key(&major) {
                    items.push((major, c, Some(major)));
                }
                groups.entry(major).or_default().push(r);
            }
        }
        items.sort();
        items.dedup();
        let name = |i: Option<u16>| i.and_then(|i| st.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default();
        let configured_if = |r: &Route| st.statics.iter().any(|x| x.prefix == r.prefix && x.via == r.via && x.iface.is_some());
        let entry = |s: &mut String, rs: &[&Route], width: usize, with_len: bool| {
            let r0 = rs[0];
            let code = if r0.src == RouteSrc::Ospf {
                // "O", "O IA", "O E2", "O*E2" (cli/ospf.rs)
                self.ospf_route_code(ni, r0)
            } else {
                let mut c = String::from(r0.src.code());
                if r0.prefix.len == 0 {
                    c.push('*');
                }
                c
            };
            let net = if with_len { format!("{}", r0.prefix) } else { format!("{}", r0.prefix.addr) };
            let head = format!("{}{}", pad(&code, width), net);
            for (k, r) in rs.iter().enumerate() {
                let lead = if k == 0 { head.clone() } else { pad("", head.len()) };
                let line = match (r.src, r.via) {
                    (RouteSrc::Connected | RouteSrc::Local, _) | (_, None) => format!("{} is directly connected, {}", lead, name(r.iface)),
                    (_, Some(v)) => {
                        let mut l = format!("{} [{}/{}] via {}", lead, r.ad, r.metric, v);
                        if r.src == RouteSrc::Rip {
                            // RIP: how long ago the route was heard, and the interface
                            let age = rip_db.and_then(|x| x.age(r.prefix, v, self.now)).unwrap_or(0);
                            l.push_str(&format!(", {}, {}", super::rip::hms(age), name(r.iface)));
                        } else if r.src == RouteSrc::Ospf {
                            // OSPF: how long the route is as it is, and the interface
                            l.push_str(&format!(", {}, {}", super::rip::hms(self.ospf_route_age(ni, r.prefix)), name(r.iface)));
                        } else if r.src == RouteSrc::Bgp {
                            // BGP: how long the route is as it is (the next hop may lie further away)
                            l.push_str(&format!(", {}", super::rip::hms(self.bgp_route_age(ni, r.prefix))));
                        } else if configured_if(r) {
                            l.push_str(&format!(", {}", name(r.iface)));
                        }
                        l
                    }
                };
                s.push_str(&line);
                s.push('\n');
            }
        };
        // the routes of one prefix together
        fn by_prefix<'a>(rs: &[&'a Route]) -> Vec<Vec<&'a Route>> {
            let mut out: Vec<Vec<&'a Route>> = Vec::new();
            for r in rs {
                match out.last_mut() {
                    Some(g) if g[0].prefix == r.prefix => g.push(r),
                    _ => out.push(alloc::vec![*r]),
                }
            }
            out
        }
        for (addr, len, group) in items {
            match group {
                None => {
                    let rs: Vec<&Route> = routes.iter().copied().filter(|r| r.prefix.addr.to_u32() == addr && r.prefix.len == len).collect();
                    if !rs.is_empty() {
                        entry(&mut s, &rs, 6, true);
                    }
                }
                Some(major) => {
                    let g = &groups[&major];
                    let nets = by_prefix(g);
                    let c = classful(g[0].prefix.addr);
                    if nets.len() == 1 && nets[0][0].prefix.len == c {
                        entry(&mut s, &nets[0], 6, true);
                        continue;
                    }
                    let mut masks: Vec<u8> = nets.iter().map(|n| n[0].prefix.len).collect();
                    masks.sort();
                    masks.dedup();
                    let maj = Ipv4::from_u32(major);
                    if masks.len() == 1 {
                        s.push_str(&format!("      {}/{} is subnetted, {} subnets\n", maj, masks[0], nets.len()));
                    } else {
                        s.push_str(&format!("      {}/{} is variably subnetted, {} subnets, {} masks\n", maj, c, nets.len(), masks.len()));
                    }
                    for n in &nets {
                        entry(&mut s, n, 9, masks.len() > 1);
                    }
                }
            }
        }
        s
    }

    fn show_arp(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::from("Protocol  Address          Age (min)  Hardware Addr   Type   Interface\n");
        let Some(st) = self.stack(ni) else {
            return s;
        };
        let mut rows: Vec<(u32, String)> = Vec::new();
        for (i, f) in st.ifaces.iter().enumerate() {
            if f.hidden || f.is_loopback() || !self.iface_up(ni, i as u16) {
                continue;
            }
            for a in &f.addrs {
                // (Ipv4's Display does not pad: the address goes in as text)
                rows.push((a.addr.to_u32(), format!("Internet  {:<15} {:>9}   {:<14}  ARPA   {}", format!("{}", a.addr), "-", f.mac.cisco(), f.name)));
            }
        }
        for r in self.arp_table(dev) {
            let ifn = st.ifaces.get(r.iface as usize).map(|f| f.name.clone()).unwrap_or_default();
            let age = if r.fixed { String::from("-") } else { format!("{}", r.age / (60 * SEC)) };
            let ip = format!("{}", r.ip);
            let line = match r.mac {
                Some(m) => format!("Internet  {:<15} {:>9}   {:<14}  ARPA   {}", ip, age, m.cisco(), ifn),
                None => format!("Internet  {:<15} {:>9}   {:<14}  ARPA", ip, age, "Incomplete"),
            };
            rows.push((r.ip.to_u32(), line));
        }
        rows.sort_by_key(|r| r.0);
        for (_, l) in rows {
            s.push_str(&l);
            s.push('\n');
        }
        s
    }

    fn show_mac(&self, ni: usize) -> String {
        let mut s = String::from("          Mac Address Table\n-------------------------------------------\n\nVlan    Mac Address       Type        Ports\n----    -----------       --------    -----\n");
        let Some(Some(n)) = self.nodes.get(ni) else {
            return s;
        };
        let Some(b) = n.bridge.as_ref() else {
            return s;
        };
        let rows = b.entries(self.now);
        for (vlan, mac, port, fixed) in &rows {
            s.push_str(&format!("{:>4}    {}    {:<8}    {}\n", vlan, mac.cisco(), if *fixed { "STATIC" } else { "DYNAMIC" }, ifname::short(n.port_name(*port))));
        }
        s.push_str(&format!("Total Mac Addresses for this criterion: {}\n", rows.len()));
        s
    }

    fn show_vlan(&self, ni: usize, brief: bool) -> String {
        let mut s = String::from("\nVLAN Name                             Status    Ports\n---- -------------------------------- --------- -------------------------------\n");
        let Some(Some(n)) = self.nodes.get(ni) else {
            return s;
        };
        let Some(b) = n.bridge.as_ref() else {
            return s;
        };
        for (vid, name) in &b.vlans {
            let ports: Vec<String> = b
                .ports
                .iter()
                .enumerate()
                .filter(|(_, bp)| bp.member && bp.mode == PortMode::Access(*vid))
                .map(|(i, _)| ifname::short(&n.ports[i].spec.name))
                .collect();
            let lines = port_lines(&ports);
            s.push_str(&format!("{:<4} {:<32} {:<9} {}\n", vid, name, "active", lines[0]));
            for l in &lines[1..] {
                s.push_str(&format!("{:48}{}\n", "", l));
            }
        }
        for (vid, name) in [(1002, "fddi-default"), (1003, "token-ring-default"), (1004, "fddinet-default"), (1005, "trnet-default")] {
            s.push_str(&format!("{:<4} {:<32} {:<9} \n", vid, name, "act/unsup"));
        }
        if brief {
            return s;
        }
        s.push_str("\nVLAN Type  SAID       MTU   Parent RingNo BridgeNo Stp  BrdgMode Trans1 Trans2\n---- ----- ---------- ----- ------ ------ -------- ---- -------- ------ ------\n");
        for vid in b.vlans.keys() {
            s.push_str(&format!("{:<4} enet  {:<10} 1500  -      -      -        -    -        0      0   \n", vid, 100_000 + *vid as u32));
        }
        s.push_str("1002 fddi  101002     1500  -      -      -        -    -        0      0   \n1003 tr    101003     1500  -      -      -        -    srb      0      0   \n1004 fdnet 101004     1500  -      -      -        ieee -        0      0   \n1005 trnet 101005     1500  -      -      -        ibm  -        0      0   \n");
        s.push_str("\nRemote SPAN VLANs\n------------------------------------------------------------------------------\n\n\nPrimary Secondary Type              Ports\n------- --------- ----------------- ------------------------------------------\n");
        s
    }

    fn show_version(&self, ni: usize) -> String {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return String::new();
        };
        let up = self.now / SEC;
        let (d, h, m) = (up / 86_400, (up / 3600) % 24, (up / 60) % 60);
        let mut uptime = String::new();
        if d > 0 {
            uptime.push_str(&format!("{} day{}, ", d, if d == 1 { "" } else { "s" }));
        }
        if d > 0 || h > 0 {
            uptime.push_str(&format!("{} hour{}, ", h, if h == 1 { "" } else { "s" }));
        }
        uptime.push_str(&format!("{} minute{}", m, if m == 1 { "" } else { "s" }));
        let mut s = format!("JFNetworker device software 1.0 for the {} ({})\n{} is up for {}\nImage: flash:{}.bin\n\nPorts:\n", n.model.name, n.model.title, n.name, uptime, n.model.id);
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for p in &n.ports {
            let k = match p.spec.media {
                Media::Console | Media::Usb => continue,
                Media::Serial => String::from("Serial"),
                Media::RadioAp | Media::RadioClient => String::from("WLAN radio"),
                _ => String::from(ios_hw(p.spec.kbps)),
            };
            match kinds.iter_mut().find(|x| x.0 == k) {
                Some(x) => x.1 += 1,
                None => kinds.push((k, 1)),
            }
        }
        for (k, c) in kinds {
            s.push_str(&format!("  {:>3}  {}\n", c, k));
        }
        s
    }
}
