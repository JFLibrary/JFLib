// win_uia_text -- (win_uia.rs) the Text and Text2 patterns of text boxes: ITextProvider2 on the
// element, ITextRangeProvider objects for its ranges
//
// Screen readers (NVDA, Narrator) read and follow the caret of a text box through these, Windows
// Magnifier also follows the caret range (and the hidden system caret, win_clip.rs ime_pos).
// A range is (window, node key, start, end) in characters of the node's text; like the elements
// it answers from a11y::current() on whatever thread UIA calls it.  Units: character, word (a
// word with the blanks after it; format goes up to word), line and paragraph (up to a '\n'),
// page and document (the whole text).  Rectangles come from Node::char_rect: exact at the caret
// (the text box reports it), estimated elsewhere.  GetCaretRange / GetSelection give the caret
// and the selection of the focused box; Select sets them (a11y::Act::Select).  win_uia.rs raises
// TextSelectionChanged when the caret moves and TextChanged with the text.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use crate::a11y::{self as ax, Act, Node, Request, Role};

use super::{
    E_INVALIDARG, E_NOINTERFACE, F_NOTSUP, F_SA_ACCESS, F_SA_CREATE, F_SA_UNACCESS, Guid, Hr, IID_UNKNOWN, MI32, MOut,
    O_SIMPLE, O_TEXT, Qi, Ref, S_OK, This, UIA_E_ELEMENTNOTAVAILABLE, UIA_E_ELEMENTNOTENABLED, UIA_E_INVALIDOPERATION, VT_BOOL,
    Variant, add, bstr, elem, f, guid, iface, look_key, new_elem, origin, qi, rel, wide_str,
};

pub(super) const P_TEXT: i32 = 10014;
pub(super) const P_TEXT2: i32 = 10024;
pub(super) const IID_TEXT: Guid = guid(0x3589c92c, 0x63f3, 0x4367, [0x99, 0xbb, 0xad, 0xa6, 0x53, 0xb7, 0x7c, 0xf2]);
pub(super) const IID_TEXT2: Guid = guid(0x0dc5e6ed, 0x3e16, 0x4bf1, [0x8f, 0x9a, 0xa9, 0x79, 0x87, 0x8b, 0xc1, 0x95]);
const IID_RANGE: Guid = guid(0x5347ad7b, 0xc355, 0x46f8, [0xaf, 0xf5, 0x90, 0x90, 0x33, 0x58, 0x2f, 0x63]);
/// UIA_Text_TextSelectionChangedEventId, UIA_Text_TextChangedEventId
pub(super) const EV_TEXT_SELECTION: i32 = 20014;
pub(super) const EV_TEXT_CHANGED: i32 = 20015;
const ATTR_READ_ONLY: i32 = 40015;
const VT_UNKNOWN: u16 = 13;
const VT_R8: u16 = 5;
const UNIT_CHAR: i32 = 0;
const UNIT_WORD: i32 = 2;
const UNIT_LINE: i32 = 3;
const UNIT_PARAGRAPH: i32 = 4;

/// UiaPoint (screen px)
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct UiaPoint {
    x: f64,
    y: f64,
}

// ---------------------------------------------------------------- the element's ITextProvider2

#[repr(C)]
pub(super) struct VtText {
    qi: Qi,
    add: Ref,
    rel: Ref,
    selection: unsafe extern "system" fn(This, *mut *mut u8) -> Hr,
    visible: unsafe extern "system" fn(This, *mut *mut u8) -> Hr,
    from_child: unsafe extern "system" fn(This, This, *mut This) -> Hr,
    from_point: unsafe extern "system" fn(This, UiaPoint, *mut This) -> Hr,
    document: MOut,
    supported: MI32,
    from_annotation: unsafe extern "system" fn(This, This, *mut This) -> Hr,
    caret: unsafe extern "system" fn(This, *mut i32, *mut This) -> Hr,
}

pub(super) static VT_TEXT: VtText = VtText {
    qi: qi::<O_TEXT>,
    add: add::<O_TEXT>,
    rel: rel::<O_TEXT>,
    selection: tp_selection,
    visible: tp_visible,
    from_child: tp_from_child,
    from_point: tp_from_point,
    document: tp_document,
    supported: tp_supported,
    from_annotation: tp_from_annotation,
    caret: tp_caret,
};

// the text of a node as characters
fn chars(n: &Node) -> Vec<char> {
    n.text.chars().collect()
}

// f(node) for the text box node of element e (None: gone)
fn with_box<R>(pw: i32, key: u64, f: impl FnOnce(&ax::WinTree, &Node) -> R) -> Option<R> {
    let (s, wi, k) = look_key(pw, key)?;
    let t = &s.wins[wi];
    t.nodes.get(k as usize).filter(|n| n.role == Role::TextBox).map(|n| f(t, n))
}

// a SAFEARRAY (vector) of vt with these 8-byte elements (null when oleaut32 is missing)
fn sa_vector(vt: u16, v: &[u64]) -> *mut u8 {
    let (c, acc, un) = (f(F_SA_CREATE), f(F_SA_ACCESS), f(F_SA_UNACCESS));
    if c == 0 || acc == 0 || un == 0 {
        return core::ptr::null_mut();
    }
    // SAFETY: SafeArrayCreateVector(VARTYPE, LONG, ULONG) -> SAFEARRAY*; VT_R8 and VT_UNKNOWN
    // elements are 8 bytes; the data of the fresh vector is written between Access and Unaccess.
    unsafe {
        let create: unsafe extern "system" fn(u16, i32, u32) -> *mut u8 = core::mem::transmute(c);
        let access: unsafe extern "system" fn(*mut u8, *mut *mut u64) -> Hr = core::mem::transmute(acc);
        let unaccess: unsafe extern "system" fn(*mut u8) -> Hr = core::mem::transmute(un);
        let sa = create(vt, 0, v.len() as u32);
        if sa.is_null() || v.is_empty() {
            return sa;
        }
        let mut d: *mut u64 = core::ptr::null_mut();
        if access(sa, &mut d) >= 0 && !d.is_null() {
            core::ptr::copy_nonoverlapping(v.as_ptr(), d, v.len());
            unaccess(sa);
        }
        sa
    }
}

// SAFETY (all methods below): `this` is the interface pointer UIA got from us; out pointers are
// valid for one write, as the interface contracts say.

unsafe extern "system" fn tp_selection(this: This, out: *mut *mut u8) -> Hr {
    let e = unsafe { elem::<O_TEXT>(this) };
    let Some(sel) = with_box(e.pw, e.key, |_, n| (n.caret >= 0).then(|| n.selection())) else { return UIA_E_ELEMENTNOTAVAILABLE };
    let v: Vec<u64> = match sel {
        Some((a, b)) => alloc::vec![new_range(e.hwnd, e.pw, e.key, a, b) as u64],
        None => Vec::new(),
    };
    unsafe { *out = sa_vector(VT_UNKNOWN, &v) };
    S_OK
}

unsafe extern "system" fn tp_visible(this: This, out: *mut *mut u8) -> Hr {
    let e = unsafe { elem::<O_TEXT>(this) };
    let Some(n) = with_box(e.pw, e.key, |_, n| n.text.chars().count() as i32) else { return UIA_E_ELEMENTNOTAVAILABLE };
    unsafe { *out = sa_vector(VT_UNKNOWN, &[new_range(e.hwnd, e.pw, e.key, 0, n) as u64]) };
    S_OK
}

unsafe extern "system" fn tp_from_child(_this: This, _child: This, out: *mut This) -> Hr {
    unsafe { *out = core::ptr::null_mut() };
    E_INVALIDARG // a text box has no children
}

unsafe extern "system" fn tp_from_point(this: This, p: UiaPoint, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_TEXT>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let (ox, oy) = origin(e.hwnd);
    let Some(k) = with_box(e.pw, e.key, |_, n| n.offset_at(p.x as i32 - ox, p.y as i32 - oy)) else { return UIA_E_ELEMENTNOTAVAILABLE };
    unsafe { *out = new_range(e.hwnd, e.pw, e.key, k, k) };
    S_OK
}

unsafe extern "system" fn tp_document(this: This, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_TEXT>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let Some(n) = with_box(e.pw, e.key, |_, n| n.text.chars().count() as i32) else { return UIA_E_ELEMENTNOTAVAILABLE };
    unsafe { *out = new_range(e.hwnd, e.pw, e.key, 0, n) };
    S_OK
}

unsafe extern "system" fn tp_supported(_this: This, out: *mut i32) -> Hr {
    unsafe { *out = 1 }; // SupportedTextSelection_Single
    S_OK
}

unsafe extern "system" fn tp_from_annotation(_this: This, _a: This, out: *mut This) -> Hr {
    unsafe { *out = core::ptr::null_mut() };
    E_INVALIDARG // no annotations
}

unsafe extern "system" fn tp_caret(this: This, active: *mut i32, out: *mut This) -> Hr {
    let e = unsafe { elem::<O_TEXT>(this) };
    unsafe { *out = core::ptr::null_mut() };
    let Some((on, k)) = with_box(e.pw, e.key, |t, n| (t.active && n.state & ax::S_FOCUSED != 0 && n.caret >= 0, n.caret.max(0)))
    else {
        return UIA_E_ELEMENTNOTAVAILABLE;
    };
    unsafe {
        *active = on as i32;
        *out = new_range(e.hwnd, e.pw, e.key, k, k);
    }
    S_OK
}

// ---------------------------------------------------------------- ranges (ITextRangeProvider)

#[repr(C)]
struct VtRange {
    qi: Qi,
    add: Ref,
    rel: Ref,
    clone: MOut,
    compare: unsafe extern "system" fn(This, This, *mut i32) -> Hr,
    compare_endpoints: unsafe extern "system" fn(This, i32, This, i32, *mut i32) -> Hr,
    expand: unsafe extern "system" fn(This, i32) -> Hr,
    find_attribute: unsafe extern "system" fn(This, i32, Variant, i32, *mut This) -> Hr,
    find_text: unsafe extern "system" fn(This, *const u16, i32, i32, *mut This) -> Hr,
    attribute: unsafe extern "system" fn(This, i32, *mut Variant) -> Hr,
    rects: unsafe extern "system" fn(This, *mut *mut u8) -> Hr,
    enclosing: MOut,
    text: unsafe extern "system" fn(This, i32, *mut usize) -> Hr,
    move_: unsafe extern "system" fn(This, i32, i32, *mut i32) -> Hr,
    move_endpoint: unsafe extern "system" fn(This, i32, i32, i32, *mut i32) -> Hr,
    move_endpoint_range: unsafe extern "system" fn(This, i32, This, i32) -> Hr,
    select: unsafe extern "system" fn(This) -> Hr,
    add_select: unsafe extern "system" fn(This) -> Hr,
    remove_select: unsafe extern "system" fn(This) -> Hr,
    scroll: unsafe extern "system" fn(This, i32) -> Hr,
    children: unsafe extern "system" fn(This, *mut *mut u8) -> Hr,
}

static VT_RANGE: VtRange = VtRange {
    qi: r_qi,
    add: r_add,
    rel: r_rel,
    clone: r_clone,
    compare: r_compare,
    compare_endpoints: r_compare_endpoints,
    expand: r_expand,
    find_attribute: r_find_attribute,
    find_text: r_find_text,
    attribute: r_attribute,
    rects: r_rects,
    enclosing: r_enclosing,
    text: r_text,
    move_: r_move,
    move_endpoint: r_move_endpoint,
    move_endpoint_range: r_move_endpoint_range,
    select: r_select,
    add_select: r_add_select,
    remove_select: r_add_select,
    scroll: r_scroll,
    children: r_children,
};

/// a range of a text box: start and end in characters (the methods run on UIA's threads)
#[repr(C)]
struct Range {
    vt: *const VtRange,
    refs: AtomicU32,
    hwnd: isize,
    pw: i32,
    key: u64,
    a: AtomicI32,
    b: AtomicI32,
}

fn new_range(hwnd: isize, pw: i32, key: u64, a: i32, b: i32) -> This {
    let (a, b) = (a.min(b), a.max(b));
    Box::into_raw(Box::new(Range { vt: &VT_RANGE, refs: AtomicU32::new(1), hwnd, pw, key, a: AtomicI32::new(a), b: AtomicI32::new(b) }))
        as This
}

// the Range behind interface pointer this
//
// SAFETY: this is a pointer to a live Range (an ITextRangeProvider we made).
unsafe fn range<'a>(this: This) -> &'a Range {
    unsafe { &*(this as *const Range) }
}

// another ITextRangeProvider as one of our ranges (None: someone else's)
//
// SAFETY: p is null or a live COM interface pointer.
unsafe fn ours<'a>(p: This) -> Option<&'a Range> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the first field of every COM object is its vtable pointer.
    let vt = unsafe { *(p as *const *const VtRange) };
    if core::ptr::eq(vt, &VT_RANGE) { Some(unsafe { range(p) }) } else { None }
}

impl Range {
    fn get(&self) -> (i32, i32) {
        (self.a.load(Ordering::Acquire), self.b.load(Ordering::Acquire))
    }

    // both ends, ordered and inside 0 ..= n
    fn set(&self, a: i32, b: i32, n: i32) {
        let (a, b) = (a.clamp(0, n), b.clamp(0, n));
        self.a.store(a.min(b), Ordering::Release);
        self.b.store(a.max(b), Ordering::Release);
    }

    // f(node, its characters) (None: the node is gone)
    fn with<R>(&self, f: impl FnOnce(&Node, &[char]) -> R) -> Option<R> {
        with_box(self.pw, self.key, |_, n| f(n, &chars(n)))
    }
}

// a unit starts at character p (0 and n always)
fn boundary(c: &[char], unit: i32, p: i32) -> bool {
    let n = c.len() as i32;
    if p <= 0 || p >= n {
        return true;
    }
    let (x, y) = (c[p as usize - 1], c[p as usize]);
    match unit {
        UNIT_CHAR => true,
        1 | UNIT_WORD => x.is_whitespace() && !y.is_whitespace(),
        UNIT_LINE | UNIT_PARAGRAPH => x == '\n',
        _ => false,
    }
}

fn next_boundary(c: &[char], unit: i32, p: i32) -> i32 {
    let n = c.len() as i32;
    let mut q = p + 1;
    while q < n && !boundary(c, unit, q) {
        q += 1;
    }
    q.min(n)
}

fn prev_boundary(c: &[char], unit: i32, p: i32) -> i32 {
    let mut q = p - 1;
    while q > 0 && !boundary(c, unit, q) {
        q -= 1;
    }
    q.max(0)
}

// the unit around character p: (start, end); at the end of the text the last unit (a character:
// the empty range there)
fn unit_at(c: &[char], unit: i32, p: i32) -> (i32, i32) {
    let n = c.len() as i32;
    if unit == UNIT_CHAR {
        let p = p.clamp(0, n);
        return (p, (p + 1).min(n));
    }
    if unit > UNIT_PARAGRAPH {
        return (0, n);
    }
    let q = p.clamp(0, (n - 1).max(0));
    let a = if boundary(c, unit, q) { q } else { prev_boundary(c, unit, q) };
    (a, if n == 0 { 0 } else { next_boundary(c, unit, a) })
}

// p moved by count units (stopping at 0 and at limit) -> (new p, units moved)
fn step(c: &[char], unit: i32, p: i32, count: i32, limit: i32) -> (i32, i32) {
    let (mut p, mut moved) = (p, 0);
    if count > 0 {
        while moved < count && p < limit {
            p = next_boundary(c, unit, p).min(limit);
            moved += 1;
        }
    } else {
        while moved > count && p > 0 {
            p = prev_boundary(c, unit, p);
            moved -= 1;
        }
    }
    (p, moved)
}

unsafe extern "system" fn r_qi(this: This, riid: *const Guid, out: *mut This) -> Hr {
    if out.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    let id = unsafe { *riid };
    if id == IID_UNKNOWN || id == IID_RANGE {
        unsafe { range(this) }.refs.fetch_add(1, Ordering::AcqRel);
        unsafe { *out = this };
        S_OK
    } else {
        unsafe { *out = core::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn r_add(this: This) -> u32 {
    unsafe { range(this) }.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn r_rel(this: This) -> u32 {
    let n = unsafe { range(this) }.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        // SAFETY: the last reference: the Box made in new_range goes.
        drop(unsafe { Box::from_raw(this as *mut Range) });
    }
    n
}

unsafe extern "system" fn r_clone(this: This, out: *mut This) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    unsafe { *out = new_range(r.hwnd, r.pw, r.key, a, b) };
    S_OK
}

unsafe extern "system" fn r_compare(this: This, other: This, out: *mut i32) -> Hr {
    let r = unsafe { range(this) };
    let same = unsafe { ours(other) }.is_some_and(|o| o.pw == r.pw && o.key == r.key && o.get() == r.get());
    unsafe { *out = same as i32 };
    S_OK
}

// endpoint 0 = start, 1 = end
fn endpoint(r: &Range, ep: i32) -> i32 {
    let (a, b) = r.get();
    if ep == 0 { a } else { b }
}

unsafe extern "system" fn r_compare_endpoints(this: This, ep: i32, other: This, oep: i32, out: *mut i32) -> Hr {
    let r = unsafe { range(this) };
    let Some(o) = (unsafe { ours(other) }).filter(|o| o.pw == r.pw && o.key == r.key) else { return E_INVALIDARG };
    unsafe { *out = (endpoint(r, ep) - endpoint(o, oep)).signum() };
    S_OK
}

unsafe extern "system" fn r_expand(this: This, unit: i32) -> Hr {
    let r = unsafe { range(this) };
    let (a, _) = r.get();
    match r.with(|_, c| (unit_at(c, unit, a), c.len() as i32)) {
        Some(((x, y), n)) => {
            r.set(x, y, n);
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn r_find_attribute(_this: This, _id: i32, _v: Variant, _back: i32, out: *mut This) -> Hr {
    unsafe { *out = core::ptr::null_mut() }; // one format for the whole text: nothing to find
    S_OK
}

unsafe extern "system" fn r_find_text(this: This, s: *const u16, back: i32, nocase: i32, out: *mut This) -> Hr {
    let r = unsafe { range(this) };
    unsafe { *out = core::ptr::null_mut() };
    let want: Vec<char> = unsafe { wide_str(s) }.chars().collect();
    if want.is_empty() {
        return E_INVALIDARG;
    }
    let (a, b) = r.get();
    let low = |c: char| if nocase != 0 { c.to_lowercase().next().unwrap_or(c) } else { c };
    let found = r.with(|_, c| {
        let (a, b) = (a.max(0) as usize, (b.max(0) as usize).min(c.len()));
        let hit = |i: &usize| want.iter().enumerate().all(|(k, &w)| low(c[i + k]) == low(w));
        let last = (b + 1).saturating_sub(want.len());
        if back != 0 { (a..last).rev().find(hit) } else { (a..last).find(hit) }
    });
    match found {
        Some(Some(i)) => {
            unsafe { *out = new_range(r.hwnd, r.pw, r.key, i as i32, (i + want.len()) as i32) };
            S_OK
        }
        Some(None) => S_OK,
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

// the reserved "not supported" value (VT_UNKNOWN; VT_EMPTY without UIAutomationCore)
fn not_supported() -> Variant {
    let mut v = Variant::default();
    let p = f(F_NOTSUP);
    if p != 0 {
        let mut u = 0usize;
        // SAFETY: UiaGetReservedNotSupportedValue(IUnknown**) -> HRESULT.
        let get: unsafe extern "system" fn(*mut usize) -> Hr = unsafe { core::mem::transmute(p) };
        if unsafe { get(&mut u) } >= 0 && u != 0 {
            v.vt = VT_UNKNOWN;
            v.v[0] = u as u64;
        }
    }
    v
}

unsafe extern "system" fn r_attribute(this: This, id: i32, out: *mut Variant) -> Hr {
    let r = unsafe { range(this) };
    let v = match id {
        ATTR_READ_ONLY => match r.with(|n, _| n.actions & ax::A_TEXT == 0) {
            Some(ro) => Variant { vt: VT_BOOL, v: [if ro { 0xFFFF } else { 0 }, 0], ..Variant::default() },
            None => return UIA_E_ELEMENTNOTAVAILABLE,
        },
        _ => not_supported(),
    };
    unsafe { *out = v };
    S_OK
}

unsafe extern "system" fn r_rects(this: This, out: *mut *mut u8) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    let (ox, oy) = origin(r.hwnd);
    let rects = r.with(|n, c| {
        let mut out: Vec<[i32; 4]> = Vec::new();
        if a >= b {
            // the insertion point: the caret, or where character a would be
            let q = if a == n.caret && n.caret_rect[3] > 0 { n.caret_rect } else { n.char_rect(a) };
            out.push([q[0], q[1], q[2].clamp(1, 2), q[3]]);
            return out;
        }
        // one rectangle per line of the range
        let mut s = a;
        while s < b {
            let mut e = s;
            while e < b && c[e as usize] != '\n' {
                e += 1;
            }
            if e > s {
                let (p, q) = (n.char_rect(s), n.char_rect(e - 1));
                let (x0, x1) = (p[0].min(q[0]), (p[0] + p[2]).max(q[0] + q[2]));
                out.push([x0, p[1], x1 - x0, p[3]]);
            }
            s = e + 1;
        }
        out
    });
    let Some(rects) = rects else { return UIA_E_ELEMENTNOTAVAILABLE };
    let v: Vec<u64> = rects
        .iter()
        .flat_map(|q| [(q[0] + ox) as f64, (q[1] + oy) as f64, q[2] as f64, q[3] as f64])
        .map(f64::to_bits)
        .collect();
    unsafe { *out = sa_vector(VT_R8, &v) };
    S_OK
}

unsafe extern "system" fn r_enclosing(this: This, out: *mut This) -> Hr {
    let r = unsafe { range(this) };
    unsafe { *out = iface(new_elem(r.hwnd, r.pw, r.key), O_SIMPLE) };
    S_OK
}

unsafe extern "system" fn r_text(this: This, max: i32, out: *mut usize) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    let t = r.with(|_, c| {
        let (a, b) = (a.max(0) as usize, (b.max(0) as usize).min(c.len()));
        let take = if max < 0 { usize::MAX } else { max as usize };
        c[a.min(b)..b].iter().take(take).collect::<String>()
    });
    match t {
        Some(t) => {
            unsafe { *out = bstr(&t) };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn r_move(this: This, unit: i32, count: i32, out: *mut i32) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    let m = r.with(|_, c| {
        let n = c.len() as i32;
        if a == b {
            // an insertion point moves to the start of the count-th unit (the end of the text too)
            let (p, moved) = step(c, unit, a, count, n);
            (p, p, moved, n)
        } else {
            // a range starts at its unit, moves by whole units and covers one unit there
            let (s, _) = unit_at(c, unit, a);
            let last = if n == 0 { 0 } else { unit_at(c, unit, n - 1).0 };
            let (p, moved) = step(c, unit, s, count, last);
            let (x, y) = unit_at(c, unit, p);
            (x, y, moved, n)
        }
    });
    match m {
        Some((x, y, moved, n)) => {
            if moved != 0 {
                r.set(x, y, n);
            }
            unsafe { *out = moved };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn r_move_endpoint(this: This, ep: i32, unit: i32, count: i32, out: *mut i32) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    let m = r.with(|_, c| {
        let n = c.len() as i32;
        let (p, moved) = step(c, unit, if ep == 0 { a } else { b }, count, n);
        // an endpoint that passes the other one takes it along
        let (x, y) = if ep == 0 { (p, b.max(p)) } else { (a.min(p), p) };
        (x, y, moved, n)
    });
    match m {
        Some((x, y, moved, n)) => {
            r.set(x, y, n);
            unsafe { *out = moved };
            S_OK
        }
        None => UIA_E_ELEMENTNOTAVAILABLE,
    }
}

unsafe extern "system" fn r_move_endpoint_range(this: This, ep: i32, other: This, oep: i32) -> Hr {
    let r = unsafe { range(this) };
    let Some(o) = (unsafe { ours(other) }).filter(|o| o.pw == r.pw && o.key == r.key) else { return E_INVALIDARG };
    let p = endpoint(o, oep);
    let (a, b) = r.get();
    let (x, y) = if ep == 0 { (p, b.max(p)) } else { (a.min(p), p) };
    r.a.store(x, Ordering::Release);
    r.b.store(y, Ordering::Release);
    S_OK
}

unsafe extern "system" fn r_select(this: This) -> Hr {
    let r = unsafe { range(this) };
    let (a, b) = r.get();
    let Some((s, wi, k)) = look_key(r.pw, r.key) else { return UIA_E_ELEMENTNOTAVAILABLE };
    let t = &s.wins[wi];
    let Some(n) = t.nodes.get(k as usize) else { return UIA_E_INVALIDOPERATION };
    if n.state & ax::S_DISABLED != 0 || n.actions & ax::A_FOCUS == 0 {
        return UIA_E_ELEMENTNOTENABLED;
    }
    // the caret at the end of the range (where a selection made with the keyboard ends)
    ax::request(Request { win: t.win, key: n.key, act: Act::Select(a, b) });
    S_OK
}

unsafe extern "system" fn r_add_select(_this: This) -> Hr {
    UIA_E_INVALIDOPERATION // one selection
}

unsafe extern "system" fn r_scroll(_this: This, _top: i32) -> Hr {
    S_OK // the text box keeps its caret in view
}

unsafe extern "system" fn r_children(_this: This, out: *mut *mut u8) -> Hr {
    unsafe { *out = sa_vector(VT_UNKNOWN, &[]) };
    S_OK
}
