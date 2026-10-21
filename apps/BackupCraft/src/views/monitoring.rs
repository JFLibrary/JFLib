// monitoring -- sessions (with their log), operations (progress, cancel), alerts, the server
// fleet (views::fleet), audit log

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_MON};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub tab: i32,
    pub sessions: GridState,
    pub ops: GridState,
    pub audit: GridState,
    pub filter: String,
    pub split: i32,
    pub kind: i32,
    pub fleet: GridState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Sessions,
    Operations,
    Alerts,
    Fleet,
    Audit,
}

/// the tabs this user sees on this server
fn tabs(app: &Console) -> Vec<(&'static str, Tab)> {
    let mut v = alloc::vec![(t("Sitzungen"), Tab::Sessions), (t("Vorgänge"), Tab::Operations), (t("Hinweise"), Tab::Alerts)];
    if app.net.data.capable(bc_proto::api::CAP_FLEET) {
        v.push((t("Serverflotte"), Tab::Fleet));
    }
    if app.may(Role::Admin) {
        v.push((t("Audit-Protokoll"), Tab::Audit));
    }
    v
}

/// show the fleet tab (an alert about a watched server)
pub fn open_fleet(app: &mut Console, id: &str) {
    if let Some(i) = tabs(app).iter().position(|(_, k)| *k == Tab::Fleet) {
        app.mon.tab = i as i32;
        app.mon.fleet.select_only(Some(ids::key(id)));
    }
}

const ID_TABS: u32 = ID_MON + 1;
const ID_SEARCH: u32 = ID_MON + 10;
const ID_KIND: u32 = ID_MON + 11;
const ID_SPLIT: u32 = ID_MON + 20;
const ID_CANCEL: u32 = ID_MON + 30;
const ID_LOG: u32 = ID_MON + 40;
/// the alerts' infobars (each also takes the next id: its close button)
const ID_ALERT: u32 = ID_MON + 0x4000;
const ID_SGRID: u32 = ID_MON + 0x2_0000;
const ID_OGRID: u32 = ID_MON + 0x5_0000;
const ID_AGRID: u32 = ID_MON + 0x8_0000;

pub fn session_kind(k: SessionKind) -> &'static str {
    match k {
        SessionKind::Backup => t("Sicherung"),
        SessionKind::Restore => t("Wiederherstellung"),
        SessionKind::Verify => t("Prüfung"),
        SessionKind::Retention => t("Aufbewahrung"),
        SessionKind::Copy => t("Kopie"),
        SessionKind::Maintenance => t("Wartung"),
        SessionKind::Replica => t("Replikat"),
        SessionKind::Failback => t("Failback"),
        SessionKind::Tape => t("Band"),
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    views::body_begin(ui);
    widgets::page_head(ui, t("Monitoring"), t("Was gelaufen ist, was gerade läuft und wer was geändert hat."));
    widgets::page_head_end(ui);
    let tabs = tabs(app);
    let names: Vec<&str> = tabs.iter().map(|(n, _)| *n).collect();
    ui.tabs(ID_TABS, &mut app.mon.tab, &names);
    match tabs.get(app.mon.tab as usize).map_or(Tab::Sessions, |x| x.1) {
        Tab::Sessions => sessions(app, ui),
        Tab::Operations => operations(app, ui),
        Tab::Alerts => alerts(app, ui),
        Tab::Fleet => crate::views::fleet::tab(app, ui),
        Tab::Audit => audit(app, ui),
    }
    views::body_end(ui);
}

fn sessions(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/sessions?limit=500", 4000, now);
    let epoch = app.epoch(ui);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    widgets::search(ui, ID_SEARCH, &mut app.mon.filter, t("Sitzungen filtern"), 280);
    ui.segmented(ID_KIND, &mut app.mon.kind, &[t("Alle"), t("Fehler"), t("Warnungen"), t("Laufend"), t("Wiederherstellungen")]);
    ui.box_end();
    let list: Vec<Session> = app
        .net
        .data
        .sessions
        .iter()
        .filter(|s| widgets::matches(&s.job_name, &app.mon.filter))
        .filter(|s| match app.mon.kind {
            1 => s.status == SessionStatus::Failed,
            2 => s.status == SessionStatus::Warning,
            3 => s.status == SessionStatus::Running,
            4 => s.kind == SessionKind::Restore,
            _ => true,
        })
        .cloned()
        .collect();
    let keys: Vec<u64> = list.iter().map(|s| ids::key(&s.id)).collect();
    app.mon.sessions.retain(&keys);
    if app.mon.sessions.sel.is_empty() {
        if let Some(s) = list.first() {
            app.mon.sessions.select_only(Some(ids::key(&s.id)));
        }
    }
    let sel = list.iter().find(|s| app.mon.sessions.sel.contains(&ids::key(&s.id))).cloned();
    if app.mon.split <= 0 {
        app.mon.split = (ui.client_size().0 - 236 - 48 - 400).max(480);
    }
    ui.next_size(-1, -1);
    ui.split_begin(ID_SPLIT, app.mon.split, false);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Job"), 200).flags(GC_FILL).min(150),
        GridCol::new(t("Art"), 100),
        GridCol::new(t("Status"), 130),
        GridCol::new(t("Gestartet"), 120),
        GridCol::new(t("Dauer"), 100),
        GridCol::new(t("Gelesen"), 80).flags(GC_RIGHT),
    ];
    let (a, b) = ui.grid_begin(ID_SGRID, &mut app.mon.sessions, &cols, &keys);
    for i in a..b {
        let s = &list[i as usize];
        let st = &mut app.mon.sessions;
        ui.grid_row(st, i);
        ui.grid_text(st, 0, &s.job_name);
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, session_kind(s.kind));
        ui.grid_cell(st, 2);
        let (label, kind) = fmt::session_status(s.status);
        ui.badge(kind, label);
        ui.grid_text_ts(st, 3, TS_BODY | TX_SECONDARY, &fmt::when_short(s.started, epoch, &app.zone));
        let dur = if s.finished > 0 { fmt::duration(s.finished - s.started) } else { fmt::duration(epoch - s.started) };
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &dur);
        ui.grid_text(st, 5, &fmt::size(s.bytes_read));
    }
    ui.grid_end(&mut app.mon.sessions);
    ui.box_end();
    ui.split_next();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED | pad(16), 10);
    match sel {
        None => {
            ui.next_size(-1, -1);
            ui.empty_state(BC_LOG, t("Keine Sitzung gewählt"), "");
        }
        Some(s) => session_detail(app, ui, &s, epoch),
    }
    ui.box_end();
    let mut sp = app.mon.split;
    if ui.split_end(&mut sp) {
        app.mon.split = sp;
    }
}

fn session_detail(app: &mut Console, ui: &mut Ui, s: &Session, epoch: i64) {
    let now = ui.now();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 0);
    ui.text(TS_SUBTITLE | TX_ELLIPSIS, &s.job_name);
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {}", session_kind(s.kind), fmt::when(s.started, &app.zone)));
    ui.box_end();
    let (label, kind) = fmt::session_status(s.status);
    ui.badge(kind, label);
    ui.box_end();
    if s.status == SessionStatus::Running {
        ui.next_size(-1, 0);
        ui.progress(ID_LOG + 1, (s.bytes_read.min(s.bytes_total) * 1000 / s.bytes_total.max(1)) as i32, 1000);
    }
    let dur = if s.finished > 0 { fmt::duration(s.finished - s.started) } else { fmt::duration(epoch - s.started) };
    let files = format!("{} / {}", fmt::count(s.files_done), fmt::count(s.files_total));
    widgets::props(ui, &[
        (t("Dauer"), &dur),
        (t("Dateien"), &files),
        (t("Gelesen"), &fmt::size(s.bytes_read)),
        (t("Neu gespeichert"), &fmt::size(s.bytes_new)),
        (t("Deduplizierung"), &fmt::ratio(s.dedup_x100)),
        (t("Kompression"), &fmt::ratio(s.compress_x100)),
    ]);
    if !s.message.is_empty() {
        widgets::notice(ui, if s.status == SessionStatus::Failed { IB_ERROR } else { IB_WARNING }, "", &s.message);
    }
    ui.text(TS_BODY_STRONG, t("Protokoll"));
    let path = format!("/api/sessions/{}/log", s.id);
    app.net.want(&path, if s.status == SessionStatus::Running { 2000 } else { 60_000 }, now);
    let lines = app.net.data.logs.get(&s.id).cloned().unwrap_or_default();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_LOG + 10, SC_V | BOX_LAYER | pad(10));
    ui.box_begin(0, BOX_V, 4);
    for l in &lines {
        ui.box_begin(0, BOX_H, 8);
        ui.icon(match l.level {
            LogLevel::Error => IC_ERROR,
            LogLevel::Warning => IC_WARNING,
            _ => IC_INFO,
        }, 14);
        ui.next_size(70, 0);
        ui.text(TS_CAPTION | TX_TERTIARY, &datetime::format_time(datetime::Timestamp::from_secs(l.at), &app.zone));
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &l.msg);
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
}

fn operations(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/operations", 2000, now);
    let epoch = app.epoch(ui);
    let list: Vec<Operation> = app.net.data.ops.clone();
    let keys: Vec<u64> = list.iter().map(|o| ids::key(&o.id)).collect();
    app.mon.ops.retain(&keys);
    let sel: Vec<Operation> = list.iter().filter(|o| app.mon.ops.sel.contains(&ids::key(&o.id))).cloned().collect();
    let can_cancel = app.may(Role::Operator) && sel.iter().any(|o| !o.status.finished());
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_STOP);
    ui.next_flags(if can_cancel { 0 } else { ST_DISABLED });
    if ui.button(ID_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
        for o in sel.iter().filter(|o| !o.status.finished()) {
            app.net.action(Method::Post, &format!("/api/operations/{}/cancel", o.id), None, "op.cancel", &["/api/operations"], After::Nothing);
        }
    }
    ui.text(TS_CAPTION | TX_SECONDARY, t("Abbrechen wirkt an der nächsten sicheren Stelle; ein halber Restore-Point entsteht nie."));
    ui.box_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Vorgang"), 260).flags(GC_FILL),
        GridCol::new(t("Benutzer"), 110),
        GridCol::new(t("Status"), 150),
        GridCol::new(t("Fortschritt"), 170),
        GridCol::new(t("Gestartet"), 130),
        GridCol::new(t("Ergebnis"), 220),
    ];
    let (a, b) = ui.grid_begin(ID_OGRID, &mut app.mon.ops, &cols, &keys);
    for i in a..b {
        let o = &list[i as usize];
        let st = &mut app.mon.ops;
        ui.grid_row(st, i);
        ui.grid_text(st, 0, &o.title);
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, &o.owner);
        ui.grid_cell(st, 2);
        let (label, kind) = fmt::op_status(o.status);
        ui.badge(kind, label);
        ui.grid_cell(st, 3);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.progress(ID_OGRID - 0x1000 + i as u32, (o.done.min(o.total) * 1000 / o.total.max(1)) as i32, 1000);
        ui.next_size(40, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_RIGHT, &fmt::percent(o.done, o.total));
        ui.box_end();
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &fmt::when_short(o.started, epoch, &app.zone));
        let res = if !o.error.is_empty() { o.error.clone() } else { o.result.clone() };
        ui.grid_text_ts(st, 5, TS_BODY | TX_SECONDARY, if o.status.finished() { &res } else { "" });
    }
    ui.grid_end(&mut app.mon.ops);
    ui.box_end();
}

fn alerts(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/summary", 5000, now);
    let epoch = app.epoch(ui);
    let alerts = app.net.data.summary.as_ref().map(|s| s.alerts.clone()).unwrap_or_default();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_MON + 50, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 10);
    if alerts.is_empty() {
        ui.next_size(-1, 300);
        ui.empty_state(BC_SHIELD_CHECK, t("Keine Hinweise"), t("Alle Jobs, Hosts und Repositories sind in Ordnung."));
    }
    for (i, a) in alerts.iter().enumerate() {
        let sev = match a.severity {
            Severity::Critical => IB_ERROR,
            Severity::Warning => IB_WARNING,
            Severity::Info => IB_INFO,
        };
        ui.next_size(-1, 0);
        ui.infobar(ID_ALERT + 2 * i as u32, sev, &a.title, &format!("{} · {}", a.detail, fmt::when_short(a.at, epoch, &app.zone)));
    }
    ui.box_end();
    ui.scroll_end();
}

fn audit(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/audit", 10_000, now);
    let list: Vec<AuditEntry> = app.net.data.audit.clone();
    let keys: Vec<u64> = list.iter().map(|a| ids::key(&a.id)).collect();
    app.mon.audit.retain(&keys);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Zeit"), 150),
        GridCol::new(t("Benutzer"), 120),
        GridCol::new(t("Aktion"), 170),
        GridCol::new(t("Ziel"), 200),
        GridCol::new(t("Details"), 260).flags(GC_FILL),
        GridCol::new(t("Ergebnis"), 110),
    ];
    let (a, b) = ui.grid_begin(ID_AGRID, &mut app.mon.audit, &cols, &keys);
    for i in a..b {
        let e = &list[i as usize];
        let st = &mut app.mon.audit;
        ui.grid_row(st, i);
        ui.grid_text_ts(st, 0, TS_BODY | TX_SECONDARY, &fmt::when(e.at, &app.zone));
        ui.grid_text(st, 1, &e.actor);
        ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, &e.action);
        ui.grid_text(st, 3, &e.target);
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &e.detail);
        ui.grid_cell(st, 5);
        ui.badge(if e.ok { BADGE_OK } else { BADGE_CRIT }, if e.ok { t("Erfolgreich") } else { t("Abgelehnt") });
    }
    ui.grid_end(&mut app.mon.audit);
    ui.box_end();
    let _ = tf;
}
