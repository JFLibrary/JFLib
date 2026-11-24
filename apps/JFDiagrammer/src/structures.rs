// structures -- (JD10) containers, lists, swimlanes and callouts on the canvas: what the
// gestures of edit_canvas do with them (diagram_core::structure), the glow of a container a
// dragged shape would join, the insertion bar and the insert triangles of a list, clicks through
// a container's inside
//
//   Structs                          the app's state: the shapes of a running move, the order the
//                                    selection was made in, the marks shown, new flowcharts'
//                                    orientation
//   move_shapes(tx, eng, owner, shapes, d, done)   a move: members and callouts come along;
//                                    released: membership by the new place, lists, sizes
//   changed(tx, eng, owner, shapes)  after a resize or a turn: lists, flowcharts, sizes
//   dropped(tx, eng, m, owner, shape)  a shape from the Shapes panel: a callout dropped on a
//                                    shape takes it as its target; a lane dropped on the page
//                                    brings its flowchart; then it joins where it lies
//   delete(o, owner, shapes)         members and callouts go along; lists close the gap
//   carried(doc, shapes)             what copies with a selection
//   pasted(tx, eng, owner, shapes)   relationships to shapes left behind dropped, then settle
//   click_through(doc, owner, top, p, tol)   a click inside a container's inner box reaches what
//                                    is under it (ST-01: the inside is "durchklickbar")
//   track(app, ev) / sync_order(app) what the canvas reported: a running move, the order
//   overlay(app, ui)                 the glow, the insertion bar
//   marks(app) -> Vec<SeMark>        the insert triangles of the lists (ui.shape_edit draws them,
//                                    a click is SeEv::Mark; insert_at handles it)
//   Masters                          the built-in stencils with the structure masters (containers,
//                                    flowchart, callouts) by NameU
//
// Everything runs inside edit_doc::edit, so one gesture with all it moves is one undo step.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::structure::{self, cff, Kind};
use diagram_core::{glue, instance, ops, Document, EditError, MasterRef, Owner, PageRef, ShapeRef, SheetRef, Transaction, Value};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::{self, key_of, shape_of};
use crate::edit_doc;
use crate::Diagrammer;

/// the app's state for structures
#[derive(Default)]
pub(crate) struct Structs {
    /// the keys of the shapes of a running move (empty: none)
    pub dragging: Vec<u64>,
    /// the selection in the order it was made (Shapes verbinden, CN-16)
    pub order: Vec<u64>,
    /// the insert triangles shown: the list and the index of each (SeMark::tag)
    pub marks: Vec<(ShapeRef, usize)>,
    /// new flowcharts are vertical (Funktionsübergreifendes Flussdiagramm › Ausrichtung)
    pub cff_vertical: bool,
    /// the drawing's switch for the Verbindungspfeile (Ansicht; DocumentSheet
    /// User.msvNoAutoConnect); the widget shows them when the option allows them too
    pub arrows: bool,
}

// ---------------------------------------------------------------- the built-in masters

/// the stencils with the structure masters (built into the app, stencils/containers, cff,
/// callouts)
pub(crate) struct Masters<'a> {
    pub containers: Option<&'a Document>,
    pub cff: Option<&'a Document>,
    pub callouts: Option<&'a Document>,
}

/// the structure stencils among the app's stencils
pub(crate) fn masters(st: &crate::stencils::Stencils) -> Masters<'_> {
    let find = |file: &str| {
        st.loaded.iter().find(|l| matches!(l.origin, crate::stencils::Origin::Builtin(k) if crate::stencils::BUILTIN.get(k).is_some_and(|b| b.file == file))).map(|l| &l.doc)
    };
    Masters { containers: find("containers"), cff: find("cff"), callouts: find("callouts") }
}

/// a master of a stencil document by its NameU
pub(crate) fn master_in(doc: &Document, name_u: &str) -> Option<MasterRef> {
    doc.masters().find(|&m| doc.master(m).and_then(|x| x.header.name_u).is_some_and(|n| doc.str(n).eq_ignore_ascii_case(name_u)))
}

/// place master `name_u` of `src` on `page` with its pin at `at` -> the new shape (computed)
pub(crate) fn place_master(tx: &mut Transaction, eng: &mut Engine, src: &Document, name_u: &str, page: PageRef, at: [f64; 2]) -> Result<ShapeRef, EditError> {
    let m = master_in(src, name_u).ok_or(EditError::NoMaster)?;
    let m = instance::import_master(tx, src, m)?;
    let s = instance::place(tx, page, m, at[0], at[1], crate::stencils::lang())?;
    let mut sheets = Vec::new();
    collect(tx.doc(), s, &mut sheets);
    eng.recalc(tx, &sheets)?;
    Ok(s)
}

fn collect(doc: &Document, s: ShapeRef, out: &mut Vec<SheetRef>) {
    out.push(SheetRef::Shape(s));
    if let Some(sh) = doc.shape(s) {
        for c in sh.children() {
            collect(doc, c, out);
        }
    }
}

// ---------------------------------------------------------------- gestures

/// a move of `shapes` by `d` (page): what they carry comes along, connectors follow; when the
/// drag is released (`done`) the shapes join and leave containers by their new place, lists
/// are laid out and containers sized
pub(crate) fn move_shapes(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], d: [f64; 2], done: bool) -> Result<(), EditError> {
    let along = structure::carried(tx.doc(), shapes);
    ops::move_shapes(tx, eng, &along, d)?;
    edit_canvas::follow(tx, eng, owner, &along)?;
    if done {
        // (JD13) equipment snaps onto the units of a rack before it joins it
        crate::rack::docked(tx, eng, owner, shapes)?;
        settle(tx, eng, owner, shapes, true)?;
    }
    Ok(())
}

/// shapes that appeared (drawn, pasted): they join the containers they lie in
pub(crate) fn joined(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    settle(tx, eng, owner, shapes, true)
}

/// a control point of shape `s` moved to page point `p`: the tip of a callout (its row
/// Association) dropped on a shape ties the callout to it there, dropped beside every shape lets
/// go of the target (ST-06: "Lösen/Neu-Anheften über den gelben Steuerpunkt")
pub(crate) fn callout_point(tx: &mut Transaction, eng: &mut Engine, owner: Owner, s: ShapeRef, row: diagram_core::RowKey, p: [f64; 2], done: bool) -> Result<(), EditError> {
    if !done || structure::kind(tx.doc(), s) != Some(Kind::Callout) {
        return Ok(());
    }
    let tip = structure::association(tx.doc(), s);
    if tip.is_none_or(|c| c.row != row) {
        return Ok(());
    }
    let target = target_under(tx.doc(), owner, p, &[s]);
    structure::set_target(tx, eng, s, target)?;
    if target.is_some() {
        structure::set_anchor(tx, s, p)?;
    }
    Ok(())
}

/// after a resize or a turn of `shapes`: lists, flowcharts and sizes (no membership)
pub(crate) fn changed(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    crate::rack::resized(tx, eng, owner, shapes)?;
    settle(tx, eng, owner, shapes, false)
}

fn settle(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], membership: bool) -> Result<(), EditError> {
    let moved = structure::settle(tx, eng, owner, shapes, membership)?;
    // a shape that joined a lane gets the lane's title as its function (ST-05)
    if membership {
        for &s in shapes {
            function_field(tx, eng, s)?;
        }
    }
    if moved.is_empty() {
        return Ok(());
    }
    edit_canvas::follow(tx, eng, owner, &moved)
}

/// the shape data field "Funktion" of a shape in a lane: the lane's title (Visio's formula
/// IFERROR(CONTAINERSHEETREF(1,"Swimlane")!User.visHeadingText,""), added when missing)
fn function_field(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<(), EditError> {
    let (in_lane, has) = {
        let doc = tx.doc();
        let in_lane = structure::containers_of(doc, s).iter().any(|&c| structure::has_category(doc, c, cff::CAT_LANE));
        let has = doc.shape_data(SheetRef::Shape(s)).iter().any(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case("Function")));
        (in_lane, has)
    };
    if !in_lane || has || structure::kind(tx.doc(), s).is_some() {
        return Ok(());
    }
    let sheet = SheetRef::Shape(s);
    let empty = tx.intern("");
    tx.set_shape_data(sheet, "Function", t("Funktion"), Value::Text(empty))?;
    let sec = tx.intern("Property");
    let row = tx.intern("Function");
    let val = tx.intern("Value");
    let a = diagram_core::CellAddr::row(diagram_core::SecKey { name: sec, index: 0 }, diagram_core::RowKey::Named(row), val);
    tx.set_formula(sheet, &a, "IFERROR(CONTAINERSHEETREF(1,\"Swimlane\")!User.visHeadingText,\"\")")?;
    eng.recalc_cells(tx, &[(sheet, a)]).map(|_| ())
}

/// a shape dropped from the Shapes panel at its pin: a callout dropped on a shape takes it as
/// its target and goes up to the right of the point; a lane or phase dropped outside a
/// flowchart brings one; then it joins the containers it lies in
pub(crate) fn dropped(tx: &mut Transaction, eng: &mut Engine, m: &Masters, owner: Owner, s: ShapeRef) -> Result<(), EditError> {
    let Owner::Page(page) = owner else { return Ok(()) };
    let (kind, cats, pin) = {
        let doc = tx.doc();
        let f = ops::frame(doc, s);
        (structure::kind(doc, s), structure::categories(doc, s), f.at([f.w * 0.5, f.h * 0.5]))
    };
    if kind == Some(Kind::Callout) {
        let target = target_under(tx.doc(), owner, pin, &[s]);
        if let Some(t) = target {
            attach_callout(tx, eng, s, t, pin)?;
        }
        return edit_canvas::follow(tx, eng, owner, &[s]);
    }
    let lane = cats.iter().any(|c| c.eq_ignore_ascii_case(cff::CAT_LANE));
    let phase = cats.iter().any(|c| c.eq_ignore_ascii_case(cff::CAT_PHASE));
    if lane || phase {
        let joins = !structure::drop_targets(tx.doc(), owner, s, &[s]).is_empty();
        if !joins {
            if let Some(src) = m.cff {
                crate::structures_cff::around(tx, eng, src, page, s, lane)?;
                return Ok(());
            }
        }
    }
    crate::rack::docked(tx, eng, owner, &[s])?;
    settle(tx, eng, owner, &[s], true)?;
    // (JD21 tools) Beim Ablegen weiterzählen: the page's next number (dlg_number.rs)
    diagram_core::tools::number::dropped(tx, eng, s)?;
    // (JD13) legends count the devices of the page
    crate::legend::refresh(tx, eng, owner)
}

/// the topmost shape under page point p that may be a callout's target (not a container's
/// inside, not a callout, not `skip`)
pub(crate) fn target_under(doc: &Document, owner: Owner, p: [f64; 2], skip: &[ShapeRef]) -> Option<ShapeRef> {
    let tops: Vec<ShapeRef> = doc.top_shapes(owner).collect();
    for &s in tops.iter().rev() {
        if skip.contains(&s) || doc.shape(s).is_none_or(|x| x.is_deleted() || x.header().kind == diagram_core::ShapeKind::Guide) {
            continue;
        }
        if structure::kind(doc, s) == Some(Kind::Callout) || glue::is_one_d(doc, s) {
            continue;
        }
        let b = structure::shape_box(doc, s);
        if p[0] < b[0] || p[0] > b[2] || p[1] < b[1] || p[1] > b[3] {
            continue;
        }
        if structure::is_container(doc, s) {
            let ib = structure::inner_box(doc, s);
            if p[0] > ib[0] && p[0] < ib[2] && p[1] > ib[1] && p[1] < ib[3] {
                continue;
            }
        }
        return Some(s);
    }
    None
}

/// tie callout `c` to target `t` pointing at page point `p`; the callout goes up to the right
/// of it (Visio's place for a new callout)
pub(crate) fn attach_callout(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, t: ShapeRef, p: [f64; 2]) -> Result<(), EditError> {
    structure::set_target(tx, eng, c, Some(t))?;
    structure::set_anchor(tx, c, p)?;
    // up and to the right of the target's box
    let (tb, cb) = {
        let doc = tx.doc();
        (structure::shape_box(doc, t), structure::shape_box(doc, c))
    };
    let gap = 0.25;
    let to = [tb[2] + gap, tb[3] + gap];
    let d = [to[0] - cb[0], to[1] - cb[1]];
    ops::move_shapes(tx, eng, &[c], d)?;
    structure::aim_callouts(tx, eng, &[c])
}

/// delete shapes with what they carry (members, callouts): the survivors let go of them, lists
/// close the gap, glue to them ends
pub(crate) fn delete(o: &mut crate::viewer::Opened, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    if shapes.is_empty() {
        return Ok(());
    }
    let shapes: Vec<ShapeRef> = shapes.to_vec();
    edit_doc::edit(o, t("Löschen"), true, |tx, eng| {
        let all = structure::carried(tx.doc(), &shapes);
        let keep: Vec<ShapeRef> = all.iter().copied().filter(|&s| !ops::locks(eng, tx.doc(), s).has(ops::Locks::DELETE)).collect();
        // the containers that stay: sized again once the members are gone
        let around: Vec<ShapeRef> = {
            let doc = tx.doc();
            let mut v: Vec<ShapeRef> = keep.iter().flat_map(|&s| structure::containers_of(doc, s)).filter(|c| !keep.contains(c)).collect();
            v.sort();
            v.dedup();
            v
        };
        let moved = structure::forget(tx, eng, &keep)?;
        glue::detach(tx, eng, owner, &keep)?;
        tx.delete_shapes(&keep)?;
        let around: Vec<ShapeRef> = around.into_iter().filter(|&c| tx.doc().shape(c).is_some()).collect();
        let mut more = structure::settle(tx, eng, owner, &around, false)?;
        more.extend(moved.into_iter().filter(|&s| tx.doc().shape(s).is_some()));
        if !more.is_empty() {
            edit_canvas::follow(tx, eng, owner, &more)?;
        }
        crate::legend::refresh(tx, eng, owner)
    })
}

/// the shapes a copy of the selection takes: with their members and callouts
pub(crate) fn carried(doc: &Document, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    structure::carried(doc, shapes)
}

/// after a paste: relationships to shapes that stayed behind go, then the copies join where
/// they lie
pub(crate) fn pasted(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    structure::repair(tx, eng, owner, shapes)?;
    // the copies that are in no copied container join by place
    let loose: Vec<ShapeRef> = {
        let doc = tx.doc();
        shapes.iter().copied().filter(|&s| structure::containers_of(doc, s).is_empty()).collect()
    };
    settle(tx, eng, owner, &loose, true)?;
    crate::legend::refresh(tx, eng, owner)
}

/// whether a click at page point `p` on top-level shape `top` goes through it: inside a
/// container's inner box, `tol` away from its edges
pub(crate) fn click_through(doc: &Document, top: ShapeRef, p: [f64; 2], tol: f64) -> bool {
    if !structure::is_container(doc, top) {
        return false;
    }
    let b = structure::inner_box(doc, top);
    p[0] > b[0] + tol && p[0] < b[2] - tol && p[1] > b[1] + tol && p[1] < b[3] - tol
}

// ---------------------------------------------------------------- what the canvas reported

/// remember the shapes of a running move (the glow follows them)
pub(crate) fn track(app: &mut Diagrammer, ev: &SeEv) {
    match ev {
        SeEv::Move { keys, done: false, .. } => app.structs.dragging = keys.clone(),
        SeEv::Move { done: true, .. } | SeEv::Cancel => app.structs.dragging.clear(),
        _ => {}
    }
}

/// the selection's order: keys that left go, new ones are appended (the primary first)
pub(crate) fn sync_order(app: &mut Diagrammer) {
    let sel = &app.se.sel;
    let st = &mut app.structs;
    st.order.retain(|k| sel.contains(k));
    let mut new: Vec<u64> = sel.iter().copied().filter(|k| !st.order.contains(k)).collect();
    if let Some(i) = new.iter().position(|&k| k == app.se.primary) {
        let p = new.remove(i);
        new.insert(0, p);
    }
    st.order.extend(new);
}

/// the selected shapes in the order they were selected
pub(crate) fn ordered(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(page) = app.model.page else { return Vec::new() };
    app.structs.order.iter().filter(|k| app.se.sel.contains(k)).map(|&k| shape_of(page, k)).collect()
}

// ---------------------------------------------------------------- drawing

/// while shapes are dragged: the glow of every container they would join (unless it asks for
/// none), the insertion bar of a list
pub(crate) fn overlay(app: &mut Diagrammer, ui: &mut Ui) {
    if app.structs.dragging.is_empty() || !app.se.busy() {
        app.structs.dragging.clear();
        return;
    }
    let (Some(geo), Some(page), Some(o)) = (app.model.geo, app.model.page, app.opened.as_ref()) else { return };
    let doc = &o.doc;
    let owner = Owner::Page(page);
    let shapes: Vec<ShapeRef> = app.structs.dragging.iter().map(|&k| shape_of(page, k)).collect();
    let moving = structure::carried(doc, &shapes);
    let vp = app.canvas.vp;
    let r = vp.rect;
    let accent = ui.color(T_ACCENT);
    let glow = (accent & 0x00FF_FFFF) | 0x5000_0000;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    let mut done: Vec<ShapeRef> = Vec::new();
    for &s in &shapes {
        if !structure::joins_by_position(doc, s) {
            continue;
        }
        for c in structure::drop_targets(doc, owner, s, &moving) {
            if done.contains(&c) {
                continue;
            }
            done.push(c);
            let quiet = structure::container(doc, c).is_some_and(|i| i.no_highlight);
            let b = structure::shape_box(doc, c);
            let (a, z) = (vp.to_screen(geo.world([b[0], b[3]])), vp.to_screen(geo.world([b[2], b[1]])));
            if !quiet && structure::list_of(doc, s) != Some(c) {
                let pad = ui.dp(3);
                ui.draw_color(glow);
                ui.draw_frame(a[0] as i32 - pad, a[1] as i32 - pad, (z[0] - a[0]) as i32 + 2 * pad, (z[1] - a[1]) as i32 + 2 * pad, ui.dp(3), ui.dp(4));
                ui.draw_color(accent);
                ui.draw_frame(a[0] as i32 - 1, a[1] as i32 - 1, (z[0] - a[0]) as i32 + 2, (z[1] - a[1]) as i32 + 2, 0, ui.dp(2).max(2));
            }
            // a list: the bar where the shape goes
            if structure::kind(doc, c) == Some(Kind::List) && structure::accepts_item(doc, c, s) {
                let sb = structure::shape_box(doc, s);
                let mid = [(sb[0] + sb[2]) * 0.5, (sb[1] + sb[3]) * 0.5];
                let k = structure::list_index_at(doc, c, mid, &[s]);
                let marks = structure::insert_marks_without(doc, c, &[s]);
                if let Some(m) = marks.iter().find(|m| m.index == k) {
                    let (p, q) = (vp.to_screen(geo.world(m.at)), vp.to_screen(geo.world(m.to)));
                    let w = (ui.dp(3).max(3) * 256) as i32;
                    ui.draw_color(accent);
                    ui.draw_line((p[0] * 256.0) as i32, (p[1] * 256.0) as i32, (q[0] * 256.0) as i32, (q[1] * 256.0) as i32, w);
                }
            }
        }
    }
    ui.clip_pop();
}

/// the insert triangles of the lists on the page shown (ui.shape_edit shows one while the
/// pointer is near it): a list's own marks, a flowchart's lanes and phases at their edges
pub(crate) fn marks(app: &mut Diagrammer) -> Vec<SeMark> {
    app.structs.marks.clear();
    let (Some(geo), Some(page), Some(o)) = (app.model.geo, app.model.page, app.opened.as_ref()) else { return Vec::new() };
    let doc = &o.doc;
    let owner = Owner::Page(page);
    let mut out = Vec::new();
    for l in doc.top_shapes(owner) {
        if structure::kind(doc, l) != Some(Kind::List) {
            continue;
        }
        let info = structure::list(doc, l).expect("a list");
        // a triangle at the outer edge of the list, beside each gap (Visio's)
        let ib = structure::inner_box(doc, l);
        for m in structure::insert_marks(doc, l) {
            let (at, dir) = if info.direction.horizontal() { ([m.at[0], ib[3]], 1u8) } else { ([ib[0], m.at[1]], 0u8) };
            // the owner whose hover shows it: the list, else the item next to the gap
            let items = structure::items(doc, l);
            let near = items.get(m.index.min(items.len().saturating_sub(1))).copied().unwrap_or(l);
            let near = if structure::kind(doc, near).is_some() || doc.shape(near).is_some() { near } else { l };
            let tag = app.structs.marks.len() as u32;
            app.structs.marks.push((l, m.index));
            out.push(SeMark { at: geo.world(at), dir, tag, owner: key_of(near) });
        }
    }
    out
}

/// a click on an insert triangle: a new item of the list's item master at that index
pub(crate) fn insert_at(app: &mut Diagrammer, ui: &mut Ui, tag: u32) {
    let Some(&(l, index)) = app.structs.marks.get(tag as usize) else { return };
    let Some(page) = app.model.page else { return };
    let ms = masters(&app.stencils);
    let Some(o) = app.opened.as_mut() else { return };
    let item = structure::user_text(&o.doc, l, "msvSDListItemMaster").map(|t| item_master_name(&t)).unwrap_or_default();
    let src = [ms.cff, ms.containers].into_iter().flatten().find(|d| master_in(d, &item).is_some()).or(ms.containers);
    let Some(src) = src else { return };
    let item = if master_in(src, &item).is_some() { item } else { String::from("List item") };
    let r = edit_doc::edit(o, t("Element einfügen"), true, |tx, eng| {
        let b = structure::shape_box(tx.doc(), l);
        let s = place_master(tx, eng, src, &item, page, [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5])?;
        structure::add_members(tx, eng, l, &[s], Some(index))?;
        let moved = structure::settle(tx, eng, Owner::Page(page), &[l], false)?;
        edit_canvas::follow(tx, eng, Owner::Page(page), &moved)?;
        Ok(s)
    });
    if let Ok(s) = r {
        app.se.select(&[key_of(s)]);
        app.text_req = Some((key_of(s), None, true));
    }
    ui.repass();
}

// "USE(\"Swimlane\")" or "Swimlane" -> Swimlane
fn item_master_name(t: &str) -> String {
    let t = t.trim();
    let inner = t.strip_prefix("USE(").or_else(|| t.strip_prefix("use(")).and_then(|r| r.strip_suffix(')')).unwrap_or(t);
    String::from(inner.trim().trim_matches('"'))
}

/// whether the page shown holds a cross-functional flowchart (its register shows)
pub(crate) fn has_cff(app: &Diagrammer) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return false };
    !cff::pools(&o.doc, Owner::Page(page)).is_empty()
}

/// the container of the selection: a selected container, else the container of a selected
/// shape
pub(crate) fn selected_container(app: &Diagrammer) -> Option<ShapeRef> {
    let o = app.opened.as_ref()?;
    let doc = &o.doc;
    let sel = edit_canvas::selected(app);
    sel.iter().copied().find(|&s| structure::is_container(doc, s) && cff::of_pool(doc, s).is_none()).or_else(|| sel.iter().find_map(|&s| structure::containers_of(doc, s).into_iter().find(|&c| cff::of_pool(doc, c).is_none() && structure::kind(doc, c) != Some(Kind::List) || structure::list_of(doc, s) == Some(c))))
}
