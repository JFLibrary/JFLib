// settings -- categories at the left: general, users and roles, security (recovery kit,
// configuration backups, login rules), restore targets, network, notifications (e-mail, web
// hook, daily summary, the last delivery), about; each form edits a copy and saves with one
// button.  Network also lists the fleet tokens central servers watch this one with.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::ids;
use bc_proto::j::{self, Json};
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_SET};
use crate::icons::*;
use crate::net::After;
use crate::tr::{self, en, t, tf};
use crate::views::dialogs::Confirm;
use crate::{fmt, views, widgets};

#[derive(Default)]
pub struct State {
    pub cat: i32,
    loaded_ver: u64,
    loaded: bool,
    pub name: String,
    pub roots: String,
    pub mbps: i32,
    pub hours: i32,
    pub attempts: i32,
    pub notify: i32,
    pub smtp_host: String,
    pub smtp_to: String,
    pub smtp_security: i32,
    pub smtp_user: String,
    pub smtp_pass: String,
    pub smtp_from: String,
    pub smtp_ca: String,
    pub webhook: String,
    pub daily_on: i32,
    pub daily_h: i32,
    pub daily_m: i32,
    pub upd_url: String,
    pub upd_key: String,
    pub upd_auto: i32,
    pub auto_backup: i32,
    pub users: GridState,
    pub tokens: GridState,
}

const ID_CATS: u32 = ID_SET + 1;
// the form's widgets; a segmented control also takes the ids after its own (one per
// segment), an infobar the next one (its close button)
const ID_F: u32 = ID_SET + 100;
const ID_SAVE: u32 = ID_SET + 90;
const ID_UGRID: u32 = ID_SET + 0x8_0000;
const ID_TGRID: u32 = ID_SET + 0x9_0000;

fn load(app: &mut Console) {
    let d = &app.net.data;
    if app.settings.loaded && app.settings.loaded_ver != 0 {
        return;
    }
    if d.settings.is_empty() {
        return;
    }
    let s = &mut app.settings;
    s.name = String::from(d.setting("server.name"));
    s.roots = String::from(d.setting("restore.allowed_roots"));
    s.mbps = d.setting("traffic.max_mbps").parse().unwrap_or(0);
    s.hours = d.setting("session.timeout_hours").parse().unwrap_or(24);
    s.attempts = d.setting("login.max_attempts").parse().unwrap_or(5);
    s.notify = match d.setting("notify.mode") {
        "failure" => 1,
        "all" => 2,
        _ => 0,
    };
    s.smtp_host = String::from(d.setting("smtp.host"));
    s.smtp_to = String::from(d.setting("smtp.to"));
    s.smtp_security = match d.setting("smtp.security") {
        "tls" => 1,
        "none" => 2,
        _ => 0,
    };
    s.smtp_user = String::from(d.setting("smtp.user"));
    s.smtp_pass = String::from(d.setting("smtp.password"));
    s.smtp_from = String::from(d.setting("smtp.from"));
    s.smtp_ca = String::from(d.setting("smtp.ca"));
    s.webhook = String::from(d.setting("notify.webhook"));
    let daily = d.setting("notify.daily");
    s.daily_on = (!daily.is_empty()) as i32;
    let (h, m) = daily.split_once(':').unwrap_or(("7", "0"));
    s.daily_h = h.parse().unwrap_or(7);
    s.daily_m = m.parse().unwrap_or(0);
    s.upd_url = String::from(d.setting("update.url"));
    s.upd_key = String::from(d.setting("update.key"));
    s.upd_auto = (d.setting("update.auto") == "true") as i32;
    s.auto_backup = (d.setting("config.auto_backup") == "true") as i32;
    s.loaded = true;
    s.loaded_ver = d.ver.max(1);
}

fn save(app: &mut Console, pairs: &[(&str, String)]) {
    let items: Vec<Setting> = pairs.iter().map(|(k, v)| Setting { key: String::from(*k), value: v.clone() }).collect();
    app.net.action(Method::Put, "/api/settings", Some(j::list(&items)), "settings.save", &["/api/settings"], After::Nothing);
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    let admin = app.may(Role::Admin);
    if admin {
        app.net.want("/api/settings", 30_000, now);
        load(app);
    }
    views::body_begin(ui);
    widgets::page_head(ui, t("Einstellungen"), t("Server, Benutzer, Sicherheit und Darstellung."));
    widgets::page_head_end(ui);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 16);
    // categories
    let cats = [
        (IC_SETTINGS, t("Allgemein")),
        (BC_USERS, t("Benutzer & Rollen")),
        (BC_SHIELD, t("Sicherheit")),
        (BC_RESTORE, t("Wiederherstellung")),
        (BC_NETWORK, t("Netzwerk")),
        (IC_BELL, t("Benachrichtigungen")),
        (IC_INFO, t("Über BackupCraft")),
    ];
    ui.next_size(240, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(6), 2);
    for (i, (icon, label)) in cats.iter().enumerate() {
        ui.next_flags(if !admin && (1..=4).contains(&i) { ST_DISABLED } else { 0 });
        if ui.nav_item(ID_CATS + i as u32, *icon, label, app.settings.cat == i as i32, None) & EV_ACT != 0 {
            app.settings.cat = i as i32;
        }
    }
    ui.box_end();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_SET + 50, SC_V);
    ui.box_begin(0, BOX_V | pad(2), 16);
    match app.settings.cat {
        0 => general(app, ui, admin),
        1 if admin => users(app, ui),
        2 if admin => security(app, ui),
        3 if admin => restore(app, ui),
        4 if admin => network(app, ui),
        5 if admin => notify(app, ui),
        _ => about(app, ui, admin),
    }
    ui.box_end();
    ui.scroll_end();
    ui.box_end();
    views::body_end(ui);
}

fn general(app: &mut Console, ui: &mut Ui, admin: bool) {
    widgets::card_begin(ui, t("Darstellung"), t("Gilt für diese Konsole auf diesem Rechner."));
    widgets::label(ui, t("Sprache"));
    let mut lang = if en() { 1 } else { 0 };
    widgets::inline_begin(ui);
    if ui.segmented(ID_F + 90, &mut lang, &["Deutsch", "English"]) & EV_ACT != 0 {
        app.prefs.lang = if lang == 1 { tr::LANG_EN } else { tr::LANG_DE };
        tr::init(app.prefs.lang);
        app.save_prefs();
    }
    widgets::inline_end(ui);
    widgets::label(ui, t("Design"));
    let mut theme = app.prefs.theme as i32;
    widgets::inline_begin(ui);
    if ui.segmented(ID_F + 94, &mut theme, &[t("Hell"), t("Dunkel"), t("Wie das System")]) & EV_ACT != 0 {
        app.prefs.theme = theme as u32;
        ui.theme_set(app.prefs.theme);
        app.save_prefs();
    }
    widgets::inline_end(ui);
    widgets::label(ui, t("Zeitzone der Anzeige"));
    ui.text(TS_BODY | TX_SECONDARY, &tf("{} – Zeitpläne verwenden ihre eigene Zeitzone.", &[app.zone.name()]));
    widgets::card_end(ui);
    if admin {
        widgets::card_begin(ui, t("Server"), "");
        widgets::label(ui, t("Name des Servers"));
        ui.next_size(360, 0);
        ui.textbox(ID_F + 3, Some("backup-srv01"), &mut app.settings.name, 64);
        widgets::hint(ui, t("Erscheint in der Titelleiste, in Berichten und im Recovery-Kit."));
        widgets::label(ui, t("Konfiguration täglich automatisch sichern"));
        ui.switch(ID_F + 4, if app.settings.auto_backup != 0 { t("An") } else { t("Aus") }, &mut app.settings.auto_backup);
        widgets::inline_begin(ui);
        if ui.accent_button(ID_SAVE, t("Speichern")) & EV_ACT != 0 {
            let s = &app.settings;
            let pairs = [("server.name", s.name.clone()), ("config.auto_backup", String::from(if s.auto_backup != 0 { "true" } else { "false" }))];
            save(app, &pairs);
        }
        widgets::inline_end(ui);
        widgets::card_end(ui);
    }
}

fn users(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/users", 15_000, now);
    let users: Vec<User> = app.net.data.users.clone();
    let keys: Vec<u64> = users.iter().map(|u| ids::key(&u.id)).collect();
    app.settings.users.retain(&keys);
    let sel: Option<User> = users.iter().find(|u| app.settings.users.sel.contains(&ids::key(&u.id))).cloned();
    widgets::card_begin(ui, t("Benutzer & Rollen"), t("Betrachter sehen alles, Wiederherstellung darf zurückholen, Operatoren starten und ändern Jobs, Administratoren verwalten alles."));
    let clicked = ui.command_bar(ID_F + 10, &[
        CmdItem::new(ID_F + 11, IC_ADD, t("Benutzer anlegen …")).flags(ST_ACCENT),
        CmdItem::new(ID_F + 12, IC_EDIT, t("Bearbeiten …")).enabled(sel.is_some()),
        CmdItem::new(ID_F + 13, IC_DELETE, t("Löschen")).enabled(sel.is_some()),
    ]);
    match (clicked, &sel) {
        (Some(x), _) if x == ID_F + 11 => app.dlg.open_user(None),
        (Some(x), Some(u)) if x == ID_F + 12 => app.dlg.open_user(Some(u)),
        (Some(x), Some(u)) if x == ID_F + 13 => {
            app.dlg.confirm(Confirm::DeleteUser(u.id.clone()), t("Benutzer löschen?"), &tf("{} kann sich danach nicht mehr anmelden.", &[&u.username]), t("Löschen"));
        }
        _ => {}
    }
    ui.next_size(-1, 320);
    ui.box_begin(0, BOX_V | BOX_LAYER | BOX_CLIP, 0);
    let cols = [GridCol::new(t("Benutzer"), 200).flags(GC_FILL), GridCol::new(t("Rolle"), 160), GridCol::new(t("Letzte Anmeldung"), 170), GridCol::new(t("Status"), 120)];
    let (a, b) = ui.grid_begin(ID_UGRID, &mut app.settings.users, &cols, &keys);
    for i in a..b {
        let u = &users[i as usize];
        let st = &mut app.settings.users;
        ui.grid_row(st, i);
        ui.grid_cell(st, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(IC_PERSON, 16);
        ui.text(TS_BODY, &u.username);
        ui.box_end();
        ui.grid_text_ts(st, 1, TS_BODY | TX_SECONDARY, views::role_name(u.role));
        ui.grid_text_ts(st, 2, TS_BODY | TX_SECONDARY, &fmt::when(u.last_login, &app.zone));
        ui.grid_cell(st, 3);
        ui.badge(if u.disabled { BADGE_NEUTRAL } else { BADGE_OK }, if u.disabled { t("Gesperrt") } else { t("Aktiv") });
    }
    let ev = ui.grid_end(&mut app.settings.users);
    ui.box_end();
    if ev.ev & EV_DOUBLE != 0 && ev.row >= 0 {
        let u = users[ev.row as usize].clone();
        app.dlg.open_user(Some(&u));
    }
    widgets::card_end(ui);
}

fn security(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/recovery", 30_000, now);
    app.net.want("/api/config/backups", 30_000, now);
    widgets::card_begin(ui, t("Recovery-Kit"), t("Der private Schlüssel öffnet jede verschlüsselte Sicherung – auch ohne diesen Server. Offline und getrennt aufbewahren."));
    let rs = app.net.data.recovery.clone().unwrap_or_default();
    if rs.created > 0 {
        widgets::props(ui, &[(t("Erzeugt"), &fmt::when(rs.created, &app.zone)), (t("Fingerabdruck"), &rs.fingerprint)]);
    } else {
        ui.infobar(ID_F + 20, IB_WARNING, t("Kein Recovery-Kit"), t("Ohne Kit hängt die Wiederherstellung verschlüsselter Daten an diesem Server."));
    }
    widgets::inline_begin(ui);
    if ui.button(ID_F + 27, t("Neues Recovery-Kit erzeugen …")) & EV_ACT != 0 {
        app.dlg.confirm(Confirm::RecoveryKit, t("Neues Recovery-Kit erzeugen?"),
                        t("Der private Schlüssel wird genau einmal angezeigt. Neue Sicherungen verwenden das neue Kit; das alte bleibt für ältere Sicherungen nötig."),
                        t("Erzeugen"));
    }
    widgets::inline_end(ui);
    widgets::card_end(ui);
    widgets::card_begin(ui, t("Konfigurationssicherung"), t("Datenbank, Schlüssel und Zertifikate des Servers – verschlüsselt mit einer Passphrase."));
    let backups = app.net.data.config_backups.clone();
    for b in backups.iter().take(5) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.icon(BC_ARCHIVE, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &b.name);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {}", fmt::when(b.created, &app.zone), fmt::size(b.size)));
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_F + 22, t("Jetzt sichern …")) & EV_ACT != 0 {
        app.dlg.open_config_backup();
    }
    ui.next_flags(if backups.is_empty() { ST_DISABLED } else { 0 });
    if ui.button(ID_F + 23, t("Wiederherstellen …")) & EV_ACT != 0 {
        app.dlg.open_config_restore(backups.first().map_or("", |b| b.name.as_str()));
    }
    ui.box_end();
    widgets::card_end(ui);
    widgets::card_begin(ui, t("Anmeldung"), "");
    widgets::label(ui, t("Sitzungsdauer (Stunden)"));
    ui.next_size(120, 0);
    ui.number_box(ID_F + 24, &mut app.settings.hours, &Range::new(1, 168, 1, 0));
    widgets::label(ui, t("Fehlversuche bis zur Sperre (pro Minute)"));
    ui.next_size(120, 0);
    ui.number_box(ID_F + 25, &mut app.settings.attempts, &Range::new(3, 20, 1, 0));
    widgets::hint(ui, t("Passwörter brauchen mindestens 12 Zeichen und werden mit Argon2id gespeichert."));
    widgets::inline_begin(ui);
    if ui.accent_button(ID_SAVE + 1, t("Speichern")) & EV_ACT != 0 {
        let s = &app.settings;
        let pairs = [("session.timeout_hours", format!("{}", s.hours)), ("login.max_attempts", format!("{}", s.attempts))];
        save(app, &pairs);
    }
    widgets::inline_end(ui);
    widgets::card_end(ui);
}

fn restore(app: &mut Console, ui: &mut Ui) {
    widgets::card_begin(ui, t("Erlaubte Wiederherstellungsziele"), t("Benutzer mit der Rolle „Wiederherstellung“ dürfen nur in diese Ordner auf dem Server zurückholen. Vorhandene Dateien werden nie stillschweigend überschrieben."));
    widgets::label(ui, t("Ordner (einer pro Zeile)"));
    ui.next_size(-1, 140);
    ui.textbox(ID_F + 30, Some("/srv/restore"), &mut app.settings.roots, TB_MULTI | 4096);
    widgets::inline_begin(ui);
    if ui.accent_button(ID_SAVE + 2, t("Speichern")) & EV_ACT != 0 {
        let pairs = [("restore.allowed_roots", app.settings.roots.clone())];
        save(app, &pairs);
    }
    widgets::inline_end(ui);
    widgets::card_end(ui);
}

fn network(app: &mut Console, ui: &mut Ui) {
    widgets::card_begin(ui, t("Bandbreite"), t("Begrenzt die Übertragung von Agenten zum Server (0 = unbegrenzt)."));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.next_size(140, 0);
    ui.number_box(ID_F + 40, &mut app.settings.mbps, &Range::new(0, 100_000, 10, 0));
    ui.text(TS_BODY | TX_SECONDARY, "Mbit/s");
    ui.box_end();
    widgets::inline_begin(ui);
    if ui.accent_button(ID_SAVE + 3, t("Speichern")) & EV_ACT != 0 {
        let pairs = [("traffic.max_mbps", format!("{}", app.settings.mbps))];
        save(app, &pairs);
    }
    widgets::inline_end(ui);
    widgets::card_end(ui);
    widgets::card_begin(ui, t("Verbindungen"), "");
    let tls = app.net.data.info.as_ref().is_some_and(|i| i.tls);
    let fp = app.net.data.info.as_ref().map_or(String::new(), |i| i.ca_fingerprint.clone());
    widgets::props(ui, &[
        (t("Konsole und bcctl"), &format!("{} · {}", bc_proto::DEFAULT_PORT, if tls { "HTTPS (TLS 1.3)" } else { "HTTP (lokal)" })),
        (t("Agenten"), &format!("{} · {}", bc_proto::AGENT_PORT, t("gegenseitiges TLS mit eigener CA"))),
        (t("Außerhalb des Servers"), t("nur verschlüsselt")),
        (t("Fingerabdruck der CA"), if fp.is_empty() { "–" } else { &fp }),
    ]);
    widgets::card_end(ui);
    if app.net.data.capable(bc_proto::api::CAP_FLEET) {
        fleet_tokens(app, ui);
    }
}

/// the tokens central servers read this server's state with
fn fleet_tokens(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    app.net.want("/api/fleet/tokens", 30_000, now);
    let epoch = app.epoch(ui);
    let list: Vec<FleetToken> = app.net.data.fleet_tokens.clone();
    widgets::card_begin(ui, t("Zentrale Überwachung"), t("Andere BackupCraft-Server (Zentralen) lesen mit einem Überwachungs-Token den Zustand dieses Servers. Mehr dürfen sie damit nicht."));
    let keys: Vec<u64> = list.iter().map(|x| ids::key(&x.id)).collect();
    app.settings.tokens.retain(&keys);
    let sel = list.iter().find(|x| app.settings.tokens.sel.contains(&ids::key(&x.id))).cloned();
    if !list.is_empty() {
        ui.next_size(-1, 44 + 36 * list.len().min(6) as i32);
        ui.box_begin(0, BOX_V | BOX_LAYER | BOX_CLIP | BOX_FIXED, 0);
        let cols = [GridCol::new(t("Bezeichnung"), 200).flags(GC_FILL), GridCol::new(t("Erzeugt"), 150), GridCol::new(t("Von"), 100), GridCol::new(t("Zuletzt verwendet"), 140)];
        let (a, b) = ui.grid_begin(ID_TGRID, &mut app.settings.tokens, &cols, &keys);
        for i in a..b {
            let x = &list[i as usize];
            let g = &mut app.settings.tokens;
            ui.grid_row(g, i);
            ui.grid_text(g, 0, &x.label);
            ui.grid_text_ts(g, 1, TS_BODY | TX_SECONDARY, &fmt::when(x.created, &app.zone));
            ui.grid_text_ts(g, 2, TS_BODY | TX_SECONDARY, &x.created_by);
            ui.grid_text_ts(g, 3, TS_BODY | TX_SECONDARY, &fmt::when_short(x.last_used, epoch, &app.zone));
        }
        ui.grid_end(&mut app.settings.tokens);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.button(ID_F + 45, t("Token erzeugen …")) & EV_ACT != 0 {
        app.dlg.open_fleet_token();
    }
    ui.next_flags(if sel.is_none() { ST_DISABLED } else { 0 });
    if ui.button(ID_F + 46, t("Widerrufen")) & EV_ACT != 0 {
        if let Some(x) = &sel {
            app.dlg.confirm(Confirm::RevokeFleetToken(x.id.clone()), &tf("Token „{}“ widerrufen?", &[&x.label]),
                            t("Die Zentrale, die es verwendet, erreicht diesen Server danach nicht mehr."), t("Widerrufen"));
        }
    }
    ui.box_end();
    widgets::card_end(ui);
}

fn notify(app: &mut Console, ui: &mut Ui) {
    let available = app.net.data.capable(bc_proto::api::CAP_NOTIFY);
    if available {
        app.net.want("/api/notify", 30_000, ui.now());
    } else {
        ui.infobar(ID_F + 50, IB_INFO, t("Nicht verfügbar"), t("Dieser Server versendet keine Benachrichtigungen (ältere Version). Die Einstellungen werden trotzdem gespeichert."));
    }
    widgets::card_begin(ui, t("Benachrichtigungen"), t("Nach jeder Sitzung je nach Auswahl, dazu auf Wunsch eine tägliche Zusammenfassung."));
    widgets::label(ui, t("Benachrichtigen bei"));
    widgets::inline_begin(ui);
    ui.next_size(480, 0);
    ui.segmented(ID_F + 100, &mut app.settings.notify, &[t("Nie"), t("Fehlern und Warnungen"), t("Jedem Lauf")]);
    widgets::inline_end(ui);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.switch(ID_F + 64, t("Tägliche Zusammenfassung um"), &mut app.settings.daily_on);
    ui.next_flags(if app.settings.daily_on != 0 { 0 } else { ST_DISABLED });
    ui.time_box(ID_F + 65, &mut app.settings.daily_h, &mut app.settings.daily_m);
    ui.text(TS_BODY | TX_SECONDARY, t("Uhr (Serverzeit)"));
    ui.box_end();
    widgets::card_end(ui);

    widgets::card_begin(ui, t("E-Mail (SMTP)"), t("Der Server prüft das Zertifikat des Mailservers wie ein Browser; eine eigene Zertifizierungsstelle kann unten ergänzt werden."));
    widgets::label(ui, t("SMTP-Server (Name:Port)"));
    ui.next_size(360, 0);
    ui.textbox(ID_F + 52, Some("smtp.example.org:587"), &mut app.settings.smtp_host, 128);
    widgets::label(ui, t("Verschlüsselung"));
    widgets::inline_begin(ui);
    ui.next_size(480, 0);
    ui.segmented(ID_F + 105, &mut app.settings.smtp_security, &[t("STARTTLS (587)"), t("TLS (465)"), t("Keine (25)")]);
    widgets::inline_end(ui);
    if app.settings.smtp_security == 2 {
        widgets::hint(ui, t("Ohne Verschlüsselung meldet sich BackupCraft nicht an – nur für Relays im eigenen Netz."));
    }
    ui.box_begin(0, BOX_H, 16);
    ui.box_begin(0, BOX_V, 4);
    widgets::label(ui, t("Benutzer (optional)"));
    ui.next_size(260, 0);
    ui.textbox(ID_F + 56, None, &mut app.settings.smtp_user, 256);
    ui.box_end();
    ui.box_begin(0, BOX_V, 4);
    widgets::label(ui, t("Passwort"));
    ui.next_size(260, 0);
    ui.textbox(ID_F + 57, None, &mut app.settings.smtp_pass, 256 | TB_PASSWORD);
    ui.box_end();
    ui.box_end();
    widgets::label(ui, t("Absender"));
    ui.next_size(360, 0);
    ui.textbox(ID_F + 58, Some("backupcraft@example.org"), &mut app.settings.smtp_from, 256);
    widgets::label(ui, t("Empfänger (mehrere mit Komma)"));
    ui.next_size(360, 0);
    ui.textbox(ID_F + 53, Some("it@example.org"), &mut app.settings.smtp_to, 512);
    widgets::label(ui, t("Eigene Zertifizierungsstelle (PEM, optional)"));
    ui.next_size(-1, 90);
    ui.textbox(ID_F + 59, Some("-----BEGIN CERTIFICATE-----"), &mut app.settings.smtp_ca, TB_MULTI | 16384);
    widgets::card_end(ui);

    widgets::card_begin(ui, t("Web-Hook"), t("POST mit JSON an diese Adresse (Teams, Slack und Mattermost nehmen das Feld „text“)."));
    ui.next_size(-1, 0);
    ui.textbox(ID_F + 60, Some("https://hooks.example.org/services/…"), &mut app.settings.webhook, 1024);
    widgets::card_end(ui);

    widgets::card_begin(ui, t("Versand"), "");
    if let Some(st) = app.net.data.notify.as_ref().filter(|s| s.last_at > 0) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.status_dot(if st.last_ok { BADGE_OK } else { BADGE_CRIT });
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &format!("{} · {} · {}", fmt::when(st.last_at, &app.zone), st.last_what, st.last_message));
        ui.box_end();
    } else {
        ui.text(TS_BODY | TX_SECONDARY, t("Noch nichts versendet."));
    }
    ui.box_begin(0, BOX_H, 8);
    if ui.accent_button(ID_SAVE + 4, t("Speichern")) & EV_ACT != 0 {
        let s = &app.settings;
        let pairs = [
            ("notify.mode", String::from(["off", "failure", "all"][s.notify.clamp(0, 2) as usize])),
            ("notify.daily", if s.daily_on != 0 { format!("{:02}:{:02}", s.daily_h.clamp(0, 23), s.daily_m.clamp(0, 59)) } else { String::new() }),
            ("notify.webhook", String::from(s.webhook.trim())),
            ("smtp.host", String::from(s.smtp_host.trim())),
            ("smtp.security", String::from(["starttls", "tls", "none"][s.smtp_security.clamp(0, 2) as usize])),
            ("smtp.user", String::from(s.smtp_user.trim())),
            ("smtp.password", s.smtp_pass.clone()),
            ("smtp.from", String::from(s.smtp_from.trim())),
            ("smtp.to", String::from(s.smtp_to.trim())),
            ("smtp.ca", String::from(s.smtp_ca.trim())),
        ];
        save(app, &pairs);
    }
    ui.next_flags(if available { 0 } else { ST_DISABLED });
    if ui.button(ID_F + 54, t("Testnachricht senden")) & EV_ACT != 0 {
        app.net.action(Method::Post, "/api/settings/test-email", None, "settings.test", &["/api/notify"], After::Nothing);
    }
    ui.next_flags(if available { 0 } else { ST_DISABLED });
    if ui.button(ID_F + 61, t("Zusammenfassung jetzt senden")) & EV_ACT != 0 {
        app.net.action(Method::Post, "/api/notify/summary", None, "notify.summary", &["/api/notify"], After::Nothing);
    }
    ui.box_end();
    widgets::hint(ui, t("Speichern Sie Änderungen vor dem Test: der Server sendet mit den gespeicherten Einstellungen."));
    widgets::card_end(ui);
}

fn about(app: &mut Console, ui: &mut Ui, admin: bool) {
    widgets::card_begin(ui, "", "");
    ui.box_begin(0, BOX_H | BOX_XCENTER, 14);
    crate::shell::logo(ui, 48);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_TITLE, "BackupCraft");
    ui.text(TS_BODY | TX_SECONDARY, &tf("Version {} · gebaut mit JFLib", &[bc_proto::VERSION]));
    ui.box_end();
    ui.box_end();
    let server = app.net.data.info.as_ref().map_or(String::from("–"), |i| format!("{} · {}", i.name, i.version));
    widgets::props(ui, &[
        (t("Server"), &server),
        (t("Konsole"), bc_proto::VERSION),
        (t("Speicherformat"), "bcrepo-1 (FastCDC, BLAKE3, LZ4 / zstd, XChaCha20-Poly1305)"),
        (t("Transport"), "TLS 1.3 (X25519, Ed25519)"),
    ]);
    widgets::card_end(ui);
    updates(app, ui, admin);
    widgets::card_begin(ui, t("Lizenzen"), "");
    ui.text(TS_BODY | TX_WRAP, t("Schrift: Inter (SIL Open Font License 1.1). Symbole: eigene Vektorgrafiken von BackupCraft und jfOffice; die jfOffice-Motive gehen auf WordCraft zurück (MIT-Lizenz). Das Programm enthält Code von Drittanbietern, darunter Rust-Bibliotheken wie HarfRust und Portierungen aus libjpeg-turbo, libwebp und libjxl. Diese Software beruht teilweise auf der Arbeit der Independent JPEG Group. Alle Hinweise und Lizenztexte liegen dem Programm bei (THIRD_PARTY_NOTICES.md und Ordner „licenses“)."));
    widgets::card_end(ui);
    let _ = Json::to_json(&Setting::default());
}

/// the update channel: this version, the newest one, check and download; the channel settings
fn updates(app: &mut Console, ui: &mut Ui, admin: bool) {
    if !app.net.data.capable(bc_proto::api::CAP_UPDATE) {
        return;
    }
    app.net.want("/api/update", 60_000, ui.now());
    widgets::card_begin(ui, t("Updates"), t("Neue Versionen kommen aus einem Update-Kanal; jedes Manifest ist vom Herausgeber signiert, jedes Paket per SHA-256 geprüft."));
    let st = app.net.data.update.clone().unwrap_or_default();
    let latest = if st.latest.is_empty() { String::from(t("noch nicht gesucht")) } else if st.available { tf("{} – verfügbar", &[&st.latest]) } else { tf("{} – aktuell", &[&st.latest]) };
    let checked = if st.checked > 0 { fmt::when(st.checked, &app.zone) } else { String::from("–") };
    widgets::props(ui, &[
        (t("Diese Version"), bc_proto::VERSION),
        (t("Neueste Version"), &latest),
        (t("Zuletzt gesucht"), &checked),
    ]);
    if !st.error.is_empty() {
        ui.infobar(ID_F + 80, IB_ERROR, t("Update-Kanal"), &st.error);
    }
    if st.available && !st.notes.is_empty() {
        ui.text(TS_BODY_STRONG, t("Neu in dieser Version"));
        ui.text(TS_BODY | TX_WRAP, &st.notes);
    }
    if !st.downloaded.is_empty() {
        let how = if st.platform.starts_with("windows") {
            t("Zum Installieren das Setup als Administrator ausführen; es ersetzt die Programme und startet die Dienste neu.")
        } else {
            t("Zum Installieren: sudo apt install ./<Paket> – der Dienst startet danach neu.")
        };
        ui.infobar(ID_F + 81, IB_SUCCESS, t("Heruntergeladen und geprüft"), &format!("{} · {}", st.downloaded, how));
    }
    if admin {
        ui.box_begin(0, BOX_H, 8);
        ui.next_flags(if st.channel.is_empty() { ST_DISABLED } else { 0 });
        if ui.button(ID_F + 82, t("Jetzt suchen")) & EV_ACT != 0 {
            app.net.action(Method::Post, "/api/update/check", None, "update.check", &["/api/update", "/api/summary"], After::Nothing);
        }
        ui.next_flags(if st.available && !st.file.is_empty() && st.downloaded.is_empty() { ST_ACCENT } else { ST_DISABLED });
        if ui.button(ID_F + 83, &tf("Herunterladen ({})", &[&fmt::size(st.size)])) & EV_ACT != 0 {
            app.net.action(Method::Post, "/api/update/download", None, "update.download", &["/api/update"], After::Operation);
        }
        ui.box_end();
        ui.separator();
        widgets::label(ui, t("Update-Kanal (Adresse von manifest.json)"));
        ui.next_size(-1, 0);
        ui.textbox(ID_F + 84, Some("https://updates.example.org/backupcraft/manifest.json"), &mut app.settings.upd_url, 1024);
        widgets::label(ui, t("Schlüssel des Herausgebers (Ed25519, hex)"));
        ui.next_size(-1, 0);
        ui.textbox(ID_F + 85, None, &mut app.settings.upd_key, 512);
        ui.switch(ID_F + 86, t("Täglich automatisch suchen"), &mut app.settings.upd_auto);
        widgets::inline_begin(ui);
        if ui.accent_button(ID_SAVE + 6, t("Speichern")) & EV_ACT != 0 {
            let s = &app.settings;
            let pairs = [
                ("update.url", String::from(s.upd_url.trim())),
                ("update.key", String::from(s.upd_key.trim())),
                ("update.auto", String::from(if s.upd_auto != 0 { "true" } else { "false" })),
            ];
            save(app, &pairs);
        }
        widgets::inline_end(ui);
    }
    widgets::card_end(ui);
}
