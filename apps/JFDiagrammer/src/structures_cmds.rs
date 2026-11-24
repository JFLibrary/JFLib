// structures_cmds -- (JD10) the commands of structures: Einfügen › Container and Beschriftung,
// the register Container-Tools › Format (RK-01), Container in the shape's menu, Shapes
// verbinden (CN-16), the Verbindungspfeile (CN-07, ZF-10)
//
//   available(app, id)            whether a command of this file can run now (commands.rs asks
//                                 through edit_cmds::available)
//   checked(app, id)              the toggles' state
//   insert_container(app, ui, k)  a container of style k around the selection (it becomes its
//                                 members), else in the middle of the view
//   insert_callout(app, ui, k)    callout master k for every selected shape (tied to it, up to
//                                 the right), else alone in the middle of the view
//   container ops                 fit, automatic size, margins, style, heading style, lock,
//                                 select the contents, disband, add, remove; (JD21) 14 styles,
//                                 a style applied through container_styles::apply
//   connect_shapes(app, ui)       the selected shapes chained by dynamic connectors in the
//                                 order they were selected; (JD15) by instances of the
//                                 connector master marked in the Shapes panel (the armed tile,
//                                 a 1-D master), its ends glued dynamically
//   arrow(app, ui, key, dir)      a click on a Verbindungspfeil: the nearest shape in that
//                                 direction is connected, else the shape chosen in the Shapes
//                                 panel (the armed tile, else the first quick shape of the
//                                 active stencil, else a copy of the shape) goes there at the
//                                 page's spacing (AvenueSizeX/Y) and is connected
//   arrows_toggle(app, ui)        Ansicht › Verbindungspfeile: the drawing's own switch
//                                 (DocumentSheet User.msvNoAutoConnect), Visio's
//
// Every command is one undo step (edit_doc::edit) named after it.

use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::structure::{self, cff, Kind};
use diagram_core::{glue, instance, ops, CellAddr, EditError, Owner, PageRef, ShapeRef, SheetRef, Transaction, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of};
use crate::edit_doc;
use crate::structures::{self, place_master};
use crate::Diagrammer;

/// the container masters' style names (msvSDContainerStyle 1..14; JD21: 7..14), the commands
/// that apply them
pub(crate) const CONTAINER_STYLES: [u32; 14] = crate::container_styles::STYLES;
/// Einfügen › Container: a new container in style 1..14
pub(crate) const CONTAINER_INSERTS: [u32; 14] = [
    CONTAINER_1, CONTAINER_2, CONTAINER_3, CONTAINER_4, CONTAINER_5, CONTAINER_6, CONTAINER_7, CONTAINER_8, CONTAINER_9, CONTAINER_10, CONTAINER_11,
    CONTAINER_12, CONTAINER_13, CONTAINER_14,
];
/// the heading styles (msvSDHeadingStyle 1..4)
pub(crate) const HEADING_STYLES: [u32; 4] = [CT_HEADING_1, CT_HEADING_2, CT_HEADING_3, CT_HEADING_4];
/// Einfügen › Beschriftung: the callout masters
pub(crate) const CALLOUTS: [(u32, &str); 6] = [
    (CALLOUT_1, "Callout box"),
    (CALLOUT_2, "Callout rounded"),
    (CALLOUT_3, "Callout bubble"),
    (CALLOUT_4, "Callout line"),
    (CALLOUT_5, "Callout bracket"),
    (CALLOUT_6, "Callout circle"),
];
/// the margins: none, narrow, medium, wide (mm; US: inches)
const MARGINS: [(u32, f64, f64); 4] = [(CT_MARGIN_0, 0.0, 0.0), (CT_MARGIN_1, 2.0, 0.08), (CT_MARGIN_2, 5.0, 0.2), (CT_MARGIN_3, 10.0, 0.4)];
/// the automatic sizes (msvSDContainerResize)
const RESIZES: [(u32, f64); 3] = [(CT_RESIZE_0, 0.0), (CT_RESIZE_1, 1.0), (CT_RESIZE_2, 2.0)];

/// whether `id` is one of this file's commands
pub(crate) fn owns(id: u32) -> bool {
    CONTAINER_STYLES.contains(&id)
        || CONTAINER_INSERTS.contains(&id)
        || HEADING_STYLES.contains(&id)
        || CALLOUTS.iter().any(|c| c.0 == id)
        || MARGINS.iter().any(|m| m.0 == id)
        || RESIZES.iter().any(|r| r.0 == id)
        || matches!(id, CT_FIT | CT_LOCK | CT_SELECT | CT_DISBAND | CT_ADD | CT_REMOVE | CONNECT_SHAPES | CONNECT_ARROWS | CONTAINER | CALLOUT)
        || crate::structures_cff::owns(id)
}

/// whether a command can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let open = app.opened.is_some();
    if crate::structures_cff::owns(id) {
        return crate::structures_cff::available(app, id);
    }
    if CONTAINER_INSERTS.contains(&id) || CALLOUTS.iter().any(|c| c.0 == id) || matches!(id, CONTAINER | CALLOUT | CONNECT_ARROWS) {
        return open;
    }
    match id {
        CONNECT_SHAPES => app.se.sel.len() > 1,
        CT_ADD => !addable(app).is_empty(),
        CT_REMOVE => removable(app),
        _ => structures::selected_container(app).is_some(),
    }
}

/// the toggles' state
pub(crate) fn checked(app: &Diagrammer, id: u32) -> bool {
    if crate::structures_cff::owns(id) {
        return crate::structures_cff::checked(app, id);
    }
    if id == CONNECT_ARROWS {
        return app.structs.arrows;
    }
    let (Some(o), Some(c)) = (app.opened.as_ref(), structures::selected_container(app)) else { return false };
    let Some(info) = structure::container(&o.doc, c) else { return false };
    let us = !crate::edit_canvas::metric(app);
    if let Some(k) = CONTAINER_STYLES.iter().position(|&x| x == id) {
        return info.style as usize == k + 1;
    }
    if let Some(k) = HEADING_STYLES.iter().position(|&x| x == id) {
        return info.heading_style as usize == k + 1;
    }
    if let Some(m) = MARGINS.iter().find(|m| m.0 == id) {
        let want = if us { m.2 } else { m.1 / 25.4 };
        return (info.margin - want).abs() < 1e-3;
    }
    if let Some(r) = RESIZES.iter().find(|r| r.0 == id) {
        return match info.resize {
            structure::Resize::None => r.1 == 0.0,
            structure::Resize::Expand => r.1 == 1.0,
            structure::Resize::Fit => r.1 == 2.0,
        };
    }
    id == CT_LOCK && info.locked
}

/// run a command of this file -> handled
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    if crate::structures_cff::owns(id) {
        crate::structures_cff::run(app, ui, id);
        return true;
    }
    if let Some(k) = CONTAINER_INSERTS.iter().position(|&x| x == id) {
        insert_container(app, ui, k as u32 + 1);
    } else if let Some(k) = CALLOUTS.iter().position(|c| c.0 == id) {
        insert_callout(app, ui, k);
    } else if let Some(k) = CONTAINER_STYLES.iter().position(|&x| x == id) {
        // (JD21) the heading style follows when the new style lacks it (container_styles.rs)
        crate::container_styles::apply(app, ui, k as u32 + 1, true);
    } else if let Some(k) = HEADING_STYLES.iter().position(|&x| x == id) {
        set_cell(app, ui, t("Kopfzeilenstil"), "msvSDHeadingStyle", k as f64 + 1.0, None);
    } else if let Some(m) = MARGINS.iter().find(|m| m.0 == id) {
        let us = !crate::edit_canvas::metric(app);
        let (v, u) = if us { (m.2, Unit::Inches) } else { (m.1 / 25.4, Unit::Millimeters) };
        set_cell(app, ui, t("Ränder"), "msvSDContainerMargin", v, Some(u));
    } else if let Some(r) = RESIZES.iter().find(|r| r.0 == id) {
        set_cell(app, ui, t("Automatische Größe"), "msvSDContainerResize", r.1, None);
    } else {
        match id {
            CONTAINER => insert_container(app, ui, 1),
            CALLOUT => insert_callout(app, ui, 0),
            CT_FIT => fit(app, ui),
            CT_LOCK => {
                let on = checked(app, CT_LOCK);
                set_cell(app, ui, t("Container sperren"), "msvSDContainerLocked", if on { 0.0 } else { 1.0 }, Some(Unit::Bool));
            }
            CT_SELECT => select_contents(app, ui),
            CT_DISBAND => disband(app, ui),
            CT_ADD => add(app, ui),
            CT_REMOVE => remove(app, ui),
            CONNECT_SHAPES => connect_shapes(app, ui),
            CONNECT_ARROWS => arrows_toggle(app, ui),
            _ => return false,
        }
    }
    ui.repass();
    true
}

// ---------------------------------------------------------------- inserting

/// a new container of style `style` around the selection (it becomes its members), else in the
/// middle of the view
pub(crate) fn insert_container(app: &mut Diagrammer, ui: &mut Ui, style: u32) {
    let (Some(page), Some(geo)) = (app.model.page, app.model.geo) else { return };
    let sel: Vec<ShapeRef> = edit_canvas::selected(app);
    let centre = geo.page([app.canvas.vp.cx as f32, app.canvas.vp.cy as f32]);
    let ms = structures::masters(&app.stencils);
    let Some(src) = ms.containers else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let r = edit_doc::edit(o, t("Container einfügen"), true, |tx, eng| {
        let members: Vec<ShapeRef> = sel.iter().copied().filter(|&s| structure::joins_by_position(tx.doc(), s)).collect();
        let b = ops::bounds(tx.doc(), &members);
        let at = b.map_or(centre, |b| [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5]);
        let c = place_master(tx, eng, src, "Container", page, at)?;
        let sheet = SheetRef::Shape(c);
        tx.set_user(sheet, "msvSDContainerStyle", Value::Num(style as f64), None)?;
        eng.recalc(tx, &[sheet])?;
        // behind what it holds
        ops::reorder(tx, &[c], ops::Z::Back)?;
        if !members.is_empty() {
            structure::add_members(tx, eng, c, &members, None)?;
            structure::fit(tx, eng, c, true)?;
        }
        Ok(c)
    });
    if let Ok(c) = r {
        let _ = owner;
        app.se.select(&[key_of(c)]);
    }
    ui.repass();
}

/// callout `k` (CALLOUTS) for each selected shape, tied to it; without a selection one in the
/// middle of the view
pub(crate) fn insert_callout(app: &mut Diagrammer, ui: &mut Ui, k: usize) {
    let (Some(page), Some(geo)) = (app.model.page, app.model.geo) else { return };
    let sel: Vec<ShapeRef> = edit_canvas::selected(app);
    let centre = geo.page([app.canvas.vp.cx as f32, app.canvas.vp.cy as f32]);
    let ms = structures::masters(&app.stencils);
    let Some(src) = ms.callouts else { return };
    let name = CALLOUTS[k.min(CALLOUTS.len() - 1)].1;
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let r = edit_doc::edit(o, t("Beschriftung einfügen"), true, |tx, eng| {
        let targets: Vec<ShapeRef> = sel.iter().copied().filter(|&s| structure::kind(tx.doc(), s) != Some(Kind::Callout) && !glue::is_one_d(tx.doc(), s)).collect();
        let mut made = Vec::new();
        if targets.is_empty() {
            made.push(place_master(tx, eng, src, name, page, centre)?);
        }
        for t in targets {
            let tb = structure::shape_box(tx.doc(), t);
            let c = place_master(tx, eng, src, name, page, [tb[2], tb[3]])?;
            // the tip on the target's upper right part
            let p = [tb[0] + (tb[2] - tb[0]) * 0.75, tb[1] + (tb[3] - tb[1]) * 0.75];
            structures::attach_callout(tx, eng, c, t, p)?;
            made.push(c);
        }
        edit_canvas::follow(tx, eng, owner, &made)?;
        Ok(made)
    });
    if let Ok(made) = r {
        let keys: Vec<u64> = made.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
    }
    ui.repass();
}

// ---------------------------------------------------------------- Container-Tools › Format

// a User cell of the selected container, then its size again
fn set_cell(app: &mut Diagrammer, ui: &mut Ui, label: &str, cell: &str, v: f64, unit: Option<Unit>) {
    let Some(c) = structures::selected_container(app) else { return };
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, label, true, |tx, eng| {
        let sheet = SheetRef::Shape(c);
        tx.set_user(sheet, cell, Value::Num(v), unit)?;
        eng.recalc(tx, &[sheet])?;
        // a new heading, margin or size rule: the container around its members again
        let info = structure::container(tx.doc(), c);
        if let Some(info) = info {
            if info.resize != structure::Resize::None {
                structure::fit(tx, eng, c, info.resize == structure::Resize::Fit)?;
            }
        }
        let moved = structure::settle(tx, eng, owner, &[c], false)?;
        let mut all = moved;
        all.push(c);
        edit_canvas::follow(tx, eng, owner, &all)
    });
    ui.repass();
}

/// An Inhalt anpassen
fn fit(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(c) = structures::selected_container(app) else { return };
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("An Inhalt anpassen"), true, |tx, eng| {
        if structure::fit(tx, eng, c, true)? {
            let mut moved = structure::settle(tx, eng, owner, &[c], false)?;
            moved.push(c);
            edit_canvas::follow(tx, eng, owner, &moved)?;
        }
        Ok(())
    });
    ui.repass();
}

/// Inhalt auswählen
fn select_contents(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(c) = structures::selected_container(app) else { return };
    let Some(o) = app.opened.as_ref() else { return };
    let keys: Vec<u64> = structure::members(&o.doc, c).into_iter().map(key_of).collect();
    app.se.select(&keys);
    ui.repass();
}

/// Container auflösen: the container goes, its members stay
fn disband(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(c) = structures::selected_container(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let keep: Vec<u64> = structure::members(&o.doc, c).into_iter().map(key_of).collect();
    if edit_doc::edit(o, t("Container auflösen"), true, |tx, eng| structure::disband(tx, eng, c)).is_ok() {
        app.se.select(&keep);
    }
    ui.repass();
}

// the selected shapes and the containers they lie in without being their members
fn addable(app: &Diagrammer) -> Vec<(ShapeRef, ShapeRef)> {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return Vec::new() };
    let doc = &o.doc;
    let sel = edit_canvas::selected(app);
    let mut out = Vec::new();
    for &s in &sel {
        if !structure::joins_by_position(doc, s) {
            continue;
        }
        let now = structure::containers_of(doc, s);
        for c in structure::drop_targets(doc, Owner::Page(page), s, &sel) {
            if !now.contains(&c) {
                out.push((c, s));
            }
        }
    }
    out
}

fn removable(app: &Diagrammer) -> bool {
    let Some(o) = app.opened.as_ref() else { return false };
    edit_canvas::selected(app).iter().any(|&s| !structure::containers_of(&o.doc, s).is_empty())
}

/// Zu Container hinzufügen: the selection joins the containers it lies in
fn add(app: &mut Diagrammer, ui: &mut Ui) {
    let pairs = addable(app);
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Zu Container hinzufügen"), true, |tx, eng| {
        let mut cs = Vec::new();
        for (c, s) in pairs {
            structure::add_members(tx, eng, c, &[s], None)?;
            cs.push(c);
        }
        let moved = structure::settle(tx, eng, owner, &cs, false)?;
        edit_canvas::follow(tx, eng, owner, &moved)
    });
    ui.repass();
}

/// Aus Container entfernen: the selection leaves its containers (it stays where it is)
fn remove(app: &mut Diagrammer, ui: &mut Ui) {
    let sel = edit_canvas::selected(app);
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Aus Container entfernen"), true, |tx, eng| {
        let mut lists = Vec::new();
        for &s in &sel {
            for c in structure::containers_of(tx.doc(), s) {
                structure::remove_members(tx, eng, c, &[s])?;
                lists.push(c);
            }
        }
        let moved = structure::settle(tx, eng, owner, &lists, false)?;
        edit_canvas::follow(tx, eng, owner, &moved)
    });
    ui.repass();
}

// ---------------------------------------------------------------- connecting

// a dynamic connector from shape a to shape b, both ends glued to the whole shape (dynamic);
// (JD15, CN-16) with `master` (a 1-D master of a stencil) an instance of it instead
fn connector(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: ShapeRef, b: ShapeRef, master: Option<(&diagram_core::Document, diagram_core::MasterRef)>) -> Result<ShapeRef, EditError> {
    let (pa, pb) = {
        let doc = tx.doc();
        (ops::frame(doc, a).centre(), ops::frame(doc, b).centre())
    };
    let c = match (master, owner) {
        (Some((src, m)), Owner::Page(page)) => {
            let lm = instance::import_master(tx, src, m)?;
            let c = instance::place(tx, page, lm, (pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, crate::stencils::lang())?;
            eng.recalc(tx, &[SheetRef::Shape(c)])?;
            glue::glue(tx, eng, c, glue::End::Begin, pa, glue::Target::Shape(a))?;
            glue::glue(tx, eng, c, glue::End::End, pb, glue::Target::Shape(b))?;
            c
        }
        _ => glue::new_connector(tx, eng, owner, (pa, glue::Target::Shape(a)), (pb, glue::Target::Shape(b)))?,
    };
    glue::update(tx, eng, &[c])?;
    Ok(c)
}

/// (JD15, CN-16) the connector master of the tile marked in the Shapes panel (the armed one),
/// when its master is a 1-D shape: (stencil key, tile)
fn marked_connector(app: &Diagrammer) -> Option<(u32, usize)> {
    let (key, k) = app.stencils.armed?;
    let (l, e) = app.stencils.entry(key, k)?;
    let slot = l.doc.master(e.master)?.master_shape()?;
    glue::is_one_d(&l.doc, ShapeRef { owner: Owner::Master(e.master), slot }).then_some((key, k))
}

/// Shapes verbinden: the selection chained in the order it was made (CN-16); (JD15) with the
/// connector master marked in the Shapes panel, else the dynamic connector
pub(crate) fn connect_shapes(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes: Vec<ShapeRef> = structures::ordered(app);
    let Some(page) = app.model.page else { return };
    let marked = marked_connector(app);
    let st = &app.stencils;
    let master = marked.and_then(|(key, k)| st.entry(key, k)).map(|(l, e)| (&l.doc, e.master));
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let _ = edit_doc::edit(o, t("Shapes verbinden"), true, |tx, eng| {
        let list: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| !glue::is_one_d(tx.doc(), s)).collect();
        for w in list.windows(2) {
            connector(tx, eng, owner, w[0], w[1], master)?;
        }
        Ok(())
    });
    if marked.is_some() {
        app.stencils.armed = None;
    }
    ui.repass();
}

// the page's spacing between shapes (Seite einrichten › Layout und Routing: AvenueSizeX/Y;
// Visio's metric default 7.5 mm) in drawing units
fn spacing(doc: &diagram_core::Document, page: PageRef) -> [f64; 2] {
    let s = SheetRef::Page(page);
    let get = |n: &str| doc.sym(n).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite() && *v > 0.0);
    let scale = match (get("PageScale"), get("DrawingScale")) {
        (Some(ps), Some(ds)) => ds / ps,
        _ => 1.0,
    };
    [get("AvenueSizeX").unwrap_or(7.5 / 25.4) * scale, get("AvenueSizeY").unwrap_or(7.5 / 25.4) * scale]
}

/// the nearest shape from `s` in direction `dir` (0 right, 1 down, 2 left, 3 up on the
/// screen): its middle within 45° of the direction and not farther than a few of s's sizes
fn neighbour(doc: &diagram_core::Document, owner: Owner, s: ShapeRef, dir: u8) -> Option<ShapeRef> {
    let b = structure::shape_box(doc, s);
    let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let reach = 4.0 * (b[2] - b[0]).max(b[3] - b[1]) + 1.0;
    let v: [f64; 2] = match dir {
        0 => [1.0, 0.0],
        1 => [0.0, -1.0],
        2 => [-1.0, 0.0],
        _ => [0.0, 1.0],
    };
    let mut best: Option<(f64, ShapeRef)> = None;
    for o in doc.top_shapes(owner) {
        if o == s || doc.shape(o).is_none_or(|x| x.is_deleted() || x.header().kind == diagram_core::ShapeKind::Guide) {
            continue;
        }
        if glue::is_one_d(doc, o) || structure::is_container(doc, o) || structure::kind(doc, o) == Some(Kind::Callout) {
            continue;
        }
        let ob = structure::shape_box(doc, o);
        let oc = [(ob[0] + ob[2]) * 0.5, (ob[1] + ob[3]) * 0.5];
        let d = [oc[0] - c[0], oc[1] - c[1]];
        let along = d[0] * v[0] + d[1] * v[1];
        let across = (d[0] * v[1] - d[1] * v[0]).abs();
        if along <= 0.0 || across > along || along > reach {
            continue;
        }
        let dist = along + across;
        if best.is_none_or(|x| dist < x.0) {
            best = Some((dist, o));
        }
    }
    best.map(|x| x.1)
}

/// a click on a Verbindungspfeil of shape `key` in direction `dir`
pub(crate) fn arrow(app: &mut Diagrammer, ui: &mut Ui, key: u64, dir: u8) {
    let Some(page) = app.model.page else { return };
    let owner = Owner::Page(page);
    let s = edit_canvas::shape_of(page, key);
    // what goes there when no shape is near: the armed tile, the first quick shape of the active
    // stencil, else a copy of the shape
    let pick: Option<(u32, usize)> = app.stencils.armed.or_else(|| {
        let key = app.stencils.panel.active;
        let l = app.stencils.get(key)?;
        let k = l.entries.iter().position(|e| e.texts.quick).unwrap_or(0);
        (!l.entries.is_empty()).then_some((key, k))
    });
    let st = &app.stencils;
    let src = pick.and_then(|(key, k)| st.entry(key, k));
    let clip = app.opened.as_ref().map(|o| ops::copy(&o.doc, &[s]));
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Verbindungspfeil"), true, |tx, eng| {
        if let Some(n) = neighbour(tx.doc(), owner, s, dir) {
            connector(tx, eng, owner, s, n, None)?;
            return Ok(None);
        }
        let gap = spacing(tx.doc(), page);
        let b = structure::shape_box(tx.doc(), s);
        // the new shape: its box next to s at the gap, middles in line
        let new = match src {
            Some((l, e)) => {
                let m = instance::import_master(tx, &l.doc, e.master)?;
                let n = instance::place(tx, page, m, (b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5, crate::stencils::lang())?;
                let mut sheets = Vec::new();
                collect(tx.doc(), n, &mut sheets);
                eng.recalc(tx, &sheets)?;
                n
            }
            None => {
                let Some(clip) = &clip else { return Err(EditError::NoShape) };
                let made = ops::paste(tx, eng, owner, clip, [0.0, 0.0])?;
                *made.first().ok_or(EditError::NoShape)?
            }
        };
        let nb = structure::shape_box(tx.doc(), new);
        let (nw, nh) = (nb[2] - nb[0], nb[3] - nb[1]);
        let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        let to_mid = match dir {
            0 => [b[2] + gap[0] + nw * 0.5, mid[1]],
            1 => [mid[0], b[1] - gap[1] - nh * 0.5],
            2 => [b[0] - gap[0] - nw * 0.5, mid[1]],
            _ => [mid[0], b[3] + gap[1] + nh * 0.5],
        };
        let d = [to_mid[0] - (nb[0] + nb[2]) * 0.5, to_mid[1] - (nb[1] + nb[3]) * 0.5];
        ops::move_shapes(tx, eng, &[new], d)?;
        structures::pasted(tx, eng, owner, &[new])?;
        connector(tx, eng, owner, s, new, None)?;
        Ok(Some(new))
    });
    if let Ok(Some(n)) = r {
        app.se.select(&[key_of(n)]);
        if app.stencils.armed.is_some() && ui.mods() & MOD_SHIFT == 0 {
            app.stencils.armed = None;
        }
    }
    ui.repass();
}

fn collect(doc: &diagram_core::Document, s: ShapeRef, out: &mut Vec<SheetRef>) {
    out.push(SheetRef::Shape(s));
    if let Some(sh) = doc.shape(s) {
        for c in sh.children() {
            collect(doc, c, out);
        }
    }
}

/// Ansicht › Verbindungspfeile: the drawing's switch (DocumentSheet User.msvNoAutoConnect)
pub(crate) fn arrows_toggle(app: &mut Diagrammer, ui: &mut Ui) {
    let on = !app.structs.arrows;
    app.structs.arrows = on;
    if let Some(o) = app.opened.as_mut() {
        let _ = edit_doc::edit(o, t("Verbindungspfeile"), true, |tx, _| tx.set_user(SheetRef::Document, "msvNoAutoConnect", Value::Num(if on { 0.0 } else { 1.0 }), None));
    }
    ui.repass();
}

/// the drawing's switch when it opens: arrows unless msvNoAutoConnect is set
pub(crate) fn arrows_of(doc: &diagram_core::Document) -> bool {
    !doc.user_cells(SheetRef::Document).iter().any(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case("msvNoAutoConnect")) && r.num("Value").is_some_and(|v| v != 0.0))
}

/// the shape flags of the edit layer for structures: no arrows on containers, lists, callouts,
/// shapes of a flowchart's frame
pub(crate) fn no_arrows(doc: &diagram_core::Document, s: ShapeRef) -> bool {
    structure::is_container(doc, s) || structure::kind(doc, s) == Some(Kind::Callout) || cff::find(doc, s).is_some_and(|f| f.pool == s)
}
