// model -- BackupCraft's data model as it travels between server, console, agent and bcctl.
// Times are seconds since the epoch (UTC; 0 = never / unknown), sizes bytes, ratios x 100
// (250 = 2.5 : 1).  Ids are UUID text.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{record, wire_enum};

wire_enum! {
    /// a user's role; each one includes the rights of the ones before it
    pub enum Role { Viewer = "viewer", Restore = "restore", Operator = "operator", Admin = "admin" }
}

impl Role {
    /// the rank (Viewer 1 .. Admin 4)
    pub fn rank(&self) -> u8 {
        match self {
            Role::Viewer => 1,
            Role::Restore => 2,
            Role::Operator => 3,
            Role::Admin => 4,
        }
    }
    /// whether this role includes `need`
    pub fn allows(&self, need: Role) -> bool {
        self.rank() >= need.rank()
    }
}

wire_enum! {
    pub enum Os { Unknown = "unknown", Linux = "linux", Windows = "windows" }
}

wire_enum! {
    /// the backup server itself or an agent on another machine
    pub enum HostKind { Server = "server", Agent = "agent" }
}

wire_enum! {
    pub enum HostStatus { Unknown = "unknown", Online = "online", Offline = "offline", Pairing = "pairing", Error = "error" }
}

record! {
    /// a machine the server backs up: itself or a paired agent
    pub struct Host {
        pub id: String,
        pub name: String,
        pub address: String,
        pub os: Os,
        pub platform: String,
        pub kind: HostKind,
        pub status: HostStatus,
        pub version: String,
        pub last_seen: i64,
        /// SHA-256 of the agent's certificate (hex)
        pub fingerprint: String,
        /// folders the agent lets the server read (backup sources)
        pub roots: Vec<String>,
        /// folders the agent lets the server write (restore targets)
        pub restore_roots: Vec<String>,
        pub message: String,
        pub created: i64,
    }
}

wire_enum! {
    pub enum RepoStatus { Ok = "ok", Warning = "warning", Offline = "offline", Locked = "locked", Error = "error" }
}

record! {
    /// a chunk store on the server's disks (or a share mounted there)
    pub struct Repository {
        pub id: String,
        pub name: String,
        pub path: String,
        pub status: RepoStatus,
        pub capacity: u64,
        pub used: u64,
        pub free: u64,
        /// bytes of the restore points before dedupe and compression
        pub logical: u64,
        pub points: u32,
        pub dedup_x100: u32,
        pub encrypted: bool,
        /// restore points cannot be deleted before this many days
        pub immutable_days: u32,
        pub message: String,
        pub created: i64,
    }
}

impl Repository {
    /// the bytes in use on the repository's disk (its own and other data)
    pub fn disk_used(&self) -> u64 {
        self.capacity.saturating_sub(self.free)
    }
}

/// the disks under the repositories: (size, free bytes); repositories on one disk (the same
/// size and free bytes) count once
pub fn disks(repos: &[Repository]) -> (u64, u64) {
    let mut seen: Vec<(u64, u64)> = Vec::new();
    for r in repos {
        if r.capacity > 0 && !seen.contains(&(r.capacity, r.free)) {
            seen.push((r.capacity, r.free));
        }
    }
    (seen.iter().map(|d| d.0).sum(), seen.iter().map(|d| d.1).sum())
}

wire_enum! {
    pub enum JobKind { File = "file", Volume = "volume", HyperV = "hyperv", Qemu = "qemu", Copy = "copy", Tape = "tape" }
}

wire_enum! {
    /// how a job compresses its data: LZ4 (fast), zstd level 3 (balanced), zstd level 6 (high)
    pub enum Compression { Lz4 = "lz4", None = "none", Zstd = "zstd", ZstdHigh = "zstd-high" }
}

wire_enum! {
    pub enum SessionStatus { Never = "never", Running = "running", Success = "success", Warning = "warning",
                             Failed = "failed", Cancelled = "cancelled" }
}

record! {
    /// when a job runs: a cron expression ("" = manually only) in an IANA time zone
    pub struct Schedule {
        pub cron: String,
        pub tz: String,
        pub enabled: bool,
    }
}

record! {
    /// which restore points stay: the newest `points`, all of the last `days` days, and the
    /// last of each week / month / year for so many weeks / months / years (GFS)
    pub struct Retention {
        pub points: u32,
        pub days: u32,
        pub weekly: u32,
        pub monthly: u32,
        pub yearly: u32,
    }
}

record! {
    /// a backup job
    pub struct Job {
        pub id: String,
        pub name: String,
        pub description: String,
        pub kind: JobKind,
        pub enabled: bool,
        pub host_id: String,
        pub sources: Vec<String>,
        pub excludes: Vec<String>,
        pub repository_id: String,
        pub schedule: Schedule,
        pub retention: Retention,
        pub encrypted: bool,
        pub key_hint: String,
        pub compression: Compression,
        /// read everything again every n days (0 = never; the chunk store still dedupes)
        pub active_full_days: u32,
        pub pre_script: String,
        pub post_script: String,
        /// copy and tape jobs: the job whose points are copied (written to tape)
        pub source_job: String,
        /// tape jobs: the media pool written to
        pub pool_id: String,
        /// continuous data protection: a point soon after changes, at most every cdp_rpo
        /// seconds; all CDP points of the last cdp_keep_hours hours stay
        pub cdp: bool,
        pub cdp_rpo: u32,
        pub cdp_keep_hours: u32,
        pub last_status: SessionStatus,
        pub last_run: i64,
        pub last_size: u64,
        pub next_run: i64,
        pub points: u32,
        pub created: i64,
        pub updated: i64,
    }
}

wire_enum! {
    pub enum SessionKind { Backup = "backup", Restore = "restore", Verify = "verify", Retention = "retention",
                           Copy = "copy", Maintenance = "maintenance", Replica = "replica", Failback = "failback",
                           Tape = "tape" }
}

record! {
    /// one run of a job (or of a restore, verify, maintenance task)
    pub struct Session {
        pub id: String,
        pub kind: SessionKind,
        pub job_id: String,
        pub job_name: String,
        pub status: SessionStatus,
        pub started: i64,
        pub finished: i64,
        pub files_total: u64,
        pub files_done: u64,
        pub bytes_total: u64,
        pub bytes_read: u64,
        /// bytes written to the repository (after dedupe and compression)
        pub bytes_new: u64,
        pub dedup_x100: u32,
        pub compress_x100: u32,
        pub warnings: u32,
        pub errors: u32,
        pub message: String,
        pub operation_id: String,
        pub point_id: String,
    }
}

record! {
    /// a restore point: a complete, signed snapshot of a job's sources
    pub struct RestorePoint {
        pub id: String,
        pub job_id: String,
        pub job_name: String,
        pub created: i64,
        /// all files read again (active full); otherwise unchanged files were taken over
        pub full: bool,
        pub files: u64,
        pub size: u64,
        pub stored_new: u64,
        pub immutable_until: i64,
        pub verified: i64,
        pub host: String,
        pub encrypted: bool,
        pub signed: bool,
        /// the repository that holds the point (it stays there when the job moves or goes)
        pub repository_id: String,
        /// a copy job's point: the point it was copied from
        pub copied_from: String,
        /// the creation in nanoseconds (orders points of the same second; 0: older points)
        pub created_ns: i64,
    }
}

impl RestorePoint {
    /// the key that orders points by age (newest = largest)
    pub fn order(&self) -> i64 {
        if self.created_ns > 0 { self.created_ns } else { self.created.saturating_mul(1_000_000_000) }
    }
}

wire_enum! {
    pub enum EntryKind { File = "file", Dir = "dir", Link = "link", Other = "other" }
}

record! {
    /// a directory entry of a host's file system or of a restore point
    pub struct Entry {
        pub name: String,
        pub kind: EntryKind,
        pub size: u64,
        pub mtime: i64,
    }
}

record! {
    /// a listing page: the entries of `path`, `more` when there are further pages
    pub struct Listing {
        pub path: String,
        pub entries: Vec<Entry>,
        pub more: bool,
        pub offset: u32,
    }
}

wire_enum! {
    pub enum OpStatus { Queued = "queued", Running = "running", Cancelling = "cancelling", Succeeded = "succeeded",
                        Failed = "failed", Cancelled = "cancelled", Interrupted = "interrupted" }
}

impl OpStatus {
    pub fn finished(&self) -> bool {
        matches!(self, OpStatus::Succeeded | OpStatus::Failed | OpStatus::Cancelled | OpStatus::Interrupted)
    }
}

record! {
    /// a long action the server runs in the background (202 + polling)
    pub struct Operation {
        pub id: String,
        pub kind: String,
        pub title: String,
        pub owner: String,
        pub status: OpStatus,
        pub done: u64,
        pub total: u64,
        pub created: i64,
        pub started: i64,
        pub finished: i64,
        pub result: String,
        pub error: String,
        pub session_id: String,
        pub target_id: String,
    }
}

record! {
    pub struct User {
        pub id: String,
        pub username: String,
        pub role: Role,
        pub created: i64,
        pub last_login: i64,
        pub disabled: bool,
    }
}

wire_enum! {
    pub enum CredKind { Password = "password", SshKey = "ssh-key" }
}

record! {
    /// stored credentials (the secret never leaves the server)
    pub struct Credential {
        pub id: String,
        pub label: String,
        pub username: String,
        pub kind: CredKind,
        pub created: i64,
    }
}

record! {
    pub struct AuditEntry {
        pub id: String,
        pub at: i64,
        pub actor: String,
        pub action: String,
        pub target: String,
        pub detail: String,
        pub ok: bool,
    }
}

wire_enum! {
    pub enum LogLevel { Info = "info", Warning = "warning", Error = "error", Debug = "debug" }
}

record! {
    pub struct LogLine {
        pub at: i64,
        pub level: LogLevel,
        pub msg: String,
    }
}

wire_enum! {
    pub enum Severity { Info = "info", Warning = "warning", Critical = "critical" }
}

record! {
    /// something that needs attention (dashboard)
    pub struct Alert {
        pub severity: Severity,
        pub title: String,
        pub detail: String,
        /// "job", "repository", "host", "session" and its id
        pub target_kind: String,
        pub target_id: String,
        pub at: i64,
    }
}

record! {
    pub struct DayStat {
        pub day: i64,
        pub ok: u32,
        pub warn: u32,
        pub fail: u32,
    }
}

record! {
    pub struct NextRun {
        pub job_id: String,
        pub job_name: String,
        pub at: i64,
    }
}

record! {
    /// the dashboard's figures
    pub struct Summary {
        pub server_name: String,
        pub version: String,
        pub jobs: u32,
        pub jobs_enabled: u32,
        pub hosts: u32,
        pub hosts_online: u32,
        pub protected_items: u32,
        pub points: u32,
        pub ok_24h: u32,
        pub warn_24h: u32,
        pub fail_24h: u32,
        pub running: u32,
        /// the bytes the repositories take
        pub storage_used: u64,
        /// the size of the disks under the repositories (each disk once, see disks)
        pub storage_capacity: u64,
        /// their free bytes (0 in the reports of older servers, which did not send it)
        pub storage_free: u64,
        pub storage_logical: u64,
        pub history: Vec<DayStat>,
        pub next_runs: Vec<NextRun>,
        pub alerts: Vec<Alert>,
        /// the protected bytes per day of the last 14 days (sparklines)
        pub protected_history: Vec<u64>,
    }
}

record! {
    /// a feature and whether this server offers it
    pub struct Capability {
        pub name: String,
        pub available: bool,
        pub reason: String,
    }
}

wire_enum! {
    /// where a replica stands: new (not synced yet), ready (a standby copy of `point_id`),
    /// syncing, failed (the last sync did not finish), failover (in use instead of the
    /// original: not synced any more), failback (its changes go back to the original)
    pub enum ReplicaState { New = "new", Ready = "ready", Syncing = "syncing", Failed = "failed", Failover = "failover", Failback = "failback" }
}

record! {
    /// a standby copy of a job's newest restore point in a folder of a host
    pub struct Replica {
        pub id: String,
        pub name: String,
        pub job_id: String,
        pub job_name: String,
        pub host_id: String,
        pub host_name: String,
        /// the folder the replica lives in (each root of the job below it)
        pub target: String,
        /// sync after every new restore point of the job
        pub auto: bool,
        pub state: ReplicaState,
        /// the restore point the replica shows, and its time
        pub point_id: String,
        pub point_time: i64,
        pub synced: i64,
        pub files: u64,
        pub bytes: u64,
        pub message: String,
        pub failover_at: i64,
        pub created: i64,
    }
}

record! {
    /// replicas to switch over in order, with a pause between them
    pub struct FailoverPlan {
        pub id: String,
        pub name: String,
        pub replicas: Vec<String>,
        pub delay_s: u32,
        pub last_run: i64,
        /// "", "success", "failed"
        pub last_status: String,
        pub message: String,
        pub created: i64,
    }
}

record! {
    /// a volume of a host (block-level backups)
    pub struct Volume {
        /// "C:" or a device path
        pub id: String,
        pub label: String,
        pub fs: String,
        pub size: u64,
        pub free: u64,
        pub kind: String,
        pub mount: String,
    }
}

record! {
    /// a disk of a VM
    pub struct VmDisk {
        /// the block node (QMP)
        pub node: String,
        pub file: String,
        /// "qcow2", "raw", ...
        pub format: String,
        pub size: u64,
        /// "virtio", "ide", "scsi"
        pub bus: String,
    }
}

record! {
    /// a QEMU/KVM VM of a hypervisor host, seen through its QMP socket
    pub struct Vm {
        /// host id + ":" + name
        pub id: String,
        pub name: String,
        pub uuid: String,
        pub host_id: String,
        pub host_name: String,
        /// "running", "paused", ... from QEMU; "stopped": its socket does not answer (the disks
        /// as last seen are backed up from their files)
        pub state: String,
        pub qmp: String,
        pub memory_mb: u64,
        pub cpus: u32,
        pub disks: Vec<VmDisk>,
        pub version: String,
        pub seen: i64,
    }
}

record! {
    /// a hypervisor host
    pub struct Hypervisor {
        pub host_id: String,
        pub host_name: String,
        /// "qemu"
        pub kind: String,
        pub version: String,
        /// hardware virtualization (/dev/kvm)
        pub kvm: bool,
        /// the folders whose QMP sockets are VMs
        pub folders: Vec<String>,
        pub vms: u32,
        pub running: u32,
        pub ok: bool,
        pub message: String,
    }
}

record! {
    /// a VM started from a restore point: an instant start or a boot check
    pub struct InstantSession {
        pub id: String,
        pub point_id: String,
        pub point_time: i64,
        pub job_id: String,
        pub job_name: String,
        pub vm: String,
        /// "instant", "bootcheck"
        pub purpose: String,
        /// "starting", "running", "migrating", "migrated", "stopped", "failed", "interrupted"
        pub state: String,
        pub memory_mb: u64,
        pub cpus: u32,
        /// "none", "isolated" (user-mode network without access outside)
        pub network: String,
        /// the folder the disks were moved to (migration)
        pub target: String,
        pub started: i64,
        pub stopped: i64,
        pub owner: String,
        pub message: String,
    }
}

record! {
    pub struct CheckResult {
        pub name: String,
        pub ok: bool,
        pub detail: String,
    }
}

record! {
    /// a boot check: a VM of a restore point started in isolation and checked
    pub struct BootCheckResult {
        pub id: String,
        pub point_id: String,
        pub point_time: i64,
        pub job_id: String,
        pub job_name: String,
        pub vm: String,
        pub started: i64,
        pub finished: i64,
        pub ok: bool,
        pub checks: Vec<CheckResult>,
        /// the end of the serial console output
        pub console: String,
        pub message: String,
    }
}

record! {
    /// a media pool: tapes with one purpose and one protection
    pub struct MediaPool {
        pub id: String,
        pub name: String,
        pub description: String,
        /// days a tape stays write-protected after its last write
        pub protect_days: u32,
        /// sets are encrypted (a key per set, sealed with the master key and to the recovery kit)
        pub encrypted: bool,
        /// sets are appended to a tape until it is full (false: each run starts a blank tape)
        pub append: bool,
        /// filled when listed: tapes, bytes written, capacity
        pub tapes: u32,
        pub used: u64,
        pub capacity: u64,
        pub created: i64,
    }
}

record! {
    /// a tape of the library (a virtual tape is a file there)
    pub struct Cartridge {
        pub id: String,
        pub barcode: String,
        /// "" = free (any pool takes it when it needs a blank tape)
        pub pool_id: String,
        pub pool_name: String,
        pub capacity: u64,
        pub used: u64,
        /// "blank", "active" (written, room left), "full", "missing" (not in the library)
        pub state: String,
        pub sets: u32,
        pub protected_until: i64,
        pub written: i64,
        pub created: i64,
        pub message: String,
    }
}

record! {
    /// a restore point written to tape (a set; it may go on over several tapes)
    pub struct TapeSet {
        pub id: String,
        pub pool_id: String,
        pub pool_name: String,
        pub job_id: String,
        pub job_name: String,
        pub point_id: String,
        pub point_time: i64,
        pub host_name: String,
        pub windows: bool,
        pub roots: Vec<String>,
        /// the barcodes in order and where the set starts on each
        pub tapes: Vec<String>,
        pub positions: Vec<u64>,
        pub files: u64,
        pub entries: u64,
        pub bytes: u64,
        pub encrypted: bool,
        pub written: i64,
        pub verified: i64,
        /// "ok", "incomplete" (a tape is missing or was erased), "locked" (encrypted, key unknown)
        pub status: String,
    }
}

record! {
    /// the tape library
    pub struct TapeStatus {
        /// the folder of the virtual tapes
        pub library: String,
        pub ok: bool,
        pub message: String,
        pub tapes: u32,
        pub blank: u32,
        pub missing: u32,
        pub used: u64,
        pub capacity: u64,
        pub sets: u32,
    }
}

record! {
    /// another BackupCraft server this one watches (a central server for several sites)
    pub struct FleetServer {
        pub id: String,
        pub name: String,
        /// "https://host:7642"
        pub url: String,
        /// SHA-256 of its CA (hex; "" for http on loopback)
        pub fingerprint: String,
        /// "unknown", "ok", "warning", "critical" (from its report), "unreachable",
        /// "unauthorized" (token refused), "untrusted" (another CA than the pinned one)
        pub status: String,
        pub message: String,
        pub checked: i64,
        /// the last answer with a report
        pub seen: i64,
        /// its summary at `seen`
        pub report: Summary,
        pub created: i64,
    }
}

record! {
    /// a token a central server uses to read this server's report (the database keeps only its
    /// digest)
    pub struct FleetToken {
        pub id: String,
        pub label: String,
        pub created: i64,
        pub created_by: String,
        pub last_used: i64,
    }
}

record! {
    /// one package of a release
    pub struct UpdateFile {
        /// "windows-x86_64", "linux-x86_64"
        pub platform: String,
        pub name: String,
        pub size: u64,
        /// SHA-256 (hex)
        pub sha256: String,
    }
}

record! {
    /// manifest.json of an update channel (signed: manifest.json.sig, Ed25519, base64)
    pub struct UpdateManifest {
        pub product: String,
        pub version: String,
        pub date: i64,
        pub notes: String,
        pub files: Vec<UpdateFile>,
    }
}

record! {
    /// what the server knows about updates
    pub struct UpdateStatus {
        pub current: String,
        pub channel: String,
        pub platform: String,
        /// the newest version of the channel ("" = not checked yet)
        pub latest: String,
        pub available: bool,
        pub notes: String,
        pub date: i64,
        pub checked: i64,
        /// the package for this platform, its size and SHA-256
        pub file: String,
        pub size: u64,
        pub sha256: String,
        /// where the checked package lies ("" = not downloaded)
        pub downloaded: String,
        pub error: String,
    }
}

record! {
    /// a job under continuous data protection and what its watcher sees
    pub struct CdpStatus {
        pub job_id: String,
        pub job_name: String,
        pub host_name: String,
        pub rpo: u32,
        pub keep_hours: u32,
        /// "watching", "backing-up", "error", "stopped"
        pub state: String,
        /// changed paths not in a point yet
        pub pending: u64,
        pub last_change: i64,
        pub last_point: i64,
        /// CDP points in the keep window
        pub points: u32,
        pub message: String,
    }
}

record! {
    /// the last delivery of notifications
    pub struct NotifyStatus {
        pub last_at: i64,
        pub last_ok: bool,
        pub last_message: String,
        /// "Test", "Sitzung", "Zusammenfassung"
        pub last_what: String,
        /// yyyymmdd of the last daily summary
        pub daily_sent: i64,
    }
}

record! {
    /// a short-lived code that pairs an agent with this server
    pub struct PairingCode {
        pub code: String,
        pub expires: i64,
        pub server_url: String,
        /// SHA-256 of the server CA (the agent shows it for checking)
        pub ca_fingerprint: String,
    }
}

record! {
    /// the server's state for clients that are not logged in
    pub struct ServerInfo {
        pub name: String,
        pub version: String,
        pub needs_setup: bool,
        pub time: i64,
        pub tls: bool,
        /// SHA-256 of the server's CA as "AB:CD:..." (to confirm it elsewhere)
        pub ca_fingerprint: String,
    }
}

record! {
    pub struct Setting {
        pub key: String,
        pub value: String,
    }
}

record! {
    /// a stored configuration backup (database and keys, encrypted with a passphrase)
    pub struct ConfigBackup {
        pub name: String,
        pub created: i64,
        pub size: u64,
        pub auto: bool,
    }
}

record! {
    /// whether a recovery kit exists, and when it was made
    pub struct RecoveryStatus {
        pub created: i64,
        pub fingerprint: String,
    }
}
