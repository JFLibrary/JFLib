// trash -- move a file or folder into the trash instead of deleting it
//
//   trash(path) -> bool (guarded.rs)   true: the item is in the trash and gone from its
//                place; false: it stays where it was (it is never deleted for good)
//   trash_raw(path)   the same without the permission check (an OS path)
//
// Windows: IFileOperation::DeleteItem with FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE and no UI,
// on a thread of its own (IFileOperation needs a single-threaded apartment, the caller's
// thread keeps its own).  A progress sink refuses every delete the shell would not recycle
// (PreDeleteItem without TSF_DELETE_RECYCLE_IF_POSSIBLE: a drive without a recycle bin, an
// item too big for it) and confirms the recycled item (PostDeleteItem with the new item in
// the bin).  COM without libc as in win_uia.rs: the vtable slots by hand.
//
// Linux: the freedesktop trash specification 1.0.  An item on the file system of
// $XDG_DATA_HOME/Trash goes there (~/.local/share/Trash), any other to the top folder of
// its own mount: $topdir/.Trash/$uid when the administrator made a sticky $topdir/.Trash
// (no link), else $topdir/.Trash-$uid (both owned by the user, no links).  First
// info/NAME.trashinfo is created exclusively ("[Trash Info]", Path = the URL-escaped
// absolute path -- below a top folder relative to it --, DeletionDate =
// YYYY-MM-DDThh:mm:ss in UTC: plat has no local time on Linux), then the item is renamed to
// files/NAME without replacing anything; a name in use gets ".2", ".3" ... before its
// extension.  Nothing is copied across file systems, a mount point is not moved, and the
// trash itself cannot be trashed.  Missing trash folders are created with mode 0700.
//
// Browser and Android: false (there is no trash).

#[cfg(windows)]
pub(crate) use win::trash_raw;

#[cfg(target_os = "linux")]
pub(crate) use lnx::trash_raw;

/// the browser and Android have no trash
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn trash_raw(_path: &str) -> bool {
    false
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod win {
    use alloc::boxed::Box;
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn LoadLibraryA(name: *const u8) -> isize;
        fn GetProcAddress(m: isize, name: *const u8) -> usize;
        fn GetFileAttributesW(name: *const u16) -> u32;
    }

    type Hr = i32;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Guid(u32, u16, u16, [u8; 8]);

    static CLSID_FILE_OPERATION: Guid = Guid(0x3AD0_5575, 0x8857, 0x4850, [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09]);
    static IID_FILE_OPERATION: Guid = Guid(0x947A_AB5F, 0x0A5C, 0x4C13, [0xB4, 0xD6, 0x4B, 0xF7, 0x83, 0x6F, 0xC9, 0xF8]);
    static IID_SHELL_ITEM: Guid = Guid(0x4382_6D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);
    const IID_UNKNOWN: Guid = Guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    const IID_PROGRESS_SINK: Guid = Guid(0x04B0_F1A7, 0x9490, 0x44BC, [0x96, 0xE1, 0x42, 0x96, 0xA3, 0x12, 0x52, 0xE2]);

    const S_OK: Hr = 0;
    const E_NOINTERFACE: Hr = 0x8000_4002u32 as Hr;
    const E_ABORT: Hr = 0x8000_4004u32 as Hr;
    const CLSCTX_INPROC_SERVER: u32 = 1;
    const COINIT_APARTMENTTHREADED: u32 = 2;
    const COINIT_DISABLE_OLE1DDE: u32 = 4;
    // FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR | FOF_NOERRORUI (FOF_NO_UI), FOF_ALLOWUNDO
    const FOF_NO_UI_UNDO: u32 = 0x0004 | 0x0010 | 0x0200 | 0x0400 | 0x0040;
    const FOFX_RECYCLEONDELETE: u32 = 0x0008_0000;
    const TSF_DELETE_RECYCLE_IF_POSSIBLE: u32 = 0x80;
    // IFileOperation vtable slots
    const VT_RELEASE: usize = 2;
    const VT_SET_FLAGS: usize = 5;
    const VT_DELETE_ITEM: usize = 18;
    const VT_PERFORM: usize = 21;
    const VT_ANY_ABORTED: usize = 22;
    const NOT_FOUND: u32 = u32::MAX; // GetFileAttributesW

    /// a function of a system DLL (0 = missing)
    fn dll_proc(dll: &[u8], name: &[u8]) -> usize {
        // SAFETY: NUL-terminated names; both calls return 0 on failure.
        unsafe {
            let m = LoadLibraryA(dll.as_ptr());
            if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
        }
    }

    fn exists(w: &[u16]) -> bool {
        // SAFETY: a NUL-terminated path.
        unsafe { GetFileAttributesW(w.as_ptr()) != NOT_FOUND }
    }

    /// path into the recycle bin -> whether it went there (and is gone from its place)
    pub(crate) fn trash_raw(path: &str) -> bool {
        let Some(n) = crate::policy::normalize(path) else { return false };
        // never a drive ("C:/") or a share ("//server/share")
        let root = n.len() <= 3 || (n.starts_with("//") && n[2..].matches('/').count() < 2);
        if root {
            return false;
        }
        let w = crate::to_wide(&n.replace('/', "\\"));
        if !exists(&w) {
            return false;
        }
        let done = Arc::new(AtomicU32::new(0));
        let (d, wt) = (done.clone(), w.clone());
        let Some(t) = crate::thread_spawn(0, Box::new(move || d.store(recycle(&wt) as u32, Ordering::Release))) else {
            return false;
        };
        t.join();
        done.load(Ordering::Acquire) == 1 && !exists(&w)
    }

    // COM in a single-threaded apartment of this (new) thread, then the operation
    fn recycle(w: &[u16]) -> bool {
        let ci = dll_proc(b"ole32.dll\0", b"CoInitializeEx\0");
        let cu = dll_proc(b"ole32.dll\0", b"CoUninitialize\0");
        let cc = dll_proc(b"ole32.dll\0", b"CoCreateInstance\0");
        let mk = dll_proc(b"shell32.dll\0", b"SHCreateItemFromParsingName\0");
        if ci == 0 || cu == 0 || cc == 0 || mk == 0 {
            return false;
        }
        // SAFETY: CoInitializeEx(LPVOID, DWORD) -> HRESULT and CoUninitialize() of ole32; the
        // operation runs between them on this thread.
        unsafe {
            let init: unsafe extern "system" fn(usize, u32) -> Hr = core::mem::transmute(ci);
            if init(0, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) < 0 {
                return false;
            }
            let ok = operate(cc, mk, w);
            let uninit: unsafe extern "system" fn() = core::mem::transmute(cu);
            uninit();
            ok
        }
    }

    /// the function in vtable slot k of COM object o
    ///
    /// SAFETY: o is a live COM interface pointer whose vtable has more than k entries.
    unsafe fn vf(o: usize, k: usize) -> usize {
        unsafe { *(*(o as *const *const usize)).add(k) }
    }

    /// SAFETY: o is a live COM interface pointer the caller holds a reference of.
    unsafe fn release(o: usize) {
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> u32>(vf(o, VT_RELEASE))(o) };
    }

    /// IShellItem of w, IFileOperation::DeleteItem with the sink, PerformOperations
    ///
    /// SAFETY: COM is initialised on this thread; cc / mk are CoCreateInstance and
    /// SHCreateItemFromParsingName; w is NUL-terminated.
    unsafe fn operate(cc: usize, mk: usize, w: &[u16]) -> bool {
        unsafe {
            let make: unsafe extern "system" fn(*const u16, usize, *const Guid, *mut usize) -> Hr = core::mem::transmute(mk);
            let mut item = 0usize;
            if make(w.as_ptr(), 0, &IID_SHELL_ITEM, &mut item) < 0 || item == 0 {
                return false;
            }
            let create: unsafe extern "system" fn(*const Guid, usize, u32, *const Guid, *mut usize) -> Hr = core::mem::transmute(cc);
            let mut op = 0usize;
            if create(&CLSID_FILE_OPERATION, 0, CLSCTX_INPROC_SERVER, &IID_FILE_OPERATION, &mut op) < 0 || op == 0 {
                release(item);
                return false;
            }
            // the sink outlives the operation: op is released below, before it goes
            let sink = Sink { vt: &VT, recycled: AtomicU32::new(0), refused: AtomicU32::new(0) };
            let set: unsafe extern "system" fn(usize, u32) -> Hr = core::mem::transmute(vf(op, VT_SET_FLAGS));
            // FOFX_RECYCLEONDELETE: Windows 8 and later (before, FOF_ALLOWUNDO alone recycles)
            let mut ok = set(op, FOF_NO_UI_UNDO | FOFX_RECYCLEONDELETE) >= 0 || set(op, FOF_NO_UI_UNDO) >= 0;
            let delete: unsafe extern "system" fn(usize, usize, *const Sink) -> Hr = core::mem::transmute(vf(op, VT_DELETE_ITEM));
            ok = ok && delete(op, item, &sink) >= 0;
            let perform: unsafe extern "system" fn(usize) -> Hr = core::mem::transmute(vf(op, VT_PERFORM));
            ok = ok && perform(op) >= 0;
            let any: unsafe extern "system" fn(usize, *mut i32) -> Hr = core::mem::transmute(vf(op, VT_ANY_ABORTED));
            let mut aborted = 0i32;
            ok = ok && any(op, &mut aborted) >= 0 && aborted == 0;
            release(op);
            release(item);
            ok && sink.recycled.load(Ordering::Acquire) == 1 && sink.refused.load(Ordering::Acquire) == 0
        }
    }

    // ---- the IFileOperationProgressSink of one operation (on the operating thread's stack)

    #[repr(C)]
    struct Sink {
        vt: *const SinkVt,
        /// PostDeleteItem: the item is in the recycle bin
        recycled: AtomicU32,
        /// PreDeleteItem: a delete that would not recycle was refused
        refused: AtomicU32,
    }

    type This = *const Sink;

    #[repr(C)]
    struct SinkVt {
        qi: unsafe extern "system" fn(This, *const Guid, *mut This) -> Hr,
        add: unsafe extern "system" fn(This) -> u32,
        rel: unsafe extern "system" fn(This) -> u32,
        start: unsafe extern "system" fn(This) -> Hr,
        finish: unsafe extern "system" fn(This, Hr) -> Hr,
        pre_rename: unsafe extern "system" fn(This, u32, usize, usize) -> Hr,
        post_rename: unsafe extern "system" fn(This, u32, usize, usize, Hr, usize) -> Hr,
        pre_move: unsafe extern "system" fn(This, u32, usize, usize, usize) -> Hr,
        post_move: unsafe extern "system" fn(This, u32, usize, usize, usize, Hr, usize) -> Hr,
        pre_copy: unsafe extern "system" fn(This, u32, usize, usize, usize) -> Hr,
        post_copy: unsafe extern "system" fn(This, u32, usize, usize, usize, Hr, usize) -> Hr,
        pre_delete: unsafe extern "system" fn(This, u32, usize) -> Hr,
        post_delete: unsafe extern "system" fn(This, u32, usize, Hr, usize) -> Hr,
        pre_new: unsafe extern "system" fn(This, u32, usize, usize) -> Hr,
        post_new: unsafe extern "system" fn(This, u32, usize, usize, usize, u32, Hr, usize) -> Hr,
        progress: unsafe extern "system" fn(This, u32, u32) -> Hr,
        reset: unsafe extern "system" fn(This) -> Hr,
        pause: unsafe extern "system" fn(This) -> Hr,
        resume: unsafe extern "system" fn(This) -> Hr,
    }

    static VT: SinkVt = SinkVt {
        qi: s_qi,
        add: s_ref,
        rel: s_ref,
        start: s_ok,
        finish: s_finish,
        pre_rename: s_pre3,
        post_rename: s_post5,
        pre_move: s_pre4,
        post_move: s_post6,
        pre_copy: s_pre4,
        post_copy: s_post6,
        pre_delete: s_pre_delete,
        post_delete: s_post_delete,
        pre_new: s_pre3,
        post_new: s_post7,
        progress: s_progress,
        reset: s_ok,
        pause: s_ok,
        resume: s_ok,
    };

    // SAFETY (the sink methods): this is the Sink of a running operation; out is valid for a
    // write.  The sink lives on the stack of operate: its references are not counted.
    unsafe extern "system" fn s_qi(this: This, riid: *const Guid, out: *mut This) -> Hr {
        let id = unsafe { *riid };
        let ok = id == IID_UNKNOWN || id == IID_PROGRESS_SINK;
        unsafe { *out = if ok { this } else { core::ptr::null() } };
        if ok { S_OK } else { E_NOINTERFACE }
    }
    unsafe extern "system" fn s_ref(_this: This) -> u32 {
        1
    }
    unsafe extern "system" fn s_ok(_this: This) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_finish(_this: This, _hr: Hr) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_pre3(_this: This, _f: u32, _a: usize, _b: usize) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_pre4(_this: This, _f: u32, _a: usize, _b: usize, _c: usize) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_post5(_this: This, _f: u32, _a: usize, _b: usize, _hr: Hr, _c: usize) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_post6(_this: This, _f: u32, _a: usize, _b: usize, _c: usize, _hr: Hr, _d: usize) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_post7(_this: This, _f: u32, _a: usize, _b: usize, _c: usize, _attr: u32, _hr: Hr, _d: usize) -> Hr {
        S_OK
    }
    unsafe extern "system" fn s_progress(_this: This, _total: u32, _done: u32) -> Hr {
        S_OK
    }
    // a delete that would not go into the recycle bin is refused (the item stays)
    unsafe extern "system" fn s_pre_delete(this: This, flags: u32, _item: usize) -> Hr {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE != 0 {
            return S_OK;
        }
        unsafe { (*this).refused.store(1, Ordering::Release) };
        E_ABORT
    }
    // the item in the recycle bin (created: null when it was deleted for good)
    unsafe extern "system" fn s_post_delete(this: This, _flags: u32, _item: usize, hr: Hr, created: usize) -> Hr {
        if hr >= 0 && created != 0 {
            unsafe { (*this).recycled.store(1, Ordering::Release) };
        }
        S_OK
    }
}

// ---------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
mod lnx {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use rt::os::{SYS_CLOSE, SYS_WRITE, syscall3, syscall6};

    const SYS_RENAME: usize = 82;
    const SYS_GETUID: usize = 102;
    const SYS_OPENAT: usize = 257;
    const SYS_MKDIRAT: usize = 258;
    const SYS_NEWFSTATAT: usize = 262;
    const SYS_UNLINKAT: usize = 263;
    const SYS_RENAMEAT2: usize = 316;
    const AT_FDCWD: usize = -100isize as usize;
    const AT_SYMLINK_NOFOLLOW: usize = 0x100;
    const RENAME_NOREPLACE: usize = 1;
    const O_WRONLY: usize = 1;
    const O_CREAT: usize = 0x40;
    const O_EXCL: usize = 0x80;
    const O_CLOEXEC: usize = 0x80000;
    const EINTR: isize = 4;
    const EEXIST: isize = 17;
    const EINVAL: isize = 22;
    const ENOSYS: isize = 38;
    const S_IFMT: u32 = 0o170000;
    const S_IFDIR: u32 = 0o040000;
    const S_ISVTX: u32 = 0o1000;
    const NAME_TRIES: u32 = 10_000;

    fn z(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        v
    }

    /// (st_dev, st_mode, st_uid) of path (follow: links followed)
    fn meta(path: &str, follow: bool) -> Option<(u64, u32, u32)> {
        let p = z(path);
        let mut st = [0u64; 18]; // struct stat, 144 bytes
        let flags = if follow { 0 } else { AT_SYMLINK_NOFOLLOW };
        // SAFETY: NUL-terminated path; st holds a struct stat.
        let r = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, p.as_ptr() as usize, st.as_mut_ptr() as usize, flags, 0, 0) };
        // st_dev at 0, st_mode at 24, st_uid at 28
        (r == 0).then(|| (st[0], st[3] as u32, (st[3] >> 32) as u32))
    }

    fn is_dir(m: Option<(u64, u32, u32)>) -> bool {
        m.is_some_and(|(_, mode, _)| mode & S_IFMT == S_IFDIR)
    }

    fn parent(p: &str) -> &str {
        match p.rfind('/') {
            Some(0) => "/",
            Some(k) => &p[..k],
            None => "",
        }
    }

    // a folder with mode 0700 (the umask applies), or an existing one
    fn mkdir700(path: &str) -> bool {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        let r = unsafe { syscall3(SYS_MKDIRAT, AT_FDCWD, p.as_ptr() as usize, 0o700) };
        r == 0 || r == -EEXIST
    }

    fn unlink(path: &str) {
        let p = z(path);
        // SAFETY: a NUL-terminated path.
        unsafe { syscall3(SYS_UNLINKAT, AT_FDCWD, p.as_ptr() as usize, 0) };
    }

    /// path into the freedesktop trash -> whether it went there
    pub(crate) fn trash_raw(path: &str) -> bool {
        let Some(abs) = crate::policy::normalize(path) else { return false };
        if abs == "/" {
            return false;
        }
        let Some((dev, _, _)) = meta(&abs, false) else { return false };
        // SAFETY: no arguments.
        let uid = unsafe { syscall3(SYS_GETUID, 0, 0, 0) } as u32;
        match trash_dir(&abs, dev, uid) {
            Some((trash, key)) => put(&abs, &trash, &key),
            None => false,
        }
    }

    // the trash for an item on device dev, and the Path of its info file
    fn trash_dir(abs: &str, dev: u64, uid: u32) -> Option<(String, String)> {
        let data = crate::data_dir()?;
        let home = format!("{data}/Trash");
        if abs == home || abs.starts_with(&format!("{home}/")) {
            return None; // the trash itself
        }
        // the device of the home trash: the folder, or the nearest one above it that exists
        let mut d = home.as_str();
        let home_dev = loop {
            if let Some((dv, _, _)) = meta(d, true) {
                break Some(dv);
            }
            if d == "/" || d.is_empty() {
                break None;
            }
            d = parent(d);
        };
        if home_dev == Some(dev) {
            crate::mkdir_raw(&data);
            let ok = mkdir700(&home) && is_dir(meta(&home, true)) && ready(&home, true);
            return ok.then(|| (home, String::from(abs)));
        }
        // the top folder of the item's own mount
        let mut top = parent(abs);
        if meta(top, true).map(|m| m.0) != Some(dev) {
            return None; // a mount point
        }
        while top != "/" && meta(parent(top), true).map(|m| m.0) == Some(dev) {
            top = parent(top);
        }
        let base = if top == "/" { "" } else { top };
        let key = String::from(&abs[base.len() + 1..]);
        let admin = format!("{base}/.Trash");
        let mut tries: Vec<String> = Vec::new();
        if meta(&admin, false).is_some_and(|(_, m, _)| m & S_IFMT == S_IFDIR && m & S_ISVTX != 0) {
            tries.push(format!("{admin}/{uid}"));
        }
        tries.push(format!("{base}/.Trash-{uid}"));
        for t in tries {
            let own = |p: &str| meta(p, false).is_some_and(|(_, m, u)| m & S_IFMT == S_IFDIR && u == uid);
            if mkdir700(&t) && own(&t) && ready(&t, false) {
                return Some((t, key));
            }
        }
        None
    }

    // files/ and info/ of a trash exist (follow: links of the home trash are fine)
    fn ready(trash: &str, follow: bool) -> bool {
        ["files", "info"].iter().all(|s| {
            let p = format!("{trash}/{s}");
            mkdir700(&p) && is_dir(meta(&p, follow))
        })
    }

    /// the info file (exclusively), then the item under the same free name
    fn put(abs: &str, trash: &str, key: &str) -> bool {
        let name = &abs[abs.rfind('/').map_or(0, |k| k + 1)..];
        if name.is_empty() {
            return false;
        }
        let (stem, ext) = match name.rfind('.') {
            Some(k) if k > 0 => (&name[..k], &name[k..]),
            _ => (name, ""),
        };
        let (d, _) = crate::date();
        let text = format!(
            "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
            escape(key),
            d.year,
            d.month,
            d.day,
            d.hour,
            d.min,
            d.sec
        );
        for n in 1..=NAME_TRIES {
            let free = if n == 1 { String::from(name) } else { format!("{stem}.{n}{ext}") };
            let info = format!("{trash}/info/{free}.trashinfo");
            match create_new(&info, text.as_bytes()) {
                Ok(()) => {}
                Err(e) if e == -EEXIST => continue,
                Err(_) => return false,
            }
            match rename_new(abs, &format!("{trash}/files/{free}")) {
                0 => return true,
                r => {
                    unlink(&info);
                    if r != -EEXIST {
                        return false;
                    }
                }
            }
        }
        false
    }

    // a new file with data (O_EXCL, 0600) -> Err(-errno); a failed write removes it
    fn create_new(path: &str, data: &[u8]) -> Result<(), isize> {
        let p = z(path);
        // SAFETY: a NUL-terminated path; O_EXCL never opens an existing entry.
        let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD, p.as_ptr() as usize, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0o600, 0, 0) };
        if fd < 0 {
            return Err(fd);
        }
        let mut off = 0;
        while off < data.len() {
            // SAFETY: data[off..] is readable; fd is ours.
            let n = unsafe { syscall3(SYS_WRITE, fd as usize, data[off..].as_ptr() as usize, data.len() - off) };
            if n == -EINTR {
                continue;
            }
            if n <= 0 {
                break;
            }
            off += n as usize;
        }
        // SAFETY: our descriptor.
        unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
        if off < data.len() {
            unlink(path);
            return Err(-EINVAL);
        }
        Ok(())
    }

    // rename without replacing an existing entry -> 0 or -errno (-EEXIST: the name is taken)
    fn rename_new(old: &str, new: &str) -> isize {
        let (o, n) = (z(old), z(new));
        // SAFETY: NUL-terminated paths.
        let r = unsafe { syscall6(SYS_RENAMEAT2, AT_FDCWD, o.as_ptr() as usize, AT_FDCWD, n.as_ptr() as usize, RENAME_NOREPLACE, 0) };
        if r != -EINVAL && r != -ENOSYS {
            return r;
        }
        // a file system without RENAME_NOREPLACE: look first (the info file holds the name)
        if meta(new, false).is_some() {
            return -EEXIST;
        }
        // SAFETY: NUL-terminated paths.
        unsafe { syscall3(SYS_RENAME, o.as_ptr() as usize, n.as_ptr() as usize, 0) }
    }

    // the Path value: bytes other than A-Z a-z 0-9 - . _ ~ / as %XX (RFC 3986, as GLib)
    fn escape(s: &str) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(s.len());
        for &b in s.as_bytes() {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                out.push(b as char);
            } else {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
        out
    }
}
