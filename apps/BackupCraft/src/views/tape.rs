// tape -- the tape library: tapes (add, to a pool, erase, remove, read the library), media
// pools, the restore points on tape (sets: restore, verify); the page as announced when the
// server has no tape support

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_TAPE};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub tab: i32,
    pub tapes: GridState,
    pub pools: GridState,
    pub sets: GridState,
}

const ID_TABS: u32 = ID_TAPE + 1;
const ID_CB: u32 = ID_TAPE + 10;
const C_ADD: u32 = ID_CB;
const C_ASSIGN: u32 = ID_CB + 1;
const C_ERASE: u32 = ID_CB + 2;
const C_REMOVE: u32 = ID_CB + 3;
const C_RESCAN: u32 = ID_CB + 4;
const C_PNEW: u32 = ID_CB + 10;
const C_PEDIT: u32 = ID_CB + 11;
const C_PDELETE: u32 = ID_CB + 12;
const C_RESTORE: u32 = ID_CB + 20;
const C_VERIFY: u32 = ID_CB + 21;
const ID_TGRID: u32 = ID_TAPE + 0x2_0000;
const ID_PGRID: u32 = ID_TAPE + 0x5_0000;
const ID_SGRID: u32 = ID_TAPE + 0x8_0000;

/// the label and badge kind of a tape's state
pub fn tape_state(s: &str) -> (&'static str, u32) {
    match s {
        "blank" => (t("Leer"), BADGE_NEUTRAL),
        "active" => (t("Beschrieben"), BADGE_INFO),
        "full" => (t("Voll"), BADGE_OK),
        "missing" => (t("Nicht in der Bibliothek"), BADGE_WARN),
        _ => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

fn set_state(s: &str) -> (&'static str, u32) {
    match s {
        "ok" => (t("Vollständig"), BADGE_OK),
        "offline" => (t("Band fehlt"), BADGE_WARN),
        "incomplete" => (t("Unvollständig"), BADGE_CRIT),
        "locked" => (t("Schlüssel fehlt"), BADGE_WARN),
        _ => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/capabilities", 60_000, now);
    if app.net.loaded("/api/capabilities") && !app.net.data.capable(api::CAP_TAPE) {
        views::staged::frame(app, ui, views::staged::Kind::Tape);
        return;
    }
    app.net.want("/api/tape", 10_000, now);
    views::body_begin(ui);
    widgets::page_head(ui, t("Tape"), t("Restore-Points vollständig auf Band – lesbar ohne Repository, auch auf einem anderen Server."));
    widgets::page_head_end(ui);
    if let Some(st) = app.net.data.tape.clone() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.icon(BC_TAPE, 16);
        ui.text(TS_BODY_STRONG, &tf("{} Bänder", &[&format!("{}", st.tapes)]));
        ui.text(TS_BODY | TX_SECONDARY, &tf("{} leer · {} von {} belegt · {} Sätze", &[&format!("{}", st.blank), &fmt::size(st.used), &fmt::size(st.capacity), &format!("{}", st.sets)]));
        if st.missing > 0 {
            ui.badge(BADGE_WARN, &tf("{} nicht in der Bibliothek", &[&format!("{}", st.missing)]));
        }
        ui.spacer(-1);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &tf("Virtuelle Bibliothek: {}", &[&st.library]));
        ui.box_end();
        if !st.ok {
            widgets::notice(ui, IB_ERROR, t("Bibliothek nicht erreichbar"), &st.message);
        }
    }
    ui.tabs(ID_TABS, &mut app.tape.tab, &[t("Bänder"), t("Medienpools"), t("Auf Band")]);
    match app.tape.tab {
        0 => tapes(app, ui),
        1 => pools(app, ui),
        _ => sets(app, ui),
    }
    views::body_end(ui);
}

fn tapes(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/tape/tapes", 5000, now);
    app.net.want("/api/tape/pools", 15_000, now);
    let epoch = app.epoch(ui);
    let list: Vec<Cartridge> = app.net.data.tapes.clone();
    let keys: Vec<u64> = list.iter().map(|c| ids::key(&c.id)).collect();
    app.tape.tapes.retain(&keys);
    let sel: Option<Cartridge> = list.iter().find(|c| app.tape.tapes.sel.contains(&ids::key(&c.id))).cloned();
    let admin = app.may(Role::Admin);
    let protected = sel.as_ref().is_some_and(|c| c.protected_until > epoch);
    let clicked = ui.command_bar(ID_CB + 30, &[
        CmdItem::new(C_ADD, IC_ADD, t("Bänder hinzufügen …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_ASSIGN, BC_ARCHIVE, t("Pool zuweisen …")).enabled(admin && sel.as_ref().is_some_and(|c| c.state == "blank")).flags(CBI_SEP),
        CmdItem::new(C_ERASE, IC_CLEAR, t("Band löschen …")).enabled(admin && sel.as_ref().is_some_and(|c| c.state != "blank" && c.state != "missing") && !protected),
        CmdItem::new(C_REMOVE, IC_DELETE, t("Entfernen")).enabled(admin && sel.is_some() && !protected),
        CmdItem::new(C_RESCAN, IC_REFRESH, t("Bibliothek einlesen")).enabled(admin).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_ADD), _) => app.dlg.open_tapes_add(),
        (Some(C_ASSIGN), Some(c)) => app.dlg.open_tape_assign(c),
        (Some(C_ERASE), Some(c)) => app.dlg.confirm(Confirm::EraseTape(c.id.clone()), &tf("Band {} löschen?", &[&c.barcode]),
                                                   &tf("Die {} Sätze darauf gehen verloren – auch Sätze, die auf anderen Bändern weitergehen. Das Band bleibt im Pool und wird wieder beschrieben.", &[&format!("{}", c.sets)]),
                                                   t("Löschen")),
        (Some(C_REMOVE), Some(c)) => app.dlg.confirm(Confirm::RemoveTape(c.id.clone()), &tf("Band {} entfernen?", &[&c.barcode]),
                                                    t("Die Datei des virtuellen Bandes wird gelöscht, seine Sätze fallen aus dem Katalog."), t("Entfernen")),
        (Some(C_RESCAN), _) => {
            app.net.action(Method::Post, "/api/tape/rescan", None, "tape.rescan", &["/api/tape/tapes", "/api/tape/sets", "/api/tape"], After::Operation);
        }
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/tape/tapes") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_TAPE, t("Noch keine Bänder"),
                       t("Die virtuelle Bandbibliothek legt jedes Band als Datei an. „Bänder hinzufügen“ erzeugt leere Bänder; mit „Bibliothek einlesen“ kommen Bänder von anderswo dazu."));
    } else {
        let cols = [
            GridCol::new(t("Barcode"), 150).flags(GC_FILL).min(130),
            GridCol::new(t("Pool"), 150),
            GridCol::new(t("Zustand"), 200),
            GridCol::new(t("Belegt"), 160),
            GridCol::new(t("Sätze"), 60).flags(GC_RIGHT),
            GridCol::new(t("Schutz bis"), 140),
            GridCol::new(t("Geschrieben"), 120),
        ];
        let (a, b) = ui.grid_begin(ID_TGRID, &mut app.tape.tapes, &cols, &keys);
        for i in a..b {
            let c = &list[i as usize];
            let g = &mut app.tape.tapes;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_TAPE, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &c.barcode);
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, if c.pool_name.is_empty() { t("frei") } else { &c.pool_name });
            ui.grid_cell(g, 2);
            let (label, kind) = tape_state(&c.state);
            ui.badge(kind, label);
            ui.grid_cell(g, 3);
            ui.box_begin(0, BOX_V, 2);
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} / {}", fmt::size(c.used), fmt::size(c.capacity)));
            ui.next_size(140, 4);
            ui.meter(ID_TGRID - 0x100 + i as u32, c.used as i64, c.capacity.max(1) as i64, fmt::fill_kind(c.used, c.capacity));
            ui.box_end();
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &format!("{}", c.sets));
            let prot = if c.protected_until > epoch { fmt::when(c.protected_until, &app.zone) } else { String::from("–") };
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &prot);
            ui.grid_text_ts(g, 6, TS_BODY | TX_SECONDARY, &fmt::when_short(c.written, epoch, &app.zone));
        }
        ui.grid_end(&mut app.tape.tapes);
    }
    ui.box_end();
}

fn pools(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/tape/pools", 5000, now);
    let list: Vec<MediaPool> = app.net.data.pools.clone();
    let keys: Vec<u64> = list.iter().map(|p| ids::key(&p.id)).collect();
    app.tape.pools.retain(&keys);
    let sel: Option<MediaPool> = list.iter().find(|p| app.tape.pools.sel.contains(&ids::key(&p.id))).cloned();
    let admin = app.may(Role::Admin);
    let clicked = ui.command_bar(ID_CB + 31, &[
        CmdItem::new(C_PNEW, IC_ADD, t("Pool anlegen …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_PEDIT, IC_EDIT, t("Bearbeiten …")).enabled(admin && sel.is_some()),
        CmdItem::new(C_PDELETE, IC_DELETE, t("Löschen")).enabled(admin && sel.is_some()).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_PNEW), _) => app.dlg.open_pool(None),
        (Some(C_PEDIT), Some(p)) => app.dlg.open_pool(Some(p)),
        (Some(C_PDELETE), Some(p)) => app.dlg.confirm(Confirm::DeletePool(p.id.clone()), &tf("Medienpool „{}“ löschen?", &[&p.name]),
                                                     t("Leere Bänder des Pools werden frei. Ein Pool mit beschriebenen Bändern bleibt."), t("Löschen")),
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/tape/pools") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_ARCHIVE, t("Noch keine Medienpools"),
                       t("Ein Medienpool fasst Bänder mit einem Zweck zusammen – etwa „Wöchentlich“ mit vier Wochen Schutz oder „Tresor“ verschlüsselt für die Auslagerung."));
    } else {
        let cols = [
            GridCol::new(t("Pool"), 220).flags(GC_FILL).min(160),
            GridCol::new(t("Bänder"), 70).flags(GC_RIGHT),
            GridCol::new(t("Belegt"), 170),
            GridCol::new(t("Schutz"), 110),
            GridCol::new(t("Verschlüsselt"), 110),
            GridCol::new(t("Schreiben"), 170),
        ];
        let (a, b) = ui.grid_begin(ID_PGRID, &mut app.tape.pools, &cols, &keys);
        for i in a..b {
            let p = &list[i as usize];
            let g = &mut app.tape.pools;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(BC_ARCHIVE, 16);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &p.name);
            if !p.description.is_empty() {
                ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &p.description);
            }
            ui.box_end();
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &format!("{}", p.tapes));
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &format!("{} / {}", fmt::size(p.used), fmt::size(p.capacity)));
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &if p.protect_days == 0 { String::from(t("keiner")) } else { tf("{} Tage", &[&format!("{}", p.protect_days)]) });
            ui.grid_cell(g, 4);
            if p.encrypted {
                ui.badge(BADGE_OK, t("Ja"));
            } else {
                ui.text(TS_BODY | TX_SECONDARY, t("Nein"));
            }
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, if p.append { t("anhängen, bis voll") } else { t("je Lauf ein leeres Band") });
        }
        ui.grid_end(&mut app.tape.pools);
    }
    ui.box_end();
}

fn sets(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/tape/sets", 5000, now);
    let epoch = app.epoch(ui);
    let list: Vec<TapeSet> = app.net.data.tape_sets.clone();
    let keys: Vec<u64> = list.iter().map(|s| ids::key(&s.id)).collect();
    app.tape.sets.retain(&keys);
    let sel: Option<TapeSet> = list.iter().find(|s| app.tape.sets.sel.contains(&ids::key(&s.id))).cloned();
    let usable = sel.as_ref().is_some_and(|s| s.status == "ok");
    let clicked = ui.command_bar(ID_CB + 32, &[
        CmdItem::new(C_RESTORE, BC_RESTORE, t("Vom Band wiederherstellen …")).flags(ST_ACCENT).enabled(app.may(Role::Restore) && usable),
        CmdItem::new(C_VERIFY, BC_SHIELD_CHECK, t("Prüfen")).enabled(app.may(Role::Operator) && usable).flags(CBI_SEP),
    ]);
    match (clicked, &sel) {
        (Some(C_RESTORE), Some(s)) => {
            let hosts = app.net.data.hosts.clone();
            app.dlg.open_set_restore(s, &hosts);
        }
        (Some(C_VERIFY), Some(s)) => {
            app.net.action(Method::Post, &format!("/api/tape/sets/{}/verify", s.id), None, "tape.verify", &["/api/tape/sets"], After::Operation);
        }
        _ => {}
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    if list.is_empty() && app.net.loaded("/api/tape/sets") {
        ui.next_size(-1, -1);
        ui.empty_state(BC_TAPE, t("Noch nichts auf Band"),
                       t("Einen Restore-Point schreibt „Auf Band …“ unter Sicherungsjobs › Restore-Points auf Band; ein Bandjob macht das regelmäßig."));
    } else {
        let cols = [
            GridCol::new(t("Job"), 200).flags(GC_FILL).min(150),
            GridCol::new(t("Restore-Point"), 140),
            GridCol::new(t("Pool"), 130),
            GridCol::new(t("Bänder"), 170),
            GridCol::new(t("Dateien"), 80).flags(GC_RIGHT),
            GridCol::new(t("Größe"), 90).flags(GC_RIGHT),
            GridCol::new(t("Zustand"), 140),
            GridCol::new(t("Geprüft"), 110),
        ];
        let (a, b) = ui.grid_begin(ID_SGRID, &mut app.tape.sets, &cols, &keys);
        for i in a..b {
            let s = &list[i as usize];
            let g = &mut app.tape.sets;
            ui.grid_row(g, i);
            ui.grid_cell(g, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(if s.encrypted { IC_LOCK } else { BC_TAPE }, 16);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &s.job_name);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &s.host_name);
            ui.box_end();
            ui.box_end();
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &fmt::when(s.point_time, &app.zone));
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &s.pool_name);
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &s.tapes.join(", "));
            ui.grid_text_ts(g, 4, TS_BODY | TX_SECONDARY, &fmt::count(s.files));
            ui.grid_text_ts(g, 5, TS_BODY | TX_SECONDARY, &fmt::size(s.bytes));
            ui.grid_cell(g, 6);
            let (label, kind) = set_state(&s.status);
            ui.badge(kind, label);
            ui.grid_text_ts(g, 7, TS_BODY | TX_SECONDARY, &fmt::when_short(s.verified, epoch, &app.zone));
        }
        ui.grid_end(&mut app.tape.sets);
    }
    ui.box_end();
}
