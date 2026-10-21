// fleet -- the other BackupCraft servers this one watches (a tab of Monitoring): their state
// from the last report, a detail card with their alerts; add (the CA is confirmed by its
// fingerprint), check now, edit, remove, connect the console to one

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
use crate::views::dialogs::Confirm;
use crate::{fmt, widgets};

const ID_CB: u32 = ID_MON + 0x100;
const C_ADD: u32 = ID_CB;
const C_CHECK: u32 = ID_CB + 1;
const C_EDIT: u32 = ID_CB + 2;
const C_REMOVE: u32 = ID_CB + 3;
const C_CONNECT: u32 = ID_CB + 4;
const ID_FGRID: u32 = ID_MON + 0xB_0000;

/// the label and badge kind of a watched server's state
pub fn status(s: &str) -> (&'static str, u32) {
    match s {
        "ok" => (t("In Ordnung"), BADGE_OK),
        "warning" => (t("Warnungen"), BADGE_WARN),
        "critical" => (t("Fehler"), BADGE_CRIT),
        "unreachable" => (t("Nicht erreichbar"), BADGE_CRIT),
        "unauthorized" => (t("Token abgelehnt"), BADGE_CRIT),
        "untrusted" => (t("CA geändert"), BADGE_CRIT),
        _ => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

/// "AB:CD:..." of a hex digest
pub fn fingerprint_text(hex: &str) -> String {
    let h = hex.to_uppercase();
    h.as_bytes().chunks(2).map(|c| core::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(":")
}

pub fn tab(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/fleet", 10_000, now);
    let epoch = app.epoch(ui);
    let list: Vec<FleetServer> = app.net.data.fleet.clone();
    let keys: Vec<u64> = list.iter().map(|s| ids::key(&s.id)).collect();
    app.mon.fleet.retain(&keys);
    if app.mon.fleet.sel.is_empty() {
        if let Some(s) = list.first() {
            app.mon.fleet.select_only(Some(ids::key(&s.id)));
        }
    }
    let sel: Option<FleetServer> = list.iter().find(|s| app.mon.fleet.sel.contains(&ids::key(&s.id))).cloned();
    let (admin, oper) = (app.may(Role::Admin), app.may(Role::Operator));
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_ADD, IC_ADD, t("Server hinzufügen …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_CHECK, IC_REFRESH, t("Jetzt abfragen")).enabled(oper && sel.is_some()).flags(CBI_SEP),
        CmdItem::new(C_EDIT, IC_EDIT, t("Bearbeiten …")).enabled(admin && sel.is_some()),
        CmdItem::new(C_CONNECT, IC_OPEN_IN_NEW_WINDOW, t("Konsole verbinden")).enabled(sel.is_some() && !app.fixture),
        CmdItem::new(C_REMOVE, IC_DELETE, t("Entfernen")).enabled(admin && sel.is_some()).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_ADD), _) => app.dlg.open_fleet(None),
        (Some(C_CHECK), Some(s)) => {
            app.net.action(Method::Post, &format!("/api/fleet/servers/{}/check", s.id), None, "fleet.check", &["/api/fleet", "/api/summary"], After::Nothing);
        }
        (Some(C_EDIT), Some(s)) => app.dlg.open_fleet(Some(s)),
        (Some(C_REMOVE), Some(s)) => app.dlg.confirm(Confirm::RemoveFleet(s.id.clone()), &tf("„{}“ nicht mehr überwachen?", &[&s.name]),
                                                    t("Der Server selbst bleibt unverändert; sein Überwachungs-Token lässt sich dort widerrufen."), t("Entfernen")),
        (Some(C_CONNECT), Some(s)) => {
            // the console signs in at that server instead (its CA is confirmed there)
            app.login.server = s.url.clone();
            app.logout(ui, None);
            return;
        }
        _ => {}
    }
    if list.is_empty() && app.net.loaded("/api/fleet") {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
        ui.next_size(-1, -1);
        ui.empty_state(BC_SERVER, t("Keine weiteren Server"),
                       t("Hier erscheinen andere BackupCraft-Server, die dieser überwacht – etwa die Server der Außenstellen. Dort unter Einstellungen › Netzwerk ein Überwachungs-Token erzeugen und hier mit „Server hinzufügen“ eintragen."));
        ui.box_end();
        return;
    }
    // the counts
    let n = |f: &dyn Fn(&FleetServer) -> bool| list.iter().filter(|s| f(s)).count();
    let ok = n(&|s| s.status == "ok");
    let warn = n(&|s| s.status == "warning");
    let bad = list.len() - ok - warn;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.text(TS_BODY_STRONG, &tf("{} Server", &[&format!("{}", list.len())]));
    ui.badge(BADGE_OK, &tf("{} in Ordnung", &[&format!("{}", ok)]));
    if warn > 0 {
        ui.badge(BADGE_WARN, &tf("{} mit Warnungen", &[&format!("{}", warn)]));
    }
    if bad > 0 {
        ui.badge(BADGE_CRIT, &tf("{} mit Fehlern", &[&format!("{}", bad)]));
    }
    ui.box_end();
    let h = (52 + 44 * list.len() as i32).min(330);
    ui.next_size(-1, h);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Server"), 240).flags(GC_FILL).min(180),
        GridCol::new(t("Zustand"), 150),
        GridCol::new(t("Version"), 80),
        GridCol::new(t("Jobs"), 70).flags(GC_RIGHT),
        GridCol::new(t("Letzte 24 h"), 130),
        GridCol::new(t("Speicher"), 150),
        GridCol::new(t("Abgefragt"), 120),
    ];
    let (a, b) = ui.grid_begin(ID_FGRID, &mut app.mon.fleet, &cols, &keys);
    for i in a..b {
        let s = &list[i as usize];
        let r = &s.report;
        let g = &mut app.mon.fleet;
        ui.grid_row(g, i);
        ui.grid_cell(g, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(BC_SERVER, 16);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &s.name);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &s.url);
        ui.box_end();
        ui.box_end();
        ui.grid_cell(g, 1);
        let (label, kind) = status(&s.status);
        ui.badge(kind, label);
        ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, if r.version.is_empty() { "–" } else { &r.version });
        ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &format!("{}", r.jobs));
        ui.grid_cell(g, 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(BADGE_OK);
        ui.text(TS_BODY | TX_SECONDARY, &format!("{}", r.ok_24h));
        ui.status_dot(BADGE_WARN);
        ui.text(TS_BODY | TX_SECONDARY, &format!("{}", r.warn_24h));
        ui.status_dot(BADGE_CRIT);
        ui.text(TS_BODY | TX_SECONDARY, &format!("{}", r.fail_24h));
        ui.box_end();
        ui.grid_cell(g, 5);
        ui.box_begin(0, BOX_V, 2);
        // the disks' free bytes; older servers do not report them (their repositories' bytes then)
        let (filled, label) = if r.storage_free > 0 || r.storage_used >= r.storage_capacity {
            (r.storage_capacity.saturating_sub(r.storage_free), tf("{} frei von {}", &[&fmt::size(r.storage_free), &fmt::size(r.storage_capacity)]))
        } else {
            (r.storage_used, format!("{} / {}", fmt::size(r.storage_used), fmt::size(r.storage_capacity)))
        };
        ui.text(TS_CAPTION | TX_SECONDARY, &label);
        ui.next_size(120, 4);
        ui.meter(ID_FGRID - 0x100 + i as u32, filled as i64, r.storage_capacity.max(1) as i64, fmt::fill_kind(filled, r.storage_capacity));
        ui.box_end();
        ui.grid_text_ts(g, 6, TS_BODY | TX_SECONDARY, &fmt::when_short(s.checked, epoch, &app.zone));
    }
    ui.grid_end(&mut app.mon.fleet);
    ui.box_end();
    let Some(s) = sel else { return };
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_MON + 0x180, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 12);
    widgets::card_begin(ui, &s.name, &s.message);
    let r = &s.report;
    let ca = if s.fingerprint.is_empty() { String::from(t("keine (http auf diesem Rechner)")) } else { fingerprint_text(&s.fingerprint) };
    let stale = s.seen > 0 && s.seen < s.checked;
    let seen = if stale { tf("{} (älter als die letzte Abfrage)", &[&fmt::when(s.seen, &app.zone)]) } else { fmt::when(s.seen, &app.zone) };
    widgets::props(ui, &[
        (t("Adresse"), &s.url),
        (t("Servername"), if r.server_name.is_empty() { "–" } else { &r.server_name }),
        (t("Letzter Bericht"), &seen),
        (t("Hosts online"), &format!("{} / {}", r.hosts_online, r.hosts)),
        (t("Restore-Points"), &fmt::count(r.points as u64)),
        (t("Läuft gerade"), &format!("{}", r.running)),
        (t("Zertifizierungsstelle"), &ca),
    ]);
    if !r.alerts.is_empty() {
        ui.separator();
        ui.text(TS_BODY_STRONG, t("Hinweise des Servers"));
        for a in r.alerts.iter().take(8) {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            ui.status_dot(fmt::severity(a.severity));
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &a.title);
            if !a.detail.is_empty() {
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &a.detail);
            }
            ui.box_end();
            ui.box_end();
        }
    }
    widgets::card_end(ui);
    ui.box_end();
    ui.scroll_end();
}
