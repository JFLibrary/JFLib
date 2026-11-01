// deploy -- the IDE's pane Bereitstellen (A2): files and a unit of this host go to a group of hosts,
// one after the other, each with a health check (jfn_sim deploy.rs runs it, this pane only asks and
// shows)
//
//   DeployUi::new()             the form's choices (they stay with the IDE's window)
//   view(n, ui, k, cx, acts)    the pane: the form on the left, the run's progress per host on the
//                               right; narrow, one above the other
//   start(n, k, cx)             Bereitstellen: every draft is saved, then Action::Deploy (a refusal
//                               goes into the form's note)
//   abort(n, cx)                Abbrechen: Action::DeployAbort
//   make_unit(n, k, cx)         Einheit anlegen: the unit file for the shown .jfs file, opened as
//                               a tab
//   prepare(n, k, cx)           the first time it opens: the folder of the shown file, the unit
//                               that runs from it, the project's first group
//
// The hosts come from a group of the canvas (jfn_model::group: the engine never sees groups, this
// pane gives it the devices), from what is selected on the canvas, or from a list of names.  What
// goes: a list of files and folders of this host, by default the folder of the shown file, and
// always the unit file (/etc/systemd/system/NAME.service, or NAME.service among the files).
//
// Ids from a2 = ID_A20 + 0x800 k: + 2 the group, + 3 the list, + 4 the files, + 5 the unit, + 6 the
// check, + 7 its port, + 8 its path, + 9 its status, + 10 the wait, + 11 the time for the check,
// + 12 Zurückrollen, + 13 Bereitstellen, + 14 Abbrechen, + 15 Einheit anlegen, + 0x10 the hosts'
// kind (.. + 0x13), + 0x20 the form's scroll, + 0x21 and + 0x22 the split, + 0x3F the progress, + 0x40 the
// hosts (rows + 1 + i).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::CliStyle;
use jfn_model::group::Groups;
use jfn_model::unit::{self, DeploySpec, Health, OnFail};
use jfn_sim::{Action, Cmd, DeployRun, HostState, RunState};
use ui::*;

use super::{editor, Act, Cx, Key};
use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::ide as dide;
use crate::workspace::icons::JN_TERMINAL;

/// the form's width and the width of its labels (DIP)
const FORM_W: i32 = 432;
const LABEL_W: i32 = 118;
/// the least height of the IDE's lower area for this pane and the load test (DIP)
pub(crate) const PANE_H: i32 = 340;

/// the kinds of hosts
const T_GROUP: i32 = 0;
const T_SELECTED: i32 = 1;
const T_LIST: i32 = 2;
/// the checks
const H_ACTIVE: i32 = 0;
const H_TCP: i32 = 1;
const H_HTTP: i32 = 2;

/// the choices of the form
pub(crate) struct DeployUi {
    pub(crate) target: i32,
    pub(crate) group: i32,
    pub(crate) list: String,
    pub(crate) files: String,
    pub(crate) unit: String,
    pub(crate) health: i32,
    pub(crate) port: i32,
    pub(crate) path: String,
    pub(crate) status: i32,
    pub(crate) wait_ms: i32,
    pub(crate) timeout_s: i32,
    pub(crate) rollback: i32,
    pub(crate) sel: i32,
    /// the last refusal
    pub(crate) msg: Option<String>,
    inited: bool,
}

impl DeployUi {
    pub(crate) fn new() -> DeployUi {
        DeployUi { target: T_GROUP, group: 0, list: String::new(), files: String::new(), unit: String::new(), health: H_HTTP, port: 80, path: String::from("/health"), status: 200, wait_ms: 2000, timeout_s: 10, rollback: 0, sel: -1, msg: None, inited: false }
    }
}

/// the usable unit files of the store with the file their ExecStart runs
fn units_of(st: &jfn_model::files::Store) -> Vec<(String, String)> {
    let Ok(list) = st.list(unit::UNIT_DIR) else { return Vec::new() };
    let mut v = Vec::new();
    for (name, s) in list {
        if s.dir || !name.ends_with(".service") {
            continue;
        }
        let Some(text) = dide::text_of(st, &unit::path_of(&name)) else { continue };
        let u = unit::parse(&text);
        // (one that cannot start is no suggestion)
        if u.bad() {
            continue;
        }
        let exec = match u.exec {
            unit::Exec::Jfs { file, .. } => file,
            _ => String::new(),
        };
        v.push((name, exec));
    }
    v
}

/// the groups of the project that hold devices: id, name, devices
fn groups_of(n: &Net) -> Vec<(u32, String, Vec<u32>)> {
    let p = &n.doc.proj;
    let g = Groups::of(p);
    p.groups.iter().filter(|x| g.has(x.id)).map(|x| (x.id, x.name.clone(), g.devices(p, x.id))).filter(|x| !x.2.is_empty()).collect()
}

/// the first time the pane opens: sensible choices from the shown file
pub(crate) fn prepare(n: &mut Net, k: usize, cx: &Cx) {
    if n.devwin.get(k).is_none_or(|w| w.ide.deploy.inited) {
        return;
    }
    let folder = match &cx.cur {
        Some(c @ Key::File(_)) => c.dir(),
        _ => String::from("/root"),
    };
    let units = units_of(&cx.st);
    let inside = format!("{}/", folder);
    let shown = match &cx.cur {
        Some(Key::File(p)) => p.as_str(),
        _ => "",
    };
    // the unit that runs the shown file, else one from its folder, else the first
    let unit = units.iter().find(|u| u.1 == shown).or(units.iter().find(|u| u.1.starts_with(&inside))).or(units.first()).map(|u| u.0.clone()).unwrap_or_default();
    if let Some(w) = n.devwin.get(k) {
        let d = &mut w.ide.deploy;
        d.inited = true;
        d.files = folder;
        d.unit = unit;
    }
}

/// the hosts the form names; why not
fn hosts(n: &Net, d: &DeployUi) -> Result<Vec<u32>, String> {
    let p = &n.doc.proj;
    let v: Vec<u32> = match d.target {
        T_GROUP => {
            let gs = groups_of(n);
            let Some(g) = gs.get(d.group.max(0) as usize) else {
                return Err(String::from(t("Das Projekt hat keine Gruppe: Geräte auf der Leinwand wählen und gruppieren (Strg+G) oder die Hosts als Liste nennen.")));
            };
            g.2.clone()
        }
        T_SELECTED => n.canvas.selected().into_iter().filter(|x| p.device(*x).is_some()).collect(),
        _ => {
            let mut v = Vec::new();
            for name in d.list.split([',', ' ']).filter(|x| !x.is_empty()) {
                match p.find(name) {
                    Some(x) => v.push(x.id),
                    None => return Err(tf("Das Gerät {} gibt es nicht.", &[name])),
                }
            }
            v
        }
    };
    // what has no shell in a group stays out (a switch in the frame); a list names what it means
    let v: Vec<u32> = v.into_iter().filter(|x| d.target == T_LIST || p.model_of(*x).is_some_and(|m| m.cli == CliStyle::Linux)).collect();
    if v.is_empty() {
        return Err(String::from(t("Keine Hosts gewählt.")));
    }
    Ok(v)
}

/// Bereitstellen
pub(crate) fn start(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    editor::save(n, k, cx, true)?;
    let Some(w) = n.devwin.get(k) else { return Ok(()) };
    w.ide.deploy.msg = None;
    let d = &w.ide.deploy;
    let (files, unit, health, on_fail, wait, timeout) = (
        d.files.split([',', ' ']).filter(|x| !x.is_empty()).map(String::from).collect::<Vec<_>>(),
        String::from(d.unit.trim()),
        match d.health {
            H_ACTIVE => Health::Active,
            H_TCP => Health::Tcp { port: d.port.clamp(1, 65535) as u16 },
            _ => Health::Http { port: d.port.clamp(1, 65535) as u16, path: String::from(d.path.trim()), status: d.status.clamp(100, 599) as u16 },
        },
        if d.rollback != 0 { OnFail::Rollback } else { OnFail::Stop },
        d.wait_ms.max(0) as u64 * 1_000_000,
        d.timeout_s.max(1) as u64 * 1_000_000_000,
    );
    if unit.is_empty() {
        return Err(String::from(t("Welche Einheit? Ihr Name fehlt.")));
    }
    let Some(w) = n.devwin.peek(k) else { return Ok(()) };
    let list = hosts(n, &w.ide.deploy)?;
    let spec = DeploySpec { src: cx.dev, hosts: list, files, unit, health, wait, timeout, on_fail };
    n.doc.sim.send(Cmd::Apply(Action::Deploy { spec })).map_err(|e| tr::msg(&e.msg()))?;
    Ok(())
}

/// Abbrechen
pub(crate) fn abort(n: &mut Net, cx: &Cx) -> Result<(), String> {
    let Some(id) = n.doc.sim.engine().deployments(cx.dev).last().filter(|r| r.state == RunState::Running).map(|r| r.id) else { return Ok(()) };
    n.doc.sim.send(Cmd::Apply(Action::DeployAbort { dev: cx.dev, id })).map(|_| ()).map_err(|e| tr::msg(&e.msg()))
}

/// Einheit anlegen: the unit file for the shown file, as a tab (one undo step)
pub(crate) fn make_unit(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let exec = match &cx.cur {
        Some(Key::File(p)) if p.ends_with(".jfs") => p.clone(),
        _ => return Err(String::from(t("Zeige zuerst die .jfs-Datei, die als Einheit laufen soll."))),
    };
    let typed = n.devwin.peek(k).map(|w| String::from(w.ide.deploy.unit.trim())).unwrap_or_default();
    let base = exec.rsplit('/').next().unwrap_or(&exec).trim_end_matches(".jfs");
    let name = unit::unit_name(if typed.is_empty() { base } else { &typed });
    let path = dide::new_unit(&mut n.doc, cx.dev, &name, &exec)?;
    if let Some(w) = n.devwin.get(k) {
        w.ide.deploy.unit = name;
    }
    super::open_later(n, k, &path);
    Ok(())
}

// a row: its label, then what follows up to row_end
fn row(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
}

/// the state of a host in words (the language) and as a badge
fn host_word(s: HostState) -> (&'static str, u32) {
    match s {
        HostState::Pending => (t("wartet"), BADGE_NEUTRAL),
        HostState::Copying => (t("kopiert"), BADGE_INFO),
        HostState::Checking => (t("prüft"), BADGE_INFO),
        HostState::Waiting => (t("wartet nach der Prüfung"), BADGE_INFO),
        HostState::Done => (t("fertig"), BADGE_OK),
        HostState::Failed => (t("gescheitert"), BADGE_CRIT),
        HostState::Skipped => (t("nicht begonnen"), BADGE_NEUTRAL),
        HostState::RolledBack => (t("zurückgerollt"), BADGE_WARN),
    }
}

/// the pane
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, acts: &mut Vec<Act>) {
    prepare(n, k, cx);
    let a2 = ID_A20 + 0x800 * k as u32;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    let w = ui.px2dip(ui.box_rect()[2]);
    let wide = w >= FORM_W + 260 || w == 0;
    let run = n.doc.sim.engine().deployments(cx.dev).last().cloned();
    if wide {
        let mut pos = FORM_W;
        ui.next_size(-1, -1);
        ui.split_begin(a2 + 0x21, pos, false);
        form(n, ui, k, a2, cx, run.as_ref(), acts);
        ui.split_next();
        progress(n, ui, k, a2, run.as_ref());
        ui.split_end(&mut pos);
    } else {
        // narrow: one above the other
        let mut pos = 170;
        ui.next_size(-1, -1);
        ui.split_begin(a2 + 0x22, pos, true);
        form(n, ui, k, a2, cx, run.as_ref(), acts);
        ui.split_next();
        progress(n, ui, k, a2, run.as_ref());
        ui.split_end(&mut pos);
    }
    ui.box_end();
}

// the form
fn form(n: &mut Net, ui: &mut Ui, k: usize, a2: u32, cx: &Cx, run: Option<&DeployRun>, acts: &mut Vec<Act>) {
    let gs = groups_of(n);
    let names: Vec<&str> = gs.iter().map(|g| g.1.as_str()).collect();
    let selected = n.canvas.selected().into_iter().filter_map(|x| n.doc.proj.device(x).map(|d| d.name.clone())).collect::<Vec<_>>().join(", ");
    let member = gs.get(n.devwin.peek(k).map_or(0, |w| w.ide.deploy.group.max(0) as usize)).map(|g| g.2.iter().filter_map(|x| n.doc.proj.device(*x).map(|d| d.name.clone())).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    let has_unit = {
        let name = n.devwin.peek(k).map(|w| unit::unit_name(w.ide.deploy.unit.trim())).unwrap_or_default();
        cx.st.exists(&unit::path_of(&name))
    };
    let running = run.is_some_and(|r| r.state == RunState::Running);
    let jfs = matches!(&cx.cur, Some(Key::File(p)) if p.ends_with(".jfs"));
    let Some(w) = n.devwin.get(k) else { return };
    let d = &mut w.ide.deploy;
    ui.next_size(-1, -1);
    ui.scroll_begin(a2 + 0x20, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 6);
    // the hosts
    row(ui, t("Hosts"));
    ui.next_flags(ST_COMPACT);
    if ui.segmented(a2 + 0x10, &mut d.target, &[t("Gruppe"), t("Auswahl"), t("Liste")]) & EV_ACT != 0 {
        ui.repass();
    }
    ui.box_end();
    match d.target {
        T_GROUP => {
            row(ui, "");
            if names.is_empty() {
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Das Projekt hat keine Gruppe."));
            } else {
                ui.next_size(160, 0);
                if ui.combobox(a2 + 2, &mut d.group, &names) & EV_ACT != 0 {
                    ui.repass();
                }
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &member);
                ui.tooltip(&member);
            }
            ui.box_end();
        }
        T_SELECTED => {
            row(ui, "");
            ui.next_size(-1, 0);
            let shown = if selected.is_empty() { String::from(t("Auf der Leinwand ist nichts gewählt.")) } else { selected };
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &shown);
            ui.box_end();
        }
        _ => {
            row(ui, "");
            ui.next_size(-1, 0);
            ui.textbox(a2 + 3, Some(t("z. B. WEB1, WEB2")), &mut d.list, 400);
            ui.box_end();
        }
    }
    // what goes
    row(ui, t("Dateien"));
    ui.next_size(-1, 0);
    ui.textbox(a2 + 4, Some(t("z. B. /root/app")), &mut d.files, 400);
    ui.box_end();
    row(ui, t("Einheit"));
    ui.next_size(-1, 0);
    ui.textbox(a2 + 5, Some(t("z. B. web.service")), &mut d.unit, 80);
    if !has_unit {
        ui.next_flags(ST_COMPACT | if jfs { 0 } else { ST_DISABLED });
        if ui.button(a2 + 15, t("Einheit anlegen")) & EV_ACT != 0 {
            acts.push(Act::DeployUnit);
        }
    }
    ui.box_end();
    // the check
    row(ui, t("Prüfung"));
    ui.next_size(120, 0);
    if ui.combobox(a2 + 6, &mut d.health, &[t("Einheit aktiv"), t("TCP-Port"), t("HTTP-Anfrage")]) & EV_ACT != 0 {
        ui.repass();
    }
    if d.health != H_ACTIVE {
        ui.text(TS_CAPTION | TX_SECONDARY, t("Port"));
        ui.next_size(72, 0);
        ui.number_box(a2 + 7, &mut d.port, &Range::new(1, 65535, 1, 0));
    }
    ui.box_end();
    if d.health == H_HTTP {
        row(ui, "");
        ui.text(TS_CAPTION | TX_SECONDARY, t("Pfad"));
        ui.next_size(-1, 0);
        ui.textbox(a2 + 8, Some("/health"), &mut d.path, 200);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Status"));
        ui.next_size(64, 0);
        ui.number_box(a2 + 9, &mut d.status, &Range::new(100, 599, 1, 0));
        ui.box_end();
    }
    row(ui, t("Zeit für die Prüfung"));
    ui.next_size(72, 0);
    ui.number_box(a2 + 11, &mut d.timeout_s, &Range::new(1, 600, 1, 0));
    ui.text(TS_CAPTION | TX_SECONDARY, t("s, danach warten"));
    ui.next_size(84, 0);
    ui.number_box(a2 + 10, &mut d.wait_ms, &Range::new(0, 600_000, 100, 0));
    ui.text(TS_CAPTION | TX_SECONDARY, t("ms"));
    ui.box_end();
    row(ui, "");
    ui.next_size(-1, 0);
    ui.checkbox(a2 + 12, t("Beim ersten Fehler zurückrollen"), &mut d.rollback);
    ui.tooltip(t("Sonst behalten die fertigen Hosts die neuen Dateien, und die Bereitstellung hält an."));
    ui.box_end();
    // the buttons
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(JN_TERMINAL);
    ui.next_flags(if running || cx.d.off { ST_DISABLED } else { ST_ACCENT });
    if ui.button(a2 + 13, t("Bereitstellen")) & EV_ACT != 0 {
        acts.push(Act::DeployStart);
    }
    ui.next_icon(IC_STOP);
    ui.next_flags(if running { 0 } else { ST_DISABLED });
    if ui.button(a2 + 14, t("Abbrechen")) & EV_ACT != 0 {
        acts.push(Act::DeployAbort);
    }
    ui.box_end();
    if let Some(m) = &d.msg {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, m);
    }
    ui.box_end();
    ui.scroll_end();
}

// the run per host
fn progress(n: &mut Net, ui: &mut Ui, k: usize, a2: u32, run: Option<&DeployRun>) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 6);
    let Some(r) = run else {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP | TX_CENTER, t("Hier steht der Fortschritt je Host: Dateien kopieren, Einheit neu starten, prüfen, dann der nächste."));
        ui.box_end();
        ui.box_end();
        return;
    };
    let total = r.hosts.len().to_string();
    let done = r.done().to_string();
    let (word, badge) = match r.state {
        RunState::Running => (tf("läuft: {} von {} Hosts fertig", &[&done, &total]), BADGE_INFO),
        RunState::Done => (tf("fertig: {} von {} Hosts", &[&done, &total]), BADGE_OK),
        RunState::Failed => (tf("gescheitert: {} von {} Hosts fertig, der Rest bleibt, wie er war", &[&done, &total]), BADGE_CRIT),
        RunState::RolledBack => (String::from(t("gescheitert: alle Hosts zurückgerollt")), BADGE_WARN),
        RunState::Aborted => (tf("abgebrochen: {} von {} Hosts fertig", &[&done, &total]), BADGE_NEUTRAL),
    };
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.status_dot(badge);
    ui.text(TS_BODY_STRONG, &r.unit);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &word);
    ui.box_end();
    ui.next_size(-1, 0);
    ui.progress(a2 + 0x3F, r.done() as i32, r.hosts.len().max(1) as i32);
    // why it failed
    if let Some(h) = r.hosts.iter().find(|h| h.state == HostState::Failed) {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &format!("{}: {}", h.name, tr::msg(&h.note)));
    }
    let Some(mut sel) = n.devwin.get(k).map(|w| w.ide.deploy.sel) else {
        ui.box_end();
        return;
    };
    ui.next_size(-1, -1);
    let (first, end) = ui.list_begin(a2 + 0x40, &mut sel, r.hosts.len() as i32, 24);
    for i in first..end {
        let h = &r.hosts[i as usize];
        let (w, badge) = host_word(h.state);
        ui.list_item_begin(i);
        ui.status_dot(badge);
        ui.next_size(80, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &h.name);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, w);
        ui.list_item_end();
    }
    let (_, s) = ui.list_end(&mut sel);
    if let Some(w) = n.devwin.get(k) {
        w.ide.deploy.sel = s;
    }
    ui.box_end();
}
