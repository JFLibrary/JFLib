// structures_cff -- (JD10) the cross-functional flowchart in the app (ST-05, RK-03): its
// register Funktionsübergreifendes Flussdiagramm and the flowchart a dropped lane brings
//
//   around(tx, eng, src, page, s, lane)   a lane (or phase) dropped outside a flowchart: the
//                                   flowchart around it (pool, lane list, phase list, a phase or
//                                   a lane to go with it), Visio's five shapes
//   new_flowchart(app, ui)          Swimlane on a page without a flowchart: one with two lanes
//                                   and a phase in the middle of the view
//   insert_lane / insert_phase      after the selected lane / phase, else at the end
//   title, separators, upright labels   the switches of Entwurf (pool / phases / lanes cells)
//   orientation                     new flowcharts horizontal or vertical
//
// The masters come from the built-in stencil "cff" (stencils/cff): Swimlane, Separator, their
// vertical forms and the hidden CFF Container, Swimlane List, Phase List.  The structure and
// the layout are diagram_core::structure::cff's.

use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::structure::{self, cff};
use diagram_core::{Document, EditError, Owner, PageRef, ShapeRef, SheetRef, Transaction, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of};
use crate::edit_doc;
use crate::structures::{self, place_master};
use crate::Diagrammer;

/// whether `id` is a command of the flowchart's register
pub(crate) fn owns(id: u32) -> bool {
    matches!(id, CFF_LANE | CFF_SEPARATOR | CFF_HORIZONTAL | CFF_VERTICAL | CFF_TITLE | CFF_SEPARATORS | CFF_ROTATE | CFF_ORIENTATION)
}

// the flowchart of the selection, else the page's first
fn current(app: &Diagrammer) -> Option<cff::Cff> {
    let o = app.opened.as_ref()?;
    let page = app.model.page?;
    let doc = &o.doc;
    edit_canvas::selected(app).into_iter().find_map(|s| cff::find(doc, s)).or_else(|| cff::pools(doc, Owner::Page(page)).into_iter().next())
}

pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        CFF_LANE | CFF_HORIZONTAL | CFF_VERTICAL | CFF_ORIENTATION => app.opened.is_some(),
        _ => current(app).is_some(),
    }
}

fn user(doc: &Document, s: ShapeRef, name: &str) -> Option<f64> {
    structure::user_num(doc, s, name)
}

pub(crate) fn checked(app: &Diagrammer, id: u32) -> bool {
    let f = current(app);
    let doc = app.opened.as_ref().map(|o| &o.doc);
    match (id, f, doc) {
        (CFF_HORIZONTAL, Some(f), _) => f.horizontal,
        (CFF_VERTICAL, Some(f), _) => !f.horizontal,
        (CFF_HORIZONTAL, None, _) => !app.structs.cff_vertical,
        (CFF_VERTICAL, None, _) => app.structs.cff_vertical,
        (CFF_TITLE, Some(f), Some(doc)) => user(doc, f.pool, "JFD_HeadingSize").is_some_and(|v| v > 0.0),
        (CFF_SEPARATORS, Some(f), Some(doc)) => cff::phases(doc, &f).iter().any(|&p| user(doc, p, "JFD_HeadingSize").is_some_and(|v| v > 0.0)),
        (CFF_ROTATE, Some(f), Some(doc)) => cff::lanes(doc, &f).first().is_some_and(|&l| user(doc, l, "JFD_LabelUpright").is_some_and(|v| v != 0.0)),
        _ => false,
    }
}

pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    match id {
        CFF_LANE => insert(app, ui, true),
        CFF_SEPARATOR => insert(app, ui, false),
        CFF_HORIZONTAL => app.structs.cff_vertical = false,
        CFF_VERTICAL => app.structs.cff_vertical = true,
        CFF_ORIENTATION => app.structs.cff_vertical = !app.structs.cff_vertical,
        CFF_TITLE => switch(app, ui, id),
        CFF_SEPARATORS => switch(app, ui, id),
        CFF_ROTATE => switch(app, ui, id),
        _ => {}
    }
    ui.repass();
}

// the masters of one orientation
fn lane_master(horizontal: bool) -> &'static str {
    if horizontal { "Swimlane" } else { "Swimlane (vertical)" }
}

fn phase_master(horizontal: bool) -> &'static str {
    if horizontal { "Separator" } else { "Separator (vertical)" }
}

/// the flowchart around a dropped lane (`lane`) or phase `s` that joined none: pool, both
/// lists and the other part (a phase for a lane, a lane for a phase), laid out from the
/// dropped shape's top left corner
pub(crate) fn around(tx: &mut Transaction, eng: &mut Engine, src: &Document, page: PageRef, s: ShapeRef, lane: bool) -> Result<(), EditError> {
    // the orientation from the dropped shape: a lane's heading on its left is horizontal
    let edge = structure::user_text(tx.doc(), s, "JFD_HeadingEdge").unwrap_or_default();
    let horizontal = if lane { edge.eq_ignore_ascii_case("Left") } else { edge.eq_ignore_ascii_case("Top") };
    let b = structure::shape_box(tx.doc(), s);
    build(tx, eng, src, page, [b[0], b[3]], horizontal, if lane { Some(s) } else { None }, if lane { None } else { Some(s) }, 1)
}

// a flowchart from top left corner `at`: the given lane (else `lanes` new ones) and phase
fn build(tx: &mut Transaction, eng: &mut Engine, src: &Document, page: PageRef, at: [f64; 2], horizontal: bool, lane: Option<ShapeRef>, phase: Option<ShapeRef>, lanes: usize) -> Result<(), EditError> {
    let owner = Owner::Page(page);
    let pool = place_master(tx, eng, src, "CFF Container", page, at)?;
    let ll = place_master(tx, eng, src, "Swimlane List", page, at)?;
    let pl = place_master(tx, eng, src, "Phase List", page, at)?;
    // the directions of the lists: lanes across the flow, phases along it
    for (l, d) in [(ll, if horizontal { 2.0 } else { 0.0 }), (pl, if horizontal { 0.0 } else { 2.0 })] {
        tx.set_user(SheetRef::Shape(l), "msvSDListDirection", Value::Num(d), None)?;
    }
    let mut ls: Vec<ShapeRef> = lane.into_iter().collect();
    while ls.len() < lanes.max(1) {
        ls.push(place_master(tx, eng, src, lane_master(horizontal), page, at)?);
    }
    let ps: Vec<ShapeRef> = match phase {
        Some(p) => alloc::vec![p],
        None => alloc::vec![place_master(tx, eng, src, phase_master(horizontal), page, at)?],
    };
    // the pool's top left corner at `at`
    let pb = structure::shape_box(tx.doc(), pool);
    diagram_core::ops::move_shapes(tx, eng, &[pool], [at[0] - pb[0], at[1] - pb[3]])?;
    // the frame behind everything: pool, lists, phases, lanes
    for &s in ls.iter().chain(ps.iter()).chain([pl, ll, pool].iter()) {
        diagram_core::ops::reorder(tx, &[s], diagram_core::ops::Z::Back)?;
    }
    structure::add_members(tx, eng, pool, &[ll, pl], None)?;
    structure::add_members(tx, eng, ll, &ls, None)?;
    structure::add_members(tx, eng, pl, &ps, None)?;
    let f = cff::of_pool(tx.doc(), pool).ok_or(EditError::Invalid)?;
    let moved = cff::layout(tx, eng, &f)?;
    edit_canvas::follow(tx, eng, owner, &moved)
}

// Swimlane / Trennlinie: after the selected one, else at the end; without a flowchart a new one
fn insert(app: &mut Diagrammer, ui: &mut Ui, lane: bool) {
    let (Some(page), Some(geo)) = (app.model.page, app.model.geo) else { return };
    let f = current(app);
    let sel = edit_canvas::selected(app);
    let vertical = app.structs.cff_vertical;
    let centre = geo.page([app.canvas.vp.cx as f32, app.canvas.vp.cy as f32]);
    let ms = structures::masters(&app.stencils);
    let Some(src) = ms.cff else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let label = if lane { t("Bahn einfügen") } else { t("Trennlinie einfügen") };
    let r = edit_doc::edit(o, label, true, |tx, eng| {
        let Some(f) = f else {
            // a new flowchart: two lanes, a phase
            let at = [centre[0] - 4.0, centre[1] + 2.0];
            build(tx, eng, src, page, at, !vertical, None, None, 2)?;
            return Ok(None);
        };
        let list = if lane { f.lane_list } else { f.phase_list };
        let Some(list) = list else { return Err(EditError::Invalid) };
        let items = structure::items(tx.doc(), list);
        // after the selected item (or the item a selected shape is in)
        let after = sel.iter().find_map(|&s| {
            if let Some(i) = items.iter().position(|&x| x == s) {
                return Some(i);
            }
            let cs = structure::containers_of(tx.doc(), s);
            items.iter().position(|x| cs.contains(x))
        });
        let at = after.map_or(items.len(), |i| i + 1);
        let pb = structure::shape_box(tx.doc(), f.pool);
        let name = if lane { lane_master(f.horizontal) } else { phase_master(f.horizontal) };
        let s = place_master(tx, eng, src, name, page, [pb[2], pb[1]])?;
        // the size of its neighbour (a lane as long as the others)
        if let Some(&n) = items.get(after.unwrap_or(items.len().saturating_sub(1))) {
            let nb = structure::shape_box(tx.doc(), n);
            let sb = structure::shape_box(tx.doc(), s);
            let to = [sb[0], sb[3] - (nb[3] - nb[1]), sb[0] + (nb[2] - nb[0]), sb[3]];
            diagram_core::ops::set_frame(tx, eng, s, &diagram_core::ops::Frame::from_box(to))?;
        }
        diagram_core::ops::reorder(tx, &[s], diagram_core::ops::Z::Back)?;
        for p in [list, f.pool] {
            diagram_core::ops::reorder(tx, &[p], diagram_core::ops::Z::Back)?;
        }
        structure::add_members(tx, eng, list, &[s], Some(at))?;
        let moved = cff::layout(tx, eng, &f)?;
        edit_canvas::follow(tx, eng, owner, &moved)?;
        Ok(Some(s))
    });
    if let Ok(Some(s)) = r {
        app.se.select(&[key_of(s)]);
    }
    ui.repass();
}

// Titelleiste / Trennlinien / Bahnbeschriftungen drehen: the cells of the pool, the phases, the
// lanes and phases; then the layout again
fn switch(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some(f) = current(app) else { return };
    let on = checked(app, id);
    let Some(page) = app.model.page else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let label = crate::commands::label(id);
    let _ = edit_doc::edit(o, label, true, |tx, eng| {
        let doc = tx.doc();
        let (lanes, phases) = (cff::lanes(doc, &f), cff::phases(doc, &f));
        match id {
            CFF_TITLE => heading(tx, eng, f.pool, !on)?,
            CFF_SEPARATORS => {
                for &p in &phases {
                    heading(tx, eng, p, !on)?;
                    // the phase's line goes with its heading
                    let a = tx.single("LinePattern");
                    if on {
                        tx.force_constant(SheetRef::Shape(p), &a, Value::Num(0.0), None)?;
                    } else {
                        tx.inherit_cell(SheetRef::Shape(p), &a)?;
                    }
                }
            }
            _ => {
                for &s in lanes.iter().chain(phases.iter()) {
                    tx.set_user(SheetRef::Shape(s), "JFD_LabelUpright", Value::Num(if on { 0.0 } else { 1.0 }), Some(Unit::Bool))?;
                    eng.recalc(tx, &[SheetRef::Shape(s)])?;
                }
            }
        }
        let moved = cff::layout(tx, eng, &f)?;
        edit_canvas::follow(tx, eng, owner, &moved)
    });
    ui.repass();
}

// a heading shown (the master's size again) or hidden (0)
fn heading(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, show: bool) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    let Some(a) = structure::user_addr(tx.doc(), "JFD_HeadingSize") else { return Ok(()) };
    if show {
        tx.inherit_cell(sheet, &a)?;
    } else {
        tx.set_user(sheet, "JFD_HeadingSize", Value::Num(0.0), Some(Unit::Millimeters))?;
    }
    eng.recalc(tx, &[sheet])?;
    eng.recalc_cells(tx, &[(sheet, a)])?;
    Ok(())
}
