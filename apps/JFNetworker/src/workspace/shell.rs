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
// simulation mode over it.  Aufgabe (right, a tab beside Eigenschaften that comes forward with
// a lab) is the lab's task (workspace/task.rs), its dialog the list of labs.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ui::*;

use crate::app::commands;
use crate::app::constants::*;
use crate::app::tr::{t, tf};
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
        DockPanel::new(P_DEVICES, t("Geräte"), JN_DEVICES).at(DOCK_LEFT, 0),
        DockPanel::new(P_PROPS, t("Eigenschaften"), IC_SETTINGS).at(DOCK_RIGHT, 0),
        DockPanel::new(P_TERMINAL, t("Terminal"), JN_TERMINAL).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_EVENTS, t("Ereignisse"), IC_BULLETED_LIST).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_PACKET, t("Paket"), JN_PACKET).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_LOG, t("Protokoll"), IC_HISTORY).at(DOCK_BOTTOM, 0),
        DockPanel::new(P_PROBLEMS, t("Probleme"), IC_WARNING).at(DOCK_BOTTOM, 0),
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_PDUS, t("PDUs"), IC_MAIL).at(DOCK_RIGHT, 1) },
        DockPanel { flags: DP_HIDDEN, ..DockPanel::new(P_TASK, t("Aufgabe"), IC_FLAG).at(DOCK_RIGHT, 0) },
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
            P_TASK => crate::workspace::task::panel(n, ui),
            _ => {}
        }
    }
    ui.dock_end(&mut n.shell.dock);
    statusbar(n, ui);
    ui.box_end();
    file::dialogs(n, ui);
    about(n, ui);
    crate::workspace::settings::dialog(n, ui);
    crate::workspace::settings::shortcuts(n, ui);
    crate::doc::autosave::frame(n, ui);
    sim::pdu::dialog(n, ui);
    sim::pdu::file_dialog(n, ui);
    crate::workspace::task::dialog(n, ui);
    n.sim_timer(ui);
    sim::timer(n, ui);
}

// "JFNetworker – Büro.jfnet" with a dot while there are unsaved changes
fn title(n: &mut Net, ui: &mut Ui) {
    let s = format!("JFNetworker – {}{}", n.doc.file_name(), if n.doc.dirty() { " •" } else { "" });
    if s != n.shell.title {
        ui.set_title(&s);
        n.shell.title = s;
    }
}

fn sep(ui: &mut Ui) {
    ui.next_size(1, 20);
    ui.separator();
}

// the toolbar: a pane of its own (Shift+F6), its parts one Tab stop each (the arrows move
// inside a part)
fn toolbar(n: &mut Net, ui: &mut Ui) {
    ui.focus_pane_begin(ID_TB_PANE);
    ui.box_begin(ID_TOOLBAR, BOX_H | BOX_XCENTER | BOX_LAYER | pad(4), 2);
    ui.focus_group_begin(ID_TB_GRP_TOOLS);
    for c in [CMD_TOOL_SELECT, CMD_TOOL_HAND, CMD_TOOL_CONNECT, CMD_TOOL_NOTE] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
    sep(ui);
    ui.focus_group_begin(ID_TB_GRP_EDIT);
    for c in [CMD_DELETE, CMD_UNDO, CMD_REDO] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
    sep(ui);
    ui.focus_group_begin(ID_TB_GRP_ZOOM);
    ui.cmd_button(CMD_ZOOM_OUT, ST_ICON_ONLY | ST_SUBTLE);
    let z = (n.canvas.st.vp.zoom * 100.0 + 0.5) as i32;
    ui.next_size(52, 0);
    ui.next_flags(ST_SUBTLE);
    if ui.label_button(ID_TB_ZOOM, &tf("{} %", &[&z.to_string()]), TS_BODY, 4) & EV_ACT != 0 {
        canvas::zoom(n, ui, 0);
    }
    ui.tooltip(t("Zoom: auf 100 % setzen"));
    for c in [CMD_ZOOM_IN, CMD_FIT] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
    sep(ui);
    ui.focus_group_begin(ID_TB_GRP_VIEW);
    for c in [CMD_GRID, CMD_SNAP, CMD_MINIMAP] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
    ui.spacer(-1);
    sim::toolbar(n, ui);
    ui.spacer(4);
    ui.box_end();
    ui.focus_pane_end();
}

fn statusbar(n: &mut Net, ui: &mut Ui) {
    let p = &n.doc.proj;
    ui.statusbar_begin();
    ui.text(TS_CAPTION, &n.canvas.tool.status());
    if matches!(n.canvas.tool, Tool::Place(_) | Tool::Note) {
        ui.text(TS_CAPTION | TX_TERTIARY, t("Klicken setzt, Umschalt+Klicken setzt mehrere, Esc beendet"));
    }
    if n.canvas.tool == Tool::Connect {
        match n.canvas.kb_from.and_then(|d| n.doc.proj.device(d)) {
            Some(a) => ui.text(TS_CAPTION | TX_TERTIARY, &tf("Ende A: {} – Tab zum zweiten Gerät, Eingabe öffnet die Port-Auswahl, Esc bricht ab", &[&a.name])),
            None => ui.text(TS_CAPTION | TX_TERTIARY, t("Von Gerät zu Gerät ziehen, beide anklicken oder mit Tab und Eingabe wählen; Umschalt verbindet sofort, Esc beendet")),
        };
    }
    sim::pdu_hint(n, ui);
    sep(ui);
    sim::status(n, ui);
    sep(ui);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} %", &[&((n.canvas.st.vp.zoom * 100.0 + 0.5) as i32).to_string()]));
    sep(ui);
    let k = n.canvas.st.sel.len();
    ui.text(TS_CAPTION | TX_SECONDARY, &if k == 0 { String::from(t("Nichts ausgewählt")) } else { tf("{} ausgewählt", &[&k.to_string()]) });
    sep(ui);
    let devs = if p.devices.len() == 1 { String::from(t("1 Gerät")) } else { tf("{} Geräte", &[&p.devices.len().to_string()]) };
    let links = if p.links.len() == 1 { String::from(t("1 Kabel")) } else { tf("{} Kabel", &[&p.links.len().to_string()]) };
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {}", devs, links));
    ui.spacer(-1);
    let grid = (n.canvas.st.grid as i32).to_string();
    let g = match (n.prefs.grid, n.prefs.snap) {
        (true, true) => tf("Raster {} · Einrasten", &[&grid]),
        (true, false) => tf("Raster {}", &[&grid]),
        (false, true) => tf("Einrasten {}", &[&grid]),
        (false, false) => String::from(t("Frei")),
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
        v.push(Finding { error: i.error, text: named(i.dev, &crate::app::tr::msg(&i.msg)), dev: i.dev, link: i.link, others: Vec::new() });
    }
    for (&id, why) in &n.doc.sim.errors {
        let why = crate::app::tr::msg(why);
        let (dev, link) = if p.link(id).is_some() { (p.link(id).map(|l| l.a.dev), Some(id)) } else { (Some(id), None) };
        let what = match link.and_then(|l| p.link(l)) {
            Some(l) => tf("Das Kabel {} läuft nicht im Simulator: {}", &[&crate::workspace::canvas::cables::ends_text(p, l), &why]),
            None => named(dev, &tf("Läuft nicht im Simulator: {}", &[&why])),
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
        v.push(Finding { error: x.severity == jfn_sim::Severity::Error, text: named(x.dev, &crate::app::tr::msg(&x.msg)), dev: Some(x.dev), link: x.link, others: x.others.clone() });
    }
    v.sort_by_key(|f| !f.error);
    v
}

// the findings: a click selects the device (and the cable, the others involved), a double
// click centres the view on it
fn problems(n: &mut Net, ui: &mut Ui) {
    let rows = findings(n);
    if rows.is_empty() {
        empty(ui, IC_ACCEPT, t("Keine Probleme"), t("Das Projekt ist in sich stimmig: eindeutige Namen, passende Kabel, belegte Ports nur einmal; der Simulator findet nichts."));
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
    if ui.dialog_begin(ID_ABOUT, t("Über JFNetworker"), &open) {
        ui.next_size(420, 0);
        ui.text(
            TS_BODY | TX_WRAP,
            t("JFNetworker – ein Netzwerksimulator und Labor für verteilte Systeme: Geräte und Kabel auf einer Zeichenfläche, echte Protokolle Byte für Byte, Befehlszeilen für Router, Switches und Hosts."),
        );
        ui.next_size(420, 0);
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            t("Phase 1: der Topologie-Editor mit Kabeln und Inspektor, der Simulator in Echtzeit, die Konsolen der Geräte, Gerätefenster und das Protokoll, der Simulationsmodus mit Ereignisliste, Zeitleiste, Paketansicht und eigenen PDUs, Einstellungen, automatische Sicherung, Deutsch und Englisch."),
        );
        ui.dialog_buttons(Some(t("Schließen")), None, None);
    }
    ui.dialog_end();
}
