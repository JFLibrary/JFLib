// inspect -- the live inspector: an overlay in the app's windows and the tool window
// "Inspektor" (ROADMAP §12)
//
// App-facing:
//   inspect_enable(on)         arm the inspector: its hot keys open and close it (off: closed,
//                              the keys reach the app again)
//   inspect_show(on)           open / close it (opening arms it)
//   inspect_open() -> bool     it is open
//   inspect_keys(&[chord])     its hot keys, VK | MOD_* << 16 (default Ctrl+Shift+I and F12)
// Command line: --inspect arms and opens it; jfgui_INSPECT=1 arms it, jfgui_INSPECT=open opens
// it too.
//
// Closed (the default) it costs nothing: the runtime's hooks test one flag and nothing is
// recorded or drawn.  Open:
//   * the overlay in every window of the app: the widget under the pointer outlined, with a tag
//     (kind, id in hex and its hashed name when known, label, size in DIP, state); the selected
//     widget; optionally the layout boxes, the Tab order and the repainted rectangles of the
//     last frames (flashing).  The overlay is drawn into the framebuffer after the app's
//     commands, outside their lists: the app's dirty regions stay its own (ins_render_begin
//     adds the overlay's old and new rectangles, which are redrawn from the app's commands)
//   * the tool window "Inspektor" (WF_TOOL: it never takes the keyboard by itself; Esc, its
//     close button and drops on it never reach the app): the widget tree of the current pass
//     (the semantic records of the app's windows inside their layout boxes; collapsible,
//     selectable; selecting highlights in the app), the properties of the selected widget, a
//     script selector to copy (Ctrl+C or "Kopieren"), frame statistics, the theme's colour
//     tokens; "Auswählen" picks a widget with a click into the app (the click does not reach
//     it; Esc ends the pick)
// The inspector's own window is left out of the tree, the overlay and the pick.
// Development builds edit in it (inspect_edit.rs): the tab "Farben" is editable, the tab
// "Darstellung" and the widget operations below the properties are added.
//
// Script commands (auto.rs `inspect ...`, sc_inspect):
//   inspect on|off|toggle      open / close (on arms it)     inspect arm|disarm   hot keys
//   inspect pick               the pick mode ("Auswählen")   inspect select S|none
//   inspect option outlines|order|dirty on|off               inspect tab widgets|frame|colors|look
//   inspect copy               copy the selector ("Kopieren")
//   inspect snap               remember the main window's frame (for `inspect expect same`)
//   inspect expect open|closed|idle|same|excluded            inspect expect pick|flash on|off
//   inspect expect selected S|none   inspect expect hover S|none
//   inspect expect selector "s"      inspect expect copied "s"
// idle: the inspector did no work since it closed (or since the last idle check); same: the main
// window renders byte for byte as at `inspect snap`; excluded: the tree holds no record of the
// inspector's own window.  Editing (development builds): inspect set|reset|undo|redo|save|list|
// do ..., inspect expect changes|list|value|undo|redo|message (inspect_edit.rs).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::auto::{parse_qstring, skip_ws, word_eq, Sel};
use super::cmd::{cmd_bbox, cmd_hash, render_layer, Cmd};
use super::consts::*;
use super::keys::*;
use super::layout::px2dip_at;
use super::{App, Sem, Shared, TreeRow, TreeState, Ui};
use crate::i18n::tr;

/// the window id of the inspector's tool window (UIC_WINCLOSE / UIC_ESCAPE / UIC_DROP with it
/// never reach the app)
pub(crate) const INSPECT_WIN: u32 = 0x7FFF_1000;

// widget ids inside the inspector's window (its own hit list; below the hashed range, bits
// 29 and 30 clear: the tree's chevrons take id | 0x4000_0000)
const ID_ROOT: u32 = 0x07FE_0000;
const ID_PICK: u32 = 0x07FE_0001;
const ID_OUTL: u32 = 0x07FE_0002;
const ID_ORDER: u32 = 0x07FE_0003;
const ID_DIRTY: u32 = 0x07FE_0004;
const ID_COPY: u32 = 0x07FE_0005;
const ID_TABS: u32 = 0x07FE_0010; // + 1 + i: the tabs
const ID_TOKENS: u32 = 0x07FE_0020;
const ID_TREE: u32 = 0x07FE_1000; // + 1 + i: the rows

const WIN_W: i32 = 400; // the tool window's first size (DIP)
const WIN_H: i32 = 660;
const ROW_H: i32 = 22; // tree rows (DIP)
const KEY_W: i32 = 140; // property names (DIP)
const LABEL_CUT: usize = 48; // characters of a label in the tree and the tag

// overlay colours: the same on every theme (the overlay must stand out on all of them)
const C_HOVER: u32 = 0xFFFF_8C00;
const C_HOVER_FILL: u32 = 0x1CFF_8C00;
const C_SEL: u32 = 0xFF1E_88E5;
const C_SEL_FILL: u32 = 0x221E_88E5;
const C_BOX: u32 = 0xB000_ACC1;
const C_DIRTY: u32 = 0x00E5_3935; // alpha by age
const C_TAG: u32 = 0xF021_2121;
const C_TAG_TEXT: u32 = 0xFFFF_FFFF;
const C_ORDER: u32 = 0xE06A_1B9A;
const FLASH_MS: u32 = 700; // a repainted rectangle fades out in this time
const FLASH_MAX: usize = 96;

/// the names of the theme tokens (T_* / 4)
use super::theme::TOKEN_NAMES;
const LAYER_NAMES: [&str; L_COUNT] = ["base", "dialog", "popup", "tip"];

/// a layout box begun or ended in a pass (box_begin / box_end while the inspector is open):
/// its window, the number of semantic records before it, id, rectangle (window px, mirrored
/// right-to-left like the records), layer
#[derive(Clone, Copy)]
pub(crate) struct BoxEv {
    win: u8,
    begin: bool,
    layer: u8,
    at: u32,
    id: u32,
    rect: [i32; 4],
}

// what a node of the tree stands for
const N_WIDGET: u8 = 0;
const N_BOX: u8 = 1;
const N_WIN: u8 = 2;

/// a node of the widget tree: its record (boxes and windows as K_BOX records), depth, a key
/// that stays the same from pass to pass, whether deeper nodes follow
struct Node {
    rec: Sem,
    what: u8,
    /// the index of the semantic record (widgets), usize::MAX for boxes and windows
    sem: usize,
    depth: u16,
    key: u64,
    kids: bool,
}

/// statistics: the last run_passes, the last render of each window (app windows)
#[derive(Clone, Copy, Default)]
struct Stats {
    passes: u32,
    build_us: u64,
    render_us: [u64; WIN_MAX],
    dirty_px: [i64; WIN_MAX],
    area_px: [i64; WIN_MAX],
    cmds: [u32; WIN_MAX],
    bytes: [u32; WIN_MAX],
    cap: [u32; WIN_MAX],
}

/// the inspector's state (ui.m.ins)
pub(crate) struct S {
    /// the hot keys work
    pub(crate) armed: bool,
    /// open: the overlay and the tool window
    pub(crate) on: bool,
    /// closed: its window is destroyed after the passes (ins_gone)
    pub(crate) gone: bool,
    keys: Vec<u32>,
    outlines: i32,
    order: i32,
    flash: i32,
    tab: i32,
    picking: bool,
    /// a pick to resolve in the next pass: window, point (px of the framebuffer)
    pick_at: Option<(u32, i32, i32)>,
    /// buttons whose release belongs to a pick (bit per BTN_*)
    up_eat: u32,
    /// the selected node (0 = none) and its record in the last pass
    sel: u64,
    pub(crate) sel_rec: Option<Sem>,
    /// passes that still scroll the selected row into view (the view's height may change
    /// with the properties shown below it)
    reveal: u8,
    tree: TreeState,
    collapsed: BTreeSet<u64>,
    /// the boxes of the current pass (all windows)
    pub(crate) boxes: Vec<BoxEv>,
    /// the pointer in each window (framebuffer px, -1 = outside), what is under it
    ptr: [(i32, i32); WIN_MAX],
    hover_key: [u64; WIN_MAX],
    /// the overlay drawn into each window: commands, their rectangles, hash
    ov: [Vec<Cmd>; WIN_MAX],
    ov_box: [Vec<[i32; 4]>; WIN_MAX],
    ov_hash: [u64; WIN_MAX],
    /// the live window's overlay is drawn after its render; when the render started
    ov_now: bool,
    t_render: u64,
    /// repainted rectangles: window, rectangle (px), time (ms)
    flashes: Vec<(u8, [i32; 4], u32)>,
    st: Stats,
    open: Shared<i32>,
    rect: Shared<WinRect>,
    /// work done (passes, renders) -- a script checks that a closed inspector does none
    work: u32,
    work_mark: u32,
    /// `inspect snap`: the main window's frame (w, h, pixels)
    snap: Option<(i32, i32, Vec<u8>)>,
}

impl Default for S {
    fn default() -> S {
        S {
            armed: false,
            on: false,
            gone: false,
            keys: alloc::vec![VK_I | (MOD_CTRL | MOD_SHIFT) << 16, VK_F12],
            outlines: 0,
            order: 0,
            flash: 0,
            tab: 0,
            picking: false,
            pick_at: None,
            up_eat: 0,
            sel: 0,
            sel_rec: None,
            reveal: 0,
            tree: TreeState::default(),
            collapsed: BTreeSet::new(),
            boxes: Vec::new(),
            ptr: [(-1, -1); WIN_MAX],
            hover_key: [0; WIN_MAX],
            ov: Default::default(),
            ov_box: Default::default(),
            ov_hash: [0; WIN_MAX],
            ov_now: false,
            t_render: 0,
            flashes: Vec::new(),
            st: Stats::default(),
            open: Shared::new(0),
            rect: Shared::default(),
            work: 0,
            work_mark: 0,
            snap: None,
        }
    }
}

// ------------------------------------------------------------------ helpers

// splitmix64 of h ^ v: node keys
fn mix(h: u64, v: u64) -> u64 {
    let mut z = (h ^ v).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// FNV-1a 64 of a text
fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3))
}

// the key of the n-th node with this base key in the pass (never 0)
fn ordinal(seen: &mut BTreeMap<u64, u32>, base: u64) -> u64 {
    let n = seen.entry(base).or_insert(0);
    let k = mix(base, *n as u64);
    *n += 1;
    k.max(1)
}

// the point lies in the rectangle (x, y, w, h)
fn inside(r: &[i32; 4], x: i32, y: i32) -> bool {
    r[2] > 0 && r[3] > 0 && x >= r[0] && y >= r[1] && x < r[0].wrapping_add(r[2]) && y < r[1].wrapping_add(r[3])
}

// the selector form of an id: #123 (small ids written by hand), #0x0812ABCD
fn id_text(id: u32) -> String {
    if id < 0x1_0000 { format!("#{}", id) } else { format!("#{:#010X}", id).replace("0X", "0x") }
}

// a label cut to LABEL_CUT characters (…), line breaks as blanks
fn cut(l: &str) -> String {
    let mut s: String = l.chars().take(LABEL_CUT).map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).collect();
    if l.chars().count() > LABEL_CUT {
        s.push('…');
    }
    s
}

// a label as a script string: " \ and control characters escaped (auto.rs parse_qstring)
fn quote(l: &str) -> String {
    let mut s = String::from("\"");
    for c in l.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\t' => s.push_str("\\t"),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn kind_name(k: u16) -> &'static str {
    KIND_NAMES[if k < K_KIND_COUNT { k as usize } else { 0 }]
}

fn state_text(st: u16) -> String {
    let mut s = String::new();
    for (k, n) in STATE_NAMES.iter().enumerate() {
        if st & (1 << k) != 0 {
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(n);
        }
    }
    s
}

// a frame of thickness t around r (4 rectangles: crisp at every scale)
fn frame(ov: &mut Vec<Cmd>, r: [i32; 4], t: i32, c: u32) {
    let [x, y, w, h] = r;
    if w <= 0 || h <= 0 {
        return;
    }
    let t = t.min(w).min(h).max(1);
    ov.push(Cmd::Rect { x, y, w, h: t, c });
    ov.push(Cmd::Rect { x, y: y + h - t, w, h: t, c });
    if h > 2 * t {
        ov.push(Cmd::Rect { x, y: y + t, w: t, h: h - 2 * t, c });
        ov.push(Cmd::Rect { x: x + w - t, y: y + t, w: t, h: h - 2 * t, c });
    }
}

// the area of the rectangles (x, y, w, h) as rectangles that do not overlap
fn disjoint(rs: &[[i32; 4]]) -> Vec<[i32; 4]> {
    let mut out: Vec<[i32; 4]> = Vec::new();
    for r in rs {
        let mut pieces = alloc::vec![[r[0], r[1], r[0] + r[2], r[1] + r[3]]]; // x0, y0, x1, y1
        for o in &out {
            let o = [o[0], o[1], o[0] + o[2], o[1] + o[3]];
            let mut next = Vec::new();
            for p in pieces {
                if p[0] >= o[2] || p[2] <= o[0] || p[1] >= o[3] || p[3] <= o[1] {
                    next.push(p);
                    continue;
                }
                // the parts of p outside o: above, below, left, right
                let (y0, y1) = (p[1].max(o[1]), p[3].min(o[3]));
                for q in [[p[0], p[1], p[2], o[1]], [p[0], o[3], p[2], p[3]], [p[0], y0, o[0], y1], [o[2], y0, p[2], y1]] {
                    let q = [q[0].max(p[0]), q[1].max(p[1]), q[2].min(p[2]), q[3].min(p[3])];
                    if q[2] > q[0] && q[3] > q[1] {
                        next.push(q);
                    }
                }
            }
            pieces = next;
        }
        out.extend(pieces.iter().map(|p| [p[0], p[1], p[2] - p[0], p[3] - p[1]]));
    }
    out
}

// two records are the same widget of the same pass
fn same(a: &Sem, b: &Sem) -> bool {
    a.win == b.win && a.id == b.id && a.kind == b.kind && a.rect == b.rect && a.label == b.label && a.layer == b.layer
}

impl Ui {
    // ------------------------------------------------------------------ app API

    /// arm the inspector (`--inspect`, `jfgui_INSPECT=1`): its hot keys (Ctrl+Shift+I and F12,
    /// inspect_keys) open and close it; false closes it, the keys reach the app again
    pub fn inspect_enable(&mut self, on: bool) {
        self.m.ins.armed = on;
        if !on {
            self.inspect_show(false);
        }
    }

    /// open / close the inspector: its overlay in the app's windows and its tool window
    /// "Inspektor" (opening arms it); closed it costs nothing
    pub fn inspect_show(&mut self, on: bool) {
        let s = &mut self.m.ins;
        if on {
            s.armed = true;
        }
        if on == s.on {
            return;
        }
        s.on = on;
        s.gone = !on;
        s.boxes.clear();
        s.picking = false;
        s.pick_at = None;
        if on {
            s.open.set(1);
        } else {
            s.open.set(0);
            s.flashes.clear();
            s.ptr = [(-1, -1); WIN_MAX];
            s.hover_key = [0; WIN_MAX];
            s.ov_hash = [0; WIN_MAX];
            for v in s.ov.iter_mut() {
                v.clear();
            }
            for v in s.ov_box.iter_mut() {
                v.clear();
            }
            s.work_mark = s.work;
            // the overlay's pixels: every window is drawn again from its commands
            self.dirty_force_full();
            for r in self.m.win.recs.iter_mut() {
                if let Some(p) = r.save.as_deref_mut() {
                    p.dirty.full = true;
                }
            }
        }
        self.invalidate();
        self.need_render = true;
    }

    /// the inspector is open
    pub fn inspect_open(&self) -> bool {
        self.m.ins.on
    }

    /// the inspector's hot keys: VK | MOD_* << 16 each (MOD_SHIFT / MOD_CTRL / MOD_ALT);
    /// default Ctrl+Shift+I and F12, none: only the API and `--inspect` open it
    pub fn inspect_keys(&mut self, keys: &[u32]) {
        self.m.ins.keys = keys.to_vec();
    }

    /// (runtime: prepare) --inspect: armed and open; jfgui_INSPECT=1 armed, =open also open
    pub(crate) fn inspect_args(&mut self, flag: bool) {
        let env = plat::env("jfgui_INSPECT");
        let env = env.as_deref().unwrap_or("");
        if flag || env == "open" {
            self.inspect_show(true);
        } else if env.starts_with('1') {
            self.inspect_enable(true);
        }
    }

    // the inspector's window index (-1: not created)
    fn ins_win(&self) -> i32 {
        self.uiw_index(INSPECT_WIN)
    }

    // ------------------------------------------------------------------ runtime hooks

    /// (process_events, while armed) the hot keys and the pick take their events: 0 the event
    /// goes on to the app, 1 taken (passes), 2 taken (nothing to build)
    pub(crate) fn ins_event(&mut self, ev: &plat::Event) -> u32 {
        let w = ev.win as u32 & (WIN_MAX as u32 - 1);
        let on = self.m.ins.on;
        match ev.typ {
            plat::E_KEY_DOWN => {
                let chord = (ev.data & 0xFFFF) | (ev.mods as u32 & (MOD_SHIFT | MOD_CTRL | MOD_ALT)) << 16;
                if self.m.ins.keys.contains(&chord) {
                    self.inspect_show(!on);
                    return 1;
                }
                if on && self.m.ins.picking && ev.data & 0xFFFF == VK_ESCAPE {
                    self.m.ins.picking = false;
                    self.invalidate();
                    return 1;
                }
                0
            }
            plat::E_DOWN if on && self.m.ins.picking && w as i32 != self.ins_win() => {
                let (x, y) = if ev.x != -1 { (ev.x, ev.y) } else { self.m.ins.ptr[w as usize] };
                self.m.ins.pick_at = Some((w, x, y));
                self.m.ins.picking = false;
                self.m.ins.up_eat |= 1 << (ev.btn & 31);
                self.invalidate();
                1
            }
            plat::E_UP if self.m.ins.up_eat & 1 << (ev.btn & 31) != 0 => {
                self.m.ins.up_eat &= !(1 << (ev.btn & 31));
                2
            }
            plat::E_MOVE | plat::E_DOWN | plat::E_UP if on && ev.x != -1 => {
                self.ins_pointer(w, ev.x, ev.y);
                0
            }
            plat::E_LEAVE if on => {
                self.ins_pointer(w, -1, -1);
                0
            }
            plat::E_FOCUS_OUT => {
                self.m.ins.up_eat = 0; // (a release the window will not see)
                0
            }
            _ => 0,
        }
    }

    // the pointer is at (x, y) of window w (-1: it left): the overlay follows what is under it
    fn ins_pointer(&mut self, w: u32, x: i32, y: i32) {
        for i in 0..WIN_MAX {
            let p = if i as u32 == w { (x, y) } else { (-1, -1) };
            if self.m.ins.ptr[i] == p {
                continue;
            }
            self.m.ins.ptr[i] = p;
            let k = self.ins_at(i as u32, p.0, p.1).map_or(0, |k| {
                let r = &self.sems[k];
                mix(mix(r.id as u64, r.kind as u64), (r.rect[0] as u64) << 32 ^ r.rect[1] as u32 as u64).max(1)
            });
            if k != self.m.ins.hover_key[i] {
                self.m.ins.hover_key[i] = k;
                self.need_render = true;
            }
        }
    }

    /// (run_passes: UIC_WINCLOSE / UIC_ESCAPE / UIC_DROP of the inspector's window) taken
    pub(crate) fn ins_cmd(&mut self, id: u32) -> bool {
        match id {
            UIC_WINCLOSE => {
                self.inspect_show(false);
                true
            }
            UIC_ESCAPE => {
                self.m.ins.picking = false;
                true
            }
            UIC_DROP => true,
            _ => false,
        }
    }

    /// (layout.rs box_begin, while open) a box of the live window begins at r (px)
    pub(crate) fn ins_box(&mut self, id: u32, r: [i32; 4]) {
        let rect = self.phys_rect(r);
        let e = BoxEv { win: self.uiw_cur as u8, begin: true, layer: self.wp.ctx.cmd_layer_ui as u8, at: self.sems.len() as u32, id, rect };
        self.m.ins.boxes.push(e);
    }

    /// (layout.rs box_end, while open) the live window's innermost box ends
    pub(crate) fn ins_box_end(&mut self) {
        let e = BoxEv { win: self.uiw_cur as u8, begin: false, layer: 0, at: self.sems.len() as u32, id: 0, rect: [0; 4] };
        self.m.ins.boxes.push(e);
    }

    // ------------------------------------------------------------------ the tree

    // the semantic record under (x, y) of window w: the highest layer, the smallest one, the
    // last one
    fn ins_at(&self, w: u32, x: i32, y: i32) -> Option<usize> {
        if x == -1 && y == -1 {
            return None;
        }
        let mut best: Option<(usize, u8, i64)> = None;
        for (i, r) in self.sems.iter().enumerate() {
            if r.win as u32 != w || !inside(&r.rect, x, y) {
                continue;
            }
            let area = r.rect[2] as i64 * r.rect[3] as i64;
            if best.is_none_or(|(_, l, a)| r.layer > l || (r.layer == l && area <= a)) {
                best = Some((i, r.layer, area));
            }
        }
        best.map(|b| b.0)
    }

    // the tree of the current pass: per window of the app (the main window first) its records
    // inside the layout boxes they were emitted in; boxes without records are left out
    fn ins_tree(&self) -> Vec<Node> {
        let iw = self.ins_win();
        let mut wins: Vec<u32> = Vec::new();
        for r in &self.sems {
            let w = r.win as u32;
            if w as i32 != iw && !wins.contains(&w) {
                wins.push(w);
            }
        }
        wins.sort_unstable();
        let multi = wins.len() > 1;
        let mut out: Vec<Node> = Vec::new();
        let mut seen: BTreeMap<u64, u32> = BTreeMap::new();
        for &w in &wins {
            let base = multi as u16;
            let wkey = mix(1, w as u64).max(1);
            if multi {
                let fb = self.win_fb(w);
                let title = if w == 0 { String::from(tr("Hauptfenster")) } else { String::from(self.uiw_title(w)) };
                let rec = Sem { kind: K_BOX, rect: [0, 0, fb.w, fb.h], label: Some(title), win: w as u8, id: self.uiw_rec_id(w), ..Sem::default() };
                out.push(Node { rec, what: N_WIN, sem: usize::MAX, depth: 0, key: wkey, kids: false });
            }
            let evs: Vec<BoxEv> = self.m.ins.boxes.iter().filter(|e| e.win as u32 == w).copied().collect();
            let mut ei = 0;
            // open boxes: their node, whether a record is inside
            let mut stack: Vec<(usize, bool)> = Vec::new();
            let n = self.sems.len();
            for k in 0..=n {
                while ei < evs.len() && (k == n || evs[ei].at as usize <= k) {
                    let e = evs[ei];
                    ei += 1;
                    if e.begin {
                        let depth = base + stack.len() as u16;
                        let key = ordinal(&mut seen, mix(mix(2, w as u64), e.id as u64));
                        let rec = Sem { id: e.id, kind: K_BOX, rect: e.rect, layer: e.layer, win: w as u8, ..Sem::default() };
                        out.push(Node { rec, what: N_BOX, sem: usize::MAX, depth, key, kids: false });
                        stack.push((out.len() - 1, false));
                    } else if let Some((i, used)) = stack.pop() {
                        if !used {
                            out.truncate(i); // an empty box (with its empty boxes)
                        }
                    }
                }
                if k == n {
                    break;
                }
                let r = &self.sems[k];
                if r.win as u32 != w {
                    continue;
                }
                for s in stack.iter_mut().rev() {
                    if s.1 {
                        break; // (the boxes below are marked already)
                    }
                    s.1 = true;
                }
                let depth = base + stack.len() as u16;
                let parent = stack.last().map_or(wkey, |s| out[s.0].key);
                let b = if r.id != 0 {
                    mix(mix(3, w as u64), (r.id as u64) << 16 | r.kind as u64)
                } else {
                    mix(mix(4, parent), hash_str(r.label.as_deref().unwrap_or("")) ^ r.kind as u64)
                };
                let key = ordinal(&mut seen, b);
                out.push(Node { rec: r.clone(), what: N_WIDGET, sem: k, depth, key, kids: false });
            }
            while let Some((i, used)) = stack.pop() {
                if !used {
                    out.truncate(i);
                }
            }
        }
        for i in 0..out.len() {
            out[i].kids = out.get(i + 1).is_some_and(|n| n.depth > out[i].depth);
        }
        out
    }

    // the node a pick at (x, y) of window w selects: the widget there (ins_at's rule), else the
    // smallest box
    fn ins_pick_node(nodes: &[Node], w: u32, x: i32, y: i32) -> Option<usize> {
        let mut best: Option<(usize, u8, u8, i64)> = None; // (node, widget?, layer, area)
        for (i, n) in nodes.iter().enumerate() {
            if n.what == N_WIN || n.rec.win as u32 != w || !inside(&n.rec.rect, x, y) {
                continue;
            }
            let wd = (n.what == N_WIDGET) as u8;
            let area = n.rec.rect[2] as i64 * n.rec.rect[3] as i64;
            if best.is_none_or(|(_, bw, l, a)| (wd, n.rec.layer) > (bw, l) || ((wd, n.rec.layer) == (bw, l) && area <= a)) {
                best = Some((i, wd, n.rec.layer, area));
            }
        }
        best.map(|b| b.0)
    }

    // select node i: its ancestors open, the tree scrolls to it
    fn ins_select(&mut self, nodes: &[Node], i: usize) {
        self.m.ins.sel = nodes[i].key;
        self.m.ins.sel_rec = Some(nodes[i].rec.clone());
        let mut d = nodes[i].depth;
        for n in nodes[..i].iter().rev() {
            if d == 0 {
                break;
            }
            if n.depth < d {
                self.m.ins.collapsed.remove(&n.key);
                d = n.depth;
            }
        }
        self.m.ins.reveal = 3;
        self.m.ins.tab = 0;
    }

    // the script selector of a record: kind:"Label" when that names it alone, else #id, else
    // its centre (DIP)
    pub(crate) fn ins_selector(&self, r: &Sem) -> String {
        let iw = self.ins_win();
        if let Some(l) = r.label.as_deref().filter(|l| !l.is_empty() && l.len() < 250) {
            let n = self.sems.iter().filter(|s| s.win as i32 != iw && s.kind == r.kind && s.label.as_deref() == Some(l)).count();
            if n == 1 {
                return format!("{}:{}", kind_name(r.kind), quote(l));
            }
        }
        if r.id != 0 {
            return id_text(r.id);
        }
        let dpi = self.win_dpi(r.win as u32);
        format!("{},{}", px2dip_at(r.rect[0] + r.rect[2] / 2, dpi), px2dip_at(r.rect[1] + r.rect[3] / 2, dpi))
    }

    // copy the selected widget's selector (Ctrl+C, "Kopieren")
    fn ins_copy(&mut self) {
        if let Some(r) = self.m.ins.sel_rec.clone() {
            let s = self.ins_selector(&r);
            self.clip_set(CLIP_TEXT, s.as_bytes());
        }
    }

    // ------------------------------------------------------------------ the tool window

    /// (run_passes, after the main window's pass, while open) the inspector's part of the
    /// pass: the tree of this pass, a pick, the tool window
    pub(crate) fn inspect_frame(&mut self) {
        if self.m.ins.open.get() == 0 {
            self.inspect_show(false); // its window was closed
            return;
        }
        self.m.ins.work = self.m.ins.work.wrapping_add(1);
        let nodes = self.ins_tree();
        if let Some((w, x, y)) = self.m.ins.pick_at.take() {
            if let Some(i) = Ui::ins_pick_node(&nodes, w, x, y) {
                self.ins_select(&nodes, i);
                self.repass();
            }
        }
        let sel = self.m.ins.sel;
        self.m.ins.sel_rec = nodes.iter().find(|n| sel != 0 && n.key == sel).map(|n| n.rec.clone());
        if self.m.ins.rect.get().flags & WRF_POS == 0 {
            self.ins_place();
        }
        let open = self.m.ins.open.clone();
        let rect = self.m.ins.rect.clone();
        self.ied_plain(true); // (inspect_edit.rs) the style's own look: the edits do not reach it
        if self.window_begin(INSPECT_WIN, tr("Inspektor"), Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
            self.ins_ui(&nodes);
        }
        self.window_end();
        self.ied_plain(false);
        self.ied_after(); // the edits its controls asked for (inspect_edit.rs)
    }

    // the first placement: beside the main window when the monitor has room (headless:
    // always), else inside its right edge
    fn ins_place(&mut self) {
        let dpi = self.win_dpi(0);
        let fb = self.win_fb(0);
        let cw = px2dip_at(fb.w, dpi);
        let outside = self.headless
            || match (plat::win_get_rect(0), plat::win_work_area(0)) {
                (Some(r), Some(a)) => r[0] + r[2] + super::layout::dp_at(WIN_W + 24, dpi) <= a[0] + a[2],
                _ => false,
            };
        let rc = if outside { WinRect::new(cw + 12, 0, WIN_W, WIN_H, WRF_POS) } else { WinRect::new((cw - WIN_W - 16).max(0), 48, WIN_W, WIN_H, WRF_POS) };
        self.m.ins.rect.set(rc);
    }

    // the content of the tool window
    fn ins_ui(&mut self, nodes: &[Node]) {
        self.next_size(-1, -1);
        self.box_begin(ID_ROOT, BOX_V | pad(8), 6);
        // the pick and the overlay's options
        self.box_begin(0, BOX_H | BOX_WRAP | BOX_XCENTER, 10);
        let mut v = self.m.ins.picking as i32;
        if self.toggle_button(ID_PICK, tr("Auswählen"), &mut v) & EV_ACT != 0 {
            self.m.ins.picking = v != 0;
        }
        let mut v = self.m.ins.outlines;
        self.checkbox(ID_OUTL, tr("Umrisse"), &mut v);
        self.m.ins.outlines = v;
        let mut v = self.m.ins.order;
        self.checkbox(ID_ORDER, tr("Tab-Reihenfolge"), &mut v);
        self.m.ins.order = v;
        let mut v = self.m.ins.flash;
        self.checkbox(ID_DIRTY, tr("Neu gezeichnet"), &mut v);
        self.m.ins.flash = v;
        self.box_end();
        if self.m.ins.picking {
            self.text(TS_CAPTION | TX_ACCENT, tr("Klicken Sie auf ein Widget (Esc beendet)"));
        }
        self.hr_ins_note(); // a failed build of hot reload (hotreload.rs)
        let mut tab = self.m.ins.tab;
        let mut tabs = alloc::vec![tr("Widgets"), tr("Bildaufbau"), tr("Farben")];
        if super::inspect_edit::EDIT {
            tabs.push(tr("Darstellung")); // (inspect_edit.rs) development builds
        }
        self.segmented(ID_TABS, &mut tab, &tabs);
        self.m.ins.tab = tab;
        match tab {
            1 => self.ins_stats(nodes),
            2 => {
                if !self.ied_colors() {
                    self.ins_colors(); // (a --release build: read only)
                }
            }
            3 => self.ied_look(),
            _ => {
                self.ins_tree_ui(nodes);
                self.ins_props();
            }
        }
        // the window's keys: Ctrl+C copies the selector, Esc ends a pick (never UIC_ESCAPE)
        if self.kb_chord(VK_C, MOD_CTRL) {
            self.ins_copy();
        }
        self.ied_keys(); // Ctrl+Z / Ctrl+Y: the changes (inspect_edit.rs)
        if self.key(VK_ESCAPE) {
            self.m.ins.picking = false;
            self.repass();
        }
        self.box_end();
    }

    // a node's row: label, detail
    fn ins_row_text(&self, n: &Node) -> (String, Option<String>) {
        match n.what {
            N_WIN => (n.rec.label.clone().unwrap_or_default(), (n.rec.id != 0).then(|| id_text(n.rec.id))),
            N_BOX => (String::from("box"), Some(id_text(n.rec.id))),
            _ => {
                let mut s = String::from(kind_name(n.rec.kind));
                if let Some(l) = n.rec.label.as_deref().filter(|l| !l.is_empty()) {
                    s.push(' ');
                    s.push_str(&quote(&cut(l)));
                }
                (s, (n.rec.id != 0).then(|| id_text(n.rec.id)))
            }
        }
    }

    // the widget tree (the visible rows: collapsed nodes hide what is below them)
    fn ins_tree_ui(&mut self, nodes: &[Node]) {
        let mut vis: Vec<usize> = Vec::new();
        let mut hide: Option<u16> = None;
        for (i, n) in nodes.iter().enumerate() {
            if let Some(d) = hide {
                if n.depth > d {
                    continue;
                }
                hide = None;
            }
            vis.push(i);
            if n.kids && self.m.ins.collapsed.contains(&n.key) {
                hide = Some(n.depth);
            }
        }
        let texts: Vec<(String, Option<String>)> = vis.iter().map(|&i| self.ins_row_text(&nodes[i])).collect();
        let rows: Vec<TreeRow> = vis
            .iter()
            .zip(texts.iter())
            .map(|(&i, (l, d))| TreeRow {
                key: nodes[i].key,
                depth: nodes[i].depth,
                has_children: nodes[i].kids,
                check: None,
                icon: 0,
                label: l,
                detail: d.as_deref(),
            })
            .collect();
        let mut st = core::mem::take(&mut self.m.ins.tree);
        st.expanded = nodes.iter().filter(|n| n.kids && !self.m.ins.collapsed.contains(&n.key)).map(|n| n.key).collect();
        let sel = self.m.ins.sel;
        st.cur = (sel != 0).then_some(sel);
        self.next_size(-1, -1);
        let ev = self.tree(ID_TREE, &mut st, &rows, ROW_H);
        // a selected widget: its row into the middle of the view when it is not in view (in
        // the next pass; the view's height may still change while the layout settles)
        if self.m.ins.reveal > 0 {
            self.m.ins.reveal -= 1;
            let vh = self.last_rect()[3];
            if let Some(p) = rows.iter().position(|r| r.key == sel) {
                let rh = self.dp(ROW_H);
                let y = p as i32 * rh;
                let s = self.slot(ID_TREE);
                if super::list::into_view(y, rh, s.scrolly, vh) != 0 {
                    s.scrolly = (y - (vh - rh) / 2).max(0); // (the view clamps it)
                    s.adur = s.scrolly as u32;
                    self.repass();
                }
            }
        }
        if let Some(k) = ev.collapsed {
            self.m.ins.collapsed.insert(k);
        }
        if let Some(k) = ev.expanded {
            self.m.ins.collapsed.remove(&k);
        }
        if ev.sel_changed {
            if let Some(k) = st.cur {
                self.m.ins.sel = k;
                self.m.ins.sel_rec = nodes.iter().find(|n| n.key == k).map(|n| n.rec.clone());
            }
        }
        self.m.ins.tree = st;
    }

    // the properties of the selected node
    fn ins_props(&mut self) {
        self.separator();
        let Some(r) = self.m.ins.sel_rec.clone() else {
            self.text(TS_BODY | TX_SECONDARY, tr("Nichts ausgewählt"));
            return;
        };
        let w = r.win as u32;
        let dpi = self.win_dpi(w);
        let d = |v: i32| px2dip_at(v, dpi);
        self.key_value(tr("Art"), kind_name(r.kind), KEY_W);
        let mut id = if r.id == 0 { String::from("0") } else { format!("{:#010X} ({})", r.id, r.id).replace("0X", "0x") };
        if let Some(n) = self.id_name(r.id) {
            id.push_str(&format!("  \"{}\"", n));
        }
        self.key_value(tr("Kennung"), &id, KEY_W);
        let label = r.label.clone().unwrap_or_default();
        self.key_value(tr("Beschriftung"), &cut(&label), KEY_W);
        let rc = r.rect;
        self.key_value(tr("Rechteck"), &format!("{}, {}  {} x {} DIP", d(rc[0]), d(rc[1]), d(rc[2]), d(rc[3])), KEY_W);
        self.key_value("", &format!("{}, {}  {} x {} px", rc[0], rc[1], rc[2], rc[3]), KEY_W);
        let value = if r.min != 0 { format!("{} / {} .. {}", r.value, r.min, r.max) } else { format!("{} / {}", r.value, r.max) };
        self.key_value(tr("Wert"), &value, KEY_W);
        let st = state_text(r.state);
        self.key_value(tr("Zustand"), if st.is_empty() { "-" } else { &st }, KEY_W);
        let layer = format!("{} ({})", r.layer, LAYER_NAMES[(r.layer as usize) & (L_COUNT - 1)]);
        self.key_value(tr("Ebene"), &layer, KEY_W);
        let win = if w == 0 { String::from(tr("Hauptfenster")) } else { format!("{} \"{}\"", self.uiw_rec_id(w), self.uiw_title(w)) };
        self.key_value(tr("Fenster"), &win, KEY_W);
        // the selector and its copy button
        let sel = self.ins_selector(&r);
        self.box_begin(0, BOX_H | BOX_XCENTER, 12);
        self.next_size(KEY_W, 0);
        self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, tr("Selektor"));
        self.next_size(-1, 0);
        self.text(TS_BODY | TX_ELLIPSIS, &sel);
        self.next_flags(ST_COMPACT);
        if self.button(ID_COPY, tr("Kopieren")) & EV_ACT != 0 {
            self.ins_copy();
        }
        self.box_end();
        self.ied_operate(&r); // click, drag, set, type, cmd (inspect_edit.rs)
    }

    // the frame statistics
    fn ins_stats(&mut self, nodes: &[Node]) {
        let iw = self.ins_win();
        let s = self.m.ins.st;
        let app = |i: usize| i as i32 != iw;
        let sum = |v: &[u64; WIN_MAX]| (0..WIN_MAX).filter(|&i| app(i)).map(|i| v[i]).sum::<u64>();
        let sumi = |v: &[i64; WIN_MAX]| (0..WIN_MAX).filter(|&i| app(i)).map(|i| v[i]).sum::<i64>();
        let sumu = |v: &[u32; WIN_MAX]| (0..WIN_MAX).filter(|&i| app(i)).map(|i| v[i] as u64).sum::<u64>();
        self.key_value(tr("Durchgänge"), &format!("{}", s.passes), KEY_W);
        self.key_value(tr("Aufbau"), &format!("{} µs", s.build_us), KEY_W);
        self.key_value(tr("Zeichnen"), &format!("{} µs", sum(&s.render_us)), KEY_W);
        let (px, area) = (sumi(&s.dirty_px), sumi(&s.area_px).max(1));
        let pm = (px * 1000 / area) as i32;
        self.key_value(tr("Neu gezeichnete Fläche"), &format!("{} px ({}.{} %)", px, pm / 10, pm % 10), KEY_W);
        let cmds = format!("{} ({} / {} KiB)", sumu(&s.cmds), sumu(&s.bytes) / 1024, sumu(&s.cap) / 1024);
        self.key_value(tr("Zeichenbefehle"), &cmds, KEY_W);
        let widgets = nodes.iter().filter(|n| n.what == N_WIDGET).count();
        self.key_value(tr("Widgets"), &format!("{}", widgets), KEY_W);
        let (n, nmax, b, bmax) = self.text.gc_usage();
        self.key_value(tr("Glyphen-Cache"), &format!("{} / {} ({} / {} KiB)", n, nmax, b / 1024, bmax / 1024), KEY_W);
    }

    // the theme's colour tokens: swatch, name, value
    fn ins_colors(&mut self) {
        self.next_size(-1, -1);
        self.scroll_begin(ID_TOKENS, SC_V);
        let stroke = self.color(T_CTRL_STROKE);
        for (i, name) in TOKEN_NAMES.iter().enumerate() {
            let c = self.theme[i];
            self.box_begin(0, BOX_H | BOX_XCENTER, 10);
            self.next_size(32, 18);
            let r = self.place(0, 0);
            // white and black halves under the colour show its alpha
            let h2 = r[2] / 2;
            self.draw_color(stroke);
            self.draw_rect(r[0], r[1], r[2], r[3]);
            self.draw_color(0xFFFF_FFFF);
            self.draw_rect(r[0] + 1, r[1] + 1, h2 - 1, r[3] - 2);
            self.draw_color(0xFF00_0000);
            self.draw_rect(r[0] + h2, r[1] + 1, r[2] - h2 - 1, r[3] - 2);
            self.draw_color(c);
            self.draw_rect(r[0] + 1, r[1] + 1, r[2] - 2, r[3] - 2);
            self.next_size(160, 0);
            self.text(TS_CAPTION, &format!("T_{}", name));
            self.text(TS_CAPTION | TX_SECONDARY, &format!("#{:08X}", c));
            self.box_end();
        }
        self.scroll_end();
    }

    // ------------------------------------------------------------------ the overlay

    /// (render_live, after dirty_compute, while open) n dirty rectangles of the live window's
    /// commands (-1 = everything): noted (statistics, flashes); when the overlay changes the
    /// rectangles of the old and the new one join them -> the new count (-1 = everything)
    pub(crate) fn ins_render_begin(&mut self, n: i32) -> i32 {
        self.m.ins.ov_now = false;
        let w = self.uiw_cur;
        if w as i32 == self.ins_win() {
            return n; // the inspector itself: no overlay
        }
        let wi = w as usize & (WIN_MAX - 1);
        self.m.ins.work = self.m.ins.work.wrapping_add(1);
        self.m.ins.t_render = plat::time_us();
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        // statistics and flashes: the app's own rectangles
        if n != 0 {
            let area = fw as i64 * fh as i64;
            let px = if n < 0 { area } else { self.drects[..n as usize].iter().map(|r| r[2] as i64 * r[3] as i64).sum() };
            self.m.ins.st.dirty_px[wi] = px;
            self.m.ins.st.area_px[wi] = area;
            if self.m.ins.flash != 0 {
                let now = self.now_ms;
                let rs: Vec<[i32; 4]> = if n < 0 { alloc::vec![[0, 0, fw, fh]] } else { self.drects[..n as usize].to_vec() };
                let f = &mut self.m.ins.flashes;
                f.retain(|e| now.wrapping_sub(e.2) < FLASH_MS);
                for r in rs {
                    f.push((w as u8, r, now));
                }
                if f.len() > FLASH_MAX {
                    let k = f.len() - FLASH_MAX;
                    f.drain(..k);
                }
            }
        }
        {
            let c = &self.wp.cmd;
            let st = &mut self.m.ins.st;
            st.cmds[wi] = c.layers.iter().map(|l| l.len() as u32).sum();
            st.bytes[wi] = c.len.iter().sum();
            st.cap[wi] = c.cap.iter().sum();
        }
        // the overlay; when it changed, its old and new rectangles are drawn again
        let ov = self.ins_overlay(w);
        let h = ov.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, c| mix(h, cmd_hash(c)));
        let boxes: Vec<[i32; 4]> = ov
            .iter()
            .filter_map(cmd_bbox)
            .filter_map(|b| {
                let (x0, y0, x1, y1) = (b[0].max(0), b[1].max(0), b[2].min(fw), b[3].min(fh));
                (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
            })
            .collect();
        let mut n = n;
        if h != self.m.ins.ov_hash[wi] && n >= 0 {
            let old = core::mem::take(&mut self.m.ins.ov_box[wi]);
            self.drects.truncate(n as usize);
            if n as usize + old.len() + boxes.len() > super::dirty::DRECTS_MAX {
                self.drects.clear();
                n = -1;
            } else {
                self.drects.extend_from_slice(&old);
                self.drects.extend_from_slice(&boxes);
                n += (old.len() + boxes.len()) as i32;
            }
        }
        self.m.ins.ov_now = n != 0 && !ov.is_empty();
        self.m.ins.ov[wi] = ov;
        self.m.ins.ov_box[wi] = boxes;
        self.m.ins.ov_hash[wi] = h;
        // repainted rectangles still fading: more frames
        if self.m.ins.flash != 0 && !self.m.ins.flashes.is_empty() {
            self.animating = true;
        }
        n
    }

    /// (render_live, after the app's commands were drawn, before --verify-dirty) the overlay
    /// on top, inside the rectangles that were drawn (split into disjoint pieces: the overlay's
    /// translucent fills are blended once)
    pub(crate) fn ins_render_end(&mut self) {
        let w = self.uiw_cur;
        if w as i32 == self.ins_win() {
            return;
        }
        let wi = w as usize & (WIN_MAX - 1);
        if self.m.ins.ov_now {
            let n = self.wp.rt.present_n;
            let pieces = if n < 0 { None } else { Some(disjoint(&self.drects[..(n as usize).min(self.drects.len())])) };
            self.ins_draw(false, pieces.as_deref());
        }
        self.m.ins.st.render_us[wi] = plat::time_us().saturating_sub(self.m.ins.t_render);
    }

    /// (runtime verify_dirty, while open) the live window's overlay into the full redraw it
    /// compares with (shadow) or into the repaired framebuffer
    pub(crate) fn ins_verify(&mut self, shadow: bool) {
        if self.uiw_cur as i32 != self.ins_win() {
            self.ins_draw(shadow, None);
        }
    }

    // draw the live window's overlay into its framebuffer or its shadow surface, inside the
    // rectangles (None: everywhere)
    fn ins_draw(&mut self, shadow: bool, clip: Option<&[[i32; 4]]>) {
        let wi = self.uiw_cur as usize & (WIN_MAX - 1);
        if self.m.ins.ov[wi].is_empty() {
            return;
        }
        let icon_style = (self.style, self.color(T_ACCENT), self.theme_dark, self.color(T_TEXT_DISABLED), self.wp.rt.dpi as u32);
        let ov = core::mem::take(&mut self.m.ins.ov[wi]);
        let Ui { wp, dc, text, icon_hook, .. } = self;
        let sf = if shadow { &mut wp.rt.shadow } else { &mut wp.rt.fb };
        dc.begin(sf);
        match clip {
            None => render_layer(&ov, 0, dc, sf, text, icon_hook, icon_style),
            Some(rs) => {
                for r in rs {
                    dc.clip_push(r[0], r[1], r[2], r[3]);
                    render_layer(&ov, 0, dc, sf, text, icon_hook, icon_style);
                    dc.clip_pop();
                }
            }
        }
        dc.end();
        self.m.ins.ov[wi] = ov;
    }

    // the overlay of the live window w (framebuffer px)
    fn ins_overlay(&mut self, w: u32) -> Vec<Cmd> {
        let mut ov = Vec::new();
        let (fw, fh) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        let hair = self.hair();
        let thick = self.dp(2).max(1);
        // the layout boxes of the last pass
        if self.m.ins.outlines != 0 {
            for e in self.m.ins.boxes.iter().filter(|e| e.begin && e.win as u32 == w) {
                frame(&mut ov, e.rect, hair, C_BOX);
            }
        }
        // the repainted rectangles of the last frames, fading
        if self.m.ins.flash != 0 {
            let now = self.now_ms;
            for &(fwin, r, t) in &self.m.ins.flashes {
                let age = now.wrapping_sub(t);
                if fwin as u32 != w || age >= FLASH_MS {
                    continue;
                }
                let k = FLASH_MS - age;
                ov.push(Cmd::Rect { x: r[0], y: r[1], w: r[2], h: r[3], c: (0x48 * k / FLASH_MS) << 24 | C_DIRTY });
                frame(&mut ov, r, hair, (0x60 + 0x9F * k / FLASH_MS) << 24 | C_DIRTY);
            }
        }
        self.ts_font(TS_CAPTION);
        let (font, size) = self.text.get_font();
        let (asc, desc) = (self.text.ascent(), self.text.descent());
        let pad = self.dp(4);
        // the selection
        if let Some(r) = self.m.ins.sel_rec.as_ref().filter(|r| r.win as u32 == w) {
            ov.push(Cmd::Rect { x: r.rect[0], y: r.rect[1], w: r.rect[2], h: r.rect[3], c: C_SEL_FILL });
            frame(&mut ov, r.rect, thick, C_SEL);
        }
        // the widget under the pointer, with its tag
        let mut tag = Vec::new();
        let (px, py) = self.m.ins.ptr[w as usize & (WIN_MAX - 1)];
        if let Some(k) = self.ins_at(w, px, py) {
            let r = self.sems[k].clone();
            ov.push(Cmd::Rect { x: r.rect[0], y: r.rect[1], w: r.rect[2], h: r.rect[3], c: C_HOVER_FILL });
            frame(&mut ov, r.rect, hair.max(thick / 2), C_HOVER);
            let mut s = String::from(kind_name(r.kind));
            if r.id != 0 {
                s.push_str("  ");
                s.push_str(&id_text(r.id));
                if let Some(n) = self.id_name(r.id) {
                    s.push_str(&format!(" (\"{}\")", n));
                }
            }
            if let Some(l) = r.label.as_deref().filter(|l| !l.is_empty()) {
                s.push_str("  ");
                s.push_str(&quote(&cut(l)));
            }
            let dpi = self.wp.rt.dpi;
            s.push_str(&format!("  {} x {}", px2dip_at(r.rect[2], dpi), px2dip_at(r.rect[3], dpi)));
            let st = state_text(r.state);
            if !st.is_empty() {
                s.push_str(&format!("  [{}]", st));
            }
            let tw = self.text.width(&s).min(fw - 2 * pad).max(1);
            let (bw, bh) = (tw + 2 * pad, asc + desc + 2 * pad);
            let x = r.rect[0].clamp(0, (fw - bw).max(0));
            let above = r.rect[1] - bh - hair;
            let y = if above >= 0 { above } else { (r.rect[1] + r.rect[3] + hair).min((fh - bh).max(0)) };
            let t = hair as u16;
            tag.push(Cmd::RRect { x, y, w: bw, h: bh, r: self.dp(3) as u16, t, fill: C_TAG, stroke: C_HOVER, stroke2: C_HOVER });
            tag.push(Cmd::Text { x: x + pad, y: y + pad + asc, font: font as u16, size, c: C_TAG_TEXT, maxw: tw, s: s.into_bytes() });
        }
        // the Tab order: a number at each Tab stop (the last pass's hit list)
        if self.m.ins.order != 0 {
            let c = &self.wp.ctx;
            let list = &c.hits[c.hit_cur ^ 1];
            let (stops, _) = super::kbnav::stops(list, c.focus, c.kb.gmem());
            // (numbered in Tab order; stops scrolled out of their view keep their number)
            let rects: Vec<(usize, [i32; 4])> = stops
                .iter()
                .enumerate()
                .map(|(k, &i)| {
                    let h = &list[i];
                    (k, if self.rtl { [fw - h.x1, h.y0, h.x1 - h.x0, h.y1 - h.y0] } else { [h.x0, h.y0, h.x1 - h.x0, h.y1 - h.y0] })
                })
                .filter(|(_, r)| r[2] > 0 && r[3] > 0)
                .collect();
            for (k, r) in rects.iter() {
                let s = format!("{}", k + 1);
                let tw = self.text.width(&s);
                let bh = asc + desc + 2;
                let bw = (tw + 2 * pad).max(bh);
                // on the stop's top left corner (it covers little of the widget)
                let (x, y) = ((r[0] - bw / 2).clamp(0, (fw - bw).max(0)), (r[1] - bh / 2).clamp(0, (fh - bh).max(0)));
                frame(&mut ov, *r, hair, C_ORDER);
                ov.push(Cmd::RRect { x, y, w: bw, h: bh, r: (bh / 2) as u16, t: 0, fill: C_ORDER, stroke: 0, stroke2: 0 });
                ov.push(Cmd::Text { x: x + (bw - tw) / 2, y: y + 1 + asc, font: font as u16, size, c: C_TAG_TEXT, maxw: 0, s: s.into_bytes() });
            }
        }
        ov.append(&mut tag); // the tag above everything
        ov
    }

    // ------------------------------------------------------------------ statistics

    /// (run_passes, at its end, while open) passes and App::frame time of that run
    pub(crate) fn ins_passes(&mut self, passes: u32, build_us: u64) {
        self.m.ins.st.passes = passes;
        self.m.ins.st.build_us = build_us;
    }

    /// (run_passes, at its end, after closing) the tool window's state and buffers are freed:
    /// the app's windows are as if the inspector never ran (its placement is kept)
    pub(crate) fn ins_gone(&mut self) {
        self.m.ins.gone = false;
        self.uiw_use(0);
        self.window_destroy(INSPECT_WIN);
    }

    // ------------------------------------------------------------------ script commands

    /// `inspect ...` (auto.rs; see the module header)
    pub(crate) fn sc_inspect(&mut self, app: &mut dyn App, a: &[u8]) {
        const BAD: &[u8] = b"inspect on|off|toggle|arm|disarm|pick|select S|option outlines|order|dirty on|off|tab widgets|frame|colors|look|copy|snap|expect ...|set|reset|undo|redo|save|list|do ...";
        let w = |n: &str| word_eq(a, n).is_some();
        if w("on") || w("off") || w("toggle") {
            let on = if w("toggle") { !self.m.ins.on } else { w("on") };
            self.inspect_show(on);
            self.run_passes(app);
        } else if w("arm") || w("disarm") {
            self.inspect_enable(w("arm"));
            self.run_passes(app);
        } else if w("pick") {
            if !self.m.ins.on {
                self.fail(b"the inspector is closed");
                return;
            }
            self.m.ins.picking = true;
            self.invalidate();
            self.run_passes(app);
        } else if let Some(r) = word_eq(a, "select") {
            let r = skip_ws(r);
            if word_eq(r, "none").is_some() {
                self.m.ins.sel = 0;
                self.m.ins.sel_rec = None;
            } else {
                let Some(k) = self.ins_resolve(r) else { return };
                let nodes = self.ins_tree();
                match nodes.iter().position(|n| n.sem == k) {
                    Some(i) => self.ins_select(&nodes, i),
                    None => {
                        self.fail(b"not in the inspector's tree");
                        return;
                    }
                }
            }
            self.invalidate();
            self.run_passes(app);
        } else if let Some(r) = word_eq(a, "option") {
            let r = skip_ws(r);
            let v = word_eq(skip_ws(super::auto::next_word(r)), "on").is_some() as i32;
            if word_eq(r, "outlines").is_some() {
                self.m.ins.outlines = v;
            } else if word_eq(r, "order").is_some() {
                self.m.ins.order = v;
            } else if word_eq(r, "dirty").is_some() {
                self.m.ins.flash = v;
            } else {
                self.fail(b"inspect option outlines|order|dirty on|off");
                return;
            }
            self.invalidate();
            self.run_passes(app);
        } else if let Some(r) = word_eq(a, "tab") {
            let r = skip_ws(r);
            // look: "Darstellung" (development builds, inspect_edit.rs)
            let n = if super::inspect_edit::EDIT { 4 } else { 3 };
            let t = ["widgets", "frame", "colors", "look"][..n].iter().position(|n| word_eq(r, n).is_some());
            match t {
                Some(t) => self.m.ins.tab = t as i32,
                None => {
                    self.fail(if n == 4 { &b"inspect tab widgets|frame|colors|look"[..] } else { b"inspect tab widgets|frame|colors" });
                    return;
                }
            }
            self.invalidate();
            self.run_passes(app);
        } else if w("copy") {
            self.ins_copy();
            self.run_passes(app);
        } else if w("snap") {
            self.render();
            let fb = &self.wp.rt.fb;
            self.m.ins.snap = Some((fb.w, fb.h, fb.px.clone()));
        } else if let Some(r) = word_eq(a, "expect") {
            self.sc_inspect_expect(skip_ws(r));
        } else if !self.ied_script(app, a) {
            // (set, reset, undo, redo, save, list, do: inspect_edit.rs)
            self.fail(BAD);
        }
    }

    // a selector of the app's widgets -> the index of its record (None: failed, reported)
    fn ins_resolve(&mut self, p: &[u8]) -> Option<usize> {
        match self.resolve(p).0 {
            None => {
                self.sel_fail();
                None
            }
            Some(Sel::Point) => {
                self.fail(b"a widget selector (#id or a label), not a point");
                None
            }
            Some(Sel::Rec(k)) => {
                if self.sems[k].win as i32 == self.ins_win() {
                    self.fail(b"a widget of the inspector itself");
                    return None;
                }
                Some(k)
            }
        }
    }

    // inspect expect ...
    fn sc_inspect_expect(&mut self, a: &[u8]) {
        let w = |n: &str| word_eq(a, n).is_some();
        if w("open") || w("closed") {
            if self.m.ins.on != w("open") {
                self.fail(if self.m.ins.on { b"the inspector is open" } else { b"the inspector is closed" });
            }
        } else if let Some(r) = word_eq(a, "pick") {
            let want = word_eq(skip_ws(r), "on").is_some();
            if self.m.ins.picking != want {
                self.fail(if want { b"no pick" } else { b"picking" });
            }
        } else if let Some(r) = word_eq(a, "flash") {
            let now = self.now_ms;
            let live = self.m.ins.flash != 0 && self.m.ins.flashes.iter().any(|e| now.wrapping_sub(e.2) < FLASH_MS);
            if live != word_eq(skip_ws(r), "on").is_some() {
                self.fail(if live { b"repainted rectangles are shown" } else { b"no repainted rectangle is shown" });
            }
        } else if w("idle") {
            let d = self.m.ins.work.wrapping_sub(self.m.ins.work_mark);
            self.m.ins.work_mark = self.m.ins.work;
            if d != 0 || !self.m.ins.boxes.is_empty() || self.m.ins.ov.iter().any(|v| !v.is_empty()) {
                self.fail(format!("the inspector worked {} times, {} boxes recorded", d, self.m.ins.boxes.len()).as_bytes());
            }
        } else if w("same") {
            self.render();
            let fb = &self.wp.rt.fb;
            let d = match &self.m.ins.snap {
                None => Err(String::from("no inspect snap")),
                Some((sw, sh, _)) if (*sw, *sh) != (fb.w, fb.h) => Err(format!("the size is {}x{}, was {}x{}", fb.w, fb.h, sw, sh)),
                Some((_, _, px)) => {
                    let n = px.chunks_exact(4).zip(fb.px.chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
                    if n == 0 { Ok(()) } else { Err(format!("{} pixels differ", n)) }
                }
            };
            if let Err(e) = d {
                self.fail(e.as_bytes());
            }
        } else if w("excluded") {
            let iw = self.ins_win();
            let own = iw >= 0 && self.sems.iter().any(|r| r.win as i32 == iw);
            if !own {
                self.fail(b"the inspector's window has no records");
            } else if self.ins_tree().iter().any(|n| n.rec.win as i32 == iw) {
                self.fail(b"the tree holds records of the inspector's window");
            }
        } else if let Some(r) = word_eq(a, "selected") {
            let r = skip_ws(r);
            if word_eq(r, "none").is_some() {
                if self.m.ins.sel_rec.is_some() {
                    self.fail(b"a widget is selected");
                }
                return;
            }
            let Some(k) = self.ins_resolve(r) else { return };
            match &self.m.ins.sel_rec {
                Some(s) if same(s, &self.sems[k]) => {}
                Some(s) => {
                    let m = format!("selected: {} {} {:?}", kind_name(s.kind), id_text(s.id), s.label.as_deref().unwrap_or(""));
                    self.fail(m.as_bytes());
                }
                None => self.fail(b"nothing selected"),
            }
        } else if let Some(r) = word_eq(a, "hover") {
            let r = skip_ws(r);
            let h = (0..WIN_MAX as u32).find_map(|i| {
                let (x, y) = self.m.ins.ptr[i as usize];
                self.ins_at(i, x, y)
            });
            if word_eq(r, "none").is_some() {
                if h.is_some() {
                    self.fail(b"a widget is under the pointer");
                }
                return;
            }
            let Some(k) = self.ins_resolve(r) else { return };
            if h != Some(k) {
                let m = match h {
                    Some(i) => format!("under the pointer: {} {}", kind_name(self.sems[i].kind), id_text(self.sems[i].id)),
                    None => String::from("nothing under the pointer"),
                };
                self.fail(m.as_bytes());
            }
        } else if let Some(r) = word_eq(a, "selector") {
            let Some((want, _)) = parse_qstring(skip_ws(r), 256) else {
                self.fail(b"expected a quoted string");
                return;
            };
            let have = self.m.ins.sel_rec.clone().map(|s| self.ins_selector(&s)).unwrap_or_default();
            if have.as_bytes() != want.as_slice() {
                self.fail(format!("selector is {}", have).as_bytes());
            }
        } else if let Some(r) = word_eq(a, "copied") {
            let Some((want, _)) = parse_qstring(skip_ws(r), 256) else {
                self.fail(b"expected a quoted string");
                return;
            };
            let have = self.clip_read(CLIP_TEXT).unwrap_or_default();
            if have != want {
                self.fail(format!("the clipboard holds {:?}", String::from_utf8_lossy(&have)).as_bytes());
            }
        } else if !self.ied_expect(a) {
            // (changes, list, undo, redo, message: inspect_edit.rs)
            self.fail(b"inspect expect open|closed|pick on|off|flash on|off|idle|same|excluded|selected S|hover S|selector \"s\"|copied \"s\"");
        }
    }

    // ------------------------------------------------------------------ hot reload

    /// (hotreload.rs) what a restart keeps: armed, open, tab, options (outlines, order,
    /// dirty), the selected widget's script selector
    pub(crate) fn ins_carry(&self) -> (bool, bool, i32, [i32; 3], Option<String>) {
        let s = &self.m.ins;
        let sel = s.sel_rec.as_ref().map(|r| self.ins_selector(r));
        (s.armed, s.on, s.tab, [s.outlines, s.order, s.flash], sel)
    }

    /// (hotreload.rs) set it again after a restart; the selector is looked up in the tree of a
    /// pass with the inspector open (a widget that is gone stays unselected)
    pub(crate) fn ins_restore(&mut self, app: &mut dyn App, armed: bool, on: bool, tab: i32, opts: [i32; 3], sel: Option<&str>) {
        self.inspect_enable(armed || on);
        self.inspect_show(on);
        [self.m.ins.outlines, self.m.ins.order, self.m.ins.flash] = opts;
        self.run_passes(app);
        if let Some(Sel::Rec(k)) = sel.filter(|_| on).and_then(|p| self.resolve(p.as_bytes()).0) {
            let nodes = self.ins_tree();
            if let Some(i) = nodes.iter().position(|n| n.sem == k) {
                self.ins_select(&nodes, i);
            }
        }
        self.m.ins.tab = tab.clamp(0, 2);
        self.invalidate();
        self.run_passes(app);
    }
}
