// shell -- Windows shell integration of the setup: known folders, .lnk shortcuts, the
// "installed apps" entry (...\Uninstall below HKCU, or HKLM for all users), starting a program
// and the uninstaller's self-delete.  The OS calls live in plat::inst (COM IShellLinkW,
// registry, CreateProcessW; ole32 / shell32 / advapi32 loaded with LoadLibraryA there).
//                                                              [apps/installer/src/shell.s]
//
//   Scope                               for the current user, or for all users (Machine: the
//                                       common desktop and start menu, HKLM; administrator
//                                       rights needed)
//   sh_folder(which, scope) -> Some(dir)   0 = desktop, 1 = start menu programs
//   sh_program_files() -> dir           the default folder of an installation for all users
//   sh_link(lnk, target, dir, desc, app_id) -> bool   create a shortcut file (app_id: its
//                                       System.AppUserModel.ID, "" = none)
//   sh_pins(exe, app_id) -> usize       the user's taskbar pins of exe get app_id (pinned before
//                                       the program ran under its id)
//   Reg::begin(app, scope) -> Some(Reg), sz(name, value), dword(name, value), end()
//                                       write the uninstall key of the app
//   sh_reg_delete(app, scope) -> bool   remove it again
//   sh_run(cmdline, dir, hidden) -> bool
//   sh_assoc_set(entries, scope) / sh_assoc_remove(list, scope)   the file types and URL schemes
//                                       (assoc.rs): registry values, then SHChangeNotify
//   sh_browser_request(pkg, headless) -> bool   (browser=1) Settings at the app's page of the
//                                       default apps (plat::assoc::browser_request)
//   sh_admin() -> bool                  this process has administrator rights
//   sh_elevate(exe, args) -> Ok(exit code) / Err   the setup again with administrator rights
//                                       (the UAC prompt), waiting for it
//
// Test mode: with the environment variable ASMINST_SHELL_DIR set, the desktop and the start
// menu are <dir>\Desktop and <dir>\Programs, the pins <dir>\Pinned, and registry values go to
// <dir>\registry.txt (the file types and schemes to <dir>\assoc.txt, "subkey|name=value"
// lines); for all users the same below <dir>\machine -- a test never touches the user's shell
// or registry; Settings never opens (the default browser's page is printed).  ASMINST_ADMIN=1 /
// 0 then stands for the administrator rights, and sh_elevate starts the setup without a UAC
// prompt (the same environment with ASMINST_ADMIN=1, waited for; ASMINST_UAC=no: as if the user
// refused it).

use alloc::format;
use alloc::string::String;

use crate::{PATH_CAP, str_cap};
use plat::inst::Root;

/// where an installation goes
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum Scope {
    /// the current user: %LOCALAPPDATA%\Programs, the user's desktop and start menu, HKCU
    #[default]
    User,
    /// all users: Program Files, the common desktop and start menu, HKLM
    Machine,
}

impl Scope {
    fn root(self) -> Root {
        match self {
            Scope::User => Root::User,
            Scope::Machine => Root::Machine,
        }
    }
}

/// ASMINST_SHELL_DIR when set (and not empty)
fn sh_testroot() -> Option<String> {
    plat::env("ASMINST_SHELL_DIR").filter(|d| !d.is_empty()).map(|d| str_cap(&d, PATH_CAP))
}

/// the test folder of a scope: ASMINST_SHELL_DIR, for all users its "machine" folder  [sh_testdir]
fn sh_testdir(scope: Scope) -> Option<String> {
    let t = sh_testroot()?;
    Some(match scope {
        Scope::User => t,
        Scope::Machine => {
            let d = str_cap(&(t + "\\machine"), PATH_CAP);
            plat::mkdir(&d);
            d
        }
    })
}

/// the setup runs in test mode (ASMINST_SHELL_DIR)
pub(crate) fn sh_test_mode() -> bool {
    sh_testroot().is_some()
}

/// 0 desktop, 1 start menu "Programs" (of the user or of all users) -> the folder (UTF-8) or
/// None  [sh_folder]
pub(crate) fn sh_folder(which: u32, scope: Scope) -> Option<String> {
    if let Some(t) = sh_testdir(scope) {
        let d = str_cap(&(t + if which == 0 { "\\Desktop" } else { "\\Programs" }), PATH_CAP);
        return if plat::mkdir(&d) { Some(d) } else { None };
    }
    plat::inst::known_folder(which + if scope == Scope::Machine { 2 } else { 0 })
}

/// the programs folder of all users ("C:\Program Files"; the environment's ProgramFiles when the
/// shell gives none)
pub(crate) fn sh_program_files() -> String {
    plat::inst::known_folder(4)
        .or_else(|| plat::env("ProgramFiles").filter(|v| !v.is_empty()))
        .map_or_else(|| String::from("C:\\Program Files"), |v| str_cap(&v, PATH_CAP))
}

/// a shortcut lnk -> target (working folder dir, icon 0 of the target, comment desc, the
/// AppUserModelID app_id)  [sh_link]
pub(crate) fn sh_link(lnk: &str, target: &str, dir: &str, desc: &str, app_id: &str) -> bool {
    plat::inst::shell_link_id(lnk, target, dir, desc, app_id)
}

/// the pins of the program exe without an id get app_id (test mode: <dir>\Pinned) -> how many
pub(crate) fn sh_pins(exe: &str, app_id: &str) -> usize {
    if app_id.is_empty() {
        return 0;
    }
    let dir = match sh_testdir(Scope::User) {
        Some(t) => t + "\\Pinned",
        None => match plat::inst::pinned_dir() {
            Some(d) => d,
            None => return 0,
        },
    };
    plat::inst::pins_set_app_id(&dir, exe, app_id)
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
    pub(crate) fn begin(app: &str, scope: Scope) -> Option<Reg> {
        if let Some(t) = sh_testdir(scope) {
            return Some(Reg::Test(str_cap(&(t + "\\registry.txt"), PATH_CAP), String::new()));
        }
        plat::inst::reg_create_in(scope.root(), &sh_regpath(app)).map(Reg::Key)
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
pub(crate) fn sh_reg_delete(app: &str, scope: Scope) -> bool {
    if let Some(t) = sh_testdir(scope) {
        return plat::delete(&str_cap(&(t + "\\registry.txt"), PATH_CAP));
    }
    plat::inst::reg_delete_in(scope.root(), &sh_regpath(app))
}

/// the command of a key that exists in the scope's registry (key\shell\open\command's default
/// value, "" without one) -> None when the key is missing.  Test mode: the lines "key|command"
/// of existing.txt in the scope's test folder stand for the keys there  [sh_key_command]
pub(crate) fn sh_key_command(key: &str, scope: Scope) -> Option<String> {
    if let Some(t) = sh_testdir(scope) {
        let text = plat::read_file(&str_cap(&(t + "\\existing.txt"), PATH_CAP))?;
        let text = String::from_utf8_lossy(&text);
        return text.lines().find_map(|l| l.split_once('|').filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, c)| String::from(c)));
    }
    if !plat::inst::reg_exists_in(scope.root(), key) {
        return None;
    }
    Some(plat::inst::reg_get_sz_in(scope.root(), &format!("{key}\\shell\\open\\command"), "").unwrap_or_default())
}

/// write the registry values of the file types and URL schemes (test mode: assoc.txt)
pub(crate) fn sh_assoc_set(entries: &[(String, String, String)], scope: Scope) {
    if entries.is_empty() {
        return;
    }
    if let Some(t) = sh_testdir(scope) {
        let mut text = String::new();
        for (k, n, v) in entries {
            text.push_str(&format!("{k}|{n}={v}\n"));
        }
        plat::write_file(&str_cap(&(t + "\\assoc.txt"), PATH_CAP), text.as_bytes());
        return;
    }
    for (k, n, v) in entries {
        if let Some(key) = plat::inst::reg_create_in(scope.root(), k) {
            key.set_sz(n, v);
        }
    }
    plat::inst::assoc_changed();
}

/// remove what sh_assoc_set added: "T:<subkey>" trees, "V:<subkey>|<name>" values (';'
/// separated; test mode: delete assoc.txt)
pub(crate) fn sh_assoc_remove(list: &str, scope: Scope) {
    if list.is_empty() {
        return;
    }
    if let Some(t) = sh_testdir(scope) {
        plat::delete(&str_cap(&(t + "\\assoc.txt"), PATH_CAP));
        return;
    }
    for item in list.split(';') {
        if let Some(k) = item.strip_prefix("T:") {
            plat::inst::reg_delete_tree_in(scope.root(), k);
        } else if let Some((k, n)) = item.strip_prefix("V:").and_then(|v| v.split_once('|')) {
            plat::inst::reg_delete_value_in(scope.root(), k, n);
        }
    }
    plat::inst::assoc_changed();
}

/// (browser=1) Settings at the app's page of the default apps, the user picks there
/// (plat::assoc::browser_request: the app must be registered, which the setup just did;
/// jfgui_OPEN_DRYRUN=1 prints the page).  Test mode and script runs (headless) never open it:
/// they print "setup_default_browser: <page>" -> opened / printed
pub(crate) fn sh_browser_request(pkg: &plat::assoc::Package, headless: bool) -> bool {
    if sh_test_mode() || headless {
        let mut app = String::new();
        for &c in pkg.app.as_bytes() {
            if c.is_ascii_alphanumeric() || b"-._~".contains(&c) {
                app.push(c as char);
            } else {
                app.push_str(&format!("%{c:02X}"));
            }
        }
        let line = format!("setup_default_browser: ms-settings:defaultapps?registeredAppUser={app}\n");
        plat::write_stdout(line.as_bytes());
        return true;
    }
    plat::assoc::browser_request(pkg).is_ok()
}

/// start a process (cmdline and dir UTF-8; dir None = the current one), hidden =
/// CREATE_NO_WINDOW  [sh_run]
pub(crate) fn sh_run(cmdline: &str, dir: Option<&str>, hidden: bool) -> bool {
    plat::inst::run_cmdline(cmdline, dir, hidden)
}

/// this process may write what an installation for all users writes (test mode: ASMINST_ADMIN=1)
pub(crate) fn sh_admin() -> bool {
    if sh_test_mode() {
        return plat::env("ASMINST_ADMIN").is_some_and(|v| v == "1");
    }
    plat::process::elevated()
}

/// the setup exe once more with administrator rights (the UAC prompt; hidden: the elevated run
/// is quiet), waited for -> its exit code; Err when the user refused or it could not start.
/// Test mode: started as it is, with this process's environment and ASMINST_ADMIN=1
pub(crate) fn sh_elevate(exe: &str, args: &[&str]) -> Result<i32, ()> {
    if !sh_test_mode() {
        return plat::process::run_elevated(exe, args).map_err(|_| ());
    }
    if plat::env("ASMINST_UAC").is_some_and(|v| v == "no") {
        return Err(()); // (the test of a refused prompt)
    }
    // (no process::Command: its job would end the uninstaller's cleanup with the child, which
    // an elevated start never does)
    let mut line = quote_arg(exe);
    for a in args {
        line.push(' ');
        line.push_str(&quote_arg(a));
    }
    let before = plat::env("ASMINST_ADMIN");
    plat::setenv("ASMINST_ADMIN", Some("1"));
    let code = plat::inst::run_cmdline_wait(&line, None, true);
    plat::setenv("ASMINST_ADMIN", before.as_deref());
    code.map(|c| c as i32).ok_or(())
}

/// an argument quoted for CommandLineToArgvW: in quotes, a quote and the backslashes before it
/// (or before the closing quote) escaped
fn quote_arg(s: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in s.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        let n = if c == '"' { slashes * 2 + 1 } else { slashes };
        out.extend(core::iter::repeat_n('\\', n));
        slashes = 0;
        out.push(c);
    }
    out.extend(core::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}
