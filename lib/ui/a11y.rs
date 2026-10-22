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
// click in the middle of the record.
//
// The tree: every record of a window becomes a node in the record order -- roles from the
// kinds (K_*), states from SS_* (a check box with value 2 is mixed), a heading's level from its
// value (K_HEADING: Node::value, clamped to 1 ..= 9).  Left out: title bars, empty texts,
// records without area, boxes without a label (a box with tab records inside is the tab
// strip).  Containers are lists, dialogs, titled boxes and tab strips: a node's parent is the
// smallest one on its layer whose rectangle holds the node's centre.  Menu bar entries next to
// each other get a menu bar; the rows of a popup (menus, drop-downs) a menu or list that hangs
// below the open record that opened it.  Names: the label; text boxes, drop-downs (their label
// is the text / the choice), sliders, progress bars and lists without one take the text or
// heading record right before them (to the left, or above and aligned).  The focus of a list
// or tab strip is passed on to its selected row / tab.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use plat::a11y::{self as ax, Act, Node, Role, Snapshot, WinTree, NONE};

use super::consts::*;
use super::{Sem, Ui};

// synthetic containers: keys outside the widget ids (bit 63, kind in bits 40..)
const SYN: u64 = 1 << 63;
const SYN_MENUBAR: u64 = 1 << 40;
const SYN_POPUP: u64 = 2 << 40;

/// the requests waiting for the widgets (shared by all windows)
#[derive(Default)]
pub(crate) struct S {
    started: bool,
    /// a click for (window, id) and the record's rectangle (the fallback click)
    click: Option<(u8, u32, [i32; 4])>,
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

// the role of a record (None: left out)
fn role_of(s: &Sem) -> Option<Role> {
    let label = s.label.as_deref().unwrap_or("");
    Some(match s.kind {
        K_TEXT if !label.is_empty() => Role::Label,
        K_HEADING if !label.is_empty() => Role::Heading,
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
        K_BOX if !label.is_empty() => Role::Group,
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
        for (k, s) in recs.iter().enumerate() {
            let n = occ.entry(s.id).or_insert(0);
            *n += 1;
            let key = (*n as u64) << 32 | s.id as u64;
            let Some(role) = role_of(s) else { continue };
            if s.rect[2] <= 0 || s.rect[3] <= 0 {
                continue;
            }
            let nd = self.a11y_node(s, role, key, &recs, k);
            let container = matches!(role, Role::List | Role::Dialog | Role::Group | Role::TabList);
            let pos = cands.len();
            cands.push(Cand { nd, layer: s.layer, pos, container, parent: usize::MAX, fixed: false });
        }
        self.a11y_groups(&mut cands);
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
            for &p in &conts {
                let q = &cands[p];
                if p == c || q.layer != cands[c].layer || !holds(&q.nd.rect, x, y) {
                    continue;
                }
                // tab strips hold tabs only
                if q.nd.role == Role::TabList && cands[c].nd.role != Role::Tab {
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
            cands[c].parent = best;
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
    fn a11y_node(&self, s: &Sem, role: Role, key: u64, recs: &[&Sem], k: usize) -> Node {
        let label = s.label.as_deref().unwrap_or("");
        let mut nd = Node { key, role, rect: s.rect, caret: -1, parent: NONE, ..Node::default() };
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
            nd.caret = chars_before(label, s.value);
            nd.anchor = chars_before(label, s.max);
            // the caret's rectangle when the box drew it in this pass (magnifiers)
            let (pass, r) = self.m.a11y.caret[(s.win as usize) & (WIN_MAX - 1)];
            if pass == self.frame && r[3] > 0 {
                nd.caret_rect = r;
            }
        }
        if borrows_name(role) && (nd.name.is_empty() || matches!(role, Role::TextBox | Role::Combo)) {
            nd.name = Self::a11y_label_before(recs, k).unwrap_or_default();
        }
        match role {
            Role::Slider => {
                nd.value = s.value as f64;
                nd.min = s.min.min(s.max) as f64;
                nd.max = s.max as f64;
                nd.step = 1.0;
            }
            Role::Progress => {
                nd.value = if s.value < 0 { -1.0 } else { s.value as f64 };
                nd.max = s.max as f64;
            }
            Role::Heading => nd.value = s.value.clamp(1, 9) as f64,
            _ => {}
        }
        if nd.state & ax::S_DISABLED == 0 {
            if matches!(
                role,
                Role::Button | Role::Link | Role::CheckBox | Role::Radio | Role::Switch | Role::Tab | Role::ListItem | Role::Menu
                    | Role::MenuItem | Role::Combo
            ) {
                nd.actions |= ax::A_CLICK;
            }
            if focusable(role) {
                nd.actions |= ax::A_FOCUS;
            }
            if role == Role::Slider {
                nd.actions |= ax::A_VALUE;
            }
            if role == Role::TextBox {
                nd.actions |= ax::A_TEXT;
            }
        }
        nd
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
        if let Some((win, _, r)) = self.m.a11y.click.take() {
            let pw = if self.headless { win as i32 } else { self.uiw_pw(win as u32) };
            if pw >= 0 && r[2] > 0 && r[3] > 0 {
                let ev = plat::Event { win: pw as u8, x: r[0] + r[2] / 2, y: r[1] + r[3] / 2, btn: plat::BTN_LEFT, ..Default::default() };
                for typ in [plat::E_MOVE, plat::E_DOWN, plat::E_UP] {
                    plat::evq_push(plat::Event { typ, ..ev });
                }
                plat::wake();
            }
        }
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
        if self.headless {
            return;
        }
        if plat::a11y::active() {
            plat::a11y::publish(self.a11y_tree());
        }
    }

    /// (runtime, E_WAKE) take the actions of the assistive technology: a node of the last
    /// pass that is enabled and not behind a modal dialog; the next passes carry them out
    pub(crate) fn a11y_poll(&mut self) {
        for r in plat::a11y::take() {
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
                Act::Click => self.m.a11y.click = Some((win, id, s.rect)),
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
