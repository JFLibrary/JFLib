// win_recent -- (recent.rs) the recent files of the shell and the app's jump list
//
//   add(path)         SHAddToRecentDocs with COM set up: SHARD_APPIDINFO (an IShellItem of the
//                     path and the process's explicit AppUserModelID, taskbar.rs) when it has
//                     one, else SHARD_PATHW: Explorer's recent files (the Recent folder, Quick
//                     access / Home, Start's recommendations) and, when Windows counts the app
//                     as a handler of the type, the "Recent" category of its jump list
//   list(max)         IApplicationDocumentLists::GetList(ADLT_RECENT): the app's jump list (the
//                     explicit id through SetAppID, else the one Windows derives from the
//                     program), IShellItem (its file system path) or IShellLinkW (its target)
//                     per entry
//   remove(path)      IApplicationDestinations::RemoveDestination(an IShellItem of the path);
//                     a path that is not in the list counts as removed
//   clear()           IApplicationDestinations::RemoveAllDestinations
//   list_of(id, max) / remove_of(id, path)   the same for another id (tests: Explorer's own
//                     list is Microsoft.Windows.Explorer)
//
// Explorer's own lists keep the file (the user removes it there).  Which app's jump list gets
// the file is Windows' decision (GUIDE.md section 21 has what was found on Windows 11 26200).
// The app's own list (ui recent.rs) does not depend on it.  COM objects are used through their
// vtable slots (as win_taskbar.rs); COM is set up on the calling thread for the call
// (CoInitializeEx; an MTA thread stays one).

use alloc::string::String;
use alloc::vec::Vec;

use super::Target;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: usize, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const Guid, outer: usize, ctx: u32, iid: *const Guid, out: *mut usize) -> i32;
    fn CoTaskMemFree(p: *mut u16);
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHAddToRecentDocs(flags: u32, pv: *const u16);
    fn SHCreateItemFromParsingName(path: *const u16, bc: usize, iid: *const Guid, out: *mut usize) -> i32;
}

/// a COM GUID
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

static CLSID_APPDOCLISTS: Guid = Guid(0x86BE_C222, 0x30F2, 0x47E0, [0x9F, 0x25, 0x60, 0xD1, 0x1C, 0xD7, 0x5C, 0x28]);
static IID_APPDOCLISTS: Guid = Guid(0x3C59_4F9F, 0x9F30, 0x47A1, [0x97, 0x9A, 0xC9, 0xE8, 0x3D, 0x3D, 0x0A, 0x06]);
static CLSID_APPDESTINATIONS: Guid = Guid(0x86C1_4003, 0x4D6B, 0x4EF3, [0xA7, 0xB4, 0x05, 0x06, 0x66, 0x3B, 0x2E, 0x68]);
static IID_APPDESTINATIONS: Guid = Guid(0x1233_7D35, 0x94C6, 0x48A0, [0xBC, 0xE7, 0x6A, 0x9C, 0x69, 0xD4, 0xD6, 0x00]);
static IID_OBJECTARRAY: Guid = Guid(0x92CA_9DCD, 0x5622, 0x4BBA, [0xA8, 0x05, 0x5E, 0x9F, 0x54, 0x1B, 0xD8, 0xC9]);
static IID_SHELLITEM: Guid = Guid(0x4382_6D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);
static IID_SHELLLINKW: Guid = Guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

const CLSCTX_INPROC_SERVER: u32 = 1;
const COINIT_APARTMENTTHREADED: u32 = 2;
const SHARD_PATHW: u32 = 3;
const SHARD_APPIDINFO: u32 = 4;
// HRESULT_FROM_WIN32(ERROR_NOT_FOUND)
const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;
const ADLT_RECENT: i32 = 0;
const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
const MAX_PATH: usize = 260;
// vtable slots
const VT_QI: usize = 0;
const VT_RELEASE: usize = 2;
// SetAppID of IApplicationDocumentLists and IApplicationDestinations
const VT_SETAPPID: usize = 3;
const ADL_GETLIST: usize = 4;
const AD_REMOVEDESTINATION: usize = 4;
const AD_REMOVEALL: usize = 5;
const OA_GETCOUNT: usize = 3;
const OA_GETAT: usize = 4;
const SI_GETDISPLAYNAME: usize = 5;
const SL_GETPATH: usize = 3;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().filter(|&c| c != 0).chain(core::iter::once(0)).collect()
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

/// IUnknown::QueryInterface (0: not supported)
///
/// SAFETY: obj is a live COM interface pointer.
unsafe fn query(obj: usize, iid: &Guid) -> usize {
    let mut p = 0usize;
    unsafe {
        let qi: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(obj, VT_QI));
        if qi(obj, iid, &mut p) < 0 { 0 } else { p }
    }
}

/// the explicit id of the process (taskbar.rs) or of `of` (tests), NUL-terminated
fn app_id(of: Option<&str>) -> Option<Vec<u16>> {
    of.map(String::from).or_else(crate::taskbar::app_user_model_id).map(|i| wide(&i))
}

/// f with COM set up on this thread and a new object of clsid (0: none could be made); an
/// IApplicationDocumentLists / IApplicationDestinations gets the explicit id first (SetAppID)
fn with_object<R>(clsid: &Guid, iid: &Guid, id: Option<&[u16]>, f: impl FnOnce(usize) -> R) -> R {
    // SAFETY: CoInitializeEx(NULL, STA); S_OK / S_FALSE are paired with CoUninitialize,
    // RPC_E_CHANGED_MODE (an MTA thread) leaves COM as it is.
    let hr = unsafe { CoInitializeEx(0, COINIT_APARTMENTTHREADED) };
    let mut p = 0usize;
    // SAFETY: valid GUIDs and an out-pointer.
    if unsafe { CoCreateInstance(clsid, 0, CLSCTX_INPROC_SERVER, iid, &mut p) } < 0 {
        p = 0;
    }
    if let (Some(id), true) = (id, p != 0) {
        // SAFETY: SetAppID(LPCWSTR) in slot 3 of both interfaces; a NUL-terminated id.
        unsafe {
            let set: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(p, VT_SETAPPID));
            set(p, id.as_ptr());
        }
    }
    let r = f(p);
    if p != 0 {
        // SAFETY: the object made above.
        unsafe { release(p) };
    }
    if hr >= 0 {
        // SAFETY: pairs the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    r
}

/// the list's place: the shell's
pub fn target() -> Target {
    Target::Native
}

/// SHARDAPPIDINFO: the item and the AppUserModelID it belongs to
#[repr(C)]
struct AppIdInfo {
    item: usize,
    id: *const u16,
}

pub fn add(path: &str) -> bool {
    let w = wide(&path.replace('/', "\\"));
    let id = app_id(None);
    // SAFETY: COM set up for the call (the jump list part of SHAddToRecentDocs needs it; the
    // link in the recent folder is made without); SHARD_APPIDINFO with an IShellItem of the
    // path (released after the call) and a NUL-terminated id, else SHARD_PATHW with the path.
    unsafe {
        let hr = CoInitializeEx(0, COINIT_APARTMENTTHREADED);
        let mut item = 0usize;
        match &id {
            Some(i) if SHCreateItemFromParsingName(w.as_ptr(), 0, &IID_SHELLITEM, &mut item) >= 0 && item != 0 => {
                let info = AppIdInfo { item, id: i.as_ptr() };
                SHAddToRecentDocs(SHARD_APPIDINFO, &info as *const AppIdInfo as *const u16);
                release(item);
            }
            _ => SHAddToRecentDocs(SHARD_PATHW, w.as_ptr()),
        }
        if hr >= 0 {
            CoUninitialize();
        }
    }
    true
}

/// the file system path of an entry of the list (IShellItem or IShellLinkW)
///
/// SAFETY: obj is a live IUnknown of the object array.
unsafe fn entry_path(obj: usize) -> Option<String> {
    unsafe {
        let item = query(obj, &IID_SHELLITEM);
        if item != 0 {
            let name: unsafe extern "system" fn(usize, u32, *mut *mut u16) -> i32 = core::mem::transmute(vfn(item, SI_GETDISPLAYNAME));
            let mut p: *mut u16 = core::ptr::null_mut();
            let hr = name(item, SIGDN_FILESYSPATH, &mut p);
            release(item);
            if hr < 0 || p.is_null() {
                return None;
            }
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
            CoTaskMemFree(p);
            return Some(s.replace('\\', "/"));
        }
        let link = query(obj, &IID_SHELLLINKW);
        if link == 0 {
            return None;
        }
        let get: unsafe extern "system" fn(usize, *mut u16, i32, usize, u32) -> i32 = core::mem::transmute(vfn(link, SL_GETPATH));
        let mut buf = [0u16; MAX_PATH * 4];
        let hr = get(link, buf.as_mut_ptr(), buf.len() as i32, 0, 0);
        release(link);
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        (hr == 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n]).replace('\\', "/"))
    }
}

pub fn list(max: usize) -> Vec<String> {
    list_in(None, max)
}

pub fn list_of(id: &str, max: usize) -> Vec<String> {
    list_in(Some(id), max)
}

fn list_in(of: Option<&str>, max: usize) -> Vec<String> {
    let id = app_id(of);
    with_object(&CLSID_APPDOCLISTS, &IID_APPDOCLISTS, id.as_deref(), |adl| {
        let mut out = Vec::new();
        if adl == 0 {
            return out;
        }
        // SAFETY: IApplicationDocumentLists::GetList and IObjectArray through their documented
        // slots; every object returned is released.
        unsafe {
            let get: unsafe extern "system" fn(usize, i32, u32, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(adl, ADL_GETLIST));
            let mut arr = 0usize;
            if get(adl, ADLT_RECENT, max.min(u32::MAX as usize) as u32, &IID_OBJECTARRAY, &mut arr) < 0 || arr == 0 {
                return out;
            }
            let count: unsafe extern "system" fn(usize, *mut u32) -> i32 = core::mem::transmute(vfn(arr, OA_GETCOUNT));
            let at: unsafe extern "system" fn(usize, u32, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(arr, OA_GETAT));
            let mut n = 0u32;
            if count(arr, &mut n) >= 0 {
                for i in 0..n {
                    let mut obj = 0usize;
                    if at(arr, i, &IID_SHELLITEM, &mut obj) < 0 || obj == 0 {
                        // a link: asked for again as itself
                        if at(arr, i, &IID_SHELLLINKW, &mut obj) < 0 || obj == 0 {
                            continue;
                        }
                    }
                    if let Some(p) = entry_path(obj) {
                        out.push(p);
                    }
                    release(obj);
                }
            }
            release(arr);
        }
        out
    })
}

pub fn remove(path: &str) -> bool {
    remove_in(None, path)
}

pub fn remove_of(id: &str, path: &str) -> bool {
    remove_in(Some(id), path)
}

fn remove_in(of: Option<&str>, path: &str) -> bool {
    let w = wide(&path.replace('/', "\\"));
    let id = app_id(of);
    with_object(&CLSID_APPDESTINATIONS, &IID_APPDESTINATIONS, id.as_deref(), |ad| {
        if ad == 0 {
            return false;
        }
        // SAFETY: an IShellItem of the path, IApplicationDestinations::RemoveDestination through
        // its documented slot; the item is released.
        unsafe {
            let mut item = 0usize;
            if SHCreateItemFromParsingName(w.as_ptr(), 0, &IID_SHELLITEM, &mut item) < 0 || item == 0 {
                return false;
            }
            let rm: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(vfn(ad, AD_REMOVEDESTINATION));
            let hr = rm(ad, item);
            release(item);
            hr >= 0 || hr == E_NOT_FOUND // not in the list: nothing to do
        }
    })
}

pub fn clear() -> bool {
    let id = app_id(None);
    with_object(&CLSID_APPDESTINATIONS, &IID_APPDESTINATIONS, id.as_deref(), |ad| {
        if ad == 0 {
            return false;
        }
        // SAFETY: IApplicationDestinations::RemoveAllDestinations through its documented slot.
        unsafe {
            let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(ad, AD_REMOVEALL));
            f(ad) >= 0
        }
    })
}
