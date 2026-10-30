// mib -- a small built-in MIB: the objects of SNMPv2-MIB (system, snmp, the notifications of
// RFC 3418), IF-MIB (interfaces, ifTable, ifXTable, linkDown and linkUp of RFC 2863), IP-MIB
// (ipForwarding, ipDefaultTTL, ipAddrTable) and IP-FORWARD-MIB (ipCidrRouteTable of RFC 2096)
// with the nodes of SNMPv2-SMI above them: names, OIDs, syntax, access, the names of enumerated
// values; for the tools of a shell and for dissectors
//
//   OBJECTS                    every object in OID order (Obj: module, name, oid, kind, syntax,
//                              access, enums)
//   find(oid) / by_name(name)  an object exactly
//   object_of(oid)             the object an OID names or is an instance of, with the arcs of the
//                              instance (sysName.0: sysName and [0])
//   name_of(oid, numeric)      "SNMPv2-MIB::sysName.0", "IF-MIB::ifDescr.2",
//                              "SNMPv2-SMI::enterprises.32473.1.4"; numeric: ".1.3.6.1.2.1.1.5.0"
//   lookup(text)               an OID from what a user types: "SNMPv2-MIB::sysName.0",
//                              "sysName.0", "ifTable", "iso.3.6.1", "1.3.6.1.2.1.1.5.0" or with a
//                              leading dot
//   show(oid, value, numeric)  the value as the tools of net-snmp print it after "=": "STRING:
//                              R1", "INTEGER: up(1)", "Timeticks: (500) 0:00:05.00", "OID:
//                              IF-MIB::linkDown" (the exceptions in our own words)
//   bare(oid, value)           the same without the type (snmpget -Oq): "R1", "up"
//   ticks_text(t)              "(500) 0:00:05.00", "(8640000) 1 day, 0:00:00.00"
//   children(oid)              the objects one level below a node (a tree of the MIB)
//
// Names and syntax follow the MIB modules; the table has only what JFNetworker's agent serves
// and what names it, no descriptions (the RFCs have them).  The access is the agent's: objects
// a module allows to write but the agent does not (ipForwarding, ipDefaultTTL,
// ifLinkUpDownTrapEnable, ifPromiscuousMode) are read-only here.  A name the table does not know
// is written with the arcs below the nearest one it knows, as net-snmp does ("iso.2.999").

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ber::{self, Oid, Value};

/// what an object is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    /// a node of the tree (system, ifMIB, enterprises)
    Node,
    /// a scalar: its instance is .0
    Scalar,
    Table,
    /// the row of a table
    Entry,
    /// a column: its instances are the indexes of the rows
    Column,
    /// a notification (a value of snmpTrapOID.0)
    Notification,
}

/// the syntax of a scalar or a column
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    None,
    Integer,
    /// text up to 255 octets (RFC 2579)
    DisplayString,
    /// octets shown as hex digits with colons (RFC 2579)
    PhysAddress,
    Octets,
    Oid,
    IpAddress,
    Counter32,
    Gauge32,
    TimeTicks,
    Counter64,
}

impl Syntax {
    /// the BER tag of its values
    pub fn tag(self) -> u8 {
        match self {
            Syntax::Integer => ber::T_INTEGER,
            Syntax::DisplayString | Syntax::PhysAddress | Syntax::Octets => ber::T_OCTETS,
            Syntax::Oid => ber::T_OID,
            Syntax::IpAddress => ber::T_IPADDRESS,
            Syntax::Counter32 => ber::T_COUNTER32,
            Syntax::Gauge32 => ber::T_GAUGE32,
            Syntax::TimeTicks => ber::T_TIMETICKS,
            Syntax::Counter64 => ber::T_COUNTER64,
            Syntax::None => 0,
        }
    }
    /// the name the MIB module gives it
    pub fn name(self) -> &'static str {
        match self {
            Syntax::None => "",
            Syntax::Integer => "INTEGER",
            Syntax::DisplayString => "DisplayString",
            Syntax::PhysAddress => "PhysAddress",
            Syntax::Octets => "OCTET STRING",
            Syntax::Oid => "OBJECT IDENTIFIER",
            Syntax::IpAddress => "IpAddress",
            Syntax::Counter32 => "Counter32",
            Syntax::Gauge32 => "Gauge32",
            Syntax::TimeTicks => "TimeTicks",
            Syntax::Counter64 => "Counter64",
        }
    }
}

/// who may do what with an object
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    NotAccessible,
    ReadOnly,
    ReadWrite,
    /// only in notifications (snmpTrapOID)
    Notify,
}

impl Access {
    pub fn name(self) -> &'static str {
        match self {
            Access::NotAccessible => "not-accessible",
            Access::ReadOnly => "read-only",
            Access::ReadWrite => "read-write",
            Access::Notify => "accessible-for-notify",
        }
    }
}

/// an object of the MIB
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obj {
    pub module: &'static str,
    pub name: &'static str,
    pub oid: &'static [u32],
    pub kind: ObjKind,
    pub syntax: Syntax,
    pub access: Access,
    /// the names of an INTEGER's values ("up" for 1)
    pub enums: &'static [(i64, &'static str)],
}

const SMI: &str = "SNMPv2-SMI";
const SNMP: &str = "SNMPv2-MIB";
const IF: &str = "IF-MIB";
const IP: &str = "IP-MIB";
const FWD: &str = "IP-FORWARD-MIB";

const UPDOWN: &[(i64, &str)] = &[(1, "up"), (2, "down"), (3, "testing")];
const OPER: &[(i64, &str)] = &[(1, "up"), (2, "down"), (3, "testing"), (4, "unknown"), (5, "dormant"), (6, "notPresent"), (7, "lowerLayerDown")];
const IFTYPE: &[(i64, &str)] = &[(1, "other"), (6, "ethernetCsmacd"), (22, "propPointToPointSerial"), (24, "softwareLoopback"), (53, "propVirtual"), (71, "ieee80211"), (135, "l2vlan")];
const ENABLED: &[(i64, &str)] = &[(1, "enabled"), (2, "disabled")];
const TRUTH: &[(i64, &str)] = &[(1, "true"), (2, "false")];
const FORWARDING: &[(i64, &str)] = &[(1, "forwarding"), (2, "notForwarding")];
const ROUTE_TYPE: &[(i64, &str)] = &[(1, "other"), (2, "reject"), (3, "local"), (4, "remote")];
const ROUTE_PROTO: &[(i64, &str)] = &[(1, "other"), (2, "local"), (3, "netmgmt"), (4, "icmp"), (5, "egp"), (6, "ggp"), (7, "hello"), (8, "rip"), (9, "isIs"), (10, "esIs"), (11, "ciscoIgrp"), (12, "bbnSpfIgp"), (13, "ospf"), (14, "bgp"), (15, "idpr"), (16, "ciscoEigrp")];
const ROW_STATUS: &[(i64, &str)] = &[(1, "active"), (2, "notInService"), (3, "notReady"), (4, "createAndGo"), (5, "createAndWait"), (6, "destroy")];

const fn node(module: &'static str, name: &'static str, oid: &'static [u32]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Node, syntax: Syntax::None, access: Access::NotAccessible, enums: &[] }
}

const fn scalar(module: &'static str, name: &'static str, oid: &'static [u32], syntax: Syntax, access: Access) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Scalar, syntax, access, enums: &[] }
}

const fn scalar_enum(module: &'static str, name: &'static str, oid: &'static [u32], access: Access, enums: &'static [(i64, &'static str)]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Scalar, syntax: Syntax::Integer, access, enums }
}

const fn table(module: &'static str, name: &'static str, oid: &'static [u32]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Table, syntax: Syntax::None, access: Access::NotAccessible, enums: &[] }
}

const fn entry(module: &'static str, name: &'static str, oid: &'static [u32]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Entry, syntax: Syntax::None, access: Access::NotAccessible, enums: &[] }
}

const fn col(module: &'static str, name: &'static str, oid: &'static [u32], syntax: Syntax, access: Access) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Column, syntax, access, enums: &[] }
}

const fn col_enum(module: &'static str, name: &'static str, oid: &'static [u32], access: Access, enums: &'static [(i64, &'static str)]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Column, syntax: Syntax::Integer, access, enums }
}

const fn notif(module: &'static str, name: &'static str, oid: &'static [u32]) -> Obj {
    Obj { module, name, oid, kind: ObjKind::Notification, syntax: Syntax::None, access: Access::Notify, enums: &[] }
}

use Access::{Notify, ReadOnly as RO, ReadWrite as RW};
use Syntax::{Counter32 as C32, Counter64 as C64, DisplayString as DS, Gauge32 as G32, Integer as INT, IpAddress as IPA, TimeTicks as TT};

/// the objects in OID order
pub const OBJECTS: &[Obj] = &[
    node(SMI, "zeroDotZero", &[0, 0]),
    node(SMI, "iso", &[1]),
    node(SMI, "org", &[1, 3]),
    node(SMI, "dod", &[1, 3, 6]),
    node(SMI, "internet", &[1, 3, 6, 1]),
    node(SMI, "mgmt", &[1, 3, 6, 1, 2]),
    node(SMI, "mib-2", &[1, 3, 6, 1, 2, 1]),
    node(SNMP, "system", &[1, 3, 6, 1, 2, 1, 1]),
    scalar(SNMP, "sysDescr", &[1, 3, 6, 1, 2, 1, 1, 1], DS, RO),
    scalar(SNMP, "sysObjectID", &[1, 3, 6, 1, 2, 1, 1, 2], Syntax::Oid, RO),
    scalar(SNMP, "sysUpTime", &[1, 3, 6, 1, 2, 1, 1, 3], TT, RO),
    scalar(SNMP, "sysContact", &[1, 3, 6, 1, 2, 1, 1, 4], DS, RW),
    scalar(SNMP, "sysName", &[1, 3, 6, 1, 2, 1, 1, 5], DS, RW),
    scalar(SNMP, "sysLocation", &[1, 3, 6, 1, 2, 1, 1, 6], DS, RW),
    scalar(SNMP, "sysServices", &[1, 3, 6, 1, 2, 1, 1, 7], INT, RO),
    node(IF, "interfaces", &[1, 3, 6, 1, 2, 1, 2]),
    scalar(IF, "ifNumber", &[1, 3, 6, 1, 2, 1, 2, 1], INT, RO),
    table(IF, "ifTable", &[1, 3, 6, 1, 2, 1, 2, 2]),
    entry(IF, "ifEntry", &[1, 3, 6, 1, 2, 1, 2, 2, 1]),
    col(IF, "ifIndex", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 1], INT, RO),
    col(IF, "ifDescr", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 2], DS, RO),
    col_enum(IF, "ifType", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 3], RO, IFTYPE),
    col(IF, "ifMtu", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 4], INT, RO),
    col(IF, "ifSpeed", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 5], G32, RO),
    col(IF, "ifPhysAddress", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 6], Syntax::PhysAddress, RO),
    col_enum(IF, "ifAdminStatus", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 7], RW, UPDOWN),
    col_enum(IF, "ifOperStatus", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 8], RO, OPER),
    col(IF, "ifLastChange", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 9], TT, RO),
    col(IF, "ifInOctets", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 10], C32, RO),
    col(IF, "ifInUcastPkts", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 11], C32, RO),
    col(IF, "ifInNUcastPkts", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 12], C32, RO),
    col(IF, "ifInDiscards", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 13], C32, RO),
    col(IF, "ifInErrors", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 14], C32, RO),
    col(IF, "ifInUnknownProtos", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 15], C32, RO),
    col(IF, "ifOutOctets", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 16], C32, RO),
    col(IF, "ifOutUcastPkts", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 17], C32, RO),
    col(IF, "ifOutNUcastPkts", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 18], C32, RO),
    col(IF, "ifOutDiscards", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 19], C32, RO),
    col(IF, "ifOutErrors", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 20], C32, RO),
    col(IF, "ifOutQLen", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 21], G32, RO),
    col(IF, "ifSpecific", &[1, 3, 6, 1, 2, 1, 2, 2, 1, 22], Syntax::Oid, RO),
    node(IP, "ip", &[1, 3, 6, 1, 2, 1, 4]),
    scalar_enum(IP, "ipForwarding", &[1, 3, 6, 1, 2, 1, 4, 1], RO, FORWARDING),
    scalar(IP, "ipDefaultTTL", &[1, 3, 6, 1, 2, 1, 4, 2], INT, RO),
    table(IP, "ipAddrTable", &[1, 3, 6, 1, 2, 1, 4, 20]),
    entry(IP, "ipAddrEntry", &[1, 3, 6, 1, 2, 1, 4, 20, 1]),
    col(IP, "ipAdEntAddr", &[1, 3, 6, 1, 2, 1, 4, 20, 1, 1], IPA, RO),
    col(IP, "ipAdEntIfIndex", &[1, 3, 6, 1, 2, 1, 4, 20, 1, 2], INT, RO),
    col(IP, "ipAdEntNetMask", &[1, 3, 6, 1, 2, 1, 4, 20, 1, 3], IPA, RO),
    col(IP, "ipAdEntBcastAddr", &[1, 3, 6, 1, 2, 1, 4, 20, 1, 4], INT, RO),
    col(IP, "ipAdEntReasmMaxSize", &[1, 3, 6, 1, 2, 1, 4, 20, 1, 5], INT, RO),
    node(FWD, "ipForward", &[1, 3, 6, 1, 2, 1, 4, 24]),
    scalar(FWD, "ipCidrRouteNumber", &[1, 3, 6, 1, 2, 1, 4, 24, 3], G32, RO),
    table(FWD, "ipCidrRouteTable", &[1, 3, 6, 1, 2, 1, 4, 24, 4]),
    entry(FWD, "ipCidrRouteEntry", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1]),
    col(FWD, "ipCidrRouteDest", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 1], IPA, RO),
    col(FWD, "ipCidrRouteMask", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 2], IPA, RO),
    col(FWD, "ipCidrRouteTos", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 3], INT, RO),
    col(FWD, "ipCidrRouteNextHop", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 4], IPA, RO),
    col(FWD, "ipCidrRouteIfIndex", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 5], INT, RO),
    col_enum(FWD, "ipCidrRouteType", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 6], RO, ROUTE_TYPE),
    col_enum(FWD, "ipCidrRouteProto", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 7], RO, ROUTE_PROTO),
    col(FWD, "ipCidrRouteAge", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 8], INT, RO),
    col(FWD, "ipCidrRouteInfo", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 9], Syntax::Oid, RO),
    col(FWD, "ipCidrRouteNextHopAS", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 10], INT, RO),
    col(FWD, "ipCidrRouteMetric1", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 11], INT, RO),
    col(FWD, "ipCidrRouteMetric2", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 12], INT, RO),
    col(FWD, "ipCidrRouteMetric3", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 13], INT, RO),
    col(FWD, "ipCidrRouteMetric4", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 14], INT, RO),
    col(FWD, "ipCidrRouteMetric5", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 15], INT, RO),
    col_enum(FWD, "ipCidrRouteStatus", &[1, 3, 6, 1, 2, 1, 4, 24, 4, 1, 16], RO, ROW_STATUS),
    node(SNMP, "snmp", &[1, 3, 6, 1, 2, 1, 11]),
    scalar(SNMP, "snmpInPkts", &[1, 3, 6, 1, 2, 1, 11, 1], C32, RO),
    scalar(SNMP, "snmpOutPkts", &[1, 3, 6, 1, 2, 1, 11, 2], C32, RO),
    scalar(SNMP, "snmpInBadVersions", &[1, 3, 6, 1, 2, 1, 11, 3], C32, RO),
    scalar(SNMP, "snmpInBadCommunityNames", &[1, 3, 6, 1, 2, 1, 11, 4], C32, RO),
    scalar(SNMP, "snmpInBadCommunityUses", &[1, 3, 6, 1, 2, 1, 11, 5], C32, RO),
    scalar(SNMP, "snmpInASNParseErrs", &[1, 3, 6, 1, 2, 1, 11, 6], C32, RO),
    scalar(SNMP, "snmpInTooBigs", &[1, 3, 6, 1, 2, 1, 11, 8], C32, RO),
    scalar(SNMP, "snmpInNoSuchNames", &[1, 3, 6, 1, 2, 1, 11, 9], C32, RO),
    scalar(SNMP, "snmpInBadValues", &[1, 3, 6, 1, 2, 1, 11, 10], C32, RO),
    scalar(SNMP, "snmpInReadOnlys", &[1, 3, 6, 1, 2, 1, 11, 11], C32, RO),
    scalar(SNMP, "snmpInGenErrs", &[1, 3, 6, 1, 2, 1, 11, 12], C32, RO),
    scalar(SNMP, "snmpInTotalReqVars", &[1, 3, 6, 1, 2, 1, 11, 13], C32, RO),
    scalar(SNMP, "snmpInTotalSetVars", &[1, 3, 6, 1, 2, 1, 11, 14], C32, RO),
    scalar(SNMP, "snmpInGetRequests", &[1, 3, 6, 1, 2, 1, 11, 15], C32, RO),
    scalar(SNMP, "snmpInGetNexts", &[1, 3, 6, 1, 2, 1, 11, 16], C32, RO),
    scalar(SNMP, "snmpInSetRequests", &[1, 3, 6, 1, 2, 1, 11, 17], C32, RO),
    scalar(SNMP, "snmpInGetResponses", &[1, 3, 6, 1, 2, 1, 11, 18], C32, RO),
    scalar(SNMP, "snmpInTraps", &[1, 3, 6, 1, 2, 1, 11, 19], C32, RO),
    scalar(SNMP, "snmpOutTooBigs", &[1, 3, 6, 1, 2, 1, 11, 20], C32, RO),
    scalar(SNMP, "snmpOutNoSuchNames", &[1, 3, 6, 1, 2, 1, 11, 21], C32, RO),
    scalar(SNMP, "snmpOutBadValues", &[1, 3, 6, 1, 2, 1, 11, 22], C32, RO),
    scalar(SNMP, "snmpOutGenErrs", &[1, 3, 6, 1, 2, 1, 11, 24], C32, RO),
    scalar(SNMP, "snmpOutGetRequests", &[1, 3, 6, 1, 2, 1, 11, 25], C32, RO),
    scalar(SNMP, "snmpOutGetNexts", &[1, 3, 6, 1, 2, 1, 11, 26], C32, RO),
    scalar(SNMP, "snmpOutSetRequests", &[1, 3, 6, 1, 2, 1, 11, 27], C32, RO),
    scalar(SNMP, "snmpOutGetResponses", &[1, 3, 6, 1, 2, 1, 11, 28], C32, RO),
    scalar(SNMP, "snmpOutTraps", &[1, 3, 6, 1, 2, 1, 11, 29], C32, RO),
    scalar_enum(SNMP, "snmpEnableAuthenTraps", &[1, 3, 6, 1, 2, 1, 11, 30], RW, ENABLED),
    scalar(SNMP, "snmpSilentDrops", &[1, 3, 6, 1, 2, 1, 11, 31], C32, RO),
    scalar(SNMP, "snmpProxyDrops", &[1, 3, 6, 1, 2, 1, 11, 32], C32, RO),
    node(IF, "ifMIB", &[1, 3, 6, 1, 2, 1, 31]),
    node(IF, "ifMIBObjects", &[1, 3, 6, 1, 2, 1, 31, 1]),
    table(IF, "ifXTable", &[1, 3, 6, 1, 2, 1, 31, 1, 1]),
    entry(IF, "ifXEntry", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1]),
    col(IF, "ifName", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 1], DS, RO),
    col(IF, "ifInMulticastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 2], C32, RO),
    col(IF, "ifInBroadcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 3], C32, RO),
    col(IF, "ifOutMulticastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 4], C32, RO),
    col(IF, "ifOutBroadcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 5], C32, RO),
    col(IF, "ifHCInOctets", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 6], C64, RO),
    col(IF, "ifHCInUcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 7], C64, RO),
    col(IF, "ifHCInMulticastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 8], C64, RO),
    col(IF, "ifHCInBroadcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 9], C64, RO),
    col(IF, "ifHCOutOctets", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 10], C64, RO),
    col(IF, "ifHCOutUcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 11], C64, RO),
    col(IF, "ifHCOutMulticastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 12], C64, RO),
    col(IF, "ifHCOutBroadcastPkts", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 13], C64, RO),
    col_enum(IF, "ifLinkUpDownTrapEnable", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 14], RO, ENABLED),
    col(IF, "ifHighSpeed", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 15], G32, RO),
    col_enum(IF, "ifPromiscuousMode", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 16], RO, TRUTH),
    col_enum(IF, "ifConnectorPresent", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 17], RO, TRUTH),
    col(IF, "ifAlias", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 18], DS, RW),
    col(IF, "ifCounterDiscontinuityTime", &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 19], TT, RO),
    node(SMI, "private", &[1, 3, 6, 1, 4]),
    node(SMI, "enterprises", &[1, 3, 6, 1, 4, 1]),
    node(SMI, "snmpV2", &[1, 3, 6, 1, 6]),
    node(SMI, "snmpModules", &[1, 3, 6, 1, 6, 3]),
    node(SNMP, "snmpMIB", &[1, 3, 6, 1, 6, 3, 1]),
    node(SNMP, "snmpMIBObjects", &[1, 3, 6, 1, 6, 3, 1, 1]),
    node(SNMP, "snmpTrap", &[1, 3, 6, 1, 6, 3, 1, 1, 4]),
    scalar(SNMP, "snmpTrapOID", &[1, 3, 6, 1, 6, 3, 1, 1, 4, 1], Syntax::Oid, Notify),
    scalar(SNMP, "snmpTrapEnterprise", &[1, 3, 6, 1, 6, 3, 1, 1, 4, 3], Syntax::Oid, Notify),
    node(SNMP, "snmpTraps", &[1, 3, 6, 1, 6, 3, 1, 1, 5]),
    notif(SNMP, "coldStart", &[1, 3, 6, 1, 6, 3, 1, 1, 5, 1]),
    notif(SNMP, "warmStart", &[1, 3, 6, 1, 6, 3, 1, 1, 5, 2]),
    notif(IF, "linkDown", &[1, 3, 6, 1, 6, 3, 1, 1, 5, 3]),
    notif(IF, "linkUp", &[1, 3, 6, 1, 6, 3, 1, 1, 5, 4]),
    notif(SNMP, "authenticationFailure", &[1, 3, 6, 1, 6, 3, 1, 1, 5, 5]),
];

/// an object by its OID exactly
pub fn find(oid: &[u32]) -> Option<&'static Obj> {
    OBJECTS.iter().find(|o| o.oid == oid)
}

/// an object by its name (case matters, as in the MIB modules)
pub fn by_name(name: &str) -> Option<&'static Obj> {
    OBJECTS.iter().find(|o| o.name == name)
}

/// the longest object the OID starts with, and the arcs after it
pub fn object_of(oid: &[u32]) -> Option<(&'static Obj, &[u32])> {
    let mut best: Option<&'static Obj> = None;
    for o in OBJECTS {
        if oid.starts_with(o.oid) && best.is_none_or(|b| o.oid.len() > b.oid.len()) {
            best = Some(o);
        }
    }
    best.map(|o| (o, &oid[o.oid.len()..]))
}

/// the scalar or column an OID is an instance of (sysName.0, ifDescr.2)
pub fn column_of(oid: &[u32]) -> Option<&'static Obj> {
    object_of(oid).map(|(o, _)| o).filter(|o| matches!(o.kind, ObjKind::Scalar | ObjKind::Column) && o.oid.len() < oid.len())
}

/// a name as net-snmp's tools write it: MODULE::name and the arcs after it; numeric: the arcs
/// with a leading dot
pub fn name_of(oid: &[u32], numeric: bool) -> String {
    if numeric || oid.is_empty() {
        let mut s = String::new();
        for a in oid {
            s.push_str(&format!(".{}", a));
        }
        return s;
    }
    match object_of(oid) {
        Some((o, rest)) => {
            let mut s = format!("{}::{}", o.module, o.name);
            for a in rest {
                s.push_str(&format!(".{}", a));
            }
            s
        }
        None => format!("{}", Oid::new(oid)),
    }
}

/// the name without the module ("sysName.0"; snmpget -Os)
pub fn short_name(oid: &[u32]) -> String {
    let s = name_of(oid, false);
    match s.split_once("::") {
        Some((_, r)) => String::from(r),
        None => s,
    }
}

/// an OID from what a user types: MODULE::name.arcs, name.arcs, dotted arcs (a leading dot
/// allowed); None for a name the table does not know or arcs that are no numbers
pub fn lookup(text: &str) -> Option<Oid> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let first = t.trim_start_matches('.');
    if first.starts_with(|c: char| c.is_ascii_digit()) {
        return Oid::parse(t);
    }
    let (module, rest) = match t.split_once("::") {
        Some((m, r)) => (Some(m), r),
        None => (None, t),
    };
    let (name, arcs) = match rest.split_once('.') {
        Some((n, a)) => (n, Some(a)),
        None => (rest, None),
    };
    let o = by_name(name).filter(|o| module.is_none_or(|m| m == o.module))?;
    let mut v = Vec::from(o.oid);
    if let Some(a) = arcs {
        for p in a.split('.') {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            v.push(p.parse::<u32>().ok()?);
        }
    }
    if v.len() > ber::OID_MAX {
        return None;
    }
    Some(Oid(v))
}

/// the objects one level below `oid` (the children of a node, the columns of an entry), in OID
/// order; an object whose parent the table lacks hangs below its nearest ancestor
pub fn children(oid: &[u32]) -> Vec<&'static Obj> {
    OBJECTS
        .iter()
        .filter(|o| o.oid.len() > oid.len() && o.oid.starts_with(oid))
        .filter(|o| {
            // nothing of the table lies between
            !OBJECTS.iter().any(|m| m.oid.len() > oid.len() && m.oid.len() < o.oid.len() && o.oid.starts_with(m.oid) && m.oid.starts_with(oid))
        })
        .collect()
}

/// "(500) 0:00:05.00", "(8640000) 1 day, 0:00:00.00" as net-snmp writes TimeTicks
pub fn ticks_text(t: u32) -> String {
    let cs = t % 100;
    let s = t / 100;
    let (d, h, m, sec) = (s / 86_400, (s / 3600) % 24, (s / 60) % 60, s % 60);
    let days = match d {
        0 => String::new(),
        1 => String::from("1 day, "),
        n => format!("{} days, ", n),
    };
    format!("({}) {}{}:{:02}:{:02}.{:02}", t, days, h, m, sec, cs)
}

/// octets as hex with spaces ("02 4A 00 ")
fn hex(b: &[u8]) -> String {
    let mut s = String::new();
    for x in b {
        s.push_str(&format!("{:02X} ", x));
    }
    s
}

/// a PhysAddress as RFC 2579 shows it ("2:4a:0:0:1:1")
fn phys(b: &[u8]) -> String {
    let v: Vec<String> = b.iter().map(|x| format!("{:x}", x)).collect();
    v.join(":")
}

/// the value after "=" as net-snmp's tools print it; the object named by `oid` gives the form
/// of strings and the names of integers; `numeric` writes OIDs as numbers
pub fn show(oid: &[u32], v: &Value, numeric: bool) -> String {
    let obj = column_of(oid);
    match v {
        Value::Int(i) => match obj.and_then(|o| o.enums.iter().find(|e| e.0 == *i)) {
            Some((_, n)) => format!("INTEGER: {}({})", n, i),
            None => format!("INTEGER: {}", i),
        },
        Value::Str(b) => match obj.map(|o| o.syntax) {
            Some(Syntax::DisplayString) => format!("STRING: {}", String::from_utf8_lossy(b)),
            Some(Syntax::PhysAddress) => format!("STRING: {}", phys(b)),
            _ => match ber::printable(b) {
                Some(t) if !b.is_empty() => format!("STRING: \"{}\"", t),
                _ if b.is_empty() => String::from("\"\""),
                _ => format!("Hex-STRING: {}", hex(b)),
            },
        },
        Value::Null => String::from("NULL"),
        Value::Oid(o) => format!("OID: {}", name_of(o.arcs(), numeric)),
        Value::Ip(a) => format!("IpAddress: {}", a),
        Value::Counter32(x) => format!("Counter32: {}", x),
        Value::Gauge32(x) => format!("Gauge32: {}", x),
        Value::TimeTicks(x) => format!("Timeticks: {}", ticks_text(*x)),
        Value::Opaque(b) => format!("Opaque: {}", hex(b)),
        Value::Counter64(x) => format!("Counter64: {}", x),
        Value::NoSuchObject => String::from("No Such Object (the agent has no object of this name)"),
        Value::NoSuchInstance => String::from("No Such Instance (the object has no such row or instance)"),
        Value::EndOfMibView => String::from("End of MIB View (nothing follows this name)"),
    }
}

/// the value alone (snmpget -Oqv): the name of an enumerated value, a text without quotes
pub fn bare(oid: &[u32], v: &Value, numeric: bool) -> String {
    let obj = column_of(oid);
    match v {
        Value::Int(i) => match obj.and_then(|o| o.enums.iter().find(|e| e.0 == *i)) {
            Some((_, n)) => String::from(*n),
            None => format!("{}", i),
        },
        Value::Str(b) => match obj.map(|o| o.syntax) {
            Some(Syntax::PhysAddress) => phys(b),
            Some(Syntax::DisplayString) => String::from_utf8_lossy(b).into_owned(),
            _ => ber::printable(b).map_or_else(|| hex(b), |t| format!("\"{}\"", t)),
        },
        Value::Oid(o) => name_of(o.arcs(), numeric),
        Value::TimeTicks(x) => {
            let t = ticks_text(*x);
            String::from(t.split_once(") ").map_or(t.as_str(), |(_, r)| r))
        }
        Value::Ip(a) => format!("{}", a),
        Value::Counter32(x) | Value::Gauge32(x) => format!("{}", x),
        Value::Counter64(x) => format!("{}", x),
        other => show(oid, other, numeric),
    }
}
