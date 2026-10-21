// t_recent -- the system's list of recently used files (lib/plat/recent.rs)
//
//   plattest --test recent      through jfgui_RECENT (a scratch .xbel, on every OS): a new list,
//                               adding twice (one bookmark, count 2), MIME types, escaped names,
//                               the order (newest first), remove, a GLib-written file (other
//                               programs' bookmarks byte for byte, a bookmark shared with another
//                               program: our entry added and removed, the bookmark stays), clear,
//                               a damaged file left alone, jfgui_RECENT=off, recent_os(false) (the
//                               headless runs), well-formed XML (python's minidom when there);
//                               Linux also the default $XDG_DATA_HOME/recently-used.xbel in a
//                               scratch folder.  At the end a manifest in this process: a path
//                               outside fs.read is refused (PERMDENY fs.read)
//   plattest --test recent-win  (Windows, by hand) the real shell: plattest the registered and
//                               default handler of a test type, a window on the taskbar,
//                               SHAddToRecentDocs (the link in Explorer's recent folder), what the
//                               jump list holds (printed), remove, clear; the test type
//                               unregistered and the links removed.  Explorer's own list (Home /
//                               Quick access) keeps the test file: remove it there by hand

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use plat::{recent_add, recent_clear, recent_list, recent_remove};

use crate::T;

fn scratch(t: &mut T, tag: &str) -> String {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-{tag}-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&base), &format!("{tag}: scratch folder"));
    base
}

fn read(path: &str) -> String {
    String::from_utf8(plat::read_file(path).unwrap_or_default()).unwrap_or_default()
}

/// the file:// URI GLib writes for a path without characters to escape
fn uri(path: &str) -> String {
    format!("file://{}{path}", if path.starts_with('/') { "" } else { "/" })
}

fn glib_file(shared: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      \
xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      \
xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n{OFFICE}{}</xbel>",
        shared_block(shared)
    )
}

const OFFICE: &str = "  <bookmark href=\"file:///home/u/Dokumente/Brief%20an%20Oma.odt\" added=\"2026-10-01T10:00:00.000000Z\" \
modified=\"2026-10-01T10:00:00.000000Z\" visited=\"2026-10-01T10:00:00.000000Z\">\n    <info>\n      \
<metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"application/vnd.oasis.opendocument.text\"/>\n        \
<bookmark:groups>\n          <bookmark:group>LibreOffice</bookmark:group>\n        </bookmark:groups>\n        \
<bookmark:applications>\n          <bookmark:application name=\"LibreOffice 7.6\" exec=\"&apos;soffice %u&apos;\" \
modified=\"2026-10-01T10:00:00Z\" count=\"3\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n";

fn shared_block(path: &str) -> String {
    format!(
        "  <bookmark href=\"{}\" added=\"2026-10-02T09:00:00.000000Z\" modified=\"2026-10-02T09:00:00.000000Z\" \
visited=\"2026-10-02T09:00:00.000000Z\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        \
<mime:mime-type type=\"text/plain\"/>\n        <bookmark:applications>\n          \
<bookmark:application name=\"gedit\" exec=\"&apos;gedit %u&apos;\" modified=\"2026-10-02T09:00:00Z\" count=\"1\"/>\n        \
</bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
        uri(path)
    )
}

/// python's XML parser accepts the file (None: no python)
fn well_formed(path: &str) -> Option<bool> {
    let py = if cfg!(windows) { "python" } else { "python3" };
    let cmd = plat::process::Command {
        program: py,
        args: &["-c", "import sys,xml.dom.minidom;xml.dom.minidom.parse(sys.argv[1])", path],
        cwd: None,
        env: &[],
    };
    let mut child = cmd.spawn().ok()?;
    let end = plat::time_us() + 30_000_000;
    let mut buf = [0u8; 1024];
    loop {
        while let Ok(Some(n)) = child.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
        }
        match child.try_wait() {
            Ok(Some(c)) => return Some(c == 0),
            Ok(None) if plat::time_us() < end => plat::sleep_ms(10),
            _ => return None,
        }
    }
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

pub fn t_recent(t: &mut T) {
    let base = scratch(t, "recent");
    let saved = plat::env("jfgui_RECENT");
    let xbel = format!("{base}/recent.xbel");
    plat::setenv("jfgui_RECENT", Some(&xbel));
    let (a, b, c) = (format!("{base}/a.txt"), format!("{base}/b.png"), format!("{base}/ä b.xyz"));
    for f in [&a, &b, &c] {
        plat::write_file(f, b"x");
    }
    t.check(recent_list(10).is_empty(), "recent: no list yet");
    t.check(recent_add(&a), "recent: added to a new list");
    let text = read(&xbel);
    t.check(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"")
        && text.contains(&format!("<bookmark href=\"{}\"", uri(&a))) && text.contains("<mime:mime-type type=\"text/plain\"/>")
        && text.contains("<bookmark:application name=\"plattest\" exec=\"&apos;") && text.contains("count=\"1\"/>"),
        "recent: the bookmark (href, MIME type, the app's entry)");
    t.info(&format!("  {}\n", text.trim_end().replace('\n', "\n  ")));
    t.check(recent_add(&a) && count(&read(&xbel), "<bookmark href=") == 1 && read(&xbel).contains("count=\"2\"/>"), "recent: added again: one bookmark, count 2");
    plat::sleep_ms(2);
    t.check(recent_add(&b) && recent_list(10) == [b.clone(), a.clone()], "recent: the newest first");
    plat::sleep_ms(2);
    t.check(recent_add(&a) && recent_list(10) == [a.clone(), b.clone()], "recent: added again: first");
    t.check(read(&xbel).contains("<mime:mime-type type=\"image/png\"/>"), "recent: image/png from the table");
    plat::sleep_ms(2);
    t.check(recent_add(&c) && read(&xbel).contains("%C3%A4%20b.xyz\"") && read(&xbel).contains("type=\"application/octet-stream\"")
        && recent_list(10).first() == Some(&c), "recent: a name with blank and umlaut (escaped, read back)");
    t.check(recent_list(1).len() == 1, "recent: at most max");
    t.check(recent_remove(&b) && recent_list(10) == [c.clone(), a.clone()] && !read(&xbel).contains("b.png"), "recent: removed");
    t.check(recent_remove(&b), "recent: removing a missing file is fine");
    t.check(well_formed(&xbel) != Some(false), "recent: well-formed XML");

    // a file GLib wrote: other programs' bookmarks stay byte for byte
    let shared = format!("{base}/shared.txt");
    plat::write_file(&shared, b"x");
    let glib = glib_file(&shared);
    plat::write_file(&xbel, glib.as_bytes());
    t.check(recent_list(10).is_empty(), "recent: the GLib file has none of ours");
    t.check(recent_add(&shared), "recent: a bookmark of another program: ours added");
    let text = read(&xbel);
    t.check(text.contains(OFFICE)
        && text.contains("<bookmark:application name=\"gedit\" exec=\"&apos;gedit %u&apos;\" modified=\"2026-10-02T09:00:00Z\" count=\"1\"/>\n          <bookmark:application name=\"plattest\"")
        && count(&text, "<bookmark href=") == 2, "recent: the other bookmark unchanged, the shared one has both programs (GLib's layout)");
    t.check(recent_list(10) == [shared.clone()], "recent: the shared file is in our list");
    t.check(well_formed(&xbel) != Some(false), "recent: well-formed after the change");
    t.check(recent_remove(&shared), "recent: removed from the shared bookmark");
    let text = read(&xbel);
    let block = shared_block(&shared);
    let inner = &block[block.find("    <info>").unwrap_or(0)..];
    t.check(text.contains(OFFICE) && text.contains(inner) && !text.contains("name=\"plattest\"") && count(&text, "<bookmark href=") == 2,
        "recent: the bookmark stays for the other program, its content as before");
    t.check(recent_add(&a) && recent_add(&shared) && recent_clear(), "recent: cleared");
    let text = read(&xbel);
    t.check(text.contains(OFFICE) && !text.contains("name=\"plattest\"") && !text.contains("a.txt") && count(&text, "<bookmark href=") == 2
        && recent_list(10).is_empty(), "recent: clear removes only ours");
    t.check(text.ends_with("</xbel>") && well_formed(&xbel) != Some(false), "recent: well-formed after clear");

    // a damaged file is never replaced
    plat::write_file(&xbel, b"not an xbel file");
    t.check(!recent_add(&a) && read(&xbel) == "not an xbel file" && recent_list(10).is_empty(), "recent: a damaged file is left alone");
    plat::write_file(&xbel, b"");
    t.check(recent_add(&a) && recent_list(10) == [a.clone()], "recent: an empty file is begun anew");
    plat::delete(&xbel);
    plat::setenv("jfgui_RECENT", Some("off"));
    t.check(!recent_add(&a) && recent_list(10).is_empty() && plat::file_exists(&xbel) == 0, "recent: jfgui_RECENT=off: nothing");

    // headless runs: the system's list stays, a redirection still works
    plat::recent::recent_os(false);
    plat::setenv("jfgui_RECENT", Some(&xbel));
    t.check(recent_add(&a), "recent: recent_os(false): jfgui_RECENT still written");
    plat::setenv("jfgui_RECENT", None);
    #[cfg(not(windows))]
    let data = plat::env("XDG_DATA_HOME");
    #[cfg(not(windows))]
    plat::setenv("XDG_DATA_HOME", Some(&format!("{base}/data")));
    t.check(!recent_add(&a) && !recent_clear(), "recent: recent_os(false): the system's list is left alone");
    plat::recent::recent_os(true);
    #[cfg(not(windows))]
    {
        let def = format!("{base}/data/recently-used.xbel");
        t.check(plat::file_exists(&def) == 0, "recent: nothing written while off");
        t.check(recent_add(&a) && plat::file_exists(&def) == 1 && recent_list(10) == [a.clone()], "recent: Linux: $XDG_DATA_HOME/recently-used.xbel");
        t.check(recent_clear() && recent_list(10).is_empty(), "recent: Linux: cleared");
        plat::setenv("XDG_DATA_HOME", data.as_deref());
    }

    // the app's own list (ui recent.rs) in a headless Ui: kept in memory, the system's list told
    plat::setenv("jfgui_RECENT", Some(&xbel));
    plat::delete(&xbel);
    ui_list(t, &base, &[&a, &b, &c]);

    // the permissions: only files the app may read
    plat::mkdir(&format!("{base}/in"));
    let inside = format!("{base}/in/ok.txt");
    plat::write_file(&inside, b"x");
    match plat::policy::install(&format!("fs.read {base}/in/**\n")) {
        Ok(key) => {
            t.check(recent_add(&inside), "recent: manifest: a readable file is added");
            t.check(!recent_add(&a), "recent: manifest: a file outside fs.read is refused");
            t.check(plat::policy::denials().iter().any(|d| d.kind == "fs.read" && d.target.ends_with("/a.txt")), "recent: manifest: PERMDENY fs.read");
            key.trusted(|| crate::rm_tree(&base));
        }
        Err(e) => {
            t.check(false, &format!("recent: manifest installed ({e})"));
            crate::rm_tree(&base);
        }
    }
    plat::setenv("jfgui_RECENT", saved.as_deref());
}

/// ui.recent_add / recent_files / recent_remove / recent_clear inside a headless Ui
fn ui_list(t: &mut T, base: &str, files: &[&String; 3]) {
    struct Probe {
        files: [String; 3],
        out: Rc<RefCell<Vec<(bool, &'static str)>>>,
    }
    impl ui::App for Probe {
        fn init(&mut self, ui: &mut ui::Ui) -> bool {
            let [a, b, c] = &self.files;
            let mut r = Vec::new();
            r.push((ui.recent_files().is_empty(), "recent: ui: the app's list starts empty (headless: no file read)"));
            ui.recent_add(a);
            ui.recent_add(b);
            let (dir, name) = a.rsplit_once('/').unwrap_or(("", a));
            ui.recent_add(&format!("{dir}/./{name}"));
            r.push((ui.recent_files() == [a.clone(), b.clone()], "recent: ui: newest first, another spelling of a file is no new entry"));
            for k in 0..ui::RECENT_KEEP + 3 {
                ui.recent_add(&format!("{}/n{k}.txt", &a[..a.rfind('/').unwrap_or(0)]));
            }
            r.push((ui.recent_files().len() == ui::RECENT_KEEP, "recent: ui: at most RECENT_KEEP"));
            ui.recent_add(c);
            ui.recent_remove(c);
            r.push((ui.recent_files().first() != Some(c), "recent: ui: removed"));
            r.push((plat::recent_list(100).contains(a) && !plat::recent_list(100).contains(c), "recent: ui: the system's list follows"));
            ui.recent_clear();
            r.push((ui.recent_files().is_empty() && plat::recent_list(100).is_empty(), "recent: ui: cleared, also in the system's list"));
            *self.out.borrow_mut() = r;
            true
        }
        fn frame(&mut self, _: &mut ui::Ui) {}
    }
    let saved = plat::env(crate::CFGVAR);
    plat::setenv(crate::CFGVAR, Some(&format!("{base}/uicfg")));
    let out = Rc::new(RefCell::new(Vec::new()));
    let probe = Probe { files: [files[0].clone(), files[1].clone(), files[2].clone()], out: out.clone() };
    let desc = ui::AppDesc { title: "recent", w: 64, h: 64, ..ui::AppDesc::NONE };
    t.check(ui::Runtime::new(&desc, probe, &style::STYLE).is_ok(), "recent: ui: a headless Ui");
    let results = out.borrow().clone();
    for (ok, what) in results {
        t.check(ok, what);
    }
    t.check(plat::file_exists(&format!("{base}/uicfg/plattest/recent.list")) == 0, "recent: ui: headless: config://recent.list not written");
    plat::setenv(crate::CFGVAR, saved.as_deref());
}

#[cfg(not(windows))]
pub fn t_recent_win(_: &mut T) {
    plat::write_stderr(b"plattest: --test recent-win is Windows only\n");
    plat::exit(2);
}

#[cfg(windows)]
pub fn t_recent_win(t: &mut T) {
    use plat::assoc::{Package, assoc_register, assoc_unregister};
    use plat::{PW_DEFAULT, PW_NOOWNER, PWF_RESIZABLE, PWF_SYSFRAME, PWS_SHOW, evq_pop, time_us, wait, win_create, win_destroy, win_show};
    let pump = |ms: u64| {
        let dl = time_us() + ms * 1000;
        while time_us() < dl {
            wait(dl);
            while evq_pop().is_some() {}
        }
    };
    let real_appdata = plat::env("APPDATA").unwrap_or_default();
    let base = scratch(t, "recent-win");
    plat::setenv("APPDATA", Some(&format!("{base}/config")));
    plat::setenv("jfgui_RECENT", None);
    // plattest the registered and default handler of a test type, with a window on the taskbar
    let pkg = Package::parse("display=JFLib Recent Test\nfiletypes=jflrecent|application/x-jflib-recent|JFLib-Recent-Test|own\n");
    t.check(assoc_register(&pkg, &mut |_| false).is_ok(), "recent-win: test type .jflrecent registered for plattest");
    t.check(win_create("plattest recent", 360, 120, PWF_RESIZABLE | PWF_SYSFRAME, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT) == 0, "recent-win: the main window");
    win_show(0, PWS_SHOW);
    pump(1000);
    let name = format!("recent-{:x}.jflrecent", plat::time_us() & 0xFF_FFFF);
    let file = format!("{base}/{name}");
    plat::write_file(&file, b"x");
    let recent = format!("{real_appdata}/Microsoft/Windows/Recent");
    let lnk = format!("{recent}/{name}.lnk");
    t.check(recent_add(&file), "recent-win: SHAddToRecentDocs");
    let mut made = false;
    for _ in 0..100 {
        made = plat::file_exists(&lnk) == 1;
        if made {
            break;
        }
        pump(50);
    }
    t.check(made, "recent-win: the link in Explorer's recent folder");
    pump(1000);
    let list = recent_list(100);
    let mine = list.iter().any(|x| x.eq_ignore_ascii_case(&file));
    t.info(&format!("  the jump list's recent items: {} ({})\n", list.len(),
        if mine { "the test file is there" } else { "Windows did not give the file to plattest's jump list" }));
    t.check(recent_remove(&file) && !recent_list(100).iter().any(|x| x.eq_ignore_ascii_case(&file)), "recent-win: removed from the jump list");
    t.check(recent_clear() && recent_list(100).is_empty(), "recent-win: jump list cleared");
    win_destroy(0);
    pump(200);
    t.check(assoc_unregister() == Ok(true), "recent-win: test type unregistered");
    let folder = base.rsplit('/').next().unwrap_or("");
    for l in [lnk, format!("{recent}/{folder}.lnk")] {
        if plat::file_exists(&l) == 1 {
            t.check(plat::delete(&l), &format!("recent-win: {} removed", &l[recent.len() + 1..]));
        }
    }
    t.info(&format!("  Explorer's recent files (Home / Quick access) keep {name}: remove it there (\"Aus Liste entfernen\")\n"));
    plat::setenv("APPDATA", Some(&real_appdata));
    crate::rm_tree(&base);
}
