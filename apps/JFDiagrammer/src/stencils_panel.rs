// stencils_panel -- JD9: the Shapes panel's content: the library's stencil panel (ui.stencil_panel)
// fed with the open stencils, the search hits and the masters' previews; its events; the
// context menus of tiles and stencil titles; "Weitere Shapes"; the file dialog of "Schablone
// öffnen"
//
//   panel(app, ui)              the panel (panes.rs P_SHAPES)
//   Previews                    the previews of a stencil's masters: one page with an instance
//                               of each master (a copy of the stencil document), drawn by
//                               diagram_view; each tile draws its instance's box (Draw)
//   TILE_MENU(_OWN), TITLE_MENU the context menus: a tile's rows act on Stencils::menu_tile
//                               (stencils::command), a title's commands on the stencil whose title
//                               was clicked (it becomes the active one)
//
// The previews are the shapes themselves (SH-G03: "aus dem Shape gerendert"), in the
// document's colours, the same for the built-in stencils, the user's own and those of other
// programs; the icons inside the .vssx are for Visio.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use diagram_core::render::Options;
use diagram_core::{CellAddr, Document, Owner, Page, PageHeader, ShapeRef, SheetRef};
use diagram_view::{FontSet, PageView};
use render2d::Camera;
use ui::i18n::{self, t, tf};
use ui::*;

use crate::commands::*;
use crate::stencils::{self, Loaded};
use crate::stencils_drop::DT_SHAPE;
use crate::Diagrammer;

const ID_PANEL: u32 = 0x6000;
const ID_MORE: u32 = 0x6F00;
const ID_OPEN_DLG: u32 = 0x9000;

/// the rows of a tile's menu (stencils::command runs them on Stencils::menu_tile)
pub(crate) const MI_ADD_FAVORITE: u32 = 0x7380;
pub(crate) const MI_REMOVE_MASTER: u32 = 0x7381;
// i18n
pub(crate) static TILE_MENU: [Mi; 1] = [Mi::Item(MI_ADD_FAVORITE, "Zu meinen Shapes hinzufügen", 0)];
// i18n
pub(crate) static TILE_MENU_OWN: [Mi; 2] = [Mi::Item(MI_ADD_FAVORITE, "Zu meinen Shapes hinzufügen", 0), Mi::Item(MI_REMOVE_MASTER, "Aus Schablone entfernen", 0)];
// i18n
pub(crate) static TITLE_MENU: [Mi; 7] = [
    Mi::Cmd(STENCIL_CLOSE),
    Mi::Cmd(STENCIL_SAVE),
    Mi::Sub("Ansichtsoptionen", &[Mi::Cmd(STENCIL_VIEW_ICONS), Mi::Cmd(STENCIL_VIEW_ICONS_ONLY), Mi::Cmd(STENCIL_VIEW_NAMES), Mi::Cmd(STENCIL_VIEW_DETAILS)]),
    Mi::Sep,
    Mi::Cmd(STENCIL_NEW_MASTER),
    Mi::Cmd(STENCIL_ORDER),
    Mi::Cmd(MORE_SHAPES),
];

/// the previews of a stencil's masters
pub(crate) struct Previews {
    view: RefCell<PageView>,
    /// per entry: the box of its instance in paper inches (x0, y0, x1, y1; y up)
    boxes: Vec<[f64; 4]>,
    /// per entry: its tile's drawing
    draws: Vec<DrawRef>,
    /// per entry: the shape draws text only (no fill, no line) or is a 1-D shape (JD13): it gets
    /// a sheet of paper under it, else dark text and lines are lost on a dark panel
    paper: Vec<bool>,
}

impl Previews {
    /// the previews of stencil `l` (None when it has nothing to show)
    fn new(l: &Loaded, fonts: &mut FontSet) -> Option<Rc<Previews>> {
        let mut doc: Document = l.doc.clone();
        // a page to put the instances on (a stencil's page has no shapes)
        let first = doc.pages().next();
        let page = match first {
            Some(p) => p,
            None => doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).ok()?,
        };
        let mut boxes = Vec::with_capacity(l.entries.len());
        let mut tops = Vec::with_capacity(l.entries.len());
        let mut x = 0.0;
        {
            let mut tx = doc.transaction("previews");
            for e in &l.entries {
                let (w, h, lx, ly) = master_box(tx.doc(), e.master);
                // (JD13) an upright 1-D shape drawn to the left or down has a negative width or
                // height: its box lies on the other side of its pin
                let (px, py) = (x + lx - w.min(0.0), ly - h.min(0.0));
                boxes.push([x, 0.0, x + w.abs(), h.abs()]);
                tops.push(tx.add_instance(Owner::Page(page), e.master, px, py).ok());
                x += w.abs() + 1.0;
            }
            tx.commit();
        }
        if boxes.is_empty() {
            return None;
        }
        let view = PageView::new(&doc, page, fonts, &Options::default());
        let paper = tops
            .iter()
            .map(|t| {
                let Some(t) = t else { return false };
                // (JD13) connectors and lines: dark lines are lost on a dark panel too
                if diagram_core::glue::is_one_d(&doc, *t) {
                    return true;
                }
                let items = view.drawing().items.iter().filter(|i| i.top == *t);
                let mut text = false;
                for i in items {
                    match i.kind {
                        diagram_core::render::ItemKind::Text(_) => text = true,
                        _ => return false,
                    }
                }
                text
            })
            .collect();
        let n = boxes.len();
        let p = Rc::new_cyclic(|me: &alloc::rc::Weak<Previews>| Previews {
            view: RefCell::new(view),
            boxes,
            draws: (0..n).map(|k| -> DrawRef { Rc::new(TileDraw { p: me.clone(), k }) }).collect(),
            paper,
        });
        Some(p)
    }

    fn draw(&self, k: usize) -> Option<DrawRef> {
        self.draws.get(k).cloned()
    }
}

// the size and the local pin of a master's shape (inches; 1 x 1 when it has none)
fn master_box(doc: &Document, m: diagram_core::MasterRef) -> (f64, f64, f64, f64) {
    let Some(slot) = doc.master(m).and_then(|x| x.master_shape()) else { return (1.0, 1.0, 0.5, 0.5) };
    let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
    let num = |name: &str| -> Option<f64> {
        let sym = doc.sym(name)?;
        {
            let r = doc.resolve(sheet, &CellAddr::single(sym));
            r.cached.or(r.cell).and_then(|c| c.value.num())
        }
    };
    // negative for an upright 1-D shape drawn to the left or down (Width = EndX-BeginX)
    let w = num("Width").filter(|v| v.is_finite() && v.abs() > 1e-9).unwrap_or(1.0);
    // 0 for a line (its tile centres it)
    let h = num("Height").filter(|v| v.is_finite()).unwrap_or(w.abs().min(1.0));
    let lx = num("LocPinX").filter(|v| v.is_finite()).unwrap_or(w * 0.5);
    let ly = num("LocPinY").filter(|v| v.is_finite()).unwrap_or(h * 0.5);
    (w, h, lx, ly)
}

// a tile's preview: its instance's box fitted into the tile's rectangle
struct TileDraw {
    p: alloc::rc::Weak<Previews>,
    k: usize,
}

impl Draw for TileDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let Some(p) = self.p.upgrade() else { return };
        let Some(b) = p.boxes.get(self.k) else { return };
        if r[2] <= 0 || r[3] <= 0 {
            return;
        }
        // a margin for the line width around the box
        let (bw, bh) = ((b[2] - b[0]).max(1e-3) * 1.12, (b[3] - b[1]).max(1e-3) * 1.12);
        let s = (bw / r[2] as f64).max(bh / r[3] as f64);
        let cam = Camera::new([(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5], s);
        if p.paper.get(self.k).copied().unwrap_or(false) {
            // the box of the shape as paper (its size in the tile)
            let (w, h) = (((b[2] - b[0]) / s) as i32, ((b[3] - b[1]) / s) as i32);
            let (x, y) = (r[0] + (r[2] - w) / 2, r[1] + (r[3] - h) / 2);
            g.dc.set_color(0xFFFF_FFFF);
            g.shapes().fill_rrect(x, y, w, h, 2);
        }
        if let Ok(mut v) = p.view.try_borrow_mut() {
            v.draw(g.dc, g.sf, r, &cam);
        }
    }
}

// the previews of stencil `key` (made on first use)
fn previews(app: &mut Diagrammer, key: u32) -> Option<Rc<Previews>> {
    let fonts = &mut app.fonts;
    let l = app.stencils.loaded.iter_mut().find(|l| l.key == key)?;
    if l.previews.is_none() {
        l.previews = Previews::new(l, fonts);
    }
    l.previews.clone()
}

// a tile's texts and flags (owned: the panel borrows them for one pass)
struct TileText {
    stencil: u32,
    k: usize,
    name: String,
    tip: String,
    flags: u32,
    draw: Option<DrawRef>,
    ver: u32,
}

/// every pass, after commands::states (main.rs): saving only for an own stencil (favourites, a
/// new stencil)
pub(crate) fn states(app: &mut Diagrammer, ui: &mut Ui) {
    let editable = target_stencil(app).and_then(|k| app.stencils.get(k)).is_some_and(|l| l.editable);
    ui.cmd_state(STENCIL_SAVE, if editable { 0 } else { CF_DISABLED });
}

/// the Shapes panel
pub(crate) fn panel(app: &mut Diagrammer, ui: &mut Ui) {
    stencils::menu_rows(app, ui);
    open_dialog(app, ui);
    // (JD15, SL-04) Optionen › Erweitert › Shape-Suche anzeigen: off hides the search box and
    // ends a search
    let show = app.opts.get(&crate::options::SEARCH_SHOW);
    app.stencils.panel.no_search = !show;
    if !show && !app.stencils.panel.query.is_empty() {
        app.stencils.panel.query.clear();
    }
    let lang = stencils::lang();
    // the tiles of the open stencils and of the search hits, with their previews
    let open: Vec<u32> = app.stencils.open.clone();
    let mut keys: Vec<u32> = open.clone();
    let searching = !app.stencils.panel.query.trim().is_empty();
    let hits: Vec<(u32, usize)> = if searching { stencils::search(app).to_vec() } else { Vec::new() };
    for &(k, _) in &hits {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut pv: Vec<(u32, Option<Rc<Previews>>)> = Vec::with_capacity(keys.len());
    for &k in &keys {
        pv.push((k, previews(app, k)));
    }
    let armed = app.stencils.armed;
    let tile = |l: &Loaded, k: usize, p: Option<&Rc<Previews>>| -> TileText {
        let e = &l.entries[k];
        TileText {
            stencil: l.key,
            k,
            name: String::from(e.texts.name(lang)),
            tip: String::from(e.texts.prompt(lang)),
            flags: if e.texts.quick { TL_QUICK } else { 0 } | if armed == Some((l.key, k)) { TL_ARMED } else { 0 },
            draw: p.and_then(|p| p.draw(k)),
            ver: l.serial.wrapping_mul(0x9E37_79B9) ^ k as u32,
        }
    };
    let pv_of = |key: u32| pv.iter().find(|x| x.0 == key).and_then(|x| x.1.as_ref());
    let mut stencil_tiles: Vec<(u32, String, u32, Vec<TileText>)> = Vec::new();
    for &key in &open {
        let Some(l) = app.stencils.get(key) else { continue };
        let tiles = (0..l.entries.len()).map(|k| tile(l, k, pv_of(key))).collect();
        stencil_tiles.push((key, l.title(lang), if l.editable { SD_EDITABLE } else { 0 }, tiles));
    }
    let hit_tiles: Vec<TileText> = hits.iter().filter_map(|&(key, k)| app.stencils.get(key).map(|l| tile(l, k, pv_of(key)))).collect();
    // the library's structures borrow the texts
    let tile_vecs: Vec<Vec<StencilTile<'_>>> = stencil_tiles.iter().map(|s| tiles_of(&s.3)).collect();
    let defs: Vec<StencilDef<'_>> = stencil_tiles.iter().zip(tile_vecs.iter()).map(|(s, tv)| StencilDef { key: s.0, title: &s.1, tiles: tv, flags: s.2 }).collect();
    let hit_defs: Vec<StencilHit<'_>> = hit_tiles
        .iter()
        .map(|x| StencilHit { stencil: x.stencil, tile: StencilTile { key: x.k as u32, name: &x.name, tip: &x.tip, draw: x.draw.clone(), ver: x.ver, flags: x.flags } })
        .collect();
    let results_label = tf("Suchergebnisse ({})", &["{}"]);
    let p = StencilPanel {
        stencils: &defs,
        results: if searching { Some(&hit_defs) } else { None },
        drag_kind: DT_SHAPE,
        more_id: ID_MORE,
        labels: StencilLabels {
            panel: t("Shapes"),
            search: t("Shapes suchen"),
            history: t("Letzte Suchen"),
            more: label(MORE_SHAPES),
            quick: t("Schnell-Shapes"),
            results: &results_label,
            quick_empty: t("Hier Schnell-Shapes ablegen"),
            drop_hint: t("Ziehen Sie Shapes hierher, um sie hinzuzufügen."),
            empty_title: t("Keine Schablone geöffnet."),
            empty_text: t("Öffnen Sie eine Schablone über „Weitere Shapes“."),
            no_results_title: t("Keine Shapes gefunden"),
            no_results_text: t("Die Suche prüft die Namen in allen fünf Sprachen und die Suchbegriffe."),
        },
    };
    ui.next_size(-1, -1);
    let events = ui.stencil_panel(ID_PANEL, &mut app.stencils.panel, &p);
    for ev in events {
        handle(app, ui, ev);
    }
    // "Weitere Shapes" from the ribbon or a menu (SL-05): the menu where the panel's button was
    if core::mem::take(&mut app.stencils.more_menu) {
        let at = app.stencils.more_at.unwrap_or_else(|| {
            let r = ui.last_rect();
            [r[0], r[1]]
        });
        ui.menu_popup_at(i18n::menu(&stencils::MORE_MENU), at[0], at[1]);
    }
    // a message: a stencil that could not be read or saved, a shape that could not be dropped
    if let Some(msg) = app.stencils.error.clone() {
        ui.next_size(-1, 0);
        if ui.infobar(ID_PANEL + 0xF00, 0, t("Hinweis"), &msg) & EV_ACT != 0 {
            app.stencils.error = None;
            ui.repass();
        }
    }
}

// the panel's events
fn handle(app: &mut Diagrammer, ui: &mut Ui, ev: StencilEvent) {
    let st = &mut app.stencils;
    match ev {
        StencilEvent::Activate { stencil, tile } => {
            // click + click (SL-03): the tile is armed until a click on the page, Esc or a
            // second click on it
            let t = (stencil, tile as usize);
            st.armed = if st.armed == Some(t) { None } else { Some(t) };
            ui.repass();
        }
        StencilEvent::Place { stencil, tile } => {
            st.armed = None;
            crate::stencils_drop::place_center(app, ui, stencil, tile as usize);
        }
        StencilEvent::TileMenu { stencil, tile, .. } => {
            st.menu_tile = Some((stencil, tile as usize));
            let own = st.get(stencil).is_some_and(|l| l.editable);
            // at the pointer, or for the menu key at the tile with its first row focused
            ui.menu_popup(i18n::menu(if own { &TILE_MENU_OWN } else { &TILE_MENU }));
        }
        StencilEvent::StencilMenu { stencil, .. } => {
            // the clicked title becomes the active stencil: the menu's commands act on it
            st.panel.active = stencil;
            states(app, ui);
            ui.menu_popup(i18n::menu(&TITLE_MENU));
        }
        StencilEvent::Search => ui.repass(),
        StencilEvent::Dropped { stencil, before, quick, from_stencil, from_tile } => {
            stencils::add_master_to(app, ui, from_stencil, from_tile as usize, stencil, Some(before), Some(quick));
        }
        StencilEvent::More { at } => {
            st.more_at = Some(at);
            ui.menu_popup_at(i18n::menu(&stencils::MORE_MENU), at[0], at[1]);
        }
        StencilEvent::DragStart { .. } => {
            st.armed = None;
        }
        _ => {}
    }
}

// Schablone öffnen: the file dialog (.vssx, .vssm; JD16: .vsx)
fn open_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.stencils.open_dlg.clone();
    if dlg.get() == 0 {
        return;
    }
    let exts = [t("vssx;vssm=Visio-Schablone"), t("vsx=Visio-XML-Schablone (2003–2010)"), t("=Alle Dateien")];
    if ui.filedlg(ID_OPEN_DLG, FD_OPEN, &dlg, &mut app.stencils.open_path, 4096, &exts) & EV_ACT != 0 {
        let path = app.stencils.open_path.clone();
        stencils::open_file(app, ui, &path);
    }
}

/// the stencil the commands of a title's menu act on: the active one (a right click on a title
/// activates it), else the first open one
pub(crate) fn target_stencil(app: &Diagrammer) -> Option<u32> {
    let a = app.stencils.panel.active;
    if app.stencils.open.contains(&a) { Some(a) } else { app.stencils.open.first().copied() }
}

// the library's tiles borrowing the texts
fn tiles_of(v: &[TileText]) -> Vec<StencilTile<'_>> {
    v.iter().map(|x| StencilTile { key: x.k as u32, name: &x.name, tip: &x.tip, draw: x.draw.clone(), ver: x.ver, flags: x.flags }).collect()
}
