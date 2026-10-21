// permcheck -- what the programs of permcli share: PASS / FAIL lines and the effects of the
// manifest that every program of an app must show (rt::main!, ui::main!, BINS)
//
//   T::new(who) / check(ok, name) / done() -> failures
//   common(t, dir, readable)   the manifest is installed, the app's name and assets, a file of
//                              the scratch folder the rules allow, a system file they do not
//                              allow (refused and recorded in policy::denials)
//   forbidden() -> path        that system file: %WINDIR%/System32/drivers/etc/hosts, /etc/passwd
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use plat::policy;

/// the results of one program
pub struct T {
    who: &'static str,
    fails: i32,
}

impl T {
    pub fn new(who: &'static str) -> T {
        T { who, fails: 0 }
    }

    pub fn check(&mut self, ok: bool, name: &str) {
        if !ok {
            self.fails += 1;
        }
        plat::write_stdout(format!("{} {}: {}\n", if ok { "PASS" } else { "FAIL" }, self.who, name).as_bytes());
    }

    /// the summary line -> the number of failures (the exit code)
    pub fn done(self) -> i32 {
        plat::write_stdout(format!("{}: {} failure(s)\n", self.who, self.fails).as_bytes());
        self.fails
    }
}

/// a file every system has and no rule of permcli names (Linux: /etc/hosts is always readable)
pub fn forbidden() -> String {
    if cfg!(windows) {
        format!("{}/System32/drivers/etc/hosts", plat::env("WINDIR").unwrap_or_else(|| String::from("C:/Windows")))
    } else {
        String::from("/etc/passwd")
    }
}

/// the checks of every program: dir is the scratch folder of the main program, readable a file
/// in it the program's rules allow (its content is its name)
pub fn common(t: &mut T, dir: &str, readable: &str) {
    t.check(policy::active(), "the manifest is installed");
    t.check(plat::vfs::app_id() == "permcli", "the app's name for its folders (JFGUI_APP)");
    t.check(plat::read_file("app://hello.txt").as_deref() == Some(&b"Hallo aus app://"[..]), "app:// holds the app's assets");
    let ok = format!("{dir}/{readable}");
    t.check(plat::read_file(&ok).as_deref() == Some(readable.as_bytes()), &format!("{readable}: the rules allow it"));
    let bad = forbidden();
    t.check(plat::read_file(&bad).is_none(), "a system file outside the rules is not read");
    t.check(policy::denials().iter().any(|d| d.kind == "fs.read" && d.target == bad), "... because the manifest refused it");
}
