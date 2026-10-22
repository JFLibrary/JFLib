// sandboxtest -- the OS sandbox end to end (docs/PERMISSIONS.md "OS sandbox").  The manifest
// (apps/sandboxtest/PERMISSIONS) opts in with `sandbox strict` and grants only a scratch
// folder; ui::main! installs it and plat::sandbox hands it to the kernel before this code runs.
// The checks then confirm, with raw system calls that bypass the in-process policy, that the OS
// itself blocks what the manifest forbids and still allows what it needs.
//
//   sandboxtest        prints PASS / FAIL and ends with the number of failures.  Checks guarded
//                      by plat::sandbox::status(): a layer the kernel does not support is
//                      reported and skipped, not failed.  (build_rs.py test runs it through
//                      tests/sandbox.uis; the body returns before the UI starts, so --script is
//                      ignored, as in permtest.)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use plat::sandbox;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod t_linux;
#[cfg(windows)]
#[allow(unsafe_code)]
mod t_win;

ui::main!(sandboxtest());

/// PASS / FAIL counter
pub struct T {
    fails: i32,
}

impl T {
    pub fn check(&mut self, ok: bool, name: &str) {
        if !ok {
            self.fails += 1;
        }
        plat::write_stdout(format!("{} {}\n", if ok { "PASS" } else { "FAIL" }, name).as_bytes());
    }
    /// a check that is skipped when the kernel does not offer the layer (reported, not failed)
    pub fn skip(&mut self, name: &str) {
        plat::write_stdout(format!("SKIP {}\n", name).as_bytes());
    }
}

// a scratch folder the manifest grants ($TEMP/sandboxtest-*)
pub fn scratch() -> String {
    let temp = plat::temp_dir().unwrap_or_default().replace('\\', "/");
    format!("{}/sandboxtest-{:x}", temp, plat::time_us() & 0xFF_FFFF)
}

fn sandboxtest() -> i32 {
    let mut t = T { fails: 0 };
    t.check(plat::policy::active(), "sandboxtest: the manifest is installed");
    let st = sandbox::status();
    t.check(st.level == 2, "sandboxtest: sandbox strict is active");
    plat::write_stdout(
        format!(
            "sandbox: level={} landlock_abi={} landlock_fs={} landlock_net={} seccomp={} exec_denied={} win_mitigations={} win_dynamic_code={}\n",
            st.level, st.landlock_abi, st.landlock_fs, st.landlock_net, st.seccomp, st.exec_denied, st.win_mitigations, st.win_dynamic_code
        )
        .as_bytes(),
    );

    // the scratch folder the manifest grants: allowed through plat and through the OS
    let dir = scratch();
    t.check(plat::mkdir(&dir), "sandboxtest: mkdir the granted scratch folder");
    let f = format!("{dir}/a.txt");
    t.check(plat::write_file(&f, b"ok") && plat::read_file(&f).as_deref() == Some(&b"ok"[..]), "sandboxtest: write and read in it");
    // the app's own folders need no rule; the sandbox creates them before granting them
    for scheme in ["config", "data", "cache"] {
        let name = format!("{scheme}://sandboxtest-check.txt");
        let ok = plat::write_file(&name, scheme.as_bytes()) && plat::read_file(&name).as_deref() == Some(scheme.as_bytes()) && plat::delete(&name);
        t.check(ok, &format!("sandboxtest: {scheme}:// is writable inside the sandbox"));
    }

    #[cfg(target_os = "linux")]
    t_linux::checks(&mut t, &dir, &st);
    #[cfg(windows)]
    t_win::checks(&mut t, &dir, &st);

    plat::delete(&f);
    plat::delete(&dir);
    plat::write_stdout(format!("sandboxtest: {} failure(s)\n", t.fails).as_bytes());
    t.fails
}
