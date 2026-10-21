// recovery -- virtual machines: the hypervisor and its VMs, instant start (VMs that run from
// restore points: start, console, merge, stop), boot checks (a VM of a point started in
// isolation and checked); the page as announced when the server has none of it

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api;
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_RECOVERY};
use crate::icons::*;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub tab: i32,
    pub vms: GridState,
    pub sessions: GridState,
    pub checks: GridState,
}

const ID_TABS: u32 = ID_RECOVERY + 1;
const ID_CB: u32 = ID_RECOVERY + 10;
const C_START: u32 = ID_CB;
const C_CONSOLE: u32 = ID_CB + 1;
const C_MERGE: u32 = ID_CB + 2;
const C_STOP: u32 = ID_CB + 3;
const C_CHECK: u32 = ID_CB + 10;
const ID_VGRID: u32 = ID_RECOVERY + 0x2_0000;
const ID_IGRID: u32 = ID_RECOVERY + 0x5_0000;
const ID_SGRID: u32 = ID_RECOVERY + 0x8_0000;

pub fn vm_state(s: &str) -> (&'static str, u32) {
    match s {
        "running" => (t("Läuft"), BADGE_OK),
        "paused" => (t("Angehalten"), BADGE_WARN),
        "stopped" => (t("Aus"), BADGE_NEUTRAL),
        _ => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

fn session_state(s: &str) -> (&'static str, u32) {
    match s {
        "starting" => (t("Startet"), BADGE_INFO),
        "running" => (t("Läuft aus dem Restore-Point"), BADGE_OK),
        "migrating" => (t("Wird zusammengeführt"), BADGE_INFO),
        "migrated" => (t("Zusammengeführt"), BADGE_ACCENT),
        "stopped" => (t("Beendet"), BADGE_NEUTRAL),
        "failed" => (t("Fehler"), BADGE_CRIT),
        "interrupted" => (t("Unterbrochen"), BADGE_WARN),
        _ => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

/// the points of the VM jobs, newest first: (point, job)
pub fn vm_points(app: &Console) -> Vec<(RestorePoint, Job)> {
    let mut v = Vec::new();
    for j in app.net.data.jobs.iter().filter(|j| j.kind == JobKind::Qemu) {
        for p in app.net.data.points.get(&j.id).map(|x| x.as_slice()).unwrap_or(&[]) {
            v.push((p.clone(), j.clone()));
        }
    }
    v.sort_by_key(|(p, _)| core::cmp::Reverse(p.order()));
    v
}

/// the VM names of a job (as its points name them)
pub fn vm_names(app: &Console, job: &Job) -> Vec<String> {
    job.sources.iter().map(|id| app.net.data.vms.iter().find(|v| v.id == *id).map_or_else(|| String::from(id.rsplit(':').next().unwrap_or(id)), |v| v.name.clone())).collect()
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/capabilities", 60_000, now);
    let d = &app.net.data;
    if app.net.loaded("/api/capabilities") && !d.capable(api::CAP_HYPERVISOR) && !d.capable(api::CAP_INSTANT) && !d.capable(api::CAP_BOOTCHECK) {
        views::staged::frame(app, ui, views::staged::Kind::Recovery);
        return;
    }
    // the points of the VM jobs (the dialogs choose from them)
    let vm_jobs: Vec<String> = app.net.data.jobs.iter().filter(|j| j.kind == JobKind::Qemu).map(|j| j.id.clone()).collect();
    for id in vm_jobs {
        app.net.want(&format!("/api/jobs/{}/points", id), 30_000, now);
    }
    views::body_begin(ui);
    widgets::page_head(ui, t("Sofortstart und Startprüfung"), t("Virtuelle Maschinen sichern, direkt aus einem Restore-Point starten und Sicherungen automatisch auf Startfähigkeit prüfen."));
    widgets::page_head_end(ui);
    ui.tabs(ID_TABS, &mut app.recovery.tab, &[t("Virtuelle Maschinen"), t("Aus Sicherungen gestartet"), t("Startprüfungen")]);
    match app.recovery.tab {
        0 => vms(app, ui),
        1 => sessions(app, ui),
        _ => checks(app, ui),
    }
    views::body_end(ui);
}

fn vms(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/hypervisors", 15_000, now);
    app.net.want("/api/vms", 10_000, now);
    let epoch = app.epoch(ui);
    if let Some(h) = app.net.data.hypervisors.first().cloned() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.icon(BC_SERVER, 16);
        ui.text(TS_BODY_STRONG, &h.host_name);
        ui.text(TS_BODY | TX_SECONDARY, &format!("QEMU {}", if h.version.is_empty() { "–" } else { &h.version }));
        if h.kvm {
            ui.badge(BADGE_OK, t("KVM"));
        } else {
            ui.badge(BADGE_WARN, t("ohne KVM"));
        }
        ui.text(TS_BODY | TX_SECONDARY, &tf("{} VMs, {} laufen", &[&format!("{}", h.vms), &format!("{}", h.running)]));
        ui.spacer(-1);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &tf("QMP-Ordner: {}", &[&if h.folders.is_empty() { String::from("–") } else { h.folders.join(", ") }]));
        ui.box_end();
        if !h.ok {
            widgets::notice(ui, IB_INFO, t("Hypervisor nicht eingerichtet"), &h.message);
        }
    }
    let list: Vec<Vm> = app.net.data.vms.clone();
    let keys: Vec<u64> = list.iter().map(|v| ids::key(&v.id)).collect();
    app.recovery.vms.retain(&keys);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/vms") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_VM, t("Keine virtuellen Maschinen"),
                       t("BackupCraft findet QEMU/KVM-VMs über ihre QMP-Sockets (qemu … -qmp unix:/run/vms/NAME.qmp,server=on,wait=off). Den Ordner trägt man in der Einstellung hypervisor.qmp ein; VM-Jobs legt der Job-Assistent an."));
    } else {
        let cols = [
            GridCol::new(t("VM"), 200).flags(GC_FILL).min(150),
            GridCol::new(t("Zustand"), 120),
            GridCol::new(t("Speicher"), 100).flags(GC_RIGHT),
            GridCol::new(t("CPUs"), 60).flags(GC_RIGHT),
            GridCol::new(t("Platten"), 220),
            GridCol::new(t("Gesehen"), 120),
        ];
        let (a, b) = ui.grid_begin(ID_VGRID, &mut app.recovery.vms, &cols, &keys);
        for i in a..b {
            let v = &list[i as usize];
            let g = &mut app.recovery.vms;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_VM, 16);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &v.name);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &v.host_name);
            ui.box_end();
            ui.box_end();
            ui.grid_cell(g, 1);
            let (label, kind) = vm_state(&v.state);
            ui.badge(kind, label);
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &format!("{} MB", v.memory_mb));
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &format!("{}", v.cpus));
            let disks: Vec<String> = v.disks.iter().map(|d| format!("{} {} ({})", d.format, fmt::size(d.size), d.bus)).collect();
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &disks.join(", "));
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &fmt::when_short(v.seen, epoch, &app.zone));
        }
        ui.grid_end(&mut app.recovery.vms);
    }
    ui.box_end();
}

fn sessions(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/instant-start", 3000, now);
    let epoch = app.epoch(ui);
    let list: Vec<InstantSession> = app.net.data.instant.iter().filter(|s| s.purpose == "instant").cloned().collect();
    let keys: Vec<u64> = list.iter().map(|s| ids::key(&s.id)).collect();
    app.recovery.sessions.retain(&keys);
    let sel: Option<InstantSession> = list.iter().find(|s| app.recovery.sessions.sel.contains(&ids::key(&s.id))).cloned();
    let oper = app.may(Role::Operator);
    let running = sel.as_ref().is_some_and(|s| matches!(s.state.as_str(), "running" | "migrated"));
    let points = vm_points(app);
    let clicked = ui.command_bar(ID_CB + 30, &[
        CmdItem::new(C_START, IC_PLAY, t("VM starten …")).flags(ST_ACCENT).enabled(oper && !points.is_empty()),
        CmdItem::new(C_CONSOLE, BC_TERMINAL, t("Konsole")).enabled(sel.is_some()).flags(CBI_SEP),
        CmdItem::new(C_MERGE, IC_ACCEPT, t("Zusammenführen …")).enabled(oper && sel.as_ref().is_some_and(|s| s.state == "running")),
        CmdItem::new(C_STOP, IC_STOP, t("Beenden")).enabled(oper && running).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_START), _) => {
            let first = points.first().map(|(p, j)| (p.clone(), vm_names(app, j)));
            if let Some((p, names)) = first {
                app.dlg.open_instant(&p, &names, &points.iter().map(|(p, j)| (p.id.clone(), format!("{} · {}", j.name, fmt::when(p.created, &app.zone)), vm_names(app, j))).collect::<Vec<_>>());
            }
        }
        (Some(C_CONSOLE), Some(s)) => app.dlg.open_console(s),
        (Some(C_MERGE), Some(s)) => {
            let root = app.net.data.hosts.iter().find(|h| h.kind == HostKind::Server).and_then(|h| h.restore_roots.first().cloned()).unwrap_or_default();
            app.dlg.open_merge(s, &root);
        }
        (Some(C_STOP), Some(s)) => {
            let text = if s.state == "migrated" { t("Die VM wird beendet. Ihre zusammengeführten Platten bleiben im Zielordner.") } else { t("Die VM wird beendet; was sie seit dem Start geschrieben hat, wird verworfen. Der Restore-Point bleibt unverändert.") };
            app.dlg.confirm(Confirm::StopInstant(s.id.clone()), &tf("{} beenden?", &[&s.vm]), text, t("Beenden"));
        }
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/instant-start") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_VM, t("Keine VM läuft aus einer Sicherung"),
                       t("Der Sofortstart startet eine VM direkt aus einem Restore-Point – ohne alles zurückzukopieren. Ihre Änderungen landen in einer eigenen Datei; „Zusammenführen“ überträgt die Platten im Betrieb auf lokalen Speicher."));
    } else {
        let cols = [
            GridCol::new(t("VM"), 180).flags(GC_FILL).min(140),
            GridCol::new(t("Restore-Point"), 150),
            GridCol::new(t("Zustand"), 200),
            GridCol::new(t("Speicher"), 90).flags(GC_RIGHT),
            GridCol::new(t("Netzwerk"), 100),
            GridCol::new(t("Gestartet"), 120),
            GridCol::new(t("Meldung"), 220),
        ];
        let (a, b) = ui.grid_begin(ID_IGRID, &mut app.recovery.sessions, &cols, &keys);
        for i in a..b {
            let s = &list[i as usize];
            let g = &mut app.recovery.sessions;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_VM, 16);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &s.vm);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &s.job_name);
            ui.box_end();
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &fmt::when(s.point_time, &app.zone));
            ui.grid_cell(g, 2);
            let (label, kind) = session_state(&s.state);
            ui.badge(kind, label);
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &format!("{} MB", s.memory_mb));
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, if s.network == "isolated" { t("isoliert") } else { t("keins") });
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &fmt::when_short(s.started, epoch, &app.zone));
            ui.grid_text_ts(g, 6, TS_BODY | TX_SECONDARY, &s.message);
        }
        ui.grid_end(&mut app.recovery.sessions);
    }
    ui.box_end();
}

fn checks(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/boot-checks", 5000, now);
    let epoch = app.epoch(ui);
    let list: Vec<BootCheckResult> = app.net.data.boot_checks.clone();
    let keys: Vec<u64> = list.iter().map(|s| ids::key(&s.id)).collect();
    app.recovery.checks.retain(&keys);
    if app.recovery.checks.sel.is_empty() {
        if let Some(s) = list.first() {
            app.recovery.checks.select_only(Some(ids::key(&s.id)));
        }
    }
    let sel: Option<BootCheckResult> = list.iter().find(|s| app.recovery.checks.sel.contains(&ids::key(&s.id))).cloned();
    let points = vm_points(app);
    let clicked = ui.command_bar(ID_CB + 31, &[CmdItem::new(C_CHECK, BC_SHIELD_CHECK, t("Prüfen …")).flags(ST_ACCENT).enabled(app.may(Role::Operator) && !points.is_empty())]);
    if clicked == Some(C_CHECK) {
        if let Some((p, j)) = points.first() {
            let names = vm_names(app, j);
            let choices: Vec<(String, String, Vec<String>)> = points.iter().map(|(p, j)| (p.id.clone(), format!("{} · {}", j.name, fmt::when(p.created, &app.zone)), vm_names(app, j))).collect();
            app.dlg.open_boot_check(p, &names, &choices);
        }
    }
    if list.is_empty() && app.net.loaded("/api/boot-checks") {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
        ui.next_size(-1, -1);
        ui.empty_state(BC_SHIELD_CHECK, t("Noch keine Prüfung"),
                       t("Die Startprüfung startet eine VM aus einem Restore-Point ohne Netzwerkzugang und prüft, ob sie läuft, ob ihre Konsole einen Text zeigt und ob sie auf einem Port antwortet. Danach wird alles verworfen."));
        ui.box_end();
        return;
    }
    let h = (52 + 44 * list.len().min(8) as i32).max(140);
    ui.next_size(-1, h);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("VM"), 180).flags(GC_FILL).min(140),
        GridCol::new(t("Restore-Point"), 150),
        GridCol::new(t("Ergebnis"), 160),
        GridCol::new(t("Prüfungen"), 280),
        GridCol::new(t("Dauer"), 110).flags(GC_RIGHT),
        GridCol::new(t("Geprüft"), 120),
    ];
    let (a, b) = ui.grid_begin(ID_SGRID, &mut app.recovery.checks, &cols, &keys);
    for i in a..b {
        let s = &list[i as usize];
        let g = &mut app.recovery.checks;
        ui.grid_row(g, i);
        ui.grid_cell(g, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(BC_SHIELD_CHECK, 16);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &s.vm);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &s.job_name);
        ui.box_end();
        ui.box_end();
        ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &fmt::when(s.point_time, &app.zone));
        ui.grid_cell(g, 2);
        if s.ok {
            ui.badge(BADGE_OK, t("Bestanden"));
        } else {
            ui.badge(BADGE_CRIT, t("Nicht bestanden"));
        }
        let summary: Vec<String> = s.checks.iter().map(|c| format!("{} {}", if c.ok { "✓" } else { "✗" }, c.name)).collect();
        ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &summary.join("  "));
        ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &fmt::duration(s.finished - s.started));
        ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &fmt::when_short(s.started, epoch, &app.zone));
    }
    ui.grid_end(&mut app.recovery.checks);
    ui.box_end();
    let Some(s) = sel else { return };
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_RECOVERY + 0x180, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 12);
    widgets::card_begin(ui, &tf("{} · {}", &[&s.vm, &fmt::when(s.point_time, &app.zone)]), &s.message);
    for c in &s.checks {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.status_dot(if c.ok { BADGE_OK } else { BADGE_CRIT });
        ui.next_size(120, 0);
        ui.text(TS_BODY_STRONG, &c.name);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &c.detail);
        ui.box_end();
    }
    if !s.console.is_empty() {
        ui.separator();
        ui.text(TS_BODY_STRONG, t("Konsole"));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_LAYER | pad(10), 2);
        for l in s.console.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_ELLIPSIS, l);
        }
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.box_end();
    ui.scroll_end();
}
