// watch -- changes below folders, as they happen (continuous data protection)
//
//   Watcher::new(&roots) -> Result<Watcher, String>   recursive; one thread watches
//   watcher.take(wait_ms) -> Changes                   the paths changed since the last take
//                                                      (waits up to wait_ms for the first);
//                                                      overflow: events were lost, rescan
//   dropping the watcher stops it
//
// Windows: ReadDirectoryChangesW per root (subtree, overlapped, 64 KiB buffer; an empty
// answer means the buffer overflowed).  Linux: inotify with a watch per folder; new folders
// are added (and reported whole), a full queue or the watch limit reports an overflow (a full
// queue after the whole tree is watched again: lost events may hide new folders).
// macOS: FSEvents with file events (dropped events report an overflow); iOS: none.
// Paths are absolute, in the form of the platform; the folder of a change is enough for a
// caller that walks the tree again.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::sync::{Condvar, Mutex};

/// what changed since the last take
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub paths: Vec<String>,
    /// events were lost: everything below the roots may have changed
    pub overflow: bool,
}

#[derive(Default)]
struct Pending {
    paths: BTreeSet<String>,
    overflow: bool,
}

struct Shared {
    pending: Mutex<Pending>,
    cv: Condvar,
    stop: AtomicBool,
}

#[cfg_attr(target_os = "ios", allow(dead_code))] // no FSEvents on iOS
impl Shared {
    fn push(&self, p: String) {
        let mut g = self.pending.lock();
        if g.paths.len() >= 200_000 {
            g.overflow = true;
        } else {
            g.paths.insert(p);
        }
        drop(g);
        self.cv.notify_all();
    }

    fn overflow(&self) {
        self.pending.lock().overflow = true;
        self.cv.notify_all();
    }
}

pub struct Watcher {
    shared: Arc<Shared>,
    threads: Vec<crate::Thread>,
}

impl Watcher {
    pub fn new(roots: &[String]) -> Result<Watcher, String> {
        let shared = Arc::new(Shared { pending: Mutex::new(Pending::default()), cv: Condvar::new(), stop: AtomicBool::new(false) });
        let threads = imp::start(roots, &shared)?;
        Ok(Watcher { shared, threads })
    }

    /// the changes so far (waiting up to wait_ms when there are none yet)
    pub fn take(&self, wait_ms: u32) -> Changes {
        let mut g = self.shared.pending.lock();
        if g.paths.is_empty() && !g.overflow && wait_ms > 0 {
            g = self.shared.cv.wait_timeout(g, Some(wait_ms)).0;
        }
        let out = Changes { paths: core::mem::take(&mut g.paths).into_iter().collect(), overflow: g.overflow };
        g.overflow = false;
        out
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            t.join();
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    const FILE_LIST_DIRECTORY: u32 = 1;
    const SHARE_ALL: u32 = 7;
    const OPEN_EXISTING: u32 = 3;
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const OVERLAPPED: u32 = 0x4000_0000;
    const FILTER: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x10 | 0x100; // names, dirs, attributes, size, write, security
    const INVALID: isize = -1;
    const WAIT_OBJECT_0: u32 = 0;
    const BUF: usize = 64 * 1024;

    #[repr(C)]
    #[derive(Default)]
    struct Ov {
        internal: usize,
        internal_high: usize,
        offset: u64,
        event: isize,
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreateFileW(name: *const u16, access: u32, share: u32, sa: usize, disp: u32, flags: u32, tmpl: isize) -> isize;
        fn CloseHandle(h: isize) -> i32;
        fn CreateEventW(sa: usize, manual: i32, initial: i32, name: usize) -> isize;
        fn ReadDirectoryChangesW(h: isize, buf: *mut u8, len: u32, subtree: i32, filter: u32, ret: *mut u32, ov: *mut Ov, cb: usize) -> i32;
        fn WaitForSingleObject(h: isize, ms: u32) -> u32;
        fn GetOverlappedResult(h: isize, ov: *mut Ov, done: *mut u32, wait: i32) -> i32;
        fn CancelIoEx(h: isize, ov: *mut Ov) -> i32;
        fn ResetEvent(h: isize) -> i32;
    }

    struct Handles {
        dir: isize,
        event: isize,
    }

    // the handles are used by the one thread that owns them
    unsafe impl Send for Handles {}

    pub fn start(roots: &[String], shared: &Arc<Shared>) -> Result<Vec<crate::Thread>, String> {
        let mut out = Vec::new();
        let fail = |out: &mut Vec<crate::Thread>, msg: String| {
            // the threads started so far must not outlive the failed watcher
            shared.stop.store(true, Ordering::SeqCst);
            for t in out.drain(..) {
                t.join();
            }
            Err(msg)
        };
        for root in roots {
            let w: Vec<u16> = crate::fs::long_path(root).encode_utf16().chain(core::iter::once(0)).collect();
            // SAFETY: a NUL-terminated wide path; no security attributes or template.
            let dir = unsafe { CreateFileW(w.as_ptr(), FILE_LIST_DIRECTORY, SHARE_ALL, 0, OPEN_EXISTING, BACKUP_SEMANTICS | OVERLAPPED, 0) };
            if dir == INVALID {
                return fail(&mut out, alloc::format!("{} kann nicht überwacht werden", root));
            }
            // SAFETY: an unnamed manual-reset event.
            let event = unsafe { CreateEventW(0, 1, 0, 0) };
            if event == 0 {
                // SAFETY: the directory handle opened above.
                unsafe { CloseHandle(dir) };
                return fail(&mut out, alloc::format!("{} kann nicht überwacht werden", root));
            }
            let h = Handles { dir, event };
            let sh = shared.clone();
            let base = root.clone();
            match crate::thread_spawn(128 * 1024, alloc::boxed::Box::new(move || run(h, &sh, &base))) {
                Some(t) => out.push(t),
                None => {
                    // SAFETY: the handles opened above; the closure that owned them never ran.
                    unsafe {
                        CloseHandle(event);
                        CloseHandle(dir);
                    }
                    return fail(&mut out, String::from("kein Thread für die Überwachung"));
                }
            }
        }
        Ok(out)
    }

    fn run(h: Handles, sh: &Shared, base: &str) {
        let mut buf = alloc::vec![0u32; BUF / 4];
        let sep = if base.ends_with(['\\', '/']) { "" } else { "\\" };
        'outer: while !sh.stop.load(Ordering::SeqCst) {
            let mut ov = Ov { event: h.event, ..Ov::default() };
            // SAFETY: the buffer and the OVERLAPPED stay alive until the operation completed or
            // was cancelled and waited for (below).
            unsafe { ResetEvent(h.event) };
            let ok = unsafe { ReadDirectoryChangesW(h.dir, buf.as_mut_ptr() as *mut u8, BUF as u32, 1, FILTER, core::ptr::null_mut(), &mut ov, 0) };
            if ok == 0 {
                sh.overflow();
                break;
            }
            loop {
                // SAFETY: a valid event handle.
                let w = unsafe { WaitForSingleObject(h.event, 300) };
                if sh.stop.load(Ordering::SeqCst) {
                    let mut n = 0u32;
                    // SAFETY: cancel the pending read, then wait for it so the buffer is free.
                    unsafe {
                        CancelIoEx(h.dir, &mut ov);
                        GetOverlappedResult(h.dir, &mut ov, &mut n, 1);
                    }
                    break 'outer;
                }
                if w == WAIT_OBJECT_0 {
                    break;
                }
            }
            let mut n = 0u32;
            // SAFETY: the read completed (its event is set).
            if unsafe { GetOverlappedResult(h.dir, &mut ov, &mut n, 0) } == 0 || n == 0 {
                sh.overflow();
                continue;
            }
            let bytes: &[u8] = {
                // SAFETY: u32 storage viewed as bytes, n <= its length.
                unsafe { core::slice::from_raw_parts(buf.as_ptr() as *const u8, (n as usize).min(BUF)) }
            };
            let mut off = 0usize;
            loop {
                if off + 12 > bytes.len() {
                    break;
                }
                let rd = |i: usize| u32::from_le_bytes([bytes[off + i], bytes[off + i + 1], bytes[off + i + 2], bytes[off + i + 3]]);
                let next = rd(0) as usize;
                let len = rd(8) as usize;
                if off + 12 + len > bytes.len() {
                    break;
                }
                let name: Vec<u16> = bytes[off + 12..off + 12 + len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                sh.push(alloc::format!("{}{}{}", base, sep, String::from_utf16_lossy(&name)));
                if next == 0 {
                    break;
                }
                off += next;
            }
        }
        // SAFETY: owned handles.
        unsafe {
            CloseHandle(h.event);
            CloseHandle(h.dir);
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use alloc::collections::BTreeMap;
    use rt::os::{syscall3, SYS_CLOSE, SYS_INOTIFY_ADD_WATCH, SYS_INOTIFY_INIT1, SYS_INOTIFY_RM_WATCH, SYS_READ};

    const IN_NONBLOCK: usize = 0o4000;
    const IN_CLOEXEC: usize = 0o2000000;
    const IN_MODIFY: u32 = 0x2;
    const IN_ATTRIB: u32 = 0x4;
    const IN_CLOSE_WRITE: u32 = 0x8;
    const IN_MOVED_FROM: u32 = 0x40;
    const IN_MOVED_TO: u32 = 0x80;
    const IN_CREATE: u32 = 0x100;
    const IN_DELETE: u32 = 0x200;
    const IN_DELETE_SELF: u32 = 0x400;
    const IN_MOVE_SELF: u32 = 0x800;
    const IN_Q_OVERFLOW: u32 = 0x4000;
    const IN_IGNORED: u32 = 0x8000;
    const IN_ONLYDIR: u32 = 0x0100_0000;
    const IN_DONT_FOLLOW: u32 = 0x0200_0000;
    const IN_ISDIR: u32 = 0x4000_0000;
    const MASK: u32 = IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE | IN_DELETE_SELF | IN_MOVE_SELF | IN_ONLYDIR | IN_DONT_FOLLOW;

    struct Fd(usize);
    // one thread owns the descriptor
    unsafe impl Send for Fd {}

    struct State {
        fd: usize,
        roots: Vec<String>,
        dirs: BTreeMap<i32, String>,
    }

    // dir and every folder below it; a worklist, not recursion: the depth of a tree is not ours
    // to trust and the watching thread has a small stack
    fn add(st: &mut State, dir: &str, sh: &Shared) {
        let mut todo = alloc::vec![String::from(dir)];
        while let Some(dir) = todo.pop() {
            let z: Vec<u8> = dir.bytes().chain(core::iter::once(0)).collect();
            // SAFETY: a NUL-terminated path.
            let wd = unsafe { syscall3(SYS_INOTIFY_ADD_WATCH, st.fd, z.as_ptr() as usize, MASK as usize) };
            if wd < 0 {
                sh.overflow(); // the watch limit or a folder that went away: rescans cover it
                continue;
            }
            for e in crate::fs::read_dir(&dir).unwrap_or_default() {
                if e.kind == crate::fs::FileKind::Dir {
                    todo.push(crate::fs::join(&dir, &e.name));
                }
            }
            st.dirs.insert(wd as i32, dir);
        }
    }

    // after a queue overflow: the creates of new folders may be among the lost events, so
    // watch the whole tree again.  inotify_add_watch hands back the existing watch of a folder
    // it already watches (under its current path, so renamed folders get their new name);
    // watches the walk no longer reaches (moved out, deleted) end.
    fn rescan(st: &mut State, sh: &Shared) {
        let old = core::mem::take(&mut st.dirs);
        for r in st.roots.clone() {
            add(st, &r, sh);
        }
        for w in old.keys().filter(|w| !st.dirs.contains_key(w)) {
            // SAFETY: our inotify descriptor and one of its watches (gone already: EINVAL).
            unsafe { syscall3(SYS_INOTIFY_RM_WATCH, st.fd, *w as usize, 0) };
        }
    }

    pub fn start(roots: &[String], shared: &Arc<Shared>) -> Result<Vec<crate::Thread>, String> {
        // SAFETY: no pointers.
        let fd = unsafe { syscall3(SYS_INOTIFY_INIT1, IN_NONBLOCK | IN_CLOEXEC, 0, 0) };
        if fd < 0 {
            return Err(String::from("inotify ist nicht verfügbar"));
        }
        let mut st = State { fd: fd as usize, roots: roots.to_vec(), dirs: BTreeMap::new() };
        for r in roots {
            if !crate::fs::lstat(r).is_some_and(|m| m.is_dir()) {
                // SAFETY: our descriptor.
                unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
                return Err(alloc::format!("{} ist kein Ordner", r));
            }
            add(&mut st, r, shared);
        }
        let sh = shared.clone();
        let owned = Fd(fd as usize);
        match crate::thread_spawn(128 * 1024, alloc::boxed::Box::new(move || run(owned, st, &sh))) {
            Some(t) => Ok(alloc::vec![t]),
            None => {
                // SAFETY: our descriptor; the closure that owned it never ran.
                unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
                Err(String::from("kein Thread für die Überwachung"))
            }
        }
    }

    fn run(fd: Fd, mut st: State, sh: &Shared) {
        let mut buf = alloc::vec![0u8; 64 * 1024];
        while !sh.stop.load(Ordering::SeqCst) {
            let mut pfd = [fd.0 as i32, 1i32 /* POLLIN */];
            // SAFETY: one pollfd (fd: i32, events: i16, revents: i16 = the second i32).
            let r = unsafe { rt::os::poll(pfd.as_mut_ptr() as usize, 1, 300) };
            if r <= 0 {
                continue;
            }
            // SAFETY: the buffer is writable for its length.
            let n = unsafe { syscall3(SYS_READ, fd.0, buf.as_mut_ptr() as usize, buf.len()) };
            if n <= 0 {
                continue;
            }
            let mut off = 0usize;
            let n = n as usize;
            while off + 16 <= n {
                let rd = |i: usize| u32::from_ne_bytes([buf[off + i], buf[off + i + 1], buf[off + i + 2], buf[off + i + 3]]);
                let (wd, mask, len) = (rd(0) as i32, rd(4), rd(12) as usize);
                let name_b = &buf[off + 16..(off + 16 + len).min(n)];
                let end = name_b.iter().position(|&c| c == 0).unwrap_or(name_b.len());
                let name = String::from_utf8_lossy(&name_b[..end]).into_owned();
                off += 16 + len;
                if mask & IN_Q_OVERFLOW != 0 {
                    // the watches first, then the report: the caller's rescan starts when
                    // every folder is watched again, so nothing falls between the two
                    rescan(&mut st, sh);
                    sh.overflow();
                    continue;
                }
                let Some(dir) = st.dirs.get(&wd).cloned() else { continue };
                if mask & IN_IGNORED != 0 {
                    st.dirs.remove(&wd);
                    continue;
                }
                let path = if name.is_empty() { dir.clone() } else { crate::fs::join(&dir, &name) };
                if mask & IN_ISDIR != 0 && mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                    add(&mut st, &path, sh);
                }
                if mask & IN_ISDIR != 0 && mask & IN_MOVED_FROM != 0 && !name.is_empty() {
                    // a folder moved away (out of the tree, or renamed: IN_MOVED_TO watches it
                    // again): its watches and those below it end, else their events would
                    // name paths that no longer exist
                    let pre = alloc::format!("{path}/");
                    let gone: Vec<i32> = st.dirs.iter().filter(|(_, d)| **d == path || d.starts_with(&pre)).map(|(w, _)| *w).collect();
                    for w in gone {
                        st.dirs.remove(&w);
                        // SAFETY: our inotify descriptor and one of its watches.
                        unsafe { syscall3(SYS_INOTIFY_RM_WATCH, fd.0, w as usize, 0) };
                    }
                }
                sh.push(path);
            }
        }
        // SAFETY: our descriptor.
        unsafe { syscall3(SYS_CLOSE, fd.0, 0, 0) };
    }
}

// macOS: FSEvents (CoreServices) with file events, recursive, on a serial dispatch queue; one thread
// of ours owns the stream until the watcher stops, then drains the queue.  FSEvents names real paths
// (/private/var for /var): a change below a root comes back under the root as the caller gave it.
// iOS has no FSEvents: Watcher::new fails there.
#[cfg(target_vendor = "apple")]
mod imp {
    use super::*;

    #[cfg(target_os = "ios")]
    pub fn start(_roots: &[String], _shared: &Arc<Shared>) -> Result<Vec<crate::Thread>, String> {
        Err(String::from("auf iOS gibt es keine Ordnerüberwachung"))
    }

    #[cfg(target_os = "macos")]
    pub fn start(roots: &[String], shared: &Arc<Shared>) -> Result<Vec<crate::Thread>, String> {
        for r in roots {
            if !crate::fs::lstat(r).is_some_and(|m| m.is_dir()) {
                return Err(alloc::format!("{} ist kein Ordner", r));
            }
        }
        let roots: Vec<(String, String)> = roots.iter().map(|r| (real_path(r).unwrap_or_else(|| r.clone()), r.clone())).collect();
        let sh = shared.clone();
        // the stream is made on the watching thread, which answers whether that worked
        let ready = Arc::new(crate::sync::Mutex::new(None::<Result<(), String>>));
        let r2 = ready.clone();
        let t = crate::thread_spawn(128 * 1024, alloc::boxed::Box::new(move || mac::run(roots, sh, r2)))
            .ok_or_else(|| String::from("kein Thread für die Überwachung"))?;
        loop {
            if let Some(r) = ready.lock().take() {
                return match r {
                    Ok(()) => Ok(alloc::vec![t]),
                    Err(e) => {
                        t.join();
                        Err(e)
                    }
                };
            }
            crate::sleep_ms(5);
        }
    }

    #[cfg(target_os = "macos")]
    fn real_path(p: &str) -> Option<String> {
        #[link(name = "System")]
        unsafe extern "C" {
            fn realpath(path: *const u8, out: *mut u8) -> *mut u8;
            fn free(p: *mut u8);
        }
        let z: Vec<u8> = p.bytes().chain(core::iter::once(0)).collect();
        // SAFETY: a NUL-terminated path; realpath mallocs the answer, freed below.
        unsafe {
            let r = realpath(z.as_ptr(), core::ptr::null_mut());
            if r.is_null() {
                return None;
            }
            let mut n = 0;
            while *r.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf8_lossy(core::slice::from_raw_parts(r, n)).into_owned();
            free(r);
            Some(s)
        }
    }

    #[cfg(target_os = "macos")]
    mod mac {
        use super::super::*;
        use core::ffi::{c_char, c_void};

        type CfRef = *const c_void;
        #[repr(C)]
        struct Context {
            version: isize,
            info: *mut c_void,
            retain: usize,
            release: usize,
            copy_description: usize,
        }
        type Callback = extern "C" fn(CfRef, *mut c_void, usize, *const *const c_char, *const u32, *const u64);

        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFTypeArrayCallBacks: [usize; 5];
            fn CFStringCreateWithBytes(alloc: CfRef, bytes: *const u8, n: isize, encoding: u32, external: u8) -> CfRef;
            fn CFArrayCreate(alloc: CfRef, values: *const CfRef, n: isize, callbacks: *const [usize; 5]) -> CfRef;
            fn CFRelease(r: CfRef);
        }
        #[link(name = "CoreServices", kind = "framework")]
        unsafe extern "C" {
            fn FSEventStreamCreate(alloc: CfRef, cb: Callback, ctx: *const Context, paths: CfRef, since: u64, latency: f64, flags: u32) -> *mut c_void;
            fn FSEventStreamSetDispatchQueue(s: *mut c_void, q: *mut c_void);
            fn FSEventStreamStart(s: *mut c_void) -> u8;
            fn FSEventStreamStop(s: *mut c_void);
            fn FSEventStreamFlushSync(s: *mut c_void);
            fn FSEventStreamInvalidate(s: *mut c_void);
            fn FSEventStreamRelease(s: *mut c_void);
        }
        #[link(name = "System")]
        unsafe extern "C" {
            fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
            fn dispatch_sync_f(q: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
            fn dispatch_release(o: *mut c_void);
        }
        const UTF8: u32 = 0x0800_0100;
        const SINCE_NOW: u64 = u64::MAX;
        const FLAG_NO_DEFER: u32 = 0x02;
        const FLAG_WATCH_ROOT: u32 = 0x04;
        const FLAG_FILE_EVENTS: u32 = 0x10;
        const EV_MUST_SCAN: u32 = 0x01;
        const EV_USER_DROPPED: u32 = 0x02;
        const EV_KERNEL_DROPPED: u32 = 0x04;

        /// the name of the start's barrier files (MARK + pid + a counter): never reported
        const MARK: &str = ".jfgui-watch-";

        /// what the callback reaches through the stream's info pointer
        struct Info {
            shared: Arc<Shared>,
            roots: Vec<(String, String)>, // (real path, as the caller gave it)
            /// the barrier files of the start (real paths) not seen yet; events before the last of
            /// them happened before the start (FSEvents delivers in order) and are dropped
            marks: crate::sync::Mutex<Vec<String>>,
            started: core::sync::atomic::AtomicBool,
        }

        extern "C" fn on_events(_s: CfRef, info: *mut c_void, n: usize, paths: *const *const c_char, flags: *const u32, _ids: *const u64) {
            // SAFETY: info is the Info run() keeps alive until the queue has drained; FSEvents hands
            // n C strings and n flags.
            let (info, paths, flags) = unsafe { (&*(info as *const Info), core::slice::from_raw_parts(paths, n), core::slice::from_raw_parts(flags, n)) };
            for (p, f) in paths.iter().zip(flags) {
                if f & (EV_MUST_SCAN | EV_USER_DROPPED | EV_KERNEL_DROPPED) != 0 {
                    info.shared.overflow();
                    continue;
                }
                // SAFETY: a NUL-terminated path of this event.
                let real = unsafe { core::ffi::CStr::from_ptr(*p) }.to_string_lossy();
                let mut path = String::from(real.trim_end_matches('/'));
                if path.rsplit('/').next().is_some_and(|n| n.starts_with(MARK)) {
                    let mut m = info.marks.lock();
                    m.retain(|x| *x != path);
                    if m.is_empty() {
                        info.started.store(true, Ordering::SeqCst);
                    }
                    continue;
                }
                if !info.started.load(Ordering::SeqCst) {
                    continue; // from before the start
                }
                for (rp, given) in &info.roots {
                    if path == *rp || path.starts_with(&alloc::format!("{rp}/")) {
                        path = alloc::format!("{given}{}", &path[rp.len()..]);
                        break;
                    }
                }
                info.shared.push(path);
            }
        }

        extern "C" fn nothing(_: *mut c_void) {}

        pub fn run(roots: Vec<(String, String)>, shared: Arc<Shared>, ready: Arc<crate::sync::Mutex<Option<Result<(), String>>>>) {
            // SAFETY: no arguments.
            let pid = unsafe { crate::apple::ffi::getpid() };
            static SEQ: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let marks: Vec<String> = roots.iter().map(|(rp, _)| alloc::format!("{rp}/{MARK}{pid}-{n}")).collect();
            let info = alloc::boxed::Box::new(Info { shared: shared.clone(), roots, marks: crate::sync::Mutex::new(marks.clone()),
                                                     started: core::sync::atomic::AtomicBool::new(false) });
            // SAFETY: CoreFoundation objects made and released here; the context is copied by FSEvents;
            // info outlives the stream (dropped after the queue drained).
            unsafe {
                let strs: Vec<CfRef> = info.roots.iter()
                    .map(|(rp, _)| CFStringCreateWithBytes(core::ptr::null(), rp.as_ptr(), rp.len() as isize, UTF8, 0))
                    .collect();
                let arr = CFArrayCreate(core::ptr::null(), strs.as_ptr(), strs.len() as isize, &kCFTypeArrayCallBacks);
                for s in &strs {
                    CFRelease(*s);
                }
                let ctx = Context { version: 0, info: &*info as *const Info as *mut c_void, retain: 0, release: 0, copy_description: 0 };
                let stream = FSEventStreamCreate(core::ptr::null(), on_events, &ctx, arr, SINCE_NOW, 0.05,
                                                 FLAG_NO_DEFER | FLAG_WATCH_ROOT | FLAG_FILE_EVENTS);
                CFRelease(arr);
                if stream.is_null() {
                    *ready.lock() = Some(Err(String::from("FSEvents ist nicht verfügbar")));
                    return;
                }
                let q = dispatch_queue_create(c"org.jflib.watch".as_ptr(), core::ptr::null());
                FSEventStreamSetDispatchQueue(stream, q);
                if FSEventStreamStart(stream) == 0 {
                    FSEventStreamInvalidate(stream);
                    FSEventStreamRelease(stream);
                    dispatch_release(q);
                    *ready.lock() = Some(Err(String::from("FSEvents startet nicht")));
                    return;
                }
                // FSEvents delivers what happened before the start (a file written just before
                // Watcher::new) late: a barrier file in every root, and nothing is reported before all of
                // them came through (a watcher reports changes from its start on, as inotify does).  A root
                // that refuses the file (read-only): two flushes and 300 ms must do.
                let mut placed = 0;
                for m in &marks {
                    if crate::write_file_raw(m, b"") {
                        placed += 1;
                    } else {
                        info.marks.lock().retain(|x| x != m);
                    }
                }
                if placed == 0 {
                    info.started.store(true, Ordering::SeqCst);
                }
                for _ in 0..200 {
                    FSEventStreamFlushSync(stream);
                    if info.started.load(Ordering::SeqCst) {
                        break;
                    }
                    crate::sleep_ms(10);
                }
                if placed < marks.len() {
                    crate::sleep_ms(300);
                    FSEventStreamFlushSync(stream);
                }
                info.started.store(true, Ordering::SeqCst);
                for m in &marks {
                    crate::delete_raw(m);
                }
                *shared.pending.lock() = Pending::default();
                *ready.lock() = Some(Ok(()));
                while !shared.stop.load(Ordering::SeqCst) {
                    crate::sleep_ms(100);
                }
                FSEventStreamStop(stream);
                FSEventStreamInvalidate(stream);
                dispatch_sync_f(q, core::ptr::null_mut(), nothing); // a callback still running ends first
                FSEventStreamRelease(stream);
                dispatch_release(q);
            }
            drop(info);
        }
    }
}
