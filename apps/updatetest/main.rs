// updatetest -- the updater (lib/update) against tools/update_server.py
//
//   updatetest --test update   in this process: a valid update (check, download, staged), a
//                              redirect, a wrong signature, a manifest changed after signing,
//                              a wrong hash, a download broken off half way and continued with
//                              Range, an older version and a replayed manifest, another app's
//                              manifest, an expired one, no package for this system (also
//                              none for an exe running as an AppImage), hosts the
//                              PERMISSIONS refuse (manifest and file), cancel in the background;
//                              then applying exe and bundle updates to stand-in programs with
//                              cfg.restart = false: trial, health marker, rollback, a package
//                              update, a damaged staged file, a program folder without write
//                              permission; on Linux the AppImage updates (fake AppImages the
//                              server signed with throwaway GPG keys: the type-2 signature,
//                              replacing $APPIMAGE, trial and rollback, refused keys, a
//                              read-only folder); Flatpak and Snap without a portal: Managed
//   updatetest --test apply    the real thing: a copy of this program in a scratch folder
//                              (version 1.0.0) puts the downloaded 2.0.0 in place, restarts into
//                              it and 2.0.0 reports healthy; 3.0.0 fails its first start and the
//                              next start rolls it back to 2.0.0; on Linux with the AppImage
//                              runtime built beside this program (appimagert) and mksquashfs the
//                              same as an AppImage: the file replaced, the restart through it
//   updatetest --test flatpak  (inside a Flatpak, tools/pkg_check.py --install) the portal's
//                              update monitor: checks until it reports the newer commit, installs
//                              it (Update) and starts it (Spawn, LATEST_VERSION); each run appends
//                              to data://flatpak.log
//   updatetest --test msix     (inside a registered MSIX package, pkg_check.py --install) the
//                              package identity, Check::Managed, the StartupTask switched on and
//                              off through autostart_set
//   updatetest --child --state DIR --log FILE --url URL --pubkey HEX
//                              (--test apply) the installed program: its version and mode come
//                              from the marker the server appends to the payload
//                              ("#JFUPDTEST 2.0.0 ok#"; none: 1.0.0)
//
// The server (python / python3, or jfgui_PYTHON: a name in PATH) runs from the repository's
// tools folder (build_rs.py test runs this from the repository root); the scratch folder is
// $TEMP/updatetest-<n>/.  Prints PASS / FAIL; the exit code is the number of failures.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use update::{Check, Config, Error, Kind, Manager, Packaged, Phase, Stage, Startup, Updater};

ui::main!(updatetest());

const APP: &str = "updatetest";

struct T {
    fails: i32,
}

impl T {
    fn check(&mut self, ok: bool, name: &str) -> bool {
        if !ok {
            self.fails += 1;
        }
        plat::write_stdout(format!("{} {}\n", if ok { "PASS" } else { "FAIL" }, name).as_bytes());
        ok
    }

    fn eq<V: PartialEq + core::fmt::Debug>(&mut self, got: V, want: V, name: &str) -> bool {
        let ok = got == want;
        if !ok {
            plat::write_stdout(format!("  got  {:?}\n  want {:?}\n", got, want).as_bytes());
        }
        self.check(ok, name)
    }
}

fn updatetest() -> i32 {
    if plat::args().iter().any(|a| a == "--child") {
        return child();
    }
    let mut t = T { fails: 0 };
    match plat::arg_value("--test").as_deref() {
        Some("update") => t_update(&mut t),
        Some("apply") => t_apply(&mut t),
        Some("flatpak") => return t_flatpak(),
        Some("msix") => t_msix(&mut t),
        _ => {
            plat::write_stderr(b"updatetest --test update|apply|flatpak|msix\n");
            return 2;
        }
    }
    plat::write_stdout(format!("updatetest: {}\n", if t.fails == 0 { "all passed" } else { "FAILED" }).as_bytes());
    t.fails
}

// ---------------------------------------------------------------- the test server

struct Server {
    _child: plat::process::Child,
    base: String,
    key: String,
}

fn exe_name() -> &'static str {
    if cfg!(windows) { "updatetest.exe" } else { "updatetest" }
}

// a new scratch folder $TEMP/updatetest-<n>
fn scratch() -> String {
    let temp = plat::temp_dir().unwrap_or_default();
    let d = format!("{}/updatetest-{:x}", temp.trim_end_matches(['/', '\\']), plat::time_us() & 0xFFFF_FFFF);
    plat::mkdir(&d);
    d
}

fn read_text(p: &str) -> Option<String> {
    plat::read_file(p).map(|b| String::from(String::from_utf8_lossy(&b).trim()))
}

fn start_server(t: &mut T, dir: &str) -> Option<Server> {
    let exe = plat::current_exe().unwrap_or_default();
    let srv = format!("{}/srv", dir);
    let py = plat::env("jfgui_PYTHON").unwrap_or_else(|| String::from(if cfg!(windows) { "python" } else { "python3" }));
    // the AppImage runtime beside this program (build_rs.py builds every app into one folder)
    let rt = format!("{}/appimagert", &exe[..exe.rfind('/').unwrap_or(0)]);
    let mut args = alloc::vec!["tools/update_server.py", "--dir", &srv, "--exe", &exe, "--target", update::target()];
    if cfg!(target_os = "linux") && plat::file_exists(&rt) == 1 {
        args.extend(["--runtime", rt.as_str()]);
    }
    let cmd = plat::process::Command { program: &py, args: &args, cwd: None, env: &[] };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            t.check(false, &format!("update: start the test server ({} tools/update_server.py): error {}", py, e));
            return None;
        }
    };
    let port_file = format!("{}/port.txt", srv);
    for _ in 0..1200 {
        if let Some(port) = read_text(&port_file).filter(|p| !p.is_empty()) {
            let key = read_text(&format!("{}/pubkey.txt", srv)).unwrap_or_default();
            t.check(key.len() == 64, "update: the test server is up and gave its public key");
            return Some(Server { _child: child, base: format!("http://127.0.0.1:{}", port), key });
        }
        if let Ok(Some(code)) = child.try_wait() {
            let mut buf = [0u8; 4096];
            let n = child.read_stderr(&mut buf).ok().flatten().unwrap_or(0);
            plat::write_stdout(&buf[..n]);
            t.check(false, &format!("update: the test server ended with code {}", code));
            return None;
        }
        plat::sleep_ms(50);
    }
    t.check(false, "update: the test server did not start within 60 s (its GPG keys and AppImages on Linux)");
    None
}

// the requests the server saw: [(path, range)]
fn stats(s: &Server) -> Vec<(String, String)> {
    let c = netio::Client::new();
    let Some(u) = netio::Url::parse(&format!("{}/stats", s.base)) else { return Vec::new() };
    let Ok(r) = c.get(&u, &netio::Headers::new()) else { return Vec::new() };
    let Ok(v) = agent_core::json::parse(r.body_bytes()) else { return Vec::new() };
    v.as_array()
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let a = e.as_array()?;
            Some((String::from(a.first()?.as_str()?), String::from(a.get(1)?.as_str()?)))
        })
        .collect()
}

fn config(s: &Server, dir: &str, chan: &str, version: &str, state: &str) -> Config {
    let mut c = Config::new(APP, version, &format!("{}/{}/update.json", s.base, chan), &[&s.key]);
    c.dir = format!("{}/{}", dir, state);
    c.restart = false;
    c.timeout_ms = 15_000;
    c
}

// the scratch folder goes when everything passed (kept for a look otherwise)
fn remove_tree(t: &T, p: &str) {
    if t.fails != 0 {
        plat::write_stdout(format!("  scratch folder kept: {}\n", p).as_bytes());
        return;
    }
    let mut names = Vec::new();
    plat::dir_list(p, &mut |n, dir, _| names.push((String::from(n), dir)));
    for (n, dir) in names {
        let c = format!("{}/{}", p, n);
        if dir {
            remove_tree(t, &c);
        } else {
            plat::delete(&c);
        }
    }
    plat::delete(p);
}

fn files_like(dir: &str, part: &str) -> Vec<String> {
    let mut out = Vec::new();
    plat::dir_list(dir, &mut |n, _, _| {
        if n.contains(part) {
            out.push(String::from(n));
        }
    });
    out
}

// ---------------------------------------------------------------- --test update

fn t_update(t: &mut T) {
    // the pure parts first
    t.check(update::newer("1.10.0", "1.9.2") && !update::newer("1.9.2", "1.10.0"), "update: newer compares numbers");
    t.check(!update::newer("1.0", "1.0.0") && !update::newer("1.0.0", "1.0"), "update: missing parts are 0");
    t.check(update::newer("2.0.0", "2.0.0-rc1") && !update::newer("2.0.0-rc1", "2.0.0"), "update: a release is newer than its pre-release");
    t.check(update::newer("2.0.0-rc2", "2.0.0-rc1") && update::newer("v1.2", "1.1.9") && !update::newer("1.0.0+b7", "1.0.0"), "update: pre-releases, 'v' and build metadata");
    let Some(base) = netio::Url::parse("https://example.org/app/stable/update.json?x=1") else {
        t.check(false, "update: parse the base address");
        return;
    };
    let j = |h: &str| update::join(&base, h).map(|u| u.to_string());
    t.eq(j("app-1.2.exe").as_deref(), Some("https://example.org/app/stable/app-1.2.exe"), "update: join a relative name");
    t.eq(j("/dl/a.bin").as_deref(), Some("https://example.org/dl/a.bin"), "update: join an absolute path");
    t.eq(j("//cdn.example.org/a.bin").as_deref(), Some("https://cdn.example.org/a.bin"), "update: join another host");
    t.eq(j("http://other.example/x").as_deref(), Some("http://other.example/x"), "update: join an absolute address");
    let man = |s: &str| update::Manifest::parse(s.as_bytes());
    let ok = r#"{"format":1,"app":"a","version":"1.2.0","files":[{"target":"linux-x86_64","url":"a.bin","size":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}]}"#;
    t.check(man(ok).is_ok_and(|m| m.files[0].kind == Kind::Exe && m.files[0].size == 3 && m.expires == 0), "update: a minimal manifest (kind exe by default)");
    t.check(man(&ok.replace("\"format\":1", "\"format\":2")).is_err(), "update: another format is refused");
    t.check(man(&ok.replace("15ad", "15")).is_err(), "update: a short SHA-256 is refused");
    t.check(man(&ok.replace("\"url\"", "\"kind\":\"zip\",\"url\"")).is_ok_and(|m| m.files.is_empty()), "update: an entry of an unknown kind is left out (a newer channel)");
    t.check(man(&ok.replace("\"url\"", "\"kind\":\"Zip!\",\"url\"")).is_err(), "update: a malformed kind is refused");
    let two = ok.replace("\"files\":[", "\"files\":[{\"target\":\"linux-x86_64\",\"kind\":\"appimage\",\"url\":\"a.AppImage\",\"size\":3,\"sha256\":\"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"},");
    let m2 = man(&two);
    t.check(m2.as_ref().is_ok_and(|m| m.package_for("linux-x86_64", true).is_some_and(|p| p.kind == Kind::AppImage)
        && m.package_for("linux-x86_64", false).is_some_and(|p| p.kind == Kind::Exe)), "update: package_for picks the AppImage entry only for an AppImage");
    t.check(man(&ok.replace("\"url\"", "\"kind\":\"flatpak\",\"url\"")).is_ok_and(|m| m.files.is_empty()), "update: a flatpak entry is left out (Flatpaks update from their repository)");
    t.check(man(&ok.replace("\"app\":\"a\"", "\"app\":\"a\",\"app\":\"b\"")).is_err(), "update: duplicate keys are refused");
    t.check(man(&ok.replace("1.2.0", "1.2 0")).is_err(), "update: an invalid version is refused");

    let dir = scratch();
    let Some(s) = start_server(t, &dir) else { return };
    let file_gets = |s: &Server, path: &str| stats(s).into_iter().filter(|(p, _)| p == path).collect::<Vec<_>>();

    // no key, a bad address
    let mut c = config(&s, &dir, "good", "1.0.0", "s-none");
    c.keys.clear();
    t.check(matches!(Updater::new(c).check(), Err(Error::Config(_))), "update: no valid key: Error::Config");
    let mut c = config(&s, &dir, "good", "1.0.0", "s-none");
    c.url = String::from("ftp://example.org/update.json");
    t.check(matches!(Updater::new(c).check(), Err(Error::Config(_))), "update: an address that is not http(s): Error::Config");

    // a valid update
    let c = config(&s, &dir, "good", "1.0.0", "s-good");
    let u = Updater::new(c.clone());
    t.eq(u.status().phase, Phase::Idle, "update: a new updater is idle");
    let size = match u.check() {
        Ok(Check::Available(o)) => {
            t.check(o.version == "2.0.0" && o.kind == Kind::Exe && !o.staged && o.notes.contains("good"), "update: check finds 2.0.0 (exe, notes)");
            o.size
        }
        r => {
            t.check(false, &format!("update: check finds 2.0.0 -> {:?}", r));
            0
        }
    };
    t.check(u.status().phase == Phase::Available && u.status().version == "2.0.0", "update: status Available 2.0.0");
    t.eq(u.download(), Ok(String::from("2.0.0")), "update: download 2.0.0");
    let st = u.status();
    t.check(st.phase == Phase::Ready && st.done == size && st.total == size && size > 0, "update: status Ready, progress complete");
    let state = update::state(&c);
    t.check(state.stage == Stage::Staged && state.version == "2.0.0" && state.from == "1.0.0" && state.file == "app-2.0.0.bin", "update: state staged 2.0.0 (file app-2.0.0.bin)");
    t.eq(state.seen.as_str(), "2.0.0", "update: the newest signed version is remembered");
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.staged) && u.status().phase == Phase::Ready, "update: a second check sees it staged");
    t.eq(u.download(), Ok(String::from("2.0.0")), "update: a second download returns at once");
    t.eq(file_gets(&s, "/good/app-2.0.0.bin").len(), 1, "update: ... without fetching the file again");
    t.check(Updater::new(c.clone()).status().phase == Phase::Ready, "update: a new updater starts Ready");

    // redirects: manifest and signature, the file relative to where the manifest came from
    let u = Updater::new(config(&s, &dir, "redirect", "1.0.0", "s-redirect"));
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.version == "2.0.0"), "update: redirected manifest and signature");
    t.eq(u.download(), Ok(String::from("2.0.0")), "update: ... the file relative to the final address");

    // wrong signatures
    let u = Updater::new(config(&s, &dir, "badsig", "1.0.0", "s-badsig"));
    t.eq(u.check(), Err(Error::Signature), "update: signed with another key: Error::Signature");
    t.check(u.status().phase == Phase::Failed && u.status().error == Some(Error::Signature), "update: status Failed with the error");
    t.eq(update::state(u.config()).seen.as_str(), "", "update: ... nothing of it is remembered");
    t.eq(Updater::new(config(&s, &dir, "tampered", "1.0.0", "s-tampered")).check(), Err(Error::Signature), "update: changed after signing: Error::Signature");

    // a wrong file
    let c = config(&s, &dir, "badhash", "1.0.0", "s-badhash");
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(_))), "update: badhash: the manifest is fine");
    t.eq(u.download(), Err(Error::Hash), "update: a file with another SHA-256: Error::Hash");
    t.check(files_like(&format!("{}/partial", c.dir), "").is_empty() && update::state(&c).stage == Stage::None, "update: ... it is deleted, nothing staged");

    // broken off half way, continued with Range
    let c = config(&s, &dir, "flaky", "1.0.0", "s-flaky");
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(_))), "update: flaky: the manifest is fine");
    match u.download() {
        Err(Error::Incomplete { done, total }) => {
            t.check(done > 0 && done < total, &format!("update: an interrupted download: Error::Incomplete ({} of {})", done, total));
            t.eq(files_like(&format!("{}/partial", c.dir), "").len(), 1, "update: ... the part is kept");
            t.check(update::state(&c).stage == Stage::None, "update: ... nothing staged");
            t.eq(u.download(), Ok(String::from("2.0.0")), "update: the next download completes it");
            let gets = file_gets(&s, "/flaky/app-2.0.0.bin");
            t.check(gets.len() == 2 && gets[1].1 == format!("bytes={}-", done), &format!("update: ... with a Range request ({:?})", gets));
        }
        r => {
            t.check(false, &format!("update: an interrupted download: Error::Incomplete -> {:?}", r));
        }
    }

    // versions
    t.eq(Updater::new(config(&s, &dir, "old", "1.0.0", "s-old")).check(), Ok(Check::UpToDate), "update: an older version: UpToDate");
    t.eq(Updater::new(config(&s, &dir, "good", "2.0.0", "s-same")).check(), Ok(Check::UpToDate), "update: the same version: UpToDate");
    t.eq(Updater::new(config(&s, &dir, "old", "0.5.0", "s-good")).check(), Err(Error::Replay(String::from("2.0.0"))), "update: a manifest older than one seen before: Error::Replay");
    t.check(matches!(Updater::new(config(&s, &dir, "otherapp", "1.0.0", "s-other")).check(), Err(Error::Manifest(_))), "update: another app's manifest: Error::Manifest");
    t.eq(Updater::new(config(&s, &dir, "expired", "1.0.0", "s-expired")).check(), Err(Error::Expired), "update: an expired manifest: Error::Expired");
    t.eq(Updater::new(config(&s, &dir, "noplatform", "1.0.0", "s-noplat")).check(), Ok(Check::NoPackage(String::from("2.0.0"))), "update: no package for this system: NoPackage");
    // an AppImage updates as a whole file: its extracted exe is no target
    #[cfg(target_os = "linux")]
    {
        plat::setenv("APPIMAGE", Some(&format!("{}/updatetest.AppImage", dir)));
        t.eq(Updater::new(config(&s, &dir, "good", "1.0.0", "s-appimage")).check(), Ok(Check::NoPackage(String::from("2.0.0"))), "update: running as an AppImage: no exe update offered (NoPackage)");
        plat::setenv("APPIMAGE", None);
    }
    t_managed(t, &s, &dir);

    // hosts the PERMISSIONS refuse
    let port = s.base.rsplit(':').next().unwrap_or("");
    let mut c = config(&s, &dir, "good", "1.0.0", "s-denied");
    c.url = format!("http://updates.invalid:{}/good/update.json", port);
    t.eq(Updater::new(c).check(), Err(Error::Denied(String::from("updates.invalid"))), "update: a manifest host net.connect does not allow: Error::Denied");
    let u = Updater::new(config(&s, &dir, "deniedfile", "1.0.0", "s-deniedfile"));
    t.check(matches!(u.check(), Ok(Check::Available(_))), "update: deniedfile: the manifest is fine");
    t.eq(u.download(), Err(Error::Denied(String::from("download.invalid"))), "update: a file host net.connect does not allow: Error::Denied");
    let d = plat::policy::denials();
    t.check(["updates.invalid", "download.invalid"].iter().all(|h| d.iter().any(|x| x.kind == "net.connect" && x.target == *h)), "update: both refusals are in plat::policy::denials()");

    // in the background, cancelled
    let c = config(&s, &dir, "big", "1.0.0", "s-big");
    let u = Updater::new(c.clone());
    t.check(u.start_check(), "update: start_check");
    let wait_idle = |u: &Updater| {
        for _ in 0..600 {
            if !u.busy() {
                return true;
            }
            plat::sleep_ms(20);
        }
        false
    };
    t.check(wait_idle(&u) && u.status().phase == Phase::Available, "update: the background check ends Available");
    t.check(u.start_download(), "update: start_download");
    let mut seen = false;
    for _ in 0..500 {
        if u.status().done > 0 {
            seen = true;
            break;
        }
        plat::sleep_ms(10);
    }
    t.check(seen && u.status().phase == Phase::Downloading, "update: progress while downloading");
    t.check(u.busy() && !u.start_check(), "update: no second background step while one runs");
    u.cancel();
    t.check(wait_idle(&u), "update: cancel ends the download");
    let st = u.status();
    t.check(st.phase == Phase::Failed && st.error == Some(Error::Cancelled) && st.done < st.total, "update: ... with Error::Cancelled");
    t.check(update::state(&c).stage == Stage::None, "update: ... nothing staged");

    t_stand_in(t, &s, &dir);
    #[cfg(target_os = "linux")]
    t_appimage(t, &s, &dir);
    drop(s);
    remove_tree(t, &dir);
}

// installations a package system updates: Flatpak without a portal (no session bus for the
// test), Snap; on Windows outside a package: Plain
fn t_managed(t: &mut T, s: &Server, dir: &str) {
    let c = config(s, dir, "good", "1.0.0", "s-managed");
    if cfg!(windows) {
        t.eq(update::install(&c), Packaged::Plain, "managed: outside an MSIX package the program is Plain");
        return;
    }
    let bus = plat::env("DBUS_SESSION_BUS_ADDRESS");
    plat::setenv("DBUS_SESSION_BUS_ADDRESS", Some("unix:path=/nonexistent/updatetest-bus"));
    plat::setenv("FLATPAK_ID", Some("org.jflib.updatetest"));
    t.eq(update::install(&c), Packaged::Flatpak(String::from("org.jflib.updatetest")), "managed: $FLATPAK_ID: a Flatpak");
    t.eq(Updater::new(c.clone()).check(), Ok(Check::Managed(Manager::Flatpak, None)), "managed: a Flatpak without the portal: Check::Managed(Flatpak)");
    let mut own = c.clone();
    own.exe = String::from("/opt/updatetest/updatetest");
    t.eq(update::install(&own), Packaged::Plain, "managed: cfg.exe set: Plain (an explicit program file)");
    plat::setenv("FLATPAK_ID", None);
    plat::setenv("DBUS_SESSION_BUS_ADDRESS", bus.as_deref());
    plat::setenv("SNAP", Some("/snap/updatetest/7"));
    t.eq(Updater::new(c).check(), Ok(Check::Managed(Manager::Snap, None)), "managed: a Snap: Check::Managed(Snap)");
    plat::setenv("SNAP", None);
}

// ---------------------------------------------------------------- AppImage (Linux)

#[cfg(target_os = "linux")]
fn t_appimage(t: &mut T, s: &Server, dir: &str) {
    let src = format!("{}/srv/appimage", dir);
    if let Some(why) = read_text(&format!("{}/error.txt", src)) {
        t.check(false, &format!("appimage: the test server made no AppImages: {}", why));
        return;
    }
    if plat::file_exists(&format!("{}/running-a.AppImage", src)) != 1 {
        plat::write_stdout(b"SKIP appimage: the test server made no AppImages (gpg missing)\n");
        return;
    }
    let fpr = |k: &str| read_text(&format!("{}/fpr-{}.txt", src, k)).unwrap_or_default();
    let signer = |f: &str| match update::appimage_signature(&format!("{}/{}.AppImage", src, f)) {
        Ok(update::Signed::By { signer, .. }) => Ok(signer.iter().map(|b| format!("{:02X}", b)).collect::<String>()),
        Ok(update::Signed::No) => Ok(String::new()),
        Err(e) => Err(e),
    };
    t.eq(signer("running-a"), Ok(fpr("a")), "appimage: a signature in .sha256_sig / .sig_key checks (Ed25519)");
    t.eq(signer("running-b"), Ok(fpr("b")), "appimage: ... RSA");
    t.eq(signer("running-c"), Ok(fpr("c")), "appimage: ... ECDSA P-256");
    t.eq(signer("running-unsigned"), Ok(String::new()), "appimage: empty sections: unsigned");
    t.check(signer("tampered").is_err(), "appimage: a byte changed after signing: the signature does not check");

    // the "installed" AppImage: a copy of a running-*.AppImage in a folder of its own
    let install = |name: &str, from: &str| -> String {
        let d = format!("{}/ai-{}", dir, name);
        plat::mkdir(&d);
        let p = format!("{}/App.AppImage", d);
        plat::write_file(&p, &plat::read_file(&format!("{}/{}.AppImage", src, from)).unwrap_or_default());
        plat::fs::set_mode(&p, 0o755);
        plat::setenv("APPIMAGE", Some(&p));
        p
    };
    let plain = config(s, dir, "appimage", "1.0.0", "s-ai-plain");
    t.check(matches!(Updater::new(plain).check(), Ok(Check::Available(o)) if o.kind == Kind::Package), "appimage: a plain program of the channel gets the .deb");
    let p = install("good", "running-a");
    let mut c = config(s, dir, "appimage", "1.0.0", "s-ai-good");
    t.eq(update::install(&c), Packaged::AppImage(p.clone()), "appimage: install() names the AppImage file");
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.kind == Kind::AppImage && o.version == "2.0.0"), "appimage: the AppImage gets the AppImage entry (2.0.0)");
    t.eq(u.download(), Ok(String::from("2.0.0")), "appimage: downloaded");
    let old = plat::read_file(&p).unwrap_or_default();
    let new = plat::read_file(&format!("{}/update-a-2.0.0.AppImage", src)).unwrap_or_default();
    t.eq(update::startup(&c), Startup::Applied(String::from("2.0.0")), "appimage: startup replaces the AppImage file");
    t.check(plat::read_file(&p) == Some(new.clone()) && plat::fs::stat(&p).is_some_and(|m| m.mode & 0o777 == 0o755), "appimage: the file is the new one, executable");
    t.check(plat::read_file(&format!("{}/backup/files/App.AppImage", c.dir)) == Some(old.clone()), "appimage: the old file is the backup");
    c.version = String::from("2.0.0");
    t.eq(update::startup(&c), Startup::Trial(String::from("2.0.0")), "appimage: 2.0.0 starts on trial");
    t.check(update::healthy(&c), "appimage: healthy()");
    // 3.0.0 fails: rolled back to the 2.0.0 file
    c.url = format!("{}/appimage-bad/update.json", s.base);
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.version == "3.0.0") && u.download().is_ok(), "appimage: 3.0.0 downloaded");
    t.eq(update::startup(&c), Startup::Applied(String::from("3.0.0")), "appimage: 3.0.0 put in place");
    c.version = String::from("3.0.0");
    t.eq(update::startup(&c), Startup::Trial(String::from("3.0.0")), "appimage: 3.0.0 on trial (no healthy())");
    t.eq(update::startup(&c), Startup::RollingBack(String::from("3.0.0")), "appimage: the next start rolls it back");
    t.check(plat::read_file(&p) == Some(new.clone()), "appimage: the AppImage file is 2.0.0 again");
    t.eq(files_like(&format!("{}/ai-good", dir), ".jfupd-"), Vec::<String>::new(), "appimage: nothing left next to the AppImage");

    // refused: another key, unsigned after signed, a broken signature, no AppImage
    for (chan, what) in [("appimage-otherkey", "signed with another key"), ("appimage-unsigned", "unsigned"),
                         ("appimage-tampered", "a signature that does not check"), ("appimage-notai", "no AppImage")] {
        let p = install(chan, "running-a");
        let before = plat::read_file(&p);
        let c = config(s, dir, chan, "1.0.0", &format!("s-{}", chan));
        let u = Updater::new(c.clone());
        t.check(matches!(u.check(), Ok(Check::Available(_))) && u.download().is_ok(), &format!("appimage: {}: downloaded (the manifest is fine)", chan));
        let r = update::startup(&c);
        t.check(matches!(r, Startup::Failed(Error::Rejected(_))), &format!("appimage: {} is refused ({:?})", what, r));
        t.check(plat::read_file(&p) == before && files_like(&format!("{}/ai-{}", dir, chan), ".jfupd-").is_empty()
                && update::state(&c).stage == Stage::None, &format!("appimage: {}: the file is unchanged, the update dropped", chan));
    }
    // accepted: a signed update over an unsigned file; RSA and ECDSA keys
    for (chan, from, name) in [("appimage", "running-unsigned", "unsigned"), ("appimage-rsa", "running-b", "rsa"), ("appimage-ecdsa", "running-c", "ecdsa")] {
        install(&format!("{}-{}", chan, name), from);
        let c = config(s, dir, chan, "1.0.0", &format!("s-ok-{}", name));
        let u = Updater::new(c.clone());
        t.check(u.check().is_ok() && u.download().is_ok(), &format!("appimage: {} -> {}: downloaded", from, chan));
        t.eq(update::startup(&c), Startup::Applied(String::from("2.0.0")), &format!("appimage: {} takes {}", from, chan));
    }
    // a folder the user cannot write: reported, nothing downloaded
    let p = install("ro", "running-a");
    let folder = String::from(&p[..p.rfind('/').unwrap_or(0)]);
    plat::fs::set_mode(&folder, 0o555);
    if plat::File::create_new(&format!("{}/probe", folder)).is_some() {
        plat::delete(&format!("{}/probe", folder));
        plat::write_stdout(b"SKIP appimage: a read-only folder (root writes anyway)\n");
    } else {
        t.eq(Updater::new(config(s, dir, "appimage", "1.0.0", "s-ai-ro")).check(), Err(Error::ReadOnly(folder.clone())), "appimage: a read-only folder: Error::ReadOnly");
    }
    plat::fs::set_mode(&folder, 0o755);
    plat::setenv("APPIMAGE", None);
}

// ---------------------------------------------------------------- --test flatpak

// inside the Flatpak: the portal's monitor reports the newer commit, Update installs it, Spawn
// starts it; the new version (its marker says 2.0.0) logs its start and finds nothing newer
fn t_flatpak() -> i32 {
    let log = "data://flatpak.log";
    let line = |s: &str| {
        let mut b = plat::read_file(log).unwrap_or_default();
        b.extend_from_slice(s.as_bytes());
        b.push(b'\n');
        plat::write_file(log, &b);
        plat::write_stdout(format!("{}\n", s).as_bytes());
    };
    let (version, _) = marker_of(&plat::current_exe().unwrap_or_default());
    let cfg = Config::new(APP, &version, "https://updates.invalid/update.json", &[]);
    line(&format!("run {} {:?}", version, update::install(&cfg)));
    let u = Updater::new(cfg);
    let mut found = None;
    for _ in 0..120 {
        match u.check() {
            Ok(Check::Available(o)) => {
                found = Some(o);
                break;
            }
            Ok(Check::UpToDate) if version != "1.0.0" => break,
            Ok(Check::UpToDate) => {}
            r => {
                line(&format!("check {} {:?}", version, r));
                return 1;
            }
        }
        plat::sleep_ms(500);
    }
    let Some(o) = found else {
        line(&format!("uptodate {}", version));
        return 0;
    };
    line(&format!("available {} kind {} staged {}", o.version, o.kind.name(), o.staged));
    match u.download() {
        Ok(v) => line(&format!("installed {} progress {}/{}", v, u.status().done, u.status().total)),
        Err(e) => {
            line(&format!("download failed {}", e));
            return 1;
        }
    }
    line(&format!("restart {}", version));
    match u.apply() {
        Ok(s) => line(&format!("apply returned {:?}", s)),
        Err(e) => line(&format!("apply failed {}", e)),
    }
    1
}

// ---------------------------------------------------------------- --test msix

// inside a registered MSIX package (started through its execution alias)
fn t_msix(t: &mut T) {
    let cfg = Config::new(APP, "1.0.0", "https://updates.invalid/update.json", &[]);
    let inst = update::install(&cfg);
    t.check(matches!(&inst, Packaged::Msix(n) if n.starts_with("org.jflib.updatetest_")), &format!("msix: a package identity ({:?})", inst));
    let r = Updater::new(cfg).check();
    t.check(matches!(r, Ok(Check::Managed(Manager::Msix | Manager::AppInstaller, _))), &format!("msix: check() answers Managed ({:?})", r));
    t.check(!plat::autostart_get(), "msix: the StartupTask starts switched off");
    let on = plat::autostart_set(true, &["--ignored"]);
    t.eq(on, Ok(()), "msix: autostart_set(true) enables the StartupTask (RequestEnableAsync)");
    t.check(plat::autostart_get(), "msix: autostart_get() is true");
    let e = plat::autostart_entry().unwrap_or_default();
    t.check(e.to_ascii_lowercase().ends_with("updatetest.exe\""), &format!("msix: autostart_entry() names the program ({})", e));
    t.eq(plat::autostart_set(false, &[]), Ok(()), "msix: autostart_set(false) disables it");
    t.check(!plat::autostart_get(), "msix: autostart_get() is false again");
}

// applying to stand-in program files (cfg.restart = false: startup() returns instead of
// restarting; a "start" is the next startup() call with the version that then runs)
fn t_stand_in(t: &mut T, s: &Server, dir: &str) {
    let fresh = config(s, dir, "good", "1.0.0", "s-fresh");
    t.check(update::startup(&fresh) == Startup::Normal && !update::healthy(&fresh), "apply: without any download startup and healthy do nothing");
    t.eq(plat::file_exists(&fresh.dir), 0, "apply: ... and create no folder");
    let inst = format!("{}/inst-exe", dir);
    let prog = format!("{}/app.bin", inst);
    plat::mkdir(&inst);
    plat::write_file(&prog, b"old program 1.0.0");
    let mut c = config(s, dir, "good", "1.0.0", "s-good");
    c.exe = prog.clone();
    let staged_bytes = plat::read_file(&format!("{}/staged/app-2.0.0.bin", c.dir)).unwrap_or_default();
    t.check(staged_bytes.ends_with(b"#JFUPDTEST 2.0.0 ok#"), "apply: the staged file is the 2.0.0 program");
    t.eq(update::startup(&c), Startup::Applied(String::from("2.0.0")), "apply: startup puts the staged 2.0.0 in place");
    t.check(plat::read_file(&prog) == Some(staged_bytes.clone()), "apply: the program file is the new one");
    t.check(plat::read_file(&format!("{}/backup/files/app.bin", c.dir)).as_deref() == Some(&b"old program 1.0.0"[..]), "apply: the old one is in backup/");
    let st = update::state(&c);
    t.check(st.stage == Stage::Trial && st.version == "2.0.0" && st.from == "1.0.0" && st.starts == 0, "apply: state trial 2.0.0 from 1.0.0");
    t.check(!update::healthy(&c), "apply: healthy() of the old version does nothing");
    c.version = String::from("2.0.0");
    t.eq(update::startup(&c), Startup::Trial(String::from("2.0.0")), "apply: the first start of 2.0.0 is a trial");
    t.eq(files_like(&inst, ".jfupd-"), Vec::<String>::new(), "apply: leftovers (.jfupd-*) are removed at the next start");
    t.check(update::healthy(&c), "apply: healthy() ends the trial");
    let st = update::state(&c);
    t.check(st.stage == Stage::Ok && plat::file_exists(&format!("{}/staged", c.dir)) == 0, "apply: state ok, staged/ removed");
    t.check(!update::healthy(&c), "apply: a second healthy() does nothing");
    t.eq(update::startup(&c), Startup::Normal, "apply: later starts are normal");

    // 3.0.0 fails its start: rolled back at the next one, never offered again
    c.url = format!("{}/bad/update.json", s.base);
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.version == "3.0.0") && u.download().is_ok(), "rollback: 3.0.0 checked and downloaded");
    t.eq(update::startup(&c), Startup::Applied(String::from("3.0.0")), "rollback: 3.0.0 put in place");
    c.version = String::from("3.0.0");
    t.eq(update::startup(&c), Startup::Trial(String::from("3.0.0")), "rollback: 3.0.0 starts on trial (and fails: no healthy())");
    t.eq(update::startup(&c), Startup::RollingBack(String::from("3.0.0")), "rollback: the next start rolls it back");
    t.check(plat::read_file(&prog) == Some(staged_bytes.clone()), "rollback: the program file is 2.0.0 again");
    let st = update::state(&c);
    t.check(st.stage == Stage::RolledBack && st.version == "2.0.0" && st.bad == ["3.0.0"], "rollback: state rolledback, 3.0.0 recorded as bad");
    c.version = String::from("2.0.0");
    t.eq(update::startup(&c), Startup::RolledBack(String::from("3.0.0")), "rollback: 2.0.0 learns of it once");
    t.eq(update::startup(&c), Startup::Normal, "rollback: ... then starts normally");
    t.eq(Updater::new(c.clone()).check(), Ok(Check::UpToDate), "rollback: 3.0.0 is not offered again");

    // a bundle: the program and a new module; rolled back both
    let inst = format!("{}/inst-bundle", dir);
    let prog = format!("{}/app.bin", inst);
    plat::mkdir(&inst);
    plat::write_file(&prog, b"old bundle program");
    let mut c = config(s, dir, "bundle", "1.0.0", "s-bundle");
    c.exe = prog.clone();
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.kind == Kind::Bundle) && u.download().is_ok(), "bundle: checked and downloaded");
    t.eq(update::startup(&c), Startup::Applied(String::from("2.0.0")), "bundle: put in place");
    let module = format!("{}/mods/extra.dat", inst);
    t.check(plat::read_file(&prog).is_some_and(|b| b.starts_with(b"Programm 2.0.0\n") && b.len() == 1500), "bundle: the program file");
    t.check(plat::read_file(&module).as_deref() == Some(&b"Modul 2.0.0\n"[..]), "bundle: the new module in its folder");
    c.version = String::from("2.0.0");
    t.eq(update::startup(&c), Startup::Trial(String::from("2.0.0")), "bundle: trial");
    t.eq(update::startup(&c), Startup::RollingBack(String::from("2.0.0")), "bundle: rolled back without healthy()");
    t.check(plat::read_file(&prog).as_deref() == Some(&b"old bundle program"[..]), "bundle: the old program is back");
    t.check(plat::file_exists(&module) == 0, "bundle: the new module is gone");

    // a package: downloaded and checked, installed by the app
    let c = config(s, dir, "package", "1.0.0", "s-package");
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.kind == Kind::Package) && u.download().is_ok(), "package: checked and downloaded");
    match u.staged() {
        Some((v, k, p)) => {
            t.check(v == "2.0.0" && k == Kind::Package && p.ends_with("updatetest-2.0.0-setup.exe"), &format!("package: staged() names the file ({})", p));
            t.check(plat::read_file(&p).is_some_and(|b| b.starts_with(b"MZ setup 2.0.0")), "package: ... with its content");
        }
        None => {
            t.check(false, "package: staged() names the file");
        }
    }
    t.eq(update::startup(&c), Startup::Normal, "package: startup leaves it alone");
    t.check(matches!(u.apply(), Err(Error::Unsupported(_))), "package: apply() refuses: Error::Unsupported");

    // a staged file damaged on disk: refused and dropped
    let c = config(s, dir, "good", "1.0.0", "s-damage");
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(_))) && u.download().is_ok(), "damage: 2.0.0 downloaded");
    plat::write_file(&format!("{}/staged/app-2.0.0.bin", c.dir), b"damaged");
    t.check(matches!(update::startup(&c), Startup::Failed(Error::Size { .. })), "damage: startup refuses the damaged file");
    t.check(update::state(&c).stage == Stage::None, "damage: ... and drops it");

    // a program folder this app may not write (no fs.write $EXE/**)
    let exe = plat::current_exe().unwrap_or_default();
    let exe_dir = &exe[..exe.rfind(['/', '\\']).unwrap_or(0)];
    let mut c = config(s, dir, "good", "1.0.0", "s-nowrite");
    c.exe = format!("{}/updatetest-standin.bin", exe_dir);
    let u = Updater::new(c.clone());
    t.check(matches!(u.check(), Ok(Check::Available(_))) && u.download().is_ok(), "nowrite: 2.0.0 downloaded");
    t.check(matches!(update::startup(&c), Startup::Failed(Error::Io(ref m)) if m.contains("fs.write $EXE")), "nowrite: startup fails and names the permission");
    t.check(plat::file_exists(&c.exe) == 0 && update::state(&c).stage == Stage::Staged, "nowrite: nothing written, the update stays staged");
    t.check(plat::policy::denials().iter().any(|d| d.kind == "fs.write" && d.target.contains("updatetest-standin.bin")), "nowrite: the refusal is in plat::policy::denials()");
}

// ---------------------------------------------------------------- --test apply

fn t_apply(t: &mut T) {
    let dir = scratch();
    let Some(s) = start_server(t, &dir) else { return };
    // the "installed" program: a copy of this one (version 1.0.0)
    let inst = format!("{}/inst", dir);
    let prog = format!("{}/{}", inst, exe_name());
    let me = plat::current_exe().and_then(|p| plat::read_file(&p)).unwrap_or_default();
    t.check(!me.is_empty() && plat::mkdir(&inst) && plat::write_file(&prog, &me), "apply: this program copied to the scratch folder");
    #[cfg(target_os = "linux")]
    plat::fs::set_mode(&prog, 0o755);
    let log = format!("{}/log.txt", dir);
    let state = format!("{}/state", dir);
    let good = config(&s, &dir, "good", "1.0.0", "state");
    let u = Updater::new(good.clone());
    t.check(matches!(u.check(), Ok(Check::Available(_))) && u.download().is_ok(), "apply: 2.0.0 downloaded and staged");
    let run = |t: &mut T, what: &str, want: &str| -> bool {
        let args = ["--child", "--state", &state, "--log", &log, "--url", &good.url, "--pubkey", &s.key];
        let cmd = plat::process::Command { program: &prog, args: &args, cwd: None, env: &[] };
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return t.check(false, &format!("apply: start the installed program: error {}", e)),
        };
        let mut buf = [0u8; 4096];
        for _ in 0..1200 {
            // keep its output pipes empty
            while child.read_stdout(&mut buf).ok().flatten().is_some_and(|n| n > 0) {}
            while child.read_stderr(&mut buf).ok().flatten().is_some_and(|n| n > 0) {}
            if read_text(&log).is_some_and(|l| l.lines().any(|x| x == want)) {
                // the processes have ended or are ending: give them a moment
                plat::sleep_ms(200);
                return t.check(true, what);
            }
            plat::sleep_ms(50);
        }
        plat::write_stdout(format!("  log:\n{}\n", read_text(&log).unwrap_or_default()).as_bytes());
        t.check(false, &format!("{} (no \"{}\" within 60 s)", what, want))
    };
    let marker = |p: &str| marker_of(p).0;
    // 1.0.0 -> 2.0.0, healthy
    if run(t, "apply: the installed 1.0.0 puts 2.0.0 in place, restarts, 2.0.0 reports healthy", "healthy 2.0.0 true") {
        let l = read_text(&log).unwrap_or_default();
        t.check(l.contains("run 1.0.0") && l.contains("run 2.0.0") && l.contains("startup 2.0.0 trial 2.0.0"), "apply: ... the log shows both starts");
        t.check(!l.contains("startup 1.0.0"), "apply: ... 1.0.0 ended inside startup() (restart)");
    }
    t.eq(marker(&prog), String::from("2.0.0"), "apply: the installed program is 2.0.0");
    let st = update::state(&good);
    t.check(st.stage == Stage::Ok && st.version == "2.0.0", "apply: state ok 2.0.0");
    t.check(plat::read_file(&format!("{}/backup/files/{}", state, exe_name())) == Some(me.clone()), "apply: the old program is in backup/");
    // 2.0.0 -> 3.0.0, which fails its start; the next start rolls back
    let mut bad = config(&s, &dir, "bad", "2.0.0", "state");
    bad.exe = prog.clone();
    let u = Updater::new(bad.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.version == "3.0.0") && u.download().is_ok(), "rollback: 3.0.0 downloaded and staged");
    run(t, "rollback: 2.0.0 puts 3.0.0 in place and restarts; 3.0.0 fails before healthy()", "crash 3.0.0");
    t.eq(marker(&prog), String::from("3.0.0"), "rollback: the installed program is 3.0.0");
    t.check(update::state(&bad).stage == Stage::Trial, "rollback: the trial is still open");
    if run(t, "rollback: the next start of 3.0.0 rolls back and restarts 2.0.0", "healthy 2.0.0 false") {
        t.check(read_text(&log).unwrap_or_default().contains("startup 2.0.0 rolledback 3.0.0"), "rollback: ... 2.0.0 learns that 3.0.0 failed");
    }
    t.eq(marker(&prog), String::from("2.0.0"), "rollback: the installed program is 2.0.0 again");
    let st = update::state(&bad);
    t.check(st.stage == Stage::None && st.bad.iter().any(|b| b == "3.0.0"), "rollback: 3.0.0 recorded as bad");
    t.eq(Updater::new(bad).check(), Ok(Check::UpToDate), "rollback: 3.0.0 is not offered again");
    #[cfg(target_os = "linux")]
    t_apply_appimage(t, &s, &dir);
    drop(s);
    plat::sleep_ms(300); // the last processes end
    remove_tree(t, &dir);
}

// start prog (with env) as the installed program and wait until the log has the line want
#[cfg(target_os = "linux")]
fn run_installed(t: &mut T, prog: &str, args: &[&str], env: &[(&str, Option<&str>)], log: &str, what: &str, want: &str) -> bool {
    let cmd = plat::process::Command { program: prog, args, cwd: None, env };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return t.check(false, &format!("{}: start {}: error {}", what, prog, e)),
    };
    let mut buf = [0u8; 4096];
    for _ in 0..1200 {
        while child.read_stdout(&mut buf).ok().flatten().is_some_and(|n| n > 0) {}
        while child.read_stderr(&mut buf).ok().flatten().is_some_and(|n| n > 0) {}
        if read_text(log).is_some_and(|l| l.lines().any(|x| x == want)) {
            plat::sleep_ms(200);
            return t.check(true, what);
        }
        plat::sleep_ms(50);
    }
    plat::write_stdout(format!("  log:\n{}\n", read_text(log).unwrap_or_default()).as_bytes());
    t.check(false, &format!("{} (no \"{}\" within 60 s)", what, want))
}

// the same with a real AppImage: the runtime unpacks it, the update replaces the .AppImage
// file, the restart goes through it (plat::pkg::restart_appimage)
#[cfg(target_os = "linux")]
fn t_apply_appimage(t: &mut T, s: &Server, dir: &str) {
    let src = format!("{}/srv/appimage", dir);
    if plat::file_exists(&format!("{}/real-1.0.0.AppImage", src)) != 1 {
        plat::write_stdout(b"SKIP apply appimage: no AppImage runtime beside this program (appimagert), mksquashfs or gpg\n");
        return;
    }
    let inst = format!("{}/ai-inst", dir);
    let prog = format!("{}/updatetest.AppImage", inst);
    let cache = format!("{}/ai-cache", dir);
    plat::mkdir(&inst);
    plat::write_file(&prog, &plat::read_file(&format!("{}/real-1.0.0.AppImage", src)).unwrap_or_default());
    plat::fs::set_mode(&prog, 0o755);
    let log = format!("{}/ai-log.txt", dir);
    let state = format!("{}/ai-state", dir);
    // downloaded as the AppImage would (the updater sees $APPIMAGE)
    plat::setenv("APPIMAGE", Some(&prog));
    let good = config(s, dir, "aiapply-good", "1.0.0", "ai-state");
    let u = Updater::new(good.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.kind == Kind::AppImage) && u.download().is_ok(), "apply appimage: 2.0.0 downloaded");
    plat::setenv("APPIMAGE", None);
    let args = ["--child", "--state", &state, "--log", &log, "--url", &good.url, "--pubkey", &s.key];
    let env = [("XDG_CACHE_HOME", Some(cache.as_str()))];
    if run_installed(t, &prog, &args, &env, &log, "apply appimage: 1.0.0 replaces its AppImage, restarts through it, 2.0.0 reports healthy", "healthy 2.0.0 true") {
        let l = read_text(&log).unwrap_or_default();
        t.check(l.contains("run 1.0.0") && l.contains("startup 2.0.0 trial 2.0.0") && !l.contains("startup 1.0.0"), "apply appimage: ... the log shows both starts");
    }
    let new = plat::read_file(&format!("{}/real-2.0.0.AppImage", src));
    t.check(plat::read_file(&prog) == new, "apply appimage: the .AppImage file is the signed 2.0.0");
    plat::setenv("APPIMAGE", Some(&prog));
    let bad = config(s, dir, "aiapply-bad", "2.0.0", "ai-state");
    let u = Updater::new(bad.clone());
    t.check(matches!(u.check(), Ok(Check::Available(o)) if o.version == "3.0.0") && u.download().is_ok(), "apply appimage: 3.0.0 downloaded");
    plat::setenv("APPIMAGE", None);
    let args = ["--child", "--state", &state, "--log", &log, "--url", &bad.url, "--pubkey", &s.key];
    run_installed(t, &prog, &args, &env, &log, "apply appimage: 2.0.0 puts 3.0.0 in place; 3.0.0 fails before healthy()", "crash 3.0.0");
    if run_installed(t, &prog, &args, &env, &log, "apply appimage: the next start rolls back to the 2.0.0 AppImage and restarts it", "healthy 2.0.0 false") {
        t.check(read_text(&log).unwrap_or_default().contains("startup 2.0.0 rolledback 3.0.0"), "apply appimage: ... 2.0.0 learns that 3.0.0 failed");
    }
    t.check(plat::read_file(&prog) == new, "apply appimage: the .AppImage file is 2.0.0 again");
}

// ---------------------------------------------------------------- --child

// the version and mode the server appended to a program ("#JFUPDTEST <version> <mode>#")
fn marker_of(path: &str) -> (String, String) {
    let none = (String::from("1.0.0"), String::from("ok"));
    let Some(f) = plat::File::open(path, plat::FO_READ) else { return none };
    let size = f.size().unwrap_or(0);
    let mut tail = [0u8; 64];
    let off = size.saturating_sub(64);
    let n = f.read_at(off, &mut tail[..(size - off) as usize]).unwrap_or(0);
    let tail = &tail[..n];
    if tail.last() != Some(&b'#') {
        return none;
    }
    let Some(i) = tail.windows(11).rposition(|w| w == b"#JFUPDTEST ") else { return none };
    let text = String::from_utf8_lossy(&tail[i + 11..tail.len() - 1]).into_owned();
    match text.split_once(' ') {
        Some((v, m)) => (String::from(v), String::from(m)),
        None => none,
    }
}

fn append(path: &str, line: &str) {
    if let Some(f) = plat::File::open(path, plat::FO_RW) {
        let end = f.size().unwrap_or(0);
        f.write_at(end, line.as_bytes());
    }
}

fn startup_text(s: &Startup) -> String {
    match s {
        Startup::Normal => String::from("normal"),
        Startup::Applied(v) => format!("applied {}", v),
        Startup::Trial(v) => format!("trial {}", v),
        Startup::RollingBack(v) => format!("rollingback {}", v),
        Startup::RolledBack(v) => format!("rolledback {}", v),
        Startup::Failed(e) => format!("failed {}", e),
    }
}

fn child() -> i32 {
    let arg = |n: &str| plat::arg_value(n).unwrap_or_default();
    let (log, url, key) = (arg("--log"), arg("--url"), arg("--pubkey"));
    let (version, mode) = marker_of(&plat::current_exe().unwrap_or_default());
    let mut cfg = Config::new(APP, &version, &url, &[&key]);
    cfg.dir = arg("--state");
    append(&log, &format!("run {}\n", version));
    let st = update::startup(&cfg);
    append(&log, &format!("startup {} {}\n", version, startup_text(&st)));
    if mode == "crash" {
        append(&log, &format!("crash {}\n", version));
        return 7;
    }
    let ok = update::healthy(&cfg);
    append(&log, &format!("healthy {} {}\n", version, ok));
    0
}
