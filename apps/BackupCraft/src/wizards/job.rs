// job -- the job wizard: name and kind, source (host and a file tree, or the job a copy or
// tape job takes the points of), target (repository, or media pool for tape jobs), retention
// (points, days, GFS; tape jobs: the pool protects), encryption and options,
// schedule and continuous protection (CDP), summary (start after saving).  Editing keeps the
// encryption key unless a new passphrase is given.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use ui::*;

use crate::app::Console;
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::widgets::picker::Picker;
use crate::widgets::schedule::{self, Sched};
use crate::wizards::{page_begin, page_end, tile, ID_WJOB};
use crate::{fmt, widgets};

pub struct JobWiz {
    step: usize,
    edit: Option<String>,
    req: api::JobRequest,
    host: String,
    picker: Option<Picker>,
    excludes: String,
    sched: Sched,
    pass1: String,
    pass2: String,
    change_key: i32,
    run_after: i32,
    full_days: i32,
    retention: [i32; 5],
    encrypt: i32,
    compress: i32,
    cdp: i32,
    cdp_rpo: i32,
    cdp_keep: i32,
    pub busy: bool,
    pub error: String,
}

const STEPS: usize = 7;

/// the choices of the compression segments, in their order
const COMPRESSIONS: [Compression; 4] = [Compression::None, Compression::Lz4, Compression::Zstd, Compression::ZstdHigh];

pub fn compression_name(c: Compression) -> &'static str {
    match c {
        Compression::None => t("Aus"),
        Compression::Lz4 => t("Schnell (LZ4)"),
        Compression::Zstd => t("Ausgewogen (zstd)"),
        Compression::ZstdHigh => t("Hoch (zstd 6)"),
    }
}

impl JobWiz {
    pub fn new(app: &Console, job: Option<&Job>) -> JobWiz {
        let server = app.net.data.hosts.iter().find(|h| h.kind == HostKind::Server).map_or(String::new(), |h| h.id.clone());
        let repo = app.net.data.repos.first().map_or(String::new(), |r| r.id.clone());
        match job {
            Some(j) => {
                let mut p = Picker::host(&j.host_id);
                p.set_selected(&j.sources);
                JobWiz {
                    step: 0,
                    edit: Some(j.id.clone()),
                    req: api::JobRequest {
                        name: j.name.clone(),
                        description: j.description.clone(),
                        kind: j.kind,
                        enabled: j.enabled,
                        host_id: j.host_id.clone(),
                        sources: j.sources.clone(),
                        excludes: j.excludes.clone(),
                        repository_id: j.repository_id.clone(),
                        schedule: j.schedule.clone(),
                        retention: j.retention.clone(),
                        encrypted: j.encrypted,
                        passphrase: String::new(),
                        key_hint: j.key_hint.clone(),
                        compression: j.compression,
                        active_full_days: j.active_full_days,
                        pre_script: j.pre_script.clone(),
                        post_script: j.post_script.clone(),
                        source_job: j.source_job.clone(),
                        pool_id: j.pool_id.clone(),
                        cdp: j.cdp,
                        cdp_rpo: j.cdp_rpo,
                        cdp_keep_hours: j.cdp_keep_hours,
                    },
                    host: j.host_id.clone(),
                    picker: Some(p),
                    excludes: j.excludes.join("\n"),
                    sched: Sched::from_schedule(&j.schedule),
                    pass1: String::new(),
                    pass2: String::new(),
                    change_key: 0,
                    run_after: 0,
                    full_days: j.active_full_days as i32,
                    retention: [j.retention.points as i32, j.retention.days as i32, j.retention.weekly as i32, j.retention.monthly as i32, j.retention.yearly as i32],
                    encrypt: j.encrypted as i32,
                    compress: COMPRESSIONS.iter().position(|c| *c == j.compression).unwrap_or(1) as i32,
                    cdp: j.cdp as i32,
                    cdp_rpo: if j.cdp_rpo == 0 { 60 } else { j.cdp_rpo as i32 },
                    cdp_keep: j.cdp_keep_hours as i32,
                    busy: false,
                    error: String::new(),
                }
            }
            None => JobWiz {
                step: 0,
                edit: None,
                req: api::JobRequest { kind: JobKind::File, enabled: true, repository_id: repo, ..api::JobRequest::default() },
                host: server.clone(),
                picker: if server.is_empty() { None } else { Some(Picker::host(&server)) },
                excludes: String::from("*.tmp\n~$*\nThumbs.db"),
                sched: Sched::default(),
                pass1: String::new(),
                pass2: String::new(),
                change_key: 1,
                run_after: 0,
                full_days: 0,
                retention: [14, 30, 8, 12, 0],
                encrypt: 1,
                compress: 1,
                cdp: 0,
                cdp_rpo: 60,
                cdp_keep: 24,
                busy: false,
                error: String::new(),
            },
        }
    }

    pub fn run_after(&self, body: &Value) -> Option<String> {
        if self.run_after != 0 {
            let id = j::str_of(body, "id");
            if !id.is_empty() {
                return Some(String::from(id));
            }
        }
        None
    }

    pub fn done(&mut self, _body: &Value) -> bool {
        self.busy = false;
        true
    }

    fn sources(&self) -> Vec<String> {
        self.picker.as_ref().map(|p| p.selected()).unwrap_or_default()
    }

    /// the problem of a step ("" = fine)
    fn check(&self, step: usize) -> &'static str {
        match step {
            0 if self.req.name.trim().is_empty() => t("Bitte einen Namen eingeben."),
            1 if self.takes_points() && self.req.source_job.is_empty() => t("Bitte den Job wählen, dessen Stände kopiert werden."),
            1 if self.req.kind == JobKind::Qemu && self.req.sources.is_empty() => t("Bitte mindestens eine VM wählen."),
            1 if self.req.kind == JobKind::Volume && self.req.sources.is_empty() => t("Bitte mindestens ein Volume wählen."),
            1 if self.req.kind == JobKind::File && self.host.is_empty() => t("Bitte einen Host wählen."),
            1 if self.req.kind == JobKind::File && self.sources().is_empty() => t("Bitte mindestens einen Ordner oder eine Datei auswählen."),
            2 if self.req.kind == JobKind::Tape && self.req.pool_id.is_empty() => t("Bitte einen Medienpool wählen."),
            2 if self.req.kind != JobKind::Tape && self.req.repository_id.is_empty() => t("Bitte ein Repository wählen."),
            4 if self.req.kind != JobKind::Tape && self.encrypt != 0 && (self.edit.is_none() || self.change_key != 0) && self.pass1.chars().count() < 12 => t("Die Passphrase braucht mindestens 12 Zeichen."),
            4 if self.req.kind != JobKind::Tape && self.encrypt != 0 && (self.edit.is_none() || self.change_key != 0) && self.pass1 != self.pass2 => t("Die Passphrasen stimmen nicht überein."),
            5 if !self.sched.error().is_empty() => t("Der Zeitplan ist ungültig."),
            5 if self.cdp != 0 && self.req.kind == JobKind::File && !(15..=86_400).contains(&self.cdp_rpo) => t("Der Abstand der CDP-Punkte liegt zwischen 15 Sekunden und einem Tag."),
            _ => "",
        }
    }

    /// copy and tape jobs take the points of another job
    fn takes_points(&self) -> bool {
        matches!(self.req.kind, JobKind::Copy | JobKind::Tape)
    }

    fn request(&self) -> api::JobRequest {
        let mut r = self.req.clone();
        r.host_id = if self.req.kind == JobKind::File { self.host.clone() } else { String::new() };
        r.sources = match r.kind {
            JobKind::File => self.sources(),
            JobKind::Qemu | JobKind::Volume => self.req.sources.clone(),
            _ => Vec::new(),
        };
        r.excludes = self.excludes.lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect();
        r.schedule = self.sched.to_schedule();
        r.retention = Retention {
            points: self.retention[0].max(0) as u32,
            days: self.retention[1].max(0) as u32,
            weekly: self.retention[2].max(0) as u32,
            monthly: self.retention[3].max(0) as u32,
            yearly: self.retention[4].max(0) as u32,
        };
        r.encrypted = self.encrypt != 0 && r.kind != JobKind::Tape;
        r.passphrase = if r.encrypted && (self.edit.is_none() || self.change_key != 0) { self.pass1.clone() } else { String::new() };
        r.compression = COMPRESSIONS[self.compress.clamp(0, 3) as usize];
        r.active_full_days = self.full_days.max(0) as u32;
        r.cdp = self.cdp != 0 && r.kind == JobKind::File;
        r.cdp_rpo = self.cdp_rpo.clamp(15, 86_400) as u32;
        r.cdp_keep_hours = self.cdp_keep.clamp(0, 720) as u32;
        r
    }

    pub fn frame(&mut self, app: &mut Console, ui: &mut Ui) -> bool {
        let id = ID_WJOB;
        let steps = [t("Name und Art"), t("Quelle"), t("Ziel"), t("Aufbewahrung"), t("Verschlüsselung"), t("Zeitplan"), t("Zusammenfassung")];
        let states: Vec<i32> = (0..STEPS).map(|i| if i < self.step { if self.check(i).is_empty() { WS_DONE } else { WS_ERROR } } else { WS_TODO }).collect();
        let title = if self.edit.is_some() { tf("Job bearbeiten: {}", &[&self.req.name]) } else { String::from(t("Neuer Sicherungsjob")) };
        let (clicked, close) = page_begin(ui, id, &title, t("Was wird gesichert, wohin, wie lange und wann."), &steps, self.step, &states);
        if let Some(s) = clicked {
            self.step = s;
        }
        match self.step {
            0 => self.step_name(app, ui, id),
            1 => self.step_source(app, ui, id),
            2 => self.step_target(app, ui, id),
            3 => self.step_retention(ui, id),
            4 => self.step_encryption(ui, id),
            5 => {
                let now = app.epoch(ui);
                ui.text(TS_SUBTITLE, t("Zeitplan"));
                schedule::editor(ui, id + 0x1000, &mut self.sched, now);
                if self.req.kind == JobKind::File && (self.cdp != 0 || app.net.data.capable(api::CAP_CDP)) {
                    self.step_cdp(ui, id);
                }
                if self.takes_points() {
                    widgets::hint(ui, t("Ohne Zeitplan läuft der Job nach jedem erfolgreichen Lauf seines Quelljobs."));
                }
            }
            _ => self.step_summary(app, ui, id),
        }
        let err = self.check(self.step);
        let shown = if !self.error.is_empty() { self.error.clone() } else { String::new() };
        let r = page_end(ui, id + 0x100, self.step == 0, self.step + 1 == STEPS, err.is_empty(), self.busy, &shown,
                         if self.edit.is_some() { t("Speichern") } else { t("Job anlegen") });
        match r {
            1 => self.step = self.step.saturating_sub(1),
            2 if self.step + 1 < STEPS => {
                self.step += 1;
                self.error.clear();
            }
            2 => {
                // save: the first step with a problem wins
                if let Some(bad) = (0..STEPS).find(|s| !self.check(*s).is_empty()) {
                    self.step = bad;
                } else {
                    self.busy = true;
                    self.error.clear();
                    let body = self.request().to_json();
                    match &self.edit {
                        Some(jid) => {
                            app.net.action(Method::Put, &format!("/api/jobs/{}", jid), Some(body), "job.save", &["/api/jobs", "/api/summary"], After::CloseWizard);
                        }
                        None => {
                            app.net.action(Method::Post, "/api/jobs", Some(body), "job.save", &["/api/jobs", "/api/summary"], After::CloseWizard);
                        }
                    }
                }
            }
            3 => return false,
            _ => {}
        }
        !close
    }

    fn step_name(&mut self, app: &Console, ui: &mut Ui, id: u32) {
        let tape = app.net.data.capable(api::CAP_TAPE);
        let vmcap = app.net.data.capable(api::CAP_HYPERVISOR);
        let volcap = app.net.data.capable(api::CAP_VOLUME);
        ui.text(TS_SUBTITLE, t("Name und Art"));
        widgets::label(ui, t("Name"));
        ui.next_size(480, 0);
        ui.textbox(id + 0x200, Some(t("z. B. Dateiserver – täglich")), &mut self.req.name, 120);
        widgets::label(ui, t("Beschreibung (optional)"));
        ui.next_size(480, 0);
        ui.textbox(id + 0x201, None, &mut self.req.description, 400);
        widgets::label(ui, t("Was soll gesichert werden?"));
        let kinds = [
            (JobKind::File, BC_DISK, t("Dateien und Ordner"), t("Ordner und Dateien auf dem Server selbst oder auf einem Agenten. Unveränderte Dateien werden nicht erneut gelesen."), true, None),
            (JobKind::Copy, BC_COPY_JOB, t("Sicherungskopie"), t("Die Restore-Points eines anderen Jobs in ein zweites Repository kopieren (3-2-1-Regel)."), true, None),
            (JobKind::Tape, BC_TAPE, t("Auf Band"), t("Den neuesten Restore-Point eines anderen Jobs vollständig auf Band schreiben – lesbar ohne Repository."), tape, if tape { None } else { Some(t("bald")) }),
            (JobKind::Volume, BC_REPOSITORY, t("Ganzes Volume"), t("Blockweise Sicherung eines Laufwerks des Backup-Servers mit Snapshot (VSS / LVM)."), volcap, if volcap { None } else { Some(t("bald")) }),
            (JobKind::Qemu, BC_VM, t("Virtuelle Maschinen (KVM)"), t("QEMU/KVM-VMs, im Betrieb konsistent durch QEMU gesichert; danach nur geänderte Blöcke."), vmcap, if vmcap { None } else { Some(t("bald")) }),
            (JobKind::HyperV, BC_VM, t("Hyper-V"), t("Hyper-V-VMs mit Prüfpunkten und Änderungsverfolgung."), false, Some(t("bald"))),
        ];
        for (i, (k, icon, title, text, enabled, badge)) in kinds.iter().enumerate() {
            if tile(ui, id + 0x210 + i as u32, *icon, title, text, self.req.kind == *k, *enabled, *badge) {
                self.req.kind = *k;
            }
        }
    }

    fn step_source(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        if self.req.kind == JobKind::Volume {
            let now = ui.now();
            let server = app.net.data.hosts.iter().find(|h| h.kind == HostKind::Server).map_or(String::new(), |h| h.id.clone());
            let path = format!("/api/hosts/{}/volumes", server);
            app.net.want(&path, 30_000, now);
            ui.text(TS_SUBTITLE, t("Welche Volumes des Backup-Servers?"));
            let vols: Vec<Volume> = app.net.data.volumes.get(&server).cloned().unwrap_or_default();
            for (i, v) in vols.iter().enumerate() {
                let mut on = self.req.sources.contains(&v.id) as i32;
                let name = if v.label.is_empty() { v.id.clone() } else { format!("{} ({})", v.id, v.label) };
                let label = format!("{} · {} · {} {}", name, v.fs, fmt::size(v.size), if v.mount.is_empty() { String::new() } else { format!("· {}", v.mount) });
                if ui.checkbox(id + 0x360 + i as u32, &label, &mut on) & EV_ACT != 0 {
                    if on != 0 {
                        self.req.sources.push(v.id.clone());
                    } else {
                        self.req.sources.retain(|x| *x != v.id);
                    }
                }
            }
            widgets::hint(ui, t("Ein Snapshot (Windows: VSS, Linux: LVM) hält das Volume während des Lesens fest; dafür braucht der Dienst Administrator- bzw. Root-Rechte. Unveränderte Blöcke speichert das Repository nur einmal. Volumes von Agenten folgen."));
            return;
        }
        if self.req.kind == JobKind::Qemu {
            let now = ui.now();
            app.net.want("/api/vms", 10_000, now);
            ui.text(TS_SUBTITLE, t("Welche VMs werden gesichert?"));
            let vms: Vec<Vm> = app.net.data.vms.clone();
            for (i, v) in vms.iter().enumerate() {
                let mut on = self.req.sources.contains(&v.id) as i32;
                let size: u64 = v.disks.iter().map(|d| d.size).sum();
                let label = format!("{} · {} · {} MB · {}", v.name, crate::views::recovery::vm_state(&v.state).0, v.memory_mb, fmt::size(size));
                if ui.checkbox(id + 0x340 + i as u32, &label, &mut on) & EV_ACT != 0 {
                    if on != 0 {
                        self.req.sources.push(v.id.clone());
                    } else {
                        self.req.sources.retain(|x| *x != v.id);
                    }
                }
            }
            if vms.is_empty() && app.net.loaded("/api/vms") {
                ui.infobar(id + 0x3F0, IB_WARNING, t("Keine VMs gefunden"), t("Unter „Sofortstart und Startprüfung“ steht, wie BackupCraft VMs findet."));
            }
            widgets::hint(ui, t("Laufende VMs sichert QEMU selbst – konsistent über alle Platten, ab dem zweiten Lauf nur geänderte Blöcke. Ausgeschaltete VMs werden aus ihren Plattendateien gelesen."));
            return;
        }
        if self.takes_points() {
            ui.text(TS_SUBTITLE, if self.req.kind == JobKind::Tape { t("Welcher Job kommt auf Band?") } else { t("Welcher Job wird kopiert?") });
            let jobs: Vec<Job> = app.net.data.jobs.iter().filter(|j| matches!(j.kind, JobKind::File | JobKind::Copy) && (j.kind != JobKind::Copy || self.req.kind == JobKind::Tape) && Some(&j.id) != self.edit.as_ref()).cloned().collect();
            for (i, j) in jobs.iter().enumerate() {
                let repo = app.net.data.repo(&j.repository_id).map_or(String::new(), |r| r.name.clone());
                if tile(ui, id + 0x300 + i as u32, BC_JOB, &j.name, &tf("{} Restore-Points in „{}“", &[&format!("{}", j.points), &repo]),
                        self.req.source_job == j.id, true, None) {
                    self.req.source_job = j.id.clone();
                }
            }
            return;
        }
        ui.text(TS_SUBTITLE, t("Quelle"));
        widgets::label(ui, t("Host"));
        let hosts: Vec<Host> = app.net.data.hosts.clone();
        let names: Vec<String> = hosts.iter().map(|h| {
            let st = if h.status == HostStatus::Online { String::new() } else { format!(" ({})", fmt::host_status(h.status).0) };
            format!("{} · {}{}", h.name, if h.kind == HostKind::Server { t("Backup-Server") } else { t("Agent") }, st)
        }).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let mut sel = hosts.iter().position(|h| h.id == self.host).map_or(-1, |p| p as i32);
        ui.next_size(480, 0);
        if ui.combobox(id + 0x320, &mut sel, &refs) & EV_ACT != 0 && sel >= 0 {
            let h = &hosts[sel as usize];
            self.host = h.id.clone();
            self.picker = Some(Picker::host(&h.id));
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        widgets::label(ui, t("Ordner und Dateien"));
        ui.spacer(-1);
        let n = self.sources().len();
        ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} ausgewählt", &[&format!("{}", n)]));
        ui.box_end();
        let now = ui.now();
        ui.next_size(-1, 300);
        ui.box_begin(0, BOX_V | BOX_LAYER | BOX_CLIP, 0);
        if let Some(p) = &mut self.picker {
            ui.next_size(-1, -1);
            p.frame(ui, &mut app.net, id + 0x400, now);
        }
        ui.box_end();
        widgets::hint(ui, t("Angeboten werden nur die Ordner, die der Host für Sicherungen freigegeben hat."));
        widgets::label(ui, t("Ausschließen (ein Muster pro Zeile)"));
        ui.next_size(480, 90);
        ui.textbox(id + 0x330, Some("*.tmp"), &mut self.excludes, 4096 | TB_MULTI);
    }

    fn step_target(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Ziel"));
        if self.req.kind == JobKind::Tape {
            let now = ui.now();
            app.net.want("/api/tape/pools", 15_000, now);
            widgets::hint(ui, t("Der Medienpool bestimmt Bänder, Schreibschutz und Verschlüsselung."));
            let pools: Vec<MediaPool> = app.net.data.pools.clone();
            for (i, p) in pools.iter().enumerate() {
                let text = format!("{} {} · {} / {} · {}", p.tapes, t("Bänder"), fmt::size(p.used), fmt::size(p.capacity),
                                   if p.protect_days == 0 { String::from(t("ohne Schreibschutz")) } else { tf("{} Tage geschützt", &[&format!("{}", p.protect_days)]) });
                if tile(ui, id + 0x500 + i as u32, BC_TAPE, &p.name, &text, self.req.pool_id == p.id, true, if p.encrypted { Some(t("verschlüsselt")) } else { None }) {
                    self.req.pool_id = p.id.clone();
                }
            }
            if pools.is_empty() && app.net.loaded("/api/tape/pools") {
                ui.infobar(id + 0x5F0, IB_WARNING, t("Kein Medienpool"), t("Bitte zuerst unter „Tape“ einen Pool anlegen und Bänder hinzufügen."));
            }
            return;
        }
        widgets::hint(ui, t("Das Repository speichert dedupliziert: Daten, die schon ein anderer Job gesichert hat, belegen keinen zusätzlichen Platz."));
        let repos: Vec<Repository> = app.net.data.repos.clone();
        for (i, r) in repos.iter().enumerate() {
            let text = format!("{} · {} {} · {} {}", r.path, fmt::percent(r.used, r.capacity), t("belegt"), fmt::size(r.free), t("frei"));
            if tile(ui, id + 0x500 + i as u32, BC_REPOSITORY, &r.name, &text, self.req.repository_id == r.id, r.status != RepoStatus::Offline,
                    if r.immutable_days > 0 { Some(t("unveränderlich")) } else { None }) {
                self.req.repository_id = r.id.clone();
            }
            ui.next_size(-1, 6);
            ui.meter(id + 0x580 + i as u32, r.used as i64, r.capacity as i64, fmt::fill_kind(r.used, r.capacity));
        }
        if repos.is_empty() {
            ui.infobar(id + 0x5F0, IB_WARNING, t("Kein Repository"), t("Bitte zuerst unter „Repositories“ eines hinzufügen."));
        }
    }

    fn step_retention(&mut self, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Aufbewahrung"));
        if self.req.kind == JobKind::Tape {
            widgets::hint(ui, t("Wie lange ein Band unangetastet bleibt, bestimmt der Schreibschutz seines Medienpools. Gelöscht wird ein Band nur von Hand, nach Ablauf des Schutzes."));
            return;
        }
        widgets::hint(ui, t("Ein Stand bleibt, solange eine der Regeln ihn braucht. Jeder Restore-Point ist vollständig wiederherstellbar – es gibt keine Ketten, die man pflegen müsste."));
        let rows = [
            (t("Mindestens die letzten Restore-Points"), 0usize, 1000),
            (t("Alle Stände der letzten Tage"), 1, 3650),
            (t("Je einen pro Woche, Wochen"), 2, 520),
            (t("Je einen pro Monat, Monate"), 3, 240),
            (t("Je einen pro Jahr, Jahre"), 4, 50),
        ];
        for (label, k, max) in rows {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
            ui.next_size(320, 0);
            ui.text(TS_BODY, label);
            ui.next_size(120, 0);
            ui.number_box(id + 0x600 + k as u32, &mut self.retention[k], &Range::new(0, max, 1, 0));
            ui.box_end();
        }
        let r = Retention {
            points: self.retention[0].max(0) as u32,
            days: self.retention[1].max(0) as u32,
            weekly: self.retention[2].max(0) as u32,
            monthly: self.retention[3].max(0) as u32,
            yearly: self.retention[4].max(0) as u32,
        };
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(12), 8);
        ui.icon(BC_ARCHIVE, 16);
        ui.text(TS_BODY_STRONG, &crate::views::jobs::retention_text(&r));
        ui.box_end();
    }

    fn step_encryption(&mut self, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Verschlüsselung und Optionen"));
        if self.req.kind == JobKind::Tape {
            widgets::hint(ui, t("Ob die Bänder verschlüsselt werden, bestimmt der Medienpool (ein eigener Schlüssel je Satz)."));
            return;
        }
        ui.switch(id + 0x700, t("Restore-Points verschlüsseln (XChaCha20-Poly1305)"), &mut self.encrypt);
        if self.encrypt != 0 {
            if self.edit.is_some() && self.req.encrypted {
                ui.switch(id + 0x701, t("Neue Passphrase festlegen (ab dem nächsten Lauf)"), &mut self.change_key);
            }
            if self.edit.is_none() || self.change_key != 0 || !self.req.encrypted {
                widgets::label(ui, t("Passphrase"));
                ui.next_size(380, 0);
                ui.textbox(id + 0x702, None, &mut self.pass1, 256 | TB_PASSWORD);
                widgets::label(ui, t("Passphrase wiederholen"));
                ui.next_size(380, 0);
                ui.textbox(id + 0x703, None, &mut self.pass2, 256 | TB_PASSWORD);
                let n = self.pass1.chars().count();
                ui.next_size(380, 0);
                ui.progress(id + 0x704, n.min(24) as i32, 24);
                widgets::hint(ui, t("Mindestens 12 Zeichen. Der Server bewahrt die Passphrase versiegelt auf; das Recovery-Kit öffnet die Daten auch ohne sie."));
            }
            widgets::label(ui, t("Hinweis zur Passphrase (optional, unverschlüsselt gespeichert)"));
            ui.next_size(380, 0);
            ui.textbox(id + 0x705, Some(t("z. B. Tresor › BackupCraft")), &mut self.req.key_hint, 120);
        }
        ui.separator();
        widgets::label(ui, t("Kompression"));
        ui.next_size(480, 0);
        // the segments are id + 0x711 .. 0x714
        ui.segmented(id + 0x710, &mut self.compress, &[t("Aus"), t("Schnell"), t("Ausgewogen"), t("Hoch")]);
        widgets::hint(ui, match self.compress {
            0 => t("Daten werden unkomprimiert gespeichert (sinnvoll für bereits komprimierte Medien)."),
            1 => t("LZ4: sehr schnell, mäßige Ersparnis."),
            2 => t("zstd Stufe 3: deutlich kleiner, etwa halb so schnell wie LZ4."),
            _ => t("zstd Stufe 6: am kleinsten, rechenintensiv – für Jobs mit viel Zeit und wenig Speicher."),
        });
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(320, 0);
        ui.text(TS_BODY, t("Alles neu lesen (aktive Vollsicherung) alle … Tage"));
        ui.next_size(120, 0);
        ui.number_box(id + 0x718, &mut self.full_days, &Range::new(0, 365, 1, 0));
        ui.box_end();
        widgets::hint(ui, t("0 = nie. Sonst vergleicht BackupCraft Größe, Änderungszeit und Datei-ID und liest nur Geänderte."));
        widgets::label(ui, t("Skript davor (auf dem Host, optional)"));
        ui.next_size(480, 0);
        ui.textbox(id + 0x719, Some("/usr/local/bin/freeze.sh"), &mut self.req.pre_script, 512);
        widgets::label(ui, t("Skript danach"));
        ui.next_size(480, 0);
        ui.textbox(id + 0x71A, Some("/usr/local/bin/thaw.sh"), &mut self.req.post_script, 512);
        widgets::hint(ui, t("Skripte müssen auf dem Host in einem freigegebenen Skriptordner liegen; beliebige Befehle führt der Agent nicht aus."));
    }

    /// continuous data protection below the schedule
    fn step_cdp(&mut self, ui: &mut Ui, id: u32) {
        ui.separator();
        ui.switch(id + 0x900, t("Laufend sichern (CDP)"), &mut self.cdp);
        widgets::hint(ui, t("Änderungen in den Quellen werden überwacht; kurz danach entsteht ein Restore-Point – höchstens einer pro RPO. Unveränderte Ordner werden dabei nicht gelesen."));
        if self.cdp == 0 {
            return;
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(320, 0);
        ui.text(TS_BODY, t("RPO: höchstens ein Punkt alle … Sekunden"));
        ui.next_size(120, 0);
        ui.number_box(id + 0x901, &mut self.cdp_rpo, &Range::new(15, 86_400, 15, 0));
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(320, 0);
        ui.text(TS_BODY, t("Alle CDP-Punkte behalten für … Stunden"));
        ui.next_size(120, 0);
        ui.number_box(id + 0x902, &mut self.cdp_keep, &Range::new(0, 720, 1, 0));
        ui.box_end();
        widgets::hint(ui, t("Danach gilt wieder die normale Aufbewahrung. Der Zeitplan oben kann zusätzlich laufen (etwa für eine nächtliche Vollprüfung) oder ausgeschaltet bleiben."));
    }

    fn step_summary(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Zusammenfassung"));
        let r = self.request();
        let d = &app.net.data;
        let host = d.host(&self.host).map_or(String::from("–"), |h| h.name.clone());
        let repo = if r.kind == JobKind::Tape {
            d.pools.iter().find(|p| p.id == r.pool_id).map_or(String::from("–"), |x| x.name.clone())
        } else {
            d.repo(&r.repository_id).map_or(String::from("–"), |x| x.name.clone())
        };
        let src = if self.takes_points() {
            d.job(&r.source_job).map_or(String::from("–"), |j| j.name.clone())
        } else if r.kind == JobKind::Volume {
            tf("{} Volumes: {}", &[&format!("{}", r.sources.len()), &r.sources.join(", ")])
        } else if r.kind == JobKind::Qemu {
            let names: Vec<String> = r.sources.iter().map(|id| d.vms.iter().find(|v| v.id == *id).map_or(id.clone(), |v| v.name.clone())).collect();
            tf("{} VMs: {}", &[&format!("{}", names.len()), &names.join(", ")])
        } else {
            tf("{} Einträge auf {}", &[&format!("{}", r.sources.len()), &host])
        };
        let enc = if r.encrypted { t("Ja") } else { t("Nein") };
        let ret = crate::views::jobs::retention_text(&r.retention);
        let mut sched = self.sched.describe();
        if r.cdp {
            sched = tf("{} · laufend (RPO {})", &[&sched, &crate::views::replication::rpo_text(r.cdp_rpo)]);
        }
        widgets::props(ui, &[
            (t("Name"), &r.name),
            (t("Art"), crate::views::jobs::kind_name(r.kind)),
            (t("Quelle"), &src),
            (if r.kind == JobKind::Tape { t("Medienpool") } else { t("Repository") }, &repo),
            (t("Aufbewahrung"), &ret),
            (t("Verschlüsselung"), enc),
            (t("Kompression"), compression_name(r.compression)),
            (t("Zeitplan"), &sched),
            (t("Zeitzone"), &self.sched.tz),
        ]);
        if r.kind == JobKind::File {
            ui.text(TS_BODY_STRONG, t("Einträge"));
            for s in r.sources.iter().take(12) {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.icon(IC_FOLDER, 16);
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_ELLIPSIS, s);
                ui.box_end();
            }
        }
        let mut en = self.req.enabled as i32;
        ui.switch(id + 0x801, t("Job aktiv (Zeitplan läuft)"), &mut en);
        self.req.enabled = en != 0;
        ui.checkbox(id + 0x802, t("Nach dem Speichern sofort starten"), &mut self.run_after);
    }
}
