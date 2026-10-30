// snmpmib -- what an SNMP agent of the simulator serves: a view of a device's state made when a
// request asks (no copy of it is kept between requests), the order of its objects, Get and
// GetNext over it, and the interface rows linkDown and linkUp name
//
//   View::of(eng, ni, agent)    the device now: the system group (sysDescr from show version's
//                               words, sysObjectID, sysUpTime since the agent started, contact,
//                               name, location, services), the interfaces, the IPv4 addresses,
//                               the routes in use and the agent's counters
//   View::get(oid)              the value, or noSuchObject / noSuchInstance (RFC 3416 4.2.1)
//   View::next(oid, v1)         the first instance after the OID in the order of SNMP across
//                               every table (None: the end of the view); v1 skips Counter64
//                               (RFC 3584 4.2.2.1)
//   View::if_of_port(port)      the interface row of a port (linkDown, linkUp, ifAdminStatus)
//   object_id(model)            sysObjectID: 1.3.6.1.4.1.32473.1.N, the documentation enterprise
//                               number of RFC 5612 and one leaf per model (MODEL_LEAVES)
//   sys_services(model)         sysServices: 2 bridging, 4 routing, 72 for hosts (RFC 3418)
//
// Interfaces (IF-MIB): on IOS-like devices every port but the console and USB is a row, ifIndex
// 1, 2, ... in port order, then the other interfaces (subinterfaces, loopbacks, VLAN interfaces)
// after them in the order they were made (a removed one keeps its number unused); on Linux hosts
// every interface ip link shows, with its number there (lo is 1).  ifDescr is the full name,
// ifName the short one (Gi0/0/0), ifAlias the description; ifSpeed in bit/s up to 4294967295,
// ifHighSpeed in Mbit/s; the counters come from the ports (the 64-bit ones whole, the 32-bit
// ones modulo 2^32; unicast are the frames that are neither multicast nor broadcast); ifOutDiscards
// the frames a port dropped; interfaces without a port of their own count nothing.
// ifLastChange is the sysUpTime of the last change the agent saw (0 before it started).
// ipAddrTable has every address of every interface (also of one that is down); ipCidrRouteTable
// the routes the device uses now (the ones of show ip route without the local /32 ones: for each
// network those of the best distance and metric), indexed by network, mask, ToS 0 and next hop.

use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::{CliStyle, Media, Model, F_BRIDGE, F_HOST, F_ROUTER};
use jfn_model::ifname;
use netpkt::ber::{Oid, Value};
use netpkt::mib::{self, Obj, ObjKind};
use netpkt::{Ipv4, Mac};

use crate::ip::{IfKind, RouteSrc};
use crate::kernel::{Engine, Ns};
use crate::node::PortCounters;

/// the enterprise number for documentation (RFC 5612): sysObjectID lies below it
pub const ENTERPRISE: [u32; 7] = [1, 3, 6, 1, 4, 1, 32473];

/// the leaf of each model below 1.3.6.1.4.1.32473.1 (a new model gets the next free number)
pub const MODEL_LEAVES: [(&str, u32); 29] = [
    ("pc", 1),
    ("laptop", 2),
    ("smartphone", 3),
    ("tablet", 4),
    ("printer", 5),
    ("ip-phone", 6),
    ("iot-sensor", 7),
    ("iot-camera", 8),
    ("smart-tv", 9),
    ("jf-s24p", 10),
    ("jf-s24l3", 11),
    ("jf-s48x", 12),
    ("jf-r4", 13),
    ("jf-r8", 14),
    ("home-router", 15),
    ("hub", 16),
    ("jf-fw", 17),
    ("jf-ap6e", 18),
    ("modem-ont", 19),
    ("cloud", 20),
    ("server-1u", 21),
    ("server-1u-sfp", 22),
    ("server-tower", 23),
    ("load-balancer", 24),
    ("database", 25),
    ("cache", 26),
    ("queue", 27),
    ("traffic-gen", 28),
    ("container-host", 29),
];

/// sysObjectID of a model (an unknown one: .1.0)
pub fn object_id(m: &Model) -> Oid {
    let leaf = MODEL_LEAVES.iter().find(|x| x.0 == m.id).map_or(0, |x| x.1);
    Oid::new(&ENTERPRISE).child(&[1, leaf])
}

/// sysServices (RFC 3418): 2 for bridging, 4 for routing (an L3 switch both), 72 (end-to-end and
/// applications) for hosts
pub fn sys_services(m: &Model) -> i64 {
    if m.has(F_HOST) {
        return 72;
    }
    (if m.has(F_BRIDGE) { 2 } else { 0 }) + (if m.has(F_ROUTER) { 4 } else { 0 })
}

/// sysDescr: the first line of show version on IOS-like devices, our own words for hosts
pub fn sys_descr(m: &Model) -> String {
    match m.cli {
        CliStyle::Ios => alloc::format!("JFNetworker device software 1.0 for the {} ({})", m.name, m.title),
        _ => alloc::format!("JFNetworker host system 1.0 (Linux-like) for the {} ({})", m.name, m.title),
    }
}

/// what an interface row holds
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfRow {
    pub index: u32,
    pub descr: String,
    pub name: String,
    pub alias: String,
    pub if_type: i64,
    pub mtu: i64,
    /// bit/s
    pub speed: u64,
    pub mac: Option<Mac>,
    pub admin_up: bool,
    pub oper_up: bool,
    pub port: Option<u16>,
    pub iface: Option<u16>,
    pub counters: PortCounters,
    /// sysUpTime of its last change (TimeTicks)
    pub last_change: u32,
}

/// a row of ipAddrTable
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddrRow {
    pub addr: Ipv4,
    pub mask: Ipv4,
    pub if_index: u32,
}

/// a row of ipCidrRouteTable
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteRow {
    pub dest: Ipv4,
    pub mask: Ipv4,
    pub next_hop: Ipv4,
    pub if_index: u32,
    /// local(3) or remote(4)
    pub kind: i64,
    /// local(2), netmgmt(3), rip(8), ospf(13), bgp(14)
    pub proto: i64,
    pub metric: i64,
}

/// the agent's numbers (the snmp group, RFC 3418 and RFC 1213), by the last arc of their OID
pub type Counters = [u32; 33];

/// the device as an agent serves it, made for one request
#[derive(Clone, Debug)]
pub struct View {
    pub descr: String,
    pub object_id: Oid,
    pub uptime: u32,
    pub contact: String,
    pub name: String,
    pub location: String,
    pub services: i64,
    pub ifs: Vec<IfRow>,
    pub addrs: Vec<AddrRow>,
    pub routes: Vec<RouteRow>,
    pub forwarding: bool,
    pub ttl: u8,
    pub reasm: i64,
    pub counters: Counters,
    pub auth_traps: bool,
}

/// what an agent adds to the device's state (snmpd.rs)
pub struct AgentPart<'a> {
    pub started: Ns,
    pub contact: &'a str,
    pub location: &'a str,
    pub counters: &'a Counters,
    pub auth_traps: bool,
    /// (port, virtual time of its last change)
    pub changes: &'a [(u16, Ns)],
}

/// TimeTicks between two moments
pub fn ticks(from: Ns, to: Ns) -> u32 {
    (to.saturating_sub(from) / 10_000_000).min(u32::MAX as u64) as u32
}

fn wrap32(x: u64) -> u32 {
    (x & 0xFFFF_FFFF) as u32
}

impl View {
    /// node `ni` now
    pub fn of(e: &Engine, ni: usize, a: &AgentPart) -> View {
        let n = e.nodes.get(ni).and_then(|x| x.as_ref());
        let now = e.now;
        let (descr, object_id, services, name) = match n {
            Some(n) => (sys_descr(n.model), object_id(n.model), sys_services(n.model), n.name.clone()),
            None => (String::new(), Oid::new(&ENTERPRISE), 0, String::new()),
        };
        let mut v = View {
            descr,
            object_id,
            uptime: ticks(a.started, now),
            contact: String::from(a.contact),
            name,
            location: String::from(a.location),
            services,
            ifs: Vec::new(),
            addrs: Vec::new(),
            routes: Vec::new(),
            forwarding: false,
            ttl: 64,
            reasm: 65535,
            counters: *a.counters,
            auth_traps: a.auth_traps,
        };
        let Some(n) = n else {
            return v;
        };
        v.ifs = interfaces(e, ni, a);
        if let Some(st) = n.ip.as_ref() {
            v.forwarding = st.forwarding;
            v.ttl = st.ttl;
            let ios = n.model.cli == CliStyle::Ios;
            v.reasm = if ios { 18024 } else { 65535 };
            for (i, f) in st.ifaces.iter().enumerate() {
                if f.hidden {
                    continue;
                }
                let Some(row) = v.ifs.iter().find(|r| r.iface == Some(i as u16)) else { continue };
                for p in &f.addrs {
                    v.addrs.push(AddrRow { addr: p.addr, mask: p.mask(), if_index: row.index });
                }
            }
            v.addrs.sort_by_key(|r| r.addr.to_u32());
            v.addrs.dedup_by_key(|r| r.addr);
            v.routes = routes(e, ni, &v.ifs);
        }
        v
    }

    /// the row of a port
    pub fn if_of_port(&self, port: u16) -> Option<&IfRow> {
        self.ifs.iter().find(|r| r.port == Some(port))
    }

    /// the row of an ifIndex
    pub fn if_row(&self, index: u32) -> Option<&IfRow> {
        self.ifs.iter().find(|r| r.index == index)
    }

    /// the instance arcs of the rows of the table a column belongs to, in order
    fn rows(&self, col: &Obj) -> Vec<Vec<u32>> {
        let o = col.oid;
        if o.starts_with(&[1, 3, 6, 1, 2, 1, 2, 2, 1]) || o.starts_with(&[1, 3, 6, 1, 2, 1, 31, 1, 1, 1]) {
            return self.ifs.iter().map(|r| alloc::vec![r.index]).collect();
        }
        if o.starts_with(&[1, 3, 6, 1, 2, 1, 4, 20, 1]) {
            return self.addrs.iter().map(|r| Vec::from(&r.addr.0[..]).into_iter().map(|x| x as u32).collect()).collect();
        }
        if o.starts_with(&[1, 3, 6, 1, 2, 1, 4, 24, 4, 1]) {
            return self.routes.iter().map(route_index).collect();
        }
        Vec::new()
    }

    /// a scalar's value
    fn scalar(&self, o: &Obj) -> Option<Value> {
        let a = o.oid;
        Some(match a {
            [1, 3, 6, 1, 2, 1, 1, x] => match x {
                1 => Value::text(&self.descr),
                2 => Value::Oid(self.object_id.clone()),
                3 => Value::TimeTicks(self.uptime),
                4 => Value::text(&self.contact),
                5 => Value::text(&self.name),
                6 => Value::text(&self.location),
                7 => Value::Int(self.services),
                _ => return None,
            },
            [1, 3, 6, 1, 2, 1, 2, 1] => Value::Int(self.ifs.len() as i64),
            [1, 3, 6, 1, 2, 1, 4, 1] => Value::Int(if self.forwarding { 1 } else { 2 }),
            [1, 3, 6, 1, 2, 1, 4, 2] => Value::Int(self.ttl as i64),
            [1, 3, 6, 1, 2, 1, 4, 24, 3] => Value::Gauge32(self.routes.len() as u32),
            [1, 3, 6, 1, 2, 1, 11, 30] => Value::Int(if self.auth_traps { 1 } else { 2 }),
            [1, 3, 6, 1, 2, 1, 11, x] => Value::Counter32(*self.counters.get(*x as usize)?),
            _ => return None,
        })
    }

    /// a column's value in row `r` (its position in rows())
    fn cell(&self, o: &Obj, r: usize) -> Option<Value> {
        let a = o.oid;
        let col = *a.last()?;
        if a.starts_with(&[1, 3, 6, 1, 2, 1, 2, 2, 1]) {
            let f = self.ifs.get(r)?;
            let c = &f.counters;
            let nu_in = c.rx_mcast + c.rx_bcast;
            let nu_out = c.tx_mcast + c.tx_bcast;
            return Some(match col {
                1 => Value::Int(f.index as i64),
                2 => Value::text(&f.descr),
                3 => Value::Int(f.if_type),
                4 => Value::Int(f.mtu),
                5 => Value::Gauge32(f.speed.min(u32::MAX as u64) as u32),
                6 => Value::Str(f.mac.map(|m| Vec::from(&m.0[..])).unwrap_or_default()),
                7 => Value::Int(if f.admin_up { 1 } else { 2 }),
                8 => Value::Int(if f.oper_up { 1 } else { 2 }),
                9 => Value::TimeTicks(f.last_change),
                10 => Value::Counter32(wrap32(c.rx_bytes)),
                11 => Value::Counter32(wrap32(c.rx_frames.saturating_sub(nu_in))),
                12 => Value::Counter32(wrap32(nu_in)),
                16 => Value::Counter32(wrap32(c.tx_bytes)),
                17 => Value::Counter32(wrap32(c.tx_frames.saturating_sub(nu_out))),
                18 => Value::Counter32(wrap32(nu_out)),
                19 => Value::Counter32(wrap32(c.drops)),
                13 | 14 | 15 | 20 => Value::Counter32(0),
                21 => Value::Gauge32(0),
                22 => Value::Oid(Oid::new(&[0, 0])),
                _ => return None,
            });
        }
        if a.starts_with(&[1, 3, 6, 1, 2, 1, 31, 1, 1, 1]) {
            let f = self.ifs.get(r)?;
            let c = &f.counters;
            let real = f.port.is_some();
            return Some(match col {
                1 => Value::text(&f.name),
                2 => Value::Counter32(wrap32(c.rx_mcast)),
                3 => Value::Counter32(wrap32(c.rx_bcast)),
                4 => Value::Counter32(wrap32(c.tx_mcast)),
                5 => Value::Counter32(wrap32(c.tx_bcast)),
                6 => Value::Counter64(c.rx_bytes),
                7 => Value::Counter64(c.rx_frames.saturating_sub(c.rx_mcast + c.rx_bcast)),
                8 => Value::Counter64(c.rx_mcast),
                9 => Value::Counter64(c.rx_bcast),
                10 => Value::Counter64(c.tx_bytes),
                11 => Value::Counter64(c.tx_frames.saturating_sub(c.tx_mcast + c.tx_bcast)),
                12 => Value::Counter64(c.tx_mcast),
                13 => Value::Counter64(c.tx_bcast),
                14 => Value::Int(if real { 1 } else { 2 }),
                15 => Value::Gauge32((f.speed / 1_000_000).min(u32::MAX as u64) as u32),
                16 => Value::Int(2),
                17 => Value::Int(if real { 1 } else { 2 }),
                18 => Value::text(&f.alias),
                19 => Value::TimeTicks(0),
                _ => return None,
            });
        }
        if a.starts_with(&[1, 3, 6, 1, 2, 1, 4, 20, 1]) {
            let x = self.addrs.get(r)?;
            return Some(match col {
                1 => Value::Ip(x.addr),
                2 => Value::Int(x.if_index as i64),
                3 => Value::Ip(x.mask),
                4 => Value::Int(1),
                5 => Value::Int(self.reasm),
                _ => return None,
            });
        }
        if a.starts_with(&[1, 3, 6, 1, 2, 1, 4, 24, 4, 1]) {
            let x = self.routes.get(r)?;
            return Some(match col {
                1 => Value::Ip(x.dest),
                2 => Value::Ip(x.mask),
                3 => Value::Int(0),
                4 => Value::Ip(x.next_hop),
                5 => Value::Int(x.if_index as i64),
                6 => Value::Int(x.kind),
                7 => Value::Int(x.proto),
                8 => Value::Int(0),
                9 => Value::Oid(Oid::new(&[0, 0])),
                10 => Value::Int(0),
                11 => Value::Int(x.metric),
                12..=15 => Value::Int(-1),
                16 => Value::Int(1),
                _ => return None,
            });
        }
        None
    }

    /// the value of an instance: noSuchObject when no object the agent serves has the OID's
    /// prefix, noSuchInstance when the object has no such instance
    pub fn get(&self, oid: &[u32]) -> Value {
        let Some((o, rest)) = mib::object_of(oid) else {
            return Value::NoSuchObject;
        };
        if !served(o) {
            return Value::NoSuchObject;
        }
        match o.kind {
            ObjKind::Scalar => {
                if rest == [0] {
                    self.scalar(o).unwrap_or(Value::NoSuchInstance)
                } else {
                    Value::NoSuchInstance
                }
            }
            _ => match self.rows(o).iter().position(|r| r.as_slice() == rest) {
                Some(i) => self.cell(o, i).unwrap_or(Value::NoSuchInstance),
                None => Value::NoSuchInstance,
            },
        }
    }

    /// the first instance after `oid` (v1: none of type Counter64)
    pub fn next(&self, oid: &[u32], v1: bool) -> Option<(Oid, Value)> {
        for o in mib::OBJECTS.iter().filter(|o| served(o)) {
            // everything of this object lies before the OID
            if o.oid < oid && !oid.starts_with(o.oid) {
                continue;
            }
            let ok = |v: &Value| !(v1 && matches!(v, Value::Counter64(_)));
            match o.kind {
                ObjKind::Scalar => {
                    let inst = Oid::new(o.oid).child(&[0]);
                    if inst.arcs() > oid
                        && let Some(v) = self.scalar(o)
                        && ok(&v)
                    {
                        return Some((inst, v));
                    }
                }
                _ => {
                    for (i, idx) in self.rows(o).iter().enumerate() {
                        let inst = Oid::new(o.oid).child(idx);
                        if inst.arcs() > oid
                            && let Some(v) = self.cell(o, i)
                            && ok(&v)
                        {
                            return Some((inst, v));
                        }
                    }
                }
            }
        }
        None
    }
}

/// the objects with instances: the scalars and columns of the MIB but those only notifications
/// carry
fn served(o: &Obj) -> bool {
    matches!(o.kind, ObjKind::Scalar | ObjKind::Column) && o.access != mib::Access::Notify
}

/// dest.mask.tos.next_hop
fn route_index(r: &RouteRow) -> Vec<u32> {
    let mut v: Vec<u32> = Vec::with_capacity(13);
    v.extend(r.dest.0.iter().map(|&x| x as u32));
    v.extend(r.mask.0.iter().map(|&x| x as u32));
    v.push(0);
    v.extend(r.next_hop.0.iter().map(|&x| x as u32));
    v
}

/// the interface rows of node `ni`
fn interfaces(e: &Engine, ni: usize, a: &AgentPart) -> Vec<IfRow> {
    let Some(Some(n)) = e.nodes.get(ni) else {
        return Vec::new();
    };
    let st = n.ip.as_ref();
    let descr = |name: &str| n.cli.cfg.descr.get(name).cloned().unwrap_or_default();
    let changed = |p: u16| a.changes.iter().find(|c| c.0 == p).map_or(0, |c| ticks(a.started, c.1));
    let port_up = |p: u16| {
        let port = &n.ports[p as usize];
        n.power && port.admin_up && !port.err_disabled && (port.link.is_some_and(|li| e.link_up(li)) || (port.spec.media == Media::RadioAp && e.wlan_radio_up(ni, p)))
    };
    let port_speed = |p: u16| {
        let port = &n.ports[p as usize];
        let k = port.link.filter(|&li| e.link_up(li)).map_or(port.spec.kbps, |li| e.link_kbps(li));
        k as u64 * 1000
    };
    let port_type = |m: Media| match m {
        Media::Serial => 22,
        Media::RadioAp | Media::RadioClient => 71,
        Media::Wan => 1,
        _ => 6,
    };
    let mut v: Vec<IfRow> = Vec::new();
    if n.model.cli == CliStyle::Ios {
        let mut k = 0u32;
        for (pi, p) in n.ports.iter().enumerate() {
            if matches!(p.spec.media, Media::Console | Media::Usb) {
                continue;
            }
            k += 1;
            let pi = pi as u16;
            let iface = st.and_then(|s| s.iface_of_port(pi)).filter(|&f| st.is_some_and(|s| !s.ifaces[f as usize].hidden));
            let mtu = iface.and_then(|f| st.map(|s| s.ifaces[f as usize].mtu as i64)).unwrap_or(1500);
            v.push(IfRow {
                index: k,
                descr: p.spec.name.clone(),
                name: ifname::short(&p.spec.name),
                alias: descr(&p.spec.name),
                if_type: port_type(p.spec.media),
                mtu,
                speed: port_speed(pi),
                mac: Some(p.mac),
                admin_up: p.admin_up,
                oper_up: port_up(pi),
                port: Some(pi),
                iface,
                counters: p.counters,
                last_change: changed(pi),
            });
        }
        let base = k;
        if let Some(s) = st {
            let mut j = 0u32;
            for (i, f) in s.ifaces.iter().enumerate() {
                if matches!(f.kind, IfKind::Port(_)) {
                    continue;
                }
                j += 1;
                if f.hidden {
                    continue;
                }
                let i = i as u16;
                let (if_type, speed, counters) = match f.kind {
                    IfKind::Loopback(_) => (24, 0, PortCounters::default()),
                    IfKind::Svi(_) => (53, 1_000_000_000, PortCounters::default()),
                    IfKind::Sub { port, .. } => (135, port_speed(port), PortCounters::default()),
                    IfKind::Port(_) => (6, 0, PortCounters::default()),
                };
                v.push(IfRow {
                    index: base + j,
                    descr: f.name.clone(),
                    name: ifname::short(&f.name),
                    alias: descr(&f.name),
                    if_type,
                    mtu: f.mtu as i64,
                    speed,
                    mac: if f.is_loopback() { None } else { Some(f.mac) },
                    admin_up: f.admin_up,
                    oper_up: e.iface_up(ni, i),
                    port: None,
                    iface: Some(i),
                    counters,
                    last_change: 0,
                });
            }
        }
        return v;
    }
    let Some(s) = st else {
        return v;
    };
    for (i, f) in s.ifaces.iter().enumerate() {
        if f.hidden {
            continue;
        }
        let iu = i as u16;
        let (port, admin) = match f.kind {
            IfKind::Port(p) => (Some(p), n.ports[p as usize].admin_up),
            _ => (None, f.admin_up),
        };
        let (if_type, speed, counters, mtu) = match f.kind {
            IfKind::Loopback(0) if f.name == "lo" => (24, 10_000_000, PortCounters::default(), 65536),
            IfKind::Loopback(_) => (6, 0, PortCounters::default(), f.mtu as i64),
            IfKind::Port(p) => (port_type(n.ports[p as usize].spec.media), port_speed(p), n.ports[p as usize].counters, f.mtu as i64),
            IfKind::Sub { port: p, .. } => (135, port_speed(p), PortCounters::default(), f.mtu as i64),
            IfKind::Svi(_) => (6, 1_000_000_000, PortCounters::default(), f.mtu as i64),
        };
        v.push(IfRow {
            index: i as u32 + 1,
            descr: f.name.clone(),
            name: f.name.clone(),
            alias: descr(&f.name),
            if_type,
            mtu,
            speed,
            mac: if f.is_loopback() && f.name == "lo" { None } else { Some(f.mac) },
            admin_up: admin,
            oper_up: e.iface_up(ni, iu),
            port,
            iface: Some(iu),
            counters,
            last_change: port.map_or(0, changed),
        });
    }
    v
}

/// the routes in use: for each network those of the best distance and metric (no local /32s),
/// in the order of their index
fn routes(e: &Engine, ni: usize, ifs: &[IfRow]) -> Vec<RouteRow> {
    let dev = e.dev_of(ni);
    let all = e.routes(dev);
    let mut out: Vec<RouteRow> = Vec::new();
    for r in all.iter().filter(|r| r.src != RouteSrc::Local) {
        let best = all.iter().filter(|x| x.prefix == r.prefix && x.src != RouteSrc::Local).map(|x| (x.ad, x.metric)).min();
        if best != Some((r.ad, r.metric)) {
            continue;
        }
        let if_index = r.iface.and_then(|i| ifs.iter().find(|f| f.iface == Some(i))).map_or(0, |f| f.index);
        let proto = match r.src {
            RouteSrc::Connected | RouteSrc::Local => 2,
            RouteSrc::Static => 3,
            RouteSrc::Rip => 8,
            RouteSrc::Ospf => 13,
            RouteSrc::Bgp => 14,
        };
        out.push(RouteRow {
            dest: r.prefix.addr,
            mask: r.prefix.mask(),
            next_hop: r.via.unwrap_or(Ipv4::UNSPECIFIED),
            if_index,
            kind: if r.via.is_some() { 4 } else { 3 },
            proto,
            metric: r.metric as i64,
        });
    }
    out.sort_by(|a, b| route_index(a).cmp(&route_index(b)));
    out.dedup_by(|a, b| route_index(a) == route_index(b));
    out
}
