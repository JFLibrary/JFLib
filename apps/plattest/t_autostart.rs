// t_autostart -- starting the app at login (lib/plat/autostart.rs)
//
//   plattest --test autostart       Windows: the real Run value "plattest" of HKCU (removed
//                                   first and at the end): written with quoted arguments,
//                                   replaced, the Task Manager's "off" (StartupApproved) read
//                                   and cleared, removed.  Linux: XDG autostart in a scratch
//                                   XDG_CONFIG_HOME (jfgui_AUTOSTART=file, desktop id
//                                   plattest-autostart): the .desktop file, Exec quoting
//                                   (spaces, %, quotes, backslashes, $), Hidden=true and
//                                   X-GNOME-Autostart-enabled=false read as off, removed
//   plattest --test autostart-bus --mode grant|deny|none   (Linux; tools/platshared/desktop_bus.py
//                                   starts it on a fake session bus) the Background portal:
//                                   granted (on, then off, remembered in config://autostart),
//                                   refused by the user (Dismissed), no portal (Unavailable)

use alloc::format;
use alloc::string::String;

use plat::{AutostartError, autostart_entry, autostart_get, autostart_set};

use crate::T;

/// the arguments the tests pass, and their Exec form after the program (Linux)
const ARGS: [&str; 6] = ["--hidden", "a b", "100%", "x\"y", "back\\slash", "$HOME"];
const EXEC_ARGS: &str = r#"--hidden "a b" 100%% "x\\"y" "back\\\\slash" "\\$HOME""#;
/// their Windows form (CommandLineToArgvW rules)
#[cfg(windows)]
const CMD_ARGS: &str = r#"--hidden "a b" 100% "x\"y" back\slash $HOME"#;

fn scratch(t: &mut T, tag: &str) -> String {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-{tag}-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&base), &format!("{tag}: scratch folder"));
    base
}

pub fn t_autostart(t: &mut T) {
    t.check(autostart_set(true, &["a\0b"]) == Err(AutostartError::Invalid), "autostart: an argument with NUL refused");
    if cfg!(windows) {
        windows(t);
    } else {
        linux(t);
    }
}

#[cfg(not(windows))]
fn windows(_: &mut T) {}

#[cfg(windows)]
fn windows(t: &mut T) {
    const APPROVED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
    let _ = autostart_set(false, &[]);
    t.check(!autostart_get() && autostart_entry().is_none(), "autostart: no entry at the start");
    let exe = plat::current_exe().unwrap_or_default().replace('/', "\\");
    t.check(autostart_set(true, &ARGS).is_ok(), "autostart: Run value written");
    t.check(autostart_get(), "autostart: on");
    t.expect_s(autostart_entry().as_deref(), Some(&format!("\"{exe}\" {CMD_ARGS}")), "autostart: the command line, arguments quoted");
    t.check(autostart_set(true, &[]).is_ok(), "autostart: replaced");
    t.expect_s(autostart_entry().as_deref(), Some(&format!("\"{exe}\"")), "autostart: the new command line");
    // the Task Manager's switch: an odd first byte = off
    let k = plat::inst::reg_create(APPROVED);
    t.check(k.is_some(), "autostart: StartupApproved\\Run opened");
    if let Some(k) = k {
        k.set_dword(&plat::vfs::app_id(), 3);
    }
    t.check(!autostart_get() && autostart_entry().is_some(), "autostart: off in the Task Manager -> autostart_get false");
    t.check(autostart_set(true, &["--tray"]).is_ok() && autostart_get(), "autostart: set again clears the Task Manager's off");
    t.check(autostart_set(false, &[]).is_ok(), "autostart: removed");
    t.check(!autostart_get() && autostart_entry().is_none(), "autostart: no entry at the end");
    t.check(autostart_set(false, &[]).is_ok(), "autostart: removing a missing entry is fine");
}

fn linux(t: &mut T) {
    let base = scratch(t, "autostart");
    let saved = plat::env("XDG_CONFIG_HOME");
    plat::setenv("XDG_CONFIG_HOME", Some(&base));
    plat::setenv("jfgui_AUTOSTART", Some("file"));
    plat::setenv("jfgui_DESKTOP_ID", Some("plattest-autostart"));
    let file = format!("{base}/autostart/plattest-autostart.desktop");
    t.check(!autostart_get() && autostart_entry().is_none(), "autostart: no entry at the start");
    t.check(autostart_set(true, &ARGS).is_ok(), "autostart: .desktop file written");
    t.check(plat::file_exists(&file) == 1, "autostart: $XDG_CONFIG_HOME/autostart/<desktop id>.desktop");
    let text = String::from_utf8(plat::read_file(&file).unwrap_or_default()).unwrap_or_default();
    t.check(text.starts_with("[Desktop Entry]\nType=Application\n") && text.contains("\nX-GNOME-Autostart-enabled=true\n")
        && text.contains("\nIcon=plattest-autostart\n"), "autostart: the entry's keys");
    t.check(autostart_get(), "autostart: on");
    let exec = autostart_entry().unwrap_or_default();
    t.check(exec.ends_with(&format!(" {EXEC_ARGS}")), "autostart: Exec quoted (spaces, %, quotes, backslashes, $)");
    t.info(&format!("  Exec={exec}\n"));
    t.check(text.contains(&format!("\nExec={exec}\n")), "autostart: autostart_entry is the Exec line");
    plat::setenv("APPIMAGE", Some("/opt/My Apps/plattest.AppImage"));
    t.check(autostart_set(true, &["--tray"]).is_ok(), "autostart: replaced");
    t.expect_s(autostart_entry().as_deref(), Some("\"/opt/My Apps/plattest.AppImage\" --tray"), "autostart: the AppImage is started");
    plat::setenv("APPIMAGE", None);
    for off in ["Hidden=true", "X-GNOME-Autostart-enabled=false"] {
        let changed = text.replace("X-GNOME-Autostart-enabled=true", off);
        plat::write_file(&file, changed.as_bytes());
        t.check(!autostart_get() && autostart_entry().is_some(), &format!("autostart: {off} -> off"));
    }
    plat::write_file(&file, b"[Other]\nHidden=true\n[Desktop Entry]\nExec=x\n");
    t.check(autostart_get(), "autostart: keys of other groups do not count");
    t.check(autostart_set(false, &[]).is_ok(), "autostart: removed");
    t.check(plat::file_exists(&file) == 0 && !autostart_get(), "autostart: the file is gone");
    t.check(autostart_set(false, &[]).is_ok(), "autostart: removing a missing entry is fine");
    for v in ["jfgui_AUTOSTART", "jfgui_DESKTOP_ID"] {
        plat::setenv(v, None);
    }
    plat::setenv("XDG_CONFIG_HOME", saved.as_deref());
    crate::rm_tree(&base);
}

#[cfg(not(target_os = "linux"))]
pub fn t_autostart_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test autostart-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_autostart_bus(t: &mut T) {
    let args = base::Args(plat::args());
    let mode = args.value("--mode").unwrap_or("grant");
    let base = scratch(t, "autostart-bus");
    plat::setenv("XDG_CONFIG_HOME", Some(&base));
    plat::setenv("jfgui_AUTOSTART", Some("portal"));
    t.check(!autostart_get(), "autostart-bus: off at the start");
    match mode {
        "grant" => {
            t.check(autostart_set(true, &["--hidden", "a b"]).is_ok(), "autostart-bus: granted");
            t.check(autostart_get(), "autostart-bus: on (remembered)");
            t.check(autostart_entry().is_some_and(|e| e.ends_with(" --hidden \"a b\"")), "autostart-bus: the command remembered");
            t.check(autostart_set(true, &[]).is_ok(), "autostart-bus: granted without arguments (no commandline)");
            t.check(autostart_set(false, &[]).is_ok(), "autostart-bus: switched off");
            t.check(!autostart_get() && autostart_entry().is_none(), "autostart-bus: off (forgotten)");
        }
        "deny" => {
            t.check(autostart_set(true, &[]) == Err(AutostartError::Dismissed), "autostart-bus: refused -> Dismissed");
            t.check(!autostart_get(), "autostart-bus: still off");
        }
        _ => {
            t.check(autostart_set(true, &[]) == Err(AutostartError::Unavailable), "autostart-bus: no portal -> Unavailable");
            t.check(!autostart_get(), "autostart-bus: still off");
        }
    }
    plat::setenv("jfgui_AUTOSTART", None);
    crate::rm_tree(&base);
    crate::out("autostart-bus: done\n");
}
