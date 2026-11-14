// apple_ios_text -- (apple_ios_a11y.rs) UITextInput of a text box: on its element (VoiceOver reads,
// navigates and selects the text there) and on JFView (the keyboard: the text around the caret,
// marked text of compositions, dictation; docs/VOICEOVER.md decision 4)
//
// The document is the text box's text from the accessibility tree: the element's node, for the
// view the focused text box of the active window (none while no tree is published: an empty
// document, typing works as before through insertText / deleteBackward).  While the keyboard
// composes (setMarkedText) the view's document is the marked text alone, as on the Mac
// (NSTextInputClient in apple_mac.rs); the UI draws it as preedit (ime_preedit_set) and gets it
// as a commit (unmarkText, insertText).  Positions are UTF-16 offsets (objects of JFTextPosition),
// ranges two of them (JFTextRange); rectangles are in JFView's points (textInputView).  Selecting
// and replacing become a11y::Act::Select / Act::Text.

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_void};

use super::super::super::objc::{Id, NIL, NSPoint, NSRange, NSRect, Sel, class, ivar_get, ivar_set, msg, string_of, subclass};
use super::super::host;
use super::{text_act, text_node, text_obj};
use crate::a11y::{self as ax, Act, Node, Role, WinTree};

// ---------------------------------------------------------------- positions and ranges

struct Classes {
    pos: Id,
    pos_off: isize,
    range: Id,
    range_off: isize,
}

struct Main(UnsafeCell<Option<Classes>>, UnsafeCell<(String, usize)>);
// SAFETY: main thread only (UIKit's).
unsafe impl Sync for Main {}
/// our classes; the view's marked text and its selected UTF-16 offset
static ST: Main = Main(UnsafeCell::new(None), UnsafeCell::new((String::new(), 0)));

fn classes() -> &'static Classes {
    // SAFETY: main thread only; made once, never changed after.
    unsafe {
        let c = &mut *ST.0.get();
        if c.is_none() {
            let (pos, pos_off) = subclass(c"UITextPosition", c"JFTextPosition", &[], &[]);
            let (range, range_off) = subclass(c"UITextRange", c"JFTextRange", &[
                (c"start", r_start as *const c_void, c"@@:"),
                (c"end", r_end as *const c_void, c"@@:"),
                (c"isEmpty", r_empty as *const c_void, c"B@:"),
            ], &[]);
            *c = Some(Classes { pos, pos_off, range, range_off });
        }
        c.as_ref().unwrap_unchecked()
    }
}

fn marked() -> &'static mut (String, usize) {
    // SAFETY: main thread only; callers never hold it across a call that could reach here again.
    unsafe { &mut *ST.1.get() }
}

/// a position object (autoreleased) at UTF-16 offset u
fn pos(u: usize) -> Id {
    let c = classes();
    // SAFETY: a new object of our class; autoreleased for the caller.
    unsafe {
        let o: Id = msg!(c.pos, c"new"; Id);
        ivar_set(o, c.pos_off, u as isize);
        msg!(o, c"autorelease"; Id)
    }
}

/// a range object (autoreleased) from a to b (UTF-16)
fn range(a: usize, b: usize) -> Id {
    let c = classes();
    let (a, b) = (a.min(b), a.max(b));
    // SAFETY: a new object of our class; autoreleased for the caller.
    unsafe {
        let o: Id = msg!(c.range, c"new"; Id);
        ivar_set(o, c.range_off, ((a as isize) << 32) | (b as isize & 0xFFFF_FFFF));
        msg!(o, c"autorelease"; Id)
    }
}

fn is_a(o: Id, cls: Id) -> bool {
    // SAFETY: isKindOfClass: of a live object (nil: false).
    !o.is_null() && unsafe { msg!(o, c"isKindOfClass:", cls => Id; bool) }
}

/// the offset of a position of ours
fn off(p: Id) -> Option<usize> {
    let c = classes();
    is_a(p, c.pos).then(|| ivar_get(p, c.pos_off).max(0) as usize)
}

/// the two offsets of a range of ours
fn ends(r: Id) -> Option<(usize, usize)> {
    let c = classes();
    is_a(r, c.range).then(|| {
        let v = ivar_get(r, c.range_off);
        ((v >> 32) as usize, (v & 0xFFFF_FFFF) as usize)
    })
}

extern "C" fn r_start(this: Id, _cmd: Sel) -> Id {
    pos(ends(this).map_or(0, |e| e.0))
}

extern "C" fn r_end(this: Id, _cmd: Sel) -> Id {
    pos(ends(this).map_or(0, |e| e.1))
}

extern "C" fn r_empty(this: Id, _cmd: Sel) -> bool {
    ends(this).is_none_or(|e| e.0 == e.1)
}

// ---------------------------------------------------------------- the document

fn u16_at(s: &str, k: i32) -> usize {
    s.chars().take(k.max(0) as usize).map(char::len_utf16).sum()
}

fn char_at(s: &str, u: usize) -> i32 {
    let mut n = 0;
    for (k, c) in s.chars().enumerate() {
        if n + c.len_utf16() > u {
            return k as i32;
        }
        n += c.len_utf16();
    }
    s.chars().count() as i32
}

/// the UTF-16 units of s from a to b
fn slice(s: &str, a: usize, b: usize) -> String {
    let (ka, kb) = (char_at(s, a), char_at(s, b));
    s.chars().skip(ka as usize).take((kb - ka).max(0) as usize).collect()
}

fn is_view(this: Id) -> bool {
    this == host().view.get()
}

/// f(tree, text box node, pw) for the element, or for the view the active window's focused text box
fn with_box<R>(this: Id, f: impl FnOnce(&WinTree, &Node, i32) -> R) -> Option<R> {
    if !is_view(this) {
        return text_node(this, f);
    }
    let s = ax::current()?;
    let t = s.wins.iter().find(|w| w.active && w.pw >= 0)?;
    let n = t.nodes.get(t.focus as usize).filter(|n| n.role == Role::TextBox)?;
    Some(f(t, n, t.pw))
}

/// the document: (text, selection start, end in UTF-16); the view while it composes: the marked
/// text with its selection
fn doc(this: Id) -> (String, usize, usize) {
    if is_view(this) && !marked().0.is_empty() {
        let (m, c) = marked().clone();
        return (m, c, c);
    }
    with_box(this, |_, n, _| {
        let s = String::from(n.text_str());
        let (a, b) = n.selection();
        let (ua, ub) = (u16_at(&s, a), u16_at(&s, b));
        (s, ua, ub)
    })
    .unwrap_or_default()
}

fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

// ---------------------------------------------------------------- UITextInput

// SAFETY (all methods below): UIKit calls them on the main thread with our element or JFView and
// with positions and ranges it got from us.

extern "C" fn t_text_in_range(this: Id, _cmd: Sel, r: Id) -> Id {
    let (s, ..) = doc(this);
    match ends(r) {
        Some((a, b)) => text_obj(&slice(&s, a, b)),
        None => NIL,
    }
}

extern "C" fn t_replace(this: Id, _cmd: Sel, r: Id, t: Id) {
    let new = string_of(t);
    let Some((a, b)) = ends(r) else { return };
    let (s, ..) = doc(this);
    if is_view(this) && !marked().0.is_empty() {
        commit(&new);
        return;
    }
    if with_box(this, |_, _, _| ()).is_none() {
        // no tree: what the keyboard types goes in at the caret
        type_text(&new);
        return;
    }
    let (ka, kb) = (char_at(&s, a), char_at(&s, b));
    let mut out: String = s.chars().take(ka as usize).collect();
    out.push_str(&new);
    out.extend(s.chars().skip(kb as usize));
    let caret = ka + new.chars().count() as i32;
    let elem = this;
    if text_act_on(elem, ax::A_TEXT, Act::Text(out)) {
        text_act_on(elem, ax::A_FOCUS, Act::Select(caret, caret));
    }
}

/// a request for the box of the element (for the view: its element) and a frame
fn text_act_on(this: Id, need: u32, a: Act) -> bool {
    if !is_view(this) {
        return text_act(this, need, a);
    }
    let Some((win, key, ok)) = with_box(this, |t, n, _| (t.win, n.key, n.actions & need != 0)) else { return false };
    if ok {
        ax::request(ax::Request { win, key, act: a });
        super::run_frame();
    }
    ok
}

extern "C" fn t_selected(this: Id, _cmd: Sel) -> Id {
    let (_, a, b) = doc(this);
    range(a, b)
}

extern "C" fn t_set_selected(this: Id, _cmd: Sel, r: Id) {
    let Some((a, b)) = ends(r) else { return };
    if is_view(this) && !marked().0.is_empty() {
        return;
    }
    let (s, ..) = doc(this);
    text_act_on(this, ax::A_FOCUS, Act::Select(char_at(&s, a), char_at(&s, b)));
}

extern "C" fn t_marked_range(this: Id, _cmd: Sel) -> Id {
    if is_view(this) && !marked().0.is_empty() { range(0, len16(&marked().0)) } else { NIL }
}

extern "C" fn t_marked_style(_this: Id, _cmd: Sel) -> Id {
    NIL
}

extern "C" fn t_set_marked_style(_this: Id, _cmd: Sel, _style: Id) {}

/// the window the keyboard types into
fn target() -> Option<u8> {
    crate::web::text_target()
}

extern "C" fn t_set_marked(this: Id, _cmd: Sel, t: Id, sel: NSRange) {
    if !is_view(this) {
        return;
    }
    let s = string_of(t);
    let caret = sel.location.min(len16(&s));
    let byte = slice(&s, 0, caret).len();
    *marked() = (s.clone(), caret);
    if let Some(w) = target() {
        crate::ime_preedit_set(w, &s, byte);
    }
    super::super::input_now();
}

/// the composition ends with text t
fn commit(t: &str) {
    marked().0.clear();
    if let Some(w) = target() {
        crate::ime_preedit_set(w, "", 0);
        crate::ime_commit_text(w, t);
    }
    super::super::input_now();
}

extern "C" fn t_unmark(this: Id, _cmd: Sel) {
    if is_view(this) && !marked().0.is_empty() {
        let m = marked().0.clone();
        commit(&m);
    }
}

extern "C" fn t_begin(_this: Id, _cmd: Sel) -> Id {
    pos(0)
}

extern "C" fn t_end(this: Id, _cmd: Sel) -> Id {
    pos(len16(&doc(this).0))
}

extern "C" fn t_range_from(_this: Id, _cmd: Sel, a: Id, b: Id) -> Id {
    match (off(a), off(b)) {
        (Some(a), Some(b)) => range(a, b),
        _ => NIL,
    }
}

extern "C" fn t_pos_offset(this: Id, _cmd: Sel, p: Id, d: isize) -> Id {
    let Some(p) = off(p) else { return NIL };
    let n = len16(&doc(this).0) as isize;
    let q = p as isize + d;
    if q < 0 || q > n { NIL } else { pos(q as usize) }
}

extern "C" fn t_pos_direction(this: Id, cmd: Sel, p: Id, dir: isize, d: isize) -> Id {
    // UITextLayoutDirection: right 2, left 3, up 4, down 5 (one line here: up / down as left / right)
    let d = if dir == 3 || dir == 4 { -d } else { d };
    t_pos_offset(this, cmd, p, d)
}

extern "C" fn t_compare(_this: Id, _cmd: Sel, a: Id, b: Id) -> isize {
    match (off(a), off(b)) {
        (Some(a), Some(b)) if a < b => -1,
        (Some(a), Some(b)) if a > b => 1,
        _ => 0,
    }
}

extern "C" fn t_offset(_this: Id, _cmd: Sel, a: Id, b: Id) -> isize {
    match (off(a), off(b)) {
        (Some(a), Some(b)) => b as isize - a as isize,
        _ => 0,
    }
}

extern "C" fn t_delegate(_this: Id, _cmd: Sel) -> Id {
    NIL
}

extern "C" fn t_set_delegate(_this: Id, _cmd: Sel, _d: Id) {}

extern "C" fn t_tokenizer(this: Id, _cmd: Sel) -> Id {
    // SAFETY: a string tokenizer of this text input (autoreleased).
    unsafe {
        let t: Id = msg!(class(c"UITextInputStringTokenizer"), c"alloc"; Id);
        let t: Id = msg!(t, c"initWithTextInput:", this => Id; Id);
        msg!(t, c"autorelease"; Id)
    }
}

extern "C" fn t_farthest(_this: Id, _cmd: Sel, r: Id, dir: isize) -> Id {
    let Some((a, b)) = ends(r) else { return NIL };
    pos(if dir == 3 || dir == 4 { a } else { b })
}

extern "C" fn t_extend(this: Id, _cmd: Sel, p: Id, dir: isize) -> Id {
    let Some(u) = off(p) else { return NIL };
    let (s, ..) = doc(this);
    let k = char_at(&s, u);
    if dir == 3 || dir == 4 {
        let a = u16_at(&s, (k - 1).max(0));
        range(a, u)
    } else {
        range(u, u16_at(&s, k + 1))
    }
}

extern "C" fn t_writing_direction(_this: Id, _cmd: Sel, _p: Id, _dir: isize) -> isize {
    0 // UITextWritingDirectionLeftToRight
}

extern "C" fn t_set_writing_direction(_this: Id, _cmd: Sel, _dir: isize, _r: Id) {}

/// client px of window pw -> JFView's points
fn view_rect(pw: i32, r: [i32; 4]) -> NSRect {
    let place = crate::win_get_rect(pw).unwrap_or([0; 4]);
    let s = host().scale.get();
    NSRect::new((place[0] + r[0]) as f64 / s, (place[1] + r[1]) as f64 / s, r[2] as f64 / s, r[3] as f64 / s)
}

/// the rectangle of characters a .. b (UTF-16) of the box, in the view's points
fn rect_of(this: Id, a: usize, b: usize) -> NSRect {
    with_box(this, |_, n, pw| {
        let s = n.text_str();
        let (ka, kb) = (char_at(s, a), char_at(s, b));
        let mut u = n.char_rect(ka);
        for k in ka + 1..kb {
            let q = n.char_rect(k);
            let (x0, y0) = (u[0].min(q[0]), u[1].min(q[1]));
            let (x1, y1) = ((u[0] + u[2]).max(q[0] + q[2]), (u[1] + u[3]).max(q[1] + q[3]));
            u = [x0, y0, x1 - x0, y1 - y0];
        }
        view_rect(pw, u)
    })
    .unwrap_or_default()
}

extern "C" fn t_first_rect(this: Id, _cmd: Sel, r: Id) -> NSRect {
    let Some((a, b)) = ends(r) else { return NSRect::default() };
    rect_of(this, a, b)
}

extern "C" fn t_caret_rect(this: Id, _cmd: Sel, p: Id) -> NSRect {
    let Some(u) = off(p) else { return NSRect::default() };
    let mut r = rect_of(this, u, u);
    r.size.w = r.size.w.min(2.0);
    r
}

extern "C" fn t_selection_rects(_this: Id, _cmd: Sel, _r: Id) -> Id {
    // SAFETY: an empty (autoreleased) array: the UI draws its own selection.
    unsafe { msg!(class(c"NSArray"), c"array"; Id) }
}

/// the character offset (UTF-16) at a point of the view
fn offset_at(this: Id, p: NSPoint) -> Option<usize> {
    with_box(this, |_, n, pw| {
        let place = crate::win_get_rect(pw).unwrap_or([0; 4]);
        let s = host().scale.get();
        let (x, y) = ((p.x * s) as i32 - place[0], (p.y * s) as i32 - place[1]);
        // outside the box: its nearest end
        let r = n.rect;
        let (x, y) = (x.clamp(r[0], r[0] + r[2] - 1), y.clamp(r[1], r[1] + r[3] - 1));
        let k = n.offset_at(x, y);
        u16_at(n.text_str(), k.max(0))
    })
}

extern "C" fn t_closest(this: Id, _cmd: Sel, p: NSPoint) -> Id {
    offset_at(this, p).map_or(NIL, pos)
}

extern "C" fn t_closest_within(this: Id, _cmd: Sel, p: NSPoint, r: Id) -> Id {
    let (Some(u), Some((a, b))) = (offset_at(this, p), ends(r)) else { return NIL };
    pos(u.clamp(a, b))
}

extern "C" fn t_char_range_at(this: Id, _cmd: Sel, p: NSPoint) -> Id {
    let Some(u) = offset_at(this, p) else { return NIL };
    let (s, ..) = doc(this);
    let k = char_at(&s, u);
    range(u, u16_at(&s, k + 1))
}

extern "C" fn t_input_view(_this: Id, _cmd: Sel) -> Id {
    host().view.get()
}

extern "C" fn t_has_text(this: Id, _cmd: Sel) -> bool {
    !doc(this).0.is_empty() || is_view(this)
}

/// typed text at the caret (the keyboard's way, through the view)
fn type_text(t: &str) {
    let view = host().view.get();
    if !view.is_null() {
        let s = text_obj(t);
        // SAFETY: insertText: of our view (UIKeyInput).
        unsafe { msg!(view, c"insertText:", s => Id; ()) };
    }
}

extern "C" fn e_insert_text(_this: Id, _cmd: Sel, t: Id) {
    type_text(&string_of(t));
}

extern "C" fn e_delete_backward(_this: Id, _cmd: Sel) {
    let view = host().view.get();
    if !view.is_null() {
        // SAFETY: deleteBackward of our view (UIKeyInput).
        unsafe { msg!(view, c"deleteBackward"; ()) };
    }
}

/// (apple_ios.rs) the view's insertText: while composing: the composition ends with it (true)
pub(crate) fn view_insert(t: &str) -> bool {
    if marked().0.is_empty() {
        return false;
    }
    commit(t);
    true
}

/// the methods of UITextInput both the view and the text box elements have
fn shared() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    let rect = c"{CGRect={CGPoint=dd}{CGSize=dd}}@:@";
    alloc::vec![
        (c"textInRange:", t_text_in_range as *const c_void, c"@@:@"),
        (c"replaceRange:withText:", t_replace as *const c_void, c"v@:@@"),
        (c"selectedTextRange", t_selected as *const c_void, c"@@:"),
        (c"setSelectedTextRange:", t_set_selected as *const c_void, c"v@:@"),
        (c"markedTextRange", t_marked_range as *const c_void, c"@@:"),
        (c"markedTextStyle", t_marked_style as *const c_void, c"@@:"),
        (c"setMarkedTextStyle:", t_set_marked_style as *const c_void, c"v@:@"),
        (c"setMarkedText:selectedRange:", t_set_marked as *const c_void, c"v@:@{_NSRange=QQ}"),
        (c"unmarkText", t_unmark as *const c_void, c"v@:"),
        (c"beginningOfDocument", t_begin as *const c_void, c"@@:"),
        (c"endOfDocument", t_end as *const c_void, c"@@:"),
        (c"textRangeFromPosition:toPosition:", t_range_from as *const c_void, c"@@:@@"),
        (c"positionFromPosition:offset:", t_pos_offset as *const c_void, c"@@:@q"),
        (c"positionFromPosition:inDirection:offset:", t_pos_direction as *const c_void, c"@@:@qq"),
        (c"comparePosition:toPosition:", t_compare as *const c_void, c"q@:@@"),
        (c"offsetFromPosition:toPosition:", t_offset as *const c_void, c"q@:@@"),
        (c"inputDelegate", t_delegate as *const c_void, c"@@:"),
        (c"setInputDelegate:", t_set_delegate as *const c_void, c"v@:@"),
        (c"tokenizer", t_tokenizer as *const c_void, c"@@:"),
        (c"positionWithinRange:farthestInDirection:", t_farthest as *const c_void, c"@@:@q"),
        (c"characterRangeByExtendingPosition:inDirection:", t_extend as *const c_void, c"@@:@q"),
        (c"baseWritingDirectionForPosition:inDirection:", t_writing_direction as *const c_void, c"q@:@q"),
        (c"setBaseWritingDirection:forRange:", t_set_writing_direction as *const c_void, c"v@:q@"),
        (c"firstRectForRange:", t_first_rect as *const c_void, rect),
        (c"caretRectForPosition:", t_caret_rect as *const c_void, rect),
        (c"selectionRectsForRange:", t_selection_rects as *const c_void, c"@@:@"),
        (c"closestPositionToPoint:", t_closest as *const c_void, c"@@:{CGPoint=dd}"),
        (c"closestPositionToPoint:withinRange:", t_closest_within as *const c_void, c"@@:{CGPoint=dd}@"),
        (c"characterRangeAtPoint:", t_char_range_at as *const c_void, c"@@:{CGPoint=dd}"),
        (c"textInputView", t_input_view as *const c_void, c"@@:"),
    ]
}

/// (apple_ios_a11y.rs) the text box element's methods: UITextInput, its keys going to the view
pub(crate) fn methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    let mut v = shared();
    v.extend([
        (c"hasText", t_has_text as *const c_void, c"B@:"),
        (c"insertText:", e_insert_text as *const c_void, c"v@:@"),
        (c"deleteBackward", e_delete_backward as *const c_void, c"v@:"),
    ]);
    v
}

/// (apple_ios.rs) JFView's methods of UITextInput (its UIKeyInput methods stay apple_ios.rs's)
pub(crate) fn view_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    shared()
}

/// (tests) the selected range of the text box element with this label: "location,length"
pub(crate) fn selected_of(e: Id) -> Option<(usize, usize)> {
    let r = t_selected(e, core::ptr::null());
    ends(r).map(|(a, b)| (a, b - a))
}

/// (tests) select UTF-16 a .. b in the text box element (UITextInput's setSelectedTextRange:)
pub(crate) fn select_on(e: Id, a: usize, b: usize) {
    t_set_selected(e, core::ptr::null(), range(a, b));
}

/// (tests) the text of UTF-16 a .. b of the element
pub(crate) fn text_of(e: Id, a: usize, b: usize) -> String {
    string_of(t_text_in_range(e, core::ptr::null(), range(a, b)))
}

/// (tests) the caret rectangle at UTF-16 u of the element, in screen points
pub(crate) fn caret_screen(e: Id, u: usize) -> NSRect {
    let r = t_caret_rect(e, core::ptr::null(), pos(u));
    let view = host().view.get();
    if view.is_null() {
        return NSRect::default();
    }
    // SAFETY: UIKit's conversion of a rectangle of our view to the screen.
    unsafe { super::UIAccessibilityConvertFrameToScreenCoordinates(r, view) }
}
