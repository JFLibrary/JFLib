// palette -- the Geräte panel: the catalog by category, a search box, tiles to place devices
//
//   Palette::new(&symbols)      the expanders' states and one drawing per catalog model
//   frame(n, ui)                the panel's content (a dock panel on the left)
//
// A tile shows the device's symbol and its model name (tooltip: title and ports).  Clicking it
// arms the canvas for placing (the next click there puts the device; Shift keeps it armed),
// Enter puts the device into the middle of the view, dragging the tile onto the canvas places
// it where it is dropped (DT_DEVICE with the catalog id).  The note tile is the Notiz tool.
// The search matches model names and descriptions; while it is used every category with a
// hit is open.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::{self, Category, Model};
use ui::*;

use crate::app::constants::*;
use crate::app::Net;
use crate::workspace::canvas::tools::{self, Tool};
use crate::workspace::icons::{tint_of, Symbols};

/// a tile (DIP) and the symbol in it
const TILE_W: i32 = 68;
const TILE_H: i32 = 78;
const SYM_W: i32 = 48;
const TILE_GAP: i32 = 4;

pub(crate) struct Palette {
    pub search: String,
    /// expander states per category (Category::ALL order)
    pub open: [i32; 7],
    /// the symbol drawing of every catalog model (render time)
    tiles: Vec<Option<DrawRef>>,
    /// the catalog model whose tile is being dragged
    pub dragging: Option<usize>,
}

/// a model's symbol in a tile (render time)
struct TileSym {
    syms: Rc<Symbols>,
    sym: usize,
    tint: usize,
}

impl Draw for TileSym {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let h = self.syms.height(self.sym, r[2]);
        self.syms.draw(g, self.sym, r[0], r[1] + (r[3] - h) / 2, r[2], self.tint, false);
    }
}

impl Palette {
    pub(crate) fn new(syms: &Rc<Symbols>) -> Palette {
        let tiles = catalog::catalog()
            .iter()
            .map(|m| syms.index(m.symbol).map(|i| -> DrawRef { Rc::new(TileSym { syms: syms.clone(), sym: i, tint: tint_of(m.category) }) }))
            .collect();
        Palette { search: String::new(), open: [1, 1, 0, 0, 0, 0, 1], tiles, dragging: None }
    }

    /// the symbol drawing of catalog model i (render time; the inspector's head uses it too)
    pub(crate) fn tile_draw(&self, i: usize) -> Option<DrawRef> {
        self.tiles.get(i).cloned().flatten()
    }
}

// the model matches the search (case-insensitive, name or title)
fn matches(m: &Model, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty() || m.name.to_lowercase().contains(&q) || m.title.to_lowercase().contains(&q) || m.id.contains(&q)
}

/// the Geräte panel
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    ui.box_begin(0, BOX_V | pad(8), 8);
    ui.next_size(-1, 0);
    ui.textbox_icon(ID_SEARCH, IC_SEARCH, Some("Geräte suchen"), &mut n.palette.search, 64);
    ui.box_end();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_PALETTE, SC_V | pad(6));
    let q = n.palette.search.clone();
    let searching = !q.trim().is_empty();
    let all = catalog::catalog();
    let mut any = false;
    for (k, &cat) in Category::ALL.iter().enumerate() {
        let models: Vec<usize> = (0..all.len()).filter(|&i| all[i].category == cat && matches(&all[i], &q)).collect();
        if models.is_empty() {
            continue;
        }
        any = true;
        let title = format!("{} ({})", cat.title(), models.len());
        let mut open = if searching { 1 } else { n.palette.open[k] };
        let shown = ui.expander_begin(ID_CATEGORY + k as u32, &title, &mut open) != 0;
        if !searching {
            n.palette.open[k] = open;
        }
        if shown {
            // rows of as many tiles as fit
            let gap = ui.dp(TILE_GAP);
            let cols = ((ui.box_rect()[2] + gap) / (ui.dp(TILE_W) + gap)).max(1) as usize;
            ui.box_begin(0, BOX_V, TILE_GAP);
            for row in models.chunks(cols) {
                ui.box_begin(0, BOX_H, TILE_GAP);
                for &i in row {
                    tile(n, ui, i);
                }
                ui.box_end();
            }
            ui.box_end();
        }
        ui.expander_end();
    }
    if !any {
        ui.empty_state(IC_SEARCH, "Kein Gerät gefunden", "Die Suche prüft Modellnamen und Beschreibungen.");
    }
    ui.scroll_end();
}

// a tile: the symbol, the name in one or two lines; click arms it, a drag places it
fn tile(n: &mut Net, ui: &mut Ui, i: usize) {
    let m = &catalog::catalog()[i];
    let id = ID_TILE + i as u32;
    let armed = match n.canvas.tool {
        Tool::Place(k) => k == i,
        Tool::Note => m.has(catalog::F_ANNOTATION),
        _ => false,
    };
    let r = ui.place(ui.dp(TILE_W), ui.dp(TILE_H));
    let ev = ui.behave(id, HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT);
    let state = if ev & EV_HOVER == 0 {
        0
    } else if ev & EV_HELD == 0 {
        1
    } else {
        2
    };
    let st = ui.style;
    let sp = Sp { x: r[0], y: r[1], w: r[2], h: r[3], state, ev, flags: ST_SUBTLE | if armed { ST_CHECKED } else { 0 }, ..Sp::default() };
    st.button(ui, &sp);
    // the symbol
    let sw = ui.dp(SYM_W);
    let sh = sw * 3 / 4;
    let top = r[1] + ui.dp(6);
    if let Some(d) = n.palette.tile_draw(i) {
        let ver = n.syms.key() ^ (i as u32).wrapping_mul(0x9E37_79B9);
        ui.cmd_custom(r[0] + (r[2] - sw) / 2, top, sw, sh, Some(d), 0, ver);
    }
    // the name: one line, or two at a space, hyphen or slash
    ui.ts_font(TS_CAPTION);
    let room = r[2] - ui.dp(2);
    let lines = two_lines(ui, m.name, room);
    let line_h = ui.text.ascent() + ui.text.descent();
    let mut y = top + sh + ui.dp(5) + ui.text.ascent();
    let c = ui.color(T_TEXT);
    for l in &lines {
        let w = ui.text.width(l);
        ui.emit_text(r[0] + (r[2] - w) / 2, y, l, c);
        y += line_h;
    }
    if ui.focus_ring(ev) {
        st.focus_ring(ui, r, 4);
    }
    let s = ui.last_state();
    ui.last_set(r, s | if armed { SS_CHECKED as u32 } else { 0 });
    ui.sem(K_BUTTON, id, Some(m.name), 0);
    ui.tooltip(&if m.groups.is_empty() { String::from(m.title) } else { format!("{} · Ports: {}", m.title, m.summary()) });
    if ev & EV_ACT != 0 {
        if ui.released() & (1 << BTN_LEFT) == 0 {
            tools::place_centre(n, ui, i); // Enter / Space
        } else {
            tools::arm(n, i);
        }
        ui.repass();
    }
    match ui.drag_source(id, ev) {
        DS_START => {
            let mut d = DragData::of_app(DT_DEVICE, m.id.as_bytes());
            d.label = String::from(m.name);
            d.effects = DE_COPY;
            ui.drag_set(d);
            n.palette.dragging = Some(i);
        }
        DS_END => {
            n.palette.dragging = None;
            ui.repass();
        }
        _ => {}
    }
}

// the name in at most two lines of at most w px (the current font): split at the last space,
// hyphen or slash that fits, the rest cut with "…"
fn two_lines(ui: &mut Ui, s: &str, w: i32) -> Vec<String> {
    if ui.text.width(s) <= w {
        return alloc::vec![String::from(s)];
    }
    let mut best = None;
    for (k, ch) in s.char_indices() {
        if ch == ' ' || ch == '-' || ch == '/' {
            let head = if ch == ' ' { &s[..k] } else { &s[..k + 1] };
            if ui.text.width(head) <= w {
                best = Some((head, s[k + 1..].trim_start()));
            }
        }
    }
    match best {
        Some((a, b)) => alloc::vec![String::from(a), ellipsize(ui, b, w)],
        None => alloc::vec![ellipsize(ui, s, w)],
    }
}

fn ellipsize(ui: &mut Ui, s: &str, w: i32) -> String {
    if ui.text.width(s) <= w {
        return String::from(s);
    }
    let mut out = String::from(s);
    while !out.is_empty() {
        out.pop();
        let cand = format!("{}…", out.trim_end());
        if ui.text.width(&cand) <= w {
            return cand;
        }
    }
    String::from("…")
}
