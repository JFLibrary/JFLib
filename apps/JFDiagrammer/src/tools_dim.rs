// tools_dim -- (JD21, AS-11) Ansicht › Add-Ons › Maßlinie horizontal / vertikal / ausgerichtet:
// a dimension between two selected shapes, along one selected shape, or -- nothing selected --
// as a tool that draws one from point to point (diagram_core::tools::dimension)
//
//   run(app, ui, id)          the command: two shapes -> the sides that face each other, one
//                             shape -> its width (height), else the tool stays on until another
//                             is chosen (Start › Zeigertool, Esc)
//   apply(app, ui, &ev)       edit_canvas' hook: a line drawn with the tool (SeEv::Connect of
//                             SE_LINE) becomes a dimension; its ends glue where the edit layer
//                             offers a connection point, a point on a shape ties to that shape
//   tool(app) -> SE_*         the tool's gesture (a line)
//   available(app, id)        a drawing is open
//
// The dimension shapes are the built-in masters of JD15 (stencils/README.md, Textfelder): Bemaßung
// horizontal and vertikal of Gestellmontierte Geräte (a field with ABS(Width) / ABS(Height), the
// dimension line on a control handle), Auto-Bemaßung of Beschriftungen for aligned ones (its
// length, the text upright).  The dimension line goes 8 mm on paper above (left of) what it
// measures.  One undo step each, the new dimension selected.

use diagram_core::formula::Engine;
use diagram_core::tools::dimension::{self, Anchor, Kind};
use diagram_core::{glue, ops, Document, EditError, Owner, PageRef, ShapeRef, Transaction};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of, shape_of};
use crate::{edit_doc, structures, Diagrammer};

/// the clearance of the dimension line from the shapes, on paper (inches)
const CLEAR: f64 = 8.0 / 25.4;

fn kind_of(id: u32) -> Option<Kind> {
    match id {
        DIM_HORIZONTAL => Some(Kind::Horizontal),
        DIM_VERTICAL => Some(Kind::Vertical),
        DIM_ALIGNED => Some(Kind::Aligned),
        _ => None,
    }
}

/// whether `id` is one of the dimension tools
pub(crate) fn is_tool(id: u32) -> bool {
    kind_of(id).is_some()
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    !is_tool(id) && id != DIMENSIONS || app.opened.is_some()
}

// the stencil (file stem) and master of a kind
fn master(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Horizontal => ("rackeq", "Dimension - horizontal"),
        Kind::Vertical => ("rackeq", "Dimension - vertical"),
        Kind::Aligned => ("callouts", "Auto-dimension"),
    }
}

fn stencil<'a>(st: &'a crate::stencils::Stencils, file: &str) -> Option<&'a Document> {
    st.loaded
        .iter()
        .find(|l| matches!(l.origin, crate::stencils::Origin::Builtin(k) if crate::stencils::BUILTIN.get(k).is_some_and(|b| b.file == file)))
        .map(|l| &l.doc)
}

// a dimension of `kind` from `a` to `b` on the page shown -> the new shape (one undo step)
fn make(app: &mut Diagrammer, ui: &mut Ui, kind: Kind, ends: impl FnOnce(&Document) -> (Anchor, Anchor)) {
    let (Some(page), Some(geo)) = (app.model.page, app.model.geo) else { return };
    let (file, nu) = master(kind);
    let clear = CLEAR / geo.scale.max(1e-9);
    let Some(src) = stencil(&app.stencils, file) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Maßlinie"), true, |tx, eng| {
        let (a, b) = ends(tx.doc());
        let (pa, pb) = (dimension::point(tx.doc(), &a), dimension::point(tx.doc(), &b));
        let d = structures::place_master(tx, eng, src, nu, page, [(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5])?;
        put(tx, eng, page, d, kind, &a, &b, clear)?;
        Ok(d)
    });
    if let Ok(d) = r {
        app.se.select(&[key_of(d)]);
    }
    ui.repass();
}

#[allow(clippy::too_many_arguments)]
fn put(tx: &mut Transaction, eng: &mut Engine, page: PageRef, d: ShapeRef, kind: Kind, a: &Anchor, b: &Anchor, clear: f64) -> Result<(), EditError> {
    dimension::place(tx, eng, d, kind, a, b, clear)?;
    edit_canvas::follow(tx, eng, Owner::Page(page), &[d])
}

/// the command: between two selected shapes, along one, else the tool
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some(kind) = kind_of(id) else { return };
    let sel: alloc::vec::Vec<ShapeRef> = match app.opened.as_ref() {
        Some(o) => structures::ordered(app).into_iter().filter(|&s| diagram_core::tools::plain(&o.doc, s)).collect(),
        None => return,
    };
    match sel.as_slice() {
        [s] => {
            let s = *s;
            make(app, ui, kind, |doc| dimension::of_shape(doc, s, kind));
        }
        [a, b] => {
            let (a, b) = (*a, *b);
            make(app, ui, kind, |doc| dimension::between(doc, a, b, kind));
        }
        _ => {
            crate::edit_cmds::tool(app, id);
            app.se.select(&[]);
            ui.repass();
        }
    }
}

// where an end of the drawn line goes: the glue the edit layer offers, a point on a shape ties
// to it, else free
fn anchor(doc: &Document, page: PageRef, p: [f64; 2], g: SeGlue) -> Anchor {
    match g {
        SeGlue::Point { key, tag } if tag & crate::glue_tools::TAG_TARGET == 0 => Anchor::Point { shape: shape_of(page, key), row: tag },
        // (JD21 glue) a vertex, handle or control point offered as a target (no connection
        // point yet): tied to the shape's box there, as on the whole shape
        SeGlue::Point { key, .. } | SeGlue::Shape(key) => {
            let s = shape_of(page, key);
            let f = ops::frame(doc, s);
            match f.local(p) {
                Some(l) if f.w.abs() > 1e-12 && f.h.abs() > 1e-12 => Anchor::Box { shape: s, at: [l[0] / f.w, l[1] / f.h] },
                _ => Anchor::Free(p),
            }
        }
        // (JD21 glue) a guide: free
        SeGlue::None | SeGlue::Guide(_) => Anchor::Free(p),
    }
}

/// edit_canvas' hook: a line drawn with a dimension tool becomes a dimension -> handled
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, ev: &SeEv) -> bool {
    let Some(kind) = kind_of(app.tool) else { return false };
    let SeEv::Connect { a, ga, b, gb, .. } = *ev else { return false };
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return true };
    let (pa, pb) = (geo.page(a), geo.page(b));
    if (pa[0] - pb[0]).abs() < 1e-9 && (pa[1] - pb[1]).abs() < 1e-9 {
        return true;
    }
    make(app, ui, kind, |doc| {
        let (x, y) = (anchor(doc, page, pa, ga), anchor(doc, page, pb, gb));
        // glue::Target::Shape of a 1-D shape (a connector) is no anchor: free there
        let fix = |q: Anchor, p: [f64; 2]| match q.shape() {
            Some(s) if glue::is_one_d(doc, s) => Anchor::Free(p),
            _ => q,
        };
        (fix(x, pa), fix(y, pb))
    });
    true
}
