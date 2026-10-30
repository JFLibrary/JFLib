// t_spawn -- (plat-win) plattest --test spawn: plat::spawn_self(--test spawned --marker FILE),
// then wait (up to 10 s) for the new process to write FILE; --test spawned is that process
//                                         [apps/plattest/src/main.s t_spawn, t_spawned]
// plattest --test launch: plat::process::launch (a program on its own, the inspector's editor)
// starts plattest --test launched --marker FILE ARGS..., which writes its arguments to FILE,
// one per line: the words arrive as they were given (blanks, quotes, backslashes, an empty
// one, non-ASCII); process::find; a program that does not exist

use alloc::string::String;
use alloc::vec::Vec;

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

// the words process::launch passes on (each must arrive unchanged)
const WORDS: [&str; 6] = ["a b", "c\"d", "e\\f\\", "", "ü€", "--line"];

pub fn t_launch(t: &mut T) {
    t.scratch_init();
    let marker = t.subp("launched.txt");
    let exe = plat::current_exe().unwrap_or_default();
    t.check(!exe.is_empty(), "launch: the program's own path");
    let mut args = alloc::vec!["--test", "launched", "--marker", marker.as_str()];
    args.extend_from_slice(&WORDS);
    let r = plat::process::launch(&exe, &args);
    t.check(r.is_ok(), "launch: process::launch starts the program");
    let want: String = WORDS.iter().map(|w| alloc::format!("{}\n", w)).collect();
    let mut got = String::new();
    for _ in 0..200 {
        // 200 x 50 ms (the file may exist before all of it is written)
        got = plat::read_file(&marker).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if got == want {
            break;
        }
        sleep_ms(50);
    }
    t.check(got == want, "launch: the program got every word as it was given");
    // a missing program: its error number, nothing started
    let e = plat::process::launch("plattest-no-such-program-xyz", &[]).err();
    t.check(e == Some(2), "launch: a missing program -> 2 (not found)");
    // find: a name looked up like launch does; paths and missing names are None
    let sys = if cfg!(windows) { "cmd" } else { "sh" };
    t.check(plat::process::find(sys).is_some_and(|p| p.contains(['/', '\\'])), "launch: find of a system program");
    t.check(plat::process::find("plattest-no-such-program-xyz").is_none(), "launch: find of a missing name");
    t.check(plat::process::find(&exe).is_none(), "launch: find of a path is None");
    t.scratch_done();
}

/// the launched process: write the words after --marker FILE to FILE, one per line
pub fn t_launched(_: &mut T) {
    let a = plat::args();
    let Some(k) = a.iter().position(|s| s == "--marker") else { return };
    let Some(m) = a.get(k + 1) else { return };
    let words: Vec<String> = a[k + 2..].iter().map(|w| alloc::format!("{}\n", w)).collect();
    write_file(m, words.concat().as_bytes());
}
