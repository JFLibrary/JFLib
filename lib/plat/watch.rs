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
// are added (and reported whole), a full queue or the watch limit reports an overflow.
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
        for root in roots {
            let w: Vec<u16> = crate::fs::long_path(root).encode_utf16().chain(core::iter::once(0)).collect();
            // SAFETY: a NUL-terminated wide path; no security attributes or template.
            let dir = unsafe { CreateFileW(w.as_ptr(), FILE_LIST_DIRECTORY, SHARE_ALL, 0, OPEN_EXISTING, BACKUP_SEMANTICS | OVERLAPPED, 0) };
            if dir == INVALID {
                return Err(alloc::format!("{} kann nicht überwacht werden", root));
            }
            // SAFETY: an unnamed manual-reset event.
            let event = unsafe { CreateEventW(0, 1, 0, 0) };
            let h = Handles { dir, event };
            let sh = shared.clone();
            let base = root.clone();
            match crate::thread_spawn(128 * 1024, alloc::boxed::Box::new(move || run(h, &sh, &base))) {
                Some(t) => out.push(t),
                None => return Err(String::from("kein Thread für die Überwachung")),
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
    use rt::os::syscall3;

    const SYS_READ: usize = 0;
    const SYS_CLOSE: usize = 3;
    const SYS_POLL: usize = 7;
    const SYS_INOTIFY_ADD_WATCH: usize = 254;
    const SYS_INOTIFY_INIT1: usize = 294;
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
        dirs: BTreeMap<i32, String>,
    }

    fn add(st: &mut State, dir: &str, sh: &Shared) {
        let z: Vec<u8> = dir.bytes().chain(core::iter::once(0)).collect();
        // SAFETY: a NUL-terminated path.
        let wd = unsafe { syscall3(SYS_INOTIFY_ADD_WATCH, st.fd, z.as_ptr() as usize, MASK as usize) };
        if wd < 0 {
            sh.overflow(); // the watch limit or a folder that went away: rescans cover it
            return;
        }
        st.dirs.insert(wd as i32, String::from(dir));
        for e in crate::fs::read_dir(dir).unwrap_or_default() {
            if e.kind == crate::fs::FileKind::Dir {
                add(st, &crate::fs::join(dir, &e.name), sh);
            }
        }
    }

    pub fn start(roots: &[String], shared: &Arc<Shared>) -> Result<Vec<crate::Thread>, String> {
        // SAFETY: no pointers.
        let fd = unsafe { syscall3(SYS_INOTIFY_INIT1, IN_NONBLOCK | IN_CLOEXEC, 0, 0) };
        if fd < 0 {
            return Err(String::from("inotify ist nicht verfügbar"));
        }
        let mut st = State { fd: fd as usize, dirs: BTreeMap::new() };
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
            None => Err(String::from("kein Thread für die Überwachung")),
        }
    }

    fn run(fd: Fd, mut st: State, sh: &Shared) {
        let mut buf = alloc::vec![0u8; 64 * 1024];
        while !sh.stop.load(Ordering::SeqCst) {
            let mut pfd = [fd.0 as i32, 1i32 /* POLLIN */];
            // SAFETY: one pollfd (fd: i32, events: i16, revents: i16 = the second i32).
            let r = unsafe { syscall3(SYS_POLL, pfd.as_mut_ptr() as usize, 1, 300) };
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
                sh.push(path);
            }
        }
        // SAFETY: our descriptor.
        unsafe { syscall3(SYS_CLOSE, fd.0, 0, 0) };
    }
}
