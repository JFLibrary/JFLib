// modelstest -- the device models of wave 17 at work, not just present (group models of --selftest;
// jfn_model::catalog, files, wlan, jfn_sim): the ports, forms and leaves of each, a ping through
// each new switch with the speed of its ports in the round trip, routing across JF-S48Y and
// JF-S32C, a serial line on JF-R2, access lists and NAT on JF-FW2 and JF-FW8 and that they are
// only that so far, JF-S8 forwards and has no command line, the repeater repeats everything and
// carries a link past the 100 m of copper, the bridge learns and keeps a local frame on its side,
// a laptop on JF-AP7 at 6 GHz.
//
// Times: a 98-byte echo frame is (98 + 24) * 8 = 976 bits, so in whole ns 976 at 1G, 390 at 2.5G, 97
// at 10G, 39 at 25G and 9 at 100G, plus 50 ns of copper or 500 ns of fibre; 90 m of copper set by
// hand take 450 ns.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use jfn_model::catalog::{self, CliStyle, DeviceKind, Form, Media, Poe};
use jfn_model::cable::CableKind;
use jfn_model::files;
use jfn_model::ifname;
use jfn_model::phys;
use jfn_model::project::{PortRef, Project};
use jfn_model::wlan::{self as mw, Band, Security, Wlan, WlanJoin, WlanNet};
use jfn_sim::node::port_mac;
use jfn_sim::snmpmib;
use jfn_sim::{Engine, IcmpKind, JobKind, Note, PingEv, PingOpts, ProblemKind, Report, WNote, SEC};
use netpkt::ipv4::Ipv4View;
use netpkt::Ipv4;

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
/// the IPv4 ethertype in a frame
const ET_IP: [u8; 2] = [0x08, 0x00];

fn s(x: &str) -> String {
    String::from(x)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

fn ip(x: &str) -> Ipv4 {
    Ipv4::parse(x).unwrap_or(Ipv4::UNSPECIFIED)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

/// a device of `model` with a startup configuration, x apart per id
fn add(p: &mut Project, model: &str, cfg: &str) -> u32 {
    let x = p.devices.len() as i32 * 100;
    let id = p.add_device(model, x, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(id) {
        d.startup_config = s(cfg);
    }
    id
}

/// a host's startup lines: its address on an interface, a default route
fn host(iface: &str, addr: &str, gw: &str) -> String {
    format!("ip addr add {} dev {}\nip route add default via {}\n", addr, iface, gw)
}

/// `count` pings from a device, then time for all of them: what the job saw
fn ping(e: &mut Engine, dev: u32, dst: &str, count: u32) -> Vec<PingEv> {
    let id = e.ping(dev, ip(dst), PingOpts { count, ..PingOpts::default() }).unwrap_or(0);
    let until = e.now() + (count as u64 + 3) * SEC;
    e.run_until(until, BUDGET);
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

/// (replies, TTL and round trip of the last one)
fn answered(ev: &[PingEv]) -> (usize, Option<(u8, u64)>) {
    let r: Vec<(u8, u64)> = ev.iter().filter_map(|x| if let PingEv::Reply { ttl, rtt, .. } = x { Some((*ttl, *rtt)) } else { None }).collect();
    (r.len(), r.last().copied())
}

/// a line at a console, then a second of time: what it printed
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let until = e.now() + SEC;
    e.run_until(until, BUDGET);
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

/// the bridge's port behind a device's port now
fn learned(e: &Engine, sw: u32, dev: u32, index: u16) -> Option<u16> {
    e.bridge(sw).and_then(|b| b.lookup(1, port_mac(dev, index), e.now()))
}

/// the IPv4 sources of what went from one device to another
fn sources(e: &Engine, from: u32, to: u32) -> Vec<Ipv4> {
    e.trace.hops().filter(|h| h.from.0 == from && h.to.0 == to && h.bytes.get(12..14) == Some(&ET_IP[..])).filter_map(|h| Ipv4View::parse(&h.bytes[14..]).map(|v| v.src())).collect()
}

/// the IPv4 frames that went from one device to another
fn ip_frames(e: &Engine, from: u32, to: u32) -> usize {
    e.trace.hops().filter(|h| h.from.0 == from && h.to.0 == to && h.bytes.get(12..14) == Some(&ET_IP[..])).count()
}

pub(crate) fn run(t: &mut T) {
    table(t);
    defaults(t);
    switches(t);
    routing(t);
    serial(t);
    firewall(t, "jf-fw2", &FW2);
    firewall(t, "jf-fw8", &FW8);
    honest(t);
    unmanaged(t);
    repeater_length(t);
    shared_segment(t);
    access_point(t);
    files_and_suggestions(t);
}

// ---------------------------------------------------------------- what each model is

fn table(t: &mut T) {
    let rows: [(&str, DeviceKind, CliStyle, usize, &str, Form, u8, u32); 13] = [
        ("jf-s8", DeviceKind::Switch, CliStyle::None, 8, "8\u{d7} 1G", Form::Desktop, 0, 30),
        ("jf-s8p", DeviceKind::Switch, CliStyle::Ios, 11, "8\u{d7} 1G PoE+, 2\u{d7} 10G SFP+, Konsole", Form::Desktop, 0, 31),
        ("jf-s24", DeviceKind::Switch, CliStyle::Ios, 29, "24\u{d7} 1G, 4\u{d7} 10G SFP+, Konsole", Form::Rack, 1, 32),
        ("jf-s48p", DeviceKind::Switch, CliStyle::Ios, 53, "48\u{d7} 1G PoE+, 4\u{d7} 10G SFP+, Konsole", Form::Rack, 1, 33),
        ("jf-s24m", DeviceKind::Switch, CliStyle::Ios, 29, "24\u{d7} 2.5G PoE++, 4\u{d7} 25G SFP28, Konsole", Form::Rack, 1, 34),
        ("jf-s48y", DeviceKind::L3Switch, CliStyle::Ios, 57, "48\u{d7} 25G SFP28, 8\u{d7} 100G QSFP28, Konsole", Form::Rack, 1, 35),
        ("jf-s32c", DeviceKind::L3Switch, CliStyle::Ios, 33, "32\u{d7} 100G QSFP28, Konsole", Form::Rack, 1, 36),
        ("jf-r2", DeviceKind::Router, CliStyle::Ios, 5, "2\u{d7} 1G, 2\u{d7} Seriell, Konsole", Form::Desktop, 0, 37),
        ("repeater", DeviceKind::Hub, CliStyle::None, 2, "2\u{d7} 1G", Form::Desktop, 0, 38),
        ("bridge", DeviceKind::Switch, CliStyle::None, 2, "2\u{d7} 1G", Form::Desktop, 0, 39),
        ("jf-fw2", DeviceKind::Firewall, CliStyle::Ios, 6, "1\u{d7} 1G, 4\u{d7} 1G, Konsole", Form::Desktop, 0, 40),
        ("jf-fw8", DeviceKind::Firewall, CliStyle::Ios, 13, "8\u{d7} 10G SFP+, 4\u{d7} 100G QSFP28, Konsole", Form::Rack, 2, 41),
        ("jf-ap7", DeviceKind::AccessPoint, CliStyle::Ios, 5, "1\u{d7} 5G (PoE-gespeist), 1\u{d7} Funk 1.3G, 1\u{d7} Funk 5.7G, 1\u{d7} Funk 11.5G, Konsole", Form::Ceiling, 0, 42),
    ];
    for (id, kind, cli, ports, summary, form, units, leaf) in rows {
        let Some(m) = catalog::model(id) else {
            t.check(&format!("{}: in the catalog", id), false);
            continue;
        };
        t.eq(&format!("{}: kind and command line", id), (m.kind, m.cli), (kind, cli));
        t.eq(&format!("{}: ports", id), (m.port_count(), m.summary()), (ports, s(summary)));
        t.eq(&format!("{}: form", id), (catalog::form(m), catalog::rack_units(m)), (form, units));
        t.eq(&format!("{}: its leaf stays", id), snmpmib::MODEL_LEAVES.iter().find(|l| l.0 == id).map(|l| l.1), Some(leaf));
        t.eq(&format!("{}: console with a command line only", id), m.port("con").is_some(), cli == CliStyle::Ios);
    }
    let port = |id: &str, name: &str| catalog::model(id).and_then(|m| m.port(name));
    t.eq("short names", ["tw1/0/24", "twe1/1/4", "twe1/0/48", "hu1/0/56", "hu1/0/32", "se0/1/1", "fi0"].map(|n| ifname::expand(n).is_some()), [true; 7]);
    t.check("PoE++ on the multi-gigabit ports", port("jf-s24m", "tw1/0/1").is_some_and(|p| p.poe == Poe::PseBt && p.kbps == 2_500_000 && p.mdix));
    t.check("PoE+ on the access ports", port("jf-s48p", "gi1/0/48").is_some_and(|p| p.poe == Poe::Pse && p.kbps == 1_000_000));
    t.check("no PoE on JF-S24", port("jf-s24", "gi1/0/1").is_some_and(|p| p.poe == Poe::None));
    t.check("the leaf's cages", port("jf-s48y", "twe1/0/1").is_some_and(|p| p.media == Media::Sfp28) && port("jf-s48y", "hu1/0/49").is_some_and(|p| p.media == Media::Qsfp28));
    t.eq("the leaf's last port and the spine's", (port("jf-s48y", "hu1/0/56").map(|p| p.index), port("jf-s32c", "hu1/0/32").map(|p| p.index)), (Some(55), Some(31)));
    t.check("the AP is powered over its 5G port", port("jf-ap7", "fi0").is_some_and(|p| p.poe == Poe::Pd && p.kbps == 5_000_000 && p.name == "FiveGigabitEthernet0"));
    t.check("the firewall's LAN ports cross, its WAN port does not", port("jf-fw2", "gi0/1").is_some_and(|p| p.mdix) && port("jf-fw2", "gi0/0").is_some_and(|p| !p.mdix));
    t.eq("serial ports", port("jf-r2", "se0/1/0").map(|p| (p.media, p.kbps)), Some((Media::Serial, 2048)));
    t.eq("no port 3 on the bridge", port("bridge", "port2"), None);
    let services = |id: &str| catalog::model(id).map_or(0, snmpmib::sys_services);
    t.eq("sysServices", ["jf-s8p", "jf-s48y", "jf-s32c", "jf-r2", "jf-fw2", "jf-ap7"].map(services), [2, 6, 6, 4, 4, 2]);
}

// the first model of a kind stays what it was: new devices of older paths keep their model
fn defaults(t: &mut T) {
    let d = |k: DeviceKind| catalog::default_model(k).map(|m| m.id);
    t.eq("default switch", d(DeviceKind::Switch), Some("jf-s24p"));
    t.eq("default L3 switch", d(DeviceKind::L3Switch), Some("jf-s24l3"));
    t.eq("default router", d(DeviceKind::Router), Some("jf-r4"));
    t.eq("default hub", d(DeviceKind::Hub), Some("hub"));
    t.eq("default firewall", d(DeviceKind::Firewall), Some("jf-fw"));
    t.eq("default access point", d(DeviceKind::AccessPoint), Some("jf-ap6e"));
}

// ---------------------------------------------------------------- a ping through each switch

/// one end of a test line: the device, its port, the switch's port and that port's number
struct End {
    model: &'static str,
    port: &'static str,
    sw: &'static str,
    index: u16,
}

/// a host's or a router's startup configuration with an address in 10.0.0.0/24
fn end_cfg(e: &End, addr: &str) -> String {
    if catalog::model(e.model).is_some_and(|m| m.cli == CliStyle::Ios) {
        format!("interface {}\n ip address {} 255.255.255.0\n no shutdown\n!\nend\n", e.port, addr)
    } else {
        format!("ip addr add {}/24 dev {}\n", addr, e.port)
    }
}

// the switch of a model between two ends; `rtt` shows the speed of both of its ports: 10.0.0.1 and
// 10.0.0.2 answer with the time of two frames on each cable
fn through(t: &mut T, model: &str, a: End, b: End, rtt: u64) {
    let mut p = Project::new(model, 7);
    let sw = add(&mut p, model, "");
    let da = add(&mut p, a.model, &end_cfg(&a, "10.0.0.1"));
    let db = add(&mut p, b.model, &end_cfg(&b, "10.0.0.2"));
    t.check(&format!("{}: cables", model), p.connect(&pr(da, a.port), &pr(sw, a.sw), None).is_ok() && p.connect(&pr(db, b.port), &pr(sw, b.sw), None).is_ok());
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.check(&format!("{}: links up", model), e.view().links.iter().all(|l| l.up));
    // an IOS-like sender drops the packet it asks ARP for, so a line of three
    let ev = ping(&mut e, da, "10.0.0.2", 3);
    let ttl = if catalog::model(b.model).is_some_and(|m| m.cli == CliStyle::Ios) { 255 } else { 64 };
    t.eq(&format!("{}: the last answer", model), answered(&ev).1, Some((ttl, rtt)));
    let index = |m: &str, port: &str| catalog::model(m).and_then(|m| m.port(port)).map_or(0, |p| p.index);
    t.eq(&format!("{}: it learned both", model), (learned(&e, sw, da, index(a.model, a.port)), learned(&e, sw, db, index(b.model, b.port))), (Some(a.index), Some(b.index)));
}

fn switches(t: &mut T) {
    // 2 * (1026 + 1026): 1G on both sides
    through(t, "jf-s8", End { model: "pc", port: "eth0", sw: "port0", index: 0 }, End { model: "pc", port: "eth0", sw: "port7", index: 7 }, 4104);
    // 2 * (1026 + 597): 1G to the PC, 10G from a server's SFP28 in the switch's SFP+ cage
    const SFP: &str = "server-1u-sfp";
    through(t, "jf-s8p", End { model: "pc", port: "eth0", sw: "gi1/0/8", index: 7 }, End { model: SFP, port: "eth1", sw: "te1/1/2", index: 9 }, 3246);
    through(t, "jf-s24", End { model: "pc", port: "eth0", sw: "gi1/0/24", index: 23 }, End { model: SFP, port: "eth1", sw: "te1/1/4", index: 27 }, 3246);
    through(t, "jf-s48p", End { model: "pc", port: "eth0", sw: "gi1/0/48", index: 47 }, End { model: SFP, port: "eth1", sw: "te1/1/4", index: 51 }, 3246);
    // 2 * (440 + 539): the PC's 2.5G, 25G in the uplink
    through(t, "jf-s24m", End { model: "pc", port: "eth0", sw: "tw1/0/24", index: 23 }, End { model: SFP, port: "eth1", sw: "twe1/1/4", index: 27 }, 1958);
    // 2 * (539 + 509): 25G, then 100G to a core router
    through(t, "jf-s48y", End { model: SFP, port: "eth1", sw: "twe1/0/48", index: 47 }, End { model: "jf-r8", port: "HundredGigE0/1/0", sw: "hu1/0/56", index: 55 }, 2096);
    // 2 * (509 + 509): 100G on both sides
    through(t, "jf-s32c", End { model: "jf-r8", port: "HundredGigE0/1/0", sw: "hu1/0/1", index: 0 }, End { model: "jf-r8", port: "HundredGigE0/1/1", sw: "hu1/0/32", index: 31 }, 2036);
}

// ---------------------------------------------------------------- routing and the serial line

// leaf - spine - leaf: SRV1 on a routed port of LEAF1, SRV2 in VLAN 10 of LEAF2, static routes
fn routing(t: &mut T) {
    const LEAF1: &str = "hostname LEAF1\n!\nip routing\n!\ninterface TwentyFiveGigE1/0/1\n no switchport\n ip address 10.1.0.1 255.255.255.0\n no shutdown\n!\ninterface HundredGigE1/0/49\n no switchport\n ip address 10.2.0.1 255.255.255.252\n no shutdown\n!\nip route 0.0.0.0 0.0.0.0 10.2.0.2\n!\nend\n";
    const SPINE: &str = "hostname SPINE\n!\nip routing\n!\ninterface HundredGigE1/0/1\n no switchport\n ip address 10.2.0.2 255.255.255.252\n no shutdown\n!\ninterface HundredGigE1/0/32\n no switchport\n ip address 10.3.0.2 255.255.255.252\n no shutdown\n!\nip route 10.1.0.0 255.255.255.0 10.2.0.1\nip route 10.4.0.0 255.255.255.0 10.3.0.1\n!\nend\n";
    const LEAF2: &str = "hostname LEAF2\n!\nip routing\n!\nvlan 10\n!\ninterface TwentyFiveGigE1/0/1\n switchport access vlan 10\n switchport mode access\n!\ninterface HundredGigE1/0/56\n no switchport\n ip address 10.3.0.1 255.255.255.252\n no shutdown\n!\ninterface Vlan10\n ip address 10.4.0.1 255.255.255.0\n no shutdown\n!\nip route 0.0.0.0 0.0.0.0 10.3.0.2\n!\nend\n";
    let mut p = Project::new("fabric", 9);
    let srv1 = add(&mut p, "server-1u-sfp", &host("eth1", "10.1.0.10/24", "10.1.0.1"));
    let leaf1 = add(&mut p, "jf-s48y", LEAF1);
    let spine = add(&mut p, "jf-s32c", SPINE);
    let leaf2 = add(&mut p, "jf-s48y", LEAF2);
    let srv2 = add(&mut p, "server-1u-sfp", &host("eth1", "10.4.0.10/24", "10.4.0.1"));
    let ok = [
        p.connect(&pr(srv1, "eth1"), &pr(leaf1, "twe1/0/1"), None),
        p.connect(&pr(leaf1, "hu1/0/49"), &pr(spine, "hu1/0/1"), None),
        p.connect(&pr(spine, "hu1/0/32"), &pr(leaf2, "hu1/0/56"), None),
        p.connect(&pr(leaf2, "twe1/0/1"), &pr(srv2, "eth1"), None),
    ];
    t.check("fabric: cables", ok.iter().all(|r| r.is_ok()));
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    // the first answers fall to the ARP of the routers on the way
    let ev = ping(&mut e, srv1, "10.4.0.10", 6);
    let (n, last) = answered(&ev);
    t.check("fabric: answers", n >= 3);
    // three routers on the way, 64 - 3; 2 * (539 + 509 + 509 + 539)
    t.eq("fabric: three hops and the speeds", last, Some((61, 4192)));
    t.check("fabric: the request crosses the spine", ip_frames(&e, leaf1, spine) > 0 && ip_frames(&e, spine, leaf2) > 0 && ip_frames(&e, leaf2, srv2) > 0);
    let o = cli(&mut e, spine, "show ip route");
    t.check("fabric: the spine's routes", o.contains("10.1.0.0") && o.contains("10.4.0.0"));
    // one device, both: a switch port in VLAN 10 and a routed port
    t.check("fabric: LEAF2 keeps its VLAN port", e.bridge(leaf2).is_some_and(|b| b.ports[0].member) && e.bridge(leaf2).is_some_and(|b| !b.ports[55].member));
}

// two JF-R2 over 2048 kbit/s with a PC behind each; the same line as JF-R4's in l3test
fn serial(t: &mut T) {
    let r = |name: &str, lan: &str, se: &str, other: &str| {
        format!(
            "hostname {}\n!\ninterface GigabitEthernet0/0/1\n ip address {} 255.255.255.0\n no shutdown\n!\ninterface Serial0/1/0\n ip address {} 255.255.255.252\n no shutdown\n!\nip route 0.0.0.0 0.0.0.0 {}\n!\nend\n",
            name, lan, se, other
        )
    };
    let mut p = Project::new("wan", 14);
    let pc1 = add(&mut p, "pc", &host("eth0", "192.168.1.10/24", "192.168.1.1"));
    let r1 = add(&mut p, "jf-r2", &r("R1", "192.168.1.1", "10.0.12.1", "10.0.12.2"));
    let r2 = add(&mut p, "jf-r2", &r("R2", "192.168.2.1", "10.0.12.2", "10.0.12.1"));
    let pc2 = add(&mut p, "pc", &host("eth0", "192.168.2.10/24", "192.168.2.1"));
    let ok = [p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/1"), None), p.connect(&pr(r1, "se0/1/0"), &pr(r2, "Serial0/1/0"), None), p.connect(&pr(r2, "gi0/0/1"), &pr(pc2, "eth0"), None)];
    t.check("serial: cables", ok.iter().all(|r| r.is_ok()));
    t.eq("serial: the cable", p.links.get(1).map(|l| (l.cable, l.kbps)), Some((CableKind::Serial, 2048)));
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    // the line alone: 100-byte datagrams in 104 bytes of HDLC, 406250 ns plus 2 ms each way
    let id = e.ping(r1, ip("10.0.12.2"), PingOpts { count: 2, size: 72, ..PingOpts::default() }).unwrap_or(0);
    let until = e.now() + 4 * SEC;
    e.run_until(until, BUDGET);
    let ev = match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    };
    t.eq("serial: the line's time", answered(&ev), (2, Some((255, 4_812_500))));
    t.check("serial: HDLC on the line", e.trace.hops().filter(|h| h.from.0 == r1 && h.to.0 == r2).all(|h| h.bytes[..4] == [0x0F, 0, 8, 0]));
    // end to end, through both routers
    let ev = ping(&mut e, pc1, "192.168.2.10", 4);
    let (n, last) = answered(&ev);
    t.check("serial: PC to PC", n >= 2 && last.is_some_and(|(ttl, _)| ttl == 62));
    t.eq("serial: shown", cli(&mut e, r1, "show interfaces Serial0/1/0").contains("Serial0/1/0 is up"), true);
}

// ---------------------------------------------------------------- the firewalls

/// the network of a firewall test
struct FwNet {
    /// model and port of the hosts
    host: (&'static str, &'static str),
    /// the firewall's two LAN ports and its way out
    lan: [&'static str; 2],
    wan: &'static str,
    /// the provider's router, its port to the firewall and to EXT
    isp: (&'static str, &'static str, &'static str),
    /// the way out in the provider's configuration: its long names
    isp_names: [&'static str; 2],
}

const FW2: FwNet = FwNet {
    host: ("pc", "eth0"),
    lan: ["GigabitEthernet0/1", "GigabitEthernet0/2"],
    wan: "GigabitEthernet0/0",
    isp: ("jf-r4", "gi0/0/0", "gi0/0/1"),
    isp_names: ["GigabitEthernet0/0/0", "GigabitEthernet0/0/1"],
};

const FW8: FwNet = FwNet {
    host: ("server-1u-sfp", "eth1"),
    lan: ["TenGigabitEthernet0/0/0", "TenGigabitEthernet0/0/1"],
    wan: "HundredGigE0/1/0",
    isp: ("jf-r8", "hu0/1/0", "te0/0/0"),
    isp_names: ["HundredGigE0/1/0", "TenGigabitEthernet0/0/0"],
};

/// two LAN hosts behind the firewall, the provider's router and EXT behind it, which knows no way
/// to 192.168.0.0/16: only translated packets come back.  `rules`: extra lines of the firewall
fn fw_project(model: &str, n: &FwNet, nat: bool, rules: &str, list: &str) -> (Project, [u32; 5]) {
    let (inside, outside) = if nat { (" ip nat inside\n", " ip nat outside\n") } else { ("", "") };
    let translate = if nat { format!("ip nat inside source list 1 interface {} overload\naccess-list 1 permit 192.168.0.0 0.0.255.255\n", n.wan) } else { String::new() };
    let fw_cfg = format!(
        "hostname FW1\n!\ninterface {}\n ip address 192.168.1.1 255.255.255.0\n{} no shutdown\n!\ninterface {}\n ip address 192.168.2.1 255.255.255.0\n{}{} no shutdown\n!\ninterface {}\n ip address 203.0.113.1 255.255.255.0\n{} no shutdown\n!\nip route 0.0.0.0 0.0.0.0 203.0.113.254\n{}{}!\nend\n",
        n.lan[0], inside, n.lan[1], inside, rules, n.wan, outside, translate, list,
    );
    let isp_cfg = format!("hostname ISP\n!\ninterface {}\n ip address 203.0.113.254 255.255.255.0\n no shutdown\n!\ninterface {}\n ip address 198.51.100.1 255.255.255.0\n no shutdown\n!\nend\n", n.isp_names[0], n.isp_names[1]);
    let mut p = Project::new(model, 11);
    let pc1 = add(&mut p, n.host.0, &host(n.host.1, "192.168.1.10/24", "192.168.1.1"));
    let pc2 = add(&mut p, n.host.0, &host(n.host.1, "192.168.2.10/24", "192.168.2.1"));
    let fw = add(&mut p, model, &fw_cfg);
    let isp = add(&mut p, n.isp.0, &isp_cfg);
    let ext = add(&mut p, n.host.0, &host(n.host.1, "198.51.100.10/24", "198.51.100.1"));
    let _ = p.connect(&pr(pc1, n.host.1), &pr(fw, n.lan[0]), None);
    let _ = p.connect(&pr(pc2, n.host.1), &pr(fw, n.lan[1]), None);
    let _ = p.connect(&pr(fw, n.wan), &pr(isp, n.isp.1), None);
    let _ = p.connect(&pr(isp, n.isp.2), &pr(ext, n.host.1), None);
    (p, [pc1, pc2, fw, isp, ext])
}

// NAT: PC1 reaches EXT only under the firewall's address; an access list on the second LAN port
// stops PC2's pings with a prohibited answer and counts them
fn firewall(t: &mut T, model: &str, n: &FwNet) {
    let (p, [pc1, pc2, fw, isp, ext]) = fw_project(model, n, true, " ip access-group 101 in\n", "access-list 101 deny icmp any any echo\naccess-list 101 permit ip any any\n");
    t.eq(&format!("{}: the cables", model), p.links.len(), 4);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    let (replies, last) = answered(&ping(&mut e, pc1, "198.51.100.10", 4));
    t.check(&format!("{}: NAT answers", model), replies >= 2 && last.is_some_and(|(ttl, _)| ttl == 62));
    let seen = sources(&e, isp, ext);
    t.check(&format!("{}: EXT sees the firewall", model), !seen.is_empty() && seen.iter().all(|&a| a == ip("203.0.113.1")));
    let inside = sources(&e, pc1, fw);
    t.check(&format!("{}: the private address stays inside", model), !inside.is_empty() && inside.iter().all(|&a| a == ip("192.168.1.10")));
    let _ = cli(&mut e, fw, "enable");
    let table = cli(&mut e, fw, "show ip nat translations");
    t.check(&format!("{}: the translation", model), table.contains("203.0.113.1") && table.contains("192.168.1.10"));
    // the list on the second LAN port
    let ev = ping(&mut e, pc2, "198.51.100.10", 1);
    t.eq(&format!("{}: the list stops PC2", model), ev.first().cloned(), Some(PingEv::Error { seq: 1, from: ip("192.168.2.1"), kind: IcmpKind::Prohibited }));
    t.eq(&format!("{}: nothing of PC2's left", model), sources(&e, fw, isp).iter().all(|&a| a == ip("203.0.113.1")), true);
    let acl = cli(&mut e, fw, "show access-lists");
    t.check(&format!("{}: the list counts", model), acl.contains("deny icmp any any echo") && acl.contains("(1 match)"));
}

// what a firewall is not yet: without a list it lets traffic through in both directions, also
// what starts outside, as a router does; zones and stateful filtering come with milestone F2
fn honest(t: &mut T) {
    for (model, n) in [("jf-fw2", &FW2), ("jf-fw8", &FW8), ("jf-fw", &FW2)] {
        let m = catalog::model(model);
        t.check(&format!("{}: its title says what it does", model), m.is_some_and(|m| m.title.contains("Router mit ACLs und NAT") && !m.title.contains("Next-Generation")));
        let (mut p, [pc1, _, fw, isp, ext]) = fw_project(model, n, false, "", "");
        // the provider knows the way in: a plain routed network
        if let Some(d) = p.device_mut(isp) {
            d.startup_config = d.startup_config.replace("end\n", "ip route 192.168.0.0 255.255.0.0 203.0.113.1\n!\nend\n");
        }
        let mut e = engine(&p);
        e.run_until(SEC, BUDGET);
        let (n_in, _) = answered(&ping(&mut e, ext, "192.168.1.10", 4));
        t.check(&format!("{}: from outside it forwards as a router does", model), n_in >= 2);
        let (n_out, _) = answered(&ping(&mut e, pc1, "198.51.100.10", 4));
        t.check(&format!("{}: from inside as well", model), n_out >= 2);
        t.check(&format!("{}: the source stays", model), sources(&e, fw, isp).contains(&ip("192.168.1.10")));
    }
}

// ---------------------------------------------------------------- without a command line

fn unmanaged(t: &mut T) {
    let mut p = Project::new("desk", 3);
    let pc1 = add(&mut p, "pc", "ip addr add 10.0.0.1/24 dev eth0\n");
    let pc2 = add(&mut p, "pc", "ip addr add 10.0.0.2/24 dev eth0\n");
    let pc3 = add(&mut p, "pc", "ip addr add 10.0.0.3/24 dev eth0\n");
    let sw = add(&mut p, "jf-s8", "");
    t.eq("JF-S8: names like the hub's", p.suggest_ports(pc1, sw), Some((s("eth0"), s("port0"))));
    for (i, d) in [pc1, pc2, pc3].iter().enumerate() {
        let _ = p.connect(&pr(*d, "eth0"), &pr(sw, &format!("port{}", i)), None);
    }
    t.eq("JF-S8: a straight cable to a PC", p.links.first().map(|l| l.cable), Some(CableKind::CopperStraight));
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    let ev = ping(&mut e, pc1, "10.0.0.2", 2);
    t.eq("JF-S8: forwards", answered(&ev), (2, Some((64, 4104))));
    t.eq("JF-S8: learned", (learned(&e, sw, pc1, 0), learned(&e, sw, pc2, 0)), (Some(0), Some(1)));
    // a known address goes to one port: PC3 saw the ARP request and nothing else
    t.eq("JF-S8: the echoes pass PC3", ip_frames(&e, sw, pc3), 0);
    t.check("JF-S8: no command line", matches!(e.cli_line(sw, "enable"), Err(jfn_sim::ActErr::Refused(_))));
    t.check("JF-S8: no address of its own", e.ifaces(sw).is_empty());
    // no STP to offer: a circle of two is a problem
    let mut q = Project::new("loop", 3);
    let a = add(&mut q, "jf-s8", "");
    let b = add(&mut q, "jf-s8", "");
    let _ = q.connect(&pr(a, "port0"), &pr(b, "port0"), None);
    let _ = q.connect(&pr(a, "port1"), &pr(b, "port1"), None);
    let mut e = engine(&q);
    e.run_until(SEC, BUDGET);
    t.check("JF-S8: a loop is a problem", e.problems().iter().any(|x| x.kind == ProblemKind::Loop));
}

// a PC a metre apart from a PC, by hand: a cable of `metres` or two of them through a repeater
fn repeater_length(t: &mut T) {
    // 180 m in one cable: too long for 1000BASE-T
    let mut p = Project::new("long", 5);
    let pc1 = add(&mut p, "pc", "ip addr add 10.0.0.1/24 dev eth0\n");
    let pc2 = add(&mut p, "pc", "ip addr add 10.0.0.2/24 dev eth0\n");
    let l = p.connect(&pr(pc1, "eth0"), &pr(pc2, "eth0"), None).unwrap_or(0);
    let _ = phys::set_len(&mut p, l, 180);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.check("one cable of 180 m stays down", e.view().links.iter().all(|x| !x.up));
    t.check("one cable of 180 m: why", e.cable_phys(l).is_some_and(|c| matches!(c.fault, Some(jfn_sim::PhysFault::TooLong { len_m: 180, max_m: 100, .. }))));
    t.eq("one cable of 180 m: no answer", answered(&ping(&mut e, pc1, "10.0.0.2", 1)).0, 0);
    // twice 90 m through a repeater: up, and the 180 m are in the round trip
    let two = |metres: u32| {
        let mut p = Project::new("long", 5);
        let pc1 = add(&mut p, "pc", "ip addr add 10.0.0.1/24 dev eth0\n");
        let rep = add(&mut p, "repeater", "");
        let pc2 = add(&mut p, "pc", "ip addr add 10.0.0.2/24 dev eth0\n");
        let a = p.connect(&pr(pc1, "eth0"), &pr(rep, "port0"), None).unwrap_or(0);
        let b = p.connect(&pr(rep, "port1"), &pr(pc2, "eth0"), None).unwrap_or(0);
        let _ = phys::set_len(&mut p, a, metres);
        let _ = phys::set_len(&mut p, b, metres);
        (p, pc1, rep)
    };
    let (p, pc1, rep) = two(90);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.check("twice 90 m through a repeater: up", e.view().links.iter().all(|x| x.up));
    // four frames of 976 ns on cables of 450 ns
    t.eq("twice 90 m: the time", answered(&ping(&mut e, pc1, "10.0.0.2", 2)), (2, Some((64, 4 * (976 + 450)))));
    t.check("the repeater repeats", e.trace.events().any(|x| x.dev == rep && x.note == Note::Repeated { ports: 1 }));
    t.check("the repeater has no command line", matches!(e.cli_line(rep, "enable"), Err(jfn_sim::ActErr::Refused(_))) && e.ifaces(rep).is_empty());
    // the limit is per cable: it extends a run, it does not lift the limit
    let (p, _, _) = two(120);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.check("twice 120 m stay down", e.view().links.iter().all(|x| !x.up));
}

// PC1 and PC2 on a hub, a box between the hub and PC3: PC1 first reaches PC3, then PC2
fn segment(model: &str) -> (Engine, [u32; 4], usize) {
    let mut p = Project::new(model, 6);
    let pc1 = add(&mut p, "pc", "ip addr add 10.0.0.1/24 dev eth0\n");
    let pc2 = add(&mut p, "pc", "ip addr add 10.0.0.2/24 dev eth0\n");
    let hub = add(&mut p, "hub", "");
    let bx = add(&mut p, model, "");
    let pc3 = add(&mut p, "pc", "ip addr add 10.0.0.3/24 dev eth0\n");
    let _ = p.connect(&pr(pc1, "eth0"), &pr(hub, "port0"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(hub, "port1"), None);
    let _ = p.connect(&pr(hub, "port7"), &pr(bx, "port0"), None);
    let _ = p.connect(&pr(bx, "port1"), &pr(pc3, "eth0"), None);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    let _ = ping(&mut e, pc1, "10.0.0.3", 1);
    let far = ip_frames(&e, bx, pc3);
    (e, [pc1, pc2, bx, pc3], far)
}

// what tells layer 1 from layer 2: a frame between PC1 and PC2 reaches PC3 through the repeater and
// stays on its side of the bridge, which has learned where PC1 and PC2 are
fn shared_segment(t: &mut T) {
    // the repeater: every frame, also what is not for PC3
    let (mut e, [pc1, _, rep, pc3], far) = segment("repeater");
    t.eq("repeater: PC3 is reached", far, 1);
    let _ = ping(&mut e, pc1, "10.0.0.2", 2);
    t.eq("repeater: the echoes of the others reach PC3", ip_frames(&e, rep, pc3) - far, 4);
    t.check("repeater: PC3 throws them away", e.trace.events().any(|x| x.dev == pc3 && x.note == Note::NotForMe { port: 0 }));
    t.eq("repeater: no table of its own", e.bridge(rep).is_none(), true);
    // the bridge
    let (mut e, [pc1, pc2, br, pc3], far) = segment("bridge");
    t.eq("bridge: PC3 is reached", far, 1);
    t.eq("bridge: learned both sides", (learned(&e, br, pc1, 0), learned(&e, br, pc3, 0)), (Some(0), Some(1)));
    let _ = ping(&mut e, pc1, "10.0.0.2", 2);
    t.eq("bridge: PC2 on PC1's side", learned(&e, br, pc2, 0), Some(0));
    t.eq("bridge: their echoes stay on their side", ip_frames(&e, br, pc3) - far, 0);
    t.check("bridge: it filters them", e.trace.events().any(|x| x.dev == br && matches!(x.note, Note::Filtered { .. })));
    t.check("bridge: no command line", matches!(e.cli_line(br, "enable"), Err(jfn_sim::ActErr::Refused(_))) && e.ifaces(br).is_empty());
    t.eq("bridge: a broadcast still crosses", e.trace.hops().filter(|h| h.from.0 == br && h.to.0 == pc3 && h.bytes.first() == Some(&0xff)).count() >= 2, true);
}

// ---------------------------------------------------------------- the access point

fn access_point(t: &mut T) {
    let (Some(ap), Some(ap6)) = (catalog::model("jf-ap7"), catalog::model("jf-ap6e")) else {
        t.check("models", false);
        return;
    };
    t.eq("JF-AP7: role and bands", (mw::role(ap), mw::ap_bands(ap)), (mw::Role::Ap, mw::B_24 | mw::B_5 | mw::B_6));
    const BANDS: [Band; 3] = [Band::G24, Band::G5, Band::G6];
    t.eq("JF-AP7: as far as JF-AP6E", BANDS.map(|b| mw::range_m(ap, b)), BANDS.map(|b| mw::range_m(ap6, b)));
    t.eq("JF-AP7: a radio per band", BANDS.map(|b| mw::radio_port(ap, b)), [Some(s("Dot11Radio0")), Some(s("Dot11Radio1")), Some(s("Dot11Radio2"))]);
    t.eq("JF-AP7: 802.11be speeds", ["Dot11Radio0", "Dot11Radio1", "Dot11Radio2"].map(|n| ap.port(n).map_or(0, |p| p.kbps)), [1_376_000, 5_765_000, 11_530_000]);
    // a laptop 3 m away on a WPA3 network at 6 GHz, the access point on a switch with a PC
    let mut p = Project::new("wifi", 5);
    let ap = add(&mut p, "jf-ap7", "hostname AP1\n");
    let nb = add(&mut p, "laptop", "hostname Laptop1\nip addr add 10.0.0.2/24 dev wlan0\n");
    let sw = add(&mut p, "jf-s24m", "hostname SW1\n");
    let pc = add(&mut p, "pc", "ip addr add 10.0.0.3/24 dev eth0\n");
    if let Some(d) = p.device_mut(ap) {
        d.wlan = Some(Wlan { nets: vec![WlanNet { ssid: s("Labor"), band: Band::G6, channel: 0, security: Security::Wpa3, passphrase: s("Sommer2026!"), ..WlanNet::default() }], join: None });
    }
    if let Some(d) = p.device_mut(nb) {
        d.x = 30;
        d.wlan = Some(Wlan { nets: Vec::new(), join: Some(WlanJoin { ssid: s("Labor"), passphrase: s("Sommer2026!"), ..WlanJoin::default() }) });
    }
    t.check("JF-AP7: on a switch", p.connect(&pr(ap, "fi0"), &pr(sw, "tw1/0/1"), None).is_ok() && p.connect(&pr(pc, "eth0"), &pr(sw, "tw1/0/2"), None).is_ok());
    t.eq("JF-AP7: 2.5G on the switch's port", p.links.first().map(|l| l.kbps), Some(2_500_000));
    let mut e = engine(&p);
    e.run_until(2 * SEC, BUDGET);
    let assoc: Vec<(Band, u32)> = e.trace.events().filter(|x| x.dev == nb).filter_map(|x| if let Note::Wlan(WNote::Assoc { band, kbps, .. }) = &x.note { Some((*band, *kbps)) } else { None }).collect();
    // the laptop is 802.11ax: its radio sets the rate, not the access point's: 3 m away on 6 GHz
    // the signal gives 80 % of its 2.4 Gbit/s
    t.eq("JF-AP7: the laptop joins at 6 GHz", assoc, vec![(Band::G6, 1_920_000)]);
    t.check("JF-AP7: it knows the laptop", e.trace.events().any(|x| x.dev == ap && matches!(&x.note, Note::Wlan(WNote::ApAssoc { sta, .. }) if sta == "Laptop1")));
    let (n, _) = answered(&ping(&mut e, nb, "10.0.0.3", 2));
    t.eq("JF-AP7: a ping over the air and the cable", n, 2);
    let o = cli(&mut e, ap, "show interfaces FiveGigabitEthernet0");
    t.check("JF-AP7: its port as IOS calls it", o.contains("Five Gigabit Ethernet"));
}

// ---------------------------------------------------------------- around them

fn files_and_suggestions(t: &mut T) {
    let cap = |id: &str| catalog::model(id).map_or(1, files::capacity_of);
    t.eq("no files without a command line", ["jf-s8", "bridge", "repeater"].map(cap), [0, 0, 0]);
    t.check("files on the managed ones", ["jf-s8p", "jf-s48y", "jf-r2", "jf-fw2", "jf-ap7"].iter().all(|&i| cap(i) > 0));
    let image = |id: &str| catalog::model(id).and_then(files::image_of).map(|i| i.0);
    t.eq("system images", (image("jf-s32c"), image("jf-s8"), image("bridge")), (Some(s("jf-s32c.bin")), None, None));
    let mut p = Project::new("fit", 4);
    let leaf = p.add_device("jf-s48y", 0, 0).unwrap_or(0);
    let spine = p.add_device("jf-s32c", 100, 0).unwrap_or(0);
    let ap = p.add_device("jf-ap7", 200, 0).unwrap_or(0);
    let nb = p.add_device("laptop", 300, 0).unwrap_or(0);
    t.eq("the fastest pair between leaf and spine", p.suggest_ports(leaf, spine), Some((s("HundredGigE1/0/49"), s("HundredGigE1/0/1"))));
    t.eq("a laptop's radio on JF-AP7", p.suggest_ports(nb, ap), Some((s("wlan0"), s("Dot11Radio1"))));
    // every new model in a file and back
    let mut q = Project::new("all", 2);
    for id in ["jf-s8", "jf-s8p", "jf-s24", "jf-s48p", "jf-s24m", "jf-s48y", "jf-s32c", "jf-r2", "repeater", "bridge", "jf-fw2", "jf-fw8", "jf-ap7"] {
        t.check(&format!("{}: placed", id), q.add_device(id, 0, 0).is_ok());
    }
    t.eq("the new models in a file", Project::from_text(&q.to_text()), Ok(q.clone()));
    t.eq("their names", q.devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), vec!["SW1", "SW2", "SW3", "SW4", "SW5", "Leaf1", "Spine1", "R1", "Rep1", "Br1", "FW1", "FW2", "AP1"]);
    t.eq("consistent", q.check(), Vec::new());
    let mut e = engine(&q);
    e.run_until(SEC, BUDGET);
    t.eq("all of them run", e.devices().count(), 13);
}
