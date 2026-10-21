// t_spawn -- (plat-win) plattest --test spawn: plat::spawn_self(--test spawned --marker FILE),
// then wait (up to 10 s) for the new process to write FILE; --test spawned is that process
//                                         [apps/plattest/src/main.s t_spawn, t_spawned]

use plat::{file_exists, sleep_ms, spawn_self, write_file};

use crate::T;

pub fn t_spawn(t: &mut T) {
    t.scratch_init();
    let marker = t.subp("spawned.txt");
    let r = spawn_self(&["--test", "spawned", "--marker", &marker]);
    t.check(r == 0, "spawn: plat_spawn_self starts the process");
    for _ in 0..200 {
        // 200 x 50 ms
        if file_exists(&marker) == 1 {
            break;
        }
        sleep_ms(50);
    }
    t.check(file_exists(&marker) == 1, "spawn: the new process wrote its marker file");
    t.scratch_done();
}

/// the spawned process: write the --marker file (no checks of its own)
pub fn t_spawned(_: &mut T) {
    if let Some(m) = plat::arg_value("--marker") {
        write_file(&m, b"spawned\n");
    }
}
