// install -- the setup's work: install the chosen style variant and the further files,
// shortcuts, the uninstall entry and install.ini; uninstall it again
//                                                              [apps/installer/src/install.s]
//
//   inst_run(st) -> Ok / Err (st.err)   install into st.dir (style st.sel, options st.opt_*)
//   inst_launch(st) -> bool             start the installed program
//   un_load(st) -> Ok / Err             uninstaller: read install.ini next to the own exe
//   un_run(st)                          remove files, shortcuts, uninstall entry, then let cmd
//                                       delete uninstall.exe and the folder after the exit
//
// install.ini (key=value): app, display, style, desktop, start (shortcut paths or empty),
// reg (1 / 0), files (further files relative to the folder, ';'-separated, '\'), assoc (what
// the file types and URL schemes added, assoc.rs), app_id (the program's AppUserModelID: the
// shortcuts, the pins and the ProgIDs carry it, and the program runs under it when it finds
// this line beside itself, lib/plat/assoc.rs app_id_boot)

use alloc::format;
use alloc::string::String;

use crate::payload::{PK_FILE, STY_MAX, kv_get, parent_dir, path_cap, pl_inflate};
use crate::shell::{Reg, sh_assoc_remove, sh_assoc_set, sh_folder, sh_link, sh_pins, sh_reg_delete, sh_run};
use crate::{Inst, PATH_CAP, str_cap};

const IN_FILES_CAP: usize = 4096; // in_files: "a;b;c" of the uninstaller
const INITXT_CAP: usize = 8192;

/// st.dir + "\" + name  [in_path]
fn in_path(st: &Inst, name: &str) -> String {
    path_cap(&format!("{}\\{}", st.dir, name))
}

/// st.err = msg with its %s replaced by arg (512 bytes)  [in_fail]
fn in_fail(st: &mut Inst, msg: &str, arg: &str) -> Result<(), ()> {
    st.err = str_cap(&msg.replacen("%s", arg, 1), 512);
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
fn in_ini(txt: &mut String, key: &str, value: &str) {
    for s in [key, "=", value, "\r\n"] {
        let t = str_cap(&(core::mem::take(txt) + s), INITXT_CAP);
        *txt = t;
    }
}

/// the style index installed: st.sel, the first when out of range
fn sel_index(st: &Inst) -> usize {
    let s = if (st.sel as u32 as usize) < st.sty.len() { st.sel as usize } else { 0 };
    s & (STY_MAX - 1)
}

/// install into st.dir -> Ok / Err (st.err)  [inst_run]
pub(crate) fn inst_run(st: &mut Inst) -> Result<(), ()> {
    let mut initxt = String::new();
    let mut files = String::new();
    st.lnk_desk.clear();
    st.lnk_start.clear();
    st.dir = in_slash(&st.dir);
    // folder
    if !plat::mkdir(&st.dir) {
        let d = st.dir.clone();
        return in_fail(st, "Der Ordner „%s“ konnte nicht angelegt werden.", &d);
    }
    // the program in the chosen style
    let ent = st.sty[sel_index(st)].ent;
    let name = str_cap(&(str_cap(&st.app, 64) + ".exe"), 64);
    st.exepath = in_path(st, &name);
    if !in_write_entry(st, ent, &st.exepath) {
        let p = st.exepath.clone();
        return in_fail(st, "„%s“ konnte nicht geschrieben werden (läuft das Programm noch?).", &p);
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
            return in_fail(st, "„%s“ konnte nicht geschrieben werden.", &path);
        }
        if !files.is_empty() {
            files = str_cap(&(files + ";"), IN_FILES_CAP);
        }
        files = str_cap(&(files + &st.ents[i].name), IN_FILES_CAP);
    }
    // the uninstaller: the setup without its payload (and without a signature of the setup)
    let un = in_path(st, "uninstall.exe");
    if !plat::write_file(&un, &crate::payload::stub(st)) {
        return in_fail(st, "„%s“ konnte nicht geschrieben werden.", &un);
    }
    // shortcuts: <folder>\<display>.lnk
    let lnk = str_cap(&(st.disp.clone() + ".lnk"), 128);
    if st.opt_desktop != 0 {
        st.lnk_desk = in_link(st, 0, &lnk);
    }
    if st.opt_start != 0 {
        st.lnk_start = in_link(st, 1, &lnk);
    }
    // the taskbar's pins of an earlier version: the program's new id (else two buttons)
    sh_pins(&st.exepath, &st.appid);
    if st.opt_reg != 0 {
        in_regentry(st);
    }
    // file types and URL schemes (PACKAGE filetypes= / schemes=)
    let types = crate::assoc::parse_types(&st.types);
    let schemes = crate::assoc::parse_schemes(&st.schemes);
    let plan = crate::assoc::plan(&st.app, &st.disp, &st.exepath, &types, &schemes, &st.appid);
    sh_assoc_set(&plan.set);
    in_ini(&mut initxt, "app", &st.app);
    in_ini(&mut initxt, "display", &st.disp);
    let style = &st.ents[st.sty[sel_index(st)].ent].name;
    in_ini(&mut initxt, "style", style);
    in_ini(&mut initxt, "desktop", &st.lnk_desk);
    in_ini(&mut initxt, "start", &st.lnk_start);
    in_ini(&mut initxt, "reg", if st.opt_reg != 0 { "1" } else { "0" });
    in_ini(&mut initxt, "files", &files);
    in_ini(&mut initxt, "assoc", &plan.remove);
    if !st.appid.is_empty() {
        in_ini(&mut initxt, "app_id", &st.appid);
    }
    plat::write_file(&in_path(st, "install.ini"), initxt.as_bytes());
    Ok(())
}

/// shortcut in the desktop (0) / start menu (1) folder -> its path ("" when it failed)  [in_link]
fn in_link(st: &Inst, which: u32, lnk: &str) -> String {
    let Some(dir) = sh_folder(which) else { return String::new() };
    let path = path_cap(&format!("{dir}\\{lnk}"));
    if sh_link(&path, &st.exepath, &st.dir, &st.disp, &st.appid) { path } else { String::new() }
}

/// the "installed apps" entry  [in_regentry]
fn in_regentry(st: &Inst) {
    let Some(mut r) = Reg::begin(&st.app) else { return };
    r.sz("DisplayName", &st.disp);
    r.sz("DisplayVersion", &st.ver);
    r.sz("Publisher", &st.pub_);
    r.sz("DisplayIcon", &st.exepath);
    r.sz("InstallLocation", &st.dir);
    // "<folder>\uninstall.exe" --uninstall
    let un = path_cap(&format!("\"{}\\uninstall.exe\" --uninstall", st.dir));
    r.sz("UninstallString", &un);
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

// ------------------------------------------------------------------ uninstall

const ERR_NOINI: &str = "Die Installationsdaten (install.ini) wurden nicht gefunden.";

/// st.dir = the own folder, the values of its install.ini  [un_load]
pub(crate) fn un_load(st: &mut Inst) -> Result<(), ()> {
    let r = un_load_ini(st);
    if r.is_err() {
        st.err = String::from(ERR_NOINI);
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
    st.files = kv_get(&ini, "files", IN_FILES_CAP).unwrap_or_default();
    st.assoc = kv_get(&ini, "assoc", INITXT_CAP).unwrap_or_default();
    let reg = kv_get(&ini, "reg", 8).unwrap_or_default();
    st.opt_reg = reg.as_bytes().first().is_some_and(|&c| c == b'1') as i32;
    Ok(())
}

/// remove the installation; cmd deletes uninstall.exe and the folder once this process has
/// ended  [un_run]
pub(crate) fn un_run(st: &mut Inst) {
    // the program
    let name = str_cap(&(str_cap(&st.app, 64) + ".exe"), 64);
    plat::delete(&in_path(st, &name));
    // further files (and their folders, when empty)
    // (as the asm walks the list: an empty list or a trailing ';' adds no name, an empty name
    // between two ';' is the folder itself, whose delete fails)
    let mut names: alloc::vec::Vec<&str> = st.files.split(';').collect();
    if names.last() == Some(&"") {
        names.pop();
    }
    for f in names {
        let path = in_path(st, f);
        plat::delete(&path);
        if let Some(d) = parent_dir(&path) {
            plat::delete(&d); // the folder (fails while not empty)
        }
    }
    if !st.lnk_desk.is_empty() {
        plat::delete(&st.lnk_desk);
    }
    if !st.lnk_start.is_empty() {
        plat::delete(&st.lnk_start);
    }
    if st.opt_reg != 0 {
        sh_reg_delete(&st.app);
    }
    sh_assoc_remove(&st.assoc);
    plat::delete(&in_path(st, "install.ini"));
    // uninstall.exe and the folder: cmd removes them once this process has ended
    let cmd = path_cap(&format!(
        "cmd.exe /c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"",
        st.selfpath, st.dir
    ));
    sh_run(&cmd, None, true);
}
