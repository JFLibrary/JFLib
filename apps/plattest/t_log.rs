// t_log -- the log (lib/plat/log.rs)
//
//   plattest --test log        levels and log! (formats only when a sink wants the level),
//                              the ring (whole lines, folded line breaks, at most 64 KiB, the
//                              newest kept), the file (rotation at 4 KiB into .1 / .2, never a
//                              .3, stopping; on mem:// at once, a mount without streams
//                              refused); a child (--test log-crash) logs and panics: exit
//                              code 3, its stderr ends with the last log lines, its file with
//                              the crash; three more children crash where the pre-hook must
//                              neither allocate nor wait (a file at its limit, a panic under
//                              the mount table with a mount's log, Linux: a stuck stderr)
//   plattest --test log-crash  (the child) the file of jfgui_LOG_TEST_FILE, a line, a panic
//   plattest --test log-crash-full / log-crash-mount / log-crash-stderr   (children)

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::vfs;
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
    // a mount's own stream gets each line at once; a provider without streams or OS files
    // would get them only when the log stops: refused
    t.check(log_file(Some("mem://logs/m.log"), 4096, 0), "log: log_file on mem://");
    log_info("file", "on mem");
    let m = String::from_utf8_lossy(&plat::read_file("mem://logs/m.log").unwrap_or_default()).into_owned();
    t.check(m.contains("INFO  file: on mem\n"), "log: ... the line is there at once");
    vfs::mount("nostream", Arc::new(NoStreams(Arc::new(vfs::MemFs::new()))));
    t.check(!log_file(Some("nostream://app.log"), 4096, 0), "log: log_file on a provider without streams or OS files is refused");
    t.check(log_file(None, 0, 0), "log: ... stopped");
    vfs::unmount("nostream");
    plat::delete("mem://logs/m.log");

    // a crash: the last lines on stderr, the crash in the file
    let cp = t.subp("logs/crash.log");
    let (code, err) = crash_child("log-crash", &cp);
    t.check(code == Some(3), "log: the crashing child ends with exit code 3");
    t.check(err.contains("--- the last log lines ---\n") && err.contains("INFO  child: before the crash\n"), "log: stderr has the last log lines");
    let f = String::from_utf8_lossy(&plat::read_file(&cp).unwrap_or_default()).into_owned();
    t.check(f.contains("INFO  child: before the crash\n") && f.contains("--- crash: kind 7") && f.contains("a planned crash"), "log: the file ends with the crash");
    if code != Some(3) || !err.contains("--- the last") {
        t.info(&format!("  child: {code:?}, stderr:\n{err}\n"));
    }
    crash_paths(t);
    t.scratch_done();
}

// a provider with read and write only (no streams, no OS files)
struct NoStreams(Arc<vfs::MemFs>);

impl vfs::Provider for NoStreams {
    fn kind(&self, p: &str) -> u32 {
        self.0.kind(p)
    }
    fn read(&self, p: &str) -> Option<Vec<u8>> {
        self.0.read(p)
    }
    fn list(&self, p: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.0.list(p, f)
    }
    fn write(&self, p: &str, d: &[u8]) -> bool {
        self.0.write(p, d)
    }
}

// the crash pre-hook neither allocates nor waits (Codex A013-C003, V054-C001): each child
// crashes where the old pre-hook rotated the file or waited on stderr
fn crash_paths(t: &mut T) {
    // a file one byte below its limit: the crash line stays in it (rotation, keep 0, deleted it)
    let fp = t.subp("logs/full.log");
    let (code, out) = crash_child("log-crash-full", &fp);
    let f = String::from_utf8_lossy(&plat::read_file(&fp).unwrap_or_default()).into_owned();
    t.check(code == Some(3) && out.contains("app hook reached"), "log: a crash at the file's limit reaches the app's hook");
    t.check(f.len() > 4096 && f.contains("--- crash: kind 7") && f.contains("at the limit"), "log: ... its crash line stays in the file (no rotation on the crash path)");
    // a panic while the mount table is held, the log on a mount's stream: no rotation that
    // routes a name through the held table (the old pre-hook spun on it for good)
    let (code, out) = crash_child("log-crash-mount", "");
    t.check(code == Some(3) && out.contains("app hook reached"), "log: a panic under the mount table with a mount's log reaches the app's hook");
    if code != Some(3) || !out.contains("app hook reached") {
        t.info(&format!("  child: {code:?}, output:\n{out}\n"));
    }
    // Linux: stderr a full non-blocking pipe nobody reads: given up after a while (the old
    // writer polled without a timeout)
    #[cfg(target_os = "linux")]
    {
        let t0 = plat::time_us();
        let (code, out) = crash_child("log-crash-stderr", "");
        let ms = (plat::time_us() - t0) / 1000;
        t.check(code == Some(3) && out.contains("app hook reached"), "log: a crash with a stuck non-blocking stderr reaches the app's hook");
        t.check(ms < 8000, &format!("log: ... in {ms} ms (a stuck stderr is given up)"));
    }
}

// run a child of t_log with jfgui_LOG_TEST_FILE = file; (exit code, stderr and stdout); a child
// still running after 10 s is ended (None)
fn crash_child(test: &str, file: &str) -> (Option<i32>, String) {
    let exe = plat::current_exe().unwrap_or_default();
    let args = ["--test", test];
    let env = [("jfgui_LOG_TEST_FILE", Some(file)), ("jfgui_LOG", None)];
    let cmd = plat::process::Command { program: &exe, args: &args, cwd: None, env: &env };
    let Ok(mut c) = cmd.spawn() else { return (None, String::from("the child did not start")) };
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let mut code = None;
    for _ in 0..1000 {
        while let Ok(Some(n)) = c.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        while let Ok(Some(n)) = c.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        if code.is_some() {
            break;
        }
        code = c.try_wait().ok().flatten();
        if code.is_none() {
            plat::sleep_ms(10);
        }
    }
    (code, String::from_utf8_lossy(&out).into_owned()) // dropping c ends a hung child
}

/// the child of t_log: a file, a line, a panic
pub fn t_log_crash(_: &mut T) {
    log_levels(LOG_OFF, LOG_INFO);
    let p = plat::env("jfgui_LOG_TEST_FILE").unwrap_or_default();
    log_file(Some(&p), 1 << 20, 1);
    log_info("child", "before the crash");
    panic!("a planned crash");
}

// the app's hook of the crash_paths children: a line on stdout (stderr may be the stuck pipe)
fn app_hook(_: &rt::crash::CrashInfo) {
    plat::write_stdout(b"app hook reached\n");
}

// the pre-hook installed by a line while no file is set (a file line at its limit would rotate)
fn hook_without_file() {
    log_levels(LOG_OFF, LOG_INFO);
    log_info("child", "the pre-hook is in");
    rt::crash::set_hook(Some(app_hook));
}

/// (child) a log file one byte below its limit (keep 0), then a panic
pub fn t_log_crash_full(_: &mut T) {
    hook_without_file();
    let p = plat::env("jfgui_LOG_TEST_FILE").unwrap_or_default();
    let mut fill = [b'x'; 4095];
    fill[4094] = b'\n';
    plat::write_file(&p, &fill);
    log_file(Some(&p), 4096, 0);
    panic!("a planned crash at the limit");
}

// a provider whose drop panics: unmount drops it while it holds the mount table
struct PanicOnDrop(Arc<vfs::MemFs>);

impl vfs::Provider for PanicOnDrop {
    fn kind(&self, p: &str) -> u32 {
        self.0.kind(p)
    }
    fn read(&self, p: &str) -> Option<Vec<u8>> {
        self.0.read(p)
    }
    fn list(&self, p: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.0.list(p, f)
    }
}

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("a planned crash under the mount table");
    }
}

/// (child) the log on mem:// one byte below its limit, then a panic inside unmount
pub fn t_log_crash_mount(_: &mut T) {
    hook_without_file();
    let mut fill = [b'x'; 4095];
    fill[4094] = b'\n';
    plat::write_file("mem://log", &fill);
    log_file(Some("mem://log"), 4096, 0);
    vfs::mount("zz", Arc::new(PanicOnDrop(Arc::new(vfs::MemFs::new()))));
    vfs::unmount("zz");
    plat::write_stdout(b"unmount returned\n");
}

/// (child, Linux) stderr a full non-blocking pipe whose reader stays open and reads nothing,
/// then a panic
#[cfg(target_os = "linux")]
#[allow(unsafe_code)] // raw pipe2 / dup2 / write: plat has no call that makes stderr a pipe
pub fn t_log_crash_stderr(_: &mut T) {
    use rt::os::{SYS_PIPE2, SYS_WRITE, dup2, syscall3};
    hook_without_file();
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 writes two descriptors into fds (O_NONBLOCK | O_CLOEXEC); dup2 replaces
    // stderr, which nothing else uses here; the write reads from a valid buffer.
    unsafe {
        if syscall3(SYS_PIPE2, fds.as_mut_ptr() as usize, 0o4000 | 0o2000000, 0) < 0 || dup2(fds[1] as usize, 2) < 0 {
            plat::write_stdout(b"no pipe\n");
            return;
        }
        let block = [b'f'; 4096];
        while syscall3(SYS_WRITE, 2, block.as_ptr() as usize, block.len()) > 0 {}
    }
    // fds[0], the reader, stays open: a write waits for room instead of failing
    panic!("a planned crash with a stuck stderr");
}
