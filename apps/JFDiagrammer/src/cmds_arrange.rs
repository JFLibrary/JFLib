// cmds_arrange -- (JD8 rest) Start › Anordnen › Ausrichten › Automatisch ausrichten (RS-24): the
// selected shapes that connectors join are lined up so the connectors between them run straight
//
//   auto_align(app, ui)        one undo step
//   available(app)             two or more 2D shapes selected
//
// How (Visio's command lines up connected shapes and leaves the rest; its exact rule is not
// documented, this is ours): the anchor -- the shape selected first -- stays.  From it the
// connections among the selected 2D shapes are walked breadth first; a shape reached over a
// connector from one already placed takes that one's middle across the connector's main direction:
// a neighbour mostly to the side gets its vertical middle, one mostly above or below its
// horizontal middle.  Each shape moves once, so cycles cannot pull it back and forth; selected
// shapes no connector reaches stay where they are.  The connectors follow (edit_canvas::follow).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use diagram_core::glue::{self, Target};
use diagram_core::{ops, Document, Owner, ShapeRef};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::{self, key_of, selected};
use crate::{edit_doc, Diagrammer};

// the 2D shapes of the selection
fn shapes_2d(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    selected(app).into_iter().filter(|&s| !glue::is_one_d(&o.doc, s)).collect()
}

/// two or more 2D shapes are selected
pub(crate) fn available(app: &Diagrammer) -> bool {
    shapes_2d(app).len() > 1
}

// the shape an end is glued to (static or dynamic glue), if any
fn target_shape(t: Target) -> Option<ShapeRef> {
    match t {
        Target::Point { shape, .. } | Target::Shape(shape) => Some(shape),
        Target::None => None,
    }
}

/// the pairs (a, b) of `shapes` that a connector of `owner` joins
pub(crate) fn links(doc: &Document, owner: Owner, shapes: &[ShapeRef]) -> Vec<(ShapeRef, ShapeRef)> {
    let mut out = Vec::new();
    for c in doc.top_shapes(owner) {
        if !glue::is_one_d(doc, c) {
            continue;
        }
        let [a, b] = glue::glued(doc, c);
        if let (Some(a), Some(b)) = (target_shape(a), target_shape(b)) {
            if a != b && shapes.contains(&a) && shapes.contains(&b) && !out.contains(&(a, b)) && !out.contains(&(b, a)) {
                out.push((a, b));
            }
        }
    }
    out
}

/// the moves that line the shapes up: from the anchor along the links, each shape once
pub(crate) fn plan(doc: &Document, anchor: ShapeRef, shapes: &[ShapeRef], links: &[(ShapeRef, ShapeRef)]) -> Vec<(ShapeRef, [f64; 2])> {
    let centre = |s: ShapeRef| ops::frame(doc, s).centre();
    // where each shape's middle goes (the anchor's stays)
    let mut placed: Vec<(ShapeRef, [f64; 2])> = alloc::vec![(anchor, centre(anchor))];
    let mut queue: VecDeque<ShapeRef> = VecDeque::from([anchor]);
    while let Some(a) = queue.pop_front() {
        let Some(&(_, ca)) = placed.iter().find(|p| p.0 == a) else { continue };
        for &(x, y) in links {
            let b = if x == a {
                y
            } else if y == a {
                x
            } else {
                continue;
            };
            if !shapes.contains(&b) || placed.iter().any(|p| p.0 == b) {
                continue;
            }
            let cb = centre(b);
            let (dx, dy) = (cb[0] - ca[0], cb[1] - ca[1]);
            let to = if dx.abs() >= dy.abs() { [cb[0], ca[1]] } else { [ca[0], cb[1]] };
            placed.push((b, to));
            queue.push_back(b);
        }
    }
    placed
        .into_iter()
        .filter(|&(s, _)| s != anchor)
        .map(|(s, to)| {
            let c = centre(s);
            (s, [to[0] - c[0], to[1] - c[1]])
        })
        .filter(|(_, d)| d[0].abs() > 1e-9 || d[1].abs() > 1e-9)
        .collect()
}

/// Automatisch ausrichten
pub(crate) fn auto_align(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = shapes_2d(app);
    let Some(page) = app.model.page else { return };
    if shapes.len() < 2 {
        return;
    }
    let anchor = shapes.iter().copied().find(|&s| key_of(s) == app.se.primary).unwrap_or(shapes[0]);
    let owner = Owner::Page(page);
    let Some(o) = app.opened.as_mut() else { return };
    let moves = {
        let doc = &o.doc;
        let l = links(doc, owner, &shapes);
        plan(doc, anchor, &shapes, &l)
    };
    if moves.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, t("Automatisch ausrichten"), true, |tx, eng| {
        let mut moved = Vec::new();
        for (s, d) in &moves {
            ops::move_shapes(tx, eng, &[*s], *d)?;
            moved.push(*s);
        }
        edit_canvas::follow(tx, eng, owner, &moved)
    });
    ui.repass();
}
