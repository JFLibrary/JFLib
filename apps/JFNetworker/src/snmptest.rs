// snmptest -- checks of SNMP (group snmp of --selftest: jfn_sim::snmpd, snmpmib, snmptrapd,
// snmptrace, cli/snmp.rs): the system group by Get on the nanosecond, walks over ifTable and
// ifXTable whose counters are the ports' bytes and frames up to the request, the lexicographic
// order across tables, GetBulk with non-repeaters, repetitions and the size limit, Set of
// sysLocation, sysName and ifAdminStatus (the port goes down, the running configuration shows it,
// the project only after copy run start), what a read-only community may not, a wrong
// community and an access list (no answer, the counter, the console line, authenticationFailure),
// the errors of version 1 against those of 2c (RFC 3584), tooBig, linkDown and linkUp traps in
// snmptrapd with its file, an inform sent again over a lossy cable until it is acknowledged,
// coldStart of a Linux agent at boot, the Linux agent's settings and refusals, the IOS-like
// commands with their running configuration read back, show snmp, debug snmp packets, the
// problems, the dissection of a recorded message and determinism
//
// The LAN (device ids in order): R1 (JF-R4, Gi0/0/0 10.0.0.1/24), SW1 (Vlan1 10.0.0.2/24), MON
// (10.0.0.50, snmptrapd), PC1 (10.0.0.20, snmpd), PC2 on R1's Gi0/0/1 (shut down as a router
// boots: the port for link changes of R1); cables 6 R1-SW1, 7 MON-SW1, 8 PC1-SW1, 9 R1-PC2.
// 1 Gbit/s copper with 50 ns of cable: a frame of n bytes takes (max(n, 60) + 24) * 8 + 50 ns
// per cable, twice that over SW1.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use jfn_model::project::{Device, PortRef, Project, ServiceDef};
use jfn_sim::os::{OsApi, Service, SvcEvent};
use jfn_sim::{Action, Engine, Metric, Note, ProblemKind, Report, SNote, MS, SEC};
use netpkt::ber::{Oid, Value};
use netpkt::decode::{self, Proto, P_SNMP};
use netpkt::snmp::{self, Kind, Message, Pdu, VarBind, Version};
use netpkt::{IpAddr, Ipv4};

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const R1: u32 = 1;
const SW1: u32 = 2;
const MON: u32 = 3;
const PC1: u32 = 4;
const PC2: u32 = 5;
const L_MON: u32 = 7;
const L_PC1: u32 = 8;
const L_PC2: u32 = 9;

fn s(x: &str) -> String {
    String::from(x)
}

fn oid(x: &str) -> Oid {
    Oid::parse(x).unwrap_or_default()
}

fn ip(a: [u8; 4]) -> IpAddr {
    IpAddr::V4(Ipv4(a))
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Json::Null) }
}

const R1_SNMP: &str = "access-list 10 permit host 10.0.0.50\n!\nsnmp-server community public RO\nsnmp-server community private RW 10\nsnmp-server location Technikraum\nsnmp-server contact netz@lab.example\nsnmp-server enable traps snmp authentication linkdown linkup coldstart\nsnmp-server host 10.0.0.50 version 2c public\n!\n";
const PC1_SNMPD: &str = r#"{"communities": [{"name": "public"}, {"name": "geheim", "access": "rw", "source": "10.0.0.50/32"}], "location": "Büro 1", "contact": "anna", "sinks": [{"host": "10.0.0.50"}], "notify": ["coldstart", "linkdown", "linkup"]}"#;

/// the LAN of the header comment; `r1` the SNMP lines of R1's startup configuration
fn project(r1: &str) -> Project {
    let mut p = Project::new("snmp", 17);
    for (i, m) in ["jf-r4", "jf-s24p", "server-1u", "pc", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(MON, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let _ = p.connect(&pr(R1, "gi0/0/1"), &pr(PC2, "eth0"), None);
    let r1 = format!("hostname R1\n!\ninterface GigabitEthernet0/0/0\n description zum LAN\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\n{}end\n", r1);
    let cfg = [
        (R1, "R1", r1.as_str()),
        (SW1, "SW1", "hostname SW1\n!\ninterface Vlan1\n ip address 10.0.0.2 255.255.255.0\n no shutdown\n!\nsnmp-server community public RO\nsnmp-server enable traps snmp linkdown linkup\nsnmp-server host 10.0.0.50 version 2c public\n!\nend\n"),
        (MON, "MON", "hostname MON\nip addr add 10.0.0.50/24 dev eth0\n"),
        (PC1, "PC1", "hostname PC1\nip addr add 10.0.0.20/24 dev eth0\n"),
        (PC2, "PC2", "hostname PC2\n"),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = s(c);
        }
    }
    if let Some(d) = p.device_mut(MON) {
        d.services = alloc::vec![def("snmptrapd", "traps", r#"{"communities": ["public"]}"#)];
    }
    if let Some(d) = p.device_mut(PC1) {
        d.services = alloc::vec![def("snmpd", "snmpd", PC1_SNMPD)];
    }
    p
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// lines typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    out(e, dev)
}

/// the SNMP messages after `from`, one per IP packet: (sent on its first cable, arrived over
/// its last, the device that sent it, the message)
fn snmp_hops(e: &Engine, from: u64) -> Vec<(u64, u64, u32, Message)> {
    let mut v: Vec<((Ipv4, u16), u64, u64, u32, Message)> = Vec::new();
    for h in e.trace.hops().filter(|h| h.mask & P_SNMP != 0 && h.drop.is_none() && h.t_tx >= from) {
        let st = decode::stack(&h.bytes);
        let (Some((io, _)), Some((o, l))) = (st.find(Proto::Ipv4), st.find(Proto::Snmp)) else { continue };
        let Some(p) = netpkt::ipv4::Ipv4View::parse(&h.bytes[io..]) else { continue };
        let key = (p.src(), p.id());
        match v.iter_mut().find(|x| x.0 == key) {
            Some(x) => x.2 = h.t_rx,
            None => {
                if let Ok(m) = Message::parse(&h.bytes[o..o + l]) {
                    v.push((key, h.t_tx, h.t_rx, h.from.0, m));
                }
            }
        }
    }
    v.into_iter().map(|x| (x.1, x.2, x.3, x.4)).collect()
}

/// the SNMP notes of a device
fn notes(e: &Engine, dev: u32) -> Vec<(u64, SNote)> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Snmp(n) = &x.note { Some((x.t, n.clone())) } else { None }).collect()
}

/// a manager for the tests: it sends the messages at their times from one port and keeps what
/// comes back
#[derive(Clone, Debug)]
struct Raw {
    to: IpAddr,
    plan: Vec<(u64, Vec<u8>)>,
    port: u16,
    next: usize,
    got: Vec<(u64, Message)>,
}

impl Raw {
    fn new(to: [u8; 4], plan: Vec<(u64, Vec<u8>)>) -> Raw {
        Raw { to: ip(to), plan, port: 0, next: 0, got: Vec::new() }
    }
    fn arm(&mut self, os: &mut OsApi) {
        if let Some((at, _)) = self.plan.get(self.next) {
            os.timer(0, *at);
        }
    }
}

impl Service for Raw {
    fn kind(&self) -> &'static str {
        "raw-snmp"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.port = os.bind(0, None).unwrap_or(0);
        self.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Timer(0) => {
                while let Some((at, b)) = self.plan.get(self.next).cloned() {
                    if at > os.now() {
                        break;
                    }
                    os.send(self.port, self.to, snmp::PORT, &b);
                    self.next += 1;
                }
                self.arm(os);
            }
            SvcEvent::Udp(d) => {
                if let Ok(m) = Message::parse(&d.data) {
                    self.got.push((os.now(), m));
                }
            }
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::new()
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

/// a request of version `v` with community `c`
fn req(v: Version, c: &str, kind: Kind, id: i32, oids: &[&str]) -> Vec<u8> {
    Message::new(v, c, Pdu::new(kind, id, oids.iter().map(|o| VarBind::null(oid(o))).collect())).to_bytes()
}

/// a raw manager on `dev` with these messages to `to` (times from now), run until 2 s after the
/// last one: what came back
fn ask_from(e: &mut Engine, dev: u32, to: [u8; 4], plan: Vec<(u64, Vec<u8>)>) -> Vec<(u64, Message)> {
    let _ = e.remove_service(dev, "raw");
    let base = e.now();
    let last = plan.iter().map(|p| p.0).max().unwrap_or(0);
    let plan = plan.into_iter().map(|(t, b)| (base + t, b)).collect();
    let _ = e.add_service(dev, "raw", Box::new(Raw::new(to, plan)), true);
    run_for(e, last + 2 * SEC);
    e.service::<Raw>(dev, "raw").map(|r| r.got.clone()).unwrap_or_default()
}

fn ask(e: &mut Engine, to: [u8; 4], plan: Vec<(u64, Vec<u8>)>) -> Vec<(u64, Message)> {
    ask_from(e, MON, to, plan)
}

/// the varbinds of a message as (OID, value)
fn vbs(m: &Message) -> Vec<(String, Value)> {
    m.varbinds().iter().map(|v| (format!("{}", v.oid), v.value.clone())).collect()
}

/// the answer with this request id
fn by_id(a: &[(u64, Message)], id: i32) -> Option<Message> {
    a.iter().find(|x| x.1.pdu().is_some_and(|p| p.request_id == id)).map(|x| x.1.clone())
}

pub(crate) fn run(t: &mut T) {
    system(t);
    walks(t);
    order(t);
    bulk(t);
    sets(t);
    refused(t);
    versions(t);
    traps(t);
    informs(t);
    linux(t);
    ios(t);
    problems(t);
    dissect(t);
    determinism(t);
}

// ---------------------------------------------------------------- the system group

fn system(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let o = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.1 sysDescr.0 sysObjectID.0 sysUpTime.0 sysContact.0 sysName.0 sysLocation.0 sysServices.0"], SEC);
    t.eq(
        "the system group",
        o,
        s("SNMPv2-MIB::sysDescr.0 = STRING: JFNetworker device software 1.0 for the JF-R4 (Edge-Router 4× 1G, 2× 10G SFP+, 2× Seriell)\nSNMPv2-MIB::sysObjectID.0 = OID: SNMPv2-SMI::enterprises.32473.1.13\nSNMPv2-MIB::sysUpTime.0 = Timeticks: (100) 0:00:01.00\nSNMPv2-MIB::sysContact.0 = STRING: netz@lab.example\nSNMPv2-MIB::sysName.0 = STRING: R1\nSNMPv2-MIB::sysLocation.0 = STRING: Technikraum\nSNMPv2-MIB::sysServices.0 = INTEGER: 4\n"),
    );
    // MON knows R1's MAC already (R1 asked for MON's at boot, for its coldStart): the request
    // leaves at once, the answer too
    let h = snmp_hops(&e, SEC);
    t.eq("request and response", h.iter().map(|x| (x.2, x.3.kind())).collect::<Vec<_>>(), alloc::vec![(MON, Kind::Get), (R1, Kind::Response)]);
    if let [(tq, rq, _, q), (ta, ra, _, a)] = &h[..] {
        let wire = |m: &Message| ((m.to_bytes().len() + 42).max(60) as u64 + 24) * 8 + 50;
        t.eq("the request leaves at once", *tq, SEC);
        t.eq("its way over SW1", *rq - *tq, 2 * wire(q));
        t.eq("the answer at once", *ta, *rq);
        t.eq("the answer's way", *ra - *ta, 2 * wire(a));
        t.eq("same request id", a.pdu().map(|p| p.request_id), q.pdu().map(|p| p.request_id));
    } else {
        t.check("two messages", false);
    }
    let v = e.snmp_view(R1);
    t.check("sysObjectID of every model", jfn_model::catalog::catalog().iter().filter(|m| m.id != "note").all(|m| jfn_sim::snmpmib::MODEL_LEAVES.iter().any(|l| l.0 == m.id)));
    let mut leaves: Vec<u32> = jfn_sim::snmpmib::MODEL_LEAVES.iter().map(|l| l.1).collect();
    leaves.sort();
    leaves.dedup();
    t.eq("leaves unique", leaves.len(), jfn_sim::snmpmib::MODEL_LEAVES.len());
    t.eq("sysServices", ["jf-r4", "jf-s24p", "jf-s24l3", "pc"].iter().map(|m| jfn_model::catalog::model(m).map(jfn_sim::snmpmib::sys_services).unwrap_or(0)).collect::<Vec<_>>(), alloc::vec![4, 2, 6, 72]);
    t.check("the view", v.is_some_and(|v| v.name == "R1" && v.ifs.len() == 8 && v.addrs.len() == 1));
    t.check("no agent no view", e.snmp_view(MON).is_none());
}

// ---------------------------------------------------------------- walks and counters

fn walks(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["ping -c 3 10.0.0.1"], 4 * SEC);
    let o = cli(&mut e, MON, &["snmpwalk -v2c -c public 10.0.0.1 ifDescr"], 2 * SEC);
    t.eq("ifDescr", o, s("IF-MIB::ifDescr.1 = STRING: GigabitEthernet0/0/0\nIF-MIB::ifDescr.2 = STRING: GigabitEthernet0/0/1\nIF-MIB::ifDescr.3 = STRING: GigabitEthernet0/0/2\nIF-MIB::ifDescr.4 = STRING: GigabitEthernet0/0/3\nIF-MIB::ifDescr.5 = STRING: TenGigabitEthernet0/1/0\nIF-MIB::ifDescr.6 = STRING: TenGigabitEthernet0/1/1\nIF-MIB::ifDescr.7 = STRING: Serial0/2/0\nIF-MIB::ifDescr.8 = STRING: Serial0/2/1\n"));
    let o = cli(&mut e, MON, &["snmpwalk -v2c -c public -Oq 10.0.0.1 ifOperStatus"], 2 * SEC);
    t.check(&format!("only Gi0/0/0 up: {}", o), o.starts_with("IF-MIB::ifOperStatus.1 up\nIF-MIB::ifOperStatus.2 down\n") && o.matches(" down\n").count() == 7);
    let o = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.1 ifAlias.1 ifName.1 ifType.1 ifSpeed.1 ifHighSpeed.1 ifPhysAddress.1 ifMtu.1 ifType.7"], SEC);
    t.eq("a row", o, s("IF-MIB::ifAlias.1 = STRING: zum LAN\nIF-MIB::ifName.1 = STRING: Gi0/0/0\nIF-MIB::ifType.1 = INTEGER: ethernetCsmacd(6)\nIF-MIB::ifSpeed.1 = Gauge32: 1000000000\nIF-MIB::ifHighSpeed.1 = Gauge32: 1000\nIF-MIB::ifPhysAddress.1 = STRING: 2:4a:0:0:1:1\nIF-MIB::ifMtu.1 = INTEGER: 1500\nIF-MIB::ifType.7 = INTEGER: propPointToPointSerial(22)\n"));
    // the counters of Gi0/0/0 are the bytes and frames of the port up to the request (its own
    // frame in, the answer not yet out)
    let at = e.now();
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, req(Version::V2c, "public", Kind::Get, 7, &["1.3.6.1.2.1.31.1.1.1.6.1", "1.3.6.1.2.1.31.1.1.1.10.1", "1.3.6.1.2.1.31.1.1.1.7.1", "1.3.6.1.2.1.31.1.1.1.9.1", "1.3.6.1.2.1.2.2.1.10.1", "1.3.6.1.2.1.31.1.1.1.11.1", "1.3.6.1.2.1.31.1.1.1.13.1"]))]);
    let q = snmp_hops(&e, at).into_iter().find(|h| h.2 == MON).map(|h| h.1).unwrap_or(0);
    let (mut rb, mut rf, mut rbc, mut tb, mut tf, mut tbc) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for h in e.trace.hops() {
        let bc = h.bytes.len() >= 6 && h.bytes[..6] == [0xFF; 6];
        if h.to == (R1, 0) && h.t_rx <= q && h.drop.is_none() {
            rb += h.bytes.len() as u64;
            rf += 1;
            rbc += bc as u64;
        }
        if h.from == (R1, 0) && h.t_tx < q {
            tb += h.bytes.len() as u64;
            tf += 1;
            tbc += bc as u64;
        }
    }
    let got = a.first().map(|x| vbs(&x.1)).unwrap_or_default();
    t.eq(
        "counters",
        got.iter().map(|x| x.1.clone()).collect::<Vec<_>>(),
        alloc::vec![Value::Counter64(rb), Value::Counter64(tb), Value::Counter64(rf - rbc), Value::Counter64(rbc), Value::Counter32(rb as u32), Value::Counter64(tf - tbc), Value::Counter64(tbc)],
    );
    t.check(&format!("traffic there: {} {} {}", rf, rbc, tf), rf > 10 && rbc >= 1 && tf > 10);
    // ifXTable on a switch: 28 ports and Vlan1
    let o = cli(&mut e, MON, &["snmpwalk -v2c -c public -Os 10.0.0.2 ifName"], 3 * SEC);
    t.check(&format!("switch ifName: {}", o), o.lines().count() == 29 && o.starts_with("ifName.1 = STRING: Gi1/0/1\n") && o.contains("ifName.25 = STRING: Te1/1/1\n") && o.ends_with("ifName.29 = STRING: Vl1\n"));
    let o = cli(&mut e, MON, &["snmpget -v2c -c public -Ov 10.0.0.2 ifHCInOctets.2 ifType.29 ifSpeed.25 ifHighSpeed.25"], SEC);
    let mon_port = e.node(SW1).map(|n| n.ports[1].counters.rx_bytes).unwrap_or(0);
    t.check(&format!("switch port counters: {}", o), o.starts_with("Counter64: ") && o.lines().next().and_then(|l| l[11..].parse::<u64>().ok()).is_some_and(|v| v > 0 && v <= mon_port));
    t.check("switch rows", o.contains("INTEGER: propVirtual(53)\nGauge32: 4294967295\nGauge32: 10000\n"));
    // a Linux host: lo and eth0 as ip link numbers them
    let o = cli(&mut e, MON, &["snmpwalk -v2c -c public -Oqs 10.0.0.20 ifDescr"], 2 * SEC) + &cli(&mut e, MON, &["snmpget -v2c -c public -Oqs 10.0.0.20 ifMtu.1 ifType.1 ifPhysAddress.2"], SEC);
    t.eq("linux interfaces", o, s("ifDescr.1 lo\nifDescr.2 eth0\nifMtu.1 65536\nifType.1 softwareLoopback\nifPhysAddress.2 2:4a:0:0:4:1\n"));
    // addresses and routes
    let o = cli(&mut e, MON, &["snmpwalk -v2c -c public 10.0.0.1 ipAddrTable"], 2 * SEC);
    t.eq("ipAddrTable", o, s("IP-MIB::ipAdEntAddr.10.0.0.1 = IpAddress: 10.0.0.1\nIP-MIB::ipAdEntIfIndex.10.0.0.1 = INTEGER: 1\nIP-MIB::ipAdEntNetMask.10.0.0.1 = IpAddress: 255.255.255.0\nIP-MIB::ipAdEntBcastAddr.10.0.0.1 = INTEGER: 1\nIP-MIB::ipAdEntReasmMaxSize.10.0.0.1 = INTEGER: 18024\n"));
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "ip route 192.168.5.0 255.255.255.0 10.0.0.20", "end"], 0);
    let mut o = String::new();
    for c in ["snmpwalk -v2c -c public -Oqs 10.0.0.1 ipCidrRouteNextHop", "snmpwalk -v2c -c public -Oqs 10.0.0.1 ipCidrRouteProto", "snmpget -v2c -c public -Oqv 10.0.0.1 ipCidrRouteNumber.0 ipForwarding.0 ipDefaultTTL.0"] {
        o.push_str(&cli(&mut e, MON, &[c], 2 * SEC));
    }
    t.eq("routes", o, s("ipCidrRouteNextHop.10.0.0.0.255.255.255.0.0.0.0.0.0 0.0.0.0\nipCidrRouteNextHop.192.168.5.0.255.255.255.0.0.10.0.0.20 10.0.0.20\nipCidrRouteProto.10.0.0.0.255.255.255.0.0.0.0.0.0 local\nipCidrRouteProto.192.168.5.0.255.255.255.0.0.10.0.0.20 netmgmt\n2\nforwarding\n255\n"));
}

// ---------------------------------------------------------------- the order of the view

fn order(t: &mut T) {
    let e = engine(&project(R1_SNMP));
    let Some(v) = e.snmp_view(R1) else {
        t.check("a view", false);
        return;
    };
    let next = |x: &str| v.next(oid(x).arcs(), false).map(|(o, _)| format!("{}", o)).unwrap_or_default();
    t.eq("first of mib-2", next("1.3.6.1.2.1"), s("1.3.6.1.2.1.1.1.0"));
    t.eq("before everything", next("0.0"), s("1.3.6.1.2.1.1.1.0"));
    t.eq("a scalar to the next", next("1.3.6.1.2.1.1.5.0"), s("1.3.6.1.2.1.1.6.0"));
    t.eq("system to interfaces", next("1.3.6.1.2.1.1.7.0"), s("1.3.6.1.2.1.2.1.0"));
    t.eq("into a table", next("1.3.6.1.2.1.2.2"), s("1.3.6.1.2.1.2.2.1.1.1"));
    t.eq("down a column", next("1.3.6.1.2.1.2.2.1.2.3"), s("1.3.6.1.2.1.2.2.1.2.4"));
    t.eq("a column's end to the next column", next("1.3.6.1.2.1.2.2.1.2.8"), s("1.3.6.1.2.1.2.2.1.3.1"));
    t.eq("an index not there", next("1.3.6.1.2.1.2.2.1.2.4.7"), s("1.3.6.1.2.1.2.2.1.2.5"));
    t.eq("ifTable to ip", next("1.3.6.1.2.1.2.2.1.22.8"), s("1.3.6.1.2.1.4.1.0"));
    t.eq("ipAddrTable to ipForward", next("1.3.6.1.2.1.4.20.1.5.10.0.0.1"), s("1.3.6.1.2.1.4.24.3.0"));
    t.eq("ip to snmp", next("1.3.6.1.2.1.4.24.4.1.16.10.0.0.0.255.255.255.0.0.0.0.0.0"), s("1.3.6.1.2.1.11.1.0"));
    t.eq("a gap in snmp", next("1.3.6.1.2.1.11.6.0"), s("1.3.6.1.2.1.11.8.0"));
    t.eq("snmp to ifMIB", next("1.3.6.1.2.1.11.32.0"), s("1.3.6.1.2.1.31.1.1.1.1.1"));
    t.eq("the end", v.next(oid("1.3.6.1.2.1.31.1.1.1.19.8").arcs(), false), None);
    t.eq("after the end", v.next(oid("1.4").arcs(), false), None);
    // version 1 skips Counter64 (RFC 3584 4.2.2.1)
    t.eq("v1 skips Counter64", v.next(oid("1.3.6.1.2.1.31.1.1.1.5.8").arcs(), true).map(|(o, _)| format!("{}", o)), Some(s("1.3.6.1.2.1.31.1.1.1.14.1")));
    t.eq("v2c takes it", v.next(oid("1.3.6.1.2.1.31.1.1.1.5.8").arcs(), false).map(|(o, _)| format!("{}", o)), Some(s("1.3.6.1.2.1.31.1.1.1.6.1")));
    // Get: no object, no instance (RFC 3416 4.2.1)
    t.eq("noSuchObject", v.get(oid("1.3.6.1.2.1.1.9.0").arcs()), Value::NoSuchObject);
    t.eq("noSuchInstance of a scalar", v.get(oid("1.3.6.1.2.1.1.5.1").arcs()), Value::NoSuchInstance);
    t.eq("the object itself", v.get(oid("1.3.6.1.2.1.1.5").arcs()), Value::NoSuchInstance);
    t.eq("a row not there", v.get(oid("1.3.6.1.2.1.2.2.1.2.99").arcs()), Value::NoSuchInstance);
    t.eq("an entry", v.get(oid("1.3.6.1.2.1.2.2.1").arcs()), Value::NoSuchObject);
    t.eq("only for notifications", v.get(oid("1.3.6.1.6.3.1.1.4.1.0").arcs()), Value::NoSuchObject);
    t.eq("outside mib-2", v.get(oid("1.3.6.1.4.1.32473.1").arcs()), Value::NoSuchObject);
}

// ---------------------------------------------------------------- GetBulk

fn bulk(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let b = |n: i32, m: i32, oids: &[&str]| Message::new(Version::V2c, "public", Pdu::bulk(9, n, m, oids.iter().map(|o| VarBind::null(oid(o))).collect())).to_bytes();
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, b(1, 3, &["1.3.6.1.2.1.1", "1.3.6.1.2.1.2.2.1.2", "1.3.6.1.2.1.2.2.1.8"]))]);
    let got: Vec<String> = a.first().map(|x| vbs(&x.1).into_iter().map(|v| v.0).collect()).unwrap_or_default();
    t.eq("non-repeater once, repeaters three times", got, alloc::vec![s("1.3.6.1.2.1.1.1.0"), s("1.3.6.1.2.1.2.2.1.2.1"), s("1.3.6.1.2.1.2.2.1.8.1"), s("1.3.6.1.2.1.2.2.1.2.2"), s("1.3.6.1.2.1.2.2.1.8.2"), s("1.3.6.1.2.1.2.2.1.2.3"), s("1.3.6.1.2.1.2.2.1.8.3")]);
    // the end of the view in every repeater stops it early
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, b(0, 50, &["1.3.6.1.2.1.31.1.1.1.19.6"]))]);
    let got = a.first().map(|x| vbs(&x.1)).unwrap_or_default();
    t.eq("to the end", got, alloc::vec![(s("1.3.6.1.2.1.31.1.1.1.19.7"), Value::TimeTicks(0)), (s("1.3.6.1.2.1.31.1.1.1.19.8"), Value::TimeTicks(0)), (s("1.3.6.1.2.1.31.1.1.1.19.8"), Value::EndOfMibView)]);
    // many repetitions: fewer bindings than asked, but within 1472 bytes
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, b(0, 500, &["1.3.6.1.2.1.2.2.1.2"]))]);
    let (n, len) = a.first().map(|x| (x.1.varbinds().len(), x.1.to_bytes().len())).unwrap_or((0, 0));
    t.check(&format!("size limit: {} bindings in {} bytes", n, len), n > 30 && n < 500 && len <= jfn_sim::snmpd::MAX_MSG && len > jfn_sim::snmpd::MAX_MSG - 60);
    let o = cli(&mut e, MON, &["snmpbulkwalk -v2c -c public -Cr4 -Oqs 10.0.0.1 ifAdminStatus"], 2 * SEC);
    t.eq("bulkwalk", o, s("ifAdminStatus.1 up\nifAdminStatus.2 down\nifAdminStatus.3 down\nifAdminStatus.4 down\nifAdminStatus.5 down\nifAdminStatus.6 down\nifAdminStatus.7 down\nifAdminStatus.8 down\n"));
    let at = e.now();
    let _ = cli(&mut e, MON, &["snmpbulkwalk -v2c -c public -Cr4 10.0.0.1 ifAdminStatus"], 2 * SEC);
    t.eq("three GetBulk for eight rows", snmp_hops(&e, at).iter().filter(|h| h.3.kind() == Kind::GetBulk).count(), 3);
    let o = cli(&mut e, MON, &["snmpbulkwalk -v1 -c public 10.0.0.1"], SEC);
    t.eq("no GetBulk in v1", o, s("snmpbulkwalk: GetBulk needs version 2c\n"));
}

// ---------------------------------------------------------------- Set

fn sets(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let start = e.startup_config(R1);
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 sysLocation.0 s \"Serverraum 2\" sysContact.0 = noc@lab"], SEC);
    t.eq("set two", o, s("SNMPv2-MIB::sysLocation.0 = STRING: Serverraum 2\nSNMPv2-MIB::sysContact.0 = STRING: noc@lab\n"));
    t.check("in the running configuration", e.running_config(R1).contains("snmp-server location Serverraum 2\nsnmp-server contact noc@lab\n"));
    t.eq("not in the startup configuration", e.startup_config(R1), start.clone());
    t.check("the notes", notes(&e, R1).iter().any(|n| n.1 == SNote::Set { var: s("SNMPv2-MIB::sysLocation.0"), value: s("STRING: Serverraum 2") }));
    let o = cli(&mut e, MON, &["snmpset -v2c -c private -r 0 10.0.0.20 sysLocation.0 s x"], 2 * SEC);
    t.eq("PC1 knows no private", o, s("snmpset: 10.0.0.20 did not answer (1 try of 1 s)\n"));
    // ifAdminStatus: Gi0/0/1 opens and shuts as with shutdown, the cable to PC2 follows
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 IF-MIB::ifAdminStatus.2 i up"], SEC);
    t.eq("ifAdminStatus up", o, s("IF-MIB::ifAdminStatus.2 = INTEGER: up(1)\n"));
    t.check("the port is up", e.node(R1).is_some_and(|n| n.ports[1].admin_up) && e.link_idx_up(L_PC2));
    t.check("open in the running configuration", e.running_config(R1).contains("interface GigabitEthernet0/0/1\n no ip address\n!\n"));
    let _ = e.take_reports();
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 ifAdminStatus.2 i 2"], SEC);
    t.eq("ifAdminStatus down", o, s("IF-MIB::ifAdminStatus.2 = INTEGER: down(2)\n"));
    t.check("the port is down", e.node(R1).is_some_and(|n| !n.ports[1].admin_up) && !e.link_idx_up(L_PC2));
    t.check("shut in the running configuration", e.running_config(R1).contains("interface GigabitEthernet0/0/1\n no ip address\n shutdown\n"));
    t.eq("still not in the startup configuration", e.startup_config(R1), start.clone());
    let o = cli(&mut e, R1, &["enable", "copy running-config startup-config", ""], 0);
    t.check("copy run start keeps it", o.contains("[OK]") && e.startup_config(R1).contains("snmp-server location Serverraum 2"));
    // sysName renames the device as hostname does (the app follows Report::Hostname)
    let _ = e.take_reports();
    let _ = e.cli_line(MON, "snmpset -v2c -c private 10.0.0.1 sysName.0 s Kern1");
    run_for(&mut e, SEC);
    let reps = e.take_reports();
    t.check("renamed", e.node(R1).is_some_and(|n| n.name == "Kern1") && reps.iter().any(|r| matches!(r, Report::Hostname { dev: R1, name } if name == "Kern1")));
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 sysName.0 s \"kein name\""], SEC);
    t.eq("a name that is none", o, s("snmpset: the agent answered wrongValue at binding 1 (SNMPv2-MIB::sysName.0)\n"));
    // all or nothing: a bad second binding changes neither
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 sysLocation.0 s Keller sysUpTime.0 t 5"], SEC);
    t.eq("all or nothing", o, s("snmpset: the agent answered notWritable at binding 2 (SNMPv2-MIB::sysUpTime.0)\n"));
    t.check("nothing changed", e.snmp_agent(R1).is_some_and(|a| a.cfg.location == "Serverraum 2"));
    let mut o = String::new();
    for c in ["snmpset -v2c -c private 10.0.0.1 sysLocation.0 i 5", "snmpset -v2c -c private 10.0.0.1 ifAdminStatus.1 i 3", "snmpset -v2c -c private 10.0.0.1 ifAdminStatus.99 i 1", "snmpset -v2c -c private 10.0.0.1 sysLocation.1 s x"] {
        o.push_str(&cli(&mut e, MON, &[c], SEC));
    }
    t.eq("the errors of 2c", o, s("snmpset: the agent answered wrongType at binding 1 (SNMPv2-MIB::sysLocation.0)\nsnmpset: the agent answered wrongValue at binding 1 (IF-MIB::ifAdminStatus.1)\nsnmpset: the agent answered noCreation at binding 1 (IF-MIB::ifAdminStatus.99)\nsnmpset: the agent answered noCreation at binding 1 (SNMPv2-MIB::sysLocation.1)\n"));
    let long: String = core::iter::repeat_n('x', 256).collect();
    let o = cli(&mut e, MON, &[&format!("snmpset -v2c -c private 10.0.0.1 sysContact.0 s {}", long)], SEC);
    t.eq("too long", o, s("snmpset: the agent answered wrongLength at binding 1 (SNMPv2-MIB::sysContact.0)\n"));
    let o = cli(&mut e, MON, &["snmpset -v2c -c public 10.0.0.1 sysLocation.0 s x"], SEC);
    t.eq("read-only community", o, s("snmpset: the agent answered noAccess at binding 1 (SNMPv2-MIB::sysLocation.0)\n"));
    t.check("counted as a bad use", e.snmp_agent(R1).is_some_and(|a| a.counters[5] == 1));
    // ifAlias is the description
    let _ = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 ifAlias.2 s Reserve"], SEC);
    t.check("ifAlias", e.running_config(R1).contains("interface GigabitEthernet0/0/1\n description Reserve\n"));
    let o = cli(&mut e, MON, &["snmpset -v2c -c private 10.0.0.1 sysLocation.0 q x", "snmpset -v2c -c private 10.0.0.1 ifAdminStatus.1 i oben", "snmpset -v2c -c private 10.0.0.1 sysLocation.0"], 0);
    t.eq("the tool's refusals", o, s("snmpset: 'q' is no type (i u t a o s x d n =)\nsnmpset: 'oben' is no value of type i\nsnmpset: each object needs a type and a value\nUsage: snmpset [-v 1|2c] -c COMMUNITY [-r RETRIES] [-t SECONDS] [-On|-Oq|-Ov|-Os] AGENT OID TYPE VALUE [OID TYPE VALUE ...]\n"));
}

// ---------------------------------------------------------------- refused requests

fn refused(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let at = e.now();
    let o = cli(&mut e, MON, &["snmpget -v2c -c falsch -r 1 10.0.0.1 sysName.0"], 3 * SEC);
    t.eq("wrong community: no answer", o, s("snmpget: 10.0.0.1 did not answer (2 tries of 1 s each)\n"));
    let h = snmp_hops(&e, at);
    t.eq("two requests, two authenticationFailure traps, no response", h.iter().map(|x| (x.2, x.3.kind())).collect::<Vec<_>>(), alloc::vec![(MON, Kind::Get), (R1, Kind::TrapV2), (MON, Kind::Get), (R1, Kind::TrapV2)]);
    t.check("the trap says it", h.get(1).and_then(|x| x.3.pdu().and_then(|p| snmp::notification(&p.varbinds).map(|o| o.arcs().to_vec()))) == Some(Vec::from(&snmp::AUTH_FAILURE[..])));
    t.check("counted", e.snmp_agent(R1).is_some_and(|a| a.counters[4] == 2 && a.refused == 2));
    t.check("noted", notes(&e, R1).iter().filter(|n| matches!(&n.1, SNote::BadCommunity { community, .. } if community == "falsch")).count() == 2);
    t.check("snmptrapd has them", e.snmp_traps(MON).iter().filter(|r| r.what == "SNMPv2-MIB::authenticationFailure").count() == 2);
    // the access list keeps PC1 out of "private"
    let at = e.now();
    let o = cli(&mut e, PC1, &["snmpget -v2c -c private -r 0 10.0.0.1 sysName.0"], 2 * SEC);
    t.eq("access list: no answer", o, s("snmpget: 10.0.0.1 did not answer (1 try of 1 s)\n"));
    t.check("denied", notes(&e, R1).iter().any(|n| n.0 >= at && matches!(&n.1, SNote::Denied { acl, .. } if acl == "Zugriffsliste 10")));
    let o = cli(&mut e, MON, &["snmpget -v2c -c private 10.0.0.1 sysName.0"], SEC);
    t.eq("MON may", o, s("SNMPv2-MIB::sysName.0 = STRING: R1\n"));
    let o = cli(&mut e, R1, &["enable", "show access-lists"], 0);
    t.check(&format!("the list counted: {}", o), o.contains("10 permit 10.0.0.50 (1 match)"));
    // the console says it in our own words
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let _ = e.take_reports();
    let _ = e.cli_line(MON, "snmpget -v2c -c falsch -r 0 10.0.0.1 sysName.0");
    run_for(&mut e, 2 * SEC);
    let r1 = out(&mut e, R1);
    t.check(&format!("console line: {}", r1), r1.ends_with("%SNMP-3-AUTHFAIL: refused a request from 10.0.0.50: unknown community\n"));
    // SNMPv3, unreadable bytes: dropped and counted
    let mut v3 = req(Version::V2c, "public", Kind::Get, 1, &["1.3.6.1.2.1.1.5.0"]);
    v3[4] = 3;
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, v3), (MS, alloc::vec![0x30, 0x03, 0x02, 0x01]), (2 * MS, req(Version::V2c, "public", Kind::Get, 2, &["1.3.6.1.2.1.11.3.0", "1.3.6.1.2.1.11.6.0"]))]);
    t.eq("only the last answered", a.len(), 1);
    t.eq("bad versions, parse errors", a.first().map(|x| vbs(&x.1).into_iter().map(|v| v.1).collect::<Vec<_>>()), Some(alloc::vec![Value::Counter32(1), Value::Counter32(1)]));
    t.check("their notes", notes(&e, R1).iter().any(|n| matches!(n.1, SNote::BadVersion { version: 3, .. })) && notes(&e, R1).iter().any(|n| matches!(n.1, SNote::Malformed { .. })));
}

// ---------------------------------------------------------------- versions 1 and 2c

fn versions(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let a = ask(
        &mut e,
        [10, 0, 0, 1],
        alloc::vec![
            (0, req(Version::V2c, "public", Kind::Get, 1, &["1.3.6.1.2.1.1.5.0", "1.3.6.1.2.1.1.9.0", "1.3.6.1.2.1.1.5.1"])),
            (MS, req(Version::V1, "public", Kind::Get, 2, &["1.3.6.1.2.1.1.5.0", "1.3.6.1.2.1.1.9.0", "1.3.6.1.2.1.1.5.1"])),
            (2 * MS, req(Version::V2c, "public", Kind::GetNext, 3, &["1.3.6.1.2.1.31.1.1.1.19.8", "1.3.6.1.2.1.1.4.0"])),
            (3 * MS, req(Version::V1, "public", Kind::GetNext, 4, &["1.3.6.1.2.1.1.4.0", "1.3.6.1.2.1.31.1.1.1.19.8"])),
            (4 * MS, req(Version::V1, "public", Kind::Get, 5, &["1.3.6.1.2.1.31.1.1.1.6.1"])),
            (5 * MS, req(Version::V1, "public", Kind::GetNext, 6, &["1.3.6.1.2.1.31.1.1.1.5.8"])),
        ],
    );
    let es = |m: Option<Message>| m.and_then(|m| m.pdu().map(|p| (p.error_status, p.error_index, p.varbinds.iter().map(|v| v.value.clone()).collect::<Vec<_>>())));
    t.eq("2c: exceptions in the bindings", es(by_id(&a, 1)), Some((0, 0, alloc::vec![Value::text("R1"), Value::NoSuchObject, Value::NoSuchInstance])));
    t.eq("v1: noSuchName at 2, the request's bindings", es(by_id(&a, 2)), Some((snmp::E_NO_SUCH_NAME, 2, alloc::vec![Value::Null, Value::Null, Value::Null])));
    t.eq("2c: endOfMibView", es(by_id(&a, 3)), Some((0, 0, alloc::vec![Value::EndOfMibView, Value::text("R1")])));
    t.eq("v1: the end is noSuchName", es(by_id(&a, 4)), Some((snmp::E_NO_SUCH_NAME, 2, alloc::vec![Value::Null, Value::Null])));
    t.eq("v1: no Counter64", es(by_id(&a, 5)), Some((snmp::E_NO_SUCH_NAME, 1, alloc::vec![Value::Null])));
    t.eq("v1: GetNext skips Counter64", by_id(&a, 6).map(|m| vbs(&m)), Some(alloc::vec![(s("1.3.6.1.2.1.31.1.1.1.14.1"), Value::Int(1))]));
    t.check("v1 responses are v1", by_id(&a, 2).is_some_and(|m| m.version == Version::V1));
    // Set: notWritable and wrongType of 2c are noSuchName and badValue in version 1
    let set = |v: Version, id: i32, o: &str, val: Value| Message::new(v, "private", Pdu::new(Kind::Set, id, alloc::vec![VarBind::new(oid(o), val)])).to_bytes();
    let a = ask(
        &mut e,
        [10, 0, 0, 1],
        alloc::vec![
            (0, set(Version::V2c, 11, "1.3.6.1.2.1.1.1.0", Value::text("x"))),
            (MS, set(Version::V1, 12, "1.3.6.1.2.1.1.1.0", Value::text("x"))),
            (2 * MS, set(Version::V2c, 13, "1.3.6.1.2.1.1.6.0", Value::Int(1))),
            (3 * MS, set(Version::V1, 14, "1.3.6.1.2.1.1.6.0", Value::Int(1))),
            (4 * MS, set(Version::V1, 15, "1.3.6.1.2.1.1.6.0", Value::text("Halle"))),
        ],
    );
    let st = |id: i32| by_id(&a, id).and_then(|m| m.pdu().map(|p| (p.error_status, p.error_index)));
    t.eq("set errors by version", [st(11), st(12), st(13), st(14), st(15)], [Some((snmp::E_NOT_WRITABLE, 1)), Some((snmp::E_NO_SUCH_NAME, 1)), Some((snmp::E_WRONG_TYPE, 1)), Some((snmp::E_BAD_VALUE, 1)), Some((0, 0))]);
    // tooBig: 2c without bindings, v1 with the request's
    let many: Vec<&str> = core::iter::repeat_n("1.3.6.1.2.1.1.1.0", 30).collect();
    let a = ask(&mut e, [10, 0, 0, 1], alloc::vec![(0, req(Version::V2c, "public", Kind::Get, 21, &many)), (MS, req(Version::V1, "public", Kind::Get, 22, &many))]);
    let big = |id: i32| by_id(&a, id).and_then(|m| m.pdu().map(|p| (p.error_status, p.error_index, p.varbinds.len())));
    t.eq("tooBig", (big(21), big(22)), (Some((snmp::E_TOO_BIG, 0, 0)), Some((snmp::E_TOO_BIG, 0, 30))));
    // the tools print the errors of version 1 and the exceptions of 2c
    let o = cli(&mut e, MON, &["snmpget -v1 -c public 10.0.0.1 sysName.0 sysFoo.0"], 0);
    t.eq("an unknown name", o, s("snmpget: sysFoo.0: no object of that name here (dotted numbers always work)\n"));
    let mut o = String::new();
    for c in ["snmpget -v1 -c public 10.0.0.1 sysName.0 1.3.6.1.2.1.1.9.0", "snmpget -c public 10.0.0.1 1.3.6.1.2.1.1.9.0 sysName.1", "snmpwalk -v1 -c public 10.0.0.1 ifSpecific", "snmpwalk -c public 10.0.0.1 sysName.0"] {
        o.push_str(&cli(&mut e, MON, &[c], SEC));
    }
    t.eq(
        "tool output",
        o,
        s("snmpget: the agent answered noSuchName at binding 2 (SNMPv2-MIB::system.9.0)\nSNMPv2-MIB::system.9.0 = No Such Object (the agent has no object of this name)\nSNMPv2-MIB::sysName.1 = No Such Instance (the object has no such row or instance)\nIF-MIB::ifSpecific.1 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.2 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.3 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.4 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.5 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.6 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.7 = OID: SNMPv2-SMI::zeroDotZero\nIF-MIB::ifSpecific.8 = OID: SNMPv2-SMI::zeroDotZero\nSNMPv2-MIB::sysName.0 = STRING: R1\n"),
    );
    let o = cli(&mut e, MON, &["snmpget", "snmpget -v3 -c x 10.0.0.1 sysName.0", "snmpwalk -h"], 0);
    t.check(&format!("usage: {}", o), o.starts_with("snmpget: no community given (-c NAME)\nUsage: snmpget ") && o.contains("snmpget: version '3' is none of 1 and 2c (SNMPv3 is not here)\n") && o.contains("Usage: snmpwalk [-v 1|2c] -c COMMUNITY") && o.contains("goes through every object below OID"));
}

// ---------------------------------------------------------------- traps

fn traps(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    // PC1's coldStart reaches MON at boot (Linux waits for ARP); IOS-like R1 and SW1 lose theirs
    // to ARP as their first ping would
    run_for(&mut e, SEC);
    let boot = e.snmp_traps(MON);
    t.eq("coldStart of PC1", boot.iter().map(|r| (r.from, r.what.clone())).collect::<Vec<_>>(), alloc::vec![(ip([10, 0, 0, 20]), s("SNMPv2-MIB::coldStart"))]);
    t.check("R1 sent one", notes(&e, R1).iter().any(|n| n.0 == 0 && matches!(&n.1, SNote::Notify { what, inform: false, .. } if what == "coldStart")));
    // MON talks to SW1 first, so SW1 knows its MAC; then PC1's cable is pulled
    let _ = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.2 sysName.0"], 2 * SEC);
    let at = e.now();
    let _ = e.apply(Action::Unplug { link: L_PC1, down: true });
    run_for(&mut e, SEC);
    let down: Vec<_> = e.snmp_traps(MON).into_iter().filter(|r| r.what == "IF-MIB::linkDown").collect();
    t.eq("linkDown of SW1 Gi1/0/3", down.iter().map(|r| (r.from, r.vars.clone())).collect::<Vec<_>>(), alloc::vec![(ip([10, 0, 0, 2]), alloc::vec![s("IF-MIB::ifIndex.3 = INTEGER: 3"), s("IF-MIB::ifAdminStatus.3 = INTEGER: up(1)"), s("IF-MIB::ifOperStatus.3 = INTEGER: down(2)")])]);
    let h: Vec<_> = snmp_hops(&e, at).into_iter().filter(|x| x.2 == SW1).collect();
    t.eq("sent at once", h.first().map(|x| x.0), Some(at));
    t.eq("its uptime", h.first().and_then(|x| x.3.pdu().map(|p| p.varbinds[0].value.clone())), Some(Value::TimeTicks((at / 10_000_000) as u32)));
    // PC1 lost its link too: its trap has no way out
    t.check("PC1 has no route now", notes(&e, PC1).iter().any(|n| n.0 >= at && matches!(n.1, SNote::NoRoute { .. })));
    let at2 = e.now();
    let _ = e.apply(Action::Unplug { link: L_PC1, down: false });
    run_for(&mut e, SEC);
    let ups: Vec<_> = e.snmp_traps(MON).into_iter().filter(|r| r.what == "IF-MIB::linkUp" && r.at >= at2).collect();
    t.eq("linkUp of SW1 and PC1", ups.iter().map(|r| format!("{} {}", r.from, r.vars.join(", "))).collect::<Vec<_>>(), alloc::vec![s("10.0.0.2 IF-MIB::ifIndex.3 = INTEGER: 3, IF-MIB::ifAdminStatus.3 = INTEGER: up(1), IF-MIB::ifOperStatus.3 = INTEGER: up(1)"), s("10.0.0.20 IF-MIB::ifIndex.2 = INTEGER: 2, IF-MIB::ifAdminStatus.2 = INTEGER: up(1), IF-MIB::ifOperStatus.2 = INTEGER: up(1)")]);
    // the log file, the device's log, the metric, ifLastChange
    let o = cli(&mut e, MON, &["cat /var/log/snmptrapd.log"], 0);
    t.check("the file", o.contains(" 10.0.0.2 [UDP: [10.0.0.2]:") && o.contains("->[10.0.0.50]:162]:\nSNMPv2-MIB::sysUpTime.0 = Timeticks: (") && o.contains("\tSNMPv2-MIB::snmpTrapOID.0 = OID: IF-MIB::linkDown\tIF-MIB::ifIndex.3 = INTEGER: 3\t"));
    t.check("the log", e.log_lines(MON).iter().any(|l| l.tag == "snmptrapd" && l.text.starts_with("linkDown trap from 10.0.0.2 (v2c, community public): IF-MIB::ifIndex.3 = INTEGER: 3")));
    t.eq("metric", e.metric(MON, "traps", "traps"), Some(4));
    t.eq("ifLastChange", e.snmp_view(SW1).and_then(|v| v.if_row(3).map(|r| r.last_change)), Some((at2 / 10_000_000) as u32));
    t.check("the file is the project's", e.files(MON).iter().any(|f| f.path == "/var/log/snmptrapd.log"));
    // ifAdminStatus down from SNMP: the trap says admin down
    let _ = cli(&mut e, SW1, &["enable", "configure terminal", "snmp-server community rw RW", "end"], 0);
    let at3 = e.now();
    let _ = cli(&mut e, MON, &["snmpset -v2c -c rw 10.0.0.2 ifAdminStatus.3 i 2"], SEC);
    let last = e.snmp_traps(MON).into_iter().filter(|r| r.at >= at3).map(|r| r.vars.join(", ")).collect::<Vec<_>>();
    t.check(&format!("admin down: {:?}", last), last.contains(&s("IF-MIB::ifIndex.3 = INTEGER: 3, IF-MIB::ifAdminStatus.3 = INTEGER: down(2), IF-MIB::ifOperStatus.3 = INTEGER: down(2)")));
    // traps of version 1
    let _ = cli(&mut e, SW1, &["configure terminal", "no snmp-server host 10.0.0.50", "snmp-server host 10.0.0.50 public", "interface gi1/0/3", "no shutdown", "end"], SEC);
    let v1: Vec<_> = e.snmp_traps(MON).into_iter().filter(|r| r.version == Version::V1).collect();
    t.eq("v1 linkUp", v1.iter().map(|r| (r.what.clone(), r.vars.clone())).collect::<Vec<_>>(), alloc::vec![(s("IF-MIB::linkUp"), alloc::vec![s("IF-MIB::ifIndex.3 = INTEGER: 3"), s("IF-MIB::ifAdminStatus.3 = INTEGER: up(1)"), s("IF-MIB::ifOperStatus.3 = INTEGER: up(1)")])]);
    let o = cli(&mut e, MON, &["cat /var/log/snmptrapd.log"], 0);
    t.check("v1 in the file", o.contains("(via UDP: [10.0.0.2]:") && o.contains(") TRAP, SNMP v1, community public\n\tSNMPv2-SMI::enterprises.32473.1.10 linkUp (0) Uptime: "));
    // a receiver that takes only "public" drops another community
    let _ = cli(&mut e, SW1, &["configure terminal", "snmp-server host 10.0.0.50 version 2c anders", "interface gi1/0/3", "shutdown", "end"], SEC);
    t.check("refused", e.metric(MON, "traps", "refused") == Some(1) && notes(&e, MON).iter().any(|n| matches!(&n.1, SNote::TrapRefused { community, .. } if community == "anders")));
}

// ---------------------------------------------------------------- informs

fn informs(t: &mut T) {
    let lines = R1_SNMP.replace("snmp-server host 10.0.0.50 version 2c public", "snmp-server host 10.0.0.50 informs version 2c public");
    let mut e = engine(&project(&lines));
    run_for(&mut e, SEC);
    // the coldStart inform at boot went out at 0, lost to ARP, again after 15 s
    let _ = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.1 sysName.0"], 20 * SEC);
    let n = notes(&e, R1);
    let again: Vec<u64> = n.iter().filter(|x| matches!(&x.1, SNote::InformAgain { what, n: 1, .. } if what == "coldStart")).map(|x| x.0).collect();
    t.eq("again after 15 s", again, alloc::vec![15 * SEC]);
    t.check("acknowledged", n.iter().any(|x| x.0 > 15 * SEC && x.0 < 15 * SEC + MS && matches!(&x.1, SNote::InformAcked { what, .. } if what == "coldStart")));
    t.eq("receiver", e.snmp_traps(MON).iter().filter(|r| r.inform).map(|r| r.what.clone()).collect::<Vec<_>>(), alloc::vec![s("SNMPv2-MIB::coldStart")]);
    t.eq("metrics", (e.metric(R1, "snmp", "informs"), e.metric(R1, "snmp", "informs_acked"), e.metric(R1, "snmp", "informs_pending"), e.metric(MON, "traps", "informs")), (Some(1), Some(1), Some(0), Some(1)));
    // a lossy cable to MON: the inform of a linkUp of Gi0/0/1 goes again until it gets through
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "interface gi0/0/1", "no shutdown", "end"], 0);
    let _ = e.apply(Action::Unplug { link: L_PC2, down: true });
    run_for(&mut e, SEC);
    let _ = e.apply(Action::Impair { link: L_MON, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    let at = e.now();
    let _ = e.apply(Action::Unplug { link: L_PC2, down: false });
    run_for(&mut e, 20 * SEC);
    let _ = e.apply(Action::Impair { link: L_MON, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(0) });
    run_for(&mut e, 20 * SEC);
    let n: Vec<(u64, SNote)> = notes(&e, R1).into_iter().filter(|x| x.0 >= at).collect();
    let tries: Vec<u64> = n.iter().filter(|x| matches!(&x.1, SNote::Notify { what, .. } | SNote::InformAgain { what, .. } if what == "linkUp")).map(|x| x.0 - at).collect();
    t.eq("tries every 15 s", tries, alloc::vec![0, 15 * SEC, 30 * SEC]);
    t.check("then acknowledged", n.iter().any(|x| x.0 >= at + 30 * SEC && matches!(&x.1, SNote::InformAcked { what, .. } if what == "linkUp")));
    t.eq("logged once", e.snmp_traps(MON).iter().filter(|r| r.what == "IF-MIB::linkUp" && r.at >= at).count(), 1);
    t.check("the receiver's answer", snmp_hops(&e, at).iter().any(|x| x.2 == MON && x.3.kind() == Kind::Response));
    // nobody acknowledges: given up after 3 more tries
    let _ = e.apply(Action::Impair { link: L_MON, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    let at = e.now();
    let _ = e.apply(Action::Unplug { link: L_PC2, down: true });
    run_for(&mut e, 70 * SEC);
    let n: Vec<(u64, SNote)> = notes(&e, R1).into_iter().filter(|x| x.0 >= at).collect();
    t.check("lost after 60 s", n.iter().any(|x| x.0 == at + 60 * SEC && matches!(&x.1, SNote::InformLost { what, .. } if what == "linkDown")));
    t.eq("lost counted", e.metric(R1, "snmp", "informs_lost"), Some(1));
    let o = cli(&mut e, R1, &["show snmp"], 0);
    t.check(&format!("show snmp: {}", o), o.contains("5 notifications: 0 traps, 5 informs (4 acknowledged, 1 given up, 0 waiting)") && o.contains("  to 10.0.0.50 port 162: informs, version 2c, community public\n"));
}

// ---------------------------------------------------------------- the Linux agent

fn linux(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let o = cli(&mut e, MON, &["snmpget -v1 -c public 10.0.0.20 sysName.0 sysLocation.0 sysContact.0 sysServices.0 sysObjectID.0"], SEC);
    t.eq("linux system", o, s("SNMPv2-MIB::sysName.0 = STRING: PC1\nSNMPv2-MIB::sysLocation.0 = STRING: Büro 1\nSNMPv2-MIB::sysContact.0 = STRING: anna\nSNMPv2-MIB::sysServices.0 = INTEGER: 72\nSNMPv2-MIB::sysObjectID.0 = OID: SNMPv2-SMI::enterprises.32473.1.1\n"));
    let o = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.20 sysDescr.0"], SEC);
    t.check(&format!("own words: {}", o), o.starts_with("SNMPv2-MIB::sysDescr.0 = STRING: JFNetworker host system 1.0 (Linux-like) for the PC"));
    // "geheim" may write from MON's address only
    let o = cli(&mut e, MON, &["snmpset -v2c -c geheim 10.0.0.20 sysLocation.0 s Lager"], SEC);
    t.eq("MON may write", o, s("SNMPv2-MIB::sysLocation.0 = STRING: Lager\n"));
    t.eq("the running service has it", e.snmp_agent(PC1).map(|a| a.cfg.location.clone()), Some(s("Lager")));
    // R1 may not (its first try is lost to IOS's ARP, the second refused)
    let a = ask_from(&mut e, R1, [10, 0, 0, 20], alloc::vec![(0, req(Version::V2c, "geheim", Kind::Get, 1, &["1.3.6.1.2.1.1.5.0"])), (SEC, req(Version::V2c, "geheim", Kind::Get, 2, &["1.3.6.1.2.1.1.5.0"]))]);
    t.eq("not for R1", (a.len(), e.snmp_agent(PC1).map(|a| a.refused)), (0, Some(1)));
    t.check("logged on PC1", e.log_lines(PC1).iter().any(|l| l.tag == "snmpd" && l.text == "refused a request from 10.0.0.1: community not for this address"));
    // the settings and their refusals
    let mut g = Engine::new(1);
    let _ = g.apply(Action::AddDevice(Device { id: 9, name: s("H"), model: s("pc"), ..Device::default() }));
    let mut bad = |kind: &str, cfg: &str| g.service_def(9, &def(kind, kind, cfg)).err().map(|e| e.msg().text()).unwrap_or_default();
    t.eq(
        "refusals",
        [
            bad("snmpd", r#"{"communities": [{"name": ""}]}"#),
            bad("snmpd", r#"{"communities": [{"name": "a", "access": "all"}]}"#),
            bad("snmpd", r#"{"communities": [{"name": "a", "source": "zehn"}]}"#),
            bad("snmpd", r#"{"sinks": [{"host": "mon"}]}"#),
            bad("snmpd", r#"{"sinks": [{"host": "10.0.0.1", "type": "inform", "version": "1"}]}"#),
            bad("snmpd", r#"{"notify": ["coldstart", "warmstart"]}"#),
            bad("snmpd", r#"{"port": 0}"#),
            bad("snmptrapd", r#"{"communities": ["a b"]}"#),
            bad("snmptrapd", r#"{"log": 5}"#),
        ],
        [
            s("Dienst snmpd: Eine Community braucht „name“ ohne Leerzeichen."),
            s("Dienst snmpd: „all“ ist kein Zugriff (ro oder rw)."),
            s("Dienst snmpd: „zehn“ ist kein Netz wie 10.0.0.0/24."),
            s("Dienst snmpd: „mon“ ist keine IPv4-Adresse."),
            s("Dienst snmpd: Informs gibt es erst ab Version 2c."),
            s("Dienst snmpd: „warmstart“ ist keine Meldung (coldstart, linkdown, linkup, authentication)."),
            s("Dienst snmpd: Port 0 gibt es nicht."),
            s("Dienst snmptrapd: Eine Community ist ein Wort ohne Leerzeichen."),
            s("Dienst snmptrapd: „log“ ist der Pfad einer Datei."),
        ]
    );
    t.check("defaults", g.service_def(9, &def("snmpd", "snmpd", "{}")).is_ok() && g.service_def(9, &def("snmptrapd", "trapd", "{}")).is_ok());
    run_for(&mut g, SEC);
    let st: Vec<String> = g.services(9).into_iter().map(|s| s.status).collect();
    t.eq("status lines", st, alloc::vec![s("ohne Community: beantwortet nichts, 0 Meldungen gesendet"), s("wartet auf Meldungen an Port 162")]);
    t.check("snmp-server is IOS", g.apply(Action::Snmp { dev: 9, set: jfn_sim::SnmpSet::Location(Some(s("x"))) }).is_err());
    // the status lines of busy services
    t.eq("busy status", e.services(PC1).into_iter().find(|x| x.kind == "snmpd").map(|x| x.status), Some(format!("{} Anfragen beantwortet, 1 abgelehnt, 1 Meldungen gesendet", e.snmp_agent(PC1).map(|a| a.responses).unwrap_or(0))));
    let rows = e.snmp_traps(MON);
    t.eq("trapd status", e.services(MON).into_iter().find(|x| x.kind == "snmptrapd").map(|x| x.status), Some(format!("{} Traps und 0 Informs empfangen; zuletzt SNMPv2-MIB::coldStart von 10.0.0.20", rows.len())));
}

// ---------------------------------------------------------------- the IOS-like command line

fn ios(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    let run = e.running_config(R1);
    t.check("the lines", run.contains("access-list 10 permit 10.0.0.50\n!\nsnmp-server community public RO\nsnmp-server community private RW 10\nsnmp-server location Technikraum\nsnmp-server contact netz@lab.example\nsnmp-server enable traps snmp authentication linkdown linkup coldstart\nsnmp-server host 10.0.0.50 version 2c public\n!\nline con 0\n"));
    // the running configuration read back is the same
    let mut p = project(R1_SNMP);
    if let Some(d) = p.device_mut(R1) {
        d.startup_config = run.clone();
    }
    let f = engine(&p);
    t.eq("read back", f.running_config(R1), run.clone());
    let h = e.cli_help(R1, "snmp-server ");
    let _ = h;
    let o = cli(&mut e, R1, &["enable", "configure terminal"], 0);
    let _ = o;
    let h = e.cli_help(R1, "snmp-server ");
    t.check(&format!("help: {}", h), h.contains("community") && h.contains("A community: the shared word of SNMPv1 and v2c") && h.contains("enable") && h.contains("host") && h.contains("location") && h.contains("contact"));
    let o = cli(&mut e, R1, &["snmp-server host 10.0.0.60 informs public", "snmp-server community x rw 150", "snmp-server host 0.0.0.0 public", "no snmp-server community nichts", "no snmp-server host 10.0.0.99"], 0);
    t.eq("refusals", o, s("% Informs need version 2c\n% Access list 150 is no standard list: SNMP takes 1-99 and 1300-1999\n% 0.0.0.0 cannot receive notifications\n% No community nichts here\n% 10.0.0.99 receives no notifications here\n"));
    let _ = cli(&mut e, R1, &["snmp-server host 10.0.0.60 informs version 2c x", "snmp-server community ops RW ADMINS", "no snmp-server enable traps snmp coldstart authentication", "no snmp-server location", "end"], 0);
    let run = e.running_config(R1);
    t.check(&format!("changed lines: {}", run), run.contains("snmp-server community ops RW ADMINS\nsnmp-server contact netz@lab.example\nsnmp-server enable traps snmp linkdown linkup\nsnmp-server host 10.0.0.50 version 2c public\nsnmp-server host 10.0.0.60 informs version 2c x\n!\n") && !run.contains("snmp-server location"));
    let o = cli(&mut e, R1, &["show snmp community"], 0);
    t.eq("show snmp community", o, s("Community         Access      Access list\npublic            read-only   -\nprivate           read-write  10\nops               read-write  ADMINS\n"));
    let o = cli(&mut e, R1, &["show snmp"], 0);
    t.check(&format!("show snmp: {}", o), o.starts_with("SNMP agent of R1 (versions 1 and 2c, UDP port 161), up for 0:00:00.00\nContact:  netz@lab.example\nLocation: -\nReceived 0 messages\n") && o.contains("Notifications: linkdown linkup\n  to 10.0.0.50 port 162: traps, version 2c, community public\n  to 10.0.0.60 port 162: informs, version 2c, community x\n"));
    // debug snmp packets: the agent's notes as console lines
    let o = cli(&mut e, R1, &["debug snmp packets", "show debugging"], 0);
    t.eq("debug on", o, s("Debug output for SNMP packets: on\nDebug output for SNMP packets: on\n"));
    let _ = e.take_reports();
    let _ = e.cli_line(MON, "snmpget -v2c -c public 10.0.0.1 sysName.0");
    run_for(&mut e, SEC);
    let r1 = out(&mut e, R1);
    t.check(&format!("debug lines: {}", r1), r1.contains(": SNMP: GetRequest from 10.0.0.50, community public, id ") && r1.contains(", 1 binding\n") && r1.contains(": SNMP: Response to 10.0.0.50, id "));
    let o = cli(&mut e, R1, &["undebug snmp packets"], 0);
    t.eq("debug off", o, s("Debug output for SNMP packets: off\n"));
    // no snmp-server: no agent, no lines
    let _ = cli(&mut e, R1, &["configure terminal", "no snmp-server", "end"], 0);
    t.check("gone", e.snmp_agent(R1).is_none() && !e.running_config(R1).contains("snmp-server"));
    let o = cli(&mut e, R1, &["show snmp"], 0);
    t.eq("no agent", o, s("%SNMP agent not enabled\n"));
    // a device without SNMP looks as before
    let plain = engine(&project(""));
    t.check("as before", !plain.running_config(R1).contains("snmp") && plain.snmp_agent(R1).is_none() && plain.services(R1).is_empty());
    t.eq("tab", e.cli_complete(MON, "snmpw").0, Some(s("snmpwalk ")));
}

// ---------------------------------------------------------------- problems

fn problems(t: &mut T) {
    let lines = format!("{}snmp-server community open RW\nsnmp-server host 192.168.9.9 version 2c public\n!\n", R1_SNMP);
    let e = engine(&project(&lines));
    let p: Vec<(ProblemKind, String)> = e.problems().into_iter().filter(|p| matches!(p.kind, ProblemKind::SnmpTrapHost | ProblemKind::SnmpOpenRw)).map(|p| (p.kind, p.text)).collect();
    t.eq("problems", p, alloc::vec![(ProblemKind::SnmpTrapHost, s("R1 schickt SNMP-Meldungen an 192.168.9.9, hat aber keinen Weg dorthin.")), (ProblemKind::SnmpOpenRw, s("Auf R1 darf jeder mit der Community „open“ schreiben: eine Zugriffsliste fehlt."))]);
    let ok = engine(&project(R1_SNMP));
    t.check("none with the list", !ok.problems().iter().any(|p| matches!(p.kind, ProblemKind::SnmpTrapHost | ProblemKind::SnmpOpenRw)));
}

// ---------------------------------------------------------------- the dissection

fn dissect(t: &mut T) {
    let mut e = engine(&project(R1_SNMP));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, MON, &["snmpget -v2c -c public 10.0.0.1 sysName.0"], SEC);
    let hop = e.trace.hops().filter(|h| h.mask & P_SNMP != 0 && h.from.0 == R1).last().cloned();
    let Some(h) = hop else {
        t.check("a response on the cable", false);
        return;
    };
    let (p, info) = decode::info(&h.bytes);
    t.check(&format!("summary: {}", info), p == Proto::Snmp && info.starts_with("Response ") && info.ends_with(": SNMPv2-MIB::sysName.0 = STRING: R1"));
    let ls = decode::layers(&h.bytes);
    t.check("fields", ls.iter().find(|l| l.proto == Proto::Snmp).is_some_and(|l| l.title == "Simple Network Management Protocol, v2c, Response" && l.fields.iter().any(|f| f.name == "Value" && f.value == "STRING: R1") && l.problems.is_empty()));
    t.check("event rows name it", e.trace.events().any(|x| matches!(&x.note, Note::Snmp(SNote::Request { kind: Kind::Get, .. }))));
    let msg = SNote::Request { from: ip([10, 0, 0, 50]), kind: Kind::Get, community: s("public"), id: 7, vars: 1 }.message();
    t.eq("a note in German", msg.text(), s("SNMP: GetRequest von 10.0.0.50 (Community „public“, Kennung 7, 1 Variablen)."));
    t.check("drops", SNote::BadCommunity { from: ip([1, 1, 1, 1]), community: s("x") }.is_drop() && !SNote::Set { var: s("a"), value: s("b") }.is_drop());
    t.check("footprint", e.snmp_agent(R1).is_some_and(|a| a.footprint() >= 512));
}

// ---------------------------------------------------------------- determinism

fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(R1_SNMP));
        run_for(&mut e, SEC);
        let _ = cli(&mut e, MON, &["snmpbulkwalk -v2c -c public 10.0.0.1", "snmpset -v2c -c private 10.0.0.1 sysLocation.0 s X"], 5 * SEC);
        let _ = e.apply(Action::Unplug { link: L_PC1, down: true });
        run_for(&mut e, 2 * SEC);
        let hops: Vec<(u64, u64, Vec<u8>)> = e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.bytes.to_vec())).collect();
        (hops, e.snmp_traps(MON), e.files(MON))
    };
    let (a, b) = (go(), go());
    t.check("twice the same", a == b && !a.0.is_empty() && !a.1.is_empty());
}
