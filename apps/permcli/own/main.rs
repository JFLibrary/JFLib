// permcli-own -- a console program of BINS with a manifest of its own
// (apps/permcli/PERMISSIONS.permcli-own): own.txt of the scratch folder, the file named on the
// command line (fs.user), nothing else -- the app's PERMISSIONS would allow the whole folder.
// Started by permcli with PERMCLI_DIR; prints PASS / FAIL, exit code = failures.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;

use permcheck::T;

rt::main!(run());

fn run() -> i32 {
    let mut t = T::new("permcli-own");
    let Some(dir) = plat::env("PERMCLI_DIR") else {
        t.check(false, "PERMCLI_DIR is set");
        return t.done();
    };
    permcheck::common(&mut t, &dir, "own.txt");
    t.check(plat::read_file(&format!("{dir}/arg.txt")).as_deref() == Some(&b"arg.txt"[..]), "the command line's file (fs.user)");
    t.check(plat::read_file(&format!("{dir}/secret.txt")).is_none(), "secret.txt: only the program's own rules count");
    t.check(plat::read_file(&format!("{dir}/shared.txt")).is_none(), "shared.txt: neither");
    t.check(!plat::write_file(&format!("{dir}/own.txt"), b"x"), "own.txt is read only");
    t.done()
}
