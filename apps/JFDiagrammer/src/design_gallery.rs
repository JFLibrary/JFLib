// design_gallery -- (JD11) Entwurf › Designs and Varianten: the galleries, their live preview,
// applying a design or a variant to all pages or the page shown (RW-05, RW-06, TH-02 .. 04)
//
//   Designs                  the state: the thumbnails, the preview, the saved designs
//   designs(app, ui)         the gallery of the group Designs (the tiles, then ▾ with all
//                            designs and the menu: Kein Design, Aktuelles Design speichern ...)
//   variants(app, ui)        the four variants of the design the page shows
//   after(app, ui)           every pass after the ribbon: a preview whose tile is no longer
//                            under the pointer is taken back
//   entries(app)             the designs offered: those in the file (Visio's too: shown, kept,
//                            never shipped), JFDiagrammer's own (diagram_core::design::catalog),
//                            the user's saved ones (design_store.rs)
//
// Applying goes through edit_doc::edit as one transaction (diagram_core::design::apply): a
// click applies to all pages, as Visio does, the tile's menu (right click) offers the page
// shown.  Pointing at a tile applies it as a preview (edit with done = false); leaving the
// tiles takes the preview back, a click makes it one undo step.  The thumbnails are a small
// drawing (three shapes, two connectors) built and drawn the way the page is: new_document,
// design::apply, diagram_view; one per design, variant and size, kept.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply::{self, Scope};
use diagram_core::design::{catalog, FIRST_ID};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::{Options, ThemeInfo};
use diagram_core::{glue, CellAddr, DocKind, Document, Owner, PageRef, ShapeRef, SheetRef, Transaction, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::PageView;
use gfx::Surface;
use ui::i18n::t;
use ui::*;

use crate::Diagrammer;

const ID_TILES: u32 = 0x5D00_0000;
const ID_VARIANTS: u32 = 0x5D10_0000;
const ID_MORE: u32 = 0x5D20_0000;
/// the flyout with all designs (the gallery's ▾; Entwurf › Designs' command opens it, cmds_shell.rs)
pub(crate) const ID_ALL: u32 = 0x5D30_0000;
const ID_MENU: u32 = 0x5D40_0000;
const ID_VMENU: u32 = 0x5D50_0000;
/// the tiles in the ribbon (the gallery's width in ribbon.rs: 6)
pub(crate) const SHOWN: usize = 6;

/// where a design comes from
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// a theme part of the open file (Visio's designs among them)
    File,
    /// JFDiagrammer's own (catalog)
    Own,
    /// saved by the user (design_store)
    Saved,
}

/// a design the gallery offers
#[derive(Clone)]
pub(crate) struct Entry {
    /// the scheme number (its vt:themeScheme)
    pub id: u32,
    /// the name shown
    pub name: String,
    /// the theme part
    pub part: Rc<Vec<u8>>,
    pub kind: Kind,
}

/// the galleries' state
#[derive(Default)]
pub(crate) struct Designs {
    thumbs: Vec<(u64, Rc<Surface>)>,
    own: Vec<Entry>,
    /// the tile shown as a live preview (its id), and whether a tile was under the pointer
    /// in this pass
    preview: Option<u32>,
    hovered: bool,
    /// the design and variant the page had before the preview (the tiles mark that one)
    base: Option<(u32, u32)>,
    /// the entry and the variant (None: a design) a tile's menu was opened for
    menu_for: Option<(Entry, Option<u32>)>,
    /// the user's saved designs (design_store.rs)
    pub saved: Vec<Entry>,
    pub saved_loaded: bool,
}

/// the name of an own design in the current language
pub(crate) fn own_name(id: u32) -> &'static str {
    match id {
        catalog::NETLAB => "NetLab",
        catalog::CONTOUR => crate::tc("Kontur|Design"),
        catalog::CONTRAST => crate::tc("Kontrast|Design"),
        _ => "",
    }
}

fn hash(bytes: &[u8], extra: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes.iter().copied().chain(extra.iter().flat_map(|v| v.to_le_bytes())) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// the page the canvas shows
pub(crate) fn page_shown(app: &Diagrammer) -> Option<PageRef> {
    let o = app.opened.as_ref()?;
    o.tabs.get(app.page_index()).map(|t| t.page)
}

/// the design and variant the page shown has
pub(crate) fn current(app: &Diagrammer) -> Option<(u32, u32)> {
    let o = app.opened.as_ref()?;
    apply::page_design(&o.doc, page_shown(app)?)
}

/// the design and variant applied (during a live preview: the one before it)
pub(crate) fn applied(app: &Diagrammer) -> Option<(u32, u32)> {
    if app.designs.preview.is_some() {
        return app.designs.base;
    }
    current(app)
}

/// the designs offered (see the module comment), file designs first
pub(crate) fn entries(app: &mut Diagrammer) -> Vec<Entry> {
    if app.designs.own.is_empty() {
        app.designs.own = catalog::all().into_iter().map(|d| Entry { id: d.id, name: String::new(), part: Rc::new(d.theme_xml()), kind: Kind::Own }).collect();
    }
    crate::design_store::load(app);
    let mut out: Vec<Entry> = Vec::new();
    if let Some(o) = app.opened.as_ref() {
        for th in &o.doc.themes {
            let Some(info) = ThemeInfo::parse(&th.data) else { continue };
            let Some(id) = info.scheme_ids[3] else { continue };
            if id >= FIRST_ID || out.iter().any(|e| e.id == id) {
                continue;
            }
            let name = if info.name.is_empty() { format!("{} {id}", t("Design")) } else { info.name.clone() };
            out.push(Entry { id, name, part: Rc::new(th.data.clone()), kind: Kind::File });
        }
    }
    for e in &app.designs.own {
        out.push(Entry { name: String::from(own_name(e.id)), ..e.clone() });
    }
    for e in &app.designs.saved {
        if !out.iter().any(|x| x.id == e.id) {
            out.push(e.clone());
        }
    }
    out
}

// ---------------------------------------------------------------- thumbnails

fn set_num(tx: &mut Transaction, sheet: SheetRef, name: &str, v: f64) -> Result<(), diagram_core::EditError> {
    let a = tx.single(name);
    tx.force_constant(sheet, &a, Value::Num(v), None)
}

fn quick(tx: &mut Transaction, s: ShapeRef, c: f64, m: f64) -> Result<(), diagram_core::EditError> {
    let sh = SheetRef::Shape(s);
    for n in ["QuickStyleLineColor", "QuickStyleFillColor", "QuickStyleShadowColor", "QuickStyleFontColor"] {
        set_num(tx, sh, n, c)?;
    }
    for n in ["QuickStyleLineMatrix", "QuickStyleFillMatrix", "QuickStyleEffectsMatrix", "QuickStyleFontMatrix"] {
        set_num(tx, sh, n, m)?;
    }
    Ok(())
}

fn pin(doc: &Document, s: ShapeRef) -> [f64; 2] {
    let n = |c: &str| doc.sym(c).and_then(|c| doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num()).unwrap_or(0.0);
    [n("PinX"), n("PinY")]
}

/// the small drawing of a thumbnail: a design (theme part `part`, number `id`) in a variant
pub(crate) fn sample(id: u32, part: &[u8], variant: u32) -> Option<Document> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Us);
    let page = doc.pages().next()?;
    let owner = Owner::Page(page);
    let mut eng = Engine::new();
    let mut tx = doc.transaction("");
    let ps = SheetRef::Page(page);
    set_num(&mut tx, ps, "PageWidth", 2.0).ok()?;
    set_num(&mut tx, ps, "PageHeight", 1.9).ok()?;
    apply::apply(&mut tx, &mut eng, id, part, Scope::Page(page), variant).ok()?;
    let a = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [0.15, 1.05], [0.95, 1.65]).ok()?;
    let b = ops::new_shape(&mut tx, &mut eng, owner, Basic::Ellipse, [1.15, 1.05], [1.85, 1.65]).ok()?;
    quick(&mut tx, b, 101.0, 101.0).ok()?;
    let c = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [0.55, 0.2], [1.45, 0.7]).ok()?;
    quick(&mut tx, c, 100.0, 102.0).ok()?;
    for (x, y) in [(a, b), (a, c)] {
        let (px, py) = (pin(tx.doc(), x), pin(tx.doc(), y));
        glue::new_connector(&mut tx, &mut eng, owner, (px, glue::Target::Shape(x)), (py, glue::Target::Shape(y))).ok()?;
    }
    let _ = tx.commit();
    Some(doc)
}

// a thumbnail of w x h pixels (kept)
fn thumb(app: &mut Diagrammer, e: &Entry, variant: u32, w: i32, h: i32) -> Option<Rc<Surface>> {
    let key = hash(&e.part, &[e.id, variant, w as u32, h as u32]);
    if let Some((_, s)) = app.designs.thumbs.iter().find(|(k, _)| *k == key) {
        return Some(s.clone());
    }
    let doc = sample(e.id, &e.part, variant)?;
    let page = doc.pages().next()?;
    let mut fonts = app.fonts.clone();
    let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
    let sf = Rc::new(v.render(w.max(8) as u32, h.max(8) as u32));
    if app.designs.thumbs.len() > 96 {
        app.designs.thumbs.remove(0);
    }
    app.designs.thumbs.push((key, sf.clone()));
    Some(sf)
}

// ---------------------------------------------------------------- applying

fn scope(app: &Diagrammer, all: bool) -> Option<Scope> {
    if all {
        return Some(Scope::All);
    }
    page_shown(app).map(Scope::Page)
}

/// apply a design (variant 1) -> applied; `done` false: as the live preview
pub(crate) fn apply_design(app: &mut Diagrammer, ui: &mut Ui, e: &Entry, all: bool, done: bool) -> bool {
    let Some(sc) = scope(app, all) else { return false };
    let Some(o) = app.opened.as_mut() else { return false };
    let part = e.part.clone();
    let id = e.id;
    let r = crate::edit_doc::edit(o, t("Design anwenden"), done, |tx, eng| apply::apply(tx, eng, id, &part, sc, 0));
    ui.repass();
    r.is_ok()
}

/// another variant of the design the pages have -> applied
pub(crate) fn apply_variant(app: &mut Diagrammer, ui: &mut Ui, variant: u32, all: bool, done: bool) -> bool {
    let Some(sc) = scope(app, all) else { return false };
    let Some(o) = app.opened.as_mut() else { return false };
    let r = crate::edit_doc::edit(o, t("Variante anwenden"), done, |tx, eng| apply::set_variant(tx, eng, sc, variant));
    ui.repass();
    r.is_ok()
}

/// Kein Design: the pages without a design
pub(crate) fn no_design(app: &mut Diagrammer, ui: &mut Ui, all: bool) {
    let Some(sc) = scope(app, all) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let _ = crate::edit_doc::edit(o, t("Kein Design"), true, |tx, eng| apply::remove(tx, eng, sc));
    ui.repass();
}

// a tile's events: the live preview while it is under the pointer, applied on a click, its
// menu on a right click; -> whether it was clicked
fn tile_events(app: &mut Diagrammer, ui: &mut Ui, tile: u32, ev: u32, e: &Entry, variant: Option<u32>) {
    if ev & EV_HOVER != 0 && ui.popup_count() == 0 && app.opts.get(&crate::options::LIVE) {
        app.designs.hovered = true;
        if app.designs.preview != Some(tile) {
            if app.designs.preview.is_none() {
                app.designs.base = current(app);
            }
            app.designs.preview = Some(tile);
            match variant {
                Some(v) => apply_variant(app, ui, v, true, false),
                None => apply_design(app, ui, e, true, false),
            };
        }
    }
    if ev & EV_ACT != 0 {
        app.designs.preview = None;
        match variant {
            Some(v) => apply_variant(app, ui, v, true, true),
            None => apply_design(app, ui, e, true, true),
        };
    }
    if ev & EV_RCLICK != 0 {
        if app.designs.preview.take().is_some() {
            if let Some(o) = app.opened.as_mut() {
                crate::edit_doc::cancel(o);
            }
        }
        app.designs.menu_for = Some((e.clone(), variant));
        let (mx, my) = ui.mouse();
        ui.popup_open_at(if variant.is_some() { ID_VMENU } else { ID_MENU }, mx, my);
    }
}

// the label of a design's tile
fn label(e: &Entry) -> String {
    match e.kind {
        Kind::File => format!("{} ({})", e.name, t("in dieser Datei")),
        Kind::Saved => format!("{} ({})", e.name, t("gespeichert")),
        Kind::Own => e.name.clone(),
    }
}

fn tile_size(ui: &Ui) -> (i32, i32) {
    (ui.dp(48), ui.dp(44))
}

/// the gallery of the group Designs
pub(crate) fn designs(app: &mut Diagrammer, ui: &mut Ui, height: i32) {
    let list = entries(app);
    let cur = applied(app).map(|c| c.0);
    let (w, h) = tile_size(ui);
    let off = app.opened.is_none();
    ui.box_begin(0, BOX_H | BOX_FIXED, 2);
    for (k, e) in list.iter().take(SHOWN).enumerate() {
        let tile = ID_TILES + k as u32;
        let Some(sf) = thumb(app, e, 0, w, h) else { continue };
        ui.next_size(56, height);
        ui.next_flags(if cur == Some(e.id) { ST_CHECKED } else { 0 } | if off { ST_DISABLED } else { 0 });
        let ev = ui.ribbon_image_tile(tile, &label(e), &sf, k as u32);
        tile_events(app, ui, tile, ev, e, None);
    }
    // ▾: all designs and the menu
    ui.next_size(16, height);
    if ui.icon_button(ID_MORE, IC_CHEV_DOWN, t("Alle Designs")) & EV_ACT != 0 {
        ui.popup_open(ID_ALL);
    }
    ui.box_end();
    all_designs(app, ui, &list, cur);
    tile_menu(app, ui);
}

// the flyout with every design by where it comes from, and the commands
fn all_designs(app: &mut Diagrammer, ui: &mut Ui, list: &[Entry], cur: Option<u32>) {
    if !ui.flyout_begin(ID_ALL, 0) {
        ui.flyout_end();
        return;
    }
    let (w, h) = tile_size(ui);
    for (kind, caption) in [(Kind::File, t("In dieser Datei")), (Kind::Own, t("JFDiagrammer")), (Kind::Saved, t("Gespeichert"))] {
        let of: Vec<(usize, &Entry)> = list.iter().enumerate().filter(|(_, e)| e.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        ui.text(TS_CAPTION | TX_SECONDARY, caption);
        ui.box_begin(0, BOX_H, 2);
        for (k, e) in of {
            let tile = ID_ALL + 1 + k as u32;
            let Some(sf) = thumb(app, e, 0, w, h) else { continue };
            ui.next_size(56, 66);
            ui.next_flags(if cur == Some(e.id) { ST_CHECKED } else { 0 });
            let ev = ui.ribbon_image_tile(tile, &label(e), &sf, k as u32);
            if ev & EV_ACT != 0 {
                apply_design(app, ui, e, true, true);
                ui.popup_close();
            }
        }
        ui.box_end();
    }
    ui.menu_sep();
    if ui.menu_item(ID_ALL + 0x100, t("Kein Design")) & EV_ACT != 0 {
        no_design(app, ui, true);
        ui.popup_close();
    }
    ui.next_flags(if new_shapes(app) { ST_CHECKED } else { 0 });
    if ui.menu_item(ID_ALL + 0x103, crate::commands::label(crate::commands::THEME_NEW_SHAPES)) & EV_ACT != 0 {
        toggle_new_shapes(app, ui);
        ui.popup_close();
    }
    ui.menu_sep();
    ui.next_flags(if app.opened.is_some() && cur.is_some() { 0 } else { ST_DISABLED });
    if ui.menu_item(ID_ALL + 0x101, t("Aktuelles Design speichern ...")) & EV_ACT != 0 {
        crate::design_store::ask_save(app, ui);
        ui.popup_close();
    }
    let saved = app.designs.saved.len();
    ui.next_flags(if saved > 0 { 0 } else { ST_DISABLED });
    if ui.menu_item(ID_ALL + 0x102, t("Gespeicherte Designs löschen")) & EV_ACT != 0 {
        crate::design_store::delete_all(app);
        ui.popup_close();
    }
    ui.flyout_end();
}

// a tile's menu: apply to all pages or the page shown
fn tile_menu(app: &mut Diagrammer, ui: &mut Ui) {
    for (pid, variant) in [(ID_MENU, false), (ID_VMENU, true)] {
        if ui.flyout_begin(pid, 0) {
            if let Some((e, v)) = app.designs.menu_for.clone() {
                if ui.menu_item(pid + 1, t("Auf alle Seiten anwenden")) & EV_ACT != 0 {
                    match (variant, v) {
                        (true, Some(v)) => apply_variant(app, ui, v, true, true),
                        _ => apply_design(app, ui, &e, true, true),
                    };
                    ui.popup_close();
                }
                if ui.menu_item(pid + 2, t("Auf aktuelle Seite anwenden")) & EV_ACT != 0 {
                    match (variant, v) {
                        (true, Some(v)) => apply_variant(app, ui, v, false, true),
                        _ => apply_design(app, ui, &e, false, true),
                    };
                    ui.popup_close();
                }
            }
        }
        ui.flyout_end();
    }
}

/// the variants of the design the page shows
pub(crate) fn variants(app: &mut Diagrammer, ui: &mut Ui, height: i32) {
    let cur = current(app);
    let marked = applied(app);
    let list = entries(app);
    let e = cur.and_then(|(id, _)| list.into_iter().find(|e| e.id == id).or_else(|| file_entry(app, id)));
    let (w, h) = tile_size(ui);
    ui.box_begin(0, BOX_H | BOX_FIXED, 2);
    for k in 0..4u32 {
        let tile = ID_VARIANTS + k;
        let name = format!("{} {}", t("Variante"), k + 1);
        match (&e, cur) {
            (Some(e), Some((_, var))) => {
                let Some(sf) = thumb(app, e, k, w, h) else { continue };
                ui.next_size(56, height);
                let on = marked.is_some_and(|m| m.0 == e.id && m.1 == k) || (marked.is_none() && var == k);
                ui.next_flags(if on { ST_CHECKED } else { 0 });
                let ev = ui.ribbon_image_tile(tile, &name, &sf, k);
                tile_events(app, ui, tile, ev, e, Some(k));
            }
            _ => {
                ui.next_size(56, height);
                ui.next_flags(ST_DISABLED);
                ui.ribbon_theme_tile(tile, &name, 0xFFB8BEC6, k);
            }
        }
    }
    ui.box_end();
}

// a design a page names that the gallery does not list (a part whose themeScheme number
// differs from the page's ThemeIndex): the first part with that number anywhere
fn file_entry(app: &Diagrammer, id: u32) -> Option<Entry> {
    let o = app.opened.as_ref()?;
    let k = apply::find_part(&o.doc, id)?;
    let data = o.doc.themes.get(k)?.data.clone();
    Some(Entry { id, name: String::new(), part: Rc::new(data), kind: Kind::File })
}

// ---------------------------------------------------------------- Design auf neue Shapes anwenden

/// the option (an application setting, as Visio's ApplyThemesOnShapeAdd): new shapes get the
/// page's design
pub(crate) fn new_shapes(app: &Diagrammer) -> bool {
    app.opts.ini().get("design", "new_shapes") != Some("0")
}

/// the command: the option on or off
pub(crate) fn toggle_new_shapes(app: &mut Diagrammer, _ui: &mut Ui) {
    let v = if new_shapes(app) { "0" } else { "1" };
    app.opts.ini_mut().set("design", "new_shapes", v);
    app.opts.save();
}

/// every shape of the pages (to see which an edit adds)
pub(crate) fn shapes_now(doc: &Document) -> Vec<ShapeRef> {
    doc.pages().flat_map(|p| doc.all_shapes(Owner::Page(p)).collect::<Vec<_>>()).collect()
}

/// the shapes an edit added (not in `before`) get no design: their theme index cells 0
pub(crate) fn unthemed_new(tx: &mut Transaction, eng: &mut Engine, before: &[ShapeRef]) -> Result<(), diagram_core::EditError> {
    let added: Vec<ShapeRef> = shapes_now(tx.doc()).into_iter().filter(|s| !before.contains(s)).collect();
    if added.is_empty() {
        return Ok(());
    }
    for &s in &added {
        for n in apply::INDEX_CELLS {
            set_num(tx, SheetRef::Shape(s), n, 0.0)?;
        }
    }
    let sheets: Vec<SheetRef> = added.into_iter().map(SheetRef::Shape).collect();
    eng.recalc(tx, &sheets)?;
    Ok(())
}

/// after the ribbon, every pass: a preview whose tile is no longer under the pointer goes
pub(crate) fn after(app: &mut Diagrammer, ui: &mut Ui) {
    let on = new_shapes(app);
    if let Some(o) = app.opened.as_mut() {
        o.ed.theme_new_shapes = on;
    }
    if !app.designs.hovered && app.designs.preview.take().is_some() {
        if let Some(o) = app.opened.as_mut() {
            crate::edit_doc::cancel(o);
        }
        ui.repass();
    }
    app.designs.hovered = false;
}
