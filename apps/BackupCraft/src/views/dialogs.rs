// dialogs -- the console's content dialogs: confirmations, session log, password, shared
// folders of a host, repository, immutability, credentials, users, recovery kit, configuration
// backup and restore, replicas and failover plans, watched servers (fleet) and fleet tokens,
// tapes, media pools, a point to tape, a restore from tape, instant start (start, console,
// merge) and boot checks

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use ui::*;

use crate::app::{Console, ID_DLG};
use crate::net::After;
use crate::tr::{t, tf};
use crate::{fmt, widgets};

/// what a confirmation does
#[derive(Clone, Debug)]
pub enum Confirm {
    DeleteJobs(Vec<String>),
    DeletePoint(String, String),
    RemoveHost(String),
    DeleteCreds(Vec<String>),
    DeleteUser(String),
    Gc(String),
    DetachRepo(String),
    RecoveryKit,
    Failover(String),
    Failback(String),
    UndoFailover(String),
    DeleteReplica(String),
    RunPlan(String),
    DeletePlan(String),
    RemoveFleet(String),
    RevokeFleetToken(String),
    EraseTape(String),
    RemoveTape(String),
    DeletePool(String),
    StopInstant(String),
}

#[derive(Default)]
struct Form {
    a: String,
    b: String,
    c: String,
    n: i32,
    flag: i32,
    id: String,
    min: i32,
    /// chosen ids in order (the replicas of a plan; the host of a replica)
    pick: Vec<String>,
}

pub struct Dialogs {
    confirm: Shared<i32>,
    confirm_what: Option<Confirm>,
    confirm_title: String,
    confirm_text: String,
    confirm_button: String,
    log: Shared<i32>,
    log_session: Session,
    password: Shared<i32>,
    roots: Shared<i32>,
    repo: Shared<i32>,
    imm: Shared<i32>,
    cred: Shared<i32>,
    user: Shared<i32>,
    kit: Shared<i32>,
    kit_key: String,
    cfg_backup: Shared<i32>,
    cfg_restore: Shared<i32>,
    replica: Shared<i32>,
    plan: Shared<i32>,
    fleet: Shared<i32>,
    fleet_token: Shared<i32>,
    tapes_add: Shared<i32>,
    tape_assign: Shared<i32>,
    pool: Shared<i32>,
    tape_write: Shared<i32>,
    set_restore: Shared<i32>,
    /// the hosts a restore from tape may write to (as opened)
    hosts: Vec<Host>,
    instant: Shared<i32>,
    boot_check: Shared<i32>,
    console: Shared<i32>,
    merge: Shared<i32>,
    /// the points with VMs to choose from: (point id, label, VM names)
    vm_points: Vec<(String, String, Vec<String>)>,
    /// the watched-server dialog: the CA its server showed, waiting for an answer, its error
    probe: Option<api::ProbeResult>,
    busy: bool,
    error: String,
    /// a new fleet token (shown once)
    new_token: String,
    f: Form,
}

impl Default for Dialogs {
    fn default() -> Dialogs {
        Dialogs {
            confirm: Shared::new(0),
            confirm_what: None,
            confirm_title: String::new(),
            confirm_text: String::new(),
            confirm_button: String::new(),
            log: Shared::new(0),
            log_session: Session::default(),
            password: Shared::new(0),
            roots: Shared::new(0),
            repo: Shared::new(0),
            imm: Shared::new(0),
            cred: Shared::new(0),
            user: Shared::new(0),
            kit: Shared::new(0),
            kit_key: String::new(),
            cfg_backup: Shared::new(0),
            cfg_restore: Shared::new(0),
            replica: Shared::new(0),
            plan: Shared::new(0),
            fleet: Shared::new(0),
            fleet_token: Shared::new(0),
            tapes_add: Shared::new(0),
            tape_assign: Shared::new(0),
            pool: Shared::new(0),
            tape_write: Shared::new(0),
            set_restore: Shared::new(0),
            hosts: Vec::new(),
            instant: Shared::new(0),
            boot_check: Shared::new(0),
            console: Shared::new(0),
            merge: Shared::new(0),
            vm_points: Vec::new(),
            probe: None,
            busy: false,
            error: String::new(),
            new_token: String::new(),
            f: Form::default(),
        }
    }
}

// a dialog's widgets are D_* + n; an infobar also takes the next id (its close button),
// a segmented control one per segment -- infobars sit at D_* + 0x80
const D_CONFIRM: u32 = ID_DLG + 0x100;
const D_LOG: u32 = ID_DLG + 0x200;
const D_PW: u32 = ID_DLG + 0x300;
const D_ROOTS: u32 = ID_DLG + 0x400;
const D_REPO: u32 = ID_DLG + 0x500;
const D_IMM: u32 = ID_DLG + 0x600;
const D_CRED: u32 = ID_DLG + 0x700;
const D_USER: u32 = ID_DLG + 0x800;
const D_KIT: u32 = ID_DLG + 0x900;
const D_CFGB: u32 = ID_DLG + 0xA00;
const D_CFGR: u32 = ID_DLG + 0xB00;
const D_REPL: u32 = ID_DLG + 0xC00;
const D_PLAN: u32 = ID_DLG + 0xD00;
const D_FLEET: u32 = ID_DLG + 0xE00;
const D_FTOK: u32 = ID_DLG + 0xF00;
const D_TADD: u32 = ID_DLG + 0x1000;
const D_TASSIGN: u32 = ID_DLG + 0x1100;
const D_POOL: u32 = ID_DLG + 0x1200;
const D_TWRITE: u32 = ID_DLG + 0x1300;
const D_SREST: u32 = ID_DLG + 0x1400;
const D_INSTANT: u32 = ID_DLG + 0x1500;
const D_BOOT_CHECK: u32 = ID_DLG + 0x1600;
const D_CONSOLE: u32 = ID_DLG + 0x1700;
const D_MERGE: u32 = ID_DLG + 0x1800;

impl Dialogs {
    pub fn confirm(&mut self, what: Confirm, title: &str, text: &str, button: &str) {
        self.confirm_what = Some(what);
        self.confirm_title = String::from(title);
        self.confirm_text = String::from(text);
        self.confirm_button = String::from(button);
        self.confirm.set(1);
    }
    pub fn open_log(&mut self, s: &Session) {
        self.log_session = s.clone();
        self.log.set(1);
    }
    pub fn open_password(&mut self) {
        self.f = Form::default();
        self.password.set(1);
    }
    pub fn open_roots(&mut self, h: &Host) {
        self.f = Form { a: h.name.clone(), b: h.roots.join("\n"), c: h.restore_roots.join("\n"), id: h.id.clone(), ..Form::default() };
        self.roots.set(1);
    }
    pub fn open_repository(&mut self) {
        self.f = Form { n: 0, flag: 1, ..Form::default() };
        self.repo.set(1);
    }
    pub fn open_immutability(&mut self, r: &Repository) {
        self.f = Form { a: r.name.clone(), id: r.id.clone(), n: r.immutable_days as i32, min: r.immutable_days as i32, ..Form::default() };
        self.imm.set(1);
    }
    pub fn open_credential(&mut self) {
        self.f = Form::default();
        self.cred.set(1);
    }
    pub fn open_user(&mut self, u: Option<&User>) {
        self.f = match u {
            Some(u) => Form { a: u.username.clone(), id: u.id.clone(), n: u.role.rank() as i32 - 1, flag: u.disabled as i32, ..Form::default() },
            None => Form { n: 0, ..Form::default() },
        };
        self.user.set(1);
    }
    pub fn open_config_backup(&mut self) {
        self.f = Form::default();
        self.cfg_backup.set(1);
    }
    pub fn open_config_restore(&mut self, name: &str) {
        self.f = Form { a: String::from(name), ..Form::default() };
        self.cfg_restore.set(1);
    }
    /// a replica: a (name), b (folder), c (job id), id, flag (auto), the host id in pick[0]
    pub fn open_replica(&mut self, r: Option<&Replica>) {
        self.f = match r {
            Some(r) => Form { a: r.name.clone(), b: r.target.clone(), c: r.job_id.clone(), id: r.id.clone(), flag: r.auto as i32, pick: alloc::vec![r.host_id.clone()], ..Form::default() },
            None => Form { flag: 1, ..Form::default() },
        };
        self.replica.set(1);
    }
    /// a watched server: a (name), b (address), c (token), id; the address it had in pick[0]
    pub fn open_fleet(&mut self, s: Option<&FleetServer>) {
        self.f = match s {
            Some(s) => Form { a: s.name.clone(), b: s.url.clone(), id: s.id.clone(), pick: alloc::vec![s.url.clone()], ..Form::default() },
            None => Form::default(),
        };
        self.probe = None;
        self.busy = false;
        self.error.clear();
        self.fleet.set(1);
    }
    pub fn open_fleet_token(&mut self) {
        self.f = Form::default();
        self.new_token.clear();
        self.busy = false;
        self.error.clear();
        self.fleet_token.set(1);
    }
    /// new tapes: n (count), min (capacity in GB), a (barcode prefix), flag (pool: 0 = free)
    pub fn open_tapes_add(&mut self) {
        self.f = Form { n: 4, min: 1500, a: String::from("BC"), ..Form::default() };
        self.tapes_add.set(1);
    }
    /// a blank tape to a pool: id (barcode), a (pool id)
    pub fn open_tape_assign(&mut self, c: &Cartridge) {
        self.f = Form { id: c.id.clone(), a: c.pool_id.clone(), ..Form::default() };
        self.tape_assign.set(1);
    }
    /// a media pool: a (name), b (description), n (protection days), flag (encrypted), min (append)
    pub fn open_pool(&mut self, p: Option<&MediaPool>) {
        self.f = match p {
            Some(p) => Form { a: p.name.clone(), b: p.description.clone(), n: p.protect_days as i32, flag: p.encrypted as i32, min: p.append as i32, id: p.id.clone(), ..Form::default() },
            None => Form { n: 28, flag: 1, min: 1, ..Form::default() },
        };
        self.pool.set(1);
    }
    /// a point to tape: id (point), a (what), b (pool id)
    pub fn open_tape_write(&mut self, point: &RestorePoint) {
        self.f = Form { id: point.id.clone(), a: format!("{} · {}", point.job_name, point.created), ..Form::default() };
        self.tape_write.set(1);
    }
    /// a restore from tape: id (set), a (target), b (paths), c (host id), flag (conflict)
    pub fn open_set_restore(&mut self, s: &TapeSet, hosts: &[Host]) {
        self.hosts = hosts.iter().filter(|h| h.kind == HostKind::Server || h.status == HostStatus::Online).cloned().collect();
        let server = self.hosts.iter().find(|h| h.kind == HostKind::Server).cloned();
        let target = server.as_ref().and_then(|h| h.restore_roots.first().cloned()).unwrap_or_default();
        self.f = Form { id: s.id.clone(), a: target, c: server.map_or(String::new(), |h| h.id), flag: 1, ..Form::default() };
        self.set_restore.set(1);
    }
    /// a VM of a point to start: id (point), a (VM), n (memory MB, 0 = as backed up), flag
    /// (network: 0 none, 1 isolated)
    pub fn open_instant(&mut self, p: &RestorePoint, names: &[String], choices: &[(String, String, Vec<String>)]) {
        self.vm_points = choices.to_vec();
        self.f = Form { id: p.id.clone(), a: names.first().cloned().unwrap_or_default(), ..Form::default() };
        self.instant.set(1);
    }
    /// a boot check: id (point), a (VM), n (boot seconds), b (console text), c (port),
    /// min (timeout seconds)
    pub fn open_boot_check(&mut self, p: &RestorePoint, names: &[String], choices: &[(String, String, Vec<String>)]) {
        self.vm_points = choices.to_vec();
        self.f = Form { id: p.id.clone(), a: names.first().cloned().unwrap_or_default(), n: 20, min: 180, ..Form::default() };
        self.boot_check.set(1);
    }
    pub fn open_console(&mut self, s: &InstantSession) {
        self.f = Form { id: s.id.clone(), a: s.vm.clone(), ..Form::default() };
        self.console.set(1);
    }
    pub fn open_merge(&mut self, s: &InstantSession, root: &str) {
        let sep = if root.contains('\\') { "\\" } else { "/" };
        let target = if root.is_empty() { String::new() } else { format!("{}{}{}", root.trim_end_matches(['/', '\\']), sep, s.vm) };
        self.f = Form { id: s.id.clone(), a: target, b: s.vm.clone(), ..Form::default() };
        self.merge.set(1);
    }
    pub fn open_plan(&mut self, p: Option<&FailoverPlan>) {
        self.f = match p {
            Some(p) => Form { a: p.name.clone(), id: p.id.clone(), n: p.delay_s as i32, pick: p.replicas.clone(), ..Form::default() },
            None => Form { n: 60, ..Form::default() },
        };
        self.plan.set(1);
    }

    /// an answer for one of the dialogs
    pub fn done(&mut self, label: &str, body: &Value, _after: &After) {
        match label {
            "recovery.kit" => {
                if let Some(k) = api::RecoveryKit::from_json(body) {
                    self.kit_key = k.private_key;
                    self.f = Form::default();
                    self.kit.set(1);
                }
            }
            "fleet.probe" => {
                self.busy = false;
                self.probe = api::ProbeResult::from_json(body);
                self.f.flag = 0;
            }
            "fleet.save" => {
                self.busy = false;
                self.f.c.clear();
                self.fleet.set(0);
            }
            "fleet.token" => {
                self.busy = false;
                if let Some(r) = api::FleetTokenCreated::from_json(body) {
                    self.new_token = r.token;
                }
            }
            "user.save" | "credential.save" | "repository.save" | "host.save" | "password.save" | "config.restore" => {}
            _ => {}
        }
    }
}

impl Dialogs {
    /// an action of a dialog failed: it stays open and shows why
    pub fn failed(&mut self, label: &str, message: &str) {
        if matches!(label, "fleet.probe" | "fleet.save" | "fleet.token") {
            self.busy = false;
            self.error = String::from(message);
        }
    }
}

fn field(ui: &mut Ui, label: &str) {
    ui.text(TS_BODY_STRONG, label);
}

/// one buttons row; DLG_PRIMARY only when `ok`, else the dialog stays open
fn buttons(ui: &mut Ui, open: &Shared<i32>, primary: &str, ok: bool) -> bool {
    let r = ui.dialog_buttons(Some(primary), None, Some(t("Abbrechen")));
    if r == DLG_PRIMARY {
        if ok {
            return true;
        }
        open.set(1);
    }
    false
}

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let d = &mut app.dlg;
    // confirmation
    if ui.dialog_begin(D_CONFIRM, &d.confirm_title.clone(), &d.confirm.clone()) {
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP, &d.confirm_text.clone());
        let r = ui.dialog_buttons(Some(&d.confirm_button.clone()), None, Some(t("Abbrechen")));
        if r == DLG_PRIMARY {
            if let Some(w) = d.confirm_what.take() {
                run_confirm(app, w);
            }
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // session log
    if ui.dialog_begin(D_LOG, &tf("Protokoll: {}", &[&d.log_session.job_name]), &d.log.clone()) {
        let s = d.log_session.clone();
        let now = ui.now();
        let path = format!("/api/sessions/{}/log", s.id);
        app.net.want(&path, 5000, now);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {}", fmt::when(s.started, &app.zone), fmt::session_status(s.status).0));
        let lines = app.net.data.logs.get(&s.id).cloned().unwrap_or_default();
        ui.next_size(520, 300);
        ui.scroll_begin(D_LOG + 1, SC_V | BOX_LAYER | pad(10));
        ui.box_begin(0, BOX_V, 4);
        for l in &lines {
            ui.box_begin(0, BOX_H, 8);
            ui.icon(match l.level {
                LogLevel::Error => IC_ERROR,
                LogLevel::Warning => IC_WARNING,
                _ => IC_INFO,
            }, 14);
            ui.next_size(60, 0);
            ui.text(TS_CAPTION | TX_TERTIARY, &datetime::format_time(datetime::Timestamp::from_secs(l.at), &app.zone));
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_WRAP, &l.msg);
            ui.box_end();
        }
        ui.box_end();
        ui.scroll_end();
        ui.dialog_buttons(None, None, Some(t("Schließen")));
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // password
    if ui.dialog_begin(D_PW, t("Passwort ändern"), &d.password.clone()) {
        field(ui, t("Bisheriges Passwort"));
        ui.next_size(360, 0);
        ui.textbox(D_PW + 1, None, &mut d.f.a, 256 | TB_PASSWORD);
        field(ui, t("Neues Passwort"));
        ui.next_size(360, 0);
        ui.textbox(D_PW + 2, None, &mut d.f.b, 256 | TB_PASSWORD);
        field(ui, t("Neues Passwort wiederholen"));
        ui.next_size(360, 0);
        ui.textbox(D_PW + 3, None, &mut d.f.c, 256 | TB_PASSWORD);
        let err = if d.f.b.chars().count() > 0 && d.f.b.chars().count() < 12 {
            t("Mindestens 12 Zeichen.")
        } else if !d.f.c.is_empty() && d.f.b != d.f.c {
            t("Die Passwörter stimmen nicht überein.")
        } else {
            ""
        };
        widgets::error(ui, err);
        let ok = err.is_empty() && !d.f.a.is_empty() && d.f.b.chars().count() >= 12 && d.f.b == d.f.c;
        if buttons(ui, &d.password, t("Ändern"), ok) {
            let body = api::PasswordChange { old: d.f.a.clone(), new: d.f.b.clone() }.to_json();
            app.net.action(Method::Post, "/api/users/me/password", Some(body), "password.save", &[], After::Nothing);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // shared folders of a host
    if ui.dialog_begin(D_ROOTS, t("Freigaben bearbeiten"), &d.roots.clone()) {
        field(ui, t("Anzeigename"));
        ui.next_size(440, 0);
        ui.textbox(D_ROOTS + 1, None, &mut d.f.a, 128);
        field(ui, t("Lesbare Ordner (Sicherungsquellen), einer pro Zeile"));
        ui.next_size(440, 100);
        ui.textbox(D_ROOTS + 2, None, &mut d.f.b, 4096 | TB_MULTI);
        field(ui, t("Beschreibbare Ordner (Wiederherstellungsziele)"));
        ui.next_size(440, 80);
        ui.textbox(D_ROOTS + 3, None, &mut d.f.c, 4096 | TB_MULTI);
        ui.next_size(440, 0);
        widgets::hint(ui, t("Der Agent lässt nur Zugriffe innerhalb dieser Ordner zu; Verknüpfungen nach außen werden nicht verfolgt."));
        if buttons(ui, &d.roots, t("Speichern"), true) {
            let lines = |s: &str| s.lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            let body = api::HostUpdate { name: d.f.a.clone(), roots: lines(&d.f.b), restore_roots: lines(&d.f.c) }.to_json();
            app.net.action(Method::Put, &format!("/api/hosts/{}", d.f.id), Some(body), "host.save", &["/api/hosts"], After::Nothing);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // repository
    if ui.dialog_begin(D_REPO, t("Repository hinzufügen"), &d.repo.clone()) {
        field(ui, t("Name"));
        ui.next_size(440, 0);
        ui.textbox(D_REPO + 1, Some(t("z. B. Primär (NAS)")), &mut d.f.a, 64);
        field(ui, t("Ordner auf dem Server"));
        ui.next_size(440, 0);
        ui.textbox(D_REPO + 2, Some("/mnt/nas/backupcraft"), &mut d.f.b, 1024);
        ui.radio(D_REPO + 3, t("Neues Repository anlegen"), &mut d.f.flag, 1);
        ui.radio(D_REPO + 4, t("Vorhandenes Repository einbinden"), &mut d.f.flag, 0);
        field(ui, t("Unveränderlichkeit (Tage, 0 = aus)"));
        ui.next_size(140, 0);
        ui.number_box(D_REPO + 5, &mut d.f.n, &Range::new(0, 3650, 1, 0));
        ui.next_size(440, 0);
        widgets::hint(ui, t("Unveränderliche Restore-Points kann niemand vor Ablauf löschen – auch kein Administrator."));
        let ok = !d.f.a.trim().is_empty() && !d.f.b.trim().is_empty();
        if buttons(ui, &d.repo, t("Hinzufügen"), ok) {
            let body = api::RepositoryRequest { name: String::from(d.f.a.trim()), path: String::from(d.f.b.trim()), create: d.f.flag == 1, immutable_days: d.f.n as u32 }.to_json();
            app.net.action(Method::Post, "/api/repositories", Some(body), "repository.save", &["/api/repositories"], After::Nothing);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // immutability
    if ui.dialog_begin(D_IMM, &tf("Unveränderlichkeit: {}", &[&d.f.a.clone()]), &d.imm.clone()) {
        field(ui, t("Tage"));
        ui.next_size(140, 0);
        let min = d.f.min;
        ui.number_box(D_IMM + 1, &mut d.f.n, &Range::new(min, 3650, 1, 0));
        ui.next_size(420, 0);
        widgets::hint(ui, t("Die Frist lässt sich nur verlängern. Sie gilt für neue Restore-Points; bestehende behalten ihre Frist."));
        if buttons(ui, &d.imm, t("Übernehmen"), d.f.n >= min) {
            let body = j::obj([("immutable_days", Value::u64(d.f.n.max(0) as u64))]);
            app.net.action(Method::Put, &format!("/api/repositories/{}", d.f.id), Some(body), "repository.save", &["/api/repositories"], After::Nothing);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // credentials
    if ui.dialog_begin(D_CRED, t("Zugangsdaten hinzufügen"), &d.cred.clone()) {
        field(ui, t("Bezeichnung"));
        ui.next_size(440, 0);
        ui.textbox(D_CRED + 1, Some(t("z. B. Linux-Server")), &mut d.f.a, 64);
        field(ui, t("Benutzer"));
        ui.next_size(440, 0);
        ui.textbox(D_CRED + 2, Some("root"), &mut d.f.b, 128);
        ui.box_begin(0, BOX_H, 16);
        ui.radio(D_CRED + 3, t("Passwort"), &mut d.f.n, 0);
        ui.radio(D_CRED + 4, t("SSH-Schlüssel (Ed25519)"), &mut d.f.n, 1);
        ui.box_end();
        if d.f.n == 0 {
            field(ui, t("Passwort"));
            ui.next_size(440, 0);
            ui.textbox(D_CRED + 5, None, &mut d.f.c, 256 | TB_PASSWORD);
        } else {
            field(ui, t("Privater Schlüssel (OpenSSH-Format)"));
            ui.next_size(440, 120);
            ui.textbox(D_CRED + 6, Some("-----BEGIN OPENSSH PRIVATE KEY-----"), &mut d.f.c, 16384 | TB_MULTI);
        }
        let ok = !d.f.a.trim().is_empty() && !d.f.b.trim().is_empty() && !d.f.c.is_empty();
        if buttons(ui, &d.cred, t("Speichern"), ok) {
            let body = api::CredentialRequest { label: String::from(d.f.a.trim()), username: String::from(d.f.b.trim()),
                                                kind: if d.f.n == 1 { CredKind::SshKey } else { CredKind::Password }, secret: d.f.c.clone() }.to_json();
            d.f.c.clear();
            app.net.action(Method::Post, "/api/credentials", Some(body), "credential.save", &["/api/credentials"], After::Nothing);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // user
    let editing = !d.f.id.is_empty();
    if ui.dialog_begin(D_USER, if editing { t("Benutzer bearbeiten") } else { t("Benutzer anlegen") }, &d.user.clone()) {
        field(ui, t("Benutzername"));
        ui.next_size(380, 0);
        if editing {
            ui.text(TS_BODY, &d.f.a.clone());
        } else {
            ui.textbox(D_USER + 1, Some("vorname.nachname"), &mut d.f.a, 64);
        }
        field(ui, t("Rolle"));
        ui.next_size(380, 0);
        ui.combobox(D_USER + 2, &mut d.f.n, &[t("Betrachter"), t("Wiederherstellung"), t("Operator"), t("Administrator")]);
        field(ui, if editing { t("Neues Passwort (leer: unverändert)") } else { t("Passwort (mindestens 12 Zeichen)") });
        ui.next_size(380, 0);
        ui.textbox(D_USER + 3, None, &mut d.f.b, 256 | TB_PASSWORD);
        if editing {
            ui.switch(D_USER + 4, t("Gesperrt"), &mut d.f.flag);
        }
        let pwn = d.f.b.chars().count();
        let err = if pwn > 0 && pwn < 12 { t("Mindestens 12 Zeichen.") } else { "" };
        widgets::error(ui, err);
        let ok = err.is_empty() && (editing || (!d.f.a.trim().is_empty() && pwn >= 12));
        if buttons(ui, &d.user, t("Speichern"), ok) {
            let role = Role::ALL[d.f.n.clamp(0, 3) as usize];
            let body = api::UserRequest { username: String::from(d.f.a.trim()), role, password: d.f.b.clone(), disabled: d.f.flag != 0 }.to_json();
            d.f.b.clear();
            if editing {
                app.net.action(Method::Put, &format!("/api/users/{}", d.f.id), Some(body), "user.save", &["/api/users"], After::Nothing);
            } else {
                app.net.action(Method::Post, "/api/users", Some(body), "user.save", &["/api/users"], After::Nothing);
            }
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // recovery kit (shown once)
    if ui.dialog_begin(D_KIT, t("Recovery-Kit"), &d.kit.clone()) {
        ui.next_size(480, 0);
        ui.infobar(D_KIT + 0x80, IB_WARNING, t("Nur jetzt sichtbar"), t("Notieren oder ausdrucken und offline verwahren."));
        ui.next_size(480, 0);
        ui.text(TS_BODY | TX_WRAP, t("Mit diesem Schlüssel lässt sich jede verschlüsselte Sicherung öffnen, auch wenn dieser Server verloren ist."));
        ui.next_size(480, 80);
        let mut key = d.kit_key.clone();
        ui.textbox(D_KIT + 2, None, &mut key, 4096 | TB_MULTI | TB_READONLY | TB_WRAP);
        ui.box_begin(0, BOX_H, 8);
        ui.next_icon(IC_COPY);
        if ui.button(D_KIT + 3, t("Kopieren")) & EV_ACT != 0 {
            ui.clip_set(CLIP_TEXT, d.kit_key.as_bytes());
            ui.toast(IB_SUCCESS, t("Kopiert"), t("Der Schlüssel liegt in der Zwischenablage."), 2500);
        }
        ui.box_end();
        ui.checkbox(D_KIT + 4, t("Ich habe den Schlüssel sicher abgelegt."), &mut d.f.flag);
        if buttons(ui, &d.kit, t("Fertig"), d.f.flag != 0) {
            d.kit_key.clear();
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    // configuration backup
    if ui.dialog_begin(D_CFGB, t("Konfiguration sichern"), &d.cfg_backup.clone()) {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP, t("Die Sicherung enthält die Datenbank und die Schlüssel des Servers. Sie wird mit dieser Passphrase verschlüsselt."));
        field(ui, t("Passphrase"));
        ui.next_size(420, 0);
        ui.textbox(D_CFGB + 1, None, &mut d.f.a, 256 | TB_PASSWORD);
        field(ui, t("Passphrase wiederholen"));
        ui.next_size(420, 0);
        ui.textbox(D_CFGB + 2, None, &mut d.f.b, 256 | TB_PASSWORD);
        let n = d.f.a.chars().count();
        let err = if n > 0 && n < 12 { t("Mindestens 12 Zeichen.") } else if !d.f.b.is_empty() && d.f.a != d.f.b { t("Die Eingaben stimmen nicht überein.") } else { "" };
        widgets::error(ui, err);
        if buttons(ui, &d.cfg_backup, t("Sichern"), err.is_empty() && n >= 12 && d.f.a == d.f.b) {
            let body = api::ConfigBackupRequest { passphrase: d.f.a.clone() }.to_json();
            d.f.a.clear();
            d.f.b.clear();
            app.net.action(Method::Post, "/api/config/backups", Some(body), "config.backup", &["/api/config/backups"], After::Operation);
        }
    }
    ui.dialog_end();
    let d = &mut app.dlg;
    if ui.dialog_begin(D_CFGR, t("Konfiguration wiederherstellen"), &d.cfg_restore.clone()) {
        ui.next_size(420, 0);
        ui.infobar(D_CFGR + 0x80, IB_WARNING, t("Neustart nötig"), t("Wird beim nächsten Start des Servers angewendet."));
        field(ui, t("Sicherung"));
        ui.text(TS_BODY, &d.f.a.clone());
        field(ui, t("Passphrase"));
        ui.next_size(420, 0);
        ui.textbox(D_CFGR + 2, None, &mut d.f.b, 256 | TB_PASSWORD);
        if buttons(ui, &d.cfg_restore, t("Vormerken"), !d.f.b.is_empty()) {
            let body = api::ConfigRestoreRequest { name: d.f.a.clone(), passphrase: d.f.b.clone() }.to_json();
            d.f.b.clear();
            app.net.action(Method::Post, "/api/config/restore", Some(body), "config.restore", &[], After::Nothing);
        }
    }
    ui.dialog_end();
    replica_dialog(app, ui);
    plan_dialog(app, ui);
    fleet_dialog(app, ui);
    fleet_token_dialog(app, ui);
    tape_dialogs(app, ui);
    vm_dialogs(app, ui);
}

/// the point and VM choosers of the VM dialogs
fn point_and_vm(d: &mut Dialogs, ui: &mut Ui, id: u32) {
    field(ui, t("Restore-Point"));
    let labels: Vec<&str> = d.vm_points.iter().map(|(_, l, _)| l.as_str()).collect();
    let mut k = d.vm_points.iter().position(|(p, _, _)| *p == d.f.id).map_or(0, |p| p as i32);
    ui.next_size(440, 0);
    if ui.combobox(id, &mut k, &labels) & EV_ACT != 0 {
        if let Some((p, _, names)) = d.vm_points.get(k.max(0) as usize) {
            d.f.id = p.clone();
            d.f.a = names.first().cloned().unwrap_or_default();
        }
    }
    field(ui, t("VM"));
    let names: Vec<String> = d.vm_points.iter().find(|(p, _, _)| *p == d.f.id).map(|(_, _, n)| n.clone()).unwrap_or_default();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let mut v = names.iter().position(|n| *n == d.f.a).map_or(0, |p| p as i32);
    ui.next_size(440, 0);
    if ui.combobox(id + 1, &mut v, &refs) & EV_ACT != 0 {
        if let Some(n) = names.get(v.max(0) as usize) {
            d.f.a = n.clone();
        }
    }
}

fn vm_dialogs(app: &mut Console, ui: &mut Ui) {
    // instant start
    let open = app.dlg.instant.clone();
    if ui.dialog_begin(D_INSTANT, t("VM aus einem Restore-Point starten"), &open) {
        let d = &mut app.dlg;
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Die VM startet auf diesem Server direkt aus dem Repository. Was sie schreibt, landet in einer eigenen Datei; der Restore-Point bleibt unverändert."));
        point_and_vm(d, ui, D_INSTANT + 1);
        field(ui, t("Arbeitsspeicher (MB, 0 = wie gesichert)"));
        ui.next_size(160, 0);
        ui.number_box(D_INSTANT + 3, &mut d.f.n, &Range::new(0, 262_144, 64, 0));
        field(ui, t("Netzwerk"));
        ui.next_size(440, 0);
        ui.segmented(D_INSTANT + 4, &mut d.f.flag, &[t("Kein Netzwerk"), t("Isoliert")]);
        ui.next_size(440, 0);
        widgets::hint(ui, t("Isoliert: ein eigenes Netz ohne Verbindung nach außen."));
        if buttons(ui, &d.instant, t("Starten"), !d.f.a.is_empty()) {
            let body = api::InstantRequest { vm: d.f.a.clone(), memory_mb: d.f.n.max(0) as u64, network: String::from(if d.f.flag == 1 { "isolated" } else { "none" }) }.to_json();
            app.net.action(Method::Post, &format!("/api/points/{}/instant-start", d.f.id), Some(body), "instant.start", &["/api/instant-start"], After::Operation);
        }
    }
    ui.dialog_end();
    // boot check
    let open = app.dlg.boot_check.clone();
    if ui.dialog_begin(D_BOOT_CHECK, t("Startprüfung"), &open) {
        let d = &mut app.dlg;
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Die VM startet ohne Netzwerkzugang aus dem Restore-Point und wird geprüft; danach wird alles verworfen."));
        point_and_vm(d, ui, D_BOOT_CHECK + 1);
        field(ui, t("Läuft mindestens (Sekunden)"));
        ui.next_size(160, 0);
        ui.number_box(D_BOOT_CHECK + 3, &mut d.f.n, &Range::new(1, 600, 5, 0));
        field(ui, t("Text auf der seriellen Konsole (optional)"));
        ui.next_size(440, 0);
        ui.textbox(D_BOOT_CHECK + 4, Some(t("z. B. login:")), &mut d.f.b, 200);
        field(ui, t("Antwortet auf Port (optional)"));
        ui.next_size(160, 0);
        ui.textbox(D_BOOT_CHECK + 5, Some("22"), &mut d.f.c, 5);
        field(ui, t("Höchstens warten (Sekunden)"));
        ui.next_size(160, 0);
        ui.number_box(D_BOOT_CHECK + 6, &mut d.f.min, &Range::new(5, 3600, 15, 0));
        let port = d.f.c.trim().parse::<u32>().ok().filter(|p| *p <= 65535);
        let ok = !d.f.a.is_empty() && (d.f.c.trim().is_empty() || port.is_some());
        if buttons(ui, &d.boot_check, t("Prüfen"), ok) {
            let body = api::BootCheckRequest { vm: d.f.a.clone(), boot_seconds: d.f.n.max(1) as u32, console_text: String::from(d.f.b.trim()), tcp_port: port.unwrap_or(0),
                                                timeout_s: d.f.min.max(5) as u32 }.to_json();
            app.net.action(Method::Post, &format!("/api/points/{}/boot-check", d.f.id), Some(body), "bootcheck", &["/api/boot-checks", "/api/instant-start"], After::Operation);
        }
    }
    ui.dialog_end();
    // the console of a VM
    let open = app.dlg.console.clone();
    if ui.dialog_begin(D_CONSOLE, &tf("Konsole: {}", &[&app.dlg.f.a.clone()]), &open) {
        let now = ui.now();
        let path = format!("/api/instant-start/{}/console", app.dlg.f.id);
        app.net.want(&path, 2000, now);
        let mut text = app.net.data.consoles.get(&app.dlg.f.id).cloned().unwrap_or_default();
        ui.next_size(560, 300);
        ui.textbox(D_CONSOLE + 1, None, &mut text, 65536 | TB_MULTI | TB_READONLY);
        ui.dialog_buttons(None, None, Some(t("Schließen")));
    }
    ui.dialog_end();
    // merge into a folder
    let open = app.dlg.merge.clone();
    if ui.dialog_begin(D_MERGE, &tf("{} zusammenführen", &[&app.dlg.f.b.clone()]), &open) {
        let d = &mut app.dlg;
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Die Platten werden im laufenden Betrieb in den Ordner übertragen; danach läuft die VM von dort und braucht den Restore-Point nicht mehr."));
        field(ui, t("Zielordner (ein Wiederherstellungsordner des Servers)"));
        ui.next_size(440, 0);
        ui.textbox(D_MERGE + 1, None, &mut d.f.a, 1024);
        if buttons(ui, &d.merge, t("Zusammenführen"), !d.f.a.trim().is_empty()) {
            let body = api::MigrateRequest { target: String::from(d.f.a.trim()) }.to_json();
            app.net.action(Method::Post, &format!("/api/instant-start/{}/migrate", d.f.id), Some(body), "instant.migrate", &["/api/instant-start"], After::Operation);
        }
    }
    ui.dialog_end();
}

/// the pools as combobox choices (first: `none`), and the index of `id`
fn pool_choice(app: &Console, none: &str, id: &str) -> (Vec<String>, Vec<String>, i32) {
    let mut names = alloc::vec![String::from(none)];
    let mut ids = alloc::vec![String::new()];
    for p in &app.net.data.pools {
        names.push(p.name.clone());
        ids.push(p.id.clone());
    }
    let sel = ids.iter().position(|x| x == id).unwrap_or(0) as i32;
    (names, ids, sel)
}

fn tape_dialogs(app: &mut Console, ui: &mut Ui) {
    // new tapes
    let open = app.dlg.tapes_add.clone();
    if ui.dialog_begin(D_TADD, t("Bänder hinzufügen"), &open) {
        let (names, pids, _) = pool_choice(app, t("Frei (jeder Pool nimmt sie)"), "");
        let d = &mut app.dlg;
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Die virtuelle Bibliothek legt jedes Band als Datei an; sie wächst erst beim Schreiben."));
        field(ui, t("Anzahl"));
        ui.next_size(140, 0);
        ui.number_box(D_TADD + 1, &mut d.f.n, &Range::new(1, 100, 1, 0));
        field(ui, t("Kapazität je Band (GB)"));
        ui.next_size(140, 0);
        ui.number_box(D_TADD + 2, &mut d.f.min, &Range::new(1, 100_000, 100, 0));
        field(ui, t("Barcode-Präfix (2 bis 4 Buchstaben)"));
        ui.next_size(140, 0);
        ui.textbox(D_TADD + 3, Some("BC"), &mut d.f.a, 4);
        field(ui, t("Medienpool"));
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        ui.next_size(420, 0);
        ui.combobox(D_TADD + 4, &mut d.f.flag, &refs);
        let prefix = d.f.a.trim().to_ascii_uppercase();
        let ok = (2..=4).contains(&prefix.len()) && prefix.chars().all(|x| x.is_ascii_uppercase());
        if buttons(ui, &d.tapes_add, t("Anlegen"), ok) {
            let body = api::CartridgeRequest { count: d.f.n.clamp(1, 100) as u32, capacity_gb: d.f.min.clamp(1, 100_000) as u32, capacity_mb: 0, prefix,
                                               pool_id: pids.get(d.f.flag.max(0) as usize).cloned().unwrap_or_default() }.to_json();
            app.net.action(Method::Post, "/api/tape/tapes", Some(body), "tape.add", &["/api/tape/tapes", "/api/tape", "/api/tape/pools"], After::Nothing);
        }
    }
    ui.dialog_end();
    // a blank tape to a pool
    let open = app.dlg.tape_assign.clone();
    if ui.dialog_begin(D_TASSIGN, &tf("Band {}: Pool", &[&app.dlg.f.id.clone()]), &open) {
        let cur = app.dlg.f.a.clone();
        let (names, pids, sel0) = pool_choice(app, t("Frei"), &cur);
        let d = &mut app.dlg;
        if d.f.flag == 0 && sel0 > 0 && d.f.min == 0 {
            d.f.flag = sel0;
            d.f.min = 1;
        }
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        ui.next_size(360, 0);
        ui.combobox(D_TASSIGN + 1, &mut d.f.flag, &refs);
        if buttons(ui, &d.tape_assign, t("Zuordnen"), true) {
            let body = api::CartridgeUpdate { pool_id: pids.get(d.f.flag.max(0) as usize).cloned().unwrap_or_default() }.to_json();
            app.net.action(Method::Put, &format!("/api/tape/tapes/{}", d.f.id), Some(body), "tape.assign", &["/api/tape/tapes", "/api/tape/pools"], After::Nothing);
        }
    }
    ui.dialog_end();
    // a media pool
    let editing = !app.dlg.f.id.is_empty();
    let open = app.dlg.pool.clone();
    if ui.dialog_begin(D_POOL, if editing { t("Medienpool bearbeiten") } else { t("Medienpool anlegen") }, &open) {
        let d = &mut app.dlg;
        field(ui, t("Name"));
        ui.next_size(420, 0);
        ui.textbox(D_POOL + 1, Some(t("z. B. Wöchentlich")), &mut d.f.a, 80);
        field(ui, t("Beschreibung (optional)"));
        ui.next_size(420, 0);
        ui.textbox(D_POOL + 2, None, &mut d.f.b, 200);
        field(ui, t("Schreibschutz nach dem Schreiben (Tage, 0 = keiner)"));
        ui.next_size(140, 0);
        ui.number_box(D_POOL + 3, &mut d.f.n, &Range::new(0, 3650, 1, 0));
        ui.switch(D_POOL + 4, t("Verschlüsseln (XChaCha20-Poly1305, ein Schlüssel je Satz)"), &mut d.f.flag);
        ui.switch(D_POOL + 5, t("An beschriebene Bänder anhängen, bis sie voll sind"), &mut d.f.min);
        ui.next_size(420, 0);
        widgets::hint(ui, t("Mit Recovery-Kit lassen sich verschlüsselte Bänder auch ohne diesen Server öffnen: der Schlüssel jedes Satzes liegt für das Kit versiegelt auf dem Band."));
        if buttons(ui, &d.pool, t("Speichern"), !d.f.a.trim().is_empty()) {
            let body = api::PoolRequest { name: String::from(d.f.a.trim()), description: String::from(d.f.b.trim()), protect_days: d.f.n.clamp(0, 3650) as u32,
                                          encrypted: d.f.flag != 0, append: d.f.min != 0 }.to_json();
            if editing {
                app.net.action(Method::Put, &format!("/api/tape/pools/{}", d.f.id), Some(body), "pool.save", &["/api/tape/pools"], After::Nothing);
            } else {
                app.net.action(Method::Post, "/api/tape/pools", Some(body), "pool.save", &["/api/tape/pools"], After::Nothing);
            }
        }
    }
    ui.dialog_end();
    // a point to tape
    let open = app.dlg.tape_write.clone();
    if ui.dialog_begin(D_TWRITE, t("Auf Band schreiben"), &open) {
        let cur = app.dlg.f.b.clone();
        let (names, pids, sel) = pool_choice(app, t("Bitte wählen"), &cur);
        let d = &mut app.dlg;
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Der Restore-Point wird vollständig auf Bänder des Pools geschrieben – lesbar auch ohne Repository."));
        field(ui, t("Medienpool"));
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut k = sel;
        ui.next_size(420, 0);
        if ui.combobox(D_TWRITE + 1, &mut k, &refs) & EV_ACT != 0 {
            d.f.b = pids.get(k.max(0) as usize).cloned().unwrap_or_default();
        }
        if buttons(ui, &d.tape_write, t("Schreiben"), !d.f.b.is_empty()) {
            let body = api::TapeWriteRequest { pool_id: d.f.b.clone() }.to_json();
            app.net.action(Method::Post, &format!("/api/points/{}/tape", d.f.id), Some(body), "tape.write", &["/api/tape/sets", "/api/tape/tapes"], After::Operation);
        }
    }
    ui.dialog_end();
    // a restore from tape
    let open = app.dlg.set_restore.clone();
    if ui.dialog_begin(D_SREST, t("Vom Band wiederherstellen"), &open) {
        let operator = app.may(Role::Operator);
        let d = &mut app.dlg;
        field(ui, t("Zielhost"));
        let names: Vec<String> = d.hosts.iter().map(|h| h.name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut k = d.hosts.iter().position(|h| h.id == d.f.c).map_or(0, |p| p as i32);
        ui.next_size(440, 0);
        if ui.combobox(D_SREST + 1, &mut k, &refs) & EV_ACT != 0 {
            if let Some(h) = d.hosts.get(k.max(0) as usize) {
                d.f.c = h.id.clone();
                d.f.a = h.restore_roots.first().cloned().unwrap_or_default();
            }
        }
        field(ui, t("Zielordner"));
        ui.next_size(440, 0);
        ui.textbox(D_SREST + 2, None, &mut d.f.a, 1024);
        field(ui, t("Nur diese Pfade (einer pro Zeile; leer = alles)"));
        ui.next_size(440, 80);
        ui.textbox(D_SREST + 3, None, &mut d.f.b, 8192 | TB_MULTI);
        field(ui, t("Vorhandene Dateien"));
        let mut choices = alloc::vec![t("Überspringen"), t("Umbenennen")];
        if operator {
            choices.push(t("Ersetzen"));
        }
        ui.next_size(440, 0);
        ui.segmented(D_SREST + 4, &mut d.f.flag, &choices);
        if buttons(ui, &d.set_restore, t("Wiederherstellen"), !d.f.a.trim().is_empty()) {
            let conflict = match d.f.flag {
                0 => api::Conflict::Skip,
                1 => api::Conflict::Rename,
                _ => api::Conflict::Overwrite,
            };
            let items: Vec<String> = d.f.b.lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect();
            let body = api::RestoreRequest { items, original: false, target: String::from(d.f.a.trim()), conflict, host_id: d.f.c.clone(), ..api::RestoreRequest::default() }.to_json();
            app.net.action(Method::Post, &format!("/api/tape/sets/{}/restore", d.f.id), Some(body), "tape.restore", &[], After::Operation);
        }
    }
    ui.dialog_end();
}

fn fleet_dialog(app: &mut Console, ui: &mut Ui) {
    let editing = !app.dlg.f.id.is_empty();
    let open = app.dlg.fleet.clone();
    if ui.dialog_begin(D_FLEET, if editing { t("Überwachten Server bearbeiten") } else { t("Server überwachen") }, &open) {
        let d = &mut app.dlg;
        ui.next_size(460, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Ein anderer BackupCraft-Server meldet diesem seinen Zustand. Das Überwachungs-Token erzeugt man dort unter Einstellungen › Netzwerk."));
        field(ui, t("Name"));
        ui.next_size(460, 0);
        ui.textbox(D_FLEET + 1, Some(t("z. B. Hamburg")), &mut d.f.a, 120);
        field(ui, t("Adresse"));
        ui.next_size(460, 0);
        ui.textbox(D_FLEET + 2, Some("https://backup-hh01:7642"), &mut d.f.b, 300);
        field(ui, if editing { t("Überwachungs-Token (leer: unverändert)") } else { t("Überwachungs-Token") });
        ui.next_size(460, 0);
        ui.textbox(D_FLEET + 3, Some("bcf_…"), &mut d.f.c, 200 | TB_PASSWORD);
        // an address that was not confirmed yet needs its CA read first
        let addr = String::from(d.f.b.trim());
        let known = editing && d.f.pick.first().is_some_and(|u| *u == addr);
        if d.probe.as_ref().is_some_and(|p| bc_proto::server_address(&addr).as_deref() != Some(p.url.as_str())) {
            d.probe = None;
        }
        let probed = d.probe.clone();
        let mut ready = known;
        if let Some(p) = &probed {
            if p.tls {
                ui.next_size(460, 0);
                ui.box_begin(0, BOX_V | BOX_LAYER | pad(12), 6);
                ui.text(TS_BODY_STRONG, &tf("Zertifizierungsstelle: {}", &[&p.ca_name]));
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_WRAP, &p.fingerprint_text);
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, t("Mit dem Fingerabdruck vergleichen, den der Server beim Start protokolliert (oder Einstellungen › Netzwerk dort)."));
                ui.box_end();
                ui.checkbox(D_FLEET + 4, t("Der Fingerabdruck stimmt"), &mut d.f.flag);
                ready = d.f.flag != 0;
            } else {
                ui.next_size(460, 0);
                widgets::hint(ui, t("Unverschlüsselt über http – nur auf diesem Rechner (localhost) zulässig."));
                ready = true;
            }
        }
        widgets::error(ui, &d.error.clone());
        let filled = !d.f.a.trim().is_empty() && !addr.is_empty() && (editing || !d.f.c.trim().is_empty());
        let step = if ready { if editing { t("Speichern") } else { t("Überwachen") } } else { t("Weiter") };
        let ok = filled && !d.busy && (ready || probed.is_none());
        let pressed = buttons(ui, &d.fleet, step, ok);
        if pressed {
            // the dialog stays until the server answered
            d.fleet.set(1);
            d.busy = true;
            d.error.clear();
            if ready {
                let fp = probed.as_ref().map_or(String::new(), |p| p.fingerprint.clone());
                let body = api::FleetServerRequest { name: String::from(d.f.a.trim()), url: addr, token: String::from(d.f.c.trim()), fingerprint: fp }.to_json();
                if editing {
                    app.net.action(Method::Put, &format!("/api/fleet/servers/{}", d.f.id), Some(body), "fleet.save", &["/api/fleet", "/api/summary"], After::Nothing);
                } else {
                    app.net.action(Method::Post, "/api/fleet/servers", Some(body), "fleet.save", &["/api/fleet", "/api/summary"], After::Nothing);
                }
            } else {
                let body = api::ProbeRequest { url: addr }.to_json();
                app.net.action(Method::Post, "/api/fleet/probe", Some(body), "fleet.probe", &[], After::Nothing);
            }
        }
    }
    ui.dialog_end();
}

fn fleet_token_dialog(app: &mut Console, ui: &mut Ui) {
    let open = app.dlg.fleet_token.clone();
    if ui.dialog_begin(D_FTOK, t("Überwachungs-Token erzeugen"), &open) {
        let d = &mut app.dlg;
        if d.new_token.is_empty() {
            ui.next_size(440, 0);
            ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Mit dem Token liest eine Zentrale den Zustand dieses Servers – Jobs, Sitzungen, Speicher, Hinweise. Ändern oder wiederherstellen kann sie damit nichts."));
            field(ui, t("Bezeichnung"));
            ui.next_size(440, 0);
            ui.textbox(D_FTOK + 1, Some(t("z. B. Zentrale Frankfurt")), &mut d.f.a, 80);
            widgets::error(ui, &d.error.clone());
            if buttons(ui, &d.fleet_token, t("Erzeugen"), !d.f.a.trim().is_empty() && !d.busy) {
                d.fleet_token.set(1);
                d.busy = true;
                d.error.clear();
                let body = api::FleetTokenRequest { label: String::from(d.f.a.trim()) }.to_json();
                app.net.action(Method::Post, "/api/fleet/tokens", Some(body), "fleet.token", &["/api/fleet/tokens"], After::Nothing);
            }
        } else {
            ui.next_size(480, 0);
            ui.infobar(D_FTOK + 0x80, IB_WARNING, t("Nur jetzt sichtbar"), t("In der Zentrale unter Monitoring › Serverflotte eintragen."));
            ui.next_size(480, 60);
            let mut tok = d.new_token.clone();
            ui.textbox(D_FTOK + 3, None, &mut tok, 400 | TB_MULTI | TB_READONLY | TB_WRAP);
            ui.box_begin(0, BOX_H, 8);
            ui.next_icon(IC_COPY);
            if ui.button(D_FTOK + 4, t("Kopieren")) & EV_ACT != 0 {
                ui.clip_set(CLIP_TEXT, d.new_token.as_bytes());
                ui.toast(IB_SUCCESS, t("Kopiert"), t("Das Token liegt in der Zwischenablage."), 2500);
            }
            ui.box_end();
            if buttons(ui, &d.fleet_token, t("Fertig"), true) {
                d.new_token.clear();
            }
        }
    }
    ui.dialog_end();
}

fn replica_dialog(app: &mut Console, ui: &mut Ui) {
    let editing = !app.dlg.f.id.is_empty();
    let open = app.dlg.replica.clone();
    if ui.dialog_begin(D_REPL, if editing { t("Replikat bearbeiten") } else { t("Replikat anlegen") }, &open) {
        let jobs: Vec<Job> = app.net.data.jobs.iter().filter(|j| j.kind != JobKind::Copy).cloned().collect();
        let hosts: Vec<Host> = app.net.data.hosts.clone();
        let d = &mut app.dlg;
        field(ui, t("Name"));
        ui.next_size(460, 0);
        ui.textbox(D_REPL + 1, Some(t("z. B. Dateiserver – Standby")), &mut d.f.a, 120);
        field(ui, t("Job (sein neuester Restore-Point wird repliziert)"));
        let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
        let mut sel = jobs.iter().position(|j| j.id == d.f.c).map_or(-1, |p| p as i32);
        ui.next_size(460, 0);
        if ui.combobox(D_REPL + 2, &mut sel, &names) & EV_ACT != 0 && sel >= 0 {
            d.f.c = jobs[sel as usize].id.clone();
        }
        field(ui, t("Zielhost"));
        let hnames: Vec<String> = hosts.iter().map(|h| {
            let st = if h.status == HostStatus::Online { String::new() } else { format!(" ({})", fmt::host_status(h.status).0) };
            format!("{}{}", h.name, st)
        }).collect();
        let hrefs: Vec<&str> = hnames.iter().map(|s| s.as_str()).collect();
        let host_id = d.f.pick.first().cloned().unwrap_or_default();
        let mut hsel = hosts.iter().position(|h| h.id == host_id).map_or(-1, |p| p as i32);
        ui.next_size(460, 0);
        if ui.combobox(D_REPL + 3, &mut hsel, &hrefs) & EV_ACT != 0 && hsel >= 0 {
            let h = &hosts[hsel as usize];
            d.f.pick = alloc::vec![h.id.clone()];
            if d.f.b.is_empty() {
                if let Some(r) = h.restore_roots.first() {
                    let sep = if h.os == Os::Windows { '\\' } else { '/' };
                    d.f.b = format!("{}{}{}", r.trim_end_matches(['/', '\\']), sep, t("Replikat"));
                }
            }
        }
        field(ui, t("Ordner auf dem Zielhost"));
        ui.next_size(460, 0);
        ui.textbox(D_REPL + 4, Some("D:\\Restore\\Replikat"), &mut d.f.b, 1024);
        let roots = hosts.iter().find(|h| Some(&h.id) == d.f.pick.first()).map(|h| h.restore_roots.join(", ")).unwrap_or_default();
        ui.next_size(460, 0);
        widgets::hint(ui, &if roots.is_empty() { String::from(t("Der Ordner muss in einem beschreibbaren Ordner (Wiederherstellungsziel) des Zielhosts liegen.")) }
                           else { tf("Muss innerhalb von {} liegen. Jede Quelle des Jobs bekommt darin einen eigenen Unterordner.", &[&roots]) });
        ui.switch(D_REPL + 5, t("Nach jedem neuen Restore-Point abgleichen"), &mut d.f.flag);
        let ok = !d.f.a.trim().is_empty() && !d.f.c.is_empty() && !d.f.pick.is_empty() && !d.f.b.trim().is_empty();
        if buttons(ui, &d.replica, t("Speichern"), ok) {
            let body = api::ReplicaRequest { name: String::from(d.f.a.trim()), job_id: d.f.c.clone(), host_id: d.f.pick[0].clone(), target: String::from(d.f.b.trim()), auto: d.f.flag != 0 }.to_json();
            if editing {
                app.net.action(Method::Put, &format!("/api/replicas/{}", d.f.id), Some(body), "replica.save", &["/api/replicas"], After::Nothing);
            } else {
                app.net.action(Method::Post, "/api/replicas", Some(body), "replica.save", &["/api/replicas"], After::Nothing);
            }
        }
    }
    ui.dialog_end();
}

fn plan_dialog(app: &mut Console, ui: &mut Ui) {
    let editing = !app.dlg.f.id.is_empty();
    let open = app.dlg.plan.clone();
    if ui.dialog_begin(D_PLAN, if editing { t("Failover-Plan bearbeiten") } else { t("Failover-Plan anlegen") }, &open) {
        let reps: Vec<Replica> = app.net.data.replicas.clone();
        let d = &mut app.dlg;
        field(ui, t("Name"));
        ui.next_size(440, 0);
        ui.textbox(D_PLAN + 1, Some(t("z. B. Standort ausgefallen")), &mut d.f.a, 120);
        field(ui, t("Replikate (in der Reihenfolge des Anklickens)"));
        ui.next_size(440, 0);
        ui.box_begin(0, BOX_V | BOX_LAYER | pad(10), 4);
        for (i, r) in reps.iter().enumerate() {
            let pos = d.f.pick.iter().position(|x| *x == r.id);
            let mut on = pos.is_some() as i32;
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(28, 0);
            ui.text(TS_BODY_STRONG | TX_SECONDARY, &pos.map_or(String::new(), |p| format!("{}.", p + 1)));
            if ui.checkbox(D_PLAN + 0x10 + i as u32, &format!("{} · {}", r.name, r.host_name), &mut on) & EV_ACT != 0 {
                if on != 0 {
                    d.f.pick.push(r.id.clone());
                } else {
                    d.f.pick.retain(|x| *x != r.id);
                }
            }
            ui.box_end();
        }
        ui.box_end();
        // ids of replicas that are gone meanwhile
        d.f.pick.retain(|x| reps.iter().any(|r| r.id == *x));
        field(ui, t("Pause zwischen zwei Replikaten (Sekunden)"));
        ui.next_size(140, 0);
        ui.number_box(D_PLAN + 2, &mut d.f.n, &Range::new(0, 3600, 5, 0));
        let ok = !d.f.a.trim().is_empty() && !d.f.pick.is_empty();
        if buttons(ui, &d.plan, t("Speichern"), ok) {
            let body = api::PlanRequest { name: String::from(d.f.a.trim()), replicas: d.f.pick.clone(), delay_s: d.f.n.clamp(0, 3600) as u32 }.to_json();
            if editing {
                app.net.action(Method::Put, &format!("/api/failover-plans/{}", d.f.id), Some(body), "plan.save", &["/api/failover-plans"], After::Nothing);
            } else {
                app.net.action(Method::Post, "/api/failover-plans", Some(body), "plan.save", &["/api/failover-plans"], After::Nothing);
            }
        }
    }
    ui.dialog_end();
}

fn run_confirm(app: &mut Console, w: Confirm) {
    let n = &mut app.net;
    match w {
        Confirm::DeleteJobs(ids) => {
            for id in ids {
                n.action(Method::Delete, &format!("/api/jobs/{}", id), None, "job.delete", &["/api/jobs", "/api/summary"], After::Nothing);
            }
        }
        Confirm::DeletePoint(pid, jid) => {
            let refresh = format!("/api/jobs/{}/points", jid);
            n.action(Method::Delete, &format!("/api/points/{}", pid), None, "point.delete", &[&refresh], After::Nothing);
        }
        Confirm::RemoveHost(id) => {
            n.action(Method::Delete, &format!("/api/hosts/{}", id), None, "host.remove", &["/api/hosts"], After::Nothing);
        }
        Confirm::DeleteCreds(ids) => {
            for id in ids {
                n.action(Method::Delete, &format!("/api/credentials/{}", id), None, "credential.delete", &["/api/credentials"], After::Nothing);
            }
        }
        Confirm::DeleteUser(id) => {
            n.action(Method::Delete, &format!("/api/users/{}", id), None, "user.delete", &["/api/users"], After::Nothing);
        }
        Confirm::Gc(id) => {
            n.action(Method::Post, &format!("/api/repositories/{}/gc", id), None, "repository.gc", &[], After::Operation);
        }
        Confirm::DetachRepo(id) => {
            n.action(Method::Delete, &format!("/api/repositories/{}", id), None, "repository.detach", &["/api/repositories"], After::Nothing);
        }
        Confirm::RecoveryKit => {
            n.action(Method::Post, "/api/recovery/kit", None, "recovery.kit", &["/api/recovery"], After::RecoveryKit);
        }
        Confirm::Failover(id) => {
            n.action(Method::Post, &format!("/api/replicas/{}/failover", id), None, "replica.failover", &["/api/replicas"], After::Nothing);
        }
        Confirm::Failback(id) => {
            n.action(Method::Post, &format!("/api/replicas/{}/failback", id), None, "replica.failback", &["/api/replicas"], After::Operation);
        }
        Confirm::UndoFailover(id) => {
            n.action(Method::Post, &format!("/api/replicas/{}/undo", id), None, "replica.undo", &["/api/replicas"], After::Operation);
        }
        Confirm::DeleteReplica(id) => {
            n.action(Method::Delete, &format!("/api/replicas/{}", id), None, "replica.delete", &["/api/replicas"], After::Nothing);
        }
        Confirm::RunPlan(id) => {
            n.action(Method::Post, &format!("/api/failover-plans/{}/run", id), None, "plan.run", &["/api/failover-plans", "/api/replicas"], After::Operation);
        }
        Confirm::DeletePlan(id) => {
            n.action(Method::Delete, &format!("/api/failover-plans/{}", id), None, "plan.delete", &["/api/failover-plans"], After::Nothing);
        }
        Confirm::RemoveFleet(id) => {
            n.action(Method::Delete, &format!("/api/fleet/servers/{}", id), None, "fleet.remove", &["/api/fleet", "/api/summary"], After::Nothing);
        }
        Confirm::RevokeFleetToken(id) => {
            n.action(Method::Delete, &format!("/api/fleet/tokens/{}", id), None, "fleet.revoke", &["/api/fleet/tokens"], After::Nothing);
        }
        Confirm::EraseTape(id) => {
            n.action(Method::Post, &format!("/api/tape/tapes/{}/erase", id), None, "tape.erase", &["/api/tape/tapes", "/api/tape/sets", "/api/tape"], After::Operation);
        }
        Confirm::RemoveTape(id) => {
            n.action(Method::Delete, &format!("/api/tape/tapes/{}", id), None, "tape.remove", &["/api/tape/tapes", "/api/tape/sets", "/api/tape"], After::Nothing);
        }
        Confirm::StopInstant(id) => {
            n.action(Method::Post, &format!("/api/instant-start/{}/stop", id), None, "instant.stop", &["/api/instant-start"], After::Nothing);
        }
        Confirm::DeletePool(id) => {
            n.action(Method::Delete, &format!("/api/tape/pools/{}", id), None, "pool.delete", &["/api/tape/pools", "/api/tape/tapes"], After::Nothing);
        }
    }
}
