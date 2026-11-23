// stencil -- the Shapes window of a diagram editor (Visio's Shapes pane), generic: the app hands
// in stencils with tiles (a name, a QuickInfo, a preview drawn through ui::Draw); the panel lays
// them out, takes the pointer, the keyboard and drags, tells screen readers, and reports what
// happened.  It knows nothing about diagrams; every text it shows comes from the app.
//
//   stencil_panel(id, &mut StencilState, &StencilPanel) -> Vec<StencilEvent>
//        top to bottom: the search box (IC_SEARCH, labels.search as its placeholder) with a
//        button for the recent searches (labels.history; neither with StencilState::no_search,
//        JD15), the "More shapes" button
//        (StencilPanel::more_id, labels.more), the title rows -- the quick shapes collection
//        (SK_QUICK, labels.quick), the search results while StencilPanel::results is Some
//        (SK_SEARCH, labels.results with "{}" = their number), a row per stencil --, a movable
//        splitter (StencilState::split, DIP; 0 = automatic), the tiles of the active view in a
//        scroll view.  Give it a size with next_size first (none: it fills its box).
//   stencil_drag_decode(bytes) -> Option<(stencil, tile)>   the keys a tile drag carries
//        (DragData::bytes of the app's kind StencilPanel::drag_kind: two u32, little-endian)
//
// Views (StencilState::active = the key of a title; a key that names none -- the default --
// shows the first stencil): a stencil shows its TL_QUICK tiles, a thin line, then the others
// (MS-04); an SD_EDITABLE stencil always has both parts, an empty part shows a hint
// (labels.quick_empty, labels.drop_hint).  The quick collection shows the TL_QUICK tiles of
// every stencil under small captions (none: labels.quick_empty).  The results show the hits
// the app found (none: labels.no_results_title / _text).  No stencil at all: an empty state
// (labels.empty_title / _text) with a link labels.more (StencilEvent::More).
// View modes (StencilState::view): SV_ICONS_NAMES tiles of 72 x 76 DIP (the preview on top,
// the name in up to two lines, cut with "…"), SV_ICONS 40 x 40 (the name in the tooltip only),
// SV_NAMES rows with the name, SV_DETAILS rows with the preview, the name and the QuickInfo.
// The tooltip of a tile is "name – QuickInfo".  Tiles without a preview show IC_SHAPES.
//
// Search: typing reports Search (the app recomputes `results`; the results become the active
// view, an empty query returns to the view before); Enter keeps the query in
// StencilState::history (newest first, at most 10, no duplicates); Down, Alt+Down or F4 in the
// box, or the button beside it, list the history (a pick searches again); Esc clears the query
// (in an empty box Esc gives up the focus).  The number of results is announced politely.
// Input: a click on a title activates it; Up / Down / Home / End move between the titles (the
// focused title is the active one).  A click on a tile -> Activate (the app arms click placing
// and marks the tile TL_ARMED), Enter / Space / a double click -> Place; a right click,
// Shift+F10 or the menu key -> TileMenu / StencilMenu with a point (window px) for
// ui.menu_popup_at -- or open the menu in the same pass with ui.menu_popup, which for the menu
// key opens at the widget with its first row focused.  The tiles are one Tab stop (entered at
// the armed tile): the arrows move in the grid, Home / End to the first / last tile.  "More
// shapes" (a click, Enter / Space, Alt+Down / F4) and the empty state's link -> More with the
// lower left corner of the button (the app opens its menu there with ui.menu_popup_at).
// Drags: every tile that is not disabled is a drag source (DragData::of_app(drag_kind, the
// stencil and tile key), the tile's name as its label, DE_COPY): DragStart, DragEnd.  An
// SD_EDITABLE stencil takes tile drags on its title row (appended) and, while it is shown, on
// its tile area (an accent mark shows where): Dropped with the index into its tiles to insert
// before (counted with the dragged tile still in place) and whether it went into the quick
// part; the app moves the tile when it came from the same stencil.
// Screen readers and scripts: the panel is a box named labels.panel; the title rows are tabs
// (K_TAB, selected = active: `click tab:"Allgemein"`, `expect tab:"Allgemein" selected`) --
// a vertical tab list is what the title rows are, and scripts and the platforms' screen
// readers know tabs with a selection; the tiles are buttons named by the tile (checked with
// TL_ARMED, disabled with TL_DISABLED, the QuickInfo as their description: `click
// button:"Rechteck"`, `drag button:"Rechteck" tab:"Favoriten"`); the quick collection's
// captions are headings; the search box is an edit named labels.search (`#ID+1`); the
// splitter is a slider named "Teiler" (F8 reaches it, the arrows move it).
// Only the rows of tiles in view (and those of the focused and the dragged tile) are laid out
// and drawn: a stencil of 300 tiles costs what a screenful costs.
//
// Ids: the panel `id`; the search box id + 1, the history button and its list id + 2 (its rows
// id + 0x10 + k), the splitter id + 4, the title view id + 5, the tile view id + 6, the empty
// state's link id + 7, the tile area's drop target id + 8; the titles id + 0x100 (quick),
// id + 0x101 (results), id + 0x102 + i (stencil i); the tiles id + 0x1000 + n (n: the place
// in the view).  StencilPanel::more_id is the app's own.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::{DrawRef, Sp, Ui};
use plat::{DragData, DE_COPY};

/// StencilTile::flags: a quick shape of its stencil (shown above the line, in the quick
/// collection)
pub const TL_QUICK: u32 = 1 << 20;
/// StencilTile::flags: armed for click placing (drawn checked)
pub const TL_ARMED: u32 = 1 << 21;
/// StencilTile::flags: disabled (no input, no drag)
pub const TL_DISABLED: u32 = ST_DISABLED;
/// StencilDef::flags: the stencil takes dropped tiles (favourites, "My shapes"): added and
/// reordered by the app on StencilEvent::Dropped
pub const SD_EDITABLE: u32 = 1 << 20;
/// StencilState::view: icons with the names below (the default)
pub const SV_ICONS_NAMES: u8 = 0;
/// StencilState::view: icons only (the name in the tooltip)
pub const SV_ICONS: u8 = 1;
/// StencilState::view: names only, one row each
pub const SV_NAMES: u8 = 2;
/// StencilState::view: rows with the icon, the name and the QuickInfo
pub const SV_DETAILS: u8 = 3;
/// StencilState::active: the quick shapes collection
pub const SK_QUICK: u32 = 0xFFFF_FFFE;
/// StencilState::active: the search results
pub const SK_SEARCH: u32 = 0xFFFF_FFFF;
// StencilState::active of a new state: no title yet (the first stencil is shown)
const SK_NONE: u32 = 0xFFFF_FFFD;

const QUERY_CAP: u32 = 256; // bytes of a query (with the asm's NUL)
const HISTORY_MAX: usize = 10;
const TITLE_H: i32 = 30; // DIP: a title row
const GRIP: i32 = 6; // DIP: the splitter
const TILES_MIN: i32 = 80; // DIP the splitter leaves for the tiles
// ids below the panel's
const P_SEARCH: u32 = 1;
const P_HISTORY: u32 = 2;
const P_GRIP: u32 = 4;
const P_TITLES: u32 = 5;
const P_TILES: u32 = 6;
const P_LINK: u32 = 7;
const P_DROP: u32 = 8;
const P_TITLE_GROUP: u32 = 9;
const P_TILE_GROUP: u32 = 10;
const P_HISTORY_ROW: u32 = 0x10;
const P_QUICK: u32 = 0x100;
const P_RESULTS: u32 = 0x101;
const P_STENCIL: u32 = 0x102;
const P_TILE: u32 = 0x1000;
// Item::t of a search hit (Item::s is then the index into the results)
const HIT: u32 = u32::MAX;
// Line::kind
const LN_TILES: u8 = 0;
const LN_CAPTION: u8 = 1;
const LN_SEP: u8 = 2;
const LN_HINT: u8 = 3;

/// a tile of a stencil (a master): what the panel shows of it
#[derive(Clone)]
pub struct StencilTile<'a> {
    /// the app's key for it (unique within its stencil; a drag and the events carry it)
    pub key: u32,
    /// its name (the label, the button's name for screen readers and scripts)
    pub name: &'a str,
    /// its QuickInfo: the tooltip after the name, the details view, the description
    pub tip: &'a str,
    /// its preview (None: a generic shapes icon)
    pub draw: Option<DrawRef>,
    /// the preview's version (cmd_custom: change it when the drawing changes)
    pub ver: u32,
    /// TL_QUICK, TL_ARMED, TL_DISABLED
    pub flags: u32,
}

impl<'a> StencilTile<'a> {
    /// a tile without a QuickInfo, a preview or flags
    pub const fn new(key: u32, name: &'a str) -> StencilTile<'a> {
        StencilTile { key, name, tip: "", draw: None, ver: 0, flags: 0 }
    }
    /// with a QuickInfo
    pub const fn tip(mut self, tip: &'a str) -> StencilTile<'a> {
        self.tip = tip;
        self
    }
    /// with a preview and its version
    pub fn preview(mut self, draw: Option<DrawRef>, ver: u32) -> StencilTile<'a> {
        self.draw = draw;
        self.ver = ver;
        self
    }
    /// add flags (TL_QUICK, TL_ARMED, TL_DISABLED)
    pub const fn flags(mut self, f: u32) -> StencilTile<'a> {
        self.flags |= f;
        self
    }
}

/// a stencil: its title row and its tiles
#[derive(Clone, Copy)]
pub struct StencilDef<'a> {
    /// the app's key (below 0xFFFF_FFF0: the SK_* keys are the panel's)
    pub key: u32,
    pub title: &'a str,
    pub tiles: &'a [StencilTile<'a>],
    /// SD_EDITABLE
    pub flags: u32,
}

/// a hit of the app's search: the tile and the stencil it belongs to
#[derive(Clone)]
pub struct StencilHit<'a> {
    pub stencil: u32,
    pub tile: StencilTile<'a>,
}

/// the texts of the panel (the app translates them; the panel has none of its own)
#[derive(Clone, Copy, Default)]
pub struct StencilLabels<'a> {
    /// the panel's name for screen readers ("Shapes")
    pub panel: &'a str,
    /// the search box's placeholder and name ("Shapes suchen")
    pub search: &'a str,
    /// the history button's tooltip and name ("Letzte Suchen")
    pub history: &'a str,
    /// the "More shapes" button and the empty state's link ("Weitere Shapes")
    pub more: &'a str,
    /// the quick collection's title ("Quick-Shapes")
    pub quick: &'a str,
    /// the results' title, "{}" = their number ("Suchergebnisse ({})"); also announced
    pub results: &'a str,
    /// the hint of an empty quick collection and of an empty quick part of an editable stencil
    /// ("Hier Quick-Shapes ablegen")
    pub quick_empty: &'a str,
    /// the hint of an editable stencil without other tiles ("Shapes hierher ziehen")
    pub drop_hint: &'a str,
    /// the empty state without stencils
    pub empty_title: &'a str,
    pub empty_text: &'a str,
    /// the results without a hit
    pub no_results_title: &'a str,
    pub no_results_text: &'a str,
}

/// what stencil_panel shows in a pass
#[derive(Clone, Copy)]
pub struct StencilPanel<'a> {
    pub stencils: &'a [StencilDef<'a>],
    /// the hits of StencilState::query: Some while the app searches (the query is not empty)
    pub results: Option<&'a [StencilHit<'a>]>,
    /// the app's own drag kind of a tile (DT_USER0 << k)
    pub drag_kind: u32,
    /// the widget id of the "More shapes" button (activated: StencilEvent::More)
    pub more_id: u32,
    pub labels: StencilLabels<'a>,
}

/// what the panel keeps between passes (the app owns it; the public fields are the app's to
/// read and set)
pub struct StencilState {
    /// the key of the active title: a stencil's, SK_QUICK or SK_SEARCH
    pub active: u32,
    /// SV_ICONS_NAMES, SV_ICONS, SV_NAMES, SV_DETAILS
    pub view: u8,
    /// the search box's text
    pub query: String,
    /// the recent searches, newest first (at most 10; the app may store and restore them)
    pub history: Vec<String>,
    /// the height of the title rows in DIP (the splitter; 0 = automatic: up to 40 % of the
    /// panel); the panel keeps it between one row and all of them
    pub split: i32,
    /// (JD15) the panel shows no search box (Visio's option "Show Shape Search pane" off); the
    /// query stays, set it to "" to end a search
    pub no_search: bool,
    // the view before the search took over
    prev: u32,
    // the view shown in the last pass (another one starts at its top)
    shown: u32,
    // the tile whose double click's release activates nothing
    dbl: u32,
    // the tile being dragged (widget id; 0 = none)
    drag: u32,
    // the number of results announced last (-1 = none)
    announced: i32,
    // the active view's tiles and rows (rebuilt every pass, the buffers kept)
    items: Vec<Item>,
    lines: Vec<Line>,
}

impl Default for StencilState {
    fn default() -> StencilState {
        StencilState {
            active: SK_NONE,
            view: SV_ICONS_NAMES,
            query: String::new(),
            history: Vec::new(),
            split: 0,
            no_search: false,
            prev: SK_NONE,
            shown: SK_NONE,
            dbl: 0,
            drag: 0,
            announced: -1,
            items: Vec::new(),
            lines: Vec::new(),
        }
    }
}

impl StencilState {
    /// a new state: the first stencil active, icons and names, no query
    pub fn new() -> StencilState {
        StencilState::default()
    }
}

/// what happened in the panel in a pass (stencil_panel returns them in order)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StencilEvent {
    /// a click on a tile: the app arms click placing (and marks the tile TL_ARMED)
    Activate { stencil: u32, tile: u32 },
    /// Enter / Space on a tile or a double click: place it in the middle of the view
    Place { stencil: u32, tile: u32 },
    /// a right click, Shift+F10 or the menu key on a tile: its menu at `at` (window px)
    TileMenu { stencil: u32, tile: u32, at: [i32; 2] },
    /// the same on a title row (SK_QUICK / SK_SEARCH for those two)
    StencilMenu { stencil: u32, at: [i32; 2] },
    /// StencilState::query changed: the app recomputes the results
    Search,
    /// a tile was dropped onto the SD_EDITABLE stencil `stencil`: insert it before its tile
    /// `before` (an index into its tiles, counted with a dragged tile of its own still in
    /// place; its length = at the end); `quick`: into the quick part
    Dropped { stencil: u32, before: usize, quick: bool, from_stencil: u32, from_tile: u32 },
    /// "More shapes" or the empty state's link: the app opens its menu at `at` (window px, the
    /// lower left corner of the button), ui.menu_popup_at
    More { at: [i32; 2] },
    /// a tile began to be dragged
    DragStart { stencil: u32, tile: u32 },
    /// that drag ended (dropped somewhere or cancelled; ui.drag_effect tells)
    DragEnd,
}

/// the stencil and tile keys of a tile drag's bytes (DragData::bytes of StencilPanel::
/// drag_kind), None when they are no tile drag's
pub fn stencil_drag_decode(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() != 8 {
        return None;
    }
    let s = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let t = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    Some((s, t))
}

// the bytes of a tile drag
fn drag_bytes(stencil: u32, tile: u32) -> [u8; 8] {
    let (s, t) = (stencil.to_le_bytes(), tile.to_le_bytes());
    [s[0], s[1], s[2], s[3], t[0], t[1], t[2], t[3]]
}

// keep a query in the history: newest first, no duplicates, at most HISTORY_MAX
fn remember(history: &mut Vec<String>, query: &str) {
    let q = query.trim();
    if q.is_empty() {
        return;
    }
    history.retain(|h| h != q);
    history.insert(0, String::from(q));
    history.truncate(HISTORY_MAX);
}

// a tile of the view
#[derive(Clone, Copy, Default)]
struct Item {
    // the stencil's index in StencilPanel::stencils (a hit: the index into the results)
    s: u32,
    // the tile's index in its stencil's tiles (HIT for a hit)
    t: u32,
    // its line
    line: u32,
    // in the quick part
    quick: bool,
}

// a row of the view (px from the top of the content)
#[derive(Clone, Copy, Default)]
struct Line {
    kind: u8,
    y: i32,
    h: i32,
    // LN_TILES: the first item; LN_CAPTION: the stencil's index; LN_HINT: 0 quick, 1 drop
    a: u32,
    // LN_TILES: the number of items
    n: u32,
}

// what the panel shows below the titles
#[derive(Clone, Copy, PartialEq)]
enum View {
    Quick,
    Search,
    Stencil(usize),
    Empty,
}

// the tile size and grid of a view mode (px)
#[derive(Clone, Copy)]
struct Metrics {
    tw: i32,
    th: i32,
    gap: i32,
    cols: usize,
    grid: bool,
}

fn metrics(ui: &Ui, view: u8, w: i32) -> Metrics {
    let (tw, th, gap, grid) = match view {
        SV_ICONS => (ui.dp(40), ui.dp(40), ui.dp(4), true),
        SV_NAMES => (w, ui.dp(28), 0, false),
        SV_DETAILS => (w, ui.dp(44), 0, false),
        _ => (ui.dp(72), ui.dp(76), ui.dp(4), true),
    };
    let cols = if grid { ((w + gap) / (tw + gap).max(1)).max(1) as usize } else { 1 };
    Metrics { tw, th, gap, cols, grid }
}

// where the tiles lie in this pass (px): the content's origin and width, the grid
struct Geo {
    x0: i32,
    y0: i32,
    w: i32,
    m: Metrics,
}

// the stencil key and the tile of an item
fn tile_of<'p, 'a>(p: &'p StencilPanel<'a>, it: Item) -> Option<(u32, &'p StencilTile<'a>)> {
    if it.t == HIT {
        let h = p.results?.get(it.s as usize)?;
        Some((h.stencil, &h.tile))
    } else {
        let s = p.stencils.get(it.s as usize)?;
        Some((s.key, s.tiles.get(it.t as usize)?))
    }
}

// the view that state and panel ask for (st.active follows: a title that is gone)
fn resolve(st: &mut StencilState, p: &StencilPanel<'_>) -> View {
    let has = !p.stencils.is_empty();
    if st.active == SK_SEARCH {
        // (a query typed in this pass: the app's results follow in the next one)
        if p.results.is_some() || !st.query.trim().is_empty() {
            return View::Search;
        }
        st.active = st.prev;
    }
    if st.active == SK_QUICK && has {
        return View::Quick;
    }
    if let Some(i) = p.stencils.iter().position(|s| s.key == st.active) {
        return View::Stencil(i);
    }
    if has {
        st.active = p.stencils[0].key;
        return View::Stencil(0);
    }
    if p.results.is_some() {
        st.active = SK_SEARCH;
        return View::Search;
    }
    View::Empty
}

// add the tiles of stencil (or results) s with these tile indices as rows of the grid
fn push_tiles(st: &mut StencilState, m: &Metrics, y: &mut i32, s: u32, tiles: impl Iterator<Item = u32>, quick: bool) {
    let start = st.items.len();
    st.items.extend(tiles.map(|t| Item { s, t, line: 0, quick }));
    let n = st.items.len() - start;
    let mut k = 0;
    while k < n {
        let c = m.cols.min(n - k);
        let li = st.lines.len() as u32;
        for it in &mut st.items[start + k..start + k + c] {
            it.line = li;
        }
        st.lines.push(Line { kind: LN_TILES, y: *y, h: m.th, a: (start + k) as u32, n: c as u32 });
        *y += m.th + m.gap;
        k += c;
    }
}

// a row that is no tile row
fn push_line(st: &mut StencilState, y: &mut i32, kind: u8, h: i32, a: u32) {
    st.lines.push(Line { kind, y: *y, h, a, n: 0 });
    *y += h;
}

impl Ui {
    /// The Shapes window of a diagram editor (see the module header): the search box, "More
    /// shapes", the title rows, a splitter and the tiles of the active view, in the size
    /// next_size gave (else it fills its box).  Returns what happened, in order.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn stencil_panel(&mut self, id: u32, st: &mut StencilState, p: &StencilPanel<'_>) -> Vec<StencilEvent> {
        let _src = self.src_scope();
        let mut out = Vec::new();
        let lb = p.labels;
        if self.wp.lay.next_w == 0 && self.wp.lay.next_h == 0 {
            self.next_size(-1, -1);
        }
        // a pressed tile that was double-clicked keeps its mark until it is released
        if st.dbl != 0 && self.wp.ctx.active != st.dbl {
            st.dbl = 0;
        }
        self.box_begin(id, BOX_V | BOX_CLIP | BOX_FIXED, 0);
        let panel = self.box_rect();
        self.wp.ctx.last_rect = panel;
        self.wp.ctx.last_state = 0;
        self.sem(K_BOX, id, Some(lb.panel), 0);

        // ---- the search box with its history, "More shapes"
        self.box_begin(0, BOX_V | pad(8), 8);
        if !st.no_search {
            self.stencil_search(id, st, p, &mut out);
        }
        let more = self.stencil_more(p, &mut out);
        self.box_end();

        // ---- the titles: the arrows first (the focused title is the active one)
        let has = !p.stencils.is_empty();
        let nq = has as usize;
        let nr = p.results.is_some() as usize;
        let n = nq + nr + p.stencils.len();
        let title_at = |k: usize| -> (u32, u32) {
            if k < nq {
                (id.wrapping_add(P_QUICK), SK_QUICK)
            } else if k < nq + nr {
                (id.wrapping_add(P_RESULTS), SK_SEARCH)
            } else {
                let i = k - nq - nr;
                (id.wrapping_add(P_STENCIL + i as u32), p.stencils[i].key)
            }
        };
        let f = self.wp.ctx.focus;
        if let Some(k) = (0..n).position(|k| title_at(k).0 == f) {
            let mut to = k;
            if self.kb_chord(VK_UP, 0) {
                to = k.saturating_sub(1);
            }
            if self.kb_chord(VK_DOWN, 0) {
                to = (k + 1).min(n - 1);
            }
            if self.kb_chord(VK_HOME, 0) {
                to = 0;
            }
            if self.kb_chord(VK_END, 0) {
                to = n - 1;
            }
            if to != k {
                let (tid, key) = title_at(to);
                st.active = key;
                self.wp.ctx.focus = tid;
                self.wp.ctx.focus_visible = true;
                self.repass();
            }
        }
        let view = resolve(st, p);
        if st.shown != st.active {
            st.shown = st.active;
            self.scroll_to(id.wrapping_add(P_TILES), -1, 0);
        }
        let results_label = p.results.map(|r| lb.results.replacen("{}", &alloc::format!("{}", r.len()), 1));
        match (p.results, &results_label) {
            (Some(r), Some(l)) => {
                if st.announced != r.len() as i32 {
                    st.announced = r.len() as i32;
                    self.announce(l, LIVE_POLITE);
                }
            }
            _ => st.announced = -1,
        }

        if n > 0 {
            // the height of the title rows: between one row and all of them, room left for the
            // tiles; 0 = up to 40 % of the panel
            let row_h = self.dp(TITLE_H);
            let all = row_h * n as i32;
            let top = more[1] + more[3] + self.dp(8);
            let room = panel[1] + panel[3] - top;
            let grip = self.dp(GRIP);
            let lo = row_h.min(all);
            let hi = all.min(room - grip - self.dp(TILES_MIN)).max(lo);
            let want = if st.split > 0 { self.dp(st.split) } else { all.min(room * 2 / 5) };
            let th = want.clamp(lo, hi);
            self.next_size(-1, self.px2dip(th));
            self.scroll_begin(id.wrapping_add(P_TITLES), SC_V);
            self.focus_group_begin(id.wrapping_add(P_TITLE_GROUP));
            for k in 0..n {
                let (tid, key) = title_at(k);
                let label: &str = if key == SK_QUICK {
                    lb.quick
                } else if key == SK_SEARCH {
                    results_label.as_deref().unwrap_or("")
                } else {
                    p.stencils[k - nq - nr].title
                };
                let def = (k >= nq + nr).then(|| p.stencils[k - nq - nr]);
                self.stencil_title(st, p, tid, key, label, def, k, &mut out);
            }
            self.focus_group_end();
            self.scroll_end();
            let titles = self.wp.ctx.last_rect;
            // ---- the splitter (the mouse, F8 and the arrows)
            self.next_size(-1, GRIP);
            let gr = self.place(grip, grip);
            let gid = id.wrapping_add(P_GRIP);
            let gev = self.behave(gid, HB_SPLITTER | CUR_SIZE_NS << HB_CURSOR_SHIFT);
            // (only input moves it: a height laid out a pixel off at 125 % is no reason)
            let d = self.splitter_keys(gev, false);
            let to = if d == i32::MIN / 2 {
                Some(lo)
            } else if d == i32::MAX / 2 {
                Some(hi)
            } else if d != 0 {
                Some(titles[3] + self.dp(d))
            } else if gev & EV_HELD != 0 {
                Some(self.mouse().1 - titles[1] - grip / 2)
            } else {
                None
            };
            if let Some(to) = to {
                let dip = self.px2dip(to.clamp(lo, hi)).max(1);
                if dip != st.split {
                    st.split = dip;
                    self.repass();
                }
            }
            let mut sp = Sp { ev: gev, flags: 1, ..Sp::default() };
            sp.place(gr);
            let sh = self.style.shell();
            sh.split_grip(self, &sp);
            self.focus_ring_draw(gev);
            let total = self.px2dip(room);
            let rec = self.sem(K_SLIDER, gid, Some(crate::i18n::tr("Teiler")), self.px2dip(titles[3]));
            rec.max = total;
        }

        // ---- the tiles of the view
        self.next_size(-1, -1);
        self.scroll_begin(id.wrapping_add(P_TILES), SC_V | pad(4));
        let b = self.box_rect();
        let geo = Geo { x0: b[0], y0: b[1], w: b[2], m: metrics(self, st.view, b[2]) };
        let total = self.stencil_build(st, p, view, &geo.m);
        let mut drag_seen = false;
        match view {
            View::Empty => {
                self.spacer(12);
                self.empty_state(IC_SHAPES, lb.empty_title, lb.empty_text);
                self.spacer(4);
                self.next_size(-1, 0);
                self.box_begin(0, BOX_H | BOX_CENTER, 0);
                let ev = self.link(id.wrapping_add(P_LINK), lb.more, TS_BODY);
                let r = self.wp.ctx.last_rect;
                self.box_end();
                if ev & EV_ACT != 0 {
                    out.push(StencilEvent::More { at: [r[0], r[1] + r[3]] });
                }
            }
            View::Search if st.items.is_empty() => {
                self.spacer(12);
                self.empty_state(IC_SEARCH, lb.no_results_title, lb.no_results_text);
            }
            View::Quick if st.items.is_empty() => {
                self.spacer(12);
                self.next_size(-1, 0);
                self.text(TS_BODY | TX_SECONDARY | TX_WRAP | TX_CENTER, lb.quick_empty);
            }
            _ => drag_seen = self.stencil_tiles(id, st, p, &geo, total, &mut out),
        }
        self.scroll_end();
        let area = self.wp.ctx.last_rect;
        // a drag whose tile is not laid out in this pass: its end still reaches the app
        if st.drag != 0 && !drag_seen {
            let r = self.drag_source(st.drag, 0);
            if r & DS_END != 0 || r == 0 {
                st.drag = 0;
                out.push(StencilEvent::DragEnd);
            }
        }
        // ---- an editable stencil takes tiles on its area
        if let View::Stencil(i) = view {
            let s = p.stencils[i];
            if s.flags & SD_EDITABLE != 0 {
                self.wp.ctx.last_rect = area;
                self.wp.ctx.last_state = 0;
                let ds = self.drop_target(id.wrapping_add(P_DROP), p.drag_kind);
                if ds & (DS_HOVER | DS_DROP) != 0 {
                    let (x, y) = if ds & DS_DROP != 0 {
                        self.drop_pos_px()
                    } else {
                        let (_, x, y) = self.drag_over_px();
                        (x, y)
                    };
                    let (before, quick, mark) = self.stencil_insert_at(st, &s, &geo, x, y);
                    if ds & DS_DROP != 0 {
                        if let Some((fs, ft)) = stencil_drag_decode(&self.drop_data().bytes) {
                            out.push(StencilEvent::Dropped { stencil: s.key, before, quick, from_stencil: fs, from_tile: ft });
                        }
                    } else {
                        let c = self.color(T_ACCENT);
                        self.draw_clip(area[0], area[1], area[2], area[3]);
                        self.draw_color(c);
                        self.draw_rect(mark[0], mark[1], mark[2], mark[3]);
                        self.draw_unclip();
                    }
                }
            }
        }
        self.box_end();
        self.wp.ctx.last_rect = panel;
        out
    }

    // the search box, the history button and its list
    fn stencil_search(&mut self, id: u32, st: &mut StencilState, p: &StencilPanel<'_>, out: &mut Vec<StencilEvent>) {
        let sid = id.wrapping_add(P_SEARCH);
        let hid = id.wrapping_add(P_HISTORY);
        let focused = self.wp.ctx.focus == sid;
        let mut changed = false;
        // Esc clears a query and keeps the focus; in an empty box it gives the focus up
        if focused && !st.query.is_empty() && self.popup_count() == 0 && self.capture_shortcut(VK_ESCAPE, 0) {
            st.query.clear();
            changed = true;
        }
        let mods = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let enter = focused && self.wp.ctx.keyq.iter().any(|&k| (k as i32) >= 0 && k & 0xFFFF == VK_RETURN && (k >> 16) & mods == 0);
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        self.next_size(-1, 0);
        let ev = self.textbox_icon(sid, IC_SEARCH, Some(p.labels.search), &mut st.query, QUERY_CAP);
        let boxr = self.wp.ctx.last_rect;
        // a text box's record is labelled with its text: name it
        if let Some(r) = self.sems.last_mut().filter(|r| r.id == sid) {
            r.name(p.labels.search);
        }
        if ev & EV_ACT != 0 {
            changed = true;
        }
        if ev & EV_COMMIT != 0 && enter {
            remember(&mut st.history, &st.query);
        }
        // the history: Down, Alt+Down or F4 in the box, or the button beside it
        let mut open = 0; // 1 by the keyboard, 2 by the pointer
        if ev & EV_FOCUS != 0 && !st.history.is_empty() && self.popup_count() == 0 && (self.kb_chord(VK_DOWN, 0) || self.open_key()) {
            open = 1;
        }
        if st.history.is_empty() {
            self.next_flags(ST_DISABLED);
        }
        let bev = self.icon_button(hid, IC_HISTORY, p.labels.history);
        if bev & EV_ACT != 0 && !st.history.is_empty() && !self.popup_is_open(hid) {
            open = if self.released() & (1 << BTN_LEFT) != 0 { 2 } else { 1 };
        }
        if open != 0 {
            self.popup_open(hid);
            if open == 1 {
                self.wp.ctx.focus_visible = true;
                self.pp_flag(hid, crate::popup::PPF_FOCUS);
            }
        }
        let lr = self.wp.ctx.last_rect;
        self.wp.ctx.last_rect = boxr; // the list hangs below the box, as wide as it
        if self.flyout_begin(hid, FL_BELOW | FL_MINW) {
            let mut pick = None;
            for (k, h) in st.history.iter().enumerate() {
                if self.menu_item(id.wrapping_add(P_HISTORY_ROW + k as u32), h) & EV_ACT != 0 {
                    pick = Some(k);
                }
            }
            if let Some(k) = pick {
                let h = st.history.remove(k);
                st.query.clone_from(&h);
                st.history.insert(0, h);
                changed = true;
            }
        }
        self.flyout_end();
        self.wp.ctx.last_rect = lr;
        self.box_end();
        if changed {
            // the results take over while there is a query; without one the view before returns
            if st.query.trim().is_empty() {
                if st.active == SK_SEARCH {
                    st.active = st.prev;
                }
            } else if st.active != SK_SEARCH {
                st.prev = st.active;
                st.active = SK_SEARCH;
            }
            out.push(StencilEvent::Search);
            self.repass();
        }
    }

    // the "More shapes" button (a drop-down's look; the app opens its menu) -> its rectangle
    fn stencil_more(&mut self, p: &StencilPanel<'_>, out: &mut Vec<StencilEvent>) -> [i32; 4] {
        let style = self.style;
        let mut sp = Sp { label: Some(p.labels.more), ..Sp::default() };
        let (w, h) = style.dropdown_size(self, &mut sp);
        let (w, h) = self.ctrl_size(w, h, 0, false);
        self.next_size(-1, 0);
        self.place(w, h);
        let ev = self.behave(p.more_id, HB_FOCUS);
        let state = self.wp.ctx.last_state;
        let r = self.wp.ctx.last_rect;
        let key = ev & EV_FOCUS != 0 && self.open_key();
        if ev & EV_ACT != 0 || key {
            if key {
                self.wp.ctx.focus_visible = true;
            }
            out.push(StencilEvent::More { at: [r[0], r[1] + r[3]] });
        }
        sp.place(r);
        sp.ev = ev;
        sp.value2 = (state == SS_DISABLED as u32) as i32;
        style.dropdown(self, &sp);
        self.focus_ring_draw(ev);
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state;
        self.sem(K_BUTTON, p.more_id, Some(p.labels.more), 0).states(AS_HASPOPUP);
        r
    }

    // a title row: a tab; an editable stencil's takes tile drags
    #[allow(clippy::too_many_arguments)]
    fn stencil_title(&mut self, st: &mut StencilState, p: &StencilPanel<'_>, tid: u32, key: u32, label: &str, def: Option<StencilDef<'_>>, k: usize, out: &mut Vec<StencilEvent>) {
        let row_h = self.dp(TITLE_H);
        let r = self.place(0, row_h);
        let active = st.active == key;
        let ev = self.behave(tid, HB_FOCUS | if active { HB_CURRENT } else { 0 });
        let state = self.wp.ctx.last_state;
        if ev & EV_ACT != 0 && st.active != key {
            st.active = key;
            self.repass();
        }
        if ev & EV_RCLICK != 0 {
            out.push(StencilEvent::StencilMenu { stencil: key, at: self.stencil_menu_at() });
        }
        if let Some(s) = def.filter(|s| s.flags & SD_EDITABLE != 0) {
            if self.drop_target(tid, p.drag_kind) & DS_DROP != 0 {
                if let Some((fs, ft)) = stencil_drag_decode(&self.drop_data().bytes) {
                    out.push(StencilEvent::Dropped { stencil: s.key, before: s.tiles.len(), quick: false, from_stencil: fs, from_tile: ft });
                }
            }
        }
        let active = st.active == key;
        let ring = self.focus_ring(ev);
        self.ts_font(TS_BODY);
        let room = r[2] - self.dp(28);
        let shown = self.stencil_fit(label, room);
        let cut = shown.len() != label.len();
        let mut sp = Sp { ev, value: active as i32, label: Some(&*shown), flags: if ring && active { 2 } else { 0 }, ..Sp::default() };
        sp.place(r);
        let style = self.style;
        style.list_row(self, &sp);
        if ring && !active {
            style.focus_rect(self, r[0] + self.dp(2), r[1], r[2] - self.dp(4), r[3]);
        }
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state | if active { SS_SELECTED as u32 } else { 0 };
        self.sem(K_TAB, tid, Some(label), k as i32);
        if cut {
            self.tooltip(label);
        }
    }

    // the point of a menu for the widget that got EV_RCLICK in this pass: the menu key's at the
    // widget's lower left corner, a click's at the pointer (window px)
    fn stencil_menu_at(&self) -> [i32; 2] {
        match self.kb_ctx_rect() {
            Some(r) => [r[0], r[1] + r[3]],
            None => {
                let (x, y) = self.mouse();
                [x, y]
            }
        }
    }

    // the items and lines of the view -> the content's height (px)
    fn stencil_build(&mut self, st: &mut StencilState, p: &StencilPanel<'_>, view: View, m: &Metrics) -> i32 {
        st.items.clear();
        st.lines.clear();
        let mut y = 0;
        let caption = self.dp(24);
        let sep = self.dp(9);
        let hint = self.dp(28);
        match view {
            View::Stencil(i) => {
                let s = &p.stencils[i];
                let ed = s.flags & SD_EDITABLE != 0;
                let nq = s.tiles.iter().filter(|t| t.flags & TL_QUICK != 0).count();
                let nr = s.tiles.len() - nq;
                let quick = (0..s.tiles.len() as u32).filter(|&t| s.tiles[t as usize].flags & TL_QUICK != 0);
                if nq > 0 {
                    push_tiles(st, m, &mut y, i as u32, quick, true);
                } else if ed {
                    push_line(st, &mut y, LN_HINT, hint, 0);
                }
                if (nq > 0 || ed) && (nr > 0 || ed) {
                    push_line(st, &mut y, LN_SEP, sep, 0);
                }
                let rest = (0..s.tiles.len() as u32).filter(|&t| s.tiles[t as usize].flags & TL_QUICK == 0);
                if nr > 0 {
                    push_tiles(st, m, &mut y, i as u32, rest, false);
                } else if ed {
                    push_line(st, &mut y, LN_HINT, hint, 1);
                }
            }
            View::Quick => {
                for (i, s) in p.stencils.iter().enumerate() {
                    if !s.tiles.iter().any(|t| t.flags & TL_QUICK != 0) {
                        continue;
                    }
                    push_line(st, &mut y, LN_CAPTION, caption, i as u32);
                    let quick = (0..s.tiles.len() as u32).filter(|&t| s.tiles[t as usize].flags & TL_QUICK != 0);
                    push_tiles(st, m, &mut y, i as u32, quick, true);
                }
            }
            View::Search => {
                let n = p.results.map_or(0, |r| r.len()) as u32;
                let start = st.items.len();
                push_tiles(st, m, &mut y, 0, 0..n, false);
                // a hit's item: s = its index into the results
                for (k, it) in st.items[start..].iter_mut().enumerate() {
                    it.s = k as u32;
                    it.t = HIT;
                }
            }
            View::Empty => {}
        }
        y
    }

    // the keys on a focused tile, then the rows in view (and those of the focused and the
    // dragged tile) -> the dragged tile was laid out
    fn stencil_tiles(&mut self, id: u32, st: &mut StencilState, p: &StencilPanel<'_>, geo: &Geo, total: i32, out: &mut Vec<StencilEvent>) -> bool {
        let base = id.wrapping_add(P_TILE);
        let n = st.items.len();
        let m = geo.m;
        // the arrows move in the grid: Left / Right to the previous / next tile, Up / Down to
        // the nearest column of the row above / below, Home / End to the ends
        let f = self.wp.ctx.focus.wrapping_sub(base) as usize;
        if f < n {
            let mut to = f;
            let line = st.items[f].line as usize;
            let col = f - st.lines[line].a as usize;
            let row = |ui: &mut Ui, vk: u32, up: bool| -> Option<usize> {
                if !ui.kb_chord(vk, 0) {
                    return None;
                }
                let mut l = line;
                loop {
                    l = if up { l.checked_sub(1)? } else { l + 1 };
                    let ln = *st.lines.get(l)?;
                    if ln.kind == LN_TILES {
                        return Some(ln.a as usize + col.min(ln.n as usize - 1));
                    }
                }
            };
            if self.kb_chord(VK_LEFT, 0) {
                to = f.saturating_sub(1);
            }
            if self.kb_chord(VK_RIGHT, 0) {
                to = (f + 1).min(n - 1);
            }
            if let Some(t) = row(self, VK_UP, true) {
                to = t;
            }
            if let Some(t) = row(self, VK_DOWN, false) {
                to = t;
            }
            if self.kb_chord(VK_HOME, 0) {
                to = 0;
            }
            if self.kb_chord(VK_END, 0) {
                to = n - 1;
            }
            if to != f {
                self.wp.ctx.focus = base.wrapping_add(to as u32);
                self.wp.ctx.focus_visible = true;
                self.repass();
            }
        }
        let line_of = |w: u32| -> Option<usize> {
            let k = w.wrapping_sub(base) as usize;
            (w != 0 && k < n).then(|| st.items[k].line as usize)
        };
        let fl = line_of(self.wp.ctx.focus);
        let dl = line_of(st.drag);
        let clip = self.wp.ctx.uiclip;
        let over = m.th + m.gap;
        let mut cur = 0;
        let mut drag_seen = false;
        self.focus_group_begin(id.wrapping_add(P_TILE_GROUP));
        for li in 0..st.lines.len() {
            let ln = st.lines[li];
            let top = geo.y0 + ln.y;
            let seen = top + ln.h > clip[1] - over && top < clip[3] + over;
            if !seen && fl != Some(li) && dl != Some(li) {
                continue;
            }
            if ln.y > cur {
                self.place(0, ln.y - cur);
            }
            let r = self.place(0, ln.h);
            cur = ln.y + ln.h;
            match ln.kind {
                LN_TILES => {
                    for k in 0..ln.n as usize {
                        let pos = ln.a as usize + k;
                        let tr = if m.grid { [r[0] + k as i32 * (m.tw + m.gap), r[1], m.tw, m.th] } else { [r[0], r[1], r[2], m.th] };
                        if self.stencil_tile(id, st, p, pos, tr, &m, out) {
                            drag_seen = true;
                        }
                    }
                }
                LN_CAPTION => {
                    let title = p.stencils.get(ln.a as usize).map_or("", |s| s.title);
                    self.ts_font(TS_CAPTION);
                    let shown = self.stencil_fit(title, r[2] - self.dp(8));
                    let line = self.ts_line(TS_CAPTION);
                    let y = r[1] + r[3] - line - self.dp(2) + self.ts_base(TS_CAPTION);
                    let c = self.color(T_TEXT2);
                    self.emit_text(r[0] + self.dp(4), y, &shown, c);
                    self.wp.ctx.last_rect = r;
                    self.wp.ctx.last_state = 0;
                    self.sem(K_HEADING, 0, Some(title), 0);
                }
                LN_SEP => {
                    let h = self.hair();
                    let c = self.color(T_DIVIDER);
                    self.draw_color(c);
                    self.draw_rect(r[0] + self.dp(4), r[1] + (r[3] - h) / 2, (r[2] - self.dp(8)).max(0), h);
                }
                _ => {
                    let text = if ln.a == 0 { p.labels.quick_empty } else { p.labels.drop_hint };
                    self.ts_font(TS_CAPTION);
                    let shown = self.stencil_fit(text, r[2] - self.dp(8));
                    let w = self.text.width(&shown);
                    let line = self.ts_line(TS_CAPTION);
                    let y = r[1] + (r[3] - line) / 2 + self.ts_base(TS_CAPTION);
                    let c = self.color(T_TEXT3);
                    self.emit_text(r[0] + (r[2] - w) / 2, y, &shown, c);
                    self.wp.ctx.last_rect = r;
                    self.wp.ctx.last_state = 0;
                    self.sem(K_TEXT, 0, Some(text), 0);
                }
            }
        }
        self.focus_group_end();
        if total > cur {
            self.place(0, total - cur); // the rows below the view
        }
        drag_seen
    }

    // a tile: input, its drag, its look, its record -> it is the dragged one
    #[allow(clippy::too_many_arguments)]
    fn stencil_tile(&mut self, id: u32, st: &mut StencilState, p: &StencilPanel<'_>, pos: usize, r: [i32; 4], m: &Metrics, out: &mut Vec<StencilEvent>) -> bool {
        let Some((skey, tile)) = tile_of(p, st.items[pos]) else { return false };
        let tid = id.wrapping_add(P_TILE).wrapping_add(pos as u32);
        let off = tile.flags & TL_DISABLED != 0;
        let armed = tile.flags & TL_ARMED != 0;
        self.wp.ctx.last_rect = r;
        let hb = HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT | if armed { HB_CURRENT } else { 0 } | if off { HB_DISABLED } else { 0 };
        let ev = self.behave(tid, hb);
        let state = self.wp.ctx.last_state;
        let (stencil, key) = (skey, tile.key);
        if ev & EV_DOUBLE != 0 {
            st.dbl = tid; // its release activates nothing
            out.push(StencilEvent::Place { stencil, tile: key });
        }
        if ev & EV_ACT != 0 {
            if self.released() & (1 << BTN_LEFT) == 0 {
                out.push(StencilEvent::Place { stencil, tile: key }); // Enter / Space, a screen reader
            } else if st.dbl == tid {
                st.dbl = 0;
            } else {
                out.push(StencilEvent::Activate { stencil, tile: key });
            }
        }
        if ev & EV_RCLICK != 0 {
            out.push(StencilEvent::TileMenu { stencil, tile: key, at: self.stencil_menu_at() });
        }
        // the drag
        let mut dragged = false;
        match self.drag_source(tid, ev) {
            DS_START if !off => {
                let mut d = DragData::of_app(p.drag_kind, &drag_bytes(stencil, key));
                d.label = String::from(tile.name);
                d.effects = DE_COPY;
                self.drag_set(d);
                st.drag = tid;
                st.dbl = 0;
                dragged = true;
                out.push(StencilEvent::DragStart { stencil, tile: key });
            }
            DS_END => {
                if st.drag == tid {
                    st.drag = 0;
                    out.push(StencilEvent::DragEnd);
                }
                dragged = true;
                self.repass();
            }
            DS_ACTIVE => dragged = st.drag == tid,
            _ => {}
        }
        // the look: the style's subtle button (checked while armed), the preview, the text
        let mut sp = Sp { flags: ST_SUBTLE | if armed { ST_CHECKED } else { 0 }, ..Sp::default() };
        self.sp_placed(&mut sp, ev);
        let style = self.style;
        style.button(self, &sp);
        let ink = self.color(if off { T_TEXT_DISABLED } else { T_TEXT });
        match (m.grid, m.tw >= self.dp(72)) {
            (true, true) => {
                // icons and names: the preview on top, the name in up to two lines below
                let pw = (r[2] - self.dp(8)).min(self.dp(48));
                let ph = self.dp(36);
                let py = r[1] + self.dp(4);
                self.stencil_preview(tile, [r[0] + (r[2] - pw) / 2, py, pw, ph]);
                self.ts_font(TS_CAPTION);
                let line = self.ts_line(TS_CAPTION);
                let mut y = py + ph + self.dp(2) + self.ts_base(TS_CAPTION);
                let (a, b) = self.stencil_two_lines(tile.name, r[2] - self.dp(4));
                for l in [Some(a), b].into_iter().flatten() {
                    let w = self.text.width(&l);
                    self.emit_text(r[0] + (r[2] - w) / 2, y, &l, ink);
                    y += line;
                }
            }
            (true, false) => {
                // icons only
                let s = self.dp(32).min(r[2]).min(r[3]);
                self.stencil_preview(tile, [r[0] + (r[2] - s) / 2, r[1] + (r[3] - s) / 2, s, s]);
            }
            (false, _) if r[3] < self.dp(40) => {
                // names only
                self.ts_font(TS_BODY);
                let shown = self.stencil_fit(tile.name, r[2] - self.dp(16));
                let y = r[1] + (r[3] - self.ts_line(TS_BODY)) / 2 + self.ts_base(TS_BODY);
                self.emit_text(r[0] + self.dp(8), y, &shown, ink);
            }
            (false, _) => {
                // details: the preview at the left, the name and the QuickInfo beside it
                let s = self.dp(32);
                self.stencil_preview(tile, [r[0] + self.dp(6), r[1] + (r[3] - s) / 2, s, s]);
                let x = r[0] + self.dp(46);
                let room = r[2] - self.dp(52);
                let (lb, lc) = (self.ts_line(TS_BODY), self.ts_line(TS_CAPTION));
                let tall = if tile.tip.is_empty() { lb } else { lb + lc };
                let y0 = r[1] + (r[3] - tall) / 2;
                self.ts_font(TS_BODY);
                let shown = self.stencil_fit(tile.name, room);
                let base = self.ts_base(TS_BODY);
                self.emit_text(x, y0 + base, &shown, ink);
                if !tile.tip.is_empty() {
                    self.ts_font(TS_CAPTION);
                    let shown = self.stencil_fit(tile.tip, room);
                    let base = self.ts_base(TS_CAPTION);
                    let c = self.color(if off { T_TEXT_DISABLED } else { T_TEXT2 });
                    self.emit_text(x, y0 + lb + base, &shown, c);
                }
            }
        }
        self.wp.ctx.last_rect = r;
        self.focus_ring_draw(ev);
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state | if armed { SS_CHECKED as u32 } else { 0 };
        let rec = self.sem(K_BUTTON, tid, Some(tile.name), pos as i32);
        if !tile.tip.is_empty() {
            rec.desc(tile.tip);
        }
        // the tooltip: name – QuickInfo (put together only under the pointer)
        if ev & EV_HOVER != 0 && !tile.tip.is_empty() {
            let tip = alloc::format!("{} – {}", tile.name, tile.tip);
            self.tooltip(&tip);
        } else {
            self.tooltip(tile.name);
        }
        dragged
    }

    // a tile's preview in r (none: the shapes icon)
    fn stencil_preview(&mut self, tile: &StencilTile<'_>, r: [i32; 4]) {
        if r[2] <= 0 || r[3] <= 0 {
            return;
        }
        match &tile.draw {
            Some(d) => self.cmd_custom(r[0], r[1], r[2], r[3], Some(d.clone()), 0, tile.ver),
            None => {
                let s = r[2].min(r[3]).min(self.dp(24));
                let c = self.color(T_TEXT3);
                crate::stock::icon_in(self, IC_SHAPES, r, s, c);
            }
        }
    }

    // s cut with "…" to w px in the current font (borrowed when it fits)
    fn stencil_fit<'s>(&mut self, s: &'s str, w: i32) -> Cow<'s, str> {
        if self.text.width(s) <= w {
            Cow::Borrowed(s)
        } else {
            Cow::Owned(self.ellipsize(s.as_bytes(), w.max(0)).0)
        }
    }

    // a name in at most two lines of w px (the current font): split at the last space, hyphen
    // or slash that fits, the rest cut with "…"; when even the first word is too long, at the
    // first such place with both lines cut; without one: one line, cut
    fn stencil_two_lines<'s>(&mut self, s: &'s str, w: i32) -> (Cow<'s, str>, Option<Cow<'s, str>>) {
        if self.text.width(s) <= w {
            return (Cow::Borrowed(s), None);
        }
        let (mut best, mut first) = (None, None);
        for (k, ch) in s.char_indices() {
            if ch == ' ' || ch == '-' || ch == '/' {
                let end = k + ch.len_utf8();
                let head = if ch == ' ' { &s[..k] } else { &s[..end] };
                let split = (head, s[end..].trim_start());
                if head.is_empty() || split.1.is_empty() {
                    continue;
                }
                first = first.or(Some(split));
                if self.text.width(head) > w {
                    break;
                }
                best = Some(split);
            }
        }
        match best.or(first) {
            Some((a, b)) => {
                let a = self.stencil_fit(a, w);
                let b = self.stencil_fit(b, w);
                (a, Some(b))
            }
            None => (self.stencil_fit(s, w), None),
        }
    }

    // where a tile dropped at (x, y) px goes in the editable stencil s: (the index to insert
    // before, into the quick part, the mark to draw)
    fn stencil_insert_at(&self, st: &StencilState, s: &StencilDef<'_>, geo: &Geo, x: i32, y: i32) -> (usize, bool, [i32; 4]) {
        let m = geo.m;
        let bar = self.dp(2);
        let hline = |yy: i32| [geo.x0, yy - bar / 2, geo.w, bar];
        let n = st.lines.len();
        for (li, ln) in st.lines.iter().enumerate() {
            let top = geo.y0 + ln.y;
            let bottom = top + ln.h + if ln.kind == LN_TILES { m.gap } else { 0 };
            let last = li + 1 == n;
            if y >= bottom && !last {
                continue;
            }
            match ln.kind {
                LN_TILES => {
                    let k = if y >= bottom {
                        ln.n // below the last row: after its last tile
                    } else if m.grid {
                        let step = (m.tw + m.gap).max(1);
                        ((x - geo.x0 + step / 2) / step).clamp(0, ln.n as i32) as u32
                    } else {
                        (y >= top + ln.h / 2) as u32
                    };
                    let v = (ln.a + k) as usize;
                    let (before, quick) = if k < ln.n {
                        let it = st.items[v];
                        (it.t as usize, it.quick)
                    } else {
                        let it = st.items[v - 1];
                        (it.t as usize + 1, it.quick)
                    };
                    let mark = if m.grid {
                        let mx = geo.x0 + k as i32 * (m.tw + m.gap) - m.gap / 2;
                        [mx - bar / 2, top, bar, ln.h]
                    } else {
                        hline(if k == 0 { top } else { top + ln.h })
                    };
                    return (before, quick, mark);
                }
                LN_HINT => {
                    return if ln.a == 0 { (0, true, hline(top + ln.h / 2)) } else { (s.tiles.len(), false, hline(top + ln.h / 2)) };
                }
                LN_SEP => {
                    return if y < top + ln.h / 2 {
                        let at = st.items.iter().rev().find(|i| i.quick).map_or(0, |i| i.t as usize + 1);
                        (at, true, hline(top + ln.h / 2))
                    } else {
                        let at = st.items.iter().find(|i| !i.quick).map_or(s.tiles.len(), |i| i.t as usize);
                        (at, false, hline(top + ln.h / 2))
                    };
                }
                _ => {}
            }
        }
        (s.tiles.len(), false, hline(geo.y0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_bytes_round_trip() {
        assert_eq!(stencil_drag_decode(&drag_bytes(7, 0x1234_5678)), Some((7, 0x1234_5678)));
        assert_eq!(stencil_drag_decode(&[1, 2, 3]), None);
    }

    #[test]
    fn history_newest_first_no_duplicates_at_most_ten() {
        let mut h: Vec<String> = Vec::new();
        remember(&mut h, "  pfeil ");
        remember(&mut h, "kreis");
        remember(&mut h, "pfeil");
        remember(&mut h, "   ");
        assert_eq!(h, ["pfeil", "kreis"]);
        for k in 0..12 {
            remember(&mut h, &alloc::format!("q{}", k));
        }
        assert_eq!(h.len(), 10);
        assert_eq!(h[0], "q11");
    }
}
