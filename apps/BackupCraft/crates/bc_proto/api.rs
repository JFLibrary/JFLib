// api -- the REST interface: one route table for the server's router, its role checks and
// audit, bcctl's help, the console's permission hints and /api/docs; the request bodies and
// the error format.
//
// Conventions: JSON bodies; errors {code, message}; long actions answer 202 {operation_id}
// (poll GET /api/operations/{id}); "{id}" path parameters; Authorization: Bearer <token>.
// A route with a capability answers 501 {code: "capability", message} while that feature is
// staged (not available on this server).

use alloc::string::String;
use alloc::vec::Vec;

use crate::model::{Retention, Role, Schedule};
use crate::{record, wire_enum};

wire_enum! {
    pub enum Method { Get = "GET", Post = "POST", Put = "PUT", Delete = "DELETE" }
}

/// who may call a route
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// everybody (health, login, setup with its token)
    Public,
    /// a logged-in user with at least this role
    Role(Role),
    /// a paired agent (mutual TLS)
    Agent,
    /// a central server with a fleet token (it reads this server's report, nothing else)
    Fleet,
}

/// one route of the API
#[derive(Clone, Copy, Debug)]
pub struct Route {
    pub method: Method,
    pub path: &'static str,
    pub access: Access,
    /// runs as an operation (202 + operation id)
    pub long: bool,
    /// the staged feature this route belongs to ("" = core)
    pub capability: &'static str,
    pub summary: &'static str,
}

const fn r(method: Method, path: &'static str, access: Access, summary: &'static str) -> Route {
    Route { method, path, access, long: false, capability: "", summary }
}

const fn op(method: Method, path: &'static str, access: Access, summary: &'static str) -> Route {
    Route { method, path, access, long: true, capability: "", summary }
}

/// a long action (operation) of an optional feature
const fn feature_op(method: Method, path: &'static str, access: Access, capability: &'static str, summary: &'static str) -> Route {
    Route { method, path, access, long: true, capability, summary }
}

/// a route of an optional feature, with its own access
const fn feature(method: Method, path: &'static str, access: Access, capability: &'static str, summary: &'static str) -> Route {
    Route { method, path, access, long: false, capability, summary }
}

use Access::{Agent, Public};
use Method::{Delete, Get, Post, Put};

const VIEW: Access = Access::Role(Role::Viewer);
const RESTORE: Access = Access::Role(Role::Restore);
const OPER: Access = Access::Role(Role::Operator);
const ADMIN: Access = Access::Role(Role::Admin);

/// the staged features (capability names)
pub const CAP_TAPE: &str = "tape";
pub const CAP_REPLICATION: &str = "replication";
pub const CAP_INSTANT: &str = "instant-start";
pub const CAP_BOOTCHECK: &str = "boot-check";
pub const CAP_CDP: &str = "cdp";
pub const CAP_HYPERVISOR: &str = "hypervisor";
pub const CAP_FLEET: &str = "fleet";
pub const CAP_NOTIFY: &str = "notifications";
pub const CAP_UPDATE: &str = "update";
pub const CAP_VOLUME: &str = "volume-backup";
pub const STAGED: &[&str] = &[CAP_TAPE, CAP_REPLICATION, CAP_INSTANT, CAP_BOOTCHECK, CAP_CDP, CAP_HYPERVISOR, CAP_FLEET,
                              CAP_NOTIFY, CAP_UPDATE, CAP_VOLUME];
/// the staged features this version has (their routes work; the others answer 501)
pub const AVAILABLE: &[&str] = &[CAP_NOTIFY, CAP_UPDATE, CAP_REPLICATION, CAP_CDP, CAP_FLEET, CAP_TAPE, CAP_HYPERVISOR, CAP_INSTANT, CAP_BOOTCHECK, CAP_VOLUME];

pub fn available(cap: &str) -> bool {
    AVAILABLE.contains(&cap)
}

/// every route of the API
pub static ROUTES: &[Route] = &[
    r(Get, "/health", Public, "Lebenszeichen des Servers"),
    r(Get, "/api", Public, "Servername, Version, Einrichtungsstatus"),
    r(Post, "/api/setup", Public, "Ersteinrichtung mit dem Setup-Token: erster Administrator"),
    r(Post, "/api/auth/login", Public, "Anmelden: Benutzername und Passwort -> Token"),
    r(Post, "/api/auth/logout", VIEW, "Abmelden (Token ungültig machen)"),
    r(Get, "/api/auth/me", VIEW, "Der angemeldete Benutzer"),
    r(Get, "/api/summary", VIEW, "Kennzahlen für die Übersicht"),
    r(Get, "/api/capabilities", VIEW, "Verfügbare Funktionen dieses Servers"),
    r(Get, "/api/jobs", VIEW, "Sicherungsjobs"),
    r(Post, "/api/jobs", OPER, "Job anlegen"),
    r(Get, "/api/jobs/{id}", VIEW, "Ein Job"),
    r(Put, "/api/jobs/{id}", OPER, "Job ändern"),
    r(Delete, "/api/jobs/{id}", ADMIN, "Job löschen (Restore-Points bleiben)"),
    op(Post, "/api/jobs/{id}/run", OPER, "Job jetzt starten"),
    r(Post, "/api/jobs/{id}/enable", OPER, "Job aktivieren"),
    r(Post, "/api/jobs/{id}/disable", OPER, "Job deaktivieren"),
    r(Get, "/api/jobs/{id}/points", VIEW, "Restore-Points eines Jobs"),
    r(Get, "/api/points/{id}", VIEW, "Ein Restore-Point"),
    r(Get, "/api/points/{id}/browse", RESTORE, "Inhalt eines Restore-Points (?path=, ?offset=)"),
    op(Post, "/api/points/{id}/restore", RESTORE, "Dateien wiederherstellen"),
    op(Post, "/api/points/{id}/verify", OPER, "Restore-Point prüfen (alle Daten lesen)"),
    r(Delete, "/api/points/{id}", ADMIN, "Restore-Point löschen (nicht bei Unveränderlichkeit)"),
    r(Get, "/api/sessions", VIEW, "Sitzungen (?limit=, ?job=)"),
    r(Get, "/api/sessions/{id}", VIEW, "Eine Sitzung"),
    r(Get, "/api/sessions/{id}/log", VIEW, "Protokoll einer Sitzung"),
    r(Get, "/api/operations", VIEW, "Laufende und letzte Vorgänge"),
    r(Get, "/api/operations/{id}", VIEW, "Ein Vorgang"),
    r(Post, "/api/operations/{id}/cancel", OPER, "Vorgang abbrechen"),
    r(Get, "/api/hosts", VIEW, "Server und Agenten"),
    r(Get, "/api/hosts/{id}", VIEW, "Ein Host"),
    r(Put, "/api/hosts/{id}", ADMIN, "Host ändern (Name, freigegebene Ordner)"),
    r(Delete, "/api/hosts/{id}", ADMIN, "Agent entfernen (Zertifikat sperren)"),
    r(Get, "/api/hosts/{id}/browse", OPER, "Dateisystem eines Hosts (?path=)"),
    op(Post, "/api/hosts/{id}/check", OPER, "Verbindung zum Host prüfen"),
    r(Post, "/api/hosts/pairing", ADMIN, "Pairing-Code für einen neuen Agenten erzeugen"),
    op(Post, "/api/hosts/ssh-push", ADMIN, "Agent per SSH installieren und koppeln"),
    r(Post, "/api/hosts/ssh-fingerprint", ADMIN, "SSH-Host-Schlüssel eines Rechners abfragen"),
    r(Post, "/api/agent/enroll", Public, "Agent koppeln: Pairing-Code + Schlüssel -> Zertifikat"),
    r(Post, "/api/agent/heartbeat", Agent, "Lebenszeichen und Status eines Agenten"),
    r(Get, "/api/repositories", VIEW, "Repositories"),
    r(Post, "/api/repositories", ADMIN, "Repository anlegen oder einbinden"),
    r(Get, "/api/repositories/{id}", VIEW, "Ein Repository"),
    r(Put, "/api/repositories/{id}", ADMIN, "Repository ändern (Name, Unveränderlichkeit)"),
    r(Delete, "/api/repositories/{id}", ADMIN, "Repository austragen (Daten bleiben)"),
    op(Post, "/api/repositories/{id}/check", OPER, "Repository prüfen"),
    op(Post, "/api/repositories/{id}/gc", ADMIN, "Nicht mehr benötigte Daten freigeben"),
    r(Get, "/api/credentials", ADMIN, "Zugangsdaten"),
    r(Post, "/api/credentials", ADMIN, "Zugangsdaten speichern"),
    r(Delete, "/api/credentials/{id}", ADMIN, "Zugangsdaten löschen"),
    r(Get, "/api/users", ADMIN, "Benutzer"),
    r(Post, "/api/users", ADMIN, "Benutzer anlegen"),
    r(Put, "/api/users/{id}", ADMIN, "Benutzer ändern (Rolle, Passwort, gesperrt)"),
    r(Delete, "/api/users/{id}", ADMIN, "Benutzer löschen"),
    r(Post, "/api/users/me/password", VIEW, "Eigenes Passwort ändern"),
    r(Get, "/api/audit", ADMIN, "Audit-Protokoll (?limit=)"),
    r(Get, "/api/settings", ADMIN, "Einstellungen"),
    r(Put, "/api/settings", ADMIN, "Einstellungen ändern"),
    r(Get, "/api/recovery", ADMIN, "Recovery-Kit: Status"),
    r(Post, "/api/recovery/kit", ADMIN, "Recovery-Kit erzeugen (privater Schlüssel einmalig)"),
    r(Get, "/api/config/backups", ADMIN, "Konfigurationssicherungen"),
    op(Post, "/api/config/backups", ADMIN, "Konfiguration sichern (verschlüsselt)"),
    r(Post, "/api/config/restore", ADMIN, "Konfiguration beim nächsten Start wiederherstellen"),
    feature(Get, "/api/tape", VIEW, CAP_TAPE, "Bandbibliothek: Ordner, Bänder, belegt, frei"),
    feature(Get, "/api/tape/tapes", VIEW, CAP_TAPE, "Bänder der Bibliothek"),
    feature(Post, "/api/tape/tapes", ADMIN, CAP_TAPE, "Leere Bänder anlegen (Anzahl, Kapazität, Barcode-Präfix, Pool)"),
    feature(Put, "/api/tape/tapes/{id}", ADMIN, CAP_TAPE, "Leeres Band einem Pool zuordnen oder freigeben"),
    feature(Delete, "/api/tape/tapes/{id}", ADMIN, CAP_TAPE, "Band aus der Bibliothek entfernen (nicht geschützt)"),
    feature_op(Post, "/api/tape/tapes/{id}/erase", ADMIN, CAP_TAPE, "Band löschen (nach Ablauf des Schutzes)"),
    feature_op(Post, "/api/tape/rescan", ADMIN, CAP_TAPE, "Bibliothek einlesen: Bänder und Sätze katalogisieren"),
    feature(Get, "/api/tape/pools", VIEW, CAP_TAPE, "Medienpools"),
    feature(Post, "/api/tape/pools", ADMIN, CAP_TAPE, "Medienpool anlegen"),
    feature(Put, "/api/tape/pools/{id}", ADMIN, CAP_TAPE, "Medienpool ändern"),
    feature(Delete, "/api/tape/pools/{id}", ADMIN, CAP_TAPE, "Medienpool löschen (ohne beschriebene Bänder)"),
    feature(Get, "/api/tape/sets", VIEW, CAP_TAPE, "Auf Band geschriebene Restore-Points (?job=)"),
    feature(Get, "/api/tape/sets/{id}/browse", RESTORE, CAP_TAPE, "Inhalt eines Bandsatzes (?path=, aus dem Katalog)"),
    feature_op(Post, "/api/tape/sets/{id}/restore", RESTORE, CAP_TAPE, "Vom Band wiederherstellen (ohne Repository)"),
    feature_op(Post, "/api/tape/sets/{id}/verify", OPER, CAP_TAPE, "Bandsatz prüfen: alles lesen und vergleichen"),
    feature_op(Post, "/api/points/{id}/tape", OPER, CAP_TAPE, "Restore-Point auf Band schreiben (Medienpool)"),
    feature(Get, "/api/replicas", VIEW, CAP_REPLICATION, "Replikate"),
    feature(Post, "/api/replicas", ADMIN, CAP_REPLICATION, "Replikat anlegen (Job, Zielhost, Ordner)"),
    feature(Put, "/api/replicas/{id}", ADMIN, CAP_REPLICATION, "Replikat ändern"),
    feature(Delete, "/api/replicas/{id}", ADMIN, CAP_REPLICATION, "Replikat austragen (die Dateien bleiben)"),
    feature_op(Post, "/api/replicas/{id}/sync", OPER, CAP_REPLICATION, "Replikat auf den neuesten Restore-Point bringen"),
    feature(Post, "/api/replicas/{id}/failover", OPER, CAP_REPLICATION, "Failover: das Replikat statt des Originals verwenden"),
    feature_op(Post, "/api/replicas/{id}/failback", OPER, CAP_REPLICATION, "Failback: Änderungen am Replikat zurück auf das Original"),
    feature_op(Post, "/api/replicas/{id}/undo", OPER, CAP_REPLICATION, "Failover verwerfen: Replikat wieder auf den Restore-Point"),
    feature(Get, "/api/failover-plans", VIEW, CAP_REPLICATION, "Failover-Pläne"),
    feature(Post, "/api/failover-plans", ADMIN, CAP_REPLICATION, "Failover-Plan anlegen"),
    feature(Put, "/api/failover-plans/{id}", ADMIN, CAP_REPLICATION, "Failover-Plan ändern"),
    feature(Delete, "/api/failover-plans/{id}", ADMIN, CAP_REPLICATION, "Failover-Plan löschen"),
    feature_op(Post, "/api/failover-plans/{id}/run", OPER, CAP_REPLICATION, "Failover-Plan ausführen"),
    feature(Get, "/api/instant-start", VIEW, CAP_INSTANT, "VMs, die aus Restore-Points laufen"),
    feature_op(Post, "/api/points/{id}/instant-start", OPER, CAP_INSTANT, "VM direkt aus einem Restore-Point starten (vm, memory_mb, network)"),
    feature(Get, "/api/instant-start/{id}/console", VIEW, CAP_INSTANT, "Ausgabe der seriellen Konsole"),
    feature(Post, "/api/instant-start/{id}/stop", OPER, CAP_INSTANT, "VM beenden; Änderungen verwerfen (nach dem Zusammenführen bleiben die Platten)"),
    feature_op(Post, "/api/instant-start/{id}/migrate", OPER, CAP_INSTANT, "Zusammenführen: Platten im Betrieb in einen Ordner übertragen"),
    feature(Get, "/api/boot-checks", VIEW, CAP_BOOTCHECK, "Startprüfungen und ihr Ergebnis"),
    feature_op(Post, "/api/points/{id}/boot-check", OPER, CAP_BOOTCHECK, "Startprüfung: VM isoliert starten und prüfen (Start, Konsolentext, Port)"),
    feature(Get, "/api/cdp", VIEW, CAP_CDP, "Jobs mit kontinuierlicher Sicherung (CDP) und ihr Zustand"),
    feature(Get, "/api/hosts/{id}/volumes", VIEW, CAP_VOLUME, "Volumes eines Hosts für blockweise Sicherungen (bisher der Backup-Server)"),
    feature(Get, "/api/hypervisors", VIEW, CAP_HYPERVISOR, "Hypervisoren (QEMU/KVM)"),
    feature(Get, "/api/vms", VIEW, CAP_HYPERVISOR, "Virtuelle Maschinen der Hypervisoren"),
    feature(Get, "/api/fleet", VIEW, CAP_FLEET, "Überwachte BackupCraft-Server und ihr letzter Bericht"),
    feature(Post, "/api/fleet/probe", ADMIN, CAP_FLEET, "Die Zertifizierungsstelle eines Servers lesen (vor dem Anlegen bestätigen)"),
    feature(Post, "/api/fleet/servers", ADMIN, CAP_FLEET, "Server überwachen (Name, Adresse, Überwachungs-Token, Fingerabdruck der CA)"),
    feature(Put, "/api/fleet/servers/{id}", ADMIN, CAP_FLEET, "Überwachten Server ändern (leeres Token: das bisherige bleibt)"),
    feature(Delete, "/api/fleet/servers/{id}", ADMIN, CAP_FLEET, "Server nicht mehr überwachen"),
    feature(Post, "/api/fleet/servers/{id}/check", OPER, CAP_FLEET, "Server jetzt abfragen"),
    feature(Get, "/api/fleet/tokens", ADMIN, CAP_FLEET, "Überwachungs-Tokens, mit denen Zentralen diesen Server lesen"),
    feature(Post, "/api/fleet/tokens", ADMIN, CAP_FLEET, "Überwachungs-Token erzeugen (wird nur einmal gezeigt)"),
    feature(Delete, "/api/fleet/tokens/{id}", ADMIN, CAP_FLEET, "Überwachungs-Token widerrufen"),
    feature(Get, "/api/fleet/report", Access::Fleet, CAP_FLEET, "Bericht dieses Servers für eine Zentrale (mit Überwachungs-Token)"),
    feature(Post, "/api/settings/test-email", ADMIN, CAP_NOTIFY, "Testnachricht senden (E-Mail und Web-Hook)"),
    feature(Get, "/api/notify", ADMIN, CAP_NOTIFY, "Benachrichtigungen: letzter Versand"),
    feature(Post, "/api/notify/summary", ADMIN, CAP_NOTIFY, "Tageszusammenfassung jetzt senden"),
    feature(Get, "/api/update", VIEW, CAP_UPDATE, "Update-Status: diese Version, der Kanal, die neueste Version"),
    feature(Post, "/api/update/check", ADMIN, CAP_UPDATE, "Jetzt nach Updates suchen (signiertes Manifest)"),
    Route { method: Post, path: "/api/update/download", access: ADMIN, long: true, capability: CAP_UPDATE, summary: "Das Paket dieser Plattform herunterladen und prüfen" },
];

/// the route matching a request and the values of its "{name}" parameters
pub fn route_match<'a>(method: &str, path: &'a str) -> Option<(&'static Route, Vec<(&'static str, &'a str)>)> {
    let path = path.split('?').next().unwrap_or(path);
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let mut best: Option<(&'static Route, Vec<(&'static str, &'a str)>, usize)> = None;
    for rt in ROUTES {
        if rt.method.as_str() != method {
            continue;
        }
        let pat: Vec<&str> = rt.path.split('/').collect();
        if pat.len() != segs.len() {
            continue;
        }
        let mut params = Vec::new();
        let mut literal = 0;
        let mut ok = true;
        for (p, s) in pat.iter().zip(&segs) {
            if p.starts_with('{') && p.ends_with('}') {
                if s.is_empty() {
                    ok = false;
                    break;
                }
                params.push((&p[1..p.len() - 1], *s));
            } else if p == s {
                literal += 1;
            } else {
                ok = false;
                break;
            }
        }
        // the most literal match wins (/api/hosts/pairing over /api/hosts/{id})
        if ok && best.as_ref().is_none_or(|b| literal > b.2) {
            best = Some((rt, params, literal));
        }
    }
    best.map(|(r, p, _)| (r, p))
}

/// whether any route has this path (for 405 Method Not Allowed)
pub fn path_known(path: &str) -> bool {
    ["GET", "POST", "PUT", "DELETE"].iter().any(|m| route_match(m, path).is_some())
}

/// the value of a query parameter (?a=1&b=x), percent-decoded
pub fn query<'a>(path: &'a str, key: &str) -> Option<alloc::string::String> {
    let q = path.split_once('?')?.1;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(crate::ids::percent_decode(v));
        }
    }
    None
}

// ------------------------------------------------------------------ errors

/// error codes of {code, message}
pub mod code {
    pub const BAD_REQUEST: &str = "bad_request";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_FOUND: &str = "not_found";
    pub const CONFLICT: &str = "conflict";
    pub const CAPABILITY: &str = "capability";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const INTERNAL: &str = "internal";
    pub const SETUP_REQUIRED: &str = "setup_required";
    pub const IMMUTABLE: &str = "immutable";
    pub const BUSY: &str = "busy";
    /// a notification could not be delivered (the message says why)
    pub const NOTIFY: &str = "notify_failed";
    /// the update channel could not be read or checked
    pub const UPDATE: &str = "update_failed";
}

record! {
    /// an error answer
    pub struct ApiError {
        pub code: String,
        pub message: String,
        /// the field a validation error is about
        pub field: String,
    }
}

record! {
    /// 202: the action runs as this operation
    pub struct Accepted {
        pub operation_id: String,
    }
}

// ------------------------------------------------------------------ request bodies

record! {
    pub struct SetupRequest {
        pub token: String,
        pub username: String,
        pub password: String,
        pub server_name: String,
    }
}

record! {
    pub struct LoginRequest {
        pub username: String,
        pub password: String,
    }
}

record! {
    pub struct LoginResponse {
        pub token: String,
        pub expires: i64,
        pub user: crate::model::User,
    }
}

record! {
    /// create or change a job (the id is in the path when changing)
    pub struct JobRequest {
        pub name: String,
        pub description: String,
        pub kind: crate::model::JobKind,
        pub enabled: bool,
        pub host_id: String,
        pub sources: Vec<String>,
        pub excludes: Vec<String>,
        pub repository_id: String,
        pub schedule: Schedule,
        pub retention: Retention,
        pub encrypted: bool,
        /// the passphrase of an encrypted job (set when creating or changing the key)
        pub passphrase: String,
        pub key_hint: String,
        pub compression: crate::model::Compression,
        pub active_full_days: u32,
        pub pre_script: String,
        pub post_script: String,
        pub source_job: String,
        pub pool_id: String,
        pub cdp: bool,
        pub cdp_rpo: u32,
        pub cdp_keep_hours: u32,
    }
}

record! {
    pub struct RunRequest {
        /// read every file again (active full)
        pub full: bool,
    }
}

wire_enum! {
    /// what a restore does with files that exist at the target
    pub enum Conflict { Skip = "skip", Rename = "rename", Overwrite = "overwrite" }
}

record! {
    pub struct RestoreRequest {
        /// paths inside the restore point ("" or "/" = everything)
        pub items: Vec<String>,
        /// restore to the original places on the original host
        pub original: bool,
        pub host_id: String,
        /// target folder (alternative location)
        pub target: String,
        pub conflict: Conflict,
        /// the job passphrase of an encrypted point (when the server does not hold it)
        pub passphrase: String,
    }
}

record! {
    pub struct HostUpdate {
        pub name: String,
        pub roots: Vec<String>,
        pub restore_roots: Vec<String>,
    }
}

record! {
    pub struct SshPushRequest {
        pub address: String,
        pub port: u32,
        pub name: String,
        /// the SHA-256 host key fingerprint the user checked
        pub host_fingerprint: String,
        pub credential_id: String,
        pub username: String,
        pub password: String,
        pub roots: Vec<String>,
    }
}

record! {
    pub struct SshFingerprint {
        pub address: String,
        pub port: u32,
        pub fingerprint: String,
        pub key_type: String,
    }
}

record! {
    /// an agent's enrolment: the pairing code and its public key (Ed25519, hex)
    pub struct EnrollRequest {
        pub code: String,
        pub name: String,
        pub os: crate::model::Os,
        pub platform: String,
        pub version: String,
        pub public_key: String,
        pub address: String,
        pub roots: Vec<String>,
    }
}

record! {
    pub struct EnrollResponse {
        pub host_id: String,
        /// the agent certificate and the server CA (PEM)
        pub certificate: String,
        pub ca: String,
    }
}

record! {
    pub struct ReplicaRequest {
        pub name: String,
        pub job_id: String,
        pub host_id: String,
        pub target: String,
        pub auto: bool,
    }
}

record! {
    /// a server to watch; the token is one of its fleet tokens; fingerprint: the SHA-256 of
    /// its CA as POST /api/fleet/probe showed it (empty for http on loopback)
    pub struct FleetServerRequest {
        pub name: String,
        pub url: String,
        pub token: String,
        pub fingerprint: String,
    }
}

record! {
    pub struct ProbeRequest {
        pub url: String,
    }
}

record! {
    /// what a server presents: its address as used, whether it speaks TLS, its CA
    pub struct ProbeResult {
        pub url: String,
        pub tls: bool,
        /// SHA-256 of the CA (hex) and "AB:CD:..." for people
        pub fingerprint: String,
        pub fingerprint_text: String,
        pub ca_name: String,
    }
}

record! {
    pub struct FleetTokenRequest {
        pub label: String,
    }
}

record! {
    /// a new fleet token: `token` is shown this once
    pub struct FleetTokenCreated {
        pub id: String,
        pub label: String,
        pub token: String,
    }
}

record! {
    /// a VM of a restore point to start: its name in the point; memory 0 = as backed up;
    /// network "none" or "isolated"
    pub struct InstantRequest {
        pub vm: String,
        pub memory_mb: u64,
        pub network: String,
    }
}

record! {
    pub struct MigrateRequest {
        pub target: String,
    }
}

record! {
    /// what a boot check checks: the VM runs boot_seconds without failing; the console shows
    /// console_text (if any) within timeout_s; the guest answers on tcp_port (0 = no check)
    pub struct BootCheckRequest {
        pub vm: String,
        pub boot_seconds: u32,
        pub console_text: String,
        pub tcp_port: u32,
        pub timeout_s: u32,
    }
}

record! {
    /// new blank tapes: count, capacity (GB; or capacity_mb for small virtual tapes, at least
    /// 32), barcode prefix (2 to 4 letters), pool ("" = free)
    pub struct CartridgeRequest {
        pub count: u32,
        pub capacity_gb: u32,
        pub capacity_mb: u32,
        pub prefix: String,
        pub pool_id: String,
    }
}

record! {
    pub struct CartridgeUpdate {
        pub pool_id: String,
    }
}

record! {
    pub struct PoolRequest {
        pub name: String,
        pub description: String,
        pub protect_days: u32,
        pub encrypted: bool,
        pub append: bool,
    }
}

record! {
    pub struct TapeWriteRequest {
        pub pool_id: String,
    }
}

record! {
    pub struct PlanRequest {
        pub name: String,
        pub replicas: Vec<String>,
        pub delay_s: u32,
    }
}

record! {
    pub struct RepositoryRequest {
        pub name: String,
        pub path: String,
        /// create a new repository there (false: attach an existing one)
        pub create: bool,
        pub immutable_days: u32,
    }
}

record! {
    pub struct CredentialRequest {
        pub label: String,
        pub username: String,
        pub kind: crate::model::CredKind,
        pub secret: String,
    }
}

record! {
    pub struct UserRequest {
        pub username: String,
        pub role: crate::model::Role,
        pub password: String,
        pub disabled: bool,
    }
}

record! {
    pub struct PasswordChange {
        pub old: String,
        pub new: String,
    }
}

record! {
    pub struct ConfigBackupRequest {
        pub passphrase: String,
    }
}

record! {
    pub struct ConfigRestoreRequest {
        pub name: String,
        pub passphrase: String,
    }
}

record! {
    /// the answer of POST /api/recovery/kit: the private key, shown once
    pub struct RecoveryKit {
        pub private_key: String,
        pub fingerprint: String,
        pub created: i64,
    }
}
