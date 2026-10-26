// ios -- the IOS-like command line: the command tree of a device (by what it can: bridge,
// router, access point) and what the commands do; help texts and messages are our own words
//
//   K                        the commands
//   prompt_suffix(mode)      ">", "#", "(config)#", "(config-if)#", ...
//   Engine::ios_line         a typed line: parsed in the console's mode; a configuration
//                            submode falls back to the global configuration (IOS does that,
//                            so "ip route ..." works in interface mode and ends it); an unknown
//                            first word in EXEC is looked up as a host name, as IOS does
//   Engine::ios_help / ios_complete / ios_answer
//
// Modes: User ">", Priv "#", Config "(config)#", If "(config-if)#", SubIf "(config-subif)#",
// Vlan "(config-vlan)#", Line "(config-line)#", DhcpPool "(dhcp-config)#" (the DHCP commands
// are in netsvc.rs), Router "(config-router)#" ("router rip": rip.rs; "router ospf N": ospf.rs,
// a tree mode of its own behind the same prompt, found by its key), AclStd
// "(config-std-nacl)#" and AclExt "(config-ext-nacl)#" (acl.rs; found by their prompt,
// tree_mode).  Interfaces are taken by any IOS spelling
// ("g0/0/0", "Gi 0/0/0", "lo0", "vlan 10"); "interface Loopback5", "interface Vlan20" and
// "interface Gi0/0/0.10" make them.  A subinterface gets its VLAN with "encapsulation dot1Q";
// until then it takes no address (as IOS).  Addresses: "ip address" replaces the primary
// address, "secondary" adds one; masks must fit (no network or broadcast address, no overlap
// with another interface).
// IPv6 (cli/ip6.rs): its commands in every mode, "ping X:X::X" and "traceroute X:X::X" go
// there too, and "Output Interface: " of a link-local ping is one more question of the console.
// Names (cli/names.rs): "ip host", "ipv6 host", "ip name-server"; "ping NAME" and "ping ipv6
// NAME" look the name up.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use base::cmdtree::{Error, ModeId, Names, Parsed, Tree, F_HIDDEN, F_NO};
use jfn_model::catalog::{Media, F_AP, F_BRIDGE, F_ROUTER};
use netpkt::{Ipv4, Mac, Prefix4};

use super::{Ask, AskKind, LineCfg, Method, Mode, D_ARP, D_ICMP, D_RIP};
use crate::action::{ActErr, Action};
use crate::bridge::{SwMode, SwSet, VlanSet};
use crate::driver::Report;
use crate::ip::IfKind;
use crate::kernel::{Engine, SEC};

/// the IOS commands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K {
    Enable,
    Disable,
    Exit,
    ConfT,
    Ping,
    Trace,
    CopyRunStart,
    WriteMem,
    ClearArp,
    Debug,
    Undebug,
    TermLen,
    ShowRun,
    ShowStart,
    ShowIpBrief,
    ShowIpRoute,
    ShowArp,
    ShowMac,
    ShowVlan,
    ShowIf,
    ShowIfTrunk,
    ShowDebug,
    ShowVersion,
    End,
    Do,
    Hostname,
    Interface,
    VlanCfg,
    IpRoute,
    IpRouting,
    DomainLookup,
    ArpStatic,
    LineCfg,
    Version,
    IpAddress,
    Shutdown,
    Switchport,
    SwMode,
    SwAccess,
    SwNative,
    SwAllowed,
    SwEncap,
    Encap,
    ProxyArp,
    IpMtu,
    Descr,
    VlanName,
    LinePassword,
    LineLogin,
    LineLogging,
    LineTimeout,
    ShowTcpBrief,
    ShowTcpStats,
    ShowOpenPorts,
    // DHCP (cli/netsvc.rs)
    DhcpPool,
    DhcpExcluded,
    PoolNetwork,
    PoolRouter,
    PoolDns,
    PoolDomain,
    PoolLease,
    Helper,
    ShowDhcpBinding,
    ShowDhcpPool,
    ShowDhcpConflict,
    ClearDhcp,
    // NTP (cli/netsvc.rs)
    NtpServer,
    NtpMaster,
    // spanning tree (cli/stp.rs)
    ShowStp,
    DebugStp,
    ClearStp,
    StpMode,
    StpPriority,
    StpTimer,
    StpIf,
    // RIP (cli/rip.rs)
    RouterRip,
    RipVersion,
    RipNetwork,
    RipPassive,
    RipAutoSummary,
    RipDefault,
    RipRedist,
    RipTimers,
    RipDistance,
    RipMaxPaths,
    RipIfVersion,
    ShowRipDb,
    ShowProtocols,
    ClearRoute,
    // access lists (cli/acl.rs)
    AclStd,
    AclExt,
    AclRemark,
    AclNamed,
    AclSeqNo,
    AclReseq,
    AccessGroup,
    ShowAcl,
    ClearAcl,
    // NAT (cli/nat.rs)
    NatSide,
    NatPool,
    NatList,
    NatStatic,
    NatTimeout,
    ShowNatTrans,
    ShowNatStats,
    ClearNat,
    // IPv6 (cli/ip6.rs)
    Ipv6Global,
    Ipv6Route,
    Ipv6Neighbor,
    Ipv6If,
    ShowIpv6,
    Ping6,
    Trace6,
    Debug6,
    ClearIpv6,
    // OSPF (cli/ospf.rs)
    RouterOspf,
    OspfRouterId,
    OspfNetwork,
    OspfPassive,
    OspfAutoCost,
    OspfDefault,
    OspfRedist,
    OspfStub,
    OspfDistance,
    OspfMaxPaths,
    OspfLogAdj,
    OspfIf,
    ShowOspf,
    ShowOspfNbr,
    ShowOspfIf,
    ShowOspfDb,
    DebugOspf,
    ClearOspf,
    IpHost,
    Ipv6Host,
    NameServer,
    // WLAN on access points (cli/wlan.rs)
    ShowDot11,
}

// the modes of every tree, in the order build() makes them
const USER: ModeId = 0;
const PRIV: ModeId = 1;
const CONF: ModeId = 2;
const IFM: ModeId = 3;
const SUBIF: ModeId = 4;
const VLAN: ModeId = 5;
const LINE: ModeId = 6;
const POOL: ModeId = 7;
const ROUTER: ModeId = 8;

const CAP_BRIDGE: u32 = 1;
const CAP_ROUTER: u32 = 2;
const CAP_AP: u32 = 4;

fn mode_id(m: Mode) -> ModeId {
    match m {
        Mode::User | Mode::Out => USER,
        Mode::Priv => PRIV,
        Mode::Config => CONF,
        Mode::If => IFM,
        Mode::SubIf => SUBIF,
        Mode::Vlan => VLAN,
        Mode::Line => LINE,
        Mode::DhcpPool => POOL,
        Mode::Router(_) => ROUTER,
        // modes of later parts are found by their prompt (tree_mode)
        Mode::AclStd | Mode::AclExt => CONF,
    }
}

/// the mode of the tree for a console mode: the fixed ones by number, those that cli/acl.rs
/// (and later parts) add by their prompt, so their numbers may differ from tree to tree
fn tree_mode(t: &Tree<K>, m: Mode) -> ModeId {
    match m {
        Mode::AclStd | Mode::AclExt => t.find_mode(prompt_suffix(m)).unwrap_or(CONF),
        // OSPF's router mode shares the prompt with RIP's: the tree knows it by its key
        Mode::Router(super::Proc::Ospf(_)) => t.find_mode(super::ospf::MODE_KEY).unwrap_or(CONF),
        _ => mode_id(m),
    }
}

/// what follows the host name in the prompt of a mode
pub fn prompt_suffix(m: Mode) -> &'static str {
    match m {
        Mode::User => ">",
        Mode::Priv => "#",
        Mode::Config => "(config)#",
        Mode::If => "(config-if)#",
        Mode::SubIf => "(config-subif)#",
        Mode::Vlan => "(config-vlan)#",
        Mode::Line => "(config-line)#",
        Mode::Out => "",
        Mode::DhcpPool => "(dhcp-config)#",
        Mode::Router(_) => "(config-router)#",
        Mode::AclStd => "(config-std-nacl)#",
        Mode::AclExt => "(config-ext-nacl)#",
    }
}

const NEGATE: &str = "Undo a setting or restore its default";

/// the command tree of a device with these capabilities
fn build(caps: u32) -> Tree<K> {
    let bridge = caps & CAP_BRIDGE != 0;
    let router = caps & CAP_ROUTER != 0;
    let l3 = bridge && router;
    let mut t: Tree<K> = Tree::new();
    let user = t.mode(">", "Commands at the user level:", None, None);
    let prv = t.mode("#", "Commands at the privileged level:", None, Some("Switch debugging off"));
    let conf = t.mode("(config)#", "Commands for the whole device:", Some(prv), Some(NEGATE));
    let ifm = t.mode("(config-if)#", "Commands for this interface:", Some(conf), Some(NEGATE));
    let sub = t.mode("(config-subif)#", "Commands for this subinterface:", Some(conf), Some(NEGATE));
    let vlan = t.mode("(config-vlan)#", "Commands for this VLAN:", Some(conf), Some(NEGATE));
    let line = t.mode("(config-line)#", "Commands for this line:", Some(conf), Some(NEGATE));
    let pool = t.mode("(dhcp-config)#", "Commands for this address pool:", Some(conf), Some(NEGATE));
    let rtr = t.mode("(config-router)#", "Commands for this routing process:", Some(conf), Some(NEGATE));
    debug_assert!(user == USER && prv == PRIV && conf == CONF && ifm == IFM && sub == SUBIF && vlan == VLAN && line == LINE && pool == POOL && rtr == ROUTER);

    // ---- EXEC (user and privileged)
    for m in [user, prv] {
        let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>| {
            t.add(m, p, h, c, 0);
        };
        a(&mut t, "enable", "Switch to the privileged level", Some(K::Enable));
        a(&mut t, "exit", "Close this session", Some(K::Exit));
        a(&mut t, "logout", "Close this session", Some(K::Exit));
        a(&mut t, "ping", "Check whether a host answers", None);
        a(&mut t, "ping ip", "Ping over IPv4", None);
        a(&mut t, "traceroute", "List the routers on the way to a host", None);
        a(&mut t, "traceroute ip", "Traceroute over IPv4", None);
        for pre in ["ping", "ping ip"] {
            let tgt = format!("{} WORD=target", pre);
            a(&mut t, &tgt, "Address or name of the host", Some(K::Ping));
            let opts: [(&str, &str, Option<&str>); 7] = [
                ("repeat", "How many echo requests to send", Some("<1-2147483647>=count|Number of echo requests")),
                ("size", "Size of each packet", Some("<36-18024>=bytes|Packet size in bytes")),
                ("timeout", "How long to wait for each answer", Some("<0-3600>=secs|Seconds to wait")),
                ("source", "Send from this address or interface", Some("A.B.C.D=src|Address to send from")),
                ("source", "Send from this address or interface", Some("NAME1=srcif|")),
                ("df-bit", "Forbid fragmentation (set the DF bit)", None),
                ("validate", "Compare the answers with the data sent", None),
            ];
            for (kw, h, val) in opts {
                let kp = format!("{} {}", tgt, kw);
                match val {
                    Some(v) => {
                        let (pp, ph) = v.split_once('|').unwrap_or((v, ""));
                        a(&mut t, &kp, h, None);
                        let full = format!("{} {}", kp, pp);
                        a(&mut t, &full, ph, Some(K::Ping));
                        t.also(m, &full, &tgt);
                    }
                    None => {
                        a(&mut t, &kp, h, Some(K::Ping));
                        t.also(m, &kp, &tgt);
                    }
                }
            }
        }
        for pre in ["traceroute", "traceroute ip"] {
            let tgt = format!("{} WORD=target", pre);
            a(&mut t, &tgt, "Address or name of the destination", Some(K::Trace));
            a(&mut t, &format!("{} numeric", tgt), "Show addresses only, no names", Some(K::Trace));
            a(&mut t, &format!("{} probe", tgt), "Probes per hop", None);
            a(&mut t, &format!("{} probe <1-65535>=probes", tgt), "How many probes", Some(K::Trace));
            a(&mut t, &format!("{} timeout", tgt), "How long to wait for each probe", None);
            a(&mut t, &format!("{} timeout <0-3600>=secs", tgt), "Seconds to wait", Some(K::Trace));
            a(&mut t, &format!("{} ttl", tgt), "First and last hop to probe", None);
            a(&mut t, &format!("{} ttl <0-255>=min", tgt), "First hop (TTL)", None);
            a(&mut t, &format!("{} ttl <0-255>=min <0-255>=max", tgt), "Last hop (TTL)", Some(K::Trace));
            a(&mut t, &format!("{} source", tgt), "Send from this address or interface", None);
            a(&mut t, &format!("{} source A.B.C.D=src", tgt), "Address to send from", Some(K::Trace));
            a(&mut t, &format!("{} source NAME1=srcif", tgt), "", Some(K::Trace));
            for end in ["numeric", "probe <1-65535>=probes", "timeout <0-3600>=secs", "ttl <0-255>=min <0-255>=max", "source A.B.C.D=src", "source NAME1=srcif"] {
                t.also(m, &format!("{} {}", tgt, end), &tgt);
            }
        }
        a(&mut t, "terminal", "Settings of this session's screen", None);
        a(&mut t, "terminal length", "Lines per page", None);
        a(&mut t, "terminal length <0-512>=len", "Lines before --More-- (0: no paging)", Some(K::TermLen));
        a(&mut t, "show", "Display the state of the device", None);
        a(&mut t, "show ip", "IPv4 state", None);
        a(&mut t, "show ip interface", "IPv4 settings of the interfaces", None);
        a(&mut t, "show ip interface brief", "One line per interface: address and state", Some(K::ShowIpBrief));
        a(&mut t, "show ip route", "Routes this device knows", Some(K::ShowIpRoute));
        a(&mut t, "show ip arp", "Learned IPv4-to-MAC mappings", Some(K::ShowArp));
        a(&mut t, "show arp", "Learned IPv4-to-MAC mappings", Some(K::ShowArp));
        a(&mut t, "show interfaces", "Details and counters of the interfaces", Some(K::ShowIf));
        a(&mut t, "show interfaces NAME1=if", "", Some(K::ShowIf));
        a(&mut t, "show version", "Software, uptime and ports", Some(K::ShowVersion));
        if bridge {
            a(&mut t, "show mac", "MAC address information", None);
            a(&mut t, "show mac address-table", "Which MAC address the switch saw on which port", Some(K::ShowMac));
            a(&mut t, "show vlan", "VLANs and their ports", Some(K::ShowVlan));
            a(&mut t, "show vlan brief", "VLANs and their ports, one line each", Some(K::ShowVlan));
            a(&mut t, "show interfaces trunk", "Trunk ports and their VLANs", Some(K::ShowIfTrunk));
        }
        // the device's own TCP (cli/tools.rs)
        a(&mut t, "show tcp", "TCP connections of the device itself", None);
        a(&mut t, "show tcp brief", "One line per connection", Some(K::ShowTcpBrief));
        a(&mut t, "show tcp brief all", "Listening sockets as well", Some(K::ShowTcpBrief));
        a(&mut t, "show tcp statistics", "What the device's TCP counted", Some(K::ShowTcpStats));
        a(&mut t, "show control-plane", "What the device itself serves", None);
        a(&mut t, "show control-plane host", "The device as an end point", None);
        a(&mut t, "show control-plane host open-ports", "Its listening ports and connections", Some(K::ShowOpenPorts));
    }
    let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>, f: u8| {
        t.add(prv, p, h, c, f);
    };
    a(&mut t, "disable", "Return to the user level", Some(K::Disable), 0);
    a(&mut t, "configure", "Start changing the configuration", Some(K::ConfT), 0);
    a(&mut t, "configure terminal", "Type configuration commands here", Some(K::ConfT), 0);
    a(&mut t, "copy", "Copy a configuration", None, 0);
    a(&mut t, "copy running-config", "The configuration in effect now", None, 0);
    a(&mut t, "copy running-config startup-config", "Keep it for the next start", Some(K::CopyRunStart), 0);
    a(&mut t, "write", "Save the configuration", Some(K::WriteMem), 0);
    a(&mut t, "write memory", "Keep it for the next start", Some(K::WriteMem), 0);
    a(&mut t, "clear", "Empty a table", None, 0);
    a(&mut t, "clear arp-cache", "Forget all learned ARP entries", Some(K::ClearArp), 0);
    a(&mut t, "debug", "Show what the device does while it happens", None, F_NO);
    a(&mut t, "debug all", "Everything (much output)", Some(K::Debug), 0);
    a(&mut t, "debug ip", "IPv4 events", None, 0);
    a(&mut t, "debug ip icmp", "ICMP messages sent and received", Some(K::Debug), 0);
    a(&mut t, "debug arp", "ARP requests and replies", Some(K::Debug), 0);
    a(&mut t, "undebug", "Stop debug output", None, 0);
    a(&mut t, "undebug all", "Stop all debug output", Some(K::Undebug), 0);
    a(&mut t, "undebug ip", "IPv4 events", None, 0);
    a(&mut t, "undebug ip icmp", "ICMP messages sent and received", Some(K::Undebug), 0);
    a(&mut t, "undebug arp", "ARP requests and replies", Some(K::Undebug), 0);
    a(&mut t, "show running-config", "Configuration in effect now", Some(K::ShowRun), 0);
    a(&mut t, "show startup-config", "Configuration used at the next start", Some(K::ShowStart), 0);
    a(&mut t, "show debugging", "Which debug output is on", Some(K::ShowDebug), 0);

    // ---- every configuration mode
    for (m, exit) in [(conf, "Leave the configuration"), (ifm, "Back to the device configuration"), (sub, "Back to the device configuration"), (vlan, "Back to the device configuration"), (line, "Back to the device configuration")] {
        t.add(m, "do", "Run an EXEC command from here", None, 0);
        t.add(m, "do LINE=cmd", "The command", Some(K::Do), 0);
        t.add(m, "exit", exit, Some(K::Exit), 0);
        t.add(m, "end", "Finish configuring", Some(K::End), if m == conf { 0 } else { F_HIDDEN });
    }

    // ---- global configuration
    let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>, f: u8| {
        t.add(conf, p, h, c, f);
    };
    a(&mut t, "hostname", "Give the device a name", None, F_NO);
    a(&mut t, "hostname WORD=name", "The new name", Some(K::Hostname), 0);
    t.add_no(conf, "hostname", K::Hostname);
    a(&mut t, "interface", "Pick an interface to set up", None, F_NO);
    a(&mut t, "interface NAME=if", "", Some(K::Interface), 0);
    a(&mut t, "ip", "IPv4 settings of the device", None, F_NO);
    a(&mut t, "ip route", "Add a static route", None, 0);
    a(&mut t, "ip route A.B.C.D=net", "Destination network", None, 0);
    a(&mut t, "ip route A.B.C.D=net MASK=mask", "Mask of the destination network", None, 0);
    t.add_no(conf, "ip route A.B.C.D=net MASK=mask", K::IpRoute);
    for via in ["A.B.C.D=gw", "NAME1=out", "NAME1=out A.B.C.D=gw"] {
        let p = format!("ip route A.B.C.D=net MASK=mask {}", via);
        a(&mut t, &p, if via.ends_with("gw") { "Next router on the way" } else { "" }, Some(K::IpRoute), 0);
        a(&mut t, &format!("{} <1-255>=ad", p), "Preference of this route, lower wins", Some(K::IpRoute), 0);
    }
    if router {
        a(&mut t, "ip routing", "Forward IPv4 packets between interfaces", Some(K::IpRouting), 0);
    }
    a(&mut t, "ip domain-lookup", "Look up unknown words as host names", Some(K::DomainLookup), 0);
    a(&mut t, "ip domain", "Name lookup", None, 0);
    a(&mut t, "ip domain lookup", "Look up unknown words as host names", Some(K::DomainLookup), 0);
    a(&mut t, "arp", "Add a fixed ARP entry", None, F_NO);
    a(&mut t, "arp A.B.C.D=ip", "IPv4 address", None, 0);
    a(&mut t, "arp A.B.C.D=ip H.H.H=mac", "MAC address (H.H.H)", None, 0);
    a(&mut t, "arp A.B.C.D=ip H.H.H=mac arpa", "Ethernet frames", Some(K::ArpStatic), 0);
    t.add_no(conf, "arp A.B.C.D=ip", K::ArpStatic);
    if bridge {
        a(&mut t, "vlan", "Create or change a VLAN", None, F_NO);
        a(&mut t, "vlan <1-4094>=vid", "VLAN number", Some(K::VlanCfg), 0);
    }
    a(&mut t, "line", "Settings of the console or the remote lines", None, 0);
    a(&mut t, "line console", "The console port", None, 0);
    a(&mut t, "line console <0-0>=first", "First line", Some(K::LineCfg), 0);
    a(&mut t, "line vty", "Lines for remote logins", None, 0);
    a(&mut t, "line vty <0-15>=first", "First line", Some(K::LineCfg), 0);
    a(&mut t, "line vty <0-15>=first <1-15>=last", "Last line", Some(K::LineCfg), 0);
    a(&mut t, "version", "Format version of the configuration", None, F_HIDDEN);
    a(&mut t, "version WORD=v", "The version", Some(K::Version), 0);

    // ---- interface and subinterface configuration
    for m in [ifm, sub] {
        let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>, f: u8| {
            t.add(m, p, h, c, f);
        };
        a(&mut t, "description", "A note about this interface", None, F_NO);
        a(&mut t, "description LINE=text", "The note, up to 240 characters", Some(K::Descr), 0);
        t.add_no(m, "description", K::Descr);
        a(&mut t, "ip", "IPv4 settings of this interface", None, F_NO);
        a(&mut t, "ip address", "Give the interface an address", None, 0);
        a(&mut t, "ip address A.B.C.D=addr", "The address", None, 0);
        a(&mut t, "ip address A.B.C.D=addr MASK=mask", "The network mask", Some(K::IpAddress), 0);
        a(&mut t, "ip address A.B.C.D=addr MASK=mask secondary", "An extra address next to the first one", Some(K::IpAddress), 0);
        t.add_no(m, "ip address", K::IpAddress);
        a(&mut t, "ip mtu", "Largest IPv4 packet on this interface", None, 0);
        a(&mut t, "ip mtu <68-1500>=mtu", "Bytes", Some(K::IpMtu), 0);
        t.add_no(m, "ip mtu", K::IpMtu);
        a(&mut t, "ip proxy-arp", "Answer ARP for hosts behind this router", Some(K::ProxyArp), 0);
        a(&mut t, "shutdown", "Switch the interface off", Some(K::Shutdown), F_NO);
    }
    t.add(sub, "encapsulation", "Frame format of this subinterface", None, 0);
    t.add(sub, "encapsulation dot1Q", "VLAN tags (802.1Q)", None, 0);
    t.add(sub, "encapsulation dot1Q <1-4094>=vid", "VLAN number", Some(K::Encap), 0);
    if bridge {
        let a = |t: &mut Tree<K>, p: &str, h: &str, c: Option<K>, f: u8| {
            t.add(ifm, p, h, c, f);
        };
        a(&mut t, "switchport", "Settings of a switch port", if l3 { Some(K::Switchport) } else { None }, F_NO);
        a(&mut t, "switchport mode", "Access port or trunk", None, 0);
        a(&mut t, "switchport mode access=mode", "One VLAN, untagged", Some(K::SwMode), 0);
        a(&mut t, "switchport mode trunk=mode", "Several VLANs, tagged", Some(K::SwMode), 0);
        a(&mut t, "switchport mode dynamic=mode", "Agree with the neighbour on access or trunk", None, 0);
        a(&mut t, "switchport mode dynamic=mode auto=dyn", "Trunk if the neighbour asks for it", Some(K::SwMode), 0);
        a(&mut t, "switchport mode dynamic=mode desirable=dyn", "Ask the neighbour for a trunk", Some(K::SwMode), 0);
        t.add_no(ifm, "switchport mode", K::SwMode);
        a(&mut t, "switchport access", "Settings as an access port", None, 0);
        a(&mut t, "switchport access vlan", "The VLAN of this port", None, 0);
        a(&mut t, "switchport access vlan <1-4094>=vid", "VLAN number", Some(K::SwAccess), 0);
        t.add_no(ifm, "switchport access vlan", K::SwAccess);
        a(&mut t, "switchport trunk", "Settings as a trunk", None, 0);
        a(&mut t, "switchport trunk native", "The untagged VLAN of the trunk", None, 0);
        a(&mut t, "switchport trunk native vlan", "The untagged VLAN of the trunk", None, 0);
        a(&mut t, "switchport trunk native vlan <1-4094>=vid", "VLAN number", Some(K::SwNative), 0);
        t.add_no(ifm, "switchport trunk native vlan", K::SwNative);
        a(&mut t, "switchport trunk allowed", "VLANs the trunk carries", None, 0);
        a(&mut t, "switchport trunk allowed vlan", "VLANs the trunk carries", None, 0);
        a(&mut t, "switchport trunk allowed vlan WORD=list", "VLAN list such as 10,20-30", Some(K::SwAllowed), 0);
        a(&mut t, "switchport trunk allowed vlan add=op", "Carry these VLANs too", None, 0);
        a(&mut t, "switchport trunk allowed vlan add=op WORD=list", "VLAN list such as 10,20-30", Some(K::SwAllowed), 0);
        a(&mut t, "switchport trunk allowed vlan all=op", "Carry every VLAN", Some(K::SwAllowed), 0);
        a(&mut t, "switchport trunk allowed vlan except=op", "Carry all VLANs but these", None, 0);
        a(&mut t, "switchport trunk allowed vlan except=op WORD=list", "VLAN list such as 10,20-30", Some(K::SwAllowed), 0);
        a(&mut t, "switchport trunk allowed vlan none=op", "Carry no VLAN", Some(K::SwAllowed), 0);
        a(&mut t, "switchport trunk allowed vlan remove=op", "Stop carrying these VLANs", None, 0);
        a(&mut t, "switchport trunk allowed vlan remove=op WORD=list", "VLAN list such as 10,20-30", Some(K::SwAllowed), 0);
        t.add_no(ifm, "switchport trunk allowed vlan", K::SwAllowed);
        a(&mut t, "switchport trunk encapsulation", "Tag format of the trunk", None, 0);
        a(&mut t, "switchport trunk encapsulation dot1q", "802.1Q tags", Some(K::SwEncap), 0);
    }

    // ---- VLAN configuration
    t.add(vlan, "name", "Give the VLAN a name", None, F_NO);
    t.add(vlan, "name WORD=name", "The name, up to 32 characters", Some(K::VlanName), 0);
    t.add_no(vlan, "name", K::VlanName);

    // ---- line configuration
    t.add(line, "password", "Password of this line", None, F_NO);
    t.add(line, "password LINE=pw", "The password as plain text", Some(K::LinePassword), 0);
    t.add_no(line, "password", K::LinePassword);
    t.add(line, "login", "Ask for the password at login", Some(K::LineLogin), F_NO);
    t.add(line, "logging", "How log messages appear", None, F_NO);
    t.add(line, "logging synchronous", "Retype the line after a log message", Some(K::LineLogging), 0);
    t.add(line, "exec-timeout", "Log out after a time without input", None, F_NO);
    t.add(line, "exec-timeout <0-35791>=min", "Minutes", Some(K::LineTimeout), 0);
    t.add(line, "exec-timeout <0-35791>=min <0-2147483>=sec", "Seconds", Some(K::LineTimeout), 0);
    t.add_no(line, "exec-timeout", K::LineTimeout);

    // ---- DHCP server and relay, NTP (cli/netsvc.rs)
    super::netsvc::build_netsvc(&mut t, [user, prv, conf, ifm, sub, pool]);
    // ---- spanning tree on switches (cli/stp.rs)
    if bridge && caps & CAP_AP == 0 {
        super::stp::build_stp(&mut t, [user, prv, conf, ifm]);
    }
    // ---- RIP (cli/rip.rs)
    super::rip::build_rip(&mut t, [user, prv, conf, ifm, sub, rtr], router);
    // ---- access lists with their two modes, NAT on routing devices (cli/acl.rs, cli/nat.rs)
    super::acl::build_acl(&mut t, [user, prv, conf, ifm, sub]);
    if router {
        super::nat::build_nat(&mut t, [user, prv, conf, ifm, sub]);
    }
    // ---- IPv6 (cli/ip6.rs)
    super::ip6::build_ip6(&mut t, [user, prv, conf, ifm, sub], router);
    // ---- OSPF with its own router mode (cli/ospf.rs)
    super::ospf::build_ospf(&mut t, [user, prv, conf, ifm, sub], router);
    // ---- static names and name servers of either family (cli/names.rs)
    super::names::build_names(&mut t, conf);
    // ---- WLAN on access points (cli/wlan.rs)
    if caps & CAP_AP != 0 {
        super::wlan::build_wlan(&mut t, [user, prv]);
    }
    t
}

/// the help text of an interface type ("interface ?")
fn type_help(ty: &str) -> String {
    String::from(match ty {
        "GigabitEthernet" => "Gigabit Ethernet port (1 Gbit/s)",
        "TwoGigabitEthernet" => "Multi-gigabit Ethernet port (2.5 Gbit/s)",
        "TenGigabitEthernet" => "10 Gigabit Ethernet port",
        "TwentyFiveGigE" => "25 Gigabit Ethernet port",
        "HundredGigE" => "100 Gigabit Ethernet port",
        "Serial" => "Serial WAN port",
        "Loopback" => "Virtual interface that is always up",
        "Vlan" => "Routed interface of a VLAN (SVI)",
        "BVI" => "Routed interface of the bridge",
        "Dot11Radio" => "WLAN radio",
        _ => return format!("{} interface", ty),
    })
}

/// the interface names of a device for the command tree
pub(crate) struct IosNames<'a> {
    pub e: &'a Engine,
    pub ni: usize,
}

impl Names for IosNames<'_> {
    /// list 0 ("interface ..."): every port and interface and what may be made; list 1: what
    /// exists
    fn names(&self, list: u8) -> Vec<String> {
        let Some(Some(n)) = self.e.nodes.get(self.ni) else {
            return Vec::new();
        };
        let mut v: Vec<String> = Vec::new();
        for p in &n.ports {
            if !matches!(p.spec.media, Media::Console | Media::Usb) {
                v.push(p.spec.name.clone());
            }
        }
        if let Some(st) = &n.ip {
            for f in &st.ifaces {
                if !f.hidden && !v.contains(&f.name) {
                    v.push(f.name.clone());
                }
            }
            if list == 0 {
                v.push(String::from("Loopback*"));
                if n.bridge.is_some() && !n.model.has(F_AP) {
                    v.push(String::from("Vlan*"));
                }
                for (i, p) in n.ports.iter().enumerate() {
                    let routed = st.iface_of_port(i as u16).is_some_and(|f| !st.ifaces[f as usize].hidden);
                    if routed && p.spec.media.is_ethernet() && !p.spec.media.is_radio() {
                        v.push(format!("{}.*", p.spec.name));
                    }
                }
            }
        }
        if list == 0 {
            for p in &n.cli.cfg.pending {
                if !v.contains(p) {
                    v.push(p.clone());
                }
            }
        }
        v
    }
    fn type_help(&self, _list: u8, ty: &str) -> String {
        type_help(ty)
    }
}

fn caps_of(e: &Engine, ni: usize) -> u32 {
    match e.nodes.get(ni) {
        Some(Some(n)) => {
            let m = n.model;
            (if m.has(F_BRIDGE) { CAP_BRIDGE } else { 0 }) | (if m.has(F_ROUTER) { CAP_ROUTER } else { 0 }) | (if m.has(F_AP) { CAP_AP } else { 0 })
        }
        _ => 0,
    }
}

/// the first line of a console text (what boot_errors keep)
fn first_line(s: &str) -> String {
    String::from(s.lines().find(|l| !l.trim().is_empty() && l.trim() != "^").unwrap_or("").trim())
}

impl Engine {
    /// the command tree of node `ni` (built once per kind of device)
    fn ios_tree(&mut self, ni: usize) -> Arc<Tree<K>> {
        let c = caps_of(self, ni);
        if let Some((_, t)) = self.cli.trees.iter().find(|(k, _)| *k == c) {
            return t.clone();
        }
        let t = Arc::new(build(c));
        self.cli.trees.push((c, t.clone()));
        t
    }
    fn ios_tree_ro(&self, ni: usize) -> Arc<Tree<K>> {
        let c = caps_of(self, ni);
        match self.cli.trees.iter().find(|(k, _)| *k == c) {
            Some((_, t)) => t.clone(),
            None => Arc::new(build(c)),
        }
    }

    fn ios_mode(&self, ni: usize) -> Mode {
        self.cdev(ni).map(|c| c.mode).unwrap_or(Mode::User)
    }
    fn set_mode(&mut self, ni: usize, m: Mode) {
        if let Some(c) = self.cdev_mut(ni) {
            c.mode = m;
            if !matches!(m, Mode::If | Mode::SubIf) {
                c.ifs.clear();
            }
            if m != Mode::Vlan {
                c.vlans.clear();
            }
            if m != Mode::DhcpPool {
                c.pool.clear();
            }
        }
    }
    fn booting(&self, ni: usize) -> bool {
        self.cdev(ni).is_some_and(|c| c.booting)
    }
    /// an error message on the console; Err with its first line
    pub(crate) fn ios_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.out(ni, text);
        Err(first_line(text))
    }

    /// the text "?" shows after `line`
    pub(crate) fn ios_help(&self, ni: usize, line: &str) -> String {
        let tree = self.ios_tree_ro(ni);
        let names = IosNames { e: self, ni };
        let mode = self.ios_mode(ni);
        let col = self.prompt_of(ni).chars().count();
        // "do ..." in configuration modes is help of the privileged EXEC
        let t = line.trim_start();
        if !matches!(mode, Mode::User | Mode::Priv | Mode::Out)
            && let Some(rest) = t.strip_prefix("do ")
        {
            let off = line.len() - rest.len();
            return tree.help(PRIV, rest, &names).render(col + off, rest);
        }
        tree.help(tree_mode(&tree, mode), line, &names).render(col, line)
    }

    /// Tab
    pub(crate) fn ios_complete(&self, ni: usize, line: &str) -> (Option<String>, Vec<String>) {
        let tree = self.ios_tree_ro(ni);
        let names = IosNames { e: self, ni };
        let c = tree.complete(tree_mode(&tree, self.ios_mode(ni)), line, &names);
        (c.line, c.choices)
    }

    /// a line typed at an IOS console
    pub(crate) fn ios_line(&mut self, ni: usize, line: &str) -> Result<(), String> {
        let mode = self.ios_mode(ni);
        if mode == Mode::Out {
            self.set_mode(ni, Mode::User);
            return Ok(());
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('!') {
            return Ok(());
        }
        let tree = self.ios_tree(ni);
        let parsed = tree.parse(tree_mode(&tree, mode), line, &IosNames { e: self, ni });
        let r = match parsed {
            Ok(p) => self.ios_do(ni, mode, &p, line),
            Err(err) => {
                // a configuration submode falls back to the global configuration
                let up = if matches!(mode, Mode::If | Mode::SubIf | Mode::Vlan | Mode::Line | Mode::DhcpPool | Mode::Router(_) | Mode::AclStd | Mode::AclExt) { tree.parse(CONF, line, &IosNames { e: self, ni }).ok() } else { None };
                match up {
                    Some(p) => {
                        self.set_mode(ni, Mode::Config);
                        self.ios_do(ni, Mode::Config, &p, line)
                    }
                    None => self.ios_parse_error(ni, mode, &tree, err, line),
                }
            }
        };
        self.ios_snap(ni);
        r
    }

    /// "% Invalid input detected at '^' marker." under byte `at` of the line
    pub(crate) fn ios_marker(&mut self, ni: usize, line: &str, at: usize) -> Result<(), String> {
        self.ios_error(ni, line, Error::Invalid(at.min(line.len())))
    }

    /// an error of the parser on the console: under the prompt, or at boot under the line itself
    fn ios_error(&mut self, ni: usize, line: &str, err: Error) -> Result<(), String> {
        let booting = self.booting(ni);
        if booting {
            self.out(ni, &format!("{}\n", line));
        }
        let col = if booting { 0 } else { self.prompt_of(ni).chars().count() };
        let text = err.render(col, line);
        self.ios_err(ni, &text)
    }

    fn ios_parse_error(&mut self, ni: usize, mode: Mode, tree: &Tree<K>, err: Error, line: &str) -> Result<(), String> {
        if matches!(mode, Mode::User | Mode::Priv) && !tree.knows_first(tree_mode(tree, mode), line) {
            // IOS takes an unknown word in EXEC for a host to connect to
            let word = line.split_whitespace().next().unwrap_or("");
            let lookup = self.cdev(ni).is_some_and(|c| c.cfg.domain_lookup);
            let text = if lookup {
                format!("Resolving \"{}\" via DNS (255.255.255.255) ...\n% Unknown command or host name\n", word)
            } else {
                String::from("% Unknown command or host name\n")
            };
            return self.ios_err(ni, &text);
        }
        self.ios_error(ni, line, err)
    }

    /// runs a parsed command
    fn ios_do(&mut self, ni: usize, mode: Mode, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match p.cmd {
            K::Enable => {
                self.set_mode(ni, Mode::Priv);
                Ok(())
            }
            K::Disable => {
                self.set_mode(ni, Mode::User);
                Ok(())
            }
            K::Exit => match mode {
                Mode::User | Mode::Priv => {
                    let host = self.host(ni);
                    self.out(ni, &format!("\n{}: session closed.\n\nConsole ready, press Enter to log in.\n\n", host));
                    self.set_mode(ni, Mode::Out);
                    Ok(())
                }
                Mode::Config => {
                    self.set_mode(ni, Mode::Priv);
                    self.ios_log(ni, "%SYS-5-CONFIG_I: configuration changed at the console");
                    Ok(())
                }
                _ => {
                    self.set_mode(ni, Mode::Config);
                    Ok(())
                }
            },
            K::End => {
                self.set_mode(ni, Mode::Priv);
                if !self.booting(ni) {
                    self.ios_log(ni, "%SYS-5-CONFIG_I: configuration changed at the console");
                }
                Ok(())
            }
            K::ConfT => {
                self.out(ni, "Configuration mode: one command per line; end or Ctrl+Z leaves it.\n");
                self.set_mode(ni, Mode::Config);
                Ok(())
            }
            K::Do => {
                let cmd = p.text("cmd").unwrap_or("");
                let tree = self.ios_tree(ni);
                match tree.parse(PRIV, cmd, &IosNames { e: self, ni }) {
                    Ok(q) => self.ios_do(ni, Mode::Priv, &q, cmd),
                    Err(e) => {
                        let off = line.len() - line.trim_start().len() + line.trim_start().find(cmd).unwrap_or(0);
                        let col = self.prompt_of(ni).chars().count() + off;
                        let text = e.render(col, cmd);
                        self.ios_err(ni, &text)
                    }
                }
            }
            K::Ping => self.ios_ping(ni, p),
            K::Trace => self.ios_traceroute(ni, p),
            K::CopyRunStart => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.ask = Some(Ask { prompt: String::from("Save to which file [startup-config]? "), kind: AskKind::CopyRunStart });
                }
                Ok(())
            }
            K::WriteMem => {
                self.ios_save(ni);
                Ok(())
            }
            K::ClearArp => {
                let _ = self.apply(Action::ClearArp { dev });
                Ok(())
            }
            K::Debug | K::Undebug => {
                let off = p.cmd == K::Undebug || p.no;
                let bits = if p.has("all") {
                    D_ICMP | D_ARP | super::D_STP | D_RIP | super::D_NAT | super::D_ICMP6 | super::D_ND | super::D_OSPF
                } else if p.has("icmp") {
                    D_ICMP
                } else if p.has("rip") {
                    D_RIP
                } else if p.has("nat") {
                    super::D_NAT
                } else {
                    D_ARP
                };
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
                let text = match (off, p.has("all"), bits) {
                    (true, true, _) => String::from("Debug output: all off\n"),
                    (false, true, _) => String::from("Debug output: all on (a busy network prints a lot)\n"),
                    (o, _, D_ICMP) => format!("Debug output for ICMP: {}\n", if o { "off" } else { "on" }),
                    (o, _, D_RIP) => format!("Debug output for RIP: {}\n", if o { "off" } else { "on" }),
                    (o, _, super::D_NAT) => format!("Debug output for NAT: {}\n", if o { "off" } else { "on" }),
                    (o, _, _) => format!("Debug output for ARP: {}\n", if o { "off" } else { "on" }),
                };
                self.out(ni, &text);
                Ok(())
            }
            K::TermLen => Ok(()),
            K::ShowRun | K::ShowStart | K::ShowIpBrief | K::ShowIpRoute | K::ShowArp | K::ShowMac | K::ShowVlan | K::ShowIf | K::ShowIfTrunk | K::ShowDebug | K::ShowVersion => {
                let text = self.ios_show(ni, p);
                self.out(ni, &text);
                Ok(())
            }
            K::Hostname => {
                let name = if p.no {
                    String::from(if self.nodes[ni].as_ref().is_some_and(|n| n.bridge.is_some() && !n.model.has(F_ROUTER)) { "Switch" } else { "Router" })
                } else {
                    String::from(p.text("name").unwrap_or(""))
                };
                if !jfn_model::project::valid_name(&name) {
                    return self.ios_err(ni, "% Hostname contains one or more illegal characters.\n");
                }
                // (a startup configuration that boots renames nothing: the project keeps its name)
                if name != self.host(ni) && self.apply(Action::Hostname { dev, name: name.clone() }).is_ok() && !self.booting(ni) {
                    self.cli.reports.push(Report::Hostname { dev, name });
                }
                Ok(())
            }
            K::Interface => self.ios_interface(ni, p, line),
            K::VlanCfg => self.ios_vlan(ni, p),
            K::IpRoute => self.ios_route(ni, p),
            K::IpRouting => {
                let _ = self.apply(Action::Forwarding { dev, on: !p.no });
                // without routing there is no routing process either
                if p.no {
                    let _ = self.apply(Action::Rip { dev, set: crate::rip::RipSet::Process(false) });
                    if let Some(pid) = self.ospf_of(ni).filter(|o| o.cfg.on).map(|o| o.cfg.pid) {
                        let _ = self.apply(Action::Ospf { dev, set: crate::ospf::OspfSet::Process { pid, on: false } });
                    }
                }
                Ok(())
            }
            K::DomainLookup => {
                if let Some(c) = self.cdev_mut(ni) {
                    c.cfg.domain_lookup = !p.no;
                }
                Ok(())
            }
            K::ArpStatic => {
                let ip = Ipv4(p.ip("ip").unwrap_or([0; 4]));
                let mac = if p.no { None } else { p.mac("mac").map(Mac) };
                match self.apply(Action::StaticArp { dev, ip, mac }) {
                    Ok(()) => Ok(()),
                    Err(_) if p.no => self.ios_err(ni, &format!("% No ARP entry for {} found\n", ip)),
                    Err(_) => self.ios_err(ni, &format!("% Invalid ARP entry - {} is not on a connected network\n", ip)),
                }
            }
            K::LineCfg => {
                let name = if p.has("console") {
                    String::from("con 0")
                } else {
                    let a = p.num("first").unwrap_or(0);
                    match p.num("last") {
                        Some(b) => format!("vty {} {}", a, b),
                        None => format!("vty {}", a),
                    }
                };
                let Some(c) = self.cdev_mut(ni) else {
                    return Ok(());
                };
                let i = match c.cfg.lines.iter().position(|l| l.name == name) {
                    Some(i) => i,
                    None => {
                        c.cfg.lines.push(LineCfg { name, opts: Vec::new() });
                        c.cfg.lines.len() - 1
                    }
                };
                c.line = i;
                c.mode = Mode::Line;
                Ok(())
            }
            K::Version => Ok(()),
            K::IpAddress | K::Shutdown | K::Switchport | K::SwMode | K::SwAccess | K::SwNative | K::SwAllowed | K::SwEncap | K::Encap | K::ProxyArp | K::IpMtu | K::Descr => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if let Err(e) = self.ios_if_cmd(ni, &f, p, line) {
                        r = Err(e);
                    }
                }
                r
            }
            K::VlanName => {
                let vids = self.cdev(ni).map(|c| c.vlans.clone()).unwrap_or_default();
                for vid in vids {
                    let name = if p.no { format!("VLAN{:04}", vid) } else { String::from(p.text("name").unwrap_or("")) };
                    let _ = self.apply(Action::Vlan { dev, vid, name: Some(name), remove: false });
                }
                Ok(())
            }
            K::LinePassword | K::LineLogin | K::LineLogging | K::LineTimeout => {
                let (key, val) = match p.cmd {
                    K::LinePassword => ("password", format!("password {}", p.text("pw").unwrap_or(""))),
                    K::LineLogin => ("login", String::from("login")),
                    K::LineLogging => ("logging synchronous", String::from("logging synchronous")),
                    _ => {
                        let m = p.num("min").unwrap_or(10);
                        ("exec-timeout", match p.num("sec") {
                            Some(s) => format!("exec-timeout {} {}", m, s),
                            None => format!("exec-timeout {} 0", m),
                        })
                    }
                };
                if let Some(c) = self.cdev_mut(ni) {
                    let i = c.line;
                    if let Some(l) = c.cfg.lines.get_mut(i) {
                        l.opts.retain(|o| !o.starts_with(key));
                        if !p.no {
                            l.opts.push(val);
                        }
                    }
                }
                Ok(())
            }
            K::ShowTcpBrief | K::ShowTcpStats | K::ShowOpenPorts => {
                let text = match p.cmd {
                    K::ShowTcpBrief => self.ios_tcp_brief(ni, p.has("all")),
                    K::ShowTcpStats => self.ios_tcp_stats(ni),
                    _ => self.ios_open_ports(ni),
                };
                self.out(ni, &text);
                Ok(())
            }
            K::Helper => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if let Err(e) = self.ios_helper(ni, &f, p) {
                        r = Err(e);
                    }
                }
                r
            }
            K::DhcpPool | K::DhcpExcluded | K::PoolNetwork | K::PoolRouter | K::PoolDns | K::PoolDomain | K::PoolLease | K::ShowDhcpBinding | K::ShowDhcpPool | K::ShowDhcpConflict | K::ClearDhcp => self.ios_dhcp(ni, p),
            K::NtpServer | K::NtpMaster => self.ios_ntp(ni, p),
            K::ShowStp | K::DebugStp | K::ClearStp | K::StpMode | K::StpPriority | K::StpTimer => self.ios_stp(ni, p),
            K::StpIf => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if let Err(e) = self.ios_stp_if(ni, &f, p, line) {
                        r = Err(e);
                    }
                }
                r
            }
            K::AclStd | K::AclExt | K::AclRemark | K::AclNamed | K::AclSeqNo | K::AclReseq | K::ShowAcl | K::ClearAcl => self.ios_acl(ni, mode, p, line),
            K::AccessGroup | K::NatSide => {
                let ifs = self.cdev(ni).map(|c| c.ifs.clone()).unwrap_or_default();
                let mut r = Ok(());
                for f in ifs {
                    if let Err(e) = self.ios_if_filter(ni, &f, p) {
                        r = Err(e);
                    }
                }
                r
            }
            K::RouterRip
            | K::RipVersion
            | K::RipNetwork
            | K::RipPassive
            | K::RipAutoSummary
            | K::RipDefault
            | K::RipRedist
            | K::RipTimers
            | K::RipDistance
            | K::RipMaxPaths
            | K::RipIfVersion
            | K::ShowRipDb
            | K::ShowProtocols
            | K::ClearRoute => self.ios_rip(ni, p),
            K::NatPool | K::NatList | K::NatStatic | K::NatTimeout | K::ShowNatTrans | K::ShowNatStats | K::ClearNat => self.ios_nat(ni, p),
            K::Ipv6Global | K::Ipv6Route | K::Ipv6Neighbor | K::Ipv6If | K::ShowIpv6 | K::Ping6 | K::Trace6 | K::Debug6 | K::ClearIpv6 => self.ios_ip6(ni, p),
            K::RouterOspf
            | K::OspfRouterId
            | K::OspfNetwork
            | K::OspfPassive
            | K::OspfAutoCost
            | K::OspfDefault
            | K::OspfRedist
            | K::OspfStub
            | K::OspfDistance
            | K::OspfMaxPaths
            | K::OspfLogAdj
            | K::OspfIf
            | K::ShowOspf
            | K::ShowOspfNbr
            | K::ShowOspfIf
            | K::ShowOspfDb
            | K::DebugOspf
            | K::ClearOspf => self.ios_ospf(ni, p),
            K::IpHost | K::Ipv6Host | K::NameServer => self.ios_names(ni, p),
            K::ShowDot11 => self.ios_dot11(ni, p),
        }
    }

    /// the answer to a question of the console
    pub(crate) fn ios_answer(&mut self, ni: usize, a: Ask, line: &str) {
        match a.kind {
            AskKind::CopyRunStart => {
                let t = line.trim();
                if t.is_empty() || t.eq_ignore_ascii_case("startup-config") {
                    self.ios_save(ni);
                } else {
                    self.out(ni, &format!("%Error opening {} (Invalid argument)\n", t));
                }
            }
            AskKind::Ping6Out { dst, count, bytes, timeout } => self.ios_ping6_answer(ni, dst, count, bytes, timeout, line),
            AskKind::ClearOspf => self.ios_ospf_clear(ni, line),
        }
    }

    /// "copy running-config startup-config", "write memory"
    pub(crate) fn ios_save(&mut self, ni: usize) {
        let text = self.ios_config(ni);
        let dev = self.dev_of(ni);
        if let Some(c) = self.cdev_mut(ni) {
            c.cfg.startup = text.clone();
        }
        self.out(ni, "Saving the configuration ...\n[OK]\n");
        self.cli.reports.push(Report::Startup { dev, text });
    }

    /// "interface NAME" (and "no interface NAME")
    fn ios_interface(&mut self, ni: usize, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let name = String::from(p.text("if").unwrap_or(""));
        let (ty, num) = jfn_model::ifname::split(&name);
        let is_port = self.port_index(dev, &name).is_some() && !name.contains('.');
        // a bad number: the marker under the last word
        let at = line.trim_end().rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        if p.no {
            if is_port {
                return self.ios_err(ni, "% Physical interfaces cannot be removed\n");
            }
            let r = if ty == "Loopback" {
                self.apply(Action::Loopback { dev, n: num.parse().unwrap_or(0), add: false })
            } else if ty == "Vlan" {
                self.apply(Action::Svi { dev, vlan: num.parse().unwrap_or(0), add: false })
            } else if name.contains('.') {
                let pending = self.cdev(ni).is_some_and(|c| c.cfg.pending.contains(&name));
                if pending {
                    if let Some(c) = self.cdev_mut(ni) {
                        c.cfg.pending.retain(|x| *x != name);
                    }
                    Ok(())
                } else {
                    self.apply(Action::SubIf { dev, name: name.clone(), vlan: 0, add: false })
                }
            } else {
                Err(ActErr::UnknownIface { dev, iface: name.clone() })
            };
            if let Some(c) = self.cdev_mut(ni) {
                c.cfg.descr.remove(&name);
                c.cfg.methods.remove(&name);
            }
            return match r {
                Ok(()) => Ok(()),
                Err(_) => self.ios_err(ni, &format!("% Interface {} does not exist\n", name)),
            };
        }
        if is_port {
            // a port named in a configuration that boots is up unless the block says
            // "shutdown" (the running configuration writes only "shutdown"; a port the
            // configuration does not name keeps the model's default, down on a router)
            if self.booting(ni) {
                let ip = self.stack(ni).is_some_and(|s| s.iface_by_name(&name).is_some());
                let _ = if ip { self.apply(Action::IfUp { dev, iface: name.clone(), up: true }) } else { self.apply(Action::PortAdmin { dev, port: name.clone(), up: true }) };
            }
            if let Some(c) = self.cdev_mut(ni) {
                if c.booting {
                    c.cfg.methods.entry(name.clone()).or_insert(Method::Nvram);
                }
                c.mode = Mode::If;
                c.ifs = alloc::vec![name];
            }
            return Ok(());
        }
        let made = if ty == "Loopback" {
            match num.parse::<u16>() {
                Ok(n) if n <= 2_147 => self.apply(Action::Loopback { dev, n, add: true }).map(|_| Mode::If),
                _ => return self.ios_marker(ni, line, at),
            }
        } else if ty == "Vlan" {
            match num.parse::<u16>() {
                Ok(v) if (1..=4094).contains(&v) => {
                    // switches make a new VLAN interface up (Vlan1 alone ships shut down)
                    let new = !self.stack(ni).is_some_and(|s| s.iface_by_name(&name).is_some());
                    let r = self.apply(Action::Svi { dev, vlan: v, add: true });
                    if new && r.is_ok() {
                        let _ = self.apply(Action::IfUp { dev, iface: name.clone(), up: true });
                    }
                    r.map(|_| Mode::If)
                }
                _ => return self.ios_marker(ni, line, at),
            }
        } else if name.contains('.') {
            let exists = self.stack(ni).is_some_and(|s| s.iface_by_name(&name).is_some());
            if !exists
                && let Some(c) = self.cdev_mut(ni)
                && !c.cfg.pending.contains(&name)
            {
                c.cfg.pending.push(name.clone());
            }
            Ok(Mode::SubIf)
        } else if self.stack(ni).is_some_and(|s| s.iface_by_name(&name).is_some()) {
            Ok(Mode::If)
        } else {
            Err(ActErr::UnknownIface { dev, iface: name.clone() })
        };
        match made {
            Ok(m) => {
                if let Some(c) = self.cdev_mut(ni) {
                    if c.booting {
                        c.cfg.methods.entry(name.clone()).or_insert(Method::Nvram);
                    }
                    c.mode = m;
                    c.ifs = alloc::vec![name];
                }
                Ok(())
            }
            Err(_) => self.ios_marker(ni, line, at),
        }
    }

    /// "vlan N" (and "no vlan N")
    fn ios_vlan(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let vid = p.num("vid").unwrap_or(1) as u16;
        if p.no {
            if vid == 1 || (1002..=1005).contains(&vid) {
                return self.ios_err(ni, &format!("%Default VLAN {} may not be deleted.\n", vid));
            }
            let _ = self.apply(Action::Vlan { dev, vid, name: None, remove: true });
            return Ok(());
        }
        let exists = self.bridge(dev).is_some_and(|b| b.vlans.contains_key(&vid));
        if !exists {
            let _ = self.apply(Action::Vlan { dev, vid, name: None, remove: false });
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.mode = Mode::Vlan;
            c.vlans = alloc::vec![vid];
        }
        Ok(())
    }

    /// "ip route ..." (and "no ip route ...")
    fn ios_route(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let net = Ipv4(p.ip("net").unwrap_or([0; 4]));
        let len = p.mask("mask").unwrap_or(0);
        let prefix = Prefix4 { addr: net, len };
        if prefix.trunc().addr != net {
            return self.ios_err(ni, "%Inconsistent address and mask\n");
        }
        let via = p.ip("gw").map(Ipv4);
        let iface = p.text("out").map(String::from);
        if p.no {
            return match self.apply(Action::Route { dev, prefix, via, iface: None, ad: 1, add: false }) {
                Ok(()) => Ok(()),
                Err(_) => self.ios_err(ni, "%No matching route to delete\n"),
            };
        }
        if let Some(gw) = via
            && self.stack(ni).is_some_and(|s| s.is_mine(gw))
        {
            return self.ios_err(ni, "%Invalid next hop address (it's this router)\n");
        }
        let ad = p.num("ad").unwrap_or(1) as u8;
        match self.apply(Action::Route { dev, prefix, via, iface, ad, add: true }) {
            Ok(()) => Ok(()),
            Err(_) => self.ios_err(ni, "%Invalid next hop address\n"),
        }
    }

    /// a command of interface mode for one interface
    fn ios_if_cmd(&mut self, ni: usize, name: &str, p: &Parsed<K>, line: &str) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let iface = self.stack(ni).and_then(|s| s.iface_by_name(name));
        let port = if name.contains('.') { None } else { self.port_index(dev, name) };
        let switch_port = port.is_some_and(|pi| self.bridge(dev).is_some_and(|b| b.ports.get(pi as usize).is_some_and(|bp| bp.member)));
        let booting = self.booting(ni);
        match p.cmd {
            K::IpAddress => self.ios_ip_address(ni, name, iface, switch_port, p),
            K::Shutdown => {
                let up = p.no;
                let r = match (iface, port) {
                    (Some(_), _) => self.apply(Action::IfUp { dev, iface: String::from(name), up }),
                    (None, Some(_)) => self.apply(Action::PortAdmin { dev, port: String::from(name), up }),
                    (None, None) => Ok(()),
                };
                let _ = r;
                Ok(())
            }
            K::Descr => {
                if let Some(c) = self.cdev_mut(ni) {
                    if p.no {
                        c.cfg.descr.remove(name);
                    } else {
                        c.cfg.descr.insert(String::from(name), String::from(p.text("text").unwrap_or("")));
                    }
                }
                Ok(())
            }
            K::ProxyArp => match iface {
                Some(_) => {
                    let _ = self.apply(Action::ProxyArp { dev, iface: String::from(name), on: !p.no });
                    Ok(())
                }
                None => self.ios_err(ni, "% IP addresses may not be configured on L2 links.\n"),
            },
            K::IpMtu => match iface {
                Some(_) => {
                    let mtu = if p.no { 1500 } else { p.num("mtu").unwrap_or(1500) as u16 };
                    let _ = self.apply(Action::Mtu { dev, iface: String::from(name), mtu });
                    Ok(())
                }
                None => self.ios_err(ni, "% IP addresses may not be configured on L2 links.\n"),
            },
            K::Encap => {
                let vid = p.num("vid").unwrap_or(1) as u16;
                match self.apply(Action::SubIf { dev, name: String::from(name), vlan: vid, add: true }) {
                    Ok(()) => {
                        if let Some(c) = self.cdev_mut(ni) {
                            c.cfg.pending.retain(|x| x != name);
                            if booting {
                                c.cfg.methods.entry(String::from(name)).or_insert(Method::Nvram);
                            }
                        }
                        Ok(())
                    }
                    Err(_) => {
                        let other = self.stack(ni).and_then(|s| {
                            let base = name.rsplit_once('.').map(|x| x.0).unwrap_or(name);
                            let pi = s.ifaces.iter().position(|f| f.name.eq_ignore_ascii_case(base)).and_then(|i| match s.ifaces[i].kind {
                                IfKind::Port(pp) => Some(pp),
                                _ => None,
                            })?;
                            s.ifaces.iter().find(|f| !f.hidden && f.kind == IfKind::Sub { port: pi, vlan: vid } && !f.name.eq_ignore_ascii_case(name)).map(|f| f.name.clone())
                        });
                        let text = match other {
                            Some(o) => format!("% VLAN {} is already in use on {}.\n", vid, o),
                            None => String::from("% Invalid input detected at '^' marker.\n\n"),
                        };
                        self.ios_err(ni, &text)
                    }
                }
            }
            K::Switchport | K::SwMode | K::SwAccess | K::SwNative | K::SwAllowed | K::SwEncap => {
                if !switch_port && !(p.cmd == K::Switchport && port.is_some()) {
                    let at = line.len() - line.trim_start().len();
                    return self.ios_marker(ni, line, at);
                }
                self.ios_switchport(ni, name, p)
            }
            _ => Ok(()),
        }
    }

    fn ios_switchport(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let port = String::from(name);
        let set = match p.cmd {
            K::Switchport => {
                // "no switchport": a routed port of an L3 switch; "switchport": back again
                return match self.apply(Action::RoutedPort { dev, port, routed: p.no }) {
                    Ok(()) => Ok(()),
                    Err(_) => self.ios_err(ni, "% Invalid input detected at '^' marker.\n\n"),
                };
            }
            K::SwEncap => return Ok(()),
            K::SwMode => SwSet::Mode(match (p.no, p.text("mode")) {
                (true, _) => SwMode::Dynamic,
                (false, Some("access")) => SwMode::Access,
                (false, Some("trunk")) => SwMode::Trunk,
                _ => SwMode::Dynamic,
            }),
            K::SwAccess => {
                let v = if p.no { 1 } else { p.num("vid").unwrap_or(1) as u16 };
                if !self.bridge(dev).is_some_and(|b| b.vlans.contains_key(&v)) {
                    self.out(ni, &format!("% Access VLAN does not exist. Creating vlan {}\n", v));
                }
                SwSet::Access(v)
            }
            K::SwNative => SwSet::Native(if p.no { 1 } else { p.num("vid").unwrap_or(1) as u16 }),
            _ => {
                let cur = self.port_index(dev, name).and_then(|pi| self.bridge(dev).and_then(|b| b.ports.get(pi as usize)).map(|bp| bp.allowed.clone())).unwrap_or_default();
                let list = p.text("list").map(VlanSet::parse);
                let set = match (p.no, p.text("op"), list) {
                    (true, _, _) | (false, Some("all"), _) => VlanSet::all(),
                    (false, Some("none"), _) => VlanSet::none(),
                    (false, _, Some(None)) => return self.ios_err(ni, "% Bad VLAN list: use numbers and ranges such as 10,20-30\n"),
                    (false, Some("add"), Some(Some(l))) => {
                        let mut s = cur;
                        for v in 1..=4094 {
                            if l.contains(v) {
                                s.insert(v);
                            }
                        }
                        s
                    }
                    (false, Some("remove"), Some(Some(l))) => {
                        let mut s = cur;
                        for v in 1..=4094 {
                            if l.contains(v) {
                                s.remove(v);
                            }
                        }
                        s
                    }
                    (false, Some("except"), Some(Some(l))) => {
                        let mut s = VlanSet::all();
                        for v in 1..=4094 {
                            if l.contains(v) {
                                s.remove(v);
                            }
                        }
                        s
                    }
                    (false, _, Some(Some(l))) => l,
                    _ => VlanSet::all(),
                };
                SwSet::Allowed(set)
            }
        };
        match self.apply(Action::Switchport { dev, port, set }) {
            Ok(()) => Ok(()),
            Err(_) => self.ios_err(ni, "% Invalid input detected at '^' marker.\n\n"),
        }
    }

    /// "ip address A M [secondary]" and its no-forms
    fn ios_ip_address(&mut self, ni: usize, name: &str, iface: Option<u16>, switch_port: bool, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let booting = self.booting(ni);
        let Some(i) = iface else {
            let pending = self.cdev(ni).is_some_and(|c| c.cfg.pending.iter().any(|x| x == name));
            let text = if pending {
                "% Give the subinterface its VLAN first (encapsulation dot1Q)\n"
            } else if switch_port {
                "% IP addresses may not be configured on L2 links.\n"
            } else {
                "% Invalid input detected at '^' marker.\n\n"
            };
            return self.ios_err(ni, text);
        };
        let Some(st) = self.stack(ni) else {
            return Ok(());
        };
        let cur: Vec<Prefix4> = st.ifaces[i as usize].addrs.clone();
        let lo = st.ifaces[i as usize].is_loopback();
        let secondary = p.has("secondary");
        let new: Vec<Prefix4> = if p.no {
            match (p.ip("addr"), p.mask("mask")) {
                (Some(a), Some(m)) => {
                    let pf = Prefix4 { addr: Ipv4(a), len: m };
                    if secondary {
                        if !cur.iter().skip(1).any(|x| *x == pf) {
                            return self.ios_err(ni, "% Invalid address\n");
                        }
                        cur.iter().copied().filter(|x| *x != pf).collect()
                    } else if cur.first() == Some(&pf) {
                        if cur.len() > 1 {
                            return self.ios_err(ni, "Must delete secondary before deleting primary\n");
                        }
                        Vec::new()
                    } else {
                        return self.ios_err(ni, "% Invalid address\n");
                    }
                }
                _ => Vec::new(),
            }
        } else {
            let a = Ipv4(p.ip("addr").unwrap_or([0; 4]));
            let len = p.mask("mask").unwrap_or(0);
            let pf = Prefix4 { addr: a, len };
            let first = a.0[0];
            if a.is_unspecified() || a.is_loopback() || a.is_multicast() || a.is_broadcast() || first >= 240 || first == 0 {
                return self.ios_err(ni, &format!("Not a valid host address - {}\n", a));
            }
            if (len == 32 && !lo) || len == 0 || pf.is_reserved_host(a) {
                return self.ios_err(ni, &format!("Bad mask /{} for address {}\n", len, a));
            }
            // no subnet twice: neither on another interface nor on this one
            for (j, f) in st.ifaces.iter().enumerate() {
                if f.hidden {
                    continue;
                }
                for q in &f.addrs {
                    let same_if = j == i as usize;
                    if same_if && (!secondary && Some(q) == cur.first()) {
                        continue;
                    }
                    if *q == pf && same_if {
                        continue;
                    }
                    if q.trunc().overlaps(&pf.trunc()) {
                        let what = if same_if && secondary { String::from(name) } else { f.name.clone() };
                        return self.ios_err(ni, &format!("% {} overlaps with {}\n", pf.trunc().addr, what));
                    }
                }
            }
            if secondary {
                if cur.is_empty() {
                    return self.ios_err(ni, "% Primary address not configured on this interface\n");
                }
                let mut v = cur.clone();
                if !v.contains(&pf) {
                    v.push(pf);
                }
                v
            } else {
                let mut v = alloc::vec![pf];
                v.extend(cur.iter().skip(1).copied().filter(|x| *x != pf));
                v
            }
        };
        if new == cur {
            return Ok(());
        }
        for q in &cur {
            let _ = self.apply(Action::IfAddr { dev, iface: String::from(name), prefix: *q, add: false });
        }
        let mut r = Ok(());
        for q in &new {
            if let Err(e) = self.apply(Action::IfAddr { dev, iface: String::from(name), prefix: *q, add: true }) {
                r = self.ios_err(ni, &format!("% {}\n", e));
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.cfg.methods.insert(String::from(name), if booting { Method::Nvram } else { Method::Manual });
        }
        r
    }

    /// "ping ..." in EXEC
    fn ios_ping(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let target = p.text("target").unwrap_or("");
        if netpkt::Ipv6::parse(target).is_some() {
            return self.ios_ping6(ni, p);
        }
        let Some(dst) = Ipv4::parse(target) else {
            // a name: the host table or the name servers (cli/names.rs)
            return self.ios_ping_name(ni, p, target, false);
        };
        let src = self.ios_ping_src(ni, p)?;
        let count = p.num("count").unwrap_or(5) as u32;
        let bytes = p.num("bytes").unwrap_or(100) as u16;
        let timeout = p.num("secs").unwrap_or(2) as u64 * SEC;
        self.ios_ping_start(ni, dst, count, bytes, timeout, src, p.has("df-bit"));
        Ok(())
    }

    /// the source of an IPv4 ping ("source A.B.C.D", "source IF"), checked
    pub(crate) fn ios_ping_src(&mut self, ni: usize, p: &Parsed<K>) -> Result<Option<Ipv4>, String> {
        let src = match (p.ip("src"), p.text("srcif")) {
            (Some(a), _) => Some(Ipv4(a)),
            (None, Some(f)) => match self.stack(ni).and_then(|s| s.iface_by_name(f).and_then(|i| s.ifaces[i as usize].primary())) {
                Some(a) => Some(a),
                None => return self.ios_err(ni, &format!("% Invalid source interface - {} has no IP address\n", f)).map(|_| None),
            },
            _ => None,
        };
        if let Some(s) = src
            && !self.stack(ni).is_some_and(|st| st.is_mine(s))
        {
            return self.ios_err(ni, &format!("% Invalid source address- IP address not on any of our up interfaces\n{}", "")).map(|_| None);
        }
        Ok(src)
    }

    /// "traceroute ..." in EXEC
    fn ios_traceroute(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let target = p.text("target").unwrap_or("");
        if netpkt::Ipv6::parse(target).is_some() {
            return self.ios_trace6(ni, p);
        }
        let Some(dst) = Ipv4::parse(target) else {
            let lookup = self.cdev(ni).is_some_and(|c| c.cfg.domain_lookup);
            let text = if lookup {
                format!("Resolving \"{}\" via DNS (255.255.255.255) ...\n% Unrecognized host or address, or protocol not running.\n\n", target)
            } else {
                String::from("% Unrecognized host or address, or protocol not running.\n\n")
            };
            return self.ios_err(ni, &text);
        };
        let probes = p.num("probes").unwrap_or(3).clamp(1, 255) as u8;
        let timeout = p.num("secs").unwrap_or(3) as u64 * SEC;
        let max = p.num("max").unwrap_or(30).clamp(1, 255) as u8;
        self.ios_trace_start(ni, dst, probes, timeout, max);
        Ok(())
    }

    /// the interfaces of IOS shows in their order: ports with their subinterfaces, loopbacks,
    /// then the bridge's interfaces; (name, IPv4 interface, port)
    pub(crate) fn ios_ifaces(&self, ni: usize) -> Vec<(String, Option<u16>, Option<u16>)> {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return Vec::new();
        };
        let mut v = Vec::new();
        let st = n.ip.as_ref();
        for (pi, p) in n.ports.iter().enumerate() {
            if matches!(p.spec.media, Media::Console | Media::Usb) {
                continue;
            }
            let f = st.and_then(|s| s.iface_of_port(pi as u16)).filter(|&f| st.is_some_and(|s| !s.ifaces[f as usize].hidden));
            v.push((p.spec.name.clone(), f, Some(pi as u16)));
            if let Some(s) = st {
                let mut subs: Vec<(u32, u16)> = s
                    .ifaces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !f.hidden && matches!(f.kind, IfKind::Sub { port, .. } if port == pi as u16))
                    .map(|(i, f)| (f.name.rsplit_once('.').and_then(|x| x.1.parse().ok()).unwrap_or(0), i as u16))
                    .collect();
                subs.sort();
                for (_, i) in subs {
                    v.push((s.ifaces[i as usize].name.clone(), Some(i), Some(pi as u16)));
                }
            }
        }
        if let Some(s) = st {
            let mut los: Vec<(u16, u16)> = s.ifaces.iter().enumerate().filter_map(|(i, f)| if f.hidden { None } else if let IfKind::Loopback(k) = f.kind { Some((k, i as u16)) } else { None }).collect();
            los.sort();
            let mut svis: Vec<(u16, u16)> = s.ifaces.iter().enumerate().filter_map(|(i, f)| if f.hidden { None } else if let IfKind::Svi(k) = f.kind { Some((k, i as u16)) } else { None }).collect();
            svis.sort();
            for (_, i) in los.into_iter().chain(svis) {
                v.push((s.ifaces[i as usize].name.clone(), Some(i), None));
            }
        }
        v
    }
}

