// apple_ios_a11y -- (a11y.rs) the accessibility tree as UIAccessibility elements (VoiceOver on the
// iPhone, docs/VOICEOVER.md VO4)
//
//   start(app)                 VoiceOver (or Switch Control) running turns a11y::active() on, its
//                              notification follows it; a question to the view turns it on too
//   view_methods()             (apple_ios.rs) JFView's accessibility: not an element itself, its
//                              elements are the nodes of the shown windows; the Escape gesture
//   changed(snapshot, changes) the UI published a new tree: layout / screen changed notifications
//   ios_ax_tree / ios_ax_act   (tests, any thread) read and drive the elements as VoiceOver does,
//                              on the main thread
//
// Every node is an object of our class JFAXIosElement (a subclass of UIAccessibilityElement), made
// on demand and kept while its node exists; text boxes are JFAXIosText, which adds UITextInput
// (apple_ios_text.rs).  The canvas holds all windows (web_windows.rs): a node's frame is its
// window's place in the canvas plus its rectangle, in points of JFView (px / the host's scale),
// converted to the screen.  Lists, tab strips, dialogs, menus and titled boxes are containers
// (not elements themselves; their nodes are their elements); every other node is an element, and
// what hangs below it (the popup of a drop-down) follows it in its container.  A modal dialog
// window shows alone (accessibilityViewIsModal).  Actions become a11y::request and run a frame at
// once, so that VoiceOver reads the new value right after it asked for it.
//
// Traits: Label static text; Heading header; Button, MenuItem, a menu bar entry, Combo button
// (pressed: selected); CheckBox, Radio button, selected when on; Switch toggle button (iOS 17, else
// button) with the value 1 / 0; Slider adjustable (5 % of the range per swipe, as Android);
// Progress updates frequently, the value in percent; Link link; ListItem, Tab button, selected;
// Image image; disabled: not enabled.  Containers: a list a list, a tab strip a tab bar, the rest
// semantic groups with their names.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_void};
use core::sync::atomic::{AtomicBool, Ordering};

use super::super::objc::{
    Id, NIL, NSRect, Sel, class, ivar_get, ivar_set, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release,
    sel, string_of, subclass,
};
use super::{host, key, tick_now};
use crate::a11y::{self as ax, Act, Change, NONE, Node, Request, Role, Snapshot, WinTree};

#[path = "apple_ios_text.rs"]
pub(crate) mod text;

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIAccessibilityPostNotification(notification: u32, argument: Id);
    fn UIAccessibilityIsVoiceOverRunning() -> bool;
    fn UIAccessibilityIsSwitchControlRunning() -> bool;
    pub(crate) fn UIAccessibilityConvertFrameToScreenCoordinates(rect: NSRect, view: Id) -> NSRect;
    static UIAccessibilityScreenChangedNotification: u32;
    static UIAccessibilityLayoutChangedNotification: u32;
    static UIAccessibilityVoiceOverStatusDidChangeNotification: Id;
    static UIAccessibilitySwitchControlStatusDidChangeNotification: Id;
    static UIAccessibilityTraitButton: u64;
    static UIAccessibilityTraitLink: u64;
    static UIAccessibilityTraitHeader: u64;
    static UIAccessibilityTraitImage: u64;
    static UIAccessibilityTraitSelected: u64;
    static UIAccessibilityTraitStaticText: u64;
    static UIAccessibilityTraitNotEnabled: u64;
    static UIAccessibilityTraitUpdatesFrequently: u64;
    static UIAccessibilityTraitAdjustable: u64;
    static UIAccessibilityTraitTabBar: u64;
    static UIAccessibilityTraitKeyboardKey: u64;
}

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_sync_f(queue: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn pthread_main_np() -> i32;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// synthetic nodes of the UI (ui/a11y.rs): bit 63 of the key
const SYN: u64 = 1 << 63;
/// UIAccessibilityContainerType
const CONTAINER_LIST: isize = 2;
const CONTAINER_GROUP: isize = 4;
/// the key of a window's own element (a dialog window: a modal container)
const WINDOW: u64 = 0;

// ---------------------------------------------------------------- the registry

struct Slot {
    pw: i32,
    key: u64,
    obj: Id,
}

struct Reg {
    cls: Id,
    text_cls: Id,
    off: isize,
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    map: BTreeMap<(i32, u64), usize>,
    observer: Id,
    /// (tests) the notifications posted: "LayoutChanged:Name"
    log: Option<Vec<String>>,
}

struct Main(UnsafeCell<Option<Reg>>);
// SAFETY: only the main thread touches the registry (UIKit's thread, where the host's runtime runs).
unsafe impl Sync for Main {}
static REG: Main = Main(UnsafeCell::new(None));

static ASKED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
/// an action runs its frame now (no second one inside)
static ACTING: AtomicBool = AtomicBool::new(false);

fn reg<R>(f: impl FnOnce(&mut Reg) -> R) -> R {
    // SAFETY: main thread only, short borrows that never call into UIKit; the classes are made once.
    unsafe {
        let r = &mut *REG.0.get();
        if r.is_none() {
            let (cls, off) = subclass(c"UIAccessibilityElement", c"JFAXIosElement", &elem_methods(), &[]);
            let (text_cls, _) = subclass(c"JFAXIosElement", c"JFAXIosText", &text::methods(), &[c"UITextInput"]);
            *r = Some(Reg {
                cls,
                text_cls,
                off,
                slots: Vec::new(),
                free: Vec::new(),
                map: BTreeMap::new(),
                observer: NIL,
                log: None,
            });
        }
        f(r.as_mut().unwrap_unchecked())
    }
}

/// an autoreleased NSString
pub(crate) fn text_obj(s: &str) -> Id {
    let o = nsstring(s);
    // SAFETY: autorelease of the string we own; the caller runs inside a pool.
    unsafe { msg!(o, c"autorelease"; Id) }
}

fn array(v: &[Id]) -> Id {
    // SAFETY: arrayWithObjects:count: retains the objects (autoreleased array).
    unsafe { msg!(class(c"NSArray"), c"arrayWithObjects:count:", v.as_ptr() => *const Id, v.len() => usize; Id) }
}

/// the element of node key of window pw (WINDOW: the window's own), in container `inside`
fn elem(pw: i32, key: u64, text_box: bool, inside: Id) -> Id {
    let found = reg(|r| r.map.get(&(pw, key)).and_then(|&k| r.slots[k].as_ref()).map(|s| s.obj));
    let obj = match found {
        Some(o) => o,
        None => {
            let cls = reg(|r| if text_box { r.text_cls } else { r.cls });
            // SAFETY: a new element of our class (+1, the registry keeps it).
            let obj: Id = unsafe {
                let o: Id = msg!(cls, c"alloc"; Id);
                msg!(o, c"initWithAccessibilityContainer:", inside => Id; Id)
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
    };
    if !inside.is_null() {
        // SAFETY: the (weak) container of our element; a node can move to another container.
        unsafe { msg!(obj, c"setAccessibilityContainer:", inside => Id; ()) };
    }
    obj
}

fn slot_of(this: Id) -> Option<(i32, u64)> {
    reg(|r| {
        let k = ivar_get(this, r.off);
        if k < 0 {
            return None;
        }
        r.slots.get(k as usize).and_then(|s| s.as_ref()).map(|s| (s.pw, s.key))
    })
}

/// f(window tree, node (None: the window's own element), pw) for an element
fn with_node<R>(this: Id, f: impl FnOnce(&WinTree, Option<&Node>, i32) -> R) -> Option<R> {
    let (pw, key) = slot_of(this)?;
    let s: Arc<Snapshot> = ax::current()?;
    let t = s.pw(pw)?;
    if key == WINDOW {
        return Some(f(t, None, pw));
    }
    let n = &t.nodes[t.find(key)?];
    Some(f(t, Some(n), pw))
}

// ---------------------------------------------------------------- roles, containers, traits

fn on(n: &Node, bit: u32) -> bool {
    n.state & bit != 0
}

/// a node whose children are its elements (it is no element itself)
fn container(n: &Node) -> bool {
    matches!(n.role, Role::List | Role::TabList | Role::Dialog | Role::MenuBar)
        || (n.role == Role::Menu && n.key & SYN != 0)
        || (n.role == Role::Group && n.first != NONE)
}

fn container_type(n: &Node) -> isize {
    if n.role == Role::List { CONTAINER_LIST } else { CONTAINER_GROUP }
}

/// the elements below parent (NONE: the window's top level), in order: containers stand for
/// their subtrees, what hangs below an element follows it
fn items(t: &WinTree, parent: u32, out: &mut Vec<u64>) {
    for c in t.children(parent) {
        let n = &t.nodes[c];
        out.push(n.key);
        if !container(n) && n.first != NONE {
            items(t, c as u32, out);
        }
    }
}

/// UIAccessibilityTraitToggleButton (iOS 17; earlier: none)
fn toggle_trait() -> u64 {
    // SAFETY: dlsym of an exported constant of UIKit (RTLD_DEFAULT = -2); read once it is there.
    unsafe {
        let p = dlsym(-2isize as *mut c_void, c"UIAccessibilityTraitToggleButton".as_ptr()) as *const u64;
        if p.is_null() { 0 } else { *p }
    }
}

fn traits(n: &Node) -> u64 {
    // SAFETY: UIKit's exported trait constants.
    let (button, selected) = unsafe { (UIAccessibilityTraitButton, UIAccessibilityTraitSelected) };
    // SAFETY (the match): UIKit's exported trait constants.
    let mut t = match n.role {
        Role::Label => unsafe { UIAccessibilityTraitStaticText },
        Role::Heading => unsafe { UIAccessibilityTraitHeader | UIAccessibilityTraitStaticText },
        Role::Link => unsafe { UIAccessibilityTraitLink },
        Role::Image => unsafe { UIAccessibilityTraitImage },
        Role::Slider => unsafe { UIAccessibilityTraitAdjustable },
        Role::Progress => unsafe { UIAccessibilityTraitUpdatesFrequently },
        // (with UITextInput on the element: a text field to VoiceOver, as Flutter's text fields)
        Role::TextBox => unsafe { UIAccessibilityTraitKeyboardKey },
        Role::Switch => {
            let tb = toggle_trait();
            if tb != 0 { tb } else { button }
        }
        Role::Button | Role::CheckBox | Role::Radio | Role::Tab | Role::ListItem | Role::Menu | Role::MenuItem | Role::Combo => button,
        _ => 0,
    };
    let sel = match n.role {
        Role::Button => on(n, ax::S_PRESSED),
        Role::CheckBox | Role::Radio => on(n, ax::S_CHECKED),
        Role::ListItem | Role::Tab => on(n, ax::S_SELECTED),
        _ => false,
    };
    if sel {
        t |= selected;
    }
    if on(n, ax::S_DISABLED) {
        // SAFETY: UIKit's exported trait constant.
        t |= unsafe { UIAccessibilityTraitNotEnabled };
    }
    t
}

fn number_text(v: f64) -> String {
    if v == (v as i64) as f64 { format!("{}", v as i64) } else { format!("{v:.2}") }
}

/// the value VoiceOver reads after the name
fn value_of(n: &Node) -> Option<String> {
    match n.role {
        Role::Switch => Some(String::from(if on(n, ax::S_CHECKED) { "1" } else { "0" })),
        Role::Slider => Some(number_text(n.value)),
        Role::Progress if n.value >= n.min && n.max > n.min => {
            let p = (n.value - n.min) * 100.0 / (n.max - n.min);
            Some(format!("{} %", (p + 0.5) as i64))
        }
        Role::TextBox | Role::Combo => Some(n.text.clone()),
        _ => None,
    }
}

/// the step of increment / decrement (as Android's scroll actions)
fn step(n: &Node) -> f64 {
    let s = (n.max - n.min) / 20.0;
    if n.step > s { n.step } else { s }
}

// ---------------------------------------------------------------- geometry

/// client pixels of window pw -> screen coordinates (points)
fn to_screen(pw: i32, r: [i32; 4]) -> NSRect {
    let h = host();
    let view = h.view.get();
    if view.is_null() {
        return NSRect::default();
    }
    let place = crate::win_get_rect(pw).unwrap_or([0; 4]);
    let s = h.scale.get();
    let local = NSRect::new((place[0] + r[0]) as f64 / s, (place[1] + r[1]) as f64 / s, r[2] as f64 / s, r[3] as f64 / s);
    // SAFETY: UIKit's conversion of a rectangle in our view to the screen.
    unsafe { UIAccessibilityConvertFrameToScreenCoordinates(local, view) }
}

// ---------------------------------------------------------------- the view (JFView)

/// an assistive technology asked: the UI builds the tree from now on
fn touch() {
    ASKED.store(true, Ordering::Release);
    if ENABLED.load(Ordering::Acquire) {
        ax::set_active(true);
    }
}

/// the elements of the view: a modal dialog window's alone, else every window's (the main
/// window's nodes directly, the others in a container of their own)
fn view_items(view: Id) -> Vec<Id> {
    let Some(s) = ax::current() else { return Vec::new() };
    let modal = s.wins.iter().rev().find(|w| w.dialog && w.pw >= 0);
    let mut out = Vec::new();
    for w in &s.wins {
        if w.pw < 0 || modal.is_some_and(|m| m.pw != w.pw) {
            continue;
        }
        if w.win == 0 && modal.is_none() {
            let mut keys = Vec::new();
            items(w, NONE, &mut keys);
            out.extend(keys.into_iter().map(|k| node_elem(w, k, view)));
        } else {
            out.push(elem(w.pw, WINDOW, false, view));
        }
    }
    out.retain(|o| !o.is_null());
    out
}

/// the element of node key of tree t in container `inside`
fn node_elem(t: &WinTree, key: u64, inside: Id) -> Id {
    let text_box = t.find(key).is_some_and(|k| t.nodes[k].role == Role::TextBox);
    elem(t.pw, key, text_box, inside)
}

extern "C" fn v_elements(this: Id, _cmd: Sel) -> Id {
    touch();
    array(&view_items(this))
}

extern "C" fn v_no(_this: Id, _cmd: Sel) -> bool {
    false
}

/// VoiceOver's Escape gesture (two fingers in a Z): Escape closes a popup or dialog
extern "C" fn v_escape(_this: Id, _cmd: Sel) -> bool {
    key(27, 0);
    run_frame();
    true
}

pub(crate) fn view_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    alloc::vec![
        (c"isAccessibilityElement", v_no as *const c_void, c"B@:"),
        (c"accessibilityElements", v_elements as *const c_void, c"@@:"),
        (c"accessibilityPerformEscape", v_escape as *const c_void, c"B@:"),
    ]
}

// ---------------------------------------------------------------- the elements

// SAFETY (all methods below): UIKit calls them on the main thread with one of our elements.

extern "C" fn e_is_element(this: Id, _cmd: Sel) -> bool {
    with_node(this, |_, n, _| n.is_some_and(|n| !container(n))).unwrap_or(false)
}

extern "C" fn e_elements(this: Id, _cmd: Sel) -> Id {
    let Some((pw, keys)) = with_node(this, |t, n, pw| {
        let mut keys = Vec::new();
        match n {
            Some(n) if container(n) => items(t, t.find(n.key).unwrap_or(0) as u32, &mut keys),
            None => items(t, NONE, &mut keys),
            _ => {}
        }
        (pw, keys)
    }) else {
        return NIL;
    };
    let Some(s) = ax::current() else { return NIL };
    let Some(t) = s.pw(pw) else { return NIL };
    let objs: Vec<Id> = keys.into_iter().map(|k| node_elem(t, k, this)).filter(|o| !o.is_null()).collect();
    array(&objs)
}

extern "C" fn e_container_type(this: Id, _cmd: Sel) -> isize {
    with_node(this, |_, n, _| n.map_or(CONTAINER_GROUP, container_type)).unwrap_or(0)
}

extern "C" fn e_label(this: Id, _cmd: Sel) -> Id {
    with_node(this, |t, n, _| match n {
        Some(n) if !n.name.is_empty() => text_obj(&n.name),
        Some(_) => NIL,
        None => text_obj(&t.title),
    })
    .unwrap_or(NIL)
}

extern "C" fn e_value(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, _| n.and_then(value_of).map_or(NIL, |v| text_obj(&v))).unwrap_or(NIL)
}

extern "C" fn e_traits(this: Id, _cmd: Sel) -> u64 {
    with_node(this, |_, n, _| match n {
        // SAFETY: UIKit's exported trait constant.
        Some(n) if n.role == Role::TabList => unsafe { UIAccessibilityTraitTabBar },
        Some(n) => traits(n),
        None => 0,
    })
    .unwrap_or(0)
}

extern "C" fn e_identifier(this: Id, _cmd: Sel) -> Id {
    with_node(this, |_, n, pw| text_obj(&n.map_or(format!("window {pw}"), |n| format!("{:x}", n.key)))).unwrap_or(NIL)
}

extern "C" fn e_frame(this: Id, _cmd: Sel) -> NSRect {
    match with_node(this, |t, n, pw| (pw, n.map_or([0, 0, t.size[0], t.size[1]], |n| n.rect))) {
        Some((pw, r)) => to_screen(pw, r),
        None => NSRect::default(),
    }
}

extern "C" fn e_modal(this: Id, _cmd: Sel) -> bool {
    with_node(this, |t, n, _| n.map_or(t.dialog, |n| n.role == Role::Dialog)).unwrap_or(false)
}

/// a frame now (the host's tick), so that VoiceOver reads the result of its action at once
fn run_frame() {
    if !ACTING.swap(true, Ordering::AcqRel) {
        tick_now();
        ACTING.store(false, Ordering::Release);
    }
}

/// a request for the element's node when it allows the action, then its frame
fn act(this: Id, need: u32, a: Act) -> bool {
    let Some(Some((win, key, ok))) = with_node(this, |t, n, _| n.map(|n| (t.win, n.key, n.actions & need != 0))) else { return false };
    if ok {
        ax::request(Request { win, key, act: a });
        run_frame();
    }
    ok
}

extern "C" fn e_activate(this: Id, _cmd: Sel) -> bool {
    match with_node(this, |_, n, _| n.map(|n| (n.role, n.actions))).flatten() {
        // a text box: the focus, the keyboard follows (as after a tap that reached it)
        Some((Role::TextBox, a)) if a & ax::A_FOCUS != 0 => {
            host().gesture.set(true);
            act(this, ax::A_FOCUS, Act::Focus)
        }
        Some(_) => act(this, ax::A_CLICK, Act::Click),
        None => false,
    }
}

fn nudge(this: Id, up: bool) {
    let Some(Some(v)) = with_node(this, |_, n, _| {
        n.map(|n| {
            let v = if up { n.value + step(n) } else { n.value - step(n) };
            if v < n.min { n.min } else if v > n.max { n.max } else { v }
        })
    }) else {
        return;
    };
    act(this, ax::A_VALUE, Act::Value(v));
}

extern "C" fn e_increment(this: Id, _cmd: Sel) {
    nudge(this, true);
}

extern "C" fn e_decrement(this: Id, _cmd: Sel) {
    nudge(this, false);
}

fn elem_methods() -> Vec<(&'static CStr, *const c_void, &'static CStr)> {
    alloc::vec![
        (c"isAccessibilityElement", e_is_element as *const c_void, c"B@:"),
        (c"accessibilityElements", e_elements as *const c_void, c"@@:"),
        (c"accessibilityContainerType", e_container_type as *const c_void, c"q@:"),
        (c"accessibilityLabel", e_label as *const c_void, c"@@:"),
        (c"accessibilityValue", e_value as *const c_void, c"@@:"),
        (c"accessibilityTraits", e_traits as *const c_void, c"Q@:"),
        (c"accessibilityIdentifier", e_identifier as *const c_void, c"@@:"),
        (c"accessibilityFrame", e_frame as *const c_void, c"{CGRect={CGPoint=dd}{CGSize=dd}}@:"),
        (c"accessibilityViewIsModal", e_modal as *const c_void, c"B@:"),
        (c"accessibilityActivate", e_activate as *const c_void, c"B@:"),
        (c"accessibilityIncrement", e_increment as *const c_void, c"v@:"),
        (c"accessibilityDecrement", e_decrement as *const c_void, c"v@:"),
    ]
}

// ---------------------------------------------------------------- the bridge

extern "C" fn o_status(_this: Id, _cmd: Sel, _note: Id) {
    status();
}

/// VoiceOver / Switch Control on: the tree; off: none (until something asks again)
fn status() {
    // SAFETY: UIKit's status queries.
    let on = unsafe { UIAccessibilityIsVoiceOverRunning() || UIAccessibilityIsSwitchControlRunning() };
    ax::set_active(on);
}

pub fn start(_app: &str) {
    ENABLED.store(true, Ordering::Release);
    status();
    if ASKED.load(Ordering::Acquire) {
        ax::set_active(true);
    }
    if !reg(|r| r.observer).is_null() {
        return;
    }
    let (cls, _) = subclass(c"NSObject", c"JFAXIosObserver", &[(c"jfStatus:", o_status as *const c_void, c"v@:@")], &[]);
    // SAFETY: an observer object kept for the process; the centre does not retain it.
    unsafe {
        let obs: Id = msg!(cls, c"new"; Id);
        let centre: Id = msg!(class(c"NSNotificationCenter"), c"defaultCenter"; Id);
        for name in [UIAccessibilityVoiceOverStatusDidChangeNotification, UIAccessibilitySwitchControlStatusDidChangeNotification] {
            msg!(centre, c"addObserver:selector:name:object:", obs => Id, sel(c"jfStatus:") => Sel, name => Id, NIL => Id; ());
        }
        reg(|r| r.observer = obs);
    }
}

fn post(what: u32, name: &str, arg: Id, label: &'static str) {
    reg(|r| {
        if let Some(log) = r.log.as_mut() {
            log.push(format!("{label}:{name}"));
        }
    });
    // SAFETY: UIKit's notification (the argument: an element of ours or nil).
    unsafe { UIAccessibilityPostNotification(what, arg) };
}

pub fn changed(s: &Snapshot, c: Vec<Change>) {
    // SAFETY: a pool for the autoreleased objects (we run in the host's tick).
    let pool = unsafe { objc_autoreleasePoolPush() };
    let view = host().view.get();
    let mut layout = false;
    let mut screen = false;
    let mut focus: Option<(i32, u64, String)> = None;
    for ch in &c {
        match ch {
            Change::Focus(w, key) if *key != 0 => {
                if let Some(t) = s.win(*w) {
                    let name = t.find(*key).map_or(String::new(), |k| t.nodes[k].name.clone());
                    focus = Some((t.pw, *key, name));
                }
            }
            // a dialog window or a dialog / popup menu in a window: a new screen
            Change::Window(w, true) => screen |= s.win(*w).is_some_and(|t| t.dialog),
            Change::Added(w, _, key) => {
                layout = true;
                screen |= s.win(*w).and_then(|t| t.find(*key).map(|k| &t.nodes[k])).is_some_and(|n| {
                    n.role == Role::Dialog || (n.role == Role::Menu && n.key & SYN != 0)
                });
            }
            Change::Removed(..) | Change::Window(_, false) => layout = true,
            _ => {}
        }
    }
    if !view.is_null() {
        let target = focus.as_ref().and_then(|(pw, key, _)| s.pw(*pw).map(|t| node_elem(t, *key, NIL)));
        let name = focus.as_ref().map_or("", |f| f.2.as_str());
        // SAFETY: UIKit's exported notification numbers.
        let (scr, lay) = unsafe { (UIAccessibilityScreenChangedNotification, UIAccessibilityLayoutChangedNotification) };
        if screen {
            post(scr, name, target.unwrap_or(NIL), "ScreenChanged");
        } else if focus.is_some() {
            post(lay, name, target.unwrap_or(NIL), "LayoutChanged");
        } else if layout {
            post(lay, "", NIL, "LayoutChanged");
        }
    }
    sweep(s);
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
}

/// the elements whose nodes (or windows) went away
fn sweep(s: &Snapshot) {
    let gone: Vec<Id> = reg(|r| {
        let ks: Vec<usize> = r
            .slots
            .iter()
            .enumerate()
            .filter_map(|(k, sl)| sl.as_ref().map(|sl| (k, sl)))
            .filter(|(_, sl)| s.pw(sl.pw).is_none_or(|t| sl.key != WINDOW && t.find(sl.key).is_none()))
            .map(|(k, _)| k)
            .collect();
        ks.into_iter()
            .filter_map(|k| {
                let sl = r.slots.get_mut(k)?.take()?;
                r.map.remove(&(sl.pw, sl.key));
                ivar_set(sl.obj, r.off, -1);
                r.free.push(k);
                Some(sl.obj)
            })
            .collect()
    });
    for obj in gone {
        release(obj);
    }
}

/// (apple_ios_text.rs) the text box node of an element: its window tree's pw and the node
pub(crate) fn text_node<R>(this: Id, f: impl FnOnce(&WinTree, &Node, i32) -> R) -> Option<R> {
    with_node(this, |t, n, pw| n.filter(|n| n.role == Role::TextBox).map(|n| f(t, n, pw))).flatten()
}

/// (apple_ios_text.rs) a request for a text box, then its frame
pub(crate) fn text_act(this: Id, need: u32, a: Act) -> bool {
    act(this, need, a)
}

// ---------------------------------------------------------------- tests: as VoiceOver asks

/// an element as VoiceOver reads it (UIAccessibility)
#[derive(Clone, Debug, Default)]
pub struct IosAxElem {
    /// containers above it
    pub depth: u32,
    /// isAccessibilityElement (false: a container)
    pub element: bool,
    pub label: String,
    pub value: String,
    pub ident: String,
    /// the traits by name: button, link, header, image, selected, static, disabled, frequent,
    /// adjustable, tabbar, keyboard, toggle
    pub traits: Vec<&'static str>,
    /// UIAccessibilityContainerType
    pub container: isize,
    pub modal: bool,
    /// screen points
    pub frame: [f64; 4],
    /// it conforms to UITextInput (a text field to VoiceOver)
    pub text_input: bool,
}

/// an action of a test
#[derive(Clone, Copy, Debug)]
pub enum IosAxAct {
    /// a double tap (accessibilityActivate)
    Activate,
    Increment,
    Decrement,
}

/// f on the main thread, inside a pool
pub(crate) fn on_main<R>(f: impl FnOnce() -> R) -> R {
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

fn trait_names(t: u64) -> Vec<&'static str> {
    // SAFETY: UIKit's exported trait constants.
    let known = unsafe {
        [
            (UIAccessibilityTraitButton, "button"),
            (UIAccessibilityTraitLink, "link"),
            (UIAccessibilityTraitHeader, "header"),
            (UIAccessibilityTraitImage, "image"),
            (UIAccessibilityTraitSelected, "selected"),
            (UIAccessibilityTraitStaticText, "static"),
            (UIAccessibilityTraitNotEnabled, "disabled"),
            (UIAccessibilityTraitUpdatesFrequently, "frequent"),
            (UIAccessibilityTraitAdjustable, "adjustable"),
            (UIAccessibilityTraitTabBar, "tabbar"),
            (UIAccessibilityTraitKeyboardKey, "keyboard"),
            (toggle_trait(), "toggle"),
        ]
    };
    known.iter().filter(|(bit, _)| *bit != 0 && t & bit == *bit).map(|(_, n)| *n).collect()
}

fn describe(e: Id, depth: u32) -> IosAxElem {
    // SAFETY: UIAccessibility's informal protocol on an accessibility object.
    unsafe {
        let s = |m: &CStr| -> String {
            let v: Id = msg!(e, m; Id);
            if v.is_null() { String::new() } else { string_of(v) }
        };
        let f: NSRect = msg!(e, c"accessibilityFrame"; NSRect);
        let traits: u64 = msg!(e, c"accessibilityTraits"; u64);
        IosAxElem {
            depth,
            element: msg!(e, c"isAccessibilityElement"; bool),
            label: s(c"accessibilityLabel"),
            value: s(c"accessibilityValue"),
            ident: s(c"accessibilityIdentifier"),
            traits: trait_names(traits),
            container: msg!(e, c"accessibilityContainerType"; isize),
            modal: msg!(e, c"accessibilityViewIsModal"; bool),
            frame: [f.origin.x, f.origin.y, f.size.w, f.size.h],
            text_input: msg!(e, c"conformsToProtocol:", super::super::objc::objc_getProtocol(c"UITextInput".as_ptr()) => Id; bool),
        }
    }
}

fn elements_of(e: Id) -> Vec<Id> {
    // SAFETY: accessibilityElements of a container (an NSArray or nil).
    unsafe {
        let a: Id = msg!(e, c"accessibilityElements"; Id);
        if a.is_null() {
            return Vec::new();
        }
        let n: usize = msg!(a, c"count"; usize);
        (0..n).map(|i| msg!(a, c"objectAtIndex:", i => usize; Id)).collect()
    }
}

// every accessibility object below JFView, depth first
fn walk(f: &mut dyn FnMut(Id, u32) -> bool) {
    let view = host().view.get();
    if view.is_null() {
        return;
    }
    let mut stack: Vec<(Id, u32)> = elements_of(view).into_iter().rev().map(|e| (e, 0)).collect();
    let mut seen = 0;
    while let Some((e, depth)) = stack.pop() {
        seen += 1;
        if seen > 5000 || !f(e, depth) {
            return;
        }
        // SAFETY: isAccessibilityElement of an accessibility object.
        if !unsafe { msg!(e, c"isAccessibilityElement"; bool) } {
            for c in elements_of(e).into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
}

/// (tests, any thread) the accessibility objects of the view as VoiceOver reads them (None: no
/// view yet)
pub fn ios_ax_tree() -> Option<Vec<IosAxElem>> {
    on_main(|| {
        if host().view.get().is_null() {
            return None;
        }
        let mut v = Vec::new();
        walk(&mut |e, depth| {
            v.push(describe(e, depth));
            true
        });
        Some(v)
    })
}

/// (tests, any thread) the element with this label (and trait `tr`, IosAxElem::traits; "": any): an
/// action as VoiceOver takes it -> the element's answer (accessibilityActivate) / whether it is
/// adjustable
pub fn ios_ax_act(label: &str, tr: &str, a: IosAxAct) -> bool {
    on_main(|| {
        let mut found = NIL;
        walk(&mut |e, depth| {
            let d = describe(e, depth);
            if d.element && d.label == label && (tr.is_empty() || d.traits.contains(&tr)) {
                found = e;
                return false;
            }
            true
        });
        if found.is_null() {
            return false;
        }
        // SAFETY: the UIAccessibilityAction methods of an element.
        unsafe {
            match a {
                IosAxAct::Activate => msg!(found, c"accessibilityActivate"; bool),
                IosAxAct::Increment | IosAxAct::Decrement => {
                    if !trait_names(msg!(found, c"accessibilityTraits"; u64)).contains(&"adjustable") {
                        return false;
                    }
                    if matches!(a, IosAxAct::Increment) {
                        msg!(found, c"accessibilityIncrement"; ());
                    } else {
                        msg!(found, c"accessibilityDecrement"; ());
                    }
                    true
                }
            }
        }
    })
}

/// (tests, any thread) VoiceOver's Escape gesture on the view
pub fn ios_ax_escape() -> bool {
    on_main(|| {
        let view = host().view.get();
        // SAFETY: accessibilityPerformEscape of our view.
        !view.is_null() && unsafe { msg!(view, c"accessibilityPerformEscape"; bool) }
    })
}

/// (tests) note the notifications the bridge posts from now on (false: stop)
pub fn ios_ax_listen(on: bool) {
    on_main(|| reg(|r| r.log = if on { Some(Vec::new()) } else { None }));
}

/// (tests) the notifications noted since the last call: "LayoutChanged:Name"
pub fn ios_ax_events() -> Vec<String> {
    on_main(|| reg(|r| r.log.as_mut().map(core::mem::take).unwrap_or_default()))
}

/// a question of a test about a text field's text (UITextInput of its element)
#[derive(Clone, Copy, Debug)]
pub enum IosAxText {
    /// the selected range: "location,length" (UTF-16)
    Selected,
    /// the text of a UTF-16 range
    Text(usize, usize),
    /// the caret's rectangle at an offset, screen points "x,y,w,h"
    Caret(usize),
}

// the text field with this label
fn find_elem(label: &str) -> Id {
    let mut found = NIL;
    walk(&mut |e, depth| {
        let d = describe(e, depth);
        if d.element && d.label == label && d.text_input {
            found = e;
            return false;
        }
        true
    });
    found
}

/// (tests, any thread) ask the text field with this label as VoiceOver does
pub fn ios_ax_text(label: &str, q: IosAxText) -> Option<String> {
    on_main(|| {
        let e = find_elem(label);
        if e.is_null() || text_node(e, |_, _, _| ()).is_none() {
            return None;
        }
        Some(match q {
            IosAxText::Selected => text::selected_of(e).map_or(String::new(), |(a, n)| format!("{a},{n}")),
            IosAxText::Text(a, n) => text::text_of(e, a, a + n),
            IosAxText::Caret(u) => {
                let r = text::caret_screen(e, u);
                format!("{},{},{},{}", r.origin.x, r.origin.y, r.size.w, r.size.h)
            }
        })
    })
}

/// (tests, any thread) select UTF-16 a .. a + n in the text field with this label (UITextInput)
pub fn ios_ax_select(label: &str, a: usize, n: usize) -> bool {
    on_main(|| {
        let e = find_elem(label);
        if e.is_null() || text_node(e, |_, _, _| ()).is_none() {
            return false;
        }
        text::select_on(e, a, a + n);
        true
    })
}

/// (tests, any thread) the keyboard's view types text (UIKeyInput / UITextInput of JFView)
pub fn ios_ax_type(t: &str) -> bool {
    on_main(|| {
        let view = host().view.get();
        if view.is_null() {
            return false;
        }
        let s = text_obj(t);
        // SAFETY: insertText: of our view.
        unsafe { msg!(view, c"insertText:", s => Id; ()) };
        true
    })
}
