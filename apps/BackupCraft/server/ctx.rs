// ctx -- the server's shared state: the data directory, the database, the journals, the keys,
// the operations; settings, audit and session logs
//
// <data>/
//   db/            datastore (collections below)
//   journal/       audit, s-<session id> (session logs)
//   keys/          master.key, signing.key, recovery.pub
//   pki/           ca.*, server.*, client.* (certificates; keys sealed with the master key)
//   setup.token    one-time token for the first administrator (deleted by the setup)
//   config_backups/, logs/
//
// Collections: meta (server), users (+phc), tokens (id = BLAKE3 of the token), login (failed
// attempts per user), settings (id = key), jobs (+passphrase sealed), hosts, repos, creds
// (+secret sealed), sessions, points, ops, pairing, replicas, plans (failover plans), fleet
// (watched servers, +token sealed), fleet_tokens (id = BLAKE3 of the token), tapes (id =
// barcode), pools (media pools), tapesets (+key sealed), vms (the VMs of hypervisors), instant
// (VMs started from points), bootcheck (boot check results).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Json, Value};
use bc_proto::model::{AuditEntry, LogLevel, LogLine, Setting};
use datastore::{Journal, Store};

use crate::keys::Keys;
use crate::ops::Ops;
use crate::util::{month, new_id, now};

pub const C_META: &str = "meta";
pub const C_USERS: &str = "users";
pub const C_TOKENS: &str = "tokens";
pub const C_LOGIN: &str = "login";
pub const C_SETTINGS: &str = "settings";
pub const C_JOBS: &str = "jobs";
pub const C_HOSTS: &str = "hosts";
pub const C_REPOS: &str = "repos";
pub const C_CREDS: &str = "creds";
pub const C_SESSIONS: &str = "sessions";
pub const C_POINTS: &str = "points";
pub const C_OPS: &str = "ops";
pub const C_PAIRING: &str = "pairing";
pub const C_REPLICAS: &str = "replicas";
pub const C_PLANS: &str = "plans";
pub const C_FLEET: &str = "fleet";
pub const C_FLEET_TOKENS: &str = "fleet_tokens";
pub const C_TAPES: &str = "tapes";
pub const C_POOLS: &str = "pools";
pub const C_TAPESETS: &str = "tapesets";
pub const C_VMS: &str = "vms";
pub const C_INSTANT: &str = "instant";
pub const C_BOOTCHECK: &str = "bootcheck";

/// the defaults of the settings (key, value)
pub const DEFAULT_SETTINGS: &[(&str, &str)] = &[
    ("server.name", ""),
    ("session.timeout_hours", "24"),
    ("login.max_attempts", "5"),
    ("restore.allowed_roots", ""),
    ("traffic.max_mbps", "0"),
    ("notify.mode", "off"),
    ("notify.daily", ""),
    ("notify.webhook", ""),
    ("smtp.host", ""),
    ("smtp.security", "starttls"),
    ("smtp.user", ""),
    ("smtp.password", ""),
    ("smtp.from", ""),
    ("smtp.to", ""),
    ("smtp.ca", ""),
    ("update.url", ""),
    ("update.key", ""),
    ("update.ca", ""),
    ("update.auto", "false"),
    ("config.auto_backup", "true"),
    // the folder of the virtual tape library ("" = <data>/vtl)
    ("tape.library", ""),
    // QEMU/KVM on this server: folders whose QMP sockets are VMs (one per line), the programs,
    // the work folder (dumps of disk backups, instant start; "" = <data>/vm)
    ("hypervisor.qmp", ""),
    ("hypervisor.qemu", "qemu-system-x86_64"),
    ("hypervisor.work", ""),
];

/// what the console sees of a stored secret setting
pub const SECRET_SET: &str = "********";

/// settings whose values are sealed with the master key and never shown
pub const SECRET_SETTINGS: &[&str] = &["smtp.password"];

static CURRENT: plat::sync::Mutex<Option<alloc::sync::Arc<Ctx>>> = plat::sync::Mutex::new(None);

/// the server's context, for threads that outlive a request (notifications)
pub fn current() -> Option<alloc::sync::Arc<Ctx>> {
    CURRENT.lock().clone()
}

pub fn set_current(c: &alloc::sync::Arc<Ctx>) {
    *CURRENT.lock() = Some(c.clone());
}

pub struct Ctx {
    pub data: String,
    pub db: Store,
    pub journal: Journal,
    pub keys: Keys,
    pub ops: Ops,
    /// the address the API listens on
    pub listen: plat::net::SocketAddr,
    pub tls: bool,
    /// the open repositories (a repository is opened once per process)
    pub repos: plat::sync::Mutex<alloc::collections::BTreeMap<String, alloc::sync::Arc<cas::Repo>>>,
    /// the certificate authority and the server's own certificates
    pub pki: crate::pki::Pki,
    /// kept-alive connections to agents ("host id#fingerprint")
    pub agents: plat::sync::Mutex<alloc::collections::BTreeMap<String, alloc::sync::Arc<plat::sync::Mutex<netio::client::Session>>>>,
}

impl Ctx {
    pub fn path(&self, name: &str) -> String {
        plat::fs::join(&self.data, name)
    }

    /// whether the first administrator exists
    pub fn is_set_up(&self) -> bool {
        self.db.get(C_META, "server").and_then(|v| v.get("setup").and_then(|x| x.as_bool())).unwrap_or(false)
    }

    pub fn setting(&self, key: &str) -> String {
        if let Some(v) = self.db.get(C_SETTINGS, key) {
            return String::from(j::str_of(&v, "value"));
        }
        DEFAULT_SETTINGS.iter().find(|(k, _)| *k == key).map_or(String::new(), |(_, v)| String::from(*v))
    }

    pub fn setting_u32(&self, key: &str, default: u32) -> u32 {
        self.setting(key).trim().parse().unwrap_or(default)
    }

    /// every setting (stored or default; secrets as SECRET_SET when they are set)
    pub fn settings(&self) -> Vec<Setting> {
        DEFAULT_SETTINGS
            .iter()
            .map(|(k, _)| {
                let v = self.setting(k);
                let value = if SECRET_SETTINGS.contains(k) && !v.is_empty() { String::from(SECRET_SET) } else { v };
                Setting { key: String::from(*k), value }
            })
            .collect()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), datastore::StoreError> {
        self.db.put(C_SETTINGS, key, Setting { key: String::from(key), value: String::from(value) }.to_json()).map(|_| ())
    }

    pub fn server_name(&self) -> String {
        let n = self.setting("server.name");
        if n.is_empty() { plat::net::hostname() } else { n }
    }

    /// one audit entry (the journal is fsynced: an audit trail survives crashes)
    pub fn audit(&self, actor: &str, action: &str, target: &str, detail: &str, ok: bool) {
        let at = now();
        let e = AuditEntry { id: new_id(), at, actor: String::from(actor), action: String::from(action), target: String::from(target), detail: String::from(detail), ok };
        let _ = self.journal.append("audit", month(at), &e.to_json());
    }

    /// the newest audit entries, newest first
    pub fn audit_tail(&self, n: usize) -> Vec<AuditEntry> {
        let mut v: Vec<AuditEntry> = self.journal.tail("audit", n).iter().filter_map(AuditEntry::from_json).collect();
        v.reverse();
        v
    }

    /// a line of a session's log
    pub fn log(&self, session: &str, level: LogLevel, msg: &str) {
        let at = now();
        let l = LogLine { at, level, msg: String::from(msg) };
        let _ = self.journal.append(&format!("s-{}", session), month(at), &l.to_json());
    }

    pub fn session_log(&self, session: &str) -> Vec<LogLine> {
        self.journal.tail(&format!("s-{}", session), 20_000).iter().filter_map(LogLine::from_json).collect()
    }

    /// a record's raw value
    pub fn get(&self, coll: &str, id: &str) -> Option<Value> {
        if id.is_empty() {
            return None;
        }
        self.db.get(coll, id)
    }
}
