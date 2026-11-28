// glue_tools -- (JD21 glue) control points that glue (SP-04, SS-30: Ethernet, ring network, bus,
// branches), glue to vertices, handles, control points and guides (CN-02), the connection point
// tool (CN-01, RS-17), several vertices together and the handles of arcs (SP-05); the rules and
// the cells are diagram_core::pointglue's and ops::arcs'
//
//   GlueState                 Ausrichten und Kleben › Kleben an: Shape-Ziehpunkte (handles and
//                             control points), Shape-Scheitelpunkte (vertices); off, as Visio's
//                             defaults (Führungslinien: app.se.glue_guides, on)
//   targets(st, doc, r, geo, out)   edit_canvas::model: the glue targets of a top-level shape
//                             beyond its connection points (SP_TARGET, tags below)
//   arc_points(doc, r, geo, chosen, out)   edit_canvas::model, vertex editing of the single
//                             selection: SP_ARC per arc row, SP_ECC for the chosen elliptical arc
//   end_flags(doc, r)          edit_canvas::model: an end on a guide shows glued (SEF_*_GLUED)
//   resolve / glue_end / connect_targets / connect_guides   an end's glue as edit_canvas::apply
//                             writes it: anchors made into connection points, guides
//   apply(app, ui, ev)         the edit layer's events this module takes: a glued control point,
//                             connection points (SE_CPOINT), arcs, several vertices -> taken
//   control_tip(app, ui)       the Tip (Prompt) of the control point under the pointer (QuickInfo)
//   point_menu(app)            the context menu is the chosen connection point's (panes.rs)
//   set_kind / kind_checked / available    CPOINT_INWARD, CPOINT_OUTWARD, CPOINT_BOTH
//   tool(app)                  CONN_POINT_TOOL
//
// Tags of the edit layer's points (SePoint::tag): a vertex section << 16 | row (edit_canvas);
// TAG_TARGET | kind | ...: a target (vertex: section << 16 | row; handle: its index 0..7 in
// ui::shape_edit's order; control point: its position in the Controls section); TAG_ARC |
// section << 16 | row: an arc's control point; TAG_ECC | which << 28 | section << 16 | row: an
// eccentricity handle; a connection point (SE_CPOINT): its row.
//
// A glued control point (a Connect entry from Controls.Row_k) follows its target through the
// formula engine; edit_canvas::follow takes the shapes whose control points moved so (pointglue::
// followers) as moved: their connectors follow too.  A callout's tail stays with structures.rs
// (Relationships, JD10).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::pointglue::{self, Anchor};
use diagram_core::{glue, ops, structure, Document, EditError, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Transaction};
use diagram_core::formula::Engine;
use ui::i18n::t;
use ui::*;

use crate::commands::{CONN_POINT_TOOL, CPOINT_BOTH, CPOINT_INWARD, CPOINT_OUTWARD};
use crate::edit_canvas::{self, shape_of, tool_of, Geo};
use crate::{edit_doc, Diagrammer};

/// Kleben an (beyond connection points and guides)
#[derive(Default)]
pub(crate) struct GlueState {
    /// Shape-Ziehpunkte: the eight handles of 2-D shapes and the control points of shapes
    pub handles: bool,
    /// Shape-Scheitelpunkte: the vertices of shapes
    pub vertices: bool,
}

pub(crate) const TAG_TARGET: u32 = 0x8000_0000;
const T_VERTEX: u32 = 0;
const T_HANDLE: u32 = 0x1000_0000;
const T_CONTROL: u32 = 0x2000_0000;
pub(crate) const TAG_ARC: u32 = 0x4000_0000;
pub(crate) const TAG_ECC: u32 = 0x2000_0000;
// the handles in ui::shape_edit's order (top left, top, ... left; y down on the screen, so
// fractions of Height from the top)
const HANDLE_FRACS: [[f64; 2]; 8] = [[0.0, 1.0], [0.5, 1.0], [1.0, 1.0], [1.0, 0.5], [1.0, 0.0], [0.5, 0.0], [0.0, 0.0], [0.0, 0.5]];

fn control_rows(doc: &Document, r: ShapeRef) -> Vec<RowKey> {
    doc.sym("Control").map(|c| doc.rows(SheetRef::Shape(r), SecKey::one(c)).into_iter().map(|x| x.key).collect()).unwrap_or_default()
}

/// the glue targets of a top-level shape beyond its connection points (Kleben an)
pub(crate) fn targets(st: &GlueState, doc: &Document, r: ShapeRef, geo: &Geo, out: &mut Vec<SePoint>) {
    let pt = |at: [f64; 2], tag: u32| SePoint { at: geo.world(at), kind: SP_TARGET, tag, dir: [0.0; 2], flags: 0 };
    if st.vertices {
        for v in ops::vertices(doc, r).into_iter().filter(|v| v.section < 0x1000) {
            out.push(pt(v.at, TAG_TARGET | T_VERTEX | v.section << 16 | (v.row & 0xFFFF)));
        }
    }
    if st.handles {
        if !glue::is_one_d(doc, r) {
            for (k, f) in HANDLE_FRACS.iter().enumerate() {
                if let Some(at) = pointglue::anchor_at(doc, r, Anchor::Handle { fx: f[0], fy: f[1] }) {
                    out.push(pt(at, TAG_TARGET | T_HANDLE | k as u32));
                }
            }
        }
        let rows = control_rows(doc, r);
        for c in ops::controls(doc, r) {
            if let Some(i) = rows.iter().position(|k| *k == c.row) {
                out.push(pt(c.at, TAG_TARGET | T_CONTROL | i as u32));
            }
        }
    }
}

/// the arc handles of the single selection in vertex editing: an SP_ARC per arc row, the two
/// SP_ECC of the arc whose control point is chosen
pub(crate) fn arc_points(doc: &Document, r: ShapeRef, geo: &Geo, chosen: Option<u32>, out: &mut Vec<SePoint>) {
    for h in ops::arc_handles(doc, r).into_iter().filter(|h| h.section < 0x1000) {
        let tag = h.section << 16 | (h.row & 0xFFFF);
        out.push(SePoint { at: geo.world(h.at), kind: SP_ARC, tag: TAG_ARC | tag, dir: [0.0; 2], flags: 0 });
        if let (Some(e), true) = (h.ecc, chosen == Some(TAG_ARC | tag)) {
            for (w, p) in e.iter().enumerate() {
                out.push(SePoint { at: geo.world(*p), kind: SP_ECC, tag: TAG_ECC | (w as u32) << 28 | tag, dir: [0.0; 2], flags: 0 });
            }
        }
    }
}

/// an end on a guide shows glued
pub(crate) fn end_flags(doc: &Document, r: ShapeRef) -> u32 {
    let g = pointglue::guide_of(doc, r);
    (if g[0].is_some() { SEF_BEGIN_GLUED } else { 0 }) | (if g[1].is_some() { SEF_END_GLUED } else { 0 })
}

// the anchor a target tag names
fn anchor_of(doc: &Document, s: ShapeRef, tag: u32) -> Option<Anchor> {
    match tag & 0x7000_0000 {
        T_VERTEX => Some(Anchor::Vertex { section: tag >> 16 & 0xFFF, row: tag & 0xFFFF }),
        T_HANDLE => HANDLE_FRACS.get((tag & 0xFF) as usize).map(|f| Anchor::Handle { fx: f[0], fy: f[1] }),
        T_CONTROL => control_rows(doc, s).get((tag & 0xFFFF) as usize).map(|k| Anchor::Control(*k)),
        _ => None,
    }
}

/// what an end or a control point glues to: a target of the edit layer made into a connection
/// point first (pointglue::anchor_point); a guide reads as nothing (glue_end puts it there)
pub(crate) fn resolve(tx: &mut Transaction, eng: &mut Engine, page: PageRef, g: SeGlue) -> Result<glue::Target, EditError> {
    Ok(match g {
        SeGlue::Point { key, tag } if tag & TAG_TARGET != 0 => {
            let s = shape_of(page, key);
            let a = anchor_of(tx.doc(), s, tag).ok_or(EditError::Invalid)?;
            let row = pointglue::anchor_point(tx, eng, s, a)?;
            glue::Target::Point { shape: s, row }
        }
        SeGlue::Point { key, tag } => glue::Target::Point { shape: shape_of(page, key), row: tag },
        SeGlue::Shape(key) => glue::Target::Shape(shape_of(page, key)),
        SeGlue::None | SeGlue::Guide(_) => glue::Target::None,
    })
}

/// an end of a 1-D shape dragged to `p` and glued to `g` (edit_canvas::apply's SeEv::End)
pub(crate) fn glue_end(tx: &mut Transaction, eng: &mut Engine, page: PageRef, s: ShapeRef, e: glue::End, p: [f64; 2], g: SeGlue) -> Result<(), EditError> {
    if let SeGlue::Guide(k) = g {
        return pointglue::glue_guide(tx, eng, s, e, p, shape_of(page, k));
    }
    let target = resolve(tx, eng, page, g)?;
    glue::glue(tx, eng, s, e, p, target)?;
    glue::update(tx, eng, &[s])
}

/// the targets of a new connector's or line's ends (edit_canvas::apply's SeEv::Connect)
pub(crate) fn connect_targets(tx: &mut Transaction, eng: &mut Engine, page: PageRef, ga: SeGlue, gb: SeGlue) -> Result<(glue::Target, glue::Target), EditError> {
    Ok((resolve(tx, eng, page, ga)?, resolve(tx, eng, page, gb)?))
}

/// a new connector's or line's ends that were dropped on guides go onto them
pub(crate) fn connect_guides(tx: &mut Transaction, eng: &mut Engine, page: PageRef, s: ShapeRef, (pa, ga): ([f64; 2], SeGlue), (pb, gb): ([f64; 2], SeGlue)) -> Result<(), EditError> {
    for (e, p, g) in [(glue::End::Begin, pa, ga), (glue::End::End, pb, gb)] {
        if let SeGlue::Guide(k) = g {
            pointglue::glue_guide(tx, eng, s, e, p, shape_of(page, k))?;
        }
    }
    Ok(())
}

// the section and row of a vertex or arc tag
fn sec_row(tag: u32) -> (u32, u32) {
    (tag >> 16 & 0xFFF, tag & 0xFFFF)
}

/// the edit layer's events of this module -> taken (edit_canvas::apply does nothing more)
pub(crate) fn apply(app: &mut Diagrammer, ui: &mut Ui, ev: &SeEv) -> bool {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return false };
    let owner = Owner::Page(page);
    let cpoint = tool_of(app) == SE_CPOINT;
    let kind_of = |idx: u32| app.model.points.get(idx as usize).map(|q| q.kind);
    let mut chosen: Option<(u64, u32)> = None;
    let r = match ev {
        SeEv::Point { key, idx, tag, at, glue: g, done } => {
            let (s, p, done, tag, g) = (shape_of(page, *key), geo.page(*at), *done, *tag, *g);
            let Some(o) = app.opened.as_mut() else { return false };
            match kind_of(*idx) {
                Some(SP_CONN) if cpoint => edit_doc::edit(o, t("Verbindungspunkt verschieben"), done, |tx, eng| {
                    pointglue::move_point(tx, eng, s, tag, p)?;
                    edit_canvas::follow(tx, eng, owner, &[s])
                }),
                Some(SP_ARC) => {
                    let (sec, row) = sec_row(tag);
                    edit_doc::edit(o, t("Bogen ändern"), done, |tx, eng| {
                        ops::set_arc_point(tx, eng, s, sec, row, p)?;
                        edit_canvas::follow(tx, eng, owner, &[s])
                    })
                }
                Some(SP_ECC) => {
                    let (sec, row) = sec_row(tag);
                    edit_doc::edit(o, t("Bogen ändern"), done, |tx, eng| {
                        ops::set_eccentricity(tx, eng, s, sec, row, p)?;
                        edit_canvas::follow(tx, eng, owner, &[s])
                    })
                }
                // a control point dropped on a target (a callout's tail is structures.rs')
                Some(SP_CONTROL) if g != SeGlue::None && structure::kind(&o.doc, s) != Some(structure::Kind::Callout) => {
                    let Some(row) = app.model.controls.get(tag as usize).copied() else { return true };
                    edit_doc::edit(o, t("Steuerpunkt verschieben"), done, |tx, eng| {
                        let target = resolve(tx, eng, page, g)?;
                        pointglue::glue_control(tx, eng, s, row, p, target)?;
                        edit_canvas::follow(tx, eng, owner, &[s])
                    })
                }
                _ => return false,
            }
        }
        SeEv::Points { key, tags, total, done } => {
            let s = shape_of(page, *key);
            let d = geo.page_vec(*total);
            let list: Vec<(u32, u32)> = tags.iter().filter(|t| *t & (TAG_TARGET | TAG_ARC | TAG_ECC) == 0).map(|t| sec_row(*t)).collect();
            let Some(o) = app.opened.as_mut() else { return true };
            edit_doc::edit(o, t("Scheitelpunkte verschieben"), *done, |tx, eng| {
                ops::move_vertices(tx, eng, s, &list, d)?;
                edit_canvas::follow(tx, eng, owner, &[s])
            })
        }
        SeEv::PointsDelete { key, tags } => {
            let s = shape_of(page, *key);
            let list: Vec<(u32, u32)> = tags.iter().filter(|t| *t & (TAG_TARGET | TAG_ARC | TAG_ECC) == 0).map(|t| sec_row(*t)).collect();
            let Some(o) = app.opened.as_mut() else { return true };
            edit_doc::edit(o, t("Scheitelpunkte löschen"), true, |tx, eng| {
                ops::delete_vertices(tx, eng, s, &list)?;
                edit_canvas::follow(tx, eng, owner, &[s])
            })
        }
        // an arc's control point chosen and Del: nothing (the vertex at its end stays)
        SeEv::PointDelete { tag, .. } if !cpoint && tag & TAG_ARC != 0 => Ok(()),
        SeEv::PointDelete { key, tag } if cpoint => {
            let (s, row) = (shape_of(page, *key), *tag);
            let Some(o) = app.opened.as_mut() else { return true };
            edit_doc::edit(o, t("Verbindungspunkt löschen"), true, |tx, eng| {
                pointglue::delete_point(tx, eng, s, row)?;
                edit_canvas::follow(tx, eng, owner, &[s])
            })
        }
        SeEv::PointInsert { key, at } if cpoint => {
            let (s, p) = (shape_of(page, *key), geo.page(*at));
            let Some(o) = app.opened.as_mut() else { return true };
            edit_doc::edit(o, t("Verbindungspunkt hinzufügen"), true, |tx, eng| pointglue::add_point(tx, eng, s, p)).map(|ix| chosen = Some((*key, ix)))
        }
        _ => return false,
    };
    // a refused edit (a locked or guarded cell): the drawing stays as it was
    let _ = r;
    if let Some((k, ix)) = chosen {
        app.se.select(&[k]);
        app.se.vertex = Some((k, ix));
        app.se.chosen.insert(ix);
    }
    ui.repass();
    true
}

/// (SS-30) the Tip of the control point of the selection under the pointer, while nothing is
/// dragged: its part key and text
pub(crate) fn control_tip(app: &Diagrammer, ui: &Ui) -> Option<(u64, String)> {
    let m = &app.model;
    if m.controls.is_empty() || app.se.busy() || ui.popup_count() != 0 || app.text.is_some() {
        return None;
    }
    let (page, o) = (m.page?, app.opened.as_ref()?);
    let vp = app.canvas.vp;
    let (mx, my) = ui.mouse();
    if mx < vp.rect[0] || my < vp.rect[1] || mx >= vp.rect[0] + vp.rect[2] || my >= vp.rect[1] + vp.rect[3] {
        return None;
    }
    let p = [mx as f32 + 0.5, my as f32 + 0.5];
    let reach = ui.dp(6) as f32;
    for sh in m.shapes.iter().filter(|s| app.se.sel.contains(&s.key)) {
        for q in m.points.get(sh.points.start as usize..sh.points.end as usize).unwrap_or(&[]) {
            if q.kind != SP_CONTROL {
                continue;
            }
            let s = vp.to_screen(q.at);
            if (s[0] - p[0]).abs() > reach || (s[1] - p[1]).abs() > reach {
                continue;
            }
            let row = *m.controls.get(q.tag as usize)?;
            let shape = shape_of(page, sh.key);
            let tip = ops::controls(&o.doc, shape).into_iter().find(|c| c.row == row).map(|c| c.tip)?;
            let tip = tip.trim();
            if tip.is_empty() {
                return None;
            }
            return Some(((sh.key << 20 | 0x8_0000 | q.tag as u64) + 1, String::from(tip)));
        }
    }
    None
}

// the chosen connection point of the connection point tool: its shape and row
fn chosen_point(app: &Diagrammer) -> Option<(ShapeRef, u32)> {
    let page = app.model.page?;
    if tool_of(app) != SE_CPOINT {
        return None;
    }
    let (k, row) = app.se.vertex?;
    let s = shape_of(page, k);
    pointglue::point_kind(&app.opened.as_ref()?.doc, s, row).map(|_| (s, row))
}

/// the canvas's context menu is the chosen connection point's (its kind)
pub(crate) fn point_menu(app: &Diagrammer) -> bool {
    chosen_point(app).is_some()
}

fn kind_of_cmd(id: u32) -> Option<u8> {
    match id {
        CPOINT_INWARD => Some(0),
        CPOINT_OUTWARD => Some(1),
        CPOINT_BOTH => Some(2),
        _ => None,
    }
}

/// CPOINT_INWARD, CPOINT_OUTWARD, CPOINT_BOTH need a chosen connection point
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    kind_of_cmd(id).is_none() || chosen_point(app).is_some()
}

/// the chosen connection point has this command's kind
pub(crate) fn kind_checked(app: &Diagrammer, id: u32) -> bool {
    let Some(k) = kind_of_cmd(id) else { return false };
    chosen_point(app).and_then(|(s, row)| pointglue::point_kind(&app.opened.as_ref()?.doc, s, row)) == Some(k)
}

/// Nach innen, Nach außen, Nach innen und außen: the chosen connection point's kind
pub(crate) fn set_kind(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let (Some(kind), Some((s, row))) = (kind_of_cmd(id), chosen_point(app)) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Art des Verbindungspunkts"), true, |tx, eng| pointglue::set_point_kind(tx, eng, s, row, kind));
    ui.repass();
}

/// Verbindungspunkt (Start › Tools): the connection point tool
pub(crate) fn tool(app: &mut Diagrammer) {
    crate::edit_cmds::tool(app, CONN_POINT_TOOL);
}

/// a glue option changed: the edit layer's targets are made anew
pub(crate) fn options_changed(app: &mut Diagrammer) {
    app.model = edit_canvas::Model::default();
}
