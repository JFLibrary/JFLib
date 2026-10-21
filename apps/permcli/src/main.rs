// permcli -- the permission manifest in console programs and in the further executables of an
// app (rt::main!, apps/NAME/BINS): every program installs its manifest before its own code runs
//
//   permcli            makes a scratch folder $TEMP/permcli-* (shared.txt, own.txt, arg.txt,
//                      secret.txt), checks its own manifest (apps/permcli/PERMISSIONS: installed
//                      by rt::main!, the app's name and assets, a system file refused), then
//                      starts its BINS programs (PERMCLI_DIR = the folder) and passes on their
//                      lines:
//     permcli-own      (rt::main!) its own PERMISSIONS.permcli-own: own.txt only, and with
//                      fs.user the file named on its command line (arg.txt), not secret.txt
//     permcli-shared   (rt::main!) no file of its own: the app's PERMISSIONS
//     permcli-gui      (ui::main!, a gui program of BINS) the app's PERMISSIONS
//   Prints PASS / FAIL; exit code = failures.  (build_rs.py test runs it through
//   tests/manifest.uis; --script is ignored)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use permcheck::T;
use plat::policy;

rt::main!(run());

fn run() -> i32 {
    let mut t = T::new("permcli");
    let temp = plat::temp_dir().unwrap_or_default().replace('\\', "/");
    let dir = format!("{}/permcli-{:x}", temp, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&dir), "mkdir $TEMP/permcli-* (fs.write)");
    for f in ["shared.txt", "own.txt", "arg.txt", "secret.txt"] {
        t.check(plat::write_file(&format!("{dir}/{f}"), f.as_bytes()), &format!("write {f}"));
    }
    permcheck::common(&mut t, &dir, "shared.txt");
    t.check(!plat::write_file(&format!("{temp}/other-permcli.txt"), b"x"), "the rest of $TEMP is refused");
    t.check(!policy::process_allowed("whoami"), "a program the manifest does not name");

    let exe = plat::current_exe().unwrap_or_default().replace('\\', "/");
    let bin_dir = &exe[..exe.rfind('/').unwrap_or(0)];
    let arg = format!("{dir}/arg.txt");
    for (bin, args) in [("permcli-own", alloc::vec![arg.as_str()]), ("permcli-shared", Vec::new()), ("permcli-gui", Vec::new())] {
        let program = format!("{bin_dir}/{bin}{}", if cfg!(windows) { ".exe" } else { "" });
        match start(&program, &args, &dir) {
            Some((code, out)) => {
                plat::write_stdout(out.as_bytes());
                t.check(code == 0, &format!("{bin} passed (exit code {code})"));
            }
            None => t.check(false, &format!("{bin} started and ended")),
        }
    }

    for f in ["shared.txt", "own.txt", "arg.txt", "secret.txt"] {
        plat::delete(&format!("{dir}/{f}"));
    }
    t.check(plat::delete(&dir), "the scratch folder is removed");
    t.done()
}

// start program with PERMCLI_DIR = dir -> its exit code and standard output (None: it did not
// start or ran longer than two minutes)
fn start(program: &str, args: &[&str], dir: &str) -> Option<(i32, String)> {
    let env = [("PERMCLI_DIR", Some(dir))];
    let mut c = plat::process::Command { program, args, cwd: None, env: &env }.spawn().ok()?;
    let mut out = Vec::new();
    let mut b = [0u8; 4096];
    let deadline = plat::time_us() + 120_000_000;
    loop {
        while let Ok(Some(n)) = c.read_stdout(&mut b) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&b[..n]);
        }
        while let Ok(Some(n)) = c.read_stderr(&mut b) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&b[..n]);
        }
        match c.try_wait() {
            Ok(Some(code)) => {
                while let Ok(Some(n)) = c.read_stdout(&mut b) {
                    if n == 0 {
                        break;
                    }
                    out.extend_from_slice(&b[..n]);
                }
                return Some((code, String::from_utf8_lossy(&out).into_owned()));
            }
            Ok(None) if plat::time_us() < deadline => plat::sleep_ms(5),
            _ => {
                c.terminate();
                return None;
            }
        }
    }
}
