// hosts -- servers and agents: pair an agent (code) or install it over SSH, check, edit the
// shared folders, remove; the inspector shows the platform, certificate and folders

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::ids;
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_HOSTS};
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets, wizards};

#[derive(Default)]
pub struct State {
    pub grid: GridState,
    pub filter: String,
    pub split: i32,
}

const ID_CB: u32 = ID_HOSTS + 10;
const C_PAIR: u32 = ID_CB;
const C_SSH: u32 = ID_CB + 1;
const C_CHECK: u32 = ID_CB + 2;
const C_EDIT: u32 = ID_CB + 3;
const C_REMOVE: u32 = ID_CB + 4;
const C_REFRESH: u32 = ID_CB + 5;
const ID_SEARCH: u32 = ID_HOSTS + 40;
const ID_SPLIT: u32 = ID_HOSTS + 50;
const ID_INSP: u32 = ID_HOSTS + 100;
const ID_GRID: u32 = ID_HOSTS + 0x8_0000;

pub fn os_name(h: &Host) -> String {
    if h.platform.is_empty() {
        String::from(match h.os {
            Os::Linux => "Linux",
            Os::Windows => "Windows",
            Os::Unknown => "–",
        })
    } else {
        h.platform.clone()
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/hosts", 5000, now);
    app.net.want("/api/jobs", 15_000, now);
    let epoch = app.epoch(ui);
    let hosts: Vec<Host> = app
        .net
        .data
        .hosts
        .iter()
        .filter(|h| widgets::matches(&h.name, &app.hosts.filter) || widgets::matches(&h.address, &app.hosts.filter))
        .cloned()
        .collect();
    let keys: Vec<u64> = hosts.iter().map(|h| ids::key(&h.id)).collect();
    app.hosts.grid.retain(&keys);
    let one: Option<Host> = if app.hosts.grid.sel.len() == 1 { hosts.iter().find(|h| app.hosts.grid.sel.contains(&ids::key(&h.id))).cloned() } else { None };
    views::body_begin(ui);
    let all = &app.net.data.hosts;
    widgets::page_head(ui, t("Server & Agenten"), &tf("{} Hosts · {} online · {} Agenten", &[
        &format!("{}", all.len()),
        &format!("{}", all.iter().filter(|h| h.status == HostStatus::Online).count()),
        &format!("{}", all.iter().filter(|h| h.kind == HostKind::Agent).count()),
    ]));
    widgets::page_head_end(ui);
    let admin = app.may(Role::Admin);
    let clicked = ui.command_bar(ID_CB + 20, &[
        CmdItem::new(C_PAIR, BC_PAIR, t("Agent koppeln …")).flags(ST_ACCENT).enabled(admin),
        CmdItem::new(C_SSH, BC_TERMINAL, t("Per SSH installieren …")).enabled(admin),
        CmdItem::new(C_CHECK, BC_ACTIVITY, t("Verbindung prüfen")).enabled(app.may(Role::Operator) && one.is_some()).flags(CBI_SEP),
        CmdItem::new(C_EDIT, IC_FOLDER, t("Freigaben bearbeiten …")).enabled(admin && one.is_some()),
        CmdItem::new(C_REMOVE, IC_DELETE, t("Entfernen")).enabled(admin && one.as_ref().is_some_and(|h| h.kind == HostKind::Agent)),
        CmdItem::new(C_REFRESH, IC_REFRESH, t("Aktualisieren")).flags(CBI_END),
    ]);
    match (clicked, &one) {
        (Some(C_PAIR), _) => wizards::open_named(app, ui, "pair"),
        (Some(C_SSH), _) => wizards::open_named(app, ui, "ssh"),
        (Some(C_CHECK), Some(h)) => {
            app.net.action(Method::Post, &format!("/api/hosts/{}/check", h.id), None, "host.check", &["/api/hosts"], After::Operation);
        }
        (Some(C_EDIT), Some(h)) => app.dlg.open_roots(h),
        (Some(C_REMOVE), Some(h)) => {
            let n = app.net.data.jobs.iter().filter(|j| j.host_id == h.id).count();
            let extra = if n > 0 { tf(" {} Jobs sichern diesen Host; sie schlagen danach fehl.", &[&format!("{}", n)]) } else { String::new() };
            app.dlg.confirm(Confirm::RemoveHost(h.id.clone()), t("Agent entfernen?"),
                            &format!("{}{}", tf("{} wird entfernt und sein Zertifikat gesperrt. Zum erneuten Koppeln braucht er einen neuen Pairing-Code.", &[&h.name]), extra),
                            t("Entfernen"));
        }
        (Some(C_REFRESH), _) => app.refresh(ui),
        _ => {}
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    widgets::search(ui, ID_SEARCH, &mut app.hosts.filter, t("Hosts filtern"), 300);
    ui.box_end();
    ui.next_size(-1, -1);
    let inspector = one.is_some();
    if inspector {
        if app.hosts.split <= 0 {
            app.hosts.split = (ui.client_size().0 - 236 - 48 - 380).max(420);
        }
        ui.split_begin(ID_SPLIT, app.hosts.split, false);
    } else {
        ui.box_begin(0, BOX_V, 0);
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    let cols = [
        GridCol::new(t("Name"), 220).flags(GC_FILL),
        GridCol::new(t("Rolle"), 150),
        GridCol::new(t("Betriebssystem"), 170),
        GridCol::new(t("Status"), 130),
        GridCol::new(t("Letzter Kontakt"), 130),
        GridCol::new(t("Jobs"), 70).flags(GC_RIGHT),
    ];
    let (a, b) = ui.grid_begin(ID_GRID, &mut app.hosts.grid, &cols, &keys);
    for i in a..b {
        let h = &hosts[i as usize];
        let st = &mut app.hosts.grid;
        ui.grid_row(st, i);
        ui.grid_cell(st, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(if h.kind == HostKind::Server { BC_SERVER } else { BC_AGENT }, 16);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &h.name);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, &h.address);
        ui.box_end();
        ui.box_end();
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, if h.kind == HostKind::Server { t("Backup-Server") } else { t("Agent") });
        ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, &os_name(h));
        ui.grid_cell(st, 3);
        let (label, kind) = fmt::host_status(h.status);
        ui.badge(kind, label);
        ui.grid_text_ts(st, 4, TS_BODY | TX_SECONDARY, &fmt::relative(h.last_seen, epoch));
        let n = app.net.data.jobs.iter().filter(|j| j.host_id == h.id).count();
        ui.grid_text_ts(st, 5, TS_BODY | TX_SECONDARY, &format!("{}", n));
    }
    let ev = ui.grid_end(&mut app.hosts.grid);
    ui.box_end();
    if ev.ev & EV_DOUBLE != 0 && ev.row >= 0 && admin {
        let h = hosts[ev.row as usize].clone();
        app.dlg.open_roots(&h);
    }
    if inspector {
        ui.split_next();
        if let Some(h) = &one {
            inspect(app, ui, h, epoch);
        }
        let mut sp = app.hosts.split;
        if ui.split_end(&mut sp) {
            app.hosts.split = sp;
        }
    } else {
        ui.box_end();
    }
    views::body_end(ui);
}

fn inspect(app: &mut Console, ui: &mut Ui, h: &Host, epoch: i64) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED | pad(16), 12);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.icon(if h.kind == HostKind::Server { BC_SERVER } else { BC_AGENT }, 22);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 0);
    ui.text(TS_SUBTITLE | TX_ELLIPSIS, &h.name);
    ui.text(TS_CAPTION | TX_SECONDARY, &h.address);
    ui.box_end();
    let (label, kind) = fmt::host_status(h.status);
    ui.badge(kind, label);
    ui.box_end();
    if !h.message.is_empty() {
        widgets::notice(ui, if h.status == HostStatus::Offline { IB_ERROR } else { IB_INFO }, "", &h.message);
    }
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_INSP + 10, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 14);
    let os = os_name(h);
    let seen = if h.last_seen > 0 { format!("{} ({})", fmt::when(h.last_seen, &app.zone), fmt::relative(h.last_seen, epoch)) } else { String::from("–") };
    let since = fmt::when(h.created, &app.zone);
    let fp = if h.fingerprint.is_empty() { String::from("–") } else { group_hex(&h.fingerprint) };
    widgets::props(ui, &[
        (t("Rolle"), if h.kind == HostKind::Server { t("Backup-Server") } else { t("Agent") }),
        (t("Betriebssystem"), &os),
        (t("Version"), &h.version),
        (t("Letzter Kontakt"), &seen),
        (t("Gekoppelt seit"), &since),
    ]);
    ui.text(TS_BODY_STRONG, t("Zertifikat (SHA-256)"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &fp);
    ui.text(TS_BODY_STRONG, t("Freigegebene Ordner"));
    widgets::hint(ui, t("Nur aus diesen Ordnern darf der Server lesen. Der Agent verweigert alles andere."));
    for r in &h.roots {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(IC_FOLDER, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, r);
        ui.box_end();
    }
    ui.text(TS_BODY_STRONG, t("Wiederherstellungsziele"));
    if h.restore_roots.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Keine – Wiederherstellungen auf diesen Host sind gesperrt."));
    }
    for r in &h.restore_roots {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(IC_DOWNLOAD, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, r);
        ui.box_end();
    }
    let jobs: Vec<Job> = app.net.data.jobs.iter().filter(|j| j.host_id == h.id).cloned().collect();
    ui.text(TS_BODY_STRONG, &tf("Jobs ({})", &[&format!("{}", jobs.len())]));
    for j in &jobs {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let (_, kind) = fmt::session_status(j.last_status);
        ui.status_dot(kind);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &j.name);
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
    ui.box_end();
}

/// "AB12 CD34 …" for long hex fingerprints
pub fn group_hex(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && i % 4 == 0 {
            out.push(' ');
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}
