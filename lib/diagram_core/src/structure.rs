// structure -- containers, lists and callouts (JD10: ST-01, ST-02, ST-04 .. ST-06)
//
//   kind(doc, shape) -> Option<Kind>        Container, List or Callout (User.msvStructureType)
//   container(doc, c) -> Option<Container>  its settings: margin, automatic size, lock, style,
//                                           heading, the categories it takes or refuses
//   list(doc, l) -> Option<List>            a list's direction, spacing and alignment
//   members(doc, c) / containers_of(doc, s) / list_of(doc, s)
//   callouts_of(doc, t) / target_of(doc, callout)
//   categories(doc, s) / has_category(doc, s, name)
//   inner_box(doc, c)                       the box members live in (the heading taken off)
//   drop_targets(doc, owner, shapes)        the containers a shape would join where it is now
//   carried(doc, shapes)                    the shapes plus what moves, copies and deletes with
//                                           them: members (nested too) and callouts
//   add_members / remove_members / set_target     both sides of a relationship
//   settle(tx, eng, owner, shapes, membership)    after a gesture: membership by position,
//                                           lists laid out, containers sized (deepest first)
//   fit(tx, eng, c, shrink)                 a container around its members (An Inhalt anpassen)
//   layout_list(tx, eng, l) / list_index_at / move_in_list / insert_marks
//   forget(tx, eng, deleted)                before deleting: the survivors let go of them
//   disband(tx, eng, c)                     the container goes, its members stay
//   repair(tx, owner)                       relationships named on one side only are dropped
//   aim_callouts(tx, eng, callouts)         a callout's tail back onto its target
//
// The file keeps the structure in the Relationships cell of both shapes (structure::rel) and
// the settings in User cells named as Visio names them (msvStructureType, msvSDContainer*,
// msvSDList*, msvSDHeadingStyle, msvShapeCategories), so Visio recognises what JFDiagrammer
// writes and the other way round.  What a container looks like is the master's (own
// stencils in apps/JFDiagrammer/stencils): the code knows only the cells.
//
// The heading: Visio's own containers are groups whose subshape with msvStructureType
// "Heading" carries msvSDContainerHeadingEdge -- the side where the inside lies, seen from the
// heading ("Bottom" for a heading on top); JFDiagrammer's containers are one shape with the
// heading drawn in it and User.JFD_HeadingEdge (the edge the heading is on: Top, Bottom, Left,
// Right, None) and User.JFD_HeadingSize on the container itself.  Both are read; the inner box
// is the frame without the heading.
//
// Rules (Visio's, as far as they are documented, else chosen and named here):
// * A 2D shape joins every container whose frame holds its whole box after a drop or move and
//   that takes it, nested ones too (Visio stores membership in each enclosing container: a
//   process shape is in its lane, its phase and the lanes' list); a list takes it as an item
//   when the list's box holds its middle and the list's item categories let it, at the index
//   its place gives.  1D shapes, guides, callouts and headings never join by position.
// * It leaves a container when it lies wholly outside it, or when it found new containers and
//   lies not wholly in this one; partly outside it stays and the container grows (automatic
//   size "expand").  A locked container (msvSDContainerLocked) neither takes nor lets go.
// * Required categories: one of them must be among the shape's (msvShapeCategories, ';'
//   separated, case-insensitive; a shape without categories is refused); excluded ones refuse.
// * Only a list's items are laid out and size the list (Visio's); its plain members are not.
// * Moving a container moves its members (nested ones too) and the callouts of everything
//   moved; deleting and copying take them along (carried).
// * Nothing is computed when a file opens: settle runs only after a gesture.

pub mod cff;
pub mod rel;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::ops::{self, Frame};
use crate::sheet::CellAddr;
use crate::units::Unit;

pub use rel::{Relations, REL_CALLOUT, REL_CONTAINER, REL_ITEM, REL_LIST, REL_MEMBER, REL_TARGET};

const EPS: f64 = 1e-6;

/// what a shape is in a structured diagram
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// a container: members inside its frame
    Container,
    /// a list: a container with ordered members side by side
    List,
    /// a callout: a note tied to a target shape
    Callout,
}

/// how a container follows its members (msvSDContainerResize)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Resize {
    /// 0: never by itself
    None,
    /// 1: grows when members need room (Visio's default)
    Expand,
    /// 2: always fits its members, grows and shrinks
    Fit,
}

/// the edge a container's heading is on
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Edge {
    None,
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    /// the edge of a cell's text ("Top", "Bottom", "Left", "Right"; else none)
    pub fn parse(s: &str) -> Edge {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Edge::Top,
            "bottom" => Edge::Bottom,
            "left" => Edge::Left,
            "right" => Edge::Right,
            _ => Edge::None,
        }
    }

    /// the edge across from this one
    pub fn opposite(self) -> Edge {
        match self {
            Edge::None => Edge::None,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
        }
    }

    /// the text Visio writes
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::None => "None",
            Edge::Top => "Top",
            Edge::Bottom => "Bottom",
            Edge::Left => "Left",
            Edge::Right => "Right",
        }
    }
}

/// a container's settings
#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    /// msvSDContainerMargin: between the members and the inner box (drawing units, inches)
    pub margin: f64,
    pub resize: Resize,
    /// msvSDContainerLocked: membership stays as it is
    pub locked: bool,
    /// msvSDContainerNoHighlight: no glow while a shape is dragged over it
    pub no_highlight: bool,
    /// msvSDContainerStyle / msvSDHeadingStyle (1-based) and how many there are
    pub style: u32,
    pub style_count: u32,
    pub heading_style: u32,
    pub heading_style_count: u32,
    /// msvSDContainerRequiredCategories / msvSDContainerExcludedCategories
    pub required: Vec<String>,
    pub excluded: Vec<String>,
    /// where the heading is and how deep it is (drawing units)
    pub heading: Edge,
    pub heading_size: f64,
}

/// a list's order on the page
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Direction {
    /// 0: left to right
    LeftToRight,
    /// 1: right to left
    RightToLeft,
    /// 2: top to bottom
    TopToBottom,
    /// 3: bottom to top
    BottomToTop,
}

impl Direction {
    pub fn from_code(v: i64) -> Direction {
        match v {
            1 => Direction::RightToLeft,
            2 => Direction::TopToBottom,
            3 => Direction::BottomToTop,
            _ => Direction::LeftToRight,
        }
    }

    pub fn code(self) -> i64 {
        match self {
            Direction::LeftToRight => 0,
            Direction::RightToLeft => 1,
            Direction::TopToBottom => 2,
            Direction::BottomToTop => 3,
        }
    }

    /// whether the members stand side by side (else one above the other)
    pub fn horizontal(self) -> bool {
        matches!(self, Direction::LeftToRight | Direction::RightToLeft)
    }
}

/// a list's settings (msvSDListDirection, msvSDListSpacing, msvSDListAlignment)
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct List {
    pub direction: Direction,
    /// between two members (drawing units)
    pub spacing: f64,
    /// across the direction: 0 start (left / top), 1 middle, 2 end (right / bottom)
    pub align: u8,
    /// User.JFD_ListStretch (JFDiagrammer's own, for lanes): every item as long across as the
    /// list's inner box or the longest item
    pub stretch: bool,
}

// ---------------------------------------------------------------- reading cells

fn user_row<'d>(doc: &'d Document, s: ShapeRef, name: &str) -> Option<crate::RowView<'d>> {
    doc.user_cells(SheetRef::Shape(s)).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

/// a User cell's value as text (a number as Visio shows it)
pub fn user_text(doc: &Document, s: ShapeRef, name: &str) -> Option<String> {
    let r = user_row(doc, s, name)?;
    match r.get("Value").value() {
        Value::Text(t) => Some(String::from(doc.str(t))),
        Value::Num(x) => Some(alloc::format!("{x}")),
        Value::Absent => None,
    }
}

/// a User cell's value as a number
pub fn user_num(doc: &Document, s: ShapeRef, name: &str) -> Option<f64> {
    let r = user_row(doc, s, name)?;
    match r.get("Value").value() {
        Value::Num(x) => Some(x),
        Value::Text(t) => {
            let t = doc.str(t).trim();
            if t.eq_ignore_ascii_case("TRUE") {
                Some(1.0)
            } else if t.eq_ignore_ascii_case("FALSE") {
                Some(0.0)
            } else {
                t.parse().ok()
            }
        }
        Value::Absent => None,
    }
}

fn user_flag(doc: &Document, s: ShapeRef, name: &str) -> bool {
    user_num(doc, s, name).is_some_and(|x| x != 0.0)
}

/// what a shape is in a structured diagram (None: an ordinary shape, also a heading)
pub fn kind(doc: &Document, s: ShapeRef) -> Option<Kind> {
    if doc.shape(s).is_none_or(|x| x.is_deleted()) {
        return None;
    }
    let t = user_text(doc, s, "msvStructureType")?;
    match t.trim().to_ascii_lowercase().as_str() {
        "container" => Some(Kind::Container),
        "list" => Some(Kind::List),
        "callout" => Some(Kind::Callout),
        _ => None,
    }
}

/// whether the shape takes members (a container or a list)
pub fn is_container(doc: &Document, s: ShapeRef) -> bool {
    matches!(kind(doc, s), Some(Kind::Container | Kind::List))
}

/// the categories of a shape (msvShapeCategories, ';' separated)
pub fn categories(doc: &Document, s: ShapeRef) -> Vec<String> {
    split_list(&user_text(doc, s, "msvShapeCategories").unwrap_or_default())
}

fn split_list(t: &str) -> Vec<String> {
    t.split(';').map(|p| p.trim()).filter(|p| !p.is_empty() && *p != "0").map(String::from).collect()
}

/// whether the shape has a category (case-insensitive)
pub fn has_category(doc: &Document, s: ShapeRef, name: &str) -> bool {
    categories(doc, s).iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// a container's settings (None: not a container or list)
pub fn container(doc: &Document, c: ShapeRef) -> Option<Container> {
    if !is_container(doc, c) {
        return None;
    }
    let num = |n: &str| user_num(doc, c, n);
    let resize = match num("msvSDContainerResize").map(|v| v as i64) {
        Some(0) => Resize::None,
        Some(2) => Resize::Fit,
        Some(_) => Resize::Expand,
        // a list fits its members; a container grows
        None if kind(doc, c) == Some(Kind::List) => Resize::Fit,
        None => Resize::Expand,
    };
    let (heading, heading_size) = heading(doc, c);
    Some(Container {
        margin: num("msvSDContainerMargin").filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.125),
        resize,
        locked: user_flag(doc, c, "msvSDContainerLocked"),
        no_highlight: user_flag(doc, c, "msvSDContainerNoHighlight"),
        style: num("msvSDContainerStyle").map_or(1, |v| v.max(0.0) as u32),
        style_count: num("msvSDContainerStyleCount").map_or(0, |v| v.max(0.0) as u32),
        heading_style: num("msvSDHeadingStyle").map_or(1, |v| v.max(0.0) as u32),
        heading_style_count: num("msvSDHeadingStyleCount").map_or(0, |v| v.max(0.0) as u32),
        required: split_list(&user_text(doc, c, "msvSDContainerRequiredCategories").unwrap_or_default()),
        excluded: split_list(&user_text(doc, c, "msvSDContainerExcludedCategories").unwrap_or_default()),
        heading,
        heading_size,
    })
}

// the heading: a subshape "Heading" (Visio's containers) or the container's own cells
fn heading(doc: &Document, c: ShapeRef) -> (Edge, f64) {
    if let Some(sh) = doc.shape(c) {
        for k in sh.children() {
            if user_text(doc, k, "msvStructureType").is_some_and(|t| t.eq_ignore_ascii_case("Heading")) {
                // the side of the inside, seen from the heading: the heading is across from it
                let edge = user_text(doc, k, "msvSDContainerHeadingEdge").map_or(Edge::Top, |t| Edge::parse(&t).opposite());
                let b = ops::frame(doc, k).bounds();
                let size = match edge {
                    Edge::Left | Edge::Right => b[2] - b[0],
                    Edge::Top | Edge::Bottom => b[3] - b[1],
                    Edge::None => 0.0,
                };
                return (edge, size.max(0.0));
            }
        }
    }
    let edge = user_text(doc, c, "JFD_HeadingEdge").map_or(Edge::None, |t| Edge::parse(&t));
    let size = if edge == Edge::None { 0.0 } else { user_num(doc, c, "JFD_HeadingSize").filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0) };
    (edge, size)
}

/// a list's settings (None: not a list)
pub fn list(doc: &Document, l: ShapeRef) -> Option<List> {
    if kind(doc, l) != Some(Kind::List) {
        return None;
    }
    Some(List {
        direction: Direction::from_code(user_num(doc, l, "msvSDListDirection").map_or(2, |v| v as i64)),
        spacing: user_num(doc, l, "msvSDListSpacing").filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0),
        align: user_num(doc, l, "msvSDListAlignment").map_or(0, |v| (v as i64).clamp(0, 2) as u8),
        stretch: user_flag(doc, l, "JFD_ListStretch"),
    })
}

// ---------------------------------------------------------------- relationships

/// every member of a container: a list's items in list order first, then its other members
pub fn members(doc: &Document, c: ShapeRef) -> Vec<ShapeRef> {
    let r = rel::relations(doc, c);
    let mut out = r.get(REL_LIST).to_vec();
    for &m in r.get(REL_CONTAINER) {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

/// the items of a list in list order (the members it arranges)
pub fn items(doc: &Document, l: ShapeRef) -> Vec<ShapeRef> {
    rel::relations(doc, l).get(REL_LIST).to_vec()
}

/// the containers and lists a shape is a member or an item of
pub fn containers_of(doc: &Document, s: ShapeRef) -> Vec<ShapeRef> {
    let r = rel::relations(doc, s);
    let mut out = r.get(REL_ITEM).to_vec();
    for &c in r.get(REL_MEMBER) {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// the list a shape is an item of
pub fn list_of(doc: &Document, s: ShapeRef) -> Option<ShapeRef> {
    rel::relations(doc, s).get(REL_ITEM).first().copied()
}

/// the callouts tied to a shape
pub fn callouts_of(doc: &Document, t: ShapeRef) -> Vec<ShapeRef> {
    rel::relations(doc, t).get(REL_TARGET).to_vec()
}

/// the target of a callout
pub fn target_of(doc: &Document, callout: ShapeRef) -> Option<ShapeRef> {
    rel::relations(doc, callout).get(REL_CALLOUT).first().copied()
}

// edit both sides of a relationship: `a` names `b` as `ka`, `b` names `a` as `kb`
fn link(tx: &mut Transaction, a: ShapeRef, ka: u32, b: ShapeRef, kb: u32, at: Option<usize>) -> Result<(), EditError> {
    let mut ra = rel::relations(tx.doc(), a);
    {
        let l = ra.get_mut(ka);
        if !l.contains(&b) {
            match at {
                Some(i) => l.insert(i.min(l.len()), b),
                None => l.push(b),
            }
        }
    }
    rel::write(tx, a, &ra)?;
    let mut rb = rel::relations(tx.doc(), b);
    {
        let l = rb.get_mut(kb);
        if !l.contains(&a) {
            l.push(a);
        }
    }
    rel::write(tx, b, &rb)
}

fn unlink(tx: &mut Transaction, a: ShapeRef, ka: u32, b: ShapeRef, kb: u32) -> Result<(), EditError> {
    let mut ra = rel::relations(tx.doc(), a);
    ra.get_mut(ka).retain(|&x| x != b);
    rel::write(tx, a, &ra)?;
    let mut rb = rel::relations(tx.doc(), b);
    rb.get_mut(kb).retain(|&x| x != a);
    rel::write(tx, b, &rb)
}

/// make shapes members of a container (`at`: the position in a list, else at the end); a
/// shape already in it stays where it is
pub fn add_members(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, shapes: &[ShapeRef], at: Option<usize>) -> Result<(), EditError> {
    join(tx, c, shapes, at)?;
    let mut all = shapes.to_vec();
    all.push(c);
    recalc(tx, eng, &all)
}

// a list takes a shape as an item when its item rules let it (at: the position), else as a
// plain member
fn join(tx: &mut Transaction, c: ShapeRef, shapes: &[ShapeRef], at: Option<usize>) -> Result<(), EditError> {
    if !is_container(tx.doc(), c) {
        return Err(EditError::Invalid);
    }
    let list = kind(tx.doc(), c) == Some(Kind::List);
    let mut k = 0;
    for &s in shapes {
        if s == c || tx.doc().shape(s).is_none() {
            continue;
        }
        if list && accepts_item(tx.doc(), c, s) {
            link(tx, c, REL_LIST, s, REL_ITEM, at.map(|i| i + k))?;
            k += 1;
        } else {
            link(tx, c, REL_CONTAINER, s, REL_MEMBER, None)?;
        }
    }
    Ok(())
}

/// take shapes out of a container
pub fn remove_members(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, shapes: &[ShapeRef]) -> Result<(), EditError> {
    leave(tx, c, shapes)?;
    let mut all = shapes.to_vec();
    all.push(c);
    recalc(tx, eng, &all)
}

fn leave(tx: &mut Transaction, c: ShapeRef, shapes: &[ShapeRef]) -> Result<(), EditError> {
    for &s in shapes {
        unlink(tx, c, REL_CONTAINER, s, REL_MEMBER)?;
        unlink(tx, c, REL_LIST, s, REL_ITEM)?;
    }
    Ok(())
}

/// tie a callout to a target (None: let go of it)
pub fn set_target(tx: &mut Transaction, eng: &mut Engine, callout: ShapeRef, target: Option<ShapeRef>) -> Result<(), EditError> {
    let mut all = alloc::vec![callout];
    if let Some(old) = target_of(tx.doc(), callout) {
        if Some(old) == target {
            return Ok(());
        }
        unlink(tx, callout, REL_CALLOUT, old, REL_TARGET)?;
        all.push(old);
    }
    if let Some(t) = target {
        if t != callout && tx.doc().shape(t).is_some() {
            link(tx, callout, REL_CALLOUT, t, REL_TARGET, None)?;
            all.push(t);
        }
    }
    recalc(tx, eng, &all)
}

/// recompute what reads the Relationships cells of these shapes (the structure functions)
pub fn recalc(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef]) -> Result<(), EditError> {
    let Some(c) = tx.doc().sym(rel::CELL) else { return Ok(()) };
    let cells: Vec<(SheetRef, CellAddr)> = shapes.iter().filter(|&&s| tx.doc().shape(s).is_some()).map(|&s| (SheetRef::Shape(s), CellAddr::single(c))).collect();
    if cells.is_empty() {
        return Ok(());
    }
    eng.recalc_cells(tx, &cells).map(|_| ())
}

// ---------------------------------------------------------------- geometry

/// a shape's upright box on the page
pub fn shape_box(doc: &Document, s: ShapeRef) -> [f64; 4] {
    ops::bounds(doc, &[s]).unwrap_or([0.0; 4])
}

/// the box members live in: the frame without the heading (page, upright)
pub fn inner_box(doc: &Document, c: ShapeRef) -> [f64; 4] {
    let b = shape_box(doc, c);
    let Some(info) = container(doc, c) else { return b };
    cut_heading(b, info.heading, info.heading_size)
}

fn cut_heading(b: [f64; 4], edge: Edge, size: f64) -> [f64; 4] {
    let mut r = b;
    match edge {
        Edge::Top => r[3] = (b[3] - size).max(b[1]),
        Edge::Bottom => r[1] = (b[1] + size).min(b[3]),
        Edge::Left => r[0] = (b[0] + size).min(b[2]),
        Edge::Right => r[2] = (b[2] - size).max(b[0]),
        Edge::None => {}
    }
    r
}

fn add_heading(b: [f64; 4], edge: Edge, size: f64) -> [f64; 4] {
    let mut r = b;
    match edge {
        Edge::Top => r[3] += size,
        Edge::Bottom => r[1] -= size,
        Edge::Left => r[0] -= size,
        Edge::Right => r[2] += size,
        Edge::None => {}
    }
    r
}

fn holds(outer: [f64; 4], inner: [f64; 4]) -> bool {
    inner[0] >= outer[0] - EPS && inner[1] >= outer[1] - EPS && inner[2] <= outer[2] + EPS && inner[3] <= outer[3] + EPS
}

fn meets(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] - EPS && b[0] < a[2] - EPS && a[1] < b[3] - EPS && b[1] < a[3] - EPS
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

// whether a container is upright (its box is its frame): only those are sized by code
fn upright(doc: &Document, c: ShapeRef) -> bool {
    let xf = doc.page_xform(c);
    xf.b.abs() < 1e-9 && xf.c.abs() < 1e-9 && xf.a > 0.0 && xf.d > 0.0
}

// ---------------------------------------------------------------- who joins where

/// whether a shape may join containers by its position: 2D, no guide, no callout, no heading,
/// not a connector
pub fn joins_by_position(doc: &Document, s: ShapeRef) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    if sh.is_deleted() || sh.parent().is_some() || sh.header().kind == crate::ShapeKind::Guide {
        return false;
    }
    if crate::glue::is_one_d(doc, s) || kind(doc, s) == Some(Kind::Callout) {
        return false;
    }
    !user_text(doc, s, "msvStructureType").is_some_and(|t| t.eq_ignore_ascii_case("Heading"))
}

/// whether container `c` takes shape `s` (categories; never itself or one that holds it)
pub fn accepts(doc: &Document, c: ShapeRef, s: ShapeRef) -> bool {
    let Some(info) = container(doc, c) else { return false };
    if c == s || within(doc, c, s) {
        return false;
    }
    rules_take(&categories(doc, s), &info.required, &info.excluded)
}

// Visio's category rule: an excluded category refuses; with required ones, one of them must be
// there (a shape without categories is refused)
fn rules_take(cats: &[String], required: &[String], excluded: &[String]) -> bool {
    let has = |n: &String| cats.iter().any(|x| x.eq_ignore_ascii_case(n));
    if excluded.iter().any(has) {
        return false;
    }
    required.is_empty() || required.iter().any(has)
}

/// whether list `l` takes shape `s` as an item (msvSDListRequiredCategories,
/// msvSDListExcludedCategories); else it may still take it as a plain member
pub fn accepts_item(doc: &Document, l: ShapeRef, s: ShapeRef) -> bool {
    if kind(doc, l) != Some(Kind::List) || l == s || within(doc, l, s) {
        return false;
    }
    let req = split_list(&user_text(doc, l, "msvSDListRequiredCategories").unwrap_or_default());
    let exc = split_list(&user_text(doc, l, "msvSDListExcludedCategories").unwrap_or_default());
    rules_take(&categories(doc, s), &req, &exc)
}

/// whether `inner` is (transitively) a member of `outer`
pub fn within(doc: &Document, inner: ShapeRef, outer: ShapeRef) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = containers_of(doc, inner);
    while let Some(c) = stack.pop() {
        if c == outer {
            return true;
        }
        if seen.insert(c) {
            stack.extend(containers_of(doc, c));
        }
    }
    false
}

/// the containers on `owner` a shape joins where it lies now, leaving out `moving` (what moves
/// with it): every container that holds its whole box and takes it (Visio keeps membership in
/// each enclosing container, nested ones too: a process shape is in its lane, its phase and
/// the lanes' list), and a list whose box holds its middle and that takes it as an item
pub fn drop_targets(doc: &Document, owner: Owner, s: ShapeRef, moving: &[ShapeRef]) -> Vec<ShapeRef> {
    let b = shape_box(doc, s);
    let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let mut out: Vec<ShapeRef> = Vec::new();
    for c in doc.top_shapes(owner) {
        if c == s || moving.contains(&c) || !is_container(doc, c) {
            continue;
        }
        let cb = shape_box(doc, c);
        let item = accepts_item(doc, c, s) && holds(cb, [mid[0], mid[1], mid[0], mid[1]]);
        if item || (accepts(doc, c, s) && holds(cb, b)) {
            out.push(c);
        }
    }
    out
}

/// the shapes plus everything that moves, copies and is deleted with them: members of
/// containers (nested too) and callouts of every shape in it; the shapes come first
pub fn carried(doc: &Document, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    let mut out: Vec<ShapeRef> = Vec::new();
    let mut seen: BTreeSet<ShapeRef> = BTreeSet::new();
    let mut queue: Vec<ShapeRef> = shapes.to_vec();
    let mut i = 0;
    while i < queue.len() {
        let s = queue[i];
        i += 1;
        if !seen.insert(s) || doc.shape(s).is_none() {
            continue;
        }
        out.push(s);
        if is_container(doc, s) {
            queue.extend(members(doc, s));
        }
        queue.extend(callouts_of(doc, s));
    }
    out
}

// how deep a container sits in others: how many containers hold it, transitively
fn depth(doc: &Document, c: ShapeRef) -> usize {
    let mut seen = BTreeSet::new();
    let mut stack = containers_of(doc, c);
    while let Some(p) = stack.pop() {
        if seen.insert(p) && seen.len() < 256 {
            stack.extend(containers_of(doc, p));
        }
    }
    seen.len()
}

// ---------------------------------------------------------------- sizes and lists

/// the box a container needs for its members (a list: its items only, they alone size it):
/// their box, the margin around it, the heading
pub fn needed_box(doc: &Document, c: ShapeRef) -> Option<[f64; 4]> {
    let info = container(doc, c)?;
    let ms = if kind(doc, c) == Some(Kind::List) { items(doc, c) } else { members(doc, c) };
    let b = ops::bounds(doc, &ms)?;
    let m = info.margin;
    Some(add_heading([b[0] - m, b[1] - m, b[2] + m, b[3] + m], info.heading, info.heading_size))
}

/// fit a container around its members: grow only (`shrink` false) or exactly; an empty or
/// turned container stays -> whether it changed
pub fn fit(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, shrink: bool) -> Result<bool, EditError> {
    let (cur, need) = {
        let doc = tx.doc();
        if !upright(doc, c) {
            return Ok(false);
        }
        let Some(need) = needed_box(doc, c) else { return Ok(false) };
        (shape_box(doc, c), need)
    };
    let to = if shrink { need } else { union(cur, need) };
    if (0..4).all(|k| (to[k] - cur[k]).abs() < EPS) {
        return Ok(false);
    }
    ops::set_frame(tx, eng, c, &Frame::from_box(to))?;
    Ok(true)
}

/// the items of a list in list order laid out side by side from the start of its inner box
/// (margin, spacing, alignment across), then the list fitted around them -> the shapes moved
pub fn layout_list(tx: &mut Transaction, eng: &mut Engine, l: ShapeRef) -> Result<Vec<ShapeRef>, EditError> {
    layout_list_with(tx, eng, l, false)
}

// `resized`: the list itself was resized, stretched items take its new width across
fn layout_list_with(tx: &mut Transaction, eng: &mut Engine, l: ShapeRef, resized: bool) -> Result<Vec<ShapeRef>, EditError> {
    let (info, cinfo, ms, inner) = {
        let doc = tx.doc();
        let (Some(info), Some(ci)) = (list(doc, l), container(doc, l)) else { return Ok(Vec::new()) };
        if !upright(doc, l) {
            return Ok(Vec::new());
        }
        (info, ci, items(doc, l), inner_box(doc, l))
    };
    if ms.is_empty() {
        return Ok(Vec::new());
    }
    let m = cinfo.margin;
    let horizontal = info.direction.horizontal();
    // the sizes along and across
    let boxes: Vec<[f64; 4]> = ms.iter().map(|&s| shape_box(tx.doc(), s)).collect();
    let across_max = boxes.iter().map(|b| if horizontal { b[3] - b[1] } else { b[2] - b[0] }).fold(0.0f64, f64::max);
    // stretched members: as long across as the inner box (or the longest member)
    let room = if horizontal { inner[3] - inner[1] - 2.0 * m } else { inner[2] - inner[0] - 2.0 * m };
    let across = match (info.stretch, resized && room > EPS) {
        (true, true) => room,
        (true, false) => across_max.max(room),
        _ => across_max,
    };
    let mut moved = Vec::new();
    // the start: left / right / top / bottom of the inner box
    let mut cursor = match info.direction {
        Direction::LeftToRight => inner[0] + m,
        Direction::RightToLeft => inner[2] - m,
        Direction::TopToBottom => inner[3] - m,
        Direction::BottomToTop => inner[1] + m,
    };
    // across: from the left (vertical lists) or the top (horizontal ones)
    let start_across = if horizontal { inner[3] - m } else { inner[0] + m };
    for (k, &s) in ms.iter().enumerate() {
        let b = boxes[k];
        let (mut w, mut h) = (b[2] - b[0], b[3] - b[1]);
        if info.stretch && upright(tx.doc(), s) {
            if horizontal {
                h = across;
            } else {
                w = across;
            }
        }
        let len = if horizontal { w } else { h };
        let slack = across - if horizontal { h } else { w };
        let off = match info.align {
            1 => slack * 0.5,
            2 => slack,
            _ => 0.0,
        };
        let to = match info.direction {
            Direction::LeftToRight => [cursor, start_across - off - h, cursor + w, start_across - off],
            Direction::RightToLeft => [cursor - w, start_across - off - h, cursor, start_across - off],
            Direction::TopToBottom => [start_across + off, cursor - h, start_across + off + w, cursor],
            Direction::BottomToTop => [start_across + off, cursor, start_across + off + w, cursor + h],
        };
        cursor += match info.direction {
            Direction::LeftToRight | Direction::BottomToTop => len + info.spacing,
            _ => -(len + info.spacing),
        };
        if (0..4).any(|i| (to[i] - b[i]).abs() > EPS) {
            place(tx, eng, s, b, to)?;
            moved.push(s);
        }
    }
    // the list around them
    if fit(tx, eng, l, true)? {
        moved.push(l);
    }
    Ok(moved)
}

// a shape from box `from` to box `to`: its size when it changes (upright shapes), else moved
// with what it carries
fn place(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, from: [f64; 4], to: [f64; 4]) -> Result<(), EditError> {
    let same_size = ((to[2] - to[0]) - (from[2] - from[0])).abs() < EPS && ((to[3] - to[1]) - (from[3] - from[1])).abs() < EPS;
    if same_size || !upright(tx.doc(), s) {
        let d = [to[0] - from[0], to[1] - from[1]];
        let along = carried(tx.doc(), &[s]);
        return ops::move_shapes(tx, eng, &along, d);
    }
    // a resized container keeps its members where they are relative to its start corner
    let d = [to[0] - from[0], to[3] - from[3]];
    if is_container(tx.doc(), s) {
        let along: Vec<ShapeRef> = carried(tx.doc(), &[s]).into_iter().filter(|&x| x != s).collect();
        if d != [0.0, 0.0] && !along.is_empty() {
            ops::move_shapes(tx, eng, &along, d)?;
        }
    }
    ops::set_frame(tx, eng, s, &Frame::from_box(to))
}

/// where a shape at page point `p` goes in a list: the index of the first member whose middle
/// lies beyond p along the list's direction
pub fn list_index_at(doc: &Document, l: ShapeRef, p: [f64; 2], leave_out: &[ShapeRef]) -> usize {
    let Some(info) = list(doc, l) else { return 0 };
    let ms: Vec<ShapeRef> = items(doc, l).into_iter().filter(|s| !leave_out.contains(s)).collect();
    let mut k = 0;
    for &s in &ms {
        let b = shape_box(doc, s);
        let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        let before = match info.direction {
            Direction::LeftToRight => mid[0] < p[0],
            Direction::RightToLeft => mid[0] > p[0],
            Direction::TopToBottom => mid[1] > p[1],
            Direction::BottomToTop => mid[1] < p[1],
        };
        if before {
            k += 1;
        }
    }
    k
}

// the index a shape's place gives it in a list (its middle against the others')
fn index_by_place(doc: &Document, l: ShapeRef, s: ShapeRef) -> usize {
    let b = shape_box(doc, s);
    list_index_at(doc, l, [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5], &[s])
}

/// a mark between two members of a list (the insert triangle, the insertion bar while a
/// shape is dragged in): its page point, the direction the list runs, the index a shape
/// inserted there gets
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Mark {
    pub at: [f64; 2],
    /// the far end of the insertion bar across the list
    pub to: [f64; 2],
    pub direction: Direction,
    pub index: usize,
}

/// the marks of a list: before the first member, between members, after the last
pub fn insert_marks(doc: &Document, l: ShapeRef) -> Vec<Mark> {
    insert_marks_without(doc, l, &[])
}

/// the marks of a list as if `leave_out` (an item being dragged) were not in it
pub fn insert_marks_without(doc: &Document, l: ShapeRef, leave_out: &[ShapeRef]) -> Vec<Mark> {
    let Some(info) = list(doc, l) else { return Vec::new() };
    let ms: Vec<ShapeRef> = items(doc, l).into_iter().filter(|s| !leave_out.contains(s)).collect();
    let inner = inner_box(doc, l);
    let boxes: Vec<[f64; 4]> = ms.iter().map(|&s| shape_box(doc, s)).collect();
    let mut out = Vec::new();
    let n = boxes.len();
    for k in 0..=n {
        // the position along the list between member k-1 and k
        let along = match info.direction {
            Direction::LeftToRight => match (k.checked_sub(1).map(|i| boxes[i]), boxes.get(k)) {
                (Some(a), Some(b)) => (a[2] + b[0]) * 0.5,
                (None, Some(b)) => b[0] - info.spacing * 0.5,
                (Some(a), None) => a[2] + info.spacing * 0.5,
                (None, None) => inner[0],
            },
            Direction::RightToLeft => match (k.checked_sub(1).map(|i| boxes[i]), boxes.get(k)) {
                (Some(a), Some(b)) => (a[0] + b[2]) * 0.5,
                (None, Some(b)) => b[2] + info.spacing * 0.5,
                (Some(a), None) => a[0] - info.spacing * 0.5,
                (None, None) => inner[2],
            },
            Direction::TopToBottom => match (k.checked_sub(1).map(|i| boxes[i]), boxes.get(k)) {
                (Some(a), Some(b)) => (a[1] + b[3]) * 0.5,
                (None, Some(b)) => b[3] + info.spacing * 0.5,
                (Some(a), None) => a[1] - info.spacing * 0.5,
                (None, None) => inner[3],
            },
            Direction::BottomToTop => match (k.checked_sub(1).map(|i| boxes[i]), boxes.get(k)) {
                (Some(a), Some(b)) => (a[3] + b[1]) * 0.5,
                (None, Some(b)) => b[1] - info.spacing * 0.5,
                (Some(a), None) => a[3] + info.spacing * 0.5,
                (None, None) => inner[1],
            },
        };
        let (at, to) = if info.direction.horizontal() { ([along, inner[3]], [along, inner[1]]) } else { ([inner[0], along], [inner[2], along]) };
        out.push(Mark { at, to, direction: info.direction, index: k });
    }
    out
}

/// put a member of a list at another index and lay the list out
pub fn move_in_list(tx: &mut Transaction, eng: &mut Engine, l: ShapeRef, s: ShapeRef, index: usize) -> Result<Vec<ShapeRef>, EditError> {
    let mut r = rel::relations(tx.doc(), l);
    {
        let ms = r.get_mut(REL_LIST);
        let Some(pos) = ms.iter().position(|&x| x == s) else { return Err(EditError::Invalid) };
        ms.remove(pos);
        let at = if index > pos { index - 1 } else { index };
        ms.insert(at.min(ms.len()), s);
    }
    rel::write(tx, l, &r)?;
    recalc(tx, eng, &[l])?;
    layout_list(tx, eng, l)
}

// ---------------------------------------------------------------- after a gesture

/// after `shapes` were dropped, moved, resized or turned (what they carried moved with them):
/// with `membership` the containers they join and leave by their new place (a list: at the
/// index their place gives); then every container concerned, the deepest first: lists laid
/// out, containers sized by their automatic size -> every shape that moved here (for the glue
/// to follow)
pub fn settle(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], membership: bool) -> Result<Vec<ShapeRef>, EditError> {
    let moving = carried(tx.doc(), shapes);
    let mut relchg: Vec<ShapeRef> = Vec::new();
    let mut touched: BTreeSet<ShapeRef> = BTreeSet::new();
    for &s in shapes {
        for c in containers_of(tx.doc(), s) {
            touched.insert(c);
        }
        if is_container(tx.doc(), s) {
            touched.insert(s);
        }
    }
    if membership {
        for &s in shapes {
            let (now, targets) = {
                let doc = tx.doc();
                if !joins_by_position(doc, s) {
                    continue;
                }
                (containers_of(doc, s), drop_targets(doc, owner, s, &moving))
            };
            // it found containers it is not in yet: it leaves the ones it is not wholly in
            let found = targets.iter().any(|t| !now.contains(t));
            let mut stays: Vec<ShapeRef> = Vec::new();
            for &c in &now {
                let keep = {
                    let doc = tx.doc();
                    moving.contains(&c)
                        || targets.contains(&c)
                        || container(doc, c).is_some_and(|i| i.locked)
                        || (!found && meets(shape_box(doc, c), shape_box(doc, s)))
                };
                if keep {
                    stays.push(c);
                } else {
                    leave(tx, c, &[s])?;
                    relchg.push(c);
                    relchg.push(s);
                }
                touched.insert(c);
            }
            for &t in &targets {
                if now.contains(&t) || tx.doc().shape(t).is_none() {
                    continue;
                }
                let (locked, is_list) = {
                    let doc = tx.doc();
                    (container(doc, t).is_some_and(|i| i.locked), kind(doc, t) == Some(Kind::List))
                };
                if locked {
                    continue;
                }
                let at = if is_list { Some(index_by_place(tx.doc(), t, s)) } else { None };
                join(tx, t, &[s], at)?;
                relchg.push(t);
                relchg.push(s);
                touched.insert(t);
            }
            // a list member dropped elsewhere in its list: its new place in the order
            for &l in &stays {
                if moving.contains(&l) || list_of(tx.doc(), s) != Some(l) {
                    continue;
                }
                let (k, pos) = {
                    let doc = tx.doc();
                    (index_by_place(doc, l, s), items(doc, l).iter().position(|&x| x == s))
                };
                if pos != Some(k) {
                    let mut r = rel::relations(tx.doc(), l);
                    let ms = r.get_mut(REL_LIST);
                    ms.retain(|&x| x != s);
                    ms.insert(k.min(ms.len()), s);
                    rel::write(tx, l, &r)?;
                    relchg.push(l);
                }
            }
        }
    }
    recalc(tx, eng, &relchg)?;
    // the containers concerned and the ones around them, the deepest first
    let mut all: BTreeSet<ShapeRef> = BTreeSet::new();
    let mut stack: Vec<ShapeRef> = touched.into_iter().collect();
    while let Some(c) = stack.pop() {
        if tx.doc().shape(c).is_none() || !all.insert(c) {
            continue;
        }
        stack.extend(containers_of(tx.doc(), c));
    }
    let mut order: Vec<(usize, ShapeRef)> = all.into_iter().map(|c| (depth(tx.doc(), c), c)).collect();
    order.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut moved: Vec<ShapeRef> = Vec::new();
    // flowcharts: their pools and lists are laid out together at the end
    let mut pools: Vec<cff::Cff> = Vec::new();
    for (_, c) in order {
        let k = kind(tx.doc(), c);
        let Some(info) = container(tx.doc(), c) else { continue };
        let pool = {
            let doc = tx.doc();
            cff::of_pool(doc, c).or_else(|| if k == Some(Kind::List) { containers_of(doc, c).into_iter().find_map(|p| cff::of_pool(doc, p)) } else { None })
        };
        if let Some(p) = pool {
            if !pools.contains(&p) {
                pools.push(p);
            }
            continue;
        }
        if k == Some(Kind::List) {
            for s in layout_list_with(tx, eng, c, !membership && shapes.contains(&c))? {
                if !moved.contains(&s) {
                    moved.push(s);
                }
            }
        } else if info.resize != Resize::None && fit(tx, eng, c, info.resize == Resize::Fit)? && !moved.contains(&c) {
            moved.push(c);
        }
    }
    for p in pools {
        // a pool or lane resized by hand sets the lanes' length
        let want = if membership {
            None
        } else {
            let doc = tx.doc();
            shapes.iter().find(|&&s| s == p.pool || cff::lanes(doc, &p).contains(&s)).map(|&s| {
                let b = shape_box(doc, s);
                if p.horizontal { b[2] - b[0] } else { b[3] - b[1] }
            })
        };
        for s in cff::layout_with(tx, eng, &p, want)? {
            if !moved.contains(&s) {
                moved.push(s);
            }
        }
    }
    // callouts whose target moved here or whose own place changed: back onto the target
    let mut calls: Vec<ShapeRef> = Vec::new();
    for &s in shapes.iter().chain(moved.iter()) {
        let doc = tx.doc();
        if kind(doc, s) == Some(Kind::Callout) {
            calls.push(s);
        }
        for c in callouts_of(doc, s) {
            if !calls.contains(&c) {
                calls.push(c);
            }
        }
    }
    aim_callouts(tx, eng, &calls)?;
    // what moved with the laid-out shapes
    let doc = tx.doc();
    let mut out = carried(doc, &moved);
    out.retain(|s| doc.shape(*s).is_some());
    Ok(out)
}

/// before `deleted` go: the shapes that stay let go of them; lists close the gap -> the
/// shapes that moved (lists laid out)
pub fn forget(tx: &mut Transaction, eng: &mut Engine, deleted: &[ShapeRef]) -> Result<Vec<ShapeRef>, EditError> {
    let gone: BTreeSet<ShapeRef> = deleted.iter().copied().collect();
    let mut lists: Vec<ShapeRef> = Vec::new();
    for &d in deleted {
        let r = rel::relations(tx.doc(), d);
        for (k, list_) in &r.groups {
            let Some(back) = rel::back(*k) else { continue };
            for &o in list_ {
                if gone.contains(&o) {
                    continue;
                }
                let mut ro = rel::relations(tx.doc(), o);
                ro.get_mut(back).retain(|&x| x != d);
                rel::write(tx, o, &ro)?;
                recalc(tx, eng, &[o])?;
                if *k == REL_ITEM && !lists.contains(&o) {
                    lists.push(o);
                }
            }
        }
    }
    let mut moved = Vec::new();
    for l in lists {
        moved.extend(layout_list(tx, eng, l)?);
    }
    Ok(moved)
}

/// the container goes, its members stay (they leave it; a nested container's members stay in
/// it); the container's own containers lose it
pub fn disband(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef) -> Result<(), EditError> {
    let ms = members(tx.doc(), c);
    remove_members(tx, eng, c, &ms)?;
    forget(tx, eng, &[c])?;
    crate::glue::detach(tx, eng, c.owner, &[c])?;
    tx.delete_shapes(&[c])
}

/// relationships named on one side only (after a paste or a foreign edit) are dropped;
/// `shapes` limits the check (empty: every shape of the owner)
pub fn repair(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<(), EditError> {
    let list: Vec<ShapeRef> = if shapes.is_empty() { rel::related_shapes(tx.doc(), owner) } else { shapes.to_vec() };
    for s in list {
        let r = rel::relations(tx.doc(), s);
        let mut fixed = r.clone();
        for (k, others) in &r.groups {
            let Some(back) = rel::back(*k) else { continue };
            for &o in others {
                if !rel::relations(tx.doc(), o).get(back).contains(&s) {
                    fixed.get_mut(*k).retain(|&x| x != o);
                }
            }
        }
        // the formula as written also names shapes that are gone: write when it differs
        let now = rel::formula(tx.doc(), s).map(String::from);
        if fixed != r || now != rel::formula_of(tx.doc(), &fixed) {
            rel::write(tx, s, &fixed)?;
            recalc(tx, eng, &[s])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- callouts

/// the anchor of a callout on its target: fractions of the target's box (User.JFD_TargetX,
/// User.JFD_TargetY; the middle when missing)
pub fn anchor(doc: &Document, callout: ShapeRef) -> [f64; 2] {
    [user_num(doc, callout, "JFD_TargetX").unwrap_or(0.5), user_num(doc, callout, "JFD_TargetY").unwrap_or(0.5)]
}

/// the page point a callout points at: the anchor on its target
pub fn target_point(doc: &Document, callout: ShapeRef) -> Option<[f64; 2]> {
    let t = target_of(doc, callout)?;
    let f = ops::frame(doc, t);
    let a = anchor(doc, callout);
    Some(f.at([f.w * a[0], f.h * a[1]]))
}

/// remember where on its target a callout points (the page point `p`)
pub fn set_anchor(tx: &mut Transaction, callout: ShapeRef, p: [f64; 2]) -> Result<(), EditError> {
    let Some(t) = target_of(tx.doc(), callout) else { return Ok(()) };
    let f = ops::frame(tx.doc(), t);
    let Some(q) = f.local(p) else { return Ok(()) };
    let fx = if f.w.abs() > EPS { q[0] / f.w } else { 0.5 };
    let fy = if f.h.abs() > EPS { q[1] / f.h } else { 0.5 };
    let sheet = SheetRef::Shape(callout);
    tx.set_user(sheet, "JFD_TargetX", Value::Num(fx), None)?;
    tx.set_user(sheet, "JFD_TargetY", Value::Num(fy), None)
}

/// the control row a callout is tied by: Controls.Association (Visio requires it for callouts),
/// else the first control point
pub fn association(doc: &Document, callout: ShapeRef) -> Option<ops::Control> {
    let cs = ops::controls(doc, callout);
    let i = cs.iter().position(|c| c.name.eq_ignore_ascii_case("Association")).unwrap_or(0);
    cs.into_iter().nth(i)
}

/// the tail of callouts back onto their target: the association point (the tip of the tail)
/// goes to the anchor on the target
pub fn aim_callouts(tx: &mut Transaction, eng: &mut Engine, callouts: &[ShapeRef]) -> Result<(), EditError> {
    for &c in callouts {
        let (row, p) = {
            let doc = tx.doc();
            let Some(p) = target_point(doc, c) else { continue };
            let Some(ctl) = association(doc, c) else { continue };
            if (ctl.at[0] - p[0]).abs() < EPS && (ctl.at[1] - p[1]).abs() < EPS {
                continue;
            }
            (ctl.row, p)
        };
        ops::set_control(tx, eng, c, row, p)?;
    }
    Ok(())
}

/// a length cell of a User row as Visio writes it: inches with a unit to show
pub fn set_user_length(tx: &mut Transaction, s: ShapeRef, name: &str, inches: f64, unit: Unit) -> Result<(), EditError> {
    tx.set_user(SheetRef::Shape(s), name, Value::Num(inches), Some(unit))
}

/// a User cell's address (for the formula engine's dependencies)
pub fn user_addr(doc: &Document, name: &str) -> Option<CellAddr> {
    let sec = crate::sheet::SecKey::one(doc.sym("User")?);
    Some(CellAddr::row(sec, crate::sheet::RowKey::Named(doc.sym(name)?), doc.sym("Value")?))
}
