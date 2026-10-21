// shell -- the frame around the pages: title bar (logo, server, search, activity, theme, user
// menu, window buttons), navigation pane (sections, counts), the page or the open wizard, and
// the activity bar (connection, running operations, clock)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::model::*;
use ui::*;

use crate::app::{Console, Page, ID_SHELL};
use crate::icons::*;
use crate::net::Link;
use crate::tr::{self, en, t, tf};
use crate::views;

const ID_TB: u32 = ID_SHELL;
const ID_SEARCH: u32 = ID_SHELL + 20;
const ID_SEARCH_POP: u32 = ID_SHELL + 21;
const ID_SEARCH_HIT: u32 = ID_SHELL + 40;
const ID_NAV: u32 = ID_SHELL + 100;
const ID_NAV_ITEM: u32 = ID_SHELL + 120;
const ID_BODY: u32 = ID_SHELL + 200;
const ID_SB: u32 = ID_SHELL + 300;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(ID_BODY, BOX_V | BOX_FIXED, 0);
    titlebar(app, ui);
    ui.next_size(-1, -1);
    ui.box_begin(ID_BODY + 1, BOX_H | BOX_FIXED, 0);
    nav(app, ui);
    ui.next_size(-1, -1);
    ui.box_begin(ID_BODY + 2, BOX_V | BOX_CLIP | BOX_FIXED, 0);
    if let Some(mut w) = app.wizard.take() {
        let keep = w.frame(app, ui);
        if keep && app.wizard.is_none() {
            app.wizard = Some(w);
        }
    } else {
        views::page(app, ui);
    }
    ui.box_end();
    ui.box_end();
    activity_bar(app, ui);
    ui.box_end();
}

/// the logo: an accent tile with the shield
pub fn logo(ui: &mut Ui, size: i32) {
    let s = ui.dp(size);
    let r = ui.place(s, s);
    let a = ui.color(T_ACCENT);
    ui.cmd_rrect(r[0], r[1], s, s, ui.dp(size / 4), 0, a, 0, 0);
    let c = ui.color(T_ON_ACCENT);
    ui.draw_color(c);
    let i = s * 2 / 3;
    ui.draw_icon(BC_SHIELD_CHECK, r[0] + (s - i) / 2, r[1] + (s - i) / 2, i << 8);
}

fn titlebar(app: &mut Console, ui: &mut Ui) {
    ui.titlebar_begin_opt(None, 0);
    ui.spacer(4);
    logo(ui, 24);
    ui.spacer(10);
    ui.text(TS_BODY_STRONG, "BackupCraft");
    ui.spacer(16);
    // server chip
    let (kind, state) = match app.net.link {
        Link::Online => (BADGE_OK, t("Verbunden")),
        Link::Connecting => (BADGE_INFO, t("Verbinde …")),
        Link::Lost => (BADGE_CRIT, t("Verbindung unterbrochen")),
        Link::Offline => (BADGE_NEUTRAL, t("Getrennt")),
    };
    let server = app.net.data.info.as_ref().map_or_else(|| app.net.server.clone(), |i| i.name.clone());
    ui.status_dot(kind);
    ui.spacer(6);
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    let chip = format!("{}{}", server, if app.fixture { t(" · Demo") } else { "" });
    ui.dropdown_button(ID_TB + 1, &chip);
    if ui.flyout_begin(ID_TB + 1, 0) {
        ui.box_begin(0, BOX_V | pad(10), 6);
        ui.text(TS_BODY_STRONG, &server);
        ui.text(TS_CAPTION | TX_SECONDARY, state);
        if let Some(i) = &app.net.data.info {
            ui.text(TS_CAPTION | TX_SECONDARY, &tf("Version {}", &[&i.version]));
            ui.text(TS_CAPTION | TX_SECONDARY, if i.tls { t("Verschlüsselt (TLS 1.3)") } else { t("Lokale Verbindung (HTTP)") });
        }
        if app.fixture {
            ui.next_size(260, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Demo-Daten: Änderungen bleiben nur bis zum Beenden erhalten."));
        }
        ui.box_end();
        ui.menu_sep();
        ui.next_icon(BC_EXIT);
        if ui.menu_item(ID_TB + 2, t("Verbindung trennen")) & EV_ACT != 0 {
            app.logout(ui, None);
        }
    }
    ui.flyout_end();
    ui.spacer(-1);
    search(app, ui);
    ui.spacer(-1);
    // activity
    let running: Vec<Operation> = app.net.data.running_ops().cloned().collect();
    ui.next_icon(IC_BELL);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
    ui.dropdown_button(ID_TB + 3, t("Aktivität"));
    if !running.is_empty() {
        let r = ui.last_rect();
        let d = ui.dp(8);
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        ui.draw_circle((r[0] + r[2] - ui.dp(12)) << 8, (r[1] + ui.dp(8)) << 8, d << 7, 0);
    }
    if ui.flyout_begin(ID_TB + 3, FL_ALIGN_END) {
        ui.box_begin(0, BOX_V | pad(8), 8);
        ui.next_size(340, 0);
        ui.text(TS_BODY_STRONG, t("Aktivität"));
        if running.is_empty() {
            ui.text(TS_BODY | TX_SECONDARY, t("Keine laufenden Vorgänge."));
        }
        for (i, o) in running.iter().take(6).enumerate() {
            ui.box_begin(0, BOX_V, 3);
            ui.next_size(340, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &o.title);
            ui.next_size(340, 0);
            ui.progress(ID_TB + 10 + i as u32, (o.done.min(o.total) * 1000 / o.total.max(1)) as i32, 1000);
            ui.box_end();
        }
        let toasts = ui.toasts();
        if !toasts.is_empty() {
            ui.separator();
            for (_, kind, title, msg) in toasts.iter().rev().take(4) {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.status_dot(match *kind {
                    IB_SUCCESS => BADGE_OK,
                    IB_WARNING => BADGE_WARN,
                    IB_ERROR => BADGE_CRIT,
                    _ => BADGE_INFO,
                });
                ui.next_size(310, 0);
                ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{} – {}", title, msg));
                ui.box_end();
            }
        }
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.button(ID_TB + 4, t("Alle Vorgänge anzeigen")) & EV_ACT != 0 {
            app.mon.tab = 1;
            app.go(ui, Page::Monitoring);
            ui.popup_close();
        }
        ui.box_end();
    }
    ui.flyout_end();
    // theme
    let dark = ui.theme_dark();
    if ui.icon_button(ID_TB + 5, if dark { BC_SUN } else { BC_MOON }, if dark { t("Helles Design") } else { t("Dunkles Design") }) & EV_ACT != 0 {
        app.prefs.theme = if dark { 0 } else { 1 };
        ui.theme_set(app.prefs.theme);
        app.save_prefs();
    }
    // user menu
    let user = app.net.data.me.clone().unwrap_or_default();
    ui.next_icon(IC_PERSON);
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.dropdown_button(ID_TB + 6, &user.username);
    if ui.flyout_begin(ID_TB + 6, FL_ALIGN_END) {
        ui.box_begin(0, BOX_V | pad(10), 4);
        ui.text(TS_BODY_STRONG, &user.username);
        ui.text(TS_CAPTION | TX_SECONDARY, views::role_name(user.role));
        ui.box_end();
        ui.menu_sep();
        ui.next_icon(BC_LANGUAGE);
        if ui.menu_item(ID_TB + 7, if en() { "Deutsch" } else { "English" }) & EV_ACT != 0 {
            app.prefs.lang = if en() { tr::LANG_DE } else { tr::LANG_EN };
            tr::init(app.prefs.lang);
            app.save_prefs();
            ui.repass();
        }
        ui.next_icon(IC_SETTINGS);
        if ui.menu_item(ID_TB + 8, t("Einstellungen")) & EV_ACT != 0 {
            app.go(ui, Page::Settings);
        }
        ui.next_icon(IC_LOCK);
        if ui.menu_item(ID_TB + 9, t("Passwort ändern …")) & EV_ACT != 0 {
            app.dlg.open_password();
        }
        ui.menu_sep();
        ui.next_icon(BC_EXIT);
        if ui.menu_item(ID_TB + 11, t("Abmelden")) & EV_ACT != 0 {
            app.logout(ui, None);
        }
    }
    ui.flyout_end();
    ui.spacer(4);
    ui.titlebar_end();
}

/// one search hit: page, id, icon, label, detail
struct Hit {
    page: Page,
    id: String,
    icon: u32,
    label: String,
    detail: String,
}

fn search(app: &mut Console, ui: &mut Ui) {
    ui.next_size(420, 0);
    ui.textbox_icon(ID_SEARCH, IC_SEARCH, Some(t("Jobs, Server, Repositories suchen (Strg+K)")), &mut app.search, 80);
    let q = app.search.trim();
    if q.is_empty() {
        if ui.popup_is_open(ID_SEARCH_POP) {
            ui.popup_close();
        }
        return;
    }
    let mut hits: Vec<Hit> = Vec::new();
    let d = &app.net.data;
    for j in &d.jobs {
        if crate::widgets::matches(&j.name, q) || j.sources.iter().any(|s| crate::widgets::matches(s, q)) {
            hits.push(Hit { page: Page::Jobs, id: j.id.clone(), icon: BC_JOB, label: j.name.clone(), detail: String::from(t("Sicherungsjob")) });
        }
    }
    for h in &d.hosts {
        if crate::widgets::matches(&h.name, q) || crate::widgets::matches(&h.address, q) {
            hits.push(Hit { page: Page::Hosts, id: h.id.clone(), icon: if h.kind == HostKind::Server { BC_SERVER } else { BC_AGENT }, label: h.name.clone(),
                            detail: h.address.clone() });
        }
    }
    for r in &d.repos {
        if crate::widgets::matches(&r.name, q) || crate::widgets::matches(&r.path, q) {
            hits.push(Hit { page: Page::Repositories, id: r.id.clone(), icon: BC_REPOSITORY, label: r.name.clone(), detail: r.path.clone() });
        }
    }
    hits.truncate(8);
    if !ui.popup_is_open(ID_SEARCH_POP) && ui.focused() == ID_SEARCH {
        ui.popup_open(ID_SEARCH_POP);
    }
    if ui.flyout_begin(ID_SEARCH_POP, FL_MINW) {
        if hits.is_empty() {
            ui.box_begin(0, BOX_V | pad(10), 4);
            ui.text(TS_BODY | TX_SECONDARY, t("Nichts gefunden."));
            ui.box_end();
        }
        let mut go = None;
        for (i, h) in hits.iter().enumerate() {
            ui.next_icon(h.icon);
            if ui.menu_item(ID_SEARCH_HIT + i as u32, &format!("{}  ·  {}", h.label, h.detail)) & EV_ACT != 0 {
                go = Some(i);
            }
        }
        if let Some(i) = go {
            let h = &hits[i];
            let (page, id) = (h.page, h.id.clone());
            app.search.clear();
            app.go(ui, page);
            views::select(app, page, &id);
        }
    }
    ui.flyout_end();
}

fn nav(app: &mut Console, ui: &mut Ui) {
    if app.net.data.capable(bc_proto::api::CAP_REPLICATION) {
        // replicas in failover or failed show on the navigation
        app.net.want("/api/replicas", 30_000, ui.now());
    }
    let d = &app.net.data;
    let failed = d.jobs.iter().filter(|j| j.last_status == SessionStatus::Failed).count();
    let offline = d.hosts.iter().filter(|h| h.status == HostStatus::Offline).count();
    let full = d.repos.iter().filter(|r| r.status != RepoStatus::Ok).count();
    let running = d.running_ops().count();
    let count = |n: usize| if n > 0 { Some(format!("{}", n)) } else { None };
    let (failed, offline, full, running) = (count(failed), count(offline), count(full), count(running));
    ui.next_size(if app.nav.compact { 56 } else { 236 }, -1);
    ui.nav_begin(ID_NAV, &mut app.nav);
    let mut nav = app.nav;
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(4), 6);
    if ui.nav_toggle(ID_NAV + 1, t("Navigation umschalten"), &mut nav) & EV_ACT != 0 {
        app.nav = nav;
        app.save_prefs();
    }
    ui.box_end();
    let wiz = app.wizard.is_some();
    // staged pages say "bald" until the server offers them
    let soon = |caps: &[&str]| if caps.iter().any(|c| d.capable(c)) { None } else { Some(String::from(t("bald"))) };
    let repl = soon(&[bc_proto::api::CAP_REPLICATION, bc_proto::api::CAP_CDP])
        .or_else(|| count(d.replicas.iter().filter(|r| matches!(r.state, ReplicaState::Failed | ReplicaState::Failover)).count()));
    let tape = soon(&[bc_proto::api::CAP_TAPE]);
    let instant = soon(&[bc_proto::api::CAP_INSTANT, bc_proto::api::CAP_BOOTCHECK, bc_proto::api::CAP_HYPERVISOR]);
    let items: [(Option<&str>, Page, u32, &str, Option<String>); 11] = [
        (Some(t("Start")), Page::Dashboard, BC_DASHBOARD, t("Übersicht"), None),
        (Some(t("Schutz")), Page::Jobs, BC_JOB, t("Sicherungsjobs"), failed),
        (None, Page::Restore, BC_RESTORE, t("Wiederherstellung"), None),
        (Some(t("Infrastruktur")), Page::Hosts, BC_SERVER, t("Server & Agenten"), offline),
        (None, Page::Repositories, BC_REPOSITORY, t("Repositories"), full),
        (None, Page::Credentials, BC_KEY, t("Zugangsdaten"), None),
        (Some(t("Erweitert")), Page::Replication, BC_REPLICATE, t("Replikation"), repl),
        (None, Page::Tape, BC_TAPE, t("Tape"), tape),
        (None, Page::Recovery, BC_VM, t("Sofortstart"), instant),
        (Some(t("Betrieb")), Page::Monitoring, BC_ACTIVITY, t("Monitoring"), running),
        (None, Page::Settings, IC_SETTINGS, t("Einstellungen"), None),
    ];
    for (i, (section, page, icon, label, badge)) in items.iter().enumerate() {
        if *page == Page::Settings {
            ui.nav_footer();
        } else if let Some(s) = section {
            ui.nav_section(s);
        }
        if wiz {
            ui.next_flags(ST_DISABLED);
        }
        if ui.nav_item(ID_NAV_ITEM + i as u32, *icon, label, app.page == *page && !wiz, badge.as_deref()) & EV_ACT != 0 {
            app.go(ui, *page);
        }
    }
    ui.nav_end();
}

fn activity_bar(app: &mut Console, ui: &mut Ui) {
    ui.statusbar_begin();
    let (kind, text) = match app.net.link {
        Link::Online => (BADGE_OK, if app.fixture { String::from(t("Demo-Server · Daten werden simuliert")) } else { tf("Verbunden mit {}", &[&app.net.server]) }),
        Link::Connecting => (BADGE_INFO, String::from(t("Verbindung wird hergestellt …"))),
        Link::Lost => (BADGE_CRIT, String::from(t("Verbindung unterbrochen – neuer Versuch läuft"))),
        Link::Offline => (BADGE_NEUTRAL, String::from(t("Nicht verbunden"))),
    };
    ui.status_dot(kind);
    ui.text(TS_CAPTION | TX_SECONDARY, &text);
    let running: Vec<Operation> = app.net.data.running_ops().cloned().collect();
    if !running.is_empty() {
        ui.next_size(1, 14);
        ui.separator();
        ui.spinner(ID_SB + 1, 12);
        let label = if running.len() == 1 { running[0].title.clone() } else { tf("{} Vorgänge laufen", &[&format!("{}", running.len())]) };
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.label_button(ID_SB + 2, &label, TS_CAPTION, 6) & EV_ACT != 0 {
            app.mon.tab = 1;
            app.go(ui, Page::Monitoring);
        }
        let o = &running[0];
        ui.next_size(120, 0);
        ui.progress(ID_SB + 3, (o.done.min(o.total) * 1000 / o.total.max(1)) as i32, 1000);
    }
    ui.spacer(-1);
    let now = app.epoch(ui);
    let clock = format!("{} · {}", app.zone.name(), datetime::format_time(datetime::Timestamp::from_secs(now), &app.zone));
    ui.text(TS_CAPTION | TX_TERTIARY, &clock);
    ui.statusbar_end();
}
