// win_taskbar -- (taskbar.rs) the taskbar button on Windows: ITaskbarList3 and the jump list
//
//   progress(&State) / badge(&State)   ITaskbarList3 (made and HrInit-ed once, on the calling
//                     thread -- the UI thread) on the main window: SetProgressState
//                     (TBPF_NOPROGRESS / NORMAL / PAUSED / ERROR / INDETERMINATE) +
//                     SetProgressValue (per mille); SetOverlayIcon with an HICON made from the
//                     state's icon (destroyed right after: the taskbar keeps a copy) and the
//                     count as its text.  Without a main window the state waits for the button.
//   window_message(msg)  "TaskbarButtonCreated" (registered on first use; allowed through
//                     UIPI for elevated processes): both applied again
//   tasks(&[JumpTask])   ICustomDestinationList: BeginList, the Recent category
//                     (AppendKnownCategory KDC_RECENT: recent.rs), an EnumerableObjectCollection of
//                     IShellLinkW (this exe, the quoted arguments, its icon 0, the tooltip; the
//                     title as PKEY_Title through IPropertyStore), AddUserTasks, CommitList;
//                     empty: DeleteList (the jump list file goes away)
//   supported()       ITaskbarList3 can be made
//   set_app_id(id) / app_id()   SetCurrentProcessExplicitAppUserModelID (not in an MSIX
//                     package: its id counts) / GetCurrentProcessExplicitAppUserModelID; the
//                     jump list is then the id's (ICustomDestinationList SetAppID, DeleteList(id))
//
// COM objects are used through their vtable slots (as win_inst.rs, win_uia.rs); every object
// made here is released.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};

use super::{JumpTask, State, TaskbarProgress};

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: usize, flags: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: usize, ctx: u32, iid: *const Guid, out: *mut usize) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn DestroyIcon(h: isize) -> i32;
    fn ChangeWindowMessageFilterEx(hwnd: isize, msg: u32, action: u32, info: usize) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
    fn GetCurrentProcessExplicitAppUserModelID(id: *mut *mut u16) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemFree(p: *mut u16);
}

/// a COM GUID
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

/// PROPERTYKEY
#[repr(C)]
struct PropKey {
    fmtid: Guid,
    pid: u32,
}

/// PROPVARIANT (x64, 24 bytes) holding a VT_LPWSTR
#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    ptr: *const u16,
    pad: usize,
}

static CLSID_TASKBARLIST: Guid = Guid(0x56FD_F344, 0xFD6D, 0x11D0, [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90]);
static IID_ITASKBARLIST3: Guid = Guid(0xEA1A_FB91, 0x9E28, 0x4B86, [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF]);
static CLSID_DESTINATIONLIST: Guid = Guid(0x77F1_0CF0, 0x3DB5, 0x4966, [0xB5, 0x20, 0xB7, 0xC5, 0x4F, 0xD3, 0x5E, 0xD6]);
static IID_ICUSTOMDESTINATIONLIST: Guid = Guid(0x6332_DEBF, 0x87B5, 0x4670, [0x90, 0xC0, 0x5E, 0x57, 0xB4, 0x08, 0xA4, 0x9E]);
static CLSID_ENUMERABLEOBJECTCOLLECTION: Guid = Guid(0x2D34_68C1, 0x36A7, 0x43B6, [0xAC, 0x24, 0xD3, 0xF0, 0x2F, 0xD9, 0x60, 0x7A]);
static IID_IOBJECTCOLLECTION: Guid = Guid(0x5632_B1A4, 0xE38A, 0x400A, [0x92, 0x8A, 0xD4, 0xCD, 0x63, 0x23, 0x02, 0x95]);
static IID_IOBJECTARRAY: Guid = Guid(0x92CA_9DCD, 0x5622, 0x4BBA, [0xA8, 0x05, 0x5E, 0x9F, 0x54, 0x1B, 0xD8, 0xC9]);
static CLSID_SHELLLINK: Guid = Guid(0x0002_1401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_ISHELLLINKW: Guid = Guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_IPROPERTYSTORE: Guid = Guid(0x886D_8EEB, 0x8CF2, 0x4446, [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99]);
static PKEY_TITLE: PropKey = PropKey { fmtid: Guid(0xF29F_85E0, 0x4FF9, 0x1068, [0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9]), pid: 2 };

const CLSCTX_INPROC_SERVER: u32 = 1;
const COINIT_APARTMENTTHREADED: u32 = 2;
const VT_LPWSTR: u16 = 31;
const MSGFLT_ALLOW: u32 = 1;
const TBPF_NOPROGRESS: u32 = 0;
const TBPF_INDETERMINATE: u32 = 1;
const TBPF_NORMAL: u32 = 2;
const TBPF_ERROR: u32 = 4;
const TBPF_PAUSED: u32 = 8;
const TASKS_MAX: usize = 32;

// vtable slots
const VT_QI: usize = 0;
const VT_RELEASE: usize = 2;
const TB_HRINIT: usize = 3;
const TB_SETPROGRESSVALUE: usize = 9;
const TB_SETPROGRESSSTATE: usize = 10;
const TB_SETOVERLAYICON: usize = 18;
const DL_SETAPPID: usize = 3;
const DL_BEGINLIST: usize = 4;
const DL_APPENDKNOWNCATEGORY: usize = 6;
const KDC_RECENT: i32 = 2;
const DL_ADDUSERTASKS: usize = 7;
const DL_COMMITLIST: usize = 8;
const DL_DELETELIST: usize = 10;
const DL_ABORTLIST: usize = 11;
const OC_ADDOBJECT: usize = 5;
const SL_SETDESCRIPTION: usize = 7;
const SL_SETARGUMENTS: usize = 11;
const SL_SETICONLOCATION: usize = 17;
const SL_SETPATH: usize = 20;
const PS_SETVALUE: usize = 6;
const PS_COMMIT: usize = 7;

// the ITaskbarList3 (0: not made yet, 1: not available), the registered message, the thread
// COM was set up on (id + 1) and the window UIPI lets the message through to
static TBL: AtomicUsize = AtomicUsize::new(0);
static BUTTON_MSG: AtomicU32 = AtomicU32::new(0);
static COM_THREAD: AtomicU32 = AtomicU32::new(0);
static FILTERED: AtomicIsize = AtomicIsize::new(0);

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

/// COM on this thread (once per thread that comes here; an MTA thread stays one)
fn com_init() {
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentThreadId() }.wrapping_add(1);
    if COM_THREAD.swap(me, Ordering::SeqCst) != me {
        // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED); S_FALSE and
        // RPC_E_CHANGED_MODE leave COM usable.
        unsafe { CoInitializeEx(0, COINIT_APARTMENTTHREADED) };
    }
}

/// a new in-process COM object (0: failed)
fn create(clsid: &Guid, iid: &Guid) -> usize {
    com_init();
    let mut p = 0usize;
    // SAFETY: CoCreateInstance with valid GUIDs and an out-pointer.
    let hr = unsafe { CoCreateInstance(clsid, 0, CLSCTX_INPROC_SERVER, iid, &mut p) };
    if hr < 0 { 0 } else { p }
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

// the registered message (once)
fn button_msg() -> u32 {
    let m = BUTTON_MSG.load(Ordering::SeqCst);
    if m != 0 {
        return m;
    }
    let name = wide("TaskbarButtonCreated");
    // SAFETY: a NUL-terminated name.
    let m = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    BUTTON_MSG.store(m, Ordering::SeqCst);
    m
}

// the ITaskbarList3 (0: not available)
fn taskbar() -> usize {
    button_msg();
    match TBL.load(Ordering::SeqCst) {
        0 => {}
        1 => return 0,
        p => return p,
    }
    let p = create(&CLSID_TASKBARLIST, &IID_ITASKBARLIST3);
    // SAFETY: HrInit() on the object just made.
    let ok = p != 0 && unsafe {
        let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(p, TB_HRINIT));
        f(p) >= 0
    };
    if !ok {
        if p != 0 {
            // SAFETY: the object made above.
            unsafe { release(p) };
        }
        TBL.store(1, Ordering::SeqCst);
        return 0;
    }
    TBL.store(p, Ordering::SeqCst);
    p
}

// an elevated process: let Explorer's message through to the main window (once per window)
fn allow_message(hwnd: isize) {
    if FILTERED.swap(hwnd, Ordering::SeqCst) == hwnd {
        return;
    }
    // SAFETY: our main window and the registered message; no CHANGEFILTERSTRUCT.
    unsafe { ChangeWindowMessageFilterEx(hwnd, button_msg(), MSGFLT_ALLOW, 0) };
}

// the main window and the taskbar object, or None (no window yet: Some((0, tb)))
fn target() -> Option<(isize, usize)> {
    let tb = taskbar();
    if tb == 0 {
        return None;
    }
    let hwnd = crate::win::hwnd_main();
    if hwnd != 0 {
        allow_message(hwnd);
    }
    Some((hwnd, tb))
}

fn apply_progress(tb: usize, hwnd: isize, p: TaskbarProgress) {
    let state = match p {
        TaskbarProgress::None => TBPF_NOPROGRESS,
        TaskbarProgress::Normal(_) => TBPF_NORMAL,
        TaskbarProgress::Paused(_) => TBPF_PAUSED,
        TaskbarProgress::Error(_) => TBPF_ERROR,
        TaskbarProgress::Indeterminate => TBPF_INDETERMINATE,
    };
    // SAFETY: SetProgressState(HWND, TBPFLAG) / SetProgressValue(HWND, ULONGLONG, ULONGLONG)
    // on the live ITaskbarList3.
    unsafe {
        let st: unsafe extern "system" fn(usize, isize, u32) -> i32 = core::mem::transmute(vfn(tb, TB_SETPROGRESSSTATE));
        st(tb, hwnd, state);
        if matches!(p, TaskbarProgress::Normal(_) | TaskbarProgress::Paused(_) | TaskbarProgress::Error(_)) {
            let val: unsafe extern "system" fn(usize, isize, u64, u64) -> i32 = core::mem::transmute(vfn(tb, TB_SETPROGRESSVALUE));
            val(tb, hwnd, (p.value() * 1000.0 + 0.5) as u64, 1000);
        }
    }
}

fn apply_badge(tb: usize, hwnd: isize, s: &State) {
    let icon = match &s.icon {
        Some(i) if s.count != 0 => crate::win_tray::make_icon(i),
        _ => 0,
    };
    let text = wide(&if icon != 0 { alloc::format!("{}", s.count) } else { String::new() });
    // SAFETY: SetOverlayIcon(HWND, HICON, LPCWSTR) on the live ITaskbarList3; the taskbar copies
    // the icon, which is destroyed afterwards.
    unsafe {
        let f: unsafe extern "system" fn(usize, isize, isize, *const u16) -> i32 = core::mem::transmute(vfn(tb, TB_SETOVERLAYICON));
        f(tb, hwnd, icon, if icon != 0 { text.as_ptr() } else { core::ptr::null() });
        if icon != 0 {
            DestroyIcon(icon);
        }
    }
}

pub fn progress(s: &State) -> bool {
    let Some((hwnd, tb)) = target() else { return false };
    if hwnd != 0 {
        apply_progress(tb, hwnd, s.progress);
    }
    true
}

pub fn badge(s: &State) -> bool {
    let Some((hwnd, tb)) = target() else { return false };
    if hwnd != 0 {
        apply_badge(tb, hwnd, s);
    }
    true
}

pub fn supported() -> bool {
    taskbar() != 0
}

pub fn set_app_id(id: &str) -> bool {
    if crate::autostart::native::packaged() {
        return false;
    }
    let w = wide(id);
    // SAFETY: a NUL-terminated id; the shell keeps its own copy.
    unsafe { SetCurrentProcessExplicitAppUserModelID(w.as_ptr()) >= 0 }
}

pub fn app_id() -> Option<String> {
    let mut p: *mut u16 = core::ptr::null_mut();
    // SAFETY: the shell allocates the NUL-terminated id (CoTaskMemAlloc), freed here; without an
    // explicit id it fails and leaves p null.
    unsafe {
        if GetCurrentProcessExplicitAppUserModelID(&mut p) < 0 || p.is_null() {
            return None;
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
        CoTaskMemFree(p);
        Some(s)
    }
}

pub fn window_message(msg: u32) {
    let m = BUTTON_MSG.load(Ordering::SeqCst);
    if m == 0 || msg != m {
        return;
    }
    if let Some((hwnd, tb)) = target().filter(|t| t.0 != 0) {
        let s = super::state();
        apply_progress(tb, hwnd, s.progress);
        apply_badge(tb, hwnd, &s);
    }
}

// ---------------------------------------------------------------- the jump list

// an IShellLinkW for one task (0: failed)
fn task_link(exe: &[u16], t: &JumpTask) -> usize {
    let sl = create(&CLSID_SHELLLINK, &IID_ISHELLLINKW);
    if sl == 0 {
        return 0;
    }
    let mut args = String::new();
    for (k, a) in t.args.iter().enumerate() {
        if k > 0 {
            args.push(' ');
        }
        crate::autostart::native::quote_arg(a, &mut args);
    }
    let title = if t.title.is_empty() { args.clone() } else { t.title.clone() };
    let (w_args, w_desc, w_title) = (wide(&args), wide(&t.desc), wide(&title));
    // SAFETY: the IShellLinkW / IPropertyStore methods through their documented slots with
    // NUL-terminated strings that outlive the calls; the property store is released.
    unsafe {
        let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, SL_SETPATH));
        let mut ok = set1(sl, exe.as_ptr()) >= 0;
        let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, SL_SETARGUMENTS));
        ok &= set1(sl, w_args.as_ptr()) >= 0;
        let icon: unsafe extern "system" fn(usize, *const u16, i32) -> i32 = core::mem::transmute(vfn(sl, SL_SETICONLOCATION));
        icon(sl, exe.as_ptr(), 0);
        if !t.desc.is_empty() {
            let set1: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(sl, SL_SETDESCRIPTION));
            set1(sl, w_desc.as_ptr());
        }
        let ps = query(sl, &IID_IPROPERTYSTORE);
        if ps == 0 {
            ok = false;
        } else {
            let pv = PropVariant { vt: VT_LPWSTR, reserved: [0; 3], ptr: w_title.as_ptr(), pad: 0 };
            let sv: unsafe extern "system" fn(usize, *const PropKey, *const PropVariant) -> i32 = core::mem::transmute(vfn(ps, PS_SETVALUE));
            ok &= sv(ps, &PKEY_TITLE, &pv) >= 0;
            let commit: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(ps, PS_COMMIT));
            ok &= commit(ps) >= 0;
            release(ps);
        }
        if !ok {
            release(sl);
            return 0;
        }
    }
    sl
}

pub fn tasks(tasks: &[JumpTask]) -> bool {
    let dl = create(&CLSID_DESTINATIONLIST, &IID_ICUSTOMDESTINATIONLIST);
    if dl == 0 {
        return false;
    }
    // the process's explicit id (else Windows' own for the program)
    let id = super::app_user_model_id().map(|i| wide(&i));
    let pid = id.as_ref().map_or(core::ptr::null(), |w| w.as_ptr());
    // SAFETY: the ICustomDestinationList / IObjectCollection methods through their documented
    // slots; every object made or returned here is released.
    unsafe {
        if !pid.is_null() {
            let set: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(dl, DL_SETAPPID));
            set(dl, pid);
        }
        if tasks.is_empty() {
            let del: unsafe extern "system" fn(usize, *const u16) -> i32 = core::mem::transmute(vfn(dl, DL_DELETELIST));
            let hr = del(dl, pid);
            release(dl);
            return hr >= 0;
        }
        let Some(exe) = crate::current_exe() else {
            release(dl);
            return false;
        };
        let w_exe = wide(&exe.replace('/', "\\"));
        let (mut slots, mut removed) = (0u32, 0usize);
        let begin: unsafe extern "system" fn(usize, *mut u32, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(dl, DL_BEGINLIST));
        if begin(dl, &mut slots, &IID_IOBJECTARRAY, &mut removed) < 0 {
            release(dl);
            return false;
        }
        if removed != 0 {
            release(removed);
        }
        let abort = |dl: usize| {
            let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(dl, DL_ABORTLIST));
            f(dl);
            release(dl);
        };
        let coll = create(&CLSID_ENUMERABLEOBJECTCOLLECTION, &IID_IOBJECTCOLLECTION);
        if coll == 0 {
            abort(dl);
            return false;
        }
        let add: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(vfn(coll, OC_ADDOBJECT));
        let mut ok = true;
        for t in tasks.iter().take(TASKS_MAX) {
            let sl = task_link(&w_exe, t);
            ok &= sl != 0 && add(coll, sl) >= 0;
            if sl != 0 {
                release(sl);
            }
        }
        let arr = if ok { query(coll, &IID_IOBJECTARRAY) } else { 0 };
        release(coll);
        if arr == 0 {
            abort(dl);
            return false;
        }
        // the Recent category stays (a custom list shows only what it appends; refused when the
        // app is no registered handler of a file type -- then there is nothing to show)
        let known: unsafe extern "system" fn(usize, i32) -> i32 = core::mem::transmute(vfn(dl, DL_APPENDKNOWNCATEGORY));
        known(dl, KDC_RECENT);
        let user: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(vfn(dl, DL_ADDUSERTASKS));
        let hr = user(dl, arr);
        release(arr);
        if hr < 0 {
            abort(dl);
            return false;
        }
        let commit: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(dl, DL_COMMITLIST));
        let hr = commit(dl);
        release(dl);
        hr >= 0
    }
}
