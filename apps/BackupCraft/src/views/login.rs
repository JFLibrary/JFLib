// login -- connect and sign in: a branded panel at the left, the sign-in card at the right
// (server with the recent ones, user, password), the demo server one click away
//
// A server whose CA the console does not know yet (https) shows its CA and fingerprint; once
// trusted, the fingerprint is kept per server ("host:port") in console.ini ([trust]).  A
// changed CA is shown as a warning and must be trusted again.  --ca FILE trusts a CA file.
//
// A server that is not set up yet (its answer: setup_required) turns the card into the first
// setup: the one-time setup token, the first administrator, the server's name; then the
// console signs in with that account.

use alloc::string::String;

use bc_proto::api::Method;
use bc_proto::j::{self, Value};
use netio::trust::{fingerprint_text, Presented, ServerTrust};
use ui::*;

use crate::app::{Console, ID_LOGIN};
use crate::icons::*;
use crate::net::{After, Net};
use crate::tr::{t, tf};
use crate::widgets;

pub struct State {
    pub server: String,
    pub user: String,
    pub password: String,
    pub busy: bool,
    pub error: String,
    /// the server's CA, waiting for the user's decision
    pub untrusted: Option<Presented>,
    /// the server is new: the first setup instead of the sign-in
    pub setup: bool,
    pub token: String,
    pub password2: String,
    pub server_name: String,
}

impl State {
    pub fn new(server: &str, user: &str) -> State {
        State {
            server: String::from(if server.is_empty() { "https://localhost:7642" } else { server }),
            user: String::from(user),
            password: String::new(),
            busy: false,
            error: String::new(),
            untrusted: None,
            setup: false,
            token: String::new(),
            password2: String::new(),
            server_name: String::new(),
        }
    }
}

const ID_SERVER: u32 = ID_LOGIN + 1;
const ID_RECENT: u32 = ID_LOGIN + 2;
const ID_USER: u32 = ID_LOGIN + 3;
const ID_PW: u32 = ID_LOGIN + 4;
const ID_GO: u32 = ID_LOGIN + 5;
const ID_DEMO: u32 = ID_LOGIN + 6;
const ID_TRUST: u32 = ID_LOGIN + 10; // + 1: its close button
const ID_TRUST_GO: u32 = ID_LOGIN + 12;
const ID_TRUST_NO: u32 = ID_LOGIN + 13;
const ID_TOKEN: u32 = ID_LOGIN + 14;
const ID_PW2: u32 = ID_LOGIN + 15;
const ID_SNAME: u32 = ID_LOGIN + 16;
const ID_SETUP_GO: u32 = ID_LOGIN + 17;
const ID_SETUP_BACK: u32 = ID_LOGIN + 18;
const ID_REC: u32 = ID_LOGIN + 20;
const ID_SETUP_WIZ: u32 = ID_LOGIN + 19;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 0);
    brand(ui);
    card(app, ui);
    ui.box_end();
}

/// the branded panel at the left (also the setup assistant's)
pub fn brand(ui: &mut Ui) {
    ui.next_size(-2, -1);
    ui.box_begin(0, BOX_V | BOX_CLIP, 0);
    let r = ui.box_rect();
    let a = ui.color(T_ACCENT);
    let deep = color_mix(a | 0xFF00_0000, 0xFF06_1218, 110);
    ui.draw_color(deep);
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    // soft accent glow
    ui.draw_color((a & 0xFF_FFFF) | 0x3000_0000);
    ui.draw_circle((r[0] + r[2] * 3 / 4) << 8, (r[1] + r[3] / 4) << 8, ui.dp(260) << 8, 0);
    ui.draw_color((a & 0xFF_FFFF) | 0x2000_0000);
    ui.draw_circle((r[0] + r[2] / 6) << 8, (r[1] + r[3] * 5 / 6) << 8, ui.dp(200) << 8, 0);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CENTER | pad(56), 18);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 14);
    let s = ui.dp(44);
    let lr = ui.place(s, s);
    ui.cmd_rrect(lr[0], lr[1], s, s, ui.dp(11), 0, 0xFFFF_FFFF, 0, 0);
    ui.draw_color(deep);
    ui.draw_icon(BC_SHIELD_CHECK, lr[0] + ui.dp(8), lr[1] + ui.dp(8), ui.dp(28) << 8);
    ui.text(TS_TITLE | TX_ON_ACCENT, "BackupCraft");
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_ON_ACCENT | TX_WRAP, t("Sicherung und Wiederherstellung für Server, Agenten und Dateien – dedupliziert, verschlüsselt, nachvollziehbar."));
    for (icon, line) in [
        (BC_SHIELD, t("Signierte, unveränderliche Restore-Points")),
        (IC_LOCK, t("Ende-zu-Ende verschlüsselt mit eigenem Recovery-Kit")),
        (BC_AGENT, t("Agenten für Windows und Linux, gekoppelt per Code")),
        (BC_CLOCK, t("Zeitpläne in jeder Zeitzone, sicher über die Zeitumstellung")),
    ] {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.draw_color(0xD0FF_FFFF);
        let s = ui.dp(18);
        let ir = ui.place(s, s);
        ui.draw_icon(icon, ir[0], ir[1], s << 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ON_ACCENT | TX_WRAP, line);
        ui.box_end();
    }
    ui.box_end();
    ui.box_end();
}

/// the sign-in card at the right
fn card(app: &mut Console, ui: &mut Ui) {
    ui.next_size(-3, -1);
    ui.box_begin(0, BOX_V | BOX_BG, 0);
    ui.titlebar_begin_overlay(None, 0);
    ui.titlebar_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
    ui.next_size(420, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(28), 12);
    if app.login.setup {
        setup_panel(app, ui);
        ui.box_end();
        ui.box_end();
        ui.box_end();
        return;
    }
    ui.text(TS_TITLE, t("Anmelden"));
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Mit einem BackupCraft-Server verbinden."));
    ui.spacer(4);
    widgets::label(ui, t("Server"));
    ui.box_begin(0, BOX_H, 6);
    ui.next_size(-1, 0);
    ui.textbox_icon(ID_SERVER, IC_GLOBE, Some("https://backup.example:7642"), &mut app.login.server, 200);
    let recent = app.prefs.recent.clone();
    ui.next_flags(ST_ICON_ONLY | if recent.is_empty() { ST_DISABLED } else { 0 });
    ui.next_icon(IC_HISTORY);
    ui.dropdown_button(ID_RECENT, t("Zuletzt verwendet"));
    if ui.flyout_begin(ID_RECENT, FL_ALIGN_END) {
        for (i, s) in recent.iter().enumerate() {
            if ui.menu_item(ID_REC + i as u32, s) & EV_ACT != 0 {
                app.login.server = s.clone();
            }
        }
    }
    ui.flyout_end();
    ui.box_end();
    widgets::label(ui, t("Benutzer"));
    ui.next_size(-1, 0);
    ui.textbox_icon(ID_USER, IC_PERSON, Some("admin"), &mut app.login.user, 64);
    widgets::label(ui, t("Passwort"));
    ui.next_size(-1, 0);
    let pev = ui.textbox(ID_PW, None, &mut app.login.password, 256 | TB_PASSWORD);
    if !app.login.error.is_empty() {
        let e = app.login.error.clone();
        widgets::notice(ui, IB_ERROR, t("Anmeldung nicht möglich"), &e);
    }
    if let Some(p) = app.login.untrusted.clone() {
        trust_panel(app, ui, &p);
        ui.box_end();
        ui.box_end();
        ui.box_end();
        return;
    }
    ui.spacer(4);
    let ready = !app.login.server.trim().is_empty() && !app.login.user.trim().is_empty() && !app.login.password.is_empty() && !app.login.busy;
    ui.next_size(-1, 36);
    ui.next_flags(if ready { 0 } else { ST_DISABLED });
    let go = ui.accent_button(ID_GO, if app.login.busy { t("Verbinde …") } else { t("Anmelden") }) & EV_ACT != 0;
    if (go || enter(pev)) && ready {
        connect(app, ui);
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Noch kein Server?"));
    if ui.link(ID_SETUP_WIZ, t("Einrichtungsassistent"), TS_CAPTION) & EV_ACT != 0 {
        app.setup = crate::views::setup::State::new(true);
        ui.repass();
    }
    ui.text(TS_CAPTION | TX_TERTIARY, "·");
    if ui.link(ID_DEMO, t("Demo mit Beispieldaten"), TS_CAPTION) & EV_ACT != 0 {
        demo(app, ui);
    }
    ui.box_end();
    ui.box_end();
    ui.spacer(14);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_CENTER, &tf("BackupCraft {} · Erstanmeldung mit dem Setup-Token des Servers", &[bc_proto::VERSION]));
    ui.box_end();
    ui.box_end();
}

/// Enter in a text box (EV_COMMIT also comes when the focus leaves it)
fn enter(ev: u32) -> bool {
    ev & (EV_COMMIT | EV_FOCUS) == EV_COMMIT | EV_FOCUS
}

/// the first setup of a new server: token, first administrator, name
fn setup_panel(app: &mut Console, ui: &mut Ui) {
    ui.text(TS_TITLE, t("Server einrichten"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &tf("{} ist noch nicht eingerichtet. Legen Sie den ersten Administrator an. Das Setup-Token zeigt der Server beim ersten Start; es steht auch in der Datei setup.token in seinem Datenordner.", &[&app.net.server]));
    ui.spacer(4);
    widgets::label(ui, t("Setup-Token"));
    ui.next_size(-1, 0);
    ui.textbox_icon(ID_TOKEN, IC_LOCK, None, &mut app.login.token, 128);
    widgets::label(ui, t("Administrator"));
    ui.next_size(-1, 0);
    ui.textbox_icon(ID_USER, IC_PERSON, Some("admin"), &mut app.login.user, 64);
    widgets::label(ui, t("Passwort (mindestens 12 Zeichen)"));
    ui.next_size(-1, 0);
    ui.textbox(ID_PW, None, &mut app.login.password, 256 | TB_PASSWORD);
    widgets::label(ui, t("Passwort wiederholen"));
    ui.next_size(-1, 0);
    let pev = ui.textbox(ID_PW2, None, &mut app.login.password2, 256 | TB_PASSWORD);
    widgets::label(ui, t("Name des Servers (optional)"));
    ui.next_size(-1, 0);
    ui.textbox(ID_SNAME, Some(&plat::net::hostname()), &mut app.login.server_name, 120);
    let l = &app.login;
    let hint = if l.password.chars().count() < 12 && !l.password.is_empty() {
        t("Das Passwort ist kürzer als 12 Zeichen.")
    } else if !l.password2.is_empty() && l.password != l.password2 {
        t("Die Passwörter stimmen nicht überein.")
    } else {
        ""
    };
    if !hint.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, hint);
    }
    if !app.login.error.is_empty() {
        let e = app.login.error.clone();
        widgets::notice(ui, IB_ERROR, t("Einrichtung nicht möglich"), &e);
    }
    let l = &app.login;
    let ready = !l.busy && !l.token.trim().is_empty() && !l.user.trim().is_empty() && l.password.chars().count() >= 12 && l.password == l.password2;
    ui.spacer(4);
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(-1, 36);
    ui.next_flags(if ready { 0 } else { ST_DISABLED });
    let go = ui.accent_button(ID_SETUP_GO, if app.login.busy { t("Richte ein …") } else { t("Einrichten und anmelden") }) & EV_ACT != 0;
    if (go || enter(pev)) && ready {
        let l = &app.login;
        let body = j::obj([
            ("token", Value::string(l.token.trim())),
            ("username", Value::string(l.user.trim())),
            ("password", Value::string(l.password.as_str())),
            ("server_name", Value::string(l.server_name.trim())),
        ]);
        app.login.busy = true;
        app.login.error.clear();
        app.net.action(Method::Post, "/api/setup", Some(body), "setup", &[], After::Setup);
        ui.repass();
    }
    ui.next_size(0, 36);
    if ui.button(ID_SETUP_BACK, t("Zurück")) & EV_ACT != 0 {
        app.login.setup = false;
        app.login.error.clear();
        ui.repass();
    }
    ui.box_end();
}

/// after the setup: sign in with the new administrator
pub fn after_setup(app: &mut Console, ui: &mut Ui) {
    app.login.setup = false;
    app.login.token.clear();
    app.login.password2.clear();
    let body = j::obj([("username", Value::string(app.login.user.trim())), ("password", Value::string(app.login.password.as_str()))]);
    app.net.action(Method::Post, "/api/auth/login", Some(body), "login", &[], After::Login);
    ui.toast(IB_SUCCESS, t("Server eingerichtet"), &tf("{} ist Administrator.", &[app.login.user.trim()]), 4000);
    ui.repass();
}

/// the CA of a server the console does not trust yet: compare, then trust or go back
fn trust_panel(app: &mut Console, ui: &mut Ui, p: &Presented) {
    let (sev, title, text) = if p.changed {
        (IB_ERROR, t("Zertifizierungsstelle geändert"), t("Dieser Server weist sich mit einer anderen Zertifizierungsstelle aus als bisher. Das ist nach einer Neuinstallation normal – oder jemand sitzt zwischen Console und Server. Nur vertrauen, wenn der Fingerabdruck stimmt."))
    } else {
        (IB_WARNING, t("Unbekannter Server"), t("Der Server verwendet eine eigene Zertifizierungsstelle. Vergleichen Sie den Fingerabdruck mit der Ausgabe von „bc-server ca“ auf dem Server oder mit dem Dialog „Agent koppeln“."))
    };
    ui.next_size(-1, 0);
    if ui.infobar(ID_TRUST, sev, title, "") & EV_ACT != 0 {
        app.login.untrusted = None;
        ui.repass();
        return;
    }
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, text);
    widgets::label(ui, t("Zertifizierungsstelle"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &p.name);
    widgets::label(ui, t("Fingerabdruck (SHA-256)"));
    let fp = fingerprint_text(&p.fingerprint);
    // two lines of 16 bytes each
    let half = fp.len() / 2 + 1;
    ui.text(TS_BODY_STRONG, &fp[..half.min(fp.len())].trim_end_matches(':'));
    ui.text(TS_BODY_STRONG, &fp[half.min(fp.len())..]);
    ui.spacer(4);
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(-1, 36);
    if ui.accent_button(ID_TRUST_GO, t("Vertrauen und anmelden")) & EV_ACT != 0 {
        if let Some(key) = trust_key(&app.login.server) {
            app.prefs.pins.insert(key, crypto::hex(&p.fingerprint));
            app.save_prefs();
        }
        app.login.untrusted = None;
        connect(app, ui);
    }
    ui.next_size(0, 36);
    if ui.button(ID_TRUST_NO, t("Abbrechen")) & EV_ACT != 0 {
        app.login.untrusted = None;
        ui.repass();
    }
    ui.box_end();
}

/// the key of a server in [trust]: "host:port"
fn trust_key(server: &str) -> Option<String> {
    crate::live::server_url(server).map(|u| u.host_header())
}

fn connect(app: &mut Console, ui: &mut Ui) {
    let server = String::from(app.login.server.trim());
    let ca = plat::arg_value("--ca").and_then(|p| plat::read_file(&p)).and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE")).and_then(|d| netio::x509::Cert::parse(&d));
    let pin = trust_key(&server).and_then(|k| app.prefs.pins.get(&k).cloned()).and_then(|h| netio::trust::parse_fingerprint(&h));
    let trust = match (ca, pin) {
        (Some(c), _) => ServerTrust::Ca(c),
        (None, Some(p)) => ServerTrust::Pin(p),
        (None, None) => ServerTrust::Ask,
    };
    app.net = Net::live(&server, trust);
    app.login.untrusted = None;
    app.fixture = false;
    app.login.busy = true;
    app.login.error.clear();
    let body = j::obj([("username", Value::string(app.login.user.trim())), ("password", Value::string(app.login.password.as_str()))]);
    app.net.action(Method::Post, "/api/auth/login", Some(body), "login", &[], After::Login);
    ui.repass();
}

fn demo(app: &mut Console, ui: &mut Ui) {
    app.net = Net::fixture(crate::fixture::FIX_EPOCH);
    app.fixture = true;
    app.login.busy = true;
    app.login.error.clear();
    app.login.user = String::from("admin");
    let body = j::obj([("username", Value::string("admin")), ("password", Value::string("demo"))]);
    app.net.action(Method::Post, "/api/auth/login", Some(body), "login", &[], After::Login);
    ui.repass();
}
