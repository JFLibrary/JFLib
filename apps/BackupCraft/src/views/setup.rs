// setup -- the setup assistant of the console: what this computer is to be, then everything
// that takes (the first start after the installation, --setup, or the link on the sign-in page)
//
//   backup server here    data folder, reachable from this computer or the network, the first
//                         administrator, a first repository -> bc-server service install with
//                         administrator rights (one UAC / pkexec prompt; --report hands the
//                         setup token and the CA's fingerprint back), the first setup through
//                         the API, sign-in, the repository, then the job wizard
//   existing server       the sign-in page
//   agent                 the server's address, pairing code and fingerprint (its console's
//                         "Agent koppeln"), the shared folders -> bc-agent pair --service with
//                         administrator rights
//
// bc-server and bc-agent are found next to the console (the setup and the .deb put them there).
//
// Tests (tests/setup_smoke.py): BC_SETUP_NO_SERVICE=1 starts the server as a child of the console
// instead of installing the service (no administrator rights, it ends with the console);
// BC_SETUP_PORT=N uses another port than 7642.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::j::{self, Json, Value};
use netio::trust::ServerTrust;
use plat::sync::Mutex;
use ui::*;

use crate::app::{Console, ID_LOGIN};
use crate::icons::*;
use crate::net::{After, Net};
use crate::tr::{t, tf};
use crate::widgets;

const ID: u32 = ID_LOGIN + 0x1000;

/// the server's port (BC_SETUP_PORT in tests)
fn port() -> u16 {
    plat::env("BC_SETUP_PORT").and_then(|p| p.parse().ok()).unwrap_or(bc_proto::DEFAULT_PORT)
}

/// the server started by a test run (BC_SETUP_NO_SERVICE), handed from the worker to State::child
static CHILD: Mutex<Option<plat::process::Child>> = Mutex::new(None);

/// a worker runs (scripts' sync waits for it)
static WORKING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn pending() -> bool {
    WORKING.load(core::sync::atomic::Ordering::SeqCst)
}

/// what a worker thread found
#[derive(Clone, Debug, Default)]
struct Outcome {
    ok: bool,
    message: String,
    token: String,
    fingerprint: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Phase {
    /// the questions
    #[default]
    Ask,
    /// the service is being installed (the worker runs)
    Installing,
    /// the API's first setup and the sign-in run
    SettingUp,
    /// the repository is being created (after the sign-in)
    Repository,
    Done,
    Failed,
}

#[derive(Default)]
pub struct State {
    pub active: bool,
    role: i32,
    step: usize,
    // the server here
    data: String,
    network: i32,
    names: String,
    user: String,
    pw1: String,
    pw2: String,
    server_name: String,
    repo_on: i32,
    repo_name: String,
    repo_path: String,
    // an agent
    a_server: String,
    a_code: String,
    a_fp: String,
    a_roots: String,
    a_restore: String,
    // the run
    phase: Phase,
    work: Option<Arc<Mutex<Option<Outcome>>>>,
    message: String,
    url: String,
    /// the test run's server: it ends with the console (dropping it ends the process)
    child: Option<plat::process::Child>,
}

fn default_server_data() -> String {
    if cfg!(windows) {
        let base = plat::env("ProgramData").unwrap_or_else(|| String::from("C:\\ProgramData"));
        plat::fs::join(&plat::fs::join(&base, "BackupCraft"), "server")
    } else {
        String::from("/var/lib/backupcraft")
    }
}

fn default_agent_data() -> String {
    if cfg!(windows) {
        let base = plat::env("ProgramData").unwrap_or_else(|| String::from("C:\\ProgramData"));
        plat::fs::join(&plat::fs::join(&base, "BackupCraft"), "agent")
    } else {
        String::from("/var/lib/backupcraft-agent")
    }
}

fn default_repo() -> String {
    if cfg!(windows) {
        let drive = if plat::fs::stat("D:\\").is_some() { "D:" } else { "C:" };
        format!("{}\\BackupCraft\\Repository", drive)
    } else {
        String::from("/srv/backupcraft/repository")
    }
}

impl State {
    pub fn new(active: bool) -> State {
        let home = plat::env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
        State {
            active,
            data: default_server_data(),
            names: plat::net::hostname(),
            user: String::from("admin"),
            server_name: plat::net::hostname(),
            repo_on: 1,
            repo_name: String::from("Primär"),
            repo_path: default_repo(),
            a_roots: if cfg!(windows) { plat::fs::join(&home, "Documents") } else { home.clone() },
            a_restore: if cfg!(windows) { String::from("C:\\BackupCraft\\Wiederherstellung") } else { plat::fs::join(&home, "Wiederhergestellt") },
            ..State::default()
        }
    }
}

/// a program next to the console
fn sibling(name: &str) -> Option<String> {
    let exe = plat::current_exe()?;
    let dir = plat::fs::parent(&exe);
    let p = plat::fs::join(dir, &format!("{}{}", name, if cfg!(windows) { ".exe" } else { "" }));
    (plat::file_exists(&p) == 1).then_some(p)
}

/// a report file the elevated program writes (made by this user, so it may delete it after)
fn report_file() -> String {
    let dir = plat::temp_dir().unwrap_or_else(|| String::from("."));
    let p = plat::fs::join(&dir, &format!("bc-setup-{:x}.json", plat::random_u64()));
    let _ = plat::write_file(&p, b"");
    p
}

fn read_report(path: &str) -> Option<Value> {
    let b = plat::read_file(path)?;
    plat::delete(path);
    j::parse(&b)
}

/// run `program args` with administrator rights in a thread; the outcome lands in the slot
fn start_worker(program: String, args: Vec<String>, report: String, wait_port: Option<u16>) -> Arc<Mutex<Option<Outcome>>> {
    let slot = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    WORKING.store(true, core::sync::atomic::Ordering::SeqCst);
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
        let refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
        let res = plat::process::run_elevated(&program, &refs);
        let mut o = match (&res, read_report(&report)) {
            (_, Some(v)) => Outcome {
                ok: v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false),
                message: String::from(j::str_of(&v, "message")),
                token: String::from(j::str_of(&v, "token")),
                fingerprint: String::from(j::str_of(&v, "ca_fingerprint")),
            },
            (Ok(code), None) => Outcome { ok: false, message: tf("Das Programm endete mit {} ohne Bericht.", &[&format!("{}", code)]), ..Outcome::default() },
            (Err(e), None) => Outcome { ok: false, message: e.clone(), ..Outcome::default() },
        };
        // the server listens a moment after its start
        if let (true, Some(port)) = (o.ok, wait_port) {
            let addr = plat::net::SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, port);
            let mut up = false;
            for _ in 0..150 {
                if plat::net::TcpStream::connect(&addr, 500).is_ok() {
                    up = true;
                    break;
                }
                plat::sleep_ms(200);
            }
            if !up {
                o.ok = false;
                o.message = tf("Der Dienst läuft, antwortet aber nicht auf Port {}.", &[&format!("{}", port)]);
            }
        }
        *s2.lock() = Some(o);
        WORKING.store(false, core::sync::atomic::Ordering::SeqCst);
        plat::wake();
    }));
    slot
}

/// the questions of each role: (step titles)
fn steps(role: i32) -> &'static [&'static str] {
    match role {
        0 => &["Rolle", "Speicherort", "Administrator", "Sicherungsziel", "Einrichten"],
        2 => &["Rolle", "Server", "Ordner", "Koppeln"],
        _ => &["Rolle"],
    }
}

fn valid(s: &State) -> &'static str {
    match (s.role, s.step) {
        (0, 1) if s.data.trim().is_empty() => t("Bitte einen Datenordner angeben."),
        (0, 2) if s.user.trim().is_empty() => t("Bitte einen Benutzernamen angeben."),
        (0, 2) if s.pw1.chars().count() < 12 => t("Das Passwort braucht mindestens 12 Zeichen."),
        (0, 2) if s.pw1 != s.pw2 => t("Die Passwörter stimmen nicht überein."),
        (0, 3) if s.repo_on != 0 && (s.repo_path.trim().is_empty() || s.repo_name.trim().is_empty()) => t("Bitte Name und Ordner des Repositorys angeben."),
        (2, 1) if s.a_server.trim().is_empty() || s.a_code.trim().is_empty() => t("Bitte Serveradresse und Pairing-Code angeben."),
        (2, 1) if netio::trust::parse_fingerprint(&s.a_fp).is_none() => t("Bitte den Fingerabdruck der Zertifizierungsstelle angeben (64 Hex-Zeichen)."),
        (2, 2) if s.a_roots.trim().is_empty() => t("Bitte mindestens einen Ordner angeben."),
        _ => "",
    }
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    progress(app, ui);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 0);
    crate::views::login::brand(ui);
    ui.next_size(-3, -1);
    ui.box_begin(0, BOX_V | BOX_BG, 0);
    ui.titlebar_begin_overlay(None, 0);
    ui.titlebar_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
    ui.next_size(520, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(28), 12);
    let st = &app.setup;
    let names = steps(st.role);
    // the steps
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            ui.text(TS_CAPTION | TX_TERTIARY, "›");
        }
        let style = if i == st.step { TS_CAPTION | TX_ACCENT } else if i < st.step { TS_CAPTION | TX_SECONDARY } else { TS_CAPTION | TX_TERTIARY };
        ui.text(style, t(n));
    }
    ui.box_end();
    let busy = !matches!(app.setup.phase, Phase::Ask | Phase::Failed);
    match (app.setup.role, app.setup.step) {
        (_, 0) => role_step(app, ui),
        (0, 1) => server_place(app, ui),
        (0, 2) => server_admin(app, ui),
        (0, 3) => server_repo(app, ui),
        (0, _) => server_run(app, ui),
        (2, 1) => agent_server(app, ui),
        (2, 2) => agent_folders(app, ui),
        _ => agent_run(app, ui),
    }
    if !app.setup.message.is_empty() && app.setup.phase == Phase::Failed {
        let m = app.setup.message.clone();
        widgets::notice(ui, IB_ERROR, t("Einrichtung nicht möglich"), &m);
    }
    let err = valid(&app.setup);
    if !err.is_empty() && app.setup.step > 0 {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, err);
    }
    // the buttons
    ui.spacer(4);
    // "an existing server" has no further step: Weiter leads to the sign-in page
    let last = app.setup.role != 1 && app.setup.step + 1 >= steps(app.setup.role).len();
    ui.box_begin(0, BOX_H, 8);
    ui.next_flags(if busy { ST_DISABLED } else { 0 });
    if ui.button(ID + 80, t("Später")) & EV_ACT != 0 {
        app.setup.active = false;
        ui.repass();
    }
    ui.spacer(-1);
    ui.next_flags(if app.setup.step == 0 || busy { ST_DISABLED } else { 0 });
    if ui.button(ID + 81, t("Zurück")) & EV_ACT != 0 {
        app.setup.step -= 1;
        app.setup.phase = Phase::Ask;
        ui.repass();
    }
    if !last {
        ui.next_flags(if err.is_empty() { 0 } else { ST_DISABLED });
        if ui.accent_button(ID + 82, t("Weiter")) & EV_ACT != 0 {
            if app.setup.role == 1 {
                // an existing server: the sign-in page
                app.setup.active = false;
            } else {
                app.setup.step += 1;
            }
            ui.repass();
        }
    }
    ui.box_end();
    ui.box_end();
    ui.box_end();
    ui.box_end();
    ui.box_end();
}

fn role_step(app: &mut Console, ui: &mut Ui) {
    ui.text(TS_TITLE, t("Willkommen bei BackupCraft"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Wie soll BackupCraft auf diesem Computer arbeiten?"));
    let roles = [
        (BC_SERVER, t("Backup-Server auf diesem Computer"), t("Dieser Computer speichert die Sicherungen, führt die Zeitpläne aus und wird mit dieser Konsole verwaltet.")),
        (BC_NETWORK, t("Mit einem vorhandenen Server verbinden"), t("Der Backup-Server läuft schon auf einem anderen Computer; diese Konsole verwaltet ihn.")),
        (BC_AGENT, t("Diesen Computer sichern lassen (Agent)"), t("Ein vorhandener Backup-Server sichert Ordner dieses Computers.")),
    ];
    for (i, (icon, title, text)) in roles.iter().enumerate() {
        if crate::wizards::tile(ui, ID + 10 + i as u32, *icon, title, text, app.setup.role == i as i32, true, None) {
            app.setup.role = i as i32;
        }
    }
}

fn server_place(app: &mut Console, ui: &mut Ui) {
    let s = &mut app.setup;
    ui.text(TS_TITLE, t("Speicherort und Netzwerk"));
    widgets::label(ui, t("Datenordner des Servers"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 20, None, &mut s.data, 1024);
    widgets::hint(ui, t("Hier liegen Datenbank, Schlüssel und Zertifikate des Servers – die Sicherungen selbst kommen in ein Repository."));
    widgets::label(ui, t("Erreichbar"));
    ui.radio(ID + 21, t("Nur von diesem Computer"), &mut s.network, 0);
    ui.radio(ID + 22, t("Im Netzwerk (für Agenten und andere Konsolen, verschlüsselt)"), &mut s.network, 1);
    if s.network == 1 {
        widgets::label(ui, t("Namen und Adressen für das Zertifikat (durch Komma getrennt)"));
        ui.next_size(-1, 0);
        ui.textbox(ID + 23, None, &mut s.names, 300);
        widgets::hint(ui, t("Unter diesen Namen erreichen Agenten und Konsolen den Server; die Firewall muss Port 7642 durchlassen."));
    }
}

fn server_admin(app: &mut Console, ui: &mut Ui) {
    let s = &mut app.setup;
    ui.text(TS_TITLE, t("Erster Administrator"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Mit diesem Konto melden Sie sich an. Weitere Benutzer legen Sie später unter Einstellungen an."));
    widgets::label(ui, t("Benutzername"));
    ui.next_size(-1, 0);
    ui.textbox_icon(ID + 30, IC_PERSON, Some("admin"), &mut s.user, 64);
    widgets::label(ui, t("Passwort (mindestens 12 Zeichen)"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 31, None, &mut s.pw1, 256 | TB_PASSWORD);
    widgets::label(ui, t("Passwort wiederholen"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 32, None, &mut s.pw2, 256 | TB_PASSWORD);
    widgets::label(ui, t("Name des Servers"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 33, None, &mut s.server_name, 120);
}

fn server_repo(app: &mut Console, ui: &mut Ui) {
    let s = &mut app.setup;
    ui.text(TS_TITLE, t("Wohin sollen die Sicherungen?"));
    ui.switch(ID + 40, t("Gleich ein Repository anlegen"), &mut s.repo_on);
    if s.repo_on != 0 {
        widgets::label(ui, t("Name"));
        ui.next_size(-1, 0);
        ui.textbox(ID + 41, None, &mut s.repo_name, 64);
        widgets::label(ui, t("Ordner"));
        ui.next_size(-1, 0);
        ui.textbox(ID + 42, None, &mut s.repo_path, 1024);
        widgets::hint(ui, t("Am besten auf einer anderen Festplatte oder einem NAS als die Daten, die gesichert werden. Das Repository speichert dedupliziert, komprimiert und verschlüsselt."));
    } else {
        widgets::hint(ui, t("Ein Repository lässt sich später unter „Repositories“ hinzufügen."));
    }
}

/// the summary and the run of the server setup
fn server_run(app: &mut Console, ui: &mut Ui) {
    ui.text(TS_TITLE, t("Einrichten"));
    let s = &app.setup;
    let net = if s.network == 1 { t("im Netzwerk (https)") } else { t("nur dieser Computer (http)") };
    let repo = if s.repo_on != 0 { format!("{} – {}", s.repo_name, s.repo_path) } else { String::from(t("später")) };
    widgets::props(ui, &[(t("Datenordner"), &s.data), (t("Erreichbar"), net), (t("Administrator"), &s.user), (t("Repository"), &repo)]);
    let phases = [
        (Phase::Installing, t("Dienst einrichten und starten (fragt nach Administratorrechten)")),
        (Phase::SettingUp, t("Administrator anlegen und anmelden")),
        (Phase::Repository, t("Repository anlegen")),
    ];
    if s.phase != Phase::Ask {
        let order = |p: Phase| match p {
            Phase::Installing => 1,
            Phase::SettingUp => 2,
            Phase::Repository => 3,
            Phase::Done => 4,
            _ => 0,
        };
        let now = order(s.phase);
        for (p, label) in phases {
            let k = order(p);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            if s.phase == Phase::Failed && k == order(failed_at(s)) {
                ui.icon(IC_ERROR, 16);
            } else if k < now || s.phase == Phase::Done {
                ui.icon(IC_ACCEPT, 16);
            } else if k == now {
                ui.next_size(16, 16);
                ui.progress_indeterminate(ID + 60 + k);
            } else {
                ui.icon(IC_CIRCLE_RING, 16);
            }
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, label);
            ui.box_end();
        }
    }
    let can = matches!(app.setup.phase, Phase::Ask | Phase::Failed) && valid(&app.setup).is_empty();
    if app.setup.phase == Phase::Ask {
        widgets::hint(ui, t("Für den Dienst fragt das System gleich nach Administratorrechten."));
    }
    ui.next_size(-1, 36);
    ui.next_flags(if can { 0 } else { ST_DISABLED });
    if ui.accent_button(ID + 50, if app.setup.phase == Phase::Failed { t("Erneut versuchen") } else { t("Einrichten") }) & EV_ACT != 0 {
        start_server(app);
        ui.repass();
    }
}

/// where a failed run stopped (kept in the url field's absence: no url = the service)
fn failed_at(s: &State) -> Phase {
    if s.url.is_empty() { Phase::Installing } else { Phase::SettingUp }
}

fn start_server(app: &mut Console) {
    let s = &mut app.setup;
    s.message.clear();
    s.url.clear();
    let Some(exe) = sibling("bc-server") else {
        s.phase = Phase::Failed;
        s.message = String::from(t("bc-server wurde neben der Konsole nicht gefunden (Installation unvollständig?)."));
        return;
    };
    let report = report_file();
    let mut args = alloc::vec![String::from("--data"), String::from(s.data.trim())];
    if s.network == 1 {
        args.extend([String::from("--listen"), format!("0.0.0.0:{}", port())]);
        if !s.names.trim().is_empty() {
            args.extend([String::from("--name"), String::from(s.names.trim())]);
        }
    } else {
        args.extend([String::from("--listen"), format!("127.0.0.1:{}", port())]);
    }
    if plat::env("BC_SETUP_NO_SERVICE").is_some() {
        plat::delete(&report);
        s.work = Some(start_child(exe, args, String::from(s.data.trim())));
    } else {
        let mut all = alloc::vec![String::from("service"), String::from("install")];
        all.extend(args);
        all.extend([String::from("--report"), report.clone()]);
        s.work = Some(start_worker(exe, all, report, Some(port())));
    }
    s.phase = Phase::Installing;
}

/// tests: the server as a child of the console; the token and the CA's fingerprint from its data
fn start_child(exe: String, args: Vec<String>, data: String) -> Arc<Mutex<Option<Outcome>>> {
    let slot = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    WORKING.store(true, core::sync::atomic::Ordering::SeqCst);
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
        let mut all = alloc::vec![String::from("run")];
        all.extend(args);
        let refs: Vec<&str> = all.iter().map(|a| a.as_str()).collect();
        let o = match (plat::process::Command { program: &exe, args: &refs, cwd: None, env: &[] }).spawn() {
            Ok(c) => {
                *CHILD.lock() = Some(c);
                let (ca, token) = (plat::fs::join(&plat::fs::join(&data, "pki"), "ca.pem"), plat::fs::join(&data, "setup.token"));
                for _ in 0..300 {
                    if plat::file_exists(&ca) == 1 && plat::file_exists(&token) == 1 {
                        break;
                    }
                    plat::sleep_ms(100);
                }
                plat::sleep_ms(200);
                let fp = plat::read_file(&ca)
                    .and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE"))
                    .map(|der| netio::trust::fingerprint_text(&crypto::sha256(&der)))
                    .unwrap_or_default();
                let tok = plat::read_file(&token).map(|b| String::from(String::from_utf8_lossy(&b).trim())).unwrap_or_default();
                let addr = plat::net::SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, port());
                let up = (0..150).any(|_| plat::net::TcpStream::connect(&addr, 500).is_ok() || {
                    plat::sleep_ms(200);
                    false
                });
                Outcome { ok: up && !tok.is_empty(), message: String::from(if up { "" } else { "Der Server antwortet nicht" }), token: tok, fingerprint: fp }
            }
            Err(e) => Outcome { ok: false, message: format!("bc-server startet nicht (Fehler {})", e), ..Outcome::default() },
        };
        *s2.lock() = Some(o);
        WORKING.store(false, core::sync::atomic::Ordering::SeqCst);
        plat::wake();
    }));
    slot
}

/// follow the run: the worker's outcome, the setup and the sign-in (app events), the repository
fn progress(app: &mut Console, ui: &mut Ui) {
    let done = app.setup.work.as_ref().and_then(|w| w.lock().take());
    if let Some(o) = done {
        app.setup.work = None;
        if let Some(c) = CHILD.lock().take() {
            app.setup.child = Some(c);
        }
        if !o.ok {
            app.setup.phase = Phase::Failed;
            app.setup.message = o.message;
            ui.repass();
            return;
        }
        if app.setup.role == 2 {
            app.setup.phase = Phase::Done;
            app.setup.message = o.message;
            ui.repass();
            return;
        }
        if o.token.is_empty() {
            app.setup.phase = Phase::Failed;
            app.setup.message = String::from(t("Der Dienst läuft, aber der Server ist bereits eingerichtet. Bitte mit „Später“ zur Anmeldung wechseln und mit dem vorhandenen Konto anmelden."));
            ui.repass();
            return;
        }
        // the server answers: the first setup through the API, then the sign-in
        let s = &mut app.setup;
        let tls = s.network == 1;
        s.url = format!("{}://{}:{}", if tls { "https" } else { "http" }, if tls { "localhost" } else { "127.0.0.1" }, port());
        let pin = netio::trust::parse_fingerprint(&o.fingerprint);
        if let (true, Some(p), Some(u)) = (tls, pin, crate::live::server_url(&s.url)) {
            app.prefs.pins.insert(u.host_header(), crypto::hex(&p));
        }
        app.login.server = app.setup.url.clone();
        app.login.user = String::from(app.setup.user.trim());
        app.login.password = app.setup.pw1.clone();
        app.login.error.clear();
        app.prefs.server = app.setup.url.clone();
        app.save_prefs();
        let trust = match pin {
            Some(p) if tls => ServerTrust::Pin(p),
            _ => ServerTrust::Ask,
        };
        app.net = Net::live(&app.setup.url, trust);
        app.fixture = false;
        app.login.busy = true;
        let body = j::obj([
            ("token", Value::string(o.token.as_str())),
            ("username", Value::string(app.setup.user.trim())),
            ("password", Value::string(app.setup.pw1.as_str())),
            ("server_name", Value::string(app.setup.server_name.trim())),
        ]);
        app.net.action(Method::Post, "/api/setup", Some(body), "setup", &[], After::Setup);
        app.setup.phase = Phase::SettingUp;
        ui.repass();
        return;
    }
    // the setup or the sign-in failed (the sign-in page's error)
    if app.setup.phase == Phase::SettingUp && !app.login.busy && !app.login.error.is_empty() {
        app.setup.phase = Phase::Failed;
        app.setup.message = app.login.error.clone();
        ui.repass();
    }
}

/// after the sign-in (from app.frame): the repository, then the first job
pub fn after_login(app: &mut Console, ui: &mut Ui) {
    if !app.setup.active || app.setup.phase != Phase::SettingUp {
        return;
    }
    app.setup.pw1.clear();
    app.setup.pw2.clear();
    if app.setup.repo_on != 0 {
        let body = api::RepositoryRequest { name: String::from(app.setup.repo_name.trim()), path: String::from(app.setup.repo_path.trim()), create: true, immutable_days: 0 }.to_json();
        app.net.action(Method::Post, "/api/repositories", Some(body), "repository.save", &["/api/repositories"], After::Nothing);
        app.setup.phase = Phase::Repository;
    } else {
        app.setup.phase = Phase::Done;
    }
    app.setup.active = false;
    ui.toast(IB_SUCCESS, t("Einrichtung abgeschlossen"), t("Der Server läuft als Dienst. Jetzt den ersten Sicherungsjob anlegen."), 6000);
    app.start_wizard_named("job");
    ui.repass();
}

fn agent_server(app: &mut Console, ui: &mut Ui) {
    let s = &mut app.setup;
    ui.text(TS_TITLE, t("Mit welchem Server koppeln?"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("In der Konsole des Servers unter „Server & Agenten“ › „Agent koppeln“ stehen Adresse, Pairing-Code und Fingerabdruck."));
    widgets::label(ui, t("Serveradresse"));
    ui.next_size(-1, 0);
    ui.textbox_icon(ID + 70, IC_GLOBE, Some("https://backup.example:7642"), &mut s.a_server, 200);
    widgets::label(ui, t("Pairing-Code"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 71, Some("ABCD-EFGH"), &mut s.a_code, 20);
    widgets::label(ui, t("Fingerabdruck der Zertifizierungsstelle"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 72, Some("AB:CD:…"), &mut s.a_fp, 200);
}

fn agent_folders(app: &mut Console, ui: &mut Ui) {
    let s = &mut app.setup;
    ui.text(TS_TITLE, t("Welche Ordner darf der Server sichern?"));
    widgets::label(ui, t("Zu sichernde Ordner (einer pro Zeile)"));
    ui.next_size(-1, 90);
    ui.textbox(ID + 73, None, &mut s.a_roots, 4096 | TB_MULTI);
    widgets::label(ui, t("Ordner für Wiederherstellungen"));
    ui.next_size(-1, 0);
    ui.textbox(ID + 74, None, &mut s.a_restore, 1024);
    widgets::hint(ui, t("Der Agent liest nur in diesen Ordnern und schreibt nur in sie und den Wiederherstellungsordner; Befehle führt er nicht aus."));
}

fn agent_run(app: &mut Console, ui: &mut Ui) {
    ui.text(TS_TITLE, t("Koppeln"));
    let s = &app.setup;
    let roots: Vec<&str> = s.a_roots.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    widgets::props(ui, &[(t("Server"), &s.a_server), (t("Ordner"), &roots.join(", ")), (t("Wiederherstellung"), &s.a_restore)]);
    match s.phase {
        Phase::Installing => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            ui.next_size(16, 16);
            ui.progress_indeterminate(ID + 75);
            ui.text(TS_BODY, t("Koppeln und Dienst einrichten (fragt nach Administratorrechten) …"));
            ui.box_end();
        }
        Phase::Done => {
            ui.next_size(-1, 0);
            ui.infobar(ID + 84, IB_SUCCESS, t("Gekoppelt"), t("Der Agent läuft als Dienst. Sicherungen dieses Computers richten Sie in der Konsole des Servers ein."));
            ui.next_size(-1, 36);
            if ui.accent_button(ID + 77, t("Konsole mit dem Server verbinden")) & EV_ACT != 0 {
                app.login.server = app.setup.a_server.clone();
                app.setup.active = false;
                ui.repass();
            }
            return;
        }
        _ => {}
    }
    let can = matches!(app.setup.phase, Phase::Ask | Phase::Failed) && valid(&app.setup).is_empty();
    ui.next_size(-1, 36);
    ui.next_flags(if can { 0 } else { ST_DISABLED });
    if ui.accent_button(ID + 78, if app.setup.phase == Phase::Failed { t("Erneut versuchen") } else { t("Koppeln") }) & EV_ACT != 0 {
        start_agent(app);
        ui.repass();
    }
}

fn start_agent(app: &mut Console) {
    let s = &mut app.setup;
    s.message.clear();
    let Some(exe) = sibling("bc-agent") else {
        s.phase = Phase::Failed;
        s.message = String::from(t("bc-agent wurde neben der Konsole nicht gefunden (Installation unvollständig?)."));
        return;
    };
    let report = report_file();
    let mut args = alloc::vec![
        String::from("pair"),
        String::from("--server"), String::from(s.a_server.trim()),
        String::from("--code"), String::from(s.a_code.trim()),
        String::from("--fingerprint"), String::from(s.a_fp.trim()),
        String::from("--data"), default_agent_data(),
    ];
    for r in s.a_roots.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        args.extend([String::from("--root"), String::from(r)]);
    }
    if !s.a_restore.trim().is_empty() {
        args.extend([String::from("--restore-root"), String::from(s.a_restore.trim())]);
    }
    args.extend([String::from("--service"), String::from("--report"), report.clone()]);
    s.work = Some(start_worker(exe, args, report, None));
    s.phase = Phase::Installing;
}
