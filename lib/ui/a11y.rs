// a11y -- the semantic records as the platform's accessibility tree (plat::a11y): after the
// passes of a frame the records of every shown window become a snapshot that screen readers
// read through AT-SPI2 (Linux), UI Automation (Windows), the browser's ARIA shadow tree or
// Android's virtual views; their actions come back as requests that the next passes carry
// out through the widgets' own behaviour.
//
//   a11y_tree() -> plat::a11y::Snapshot   the tree as assistive technology gets it
// Runtime (runtime.rs): a11y_frame after the passes (publishes while plat::a11y::active();
// starts the bridge with the first window frame and hands it i18n::tr for the names of the
// roles: plat::a11y::set_translator), a11y_poll on E_WAKE (takes the requests).
// Widgets: behave asks a11y_click(id) (a pending click, toggle, select or open acts like
// Space on the focused widget: EV_ACT); the sliders ask a11y_value(id), the text boxes
// a11y_text(id) for a new value (both public for widgets of an app's own).  A click no widget took in its passes becomes a pointer
// click where the press reaches the record (a11y_aim), after the scroll views brought it into view; there is
// none when the middle lies on something else.
//
// The tree: every record of a window becomes a node in the record order -- roles from the
// kinds (K_*), states from SS_* (a check box with value 2 is mixed), a heading's level from its
// value (K_HEADING: Node::value, clamped to 1 ..= 9).  Left out: title bars, empty texts,
// records without area, boxes without a label (a box with tab records inside is the tab
// strip).  Containers are lists, dialogs, titled boxes and tab strips: a node's parent is the
// smallest one on its layer whose rectangle holds the node's centre -- inside a scoped record
// (SEM_SCOPE: a window of a window area, desk.rs) only one it holds, else itself, and scoped
// records hold nothing by their place (overlapping windows are siblings).  Menu bar entries
// next to each other get a menu bar; the rows of a popup (menus, drop-downs) a menu or list that
// hangs below the open record that opened it.  Names: the label; text boxes, drop-downs (their label
// is the text / the choice), sliders, progress bars and lists without one take the text or
// heading record right before them (to the left, or above and aligned).  The focus of a list
// or tab strip is passed on to its selected row / tab.
//
// Rich records (section 19 of the guide; a web page's accessibility tree): the kinds K_TABLE ..=
// K_FIGURE become nodes with a Sub (Node::role their nearest basic role); a record's SemAx (its
// setters: id64, parent, name, desc, url, value_text, states, set_pos, level, cell, span, grid,
// live, range) fills the further fields of its node.  A record with an id64 is keyed by it
// (RICH | id), takes its name from nothing but its label / name, and every request for it --
// focus, click, value, text, selection, scroll into view, expand / collapse, increment /
// decrement, select items -- goes to the app (ui.a11y_request_take) instead of the widgets.  A
// record with a parent hangs under the record of that id64 (else that widget id) of its window
// in the same frame; a left out record passes its children on to its own parent, an unknown
// parent or a loop leaves the record to its rectangle.
//
//   ui.announce(text, LIVE_*)       a text for the screen reader (the next frame publishes it)
//   ui.a11y_request_take()          the next request for a record with an id64
//   ui.a11y_active()                a client listens: publish what only it needs then

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use plat::a11y::{self as ax, Act, Node, Op, Role, Snapshot, Sub, WinTree, NONE};

use super::consts::*;
use super::{Sem, SemAx, Ui};

/// Sem::min of a record that holds the next Sem::max records of its window by their order, not
/// by their place (a window inside a window area, desk.rs: its content stays inside it where the
/// windows overlap)
pub(crate) const SEM_SCOPE: i32 = 0x5C09_E000;

// synthetic containers: keys outside the widget ids (bit 63, kind in bits 40..)
const SYN: u64 = 1 << 63;
const SYN_MENUBAR: u64 = 1 << 40;
const SYN_POPUP: u64 = 2 << 40;
// the keys of records with an id64: bit 62 and the id (below 2^62; widget keys are the
// occurrence << 32 | the widget id, far below)
pub(crate) const RICH: u64 = 1 << 62;
pub(crate) const ID_MASK: u64 = RICH - 1;
// at most this many requests wait for the app, announcements for the next frame
const REQS_MAX: usize = 64;
const ANNOUNCE_MAX: usize = 16;
// the longest text of a record's SemAx (bytes; a longer one is cut at a character)
const AX_TEXT_MAX: usize = 4096;

/// A screen reader's request for a record with an id64 (`ui.a11y_request_take`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct A11yRequest {
    /// the id of the record's window: 0 the main window, else the id its window_begin got
    pub win: u32,
    /// the record's id64
    pub id: u64,
    pub action: A11yAction,
}

/// What a screen reader asks of a record (`A11yRequest::action`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum A11yAction {
    /// give it the keyboard focus
    Focus,
    /// its default action: press, follow, toggle, select, open
    Click,
    /// scroll it into view
    ScrollIntoView,
    /// open / close it (AS_COLLAPSED, SS_OPEN)
    Expand,
    Collapse,
    /// one step up / down (spin buttons, sliders, scroll bars)
    Increment,
    Decrement,
    /// a new number (sliders, spin buttons, scroll bars)
    SetValue(f64),
    /// a new text (text boxes)
    SetText(String),
    /// the selection of a text box: anchor, caret in characters (equal: the caret alone)
    SelectText(u32, u32),
    /// select it alone (a row, a tab, a tree item)
    Select,
    /// add it to / take it out of the selection (AS_MULTISELECT containers)
    AddToSelection,
    RemoveFromSelection,
}

/// the requests waiting for the widgets (shared by all windows)
#[derive(Default)]
pub(crate) struct S {
    started: bool,
    /// the requests for records with an id64, for the app (a11y_request_take)
    reqs: VecDeque<A11yRequest>,
    /// texts to say with the next publish: window, text, politeness (announce)
    announce: Vec<(u8, String, u8)>,
    /// (headless runs) the texts said, for the scripts' `a11y expect announced`
    said: Vec<String>,
    /// a click for (window, id) and which record of that id it is (the fallback click)
    click: Option<(u8, u32, usize)>,
    value: Option<(u8, u32, i32)>,
    text: Option<(u8, u32, String)>,
    /// a selection for (window, text box): anchor, caret in characters (waits for the focus)
    select: Option<(u8, u32, i32, i32)>,
    /// the caret of the focused text box per window: the pass that drew it, x, y, w, h (window
    /// px; ime_pos reports it)
    caret: [(u32, [i32; 4]); WIN_MAX],
}

impl S {
    /// a click waits for its widget (behave asks a11y_click only then)
    pub(crate) fn pending(&self) -> bool {
        self.click.is_some()
    }
}

// a node being built: its node, its layer, where it hangs
struct Cand {
    nd: Node,
    layer: u8,
    /// place among its siblings (record order; a synthetic container: its first member's)
    pos: usize,
    container: bool,
    parent: usize,
    /// members of a synthetic container (their parent is fixed)
    fixed: bool,
    /// the innermost scoped record (SEM_SCOPE) on its layer that holds it (usize::MAX: none)
    scope: usize,
    /// it is a scoped record itself (it holds only what its scope says)
    scoped: bool,
    /// its record's widget id, the parent it names (Sem::parent; 0: none)
    wid: u32,
    want: u64,
}

fn centre(r: &[i32; 4]) -> (i32, i32) {
    (r[0] + r[2] / 2, r[1] + r[3] / 2)
}

fn holds(r: &[i32; 4], x: i32, y: i32) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

fn area(r: &[i32; 4]) -> i64 {
    r[2].max(0) as i64 * r[3].max(0) as i64
}

fn union(a: [i32; 4], b: &[i32; 4]) -> [i32; 4] {
    if a[2] <= 0 || a[3] <= 0 {
        return *b;
    }
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

// the Sub of a record's kind (Sub::None: a basic kind)
fn sub_of(s: &Sem, named: bool) -> Sub {
    match s.kind {
        K_TABLE => Sub::Table,
        K_ROW => Sub::Row,
        K_CELL => Sub::Cell,
        K_COLHEADER => Sub::ColumnHeader,
        K_ROWHEADER => Sub::RowHeader,
        K_LANDMARK => match s.value {
            LM_BANNER => Sub::Banner,
            LM_MAIN => Sub::Main,
            LM_NAVIGATION => Sub::Navigation,
            LM_COMPLEMENTARY => Sub::Complementary,
            LM_CONTENTINFO => Sub::ContentInfo,
            LM_SEARCH => Sub::Search,
            LM_FORM => Sub::Form,
            _ => Sub::Region,
        },
        K_TREE => Sub::Tree,
        K_TREEITEM => Sub::TreeItem,
        K_RADIOGROUP => Sub::RadioGroup,
        K_TOOLBAR => Sub::Toolbar,
        K_TOOLTIP if named => Sub::Tooltip,
        K_SEPARATOR => Sub::Separator,
        K_ALERT => Sub::Alert,
        K_STATUS => Sub::Status,
        K_LOG => Sub::Log,
        K_METER => Sub::Meter,
        K_SPIN => Sub::SpinButton,
        K_SCROLLBAR => Sub::ScrollBar,
        K_ARTICLE => Sub::Article,
        K_FIGURE => Sub::Figure,
        _ => Sub::None,
    }
}

// the role of a record and its Sub (None: left out)
fn role_of(s: &Sem) -> Option<(Role, Sub)> {
    let label = s.label.as_deref().unwrap_or("");
    let named = !label.is_empty() || s.ax.as_ref().and_then(|a| a.name.as_deref()).is_some_and(|n| !n.is_empty());
    let sub = sub_of(s, named);
    if let Some(r) = sub.role() {
        return Some((r, sub));
    }
    if s.kind >= K_KIND_COUNT {
        return None; // (a tooltip without a text)
    }
    Some((role_basic(s, named)?, Sub::None))
}

// the role of a record of a basic kind (None: left out)
fn role_basic(s: &Sem, named: bool) -> Option<Role> {
    Some(match s.kind {
        K_TEXT if named => Role::Label,
        K_HEADING if named => Role::Heading,
        K_BUTTON | K_SWATCH => Role::Button,
        K_CHECKBOX => Role::CheckBox,
        K_RADIO => Role::Radio,
        K_SWITCH => Role::Switch,
        K_SLIDER => Role::Slider,
        K_PROGRESS => Role::Progress,
        K_TEXTBOX => Role::TextBox,
        K_MENU => Role::Menu,
        K_MENUITEM => Role::MenuItem,
        K_LIST => Role::List,
        K_LISTITEM => Role::ListItem,
        K_TAB => Role::Tab,
        K_CANVAS => Role::Canvas,
        K_ICON | K_IMAGE => Role::Image,
        K_COMBO => Role::Combo,
        K_DIALOG => Role::Dialog,
        K_LINK => Role::Link,
        // a titled box (notifications, info bars); the tab strip's box (max = number of tabs)
        K_BOX if named => Role::Group,
        K_BOX if s.max > 0 => Role::TabList,
        _ => return None,
    })
}

// roles that need a name from the text before them when their label is no name
fn borrows_name(role: Role) -> bool {
    matches!(role, Role::TextBox | Role::Combo | Role::Slider | Role::Progress | Role::List)
}

fn focusable(role: Role) -> bool {
    matches!(
        role,
        Role::Button | Role::CheckBox | Role::Radio | Role::Switch | Role::Slider | Role::TextBox | Role::Link | Role::List
            | Role::ListItem | Role::Menu | Role::MenuItem | Role::TabList | Role::Tab | Role::Combo
    )
}

// the characters before byte offset b of s
fn chars_before(s: &str, b: i32) -> i32 {
    let mut b = (b.max(0) as usize).min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s[..b].chars().count() as i32
}

impl Ui {
    /// The accessibility tree of the last pass as assistive technology gets it (screen
    /// readers through AT-SPI2 / UI Automation): every shown window with its nodes (roles,
    /// names, states, values, rectangles), built from the semantic records (see sem()).
    pub fn a11y_tree(&self) -> Snapshot {
        let mut snap = Snapshot { app: String::from(self.desc.title), wins: Vec::new() };
        for i in 0..WIN_MAX as u32 {
            if i == 0 || self.uiw_drawable(i) {
                snap.wins.push(self.a11y_win(i));
            }
        }
        snap
    }

    // the tree of window i
    fn a11y_win(&self, i: u32) -> WinTree {
        let recs: Vec<&Sem> = self.sems.iter().filter(|s| s.win as u32 == i).collect();
        let mut cands: Vec<Cand> = Vec::new();
        let mut occ: BTreeMap<u32, u32> = BTreeMap::new();
        // the scoped records open at a record: (the last record they hold, their candidate)
        let mut scopes: Vec<(usize, usize)> = Vec::new();
        // the id64s given so far; the left out records with an id64: their own parent
        let mut rich_ids: BTreeMap<u64, usize> = BTreeMap::new();
        let mut skip: BTreeMap<u64, u64> = BTreeMap::new();
        for (k, s) in recs.iter().enumerate() {
            while scopes.last().is_some_and(|e| e.0 < k) {
                scopes.pop();
            }
            let n = occ.entry(s.id).or_insert(0);
            *n += 1;
            let (rid, want) = s.ax.as_deref().map_or((0, 0), |a| (a.id & ID_MASK, a.parent));
            // (a second record of the same id64 is keyed as a widget's)
            let rich = rid != 0 && !rich_ids.contains_key(&rid);
            let key = if rich { RICH | rid } else { (*n as u64) << 32 | s.id as u64 };
            // records without area are left out (those of a rich tree only with a negative size)
            let empty = if rich { s.rect[2] < 0 || s.rect[3] < 0 } else { s.rect[2] <= 0 || s.rect[3] <= 0 };
            let Some((role, sub)) = role_of(s).filter(|_| !empty) else {
                if rich {
                    skip.insert(rid, want);
                }
                continue;
            };
            let nd = self.a11y_node(s, role, sub, key, &recs, k);
            let container = (matches!(role, Role::List | Role::Dialog | Role::Group | Role::TabList) && sub != Sub::Separator)
                || matches!(sub, Sub::Cell | Sub::ColumnHeader | Sub::RowHeader | Sub::Alert | Sub::Status | Sub::Log);
            let pos = cands.len();
            let scope = scopes.iter().rev().find(|e| cands[e.1].layer == s.layer).map_or(usize::MAX, |e| e.1);
            let scoped = container && s.min == SEM_SCOPE;
            if rich {
                rich_ids.insert(rid, pos);
            }
            cands.push(Cand { nd, layer: s.layer, pos, container, parent: usize::MAX, fixed: false, scope, scoped, wid: s.id, want });
            if scoped && s.max > 0 {
                scopes.push((k + s.max as usize, pos));
            }
        }
        self.a11y_groups(&mut cands);
        Self::a11y_parents(&mut cands, &rich_ids, &skip);
        // parents: the smallest container on the same layer around the centre
        let n = cands.len();
        let conts: Vec<usize> = (0..n).filter(|&p| cands[p].container).collect();
        for c in 0..n {
            if cands[c].fixed {
                continue;
            }
            let (x, y) = centre(&cands[c].nd.rect);
            let a = area(&cands[c].nd.rect);
            let mut best = usize::MAX;
            let mut best_area = i64::MAX;
            let scope = cands[c].scope;
            for &p in &conts {
                let q = &cands[p];
                if p == c || q.layer != cands[c].layer || !holds(&q.nd.rect, x, y) {
                    continue;
                }
                // inside a scoped record only what it holds (or itself) can be the parent; a
                // scoped record holds nothing by its place (overlapping windows are siblings)
                if (q.scope != scope || q.scoped) && p != scope {
                    continue;
                }
                // tab strips hold tabs only
                if q.nd.role == Role::TabList && cands[c].nd.role != Role::Tab {
                    continue;
                }
                // a record out of view is not where its rectangle is; a record with an id64 (a
                // page's) holds by its place only others with one (never the app's own widgets)
                let rich = |nd: &Node| nd.key & SYN == 0 && nd.key & RICH != 0;
                if q.nd.state & ax::S_OFFSCREEN != 0 || (rich(&q.nd) && !rich(&cands[c].nd)) {
                    continue;
                }
                // never inside something smaller (equal: the earlier record is the outer one)
                let qa = area(&q.nd.rect);
                if qa < a || (qa == a && p > c) {
                    continue;
                }
                if qa < best_area {
                    best = p;
                    best_area = qa;
                }
            }
            // nothing inside a scoped record around it: the scoped record itself
            if best == usize::MAX && scope != usize::MAX {
                best = scope;
            }
            cands[c].parent = best;
        }
        // a loop (named parents against the rectangles): its record hangs at the top
        for c in 0..n {
            let mut p = cands[c].parent;
            let mut steps = 0;
            while p != usize::MAX && p != c && steps < n {
                p = cands[p].parent;
                steps += 1;
            }
            if p == c {
                cands[c].parent = usize::MAX;
            }
        }
        // tab strips without tabs are no nodes
        let mut used = alloc::vec![false; n];
        for c in &cands {
            if c.parent < n {
                used[c.parent] = true;
            }
        }
        let keep: Vec<bool> = (0..n).map(|p| cands[p].nd.role != Role::TabList || used[p]).collect();
        // the order: parents before their children, siblings in record order
        let mut kids: Vec<Vec<usize>> = alloc::vec![Vec::new(); n + 1];
        for c in 0..n {
            if !keep[c] {
                continue;
            }
            let p = cands[c].parent;
            let p = if p != usize::MAX && keep[p] { p } else { n };
            kids[p].push(c);
        }
        for k in kids.iter_mut() {
            k.sort_by_key(|&c| cands[c].pos);
        }
        let mut order: Vec<(usize, u32)> = Vec::new(); // (candidate, parent node index)
        let mut stack: Vec<(usize, u32)> = kids[n].iter().rev().map(|&c| (c, NONE)).collect();
        let mut index_of = alloc::vec![NONE; n];
        while let Some((c, parent)) = stack.pop() {
            index_of[c] = order.len() as u32;
            order.push((c, parent));
            for &k in kids[c].iter().rev() {
                stack.push((k, index_of[c]));
            }
        }
        let mut nodes: Vec<Node> = Vec::with_capacity(order.len());
        for &(c, parent) in &order {
            let mut nd = core::mem::take(&mut cands[c].nd);
            nd.parent = parent;
            nodes.push(nd);
        }
        // the focus: the last focused node; a list's / tab strip's goes on to its selected child
        let mut focus = nodes.iter().rposition(|nd| nd.state & ax::S_FOCUSED != 0).map_or(NONE, |k| k as u32);
        if focus != NONE && matches!(nodes[focus as usize].role, Role::List | Role::TabList) {
            if let Some(k) = (0..nodes.len()).find(|&k| nodes[k].parent == focus && nodes[k].state & ax::S_SELECTED != 0) {
                nodes[focus as usize].state &= !ax::S_FOCUSED;
                nodes[k].state |= ax::S_FOCUSED;
                focus = k as u32;
            }
        }
        let rec = &self.m.win.recs[i as usize];
        let fb = self.win_fb(i);
        let active = if self.headless { i == self.m.win.active } else { self.uiw_plat_state(i) & plat::WST_ACTIVE != 0 };
        let mut w = WinTree {
            win: i as u8,
            pw: if self.headless { -1 } else { self.uiw_pw(i) },
            title: String::from(if i == 0 { self.title() } else { self.uiw_title(i) }),
            dialog: rec.flags & WF_DIALOG != 0,
            active,
            origin: [rec.sx, rec.sy],
            size: [fb.w, fb.h],
            nodes,
            focus,
            ..WinTree::default()
        };
        w.link();
        w
    }

    // the node of record s (record k of the window's records)
    fn a11y_node(&self, s: &Sem, role: Role, sub: Sub, key: u64, recs: &[&Sem], k: usize) -> Node {
        let label = s.label.as_deref().unwrap_or("");
        let mut nd = Node { key, role, sub, rect: s.rect, caret: -1, parent: NONE, ..Node::default() };
        let rich = key & SYN == 0 && key & RICH != 0;
        let a = s.ax.as_deref();
        let st = s.state;
        if st & SS_FOCUSED != 0 {
            nd.state |= ax::S_FOCUSED;
        }
        if st & SS_DISABLED != 0 {
            nd.state |= ax::S_DISABLED;
        }
        if st & SS_SELECTED != 0 && role != Role::Combo {
            nd.state |= ax::S_SELECTED;
        }
        if st & SS_OPEN != 0 && role != Role::Dialog {
            nd.state |= ax::S_EXPANDED;
        }
        if st & SS_CHECKED != 0 {
            nd.state |= match role {
                Role::Button => ax::S_PRESSED,
                Role::CheckBox if s.value == 2 => ax::S_MIXED,
                _ => ax::S_CHECKED,
            };
        }
        if focusable(role) {
            nd.state |= ax::S_FOCUSABLE;
        }
        if role == Role::TextBox {
            // the kind of box (edit.rs: Sem::min)
            let kind = (s.min as u32) << 24;
            for (tb, st) in [(TB_MULTI, ax::S_MULTILINE), (TB_PASSWORD, ax::S_PROTECTED), (TB_READONLY, ax::S_READONLY)] {
                if kind & tb != 0 {
                    nd.state |= st;
                }
            }
        }
        match role {
            Role::TextBox | Role::Combo => nd.text = String::from(label),
            _ if s.kind == K_SWATCH => nd.name = alloc::format!("#{:06X}", s.value as u32 & 0xFF_FFFF),
            _ => nd.name = String::from(label),
        }
        if role == Role::TextBox && st & SS_FOCUSED != 0 {
            // (a rich box: the selection it gives in characters; a built-in one: bytes)
            match a.and_then(|a| a.sel) {
                Some([anchor, caret]) => {
                    nd.caret = caret as i32;
                    nd.anchor = anchor as i32;
                }
                None => {
                    nd.caret = chars_before(label, s.value);
                    nd.anchor = chars_before(label, s.max);
                }
            }
            // the caret's rectangle when the box drew it in this pass (magnifiers)
            let (pass, r) = self.m.a11y.caret[(s.win as usize) & (WIN_MAX - 1)];
            if pass == self.frame && r[3] > 0 {
                nd.caret_rect = r;
            }
        }
        let named = a.and_then(|a| a.name.as_deref());
        if borrows_name(role) && !rich && named.is_none() && (nd.name.is_empty() || matches!(role, Role::TextBox | Role::Combo)) {
            nd.name = Self::a11y_label_before(recs, k).unwrap_or_default();
        }
        if let Some(n) = named {
            nd.name = String::from(n);
        }
        match role {
            Role::Slider => {
                nd.value = s.value as f64;
                nd.min = s.min.min(s.max) as f64;
                nd.max = s.max as f64;
                nd.step = 1.0;
            }
            Role::Progress if sub == Sub::Meter => {
                nd.value = s.value as f64;
                nd.min = s.min.min(s.max) as f64;
                nd.max = s.max as f64;
            }
            Role::Progress => {
                nd.value = if s.value < 0 { -1.0 } else { s.value as f64 };
                nd.max = s.max as f64;
            }
            Role::Heading => nd.value = s.value.clamp(1, 9) as f64,
            _ => {}
        }
        if let Some(a) = a {
            Self::a11y_rich_node(&mut nd, a);
        }
        // live regions: as the record says, else as its kind says
        nd.live = match a.map_or(0, |a| a.live) {
            LIVE_POLITE => ax::LIVE_POLITE,
            LIVE_ASSERTIVE => ax::LIVE_ASSERTIVE,
            LIVE_OFF => 0,
            _ => match s.kind {
                K_ALERT => ax::LIVE_ASSERTIVE,
                K_STATUS | K_LOG => ax::LIVE_POLITE,
                _ => 0,
            },
        };
        let readonly = rich && nd.state & ax::S_READONLY != 0;
        if nd.state & ax::S_DISABLED == 0 {
            if matches!(
                role,
                Role::Button | Role::Link | Role::CheckBox | Role::Radio | Role::Switch | Role::Tab | Role::ListItem | Role::Menu
                    | Role::MenuItem | Role::Combo
            ) || (rich && a.is_some_and(|a| a.states & AS_CLICKABLE != 0))
            {
                nd.actions |= ax::A_CLICK;
            }
            if nd.state & ax::S_FOCUSABLE != 0 {
                nd.actions |= ax::A_FOCUS;
            }
            if role == Role::Slider && !readonly {
                nd.actions |= ax::A_VALUE;
            }
            if role == Role::TextBox && !readonly {
                nd.actions |= ax::A_TEXT;
            }
            // the further actions: only for records whose requests go to the app
            if rich {
                if nd.state & (ax::S_COLLAPSED | ax::S_EXPANDED) != 0 && role != Role::Dialog {
                    nd.actions |= ax::A_EXPAND;
                }
                if role == Role::Slider && !readonly {
                    nd.actions |= ax::A_STEP;
                }
                if matches!(role, Role::ListItem | Role::Tab) || nd.state & ax::S_SELECTABLE != 0 {
                    nd.actions |= ax::A_SELECT;
                }
            }
        }
        if rich {
            nd.actions |= ax::A_SCROLL; // (also a disabled one)
        }
        nd
    }

    // the further fields of a record's node from its SemAx: states, texts, places, a range with
    // fractions
    fn a11y_rich_node(nd: &mut Node, a: &SemAx) {
        let map = [
            (AS_REQUIRED, ax::S_REQUIRED),
            (AS_INVALID, ax::S_INVALID),
            (AS_BUSY, ax::S_BUSY),
            (AS_HASPOPUP, ax::S_HASPOPUP),
            (AS_VISITED, ax::S_VISITED),
            (AS_MODAL, ax::S_MODAL),
            (AS_MULTISELECT, ax::S_MULTISELECT),
            (AS_OFFSCREEN, ax::S_OFFSCREEN),
            (AS_FOCUSABLE, ax::S_FOCUSABLE),
            (AS_SELECTABLE, ax::S_SELECTABLE),
            (AS_READONLY, ax::S_READONLY),
            (AS_MULTILINE, ax::S_MULTILINE),
            (AS_PASSWORD, ax::S_PROTECTED),
        ];
        for (f, bit) in map {
            if a.states & f != 0 {
                nd.state |= bit;
            }
        }
        if a.states & AS_COLLAPSED != 0 && nd.state & ax::S_EXPANDED == 0 {
            nd.state |= ax::S_COLLAPSED;
        }
        if let Some(d) = &a.desc {
            nd.desc = d.clone();
        }
        if let Some(u) = &a.url {
            nd.url = u.clone();
        }
        nd.set = a.set;
        if nd.role == Role::Heading {
            if a.level > 0 {
                nd.value = a.level.clamp(1, 9) as f64;
            }
        } else {
            nd.item_level = a.level;
        }
        if a.cell[2] > 0 && a.cell[3] > 0 {
            nd.cell = a.cell;
        }
        nd.grid = a.grid;
        if matches!(nd.role, Role::Slider | Role::Progress) {
            if let Some([v, lo, hi, step]) = a.range {
                nd.value = v;
                nd.min = lo.min(hi);
                nd.max = hi;
                nd.step = step.max(0.0);
            }
            if let Some(t) = &a.value_text {
                nd.text = t.clone();
            }
        }
    }

    // the label of the text (or heading) record right before record k (records without a
    // label in between, e.g. boxes, do not count): on its layer, to the left of it in the same
    // row, or above it with the same left edge and at most a line apart
    fn a11y_label_before(recs: &[&Sem], k: usize) -> Option<String> {
        let me = recs[k];
        let r = me.rect;
        let t = recs[..k].iter().rev().find(|t| t.kind != K_BOX || t.label.is_some())?;
        if !matches!(t.kind, K_TEXT | K_HEADING) || t.layer != me.layer {
            return None;
        }
        let q = t.rect;
        let rows = q[1] < r[1] + r[3] && r[1] < q[1] + q[3];
        let left = rows && q[0] + q[2] <= r[0] + 4;
        let above = (q[0] - r[0]).abs() <= 4 && q[1] + q[3] <= r[1] + 4 && r[1] - (q[1] + q[3]) <= r[3].max(q[3]);
        if left || above {
            return t.label.clone().filter(|l| !l.is_empty());
        }
        None
    }

    // the parents the records name (Sem::parent): the record with that id64 (rich_ids), else the
    // first with that widget id; through left out records with an id64 (skip) to theirs; none
    // found (or itself): its rectangle decides
    fn a11y_parents(cands: &mut [Cand], rich_ids: &BTreeMap<u64, usize>, skip: &BTreeMap<u64, u64>) {
        let mut wids: BTreeMap<u32, usize> = BTreeMap::new();
        for (c, cand) in cands.iter().enumerate() {
            if cand.wid != 0 {
                wids.entry(cand.wid).or_insert(c);
            }
        }
        for c in 0..cands.len() {
            let mut want = cands[c].want;
            let mut found = None;
            for _ in 0..64 {
                if want == 0 {
                    break;
                }
                if let Some(&p) = rich_ids.get(&(want & ID_MASK)) {
                    found = Some(p);
                    break;
                }
                if let Some(&p) = u32::try_from(want).ok().and_then(|w| wids.get(&w)) {
                    found = Some(p);
                    break;
                }
                want = skip.get(&(want & ID_MASK)).copied().unwrap_or(0);
            }
            if let Some(p) = found.filter(|&p| p != c) {
                cands[c].parent = p;
                cands[c].fixed = true;
            }
        }
    }

    // the synthetic containers: menu bar entries next to each other get a menu bar; the rows
    // of a popup (menu items, the rows of a drop-down) a menu / list below the record that is
    // open (in record order: the n-th popup group belongs to the n-th open record)
    fn a11y_groups(&self, cands: &mut Vec<Cand>) {
        let n = cands.len();
        let mut groups: Vec<(u64, Role, Vec<usize>)> = Vec::new();
        let mut k = 0;
        while k < n {
            let r = cands[k].nd.role;
            let popup = cands[k].layer as u32 >= L_POPUP && matches!(r, Role::MenuItem | Role::ListItem);
            if r != Role::Menu && !popup {
                k += 1;
                continue;
            }
            let mut j = k + 1;
            while j < n && cands[j].nd.role == r && cands[j].layer == cands[k].layer {
                j += 1;
            }
            let id = cands[k].nd.key & 0xFFFF_FFFF;
            let (kind, role) = if r == Role::Menu {
                (SYN_MENUBAR, Role::MenuBar)
            } else if r == Role::MenuItem {
                (SYN_POPUP, Role::Menu)
            } else {
                (SYN_POPUP, Role::List)
            };
            groups.push((SYN | kind | id, role, (k..j).collect()));
            k = j;
        }
        // the records that are open, in order (they opened the popups)
        let opened: Vec<usize> =
            (0..n).filter(|&c| cands[c].nd.state & ax::S_EXPANDED != 0 && !matches!(cands[c].nd.role, Role::Dialog | Role::Group)).collect();
        let mut popups = 0;
        for (key, role, members) in groups {
            let at = cands.len();
            let mut rect = [0; 4];
            for &m in &members {
                rect = union(rect, &cands[m].nd.rect);
                cands[m].parent = at;
                cands[m].fixed = true;
            }
            let layer = cands[members[0]].layer;
            let mut g = Cand {
                nd: Node { key, role, rect, caret: -1, parent: NONE, ..Node::default() },
                layer,
                pos: members[0],
                container: false,
                parent: usize::MAX,
                fixed: false,
                scope: cands[members[0]].scope,
                scoped: false,
                wid: 0,
                want: 0,
            };
            if role != Role::MenuBar {
                g.nd.state = ax::S_EXPANDED;
                g.fixed = true;
                if let Some(&o) = opened.get(popups) {
                    g.parent = o;
                }
                popups += 1;
            }
            cands.push(g);
        }
    }

    // ------------------------------------------------------------------ runtime hooks

    /// (runtime) after the passes: a click no widget took becomes a pointer click; the tree
    /// is published while assistive technology listens (the first window frame starts the
    /// platform bridge)
    pub(crate) fn a11y_frame(&mut self) {
        if let Some(point) = self.a11y_click_record().and_then(|rec| self.a11y_aim(&rec)) {
            let win = self.m.a11y.click.map_or(0, |c| c.0);
            let pw = if self.headless { win as i32 } else { self.uiw_pw(win as u32) };
            if pw >= 0 {
                let ev = plat::Event { win: pw as u8, x: point.0, y: point.1, btn: plat::BTN_LEFT, ..Default::default() };
                for typ in [plat::E_MOVE, plat::E_DOWN, plat::E_UP] {
                    plat::evq_push(plat::Event { typ, ..ev });
                }
                plat::wake();
            }
        }
        self.m.a11y.click = None;
        self.m.a11y.value = None;
        self.m.a11y.text = None;
        self.a11y_select();
        if !self.m.a11y.started {
            self.m.a11y.started = true;
            // the role names the bridges give, in the UI language (plat cannot ask ui)
            plat::a11y::set_translator(crate::i18n::tr);
            if !self.headless {
                plat::a11y::start(self.desc.title);
            }
        }
        let said = core::mem::take(&mut self.m.a11y.announce);
        if self.headless {
            // (no bridge: kept for the scripts)
            let log = &mut self.m.a11y.said;
            log.extend(said.into_iter().map(|e| e.1));
            let over = log.len().saturating_sub(ANNOUNCE_MAX);
            log.drain(..over);
            return;
        }
        if plat::a11y::active() {
            for (w, text, p) in said {
                plat::a11y::announce(w, &text, p);
            }
            plat::a11y::publish(self.a11y_tree());
        }
    }

    // the record of the click waiting for a widget, as of the last pass
    fn a11y_click_record(&self) -> Option<Sem> {
        let (win, id, nth) = self.m.a11y.click?;
        self.sems.iter().filter(|s| s.win == win && s.id == id).nth(nth - 1).cloned()
    }

    /// (runtime, after the passes) a click no widget took, its record's middle not on it: the
    /// scroll views bring the record into view in further passes -> those are needed
    pub(crate) fn a11y_reveal(&mut self) -> bool {
        let Some(rec) = self.a11y_click_record().filter(|r| r.id != 0) else { return false };
        match self.agent_aim(&rec, false) {
            Err(miss) if miss.reason != "popup" => {
                self.agent.reveal = Some((rec.win, rec.id));
                true
            }
            _ => false,
        }
    }

    // Where a click no widget took is pressed: where a click from outside would be (agent_aim),
    // or in the middle of a record without a hit box of its own when the widget on top there
    // draws it: that box lies inside the record (a link of a document, an app's own widget with
    // another id), or holds it and is no control of its own (a row of a popup's list).  Else
    // none: the middle lies on something else, and a press would go there.
    fn a11y_aim(&self, rec: &Sem) -> Option<(i32, i32)> {
        if rec.rect[2] <= 0 || rec.rect[3] <= 0 {
            return None;
        }
        let miss = match self.agent_aim(rec, true) {
            Ok(point) => return Some(point),
            Err(miss) => miss,
        };
        let (x, y) = miss.point.filter(|_| miss.reason == "no_input" && miss.by != 0)?;
        let p = self.uiw_parts(rec.win as u32)?;
        // hit boxes are logical
        let w = p.rt.fb.w;
        let (lx, r) = if self.rtl { (w - 1 - x, [w - rec.rect[0] - rec.rect[2], rec.rect[1], rec.rect[2], rec.rect[3]]) } else { (x, rec.rect) };
        let h = p.ctx.hits[p.ctx.hit_cur ^ 1].iter().rev().find(|h| h.id == miss.by && h.layer == rec.layer && lx >= h.x0 && lx < h.x1 && y >= h.y0 && y < h.y1)?;
        let inside = h.x0 >= r[0] && h.y0 >= r[1] && h.x1 <= r[0] + r[2] && h.y1 <= r[1] + r[3];
        let holds = h.x0 <= r[0] && h.y0 <= r[1] && h.x1 >= r[0] + r[2] && h.y1 >= r[1] + r[3];
        let control = self.sems.iter().any(|s| s.id == miss.by && s.win == rec.win && !matches!(s.kind, K_BOX | K_LIST | K_CANVAS));
        (inside || (holds && !control)).then_some((x, y))
    }

    /// (runtime, E_WAKE) take the actions of the assistive technology: a node of the last
    /// pass that is enabled and not behind a modal dialog; the next passes carry them out
    pub(crate) fn a11y_poll(&mut self) {
        // the further actions: offered by records with an id64 only, for the app
        for o in plat::a11y::take_ops() {
            let action = match o.op {
                Op::ScrollIntoView => A11yAction::ScrollIntoView,
                Op::Expand => A11yAction::Expand,
                Op::Collapse => A11yAction::Collapse,
                Op::Increment => A11yAction::Increment,
                Op::Decrement => A11yAction::Decrement,
                Op::SelectItem => A11yAction::Select,
                Op::AddToSelection => A11yAction::AddToSelection,
                Op::RemoveFromSelection => A11yAction::RemoveFromSelection,
                _ => continue,
            };
            self.a11y_rich_request(o.win, o.key, action);
        }
        for r in plat::a11y::take() {
            if r.key & SYN == 0 && r.key & RICH != 0 {
                let action = match r.act {
                    Act::Click => A11yAction::Click,
                    Act::Focus => A11yAction::Focus,
                    Act::Value(v) => A11yAction::SetValue(v),
                    Act::Text(t) => A11yAction::SetText(t),
                    Act::Select(a, c) => A11yAction::SelectText(a.max(0) as u32, c.max(0) as u32),
                };
                self.a11y_rich_request(r.win, r.key, action);
                continue;
            }
            let win = r.win;
            let id = r.key as u32;
            let nth = (r.key >> 32) as usize;
            if r.key & SYN != 0 || nth == 0 || win as usize >= WIN_MAX {
                continue;
            }
            let Some(s) = self.sems.iter().filter(|s| s.win == win && s.id == id).nth(nth - 1) else { continue };
            let modal = self.uiw_parts(win as u32).map_or(0, |p| p.ctx.modal_prev);
            if s.state & SS_DISABLED != 0 || (s.layer as u32) < modal || (self.m.win.inert >> win) & 1 != 0 {
                continue;
            }
            match r.act {
                Act::Click => self.m.a11y.click = Some((win, id, nth)),
                Act::Focus => {
                    if win as u32 != self.uiw_cur {
                        self.uiw_activate(win as u32);
                    }
                    self.focus(id);
                }
                Act::Value(v) => self.m.a11y.value = Some((win, id, (if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32)),
                Act::Text(t) => self.m.a11y.text = Some((win, id, t)),
                Act::Select(a, c) => {
                    // a box without the focus gets it first (a11y_frame applies the selection then)
                    self.m.a11y.select = Some((win, id, a, c));
                    if s.kind == K_TEXTBOX && !self.a11y_editing(win, id) {
                        if win as u32 != self.uiw_cur {
                            self.uiw_activate(win as u32);
                        }
                        self.focus(id);
                    } else if !self.m.a11y.text.as_ref().is_some_and(|t| t.0 == win && t.1 == id) {
                        // (a new text waiting for the box: a11y_frame selects after the passes)
                        self.a11y_select();
                    }
                }
            }
            self.need = true;
        }
    }

    // a request for the record of key (RICH | id64) of window win: queued for the app when that
    // record was in the last pass, enabled and not behind a modal dialog (scrolling into view
    // also when it is disabled)
    fn a11y_rich_request(&mut self, win: u8, key: u64, action: A11yAction) {
        if key & SYN != 0 || key & RICH == 0 || win as usize >= WIN_MAX {
            return;
        }
        let id = key & ID_MASK;
        let Some(s) = self.sems.iter().find(|s| s.win == win && s.ax.as_ref().is_some_and(|a| a.id & ID_MASK == id)) else { return };
        let modal = self.uiw_parts(win as u32).map_or(0, |p| p.ctx.modal_prev);
        let blocked = s.state & SS_DISABLED != 0 || (s.layer as u32) < modal || (self.m.win.inert >> win) & 1 != 0;
        if blocked && action != A11yAction::ScrollIntoView {
            return;
        }
        let id = s.ax.as_ref().map_or(id, |a| a.id);
        let win = self.uiw_rec_id(win as u32);
        let q = &mut self.m.a11y.reqs;
        if q.len() >= REQS_MAX {
            q.pop_front();
        }
        q.push_back(A11yRequest { win, id, action });
        self.need = true;
    }

    /// (scripts) the texts ui.announce said in headless runs since the last call
    pub(crate) fn a11y_said_take(&mut self) -> Vec<String> {
        core::mem::take(&mut self.m.a11y.said)
    }

    /// The next request of a screen reader for a record with an id64 (`Sem::id64`), oldest first:
    /// focus, click, scroll into view, expand / collapse, increment / decrement, a value, a text,
    /// a text selection, select items.  Take them in every frame and carry them out (a web view
    /// hands them to its engine); at most 64 wait.  Requests for the other records go to their
    /// widgets as before (EV_ACT, `a11y_value`, `a11y_text`).
    pub fn a11y_request_take(&mut self) -> Option<A11yRequest> {
        self.m.a11y.reqs.pop_front()
    }

    /// Have the screen reader say `text`: `LIVE_POLITE` when the person is idle, `LIVE_ASSERTIVE`
    /// at once (a web page's live region announcement, "Saved").  It goes out with this frame's
    /// tree: UI Automation's notification event, AT-SPI2's announcement, the browser's live
    /// region.  Nothing is said (or kept) while no assistive technology listens; at most 16 per
    /// frame.
    pub fn announce(&mut self, text: &str, politeness: u32) {
        let q = &mut self.m.a11y.announce;
        if !text.is_empty() && q.len() < ANNOUNCE_MAX {
            let p = if politeness == LIVE_ASSERTIVE { ax::LIVE_ASSERTIVE } else { ax::LIVE_POLITE };
            q.push((self.uiw_cur as u8, cut(text, AX_TEXT_MAX), p));
        }
    }

    /// A screen reader (or another client of the accessibility tree) listens: the library builds
    /// the tree from the records only then.  Records that only it needs -- the content of a page
    /// out of view (`AS_OFFSCREEN`) -- are best published only while this is true; it turns true
    /// with a frame of its own when a client comes.  Always false in headless runs.
    pub fn a11y_active(&self) -> bool {
        !self.headless && plat::a11y::active()
    }

    /// (behave) a pending click of the assistive technology for widget id of the live window
    pub(crate) fn a11y_click(&mut self, id: u32) -> bool {
        match self.m.a11y.click {
            Some((w, i, _)) if i == id && w as u32 == self.uiw_cur => {
                self.m.a11y.click = None;
                true
            }
            _ => false,
        }
    }

    // a selection a screen reader asked for (Act::Select): once the text box has the focus its
    // anchor and caret move there (characters of its record's text -> bytes); the next passes
    // draw it
    fn a11y_select(&mut self) {
        let Some((win, id, a, c)) = self.m.a11y.select else { return };
        let Some(text) = self.sems.iter().find(|s| s.win == win && s.id == id && s.kind == K_TEXTBOX).and_then(|s| s.label.clone()) else {
            self.m.a11y.select = None; // the box is gone
            return;
        };
        if !self.a11y_editing(win, id) {
            return; // the focus arrives with the next pass
        }
        self.m.a11y.select = None;
        let byte = |k: i32| text.char_indices().nth(k.max(0) as usize).map_or(text.len(), |(b, _)| b);
        let (a, c) = (byte(a), byte(c));
        let cur = self.uiw_cur;
        self.uiw_use(win as u32);
        if self.textbox_select(id, &text, a, c) {
            // another frame (after the passes of a11y_frame the window loop waits for events)
            self.need = true;
            if !self.headless {
                plat::wake();
            }
        }
        self.uiw_use(cur);
    }

    // text box id of window win holds the edit state (it has the focus)
    fn a11y_editing(&mut self, win: u8, id: u32) -> bool {
        let cur = self.uiw_cur;
        self.uiw_use(win as u32);
        let on = self.textbox_selection(id).is_some();
        self.uiw_use(cur);
        on
    }

    /// (edit.rs ime_pos) the focused text box of the live window drew its caret at x, y (window
    /// px) with height h (h <= 0: no caret any more): the accessibility tree reports it, screen
    /// magnifiers follow it
    pub(crate) fn a11y_caret(&mut self, x: i32, y: i32, h: i32) {
        let w = (self.uiw_cur as usize) & (WIN_MAX - 1);
        self.m.a11y.caret[w] = if h > 0 { (self.frame, [x, y, self.hair().max(1), h]) } else { (0, [0; 4]) };
    }

    /// A value a screen reader set for slider-like widget `id` (its record's value scale),
    /// taken once: the built-in sliders apply it themselves; a widget of its own applies it
    /// like keyboard input (clamped) in the pass it is built.
    pub fn a11y_value(&mut self, id: u32) -> Option<i32> {
        match self.m.a11y.value {
            Some((w, i, v)) if i == id && w as u32 == self.uiw_cur => {
                self.m.a11y.value = None;
                Some(v)
            }
            _ => None,
        }
    }

    /// A text a screen reader set for text widget `id`, taken once (the built-in text boxes
    /// apply it themselves).
    pub fn a11y_text(&mut self, id: u32) -> Option<String> {
        match &self.m.a11y.text {
            Some((w, i, _)) if *i == id && *w as u32 == self.uiw_cur => self.m.a11y.text.take().map(|t| t.2),
            _ => None,
        }
    }
}

// s cut to at most max bytes (at a character)
fn cut(s: &str, max: usize) -> String {
    let mut b = s.len().min(max);
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    String::from(&s[..b])
}

// a text of a record's SemAx: None when empty, cut to AX_TEXT_MAX
fn ax_text(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| cut(s, AX_TEXT_MAX))
}

/// The setters of a record's further accessibility data (`Sem::ax`, section 19 of the guide):
/// each returns the record, so they chain after `sem` / `sem_rect`.  A web page's tree:
///
/// ```ignore
/// ui.sem_rect(K_CELL, 0, r, Some("12,50 €"), 0).id64(node).parent(row).cell(2, 1);
/// ```
impl Sem {
    fn ax_mut(&mut self) -> &mut SemAx {
        self.ax.get_or_insert_with(|| Box::new(SemAx::default()))
    }

    /// Its own 64-bit id (below 2^62; a web page's node id).  The node keeps it from frame to
    /// frame, other records name it as their parent, and every request of a screen reader for it
    /// comes back to the app (`ui.a11y_request_take`) instead of going to a widget.
    pub fn id64(&mut self, id: u64) -> &mut Sem {
        self.ax_mut().id = id;
        self
    }

    /// The record it hangs under: the one with this id64 (else this widget id) in the same frame
    /// and window -- the tree follows it, not the rectangles.  A parent that is left out (a box
    /// without a label) passes it on to its own parent; an unknown one leaves it to its rectangle.
    pub fn parent(&mut self, id: u64) -> &mut Sem {
        self.ax_mut().parent = id;
        self
    }

    /// Its name when the label is something else: a text box's or drop-down's label is its text,
    /// so `.name("Suche")` names it.
    pub fn name(&mut self, name: &str) -> &mut Sem {
        self.ax_mut().name = ax_text(name);
        self
    }

    /// A description beside the name (a hint; a page's aria-describedby).
    pub fn desc(&mut self, text: &str) -> &mut Sem {
        self.ax_mut().desc = ax_text(text);
        self
    }

    /// A link's address.
    pub fn url(&mut self, url: &str) -> &mut Sem {
        self.ax_mut().url = ax_text(url);
        self
    }

    /// The text a screen reader says for a slider's, meter's or spin button's value ("Montag").
    pub fn value_text(&mut self, text: &str) -> &mut Sem {
        self.ax_mut().value_text = ax_text(text);
        self
    }

    /// Further states (`AS_*`: required, invalid, busy, has a popup, visited, modal,
    /// multi-select, collapsed, off screen, focusable, clickable, selectable, read-only,
    /// multi-line, password), added to those it has.
    pub fn states(&mut self, states: u32) -> &mut Sem {
        self.ax_mut().states |= states;
        self
    }

    /// Its position in its set (1-based) and the set's size: list and tree items, tabs, rows.
    pub fn set_pos(&mut self, pos: u32, size: u32) -> &mut Sem {
        self.ax_mut().set = [pos, size];
        self
    }

    /// The level of a heading (1 ..= 9, in place of the value), a tree item, list item or row
    /// (1 the top).
    pub fn level(&mut self, level: u32) -> &mut Sem {
        self.ax_mut().level = level;
        self
    }

    /// A cell's or header's place in its table: row, column (0-based).
    pub fn cell(&mut self, row: u32, col: u32) -> &mut Sem {
        let a = self.ax_mut();
        a.cell = [row, col, a.cell[2].max(1), a.cell[3].max(1)];
        self
    }

    /// The rows and columns a cell spans (1, 1 without).
    pub fn span(&mut self, rows: u32, cols: u32) -> &mut Sem {
        let a = self.ax_mut();
        a.cell[2] = rows.max(1);
        a.cell[3] = cols.max(1);
        self
    }

    /// A table's number of rows and columns (without: counted from its cells).
    pub fn grid(&mut self, rows: u32, cols: u32) -> &mut Sem {
        self.ax_mut().grid = [rows, cols];
        self
    }

    /// How it speaks as a live region: `LIVE_POLITE`, `LIVE_ASSERTIVE`, `LIVE_OFF` (0: as its kind
    /// says -- K_ALERT assertive, K_STATUS and K_LOG polite).
    pub fn live(&mut self, live: u32) -> &mut Sem {
        self.ax_mut().live = live;
        self
    }

    /// A value with fractions in place of `value` / `min` / `max` (sliders, meters, spin buttons,
    /// scroll bars, progress bars; a value below min: unknown progress), and the step of a
    /// screen reader's increment.
    pub fn range(&mut self, value: f64, min: f64, max: f64, step: f64) -> &mut Sem {
        self.ax_mut().range = Some([value, min, max, step]);
        self
    }

    /// A focused text box's selection in characters: anchor, caret (equal: the caret alone) -- a
    /// page's own field; the built-in text boxes report theirs themselves.
    pub fn text_sel(&mut self, anchor: u32, caret: u32) -> &mut Sem {
        self.ax_mut().sel = Some([anchor, caret]);
        self
    }
}
