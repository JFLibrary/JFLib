// app -- the console's state and frame: login or the shell (title bar, navigation, the page
// or an open wizard, activity bar), events of the connection as toasts, refresh timer,
// keyboard shortcuts, settings file
//
// Command line: --fixture (demo server), --server URL [--ca FILE], --user NAME, --page NAME
// (dashboard jobs restore hosts repositories credentials replication tape recovery monitoring
// settings), --wizard job|restore|repository|pair|ssh, --lang de|en, --config DIR.

use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::Method;
use bc_proto::j::{self, Json};
use bc_proto::model::*;
use datetime::Zone;
use ui::*;

use crate::fixture::FIX_EPOCH;
use crate::net::{After, Event, Link, Net};
use crate::tr::{self, t, tf};
use crate::{icons, shell, views, wizards};

// widget id ranges of the parts (each part numbers its widgets inside its range; grids use
// part + 0x8_0000 and the rows after it)
pub const ID_SHELL: u32 = 0x0010_0000;
pub const ID_DASH: u32 = 0x0020_0000;
pub const ID_JOBS: u32 = 0x0030_0000;
pub const ID_RESTORE: u32 = 0x0040_0000;
pub const ID_HOSTS: u32 = 0x0050_0000;
pub const ID_REPOS: u32 = 0x0060_0000;
pub const ID_CREDS: u32 = 0x0070_0000;
pub const ID_STAGED: u32 = 0x0080_0000;
pub const ID_MON: u32 = 0x0090_0000;
pub const ID_SET: u32 = 0x00A0_0000;
pub const ID_WIZ: u32 = 0x00B0_0000;
pub const ID_REPL: u32 = 0x00C0_0000;
pub const ID_TAPE: u32 = 0x0120_0000;
pub const ID_RECOVERY: u32 = 0x0130_0000;
pub const ID_DLG: u32 = 0x00D0_0000;
pub const ID_LOGIN: u32 = 0x00E0_0000;

const TIMER_POLL: u32 = 1;
/// the demo server answered: take the answers before the frame is shown
const CMD_PUMP: u32 = 0x00F0_0001;

/// the pages of the navigation
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Dashboard,
    Jobs,
    Restore,
    Hosts,
    Repositories,
    Credentials,
    Replication,
    Tape,
    Recovery,
    Monitoring,
    Settings,
}

impl Page {
    pub fn parse(s: &str) -> Option<Page> {
        Some(match s {
            "dashboard" | "overview" => Page::Dashboard,
            "jobs" => Page::Jobs,
            "restore" => Page::Restore,
            "hosts" | "infrastructure" => Page::Hosts,
            "repositories" | "repos" => Page::Repositories,
            "credentials" => Page::Credentials,
            "replication" => Page::Replication,
            "tape" => Page::Tape,
            "recovery" | "instant" => Page::Recovery,
            "monitoring" => Page::Monitoring,
            "settings" => Page::Settings,
            _ => return None,
        })
    }
}

/// the remembered preferences (console.ini)
#[derive(Clone)]
pub struct Prefs {
    pub lang: u32,
    pub theme: u32,
    pub server: String,
    pub user: String,
    pub recent: Vec<String>,
    pub nav_compact: bool,
    /// trusted server CAs: "host:port" -> SHA-256 (hex)
    pub pins: alloc::collections::BTreeMap<String, String>,
    pub path: Option<String>,
    /// console.ini did not exist: the first start (the setup assistant opens)
    pub first_run: bool,
}

pub struct Console {
    pub net: Net,
    pub page: Page,
    pub nav: NavState,
    pub zone: Zone,
    pub prefs: Prefs,
    pub login: views::login::State,
    pub jobs: views::jobs::State,
    pub restore: views::restore::State,
    pub hosts: views::hosts::State,
    pub repos: views::repos::State,
    pub creds: views::creds::State,
    pub repl: views::replication::State,
    pub tape: views::tape::State,
    pub recovery: views::recovery::State,
    pub mon: views::monitoring::State,
    pub settings: views::settings::State,
    pub wizard: Option<wizards::Wizard>,
    pub dlg: views::dialogs::Dialogs,
    pub setup: views::setup::State,
    pub search: String,
    pub fixture: bool,
    pub logged_in: bool,
    started: bool,
    start_wizard: Option<String>,
    epoch_ms0: u32,
}

/// ini file of the preferences
fn prefs_load(path: Option<String>) -> Prefs {
    let mut p = Prefs { lang: tr::LANG_DE, theme: 0, server: String::new(), user: String::new(), recent: Vec::new(), nav_compact: false, pins: Default::default(), path: None, first_run: false };
    let file = path.or_else(|| plat::config_dir().map(|d| alloc::format!("{}/BackupCraft/console.ini", d)));
    p.first_run = file.as_ref().is_some_and(|f| plat::file_exists(f) != 1);
    if let Some(f) = &file {
        let ini = base::ini::Ini::load(f);
        {
            p.lang = if ini.get("ui", "language") == Some("en") { tr::LANG_EN } else { tr::LANG_DE };
            p.theme = match ini.get("ui", "theme") {
                Some("dark") => 1,
                Some("system") => 2,
                _ => 0,
            };
            p.server = String::from(ini.get("connection", "server").unwrap_or(""));
            p.user = String::from(ini.get("connection", "user").unwrap_or(""));
            p.recent = ini.get("connection", "recent").unwrap_or("").split('|').filter(|s| !s.is_empty()).map(String::from).collect();
            p.nav_compact = ini.get("ui", "nav_compact") == Some("1");
            for (k, v) in ini.section("trust") {
                p.pins.insert(String::from(k), String::from(v));
            }
        }
    }
    p.path = file;
    p
}

impl Console {
    pub fn new() -> Console {
        let cfg = plat::arg_value("--config").map(|d| alloc::format!("{}/console.ini", d));
        // headless runs (screenshots, scripts) neither read nor change the user's settings
        let headless = plat::arg_value("--shot").is_some() || plat::arg_value("--script").is_some();
        let mut prefs = if headless && cfg.is_none() {
            Prefs { lang: tr::LANG_DE, theme: 0, server: String::new(), user: String::new(), recent: Vec::new(), nav_compact: false, pins: Default::default(), path: None, first_run: false }
        } else {
            prefs_load(cfg)
        };
        if let Some(l) = plat::arg_value("--lang") {
            prefs.lang = if l == "en" { tr::LANG_EN } else { tr::LANG_DE };
        }
        tr::init(prefs.lang);
        let fixture = plat::args().iter().any(|a| a == "--fixture");
        let server = plat::arg_value("--server").map(String::from).unwrap_or_else(|| prefs.server.clone());
        let net = if fixture { Net::fixture(FIX_EPOCH) } else { Net::offline() };
        let zone = Zone::by_name("Europe/Berlin").unwrap_or_else(Zone::utc);
        let mut c = Console {
            net,
            page: plat::arg_value("--page").and_then(|p| Page::parse(&p)).unwrap_or(Page::Dashboard),
            nav: NavState { compact: prefs.nav_compact },
            zone,
            login: views::login::State::new(&server, &prefs.user),
            jobs: Default::default(),
            restore: Default::default(),
            hosts: Default::default(),
            repos: Default::default(),
            creds: Default::default(),
            repl: Default::default(),
            tape: Default::default(),
            recovery: Default::default(),
            mon: Default::default(),
            settings: Default::default(),
            wizard: None,
            dlg: Default::default(),
            setup: views::setup::State::new(!fixture && (plat::args().iter().any(|a| a == "--setup") || (prefs.first_run && plat::arg_value("--server").is_none()))),
            search: String::new(),
            fixture,
            logged_in: false,
            started: false,
            start_wizard: plat::arg_value("--wizard").map(String::from),
            epoch_ms0: 0,
            prefs,
        };
        if fixture {
            // the demo logs in as admin at once
            let user = plat::arg_value("--user").map(String::from).unwrap_or_else(|| String::from("admin"));
            c.login.user = user.clone();
            c.net.action(Method::Post, "/api/auth/login", Some(j::obj([("username", Value::string(user)), ("password", Value::string("demo"))])),
                         "login", &[], After::Login);
        }
        c
    }

    /// open a wizard once the data it starts from is there (as --wizard NAME)
    pub fn start_wizard_named(&mut self, name: &str) {
        self.start_wizard = Some(String::from(name));
    }

    /// seconds since the epoch now (the demo: its fixed start plus the run time)
    pub fn epoch(&self, ui: &Ui) -> i64 {
        if self.fixture {
            return FIX_EPOCH + (ui.now().wrapping_sub(self.epoch_ms0) / 1000) as i64;
        }
        plat_epoch()
    }

    /// the signed-in user's role (Viewer when unknown)
    pub fn role(&self) -> Role {
        self.net.data.me.as_ref().map_or(Role::Viewer, |u| u.role)
    }

    /// whether the user may do something that needs `need`
    pub fn may(&self, need: Role) -> bool {
        self.role().allows(need)
    }

    pub fn save_prefs(&self) {
        let Some(path) = &self.prefs.path else { return };
        let mut ini = base::ini::Ini::load(path);
        ini.set("ui", "language", if self.prefs.lang == tr::LANG_EN { "en" } else { "de" });
        ini.set("ui", "theme", match self.prefs.theme {
            1 => "dark",
            2 => "system",
            _ => "light",
        });
        ini.set("ui", "nav_compact", if self.nav.compact { "1" } else { "0" });
        if !self.fixture {
            ini.set("connection", "server", &self.prefs.server);
            ini.set("connection", "user", &self.prefs.user);
            ini.set("connection", "recent", &self.prefs.recent.join("|"));
            for (k, v) in &self.prefs.pins {
                ini.set("trust", k, v);
            }
        }
        if let Some(dir) = path.rsplit_once(['/', '\\']).map(|(d, _)| d) {
            plat::mkdir(dir);
        }
        ini.save(path);
    }

    /// go to a page (closes nothing; a wizard stays open on top)
    pub fn go(&mut self, ui: &mut Ui, page: Page) {
        if self.page != page {
            self.page = page;
            ui.repass();
        }
    }

    /// refresh what the current page shows
    pub fn refresh(&mut self, ui: &mut Ui) {
        let now = ui.now();
        for p in ["/api/summary", "/api/jobs", "/api/hosts", "/api/repositories", "/api/sessions?limit=500", "/api/operations"] {
            self.net.get(p, now);
        }
        ui.toast(IB_INFO, t("Aktualisiert"), t("Die Daten wurden neu geladen."), 1800);
    }

    /// a logged-in session ended or was refused: back to the login
    pub fn logout(&mut self, ui: &mut Ui, reason: Option<&str>) {
        if !self.net.token.is_empty() {
            self.net.action(Method::Post, "/api/auth/logout", None, "logout", &[], After::Nothing);
        }
        self.net.reset_session();
        self.logged_in = false;
        self.wizard = None;
        if let Some(r) = reason {
            self.login.error = String::from(r);
        }
        ui.repass();
    }

    fn handle_events(&mut self, ui: &mut Ui) {
        let events = core::mem::take(&mut self.net.events);
        for e in events {
            match e {
                Event::Done { label, body, after } => self.done(ui, &label, body, after),
                Event::Failed { label, code, message } => {
                    if label == "login" || label == "setup" {
                        self.login.busy = false;
                        if code == "setup_required" {
                            // the password of the sign-in means nothing to a new server
                            self.login.setup = true;
                            self.login.password.clear();
                            self.login.error.clear();
                        } else if code == "untrusted" {
                            self.login.untrusted = self.net.presented();
                            self.login.error.clear();
                        } else {
                            self.login.error = message;
                        }
                        ui.repass();
                        continue;
                    }
                    if let Some(w) = &mut self.wizard {
                        if w.owns(&label) {
                            w.failed(&code, &message);
                        }
                    }
                    self.dlg.failed(&label, &message);
                    let title = match code.as_str() {
                        "forbidden" => t("Keine Berechtigung"),
                        "capability" => t("Noch nicht verfügbar"),
                        "immutable" => t("Unveränderlich"),
                        _ => t("Aktion fehlgeschlagen"),
                    };
                    ui.toast(IB_ERROR, title, &message, 6000);
                }
                Event::OpFinished(op) => {
                    let (sev, title) = match op.status {
                        OpStatus::Succeeded => (IB_SUCCESS, t("Abgeschlossen")),
                        OpStatus::Cancelled => (IB_WARNING, t("Abgebrochen")),
                        _ => (IB_ERROR, t("Fehlgeschlagen")),
                    };
                    let msg = if op.error.is_empty() { alloc::format!("{} – {}", op.title, op.result) } else { alloc::format!("{} – {}", op.title, op.error) };
                    ui.toast(sev, title, &msg, 6000);
                }
                Event::Unauthorized => {
                    if self.logged_in {
                        self.logout(ui, Some(t("Die Anmeldung ist abgelaufen. Bitte erneut anmelden.")));
                    }
                }
            }
        }
    }

    fn done(&mut self, ui: &mut Ui, label: &str, body: Value, after: After) {
        match after {
            After::Setup => {
                views::login::after_setup(self, ui);
                return;
            }
            After::Login => {
                if let Some(r) = bc_proto::api::LoginResponse::from_json(&body) {
                    self.net.token = r.token;
                    self.net.data.me = Some(r.user.clone());
                    self.net.link = Link::Online;
                    self.logged_in = true;
                    self.login.busy = false;
                    self.login.error.clear();
                    self.login.password.clear();
                    self.prefs.user = r.user.username.clone();
                    if !self.fixture {
                        let s = self.login.server.clone();
                        self.prefs.server = s.clone();
                        self.prefs.recent.retain(|x| *x != s);
                        self.prefs.recent.insert(0, s);
                        self.prefs.recent.truncate(6);
                        self.save_prefs();
                    }
                    let now = ui.now();
                    for p in ["/api", "/api/capabilities", "/api/summary", "/api/jobs", "/api/hosts", "/api/repositories"] {
                        self.net.get(p, now);
                    }
                    if !self.fixture {
                        ui.toast(IB_SUCCESS, t("Angemeldet"), &tf("Verbunden mit {}", &[&self.net.server]), 3000);
                    }
                    ui.repass();
                }
                return;
            }
            _ => {}
        }
        // the wizards and dialogs that wait for this answer
        let mut run = None;
        if let Some(w) = &mut self.wizard {
            if w.owns(label) {
                run = w.run_after(&body);
                let close = w.done(label, &body);
                if close || after == After::CloseWizard {
                    self.wizard = None;
                }
            }
        }
        if let Some(id) = run {
            self.net.action(Method::Post, &alloc::format!("/api/jobs/{}/run", id), Some(j::obj([("full", Value::Bool(false))])), "job.run",
                            &["/api/jobs"], After::Operation);
        }
        self.dlg.done(label, &body, &after);
        if label == "update.check" {
            if let Some(st) = UpdateStatus::from_json(&body) {
                let m = if st.available { tf("Version {} ist verfügbar.", &[&st.latest]) } else { alloc::string::String::from(t("BackupCraft ist aktuell.")) };
                ui.toast(IB_SUCCESS, t("Updates"), &m, 5000);
            }
            return;
        }
        if label == "fleet.check" {
            if let Some(s) = FleetServer::from_json(&body) {
                let sev = if matches!(s.status.as_str(), "ok" | "warning") { IB_SUCCESS } else { IB_WARNING };
                ui.toast(sev, &s.name, &alloc::format!("{} – {}", views::fleet::status(&s.status).0, s.message), 5000);
            }
            return;
        }
        if label == "settings.test" || label == "notify.summary" {
            let m = j::str_of(&body, "message");
            ui.toast(IB_SUCCESS, t("Gesendet"), m, 5000);
            return;
        }
        let msg = match label {
            "job.save" => Some(t("Der Job wurde gespeichert.")),
            "job.delete" => Some(t("Der Job wurde gelöscht. Seine Restore-Points bleiben erhalten.")),
            "job.enable" => Some(t("Der Job ist aktiviert.")),
            "job.disable" => Some(t("Der Job ist deaktiviert.")),
            "job.run" => Some(t("Der Job wurde gestartet.")),
            "restore.start" => Some(t("Die Wiederherstellung wurde gestartet.")),
            "point.verify" => Some(t("Die Prüfung des Restore-Points läuft.")),
            "repository.save" => Some(t("Das Repository wurde gespeichert.")),
            "host.save" => Some(t("Die Freigaben wurden gespeichert.")),
            "host.remove" => Some(t("Der Agent wurde entfernt und sein Zertifikat gesperrt.")),
            "settings.save" => Some(t("Die Einstellungen wurden gespeichert.")),
            "user.save" => Some(t("Der Benutzer wurde gespeichert.")),
            "credential.save" => Some(t("Die Zugangsdaten wurden gespeichert.")),
            "op.cancel" => Some(t("Der Vorgang wird abgebrochen.")),
            "replica.save" => Some(t("Das Replikat wurde gespeichert.")),
            "replica.delete" => Some(t("Das Replikat wurde ausgetragen. Die Dateien im Zielordner bleiben.")),
            "replica.failover" => Some(t("Failover aktiv: das Replikat ist in Gebrauch.")),
            "plan.save" => Some(t("Der Failover-Plan wurde gespeichert.")),
            "plan.delete" => Some(t("Der Failover-Plan wurde gelöscht.")),
            "fleet.save" => Some(t("Der Server wird überwacht.")),
            "tape.add" => Some(t("Die Bänder wurden angelegt.")),
            "tape.assign" => Some(t("Das Band wurde zugeordnet.")),
            "tape.remove" => Some(t("Das Band wurde entfernt.")),
            "pool.save" => Some(t("Der Medienpool wurde gespeichert.")),
            "pool.delete" => Some(t("Der Medienpool wurde gelöscht.")),
            "instant.stop" => Some(t("Die VM wurde beendet.")),
            "fleet.remove" => Some(t("Der Server wird nicht mehr überwacht.")),
            "fleet.revoke" => Some(t("Das Überwachungs-Token wurde widerrufen.")),
            _ => None,
        };
        if let Some(m) = msg {
            ui.toast(IB_SUCCESS, t("Erledigt"), m, 3500);
        }
    }

    fn shortcuts(&mut self, ui: &mut Ui) {
        if ui.modal_active() != 0 {
            return;
        }
        if ui.key(VK_F5) {
            self.refresh(ui);
        }
        if ui.mods() & MOD_CTRL != 0 && ui.key(b'K' as u32) {
            ui.focus(ID_SHELL + 20);
        }
        if ui.mods() & MOD_CTRL != 0 && ui.key(b'N' as u32) && self.wizard.is_none() && self.may(Role::Operator) {
            wizards::open_named(self, ui, "job");
        }
    }
}

/// seconds since the epoch from the platform clock
pub fn plat_epoch() -> i64 {
    (plat::unix_time_ns() / 1_000_000_000) as i64
}

use bc_proto::j::Value;

/// scripts' sync: answers outstanding, or the setup assistant's worker running
fn sync_pending() -> bool {
    crate::live::pending() || views::setup::pending()
}

impl App for Console {
    fn init(&mut self, ui: &mut Ui) -> bool {
        icons::install(ui);
        ui.sync_hook_set(Some(sync_pending));
        if plat::arg_value("--theme").is_none() {
            ui.theme_set(self.prefs.theme);
        }
        self.epoch_ms0 = ui.now();
        ui.timer(TIMER_POLL, 400);
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            self.net.pump(ui.now(), self.epoch(ui));
            self.handle_events(ui);
        }
        self.shortcuts(ui);
        if !self.logged_in {
            if self.setup.active {
                views::setup::frame(self, ui);
            } else {
                views::login::frame(self, ui);
            }
            ui.toast_host();
            if self.net.answered() {
                ui.cmd_run(CMD_PUMP, 0);
            }
            return;
        }
        views::setup::after_login(self, ui);
        // --wizard NAME: once the data it starts from is there
        if self.start_wizard.is_some() && self.net.loaded("/api/jobs") && self.net.loaded("/api/hosts") {
            if let Some(w) = self.start_wizard.take() {
                wizards::open_named(self, ui, &w);
            }
        }
        shell::frame(self, ui);
        views::dialogs::frame(self, ui);
        ui.toast_host();
        if self.net.answered() {
            ui.cmd_run(CMD_PUMP, 0);
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_TIMER if arg == TIMER_POLL => {
                let busy = self.net.pump(ui.now(), self.epoch(ui));
                self.handle_events(ui);
                ui.timer(TIMER_POLL, if busy { 400 } else { 1500 });
                ui.invalidate();
                true
            }
            UIC_WAKE | CMD_PUMP => {
                self.net.pump(ui.now(), self.epoch(ui));
                self.handle_events(ui);
                ui.invalidate();
                true
            }
            UIC_THEME => {
                ui.invalidate();
                false
            }
            UIC_EXIT => {
                self.save_prefs();
                false
            }
            _ => false,
        }
    }
}
