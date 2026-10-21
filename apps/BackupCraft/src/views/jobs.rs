// jobs -- the backup jobs: command bar, search and status filter, the grid (sortable,
// several rows selectable) and the inspector of the selected job (overview, history, restore
// points)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::ids;
use bc_proto::j::{self, Value};
use bc_proto::model::*;
use datetime::Cron;
use ui::*;

use crate::app::{Console, Page, ID_JOBS};
use crate::icons::*;
use crate::net::After;
use crate::tr::{en, t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets, wizards};

pub struct State {
    pub grid: GridState,
    pub filter: String,
    pub status: i32,
    pub split: i32,
    pub tab: i32,
}

impl Default for State {
    fn default() -> State {
        State { grid: GridState::default(), filter: String::new(), status: 0, split: 0, tab: 0 }
    }
}

const ID_CB: u32 = ID_JOBS + 10;
const ID_SEARCH: u32 = ID_JOBS + 40;
const ID_STATUS: u32 = ID_JOBS + 50;
const ID_SPLIT: u32 = ID_JOBS + 70;
const ID_INSP: u32 = ID_JOBS + 100;
const ID_GRID: u32 = ID_JOBS + 0x8_0000;

const C_RUN: u32 = ID_CB;
const C_FULL: u32 = ID_CB + 1;
const C_EDIT: u32 = ID_CB + 2;
const C_TOGGLE: u32 = ID_CB + 3;
const C_DELETE: u32 = ID_CB + 4;
const C_POINTS: u32 = ID_CB + 5;
const C_REFRESH: u32 = ID_CB + 6;
const C_NEW: u32 = ID_CB + 7;

/// the icon of a job's kind
pub fn job_icon(k: JobKind) -> u32 {
    match k {
        JobKind::Copy => BC_COPY_JOB,
        JobKind::Tape => BC_TAPE,
        JobKind::Qemu | JobKind::HyperV => BC_VM,
        JobKind::Volume => BC_DISK,
        _ => BC_JOB,
    }
}

pub fn kind_name(k: JobKind) -> &'static str {
    match k {
        JobKind::File => t("Dateien"),
        JobKind::Volume => t("Volume"),
        JobKind::HyperV => t("Hyper-V-VM"),
        JobKind::Qemu => t("KVM-VM"),
        JobKind::Copy => t("Sicherungskopie"),
        JobKind::Tape => t("Band"),
    }
}

/// "Täglich um 02:00" or "Nur manuell" / "Deaktiviert"
pub fn schedule_text(j: &Job) -> String {
    if j.schedule.cron.is_empty() {
        return String::from(t("Nur manuell"));
    }
    match Cron::parse(&j.schedule.cron) {
        Ok(c) => c.describe(en()),
        Err(_) => format!("Cron: {}", j.schedule.cron),
    }
}

/// "14 Punkte · 30 Tage · 8 Wochen · 12 Monate"
pub fn retention_text(r: &Retention) -> String {
    let mut parts = Vec::new();
    if r.points > 0 {
        parts.push(tf("{} Punkte", &[&format!("{}", r.points)]));
    }
    if r.days > 0 {
        parts.push(tf("{} Tage", &[&format!("{}", r.days)]));
    }
    if r.weekly > 0 {
        parts.push(tf("{} Wochen", &[&format!("{}", r.weekly)]));
    }
    if r.monthly > 0 {
        parts.push(tf("{} Monate", &[&format!("{}", r.monthly)]));
    }
    if r.yearly > 0 {
        parts.push(tf("{} Jahre", &[&format!("{}", r.yearly)]));
    }
    if parts.is_empty() {
        return String::from(t("Alle behalten"));
    }
    parts.join(" · ")
}

fn visible(app: &Console) -> Vec<Job> {
    let st = &app.jobs;
    let mut v: Vec<Job> = app
        .net
        .data
        .jobs
        .iter()
        .filter(|j| {
            let host = app.net.data.host(&j.host_id).map_or("", |h| h.name.as_str());
            (widgets::matches(&j.name, &st.filter) || widgets::matches(host, &st.filter) || j.sources.iter().any(|s| widgets::matches(s, &st.filter)))
                && match st.status {
                    1 => j.last_status == SessionStatus::Failed,
                    2 => j.last_status == SessionStatus::Warning,
                    3 => j.last_status == SessionStatus::Running,
                    4 => !j.enabled,
                    _ => true,
                }
        })
        .cloned()
        .collect();
    let (col, desc) = st.grid.sort.unwrap_or((0, false));
    let d = &app.net.data;
    v.sort_by(|a, b| {
        let o = match col {
            1 => kind_name(a.kind).cmp(kind_name(b.kind)),
            2 => d.host(&a.host_id).map(|h| h.name.as_str()).cmp(&d.host(&b.host_id).map(|h| h.name.as_str())),
            3 => d.repo(&a.repository_id).map(|r| r.name.as_str()).cmp(&d.repo(&b.repository_id).map(|r| r.name.as_str())),
            4 => a.next_run.cmp(&b.next_run),
            5 => a.last_status.cmp(&b.last_status).then(a.last_run.cmp(&b.last_run)),
            6 => a.last_size.cmp(&b.last_size),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if desc { o.reverse() } else { o }
    });
    v
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/jobs", 4000, now);
    app.net.want("/api/hosts", 30_000, now);
    app.net.want("/api/repositories", 30_000, now);
    let epoch = app.epoch(ui);
    let jobs = visible(app);
    let keys: Vec<u64> = jobs.iter().map(|j| ids::key(&j.id)).collect();
    app.jobs.grid.retain(&keys);
    let sel: Vec<Job> = jobs.iter().filter(|j| app.jobs.grid.sel.contains(&ids::key(&j.id))).cloned().collect();
    let one = if sel.len() == 1 { Some(sel[0].clone()) } else { None };
    let all = &app.net.data.jobs;
    views::body_begin(ui);
    let failed = all.iter().filter(|j| j.last_status == SessionStatus::Failed).count();
    widgets::page_head(ui, t("Sicherungsjobs"), &tf("{} Jobs · {} aktiv · {} fehlgeschlagen", &[
        &format!("{}", all.len()),
        &format!("{}", all.iter().filter(|j| j.enabled).count()),
        &format!("{}", failed),
    ]));
    widgets::page_head_end(ui);
    // command bar
    let oper = app.may(Role::Operator);
    let admin = app.may(Role::Admin);
    let any = !sel.is_empty();
    let all_enabled = any && sel.iter().all(|j| j.enabled);
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_NEW, IC_ADD, t("Neuer Job")).flags(ST_ACCENT).enabled(oper),
        CmdItem::new(C_RUN, IC_PLAY, t("Jetzt starten")).enabled(oper && any).flags(CBI_SEP),
        CmdItem::new(C_FULL, BC_SNAPSHOT, t("Aktive Vollsicherung")).enabled(oper && one.is_some()),
        CmdItem::new(C_EDIT, IC_EDIT, t("Bearbeiten")).enabled(oper && one.is_some()),
        CmdItem::new(C_TOGGLE, if all_enabled { IC_PAUSE } else { IC_PLAY }, if all_enabled { t("Deaktivieren") } else { t("Aktivieren") }).enabled(oper && any),
        CmdItem::new(C_DELETE, IC_DELETE, t("Löschen")).enabled(admin && any),
        CmdItem::new(C_POINTS, BC_RESTORE, t("Wiederherstellen …")).enabled(one.is_some() && app.may(Role::Restore)).flags(CBI_SEP),
        CmdItem::new(C_REFRESH, IC_REFRESH, t("Aktualisieren")).flags(CBI_END),
    ]);
    match clicked {
        Some(C_NEW) => wizards::open_named(app, ui, "job"),
        Some(C_RUN) | Some(C_FULL) => {
            let full = clicked == Some(C_FULL);
            for j in &sel {
                app.net.action(Method::Post, &format!("/api/jobs/{}/run", j.id), Some(j::obj([("full", Value::Bool(full))])), "job.run",
                               &["/api/jobs"], After::Operation);
            }
        }
        Some(C_EDIT) => {
            if let Some(j) = &one {
                wizards::open_job_edit(app, ui, j);
            }
        }
        Some(C_TOGGLE) => {
            for j in &sel {
                let what = if all_enabled { "disable" } else { "enable" };
                app.net.action(Method::Post, &format!("/api/jobs/{}/{}", j.id, what), None, if all_enabled { "job.disable" } else { "job.enable" },
                               &["/api/jobs", "/api/summary"], After::Nothing);
            }
        }
        Some(C_DELETE) => {
            let names: Vec<String> = sel.iter().map(|j| j.name.clone()).collect();
            let ids: Vec<String> = sel.iter().map(|j| j.id.clone()).collect();
            app.dlg.confirm(Confirm::DeleteJobs(ids), t("Jobs löschen?"),
                            &tf("{} wird gelöscht. Die Restore-Points bleiben im Repository und können weiter wiederhergestellt werden.", &[&names.join(", ")]),
                            t("Löschen"));
        }
        Some(C_POINTS) => {
            if let Some(j) = &one {
                app.restore.job = j.id.clone();
                app.go(ui, Page::Restore);
            }
        }
        Some(C_REFRESH) => app.refresh(ui),
        _ => {}
    }
    // filters
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    widgets::search(ui, ID_SEARCH, &mut app.jobs.filter, t("Jobs filtern"), 300);
    ui.segmented(ID_STATUS, &mut app.jobs.status, &[t("Alle"), t("Fehler"), t("Warnungen"), t("Laufend"), t("Deaktiviert")]);
    ui.spacer(-1);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} angezeigt", &[&format!("{}", jobs.len())]));
    ui.box_end();
    // grid and inspector
    if app.net.data.jobs.is_empty() && app.net.loaded("/api/jobs") {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CARD, 0);
        ui.next_size(-1, -1);
        ui.empty_state(BC_JOB, t("Noch keine Sicherungsjobs"), t("Ein Job legt fest, was gesichert wird, wohin, wie lange die Stände bleiben und wann er läuft."));
        ui.box_end();
        views::body_end(ui);
        return;
    }
    let inspector = one.is_some();
    ui.next_size(-1, -1);
    if inspector {
        let w = ui.client_size().0;
        if app.jobs.split <= 0 {
            app.jobs.split = (w - 236 - 48 - 400).max(420);
        }
        ui.split_begin(ID_SPLIT, app.jobs.split, false);
    } else {
        ui.box_begin(0, BOX_V, 0);
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Name"), 220).flags(GC_FILL).min(140),
        GridCol::new(t("Typ"), 110),
        GridCol::new(t("Quelle"), 170),
        GridCol::new(t("Ziel"), 140),
        GridCol::new(t("Nächster Lauf"), 130),
        GridCol::new(t("Letzter Lauf"), 230),
        GridCol::new(t("Größe"), 90).flags(GC_RIGHT),
    ];
    let (a, b) = ui.grid_begin(ID_GRID, &mut app.jobs.grid, &cols, &keys);
    for i in a..b {
        let jb = &jobs[i as usize];
        let st = &mut app.jobs.grid;
        ui.grid_row(st, i);
        ui.grid_cell(st, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(job_icon(jb.kind), 16);
        ui.next_size(-1, 0);
        ui.text(if jb.enabled { TS_BODY | TX_ELLIPSIS } else { TS_BODY | TX_ELLIPSIS | TX_TERTIARY }, &jb.name);
        ui.box_end();
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, kind_name(jb.kind));
        let host = app.net.data.host(&jb.host_id).map_or(String::new(), |h| h.name.clone());
        let src = if matches!(jb.kind, JobKind::Copy | JobKind::Tape) {
            app.net.data.job(&jb.source_job).map_or(String::new(), |j| j.name.clone())
        } else if jb.sources.len() > 1 {
            tf("{} · {} Quellen", &[&host, &format!("{}", jb.sources.len())])
        } else {
            format!("{} · {}", host, jb.sources.first().map_or("", |s| s.as_str()))
        };
        ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, &src);
        let repo = if jb.kind == JobKind::Tape {
            app.net.data.pools.iter().find(|p| p.id == jb.pool_id).map_or(String::from(t("Medienpool")), |p| format!("{} {}", t("Band:"), p.name))
        } else {
            app.net.data.repo(&jb.repository_id).map_or(String::new(), |r| r.name.clone())
        };
        ui.grid_text_ts(st, 3, TS_BODY | TX_SECONDARY, &repo);
        let next = if !jb.enabled { String::from(t("Deaktiviert")) } else if jb.next_run > 0 { fmt::when_short(jb.next_run, epoch, &app.zone) } else { String::from(t("Manuell")) };
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &next);
        ui.grid_cell(st, 5);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let (label, kind) = fmt::session_status(jb.last_status);
        ui.badge(kind, label);
        if jb.last_run > 0 && jb.last_status != SessionStatus::Running {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &fmt::when_short(jb.last_run, epoch, &app.zone));
        }
        ui.box_end();
        ui.grid_text(st, 6, &fmt::size(jb.last_size));
    }
    let ev = ui.grid_end(&mut app.jobs.grid);
    ui.box_end();
    if ev.ev & EV_DOUBLE != 0 && ev.row >= 0 && oper {
        let j = jobs[ev.row as usize].clone();
        wizards::open_job_edit(app, ui, &j);
    }
    if inspector {
        ui.split_next();
        if let Some(j) = &one {
            inspect(app, ui, j, epoch);
        }
        let mut sp = app.jobs.split;
        if ui.split_end(&mut sp) {
            app.jobs.split = sp;
        }
    } else {
        ui.box_end();
    }
    views::body_end(ui);
}

fn inspect(app: &mut Console, ui: &mut Ui, j: &Job, epoch: i64) {
    let now = ui.now();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED | pad(16), 12);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.icon(job_icon(j.kind), 20);
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_ELLIPSIS, &j.name);
    let (label, kind) = fmt::session_status(j.last_status);
    ui.badge(kind, label);
    ui.box_end();
    ui.tabs(ID_INSP, &mut app.jobs.tab, &[t("Übersicht"), t("Verlauf"), t("Restore-Points")]);
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_INSP + 10, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 14);
    match app.jobs.tab {
        0 => {
            let d = &app.net.data;
            let host = d.host(&j.host_id).map_or(String::from("–"), |h| h.name.clone());
            let repo = d.repo(&j.repository_id).map_or(String::from("–"), |r| r.name.clone());
            let sched = schedule_text(j);
            let next = if !j.enabled { String::from(t("Deaktiviert")) } else if j.next_run > 0 { fmt::when(j.next_run, &app.zone) } else { String::from(t("Nur manuell")) };
            let enc = if j.encrypted {
                if j.key_hint.is_empty() { String::from(t("Ja")) } else { tf("Ja · Hinweis: {}", &[&j.key_hint]) }
            } else {
                String::from(t("Nein"))
            };
            let full = if j.active_full_days > 0 { tf("Alle {} Tage", &[&format!("{}", j.active_full_days)]) } else { String::from(t("Nie (nur geänderte Dateien)")) };
            let ret = retention_text(&j.retention);
            let last = if j.last_run > 0 { format!("{} · {}", fmt::when(j.last_run, &app.zone), fmt::size(j.last_size)) } else { String::from("–") };
            widgets::props(ui, &[
                (t("Typ"), kind_name(j.kind)),
                (t("Host"), &host),
                (t("Repository"), &repo),
                (t("Zeitplan"), &sched),
                (t("Zeitzone"), &j.schedule.tz),
                (t("Nächster Lauf"), &next),
                (t("Letzter Lauf"), &last),
                (t("Aufbewahrung"), &ret),
                (t("Verschlüsselung"), &enc),
                (t("Vollsicherung"), &full),
            ]);
            ui.text(TS_BODY_STRONG, match j.kind {
                JobKind::Copy => t("Kopiert"),
                JobKind::Tape => t("Schreibt auf Band"),
                _ => t("Quellen"),
            });
            ui.box_begin(0, BOX_V, 4);
            if matches!(j.kind, JobKind::Copy | JobKind::Tape) {
                let src = d.job(&j.source_job).map_or(String::from("–"), |x| x.name.clone());
                ui.text(TS_BODY, &src);
            }
            for s in &j.sources {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.icon(IC_FOLDER, 16);
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_ELLIPSIS, s);
                ui.box_end();
            }
            for e in &j.excludes {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.icon(IC_REMOVE_CIRCLE, 16);
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &tf("Ausgeschlossen: {}", &[e]));
                ui.box_end();
            }
            ui.box_end();
            if !j.pre_script.is_empty() || !j.post_script.is_empty() {
                ui.text(TS_BODY_STRONG, t("Skripte"));
                widgets::props(ui, &[(t("Davor"), &j.pre_script), (t("Danach"), &j.post_script)]);
            }
            if app.may(Role::Operator) {
                ui.box_begin(0, BOX_H, 8);
                ui.next_icon(IC_PLAY);
                if ui.accent_button(ID_INSP + 20, t("Jetzt starten")) & EV_ACT != 0 {
                    app.net.action(Method::Post, &format!("/api/jobs/{}/run", j.id), Some(j::obj([("full", Value::Bool(false))])), "job.run",
                                   &["/api/jobs"], After::Operation);
                }
                ui.next_icon(IC_EDIT);
                if ui.button(ID_INSP + 21, t("Bearbeiten")) & EV_ACT != 0 {
                    wizards::open_job_edit(app, ui, j);
                }
                ui.box_end();
            }
        }
        1 => {
            app.net.want("/api/sessions?limit=500", 6000, now);
            let ss: Vec<Session> = app.net.data.sessions.iter().filter(|s| s.job_id == j.id).take(30).cloned().collect();
            if ss.is_empty() {
                ui.text(TS_BODY | TX_SECONDARY, t("Noch keine Sitzungen."));
            }
            for (i, s) in ss.iter().enumerate() {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
                let (label, kind) = fmt::session_status(s.status);
                ui.status_dot(kind);
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_V, 0);
                ui.text(TS_BODY, &format!("{} · {}", fmt::when(s.started, &app.zone), label));
                let dur = if s.finished > 0 { fmt::duration(s.finished - s.started) } else { String::from(t("läuft")) };
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &format!("{} · {} {} · {}", dur, fmt::size(s.bytes_read), t("gelesen"),
                                                                       if s.message.is_empty() { String::from(t("ohne Meldungen")) } else { s.message.clone() }));
                ui.box_end();
                ui.next_flags(ST_SUBTLE | ST_COMPACT);
                if ui.button(ID_INSP + 100 + i as u32, t("Protokoll")) & EV_ACT != 0 {
                    app.dlg.open_log(s);
                }
                ui.box_end();
            }
        }
        _ => {
            let path = format!("/api/jobs/{}/points", j.id);
            app.net.want(&path, 15_000, now);
            let pts = app.net.data.points.get(&j.id).cloned().unwrap_or_default();
            let tape = app.net.data.capable(bc_proto::api::CAP_TAPE);
            if pts.is_empty() {
                ui.text(TS_BODY | TX_SECONDARY, if app.net.loaded(&path) { t("Noch keine Restore-Points.") } else { t("Wird geladen …") });
            }
            for (i, p) in pts.iter().take(40).enumerate() {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
                ui.icon(if p.full { BC_SNAPSHOT } else { BC_ARCHIVE }, 16);
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_V, 0);
                ui.text(TS_BODY, &fmt::when(p.created, &app.zone));
                let mut d = format!("{} · {} {}", fmt::size(p.size), p.files, t("Dateien"));
                if p.immutable_until > epoch {
                    d.push_str(" · ");
                    d.push_str(&tf("unveränderlich bis {}", &[&fmt::when(p.immutable_until, &app.zone)]));
                }
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &d);
                ui.box_end();
                if j.kind == JobKind::Qemu && app.may(Role::Operator) && app.net.data.capable(bc_proto::api::CAP_INSTANT) {
                    let names = crate::views::recovery::vm_names(app, j);
                    let choices: Vec<(String, String, Vec<String>)> = crate::views::recovery::vm_points(app).iter()
                        .map(|(p, jb)| (p.id.clone(), format!("{} · {}", jb.name, fmt::when(p.created, &app.zone)), crate::views::recovery::vm_names(app, jb))).collect();
                    ui.next_flags(ST_SUBTLE | ST_COMPACT);
                    if ui.button(ID_INSP + 500 + i as u32, t("Starten …")) & EV_ACT != 0 {
                        app.dlg.open_instant(p, &names, &choices);
                    }
                    ui.next_flags(ST_SUBTLE | ST_COMPACT);
                    if ui.button(ID_INSP + 600 + i as u32, t("Startprüfung …")) & EV_ACT != 0 {
                        app.dlg.open_boot_check(p, &names, &choices);
                    }
                }
                if tape && app.may(Role::Operator) {
                    ui.next_flags(ST_SUBTLE | ST_COMPACT);
                    if ui.button(ID_INSP + 400 + i as u32, t("Auf Band …")) & EV_ACT != 0 {
                        app.dlg.open_tape_write(p);
                        app.net.get("/api/tape/pools", now);
                    }
                }
                ui.next_flags(ST_SUBTLE | ST_COMPACT);
                if ui.button(ID_INSP + 300 + i as u32, t("Wiederherstellen")) & EV_ACT != 0 {
                    wizards::open_restore(app, ui, j, Some(p.id.clone()));
                }
                ui.box_end();
            }
        }
    }
    ui.box_end();
    ui.scroll_end();
    ui.box_end();
}
