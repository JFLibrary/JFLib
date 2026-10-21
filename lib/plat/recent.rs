// recent -- the system's list of recently used files
//
//   recent_add(path) -> bool        a file the user opened or saved.  Windows:
//                                   SHAddToRecentDocs (Explorer's recent files; the app's jump
//                                   list shows it under "Recent" when the app is a registered
//                                   handler of the type: the setup, assoc_register, MSIX).
//                                   Linux: a bookmark in $XDG_DATA_HOME/recently-used.xbel (the
//                                   freedesktop format GTK and KDE read: href, added / modified /
//                                   visited, the MIME type, this app's entry with exec and count)
//   recent_list(max) -> [path]      this app's files, newest first (Windows: the jump list's
//                                   recent items, IApplicationDocumentLists; Linux: the
//                                   bookmarks with an entry of this app)
//   recent_remove(path) -> bool     the file out of the app's list (Windows: the jump list,
//                                   IApplicationDestinations -- Explorer's recent files keep it;
//                                   Linux: this app's entry, the bookmark when no other app is
//                                   left in it)
//   recent_clear() -> bool          every file of the app (RemoveAllDestinations; every entry of
//                                   this app)
//   recent_os(on)                   (ui::main!) off in headless runs (--script, --shot, the
//                                   agent protocols): add / remove / clear leave the system's
//                                   list alone and return false
//
// The path must be readable under the app's permissions (fs.read / fs.user, policy.rs; else
// "PERMDENY fs.read" and false) and be an OS file (a folder mount's file counts as its OS path;
// mem:// and app:// do not).  jfgui_RECENT=off: nothing is read or written; jfgui_RECENT=
// /path/x.xbel: that file instead of the system's list, on every OS (tests).  The xbel file is
// changed under a lock (a sidecar file in $XDG_RUNTIME_DIR, else beside it) and replaced
// atomically; other programs' bookmarks stay byte for byte.  GLib does not lock: a write of a
// GTK program between our read and our rename can be lost (as between two GTK programs).  The
// MIME type of a bookmark comes from the app's PACKAGE filetypes, else a short table, else
// application/octet-stream.  In a Flatpak $XDG_DATA_HOME is the app's own folder: the host's
// list needs --filesystem=xdg-data/recently-used.xbel (PACKAGE flatpak_args=) and is used
// then.  Web and Android: false / empty.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::assoc::{file_uri, file_uri_path};

static OS_ON: AtomicBool = AtomicBool::new(true);
/// how long a writer waits for the lock of the xbel file
const LOCK_WAIT_MS: u32 = 2000;

/// (ui::main!) false: add / remove / clear leave the system's list alone (headless runs)
#[doc(hidden)]
pub fn recent_os(on: bool) {
    OS_ON.store(on, Ordering::Relaxed);
}

/// where the list is
enum Target {
    Off,
    Xbel(String),
    #[allow(dead_code)] // Windows only
    Native,
}

fn target(writing: bool) -> Target {
    match crate::env("jfgui_RECENT").as_deref() {
        Some("off") => return Target::Off,
        Some(p) if !p.is_empty() => return Target::Xbel(String::from(p)),
        _ => {}
    }
    if writing && !OS_ON.load(Ordering::Relaxed) {
        return Target::Off;
    }
    native::target()
}

/// the OS path of a file the app may read (absolute, '/' separators), else None
fn checked(path: &str) -> Option<String> {
    let os = crate::vfs::os_path(path)?;
    let os = crate::policy::normalize(&os)?;
    if !crate::policy::fs_allowed(&os, crate::policy::FS_READ) {
        return None;
    }
    Some(os)
}

/// put a file the user opened or saved into the system's list of recent files
pub fn recent_add(path: &str) -> bool {
    let Some(os) = checked(path) else { return false };
    match target(true) {
        Target::Off => false,
        Target::Xbel(f) => xbel_add(&f, &os),
        Target::Native => native::add(&os),
    }
}

/// this app's recent files, newest first (at most max)
pub fn recent_list(max: usize) -> Vec<String> {
    let mut v = match target(false) {
        Target::Off => Vec::new(),
        Target::Xbel(f) => xbel_list(&f),
        Target::Native => native::list(max),
    };
    v.truncate(max);
    v
}

/// take a file out of this app's recent files
pub fn recent_remove(path: &str) -> bool {
    let Some(os) = crate::vfs::os_path(path).and_then(|p| crate::policy::normalize(&p)) else { return false };
    match target(true) {
        Target::Off => false,
        Target::Xbel(f) => xbel_remove(&f, Some(&os)),
        Target::Native => native::remove(&os),
    }
}

/// forget every recent file of this app
pub fn recent_clear() -> bool {
    match target(true) {
        Target::Off => false,
        Target::Xbel(f) => xbel_remove(&f, None),
        Target::Native => native::clear(),
    }
}

// ---------------------------------------------------------------- MIME types, times

const MIMES: &[(&str, &str)] = &[
    ("txt", "text/plain"), ("md", "text/markdown"), ("csv", "text/csv"), ("html", "text/html"), ("htm", "text/html"),
    ("xml", "application/xml"), ("json", "application/json"), ("pdf", "application/pdf"), ("zip", "application/zip"),
    ("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg"), ("gif", "image/gif"), ("webp", "image/webp"),
    ("svg", "image/svg+xml"), ("bmp", "image/bmp"), ("tif", "image/tiff"), ("tiff", "image/tiff"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("mp3", "audio/mpeg"), ("ogg", "audio/ogg"), ("mp4", "video/mp4"),
];

/// the MIME type of a file name: the app's file types, the table, application/octet-stream
pub(crate) fn mime_of(path: &str) -> String {
    if let Some(m) = crate::assoc::package().mime_of(path) {
        return m;
    }
    let name = &path[path.rfind(['/', '\\']).map_or(0, |k| k + 1)..];
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    String::from(MIMES.iter().find(|(e, _)| *e == ext).map_or("application/octet-stream", |(_, m)| *m))
}

/// days since 1970-01-01 -> (year, month, day)
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (year, month, day) -> days since 1970-01-01
fn days_of(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// nanoseconds since 1970 as GLib writes them: 2026-10-19T08:30:00.123456Z
pub(crate) fn iso_time(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let us = ns.rem_euclid(1_000_000_000) / 1000;
    let (y, mo, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{us:06}Z", t / 3600, t / 60 % 60, t % 60)
}

/// an ISO 8601 time in UTC (YYYY-MM-DDTHH:MM:SS[.fraction]Z) -> nanoseconds since 1970
pub(crate) fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: core::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let days = days_of(num(0..4)?, num(5..7)?, num(8..10)?);
    let mut ns = ((days * 24 + num(11..13)?) * 60 + num(14..16)?) * 60 + num(17..19)?;
    ns *= 1_000_000_000;
    if b[19] == b'.' {
        let frac: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).take(9).collect();
        let digits = frac.len() as u32;
        ns += frac.parse::<i64>().ok()? * 10i64.pow(9 - digits);
    }
    Some(ns)
}

// ---------------------------------------------------------------- the xbel file

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

fn xml_unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        o.push_str(&rest[..k]);
        rest = &rest[k..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16).ok()).or_else(|| ent.strip_prefix('#').map(|d| d.parse().ok()))
                .flatten().and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                o.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                o.push('&');
                rest = &rest[1..];
            }
        }
    }
    o.push_str(rest);
    o
}

/// the value of attribute name in an element's start tag
fn attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(k) = tag[from..].find(name) {
        let at = from + k;
        from = at + name.len();
        let before = tag[..at].chars().last();
        if !before.is_some_and(|c| c.is_ascii_whitespace()) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let q = rest.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = rest[1..].find(q)?;
        return Some(xml_unescape(&rest[1..1 + end]));
    }
    None
}

/// the start tag with attribute name = value (replaced, else added before the end)
fn attr_set(tag: &str, name: &str, value: &str) -> String {
    let v = xml_escape(value);
    let mut from = 0;
    while let Some(k) = tag[from..].find(name) {
        let at = from + k;
        from = at + name.len();
        if !tag[..at].chars().last().is_some_and(|c| c.is_ascii_whitespace()) {
            continue;
        }
        let rest = &tag[from..];
        let eq = rest.len() - rest.trim_start().len();
        let Some(after) = rest[eq..].strip_prefix('=') else { continue };
        let pad = after.len() - after.trim_start().len();
        let vstart = from + eq + 1 + pad;
        let q = tag[vstart..].chars().next().unwrap_or('"');
        let Some(len) = tag[vstart + 1..].find(q) else { break };
        return format!("{}\"{v}\"{}", &tag[..vstart], &tag[vstart + 1 + len + 1..]);
    }
    let end = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
    format!("{} {name}=\"{v}\"{}", tag[..end].trim_end(), &tag[end..])
}

/// a <bookmark> element: its byte range in the file
struct Block {
    start: usize,
    end: usize,
}

/// the start of an element named name ("<name" + blank, '>' or "/>") from at
fn find_elem(text: &str, name: &str, at: usize) -> Option<usize> {
    let pat = format!("<{name}");
    let mut from = at;
    while let Some(k) = text[from..].find(&pat) {
        let s = from + k;
        let next = text[s + pat.len()..].chars().next();
        if next.is_some_and(|c| c.is_ascii_whitespace() || c == '>' || c == '/') {
            return Some(s);
        }
        from = s + pat.len();
    }
    None
}

/// the end of the start tag that begins at s (after its '>')
fn tag_end(text: &str, s: usize) -> Option<usize> {
    let mut q: Option<char> = None;
    for (i, c) in text[s..].char_indices() {
        match (q, c) {
            (None, '"' | '\'') => q = Some(c),
            (Some(o), c) if c == o => q = None,
            (None, '>') => return Some(s + i + 1),
            _ => {}
        }
    }
    None
}

/// the bookmarks of the file, None when it is no xbel document
fn blocks(text: &str) -> Option<Vec<Block>> {
    let root = find_elem(text, "xbel", 0)?;
    text.rfind("</xbel>")?;
    let mut out = Vec::new();
    let mut at = tag_end(text, root)?;
    while let Some(s) = find_elem(text, "bookmark", at) {
        let te = tag_end(text, s)?;
        let end = if text[..te].ends_with("/>") { te } else { text[te..].find("</bookmark>").map(|k| te + k + "</bookmark>".len())? };
        out.push(Block { start: s, end });
        at = end;
    }
    Some(out)
}

/// the href of a block as a path, decoded (comparable whatever the escaping)
fn block_path(b: &str) -> Option<String> {
    let te = tag_end(b, 0)?;
    file_uri_path(&attr(&b[..te], "href")?)
}

/// this app's <bookmark:application> element in a block: its byte range
fn app_elem(b: &str, app: &str) -> Option<(usize, usize)> {
    let mut at = 0;
    while let Some(s) = find_elem(b, "bookmark:application", at) {
        let te = tag_end(b, s)?;
        let end = if b[..te].ends_with("/>") {
            te
        } else {
            b[te..].find("</bookmark:application>").map_or(te, |k| te + k + "</bookmark:application>".len())
        };
        if attr(&b[s..te], "name").as_deref() == Some(app) {
            return Some((s, end));
        }
        at = end;
    }
    None
}

/// the shell-quoted command line GLib keeps in exec ('prog %u')
fn exec_attr() -> String {
    #[cfg(any(windows, target_os = "linux"))]
    let exe = crate::env("APPIMAGE").filter(|p| p.starts_with('/')).or_else(crate::current_exe).unwrap_or_default();
    #[cfg(not(any(windows, target_os = "linux")))]
    let exe = crate::vfs::app_id();
    format!("'{} %u'", exe.replace('\'', "'\\''"))
}

fn app_name() -> String {
    crate::vfs::app_id()
}

/// the lock of an xbel file: a sidecar in $XDG_RUNTIME_DIR, else beside it
fn lock(file: &str) -> Option<crate::OsFileLock> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in file.as_bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let path = match crate::env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/') && cfg!(target_os = "linux")) {
        Some(d) => format!("{}/jflib-recent-{h:016x}.lock", d.trim_end_matches('/')),
        None => format!("{file}.lock"),
    };
    let end = crate::time_us() + LOCK_WAIT_MS as u64 * 1000;
    loop {
        if let Some(l) = crate::OsFileLock::acquire_raw(&path) {
            return Some(l);
        }
        if crate::time_us() > end {
            return None;
        }
        crate::sleep_ms(5);
    }
}

const XBEL_EMPTY: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      \
xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      \
xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n</xbel>\n";

/// change the file under its lock: f(text) -> Some(new text) to write it (None: unchanged)
fn xbel_edit(file: &str, f: impl FnOnce(&str) -> Option<String>) -> bool {
    let Some(_lock) = lock(file) else { return false };
    let text = match crate::file_exists_raw(file) {
        0 => String::from(XBEL_EMPTY),
        1 => match crate::read_file_raw(file).and_then(|d| String::from_utf8(d).ok()) {
            Some(t) if t.trim().is_empty() => String::from(XBEL_EMPTY), // an empty file: begun anew
            Some(t) => t,
            None => return false,
        },
        _ => return false,
    };
    if blocks(&text).is_none() {
        return false; // never replace a file we cannot read
    }
    match f(&text) {
        Some(new) => {
            let dir = &file[..file.rfind(['/', '\\']).unwrap_or(0)];
            (dir.is_empty() || crate::mkdir_raw(dir)) && crate::write_file_raw(file, new.as_bytes())
        }
        None => true,
    }
}

fn app_elem_text(app: &str, exec: &str, now: &str, count: u64) -> String {
    format!("<bookmark:application name=\"{}\" exec=\"{}\" modified=\"{now}\" count=\"{count}\"/>", xml_escape(app), xml_escape(exec))
}

fn xbel_add(file: &str, path: &str) -> bool {
    let (app, exec) = (app_name(), exec_attr());
    let now = iso_time(crate::unix_time_ns());
    let uri = file_uri(path);
    let mime = mime_of(path);
    xbel_edit(file, |text| {
        let list = blocks(text)?;
        let found = list.iter().find(|b| block_path(&text[b.start..b.end]).as_deref() == Some(path));
        let Some(b) = found else {
            let block = format!(
                "  <bookmark href=\"{}\" added=\"{now}\" modified=\"{now}\" visited=\"{now}\">\n    <info>\n      \
<metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{}\"/>\n        <bookmark:applications>\n          \
{}\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
                xml_escape(&uri), xml_escape(&mime), app_elem_text(&app, &exec, &now, 1)
            );
            let end = text.rfind("</xbel>")?;
            let line = text[..end].rfind('\n').map_or(end, |k| k + 1);
            let at = if text[line..end].trim().is_empty() { line } else { end };
            return Some(format!("{}{block}{}", &text[..at], &text[at..]));
        };
        let old = &text[b.start..b.end];
        let te = tag_end(old, 0)?;
        let mut head = attr_set(&old[..te], "modified", &now);
        head = attr_set(&head, "visited", &now);
        let mut body = String::from(&old[te..]);
        match app_elem(&body, &app) {
            Some((s, e)) => {
                let count = attr(&body[s..e], "count").and_then(|c| c.parse::<u64>().ok()).unwrap_or(0) + 1;
                body = format!("{}{}{}", &body[..s], app_elem_text(&app, &exec, &now, count), &body[e..]);
            }
            None => {
                let el = app_elem_text(&app, &exec, &now, 1);
                // GLib's layout: applications at 8 blanks, each application at 10
                if let Some(k) = body.find("</bookmark:applications>") {
                    body = format!("{}          {el}\n        {}", body[..k].trim_end_matches(' '), &body[k..]);
                } else if let Some(k) = body.find("</metadata>") {
                    let list = format!("        <bookmark:applications>\n          {el}\n        </bookmark:applications>\n      ");
                    body = format!("{}{list}{}", body[..k].trim_end_matches(' '), &body[k..]);
                } else {
                    return None; // a bookmark of another layout: left alone
                }
            }
        }
        Some(format!("{}{head}{body}{}", &text[..b.start], &text[b.end..]))
    })
}

fn xbel_list(file: &str) -> Vec<String> {
    let app = app_name();
    let Some(text) = crate::read_file_raw(file).and_then(|d| String::from_utf8(d).ok()) else { return Vec::new() };
    let Some(list) = blocks(&text) else { return Vec::new() };
    let mut out: Vec<(i64, String)> = Vec::new();
    for b in list {
        let bt = &text[b.start..b.end];
        let Some((s, e)) = app_elem(bt, &app) else { continue };
        let Some(path) = block_path(bt) else { continue };
        let el = &bt[s..e];
        let when = attr(el, "modified").and_then(|t| parse_time(&t))
            .or_else(|| attr(el, "timestamp").and_then(|t| t.parse::<i64>().ok()).map(|t| t * 1_000_000_000))
            .or_else(|| tag_end(bt, 0).and_then(|te| attr(&bt[..te], "modified")).and_then(|t| parse_time(&t)))
            .unwrap_or(0);
        out.push((when, path));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, p)| p).collect()
}

fn xbel_remove(file: &str, path: Option<&str>) -> bool {
    if crate::file_exists_raw(file) == 0 {
        return true;
    }
    let app = app_name();
    xbel_edit(file, |text| {
        let list = blocks(text)?;
        let mut out = String::new();
        let mut at = 0;
        let mut changed = false;
        for b in list {
            let bt = &text[b.start..b.end];
            if path.is_some_and(|p| block_path(bt).as_deref() != Some(p)) {
                continue;
            }
            let Some((s, e)) = app_elem(bt, &app) else { continue };
            changed = true;
            // the element with its line (indentation and line end)
            let ls = bt[..s].rfind('\n').map_or(s, |k| k + 1);
            let ls = if bt[ls..s].trim().is_empty() { ls } else { s };
            let le = if bt[e..].starts_with('\n') { e + 1 } else { e };
            let rest = format!("{}{}", &bt[..ls], &bt[le..]);
            out.push_str(&text[at..b.start]);
            if find_elem(&rest, "bookmark:application", 0).is_some() {
                out.push_str(&rest);
                at = b.end;
            } else {
                // no program left: the bookmark goes, with its line
                let ls = out.rfind('\n').map_or(out.len(), |k| k + 1);
                if out[ls..].trim().is_empty() {
                    out.truncate(ls);
                }
                at = if text[b.end..].starts_with('\n') { b.end + 1 } else { b.end };
            }
        }
        if !changed {
            return None;
        }
        out.push_str(&text[at..]);
        Some(out)
    })
}

#[cfg(windows)]
#[path = "win_recent.rs"]
mod native;

#[cfg(not(windows))]
mod native {
    use super::Target;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Linux: $XDG_DATA_HOME/recently-used.xbel (in a Flatpak the host's when it is shared)
    pub fn target() -> Target {
        #[cfg(target_os = "linux")]
        {
            if crate::linux_desk::in_flatpak() {
                if let Some(home) = crate::env("HOME").filter(|h| h.starts_with('/')) {
                    let host = alloc::format!("{}/.local/share/recently-used.xbel", home.trim_end_matches('/'));
                    if crate::file_exists_raw(&host) == 1 {
                        return Target::Xbel(host);
                    }
                }
            }
            if let Some(d) = crate::data_dir() {
                return Target::Xbel(alloc::format!("{d}/recently-used.xbel"));
            }
        }
        Target::Off
    }
    pub fn add(_: &str) -> bool {
        false
    }
    pub fn list(_: usize) -> Vec<String> {
        Vec::new()
    }
    pub fn remove(_: &str) -> bool {
        false
    }
    pub fn clear() -> bool {
        false
    }
}
