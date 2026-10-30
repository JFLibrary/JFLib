// props -- the physical view's parts of the inspector (P1): a place, a device's place and the
// optics in its cages, a cable's length and limit
//
//   place(n, ui, id)     a place: name, kind, where it lies, position and size in metres, a
//                        rack's height in rack units, a floor's storey, its devices; Öffnen,
//                        Entfernen
//   device(n, ui, id)    the "Ort" of a device (path, slot), a list to move it to another room,
//                        rack or desk, its rack unit, "Im Plan zeigen"; the optic of each optics
//                        cage (SR or LR)
//   cable(n, ui, id)     length (automatic or by hand, and the rule's value), the limit of its
//                        medium at its speed with the standard, too long or optics that do not
//                        fit; the length by hand ("0" or Automatisch: automatic again)
//
// Every change is one undo step (doc/phys.rs); a refusal shows below the fields.  Decimal
// numbers take the language's mark or a point.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, Form, Optic};
use jfn_model::phys::{self, PlaceKind, Tree};
use ui::*;

use super::scene::{self, dm_text, place_name, size_text, where_text};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::phys as dphys;
use crate::workspace::inspector::{field_begin, row, rows_begin};

// "2,5" / "2.5" / "-3" metres -> decimetres
fn parse_dm(s: &str) -> Option<i32> {
    let t = s.trim().replace(',', ".");
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, String::from(r)),
        None => (false, t),
    };
    let (a, b) = t.split_once('.').unwrap_or((&t, ""));
    if a.is_empty() && b.is_empty() || !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) || b.len() > 1 || a.len() > 6 {
        return None;
    }
    let v = a.parse::<i32>().unwrap_or(0) * 10 + b.parse::<i32>().unwrap_or(0);
    Some(if neg { -v } else { v })
}

/// a place in the inspector
pub(crate) fn place(n: &mut Net, ui: &mut Ui, id: u32) {
    let tree = Tree::of(&n.doc.proj);
    let Some(x) = tree.get(id).cloned() else { return };
    n.phys.bufs.scope(scene::place_key(id));
    let name = place_name(&tree, id);
    ui.text(TS_SUBTITLE, &tf("{} {}", &[t(x.kind.title()), &name]));
    // the name: Enter applies, Esc puts it back
    let b = &mut n.phys.bufs;
    if b.name_src != (id, name.clone()) {
        b.name = name.clone();
        b.name_src = (id, name.clone());
        b.err.clear();
    }
    ui.text(TS_BODY_STRONG, t("Name"));
    if ui.focused() == ID_PHY_NAME && ui.capture_shortcut(VK_ESCAPE, 0) {
        b.name = name.clone();
    }
    ui.next_size(-1, 0);
    if ui.textbox(ID_PHY_NAME, Some(t("Name")), &mut b.name, 64) & EV_COMMIT != 0 && ui.focused() == ID_PHY_NAME {
        let text = b.name.clone();
        let names = dphys::names();
        match dphys::step(&mut n.doc, "Ort umbenennen", |p| phys::rename_place(p, id, &text, &names)) {
            Ok(_) => n.phys.bufs.err.clear(),
            Err(e) => n.phys.bufs.err = crate::app::tr::msg(&e.msg()),
        }
        ui.repass();
    }
    let devs = tree.devices_in(&n.doc.proj, id).len();
    let (w, d) = tree.size(id);
    rows_begin(ui);
    row(ui, t("Ort"), &scene::path_text(&tree, id));
    if x.kind != PlaceKind::Floor {
        row(ui, t("Größe"), &size_text(w, d));
    }
    if x.kind == PlaceKind::Rack {
        row(ui, t("Höhe"), &tf("{} HE", &[&tree.units(id).to_string()]));
    }
    row(ui, t("Geräte"), &super::count_text(devs));
    ui.box_end();
    // position, size, height or storey
    let fields: Vec<(usize, &str)> = match x.kind {
        PlaceKind::Floor => alloc::vec![(5, t("Etage"))],
        PlaceKind::Rack => alloc::vec![(0, t("Ost (m)")), (1, t("Süd (m)")), (2, t("Breite (m)")), (3, t("Tiefe (m)")), (4, t("Höhe (HE)"))],
        _ => alloc::vec![(0, t("Ost (m)")), (1, t("Süd (m)")), (2, t("Breite (m)")), (3, t("Tiefe (m)"))],
    };
    let src = (id, [x.x_dm as i64, x.y_dm as i64, w as i64, d as i64, tree.units(id) as i64, x.level as i64]);
    let b = &mut n.phys.bufs;
    if b.fields_src != src {
        b.fields = [dm_text(x.x_dm), dm_text(x.y_dm), dm_text(w as i32), dm_text(d as i32), tree.units(id).to_string(), x.level.to_string()];
        b.fields_src = src;
    }
    let mut set = false;
    for (k, key) in fields.iter().copied() {
        field_begin(ui, key);
        let fid = ID_PHY_FIELD0 + k as u32;
        ui.next_size(-1, 0);
        if ui.textbox(fid, None, &mut n.phys.bufs.fields[k], 16) & EV_COMMIT != 0 && ui.focused() == fid {
            set = true;
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_PHY_SET, t("Setzen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        let f = &n.phys.bufs.fields;
        let num = |s: &str| s.trim().parse::<i32>().ok();
        let (px, py, pw, pd) = (parse_dm(&f[0]), parse_dm(&f[1]), parse_dm(&f[2]), parse_dm(&f[3]));
        let (units, level) = (num(&f[4]), num(&f[5]));
        let names = dphys::names();
        let res = match x.kind {
            PlaceKind::Floor => match level {
                Some(l) if (-9..=99).contains(&l) => dphys::step(&mut n.doc, "Etage ändern", |p| phys::set_place(p, id, None, None, None, Some(l), &names)).map_err(|e| crate::app::tr::msg(&e.msg())),
                _ => Err(String::from(t("Die Etage ist eine ganze Zahl von -9 bis 99."))),
            },
            _ => match (px, py, pw, pd) {
                (Some(ax), Some(ay), Some(aw), Some(ad)) if aw > 0 && ad > 0 => {
                    let u = if x.kind == PlaceKind::Rack { units.map(|u| u.clamp(0, 99) as u16) } else { None };
                    if x.kind == PlaceKind::Rack && u.is_none() {
                        Err(String::from(t("Die Höhe ist eine Zahl von Höheneinheiten (1 bis 60).")))
                    } else {
                        dphys::step(&mut n.doc, "Ort ändern", |p| phys::set_place(p, id, Some((ax, ay)), Some((aw as u32, ad as u32)), u, None, &names)).map_err(|e| crate::app::tr::msg(&e.msg()))
                    }
                }
                _ => Err(String::from(t("Position und Größe in Metern, z. B. 2,5; Breite und Tiefe größer als 0."))),
            },
        };
        n.phys.bufs.err = match res {
            Ok(_) => String::new(),
            Err(e) => e,
        };
        ui.repass();
    }
    if !n.phys.bufs.err.is_empty() {
        let e = n.phys.bufs.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if x.kind != PlaceKind::Desk && ui.button(ID_PHY_OPEN, t("Öffnen")) & EV_ACT != 0 {
        super::open(n, id);
        ui.repass();
    }
    if ui.button(ID_PHY_REMOVE, t("Entfernen")) & EV_ACT != 0 {
        super::remove(n, id);
        ui.repass();
    }
    ui.box_end();
}

/// the places a device can go to: every room, rack and desk in the tree's order, with their path
fn targets(tree: &Tree) -> Vec<(u32, String)> {
    let mut v = Vec::new();
    for x in &tree.places {
        if x.kind.holds_devices() && !tree.path(x.id).is_empty() {
            v.push((x.id, scene::path_text(tree, x.id)));
        }
    }
    v
}

/// the "Ort" of a device and its optics
pub(crate) fn device(n: &mut Net, ui: &mut Ui, id: u32) {
    let tree = Tree::of(&n.doc.proj);
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let Some(m) = n.doc.proj.model_of(id) else { return };
    n.phys.bufs.scope(id as u64);
    let w = tree.whereis(&d);
    ui.text(TS_BODY_STRONG, t("Ort"));
    rows_begin(ui);
    row(ui, t("Platz"), &where_text(&n.doc.proj, &tree, id));
    let form = catalog::form(m);
    let h = catalog::rack_units(m);
    row(ui, t("Bauform"), &if form == Form::Rack { tf("{}, {} HE", &[t(form.title()), &h.to_string()]) } else { String::from(t(form.title())) });
    ui.box_end();
    if form != Form::Virtual {
        let list = targets(&tree);
        let mut items: Vec<String> = list.iter().map(|x| x.1.clone()).collect();
        items.insert(0, String::from(t("Ort wählen …")));
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let mut sel = list.iter().position(|x| x.0 == w.place()).map_or(0, |i| i as i32 + 1);
        ui.next_size(-1, 0);
        if ui.combobox(ID_PHY_PLACE, &mut sel, &refs) & EV_ACT != 0
            && sel > 0
            && let Some(&(to, _)) = list.get(sel as usize - 1)
        {
            let names = dphys::names();
            n.phys.bufs.err = match dphys::step(&mut n.doc, "Gerät umstellen", |p| phys::place_device(p, id, to, 0, None, &names)) {
                Ok(_) => String::new(),
                Err(e) => crate::app::tr::msg(&e.msg()),
            };
            ui.repass();
        }
        // the rack unit
        if w.rack != 0 {
            let b = &mut n.phys.bufs;
            if b.unit_src != (id, w.unit) {
                b.unit = w.unit.to_string();
                b.unit_src = (id, w.unit);
            }
            field_begin(ui, t("Höheneinheit"));
            ui.next_size(-1, 0);
            let mut set = ui.textbox(ID_PHY_UNIT, None, &mut b.unit, 4) & EV_COMMIT != 0 && ui.focused() == ID_PHY_UNIT;
            ui.box_end();
            ui.box_begin(0, BOX_H | BOX_END, 8);
            if ui.button(ID_PHY_UNIT_SET, t("Setzen")) & EV_ACT != 0 {
                set = true;
            }
            ui.box_end();
            if set {
                let names = dphys::names();
                let rack = w.rack;
                n.phys.bufs.err = match n.phys.bufs.unit.trim().parse::<u16>() {
                    Ok(u) if u > 0 => match dphys::step(&mut n.doc, "Gerät umstellen", |p| phys::place_device(p, id, rack, u, None, &names)) {
                        Ok(_) => String::new(),
                        Err(e) => crate::app::tr::msg(&e.msg()),
                    },
                    _ => String::from(t("Die Höheneinheit ist eine Zahl ab 1.")),
                };
                ui.repass();
            }
        }
        if !n.phys.bufs.err.is_empty() {
            let e = n.phys.bufs.err.clone();
            crate::workspace::error_text(ui, &e, 0);
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.button(ID_PHY_SHOW, t("Im Plan zeigen")) & EV_ACT != 0 {
            super::show_device(n, id);
            ui.repass();
        }
        ui.box_end();
    }
    // the optics of its cages
    let cages: Vec<jfn_model::PortSpec> = m.ports().into_iter().filter(|p| p.optic.is_some()).map(|p| d.with_optic(p)).collect();
    if cages.is_empty() {
        return;
    }
    ui.text(TS_BODY_STRONG, t("Optiken"));
    let titles = [t(Optic::Sr.title()), t(Optic::Lr.title())];
    for (i, p) in cages.iter().enumerate().take(64) {
        field_begin(ui, &jfn_model::ifname::short(&p.name));
        let mut sel = (p.optic == Some(Optic::Lr)) as i32;
        ui.next_size(-1, 0);
        if ui.combobox(ID_PHY_OPTIC0 + i as u32, &mut sel, &titles) & EV_ACT != 0 {
            let optic = if sel == 1 { Optic::Lr } else { Optic::Sr };
            let port = p.name.clone();
            let _ = dphys::step(&mut n.doc, "Optik tauschen", |pr| phys::set_optic(pr, id, &port, optic));
            ui.repass();
        }
        ui.box_end();
    }
}

/// a cable's length and limit
pub(crate) fn cable(n: &mut Net, ui: &mut Ui, id: u32) {
    let p = &n.doc.proj;
    let tree = Tree::of(p);
    let Some(l) = p.link(id).cloned() else { return };
    if l.cable == jfn_model::CableKind::Wireless {
        return;
    }
    n.phys.bufs.scope(1 << 40 | id as u64);
    let p = &n.doc.proj;
    let f = scene::facts(p, &tree, &l);
    let auto = phys::auto_len_m(p, &tree, l.a.dev, l.b.dev);
    ui.text(TS_BODY_STRONG, t("Länge"));
    rows_begin(ui);
    row(ui, t("Länge"), &if f.len.auto { tf("{} m (automatisch)", &[&f.len.m.to_string()]) } else { tf("{} m (von Hand; automatisch {} m)", &[&f.len.m.to_string(), &auto.to_string()]) });
    row(ui, t("Grenze"), &match f.reach {
        Some(r) => tf("{} m ({})", &[&r.max_m.to_string(), r.std]),
        None => String::from(t("keine")),
    });
    let a = tree.whereis(p.device(l.a.dev).unwrap_or(&jfn_model::Device::default())).place();
    let b = tree.whereis(p.device(l.b.dev).unwrap_or(&jfn_model::Device::default())).place();
    row(ui, t("Von"), &scene::path_text(&tree, a));
    row(ui, t("Nach"), &scene::path_text(&tree, b));
    let delay = phys::delay_ns(p, &tree, &l);
    if delay != l.delay_ns {
        row(ui, t("Laufzeit"), &tf("{} ns aus der Länge", &[&delay.to_string()]));
    }
    ui.box_end();
    if f.too_long
        && let Some(r) = f.reach
    {
        crate::workspace::error_text(ui, &tf("Zu lang: {} m, erlaubt {} m. Die Leitung bleibt aus.", &[&f.len.m.to_string(), &r.max_m.to_string()]), 0);
    } else if let Some(o) = f.optics {
        let text = match o {
            phys::OpticFault::Mismatch(x, y) => tf("Die Optiken passen nicht: {} und {}. Die Leitung bleibt aus.", &[x.short(), y.short()]),
            phys::OpticFault::WrongFiber(x) => tf("Die Faser passt nicht zur Optik {}. Die Leitung bleibt aus.", &[x.short()]),
        };
        crate::workspace::error_text(ui, &text, 0);
    }
    // the length by hand
    let b = &mut n.phys.bufs;
    if b.len_src != (id, l.len_m) {
        b.len = l.len_m.to_string();
        b.len_src = (id, l.len_m);
    }
    field_begin(ui, t("Von Hand (m)"));
    if ui.focused() == ID_PHY_LEN && ui.capture_shortcut(VK_ESCAPE, 0) {
        b.len = l.len_m.to_string();
    }
    ui.next_size(-1, 0);
    let mut set = ui.textbox(ID_PHY_LEN, Some(t("0: automatisch")), &mut b.len, 8) & EV_COMMIT != 0 && ui.focused() == ID_PHY_LEN;
    ui.box_end();
    let mut to: Option<u32> = None;
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(if l.len_m == 0 { ST_DISABLED } else { 0 });
    if ui.button(ID_PHY_LEN_AUTO, t("Automatisch")) & EV_ACT != 0 {
        to = Some(0);
    }
    if ui.button(ID_PHY_LEN_SET, t("Länge setzen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        match n.phys.bufs.len.trim().trim_end_matches('m').trim().parse::<u32>() {
            Ok(m) => to = Some(m),
            Err(_) => n.phys.bufs.err = String::from(t("Die Länge ist eine ganze Zahl von Metern (0: automatisch).")),
        }
    }
    if let Some(m) = to {
        n.phys.bufs.err = match dphys::step(&mut n.doc, "Kabellänge setzen", |pr| phys::set_len(pr, id, m)) {
            Ok(_) => String::new(),
            Err(e) => crate::app::tr::msg(&e.msg()),
        };
        n.phys.bufs.len_src = (0, 0);
        ui.repass();
    }
    if !n.phys.bufs.err.is_empty() {
        let e = n.phys.bufs.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
}
