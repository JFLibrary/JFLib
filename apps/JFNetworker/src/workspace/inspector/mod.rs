// inspector -- the Eigenschaften panel: the selected device, cable or note, several things, the
// project when nothing is selected
//
//   Inspector::new()            the edit buffers and the interface grid's state
//   frame(n, ui)                the panel's content (a dock panel on the right)
//
// device.rs shows a device (name, power, model, interfaces from the simulator with the quick
// settings of the chosen one, cables), cable.rs a cable (ends, kind, speed, delay, loss, plug,
// route), faults.rs the lab's fault injection on each of them (a device, a cable, the
// partition of several devices) and the active faults (also on the project's page).  Here:
// notes edit their text, several things show what is selected and switch all
// their devices on or off, the project edits its title and shows file, state and counts (with
// the cables that are up).  Every change is one undo step; the simulator follows each.  Edit buffers
// keep what the user typed until Enter, a button or Esc; they start over when the value
// changes elsewhere (undo, the canvas).
//
// wlan.rs: the WLAN part of access points (their networks) and stations (the network they
// join, where they stand in the simulator) and the scale on the project's page (shown once the
// project has a device with a radio).
//
// The physical view (P1): a place selected there (and nothing else) shows here
// (workspace/phys/props.rs); devices and cables get their physical part last.
//
// group.rs (G1): a group (name, colour, state, members, Zuklappen, Gruppierung aufheben), the
// background picture (canvas/backdrop.rs's card, also on the project's page), "Gruppieren"
// with several things.

mod cable;
pub(crate) mod device;
pub(crate) mod faults;
pub(crate) mod wlan;
pub(crate) mod group;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::Project;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tc, tf};
use crate::app::Net;
use crate::doc::edits::{self, Item};
use crate::workspace::canvas::{self, cables};

const KEY_W: i32 = 92;

/// the edit buffers: what the user typed, the value it started from (a change of that value
/// elsewhere -- undo, the flyout -- starts the buffer over)
#[derive(Default)]
pub(crate) struct Inspector {
    name: String,
    name_src: String,
    name_for: u32,
    name_err: String,
    note: String,
    note_src: String,
    note_for: u32,
    title: String,
    title_src: String,
    /// the device's interfaces: the grid, the device it shows
    ifs: GridState,
    ifs_for: u32,
    /// the address and gateway fields of the chosen interface, their sources
    ip: String,
    ip_src: (u32, String, String),
    gw: String,
    gw_src: (u32, String),
    /// why the last quick setting was refused
    cfg_err: String,
    /// the cable's fields: speed (Mbit/s), delay (µs), loss (%), their sources
    kbps: String,
    delay: String,
    loss: String,
    link_src: (u32, u32, u64, u32),
    link_err: String,
    /// the fields of the fault cards (faults.rs)
    flt: faults::Bufs,
    /// the WLAN fields (wlan.rs)
    wl: wlan::Bufs,
}

impl Inspector {
    pub(crate) fn new() -> Inspector {
        Inspector::default()
    }

    /// another project: the buffers start over
    pub(crate) fn reset(&mut self) {
        *self = Inspector::new();
    }
}

/// the Eigenschaften panel
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_INSPECTOR, SC_V | pad(12));
    ui.box_begin(0, BOX_V, 10);
    let sel = n.canvas.selected();
    // a place of the physical view (workspace/phys/props.rs)
    let place = n.phys.sel_place.filter(|_| n.phys.on && sel.is_empty());
    match canvas::single_item(n) {
        None if place.is_some() => crate::workspace::phys::props::place(n, ui, place.unwrap_or_default()),
        // a group, the background picture (G1)
        None if group::shows(n) => group::frame(n, ui),
        Some(Item::Device(id)) => device::frame(n, ui, id),
        Some(Item::Link(id)) => cable::frame(n, ui, id),
        Some(Item::Note(id)) => note(n, ui, id),
        None if sel.is_empty() => project(n, ui),
        None => several(n, ui, &sel),
    }
    ui.box_end();
    ui.scroll_end();
}

/// a "key  value" row (the value wraps)
pub(crate) fn row(ui: &mut Ui, key: &str, value: &str) {
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(KEY_W, 0);
    ui.text(TS_BODY | TX_SECONDARY, key);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, value);
    ui.box_end();
}

/// a block of rows
pub(crate) fn rows_begin(ui: &mut Ui) {
    ui.box_begin(0, BOX_V, 6);
}

/// the key column of a row with a control (the control follows, then box_end)
pub(crate) fn field_begin(ui: &mut Ui, key: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(KEY_W, 0);
    ui.text(TS_BODY | TX_SECONDARY, key);
}

fn note(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(o) = n.doc.proj.note(id).cloned() else { return };
    ui.text(TS_SUBTITLE, t("Notiz"));
    let ins = &mut n.inspector;
    if ins.note_for != id || ins.note_src != o.text {
        ins.note = o.text.clone();
        ins.note_src = o.text.clone();
        ins.note_for = id;
    }
    ui.next_size(-1, 120);
    ui.textbox(ID_INS_NOTE, Some(t("Text der Notiz")), &mut ins.note, 2000 | TB_MULTI | TB_WRAP);
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(if ins.note == o.text { ST_DISABLED } else { 0 });
    if ui.button(ID_INS_NOTE_APPLY, t("Übernehmen")) & EV_ACT != 0 {
        let text = String::from(ins.note.trim_end());
        if let Some(e) = edits::note_text(&n.doc.proj, id, &text) {
            n.doc.apply(e);
        }
        ui.repass();
    }
    ui.box_end();
    rows_begin(ui);
    row(ui, t("Position"), &format!("{}, {}", o.x, o.y));
    row(ui, t("Größe"), &format!("{} × {}", o.w, o.h));
    ui.box_end();
}

fn several(n: &mut Net, ui: &mut Ui, sel: &[u32]) {
    let p = &n.doc.proj;
    let devs: Vec<u32> = sel.iter().copied().filter(|&id| p.device(id).is_some()).collect();
    let names: Vec<&str> = devs.iter().filter_map(|&id| p.device(id)).map(|d| d.name.as_str()).collect();
    let links = sel.iter().filter(|&&id| p.link(id).is_some()).count();
    let notes = sel.iter().filter(|&&id| p.note(id).is_some()).count();
    let off = devs.iter().filter(|&&d| p.device(d).is_some_and(|d| d.off)).count();
    ui.text(TS_SUBTITLE, &tf("{} Objekte", &[&sel.len().to_string()]));
    rows_begin(ui);
    row(ui, t("Geräte"), &if names.is_empty() { String::from(t("keine")) } else { names.join(", ") });
    if !devs.is_empty() {
        row(ui, t("Strom"), &match off {
            0 => String::from(t("alle eingeschaltet")),
            k if k == devs.len() => String::from(t("alle ausgeschaltet")),
            k => tf("{} von {} ausgeschaltet", &[&k.to_string(), &devs.len().to_string()]),
        });
    }
    if links > 0 {
        row(ui, tc("Kabel|Anzahl"), &format!("{}", links));
    }
    if notes > 0 {
        row(ui, t("Notizen"), &format!("{}", notes));
    }
    ui.box_end();
    if !devs.is_empty() {
        ui.box_begin(0, BOX_H, 8);
        ui.next_flags(if off == 0 { ST_DISABLED } else { 0 });
        if ui.button(ID_INS_ALL_ON, t("Alle einschalten")) & EV_ACT != 0 {
            cables::set_power(n, &devs, true);
            ui.repass();
        }
        ui.next_flags(if off == devs.len() { ST_DISABLED } else { 0 });
        if ui.button(ID_INS_ALL_OFF, t("Alle ausschalten")) & EV_ACT != 0 {
            cables::set_power(n, &devs, false);
            ui.repass();
        }
        ui.box_end();
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ziehen verschiebt alle; Entf löscht sie, Geräte mit ihren Kabeln."));
    // the lab: a partition of the devices, the faults of what is selected (faults.rs)
    let p = &n.doc.proj;
    let links: Vec<u32> = sel.iter().copied().filter(|&id| p.link(id).is_some()).collect();
    faults::several(n, ui, &devs, &links);
    // Gruppieren (G1, group.rs)
    group::several(n, ui);
}

fn project(n: &mut Net, ui: &mut Ui) {
    ui.text(TS_SUBTITLE, t("Projekt"));
    let title = n.doc.proj.title.clone();
    let ins = &mut n.inspector;
    if ins.title_src != title {
        ins.title = title.clone();
        ins.title_src = title.clone();
    }
    ui.text(TS_BODY_STRONG, t("Titel"));
    if ui.focused() == ID_INS_TITLE && ui.capture_shortcut(VK_ESCAPE, 0) {
        ins.title = title.clone();
    }
    ui.next_size(-1, 0);
    let ev = ui.textbox(ID_INS_TITLE, Some(t("Ohne Titel")), &mut ins.title, 120);
    if ev & EV_COMMIT != 0 && ui.focused() == ID_INS_TITLE {
        if let Some(e) = edits::title(&n.doc.proj, &n.inspector.title) {
            n.doc.apply(e);
            ui.repass();
        }
    }
    let p: &Project = &n.doc.proj;
    rows_begin(ui);
    match &n.doc.path {
        Some(path) => {
            row(ui, t("Datei"), &n.doc.file_name());
            // the folder, shortened to its last two parts
            let dir = path.rsplit_once('/').map_or("", |d| d.0);
            let parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
            let short = if parts.len() > 2 { format!("…/{}", parts[parts.len() - 2..].join("/")) } else { String::from(dir) };
            row(ui, t("Ordner"), &short);
        }
        None => row(ui, t("Datei"), t("noch nicht gespeichert")),
    }
    // unsaved changes with a copy of them (doc/autosave.rs) say so
    let state = match (n.doc.dirty(), n.autosave.has_copy(n.doc.rev)) {
        (false, _) => "gespeichert",
        (true, false) => "geändert",
        (true, true) => "geändert, automatisch gesichert",
    };
    row(ui, t("Zustand"), t(state));
    row(ui, t("Geräte"), &format!("{}", p.devices.len()));
    let up = n.doc.sim.leds.values().filter(|l| l[0] == jfn_sim::Led::Up && l[1] == jfn_sim::Led::Up).count();
    row(ui, tc("Kabel|Anzahl"), &tf("{} ({} aktiv)", &[&p.links.len().to_string(), &up.to_string()]));
    row(ui, t("Notizen"), &format!("{}", p.notes.len()));
    row(ui, t("Simulation"), crate::workspace::sim::mode_text(n));
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(
        TS_CAPTION | TX_SECONDARY | TX_WRAP,
        t("Ziehen Sie ein Gerät aus der Palette auf die Zeichenfläche, oder klicken Sie eine Kachel und dann auf die Fläche (Umschalt: mehrere). Verbinden (C): von einem Gerät auf ein anderes ziehen."),
    );
    // the lab's active faults (faults.rs)
    faults::project(n, ui);
    // the scale of the WLAN ranges, once there is a radio (wlan.rs)
    let p: &Project = &n.doc.proj;
    if p.devices.iter().any(|d| p.model_of(d.id).is_some_and(|m| jfn_model::wlan::role(m) != jfn_model::wlan::Role::None)) {
        wlan::project(n, ui);
    }
    // the background picture (G1: canvas/backdrop.rs)
    crate::workspace::canvas::backdrop::project(n, ui);
}
