// permcli-shared -- a console program of BINS without a manifest of its own: it gets the app's
// apps/permcli/PERMISSIONS.  Started by permcli with PERMCLI_DIR; prints PASS / FAIL, exit
// code = failures.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;

use permcheck::T;

rt::main!(run());

fn run() -> i32 {
    let mut t = T::new("permcli-shared");
    let Some(dir) = plat::env("PERMCLI_DIR") else {
        t.check(false, "PERMCLI_DIR is set");
        return t.done();
    };
    permcheck::common(&mut t, &dir, "shared.txt");
    t.check(plat::read_file(&format!("{dir}/secret.txt")).as_deref() == Some(&b"secret.txt"[..]), "secret.txt: the app's rules allow the whole folder");
    t.done()
}
