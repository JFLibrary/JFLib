// phystest -- checks of the physical view (group phys of --selftest; jfn_model::phys, the
// catalogue's forms and optics, jfn_sim::phys, doc/phys.rs, the scenario steps and
// expectations): the reach of every medium at every speed after IEEE 802.3, the forms and rack
// heights of the models, optics and the fibre they pick, the default tree, older projects in
// the default room (every fixture: 7 m, no cable over its limit, delays as before), the
// automatic lengths of the campus fixture to the metre (in a rack, on a desk, between rooms,
// floors and buildings, to the ceiling), a cable over its limit down at both ends with its
// note, log lines, problem, "show interfaces" and "ip link", back up when it is shorter (moved
// into another room, a length by hand, automatic again), optics that do not fit, the delay that
// follows a plan, places added, renamed, moved, resized and removed and devices placed as undo
// steps with the simulator following, the refusals, the file format, the scenario syntax and a
// scenario run twice (determinism)
//
// The campus (tests/fixtures/phys_campus.jfnet): Haus A with the server room (Rack 1: R1 at
// unit 30, SW1 at 20; Rack 2: SRV1 at 10), Büro 1 (PC1 and PC2 on desks, AP1 on the ceiling)
// and Büro 2 one floor up (PC3), Haus B 180 m east with the lab (SW2 in Rack B1, PC4 without a
// spot).  PC4's copper cable to SW1 is 208 m long.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use jfn_model::cable::{self, CableKind};
use jfn_model::catalog::{self, Form, Optic};
use jfn_model::phys::{self, OpticFault, PhysErr, PlaceKind, Tree};
use jfn_model::project::Project;
use jfn_model::scenario::{self, CableCheck, Cmp, Expect, StepKind};
use jfn_sim::{Action, Engine, Note, PhysFault, PhysNote, ProblemKind, Report, SEC};

use crate::doc::Doc;
use crate::selftest::T;

const BUDGET: usize = 20_000_000;
const CAMPUS: &str = include_str!("../tests/fixtures/phys_campus.jfnet");
// devices and cables of the campus
const R1: u32 = 1;
const SW1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;
const PC3: u32 = 6;
const PC4: u32 = 7;
const SW2: u32 = 8;
const L_PC4: u32 = 24;
const L_FIBRE: u32 = 27;

fn s(x: &str) -> String {
    String::from(x)
}

fn campus() -> Project {
    Project::from_text(CAMPUS).unwrap_or_default()
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
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

fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + SEC;
    e.run_until(t, BUDGET);
    out(e, dev)
}

pub(crate) fn run(t: &mut T) {
    reach(t);
    forms(t);
    optics(t);
    tree(t);
    old_projects(t);
    lengths(t);
    too_long(t);
    fixed_again(t);
    optic_faults(t);
    delays(t);
    edits(t);
    undo(t);
    file(t);
    scenarios(t);
}

fn reach(t: &mut T) {
    let r = |k: CableKind, kbps: u32, o: Option<Optic>| phys::reach(k, kbps, o).map(|x| (x.max_m, x.std));
    t.eq("copper 10M", r(CableKind::CopperStraight, 10_000, None), Some((100, "10BASE-T")));
    t.eq("copper 100M", r(CableKind::CopperCross, 100_000, None), Some((100, "100BASE-TX")));
    t.eq("copper 1G", r(CableKind::CopperStraight, 1_000_000, None), Some((100, "1000BASE-T")));
    t.eq("copper 2.5G", r(CableKind::CopperStraight, 2_500_000, None), Some((100, "2.5GBASE-T")));
    t.eq("copper 10G on Cat6A", r(CableKind::CopperStraight, 10_000_000, None), Some((100, "10GBASE-T")));
    t.eq("copper 25G", r(CableKind::CopperStraight, 25_000_000, None), Some((30, "25GBASE-T")));
    t.eq("SR 1G", r(CableKind::FiberMm, 1_000_000, Some(Optic::Sr)), Some((550, "1000BASE-SX")));
    t.eq("SR 10G on OM3", r(CableKind::FiberMm, 10_000_000, Some(Optic::Sr)), Some((300, "10GBASE-SR")));
    t.eq("SR 25G", r(CableKind::FiberMm, 25_000_000, Some(Optic::Sr)), Some((70, "25GBASE-SR")));
    t.eq("SR4 100G", r(CableKind::FiberMm, 100_000_000, Some(Optic::Sr)), Some((70, "100GBASE-SR4")));
    t.eq("LX 1G", r(CableKind::FiberSm, 1_000_000, Some(Optic::Lr)), Some((5_000, "1000BASE-LX")));
    t.eq("LR 10G", r(CableKind::FiberSm, 10_000_000, Some(Optic::Lr)), Some((10_000, "10GBASE-LR")));
    t.eq("LR 25G", r(CableKind::FiberSm, 25_000_000, Some(Optic::Lr)), Some((10_000, "25GBASE-LR")));
    t.eq("LR4 100G", r(CableKind::FiberSm, 100_000_000, Some(Optic::Lr)), Some((10_000, "100GBASE-LR4")));
    t.eq("no optic counts as SR", r(CableKind::FiberMm, 10_000_000, None), Some((300, "10GBASE-SR")));
    t.eq("console", r(CableKind::Console, 115, None), Some((15, "RS-232")));
    t.eq("serial at E1", r(CableKind::Serial, 2048, None), Some((15, "V.35")));
    t.eq("serial at 64k", r(CableKind::Serial, 64, None), Some((51, "V.35")));
    t.eq("serial at 9.6k", r(CableKind::Serial, 9, None), Some((312, "V.35")));
    t.eq("serial at 2.4k", r(CableKind::Serial, 2, None), Some((1250, "V.35")));
    t.eq("WAN has none", r(CableKind::Wan, 1_000_000, None), None);
    t.eq("radio has none", r(CableKind::Wireless, 2_400_000, None), None);
    t.eq("max_len_m: copper", CableKind::CopperCross.max_len_m(1_000_000), Some(100));
    t.eq("max_len_m: multimode 25G", CableKind::FiberMm.max_len_m(25_000_000), Some(70));
    t.eq("max_len_m: single-mode", CableKind::FiberSm.max_len_m(10_000_000), Some(10_000));
}

fn forms(t: &mut T) {
    let f = |id: &str| catalog::model(id).map(|m| (catalog::form(m), catalog::rack_units(m)));
    t.eq("pc", f("pc"), Some((Form::Desktop, 0)));
    t.eq("laptop", f("laptop"), Some((Form::Handheld, 0)));
    t.eq("access point", f("jf-ap6e"), Some((Form::Ceiling, 0)));
    t.eq("camera", f("iot-camera"), Some((Form::Ceiling, 0)));
    t.eq("switch 1U", f("jf-s24p"), Some((Form::Rack, 1)));
    t.eq("core router 2U", f("jf-r8"), Some((Form::Rack, 2)));
    t.eq("db shard 2U", f("database"), Some((Form::Rack, 2)));
    t.eq("tower", f("server-tower"), Some((Form::Tower, 0)));
    t.eq("cloud", f("cloud"), Some((Form::Virtual, 0)));
    t.check("every device model has a form", catalog::catalog().iter().all(|m| catalog::form(m) != Form::Rack || catalog::rack_units(m) >= 1));
}

fn optics(t: &mut T) {
    let sw = catalog::model("jf-s24p").and_then(|m| m.port("Te1/1/1"));
    let r = catalog::model("jf-r4").and_then(|m| m.port("Te0/1/0"));
    let (Some(sw), Some(r)) = (sw, r) else {
        t.check("ports", false);
        return;
    };
    t.eq("cages come with SR", (sw.optic, r.optic), (Some(Optic::Sr), Some(Optic::Sr)));
    t.eq("copper has none", catalog::model("pc").and_then(|m| m.port("eth0")).and_then(|p| p.optic), None);
    t.eq("SR to SR: multimode", cable::cable_kind(&sw, &r), Ok(CableKind::FiberMm));
    let lr = |p: &jfn_model::PortSpec| jfn_model::PortSpec { optic: Some(Optic::Lr), ..p.clone() };
    t.eq("LR to LR: single-mode", cable::cable_kind(&lr(&sw), &lr(&r)), Ok(CableKind::FiberSm));
    t.eq("SR to LR still plugs in", cable::cable_kind(&sw, &lr(&r)), Ok(CableKind::FiberMm));
    t.eq("optics differ", phys::optic_fault(CableKind::FiberMm, Some(Optic::Sr), Some(Optic::Lr)), Some(OpticFault::Mismatch(Optic::Sr, Optic::Lr)));
    t.eq("SR on single-mode", phys::optic_fault(CableKind::FiberSm, Some(Optic::Sr), Some(Optic::Sr)), Some(OpticFault::WrongFiber(Optic::Sr)));
    t.eq("LR on multimode", phys::optic_fault(CableKind::FiberMm, Some(Optic::Lr), Some(Optic::Lr)), Some(OpticFault::WrongFiber(Optic::Lr)));
    t.eq("fitting optics", (phys::optic_fault(CableKind::FiberMm, None, None), phys::optic_fault(CableKind::FiberSm, Some(Optic::Lr), Some(Optic::Lr))), (None, None));
    t.eq("copper has no optic fault", phys::optic_fault(CableKind::CopperStraight, Some(Optic::Lr), None), None);
}

fn tree(t: &mut T) {
    let p = Project::new("x", 1);
    let tr = Tree::of(&p);
    t.check("a new project has the default tree", tr.is_default() && p.places.is_empty());
    t.eq("default room", tr.default_room(), phys::DEFAULT_ROOM);
    t.eq("its path", tr.path(4), vec![1, 2, 3, 4]);
    t.eq("its names", tr.name_path(4), s("Standort \u{203a} Gebäude \u{203a} Erdgeschoss \u{203a} Raum"));
    t.eq("kinds", tr.places.iter().map(|x| x.kind).collect::<Vec<_>>(), vec![PlaceKind::Site, PlaceKind::Building, PlaceKind::Floor, PlaceKind::Room]);
    let c = campus();
    let tc = Tree::of(&c);
    t.check("the campus has its own tree", !tc.is_default());
    t.eq("campus: first room", tc.default_room(), 4);
    t.eq("up to the building", tc.up(5, PlaceKind::Building), Some(2));
    t.eq("a floor has its building's size", tc.size(3), (400, 250));
    t.eq("rack B1 height", tc.units(15), 24);
    t.eq("absolute position of Rack 1 (mm)", tc.abs(5), (23_000, 23_000));
    t.eq("storey of Büro 2", tc.level(11), 1);
    let w = |d: u32| c.device(d).map(|x| tc.whereis(x)).unwrap_or_default();
    t.eq("R1 in Rack 1 at 30", (w(R1).rack, w(R1).unit, w(R1).room), (5, 30, 4));
    t.eq("PC1 on Tisch 1", (w(PC1).desk, w(PC1).room), (8, 7));
    t.eq("PC3 at its spot", (w(PC3).room, w(PC3).at), (11, Some((50, 40))));
    t.eq("PC4 without a spot", (w(PC4).room, w(PC4).at), (14, None));
    t.eq("devices in Haus A", tc.devices_in(&c, 2).len(), 7);
    t.eq("devices in Rack 1", tc.devices_in(&c, 5).iter().map(|d| d.name.clone()).collect::<Vec<_>>(), vec![s("R1"), s("SW1")]);
    t.check("units 21 .. 29 free", tc.rack_free(&c, 5, 21, 9, 0) && !tc.rack_free(&c, 5, 20, 1, 0) && tc.rack_free(&c, 5, 20, 1, SW1));
    t.eq("lowest free slot of 2 U", tc.rack_slot(&c, 5, 2, 0), Some(1));
    t.check("past the top", !tc.rack_free(&c, 15, 24, 2, 0) && tc.rack_free(&c, 15, 23, 2, 0));
}

fn old_projects(t: &mut T) {
    let fixtures: [(&str, &str); 6] = [
        ("two_pcs", include_str!("../tests/fixtures/two_pcs.jfnet")),
        ("office", include_str!("../tests/fixtures/office.jfnet")),
        ("campus", include_str!("../tests/fixtures/campus.jfnet")),
        ("serial_wan", include_str!("../tests/fixtures/serial_wan.jfnet")),
        ("lab_window", include_str!("../tests/fixtures/lab_window.jfnet")),
        ("wlan_office", include_str!("../tests/fixtures/wlan_office.jfnet")),
    ];
    for (name, text) in fixtures {
        let Ok(p) = Project::from_text(text) else {
            t.check(&format!("{} loads", name), false);
            continue;
        };
        let tr = Tree::of(&p);
        t.check(&format!("{}: the default tree", name), tr.is_default());
        t.check(&format!("{}: every device in the default room without a spot", name), p.devices.iter().all(|d| tr.whereis(d) == phys::Where { room: 4, ..phys::Where::default() }));
        // in one room without a spot: up from the floor (or from the ceiling: 0) to the trays
        // and down again, 10 % more -- 7 m, 4 m to a device on the ceiling; 3 m to the provider's
        // cloud, radio none
        let form = |d: u32| p.model_of(d).map_or(Form::Desktop, catalog::form);
        let up = |d: u32| if form(d) == Form::Ceiling { 0 } else { 2800u32 };
        let want = |l: &jfn_model::Link| {
            if l.cable == CableKind::Wireless {
                0
            } else if form(l.a.dev) == Form::Virtual || form(l.b.dev) == Form::Virtual {
                3
            } else {
                ((up(l.a.dev) + up(l.b.dev)) * 110 / 100).div_ceil(1000).max(1)
            }
        };
        let off: Vec<(u32, u32, u32)> = p.links.iter().map(|l| (l.id, phys::length(&p, &tr, l).m, want(l))).filter(|x| x.1 != x.2).collect();
        t.eq(&format!("{}: every cable 7 m (to the cloud 3 m, radio 0)", name), off, Vec::new());
        t.check(&format!("{}: delays as before", name), p.links.iter().all(|l| phys::delay_ns(&p, &tr, l) == l.delay_ns));
        t.check(&format!("{}: no place issue", name), phys::issues(&p).is_empty());
        let e = engine(&p);
        let held: Vec<(u32, u32)> = p.links.iter().filter_map(|l| e.cable_phys(l.id).filter(|c| c.fault.is_some() || c.len_m != want(l)).map(|c| (l.id, c.len_m))).collect();
        t.eq(&format!("{}: no cable held down by its length", name), held, Vec::new());
        t.check(&format!("{}: no physical problem", name), !e.problems().iter().any(|x| matches!(x.kind, ProblemKind::CableTooLong | ProblemKind::OpticMismatch)));
    }
    // the console and serial cables of the office stay within their limits
    let p = Project::from_text(include_str!("../tests/fixtures/office.jfnet")).unwrap_or_default();
    let kinds: Vec<CableKind> = p.links.iter().map(|l| l.cable).collect();
    t.check("the office has console and serial cables", kinds.contains(&CableKind::Console) && kinds.contains(&CableKind::Serial));
    let tr = Tree::of(&p);
    t.eq("a switch to an access point on the ceiling: 4 m", p.links.iter().find(|l| l.id == 31).map(|l| phys::length(&p, &tr, l).m), Some(4));
}

fn lengths(t: &mut T) {
    let p = campus();
    let tr = Tree::of(&p);
    let len = |id: u32| p.link(id).map(|l| phys::length(&p, &tr, l));
    let auto = |m: u32| Some(jfn_model::Len { m, auto: true });
    t.eq("R1 - SW1 in one rack: a patch cable", len(20), auto(1));
    t.eq("SRV1 - SW1 between two racks", len(21), auto(6));
    t.eq("PC1 on a desk to the server room", len(22), auto(16));
    t.eq("PC2 on the other desk", len(23), auto(20));
    t.eq("PC4 in Haus B", len(L_PC4), auto(208));
    t.eq("AP1 on the ceiling", len(25), auto(20));
    t.eq("PC3 one floor up", len(26), auto(16));
    t.eq("the fibre between the houses", len(L_FIBRE), auto(203));
    // the rule's parts
    let mut q = p.clone();
    let names = phys::DEFAULT_NAMES;
    t.check("PC2 onto PC1's desk", phys::place_device(&mut q, 5, 8, 0, None, &names).is_ok());
    let tq = Tree::of(&q);
    let both = |a: u32, b: u32| phys::auto_len_m(&q, &tq, a, b);
    t.eq("on one desk: 2 m", both(PC1, 5), 2);
    t.eq("to the cloud: 3 m", {
        let mut c = q.clone();
        let id = c.add_device("cloud", 0, 0).unwrap_or(0);
        let tc = Tree::of(&c);
        phys::auto_len_m(&c, &tc, id, PC1)
    }, 3);
    t.eq("a link of another length by hand", {
        let mut c = q.clone();
        let _ = phys::set_len(&mut c, 22, 42);
        let tc = Tree::of(&c);
        c.link(22).map(|l| phys::length(&c, &tc, l))
    }, Some(jfn_model::Len { m: 42, auto: false }));
    let ra = phys::link_reach(&p, p.link(L_PC4).unwrap_or(&jfn_model::Link::default()));
    t.eq("PC4's limit", ra.map(|r| (r.max_m, r.std)), Some((100, "1000BASE-T")));
    let rf = phys::link_reach(&p, p.link(L_FIBRE).unwrap_or(&jfn_model::Link::default()));
    t.eq("the fibre's limit", rf.map(|r| (r.max_m, r.std)), Some((300, "10GBASE-SR")));
}

fn notes(e: &Engine, dev: u32) -> Vec<Note> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| x.note.clone()).collect()
}

fn too_long(t: &mut T) {
    let p = campus();
    let mut e = engine(&p);
    let reports = e.take_reports();
    e.run_until(2 * SEC, BUDGET);
    let f = e.cable_phys(L_PC4);
    t.eq("PC4's cable in the engine", f.map(|x| (x.len_m, x.fault)), Some((208, Some(PhysFault::TooLong { len_m: 208, max_m: 100, std: "1000BASE-T" }))));
    t.check("it is down", !e.link_idx_up(L_PC4));
    t.check("the others are up", [20, 21, 22, 23, 25, 26, L_FIBRE].iter().all(|&l| e.link_idx_up(l)));
    let sw_port = e.port_index(SW1, "Gi1/0/5").unwrap_or(99);
    t.check("SW1 notes it", notes(&e, SW1).contains(&Note::Phys(PhysNote::TooLong { port: sw_port, len_m: 208, max_m: 100, std: "1000BASE-T" })));
    t.check("PC4 notes it", notes(&e, PC4).contains(&Note::Phys(PhysNote::TooLong { port: 0, len_m: 208, max_m: 100, std: "1000BASE-T" })));
    let msg = Note::Phys(PhysNote::TooLong { port: 4, len_m: 208, max_m: 100, std: "1000BASE-T" }).message(&|_| s("Gi1/0/5"), &|_| s(""));
    t.eq("the note's sentence", msg.text(), s("Kabel an Gi1/0/5 zu lang: 208 m, erlaubt 100 m (1000BASE-T); die Leitung bleibt aus."));
    let log = reports.iter().any(|r| matches!(r, Report::Log { dev, tag, text, .. } if *dev == SW1 && tag == "%PHY-4-CABLE_LENGTH" && text.contains("GigabitEthernet1/0/5: the cable is 208 m long, 1000BASE-T allows 100 m")));
    t.check("SW1 logs it", log);
    let klog = reports.iter().any(|r| matches!(r, Report::Log { dev, tag, text, .. } if *dev == PC4 && tag == "kernel" && text.contains("eth0: cable of 208 m longer than the 100 m 1000BASE-T allows")));
    t.check("PC4's kernel logs it", klog);
    let probs = e.problems();
    let pr = probs.iter().find(|x| x.kind == ProblemKind::CableTooLong);
    t.eq("the problem", pr.map(|x| x.text.clone()), Some(s("Kabel zu lang: 208 m, erlaubt 100 m (1000BASE-T zwischen PC4 eth0 und SW1 Gi1/0/5); die Leitung bleibt aus.")));
    t.eq("the problem's cable", pr.map(|x| (x.link, x.dev)), Some((Some(L_PC4), PC4)));
    let _ = cli(&mut e, SW1, "enable");
    let sh = cli(&mut e, SW1, "show interfaces GigabitEthernet1/0/5");
    t.check("show interfaces says it", sh.contains("GigabitEthernet1/0/5 is down, line protocol is down (notconnect)") && sh.contains("  Cable is 208 m long, 1000BASE-T allows 100 m: no link"));
    let ok = cli(&mut e, SW1, "show interfaces GigabitEthernet1/0/3");
    t.check("a good cable says nothing", ok.contains("is up, line protocol is up") && !ok.contains("Cable is"));
    let il = cli(&mut e, PC4, "ip link");
    t.check("ip link: no carrier", il.contains("eth0: <NO-CARRIER,BROADCAST,MULTICAST,UP>"));
    let ping = cli(&mut e, PC4, "ping -c 1 10.0.0.11");
    t.check("PC4 cannot ping", !ping.contains("1 received"));
}

fn fixed_again(t: &mut T) {
    let p = campus();
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    // PC4 into Büro 1: the automatic length follows (the app tells the engine, doc/sim.rs)
    let mut q = p.clone();
    t.check("PC4 into Büro 1", phys::place_device(&mut q, PC4, 7, 0, None, &phys::DEFAULT_NAMES).is_ok());
    let tq = Tree::of(&q);
    let m = q.link(L_PC4).map_or(0, |l| phys::length(&q, &tq, l).m);
    t.eq("its new length", m, 23);
    t.check("CableLen", e.apply(Action::CableLen { link: L_PC4, len_m: m }).is_ok());
    t.check("up again", e.link_idx_up(L_PC4) && e.cable_phys(L_PC4).is_some_and(|c| c.fault.is_none() && c.len_m == 23));
    e.run_until(e.now() + 3 * SEC, BUDGET);
    let ping = cli(&mut e, PC4, "ping -c 2 10.0.0.11");
    let t2 = e.now() + 3 * SEC;
    e.run_until(t2, BUDGET);
    let ping = ping + &out(&mut e, PC4);
    t.check("PC4 pings PC1", ping.contains("2 packets transmitted, 2 received"));
    // too long by hand, then automatic again
    t.check("132 m by hand", e.apply(Action::CableLen { link: L_PC4, len_m: 132 }).is_ok() && !e.link_idx_up(L_PC4));
    t.check("the fault names 132 m", e.cable_phys(L_PC4).is_some_and(|c| c.fault == Some(PhysFault::TooLong { len_m: 132, max_m: 100, std: "1000BASE-T" })));
    t.check("unknown length: never too long", e.apply(Action::CableLen { link: L_PC4, len_m: 0 }).is_ok() && e.link_idx_up(L_PC4));
    t.check("100 m is just allowed", e.apply(Action::CableLen { link: L_PC4, len_m: 100 }).is_ok() && e.link_idx_up(L_PC4));
    t.check("an unknown cable is refused", e.apply(Action::CableLen { link: 999, len_m: 5 }).is_err());
}

fn optic_faults(t: &mut T) {
    let mut p = campus();
    t.check("LR in SW1's cage", phys::set_optic(&mut p, SW1, "Te1/1/1", Optic::Lr).is_ok());
    t.eq("stored by its full name", p.device(SW1).map(|d| d.optics.get("TenGigabitEthernet1/1/1").copied()), Some(Some(Optic::Lr)));
    t.eq("the automatic cable stays multimode (SR at SW2)", p.link(L_FIBRE).map(|l| l.cable), Some(CableKind::FiberMm));
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.eq("the engine: optics differ", e.cable_phys(L_FIBRE).and_then(|c| c.fault), Some(PhysFault::Optics(OpticFault::Mismatch(Optic::Lr, Optic::Sr))));
    t.check("down", !e.link_idx_up(L_FIBRE));
    let probs = e.problems();
    t.eq(
        "the problem",
        probs.iter().find(|x| x.kind == ProblemKind::OpticMismatch).map(|x| x.text.clone()),
        Some(s("Optiken passen nicht: LR an SW1 Te1/1/1, SR an SW2 Te1/1/1; die Leitung bleibt aus.")),
    );
    t.check("LR in SW2's too", phys::set_optic(&mut p, SW2, "TenGigabitEthernet1/1/1", Optic::Lr).is_ok());
    t.eq("the cable follows: single-mode", p.link(L_FIBRE).map(|l| l.cable), Some(CableKind::FiberSm));
    t.check("the engine follows", e.apply(Action::Optic { dev: SW2, port: s("Te1/1/1"), optic: Optic::Lr }).is_ok() && e.apply(Action::SetCable { link: L_FIBRE, cable: CableKind::FiberSm }).is_ok());
    t.check("up with LR", e.link_idx_up(L_FIBRE) && e.cable_phys(L_FIBRE).is_some_and(|c| c.reach.map(|r| r.max_m) == Some(10_000)));
    t.check("a fixed multimode cable on LR stays down", e.apply(Action::SetCable { link: L_FIBRE, cable: CableKind::FiberMm }).is_ok() && e.cable_phys(L_FIBRE).and_then(|c| c.fault) == Some(PhysFault::Optics(OpticFault::WrongFiber(Optic::Lr))));
    t.check("no cage, no optic", phys::set_optic(&mut p, SW1, "Gi1/0/1", Optic::Lr) == Err(PhysErr::NoCage(s("GigabitEthernet1/0/1"))));
    t.check("the engine refuses too", e.apply(Action::Optic { dev: SW1, port: s("Gi1/0/1"), optic: Optic::Lr }).is_err());
    // back to SR is no entry in the file
    t.check("SR again", phys::set_optic(&mut p, SW1, "Te1/1/1", Optic::Sr).is_ok() && p.device(SW1).is_some_and(|d| d.optics.is_empty()));
}

fn delays(t: &mut T) {
    let p = campus();
    let tr = Tree::of(&p);
    let d = |id: u32| p.link(id).map(|l| phys::delay_ns(&p, &tr, l));
    t.eq("placed copper: 5 ns per metre", d(22), Some(80));
    t.eq("placed fibre", d(L_FIBRE), Some(203 * 5));
    let mut q = p.clone();
    if let Some(l) = q.link_mut(22) {
        l.delay_ns = 1234;
    }
    let tq = Tree::of(&q);
    t.eq("a delay of its own stays", q.link(22).map(|l| phys::delay_ns(&q, &tq, l)), Some(1234));
    let e = engine(&p);
    t.check("the engine runs with it", e.link_rt(22).is_some_and(|l| l.delay_ns == 80));
    let old = Project::from_text(include_str!("../tests/fixtures/two_pcs.jfnet")).unwrap_or_default();
    let eo = engine(&old);
    t.check("an older project keeps 50 ns", old.links.iter().all(|l| eo.link_rt(l.id).is_some_and(|x| x.delay_ns == l.delay_ns)));
}

fn edits(t: &mut T) {
    let names = phys::DEFAULT_NAMES;
    let mut p = Project::new("x", 1);
    let pc = p.add_device("pc", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let next = p.next_id;
    // the first edit stores the default tree; place ids count on their own
    let rack = phys::add_place(&mut p, PlaceKind::Rack, 4, "Rack", &names);
    t.eq("a rack in the default room", rack, Ok(5));
    t.eq("the tree is stored", p.places.len(), 5);
    t.eq("devices and cables keep their counter", p.next_id, next);
    t.eq("its name and height", p.places.last().map(|x| (x.name.clone(), x.units)), Some((s("Rack 1"), 42)));
    t.eq("a second one beside it", phys::add_place(&mut p, PlaceKind::Rack, 4, "Rack", &names), Ok(6));
    t.eq("named Rack 2", p.places.last().map(|x| (x.name.clone(), x.x_dm)), Some((s("Rack 2"), 8)));
    t.eq("a desk", phys::add_place(&mut p, PlaceKind::Desk, 4, "Tisch", &names), Ok(7));
    t.check("a room does not go into a room", phys::add_place(&mut p, PlaceKind::Room, 4, "Raum", &names) == Err(PhysErr::WrongParent(PlaceKind::Room, PlaceKind::Room)));
    t.check("a site at the top", phys::add_place(&mut p, PlaceKind::Site, 0, "Standort", &names) == Ok(8));
    t.check("unknown parent", phys::add_place(&mut p, PlaceKind::Building, 99, "Haus", &names) == Err(PhysErr::UnknownPlace(99)));
    // devices
    t.check("the switch into the rack", phys::place_device(&mut p, sw, 5, 0, None, &names).is_ok());
    t.eq("at the lowest unit", p.device(sw).map(|d| (d.phys.place, d.phys.unit)), Some((5, 1)));
    t.check("a desktop does not go into a rack", phys::place_device(&mut p, pc, 5, 0, None, &names) == Err(PhysErr::NotRack(s("PC1"))));
    t.check("onto the desk", phys::place_device(&mut p, pc, 7, 0, None, &names).is_ok());
    t.check("not on a site", phys::place_device(&mut p, pc, 1, 0, None, &names) == Err(PhysErr::NoDevices(PlaceKind::Site)));
    let sw2 = p.add_device("jf-s24p", 200, 0).unwrap_or(0);
    t.check("unit 1 is taken", phys::place_device(&mut p, sw2, 5, 1, None, &names) == Err(PhysErr::UnitsTaken { rack: s("Rack 1"), unit: 1, h: 1 }));
    t.check("unit 2 is free", phys::place_device(&mut p, sw2, 5, 2, None, &names).is_ok());
    t.check("a rack lower than its devices", phys::set_place(&mut p, 5, None, None, Some(1), None, &names) == Err(PhysErr::TooLow(2)));
    t.check("a lower rack that holds them", phys::set_place(&mut p, 5, None, None, Some(12), None, &names).is_ok());
    let full = phys::set_place(&mut p, 6, None, None, Some(1), None, &names).and_then(|_| {
        phys::place_device(&mut p, sw2, 6, 0, None, &names)?;
        let r = p.add_device("jf-r4", 0, 0).unwrap_or(0);
        phys::place_device(&mut p, r, 6, 0, None, &names)
    });
    t.eq("a full rack", full, Err(PhysErr::RackFull(s("Rack 2"))));
    t.check("a free spot in the room", phys::place_device(&mut p, pc, 4, 0, Some((30, 20)), &names).is_ok() && p.device(pc).is_some_and(|d| d.phys.placed && d.phys.x_dm == 30));
    // places
    t.check("rename", phys::rename_place(&mut p, 5, "Netzwerk", &names).is_ok() && p.places.iter().any(|x| x.name == "Netzwerk"));
    t.check("an empty name", phys::rename_place(&mut p, 5, "  ", &names) == Err(PhysErr::BadName(s(""))));
    t.check("moved and resized", phys::set_place(&mut p, 4, Some((5, 6)), Some((70, 50)), None, None, &names).is_ok() && Tree::of(&p).size(4) == (70, 50));
    t.check("a size of 0", phys::set_place(&mut p, 4, None, Some((0, 50)), None, None, &names) == Err(PhysErr::BadSize));
    t.check("the last room stays", phys::remove_place(&mut p, 4, &names) == Err(PhysErr::LastRoom));
    t.check("the rack goes, its devices go to the room", phys::remove_place(&mut p, 5, &names).is_ok() && p.device(sw).is_some_and(|d| d.phys == jfn_model::Spot::default()));
    t.check("a length by hand", phys::set_len(&mut p, 0, 5) == Err(PhysErr::UnknownLink(0)));
    let l = p.connect(&jfn_model::PortRef { dev: pc, port: s("eth0") }, &jfn_model::PortRef { dev: sw, port: s("Gi1/0/1") }, None).unwrap_or(0);
    t.check("too long by hand", phys::set_len(&mut p, l, 100_001) == Err(PhysErr::BadLen(100_001)) && phys::set_len(&mut p, l, 132).is_ok());
    // issues of a file that does not fit
    let mut bad = p.clone();
    bad.places.push(jfn_model::Place { id: 50, kind: PlaceKind::Desk, name: s("Irgendwo"), parent: 1, ..jfn_model::Place::default() });
    if let Some(d) = bad.device_mut(pc) {
        d.phys = jfn_model::Spot { place: 6, unit: 1, ..jfn_model::Spot::default() };
    }
    let iss: Vec<String> = phys::issues(&bad).into_iter().map(|x| x.2.text()).collect();
    t.check("a desk on a site", iss.iter().any(|x| x == "Der Ort \u{201e}Irgendwo\u{201c} liegt in keinem passenden Ort."));
    t.check("a desktop in a rack", iss.iter().any(|x| x == "PC1 ist kein Rack-Gerät und steckt doch im Rack \u{201e}Rack 2\u{201c}."));
    t.check("the project's check has them", bad.check().iter().any(|x| x.text.contains("Irgendwo")));
    // floors in each other: no endless walk
    bad.places.push(jfn_model::Place { id: 51, kind: PlaceKind::Floor, name: s("A"), parent: 52, ..jfn_model::Place::default() });
    bad.places.push(jfn_model::Place { id: 52, kind: PlaceKind::Floor, name: s("B"), parent: 51, ..jfn_model::Place::default() });
    let tb = Tree::of(&bad);
    t.eq("floors in floors: a building's size", tb.size(51), (400, 250));
    t.eq("their path ends", tb.path(51).len(), 2);
}

fn undo(t: &mut T) {
    let mut doc = Doc::new();
    doc.replace(campus(), None);
    let names = phys::DEFAULT_NAMES;
    let snap = |d: &Doc| (d.proj.places.clone(), d.proj.devices.iter().map(|x| (x.phys.clone(), x.optics.clone())).collect::<Vec<_>>(), d.proj.links.iter().map(|l| (l.len_m, l.cable)).collect::<Vec<_>>());
    let up = |d: &Doc| d.sim.engine().link_idx_up(L_PC4);
    let len = |d: &Doc| d.sim.engine().cable_phys(L_PC4).map_or(0, |c| c.len_m);
    t.check("PC4's cable is down in the window's simulator", !up(&doc) && len(&doc) == 208);
    let steps: Vec<(&str, alloc::boxed::Box<dyn Fn(&mut Project) -> Result<(), PhysErr>>)> = vec![
        ("add a room", alloc::boxed::Box::new(move |p: &mut Project| phys::add_place(p, PlaceKind::Room, 3, "Raum", &names).map(|_| ()))),
        ("rename", alloc::boxed::Box::new(move |p: &mut Project| phys::rename_place(p, 7, "Großraum", &names))),
        ("move and resize", alloc::boxed::Box::new(move |p: &mut Project| phys::set_place(p, 7, Some((130, 30)), Some((120, 90)), None, None, &names))),
        ("rack height", alloc::boxed::Box::new(move |p: &mut Project| phys::set_place(p, 6, None, None, Some(24), None, &names))),
        ("storey", alloc::boxed::Box::new(move |p: &mut Project| phys::set_place(p, 10, None, None, None, Some(2), &names))),
        ("PC4 into Büro 1", alloc::boxed::Box::new(move |p: &mut Project| phys::place_device(p, PC4, 7, 0, Some((60, 50)), &names))),
        ("SRV1 to unit 20", alloc::boxed::Box::new(move |p: &mut Project| phys::place_device(p, SRV1, 6, 20, None, &names))),
        ("a length by hand", alloc::boxed::Box::new(|p: &mut Project| phys::set_len(p, 22, 30))),
        ("an optic", alloc::boxed::Box::new(|p: &mut Project| phys::set_optic(p, SW1, "Te1/1/1", Optic::Lr))),
        ("remove Haus B", alloc::boxed::Box::new(move |p: &mut Project| phys::remove_place(p, 12, &names))),
    ];
    for (name, f) in steps {
        let before = snap(&doc);
        let r = crate::doc::phys::step(&mut doc, "x", |p| f(p));
        let after = snap(&doc);
        t.check(&format!("{}: one step", name), r == Ok(true) && after != before);
        t.check(&format!("{}: undo", name), doc.undo() && snap(&doc) == before);
        t.check(&format!("{}: redo", name), doc.redo() && snap(&doc) == after);
    }
    t.check("the simulator followed: PC4 in Büro 1 is up", up(&doc) && len(&doc) < 100);
    t.check("a refused edit is no step", crate::doc::phys::step(&mut doc, "x", |p| phys::remove_place(p, 999, &names)) == Err(PhysErr::UnknownPlace(999)));
    t.check("nothing changed is no step", crate::doc::phys::step(&mut doc, "x", |p| phys::set_len(p, 22, 30)) == Ok(false));
    // back to the start: PC4 far away again, down again
    while doc.undo() {}
    t.check("all undone: down again", !up(&doc) && len(&doc) == 208 && doc.proj.places == campus().places);
    // a project without places: the first step stores the default tree, undo takes it away
    let mut d2 = Doc::new();
    d2.replace(Project::from_text(include_str!("../tests/fixtures/two_pcs.jfnet")).unwrap_or_default(), None);
    t.check("a rack in the default room", crate::doc::phys::step(&mut d2, "x", |p| phys::add_place(p, PlaceKind::Rack, 4, "Rack", &names).map(|_| ())) == Ok(true) && d2.proj.places.len() == 5);
    t.check("undone: no places again", d2.undo() && d2.proj.places.is_empty());
    // a device placed by both ends: the delay follows the length in the engine
    let mut d3 = Doc::new();
    d3.replace(Project::from_text(include_str!("../tests/fixtures/two_pcs.jfnet")).unwrap_or_default(), None);
    let link = d3.proj.links.first().map_or(0, |l| l.id);
    let (a, b) = d3.proj.links.first().map_or((0, 0), |l| (l.a.dev, l.b.dev));
    let _ = crate::doc::phys::step(&mut d3, "x", |p| {
        phys::place_device(p, a, 4, 0, Some((10, 10)), &names)?;
        phys::place_device(p, b, 4, 0, Some((60, 10)), &names)
    });
    let tr = Tree::of(&d3.proj);
    let want = d3.proj.link(link).map_or(0, |l| phys::length(&d3.proj, &tr, l).m) as u64 * 5;
    t.check("the engine's delay follows the plan", d3.sim.engine().link_rt(link).is_some_and(|l| l.delay_ns == want) && want != 50);
}

fn file(t: &mut T) {
    let mut p = campus();
    let _ = phys::set_optic(&mut p, SW1, "Te1/1/1", Optic::Lr);
    let _ = phys::set_len(&mut p, 22, 33);
    p.layout.phys = true;
    p.layout.phys_at = 5;
    let text = p.to_text();
    t.check("in the file", text.contains("\"places\": [") && text.contains("\"kind\": \"rack\"") && text.contains("\"len_m\": 33") && text.contains("\"TenGigabitEthernet1/1/1\": \"lr\"") && text.contains("\"phys_at\": 5"));
    let q = Project::from_text(&text);
    t.check("read back the same", q.as_ref().is_ok_and(|q| q.places == p.places && q.devices == p.devices && q.links == p.links && q.layout == p.layout));
    // the view's choice in the layout (workspace/phys: store_view before saving, reset on loading)
    let mut v = crate::workspace::phys::Phys::new();
    v.reset(&p);
    t.check("the view comes back from the layout", v.on && v.at == 5);
    let mut q2 = campus();
    v.store_view(&mut q2);
    t.eq("and goes into it", (q2.layout.phys, q2.layout.phys_at), (true, 5));
    let mut w = crate::workspace::phys::Phys::new();
    w.reset(&campus());
    t.check("a file without a choice: logical, the default room", !w.on && w.at == 4);
    w.store_view(&mut q2);
    t.eq("the default room is stored as 0", (q2.layout.phys, q2.layout.phys_at), (false, 0));
    let old = Project::from_text(include_str!("../tests/fixtures/two_pcs.jfnet")).unwrap_or_default();
    t.check("an older file: nothing physical", old.places.is_empty() && old.devices.iter().all(|d| d.phys == jfn_model::Spot::default() && d.optics.is_empty()) && old.links.iter().all(|l| l.len_m == 0) && !old.layout.phys);
}

fn scenarios(t: &mut T) {
    let sc = scenario::parse("place PC4 \"Büro 1\"\nplace SW2 \"Rack B1\" 12\ncable SW1 Gi1/0/5 length 132\ncable SW1 Gi1/0/5 length auto\nexpect cable SW1 Gi1/0/5 length <= 100\nexpect cable SW1 Gi1/0/5 length 21m\nexpect cable SW1 Gi1/0/5 up\nexpect cable PC4 eth0 down\n");
    let kinds = sc.map(|x| x.steps.into_iter().map(|s| s.kind).collect::<Vec<_>>());
    let cable = |dev: &str, port: &str, check: CableCheck| StepKind::Expect(Expect::Cable { dev: s(dev), port: s(port), check });
    t.eq(
        "the steps",
        kinds,
        Ok(vec![
            StepKind::Place { dev: s("PC4"), place: s("Büro 1"), unit: 0 },
            StepKind::Place { dev: s("SW2"), place: s("Rack B1"), unit: 12 },
            StepKind::CableLen { dev: s("SW1"), port: s("Gi1/0/5"), m: 132 },
            StepKind::CableLen { dev: s("SW1"), port: s("Gi1/0/5"), m: 0 },
            cable("SW1", "Gi1/0/5", CableCheck::Len(Cmp::Le, 100)),
            cable("SW1", "Gi1/0/5", CableCheck::Len(Cmp::Eq, 21)),
            cable("SW1", "Gi1/0/5", CableCheck::Up(true)),
            cable("PC4", "eth0", CableCheck::Up(false)),
        ]),
    );
    t.check("bad ones", scenario::parse("place PC4\n").is_err() && scenario::parse("place SW2 \"Rack B1\" 0\n").is_err() && scenario::parse("cable SW1 Gi1/0/5 length lang\n").is_err());
    t.eq("as a check", jfn_sim::goal::check_text(&Expect::Cable { dev: s("SW1"), port: s("Gi1/0/5"), check: CableCheck::Len(Cmp::Le, 100) }), s("cable SW1 Gi1/0/5 length <= 100"));
    let text = include_str!("../tests/scen/phys_lengths.scn");
    let Ok(sc) = scenario::parse(text) else {
        t.check("phys_lengths.scn parses", false);
        return;
    };
    let p = campus();
    let a = jfn_sim::run_scenario(&p, &sc, "phys_lengths", false);
    let b = jfn_sim::run_scenario(&p, &sc, "phys_lengths", false);
    t.check(&format!("the scenario holds ({} of {})", a.expects - a.failed, a.expects), a.ok && a.expects > 0);
    t.check("twice the same report", a.report == b.report);
}
