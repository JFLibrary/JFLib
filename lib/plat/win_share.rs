// win_share -- (share.rs) Windows' share sheet: the DataTransferManager of a window through
// IDataTransferManagerInterop (Windows 10), raw COM as win_winrt.rs
//
//   share(&Share) -> SHARE_PENDING / SHARE_NONE   the active window of the UI thread (else the main
//                                window): DataRequested is registered, ShowShareUIForWindow opens
//                                the sheet; the files become StorageFiles first (on an MTA thread,
//                                GetFileFromPathAsync, at most 5 s each)
//   available() -> bool          the interop factory exists (Windows 10 and later)
//   win_share_dismiss()          (tests) the sheet in front gets Esc, the app's window comes back
//
// DataRequested (on the UI thread, while the sheet opens) fills the request's DataPackage: the title
// (the sheet requires one: the title, else the app's id), the text (SetText), the link (SetWebLink
// through Windows.Foundation.Uri), the files (SetStorageItems with an IIterable of our own), and
// registers ShareCompleted (IDataPackage3: SHARE_DONE) and ShareCanceled (IDataPackage4, Windows 10
// 2004: SHARE_CANCELLED).  The handlers are small COM objects of ours (IUnknown + Invoke, agile).
// A failure inside DataRequested answers SHARE_FAILED (the sheet then says it cannot share).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicIsize, AtomicU32, AtomicU8, Ordering};

use rt::sync::SpinLock;

use crate::share::{SHARE_CANCELLED, SHARE_DONE, SHARE_FAILED, SHARE_NONE, SHARE_PENDING, Share, answer};
use crate::win_winrt::{self, Guid, HStr, Obj};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetActiveWindow() -> isize;
    fn SetForegroundWindow(h: isize) -> i32;
    fn GetForegroundWindow() -> isize;
    fn GetClassNameW(h: isize, buf: *mut u16, n: i32) -> i32;
    fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
    fn PostMessageW(h: isize, m: u32, wp: usize, lp: isize) -> i32;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

const fn g(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    Guid(a, b, c, d)
}

static IID_IUNKNOWN: Guid = g(0x0000_0000, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_IINSPECTABLE: Guid = g(0xAF86_E2E0, 0xB12D, 0x4C6A, [0x9C, 0x5A, 0xD7, 0xAA, 0x65, 0x10, 0x1E, 0x90]);
static IID_IAGILEOBJECT: Guid = g(0x94EA_2B94, 0xE9CC, 0x49E0, [0xC0, 0xFF, 0xEE, 0x64, 0xCA, 0x8F, 0x5B, 0x90]);
static IID_INTEROP: Guid = g(0x3A3D_CD6C, 0x3EAB, 0x43DC, [0xBC, 0xDE, 0x45, 0x67, 0x1C, 0xE8, 0x00, 0xC8]);
static IID_DTM: Guid = g(0xA5CA_EE9B, 0x8708, 0x49D1, [0x8D, 0x36, 0x67, 0xD2, 0x5A, 0x8D, 0xA0, 0x0C]);
static IID_DATA_PACKAGE2: Guid = g(0x041C_1FE9, 0x2409, 0x45E1, [0xA5, 0x38, 0x4C, 0x53, 0xEE, 0xEE, 0x04, 0xA7]);
static IID_DATA_PACKAGE3: Guid = g(0x88F3_1F5D, 0x787B, 0x4D32, [0x96, 0x5A, 0xA9, 0x83, 0x81, 0x05, 0xA0, 0x56]);
static IID_DATA_PACKAGE4: Guid = g(0x13A2_4EC8, 0x9382, 0x536F, [0x85, 0x2A, 0x30, 0x45, 0xE1, 0xB2, 0x9A, 0x3B]);
static IID_URI_FACTORY: Guid = g(0x44A9_796F, 0x723E, 0x4FDF, [0xA2, 0x18, 0x03, 0x3E, 0x75, 0xB0, 0xC0, 0x84]);
static IID_STORAGE_FILE_STATICS: Guid = g(0x5984_C710, 0xDAF2, 0x43C8, [0x8B, 0xB4, 0xA4, 0xD3, 0xEA, 0xCF, 0xD0, 0x3F]);
static IID_STORAGE_ITEM: Guid = g(0x4207_A996, 0xCA2F, 0x42F7, [0xBD, 0xE8, 0x8B, 0x10, 0x45, 0x7A, 0x7F, 0x30]);
// TypedEventHandler<DataTransferManager, DataRequestedEventArgs>
static IID_ON_REQUESTED: Guid = g(0xEC6F_9CC8, 0x46D0, 0x5E0E, [0xB4, 0xD2, 0x7D, 0x77, 0x73, 0xAE, 0x37, 0xA0]);
// TypedEventHandler<DataPackage, ShareCompletedEventArgs>
static IID_ON_COMPLETED: Guid = g(0xF8F7_E24A, 0x56FE, 0x58DF, [0xBC, 0x15, 0x23, 0x65, 0xAE, 0xC0, 0x39, 0x66]);
// TypedEventHandler<DataPackage, IInspectable> (ShareCanceled)
static IID_ON_CANCELED: Guid = g(0xC156_B0C3, 0x1CBC, 0x5CA4, [0x90, 0x1C, 0x62, 0xC5, 0xA8, 0xCA, 0x5C, 0xB5]);
// IIterable<IStorageItem> / IIterator<IStorageItem>
static IID_ITERABLE_ITEMS: Guid = g(0xBB8B_8418, 0x65D1, 0x544B, [0xB0, 0x83, 0x6D, 0x17, 0x2F, 0x56, 0x8C, 0x73]);
static IID_ITERATOR_ITEMS: Guid = g(0x05B4_87C2, 0x3830, 0x5D3C, [0x98, 0xDA, 0x25, 0xFA, 0x11, 0x54, 0x2D, 0xBD]);

const S_OK: i32 = 0;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_POINTER: i32 = 0x8000_4003u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const E_BOUNDS: i32 = 0x8000_000Bu32 as i32;

// vtable slots (IInspectable takes 3..5)
const INTEROP_GET_FOR_WINDOW: usize = 3;
const INTEROP_SHOW: usize = 4;
const DTM_ADD_REQUESTED: usize = 6;
const DTM_REMOVE_REQUESTED: usize = 7;
const ARGS_GET_REQUEST: usize = 6;
const REQUEST_GET_DATA: usize = 6;
const PACKAGE_GET_PROPERTIES: usize = 7;
const PACKAGE_SET_TEXT: usize = 16;
const PACKAGE_SET_STORAGE_ITEMS: usize = 23;
const PACKAGE2_SET_WEB_LINK: usize = 7;
const PACKAGE3_ADD_COMPLETED: usize = 6;
const PACKAGE4_ADD_CANCELED: usize = 6;
const PROPS_PUT_TITLE: usize = 7;
const URI_CREATE: usize = 6;
const FILE_FROM_PATH: usize = 6;
const OP_GET_RESULTS: usize = 8;

fn same(a: &Guid, b: &Guid) -> bool {
    a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3
}

/// what the next DataRequested fills in (one sheet at a time)
struct Pending {
    title: String,
    text: String,
    url: String,
    /// IStorageItem pointers (one reference each)
    items: Vec<usize>,
    dtm: Obj,
    token: i64,
}

static PENDING: SpinLock<Option<Pending>> = SpinLock::new(None);
/// the window of the last sheet (win_share_dismiss: any thread, no UI state)
static LAST_HWND: AtomicIsize = AtomicIsize::new(0);
/// available(): 0 not asked yet, 1 no, 2 yes
static AVAILABLE: AtomicU8 = AtomicU8::new(0);

pub fn available() -> bool {
    // asked once, on an MTA thread: the caller's thread may have no COM apartment
    if AVAILABLE.load(Ordering::Acquire) == 0 {
        let ok = win_winrt::mta(|| win_winrt::factory("Windows.ApplicationModel.DataTransfer.DataTransferManager", &IID_INTEROP).is_ok()).unwrap_or(false);
        AVAILABLE.store(if ok { 2 } else { 1 }, Ordering::Release);
    }
    AVAILABLE.load(Ordering::Acquire) == 2
}

pub fn share(s: &Share) -> i32 {
    // SAFETY: the active window of this thread (0: none of ours is active).
    let active = unsafe { GetActiveWindow() };
    let hwnd = if active != 0 { active } else { crate::win::hwnd_main() };
    if hwnd == 0 {
        return SHARE_NONE;
    }
    LAST_HWND.store(hwnd, Ordering::Release);
    // the files as StorageFiles (an MTA thread: their asynchronous calls complete there)
    let items = if s.files.is_empty() {
        Vec::new()
    } else {
        let paths: Vec<String> = s.files.iter().map(|f| f.replace('/', "\\")).collect();
        match win_winrt::mta(move || storage_items(&paths)) {
            Some(Some(v)) => v,
            _ => return SHARE_NONE,
        }
    };
    let release_items = |items: &[usize]| {
        for &p in items {
            drop(Obj::own(p));
        }
    };
    let Ok(interop) = win_winrt::factory("Windows.ApplicationModel.DataTransfer.DataTransferManager", &IID_INTEROP) else {
        release_items(&items);
        return SHARE_NONE;
    };
    let mut dtm = 0usize;
    // SAFETY: IDataTransferManagerInterop::GetForWindow(this, HWND, REFIID, void**).
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, isize, *const Guid, *mut usize) -> i32 = core::mem::transmute(interop.vfn(INTEROP_GET_FOR_WINDOW));
        f(interop.ptr(), hwnd, &IID_DTM, &mut dtm)
    };
    let Some(dtm) = (if hr < 0 { None } else { Obj::own(dtm) }) else {
        release_items(&items);
        return SHARE_NONE;
    };
    let handler = Handler::new(&IID_ON_REQUESTED, on_requested);
    let mut token = 0i64;
    // SAFETY: IDataTransferManager::add_DataRequested(this, handler, EventRegistrationToken*); the
    // manager takes its own reference of the handler, ours goes after.
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, usize, *mut i64) -> i32 = core::mem::transmute(dtm.vfn(DTM_ADD_REQUESTED));
        let hr = f(dtm.ptr(), handler as usize, &mut token);
        Handler::release(handler);
        hr
    };
    if hr < 0 {
        release_items(&items);
        return SHARE_NONE;
    }
    let title = if s.title.is_empty() { crate::vfs::app_id() } else { s.title.clone() };
    *PENDING.lock() = Some(Pending { title, text: s.text.clone(), url: s.url.clone(), items, dtm, token });
    // SAFETY: IDataTransferManagerInterop::ShowShareUIForWindow(this, HWND).
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, isize) -> i32 = core::mem::transmute(interop.vfn(INTEROP_SHOW));
        f(interop.ptr(), hwnd)
    };
    if hr < 0 {
        if let Some(p) = PENDING.lock().take() {
            p.unregister();
        }
        return SHARE_NONE;
    }
    SHARE_PENDING
}

impl Pending {
    /// DataRequested off again; the items released
    fn unregister(self) {
        // SAFETY: IDataTransferManager::remove_DataRequested(this, token).
        unsafe {
            let f: unsafe extern "system" fn(usize, i64) -> i32 = core::mem::transmute(self.dtm.vfn(DTM_REMOVE_REQUESTED));
            f(self.dtm.ptr(), self.token);
        }
        for p in self.items {
            drop(Obj::own(p));
        }
    }
}

/// the paths as IStorageItems (one reference each) -> None when one of them is not a file Windows
/// gives (on an MTA thread)
fn storage_items(paths: &[String]) -> Option<Vec<usize>> {
    let statics = win_winrt::factory("Windows.Storage.StorageFile", &IID_STORAGE_FILE_STATICS).ok()?;
    let mut out: Vec<usize> = Vec::new();
    let fail = |out: Vec<usize>| {
        for p in out {
            drop(Obj::own(p));
        }
        None
    };
    for p in paths {
        let Some(h) = HStr::new(p) else { return fail(out) };
        let mut op = 0usize;
        // SAFETY: IStorageFileStatics::GetFileFromPathAsync(this, HSTRING, IAsyncOperation**).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, isize, *mut usize) -> i32 = core::mem::transmute(statics.vfn(FILE_FROM_PATH));
            f(statics.ptr(), h.raw(), &mut op)
        };
        let Some(op) = (if hr < 0 { None } else { Obj::own(op) }) else { return fail(out) };
        if win_winrt::wait(&op, 5000).is_err() {
            return fail(out);
        }
        let Ok(file) = op.get_obj(OP_GET_RESULTS) else { return fail(out) };
        let Some(item) = file.query(&IID_STORAGE_ITEM) else { return fail(out) };
        out.push(item.ptr());
        core::mem::forget(item); // the reference goes with the pointer
    }
    Some(out)
}

/// DataRequested: the package filled from PENDING (the UI thread)
fn on_requested(_sender: usize, args: usize) -> i32 {
    let Some(p) = PENDING.lock().take() else { return S_OK };
    let ok = fill(args, &p).is_ok();
    p.unregister();
    if !ok {
        answer(SHARE_FAILED);
    }
    S_OK
}

fn fill(args: usize, p: &Pending) -> Result<(), i32> {
    // the event's arguments are borrowed: an own reference for Obj
    // SAFETY: slot 1 of a COM object is AddRef(this).
    unsafe {
        let add_ref: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(*(*(args as *const *const usize)).add(1));
        add_ref(args);
    }
    let args = Obj::own(args).ok_or(E_POINTER)?;
    let request = args.get_obj(ARGS_GET_REQUEST)?;
    let package = request.get_obj(REQUEST_GET_DATA)?;
    let props = package.get_obj(PACKAGE_GET_PROPERTIES)?;
    let put_text = |obj: &Obj, slot: usize, text: &str| -> Result<(), i32> {
        let h = HStr::new(text).ok_or(E_POINTER)?;
        // SAFETY: a method of the form HRESULT (this, HSTRING).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, isize) -> i32 = core::mem::transmute(obj.vfn(slot));
            f(obj.ptr(), h.raw())
        };
        if hr < 0 { Err(hr) } else { Ok(()) }
    };
    put_text(&props, PROPS_PUT_TITLE, &p.title)?;
    if !p.text.is_empty() {
        put_text(&package, PACKAGE_SET_TEXT, &p.text)?;
    }
    if !p.url.is_empty() {
        let factory = win_winrt::factory("Windows.Foundation.Uri", &IID_URI_FACTORY)?;
        let h = HStr::new(&p.url).ok_or(E_POINTER)?;
        let mut uri = 0usize;
        // SAFETY: IUriRuntimeClassFactory::CreateUri(this, HSTRING, IUriRuntimeClass**).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, isize, *mut usize) -> i32 = core::mem::transmute(factory.vfn(URI_CREATE));
            f(factory.ptr(), h.raw(), &mut uri)
        };
        let uri = (if hr < 0 { None } else { Obj::own(uri) }).ok_or(if hr < 0 { hr } else { E_POINTER })?;
        let p2 = package.query(&IID_DATA_PACKAGE2).ok_or(E_NOINTERFACE)?;
        // SAFETY: IDataPackage2::SetWebLink(this, IUriRuntimeClass*).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(p2.vfn(PACKAGE2_SET_WEB_LINK));
            f(p2.ptr(), uri.ptr())
        };
        if hr < 0 {
            return Err(hr);
        }
    }
    if !p.items.is_empty() {
        let list = Items::new(&p.items, false);
        // SAFETY: IDataPackage::SetStorageItems(this, IIterable<IStorageItem>*, boolean readOnly); the
        // package takes its own reference, ours goes after.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, usize, u8) -> i32 = core::mem::transmute(package.vfn(PACKAGE_SET_STORAGE_ITEMS));
            let hr = f(package.ptr(), list as usize, 1);
            Items::release(list);
            hr
        };
        if hr < 0 {
            return Err(hr);
        }
    }
    // the answers: ShareCompleted (Windows 10 1709), ShareCanceled (2004)
    for (iid, slot, handler_iid, f) in [
        (&IID_DATA_PACKAGE3, PACKAGE3_ADD_COMPLETED, &IID_ON_COMPLETED, on_completed as fn(usize, usize) -> i32),
        (&IID_DATA_PACKAGE4, PACKAGE4_ADD_CANCELED, &IID_ON_CANCELED, on_canceled as fn(usize, usize) -> i32),
    ] {
        let Some(pk) = package.query(iid) else { continue };
        let h = Handler::new(handler_iid, f);
        let mut token = 0i64;
        // SAFETY: add_ShareCompleted / add_ShareCanceled(this, handler, token*); the package keeps
        // its reference (and the registration) until it goes.
        unsafe {
            let add: unsafe extern "system" fn(usize, usize, *mut i64) -> i32 = core::mem::transmute(pk.vfn(slot));
            add(pk.ptr(), h as usize, &mut token);
            Handler::release(h);
        }
    }
    Ok(())
}

fn on_completed(_sender: usize, _args: usize) -> i32 {
    answer(SHARE_DONE);
    S_OK
}

fn on_canceled(_sender: usize, _args: usize) -> i32 {
    answer(SHARE_CANCELLED);
    S_OK
}

/// (tests, any thread) the open sheet closed as the user's Esc would: the sheet is the foreground
/// window of another process with the class Windows.UI.Core.CoreWindow (ShellExperienceHost's
/// "Share"); it gets Esc (a posted key, no real input), then the window the sheet came from is put
/// in front again -> false when no such sheet was in front
pub fn win_share_dismiss() -> bool {
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const VK_ESCAPE: usize = 0x1B;
    let h = LAST_HWND.load(Ordering::Acquire);
    // SAFETY: plain window queries; the posted keys go to the sheet's window only; a window of ours
    // comes to the front (this process opened the sheet).
    unsafe {
        let fg = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, &mut pid);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&buf[..n]);
        let sheet = fg != 0 && pid != GetCurrentProcessId() && class == "Windows.UI.Core.CoreWindow";
        if sheet {
            PostMessageW(fg, WM_KEYDOWN, VK_ESCAPE, 0x0001_0001);
            PostMessageW(fg, WM_KEYUP, VK_ESCAPE, 0xC001_0001u32 as isize);
        }
        if h != 0 {
            SetForegroundWindow(h);
        }
        sheet
    }
}

// ---------------------------------------------------------------- COM objects of our own

/// an event handler: IUnknown + Invoke(sender, args); agile
#[repr(C)]
struct Handler {
    vtbl: &'static HandlerVtbl,
    refs: AtomicU32,
    iid: &'static Guid,
    f: fn(usize, usize) -> i32,
}

#[repr(C)]
struct HandlerVtbl {
    qi: unsafe extern "system" fn(*mut Handler, *const Guid, *mut usize) -> i32,
    add_ref: unsafe extern "system" fn(*mut Handler) -> u32,
    release: unsafe extern "system" fn(*mut Handler) -> u32,
    invoke: unsafe extern "system" fn(*mut Handler, usize, usize) -> i32,
}

static HANDLER_VTBL: HandlerVtbl = HandlerVtbl { qi: h_qi, add_ref: h_add_ref, release: h_release, invoke: h_invoke };

impl Handler {
    /// a new handler with one reference (the caller's)
    fn new(iid: &'static Guid, f: fn(usize, usize) -> i32) -> *mut Handler {
        Box::into_raw(Box::new(Handler { vtbl: &HANDLER_VTBL, refs: AtomicU32::new(1), iid, f }))
    }

    /// SAFETY: h is a live handler of ours; the caller gives up one reference.
    unsafe fn release(h: *mut Handler) {
        // SAFETY: (caller)
        unsafe { h_release(h) };
    }
}

unsafe extern "system" fn h_qi(this: *mut Handler, iid: *const Guid, out: *mut usize) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM's contract: a live object, a readable GUID, a writable out pointer.
    unsafe {
        let iid = &*iid;
        if same(iid, &IID_IUNKNOWN) || same(iid, &IID_IAGILEOBJECT) || same(iid, (*this).iid) {
            h_add_ref(this);
            *out = this as usize;
            S_OK
        } else {
            *out = 0;
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn h_add_ref(this: *mut Handler) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn h_release(this: *mut Handler) -> u32 {
    // SAFETY: a live object; the last reference frees it.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            drop(Box::from_raw(this));
        }
        n
    }
}

unsafe extern "system" fn h_invoke(this: *mut Handler, sender: usize, args: usize) -> i32 {
    // SAFETY: a live object.
    let f = unsafe { (*this).f };
    f(sender, args)
}

/// IIterable<IStorageItem> (iterating: false) or its IIterator (true) over our items; both hold a
/// reference of each item
#[repr(C)]
struct Items {
    vtbl: *const usize,
    refs: AtomicU32,
    items: Vec<usize>,
    at: AtomicU32,
    iterating: bool,
}

type QiFn = unsafe extern "system" fn(*mut Items, *const Guid, *mut usize) -> i32;
type RefFn = unsafe extern "system" fn(*mut Items) -> u32;
type IidsFn = unsafe extern "system" fn(*mut Items, *mut u32, *mut usize) -> i32;
type NameFn = unsafe extern "system" fn(*mut Items, *mut isize) -> i32;
type TrustFn = unsafe extern "system" fn(*mut Items, *mut i32) -> i32;
type OutFn = unsafe extern "system" fn(*mut Items, *mut usize) -> i32;
type BoolFn = unsafe extern "system" fn(*mut Items, *mut u8) -> i32;
type ManyFn = unsafe extern "system" fn(*mut Items, u32, *mut usize, *mut u32) -> i32;

#[repr(C)]
struct IterableVtbl(QiFn, RefFn, RefFn, IidsFn, NameFn, TrustFn, OutFn);
#[repr(C)]
struct IteratorVtbl(QiFn, RefFn, RefFn, IidsFn, NameFn, TrustFn, OutFn, BoolFn, BoolFn, ManyFn);

static ITERABLE_VTBL: IterableVtbl = IterableVtbl(i_qi, i_add_ref, i_release, i_iids, i_name, i_trust, i_first);
static ITERATOR_VTBL: IteratorVtbl = IteratorVtbl(i_qi, i_add_ref, i_release, i_iids, i_name, i_trust, i_current, i_has_current, i_move_next, i_get_many);

/// AddRef of a COM pointer
fn add_ref(p: usize) {
    // SAFETY: slot 1 of every COM object is AddRef(this); p is live.
    unsafe {
        let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(*(*(p as *const *const usize)).add(1));
        f(p);
    }
}

impl Items {
    fn new(items: &[usize], iterating: bool) -> *mut Items {
        for &p in items {
            add_ref(p);
        }
        let vtbl = if iterating { &ITERATOR_VTBL as *const IteratorVtbl as *const usize } else { &ITERABLE_VTBL as *const IterableVtbl as *const usize };
        Box::into_raw(Box::new(Items { vtbl, refs: AtomicU32::new(1), items: items.to_vec(), at: AtomicU32::new(0), iterating }))
    }

    /// SAFETY: a live object of ours; the caller gives up one reference.
    unsafe fn release(this: *mut Items) {
        // SAFETY: (caller)
        unsafe { i_release(this) };
    }
}

unsafe extern "system" fn i_qi(this: *mut Items, iid: *const Guid, out: *mut usize) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: COM's contract: a live object, a readable GUID, a writable out pointer.
    unsafe {
        let iid = &*iid;
        let own = if (*this).iterating { &IID_ITERATOR_ITEMS } else { &IID_ITERABLE_ITEMS };
        if same(iid, &IID_IUNKNOWN) || same(iid, &IID_IINSPECTABLE) || same(iid, &IID_IAGILEOBJECT) || same(iid, own) {
            i_add_ref(this);
            *out = this as usize;
            S_OK
        } else {
            *out = 0;
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn i_add_ref(this: *mut Items) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn i_release(this: *mut Items) -> u32 {
    // SAFETY: a live object; the last reference frees it and its items' references.
    unsafe {
        let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
        if n == 0 {
            let b = Box::from_raw(this);
            for &p in &b.items {
                drop(Obj::own(p));
            }
        }
        n
    }
}

unsafe extern "system" fn i_iids(_this: *mut Items, n: *mut u32, out: *mut usize) -> i32 {
    // SAFETY: COM's contract: writable out pointers.
    unsafe {
        if !n.is_null() {
            *n = 0;
        }
        if !out.is_null() {
            *out = 0;
        }
    }
    S_OK
}

unsafe extern "system" fn i_name(_this: *mut Items, out: *mut isize) -> i32 {
    if !out.is_null() {
        // SAFETY: a writable out pointer; the empty HSTRING is 0.
        unsafe { *out = 0 };
    }
    E_NOTIMPL
}

unsafe extern "system" fn i_trust(_this: *mut Items, out: *mut i32) -> i32 {
    if !out.is_null() {
        // SAFETY: a writable out pointer; BaseTrust 0.
        unsafe { *out = 0 };
    }
    S_OK
}

unsafe extern "system" fn i_first(this: *mut Items, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live iterable; the iterator handed out with one reference.
    unsafe {
        *out = Items::new(&(*this).items, true) as usize;
    }
    S_OK
}

unsafe extern "system" fn i_current(this: *mut Items, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live iterator; the item handed out with its own reference.
    unsafe {
        let k = (*this).at.load(Ordering::Acquire) as usize;
        match (&(*this).items).get(k) {
            Some(&p) => {
                add_ref(p);
                *out = p;
                S_OK
            }
            None => {
                *out = 0;
                E_BOUNDS
            }
        }
    }
}

unsafe extern "system" fn i_has_current(this: *mut Items, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live iterator, a writable out pointer.
    unsafe { *out = (((*this).at.load(Ordering::Acquire) as usize) < (&(*this).items).len()) as u8 };
    S_OK
}

unsafe extern "system" fn i_move_next(this: *mut Items, out: *mut u8) -> i32 {
    // SAFETY: a live iterator, a writable out pointer (or none).
    unsafe {
        let n = (&(*this).items).len() as u32;
        let k = (*this).at.load(Ordering::Acquire);
        let k = if k < n { k + 1 } else { k };
        (*this).at.store(k, Ordering::Release);
        if !out.is_null() {
            *out = (k < n) as u8;
        }
    }
    S_OK
}

unsafe extern "system" fn i_get_many(this: *mut Items, cap: u32, items: *mut usize, actual: *mut u32) -> i32 {
    if items.is_null() || actual.is_null() {
        return E_POINTER;
    }
    // SAFETY: a live iterator; items has room for cap pointers, each handed out with a reference.
    unsafe {
        let n = (&(*this).items).len() as u32;
        let k = (*this).at.load(Ordering::Acquire);
        let mut got = 0u32;
        while got < cap && k + got < n {
            let p = (&(*this).items)[(k + got) as usize];
            add_ref(p);
            *items.add(got as usize) = p;
            got += 1;
        }
        (*this).at.store(k + got, Ordering::Release);
        *actual = got;
    }
    S_OK
}
