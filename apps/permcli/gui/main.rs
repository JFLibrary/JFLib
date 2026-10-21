// permcli-gui -- a gui program of BINS (ui::main!, the app's style): it gets the app's
// apps/permcli/PERMISSIONS like the main program.  Started by permcli with PERMCLI_DIR; opens
// no window, prints PASS / FAIL, exit code = failures.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use permcheck::T;

ui::main!(run());

fn run() -> i32 {
    let mut t = T::new("permcli-gui");
    let Some(dir) = plat::env("PERMCLI_DIR") else {
        t.check(false, "PERMCLI_DIR is set");
        return t.done();
    };
    permcheck::common(&mut t, &dir, "shared.txt");
    t.done()
}
