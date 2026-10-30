// group -- the inspector of a group and of the background picture (G1)
//
//   shows(n)              a group or the picture alone is selected
//   frame(n, ui)          a group: its name (Enter applies, Esc puts it back), where it lies,
//                         what it holds, its colour, Zuklappen / Aufklappen, Gruppierung
//                         aufheben, its members (a click selects one and shows it: a hidden one's
//                         groups open first); the picture: canvas/backdrop.rs's card
//   several(n, ui)        "Gruppieren" for several selected things
//
// Every change is one undo step (doc/groups.rs); a refused name shows why below the field.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::group::{self, GroupColor, Groups};
use ui::*;

use super::{field_begin, row, rows_begin};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::groups as dgroups;
use crate::workspace::canvas::{backdrop, groups};

/// a group or the background picture alone is selected
pub(crate) fn shows(n: &Net) -> bool {
    crate::workspace::canvas::single_group(n).is_some() || backdrop::selected(n)
}

/// the group or picture in the inspector
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    match crate::workspace::canvas::single_group(n) {
        Some(g) => group_page(n, ui, g),
        None => backdrop::inspector(n, ui),
    }
}

// a member's name: a device's, "Notiz: …", "Gruppe …"
fn member_text(n: &Net, gs: &Groups, id: u32) -> String {
    let p = &n.doc.proj;
    if let Some(d) = p.device(id) {
        return d.name.clone();
    }
    if let Some(o) = p.note(id) {
        let first = o.text.lines().next().unwrap_or("").trim();
        return if first.is_empty() { String::from(t("Notiz")) } else { tf("Notiz: {}", &[first]) };
    }
    match gs.get(p, id) {
        Some(g) => tf("Gruppe „{}“", &[&g.name]),
        None => String::from("?"),
    }
}

fn group_page(n: &mut Net, ui: &mut Ui, gid: u32) {
    let gs = Groups::of(&n.doc.proj);
    let Some(g) = gs.get(&n.doc.proj, gid).cloned() else { return };
    ui.text(TS_SUBTITLE, &tf("Gruppe „{}“", &[&g.name]));
    // the name: Enter or Umbenennen applies, Esc puts it back
    let b = &mut n.canvas.grp;
    if b.name_src != (gid, g.name.clone()) {
        b.name = g.name.clone();
        b.name_src = (gid, g.name.clone());
        b.err.clear();
    }
    ui.text(TS_BODY_STRONG, t("Name"));
    if ui.focused() == ID_GRP_NAME && ui.capture_shortcut(VK_ESCAPE, 0) {
        n.canvas.grp.name = g.name.clone();
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    let mut apply = ui.textbox(ID_GRP_NAME, Some(t("Name der Gruppe")), &mut n.canvas.grp.name, 64) & EV_COMMIT != 0 && ui.focused() == ID_GRP_NAME;
    ui.next_flags(if n.canvas.grp.name == g.name { ST_DISABLED } else { 0 });
    if ui.button(ID_GRP_RENAME, t("Umbenennen")) & EV_ACT != 0 {
        apply = true;
    }
    ui.box_end();
    if apply {
        let text = n.canvas.grp.name.clone();
        match dgroups::step(&mut n.doc, "Gruppe umbenennen", |p| group::rename(p, gid, &text)) {
            Ok(_) => n.canvas.grp.err.clear(),
            Err(e) => n.canvas.grp.err = crate::app::tr::msg(&e.msg()),
        }
        ui.repass();
    }
    if !n.canvas.grp.err.is_empty() {
        let e = n.canvas.grp.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    // where it lies, what it holds
    let leaves = gs.leaves(gid);
    let devs = leaves.iter().filter(|&&d| n.doc.proj.device(d).is_some()).count();
    let subs = gs.members(gid).iter().filter(|&&m| gs.has(m)).count();
    rows_begin(ui);
    row(ui, t("Zustand"), t(if g.collapsed { "zugeklappt" } else { "aufgeklappt" }));
    let up = gs.parent(gid).and_then(|u| gs.get(&n.doc.proj, u)).map_or_else(|| String::from(t("oben, in keiner Gruppe")), |u| tf("in der Gruppe „{}“", &[&u.name]));
    row(ui, t("Liegt"), &up);
    let mut what = groups::count_text(devs, leaves.len() - devs);
    if subs > 0 {
        what = if subs == 1 { tf("{}, 1 Gruppe", &[&what]) } else { tf("{}, {} Gruppen", &[&what, &subs.to_string()]) };
    }
    row(ui, t("Inhalt"), &what);
    ui.box_end();
    // the colour role (its name, never the colour alone)
    field_begin(ui, t("Farbe"));
    let names: Vec<&str> = GroupColor::ALL.iter().map(|c| t(c.title())).chain(core::iter::once("")).collect();
    let mut sel = GroupColor::ALL.iter().position(|&c| c == g.color).unwrap_or(0) as i32;
    ui.next_size(-1, 0);
    if ui.combobox(ID_GRP_COLOR, &mut sel, &names) & EV_ACT != 0 {
        if let Some(&c) = GroupColor::ALL.get(sel as usize) {
            let _ = dgroups::step(&mut n.doc, "Farbe der Gruppe ändern", |p| group::set_color(p, gid, c));
        }
        ui.repass();
    }
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_WRAP, 8);
    if ui.button(ID_GRP_COLLAPSE, t(if g.collapsed { "Aufklappen" } else { "Zuklappen" })) & EV_ACT != 0 {
        groups::set_collapsed(n, ui, &[gid], !g.collapsed);
    }
    if ui.button(ID_GRP_UNGROUP, t("Gruppierung aufheben")) & EV_ACT != 0 {
        groups::ungroup_selection(n, ui);
        return;
    }
    ui.box_end();
    // the members: a click selects one and shows it (a hidden one's groups open first)
    ui.text(TS_BODY_STRONG, t("Mitglieder"));
    let members: Vec<u32> = gs.members(gid).to_vec();
    let mut jump = None;
    ui.box_begin(0, BOX_H | BOX_WRAP, 4);
    for (i, &m) in members.iter().enumerate().take(GROUP_MEMBERS) {
        let text = member_text(n, &gs, m);
        ui.next_flags(ST_SUBTLE);
        if ui.label_button(ID_GRP_MEMBER0 + i as u32, &text, TS_BODY, 4) & EV_ACT != 0 {
            jump = Some(m);
        }
        ui.tooltip(t("Wählen und zeigen"));
    }
    ui.box_end();
    if members.len() > GROUP_MEMBERS {
        ui.text(TS_CAPTION | TX_SECONDARY, &tf("… und {} weitere", &[&(members.len() - GROUP_MEMBERS).to_string()]));
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ziehen am Titel oder am Symbol verschiebt die Gruppe mit allem darin; Entf löst sie auf, ihre Geräte bleiben. Doppelklick auf das Symbol klappt sie auf."));
    if let Some(m) = jump {
        groups::show(n, m);
        ui.repass();
    }
}

/// several things selected: "Gruppieren" (Ctrl+G)
pub(crate) fn several(n: &mut Net, ui: &mut Ui) {
    let p = &n.doc.proj;
    let gs = &n.canvas.scene.gs;
    let any = n.canvas.st.sel.iter().any(|&k| k >> 32 == 0 && (p.device(k as u32).is_some() || p.note(k as u32).is_some() || gs.has(k as u32)));
    if !any {
        return;
    }
    let k = groups::selected_groups(n).len();
    if k > 0 {
        rows_begin(ui);
        row(ui, t("Gruppen"), &k.to_string());
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_GRP_GROUP, t("Gruppieren")) & EV_ACT != 0 {
        groups::group_selection(n, ui);
    }
    ui.tooltip(t("Die gewählten Geräte, Notizen und Gruppen in eine neue Gruppe (Strg+G)"));
    ui.box_end();
}
