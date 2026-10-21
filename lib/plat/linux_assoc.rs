// linux_assoc -- (assoc.rs) the .desktop file, the own MIME types and the defaults on Linux
//
//   available()           Unavailable in a Flatpak (its exported .desktop file and MIME XML)
//   conflicts(pkg, exe, mine)   what register would ask about, as if the record mine were undone
//   register(pkg, exe, take, lines, done)   the files and defaults; the record lines:
//                         file <path> <fnv>     a file written (removed while it still has
//                                               that content)
//                         saved <path>          another program's file moved aside to
//                                               <path>.jflib-saved (moved back)
//                         default <list> <mime> <had> <old> <ours> <made>   the [Default
//                                               Applications] value of mime in the mimeapps.list
//                                               <list>: put back (had 0: removed) while it is
//                                               still ours; made "file" / "group": the file / the
//                                               group was new (removed again when empty)
//   undo(lines)           the lines backwards
//   changed(notes)        update-mime-database $XDG_DATA_HOME/mime (when packages/ is there),
//                         update-desktop-database $XDG_DATA_HOME/applications (when that folder
//                         has a mimeinfo.cache); a missing tool is a note
//
// Files: $XDG_DATA_HOME/applications/<app_id>.desktop (Exec = the program, %U with schemes,
// else %F; MimeType = the file types and x-scheme-handler/<scheme>), own types in
// $XDG_DATA_HOME/mime/packages/<app_id>.xml.  The defaults of own types and schemes go to the
// user's mimeapps.list that already names one for the type ($XDG_CONFIG_HOME/<desktop>-mimeapps.list,
// mimeapps.list), else $XDG_CONFIG_HOME/mimeapps.list, as "<app_id>.desktop;" before the
// earlier list.  The default of another program: the first installed .desktop id of the first
// mimeapps.list with the type (the spec's order: config home, config dirs, data home, data
// dirs; desktop-specific files first), else another .desktop file with the type in MimeType=.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{AssocError, Conflict, Line, Package, Registered};
use crate::linux_desk::in_flatpak;

const SAVED: &str = ".jflib-saved";
const GROUP: &str = "[Default Applications]";

fn fnv(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn read_text(path: &str) -> Option<String> {
    crate::read_file_raw(path).map(|d| String::from_utf8_lossy(&d).into_owned())
}

// ---------------------------------------------------------------- places

fn data_home() -> Result<String, AssocError> {
    crate::data_dir().ok_or_else(|| AssocError::Failed(String::from("no $XDG_DATA_HOME / $HOME")))
}

fn desktop_path(pkg: &Package) -> Result<String, AssocError> {
    Ok(format!("{}/applications/{}.desktop", data_home()?, pkg.app_id))
}

fn mime_path(pkg: &Package) -> Result<String, AssocError> {
    Ok(format!("{}/mime/packages/{}.xml", data_home()?, pkg.app_id))
}

fn list(var: &str, default: &str) -> Vec<String> {
    let v = crate::env(var).filter(|s| !s.is_empty()).unwrap_or_else(|| String::from(default));
    v.split(':').filter(|d| d.starts_with('/')).map(|d| String::from(d.trim_end_matches('/'))).collect()
}

/// the desktops of $XDG_CURRENT_DESKTOP, lower case
fn desktops() -> Vec<String> {
    crate::env("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(|s| s.to_ascii_lowercase()).collect()
}

/// the mimeapps.list files in the spec's order; the first ones (up to the returned count)
/// are the user's own in $XDG_CONFIG_HOME
fn mimeapps_files() -> (Vec<String>, usize) {
    let mut dirs: Vec<String> = Vec::new();
    let mut user = 0;
    if let Some(c) = crate::config_dir() {
        dirs.push(c);
        user = 1;
    }
    dirs.extend(list("XDG_CONFIG_DIRS", "/etc/xdg"));
    if let Some(d) = crate::data_dir() {
        dirs.push(format!("{d}/applications"));
    }
    dirs.extend(list("XDG_DATA_DIRS", "/usr/local/share:/usr/share").into_iter().map(|d| format!("{d}/applications")));
    let mut out = Vec::new();
    let mut user_files = 0;
    for (k, d) in dirs.iter().enumerate() {
        for desk in desktops() {
            out.push(format!("{d}/{desk}-mimeapps.list"));
        }
        out.push(format!("{d}/mimeapps.list"));
        if k < user {
            user_files = out.len();
        }
    }
    (out, user_files)
}

/// the folders of installed .desktop files ($XDG_DATA_HOME first)
fn app_dirs() -> Vec<String> {
    let mut v = Vec::new();
    if let Some(d) = crate::data_dir() {
        v.push(format!("{d}/applications"));
    }
    v.extend(list("XDG_DATA_DIRS", "/usr/local/share:/usr/share").into_iter().map(|d| format!("{d}/applications")));
    v
}

fn installed(id: &str) -> bool {
    !id.contains('/') && app_dirs().iter().any(|d| crate::file_exists_raw(&format!("{d}/{id}")) == 1)
}

// ---------------------------------------------------------------- mimeapps.list

/// the value of key in the [Default Applications] group
fn group_get(text: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == GROUP;
            continue;
        }
        if inside {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return Some(String::from(v.trim()));
                }
            }
        }
    }
    None
}

/// text with key = value in [Default Applications] (None: the line removed): an existing line
/// is replaced where it is, a new one goes after the group's last entry (a new group at the
/// end); every other byte stays
fn group_set(text: &str, key: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let newline = text.is_empty() || text.ends_with('\n');
    let entry = |l: &str| l.trim().split_once('=').is_some_and(|(k, _)| k.trim() == key);
    match lines.iter().position(|l| l.trim() == GROUP) {
        Some(g) => {
            let end = lines[g + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |e| g + 1 + e);
            match (lines[g + 1..end].iter().position(|l| entry(l)).map(|p| g + 1 + p), value) {
                (Some(p), Some(v)) => lines[p] = format!("{key}={v}"),
                (Some(p), None) => {
                    lines.remove(p);
                }
                (None, Some(v)) => {
                    let last = (g + 1..end).rev().find(|&k| !lines[k].trim().is_empty()).unwrap_or(g);
                    lines.insert(last + 1, format!("{key}={v}"));
                }
                (None, None) => {}
            }
        }
        None => {
            let Some(v) = value else { return String::from(text) };
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(String::from(GROUP));
            lines.push(format!("{key}={v}"));
        }
    }
    let mut s = lines.join("\n");
    if newline && !s.is_empty() {
        s.push('\n');
    }
    s
}

/// text without a [Default Applications] group that has no entries (and the blank line before
/// it, which group_set added)
fn drop_empty_group(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(k) = lines.iter().position(|l| l.trim() == GROUP) else { return String::from(text) };
    let end = lines[k + 1..].iter().position(|l| l.trim().starts_with('[')).map_or(lines.len(), |e| k + 1 + e);
    if lines[k + 1..end].iter().any(|l| !l.trim().is_empty() && !l.trim().starts_with('#')) {
        return String::from(text);
    }
    let start = if k > 0 && lines[k - 1].trim().is_empty() { k - 1 } else { k };
    let mut out: Vec<&str> = lines[..start].to_vec();
    out.extend_from_slice(&lines[end..]);
    let mut s = out.join("\n");
    if !s.is_empty() && text.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn ids(value: &str) -> impl Iterator<Item = &str> {
    value.split(';').map(str::trim).filter(|s| !s.is_empty())
}

/// the value of mime in the file f as it was before the record mine (None: none)
fn value_before(f: &str, mime: &str, mine: &[Line]) -> Option<String> {
    if let Some(l) = mine.iter().find(|l| l[0] == "default" && l.get(1).is_some_and(|x| x == f) && l.get(2).is_some_and(|m| m == mime)) {
        return (l.get(3).map(String::as_str) == Some("1")).then(|| l.get(4).cloned().unwrap_or_default());
    }
    group_get(&read_text(f)?, mime)
}

/// the program that opens mime (as it was before the record mine), not ours
fn default_holder(mime: &str, ours: &str, mine: &[Line]) -> Option<String> {
    let (files, _) = mimeapps_files();
    for f in &files {
        let Some(v) = value_before(f, mime, mine) else { continue };
        if let Some(id) = ids(&v).find(|id| *id != ours && installed(id)) {
            return Some(String::from(id));
        }
        if ids(&v).any(|id| id == ours) {
            return None; // ours comes first here
        }
    }
    // no default: another installed .desktop file with the type
    for d in app_dirs() {
        let mut found = None;
        crate::dir_list_raw(&d, &mut |name, is_dir, _| {
            if found.is_none() && !is_dir && name.ends_with(".desktop") && name != ours {
                let text = read_text(&format!("{d}/{name}")).unwrap_or_default();
                if text.lines().any(|l| l.strip_prefix("MimeType=").is_some_and(|v| ids(v).any(|m| m == mime))) {
                    found = Some(String::from(name));
                }
            }
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

/// the user's file to write mime's default to: the one that names it already, else mimeapps.list
fn default_file(mime: &str) -> Option<String> {
    let (files, user) = mimeapps_files();
    files[..user].iter().find(|f| read_text(f).is_some_and(|t| group_get(&t, mime).is_some())).cloned()
        .or_else(|| crate::config_dir().map(|c| format!("{c}/mimeapps.list")))
}

// ---------------------------------------------------------------- the files

fn desktop_text(pkg: &Package, exe: &str) -> String {
    let code = if pkg.schemes.is_empty() { "%F" } else { "%U" };
    let mut mimes: Vec<String> = Vec::new();
    for m in pkg.types.iter().map(|t| t.mime.clone()).chain(pkg.schemes.iter().map(|s| format!("x-scheme-handler/{s}"))) {
        if !mimes.contains(&m) {
            mimes.push(m);
        }
    }
    let exec = crate::autostart::native::exec_line(exe, &[]);
    let mut s = format!("[Desktop Entry]\nType=Application\nName={}\nExec={exec} {code}\nTerminal=false\nCategories={}\n", pkg.display, pkg.categories);
    if !mimes.is_empty() {
        s.push_str(&format!("MimeType={};\n", mimes.join(";")));
    }
    // JFLib's window class is the name the program was started by (an AppImage: its runtime's)
    if crate::env("APPIMAGE").is_none() {
        let name = &exe[exe.rfind('/').map_or(0, |k| k + 1)..];
        if !name.is_empty() {
            s.push_str(&format!("StartupWMClass={name}\n"));
        }
    }
    s
}

fn mime_text(pkg: &Package) -> Option<String> {
    let own: Vec<_> = pkg.types.iter().filter(|t| t.own).collect();
    if own.is_empty() {
        return None;
    }
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n");
    for t in own {
        s.push_str(&format!("  <mime-type type=\"{}\">\n    <comment>{}</comment>\n", xml(&t.mime), xml(&t.desc)));
        for e in &t.exts {
            s.push_str(&format!("    <glob pattern=\"*.{}\"/>\n", xml(e)));
        }
        s.push_str("  </mime-type>\n");
    }
    s.push_str("</mime-info>\n");
    Some(s)
}

/// the MIME types the app becomes the default of: own types and the schemes
fn default_types(pkg: &Package) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for m in pkg.types.iter().filter(|t| t.own).map(|t| t.mime.clone()).chain(pkg.schemes.iter().map(|s| format!("x-scheme-handler/{s}"))) {
        if !v.contains(&m) {
            v.push(m);
        }
    }
    v
}

/// a file at path that is not the one we would write (mine: the record; a file it wrote counts
/// as missing, a file it moved aside as there) -> the conflict
fn file_conflict(path: &str, text: &str, mine: &[Line]) -> Option<Conflict> {
    let saved = mine.iter().any(|l| l[0] == "saved" && l.get(1).is_some_and(|p| p == path));
    let wrote = mine.iter().any(|l| l[0] == "file" && l.get(1).is_some_and(|p| p == path));
    let current = if saved {
        read_text(&format!("{path}{SAVED}"))
    } else if wrote {
        None
    } else {
        read_text(path)
    };
    let current = current?;
    if current == text {
        return None;
    }
    let exec = current.lines().find_map(|l| l.strip_prefix("Exec=")).map(String::from);
    Some(Conflict { what: String::from(path), holder: exec.unwrap_or_else(|| String::from("another program's file")), fixed: false })
}

pub fn available() -> Result<(), AssocError> {
    if in_flatpak() {
        return Err(AssocError::Unavailable);
    }
    Ok(())
}

pub fn conflicts(pkg: &Package, exe: &str, mine: &[Line]) -> Vec<Conflict> {
    let mut out = Vec::new();
    if let Ok(p) = desktop_path(pkg) {
        out.extend(file_conflict(&p, &desktop_text(pkg, exe), mine));
    }
    if let (Ok(p), Some(t)) = (mime_path(pkg), mime_text(pkg)) {
        out.extend(file_conflict(&p, &t, mine));
    }
    let ours = format!("{}.desktop", pkg.app_id);
    for m in default_types(pkg) {
        if let Some(holder) = default_holder(&m, &ours, mine) {
            out.push(Conflict { what: m, holder, fixed: false });
        }
    }
    out
}

/// write a file of ours at path (another one moved aside first) and record it
fn put_file(path: &str, text: &str, lines: &mut Vec<Line>) -> Result<(), AssocError> {
    let dir = &path[..path.rfind('/').unwrap_or(0)];
    if crate::file_exists_raw(path) == 1 {
        let aside = format!("{path}{SAVED}");
        if crate::file_exists_raw(&aside) != 0 || !crate::rename_raw(path, &aside) {
            return Err(AssocError::Failed(format!("{path} could not be moved aside")));
        }
        lines.push(alloc::vec![String::from("saved"), String::from(path)]);
    }
    if !crate::mkdir_raw(dir) || !crate::write_file_raw(path, text.as_bytes()) {
        return Err(AssocError::Failed(format!("{path} was not written")));
    }
    lines.push(alloc::vec![String::from("file"), String::from(path), fnv(text.as_bytes())]);
    Ok(())
}

pub fn register(pkg: &Package, exe: &str, take: &mut dyn FnMut(&Conflict) -> bool, lines: &mut Vec<Line>, done: &mut Registered) -> Result<(), AssocError> {
    let ours = format!("{}.desktop", pkg.app_id);
    // the .desktop file: everything else needs it
    let desk = desktop_path(pkg)?;
    let text = desktop_text(pkg, exe);
    match file_conflict(&desk, &text, &[]) {
        Some(c) if !take(&c) => return Err(AssocError::Declined(c)),
        _ => {}
    }
    if read_text(&desk).as_deref() != Some(text.as_str()) {
        put_file(&desk, &text, lines)?;
    }
    // the own types
    let mut own_ok = true;
    if let Some(t) = mime_text(pkg) {
        let p = mime_path(pkg)?;
        match file_conflict(&p, &t, &[]) {
            Some(c) if !take(&c) => {
                done.kept.push(c);
                own_ok = false;
            }
            _ => {
                if read_text(&p).as_deref() != Some(t.as_str()) {
                    put_file(&p, &t, lines)?;
                }
            }
        }
    }
    // the defaults
    for m in default_types(pkg) {
        if !own_ok && !m.starts_with("x-scheme-handler/") {
            continue;
        }
        if let Some(holder) = default_holder(&m, &ours, &[]) {
            let c = Conflict { what: m.clone(), holder, fixed: false };
            if !take(&c) {
                done.kept.push(c);
                continue;
            }
        }
        let file = default_file(&m).ok_or_else(|| AssocError::Failed(String::from("no $XDG_CONFIG_HOME / $HOME")))?;
        let existing = read_text(&file);
        let made = match &existing {
            None => "file",
            Some(t) if !t.lines().any(|l| l.trim() == GROUP) => "group",
            _ => "",
        };
        let before = existing.unwrap_or_default();
        let old = group_get(&before, &m);
        let mut value = format!("{ours};");
        for id in old.as_deref().map(ids).into_iter().flatten().filter(|id| *id != ours) {
            value.push_str(id);
            value.push(';');
        }
        if old.as_deref() == Some(value.as_str()) {
            continue;
        }
        let dir = &file[..file.rfind('/').unwrap_or(0)];
        if !crate::mkdir_raw(dir) || !crate::write_file_raw(&file, group_set(&before, &m, Some(&value)).as_bytes()) {
            return Err(AssocError::Failed(format!("{file} was not written")));
        }
        let had = String::from(if old.is_some() { "1" } else { "0" });
        lines.push(alloc::vec![String::from("default"), file, m, had, old.unwrap_or_default(), value, String::from(made)]);
    }
    done.types = if own_ok { pkg.types.len() } else { pkg.types.iter().filter(|t| !t.own).count() };
    done.schemes = pkg.schemes.len();
    Ok(())
}

pub fn undo(lines: &[Line]) {
    let field = |l: &Line, i: usize| l.get(i).cloned().unwrap_or_default();
    for l in lines.iter().rev() {
        match l[0].as_str() {
            "file" => {
                let p = field(l, 1);
                if crate::read_file_raw(&p).is_some_and(|d| fnv(&d) == field(l, 2)) {
                    crate::delete_raw(&p);
                }
            }
            "saved" => {
                let p = field(l, 1);
                if crate::file_exists_raw(&p) == 0 {
                    crate::rename_raw(&format!("{p}{SAVED}"), &p);
                }
            }
            "default" => {
                let (file, mime) = (field(l, 1), field(l, 2));
                let Some(text) = read_text(&file) else { continue };
                if group_get(&text, &mime).as_deref() != Some(field(l, 5).as_str()) {
                    continue;
                }
                let old = field(l, 4);
                let mut back = group_set(&text, &mime, (field(l, 3) == "1").then_some(old.as_str()));
                let made = field(l, 6);
                if made == "file" && back.trim() == GROUP {
                    crate::delete_raw(&file);
                    continue;
                }
                if made == "group" {
                    back = drop_empty_group(&back);
                }
                crate::write_file_raw(&file, back.as_bytes());
            }
            _ => {}
        }
    }
}

/// run a tool of the desktop (from PATH) with args, at most 30 s -> its exit code, None when
/// it is not there
fn run(tool: &str, args: &[&str]) -> Option<i32> {
    let cmd = crate::process::Command { program: tool, args, cwd: None, env: &[] };
    let mut child = cmd.spawn_own().ok()?;
    let end = crate::time_us() + 30_000_000;
    let mut buf = [0u8; 512];
    loop {
        while let Ok(Some(n)) = child.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
        }
        while let Ok(Some(n)) = child.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
        }
        match child.try_wait() {
            Ok(Some(code)) => return Some(code),
            Err(_) => return None,
            Ok(None) if crate::time_us() > end => {
                child.terminate();
                return Some(-1);
            }
            Ok(None) => crate::sleep_ms(10),
        }
    }
}

pub fn changed(notes: &mut Vec<String>) {
    let Some(d) = crate::data_dir() else { return };
    let mime = format!("{d}/mime");
    if crate::file_exists_raw(&format!("{mime}/packages")) == 2 {
        match run("update-mime-database", &[&mime]) {
            Some(0) => {}
            Some(c) => notes.push(format!("update-mime-database {mime}: exit code {c}")),
            None => notes.push(String::from(
                "update-mime-database is missing: the own file types are known after the next update of the MIME database",
            )),
        }
    }
    let apps = format!("{d}/applications");
    if crate::file_exists_raw(&format!("{apps}/mimeinfo.cache")) == 1 {
        if let Some(c) = run("update-desktop-database", &[&apps]).filter(|&c| c != 0) {
            notes.push(format!("update-desktop-database {apps}: exit code {c}"));
        }
    }
}
