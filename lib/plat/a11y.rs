// a11y -- the accessibility tree for screen readers and other assistive technology
//
// The UI (ui/a11y.rs) turns the semantic records of every frame into a Snapshot and publishes
// it here; the platform bridge serves it from threads of its own and hands the actions of the
// assistive technology back to the UI thread:
//
//   start(app)                   start the bridge (once; the UI's first window frame)
//   active() -> bool             assistive technology may be listening: publish snapshots
//   publish(Snapshot)            the new tree; its differences to the last one become events
//                                (focus, states, names, values, text, children, windows)
//   take() -> Vec<Request>       the actions asked for since the last call (UI thread)
//   request(Request)             (bridges, tests) queue an action and wake the UI thread
//   current() -> Option<Arc<Snapshot>>   (bridges) the tree last published
//   set_translator(f) / role_name(role) / role_text(k)   the names of the roles the bridges
//                                give (UIA LocalizedControlType, AT-SPI GetLocalizedRoleName) in
//                                the UI language: ROLE_TEXTS holds them in German, the keys of
//                                the UI's translation (ui registers i18n::tr; plat cannot ask
//                                ui itself); without one they stay German
//
// Linux: AT-SPI2 on the accessibility bus (linux_atspi.rs).  Windows: UI Automation providers
// answering WM_GETOBJECT (win_uia.rs).  The browser and Android (web_a11y.rs): the host keeps
// a mirror of the tree -- an ARIA shadow tree in the DOM (web/a11y.js), virtual views of an
// AccessibilityNodeProvider (android/GalleryAccessibility.java) -- fed with the differences
// of every published tree (a11y_wire.rs: Mirror, the wire format); the host turns the mirror
// on (active() follows it) and sends the actions back.
//
// A Snapshot holds every shown window (WinTree) with its nodes in the order of the UI's
// records; Node::parent gives the hierarchy (NONE: a top-level node of the window), link()
// adds the sibling links and the key index the bridges navigate with.  A node's key stays the
// same from frame to frame while its widget does (the UI derives it from the widget id).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

/// no node (Node::parent of a top-level node, WinTree::focus without a focused node)
pub const NONE: u32 = u32::MAX;

/// what a node is
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// a container without a meaning of its own (a titled box)
    #[default]
    Group,
    Label,
    Button,
    CheckBox,
    Radio,
    /// an on / off switch
    Switch,
    Slider,
    Progress,
    TextBox,
    Link,
    /// a selectable list and its rows
    List,
    ListItem,
    /// the menu bar, a menu (a menu bar entry or a popup menu) and its entries
    MenuBar,
    Menu,
    MenuItem,
    /// the strip of tabs and one tab
    TabList,
    Tab,
    /// a drop-down list (Node::text: the current choice)
    Combo,
    Image,
    /// a drawing surface (charts, the app's own views)
    Canvas,
    Dialog,
    /// a heading of a document or section (Node::value: its level, level())
    Heading,
}

/// Node::state: the keyboard focus is on the node
pub const S_FOCUSED: u32 = 1;
/// it can take the keyboard focus
pub const S_FOCUSABLE: u32 = 2;
/// greyed out: no input, no actions
pub const S_DISABLED: u32 = 4;
/// a check box, radio button or switch that is on
pub const S_CHECKED: u32 = 8;
/// a check box in the third state ("some")
pub const S_MIXED: u32 = 16;
/// a selected list row or tab
pub const S_SELECTED: u32 = 32;
/// an open drop-down, menu or dialog
pub const S_EXPANDED: u32 = 64;
/// a button shown pressed (a toggle or tool button that is on)
pub const S_PRESSED: u32 = 128;
/// a text box with several lines
pub const S_MULTILINE: u32 = 256;
/// a password box (its text: one dot per character)
pub const S_PROTECTED: u32 = 512;
/// a text box that can be read and selected but not edited
pub const S_READONLY: u32 = 1024;

/// Node::actions: press, toggle, select, open / close -- what a click on the widget does
pub const A_CLICK: u32 = 1;
/// give it the keyboard focus
pub const A_FOCUS: u32 = 2;
/// set a number (sliders)
pub const A_VALUE: u32 = 4;
/// set the text (text boxes)
pub const A_TEXT: u32 = 8;

/// one element of a window
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    /// identity within its window, the same from frame to frame (never 0)
    pub key: u64,
    pub role: Role,
    /// S_*
    pub state: u32,
    /// A_*
    pub actions: u32,
    /// x, y, w, h: physical px of the window's client area
    pub rect: [i32; 4],
    pub name: String,
    /// text boxes: their text; combo boxes: the current choice; else empty
    pub text: String,
    /// text boxes: the caret in characters (-1 unknown)
    pub caret: i32,
    /// text boxes: the other end of the selection in characters (== caret: nothing selected)
    pub anchor: i32,
    /// text boxes: the caret's rectangle, physical px of the client area (w, h 0: unknown) --
    /// screen magnifiers follow it (char_rect)
    pub caret_rect: [i32; 4],
    /// sliders and progress bars: value in min ..= max (value < min: progress unknown);
    /// headings: the level (1 ..= 9)
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// index of the parent node in WinTree::nodes (NONE: a top-level node)
    pub parent: u32,
    /// (link) first child, next and previous sibling (NONE: none), number of children,
    /// position among the siblings
    pub first: u32,
    pub next: u32,
    pub prev: u32,
    pub count: u32,
    pub index: u32,
}

/// one shown window
#[derive(Clone, Debug, Default)]
pub struct WinTree {
    /// the UI's window index (0 = the main window)
    pub win: u8,
    /// its platform window (plat::win_*; -1 none)
    pub pw: i32,
    pub title: String,
    /// a dialog window (modal)
    pub dialog: bool,
    /// it has the keyboard
    pub active: bool,
    /// screen position of the client area (px; 0, 0 when the platform does not tell)
    pub origin: [i32; 2],
    /// client size (px)
    pub size: [i32; 2],
    pub nodes: Vec<Node>,
    /// the focused node (NONE: the window itself)
    pub focus: u32,
    /// (link) the first top-level node, their number
    pub first: u32,
    pub count: u32,
    /// (link) key -> node index, sorted by key
    pub keys: Vec<(u64, u32)>,
}

/// the whole application
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// the application's name
    pub app: String,
    pub wins: Vec<WinTree>,
}

impl Node {
    /// the text the text interfaces see: text boxes and drop-downs their text, labels their name
    pub fn text_str(&self) -> &str {
        match self.role {
            Role::TextBox | Role::Combo => &self.text,
            _ => &self.name,
        }
    }

    /// a heading's level 1 ..= 9 (0: not a heading)
    pub fn level(&self) -> u32 {
        if self.role == Role::Heading { (self.value as u32).clamp(1, 9) } else { 0 }
    }

    /// the selection of a text box: (start, end) in characters, start == end at the caret
    pub fn selection(&self) -> (i32, i32) {
        let a = if self.anchor >= 0 { self.anchor } else { self.caret };
        (a.min(self.caret).max(0), a.max(self.caret).max(0))
    }

    /// the rectangle of character k (client px): exact at the caret of a focused text box
    /// (caret_rect), around it estimated from the line height (lines after '\n', columns of
    /// about half a line height); without a caret the characters share the node's width
    pub fn char_rect(&self, k: i32) -> [i32; 4] {
        let r = self.rect;
        let s = self.text_str();
        let n = s.chars().count() as i32;
        let k = k.clamp(0, n);
        let c = self.caret_rect;
        if self.caret < 0 || c[3] <= 0 {
            let cw = if n > 0 { (r[2] / n).max(1) } else { r[2].max(1) };
            return [r[0] + cw * k.min((n - 1).max(0)), r[1], cw, r[3]];
        }
        // line and column of a character offset
        let at = |p: i32| {
            let (mut line, mut col) = (0, 0);
            for ch in s.chars().take(p.max(0) as usize) {
                if ch == '\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
            }
            (line, col)
        };
        let cw = (c[3] * 11 / 20).max(1);
        let ((lk, ck), (lc, cc)) = (at(k), at(self.caret));
        let x = if lk == lc { c[0] + (ck - cc) * cw } else { r[0] + (c[3] >> 2) + ck * cw };
        let y = c[1] + (lk - lc) * c[3];
        let x = x.clamp(r[0], (r[0] + r[2] - 1).max(r[0]));
        [x, y, if k == self.caret { c[2].max(1) } else { cw }, c[3]]
    }

    /// the character offset nearest to client point x, y (char_rect's inverse; -1: outside)
    pub fn offset_at(&self, x: i32, y: i32) -> i32 {
        let r = self.rect;
        if x < r[0] || y < r[1] || x >= r[0] + r[2] || y >= r[1] + r[3] {
            return -1;
        }
        let n = self.text_str().chars().count() as i32;
        let mut best = (i64::MAX, 0);
        for k in 0..=n {
            let q = self.char_rect(k);
            let dy = if y < q[1] { q[1] - y } else if y >= q[1] + q[3] { y - q[1] - q[3] + 1 } else { 0 };
            let d = (dy as i64) * 100_000 + (x - q[0]).abs() as i64;
            if d < best.0 {
                best = (d, k);
            }
        }
        best.1
    }
}

impl WinTree {
    /// fill the links (first / next / prev / count / index, the key index) from the parents
    pub fn link(&mut self) {
        let n = self.nodes.len();
        let mut last = alloc::vec![NONE; n]; // last child of each node
        let mut top_last = NONE;
        self.first = NONE;
        self.count = 0;
        for i in 0..n {
            let nd = &mut self.nodes[i];
            nd.first = NONE;
            nd.next = NONE;
            nd.prev = NONE;
            nd.count = 0;
            if nd.parent != NONE && nd.parent as usize >= i {
                nd.parent = NONE; // parents come first (ui/a11y.rs orders them)
            }
        }
        for i in 0..n {
            let p = self.nodes[i].parent;
            let prev = if p == NONE { top_last } else { last[p as usize] };
            self.nodes[i].prev = prev;
            if prev != NONE {
                self.nodes[prev as usize].next = i as u32;
            }
            if p == NONE {
                if self.first == NONE {
                    self.first = i as u32;
                }
                self.nodes[i].index = self.count;
                self.count += 1;
                top_last = i as u32;
            } else {
                let pp = &mut self.nodes[p as usize];
                if pp.first == NONE {
                    pp.first = i as u32;
                }
                let k = pp.count;
                pp.count += 1;
                self.nodes[i].index = k;
                last[p as usize] = i as u32;
            }
        }
        self.keys = self.nodes.iter().enumerate().map(|(i, nd)| (nd.key, i as u32)).collect();
        self.keys.sort_unstable();
    }

    /// the node with this key
    pub fn find(&self, key: u64) -> Option<usize> {
        self.keys.binary_search_by_key(&key, |e| e.0).ok().map(|k| self.keys[k].1 as usize)
    }

    /// the children of a node (NONE: the top-level nodes), in order
    pub fn children(&self, parent: u32) -> impl Iterator<Item = usize> + '_ {
        let mut i = if parent == NONE { self.first } else { self.nodes.get(parent as usize).map_or(NONE, |n| n.first) };
        core::iter::from_fn(move || {
            if i == NONE {
                return None;
            }
            let k = i as usize;
            i = self.nodes[k].next;
            Some(k)
        })
    }

    /// the child number k of a node (NONE: of the window)
    pub fn child(&self, parent: u32, k: usize) -> Option<usize> {
        self.children(parent).nth(k)
    }

    /// the deepest node at a client point (px): among siblings the last one wins (drawn on top)
    pub fn at_point(&self, x: i32, y: i32) -> Option<usize> {
        let inside = |r: &[i32; 4]| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
        let mut found = None;
        let mut parent = NONE;
        loop {
            let hit = self.children(parent).filter(|&k| inside(&self.nodes[k].rect)).last();
            match hit {
                Some(k) => {
                    found = Some(k);
                    parent = k as u32;
                }
                None => return found,
            }
        }
    }

    /// the node lies (partly) inside the window's client area (else it is scrolled away)
    pub fn on_screen(&self, n: &Node) -> bool {
        let r = n.rect;
        r[2] > 0 && r[3] > 0 && r[0] < self.size[0] && r[1] < self.size[1] && r[0] + r[2] > 0 && r[1] + r[3] > 0
    }

    /// the focused node's key (0: the window itself)
    pub fn focus_key(&self) -> u64 {
        self.nodes.get(self.focus as usize).map_or(0, |n| n.key)
    }
}

impl Snapshot {
    /// the tree of the UI's window index win
    pub fn win(&self, win: u8) -> Option<&WinTree> {
        self.wins.iter().find(|w| w.win == win)
    }

    /// the tree of platform window pw
    pub fn pw(&self, pw: i32) -> Option<&WinTree> {
        self.wins.iter().find(|w| w.pw == pw)
    }
}

// ---------------------------------------------------------------- changes

/// what changed between two snapshots (window: the UI's window index; key 0: the window)
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// a window appeared (true) or went away
    Window(u8, bool),
    /// a window got (true) or lost the keyboard
    Activate(u8, bool),
    /// the keyboard focus moved to this node (in the active window)
    Focus(u8, u64),
    /// state bit S_* of a node went on (true) or off
    State(u8, u64, u32, bool),
    Name(u8, u64),
    /// the number of a slider / progress bar, the level of a heading
    Value(u8, u64),
    /// the text of a text box / combo box (the old text)
    Text(u8, u64, String),
    /// the caret of a text box (characters)
    Caret(u8, u64, i32),
    /// a node (with its subtree) appeared: parent key (0: the window), its key; in the order
    /// of the new tree, so its index there is right when the event arrives
    Added(u8, u64, u64),
    /// a node (with its subtree) went away: parent key (0: the window), its key, its index
    /// there before; the last index of a parent first
    Removed(u8, u64, u64, u32),
}

const STATE_EVENTS: u32 = S_CHECKED | S_MIXED | S_SELECTED | S_EXPANDED | S_DISABLED | S_PRESSED;

/// the changes from old to new
pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    for o in &old.wins {
        if new.win(o.win).is_none() {
            out.push(Change::Window(o.win, false));
        }
    }
    for w in &new.wins {
        let Some(o) = old.win(w.win) else {
            out.push(Change::Window(w.win, true));
            if w.active {
                out.push(Change::Activate(w.win, true));
                out.push(Change::Focus(w.win, w.focus_key()));
                caret_of_focus(w, &mut out);
            }
            continue;
        };
        let key_of = |t: &WinTree, p: u32| if p == NONE { 0 } else { t.nodes[p as usize].key };
        // the roots of the subtrees that went away (their parent stays) and that came (their
        // parent was there before); a node that moved to another parent is both
        let gone = |t: &WinTree, nd: &Node, other: &WinTree| match other.find(nd.key) {
            None => true,
            Some(k) => key_of(other, other.nodes[k].parent) != key_of(t, nd.parent),
        };
        let mut removed: Vec<(u64, u32, u64)> = Vec::new(); // (parent, index, key)
        for od in &o.nodes {
            let p = key_of(o, od.parent);
            if gone(o, od, w) && (p == 0 || w.find(p).is_some_and(|k| !gone(w, &w.nodes[k], o))) {
                removed.push((p, od.index, od.key));
            }
        }
        removed.sort_unstable_by(|a, b| b.cmp(a));
        for (p, index, key) in removed {
            out.push(Change::Removed(w.win, p, key, index));
        }
        for nd in &w.nodes {
            let p = key_of(w, nd.parent);
            if gone(w, nd, o) && (p == 0 || o.find(p).is_some_and(|k| !gone(o, &o.nodes[k], w))) {
                out.push(Change::Added(w.win, p, nd.key));
            }
        }
        for nd in &w.nodes {
            match o.find(nd.key) {
                None => {}
                Some(k) => {
                    let od = &o.nodes[k];
                    let ch = (od.state ^ nd.state) & STATE_EVENTS;
                    for b in 0..32 {
                        if ch & (1 << b) != 0 {
                            out.push(Change::State(w.win, nd.key, 1 << b, nd.state & (1 << b) != 0));
                        }
                    }
                    if od.name != nd.name {
                        out.push(Change::Name(w.win, nd.key));
                    }
                    if od.value != nd.value || od.min != nd.min || od.max != nd.max {
                        out.push(Change::Value(w.win, nd.key));
                    }
                    if od.text != nd.text {
                        out.push(Change::Text(w.win, nd.key, od.text.clone()));
                    }
                    if od.caret != nd.caret && nd.caret >= 0 && nd.state & S_FOCUSED != 0 {
                        out.push(Change::Caret(w.win, nd.key, nd.caret));
                    }
                }
            }
        }
        if o.active != w.active {
            out.push(Change::Activate(w.win, w.active));
        }
        if w.active && (!o.active || o.focus_key() != w.focus_key()) {
            out.push(Change::Focus(w.win, w.focus_key()));
            caret_of_focus(w, &mut out);
        }
    }
    out
}

// the focus moved to a text box: its caret too (magnifiers follow the caret, not the box)
fn caret_of_focus(w: &WinTree, out: &mut Vec<Change>) {
    if let Some(n) = w.nodes.get(w.focus as usize).filter(|n| n.caret >= 0) {
        let c = Change::Caret(w.win, n.key, n.caret);
        if !out.contains(&c) {
            out.push(c);
        }
    }
}

// ---------------------------------------------------------------- state, requests

/// an action of the assistive technology
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// press, toggle, select, open / close: what a click does (A_CLICK)
    Click,
    /// the keyboard focus (A_FOCUS)
    Focus,
    /// a new number (A_VALUE)
    Value(f64),
    /// a new text (A_TEXT)
    Text(String),
    /// (text boxes, A_FOCUS) the selection from anchor to caret in characters (equal: the caret
    /// alone); a box without the focus gets it first
    Select(i32, i32),
}

/// an action for a node
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// the UI's window index
    pub win: u8,
    pub key: u64,
    pub act: Act,
}

const REQ_MAX: usize = 64;

static CUR: SpinLock<Option<Arc<Snapshot>>> = SpinLock::new(None);
static REQ: SpinLock<Vec<Request>> = SpinLock::new(Vec::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// start the platform bridge (once): Linux connects to the accessibility bus when assistive
/// technology is enabled (jfgui_A11Y=1 forces it, jfgui_A11Y=0 or NO_AT_BRIDGE=1 turn it off);
/// Windows answers WM_GETOBJECT from now on; the browser and Android hosts turn their mirror
/// on themselves (jfgui_a11y), jfgui_A11Y=0 keeps it off there too
pub fn start(app: &str) {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    if crate::env("jfgui_A11Y").is_some_and(|v| v.starts_with('0')) || crate::env("NO_AT_BRIDGE").is_some_and(|v| v.starts_with('1')) {
        return;
    }
    native::start(app);
}

/// assistive technology may be listening (the UI publishes snapshots only then)
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// (bridges) assistive technology is there (true) or gone; true wakes the UI thread so that it
/// publishes
#[allow(dead_code)]
pub(crate) fn set_active(on: bool) {
    if ACTIVE.swap(on, Ordering::AcqRel) != on && on {
        crate::wake();
    }
}

/// the tree last published
pub fn current() -> Option<Arc<Snapshot>> {
    CUR.lock().clone()
}

/// publish a new tree: the links are filled, the differences to the last tree go to the bridge
pub fn publish(mut s: Snapshot) {
    for w in &mut s.wins {
        w.link();
    }
    let s = Arc::new(s);
    let old = CUR.lock().replace(s.clone());
    let changes = match &old {
        Some(o) => diff(o, &s),
        None => s.wins.iter().flat_map(|w| [Change::Window(w.win, true), Change::Activate(w.win, w.active)]).collect(),
    };
    if (!changes.is_empty() || MIRROR) && active() {
        native::changed(&s, changes);
    }
}

// the browser and Android hosts mirror the whole tree (rectangles too, which raise no event):
// their bridge sees every published tree
const MIRROR: bool = cfg!(any(target_arch = "wasm32", target_os = "android"));

#[path = "a11y_wire.rs"]
pub mod wire;

/// (bridges, tests) queue an action and wake the UI thread (the oldest is dropped when 64 wait)
pub fn request(r: Request) {
    {
        let mut q = REQ.lock();
        if q.len() >= REQ_MAX {
            q.remove(0);
        }
        q.push(r);
    }
    crate::wake();
}

/// the actions asked for since the last call, oldest first (UI thread)
pub fn take() -> Vec<Request> {
    core::mem::take(&mut *REQ.lock())
}

// ---------------------------------------------------------------- the texts of the bridges

/// what the bridges call the roles (UIA LocalizedControlType, AT-SPI GetLocalizedRoleName) in
/// Role order, then a window (AT-SPI frame), the application and an unknown role: in German,
/// the keys of their translations (set_translator; ui's i18n::tr has them in its languages)
pub const ROLE_TEXTS: [&str; 25] = [
    "Bereich",
    "Bezeichnung",
    "Schaltfläche",
    "Kontrollkästchen",
    "Optionsfeld",
    "Umschalter",
    "Schieberegler",
    "Fortschrittsanzeige",
    "Eingabefeld",
    "Verweis",
    "Listenfeld",
    "Listeneintrag",
    "Menüleiste",
    "Menü",
    "Menüeintrag",
    "Registerleiste",
    "Registerkarte",
    "Kombinationsfeld",
    "Grafik",
    "Zeichenfläche",
    "Dialog",
    "Überschrift",
    "Rahmen",
    "Anwendung",
    "unbekannt",
];
/// ROLE_TEXTS: a window
pub const ROLE_WINDOW: usize = 22;
/// ROLE_TEXTS: the application
pub const ROLE_APP: usize = 23;
/// ROLE_TEXTS: an unknown role
pub const ROLE_UNKNOWN: usize = 24;

static TR: SpinLock<Option<fn(&'static str) -> &'static str>> = SpinLock::new(None);

/// the translation of the bridges' texts into the UI language (ui sets i18n::tr with its first
/// frame; it reads the language each time, so the texts follow a change of it); without one
/// they stay German
pub fn set_translator(f: fn(&'static str) -> &'static str) {
    *TR.lock() = Some(f);
}

/// ROLE_TEXTS[k] in the UI language
pub fn role_text(k: usize) -> &'static str {
    let de = ROLE_TEXTS[k.min(ROLE_UNKNOWN)];
    let f = *TR.lock();
    f.map_or(de, |f| f(de))
}

/// what the bridges call a role, in the UI language
pub fn role_name(r: Role) -> &'static str {
    role_text(r as usize)
}

#[cfg(target_os = "linux")]
use crate::linux_atspi as native;

#[cfg(windows)]
use crate::win_uia as native;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
use crate::web::a11y_host as native;

#[cfg(not(any(windows, target_os = "linux", target_arch = "wasm32", target_os = "android")))]
mod native {
    use super::{Change, Snapshot};
    use alloc::vec::Vec;
    pub fn start(_app: &str) {}
    pub fn changed(_s: &Snapshot, _c: Vec<Change>) {}
}
