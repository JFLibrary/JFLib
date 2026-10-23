// linux_atspi -- (a11y.rs) the accessibility tree on the AT-SPI2 bus, from a thread of its own
// with its own bus connections (linux_dbus.rs)
//
//   start(app)                  the thread: asks org.a11y.Bus on the session bus whether
//                               assistive technology is enabled (org.a11y.Status IsEnabled /
//                               ScreenReaderEnabled; jfgui_A11Y=1 skips the question, a later
//                               change of the properties is followed), GetAddress, connects to
//                               the accessibility bus and embeds the application in the
//                               registry (org.a11y.atspi.Socket.Embed); then a11y::set_active
//   changed(snapshot, changes)  the UI published a new tree: the changes become AT-SPI2 events
//
// Objects (all on our connection): /org/a11y/atspi/accessible/root the application (role
// application, Application interface), .../w<window> a window (frame / dialog),
// .../n<window>_<key hex> a node; /org/a11y/atspi/null the null reference.  Interfaces:
// Accessible (everything; headings: role heading, the attribute level:N; GetRoleName in
// English, GetLocalizedRoleName in the UI language: a11y::role_name), Component (windows,
// nodes), Action (A_CLICK: "click"), Value (sliders, progress bars; CurrentValue is writable),
// Text (labels, headings, text boxes; the basic queries, the caret and the selection, which
// SetCaretOffset / SetSelection move; character extents from Node::char_rect: exact at the
// caret, which screen magnifiers follow (Orca, GNOME Shell's zoom), estimated elsewhere),
// EditableText (text boxes).  They answer from a11y::current(); actions become a11y::request
// (the UI thread carries them out after its next wake-up).
// Events: Object:StateChanged (focused, checked, indeterminate, selected, expanded, enabled /
// sensitive, pressed, active), Focus:Focus, Object:PropertyChange (accessible-name,
// accessible-value), Object:AttributesChanged (a heading's level), Object:TextChanged (delete /
// insert), Object:TextCaretMoved (also when a text box gets the focus), Object:ChildrenChanged
// (add / remove with the child), Window:Create / Destroy / Activate / Deactivate.  The UI
// thread hands its changes over in a queue and wakes the thread with an eventfd.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rt::os::syscall6;
use rt::sync::SpinLock;

use crate::a11y::{self as ax, Act, Change, Node, Request, Role, Snapshot, WinTree, NONE};
use crate::linux_dbus::{Conn, Msg, T_CALL, T_RETURN, T_SIGNAL, Value, session_available};

const ROOT: &str = "/org/a11y/atspi/accessible/root";
const PREFIX: &str = "/org/a11y/atspi/accessible/";
const NULL_PATH: &str = "/org/a11y/atspi/null";
const REGISTRY: &str = "org.a11y.atspi.Registry";
const I_ACC: &str = "org.a11y.atspi.Accessible";
const I_APP: &str = "org.a11y.atspi.Application";
const I_COMP: &str = "org.a11y.atspi.Component";
const I_ACTION: &str = "org.a11y.atspi.Action";
const I_VALUE: &str = "org.a11y.atspi.Value";
const I_TEXT: &str = "org.a11y.atspi.Text";
const I_EDIT: &str = "org.a11y.atspi.EditableText";
const I_PROPS: &str = "org.freedesktop.DBus.Properties";
const EV_OBJECT: &str = "org.a11y.atspi.Event.Object";
const EV_WINDOW: &str = "org.a11y.atspi.Event.Window";
const EV_FOCUS: &str = "org.a11y.atspi.Event.Focus";
const CHANGES_MAX: usize = 4096;

// AtspiRole
const R_CANVAS: u32 = 6;
const R_CHECK_BOX: u32 = 7;
const R_COMBO_BOX: u32 = 11;
const R_DIALOG: u32 = 16;
const R_FRAME: u32 = 23;
const R_IMAGE: u32 = 27;
const R_LABEL: u32 = 29;
const R_LIST_ITEM: u32 = 32;
const R_MENU: u32 = 33;
const R_MENU_BAR: u32 = 34;
const R_MENU_ITEM: u32 = 35;
const R_PAGE_TAB: u32 = 37;
const R_PAGE_TAB_LIST: u32 = 38;
const R_PANEL: u32 = 39;
const R_PROGRESS_BAR: u32 = 42;
const R_PUSH_BUTTON: u32 = 43;
const R_RADIO_BUTTON: u32 = 44;
const R_SLIDER: u32 = 51;
const R_TOGGLE_BUTTON: u32 = 62;
const R_APPLICATION: u32 = 75;
const R_ENTRY: u32 = 79;
const R_HEADING: u32 = 83;
const R_LINK: u32 = 88;
const R_LIST_BOX: u32 = 98;

// AtspiStateType
const ST_ACTIVE: u32 = 1;
const ST_CHECKED: u32 = 4;
const ST_COLLAPSED: u32 = 5;
const ST_EDITABLE: u32 = 7;
const ST_ENABLED: u32 = 8;
const ST_EXPANDABLE: u32 = 9;
const ST_EXPANDED: u32 = 10;
const ST_FOCUSABLE: u32 = 11;
const ST_FOCUSED: u32 = 12;
const ST_HORIZONTAL: u32 = 14;
const ST_MODAL: u32 = 16;
const ST_PRESSED: u32 = 20;
const ST_RESIZABLE: u32 = 21;
const ST_SELECTABLE: u32 = 22;
const ST_SELECTED: u32 = 23;
const ST_SENSITIVE: u32 = 24;
const ST_SHOWING: u32 = 25;
const ST_SINGLE_LINE: u32 = 26;
const ST_VISIBLE: u32 = 30;
const ST_INDETERMINATE: u32 = 32;
const ST_SELECTABLE_TEXT: u32 = 38;
const ST_CHECKABLE: u32 = 41;
const ST_HAS_POPUP: u32 = 42;

static OPS: SpinLock<Vec<Change>> = SpinLock::new(Vec::new());
static WAKE: AtomicI32 = AtomicI32::new(-1);
// 0 not started, 1 running, 2 no accessibility bus
static STATE: AtomicU32 = AtomicU32::new(0);
static APP: SpinLock<String> = SpinLock::new(String::new());

pub fn start(app: &str) {
    if !session_available() || STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return;
    }
    *APP.lock() = String::from(app);
    // SAFETY: eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK).
    let fd = unsafe { syscall6(290, 0, 0x80000 | 0x800, 0, 0, 0, 0) };
    WAKE.store(fd as i32, Ordering::Release);
    if crate::thread_spawn(256 << 10, Box::new(run)).is_none() {
        STATE.store(2, Ordering::Release);
    }
}

pub fn changed(_s: &Snapshot, c: Vec<Change>) {
    if STATE.load(Ordering::Acquire) != 1 {
        return;
    }
    {
        let mut q = OPS.lock();
        if q.len() + c.len() > CHANGES_MAX {
            return; // a screen reader that does not keep up: it re-reads the tree anyway
        }
        q.extend(c);
    }
    let fd = WAKE.load(Ordering::Acquire);
    if fd >= 0 {
        let one = 1u64;
        // SAFETY: an 8-byte counter increment on our eventfd.
        unsafe { syscall6(1, fd as usize, &one as *const u64 as usize, 8, 0, 0, 0) };
    }
}

// ---------------------------------------------------------------- the thread

fn call(dest: &str, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Msg {
    Msg::call(dest, path, iface, member, body)
}

// the answer to serial; calls that arrive meanwhile are answered (b: the bridge, if any)
fn wait(c: &mut Conn, serial: u32, mut b: Option<&mut Bridge>) -> Option<Msg> {
    let end = crate::time_us() + 3_000_000;
    while crate::time_us() < end {
        let m = c.read((end.saturating_sub(crate::time_us()) / 1000) as i32 + 1)?;
        if (m.typ == T_RETURN || m.typ == crate::linux_dbus::T_ERROR) && m.reply_to == serial {
            return (m.typ == T_RETURN).then_some(m);
        }
        if let Some(b) = b.as_deref_mut() {
            b.handle(c, &m);
        }
    }
    None
}

// org.a11y.Status IsEnabled || ScreenReaderEnabled on the session bus
fn enabled(s: &mut Conn) -> bool {
    let get = |s: &mut Conn, prop: &str| {
        let m = call("org.a11y.Bus", "/org/a11y/bus", I_PROPS, "Get", alloc::vec![Value::str("org.a11y.Status"), Value::str(prop)]);
        s.send(&m).and_then(|serial| s.wait_reply(serial, 3000)).and_then(|r| r.body.first().and_then(|v| v.as_u32())) == Some(1)
    };
    get(s, "IsEnabled") || get(s, "ScreenReaderEnabled")
}

fn off() {
    STATE.store(2, Ordering::Release);
}

fn run() {
    let Some(mut s) = Conn::session() else { return off() };
    if !crate::env("jfgui_A11Y").is_some_and(|v| v.starts_with('1')) {
        let rule = "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path='/org/a11y/bus'";
        s.send(&call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::str(rule)]));
        // wait until assistive technology is turned on
        while !enabled(&mut s) {
            loop {
                let Some(m) = s.read(-1) else { return off() };
                if m.typ == T_SIGNAL && m.member == "PropertiesChanged" {
                    break;
                }
            }
        }
    }
    let ask = call("org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress", Vec::new());
    let Some(addr) = s.send(&ask).and_then(|serial| s.wait_reply(serial, 3000)).and_then(|r| r.body.first().and_then(|v| v.as_str()).map(String::from)) else {
        return off();
    };
    drop(s);
    let Some(mut c) = Conn::connect(&addr) else { return off() };
    let mut b = Bridge { me: c.unique.clone(), parent: (String::from(REGISTRY), String::from(ROOT)), app_id: 0, focus: None, app: APP.lock().clone() };
    let me = b.reference(ROOT);
    let Some(serial) = c.send(&call(REGISTRY, ROOT, "org.a11y.atspi.Socket", "Embed", alloc::vec![me])) else { return off() };
    if let Some(r) = wait(&mut c, serial, Some(&mut b)) {
        if let Some(Value::R(f)) = r.body.first() {
            if let (Some(Value::S(n)), Some(Value::O(p))) = (f.first(), f.get(1)) {
                b.parent = (n.clone(), p.clone());
            }
        }
    }
    ax::set_active(true);
    loop {
        let ops = core::mem::take(&mut *OPS.lock());
        if !ops.is_empty() {
            if let Some(snap) = ax::current() {
                for op in ops {
                    b.event(&mut c, &snap, op);
                }
            }
        }
        while let Some(m) = c.read(0) {
            b.handle(&mut c, &m);
        }
        if c.broken {
            ax::set_active(false);
            return off();
        }
        let wake = WAKE.load(Ordering::Acquire);
        let mut pfd = [c.fd() as u32 as u64 | (1u64 << 32), wake as u32 as u64 | (1u64 << 32)];
        // SAFETY: two pollfd records.
        unsafe { syscall6(7, pfd.as_mut_ptr() as usize, 2, usize::MAX, 0, 0, 0) };
        let mut n = 0u64;
        // SAFETY: drain the eventfd counter (nonblocking).
        unsafe { syscall6(0, wake as usize, &mut n as *mut u64 as usize, 8, 0, 0, 0) };
    }
}

// ---------------------------------------------------------------- objects

#[derive(Clone, Copy, PartialEq)]
enum Obj {
    App,
    Win(u8),
    Node(u8, u64),
    Null,
}

fn parse_path(p: &str) -> Option<Obj> {
    if p == ROOT {
        return Some(Obj::App);
    }
    if p == NULL_PATH {
        return Some(Obj::Null);
    }
    let rest = p.strip_prefix(PREFIX)?;
    if let Some(w) = rest.strip_prefix('w') {
        return w.parse::<u8>().ok().map(Obj::Win);
    }
    let n = rest.strip_prefix('n')?;
    let (w, k) = n.split_once('_')?;
    Some(Obj::Node(w.parse().ok()?, u64::from_str_radix(k, 16).ok()?))
}

fn path_of(o: Obj) -> String {
    match o {
        Obj::App => String::from(ROOT),
        Obj::Win(w) => alloc::format!("{PREFIX}w{w}"),
        Obj::Node(w, k) => alloc::format!("{PREFIX}n{w}_{k:x}"),
        Obj::Null => String::from(NULL_PATH),
    }
}

fn role_of(r: Role) -> u32 {
    match r {
        Role::Group => R_PANEL,
        Role::Label => R_LABEL,
        Role::Button => R_PUSH_BUTTON,
        Role::CheckBox => R_CHECK_BOX,
        Role::Radio => R_RADIO_BUTTON,
        Role::Switch => R_TOGGLE_BUTTON,
        Role::Slider => R_SLIDER,
        Role::Progress => R_PROGRESS_BAR,
        Role::TextBox => R_ENTRY,
        Role::Link => R_LINK,
        Role::List => R_LIST_BOX,
        Role::ListItem => R_LIST_ITEM,
        Role::MenuBar => R_MENU_BAR,
        Role::Menu => R_MENU,
        Role::MenuItem => R_MENU_ITEM,
        Role::TabList => R_PAGE_TAB_LIST,
        Role::Tab => R_PAGE_TAB,
        Role::Combo => R_COMBO_BOX,
        Role::Image => R_IMAGE,
        Role::Canvas => R_CANVAS,
        Role::Dialog => R_DIALOG,
        Role::Heading => R_HEADING,
    }
}

fn role_name(r: u32) -> &'static str {
    match r {
        R_CANVAS => "canvas",
        R_CHECK_BOX => "check box",
        R_COMBO_BOX => "combo box",
        R_DIALOG => "dialog",
        R_FRAME => "frame",
        R_IMAGE => "image",
        R_LABEL => "label",
        R_LIST_ITEM => "list item",
        R_MENU => "menu",
        R_MENU_BAR => "menu bar",
        R_MENU_ITEM => "menu item",
        R_PAGE_TAB => "page tab",
        R_PAGE_TAB_LIST => "page tab list",
        R_PANEL => "panel",
        R_PROGRESS_BAR => "progress bar",
        R_PUSH_BUTTON => "push button",
        R_RADIO_BUTTON => "radio button",
        R_SLIDER => "slider",
        R_TOGGLE_BUTTON => "toggle button",
        R_APPLICATION => "application",
        R_ENTRY => "entry",
        R_HEADING => "heading",
        R_LINK => "link",
        R_LIST_BOX => "list box",
        _ => "unknown",
    }
}

fn node_states(t: &WinTree, n: &Node) -> u64 {
    let mut s = 0u64;
    let mut on = |b: u32| s |= 1 << b;
    if n.state & ax::S_DISABLED == 0 {
        on(ST_ENABLED);
        on(ST_SENSITIVE);
    }
    if n.rect[2] > 0 && n.rect[3] > 0 {
        on(ST_VISIBLE);
    }
    if t.on_screen(n) {
        on(ST_SHOWING); // not when scrolled out of the window
    }
    if n.state & ax::S_FOCUSABLE != 0 {
        on(ST_FOCUSABLE);
    }
    if n.state & ax::S_FOCUSED != 0 {
        on(ST_FOCUSED);
    }
    if n.state & (ax::S_CHECKED | ax::S_PRESSED) != 0 {
        on(ST_CHECKED);
    }
    if n.state & ax::S_PRESSED != 0 {
        on(ST_PRESSED);
    }
    if n.state & ax::S_MIXED != 0 {
        on(ST_INDETERMINATE);
    }
    if n.state & ax::S_SELECTED != 0 {
        on(ST_SELECTED);
    }
    match n.role {
        Role::CheckBox | Role::Radio | Role::Switch => on(ST_CHECKABLE),
        Role::ListItem | Role::Tab => on(ST_SELECTABLE),
        Role::Combo | Role::Menu => {
            on(ST_EXPANDABLE);
            on(ST_HAS_POPUP);
            on(if n.state & ax::S_EXPANDED != 0 { ST_EXPANDED } else { ST_COLLAPSED });
        }
        Role::TextBox => {
            on(ST_EDITABLE);
            on(ST_SINGLE_LINE);
            on(ST_SELECTABLE_TEXT);
        }
        Role::Slider | Role::Progress => on(ST_HORIZONTAL),
        _ => {}
    }
    if n.state & ax::S_EXPANDED != 0 && !matches!(n.role, Role::Combo | Role::Menu) {
        on(ST_EXPANDED);
    }
    s
}

fn win_states(w: &WinTree) -> u64 {
    let mut s = 1u64 << ST_ENABLED | 1 << ST_SENSITIVE | 1 << ST_VISIBLE | 1 << ST_SHOWING | 1 << ST_RESIZABLE;
    if w.active {
        s |= 1 << ST_ACTIVE;
    }
    if w.dialog {
        s |= 1 << ST_MODAL;
    }
    s
}

fn state_name(bit: u32) -> &'static str {
    match bit {
        ax::S_CHECKED => "checked",
        ax::S_MIXED => "indeterminate",
        ax::S_SELECTED => "selected",
        ax::S_EXPANDED => "expanded",
        ax::S_PRESSED => "pressed",
        ax::S_DISABLED => "enabled",
        _ => "",
    }
}

// the text interfaces' text of a node (labels: their name)
fn text_of(n: &Node) -> Option<&str> {
    match n.role {
        Role::TextBox | Role::Combo => Some(&n.text),
        Role::Label | Role::Heading => Some(&n.name),
        _ => None,
    }
}

// characters start .. end (end < 0: to the end)
fn sub(s: &str, start: i32, end: i32) -> String {
    let n = s.chars().count() as i32;
    let a = start.clamp(0, n);
    let b = if end < 0 { n } else { end.clamp(a, n) };
    s.chars().skip(a as usize).take((b - a) as usize).collect()
}

// the word / line around character offset k: (text, start, end); granularity: 0 char,
// 1 word, 2 sentence and more: the whole text
fn around(s: &str, k: i32, gran: u32) -> (String, i32, i32) {
    let cs: Vec<char> = s.chars().collect();
    let n = cs.len() as i32;
    if k < 0 || k > n {
        return (String::new(), 0, 0);
    }
    match gran {
        0 => {
            if k >= n {
                return (String::new(), n, n);
            }
            (cs[k as usize].into(), k, k + 1)
        }
        1 => {
            let mut a = k.min(n);
            while a > 0 && !cs[a as usize - 1].is_whitespace() {
                a -= 1;
            }
            let mut b = k;
            while b < n && !cs[b as usize].is_whitespace() {
                b += 1;
            }
            while b < n && cs[b as usize].is_whitespace() {
                b += 1;
            }
            (cs[a as usize..b as usize].iter().collect(), a, b)
        }
        _ => (String::from(s), 0, n),
    }
}

// rectangle r of node n (client px) in coordinate type ctype: 0 screen, 1 window, 2 parent
fn coords(t: &WinTree, n: &Node, mut r: [i32; 4], ctype: u32) -> [i32; 4] {
    match ctype {
        0 => {
            r[0] += t.origin[0];
            r[1] += t.origin[1];
        }
        2 => {
            if let Some(p) = t.nodes.get(n.parent as usize) {
                r[0] -= p.rect[0];
                r[1] -= p.rect[1];
            }
        }
        _ => {}
    }
    r
}

// a reference (so): our bus name and an object path
struct Bridge {
    me: String,
    /// the registry's object we are embedded in
    parent: (String, String),
    app_id: i32,
    /// the node / window that last got the focus event
    focus: Option<Obj>,
    app: String,
}

fn arg_i(m: &Msg, k: usize) -> i32 {
    match m.body.get(k) {
        Some(Value::I(v)) => *v,
        Some(v) => v.as_u32().unwrap_or(0) as i32,
        None => 0,
    }
}

fn arg_s(m: &Msg, k: usize) -> &str {
    m.body.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn props_dict(v: Vec<(&str, Value)>) -> Value {
    Value::dict(v)
}

fn empty_ss() -> Value {
    Value::A(String::from("{ss}"), Vec::new())
}

impl Bridge {
    fn reference(&self, path: &str) -> Value {
        Value::R(alloc::vec![Value::S(self.me.clone()), Value::O(String::from(path))])
    }

    fn obj_ref(&self, o: Obj) -> Value {
        self.reference(&path_of(o))
    }

    fn node_obj(w: &WinTree, k: u32) -> Obj {
        if k == NONE { Obj::Win(w.win) } else { Obj::Node(w.win, w.nodes[k as usize].key) }
    }

    // ------------------------------------------------------------ events

    fn emit(&self, c: &mut Conn, o: Obj, iface: &str, member: &str, detail: &str, d1: i32, d2: i32, any: Value) {
        let body = alloc::vec![Value::str(detail), Value::I(d1), Value::I(d2), Value::var(any), props_dict(Vec::new())];
        c.send(&Msg::signal(&path_of(o), iface, member, body));
    }

    fn event(&mut self, c: &mut Conn, s: &Arc<Snapshot>, ch: Change) {
        let obj = |w: u8, key: u64| if key == 0 { Obj::Win(w) } else { Obj::Node(w, key) };
        match ch {
            Change::Window(w, added) => {
                let index = s.wins.iter().position(|t| t.win == w).unwrap_or(0) as i32;
                let title = s.win(w).map(|t| t.title.clone()).unwrap_or_default();
                let r = self.obj_ref(Obj::Win(w));
                self.emit(c, Obj::App, EV_OBJECT, "ChildrenChanged", if added { "add" } else { "remove" }, index, 0, r);
                self.emit(c, Obj::Win(w), EV_WINDOW, if added { "Create" } else { "Destroy" }, "", 0, 0, Value::S(title));
            }
            Change::Activate(w, on) => {
                let title = s.win(w).map(|t| t.title.clone()).unwrap_or_default();
                self.emit(c, Obj::Win(w), EV_OBJECT, "StateChanged", "active", on as i32, 0, Value::I(0));
                self.emit(c, Obj::Win(w), EV_WINDOW, if on { "Activate" } else { "Deactivate" }, "", 0, 0, Value::S(title));
            }
            Change::Focus(w, key) => {
                let o = obj(w, key);
                if let Some(old) = self.focus.take() {
                    if old != o {
                        self.emit(c, old, EV_OBJECT, "StateChanged", "focused", 0, 0, Value::I(0));
                    }
                }
                self.emit(c, o, EV_OBJECT, "StateChanged", "focused", 1, 0, Value::I(0));
                self.emit(c, o, EV_FOCUS, "Focus", "", 0, 0, Value::I(0));
                self.focus = Some(o);
            }
            Change::State(w, key, bit, on) => {
                let name = state_name(bit);
                if bit == ax::S_DISABLED {
                    self.emit(c, obj(w, key), EV_OBJECT, "StateChanged", "enabled", !on as i32, 0, Value::I(0));
                    self.emit(c, obj(w, key), EV_OBJECT, "StateChanged", "sensitive", !on as i32, 0, Value::I(0));
                } else if !name.is_empty() {
                    self.emit(c, obj(w, key), EV_OBJECT, "StateChanged", name, on as i32, 0, Value::I(0));
                    if bit == ax::S_PRESSED {
                        self.emit(c, obj(w, key), EV_OBJECT, "StateChanged", "checked", on as i32, 0, Value::I(0));
                    }
                }
            }
            Change::Name(w, key) => {
                let name = s.win(w).and_then(|t| t.find(key).map(|k| t.nodes[k].name.clone())).unwrap_or_default();
                self.emit(c, obj(w, key), EV_OBJECT, "PropertyChange", "accessible-name", 0, 0, Value::S(name));
            }
            Change::Value(w, key) => {
                let n = s.win(w).and_then(|t| t.find(key).map(|k| &t.nodes[k]));
                if n.is_some_and(|n| n.role == Role::Heading) {
                    // a heading's level is an attribute
                    self.emit(c, obj(w, key), EV_OBJECT, "AttributesChanged", "", 0, 0, Value::I(0));
                } else {
                    let v = n.map_or(0.0, |n| n.value);
                    self.emit(c, obj(w, key), EV_OBJECT, "PropertyChange", "accessible-value", 0, 0, Value::D(v));
                }
            }
            Change::Text(w, key, old) => {
                let new = s.win(w).and_then(|t| t.find(key).map(|k| t.nodes[k].text.clone())).unwrap_or_default();
                let a: Vec<char> = old.chars().collect();
                let b: Vec<char> = new.chars().collect();
                let mut p = 0;
                while p < a.len() && p < b.len() && a[p] == b[p] {
                    p += 1;
                }
                let mut q = 0;
                while q < a.len() - p && q < b.len() - p && a[a.len() - 1 - q] == b[b.len() - 1 - q] {
                    q += 1;
                }
                let del: String = a[p..a.len() - q].iter().collect();
                let ins: String = b[p..b.len() - q].iter().collect();
                if !del.is_empty() {
                    self.emit(c, obj(w, key), EV_OBJECT, "TextChanged", "delete", p as i32, (a.len() - q - p) as i32, Value::S(del));
                }
                if !ins.is_empty() {
                    self.emit(c, obj(w, key), EV_OBJECT, "TextChanged", "insert", p as i32, (b.len() - q - p) as i32, Value::S(ins));
                }
            }
            Change::Caret(w, key, caret) => {
                self.emit(c, obj(w, key), EV_OBJECT, "TextCaretMoved", "", caret, 0, Value::I(0));
            }
            Change::Added(w, parent, key) => {
                let Some(t) = s.win(w) else { return };
                let index = t.find(key).map_or(0, |k| t.nodes[k].index as i32);
                let r = self.obj_ref(obj(w, key));
                self.emit(c, obj(w, parent), EV_OBJECT, "ChildrenChanged", "add", index, 0, r);
            }
            Change::Removed(w, parent, key, index) => {
                let r = self.obj_ref(obj(w, key));
                self.emit(c, obj(w, parent), EV_OBJECT, "ChildrenChanged", "remove", index as i32, 0, r);
            }
        }
    }

    // ------------------------------------------------------------ calls

    fn handle(&mut self, c: &mut Conn, m: &Msg) {
        if m.typ != T_CALL {
            return;
        }
        let r = match ax::current() {
            Some(s) => self.call(&s, m),
            None => self.call(&Arc::new(Snapshot::default()), m),
        };
        if m.flags & crate::linux_dbus::F_NO_REPLY == 0 {
            c.send(&match r {
                Ok(body) => Msg::reply(m, body),
                Err(e) => Msg::error(m, e, "not available"),
            });
        }
    }

    fn call(&mut self, s: &Snapshot, m: &Msg) -> Result<Vec<Value>, &'static str> {
        const UNKNOWN: &str = "org.freedesktop.DBus.Error.UnknownMethod";
        const GONE: &str = "org.freedesktop.DBus.Error.UnknownObject";
        if m.iface == "org.freedesktop.DBus.Peer" && m.member == "Ping" {
            return Ok(Vec::new());
        }
        // no cache of our own: clients ask the objects themselves (libatspi asks at the start)
        if m.iface == "org.a11y.atspi.Cache" && m.member == "GetItems" {
            return Ok(alloc::vec![Value::A(String::from("((so)(so)(so)iiassusau)"), Vec::new())]);
        }
        let Some(o) = parse_path(&m.path) else { return Err(GONE) };
        // the object must exist in the tree
        let (w, k): (Option<&WinTree>, u32) = match o {
            Obj::App | Obj::Null => (None, NONE),
            Obj::Win(w) => (Some(s.win(w).ok_or(GONE)?), NONE),
            Obj::Node(w, key) => {
                let t = s.win(w).ok_or(GONE)?;
                (Some(t), t.find(key).ok_or(GONE)? as u32)
            }
        };
        let node = w.and_then(|t| t.nodes.get(k as usize));
        if m.iface == I_PROPS {
            let iface = arg_s(m, 0);
            return match m.member.as_str() {
                "Get" => self.property(s, o, w, node, iface, arg_s(m, 1)).map(|v| alloc::vec![Value::var(v)]).ok_or(UNKNOWN),
                "GetAll" => {
                    let names: &[&str] = match iface {
                        I_ACC => &["Name", "Description", "Parent", "ChildCount", "Locale", "AccessibleId"],
                        I_APP => &["ToolkitName", "Version", "AtspiVersion", "Id"],
                        I_ACTION => &["NActions"],
                        I_VALUE => &["MinimumValue", "MaximumValue", "MinimumIncrement", "CurrentValue", "Text"],
                        I_TEXT => &["CharacterCount", "CaretOffset"],
                        _ => &[],
                    };
                    let v = names.iter().filter_map(|n| self.property(s, o, w, node, iface, n).map(|v| (*n, v))).collect();
                    Ok(alloc::vec![Value::dict(v)])
                }
                "Set" => {
                    let v = match m.body.get(2) {
                        Some(Value::V(v)) => (**v).clone(),
                        Some(v) => v.clone(),
                        None => return Err(UNKNOWN),
                    };
                    match (iface, arg_s(m, 1), o, node) {
                        (I_APP, "Id", Obj::App, _) => {
                            self.app_id = v.as_u32().unwrap_or(0) as i32;
                            Ok(Vec::new())
                        }
                        (I_VALUE, "CurrentValue", Obj::Node(win, key), Some(n)) if n.actions & ax::A_VALUE != 0 => {
                            let x = match v {
                                Value::D(d) => d,
                                Value::I(i) => i as f64,
                                other => other.as_u32().unwrap_or(0) as f64,
                            };
                            ax::request(Request { win, key, act: Act::Value(x) });
                            Ok(Vec::new())
                        }
                        _ => Err("org.freedesktop.DBus.Error.PropertyReadOnly"),
                    }
                }
                _ => Err(UNKNOWN),
            };
        }
        let b = |x: bool| Ok(alloc::vec![Value::B(x)]);
        match (m.iface.as_str(), m.member.as_str()) {
            (I_ACC, "GetChildren") => {
                let kids: Vec<Value> = self.children(s, o, w, k).into_iter().map(|c| self.obj_ref(c)).collect();
                Ok(alloc::vec![Value::A(String::from("(so)"), kids)])
            }
            (I_ACC, "GetChildAtIndex") => {
                let i = arg_i(m, 0);
                let kid = (i >= 0).then(|| self.children(s, o, w, k).into_iter().nth(i as usize)).flatten().unwrap_or(Obj::Null);
                Ok(alloc::vec![self.obj_ref(kid)])
            }
            (I_ACC, "GetIndexInParent") => Ok(alloc::vec![Value::I(match o {
                Obj::App | Obj::Null => -1,
                Obj::Win(win) => s.wins.iter().position(|t| t.win == win).map_or(-1, |i| i as i32),
                Obj::Node(..) => node.map_or(-1, |n| n.index as i32),
            })]),
            (I_ACC, "GetRole") => Ok(alloc::vec![Value::U(self.role(o, w, node))]),
            (I_ACC, "GetRoleName") => Ok(alloc::vec![Value::str(role_name(self.role(o, w, node)))]),
            // (what Orca says: in the UI language)
            (I_ACC, "GetLocalizedRoleName") => Ok(alloc::vec![Value::str(match (o, w, node) {
                (Obj::App, ..) => ax::role_text(ax::ROLE_APP),
                (Obj::Win(_), Some(t), _) if t.dialog => ax::role_name(Role::Dialog),
                (Obj::Win(_), Some(_), _) => ax::role_text(ax::ROLE_WINDOW),
                (Obj::Node(..), _, Some(n)) => ax::role_name(n.role),
                _ => ax::role_text(ax::ROLE_UNKNOWN),
            })]),
            (I_ACC, "GetState") => {
                let st = match (o, w, node) {
                    (Obj::Node(..), Some(t), Some(n)) => node_states(t, n),
                    (Obj::Win(_), Some(t), _) => win_states(t),
                    _ => 0,
                };
                Ok(alloc::vec![Value::A(String::from("u"), alloc::vec![Value::U(st as u32), Value::U((st >> 32) as u32)])])
            }
            (I_ACC, "GetAttributes") => Ok(alloc::vec![match (o, node) {
                (Obj::App, _) => Value::A(String::from("{ss}"), alloc::vec![Value::E(Box::new(Value::str("toolkit")), Box::new(Value::str("JFLib")))]),
                // a heading's level (Orca: "heading level 2", heading navigation by level)
                (Obj::Node(..), Some(n)) if n.role == Role::Heading => Value::A(
                    String::from("{ss}"),
                    alloc::vec![Value::E(Box::new(Value::str("level")), Box::new(Value::S(alloc::format!("{}", n.level()))))],
                ),
                _ => empty_ss(),
            }]),
            (I_ACC, "GetApplication") => Ok(alloc::vec![self.obj_ref(Obj::App)]),
            (I_ACC, "GetRelationSet") => Ok(alloc::vec![Value::A(String::from("(ua(so))"), Vec::new())]),
            (I_ACC, "GetInterfaces") => {
                let mut l = alloc::vec![I_ACC];
                match (o, node) {
                    (Obj::App, _) => l.push(I_APP),
                    (Obj::Win(_), _) => l.push(I_COMP),
                    (Obj::Node(..), Some(n)) => {
                        l.push(I_COMP);
                        if n.actions & ax::A_CLICK != 0 {
                            l.push(I_ACTION);
                        }
                        if matches!(n.role, Role::Slider | Role::Progress) {
                            l.push(I_VALUE);
                        }
                        if text_of(n).is_some() {
                            l.push(I_TEXT);
                        }
                        if n.role == Role::TextBox {
                            l.push(I_EDIT);
                        }
                    }
                    _ => {}
                }
                Ok(alloc::vec![Value::A(String::from("s"), l.into_iter().map(Value::str).collect())])
            }
            (I_APP, "GetLocale") => Ok(alloc::vec![Value::S(crate::system_locale().unwrap_or_default())]),
            (I_APP, "GetApplicationBusAddress") => Ok(alloc::vec![Value::str("")]),
            // Component
            (I_COMP, "GetExtents") => {
                let r = self.extents(w, node, arg_i(m, 0) as u32).ok_or(GONE)?;
                Ok(alloc::vec![Value::R(r.iter().map(|&v| Value::I(v)).collect())])
            }
            (I_COMP, "GetPosition") => {
                let r = self.extents(w, node, arg_i(m, 0) as u32).ok_or(GONE)?;
                Ok(alloc::vec![Value::I(r[0]), Value::I(r[1])])
            }
            (I_COMP, "GetSize") => {
                let r = self.extents(w, node, 1).ok_or(GONE)?;
                Ok(alloc::vec![Value::I(r[2]), Value::I(r[3])])
            }
            (I_COMP, "Contains") => {
                let r = self.extents(w, node, arg_i(m, 2) as u32).ok_or(GONE)?;
                let (x, y) = (arg_i(m, 0), arg_i(m, 1));
                b(x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3])
            }
            (I_COMP, "GetAccessibleAtPoint") => {
                let t = w.ok_or(GONE)?;
                let wr = self.extents(Some(t), None, arg_i(m, 2) as u32).ok_or(GONE)?;
                let (x, y) = (arg_i(m, 0) - wr[0], arg_i(m, 1) - wr[1]);
                // the deepest node at the point that is this object or below it
                let mut hit = t.at_point(x, y).map(|i| i as u32);
                if k != NONE {
                    let mut p = hit;
                    while let Some(i) = p.filter(|&i| i != k) {
                        p = Some(t.nodes[i as usize].parent).filter(|&q| q != NONE);
                    }
                    if p != Some(k) {
                        hit = None;
                    }
                }
                let o = hit.filter(|&i| i != k).map_or(Obj::Null, |i| Self::node_obj(t, i));
                Ok(alloc::vec![self.obj_ref(o)])
            }
            (I_COMP, "GetLayer") => Ok(alloc::vec![Value::U(if node.is_some() { 3 } else { 7 })]),
            (I_COMP, "GrabFocus") => {
                let ok = matches!((o, node), (Obj::Node(..), Some(n)) if n.actions & ax::A_FOCUS != 0);
                if let (true, Obj::Node(win, key)) = (ok, o) {
                    ax::request(Request { win, key, act: Act::Focus });
                }
                b(ok)
            }
            (I_COMP, "GetAlpha") => Ok(alloc::vec![Value::D(1.0)]),
            (I_COMP, "SetExtents" | "SetPosition" | "SetSize" | "ScrollTo" | "ScrollToPoint") => b(false),
            // Action
            (I_ACTION, "GetActions") => {
                let n = if node.is_some_and(|n| n.actions & ax::A_CLICK != 0) { 1 } else { 0 };
                let l = (0..n).map(|_| Value::R(alloc::vec![Value::str("click"), Value::str("click"), Value::str("")])).collect();
                Ok(alloc::vec![Value::A(String::from("(sss)"), l)])
            }
            (I_ACTION, "GetName" | "GetLocalizedName" | "GetDescription") => {
                Ok(alloc::vec![Value::str(if arg_i(m, 0) == 0 && node.is_some_and(|n| n.actions & ax::A_CLICK != 0) { "click" } else { "" })])
            }
            (I_ACTION, "GetKeyBinding") => Ok(alloc::vec![Value::str("")]),
            (I_ACTION, "DoAction") => {
                let ok = arg_i(m, 0) == 0 && node.is_some_and(|n| n.actions & ax::A_CLICK != 0);
                if let (true, Obj::Node(win, key)) = (ok, o) {
                    ax::request(Request { win, key, act: Act::Click });
                }
                b(ok)
            }
            // Text (the basic queries)
            (I_TEXT, member) => {
                let text = node.and_then(text_of).ok_or(UNKNOWN)?;
                let n = text.chars().count() as i32;
                match member {
                    "GetText" => Ok(alloc::vec![Value::S(sub(text, arg_i(m, 0), arg_i(m, 1)))]),
                    "GetCharacterAtOffset" => {
                        let ch = text.chars().nth(arg_i(m, 0).max(0) as usize).map_or(0, |c| c as i32);
                        Ok(alloc::vec![Value::I(if arg_i(m, 0) < 0 { 0 } else { ch })])
                    }
                    "GetStringAtOffset" => {
                        let (t, a, e) = around(text, arg_i(m, 0), arg_i(m, 1) as u32);
                        Ok(alloc::vec![Value::S(t), Value::I(a), Value::I(e)])
                    }
                    "GetTextAtOffset" | "GetTextBeforeOffset" | "GetTextAfterOffset" => {
                        // boundaries: 0 char, 1 / 2 word, 3 / 4 sentence, 5 / 6 line
                        let bt = arg_i(m, 1) as u32;
                        let gran = match bt {
                            0 => 0,
                            1 | 2 => 1,
                            _ => 3,
                        };
                        let mut off = arg_i(m, 0);
                        if member == "GetTextBeforeOffset" {
                            let (_, a, _) = around(text, off, gran);
                            off = a - 1;
                        } else if member == "GetTextAfterOffset" {
                            let (_, _, e) = around(text, off, gran);
                            off = e;
                        }
                        let (t, a, e) = if off < 0 || off > n { (String::new(), 0, 0) } else { around(text, off, gran) };
                        Ok(alloc::vec![Value::S(t), Value::I(a), Value::I(e)])
                    }
                    "GetNSelections" => {
                        let (a, e) = node.map_or((0, 0), |n| n.selection());
                        Ok(alloc::vec![Value::I((node.is_some_and(|n| n.caret >= 0) && a < e) as i32)])
                    }
                    "GetSelection" => {
                        let (a, e) = node.filter(|n| n.caret >= 0 && arg_i(m, 0) == 0).map_or((0, 0), |n| n.selection());
                        Ok(alloc::vec![Value::I(a), Value::I(if a < e { e } else { a })])
                    }
                    // the caret and the selection of a text box (it gets the focus first)
                    "SetCaretOffset" | "SetSelection" | "AddSelection" | "RemoveSelection" => {
                        let (Some(nd), Obj::Node(win, key)) = (node.filter(|n| n.role == Role::TextBox && n.actions & ax::A_FOCUS != 0), o)
                        else {
                            return b(false);
                        };
                        let (a, c) = match member {
                            "SetCaretOffset" => (arg_i(m, 0), arg_i(m, 0)),
                            "SetSelection" => (arg_i(m, 1), arg_i(m, 2)),
                            "AddSelection" => (arg_i(m, 0), arg_i(m, 1)),
                            _ => (nd.caret.max(0), nd.caret.max(0)),
                        };
                        let (a, c) = (a.clamp(0, n), if c < 0 { n } else { c.clamp(0, n) });
                        ax::request(Request { win, key, act: Act::Select(a, c) });
                        b(true)
                    }
                    "ScrollSubstringTo" | "ScrollSubstringToPoint" => b(false),
                    "GetAttributes" | "GetAttributeRun" => Ok(alloc::vec![empty_ss(), Value::I(0), Value::I(n)]),
                    "GetDefaultAttributes" | "GetDefaultAttributeSet" => Ok(alloc::vec![empty_ss()]),
                    "GetAttributeValue" => Ok(alloc::vec![Value::str("")]),
                    // (magnifiers ask the caret's character after text-caret-moved)
                    "GetCharacterExtents" => {
                        let (t, nd) = (w.ok_or(GONE)?, node.ok_or(GONE)?);
                        let r = coords(t, nd, nd.char_rect(arg_i(m, 0)), arg_i(m, 1) as u32);
                        Ok(r.iter().map(|&v| Value::I(v)).collect())
                    }
                    "GetRangeExtents" => {
                        let (t, nd) = (w.ok_or(GONE)?, node.ok_or(GONE)?);
                        let (a, e) = (arg_i(m, 0).clamp(0, n), arg_i(m, 1).clamp(0, n));
                        let mut r = nd.char_rect(a.min(e));
                        for k in a.min(e) + 1..a.max(e) {
                            let q = nd.char_rect(k);
                            let (x0, y0) = (r[0].min(q[0]), r[1].min(q[1]));
                            r = [x0, y0, (r[0] + r[2]).max(q[0] + q[2]) - x0, (r[1] + r[3]).max(q[1] + q[3]) - y0];
                        }
                        let r = coords(t, nd, r, arg_i(m, 2) as u32);
                        Ok(r.iter().map(|&v| Value::I(v)).collect())
                    }
                    "GetOffsetAtPoint" => {
                        let (t, nd) = (w.ok_or(GONE)?, node.ok_or(GONE)?);
                        let o = coords(t, nd, [0; 4], arg_i(m, 2) as u32);
                        Ok(alloc::vec![Value::I(nd.offset_at(arg_i(m, 0) - o[0], arg_i(m, 1) - o[1]))])
                    }
                    "GetBoundedRanges" => Ok(alloc::vec![Value::A(String::from("(iisv)"), Vec::new())]),
                    _ => Err(UNKNOWN),
                }
            }
            // EditableText
            (I_EDIT, member) => {
                let n = node.filter(|n| n.actions & ax::A_TEXT != 0).ok_or(UNKNOWN)?;
                let Obj::Node(win, key) = o else { return Err(UNKNOWN) };
                let cur: Vec<char> = n.text.chars().collect();
                let new: Option<String> = match member {
                    "SetTextContents" => Some(String::from(arg_s(m, 0))),
                    "InsertText" => {
                        let at = (arg_i(m, 0).max(0) as usize).min(cur.len());
                        let ins: String = arg_s(m, 1).chars().take(arg_i(m, 2).max(0) as usize).collect();
                        Some(cur[..at].iter().collect::<String>() + &ins + &cur[at..].iter().collect::<String>())
                    }
                    "DeleteText" => {
                        let a = (arg_i(m, 0).max(0) as usize).min(cur.len());
                        let e = if arg_i(m, 1) < 0 { cur.len() } else { (arg_i(m, 1) as usize).clamp(a, cur.len()) };
                        Some(cur[..a].iter().collect::<String>() + &cur[e..].iter().collect::<String>())
                    }
                    "CopyText" => return Ok(Vec::new()),
                    "CutText" | "PasteText" => None,
                    _ => return Err(UNKNOWN),
                };
                match new {
                    Some(t) => {
                        ax::request(Request { win, key, act: Act::Text(t) });
                        b(true)
                    }
                    None => b(false),
                }
            }
            _ => Err(UNKNOWN),
        }
    }

    fn role(&self, o: Obj, w: Option<&WinTree>, node: Option<&Node>) -> u32 {
        match (o, w, node) {
            (Obj::App, ..) => R_APPLICATION,
            (Obj::Win(_), Some(t), _) => {
                if t.dialog {
                    R_DIALOG
                } else {
                    R_FRAME
                }
            }
            (Obj::Node(..), _, Some(n)) => role_of(n.role),
            _ => 0,
        }
    }

    // the children of an object
    fn children(&self, s: &Snapshot, o: Obj, w: Option<&WinTree>, k: u32) -> Vec<Obj> {
        match (o, w) {
            (Obj::App, _) => s.wins.iter().map(|t| Obj::Win(t.win)).collect(),
            (Obj::Win(_) | Obj::Node(..), Some(t)) => t.children(k).map(|i| Obj::Node(t.win, t.nodes[i].key)).collect(),
            _ => Vec::new(),
        }
    }

    // x, y, w, h of a window (node None) or node: coordinate type 0 screen, 1 window, 2 parent
    fn extents(&self, w: Option<&WinTree>, node: Option<&Node>, ctype: u32) -> Option<[i32; 4]> {
        let t = w?;
        let Some(n) = node else {
            let (x, y) = if ctype == 0 { (t.origin[0], t.origin[1]) } else { (0, 0) };
            return Some([x, y, t.size[0], t.size[1]]);
        };
        let mut r = n.rect;
        match ctype {
            0 => {
                r[0] += t.origin[0];
                r[1] += t.origin[1];
            }
            2 => {
                if let Some(p) = t.nodes.get(n.parent as usize) {
                    r[0] -= p.rect[0];
                    r[1] -= p.rect[1];
                }
            }
            _ => {}
        }
        Some(r)
    }

    fn property(&self, s: &Snapshot, o: Obj, w: Option<&WinTree>, node: Option<&Node>, iface: &str, name: &str) -> Option<Value> {
        Some(match (iface, name) {
            (I_ACC, "Name") => Value::S(match (o, w, node) {
                (Obj::App, ..) => self.app.clone(),
                (Obj::Win(_), Some(t), _) => t.title.clone(),
                (_, _, Some(n)) => n.name.clone(),
                _ => String::new(),
            }),
            (I_ACC, "Description" | "Locale" | "HelpText") => Value::str(""),
            (I_ACC, "AccessibleId") => Value::S(match (o, node) {
                (Obj::Node(..), Some(n)) => alloc::format!("{:x}", n.key),
                _ => String::new(),
            }),
            (I_ACC, "Parent") => match (o, w, node) {
                (Obj::App, ..) => Value::R(alloc::vec![Value::S(self.parent.0.clone()), Value::O(self.parent.1.clone())]),
                (Obj::Win(_), ..) => self.obj_ref(Obj::App),
                (Obj::Node(..), Some(t), Some(n)) => self.obj_ref(Self::node_obj(t, n.parent)),
                _ => self.obj_ref(Obj::Null),
            },
            (I_ACC, "ChildCount") => Value::I(match (o, w, node) {
                (Obj::App, ..) => s.wins.len() as i32,
                (Obj::Win(_), Some(t), _) => t.count as i32,
                (_, _, Some(n)) => n.count as i32,
                _ => 0,
            }),
            (I_APP, "ToolkitName") if o == Obj::App => Value::str("JFLib"),
            (I_APP, "Version") if o == Obj::App => Value::str("1.0"),
            (I_APP, "AtspiVersion") if o == Obj::App => Value::str("2.1"),
            (I_APP, "Id") if o == Obj::App => Value::I(self.app_id),
            (I_ACTION, "NActions") => Value::I(node.is_some_and(|n| n.actions & ax::A_CLICK != 0) as i32),
            (I_VALUE, _) => {
                let n = node.filter(|n| matches!(n.role, Role::Slider | Role::Progress))?;
                match name {
                    "MinimumValue" => Value::D(n.min),
                    "MaximumValue" => Value::D(n.max),
                    "MinimumIncrement" => Value::D(n.step),
                    "CurrentValue" => Value::D(n.value.max(n.min)),
                    "Text" => Value::S(if n.value < n.min { String::new() } else { alloc::format!("{}", n.value) }),
                    _ => return None,
                }
            }
            (I_TEXT, "CharacterCount") => Value::I(node.and_then(text_of)?.chars().count() as i32),
            (I_TEXT, "CaretOffset") => {
                let n = node?;
                let len = text_of(n)?.chars().count() as i32;
                Value::I(if n.caret >= 0 { n.caret.min(len) } else { len })
            }
            _ => return None,
        })
    }
}
