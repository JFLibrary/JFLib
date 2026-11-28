// container_styles -- (JD21, ST-02) the container styles as galleries with pictures: Container-
// Tools › Format › Containerformatvorlagen (tiles in the ribbon, ▾ every style), Kopfzeilenstil
// (the heading styles of the selected container's style) and Einfügen › Container (every style);
// a live preview while a tile of a style or a heading style is pointed at, as the design gallery
// has it (design_gallery.rs)
//
//   Styles                   the state: the pictures, the preview
//   STYLES                   the 14 styles' commands (their labels are the names)
//   gallery(app, ui, h)      the tiles of the ribbon and ▾
//   flyout(app, ui, pid)     ribbon.rs's hook: the flyouts of ▾ (ID_ALL), Kopfzeilenstil and
//                            Einfügen › Container drawn as pictures -> drawn here
//   apply(app, ui, style, done)    the selected container in a style (its heading style kept
//                            when the new style has it, else the first); done = false: the preview
//   heading(app, ui, k, done)       the selected container's heading style k
//   available(app, id)       a style or heading style beyond what the container's master offers
//                            (msvSDContainerStyleCount, msvSDHeadingStyleCount) is off
//   after(app, ui)           every pass: a preview whose tile is no longer under the pointer goes
//
// The pictures are the container master of the built-in stencil (stencils/containers) placed on
// a small page in that style and heading style, a member inside, drawn as the page is
// (diagram_view); one per style, heading style and size, kept.  A container from an older
// drawing has the master it came with: its count of styles (6 before JD21) limits the gallery, so
// a style is never chosen that its master cannot draw.  Every change is one undo step through
// edit_doc::edit; after it the container fits its members again when its automatic size asks for
// it, as Container-Tools' other formats do (structures_cmds::set_cell).

use alloc::rc::Rc;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::Options;
use diagram_core::structure;
use diagram_core::{DocKind, Owner, ShapeRef, SheetRef, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::PageView;
use gfx::Surface;
use ui::i18n::t;
use ui::*;

use crate::commands::{self, *};
use crate::{edit_canvas, edit_doc, structures, tc, Diagrammer};

const ID_TILES: u32 = 0x7EA0_0000;
const ID_MORE: u32 = 0x7EA1_0000;
/// the flyout with every style (the gallery's ▾; Containerformatvorlagen's command opens it)
pub(crate) const ID_ALL: u32 = 0x7EA2_0000;
const ID_HEADINGS: u32 = 0x7EA3_0000;
const ID_INSERT: u32 = 0x7EA4_0000;
/// the tiles in the ribbon (ribbon.rs gives the gallery the width of 4)
pub(crate) const SHOWN: usize = 4;

/// the styles' commands in order (msvSDContainerStyle 1..14)
pub(crate) const STYLES: [u32; 14] = [
    CT_STYLE_1, CT_STYLE_2, CT_STYLE_3, CT_STYLE_4, CT_STYLE_5, CT_STYLE_6, CT_STYLE_7, CT_STYLE_8, CT_STYLE_9, CT_STYLE_10, CT_STYLE_11, CT_STYLE_12,
    CT_STYLE_13, CT_STYLE_14,
];

// the heading styles' names: on top / at the bottom (style 14)
// i18n
const HEADS_TOP: [&str; 4] = ["Kopfband oben", "Titel oben", "Kopfband links", "Ohne Kopfzeile"];
// i18n
const HEADS_BOTTOM: [&str; 4] = ["Kopfband unten", "Titel unten", "Kopfband rechts", "Ohne Kopfzeile"];

/// the galleries' state
#[derive(Default)]
pub(crate) struct Styles {
    thumbs: Vec<((u32, u32, i32, i32), Rc<Surface>)>,
    /// the tile shown as a live preview, and whether a tile was under the pointer this pass
    preview: Option<u32>,
    hovered: bool,
    /// the style and heading style the container had before the preview (the tiles mark them)
    base: Option<(u32, u32)>,
}

/// the name of style `k` (1-based)
pub(crate) fn style_name(k: u32) -> &'static str {
    STYLES.get(k.wrapping_sub(1) as usize).map_or("", |&id| commands::label(id))
}

/// the name of heading style `k` of style `style`
pub(crate) fn heading_name(style: u32, k: u32) -> &'static str {
    let names = if style == 14 { &HEADS_BOTTOM } else { &HEADS_TOP };
    names.get(k.wrapping_sub(1) as usize).map_or("", |s| tc(s))
}

// the selected container's settings: style, its count, heading style, its count
fn now(app: &Diagrammer) -> Option<(ShapeRef, u32, u32, u32, u32)> {
    let o = app.opened.as_ref()?;
    let c = structures::selected_container(app)?;
    let info = structure::container(&o.doc, c)?;
    let styles = if info.style_count == 0 { 6 } else { info.style_count };
    let heads = if info.heading_style_count == 0 { 4 } else { info.heading_style_count };
    Some((c, info.style, styles.min(STYLES.len() as u32), info.heading_style, heads.min(4)))
}

/// the style and heading style the tiles mark (during a preview: the ones before it)
fn marked(app: &Diagrammer) -> Option<(u32, u32)> {
    if app.addons.styles.preview.is_some() {
        return app.addons.styles.base;
    }
    now(app).map(|n| (n.1, n.3))
}

/// whether a style command can run: the selected container's master has that style
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    if let Some(k) = STYLES.iter().position(|&x| x == id) {
        return now(app).is_some_and(|n| k as u32 + 1 <= n.2);
    }
    if let Some(k) = crate::structures_cmds::HEADING_STYLES.iter().position(|&x| x == id) {
        return now(app).is_some_and(|n| k as u32 + 1 <= n.4);
    }
    true
}

// ---------------------------------------------------------------- applying

// a User cell of container c and what follows from it (size, members, connectors)
fn write(tx: &mut diagram_core::Transaction, eng: &mut Engine, owner: Owner, c: ShapeRef, cells: &[(&str, f64)]) -> Result<(), diagram_core::EditError> {
    let sheet = SheetRef::Shape(c);
    for &(n, v) in cells {
        tx.set_user(sheet, n, Value::Num(v), None)?;
    }
    eng.recalc(tx, &[sheet])?;
    if let Some(info) = structure::container(tx.doc(), c) {
        if info.resize != structure::Resize::None {
            structure::fit(tx, eng, c, info.resize == structure::Resize::Fit)?;
        }
    }
    let mut moved = structure::settle(tx, eng, owner, &[c], false)?;
    moved.push(c);
    edit_canvas::follow(tx, eng, owner, &moved)
}

/// the selected container in style `style`; its heading style stays when the new style has it
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, style: u32, done: bool) {
    let (Some(n), Some(page)) = (now(app), app.model.page) else { return };
    let c = n.0;
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Containerformatvorlage"), done, |tx, eng| {
        write(tx, eng, owner, c, &[("msvSDContainerStyle", style as f64)])?;
        // the new style's count of heading styles: beyond it, the first
        let heads = structure::container(tx.doc(), c).map_or(4, |i| i.heading_style_count);
        if heads > 0 && n.3 > heads {
            write(tx, eng, owner, c, &[("msvSDHeadingStyle", 1.0)])?;
        }
        Ok(())
    });
    ui.repass();
}

/// the selected container's heading style `k`
pub(crate) fn heading(app: &mut Diagrammer, ui: &mut Ui, k: u32, done: bool) {
    let (Some(n), Some(page)) = (now(app), app.model.page) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Kopfzeilenstil"), done, |tx, eng| write(tx, eng, owner, n.0, &[("msvSDHeadingStyle", k as f64)]));
    ui.repass();
}

// ---------------------------------------------------------------- pictures

fn set_num(tx: &mut diagram_core::Transaction, sheet: SheetRef, name: &str, v: f64) -> Result<(), diagram_core::EditError> {
    let a = tx.single(name);
    tx.force_constant(sheet, &a, Value::Num(v), None)
}

// a picture of the container master in a style and heading style, w x h pixels (kept)
fn thumb(app: &mut Diagrammer, style: u32, head: u32, w: i32, h: i32) -> Option<Rc<Surface>> {
    let key = (style, head, w, h);
    if let Some((_, s)) = app.addons.styles.thumbs.iter().find(|(k, _)| *k == key) {
        return Some(s.clone());
    }
    let src = structures::masters(&app.stencils).containers?;
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Us);
    let page = doc.pages().next()?;
    let mut eng = Engine::new();
    let mut tx = doc.transaction("");
    let ps = SheetRef::Page(page);
    set_num(&mut tx, ps, "PageWidth", 2.0).ok()?;
    set_num(&mut tx, ps, "PageHeight", 1.8).ok()?;
    let c = structures::place_master(&mut tx, &mut eng, src, "Container", page, [1.0, 0.9]).ok()?;
    let sc = SheetRef::Shape(c);
    set_num(&mut tx, sc, "Width", 1.7).ok()?;
    set_num(&mut tx, sc, "Height", 1.5).ok()?;
    tx.set_user(sc, "msvSDContainerStyle", Value::Num(style as f64), None).ok()?;
    tx.set_user(sc, "msvSDHeadingStyle", Value::Num(head as f64), None).ok()?;
    eng.recalc(&mut tx, &[sc]).ok()?;
    // a member in the middle of the inside
    let b = structure::inner_box(tx.doc(), c);
    let (mx, my) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
    ops::new_shape(&mut tx, &mut eng, Owner::Page(page), Basic::Rectangle, [mx - 0.35, my - 0.22], [mx + 0.35, my + 0.22]).ok()?;
    let _ = tx.commit();
    let mut fonts = app.fonts.clone();
    let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
    let sf = Rc::new(v.render(w.max(8) as u32, h.max(8) as u32));
    if app.addons.styles.thumbs.len() > 128 {
        app.addons.styles.thumbs.remove(0);
    }
    app.addons.styles.thumbs.push((key, sf.clone()));
    Some(sf)
}

fn tile_size(ui: &Ui) -> (i32, i32) {
    (ui.dp(48), ui.dp(44))
}

// ---------------------------------------------------------------- tiles

// what a tile changes
#[derive(Copy, Clone, PartialEq)]
enum Tile {
    Style(u32),
    Heading(u32),
}

// a tile's events: the preview while it is under the pointer, applied on a click -> clicked
fn events(app: &mut Diagrammer, ui: &mut Ui, id: u32, ev: u32, what: Tile) -> bool {
    if ev & EV_HOVER != 0 && ui.popup_count() <= 1 && app.opts.get(&crate::options::LIVE) {
        let before = now(app).map(|n| (n.1, n.3));
        let st = &mut app.addons.styles;
        st.hovered = true;
        if st.preview != Some(id) {
            if st.preview.is_none() {
                st.base = before;
            }
            st.preview = Some(id);
            match what {
                Tile::Style(k) => apply(app, ui, k, false),
                Tile::Heading(k) => heading(app, ui, k, false),
            }
        }
    }
    if ev & EV_ACT != 0 {
        app.addons.styles.preview = None;
        match what {
            Tile::Style(k) => apply(app, ui, k, true),
            Tile::Heading(k) => heading(app, ui, k, true),
        }
        return true;
    }
    false
}

// the heading style a style's picture shows: the container's own when the style has it (the
// counts are the master's msvSDHeadingStyleCount, stencils/containers/container.master)
fn head_for(app: &Diagrammer, style: u32) -> u32 {
    let h = marked(app).map_or(1, |m| m.1);
    let count = match style {
        10 | 12 => 2,
        9 | 11 => 3,
        _ => 4,
    };
    if (1..=count).contains(&h) { h } else { 1 }
}

/// Containerformatvorlagen in the ribbon: the first SHOWN styles and ▾
pub(crate) fn gallery(app: &mut Diagrammer, ui: &mut Ui, height: i32) {
    let n = now(app);
    let cur = marked(app).map(|m| m.0);
    let (w, h) = tile_size(ui);
    ui.box_begin(0, BOX_H | BOX_FIXED, 2);
    for k in 1..=SHOWN as u32 {
        let id = ID_TILES + k;
        let head = head_for(app, k);
        let Some(sf) = thumb(app, k, head, w, h) else { continue };
        let on = n.is_some_and(|n| k <= n.2);
        ui.next_size(56, height);
        ui.next_flags(if cur == Some(k) { ST_CHECKED } else { 0 } | if on { 0 } else { ST_DISABLED });
        let ev = ui.ribbon_image_tile(id, style_name(k), &sf, k * 8 + head);
        if on {
            events(app, ui, id, ev, Tile::Style(k));
        }
    }
    ui.next_size(16, height);
    ui.next_flags(if n.is_some() { 0 } else { ST_DISABLED });
    if ui.icon_button(ID_MORE, IC_CHEV_DOWN, t("Alle Containerformatvorlagen")) & EV_ACT != 0 {
        ui.popup_open(ID_ALL);
    }
    ui.box_end();
    all_styles(app, ui);
}

// a grid of tiles in a flyout: `items` (tile number, name, style, heading), `per` in a row ->
// the tile clicked
fn grid(app: &mut Diagrammer, ui: &mut Ui, base: u32, items: &[(u32, &str, u32, u32, bool)], per: usize, live: Option<fn(u32) -> Tile>) -> Option<u32> {
    let (w, h) = tile_size(ui);
    let mut hit = None;
    for row in items.chunks(per) {
        ui.box_begin(0, BOX_H, 2);
        for &(k, name, style, head, checked) in row {
            let Some(sf) = thumb(app, style, head, w, h) else { continue };
            ui.next_size(56, 66);
            ui.next_flags(if checked { ST_CHECKED } else { 0 });
            let id = base + k;
            let ev = ui.ribbon_image_tile(id, name, &sf, style * 8 + head);
            match live {
                Some(f) => {
                    if events(app, ui, id, ev, f(k)) {
                        hit = Some(k);
                    }
                }
                None if ev & EV_ACT != 0 => hit = Some(k),
                None => {}
            }
        }
        ui.box_end();
    }
    hit
}

// the flyout of ▾: every style the container's master has
fn all_styles(app: &mut Diagrammer, ui: &mut Ui) {
    if ui.flyout_begin(ID_ALL, 0) {
        let count = now(app).map_or(0, |n| n.2);
        let cur = marked(app).map(|m| m.0);
        let items: Vec<(u32, &str, u32, u32, bool)> = (1..=count).map(|k| (k, style_name(k), k, head_for(app, k), cur == Some(k))).collect();
        ui.text(TS_CAPTION | TX_SECONDARY, tc("Containerformatvorlagen"));
        if grid(app, ui, ID_ALL + 0x10, &items, 7, Some(Tile::Style)).is_some() {
            ui.popup_close();
        }
    }
    ui.flyout_end();
}

/// ribbon.rs's hook for the flyouts drawn as pictures -> whether `pid` is one of them
pub(crate) fn flyout(app: &mut Diagrammer, ui: &mut Ui, pid: u32) -> bool {
    match pid {
        CT_HEADING => {
            if ui.flyout_begin(pid, 0) {
                let (style, count) = now(app).map_or((1, 4), |n| (n.1, n.4));
                let cur = marked(app).map(|m| m.1);
                let items: Vec<(u32, &str, u32, u32, bool)> = (1..=count).map(|k| (k, heading_name(style, k), style, k, cur == Some(k))).collect();
                if grid(app, ui, ID_HEADINGS, &items, 4, Some(Tile::Heading)).is_some() {
                    ui.popup_close();
                }
            }
            ui.flyout_end();
            true
        }
        CONTAINER => {
            if ui.flyout_begin(pid, 0) {
                let items: Vec<(u32, &str, u32, u32, bool)> = (1..=STYLES.len() as u32).map(|k| (k, style_name(k), k, 1, false)).collect();
                ui.text(TS_CAPTION | TX_SECONDARY, tc("Container einfügen"));
                if let Some(k) = grid(app, ui, ID_INSERT, &items, 7, None) {
                    ui.popup_close();
                    crate::structures_cmds::insert_container(app, ui, k);
                }
            }
            ui.flyout_end();
            true
        }
        _ => false,
    }
}

/// every pass after the ribbon: a preview whose tile is no longer under the pointer goes
pub(crate) fn after(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.addons.styles;
    if !st.hovered && st.preview.take().is_some() {
        st.base = None;
        if let Some(o) = app.opened.as_mut() {
            edit_doc::cancel(o);
        }
        ui.repass();
    }
    app.addons.styles.hovered = false;
}
