// plattest -- self tests of the platform basics (threads, files, ini, cpu)  [apps/plattest/src/main.s]
//
//   plattest --test threads [--repeat N]   raw threads + atomics, semaphores (mutex, timeout,
//                                          post from another thread), the worker pool
//                                          (parallel_for, nested, cancel), background jobs
//                                          (wait, restart, cancel, E_WAKE when finished),
//                                          wake latency and coalescing (headless wait)
//   plattest --test files                  mkdir / delete / rename, the streaming file API
//                                          (also from 64 parallel items), temp files, the
//                                          temp and config dirs, environment overrides
//   plattest --test ini                    parse / query / modify / save / reload, malformed
//                                          and random input, f64 text round trips, INI
//                                          objects on worker threads
//   plattest --test cpu                    cpu_features (--simd sse2 -> 0), brand, popcount,
//                                          the date
//   plattest --test crash                  plat::crash_hook: plat::crash_test writes to address
//                                          0x10; the hook checks the CrashInfo (kind, access,
//                                          address, ip, rax, code range) and prints the summary;
//                                          the process then ends with exit code 3 (not in "all";
//                                          the call chain check follows with R4)
//   plattest --test panic                  (Rust only) a panic reaches the hook as CK_PANIC with
//                                          its message, exit code 3
//   plattest --test pen                    (PN1, t_pen.rs) the pen paths without a pen: Win32
//                                          WM_POINTER* to an own hidden window with a fake
//                                          pointer API (Linux: not ported yet)
//   plattest --test cursor                 (t_cursor.rs) custom cursors: cursor_create from ARGB
//                                          pixels, set on window 0, destroy, slot reuse, size
//                                          limits, cursor_dpi
//   plattest --test spawn                  (t_spawn.rs) spawn_self: a second plattest (--test
//                                          spawned --marker FILE) writes FILE, waited for up to
//                                          10 s (not in "all")
//   plattest --test window                 (Rust only, t_window.rs, Windows) a real-window smoke
//                                          test of the platform layer; the window closes itself
//   plattest --test policy                 (t_policy.rs) the app's permissions: path rules,
//                                          patterns, then a manifest installed in this
//                                          process (files, grants, processes, URLs, plugins,
//                                          print, capabilities, denials; not in "all")
//   plattest --test permask                (t_permask.rs) questions at run time without the UI:
//                                          ask lines, fixed answers, the asking thread, a waiting
//                                          thread and its timeout, stored answers, agent rules
//                                          (not in "all")
//   plattest --test plugperm               (t_plugperm.rs) the rights of native plugins: their
//                                          manifests, calls on their behalf intersected with the
//                                          app's, nested scopes, pool jobs (not in "all")
//   plattest --test vfs                    (t_vfs.rs) one file namespace: routes, mem://, a
//                                          DirFs mount, JFA1 bundles, Overlay, Sub, ZipFs,
//                                          config:// (not in "all")
//   plattest --test instance               (t_instance.rs) one instance per app and user: claim,
//                                          send, a second process (--test instance-client)
//                                          that is forwarded (not in "all")
//   plattest --test reg                    (t_reg.rs, Windows, by hand) plat::inst's registry
//                                          calls on a test key of HKCU, removed again
//   plattest --test tray / tray-bus / tray-win   (t_tray.rs) the notification-area symbol and
//                                          notifications: dry run; Linux against a fake bus
//                                          (tools/platshared/tray_bus.py); Windows by hand
//   plattest --test ibus-bus / ibus-none / ibus-x11   (t_ibus.rs, Linux) the IBus client of X11
//                                          against a fake bus (tools/platshared/ibus_bus.py);
//                                          without one; its X11 glue (synthetic X events)
//   plattest --test log / log-crash        (t_log.rs) the log: levels, ring, file rotation, the
//                                          last lines after a crash (a child)
//   plattest --test life-bus               (t_life.rs, Linux) sleep, settings and SIGTERM against
//                                          fake buses (tools/platshared/life_bus.py)
//   plattest --test a11y / a11y-bus / uia / caret   (t_a11y.rs) the accessibility tree for
//                                          screen readers: headless model and actions; Linux
//                                          against a fake AT-SPI2 bus (tools/platshared/
//                                          atspi_bus.py); Windows through IUIAutomation (by
//                                          hand); caret: the caret for screen magnifiers
//   plattest --test settings / settings-bus   (t_settings.rs) the system settings: the jfgui_*
//                                          overrides; Linux against a fake settings portal
//   plattest --test secret / secret-bus    (t_secret.rs) the system's secret store: the real
//                                          one (Credential Manager + DPAPI, Secret Service; the
//                                          test entries are removed again); Linux against a
//                                          fake Secret Service (tools/platshared/secret_bus.py)
//   plattest --test dirs                   (t_dirs.rs) the user's folders (known folders, XDG user
//                                          dirs in a scratch HOME), log_dir, logs:// and
//                                          documents://, $DOCUMENTS ... in a manifest (not in "all")
//   plattest --test trash                  (t_trash.rs) plat::trash: Linux into a scratch
//                                          XDG_DATA_HOME (and /dev/shm's .Trash-$uid), Windows one
//                                          throwaway file into the recycle bin (not in "all")
//   plattest --test power / power-bus      (t_power.rs) plat::inhibit: Windows real power
//                                          requests (made and released); Linux without a bus,
//                                          and against a fake bus (tools/platshared/desktop_bus.py:
//                                          the Inhibit portal, ScreenSaver / PowerManagement)
//   plattest --test autostart / autostart-bus   (t_autostart.rs) plat::autostart_*: Windows the
//                                          real Run value (removed again), Linux XDG autostart in
//                                          a scratch XDG_CONFIG_HOME; the Background portal
//                                          against a fake bus (not in "all")
//   plattest --test taskbar / taskbar-bus / taskbar-win   (t_taskbar.rs) the taskbar button:
//                                          the dry run and no window / no bus; Linux the
//                                          LauncherEntry signals on a fake bus; Windows by hand
//                                          (a window, every progress state, badge, jump list)
//   plattest --test assoc                  (t_assoc.rs) file types and URL schemes without a
//                                          setup: the parts, the switches, a test package
//                                          registered and unregistered (Windows: test names in
//                                          the real HKCU, removed again; Linux: scratch XDG
//                                          folders), other programs' defaults kept / taken / put
//                                          back (not in "all")
//   plattest --test recent / recent-win    (t_recent.rs) the system's recent files: the xbel
//                                          list through jfgui_RECENT (every OS), Linux's default
//                                          file in a scratch folder, headless, a manifest;
//                                          Windows by hand: the real shell and jump list
//   plattest --test async                  (t_async.rs) plat::task and plat::aio: executors,
//                                          wake-ups from other threads (block_on, the UI
//                                          executor's E_WAKE), the blocking pool, cancellation,
//                                          timers and the virtual clock, Progress, chunked file
//                                          reads / writes / copies cancelled between chunks,
//                                          AtomicFile; children: panics in tasks and jobs (exit
//                                          code 3), aio under a manifest (async-policy)
//   plattest --test dnd                    (t_dnd.rs) drag and drop between programs: effects,
//                                          the target hook, URL lists, the pending drop; Windows:
//                                          our IDataObject / IDropTarget / IDropSource without a
//                                          user, a window registered with OLE
//   plattest --test spell                  (t_spell.rs) the system's spell checker: Windows the
//                                          real ISpellChecker (languages, find, check, suggest
//                                          with sample words), Linux none
//   plattest --test content                (t_content.rs) Android's content:// names (plat::content)
//                                          against a storage provider in memory: names below a
//                                          chosen folder, single documents, AtomicFile in place,
//                                          streams, rename / move, folders, display names, the
//                                          file dialog's desktop answers
//   plattest --test all                    threads, files, ini, cpu
//   --repeat N                             run the selected tests N times (only failures are
//                                          printed then) -- a stress loop for races
//
// Scratch files live in build/plattest/<os>-<id>/ and are removed at the end; the config
// directory is redirected there (APPDATA / XDG_CONFIG_HOME) before anything is written.
// Every check prints PASS / FAIL with the same text as the assembly version; the exit code
// is the number of failures.  Lines starting with two blanks are details (times, paths).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};

use base::{Args, Ini, f64_format, f64_parse};

// the accessibility tree (screen readers)
mod t_a11y;
mod t_async;
// plat-win: the R4 tests of the Windows platform layer
mod t_cursor;
// the user's folders, the log folder and the trash
mod t_dirs;
mod t_trash;
// clipboard_platform: deterministic rich clipboard framing checks.
mod t_cliprich;
// drag and drop between programs: the shared part, Windows' OLE objects
mod t_dnd;
mod t_pen;
mod t_spawn;
mod t_window;
// plat-shared: printing, the crash guard, the plugin loader, the portal
mod t_guard;
mod t_ibus;
mod t_icc;
mod t_instance;
mod t_life;
mod t_lib;
mod t_log;
mod t_policy;
mod t_permask;
mod t_plugperm;
mod t_vfs;
mod t_content;
mod t_portal;
mod t_print;
mod t_reg;
mod t_secret;
mod t_settings;
mod t_tray;
// the desktop integration: keeping awake, autostart, the taskbar button
mod t_autostart;
mod t_power;
mod t_taskbar;
// the system's spell checker (Windows ISpellChecker)
mod t_spell;
// file types without a setup, the system's recent files
mod t_assoc;
mod t_recent;

use plat::{
    CPU_AVX2, CPU_BMI2, CPU_FMA, CPU_POPCNT, CPU_SSE41, Cancelled, E_WAKE, FO_READ, FO_RW, FO_WRITE, File, Job,
    Sem, parallel_for,
};

const BLOCK: usize = 4096; // parallel file test: one block per item
const NBLOCKS: usize = 64;
const NJ_ROUNDS: u64 = 40; // nested-loop job test: jobs, loops per job, items per loop
const NJ_LOOPS: u64 = 200;
const NJ_ITEMS: usize = 24;
const BIG: usize = 0x40000; // 256 KB pattern buffer
const LONG_LINE: usize = 100000; // 'x' bytes in the long ini line
const RND_SIZE: usize = 65536;
const NRANDOM: u32 = 20000; // random doubles for the f64 round trip
const OS: &str = if cfg!(windows) { "win" } else { "linux" };
const CFGVAR: &str = if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" };

ui::main!(plattest());

struct T {
    passes: u32,
    fails: u32,
    quiet: bool,
    base: String,
    simd: Option<String>,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

impl T {
    /// count and print PASS / FAIL (PASS only when not quiet)
    fn check(&mut self, ok: bool, name: &str) {
        if ok {
            self.passes += 1;
            if !self.quiet {
                out(&format!("PASS {name}\n"));
            }
        } else {
            self.fails += 1;
            out(&format!("FAIL {name}\n"));
        }
    }

    fn info(&self, s: &str) {
        if !self.quiet {
            out(s);
        }
    }

    /// string equality (None = missing on either side)
    fn expect_s(&mut self, got: Option<&str>, want: Option<&str>, name: &str) {
        let ok = got == want;
        self.check(ok, name);
        if !ok {
            out(&format!("  got \"{}\", want \"{}\"\n", got.unwrap_or("(null)"), want.unwrap_or("(null)")));
        }
    }

    fn expect_i(&mut self, got: i64, want: i64, name: &str) {
        self.check(got == want, name);
        if got != want {
            out(&format!("  got {got} (0x{got:x}), want {want}\n"));
        }
    }

    fn subp(&self, name: &str) -> String {
        format!("{}/{}", self.base, name)
    }

    /// base = build/plattest/<os>-<id>, created
    fn scratch_init(&mut self) {
        self.base = format!("build/plattest/{OS}-{:x}", plat::time_us() & 0xFF_FFFF);
        let ok = plat::mkdir(&self.base);
        self.check(ok, "scratch: plat_mkdir build/plattest/<os>-<id>");
    }

    /// remove the scratch tree (and build/plattest when it is empty)
    fn scratch_done(&self) {
        rm_tree(&self.base);
        plat::delete("build/plattest");
    }

    /// base/name through the streaming API
    fn write_small(&self, name: &str, data: &[u8]) -> bool {
        match File::open(&self.subp(name), FO_WRITE) {
            Some(f) => f.write_at(0, data) == Some(data.len()),
            None => false,
        }
    }
}

fn plattest() -> i32 {
    let args = Args(plat::args());
    let repeat = args.int("--repeat", 1).max(1);
    let mut t = T { passes: 0, fails: 0, quiet: repeat > 1, base: String::new(), simd: args.value("--simd").map(String::from) };
    let test: fn(&mut T) = match args.value("--test") {
        Some("threads") => t_threads,
        Some("files") => t_files,
        Some("ini") => t_ini,
        Some("cpu") => t_cpu,
        Some("all") => t_all,
        Some("crash") => t_crash,
        Some("panic") => t_panic,
        Some("window") => t_window::t_window, // plat-win
        Some("pen") => t_pen::t_pen,             // plat-win
        Some("cursor") => t_cursor::t_cursor,    // plat-win
        Some("cliprich") => t_cliprich::t_cliprich,
        Some("cliprich-owner") => t_cliprich::t_owner,
        Some("cliprich-reader") => t_cliprich::t_reader,
        Some("dnd") => t_dnd::t_dnd,                   // drag and drop: effects, hook, OLE objects
        Some("dnd-x11") => t_dnd::t_dnd_x11,           // Linux: XDND against tools/platshared/dnd_x11.py
        Some("spawn") => t_spawn::t_spawn,       // plat-win
        Some("spawned") => t_spawn::t_spawned,   // plat-win
        Some("print") => t_print::t_print,             // plat-shared
        Some("printlayout") => t_print::t_printlayout, // plat-shared
        Some("printjob") => t_print::t_printjob,       // plat-shared
        Some("guard") => t_guard::t_guard,             // plat-shared
        Some("guardcrash") => t_guard::t_guardcrash,   // plat-shared
        Some("guardlimit") => t_guard::t_guardlimit,   // plat-shared
        Some("lib") => t_lib::t_lib,                   // plat-shared
        Some("libfuzz") => t_lib::t_libfuzz,           // plat-shared
        Some("portal") => t_portal::t_portal,          // plat-shared
        Some("icc") => t_icc::t_icc,                   // plat-shared
        Some("iccx11") => t_icc::t_iccx11,             // plat-shared
        Some("policy") => t_policy::t_policy,          // permissions (not in "all": it stays installed)
        Some("permask") => t_permask::t_permask,       // questions at run time (not in "all")
        Some("plugperm") => t_plugperm::t_plugperm,    // the rights of plugins (not in "all")
        Some("vfs") => t_vfs::t_vfs,                   // vfs: mounts, bundles, overlays, zip, app folders
        Some("content") => t_content::t_content,       // Android's content:// names against a provider in memory
        Some("dirs") => t_dirs::t_dirs,                // user folders, log folder, their schemes and variables
        Some("trash") => t_trash::t_trash,             // plat::trash (Windows: one file into the recycle bin)
        Some("instance") => t_instance::t_instance,    // single instance: claim, forwarding
        Some("instance-client") => t_instance::t_instance_client, // its second process
        Some("reg") => t_reg::t_reg,                   // registry of the setup (by hand: real HKCU)
        Some("tray") => t_tray::t_tray,                // the tray contract (dry run)
        Some("tray-bus") => t_tray::t_tray_bus,        // Linux: against tools/platshared/tray_bus.py
        Some("tray-win") => t_tray::t_tray_win,        // Windows, by hand: the real symbol
        Some("ibus-bus") => t_ibus::t_ibus_bus,        // Linux: against tools/platshared/ibus_bus.py
        Some("ibus-none") => t_ibus::t_ibus_none,      // Linux: without an input method
        Some("ibus-x11") => t_ibus::t_ibus_x11,        // Linux: the X11 glue (synthetic X events)
        Some("log") => t_log::t_log,                   // the log: levels, ring, file, crash
        Some("log-crash") => t_log::t_log_crash,       // its child: logs and panics
        Some("life-bus") => t_life::t_life_bus,        // Linux: against tools/platshared/life_bus.py
        Some("settings") => t_settings::t_settings,    // the system settings: overrides, defaults
        Some("settings-bus") => t_settings::t_settings_bus, // Linux: against tools/platshared/settings_bus.py
        Some("a11y") => t_a11y::t_a11y,                // the accessibility tree, headless
        Some("a11y-bus") => t_a11y::t_a11y_bus,        // Linux: against tools/platshared/atspi_bus.py
        Some("uia") => t_a11y::t_uia,                  // Windows, by hand: UI Automation
        Some("caret") => t_a11y::t_caret,              // Windows, by hand: the caret for magnifiers
        Some("secret") => t_secret::t_secret,          // the system's secret store (not in "all")
        Some("secret-bus") => t_secret::t_secret_bus,  // Linux: against tools/platshared/secret_bus.py
        Some("power") => t_power::t_power,             // plat::inhibit (Windows: real power requests)
        Some("power-bus") => t_power::t_power_bus,     // Linux: against tools/platshared/desktop_bus.py
        Some("autostart") => t_autostart::t_autostart, // Run value / XDG autostart (removed again)
        Some("autostart-bus") => t_autostart::t_autostart_bus, // Linux: the Background portal (desktop_bus.py)
        Some("taskbar") => t_taskbar::t_taskbar,       // the taskbar button: dry run, no window / bus
        Some("taskbar-bus") => t_taskbar::t_taskbar_bus, // Linux: LauncherEntry (desktop_bus.py)
        Some("taskbar-win") => t_taskbar::t_taskbar_win, // Windows, by hand: a window's button
        Some("spell") => t_spell::t_spell,             // the system's spell checker (Windows: ISpellChecker)
        Some("assoc") => t_assoc::t_assoc,             // file types / schemes without a setup (removed again)
        Some("recent") => t_recent::t_recent,          // the system's recent files (redirected, scratch)
        Some("recent-win") => t_recent::t_recent_win,  // Windows, by hand: the real shell and jump list
        Some("async") => t_async::t_async,             // plat::task / plat::aio: executors, pool, timers, files
        Some("async-panic") => t_async::t_async_panic, // its children: a panic in a task,
        Some("async-panic-job") => t_async::t_async_panic_job, // in a blocking job,
        Some("async-policy") => t_async::t_async_policy, // aio under a manifest
        _ => {
            plat::write_stderr(b"usage: plattest --test threads|files|ini|cpu|pen|cursor|all [--repeat N]\n");
            return 2;
        }
    };
    for _ in 0..repeat {
        test(&mut t);
    }
    out(&format!("plattest: {} passed, {} failure(s)\n", t.passes, t.fails));
    t.fails as i32
}

fn t_all(t: &mut T) {
    t_threads(t);
    t_files(t);
    t_ini(t);
    t_cpu(t);
}

// ---------------------------------------------------------------- helpers

/// delete a directory tree (entries are collected first, then removed)
fn rm_tree(path: &str) {
    let mut list: Vec<(String, bool)> = Vec::new();
    plat::dir_list(path, &mut |name, is_dir, _| list.push((name.to_string(), is_dir)));
    for (name, is_dir) in list {
        let p = format!("{path}/{name}");
        if is_dir {
            rm_tree(&p);
        } else {
            plat::delete(&p);
        }
    }
    plat::delete(path);
}

fn count_entries(path: &str) -> u32 {
    let mut n = 0;
    plat::dir_list(path, &mut |_, _, _| n += 1);
    n
}

/// bytes read from the start of a file (0 on failure)
fn read_small(path: &str, buf: &mut [u8]) -> usize {
    File::open(path, FO_READ).and_then(|f| f.read_at(0, buf)).unwrap_or(0)
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn files_equal(a: &str, b: &str) -> bool {
    match (plat::read_file(a), plat::read_file(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

// write_at / read_at / size of a file that may not have opened (-1 like the assembly)
fn wa(f: &Option<File>, off: u64, data: &[u8]) -> i64 {
    f.as_ref().and_then(|f| f.write_at(off, data)).map_or(-1, |n| n as i64)
}

fn ra(f: &Option<File>, off: u64, buf: &mut [u8]) -> i64 {
    f.as_ref().and_then(|f| f.read_at(off, buf)).map_or(-1, |n| n as i64)
}

fn fsize(f: &Option<File>) -> i64 {
    f.as_ref().and_then(|f| f.size()).map_or(-1, |n| n as i64)
}

/// the 64-bit LCG of the random tests
fn rnd_next(g: &mut u64) -> u64 {
    *g = g.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *g
}

// ---------------------------------------------------------------- events

/// poll once and drop every pending wake-up
fn drain() {
    plat::wait_events(0, &mut |_| {});
}

/// (an E_WAKE arrived before the timeout, elapsed us)
fn wait_for_wake(ms: u64) -> (bool, u64) {
    let t0 = plat::time_us();
    let deadline = t0 + ms * 1000;
    let mut woke = false;
    loop {
        plat::wait_events(deadline, &mut |ev| woke |= ev.typ == E_WAKE);
        if woke || plat::time_us() >= deadline {
            break;
        }
    }
    (woke, plat::time_us() - t0)
}

// ---------------------------------------------------------------- threads

static COUNTER: AtomicU32 = AtomicU32::new(0);
static ALIGN_BAD: AtomicU32 = AtomicU32::new(0);
static ARGSUM: AtomicU32 = AtomicU32::new(0);
static PLAIN: AtomicU32 = AtomicU32::new(0);
static SUM: AtomicU64 = AtomicU64::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static NEST: AtomicU64 = AtomicU64::new(0);
static JCNT: AtomicU64 = AtomicU64::new(0);
static NJ_DONE: AtomicU32 = AtomicU32::new(0);
static WAKE_DONE: AtomicU32 = AtomicU32::new(0);
static T_WAKE: AtomicU64 = AtomicU64::new(0);
static IO_BAD: AtomicU32 = AtomicU32::new(0);
static INI_BAD: AtomicU32 = AtomicU32::new(0);

/// all thread tests, then stop the pool (the next run starts a new one)
fn t_threads(t: &mut T) {
    th_raw(t);
    th_sem(t);
    th_pool(t);
    th_jobs(t);
    th_wake(t);
    plat::thr_shutdown();
}

/// 8 threads running f(index)
fn spawn8(f: fn(u32)) -> (u32, Vec<Option<plat::Thread>>) {
    let mut v = Vec::new();
    let mut n = 0;
    for i in 0..8u32 {
        let h = plat::thread_spawn(0, Box::new(move || f(i)));
        n += h.is_some() as u32;
        v.push(h);
    }
    (n, v)
}

fn join_all(v: Vec<Option<plat::Thread>>) {
    for h in v.into_iter().flatten() {
        h.join();
    }
}

/// thread: 100000 atomic increments
fn w_inc(arg: u32) {
    if !plat::stack_aligned() {
        ALIGN_BAD.fetch_add(1, SeqCst);
    }
    ARGSUM.fetch_add(arg, SeqCst);
    for _ in 0..100000 {
        COUNTER.fetch_add(1, SeqCst);
    }
}

/// thread_spawn / join, atomics, argument and stack alignment
fn th_raw(t: &mut T) {
    COUNTER.store(0, SeqCst);
    ALIGN_BAD.store(0, SeqCst);
    ARGSUM.store(0, SeqCst);
    let (n, hs) = spawn8(w_inc);
    t.check(n == 8, "threads: plat_thread_create x 8");
    join_all(hs);
    t.check(COUNTER.load(SeqCst) == 800000, "threads: 8 x 100000 lock inc = 800000 after join");
    t.check(ARGSUM.load(SeqCst) == 28, "threads: fn(arg) gets its argument in rcx");
    t.check(ALIGN_BAD.load(SeqCst) == 0, "threads: rsp is 16-byte aligned at the call of fn");
}

/// semaphores: mutual exclusion, timeout, counting, post from another thread
fn th_sem(t: &mut T) {
    let sem = Sem::new(1).map(Arc::new);
    t.check(sem.is_some(), "sem: plat_sem_create");
    PLAIN.store(0, SeqCst);
    if let Some(sem) = &sem {
        let mut hs = Vec::new();
        for _ in 0..8 {
            let s = sem.clone();
            hs.push(plat::thread_spawn(0, Box::new(move || {
                for _ in 0..5000 {
                    s.wait(-1);
                    let v = PLAIN.load(core::sync::atomic::Ordering::Relaxed); // a plain read-modify-write
                    core::hint::spin_loop();
                    PLAIN.store(v + 1, core::sync::atomic::Ordering::Relaxed);
                    s.post(1);
                }
            })));
        }
        join_all(hs);
    }
    t.check(PLAIN.load(SeqCst) == 40000, "sem: 8 x 5000 plain increments under a semaphore used as a mutex");
    drop(sem);
    // timeout
    let s = Arc::new(Sem::new(0).expect("semaphore"));
    let t0 = plat::time_us();
    let got = s.wait(50);
    let dt = plat::time_us() - t0;
    t.check(!got && (40000..3000000).contains(&dt), "sem: wait(50 ms) on a zero count times out after about 50 ms");
    t.info(&format!("  (took {dt} us)\n"));
    // counting
    s.post(2);
    let (a, b, c) = (s.wait(0), s.wait(0), s.wait(0));
    t.check(a && b && !c, "sem: post(2) -> two tries succeed, the third one fails");
    // a post from another thread wakes the waiter
    let s2 = s.clone();
    let h = plat::thread_spawn(0, Box::new(move || {
        plat::sleep_ms(20);
        s2.post(1);
    }));
    let t0 = plat::time_us();
    let got = s.wait(5000);
    let dt = plat::time_us() - t0;
    if let Some(h) = h {
        h.join();
    }
    t.check(got && dt < 3000000, "sem: a post from another thread wakes a sleeping wait");
}

/// thr_init, parallel_for (results, 0 / 1 items, nesting, cancel, bursts)
fn th_pool(t: &mut T) {
    let n = plat::thr_init(0);
    t.check(n != 0, "pool: thr_init starts worker threads");
    t.info(&format!("  ({n} workers, plat_cpu_count {})\n", plat::cpu_count()));
    t.check(plat::thr_count() == n, "pool: thr_count = thr_init result");
    // 10000 items
    let outv: Vec<AtomicU64> = (0..10000).map(|_| AtomicU64::new(0)).collect();
    SUM.store(0, SeqCst);
    let r = parallel_for(10000, &|i, _| {
        outv[i].store((i * i + 7) as u64, SeqCst);
        SUM.fetch_add(i as u64, SeqCst);
    });
    t.check(r.is_ok(), "pool: parallel_for(10000) returns 0");
    t.check(SUM.load(SeqCst) == 49995000, "pool: every index exactly once (sum of the indexes)");
    let bad = (0..10000).filter(|&i| outv[i].load(SeqCst) != (i * i + 7) as u64).count();
    t.check(bad == 0, "pool: out[i] = i*i + 7 for all 10000 items");
    // 0 and 1 items
    CALLS.store(0, SeqCst);
    let count = |_: usize, _: &plat::JobCtl| {
        CALLS.fetch_add(1, SeqCst);
    };
    let r = parallel_for(0, &count);
    t.check(r.is_ok() && CALLS.load(SeqCst) == 0, "pool: parallel_for(0) calls nothing and returns 0");
    let _ = parallel_for(1, &count);
    t.check(CALLS.load(SeqCst) == 1, "pool: parallel_for(1) calls fn once");
    // nested loops
    NEST.store(0, SeqCst);
    let _ = parallel_for(8, &|_, _| {
        let _ = parallel_for(100, &|_, _| {
            NEST.fetch_add(1, SeqCst);
        });
    });
    t.check(NEST.load(SeqCst) == 800, "pool: nested parallel_for (8 x 100 items)");
    // cancel from inside fn; the items after 1000 wait (at most 2 s) until the cancel is
    // visible: the pool runs one thread more than there are cores, and a thread descheduled
    // with item 500 must not let the others finish every item first (a broken cancel still
    // runs all 100000)
    CALLS.store(0, SeqCst);
    let until = plat::time_us() + 2_000_000;
    let r = parallel_for(100000, &|i, job| {
        CALLS.fetch_add(1, SeqCst);
        if i == 500 {
            job.cancel();
        } else if i > 1000 {
            while !job.cancelled() && plat::time_us() < until {
                core::hint::spin_loop();
            }
        }
    });
    let c = CALLS.load(SeqCst);
    t.check(r == Err(Cancelled) && (501..100000).contains(&c),
            "pool: thr_job_cancel inside fn -> ERR_CANCEL, the rest is skipped");
    t.info(&format!("  ({c} of 100000 items ran)\n"));
    // bursts of short loops
    CALLS.store(0, SeqCst);
    for _ in 0..200 {
        let _ = parallel_for(50, &count);
    }
    t.check(CALLS.load(SeqCst) == 10000, "pool: 200 short loops back to back");
}

/// background jobs: start / wait / restart / cancel / empty / E_WAKE / nested loops
fn th_jobs(t: &mut T) {
    let mut job = Job::new();
    job.set(64, |_, _| {
        plat::sleep_ms(2);
        JCNT.fetch_add(1, SeqCst);
    });
    JCNT.store(0, SeqCst);
    let d0 = job.done();
    let s1 = job.start();
    let s2 = job.start();
    t.check(d0 && s1 && !s2, "jobs: thr_job_start (a second start while running is refused)");
    let r = job.wait();
    t.check(r.is_ok() && JCNT.load(SeqCst) == 64 && job.items_done() == 64, "jobs: thr_job_wait -> all 64 items done");
    t.check(job.done(), "jobs: thr_job_done = 1 afterwards");
    // the same record again
    job.start();
    let r = job.wait();
    t.check(r.is_ok() && JCNT.load(SeqCst) == 128, "jobs: a finished record can be started again");
    // cancel a long job
    job.set_count(100000);
    JCNT.store(0, SeqCst);
    job.start();
    plat::sleep_ms(30);
    job.cancel();
    let r = job.wait();
    let c = JCNT.load(SeqCst);
    t.check(r == Err(Cancelled) && c < 100000 && job.done(),
            "jobs: thr_job_cancel -> ERR_CANCEL, unstarted items skipped, done");
    t.info(&format!("  ({c} of 100000 items ran before the cancel)\n"));
    // an empty job is finished at once
    job.set_count(0);
    let s = job.start();
    t.check(s && job.done(), "jobs: an empty job is done at once");
    // a finished background job wakes the wait
    drain();
    job.set(16, |_, _| {
        JCNT.fetch_add(1, SeqCst);
    });
    job.start();
    let (woke, us) = wait_for_wake(3000);
    let _ = job.wait();
    t.check(woke, "jobs: a finished background job ends plat_wait with E_WAKE");
    t.info(&format!("  (E_WAKE after {us} us)\n"));
    // a background job whose item runs nested loops while the owner waits for it: every loop
    // end must wake its owner (Linux: a waiter arriving later must not take the wake-up meant
    // for a sleeping one)
    NJ_DONE.store(0, SeqCst);
    NEST.store(0, SeqCst);
    let watch = plat::thread_spawn(0, Box::new(|| {
        for _ in 0..1000 {
            if NJ_DONE.load(SeqCst) != 0 {
                return;
            }
            plat::sleep_ms(10);
        }
        out("FAIL jobs: nested loops in a job hang (lost wake-up)\n");
        plat::exit(1);
    }));
    job.set(1, |_, _| {
        for _ in 0..NJ_LOOPS {
            let _ = parallel_for(NJ_ITEMS, &|_, _| {
                NEST.fetch_add(1, SeqCst);
            });
        }
    });
    for _ in 0..NJ_ROUNDS {
        job.start();
        let _ = job.wait();
    }
    NJ_DONE.store(1, SeqCst);
    if let Some(w) = watch {
        w.join();
    }
    t.check(NEST.load(SeqCst) == NJ_ROUNDS * NJ_LOOPS * NJ_ITEMS as u64,
            "jobs: nested loops inside a background job while the owner waits");
}

/// wake from another thread ends wait(forever); bursts coalesce
fn th_wake(t: &mut T) {
    if cfg!(windows) {
        t.info("  (plat_wait: window 0 -- 0 none = wake event, 1 = WM_APP_WAKE)\n");
    } else {
        t.info("  (plat_wait: display backend 0 -- 0 none, 1 Wayland, 2 X11)\n");
    }
    drain();
    WAKE_DONE.store(0, SeqCst);
    T_WAKE.store(0, SeqCst);
    let watch = plat::thread_spawn(0, Box::new(|| {
        for _ in 0..500 {
            if WAKE_DONE.load(SeqCst) != 0 {
                return;
            }
            plat::sleep_ms(10);
        }
        out("FAIL wake: plat_wait(-1) not woken within 5 s\n");
        plat::exit(1);
    }));
    let waker = plat::thread_spawn(0, Box::new(|| {
        plat::sleep_ms(30);
        T_WAKE.store(plat::time_us(), SeqCst);
        plat::wake();
    }));
    let t0 = plat::time_us();
    loop {
        let mut woke = false;
        plat::wait_events(u64::MAX, &mut |ev| woke |= ev.typ == E_WAKE);
        if woke {
            break;
        }
    }
    let t1 = plat::time_us();
    WAKE_DONE.store(1, SeqCst);
    for h in [waker, watch].into_iter().flatten() {
        h.join();
    }
    let lat = t1 - T_WAKE.load(SeqCst);
    let blocked = t1 - t0;
    let ok = lat < 50000 && blocked >= 20000;
    t.check(ok, "wake: plat_wake from another thread ends plat_wait(-1) within 50 ms");
    let detail = format!("  (E_WAKE {lat} us after plat_wake; blocked {blocked} us)\n");
    if ok {
        t.info(&detail);
    } else {
        out(&detail); // always shown after a failure
    }
    // 100 calls -> one event
    drain();
    for _ in 0..100 {
        plat::wake();
    }
    let deadline = plat::time_us() + 100000;
    let mut n = 0;
    loop {
        plat::wait_events(deadline, &mut |ev| n += (ev.typ == E_WAKE) as u32);
        if plat::time_us() >= deadline {
            break;
        }
    }
    t.check(n == 1, "wake: 100 plat_wake calls in a row give one E_WAKE");
    t.info(&format!("  ({n} E_WAKE)\n"));
}

// ---------------------------------------------------------------- files

/// block i of the parallel file test
fn block_byte(i: usize, c: usize) -> u8 {
    (i * 31 + c * 7) as u8
}

/// directories, names, the streaming file API, temp files, system dirs
fn t_files(t: &mut T) {
    t.scratch_init();
    // ---- mkdir
    let p = t.subp("a/b/c");
    let r1 = plat::mkdir(&p);
    let e = plat::file_exists(&p);
    let r2 = plat::mkdir(&p);
    t.check(r1 && e == 2 && r2, "files: plat_mkdir creates the parents; again = ok");
    t.write_small("f.txt", b"x");
    t.check(!plat::mkdir(&t.subp("f.txt")), "files: plat_mkdir over an existing file fails");
    // ---- buffers
    let pat: Vec<u8> = (0..BIG).map(|i| (i as u32 * 7 + (i as u32 >> 9)) as u8).collect();
    let mut rb = vec![0u8; BIG];
    // ---- FO_WRITE, write_at / read_at / size
    let f = File::open(&t.subp("data.bin"), FO_WRITE);
    t.check(f.is_some(), "files: plat_file_open FO_WRITE");
    t.expect_i(wa(&f, 0, &pat), BIG as i64, "files: write_at 256 KB");
    t.expect_i(wa(&f, 0x100000, &pat[..100]), 100, "files: write_at at 1 MB (past the end)");
    t.expect_i(fsize(&f), 0x100064, "files: plat_file_size = 1 MB + 100");
    let n = ra(&f, 1000, &mut rb[..5000]);
    t.check(n == 5000 && rb[..5000] == pat[1000..6000], "files: read_at returns what write_at wrote");
    t.expect_i(ra(&f, 0x100000 + 50, &mut rb[..100]), 50, "files: read_at across the end is short");
    t.expect_i(ra(&f, 0x200000, &mut rb[..10]), 0, "files: read_at past the end = 0");
    rb[..16].fill(0xFF);
    let n = ra(&f, 0x80000, &mut rb[..16]);
    t.check(n == 16 && rb[..16].iter().all(|&b| b == 0), "files: the gap reads as zeros");
    drop(f);
    // ---- FO_READ
    let f = File::open(&t.subp("data.bin"), FO_READ);
    let sz = fsize(&f);
    let w = wa(&f, 0, &pat[..4]);
    let ok = f.is_some() && sz == 0x100064 && w == -1;
    drop(f);
    t.check(ok, "files: FO_READ: size kept, write_at fails");
    t.check(File::open(&t.subp("missing.bin"), FO_READ).is_none(), "files: FO_READ of a missing file = -1");
    // ---- FO_RW
    let f = File::open(&t.subp("data.bin"), FO_RW);
    let sz = fsize(&f);
    wa(&f, 10, b"XYZ");
    rb[..4].fill(0);
    let n = ra(&f, 9, &mut rb[..5]);
    drop(f);
    t.check(sz == 0x100064 && n == 5 && &rb[1..4] == b"XYZ" && rb[0] == pat[9] && rb[4] == pat[13],
            "files: FO_RW keeps the contents and writes in place");
    // ---- 64 parallel items on one handle
    let fh = File::open(&t.subp("par.bin"), FO_WRITE);
    IO_BAD.store(0, SeqCst);
    let _ = parallel_for(NBLOCKS, &|i, _| {
        let blk: Vec<u8> = (0..BLOCK).map(|c| block_byte(i, c)).collect();
        if wa(&fh, (i * BLOCK) as u64, &blk) != BLOCK as i64 {
            IO_BAD.fetch_add(1, SeqCst);
        }
    });
    let _ = parallel_for(NBLOCKS, &|i, _| {
        let mut b = vec![0u8; BLOCK];
        if ra(&fh, (i * BLOCK) as u64, &mut b) != BLOCK as i64 || (0..BLOCK).any(|c| b[c] != block_byte(i, c)) {
            IO_BAD.fetch_add(1, SeqCst);
        }
    });
    let sz = fsize(&fh);
    drop(fh);
    t.check(IO_BAD.load(SeqCst) == 0 && sz == (NBLOCKS * BLOCK) as i64, "files: 64 parallel write_at + read_at on one handle");
    // ---- rename over an existing file
    t.write_small("x.txt", b"AAA");
    t.write_small("y.txt", b"BBBB");
    let (px, py) = (t.subp("x.txt"), t.subp("y.txt"));
    let r = plat::rename(&px, &py);
    let e = plat::file_exists(&px);
    rb[..4].fill(0);
    let n = read_small(&py, &mut rb[..16]);
    t.check(r && e == 0 && n == 3 && &rb[..4] == b"AAA\0", "files: plat_rename replaces an existing target");
    // ---- delete
    let d1 = plat::delete(&py);
    let e = plat::file_exists(&py);
    let d2 = plat::delete(&py);
    t.check(d1 && e == 0 && !d2, "files: plat_delete; deleting again fails");
    let d1 = plat::delete(&t.subp("a/b/c"));
    let d2 = plat::delete(&t.subp("a"));
    t.check(d1 && !d2, "files: plat_delete removes an empty directory, not a full one");
    // ---- temp files
    let tmp = t.subp("tmp");
    plat::mkdir(&tmp);
    let f = File::open_temp(Some(&tmp));
    let w = wa(&f, 0, &pat[..4096]);
    let n = ra(&f, 0, &mut rb[..4096]);
    let ok = n == 4096 && w == 4096 && rb[..4096] == pat[..4096];
    let during = count_entries(&tmp);
    let opened = f.is_some();
    drop(f);
    let after = count_entries(&tmp);
    t.check(opened && ok && after == 0, "files: plat_file_open_temp: usable, nothing left after close");
    t.info(&format!("  (directory entries while it was open: {during})\n"));
    let f = File::open_temp(None);
    let w = wa(&f, 100, &pat[..10]);
    t.check(f.is_some() && w == 10, "files: plat_file_open_temp(0) in the system temp directory");
    drop(f);
    // ---- system directories
    let td = plat::temp_dir();
    t.check(td.as_deref().is_some_and(|d| plat::file_exists(d) == 2), "files: plat_temp_dir is an existing directory");
    t.info(&format!("  temp dir: {}\n", td.unwrap_or_default()));
    let cd = plat::config_dir();
    t.check(cd.as_deref().is_some_and(|c| !c.is_empty()), "files: plat_config_dir");
    t.info(&format!("  config dir: {}\n", cd.unwrap_or_default()));
    t_env(t);
    // ---- memory, CPUs
    let mt = plat::mem_total();
    t.check(mt > 0x400_0000, "sys: plat_mem_total > 64 MB");
    t.info(&format!("  ({} MB, {} CPUs)\n", mt >> 20, plat::cpu_count()));
    t.scratch_done();
}

/// env(name) with the assembly's buffer: the length, or -1 when it is not set or does not
/// fit into cap bytes (with the NUL)
fn env_cap(name: &str, cap: usize) -> i64 {
    match plat::env(name) {
        Some(v) if v.len() < cap => v.len() as i64,
        _ => -1,
    }
}

/// env / setenv and the config dir override (never the real config dir)
fn t_env(t: &mut T) {
    let orig = plat::env(CFGVAR);
    // expected = cwd/base/cfg with '/'; the value set in the native form
    let raw = format!("{}/{}/cfg", plat::cwd(), t.base);
    let expected = raw.replace('\\', "/");
    let native = if cfg!(windows) { raw.replace('/', "\\") } else { raw };
    let s = plat::setenv(CFGVAR, Some(&native));
    let cd = plat::config_dir().unwrap_or_default();
    t.check(s && cd == expected, "env: plat_config_dir follows APPDATA / XDG_CONFIG_HOME");
    t.info(&format!("  ({cd})\n"));
    // a trailing slash is dropped
    plat::setenv(CFGVAR, Some(&format!("{native}/")));
    let cd = plat::config_dir().unwrap_or_default();
    t.check(cd == expected, "env: plat_config_dir drops a trailing slash");
    // env
    let v = plat::env(CFGVAR).unwrap_or_default();
    t.check(!v.is_empty() && v.starts_with(&native), "env: plat_env reads what plat_setenv set");
    t.check(env_cap(CFGVAR, 4) == -1, "env: plat_env with a too small buffer = -1");
    t.check(env_cap("jfgui_PLATTEST_UNSET", 64) == -1, "env: plat_env of an unset variable = -1");
    if cfg!(not(windows)) {
        plat::setenv(CFGVAR, Some("relative/dir"));
        let cd = plat::config_dir().unwrap_or_default();
        t.check(cd.ends_with("/.config"), "env: a relative XDG_CONFIG_HOME is ignored");
    }
    // without the variable: the fallback
    plat::setenv(CFGVAR, None);
    let cd = plat::config_dir();
    let tail = if cfg!(windows) { "/AppData/Roaming" } else { "/.config" };
    t.check(cd.as_deref().is_some_and(|c| c.ends_with(tail)), "env: plat_config_dir fallback without the variable");
    t.info(&format!("  ({})\n", cd.unwrap_or_default()));
    // settings through the redirected config dir
    plat::setenv(CFGVAR, Some(&native));
    let path = plat::config_dir().unwrap_or_default() + "/jfgui-plattest/settings.ini";
    let mut ini = Ini::new();
    ini.set_int("Window", "width", 1234);
    let ok = ini.save(&path) && Ini::load(&path).get_int("Window", "width", 0) == 1234;
    t.check(ok, "env: settings saved to and read from the (redirected) config dir");
    // restore
    plat::setenv(CFGVAR, orig.as_deref());
}

// ---------------------------------------------------------------- ini

// the test input (a 100000-byte "long=" line and "after=1" follow at run time)
const INI_SRC: &[u8] = b"\xEF\xBB\xBF; comment line one\r\nglobal = g1\r\n\r\n[Main]\r\n  name =   value with spaces  \r\n  \
name2  =  keep   me  \ngarbage line\n=novalue\nsemi=a;b#c\nempty=\ndup=1\ndup=2\n# hash comment\n[unterminated\n\
i1=42\ni2=-7\ni3=0x1F\ni4=12abc\ni5=true\ni6=Off\n\
i7=9223372036854775807\ni8=9223372036854775808\ni9=-9223372036854775808\n\
f1=1.5\nf2=-0.25\nf3=1e3\nf4=.5\nf5=5.\nf6=1,5\nf7=abc\nf9=1e-7\nf10=0.1\n\
nul=a\0b\n[ sec two ]  ; trailing comment\nk = v2\n[Gr\xC3\xB6\xC3\x9Fe]\n\xC3\x84pfel=Birnen \xE2\x82\xAC\n\t\t\n[Main]\nlate=yes\nlong=";

// ini_get checks after loading: sec, key, want (None = missing), name
const INI_EXPECT: [(&str, &str, Option<&str>, &str); 18] = [
    ("", "global", Some("g1"), "ini: global key"),
    ("", "global", Some("g1"), "ini: section \"\" = the global part"),
    ("Main", "name", Some("value with spaces"), "ini: blanks around key and value are trimmed"),
    ("main", "NAME", Some("value with spaces"), "ini: section and key names ignore case"),
    ("Main", "name2", Some("keep   me"), "ini: inner blanks of a value are kept"),
    ("Main", "semi", Some("a;b#c"), "ini: ';' and '#' inside a value"),
    ("Main", "empty", Some(""), "ini: empty value"),
    ("Main", "dup", Some("1"), "ini: a repeated key -> the first one"),
    ("Main", "nul", None, "ini: a line with a NUL byte is not interpreted"),
    ("Main", "long", None, "ini: an overlong line is not interpreted"),
    ("Main", "after", Some("1"), "ini: the line after the overlong line"),
    ("Main", "late", Some("yes"), "ini: a repeated section header continues the section"),
    ("sec two", "k", Some("v2"), "ini: [ name ] with blanks and a trailing comment"),
    ("Größe", "Äpfel", Some("Birnen €"), "ini: UTF-8 section, key and value"),
    ("Main", "missing", None, "ini: missing key -> 0"),
    ("Nope", "name", None, "ini: missing section -> 0"),
    ("", "name", None, "ini: section keys are not global"),
    ("Main", "novalue", None, "ini: '=value' without a key is not interpreted"),
];

// after the changes, saved and loaded again
const INI_EXPECT2: [(&str, &str, &str, &str); 13] = [
    ("Main", "name", "new value", "ini2: changed value"),
    ("Main", "added", "x", "ini2: added key"),
    ("New Section", "k", "v", "ini2: new section"),
    ("", "g2", "gv", "ini2: new global key"),
    ("Main", "multi", "line1 line2", "ini2: CR / LF in a value became blanks"),
    ("Main", "dup", "2", "ini2: ini_remove removed the first 'dup'"),
    ("", "global", "g1", "ini2: global key"),
    ("Main", "name2", "keep   me", "ini2: untouched value"),
    ("sec two", "k", "v2", "ini2: [ sec two ]"),
    ("Größe", "Äpfel", "Birnen €", "ini2: UTF-8"),
    ("Main", "after", "1", "ini2: the line after the overlong line"),
    ("Main", "late", "yes", "ini2: second [Main] block"),
    ("Main", "semi", "a;b#c", "ini2: ';' and '#' inside a value"),
];

// ini_get_int in [Main]: key, default, want, name
const INI_INTS: [(&str, i64, i64, &str); 12] = [
    ("i1", 99, 42, "ini: int 42"),
    ("i2", 99, -7, "ini: int -7"),
    ("i3", 99, 31, "ini: int 0x1F"),
    ("i4", 99, 99, "ini: int 12abc -> default"),
    ("i5", 99, 1, "ini: int true -> 1"),
    ("i6", 99, 0, "ini: int Off -> 0"),
    ("i7", 99, i64::MAX, "ini: int max"),
    ("i8", 99, 99, "ini: int max + 1 -> default"),
    ("i9", 99, i64::MIN, "ini: int min"),
    ("late", 99, 1, "ini: int yes -> 1"),
    ("missing", 99, 99, "ini: int missing -> default"),
    ("f1", 99, 99, "ini: int 1.5 -> default"),
];

// ini_get_f64 in [Main]: key, default bits, want bits, name
const INI_F64S: [(&str, u64, u64, &str); 11] = [
    ("f1", 0xBFF0000000000000, 0x3FF8000000000000, "ini: f64 1.5"),
    ("f2", 0xBFF0000000000000, 0xBFD0000000000000, "ini: f64 -0.25"),
    ("f3", 0xBFF0000000000000, 0x408F400000000000, "ini: f64 1e3"),
    ("f4", 0xBFF0000000000000, 0x3FE0000000000000, "ini: f64 .5"),
    ("f5", 0xBFF0000000000000, 0x4014000000000000, "ini: f64 5."),
    ("f6", 0xBFF0000000000000, 0xBFF0000000000000, "ini: f64 1,5 -> default (no ',' decimal separator)"),
    ("f7", 0xBFF0000000000000, 0xBFF0000000000000, "ini: f64 abc -> default"),
    ("f9", 0xBFF0000000000000, 0x3E7AD7F29ABCAF48, "ini: f64 1e-7"),
    ("f10", 0xBFF0000000000000, 0x3FB999999999999A, "ini: f64 0.1 exact"),
    ("i1", 0xBFF0000000000000, 0x4045000000000000, "ini: f64 of an integer value (42)"),
    ("missing", 0xBFF0000000000000, 0xBFF0000000000000, "ini: f64 missing -> default"),
];

// the saved text: needle, check name (None = must not be there)
const INI_KEPT: [(&str, Option<&str>); 11] = [
    ("; comment line one", Some("ini: kept verbatim: ; comment")),
    ("garbage line", Some("ini: kept verbatim: malformed line")),
    ("=novalue", Some("ini: kept verbatim: =novalue")),
    ("# hash comment", Some("ini: kept verbatim: # comment")),
    ("[unterminated", Some("ini: kept verbatim: [unterminated")),
    ("  name2  =  keep   me  ", Some("ini: kept verbatim: spacing of an untouched line")),
    ("[ sec two ]  ; trailing comment", Some("ini: kept verbatim: section header with comment")),
    ("name=new value", Some("ini: changed line written as key=value")),
    ("[New Section]", Some("ini: new section header")),
    ("  name =   value with spaces  ", None),
    ("global = g1", Some("ini: kept verbatim: global = g1")),
];
const S_GONE: &str = "ini: a changed line is written anew (old text gone)";

// f64_format cases: bits, expected text (all of them are also saved by set_f64)
const F64_CASES: [(u64, &str); 40] = [
    (0x3FB999999999999A, "0.1"),
    (0x3FD3333333333334, "0.30000000000000004"),
    (0x3FD5555555555555, "0.3333333333333333"),
    (0x3FE5555555555555, "0.6666666666666666"),
    (0x4059000000000000, "100"),
    (0x3E7AD7F29ABCAF48, "1e-07"),
    (0x4376345785D8A000, "1e17"),
    (0xC004000000000000, "-2.5"),
    (0x0000000000000001, "5e-324"),
    (0x7FEFFFFFFFFFFFFF, "1.7976931348623157e308"),
    (0x7FF0000000000000, "inf"),
    (0xFFF0000000000000, "-inf"),
    (0x7FF8000000000000, "nan"),
    (0x0000000000000000, "0"),
    (0x8000000000000000, "-0"),
    (0x400921FB54442D18, "3.141592653589793"),
    (0x4005BF0A8B145769, "2.718281828459045"),
    (0x0010000000000000, "2.2250738585072014e-308"),
    (0x444B1AE4D6E2EF50, "1e21"),
    (0x4480F0CF064DD592, "1e22"),
    (0x44B52D02C7E14AF6, "1e23"),
    (0x419D6F3454800000, "123456789.125"),
    (0x3FB1EB851EB851EC, "0.07"),
    (0x3EE4F8B588E368F1, "0.00001"),
    (0x3EB92A737110E454, "1.5e-06"),
    (0x4340000000000000, "9007199254740992"),
    (0x4341C37937E08000, "10000000000000000"),
    (0x40C81CD6C8B43958, "12345.678"),
    (0x3FE0000000000000, "0.5"),
    (0x3FF0000000000000, "1"),
    (0x43B12210F47DE981, "1.2345678901234568e18"),
    (0x4011666666666666, "4.35"),
    (0x3FE9999999999999, "0.7999999999999999"),
    (0x01A56E1FC2F8F359, "1e-300"),
    (0x54CB6E83B85F253B, "3e100"),
    (0x41B1DE784A000000, "299792458"),
    (0x44DFE185CA57C517, "6.02214076e23"),
    (0xBDDB7CDFD9D7BDBB, "-1e-10"),
    (0x40EFFFE000000000, "65535"),
    (0x3F50624DD2F1A9FC, "0.001"),
];

// f64_parse cases: text, want bits, characters used
const PARSE_CASES: [(&str, u64, usize); 13] = [
    ("  -1.5e+3", 0xC097700000000000, 9),
    ("1e", 0x3FF0000000000000, 1),
    ("abc", 0, 0),
    ("-.5", 0xBFE0000000000000, 3),
    ("INF", 0x7FF0000000000000, 3),
    ("Infinity", 0x7FF0000000000000, 8),
    ("1e99999", 0x7FF0000000000000, 7),
    ("1e-99999", 0, 8),
    ("00012.5000", 0x4029000000000000, 10),
    ("123456789012345678901234567890", 0x45F8EE90FF6C373E, 30),
    ("1.0000000000000000000", 0x3FF0000000000000, 21),
    ("-0", 0x8000000000000000, 2),
    ("0.1xyz", 0x3FB999999999999A, 3),
];

// random ini bytes are drawn from these
const RND_CHARSET: &[u8; 26] = b"[]=;# \t\r\n\0abcXYZ019.,\xC3\xA9\xE2\x82\xAC";

/// the lookups the input file must give
fn ini_checks(t: &mut T, ini: &Ini) {
    for (sec, key, want, name) in INI_EXPECT {
        t.expect_s(ini.get(sec, key), want, name);
    }
    for (key, def, want, name) in INI_INTS {
        t.expect_i(ini.get_int("Main", key, def), want, name);
    }
    for (key, def, want, name) in INI_F64S {
        t.expect_i(ini.get_f64("Main", key, f64::from_bits(def)).to_bits() as i64, want as i64, name);
    }
}

/// the settings module
fn t_ini(t: &mut T) {
    t.scratch_init();
    // ---- the input file: INI_SRC + LONG_LINE x + "\nafter=1"
    let mut src = INI_SRC.to_vec();
    src.resize(src.len() + LONG_LINE, b'x');
    src.extend_from_slice(b"\nafter=1");
    let path = t.subp("in.ini");
    t.check(plat::write_file(&path, &src), "ini: test input written");
    let mut ini = Ini::load(&path);
    t.check(true, "ini: ini_load");
    ini_checks(t, &ini);
    // ---- changes
    let ok = ini.set("Main", "name", "new value")
        & ini.set("Main", "added", "x")
        & ini.set("New Section", "k", "v")
        & ini.set("", "g2", "gv")
        & ini.set_int("Main", "int", -12345)
        & ini.set("Main", "multi", "line1\nline2")
        & ini.remove("Main", "dup");
    t.check(ok, "ini: ini_set / ini_set_int / ini_remove succeed");
    let any = ini.remove("Main", "nothere")
        | ini.set("Main", "a=b", "x")
        | ini.set("x]y", "k", "v")
        | ini.set("Main", " lead", "v")
        | ini.set("Main", ";c", "v");
    t.check(!any, "ini: bad names and missing keys are refused (-1)");
    // doubles
    for (i, (bits, _)) in F64_CASES.iter().enumerate() {
        ini.set_f64("F64", &format!("v{i}"), f64::from_bits(*bits));
    }
    // ---- save into a new directory, reload
    let path = t.subp("sub/dir/out.ini");
    t.check(ini.save(&path), "ini: ini_save creates the directory and writes the file");
    drop(ini);
    let ini = Ini::load(&path);
    for (sec, key, want, name) in INI_EXPECT2 {
        t.expect_s(ini.get(sec, key), Some(want), name);
    }
    t.expect_i(ini.get_int("Main", "int", 0), -12345, "ini: reloaded int -12345");
    // doubles bit-exact
    let mut bad = 0;
    for (i, (want, _)) in F64_CASES.iter().enumerate() {
        let got = ini.get_f64("F64", &format!("v{i}"), f64::from_bits(0x7FF0DEAD00000000)).to_bits();
        if got == *want {
            continue;
        }
        // NaN only needs to read back as some NaN
        if *want == 0x7FF8000000000000 && got & !(1 << 63) > 0x7FF0000000000000 && got != 0x7FF0DEAD00000000 {
            continue;
        }
        bad += 1;
        out(&format!("  v{i}: got 0x{got:x}, want 0x{want:x}\n"));
    }
    t.check(bad == 0, "ini: doubles read back bit-exact");
    // ---- the saved text
    let text = plat::read_file(&path);
    t.check(text.is_some(), "ini: the saved file can be read");
    if let Some(text) = &text {
        for (needle, name) in INI_KEPT {
            let found = find_bytes(text, needle.as_bytes()).is_some();
            match name {
                Some(n) => t.check(found, n),
                None => t.check(!found, S_GONE),
            }
        }
        t.check(find_bytes(text, b"nul=a\0b").is_some(), "ini: a line with a NUL byte is kept verbatim");
        let long = &src[INI_SRC.len() - 5..INI_SRC.len() + LONG_LINE]; // "long=" + the x run
        t.check(find_bytes(text, long).is_some(), "ini: a 100000-byte line is kept verbatim");
        // the new global key sits before the first section
        let g2 = find_bytes(text, b"g2=gv");
        let main = find_bytes(text, b"[Main]");
        t.check(matches!((g2, main), (Some(a), Some(b)) if a < b), "ini: a new global key goes before the first section");
    }
    // ---- load + save reproduces the file
    let path2 = t.subp("out2.ini");
    ini.save(&path2);
    t.check(files_equal(&path, &path2), "ini: load + save gives the same bytes");
    drop(ini);
    // ---- a missing file
    let p = t.subp("nope.ini");
    let mut ini = Ini::load(&p);
    let ok = ini.get("", "x").is_none() && {
        ini.set("S", "k", "v");
        ini.save(&p)
    };
    t.check(ok, "ini: a missing file loads empty and can be saved");
    // ---- an empty file
    t.write_small("empty.ini", b"");
    let ini = Ini::load(&t.subp("empty.ini"));
    t.check(ini.get("S", "k").is_none(), "ini: an empty file");
    ini_self(t);
    ini_random(t);
    t_f64(t);
    // ---- INI objects on worker threads
    INI_BAD.store(0, SeqCst);
    let _ = parallel_for(32, &|i, _| pf_ini(i));
    t.check(INI_BAD.load(SeqCst) == 0, "ini: 32 parallel items with their own INI objects and f64 text");
    t.scratch_done();
}

/// set with a section name and a value that get returned from the same object (in Rust
/// they are copied first: the borrow checker sees to that)
fn ini_self(t: &mut T) {
    let longval: String = "0123456789".repeat(30);
    let mut ini = Ini::new();
    let mut ok = ini.set("S", "src", &longval) & ini.set("S", "secname", "Moved Section");
    for i in 0..100 {
        let v = ini.get("S", "src").unwrap_or("").to_string();
        let sec = ini.get("S", "secname").unwrap_or("").to_string();
        ok &= ini.set(&sec, &format!("c{i}"), &v);
    }
    for i in 0..100 {
        ok &= ini.get("Moved Section", &format!("c{i}")) == Some(longval.as_str());
    }
    t.check(ok, "ini: ini_set with strings ini_get returned (while the pool grows)");
}

/// 20 files of random bytes: load / save / load / save must be stable
fn ini_random(t: &mut T) {
    let mut g = 12345u64;
    let mut bad = 0;
    let mut data = vec![0u8; RND_SIZE];
    for _ in 0..20 {
        for b in data.iter_mut() {
            *b = RND_CHARSET[((rnd_next(&mut g) >> 33) % 26) as usize];
        }
        let p = t.subp("rnd.ini");
        plat::write_file(&p, &data);
        let ini = Ini::load(&p);
        let _ = ini.get("a", "b"); // lookups must not crash either
        let _ = ini.get_int("a", "b", 7);
        let (p1, p2) = (t.subp("rnd1.ini"), t.subp("rnd2.ini"));
        if !ini.save(&p1) {
            bad += 1;
            continue;
        }
        drop(ini);
        Ini::load(&p1).save(&p2);
        if !files_equal(&p1, &p2) {
            bad += 1;
        }
    }
    t.check(bad == 0, "ini: 20 random 64 KB files: no crash, stable after one save");
}

/// worker: an own INI with 100 ints and doubles, checked
fn pf_ini(i: usize) {
    let mut ini = Ini::new();
    let mut ok = true;
    for k in 0..100usize {
        let key = format!("k{k}");
        let v = (i * 1000 + k) as i64;
        ok &= ini.set_int("S", &key, v) && ini.set_f64("D", &key, v as f64 / 7.0);
    }
    for k in 0..100usize {
        let key = format!("k{k}");
        let v = (i * 1000 + k) as i64;
        ok &= ini.get_int("S", &key, -1) == v
            && ini.get_f64("D", &key, 0.0).to_bits() == (v as f64 / 7.0).to_bits();
    }
    if !ok {
        INI_BAD.fetch_add(1, SeqCst);
    }
}

/// f64_format strings, f64_parse corner cases, random round trips
fn t_f64(t: &mut T) {
    for (bits, text) in F64_CASES {
        let s = f64_format(f64::from_bits(bits));
        t.expect_s(Some(&s), Some(text), text);
    }
    for (text, want, used) in PARSE_CASES {
        let (v, n) = f64_parse(text.as_bytes());
        let ok = v.to_bits() == want && n == used;
        t.check(ok, text);
        if !ok {
            out(&format!("  got 0x{:x} after {n} chars\n", v.to_bits()));
        }
    }
    let mut g = 987654321u64;
    let mut bad = 0;
    for _ in 0..NRANDOM {
        let b = rnd_next(&mut g);
        if (b >> 52) & 0x7FF == 0x7FF {
            continue; // inf / nan
        }
        let s = f64_format(f64::from_bits(b));
        let (v, _) = f64_parse(s.as_bytes());
        if v.to_bits() != b {
            bad += 1;
            if bad <= 5 {
                out(&format!("  0x{b:x} -> \"{s}\" -> 0x{:x}\n", v.to_bits()));
            }
        }
    }
    t.check(bad == 0, "f64: 20000 random doubles: format + parse is bit-exact");
}

// ---------------------------------------------------------------- cpu

/// feature bits, override, brand, popcount, the date
fn t_cpu(t: &mut T) {
    let f = plat::cpu_features();
    t.check(f & !(CPU_SSE41 | CPU_AVX2 | CPU_FMA | CPU_POPCNT | CPU_BMI2) == 0, "cpu: only defined feature bits");
    t.check(plat::cpu_features() == f, "cpu: the cached result is stable");
    if t.simd.as_deref() == Some("sse2") {
        t.check(f == 0, "cpu: --simd sse2 forces the SSE2 baseline (no bits)");
    }
    let brand = plat::cpu_brand();
    t.info(&format!("  cpu: {brand}\n"));
    t.info(&format!("  features: SSE4.1 {}  AVX2 {}  FMA {}  POPCNT {}  BMI2 {}\n", f & 1, f >> 1 & 1, f >> 2 & 1,
                    f >> 3 & 1, f >> 4 & 1));
    t.check(!brand.is_empty(), "cpu: cpu_brand");
    if f & CPU_AVX2 != 0 {
        t.check(plat::avx2_probe(), "cpu: AVX2 instructions run when CPU_AVX2 is reported");
    }
    let pc = |x: u64| x.count_ones();
    t.check(pc(0) == 0 && pc(u64::MAX) == 64 && pc(0x8000000000000001) == 2 && pc(0x5555555555555555) == 32,
            "cpu: popcount64 (0, -1, 0x8000000000000001, 0x5555...)");
    t.check(plat::cpu_count() >= 1, "cpu: plat_cpu_count >= 1");
    let (d, utc) = plat::date();
    t.info(&format!("  (plat_date: {:04}-{:02}-{:02} {})\n", d.year, d.month, d.day, if utc { "UTC" } else { "local" }));
    let ok = (2024..=2200).contains(&d.year)
        && (1..=12).contains(&d.month)
        && (1..=31).contains(&d.day)
        && d.hour < 24
        && d.min < 60
        && d.wday < 7;
    t.check(ok, "cpu: plat_date gives a plausible date and time");
}

// ---------------------------------------------------------------- crash

static CR_PASS: AtomicU32 = AtomicU32::new(0);
static CR_FAIL: AtomicU32 = AtomicU32::new(0);

/// check() for the crash hook (no T there)
fn ck(ok: bool, name: &str) {
    if ok {
        CR_PASS.fetch_add(1, SeqCst);
        out(&format!("PASS {name}\n"));
    } else {
        CR_FAIL.fetch_add(1, SeqCst);
        out(&format!("FAIL {name}\n"));
    }
}

fn cr_summary() {
    out(&format!("plattest: {} passed, {} failure(s)\n", CR_PASS.load(SeqCst), CR_FAIL.load(SeqCst)));
}

/// install the hook, then crash in plat::crash_test (a write to 0x10)
fn t_crash(_: &mut T) {
    plat::crash_hook(Some(ct_hook));
    plat::crash_test_caller(); // plat-win: crash_test from a known return address
}

/// (in the crash handler) check the CrashInfo, print the summary
fn ct_hook(ci: &plat::CrashInfo) {
    out(&format!("  (code 0x{:x}, kind {}, access {}, address 0x{:x}, {} frames)\n", ci.code, ci.kind, ci.access, ci.addr,
                 ci.nframes));
    out(&format!("  (rax {:x} rsp {:x} rip {:x} code {:x}..{:x} base {:x})\n", ci.regs[0], ci.regs[7], ci.ip, ci.code0,
                 ci.code1, ci.base));
    ck(ci.kind == plat::CK_ACCESS, "crash: kind = access violation");
    ck(ci.access == 1, "crash: a write");
    ck(ci.addr == 16, "crash: at address 0x10");
    let fault = plat::crash_test as *const () as u64 + 5;
    ck(ci.ip == fault, "crash: the instruction pointer is the faulting store");
    ck(ci.regs[0] as u32 == 16, "crash: register rax = 16");
    ck(ci.code0 <= fault && fault < ci.code1, "crash: the code range holds the faulting instruction");
    // plat-win: the call chain (plat-x11: Linux fills CrashInfo::frames too)
    {
        let ret = plat::crash_test_caller as *const () as u64 + 5;
        let n = (ci.nframes as usize).min(ci.frames.len());
        ck(ci.frames[..n].contains(&ret), "crash: the return address into ct_outer is among the frames");
    }
    cr_summary();
}

/// install the hook, then panic
fn t_panic(_: &mut T) {
    plat::crash_hook(Some(pn_hook));
    panic!("plattest: a deliberate panic");
}

fn pn_hook(ci: &plat::CrashInfo) {
    ck(ci.kind == plat::CK_PANIC, "panic: kind = panic");
    ck(ci.msg.ends_with("plattest: a deliberate panic"), "panic: the message reaches the hook");
    cr_summary();
}
