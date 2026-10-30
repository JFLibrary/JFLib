// recent -- the app's list of recent files ("Zuletzt geöffnet") and the system's (plat::recent)
//
//   recent_add(path)         a file the user opened or saved: first in the app's list (no
//                            duplicates, at most RECENT_KEEP) and in the system's list
//                            (plat::recent_add: Windows' recent files and jump list,
//                            recently-used.xbel; needs the file readable under the app's
//                            permissions)
//   recent_files() -> [path] the app's list, newest first (a file may be gone meanwhile: the
//                            app tells the user and calls recent_remove)
//   recent_open(path) -> RecentOpen   a file of the list about to be opened (call it from the
//                            user's click or key): Ready (read it now), Asking (the browser asks
//                            for the permission again: App::command(UIC_RECENT, 1 readable /
//                            0 refused or gone) follows, recent_reopened() names the file) or
//                            Missing
//   recent_reopened() -> Option<path>   the file UIC_RECENT answered for (once)
//   recent_remove(path)      out of both lists
//   recent_clear()           both lists emptied (the system's: this app's entries)
//
// The app's list is config://recent.list (one path per line, newest first), read on first use
// and kept in memory.  OS paths are kept absolute ("C:/x/a.png", "/home/u/a.png"; a folder
// mount's file as its OS path), so another spelling of the same file is no new entry, and as
// plat::persistent_name gives them (a Flatpak's file from the documents portal as its lasting
// document: one of the portal's file dialog opens again after a restart without a new
// dialog).  Headless runs (--shot, --script, agents) never touch the file and leave the
// system's list alone (the list lives in memory there; jfgui_RECENT redirects the system's
// list, plat/recent.rs).
//
// The browser: the list is persisted in the origin's storage with every change (an emptied
// list as an empty file: the storage keeps no deletions), so a reload starts with it.  A file
// of the user's own (/handles/ID/NAME, Chromium's pickers) is read in at the start while the
// browser still grants it; else recent_open asks through plat::web::reopen (the browser wants
// the user's gesture).  Firefox and Safari have no such handles: the list stays, its files open
// when the app kept them in the storage itself (plat::web::persist).
// Android: the list is a file of the app's internal storage; the picker's grants are lasting
// (takePersistableUriPermission), and a single document's grant is given up when its entry
// leaves the list (pushed out, removed, cleared: Android keeps at most 128 / 512 of them); a
// folder's grant stays, the app may use the folder for more.

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

#[cfg(target_arch = "wasm32")]
use super::UIC_RECENT;
use super::Ui;

/// how many files the app's list keeps
pub const RECENT_KEEP: usize = 20;
const FILE: &str = "config://recent.list";

static LIST: SpinLock<Option<Vec<String>>> = SpinLock::new(None);

/// the form a path is kept in: absolute OS path when it is one, else as given
fn canon(path: &str) -> String {
    plat::vfs::os_path(path).and_then(|p| plat::policy::normalize(&p)).unwrap_or_else(|| String::from(path))
}

/// the form a path is added in: canon, as a name that lasts across restarts
fn lasting(path: &str) -> String {
    plat::persistent_name(&canon(path))
}

/// what recent_open found for a file of the list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentOpen {
    /// the file can be read now
    Ready,
    /// the browser asks the user for the permission again (a file of the user's own after a
    /// reload): App::command(UIC_RECENT, 1 readable now / 0 refused or gone) follows
    Asking,
    /// gone or out of reach: the app tells the user and may call recent_remove
    Missing,
}

/// the reopen that waits for the browser's answer, the file UIC_RECENT answered for
#[derive(Default)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct S {
    asking: Option<(u32, String)>,
    answered: Option<String>,
}

/// Android: the entries that left the list give up their lasting grant (a single document the
/// picker handed out; a folder's stays)
#[cfg(target_os = "android")]
fn release_dropped(old: &[String], new: &[String]) {
    for p in old.iter().filter(|p| !new.contains(p)) {
        if p.starts_with("content://") && p.contains("/document/") && !p.contains("/tree/") {
            let _ = plat::web::content_release(p);
        }
    }
}

impl Ui {
    /// the app's list (read from config:// on first use, not in headless runs)
    fn recent_with<R>(&self, f: impl FnOnce(&mut Vec<String>) -> R) -> R {
        let mut g = LIST.lock();
        let list = g.get_or_insert_with(|| {
            if self.headless {
                return Vec::new();
            }
            let text = plat::read_file(FILE).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
            text.lines().map(str::trim).filter(|l| !l.is_empty()).take(RECENT_KEEP).map(String::from).collect()
        });
        f(list)
    }

    fn recent_save(&self, list: &[String]) {
        if self.headless {
            return;
        }
        let mut text = String::new();
        for p in list {
            text.push_str(p);
            text.push('\n');
        }
        // the browser's storage keeps no deletions: an empty list is an empty file there
        if list.is_empty() && !cfg!(target_arch = "wasm32") {
            plat::delete(FILE);
        } else if let Some(d) = plat::vfs::app_dir(plat::vfs::APP_CONFIG) {
            if plat::mkdir(&d) && plat::write_file(FILE, text.as_bytes()) {
                // the browser keeps it in its storage (a reload starts with it)
                #[cfg(target_arch = "wasm32")]
                if let Some(os) = plat::vfs::os_path(FILE) {
                    let _ = plat::web::persist(&os);
                }
            }
        }
    }

    /// a file the user opened or saved: first in the app's recent files and in the system's
    pub fn recent_add(&mut self, path: &str) {
        let p = lasting(path);
        if p.is_empty() || p.contains(['\n', '\r']) {
            return;
        }
        let (old, list) = self.recent_with(|l| {
            let old = l.clone();
            l.retain(|x| *x != p);
            l.insert(0, p.clone());
            l.truncate(RECENT_KEEP);
            (old, l.clone())
        });
        self.recent_save(&list);
        self.recent_dropped(&old, &list);
        plat::recent_add(&p);
    }

    // the entries that left the list (Android: their grants)
    fn recent_dropped(&self, old: &[String], new: &[String]) {
        #[cfg(target_os = "android")]
        if !self.headless {
            release_dropped(old, new);
        }
        let _ = (old, new);
    }

    /// a file of the recent list about to be opened (from the user's click or key): Ready,
    /// Asking (the browser asks for the permission; UIC_RECENT follows) or Missing
    pub fn recent_open(&mut self, path: &str) -> RecentOpen {
        if plat::file_exists(path) == 1 {
            return RecentOpen::Ready;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(id) = plat::web::reopen(path) {
            if let Some((old, _)) = self.m.recent.asking.replace((id, String::from(path))) {
                plat::web::forget_request(old);
            }
            return RecentOpen::Asking;
        }
        RecentOpen::Missing
    }

    /// the file UIC_RECENT answered for, once
    pub fn recent_reopened(&mut self) -> Option<String> {
        self.m.recent.answered.take()
    }

    /// (E_WAKE) the browser answered a reopen: App::command(UIC_RECENT, 1 readable / 0 not)
    pub(crate) fn recent_poll(&mut self) {
        #[cfg(target_arch = "wasm32")]
        if let Some((id, path)) = self.m.recent.asking.clone() {
            let st = plat::web::request_status(id);
            if st == Some(plat::web::RequestStatus::Pending) {
                return;
            }
            plat::web::forget_request(id);
            self.m.recent.asking = None;
            let ok = st == Some(plat::web::RequestStatus::Completed) && plat::file_exists(&path) == 1;
            self.m.recent.answered = Some(path);
            self.cmd_run(UIC_RECENT, ok as u32);
        }
    }

    /// the app's recent files, newest first
    pub fn recent_files(&self) -> Vec<String> {
        self.recent_with(|l| l.clone())
    }

    /// a file out of the app's and the system's recent files
    pub fn recent_remove(&mut self, path: &str) {
        let p = canon(path);
        let (old, list) = self.recent_with(|l| {
            let old = l.clone();
            l.retain(|x| *x != p);
            (old, l.clone())
        });
        self.recent_save(&list);
        self.recent_dropped(&old, &list);
        plat::recent_remove(&p);
    }

    /// the app's recent files and its entries in the system's list forgotten
    pub fn recent_clear(&mut self) {
        let old = self.recent_with(core::mem::take);
        self.recent_save(&[]);
        self.recent_dropped(&old, &[]);
        plat::recent_clear();
    }
}
