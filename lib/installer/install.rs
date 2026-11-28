// install -- the setup's work: install the chosen style variant and the further files,
// shortcuts, the uninstall entry and install.ini; uninstall it again
//                                                              [apps/installer/src/install.s]
//
//   inst_run(st) -> Ok / Err (st.err)   install into st.dir (style st.sel, options st.opt_*,
//                                       for st.scope: the user or all users)
//   inst_launch(st) -> bool             start the installed program
//   inst_default_browser(st, headless)  (browser=1) Settings at the program's page of the
//                                       default apps (test mode, script runs: printed only)
//   un_load(st) -> Ok / Err             uninstaller: read install.ini next to the own exe
//   un_run(st)                          remove files, shortcuts, uninstall entry, then let cmd
//                                       delete uninstall.exe and the folder once it has ended
//
// install.ini (key=value): app, display, style, desktop, start (shortcut paths or empty),
// reg (1 / 0), files (further files relative to the folder, ';'-separated, '\'; one line of any
// length, an older setup cut it at 4096 bytes), assoc (what
// the file types, URL schemes and a browser's registration added, assoc.rs), app_id (the program's AppUserModelID: the
// shortcuts, the pins and the ProgIDs carry it, and the program runs under it when it finds
// this line beside itself, lib/plat/assoc.rs app_id_boot), scope=machine for an installation
// for all users (its keys are below HKLM; no line: the user's, HKCU)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::i18n::{t, tf};

use crate::payload::{PK_FILE, STY_MAX, kv_get, parent_dir, path_cap, pl_inflate};
use crate::shell::{Reg, Scope, sh_assoc_remove, sh_assoc_set, sh_browser_request, sh_folder, sh_key_command, sh_link, sh_pins, sh_reg_delete, sh_run, sh_test_mode};
use crate::{Inst, PATH_CAP, str_cap};

/// install.ini's lists (files=, assoc=) have no limit: the asm's buffers of 4096 and 8192 bytes
/// cut a long files= (licences of many crates), and the uninstaller left the rest behind
const NO_CAP: usize = usize::MAX;

/// st.dir + "\" + name  [in_path]
fn in_path(st: &Inst, name: &str) -> String {
    path_cap(&format!("{}\\{}", st.dir, name))
}

/// st.err = the text msg (a catalogue key) in the language of the pages, its {} replaced by
/// arg (512 bytes)  [in_fail]
fn in_fail(st: &mut Inst, msg: &'static str, arg: &str) -> Result<(), ()> {
    st.err = str_cap(&tf(msg, &[arg]), 512);
    Err(())
}

/// '/' -> '\'  [in_slash]
fn in_slash(s: &str) -> String {
    s.replace('/', "\\")
}

/// create the folder that holds path  [in_mkparent]
fn in_mkparent(path: &str) {
    if let Some(d) = parent_dir(path) {
        plat::mkdir(&d);
    }
}

/// inflate payload entry i into a file  [in_write_entry]
fn in_write_entry(st: &Inst, i: usize, path: &str) -> bool {
    match pl_inflate(st, i) {
        Some(data) => plat::write_file(path, &data),
        None => false,
    }
}

/// append "key=value\r\n" to the install.ini text  [in_ini]
// (never cut to a fixed size, as the asm's 8192-byte buffer was: the lists of the files and of
// what the file types added stand last, and a list cut inside an entry hands the uninstaller
// another name -- "T:Software" for "T:Software\Classes\App.ext")
fn in_ini(txt: &mut String, key: &str, value: &str) {
    for s in [key, "=", value, "\r\n"] {
        txt.push_str(s);
    }
}

/// the style index installed: st.sel, the first when out of range
pub(crate) fn sel_index(st: &Inst) -> usize {
    let s = if (st.sel as u32 as usize) < st.sty.len() { st.sel as usize } else { 0 };
    s & (STY_MAX - 1)
}

/// the program's file name: "<app>.exe"
pub(crate) fn exe_name(st: &Inst) -> String {
    str_cap(&(str_cap(&st.app, 64) + ".exe"), 64)
}

const E_WRITE: &str = "„{}“ konnte nicht geschrieben werden.";

/// (all users) what may lie in the folder already: nothing, or what an earlier installation of
/// this app wrote there (install.ini names it).  Anything else may be a user's, put there to run
/// with the program (a DLL beside it) once the folder is the computer's: refused.  A folder that is
/// not there yet is fine
fn in_dir_clean(st: &Inst) -> bool {
    let Some(top) = plat::fs::read_dir(&st.dir) else { return true };
    if top.is_empty() {
        return true;
    }
    let Some(ini) = plat::read_file(&in_path(st, "install.ini")) else { return false };
    if kv_get(&ini, "app", 64).as_deref() != Some(st.app.as_str()) {
        return false;
    }
    let names = un_names(&kv_get(&ini, "files", NO_CAP).unwrap_or_default());
    let folders = un_folders(&names);
    let known = |rel: &str| {
        [exe_name(st).as_str(), "uninstall.exe", "install.ini"].iter().any(|n| rel.eq_ignore_ascii_case(n))
            || names.iter().chain(folders.iter()).any(|n| rel.eq_ignore_ascii_case(n))
    };
    // every entry, folders included, down to a depth no setup writes
    let mut todo: Vec<(String, u32)> = alloc::vec![(String::new(), 0)];
    while let Some((rel, depth)) = todo.pop() {
        let dir = if rel.is_empty() { st.dir.clone() } else { in_path(st, &rel) };
        let Some(list) = plat::fs::read_dir(&dir) else { return false };
        for e in list {
            let r = if rel.is_empty() { e.name.clone() } else { format!("{rel}\\{}", e.name) };
            if !known(&r) {
                return false;
            }
            if e.kind == plat::fs::FileKind::Dir {
                if depth >= 32 {
                    return false;
                }
                todo.push((r, depth + 1));
            }
        }
    }
    true
}

/// install into st.dir -> Ok / Err (st.err)  [inst_run]
pub(crate) fn inst_run(st: &mut Inst) -> Result<(), ()> {
    let mut initxt = String::new();
    let mut files = String::new();
    st.lnk_desk.clear();
    st.lnk_start.clear();
    st.dir = in_slash(&st.dir);
    // folder: for all users a new one (or the app's own), the administrators' with a DACL of its
    // own (test mode: the rights stay, the test runs without administrator rights)
    let machine = st.scope == Scope::Machine;
    if machine && !in_dir_clean(st) {
        let d = st.dir.clone();
        return in_fail(st, "Der Ordner „{}“ enthält schon andere Dateien. Wählen Sie für alle Benutzer einen neuen oder leeren Ordner.", &d);
    }
    let secure = machine && !sh_test_mode();
    let made = if secure {
        parent_dir(&st.dir).is_none_or(|p| plat::fs::read_dir(&p).is_some() || plat::mkdir(&p)) && plat::inst::secure_dir(&st.dir)
    } else {
        plat::mkdir(&st.dir)
    };
    if !made {
        let d = st.dir.clone();
        return in_fail(st, "Der Ordner „{}“ konnte nicht angelegt werden.", &d);
    }
    // the payload's folders the same way, the outermost first (an earlier installation's keep
    // what they had otherwise: SetFileSecurityW does not pass the folder's rights down)
    if secure {
        let names: Vec<String> = st.ents.iter().filter(|e| e.kind == PK_FILE).map(|e| in_slash(&e.name)).collect();
        for d in un_folders(&names).iter().rev() {
            let p = in_path(st, d);
            if !plat::inst::secure_dir(&p) {
                return in_fail(st, "Der Ordner „{}“ konnte nicht angelegt werden.", &p);
            }
        }
    }
    // the program in the chosen style
    let ent = st.sty[sel_index(st)].ent;
    st.exepath = in_path(st, &exe_name(st));
    if !in_write_entry(st, ent, &st.exepath) {
        let p = st.exepath.clone();
        return in_fail(st, "„{}“ konnte nicht geschrieben werden (läuft das Programm noch?).", &p);
    }
    // further files
    for i in 0..st.ents.len() {
        if st.ents[i].kind != PK_FILE {
            continue;
        }
        st.ents[i].name = in_slash(&st.ents[i].name);
        let path = in_path(st, &st.ents[i].name);
        in_mkparent(&path);
        if !in_write_entry(st, i, &path) {
            return in_fail(st, E_WRITE, &path);
        }
        // (the whole list: a name cut short is another file for the uninstaller, a name left
        // out a file it leaves behind)
        if !files.is_empty() {
            files.push(';');
        }
        files.push_str(&st.ents[i].name);
    }
    // the uninstaller: the setup without its payload (and without a signature of the setup)
    let un = in_path(st, "uninstall.exe");
    if !plat::write_file(&un, &crate::payload::stub(st)) {
        return in_fail(st, E_WRITE, &un);
    }
    // shortcuts: <folder>\<display>.lnk
    let lnk = str_cap(&(st.disp.clone() + ".lnk"), 128);
    if st.opt_desktop != 0 {
        st.lnk_desk = in_link(st, 0, &lnk);
    }
    if st.opt_start != 0 {
        st.lnk_start = in_link(st, 1, &lnk);
    }
    // the taskbar's pins of an earlier version: the program's new id (else two buttons); the
    // pins are the user's, an installation for all users had no earlier version without an id
    if st.scope == Scope::User {
        sh_pins(&st.exepath, &st.appid);
    }
    if st.opt_reg != 0 {
        in_regentry(st);
    }
    // file types and URL schemes (PACKAGE filetypes= / schemes=), a web browser (browser=1)
    let scope = st.scope;
    let plan = crate::assoc::plan(&package(st), &st.exepath, &|key: &str| sh_key_command(key, scope));
    for k in &plan.kept {
        st.log.line(&format!("kept: {k} belongs to another program"));
    }
    sh_assoc_set(&plan.set, st.scope);
    in_ini(&mut initxt, "app", &st.app);
    in_ini(&mut initxt, "display", &st.disp);
    let style = &st.ents[st.sty[sel_index(st)].ent].name;
    in_ini(&mut initxt, "style", style);
    in_ini(&mut initxt, "desktop", &st.lnk_desk);
    in_ini(&mut initxt, "start", &st.lnk_start);
    in_ini(&mut initxt, "reg", if st.opt_reg != 0 { "1" } else { "0" });
    if st.scope == Scope::Machine {
        in_ini(&mut initxt, "scope", "machine");
    }
    in_ini(&mut initxt, "files", &files);
    in_ini(&mut initxt, "assoc", &plan.remove);
    if !st.appid.is_empty() {
        in_ini(&mut initxt, "app_id", &st.appid);
    }
    // (without it the uninstaller of "installed apps" finds nothing to remove)
    let ini = in_path(st, "install.ini");
    if !plat::write_file(&ini, initxt.as_bytes()) {
        return in_fail(st, E_WRITE, &ini);
    }
    Ok(())
}

/// shortcut in the desktop (0) / start menu (1) folder of st.scope -> its path ("" when it
/// failed)  [in_link]
fn in_link(st: &Inst, which: u32, lnk: &str) -> String {
    let Some(dir) = sh_folder(which, st.scope) else { return String::new() };
    let path = path_cap(&format!("{dir}\\{lnk}"));
    if sh_link(&path, &st.exepath, &st.dir, &st.disp, &st.appid) { path } else { String::new() }
}

/// the "installed apps" entry  [in_regentry]
fn in_regentry(st: &Inst) {
    let Some(mut r) = Reg::begin(&st.app, st.scope) else { return };
    r.sz("DisplayName", &st.disp);
    r.sz("DisplayVersion", &st.ver);
    r.sz("Publisher", &st.pub_);
    r.sz("DisplayIcon", &st.exepath);
    r.sz("InstallLocation", &st.dir);
    // "<folder>\uninstall.exe" --uninstall; without questions (package managers, scripts):
    // --quiet
    let un = path_cap(&format!("\"{}\\uninstall.exe\" --uninstall", st.dir));
    r.sz("UninstallString", &un);
    r.sz("QuietUninstallString", &(un.clone() + " --quiet"));
    r.dword("NoModify", 1);
    r.dword("NoRepair", 1);
    r.dword("EstimatedSize", (st.self_.len() >> 10) as u32);
    r.end();
}

/// "<exe>" started in its folder  [inst_launch]
pub(crate) fn inst_launch(st: &Inst) -> bool {
    let cmd = path_cap(&format!("\"{}\"", st.exepath));
    sh_run(&cmd, Some(&st.dir), false)
}

/// the manifest's app as plat::assoc sees it: name, display name, app id, file types, URL
/// schemes, browser=1
fn package(st: &Inst) -> crate::assoc::Package {
    crate::assoc::Package {
        app: st.app.clone(),
        display: st.disp.clone(),
        app_id: st.appid.clone(),
        types: crate::assoc::parse_types(&st.types),
        schemes: crate::assoc::parse_schemes(&st.schemes),
        browser: st.browser,
        ..crate::assoc::Package::default()
    }
}

/// (browser=1, the option checked) let the user make the installed program the default web
/// browser: Settings at its page of the default apps, as the program's own
/// plat::default_browser_request (Windows lets no program take it itself); test mode and
/// script runs (headless) print the page instead -> whether it opened (or was printed)
pub(crate) fn inst_default_browser(st: &Inst, headless: bool) -> bool {
    sh_browser_request(&package(st), headless)
}

/// after an installation by another process (the setup with administrator rights): the
/// program's path and the shortcuts it made, from the install.ini it wrote
pub(crate) fn inst_read_back(st: &mut Inst) {
    st.dir = in_slash(&st.dir);
    st.exepath = in_path(st, &exe_name(st));
    if let Some(ini) = plat::read_file(&in_path(st, "install.ini")) {
        st.lnk_desk = kv_get(&ini, "desktop", PATH_CAP).unwrap_or_default();
        st.lnk_start = kv_get(&ini, "start", PATH_CAP).unwrap_or_default();
    }
}

// ------------------------------------------------------------------ uninstall

/// st.dir = the own folder, the values of its install.ini  [un_load]
pub(crate) fn un_load(st: &mut Inst) -> Result<(), ()> {
    let r = un_load_ini(st);
    if r.is_err() {
        st.err = String::from(t("Die Installationsdaten (install.ini) wurden nicht gefunden."));
    }
    r
}

fn un_load_ini(st: &mut Inst) -> Result<(), ()> {
    st.selfpath = plat::inst::exe_path().ok_or(())?;
    st.dir = parent_dir(&st.selfpath).ok_or(())?;
    let ini = plat::read_file(&in_path(st, "install.ini")).ok_or(())?;
    st.app = kv_get(&ini, "app", 64).ok_or(())?;
    st.disp = kv_get(&ini, "display", 128).unwrap_or_default();
    st.lnk_desk = kv_get(&ini, "desktop", PATH_CAP).unwrap_or_default();
    st.lnk_start = kv_get(&ini, "start", PATH_CAP).unwrap_or_default();
    st.files = kv_get(&ini, "files", NO_CAP).unwrap_or_default();
    st.assoc = kv_get(&ini, "assoc", NO_CAP).unwrap_or_default();
    let reg = kv_get(&ini, "reg", 8).unwrap_or_default();
    st.opt_reg = reg.as_bytes().first().is_some_and(|&c| c == b'1') as i32;
    st.scope = if kv_get(&ini, "scope", 16).is_some_and(|s| s == "machine") { Scope::Machine } else { Scope::User };
    Ok(())
}

/// install.ini's files= -> the names in it: relative paths inside the program's folder (empty
/// names, absolute paths and ".." are no files the setup wrote: skipped, so an edited list never
/// deletes outside the folder)
fn un_names(files: &str) -> Vec<String> {
    files.split(';').filter(|f| !f.is_empty() && !f.contains(':') && !f.starts_with(['\\', '/'])
                                && !f.split(['\\', '/']).any(|p| p == ".." || p == "." || p.is_empty()))
        .map(in_slash).collect()
}

/// the folders below the program's folder that hold the files, each once, the deepest first
/// ("a\b\c.txt" -> "a\b", "a")
fn un_folders(names: &[String]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for f in names {
        let mut d = parent_dir(f);
        while let Some(p) = d {
            d = parent_dir(&p);
            dirs.push(p);
        }
    }
    // more separators first: a folder before the one that holds it
    dirs.sort_unstable_by(|a, b| b.matches('\\').count().cmp(&a.matches('\\').count()).then_with(|| a.cmp(b)));
    dirs.dedup();
    dirs
}

/// a shortcut install.ini's desktop= / start= names is one the setup made: a .lnk right in the
/// scope's desktop (0) or start menu (1) folder.  The uninstaller deleted any path there, and for
/// all users it runs with administrator rights while install.ini may lie in a folder users could
/// change
fn un_link_ok(path: &str, which: u32, scope: Scope) -> bool {
    if path.is_empty() || !path.to_ascii_lowercase().ends_with(".lnk") || path.split(['\\', '/']).any(|p| p == "..") {
        return false;
    }
    let (Some(dir), Some(parent)) = (sh_folder(which, scope), parent_dir(path)) else { return false };
    in_slash(&parent).eq_ignore_ascii_case(&in_slash(&dir))
}

/// the hidden command that deletes uninstall.exe (self) and then its folder: a running exe
/// cannot be deleted, so it tries once a second until the uninstaller has ended (at most five
/// minutes; the window of the uninstaller stays open on its last page, and an uninstaller with
/// administrator rights runs while the one that started it waits).  An older uninstaller
/// tried once after two seconds and left both behind when its window stayed open longer
fn un_cleanup_cmd(self_: &str, dir: &str) -> String {
    format!("cmd.exe /c (for /l %i in (1,1,300) do @if exist \"{self_}\" (del /f /q \"{self_}\" & ping -n 2 127.0.0.1 >nul)) & rmdir \"{dir}\"")
}

/// remove the installation; cmd deletes uninstall.exe and the folder once this process has
/// ended  [un_run]
pub(crate) fn un_run(st: &mut Inst) {
    // the program
    plat::delete(&in_path(st, &exe_name(st)));
    // further files, then the folders that held them, deepest first (a delete fails while the
    // folder is not empty: what the user put there stays)
    let names = un_names(&st.files);
    for f in &names {
        plat::delete(&in_path(st, f));
    }
    for d in un_folders(&names) {
        plat::delete(&in_path(st, &d));
    }
    for (which, lnk) in [(0, st.lnk_desk.clone()), (1, st.lnk_start.clone())] {
        if un_link_ok(&lnk, which, st.scope) {
            plat::delete(&lnk);
        }
    }
    if st.opt_reg != 0 {
        sh_reg_delete(&st.app, st.scope);
    }
    sh_assoc_remove(&st.assoc, st.scope);
    plat::delete(&in_path(st, "install.ini"));
    // uninstall.exe and the folder: cmd removes them once this process has ended
    sh_run(&un_cleanup_cmd(&st.selfpath, &st.dir), None, true);
}
