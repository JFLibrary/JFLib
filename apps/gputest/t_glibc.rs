// t_glibc -- the process start, threads and exit of a program linked against glibc (build_rs.py
// section "glibc", lib/plat/linux_glibc.rs), and a missing Vulkan loader on every target
//
//   glibc     (Linux; SKIP where the program is not linked against glibc) the arguments and the
//             environment reach the program; the main thread and every plat thread have their own
//             thread control block and TLS (FS base = pthread_self, errno elsewhere per thread, libc's
//             per-thread dlerror text); a thread gets the stack it asked for; 300 threads started and
//             joined leave no mapping behind; Vulkan on a plat thread (instance, device, compute).
//             Children of this program: returning from main and plat::exit on a thread run glibc's
//             exit (an at_exit handler, the exit code), also with threads still spinning and with a
//             device still open and working; a crash on the main thread and on a plat thread, a stack
//             overflow on a plat thread and a panic give the report and exit code 3 and no at_exit
//   noloader  jfgui_GPU_DLL names a missing file (by name, by path) or a library that is no Vulkan
//             loader: Err(NoLoader); a child with it starts, --info says SKIP, exit code 0

use alloc::format;
use alloc::string::String;
#[cfg(not(target_os = "android"))]
use alloc::vec::Vec;

use gpu::*;

use crate::T;

/// (Android: this program has no plat::process; the paths in this process are checked)
#[cfg(target_os = "android")]
fn child(_: &mut T, _: &[&str], _: &[(&str, Option<&str>)]) -> Option<(i32, String, String)> {
    crate::out("  note: no child processes on this target");
    None
}

/// this program again with args and env -> (exit code, stdout, stderr); None: it did not start or
/// did not end within 60 s
#[cfg(not(target_os = "android"))]
fn child(t: &mut T, args: &[&str], env: &[(&str, Option<&str>)]) -> Option<(i32, String, String)> {
    let exe = plat::current_exe().unwrap_or_default();
    let cmd = plat::process::Command { program: &exe, args, cwd: None, env };
    let mut c = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            t.fail(&format!("the child {args:?} starts (error {e})"));
            return None;
        }
    };
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let mut buf = [0u8; 4096];
    let mut drain = |c: &mut plat::process::Child| {
        while let Ok(Some(n)) = c.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            o.extend_from_slice(&buf[..n]);
        }
        while let Ok(Some(n)) = c.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            e.extend_from_slice(&buf[..n]);
        }
    };
    let t0 = plat::time_us();
    let code = loop {
        drain(&mut c);
        if let Some(code) = c.try_wait().ok().flatten() {
            break Some(code);
        }
        if plat::time_us() - t0 > 60_000_000 {
            break None;
        }
        plat::sleep_ms(5);
    };
    drain(&mut c);
    let (o, e) = (String::from_utf8_lossy(&o).into_owned(), String::from_utf8_lossy(&e).into_owned());
    let Some(code) = code else {
        t.fail(&format!("the child {args:?} ends within 60 s; stdout: {o}; stderr: {e}"));
        return None;
    };
    Some((code, o, e))
}

pub fn noloader(t: &mut T) {
    if cfg!(target_vendor = "apple") {
        crate::out("SKIP: Metal is part of the system, there is no loader to miss");
        return;
    }
    let (missing, not_vulkan): ([&str; 2], &str) = if cfg!(windows) {
        (["jfgui-no-such-vulkan.dll", "C:\\jfgui-no-such-dir\\vulkan-1.dll"], "kernel32.dll")
    } else {
        (["jfgui-no-such-vulkan.so.1", "/jfgui-no-such-dir/libvulkan.so.1"], "libc.so.6")
    };
    for name in missing.iter().chain([&not_vulkan]) {
        plat::setenv("jfgui_GPU_DLL", Some(name));
        let r = Instance::new();
        plat::setenv("jfgui_GPU_DLL", None);
        t.ok(matches!(r, Err(GpuError::NoLoader)), &format!("jfgui_GPU_DLL={name}: Err(NoLoader)"));
    }
    if let Some((code, o, e)) = child(t, &["--info"], &[("jfgui_GPU_DLL", Some(missing[0]))]) {
        t.ok(code == 0 && o.contains("SKIP"), &format!("a program without the loader starts: --info says SKIP, exit code {code}; {e}"));
    }
}

#[cfg(not(jfgui_glibc))]
pub fn glibc(_: &mut T) {
    crate::out("SKIP: this program is not linked against glibc");
}

#[cfg(not(jfgui_glibc))]
pub fn glibc_child(_: &str) -> i32 {
    2
}

#[cfg(jfgui_glibc)]
pub use imp::{glibc, glibc_child};

#[cfg(jfgui_glibc)]
mod imp {
    use alloc::boxed::Box;
    use alloc::format;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::SeqCst};

    use super::child;
    use crate::{Gpu, T, out, setup};

    const MARK: &str = "at_exit ran";

    fn mark_exit() {
        out(MARK);
    }

    /// recursion over n * 1 KiB of stack
    #[inline(never)]
    fn burn(n: usize) -> u32 {
        let mut a = [n as u8; 1024];
        core::hint::black_box(&mut a);
        if n == 0 { a[0] as u32 } else { burn(n - 1).wrapping_add(a[1023] as u32) }
    }

    /// the mappings of this process
    fn maps() -> usize {
        plat::read_file("/proc/self/maps").map_or(0, |d| d.iter().filter(|&&c| c == b'\n').count())
    }

    /// start, threads, TLS
    fn threads(t: &mut T) {
        let (fs, me, errno) = plat::glibc_thread_ids();
        out(&format!("  main thread: FS base {fs:x}, errno at {errno:x}"));
        t.ok(fs != 0 && (fs == me || cfg!(target_arch = "aarch64") && me < fs && fs - me < 8192), "main thread: FS base = pthread_self");
        t.ok(errno != 0, "main thread: errno has an address");
        // every thread its own control block and TLS
        const N: usize = 8;
        let ids: Arc<[[AtomicUsize; 3]; N]> = Arc::new(core::array::from_fn(|_| core::array::from_fn(|_| AtomicUsize::new(0))));
        // all at once, each with its own libc error text
        let go = Arc::new(AtomicU32::new(0));
        let dl_ok = Arc::new(AtomicU32::new(0));
        let mut joins = Vec::new();
        for k in 0..N {
            let (ids, go, dl_ok) = (ids.clone(), go.clone(), dl_ok.clone());
            let th = plat::thread_spawn(256 << 10, Box::new(move || {
                let (a, b, c) = plat::glibc_thread_ids();
                ids[k][0].store(a, SeqCst);
                ids[k][1].store(b, SeqCst);
                ids[k][2].store(c, SeqCst);
                let name = format!("jfgui-no-such-library-{k}.so.1");
                let open = plat::dl_open(&name);
                go.fetch_add(1, SeqCst);
                while go.load(SeqCst) < N as u32 {
                    plat::thread_yield(); // every thread has failed once: the texts must not mix
                }
                if open.is_none() && plat::dl_error().contains(&name) {
                    dl_ok.fetch_add(1, SeqCst);
                }
            }));
            let Some(th) = th else {
                t.fail("thread_spawn");
                return;
            };
            joins.push(th);
        }
        for th in joins {
            th.join();
        }
        let got: Vec<[usize; 3]> = ids.iter().map(|x| [x[0].load(SeqCst), x[1].load(SeqCst), x[2].load(SeqCst)]).collect();
        t.ok(got.iter().all(|x| x[0] != 0 && (x[0] == x[1] || cfg!(target_arch = "aarch64") && x[1] < x[0] && x[0] - x[1] < 8192) && x[2] != 0), "threads: FS base = pthread_self on each");
        let distinct = |i: usize, extra: usize| {
            let mut v: Vec<usize> = got.iter().map(|x| x[i]).collect();
            v.push(extra);
            v.sort_unstable();
            v.dedup();
            v.len() == N + 1
        };
        t.ok(distinct(0, fs) && distinct(2, errno), &format!("threads: {N} control blocks and errno addresses of their own"));
        t.ok(dl_ok.load(SeqCst) == N as u32, &format!("threads: dlerror per thread, {} of {N} kept their own text", dl_ok.load(SeqCst)));
        // the stack asked for
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let th = plat::thread_spawn(256 << 10, Box::new(move || {
            core::hint::black_box(burn(235));
            d2.store(true, SeqCst);
        }));
        if let Some(th) = th {
            th.join();
        }
        t.ok(done.load(SeqCst), "a thread of 256 KiB has about that much stack (235 KiB used)");
        // a thread that joins itself: pthread_join says EDEADLK, and the stack it still runs on
        // stays mapped (it used to be unmapped under it)
        let (tx, rx) = plat::sync::channel::<plat::Thread>(1);
        let alive = Arc::new(AtomicBool::new(false));
        let a2 = alive.clone();
        let th = plat::thread_spawn(256 << 10, Box::new(move || {
            if let Some(me) = rx.recv() {
                me.join();
            }
            core::hint::black_box(burn(100));
            a2.store(true, SeqCst);
        }));
        if let Some(th) = th {
            let _ = tx.send(th);
            let t0 = plat::time_us();
            while !alive.load(SeqCst) && plat::time_us() - t0 < 5_000_000 {
                plat::sleep_ms(1);
            }
        }
        t.ok(alive.load(SeqCst), "a thread that joins itself goes on running on its stack");
        // nothing stays behind
        let before = maps();
        let mut started = 0;
        for _ in 0..300 {
            if let Some(th) = plat::thread_spawn(64 << 10, Box::new(|| {})) {
                th.join();
                started += 1;
            }
        }
        let after = maps();
        t.ok(started == 300, "300 threads start and end");
        t.ok(after <= before + 4, &format!("after 300 joined threads: {before} mappings before, {after} after"));
    }

    /// instance, device and compute on a plat thread of the UI output's size
    fn vulkan_on_thread(t: &mut T) {
        let fails = Arc::new(AtomicU32::new(u32::MAX));
        let f2 = fails.clone();
        let th = plat::thread_spawn(2 << 20, Box::new(move || {
            let mut t = T { fails: 0, validate: false, expected: 0 };
            if let Some(g) = setup(&mut t) {
                crate::t_compute::fill_check(&mut t, &g.dev, 1000, "on a plat thread: compute");
            }
            f2.store(t.fails, SeqCst);
        }));
        match th {
            Some(th) => th.join(),
            None => t.fail("thread_spawn"),
        }
        t.fails += fails.load(SeqCst).min(1000);
    }

    /// (kind, the exit code, at_exit runs, what stderr must say)
    const CHILDREN: [(&str, i32, bool, &str); 8] = [
        ("return", 7, true, ""),
        ("exit-thread", 9, true, ""),
        ("exit-busy", 5, true, ""),
        ("exit-gpu", 6, true, ""),
        ("crash", 3, false, "jfgui: crash signal 11"),
        ("crash-thread", 3, false, "jfgui: crash signal 11"),
        ("overflow", 3, false, "jfgui: crash signal 11"),
        ("panic", 3, false, "a planned panic"),
    ];

    pub fn glibc(t: &mut T) {
        out(&format!("  glibc {}", plat::glibc_version()));
        if let Some((code, o, e)) = child(t, &["--glibc-child", "start", "a b", "ü"], &[("jfgui_GLIBC_TEST", Some("wert"))]) {
            t.ok(code == 0 && o.contains("args [\"a b\", \"ü\"] env Some(\"wert\")"), &format!("start: arguments and environment (exit code {code}): {o}{e}"));
        }
        threads(t);
        vulkan_on_thread(t);
        for (kind, want, runs, says) in CHILDREN {
            let Some((code, o, e)) = child(t, &["--glibc-child", kind], &[]) else { continue };
            let what = format!("{kind}: exit code {code}, at_exit {}", o.contains(MARK));
            t.ok(code == want && o.contains(MARK) == runs && e.contains(says), &what);
            if code != want || o.contains(MARK) != runs || !e.contains(says) {
                out(&format!("  stdout: {o}\n  stderr: {e}"));
            }
            if kind == "crash" {
                t.ok(o.contains("frames ok"), &format!("crash: the report has the call chain and the code range: {o}"));
            }
        }
    }

    fn crash_hook(ci: &plat::CrashInfo) {
        let fault = plat::crash_test as *const () as u64 + 5;
        let ret = plat::crash_test_caller as *const () as u64 + 5;
        let n = (ci.nframes as usize).min(ci.frames.len());
        if ci.kind == plat::CK_ACCESS && ci.addr == 16 && ci.ip == fault && ci.code0 <= fault && fault < ci.code1
            && ci.frames[..n].contains(&ret) {
            out("frames ok");
        } else {
            out(&format!("crash report: kind {} address {:x} ip {:x}, code {:x}..{:x}, {n} frames", ci.kind, ci.addr, ci.ip, ci.code0, ci.code1));
        }
    }

    /// what the children of glibc do -> the exit code
    pub fn glibc_child(kind: &str) -> i32 {
        plat::at_exit(mark_exit);
        match kind {
            "start" => {
                let a = plat::args();
                out(&format!("args {:?} env {:?}", a.iter().skip(3).collect::<Vec<_>>(), plat::env("jfgui_GLIBC_TEST")));
                plat::thread_spawn(0, Box::new(|| {})).map(|t| t.join());
                0
            }
            "return" => 7,
            "exit-thread" => {
                let _ = plat::thread_spawn(0, Box::new(|| plat::exit(9)));
                plat::sleep_ms(30_000);
                1
            }
            "exit-busy" => {
                for _ in 0..4 {
                    let _ = plat::thread_spawn(0, Box::new(|| {
                        loop {
                            core::hint::black_box(burn(3));
                        }
                    }));
                }
                plat::sleep_ms(50);
                5
            }
            "exit-gpu" => {
                // the instance and device stay open, work may still run: what the driver's
                // destructors meet at the end of a program that does not clean up
                let mut t = T { fails: 0, validate: false, expected: 0 };
                if let Some(g) = setup(&mut t) {
                    crate::t_compute::fill_check(&mut t, &g.dev, 1000, "before the exit");
                    if let Ok(e) = g.dev.encoder() {
                        if let Ok(cmd) = e.finish() {
                            let _ = g.dev.queue().submit([cmd]);
                        }
                    }
                    core::mem::forget::<Gpu>(g);
                }
                6
            }
            "crash" => {
                plat::crash_hook(Some(crash_hook));
                plat::crash_test_caller()
            }
            "crash-thread" => {
                let _ = plat::thread_spawn(0, Box::new(|| plat::crash_test_caller()));
                plat::sleep_ms(30_000);
                1
            }
            "overflow" => {
                let _ = plat::thread_spawn(128 << 10, Box::new(|| {
                    core::hint::black_box(burn(1 << 20));
                }));
                plat::sleep_ms(30_000);
                1
            }
            "panic" => panic!("a planned panic"),
            _ => 2,
        }
    }
}
