// a11y_wire -- the accessibility tree for hosts that keep a mirror of it (the browser's ARIA
// shadow tree, Android's virtual views; web_a11y.rs): every published tree becomes its
// differences to the tree the host has, in a byte format that web/a11y.js and
// android/GalleryAccessibility.java read.  Compiled on every target (plattest checks it).
//
//   Mirror::default()      nothing sent yet: the next update is a full one
//   Mirror::update(&tree, &changes, place) -> Option<Vec<u8>>
//                          the update for the host (None: nothing it shows changed); place
//                          gives a window's client area in host px (x, y, w, h)
//   Mirror::reset()        the host lost its mirror: the next update is a full one
//   Mirror::set_version(v) the format the host reads: VERSION (1, the default) or VERSION2 (the
//                          rich node data and announcements; web/a11y.js); a full update follows
//
// The format: integers little-endian, str = u32 byte count + UTF-8, key = u64 (Node::key)
//   u8 VERSION (or VERSION2), u8 flags (F_FULL: the host drops its mirror first), u8 number of windows
//   every shown window in the order of the snapshot (later ones lie on top):
//     u8 window index, u8 flags (W_DIALOG, W_ACTIVE), i32 x, y, w, h (place),
//     key of the focused node (0: none), str title,
//     u32 order count (UNCHANGED: the same nodes as before), then per node in tree order:
//         key, u32 parent (its position in this list; UNCHANGED: a top-level node) --
//         the nodes of the window that are not in the list are gone
//     u32 node count, then the nodes that are new or changed: key, u8 role (Role in
//         declaration order: Group 0 .. Dialog 20, Heading 21), u32 state (S_*), u32 actions
//         (A_*), i32 x, y, w, h, str name, str text, i32 caret, i32 anchor, i32 caret x, y, w,
//         h, f64 value (headings: the level), min, max, step;
//         VERSION2 adds: u8 sub (Sub in declaration order: None 0, Table 1 .. Figure 27),
//         str desc, str url, u32 position in set, set size, u32 item level, u32 cell row,
//         column, row span, column span, u32 grid rows, columns, u8 live
//   windows that are not listed are gone
//   u32 number of events, each: u8 kind (E_*), u8 window, key, i32 a, i32 b, and for E_TEXT
//     str old text, for E_ANNOUNCE (VERSION2 only; a: the politeness) str the text -- the events
//     of plat::a11y::diff without Added / Removed (the order tells those) and Live (the host's
//     own live regions say those)

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::{Change, Node, Snapshot, WinTree};

/// the first byte of an update
pub const VERSION: u8 = 1;
/// the first byte of an update with the rich node data (Mirror::set_version)
pub const VERSION2: u8 = 2;
/// update flag: a full update (the host drops what it has)
pub const F_FULL: u8 = 1;
/// window flags
pub const W_DIALOG: u8 = 1;
pub const W_ACTIVE: u8 = 2;
/// order count: the window's nodes are the same as before; parent: a top-level node
pub const UNCHANGED: u32 = u32::MAX;
/// events: a window appeared (a 1) or went away
pub const E_WINDOW: u8 = 1;
/// the window got (a 1) or lost the keyboard
pub const E_ACTIVE: u8 = 2;
/// the keyboard focus moved to the node
pub const E_FOCUS: u8 = 3;
/// state bit a (S_*) went on (b 1) or off
pub const E_STATE: u8 = 4;
pub const E_NAME: u8 = 5;
pub const E_VALUE: u8 = 6;
/// the text changed (the old one follows)
pub const E_TEXT: u8 = 7;
/// the caret moved to character a
pub const E_CARET: u8 = 8;
/// (VERSION2) a text to say: a the politeness (LIVE_*), the text follows
pub const E_ANNOUNCE: u8 = 9;

/// what the host has: the last tree sent and its window headers
#[derive(Default)]
pub struct Mirror {
    last: Option<Arc<Snapshot>>,
    heads: Vec<u8>,
    /// the format the host reads (0: VERSION)
    version: u8,
}

struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn rect(&mut self, r: &[i32; 4]) {
        for v in r {
            self.i32(*v);
        }
    }
    fn node(&mut self, n: &Node, rich: bool) {
        self.u64(n.key);
        self.u8(n.role as u8);
        self.u32(n.state);
        self.u32(n.actions);
        self.rect(&n.rect);
        self.str(&n.name);
        self.str(&n.text);
        self.i32(n.caret);
        self.i32(n.anchor);
        self.rect(&n.caret_rect);
        for v in [n.value, n.min, n.max, n.step] {
            self.f64(v);
        }
        if rich {
            self.u8(n.sub as u8);
            self.str(&n.desc);
            self.str(&n.url);
            for v in [n.set[0], n.set[1], n.item_level, n.cell[0], n.cell[1], n.cell[2], n.cell[3], n.grid[0], n.grid[1]] {
                self.u32(v);
            }
            self.u8(n.live);
        }
    }
}

// the host shows the same node (the links aside: the order carries them)
fn same(a: &Node, b: &Node) -> bool {
    a.key == b.key
        && a.role == b.role
        && a.state == b.state
        && a.actions == b.actions
        && a.rect == b.rect
        && a.name == b.name
        && a.text == b.text
        && a.caret == b.caret
        && a.anchor == b.anchor
        && a.caret_rect == b.caret_rect
        && a.value.to_bits() == b.value.to_bits()
        && a.min.to_bits() == b.min.to_bits()
        && a.max.to_bits() == b.max.to_bits()
        && a.step.to_bits() == b.step.to_bits()
        && a.sub == b.sub
        && a.desc == b.desc
        && a.url == b.url
        && a.set == b.set
        && a.item_level == b.item_level
        && a.cell == b.cell
        && a.grid == b.grid
        && a.live == b.live
}

// the same nodes under the same parents in the same order
fn same_order(a: &WinTree, b: &WinTree) -> bool {
    a.nodes.len() == b.nodes.len() && a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.key == y.key && x.parent == y.parent)
}

impl Mirror {
    pub const fn new() -> Self {
        Mirror { last: None, heads: Vec::new(), version: VERSION }
    }

    /// the host lost its mirror (or turned it off): the next update is a full one
    pub fn reset(&mut self) {
        self.last = None;
        self.heads.clear();
    }

    /// the format the host reads: VERSION2 for the rich node data and announcements, anything
    /// else VERSION; the next update is a full one
    pub fn set_version(&mut self, v: u8) {
        self.version = if v == VERSION2 { VERSION2 } else { VERSION };
        self.reset();
    }

    /// the format of the updates (VERSION or VERSION2)
    pub fn version(&self) -> u8 {
        if self.version == VERSION2 { VERSION2 } else { VERSION }
    }

    /// the update that brings the host from the last tree sent to s (changes: plat::a11y::diff
    /// of the two); None when nothing the host shows changed
    pub fn update(&mut self, s: &Arc<Snapshot>, changes: &[Change], place: &dyn Fn(&WinTree) -> [i32; 4]) -> Option<Vec<u8>> {
        let old = self.last.take();
        let full = old.is_none();
        let rich = self.version() == VERSION2;
        let mut o = Out(Vec::new());
        let wins = &s.wins[..s.wins.len().min(255)];
        o.u8(self.version());
        o.u8(if full { F_FULL } else { 0 });
        o.u8(wins.len() as u8);
        let mut heads = Vec::new();
        let mut news = full;
        for w in wins {
            let ow = old.as_ref().and_then(|o| o.win(w.win));
            let at = o.0.len();
            o.u8(w.win);
            o.u8(if w.dialog { W_DIALOG } else { 0 } | if w.active { W_ACTIVE } else { 0 });
            o.rect(&place(w));
            o.u64(w.focus_key());
            o.str(&w.title);
            heads.extend_from_slice(&o.0[at..]);
            if ow.is_some_and(|ow| same_order(ow, w)) {
                o.u32(UNCHANGED);
            } else {
                news = true;
                o.u32(w.nodes.len() as u32);
                for n in &w.nodes {
                    o.u64(n.key);
                    o.u32(n.parent);
                }
            }
            let at = o.0.len();
            o.u32(0);
            let mut count = 0u32;
            for n in &w.nodes {
                let prev = ow.and_then(|ow| ow.find(n.key).map(|k| &ow.nodes[k]));
                if prev.is_some_and(|p| same(p, n)) {
                    continue;
                }
                o.node(n, rich);
                count += 1;
            }
            o.0[at..at + 4].copy_from_slice(&count.to_le_bytes());
            news |= count > 0;
        }
        let at = o.0.len();
        o.u32(0);
        let mut count = 0u32;
        for c in changes {
            let (kind, win, key, a, b) = match c {
                Change::Window(w, on) => (E_WINDOW, *w, 0, *on as i32, 0),
                Change::Activate(w, on) => (E_ACTIVE, *w, 0, *on as i32, 0),
                Change::Focus(w, k) => (E_FOCUS, *w, *k, 0, 0),
                Change::State(w, k, bit, on) => (E_STATE, *w, *k, *bit as i32, *on as i32),
                Change::Name(w, k) => (E_NAME, *w, *k, 0, 0),
                Change::Value(w, k) => (E_VALUE, *w, *k, 0, 0),
                Change::Text(w, k, _) => (E_TEXT, *w, *k, 0, 0),
                Change::Caret(w, k, c) => (E_CARET, *w, *k, *c, 0),
                Change::Announce(w, _, p) if rich => (E_ANNOUNCE, *w, 0, *p as i32, 0),
                Change::Added(..) | Change::Removed(..) | Change::Live(..) | Change::Announce(..) => continue,
            };
            o.u8(kind);
            o.u8(win);
            o.u64(key);
            o.i32(a);
            o.i32(b);
            match c {
                Change::Text(_, _, old) => o.str(old),
                Change::Announce(_, text, _) => o.str(text),
                _ => {}
            }
            count += 1;
        }
        o.0[at..at + 4].copy_from_slice(&count.to_le_bytes());
        news |= count > 0 || heads != self.heads;
        self.heads = heads;
        self.last = Some(s.clone());
        news.then_some(o.0)
    }
}
