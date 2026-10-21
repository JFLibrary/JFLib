// permtest -- the permission manifest end to end: apps/permtest/PERMISSIONS goes through
// build_rs.py (JFGUI_PERMISSIONS) into ui::main!, which installs it before this code runs
//
//   permtest FILE     FILE (named on the command line: the user's, fs.user) may be written,
//                     a neighbour of it may not; $TEMP/permtest-* may be written, the rest
//                     of $TEMP not; print, secrets and the listed URL schemes are allowed,
//                     plugins, other schemes and the network (no net.* rule) are not.  Prints
//                     PASS / FAIL; exit code = failures.
//   (build_rs.py test runs it through tests/manifest.uis; --script is ignored)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;

use plat::policy;

ui::main!(permtest());

struct T {
    fails: i32,
}

impl T {
    fn check(&mut self, ok: bool, name: &str) {
        if !ok {
            self.fails += 1;
        }
        plat::write_stdout(format!("{} {}\n", if ok { "PASS" } else { "FAIL" }, name).as_bytes());
    }
}

fn permtest() -> i32 {
    let mut t = T { fails: 0 };
    t.check(policy::active(), "permtest: ui::main! installed the manifest");
    let args = plat::args();
    let Some(file) = args.iter().skip(1).find(|a| !a.starts_with('-') && !a.ends_with(".uis")).cloned() else {
        t.check(false, "permtest: usage: permtest FILE");
        return t.fails;
    };

    // the command line's file (fs.user), not its neighbour
    t.check(plat::write_file(&file, b"granted"), "permtest: the command line's file can be written");
    t.check(plat::read_file(&file).as_deref() == Some(&b"granted"[..]), "permtest: ... and read");
    let other = format!("{}.other", file);
    t.check(!plat::write_file(&other, b"x"), "permtest: its neighbour cannot be written");
    t.check(plat::delete(&file), "permtest: the command line's file can be deleted");

    // $TEMP/permtest-*/** only
    let temp = plat::temp_dir().unwrap_or_default();
    let dir = format!("{}/permtest-{:x}", temp, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&dir), "permtest: mkdir $TEMP/permtest-*");
    let f = format!("{dir}/a.txt");
    t.check(plat::write_file(&f, b"temp") && plat::read_file(&f).is_some(), "permtest: write and read below it");
    t.check(plat::delete(&f) && plat::delete(&dir), "permtest: delete below it");
    t.check(!plat::write_file(&format!("{}/other-permtest.txt", temp), b"x"), "permtest: the rest of $TEMP is refused");

    // always readable: the exe folder; nothing else of the repository
    let exe = plat::current_exe().unwrap_or_default();
    t.check(plat::file_exists(&exe) == 1, "permtest: the exe folder is readable");
    t.check(plat::read_file("README.md").is_none(), "permtest: other files are refused");

    // the bundled assets (apps/permtest/assets as app://) and the app's own folders, which the
    // manifest does not name
    t.check(plat::read_file("app://hello.txt").as_deref() == Some(&b"Hallo aus app://\n"[..]), "permtest: app://hello.txt from the bundle");
    let rep = plat::read_file("app://docs/repeat.txt");
    t.check(rep.as_ref().is_some_and(|r| r.len() == 7000 && r.starts_with(b"JFLib bundles")), "permtest: an LZ4-packed asset");
    t.check(plat::file_exists("app://docs/deep/empty.bin") == 1 && plat::file_exists("app://.hidden") == 0, "permtest: an empty asset; hidden folders are not packed");
    t.check(!plat::write_file("app://hello.txt", b"x"), "permtest: app:// is read-only");
    for scheme in ["config", "data", "cache"] {
        let name = format!("{scheme}://permtest-check.txt");
        let ok = plat::write_file(&name, scheme.as_bytes()) && plat::read_file(&name).as_deref() == Some(scheme.as_bytes()) && plat::delete(&name);
        t.check(ok, &format!("permtest: {scheme}:// is always writable"));
    }

    // no net.* rule: no connection, no listener, no name lookup
    let lo = plat::net::SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 1);
    let refused = |e: plat::net::NetError| e.kind == plat::net::NetErrorKind::Denied;
    t.check(plat::net::TcpStream::connect(&lo, 500).err().is_some_and(refused), "permtest: no connection without net.connect");
    t.check(plat::net::TcpListener::bind(&lo).err().is_some_and(refused), "permtest: no listener without net.listen");
    t.check(!policy::net_name_allowed("example.org"), "permtest: no name lookup without net.connect");

    // the other permissions
    t.check(policy::url_allowed("mailto:a@b") && !policy::url_allowed("file:///etc/passwd"), "permtest: url https mailto");
    t.check(policy::secrets_allowed("permtest"), "permtest: secrets (the manifest's key, the store itself is not used)");
    let caps = plat::capabilities();
    t.check(caps.supports(plat::CAP_PRINT | plat::CAP_FILE_WRITE), "permtest: print and files in capabilities()");
    t.check(!caps.supports(plat::CAP_NATIVE_PLUGINS), "permtest: no plugins in capabilities()");
    let d = policy::denials();
    t.check(d.iter().any(|x| x.kind == "fs.write" && x.target == other), "permtest: the refusal is recorded");
    plat::write_stdout(format!("permtest: {} failure(s)\n", t.fails).as_bytes());
    t.fails
}
