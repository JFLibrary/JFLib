// cmds_draw -- (JD8 rest) Start › Tools › Zeichentools (RS-16, KB-09): the split button that
// keeps the drawing tool used last, the freeform and the arc tool
//
//   DrawState                    the drawing tool used last (the split button's action; at
//                                first the rectangle)
//   draw_tools(app, ui)          DRAW_TOOLS: the last tool again (FREEFORM and ARC choose
//                                their tool through edit_cmds::tool, commands.rs HANDLERS)
//   any_tool(app)                DRAW_TOOLS is checked while one of its tools is active
//   chosen(app, id)              edit_cmds::tool: a drawing tool becomes the last one
//   split_icon / split_tip       the split button shows the last tool's icon and QuickInfo
//   apply(app, ui, ev)           the edit layer's SeEv::Stroke and SeEv::Arc as one undo step
//                                each (diagram_core::ops::new_freeform / new_arc), the new
//                                shape selected
//
// The freeform keeps the stroke's corners within 2 px of the screen (ops::simplify) as the
// control points of a NURBS curve; a stroke that ends within 8 px of its start is a closed,
// filled shape.  Both tools glue their ends to connection points like the line (CN-12); the
// shapes are 1-D (ops/draw.rs has the cells Visio keeps for them).

use alloc::vec::Vec;

use diagram_core::{glue, ops, Owner, PageRef};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{key_of, shape_of, Geo};
use crate::edit_doc;
use crate::Diagrammer;

/// the drawing tools of the split button's menu, in its order
pub(crate) const TOOLS: [u32; 6] = [RECTANGLE, ELLIPSE, LINE_TOOL, FREEFORM, ARC, PENCIL];

/// the split button's memory
pub(crate) struct DrawState {
    /// the drawing tool chosen last (Visio's Zeichentools button repeats it)
    pub last: u32,
}

impl Default for DrawState {
    fn default() -> DrawState {
        DrawState { last: RECTANGLE }
    }
}

/// Zeichentools (the split button's action): the drawing tool used last
pub(crate) fn draw_tools(app: &mut Diagrammer, _ui: &mut Ui) {
    let last = app.draw.last;
    crate::edit_cmds::tool(app, last);
}

/// the split button is on while one of its tools is
pub(crate) fn any_tool(app: &Diagrammer) -> bool {
    TOOLS.contains(&app.tool)
}

/// a tool was chosen: a drawing tool becomes the split button's
pub(crate) fn chosen(app: &mut Diagrammer, id: u32) {
    if TOOLS.contains(&id) {
        app.draw.last = id;
    }
}

/// the split button's icon: the last tool's
pub(crate) fn split_icon(app: &Diagrammer) -> u32 {
    def(app.draw.last).map_or(0, |d| d.icon)
}

/// the split button's QuickInfo: "Zeichentools: Ellipse (Strg+9)"
pub(crate) fn split_tip(app: &Diagrammer, ui: &Ui) -> alloc::string::String {
    alloc::format!("{}: {}", label(DRAW_TOOLS), tip(ui, app.draw.last))
}

// world units of `px` screen pixels as a length on the page (drawing units)
fn page_len(app: &Diagrammer, geo: &Geo, px: f32) -> f64 {
    let s = app.canvas.vp.scale() as f32;
    let v = geo.page_vec([px / s.max(1e-6), 0.0]);
    v[0].abs()
}

fn target(page: PageRef, g: SeGlue) -> glue::Target {
    match g {
        SeGlue::Point { key, tag } => glue::Target::Point { shape: shape_of(page, key), row: tag },
        // the freeform and the arc glue statically only (like lines)
        _ => glue::Target::None,
    }
}

/// the edit layer's freeform and arc events -> handled
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, ev: &SeEv) -> bool {
    if !matches!(ev, SeEv::Stroke { .. } | SeEv::Arc { .. }) {
        return false;
    }
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return true };
    let owner = Owner::Page(page);
    let tol = page_len(app, &geo, 2.0);
    let close = page_len(app, &geo, 8.0);
    let Some(o) = app.opened.as_mut() else { return true };
    let r = match ev {
        SeEv::Stroke { pts, ga, gb } => {
            let stroke: Vec<[f64; 2]> = pts.iter().map(|p| geo.page(*p)).collect();
            let (ta, tb) = (target(page, *ga), target(page, *gb));
            edit_doc::edit(o, t("Freihandform"), true, |tx, eng| ops::new_freeform(tx, eng, owner, &stroke, tol, close, ta, tb))
        }
        SeEv::Arc { a, ga, b, gb, horizontal } => {
            let (pa, pb) = (geo.page(*a), geo.page(*b));
            let (ta, tb) = (target(page, *ga), target(page, *gb));
            // the page's y runs up, the screen's down: the bow's axis is the same
            edit_doc::edit(o, t("Bogen"), true, |tx, eng| ops::new_arc(tx, eng, owner, (pa, ta), (pb, tb), *horizontal))
        }
        _ => return false,
    };
    if let Ok(s) = r {
        app.se.select(&[key_of(s)]);
    }
    ui.repass();
    true
}
