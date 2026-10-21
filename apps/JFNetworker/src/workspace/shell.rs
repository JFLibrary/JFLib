// shell -- the frame of the main window: title, menu bar, toolbar, the dock with the palette,
// the canvas, the inspector and the simulator's panels, the status bar, the info dialog
//
//   Shell::new()        the dock layout (the user's, else the default) and the dialog flag
//   frame(n, ui)        the whole window, once per frame
//
// Dock: Geräte (left), Eigenschaften and PDUs (right; PDUs shows with the simulation mode),
// Terminal / Ereignisse / Paket / Protokoll / Probleme (bottom, one group).  Terminal (the
// consoles, workspace/terminal.rs) and Protokoll (the devices' log, workspace/logview.rs) are
// U3's; Ereignisse and Paket are the simulation mode's (workspace/sim); Probleme lists
// jfn_model's consistency check, the simulator's findings (Engine::problems) and the parts of
// the project the simulator refused -- a click selects the device (and cable), a double click
// also centres the view on it.  The toolbar has the tools, undo / redo, zoom, grid and
// snapping, and the simulation's part (Echtzeit | Simulation, play, steps, speed, PDUs:
// sim::toolbar); the canvas has the hint of a view in the past above it and the frames of the
// simulation mode over it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::commands;
use crate::app::constants::*;
use crate::app::Net;
use crate::doc::file;
use crate::workspace::icons::*;
use crate::workspace::canvas::tools::Tool;
use crate::workspace::{canvas, inspector, palette, sim};

pub(crate) struct Shell {
    pub dock: DockState,
    pub about: Shared<i32>,
    /// the window title last set
    title: String,
    /// the chosen row of the problems list
    prob_sel: i32,
}

impl Shell {
    pub(crate) fn new() -> Shell {
        Shell { dock: DockState::new(), about: Shared::new(0), title: String::new(), prob_sel: -1 }
    }
}

/// the window
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    n.syms.update(ui);
    commands::states(n, ui);
    // Ctrl+Shift+Z redoes as well (outside text fields, which have their own undo)
    if ui.modal_active() == 0 && ui.popup_count() == 0 {
        match ui.undo_keys() {
            Some(true) => ui.cmd_run(CMD_UNDO, 0),
            Some(false) => ui.cmd_run(CMD_REDO, 0),
            None => {}
        }
    }
    title(n, ui);
    let recent = n.file.recent.clone();
    ui.menu_list(CMD_RECENT0, &recent);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
    ui.menubar(None);
    toolbar(n, ui);
    let panels = [
        DockPanel::new(P_DEVICES, "Geräte", JN_DEVICES).at(DOCK_LEFT, 0),
        DockPanel::new(P_PROPS, "Eigenschaften", IC_SETTINGS).at(DOCK_RIGHT, 0),
        DockPanel::new(P_TERMINAL, "Terminal", JN_TERMINAL).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_EVENTS, "Ereignisse", IC_BULLETED_LIST).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_PACKET, "Paket", JN_PACKET).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_LOG, "Protokoll", IC_HISTORY).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_PROBLEMS, "Probleme", IC_WARNING).at(DOCK_BOTTOM, 0),
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_PDUS, "PDUs", IC_MAIL).at(DOCK_RIGHT, 1) },
    ];
    ui.next_size(-1, -1);
    ui.dock_begin(ID_DOCK, &mut n.shell.dock, &panels);
    while let Some(p) = ui.dock_next(&mut n.shell.dock) {
        match p {
            DOCK_CENTER => {
                sim::hint(n, ui);
                canvas::frame(n, ui);
                sim::anim::overlay(n, ui);
            }
            P_DEVICES => palette::frame(n, ui),
            P_PROPS => inspector::frame(n, ui),
            P_PROBLEMS => problems(n, ui),
            P_TERMINAL => crate::workspace::terminal::panel(n, ui),
            P_EVENTS => sim::events::frame(n, ui),
            P_PACKET => sim::packet::frame(n, ui),
            P_PDUS => sim::pdu::panel(n, ui),
            P_LOG => crate::workspace::logview::panel(n, ui),
            _ => {}
        }
    }
    ui.dock_end(&mut n.shell.dock);
    statusbar(n, ui);
    ui.box_end();
    file::dialogs(n, ui);
    about(n, ui);
    sim::pdu::dialog(n, ui);
    sim::pdu::file_dialog(n, ui);
    n.sim_timer(ui);
    sim::timer(n, ui);
}

// "JFNetworker – Büro.jfnet" with a dot while there are unsaved changes
fn title(n: &mut Net, ui: &mut Ui) {
    let t = format!("JFNetworker – {}{}", n.doc.file_name(), if n.doc.dirty() { " •" } else { "" });
    if t != n.shell.title {
        ui.set_title(&t);
        n.shell.title = t;
    }
}

fn sep(ui: &mut Ui) {
    ui.next_size(1, 20);
    ui.separator();
}

fn toolbar(n: &mut Net, ui: &mut Ui) {
    ui.box_begin(ID_TOOLBAR, BOX_H | BOX_XCENTER | BOX_LAYER | pad(4), 2);
    for c in [CMD_TOOL_SELECT, CMD_TOOL_HAND, CMD_TOOL_CONNECT, CMD_TOOL_NOTE] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    sep(ui);
    for c in [CMD_DELETE, CMD_UNDO, CMD_REDO] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    sep(ui);
    ui.cmd_button(CMD_ZOOM_OUT, ST_ICON_ONLY | ST_SUBTLE);
    let z = (n.canvas.st.vp.zoom * 100.0 + 0.5) as i32;
    ui.next_size(52, 0);
    ui.next_flags(ST_SUBTLE);
    if ui.label_button(ID_TB_ZOOM, &format!("{} %", z), TS_BODY, 4) & EV_ACT != 0 {
        canvas::zoom(n, ui, 0);
    }
    ui.tooltip("Zoom: auf 100 % setzen");
    for c in [CMD_ZOOM_IN, CMD_FIT] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    sep(ui);
    for c in [CMD_GRID, CMD_SNAP, CMD_MINIMAP] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.spacer(-1);
    sim::toolbar(n, ui);
    ui.spacer(4);
    ui.box_end();
}

fn statusbar(n: &mut Net, ui: &mut Ui) {
    let p = &n.doc.proj;
    ui.statusbar_begin();
    ui.text(TS_CAPTION, &n.canvas.tool.status());
    if matches!(n.canvas.tool, Tool::Place(_) | Tool::Note) {
        ui.text(TS_CAPTION | TX_TERTIARY, "Klicken setzt, Umschalt+Klicken setzt mehrere, Esc beendet");
    }
    if n.canvas.tool == Tool::Connect {
        ui.text(TS_CAPTION | TX_TERTIARY, "Von Gerät zu Gerät ziehen oder beide anklicken; Umschalt verbindet sofort, Esc beendet");
    }
    sim::pdu_hint(n, ui);
    sep(ui);
    sim::status(n, ui);
    sep(ui);
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} %", (n.canvas.st.vp.zoom * 100.0 + 0.5) as i32));
    sep(ui);
    let k = n.canvas.st.sel.len();
    ui.text(TS_CAPTION | TX_SECONDARY, &if k == 0 { String::from("Nichts ausgewählt") } else { format!("{} ausgewählt", k) });
    sep(ui);
    let (d, l) = (p.devices.len(), p.links.len());
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} {} · {} Kabel", d, if d == 1 { "Gerät" } else { "Geräte" }, l));
    ui.spacer(-1);
    let grid = n.canvas.st.grid as i32;
    let g = match (n.prefs.grid, n.prefs.snap) {
        (true, true) => format!("Raster {} · Einrasten", grid),
        (true, false) => format!("Raster {}", grid),
        (false, true) => format!("Einrasten {}", grid),
        (false, false) => String::from("Frei"),
    };
    ui.text(TS_CAPTION | TX_TERTIARY, &g);
    ui.statusbar_end();
}

// an empty panel of the simulator
fn empty(ui: &mut Ui, icon: u32, title: &str, text: &str) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
    ui.empty_state(icon, title, text);
    ui.box_end();
}

/// a finding of the problems list
pub(crate) struct Finding {
    pub error: bool,
    pub text: String,
    /// the device to select, the cable and the others involved
    pub dev: Option<u32>,
    pub link: Option<u32>,
    pub others: Vec<u32>,
}

/// jfn_model's check, the simulator's findings and its refusals (errors first)
pub(crate) fn findings(n: &mut Net) -> Vec<Finding> {
    let p = &n.doc.proj;
    let named = |d: Option<u32>, text: &str| match d.and_then(|d| p.device(d)) {
        Some(x) if !text.starts_with(&x.name) => format!("{}: {}", x.name, text),
        _ => String::from(text),
    };
    let mut v: Vec<Finding> = Vec::new();
    for i in p.check() {
        v.push(Finding { error: i.error, text: named(i.dev, &i.text), dev: i.dev, link: i.link, others: Vec::new() });
    }
    for (&id, why) in &n.doc.sim.errors {
        let (dev, link) = if p.link(id).is_some() { (p.link(id).map(|l| l.a.dev), Some(id)) } else { (Some(id), None) };
        let what = match link.and_then(|l| p.link(l)) {
            Some(l) => format!("Das Kabel {} läuft nicht im Simulator: {}", crate::workspace::canvas::cables::ends_text(p, l), why),
            None => named(dev, &format!("Läuft nicht im Simulator: {}", why)),
        };
        v.push(Finding { error: true, text: what, dev, link, others: Vec::new() });
    }
    let probs = n.doc.sim.problems();
    let p = &n.doc.proj;
    let named = |d: u32, text: &str| match p.device(d) {
        Some(x) if !text.contains(x.name.as_str()) => format!("{}: {}", x.name, text),
        _ => String::from(text),
    };
    for x in probs.iter() {
        v.push(Finding { error: x.severity == jfn_sim::Severity::Error, text: named(x.dev, &x.text), dev: Some(x.dev), link: x.link, others: x.others.clone() });
    }
    v.sort_by_key(|f| !f.error);
    v
}

// the findings: a click selects the device (and the cable, the others involved), a double
// click centres the view on it
fn problems(n: &mut Net, ui: &mut Ui) {
    let rows = findings(n);
    if rows.is_empty() {
        empty(ui, IC_ACCEPT, "Keine Probleme", "Das Projekt ist in sich stimmig: eindeutige Namen, passende Kabel, belegte Ports nur einmal; der Simulator findet nichts.");
        return;
    }
    let mut sel = n.shell.prob_sel.min(rows.len() as i32 - 1);
    ui.next_size(-1, -1);
    let (first, end) = ui.list_begin(ID_PROBLEM_LIST, &mut sel, rows.len() as i32, 28);
    for i in first..end {
        let r = &rows[i as usize];
        ui.list_item_begin(i);
        ui.status_dot(if r.error { BADGE_CRIT } else { BADGE_WARN });
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &r.text);
        ui.list_item_end();
    }
    let (ev, s) = ui.list_end(&mut sel);
    n.shell.prob_sel = s;
    if ev & (EV_ACT | EV_DOUBLE) != 0 {
        if let Some(r) = rows.get(s as usize) {
            show(n, r, ev & EV_DOUBLE != 0);
            ui.repass();
        }
    }
}

// select what a finding is about; centre: the view moves to the device
fn show(n: &mut Net, r: &Finding, centre: bool) {
    let st = &mut n.canvas.st;
    st.sel.clear();
    for id in r.dev.iter().chain(r.link.iter()).chain(r.others.iter()) {
        st.sel.insert(*id as u64);
    }
    if let Some(d) = r.dev {
        st.focus = d as u64;
        if centre {
            if let Some(x) = n.doc.proj.device(d) {
                st.vp.cx = x.x as f64 + 32.0;
                st.vp.cy = x.y as f64 + 24.0;
            }
        }
    }
}

fn about(n: &mut Net, ui: &mut Ui) {
    let open = n.shell.about.clone();
    if ui.dialog_begin(ID_ABOUT, "Über JFNetworker", &open) {
        ui.next_size(420, 0);
        ui.text(
            TS_BODY | TX_WRAP,
            "JFNetworker – ein Netzwerksimulator und Labor für verteilte Systeme: Geräte und Kabel auf einer Zeichenfläche, echte Protokolle Byte für Byte, Befehlszeilen für Router, Switches und Hosts.",
        );
        ui.next_size(420, 0);
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            "Dieser Stand: der Topologie-Editor mit Kabeln, Inspektor und dem Simulator in Echtzeit, die Konsolen der Geräte, Gerätefenster und das Protokoll, dazu der Simulationsmodus mit Ereignisliste, Zeitleiste, Paketansicht und eigenen PDUs.",
        );
        ui.dialog_buttons(Some("Schließen"), None, None);
    }
    ui.dialog_end();
}
