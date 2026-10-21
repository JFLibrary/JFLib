// fixture -- an in-process demo server: the REST routes of bc_proto::api over in-memory data
// (servers and agents, repositories, nine jobs, two weeks of sessions and restore points,
// users, audit), deterministic for screenshots and scripts (--fixture).  Running jobs,
// restores and checks advance with the clock and finish with a session, a restore point and
// an audit entry, like the real server.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use bc_proto::api::{self, code, route_match, Access, Method};
use bc_proto::ids;
use bc_proto::j::{self, list, obj, Json, Value};
use bc_proto::model::*;
use datetime::{Cron, Timestamp, Zone};

use crate::net::Response;

/// 2026-10-12 19:58:00 UTC: the demo's "now" at start
pub const FIX_EPOCH: i64 = 1_791_835_080;

const DAY: i64 = 86_400;
const TB: u64 = 1_000_000_000_000;
const GB: u64 = 1_000_000_000;

struct Run {
    op: String,
    session: String,
    job: String,
    kind: SessionKind,
    started: i64,
    total: u64,
    files: u64,
    speed: u64,
    point: String,
}

pub struct Fixture {
    now: i64,
    rng: u64,
    out: Vec<Response>,
    users: Vec<(User, String)>,
    tokens: Vec<(String, String)>,
    hosts: Vec<Host>,
    repos: Vec<Repository>,
    jobs: Vec<Job>,
    sessions: Vec<Session>,
    points: Vec<RestorePoint>,
    ops: Vec<Operation>,
    creds: Vec<Credential>,
    audit: Vec<AuditEntry>,
    settings: Vec<Setting>,
    backups: Vec<ConfigBackup>,
    recovery: RecoveryStatus,
    runs: Vec<Run>,
    pairing: Vec<PairingCode>,
    replicas: Vec<Replica>,
    plans: Vec<FailoverPlan>,
    fleet: Vec<FleetServer>,
    fleet_tokens: Vec<FleetToken>,
    pools: Vec<MediaPool>,
    tapes: Vec<Cartridge>,
    tape_sets: Vec<TapeSet>,
    vms: Vec<Vm>,
    instant: Vec<InstantSession>,
    boot_checks: Vec<BootCheckResult>,
    zone: Zone,
}

fn s(x: &str) -> String {
    String::from(x)
}

fn err(status: u16, c: &str, m: &str) -> (u16, Value) {
    (status, obj([("code", Value::string(c)), ("message", Value::string(m))]))
}

impl Fixture {
    pub fn new(now: i64) -> Fixture {
        let mut f = Fixture {
            now,
            rng: 0x9E37_79B9_7F4A_7C15,
            out: Vec::new(),
            users: Vec::new(),
            tokens: Vec::new(),
            hosts: Vec::new(),
            repos: Vec::new(),
            jobs: Vec::new(),
            sessions: Vec::new(),
            points: Vec::new(),
            ops: Vec::new(),
            creds: Vec::new(),
            audit: Vec::new(),
            settings: Vec::new(),
            backups: Vec::new(),
            recovery: RecoveryStatus::default(),
            runs: Vec::new(),
            pairing: Vec::new(),
            replicas: Vec::new(),
            plans: Vec::new(),
            fleet: Vec::new(),
            fleet_tokens: Vec::new(),
            pools: Vec::new(),
            tapes: Vec::new(),
            tape_sets: Vec::new(),
            vms: Vec::new(),
            instant: Vec::new(),
            boot_checks: Vec::new(),
            zone: Zone::by_name("Europe/Berlin").unwrap_or_else(Zone::utc),
        };
        f.populate();
        f
    }

    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn id(&mut self) -> String {
        let a = self.rand();
        let b = self.rand();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&a.to_le_bytes());
        bytes[8..].copy_from_slice(&b.to_le_bytes());
        ids::uuid(bytes)
    }

    fn next_run(&self, cron: &str, enabled: bool) -> i64 {
        if !enabled || cron.is_empty() {
            return 0;
        }
        Cron::parse(cron).ok().and_then(|c| c.next_after(Timestamp::from_secs(self.now), &self.zone)).map_or(0, |t| t.secs)
    }

    // ------------------------------------------------------------------ demo data

    fn populate(&mut self) {
        let now = self.now;
        // users
        for (name, role) in [("admin", Role::Admin), ("j.mueller", Role::Operator), ("helpdesk", Role::Restore), ("revision", Role::Viewer)] {
            let id = self.id();
            self.users.push((User { id, username: s(name), role, created: now - 180 * DAY, last_login: now - 3600, disabled: false }, s("demo")));
        }
        // hosts
        let hosts: [(&str, &str, Os, &str, HostKind, HostStatus, &[&str]); 7] = [
            ("backup-srv01", "10.0.10.5", Os::Linux, "Ubuntu 24.04 LTS", HostKind::Server, HostStatus::Online, &["/srv", "/etc", "/home"]),
            ("fs01.corp.local", "10.0.20.11", Os::Windows, "Windows Server 2022", HostKind::Agent, HostStatus::Online, &["D:\\Daten", "E:\\Medien"]),
            ("sql01.corp.local", "10.0.20.15", Os::Windows, "Windows Server 2022", HostKind::Agent, HostStatus::Online, &["C:\\SQLBackup"]),
            ("mail01.corp.local", "10.0.20.21", Os::Linux, "Debian 12", HostKind::Agent, HostStatus::Online, &["/var/mail", "/etc"]),
            ("web01.corp.local", "10.0.30.8", Os::Linux, "Ubuntu 22.04 LTS", HostKind::Agent, HostStatus::Online, &["/var/www", "/etc/nginx"]),
            ("dc01.corp.local", "10.0.20.2", Os::Windows, "Windows Server 2019", HostKind::Agent, HostStatus::Online, &["C:\\Windows\\NTDS", "C:\\Windows\\SYSVOL"]),
            ("dev-vm07", "10.0.40.77", Os::Linux, "Ubuntu 22.04 LTS", HostKind::Agent, HostStatus::Offline, &["/home", "/opt/builds"]),
        ];
        for (i, (name, addr, os, platform, kind, status, roots)) in hosts.iter().enumerate() {
            let id = self.id();
            let mut fp = [0u8; 32];
            for (k, b) in fp.iter_mut().enumerate() {
                *b = (self.rand() >> (k % 7)) as u8;
            }
            self.hosts.push(Host {
                id,
                name: s(name),
                address: s(addr),
                os: *os,
                platform: s(platform),
                kind: *kind,
                status: *status,
                version: s(bc_proto::VERSION),
                last_seen: if *status == HostStatus::Online { now - 20 - i as i64 * 7 } else { now - 2 * DAY - 4000 },
                fingerprint: ids::hex(&fp),
                roots: roots.iter().map(|r| s(r)).collect(),
                restore_roots: roots.iter().map(|r| if *os == Os::Windows { s("D:\\Restore") } else { format!("{}/restore", r) }).take(1).collect(),
                message: if *status == HostStatus::Offline { s("Keine Antwort seit 2 Tagen") } else { String::new() },
                created: now - (120 - i as i64 * 9) * DAY,
            });
        }
        // repositories: name, path, disk size, used, logical, dedupe, immutable days, encrypted,
        // other data on the disk
        let repos: [(&str, &str, u64, u64, u64, u32, u32, bool, u64); 3] = [
            ("Primär (NAS)", "/mnt/nas/backupcraft", 20 * TB, 14_200 * GB, 41_000 * GB, 289, 0, true, 1_100 * GB),
            ("Offsite (S3-Gateway)", "/mnt/offsite/backupcraft", 30 * TB, 11_400 * GB, 29_600 * GB, 260, 30, true, 0),
            ("Archiv", "/srv/archive", 8 * TB, 7_440 * GB, 15_800 * GB, 212, 365, false, 160 * GB),
        ];
        for (name, path, cap, used, logical, dedup, imm, enc, other) in repos {
            let free = cap - used - other;
            let id = self.id();
            self.repos.push(Repository {
                id,
                name: s(name),
                path: s(path),
                status: if free * 10 < cap { RepoStatus::Warning } else { RepoStatus::Ok },
                capacity: cap,
                used,
                free,
                logical,
                points: 0,
                dedup_x100: dedup,
                encrypted: enc,
                immutable_days: imm,
                message: if free * 10 < cap { s("Weniger als 10 % frei") } else { String::new() },
                created: now - 200 * DAY,
            });
        }
        // jobs: name, host index, sources, repo index, cron, kind, base size, last status
        let jobs: [(&str, usize, &[&str], usize, &str, JobKind, u64, SessionStatus, bool); 9] = [
            ("Dateiserver – täglich", 1, &["D:\\Daten"], 0, "0 2 * * *", JobKind::File, 182 * GB, SessionStatus::Success, true),
            ("SQL-Cluster Logs", 2, &["C:\\SQLBackup"], 0, "30 1 * * *", JobKind::File, 21 * GB, SessionStatus::Warning, true),
            ("Mailserver", 3, &["/var/mail", "/etc"], 0, "0 23 * * *", JobKind::File, 96 * GB, SessionStatus::Success, true),
            ("Entwicklung VM-Images", 6, &["/opt/builds"], 2, "15 22 * * 1-5", JobKind::File, 512 * GB, SessionStatus::Failed, true),
            ("Webserver Konfiguration", 4, &["/var/www", "/etc/nginx"], 0, "10 3 * * *", JobKind::File, 1_200_000_000, SessionStatus::Success, true),
            ("Buchhaltung Archiv", 1, &["D:\\Daten\\Buchhaltung"], 2, "0 4 1 * *", JobKind::File, 84 * GB, SessionStatus::Never, false),
            ("Medien – wöchentlich", 1, &["E:\\Medien"], 1, "0 1 * * 6", JobKind::File, 340 * GB, SessionStatus::Success, true),
            ("Monatsabschluss (Kopie)", 0, &[], 1, "0 4 1 * *", JobKind::Copy, 340 * GB, SessionStatus::Success, true),
            ("Domänencontroller", 5, &["C:\\Windows\\NTDS", "C:\\Windows\\SYSVOL"], 0, "30 2 * * *", JobKind::File, 14 * GB, SessionStatus::Success, true),
        ];
        for (i, (name, hi, src, ri, cron, kind, size, last, enabled)) in jobs.iter().enumerate() {
            let id = self.id();
            let host = self.hosts[*hi].id.clone();
            let repo = self.repos[*ri].id.clone();
            let next = self.next_run(cron, *enabled);
            let mut job = Job {
                id,
                name: s(name),
                description: String::new(),
                kind: *kind,
                enabled: *enabled,
                host_id: host,
                sources: src.iter().map(|x| s(x)).collect(),
                excludes: if *hi == 1 { vec![s("*.tmp"), s("~$*")] } else { Vec::new() },
                repository_id: repo,
                schedule: Schedule { cron: s(cron), tz: s("Europe/Berlin"), enabled: *enabled },
                retention: Retention { points: 14, days: 30, weekly: 8, monthly: if i % 2 == 0 { 12 } else { 6 }, yearly: if i == 7 { 7 } else { 0 } },
                encrypted: *ri != 2,
                key_hint: if *ri != 2 { s("Tresor › BackupCraft › Jobschlüssel") } else { String::new() },
                compression: Compression::Lz4,
                active_full_days: if i % 3 == 0 { 30 } else { 0 },
                pre_script: if *hi == 2 { s("C:\\Skripte\\sql-freeze.ps1") } else { String::new() },
                post_script: if *hi == 2 { s("C:\\Skripte\\sql-thaw.ps1") } else { String::new() },
                source_job: String::new(),
                pool_id: String::new(),
                cdp: i == 1,
                cdp_rpo: 60,
                cdp_keep_hours: 24,
                last_status: *last,
                last_run: 0,
                last_size: *size,
                next_run: next,
                points: 0,
                created: now - (90 + i as i64 * 3) * DAY,
                updated: now - (10 + i as i64) * DAY,
            };
            if *kind == JobKind::Copy {
                job.source_job = self.jobs.first().map(|j| j.id.clone()).unwrap_or_default();
            }
            self.jobs.push(job);
        }
        // two weeks of history: sessions and restore points
        for d in (0..14).rev() {
            for ji in 0..self.jobs.len() {
                let job = self.jobs[ji].clone();
                if !job.enabled || job.schedule.cron.is_empty() {
                    continue;
                }
                let Ok(c) = Cron::parse(&job.schedule.cron) else { continue };
                let day_start = (now - d * DAY) - (now - d * DAY).rem_euclid(DAY) - 2 * 3600;
                let Some(at) = c.next_after(Timestamp::from_secs(day_start), &self.zone) else { continue };
                if at.secs >= now || at.secs < day_start || at.secs >= day_start + DAY {
                    continue;
                }
                let r = self.rand();
                let status = if d == 0 && ji < 9 {
                    if job.last_status == SessionStatus::Never { continue } else { job.last_status }
                } else if r % 23 == 0 {
                    SessionStatus::Failed
                } else if r % 9 == 0 {
                    SessionStatus::Warning
                } else {
                    SessionStatus::Success
                };
                self.add_session(ji, at.secs, status);
            }
        }
        // the copy job's and repositories' point counts
        for r in 0..self.repos.len() {
            let rid = self.repos[r].id.clone();
            let n = self.points.iter().filter(|p| self.jobs.iter().any(|j| j.id == p.job_id && j.repository_id == rid)).count();
            self.repos[r].points = n as u32;
        }
        // operations of the last hours
        let titles = ["Dateiserver – täglich: Sicherung", "Repository „Primär (NAS)“ prüfen", "Konfiguration sichern"];
        for (i, t) in titles.iter().enumerate() {
            let id = self.id();
            self.ops.push(Operation {
                id,
                kind: s(["job.run", "repository.check", "config.backup"][i]),
                title: s(t),
                owner: s("admin"),
                status: OpStatus::Succeeded,
                done: 100,
                total: 100,
                created: now - 7000 * (i as i64 + 1),
                started: now - 7000 * (i as i64 + 1),
                finished: now - 7000 * (i as i64 + 1) + 640,
                result: s("Abgeschlossen"),
                error: String::new(),
                session_id: String::new(),
                target_id: String::new(),
            });
        }
        // credentials
        for (label, user, kind) in [("Linux-Server (SSH-Schlüssel)", "backupcraft", CredKind::SshKey), ("Domänen-Dienstkonto", "CORP\\svc-backup", CredKind::Password)] {
            let id = self.id();
            self.creds.push(Credential { id, label: s(label), username: s(user), kind, created: now - 60 * DAY });
        }
        // audit
        let entries = [
            ("admin", "login", "", "Anmeldung von 10.0.10.44", true),
            ("j.mueller", "job.run", "Dateiserver – täglich", "Manuell gestartet", true),
            ("admin", "job.update", "SQL-Cluster Logs", "Aufbewahrung: 14 Punkte, 30 Tage", true),
            ("helpdesk", "restore", "Dateiserver – täglich", "3 Dateien nach D:\\Restore", true),
            ("admin", "host.pair", "web01.corp.local", "Agent gekoppelt", true),
            ("unbekannt", "login", "", "Falsches Passwort (10.0.99.3)", false),
            ("admin", "settings.update", "restore.allowed_roots", "D:\\Restore, /srv/restore", true),
            ("admin", "repository.create", "Archiv", "/srv/archive", true),
            ("admin", "user.create", "revision", "Rolle: Betrachter", true),
            ("admin", "config.backup", "", "Automatisch, verschlüsselt", true),
        ];
        for (i, (actor, action, target, detail, ok)) in entries.iter().enumerate() {
            let id = self.id();
            self.audit.push(AuditEntry { id, at: now - 3000 * (i as i64 + 1), actor: s(actor), action: s(action), target: s(target), detail: s(detail), ok: *ok });
        }
        // settings
        for (k, v) in [
            ("server.name", "backup-srv01"),
            ("restore.allowed_roots", "D:\\Restore\n/srv/restore"),
            ("traffic.max_mbps", "0"),
            ("notify.mode", "failure"),
            ("notify.daily", "07:00"),
            ("notify.webhook", ""),
            ("smtp.host", "mail.example.org:587"),
            ("smtp.security", "starttls"),
            ("smtp.user", "backup"),
            ("smtp.password", "********"),
            ("smtp.from", "backupcraft@example.org"),
            ("smtp.to", "it@example.org"),
            ("smtp.ca", ""),
            ("update.url", "https://updates.example.org/backupcraft/manifest.json"),
            ("update.key", ""),
            ("update.auto", "true"),
            ("session.timeout_hours", "24"),
            ("login.max_attempts", "5"),
            ("config.auto_backup", "true"),
            ("display.timezone", "Europe/Berlin"),
        ] {
            self.settings.push(Setting { key: s(k), value: s(v) });
        }
        for d in 0..5i64 {
            self.backups.push(ConfigBackup { name: format!("auto-{}.bcconfig", 20261012 - d), created: now - d * DAY - 3600 * 5, size: 182_000 + d as u64 * 211, auto: true });
        }
        self.recovery = RecoveryStatus { created: now - 150 * DAY, fingerprint: s("4f2a 91c0 7be3 1d55") };
        // replicas: name, job, target host, folder, state, auto, message
        let reps: [(&str, usize, usize, &str, ReplicaState, bool, &str); 4] = [
            ("Dateiserver – Standby", 0, 0, "/srv/restore/fs01", ReplicaState::Ready, true, ""),
            ("SQL-Logs – Standby", 1, 1, "D:\\Restore\\sql01", ReplicaState::Failed, true, "fs01.corp.local: Zugriff verweigert auf D:\\Restore\\sql01\\C\\SQLBackup"),
            ("Webserver – Standby", 4, 0, "/srv/restore/web01", ReplicaState::Failover, false, "Failover seit gestern 14:12: /srv/restore/web01 auf backup-srv01 ist in Gebrauch"),
            ("Mailserver – Standby", 2, 0, "/srv/restore/mail01", ReplicaState::New, false, ""),
        ];
        for (i, (name, ji, hi, target, state, auto, msg)) in reps.iter().enumerate() {
            let id = self.id();
            let job = &self.jobs[*ji];
            let host = &self.hosts[*hi];
            let newest = self.points.iter().filter(|p| p.job_id == job.id).max_by_key(|p| p.created).cloned();
            let synced = *state != ReplicaState::New;
            self.replicas.push(Replica {
                id,
                name: s(name),
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                target: s(target),
                auto: *auto,
                state: *state,
                point_id: if synced { newest.as_ref().map_or(String::new(), |p| p.id.clone()) } else { String::new() },
                point_time: if synced { newest.as_ref().map_or(0, |p| p.created) - if *state == ReplicaState::Failover { DAY } else { 0 } } else { 0 },
                synced: if synced { now - 3600 * (i as i64 + 2) } else { 0 },
                files: if synced { 48_000 + i as u64 * 9_000 } else { 0 },
                bytes: if synced { job.last_size } else { 0 },
                message: s(msg),
                failover_at: if *state == ReplicaState::Failover { now - DAY + 3600 } else { 0 },
                created: now - (40 - i as i64 * 5) * DAY,
            });
        }
        let order = [1usize, 0, 2].iter().map(|i| self.replicas[*i].id.clone()).collect();
        let id = self.id();
        self.plans.push(FailoverPlan { id, name: s("Standort Frankfurt ausgefallen"), replicas: order, delay_s: 120, last_run: now - 20 * DAY, last_status: s("success"),
                                       message: s("3 Replikate umgeschaltet"), created: now - 30 * DAY });
        let order = alloc::vec![self.replicas[2].id.clone()];
        let id = self.id();
        self.plans.push(FailoverPlan { id, name: s("Nur Webserver"), replicas: order, delay_s: 0, created: now - 12 * DAY, ..FailoverPlan::default() });
        // other sites this server watches: name, address, state, message, jobs, ok, warn, fail, used, capacity, seen
        let sites: [(&str, &str, &str, &str, u32, u32, u32, u32, u64, u64, i64); 3] = [
            ("Hamburg", "https://backup-hh01.corp.local:7642", "ok", "14 Jobs, 412 Restore-Points", 14, 14, 0, 0, 9_800 * GB, 24 * TB, 40),
            ("München", "https://backup-muc02.corp.local:7642", "warning", "Fileserver MUC mit Warnungen", 9, 8, 1, 0, 6_100 * GB, 8 * TB, 35),
            ("Berlin (Außenstelle)", "https://10.8.0.12:7642", "unreachable", "Verbindung abgelehnt (10.8.0.12:7642)", 4, 4, 0, 0, 1_900 * GB, 4 * TB, 3 * 3600 + 120),
        ];
        for (i, (name, url, status, msg, jobs, okc, warn, fail, used, cap, ago)) in sites.iter().enumerate() {
            let id = self.id();
            let mut alerts = Vec::new();
            if *warn > 0 {
                alerts.push(Alert { severity: Severity::Warning, title: s("Fileserver MUC mit Warnungen"), detail: s("3 Dateien waren gesperrt"), target_kind: s("job"), target_id: String::new(), at: now - 5 * 3600 });
            }
            let report = Summary {
                server_name: format!("backup-{}", ["hh01", "muc02", "ber03"][i]),
                version: s(if i == 2 { "0.0.9" } else { bc_proto::VERSION }),
                jobs: *jobs,
                jobs_enabled: *jobs,
                hosts: jobs / 2 + 1,
                hosts_online: jobs / 2 + 1,
                points: jobs * 29 + 6,
                ok_24h: *okc,
                warn_24h: *warn,
                fail_24h: *fail,
                storage_used: *used,
                storage_capacity: *cap,
                storage_free: if i == 2 { 0 } else { cap - used - cap / 20 }, // Berlin: an older server
                storage_logical: used * 27 / 10,
                alerts,
                ..Summary::default()
            };
            let mut fp = [0u8; 32];
            for b in fp.iter_mut() {
                *b = self.rand() as u8;
            }
            self.fleet.push(FleetServer {
                id,
                name: s(name),
                url: s(url),
                fingerprint: ids::hex(&fp),
                status: s(status),
                message: s(msg),
                checked: now - 40,
                seen: now - ago,
                report,
                created: now - (60 - i as i64 * 10) * DAY,
            });
        }
        let id = self.id();
        self.fleet_tokens.push(FleetToken { id, label: s("Zentrale Frankfurt"), created: now - 60 * DAY, created_by: s("admin"), last_used: now - 40 });
        self.populate_tape();
        self.populate_vm();
    }

    /// the server's KVM: three VMs, a VM job with a week of points, an instantly started VM,
    /// boot check results
    fn populate_vm(&mut self) {
        let now = self.now;
        let server = self.hosts[0].clone();
        let vms: [(&str, &str, u64, u32, &[(u64, &str)]); 3] = [
            ("dc02", "running", 4096, 2, &[(60, "virtio")]),
            ("erp-app", "running", 8192, 4, &[(80, "virtio"), (200, "virtio")]),
            ("test-vm", "stopped", 2048, 2, &[(40, "ide")]),
        ];
        for (name, state, mem, cpus, disks) in vms {
            self.vms.push(Vm {
                id: format!("{}:{}", server.id, name),
                name: s(name),
                uuid: format!("6a1c{:04x}-91d2-4f0e-8b7a-0c5e3d2f{:04x}", name.len() * 4099, mem),
                host_id: server.id.clone(),
                host_name: server.name.clone(),
                state: s(state),
                qmp: format!("/run/vms/{}.qmp", name),
                memory_mb: mem,
                cpus,
                disks: disks.iter().enumerate().map(|(k, (gb, bus))| VmDisk { node: format!("drive{}", k), file: format!("/var/lib/vms/{}-{}.qcow2", name, k), format: s("qcow2"), size: gb * GB, bus: s(bus) }).collect(),
                version: s("8.2.2"),
                seen: if state == "running" { now - 8 } else { now - 9 * DAY },
            });
        }
        let id = self.id();
        let next = self.next_run("0 1 * * *", true);
        self.jobs.push(Job {
            id,
            name: s("VMs – nachts"),
            kind: JobKind::Qemu,
            enabled: true,
            host_id: server.id.clone(),
            sources: self.vms[..2].iter().map(|v| v.id.clone()).collect(),
            repository_id: self.repos[0].id.clone(),
            schedule: Schedule { cron: s("0 1 * * *"), tz: s("Europe/Berlin"), enabled: true },
            retention: Retention { points: 14, days: 30, weekly: 8, monthly: 6, yearly: 0 },
            encrypted: true,
            key_hint: s("Tresor › BackupCraft › VMs"),
            compression: Compression::Zstd,
            last_size: 340 * GB,
            next_run: next,
            created: now - 40 * DAY,
            updated: now - 30 * DAY,
            ..Job::default()
        });
        let ji = self.jobs.len() - 1;
        for d in (0..7).rev() {
            let at = now - d * DAY - (now - d * DAY).rem_euclid(DAY) - 3600 + 3600;
            self.add_session(ji, at.min(now - 3600), SessionStatus::Success);
        }
        let jid = self.jobs[ji].id.clone();
        let pts: Vec<RestorePoint> = self.points.iter().filter(|p| p.job_id == jid).cloned().collect();
        for p in self.points.iter_mut().filter(|p| p.job_id == jid) {
            p.files = 5;
        }
        let newest = pts.iter().max_by_key(|p| p.created).cloned().unwrap_or_default();
        let older = pts.iter().min_by_key(|p| p.created).cloned().unwrap_or_default();
        let id = self.id();
        self.instant.push(InstantSession {
            id,
            point_id: newest.id.clone(),
            point_time: newest.created,
            job_id: jid.clone(),
            job_name: s("VMs – nachts"),
            vm: s("erp-app"),
            purpose: s("instant"),
            state: s("running"),
            memory_mb: 8192,
            cpus: 4,
            network: s("isolated"),
            started: now - 2400,
            owner: s("j.mueller"),
            message: s("läuft (KVM)"),
            ..InstantSession::default()
        });
        let id = self.id();
        self.instant.push(InstantSession {
            id,
            point_id: older.id.clone(),
            point_time: older.created,
            job_id: jid.clone(),
            job_name: s("VMs – nachts"),
            vm: s("dc02"),
            purpose: s("instant"),
            state: s("stopped"),
            memory_mb: 4096,
            cpus: 2,
            network: s("none"),
            started: now - 3 * DAY,
            stopped: now - 3 * DAY + 1800,
            owner: s("admin"),
            message: s("Beendet; die Änderungen wurden verworfen"),
            ..InstantSession::default()
        });
        for (k, p) in pts.iter().rev().take(4).enumerate() {
            for (vm, failed) in [("dc02", false), ("erp-app", k == 2)] {
                let id = self.id();
                let mut checks = alloc::vec![CheckResult { name: s("Start"), ok: true, detail: s("läuft seit 30 s") },
                                             CheckResult { name: s("Konsole"), ok: true, detail: s("„login:“ erschienen") }];
                if vm == "erp-app" {
                    checks.push(CheckResult { name: s("Port 443"), ok: !failed, detail: if failed { s("keine Antwort innerhalb von 180 s") } else { s("antwortet") } });
                }
                self.boot_checks.push(BootCheckResult {
                    id,
                    point_id: p.id.clone(),
                    point_time: p.created,
                    job_id: jid.clone(),
                    job_name: s("VMs – nachts"),
                    vm: s(vm),
                    started: p.created + 1800,
                    finished: p.created + 1800 + if failed { 214 } else { 46 },
                    ok: !failed,
                    console: format!("Ubuntu 24.04 LTS {} ttyS0\n\n{} login:", vm, vm),
                    message: format!("{}: {} von {} Prüfungen bestanden", vm, checks.iter().filter(|c| c.ok).count(), checks.len()),
                    checks,
                });
            }
        }
        self.boot_checks.sort_by_key(|x| core::cmp::Reverse(x.started));
    }

    /// the tape library: two pools, ten LTO-sized virtual tapes, sets of the file server and
    /// the month-end copies, a tape job
    fn populate_tape(&mut self) {
        let now = self.now;
        for (name, desc, days, enc, append) in [("Wöchentlich", "Samstags, vier Wochen geschützt", 28u32, false, true), ("Tresor", "Monatsende, ausgelagert", 365, true, false)] {
            let id = self.id();
            self.pools.push(MediaPool { id, name: s(name), description: s(desc), protect_days: days, encrypted: enc, append, created: now - 90 * DAY, ..MediaPool::default() });
        }
        let cap = 12 * TB;
        // barcode index, pool (0, 1, none), state, used share in %, written days ago
        let tapes: [(u32, Option<usize>, &str, u64, i64); 10] = [
            (1, Some(0), "full", 99, 21), (2, Some(0), "full", 98, 14), (3, Some(0), "full", 99, 7), (4, Some(0), "active", 41, 0),
            (5, Some(0), "blank", 0, 0), (6, Some(1), "full", 97, 43), (7, Some(1), "full", 96, 12), (8, Some(1), "missing", 95, 74),
            (9, None, "blank", 0, 0), (10, None, "blank", 0, 0),
        ];
        for (k, pool, state, pct, ago) in tapes {
            let p = pool.map(|i| self.pools[i].clone());
            let written = if pct > 0 { now - ago * DAY - 7200 } else { 0 };
            self.tapes.push(Cartridge {
                id: format!("BC{:04}V1", k),
                barcode: format!("BC{:04}V1", k),
                pool_id: p.as_ref().map_or(String::new(), |x| x.id.clone()),
                pool_name: p.as_ref().map_or(String::new(), |x| x.name.clone()),
                capacity: cap,
                used: cap * pct / 100 + 4096,
                state: s(state),
                sets: if pct > 0 { 1 + (k % 2) } else { 0 },
                protected_until: if pct > 0 { written + p.as_ref().map_or(0, |x| x.protect_days as i64) * DAY } else { 0 },
                written,
                created: now - 120 * DAY,
                message: if state == "missing" { s("Nicht in der Bibliothek (ausgelagert?)") } else { String::new() },
            });
        }
        let file_job = self.jobs[0].clone();
        let copy_job = self.jobs.iter().find(|j| j.kind == JobKind::Copy).cloned().unwrap_or_else(|| file_job.clone());
        let host = self.hosts.iter().find(|h| h.id == file_job.host_id).map_or(String::new(), |h| h.name.clone());
        // job, pool, tapes, written days ago, status, verified days ago
        let sets: [(usize, usize, &[u32], i64, &str, i64); 6] = [
            (0, 0, &[4], 0, "ok", -1), (0, 0, &[3, 4], 7, "ok", 6), (0, 0, &[2, 3], 14, "ok", 13), (0, 0, &[1, 2], 21, "ok", -1),
            (1, 1, &[7], 12, "ok", 10), (1, 1, &[8, 6], 43, "offline", 40),
        ];
        for (jk, pk, barcodes, ago, status, verified) in sets {
            let job = if jk == 0 { &file_job } else { &copy_job };
            let pool = self.pools[pk].clone();
            let id = self.id();
            let at = now - ago * DAY - 7200;
            self.tape_sets.push(TapeSet {
                id,
                pool_id: pool.id.clone(),
                pool_name: pool.name.clone(),
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                point_id: String::new(),
                point_time: at - 4 * 3600,
                host_name: host.clone(),
                windows: true,
                roots: file_job.sources.clone(),
                tapes: barcodes.iter().map(|b| format!("BC{:04}V1", b)).collect(),
                positions: barcodes.iter().map(|_| 4096).collect(),
                files: 412_000 + ago as u64 * 310,
                entries: 431_000 + ago as u64 * 320,
                bytes: 182 * GB + ago as u64 * GB,
                encrypted: pool.encrypted,
                written: at,
                verified: if verified >= 0 { now - verified * DAY } else { 0 },
                status: s(status),
            });
        }
        // a tape job: Saturdays, the file server
        let id = self.id();
        let next = self.next_run("0 6 * * 6", true);
        let pool = self.pools[0].id.clone();
        self.jobs.push(Job {
            id,
            name: s("Dateiserver – auf Band"),
            kind: JobKind::Tape,
            enabled: true,
            source_job: file_job.id.clone(),
            pool_id: pool,
            schedule: Schedule { cron: s("0 6 * * 6"), tz: s("Europe/Berlin"), enabled: true },
            last_status: SessionStatus::Success,
            last_run: now - 2 * DAY,
            last_size: 182 * GB,
            next_run: next,
            created: now - 60 * DAY,
            updated: now - 20 * DAY,
            ..Job::default()
        });
    }

    fn tape_status(&self) -> TapeStatus {
        TapeStatus {
            library: s("D:\\BackupCraft\\vtl"),
            ok: true,
            message: String::new(),
            tapes: self.tapes.len() as u32,
            blank: self.tapes.iter().filter(|c| c.state == "blank").count() as u32,
            missing: self.tapes.iter().filter(|c| c.state == "missing").count() as u32,
            used: self.tapes.iter().map(|c| c.used).sum(),
            capacity: self.tapes.iter().map(|c| c.capacity).sum(),
            sets: self.tape_sets.len() as u32,
        }
    }

    fn fleet_alerts(&self) -> Vec<Alert> {
        self.fleet.iter().filter_map(|f| {
            let (sev, title) = match f.status.as_str() {
                "unreachable" => (Severity::Critical, format!("Server {} nicht erreichbar", f.name)),
                "critical" => (Severity::Critical, format!("{}: {}", f.name, f.message)),
                "warning" => (Severity::Warning, format!("{}: {}", f.name, f.message)),
                _ => return None,
            };
            Some(Alert { severity: sev, title, detail: if f.status == "unreachable" { f.message.clone() } else { f.url.clone() }, target_kind: s("fleet"), target_id: f.id.clone(), at: f.checked })
        }).collect()
    }

    fn replica_at(&self, id: &str) -> Option<usize> {
        self.replicas.iter().position(|r| r.id == id)
    }

    fn cdp_status(&self) -> Vec<CdpStatus> {
        let now = self.now;
        self.jobs.iter().filter(|j| j.cdp).map(|j| {
            let host = self.hosts.iter().find(|h| h.id == j.host_id).map_or(String::new(), |h| h.name.clone());
            let ok = j.enabled;
            CdpStatus {
                job_id: j.id.clone(),
                job_name: j.name.clone(),
                host_name: host,
                rpo: j.cdp_rpo,
                keep_hours: j.cdp_keep_hours,
                state: s(if ok { "watching" } else { "stopped" }),
                pending: if ok { 3 } else { 0 },
                last_change: now - 41,
                last_point: now - 75,
                points: 37,
                message: String::new(),
            }
        }).collect()
    }

    fn add_session(&mut self, ji: usize, at: i64, status: SessionStatus) {
        let job = self.jobs[ji].clone();
        let r = self.rand();
        let size = job.last_size + (r % 1000) * job.last_size / 40_000;
        let read = if status == SessionStatus::Failed { size / 3 } else { size };
        let changed = size / 50 + r % (size / 80 + 1);
        let dur = 120 + (size / 60_000_000) as i64 + (r % 300) as i64;
        let sid = self.id();
        let point = if status != SessionStatus::Failed {
            let pid = self.id();
            let full = self.points.iter().all(|p| p.job_id != job.id);
            let repo_imm = self.repos.iter().find(|x| x.id == job.repository_id).map_or(0, |x| x.immutable_days);
            self.points.push(RestorePoint {
                id: pid.clone(),
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                created: at + dur,
                full,
                files: 12_000 + (size / 4_000_000),
                size,
                stored_new: if full { size * 100 / 289 } else { changed * 100 / 260 },
                immutable_until: if repo_imm > 0 { at + repo_imm as i64 * DAY } else { 0 },
                verified: if r % 4 == 0 { at + dur + 3600 } else { 0 },
                host: self.hosts.iter().find(|h| h.id == job.host_id).map_or(String::new(), |h| h.name.clone()),
                encrypted: job.encrypted,
                signed: true,
                repository_id: job.repository_id.clone(),
                copied_from: String::new(),
                created_ns: 0,
            });
            pid
        } else {
            String::new()
        };
        self.sessions.push(Session {
            id: sid,
            kind: if job.kind == JobKind::Copy { SessionKind::Copy } else { SessionKind::Backup },
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            status,
            started: at,
            finished: at + dur,
            files_total: 12_000 + (size / 4_000_000),
            files_done: if status == SessionStatus::Failed { 4_100 } else { 12_000 + (size / 4_000_000) },
            bytes_total: size,
            bytes_read: read,
            bytes_new: changed * 100 / 260,
            dedup_x100: 240 + (r % 90) as u32,
            compress_x100: 130 + (r % 60) as u32,
            warnings: if status == SessionStatus::Warning { 1 + (r % 4) as u32 } else { 0 },
            errors: if status == SessionStatus::Failed { 1 } else { 0 },
            message: match status {
                SessionStatus::Warning => s("2 Dateien waren gesperrt und wurden übersprungen"),
                SessionStatus::Failed => s("Agent dev-vm07 nicht erreichbar (Zeitüberschreitung)"),
                _ => String::new(),
            },
            operation_id: String::new(),
            point_id: point,
        });
        let j = &mut self.jobs[ji];
        j.last_run = at;
        j.last_status = status;
        if status != SessionStatus::Failed {
            j.points += 1;
        }
    }

    // ------------------------------------------------------------------ requests

    /// handle a request now; the answer is taken with take()
    pub fn request(&mut self, id: u64, method: Method, path: &str, body: Option<Value>, token: &str) {
        let (status, body) = self.handle(method, path, body.unwrap_or(Value::Null), token);
        self.out.push(Response { id, status, body });
    }

    pub fn has_answers(&self) -> bool {
        !self.out.is_empty()
    }

    pub fn take(&mut self) -> Vec<Response> {
        core::mem::take(&mut self.out)
    }

    fn user_of(&self, token: &str) -> Option<&User> {
        let uid = &self.tokens.iter().find(|(t, _)| t == token)?.1;
        self.users.iter().map(|(u, _)| u).find(|u| &u.id == uid)
    }

    fn log_audit(&mut self, actor: &str, action: &str, target: &str, detail: &str) {
        let id = self.id();
        self.audit.insert(0, AuditEntry { id, at: self.now, actor: s(actor), action: s(action), target: s(target), detail: s(detail), ok: true });
    }

    fn handle(&mut self, method: Method, path: &str, body: Value, token: &str) -> (u16, Value) {
        let Some((route, params)) = route_match(method.as_str(), path) else {
            return err(404, code::NOT_FOUND, "Unbekannter Pfad");
        };
        let param = |n: &str| params.iter().find(|(k, _)| *k == n).map(|(_, v)| String::from(*v)).unwrap_or_default();
        let actor = match route.access {
            Access::Public => String::from("anonym"),
            Access::Agent => String::from("agent"),
            Access::Fleet => String::from("fleet"),
            Access::Role(need) => match self.user_of(token) {
                None => return err(401, code::UNAUTHORIZED, "Bitte erneut anmelden"),
                Some(u) if !u.role.allows(need) => return err(403, code::FORBIDDEN, "Dafür fehlt die Berechtigung"),
                Some(u) => u.username.clone(),
            },
        };
        if !route.capability.is_empty() && !api::available(route.capability) {
            return err(501, code::CAPABILITY, "Diese Funktion ist in dieser Version noch nicht verfügbar");
        }
        let id = param("id");
        let now = self.now;
        match (method, route.path) {
            (Method::Get, "/health") => (200, obj([("status", Value::string("ok"))])),
            (Method::Get, "/api") => (200, ServerInfo {
                name: s(self.settings.iter().find(|x| x.key == "server.name").map_or("backup-srv01", |x| x.value.as_str())),
                version: s(bc_proto::VERSION),
                needs_setup: false,
                time: now,
                tls: true,
                ca_fingerprint: s("4F:2A:91:C0:7B:E3:1D:55:0C:93:5A:A2:E4:10:B7:1E:6D:28:3F:C4:91:0A:5E:77:D2:13:88:F0:6B:C9:24:A1"),
            }.to_json()),
            (Method::Post, "/api/auth/login") => {
                let user = j::str_of(&body, "username");
                let Some((u, _)) = self.users.iter().find(|(u, _)| u.username == user && !u.disabled) else {
                    return err(401, code::UNAUTHORIZED, "Benutzername oder Passwort falsch");
                };
                let u = u.clone();
                let tok = self.id().replace('-', "");
                self.tokens.push((tok.clone(), u.id.clone()));
                (200, api::LoginResponse { token: tok, expires: now + DAY, user: u }.to_json())
            }
            (Method::Post, "/api/auth/logout") => {
                self.tokens.retain(|(t, _)| t != token);
                (204, Value::Null)
            }
            (Method::Get, "/api/auth/me") => match self.user_of(token) {
                Some(u) => (200, u.to_json()),
                None => err(401, code::UNAUTHORIZED, ""),
            },
            (Method::Get, "/api/summary") => (200, self.summary().to_json()),
            (Method::Get, "/api/capabilities") => {
                let caps: Vec<Capability> = api::STAGED
                    .iter()
                    .map(|c| {
                        let on = api::available(c);
                        Capability { name: s(c), available: on, reason: s(if on { "" } else { "Für eine spätere Version geplant" }) }
                    })
                    .collect();
                (200, list(&caps))
            }
            (Method::Get, "/api/jobs") => (200, list(&self.jobs)),
            (Method::Get, "/api/jobs/{id}") => match self.jobs.iter().find(|j| j.id == id) {
                Some(j) => (200, j.to_json()),
                None => err(404, code::NOT_FOUND, "Job nicht gefunden"),
            },
            (Method::Post, "/api/jobs") | (Method::Put, "/api/jobs/{id}") => {
                let Some(req) = api::JobRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if req.name.trim().is_empty() {
                    return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Der Name fehlt"), field: s("name") }.to_json());
                }
                if req.schedule.enabled && !req.schedule.cron.is_empty() && Cron::parse(&req.schedule.cron).is_err() {
                    return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Ungültiger Zeitplan"), field: s("schedule") }.to_json());
                }
                let next = self.next_run(&req.schedule.cron, req.enabled && req.schedule.enabled);
                let jid = if method == Method::Post { self.id() } else { id.clone() };
                let base = self.jobs.iter().find(|j| j.id == jid).cloned().unwrap_or(Job { created: now, last_status: SessionStatus::Never, ..Job::default() });
                let job = Job {
                    id: jid.clone(),
                    name: req.name.clone(),
                    description: req.description,
                    kind: req.kind,
                    enabled: req.enabled,
                    host_id: req.host_id,
                    sources: req.sources,
                    excludes: req.excludes,
                    repository_id: req.repository_id,
                    schedule: req.schedule,
                    retention: req.retention,
                    encrypted: req.encrypted,
                    key_hint: req.key_hint,
                    compression: req.compression,
                    active_full_days: req.active_full_days,
                    pre_script: req.pre_script,
                    post_script: req.post_script,
                    source_job: req.source_job,
                    pool_id: req.pool_id,
                    next_run: next,
                    updated: now,
                    ..base
                };
                let created = method == Method::Post;
                match self.jobs.iter_mut().find(|j| j.id == jid) {
                    Some(j) => *j = job.clone(),
                    None => self.jobs.push(job.clone()),
                }
                self.log_audit(&actor, if created { "job.create" } else { "job.update" }, &job.name, "");
                (if created { 201 } else { 200 }, job.to_json())
            }
            (Method::Delete, "/api/jobs/{id}") => {
                let Some(p) = self.jobs.iter().position(|j| j.id == id) else { return err(404, code::NOT_FOUND, "Job nicht gefunden") };
                let j = self.jobs.remove(p);
                self.log_audit(&actor, "job.delete", &j.name, "Restore-Points bleiben erhalten");
                (204, Value::Null)
            }
            (Method::Post, "/api/jobs/{id}/enable") | (Method::Post, "/api/jobs/{id}/disable") => {
                let on = route.path.ends_with("enable");
                let Some(p) = self.jobs.iter().position(|j| j.id == id) else { return err(404, code::NOT_FOUND, "Job nicht gefunden") };
                self.jobs[p].enabled = on;
                let next = self.next_run(&self.jobs[p].schedule.cron.clone(), on);
                self.jobs[p].next_run = next;
                let name = self.jobs[p].name.clone();
                self.log_audit(&actor, if on { "job.enable" } else { "job.disable" }, &name, "");
                (200, self.jobs[p].to_json())
            }
            (Method::Post, "/api/jobs/{id}/run") => {
                let Some(p) = self.jobs.iter().position(|j| j.id == id) else { return err(404, code::NOT_FOUND, "Job nicht gefunden") };
                if self.runs.iter().any(|r| r.job == id) {
                    return err(409, code::BUSY, "Der Job läuft bereits");
                }
                let job = self.jobs[p].clone();
                let full = j::Field::take(body.get("full"));
                let op = self.start(&actor, SessionKind::Backup, &job, if full { "Aktive Vollsicherung" } else { "Sicherung" }, String::new());
                self.jobs[p].last_status = SessionStatus::Running;
                self.log_audit(&actor, "job.run", &job.name, if full { "Aktive Vollsicherung" } else { "Manuell gestartet" });
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/jobs/{id}/points") => {
                let mut v: Vec<RestorePoint> = self.points.iter().filter(|p| p.job_id == id).cloned().collect();
                v.sort_by(|a, b| b.created.cmp(&a.created));
                (200, list(&v))
            }
            (Method::Get, "/api/points/{id}") => match self.points.iter().find(|p| p.id == id) {
                Some(p) => (200, p.to_json()),
                None => err(404, code::NOT_FOUND, "Restore-Point nicht gefunden"),
            },
            (Method::Get, "/api/points/{id}/browse") => {
                let Some(pt) = self.points.iter().find(|p| p.id == id) else { return err(404, code::NOT_FOUND, "Restore-Point nicht gefunden") };
                let job = self.jobs.iter().find(|j| j.id == pt.job_id).cloned().unwrap_or_default();
                let path_q = api::query(path, "path").unwrap_or_default();
                if job.kind == JobKind::Qemu {
                    // a VM point: a folder per VM with vm.json and its disks
                    let vms: Vec<Vm> = job.sources.iter().filter_map(|id| self.vms.iter().find(|v| v.id == *id).cloned()).collect();
                    let entries: Vec<Entry> = if path_q.is_empty() {
                        vms.iter().map(|v| Entry { name: v.name.clone(), kind: EntryKind::Dir, size: 0, mtime: pt.created }).collect()
                    } else {
                        let v = vms.iter().find(|v| v.name == path_q);
                        let mut e = alloc::vec![Entry { name: s("vm.json"), kind: EntryKind::File, size: 412, mtime: pt.created }];
                        for (k, d) in v.map(|v| v.disks.clone()).unwrap_or_default().iter().enumerate() {
                            e.push(Entry { name: format!("disk{}.raw", k), kind: EntryKind::File, size: d.size, mtime: pt.created });
                        }
                        e
                    };
                    return (200, Listing { path: path_q, entries, more: false, offset: 0 }.to_json());
                }
                let roots = job.sources.clone();
                (200, browse(&roots, &path_q, pt.created).to_json())
            }
            (Method::Post, "/api/points/{id}/restore") | (Method::Post, "/api/points/{id}/verify") => {
                let Some(pt) = self.points.iter().find(|p| p.id == id).cloned() else { return err(404, code::NOT_FOUND, "Restore-Point nicht gefunden") };
                let Some(job) = self.jobs.iter().find(|j| j.id == pt.job_id).cloned() else { return err(404, code::NOT_FOUND, "Job nicht gefunden") };
                let restore = route.path.ends_with("restore");
                let op = self.start(&actor, if restore { SessionKind::Restore } else { SessionKind::Verify }, &job,
                                    if restore { "Wiederherstellung" } else { "Prüfung" }, pt.id.clone());
                let items = j::Field::take(body.get("items"));
                let items: Vec<String> = items;
                self.log_audit(&actor, if restore { "restore" } else { "point.verify" }, &job.name,
                               &if restore { format!("{} Einträge", items.len().max(1)) } else { String::new() });
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Delete, "/api/points/{id}") => {
                let Some(p) = self.points.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Restore-Point nicht gefunden") };
                if self.points[p].immutable_until > now {
                    return err(409, code::IMMUTABLE, "Der Restore-Point ist noch unveränderlich");
                }
                self.points.remove(p);
                (204, Value::Null)
            }
            (Method::Get, "/api/sessions") => {
                let limit = api::query(path, "limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(500);
                let job = api::query(path, "job").unwrap_or_default();
                let mut v: Vec<Session> = self.sessions.iter().filter(|x| job.is_empty() || x.job_id == job).cloned().collect();
                v.sort_by(|a, b| b.started.cmp(&a.started));
                v.truncate(limit);
                (200, list(&v))
            }
            (Method::Get, "/api/sessions/{id}") => match self.sessions.iter().find(|x| x.id == id) {
                Some(x) => (200, x.to_json()),
                None => err(404, code::NOT_FOUND, "Sitzung nicht gefunden"),
            },
            (Method::Get, "/api/sessions/{id}/log") => match self.sessions.iter().find(|x| x.id == id) {
                Some(x) => (200, list(&session_log(x, &self.jobs))),
                None => err(404, code::NOT_FOUND, "Sitzung nicht gefunden"),
            },
            (Method::Get, "/api/operations") => {
                let mut v = self.ops.clone();
                v.sort_by(|a, b| b.created.cmp(&a.created));
                (200, list(&v))
            }
            (Method::Get, "/api/operations/{id}") => match self.ops.iter().find(|o| o.id == id) {
                Some(o) => (200, o.to_json()),
                None => err(404, code::NOT_FOUND, "Vorgang nicht gefunden"),
            },
            (Method::Post, "/api/operations/{id}/cancel") => {
                let Some(o) = self.ops.iter_mut().find(|o| o.id == id) else { return err(404, code::NOT_FOUND, "Vorgang nicht gefunden") };
                if o.status.finished() {
                    return err(409, code::CONFLICT, "Der Vorgang ist bereits beendet");
                }
                o.status = OpStatus::Cancelling;
                (200, o.to_json())
            }
            (Method::Get, "/api/hosts") => (200, list(&self.hosts)),
            (Method::Get, "/api/hosts/{id}") => match self.hosts.iter().find(|h| h.id == id) {
                Some(h) => (200, h.to_json()),
                None => err(404, code::NOT_FOUND, "Host nicht gefunden"),
            },
            (Method::Put, "/api/hosts/{id}") => {
                let Some(req) = api::HostUpdate::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let Some(h) = self.hosts.iter_mut().find(|h| h.id == id) else { return err(404, code::NOT_FOUND, "Host nicht gefunden") };
                if !req.name.is_empty() {
                    h.name = req.name;
                }
                h.roots = req.roots;
                h.restore_roots = req.restore_roots;
                let (n, v) = (h.name.clone(), h.to_json());
                self.log_audit(&actor, "host.update", &n, "Freigegebene Ordner geändert");
                (200, v)
            }
            (Method::Delete, "/api/hosts/{id}") => {
                let Some(p) = self.hosts.iter().position(|h| h.id == id) else { return err(404, code::NOT_FOUND, "Host nicht gefunden") };
                if self.hosts[p].kind == HostKind::Server {
                    return err(409, code::CONFLICT, "Der Backup-Server selbst kann nicht entfernt werden");
                }
                let h = self.hosts.remove(p);
                self.log_audit(&actor, "host.remove", &h.name, "Zertifikat gesperrt");
                (204, Value::Null)
            }
            (Method::Get, "/api/hosts/{id}/browse") => {
                let Some(h) = self.hosts.iter().find(|h| h.id == id) else { return err(404, code::NOT_FOUND, "Host nicht gefunden") };
                if h.status == HostStatus::Offline {
                    return err(503, code::BUSY, "Der Agent ist nicht erreichbar");
                }
                let path_q = api::query(path, "path").unwrap_or_default();
                (200, browse(&h.roots, &path_q, now).to_json())
            }
            (Method::Post, "/api/hosts/{id}/check") => {
                let Some(h) = self.hosts.iter().find(|h| h.id == id).cloned() else { return err(404, code::NOT_FOUND, "Host nicht gefunden") };
                let op = self.quick_op(&actor, "host.check", &format!("Verbindung zu {} prüfen", h.name),
                                       if h.status == HostStatus::Offline { Err("Keine Antwort innerhalb von 10 s") } else { Ok("Erreichbar, Zertifikat gültig") });
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/hosts/pairing") => {
                let r = self.rand();
                let alpha = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
                let mut c = String::new();
                for i in 0..8 {
                    if i == 4 {
                        c.push('-');
                    }
                    c.push(alpha[((r >> (i * 5)) & 31) as usize] as char);
                }
                let pc = PairingCode { code: c, expires: now + 15 * 60, server_url: s("https://backup-srv01.corp.local:7642"), ca_fingerprint: s("9C:4E:21:07:B3:5A:E0:4D:91:2F:66:C8:0A:1B:7E:D3") };
                self.pairing.push(pc.clone());
                self.log_audit(&actor, "host.pairing", "", "Pairing-Code erzeugt (15 min gültig)");
                (201, pc.to_json())
            }
            (Method::Post, "/api/hosts/ssh-fingerprint") => {
                let addr = String::from(j::str_of(&body, "address"));
                if addr.is_empty() {
                    return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Die Adresse fehlt"), field: s("address") }.to_json());
                }
                (200, api::SshFingerprint { address: addr, port: 22, fingerprint: s("SHA256:q8YlCq0k3w7mF8e3Wb2pVNh1sR8xZJcE6Q0aKkzP5wE"), key_type: s("ssh-ed25519") }.to_json())
            }
            (Method::Post, "/api/hosts/ssh-push") => {
                let Some(req) = api::SshPushRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if req.host_fingerprint.is_empty() {
                    return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Bitte den Host-Schlüssel bestätigen"), field: s("host_fingerprint") }.to_json());
                }
                let hid = self.id();
                let name = if req.name.is_empty() { req.address.clone() } else { req.name.clone() };
                self.hosts.push(Host {
                    id: hid,
                    name: name.clone(),
                    address: req.address.clone(),
                    os: Os::Linux,
                    platform: s("Linux"),
                    kind: HostKind::Agent,
                    status: HostStatus::Pairing,
                    version: s(bc_proto::VERSION),
                    last_seen: 0,
                    fingerprint: String::new(),
                    roots: req.roots,
                    restore_roots: Vec::new(),
                    message: s("Agent wird installiert …"),
                    created: now,
                });
                let op = self.quick_op(&actor, "host.ssh_push", &format!("Agent auf {} installieren", name), Ok("Agent installiert und gekoppelt"));
                self.log_audit(&actor, "host.ssh_push", &name, &req.host_fingerprint);
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/repositories") => {
                let mut v = self.repos.clone();
                for r in v.iter_mut() {
                    r.points = self.points.iter().filter(|p| self.jobs.iter().any(|j| j.id == p.job_id && j.repository_id == r.id)).count() as u32;
                }
                (200, list(&v))
            }
            (Method::Get, "/api/repositories/{id}") => match self.repos.iter().find(|r| r.id == id) {
                Some(r) => (200, r.to_json()),
                None => err(404, code::NOT_FOUND, "Repository nicht gefunden"),
            },
            (Method::Post, "/api/repositories") => {
                let Some(req) = api::RepositoryRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if req.name.trim().is_empty() || req.path.trim().is_empty() {
                    return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Name und Pfad sind nötig"), field: s(if req.name.trim().is_empty() { "name" } else { "path" }) }.to_json());
                }
                let rid = self.id();
                let r = Repository {
                    id: rid,
                    name: req.name.clone(),
                    path: req.path,
                    status: RepoStatus::Ok,
                    capacity: 4 * TB,
                    used: 0,
                    free: 4 * TB,
                    logical: 0,
                    points: 0,
                    dedup_x100: 0,
                    encrypted: true,
                    immutable_days: req.immutable_days,
                    message: String::new(),
                    created: now,
                };
                self.repos.push(r.clone());
                self.log_audit(&actor, "repository.create", &req.name, "");
                (201, r.to_json())
            }
            (Method::Put, "/api/repositories/{id}") => {
                let Some(r) = self.repos.iter_mut().find(|r| r.id == id) else { return err(404, code::NOT_FOUND, "Repository nicht gefunden") };
                let name = j::str_of(&body, "name");
                if !name.is_empty() {
                    r.name = s(name);
                }
                let days: u32 = j::Field::take(body.get("immutable_days"));
                if days < r.immutable_days {
                    return err(409, code::IMMUTABLE, "Die Unveränderlichkeit kann nur verlängert werden");
                }
                r.immutable_days = days;
                (200, r.to_json())
            }
            (Method::Delete, "/api/repositories/{id}") => {
                if self.jobs.iter().any(|j| j.repository_id == id) {
                    return err(409, code::CONFLICT, "Jobs verwenden dieses Repository noch");
                }
                self.repos.retain(|r| r.id != id);
                (204, Value::Null)
            }
            (Method::Post, "/api/repositories/{id}/check") | (Method::Post, "/api/repositories/{id}/gc") => {
                let Some(r) = self.repos.iter().find(|r| r.id == id).cloned() else { return err(404, code::NOT_FOUND, "Repository nicht gefunden") };
                let gc = route.path.ends_with("gc");
                let op = self.quick_op(&actor, if gc { "repository.gc" } else { "repository.check" },
                                       &format!("Repository „{}“ {}", r.name, if gc { "aufräumen" } else { "prüfen" }),
                                       Ok(if gc { "312 GB freigegeben" } else { "Keine Fehler gefunden" }));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/credentials") => (200, list(&self.creds)),
            (Method::Post, "/api/credentials") => {
                let Some(req) = api::CredentialRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let cid = self.id();
                let c = Credential { id: cid, label: req.label.clone(), username: req.username, kind: req.kind, created: now };
                self.creds.push(c.clone());
                self.log_audit(&actor, "credential.create", &req.label, "");
                (201, c.to_json())
            }
            (Method::Delete, "/api/credentials/{id}") => {
                self.creds.retain(|c| c.id != id);
                (204, Value::Null)
            }
            (Method::Get, "/api/users") => (200, list(&self.users.iter().map(|(u, _)| u.clone()).collect::<Vec<_>>())),
            (Method::Post, "/api/users") | (Method::Put, "/api/users/{id}") => {
                let Some(req) = api::UserRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if method == Method::Post {
                    if req.username.trim().is_empty() {
                        return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Der Benutzername fehlt"), field: s("username") }.to_json());
                    }
                    if req.password.chars().count() < 12 {
                        return (400, api::ApiError { code: s(code::BAD_REQUEST), message: s("Das Passwort braucht mindestens 12 Zeichen"), field: s("password") }.to_json());
                    }
                    if self.users.iter().any(|(u, _)| u.username == req.username) {
                        return (409, api::ApiError { code: s(code::CONFLICT), message: s("Den Benutzer gibt es schon"), field: s("username") }.to_json());
                    }
                    let uid = self.id();
                    let u = User { id: uid, username: req.username.clone(), role: req.role, created: now, last_login: 0, disabled: false };
                    self.users.push((u.clone(), req.password));
                    self.log_audit(&actor, "user.create", &req.username, req.role.as_str());
                    return (201, u.to_json());
                }
                let Some((u, pw)) = self.users.iter_mut().find(|(u, _)| u.id == id) else { return err(404, code::NOT_FOUND, "Benutzer nicht gefunden") };
                u.role = req.role;
                u.disabled = req.disabled;
                if !req.password.is_empty() {
                    *pw = req.password;
                }
                let v = u.to_json();
                (200, v)
            }
            (Method::Delete, "/api/users/{id}") => {
                if self.users.iter().filter(|(u, _)| u.role == Role::Admin).count() <= 1 && self.users.iter().any(|(u, _)| u.id == id && u.role == Role::Admin) {
                    return err(409, code::CONFLICT, "Der letzte Administrator kann nicht gelöscht werden");
                }
                self.users.retain(|(u, _)| u.id != id);
                (204, Value::Null)
            }
            (Method::Post, "/api/users/me/password") => (204, Value::Null),
            (Method::Get, "/api/audit") => (200, list(&self.audit)),
            (Method::Get, "/api/settings") => (200, list(&self.settings)),
            (Method::Put, "/api/settings") => {
                let items: Vec<Setting> = j::from_list(&body);
                for it in items {
                    match self.settings.iter_mut().find(|x| x.key == it.key) {
                        Some(x) => x.value = it.value,
                        None => self.settings.push(it),
                    }
                }
                self.log_audit(&actor, "settings.update", "", "");
                (200, list(&self.settings))
            }
            (Method::Get, "/api/recovery") => (200, self.recovery.to_json()),
            (Method::Post, "/api/settings/test-email") | (Method::Post, "/api/notify/summary") => {
                self.log_audit(&actor, "notify.test", "", "Demo");
                (200, obj([("message", Value::string("Demo-Server: nichts versendet"))]))
            }
            (Method::Get, "/api/update") | (Method::Post, "/api/update/check") => (200, UpdateStatus {
                current: s(bc_proto::VERSION),
                channel: s("https://updates.example.org/backupcraft/manifest.json"),
                platform: s("windows-x86_64"),
                latest: s("0.2.0"),
                available: true,
                notes: s("Kopierjobs, Bandbibliothek und zstd-Kompression."),
                date: now - 2 * DAY,
                checked: now - 3600,
                file: s("BackupCraft_Setup.exe"),
                size: 5_412_864,
                ..UpdateStatus::default()
            }.to_json()),
            (Method::Get, "/api/notify") => (200, NotifyStatus {
                last_at: now - 5 * 3600,
                last_ok: true,
                last_message: s("E-Mail an it@example.org gesendet"),
                last_what: s("Zusammenfassung"),
                daily_sent: 0,
            }.to_json()),
            (Method::Post, "/api/recovery/kit") => {
                let mut k = [0u8; 32];
                for b in k.iter_mut() {
                    *b = self.rand() as u8;
                }
                self.recovery = RecoveryStatus { created: now, fingerprint: s("b71e 0c93 5aa2 e410") };
                self.log_audit(&actor, "recovery.kit", "", "Neues Recovery-Kit erzeugt");
                (201, api::RecoveryKit { private_key: format!("BCRK-1-{}", ids::hex(&k)), fingerprint: self.recovery.fingerprint.clone(), created: now }.to_json())
            }
            (Method::Get, "/api/config/backups") => (200, list(&self.backups)),
            (Method::Post, "/api/config/backups") => {
                self.backups.insert(0, ConfigBackup { name: format!("manual-{}.bcconfig", now), created: now, size: 183_400, auto: false });
                let op = self.quick_op(&actor, "config.backup", "Konfiguration sichern", Ok("Gesichert und verschlüsselt"));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/config/restore") => (200, obj([("message", Value::string("Wird beim nächsten Start angewendet"))])),
            (Method::Get, "/api/cdp") => (200, list(&self.cdp_status())),
            (Method::Get, "/api/replicas") => (200, list(&self.replicas)),
            (Method::Post, "/api/replicas") | (Method::Put, "/api/replicas/{id}") => {
                let Some(r) = api::ReplicaRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if r.name.trim().is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte einen Namen angeben");
                }
                let Some(job) = self.jobs.iter().find(|j| j.id == r.job_id).cloned() else { return err(400, code::BAD_REQUEST, "Bitte einen Job wählen") };
                let Some(host) = self.hosts.iter().find(|h| h.id == r.host_id).cloned() else { return err(400, code::BAD_REQUEST, "Bitte einen Zielhost wählen") };
                if !host.restore_roots.iter().any(|x| r.target.starts_with(x.as_str())) {
                    return err(403, code::FORBIDDEN, &format!("„{}“ liegt nicht in einem Wiederherstellungsordner von {}", r.target, host.name));
                }
                let mut rep = match self.replica_at(&id) {
                    Some(p) => self.replicas[p].clone(),
                    None if id.is_empty() => Replica { id: self.id(), state: ReplicaState::New, created: now, ..Replica::default() },
                    None => return err(404, code::NOT_FOUND, "Replikat nicht gefunden"),
                };
                if rep.host_id != host.id || rep.target != r.target || rep.job_id != job.id {
                    rep.state = ReplicaState::New;
                    rep.point_id.clear();
                    rep.point_time = 0;
                }
                rep.name = s(r.name.trim());
                rep.job_id = job.id.clone();
                rep.job_name = job.name.clone();
                rep.host_id = host.id.clone();
                rep.host_name = host.name.clone();
                rep.target = r.target.clone();
                rep.auto = r.auto;
                self.log_audit(&actor, if id.is_empty() { "replica.create" } else { "replica.update" }, &rep.name, &format!("{} → {}:{}", job.name, host.name, rep.target));
                match self.replica_at(&rep.id) {
                    Some(p) => self.replicas[p] = rep.clone(),
                    None => self.replicas.push(rep.clone()),
                }
                (if id.is_empty() { 201 } else { 200 }, rep.to_json())
            }
            (Method::Delete, "/api/replicas/{id}") => {
                let Some(p) = self.replica_at(&id) else { return err(404, code::NOT_FOUND, "Replikat nicht gefunden") };
                if self.plans.iter().any(|x| x.replicas.contains(&id)) {
                    return err(409, code::CONFLICT, "Ein Failover-Plan verwendet dieses Replikat");
                }
                let r = self.replicas.remove(p);
                self.log_audit(&actor, "replica.delete", &r.name, "Die Dateien im Zielordner bleiben");
                (204, Value::Null)
            }
            (Method::Post, "/api/replicas/{id}/sync") | (Method::Post, "/api/replicas/{id}/undo") | (Method::Post, "/api/replicas/{id}/failback") => {
                let Some(p) = self.replica_at(&id) else { return err(404, code::NOT_FOUND, "Replikat nicht gefunden") };
                let what = route.path.rsplit('/').next().unwrap_or("");
                let st = self.replicas[p].state;
                if (what == "sync" && st == ReplicaState::Failover) || (what != "sync" && st != ReplicaState::Failover) {
                    return err(409, code::CONFLICT, if what == "sync" { "Das Replikat ist im Failover" } else { "Nur aus dem Failover" });
                }
                let newest = self.points.iter().filter(|x| x.job_id == self.replicas[p].job_id).max_by_key(|x| x.created).cloned();
                let r = &mut self.replicas[p];
                r.state = ReplicaState::Ready;
                r.synced = now;
                r.failover_at = 0;
                r.point_id = newest.as_ref().map_or(String::new(), |x| x.id.clone());
                r.point_time = newest.as_ref().map_or(0, |x| x.created);
                r.message.clear();
                let name = r.name.clone();
                let (title, res) = match what {
                    "sync" => (format!("Replikat {}: Abgleich", name), "Abgeglichen: 312 geändert, 48.211 unverändert"),
                    "undo" => (format!("Replikat {}: Failover verwerfen", name), "Zurück auf den Restore-Point"),
                    _ => (format!("Replikat {}: Failback", name), "Zurückgeschrieben: 87 Dateien"),
                };
                self.log_audit(&actor, &format!("replica.{}", what), &name, "");
                let op = self.quick_op(&actor, &format!("replica.{}", what), &title, Ok(res));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/replicas/{id}/failover") => {
                let Some(p) = self.replica_at(&id) else { return err(404, code::NOT_FOUND, "Replikat nicht gefunden") };
                if self.replicas[p].state != ReplicaState::Ready {
                    return err(409, code::CONFLICT, "Das Replikat ist nicht bereit");
                }
                let r = &mut self.replicas[p];
                r.state = ReplicaState::Failover;
                r.failover_at = now;
                r.message = format!("Failover seit {}: {} auf {} ist in Gebrauch", fmt_hm(now, &self.zone), r.target, r.host_name);
                let (name, v) = (r.name.clone(), r.to_json());
                self.log_audit(&actor, "replica.failover", &name, "");
                (200, v)
            }
            (Method::Get, "/api/failover-plans") => (200, list(&self.plans)),
            (Method::Post, "/api/failover-plans") | (Method::Put, "/api/failover-plans/{id}") => {
                let Some(r) = api::PlanRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if r.name.trim().is_empty() || r.replicas.is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte einen Namen und mindestens ein Replikat angeben");
                }
                let mut plan = match self.plans.iter().find(|x| x.id == id) {
                    Some(x) => x.clone(),
                    None if id.is_empty() => FailoverPlan { id: self.id(), created: now, ..FailoverPlan::default() },
                    None => return err(404, code::NOT_FOUND, "Failover-Plan nicht gefunden"),
                };
                plan.name = s(r.name.trim());
                plan.replicas = r.replicas;
                plan.delay_s = r.delay_s;
                self.log_audit(&actor, if id.is_empty() { "plan.create" } else { "plan.update" }, &plan.name, "");
                match self.plans.iter().position(|x| x.id == plan.id) {
                    Some(p) => self.plans[p] = plan.clone(),
                    None => self.plans.push(plan.clone()),
                }
                (if id.is_empty() { 201 } else { 200 }, plan.to_json())
            }
            (Method::Delete, "/api/failover-plans/{id}") => {
                let Some(p) = self.plans.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Failover-Plan nicht gefunden") };
                let x = self.plans.remove(p);
                self.log_audit(&actor, "plan.delete", &x.name, "");
                (204, Value::Null)
            }
            (Method::Post, "/api/failover-plans/{id}/run") => {
                let Some(p) = self.plans.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Failover-Plan nicht gefunden") };
                let plan = self.plans[p].clone();
                let mut done = 0;
                let mut failed = Vec::new();
                for rid in &plan.replicas {
                    let Some(k) = self.replica_at(rid) else { continue };
                    let r = &mut self.replicas[k];
                    if r.state == ReplicaState::Ready {
                        r.state = ReplicaState::Failover;
                        r.failover_at = now;
                        r.message = format!("Failover durch Plan „{}“", plan.name);
                        done += 1;
                    } else {
                        failed.push(r.name.clone());
                    }
                }
                let msg = if failed.is_empty() { format!("{} Replikate umgeschaltet", done) } else { format!("{} umgeschaltet, nicht bereit: {}", done, failed.join(", ")) };
                let x = &mut self.plans[p];
                x.last_run = now;
                x.last_status = s(if failed.is_empty() { "success" } else { "failed" });
                x.message = msg.clone();
                self.log_audit(&actor, "plan.run", &plan.name, &msg);
                let op = if failed.is_empty() {
                    self.quick_op(&actor, "plan.run", &format!("Failover-Plan {}", plan.name), Ok(msg.as_str()))
                } else {
                    self.quick_op(&actor, "plan.run", &format!("Failover-Plan {}", plan.name), Err(msg.as_str()))
                };
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/tape") => (200, self.tape_status().to_json()),
            (Method::Get, "/api/tape/tapes") => (200, list(&self.tapes)),
            (Method::Post, "/api/tape/tapes") => {
                let Some(r) = api::CartridgeRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let prefix = r.prefix.trim().to_ascii_uppercase();
                if !(2..=4).contains(&prefix.len()) || !prefix.chars().all(|c| c.is_ascii_uppercase()) {
                    return err(400, code::BAD_REQUEST, "Das Barcode-Präfix hat 2 bis 4 Buchstaben");
                }
                let pool = self.pools.iter().find(|p| p.id == r.pool_id).cloned();
                let cap = if r.capacity_mb > 0 { r.capacity_mb as u64 * 1_000_000 } else { r.capacity_gb.max(1) as u64 * GB };
                let mut made = Vec::new();
                let mut n = 1u32;
                while made.len() < r.count.clamp(1, 100) as usize && n < 10_000 {
                    let b = format!("{}{:04}V1", prefix, n);
                    n += 1;
                    if self.tapes.iter().any(|c| c.barcode == b) {
                        continue;
                    }
                    let c = Cartridge { id: b.clone(), barcode: b, pool_id: r.pool_id.clone(), pool_name: pool.as_ref().map_or(String::new(), |p| p.name.clone()),
                                        capacity: cap, used: 4096, state: s("blank"), created: now, ..Cartridge::default() };
                    self.tapes.push(c.clone());
                    made.push(c);
                }
                self.log_audit(&actor, "tape.add", &format!("{} Bänder", made.len()), "");
                (200, list(&made))
            }
            (Method::Put, "/api/tape/tapes/{id}") => {
                let Some(r) = api::CartridgeUpdate::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let pool = self.pools.iter().find(|p| p.id == r.pool_id).cloned();
                let Some(c) = self.tapes.iter_mut().find(|c| c.id == id) else { return err(404, code::NOT_FOUND, "Band nicht gefunden") };
                if c.state != "blank" {
                    return err(409, code::CONFLICT, "Nur leere Bänder wechseln den Pool");
                }
                c.pool_id = r.pool_id;
                c.pool_name = pool.map_or(String::new(), |p| p.name);
                (200, c.to_json())
            }
            (Method::Delete, "/api/tape/tapes/{id}") | (Method::Post, "/api/tape/tapes/{id}/erase") => {
                let Some(p) = self.tapes.iter().position(|c| c.id == id) else { return err(404, code::NOT_FOUND, "Band nicht gefunden") };
                if self.tapes[p].protected_until > now {
                    return err(409, code::IMMUTABLE, "Das Band ist noch schreibgeschützt");
                }
                let b = self.tapes[p].barcode.clone();
                self.tape_sets.retain(|x| !x.tapes.contains(&b));
                if method == Method::Delete {
                    self.tapes.remove(p);
                    self.log_audit(&actor, "tape.remove", &b, "");
                    return (204, Value::Null);
                }
                let c = &mut self.tapes[p];
                c.state = s("blank");
                c.used = 4096;
                c.sets = 0;
                c.written = 0;
                let op = self.quick_op(&actor, "tape.erase", &format!("Band {} löschen", b), Ok("Das Band ist leer"));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/tape/rescan") => {
                let msg = format!("{} Bänder gelesen (0 neu), 0 Sätze katalogisiert", self.tapes.iter().filter(|c| c.state != "missing").count());
                let op = self.quick_op(&actor, "tape.rescan", "Bandbibliothek einlesen", Ok(&msg));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/tape/pools") => {
                let mut v = self.pools.clone();
                for p in &mut v {
                    let mine: Vec<&Cartridge> = self.tapes.iter().filter(|c| c.pool_id == p.id).collect();
                    p.tapes = mine.len() as u32;
                    p.used = mine.iter().map(|c| c.used).sum();
                    p.capacity = mine.iter().map(|c| c.capacity).sum();
                }
                (200, list(&v))
            }
            (Method::Post, "/api/tape/pools") | (Method::Put, "/api/tape/pools/{id}") => {
                let Some(r) = api::PoolRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                if r.name.trim().is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte einen Namen angeben");
                }
                let mut p = match self.pools.iter().find(|x| x.id == id) {
                    Some(x) => x.clone(),
                    None if id.is_empty() => MediaPool { id: self.id(), created: now, ..MediaPool::default() },
                    None => return err(404, code::NOT_FOUND, "Medienpool nicht gefunden"),
                };
                p.name = s(r.name.trim());
                p.description = r.description;
                p.protect_days = r.protect_days;
                p.encrypted = r.encrypted;
                p.append = r.append;
                match self.pools.iter().position(|x| x.id == p.id) {
                    Some(k) => self.pools[k] = p.clone(),
                    None => self.pools.push(p.clone()),
                }
                self.log_audit(&actor, if id.is_empty() { "pool.create" } else { "pool.update" }, &p.name, "");
                (if id.is_empty() { 201 } else { 200 }, p.to_json())
            }
            (Method::Delete, "/api/tape/pools/{id}") => {
                if self.tapes.iter().any(|c| c.pool_id == id && c.state != "blank") {
                    return err(409, code::CONFLICT, "Der Pool hat beschriebene Bänder");
                }
                let Some(k) = self.pools.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Medienpool nicht gefunden") };
                let p = self.pools.remove(k);
                for c in self.tapes.iter_mut().filter(|c| c.pool_id == id) {
                    c.pool_id.clear();
                    c.pool_name.clear();
                }
                self.log_audit(&actor, "pool.delete", &p.name, "");
                (204, Value::Null)
            }
            (Method::Get, "/api/tape/sets") => (200, list(&self.tape_sets)),
            (Method::Get, "/api/tape/sets/{id}/browse") => {
                let Some(x) = self.tape_sets.iter().find(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Bandsatz nicht gefunden") };
                let path_q = api::query(path, "path").unwrap_or_default();
                (200, browse(&x.roots, &path_q, x.point_time).to_json())
            }
            (Method::Post, "/api/tape/sets/{id}/restore") | (Method::Post, "/api/tape/sets/{id}/verify") => {
                let Some(x) = self.tape_sets.iter().find(|x| x.id == id).cloned() else { return err(404, code::NOT_FOUND, "Bandsatz nicht gefunden") };
                let restore = route.path.ends_with("restore");
                let msg = if restore { format!("{} Dateien vom Band wiederhergestellt", fmt_count(x.files)) } else { format!("{} Dateien von {} gelesen und geprüft", fmt_count(x.files), x.tapes.join(", ")) };
                let title = format!("{}: {}", x.job_name, if restore { "vom Band wiederherstellen" } else { "Bandsatz prüfen" });
                let op = self.quick_op(&actor, if restore { "tape.restore" } else { "tape.verify" }, &title, Ok(&msg));
                if !restore {
                    if let Some(y) = self.tape_sets.iter_mut().find(|y| y.id == id) {
                        y.verified = now;
                    }
                }
                self.log_audit(&actor, if restore { "tape.restore" } else { "tape.verify" }, &x.job_name, "");
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/points/{id}/tape") => {
                let Some(pt) = self.points.iter().find(|p| p.id == id).cloned() else { return err(404, code::NOT_FOUND, "Restore-Point nicht gefunden") };
                let Some(r) = api::TapeWriteRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let Some(pool) = self.pools.iter().find(|p| p.id == r.pool_id).cloned() else { return err(400, code::BAD_REQUEST, "Bitte einen Medienpool wählen") };
                let Some(k) = self.tapes.iter().position(|c| c.pool_id == pool.id && c.state == "active").or_else(|| self.tapes.iter().position(|c| c.state == "blank" && (c.pool_id == pool.id || c.pool_id.is_empty()))) else {
                    return err(409, code::CONFLICT, "Kein Band mit Platz im Pool");
                };
                let c = &mut self.tapes[k];
                c.pool_id = pool.id.clone();
                c.pool_name = pool.name.clone();
                c.state = s("active");
                c.used += pt.size;
                c.sets += 1;
                c.written = now;
                c.protected_until = now + pool.protect_days as i64 * DAY;
                let barcode = c.barcode.clone();
                let roots = self.jobs.iter().find(|j| j.id == pt.job_id).map(|j| j.sources.clone()).unwrap_or_default();
                let sid = self.id();
                self.tape_sets.insert(0, TapeSet {
                    id: sid, pool_id: pool.id.clone(), pool_name: pool.name.clone(), job_id: pt.job_id.clone(), job_name: pt.job_name.clone(), point_id: pt.id.clone(),
                    point_time: pt.created, host_name: pt.host.clone(), windows: true, roots, tapes: alloc::vec![barcode.clone()], positions: alloc::vec![4096],
                    files: pt.files, entries: pt.files + pt.files / 10, bytes: pt.size, encrypted: pool.encrypted, written: now, verified: 0, status: s("ok"),
                });
                let msg = format!("{} Dateien auf {} geschrieben", fmt_count(pt.files), barcode);
                let op = self.quick_op(&actor, "tape.write", &format!("{}: auf Band ({})", pt.job_name, pool.name), Ok(&msg));
                self.log_audit(&actor, "tape.write", &pt.job_name, &pool.name);
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/hypervisors") => {
                let server = &self.hosts[0];
                (200, list(&[Hypervisor {
                    host_id: server.id.clone(),
                    host_name: server.name.clone(),
                    kind: s("qemu"),
                    version: s("8.2.2"),
                    kvm: true,
                    folders: alloc::vec![s("/run/vms")],
                    vms: self.vms.len() as u32,
                    running: self.vms.iter().filter(|v| v.state == "running").count() as u32,
                    ok: true,
                    message: String::new(),
                }]))
            }
            (Method::Get, "/api/vms") => (200, list(&self.vms)),
            (Method::Get, "/api/hosts/{id}/volumes") => {
                if self.hosts.first().is_none_or(|h| h.id != id) {
                    return err(501, code::CAPABILITY, "Volumes von Agenten: in Vorbereitung (bisher nur die des Backup-Servers)");
                }
                let v = [("/dev/sda2", "", "ext4", 256u64, 61u64, "part", "/"), ("/dev/vg0/srv", "daten", "xfs", 2000, 820, "lvm", "/srv"), ("/dev/vg0/home", "", "ext4", 500, 310, "lvm", "/home")];
                (200, list(&v.iter().map(|(id, label, fs, size, free, kind, mount)| Volume { id: s(id), label: s(label), fs: s(fs), size: size * GB, free: free * GB, kind: s(kind), mount: s(mount) }).collect::<Vec<_>>()))
            }
            (Method::Get, "/api/instant-start") => (200, list(&self.instant)),
            (Method::Get, "/api/boot-checks") => (200, list(&self.boot_checks)),
            (Method::Get, "/api/instant-start/{id}/console") => {
                let Some(x) = self.instant.iter().find(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Sitzung nicht gefunden") };
                (200, obj([("text", Value::string(format!("SeaBIOS (version 1.16.3)\nBooting from Hard Disk...\n\nUbuntu 24.04 LTS {} ttyS0\n\n{} login: ", x.vm, x.vm)))]))
            }
            (Method::Post, "/api/instant-start/{id}/stop") => {
                let Some(x) = self.instant.iter_mut().find(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Sitzung nicht gefunden") };
                let migrated = x.state == "migrated";
                x.state = s("stopped");
                x.stopped = now;
                x.message = s(if migrated { "Beendet; die zusammengeführten Platten bleiben" } else { "Beendet; die Änderungen wurden verworfen" });
                let v = x.to_json();
                self.log_audit(&actor, "instant.stop", "", "");
                (200, v)
            }
            (Method::Post, "/api/instant-start/{id}/migrate") => {
                let Some(r) = api::MigrateRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let Some(x) = self.instant.iter_mut().find(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Sitzung nicht gefunden") };
                if x.state != "running" {
                    return err(409, code::CONFLICT, "Die VM läuft nicht aus einem Restore-Point");
                }
                x.state = s("migrated");
                x.target = r.target.clone();
                x.message = format!("Läuft von {}", r.target);
                let vm = x.vm.clone();
                let op = self.quick_op(&actor, "instant.migrate", &format!("{}: zusammenführen", vm), Ok("Die Platten wurden übertragen"));
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Post, "/api/points/{id}/instant-start") | (Method::Post, "/api/points/{id}/boot-check") => {
                let Some(pt) = self.points.iter().find(|p| p.id == id).cloned() else { return err(404, code::NOT_FOUND, "Restore-Point nicht gefunden") };
                let vm = String::from(j::str_of(&body, "vm"));
                if vm.is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte eine VM wählen");
                }
                let mem = self.vms.iter().find(|v| v.name == vm).map_or(2048, |v| v.memory_mb);
                if route.path.ends_with("instant-start") {
                    let sid = self.id();
                    let m = j::u64_of(&body, "memory_mb");
                    self.instant.insert(0, InstantSession {
                        id: sid, point_id: pt.id.clone(), point_time: pt.created, job_id: pt.job_id.clone(), job_name: pt.job_name.clone(), vm: vm.clone(),
                        purpose: s("instant"), state: s("running"), memory_mb: if m == 0 { mem } else { m }, cpus: 2,
                        network: s(if j::str_of(&body, "network") == "isolated" { "isolated" } else { "none" }), started: now, owner: actor.clone(), message: s("läuft (KVM)"),
                        ..InstantSession::default()
                    });
                    let op = self.quick_op(&actor, "instant.start", &format!("{}: {} aus dem Restore-Point starten", pt.job_name, vm), Ok("Die VM läuft"));
                    self.log_audit(&actor, "instant.start", &vm, "");
                    return (202, obj([("operation_id", Value::string(op))]));
                }
                let text = String::from(j::str_of(&body, "console_text"));
                let mut checks = alloc::vec![CheckResult { name: s("Start"), ok: true, detail: format!("läuft seit {} s", j::u64_of(&body, "boot_seconds").max(1)) }];
                if !text.is_empty() {
                    checks.push(CheckResult { name: s("Konsole"), ok: true, detail: format!("„{}“ erschienen", text) });
                }
                let port = j::u64_of(&body, "tcp_port");
                if port > 0 {
                    checks.push(CheckResult { name: format!("Port {}", port), ok: true, detail: s("antwortet") });
                }
                let rid = self.id();
                let msg = format!("{}: {} von {} Prüfungen bestanden", vm, checks.len(), checks.len());
                self.boot_checks.insert(0, BootCheckResult {
                    id: rid, point_id: pt.id.clone(), point_time: pt.created, job_id: pt.job_id.clone(), job_name: pt.job_name.clone(), vm: vm.clone(),
                    started: now, finished: now + 40, ok: true, checks, console: format!("{} login:", vm), message: msg.clone(),
                });
                let op = self.quick_op(&actor, "bootcheck", &format!("{}: Startprüfung {}", pt.job_name, vm), Ok(&msg));
                self.log_audit(&actor, "bootcheck", &vm, "");
                (202, obj([("operation_id", Value::string(op))]))
            }
            (Method::Get, "/api/fleet") => (200, list(&self.fleet)),
            (Method::Get, "/api/fleet/report") => (200, self.summary().to_json()),
            (Method::Post, "/api/fleet/probe") => {
                let Some(r) = api::ProbeRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let Some(url) = bc_proto::server_address(&r.url) else { return err(400, code::BAD_REQUEST, "Bitte eine gültige Adresse angeben") };
                let tls = url.starts_with("https");
                let fp = crypto::sha256(url.as_bytes());
                let host = url.split("://").nth(1).unwrap_or("").rsplit_once(':').map_or("", |(h, _)| h);
                (200, api::ProbeResult {
                    url: url.clone(),
                    tls,
                    fingerprint: if tls { ids::hex(&fp) } else { String::new() },
                    fingerprint_text: if tls { netio::trust::fingerprint_text(&fp) } else { String::new() },
                    ca_name: if tls { format!("BackupCraft CA {}", host) } else { String::new() },
                }.to_json())
            }
            (Method::Post, "/api/fleet/servers") | (Method::Put, "/api/fleet/servers/{id}") => {
                let Some(r) = api::FleetServerRequest::from_json(&body) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
                let Some(url) = bc_proto::server_address(&r.url) else { return err(400, code::BAD_REQUEST, "Bitte eine gültige Adresse angeben") };
                if r.name.trim().is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte einen Namen angeben");
                }
                let mut f = match self.fleet.iter().find(|x| x.id == id) {
                    Some(x) => x.clone(),
                    None if id.is_empty() => FleetServer { id: self.id(), created: now, ..FleetServer::default() },
                    None => return err(404, code::NOT_FOUND, "Überwachter Server nicht gefunden"),
                };
                if id.is_empty() && r.token.trim().is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte das Überwachungs-Token des Servers angeben");
                }
                if url.starts_with("https") && r.fingerprint.is_empty() && f.url != url {
                    return err(400, code::BAD_REQUEST, "Bitte zuerst die Zertifizierungsstelle des Servers bestätigen");
                }
                let host = url.split("://").nth(1).unwrap_or("").rsplit_once(':').map_or("", |(h, _)| h);
                f.name = s(r.name.trim());
                f.url = url.clone();
                if !r.fingerprint.is_empty() {
                    f.fingerprint = r.fingerprint.clone();
                }
                f.status = s("ok");
                f.checked = now;
                f.seen = now;
                f.report = Summary { server_name: s(host.split('.').next().unwrap_or(host)), version: s(bc_proto::VERSION), jobs: 6, jobs_enabled: 6, hosts: 3, hosts_online: 3, points: 120, ok_24h: 6,
                                     storage_used: 2_400 * GB, storage_capacity: 8 * TB, storage_free: 5_200 * GB, storage_logical: 6_000 * GB, ..Summary::default() };
                f.message = s("6 Jobs, 120 Restore-Points");
                self.log_audit(&actor, if id.is_empty() { "fleet.add" } else { "fleet.update" }, &f.name, &url);
                match self.fleet.iter().position(|x| x.id == f.id) {
                    Some(p) => self.fleet[p] = f.clone(),
                    None => self.fleet.push(f.clone()),
                }
                (if id.is_empty() { 201 } else { 200 }, f.to_json())
            }
            (Method::Delete, "/api/fleet/servers/{id}") => {
                let Some(p) = self.fleet.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Überwachter Server nicht gefunden") };
                let f = self.fleet.remove(p);
                self.log_audit(&actor, "fleet.remove", &f.name, &f.url);
                (204, Value::Null)
            }
            (Method::Post, "/api/fleet/servers/{id}/check") => {
                let Some(p) = self.fleet.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Überwachter Server nicht gefunden") };
                let f = &mut self.fleet[p];
                f.checked = now;
                if f.status != "unreachable" {
                    f.seen = now;
                }
                (200, f.to_json())
            }
            (Method::Get, "/api/fleet/tokens") => (200, list(&self.fleet_tokens)),
            (Method::Post, "/api/fleet/tokens") => {
                let label = api::FleetTokenRequest::from_json(&body).map(|r| s(r.label.trim())).unwrap_or_default();
                if label.is_empty() {
                    return err(400, code::BAD_REQUEST, "Bitte eine Bezeichnung angeben");
                }
                let tid = self.id();
                let mut k = [0u8; 24];
                for b in k.iter_mut() {
                    *b = self.rand() as u8;
                }
                let token = format!("bcf_{}", ids::hex(&k));
                self.fleet_tokens.push(FleetToken { id: tid.clone(), label: label.clone(), created: now, created_by: actor.clone(), last_used: 0 });
                self.log_audit(&actor, "fleet.token.create", &label, "");
                (201, api::FleetTokenCreated { id: tid, label, token }.to_json())
            }
            (Method::Delete, "/api/fleet/tokens/{id}") => {
                let Some(p) = self.fleet_tokens.iter().position(|x| x.id == id) else { return err(404, code::NOT_FOUND, "Überwachungs-Token nicht gefunden") };
                let x = self.fleet_tokens.remove(p);
                self.log_audit(&actor, "fleet.token.revoke", &x.label, "");
                (204, Value::Null)
            }
            _ => err(404, code::NOT_FOUND, "Im Demo-Server nicht vorhanden"),
        }
    }

    fn summary(&self) -> Summary {
        let now = self.now;
        let day0 = now - DAY;
        let recent = |st: SessionStatus| self.sessions.iter().filter(|x| x.started >= day0 && x.status == st).count() as u32;
        let mut history = Vec::new();
        for d in (0..14).rev() {
            let a = now - (d + 1) * DAY;
            let b = now - d * DAY;
            let w = self.sessions.iter().filter(|x| x.started >= a && x.started < b);
            let (mut ok, mut warn, mut fail) = (0, 0, 0);
            for x in w {
                match x.status {
                    SessionStatus::Success => ok += 1,
                    SessionStatus::Warning => warn += 1,
                    SessionStatus::Failed => fail += 1,
                    _ => {}
                }
            }
            history.push(DayStat { day: b, ok, warn, fail });
        }
        let mut next: Vec<NextRun> = self.jobs.iter().filter(|j| j.next_run > 0).map(|j| NextRun { job_id: j.id.clone(), job_name: j.name.clone(), at: j.next_run }).collect();
        next.sort_by_key(|n| n.at);
        next.truncate(5);
        let mut alerts = Vec::new();
        for j in &self.jobs {
            if j.last_status == SessionStatus::Failed {
                alerts.push(Alert { severity: Severity::Critical, title: format!("{} ist fehlgeschlagen", j.name), detail: s("Agent nicht erreichbar"), target_kind: s("job"), target_id: j.id.clone(), at: j.last_run });
            } else if j.last_status == SessionStatus::Warning {
                alerts.push(Alert { severity: Severity::Warning, title: format!("{} mit Warnungen", j.name), detail: s("2 Dateien waren gesperrt"), target_kind: s("job"), target_id: j.id.clone(), at: j.last_run });
            }
        }
        for r in &self.repos {
            if r.capacity > 0 && r.free * 10 < r.capacity {
                let detail = format!("{} % belegt, noch {} frei", r.disk_used() * 100 / r.capacity, crate::fmt::size(r.free));
                alerts.push(Alert { severity: Severity::Warning, title: format!("Repository „{}“ fast voll", r.name), detail, target_kind: s("repository"), target_id: r.id.clone(), at: now });
            }
        }
        for h in &self.hosts {
            if h.status == HostStatus::Offline {
                alerts.push(Alert { severity: Severity::Critical, title: format!("{} ist offline", h.name), detail: s("Letzter Kontakt vor 2 Tagen"), target_kind: s("host"), target_id: h.id.clone(), at: h.last_seen });
            }
        }
        alerts.extend(self.fleet_alerts());
        let used: u64 = self.repos.iter().map(|r| r.used).sum();
        let (cap, free) = bc_proto::model::disks(&self.repos);
        let logical: u64 = self.repos.iter().map(|r| r.logical).sum();
        Summary {
            server_name: s("backup-srv01"),
            version: s(bc_proto::VERSION),
            jobs: self.jobs.len() as u32,
            jobs_enabled: self.jobs.iter().filter(|j| j.enabled).count() as u32,
            hosts: self.hosts.len() as u32,
            hosts_online: self.hosts.iter().filter(|h| h.status == HostStatus::Online).count() as u32,
            protected_items: self.jobs.iter().map(|j| j.sources.len() as u32).sum::<u32>() + 236,
            points: self.points.len() as u32,
            ok_24h: recent(SessionStatus::Success),
            warn_24h: recent(SessionStatus::Warning),
            fail_24h: recent(SessionStatus::Failed),
            running: self.runs.len() as u32,
            storage_used: used,
            storage_capacity: cap,
            storage_free: free,
            storage_logical: logical,
            history,
            next_runs: next,
            alerts,
            protected_history: (0..14).map(|d| 950 * GB + d * 6 * GB).collect(),
        }
    }

    // ------------------------------------------------------------------ simulation

    fn start(&mut self, actor: &str, kind: SessionKind, job: &Job, what: &str, point: String) -> String {
        let op = self.id();
        let sid = self.id();
        let total = match kind {
            SessionKind::Restore => (job.last_size / 40).max(400_000_000),
            _ => job.last_size.max(1_000_000_000),
        };
        let speed = (total / 26).max(1);
        self.ops.insert(0, Operation {
            id: op.clone(),
            kind: s(match kind {
                SessionKind::Backup => "job.run",
                SessionKind::Restore => "restore",
                _ => "point.verify",
            }),
            title: format!("{}: {}", job.name, what),
            owner: s(actor),
            status: OpStatus::Running,
            done: 0,
            total,
            created: self.now,
            started: self.now,
            finished: 0,
            result: String::new(),
            error: String::new(),
            session_id: sid.clone(),
            target_id: job.id.clone(),
        });
        self.sessions.push(Session {
            id: sid.clone(),
            kind,
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            status: SessionStatus::Running,
            started: self.now,
            bytes_total: total,
            files_total: 12_000 + total / 4_000_000,
            operation_id: op.clone(),
            point_id: point.clone(),
            ..Session::default()
        });
        self.runs.push(Run { op: op.clone(), session: sid, job: job.id.clone(), kind, started: self.now, total, files: 12_000 + total / 4_000_000, speed, point });
        op
    }

    fn quick_op(&mut self, actor: &str, kind: &str, title: &str, result: Result<&str, &str>) -> String {
        let op = self.id();
        let (status, res, error) = match result {
            Ok(r) => (OpStatus::Running, s(r), String::new()),
            Err(e) => (OpStatus::Running, String::new(), s(e)),
        };
        self.ops.insert(0, Operation {
            id: op.clone(),
            kind: s(kind),
            title: s(title),
            owner: s(actor),
            status,
            done: 0,
            total: 100,
            created: self.now,
            started: self.now,
            finished: 0,
            result: res,
            error,
            session_id: String::new(),
            target_id: String::new(),
        });
        self.runs.push(Run { op: op.clone(), session: String::new(), job: String::new(), kind: SessionKind::Maintenance, started: self.now, total: 100, files: 0, speed: 34, point: String::new() });
        op
    }

    /// advance the clock: running work progresses and finishes -> work is running
    pub fn tick(&mut self, now: i64) -> bool {
        if now <= self.now {
            return !self.runs.is_empty();
        }
        self.now = now;
        let mut finished = Vec::new();
        for (i, r) in self.runs.iter().enumerate() {
            let el = (now - r.started).max(0) as u64;
            let done = (el * r.speed).min(r.total);
            let cancelled = self.ops.iter().any(|o| o.id == r.op && o.status == OpStatus::Cancelling);
            if let Some(o) = self.ops.iter_mut().find(|o| o.id == r.op) {
                o.done = done;
            }
            if let Some(sx) = self.sessions.iter_mut().find(|x| x.id == r.session) {
                sx.bytes_read = done;
                sx.files_done = r.files * done / r.total.max(1);
                sx.bytes_new = done / 38;
                sx.dedup_x100 = 270;
                sx.compress_x100 = 150;
            }
            if done >= r.total || cancelled {
                finished.push((i, cancelled));
            }
        }
        for (i, cancelled) in finished.into_iter().rev() {
            let r = self.runs.remove(i);
            let ok_status = if cancelled { OpStatus::Cancelled } else { OpStatus::Succeeded };
            let mut failed = false;
            if let Some(o) = self.ops.iter_mut().find(|o| o.id == r.op) {
                if !o.error.is_empty() {
                    failed = true;
                }
                o.status = if failed { OpStatus::Failed } else { ok_status };
                o.finished = now;
                if o.result.is_empty() && !failed {
                    o.result = s(match r.kind {
                        SessionKind::Backup => "Restore-Point erstellt",
                        SessionKind::Restore => "Dateien wiederhergestellt",
                        SessionKind::Verify => "Alle Daten lesbar und unverändert",
                        _ => "Abgeschlossen",
                    });
                }
            }
            let st = if cancelled { SessionStatus::Cancelled } else if failed { SessionStatus::Failed } else { SessionStatus::Success };
            if let Some(sx) = self.sessions.iter_mut().find(|x| x.id == r.session) {
                sx.status = st;
                sx.finished = now;
            }
            if r.kind == SessionKind::Backup {
                if let Some(ji) = self.jobs.iter().position(|j| j.id == r.job) {
                    if !cancelled {
                        // the restore point of the run
                        let pid = self.id();
                        let job = self.jobs[ji].clone();
                        self.points.push(RestorePoint {
                            id: pid.clone(),
                            job_id: job.id.clone(),
                            job_name: job.name.clone(),
                            created: now,
                            full: false,
                            files: r.files,
                            size: r.total,
                            stored_new: r.total / 38,
                            immutable_until: 0,
                            verified: 0,
                            host: self.hosts.iter().find(|h| h.id == job.host_id).map_or(String::new(), |h| h.name.clone()),
                            encrypted: job.encrypted,
                            signed: true,
                            repository_id: job.repository_id.clone(),
                            copied_from: String::new(),
                            created_ns: 0,
                        });
                        if let Some(sx) = self.sessions.iter_mut().find(|x| x.id == r.session) {
                            sx.point_id = pid;
                        }
                        self.jobs[ji].points += 1;
                    }
                    self.jobs[ji].last_status = st;
                    self.jobs[ji].last_run = r.started;
                }
            }
            if r.kind == SessionKind::Verify {
                if let Some(p) = self.points.iter_mut().find(|p| p.id == r.point) {
                    p.verified = now;
                }
            }
            if let Some(h) = self.hosts.iter_mut().find(|h| h.status == HostStatus::Pairing) {
                h.status = HostStatus::Online;
                h.last_seen = now;
                h.message = String::new();
                h.fingerprint = s("5d1c0e9a77b2f4c3a8e61d29b0f57a4c3e2d1b0a9f8e7d6c5b4a39281706f5e4");
            }
        }
        !self.runs.is_empty()
    }
}

/// a session's log, made from its figures
/// "412.000"
fn fmt_count(n: u64) -> String {
    let d = format!("{}", n);
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// "12.10. 21:58"
fn fmt_hm(secs: i64, zone: &Zone) -> String {
    let l = zone.to_local(Timestamp::from_secs(secs));
    format!("{:02}.{:02}. {:02}:{:02}", l.day, l.month, l.hour, l.minute)
}

fn session_log(x: &Session, jobs: &[Job]) -> Vec<LogLine> {
    let job = jobs.iter().find(|j| j.id == x.job_id);
    let mut v = Vec::new();
    let mut at = x.started;
    let mut push = |dt: i64, level: LogLevel, msg: String| {
        at += dt;
        v.push(LogLine { at, level, msg });
    };
    push(0, LogLevel::Info, format!("Sitzung gestartet: {}", x.job_name));
    if let Some(j) = job {
        if !j.pre_script.is_empty() {
            push(2, LogLevel::Info, format!("Vor-Skript ausgeführt: {} (Exitcode 0)", j.pre_script));
        }
        for src in &j.sources {
            push(1, LogLevel::Info, format!("Quelle wird gelesen: {}", src));
        }
    }
    push(3, LogLevel::Info, String::from("Änderungen ermittelt (Größe, Änderungszeit, Datei-ID)"));
    if x.warnings > 0 {
        push(40, LogLevel::Warning, String::from("Datei gesperrt, übersprungen: D:\\Daten\\Projekte\\~$Planung.xlsx"));
        push(2, LogLevel::Warning, String::from("Datei gesperrt, übersprungen: D:\\Daten\\Buchhaltung\\~$Abschluss.docx"));
    }
    if x.status == SessionStatus::Failed {
        push(60, LogLevel::Error, x.message.clone());
        push(1, LogLevel::Info, String::from("Sitzung abgebrochen, kein Restore-Point erstellt"));
        return v;
    }
    push(90, LogLevel::Info, format!("{} Dateien gelesen, {} neue Daten nach Dedupe und Kompression", crate::fmt::count(x.files_done), crate::fmt::size(x.bytes_new)));
    if x.status != SessionStatus::Running {
        push(5, LogLevel::Info, String::from("Restore-Point signiert und veröffentlicht"));
        push(1, LogLevel::Info, String::from("Aufbewahrung angewendet: 1 alter Punkt entfernt"));
        push(1, LogLevel::Info, String::from("Sitzung beendet"));
    }
    v
}

/// a deterministic file tree under the roots: "" lists the roots, below names from a hash
fn browse(roots: &[String], path: &str, mtime: i64) -> Listing {
    let mut entries = Vec::new();
    if path.is_empty() || path == "/" {
        for r in roots {
            entries.push(Entry { name: r.clone(), kind: EntryKind::Dir, size: 0, mtime });
        }
        return Listing { path: String::new(), entries, more: false, offset: 0 };
    }
    let h = ids::key(path);
    let depth = path.matches(['/', '\\']).count();
    let dirs = ["Projekte", "Buchhaltung", "Archiv", "Personal", "Vorlagen", "Temp", "Marketing", "Verträge"];
    let files = [("Angebot_2026.docx", 48_200u64), ("Planung.xlsx", 612_000), ("Protokoll.pdf", 1_240_000), ("Logo.png", 88_400),
                 ("Bericht_Q3.pdf", 2_310_000), ("notes.txt", 4_100), ("Export.csv", 940_000), ("Präsentation.pptx", 6_800_000)];
    if depth < 4 {
        for k in 0..(3 + (h % 4) as usize) {
            entries.push(Entry { name: String::from(dirs[(h as usize + k * 3) % dirs.len()]), kind: EntryKind::Dir, size: 0, mtime: mtime - (k as i64) * 7200 });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries.dedup_by(|a, b| a.name == b.name);
    }
    for k in 0..(2 + (h >> 8) % 6) as usize {
        let (n, sz) = files[(h as usize >> 3).wrapping_add(k * 5) % files.len()];
        entries.push(Entry { name: String::from(n), kind: EntryKind::File, size: sz + (h % 977) * k as u64, mtime: mtime - (k as i64 + 1) * 86_400 });
    }
    entries.dedup_by(|a, b| a.name == b.name);
    Listing { path: String::from(path), entries, more: false, offset: 0 }
}
