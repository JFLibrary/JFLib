// repos -- repositories as cards: fill level, savings by dedupe and compression, points,
// encryption, immutability; add, check, clean up, change immutability, detach

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_REPOS};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub sel: String,
}

const ID_CB: u32 = ID_REPOS + 10;
const C_NEW: u32 = ID_CB;
const C_CHECK: u32 = ID_CB + 1;
const C_GC: u32 = ID_CB + 2;
const C_IMM: u32 = ID_CB + 3;
const C_DETACH: u32 = ID_CB + 4;
const C_REFRESH: u32 = ID_CB + 5;
const ID_CARD: u32 = ID_REPOS + 100;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/repositories", 6000, now);
    app.net.want("/api/jobs", 15_000, now);
    let repos: Vec<Repository> = app.net.data.repos.clone();
    if !repos.iter().any(|r| r.id == app.repos.sel) {
        app.repos.sel = repos.first().map_or(String::new(), |r| r.id.clone());
    }
    let sel = repos.iter().find(|r| r.id == app.repos.sel).cloned();
    views::scroll_body_begin(ui, ID_REPOS + 1);
    let used: u64 = repos.iter().map(|r| r.used).sum();
    let (_, free) = bc_proto::model::disks(&repos);
    widgets::page_head(ui, t("Repositories"), &tf("{} Repositories · {} belegt · {} frei", &[&format!("{}", repos.len()), &fmt::size(used), &fmt::size(free)]));
    widgets::page_head_end(ui);
    let admin = app.may(Role::Admin);
    let jobs_using = sel.as_ref().map_or(0, |r| app.net.data.jobs.iter().filter(|j| j.repository_id == r.id).count());
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_NEW, IC_ADD, t("Repository hinzufügen …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_CHECK, BC_SHIELD_CHECK, t("Prüfen")).enabled(app.may(Role::Operator) && sel.is_some()).flags(CBI_SEP),
        CmdItem::new(C_GC, IC_CLEAR, t("Aufräumen")).enabled(admin && sel.is_some()),
        CmdItem::new(C_IMM, IC_LOCK, t("Unveränderlichkeit …")).enabled(admin && sel.is_some()),
        CmdItem::new(C_DETACH, IC_REMOVE_CIRCLE, t("Austragen")).enabled(admin && sel.is_some() && jobs_using == 0),
        CmdItem::new(C_REFRESH, IC_REFRESH, t("Aktualisieren")).flags(CBI_END),
    ]);
    match (clicked, &sel) {
        (Some(C_NEW), _) => app.dlg.open_repository(),
        (Some(C_CHECK), Some(r)) => {
            app.net.action(Method::Post, &format!("/api/repositories/{}/check", r.id), None, "repository.check", &[], After::Operation);
        }
        (Some(C_GC), Some(r)) => {
            app.dlg.confirm(Confirm::Gc(r.id.clone()), t("Repository aufräumen?"),
                            &tf("Daten, die kein Restore-Point von „{}“ mehr braucht, werden freigegeben. Das kann bei großen Repositories dauern; Sicherungen warten so lange.", &[&r.name]),
                            t("Aufräumen"));
        }
        (Some(C_IMM), Some(r)) => app.dlg.open_immutability(r),
        (Some(C_DETACH), Some(r)) => {
            app.dlg.confirm(Confirm::DetachRepo(r.id.clone()), t("Repository austragen?"),
                            &tf("„{}“ wird aus BackupCraft entfernt. Die Daten im Ordner bleiben erhalten und lassen sich später wieder einbinden.", &[&r.name]),
                            t("Austragen"));
        }
        (Some(C_REFRESH), _) => app.refresh(ui),
        _ => {}
    }
    if repos.is_empty() && app.net.loaded("/api/repositories") {
        ui.next_size(-1, 360);
        ui.empty_state(BC_REPOSITORY, t("Noch kein Repository"), t("Ein Repository ist ein Ordner auf dem Server (lokale Platte, NAS, eingebundene Freigabe), in dem BackupCraft dedupliziert und verschlüsselt speichert."));
    }
    // cards, three per row
    let mut i = 0;
    while i < repos.len() {
        ui.box_begin(0, BOX_H, 16);
        for k in 0..3 {
            ui.next_size(-1, 0);
            if let Some(r) = repos.get(i + k) {
                card(app, ui, r, ID_CARD + (i + k) as u32 * 16);
            } else {
                ui.box_begin(0, BOX_V, 0);
                ui.box_end();
            }
        }
        ui.box_end();
        i += 3;
    }
    views::scroll_body_end(ui);
}

fn card(app: &mut Console, ui: &mut Ui, r: &Repository, id: u32) {
    let selected = r.id == app.repos.sel;
    ui.box_begin(0, BOX_V | BOX_CARD | pad(18), 12);
    let rect = ui.box_rect();
    let ev = ui.hit_rect(id, rect[0] - ui.dp(18), rect[1] - ui.dp(18), rect[2] + ui.dp(36), rect[3] + ui.dp(36), 0);
    if ev & EV_ACT != 0 {
        app.repos.sel = r.id.clone();
    }
    if selected {
        let c = ui.color(T_ACCENT);
        let d = ui.dp(18);
        ui.cmd_rrect(rect[0] - d, rect[1] - d, rect[2] + d * 2, rect[3] + d * 2, ui.dp(8), ui.dp(2), 0, c, c);
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.icon(BC_REPOSITORY, 22);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 0);
    ui.text(TS_BODY_STRONG | TX_ELLIPSIS, &r.name);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &r.path);
    ui.box_end();
    let (label, kind) = fmt::repo_status(r.status);
    ui.badge(kind, label);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XEND, 8);
    // the disk: other data on it takes room too
    if r.capacity > 0 && r.status != RepoStatus::Offline {
        ui.text(TS_TITLE, &fmt::size(r.free));
        ui.text(TS_BODY | TX_SECONDARY, &tf("frei von {} · Platte {} belegt", &[&fmt::size(r.capacity), &fmt::percent(r.disk_used(), r.capacity)]));
    } else {
        ui.text(TS_TITLE, "–");
        ui.text(TS_BODY | TX_SECONDARY, t("freier Platz unbekannt"));
    }
    ui.box_end();
    ui.next_size(-1, 8);
    ui.meter(id + 1, r.disk_used() as i64, r.capacity as i64, fmt::fill_kind(r.disk_used(), r.capacity));
    let savings = if r.used > 0 { fmt::ratio((r.logical as u128 * 100 / r.used as u128) as u32) } else { String::from("–") };
    let imm = if r.immutable_days > 0 { tf("{} Tage", &[&format!("{}", r.immutable_days)]) } else { String::from(t("Aus")) };
    let jobs = app.net.data.jobs.iter().filter(|j| j.repository_id == r.id).count();
    widgets::props(ui, &[
        (t("Im Repository"), &fmt::size(r.used)),
        (t("Restore-Points"), &format!("{}", r.points)),
        (t("Gesicherte Daten"), &fmt::size(r.logical)),
        (t("Ersparnis"), &savings),
        (t("Verschlüsselt"), if r.encrypted { t("Ja") } else { t("Nein") }),
        (t("Unveränderlich"), &imm),
        (t("Jobs"), &format!("{}", jobs)),
    ]);
    if !r.message.is_empty() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.icon(IC_WARNING, 14);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, &r.message);
        ui.box_end();
    }
    ui.box_end();
}
