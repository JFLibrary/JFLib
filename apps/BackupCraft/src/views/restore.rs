// restore -- restore: the jobs with restore points at the left, the selected job's points at
// the right (restore, browse, verify, delete), a 14-day strip of the points

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_RESTORE};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets, wizards};

#[derive(Default)]
pub struct State {
    pub job: String,
    pub jobs_grid: GridState,
    pub grid: GridState,
    pub filter: String,
}

const ID_CB: u32 = ID_RESTORE + 10;
const C_RESTORE: u32 = ID_CB;
const C_BROWSE: u32 = ID_CB + 1;
const C_VERIFY: u32 = ID_CB + 2;
const C_DELETE: u32 = ID_CB + 3;
const ID_SEARCH: u32 = ID_RESTORE + 40;
const ID_STRIP: u32 = ID_RESTORE + 60;
const ID_JGRID: u32 = ID_RESTORE + 0x4_0000;
const ID_GRID: u32 = ID_RESTORE + 0x8_0000;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/jobs", 8000, now);
    let epoch = app.epoch(ui);
    views::body_begin(ui);
    widgets::page_head(ui, t("Wiederherstellung"), t("Dateien und Ordner aus einem Restore-Point zurückholen – an den Originalort oder woandershin."));
    widgets::page_head_end(ui);
    let jobs: Vec<Job> = app
        .net
        .data
        .jobs
        .iter()
        .filter(|j| j.points > 0 && widgets::matches(&j.name, &app.restore.filter))
        .cloned()
        .collect();
    if app.restore.job.is_empty() || !jobs.iter().any(|j| j.id == app.restore.job) {
        if let Some(j) = jobs.first() {
            app.restore.job = j.id.clone();
        }
    }
    let jkeys: Vec<u64> = jobs.iter().map(|j| ids::key(&j.id)).collect();
    app.restore.jobs_grid.select_only(Some(ids::key(&app.restore.job)));
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 16);
    // jobs
    ui.next_size(340, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    ui.box_begin(0, BOX_V | pad(12), 0);
    ui.next_size(-1, 0);
    widgets::search(ui, ID_SEARCH, &mut app.restore.filter, t("Jobs filtern"), -1);
    ui.box_end();
    let cols = [GridCol::new(t("Job"), 180).flags(GC_FILL), GridCol::new(t("Letzter Stand"), 120)];
    let (a, b) = ui.grid_begin(ID_JGRID, &mut app.restore.jobs_grid, &cols, &jkeys);
    for i in a..b {
        let j = &jobs[i as usize];
        let st = &mut app.restore.jobs_grid;
        ui.grid_row(st, i);
        ui.grid_cell(st, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &j.name);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &tf("{} Restore-Points", &[&format!("{}", j.points)]));
        ui.box_end();
        ui.grid_text_ts(st, 1, TS_CAPTION | TX_SECONDARY, &fmt::when_short(j.last_run, epoch, &app.zone));
    }
    let ev = ui.grid_end(&mut app.restore.jobs_grid);
    if ev.ev & EV_ACT != 0 {
        if let Some(k) = app.restore.jobs_grid.cur {
            if let Some(j) = jobs.iter().find(|j| ids::key(&j.id) == k) {
                app.restore.job = j.id.clone();
                app.restore.grid = GridState::default();
            }
        }
    }
    ui.box_end();
    // points of the job
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 12);
    let job = app.net.data.job(&app.restore.job).cloned();
    match job {
        None => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V | BOX_CARD, 0);
            ui.next_size(-1, -1);
            ui.empty_state(BC_RESTORE, t("Noch keine Restore-Points"), t("Sobald ein Job gelaufen ist, erscheinen seine Restore-Points hier."));
            ui.box_end();
        }
        Some(job) => points(app, ui, &job, epoch),
    }
    ui.box_end();
    ui.box_end();
    views::body_end(ui);
}

fn points(app: &mut Console, ui: &mut Ui, job: &Job, epoch: i64) {
    let now = ui.now();
    let path = format!("/api/jobs/{}/points", job.id);
    app.net.want(&path, 10_000, now);
    let pts = app.net.data.points.get(&job.id).cloned().unwrap_or_default();
    let keys: Vec<u64> = pts.iter().map(|p| ids::key(&p.id)).collect();
    app.restore.grid.retain(&keys);
    if app.restore.grid.sel.is_empty() {
        if let Some(p) = pts.first() {
            app.restore.grid.select_only(Some(ids::key(&p.id)));
        }
    }
    let sel: Option<RestorePoint> = pts.iter().find(|p| app.restore.grid.sel.contains(&ids::key(&p.id))).cloned();
    let restore_ok = app.may(Role::Restore);
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_RESTORE, BC_RESTORE, t("Dateien wiederherstellen …")).flags(ST_ACCENT).enabled(restore_ok && sel.is_some()),
        CmdItem::new(C_BROWSE, IC_FOLDER_OPEN, t("Inhalt durchsuchen")).enabled(restore_ok && sel.is_some()),
        CmdItem::new(C_VERIFY, BC_SHIELD_CHECK, t("Prüfen")).enabled(app.may(Role::Operator) && sel.is_some()),
        CmdItem::new(C_DELETE, IC_DELETE, t("Löschen")).enabled(app.may(Role::Admin) && sel.as_ref().is_some_and(|p| p.immutable_until <= epoch)),
    ]);
    match (clicked, &sel) {
        (Some(C_RESTORE), Some(p)) | (Some(C_BROWSE), Some(p)) => wizards::open_restore(app, ui, job, Some(p.id.clone())),
        (Some(C_VERIFY), Some(p)) => {
            app.net.action(Method::Post, &format!("/api/points/{}/verify", p.id), None, "point.verify", &[], After::Operation);
        }
        (Some(C_DELETE), Some(p)) => {
            app.dlg.confirm(Confirm::DeletePoint(p.id.clone(), job.id.clone()), t("Restore-Point löschen?"),
                            &tf("Der Stand vom {} wird gelöscht. Daten, die kein anderer Stand braucht, gibt die nächste Bereinigung frei.", &[&fmt::when(p.created, &app.zone)]),
                            t("Löschen"));
        }
        _ => {}
    }
    // 14-day strip
    widgets::card_begin(ui, &job.name, &tf("{} Restore-Points · Aufbewahrung: {}", &[&format!("{}", pts.len()), &crate::views::jobs::retention_text(&job.retention)]));
    strip(app, ui, &pts, epoch);
    widgets::card_end(ui);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Zeitpunkt"), 150).flags(GC_FILL).min(140),
        GridCol::new(t("Art"), 100),
        GridCol::new(t("Dateien"), 80).flags(GC_RIGHT),
        GridCol::new(t("Größe"), 80).flags(GC_RIGHT),
        GridCol::new(t("Neu gespeichert"), 110).flags(GC_RIGHT),
        GridCol::new(t("Schutz"), 130),
        GridCol::new(t("Geprüft"), 120),
    ];
    let (a, b) = ui.grid_begin(ID_GRID, &mut app.restore.grid, &cols, &keys);
    for i in a..b {
        let p = &pts[i as usize];
        let st = &mut app.restore.grid;
        ui.grid_row(st, i);
        ui.grid_text(st, 0, &fmt::when(p.created, &app.zone));
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, if p.full { t("Vollständig") } else { t("Inkrementell") });
        ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, &fmt::count(p.files));
        ui.grid_text(st, 3, &fmt::size(p.size));
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &fmt::size(p.stored_new));
        ui.grid_cell(st, 5);
        if p.immutable_until > epoch {
            ui.next_icon(IC_LOCK);
            ui.badge(BADGE_INFO, &tf("bis {}", &[&datetime::format_date(datetime::Timestamp::from_secs(p.immutable_until), &app.zone, crate::tr::en())]));
        } else if p.encrypted {
            ui.next_icon(IC_LOCK);
            ui.badge(BADGE_NEUTRAL, t("Verschlüsselt"));
        } else {
            ui.text(TS_BODY | TX_TERTIARY, "–");
        }
        if p.verified > 0 {
            ui.grid_cell(st, 6);
            ui.badge(BADGE_OK, &fmt::when_short(p.verified, epoch, &app.zone));
        } else {
            ui.grid_text_ts(st, 6, TS_BODY | TX_TERTIARY, t("Nicht geprüft"));
        }
    }
    let ev = ui.grid_end(&mut app.restore.grid);
    ui.box_end();
    if ev.ev & EV_DOUBLE != 0 && ev.row >= 0 && app.may(Role::Restore) {
        let id = pts[ev.row as usize].id.clone();
        wizards::open_restore(app, ui, job, Some(id));
    }
}

/// the points of the last 14 days as dots on a day strip (click: select)
fn strip(app: &mut Console, ui: &mut Ui, pts: &[RestorePoint], epoch: i64) {
    ui.next_size(-1, 54);
    let r = ui.place(0, 0);
    let days = 14;
    let start = epoch - days * 86_400;
    let w = r[2].max(1);
    let y = r[1] + ui.dp(18);
    let line = ui.color(T_DIVIDER);
    ui.draw_color(line);
    ui.draw_rect(r[0], y, w, ui.hair());
    let t3 = ui.color(T_TEXT3);
    for d in 0..=days {
        let x = r[0] + (w as i64 * d / days) as i32;
        ui.draw_color(line);
        ui.draw_rect(x, y - ui.dp(4), ui.hair(), ui.dp(8));
        if d % 2 == 0 && d < days {
            let c = app.zone.to_local(datetime::Timestamp::from_secs(start + d * 86_400));
            let s = format!("{}.{}.", c.day, c.month);
            ui.ts_font(TS_CAPTION);
            let (font, size) = ui.text.get_font();
            ui.cmd_text(x + ui.dp(3), y + ui.dp(22), s.as_bytes(), font, size, t3, 0);
        }
    }
    let sel = app.restore.grid.cur;
    for (i, p) in pts.iter().enumerate() {
        if p.created < start {
            continue;
        }
        let x = r[0] + ((p.created - start) as i64 * w as i64 / (days * 86_400)) as i32;
        let s = ui.dp(12);
        let ev = ui.hit_rect(ID_STRIP + i as u32, x - s / 2, y - s / 2, s, s, 0);
        let selected = sel == Some(ids::key(&p.id));
        let c = ui.color(if selected { T_ACCENT } else if p.full { T_INFO } else { T_TEXT3 });
        ui.draw_color(c);
        let rad = if selected || ev & EV_HOVER != 0 { ui.dp(6) } else { ui.dp(4) };
        ui.draw_circle(x << 8, y << 8, rad << 8, 0);
        if ev & EV_ACT != 0 {
            app.restore.grid.select_only(Some(ids::key(&p.id)));
        }
        if ev & EV_HOVER != 0 {
            ui.tooltip(&format!("{} · {}", fmt::when(p.created, &app.zone), fmt::size(p.size)));
        }
    }
}
