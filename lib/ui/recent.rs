// recent -- the app's list of recent files ("Zuletzt geöffnet") and the system's (plat::recent)
//
//   recent_add(path)         a file the user opened or saved: first in the app's list (no
//                            duplicates, at most RECENT_KEEP) and in the system's list
//                            (plat::recent_add: Windows' recent files and jump list,
//                            recently-used.xbel; needs the file readable under the app's
//                            permissions)
//   recent_files() -> [path] the app's list, newest first (a file may be gone meanwhile: the
//                            app tells the user and calls recent_remove)
//   recent_remove(path)      out of both lists
//   recent_clear()           both lists emptied (the system's: this app's entries)
//
// The app's list is config://recent.list (one path per line, newest first), read on first use
// and kept in memory.  OS paths are kept absolute ("C:/x/a.png", "/home/u/a.png"; a folder
// mount's file as its OS path), so another spelling of the same file is no new entry.  Headless
// runs (--shot, --script, agents) never touch the file and leave the system's list alone (the
// list lives in memory there; jfgui_RECENT redirects the system's list, plat/recent.rs).

use alloc::string::String;
use alloc::vec::Vec;

use rt::sync::SpinLock;

use super::Ui;

/// how many files the app's list keeps
pub const RECENT_KEEP: usize = 20;
const FILE: &str = "config://recent.list";

static LIST: SpinLock<Option<Vec<String>>> = SpinLock::new(None);

/// the form a path is kept in: absolute OS path when it is one, else as given
fn canon(path: &str) -> String {
    plat::vfs::os_path(path).and_then(|p| plat::policy::normalize(&p)).unwrap_or_else(|| String::from(path))
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
        if list.is_empty() {
            plat::delete(FILE);
        } else if let Some(d) = plat::vfs::app_dir(plat::vfs::APP_CONFIG) {
            if plat::mkdir(&d) {
                plat::write_file(FILE, text.as_bytes());
            }
        }
    }

    /// a file the user opened or saved: first in the app's recent files and in the system's
    pub fn recent_add(&mut self, path: &str) {
        let p = canon(path);
        if p.is_empty() || p.contains(['\n', '\r']) {
            return;
        }
        let list = self.recent_with(|l| {
            l.retain(|x| *x != p);
            l.insert(0, p.clone());
            l.truncate(RECENT_KEEP);
            l.clone()
        });
        self.recent_save(&list);
        plat::recent_add(&p);
    }

    /// the app's recent files, newest first
    pub fn recent_files(&self) -> Vec<String> {
        self.recent_with(|l| l.clone())
    }

    /// a file out of the app's and the system's recent files
    pub fn recent_remove(&mut self, path: &str) {
        let p = canon(path);
        let list = self.recent_with(|l| {
            l.retain(|x| *x != p);
            l.clone()
        });
        self.recent_save(&list);
        plat::recent_remove(&p);
    }

    /// the app's recent files and its entries in the system's list forgotten
    pub fn recent_clear(&mut self) {
        self.recent_with(|l| l.clear());
        self.recent_save(&[]);
        plat::recent_clear();
    }
}
