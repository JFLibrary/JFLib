// apple_mac_a11y -- (a11y.rs) the accessibility tree as NSAccessibility elements (VoiceOver on the
// Mac, docs/VOICEOVER.md)
//
//   start(app)                 the bridge may turn on (a11y::start): the first question of an
//                              assistive technology makes a11y::active() true
//   view_methods()             (apple_mac.rs) JFView's accessibility: its children (the top-level
//                              nodes of its window), the focused element, the hit test; a view
//                              itself stays ignored, so the window holds the nodes
//   changed(snapshot, changes) the UI published a new tree: notifications (NSAccessibilityPost-
//                              Notification), the elements of nodes that went away are destroyed
//   forget(pw)                 (win_destroy) the elements of a window go
//   ax_test_tree / ax_test_act / ax_test_listen / ax_test_events   (tests) read and drive the
//                              tree through AppKit's own entry points on the main thread
//                              (accessibilityAttributeValue: and friends, which serve VoiceOver's
//                              requests), from any thread
//
// Every node is an object of our class JFAXElement (a subclass of NSAccessibilityElement), made on
// demand and kept while its node exists, so that VoiceOver sees the same element from frame to
// frame; its instance variable is a slot of the registry, which names (platform window, node key).
// The methods answer from a11y::current() on the main thread, where AppKit asks them and the UI
// runs (never from a frame being built); a node that is gone answers nothing.  Which attributes
// and actions an element has follows its role (isAccessibilitySelectorAllowed:).  Rectangles are
// the node's client pixels / the window's scale, converted to screen points by the view.  Actions
// become a11y::request.  Popups (menus, lists) are panels of their own whose nodes stay in their
// owner's tree (their rectangles lie over the panel); a popup's view answers hit tests with them.
//
// Roles (role / subrole; where the name goes: T title, L label (AXDescription), V value):
//   Label AXStaticText V; Heading AXHeading T, value its level; Button AXButton T; CheckBox
//   AXCheckBox T, value 0 / 1 / 2 (mixed); Radio AXRadioButton T, value 0 / 1; Switch AXCheckBox /
//   AXSwitch T, value 0 / 1; Slider AXSlider L, value, min, max, increment / decrement (5 % of the
//   range, as Android's scrolling); Progress AXProgressIndicator L, value, min, max; TextBox
//   AXTextField / AXTextArea (multi-line) / AXSecureTextField (password) L, value the text; Link
//   AXLink T; List AXList L; ListItem AXStaticText V, selected; MenuBar AXMenuBar; Menu a menu bar
//   entry AXMenuBarItem T, a popup menu AXMenu; MenuItem AXMenuItem T; TabList AXTabGroup L; Tab
//   AXRadioButton / AXTabButton T, value 0 / 1; Combo AXPopUpButton L, value the choice, expanded;
//   Image AXImage L; Group, Canvas AXGroup L; Dialog AXGroup T with the role description "Dialog"
//   (a::role_name, the UI language; a dialog window is an NSWindow of its own anyway).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_void};
use core::sync::atomic::{AtomicBool, Ordering};

use super::super::objc::{
    Id, NIL, NOT_FOUND, NSPoint, NSRange, NSRect, Sel, class, ivar_get, ivar_set, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release, sel,
    string_of, subclass,
};
use super::{POP_BASE, VIEW_OFF, ids, with};
use crate::a11y::{self as ax, Act, Change, Node, Request, Role, Snapshot, WinTree, NONE};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSAccessibilityPostNotification(element: Id, notification: Id);
    fn NSAccessibilityUnignoredAncestor(element: Id) -> Id;
    fn NSAccessibilityRoleDescription(role: Id, subrole: Id) -> Id;
}

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_sync_f(queue: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
}

/// synthetic nodes of the UI (ui/a11y.rs: a popup menu, the menu bar): bit 63 of the key, the
/// kind in bits 40.. (2: the menu or list of a popup)
const SYN: u64 = 1 << 63;
const SYN_POPUP: u64 = 2;

/// the key of a popup's menu (ui/a11y.rs: synthetic, kind popup; a drop-down's list is one too,
/// but only a menu is announced as opened)
fn popup_menu(key: u64) -> bool {
    key & SYN != 0 && (key >> 40) & 0xFF == SYN_POPUP && ax::current().is_none_or(|s| {
        s.wins.iter().all(|w| w.find(key).is_none_or(|k| w.nodes[k].role == Role::Menu))
    })
}

// ---------------------------------------------------------------- the registry

struct Slot {
    pw: i32,
    key: u64,
    obj: Id,
}

struct Reg {
    cls: Id,
    off: isize,
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    map: BTreeMap<(i32, u64), usize>,
    /// NSStrings of our constant texts, by the address of the &'static str
    strs: BTreeMap<usize, Id>,
    /// (tests) the notifications posted: "AXName:node name"
    log: Option<Vec<String>>,
}

struct Main(UnsafeCell<Option<Reg>>);
// SAFETY: only the main thread touches the registry (AppKit's thread, where the UI runs too).
unsafe impl Sync for Main {}
static REG: Main = Main(UnsafeCell::new(None));

static ASKED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);

/// a short borrow of the registry (main thread; never across a call into AppKit that could call
/// back into an element)
fn reg<R>(f: impl FnOnce(&mut Reg) -> R) -> R {
    // SAFETY: main thread only, short borrows (see above); the class is made once.
    unsafe {
        let r = &mut *REG.0.get();
        if r.is_none() {
            let (cls, off) = subclass(c"NSAccessibilityElement", c"JFAXElement", &elem_methods(), &[]);
            *r = Some(Reg { cls, off, slots: Vec::new(), free: Vec::new(), map: BTreeMap::new(), strs: BTreeMap::new(), log: None });
        }
        f(r.as_mut().unwrap_unchecked())
    }
}

/// the NSString of a constant text (made once, kept)
fn ns(s: &'static str) -> Id {
    let k = s.as_ptr() as usize;
    if let Some(o) = reg(|r| r.strs.get(&k).copied()) {
        return o;
    }
    let o = nsstring(s);
    reg(|r| r.strs.insert(k, o));
    o
}

/// an autoreleased NSString
fn text(s: &str) -> Id {
    let o = nsstring(s);
    // SAFETY: autorelease of the string we own; the caller runs inside a pool.
    unsafe { msg!(o, c"autorelease"; Id) }
}

fn number(v: f64) -> Id {
    // SAFETY: an autoreleased NSNumber.
    unsafe { msg!(class(c"NSNumber"), c"numberWithDouble:", v => f64; Id) }
}

fn integer(v: isize) -> Id {
    // SAFETY: an autoreleased NSNumber.
    unsafe { msg!(class(c"NSNumber"), c"numberWithInteger:", v => isize; Id) }
}

fn array(v: &[Id]) -> Id {
    // SAFETY: arrayWithObjects:count: retains the objects (autoreleased array).
    unsafe { msg!(class(c"NSArray"), c"arrayWithObjects:count:", v.as_ptr() => *const Id, v.len() => usize; Id) }
}

/// the element of node key in platform window pw (made on the first question, then the same)
fn elem(pw: i32, key: u64) -> Id {
    if let Some(o) = reg(|r| r.map.get(&(pw, key)).and_then(|&k| r.slots[k].as_ref()).map(|s| s.obj)) {
        return o;
    }
    let cls = reg(|r| r.cls);
    // SAFETY: a new element of our class (+1, the registry keeps it).
    let obj: Id = unsafe {
        let o: Id = msg!(cls, c"alloc"; Id);
        msg!(o, c"init"; Id)
    };
    if obj.is_null() {
        return NIL;
    }
    reg(|r| {
        let k = r.free.pop().unwrap_or_else(|| {
            r.slots.push(None);
            r.slots.len() - 1
        });
        r.slots[k] = Some(Slot { pw, key, obj });
        r.map.insert((pw, key), k);
        ivar_set(obj, r.off, k as isize);
    });
    obj
}

/// (platform window, key) of an element (None: its node went away)
fn slot_of(this: Id) -> Option<(i32, u64)> {
    reg(|r| {
        let k = ivar_get(this, r.off);
        if k < 0 {
            return None;
        }
        r.slots.get(k as usize).and_then(|s| s.as_ref()).map(|s| (s.pw, s.key))
    })
}

/// the tree, its window and the element's node
fn look(this: Id) -> Option<(Arc<Snapshot>, usize, usize, i32)> {
    let (pw, key) = slot_of(this)?;
    let s = ax::current()?;
    let wi = s.wins.iter().position(|w| w.pw == pw)?;
    let ni = s.wins[wi].find(key)?;
    Some((s, wi, ni, pw))
}

fn with_node<R>(this: Id, f: impl FnOnce(&WinTree, &Node, i32) -> R) -> Option<R> {
    let (s, wi, ni, pw) = look(this)?;
    let t = &s.wins[wi];
    Some(f(t, &t.nodes[ni], pw))
}

/// let go of an element: it answers nothing from now on
fn drop_slot(r: &mut Reg, k: usize) -> Option<Id> {
    let s = r.slots.get_mut(k)?.take()?;
    r.map.remove(&(s.pw, s.key));
    ivar_set(s.obj, r.off, -1);
    r.free.push(k);
    Some(s.obj)
}

// ---------------------------------------------------------------- roles

fn role_str(n: &Node) -> &'static str {
    match n.role {
        Role::Group | Role::Canvas | Role::Dialog => "AXGroup",
        Role::Label | Role::ListItem => "AXStaticText",
        Role::Heading => "AXHeading",
        Role::Button => "AXButton",
        Role::CheckBox | Role::Switch => "AXCheckBox",
        Role::Radio | Role::Tab => "AXRadioButton",
        Role::Slider => "AXSlider",
        Role::Progress => "AXProgressIndicator",
        Role::TextBox if n.state & ax::S_MULTILINE != 0 => "AXTextArea",
        Role::TextBox => "AXTextField",
        Role::Link => "AXLink",
        Role::List => "AXList",
        Role::MenuBar => "AXMenuBar",
        Role::Menu if n.key & SYN != 0 => "AXMenu",
        Role::Menu => "AXMenuBarItem",
        Role::MenuItem => "AXMenuItem",
        Role::TabList => "AXTabGroup",
        Role::Combo => "AXPopUpButton",
        Role::Image => "AXImage",
    }
}

fn subrole_str(n: &Node) -> Option<&'static str> {
    match n.role {
        Role::Switch => Some("AXSwitch"),
        Role::Tab => Some("AXTabButton"),
        Role::TextBox if n.state & ax::S_PROTECTED != 0 && n.state & ax::S_MULTILINE == 0 => Some("AXSecureTextField"),
        _ => None,
    }
}

/// where a node's name goes
#[derive(PartialEq)]
enum Name {
    Title,
    Label,
    Value,
}

fn name_slot(n: &Node) -> Name {
    match n.role {
        Role::Label | Role::ListItem => Name::Value,
        Role::Heading | Role::Button | Role::CheckBox | Role::Radio | Role::Switch | Role::Link | Role::Menu | Role::MenuItem | Role::Tab
        | Role::Dialog => Name::Title,
        _ => Name::Label,
    }
}

fn has_value(n: &Node) -> bool {
    matches!(
        n.role,
        Role::Label | Role::ListItem | Role::Heading | Role::CheckBox | Role::Radio | Role::Switch | Role::Tab | Role::Slider | Role::Progress
            | Role::TextBox | Role::Combo
    )
}

fn pressable(r: Role) -> bool {
    matches!(
        r,
        Role::Button | Role::Link | Role::CheckBox | Role::Radio | Role::Switch | Role::Tab | Role::ListItem | Role::Menu | Role::MenuItem
            | Role::Combo
    )
}

fn on(n: &Node, bit: u32) -> bool {
    n.state & bit != 0
}

/// the value of a node (nil: none)
fn value_of(n: &Node) -> Id {
    match n.role {
        Role::Label | Role::ListItem => text(&n.name),
        Role::Heading => integer(n.level() as isize),
        Role::CheckBox => integer(if on(n, ax::S_MIXED) { 2 } else { on(n, ax::S_CHECKED) as isize }),
        Role::Radio | Role::Switch => integer(on(n, ax::S_CHECKED) as isize),
        Role::Tab => integer(on(n, ax::S_SELECTED) as isize),
        Role::Slider => number(n.value),
        Role::Progress if n.value < n.min => NIL, // unknown progress
        Role::Progress => number(n.value),
        Role::TextBox | Role::Combo => text(&n.text),
        _ => NIL,
    }
}

/// the step of increment / decrement (as Android's scroll actions)
fn step(n: &Node) -> f64 {
    let s = (n.max - n.min) / 20.0;
    if n.step > s { n.step } else { s }
}

// ---------------------------------------------------------------- geometry

/// client pixels of window pw -> screen points (Cocoa: origin at the bottom left)
fn to_screen(pw: i32, r: [i32; 4]) -> NSRect {
    let Some((win, view, scale)) = ids(pw) else { return NSRect::default() };
    let local = NSRect::new(r[0] as f64 / scale, r[1] as f64 / scale, r[2] as f64 / scale, r[3] as f64 / scale);
    // SAFETY: convertRect:toView: (nil: the window) and convertRectToScreen: of our view and window.
    unsafe {
        let in_win: NSRect = msg!(view, c"convertRect:toView:", local => NSRect, NIL => Id; NSRect);
        msg!(win, c"convertRectToScreen:", in_win => NSRect; NSRect)
    }
}

/// a screen point -> client pixels of window pw
fn from_screen(pw: i32, p: NSPoint) -> Option<(i32, i32)> {
    let (win, view, scale) = ids(pw)?;
    // SAFETY: convertRectFromScreen: and convertPoint:fromView: of our window and view.
    let q = unsafe {
        let in_win: NSRect = msg!(win, c"convertRectFromScreen:", NSRect { origin: p, ..NSRect::default() } => NSRect; NSRect);
        msg!(view, c"convertPoint:fromView:", in_win.origin => NSPoint, NIL => Id; NSPoint)
    };
    Some(((q.x * scale) as i32, (q.y * scale) as i32))
}

// ---------------------------------------------------------------- the view (JFView)

/// an assistive technology asked: the UI builds the tree from now on
fn touch() {
    ASKED.store(true, Ordering::Release);
    if ENABLED.load(Ordering::Acquire) {
        ax::set_active(true);
    }
}

/// the window of a view: its own, a popup's owner's (and whether it is a popup)
fn view_pw(view: Id) -> Option<(i32, bool)> {
    let v = ivar_get(view, VIEW_OFF.load(Ordering::SeqCst));
    if v >= POP_BASE {
        let owner = with(|s| s.pops.get((v - POP_BASE) as usize).map(|p| p.owner)).flatten()?;
        (owner >= 0).then_some((owner, true))
    } else if v >= 0 {
        Some((v as i32, false))
    } else {
        None
    }
}

extern "C" fn v_children(this: Id, _cmd: Sel) -> Id {
    touch();
    let kids: Vec<Id> = match view_pw(this) {
        Some((pw, false)) => top_level(pw),
        _ => Vec::new(),
    };
    array(&kids)
}

// the elements of window pw's top-level nodes
fn top_level(pw: i32) -> Vec<Id> {
    let Some(s) = ax::current() else { return Vec::new() };
    let Some(t) = s.pw(pw) else { return Vec::new() };
    let keys: Vec<u64> = t.children(NONE).map(|k| t.nodes[k].key).collect();
    keys.into_iter().map(|k| elem(pw, k)).filter(|o| !o.is_null()).collect()
}

extern "C" fn v_focused(this: Id, _cmd: Sel) -> Id {
    touch();
    let focus = view_pw(this).filter(|v| !v.1).and_then(|(pw, _)| {
        let s = ax::current()?;
        let t = s.pw(pw)?;
        let key = t.focus_key();
        (key != 0).then_some((pw, key))
    });
    match focus {
        Some((pw, key)) => elem(pw, key),
        // SAFETY: the unignored ancestor of our view (its window).
        None => unsafe { NSAccessibilityUnignoredAncestor(this) },
    }
}

extern "C" fn v_hit_test(this: Id, _cmd: Sel, p: NSPoint) -> Id {
    touch();
    let hit = view_pw(this).and_then(|(pw, _)| {
        let (x, y) = from_screen(pw, p)?;
        let s = ax::current()?;
        let t = s.pw(pw)?;
        t.at_point(x, y).map(|k| (pw, t.nodes[k].key))
    });
    match hit {
        Some((pw, key)) => elem(pw, key),
        // SAFETY: as in v_focused.
        None => unsafe { NSAccessibilityUnignoredAncestor(this) },
    }
}

/// JFView's methods of the bridge
pub(crate) fn view_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    alloc::vec![
        (c"accessibilityChildren", v_children as *const c_void, c"@@:"),
        (c"accessibilityFocusedUIElement", v_focused as *const c_void, c"@@:"),
        (c"accessibilityHitTest:", v_hit_test as *const c_void, c"@@:{CGPoint=dd}"),
    ]
}

// ---------------------------------------------------------------- the elements (JFAXElement)

// SAFETY (all methods below): AppKit calls them on the main thread with one of our elements.

extern "C" fn e_role(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| ns(role_str(n))).unwrap_or_else(|| ns("AXUnknown"))
}

extern "C" fn e_subrole(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| subrole_str(n).map_or(NIL, ns)).unwrap_or(NIL)
}

extern "C" fn e_role_description(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| match n.role {
        // the roles AppKit has no name for: the bridges' own (in the UI language)
        Role::Dialog | Role::Heading => text(ax::role_name(n.role)),
        // SAFETY: AppKit's localized name of a role / subrole.
        _ => unsafe { NSAccessibilityRoleDescription(ns(role_str(n)), subrole_str(n).map_or(NIL, ns)) },
    })
    .unwrap_or(NIL)
}

extern "C" fn e_title(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| if name_slot(n) == Name::Title { text(&n.name) } else { NIL }).unwrap_or(NIL)
}

extern "C" fn e_label(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| if name_slot(n) == Name::Label && !n.name.is_empty() { text(&n.name) } else { NIL }).unwrap_or(NIL)
}

extern "C" fn e_value(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| value_of(n)).unwrap_or(NIL)
}

extern "C" fn e_identifier(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| text(&format!("{:x}", n.key))).unwrap_or(NIL)
}

extern "C" fn e_parent(this: Id, _cmd: Sel) -> Id {
    let Some((pw, parent)) = with_node(this, |t, n, pw| (pw, if n.parent == NONE { 0 } else { t.nodes[n.parent as usize].key })) else {
        return NIL;
    };
    if parent != 0 {
        return elem(pw, parent);
    }
    match ids(pw) {
        // SAFETY: the unignored ancestor of the window's view: the window.
        Some((_, view, _)) => unsafe { NSAccessibilityUnignoredAncestor(view) },
        None => NIL,
    }
}

// the elements of a node's children that pass a filter
fn kids(this: Id, keep: impl Fn(&WinTree, &Node) -> bool) -> Id {
    let kids = with_node(this, |t, n, pw| {
        let k = t.find(n.key).unwrap_or(0) as u32;
        (pw, t.children(k).filter(|&c| keep(t, &t.nodes[c])).map(|c| t.nodes[c].key).collect::<Vec<u64>>())
    });
    let objs: Vec<Id> = kids.map_or(Vec::new(), |(pw, keys)| keys.into_iter().map(|k| elem(pw, k)).collect());
    array(&objs)
}

extern "C" fn e_children(this: Id, _cmd: Sel) -> Id {
    kids(this, |_, _| true)
}

extern "C" fn e_visible_children(this: Id, _cmd: Sel) -> Id {
    kids(this, |t, n| t.on_screen(n))
}

extern "C" fn e_selected_children(this: Id, _cmd: Sel) -> Id {
    kids(this, |_, n| on(n, ax::S_SELECTED))
}

extern "C" fn e_window(this: Id, _cmd: Sel) -> Id {
    slot_of(this).and_then(|(pw, _)| ids(pw)).map_or(NIL, |(win, _, _)| win)
}

extern "C" fn e_frame(this: Id, _cmd: Sel) -> NSRect {
    match with_node(this, |_, n, pw| (pw, n.rect)) {
        Some((pw, r)) => to_screen(pw, r),
        None => NSRect::default(),
    }
}

/// where VoiceOver clicks when it has no action to send: the middle of the frame
extern "C" fn e_activation_point(this: Id, _cmd: Sel) -> NSPoint {
    let f = e_frame(this, _cmd);
    NSPoint { x: f.origin.x + f.size.w / 2.0, y: f.origin.y + f.size.h / 2.0 }
}

extern "C" fn e_yes(_this: Id, _cmd: Sel) -> bool {
    true
}

extern "C" fn e_enabled(this: Id, _cmd: Sel) -> bool {
    with_node(this, |_, n, _| !on(n, ax::S_DISABLED)).unwrap_or(false)
}

extern "C" fn e_focused(this: Id, _cmd: Sel) -> bool {
    with_node(this, |t, n, _| t.active && on(n, ax::S_FOCUSED)).unwrap_or(false)
}

extern "C" fn e_selected(this: Id, _cmd: Sel) -> bool {
    with_node(this, |_, n, _| on(n, ax::S_SELECTED)).unwrap_or(false)
}

extern "C" fn e_expanded(this: Id, _cmd: Sel) -> bool {
    with_node(this, |_, n, _| on(n, ax::S_EXPANDED)).unwrap_or(false)
}

extern "C" fn e_min(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| number(n.min)).unwrap_or(NIL)
}

extern "C" fn e_max(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| number(n.max)).unwrap_or(NIL)
}

extern "C" fn e_orientation(_this: Id, _cmd: Sel) -> isize {
    2 // NSAccessibilityOrientationHorizontal
}

/// a request for the element's node when it allows the action
fn act(this: Id, need: u32, a: Act) -> bool {
    let Some((win, key, ok)) = with_node(this, |t, n, _| (t.win, n.key, n.actions & need != 0)) else { return false };
    if ok {
        ax::request(Request { win, key, act: a });
    }
    ok
}

extern "C" fn e_press(this: Id, _cmd: Sel) -> bool {
    act(this, ax::A_CLICK, Act::Click)
}

fn nudge(this: Id, up: bool) -> bool {
    let Some(v) = with_node(this, |_, n, _| {
        let v = if up { n.value + step(n) } else { n.value - step(n) };
        if v < n.min { n.min } else if v > n.max { n.max } else { v }
    }) else {
        return false;
    };
    act(this, ax::A_VALUE, Act::Value(v))
}

extern "C" fn e_increment(this: Id, _cmd: Sel) -> bool {
    nudge(this, true)
}

extern "C" fn e_decrement(this: Id, _cmd: Sel) -> bool {
    nudge(this, false)
}

extern "C" fn e_set_value(this: Id, _cmd: Sel, v: Id) {
    if v.is_null() {
        return;
    }
    match with_node(this, |_, n, _| n.role) {
        // SAFETY: doubleValue of an NSNumber or NSString (both answer it).
        Some(Role::Slider) => {
            let x: f64 = unsafe { msg!(v, c"doubleValue"; f64) };
            act(this, ax::A_VALUE, Act::Value(x));
        }
        Some(Role::TextBox) => {
            act(this, ax::A_TEXT, Act::Text(string_of(v)));
        }
        _ => {}
    }
}

extern "C" fn e_set_focused(this: Id, _cmd: Sel, yes: bool) {
    if yes {
        act(this, ax::A_FOCUS, Act::Focus);
    }
}

extern "C" fn e_set_selected(this: Id, _cmd: Sel, yes: bool) {
    // single selection: selecting a row or tab is its click; nothing takes it out again
    if yes && with_node(this, |_, n, _| !on(n, ax::S_SELECTED)) == Some(true) {
        act(this, ax::A_CLICK, Act::Click);
    }
}

extern "C" fn e_hit_test(this: Id, _cmd: Sel, p: NSPoint) -> Id {
    let hit = with_node(this, |t, n, pw| {
        let (x, y) = from_screen(pw, p)?;
        // the deepest node at the point below this one
        let k = t.find(n.key)?;
        let inside = |r: &[i32; 4]| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
        let mut at = k;
        while let Some(c) = t.children(at as u32).filter(|&c| inside(&t.nodes[c].rect)).last() {
            at = c;
        }
        Some((pw, t.nodes[at].key))
    })
    .flatten();
    match hit {
        Some((pw, key)) => elem(pw, key),
        None => this,
    }
}

// ---------------------------------------------------------------- text (text boxes)
//
// VoiceOver reads a text box by character, word and line and follows its caret through these:
// offsets are UTF-16 units of the text (NSString's), the node's caret and anchor characters.
// Rectangles come from Node::char_rect (exact at the caret, estimated elsewhere).

/// the UTF-16 offset of character k of s (past the end: the length)
fn u16_at(s: &str, k: i32) -> usize {
    s.chars().take(k.max(0) as usize).map(char::len_utf16).sum()
}

/// the character at UTF-16 offset u of s (inside a pair: the character it belongs to; past the
/// end: the number of characters)
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

/// characters a .. b of s as a UTF-16 range
fn range_of(s: &str, a: i32, b: i32) -> NSRange {
    let x = u16_at(s, a);
    NSRange { location: x, length: u16_at(s, b.max(a)) - x }
}

/// a UTF-16 range of s as characters a .. b
fn chars_of(s: &str, r: NSRange) -> (i32, i32) {
    let end = if r.length == 0 { r.location } else { r.location.saturating_add(r.length) };
    let a = char_at(s, r.location);
    let b = if end == r.location { a } else { char_at(s, end - 1) + 1 };
    (a, b.max(a))
}

/// the line of character k (lines end after a '\n')
fn line_of(s: &str, k: i32) -> isize {
    s.chars().take(k.max(0) as usize).filter(|&c| c == '\n').count() as isize
}

/// the characters of line l, its '\n' included
fn line_chars(s: &str, l: isize) -> Option<(i32, i32)> {
    let (mut line, mut start) = (0, 0);
    for (k, c) in s.chars().enumerate() {
        if c == '\n' {
            if line == l {
                return Some((start, k as i32 + 1));
            }
            line += 1;
            start = k as i32 + 1;
        }
    }
    (line == l).then(|| (start, s.chars().count() as i32))
}

fn with_text<R>(this: Id, f: impl FnOnce(&Node, &str, i32) -> R) -> Option<R> {
    with_node(this, |_, n, pw| f(n, n.text_str(), pw))
}

fn nsvalue_range(r: NSRange) -> Id {
    // SAFETY: an autoreleased NSValue of a range.
    unsafe { msg!(class(c"NSValue"), c"valueWithRange:", r => NSRange; Id) }
}

extern "C" fn t_count(this: Id, _cmd: Sel) -> isize {
    with_text(this, |_, s, _| s.encode_utf16().count() as isize).unwrap_or(0)
}

extern "C" fn t_selected_text(this: Id, _cmd: Sel) -> Id {
    with_text(this, |n, s, _| {
        let (a, b) = n.selection();
        text(&s.chars().skip(a as usize).take((b - a) as usize).collect::<String>())
    })
    .unwrap_or(NIL)
}

fn selected_range(this: Id) -> NSRange {
    with_text(this, |n, s, _| {
        let (a, b) = n.selection();
        range_of(s, a, b)
    })
    .unwrap_or_default()
}

extern "C" fn t_selected_range(this: Id, _cmd: Sel) -> NSRange {
    selected_range(this)
}

extern "C" fn t_selected_ranges(this: Id, _cmd: Sel) -> Id {
    array(&[nsvalue_range(selected_range(this))])
}

extern "C" fn t_set_selected_range(this: Id, _cmd: Sel, r: NSRange) {
    let Some((a, b)) = with_text(this, |_, s, _| chars_of(s, r)) else { return };
    // the caret at the end of the range (VoiceOver moves it there when it sets one)
    act(this, ax::A_FOCUS, Act::Select(a, b));
}

extern "C" fn t_insertion_line(this: Id, _cmd: Sel) -> isize {
    with_text(this, |n, s, _| line_of(s, n.caret)).unwrap_or(0)
}

extern "C" fn t_visible_range(this: Id, _cmd: Sel) -> NSRange {
    with_text(this, |_, s, _| NSRange { location: 0, length: s.encode_utf16().count() }).unwrap_or_default()
}

extern "C" fn t_string_for_range(this: Id, _cmd: Sel, r: NSRange) -> Id {
    with_text(this, |_, s, _| {
        let (a, b) = chars_of(s, r);
        text(&s.chars().skip(a as usize).take((b - a) as usize).collect::<String>())
    })
    .unwrap_or(NIL)
}

extern "C" fn t_attributed_for_range(this: Id, cmd: Sel, r: NSRange) -> Id {
    let plain = t_string_for_range(this, cmd, r);
    if plain.is_null() {
        return NIL;
    }
    // SAFETY: an autoreleased NSAttributedString of the plain text.
    unsafe {
        let a: Id = msg!(class(c"NSAttributedString"), c"alloc"; Id);
        let a: Id = msg!(a, c"initWithString:", plain => Id; Id);
        msg!(a, c"autorelease"; Id)
    }
}

extern "C" fn t_range_for_line(this: Id, _cmd: Sel, l: isize) -> NSRange {
    with_text(this, |_, s, _| line_chars(s, l).map_or(NSRange { location: NOT_FOUND, length: 0 }, |(a, b)| range_of(s, a, b)))
        .unwrap_or(NSRange { location: NOT_FOUND, length: 0 })
}

extern "C" fn t_line_for_index(this: Id, _cmd: Sel, i: isize) -> isize {
    with_text(this, |_, s, _| line_of(s, char_at(s, i.max(0) as usize))).unwrap_or(0)
}

/// the range of the (composed) character at character k: one character, nothing at the end
fn char_range(s: &str, k: i32) -> NSRange {
    let n = s.chars().count() as i32;
    range_of(s, k.min(n), (k + 1).min(n))
}

extern "C" fn t_range_for_index(this: Id, _cmd: Sel, i: isize) -> NSRange {
    with_text(this, |_, s, _| char_range(s, char_at(s, i.max(0) as usize))).unwrap_or_default()
}

extern "C" fn t_range_for_position(this: Id, _cmd: Sel, p: NSPoint) -> NSRange {
    with_text(this, |n, s, pw| {
        let (x, y) = from_screen(pw, p)?;
        let k = n.offset_at(x, y);
        (k >= 0).then(|| char_range(s, k))
    })
    .flatten()
    .unwrap_or(NSRange { location: NOT_FOUND, length: 0 })
}

extern "C" fn t_frame_for_range(this: Id, _cmd: Sel, r: NSRange) -> NSRect {
    let Some((pw, rect)) = with_text(this, |n, s, pw| {
        let (a, b) = chars_of(s, r);
        let mut u = n.char_rect(a);
        for k in a + 1..b {
            let q = n.char_rect(k);
            let (x0, y0) = (u[0].min(q[0]), u[1].min(q[1]));
            let (x1, y1) = ((u[0] + u[2]).max(q[0] + q[2]), (u[1] + u[3]).max(q[1] + q[3]));
            u = [x0, y0, x1 - x0, y1 - y0];
        }
        (pw, u)
    }) else {
        return NSRect::default();
    };
    to_screen(pw, rect)
}

extern "C" fn t_style_range(this: Id, cmd: Sel, _i: isize) -> NSRange {
    t_visible_range(this, cmd) // one style for the whole text
}

/// the text selectors (only text boxes have them)
const TEXT_SELS: [&CStr; 13] = [
    c"accessibilityNumberOfCharacters",
    c"accessibilitySelectedText",
    c"accessibilitySelectedTextRange",
    c"accessibilitySelectedTextRanges",
    c"accessibilityInsertionPointLineNumber",
    c"accessibilityVisibleCharacterRange",
    c"accessibilityStringForRange:",
    c"accessibilityAttributedStringForRange:",
    c"accessibilityRangeForLine:",
    c"accessibilityLineForIndex:",
    c"accessibilityRangeForIndex:",
    c"accessibilityRangeForPosition:",
    c"accessibilityFrameForRange:",
];

fn text_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    let range = c"{_NSRange=QQ}@:";
    let range_q = c"{_NSRange=QQ}@:q";
    alloc::vec![
        (c"accessibilityNumberOfCharacters", t_count as *const c_void, c"q@:"),
        (c"accessibilitySelectedText", t_selected_text as *const c_void, c"@@:"),
        (c"accessibilitySelectedTextRange", t_selected_range as *const c_void, range),
        (c"accessibilitySelectedTextRanges", t_selected_ranges as *const c_void, c"@@:"),
        (c"setAccessibilitySelectedTextRange:", t_set_selected_range as *const c_void, c"v@:{_NSRange=QQ}"),
        (c"accessibilityInsertionPointLineNumber", t_insertion_line as *const c_void, c"q@:"),
        (c"accessibilityVisibleCharacterRange", t_visible_range as *const c_void, range),
        (c"accessibilityStringForRange:", t_string_for_range as *const c_void, c"@@:{_NSRange=QQ}"),
        (c"accessibilityAttributedStringForRange:", t_attributed_for_range as *const c_void, c"@@:{_NSRange=QQ}"),
        (c"accessibilityRangeForLine:", t_range_for_line as *const c_void, range_q),
        (c"accessibilityLineForIndex:", t_line_for_index as *const c_void, c"q@:q"),
        (c"accessibilityRangeForIndex:", t_range_for_index as *const c_void, range_q),
        (c"accessibilityRangeForPosition:", t_range_for_position as *const c_void, c"{_NSRange=QQ}@:{CGPoint=dd}"),
        (c"accessibilityFrameForRange:", t_frame_for_range as *const c_void, c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{_NSRange=QQ}"),
        (c"accessibilityStyleRangeForIndex:", t_style_range as *const c_void, range_q),
    ]
}

// the selectors whose answer depends on the role (all others: every element)
extern "C" fn e_allowed(this: Id, _cmd: Sel, s: Sel) -> bool {
    let Some((role, state, actions, slot, value)) = with_node(this, |_, n, _| (n.role, n.state, n.actions, name_slot(n), has_value(n))) else {
        return true;
    };
    let is = |name: &CStr| s == sel(name);
    if TEXT_SELS.iter().any(|t| is(t)) || is(c"accessibilityStyleRangeForIndex:") {
        role == Role::TextBox
    } else if is(c"setAccessibilitySelectedTextRange:") {
        role == Role::TextBox && actions & ax::A_FOCUS != 0
    } else if is(c"accessibilityTitle") {
        slot == Name::Title
    } else if is(c"accessibilityLabel") {
        slot == Name::Label
    } else if is(c"accessibilityValue") {
        value
    } else if is(c"setAccessibilityValue:") {
        (role == Role::Slider && actions & ax::A_VALUE != 0) || (role == Role::TextBox && actions & ax::A_TEXT != 0)
    } else if is(c"accessibilityPerformPress") {
        pressable(role)
    } else if is(c"accessibilityPerformIncrement") || is(c"accessibilityPerformDecrement") || is(c"accessibilityOrientation") {
        role == Role::Slider
    } else if is(c"accessibilityMinValue") || is(c"accessibilityMaxValue") {
        matches!(role, Role::Slider | Role::Progress)
    } else if is(c"setAccessibilityFocused:") {
        state & ax::S_FOCUSABLE != 0
    } else if is(c"isAccessibilitySelected") || is(c"setAccessibilitySelected:") {
        matches!(role, Role::ListItem | Role::Tab)
    } else if is(c"isAccessibilityExpanded") {
        matches!(role, Role::Combo | Role::Menu)
    } else if is(c"accessibilitySelectedChildren") {
        matches!(role, Role::List | Role::TabList)
    } else if is(c"accessibilityTabs") {
        role == Role::TabList
    } else {
        // the other actions of NSAccessibilityElement (show menu, confirm, cancel, ..): none
        !sel_name(s).starts_with("accessibilityPerform")
    }
}

/// the name of a selector
fn sel_name(s: Sel) -> &'static str {
    // SAFETY: sel_getName gives the selector's NUL-terminated name, alive for the process.
    unsafe { CStr::from_ptr(sel_getName(s)).to_str().unwrap_or("") }
}

unsafe extern "C" {
    fn sel_getName(s: Sel) -> *const core::ffi::c_char;
}

fn elem_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    let id = c"@@:";
    let b = c"B@:";
    let mut v = alloc::vec![
        (c"accessibilityRole", e_role as *const c_void, id),
        (c"accessibilitySubrole", e_subrole as *const c_void, id),
        (c"accessibilityRoleDescription", e_role_description as *const c_void, id),
        (c"accessibilityTitle", e_title as *const c_void, id),
        (c"accessibilityLabel", e_label as *const c_void, id),
        (c"accessibilityValue", e_value as *const c_void, id),
        (c"accessibilityIdentifier", e_identifier as *const c_void, id),
        (c"accessibilityParent", e_parent as *const c_void, id),
        (c"accessibilityChildren", e_children as *const c_void, id),
        // (NSAccessibilityElement answers these from storage of its own, which we leave empty)
        (c"accessibilityChildrenInNavigationOrder", e_children as *const c_void, id),
        (c"accessibilityVisibleChildren", e_visible_children as *const c_void, id),
        (c"accessibilityTabs", e_children as *const c_void, id),
        (c"accessibilitySelectedChildren", e_selected_children as *const c_void, id),
        (c"accessibilityActivationPoint", e_activation_point as *const c_void, c"{CGPoint=dd}@:"),
        (c"accessibilityWindow", e_window as *const c_void, id),
        (c"accessibilityTopLevelUIElement", e_window as *const c_void, id),
        (c"accessibilityFrame", e_frame as *const c_void, c"{CGRect={CGPoint=dd}{CGSize=dd}}@:"),
        (c"isAccessibilityElement", e_yes as *const c_void, b),
        (c"accessibilityNotifiesWhenDestroyed", e_yes as *const c_void, b),
        (c"isAccessibilityEnabled", e_enabled as *const c_void, b),
        (c"isAccessibilityFocused", e_focused as *const c_void, b),
        (c"setAccessibilityFocused:", e_set_focused as *const c_void, c"v@:B"),
        (c"isAccessibilitySelected", e_selected as *const c_void, b),
        (c"setAccessibilitySelected:", e_set_selected as *const c_void, c"v@:B"),
        (c"isAccessibilityExpanded", e_expanded as *const c_void, b),
        (c"accessibilityMinValue", e_min as *const c_void, id),
        (c"accessibilityMaxValue", e_max as *const c_void, id),
        (c"accessibilityOrientation", e_orientation as *const c_void, c"q@:"),
        (c"setAccessibilityValue:", e_set_value as *const c_void, c"v@:@"),
        (c"accessibilityPerformPress", e_press as *const c_void, b),
        (c"accessibilityPerformIncrement", e_increment as *const c_void, b),
        (c"accessibilityPerformDecrement", e_decrement as *const c_void, b),
        (c"accessibilityHitTest:", e_hit_test as *const c_void, c"@@:{CGPoint=dd}"),
        (c"isAccessibilitySelectorAllowed:", e_allowed as *const c_void, c"B@::"),
    ];
    v.extend(text_methods());
    v
}

// ---------------------------------------------------------------- the bridge

pub fn start(_app: &str) {
    ENABLED.store(true, Ordering::Release);
    if ASKED.load(Ordering::Acquire) {
        ax::set_active(true);
    }
}

/// post a notification for an element (and note it for a test)
fn post(obj: Id, what: &'static str, name: &str) {
    if obj.is_null() {
        return;
    }
    reg(|r| {
        if let Some(log) = r.log.as_mut() {
            log.push(format!("{what}:{name}"));
        }
    });
    // SAFETY: AppKit's notification for our element (or window).
    unsafe { NSAccessibilityPostNotification(obj, ns(what)) };
}

pub fn changed(s: &Snapshot, c: Vec<Change>) {
    // SAFETY: a pool for the autoreleased objects of the notifications (we run in the UI's frame).
    let pool = unsafe { objc_autoreleasePoolPush() };
    let mut layout: Vec<i32> = Vec::new();
    for ch in &c {
        let (w, key) = match ch {
            Change::Focus(w, k) | Change::State(w, k, ..) | Change::Name(w, k) | Change::Value(w, k) | Change::Text(w, k, _) | Change::Caret(w, k, _) => {
                (*w, *k)
            }
            Change::Added(w, _, key) | Change::Removed(w, _, key, _) => {
                if let Some(t) = s.win(*w) {
                    if !layout.contains(&t.pw) {
                        layout.push(t.pw);
                    }
                    // a popup menu opened or closed (VoiceOver says so and moves into it)
                    if popup_menu(*key) && ids(t.pw).is_some() {
                        if matches!(ch, Change::Added(..)) {
                            post(elem(t.pw, *key), "AXMenuOpened", "");
                        } else if let Some(e) = reg(|r| r.map.get(&(t.pw, *key)).and_then(|&k| r.slots[k].as_ref()).map(|s| s.obj)) {
                            post(e, "AXMenuClosed", "");
                        }
                    }
                }
                continue;
            }
            _ => continue,
        };
        let Some(t) = s.win(w) else { continue };
        let Some(n) = t.find(key).map(|k| &t.nodes[k]) else { continue };
        if ids(t.pw).is_none() {
            continue;
        }
        let e = elem(t.pw, key);
        match ch {
            Change::Focus(..) => post(e, "AXFocusedUIElementChanged", &n.name),
            Change::State(_, _, bit, _) => match *bit {
                ax::S_CHECKED | ax::S_MIXED if has_value(n) => post(e, "AXValueChanged", &n.name),
                ax::S_SELECTED => {
                    if has_value(n) {
                        post(e, "AXValueChanged", &n.name);
                    }
                    // a list's / tab strip's selected children
                    if let Some(p) = t.nodes.get(n.parent as usize).filter(|p| matches!(p.role, Role::List | Role::TabList)) {
                        post(elem(t.pw, p.key), "AXSelectedChildrenChanged", &p.name);
                    }
                }
                ax::S_EXPANDED => post(e, "AXExpandedChanged", &n.name),
                _ => {}
            },
            // a label's text is its value
            Change::Name(..) if name_slot(n) == Name::Value => post(e, "AXValueChanged", &n.name),
            Change::Name(..) => post(e, "AXTitleChanged", &n.name),
            Change::Value(..) | Change::Text(..) => post(e, "AXValueChanged", &n.name),
            Change::Caret(..) => post(e, "AXSelectedTextChanged", &n.name),
            _ => {}
        }
    }
    for pw in layout {
        if let Some((win, _, _)) = ids(pw) {
            post(win, "AXLayoutChanged", "");
        }
    }
    sweep(s);
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
}

/// the elements whose nodes went away are destroyed (VoiceOver lets go of them)
fn sweep(s: &Snapshot) {
    let gone: Vec<usize> = reg(|r| {
        r.slots
            .iter()
            .enumerate()
            .filter_map(|(k, sl)| sl.as_ref().map(|sl| (k, sl)))
            .filter(|(_, sl)| s.pw(sl.pw).is_none_or(|t| t.find(sl.key).is_none()))
            .map(|(k, _)| k)
            .collect()
    });
    for k in gone {
        if let Some(obj) = reg(|r| drop_slot(r, k)) {
            post(obj, "AXUIElementDestroyed", "");
            release(obj);
        }
    }
}

/// (win_destroy) the elements of window pw go with it
pub(crate) fn forget(pw: i32) {
    let gone: Vec<Id> = reg(|r| {
        let ks: Vec<usize> = r.map.range((pw, 0)..=(pw, u64::MAX)).map(|(_, &k)| k).collect();
        ks.into_iter().filter_map(|k| drop_slot(r, k)).collect()
    });
    for obj in gone {
        post(obj, "AXUIElementDestroyed", "");
        release(obj);
    }
}

// ---------------------------------------------------------------- tests: AppKit's own entry points

/// an element as VoiceOver reads it (AppKit's attributes)
#[derive(Clone, Debug, Default)]
pub struct AxTestElem {
    pub depth: u32,
    pub role: String,
    pub subrole: String,
    pub role_desc: String,
    pub title: String,
    /// AXDescription
    pub label: String,
    /// AXValue as text (numbers: their description, "1", "75")
    pub value: String,
    pub ident: String,
    pub enabled: bool,
    pub focused: bool,
    pub selected: bool,
    pub expanded: bool,
    /// AXPosition (top left of the primary screen) and AXSize, points
    pub frame: [f64; 4],
    pub actions: Vec<String>,
    pub attrs: Vec<String>,
    pub value_settable: bool,
}

impl AxTestElem {
    /// the name VoiceOver reads: title, else description, else value
    pub fn name(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.label.is_empty() {
            &self.label
        } else {
            &self.value
        }
    }
}

/// an action of a test
#[derive(Clone, Debug)]
pub enum AxTestAct {
    Press,
    Increment,
    Decrement,
    /// AXValue := a number (sliders)
    Number(f64),
    /// AXValue := a text (text boxes)
    Text(String),
    /// AXFocused := YES
    Focus,
    /// AXSelected := YES
    Select,
    /// AXSelectedTextRange := location, length (UTF-16)
    Range(usize, usize),
}

/// a question of a test about a text box's text (the answers: numbers as text, ranges
/// "location,length", rectangles "x,y,w,h" in points from the top left of the primary screen)
#[derive(Clone, Debug)]
pub enum AxTestText {
    /// AXNumberOfCharacters
    Count,
    /// AXSelectedText
    Selected,
    /// AXSelectedTextRange
    SelRange,
    /// AXInsertionPointLineNumber
    Line,
    /// AXStringForRange (location, length)
    StringFor(usize, usize),
    /// AXAttributedStringForRange: its string
    AttrFor(usize, usize),
    /// AXBoundsForRange
    FrameFor(usize, usize),
    /// AXRangeForPosition (a point as AXPosition)
    RangeAt(f64, f64),
    /// AXRangeForIndex
    RangeFor(usize),
    /// AXLineForIndex
    LineFor(usize),
    /// AXRangeForLine
    RangeForLine(usize),
}

/// f on the main thread (where AppKit serves the assistive technology), inside a pool
fn on_main<R>(f: impl FnOnce() -> R) -> R {
    let mut f = Some(f);
    let mut out: Option<R> = None;
    let mut call = || {
        // SAFETY: a pool around the work.
        let pool = unsafe { objc_autoreleasePoolPush() };
        out = f.take().map(|f| f());
        unsafe { objc_autoreleasePoolPop(pool) };
    };
    // SAFETY: pthread_main_np has no preconditions.
    if unsafe { pthread_main_np() } != 0 {
        call();
    } else {
        let mut dynf: &mut dyn FnMut() = &mut call;
        extern "C" fn tramp(ctx: *mut c_void) {
            // SAFETY: ctx is the &mut dyn FnMut of on_main, alive until dispatch_sync_f returns.
            let f = unsafe { &mut *(ctx as *mut &mut dyn FnMut()) };
            f();
        }
        // SAFETY: the main queue runs tramp on the main thread while this thread waits for it.
        unsafe { dispatch_sync_f((&raw const _dispatch_main_q).cast_mut(), &mut dynf as *mut &mut dyn FnMut() as *mut c_void, tramp) };
    }
    out.expect("on_main: the work ran")
}

fn attr(e: Id, name: &'static str) -> Id {
    // SAFETY: AppKit's own attribute entry point of its objects (a window, the application).
    unsafe { msg!(e, c"accessibilityAttributeValue:", ns(name) => Id; Id) }
}

/// the text of any object (numbers: their description)
fn describe_obj(v: Id) -> String {
    if v.is_null() {
        return String::new();
    }
    // SAFETY: isKindOfClass: / description of a live object.
    unsafe {
        if msg!(v, c"isKindOfClass:", class(c"NSString") => Id; bool) {
            string_of(v)
        } else {
            string_of(msg!(v, c"description"; Id))
        }
    }
}

fn elements(a: Id) -> Vec<Id> {
    if a.is_null() {
        return Vec::new();
    }
    // SAFETY: an NSArray of elements.
    unsafe {
        let n: usize = msg!(a, c"count"; usize);
        (0..n).map(|i| msg!(a, c"objectAtIndex:", i => usize; Id)).collect()
    }
}

/// the element has method m and allows it -- the rule by which AppKit serves the protocol's
/// attributes and actions to assistive technology (isAccessibilitySelectorAllowed:)
fn offers(e: Id, m: &CStr) -> bool {
    // SAFETY: respondsToSelector: / isAccessibilitySelectorAllowed: of an accessibility element.
    unsafe {
        msg!(e, c"respondsToSelector:", sel(m) => Sel; bool)
            && (!msg!(e, c"respondsToSelector:", sel(c"isAccessibilitySelectorAllowed:") => Sel; bool)
                || msg!(e, c"isAccessibilitySelectorAllowed:", sel(m) => Sel; bool))
    }
}

/// an object attribute through its getter (nil: not offered)
fn get(e: Id, m: &CStr) -> Id {
    // SAFETY: a getter of the protocol returning an object, offered by the element.
    if offers(e, m) { unsafe { msg!(e, m; Id) } } else { NIL }
}

fn get_bool(e: Id, m: &CStr) -> bool {
    // SAFETY: a BOOL getter of the protocol, offered by the element.
    offers(e, m) && unsafe { msg!(e, m; bool) }
}

// the attributes of the protocol VoiceOver reads (AX name, getter)
const ATTRS: [(&str, &CStr); 22] = [
    ("AXRole", c"accessibilityRole"),
    ("AXSubrole", c"accessibilitySubrole"),
    ("AXRoleDescription", c"accessibilityRoleDescription"),
    ("AXTitle", c"accessibilityTitle"),
    ("AXDescription", c"accessibilityLabel"),
    ("AXValue", c"accessibilityValue"),
    ("AXIdentifier", c"accessibilityIdentifier"),
    ("AXParent", c"accessibilityParent"),
    ("AXChildren", c"accessibilityChildren"),
    ("AXSelectedChildren", c"accessibilitySelectedChildren"),
    ("AXWindow", c"accessibilityWindow"),
    ("AXFrame", c"accessibilityFrame"),
    ("AXEnabled", c"isAccessibilityEnabled"),
    ("AXFocused", c"isAccessibilityFocused"),
    ("AXSelected", c"isAccessibilitySelected"),
    ("AXExpanded", c"isAccessibilityExpanded"),
    ("AXMinValue", c"accessibilityMinValue"),
    ("AXMaxValue", c"accessibilityMaxValue"),
    ("AXOrientation", c"accessibilityOrientation"),
    ("AXNumberOfCharacters", c"accessibilityNumberOfCharacters"),
    ("AXSelectedTextRange", c"accessibilitySelectedTextRange"),
    ("AXInsertionPointLineNumber", c"accessibilityInsertionPointLineNumber"),
];

// the actions of the protocol (AX name, method)
const ACTIONS: [(&str, &CStr); 10] = [
    ("AXPress", c"accessibilityPerformPress"),
    ("AXIncrement", c"accessibilityPerformIncrement"),
    ("AXDecrement", c"accessibilityPerformDecrement"),
    ("AXShowMenu", c"accessibilityPerformShowMenu"),
    ("AXConfirm", c"accessibilityPerformConfirm"),
    ("AXCancel", c"accessibilityPerformCancel"),
    ("AXPick", c"accessibilityPerformPick"),
    ("AXRaise", c"accessibilityPerformRaise"),
    ("AXDelete", c"accessibilityPerformDelete"),
    ("AXShowAlternateUI", c"accessibilityPerformShowAlternateUI"),
];

fn describe(e: Id, depth: u32) -> AxTestElem {
    let s = |m: &CStr| describe_obj(get(e, m));
    let mut d = AxTestElem {
        depth,
        role: s(c"accessibilityRole"),
        subrole: s(c"accessibilitySubrole"),
        role_desc: s(c"accessibilityRoleDescription"),
        title: s(c"accessibilityTitle"),
        label: s(c"accessibilityLabel"),
        value: s(c"accessibilityValue"),
        ident: s(c"accessibilityIdentifier"),
        enabled: get_bool(e, c"isAccessibilityEnabled"),
        focused: get_bool(e, c"isAccessibilityFocused"),
        selected: get_bool(e, c"isAccessibilitySelected"),
        expanded: get_bool(e, c"isAccessibilityExpanded"),
        ..AxTestElem::default()
    };
    d.attrs = ATTRS.iter().filter(|(_, m)| offers(e, m)).map(|(a, _)| String::from(*a)).collect();
    d.actions = ACTIONS.iter().filter(|(_, m)| offers(e, m)).map(|(a, _)| String::from(*a)).collect();
    d.value_settable = offers(e, c"setAccessibilityValue:");
    if offers(e, c"accessibilityFrame") {
        // SAFETY: the frame getter (screen points, Cocoa: y from the bottom of the primary screen).
        let f: NSRect = unsafe { msg!(e, c"accessibilityFrame"; NSRect) };
        d.frame = [f.origin.x, super::primary_height() - f.origin.y - f.size.h, f.size.w, f.size.h];
    }
    d
}

// every element below window pw's NSWindow (depth 0: the window's children as AppKit serves
// them -- the view is ignored, so they are our nodes), depth first
fn walk(pw: i32, f: &mut dyn FnMut(Id, u32) -> bool) {
    let Some((win, _, _)) = ids(pw) else { return };
    let mut stack: Vec<(Id, u32)> = elements(attr(win, "AXChildren")).into_iter().rev().map(|e| (e, 0)).collect();
    let mut seen = 0;
    while let Some((e, depth)) = stack.pop() {
        seen += 1;
        if seen > 5000 || !f(e, depth) {
            return;
        }
        for c in elements(get(e, c"accessibilityChildren")).into_iter().rev() {
            stack.push((c, depth + 1));
        }
    }
}

/// (tests, any thread) the elements of platform window pw as VoiceOver reads them
pub fn ax_test_tree(pw: i32) -> Option<Vec<AxTestElem>> {
    on_main(|| {
        ids(pw)?;
        let mut v = Vec::new();
        walk(pw, &mut |e, depth| {
            v.push(describe(e, depth));
            true
        });
        Some(v)
    })
}

/// (tests, any thread) act on the element with this role and name (AxTestElem::name) as VoiceOver
/// does: false when there is none or AppKit does not offer the action (disabled, not settable)
pub fn ax_test_act(pw: i32, role: &str, name: &str, a: AxTestAct) -> bool {
    on_main(|| {
        let mut found = NIL;
        walk(pw, &mut |e, depth| {
            let d = describe(e, depth);
            if d.role == role && d.name() == name {
                found = e;
                return false;
            }
            true
        });
        if found.is_null() {
            return false;
        }
        let d = describe(found, 0);
        // SAFETY (below): the protocol's methods of an element that offers them (offers).
        let perform = |m: &CStr| {
            if !d.enabled || !offers(found, m) {
                return false;
            }
            unsafe { msg!(found, m; bool) }
        };
        let set_bool = |m: &CStr| {
            if !offers(found, m) {
                return false;
            }
            unsafe { msg!(found, m, true => bool; ()) };
            true
        };
        let set_value = |v: Id| {
            if !offers(found, c"setAccessibilityValue:") {
                return false;
            }
            unsafe { msg!(found, c"setAccessibilityValue:", v => Id; ()) };
            true
        };
        match a {
            AxTestAct::Press => perform(c"accessibilityPerformPress"),
            AxTestAct::Increment => perform(c"accessibilityPerformIncrement"),
            AxTestAct::Decrement => perform(c"accessibilityPerformDecrement"),
            AxTestAct::Number(x) => set_value(number(x)),
            AxTestAct::Text(t) => set_value(text(&t)),
            AxTestAct::Focus => set_bool(c"setAccessibilityFocused:"),
            AxTestAct::Select => set_bool(c"setAccessibilitySelected:"),
            AxTestAct::Range(loc, len) => {
                if !offers(found, c"setAccessibilitySelectedTextRange:") {
                    return false;
                }
                unsafe { msg!(found, c"setAccessibilitySelectedTextRange:", NSRange { location: loc, length: len } => NSRange; ()) };
                true
            }
        }
    })
}

/// (tests, any thread) ask the element with this role and name about its text as VoiceOver does
/// (None: no such element, or it does not offer the attribute)
pub fn ax_test_text(pw: i32, role: &str, name: &str, q: AxTestText) -> Option<String> {
    on_main(|| {
        let mut found = NIL;
        walk(pw, &mut |e, depth| {
            let d = describe(e, depth);
            if d.role == role && d.name() == name {
                found = e;
                return false;
            }
            true
        });
        if found.is_null() {
            return None;
        }
        let e = found;
        let r = |loc: usize, len: usize| NSRange { location: loc, length: len };
        let fmt_range = |x: NSRange| format!("{},{}", x.location as isize, x.length);
        let fmt_rect = |f: NSRect| {
            let top = super::primary_height() - f.origin.y - f.size.h;
            format!("{},{},{},{}", f.origin.x, top, f.size.w, f.size.h)
        };
        let has = |m: &CStr| offers(e, m);
        // SAFETY (below): the protocol's text methods of an element that offers them.
        Some(match q {
            AxTestText::Count if has(c"accessibilityNumberOfCharacters") => {
                format!("{}", unsafe { msg!(e, c"accessibilityNumberOfCharacters"; isize) })
            }
            AxTestText::Selected if has(c"accessibilitySelectedText") => describe_obj(unsafe { msg!(e, c"accessibilitySelectedText"; Id) }),
            AxTestText::SelRange if has(c"accessibilitySelectedTextRange") => {
                fmt_range(unsafe { msg!(e, c"accessibilitySelectedTextRange"; NSRange) })
            }
            AxTestText::Line if has(c"accessibilityInsertionPointLineNumber") => {
                format!("{}", unsafe { msg!(e, c"accessibilityInsertionPointLineNumber"; isize) })
            }
            AxTestText::StringFor(a, n) if has(c"accessibilityStringForRange:") => {
                describe_obj(unsafe { msg!(e, c"accessibilityStringForRange:", r(a, n) => NSRange; Id) })
            }
            AxTestText::AttrFor(a, n) if has(c"accessibilityAttributedStringForRange:") => {
                describe_obj(unsafe { msg!(msg!(e, c"accessibilityAttributedStringForRange:", r(a, n) => NSRange; Id), c"string"; Id) })
            }
            AxTestText::FrameFor(a, n) if has(c"accessibilityFrameForRange:") => {
                fmt_rect(unsafe { msg!(e, c"accessibilityFrameForRange:", r(a, n) => NSRange; NSRect) })
            }
            AxTestText::RangeAt(x, y) if has(c"accessibilityRangeForPosition:") => {
                let p = NSPoint { x, y: super::primary_height() - y };
                fmt_range(unsafe { msg!(e, c"accessibilityRangeForPosition:", p => NSPoint; NSRange) })
            }
            AxTestText::RangeFor(i) if has(c"accessibilityRangeForIndex:") => {
                fmt_range(unsafe { msg!(e, c"accessibilityRangeForIndex:", i as isize => isize; NSRange) })
            }
            AxTestText::LineFor(i) if has(c"accessibilityLineForIndex:") => {
                format!("{}", unsafe { msg!(e, c"accessibilityLineForIndex:", i as isize => isize; isize) })
            }
            AxTestText::RangeForLine(l) if has(c"accessibilityRangeForLine:") => {
                fmt_range(unsafe { msg!(e, c"accessibilityRangeForLine:", l as isize => isize; NSRange) })
            }
            _ => return None,
        })
    })
}

/// (tests, any thread) window pw on the main thread, where its state lives (UiOnly): its client
/// area in px of the screen, its dpi and whether it is the key window
pub fn ax_test_window(pw: i32) -> ([i32; 4], i32, bool) {
    on_main(|| (crate::win_get_rect(pw).unwrap_or([0; 4]), crate::win_dpi(pw), crate::win_test(crate::WT_ACTIVE, pw, [0; 4]) == 1))
}

/// (tests, any thread) the focused element of the application as VoiceOver asks it (role, name)
pub fn ax_test_focused() -> Option<(String, String)> {
    on_main(|| {
        // SAFETY: NSApp's focused element through AppKit's attribute entry point.
        let app: Id = unsafe { msg!(class(c"NSApplication"), c"sharedApplication"; Id) };
        let f = attr(app, "AXFocusedUIElement");
        if f.is_null() {
            return None;
        }
        let d = describe(f, 0);
        Some((d.role.clone(), String::from(d.name())))
    })
}

/// (tests, any thread) the element at screen point x, y (points from the top left of the primary
/// screen, as AXPosition) as VoiceOver's pointer finds it: role, name
pub fn ax_test_hit(x: f64, y: f64) -> Option<(String, String)> {
    on_main(|| {
        // SAFETY: NSApp's hit test (Cocoa screen points: y from the bottom of the primary screen).
        let app: Id = unsafe { msg!(class(c"NSApplication"), c"sharedApplication"; Id) };
        let p = NSPoint { x, y: super::primary_height() - y };
        let e: Id = unsafe { msg!(app, c"accessibilityHitTest:", p => NSPoint; Id) };
        if e.is_null() {
            return None;
        }
        let d = describe(e, 0);
        Some((d.role.clone(), String::from(d.name())))
    })
}

/// (tests) note the notifications the bridge posts from now on (false: stop)
pub fn ax_test_listen(on: bool) {
    on_main(|| reg(|r| r.log = if on { Some(Vec::new()) } else { None }));
}

/// (tests) the notifications noted since the last call: "AXValueChanged:Name"
pub fn ax_test_events() -> Vec<String> {
    on_main(|| reg(|r| r.log.as_mut().map(core::mem::take).unwrap_or_default()))
}
