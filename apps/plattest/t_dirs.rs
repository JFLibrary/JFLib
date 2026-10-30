// t_dirs -- plattest --test dirs: the user's folders and the log folder (lib/plat/dirs.rs)
//
//   user_dir for every DIR_* (absolute, '/' separators; Windows: the known folders, the same
//   desktop as the setup's plat::inst::known_folder; Linux: a user-dirs.dirs in a scratch
//   HOME / XDG_CONFIG_HOME -- "$HOME/..", absolute, escapes, the last line, bad values --,
//   then the fallbacks without it), log_dir and app_dir(APP_LOG) (XDG_STATE_HOME;
//   Windows: a redirected LOCALAPPDATA), the schemes logs:// and documents:// ..., the
//   manifest variables $DOCUMENTS ... $APPLOG, and at the end a manifest installed in this
//   process (documents writable, pictures read only, logs:// always)
// Nothing outside the scratch tree is written; a policy stays installed: not part of "all".

use alloc::format;
use alloc::string::String;

use plat::policy::{self, FS_READ, FS_WRITE};
use plat::vfs::{self, APP_LOG};
use plat::{DIR_COUNT, DIR_DESKTOP, DIR_DOCUMENTS, DIR_DOWNLOADS, DIR_MUSIC, DIR_PICTURES, DIR_VIDEOS, user_dir, user_dir_scheme};

use crate::{T, rm_tree};

// absolute ('/' or "X:/"), '/' separators, no trailing slash
fn well_formed(p: &str) -> bool {
    let abs = p.starts_with('/') || (p.len() > 2 && p.as_bytes()[1] == b':' && p.as_bytes()[2] == b'/');
    abs && !p.contains('\\') && (p.len() <= 3 || !p.ends_with('/'))
}

pub fn t_dirs(t: &mut T) {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-dirs-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&base), "dirs: scratch folder");
    let names = ["DESKTOP", "DOCUMENTS", "DOWNLOADS", "PICTURES", "MUSIC", "VIDEOS"];
    t.check(user_dir(DIR_COUNT).is_none() && user_dir_scheme(DIR_COUNT).is_none(), "dirs: an unknown kind has no folder");
    let schemes: [Option<&str>; 6] = core::array::from_fn(|k| user_dir_scheme(k as u32));
    t.check(schemes == [Some("desktop"), Some("documents"), Some("downloads"), Some("pictures"), Some("music"), Some("videos")], "dirs: the schemes of the user folders");
    if cfg!(windows) {
        windows(t, &base);
    } else {
        linux(t, &base);
    }
    // the manifest variables name the same folders
    for (k, n) in names.iter().enumerate() {
        let d = user_dir(k as u32).unwrap_or_default();
        t.check(!d.is_empty() && policy::pattern_matches(&format!("${n}/**"), &format!("{d}/x/y.txt")), &format!("dirs: ${n} is user_dir"));
    }
    let log = vfs::app_dir(APP_LOG).unwrap_or_default();
    t.check(policy::pattern_matches("$APPLOG/*.log", &format!("{log}/plattest.log")), "dirs: $APPLOG is app_dir(APP_LOG)");
    // $APPIMAGE: the folder of the running AppImage file (Linux), no value elsewhere
    t.check(!policy::pattern_matches("$APPIMAGE/**", "/opt/apps/x.AppImage"), "dirs: $APPIMAGE has no value outside an AppImage");
    if cfg!(target_os = "linux") {
        plat::setenv("APPIMAGE", Some("/opt/apps/Plattest.AppImage"));
        t.check(policy::pattern_matches("$APPIMAGE/*.AppImage*", "/opt/apps/Plattest.AppImage.jfupd-new")
            && !policy::pattern_matches("$APPIMAGE/**", "/opt/other/x"), "dirs: $APPIMAGE is the folder of $APPIMAGE");
        plat::setenv("APPIMAGE", None);
    }
    t.check(policy::install("fs.read $NOPE/x").is_err(), "dirs: an unknown variable is still refused");
    installed(t, &base);
}

fn windows(t: &mut T, base: &str) {
    for k in 0..DIR_COUNT {
        let d = user_dir(k);
        t.check(d.as_deref().is_some_and(well_formed), &format!("dirs: user_dir({k}) is an absolute path"));
        t.info(&format!("  {k}: {}\n", d.unwrap_or_default()));
    }
    #[cfg(windows)]
    {
        let setup = plat::inst::known_folder(0).map(|p| p.replace('\\', "/"));
        t.expect_s(user_dir(DIR_DESKTOP).as_deref(), setup.as_deref(), "dirs: the desktop of the setup (plat::inst::known_folder)");
    }
    let docs = user_dir(DIR_DOCUMENTS).unwrap_or_default();
    t.check(plat::file_exists(&docs) == 2, "dirs: the documents folder exists");
    t.check(user_dir(DIR_DOWNLOADS).is_some_and(|d| d != docs), "dirs: downloads is a folder of its own");
    // the log folder: below LOCALAPPDATA (redirected into the scratch tree before logs:// is used)
    t.expect_s(plat::log_dir().as_deref(), plat::data_dir().as_deref(), "dirs: log_dir is LOCALAPPDATA");
    let local = format!("{base}/local");
    plat::setenv("LOCALAPPDATA", Some(&local.replace('/', "\\")));
    let want = format!("{local}/plattest/Logs");
    t.expect_s(vfs::app_dir(APP_LOG).as_deref(), Some(want.as_str()), "dirs: app_dir(APP_LOG) = LOCALAPPDATA/<app>/Logs");
    logs_scheme(t, &want);
    t.expect_s(vfs::os_path("documents://a/b.txt").as_deref(), Some(&*format!("{docs}/a/b.txt")), "dirs: documents:// is the documents folder");
    let pics = user_dir(DIR_PICTURES).unwrap_or_default();
    t.expect_s(vfs::os_path("pictures://").as_deref(), Some(pics.as_str()), "dirs: pictures:// is the pictures folder");
}

fn linux(t: &mut T, base: &str) {
    let home = format!("{base}/home");
    let cfg = format!("{base}/cfg");
    plat::setenv("HOME", Some(&home));
    plat::setenv("XDG_CONFIG_HOME", Some(&cfg));
    let dirs_file = format!("{cfg}/user-dirs.dirs");
    let text = "# written by xdg-user-dirs-update\n\
        XDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n\
        XDG_DOCUMENTS_DIR=\"$HOME/Dokumente/\"\n\
        XDG_DOWNLOAD_DIR=\"/srv/Down \\\"load\\\\s\"\n\
        XDG_PICTURES_DIR=\"$HOME/\"\n\
        XDG_MUSIC_DIR=\"Musik\"\n\
        XDG_VIDEOS_DIR=\"$HOME/Filme\"\n\
        XDG_VIDEOS_DIR=\"$HOME/Videos 2\"\n\
        XDG_TEMPLATES_DIR=\"$HOME/Vorlagen\"\n";
    t.check(plat::mkdir(&cfg) && plat::write_file(&dirs_file, text.as_bytes()), "dirs: a user-dirs.dirs in a scratch XDG_CONFIG_HOME");
    t.expect_s(user_dir(DIR_DESKTOP).as_deref(), Some(&*format!("{home}/Schreibtisch")), "dirs: $HOME/.. of user-dirs.dirs");
    t.expect_s(user_dir(DIR_DOCUMENTS).as_deref(), Some(&*format!("{home}/Dokumente")), "dirs: a trailing slash is dropped");
    t.expect_s(user_dir(DIR_DOWNLOADS).as_deref(), Some("/srv/Down \"load\\s"), "dirs: an absolute path with escapes");
    t.expect_s(user_dir(DIR_PICTURES).as_deref(), Some(home.as_str()), "dirs: \"$HOME/\" is the home folder (disabled)");
    t.expect_s(user_dir(DIR_MUSIC).as_deref(), Some(&*format!("{home}/Music")), "dirs: a relative value falls back to $HOME/Music");
    t.expect_s(user_dir(DIR_VIDEOS).as_deref(), Some(&*format!("{home}/Videos 2")), "dirs: the last line wins");
    // the schemes: DirFs on the folders of the scratch home
    t.expect_s(vfs::os_path("documents://a/b.txt").as_deref(), Some(&*format!("{home}/Dokumente/a/b.txt")), "dirs: documents:// is the documents folder");
    t.check(plat::write_file("documents://notes/n.txt", b"doc") && plat::file_exists(&format!("{home}/Dokumente/notes/n.txt")) == 1,
        "dirs: a write through documents:// lands in the folder");
    t.check(plat::write_file("pictures://p.png", b"png") && plat::file_exists(&format!("{home}/p.png")) == 1, "dirs: pictures:// is the home (disabled)");
    // without the file: $HOME/Desktop, Documents, ...
    t.check(plat::delete(&dirs_file), "dirs: user-dirs.dirs removed");
    let fallback = ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos"];
    let all = (0..DIR_COUNT).all(|k| user_dir(k).as_deref() == Some(&*format!("{home}/{}", fallback[k as usize])));
    t.check(all, "dirs: the fallbacks below $HOME without user-dirs.dirs");
    plat::setenv("HOME", Some(&format!("{home}/")));
    t.expect_s(user_dir(DIR_MUSIC).as_deref(), Some(&*format!("{home}/Music")), "dirs: a trailing slash of $HOME is dropped");
    plat::setenv("HOME", Some(&home));
    t.check(plat::write_file(&dirs_file, text.as_bytes()), "dirs: user-dirs.dirs again (for the manifest)");
    // the log folder: $XDG_STATE_HOME, else $HOME/.local/state
    plat::setenv("XDG_STATE_HOME", None);
    t.expect_s(plat::log_dir().as_deref(), Some(&*format!("{home}/.local/state")), "dirs: log_dir falls back to $HOME/.local/state");
    plat::setenv("XDG_STATE_HOME", Some("relative/state"));
    t.expect_s(plat::log_dir().as_deref(), Some(&*format!("{home}/.local/state")), "dirs: a relative XDG_STATE_HOME is ignored");
    let state = format!("{base}/state");
    plat::setenv("XDG_STATE_HOME", Some(&format!("{state}/")));
    t.expect_s(plat::log_dir().as_deref(), Some(state.as_str()), "dirs: log_dir is XDG_STATE_HOME");
    let want = format!("{state}/plattest");
    t.expect_s(vfs::app_dir(APP_LOG).as_deref(), Some(want.as_str()), "dirs: app_dir(APP_LOG) = XDG_STATE_HOME/<app>");
    logs_scheme(t, &want);
    // HOME and the XDG variables stay redirected: the manifest test needs the scratch home
}

// logs:// is a DirFs on app_dir(APP_LOG) (want); the log file of the runtime lives there
fn logs_scheme(t: &mut T, want: &str) {
    t.expect_s(vfs::os_path("logs://plattest.log").as_deref(), Some(&*format!("{want}/plattest.log")), "dirs: logs:// is the app's log folder");
    t.check(plat::log_file(Some("logs://plattest.log"), 4096, 1), "dirs: log_file on logs:// (makes the folder)");
    plat::log_levels(plat::LOG_OFF, plat::LOG_INFO); // nothing on stderr
    plat::log_warn("dirs", "a line in logs://");
    t.check(plat::log_file(None, 0, 0), "dirs: log_file(None)");
    let text = String::from_utf8(plat::read_file(&format!("{want}/plattest.log")).unwrap_or_default()).unwrap_or_default();
    t.check(text.contains("WARN  dirs: a line in logs://"), "dirs: the log line is in the folder");
}

// a manifest with the new variables, installed in this process
fn installed(t: &mut T, base: &str) {
    let docs = user_dir(DIR_DOCUMENTS).unwrap_or_default();
    let pics = user_dir(DIR_PICTURES).unwrap_or_default();
    // (Linux: the scratch pictures folder is $HOME, so downloads -- /srv/.. -- lies outside)
    let down = user_dir(DIR_DOWNLOADS).unwrap_or_default();
    let key = match policy::install("fs.write $DOCUMENTS/**\nfs.read $PICTURES/**\n") {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("dirs: install ({e})"));
            return;
        }
    };
    t.check(policy::fs_allowed(&format!("{docs}/x.txt"), FS_WRITE), "dirs: fs.write $DOCUMENTS/** allows the documents");
    t.check(policy::fs_allowed(&format!("{pics}/x.png"), FS_READ) && !policy::fs_allowed(&format!("{pics}/x.png"), FS_WRITE),
        "dirs: fs.read $PICTURES/** reads, does not write");
    t.check(!policy::fs_allowed(&format!("{down}/x.zip"), FS_READ), "dirs: the downloads folder is not allowed");
    t.check(plat::write_file("logs://after.txt", b"log") && plat::read_file("logs://after.txt").is_some(), "dirs: logs:// stays writable (the app's own folder)");
    t.check(plat::delete("logs://after.txt"), "dirs: ... and deletable");
    if cfg!(not(windows)) {
        t.check(plat::write_file("documents://allowed.txt", b"ok"), "dirs: documents:// is writable with the rule");
        t.check(!plat::write_file("downloads://refused.txt", b"no"), "dirs: downloads:// without a rule is refused");
        t.check(plat::read_file("pictures://p.png").as_deref() == Some(&b"png"[..]) && !plat::write_file("pictures://q.png", b"no"),
            "dirs: pictures:// reads but does not write");
        let d = policy::denials();
        t.check(d.iter().any(|x| x.kind == "fs.write" && x.target.ends_with("refused.txt")), "dirs: the refusal is recorded");
    }
    key.trusted(|| rm_tree(base));
    t.check(key.trusted(|| plat::file_exists(base)) == 0, "dirs: scratch tree removed");
}
