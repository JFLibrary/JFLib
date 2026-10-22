// win_uia -- (a11y.rs) the accessibility tree as UI Automation providers
//
//   get_object(hwnd, pw, wp, lp)   (wndproc, WM_GETOBJECT) UiaRootObjectId: the provider of the
//                                  window's root (UiaReturnRawElementProvider); assistive
//                                  technology is there from now on (a11y::set_active)
//   destroyed(hwnd)                (WM_DESTROY) UIA lets go of the window's providers
//   changed(snapshot, changes)     the UI published a new tree: UIA events (only while clients
//                                  listen: UiaClientsAreListening)
//   uia_test_hwnd / uia_test_tree / uia_test_act / uia_test_listen / uia_test_events /
//   uia_test_unlisten              (tests) a UI Automation client (IUIAutomation) that reads a
//                                  window, acts and listens to events as a screen reader does
//
// Every element is a small COM object of our own (Elem): the window's root (key 0) and one per
// node, created on demand and identified by (platform window, node key); its methods answer
// from a11y::current() on whatever thread UIA calls them (the provider is free-threaded: no
// UseComThreading) and fail with UIA_E_ELEMENTNOTAVAILABLE once the node is gone.  Interfaces:
// IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot (the
// root; its host provider comes from UiaHostProviderFromHwnd) and the patterns Invoke (buttons,
// links, menu items), Toggle (check boxes, switches), Value (text boxes; drop-downs read-only),
// RangeValue (sliders; progress bars read-only), SelectionItem (radio buttons, tabs, list rows),
// ExpandCollapse (drop-downs, menus), Text / Text2 (text boxes: ranges, the caret and the
// selection; win_uia_text.rs).  Headings are Text elements with HeadingLevel1 .. 9 (and the
// AriaRole "heading", AriaProperties "level=N").  Dialogs, switches and headings name their
// control type themselves (LocalizedControlType: a11y::role_name, in the UI language), the
// others leave it to UIA.  Actions become a11y::request.  Events: focus
// changed, property changes (toggle state, selected, expand state, enabled, name, value, range
// value, heading level), text selection changed (the caret moved: screen readers and Magnifier
// follow it), text changed, structure changed (children invalidated).  UIAutomationCore.dll
// and oleaut32.dll are loaded on the first WM_GETOBJECT.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use rt::sync::SpinLock;

use crate::a11y::{self as ax, Act, Change, Node, Request, Role, Snapshot, WinTree, NONE};

use super::{ClientToScreen, GetClientRect, Point, Rect, dll_proc, win_hwnd};

#[path = "win_uia_text.rs"]
mod text;
use text::{EV_TEXT_CHANGED, EV_TEXT_SELECTION, IID_TEXT, IID_TEXT2, P_TEXT, P_TEXT2, VT_TEXT};

type Hr = i32;
type This = *mut u8;

const S_OK: Hr = 0;
const E_NOINTERFACE: Hr = 0x8000_4002_u32 as i32;
const E_INVALIDARG: Hr = 0x8007_0057_u32 as i32;
const UIA_E_ELEMENTNOTENABLED: Hr = 0x8004_0200_u32 as i32;
const UIA_E_ELEMENTNOTAVAILABLE: Hr = 0x8004_0201_u32 as i32;
const UIA_E_INVALIDOPERATION: Hr = 0x8013_1509_u32 as i32;
const UIA_ROOT_OBJECT_ID: i32 = -25;

const VT_I4: u16 = 3;
const VT_R8: u16 = 5;
const VT_BSTR: u16 = 8;
const VT_BOOL: u16 = 11;

// patterns
const P_INVOKE: i32 = 10000;
const P_VALUE: i32 = 10002;
const P_RANGE: i32 = 10003;
const P_EXPAND: i32 = 10005;
const P_SELITEM: i32 = 10010;
const P_TOGGLE: i32 = 10015;

// properties
const PR_CONTROL_TYPE: i32 = 30003;
const PR_LOCALIZED_TYPE: i32 = 30004;
const PR_NAME: i32 = 30005;
const PR_HAS_FOCUS: i32 = 30008;
const PR_FOCUSABLE: i32 = 30009;
const PR_ENABLED: i32 = 30010;
const PR_AUTOMATION_ID: i32 = 30011;
const PR_CONTROL_ELEMENT: i32 = 30016;
const PR_CONTENT_ELEMENT: i32 = 30017;
const PR_PASSWORD: i32 = 30019;
const PR_OFFSCREEN: i32 = 30022;
const PR_ORIENTATION: i32 = 30023;
const PR_FRAMEWORK: i32 = 30024;
const PR_VALUE: i32 = 30045;
const PR_RANGE_VALUE: i32 = 30047;
const PR_RANGE_MIN: i32 = 30049;
const PR_RANGE_MAX: i32 = 30050;
const PR_EXPAND_STATE: i32 = 30070;
const PR_SELECTED: i32 = 30079;
const PR_TOGGLE_STATE: i32 = 30086;
const PR_ARIA_ROLE: i32 = 30101;
const PR_ARIA_PROPERTIES: i32 = 30102;
const PR_HEADING_LEVEL: i32 = 30173;
// HeadingLevel_None; HeadingLevel1 .. 9 follow it
const HEADING_LEVEL_NONE: i32 = 80050;

// events
const EV_FOCUS: i32 = 20005;
const EV_SELECTED: i32 = 20012;

// control types
const CT_BUTTON: i32 = 50000;
const CT_CHECKBOX: i32 = 50002;
const CT_COMBO: i32 = 50003;
const CT_EDIT: i32 = 50004;
const CT_LINK: i32 = 50005;
const CT_IMAGE: i32 = 50006;
const CT_LISTITEM: i32 = 50007;
const CT_LIST: i32 = 50008;
const CT_MENU: i32 = 50009;
const CT_MENUBAR: i32 = 50010;
const CT_MENUITEM: i32 = 50011;
const CT_PROGRESS: i32 = 50012;
const CT_RADIO: i32 = 50013;
const CT_SLIDER: i32 = 50015;
const CT_TAB: i32 = 50018;
const CT_TABITEM: i32 = 50019;
const CT_TEXT: i32 = 50020;
const CT_CUSTOM: i32 = 50025;
const CT_GROUP: i32 = 50026;
const CT_PANE: i32 = 50033;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Guid(u32, u16, u16, [u8; 8]);

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    Guid(a, b, c, d)
}

const IID_UNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_SIMPLE: Guid = guid(0xd6dd68d1, 0x86fd, 0x4332, [0x86, 0x66, 0x9a, 0xbe, 0xde, 0xa2, 0xd2, 0x4c]);
const IID_FRAGMENT: Guid = guid(0xf7063da8, 0x8359, 0x439c, [0x92, 0x97, 0xbb, 0xc5, 0x29, 0x9a, 0x7d, 0x87]);
const IID_ROOT: Guid = guid(0x620ce2a5, 0xab8f, 0x40a9, [0x86, 0xcb, 0xde, 0x3c, 0x75, 0x59, 0x9b, 0x58]);
const IID_INVOKE: Guid = guid(0x54fcb24b, 0xe18e, 0x47a2, [0xb4, 0xd3, 0xec, 0xcb, 0xe7, 0x75, 0x99, 0xa2]);
const IID_TOGGLE: Guid = guid(0x56d00bd0, 0xc4f4, 0x433c, [0xa8, 0x36, 0x1a, 0x52, 0xa5, 0x7e, 0x08, 0x92]);
const IID_VALUE: Guid = guid(0xc7935180, 0x6fb3, 0x4201, [0xb1, 0x74, 0x7d, 0xf7, 0x3a, 0xdb, 0xf6, 0x4a]);
const IID_RANGE: Guid = guid(0x36dc7aef, 0x33e6, 0x4691, [0xaf, 0xe1, 0x2b, 0xe7, 0x27, 0x4b, 0x3d, 0x33]);
const IID_SELITEM: Guid = guid(0x2acad808, 0xb2d4, 0x452d, [0xa4, 0x07, 0x91, 0xff, 0x1a, 0xd1, 0x67, 0xb2]);
const IID_EXPAND: Guid = guid(0xd847d3a5, 0xcab0, 0x4a98, [0x8e, 0xcb, 0xec, 0xb4, 0x5c, 0x59, 0xad, 0x24]);

/// a VARIANT (x64: 24 bytes)
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Variant {
    vt: u16,
    r: [u16; 3],
    v: [u64; 2],
}

impl Variant {
    fn i4(x: i32) -> Variant {
        Variant { vt: VT_I4, v: [x as u32 as u64, 0], ..Variant::default() }
    }
    fn r8(x: f64) -> Variant {
        Variant { vt: VT_R8, v: [x.to_bits(), 0], ..Variant::default() }
    }
    fn bool(x: bool) -> Variant {
        Variant { vt: VT_BOOL, v: [if x { 0xFFFF } else { 0 }, 0], ..Variant::default() }
    }
    fn bstr(s: &str) -> Variant {
        Variant { vt: VT_BSTR, v: [bstr(s) as u64, 0], ..Variant::default() }
    }
}

#[repr(C)]
struct UiaRect {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

// ---------------------------------------------------------------- the DLLs

const F_RETURN: usize = 0;
const F_HOST: usize = 1;
const F_RAISE: usize = 2;
const F_RAISE_PROP: usize = 3;
const F_RAISE_STRUCT: usize = 4;
const F_LISTENING: usize = 5;
const F_ALLOC: usize = 6;
const F_FREE: usize = 7;
const F_SA_CREATE: usize = 8;
const F_SA_ACCESS: usize = 9;
const F_SA_UNACCESS: usize = 10;
const F_VARIANT_CLEAR: usize = 11;
const F_COINIT: usize = 12;
const F_COCREATE: usize = 13;
const F_NOTSUP: usize = 14;
const F_SA_DESTROY: usize = 15;
const F_N: usize = 16;
static FNS: [AtomicUsize; F_N] = [const { AtomicUsize::new(0) }; F_N];
static LOADED: AtomicU32 = AtomicU32::new(0);

fn load() {
    if LOADED.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return; // loaded, or another thread is loading
    }
    let names: [(&[u8], &[u8]); F_N] = [
        (b"UIAutomationCore.dll\0", b"UiaReturnRawElementProvider\0"),
        (b"UIAutomationCore.dll\0", b"UiaHostProviderFromHwnd\0"),
        (b"UIAutomationCore.dll\0", b"UiaRaiseAutomationEvent\0"),
        (b"UIAutomationCore.dll\0", b"UiaRaiseAutomationPropertyChangedEvent\0"),
        (b"UIAutomationCore.dll\0", b"UiaRaiseStructureChangedEvent\0"),
        (b"UIAutomationCore.dll\0", b"UiaClientsAreListening\0"),
        (b"oleaut32.dll\0", b"SysAllocStringLen\0"),
        (b"oleaut32.dll\0", b"SysFreeString\0"),
        (b"oleaut32.dll\0", b"SafeArrayCreateVector\0"),
        (b"oleaut32.dll\0", b"SafeArrayAccessData\0"),
        (b"oleaut32.dll\0", b"SafeArrayUnaccessData\0"),
        (b"oleaut32.dll\0", b"VariantClear\0"),
        (b"ole32.dll\0", b"CoInitializeEx\0"),
        (b"ole32.dll\0", b"CoCreateInstance\0"),
        (b"UIAutomationCore.dll\0", b"UiaGetReservedNotSupportedValue\0"),
        (b"oleaut32.dll\0", b"SafeArrayDestroy\0"),
    ];
    for (k, (dll, name)) in names.iter().enumerate() {
        FNS[k].store(dll_proc(dll, name), Ordering::Release);
    }
    LOADED.store(2, Ordering::Release);
}

fn f(k: usize) -> usize {
    load();
    while LOADED.load(Ordering::Acquire) != 2 {
        core::hint::spin_loop(); // another thread is loading
    }
    FNS[k].load(Ordering::Acquire)
}

// a BSTR of s (0 when oleaut32 is missing); the receiver frees it
fn bstr(s: &str) -> usize {
    let p = f(F_ALLOC);
    if p == 0 {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    // SAFETY: SysAllocStringLen(const OLECHAR*, UINT) -> BSTR copies w.
    let a: unsafe extern "system" fn(*const u16, u32) -> usize = unsafe { core::mem::transmute(p) };
    unsafe { a(w.as_ptr(), w.len() as u32) }
}

fn bstr_free(b: usize) {
    let p = f(F_FREE);
    if p != 0 && b != 0 {
        // SAFETY: SysFreeString(BSTR) on a BSTR we own.
        let ff: unsafe extern "system" fn(usize) = unsafe { core::mem::transmute(p) };
        unsafe { ff(b) };
    }
}

// a BSTR / NUL-terminated wide string -> String
//
// SAFETY: p is null or points to NUL-terminated UTF-16.
unsafe fn wide_str(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    // SAFETY: the caller guarantees the terminating NUL.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n units were read above.
    crate::from_wide(unsafe { core::slice::from_raw_parts(p, n) })
}

// a SAFEARRAY of VT_I4 with these numbers (null when oleaut32 is missing)
fn int_array(v: &[i32]) -> *mut u8 {
    let (c, acc, un) = (f(F_SA_CREATE), f(F_SA_ACCESS), f(F_SA_UNACCESS));
    if c == 0 || acc == 0 || un == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: SafeArrayCreateVector(VARTYPE, LONG, ULONG) -> SAFEARRAY*; the data of a fresh
    // vector of v.len() VT_I4 elements is written between Access and Unaccess.
    unsafe {
        let create: unsafe extern "system" fn(u16, i32, u32) -> *mut u8 = core::mem::transmute(c);
        let access: unsafe extern "system" fn(*mut u8, *mut *mut i32) -> Hr = core::mem::transmute(acc);
        let unaccess: unsafe extern "system" fn(*mut u8) -> Hr = core::mem::transmute(un);
        let sa = create(VT_I4, 0, v.len() as u32);
        if sa.is_null() {
            return sa;
        }
        let mut d: *mut i32 = core::ptr::null_mut();
        if access(sa, &mut d) >= 0 && !d.is_null() {
            core::ptr::copy_nonoverlapping(v.as_ptr(), d, v.len());
            unaccess(sa);
        }
        sa
    }
}

// ---------------------------------------------------------------- the element objects

type Qi = unsafe extern "system" fn(This, *const Guid, *mut This) -> Hr;
type Ref = unsafe extern "system" fn(This) -> u32;
type M0 = unsafe extern "system" fn(This) -> Hr;
type MOut = unsafe extern "system" fn(This, *mut This) -> Hr;
type MI32 = unsafe extern "system" fn(This, *mut i32) -> Hr;
type MF64 = unsafe extern "system" fn(This, *mut f64) -> Hr;

#[repr(C)]
struct VtSimple {
    qi: Qi,
    add: Ref,
    rel: Ref,
    options: MI32,
    pattern: unsafe extern "system" fn(This, i32, *mut This) -> Hr,
    prop: unsafe extern "system" fn(This, i32, *mut Variant) -> Hr,
    host: MOut,
}

#[repr(C)]
struct VtFrag {
    qi: Qi,
    add: Ref,
    rel: Ref,
    navigate: unsafe extern "system" fn(This, i32, *mut This) -> Hr,
    runtime_id: MOut,
    bounds: unsafe extern "system" fn(This, *mut UiaRect) -> Hr,
    embedded: MOut,
    set_focus: M0,
    root: MOut,
}

#[repr(C)]
struct VtRoot {
    qi: Qi,
    add: Ref,
    rel: Ref,
    from_point: unsafe extern "system" fn(This, f64, f64, *mut This) -> Hr,
    focus: MOut,
}

#[repr(C)]
struct VtInvoke {
    qi: Qi,
    add: Ref,
    rel: Ref,
    invoke: M0,
}

#[repr(C)]
struct VtToggle {
    qi: Qi,
    add: Ref,
    rel: Ref,
    toggle: M0,
    state: MI32,
}

#[repr(C)]
struct VtValue {
    qi: Qi,
    add: Ref,
    rel: Ref,
    set: unsafe extern "system" fn(This, *const u16) -> Hr,
    get: unsafe extern "system" fn(This, *mut usize) -> Hr,
    read_only: MI32,
}

#[repr(C)]
struct VtRange {
    qi: Qi,
    add: Ref,
    rel: Ref,
    set: unsafe extern "system" fn(This, f64) -> Hr,
    value: MF64,
    read_only: MI32,
    max: MF64,
    min: MF64,
    large: MF64,
    small: MF64,
}

#[repr(C)]
struct VtSelItem {
    qi: Qi,
    add: Ref,
    rel: Ref,
    select: M0,
    add_to: M0,
    remove: M0,
    selected: MI32,
    container: MOut,
}

#[repr(C)]
struct VtExpand {
    qi: Qi,
    add: Ref,
    rel: Ref,
    expand: M0,
    collapse: M0,
    state: MI32,
}

// the interface slots of an Elem
const O_SIMPLE: usize = 0;
const O_FRAG: usize = 1;
const O_ROOT: usize = 2;
const O_INVOKE: usize = 3;
const O_TOGGLE: usize = 4;
const O_VALUE: usize = 5;
const O_RANGE: usize = 6;
const O_SELITEM: usize = 7;
const O_EXPAND: usize = 8;
const O_TEXT: usize = 9;

/// one element: ten interface pointers (each to its vtable), the reference count, which
/// window and node it is (key 0: the window's root)
#[repr(C)]
struct Elem {
    vt: [*const u8; 10],
    refs: AtomicU32,
    hwnd: isize,
    pw: i32,
    key: u64,
}

static VT_SIMPLE: VtSimple = VtSimple {
    qi: qi::<O_SIMPLE>,
    add: add::<O_SIMPLE>,
    rel: rel::<O_SIMPLE>,
    options: s_options,
    pattern: s_pattern,
    prop: s_prop,
    host: s_host,
};
static VT_FRAG: VtFrag = VtFrag {
    qi: qi::<O_FRAG>,
    add: add::<O_FRAG>,
    rel: rel::<O_FRAG>,
    navigate: f_navigate,
    runtime_id: f_runtime_id,
    bounds: f_bounds,
    embedded: f_embedded,
    set_focus: f_set_focus,
    root: f_root,
};
static VT_ROOT: VtRoot = VtRoot { qi: qi::<O_ROOT>, add: add::<O_ROOT>, rel: rel::<O_ROOT>, from_point: r_from_point, focus: r_focus };
static VT_INVOKE: VtInvoke = VtInvoke { qi: qi::<O_INVOKE>, add: add::<O_INVOKE>, rel: rel::<O_INVOKE>, invoke: i_invoke };
static VT_TOGGLE: VtToggle = VtToggle { qi: qi::<O_TOGGLE>, add: add::<O_TOGGLE>, rel: rel::<O_TOGGLE>, toggle: t_toggle, state: t_state };
static VT_VALUE: VtValue =
    VtValue { qi: qi::<O_VALUE>, add: add::<O_VALUE>, rel: rel::<O_VALUE>, set: v_set, get: v_get, read_only: v_read_only };
static VT_RANGE: VtRange = VtRange {
    qi: qi::<O_RANGE>,
    add: add::<O_RANGE>,
    rel: rel::<O_RANGE>,
    set: rv_set,
    value: rv_value,
    read_only: rv_read_only,
    max: rv_max,
    min: rv_min,
    large: rv_large,
    small: rv_small,
};
static VT_SELITEM: VtSelItem = VtSelItem {
    qi: qi::<O_SELITEM>,
    add: add::<O_SELITEM>,
    rel: rel::<O_SELITEM>,
    select: si_select,
    add_to: si_select,
    remove: si_remove,
    selected: si_selected,
    container: si_container,
};
static VT_EXPAND: VtExpand =
    VtExpand { qi: qi::<O_EXPAND>, add: add::<O_EXPAND>, rel: rel::<O_EXPAND>, expand: e_expand, collapse: e_collapse, state: e_state };

fn new_elem(hwnd: isize, pw: i32, key: u64) -> *mut Elem {
    let vt = [
        &VT_SIMPLE as *const VtSimple as *const u8,
        &VT_FRAG as *const VtFrag as *const u8,
        &VT_ROOT as *const VtRoot as *const u8,
        &VT_INVOKE as *const VtInvoke as *const u8,
        &VT_TOGGLE as *const VtToggle as *const u8,
        &VT_VALUE as *const VtValue as *const u8,
        &VT_RANGE as *const VtRange as *const u8,
        &VT_SELITEM as *const VtSelItem as *const u8,
        &VT_EXPAND as *const VtExpand as *const u8,
        &VT_TEXT as *const text::VtText as *const u8,
    ];
    Box::into_raw(Box::new(Elem { vt, refs: AtomicU32::new(1), hwnd, pw, key }))
}

// the interface pointer of slot k
fn iface(e: *mut Elem, k: usize) -> This {
    // SAFETY: e is a live Elem; slot k lies inside it.
    unsafe { (e as *mut u8).add(k * core::mem::size_of::<usize>()) }
}

// the Elem behind interface pointer this of slot K
//
// SAFETY: this is an interface pointer of slot K of a live Elem.
unsafe fn elem<const K: usize>(this: This) -> &'static Elem {
    unsafe { &*(this.sub(K * core::mem::size_of::<usize>()) as *const Elem) }
}

fn release(e: *mut Elem) {
    // SAFETY: e is a live Elem the caller holds a reference of.
    unsafe { rel::<O_SIMPLE>(iface(e, O_SIMPLE)) };
}

// the patterns of a node (bit = slot)
fn patterns(n: &Node) -> u32 {
    match n.role {
        Role::Button | Role::Link | Role::MenuItem => 1 << O_INVOKE,
        Role::CheckBox | Role::Switch => 1 << O_TOGGLE,
        Role::TextBox => 1 << O_VALUE | 1 << O_TEXT,
        Role::Combo => 1 << O_VALUE | 1 << O_EXPAND,
        Role::Slider | Role::Progress => 1 << O_RANGE,
        Role::Radio | Role::Tab | Role::ListItem => 1 << O_SELITEM,
        Role::Menu if n.key & 1 << 63 == 0 => 1 << O_EXPAND,
        _ => 0,
    }
}

fn control_type(n: &Node) -> i32 {
    match n.role {
        Role::Group => CT_GROUP,
        Role::Label | Role::Heading => CT_TEXT,
        Role::Button | Role::Switch => CT_BUTTON,
        Role::CheckBox => CT_CHECKBOX,
        Role::Radio => CT_RADIO,
        Role::Slider => CT_SLIDER,
        Role::Progress => CT_PROGRESS,
        Role::TextBox => CT_EDIT,
        Role::Link => CT_LINK,
        Role::List => CT_LIST,
        Role::ListItem => CT_LISTITEM,
        Role::MenuBar => CT_MENUBAR,
        // a popup menu (synthetic) / a menu bar entry
        Role::Menu if n.key & 1 << 63 != 0 => CT_MENU,
        Role::Menu | Role::MenuItem => CT_MENUITEM,
        Role::TabList => CT_TAB,
        Role::Tab => CT_TABITEM,
        Role::Combo => CT_COMBO,
        Role::Image => CT_IMAGE,
        Role::Canvas => CT_CUSTOM,
        Role::Dialog => CT_PANE,
    }
}

// the element's window tree and node (NONE: the root); None: gone
fn look(e: &Elem) -> Option<(Arc<Snapshot>, usize, u32)> {
    look_key(e.pw, e.key)
}

// the window tree of platform window pw and its node with this key (NONE: key 0, the root)
fn look_key(pw: i32, key: u64) -> Option<(Arc<Snapshot>, usize, u32)> {
    let s = ax::current()?;
    let wi = s.wins.iter().position(|w| w.pw == pw)?;
    let k = if key == 0 { NONE } else { s.wins[wi].find(key)? as u32 };
    Some((s, wi, k))
}

// f(tree, node) for a node element (None: gone, or the root)
fn with_node<R>(e: &Elem, f: impl FnOnce(&WinTree, &Node) -> R) -> Option<R> {
    let (s, wi, k) = look(e)?;
    let t = &s.wins[wi];
    t.nodes.get(k as usize).map(|n| f(t, n))
}

// an element of the same window (key 0: the root) as interface slot k
fn out_elem(e: &Elem, key: u64, k: usize) -> This {
    iface(new_elem(e.hwnd, e.pw, key), k)
}

// SAFETY (all methods below): `this` is the interface pointer UIA got from us; out pointers
// are valid for one write, as the interface contracts say.

unsafe extern "system" fn qi<const K: usize>(this: This, riid: *const Guid, out: *mut This) -> Hr {
    let e = unsafe { elem::<K>(this) };
    if out.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let id = unsafe { *riid };
    let want = |p: usize| with_node(e, |_, n| patterns(n) & 1 << p != 0).unwrap_or(false);
    let slot = if id == IID_UNKNOWN || id == IID_SIMPLE {
        Some(O_SIMPLE)
    } else if id == IID_FRAGMENT {
        Some(O_FRAG)
    } else if id == IID_ROOT && e.key == 0 {
        Some(O_ROOT)
    } else if id == IID_INVOKE && want(O_INVOKE) {
        Some(O_INVOKE)
    } else if id == IID_TOGGLE && want(O_TOGGLE) {
        Some(O_TOGGLE)
    } else if id == IID_VALUE && want(O_VALUE) {
        Some(O_VALUE)
    } else if id == IID_RANGE && want(O_RANGE) {
        Some(O_RANGE)
    } else if id == IID_SELITEM && want(O_SELITEM) {
        Some(O_SELITEM)
    } else if id == IID_EXPAND && want(O_EXPAND) {
        Some(O_EXPAND)
    } else if (id == IID_TEXT || id == IID_TEXT2) && want(O_TEXT) {
        Some(O_TEXT)
    } else {
        None
    };
    match slot {
        Some(s) => {
            e.refs.fetch_add(1, Ordering::AcqRel);
            unsafe { *out = iface(e as *const Elem as *mut Elem, s) };
            S_OK
        }
        None => {
            unsafe { *out = core::ptr::null_mut() };
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn add<const K: usize>(this: This) -> u32 {
    unsafe { elem::<K>(this) }.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn rel<const K: usize>(this: This) -> u32 {
    let e = unsafe { elem::<K>(this) };
    let n = e.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference: the Box made in new_elem goes.
        drop(unsafe { Box::from_raw(e as *const Elem as *mut Elem) });
    }
    n
}

// ---- IRawElementProviderSimple

unsafe extern "system" fn s_options(_this: This, out: *mut i32) -> Hr {
    unsafe { *out = 2 }; // ProviderOptions_ServerSideProvider
    S_OK
}

unsafe extern "system" fn s_pattern(this: This, id: i32, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_SIMPLE>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let slot = match id {
        P_INVOKE => O_INVOKE,
        P_TOGGLE => O_TOGGLE,
        P_VALUE => O_VALUE,
        P_RANGE => O_RANGE,
        P_SELITEM => O_SELITEM,
        P_EXPAND => O_EXPAND,
        P_TEXT | P_TEXT2 => O_TEXT,
        _ => return S_OK,
    };
    if with_node(e, |_, n| patterns(n) & 1 << slot != 0).unwrap_or(false) {
        e.refs.fetch_add(1, Ordering::AcqRel);
        unsafe { *out = iface(e as *const Elem as *mut Elem, slot) };
    }
    S_OK
}

unsafe extern "system" fn s_prop(this: This, id: i32, out: *mut Variant) -> Hr {
    let e = unsafe { elem::<O_SIMPLE>(this) };
    let v = if e.key == 0 {
        // the root: the host provider (the HWND) has the rest
        match id {
            PR_FRAMEWORK => Variant::bstr("JFLib"),
            _ => Variant::default(),
        }
    } else {
        match with_node(e, |t, n| prop(t, n, id)) {
            Some(v) => v,
            None => return UIA_E_ELEMENTNOTAVAILABLE,
        }
    };
    unsafe { *out = v };
    S_OK
}

// a property of a node
fn prop(t: &WinTree, n: &Node, id: i32) -> Variant {
    match id {
        PR_CONTROL_TYPE => Variant::i4(control_type(n)),
        // (the other control types: UIA's own names, in the system's language)
        PR_LOCALIZED_TYPE if matches!(n.role, Role::Dialog | Role::Switch | Role::Heading) => Variant::bstr(ax::role_name(n.role)),
        // headings: Text elements with their level (Narrator, NVDA: heading navigation), the
        // ARIA role and level for clients that read those
        PR_HEADING_LEVEL if n.role == Role::Heading => Variant::i4(HEADING_LEVEL_NONE + n.level() as i32),
        PR_ARIA_ROLE if n.role == Role::Heading => Variant::bstr("heading"),
        PR_ARIA_PROPERTIES if n.role == Role::Heading => Variant::bstr(&alloc::format!("level={}", n.level())),
        PR_NAME => Variant::bstr(&n.name),
        PR_AUTOMATION_ID => Variant::bstr(&alloc::format!("{:x}", n.key)),
        PR_ENABLED => Variant::bool(n.state & ax::S_DISABLED == 0),
        PR_FOCUSABLE => Variant::bool(n.state & ax::S_FOCUSABLE != 0),
        PR_HAS_FOCUS => Variant::bool(n.state & ax::S_FOCUSED != 0 && t.active),
        PR_CONTROL_ELEMENT | PR_CONTENT_ELEMENT => Variant::bool(true),
        PR_OFFSCREEN => Variant::bool(!t.on_screen(n)), // scrolled out of the window
        PR_PASSWORD => Variant::bool(false),
        PR_FRAMEWORK => Variant::bstr("JFLib"),
        PR_ORIENTATION if matches!(n.role, Role::Slider | Role::Progress) => Variant::i4(1), // horizontal
        _ => Variant::default(),
    }
}

unsafe extern "system" fn s_host(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_SIMPLE>(this) };
    unsafe { *out = core::ptr::null_mut() };
    if e.key != 0 {
        return S_OK;
    }
    let p = f(F_HOST);
    if p == 0 {
        return S_OK;
    }
    // SAFETY: UiaHostProviderFromHwnd(HWND, IRawElementProviderSimple**) -> HRESULT.
    let host: unsafe extern "system" fn(isize, *mut This) -> Hr = unsafe { core::mem::transmute(p) };
    unsafe { host(e.hwnd, out) }
}

// ---- IRawElementProviderFragment

unsafe extern "system" fn f_navigate(this: This, dir: i32, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_FRAG>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let Some((s, wi, k)) = look(e) else {
        return if e.key == 0 { S_OK } else { UIA_E_ELEMENTNOTAVAILABLE };
    };
    let t = &s.wins[wi];
    let key = |i: u32| if i == NONE { None } else { Some(t.nodes[i as usize].key) };
    let to: Option<u64> = match (dir, k) {
        // parent: the root for a top-level node; the root has none of its own
        (0, NONE) => None,
        (0, k) => Some(key(t.nodes[k as usize].parent).unwrap_or(0)),
        (1, NONE) | (2, NONE) => None,
        (1, k) => key(t.nodes[k as usize].next),
        (2, k) => key(t.nodes[k as usize].prev),
        (3, k) => t.children(k).next().map(|i| t.nodes[i].key),
        (4, k) => t.children(k).last().map(|i| t.nodes[i].key),
        _ => return E_INVALIDARG,
    };
    if let Some(key) = to {
        unsafe { *out = out_elem(e, key, O_FRAG) };
    }
    S_OK
}

unsafe extern "system" fn f_runtime_id(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_FRAG>(this) };
    // the root: UIA takes the HWND's
    unsafe { *out = if e.key == 0 { core::ptr::null_mut() } else { int_array(&runtime_id(e.pw, e.key)) } };
    S_OK
}

fn runtime_id(pw: i32, key: u64) -> [i32; 4] {
    [3, pw, key as u32 as i32, (key >> 32) as u32 as i32] // UiaAppendRuntimeId
}

// the client area's origin on the screen
fn origin(hwnd: isize) -> (i32, i32) {
    let mut p = Point { x: 0, y: 0 };
    // SAFETY: a valid POINT; a stale HWND only fails.
    unsafe { ClientToScreen(hwnd, &mut p) };
    (p.x, p.y)
}

unsafe extern "system" fn f_bounds(this: This, out: *mut UiaRect) -> Hr {
    let e = unsafe { elem::<O_FRAG>(this) };
    let (ox, oy) = origin(e.hwnd);
    let r = if e.key == 0 {
        let mut rc = Rect { l: 0, t: 0, r: 0, b: 0 };
        // SAFETY: a valid RECT.
        unsafe { GetClientRect(e.hwnd, &mut rc) };
        [0, 0, rc.r - rc.l, rc.b - rc.t]
    } else {
        match with_node(e, |_, n| n.rect) {
            Some(r) => r,
            None => return UIA_E_ELEMENTNOTAVAILABLE,
        }
    };
    unsafe {
        *out = UiaRect { left: (r[0] + ox) as f64, top: (r[1] + oy) as f64, width: r[2] as f64, height: r[3] as f64 };
    }
    S_OK
}

unsafe extern "system" fn f_embedded(_this: This, out: *mut This) -> Hr {
    unsafe { *out = core::ptr::null_mut() };
    S_OK
}

// a request for this element's node (UIA_E_ELEMENTNOTENABLED when the node does not allow it)
fn act(e: &Elem, need: u32, a: Act) -> Hr {
    let Some((s, wi, k)) = look(e) else { return UIA_E_ELEMENTNOTAVAILABLE };
    let t = &s.wins[wi];
    let Some(n) = t.nodes.get(k as usize) else { return UIA_E_INVALIDOPERATION };
    if n.actions & need == 0 {
        return UIA_E_ELEMENTNOTENABLED;
    }
    ax::request(Request { win: t.win, key: n.key, act: a });
    S_OK
}

unsafe extern "system" fn f_set_focus(this: This) -> Hr {
    let e = unsafe { elem::<O_FRAG>(this) };
    if e.key == 0 {
        return S_OK;
    }
    act(e, ax::A_FOCUS, Act::Focus)
}

unsafe extern "system" fn f_root(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_FRAG>(this) };
    unsafe { *out = out_elem(e, 0, O_ROOT) };
    S_OK
}

// ---- IRawElementProviderFragmentRoot

unsafe extern "system" fn r_from_point(this: This, x: f64, y: f64, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_ROOT>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let (ox, oy) = origin(e.hwnd);
    if let Some((s, wi, _)) = look(e) {
        let t = &s.wins[wi];
        if let Some(i) = t.at_point(x as i32 - ox, y as i32 - oy) {
            unsafe { *out = out_elem(e, t.nodes[i].key, O_FRAG) };
        }
    }
    S_OK
}

unsafe extern "system" fn r_focus(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_ROOT>(this) };
    unsafe { *out = core::ptr::null_mut() };
    if let Some((s, wi, _)) = look(e) {
        let t = &s.wins[wi];
        if let Some(n) = t.nodes.get(t.focus as usize) {
            unsafe { *out = out_elem(e, n.key, O_FRAG) };
        }
    }
    S_OK
}

// ---- the patterns

unsafe extern "system" fn i_invoke(this: This) -> Hr {
    act(unsafe { elem::<O_INVOKE>(this) }, ax::A_CLICK, Act::Click)
}

unsafe extern "system" fn t_toggle(this: This) -> Hr {
    act(unsafe { elem::<O_TOGGLE>(this) }, ax::A_CLICK, Act::Click)
}

fn toggle_state(n: &Node) -> i32 {
    if n.state & ax::S_MIXED != 0 {
        2
    } else {
        (n.state & (ax::S_CHECKED | ax::S_PRESSED) != 0) as i32
    }
}

unsafe extern "system" fn t_state(this: This, out: *mut i32) -> Hr {
    match with_node(unsafe { elem::<O_TOGGLE>(this) }, |_, n| toggle_state(n)) {
        Some(v) => {
            unsafe { *out = v };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn v_set(this: This, s: *const u16) -> Hr {
    let text = unsafe { wide_str(s) };
    act(unsafe { elem::<O_VALUE>(this) }, ax::A_TEXT, Act::Text(text))
}

unsafe extern "system" fn v_get(this: This, out: *mut usize) -> Hr {
    match with_node(unsafe { elem::<O_VALUE>(this) }, |_, n| bstr(&n.text)) {
        Some(b) => {
            unsafe { *out = b };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn v_read_only(this: This, out: *mut i32) -> Hr {
    match with_node(unsafe { elem::<O_VALUE>(this) }, |_, n| n.actions & ax::A_TEXT == 0) {
        Some(v) => {
            unsafe { *out = v as i32 };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn rv_set(this: This, v: f64) -> Hr {
    act(unsafe { elem::<O_RANGE>(this) }, ax::A_VALUE, Act::Value(v))
}

fn range(this: This, out: *mut f64, f: impl FnOnce(&Node) -> f64) -> Hr {
    match with_node(unsafe { elem::<O_RANGE>(this) }, |_, n| f(n)) {
        Some(v) => {
            unsafe { *out = v };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn rv_value(this: This, out: *mut f64) -> Hr {
    range(this, out, |n| n.value.max(n.min))
}

unsafe extern "system" fn rv_read_only(this: This, out: *mut i32) -> Hr {
    match with_node(unsafe { elem::<O_RANGE>(this) }, |_, n| n.actions & ax::A_VALUE == 0) {
        Some(v) => {
            unsafe { *out = v as i32 };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn rv_max(this: This, out: *mut f64) -> Hr {
    range(this, out, |n| n.max)
}

unsafe extern "system" fn rv_min(this: This, out: *mut f64) -> Hr {
    range(this, out, |n| n.min)
}

unsafe extern "system" fn rv_large(this: This, out: *mut f64) -> Hr {
    range(this, out, |n| ((n.max - n.min) / 10.0).max(n.step))
}

unsafe extern "system" fn rv_small(this: This, out: *mut f64) -> Hr {
    range(this, out, |n| n.step)
}

fn selected(n: &Node) -> bool {
    n.state & if n.role == Role::Radio { ax::S_CHECKED } else { ax::S_SELECTED } != 0
}

unsafe extern "system" fn si_select(this: This) -> Hr {
    let e = unsafe { elem::<O_SELITEM>(this) };
    if with_node(e, |_, n| selected(n)) == Some(true) {
        return S_OK;
    }
    act(e, ax::A_CLICK, Act::Click)
}

unsafe extern "system" fn si_remove(_this: This) -> Hr {
    UIA_E_INVALIDOPERATION // single selection: a row cannot be taken out
}

unsafe extern "system" fn si_selected(this: This, out: *mut i32) -> Hr {
    match with_node(unsafe { elem::<O_SELITEM>(this) }, |_, n| selected(n)) {
        Some(v) => {
            unsafe { *out = v as i32 };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn si_container(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_SELITEM>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let p = with_node(e, |t, n| t.nodes.get(n.parent as usize).filter(|p| matches!(p.role, Role::List | Role::TabList)).map(|p| p.key));
    if let Some(Some(key)) = p {
        unsafe { *out = out_elem(e, key, O_SIMPLE) };
    }
    S_OK
}

fn expand(this: This, open: bool) -> Hr {
    let e = unsafe { elem::<O_EXPAND>(this) };
    if with_node(e, |_, n| n.state & ax::S_EXPANDED != 0) == Some(open) {
        return S_OK;
    }
    act(e, ax::A_CLICK, Act::Click)
}

unsafe extern "system" fn e_expand(this: This) -> Hr {
    expand(this, true)
}

unsafe extern "system" fn e_collapse(this: This) -> Hr {
    expand(this, false)
}

unsafe extern "system" fn e_state(this: This, out: *mut i32) -> Hr {
    match with_node(unsafe { elem::<O_EXPAND>(this) }, |_, n| (n.state & ax::S_EXPANDED != 0) as i32) {
        Some(v) => {
            unsafe { *out = v };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

// ---------------------------------------------------------------- the window procedure, events

pub fn start(_app: &str) {}

/// (wndproc, WM_GETOBJECT) the root provider of platform window pw for UiaRootObjectId; None:
/// another object id (DefWindowProc answers)
pub(crate) fn get_object(hwnd: isize, pw: i32, wp: usize, lp: isize) -> Option<isize> {
    if lp as i32 != UIA_ROOT_OBJECT_ID {
        return None;
    }
    let p = f(F_RETURN);
    if p == 0 {
        return None;
    }
    ax::set_active(true);
    let e = new_elem(hwnd, pw, 0);
    // SAFETY: UiaReturnRawElementProvider(HWND, WPARAM, LPARAM, IRawElementProviderSimple*) ->
    // LRESULT; UIA takes its own references.
    let ret: unsafe extern "system" fn(isize, usize, isize, This) -> isize = unsafe { core::mem::transmute(p) };
    let r = unsafe { ret(hwnd, wp, lp, iface(e, O_SIMPLE)) };
    release(e);
    Some(r)
}

/// (wndproc, WM_DESTROY) UIA lets go of the window's providers
pub(crate) fn destroyed(hwnd: isize) {
    if LOADED.load(Ordering::Acquire) == 0 {
        return;
    }
    let p = f(F_RETURN);
    if p != 0 {
        // SAFETY: UiaReturnRawElementProvider(hwnd, 0, 0, NULL): the documented release call.
        let ret: unsafe extern "system" fn(isize, usize, isize, This) -> isize = unsafe { core::mem::transmute(p) };
        unsafe { ret(hwnd, 0, 0, core::ptr::null_mut()) };
    }
}

fn listening() -> bool {
    if LOADED.load(Ordering::Acquire) == 0 {
        return false; // no client asked for a window yet
    }
    let p = f(F_LISTENING);
    // SAFETY: UiaClientsAreListening() -> BOOL.
    p != 0 && unsafe { core::mem::transmute::<usize, unsafe extern "system" fn() -> i32>(p)() } != 0
}

fn raise(e: *mut Elem, id: i32) {
    let p = f(F_RAISE);
    if p != 0 {
        // SAFETY: UiaRaiseAutomationEvent(IRawElementProviderSimple*, EVENTID) -> HRESULT.
        let r: unsafe extern "system" fn(This, i32) -> Hr = unsafe { core::mem::transmute(p) };
        unsafe { r(iface(e, O_SIMPLE), id) };
    }
}

fn raise_prop(e: *mut Elem, id: i32, new: Variant) {
    let p = f(F_RAISE_PROP);
    if p != 0 {
        // SAFETY: UiaRaiseAutomationPropertyChangedEvent(IRawElementProviderSimple*, PROPERTYID,
        // VARIANT, VARIANT) -> HRESULT; the VARIANTs are copied by value.
        let r: unsafe extern "system" fn(This, i32, Variant, Variant) -> Hr = unsafe { core::mem::transmute(p) };
        unsafe { r(iface(e, O_SIMPLE), id, Variant::default(), new) };
    }
    if new.vt == VT_BSTR {
        bstr_free(new.v[0] as usize);
    }
}

pub fn changed(s: &Snapshot, c: Vec<Change>) {
    if !listening() {
        return;
    }
    let mut parents: Vec<(u8, u64)> = Vec::new();
    for ch in c {
        let (w, key) = match &ch {
            Change::Focus(w, k)
            | Change::State(w, k, ..)
            | Change::Name(w, k)
            | Change::Value(w, k)
            | Change::Text(w, k, _)
            | Change::Caret(w, k, _) => (*w, *k),
            Change::Added(w, p, _) | Change::Removed(w, p, ..) => {
                parents.push((*w, *p));
                continue;
            }
            _ => continue,
        };
        let Some(t) = s.win(w) else { continue };
        let Some(n) = t.find(key).map(|k| &t.nodes[k]) else { continue };
        let hwnd = win_hwnd(t.pw);
        if hwnd == 0 {
            continue;
        }
        let e = new_elem(hwnd, t.pw, key);
        match ch {
            Change::Focus(..) => raise(e, EV_FOCUS),
            Change::State(_, _, bit, on) => match bit {
                ax::S_CHECKED | ax::S_MIXED | ax::S_PRESSED if n.role == Role::Radio => {
                    raise_prop(e, PR_SELECTED, Variant::bool(on));
                    if on {
                        raise(e, EV_SELECTED);
                    }
                }
                ax::S_CHECKED | ax::S_MIXED | ax::S_PRESSED => raise_prop(e, PR_TOGGLE_STATE, Variant::i4(toggle_state(n))),
                ax::S_SELECTED => {
                    raise_prop(e, PR_SELECTED, Variant::bool(on));
                    if on {
                        raise(e, EV_SELECTED);
                    }
                }
                ax::S_EXPANDED => raise_prop(e, PR_EXPAND_STATE, Variant::i4(on as i32)),
                ax::S_DISABLED => raise_prop(e, PR_ENABLED, Variant::bool(!on)),
                _ => {}
            },
            Change::Name(..) => raise_prop(e, PR_NAME, Variant::bstr(&n.name)),
            Change::Value(..) if n.role == Role::Heading => raise_prop(e, PR_HEADING_LEVEL, Variant::i4(HEADING_LEVEL_NONE + n.level() as i32)),
            Change::Value(..) => raise_prop(e, PR_RANGE_VALUE, Variant::r8(n.value)),
            Change::Text(..) => {
                raise_prop(e, PR_VALUE, Variant::bstr(&n.text));
                if n.role == Role::TextBox {
                    raise(e, EV_TEXT_CHANGED);
                }
            }
            // the caret moved (or the box got the focus): its caret range follows
            Change::Caret(..) if n.role == Role::TextBox => raise(e, EV_TEXT_SELECTION),
            _ => {}
        }
        release(e);
    }
    parents.sort_unstable();
    parents.dedup();
    let p = f(F_RAISE_STRUCT);
    for (w, key) in parents {
        let Some(t) = s.win(w) else { continue };
        let hwnd = win_hwnd(t.pw);
        if p == 0 || hwnd == 0 {
            continue;
        }
        let e = new_elem(hwnd, t.pw, key);
        let id = runtime_id(t.pw, key);
        // SAFETY: UiaRaiseStructureChangedEvent(IRawElementProviderSimple*,
        // StructureChangeType, int*, int) -> HRESULT; ChildrenInvalidated (2) with the parent's
        // runtime id (none for the root).
        let r: unsafe extern "system" fn(This, i32, *const i32, i32) -> Hr = unsafe { core::mem::transmute(p) };
        unsafe { r(iface(e, O_SIMPLE), 2, if key == 0 { core::ptr::null() } else { id.as_ptr() }, if key == 0 { 0 } else { 4 }) };
        release(e);
    }
}

// ---------------------------------------------------------------- a client (tests)

const CLSID_CUIAUTOMATION: Guid = guid(0xff48dba4, 0x60ef, 0x4201, [0xaa, 0x87, 0x54, 0x10, 0x3e, 0xef, 0x59, 0x4e]);
const IID_IUIAUTOMATION: Guid = guid(0x30cbe57d, 0xd9d0, 0x452a, [0xab, 0x13, 0x7a, 0xc5, 0xac, 0x48, 0x25, 0xee]);

/// (tests) one element as a UI Automation client sees it
#[derive(Clone, Debug, Default)]
pub struct UiaTestElem {
    pub name: String,
    pub control_type: i32,
    /// LocalizedControlType (UIA's own name of the control type unless the provider gives one)
    pub localized_type: String,
    /// below the window's element (1 = a child of it)
    pub depth: u32,
    pub enabled: bool,
    pub focused: bool,
    /// Toggle state (-1: no Toggle pattern)
    pub toggle: i32,
    /// Value (None: no Value pattern)
    pub value: Option<String>,
    /// RangeValue: value, minimum, maximum
    pub range: Option<(f64, f64, f64)>,
    /// SelectionItem: selected
    pub selected: Option<bool>,
    /// ExpandCollapse state (-1: no pattern)
    pub expand: i32,
    /// HeadingLevel 1 ..= 9 (0: HeadingLevel_None)
    pub heading: i32,
    /// the bounding rectangle: x, y, w, h (screen px)
    pub rect: [i32; 4],
}

/// (tests) what uia_test_act does to an element
#[derive(Clone, Debug, PartialEq)]
pub enum UiaTestAct {
    Invoke,
    Toggle,
    Select,
    Expand,
    Collapse,
    Focus,
    SetValue(String),
    SetRange(f64),
}

/// (tests; UI thread) the HWND of platform window pw
pub fn uia_test_hwnd(pw: i32) -> isize {
    win_hwnd(pw)
}

// the function in vtable slot k of COM object o
//
// SAFETY: o is a live COM interface pointer whose vtable has more than k entries.
unsafe fn vf(o: usize, k: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(k) }
}

fn com_release(o: usize) {
    if o != 0 {
        // SAFETY: IUnknown::Release on a pointer we hold a reference of.
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> u32>(vf(o, 2))(o) };
    }
}

// IUIAutomation in a multithreaded apartment of this thread (0: unavailable)
fn automation() -> usize {
    let (ci, cc) = (f(F_COINIT), f(F_COCREATE));
    if ci == 0 || cc == 0 {
        return 0;
    }
    let mut ua = 0usize;
    // SAFETY: CoInitializeEx(NULL, COINIT_MULTITHREADED) (S_FALSE: already); CoCreateInstance(
    // REFCLSID, LPUNKNOWN, DWORD, REFIID, LPVOID*) -> HRESULT.
    unsafe {
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, u32) -> Hr>(ci)(0, 0);
        let create: unsafe extern "system" fn(*const Guid, usize, u32, *const Guid, *mut usize) -> Hr = core::mem::transmute(cc);
        if create(&CLSID_CUIAUTOMATION, 0, 1, &IID_IUIAUTOMATION, &mut ua) < 0 {
            return 0;
        }
    }
    ua
}

// IUIAutomationElement::GetCurrentPropertyValue (slot 10) -> the VARIANT (cleared by the caller)
fn get_prop(el: usize, id: i32) -> Variant {
    let mut v = Variant::default();
    // SAFETY: GetCurrentPropertyValue(PROPERTYID, VARIANT*) -> HRESULT.
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, *mut Variant) -> Hr>(vf(el, 10))(el, id, &mut v) };
    v
}

fn variant_clear(v: &mut Variant) {
    let p = f(F_VARIANT_CLEAR);
    if p != 0 {
        // SAFETY: VariantClear(VARIANTARG*) frees what the VARIANT owns.
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(*mut Variant) -> Hr>(p)(v) };
    }
}

fn prop_str(el: usize, id: i32) -> Option<String> {
    let mut v = get_prop(el, id);
    // SAFETY: a VT_BSTR VARIANT holds a BSTR (NUL-terminated).
    let s = (v.vt == VT_BSTR).then(|| unsafe { wide_str(v.v[0] as *const u16) });
    variant_clear(&mut v);
    s
}

fn prop_i(el: usize, id: i32) -> Option<i32> {
    let v = get_prop(el, id);
    match v.vt {
        VT_I4 => Some(v.v[0] as u32 as i32),
        VT_BOOL => Some((v.v[0] as u16 != 0) as i32),
        _ => None,
    }
}

fn prop_f(el: usize, id: i32) -> Option<f64> {
    let v = get_prop(el, id);
    (v.vt == VT_R8).then(|| f64::from_bits(v.v[0]))
}

// the raw view walker's slot k (4 first child, 6 next sibling) of el -> element or 0
fn walk(walker: usize, k: usize, el: usize) -> usize {
    let mut out = 0usize;
    // SAFETY: IUIAutomationTreeWalker::GetFirstChildElement / GetNextSiblingElement(
    // IUIAutomationElement*, IUIAutomationElement**) -> HRESULT.
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, *mut usize) -> Hr>(vf(walker, k))(walker, el, &mut out) };
    out
}

// every element below el (depth first), with its depth; the caller releases them
fn collect(walker: usize, el: usize, depth: u32, out: &mut Vec<(usize, u32)>) {
    let mut c = walk(walker, 4, el);
    while c != 0 && out.len() < 2000 {
        out.push((c, depth));
        collect(walker, c, depth + 1, out);
        c = walk(walker, 6, c);
    }
}

fn describe(el: usize, depth: u32) -> UiaTestElem {
    let has = |id: i32| prop_i(el, id).unwrap_or(0) != 0;
    UiaTestElem {
        name: prop_str(el, PR_NAME).unwrap_or_default(),
        control_type: prop_i(el, PR_CONTROL_TYPE).unwrap_or(0),
        localized_type: prop_str(el, PR_LOCALIZED_TYPE).unwrap_or_default(),
        depth,
        enabled: has(PR_ENABLED),
        focused: has(PR_HAS_FOCUS),
        // IsTogglePatternAvailable 30041, IsValuePatternAvailable 30043, IsRangeValuePatternAvailable
        // 30033, IsSelectionItemPatternAvailable 30036, IsExpandCollapsePatternAvailable 30028
        toggle: if has(30041) { prop_i(el, PR_TOGGLE_STATE).unwrap_or(-1) } else { -1 },
        value: if has(30043) { prop_str(el, PR_VALUE) } else { None },
        range: if has(30033) {
            Some((prop_f(el, PR_RANGE_VALUE).unwrap_or(-1.0), prop_f(el, PR_RANGE_MIN).unwrap_or(-1.0), prop_f(el, PR_RANGE_MAX).unwrap_or(-1.0)))
        } else {
            None
        },
        selected: if has(30036) { Some(has(PR_SELECTED)) } else { None },
        expand: if has(30028) { prop_i(el, PR_EXPAND_STATE).unwrap_or(-1) } else { -1 },
        heading: prop_i(el, PR_HEADING_LEVEL).map_or(0, |v| (v - HEADING_LEVEL_NONE).clamp(0, 9)),
        rect: {
            let mut r = Rect { l: 0, t: 0, r: 0, b: 0 };
            // SAFETY: IUIAutomationElement::get_CurrentBoundingRectangle(RECT*) (slot 43).
            unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut Rect) -> Hr>(vf(el, 43))(el, &mut r) };
            [r.l, r.t, r.r - r.l, r.b - r.t]
        },
    }
}

// f(elements) with the elements below the window's element (released afterwards)
fn with_tree<R>(hwnd: isize, f: impl FnOnce(&[(usize, u32)]) -> R) -> Option<R> {
    let ua = automation();
    if ua == 0 {
        return None;
    }
    let (mut win, mut walker) = (0usize, 0usize);
    // SAFETY: IUIAutomation::ElementFromHandle(UIA_HWND, IUIAutomationElement**) (slot 6) and
    // get_RawViewWalker(IUIAutomationTreeWalker**) (slot 16).
    unsafe {
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, isize, *mut usize) -> Hr>(vf(ua, 6))(ua, hwnd, &mut win);
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut usize) -> Hr>(vf(ua, 16))(ua, &mut walker);
    }
    let mut all = Vec::new();
    if win != 0 && walker != 0 {
        collect(walker, win, 1, &mut all);
    }
    let r = f(&all);
    for (el, _) in all {
        com_release(el);
    }
    com_release(walker);
    com_release(win);
    com_release(ua);
    Some(r)
}

/// (tests; not on the UI thread) the elements below window hwnd as a UI Automation client sees
/// them (the raw view, depth first); None: no UI Automation
pub fn uia_test_tree(hwnd: isize) -> Option<Vec<UiaTestElem>> {
    with_tree(hwnd, |all| all.iter().map(|&(el, d)| describe(el, d)).collect())
}

/// (tests; not on the UI thread) act on the first element with this name and control type
/// through its pattern -> the call succeeded
pub fn uia_test_act(hwnd: isize, name: &str, control_type: i32, a: UiaTestAct) -> bool {
    with_tree(hwnd, |all| {
        let Some(&(el, _)) = all.iter().find(|&&(el, _)| prop_str(el, PR_NAME).as_deref() == Some(name) && prop_i(el, PR_CONTROL_TYPE) == Some(control_type)) else {
            return false;
        };
        if a == UiaTestAct::Focus {
            // SAFETY: IUIAutomationElement::SetFocus (slot 3).
            return unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(vf(el, 3))(el) } >= 0;
        }
        let pid = match a {
            UiaTestAct::Invoke => P_INVOKE,
            UiaTestAct::Toggle => P_TOGGLE,
            UiaTestAct::Select => P_SELITEM,
            UiaTestAct::Expand | UiaTestAct::Collapse => P_EXPAND,
            UiaTestAct::SetValue(_) => P_VALUE,
            UiaTestAct::SetRange(_) => P_RANGE,
            UiaTestAct::Focus => 0,
        };
        let mut p = 0usize;
        // SAFETY: IUIAutomationElement::GetCurrentPattern(PATTERNID, IUnknown**) (slot 16): the
        // pattern interface; its first method (slot 3) is Invoke / Toggle / SetValue / Select /
        // Expand, Collapse is slot 4 of IUIAutomationExpandCollapsePattern.
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, *mut usize) -> Hr>(vf(el, 16))(el, pid, &mut p);
            if p == 0 {
                return false;
            }
            let hr = match &a {
                UiaTestAct::SetValue(s) => {
                    let b = bstr(s);
                    let hr = core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize) -> Hr>(vf(p, 3))(p, b);
                    bstr_free(b);
                    hr
                }
                UiaTestAct::SetRange(v) => core::mem::transmute::<usize, unsafe extern "system" fn(usize, f64) -> Hr>(vf(p, 3))(p, *v),
                UiaTestAct::Collapse => core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(vf(p, 4))(p),
                _ => core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(vf(p, 3))(p),
            };
            com_release(p);
            hr >= 0
        }
    })
    .unwrap_or(false)
}

// ---- the caret (tests): TextPattern2 of a client, the system caret

const IID_TEXTPATTERN2: Guid = guid(0x506a921a, 0xfcc9, 0x409f, [0xb2, 0x3b, 0x37, 0xeb, 0x74, 0x10, 0x68, 0x72]);

/// (tests) the caret of a text box as a UI Automation client sees it
#[derive(Clone, Debug, Default)]
pub struct UiaTestCaret {
    /// GetCaretRange: the caret is active (the box has the keyboard focus)
    pub active: bool,
    /// the caret range's bounding rectangles (screen px: x, y, w, h each)
    pub rects: Vec<[f64; 4]>,
    /// the text of the document range and of the selection
    pub text: String,
    pub selected: String,
}

// the elements of a SAFEARRAY of VT_R8 (destroyed afterwards)
fn sa_doubles(sa: *mut u8) -> Vec<f64> {
    let mut out = Vec::new();
    if sa.is_null() {
        return out;
    }
    let (acc, un, des) = (f(F_SA_ACCESS), f(F_SA_UNACCESS), f(F_SA_DESTROY));
    // SAFETY: a one-dimensional SAFEARRAY (x64: cElements of rgsabound[0] at offset 24) of 8-byte
    // VT_R8 elements, read between AccessData and UnaccessData, then destroyed.
    unsafe {
        let n = *(sa.add(24) as *const u32) as usize;
        let mut d: *mut f64 = core::ptr::null_mut();
        if acc != 0 && un != 0 {
            let access: unsafe extern "system" fn(*mut u8, *mut *mut f64) -> Hr = core::mem::transmute(acc);
            let unaccess: unsafe extern "system" fn(*mut u8) -> Hr = core::mem::transmute(un);
            if access(sa, &mut d) >= 0 && !d.is_null() {
                out.extend_from_slice(core::slice::from_raw_parts(d, n));
                unaccess(sa);
            }
        }
        if des != 0 {
            core::mem::transmute::<usize, unsafe extern "system" fn(*mut u8) -> Hr>(des)(sa);
        }
    }
    out
}

// IUIAutomationTextRange::GetText(-1) (slot 12)
fn range_text(r: usize) -> String {
    let mut b = 0usize;
    // SAFETY: GetText(int, BSTR*) -> HRESULT; the BSTR is ours to free.
    unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, *mut usize) -> Hr>(vf(r, 12))(r, -1, &mut b) };
    // SAFETY: a BSTR (NUL-terminated) or null.
    let t = unsafe { wide_str(b as *const u16) };
    bstr_free(b);
    t
}

// f(text pattern 2) of the first text box with this name below window hwnd
fn with_text_pattern<R>(hwnd: isize, name: &str, f: impl FnOnce(usize) -> R) -> Option<R> {
    with_tree(hwnd, |all| {
        let &(el, _) = all.iter().find(|&&(el, _)| prop_str(el, PR_NAME).as_deref() == Some(name) && prop_i(el, PR_CONTROL_TYPE) == Some(CT_EDIT))?;
        let mut tp = 0usize;
        // SAFETY: IUIAutomationElement::GetCurrentPatternAs(PATTERNID, REFIID, void**) (slot 14).
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, *const Guid, *mut usize) -> Hr>(vf(el, 14))(
                el,
                P_TEXT2,
                &IID_TEXTPATTERN2,
                &mut tp,
            )
        };
        if tp == 0 {
            return None;
        }
        let r = f(tp);
        com_release(tp);
        Some(r)
    })
    .flatten()
}

/// (tests; not on the UI thread) the caret of the first text box with this name through
/// IUIAutomationTextPattern2: GetCaretRange and its rectangles, the document's text, the
/// selection's text; None: no such box / no Text pattern
pub fn uia_test_caret(hwnd: isize, name: &str) -> Option<UiaTestCaret> {
    with_text_pattern(hwnd, name, |tp| {
        let mut c = UiaTestCaret::default();
        let (mut active, mut cr, mut doc, mut sel) = (0i32, 0usize, 0usize, 0usize);
        // SAFETY: IUIAutomationTextPattern2 GetCaretRange(BOOL*, IUIAutomationTextRange**) (slot
        // 10), get_DocumentRange (slot 7), GetSelection(IUIAutomationTextRangeArray**) (slot 5);
        // IUIAutomationTextRange::GetBoundingRectangles(SAFEARRAY**) (slot 10);
        // IUIAutomationTextRangeArray get_Length (slot 3), GetElement (slot 4).
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut i32, *mut usize) -> Hr>(vf(tp, 10))(tp, &mut active, &mut cr);
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut usize) -> Hr>(vf(tp, 7))(tp, &mut doc);
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut usize) -> Hr>(vf(tp, 5))(tp, &mut sel);
            c.active = active != 0;
            if cr != 0 {
                let mut sa: *mut u8 = core::ptr::null_mut();
                core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut *mut u8) -> Hr>(vf(cr, 10))(cr, &mut sa);
                c.rects = sa_doubles(sa).chunks_exact(4).map(|q| [q[0], q[1], q[2], q[3]]).collect();
            }
            if doc != 0 {
                c.text = range_text(doc);
            }
            if sel != 0 {
                let mut n = 0i32;
                core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut i32) -> Hr>(vf(sel, 3))(sel, &mut n);
                if n > 0 {
                    let mut r = 0usize;
                    core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, *mut usize) -> Hr>(vf(sel, 4))(sel, 0, &mut r);
                    if r != 0 {
                        c.selected = range_text(r);
                        com_release(r);
                    }
                }
            }
        }
        com_release(cr);
        com_release(doc);
        com_release(sel);
        c
    })
}

/// (tests; not on the UI thread) move the caret of the first text box with this name: its caret
/// range moved by `chars` characters (ITextRangeProvider::Move), then, with `extend` > 0, its end
/// moved `extend` more characters, and Select() -> the calls succeeded
pub fn uia_test_caret_move(hwnd: isize, name: &str, chars: i32, extend: i32) -> bool {
    with_text_pattern(hwnd, name, |tp| {
        let (mut active, mut cr, mut moved) = (0i32, 0usize, 0i32);
        // SAFETY: GetCaretRange (slot 10); IUIAutomationTextRange Move(TextUnit, int, int*) (slot
        // 13), MoveEndpointByUnit(endpoint, unit, count, int*) (slot 14), Select (slot 16).
        unsafe {
            core::mem::transmute::<usize, unsafe extern "system" fn(usize, *mut i32, *mut usize) -> Hr>(vf(tp, 10))(tp, &mut active, &mut cr);
            if cr == 0 {
                return false;
            }
            let mut ok = core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, i32, *mut i32) -> Hr>(vf(cr, 13))(cr, 0, chars, &mut moved) >= 0;
            if extend > 0 {
                ok &= core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, i32, i32, *mut i32) -> Hr>(vf(cr, 14))(cr, 1, 0, extend, &mut moved)
                    >= 0;
            }
            ok &= core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(vf(cr, 16))(cr) >= 0;
            com_release(cr);
            ok
        }
    })
    .unwrap_or(false)
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetGUIThreadInfo(tid: u32, gi: *mut GuiThreadInfo) -> i32;
    fn GetWindowThreadProcessId(h: isize, pid: *mut u32) -> u32;
}

#[repr(C)]
#[derive(Default)]
struct GuiThreadInfo {
    size: u32,
    flags: u32,
    active: isize,
    focus: isize,
    capture: isize,
    menu_owner: isize,
    move_size: isize,
    caret: isize,
    rc: [i32; 4],
}

/// (tests; any thread) the system caret of hwnd's thread as magnifiers read it
/// (GetGUIThreadInfo): its window and its rectangle on the screen (left, top, right, bottom)
pub fn uia_test_system_caret(hwnd: isize) -> Option<(isize, [i32; 4])> {
    let mut gi = GuiThreadInfo { size: core::mem::size_of::<GuiThreadInfo>() as u32, ..Default::default() };
    // SAFETY: a window of ours; GUITHREADINFO with its size set.
    let ok = unsafe { GetGUIThreadInfo(GetWindowThreadProcessId(hwnd, core::ptr::null_mut()), &mut gi) } != 0;
    if !ok || gi.caret == 0 {
        return None;
    }
    let (mut a, mut b) = (Point { x: gi.rc[0], y: gi.rc[1] }, Point { x: gi.rc[2], y: gi.rc[3] });
    // SAFETY: the caret's window (client coordinates) and valid POINTs.
    unsafe {
        ClientToScreen(gi.caret, &mut a);
        ClientToScreen(gi.caret, &mut b);
    }
    Some((gi.caret, [a.x, a.y, b.x, b.y]))
}

// ---- events (tests): focus changed and property changed handlers of a client

static EVENTS: SpinLock<Vec<String>> = SpinLock::new(Vec::new());
// the IUIAutomation the handlers are registered with
static LISTENER: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
struct VtFocusHandler {
    qi: Qi,
    add: Ref,
    rel: Ref,
    handle: unsafe extern "system" fn(This, usize) -> Hr,
}

#[repr(C)]
struct VtPropHandler {
    qi: Qi,
    add: Ref,
    rel: Ref,
    handle: unsafe extern "system" fn(This, usize, i32, Variant) -> Hr,
}

#[repr(C)]
struct VtEventHandler {
    qi: Qi,
    add: Ref,
    rel: Ref,
    handle: unsafe extern "system" fn(This, usize, i32) -> Hr,
}

// a static COM object: the pointer to its vtable
#[repr(C)]
struct Handler(*const u8);
// SAFETY: the object is immutable (a pointer to a static vtable).
unsafe impl Sync for Handler {}

static VT_FOCUS_HANDLER: VtFocusHandler = VtFocusHandler { qi: h_qi, add: h_ref, rel: h_ref, handle: h_focus };
static VT_PROP_HANDLER: VtPropHandler = VtPropHandler { qi: h_qi, add: h_ref, rel: h_ref, handle: h_prop };
static VT_EVENT_HANDLER: VtEventHandler = VtEventHandler { qi: h_qi, add: h_ref, rel: h_ref, handle: h_event };
static FOCUS_HANDLER: Handler = Handler(&VT_FOCUS_HANDLER as *const VtFocusHandler as *const u8);
static PROP_HANDLER: Handler = Handler(&VT_PROP_HANDLER as *const VtPropHandler as *const u8);
static EVENT_HANDLER: Handler = Handler(&VT_EVENT_HANDLER as *const VtEventHandler as *const u8);

// SAFETY (the handler methods): this is one of the static handlers, out is valid for a write,
// sender is an IUIAutomationElement UIA holds for the call.
const IID_FOCUS_HANDLER: Guid = guid(0xc270f6b5, 0x5c69, 0x4290, [0x97, 0x45, 0x7a, 0x7f, 0x97, 0x16, 0x94, 0x68]);
const IID_PROP_HANDLER: Guid = guid(0x40cd37d4, 0xc756, 0x4b0c, [0x8c, 0x6f, 0xbd, 0xdf, 0xee, 0xb1, 0x3b, 0x50]);
const IID_EVENT_HANDLER: Guid = guid(0x146c3c17, 0xf12e, 0x4e22, [0x8c, 0x27, 0xf8, 0x94, 0xb9, 0xb7, 0x9c, 0x69]);
const IID_AGILE: Guid = guid(0x94ea2b94, 0xe9cc, 0x49e0, [0xc0, 0xff, 0xee, 0x64, 0xca, 0x8f, 0x5b, 0x90]);

unsafe extern "system" fn h_qi(this: This, riid: *const Guid, out: *mut This) -> Hr {
    let id = unsafe { *riid };
    // IUnknown, the handler interface, IAgileObject (free-threaded)
    let ok = id == IID_UNKNOWN || id == IID_AGILE || id == IID_FOCUS_HANDLER || id == IID_PROP_HANDLER || id == IID_EVENT_HANDLER;
    unsafe { *out = if ok { this } else { core::ptr::null_mut() } };
    if ok { S_OK } else { E_NOINTERFACE }
}

unsafe extern "system" fn h_ref(_this: This) -> u32 {
    1 // static
}

unsafe extern "system" fn h_focus(_this: This, sender: usize) -> Hr {
    let name = prop_str(sender, PR_NAME).unwrap_or_default();
    EVENTS.lock().push(alloc::format!("focus:{name}"));
    S_OK
}

unsafe extern "system" fn h_prop(_this: This, sender: usize, id: i32, v: Variant) -> Hr {
    let name = prop_str(sender, PR_NAME).unwrap_or_default();
    let value = match v.vt {
        VT_I4 => alloc::format!("{}", v.v[0] as u32 as i32),
        VT_BOOL => alloc::format!("{}", (v.v[0] as u16 != 0) as i32),
        VT_R8 => alloc::format!("{}", f64::from_bits(v.v[0])),
        // SAFETY: a VT_BSTR VARIANT holds a BSTR.
        VT_BSTR => unsafe { wide_str(v.v[0] as *const u16) },
        _ => String::new(),
    };
    EVENTS.lock().push(alloc::format!("prop:{id}:{name}={value}"));
    S_OK
}

unsafe extern "system" fn h_event(_this: This, sender: usize, id: i32) -> Hr {
    let name = prop_str(sender, PR_NAME).unwrap_or_default();
    EVENTS.lock().push(alloc::format!("event:{id}:{name}"));
    S_OK
}

/// (tests; not on the UI thread) listen to focus changes everywhere, to the property changes
/// below window hwnd (toggle state, selected, expand state, enabled, name, value, range value)
/// and to its text events (the caret moved: TextSelectionChanged, TextChanged) -> registered
pub fn uia_test_listen(hwnd: isize) -> bool {
    let ua = automation();
    if ua == 0 {
        return false;
    }
    let mut win = 0usize;
    let props = [PR_TOGGLE_STATE, PR_SELECTED, PR_EXPAND_STATE, PR_ENABLED, PR_NAME, PR_VALUE, PR_RANGE_VALUE];
    // SAFETY: IUIAutomation::ElementFromHandle (slot 6), AddFocusChangedEventHandler(
    // IUIAutomationCacheRequest*, IUIAutomationFocusChangedEventHandler*) (slot 39) and
    // AddPropertyChangedEventHandlerNativeArray(IUIAutomationElement*, TreeScope,
    // IUIAutomationCacheRequest*, IUIAutomationPropertyChangedEventHandler*, PROPERTYID*, int)
    // (slot 34; TreeScope_Subtree 7); the handlers are static.
    let ok = unsafe {
        core::mem::transmute::<usize, unsafe extern "system" fn(usize, isize, *mut usize) -> Hr>(vf(ua, 6))(ua, hwnd, &mut win);
        let a = core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, *const Handler) -> Hr>(vf(ua, 39))(ua, 0, &FOCUS_HANDLER);
        let b = core::mem::transmute::<usize, unsafe extern "system" fn(usize, usize, i32, usize, *const Handler, *const i32, i32) -> Hr>(vf(ua, 34))(
            ua,
            win,
            7,
            0,
            &PROP_HANDLER,
            props.as_ptr(),
            props.len() as i32,
        );
        // AddAutomationEventHandler(EVENTID, IUIAutomationElement*, TreeScope,
        // IUIAutomationCacheRequest*, IUIAutomationEventHandler*) (slot 32)
        let mut c = 0;
        for ev in [EV_TEXT_SELECTION, EV_TEXT_CHANGED] {
            c |= core::mem::transmute::<usize, unsafe extern "system" fn(usize, i32, usize, i32, usize, *const Handler) -> Hr>(vf(ua, 32))(
                ua,
                ev,
                win,
                7,
                0,
                &EVENT_HANDLER,
            )
            .min(0);
        }
        if a < 0 || b < 0 || c < 0 {
            crate::write_stdout(alloc::format!("  uia_test_listen: {a:x} {b:x} {c:x}\n").as_bytes());
        }
        a >= 0 && b >= 0 && c >= 0
    };
    com_release(win);
    LISTENER.store(ua, Ordering::Release);
    ok
}

/// (tests) the events since the last call: "focus:<name>", "prop:<id>:<name>=<value>",
/// "event:<id>:<name>"
pub fn uia_test_events() -> Vec<String> {
    core::mem::take(&mut *EVENTS.lock())
}

/// (tests; the thread of uia_test_listen) remove the handlers
pub fn uia_test_unlisten() {
    let ua = LISTENER.swap(0, Ordering::AcqRel);
    if ua != 0 {
        // SAFETY: IUIAutomation::RemoveAllEventHandlers (slot 41).
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(usize) -> Hr>(vf(ua, 41))(ua) };
        com_release(ua);
    }
}
