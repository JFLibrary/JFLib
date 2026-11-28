// autoconn -- (JD21) the connector automatics of the canvas: a shape dropped, pasted or moved
// onto a connector goes into it (CN-08, SL-03), a deleted shape takes its connectors along and
// the flow heals (CN-09), the connector tool chains shapes dragged from the Shapes panel
// (CN-10), the connector tool for one connector (CN-11), the connector tool with the connector
// master marked in the panel (CN-16), the quick shapes bar at the Verbindungspfeile with its
// live preview (CN-07, SL-06, ZF-10), several masters dragged at once and the glow while a
// master is dragged from the panel (SL-03), the page's PlowCode and PageShapeSplit (PR-09).
// The rules are diagram_core::autoconn's; this file asks the options and feeds the canvas.
//
//   AutoConn                       the state: the connector a running move would split, the
//                                  tool "once", the tiles marked with Ctrl, the tile dragged
//                                  from the panel, the quick shapes bar and its preview
//   split_on(app) / heal_on(app)   Optionen › Erweitert › Verbinder teilen / Verbinder mit
//                                  Shapes löschen
//   drop_rules(app) -> DropRules   what a drop does besides placing: split (the option), the
//                                  shape to chain to (connector tool + one selected shape)
//   dropped(tx, eng, owner, shapes, rules)   in the drop's edit, before connectors follow
//                                  (stencils_drop.rs, edit_cmds::paste): split and make room,
//                                  else chain; plow when the page says so
//   move_target(doc, owner, shapes, d) / moved(tx, eng, owner, shapes, hint)   a move: the
//                                  connector is found with the drawing as it was (the move's
//                                  connectors reroute around the shape), split when released
//   tool_master(app) / connector(tx, eng, owner, a, b, master)   (CN-16) the connector tool's
//                                  connector: an instance of the marked 1-D master, else the
//                                  dynamic connector
//   once(app, ui) / connected(app) (CN-11) CONNECTOR_ONCE: the connector tool until one
//                                  connector is drawn
//   quick_tiles(app)               every pass before ui.shape_edit: SeState::quick from the
//                                  active stencil's first four quick shapes (2-D masters)
//   quick_hover(app, ui, ..) / quick(app, ui, ..)   the bar's events: the live preview (edit_doc's
//                                  preview, the new shape and its connector half transparent
//                                  at the page's spacing; Optionen › Livevorschau), the click
//   overlay(app, ui)               the glow of the connector a moved shape would split
//   drag_over(app, ui)             (stencils_drop.rs) a master dragged from the panel over the
//                                  canvas: the containers and lists it would join, the
//                                  connector it would split
//   mark(app, ui, key, k) / place_many(app, ui, at)   (SL-03) Ctrl+click marks several tiles,
//                                  a drag of one of them (or a click on the page) drops all of
//                                  them in a row
//
// Every gesture stays one undo step: the hooks run inside the edit of the drop, paste, move or
// delete.  Nothing happens on opening a file.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::autoconn as ac;
use diagram_core::formula::Engine;
use diagram_core::structure::{self, Kind};
use diagram_core::{glue, instance, ops, CellAddr, Document, EditError, MasterRef, Owner, PageRef, ShapeRef, SheetRef, Transaction, Value};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of, shape_of};
use crate::edit_doc;
use crate::Diagrammer;

/// the state of the connector automatics
#[derive(Default)]
pub(crate) struct AutoConn {
    /// the connector the shape of a running move would split (its glow)
    pub hint: Option<ShapeRef>,
    /// (CN-11) the connector tool draws one connector, then the pointer comes back
    pub once: bool,
    /// (SL-03) the tiles marked with Ctrl+click in the Shapes panel (stencil key, entry)
    pub multi: Vec<(u32, usize)>,
    /// the tile being dragged from the Shapes panel
    pub dragging: Option<(u32, usize)>,
    /// the quick shapes bar: its tiles (stencil key, entry), what they were made for (stencil
    /// key, its serial, the language), the tile shown as a preview (shape key, arrow, tile)
    quick: Vec<(u32, usize)>,
    quick_for: Option<(u32, u32, usize)>,
    preview: Option<(u64, u8, u8)>,
}

/// Optionen › Erweitert › Verbinder teilen
pub(crate) fn split_on(app: &Diagrammer) -> bool {
    app.opts.get(&crate::options::SPLIT_CONN)
}

/// Optionen › Erweitert › Verbinder mit Shapes löschen
pub(crate) fn heal_on(app: &Diagrammer) -> bool {
    app.opts.get(&crate::options::DELETE_CONN)
}

/// what a drop does besides placing the shape
#[derive(Clone, Copy, Default)]
pub(crate) struct DropRules {
    /// a shape on a connector splits it (the option)
    pub split: bool,
    /// (CN-10) the connector tool is active: the shape selected before the drop, connected to
    /// the dropped one
    pub chain: Option<ShapeRef>,
}

/// the rules of a drop now
pub(crate) fn drop_rules(app: &Diagrammer) -> DropRules {
    let chain = match (app.tool == CONNECTOR_TOOL, app.model.page, app.se.sel.len()) {
        (true, Some(page), 1) => app.se.sel.iter().next().map(|&k| shape_of(page, k)),
        _ => None,
    };
    DropRules { split: split_on(app), chain }
}

// a dynamic connector from a to b, both glued to the whole shape, routed
fn connect(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: ShapeRef, b: ShapeRef) -> Result<ShapeRef, EditError> {
    let (pa, pb) = {
        let doc = tx.doc();
        (ops::frame(doc, a).centre(), ops::frame(doc, b).centre())
    };
    let c = glue::new_connector(tx, eng, owner, (pa, glue::Target::Shape(a)), (pb, glue::Target::Shape(b)))?;
    glue::update(tx, eng, &[c])?;
    Ok(c)
}

// a 2-D top-level shape of the page that a connector may join
fn joinable(doc: &Document, s: ShapeRef) -> bool {
    doc.shape(s).is_some_and(|x| !x.is_deleted() && x.parent().is_none() && x.header().kind != diagram_core::ShapeKind::Guide) && !glue::is_one_d(doc, s) && structure::kind(doc, s) != Some(Kind::Callout)
}

/// in the edit of a drop (from the Shapes panel, a paste), before the connectors follow: one
/// shape on a connector goes into it and the shapes after it make room (CN-08); else with the
/// connector tool it is connected to the shape selected before (CN-10); then the page plows
pub(crate) fn dropped(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], rules: &DropRules) -> Result<(), EditError> {
    let Owner::Page(page) = owner else { return Ok(()) };
    let gap = ac::spacing(tx.doc(), page);
    let mut inserted = false;
    if let ([s], true) = (shapes, rules.split) {
        let target = {
            let doc = tx.doc();
            if ac::page_splits(doc, page) && ac::can_insert(doc, owner, *s) {
                let skip = structure::carried(doc, &[*s]);
                ac::split_target(doc, owner, structure::shape_box(doc, *s), &skip)
            } else {
                None
            }
        };
        if let Some(c) = target {
            ac::insert(tx, eng, owner, *s, c, gap)?;
            inserted = true;
        }
    }
    if let (false, Some(prev), [s]) = (inserted, rules.chain, shapes) {
        if prev != *s && joinable(tx.doc(), prev) && joinable(tx.doc(), *s) {
            connect(tx, eng, owner, prev, *s)?;
        }
    }
    if ac::plows(tx.doc(), page) {
        ac::plow(tx, eng, owner, shapes, gap)?;
    }
    Ok(())
}

/// a move of one shape by `d` (page), asked with the drawing as it was before the move: the
/// connector it would split at its new place (None: none, or splitting is off)
pub(crate) fn move_target(doc: &Document, owner: Owner, shapes: &[ShapeRef], d: [f64; 2]) -> Option<ShapeRef> {
    let ([s], Owner::Page(page)) = (shapes, owner) else { return None };
    if !ac::page_splits(doc, page) || !ac::can_insert(doc, owner, *s) {
        return None;
    }
    let b = structure::shape_box(doc, *s);
    let b = [b[0] + d[0], b[1] + d[1], b[2] + d[0], b[3] + d[1]];
    let skip = structure::carried(doc, &[*s]);
    ac::split_target(doc, owner, b, &skip)
}

/// a move released: the shape goes into the connector found before (`hint`), the page plows
pub(crate) fn moved(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], hint: Option<ShapeRef>) -> Result<(), EditError> {
    let Owner::Page(page) = owner else { return Ok(()) };
    let gap = ac::spacing(tx.doc(), page);
    if let (Some(c), [s]) = (hint, shapes) {
        if ac::splittable(tx.doc(), c) && ac::can_insert(tx.doc(), owner, *s) {
            ac::insert(tx, eng, owner, *s, c, gap)?;
        }
    }
    if ac::plows(tx.doc(), page) {
        ac::plow(tx, eng, owner, shapes, gap)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- the connector tool

/// (CN-16) the connector master the connector tool draws: the tile armed in the Shapes panel
/// when its master is 1-D and the connector tool is active
pub(crate) fn tool_master(app: &Diagrammer) -> Option<(u32, usize)> {
    if app.tool != CONNECTOR_TOOL {
        return None;
    }
    let (key, k) = app.stencils.armed?;
    let (l, e) = app.stencils.entry(key, k)?;
    let slot = l.doc.master(e.master)?.master_shape()?;
    glue::is_one_d(&l.doc, ShapeRef { owner: Owner::Master(e.master), slot }).then_some((key, k))
}

// an end of a new connector; a 1-D master that is no dynamic connector cannot glue to a whole
// shape: that end stays free there
fn glue_end(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, end: glue::End, (at, to): ([f64; 2], glue::Target)) -> Result<(), EditError> {
    match glue::glue(tx, eng, c, end, at, to) {
        Err(EditError::Invalid) if matches!(to, glue::Target::Shape(_)) => glue::glue(tx, eng, c, end, at, glue::Target::None),
        r => r,
    }
}

/// the connector tool's new connector from `a` to `b` (point, glue): an instance of `master`
/// (a 1-D master of a stencil, CN-16), else the dynamic connector
pub(crate) fn connector(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: ([f64; 2], glue::Target), b: ([f64; 2], glue::Target), master: Option<(&Document, MasterRef)>) -> Result<ShapeRef, EditError> {
    let (Some((src, m)), Owner::Page(page)) = (master, owner) else { return glue::new_connector(tx, eng, owner, a, b) };
    let lm = instance::import_master(tx, src, m)?;
    let c = instance::place(tx, page, lm, (a.0[0] + b.0[0]) * 0.5, (a.0[1] + b.0[1]) * 0.5, crate::stencils::lang())?;
    let mut sheets = Vec::new();
    collect(tx.doc(), c, &mut sheets);
    eng.recalc(tx, &sheets)?;
    glue_end(tx, eng, c, glue::End::Begin, a)?;
    glue_end(tx, eng, c, glue::End::End, b)?;
    glue::update(tx, eng, &[c])?;
    Ok(c)
}

/// (CN-11) Verbinder (einmal): the connector tool for one connector, then the pointer
pub(crate) fn once(app: &mut Diagrammer, ui: &mut Ui) {
    crate::edit_cmds::tool(app, CONNECTOR_TOOL);
    app.autoconn.once = true;
    ui.repass();
}

/// a connector was drawn: the tool "once" gives way to the pointer
pub(crate) fn connected(app: &mut Diagrammer) {
    if core::mem::take(&mut app.autoconn.once) && app.tool == CONNECTOR_TOOL {
        app.tool = POINTER;
    }
}

// a shape and its subshapes as sheets
fn collect(doc: &Document, s: ShapeRef, out: &mut Vec<SheetRef>) {
    out.push(SheetRef::Shape(s));
    if let Some(sh) = doc.shape(s) {
        for c in sh.children() {
            collect(doc, c, out);
        }
    }
}

// ---------------------------------------------------------------- the quick shapes bar

// the stencil whose quick shapes the bar offers: the active one, else the first open one
fn active_stencil(app: &Diagrammer) -> Option<u32> {
    let a = app.stencils.panel.active;
    if app.stencils.open.contains(&a) { Some(a) } else { app.stencils.open.first().copied() }
}

/// every pass before ui.shape_edit: the quick shapes bar's tiles (the active stencil's first
/// four quick shapes that are 2-D); the tool "once" ends when another tool was chosen
pub(crate) fn quick_tiles(app: &mut Diagrammer) {
    if app.tool != CONNECTOR_TOOL {
        app.autoconn.once = false;
    }
    let lang = crate::stencils::lang();
    let key = active_stencil(app);
    let stamp = key.and_then(|k| app.stencils.get(k).map(|l| (k, l.serial, lang)));
    if stamp == app.autoconn.quick_for {
        return;
    }
    app.autoconn.quick_for = stamp;
    app.autoconn.quick.clear();
    app.se.quick.clear();
    let Some(key) = key else { return };
    let picks: Vec<(usize, String, u32)> = {
        let Some(l) = app.stencils.get(key) else { return };
        l.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.texts.quick)
            .filter(|(_, e)| {
                let slot = l.doc.master(e.master).and_then(|m| m.master_shape());
                slot.is_some_and(|slot| !glue::is_one_d(&l.doc, ShapeRef { owner: Owner::Master(e.master), slot }))
            })
            .take(4)
            .map(|(k, e)| (k, String::from(e.texts.name(lang)), l.serial.wrapping_mul(0x9E37_79B9) ^ k as u32))
            .collect()
    };
    for (k, name, ver) in picks {
        let draw = crate::stencils_panel::tile_draw(app, key, k);
        app.autoconn.quick.push((key, k));
        app.se.quick.push(SeQuick { name, draw, ver });
    }
}

// where the shape of tile (skey, k) goes next to `s` in direction `dir` (0 right, 1 down,
// 2 left, 3 up, as seen) at the page's spacing, connected -> the new shape and the connector
fn place_next(tx: &mut Transaction, eng: &mut Engine, page: PageRef, s: ShapeRef, dir: u8, src: (&Document, MasterRef)) -> Result<(ShapeRef, ShapeRef), EditError> {
    let owner = Owner::Page(page);
    let gap = ac::spacing(tx.doc(), page);
    let b = structure::shape_box(tx.doc(), s);
    let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let m = instance::import_master(tx, src.0, src.1)?;
    let n = instance::place(tx, page, m, mid[0], mid[1], crate::stencils::lang())?;
    let mut sheets = Vec::new();
    collect(tx.doc(), n, &mut sheets);
    eng.recalc(tx, &sheets)?;
    let nb = structure::shape_box(tx.doc(), n);
    let (nw, nh) = (nb[2] - nb[0], nb[3] - nb[1]);
    let to = match dir {
        0 => [b[2] + gap[0] + nw * 0.5, mid[1]],
        1 => [mid[0], b[1] - gap[1] - nh * 0.5],
        2 => [b[0] - gap[0] - nw * 0.5, mid[1]],
        _ => [mid[0], b[3] + gap[1] + nh * 0.5],
    };
    let d = [to[0] - (nb[0] + nb[2]) * 0.5, to[1] - (nb[1] + nb[3]) * 0.5];
    ops::move_shapes(tx, eng, &[n], d)?;
    crate::structures::pasted(tx, eng, owner, &[n])?;
    let c = connect(tx, eng, owner, s, n)?;
    Ok((n, c))
}

// the preview half transparent: fill, its background and line of the shape, its subshapes and
// the connector
fn ghost(tx: &mut Transaction, shapes: &[ShapeRef]) -> Result<(), EditError> {
    let mut sheets = Vec::new();
    for &s in shapes {
        collect(tx.doc(), s, &mut sheets);
    }
    for cell in ["FillForegndTrans", "FillBkgndTrans", "LineColorTrans"] {
        let a = tx.single(cell);
        for &sh in &sheets {
            tx.set_constant(sh, &a, Value::Num(0.5), None)?;
        }
    }
    Ok(())
}

// the stencil document and master of a tile of the bar
fn quick_src(app: &Diagrammer, tile: u8) -> Option<(u32, usize)> {
    app.autoconn.quick.get(tile as usize).copied()
}

/// the pointer on tile `tile` of the bar at arrow `dir` of shape `key` (255: off the tiles):
/// the new shape and its connector as a live preview, half transparent (Optionen ›
/// Livevorschau aktivieren)
pub(crate) fn quick_hover(app: &mut Diagrammer, ui: &mut Ui, key: u64, dir: u8, tile: u8) {
    let live = app.opts.get(&crate::options::LIVE);
    let pick = quick_src(app, tile);
    let Some(page) = app.model.page else { return };
    let st = &app.stencils;
    let src = pick.and_then(|(skey, k)| st.entry(skey, k)).map(|(l, e)| (&l.doc, e.master));
    let Some(o) = app.opened.as_mut() else { return };
    let had = app.autoconn.preview.take().is_some();
    match (tile, src, live) {
        (0..=254, Some(src), true) => {
            let s = shape_of(page, key);
            let r = edit_doc::edit(o, t("Schnell-Shape"), false, |tx, eng| {
                let (n, c) = place_next(tx, eng, page, s, dir, src)?;
                ghost(tx, &[n, c])
            });
            if r.is_ok() {
                app.autoconn.preview = Some((key, dir, tile));
            }
        }
        _ if had => edit_doc::cancel(o),
        _ => {}
    }
    ui.repass();
}

/// a click on tile `tile` of the bar at arrow `dir` of shape `key`: the shape goes there at the
/// page's spacing and is connected; it becomes the selection
pub(crate) fn quick(app: &mut Diagrammer, ui: &mut Ui, key: u64, dir: u8, tile: u8) {
    app.autoconn.preview = None;
    let pick = quick_src(app, tile);
    let Some(page) = app.model.page else { return };
    let lang = crate::stencils::lang();
    let st = &app.stencils;
    let Some((l, e)) = pick.and_then(|(skey, k)| st.entry(skey, k)) else { return };
    let name = String::from(e.texts.name(lang));
    let src = (&l.doc, e.master);
    let Some(o) = app.opened.as_mut() else { return };
    let s = shape_of(page, key);
    let r = edit_doc::edit(o, &tf("{} ablegen", &[&name]), true, |tx, eng| place_next(tx, eng, page, s, dir, src));
    if let Ok((n, _)) = r {
        app.se.select(&[key_of(n)]);
        ui.announce(&tf("„{}“ abgelegt", &[&name]), LIVE_POLITE);
        crate::data_ask::after_drop(app, ui, &[n]);
    }
    ui.repass();
}

// ---------------------------------------------------------------- feedback

// the glow of a connector: its way thick and half transparent in the accent colour
fn glow_connector(app: &Diagrammer, ui: &mut Ui, c: ShapeRef) {
    let (Some(geo), Some(o)) = (app.model.geo, app.opened.as_ref()) else { return };
    let way = glue::path(&o.doc, c);
    if way.len() < 2 {
        return;
    }
    let vp = app.canvas.vp;
    let r = vp.rect;
    let accent = ui.color(T_ACCENT);
    ui.clip_push(r[0], r[1], r[2], r[3]);
    let pts: Vec<[f32; 2]> = way.iter().map(|&p| vp.to_screen(geo.world(p))).collect();
    let (wide, thin) = ((ui.dp(6).max(4) * 256) as i32, (ui.dp(2).max(2) * 256) as i32);
    for (c, w) in [((accent & 0x00FF_FFFF) | 0x5000_0000, wide), (accent | 0xFF00_0000, thin)] {
        ui.draw_color(c);
        for s in pts.windows(2) {
            ui.draw_line((s[0][0] * 256.0) as i32, (s[0][1] * 256.0) as i32, (s[1][0] * 256.0) as i32, (s[1][1] * 256.0) as i32, w);
        }
    }
    ui.clip_pop();
}

// the glow of a container (and the bar of a list) a box would join
fn glow_box(app: &Diagrammer, ui: &mut Ui, b: [f64; 4]) {
    let (Some(geo), Some(o)) = (app.model.geo, app.opened.as_ref()) else { return };
    let vp = app.canvas.vp;
    let r = vp.rect;
    let accent = ui.color(T_ACCENT);
    ui.clip_push(r[0], r[1], r[2], r[3]);
    let (a, z) = (vp.to_screen(geo.world([b[0], b[3]])), vp.to_screen(geo.world([b[2], b[1]])));
    let pad = ui.dp(3);
    ui.draw_color((accent & 0x00FF_FFFF) | 0x5000_0000);
    ui.draw_frame(a[0] as i32 - pad, a[1] as i32 - pad, (z[0] - a[0]) as i32 + 2 * pad, (z[1] - a[1]) as i32 + 2 * pad, ui.dp(3), ui.dp(4));
    ui.draw_color(accent);
    ui.draw_frame(a[0] as i32 - 1, a[1] as i32 - 1, (z[0] - a[0]) as i32 + 2, (z[1] - a[1]) as i32 + 2, 0, ui.dp(2).max(2));
    let _ = o;
    ui.clip_pop();
}

/// after the edit layer: the glow of the connector the shape of a running move would split
pub(crate) fn overlay(app: &mut Diagrammer, ui: &mut Ui) {
    if !app.se.busy() {
        app.autoconn.hint = None;
    }
    if let Some(c) = app.autoconn.hint {
        glow_connector(app, ui, c);
    }
}

// a master's box on the page with its middle at p, and its categories and ShapeSplit
fn master_at(doc: &Document, m: MasterRef, p: [f64; 2]) -> Option<([f64; 4], Vec<String>, bool)> {
    let slot = doc.master(m)?.master_shape()?;
    let s = ShapeRef { owner: Owner::Master(m), slot };
    if glue::is_one_d(doc, s) {
        return None;
    }
    let sheet = SheetRef::Shape(s);
    let num = |n: &str| doc.sym(n).and_then(|c| doc.resolve(sheet, &CellAddr::single(c)).num()).filter(|v| v.is_finite());
    let (w, h) = (num("Width").unwrap_or(1.0).abs(), num("Height").unwrap_or(1.0).abs());
    let b = [p[0] - w * 0.5, p[1] - h * 0.5, p[0] + w * 0.5, p[1] + h * 0.5];
    Some((b, structure::categories(doc, s), num("ShapeSplit").is_some_and(|v| v != 0.0)))
}

// Visio's category rule: an excluded category refuses; with required ones, one must be there
fn takes(cats: &[String], required: &[String], excluded: &[String]) -> bool {
    let has = |n: &String| cats.iter().any(|x| x.eq_ignore_ascii_case(n));
    !excluded.iter().any(has) && (required.is_empty() || required.iter().any(has))
}

fn split_cats(t: &str) -> Vec<String> {
    t.split(';').map(|x| String::from(x.trim())).filter(|x| !x.is_empty()).collect()
}

/// (stencils_drop.rs, while the drop target says DS_HOVER) a master dragged from the Shapes
/// panel over the canvas: the containers and lists it would join glow, so does the connector
/// it would split (SL-03, CN-08)
pub(crate) fn drag_over(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((key, k)) = app.autoconn.dragging else { return };
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let (_, x, y) = ui.drag_over_px();
    let p = geo.page(app.canvas.vp.to_world(x as f32 + 0.5, y as f32 + 0.5));
    let Some((l, e)) = app.stencils.entry(key, k) else { return };
    let Some((b, cats, splits)) = master_at(&l.doc, e.master, p) else { return };
    let Some(o) = app.opened.as_ref() else { return };
    let doc = &o.doc;
    let owner = Owner::Page(page);
    let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let holds = |o: [f64; 4], i: [f64; 4]| i[0] >= o[0] && i[1] >= o[1] && i[2] <= o[2] && i[3] <= o[3];
    let mut glows: Vec<[f64; 4]> = Vec::new();
    for c in doc.top_shapes(owner) {
        let Some(info) = structure::container(doc, c) else { continue };
        let cb = structure::shape_box(doc, c);
        let list = structure::kind(doc, c) == Some(Kind::List);
        let item = list && holds(cb, [mid[0], mid[1], mid[0], mid[1]]) && {
            let req = split_cats(&structure::user_text(doc, c, "msvSDListRequiredCategories").unwrap_or_default());
            let exc = split_cats(&structure::user_text(doc, c, "msvSDListExcludedCategories").unwrap_or_default());
            takes(&cats, &req, &exc)
        };
        if (item || (takes(&cats, &info.required, &info.excluded) && holds(cb, b))) && !info.no_highlight {
            glows.push(cb);
        }
    }
    let connector = if splits && split_on(app) && ac::page_splits(doc, page) { ac::split_target(doc, owner, b, &[]) } else { None };
    for g in glows {
        glow_box(app, ui, g);
    }
    if let Some(c) = connector {
        glow_connector(app, ui, c);
    }
}

// ---------------------------------------------------------------- several masters at once

/// (SL-03) a click on a tile with Ctrl: it joins the marked tiles (a second one leaves them);
/// the armed tile is the first of them.  -> handled (a click without Ctrl clears the marks)
pub(crate) fn mark(app: &mut Diagrammer, ui: &Ui, key: u32, k: usize) -> bool {
    let t = (key, k);
    if ui.mods() & MOD_CTRL == 0 {
        app.autoconn.multi.clear();
        return false;
    }
    let m = &mut app.autoconn.multi;
    if m.is_empty() {
        if let Some(a) = app.stencils.armed {
            m.push(a);
        }
    }
    if let Some(i) = m.iter().position(|&x| x == t) {
        m.remove(i);
    } else {
        m.push(t);
    }
    app.stencils.armed = m.first().copied();
    true
}

/// a drag of tile (key, k) begins: it is the dragged one; when it is one of several marked
/// tiles they all go with it
pub(crate) fn drag_start(app: &mut Diagrammer, key: u32, k: usize) {
    app.autoconn.dragging = Some((key, k));
    if !app.autoconn.multi.contains(&(key, k)) {
        app.autoconn.multi.clear();
    }
}

/// the marked tiles a drop of tile (key, k) places (None: only that one)
pub(crate) fn many(app: &Diagrammer, key: u32, k: usize) -> Option<Vec<(u32, usize)>> {
    let m = &app.autoconn.multi;
    (m.len() > 1 && m.contains(&(key, k))).then(|| m.clone())
}

/// (SL-03) several marked masters dropped at world point `at`: in a row from left to right at
/// the page's spacing, centred there; one undo step; they become the selection
pub(crate) fn place_many(app: &mut Diagrammer, ui: &mut Ui, tiles: &[(u32, usize)], at: [f32; 2]) {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let p = geo.page(at);
    let lang = crate::stencils::lang();
    let rules = DropRules { split: false, chain: None };
    let st = &app.stencils;
    let srcs: Vec<(&Document, MasterRef)> = tiles.iter().filter_map(|&(key, k)| st.entry(key, k)).map(|(l, e)| (&l.doc, e.master)).collect();
    let masters = crate::structures::masters(st);
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let r = edit_doc::edit(o, t("Shapes ablegen"), true, |tx, eng| {
        let gap = ac::spacing(tx.doc(), page);
        let mut made = Vec::new();
        for &(src, m) in &srcs {
            let lm = instance::import_master(tx, src, m)?;
            let s = instance::place(tx, page, lm, p[0], p[1], lang)?;
            let mut sheets = Vec::new();
            collect(tx.doc(), s, &mut sheets);
            eng.recalc(tx, &sheets)?;
            made.push(s);
        }
        // a row: widths plus the spacing, centred on the point
        let boxes: Vec<[f64; 4]> = made.iter().map(|&s| structure::shape_box(tx.doc(), s)).collect();
        let total: f64 = boxes.iter().map(|b| b[2] - b[0]).sum::<f64>() + gap[0] * (boxes.len().saturating_sub(1)) as f64;
        let mut x = p[0] - total * 0.5;
        for (&s, b) in made.iter().zip(&boxes) {
            let w = b[2] - b[0];
            let d = [x + w * 0.5 - (b[0] + b[2]) * 0.5, p[1] - (b[1] + b[3]) * 0.5];
            ops::move_shapes(tx, eng, &[s], d)?;
            x += w + gap[0];
        }
        dropped(tx, eng, owner, &made, &rules)?;
        edit_canvas::follow(tx, eng, owner, &made)?;
        for &s in &made {
            crate::structures::dropped(tx, eng, &masters, owner, s)?;
        }
        Ok(made)
    });
    if let Ok(made) = r {
        let keys: Vec<u64> = made.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
        ui.announce(&tf("{} Shapes abgelegt", &[&alloc::format!("{}", made.len())]), LIVE_POLITE);
        crate::data_ask::after_drop(app, ui, &made);
    }
    app.autoconn.multi.clear();
    app.stencils.armed = None;
    ui.repass();
}
