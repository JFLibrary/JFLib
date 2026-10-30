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
//                               scratch folder.  Windows: the explicit AppUserModelID (refused
//                               ids, app_id_boot: jfgui_AUMID, PACKAGE without and with an
//                               install.ini beside the program), shortcuts with the id
//                               (shell_link_id, link_read, link_set_app_id), the pins of the
//                               program without an id get it (pins_set_app_id in a scratch
//                               folder: other programs' and other ids' links stay), the ProgID's
//                               AppUserModelID (reg_entries).  At the end a manifest in this
//                               process: a path outside fs.read is refused (PERMDENY fs.read)
//   plattest --test recent-win  (Windows, by hand; no window) the real shell: a test type
//                               registered for plattest, first with the id Windows derives
//                               (SHARD_PATHW), then with the explicit id of a registration
//                               (app_id_boot through the record, SHARD_APPIDINFO, AppUserModelID in
//                               the ProgID): the link in Explorer's recent folder, the id's jump
//                               list file (AutomaticDestinations/<CRC-64 of the id>), whether the
//                               file got into it (printed: on Windows 11 26200 the list stays
//                               empty, GUIDE.md section 21), remove, clear; the test type
//                               unregistered, the links, the test's jump list files and its
//                               entries in Explorer's jump list removed.  Explorer's Home (Quick
//                               access) keeps the test files: remove them there by hand

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

    #[cfg(windows)]
    app_id(t, &base);

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
            let gone = format!("{}/gone.txt", &a[..a.rfind('/').unwrap_or(0)]);
            r.push((ui.recent_open(a) == ui::RecentOpen::Ready && ui.recent_open(&gone) == ui::RecentOpen::Missing
                && ui.recent_reopened().is_none(), "recent: ui: recent_open: Ready for a file that is there, Missing for one that is gone"));
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

/// Windows: the explicit AppUserModelID, shortcuts and pins that carry it (scratch folders only)
#[cfg(windows)]
fn app_id(t: &mut T, base: &str) {
    use plat::assoc::{Package, app_id_boot, package_set, parse_types, reg_entries};
    use plat::inst::{link_read, link_set_app_id, pins_set_app_id, shell_link, shell_link_id};
    use plat::taskbar::{app_user_model_id, set_app_user_model_id};
    let long: String = core::iter::repeat_n('x', 129).collect();
    t.check(!set_app_user_model_id("") && !set_app_user_model_id("a b") && !set_app_user_model_id(&long), "recent: aumid: empty, blank, too long refused");
    let saved_cfg = plat::env(crate::CFGVAR);
    plat::setenv(crate::CFGVAR, Some(&format!("{base}/idcfg"))); // no assoc record of plattest
    plat::setenv("jfgui_AUMID", None);
    t.check(app_id_boot().is_none() && app_user_model_id().is_none(), "recent: aumid: no PACKAGE: Windows' own id");
    package_set(Some("display=Plattest\napp_id=org.jflib.plattest.Recent\n"));
    t.check(app_id_boot().is_none() && app_user_model_id().is_none(), "recent: aumid: PACKAGE, not installed, not registered: Windows' own id");
    let exe = plat::current_exe().unwrap_or_default().replace('/', "\\");
    let ini = format!("{}\\install.ini", &exe[..exe.rfind('\\').unwrap_or(0)]);
    if plat::file_exists(&ini) == 0 {
        plat::write_file(&ini, b"app=plattest\r\napp_id=org.jflib.plattest.Other\r\n");
        t.check(app_id_boot().is_none(), "recent: aumid: install.ini of another id: none");
        plat::write_file(&ini, b"app=plattest\r\ndisplay=Plattest\r\napp_id=org.jflib.plattest.Recent\r\n");
        let got = app_id_boot();
        plat::delete(&ini);
        t.check(got.as_deref() == Some("org.jflib.plattest.Recent") && app_user_model_id() == got,
            &format!("recent: aumid: installed (install.ini beside the program): the app id ({got:?}, the shell: {:?})", app_user_model_id()));
    } else {
        t.info("  (an install.ini beside plattest: the installed case is not checked)\n");
    }
    plat::setenv("jfgui_AUMID", Some("off"));
    t.check(app_id_boot().is_none(), "recent: aumid: jfgui_AUMID=off: none");
    plat::setenv("jfgui_AUMID", Some("org.jflib.plattest.Env"));
    t.check(app_id_boot().as_deref() == Some("org.jflib.plattest.Env") && app_user_model_id().as_deref() == Some("org.jflib.plattest.Env"),
        "recent: aumid: jfgui_AUMID=ID");
    plat::setenv("jfgui_AUMID", None);
    package_set(None);
    plat::setenv(crate::CFGVAR, saved_cfg.as_deref());

    // shortcuts with the id; the pins of this program without one get it
    let dir = base.replace('/', "\\");
    let lnk = format!("{dir}\\with-id.lnk");
    t.check(shell_link_id(&lnk, &exe, &dir, "Test", "org.jflib.plattest.Link"), "recent: aumid: shortcut with an id written");
    let back = link_read(&lnk);
    t.check(back.as_ref().is_some_and(|(p, i)| p.eq_ignore_ascii_case(&exe) && i == "org.jflib.plattest.Link"), &format!("recent: aumid: its target and id read back ({back:?})"));
    let plain = format!("{dir}\\plain.lnk");
    t.check(shell_link(&plain, &exe, &dir, "Test") && link_read(&plain).is_some_and(|(_, i)| i.is_empty()), "recent: aumid: shortcut without an id");
    t.check(link_set_app_id(&plain, "org.jflib.plattest.Set") && link_read(&plain).is_some_and(|(_, i)| i == "org.jflib.plattest.Set"), "recent: aumid: id written into a shortcut");
    t.check(link_read(&format!("{dir}\\missing.lnk")).is_none(), "recent: aumid: no shortcut: None");
    let pins = format!("{dir}\\Pinned");
    let deep = format!("{pins}\\ImplicitAppShortcuts\\0123abcd");
    plat::mkdir(&format!("{pins}\\TaskBar"));
    plat::mkdir(&deep);
    let other = plat::env("SystemRoot").map(|r| format!("{r}\\notepad.exe")).unwrap_or_else(|| String::from("C:\\Windows\\notepad.exe"));
    let made = shell_link(&format!("{pins}\\TaskBar\\Plattest.lnk"), &exe, &dir, "")
        && shell_link(&format!("{deep}\\Plattest.lnk"), &exe, &dir, "")
        && shell_link(&format!("{pins}\\TaskBar\\Notepad.lnk"), &other, "", "")
        && shell_link_id(&format!("{pins}\\TaskBar\\Mine.lnk"), &exe, &dir, "", "org.jflib.plattest.Kept");
    t.check(made, "recent: aumid: pins in a scratch folder");
    let n = pins_set_app_id(&pins, &exe, "org.jflib.plattest.Pin");
    let id_of = |p: &str| link_read(&format!("{pins}\\{p}")).map(|(_, i)| i).unwrap_or_else(|| String::from("?"));
    let ids = [id_of("TaskBar\\Plattest.lnk"), id_of("ImplicitAppShortcuts\\0123abcd\\Plattest.lnk"), id_of("TaskBar\\Notepad.lnk"), id_of("TaskBar\\Mine.lnk")];
    t.check(n == 2 && ids[0] == "org.jflib.plattest.Pin" && ids[1] == "org.jflib.plattest.Pin" && ids[2].is_empty() && ids[3] == "org.jflib.plattest.Kept",
        &format!("recent: aumid: the program's pins without an id got it, others stay ({n}: {ids:?})"));
    t.check(pins_set_app_id(&pins, &exe, "org.jflib.plattest.Pin") == 0, "recent: aumid: pins set again: nothing to do");
    let user = plat::inst::pinned_dir();
    t.check(user.as_deref().is_some_and(|d| d.ends_with("\\User Pinned")), &format!("recent: aumid: the user's pinned items ({user:?}; not changed)"));

    // the ProgIDs name the id
    let types = parse_types("jflid|application/x-jflid|JFL-Id|own");
    let e = reg_entries("App", "My App", "C:\\a\\App.exe", &types, &[String::from("jflid")], "org.jflib.App");
    let id_val = e.iter().filter(|v| v.name == "AppUserModelID").map(|v| (v.key.as_str(), v.value.as_str(), v.root.as_str())).collect::<Vec<_>>();
    t.check(id_val == [("Software\\Classes\\App.jflid", "org.jflib.App", "Software\\Classes\\App.jflid")], &format!("recent: aumid: the ProgID's AppUserModelID ({id_val:?})"));
    t.check(!reg_entries("App", "My App", "C:\\a\\App.exe", &types, &[], "").iter().any(|v| v.name == "AppUserModelID"), "recent: aumid: no id, no AppUserModelID");
    let _ = Package::default();
}

#[cfg(not(windows))]
pub fn t_recent_win(_: &mut T) {
    plat::write_stderr(b"plattest: --test recent-win is Windows only\n");
    plat::exit(2);
}

/// the jump list file of an AppUserModelID (a derived one is the program's path): the CRC-64
/// (reflected, polynomial 0x92C64265D32139A4, start all ones) of the id in upper case as UTF-16
#[cfg(windows)]
fn dest_name(id: &str) -> String {
    let mut c = u64::MAX;
    for u in id.to_uppercase().encode_utf16() {
        for b in u.to_le_bytes() {
            c ^= b as u64;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0x92C6_4265_D321_39A4 } else { c >> 1 };
            }
        }
    }
    format!("{c:016x}.automaticDestinations-ms")
}

#[cfg(windows)]
pub fn t_recent_win(t: &mut T) {
    use plat::assoc::{Package, app_id_boot, assoc_register, assoc_unregister, hkcu_get, package_set};
    use plat::recent::recent_remove_of;
    use plat::{evq_pop, time_us, wait};
    const EXPLORER: &str = "Microsoft.Windows.Explorer";
    const ID: &str = "org.jflib.plattest.RecentTest";
    let pump = |ms: u64| {
        let dl = time_us() + ms * 1000;
        while time_us() < dl {
            wait(dl);
            while evq_pop().is_some() {}
        }
    };
    t.check(dest_name(EXPLORER) == "f01b4d95cf55d32a.automaticDestinations-ms", "recent-win: the jump list file of an id (Explorer's)");
    let real_appdata = plat::env("APPDATA").unwrap_or_default().replace('\\', "/");
    let base = scratch(t, "recent-win");
    // how long the jump list is watched for the file (ms)
    let wait_ms = plat::env("PLATTEST_RECENT_WAIT").and_then(|w| w.parse::<u64>().ok()).unwrap_or(2000);
    plat::setenv("APPDATA", Some(&format!("{base}/config")));
    plat::setenv("jfgui_RECENT", None);
    plat::setenv("jfgui_AUMID", None);
    let recent = format!("{real_appdata}/Microsoft/Windows/Recent");
    let dests = format!("{recent}/AutomaticDestinations");
    let exe = plat::current_exe().unwrap_or_default().replace('/', "\\");
    let mut links: Vec<String> = Vec::new();
    // the jump list files of the two ids (the derived one is the program's path): the test's
    // when they were not there before
    let ours: Vec<String> = [dest_name(&exe), dest_name(ID)].into_iter().filter(|n| plat::file_exists(&format!("{dests}/{n}")) == 0).collect();
    let text = format!("display=JFLib Recent Test\napp_id={ID}\nfiletypes=jflrecent|application/x-jflib-recent|JFLib-Recent-Test|own\n");
    let progid = "Software\\Classes\\plattest.jflrecent";

    // 1. the id Windows derives from the program: SHARD_PATHW, no AppUserModelID in the ProgID
    let mut pkg = Package::parse(&text);
    pkg.app_id = String::new();
    t.check(assoc_register(&pkg, &mut |_| false).is_ok() && hkcu_get(progid, Some("AppUserModelID")).is_none(),
        "recent-win: derived id: test type .jflrecent registered for plattest (no AppUserModelID)");
    for (phase, explicit) in [("derived id", false), ("explicit id", true)] {
        if explicit {
            // 2. a registration with the app id: the ProgID names it, the program runs under it
            let pkg = Package::parse(&text);
            t.check(assoc_register(&pkg, &mut |_| false).is_ok() && hkcu_get(progid, Some("AppUserModelID")).as_deref() == Some(ID),
                "recent-win: explicit id: registered again, the ProgID's AppUserModelID is the app id");
            package_set(Some(&text));
            let got = app_id_boot();
            t.check(got.as_deref() == Some(ID) && plat::taskbar::app_user_model_id().as_deref() == Some(ID),
                &format!("recent-win: explicit id: the record names plattest: it runs under the app id ({got:?})"));
        } else {
            t.check(plat::taskbar::app_user_model_id().is_none(), "recent-win: derived id: no explicit id");
        }
        let dest = format!("{dests}/{}", dest_name(if explicit { ID } else { &exe }));
        let name = format!("recent-{}-{:x}.jflrecent", if explicit { "id" } else { "derived" }, plat::time_us() & 0xFF_FFFF);
        let file = format!("{base}/{name}");
        plat::write_file(&file, b"x");
        let lnk = format!("{recent}/{name}.lnk");
        links.push(lnk.clone());
        let marker: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        t.check(recent_add(&file), &format!("recent-win: {phase}: SHAddToRecentDocs"));
        let mut made = false;
        for _ in 0..100 {
            made = plat::file_exists(&lnk) == 1;
            if made {
                break;
            }
            pump(50);
        }
        t.check(made, &format!("recent-win: {phase}: the link in Explorer's recent folder"));
        // the jump list of the id: Windows makes its file; whether the file gets in is printed
        let end = time_us() + wait_ms * 1000;
        let (mut mine, mut holds, mut size);
        loop {
            let data = plat::read_file(&dest);
            holds = data.as_ref().is_some_and(|d| d.windows(marker.len()).any(|w| w == marker.as_slice()));
            size = data.map(|d| d.len());
            mine = recent_list(100).iter().any(|x| x.eq_ignore_ascii_case(&file));
            if mine || time_us() >= end {
                break;
            }
            pump(250);
        }
        t.check(size.is_some(), &format!("recent-win: {phase}: SHAddToRecentDocs gave the item to the id's jump list ({})", &dest[dests.len() + 1..]));
        t.info(&format!("  {phase}: the jump list file: {size:?} bytes, holds the file: {holds}; recent_list has it: {mine}{}\n",
            if mine { "" } else { " (Windows keeps the list empty)" }));
        t.check(recent_remove(&file) && !recent_list(100).iter().any(|x| x.eq_ignore_ascii_case(&file)), &format!("recent-win: {phase}: removed from the jump list"));
        if mine {
            t.check(recent_add(&file), &format!("recent-win: {phase}: added again"));
            pump(1000);
            let back = recent_list(100).iter().any(|x| x.eq_ignore_ascii_case(&file));
            t.check(back, &format!("recent-win: {phase}: in the jump list again"));
        }
        t.check(recent_clear() && recent_list(100).is_empty(), &format!("recent-win: {phase}: jump list cleared"));
        // Explorer keeps the file in its own list: out of its jump list while it exists
        t.check(recent_remove_of(EXPLORER, &file), &format!("recent-win: {phase}: the test file out of Explorer's jump list"));
    }
    t.check(recent_remove_of(EXPLORER, &base), "recent-win: the test folder out of Explorer's jump list");
    package_set(None);
    t.check(assoc_unregister() == Ok(true) && hkcu_get(progid, None).is_none(), "recent-win: test type unregistered");
    let folder = String::from(base.rsplit('/').next().unwrap_or(""));
    links.push(format!("{recent}/{folder}.lnk"));
    for l in &links {
        if plat::file_exists(l) == 1 {
            t.check(plat::delete(l), &format!("recent-win: {} removed", &l[recent.len() + 1..]));
        }
    }
    for f in &ours {
        let path = format!("{dests}/{f}");
        if plat::file_exists(&path) == 1 {
            t.check(plat::delete(&path), &format!("recent-win: AutomaticDestinations/{f} (the test's) removed"));
        }
    }
    t.info(&format!("  Explorer's Home (Quick access) may still list {folder}/recent-*.jflrecent: remove them there\n"));
    plat::setenv("APPDATA", Some(&real_appdata));
    crate::rm_tree(&base);
}
