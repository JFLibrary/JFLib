// cablestest -- checks of the choice of cables (group cables of --selftest; jfn_model::cable, the
// categories in phys::reach_graded, jfn_sim's Action::CableGrade and Action::StrictMdix, what
// canvas/connect.rs and paint.rs decide): every kind on every pair of media it fits and is refused
// on, the limits per category, older files with the same kinds and link states, a round trip with
// categories and colours, the ranking for a chosen kind and its reasons, the colours against the
// canvas, "show interfaces" where a kind changes what it says, the same run twice
//
// The file of an older version (tests/fixtures/cables_v1.jfnet) holds the eight kinds there were,
// without "grade" and "color".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::cable::{self, CableColor, CableGrade, CableKind, Why};
use jfn_model::catalog::{self, Media, Optic, PortSpec, Poe};
use jfn_model::ifname;
use jfn_model::lab::{Assess, Item};
use jfn_model::phys;
use jfn_model::project::{PortRef, ProjErr, Project};
use jfn_model::scenario;
use jfn_sim::assess::Grader;
use jfn_sim::{Action, Engine, Led, Note, PhysFault, PhysNote, ProblemKind, Report, SEC};
use ui::{contrast_ratio, Style};

use crate::selftest::T;
use crate::workspace::canvas::{connect, paint};

const BUDGET: usize = 20_000_000;
const OLD: &str = include_str!("../tests/fixtures/cables_v1.jfnet");
const GRADES: &str = include_str!("../tests/fixtures/cable_grades.jfnet");
const KINDS: &str = include_str!("../tests/fixtures/cable_kinds.jfnet");

fn s(x: &str) -> String {
    String::from(x)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

/// what a device printed for a command line
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let at = e.now() + SEC;
    e.run_until(at, BUDGET);
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

/// a port of a medium (mdix for the two kinds of copper ports)
fn port(media: Media, mdix: bool, index: u16) -> PortSpec {
    let optic = if media.is_optical() { Some(Optic::Sr) } else { None };
    PortSpec { name: format!("p{}", index), index, media, kbps: 1_000_000, mdix, poe: Poe::None, optic }
}

const MEDIA: [Media; 13] = [Media::Rj45, Media::Sfp, Media::SfpPlus, Media::Sfp28, Media::Qsfp28, Media::Serial, Media::Console, Media::Usb, Media::RadioAp, Media::RadioClient, Media::Wan, Media::Rj11, Media::Coax];

// which pairs a kind plugs into (independent of cable.rs)
fn want(k: CableKind, a: Media, b: Media) -> bool {
    let sfp = |m: Media| matches!(m, Media::Sfp | Media::SfpPlus | Media::Sfp28);
    let cages = |x: Media, y: Media| (sfp(x) && sfp(y)) || (x == Media::Qsfp28 && y == Media::Qsfp28);
    match k {
        CableKind::CopperStraight | CableKind::CopperCross => a == Media::Rj45 && b == Media::Rj45,
        CableKind::FiberMm | CableKind::FiberSm => cages(a, b),
        CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte => a == Media::Serial && b == Media::Serial,
        CableKind::Console => matches!((a, b), (Media::Console, Media::Usb) | (Media::Usb, Media::Console)),
        CableKind::Wireless => matches!((a, b), (Media::RadioAp, Media::RadioClient) | (Media::RadioClient, Media::RadioAp)),
        CableKind::Wan => a == Media::Wan && b == Media::Wan,
        CableKind::Phone => matches!((a, b), (Media::Rj11, Media::Rj11) | (Media::Rj11, Media::Wan) | (Media::Wan, Media::Rj11) | (Media::Wan, Media::Wan)),
        CableKind::Coax => matches!((a, b), (Media::Coax, Media::Coax) | (Media::Coax, Media::Wan) | (Media::Wan, Media::Coax) | (Media::Wan, Media::Wan)),
        CableKind::Dac | CableKind::Aoc => cages(a, b) && a != Media::Sfp && b != Media::Sfp,
    }
}

// every kind on every pair of media
fn fits(t: &mut T) {
    t.eq("kinds", CableKind::ALL.len(), 14);
    t.eq("the old kinds keep their places", CableKind::ALL[..8].iter().map(|k| k.as_str()).collect::<Vec<_>>(), alloc::vec!["copper", "copper-cross", "fiber-mm", "fiber-sm", "serial", "console", "wireless", "wan"]);
    for &k in CableKind::ALL {
        let mut bad: Vec<String> = Vec::new();
        for &a in &MEDIA {
            for &b in &MEDIA {
                let got = cable::cable_fits(k, &port(a, false, 0), &port(b, true, 1)).is_ok();
                if got != want(k, a, b) {
                    bad.push(format!("{:?}-{:?}: {}", a, b, got));
                }
            }
        }
        t.eq(&format!("{} on every pair", k.as_str()), bad, Vec::new());
        t.eq(&format!("{} on its wire name", k.as_str()), CableKind::parse(k.as_str()), Some(k));
        t.check(&format!("{} has a tile name", k.as_str()), !k.tile().is_empty());
    }
    // the reasons
    let (rj, sp, qs, se) = (port(Media::Rj45, false, 0), port(Media::SfpPlus, false, 1), port(Media::Qsfp28, false, 2), port(Media::Serial, false, 3));
    let (sf, rj11, coax, wan) = (port(Media::Sfp, false, 4), port(Media::Rj11, false, 5), port(Media::Coax, false, 6), port(Media::Wan, false, 7));
    t.eq("fibre into RJ45", cable::cable_fits(CableKind::FiberMm, &rj, &rj), Err(Why::CableDoesNotFit(CableKind::FiberMm, Media::Rj45)));
    t.eq("copper into a cage", cable::cable_fits(CableKind::CopperStraight, &sp, &sp), Err(Why::CableDoesNotFit(CableKind::CopperStraight, Media::SfpPlus)));
    t.eq("no common cable", cable::cable_fits(CableKind::Dac, &sp, &qs), Err(Why::MediaMismatch(Media::SfpPlus, Media::Qsfp28)));
    t.eq("DAC into plain SFP", cable::cable_fits(CableKind::Dac, &sf, &sp), Err(Why::CableDoesNotFit(CableKind::Dac, Media::Sfp)));
    t.eq("the serial kinds", (cable::cable_fits(CableKind::SerialDte, &se, &se), cable::cable_kind(&se, &se)), (Ok(()), Ok(CableKind::Serial)));
    t.eq("telephone", (cable::cable_kind(&rj11, &rj11), cable::cable_kind(&rj11, &wan), cable::cable_kind(&wan, &rj11)), (Ok(CableKind::Phone), Ok(CableKind::Phone), Ok(CableKind::Phone)));
    t.eq("coax", (cable::cable_kind(&coax, &coax), cable::cable_kind(&coax, &wan)), (Ok(CableKind::Coax), Ok(CableKind::Coax)));
    t.eq("telephone is no coax", cable::cable_kind(&rj11, &coax), Err(Why::MediaMismatch(Media::Rj11, Media::Coax)));
    t.check("the serial ends", CableKind::SerialDce.dce_end() == Some(0) && CableKind::SerialDte.dce_end() == Some(1) && CableKind::Serial.dce_end().is_none() && CableKind::CopperStraight.dce_end().is_none());
    t.check("ethernet or not", CableKind::Phone.is_ethernet() && CableKind::Coax.is_ethernet() && CableKind::Dac.is_ethernet() && CableKind::Aoc.is_ethernet() && !CableKind::SerialDce.is_ethernet() && !CableKind::SerialDte.is_ethernet() && !CableKind::Console.is_ethernet());
    t.eq("delays", [CableKind::SerialDte, CableKind::Phone, CableKind::Coax, CableKind::Dac, CableKind::Aoc].map(cable::default_delay_ns), [2_000_000, 5_000_000, 5_000_000, 15, 50]);
}

// the new models and the ports on them
fn models(t: &mut T) {
    let port_of = |m: &str, p: &str| catalog::model(m).and_then(|x| x.port(p)).map(|x| (x.media, x.kbps, x.index));
    t.eq("the cable modem's line", port_of("cable-modem", "coax0"), Some((Media::Coax, catalog::K1G, 0)));
    t.eq("the cable modem's LAN", port_of("cable-modem", "lan0"), Some((Media::Rj45, catalog::K1G, 1)));
    t.eq("the DSL modem's line", port_of("dsl-modem", "dsl0"), Some((Media::Rj11, 100_000, 0)));
    t.eq("the TV keeps its first ports", (port_of("smart-tv", "eth0"), port_of("smart-tv", "wlan0")), (Some((Media::Rj45, catalog::K1G, 0)), Some((Media::RadioClient, 1_200_000, 1))));
    t.eq("the TV's coax", port_of("smart-tv", "coax0"), Some((Media::Coax, catalog::K1G, 2)));
    let mut p = Project::new("modems", 1);
    let (cm, dsl, net, tv, pc) = (p.add_device("cable-modem", 0, 0).unwrap_or(0), p.add_device("dsl-modem", 0, 60).unwrap_or(0), p.add_device("cloud", 200, 0).unwrap_or(0), p.add_device("smart-tv", 400, 0).unwrap_or(0), p.add_device("pc", 400, 60).unwrap_or(0));
    t.eq("the names", p.device(cm).map(|d| d.name.clone()), Some(s("Modem1")));
    let lc = p.connect(&pr(cm, "coax0"), &pr(net, "wan0"), None).unwrap_or(0);
    let ld = p.connect(&pr(dsl, "dsl0"), &pr(net, "wan1"), None).unwrap_or(0);
    let lt = p.connect(&pr(tv, "coax0"), &pr(net, "wan2"), Some(CableKind::Coax)).unwrap_or(0);
    t.eq("their cables", [lc, ld, lt].map(|l| p.link(l).map(|x| (x.cable, x.cable_fixed))), [Some((CableKind::Coax, false)), Some((CableKind::Phone, false)), Some((CableKind::Coax, true))]);
    t.eq("the DSL line's speed", p.link(ld).map(|x| x.kbps), Some(100_000));
    t.check("a PC has no coax", matches!(p.connect(&pr(pc, "eth0"), &pr(net, "wan3"), Some(CableKind::Coax)), Err(ProjErr::Cable(_))));
    t.check("no issue", p.check().iter().all(|x| !x.error));
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    t.check("the lines are up", [lc, ld, lt].iter().all(|&l| e.link_idx_up(l)));
    t.check("every model has its leaf", catalog::catalog().iter().filter(|m| !m.has(catalog::F_ANNOTATION)).all(|m| jfn_sim::snmpmib::MODEL_LEAVES.iter().any(|x| x.0 == m.id)));
    t.check("every medium has a name and an icon", MEDIA.iter().all(|&m| !m.name().is_empty() && crate::workspace::icons::port_icon(m) != 0));
    t.eq("the summary", catalog::model("cable-modem").map(|m| m.summary()), Some(s("1× Koax, 1× 1G")));
    t.eq("the summary of DSL", catalog::model("dsl-modem").map(|m| m.summary()), Some(s("1× RJ11, 1× 1G")));
}

// the limits per category
fn limits(t: &mut T) {
    let m = |k: CableKind, g: CableGrade, kbps: u32, o: Option<Optic>| phys::reach_graded(k, g, kbps, o).map(|r| r.max_m);
    let c = CableKind::CopperStraight;
    let row = |g: CableGrade| [10_000, catalog::K1G, 2_500_000, 5_000_000, catalog::K10G, catalog::K25G, 40_000_000].map(|k| m(c, g, k, None).unwrap_or(99_999));
    t.eq("no category: as before", row(CableGrade::Unset), [100, 100, 100, 100, 100, 30, 30]);
    t.eq("Cat5e", row(CableGrade::Cat5e), [100, 100, 100, 100, 0, 0, 0]);
    t.eq("Cat6", row(CableGrade::Cat6), [100, 100, 100, 100, 55, 0, 0]);
    t.eq("Cat6a", row(CableGrade::Cat6a), [100, 100, 100, 100, 100, 0, 0]);
    t.eq("Cat7", row(CableGrade::Cat7), [100, 100, 100, 100, 100, 0, 0]);
    t.eq("Cat8", row(CableGrade::Cat8), [100, 100, 100, 100, 100, 30, 30]);
    t.eq("crossover too", m(CableKind::CopperCross, CableGrade::Cat6, catalog::K10G, None), Some(55));
    let sr = Some(Optic::Sr);
    let f = |g: CableGrade| [catalog::K1G, catalog::K10G, catalog::K25G, 40_000_000, catalog::K100G].map(|k| m(CableKind::FiberMm, g, k, sr).unwrap_or(99_999));
    t.eq("multimode: as before", f(CableGrade::Unset), [550, 300, 70, 100, 70]);
    t.eq("OM3", f(CableGrade::Om3), [550, 300, 70, 100, 70]);
    t.eq("OM4", f(CableGrade::Om4), [550, 400, 100, 150, 100]);
    let lr = Some(Optic::Lr);
    let g = |g: CableGrade| [catalog::K1G, catalog::K10G, catalog::K100G].map(|k| m(CableKind::FiberSm, g, k, lr).unwrap_or(99_999));
    t.eq("OS2", g(CableGrade::Os2), g(CableGrade::Unset));
    t.eq("OS2's lengths", g(CableGrade::Os2), [5_000, 10_000, 10_000]);
    t.eq("the standard stays", phys::reach_graded(c, CableGrade::Cat6, catalog::K10G, None).map(|r| r.std), Some("10GBASE-T"));
    // a category its kind cannot have counts as none
    t.check("what fits", CableGrade::Cat6.fits(CableKind::CopperCross) && CableGrade::Om4.fits(CableKind::FiberMm) && CableGrade::Os2.fits(CableKind::FiberSm) && CableGrade::Unset.fits(CableKind::Wan));
    t.check("what does not", !CableGrade::Cat6.fits(CableKind::FiberMm) && !CableGrade::Om4.fits(CableKind::FiberSm) && !CableGrade::Os2.fits(CableKind::FiberMm) && !CableGrade::Cat8.fits(CableKind::Dac));
    t.eq("OM4 on single-mode", m(CableKind::FiberSm, CableGrade::Om4, catalog::K10G, lr), Some(10_000));
    t.eq("Cat5e on fibre", m(CableKind::FiberMm, CableGrade::Cat5e, catalog::K10G, sr), Some(300));
    // the kinds without optics, the lines
    t.eq("DAC", [catalog::K10G, catalog::K25G, catalog::K100G].map(|k| phys::reach(CableKind::Dac, k, None).map(|r| (r.max_m, r.std))), [Some((7, "SFP+ DAC")), Some((7, "SFP28 DAC")), Some((7, "QSFP28 DAC"))]);
    t.eq("AOC", [catalog::K10G, catalog::K25G, catalog::K100G].map(|k| phys::reach(CableKind::Aoc, k, None).map(|r| (r.max_m, r.std))), [Some((100, "SFP+ AOC")), Some((100, "SFP28 AOC")), Some((100, "QSFP28 AOC"))]);
    t.eq("telephone and coax", (phys::reach(CableKind::Phone, 100_000, None).map(|r| r.max_m), phys::reach(CableKind::Coax, catalog::K1G, None).map(|r| r.max_m)), (Some(1_000), Some(500)));
    t.eq("the chosen ends like serial", phys::reach(CableKind::SerialDte, 64, None), phys::reach(CableKind::Serial, 64, None));
    t.eq("no optics count", (phys::optic_fault(CableKind::Dac, Some(Optic::Lr), Some(Optic::Sr)), phys::optic_fault(CableKind::Aoc, Some(Optic::Lr), Some(Optic::Sr))), (None, None));
    // the tooltip's text comes from the table
    t.eq("limits of copper", cable::limits_msg(CableKind::CopperStraight, CableGrade::Unset).text(), s("100 m bei 1G, 100 m bei 10G, 30 m bei 25G"));
    t.eq("limits of Cat6", cable::limits_msg(CableKind::CopperCross, CableGrade::Cat6).text(), s("100 m bei 1G, 55 m bei 10G, nicht bei 25G"));
    t.eq("limits of DAC", cable::limits_msg(CableKind::Dac, CableGrade::Unset).text(), s("7 m bei 10G, 7 m bei 100G"));
    t.eq("limits of a WAN line", cable::limits_msg(CableKind::Wan, CableGrade::Unset).text(), s("keine Längengrenze"));
    let en = |k: &'static str| -> &'static str {
        include_str!("locales/engine.tsv").lines().filter_map(|l| l.split_once('\t')).find(|r| r.0 == k).map_or(k, |r| r.1.trim_end())
    };
    t.eq("limits in English", cable::limits_msg(CableKind::Aoc, CableGrade::Unset).text_with(&en), s("100 m at 10G, 100 m at 100G"));
}

// a file of an older version
fn old_files(t: &mut T) {
    let Ok(p) = Project::from_text(OLD) else {
        t.check("the old file loads", false);
        return;
    };
    let kinds: Vec<(u32, CableKind, bool)> = p.links.iter().map(|l| (l.id, l.cable, l.cable_fixed)).collect();
    t.eq(
        "the same kinds",
        kinds,
        alloc::vec![
            (20, CableKind::CopperStraight, false),
            (21, CableKind::CopperCross, false),
            (22, CableKind::FiberMm, false),
            (23, CableKind::FiberSm, true),
            (24, CableKind::Serial, false),
            (25, CableKind::Console, false),
            (26, CableKind::Wireless, false),
            (27, CableKind::Wan, false),
            (28, CableKind::CopperStraight, false),
        ],
    );
    t.check("no category, no colour", p.links.iter().all(|l| l.grade == CableGrade::Unset && l.color == CableColor::Plain));
    t.check("no error in it", p.check().iter().all(|x| !x.error));
    let mut e = engine(&p);
    e.run_until(2 * SEC, BUDGET);
    let up: Vec<u32> = p.links.iter().map(|l| l.id).filter(|&l| e.link_idx_up(l)).collect();
    // fibre 23 is single-mode on SR optics: down as ever (phys.rs); a router's ports (24) start
    // shut down
    t.eq("the same link states", up, alloc::vec![20, 21, 22, 25, 26, 27, 28]);
    t.eq("why 23 is down", e.cable_phys(23).and_then(|c| c.fault), Some(PhysFault::Optics(phys::OpticFault::WrongFiber(Optic::Sr))));
    t.eq("the old serial line says no end", cli(&mut e, 4, "show interfaces Serial0/2/0").contains("Serial cable end"), false);
    // the older fixtures say nothing of it either
    for (name, text) in [("two_pcs", include_str!("../tests/fixtures/two_pcs.jfnet")), ("office", include_str!("../tests/fixtures/office.jfnet")), ("serial_wan", include_str!("../tests/fixtures/serial_wan.jfnet"))] {
        let q = Project::from_text(text).unwrap_or_default();
        t.check(&format!("{}: as before", name), !q.links.is_empty() && !text.contains("\"grade\"") && q.links.iter().all(|l| l.grade == CableGrade::Unset && l.color == CableColor::Plain));
    }
}

// a save and a load with categories and colours
fn round_trip(t: &mut T) {
    let Ok(mut p) = Project::from_text(KINDS) else {
        t.check("the kinds' file loads", false);
        return;
    };
    t.check("no issue in it", p.check().is_empty());
    t.eq("it has every kind", CableKind::ALL.iter().filter(|&&k| !p.links.iter().any(|l| l.cable == k)).count(), 0);
    if let Some(l) = p.link_mut(50) {
        l.grade = CableGrade::Cat8;
        l.color = CableColor::Teal;
    }
    let q = Project::from_text(&p.to_text()).unwrap_or_default();
    t.check("the same after a round trip", p.links == q.links && p.strict_mdix == q.strict_mdix);
    t.eq("what the file says", q.link(50).map(|l| (l.grade, l.color)), Some((CableGrade::Cat8, CableColor::Teal)));
    t.eq("every name", CableGrade::ALL.iter().map(|g| CableGrade::parse(g.as_str()) == Some(*g)).filter(|x| !x).count() + CableColor::ALL.iter().map(|c| CableColor::parse(c.as_str()) == Some(*c)).filter(|x| !x).count(), 0);
    t.check("the member's names", p.to_text().contains("\"grade\": \"cat8\"") && p.to_text().contains("\"color\": \"teal\""));
    // what no program of this version writes: a category the kind cannot have is a warning
    if let Some(l) = p.link_mut(50) {
        l.grade = CableGrade::Os2;
    }
    let issues = p.check();
    t.eq("a category that does not fit", issues.iter().map(|x| (x.error, x.link, x.text.clone())).collect::<Vec<_>>(), alloc::vec![(false, Some(50), s("Die Kategorie OS2 passt nicht zu Kupfer, gerade; es zählen die Grenzen ohne Kategorie."))]);
    let mut s1 = Project::new("mdix", 1);
    s1.strict_mdix = true;
    t.check("the setting round trips", Project::from_text(&s1.to_text()).is_ok_and(|x| x.strict_mdix));
}

// the engine: categories, the setting, the same run twice
fn engine_part(t: &mut T) {
    let p = Project::from_text(GRADES).unwrap_or_default();
    let mut e = engine(&p);
    e.run_until(2 * SEC, BUDGET);
    t.eq("Cat5e at 10G", e.cable_phys(20).and_then(|c| c.fault), Some(PhysFault::Grade { grade: CableGrade::Cat5e, std: "10GBASE-T" }));
    t.eq("Cat6 over 55 m", e.cable_phys(21).and_then(|c| c.fault), Some(PhysFault::TooLong { len_m: 60, max_m: 55, std: "10GBASE-T" }));
    t.eq("Cat6a at 60 m", e.cable_phys(22).and_then(|c| c.fault), None);
    // (the routers' ports, 23 and 24, start shut down)
    t.eq("up or down", [20, 21, 22, 23, 24, 25, 26, 27].map(|l| e.link_idx_up(l)), [false, false, true, false, false, true, true, true]);
    t.eq("the reach", e.cable_phys(21).and_then(|c| c.reach).map(|r| r.max_m), Some(55));
    let dc1 = 1;
    let port = e.port_index(dc1, "Te1/0/1").unwrap_or(99);
    t.check("the note", e.trace.events().any(|x| x.dev == dc1 && x.note == Note::Phys(PhysNote::Grade { port, grade: CableGrade::Cat5e, std: "10GBASE-T" })));
    let text = Note::Phys(PhysNote::Grade { port: 0, grade: CableGrade::Cat5e, std: "10GBASE-T" }).message(&|_| s("Te1/0/1"), &|_| s(""));
    t.eq("the note's sentence", text.text(), s("Cat5e-Kabel an Te1/0/1 trägt 10GBASE-T nicht; die Leitung bleibt aus."));
    let probs = e.problems();
    let pr = probs.iter().find(|x| x.kind == ProblemKind::CableGrade);
    t.eq("the problem", pr.map(|x| (x.link, x.text.clone())), Some((Some(20), s("Cat5e-Kabel zwischen DC1 Te1/0/1 und DC2 Te1/0/1 trägt 10GBASE-T nicht; die Leitung bleibt aus."))));
    let sh = cli(&mut e, dc1, "show interfaces TenGigabitEthernet1/0/1");
    t.check("show interfaces says it", sh.contains("is down, line protocol is down") && sh.contains("  A Cat5e cable does not carry 10GBASE-T: no link"));
    // a better cable: up; the action counts
    t.check("Cat6a by action", e.apply(Action::CableGrade { link: 20, grade: CableGrade::Cat6a }).is_ok() && e.link_idx_up(20) && e.problems().iter().all(|x| x.kind != ProblemKind::CableGrade));
    t.check("no category: up", e.apply(Action::CableGrade { link: 20, grade: CableGrade::Unset }).is_ok() && e.link_idx_up(20));
    t.check("Cat5e again: down", e.apply(Action::CableGrade { link: 20, grade: CableGrade::Cat5e }).is_ok() && !e.link_idx_up(20));
    t.check("an unknown cable is refused", e.apply(Action::CableGrade { link: 999, grade: CableGrade::Cat6 }).is_err());
    // what a kind says
    let sh = cli(&mut e, dc1, "show interfaces TenGigabitEthernet1/0/3");
    t.check("the category in the media type", sh.contains("media type is RJ45 Cat6a"));
    t.check("a cable without one as before", !cli(&mut e, 5, "show interfaces TenGigabitEthernet1/1/4").contains("media type is 10G SFP+ "));
    t.check("DAC", cli(&mut e, 5, "show interfaces TenGigabitEthernet1/1/1").contains("media type is 10G SFP+ DAC"));
    t.check("AOC", cli(&mut e, 5, "show interfaces TenGigabitEthernet1/1/2").contains("media type is 10G SFP+ AOC"));
    t.check("OM4", cli(&mut e, 5, "show interfaces TenGigabitEthernet1/1/3").contains("media type is 10G SFP+ OM4"));
    t.check("the first end DCE", cli(&mut e, 3, "show interfaces Serial0/2/0").contains("  Serial cable end: DCE\n"));
    t.check("the far end DTE", cli(&mut e, 4, "show interfaces Serial0/2/0").contains("  Serial cable end: DTE\n"));
    t.check("the first end DTE", cli(&mut e, 3, "show interfaces Serial0/2/1").contains("  Serial cable end: DTE\n"));
    t.check("its far end DCE", cli(&mut e, 4, "show interfaces Serial0/2/1").contains("  Serial cable end: DCE\n"));
    // a task that asks for a serial line takes either chosen end, one that names an end only it
    let kind = |k: &str| Item { what: s("cable"), dev: s("R1"), port: s("Se0/2/0"), peer: s("R2"), peer_port: s("Se0/2/0"), points: 1, value: Value::string(k), ..Item::default() };
    let task = Assess { items: alloc::vec![kind("serial"), kind("serial-dce"), kind("serial-dte")], ..Assess::default() };
    t.eq("the grade of a chosen end", Grader::new(&task).grade(&p, &e, &|_| String::new()).items.iter().map(|g| g.held).collect::<Vec<_>>(), alloc::vec![true, true, false]);
    // a chosen serial end carries as a serial line does
    t.check("HDLC on it", cli(&mut e, 3, "show interfaces Serial0/2/0").contains("Encapsulation HDLC"));
    // Auto-MDI-X off
    let mut q = Project::new("mdix", 3);
    let (pc, sw) = (q.add_device("pc", 0, 0).unwrap_or(0), q.add_device("jf-s24p", 200, 0).unwrap_or(0));
    let l = q.connect(&PortRef { dev: pc, port: s("eth0") }, &PortRef { dev: sw, port: s("Gi1/0/1") }, Some(CableKind::CopperCross)).unwrap_or(0);
    let mut e = engine(&q);
    e.run_until(SEC, BUDGET);
    let led = |e: &Engine| e.view().links.iter().find(|x| x.id == l).map(|x| x.led);
    t.check("auto-MDI-X: the wrong cable works", e.link_idx_up(l) && led(&e) == Some([Led::Up, Led::Up]));
    t.check("off: it does not", e.apply(Action::StrictMdix { on: true }).is_ok() && !e.link_idx_up(l) && led(&e) == Some([Led::Down, Led::Down]));
    t.check("its problem", e.problems().iter().any(|x| x.kind == ProblemKind::WrongCable && x.link == Some(l)));
    t.check("on again: it works", e.apply(Action::StrictMdix { on: false }).is_ok() && e.link_idx_up(l) && e.problems().iter().all(|x| x.kind != ProblemKind::WrongCable));
    // the scenario twice
    let text = include_str!("../tests/scen/cable_grades.scn");
    let Ok(sc) = scenario::parse(text) else {
        t.check("cable_grades.scn parses", false);
        return;
    };
    let a = jfn_sim::run_scenario(&p, &sc, "cable_grades", false);
    let b = jfn_sim::run_scenario(&p, &sc, "cable_grades", false);
    t.check(&format!("the scenario holds ({} of {})", a.expects - a.failed, a.expects), a.ok && a.expects > 0);
    t.check("twice the same report", a.report == b.report);
}

// what the canvas decides for a chosen kind
fn choose(t: &mut T) {
    let mut p = Project::new("choose", 1);
    let (pc, sw, r1, r2, sw2) = (p.add_device("pc", 0, 0).unwrap_or(0), p.add_device("jf-s24p", 100, 0).unwrap_or(0), p.add_device("jf-r4", 200, 0).unwrap_or(0), p.add_device("jf-r4", 300, 0).unwrap_or(0), p.add_device("jf-s24p", 400, 0).unwrap_or(0));
    let port_at = |d: u32, i: u16| p.ports(d).into_iter().find(|x| x.0.index == i).map_or_else(String::new, |x| ifname::short(&x.0.name));
    let name_of = |pair: Option<(u16, u16)>, a: u32, b: u32| pair.map(|(x, y)| (port_at(a, x), port_at(b, y)));
    // nothing chosen: as ever
    t.eq("automatic: the fastest", name_of(connect::best(&p, r1, None, r2, None, None), r1, r2), Some((s("Te0/1/0"), s("Te0/1/0"))));
    t.eq("automatic is suggest_ports", p.suggest_ports(r1, r2).map(|x| (ifname::short(&x.0), ifname::short(&x.1))), Some((s("Te0/1/0"), s("Te0/1/0"))));
    // a kind chosen: the pair it fits
    t.eq("serial", name_of(connect::best(&p, r1, None, r2, None, Some(CableKind::SerialDte)), r1, r2), Some((s("Se0/2/0"), s("Se0/2/0"))));
    t.eq("copper", name_of(connect::best(&p, r1, None, r2, None, Some(CableKind::CopperCross)), r1, r2), Some((s("Gi0/0/0"), s("Gi0/0/0"))));
    t.eq("wrong copper is allowed", name_of(connect::best(&p, pc, None, sw, None, Some(CableKind::CopperCross)), pc, sw), Some((s("eth0"), s("Gi1/0/1"))));
    t.eq("fibre", name_of(connect::best(&p, sw, None, sw2, None, Some(CableKind::FiberSm)), sw, sw2), Some((s("Te1/1/1"), s("Te1/1/1"))));
    t.eq("DAC", name_of(connect::best(&p, sw, None, r1, None, Some(CableKind::Dac)), sw, r1), Some((s("Te1/1/1"), s("Te0/1/0"))));
    t.eq("a console cable", name_of(connect::best(&p, pc, None, sw, None, Some(CableKind::Console)), pc, sw), Some((s("usb0"), s("Con"))));
    // a port fixed on a side
    let se0 = p.ports(r1).into_iter().find(|x| x.0.media == Media::Serial).map_or(0, |x| x.0.index);
    t.eq("a dot and a kind that fits", name_of(connect::best(&p, r1, Some(se0), r2, None, Some(CableKind::SerialDce)), r1, r2), Some((s("Se0/2/0"), s("Se0/2/0"))));
    t.eq("a dot and a kind that does not", connect::best(&p, r1, Some(se0), r2, None, Some(CableKind::CopperCross)), None);
    // refused, with its reason
    t.eq("fibre into a PC", connect::best(&p, pc, None, sw, None, Some(CableKind::FiberMm)), None);
    t.eq("its reason", connect::refusal(&p, sw, None, pc, None, CableKind::FiberMm), s("Glasfaser, Multimode passt nicht in einen RJ45-Port."));
    t.eq("the start's reason", connect::refusal(&p, pc, None, sw, None, CableKind::FiberMm), s("Glasfaser, Multimode passt nicht in einen RJ45-Port."));
    t.eq("coax into a switch", connect::refusal(&p, r1, None, sw, None, CableKind::Coax), s("Koaxialkabel passt nicht in einen RJ45-Port."));
    let mut q = p.clone();
    let _ = q.connect(&pr(pc, "eth0"), &pr(sw, "Gi1/0/1"), None);
    let _ = q.connect(&pr(pc, "usb0"), &pr(sw, "Con"), None);
    t.eq("no free port", connect::refusal(&q, sw, None, pc, None, CableKind::CopperStraight), s("PC1 hat keinen freien Port"));
    let mut d = p.clone();
    let dc2 = d.add_device("jf-s48x", 500, 0).unwrap_or(0);
    t.eq("cages that do not join", connect::best(&d, sw, None, dc2, None, Some(CableKind::Dac)), None);
    t.eq("their reason", connect::refusal(&d, sw, None, dc2, None, CableKind::Dac), s("Direktkabel (DAC) verbindet keine freien Ports von SW1 und DCSW1."));
    t.eq("a start without a cage", connect::refusal(&p, pc, None, sw, None, CableKind::Dac), s("Direktkabel (DAC) passt nicht in einen RJ45-Port."));
    // what Project::connect makes of it
    let mut c = p.clone();
    let l = c.connect(&pr(r1, "Se0/2/0"), &pr(r2, "Se0/2/0"), Some(CableKind::SerialDte)).unwrap_or(0);
    t.eq("a chosen end sticks", c.link(l).map(|x| (x.cable, x.cable_fixed, x.kbps, x.delay_ns)), Some((CableKind::SerialDte, true, 2048, 2_000_000)));
    // the palette's tiles: one per kind, the plain serial line under "Automatisch"
    let (rj, mdix) = (port(Media::Rj45, false, 0), port(Media::Rj45, true, 1));
    let (sp, se) = (port(Media::SfpPlus, false, 2), port(Media::Serial, false, 3));
    t.eq("the list: the right kind once", connect::offered(&rj, &mdix, None), alloc::vec![CableKind::CopperCross]);
    t.eq("the list: the palette's choice of it", connect::offered(&rj, &mdix, Some(CableKind::CopperStraight)), alloc::vec![CableKind::CopperStraight, CableKind::CopperCross]);
    t.eq("the list of cages", connect::offered(&sp, &sp, None), alloc::vec![CableKind::FiberSm, CableKind::Dac, CableKind::Aoc]);
    t.eq("the list of a serial line", connect::offered(&se, &se, None), alloc::vec![CableKind::SerialDce, CableKind::SerialDte]);
    // the rows of the choice: a kind shows only the ports it plugs into
    let rows = connect::rows(&p, sw, sw2, Some(CableKind::FiberSm));
    t.eq("only the cages", rows.iter().map(|r| (r.spec.media, r.why.clone())).collect::<Vec<_>>(), alloc::vec![(Media::SfpPlus, None); 4]);
    t.eq("automatic: every port", connect::rows(&p, sw, sw2, None).len(), 29);
    t.eq("a port without a partner says so", connect::rows(&d, sw, dc2, Some(CableKind::Dac)).first().and_then(|r| r.why.clone()), Some(s("passt nicht zu RJ45, QSFP28, Konsole")));
    t.eq("the tiles", crate::workspace::palette::cable_kinds().count(), CableKind::ALL.len() - 1);
    t.check("no plain serial tile", crate::workspace::palette::cable_kinds().all(|k| k != CableKind::Serial));
    t.eq("the commands of the kinds", (0..CableKind::ALL.len()).map(crate::app::constants::cable_cmd).collect::<Vec<_>>(), alloc::vec![101, 102, 103, 104, 105, 106, 107, 108, 970, 971, 972, 973, 974, 975]);
}

// the drawing: colours against the canvas, fibre not by colour alone, status not in the colour
fn drawing(t: &mut T) {
    // (the canvas as selftest group ui has it)
    let canvas = |dark: bool| style::STYLE.tokens(dark)[(ui::T_BG / 4) as usize] | 0xFF00_0000;
    let themes = [(false, canvas(false), "hell"), (true, canvas(true), "dunkel"), (true, 0xFF00_0000 | plat::HcColors::BLACK.bg, "kontrast schwarz"), (false, 0xFF00_0000 | plat::HcColors::WHITE.bg, "kontrast weiß")];
    for (dark, bg, theme) in themes {
        for &c in CableColor::ALL.iter().filter(|&&c| c != CableColor::Plain) {
            let argb = paint::cable_tint(c, dark).unwrap_or(0);
            let r = contrast_ratio(argb, bg);
            t.check(&format!("contrast {} colour {:?}: {:.2} >= 3", theme, c, r), r >= 3.0);
            t.eq(&format!("{} {:?} is what link_color takes", theme, c), paint::link_color(CableKind::Wan, c, dark, false, bg), argb);
        }
    }
    t.eq("no colour: the kind's own", paint::link_color(CableKind::Phone, CableColor::Plain, true, false, 0xFF00_0000), paint::cable_color(CableKind::Phone, true, false, 0xFF00_0000));
    t.check("an unplugged coloured cable fades", paint::link_color(CableKind::Wan, CableColor::Blue, false, true, 0xFFFF_FFFF) != paint::link_color(CableKind::Wan, CableColor::Blue, false, false, 0xFFFF_FFFF));
    let style = |k: CableKind| paint::cable_style(k);
    t.check("multimode and single-mode differ in their pattern", style(CableKind::FiberMm).2 != style(CableKind::FiberSm).2);
    // every kind has a look of its own beside its colour: width and pattern (the serial lines, WAN,
    // coax and DAC have marks of their own; straight copper and multimode differ in colour, as ever)
    let marked = [CableKind::Serial, CableKind::SerialDce, CableKind::SerialDte, CableKind::Wan, CableKind::Coax, CableKind::Dac, CableKind::FiberMm];
    let plain: Vec<(u32, u32)> = CableKind::ALL.iter().filter(|k| !marked.contains(k)).map(|&k| ((style(k).1 * 10.0) as u32, style(k).2)).collect();
    t.check("no two kinds with the same width and pattern", plain.iter().enumerate().all(|(i, x)| !plain[i + 1..].contains(x)));
}

pub(crate) fn run(t: &mut T) {
    fits(t);
    models(t);
    limits(t);
    old_files(t);
    round_trip(t);
    engine_part(t);
    choose(t);
    drawing(t);
}
