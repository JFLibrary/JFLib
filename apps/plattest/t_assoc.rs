// t_assoc -- the app's file types and URL schemes without a setup (lib/plat/assoc.rs,
// lib/ui/assoc.rs)
//
//   plattest --test assoc     the parts: parse_types / parse_schemes, reg_entries (the setup's
//                             keys), Package::parse, file_uri_path (also Request::paths), the
//                             switches (assoc_switch; plattest itself has no PACKAGE: --register
//                             in a child ends with exit code 2, --unregister with 0).  Then a
//                             test package (.jfltest own, .jfltest2 / .jfltest3 "Open with",
//                             scheme jflibtest) registered and unregistered, the record in a
//                             scratch config folder.  Windows: the real HKCU, only test names
//                             (removed first and at the end): the keys, registering twice, a
//                             default of another program kept / taken (dry run while
//                             registered) / put back, another program's ProgID of the same name
//                             kept / saved and put back.  Linux: scratch XDG_DATA_HOME /
//                             XDG_CONFIG_HOME / XDG_DATA_DIRS / XDG_CONFIG_DIRS: the .desktop
//                             file (desktop-file-validate when installed), the MIME XML
//                             (update-mime-database when installed: the glob in globs2),
//                             mimeapps.list, another program as implicit / explicit default
//                             (kept, taken, the file byte for byte back), a .desktop file of the
//                             same name of another program (Declined, moved aside and back)
//                             (E13) a web browser (browser=1, package_entries): Windows -- the
//                             ProgID plattestURL, Software\Clients\StartMenuInternet\plattest with
//                             its Capabilities and RegisteredApplications\plattest in the real HKCU
//                             (test names, removed again), no key of http / https, the status
//                             (never the user's default), the request as a dry run
//                             (jfgui_OPEN_DRYRUN: Settings is not opened), another copy's key as a
//                             conflict; Linux -- WebBrowser in the .desktop file, the status from
//                             mimeapps.list (Unknown, NotDefault with another browser, Default),
//                             the request writing http / https / text/html and KDE's kdeglobals

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::assoc::{
    AssocError, BrowserStatus, Conflict, Package, assoc_conflicts, assoc_register, assoc_registered, assoc_unregister, browser_request,
    browser_status, file_uri_path, package_entries, parse_schemes, parse_types, reg_entries,
};

use crate::T;

const PKG: &str = "display=JFLib Assoc Test\napp_id=org.jflib.AssocTest\ncategories=Utility;\n\
filetypes=jfltest|application/x-jflib-test|JFLib-Testdokument|own; jfltest2,jfltest3|text/x-jflib-test2|JFLib-Testtext\n\
schemes=jflibtest\n";

// (E13) a web browser: a test type (no real extension of the system), http and https
const PKG_WEB: &str = "display=JFLib Web Test\napp_id=org.jflib.WebTest\ncategories=Utility;\n\
filetypes=jflweb,jflweb2|text/x-jflib-web|JFLib-Webseite\nschemes=http https\nbrowser=1\n";

fn scratch(t: &mut T, tag: &str) -> String {
    let cwd = plat::cwd().replace('\\', "/");
    let base = format!("{cwd}/build/plattest/{}-{tag}-{:x}", crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&base), &format!("{tag}: scratch folder"));
    base
}

#[cfg(not(windows))]
fn read(path: &str) -> String {
    String::from_utf8(plat::read_file(path).unwrap_or_default()).unwrap_or_default()
}

/// run this program with args -> (exit code, stdout)
fn run_self(args: &[&str]) -> Option<(i32, String)> {
    let exe = plat::current_exe()?;
    run(&exe, args)
}

/// run a program -> (exit code, stdout + stderr); None: it could not be started
fn run(program: &str, args: &[&str]) -> Option<(i32, String)> {
    let cmd = plat::process::Command { program, args, cwd: None, env: &[] };
    let mut child = cmd.spawn().ok()?;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    let end = plat::time_us() + 30_000_000;
    loop {
        let mut got = false;
        while let Ok(Some(n)) = child.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            got = true;
        }
        while let Ok(Some(n)) = child.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            got = true;
        }
        match child.try_wait() {
            Ok(Some(code)) if !got => {
                while let Ok(Some(n)) = child.read_stdout(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    out.extend_from_slice(&buf[..n]);
                }
                return Some((code, String::from_utf8_lossy(&out).into_owned()));
            }
            Ok(_) if plat::time_us() > end => {
                child.terminate();
                return None;
            }
            Ok(_) => {
                if !got {
                    plat::sleep_ms(5);
                }
            }
            Err(_) => return None,
        }
    }
}

pub fn t_assoc(t: &mut T) {
    parts(t);
    if cfg!(target_vendor = "apple") {
        t.info("assoc: not on Apple yet (docs/APPLE.md AP7)\n");
        return;
    }

    let base = scratch(t, "assoc");
    let cfg_var = crate::CFGVAR;
    let saved_cfg = plat::env(cfg_var);
    plat::setenv(cfg_var, Some(&format!("{base}/config")));
    switches(t);
    let pkg = Package::parse(PKG);
    if cfg!(windows) {
        windows(t, &pkg);
        windows_browser(t);
    } else {
        linux(t, &pkg, &base);
        linux_browser(t, &base);
    }
    plat::setenv(cfg_var, saved_cfg.as_deref());
    crate::rm_tree(&base);
}

fn parts(t: &mut T) {
    let types = parse_types("png|image/png|PNG; jpg,JPEG,.jpe|image/jpeg|JPEG|own; bad; x|nomime|X; y|a/b|Y|mine; z|a/b|\"q\"");
    t.check(types.len() == 2 && types[1].exts == ["jpg", "jpeg", "jpe"] && types[1].own && !types[0].own,
        "assoc: parse_types (bad entries dropped, extensions lower case without the dot)");
    t.check(parse_schemes("web+jf, JFTest;1bad ok") == ["web+jf", "jftest", "ok"], "assoc: parse_schemes");
    let e = reg_entries("App", "My App", "C:\\a\\App.exe", &types, &[String::from("jfx")], "");
    t.check(e.len() == 20, "assoc: reg_entries: 20 values (ProgIDs, Open with, own defaults, the scheme)");
    let find = |k: &str, n: &str| e.iter().find(|v| v.key == k && v.name == n).map(|v| v.value.as_str());
    t.check(find("Software\\Classes\\App.png", "") == Some("PNG")
        && find("Software\\Classes\\App.png\\shell\\open\\command", "") == Some("\"C:\\a\\App.exe\" \"%1\"")
        && find("Software\\Classes\\.png\\OpenWithProgids", "App.png") == Some("")
        && find("Software\\Classes\\.png", "").is_none()
        && find("Software\\Classes\\.jpe", "") == Some("App.jpg")
        && find("Software\\Classes\\.jpe", "Content Type") == Some("image/jpeg")
        && find("Software\\Classes\\jfx", "") == Some("URL:My App")
        && find("Software\\Classes\\jfx", "URL Protocol") == Some("")
        && find("Software\\Classes\\App.jpg\\DefaultIcon", "") == Some("C:\\a\\App.exe,0"), "assoc: reg_entries: the setup's keys");
    let none = Package::parse("");
    t.check(none.app == "plattest" && none.display == "plattest" && none.app_id == "org.jflib.plattest"
        && none.categories == "Utility;" && none.types.is_empty() && none.schemes.is_empty(), "assoc: Package::parse without PACKAGE: the app's name");
    let p = Package::parse(PKG);
    t.check(p.display == "JFLib Assoc Test" && p.app_id == "org.jflib.AssocTest" && p.types.len() == 2 && p.schemes == ["jflibtest"],
        "assoc: Package::parse");
    t.check(p.mime_of("dir/A.JFLTEST3").as_deref() == Some("text/x-jflib-test2") && p.mime_of("a.txt").is_none(), "assoc: mime_of");
    let (abs_in, abs_out) = if cfg!(windows) { ("file:///C:/u/a%20b%C3%A4.txt", "C:/u/a bä.txt") } else { ("file:///home/u/a%20b%C3%A4.txt", "/home/u/a bä.txt") };
    t.expect_s(file_uri_path(abs_in).as_deref(), Some(abs_out), "assoc: file_uri_path decodes %XX");
    t.expect_s(file_uri_path("file://localhost/tmp/x").as_deref(), Some("/tmp/x"), "assoc: file_uri_path with localhost");
    t.check(file_uri_path("file://server/share/x").is_none() && file_uri_path("https://a/b").is_none()
        && file_uri_path("file:///a%zz").is_none() && file_uri_path("/plain/path").is_none(), "assoc: file_uri_path: other hosts, URLs, broken escapes, paths -> None");
    let r = plat::instance::Request { cwd: String::from("/w"), args: vec![String::from("-x"), String::from(abs_in), String::from("https://a/b")] };
    t.check(r.paths() == [String::from(abs_out), String::from("https://a/b")], "assoc: Request::paths takes file:// URIs as paths");
    t.check(assoc_register(&none, &mut |_| true) == Err(AssocError::Empty) && assoc_conflicts(&none) == Err(AssocError::Empty),
        "assoc: a package without types and schemes -> Empty");

    // (E13) a web browser's values: http and https through <app>URL, Default Programs
    let web = Package::parse("display=Netz\napp_id=org.jflib.Netz\nfiletypes=html,htm|text/html|HTML-Dokument\nschemes=http https netz\nbrowser=1\n");
    t.check(web.browser && !p.browser && !Package::parse("schemes=http https\nbrowser=0\n").browser, "assoc: Package::parse browser=1");
    let e = package_entries(&web, "C:\\n\\Netz.exe");
    let find = |k: &str, n: &str| e.iter().find(|v| v.key == k && v.name == n).map(|v| v.value.as_str());
    let smi = "Software\\Clients\\StartMenuInternet\\plattest";
    t.check(find("Software\\Classes\\plattestURL\\shell\\open\\command", "") == Some("\"C:\\n\\Netz.exe\" \"%1\"")
        && find("Software\\Classes\\plattestURL", "URL Protocol") == Some("")
        && find("Software\\Classes\\plattestURL\\Application", "ApplicationName") == Some("Netz")
        && find("Software\\Classes\\plattestURL", "AppUserModelID") == Some("org.jflib.Netz")
        && find(&format!("{smi}\\shell\\open\\command"), "") == Some("\"C:\\n\\Netz.exe\"")
        && find(&format!("{smi}\\Capabilities"), "ApplicationName") == Some("Netz")
        && find(&format!("{smi}\\Capabilities\\StartMenu"), "StartMenuInternet") == Some("plattest")
        && find(&format!("{smi}\\Capabilities\\URLAssociations"), "http") == Some("plattestURL")
        && find(&format!("{smi}\\Capabilities\\URLAssociations"), "https") == Some("plattestURL")
        && find(&format!("{smi}\\Capabilities\\FileAssociations"), ".htm") == Some("plattest.html")
        && find("Software\\RegisteredApplications", "plattest") == Some(&format!("{smi}\\Capabilities") as &str)
        && find("Software\\Classes\\netz", "URL Protocol") == Some("")
        && !e.iter().any(|v| v.key.starts_with("Software\\Classes\\http")),
        "assoc: package_entries of a browser: <app>URL, StartMenuInternet with Capabilities, RegisteredApplications, no key of http / https");
    t.check(package_entries(&p, "C:\\a.exe") == reg_entries("plattest", &p.display, "C:\\a.exe", &p.types, &p.schemes, &p.app_id),
        "assoc: package_entries without browser=1 = reg_entries");
    t.check(browser_status(&p) == BrowserStatus::NotDefault && browser_request(&p) == Err(AssocError::NotRegistered),
        "assoc: no browser=1: NotDefault, the request NotRegistered");
}

fn switches(t: &mut T) {
    let none = Package::parse("");
    let r = ui::assoc_switch(&none, "--register");
    t.check(r == Some((2, String::from("error: PACKAGE declares no filetypes= and no schemes=\n"))), "assoc: --register without types -> exit code 2");
    t.check(ui::assoc_switch(&none, "--other").is_none(), "assoc: no switch -> None");
    t.check(ui::assoc_switch(&none, "--unregister") == Some((0, String::from("nothing to unregister\n"))), "assoc: --unregister without a record");
    // the switches through ui::main! (plattest has no PACKAGE)
    match run_self(&["--register"]) {
        Some((code, out)) => t.check(code == 2 && out.contains("declares no filetypes"), "assoc: plattest --register: exit code 2"),
        None => t.check(false, "assoc: plattest --register started"),
    }
    match run_self(&["--unregister"]) {
        Some((code, out)) => t.check(code == 0 && out.contains("nothing to unregister"), "assoc: plattest --unregister: exit code 0"),
        None => t.check(false, "assoc: plattest --unregister started"),
    }
}

#[cfg(not(windows))]
fn windows(_: &mut T, _: &Package) {}

#[cfg(windows)]
const C: &str = "Software\\Classes";

/// the test keys away (an earlier run that stopped half-way)
#[cfg(windows)]
fn win_cleanup() {
    let _ = assoc_unregister();
    for k in ["plattest.jfltest", "plattest.jfltest2", ".jfltest", ".jfltest2", ".jfltest3", "jflibtest", "JFLibOther.jfltest"] {
        plat::inst::reg_delete_tree(&format!("{C}\\{k}"));
        plat::inst::reg_delete(&format!("{C}\\{k}"));
    }
}

#[cfg(windows)]
fn windows(t: &mut T, pkg: &Package) {
    use plat::assoc::hkcu_get;
    let get = |k: &str, n: &str| hkcu_get(&format!("{C}\\{k}"), Some(n));
    let has = |k: &str| hkcu_get(&format!("{C}\\{k}"), None).is_some();
    let set = |k: &str, n: &str, v: &str| {
        if let Some(key) = plat::inst::reg_create(&format!("{C}\\{k}")) {
            key.set_sz(n, v);
        }
    };
    win_cleanup();
    let exe = plat::current_exe().unwrap_or_default().replace('/', "\\");
    let cmd = format!("\"{exe}\" \"%1\"");
    t.check(assoc_conflicts(pkg) == Ok(Vec::new()), "assoc: no conflicts in a clean HKCU");
    let mut asked = 0;
    let r = assoc_register(pkg, &mut |_| {
        asked += 1;
        false
    });
    t.check(r.as_ref().is_ok_and(|r| r.types == 2 && r.schemes == 1 && r.kept.is_empty() && r.exe == exe) && asked == 0,
        "assoc: registered (2 file types, 1 scheme, nothing asked)");
    let keys_ok = |t: &mut T, what: &str| {
        t.check(get("plattest.jfltest", "").as_deref() == Some("JFLib-Testdokument")
            && get("plattest.jfltest\\shell\\open\\command", "").as_deref() == Some(cmd.as_str())
            && get("plattest.jfltest\\DefaultIcon", "").as_deref() == Some(format!("{exe},0").as_str())
            && get(".jfltest", "").as_deref() == Some("plattest.jfltest")
            && get(".jfltest", "Content Type").as_deref() == Some("application/x-jflib-test")
            && get(".jfltest\\OpenWithProgids", "plattest.jfltest").as_deref() == Some("")
            && get(".jfltest3\\OpenWithProgids", "plattest.jfltest2").as_deref() == Some("")
            && get(".jfltest2", "").is_none()
            && get("jflibtest", "").as_deref() == Some("URL:JFLib Assoc Test")
            && get("jflibtest", "URL Protocol").as_deref() == Some("")
            && get("jflibtest\\shell\\open\\command", "").as_deref() == Some(cmd.as_str()), what);
    };
    keys_ok(t, "assoc: HKCU: ProgIDs, Open with, the own type's default and Content Type, the scheme");
    t.check(assoc_registered().as_deref() == Some(exe.as_str()), "assoc: assoc_registered names the program");
    t.check(assoc_register(pkg, &mut |_| false).is_ok(), "assoc: registered again");
    keys_ok(t, "assoc: the same keys after registering again");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");
    let gone = |t: &mut T, what: &str| {
        t.check(!has("plattest.jfltest") && !has("plattest.jfltest2") && !has(".jfltest") && !has(".jfltest2") && !has(".jfltest3")
            && !has("jflibtest") && assoc_registered().is_none(), what);
    };
    gone(t, "assoc: every key removed");
    t.check(assoc_unregister() == Ok(false), "assoc: unregistering again: nothing to do");

    // another program's default of the own type
    set(".jfltest", "", "JFLibOther.jfltest");
    set("JFLibOther.jfltest\\shell\\open\\command", "", "\"C:\\Other\\other.exe\" \"%1\"");
    let other = Conflict { what: String::from(".jfltest"), holder: String::from("JFLibOther.jfltest"), fixed: false };
    t.check(assoc_conflicts(pkg) == Ok(vec![other.clone()]), "assoc: another program's default -> one conflict");
    let r = assoc_register(pkg, &mut |_| false);
    t.check(r.as_ref().is_ok_and(|r| r.kept == [other.clone()] && r.types == 2), "assoc: kept when take says no");
    t.check(get(".jfltest", "").as_deref() == Some("JFLibOther.jfltest") && get(".jfltest", "Content Type").is_none()
        && get(".jfltest\\OpenWithProgids", "plattest.jfltest").as_deref() == Some(""), "assoc: the default stays, Open with is added");
    t.check(assoc_unregister() == Ok(true) && get(".jfltest", "").as_deref() == Some("JFLibOther.jfltest")
        && !has(".jfltest\\OpenWithProgids"), "assoc: unregistered: the other default stays, our Open with key is gone");
    t.check(assoc_register(pkg, &mut |c| c == &other).is_ok() && get(".jfltest", "").as_deref() == Some("plattest.jfltest"),
        "assoc: taken when take says yes");
    t.check(assoc_conflicts(pkg) == Ok(vec![other.clone()]), "assoc: dry run while registered: the conflict of before");
    t.check(assoc_unregister() == Ok(true) && get(".jfltest", "").as_deref() == Some("JFLibOther.jfltest")
        && get(".jfltest", "Content Type").is_none(), "assoc: unregistered: the other program's default is back");

    // another program's ProgID of the same name
    let other_cmd = "\"C:\\Other\\other.exe\" /open \"%1\"";
    set("plattest.jfltest2", "", "Fremd");
    set("plattest.jfltest2", "Extra", "x");
    set("plattest.jfltest2\\shell\\open\\command", "", other_cmd);
    set("plattest.jfltest2\\shell\\edit\\command", "", other_cmd);
    let foreign = Conflict { what: String::from("plattest.jfltest2"), holder: String::from(other_cmd), fixed: false };
    t.check(assoc_conflicts(pkg).is_ok_and(|c| c.contains(&foreign)), "assoc: a ProgID of the same name with another command -> conflict");
    let r = assoc_register(pkg, &mut |c| c != &foreign);
    t.check(r.as_ref().is_ok_and(|r| r.types == 1 && r.kept == [foreign.clone()]) && get("plattest.jfltest2", "").as_deref() == Some("Fremd")
        && !has(".jfltest3"), "assoc: kept: the other ProgID unchanged, its type not registered");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");
    let r = assoc_register(pkg, &mut |_| true);
    t.check(r.is_ok() && get("plattest.jfltest2", "").as_deref() == Some("JFLib-Testtext") && get("plattest.jfltest2", "Extra").is_none()
        && get("plattest.jfltest2\\shell\\open\\command", "").as_deref() == Some(cmd.as_str()) && !has("plattest.jfltest2\\shell\\edit"),
        "assoc: taken: the key replaced by ours");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");
    t.check(get("plattest.jfltest2", "").as_deref() == Some("Fremd") && get("plattest.jfltest2", "Extra").as_deref() == Some("x")
        && get("plattest.jfltest2\\shell\\open\\command", "").as_deref() == Some(other_cmd)
        && get("plattest.jfltest2\\shell\\edit\\command", "").as_deref() == Some(other_cmd), "assoc: the other program's key is back as it was");

    // the switch's report (the other default still there)
    plat::inst::reg_delete_tree(&format!("{C}\\plattest.jfltest2"));
    plat::inst::reg_delete(&format!("{C}\\plattest.jfltest2"));
    let (code, text) = ui::assoc_switch(pkg, "--register").unwrap_or_default();
    t.check(code == 0 && text.contains(": 2 file types, 1 URL scheme\n")
        && text.contains("kept: .jfltest stays with JFLibOther.jfltest (--register-replace takes it)\n"), "assoc: --register reports the kept default");
    t.info(&format!("  {}\n", text.trim_end().replace('\n', "\n  ")));
    let (code, _) = ui::assoc_switch(pkg, "--unregister").unwrap_or_default();
    t.check(code == 0 && get(".jfltest", "").as_deref() == Some("JFLibOther.jfltest"), "assoc: --unregister");
    win_cleanup();
    gone(t, "assoc: test keys removed at the end");
}

#[cfg(not(windows))]
fn windows_browser(_: &mut T) {}

/// (E13) the browser's test keys away
#[cfg(windows)]
fn win_browser_cleanup() {
    let _ = assoc_unregister();
    for k in ["Software\\Classes\\plattestURL", "Software\\Classes\\plattest.jflweb", "Software\\Classes\\.jflweb", "Software\\Classes\\.jflweb2",
              "Software\\Clients\\StartMenuInternet\\plattest"] {
        plat::inst::reg_delete_tree(k);
        plat::inst::reg_delete(k);
    }
    plat::inst::reg_delete_value("Software\\RegisteredApplications", "plattest");
}

#[cfg(windows)]
fn windows_browser(t: &mut T) {
    use plat::assoc::hkcu_get;
    let pkg = Package::parse(PKG_WEB);
    let get = |k: &str, n: &str| hkcu_get(k, Some(n));
    let has = |k: &str| hkcu_get(k, None).is_some();
    let smi = "Software\\Clients\\StartMenuInternet\\plattest";
    win_browser_cleanup();
    let clients_before = has("Software\\Clients\\StartMenuInternet");
    let exe = plat::current_exe().unwrap_or_default().replace('/', "\\");
    let cmd = format!("\"{exe}\" \"%1\"");
    let http_before = get("Software\\Classes\\http\\shell\\open\\command", "");
    t.check(browser_request(&pkg) == Err(AssocError::NotRegistered), "assoc: browser: the request before registering -> NotRegistered");
    t.check(assoc_conflicts(&pkg) == Ok(Vec::new()), "assoc: browser: no conflicts in a clean HKCU");
    let r = assoc_register(&pkg, &mut |_| false);
    t.check(r.as_ref().is_ok_and(|r| r.types == 1 && r.schemes == 2 && r.kept.is_empty()), "assoc: browser registered (1 file type, http and https)");
    t.check(get("Software\\Classes\\plattestURL", "").as_deref() == Some("JFLib Web Test URL")
        && get("Software\\Classes\\plattestURL", "URL Protocol").as_deref() == Some("")
        && get("Software\\Classes\\plattestURL\\shell\\open\\command", "").as_deref() == Some(cmd.as_str())
        && get(&format!("{smi}\\shell\\open\\command"), "").as_deref() == Some(format!("\"{exe}\"").as_str())
        && get(&format!("{smi}\\Capabilities"), "ApplicationName").as_deref() == Some("JFLib Web Test")
        && get(&format!("{smi}\\Capabilities\\URLAssociations"), "https").as_deref() == Some("plattestURL")
        && get(&format!("{smi}\\Capabilities\\FileAssociations"), ".jflweb2").as_deref() == Some("plattest.jflweb")
        && get("Software\\RegisteredApplications", "plattest").as_deref() == Some(&format!("{smi}\\Capabilities") as &str)
        && get("Software\\Classes\\.jflweb\\OpenWithProgids", "plattest.jflweb").as_deref() == Some(""),
        "assoc: browser: HKCU: plattestURL, StartMenuInternet\\plattest with Capabilities, RegisteredApplications, Open with");
    t.check(get("Software\\Classes\\http\\shell\\open\\command", "") == http_before, "assoc: browser: no key of http written");
    t.check(browser_status(&pkg) == BrowserStatus::NotDefault, "assoc: browser: the status (the user's choice is another program)");
    let saved = plat::env("jfgui_OPEN_DRYRUN");
    plat::setenv("jfgui_OPEN_DRYRUN", Some("1"));
    t.check(browser_request(&pkg) == Ok(()), "assoc: browser: the request (dry run: Settings at registeredAppUser=plattest, printed)");
    plat::setenv("jfgui_OPEN_DRYRUN", saved.as_deref());
    let (code, text) = ui::assoc_switch(&pkg, "--register").unwrap_or_default();
    t.check(code == 0 && text.contains(": 1 file type, 2 URL schemes, as a web browser"), "assoc: browser: --register says so");
    t.check(assoc_unregister() == Ok(true), "assoc: browser unregistered");
    t.check(!has("Software\\Classes\\plattestURL") && !has(smi) && get("Software\\RegisteredApplications", "plattest").is_none()
        && !has("Software\\Classes\\.jflweb") && has("Software\\Clients\\StartMenuInternet") == clients_before,
        "assoc: browser: every key and the RegisteredApplications value removed");
    t.check(browser_request(&pkg) == Err(AssocError::NotRegistered), "assoc: browser: the request after unregistering -> NotRegistered");

    // another copy of the app holds the browser's key
    let other = "\"C:\\Other\\plattest.exe\"";
    if let Some(k) = plat::inst::reg_create(&format!("{smi}\\shell\\open\\command")) {
        k.set_sz("", other);
    }
    let c = Conflict { what: String::from("StartMenuInternet\\plattest"), holder: String::from(other), fixed: false };
    t.check(assoc_conflicts(&pkg).is_ok_and(|v| v.contains(&c)), "assoc: browser: another copy's StartMenuInternet key -> conflict");
    let r = assoc_register(&pkg, &mut |x| x != &c);
    t.check(r.as_ref().is_ok_and(|r| r.kept == [c.clone()]) && get("Software\\RegisteredApplications", "plattest").is_none()
        && get(&format!("{smi}\\shell\\open\\command"), "").as_deref() == Some(other), "assoc: browser: kept: the other key and no RegisteredApplications value");
    t.check(assoc_unregister() == Ok(true), "assoc: browser unregistered");
    win_browser_cleanup();
    t.check(!has(smi) && !has("Software\\Classes\\plattestURL"), "assoc: browser: test keys removed at the end");
}

#[cfg(windows)]
fn linux(_: &mut T, _: &Package, _: &str) {}

#[cfg(windows)]
fn linux_browser(_: &mut T, _: &str) {}

#[cfg(not(windows))]
fn linux_browser(t: &mut T, base: &str) {
    const VARS: [&str; 5] = ["XDG_DATA_HOME", "XDG_DATA_DIRS", "XDG_CONFIG_DIRS", "XDG_CURRENT_DESKTOP", "APPIMAGE"];
    let saved: Vec<Option<String>> = VARS.iter().map(|v| plat::env(v)).collect();
    let b = format!("{base}/web");
    plat::setenv("XDG_DATA_HOME", Some(&format!("{b}/data")));
    plat::setenv("XDG_DATA_DIRS", Some(&format!("{b}/sys")));
    plat::setenv("XDG_CONFIG_DIRS", Some(&format!("{b}/etc")));
    plat::setenv("XDG_CURRENT_DESKTOP", None);
    plat::setenv("APPIMAGE", None);
    let cfg = plat::env(crate::CFGVAR).unwrap_or_default();
    let list = format!("{cfg}/mimeapps.list");
    let kde = format!("{cfg}/kdeglobals");
    let desk = format!("{b}/data/applications/org.jflib.WebTest.desktop");
    let pkg = Package::parse(PKG_WEB);
    plat::delete(&list);
    t.check(browser_status(&pkg) == BrowserStatus::Unknown, "assoc: browser: Unknown without any default");
    t.check(browser_request(&pkg) == Err(AssocError::NotRegistered), "assoc: browser: the request without a .desktop file -> NotRegistered");
    // another browser is the default
    plat::mkdir(&format!("{b}/sys/applications"));
    plat::write_file(&format!("{b}/sys/applications/other.desktop"),
        b"[Desktop Entry]\nType=Application\nName=Other\nExec=other %u\nMimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n");
    let before = "[Default Applications]\nx-scheme-handler/http=other.desktop;\nx-scheme-handler/https=other.desktop;\ntext/html=other.desktop;\n";
    plat::mkdir(&cfg);
    plat::write_file(&list, before.as_bytes());
    t.check(browser_status(&pkg) == BrowserStatus::NotDefault, "assoc: browser: NotDefault (another browser in mimeapps.list)");
    let r = assoc_register(&pkg, &mut |_| false);
    t.check(r.as_ref().is_ok_and(|r| r.kept.len() == 2), "assoc: browser registered, the other browser's http and https kept");
    let text = read(&desk);
    t.check(text.contains("\nCategories=Utility;Network;WebBrowser;\n") && text.contains("\nMimeType=text/x-jflib-web;x-scheme-handler/http;x-scheme-handler/https;\n")
        && text.contains(" %U\n"), "assoc: browser: the .desktop file (Network;WebBrowser;, the web schemes, %U)");
    match run("desktop-file-validate", &[&desk]) {
        Some((code, out)) => t.check(code == 0 && !out.contains("error:"), "assoc: browser: desktop-file-validate accepts it"),
        None => t.info("  SKIP desktop-file-validate: not installed\n"),
    }
    t.check(browser_status(&pkg) == BrowserStatus::NotDefault, "assoc: browser: still NotDefault after registering");
    plat::setenv("XDG_CURRENT_DESKTOP", Some("KDE"));
    t.check(browser_request(&pkg) == Ok(()), "assoc: browser: the request");
    let now = read(&list);
    t.check(now.contains("x-scheme-handler/http=org.jflib.WebTest.desktop;other.desktop;\n")
        && now.contains("x-scheme-handler/https=org.jflib.WebTest.desktop;other.desktop;\n")
        && now.contains("text/html=org.jflib.WebTest.desktop;other.desktop;\n"), "assoc: browser: mimeapps.list: ours first for http, https, text/html");
    t.check(read(&kde).contains("[General]\nBrowserApplication=org.jflib.WebTest.desktop\n"), "assoc: browser: KDE: kdeglobals BrowserApplication");
    t.check(browser_status(&pkg) == BrowserStatus::Default, "assoc: browser: Default after the request");
    t.check(assoc_unregister() == Ok(true) && plat::file_exists(&desk) == 0, "assoc: browser unregistered (the .desktop file gone)");
    t.check(browser_status(&pkg) == BrowserStatus::NotDefault, "assoc: browser: NotDefault again (our id is no longer installed)");
    plat::delete(&list);
    plat::delete(&kde);
    crate::rm_tree(&b);
    for (v, s) in VARS.iter().zip(saved) {
        plat::setenv(v, s.as_deref());
    }
}

#[cfg(not(windows))]
fn linux(t: &mut T, pkg: &Package, base: &str) {
    const VARS: [&str; 6] = ["XDG_DATA_HOME", "XDG_DATA_DIRS", "XDG_CONFIG_DIRS", "XDG_CURRENT_DESKTOP", "APPIMAGE", "XDG_CONFIG_HOME"];
    let saved: Vec<Option<String>> = VARS.iter().map(|v| plat::env(v)).collect();
    plat::setenv("XDG_DATA_HOME", Some(&format!("{base}/data")));
    plat::setenv("XDG_DATA_DIRS", Some(&format!("{base}/sys")));
    plat::setenv("XDG_CONFIG_DIRS", Some(&format!("{base}/etc")));
    plat::setenv("XDG_CURRENT_DESKTOP", None);
    plat::setenv("APPIMAGE", None);
    let desk = format!("{base}/data/applications/org.jflib.AssocTest.desktop");
    let xml = format!("{base}/data/mime/packages/org.jflib.AssocTest.xml");
    let list = format!("{base}/config/mimeapps.list");
    let other_desk = format!("{base}/sys/applications/other.desktop");
    let exe = plat::current_exe().unwrap_or_default();

    t.check(assoc_conflicts(pkg) == Ok(Vec::new()), "assoc: no conflicts in an empty XDG tree");
    let r = assoc_register(pkg, &mut |_| false);
    t.check(r.as_ref().is_ok_and(|r| r.types == 2 && r.schemes == 1 && r.kept.is_empty() && r.exe == exe), "assoc: registered (2 file types, 1 scheme)");
    if let Ok(r) = &r {
        for n in &r.notes {
            t.info(&format!("  note: {n}\n"));
        }
    }
    let text = read(&desk);
    t.check(text.starts_with("[Desktop Entry]\nType=Application\nName=JFLib Assoc Test\nExec=") && text.contains(" %U\n")
        && text.contains("\nMimeType=application/x-jflib-test;text/x-jflib-test2;x-scheme-handler/jflibtest;\n")
        && text.contains("\nCategories=Utility;\n") && text.contains("\nStartupWMClass=plattest\n"), "assoc: the .desktop file (Exec %U, MimeType, Categories)");
    t.info(&format!("  {}\n", text.trim_end().replace('\n', "\n  ")));
    match run("desktop-file-validate", &[&desk]) {
        Some((code, out)) => {
            t.check(code == 0 && !out.contains("error:"), "assoc: desktop-file-validate accepts it");
            if !out.is_empty() {
                t.info(&format!("  desktop-file-validate: {}\n", out.trim_end()));
            }
        }
        None => t.info("  SKIP desktop-file-validate: not installed\n"),
    }
    let x = read(&xml);
    t.check(x.contains("<mime-type type=\"application/x-jflib-test\">") && x.contains("<glob pattern=\"*.jfltest\"/>") && !x.contains("jfltest2"),
        "assoc: the MIME XML of the own type");
    if run("update-mime-database", &["-v"]).is_some() {
        t.check(read(&format!("{base}/data/mime/globs2")).contains(":application/x-jflib-test:*.jfltest"), "assoc: update-mime-database knows the glob");
    } else {
        t.info("  SKIP update-mime-database: not installed\n");
    }
    t.expect_s(Some(&read(&list)), Some("[Default Applications]\napplication/x-jflib-test=org.jflib.AssocTest.desktop;\nx-scheme-handler/jflibtest=org.jflib.AssocTest.desktop;\n"),
        "assoc: mimeapps.list: the own type and the scheme");
    t.check(assoc_registered().as_deref() == Some(exe.as_str()), "assoc: assoc_registered names the program");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");
    t.check(plat::file_exists(&desk) == 0 && plat::file_exists(&xml) == 0 && plat::file_exists(&list) == 0, "assoc: the files are gone (mimeapps.list was new)");

    // another program opens the scheme (its .desktop file, no default)
    plat::mkdir(&format!("{base}/sys/applications"));
    plat::write_file(&other_desk, b"[Desktop Entry]\nType=Application\nName=Other\nExec=other %u\nMimeType=x-scheme-handler/jflibtest;\n");
    let scheme = Conflict { what: String::from("x-scheme-handler/jflibtest"), holder: String::from("other.desktop"), fixed: false };
    t.check(assoc_conflicts(pkg) == Ok(vec![scheme.clone()]), "assoc: another program with the scheme -> conflict");
    let r = assoc_register(pkg, &mut |_| false);
    t.check(r.as_ref().is_ok_and(|r| r.kept == [scheme.clone()]) && !read(&list).contains("x-scheme-handler"), "assoc: kept: no default for the scheme");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");

    // an explicit default in the user's mimeapps.list
    let before = "# mine\n[Added Associations]\nimage/png=viewer.desktop;\n\n[Default Applications]\napplication/x-jflib-test=other.desktop;\n";
    plat::mkdir(&format!("{base}/config"));
    plat::write_file(&list, before.as_bytes());
    let own = Conflict { what: String::from("application/x-jflib-test"), holder: String::from("other.desktop"), fixed: false };
    t.check(assoc_conflicts(pkg) == Ok(vec![own.clone(), scheme.clone()]), "assoc: the explicit default -> conflict");
    t.check(assoc_register(pkg, &mut |_| true).is_ok(), "assoc: registered, both taken");
    let now = read(&list);
    t.check(now.contains("\napplication/x-jflib-test=org.jflib.AssocTest.desktop;other.desktop;\n")
        && now.contains("\nx-scheme-handler/jflibtest=org.jflib.AssocTest.desktop;\n") && now.starts_with("# mine\n[Added Associations]\nimage/png=viewer.desktop;\n"),
        "assoc: ours first, the old list after it, other groups unchanged");
    t.check(assoc_conflicts(pkg) == Ok(vec![own.clone(), scheme.clone()]), "assoc: dry run while registered: the conflicts of before");
    t.check(assoc_unregister() == Ok(true), "assoc: unregistered");
    t.expect_s(Some(&read(&list)), Some(before), "assoc: mimeapps.list byte for byte as before");
    plat::delete(&list);

    // a .desktop file of the same name of another program
    let old = "[Desktop Entry]\nType=Application\nName=Old\nExec=/opt/old/app %F\n";
    plat::mkdir(&format!("{base}/data/applications"));
    plat::write_file(&desk, old.as_bytes());
    let r = assoc_register(pkg, &mut |c| c.what != desk);
    t.check(matches!(&r, Err(AssocError::Declined(c)) if c.what == desk && c.holder == "/opt/old/app %F") && read(&desk) == old,
        "assoc: a foreign .desktop file of the same name -> Declined, nothing written");
    t.check(assoc_register(pkg, &mut |_| true).is_ok() && read(&desk).contains("Name=JFLib Assoc Test")
        && read(&format!("{desk}.jflib-saved")) == old, "assoc: taken: the other file moved aside");
    t.check(assoc_unregister() == Ok(true) && read(&desk) == old && plat::file_exists(&format!("{desk}.jflib-saved")) == 0,
        "assoc: unregistered: the other file is back");

    for (v, s) in VARS.iter().zip(saved) {
        if *v != "XDG_CONFIG_HOME" {
            plat::setenv(v, s.as_deref());
        }
    }
}
