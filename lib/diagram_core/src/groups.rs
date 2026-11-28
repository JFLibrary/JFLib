// groups -- (JD21) how groups behave (SS-17, SP-06, SP-17, RS-27): the cells of the Group
// Properties section, Zu Gruppe hinzufügen / Aus Gruppe entfernen, shapes dropped into a group,
// the warning before a group is taken apart that formulas or a master bind
//
//   props(doc, g) -> Props               SelectMode, DisplayMode, IsDropTarget, IsSnapTarget,
//                                        IsTextEditTarget, DontMoveChildren (Visio's values when
//                                        a cell is missing)
//   set_props(tx, eng, g, &props)        the six cells written (constants, Behavior dialog)
//   is_drop_source(doc, s) / set_drop_source(tx, eng, s, on)   IsDropSource (Miscellaneous)
//   double_click_opens(doc, g) / set_double_click_opens(tx, eng, g, on)   EventDblClick
//                                        OPENGROUPWIN() (Verhalten › Doppelklicken)
//   text_target(doc, s) -> ShapeRef      where typing into a group goes (IsTextEditTarget)
//   drop_target_at(doc, owner, p, skip)  the group a shape dropped at page point p joins
//   dropped(tx, eng, owner, shapes)      shapes that may (IsDropSource) dropped onto a group
//                                        that takes them (IsDropTarget) join it
//   add_to_group(tx, eng, g, shapes)     Zu Gruppe hinzufügen: the shapes into the group, where
//                                        they are on the page, the group's box around them
//   remove_from_group(tx, eng, shapes)   Aus Gruppe entfernen: members out to the group's parent,
//                                        just above the group, where they are on the page
//   ungroup_warning(doc, g) -> Option<Warning>   what ungrouping loses (a master, formulas)
//   guide_glue(tx, eng, owner, shapes)   (SP-17) glue between a guide and shapes breaks unless
//                                        both go into a group
//   reparent(tx, eng, s, to, pos)        a shape into another parent, its place on the page kept
//
// Visio's rules (belegt: learn.microsoft.com, cells of the Group Properties section, IsDropSource,
// "Controlling the Behavior of Groups" of the Visio 2002 developer reference, the Behavior dialog
// box on support.microsoft.com, Selection.AddToGroup / RemoveFromGroup / ConvertToGroup, the
// Visio User's Guide ch. 4 "Working with shapes in groups"):
// * SelectMode 0: a click selects the group only; 1 (default): the group first, a later click a
//   member; 2: the member under the pointer first, a later click the group ("click the bounding
//   box to select the group itself"); nested groups follow the selected group's mode.
// * DisplayMode 0 hides the group's own geometry and text, 1 draws it behind the members, 2 (the
//   default) in front (render.rs since JD4).
// * IsDropTarget (Accept dropped shapes) takes a dropped shape only when the shape's IsDropSource
//   (Add shape to groups on drop) is set too; both default to FALSE.
// * IsSnapTarget (Snap to member shapes, default TRUE): shapes snap and glue to the members;
//   FALSE: only to the group.  IsTextEditTarget (Edit text of group, default TRUE since Visio
//   2000): typing goes into the group's text; FALSE: into the member on top of the stacking order.
// * DontMoveChildren: members cannot be dragged with the mouse (other ways still move them).
// * AddToGroup: the selection holds the shapes and the group, the group is the primary selection
//   or the only group.  RemoveFromGroup: the subselected members leave the group.
// * Guides: "If you add a guide and the shapes glued to it to a group, be sure to select both ...;
//   otherwise the glue breaks" (UG ch. 4).
// * Bitmaps: the old guide says imported bitmaps do not turn with a group; Visio 2010 and later
//   turn pictures (Bild › Drehen, the rotation handle) -- kept: they turn (not built, on purpose).
// * In Gruppe umwandeln (ConvertToGroup) turns a metafile into shapes; the model keeps metafiles
//   as bytes it does not draw (render Note::Metafile), so there is nothing to convert yet.
// Angenommen (the Gegenprobe checks): the alignment box follows after Zu Gruppe hinzufügen and Aus
// Gruppe entfernen (update_alignment_box; not with LockCalcWH); a group left without members and
// without geometry of its own goes; added shapes go on top of the members in their page order and
// leave containers and lists they were in; members of an instance of a master (subshapes that
// inherit from its master) stay; a dropped shape joins when its middle lies in the frame of the
// topmost top-level shape under it and that shape is a group taking drops.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::cell::{Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::{math, Engine};
use crate::geometry::Xform;
use crate::names;
use crate::ops::{self, Put, Writer};
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;

#[cfg(test)]
mod tests;

/// how a click selects in a group (SelectMode)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SelectMode {
    /// 0: only the group
    GroupOnly,
    /// 1: the group first, then a member (Visio's default)
    GroupFirst,
    /// 2: the member under the pointer first, then the group
    MembersFirst,
}

impl SelectMode {
    /// the cell's value
    pub fn code(self) -> u8 {
        match self {
            SelectMode::GroupOnly => 0,
            SelectMode::GroupFirst => 1,
            SelectMode::MembersFirst => 2,
        }
    }

    /// the mode of a cell's value (unknown: group first)
    pub fn of(v: i64) -> SelectMode {
        match v {
            0 => SelectMode::GroupOnly,
            2 => SelectMode::MembersFirst,
            _ => SelectMode::GroupFirst,
        }
    }
}

/// the cells of a group's Group Properties section
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Props {
    pub select_mode: SelectMode,
    /// DisplayMode: 0 the group's own geometry and text hidden, 1 behind the members, 2 in front
    pub display_mode: u8,
    /// IsDropTarget: takes shapes dropped onto it (with IsDropSource)
    pub drop_target: bool,
    /// IsSnapTarget: shapes snap and glue to the members
    pub snap_target: bool,
    /// IsTextEditTarget: typing goes into the group's own text
    pub text_edit_target: bool,
    /// DontMoveChildren: members cannot be dragged
    pub dont_move_children: bool,
}

impl Default for Props {
    /// Visio's values for a new group
    fn default() -> Props {
        Props { select_mode: SelectMode::GroupFirst, display_mode: 2, drop_target: false, snap_target: true, text_edit_target: true, dont_move_children: false }
    }
}

// a cell's number with inheritance
fn num(doc: &Document, s: ShapeRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(SheetRef::Shape(s), &CellAddr::single(c)).num())
}

fn flag(doc: &Document, s: ShapeRef, cell: &str, default: bool) -> bool {
    num(doc, s, cell).map_or(default, |v| v != 0.0)
}

/// whether the shape is a group
pub fn is_group(doc: &Document, s: ShapeRef) -> bool {
    doc.shape(s).is_some_and(|x| x.header().kind == ShapeKind::Group && !x.is_deleted())
}

/// the Group Properties of a shape (Visio's values for missing cells)
pub fn props(doc: &Document, g: ShapeRef) -> Props {
    let d = Props::default();
    Props {
        select_mode: num(doc, g, "SelectMode").map_or(d.select_mode, |v| SelectMode::of(v as i64)),
        display_mode: num(doc, g, "DisplayMode").map_or(d.display_mode, |v| (v as i64).clamp(0, 2) as u8),
        drop_target: flag(doc, g, "IsDropTarget", d.drop_target),
        snap_target: flag(doc, g, "IsSnapTarget", d.snap_target),
        text_edit_target: flag(doc, g, "IsTextEditTarget", d.text_edit_target),
        dont_move_children: flag(doc, g, "DontMoveChildren", d.dont_move_children),
    }
}

// write a constant into a single cell (a guarded cell stays)
fn put_num(tx: &mut Transaction, s: ShapeRef, cell: &str, v: f64, unit: Option<crate::units::Unit>) -> Result<(), EditError> {
    let a = tx.single(cell);
    match tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v), unit) {
        Ok(()) | Err(EditError::Guarded) => Ok(()),
        Err(e) => Err(e),
    }
}

fn bool_v(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// write a group's Group Properties (only the cells that change; booleans as TRUE/FALSE values)
pub fn set_props(tx: &mut Transaction, eng: &mut Engine, g: ShapeRef, p: &Props) -> Result<(), EditError> {
    let now = props(tx.doc(), g);
    let b = Some(crate::units::Unit::Bool);
    let mut cells = Vec::new();
    if now.select_mode != p.select_mode {
        put_num(tx, g, "SelectMode", p.select_mode.code() as f64, None)?;
        cells.push("SelectMode");
    }
    if now.display_mode != p.display_mode {
        put_num(tx, g, "DisplayMode", p.display_mode as f64, None)?;
        cells.push("DisplayMode");
    }
    for (name, old, new) in [
        ("IsDropTarget", now.drop_target, p.drop_target),
        ("IsSnapTarget", now.snap_target, p.snap_target),
        ("IsTextEditTarget", now.text_edit_target, p.text_edit_target),
        ("DontMoveChildren", now.dont_move_children, p.dont_move_children),
    ] {
        if old != new {
            put_num(tx, g, name, bool_v(new), b)?;
            cells.push(name);
        }
    }
    recalc_named(tx, eng, g, &cells)
}

fn recalc_named(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, cells: &[&str]) -> Result<(), EditError> {
    let list: Vec<(SheetRef, CellAddr)> = cells.iter().map(|c| (SheetRef::Shape(s), tx.single(c))).collect();
    if !list.is_empty() {
        eng.recalc_cells(tx, &list)?;
    }
    Ok(())
}

/// IsDropSource: the shape joins a group it is dropped onto (when the group takes drops)
pub fn is_drop_source(doc: &Document, s: ShapeRef) -> bool {
    flag(doc, s, "IsDropSource", false)
}

/// set IsDropSource of a shape
pub fn set_drop_source(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, on: bool) -> Result<(), EditError> {
    if is_drop_source(tx.doc(), s) == on {
        return Ok(());
    }
    put_num(tx, s, "IsDropSource", bool_v(on), Some(crate::units::Unit::Bool))?;
    recalc_named(tx, eng, s, &["IsDropSource"])
}

/// whether a double click opens the group (its EventDblClick calls OPENGROUPWIN)
pub fn double_click_opens(doc: &Document, g: ShapeRef) -> bool {
    let Some(c) = doc.sym("EventDblClick") else { return false };
    match doc.resolve(SheetRef::Shape(g), &CellAddr::single(c)).formula() {
        Formula::Expr(f) => doc.str(f).to_ascii_uppercase().contains("OPENGROUPWIN("),
        _ => false,
    }
}

/// a double click opens the group (EventDblClick = OPENGROUPWIN()) or does what it did before
/// (off: the cell inherits again, else 0 -- Visio's "Bearbeitet Text des Shapes")
pub fn set_double_click_opens(tx: &mut Transaction, eng: &mut Engine, g: ShapeRef, on: bool) -> Result<(), EditError> {
    if double_click_opens(tx.doc(), g) == on {
        return Ok(());
    }
    let a = tx.single("EventDblClick");
    let sheet = SheetRef::Shape(g);
    if on {
        tx.set_formula(sheet, &a, "OPENGROUPWIN()")?;
    } else {
        tx.inherit_cell(sheet, &a)?;
        if double_click_opens(tx.doc(), g) {
            tx.set_constant(sheet, &a, Value::Num(0.0), None)?;
        }
    }
    eng.recalc_cells(tx, &[(sheet, a)]).map(|_| ())
}

/// where typing into shape `s` goes: a group whose IsTextEditTarget is off hands it to its member
/// on top of the stacking order (and so on down)
pub fn text_target(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut cur = s;
    for _ in 0..64 {
        if !is_group(doc, cur) || props(doc, cur).text_edit_target {
            break;
        }
        match doc.shape(cur).and_then(|x| x.children().filter(|&k| doc.shape(k).is_some_and(|y| !y.is_deleted())).last()) {
            Some(k) => cur = k,
            None => break,
        }
    }
    cur
}

// whether page point p lies in the frame of shape s
fn in_frame(doc: &Document, s: ShapeRef, p: [f64; 2]) -> bool {
    let f = ops::frame(doc, s);
    match f.local(p) {
        Some(q) => q[0] >= -1e-9 && q[1] >= -1e-9 && q[0] <= f.w + 1e-9 && q[1] <= f.h + 1e-9,
        None => false,
    }
}

/// the group a shape dropped at page point `p` joins: the topmost top-level shape of `owner`
/// (not in `skip`, no guide, no 1-D shape) whose frame holds p, when it is a group that takes
/// drops (IsDropTarget)
pub fn drop_target_at(doc: &Document, owner: Owner, p: [f64; 2], skip: &[ShapeRef]) -> Option<ShapeRef> {
    let tops: Vec<ShapeRef> = doc.top_shapes(owner).collect();
    for &s in tops.iter().rev() {
        if skip.contains(&s) || doc.shape(s).is_none_or(|x| x.is_deleted() || x.header().kind == ShapeKind::Guide) || ops::one_d(doc, s) {
            continue;
        }
        if !in_frame(doc, s, p) {
            continue;
        }
        return (is_group(doc, s) && props(doc, s).drop_target).then_some(s);
    }
    None
}

/// shapes dropped or moved: each top-level one with IsDropSource whose middle lies on a group
/// that takes drops joins that group (add_to_group) -> the groups that took shapes
pub fn dropped(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<Vec<ShapeRef>, EditError> {
    let mut by_group: Vec<(ShapeRef, Vec<ShapeRef>)> = Vec::new();
    {
        let doc = tx.doc();
        let moving = crate::structure::carried(doc, shapes);
        for &s in shapes {
            if doc.shape(s).is_none_or(|x| x.parent().is_some()) || !is_drop_source(doc, s) {
                continue;
            }
            let Some(b) = ops::bounds(doc, &[s]) else { continue };
            let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
            let Some(g) = drop_target_at(doc, owner, mid, &moving) else { continue };
            match by_group.iter_mut().find(|x| x.0 == g) {
                Some(x) => x.1.push(s),
                None => by_group.push((g, alloc::vec![s])),
            }
        }
    }
    let mut out = Vec::new();
    for (g, list) in by_group {
        if !add_to_group(tx, eng, g, &list)?.is_empty() {
            out.push(g);
        }
    }
    Ok(out)
}

// whether `inner` lies (transitively) inside `outer`
fn inside(doc: &Document, inner: ShapeRef, outer: ShapeRef) -> bool {
    let mut cur = doc.shape(inner).and_then(|s| s.parent());
    while let Some(p) = cur {
        if p == outer {
            return true;
        }
        cur = doc.shape(p).and_then(|s| s.parent());
    }
    false
}

// a subshape of an instance that inherits from its master (it cannot leave its group)
fn inherits(doc: &Document, s: ShapeRef) -> bool {
    doc.shape(s).is_some_and(|x| x.header().master_shape.is_some() && x.header().master.is_none())
}

/// put shape `s` into parent `to` (None: the page) at `pos` in its z-order (None: on top),
/// where it is on the page: its pin, angle and flips (1-D: its ends) in the new parent's
/// coordinates; formulas of it and its subshapes that name its old group become their values
pub fn reparent(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, to: Option<ShapeRef>, pos: Option<usize>) -> Result<(), EditError> {
    let (old_parent, q, one_d) = {
        let doc = tx.doc();
        let sh = doc.shape(s).ok_or(EditError::NoShape)?;
        let old = sh.parent();
        let to_xf = to.map_or(Xform::IDENTITY, |g| doc.page_xform(g));
        let q = ops::parent_xf(doc, s).then(&to_xf.inverse().ok_or(EditError::Invalid)?);
        (old, q, ops::one_d(doc, s))
    };
    let sheet = SheetRef::Shape(s);
    let mirrored = q.det() < 0.0;
    let mut sets: Vec<(CellAddr, Put)> = Vec::new();
    let mut results: Vec<(CellAddr, f64)> = Vec::new();
    {
        let doc = tx.doc();
        if one_d {
            let (gb, ge) = ops::glued_ends(doc, s);
            let (bp, ep) = ops::ends(doc, s);
            for (glued, cx, cy, p) in [(gb, names::BEGIN_X, names::BEGIN_Y, bp), (ge, names::END_X, names::END_Y, ep)] {
                let n = q.apply(p);
                if glued {
                    results.push((ops::single(cx), n[0]));
                    results.push((ops::single(cy), n[1]));
                } else {
                    sets.push((ops::single(cx), Put::Num(n[0])));
                    sets.push((ops::single(cy), Put::Num(n[1])));
                }
            }
            if mirrored {
                let fy = ops::val(doc, sheet, names::FLIP_Y) != 0.0;
                sets.push((ops::single(names::FLIP_Y), Put::Num(bool_v(!fy))));
            }
        } else {
            let p = q.apply([ops::val(doc, sheet, names::PIN_X), ops::val(doc, sheet, names::PIN_Y)]);
            let a = ops::val(doc, sheet, names::ANGLE);
            let qa = math::atan2(q.b, q.a);
            let na = if mirrored { qa - a } else { qa + a };
            sets.push((ops::single(names::PIN_X), Put::Num(p[0])));
            sets.push((ops::single(names::PIN_Y), Put::Num(p[1])));
            if (ops::norm_angle(na) - ops::norm_angle(a)).abs() > 1e-12 {
                sets.push((ops::single(names::ANGLE), Put::Num(ops::norm_angle(na))));
            }
            if mirrored {
                let fy = ops::val(doc, sheet, names::FLIP_Y) != 0.0;
                sets.push((ops::single(names::FLIP_Y), Put::Num(bool_v(!fy))));
            }
        }
    }
    // formulas that name the old group: their values (the shape and everything inside it)
    let mut touched: Vec<SheetRef> = Vec::new();
    if let Some(og) = old_parent {
        let gid = tx.doc().shape(og).map_or(0, |x| x.id());
        let mut stack = alloc::vec![s];
        while let Some(x) = stack.pop() {
            let sh = SheetRef::Shape(x);
            for a in ops::cells_naming(tx.doc(), sh, gid) {
                ops::freeze(tx, sh, &a)?;
            }
            touched.push(sh);
            if let Some(n) = tx.doc().shape(x) {
                stack.extend(n.children());
            }
        }
    }
    tx.move_shape(s, to, pos)?;
    let mut w = Writer::new();
    for (a, v) in results {
        tx.set_result(sheet, &a, Value::Num(v), None)?;
    }
    w.put_all(tx, eng, sheet, sets)?;
    if !touched.is_empty() {
        eng.recalc(tx, &touched)?;
    }
    w.finish(tx, eng)
}

/// Zu Gruppe hinzufügen: `shapes` (not the group, nothing it holds or that holds it, no
/// inheriting subshape of an instance) go into group `g` on top of its members, where they are on
/// the page; they get the formulas of members (PinX "Sheet.<g>!Width*k" ...), leave the
/// containers and lists they were in, and the group's box fits them all (update_alignment_box).
/// Glue between a guide and shapes breaks unless both are in the group -> the shapes added
pub fn add_to_group(tx: &mut Transaction, eng: &mut Engine, g: ShapeRef, shapes: &[ShapeRef]) -> Result<Vec<ShapeRef>, EditError> {
    let list: Vec<ShapeRef> = {
        let doc = tx.doc();
        if !is_group(doc, g) {
            return Err(EditError::Invalid);
        }
        ops::outermost(doc, shapes)
            .into_iter()
            .filter(|&s| s != g && s.owner == g.owner && !inside(doc, g, s) && doc.shape(s).is_some_and(|x| x.parent() != Some(g)) && !inherits(doc, s))
            .collect()
    };
    if list.is_empty() {
        return Ok(list);
    }
    // in page z-order
    let order: Vec<ShapeRef> = tx.doc().all_shapes(g.owner).filter(|s| list.contains(s)).collect();
    // the glue of guides: what ends up in the group with the guide keeps it
    let members: Vec<ShapeRef> = tx.doc().shape(g).map(|x| x.children().collect::<Vec<_>>()).unwrap_or_default();
    guide_glue(tx, eng, g.owner, &order, &members)?;
    // out of containers and lists (members of a group are no container members)
    let rel: Vec<ShapeRef> = order.iter().copied().filter(|&s| !crate::structure::containers_of(tx.doc(), s).is_empty()).collect();
    for &s in &rel {
        for c in crate::structure::containers_of(tx.doc(), s) {
            crate::structure::remove_members(tx, eng, c, &[s])?;
        }
    }
    for &s in &order {
        reparent(tx, eng, s, Some(g), None)?;
    }
    // the members' formulas in the group's box
    let (gw, gh, gid, grm) = {
        let doc = tx.doc();
        let gs = SheetRef::Shape(g);
        (ops::val(doc, gs, names::WIDTH), ops::val(doc, gs, names::HEIGHT), doc.shape(g).map_or(0, |x| x.id()), ops::val(doc, gs, names::RESIZE_MODE))
    };
    let mut w = Writer::new();
    for &s in &order {
        let (sets, results) = ops::member_sets(tx.doc(), s, [0.0, 0.0], gw, gh, gid, grm);
        let sheet = SheetRef::Shape(s);
        for (a, v) in results {
            tx.set_result(sheet, &a, Value::Num(v), None)?;
        }
        w.put_all(tx, eng, sheet, sets)?;
    }
    w.finish(tx, eng)?;
    crate::shapeops::update_alignment_box(tx, eng, g)?;
    Ok(order)
}

/// Aus Gruppe entfernen: members (not an inheriting subshape of an instance) go to their group's
/// parent just above the group, where they are on the page; the group's box fits the members that
/// stay, a group left without members and without geometry of its own goes -> the shapes moved
pub fn remove_from_group(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef]) -> Result<Vec<ShapeRef>, EditError> {
    let list: Vec<(ShapeRef, ShapeRef)> = {
        let doc = tx.doc();
        let mut v: Vec<(ShapeRef, ShapeRef)> = Vec::new();
        for s in ops::outermost(doc, shapes) {
            let Some(g) = doc.shape(s).and_then(|x| x.parent()) else { continue };
            if inherits(doc, s) {
                continue;
            }
            v.push((s, g));
        }
        v
    };
    let mut moved = Vec::new();
    // the groups left and how many of their members went above them so far
    let mut groups: Vec<(ShapeRef, usize)> = Vec::new();
    for (s, g) in list {
        let (parent, pos) = {
            let doc = tx.doc();
            let parent = doc.shape(g).and_then(|x| x.parent());
            let sib: Vec<ShapeRef> = match parent {
                Some(p) => doc.shape(p).map(|x| x.children().collect()).unwrap_or_default(),
                None => doc.top_shapes(g.owner).collect(),
            };
            (parent, sib.iter().position(|&x| x == g).map_or(sib.len(), |i| i + 1))
        };
        let k = match groups.iter_mut().find(|x| x.0 == g) {
            Some(x) => {
                x.1 += 1;
                x.1 - 1
            }
            None => {
                groups.push((g, 1));
                0
            }
        };
        // the glue between the leaving shape and a guide that stays (or the other way round)
        let stay: Vec<ShapeRef> = tx.doc().shape(g).map(|x| x.children().filter(|&c| c != s).collect()).unwrap_or_default();
        break_guide_glue(tx, eng, g.owner, &[s], &stay)?;
        reparent(tx, eng, s, parent, Some(pos + k))?;
        moved.push(s);
    }
    let groups: Vec<ShapeRef> = groups.into_iter().map(|x| x.0).collect();
    for g in groups {
        let (empty, geo) = {
            let doc = tx.doc();
            let kids = doc.shape(g).map_or(0, |x| x.children().filter(|&k| doc.shape(k).is_some_and(|y| !y.is_deleted())).count());
            (kids == 0, crate::shapeops::geometry_bounds(doc, g).is_some())
        };
        if empty && !geo {
            crate::glue::detach(tx, eng, g.owner, &[g])?;
            crate::structure::forget(tx, eng, &[g])?;
            tx.delete_shapes(&[g])?;
        } else if !empty {
            crate::shapeops::update_alignment_box(tx, eng, g)?;
        }
    }
    Ok(moved)
}

/// what ungrouping a group loses (the app asks first)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Warning {
    /// the group is an instance of a master: its link to the master goes (Visio's warning)
    Master,
    /// this many cells of members name the group in formulas that are not their place or size
    /// (Sheet.N!Prop.X, Sheet.N!User.Y ...): they keep their values only
    Formulas(usize),
}

/// what ungrouping `g` would lose (None: nothing)
pub fn ungroup_warning(doc: &Document, g: ShapeRef) -> Option<Warning> {
    let sh = doc.shape(g)?;
    if sh.header().kind != ShapeKind::Group {
        return None;
    }
    if sh.header().master.is_some() {
        return Some(Warning::Master);
    }
    let gid = sh.id();
    let mut n = 0;
    let mut stack: Vec<ShapeRef> = sh.children().collect();
    while let Some(s) = stack.pop() {
        let sheet = SheetRef::Shape(s);
        for a in ops::cells_naming(doc, sheet, gid) {
            let r = doc.resolve(sheet, &a);
            let plain = match r.formula() {
                Formula::Expr(f) => ops::text::proportional(doc.str(f)).is_some(),
                _ => true,
            };
            if !plain {
                n += 1;
            }
        }
        if let Some(x) = doc.shape(s) {
            stack.extend(x.children());
        }
    }
    (n > 0).then_some(Warning::Formulas(n))
}

/// (SP-17) the glue between a guide and a shape breaks when one of them is among `moving` (what
/// goes into a group or out of it) and the other is not among `together` (what ends up beside
/// it): the glued cells keep their values, the Connect entry goes
pub fn guide_glue(tx: &mut Transaction, eng: &mut Engine, owner: Owner, moving: &[ShapeRef], together: &[ShapeRef]) -> Result<(), EditError> {
    let rest: Vec<ShapeRef> = {
        let doc = tx.doc();
        let all: BTreeSet<ShapeRef> = together.iter().chain(moving.iter()).copied().collect();
        doc.all_shapes(owner).filter(|s| !all.contains(s)).collect()
    };
    break_guide_glue(tx, eng, owner, moving, &rest)
}

// the glue between a guide and a shape, one of them among `a`, the other among `b`, breaks
fn break_guide_glue(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: &[ShapeRef], b: &[ShapeRef]) -> Result<(), EditError> {
    let breaks: Vec<usize> = {
        let doc = tx.doc();
        let sa: BTreeSet<ShapeRef> = a.iter().copied().collect();
        let sb: BTreeSet<ShapeRef> = b.iter().copied().collect();
        ops::connects(doc, owner)
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let (Some(from), Some(to)) = (doc.shape_by_id(owner, c.from_sheet), doc.shape_by_id(owner, c.to_sheet)) else { return false };
                let guide = |s: ShapeRef| doc.shape(s).is_some_and(|x| x.header().kind == ShapeKind::Guide);
                (guide(to) || guide(from)) && ((sa.contains(&from) && sb.contains(&to)) || (sa.contains(&to) && sb.contains(&from)))
            })
            .map(|(i, _)| i)
            .collect()
    };
    let mut cells: Vec<(SheetRef, CellAddr)> = Vec::new();
    for i in breaks.into_iter().rev() {
        let c = ops::connects(tx.doc(), owner)[i].clone();
        tx.remove_connect(owner, i)?;
        let (Some(f), Some(cell)) = (tx.doc().shape_by_id(owner, c.from_sheet), c.from_cell) else { continue };
        let sheet = SheetRef::Shape(f);
        for a in ops::glue_cells(tx.doc(), cell) {
            ops::freeze(tx, sheet, &a)?;
            cells.push((sheet, a));
        }
    }
    if !cells.is_empty() {
        eng.recalc_cells(tx, &cells)?;
    }
    Ok(())
}
