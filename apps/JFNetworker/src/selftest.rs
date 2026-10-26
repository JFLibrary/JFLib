// selftest -- checks of jfn_model and jfn_sim (JFNetworker --selftest, run by
// tests/model.uis): the catalog, IOS interface names, cable rules, project edits with undo,
// the file format, scenario scripts and (simtest.rs, l3test.rs, simmodetest.rs, svctest.rs,
// clitest.rs, tcptest.rs, httptest.rs, pdutest.rs, labtest.rs) the simulator, its command
// lines, TCP, HTTP, the PDUs of the simulation mode and the distributed-systems lab;
// synthtest.rs the synthetic projects of performance tests; consoletest.rs what the consoles
// build on (log reports, saved configurations through undo and redo); campustest.rs the
// campus of the acceptance (tests/fixtures/campus.jfnet); uitest.rs what U5 adds around the
// window (contrast, settings, autosave, the English catalogue).  Prints "FAIL group: name"
// lines and a summary; exit code 1 on a failure.
//
// msgtest.rs: the texts of model and simulator as messages (jfn_model::msg) and their English.
// scripttest.rs: scripts as services in the hosts (group script, jfn_sim::scriptsvc).
// seriestest.rs: the time series of the lab (jfn_sim::series), the windowed expectations of
// scenarios.
// tasktest.rs: the labs and their tasks (group task: jfn_model::lab, jfn_sim::goal, the labs of
// assets/labs unsolved and solved).
// scriptedtest.rs: what the script editor builds on (group scripted: doc/scripts.rs, the steps
// that keep a running script's disk through undo and redo, keep_steps, the marks).
// labuitest.rs: what the lab in the window builds on (group labui: the lab's texts as
// messages, KPIs of past spans, the Metriken panel's tables, faults as undo steps).
// stptest.rs: spanning tree on the switches (group stp: STP and RSTP, jfn_sim::stp).
// riptest.rs: RIP on routers and L3 switches (group rip: jfn_sim::rip, cli/rip.rs).
// nattest.rs: access lists and NAT (group nat: jfn_sim::acl, jfn_sim::nat, their console).
// ipv6test.rs: IPv6 (group ipv6: jfn_sim::ip6, nd, l3v6, jobs6, their consoles).
// ospftest.rs: OSPF version 2 on routers and L3 switches (group ospf: jfn_sim::ospf, cli/ospf.rs).
// ipv6tcptest.rs: UDP and TCP over IPv6 and what uses them (group ipv6tcp: jfn_sim::os, tcp,
// resolver, the services, cli/tools.rs, cli/names.rs).
// assesstest.rs: assessments (group assess: jfn_model::lab's Assess and items, jfn_sim::assess,
// the reach expectation, the Aufgabe panel's run, authoring, the samples of assets/assess).
// wlantest.rs: WLAN (group wlan: jfn_model::wlan, jfn_sim::wlan, wlankey, wlantrace, cli/wlan.rs,
// doc/wlan.rs).
// phystest.rs: the physical view (group phys: jfn_model::phys, the forms and optics of the
// catalogue, jfn_sim::phys, doc/phys.rs, the scenario steps place / cable and expect cable).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use agent_core::json::{self, Limits};
use jfn_model::cable::{self, CableKind, Why};
use jfn_model::catalog::{self, Category, DeviceKind, Media, PortSpec, F_ANNOTATION};
use jfn_model::ifname;
use jfn_model::project::{PortRef, ProjErr, Project, ServiceDef};
use jfn_model::scenario::{self, Cmp, Expect, StepKind, Via};
use netpkt::{Ipv4, Prefix4};

pub(crate) struct T {
    group: &'static str,
    ok: u32,
    failed: u32,
}

impl T {
    pub(crate) fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}\n", self.group, name).as_bytes());
        }
    }
    pub(crate) fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
}

#[path = "msgtest.rs"]
mod msgtest;
#[path = "scriptedtest.rs"]
mod scriptedtest;
#[path = "labuitest.rs"]
mod labuitest;
#[path = "nattest.rs"]
mod nattest;
#[path = "ipv6test.rs"]
mod ipv6test;
#[path = "ospftest.rs"]
mod ospftest;
#[path = "ipv6tcptest.rs"]
mod ipv6tcptest;
#[path = "assesstest.rs"]
mod assesstest;
#[path = "wlantest.rs"]
mod wlantest;
#[path = "phystest.rs"]
mod phystest;

pub fn run() -> i32 {
    let groups: [(&'static str, fn(&mut T)); 34] = [
        ("catalog", catalog_t),
        ("ifname", ifname_t),
        ("cable", cable_t),
        ("project", project_t),
        ("file", file_t),
        ("scenario", scenario_t),
        ("sim", crate::simtest::run),
        ("l3", crate::simtest::l3test::run),
        ("simmode", crate::simtest::simmodetest::run),
        ("svc", crate::simtest::svctest::run),
        ("cli", crate::simtest::clitest::run),
        ("tcp", crate::simtest::tcptest::run),
        ("http", crate::simtest::httptest::run),
        ("console", crate::consoletest::run),
        ("pdu", crate::simtest::pdutest::run),
        ("lab", crate::simtest::labtest::run),
        ("ui", crate::uitest::run),
        ("msg", msgtest::run),
        ("campus", crate::simtest::campustest::run),
        ("synth", crate::simtest::synthtest::run),
        ("script", crate::simtest::scripttest::run),
        ("series", crate::simtest::seriestest::run),
        ("task", crate::simtest::tasktest::run),
        ("scripted", scriptedtest::run),
        ("labui", labuitest::run),
        ("stp", crate::simtest::stptest::run),
        ("rip", crate::simtest::riptest::run),
        ("nat", nattest::run),
        ("ipv6", ipv6test::run),
        ("ospf", ospftest::run),
        ("ipv6tcp", ipv6tcptest::run),
        ("assess", assesstest::run),
        ("wlan", wlantest::run),
        ("phys", phystest::run),
    ];
    let (mut total, mut failed) = (0, 0);
    for (name, f) in groups {
        let mut t = T { group: name, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed).as_bytes());
        total += t.ok + t.failed;
        failed += t.failed;
    }
    plat::write_stdout(format!("JFNetworker selftest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 { 0 } else { 1 }
}

fn port(model: &str, name: &str) -> PortSpec {
    catalog::model(model).and_then(|m| m.port(name)).unwrap_or(PortSpec {
        name: String::from("?"),
        index: 0,
        media: Media::Usb,
        kbps: 0,
        mdix: false,
        poe: catalog::Poe::None,
        optic: None,
    })
}

fn catalog_t(t: &mut T) {
    let all = catalog::catalog();
    let mut ids: Vec<&str> = all.iter().map(|m| m.id).collect();
    ids.sort();
    ids.dedup();
    t.eq("unique ids", ids.len(), all.len());
    let mut names: Vec<&str> = all.iter().map(|m| m.name).collect();
    names.sort();
    names.dedup();
    t.eq("unique names", names.len(), all.len());
    for m in all {
        let ports = m.ports();
        let mut pn: Vec<String> = ports.iter().map(|p| p.name.to_ascii_lowercase()).collect();
        pn.sort();
        pn.dedup();
        t.eq(&format!("{}: unique port names", m.id), pn.len(), ports.len());
        t.check(&format!("{}: ports unless a note", m.id), m.has(F_ANNOTATION) == ports.is_empty());
        t.check(&format!("{}: symbol and prefix", m.id), !m.symbol.is_empty() && !m.name_prefix.is_empty());
        t.check(&format!("{}: indexes", m.id), ports.iter().enumerate().all(|(i, p)| p.index as usize == i));
    }
    for c in Category::ALL {
        t.check(&format!("category {} used", c.title()), catalog::in_category(c).next().is_some());
    }
    let sw = catalog::model("JF-S24P");
    t.check("model lookup ignores case", sw.is_some_and(|m| m.id == "jf-s24p"));
    if let Some(sw) = sw {
        let p = sw.ports();
        t.eq("switch port count", (p.len(), sw.port_count()), (29, 29));
        t.eq("switch first/last access", (p[0].name.as_str(), p[23].name.as_str()), ("GigabitEthernet1/0/1", "GigabitEthernet1/0/24"));
        t.eq("switch uplinks", (p[24].name.as_str(), p[27].name.as_str()), ("TenGigabitEthernet1/1/1", "TenGigabitEthernet1/1/4"));
        t.eq("switch console", p[28].name.as_str(), "Console");
        t.eq("port by abbreviation", sw.port("gi1/0/5").map(|p| p.name), Some(String::from("GigabitEthernet1/0/5")));
        t.eq("port by short name", sw.port("Te1/1/4").map(|p| p.index), Some(27));
        t.eq("no such port", sw.port("gi1/0/25"), None);
        t.check("console port", sw.port("con").is_some_and(|p| p.media == Media::Console));
        t.eq("summary", sw.summary(), String::from("24\u{d7} 1G PoE+, 4\u{d7} 10G SFP+, Konsole"));
        t.check("access ports are MDI-X with PoE", p[0].mdix && p[0].poe == catalog::Poe::Pse);
    }
    t.eq("linux port name", catalog::model("pc").and_then(|m| m.port("ETH0")).map(|p| p.name), Some(String::from("eth0")));
    t.eq("speeds", [catalog::speed_str(1_000_000), catalog::speed_str(2_500_000), catalog::speed_str(100_000), catalog::speed_str(2048), catalog::speed_str(115)].join(" "), String::from("1G 2.5G 100M 2 Mbit/s 115 kbit/s"));
    t.eq("default router", catalog::default_model(DeviceKind::Router).map(|m| m.id), Some("jf-r4"));
    t.check("kind names round trip", DeviceKind::ALL.iter().all(|k| DeviceKind::parse(k.as_str()) == Some(*k)));
    t.eq("kind parse", DeviceKind::parse("l3-switch"), Some(DeviceKind::L3Switch));
}

fn ifname_t(t: &mut T) {
    t.eq("short", ifname::short("GigabitEthernet1/0/1"), String::from("Gi1/0/1"));
    t.eq("short hundred", ifname::short("HundredGigE1/0/49"), String::from("Hu1/0/49"));
    t.eq("short linux", ifname::short("eth0"), String::from("eth0"));
    for s in ["gi1/0/1", "g1/0/1", "Gig 1/0/1", "GIGABITETHERNET1/0/1", "Gi1/0/1"] {
        t.eq(&format!("expand {}", s), ifname::expand(s), Some(String::from("GigabitEthernet1/0/1")));
    }
    t.eq("ambiguous t", ifname::expand("t1/0/1"), None);
    t.eq("ambiguous f", ifname::expand("f0/1"), None);
    t.eq("te", ifname::expand("te1/1/1"), Some(String::from("TenGigabitEthernet1/1/1")));
    t.eq("tw is two", ifname::expand("tw0"), Some(String::from("TwoGigabitEthernet0")));
    t.eq("twe is twenty-five", ifname::expand("twe1/0/1"), Some(String::from("TwentyFiveGigE1/0/1")));
    t.eq("fa", ifname::expand("fa0/1"), Some(String::from("FastEthernet0/1")));
    t.eq("serial", ifname::expand("se0/2/0"), Some(String::from("Serial0/2/0")));
    t.eq("console", ifname::expand("con"), Some(String::from("Console")));
    t.eq("port-channel", ifname::expand("po1"), Some(String::from("Port-channel1")));
    t.eq("vlan", ifname::expand("vl10"), Some(String::from("Vlan10")));
    t.eq("not ios", ifname::expand("wlan0"), None);
    t.check("same", ifname::same("Gi1/0/1", "GigabitEthernet1/0/1") && ifname::same("eth0", "ETH0"));
    t.check("not same", !ifname::same("gi1/0/1", "gi1/0/2"));
}

fn cable_t(t: &mut T) {
    let pc = port("pc", "eth0");
    let sw = port("jf-s24p", "gi1/0/1");
    let sw_up = port("jf-s24p", "te1/1/1");
    let r = port("jf-r4", "gi0/0/0");
    let r_te = port("jf-r4", "te0/1/0");
    t.eq("pc-switch", cable::cable_kind(&pc, &sw), Ok(CableKind::CopperStraight));
    t.eq("switch-switch", cable::cable_kind(&sw, &sw), Ok(CableKind::CopperCross));
    t.eq("pc-pc", cable::cable_kind(&pc, &pc), Ok(CableKind::CopperCross));
    t.eq("pc-router", cable::cable_kind(&pc, &r), Ok(CableKind::CopperCross));
    t.eq("router-switch", cable::cable_kind(&r, &sw), Ok(CableKind::CopperStraight));
    t.eq("sfp+ pair", cable::cable_kind(&sw_up, &r_te), Ok(CableKind::FiberMm));
    let hu = port("jf-s48x", "hu1/0/49");
    t.eq("qsfp28 pair", cable::cable_kind(&hu, &port("jf-r8", "hu0/1/0")), Ok(CableKind::FiberMm));
    t.eq("qsfp28 vs sfp+", cable::cable_kind(&hu, &sw_up), Err(Why::MediaMismatch(Media::Qsfp28, Media::SfpPlus)));
    let s28 = port("server-1u-sfp", "eth1");
    t.eq("sfp28 into sfp+", cable::cable_kind(&s28, &sw_up), Ok(CableKind::FiberMm));
    t.eq("sfp28 into sfp+ speed", cable::link_kbps(&s28, &sw_up), 10_000_000);
    t.eq("copper vs sfp+", cable::cable_kind(&pc, &sw_up), Err(Why::MediaMismatch(Media::Rj45, Media::SfpPlus)));
    t.eq("pc-switch speed", cable::link_kbps(&pc, &sw), 1_000_000);
    t.eq("serial", cable::cable_kind(&port("jf-r4", "se0/2/0"), &port("jf-r4", "se0/2/1")), Ok(CableKind::Serial));
    t.eq("console", cable::cable_kind(&port("jf-r4", "console"), &port("laptop", "usb0")), Ok(CableKind::Console));
    let wl = port("laptop", "wlan0");
    t.eq("wireless", cable::cable_kind(&wl, &port("jf-ap6e", "Dot11Radio1")), Ok(CableKind::Wireless));
    t.eq("no ad hoc", cable::cable_kind(&wl, &port("smartphone", "wlan0")), Err(Why::NoAccessPoint));
    t.eq("wan", cable::cable_kind(&port("modem-ont", "wan0"), &port("cloud", "wan0")), Ok(CableKind::Wan));
    t.eq("crossover fits", cable::cable_fits(CableKind::CopperCross, &pc, &sw), Ok(()));
    t.eq("fiber does not fit copper", cable::cable_fits(CableKind::FiberMm, &pc, &sw), Err(Why::CableDoesNotFit(CableKind::FiberMm, Media::Rj45)));
    t.eq("singlemode fits optics", cable::cable_fits(CableKind::FiberSm, &sw_up, &r_te), Ok(()));
    t.check("reasons are German", format!("{}", Why::NoAccessPoint).contains("Access Point"));
    t.eq("copper delay", cable::default_delay_ns(CableKind::CopperStraight), 50);
    t.eq("copper max length", CableKind::CopperCross.max_len_m(1_000_000), Some(100));
    t.eq("cable wire names", CableKind::parse("fiber-mm"), Some(CableKind::FiberMm));
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

/// a small project: PC1, PC2, SW1 (PC1 eth0 - SW1 Gi1/0/1)
fn sample() -> Project {
    let mut p = Project::new("Test", 42);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let _pc2 = p.add_device("pc", 100, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    p
}

fn project_t(t: &mut T) {
    let mut p = Project::new("Test", 42);
    let pc1 = p.add_device("pc", 0, 0);
    let pc2 = p.add_device("pc", 100, 0);
    let sw = p.add_device("jf-s24p", 50, 100);
    t.eq("ids", (pc1.clone(), pc2.clone(), sw.clone()), (Ok(1), Ok(2), Ok(3)));
    let (pc1, pc2, sw) = (1, 2, 3);
    t.eq("names", p.devices.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), alloc::vec!["PC1", "PC2", "SW1"]);
    t.eq("note is no device", p.add_device("note", 0, 0), Err(ProjErr::NotADevice(String::from("Notiz"))));
    t.eq("unknown model", p.add_device("cisco-2960", 0, 0), Err(ProjErr::UnknownModel(String::from("cisco-2960"))));
    let l1 = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    t.eq("connect", l1, Ok(4));
    if let Some(l) = p.link(4) {
        t.eq("link ends", (l.a.port.as_str(), l.b.port.as_str()), ("eth0", "GigabitEthernet1/0/1"));
        t.eq("link cable", (l.cable, l.cable_fixed, l.kbps, l.delay_ns), (CableKind::CopperStraight, false, 1_000_000, 50));
    }
    t.check("port in use", matches!(p.connect(&pr(pc2, "eth0"), &pr(sw, "Gi1/0/1"), None), Err(ProjErr::Cable(Why::PortInUse(_)))));
    t.eq("same device", p.connect(&pr(pc1, "eth0"), &pr(pc1, "usb0"), None), Err(ProjErr::Cable(Why::SameDevice)));
    t.eq("unknown port", p.connect(&pr(pc2, "eth9"), &pr(sw, "gi1/0/2"), None), Err(ProjErr::UnknownPort { dev: String::from("PC2"), port: String::from("eth9") }));
    t.eq("suggest pc-switch", p.suggest_ports(pc2, sw), Some((String::from("eth0"), String::from("GigabitEthernet1/0/2"))));
    let sw2 = p.add_device("jf-s24p", 200, 100).unwrap_or(0);
    t.eq("suggest uplinks", p.suggest_ports(sw, sw2), Some((String::from("TenGigabitEthernet1/1/1"), String::from("TenGigabitEthernet1/1/1"))));
    let ap = p.add_device("jf-ap6e", 300, 0).unwrap_or(0);
    let nb = p.add_device("laptop", 300, 100).unwrap_or(0);
    t.eq("suggest wireless", p.suggest_ports(nb, ap), Some((String::from("wlan0"), String::from("Dot11Radio1"))));
    t.eq("fixed cable", p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), Some(CableKind::CopperCross)).map(|id| p.link(id).map(|l| (l.cable, l.cable_fixed))), Ok(Some((CableKind::CopperCross, true))));
    t.check("fiber refused", matches!(p.connect(&pr(sw2, "gi1/0/1"), &pr(nb, "eth0"), Some(CableKind::FiberMm)), Err(ProjErr::Cable(Why::CableDoesNotFit(..)))));
    t.eq("free ports", p.free_ports(sw).len(), 27);
    t.eq("neighbors", p.neighbors(sw).iter().map(|n| n.1).collect::<Vec<_>>(), alloc::vec![pc1, pc2]);
    t.eq("rename", p.rename(pc1, "Client-1"), Ok(()));
    t.eq("rename taken", p.rename(pc2, "client-1"), Err(ProjErr::NameTaken(String::from("client-1"))));
    t.eq("rename bad", p.rename(pc2, "1abc"), Err(ProjErr::BadName(String::from("1abc"))));
    t.check("rename bad space", p.rename(pc2, "a b").is_err() && p.rename(pc2, "end-").is_err());
    t.eq("free name reused", p.add_device("pc", 0, 200).ok().and_then(|id| p.device(id).map(|d| d.name.clone())), Some(String::from("PC1")));
    // remove and restore (undo)
    let links_before = p.links.len();
    let gone = p.remove_device(sw);
    t.check("remove device", gone.as_ref().is_some_and(|(d, ls)| d.name == "SW1" && ls.len() == 2) && p.links.len() == links_before - 2);
    t.check("removed", p.device(sw).is_none());
    if let Some((d, ls)) = gone {
        t.eq("restore device", p.insert_device(d.clone()), Ok(()));
        t.eq("restore twice", p.insert_device(d), Err(ProjErr::IdTaken(sw)));
        for l in ls {
            let id = l.id;
            t.eq(&format!("restore link {}", id), p.insert_link(l), Ok(()));
        }
    }
    t.eq("links back", p.links.len(), links_before);
    let before = p.max_id();
    let n = p.add_device("server-1u", 0, 0).unwrap_or(0);
    t.check("ids never reused", n > before);
    t.eq("consistent", p.check(), Vec::new());
    // an inconsistency is found
    let mut bad = p.clone();
    if let Some(d) = bad.devices.first().cloned() {
        let mut d = d;
        d.id = 999;
        bad.devices.push(d);
    }
    bad.links.push(jfn_model::Link { id: 1000, a: pr(pc2, "eth0"), b: pr(sw2, "gi1/0/3"), cable: CableKind::CopperStraight, kbps: 1000, ..jfn_model::Link::default() });
    let issues = bad.check();
    t.check("duplicate name found", issues.iter().any(|i| i.error && i.dev == Some(999)));
    t.check("port used twice found", issues.iter().any(|i| i.error && i.link == Some(1000)));
    t.eq("bounds", sample().bounds(), Some([0, 0, 100, 100]));
    let nid = p.add_note(10, 20, "Hallo");
    t.check("note", p.note(nid).is_some_and(|n| n.text == "Hallo") && p.remove_note(nid).is_some());
}

fn file_t(t: &mut T) {
    let mut p = sample();
    p.description = String::from("Zeile 1\n\"Zitat\"");
    p.lab = Some(jfn_model::Lab {
        title: jfn_model::Text::new("Ping", "Ping"),
        goals: alloc::vec![jfn_model::Goal { text: jfn_model::Text::new("PC1 erreicht PC2", ""), checks: alloc::vec![String::from("PC1 output contains \"0% packet loss\"")], ..jfn_model::Goal::default() }],
        ..jfn_model::Lab::default()
    });
    if let Some(d) = p.devices.get_mut(0) {
        d.startup_config = String::from("hostname PC1\nip addr add 10.0.0.1/24 dev eth0\n");
        d.services.push(ServiceDef { kind: String::from("http"), name: String::from("web"), disabled: false, config: Value::object([("port", Value::u64(80))]) });
    }
    let text = p.to_text();
    t.check("indented", text.starts_with("{\n  \"format\": \"jfn\",\n  \"version\": 1,") && text.ends_with("}\n"));
    t.eq("round trip", Project::from_text(&text), Ok(p.clone()));
    t.check("not json", Project::from_text("{").is_err_and(|e| e.offset > 0 || e.text.contains("JSON")));
    t.check("not an object", Project::from_text("[1, 2]").is_err());
    t.check("not ours", Project::from_text("{}").is_err_and(|e| e.text.contains("keine JFNetworker-Datei")));
    t.check("newer version", Project::from_text("{\"format\": \"jfn\", \"version\": 99}").is_err_and(|e| e.text.contains("99")));
    let min = Project::from_text("{\"format\": \"jfn\", \"version\": 1, \"future\": true, \"devices\": [{\"id\": 5, \"name\": \"R1\", \"model\": \"jf-r4\"}]}");
    t.check("defaults for missing members", min.as_ref().is_ok_and(|p| p.devices.len() == 1 && !p.devices[0].off && p.links.is_empty() && p.next_id == 6));
    let v = Value::object([("a", Value::Array(alloc::vec![Value::u64(1), Value::u64(2)])), ("b", Value::Object(Vec::new()))]);
    t.eq("pretty json", json::stringify_pretty(&v, Limits::default(), 2), Ok(String::from("{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {}\n}")));
    t.eq("compact json unchanged", json::stringify(&v), Ok(String::from("{\"a\":[1,2],\"b\":{}}")));
}

fn scenario_t(t: &mut T) {
    t.eq("ms", scenario::parse_duration("250ms"), Some(250_000_000));
    t.eq("fraction", scenario::parse_duration("1.5s"), Some(1_500_000_000));
    t.eq("us", scenario::parse_duration("10us"), Some(10_000));
    t.eq("micro sign", scenario::parse_duration("3\u{b5}s"), Some(3_000));
    t.eq("minutes", scenario::parse_duration("2m"), Some(120_000_000_000));
    t.eq("no unit", scenario::parse_duration("5"), None);
    t.eq("bad unit", scenario::parse_duration("1.5x"), None);
    t.eq("rate M", scenario::parse_rate("100M"), Some(100_000_000));
    t.eq("rate G fraction", scenario::parse_rate("2.5G"), Some(2_500_000_000));
    t.eq("rate k", scenario::parse_rate("64k"), Some(64_000));
    t.eq("rate plain", scenario::parse_rate("2048000"), Some(2_048_000));
    t.eq("rate suffix", scenario::parse_rate("1Gbit/s"), Some(1_000_000_000));
    t.eq("ppm percent", (scenario::parse_ppm("5%"), scenario::parse_ppm("0.1%"), scenario::parse_ppm("250ppm")), (Some(50_000), Some(1_000), Some(250)));
    t.eq("ppm too much", scenario::parse_ppm("101%"), None);
    let text = "# demo\nseed 7\nat 1s PC1: ping -c 4 10.0.0.2\nPC1:ip addr\nrun 5s\nrun until idle\nlink down R1 Gi0/0\nlink up R1 Gi0/0\nimpair R1 Gi0/1 loss 5% delay 20ms jitter 2ms rate 100M\npower off SRV2\nexpect PC1 output contains \"4 received\"\nexpect PC1 output not contains \"timeout\"\nexpect R1 route 10.0.2.0/24 via 10.0.1.2\nexpect R1 route 10.0.2.7/24 via connected\nexpect R1 route 0.0.0.0/0 dev Gi0/0\nexpect R1 no route 192.168.9.0/24\nexpect hops ICMP >= 8\nexpect SRV1 svc web metric requests == 4\npcap build/run.pcapng\nprint \"fertig: 1 Test\"\n";
    match scenario::parse(text) {
        Ok(sc) => {
            t.eq("seed", sc.seed, Some(7));
            t.eq("steps", sc.steps.len(), 18);
            let k: Vec<&StepKind> = sc.steps.iter().map(|s| &s.kind).collect();
            t.eq("timed cli", (sc.steps[0].line, sc.steps[0].at, k[0]), (3, Some(1_000_000_000), &StepKind::Cli { dev: String::from("PC1"), cmd: String::from("ping -c 4 10.0.0.2") }));
            t.eq("cli without space", k[1], &StepKind::Cli { dev: String::from("PC1"), cmd: String::from("ip addr") });
            t.eq("run", (k[2], k[3]), (&StepKind::Run(5_000_000_000), &StepKind::RunIdle));
            t.eq("link", k[4], &StepKind::Link { dev: String::from("R1"), port: String::from("Gi0/0"), up: false });
            t.eq("impair", k[6], &StepKind::Impair { dev: String::from("R1"), port: String::from("Gi0/1"), imp: scenario::Impair { loss_ppm: Some(50_000), delay_ns: Some(20_000_000), jitter_ns: Some(2_000_000), bps: Some(100_000_000) } });
            t.eq("power", k[7], &StepKind::Power { dev: String::from("SRV2"), on: false });
            t.eq("output", k[9], &StepKind::Expect(Expect::Output { dev: String::from("PC1"), needle: String::from("timeout"), negate: true }));
            let net = Prefix4::parse("10.0.2.0/24").unwrap_or_default();
            t.eq("route via", k[10], &StepKind::Expect(Expect::Route { dev: String::from("R1"), prefix: net, via: Via::Addr(Ipv4([10, 0, 1, 2])) }));
            t.eq("route prefix truncated", k[11], &StepKind::Expect(Expect::Route { dev: String::from("R1"), prefix: net, via: Via::Connected }));
            t.eq("route dev", k[12], &StepKind::Expect(Expect::Route { dev: String::from("R1"), prefix: Prefix4::parse("0.0.0.0/0").unwrap_or_default(), via: Via::Iface(String::from("Gi0/0")) }));
            t.check("no route", matches!(k[13], StepKind::Expect(Expect::NoRoute { .. })));
            t.eq("hops", k[14], &StepKind::Expect(Expect::Hops { proto: String::from("icmp"), cmp: Cmp::Ge, n: 8 }));
            t.eq("metric", k[15], &StepKind::Expect(Expect::Metric { dev: String::from("SRV1"), svc: String::from("web"), name: String::from("requests"), cmp: Cmp::Eq, value: 4 }));
            t.eq("pcap and print", (k[16], k[17]), (&StepKind::Pcap(String::from("build/run.pcapng")), &StepKind::Print(String::from("fertig: 1 Test"))));
        }
        Err(e) => t.check(&format!("scenario parses: {:?}", e), false),
    }
    let bad = "run 5\nexpect PC1 output maybe \"x\"\nimpair R1 Gi0/1 loss\nfrobnicate now\nprint \"open\nat 1s\n";
    match scenario::parse(bad) {
        Ok(_) => t.check("errors found", false),
        Err(e) => t.eq("error lines", e.iter().map(|x| x.line).collect::<Vec<_>>(), alloc::vec![1, 2, 3, 4, 5, 6]),
    }
    t.check("compare", Cmp::Ge.holds(8, 8) && Cmp::Lt.holds(1, 2) && !Cmp::Ne.holds(3, 3) && Cmp::parse("=") == Some(Cmp::Eq));
}
