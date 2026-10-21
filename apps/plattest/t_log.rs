// t_log -- the log (lib/plat/log.rs)
//
//   plattest --test log        levels and log! (formats only when a sink wants the level),
//                              the ring (whole lines, folded line breaks, at most 64 KiB, the
//                              newest kept), the file (rotation at 4 KiB into .1 / .2, never a
//                              .3, stopping); a child (--test log-crash) logs and panics: exit
//                              code 3, its stderr ends with the last log lines, its file with
//                              the crash
//   plattest --test log-crash  (the child) the file of jfgui_LOG_TEST_FILE, a line, a panic

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use plat::{LOG_DEBUG, LOG_INFO, LOG_OFF, LOG_TRACE, log_debug, log_file, log_info, log_levels, log_recent, log_warn};

use crate::T;

static FORMATS: AtomicU32 = AtomicU32::new(0);

fn costly() -> u32 {
    FORMATS.fetch_add(1, Ordering::Relaxed)
}

fn size(p: &str) -> u64 {
    plat::read_file(p).map_or(0, |b| b.len() as u64)
}

pub fn t_log(t: &mut T) {
    t.scratch_init();
    log_levels(LOG_OFF, LOG_INFO); // nothing on stderr; the ring keeps up to DEBUG
    log_info("t", "eins");
    log_debug("t", "zwei");
    plat::log!(LOG_TRACE, "t", "drei {}", costly());
    t.check(FORMATS.load(Ordering::Relaxed) == 0, "log: a level no sink wants is not formatted");
    plat::log!(LOG_DEBUG, "t", "vier {}", costly());
    t.check(FORMATS.load(Ordering::Relaxed) == 1, "log: log! formats a wanted level");
    t.check(plat::log_enabled(LOG_DEBUG) && !plat::log_enabled(LOG_TRACE) && !plat::log_enabled(LOG_OFF), "log: log_enabled");
    log_warn("t", "a\nb\r\nc");
    let r = log_recent(4096);
    t.check(r.contains("INFO  t: eins\n") && r.contains("DEBUG t: zwei\n") && r.contains("DEBUG t: vier 0\n"), "log: the ring has the lines with level and target");
    t.check(!r.contains("drei"), "log: TRACE is not kept");
    t.check(r.contains("WARN  t: a | b | c\n"), "log: line breaks are folded");
    let first = r.lines().next().unwrap_or("");
    t.check(first.len() > 24 && first.as_bytes()[4] == b'-' && first.as_bytes()[13] == b':' && first.as_bytes()[19] == b'.', "log: lines start with the time");
    // the ring keeps the newest 64 KiB, from a whole line
    for k in 0..2000 {
        log_info("ring", &format!("{k:05} {}", "x".repeat(80)));
    }
    let r = log_recent(1 << 20);
    t.check(r.len() <= 64 << 10 && r.len() > 60 << 10, "log: the ring holds 64 KiB");
    t.check(r.ends_with(&format!("01999 {}\n", "x".repeat(80))) && !r.contains("ring: 00000 "), "log: the newest lines stay");
    t.check(r.starts_with("20") && r.lines().all(|l| l.contains(" ring: ") || l.contains(" t: ")), "log: only whole lines");
    t.check(log_recent(500).len() <= 500 && log_recent(500).ends_with('\n'), "log: log_recent(max) gives whole lines");

    // the file and its rotation
    let p = t.subp("logs/app.log");
    t.check(log_file(Some(&p), 4096, 2), "log: log_file opens (and makes its folder)");
    for k in 0..45 {
        log_info("file", &format!("{k:03} {}", "y".repeat(60)));
    }
    t.check(plat::file_exists(&format!("{p}.1")) == 1 && size(&p) < 4096, "log: rotated at 4 KiB into .1");
    for k in 0..90 {
        log_info("file", &format!("{k:03} {}", "z".repeat(60)));
    }
    t.check(plat::file_exists(&format!("{p}.2")) == 1 && plat::file_exists(&format!("{p}.3")) == 0, "log: two older files at most");
    let text = String::from_utf8(plat::read_file(&p).unwrap_or_default()).unwrap_or_default();
    t.check(text.lines().all(|l| l.contains(" INFO  file: ")), "log: the file has the lines");
    log_debug("file", "not in the file");
    t.check(!String::from_utf8_lossy(&plat::read_file(&p).unwrap_or_default()).contains("not in the file"), "log: the file keeps INFO and above");
    t.check(log_file(None, 0, 0), "log: log_file(None) stops");
    let before = size(&p);
    log_info("file", "after the stop");
    t.check(size(&p) == before, "log: no lines after the stop");

    // a crash: the last lines on stderr, the crash in the file
    let cp = t.subp("logs/crash.log");
    let exe = plat::current_exe().unwrap_or_default();
    let args = ["--test", "log-crash"];
    let env = [("jfgui_LOG_TEST_FILE", Some(cp.as_str())), ("jfgui_LOG", None)];
    let cmd = plat::process::Command { program: &exe, args: &args, cwd: None, env: &env };
    match cmd.spawn() {
        Ok(mut c) => {
            let mut err: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            let mut code = None;
            for _ in 0..1000 {
                while let Ok(Some(n)) = c.read_stderr(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    err.extend_from_slice(&buf[..n]);
                }
                while let Ok(Some(n)) = c.read_stdout(&mut buf) {
                    if n == 0 {
                        break;
                    }
                }
                code = c.try_wait().ok().flatten();
                if code.is_some() {
                    while let Ok(Some(n)) = c.read_stderr(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        err.extend_from_slice(&buf[..n]);
                    }
                    break;
                }
                plat::sleep_ms(10);
            }
            let err = String::from_utf8_lossy(&err);
            t.check(code == Some(3), "log: the crashing child ends with exit code 3");
            t.check(err.contains("--- the last log lines ---\n") && err.contains("INFO  child: before the crash\n"), "log: stderr has the last log lines");
            let f = String::from_utf8_lossy(&plat::read_file(&cp).unwrap_or_default()).into_owned();
            t.check(f.contains("INFO  child: before the crash\n") && f.contains("--- crash: kind 7") && f.contains("a planned crash"), "log: the file ends with the crash");
            if code != Some(3) || !err.contains("--- the last") {
                t.info(&format!("  child: {code:?}, stderr:\n{err}\n"));
            }
        }
        Err(e) => t.check(false, &format!("log: the child starts (error {e})")),
    }
    t.scratch_done();
}

/// the child of t_log: a file, a line, a panic
pub fn t_log_crash(_: &mut T) {
    log_levels(LOG_OFF, LOG_INFO);
    let p = plat::env("jfgui_LOG_TEST_FILE").unwrap_or_default();
    log_file(Some(&p), 1 << 20, 1);
    log_info("child", "before the crash");
    panic!("a planned crash");
}
