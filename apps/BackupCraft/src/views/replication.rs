// replication -- replicas (a job's newest restore point kept ready in a folder of a host:
// sync, failover, failback, undo), failover plans (replicas switched over in order) and the
// jobs with continuous data protection (CDP)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_REPL};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub tab: i32,
    pub replicas: GridState,
    pub plans: GridState,
    pub cdp: GridState,
}

const ID_TABS: u32 = ID_REPL + 1; // its tabs: + 2 .. + 4
const ID_INFO: u32 = ID_REPL + 6; // its close button: + 7
const ID_CB: u32 = ID_REPL + 10;
const C_NEW: u32 = ID_CB;
const C_SYNC: u32 = ID_CB + 1;
const C_FAILOVER: u32 = ID_CB + 2;
const C_FAILBACK: u32 = ID_CB + 3;
const C_UNDO: u32 = ID_CB + 4;
const C_EDIT: u32 = ID_CB + 5;
const C_DELETE: u32 = ID_CB + 6;
const C_PNEW: u32 = ID_CB + 10;
const C_PEDIT: u32 = ID_CB + 11;
const C_PRUN: u32 = ID_CB + 12;
const C_PDELETE: u32 = ID_CB + 13;
const C_JOB: u32 = ID_CB + 20;
const ID_RGRID: u32 = ID_REPL + 0x2_0000;
const ID_PGRID: u32 = ID_REPL + 0x5_0000;
const ID_CGRID: u32 = ID_REPL + 0x8_0000;

/// the label and badge kind of a replica state
pub fn replica_state(s: ReplicaState) -> (&'static str, u32) {
    match s {
        ReplicaState::New => (t("Noch nicht abgeglichen"), BADGE_NEUTRAL),
        ReplicaState::Ready => (t("Bereit"), BADGE_OK),
        ReplicaState::Syncing => (t("Wird abgeglichen"), BADGE_INFO),
        ReplicaState::Failed => (t("Fehler"), BADGE_CRIT),
        ReplicaState::Failover => (t("Failover aktiv"), BADGE_WARN),
        ReplicaState::Failback => (t("Failback läuft"), BADGE_INFO),
    }
}

fn cdp_state(s: &str) -> (&'static str, u32) {
    match s {
        "watching" => (t("Überwacht"), BADGE_OK),
        "backing-up" => (t("Sichert"), BADGE_INFO),
        "starting" => (t("Startet"), BADGE_NEUTRAL),
        "error" => (t("Fehler"), BADGE_CRIT),
        _ => (t("Angehalten"), BADGE_NEUTRAL),
    }
}

/// "15 s", "5 min", "1 h"
pub fn rpo_text(secs: u32) -> String {
    if secs % 3600 == 0 {
        format!("{} h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{} min", secs / 60)
    } else {
        format!("{} s", secs)
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/capabilities", 60_000, now);
    let d = &app.net.data;
    let caps_known = app.net.loaded("/api/capabilities");
    if caps_known && !d.capable(api::CAP_REPLICATION) && !d.capable(api::CAP_CDP) {
        // an older server: the page as announced
        views::staged::frame(app, ui, views::staged::Kind::Replication);
        return;
    }
    views::body_begin(ui);
    widgets::page_head(ui, t("Replikation"), t("Startbereite Kopien auf einem Zielhost, Failover und Failback – und Jobs, die Änderungen laufend sichern."));
    widgets::page_head_end(ui);
    ui.tabs(ID_TABS, &mut app.repl.tab, &[t("Replikate"), t("Failover-Pläne"), t("Datei-CDP")]);
    match app.repl.tab {
        0 => replicas(app, ui),
        1 => plans(app, ui),
        _ => cdp(app, ui),
    }
    views::body_end(ui);
}

fn replicas(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/replicas", 4000, now);
    let epoch = app.epoch(ui);
    let list: Vec<Replica> = app.net.data.replicas.clone();
    let keys: Vec<u64> = list.iter().map(|r| ids::key(&r.id)).collect();
    app.repl.replicas.retain(&keys);
    let sel: Option<Replica> = list.iter().find(|r| app.repl.replicas.sel.contains(&ids::key(&r.id))).cloned();
    let (admin, oper) = (app.may(Role::Admin), app.may(Role::Operator));
    let st = sel.as_ref().map(|r| r.state);
    let clicked = ui.command_bar(ID_CB + 30, &[
        CmdItem::new(C_NEW, IC_ADD, t("Replikat anlegen …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_EDIT, IC_EDIT, t("Bearbeiten …")).enabled(admin && sel.is_some()),
        CmdItem::new(C_SYNC, IC_REFRESH, t("Abgleichen")).enabled(oper && matches!(st, Some(ReplicaState::New | ReplicaState::Ready | ReplicaState::Failed))).flags(CBI_SEP),
        CmdItem::new(C_FAILOVER, IC_PLAY, t("Failover")).enabled(oper && st == Some(ReplicaState::Ready)),
        CmdItem::new(C_FAILBACK, IC_BACK, t("Failback")).enabled(oper && st == Some(ReplicaState::Failover)),
        CmdItem::new(C_UNDO, IC_UNDO, t("Failover verwerfen")).enabled(oper && st == Some(ReplicaState::Failover)),
        CmdItem::new(C_DELETE, IC_DELETE, t("Austragen")).enabled(admin && sel.is_some()).flags(CBI_SEP),
    ]);
    if let Some(r) = &sel {
        let rid = r.id.clone();
        match clicked {
            Some(C_EDIT) => app.dlg.open_replica(Some(r)),
            Some(C_SYNC) => {
                app.net.action(Method::Post, &format!("/api/replicas/{}/sync", rid), None, "replica.sync", &["/api/replicas"], After::Operation);
            }
            Some(C_FAILOVER) => app.dlg.confirm(Confirm::Failover(rid), &tf("Failover auf „{}“?", &[&r.name]),
                                                &tf("Ab jetzt gilt {} auf {} als die Arbeitskopie. Das Replikat wird nicht mehr abgeglichen, bis Failback oder Verwerfen es zurückholt.", &[&r.target, &r.host_name]),
                                                t("Failover")),
            Some(C_FAILBACK) => app.dlg.confirm(Confirm::Failback(rid), &tf("Failback von „{}“?", &[&r.name]),
                                                t("Die Ordner des Replikats werden als Restore-Point des Jobs gesichert und dann an die ursprünglichen Orte zurückgeschrieben. Geänderte Dateien werden ersetzt; am Original wird nichts gelöscht."),
                                                t("Failback")),
            Some(C_UNDO) => app.dlg.confirm(Confirm::UndoFailover(rid), &tf("Failover von „{}“ verwerfen?", &[&r.name]),
                                            t("Änderungen, die während des Failovers am Replikat gemacht wurden, gehen verloren: das Replikat wird wieder auf seinen Restore-Point gebracht."),
                                            t("Verwerfen")),
            Some(C_DELETE) => app.dlg.confirm(Confirm::DeleteReplica(rid), &tf("Replikat „{}“ austragen?", &[&r.name]),
                                              t("BackupCraft gleicht den Ordner nicht mehr ab. Die Dateien im Zielordner bleiben erhalten."), t("Austragen")),
            _ => {}
        }
    }
    if clicked == Some(C_NEW) {
        app.dlg.open_replica(None);
    }
    if let Some(f) = list.iter().find(|r| r.state == ReplicaState::Failover) {
        let more = list.iter().filter(|r| r.state == ReplicaState::Failover).count();
        let title = if more > 1 { tf("{} Replikate im Failover", &[&format!("{}", more)]) } else { tf("Failover aktiv: {}", &[&f.name]) };
        ui.infobar(ID_INFO, IB_WARNING, &title, t("Änderungen am Replikat bringt „Failback“ zurück auf das Original; „Failover verwerfen“ setzt das Replikat zurück."));
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/replicas") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_REPLICATE, t("Noch keine Replikate"),
                       t("Ein Replikat hält den neuesten Restore-Point eines Jobs startbereit in einem Ordner eines anderen Hosts – für den Fall, dass das Original ausfällt."));
    } else {
        let cols = [
            GridCol::new(t("Replikat"), 200).flags(GC_FILL).min(150),
            GridCol::new(t("Job"), 180),
            GridCol::new(t("Zielhost"), 150),
            GridCol::new(t("Ordner"), 170),
            GridCol::new(t("Stand"), 135),
            GridCol::new(t("Zustand"), 170),
            GridCol::new(t("Abgeglichen"), 120),
        ];
        let (a, b) = ui.grid_begin(ID_RGRID, &mut app.repl.replicas, &cols, &keys);
        for i in a..b {
            let r = &list[i as usize];
            let g = &mut app.repl.replicas;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_REPLICATE, 16);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &r.name);
            if r.auto {
                ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, t("folgt jedem neuen Restore-Point"));
            }
            ui.box_end();
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &r.job_name);
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &r.host_name);
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &r.target);
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &if r.point_time > 0 { fmt::when_short(r.point_time, epoch, &app.zone) } else { String::from("–") });
            ui.grid_cell(g, 5);
            let (label, kind) = replica_state(r.state);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.badge(kind, label);
            if !r.message.is_empty() && matches!(r.state, ReplicaState::Failed | ReplicaState::Failover) {
                ui.icon(IC_INFO, 14);
                ui.tooltip(&r.message);
            }
            ui.box_end();
            ui.grid_text_ts(g, 6, TS_BODY | TX_SECONDARY, &fmt::when_short(r.synced, epoch, &app.zone));
        }
        ui.grid_end(&mut app.repl.replicas);
    }
    ui.box_end();
}

fn plans(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/failover-plans", 5000, now);
    app.net.want("/api/replicas", 10_000, now);
    let epoch = app.epoch(ui);
    let list: Vec<FailoverPlan> = app.net.data.plans.clone();
    let reps: Vec<Replica> = app.net.data.replicas.clone();
    let keys: Vec<u64> = list.iter().map(|p| ids::key(&p.id)).collect();
    app.repl.plans.retain(&keys);
    let sel: Option<FailoverPlan> = list.iter().find(|p| app.repl.plans.sel.contains(&ids::key(&p.id))).cloned();
    let (admin, oper) = (app.may(Role::Admin), app.may(Role::Operator));
    let clicked = ui.command_bar(ID_CB + 31, &[
        CmdItem::new(C_PNEW, IC_ADD, t("Failover-Plan anlegen …")).flags(ST_ACCENT).enabled(admin && !reps.is_empty()),
        CmdItem::new(C_PEDIT, IC_EDIT, t("Bearbeiten …")).enabled(admin && sel.is_some()),
        CmdItem::new(C_PRUN, IC_PLAY, t("Plan ausführen")).enabled(oper && sel.is_some()).flags(CBI_SEP),
        CmdItem::new(C_PDELETE, IC_DELETE, t("Löschen")).enabled(admin && sel.is_some()).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_PNEW), _) => app.dlg.open_plan(None),
        (Some(C_PEDIT), Some(p)) => app.dlg.open_plan(Some(p)),
        (Some(C_PRUN), Some(p)) => {
            let names: Vec<String> = p.replicas.iter().filter_map(|id| reps.iter().find(|r| r.id == *id)).map(|r| r.name.clone()).collect();
            app.dlg.confirm(Confirm::RunPlan(p.id.clone()), &tf("Failover-Plan „{}“ ausführen?", &[&p.name]),
                            &tf("Nacheinander auf ihre Replikate umgeschaltet werden: {}. Pause dazwischen: {} s.", &[&names.join(", "), &format!("{}", p.delay_s)]),
                            t("Ausführen"));
        }
        (Some(C_PDELETE), Some(p)) => app.dlg.confirm(Confirm::DeletePlan(p.id.clone()), &tf("Failover-Plan „{}“ löschen?", &[&p.name]),
                                                     t("Die Replikate bleiben unverändert."), t("Löschen")),
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/failover-plans") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_REPLICATE, t("Noch keine Failover-Pläne"),
                       t("Ein Plan schaltet mehrere Replikate in fester Reihenfolge um, mit einer Pause dazwischen – etwa erst die Datenbank, dann die Anwendung."));
    } else {
        let cols = [
            GridCol::new(t("Plan"), 200).flags(GC_FILL).min(150),
            GridCol::new(t("Reihenfolge"), 320),
            GridCol::new(t("Pause"), 80).flags(GC_RIGHT),
            GridCol::new(t("Zuletzt ausgeführt"), 140),
            GridCol::new(t("Ergebnis"), 200),
        ];
        let (a, b) = ui.grid_begin(ID_PGRID, &mut app.repl.plans, &cols, &keys);
        for i in a..b {
            let p = &list[i as usize];
            let g = &mut app.repl.plans;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(IC_BULLETED_LIST, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &p.name);
            ui.box_end();
            let order: Vec<String> = p.replicas.iter().enumerate().map(|(k, id)| {
                format!("{}. {}", k + 1, reps.iter().find(|r| r.id == *id).map_or(t("(fehlt)"), |r| r.name.as_str()))
            }).collect();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &order.join("  "));
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &format!("{} s", p.delay_s));
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &fmt::when_short(p.last_run, epoch, &app.zone));
            ui.grid_cell(g, 4);
            match p.last_status.as_str() {
                "" => ui.text(TS_BODY | TX_TERTIARY, "–"),
                s => {
                    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
                    if s == "success" {
                        ui.badge(BADGE_OK, t("Umgeschaltet"));
                    } else {
                        ui.badge(BADGE_CRIT, t("Fehler"));
                    }
                    if !p.message.is_empty() {
                        ui.next_size(-1, 0);
                        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &p.message);
                    }
                    ui.box_end();
                }
            }
        }
        ui.grid_end(&mut app.repl.plans);
    }
    ui.box_end();
}

fn cdp(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/cdp", 3000, now);
    let epoch = app.epoch(ui);
    let list: Vec<CdpStatus> = app.net.data.cdp.clone();
    let keys: Vec<u64> = list.iter().map(|c| ids::key(&c.job_id)).collect();
    app.repl.cdp.retain(&keys);
    let sel: Option<CdpStatus> = list.iter().find(|c| app.repl.cdp.sel.contains(&ids::key(&c.job_id))).cloned();
    let oper = app.may(Role::Operator);
    let clicked = ui.command_bar(ID_CB + 32, &[CmdItem::new(C_JOB, IC_EDIT, t("Job bearbeiten …")).enabled(oper && sel.is_some())]);
    if clicked == Some(C_JOB) {
        if let Some(job) = sel.as_ref().and_then(|c| app.net.data.job(&c.job_id)).cloned() {
            crate::wizards::open_job_edit(app, ui, &job);
        }
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/cdp") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_ACTIVITY, t("Kein Job sichert laufend"),
                       t("Im Job-Assistenten unter „Zeitplan“ lässt sich „Laufend sichern (CDP)“ einschalten: Änderungen werden dann innerhalb des RPO zu einem Restore-Point."));
    } else {
        let cols = [
            GridCol::new(t("Job"), 200).flags(GC_FILL).min(150),
            GridCol::new(t("Host"), 150),
            GridCol::new(t("RPO"), 70).flags(GC_RIGHT),
            GridCol::new(t("Punkte"), 110).flags(GC_RIGHT),
            GridCol::new(t("Offen"), 70).flags(GC_RIGHT),
            GridCol::new(t("Letzte Änderung"), 130),
            GridCol::new(t("Letzter Punkt"), 130),
            GridCol::new(t("Zustand"), 140),
        ];
        let (a, b) = ui.grid_begin(ID_CGRID, &mut app.repl.cdp, &cols, &keys);
        for i in a..b {
            let c = &list[i as usize];
            let g = &mut app.repl.cdp;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_ACTIVITY, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &c.job_name);
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, if c.host_name.is_empty() { "–" } else { &c.host_name });
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &rpo_text(c.rpo));
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &tf("{} in {} h", &[&format!("{}", c.points), &format!("{}", c.keep_hours)]));
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &fmt::count(c.pending));
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &fmt::when_short(c.last_change, epoch, &app.zone));
            ui.grid_text_ts(g, 6, TS_BODY | TX_SECONDARY, &fmt::when_short(c.last_point, epoch, &app.zone));
            ui.grid_cell(g, 7);
            let (label, kind) = cdp_state(&c.state);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.badge(kind, label);
            if !c.message.is_empty() {
                ui.icon(IC_INFO, 14);
                ui.tooltip(&c.message);
            }
            ui.box_end();
        }
        ui.grid_end(&mut app.repl.cdp);
    }
    ui.box_end();
    widgets::hint(ui, t("Server: Dateisystem-Ereignisse des Betriebssystems. Agenten: der Agent meldet Änderungen in seinen freigegebenen Ordnern. Unveränderte Ordner übernimmt ein CDP-Punkt aus dem vorigen."));
}
