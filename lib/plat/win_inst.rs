// win_inst -- the Windows calls of the setup (apps/installer): the own exe path, known
// folders, .lnk shortcuts through COM IShellLinkW, the HKCU registry key of the "installed
// apps" entry, starting a command line             (r12-inst)  [apps/installer/src/shell.s]
//
//   exe_path() -> Some(path)        GetModuleFileNameW of the running exe as UTF-8 (None when
//                                   it does not fit PATH_CAP wide chars)  [self_path]
//   known_folder(which) -> Some(p)  0 = desktop, 1 = start menu "Programs" (SHGetKnownFolderPath,
//                                   dirs.rs)
//   shell_link(lnk, target, dir, desc) -> bool   a shortcut file lnk -> target (working
//                                   folder dir, icon 0 of the target, comment desc)  [sh_link]
//   reg_create(subkey) -> Some(RegKey)   HKCU\subkey opened for writing (created when missing);
//                                   RegKey::set_sz / set_dword, closed when dropped
//   reg_delete(subkey) -> bool      RegDeleteKeyW(HKCU, subkey)
//   reg_delete_tree(subkey) -> bool RegDeleteTreeW: the key and everything below it
//   reg_delete_value(subkey, name) -> bool   RegDeleteKeyValueW (name "" = the default value)
//   assoc_changed()                 SHChangeNotify(SHCNE_ASSOCCHANGED): Explorer re-reads the
//                                   file types and URL schemes
//   run_cmdline(cmdline, dir, hidden) -> bool    CreateProcessW (dir None = the current one,
//                                   hidden = CREATE_NO_WINDOW)  [sh_run]
//
// ole32 / shell32 / advapi32 are loaded with LoadLibraryA on the first use (inst_load, as the
// asm's sh_load): only kernel32 / user32 / gdi32 are linked.  Paths are converted with the
// asm's capacities (utf8_to_w with PATH_CAP wide chars, w_to_utf8 with PATH_CAP bytes).  The
// test mode (ASMINST_SHELL_DIR) is the app's business: these functions always touch the real
// shell / registry.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::{dll_proc, wide_cap};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(m: isize, buf: *mut u16, n: u32) -> u32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: usize, ta: usize, inherit: i32, flags: u32, env: usize,
                      dir: *const u16, si: *const StartupInfo, pi: *mut [isize; 3]) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

/// STARTUPINFOW (x64, 104 bytes)
#[repr(C)]
struct StartupInfo {
    cb: u32,
    rest: [u32; 25],
}

/// a COM GUID
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

static CLSID_SHELLLINK: Guid = Guid(0x0002_1401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_SHELLLINKW: Guid = Guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_PERSISTFILE: Guid = Guid(0x0000_010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// the capacity of the asm's path buffers: bytes of UTF-8 / wide chars (inst.inc PATH_CAP)
pub const PATH_CAP: usize = 520;
const NAME_CAP: usize = 64; // w_name
const HKCU: isize = 0xFFFF_FFFF_8000_0001u64 as isize;
const KEY_WRITE: u32 = 0x20006;
const REG_SZ: u32 = 1;
const REG_DWORD: u32 = 4;
const CLSCTX_INPROC: u32 = 1;
const COINIT_APT: u32 = 2;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// IShellLinkW / IPersistFile vtable slots
const VT_QI: usize = 0;
const VT_RELEASE: usize = 2;
const VT_SL_SETDESC: usize = 7;
const VT_SL_SETWDIR: usize = 9;
const VT_SL_SETICON: usize = 17;
const VT_SL_SETPATH: usize = 20;
const VT_PF_SAVE: usize = 6;

// the optional functions (0 = missing), resolved once
const P_COINIT: usize = 0;
const P_COCREATE: usize = 1;
const P_REGCREATE: usize = 2;
const P_REGSET: usize = 3;
const P_REGCLOSE: usize = 4;
const P_REGDELETE: usize = 5;
const P_REGTREE: usize = 6;
const P_REGDELVAL: usize = 7;
const P_CHANGENOTIFY: usize = 8;
static LOADED: AtomicU32 = AtomicU32::new(0);
static PROCS: [AtomicUsize; 9] = [const { AtomicUsize::new(0) }; 9];

/// resolve the optional APIs once and initialise COM  [sh_load]
fn inst_load() {
    if LOADED.swap(1, Ordering::SeqCst) != 0 {
        return;
    }
    let set = |i: usize, dll: &[u8], name: &[u8]| PROCS[i].store(dll_proc(dll, name), Ordering::SeqCst);
    set(P_COINIT, b"ole32.dll\0", b"CoInitializeEx\0");
    set(P_COCREATE, b"ole32.dll\0", b"CoCreateInstance\0");
    set(P_REGCREATE, b"advapi32.dll\0", b"RegCreateKeyExW\0");
    set(P_REGSET, b"advapi32.dll\0", b"RegSetValueExW\0");
    set(P_REGCLOSE, b"advapi32.dll\0", b"RegCloseKey\0");
    set(P_REGDELETE, b"advapi32.dll\0", b"RegDeleteKeyW\0");
    set(P_REGTREE, b"advapi32.dll\0", b"RegDeleteTreeW\0");
    set(P_REGDELVAL, b"advapi32.dll\0", b"RegDeleteKeyValueW\0");
    set(P_CHANGENOTIFY, b"shell32.dll\0", b"SHChangeNotify\0");
    let p = PROCS[P_COINIT].load(Ordering::SeqCst);
    if p != 0 {
        // SAFETY: CoInitializeEx(LPVOID reserved, DWORD) -> HRESULT; S_FALSE / RPC_E_CHANGED_MODE
        // are fine.
        let f: unsafe extern "system" fn(usize, u32) -> i32 = unsafe { core::mem::transmute(p) };
        unsafe { f(0, COINIT_APT) };
    }
}

fn proc_(i: usize) -> usize {
    inst_load();
    PROCS[i].load(Ordering::SeqCst)
}

/// NUL-terminated wide text -> UTF-8 of at most cap - 1 bytes (never a split character)  [w_to_utf8]
fn utf8_cap(w: &[u16], cap: usize) -> String {
    let mut s = String::new();
    for c in core::char::decode_utf16(w.iter().copied().take_while(|&c| c != 0)) {
        let c = c.unwrap_or('\u{FFFD}');
        if s.len() + c.len_utf8() > cap - 1 {
            break;
        }
        s.push(c);
    }
    s
}

/// the running exe's path, UTF-8  [self_path]
pub fn exe_path() -> Option<String> {
    let mut buf = alloc::vec![0u16; PATH_CAP];
    // SAFETY: buf holds PATH_CAP wide chars.
    let n = unsafe { GetModuleFileNameW(0, buf.as_mut_ptr(), PATH_CAP as u32) } as usize;
    if n == 0 || n >= PATH_CAP {
        return None;
    }
    Some(utf8_cap(&buf[..n], PATH_CAP))
}

/// 0 = the desktop, 1 = the start menu's "Programs" folder (dirs.rs; backslashes, at most
/// PATH_CAP - 1 bytes)  [sh_folder, real mode]
pub fn known_folder(which: u32) -> Option<String> {
    let k = if which == 0 { crate::DIR_DESKTOP as usize } else { crate::KF_PROGRAMS };
    let mut s = crate::known_folder(k)?;
    let mut n = s.len().min(PATH_CAP - 1);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    s.truncate(n);
    Some(s)
}

/// the function in vtable slot `slot` of COM object `obj`
///
/// SAFETY: obj is a live COM interface pointer whose vtable has more than `slot` entries.
unsafe fn vfn(obj: usize, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

/// release a COM object
///
/// SAFETY: obj is a live COM interface pointer owned by the caller.
unsafe fn release(obj: usize) {
    unsafe {
        let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vfn(obj, VT_RELEASE));
        f(obj);
    }
}

/// a shortcut lnk -> target (working folder dir, icon 0 of the target, comment desc)  [sh_link]
pub fn shell_link(lnk: &str, target: &str, dir: &str, desc: &str) -> bool {
    let create = proc_(P_COCREATE);
    if create == 0 {
        return false;
    }
    let (w_lnk, w_target) = (wide_cap(lnk, PATH_CAP), wide_cap(target, PATH_CAP));
    let (w_dir, w_desc) = (wide_cap(dir, PATH_CAP), wide_cap(desc, PATH_CAP));
    let mut sl: usize = 0;
    let mut pf: usize = 0;
    // SAFETY: CoCreateInstance(REFCLSID, LPUNKNOWN, DWORD, REFIID, LPVOID*) -> HRESULT; the
    // methods are called through the documented IShellLinkW / IPersistFile vtable slots with
    // NUL-terminated wide strings that outlive the calls; every object is released.
    unsafe {
        let cc: unsafe extern "system" fn(*const Guid, usize, u32, *const Guid, *mut usize) -> i32 =
            core::mem::transmute(create);
        if cc(&CLSID_SHELLLINK, 0, CLSCTX_INPROC, &IID_SHELLLINKW, &mut sl) < 0 || sl == 0 {
            return false;
        }
        let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, VT_SL_SETPATH));
        set1(sl, w_target.as_ptr());
        let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, VT_SL_SETWDIR));
        set1(sl, w_dir.as_ptr());
        let icon: unsafe extern "system" fn(usize, *const u16, i32) -> i32 = core::mem::transmute(vfn(sl, VT_SL_SETICON));
        icon(sl, w_target.as_ptr(), 0);
        let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, VT_SL_SETDESC));
        set1(sl, w_desc.as_ptr());
        let qi: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(sl, VT_QI));
        if qi(sl, &IID_PERSISTFILE, &mut pf) < 0 || pf == 0 {
            release(sl);
            return false;
        }
        let save: unsafe extern "system" fn(usize, *const u16, i32) -> i32 = core::mem::transmute(vfn(pf, VT_PF_SAVE));
        let hr = save(pf, w_lnk.as_ptr(), 1);
        release(pf);
        release(sl);
        hr >= 0
    }
}

/// an open registry key (closed when dropped)
pub struct RegKey(isize);

/// HKCU\subkey opened for writing, created when missing  [sh_reg_begin, real mode]
pub fn reg_create(subkey: &str) -> Option<RegKey> {
    let p = proc_(P_REGCREATE);
    if p == 0 {
        return None;
    }
    let w_key = wide_cap(subkey, PATH_CAP);
    let mut hkey: isize = 0;
    // SAFETY: RegCreateKeyExW(HKEY, LPCWSTR, DWORD, LPWSTR, DWORD, REGSAM,
    // LPSECURITY_ATTRIBUTES, PHKEY, LPDWORD) -> LSTATUS; the disposition is not needed.
    let f: unsafe extern "system" fn(isize, *const u16, u32, usize, u32, u32, usize, *mut isize, usize) -> i32 =
        unsafe { core::mem::transmute(p) };
    if unsafe { f(HKCU, w_key.as_ptr(), 0, 0, 0, KEY_WRITE, 0, &mut hkey, 0) } != 0 || hkey == 0 {
        return None;
    }
    Some(RegKey(hkey))
}

impl RegKey {
    fn set(&self, name: &str, typ: u32, data: *const u8, n: u32) {
        let p = proc_(P_REGSET);
        if p == 0 {
            return;
        }
        let w_name = wide_cap(name, NAME_CAP);
        // SAFETY: RegSetValueExW(HKEY, LPCWSTR, DWORD, DWORD, const BYTE*, DWORD) -> LSTATUS
        // on our open key; data holds n bytes.
        let f: unsafe extern "system" fn(isize, *const u16, u32, u32, *const u8, u32) -> i32 =
            unsafe { core::mem::transmute(p) };
        unsafe { f(self.0, w_name.as_ptr(), 0, typ, data, n) };
    }

    /// a REG_SZ value (UTF-8 in, UTF-16 stored, PATH_CAP wide chars)  [sh_reg_sz]
    pub fn set_sz(&self, name: &str, value: &str) {
        let w: Vec<u16> = wide_cap(value, PATH_CAP);
        self.set(name, REG_SZ, w.as_ptr() as *const u8, (w.len() * 2) as u32); // incl. the NUL
    }

    /// a REG_DWORD value  [sh_reg_dword]
    pub fn set_dword(&self, name: &str, value: u32) {
        let v = value;
        self.set(name, REG_DWORD, &v as *const u32 as *const u8, 4);
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        let p = proc_(P_REGCLOSE);
        if p != 0 {
            // SAFETY: RegCloseKey(HKEY) on the key this RegKey owns.
            let f: unsafe extern "system" fn(isize) -> i32 = unsafe { core::mem::transmute(p) };
            unsafe { f(self.0) };
        }
    }
}

/// RegDeleteKeyW(HKCU, subkey)  [sh_reg_delete, real mode]
pub fn reg_delete(subkey: &str) -> bool {
    let p = proc_(P_REGDELETE);
    if p == 0 {
        return false;
    }
    let w_key = wide_cap(subkey, PATH_CAP);
    // SAFETY: RegDeleteKeyW(HKEY, LPCWSTR) -> LSTATUS.
    let f: unsafe extern "system" fn(isize, *const u16) -> i32 = unsafe { core::mem::transmute(p) };
    unsafe { f(HKCU, w_key.as_ptr()) == 0 }
}

/// HKCU\subkey with all its keys and values  (RegDeleteTreeW, then RegDeleteKeyW)
pub fn reg_delete_tree(subkey: &str) -> bool {
    let p = proc_(P_REGTREE);
    if p == 0 {
        return false;
    }
    let w_key = wide_cap(subkey, PATH_CAP);
    // SAFETY: RegDeleteTreeW(HKEY, LPCWSTR) -> LSTATUS: the key with everything below it.
    let f: unsafe extern "system" fn(isize, *const u16) -> i32 = unsafe { core::mem::transmute(p) };
    let ok = unsafe { f(HKCU, w_key.as_ptr()) } == 0;
    ok | reg_delete(subkey) // older systems leave the emptied key itself
}

/// one value of HKCU\subkey ("" = its default value)  (RegDeleteKeyValueW)
pub fn reg_delete_value(subkey: &str, name: &str) -> bool {
    let p = proc_(P_REGDELVAL);
    if p == 0 {
        return false;
    }
    let (w_key, w_name) = (wide_cap(subkey, PATH_CAP), wide_cap(name, NAME_CAP));
    // SAFETY: RegDeleteKeyValueW(HKEY, LPCWSTR, LPCWSTR) -> LSTATUS.
    let f: unsafe extern "system" fn(isize, *const u16, *const u16) -> i32 = unsafe { core::mem::transmute(p) };
    unsafe { f(HKCU, w_key.as_ptr(), w_name.as_ptr()) == 0 }
}

/// tell the shell that file types or URL schemes changed (SHCNE_ASSOCCHANGED, SHCNF_IDLIST)
pub fn assoc_changed() {
    let p = proc_(P_CHANGENOTIFY);
    if p != 0 {
        // SAFETY: SHChangeNotify(LONG, UINT, LPCVOID, LPCVOID) with no items.
        let f: unsafe extern "system" fn(i32, u32, usize, usize) = unsafe { core::mem::transmute(p) };
        unsafe { f(0x0800_0000, 0, 0, 0) };
    }
}

/// start a command line (UTF-8, PATH_CAP wide chars) in dir (None = the current folder);
/// hidden = CREATE_NO_WINDOW  [sh_run]
pub fn run_cmdline(cmdline: &str, dir: Option<&str>, hidden: bool) -> bool {
    let mut w_val = wide_cap(cmdline, PATH_CAP); // writable, as CreateProcessW wants
    let w_dir = dir.map(|d| wide_cap(d, PATH_CAP));
    let pdir = w_dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr());
    let si = StartupInfo { cb: 104, rest: [0; 25] };
    let mut pi = [0isize; 3]; // hProcess, hThread, ids
    let flags = if hidden { CREATE_NO_WINDOW } else { 0 };
    // SAFETY: application from the command line (a writable NUL-terminated buffer), a valid
    // STARTUPINFOW and PROCESS_INFORMATION (24 bytes); our environment.
    let ok = unsafe { CreateProcessW(core::ptr::null(), w_val.as_mut_ptr(), 0, 0, 0, flags, 0, pdir, &si, &mut pi) };
    if ok == 0 {
        return false;
    }
    // SAFETY: the handles of the new process.
    unsafe {
        CloseHandle(pi[0]);
        CloseHandle(pi[1]);
    }
    true
}
