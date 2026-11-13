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
//   take_ops() / request_op(OpRequest)   the same for the further actions (Op: scroll into
//                                view, expand / collapse, increment / decrement, select items)
//   announce(win, text, politeness)   (UI thread) a text for the screen reader to say; the next
//                                publish hands it to the bridge (Change::Announce)
//   current() -> Option<Arc<Snapshot>>   (bridges) the tree last published
//   set_translator(f) / role_name(role) / role_text(k)   the names of the roles the bridges
//                                give (UIA LocalizedControlType, AT-SPI GetLocalizedRoleName) in
//                                the UI language: ROLE_TEXTS holds them in German, the keys of
//                                the UI's translation (ui registers i18n::tr; plat cannot ask
//                                ui itself); without one they stay German
//
// Linux: AT-SPI2 on the accessibility bus (linux_atspi.rs).  Windows: UI Automation providers
// answering WM_GETOBJECT (win_uia.rs).  macOS: NSAccessibility elements under each window's view,
// turned on by the first question of an assistive technology (apple_mac_a11y.rs, VoiceOver).  iOS:
// UIAccessibility elements under the host's view while VoiceOver runs (apple_ios_a11y.rs).  The
// browser and Android (web_a11y.rs): the host keeps a mirror of the tree -- an ARIA shadow tree
// in the DOM (web/a11y.js), virtual views of an
// AccessibilityNodeProvider (android/GalleryAccessibility.java) -- fed with the differences
// of every published tree (a11y_wire.rs: Mirror, the wire format); the host turns the mirror
// on (active() follows it) and sends the actions back.
//
// A Snapshot holds every shown window (WinTree) with its nodes in the order of the UI's
// records; Node::parent gives the hierarchy (NONE: a top-level node of the window), link()
// adds the sibling links and the key index the bridges navigate with.  A node's key stays the
// same from frame to frame while its widget does (the UI derives it from the widget id).
//
// Rich nodes (the records of a web page's tree, ui's Sem setters): Node::sub says what a node
// is more exactly than its role (a table, a cell, a landmark, a tree item ...); a bridge that
// knows Sub maps it to a role of its own, one that does not (yet) shows Node::role, the nearest
// basic role -- so a new Sub never breaks a bridge.  Further fields: a description, a link's
// address, the place in a set, a level, a cell's place and a table's size, a live region's
// politeness; states S_REQUIRED .. S_SELECTABLE; actions A_EXPAND .. A_SELECT, which come back
// as Op requests (take_ops).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use rt::sync::SpinLock;

/// no node (Node::parent of a top-level node, WinTree::focus without a focused node)
pub const NONE: u32 = u32::MAX;

/// what a node is
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
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

/// What a node is more exactly than its role (Node::sub; the wire format sends it as its number in
/// declaration order).  A bridge that knows it maps it to a role of its own; the others show
/// Node::role, the nearest basic role, which `role()` gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Sub {
    /// nothing more than the role
    #[default]
    None,
    /// a table or grid (role Group; Node::grid: its rows and columns), one of its rows (Group)
    Table,
    Row,
    /// a cell (role Label; Node::cell: its place), a column header, a row header (Label)
    Cell,
    ColumnHeader,
    RowHeader,
    /// the landmarks of a page (role Group): its banner, its main content, a navigation,
    /// complementary content, the content information (a footer), a search, a form, a named region
    Banner,
    Main,
    Navigation,
    Complementary,
    ContentInfo,
    Search,
    Form,
    Region,
    /// a tree (role List) and its items (ListItem; Node::level(): its depth, 1 the top)
    Tree,
    TreeItem,
    /// a group of radio buttons, a tool bar (Group), a tooltip (Label), a separator (Group)
    RadioGroup,
    Toolbar,
    Tooltip,
    Separator,
    /// live regions (role Label): an alert (assertive), a status line and a log (polite)
    Alert,
    Status,
    Log,
    /// a meter (role Progress), a spin button and a scroll bar (Slider)
    Meter,
    SpinButton,
    ScrollBar,
    /// an article, a figure (Group)
    Article,
    Figure,
}

/// the number of Sub values (SUB_TEXTS)
pub const SUB_COUNT: usize = 28;

impl Sub {
    /// the nearest basic role (what a bridge that does not know Sub shows; None: no rule)
    pub fn role(self) -> Option<Role> {
        Some(match self {
            Sub::None => return None,
            Sub::Cell | Sub::ColumnHeader | Sub::RowHeader | Sub::Tooltip | Sub::Alert | Sub::Status | Sub::Log => Role::Label,
            Sub::Tree => Role::List,
            Sub::TreeItem => Role::ListItem,
            Sub::Meter => Role::Progress,
            Sub::SpinButton | Sub::ScrollBar => Role::Slider,
            _ => Role::Group,
        })
    }

    /// a landmark (Banner ..= Region)
    pub fn landmark(self) -> bool {
        matches!(self, Sub::Banner | Sub::Main | Sub::Navigation | Sub::Complementary | Sub::ContentInfo | Sub::Search | Sub::Form | Sub::Region)
    }

    /// its WAI-ARIA role ("" for None)
    pub fn aria(self) -> &'static str {
        match self {
            Sub::None => "",
            Sub::Table => "table",
            Sub::Row => "row",
            Sub::Cell => "cell",
            Sub::ColumnHeader => "columnheader",
            Sub::RowHeader => "rowheader",
            Sub::Banner => "banner",
            Sub::Main => "main",
            Sub::Navigation => "navigation",
            Sub::Complementary => "complementary",
            Sub::ContentInfo => "contentinfo",
            Sub::Search => "search",
            Sub::Form => "form",
            Sub::Region => "region",
            Sub::Tree => "tree",
            Sub::TreeItem => "treeitem",
            Sub::RadioGroup => "radiogroup",
            Sub::Toolbar => "toolbar",
            Sub::Tooltip => "tooltip",
            Sub::Separator => "separator",
            Sub::Alert => "alert",
            Sub::Status => "status",
            Sub::Log => "log",
            Sub::Meter => "meter",
            Sub::SpinButton => "spinbutton",
            Sub::ScrollBar => "scrollbar",
            Sub::Article => "article",
            Sub::Figure => "figure",
        }
    }
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
/// a field that must be filled in
pub const S_REQUIRED: u32 = 1 << 11;
/// a field whose content is not valid
pub const S_INVALID: u32 = 1 << 12;
/// its content is being changed (a live region says nothing meanwhile)
pub const S_BUSY: u32 = 1 << 13;
/// it opens a popup (a menu, a list, a dialog)
pub const S_HASPOPUP: u32 = 1 << 14;
/// a link to an address that was visited
pub const S_VISITED: u32 = 1 << 15;
/// a modal dialog (nothing else of the window takes input)
pub const S_MODAL: u32 = 1 << 16;
/// a list, tree or table that allows several selected items
pub const S_MULTISELECT: u32 = 1 << 17;
/// it can be expanded and is closed now (S_EXPANDED: open)
pub const S_COLLAPSED: u32 = 1 << 18;
/// outside the visible area (scrolled away, clipped): browse modes reach it, the pointer does not
pub const S_OFFSCREEN: u32 = 1 << 19;
/// it can be selected (rows and cells of a grid; list rows and tabs are anyway)
pub const S_SELECTABLE: u32 = 1 << 20;

/// Node::actions: press, toggle, select, open / close -- what a click on the widget does
pub const A_CLICK: u32 = 1;
/// give it the keyboard focus
pub const A_FOCUS: u32 = 2;
/// set a number (sliders)
pub const A_VALUE: u32 = 4;
/// set the text (text boxes)
pub const A_TEXT: u32 = 8;
/// expand and collapse it (Op::Expand / Op::Collapse)
pub const A_EXPAND: u32 = 16;
/// one step up or down (Op::Increment / Op::Decrement)
pub const A_STEP: u32 = 32;
/// scroll it into view (Op::ScrollIntoView)
pub const A_SCROLL: u32 = 64;
/// select it (Op::SelectItem; in a S_MULTISELECT container also Op::AddToSelection /
/// Op::RemoveFromSelection)
pub const A_SELECT: u32 = 128;

/// Node::live, Change::Announce: said when the person is idle
pub const LIVE_POLITE: u8 = 1;
/// at once, interrupting
pub const LIVE_ASSERTIVE: u8 = 2;

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
    /// what it is more exactly than its role (Sub::None: the role says it)
    pub sub: Sub,
    /// a description beside the name (a hint, a page's aria-describedby); empty: none
    pub desc: String,
    /// a link's address; empty: none
    pub url: String,
    /// its position in its set (1-based) and the set's size: list and tree items, tabs, radio
    /// buttons, rows (0: unknown)
    pub set: [u32; 2],
    /// the level of a tree item, list item or row (1 the top; 0: none); a heading's is its value
    /// (level() gives both)
    pub item_level: u32,
    /// cells and headers: row, column (0-based), row span, column span (spans 0: no place)
    pub cell: [u32; 4],
    /// tables and grids: the number of rows and columns (0: unknown)
    pub grid: [u32; 2],
    /// a live region's root: LIVE_POLITE / LIVE_ASSERTIVE (0: none) -- changes inside it are said
    pub live: u8,
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

    /// a heading's level 1 ..= 9; a tree item's, list item's or row's level (item_level; 0: none)
    pub fn level(&self) -> u32 {
        if self.role == Role::Heading { (self.value as u32).clamp(1, 9) } else { self.item_level }
    }

    /// a cell's place: row, column (0-based), row span, column span (None: no place)
    pub fn cell_place(&self) -> Option<[u32; 4]> {
        (self.cell[2] > 0 && self.cell[3] > 0).then_some(self.cell)
    }

    /// what the bridges call the node, in the UI language: its Sub's name, else its role's
    pub fn role_text(&self) -> &'static str {
        if self.sub == Sub::None { role_name(self.role) } else { sub_name(self.sub) }
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

    /// the deepest node at a client point (px): among siblings the last one wins (drawn on top);
    /// nodes outside the visible area (S_OFFSCREEN) are not hit
    pub fn at_point(&self, x: i32, y: i32) -> Option<usize> {
        let inside = |n: &Node| {
            let r = &n.rect;
            n.state & S_OFFSCREEN == 0 && x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
        };
        let mut found = None;
        let mut parent = NONE;
        loop {
            let hit = self.children(parent).filter(|&k| inside(&self.nodes[k])).last();
            match hit {
                Some(k) => {
                    found = Some(k);
                    parent = k as u32;
                }
                None => return found,
            }
        }
    }

    /// the node lies (partly) inside the window's client area (else it is scrolled away) and is
    /// not marked S_OFFSCREEN
    pub fn on_screen(&self, n: &Node) -> bool {
        let r = n.rect;
        n.state & S_OFFSCREEN == 0 && r[2] > 0 && r[3] > 0 && r[0] < self.size[0] && r[1] < self.size[1] && r[0] + r[2] > 0 && r[1] + r[3] > 0
    }

    /// the nearest ancestor of node k (or k itself) that is a live region's root (Node::live)
    pub fn live_root(&self, k: usize) -> Option<usize> {
        let mut i = k as u32;
        for _ in 0..self.nodes.len() {
            let n = self.nodes.get(i as usize)?;
            if n.live != 0 {
                return Some(i as usize);
            }
            i = n.parent;
        }
        None
    }

    /// the nodes below node k (its subtree without itself) are k + 1 .. the returned index (the
    /// nodes are in tree order: parents first, a subtree in one piece)
    pub fn subtree_end(&self, k: usize) -> usize {
        let mut j = k + 1;
        while j < self.nodes.len() && self.ancestor(k, j) {
            j += 1;
        }
        j
    }

    /// node a is an ancestor of node b
    pub fn ancestor(&self, a: usize, b: usize) -> bool {
        let mut p = self.nodes.get(b).map_or(NONE, |n| n.parent);
        for _ in 0..self.nodes.len() {
            if p == NONE {
                return false;
            }
            if p as usize == a {
                return true;
            }
            p = self.nodes[p as usize].parent;
        }
        false
    }

    /// the nearest ancestor of node k with this Sub (a cell's table)
    pub fn up_to(&self, k: usize, sub: Sub) -> Option<usize> {
        let mut p = self.nodes.get(k)?.parent;
        for _ in 0..self.nodes.len() {
            let n = self.nodes.get(p as usize)?;
            if n.sub == sub {
                return Some(p as usize);
            }
            p = n.parent;
        }
        None
    }

    /// a table's number of rows and columns: Node::grid, else counted from its cells' places (and
    /// its rows)
    pub fn grid_size(&self, table: usize) -> (u32, u32) {
        let Some(t) = self.nodes.get(table) else { return (0, 0) };
        let (mut rows, mut cols, mut row_nodes) = (t.grid[0], t.grid[1], 0u32);
        if rows == 0 || cols == 0 {
            let (mut r, mut c) = (0, 0);
            for j in table + 1..self.subtree_end(table) {
                let n = &self.nodes[j];
                if let Some(p) = n.cell_place() {
                    r = r.max(p[0] + p[2]);
                    c = c.max(p[1] + p[3]);
                }
                if n.sub == Sub::Row {
                    row_nodes += 1;
                }
            }
            if rows == 0 {
                rows = r.max(row_nodes);
            }
            if cols == 0 {
                cols = c;
            }
        }
        (rows, cols)
    }

    /// the cell of a table at row, column (one that spans it too)
    pub fn cell_at(&self, table: usize, row: u32, col: u32) -> Option<usize> {
        (table + 1..self.subtree_end(table)).find(|&j| {
            self.nodes[j].cell_place().is_some_and(|p| row >= p[0] && row < p[0] + p[2] && col >= p[1] && col < p[1] + p[3])
                && self.up_to(j, Sub::Table) == Some(table)
        })
    }

    /// the headers of a cell: column headers over its columns (rows false), or row headers in its
    /// rows (rows true), of the same table
    pub fn headers_of(&self, cell: usize, rows: bool) -> Vec<usize> {
        let (Some(p), Some(table)) = (self.nodes.get(cell).and_then(|n| n.cell_place()), self.up_to(cell, Sub::Table)) else { return Vec::new() };
        let want = if rows { Sub::RowHeader } else { Sub::ColumnHeader };
        (table + 1..self.subtree_end(table))
            .filter(|&j| j != cell && self.nodes[j].sub == want)
            .filter(|&j| {
                self.nodes[j].cell_place().is_some_and(|h| {
                    if rows { h[0] < p[0] + p[2] && p[0] < h[0] + h[2] } else { h[1] < p[1] + p[3] && p[1] < h[1] + h[3] }
                })
            })
            .collect()
    }

    /// the headers of a table: its column headers (rows false) or row headers
    pub fn headers(&self, table: usize, rows: bool) -> Vec<usize> {
        let want = if rows { Sub::RowHeader } else { Sub::ColumnHeader };
        (table + 1..self.subtree_end(table)).filter(|&j| self.nodes[j].sub == want && self.up_to(j, Sub::Table) == Some(table)).collect()
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
#[non_exhaustive]
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
    /// something inside a live region changed (a name, a text, a value, nodes that came or went):
    /// the region's root (Node::live says how urgently)
    Live(u8, u64),
    /// a text for the screen reader to say (announce): window, text, LIVE_POLITE / LIVE_ASSERTIVE
    Announce(u8, String, u8),
}

const STATE_EVENTS: u32 =
    S_CHECKED | S_MIXED | S_SELECTED | S_EXPANDED | S_DISABLED | S_PRESSED | S_REQUIRED | S_INVALID | S_BUSY | S_VISITED | S_COLLAPSED;

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
        // the live regions something changed in (their roots' keys, in order)
        let mut live: Vec<u64> = Vec::new();
        // (a polite region that appears says nothing of its first content; an alert does)
        let live_at = |t: &WinTree, k: usize, new: bool, live: &mut Vec<u64>| {
            if let Some(r) = t.live_root(k) {
                let key = t.nodes[r].key;
                if !live.contains(&key) && !(new && r == k && t.nodes[r].live != LIVE_ASSERTIVE) {
                    live.push(key);
                }
            }
        };
        for (p, index, key) in removed {
            if let Some(k) = o.find(key) {
                live_at(o, k, false, &mut live);
            }
            out.push(Change::Removed(w.win, p, key, index));
        }
        for (k, nd) in w.nodes.iter().enumerate() {
            let p = key_of(w, nd.parent);
            if gone(w, nd, o) && (p == 0 || o.find(p).is_some_and(|k| !gone(o, &o.nodes[k], w))) {
                live_at(w, k, true, &mut live);
                out.push(Change::Added(w.win, p, nd.key));
            }
        }
        for (i, nd) in w.nodes.iter().enumerate() {
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
                    let said = od.name != nd.name || od.value != nd.value || od.text != nd.text;
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
                    // (a region that just became live, or one that is busy, says nothing)
                    if said && od.live == nd.live {
                        live_at(w, i, false, &mut live);
                    }
                }
            }
        }
        for key in live {
            let busy = w.find(key).is_some_and(|k| w.nodes[k].state & S_BUSY != 0);
            if !busy {
                out.push(Change::Live(w.win, key));
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

/// a further action of the assistive technology (OpRequest; the node's Node::actions allow it)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Op {
    /// scroll the node into view (A_SCROLL)
    ScrollIntoView,
    /// open / close it (A_EXPAND)
    Expand,
    Collapse,
    /// one step up / down (A_STEP)
    Increment,
    Decrement,
    /// select it alone; add it to / take it out of the selection (A_SELECT)
    SelectItem,
    AddToSelection,
    RemoveFromSelection,
}

/// a further action for a node (request_op)
#[derive(Clone, Debug, PartialEq)]
pub struct OpRequest {
    /// the UI's window index
    pub win: u8,
    pub key: u64,
    pub op: Op,
}

const REQ_MAX: usize = 64;
const ANNOUNCE_MAX: usize = 16;

static CUR: SpinLock<Option<Arc<Snapshot>>> = SpinLock::new(None);
static REQ: SpinLock<Vec<Request>> = SpinLock::new(Vec::new());
static OPS: SpinLock<Vec<OpRequest>> = SpinLock::new(Vec::new());
static ANNOUNCED: SpinLock<Vec<(u8, String, u8)>> = SpinLock::new(Vec::new());
static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static EPOCH: AtomicU32 = AtomicU32::new(0);

/// start the platform bridge (once): Linux connects to the accessibility bus when assistive
/// technology is enabled (jfgui_A11Y=1 forces it, jfgui_A11Y=0 or NO_AT_BRIDGE=1 turn it off);
/// Windows answers WM_GETOBJECT from now on, macOS the accessibility questions of its views; the
/// browser and Android hosts turn their mirror on themselves (jfgui_a11y), jfgui_A11Y=0 keeps it
/// off there too
pub fn start(app: &str) {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    if crate::env("jfgui_A11Y").is_some_and(|v| v.starts_with('0')) || crate::env("NO_AT_BRIDGE").is_some_and(|v| v.starts_with('1')) {
        return;
    }
    native::start(app);
}

/// assistive technology may be listening (the UI publishes snapshots only then); on Windows it
/// turns false again 10 s after the last question of a UI Automation client unless a screen
/// reader (SPI_GETSCREENREADER) or Magnifier listens for events, and true with the next question
/// (win_uia.rs idle)
pub fn active() -> bool {
    #[cfg(windows)]
    if ACTIVE.load(Ordering::Relaxed) {
        crate::win_uia::idle();
    }
    ACTIVE.load(Ordering::Acquire)
}

/// when (time_us) active() wants to be asked again although nothing happens: the platform's
/// fallback that notices a client is gone (Windows: at most every 2 s while active); None: never.
/// The UI's window loop waits at most until then and asks, so an idle app learns it too
/// (UIC_A11Y)
pub fn check_due() -> Option<u64> {
    #[cfg(windows)]
    if ACTIVE.load(Ordering::Relaxed) {
        return crate::win_uia::idle_due();
    }
    None
}

/// (bridges) assistive technology is there (true) or gone; a change wakes the UI thread (it
/// publishes, and the app hears of it: UIC_A11Y).  Every call is a new epoch: a host that turned
/// its mirror on again needs the whole tree, even an unchanged one
#[allow(dead_code)]
pub(crate) fn set_active(on: bool) {
    EPOCH.fetch_add(1, Ordering::AcqRel);
    if ACTIVE.swap(on, Ordering::AcqRel) != on {
        crate::wake();
    }
}

/// a number that changes whenever a bridge turned on or off (set_active): the UI publishes a tree
/// equal to the last one again only after it changed (an unchanged frame publishes nothing)
pub fn epoch() -> u32 {
    EPOCH.load(Ordering::Acquire)
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
    let mut changes = match &old {
        Some(o) => diff(o, &s),
        None => s.wins.iter().flat_map(|w| [Change::Window(w.win, true), Change::Activate(w.win, w.active)]).collect(),
    };
    for (w, text, p) in core::mem::take(&mut *ANNOUNCED.lock()) {
        changes.push(Change::Announce(w, text, p));
    }
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

/// (bridges, tests) queue a further action (Op) and wake the UI thread (the oldest is dropped
/// when 64 wait)
pub fn request_op(r: OpRequest) {
    {
        let mut q = OPS.lock();
        if q.len() >= REQ_MAX {
            q.remove(0);
        }
        q.push(r);
    }
    crate::wake();
}

/// the further actions asked for since the last call, oldest first (UI thread)
pub fn take_ops() -> Vec<OpRequest> {
    core::mem::take(&mut *OPS.lock())
}

/// (UI thread) a text for the screen reader to say in window win: LIVE_POLITE when the person is
/// idle, LIVE_ASSERTIVE at once.  The next publish hands it to the bridge (Change::Announce:
/// UI Automation's notification event, AT-SPI2's announcement, the browser's live region);
/// nothing is kept while no assistive technology listens, at most 16 wait.
pub fn announce(win: u8, text: &str, politeness: u8) {
    if !active() || text.is_empty() {
        return;
    }
    let mut q = ANNOUNCED.lock();
    if q.len() < ANNOUNCE_MAX {
        let p = if politeness == LIVE_ASSERTIVE { LIVE_ASSERTIVE } else { LIVE_POLITE };
        q.push((win, String::from(text), p));
    }
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

/// what the bridges call the Sub values (UIA LocalizedControlType and LocalizedLandmarkType,
/// AT-SPI GetLocalizedRoleName) in Sub order, in German: the keys of their translations as for
/// ROLE_TEXTS (Sub::None has none)
pub const SUB_TEXTS: [&str; SUB_COUNT] = [
    "",
    "Tabelle",
    "Tabellenzeile",
    "Tabellenzelle",
    "Spaltenkopf",
    "Zeilenkopf",
    "Banner",
    "Hauptinhalt",
    "Navigation",
    "Ergänzung",
    "Inhaltsinformation",
    "Suche",
    "Formular",
    "Region",
    "Baumansicht",
    "Baumelement",
    "Optionsgruppe",
    "Symbolleiste",
    "Kurzinfo",
    "Trennlinie",
    "Meldung",
    "Status",
    "Protokoll",
    "Messanzeige",
    "Drehfeld",
    "Bildlaufleiste",
    "Artikel",
    "Abbildung",
];

/// what the bridges call a Sub, in the UI language ("" for Sub::None)
pub fn sub_name(s: Sub) -> &'static str {
    let de = SUB_TEXTS[(s as usize).min(SUB_COUNT - 1)];
    if de.is_empty() {
        return de;
    }
    let f = *TR.lock();
    f.map_or(de, |f| f(de))
}

#[cfg(target_os = "linux")]
use crate::linux_atspi as native;

#[cfg(windows)]
use crate::win_uia as native;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
use crate::web::a11y_host as native;

#[cfg(target_os = "macos")]
use crate::apple::mac::a11y_mac as native;

#[cfg(target_os = "ios")]
use crate::apple::ios::a11y_ios as native;

#[cfg(not(any(windows, target_os = "linux", target_arch = "wasm32", target_os = "android", target_vendor = "apple")))]
mod native {
    use super::{Change, Snapshot};
    use alloc::vec::Vec;
    pub fn start(_app: &str) {}
    pub fn changed(_s: &Snapshot, _c: Vec<Change>) {}
}
