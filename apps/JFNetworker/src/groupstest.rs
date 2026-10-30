// groupstest -- checks of the groups and the background picture of the canvas (group groups of
// --selftest, G1): jfn_model::group (the outermost of what is given into the innermost group
// that holds it all, nesting, a device in one group per level, groups left empty go, ids that
// are gone and come back with undo, cycles, a member listed twice, names, ungroup, what a
// collapsed group hides), the file (groups and the picture's bytes intact, an older file
// without either, members that are gone dropped on reading), the picture (its formats, the
// limits, decoding the fixture, its place in a view), the steps of doc/groups.rs with undo and
// redo (the bytes shared), the canvas' scene (frames around their members with room for their
// names, an expanded group changes no cable, clusters at the middle of their members, one line
// per pair of ends and kind with the worst of their LEDs, a single cable keeps its key, cables
// inside a cluster hidden, nested groups, labels, what a drag moves, reveal), the colours of
// the roles (3:1 against the canvas) and that the simulator runs the same with and without
// groups and a picture (the golden scenario lan_router).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use jfn_model::background::{self, Background, BgErr};
use jfn_model::group::{self, Group, GroupColor, GroupErr, Groups};
use jfn_model::project::{PortRef, Project};
use jfn_sim::Led;
use ui::{contrast_ratio, Style};

use crate::doc::groups as dgroups;
use crate::doc::Doc;
use crate::selftest::T;
use crate::workspace::canvas::groups::{agg_key, role_stroke, FRAME_BAR, FRAME_PAD};
use crate::workspace::canvas::paint::{Scene, DEV_H, DEV_W};
use crate::workspace::icons::Symbols;

const PLAN: &[u8] = include_bytes!("../tests/fixtures/bg_plan.png");
const LAN: &str = include_str!("../tests/fixtures/lan_router.jfnet");
const LAN_SCN: &str = include_str!("../tests/scen/lan_router.scn");

fn s(x: &str) -> String {
    String::from(x)
}

/// the devices and cables of the test project
struct Ids {
    r1: u32,
    sw: u32,
    pc1: u32,
    pc2: u32,
    s1: u32,
    s2: u32,
    l_r: u32,
    l_p1: u32,
    l_s1: u32,
    l_s2: u32,
    l_ss: u32,
}

// R1 over SW1 with PC1, PC2 and the servers SRV1, SRV2 (a cable between them)
fn project() -> (Project, Ids) {
    let mut p = Project::new("G1", 1);
    let mut dev = |m: &str, x: i32, y: i32| p.add_device(m, x, y).unwrap_or(0);
    let r1 = dev("jf-r4", 0, 0);
    let sw = dev("jf-s24p", 0, 160);
    let pc1 = dev("pc", -160, 320);
    let pc2 = dev("pc", -40, 320);
    let s1 = dev("server-1u", 160, 320);
    let s2 = dev("server-1u", 280, 320);
    let mut link = |a: u32, pa: &str, b: u32, pb: &str| p.connect(&PortRef { dev: a, port: s(pa) }, &PortRef { dev: b, port: s(pb) }, None).unwrap_or(0);
    let l_r = link(r1, "Gi0/0/0", sw, "Gi1/0/24");
    let l_p1 = link(pc1, "eth0", sw, "Gi1/0/1");
    let _l_p2 = link(pc2, "eth0", sw, "Gi1/0/2");
    let l_s1 = link(s1, "eth0", sw, "Gi1/0/3");
    let l_s2 = link(s2, "eth0", sw, "Gi1/0/4");
    let l_ss = link(s1, "eth1", s2, "eth1");
    (p, Ids { r1, sw, pc1, pc2, s1, s2, l_r, l_p1, l_s1, l_s2, l_ss })
}

// every project the app brings and the tests use: none has groups or a picture
const FIXTURES: [(&str, &str); 38] = [
    ("backup", include_str!("../tests/fixtures/backup.jfnet")),
    ("bgp_isp", include_str!("../tests/fixtures/bgp_isp.jfnet")),
    ("bgp_pair", include_str!("../tests/fixtures/bgp_pair.jfnet")),
    ("cables", include_str!("../tests/fixtures/cables.jfnet")),
    ("campus", include_str!("../tests/fixtures/campus.jfnet")),
    ("groups", include_str!("../tests/fixtures/groups.jfnet")),
    ("http_load", include_str!("../tests/fixtures/http_load.jfnet")),
    ("ipv6_routing", include_str!("../tests/fixtures/ipv6_routing.jfnet")),
    ("ipv6_slaac", include_str!("../tests/fixtures/ipv6_slaac.jfnet")),
    ("ipv6_web", include_str!("../tests/fixtures/ipv6_web.jfnet")),
    ("l3_switch", include_str!("../tests/fixtures/l3_switch.jfnet")),
    ("lab_shards", include_str!("../tests/fixtures/lab_shards.jfnet")),
    ("lab_window", include_str!("../tests/fixtures/lab_window.jfnet")),
    ("lan_router", include_str!("../tests/fixtures/lan_router.jfnet")),
    ("nat_pat", include_str!("../tests/fixtures/nat_pat.jfnet")),
    ("office", include_str!("../tests/fixtures/office.jfnet")),
    ("ospf_areas", include_str!("../tests/fixtures/ospf_areas.jfnet")),
    ("ospf_triangle", include_str!("../tests/fixtures/ospf_triangle.jfnet")),
    ("pdu_lab", include_str!("../tests/fixtures/pdu_lab.jfnet")),
    ("phys_campus", include_str!("../tests/fixtures/phys_campus.jfnet")),
    ("rip_ring", include_str!("../tests/fixtures/rip_ring.jfnet")),
    ("rip_versions", include_str!("../tests/fixtures/rip_versions.jfnet")),
    ("router_on_a_stick", include_str!("../tests/fixtures/router_on_a_stick.jfnet")),
    ("rstp_triangle", include_str!("../tests/fixtures/rstp_triangle.jfnet")),
    ("script_kv", include_str!("../tests/fixtures/script_kv.jfnet")),
    ("scripts", include_str!("../tests/fixtures/scripts.jfnet")),
    ("serial_wan", include_str!("../tests/fixtures/serial_wan.jfnet")),
    ("stp_triangle", include_str!("../tests/fixtures/stp_triangle.jfnet")),
    ("tcp_wan", include_str!("../tests/fixtures/tcp_wan.jfnet")),
    ("two_pcs", include_str!("../tests/fixtures/two_pcs.jfnet")),
    ("wlan_office", include_str!("../tests/fixtures/wlan_office.jfnet")),
    ("labs/01-campus-vlan", include_str!("../assets/labs/01-campus-vlan.jfnet")),
    ("labs/02-lastverteilung", include_str!("../assets/labs/02-lastverteilung.jfnet")),
    ("labs/03-shards", include_str!("../assets/labs/03-shards.jfnet")),
    ("labs/04-cache", include_str!("../assets/labs/04-cache.jfnet")),
    ("assess/01-router-zwei-netze", include_str!("../assets/assess/01-router-zwei-netze.jfnet")),
    ("assess/02-vlans-trunk", include_str!("../assets/assess/02-vlans-trunk.jfnet")),
    ("assess/03-statische-routen", include_str!("../assets/assess/03-statische-routen.jfnet")),
];

// a device's box
fn box_of(p: &Project, d: u32) -> [f32; 4] {
    p.device(d).map(|x| [x.x as f32, x.y as f32, DEV_W, DEV_H]).unwrap_or_default()
}

// r lies inside o (both x, y, w, h)
fn inside(r: [f32; 4], o: [f32; 4]) -> bool {
    o[0] <= r[0] && o[1] <= r[1] && r[0] + r[2] <= o[0] + o[2] && r[1] + r[3] <= o[1] + o[3]
}

fn rec(p: &Project, id: u32) -> Option<Group> {
    p.groups.iter().find(|g| g.id == id).cloned()
}

pub(crate) fn run(t: &mut T) {
    model(t);
    resolve(t);
    file(t);
    picture(t);
    steps(t);
    scene(t);
    colours(t);
    engine(t);
}

// ---------------------------------------------------------------- the model

fn model(t: &mut T) {
    let (mut p, i) = project();
    let links = p.links.len();
    t.check("a project of six devices and six cables", p.devices.len() == 6 && links == 6 && i.l_ss != 0 && i.l_r != 0);
    let next = p.next_id;
    let g1 = group::group(&mut p, &[i.s1, i.s2], "", "Gruppe").unwrap_or(0);
    let r = rec(&p, g1);
    t.eq("a new group: its name, colour, members", r.as_ref().map(|g| (g.name.clone(), g.color, g.members.clone(), g.collapsed)), Some((s("Gruppe 1"), GroupColor::Blue, vec![i.s1, i.s2], false)));
    t.eq("its id from the project's counter", (g1, p.next_id, p.max_id()), (next, next + 1, g1));
    let gs = Groups::of(&p);
    t.check("resolved", gs.has(g1) && gs.parent(i.s1) == Some(g1) && gs.parent(i.sw).is_none() && gs.members(g1) == [i.s1, i.s2] && gs.depth(g1) == 0);
    // nesting: the switch and the servers' group into a group named by the user
    let g2 = group::group(&mut p, &[i.sw, g1], "Kern", "Gruppe").unwrap_or(0);
    let gs = Groups::of(&p);
    t.eq("nested: members", gs.members(g2).to_vec(), vec![i.sw, g1]);
    t.eq("nested: the chain of a server", gs.chain(i.s1), vec![g1, g2]);
    t.eq("nested: depths, order, leaves", (gs.depth(g1), gs.depth(g2), gs.order().to_vec(), gs.leaves(g2)), (1, 0, vec![g2, g1], vec![i.sw, i.s1, i.s2]));
    t.eq("nested: devices and groups below", (gs.devices(&p, g2), gs.subgroups(g2)), (vec![i.sw, i.s1, i.s2], vec![g1]));
    t.eq("the second group takes the next colour", rec(&p, g2).map(|g| g.color), Some(GroupColor::Green));
    // a member of a given group stays in it: only the outermost go into the new group
    let g3 = group::group(&mut p, &[g2, i.s1], "", "Gruppe").unwrap_or(0);
    t.eq("the outermost only", rec(&p, g3).map(|g| g.members), Some(vec![g2]));
    t.eq("unique name", rec(&p, g3).map(|g| g.name), Some(s("Gruppe 2")));
    t.check("ungroup: the members take its place", group::ungroup(&mut p, g3).is_ok() && rec(&p, g3).is_none() && Groups::of(&p).parent(g2).is_none());
    // things from different levels: the new group at their common level, a device in one group
    let g4 = group::group(&mut p, &[i.pc1, i.s1], "", "Gruppe").unwrap_or(0);
    let gs = Groups::of(&p);
    t.check("different levels: at the top", gs.parent(g4).is_none() && gs.members(g4) == [i.pc1, i.s1]);
    t.check("a device in one group per level", gs.parent(i.s1) == Some(g4) && gs.members(g1) == [i.s2]);
    t.check("ungroup at the top", group::ungroup(&mut p, g4).is_ok() && Groups::of(&p).parent(i.s1).is_none() && Groups::of(&p).parent(i.pc1).is_none());
    // a group inside a group: where the first of them was
    let g5 = group::group(&mut p, &[i.s2], "Rack", "Gruppe").unwrap_or(0);
    let gs = Groups::of(&p);
    t.check("inside the innermost group that holds them", gs.parent(g5) == Some(g1) && gs.members(g1) == [g5] && gs.chain(i.s2) == [g5, g1, g2]);
    // a group left empty goes
    let a = group::group(&mut p, &[i.pc1, i.pc2], "A", "Gruppe").unwrap_or(0);
    let b = group::group(&mut p, &[i.pc1, i.pc2], "B", "Gruppe").unwrap_or(0);
    t.check("grouping all of a group's members nests", Groups::of(&p).parent(b) == Some(a) && rec(&p, a).map(|g| g.members) == Some(vec![b]));
    let c = group::group(&mut p, &[b, i.r1], "C", "Gruppe").unwrap_or(0);
    t.check("the group left empty goes", rec(&p, a).is_none() && rec(&p, c).map(|g| g.members) == Some(vec![b, i.r1]) && Groups::of(&p).parent(b) == Some(c));
    // refusals
    t.eq("nothing given", group::group(&mut p, &[], "", "Gruppe"), Err(GroupErr::Nothing));
    t.eq("unknown ids", group::group(&mut p, &[9999, i.l_r], "", "Gruppe"), Err(GroupErr::Nothing));
    t.eq("an empty name", group::rename(&mut p, g2, "  "), Err(GroupErr::BadName(String::new())));
    let long: String = core::iter::repeat_n('x', 65).collect();
    t.check("a name too long", matches!(group::rename(&mut p, g2, &long), Err(GroupErr::BadName(_))));
    t.eq("a name taken (letter case aside)", group::rename(&mut p, g5, "kern"), Err(GroupErr::NameTaken(s("kern"))));
    t.check("its own name in another case", group::rename(&mut p, g2, "KERN").is_ok() && rec(&p, g2).map(|g| g.name) == Some(s("KERN")));
    t.eq("a name given twice when grouping", group::group(&mut p, &[i.pc1], "Rack", "Gruppe"), Err(GroupErr::NameTaken(s("Rack"))));
    t.eq("ungroup of no group", group::ungroup(&mut p, 12345), Err(GroupErr::UnknownGroup(12345)));
    t.check("colour", group::set_color(&mut p, g5, GroupColor::Ochre).is_ok() && rec(&p, g5).map(|g| g.color) == Some(GroupColor::Ochre));
    t.eq("the refusals as texts", (GroupErr::Nothing.msg().text(), GroupErr::NameTaken(s("Kern")).msg().text()), (s("Nichts zum Gruppieren: Geräte, Notizen oder Gruppen auswählen."), s("Eine Gruppe \u{201e}Kern\u{201c} gibt es schon.")));
    // collapsed: what shows instead of a member
    let _ = group::set_collapsed(&mut p, g1, true);
    let gs = Groups::of(&p);
    t.eq("collapsed: a server shows as its cluster", (gs.hidden_by(&p, i.s2), gs.rep(&p, i.s2), gs.rep(&p, g5), gs.hidden_by(&p, g1)), (Some(g1), g1, g1, None));
    let _ = group::set_collapsed(&mut p, g2, true);
    let gs = Groups::of(&p);
    t.eq("an outer one collapsed hides the inner ones", (gs.rep(&p, i.s2), gs.rep(&p, g1), gs.rep(&p, i.sw), gs.rep(&p, i.pc2)), (g2, g2, g2, i.pc2));
    t.check("no issues", group::issues(&p).is_empty() && p.check().iter().all(|x| !x.text.contains("Gruppe")));
}

// ids that are gone, members listed twice, cycles
fn resolve(t: &mut T) {
    let (mut p, i) = project();
    let gid = |p: &mut Project| {
        let id = p.next_id;
        p.next_id += 1;
        id
    };
    let a = gid(&mut p);
    let gone = gid(&mut p);
    p.groups = vec![
        Group { id: a, name: s("A"), members: vec![i.s1, 99_999], ..Group::default() },
        Group { id: gone, name: s("Leer"), members: vec![99_998], ..Group::default() },
    ];
    let gs = Groups::of(&p);
    t.check("an id that is gone is skipped", gs.members(a) == [i.s1] && !gs.has(gone) && gs.order() == [a]);
    let mut q = p.clone();
    group::normalize(&mut q);
    t.eq("normalize drops them", q.groups.iter().map(|g| (g.id, g.members.clone())).collect::<Vec<_>>(), vec![(a, vec![i.s1])]);
    // a deleted member that undo brings back is in its group again
    let (d, l) = p.remove_device(i.s1).unwrap_or_default();
    t.check("a deleted member: the group does not count", !Groups::of(&p).has(a));
    let _ = p.insert_device(d);
    for x in l {
        let _ = p.insert_link(x);
    }
    t.check("back again: in its group", Groups::of(&p).parent(i.s1) == Some(a));
    // a member listed twice: the first group gets it
    let b = gid(&mut p);
    p.groups = vec![Group { id: a, name: s("A"), members: vec![i.s1], ..Group::default() }, Group { id: b, name: s("B"), members: vec![i.s1, i.s2], ..Group::default() }];
    let gs = Groups::of(&p);
    t.check("twice: the first group", gs.parent(i.s1) == Some(a) && gs.members(b) == [i.s2]);
    t.check("twice: an issue", group::issues(&p).iter().any(|(e, m)| !e && m.text().contains("SRV1") && m.text().contains("mehr als einer Gruppe")));
    // a cycle
    p.groups = vec![Group { id: a, name: s("A"), members: vec![b, i.pc1], ..Group::default() }, Group { id: b, name: s("B"), members: vec![a, i.pc2], ..Group::default() }];
    let gs = Groups::of(&p);
    t.check("a cycle is broken", gs.order().len() == 2 && gs.chain(a).len() + gs.chain(b).len() == 1 && gs.leaves(a).len() + gs.leaves(b).len() >= 3);
    t.check("a cycle: an issue", group::issues(&p).iter().any(|(_, m)| m.text().contains("liegt in sich selbst")));
    // itself as a member, an id of a device
    p.groups = vec![Group { id: a, name: s("A"), members: vec![a, i.pc1], ..Group::default() }, Group { id: i.sw, name: s("SW"), members: vec![i.pc2], ..Group::default() }];
    let gs = Groups::of(&p);
    t.check("itself as a member is skipped", gs.members(a) == [i.pc1]);
    t.check("a device's id is no group", !gs.has(i.sw) && group::issues(&p).iter().any(|(e, m)| *e && m.text().contains("schon vergeben")) && p.check().iter().any(|x| x.error && x.text.contains("Gruppe \u{201e}SW\u{201c}")));
}

// ---------------------------------------------------------------- the file

fn file(t: &mut T) {
    let (mut p, i) = project();
    let g = group::group(&mut p, &[i.s1, i.s2], "Server", "Gruppe").unwrap_or(0);
    let _ = group::set_collapsed(&mut p, g, true);
    let mut bg = Background::new("bg_plan.png", PLAN.to_vec(), 160, 120).unwrap_or_default();
    bg.x = -80;
    bg.y = 40;
    bg.scale_pm = 2500;
    bg.locked = true;
    p.background = Some(bg);
    let text = p.to_text();
    t.check("in the file: the group", text.contains("\"groups\": [") && text.contains("\"name\": \"Server\"") && text.contains("\"collapsed\": true") && text.contains("\"color\": \"blue\""));
    t.check("in the file: the picture", text.contains("\"background\": {") && text.contains("\"format\": \"png\"") && text.contains("\"base64\": \"iVBORw0KGgo") && text.contains("\"scale_pm\": 2500"));
    match Project::from_text(&text) {
        Ok(q) => {
            t.eq("groups read back", q.groups.clone(), p.groups.clone());
            t.check("the picture's bytes intact", q.background.as_ref().is_some_and(|b| *b.data == *PLAN && b.w == 160 && b.h == 120 && b.x == -80 && b.y == 40 && b.scale_pm == 2500 && b.locked && b.name == "bg_plan.png"));
            t.eq("the whole project the same", q, p.clone());
        }
        Err(e) => t.check(&format!("read back: {}", e.text), false),
    }
    // without either
    let (plain, _) = project();
    let text = plain.to_text();
    t.check("none: [] and null", text.contains("\"groups\": []") && text.contains("\"background\": null"));
    let old = "{\"format\": \"jfn\", \"version\": 1, \"devices\": [{\"id\": 1, \"name\": \"PC1\", \"model\": \"pc\"}]}";
    t.check("an older file has neither", Project::from_text(old).is_ok_and(|q| q.groups.is_empty() && q.background.is_none() && Groups::of(&q).is_empty()));
    // reading drops what does not resolve; the counter stays above every group
    let stale = "{\"format\": \"jfn\", \"version\": 1, \"next_id\": 2, \"devices\": [{\"id\": 1, \"name\": \"PC1\", \"model\": \"pc\"}], \"groups\": [{\"id\": 7, \"name\": \"G\", \"members\": [1, 5]}, {\"id\": 8, \"name\": \"Leer\", \"members\": [6]}]}";
    t.check("reading drops members that are gone", Project::from_text(stale).is_ok_and(|q| q.groups.len() == 1 && q.groups[0].members == [1] && q.groups[0].color == GroupColor::Grey && q.next_id == 9));
    // the fixtures, labs and assessments have neither
    for (name, text) in FIXTURES {
        let q = Project::from_text(text);
        t.check(&format!("{}: no groups, no picture", name), q.is_ok_and(|q| q.groups.is_empty() && q.background.is_none()));
    }
}

// ---------------------------------------------------------------- the picture

fn picture(t: &mut T) {
    img_all::use_core_codecs();
    t.eq("sniff", (background::sniff(PLAN), background::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), background::sniff(b"GIF89a.."), background::sniff(b"hello")), (Some("png"), Some("jpeg"), Some("gif"), None));
    let mut bmp = b"BM".to_vec();
    bmp.resize(54, 0);
    t.eq("sniff bmp", background::sniff(&bmp), Some("bmp"));
    let b = Background::new("plan.png", PLAN.to_vec(), 160, 120);
    t.check("a new picture", b.as_ref().is_ok_and(|b| b.format == "png" && b.opacity_pct() == background::OPACITY_NEW && b.scale() == 1.0 && b.rect() == [0.0, 0.0, 160.0, 120.0] && !b.locked));
    t.eq("too many bytes", Background::new("x.png", vec![0x89; background::MAX_BYTES + 1], 10, 10).map(|_| ()), Err(BgErr::TooLarge(background::MAX_BYTES + 1)));
    t.eq("too wide", Background::new("x.png", PLAN.to_vec(), 9000, 10).map(|_| ()), Err(BgErr::TooWide(9000, 10)));
    t.eq("too many pixels", Background::new("x.png", PLAN.to_vec(), 8192, 8192).map(|_| ()), Err(BgErr::TooWide(8192, 8192)));
    t.eq("no picture", Background::new("x.txt", b"hallo".to_vec(), 10, 10).map(|_| ()), Err(BgErr::Format));
    t.eq("the limit as a text", BgErr::TooLarge(9 << 20).msg().text(), s("Das Bild ist zu groß: 9 MB; ins Projekt passen höchstens 8 MB."));
    // the fixture decoded: its size, the desk's grey (premultiplied B, G, R, A)
    match crate::workspace::canvas::backdrop::decode(PLAN) {
        Some(sf) => {
            let k = (25 * sf.stride + 30 * 4) as usize;
            t.eq("decoded: size", (sf.w, sf.h), (160, 120));
            t.eq("decoded: a pixel", sf.px.get(k..k + 4).map(|x| x.to_vec()), Some(vec![184, 176, 170, 255]));
        }
        None => t.check("the fixture decodes", false),
    }
    t.check("garbage does not decode", crate::workspace::canvas::backdrop::decode(b"\x89PNG\r\n\x1a\nkaputt").is_none());
    // its place in a view: centred, nine tenths of it
    let mut b = b.unwrap_or_default();
    crate::workspace::canvas::backdrop::fit_into(&mut b, [0.0, 0.0, 1000.0, 600.0]);
    t.eq("fitted into a view", (b.scale_pm, b.x, b.y), (4500, 140, 30));
    let mut c = b.clone();
    c.set_place([1, 2, 3000, 40, 1]);
    t.eq("its place without the bytes", (c.x, c.y, c.scale_pm, c.opacity, c.locked, c.place()), (1, 2, 3000, 40, true, [1, 2, 3000, 40, 1]));
    t.check("the bytes shared, not copied", Arc::ptr_eq(&c.data, &b.data));
}

// ---------------------------------------------------------------- undo steps

fn steps(t: &mut T) {
    let (p, i) = project();
    let mut doc = Doc::new();
    doc.replace(p, None);
    let steps0 = doc.hist.len();
    let r = dgroups::step(&mut doc, "Gruppieren", |p| group::group(p, &[i.s1, i.s2], "", "Gruppe"));
    let g = r.clone().ok().flatten().unwrap_or(0);
    t.check("a step: grouped, unsaved", g != 0 && doc.proj.groups.len() == 1 && doc.dirty() && doc.hist.len() == steps0 + 1);
    t.check("undo", doc.undo() && doc.proj.groups.is_empty());
    t.check("redo: the same group", doc.redo() && rec(&doc.proj, g).is_some_and(|x| x.members == [i.s1, i.s2]));
    t.check("nothing changed: no step", dgroups::step(&mut doc, "Gruppe umbenennen", |p| group::rename(p, g, "Gruppe 1")) == Ok(None) && doc.hist.len() == steps0 + 1);
    t.check("refused: no step", dgroups::step(&mut doc, "Gruppe umbenennen", |p| group::rename(p, g, "")).is_err() && doc.hist.len() == steps0 + 1 && rec(&doc.proj, g).is_some_and(|x| x.name == "Gruppe 1"));
    let _ = dgroups::step(&mut doc, "Gruppe zuklappen", |p| group::set_collapsed(p, g, true));
    t.check("collapsed as a step", rec(&doc.proj, g).is_some_and(|x| x.collapsed) && doc.undo() && rec(&doc.proj, g).is_some_and(|x| !x.collapsed) && doc.redo());
    let _ = dgroups::step(&mut doc, "Gruppierung aufheben", |p| group::ungroup(p, g));
    t.check("ungrouped as a step", doc.proj.groups.is_empty() && doc.undo() && rec(&doc.proj, g).is_some_and(|x| x.collapsed));
    // the picture: in, its place, out; undo brings the same bytes back
    let bg = Background::new("bg_plan.png", PLAN.to_vec(), 160, 120).unwrap_or_default();
    let data = bg.data.clone();
    t.check("the picture in", dgroups::set_bg(&mut doc, "Hintergrundbild einfügen", Some(bg)) && doc.proj.background.is_some());
    t.check("its place as a step", dgroups::bg_place(&mut doc, "Hintergrundbild einstellen", |b| b.x += 100) && doc.proj.background.as_ref().is_some_and(|b| b.x == 100));
    t.check("the same place: no step", !dgroups::bg_place(&mut doc, "Hintergrundbild einstellen", |b| b.x = 100));
    let start = doc.proj.background.as_ref().map(|b| b.place()).unwrap_or_default();
    dgroups::bg_live(&mut doc, |b| b.y = 70);
    dgroups::bg_live(&mut doc, |b| b.y = 80);
    t.check("a drag: one step at its end", dgroups::bg_since(&mut doc, start, "Hintergrundbild verschieben") && doc.undo() && doc.proj.background.as_ref().is_some_and(|b| b.y == 0 && b.x == 100));
    t.check("out", dgroups::set_bg(&mut doc, "Hintergrund entfernen", None) && doc.proj.background.is_none());
    t.check("undo: the same bytes", doc.undo() && doc.proj.background.as_ref().is_some_and(|b| Arc::ptr_eq(&b.data, &data)));
    t.check("the simulator knows nothing of it", doc.sim.errors.is_empty() && doc.proj.devices.len() == 6);
}

// ---------------------------------------------------------------- the canvas' scene

fn scene(t: &mut T) {
    let syms = Symbols::new();
    let (mut p, i) = project();
    let mut sc0 = Scene::default();
    sc0.build(&p, &syms, 1);
    t.check("no groups: the scene as before", sc0.frames.is_empty() && sc0.clusters.is_empty() && sc0.edges.len() == p.links.len() && sc0.edges.iter().zip(p.links.iter()).all(|(e, l)| e.key == l.id as u64) && sc0.via.is_empty() && sc0.inside.is_empty());
    t.eq("no groups: the nodes", (sc0.f0, sc0.n0, sc0.d0, sc0.c0, sc0.nodes.len()), (0, 0, 0, 6, 6));
    // an expanded group: a frame around its members, nothing else changes
    let g = group::group(&mut p, &[i.s1, i.s2], "", "Gruppe").unwrap_or(0);
    let mut sc1 = Scene::default();
    sc1.build(&p, &syms, 2);
    let fr = sc1.frames.first().map(|f| f.rect).unwrap_or_default();
    t.eq("a frame (the first node)", (sc1.frames.len(), sc1.f0, sc1.n0, sc1.d0, sc1.c0, sc1.nodes.first().map(|n| (n.key, n.flags))), (1, 0, 1, 1, 7, Some((g as u64, ui::DN_FRAME | ui::DN_TOGGLE | ui::DN_STEP))));
    t.check("the frame holds its members with their names", inside(box_of(&p, i.s1), fr) && inside(box_of(&p, i.s2), fr) && fr[1] <= 320.0 - FRAME_BAR - FRAME_PAD && fr[1] + fr[3] >= 368.0 + 22.0);
    t.check("the frame holds nothing else", !inside(box_of(&p, i.pc2), fr) && !inside(box_of(&p, i.sw), fr));
    let same = sc0.nodes.iter().all(|n| sc1.rect(n.key) == Some(n.rect)) && sc0.edges.iter().zip(sc1.edges.iter()).all(|(a, b)| (a.key, a.a, a.b, a.route) == (b.key, b.a, b.b, b.route));
    t.check("expanded: every device and cable as before", same && sc1.edges.len() == sc0.edges.len());
    t.eq("expanded: its label", sc1.label(g as u64), s("Gruppe \u{201e}Gruppe 1\u{201c}, 2 Geräte"));
    // collapsed: a cluster at the middle of its members, one line per far end and kind
    let _ = group::set_collapsed(&mut p, g, true);
    let mut sc2 = Scene::default();
    sc2.build(&p, &syms, 3);
    let c = sc2.clusters.first().map(|c| c.rect).unwrap_or_default();
    t.eq("collapsed: the cluster at the middle", (sc2.clusters.len(), sc2.frames.len(), c), (1, 0, [220.0, 320.0, DEV_W, DEV_H]));
    t.check("collapsed: its members hidden", sc2.rect(i.s1 as u64).is_none() && sc2.rect(i.s2 as u64).is_none() && sc2.vis_rect(i.s1) == Some(c) && sc2.hidden.get(&i.s2) == Some(&g));
    t.eq("collapsed: the nodes (the cluster last)", (sc2.d0, sc2.c0, sc2.nodes.len(), sc2.nodes.last().map(|n| (n.key, n.flags))), (0, 4, 5, Some((g as u64, ui::DN_STEP))));
    let agg = agg_key(i.l_s1.min(i.l_s2));
    t.eq("collapsed: one line for both cables to SW1", (sc2.edges.len(), sc2.line_links(agg)), (4, vec![i.l_s1, i.l_s2]));
    t.eq("collapsed: the line's ends", sc2.line_ends(agg), Some((g, i.sw)));
    t.eq("collapsed: the cables on it", (sc2.line_of(i.l_s1), sc2.line_of(i.l_s2), sc2.line_of(i.l_p1)), (Some((agg, false)), Some((agg, false)), Some((i.l_p1 as u64, false))));
    t.eq("collapsed: the cable inside is hidden", (sc2.line_of(i.l_ss), sc2.inside.contains(&i.l_ss)), (None, true));
    let mut leds = alloc::collections::BTreeMap::new();
    leds.insert(i.l_s1, [Led::Up, Led::Up]);
    leds.insert(i.l_s2, [Led::Down, Led::Up]);
    t.eq("collapsed: the worst LEDs of the line", sc2.line_leds(agg, &leds), Some([Led::Down, Led::Up]));
    t.eq("collapsed: its label", sc2.label(g as u64), s("Gruppe \u{201e}Gruppe 1\u{201c}, 2 Geräte, zugeklappt"));
    t.check("collapsed: the line names no port, has no waypoints", sc2.line(agg).is_some_and(|l| l.names[0].is_empty() && l.names[1].is_empty() && l.agg.len() == 2) && sc2.edges.iter().find(|e| e.key == agg).is_some_and(|e| e.wps.is_empty() && e.route == ui::DR_STRAIGHT));
    // a single cable into a cluster keeps its key, its port name at the device
    let gp = group::group(&mut p, &[i.pc1], "", "Gruppe").unwrap_or(0);
    let _ = group::set_collapsed(&mut p, gp, true);
    let mut sc3 = Scene::default();
    sc3.build(&p, &syms, 4);
    t.check("one cable: its own key", sc3.line_of(i.l_p1) == Some((i.l_p1 as u64, false)) && sc3.line(i.l_p1 as u64).is_some_and(|l| l.names[0].is_empty() && l.names[1] == "Gi1/0/1" && l.agg.len() == 1));
    // nested: the switch with the collapsed servers in an expanded group
    let g2 = group::group(&mut p, &[i.sw, g], "Kern", "Gruppe").unwrap_or(0);
    let mut sc4 = Scene::default();
    sc4.build(&p, &syms, 5);
    let f2 = sc4.frames.iter().find(|f| f.id == g2).map(|f| f.rect).unwrap_or_default();
    t.check("nested: the frame holds the switch and the cluster", inside(box_of(&p, i.sw), f2) && inside(c, f2));
    let _ = group::set_collapsed(&mut p, g2, true);
    let mut sc5 = Scene::default();
    sc5.build(&p, &syms, 6);
    t.check("nested: the outer cluster hides the inner one", sc5.clusters.len() == 2 && sc5.rect(g as u64).is_none() && sc5.hidden.get(&g) == Some(&g2) && sc5.hidden.get(&i.s1) == Some(&g2));
    // the line from R1 to the cluster that hides the switch
    t.check("nested: R1's cable to the outer cluster", sc5.line_of(i.l_r) == Some((i.l_r as u64, false)) && sc5.line_ends(i.l_r as u64) == Some((i.r1, g2)));
    // what a drag moves, reveal (the window's state)
    let mut n = crate::app::Net::new();
    n.doc.replace(p.clone(), None);
    let rev = n.doc.rev;
    n.canvas.scene.build(&n.doc.proj, &n.syms, rev);
    let mut moved = crate::workspace::canvas::groups::move_set(&n, &[g2 as u64]);
    moved.sort_unstable();
    t.eq("a drag of a cluster moves everything below it", moved, vec![i.sw, i.s1, i.s2]);
    let mut moved = crate::workspace::canvas::groups::move_set(&n, &[g2 as u64, i.s1 as u64, i.pc2 as u64]);
    moved.sort_unstable();
    t.eq("... once each, with what else is selected", moved, vec![i.sw, i.pc2, i.s1, i.s2]);
    t.check("reveal: a hidden member's groups open (one step)", crate::workspace::canvas::groups::reveal(&mut n, i.s1) && rec(&n.doc.proj, g).is_some_and(|x| !x.collapsed) && rec(&n.doc.proj, g2).is_some_and(|x| !x.collapsed) && rec(&n.doc.proj, gp).is_some_and(|x| x.collapsed));
    t.check("reveal: one undo step", n.doc.undo() && rec(&n.doc.proj, g).is_some_and(|x| x.collapsed) && rec(&n.doc.proj, g2).is_some_and(|x| x.collapsed));
    t.check("reveal of a device in view: nothing", !crate::workspace::canvas::groups::reveal(&mut n, i.pc2));
    // the selection: a line stands for its cables, the picture for nothing
    let _ = group::set_collapsed(&mut n.doc.proj, g2, false);
    n.canvas.scene.build(&n.doc.proj, &n.syms, n.doc.rev.wrapping_add(1));
    n.canvas.st.sel = [agg, crate::workspace::canvas::paint::BG_KEY, g as u64].into_iter().collect();
    let mut sel = n.canvas.selected();
    sel.sort_unstable();
    t.eq("the selection's ids", sel, vec![i.l_s1, i.l_s2, g]);
}

// ---------------------------------------------------------------- colours

fn colours(t: &mut T) {
    let bg = |dark: bool| style::STYLE.tokens(dark)[(ui::consts::T_BG / 4) as usize] | 0xFF00_0000;
    let themes = [(false, bg(false), "hell"), (true, bg(true), "dunkel"), (true, 0xFF00_0000 | plat::HcColors::BLACK.bg, "kontrast schwarz"), (false, 0xFF00_0000 | plat::HcColors::WHITE.bg, "kontrast weiß")];
    for (dark, bg, theme) in themes {
        for &c in GroupColor::ALL {
            let r = contrast_ratio(role_stroke(c, dark), bg);
            t.check(&format!("contrast {} {:?}: {:.2} >= 3", theme, c, r), r >= 3.0);
        }
    }
    t.check("the roles have names", GroupColor::ALL.iter().all(|c| !c.title().is_empty()) && GroupColor::nth(0) == GroupColor::Blue && GroupColor::nth(5) == GroupColor::Grey);
}

// ---------------------------------------------------------------- the simulator

fn engine(t: &mut T) {
    let Ok(sc) = jfn_model::scenario::parse(LAN_SCN) else {
        t.check("lan_router.scn parses", false);
        return;
    };
    let Ok(p) = Project::from_text(LAN) else {
        t.check("lan_router.jfnet reads", false);
        return;
    };
    let mut q = p.clone();
    let ids: Vec<u32> = q.devices.iter().skip(1).map(|d| d.id).collect();
    let g = group::group(&mut q, &ids, "", "Gruppe").unwrap_or(0);
    let _ = group::set_collapsed(&mut q, g, true);
    q.background = Background::new("bg_plan.png", PLAN.to_vec(), 160, 120).ok();
    let a = jfn_sim::run_scenario(&p, &sc, "lan_router", false);
    let b = jfn_sim::run_scenario(&q, &sc, "lan_router", false);
    t.check(&format!("the scenario holds ({} of {})", a.expects - a.failed, a.expects), a.ok && a.expects > 0);
    t.check("groups and a picture change no report", g != 0 && b.ok && a.report == b.report);
}
