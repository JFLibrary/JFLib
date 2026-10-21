// shell -- Windows shell integration of the setup: known folders, .lnk shortcuts, the
// "installed apps" entry (HKCU ...\Uninstall), starting a program and the uninstaller's
// self-delete.  The OS calls live in plat::inst (COM IShellLinkW, registry, CreateProcessW;
// ole32 / shell32 / advapi32 loaded with LoadLibraryA there).    [apps/installer/src/shell.s]
//
//   sh_folder(which) -> Some(dir)       0 = desktop, 1 = start menu programs
//   sh_link(lnk, target, dir, desc) -> bool   create a shortcut file
//   Reg::begin(app) -> Some(Reg), sz(name, value), dword(name, value), end()
//                                       write the uninstall key of the app
//   sh_reg_delete(app) -> bool          remove it again
//   sh_run(cmdline, dir, hidden) -> bool
//   sh_assoc_set(entries) / sh_assoc_remove(list)   the file types and URL schemes (assoc.rs):
//                                       registry values under HKCU, then SHChangeNotify
//
// Test mode: with the environment variable ASMINST_SHELL_DIR set, the desktop and the start
// menu are <dir>\Desktop and <dir>\Programs and registry values go to <dir>\registry.txt
// (the file types and schemes to <dir>\assoc.txt, "subkey|name=value" lines) -- a test never
// touches the user's shell or registry.

use alloc::format;
use alloc::string::String;

use crate::{PATH_CAP, str_cap};

/// ASMINST_SHELL_DIR when set (and not empty)  [sh_testdir]
fn sh_testdir() -> Option<String> {
    plat::env("ASMINST_SHELL_DIR").filter(|d| !d.is_empty()).map(|d| str_cap(&d, PATH_CAP))
}

/// 0 desktop, 1 start menu "Programs" -> the folder (UTF-8) or None  [sh_folder]
pub(crate) fn sh_folder(which: u32) -> Option<String> {
    if let Some(t) = sh_testdir() {
        let d = str_cap(&(t + if which == 0 { "\\Desktop" } else { "\\Programs" }), PATH_CAP);
        return if plat::mkdir(&d) { Some(d) } else { None };
    }
    plat::inst::known_folder(which)
}

/// a shortcut lnk -> target (working folder dir, icon 0 of the target, comment desc)  [sh_link]
pub(crate) fn sh_link(lnk: &str, target: &str, dir: &str, desc: &str) -> bool {
    plat::inst::shell_link(lnk, target, dir, desc)
}

/// w_key = "Software\Microsoft\Windows\CurrentVersion\Uninstall\" + app  [sh_regpath]
fn sh_regpath(app: &str) -> String {
    str_cap(&(String::from("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\") + app), PATH_CAP)
}

/// the open uninstall key, or the registry text of the test mode
pub(crate) enum Reg {
    Key(plat::inst::RegKey),
    /// test mode: registry.txt and its text ("name=value\n" lines, 4096 bytes)
    Test(String, String),
}

const REGBUF_CAP: usize = 4096;

impl Reg {
    /// open (create) the uninstall key; test mode: start the text  [sh_reg_begin]
    pub(crate) fn begin(app: &str) -> Option<Reg> {
        if let Some(t) = sh_testdir() {
            return Some(Reg::Test(str_cap(&(t + "\\registry.txt"), PATH_CAP), String::new()));
        }
        plat::inst::reg_create(&sh_regpath(app)).map(Reg::Key)
    }

    /// a REG_SZ value  [sh_reg_sz]
    pub(crate) fn sz(&mut self, name: &str, value: &str) {
        match self {
            Reg::Key(k) => k.set_sz(name, value),
            Reg::Test(_, buf) => txt_line(buf, name, value),
        }
    }

    /// a REG_DWORD value  [sh_reg_dword]
    pub(crate) fn dword(&mut self, name: &str, value: u32) {
        match self {
            Reg::Key(k) => k.set_dword(name, value),
            Reg::Test(_, buf) => txt_line(buf, name, &format!("{value}")),
        }
    }

    /// close the key (test mode: write registry.txt)  [sh_reg_end]
    pub(crate) fn end(self) {
        if let Reg::Test(path, buf) = self {
            plat::write_file(&path, buf.as_bytes());
        }
    }
}

/// test mode: append "name=value\n" to the registry text  [sh_txt_line]
fn txt_line(buf: &mut String, name: &str, value: &str) {
    for s in [name, "=", value, "\n"] {
        let t = str_cap(&(core::mem::take(buf) + s), REGBUF_CAP);
        *buf = t;
    }
}

/// remove the uninstall key (test mode: delete registry.txt)  [sh_reg_delete]
pub(crate) fn sh_reg_delete(app: &str) -> bool {
    if let Some(t) = sh_testdir() {
        return plat::delete(&str_cap(&(t + "\\registry.txt"), PATH_CAP));
    }
    plat::inst::reg_delete(&sh_regpath(app))
}

/// write the registry values of the file types and URL schemes (test mode: assoc.txt)
pub(crate) fn sh_assoc_set(entries: &[(String, String, String)]) {
    if entries.is_empty() {
        return;
    }
    if let Some(t) = sh_testdir() {
        let mut text = String::new();
        for (k, n, v) in entries {
            text.push_str(&format!("{k}|{n}={v}\n"));
        }
        plat::write_file(&str_cap(&(t + "\\assoc.txt"), PATH_CAP), text.as_bytes());
        return;
    }
    for (k, n, v) in entries {
        if let Some(key) = plat::inst::reg_create(k) {
            key.set_sz(n, v);
        }
    }
    plat::inst::assoc_changed();
}

/// remove what sh_assoc_set added: "T:<subkey>" trees, "V:<subkey>|<name>" values (';'
/// separated; test mode: delete assoc.txt)
pub(crate) fn sh_assoc_remove(list: &str) {
    if list.is_empty() {
        return;
    }
    if let Some(t) = sh_testdir() {
        plat::delete(&str_cap(&(t + "\\assoc.txt"), PATH_CAP));
        return;
    }
    for item in list.split(';') {
        if let Some(k) = item.strip_prefix("T:") {
            plat::inst::reg_delete_tree(k);
        } else if let Some((k, n)) = item.strip_prefix("V:").and_then(|v| v.split_once('|')) {
            plat::inst::reg_delete_value(k, n);
        }
    }
    plat::inst::assoc_changed();
}

/// start a process (cmdline and dir UTF-8; dir None = the current one), hidden =
/// CREATE_NO_WINDOW  [sh_run]
pub(crate) fn sh_run(cmdline: &str, dir: Option<&str>, hidden: bool) -> bool {
    plat::inst::run_cmdline(cmdline, dir, hidden)
}
