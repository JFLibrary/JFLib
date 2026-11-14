// apple_mac_dnd -- (dnd.rs) drag and drop on macOS: every window's view is a dragging destination;
// drag_start drags the app's content out through a dragging session of the view
//
//   register(view)            (win_create) registerForDraggedTypes: file URLs, URLs, text, HTML, PNG,
//                             TIFF
//   the view's methods        draggingEntered: / draggingUpdated: ask the UI (dnd::drag_query at the
//                             point, px) and answer its effect; E_DRAG on enter and on every new
//                             point, data 0 on draggingExited:; performDragOperation: reads only the
//                             kinds the UI takes (file URLs as paths, text, HTML, other URLs, the PNG
//                             or a TIFF as PNG), then E_DROP (data = the number of paths)
//   os_drag_start(data)       (dnd.rs drag_start) remember the drag; wait() starts it (run_pending):
//                             beginDraggingSessionWithItems:event:source: with the last left-button
//                             event of a window (note_mouse).  The session ends in
//                             draggingSession:endedAtPoint:operation: -> E_DRAG_END (the effect) and
//                             the E_UP the session took
//   writers(data)             what a drag (and dnd_test_formats) puts on the pasteboard: an NSURL per
//                             file, one NSPasteboardItem with the text (else the first URL), HTML, the
//                             PNG (else one made of the BMP, and the BMP) and the URL; further URLs as
//                             NSURLs
//   dnd_test_formats(data) / dnd_test_drop(data, w, x, y, mask)   (plattest, no user) the writers on
//                             a private pasteboard: its types and bytes; the view's destination
//                             methods fed with it through a stand-in NSDraggingInfo (mask: the
//                             NSDragOperation bits the source offers, as AppKit hands them on)
//
// Effects: AppKit narrows the source's mask by the modifier keys itself (Option copy, Command move),
// so the UI's pick gets no keys; NSDragOperationGeneric counts as a move.  Everything a drop brings
// is bounded by dnd.rs / clipdrop.rs (drop_add, drop_set_*).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_void};

use super::{Id, NIL, NSPoint, NSRect, NSSize, Sel, UTI_BMP, UTI_PNG, UTI_TIFF, class, floor, ids, image_convert, msg, nsstring, push, release,
            string_of, subclass, view_win, with};
use crate::{BTN_LEFT, DE_ALL, DE_COPY, DE_LINK, DE_MOVE, DT_FILES, DT_HTML, DT_IMAGE, DT_STD, DT_TEXT, DT_URL, DragData, E_DRAG, E_DROP, E_UP,
            PW_MAX};

const T_FILE: &CStr = c"public.file-url";
const T_URL: &CStr = c"public.url";
const T_TEXT: &CStr = c"public.utf8-plain-text";
const T_HTML: &CStr = c"public.html";
// NSDragOperation
const OP_COPY: u64 = 1;
const OP_LINK: u64 = 2;
const OP_GENERIC: u64 = 4;
const OP_MOVE: u64 = 16;

struct D {
    types: u32,            // what the drag over a window offers (DT_*)
    last: (i32, i32),      // its last point (E_DRAG when it changes)
    out: Option<DragData>, // drag_start's drag, started by wait()
    out_w: i32,            // the window it runs from
    out_allowed: u32,      // its effects
    ev: Id,                // the last left-button event of a window (retained)
    test: (Id, NSPoint, u64), // dnd_test_drop: the pasteboard, the point (window coordinates), the mask
}

// the main thread only (AppKit's thread; UiOnly refuses other threads), like apple_mac.rs's state;
// no borrow spans a call into AppKit
static DS: crate::ui_only::UiOnly<UnsafeCell<D>> = crate::ui_only::UiOnly::new(UnsafeCell::new(D {
    types: 0,
    last: (0, 0),
    out: None,
    out_w: 0,
    out_allowed: 0,
    ev: core::ptr::null_mut(),
    test: (core::ptr::null_mut(), NSPoint { x: 0.0, y: 0.0 }, 0),
}));

fn d<R>(f: impl FnOnce(&mut D) -> R) -> R {
    // SAFETY: main thread only; the borrow ends inside f, which never calls into AppKit.
    unsafe { f(&mut *DS.get().get()) }
}

/// (win_create) the view takes drags of these types
pub(super) fn register(view: Id) {
    // SAFETY: an array of ours (released after) handed to our view.
    unsafe {
        let arr: Id = msg!(class(c"NSMutableArray"), c"new"; Id);
        for t in [T_FILE, T_URL, T_TEXT, T_HTML, UTI_PNG, UTI_TIFF] {
            let s = nsstring(t.to_str().unwrap_or(""));
            msg!(arr, c"addObject:", s => Id; ());
            release(s);
        }
        msg!(view, c"registerForDraggedTypes:", arr => Id; ());
        release(arr);
    }
}

/// (the view's mouse methods) the last left-button down / drag event: a drag out starts from it
pub(super) fn note_mouse(ev: Id) {
    // SAFETY: retain the new event, release the one kept before.
    unsafe {
        msg!(ev, c"retain"; Id);
        let old = d(|d| core::mem::replace(&mut d.ev, ev));
        if !old.is_null() {
            release(old);
        }
    }
}

// ---------------------------------------------------------------- pasteboard helpers

fn pb_has(pb: Id, t: &CStr) -> bool {
    let ty = nsstring(t.to_str().unwrap_or(""));
    // SAFETY: availableTypeFromArray: with a one-element array (autoreleased in the caller's pool).
    let has = unsafe {
        let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", ty => Id; Id);
        !msg!(pb, c"availableTypeFromArray:", arr => Id; Id).is_null()
    };
    release(ty);
    has
}

fn pb_data(pb: Id, t: &CStr) -> Option<Vec<u8>> {
    let ty = nsstring(t.to_str().unwrap_or(""));
    // SAFETY: dataForType: of a pasteboard; the bytes are copied at once.
    let out = unsafe {
        let data: Id = msg!(pb, c"dataForType:", ty => Id; Id);
        if data.is_null() {
            None
        } else {
            let p: *const u8 = msg!(data, c"bytes"; *const u8);
            let n: usize = msg!(data, c"length"; usize);
            Some(if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() })
        }
    };
    release(ty);
    out
}

fn pb_string(pb: Id, t: &CStr) -> Option<String> {
    let ty = nsstring(t.to_str().unwrap_or(""));
    // SAFETY: stringForType: of a pasteboard.
    let s: Id = unsafe { msg!(pb, c"stringForType:", ty => Id; Id) };
    release(ty);
    (!s.is_null()).then(|| string_of(s))
}

/// the DT_* kinds a pasteboard offers
fn offered(pb: Id) -> u32 {
    let mut t = 0;
    for (ty, bit) in [(T_FILE, DT_FILES), (T_TEXT, DT_TEXT), (T_HTML, DT_HTML), (T_URL, DT_URL), (UTI_PNG, DT_IMAGE), (UTI_TIFF, DT_IMAGE)] {
        if t & bit == 0 && pb_has(pb, ty) {
            t |= bit;
        }
    }
    t
}

/// NSDragOperation bits -> DE_* (Generic counts as a move)
fn de_of(mask: u64) -> u32 {
    let mut e = 0;
    if mask & OP_COPY != 0 {
        e |= DE_COPY;
    }
    if mask & OP_LINK != 0 {
        e |= DE_LINK;
    }
    if mask & (OP_MOVE | OP_GENERIC) != 0 {
        e |= DE_MOVE;
    }
    e
}

/// one DE_* effect -> the operation of the mask that means it
fn op_of(e: u32, mask: u64) -> u64 {
    match e {
        DE_COPY => OP_COPY,
        DE_LINK => OP_LINK,
        DE_MOVE if mask & OP_MOVE != 0 => OP_MOVE,
        DE_MOVE => OP_GENERIC,
        _ => 0,
    }
}

/// DE_* of the effects a drag out allows -> NSDragOperation bits
fn ops_of(allowed: u32) -> u64 {
    let mut m = 0;
    if allowed & DE_COPY != 0 {
        m |= OP_COPY;
    }
    if allowed & DE_MOVE != 0 {
        m |= OP_MOVE | OP_GENERIC;
    }
    if allowed & DE_LINK != 0 {
        m |= OP_LINK;
    }
    m
}

// ---------------------------------------------------------------- the destination

/// the drag's point in the view: px from the top left
fn info_pos(view: Id, info: Id) -> (i32, i32) {
    let scale = rec_scale(view_win(view));
    // SAFETY: draggingLocation (window coordinates) of the dragging info, converted by our view.
    let p = unsafe {
        let p: NSPoint = msg!(info, c"draggingLocation"; NSPoint);
        msg!(view, c"convertPoint:fromView:", p => NSPoint, NIL => Id; NSPoint)
    };
    (floor(p.x * scale) as i32, floor(p.y * scale) as i32)
}

fn rec_scale(w: i32) -> f64 {
    ids(w).map_or(1.0, |x| x.2)
}

/// draggingEntered: / draggingUpdated: -> the operation the UI takes at the point
fn over(view: Id, info: Id, enter: bool) -> u64 {
    let w = view_win(view);
    // SAFETY: the dragging info's pasteboard and mask.
    let (pb, mask) = unsafe { (msg!(info, c"draggingPasteboard"; Id), msg!(info, c"draggingSourceOperationMask"; u64)) };
    if enter {
        let t = offered(pb) & DT_STD;
        d(|d| {
            d.types = t;
            d.last = (i32::MIN, i32::MIN);
        });
    }
    let (x, y) = info_pos(view, info);
    let (types, allowed) = (d(|d| d.types), de_of(mask));
    let (_, e) = crate::drag_query(w, x, y, types, allowed, 0);
    let moved = d(|d| core::mem::replace(&mut d.last, (x, y)) != (x, y));
    if moved && types != 0 {
        push(w, E_DRAG, 0, x, y, types | (allowed & DE_ALL) << 8);
    }
    op_of(e, mask)
}

extern "C" fn v_drag_entered(this: Id, _cmd: Sel, info: Id) -> u64 {
    over(this, info, true)
}
extern "C" fn v_drag_updated(this: Id, _cmd: Sel, info: Id) -> u64 {
    over(this, info, false)
}
extern "C" fn v_drag_exited(this: Id, _cmd: Sel, _info: Id) {
    push(view_win(this), E_DRAG, 0, -1, -1, 0);
}
extern "C" fn v_drag_prepare(_this: Id, _cmd: Sel, _info: Id) -> bool {
    true
}
extern "C" fn v_drag_perform(this: Id, _cmd: Sel, info: Id) -> bool {
    let w = view_win(this);
    // SAFETY: the dragging info's pasteboard and mask.
    let (pb, mask) = unsafe { (msg!(info, c"draggingPasteboard"; Id), msg!(info, c"draggingSourceOperationMask"; u64)) };
    let (x, y) = info_pos(this, info);
    let (types, e) = crate::drag_query(w, x, y, offered(pb) & DT_STD, de_of(mask), 0);
    if e == 0 {
        push(w, E_DRAG, 0, -1, -1, 0);
        return false;
    }
    crate::drop_reset();
    let files = read(pb, types);
    crate::drop_set_effect(e);
    crate::drop_set_allowed(de_of(mask)); // the drag's operation mask
    push(w, E_DROP, 0, x, y, files);
    true
}

/// the kinds the UI takes into the pending drop -> the number of paths
fn read(pb: Id, types: u32) -> u32 {
    let mut files = 0;
    if types & (DT_FILES | DT_URL) != 0 {
        // SAFETY: readObjectsForClasses:options: with NSURL (autoreleased in the caller's pool); the
        // strings are copied.
        unsafe {
            let classes: Id = msg!(class(c"NSArray"), c"arrayWithObject:", class(c"NSURL") => Id; Id);
            let urls: Id = msg!(pb, c"readObjectsForClasses:options:", classes => Id, NIL => Id; Id);
            let n: usize = if urls.is_null() { 0 } else { msg!(urls, c"count"; usize) };
            for k in 0..n {
                let u: Id = msg!(urls, c"objectAtIndex:", k => usize; Id);
                if msg!(u, c"isFileURL"; bool) {
                    let p = string_of(msg!(u, c"path"; Id));
                    if types & DT_FILES != 0 && !p.is_empty() && crate::drop_add(&p) {
                        files += 1;
                    }
                } else if types & DT_URL != 0 {
                    crate::drop_add_url(&string_of(msg!(u, c"absoluteString"; Id)));
                }
            }
        }
    }
    if types & DT_TEXT != 0 {
        if let Some(s) = pb_string(pb, T_TEXT) {
            crate::drop_set_text(s.as_bytes());
        }
    }
    if types & DT_HTML != 0 {
        if let Some(h) = pb_data(pb, T_HTML) {
            crate::drop_set_html(&h);
        }
    }
    if types & DT_IMAGE != 0 {
        let img = pb_data(pb, UTI_PNG).filter(|d| !d.is_empty()).or_else(|| pb_data(pb, UTI_TIFF).and_then(|t| image_convert(&t, "public.png")));
        if let Some(i) = img {
            crate::drop_set_image(i);
        }
    }
    files
}

// ---------------------------------------------------------------- the source

/// what a drag of `data` puts on the pasteboard (each +1: the caller releases them)
fn writers(data: &DragData) -> Vec<Id> {
    let mut v = Vec::new();
    // SAFETY: Foundation / AppKit objects made here; what is kept is retained for the caller.
    unsafe {
        for f in &data.files {
            let abs = if f.starts_with('/') { f.clone() } else { format!("{}/{}", crate::cwd().trim_end_matches('/'), f) };
            let s = nsstring(&abs);
            let u: Id = msg!(class(c"NSURL"), c"fileURLWithPath:", s => Id; Id);
            release(s);
            if !u.is_null() {
                msg!(u, c"retain"; Id);
                v.push(u);
            }
        }
        let text = data.text.clone().or_else(|| data.urls.first().cloned());
        let made = if data.image.is_none() { data.bmp.as_deref().and_then(|b| image_convert(b, "public.png")) } else { None };
        let png = data.image.as_deref().or(made.as_deref());
        if text.is_some() || data.html.is_some() || png.is_some() || !data.urls.is_empty() {
            let item: Id = msg!(class(c"NSPasteboardItem"), c"new"; Id);
            let set_string = |t: &CStr, s: &str| {
                let (ty, ns) = (nsstring(t.to_str().unwrap_or("")), nsstring(s));
                msg!(item, c"setString:forType:", ns => Id, ty => Id; bool);
                release(ns);
                release(ty);
            };
            let set_data = |t: &CStr, b: &[u8]| {
                let ty = nsstring(t.to_str().unwrap_or(""));
                let nd: Id = msg!(class(c"NSData"), c"dataWithBytes:length:", b.as_ptr() => *const u8, b.len() => usize; Id);
                msg!(item, c"setData:forType:", nd => Id, ty => Id; bool);
                release(ty);
            };
            if let Some(t) = &text {
                set_string(T_TEXT, t);
            }
            if let Some(h) = &data.html {
                set_data(T_HTML, h.as_bytes());
            }
            if let Some(p) = png {
                set_data(UTI_PNG, p);
            }
            if let (Some(b), None) = (&data.bmp, &data.image) {
                set_data(UTI_BMP, b);
            }
            if let Some(u) = data.urls.first() {
                set_string(T_URL, u);
            }
            v.push(item);
        }
        for u in data.urls.iter().skip(1) {
            let s = nsstring(u);
            let url: Id = msg!(class(c"NSURL"), c"URLWithString:", s => Id; Id);
            release(s);
            if !url.is_null() {
                msg!(url, c"retain"; Id);
                v.push(url);
            }
        }
    }
    v
}

/// (dnd.rs drag_start) the drag starts in wait() from the last left-button event
pub(crate) fn os_drag_start(data: DragData) -> bool {
    if d(|d| d.ev.is_null() || d.out.is_some()) {
        return false;
    }
    d(|d| d.out = Some(data));
    true
}

/// (wait, top) start the remembered drag: a dragging session of the window the event came from
pub(super) fn run_pending() {
    let Some(data) = d(|d| d.out.take()) else { return };
    let ev = d(|d| d.ev);
    // SAFETY: the kept event and our window's view; dragging items made here, the array released after
    // the session took it.
    let started = unsafe {
        let win: Id = msg!(ev, c"window"; Id);
        let w = with(|s| s.wins.iter().position(|r| r.as_ref().is_some_and(|r| r.win == win && !win.is_null()))).flatten();
        match w.and_then(|w| ids(w as i32).map(|x| (w as i32, x.1, x.2))) {
            None => None,
            Some((w, view, _)) => {
                let p: NSPoint = msg!(ev, c"locationInWindow"; NSPoint);
                let p: NSPoint = msg!(view, c"convertPoint:fromView:", p => NSPoint, NIL => Id; NSPoint);
                let name = nsstring("NSMultipleDocuments");
                let img: Id = msg!(class(c"NSImage"), c"imageNamed:", name => Id; Id);
                release(name);
                let items: Id = msg!(class(c"NSMutableArray"), c"new"; Id);
                for (k, wr) in writers(&data).into_iter().enumerate() {
                    let item: Id = msg!(class(c"NSDraggingItem"), c"alloc"; Id);
                    let item: Id = msg!(item, c"initWithPasteboardWriter:", wr => Id; Id);
                    release(wr);
                    if item.is_null() {
                        continue;
                    }
                    let o = k as f64 * 4.0;
                    let frame = NSRect { origin: NSPoint { x: p.x - 16.0 + o, y: p.y - 16.0 + o }, size: NSSize { w: 32.0, h: 32.0 } };
                    msg!(item, c"setDraggingFrame:contents:", frame => NSRect, img => Id; ());
                    msg!(items, c"addObject:", item => Id; ());
                    release(item);
                }
                d(|d| {
                    d.out_w = w;
                    d.out_allowed = data.allowed();
                });
                let n: usize = msg!(items, c"count"; usize);
                let session: Id = if n == 0 { NIL } else { msg!(view, c"beginDraggingSessionWithItems:event:source:", items => Id, ev => Id, view => Id; Id) };
                release(items);
                Some((w, !session.is_null()))
            }
        }
    };
    match started {
        Some((_, true)) => {}
        Some((w, false)) => crate::dnd::drag_finished(w as u8, 0),
        None => crate::dnd::drag_finished(0, 0),
    }
}

extern "C" fn v_src_mask(_this: Id, _cmd: Sel, _session: Id, _context: isize) -> u64 {
    ops_of(d(|d| d.out_allowed))
}

extern "C" fn v_src_ended(_this: Id, _cmd: Sel, _session: Id, p: NSPoint, op: u64) {
    let (w, allowed) = d(|d| (d.out_w, d.out_allowed));
    let eff = match op {
        o if o & OP_COPY != 0 => DE_COPY,
        o if o & (OP_MOVE | OP_GENERIC) != 0 => DE_MOVE,
        o if o & OP_LINK != 0 => DE_LINK,
        _ => 0,
    } & allowed;
    crate::dnd::drag_finished((w.max(0) as u32 & (PW_MAX as u32 - 1)) as u8, eff);
    // the button's release went to the session: E_UP at the end point (client px)
    if let Some((win, _, scale)) = ids(w) {
        // SAFETY: frame / contentRectForFrameRect: of our window.
        let c = unsafe {
            let fr: NSRect = msg!(win, c"frame"; NSRect);
            msg!(win, c"contentRectForFrameRect:", fr => NSRect; NSRect)
        };
        let (x, y) = (floor((p.x - c.origin.x) * scale) as i32, floor((c.origin.y + c.size.h - p.y) * scale) as i32);
        push(w, E_UP, BTN_LEFT, x, y, 0);
    }
}

/// the view's dragging destination and source methods (apple_mac.rs view_methods)
pub(super) fn view_methods() -> [(&'static CStr, *const c_void, &'static CStr); 7] {
    [
        (c"draggingEntered:", v_drag_entered as *const c_void, c"Q@:@"),
        (c"draggingUpdated:", v_drag_updated as *const c_void, c"Q@:@"),
        (c"draggingExited:", v_drag_exited as *const c_void, c"v@:@"),
        (c"prepareForDragOperation:", v_drag_prepare as *const c_void, c"B@:@"),
        (c"performDragOperation:", v_drag_perform as *const c_void, c"B@:@"),
        (c"draggingSession:sourceOperationMaskForDraggingContext:", v_src_mask as *const c_void, c"Q@:@q"),
        (c"draggingSession:endedAtPoint:operation:", v_src_ended as *const c_void, c"v@:@{CGPoint=dd}Q"),
    ]
}

// ---------------------------------------------------------------- test aids

/// (plattest) the pasteboard a drag of data fills: (type, bytes) of each item's types
pub fn dnd_test_formats(data: &DragData) -> Option<Vec<(String, Vec<u8>)>> {
    if !super::init() {
        return None;
    }
    let mut out = Vec::new();
    // SAFETY: a private pasteboard of ours (released globally after), a pool of ours.
    unsafe {
        let pool = crate::apple::objc::objc_autoreleasePoolPush();
        let pb = filled(data);
        let items: Id = msg!(pb, c"pasteboardItems"; Id);
        let n: usize = if items.is_null() { 0 } else { msg!(items, c"count"; usize) };
        for k in 0..n {
            let item: Id = msg!(items, c"objectAtIndex:", k => usize; Id);
            let tys: Id = msg!(item, c"types"; Id);
            let m: usize = if tys.is_null() { 0 } else { msg!(tys, c"count"; usize) };
            for j in 0..m {
                let ty: Id = msg!(tys, c"objectAtIndex:", j => usize; Id);
                let data: Id = msg!(item, c"dataForType:", ty => Id; Id);
                let bytes = if data.is_null() {
                    Vec::new()
                } else {
                    let p: *const u8 = msg!(data, c"bytes"; *const u8);
                    let len: usize = msg!(data, c"length"; usize);
                    if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, len).to_vec() }
                };
                out.push((string_of(ty), bytes));
            }
        }
        msg!(pb, c"releaseGlobally"; ());
        crate::apple::objc::objc_autoreleasePoolPop(pool);
    }
    Some(out)
}

/// a private pasteboard with the writers of data on it
fn filled(data: &DragData) -> Id {
    // SAFETY: a pasteboard with a unique name, an array of the writers (released after writing).
    unsafe {
        let pb: Id = msg!(class(c"NSPasteboard"), c"pasteboardWithUniqueName"; Id);
        msg!(pb, c"clearContents"; isize);
        let arr: Id = msg!(class(c"NSMutableArray"), c"new"; Id);
        for wr in writers(data) {
            msg!(arr, c"addObject:", wr => Id; ());
            release(wr);
        }
        msg!(pb, c"writeObjects:", arr => Id; bool);
        release(arr);
        pb
    }
}

/// (plattest) a drop of data at px x, y of window w, the source offering `mask` (NSDragOperation
/// bits): draggingEntered:, draggingUpdated:, then performDragOperation: (or draggingExited: when
/// refused) of the view, with a stand-in NSDraggingInfo -> the effect (0 refused); None: no window
pub fn dnd_test_drop(data: &DragData, w: i32, x: i32, y: i32, mask: u64) -> Option<u32> {
    let (_, view, scale) = ids(w)?;
    let (cls, _) = subclass(c"NSObject", c"JFTestDraggingInfo", &[
        (c"draggingPasteboard", ti_pasteboard as *const c_void, c"@@:"),
        (c"draggingLocation", ti_location as *const c_void, c"{CGPoint=dd}@:"),
        (c"draggingSourceOperationMask", ti_mask as *const c_void, c"Q@:"),
    ], &[]);
    // SAFETY: a pool of ours; our view's methods with a stand-in object of ours (released after).
    unsafe {
        let pool = crate::apple::objc::objc_autoreleasePoolPush();
        let pb = filled(data);
        let loc: NSPoint = msg!(view, c"convertPoint:toView:", NSPoint { x: x as f64 / scale, y: y as f64 / scale } => NSPoint, NIL => Id; NSPoint);
        d(|d| d.test = (pb, loc, mask));
        let info: Id = msg!(cls, c"new"; Id);
        msg!(view, c"draggingEntered:", info => Id; u64);
        let op: u64 = msg!(view, c"draggingUpdated:", info => Id; u64);
        let ok = op != 0 && msg!(view, c"prepareForDragOperation:", info => Id; bool) && msg!(view, c"performDragOperation:", info => Id; bool);
        if op == 0 {
            msg!(view, c"draggingExited:", info => Id; ());
        }
        release(info);
        msg!(pb, c"releaseGlobally"; ());
        d(|d| d.test = (NIL, NSPoint { x: 0.0, y: 0.0 }, 0));
        crate::apple::objc::objc_autoreleasePoolPop(pool);
        Some(if ok { de_of(op) } else { 0 })
    }
}

extern "C" fn ti_pasteboard(_this: Id, _cmd: Sel) -> Id {
    d(|d| d.test.0)
}
extern "C" fn ti_location(_this: Id, _cmd: Sel) -> NSPoint {
    d(|d| d.test.1)
}
extern "C" fn ti_mask(_this: Id, _cmd: Sel) -> u64 {
    d(|d| d.test.2)
}
