// update -- signed updates of an app: check, download, put in place at the next start, roll
// back a version that does not start
//
//   Config::new(app, version, url, keys)   the app's id and running version, the address of
//                                its update.json and the trusted Ed25519 public keys (hex);
//                                then dir (default data://update), exe, target, attempts,
//                                restart, max_size, ca_pem, timeout_ms
//   startup(&cfg) -> Startup     first thing in main (before ui::launch): puts a downloaded
//                                update in place and restarts into it, counts the starts of a
//                                new version, rolls back one that never reported healthy
//   healthy(&cfg) -> bool        the health marker: the new version works (after its first
//                                frame, for example); without it the next start rolls back
//   Updater::new(cfg)            check() / download() / apply() (blocking), start_check() /
//                                start_download() (a background thread, plat::wake() when
//                                done), status() (phase, progress, error), cancel(), staged()
//   state(&cfg), newer(a, b), target(), Manifest::parse, verify
//
// The channel: update.json (UTF-8 JSON) and update.json.sig beside it (the Base64 Ed25519
// signature over the exact bytes of update.json); tools/update_manifest.py writes both:
//   {"format": 1, "app": "Kritzel", "version": "1.2.0", "date": 1760000000,
//    "expires": 1762600000, "notes": "...",
//    "files": [{"target": "windows-x86_64", "kind": "exe", "url": "Kritzel-1.2.0.exe",
//               "size": 123456, "sha256": "<64 hex digits>"}, ...]}
// kind "exe" replaces the program file, "bundle" is a JFA1 bundle of files relative to the
// program's folder (the program and its modules), "package" an installer (.deb, setup) that
// is only downloaded and checked (Updater::staged).  A relative url is resolved against the
// address update.json came from; redirects are followed (at most 5).
//
// Checks: the signature against cfg.keys; "app" must be cfg.app; "expires" (unix seconds) not
// passed; a version older than one a signed manifest named before is refused (replay); only a
// newer version than cfg.version that was not rolled back is offered; the file's size and
// SHA-256 while it downloads, and all of it again before it is put in place.  Connections go
// through netio and plat::net, so the app's net.connect rules apply (a refused host:
// Error::Denied and PERMDENY on stderr); https checks the server against the system's CAs
// (plus cfg.ca_pem).  The signature protects the content, so http works too.
//
// cfg.dir holds state (stage, version, from, starts, bad, seen), lock, partial/<sha256> (an
// interrupted download, continued with a Range request), staged/ (update.json, its signature
// and the file) and backup/ (the replaced files of the last update).  Putting an update in
// place writes the program's folder: in an app with a PERMISSIONS manifest that needs
// fs.write $EXE/**; a folder of the system (/usr/lib, Program Files) needs a "package" update.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod apply;
mod fetch;
mod manifest;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};

use netio::Url;
use rt::sync::SpinLock;

pub use apply::{state, Stage, State};
pub use fetch::join;
pub use manifest::{newer, target, verify, Kind, Manifest, Package, MAX_MANIFEST, MAX_SIG};

/// what the updater needs to know about the app
#[derive(Clone, Debug)]
pub struct Config {
    /// the app's id as the manifest names it ("app")
    pub app: String,
    /// the running version
    pub version: String,
    /// the address of update.json (https://... or http://...)
    pub url: String,
    /// the trusted Ed25519 public keys (built into the app)
    pub keys: Vec<[u8; 32]>,
    /// the folder of the updater's files (default "data://update")
    pub dir: String,
    /// the program file an update replaces ("" = the running exe)
    pub exe: String,
    /// the system whose package is taken (default target())
    pub target: String,
    /// starts of a new version without healthy() before it is rolled back (default 1)
    pub attempts: u32,
    /// startup() and apply() restart the program after a change (default true; false: they
    /// return Startup::Applied / RollingBack and the caller restarts)
    pub restart: bool,
    /// the largest package accepted (default 1 GiB)
    pub max_size: u64,
    /// further CA certificates (PEM) for an https channel with its own CA
    pub ca_pem: String,
    /// the longest wait for the server (ms; default 60000)
    pub timeout_ms: u32,
}

impl Config {
    /// the app's id and version, the address of update.json and the trusted public keys (64
    /// hex digits each; others are left out, so a channel without a valid key fails with
    /// Error::Config) -- the other fields at their defaults
    pub fn new(app: &str, version: &str, url: &str, keys: &[&str]) -> Config {
        Config {
            app: String::from(app),
            version: String::from(version),
            url: String::from(url),
            keys: keys.iter().filter_map(|k| crypto::unhex(k.trim()).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())).collect(),
            dir: String::from("data://update"),
            exe: String::new(),
            target: String::from(target()),
            attempts: 1,
            restart: true,
            max_size: 1 << 30,
            ca_pem: String::new(),
            timeout_ms: 60_000,
        }
    }
}

/// why a step failed
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// no channel, no valid key or an invalid address
    Config(String),
    /// the app's PERMISSIONS do not allow the host (net.connect)
    Denied(String),
    /// connection, TLS or protocol failure
    Net(String),
    /// the server answered with another status than 200 / 206
    Http(u16),
    /// the signature of update.json matches no trusted key
    Signature,
    /// update.json is malformed or for another app
    Manifest(String),
    /// the manifest's "expires" has passed
    Expired,
    /// the manifest is older than one seen before (its version)
    Replay(String),
    /// the file has not the size of the manifest
    Size { expected: u64, got: u64 },
    /// the file's SHA-256 is not the manifest's
    Hash,
    /// the connection ended early; what arrived is kept and the next download continues it
    Incomplete { done: u64, total: u64 },
    /// stopped by cancel()
    Cancelled,
    /// another check, download or apply runs (this process or another one)
    Busy,
    /// nothing to download or to apply
    Nothing,
    /// a package that is not put in place by the updater
    Unsupported(String),
    /// files could not be read or written
    Io(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Config(s) => write!(f, "Update nicht eingerichtet: {}", s),
            Error::Denied(h) => write!(f, "Verbindung zu {} von den Berechtigungen der App (net.connect) nicht erlaubt", h),
            Error::Net(s) => write!(f, "Netzwerkfehler: {}", s),
            Error::Http(s) => write!(f, "Der Update-Server antwortet {}", s),
            Error::Signature => f.write_str("Die Signatur des Update-Manifests passt zu keinem vertrauenswürdigen Schlüssel"),
            Error::Manifest(s) => write!(f, "Ungültiges Update-Manifest: {}", s),
            Error::Expired => f.write_str("Das Update-Manifest ist abgelaufen"),
            Error::Replay(v) => write!(f, "Das Update-Manifest ist älter als ein bereits gesehenes (Version {})", v),
            Error::Size { expected, got } => write!(f, "Die Update-Datei hat {} statt {} Bytes", got, expected),
            Error::Hash => f.write_str("Die Prüfsumme der Update-Datei stimmt nicht (SHA-256)"),
            Error::Incomplete { done, total } => write!(f, "Download unterbrochen bei {} von {} Bytes", done, total),
            Error::Cancelled => f.write_str("Abgebrochen"),
            Error::Busy => f.write_str("Ein anderer Update-Vorgang läuft"),
            Error::Nothing => f.write_str("Kein Update vorhanden"),
            Error::Unsupported(s) => write!(f, "Nicht möglich: {}", s),
            Error::Io(s) => write!(f, "Dateifehler: {}", s),
        }
    }
}

/// a newer version the channel offers
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub notes: String,
    /// unix seconds (0: not given)
    pub date: i64,
    /// the package's size in bytes
    pub size: u64,
    pub kind: Kind,
    /// it is downloaded already (staged)
    pub staged: bool,
}

/// the answer of a check
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// no newer version (or only one that was rolled back)
    UpToDate,
    /// a newer version without a package for this system (its version)
    NoPackage(String),
    /// a newer version to download
    Available(Offer),
}

/// what startup() did
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Startup {
    /// nothing to do
    Normal,
    /// the staged version is in place; the program must restart into it (only with
    /// cfg.restart = false or when the restart failed)
    Applied(String),
    /// this is a new version on trial: call healthy() once the app works
    Trial(String),
    /// the new version never reported healthy and the old one is back; the program must
    /// restart (only with cfg.restart = false or when the restart failed)
    RollingBack(String),
    /// the old version runs again because this new one failed (reported once)
    RolledBack(String),
    /// putting the staged update in place failed (a damaged one is dropped)
    Failed(Error),
}

/// what an Updater is doing
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    /// the last check found nothing newer
    UpToDate,
    /// a newer version can be downloaded (Status::version)
    Available,
    Downloading,
    /// downloaded and checked: put in place at the next start or by apply()
    Ready,
    /// the last step failed (Status::error)
    Failed,
}

/// the state for a dialog or a status line
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    /// the offered or downloaded version
    pub version: String,
    pub notes: String,
    /// bytes downloaded and the file's size
    pub done: u64,
    pub total: u64,
    pub error: Option<Error>,
}

// what check() found: the signed manifest and the package of this system
#[derive(Clone)]
struct Found {
    manifest: Vec<u8>,
    sig: Vec<u8>,
    m: Manifest,
    pkg: Package,
    url: Url,
}

struct Shared {
    status: SpinLock<Status>,
    found: SpinLock<Option<Found>>,
    busy: AtomicBool,
    cancel: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    thread: SpinLock<Option<plat::Thread>>,
}

/// checks and downloads for one Config; clones share the same state
#[derive(Clone)]
pub struct Updater {
    cfg: Arc<Config>,
    sh: Arc<Shared>,
}

fn now_secs() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000_000)
}

// a file name of the package's address (its last segment, safe characters only)
fn file_name(url: &str) -> String {
    let last = url.split(['?', '#']).next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let s: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).take(100).collect();
    if s.is_empty() || s.starts_with('.') || s.starts_with("update.json") {
        String::from("payload")
    } else {
        s
    }
}

impl Updater {
    /// an updater for cfg; a downloaded update (stage Staged) starts it in Phase::Ready
    pub fn new(cfg: Config) -> Updater {
        let st = state(&cfg);
        let mut status = Status::default();
        if st.stage == Stage::Staged {
            status.phase = Phase::Ready;
            status.version = st.version;
        }
        Updater {
            cfg: Arc::new(cfg),
            sh: Arc::new(Shared {
                status: SpinLock::new(status),
                found: SpinLock::new(None),
                busy: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
                done: AtomicU64::new(0),
                total: AtomicU64::new(0),
                thread: SpinLock::new(None),
            }),
        }
    }

    /// the configuration it was made with
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// the current phase, progress and last error
    pub fn status(&self) -> Status {
        let mut s = self.sh.status.lock().clone();
        s.done = self.sh.done.load(SeqCst);
        s.total = self.sh.total.load(SeqCst);
        s
    }

    fn set(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.sh.status.lock());
    }

    /// stop a running download (it ends with Error::Cancelled; what arrived is kept)
    pub fn cancel(&self) {
        self.sh.cancel.store(true, SeqCst);
    }

    /// fetch update.json and its signature and check them (blocking)
    pub fn check(&self) -> Result<Check, Error> {
        self.set(|s| {
            s.phase = Phase::Checking;
            s.error = None;
        });
        let r = self.check_now();
        self.set(|s| match &r {
            Ok(Check::Available(o)) => {
                s.phase = if o.staged { Phase::Ready } else { Phase::Available };
                s.version = o.version.clone();
                s.notes = o.notes.clone();
            }
            Ok(_) => s.phase = Phase::UpToDate,
            Err(e) => {
                s.phase = Phase::Failed;
                s.error = Some(e.clone());
            }
        });
        r
    }

    fn check_now(&self) -> Result<Check, Error> {
        let cfg = &*self.cfg;
        if cfg.keys.is_empty() {
            return Err(Error::Config(String::from("kein gültiger öffentlicher Schlüssel")));
        }
        let url = Url::parse(cfg.url.trim()).ok_or_else(|| Error::Config(format!("ungültige Adresse „{}“", cfg.url)))?;
        let (at, body) = fetch::get(cfg, &url, MAX_MANIFEST)?;
        // update.json.sig beside the address update.json came from
        let (path, query) = at.target.split_once('?').map_or((at.target.as_str(), None), |(p, q)| (p, Some(q)));
        let sig_url = at.with_target(&match query {
            Some(q) => format!("{}.sig?{}", path, q),
            None => format!("{}.sig", path),
        });
        let (_, sig) = fetch::get(cfg, &sig_url, MAX_SIG)?;
        if !verify(&body, &sig, &cfg.keys) {
            return Err(Error::Signature);
        }
        let m = Manifest::parse(&body).map_err(Error::Manifest)?;
        if m.app != cfg.app {
            return Err(Error::Manifest(format!("für „{}“, nicht für „{}“", m.app, cfg.app)));
        }
        if m.expires != 0 && now_secs() > m.expires {
            return Err(Error::Expired);
        }
        let _lock = apply::lock(cfg).ok_or(Error::Busy)?;
        let mut st = state(cfg);
        if !st.seen.is_empty() && newer(&st.seen, &m.version) {
            return Err(Error::Replay(st.seen));
        }
        if st.seen != m.version {
            st.seen = m.version.clone();
            apply::save(cfg, &st)?;
        }
        if !newer(&m.version, &cfg.version) || st.bad.contains(&m.version) {
            return Ok(Check::UpToDate);
        }
        let Some(pkg) = m.package(&cfg.target).cloned() else {
            return Ok(Check::NoPackage(m.version));
        };
        if pkg.size > cfg.max_size {
            return Err(Error::Manifest(format!("die Datei ist mit {} Bytes zu groß", pkg.size)));
        }
        let file = join(&at, &pkg.url).ok_or_else(|| Error::Manifest(format!("ungültige Adresse „{}“", pkg.url)))?;
        let staged = st.stage == Stage::Staged && st.version == m.version && apply::load_staged(cfg, &st).is_ok();
        let offer = Offer { version: m.version.clone(), notes: m.notes.clone(), date: m.date, size: pkg.size, kind: pkg.kind, staged };
        *self.sh.found.lock() = Some(Found { manifest: body, sig, m, pkg, url: file });
        Ok(Check::Available(offer))
    }

    /// download the package the last check() offered, check it and stage it for the next
    /// start (blocking; progress in status()) -> its version
    pub fn download(&self) -> Result<String, Error> {
        self.sh.cancel.store(false, SeqCst);
        self.set(|s| {
            s.phase = Phase::Downloading;
            s.error = None;
        });
        let r = self.download_now();
        self.set(|s| match &r {
            Ok(v) => {
                s.phase = Phase::Ready;
                s.version = v.clone();
            }
            Err(e) => {
                s.phase = Phase::Failed;
                s.error = Some(e.clone());
            }
        });
        r
    }

    fn download_now(&self) -> Result<String, Error> {
        let cfg = &*self.cfg;
        let f = self.sh.found.lock().clone().ok_or(Error::Nothing)?;
        let _lock = apply::lock(cfg).ok_or(Error::Busy)?;
        let mut st = state(cfg);
        if st.stage == Stage::Staged && st.version == f.m.version && apply::load_staged(cfg, &st).is_ok() {
            return Ok(st.version);
        }
        let partial = apply::path(cfg, "partial");
        let hex = crypto::hex(&f.pkg.sha256);
        // only the file of this package continues; older parts go
        let mut old = Vec::new();
        plat::dir_list(&partial, &mut |n, _, _| {
            if n != hex {
                old.push(String::from(n));
            }
        });
        for n in old {
            plat::delete(&format!("{}/{}", partial, n));
        }
        if !plat::mkdir(&partial) {
            return Err(Error::Io(format!("{} kann nicht angelegt werden", partial)));
        }
        let part = format!("{}/{}", partial, hex);
        let p = fetch::Progress { done: &self.sh.done, total: &self.sh.total, cancel: &self.sh.cancel };
        fetch::download(cfg, &f.url, &part, f.pkg.size, &f.pkg.sha256, &p)?;
        // staged/: the manifest, its signature and the file
        let staged = apply::path(cfg, "staged");
        apply::remove_tree(&staged);
        let name = file_name(&f.pkg.url);
        let w = |n: &str, b: &[u8]| plat::write_file(&format!("{}/{}", staged, n), b);
        if !plat::mkdir(&staged) || !w("update.json", &f.manifest) || !w("update.json.sig", &f.sig)
            || !plat::rename(&part, &format!("{}/{}", staged, name))
        {
            return Err(Error::Io(format!("{} kann nicht geschrieben werden", staged)));
        }
        st.stage = Stage::Staged;
        st.version = f.m.version.clone();
        st.from = cfg.version.clone();
        st.starts = 0;
        st.kind = String::from(f.pkg.kind.name());
        st.file = name;
        apply::save(cfg, &st)?;
        Ok(f.m.version)
    }

    fn spawn(&self, job: fn(&Updater)) -> bool {
        if self.sh.busy.swap(true, SeqCst) {
            return false;
        }
        // the thread of the last step has finished its work: wait for its end
        let prev = self.sh.thread.lock().take();
        if let Some(t) = prev {
            t.join();
        }
        let me = self.clone();
        let t = plat::thread_spawn(0, Box::new(move || {
            job(&me);
            me.sh.busy.store(false, SeqCst);
            plat::wake();
        }));
        match t {
            Some(t) => {
                *self.sh.thread.lock() = Some(t);
                true
            }
            None => {
                self.sh.busy.store(false, SeqCst);
                false
            }
        }
    }

    /// check() on a background thread; plat::wake() (UIC_WAKE / E_WAKE) when it is done ->
    /// false while another background step runs
    pub fn start_check(&self) -> bool {
        self.spawn(|u| {
            let _ = u.check();
        })
    }

    /// download() on a background thread; plat::wake() when it is done -> false while another
    /// background step runs
    pub fn start_download(&self) -> bool {
        self.spawn(|u| {
            let _ = u.download();
        })
    }

    /// whether a background step runs
    pub fn busy(&self) -> bool {
        self.sh.busy.load(SeqCst)
    }

    /// the downloaded update: its version, kind and the OS path of its file (a "package" is
    /// installed from there by the app or the user)
    pub fn staged(&self) -> Option<(String, Kind, String)> {
        let st = state(&self.cfg);
        if st.stage != Stage::Staged {
            return None;
        }
        let p = apply::path(&self.cfg, &format!("staged/{}", st.file));
        let os = plat::vfs::os_path(&p)?;
        Some((st.version, Kind::parse(&st.kind)?, os))
    }

    /// put the downloaded update in place now and restart into it (with cfg.restart the
    /// process ends here)
    pub fn apply(&self) -> Result<Startup, Error> {
        let cfg = &*self.cfg;
        let lock = apply::lock(cfg).ok_or(Error::Busy)?;
        let mut st = state(cfg);
        if st.stage != Stage::Staged {
            return Err(Error::Nothing);
        }
        apply::apply_staged(cfg, &mut st)?;
        drop(lock);
        Ok(restart(cfg, Startup::Applied(st.version)))
    }
}

// restart the program (with cfg.restart): the process ends; otherwise, or when no process
// could be started, `then` is returned
fn restart(cfg: &Config, then: Startup) -> Startup {
    if cfg.restart {
        let args = plat::args();
        let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
        if plat::restart_self(&rest) == 0 {
            plat::exit(0);
        }
    }
    then
}

/// at the start of the program, before its UI: a staged update is put in place and the
/// program restarted into it; a new version on trial counts its start, and one that already
/// had cfg.attempts starts without healthy() is rolled back (the old version restarted).
/// Another process holding the updater's lock: Normal.
pub fn startup(cfg: &Config) -> Startup {
    // nothing was ever downloaded: no folder is made, no lock taken
    if plat::file_exists(&apply::path(cfg, "state")) != 1 {
        return Startup::Normal;
    }
    let Some(lock) = apply::lock(cfg) else { return Startup::Normal };
    apply::cleanup(cfg);
    let mut st = state(cfg);
    match st.stage {
        Stage::Staged if st.kind != Kind::Package.name() => match apply::apply_staged(cfg, &mut st) {
            Ok(()) => {
                drop(lock);
                restart(cfg, Startup::Applied(st.version))
            }
            Err(e) => Startup::Failed(e),
        },
        Stage::Trial if st.version == cfg.version => {
            st.starts += 1;
            if st.starts <= cfg.attempts.max(1) {
                return match apply::save(cfg, &st) {
                    Ok(()) => Startup::Trial(st.version),
                    Err(e) => Startup::Failed(e),
                };
            }
            let bad = st.version.clone();
            match apply::rollback(cfg, &mut st) {
                Ok(()) => {
                    drop(lock);
                    restart(cfg, Startup::RollingBack(bad))
                }
                Err(e) => Startup::Failed(e),
            }
        }
        Stage::RolledBack => {
            let bad = st.bad.last().cloned().unwrap_or_default();
            st.stage = Stage::None;
            let _ = apply::save(cfg, &st);
            Startup::RolledBack(bad)
        }
        _ => Startup::Normal,
    }
}

/// the health marker: the new version on trial works.  Call it once the app is up (after its
/// first frame, for example); true when it ended a trial.  Cheap when no trial runs.
pub fn healthy(cfg: &Config) -> bool {
    let st = state(cfg);
    if st.stage != Stage::Trial || st.version != cfg.version {
        return false;
    }
    let Some(_lock) = apply::lock(cfg) else { return false };
    let mut st = state(cfg);
    if st.stage != Stage::Trial || st.version != cfg.version {
        return false;
    }
    st.stage = Stage::Ok;
    st.starts = 0;
    apply::remove_tree(&apply::path(cfg, "staged"));
    apply::save(cfg, &st).is_ok()
}
