// log -- the process's log: levels, a ring of the newest lines for crash reports, stderr
// and a rotating file
//
//   LOG_ERROR 1, LOG_WARN 2, LOG_INFO 3, LOG_DEBUG 4, LOG_TRACE 5 (LOG_OFF 0)
//   log!(LOG_INFO, "net", "{} bytes from {}", n, host)   formats only when the level is on
//   log_line(level, target, msg)         one line; log_error / log_warn / log_info /
//                                        log_debug(target, msg) are short forms
//   log_quiet(level, target, msg)        ring and file only (the text went to stderr already)
//   log_enabled(level) -> bool           any sink wants this level
//   log_levels(stderr, file)             the levels of the sinks (default WARN / INFO; the
//                                        ring keeps everything up to DEBUG); jfgui_LOG=error |
//                                        warn | info | debug | trace sets both
//   log_file(Some(path), max_bytes, keep) -> bool   append the lines to path (a vfs name like
//                                        logs://app.log or an OS path); when it grows
//                                        beyond max_bytes it moves to path.1 (path.1 to path.2,
//                                        ... up to keep files) and a new one starts; None stops
//   log_recent(max_bytes) -> String      the newest whole lines of the ring (a crash report,
//                                        a "copy log" button)
//
// Lines: "2026-10-17 19:58:07.123 WARN  net: text" (Windows local time, elsewhere UTC with a
// Z); line breaks inside the text become " | ".  The ring holds 64 KiB.  A crash or panic
// writes the ring to stderr and to the log file before the app's crash hook runs (rt::crash
// pre-hook, installed with the first line); it takes the locks only if they are free, so a
// thread that crashed while logging cannot stop the report.

use alloc::string::String;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use rt::sync::SpinLock;

use crate::{File, FO_RW};

pub const LOG_OFF: u32 = 0;
pub const LOG_ERROR: u32 = 1;
pub const LOG_WARN: u32 = 2;
pub const LOG_INFO: u32 = 3;
pub const LOG_DEBUG: u32 = 4;
pub const LOG_TRACE: u32 = 5;

const RING: usize = 64 << 10;
const LINE_MAX: usize = 4096;
const NAMES: [&str; 6] = ["OFF  ", "ERROR", "WARN ", "INFO ", "DEBUG", "TRACE"];

struct Ring {
    buf: [u8; RING],
    /// bytes written so far (the write position is pos % RING)
    pos: usize,
}

struct Sink {
    file: Option<File>,
    path: String,
    size: u64,
    max: u64,
    keep: u32,
}

static RINGBUF: SpinLock<Ring> = SpinLock::new(Ring { buf: [0; RING], pos: 0 });
static FILE: SpinLock<Sink> = SpinLock::new(Sink { file: None, path: String::new(), size: 0, max: 0, keep: 0 });
// levels: stderr, file, ring; 0xFF = not read from jfgui_LOG yet
static LV_ERR: AtomicU32 = AtomicU32::new(0xFF);
static LV_FILE: AtomicU32 = AtomicU32::new(LOG_INFO);
static LV_RING: AtomicU32 = AtomicU32::new(LOG_DEBUG);
static HOOKED: AtomicBool = AtomicBool::new(false);

fn env_level() -> Option<u32> {
    let v = crate::env("jfgui_LOG")?;
    let v = v.trim().to_ascii_lowercase();
    Some(match v.as_str() {
        "off" | "0" => LOG_OFF,
        "error" | "1" => LOG_ERROR,
        "warn" | "warning" | "2" => LOG_WARN,
        "info" | "3" => LOG_INFO,
        "debug" | "4" => LOG_DEBUG,
        "trace" | "5" => LOG_TRACE,
        _ => return None,
    })
}

fn lv_err() -> u32 {
    let v = LV_ERR.load(Ordering::Relaxed);
    if v != 0xFF {
        return v;
    }
    let e = env_level();
    if let Some(l) = e {
        LV_FILE.store(l, Ordering::Relaxed);
        LV_RING.store(l.max(LOG_DEBUG), Ordering::Relaxed);
    }
    let l = e.unwrap_or(LOG_WARN);
    LV_ERR.store(l, Ordering::Relaxed);
    l
}

/// set the levels of stderr and the file (jfgui_LOG wins over both)
pub fn log_levels(stderr: u32, file: u32) {
    let e = env_level();
    LV_ERR.store(e.unwrap_or(stderr.min(LOG_TRACE)), Ordering::Relaxed);
    LV_FILE.store(e.unwrap_or(file.min(LOG_TRACE)), Ordering::Relaxed);
}

/// any sink wants lines of this level
pub fn log_enabled(level: u32) -> bool {
    level != LOG_OFF && (level <= lv_err() || level <= LV_RING.load(Ordering::Relaxed) || level <= LV_FILE.load(Ordering::Relaxed))
}

/// one line to every sink that wants its level
pub fn log_line(level: u32, target: &str, msg: &str) {
    emit(level, target, format_args!("{msg}"), true);
}

/// one line to the ring and the file only (its text went to stderr already)
pub fn log_quiet(level: u32, target: &str, msg: &str) {
    emit(level, target, format_args!("{msg}"), false);
}

/// a LOG_ERROR line
pub fn log_error(target: &str, msg: &str) {
    log_line(LOG_ERROR, target, msg);
}
/// a LOG_WARN line
pub fn log_warn(target: &str, msg: &str) {
    log_line(LOG_WARN, target, msg);
}
/// a LOG_INFO line
pub fn log_info(target: &str, msg: &str) {
    log_line(LOG_INFO, target, msg);
}
/// a LOG_DEBUG line
pub fn log_debug(target: &str, msg: &str) {
    log_line(LOG_DEBUG, target, msg);
}

/// (log!) a formatted line
#[doc(hidden)]
pub fn log_fmt(level: u32, target: &str, args: core::fmt::Arguments) {
    emit(level, target, args, true);
}

/// log!(LOG_INFO, "target", "format", args...): a line, formatted only when its level is on
#[macro_export]
macro_rules! log {
    ($level:expr, $target:expr, $($arg:tt)*) => {
        if $crate::log_enabled($level) {
            $crate::log_fmt($level, $target, format_args!($($arg)*));
        }
    };
}

// the line text: time, level, target, message (line breaks folded)
fn format_line(level: u32, target: &str, args: core::fmt::Arguments) -> String {
    let (d, utc) = crate::date();
    let mut s = String::with_capacity(96);
    let _ = write!(
        s,
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}{} {} {}: ",
        d.year,
        d.month,
        d.day,
        d.hour,
        d.min,
        d.sec,
        d.ms,
        if utc { "Z" } else { "" },
        NAMES[level.min(5) as usize],
        target
    );
    let start = s.len();
    let _ = s.write_fmt(args);
    if s[start..].contains(['\n', '\r']) {
        let body: String = s[start..].trim_end_matches(['\n', '\r']).replace("\r\n", "\n").replace(['\n', '\r'], " | ");
        s.truncate(start);
        s.push_str(&body);
    }
    if s.len() > LINE_MAX {
        let mut n = LINE_MAX;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
    s.push('\n');
    s
}

fn emit(level: u32, target: &str, args: core::fmt::Arguments, stderr: bool) {
    if level == LOG_OFF {
        return;
    }
    let to_err = stderr && level <= lv_err();
    let to_ring = level <= LV_RING.load(Ordering::Relaxed);
    let to_file = level <= LV_FILE.load(Ordering::Relaxed);
    if !(to_err || to_ring || to_file) {
        return;
    }
    if !HOOKED.swap(true, Ordering::Relaxed) {
        rt::crash::set_pre_hook(Some(crash_dump));
    }
    let line = format_line(level, target, args);
    if to_err {
        crate::write_stderr(line.as_bytes());
    }
    if to_ring {
        ring_put(line.as_bytes());
    }
    if to_file {
        file_put(line.as_bytes());
    }
}

fn ring_put(b: &[u8]) {
    let mut r = RINGBUF.lock();
    let b = if b.len() > RING { &b[b.len() - RING..] } else { b };
    let at = r.pos % RING;
    let first = b.len().min(RING - at);
    r.buf[at..at + first].copy_from_slice(&b[..first]);
    r.buf[..b.len() - first].copy_from_slice(&b[first..]);
    r.pos += b.len();
}

// the ring's bytes, oldest first, from its first whole line, to f in at most two pieces
fn ring_read(r: &Ring, max: usize, f: &mut dyn FnMut(&[u8])) {
    let n = r.pos.min(RING).min(max);
    if n == 0 {
        return;
    }
    let start = r.pos - n;
    let (a, b) = {
        let s = start % RING;
        if s + n <= RING { (&r.buf[s..s + n], &r.buf[..0]) } else { (&r.buf[s..], &r.buf[..s + n - RING]) }
    };
    // skip a cut first line (unless everything there is was written)
    let skip = if start == 0 { 0 } else { a.iter().chain(b.iter()).position(|&c| c == b'\n').map_or(n, |p| p + 1) };
    if skip < a.len() {
        f(&a[skip..]);
        f(b);
    } else {
        f(&b[skip - a.len()..]);
    }
}

/// the newest whole lines of the ring, at most max_bytes
pub fn log_recent(max_bytes: usize) -> String {
    let r = RINGBUF.lock();
    let mut out = alloc::vec::Vec::new();
    ring_read(&r, max_bytes, &mut |p| out.extend_from_slice(p));
    String::from_utf8_lossy(&out).into_owned()
}

/// write the lines to path (rotated at max_bytes, keep older files path.1 .. path.keep);
/// None stops -> false when the file cannot be opened
pub fn log_file(path: Option<&str>, max_bytes: u64, keep: u32) -> bool {
    let mut s = FILE.lock();
    s.file = None;
    s.path.clear();
    let Some(p) = path.filter(|p| !p.is_empty()) else { return true };
    if let Some(k) = p.rfind(['/', '\\']) {
        crate::mkdir(&p[..k]);
    }
    let Some(f) = File::open(p, FO_RW) else { return false };
    s.size = f.size().unwrap_or(0);
    s.file = Some(f);
    s.path = String::from(p);
    s.max = max_bytes.max(4096);
    s.keep = keep.min(20);
    true
}

fn file_put(b: &[u8]) {
    let mut s = FILE.lock();
    write_file(&mut s, b);
}

fn write_file(s: &mut Sink, b: &[u8]) {
    let Some(f) = s.file.as_ref() else { return };
    if f.write_at(s.size, b).is_some() {
        s.size += b.len() as u64;
    }
    if s.size >= s.max {
        rotate(s);
    }
}

// path -> path.1 -> path.2 ... (the oldest goes), then a new path
fn rotate(s: &mut Sink) {
    s.file = None;
    let p = s.path.clone();
    if s.keep == 0 {
        crate::delete(&p);
    } else {
        crate::delete(&alloc::format!("{p}.{}", s.keep));
        for k in (1..s.keep).rev() {
            crate::rename(&alloc::format!("{p}.{k}"), &alloc::format!("{p}.{}", k + 1));
        }
        crate::rename(&p, &alloc::format!("{p}.1"));
    }
    s.file = File::open(&p, FO_RW);
    s.size = s.file.as_ref().and_then(|f| f.size()).unwrap_or(0);
}

// rt::crash pre-hook: the ring to stderr, the crash to the log file; no allocation, no
// waiting
fn crash_dump(info: &rt::crash::CrashInfo) {
    if let Some(r) = RINGBUF.try_lock() {
        crate::write_stderr(b"--- the last log lines ---\n");
        ring_read(&r, RING, &mut |p| crate::write_stderr(p));
    }
    if let Some(mut s) = FILE.try_lock() {
        let mut t = rt::crash::StackText::<512>::new();
        let _ = write!(t, "--- crash: kind {} code {:#x} ip {:#x}", info.kind, info.code, info.ip);
        if !info.msg.is_empty() {
            let _ = write!(t, ": {}", info.msg);
        }
        let _ = t.write_str("\n");
        let n = t.bytes().len();
        let mut line = [0u8; 512];
        line[..n].copy_from_slice(t.bytes());
        write_file(&mut s, &line[..n]);
    }
}
