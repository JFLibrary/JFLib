// win_dnd -- (dnd.rs) drag and drop through OLE: every window is an IDropTarget; DoDragDrop
// drags the app's content out with our own IDataObject and IDropSource
//
//   register(hwnd, w) -> ok   (win_create) OleInitialize once, RegisterDragDrop(hwnd, a target
//                             of window w); false: no OLE on this thread (WM_DROPFILES stays)
//   revoke(hwnd)              (WM_DESTROY) RevokeDragDrop (releases the target)
//   os_drag_start(data)       (dnd.rs drag_start) remember the drag; wait() runs it
//   run_pending()             (wait, top) DoDragDrop of the remembered drag.  It is modal: the
//                             UI keeps drawing from the source's QueryContinueDrag (the
//                             wait_refresh closure, dnd_refresh) while events are queued, so
//                             drop targets of the app highlight; then E_DRAG_END (the effect)
//                             and E_UP for the buttons DoDragDrop swallowed
//   dnd_test_formats(data) / dnd_test_drop(data, w, x, y, keys) / dnd_test_source(esc, keys)
//                             (plattest, no user) our IDataObject read as a drop target reads
//                             it; our IDropTarget fed with it (DragEnter, DragOver, Drop);
//                             the IDropSource's answers
//
// Target: DragEnter asks the data object which formats it has (QueryGetData: CF_HDROP,
// CF_UNICODETEXT / CF_TEXT, "HTML Format", "UniformResourceLocatorW" / "UniformResourceLocator" /
// "text/x-moz-url", "PNG" / "image/png" / CF_DIBV5 / CF_DIB), asks the UI (dnd::drag_query with
// the client point and MK_SHIFT / MK_CONTROL) and answers that effect; E_DRAG goes to the UI on
// enter, on every new point and on leave.  Drop reads only the kinds the UI takes (HGLOBAL
// media, each at most 256 MiB): the paths of the DROPFILES block (parsed here, wide or ANSI),
// text (UTF-16, else bytes), HTML (CF_HTML framing: clip_html_decode), URLs, the image (PNG, else
// the DIB as a BMP file: clip_dib_to_bmp), then E_DROP (data = the number of paths).
// Source: the data object offers CF_UNICODETEXT (the text, else the URLs), CF_HDROP (DROPFILES,
// wide absolute paths), "HTML Format", "UniformResourceLocatorW" + "UniformResourceLocator",
// "PNG", CF_DIBV5 + CF_DIB (from DragData::bmp).  GetData hands out a new HGLOBAL each time
// (TYMED_HGLOBAL only), EnumFormatEtc goes through SHCreateStdEnumFmtEtc, SetData, GetDataHere
// and advise sinks are refused.  ole32 / shell32 are loaded on the first window.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use rt::sync::SpinLock;

use crate::{DE_ALL, DT_FILES, DT_HTML, DT_IMAGE, DT_STD, DT_TEXT, DT_URL, DragData, E_DRAG, E_DROP, Event, MOD_ALT,
            MOD_CTRL, MOD_SHIFT, PW_MAX, evq_push};

use super::{Point, ScreenToClient, dll_proc};

type Hr = i32;
type This = *mut u8;

const S_OK: Hr = 0;
const E_NOTIMPL: Hr = 0x8000_4001_u32 as i32;
const E_NOINTERFACE: Hr = 0x8000_4002_u32 as i32;
const E_INVALIDARG: Hr = 0x8007_0057_u32 as i32;
const E_OUTOFMEMORY: Hr = 0x8007_000E_u32 as i32;
const DV_E_FORMATETC: Hr = 0x8004_0064_u32 as i32;
const DV_E_TYMED: Hr = 0x8004_0069_u32 as i32;
const DV_E_DVASPECT: Hr = 0x8004_006B_u32 as i32;
const OLE_E_ADVISENOTSUPPORTED: Hr = 0x8004_0003_u32 as i32;
const DATA_S_SAMEFORMATETC: Hr = 0x0004_0130;
const DRAGDROP_S_DROP: Hr = 0x0004_0100;
const DRAGDROP_S_CANCEL: Hr = 0x0004_0101;
const DRAGDROP_S_USEDEFAULTCURSORS: Hr = 0x0004_0102;

const TYMED_HGLOBAL: u32 = 1;
const DVASPECT_CONTENT: u32 = 1;
const DATADIR_GET: u32 = 1;
const MK_LBUTTON: u32 = 1;
const MK_SHIFT: u32 = 4;
const MK_CONTROL: u32 = 8;
const MK_ALT: u32 = 0x20;

const CF_TEXT: u16 = 1;
const CF_DIB: u16 = 8;
const CF_UNICODETEXT: u16 = 13;
const CF_HDROP: u16 = 15;
const CF_DIBV5: u16 = 17;
const GMEM_MOVEABLE: u32 = 2;
const DROPFILES_SIZE: usize = 20;
const BLOCK_MAX: usize = 1 << 28; // the largest block of another program we read
const FILES_MAX: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct Iid(u32, u16, u16, [u8; 8]);

const IID_UNKNOWN: Iid = Iid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_DATA_OBJECT: Iid = Iid(0x0000_010e, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_DROP_SOURCE: Iid = Iid(0x0000_0121, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_DROP_TARGET: Iid = Iid(0x0000_0122, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// FORMATETC (x64: 32 bytes)
#[repr(C)]
#[derive(Clone, Copy)]
struct FormatEtc {
    cf: u16,
    ptd: usize,
    aspect: u32,
    index: i32,
    tymed: u32,
}

/// STGMEDIUM (x64: 24 bytes): TYMED_HGLOBAL only here
#[repr(C)]
struct StgMedium {
    tymed: u32,
    h: isize,
    unk: usize,
}

fn fe(cf: u16) -> FormatEtc {
    FormatEtc { cf, ptd: 0, aspect: DVASPECT_CONTENT, index: -1, tymed: TYMED_HGLOBAL }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, n: usize) -> isize;
    fn GlobalFree(h: isize) -> isize;
    fn GlobalLock(h: isize) -> *mut u8;
    fn GlobalUnlock(h: isize) -> i32;
    fn GlobalSize(h: isize) -> usize;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn GetClipboardFormatNameW(fmt: u32, name: *mut u16, cap: i32) -> i32;
}

// ---------------------------------------------------------------- ole32 / shell32

const F_OLEINIT: usize = 0;
const F_REGISTER: usize = 1;
const F_REVOKE: usize = 2;
const F_DODRAG: usize = 3;
const F_RELEASE_MEDIUM: usize = 4;
const F_ENUM_FMT: usize = 5;
const F_N: usize = 6;
static FNS: [AtomicUsize; F_N] = [const { AtomicUsize::new(0) }; F_N];
static LOADED: AtomicU32 = AtomicU32::new(0); // 0 not tried, 1 there, 2 missing

// OLE on this (the UI) thread -> usable
fn load() -> bool {
    match LOADED.load(Ordering::Acquire) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let names: [(&[u8], &[u8]); F_N] = [
        (b"ole32.dll\0", b"OleInitialize\0"),
        (b"ole32.dll\0", b"RegisterDragDrop\0"),
        (b"ole32.dll\0", b"RevokeDragDrop\0"),
        (b"ole32.dll\0", b"DoDragDrop\0"),
        (b"ole32.dll\0", b"ReleaseStgMedium\0"),
        (b"shell32.dll\0", b"SHCreateStdEnumFmtEtc\0"),
    ];
    for (k, (dll, name)) in names.iter().enumerate() {
        let p = dll_proc(dll, name);
        if p == 0 {
            LOADED.store(2, Ordering::Release);
            return false;
        }
        FNS[k].store(p, Ordering::Release);
    }
    // SAFETY: OleInitialize(LPVOID reserved) -> HRESULT; S_FALSE = already; RPC_E_CHANGED_MODE
    // (a multithreaded apartment) leaves OLE drag and drop out.
    let hr = unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(FNS[F_OLEINIT].load(Ordering::Acquire))(0) };
    let ok = hr >= 0;
    LOADED.store(if ok { 1 } else { 2 }, Ordering::Release);
    ok
}

fn f(k: usize) -> usize {
    FNS[k].load(Ordering::Acquire)
}

// the registered formats (0 = not yet)
const R_HTML: usize = 0;
const R_URLW: usize = 1;
const R_URLA: usize = 2;
const R_PNG: usize = 3;
const R_IMPNG: usize = 4;
const R_MOZURL: usize = 5;
const R_NAMES: [&str; 6] = ["HTML Format", "UniformResourceLocatorW", "UniformResourceLocator", "PNG", "image/png", "text/x-moz-url"];
static REG: [AtomicU32; 6] = [const { AtomicU32::new(0) }; 6];

fn reg(k: usize) -> u16 {
    let v = REG[k].load(Ordering::Relaxed);
    if v != 0 {
        return v as u16;
    }
    let w = super::super::to_wide(R_NAMES[k]);
    // SAFETY: a NUL-terminated name that lives through the call.
    let v = unsafe { RegisterClipboardFormatW(w.as_ptr()) };
    REG[k].store(v, Ordering::Relaxed);
    v as u16
}

// a movable global block with these bytes (0 = out of memory)
fn gblock(data: &[u8]) -> isize {
    // SAFETY: a new block of at least data.len() bytes, locked for the copy.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
        if h == 0 {
            return 0;
        }
        let p = GlobalLock(h);
        if p.is_null() {
            GlobalFree(h);
            return 0;
        }
        core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        h
    }
}

// slot k of the vtable of COM object o
//
// SAFETY: o is a live COM interface pointer whose vtable has more than k slots.
unsafe fn vf(o: usize, k: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(k) }
}

fn com_add(o: usize) {
    if o != 0 {
        // SAFETY: IUnknown::AddRef (slot 1) of a live interface.
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> u32>(vf(o, 1))(o) };
    }
}

fn com_release(o: usize) {
    if o != 0 {
        // SAFETY: IUnknown::Release (slot 2) of a live interface we hold a reference of.
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> u32>(vf(o, 2))(o) };
    }
}

// ---------------------------------------------------------------- reading a data object

// IDataObject::QueryGetData (slot 5): format cf as an HGLOBAL
fn obj_has(o: usize, cf: u16) -> bool {
    if o == 0 || cf == 0 {
        return false;
    }
    let e = fe(cf);
    // SAFETY: QueryGetData(this, const FORMATETC*) of a live data object.
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize, *const FormatEtc) -> Hr>(vf(o, 5))(o, &e) == S_OK }
}

// IDataObject::GetData (slot 3) of format cf as bytes (None: not there, not an HGLOBAL, too large)
fn obj_get(o: usize, cf: u16) -> Option<Vec<u8>> {
    if o == 0 || cf == 0 {
        return None;
    }
    let e = fe(cf);
    let mut m = StgMedium { tymed: 0, h: 0, unk: 0 };
    // SAFETY: GetData(this, const FORMATETC*, STGMEDIUM*) fills the medium; we release it with
    // ReleaseStgMedium; the block is read while locked, within GlobalSize.
    unsafe {
        let get: unsafe extern "system" fn(usize, *const FormatEtc, *mut StgMedium) -> Hr = core::mem::transmute(vf(o, 3));
        if get(o, &e, &mut m) < 0 {
            return None;
        }
        let mut r = None;
        if m.tymed == TYMED_HGLOBAL && m.h != 0 {
            let n = GlobalSize(m.h);
            let p = GlobalLock(m.h);
            if !p.is_null() {
                if n <= BLOCK_MAX {
                    r = Some(core::slice::from_raw_parts(p, n).to_vec());
                }
                GlobalUnlock(m.h);
            }
        }
        let rel: unsafe extern "system" fn(*mut StgMedium) = core::mem::transmute(f(F_RELEASE_MEDIUM));
        rel(&mut m);
        r
    }
}

// the DT_* kinds a data object has
fn offered(o: usize) -> u32 {
    let mut t = 0;
    if obj_has(o, CF_HDROP) {
        t |= DT_FILES;
    }
    if obj_has(o, CF_UNICODETEXT) || obj_has(o, CF_TEXT) {
        t |= DT_TEXT;
    }
    if obj_has(o, reg(R_HTML)) {
        t |= DT_HTML;
    }
    if obj_has(o, reg(R_URLW)) || obj_has(o, reg(R_URLA)) || obj_has(o, reg(R_MOZURL)) {
        t |= DT_URL;
    }
    if [reg(R_PNG), reg(R_IMPNG), CF_DIBV5, CF_DIB].into_iter().any(|cf| obj_has(o, cf)) {
        t |= DT_IMAGE;
    }
    t
}

// the paths of a DROPFILES block (wide or ANSI) -> added to the pending drop
fn hdrop_paths(b: &[u8]) -> u32 {
    if b.len() < DROPFILES_SIZE {
        return 0;
    }
    let off = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let wide = u32::from_le_bytes([b[16], b[17], b[18], b[19]]) != 0;
    if off < DROPFILES_SIZE || off > b.len() {
        return 0;
    }
    let mut n = 0;
    let mut paths: Vec<String> = Vec::new();
    if wide {
        let units: Vec<u16> = b[off..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for p in units.split(|&u| u == 0) {
            if p.is_empty() || paths.len() >= FILES_MAX {
                break; // the empty string ends the list
            }
            paths.push(super::super::from_wide(p));
        }
    } else {
        for p in b[off..].split(|&c| c == 0) {
            if p.is_empty() || paths.len() >= FILES_MAX {
                break;
            }
            paths.push(String::from_utf8_lossy(p).into_owned());
        }
    }
    for p in paths {
        if crate::drop_add(&p) {
            n += 1;
        }
    }
    n
}

// the drop: read the kinds `t` of data object o into the pending drop -> paths added
fn read_drop(o: usize, t: u32) -> u32 {
    crate::drop_reset();
    let mut files = 0;
    if t & DT_FILES != 0 {
        if let Some(b) = obj_get(o, CF_HDROP) {
            files = hdrop_paths(&b);
        }
    }
    if t & DT_TEXT != 0 {
        let text = match obj_get(o, CF_UNICODETEXT) {
            Some(b) => Some(crate::drop_text_from(&b, true)),
            None => obj_get(o, CF_TEXT).map(|b| crate::drop_text_from(&b, false)),
        };
        if let Some(mut s) = text {
            s.retain(|&c| c != b'\r'); // CR LF -> LF
            crate::drop_set_text(&s);
        }
    }
    if t & DT_HTML != 0 {
        if let Some(fr) = obj_get(o, reg(R_HTML)).and_then(|b| crate::clip_html_decode(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])) {
            crate::drop_set_html(&fr);
        }
    }
    if t & DT_URL != 0 {
        if let Some(b) = obj_get(o, reg(R_URLW)) {
            crate::drop_url_list(&crate::drop_text_from(&b, true));
        } else if let Some(b) = obj_get(o, reg(R_URLA)) {
            crate::drop_url_list(&crate::drop_text_from(&b, false));
        } else if let Some(b) = obj_get(o, reg(R_MOZURL)) {
            // "URL\ntitle": the first line
            let s = crate::drop_text_from(&b, true);
            crate::drop_url_list(s.split(|&c| c == b'\n').next().unwrap_or(&[]));
        }
    }
    if t & DT_IMAGE != 0 {
        let png = obj_get(o, reg(R_PNG)).or_else(|| obj_get(o, reg(R_IMPNG)));
        let img = png.or_else(|| obj_get(o, CF_DIBV5).or_else(|| obj_get(o, CF_DIB)).and_then(|d| crate::clip_dib_to_bmp(&d)));
        if let Some(b) = img {
            crate::drop_set_image(b);
        }
    }
    files
}

fn mods_of(keys: u32) -> u8 {
    let mut m = 0;
    if keys & MK_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if keys & MK_CONTROL != 0 {
        m |= MOD_CTRL;
    }
    if keys & MK_ALT != 0 {
        m |= MOD_ALT;
    }
    m
}

// ---------------------------------------------------------------- the drop target

#[repr(C)]
struct VtTarget {
    qi: unsafe extern "system" fn(This, *const Iid, *mut This) -> Hr,
    add: unsafe extern "system" fn(This) -> u32,
    rel: unsafe extern "system" fn(This) -> u32,
    enter: unsafe extern "system" fn(This, usize, u32, u64, *mut u32) -> Hr,
    over: unsafe extern "system" fn(This, u32, u64, *mut u32) -> Hr,
    leave: unsafe extern "system" fn(This) -> Hr,
    drop: unsafe extern "system" fn(This, usize, u32, u64, *mut u32) -> Hr,
}

/// one window's drop target: the data object of the drag over it (a reference), what it offers
#[repr(C)]
struct Target {
    vt: *const VtTarget,
    refs: AtomicU32,
    w: i32,
    hwnd: isize,
    obj: AtomicUsize,
    types: AtomicU32,
    allowed: AtomicU32,
    last: AtomicU64,
}

static VT_TARGET: VtTarget = VtTarget { qi: t_qi, add: t_add, rel: t_rel, enter: t_enter, over: t_over, leave: t_leave, drop: t_drop };

fn new_target(w: i32, hwnd: isize) -> *mut Target {
    Box::into_raw(Box::new(Target {
        vt: &VT_TARGET,
        refs: AtomicU32::new(1),
        w,
        hwnd,
        obj: AtomicUsize::new(0),
        types: AtomicU32::new(0),
        allowed: AtomicU32::new(0),
        last: AtomicU64::new(u64::MAX),
    }))
}

// SAFETY (all methods below): `this` is the interface pointer OLE got from us (a live Target /
// Source / DataObj); out pointers are valid for one write, as the interface contracts say.

unsafe extern "system" fn t_qi(this: This, riid: *const Iid, out: *mut This) -> Hr {
    if out.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let id = unsafe { *riid };
    if id == IID_UNKNOWN || id == IID_DROP_TARGET {
        unsafe {
            *out = this;
            t_add(this);
        }
        S_OK
    } else {
        unsafe { *out = core::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn t_add(this: This) -> u32 {
    let t = unsafe { &*(this as *const Target) };
    t.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn t_rel(this: This) -> u32 {
    let t = unsafe { &*(this as *const Target) };
    let n = t.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        com_release(t.obj.swap(0, Ordering::AcqRel));
        // SAFETY: the last reference: the Box of new_target.
        drop(unsafe { Box::from_raw(this as *mut Target) });
    }
    n
}

// a screen point (POINTL by value) -> client px of the target's window
fn client_xy(hwnd: isize, pt: u64) -> (i32, i32) {
    let mut p = Point { x: pt as u32 as i32, y: (pt >> 32) as u32 as i32 };
    if hwnd != 0 {
        // SAFETY: a valid POINT; a window of ours.
        unsafe { ScreenToClient(hwnd, &mut p) };
    }
    (p.x, p.y)
}

fn push(typ: u8, w: i32, x: i32, y: i32, data: u32, mods: u8) {
    let win = (w as u32 & (PW_MAX as u32 - 1)) as u8;
    evq_push(Event { typ, btn: 0, mods, win, x, y, data });
}

// the answer to the source at this point (the UI's hook), E_DRAG when the point is new
fn answer(t: &Target, keys: u32, pt: u64, eff: *mut u32, force: bool) {
    let (x, y) = client_xy(t.hwnd, pt);
    let (types, allowed) = (t.types.load(Ordering::Acquire), t.allowed.load(Ordering::Acquire));
    let (_, e) = crate::drag_query(t.w, x, y, types, allowed, mods_of(keys));
    if !eff.is_null() {
        // SAFETY: the DWORD* of DragEnter / DragOver.
        unsafe { *eff = e };
    }
    let key = (x as u32 as u64) | ((y as u32 as u64) << 32);
    let moved = t.last.swap(key, Ordering::AcqRel) != key;
    if (force || moved) && types != 0 {
        push(E_DRAG, t.w, x, y, types | (allowed & DE_ALL) << 8, mods_of(keys));
    }
    super::dnd_refresh(); // our own DoDragDrop runs: show the highlight now
}

unsafe extern "system" fn t_enter(this: This, obj: usize, keys: u32, pt: u64, eff: *mut u32) -> Hr {
    let t = unsafe { &*(this as *const Target) };
    com_add(obj);
    com_release(t.obj.swap(obj, Ordering::AcqRel));
    t.types.store(offered(obj) & DT_STD, Ordering::Release);
    let allowed = if eff.is_null() { 0 } else { unsafe { *eff } };
    t.allowed.store(allowed & DE_ALL, Ordering::Release);
    t.last.store(u64::MAX, Ordering::Release);
    answer(t, keys, pt, eff, true);
    S_OK
}

unsafe extern "system" fn t_over(this: This, keys: u32, pt: u64, eff: *mut u32) -> Hr {
    let t = unsafe { &*(this as *const Target) };
    answer(t, keys, pt, eff, false);
    S_OK
}

unsafe extern "system" fn t_leave(this: This) -> Hr {
    let t = unsafe { &*(this as *const Target) };
    com_release(t.obj.swap(0, Ordering::AcqRel));
    push(E_DRAG, t.w, -1, -1, 0, 0);
    super::dnd_refresh();
    S_OK
}

unsafe extern "system" fn t_drop(this: This, obj: usize, keys: u32, pt: u64, eff: *mut u32) -> Hr {
    let t = unsafe { &*(this as *const Target) };
    let (x, y) = client_xy(t.hwnd, pt);
    let allowed = if eff.is_null() { t.allowed.load(Ordering::Acquire) } else { (unsafe { *eff }) & DE_ALL };
    let offer = if obj != 0 { offered(obj) & DT_STD } else { 0 };
    let (types, e) = crate::drag_query(t.w, x, y, offer, allowed, mods_of(keys));
    if e != 0 {
        let files = read_drop(obj, types);
        crate::drop_set_effect(e);
        push(E_DROP, t.w, x, y, files, mods_of(keys));
    } else {
        push(E_DRAG, t.w, -1, -1, 0, 0);
    }
    if !eff.is_null() {
        unsafe { *eff = e };
    }
    com_release(t.obj.swap(0, Ordering::AcqRel));
    S_OK
}

/// (win_create) the window becomes an OLE drop target -> false: OLE is not there
pub(super) fn register(hwnd: isize, w: i32) -> bool {
    if hwnd == 0 || !load() {
        return false;
    }
    let t = new_target(w, hwnd);
    // SAFETY: RegisterDragDrop(HWND, IDropTarget*) -> HRESULT takes its own reference.
    let hr = unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(isize, *mut Target) -> Hr>(f(F_REGISTER))(hwnd, t) };
    // SAFETY: our first reference goes (the registration keeps the target alive).
    unsafe { t_rel(t as This) };
    hr >= 0
}

/// (WM_DESTROY) the window is no drop target any more
pub(super) fn revoke(hwnd: isize) {
    if LOADED.load(Ordering::Acquire) == 1 {
        // SAFETY: RevokeDragDrop(HWND) -> HRESULT (an error when it was never registered).
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(isize) -> Hr>(f(F_REVOKE))(hwnd) };
    }
}

// ---------------------------------------------------------------- the data object (source)

#[repr(C)]
struct VtData {
    qi: unsafe extern "system" fn(This, *const Iid, *mut This) -> Hr,
    add: unsafe extern "system" fn(This) -> u32,
    rel: unsafe extern "system" fn(This) -> u32,
    get: unsafe extern "system" fn(This, *const FormatEtc, *mut StgMedium) -> Hr,
    get_here: unsafe extern "system" fn(This, *const FormatEtc, *mut StgMedium) -> Hr,
    query: unsafe extern "system" fn(This, *const FormatEtc) -> Hr,
    canonical: unsafe extern "system" fn(This, *const FormatEtc, *mut FormatEtc) -> Hr,
    set: unsafe extern "system" fn(This, *const FormatEtc, *mut StgMedium, i32) -> Hr,
    enum_fmt: unsafe extern "system" fn(This, u32, *mut usize) -> Hr,
    advise: unsafe extern "system" fn(This, *const FormatEtc, u32, usize, *mut u32) -> Hr,
    unadvise: unsafe extern "system" fn(This, u32) -> Hr,
    enum_advise: unsafe extern "system" fn(This, *mut usize) -> Hr,
}

/// our data object: one block of bytes per clipboard format
#[repr(C)]
struct DataObj {
    vt: *const VtData,
    refs: AtomicU32,
    items: Vec<(u16, Vec<u8>)>,
    fmts: Vec<FormatEtc>,
}

static VT_DATA: VtData = VtData {
    qi: d_qi,
    add: d_add,
    rel: d_rel,
    get: d_get,
    get_here: d_get_here,
    query: d_query,
    canonical: d_canonical,
    set: d_set,
    enum_fmt: d_enum,
    advise: d_advise,
    unadvise: d_unadvise,
    enum_advise: d_enum_advise,
};

// UTF-16 with CR LF line ends and a final NUL, as bytes
fn wide_bytes(s: &str, crlf: bool) -> Vec<u8> {
    let mut b = Vec::with_capacity(s.len() * 2 + 2);
    let mut prev = 0u16;
    for u in s.encode_utf16() {
        if crlf && u == 10 && prev != 13 {
            b.extend_from_slice(&13u16.to_le_bytes());
        }
        b.extend_from_slice(&u.to_le_bytes());
        prev = u;
    }
    b.extend_from_slice(&[0, 0]);
    b
}

// an absolute Windows path of p (relative ones: from the working folder; '/' -> '\')
fn abs_path(p: &str) -> String {
    let p: String = p.chars().map(|c| if c == '/' { '\\' } else { c }).collect();
    let b = p.as_bytes();
    let abs = b.first() == Some(&b'\\') || (b.len() >= 2 && b[1] == b':');
    if abs {
        return p;
    }
    let mut d: String = super::super::cwd().chars().map(|c| if c == '/' { '\\' } else { c }).collect();
    if !d.ends_with('\\') {
        d.push('\\');
    }
    d.push_str(p.trim_start_matches(".\\"));
    d
}

// the blocks of a drag: (format, bytes)
fn data_items(d: &DragData) -> Vec<(u16, Vec<u8>)> {
    let mut v = Vec::new();
    let text = d.text.clone().or_else(|| if d.urls.is_empty() { None } else { Some(d.urls.join("\n")) });
    if let Some(t) = text {
        v.push((CF_UNICODETEXT, wide_bytes(&t, true)));
    }
    if !d.files.is_empty() {
        let mut b = Vec::new();
        b.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
        b.extend_from_slice(&[0; 12]); // pt, fNC
        b.extend_from_slice(&1u32.to_le_bytes()); // fWide
        for p in &d.files {
            b.extend_from_slice(&wide_bytes(&abs_path(p), false));
        }
        b.extend_from_slice(&[0, 0]);
        v.push((CF_HDROP, b));
    }
    if let Some(h) = d.html.as_deref().and_then(|h| crate::clip_html_encode(h.as_bytes())) {
        v.push((reg(R_HTML), h));
    }
    if let Some(u) = d.urls.first() {
        v.push((reg(R_URLW), wide_bytes(u, false)));
        let mut a = u.as_bytes().to_vec();
        a.push(0);
        v.push((reg(R_URLA), a));
    }
    if let Some(png) = &d.image {
        v.push((reg(R_PNG), png.clone()));
    }
    // the BMP file: 14-byte file header + BITMAPV5HEADER (124) + pixels -> CF_DIBV5, CF_DIB
    if let Some(bmp) = &d.bmp {
        let n = bmp.len();
        if n >= 14 + 124 && u32::from_le_bytes([bmp[14], bmp[15], bmp[16], bmp[17]]) == 124 {
            let off = u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]) as usize;
            if (14 + 124..=n).contains(&off) {
                v.push((CF_DIBV5, bmp[14..].to_vec()));
                let mut dib = Vec::with_capacity(40 + n - off);
                dib.extend_from_slice(&bmp[14..54]);
                dib[0..4].copy_from_slice(&40u32.to_le_bytes());
                dib[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
                dib[32..40].copy_from_slice(&[0; 8]);
                dib.extend_from_slice(&bmp[off..]);
                v.push((CF_DIB, dib));
            }
        }
    }
    v.retain(|e| e.0 != 0);
    v
}

fn new_data(d: &DragData) -> *mut DataObj {
    let items = data_items(d);
    let fmts = items.iter().map(|e| fe(e.0)).collect();
    Box::into_raw(Box::new(DataObj { vt: &VT_DATA, refs: AtomicU32::new(1), items, fmts }))
}

unsafe extern "system" fn d_qi(this: This, riid: *const Iid, out: *mut This) -> Hr {
    if out.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let id = unsafe { *riid };
    if id == IID_UNKNOWN || id == IID_DATA_OBJECT {
        unsafe {
            *out = this;
            d_add(this);
        }
        S_OK
    } else {
        unsafe { *out = core::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn d_add(this: This) -> u32 {
    let d = unsafe { &*(this as *const DataObj) };
    d.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn d_rel(this: This) -> u32 {
    let d = unsafe { &*(this as *const DataObj) };
    let n = d.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference: the Box of new_data.
        drop(unsafe { Box::from_raw(this as *mut DataObj) });
    }
    n
}

// the block of a FORMATETC (Err: the HRESULT of the refusal)
fn d_find<'a>(d: &'a DataObj, e: *const FormatEtc) -> Result<&'a [u8], Hr> {
    if e.is_null() {
        return Err(E_INVALIDARG);
    }
    // SAFETY: the caller's FORMATETC.
    let e = unsafe { *e };
    let Some(item) = d.items.iter().find(|i| i.0 == e.cf) else { return Err(DV_E_FORMATETC) };
    if e.aspect != DVASPECT_CONTENT {
        return Err(DV_E_DVASPECT);
    }
    if e.tymed & TYMED_HGLOBAL == 0 {
        return Err(DV_E_TYMED);
    }
    Ok(&item.1)
}

unsafe extern "system" fn d_get(this: This, e: *const FormatEtc, m: *mut StgMedium) -> Hr {
    let d = unsafe { &*(this as *const DataObj) };
    if m.is_null() {
        return E_INVALIDARG;
    }
    match d_find(d, e) {
        Ok(bytes) => {
            let h = gblock(bytes);
            if h == 0 {
                return E_OUTOFMEMORY;
            }
            unsafe { *m = StgMedium { tymed: TYMED_HGLOBAL, h, unk: 0 } };
            S_OK
        }
        Err(hr) => hr,
    }
}

unsafe extern "system" fn d_get_here(_this: This, _e: *const FormatEtc, _m: *mut StgMedium) -> Hr {
    E_NOTIMPL
}

unsafe extern "system" fn d_query(this: This, e: *const FormatEtc) -> Hr {
    let d = unsafe { &*(this as *const DataObj) };
    match d_find(d, e) {
        Ok(_) => S_OK,
        Err(hr) => hr,
    }
}

unsafe extern "system" fn d_canonical(_this: This, e: *const FormatEtc, out: *mut FormatEtc) -> Hr {
    if out.is_null() || e.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *out = *e;
        (*out).ptd = 0;
    }
    DATA_S_SAMEFORMATETC
}

unsafe extern "system" fn d_set(_this: This, _e: *const FormatEtc, _m: *mut StgMedium, _release: i32) -> Hr {
    E_NOTIMPL
}

unsafe extern "system" fn d_enum(this: This, dir: u32, out: *mut usize) -> Hr {
    let d = unsafe { &*(this as *const DataObj) };
    if out.is_null() {
        return E_INVALIDARG;
    }
    if dir != DATADIR_GET {
        unsafe { *out = 0 };
        return E_NOTIMPL;
    }
    // SAFETY: SHCreateStdEnumFmtEtc(UINT, const FORMATETC*, IEnumFORMATETC**) copies the array.
    unsafe {
        let mk: unsafe extern "system" fn(u32, *const FormatEtc, *mut usize) -> Hr = core::mem::transmute(f(F_ENUM_FMT));
        mk(d.fmts.len() as u32, d.fmts.as_ptr(), out)
    }
}

unsafe extern "system" fn d_advise(_this: This, _e: *const FormatEtc, _f: u32, _sink: usize, _c: *mut u32) -> Hr {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn d_unadvise(_this: This, _c: u32) -> Hr {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn d_enum_advise(_this: This, out: *mut usize) -> Hr {
    if !out.is_null() {
        unsafe { *out = 0 };
    }
    OLE_E_ADVISENOTSUPPORTED
}

// ---------------------------------------------------------------- the drop source

#[repr(C)]
struct VtSource {
    qi: unsafe extern "system" fn(This, *const Iid, *mut This) -> Hr,
    add: unsafe extern "system" fn(This) -> u32,
    rel: unsafe extern "system" fn(This) -> u32,
    query: unsafe extern "system" fn(This, i32, u32) -> Hr,
    feedback: unsafe extern "system" fn(This, u32) -> Hr,
}

#[repr(C)]
struct Source {
    vt: *const VtSource,
    refs: AtomicU32,
}

static VT_SOURCE: VtSource = VtSource { qi: s_qi, add: s_add, rel: s_rel, query: s_query, feedback: s_feedback };

unsafe extern "system" fn s_qi(this: This, riid: *const Iid, out: *mut This) -> Hr {
    if out.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let id = unsafe { *riid };
    if id == IID_UNKNOWN || id == IID_DROP_SOURCE {
        unsafe {
            *out = this;
            s_add(this);
        }
        S_OK
    } else {
        unsafe { *out = core::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn s_add(this: This) -> u32 {
    let s = unsafe { &*(this as *const Source) };
    s.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn s_rel(this: This) -> u32 {
    let s = unsafe { &*(this as *const Source) };
    let n = s.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference: the Box of run_pending.
        drop(unsafe { Box::from_raw(this as *mut Source) });
    }
    n
}

// Esc cancels, the left button's release drops; meanwhile the UI keeps drawing
fn source_answer(esc: bool, keys: u32) -> Hr {
    if esc {
        DRAGDROP_S_CANCEL
    } else if keys & MK_LBUTTON == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn s_query(_this: This, esc: i32, keys: u32) -> Hr {
    super::dnd_refresh();
    source_answer(esc != 0, keys)
}

unsafe extern "system" fn s_feedback(_this: This, _effect: u32) -> Hr {
    DRAGDROP_S_USEDEFAULTCURSORS
}

// ---------------------------------------------------------------- the outgoing drag

static PENDING: SpinLock<Option<(DragData, i32)>> = SpinLock::new(None);
static RUNNING_WIN: AtomicI32 = AtomicI32::new(-1);

/// our own DoDragDrop runs
pub(super) fn dragging() -> bool {
    RUNNING_WIN.load(Ordering::Acquire) >= 0
}

/// (dnd.rs drag_start) DoDragDrop runs at the start of the next wait()
pub(crate) fn os_drag_start(data: DragData) -> bool {
    if !load() {
        return false;
    }
    *PENDING.lock() = Some((data, super::st().btn_win.get()));
    true
}

/// (wait) DoDragDrop of the remembered drag, then E_DRAG_END and E_UP of the held buttons
pub(super) fn run_pending() {
    let Some((data, w)) = PENDING.lock().take() else { return };
    let obj = new_data(&data);
    let src = Box::into_raw(Box::new(Source { vt: &VT_SOURCE, refs: AtomicU32::new(1) }));
    let mut eff = 0u32;
    RUNNING_WIN.store(w, Ordering::Release);
    // SAFETY: DoDragDrop(IDataObject*, IDropSource*, DWORD ok effects, DWORD* effect) with our
    // live objects; it runs the modal drag loop on this thread.
    let hr = unsafe {
        let dd: unsafe extern "system" fn(*mut DataObj, *mut Source, u32, *mut u32) -> Hr = core::mem::transmute(f(F_DODRAG));
        dd(obj, src, data.allowed(), &mut eff)
    };
    RUNNING_WIN.store(-1, Ordering::Release);
    // SAFETY: our references of the two objects (OLE took and gave back its own).
    unsafe {
        d_rel(obj as This);
        s_rel(src as This);
    }
    let effect = if hr == DRAGDROP_S_DROP { eff & DE_ALL } else { 0 };
    crate::dnd::drag_finished((w.max(0) as u32 & (PW_MAX as u32 - 1)) as u8, effect);
    super::release_all_buttons(); // DoDragDrop took the release
}

// ---------------------------------------------------------------- tests (plattest)

// the name of a clipboard format
fn format_name(cf: u16) -> String {
    match cf {
        CF_TEXT => return String::from("CF_TEXT"),
        CF_DIB => return String::from("CF_DIB"),
        CF_UNICODETEXT => return String::from("CF_UNICODETEXT"),
        CF_HDROP => return String::from("CF_HDROP"),
        CF_DIBV5 => return String::from("CF_DIBV5"),
        _ => {}
    }
    let mut w = [0u16; 128];
    // SAFETY: a buffer of 128 wide chars.
    let n = unsafe { GetClipboardFormatNameW(cf as u32, w.as_mut_ptr(), 128) };
    super::super::from_wide(&w[..n.clamp(0, 128) as usize])
}

/// (plattest) our IDataObject of `data` read as a drop target reads it: EnumFormatEtc, then
/// QueryGetData and GetData of every format -> (format name, bytes); None: no OLE
pub fn dnd_test_formats(data: &DragData) -> Option<Vec<(String, Vec<u8>)>> {
    if !load() {
        return None;
    }
    let o = new_data(data) as usize;
    let mut out = Vec::new();
    let mut en = 0usize;
    // SAFETY: EnumFormatEtc (slot 8) of our object; IEnumFORMATETC::Next (slot 3) fills one
    // FORMATETC at a time; every interface is released again.
    unsafe {
        let ef: unsafe extern "system" fn(usize, u32, *mut usize) -> Hr = core::mem::transmute(vf(o, 8));
        if ef(o, DATADIR_GET, &mut en) >= 0 && en != 0 {
            let next: unsafe extern "system" fn(usize, u32, *mut FormatEtc, *mut u32) -> Hr = core::mem::transmute(vf(en, 3));
            loop {
                let mut e = fe(0);
                let mut got = 0u32;
                if next(en, 1, &mut e, &mut got) != S_OK || got != 1 {
                    break;
                }
                if obj_has(o, e.cf) {
                    out.push((format_name(e.cf), obj_get(o, e.cf).unwrap_or_default()));
                }
            }
            com_release(en);
        }
        d_rel(o as This);
    }
    Some(out)
}

/// (plattest) a drop of `data` onto platform window w at client (x, y) as OLE would make it:
/// DragEnter, DragOver, Drop of our IDropTarget with our IDataObject (MK_* keys) -> the effect
/// of the drop (the E_DRAG / E_DROP events are queued, the content waits in drop_data_take);
/// None: no OLE
pub fn dnd_test_drop(data: &DragData, w: i32, x: i32, y: i32, keys: u32) -> Option<u32> {
    if !load() {
        return None;
    }
    let hwnd = super::hwnd_of(w);
    let mut p = Point { x, y };
    if hwnd != 0 {
        // SAFETY: our window, a valid POINT.
        unsafe { super::ClientToScreen(hwnd, &mut p) };
    }
    let pt = (p.x as u32 as u64) | ((p.y as u32 as u64) << 32);
    let t = new_target(w, hwnd);
    let o = new_data(data);
    let mut eff = data.allowed();
    // SAFETY: our own objects through their vtables, as OLE calls them.
    unsafe {
        t_enter(t as This, o as usize, keys, pt, &mut eff);
        eff = data.allowed();
        t_over(t as This, keys, pt, &mut eff);
        eff = data.allowed();
        t_drop(t as This, o as usize, keys, pt, &mut eff);
        d_rel(o as This);
        t_rel(t as This);
    }
    Some(eff)
}

/// (plattest) the IDropSource's answer to QueryContinueDrag(esc, MK_* keys): 0 go on,
/// 1 drop, 2 cancel
pub fn dnd_test_source(esc: bool, keys: u32) -> u32 {
    match source_answer(esc, keys) {
        DRAGDROP_S_DROP => 1,
        DRAGDROP_S_CANCEL => 2,
        _ => 0,
    }
}
