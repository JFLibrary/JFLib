// edit_cmds -- (JD7) the handlers of the editing commands (commands.rs HANDLERS calls them):
// undo and redo, the clipboard for shapes inside the app, select all, the tools, arrange (z-order,
// align, distribute, rotate, flip), group and ungroup, the connector styles, edit text, the view
// toggles (rulers, page breaks, guides, dynamic grid, connection points; snap and glue: dlg_snap.rs), saving
// and a new drawing
//
//   available(app, id)     whether a command can run now (a selection, something to undo ...);
//                          commands.rs disables it otherwise
//
// Every change is one undo step (edit_doc::edit) named after the command, so Rückgängig names
// what it takes back.  The clipboard holds shapes copied from this app (diagram_core::ops::Clip:
// the shapes with their connections among them); pasting gives them new IDs and puts them a grid
// step down and to the right of the originals, or where they were after a cut.

use alloc::vec::Vec;

use diagram_core::{ops, Owner, ShapeRef};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, key_of, selected};
use crate::edit_doc;
use crate::Diagrammer;

/// whether command `id` can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let open = app.opened.is_some();
    let sel = !app.se.sel.is_empty();
    match id {
        // (JD10) structures and layout
        id if crate::structures_cmds::owns(id) => crate::structures_cmds::available(app, id),
        id if crate::layout_ui::owns(id) => crate::layout_ui::available(app, id),
        UNDO => app.opened.as_ref().is_some_and(|o| o.ed.history.can_undo()),
        REDO => app.opened.as_ref().is_some_and(|o| o.ed.history.can_redo()),
        CUT | COPY | DUPLICATE | BRING_FRONT | BRING_FORWARD | SEND_BACK | SEND_BACKWARD | ROTATE_LEFT | ROTATE_RIGHT | FLIP_H | FLIP_V => sel,
        GROUP | ALIGN_LEFT | ALIGN_CENTER | ALIGN_RIGHT | ALIGN_TOP | ALIGN_MIDDLE | ALIGN_BOTTOM => app.se.sel.len() > 1,
        DISTRIBUTE_H | DISTRIBUTE_V => app.se.sel.len() > 2,
        UNGROUP => sel && group_selected(app),
        EDIT_TEXT => app.se.sel.len() == 1,
        // (JD8 rest) an empty app clipboard: the system's text or picture (cmds_paste.rs)
        PASTE => open && (app.clip.is_some() || crate::cmds_paste::system_has(app)),
        SELECT_BY_TYPE => open,
        CONN_RIGHT_ANGLE | CONN_STRAIGHT | CONN_CURVED | CONN_RESET => sel,
        SAVE | SAVE_AS | SELECT_ALL => open,
        _ => true,
    }
}

fn group_selected(app: &Diagrammer) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return false };
    app.se.sel.iter().any(|&k| o.doc.shape(edit_canvas::shape_of(page, k)).is_some_and(|s| s.header().kind == diagram_core::ShapeKind::Group))
}

fn owner(app: &Diagrammer) -> Option<Owner> {
    app.model.page.map(Owner::Page)
}

/// Rückgängig
pub(crate) fn undo(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(o) = app.opened.as_mut() {
        if edit_doc::undo(o) {
            ui.repass();
        }
    }
}

/// Wiederholen
pub(crate) fn redo(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(o) = app.opened.as_mut() {
        if edit_doc::redo(o) {
            ui.repass();
        }
    }
}

/// Kopieren
pub(crate) fn copy(app: &mut Diagrammer, _ui: &mut Ui) {
    let shapes = selected(app);
    if let (Some(o), false) = (app.opened.as_ref(), shapes.is_empty()) {
        // (JD10) members and callouts come along
        app.clip = Some(ops::copy(&o.doc, &crate::structures::carried(&o.doc, &shapes)));
        app.clip_offset = 1;
        // (JD8 rest) Als Link einfügen links to them
        crate::cmds_paste::copied(app, &shapes);
    }
}

/// Ausschneiden: copy, then delete
pub(crate) fn cut(app: &mut Diagrammer, ui: &mut Ui) {
    copy(app, ui);
    app.clip_offset = 0;
    let shapes = selected(app);
    let ow = owner(app);
    let (Some(o), Some(ow)) = (app.opened.as_mut(), ow) else { return };
    if edit_canvas::delete(o, ow, &shapes).is_ok() {
        app.se.select(&[]);
    }
    ui.repass();
}

/// Einfügen: the clipboard's shapes, a grid step further each time
pub(crate) fn paste(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(ow), Some(geo)) = (owner(app), app.model.geo) else { return };
    // (JD8 rest) nothing copied in the app: the system's text or picture
    if app.clip.is_none() {
        crate::cmds_paste::paste_system(app, ui);
        return;
    }
    let Some(clip) = app.clip.take() else { return };
    let step = if app.canvas.grid > 0.0 { app.canvas.grid } else { 16.0 };
    let n = app.clip_offset as f32;
    let d = geo.page_vec([step * n, step * n]);
    let r = match app.opened.as_mut() {
        Some(o) => edit_doc::edit(o, t("Einfügen"), true, |tx, eng| {
            let new = ops::paste(tx, eng, ow, &clip, d)?;
            crate::structures::pasted(tx, eng, ow, &new)?;
            Ok(new)
        }),
        None => Ok(Vec::new()),
    };
    app.clip = Some(clip);
    if let Ok(new) = r {
        app.clip_offset += 1;
        let keys: Vec<u64> = new.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
        // (JD18, SP-13) pasted or duplicated: fields marked "ask on drop"
        crate::data_ask::after_drop(app, ui, &new);
    }
    ui.repass();
}

/// Duplizieren (Strg+D): copy and paste in one step
pub(crate) fn duplicate(app: &mut Diagrammer, ui: &mut Ui) {
    let keep = app.clip.take();
    let keep_off = app.clip_offset;
    copy(app, ui);
    paste(app, ui);
    app.clip = keep;
    app.clip_offset = keep_off;
}

/// Alles markieren
pub(crate) fn select_all(app: &mut Diagrammer, ui: &mut Ui) {
    let keys: Vec<u64> = app.model.shapes.iter().filter(|s| s.parent == 0 && s.flags & SEF_NOSELECT == 0).map(|s| s.key).collect();
    app.se.select(&keys);
    ui.repass();
}

/// a tool: the pointer, the connector, a drawing tool (stays until another is chosen)
pub(crate) fn tool(app: &mut Diagrammer, id: u32) {
    app.tool = id;
    // (JD8 rest) the Zeichentools split button keeps the drawing tool used last
    crate::cmds_draw::chosen(app, id);
}

fn change(app: &mut Diagrammer, ui: &mut Ui, label: &str, f: impl FnOnce(&mut diagram_core::Transaction, &mut diagram_core::formula::Engine, Owner, &[ShapeRef]) -> Result<(), diagram_core::EditError>) {
    let shapes = selected(app);
    let ow = owner(app);
    let (Some(o), Some(ow)) = (app.opened.as_mut(), ow) else { return };
    if shapes.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, label, true, |tx, eng| f(tx, eng, ow, &shapes));
    ui.repass();
}

/// In den Vordergrund, Eine Ebene nach vorne, In den Hintergrund, Eine Ebene nach hinten
pub(crate) fn arrange(app: &mut Diagrammer, ui: &mut Ui, z: ops::Z) {
    let label = match z {
        ops::Z::Front => t("In den Vordergrund"),
        ops::Z::Forward => t("Eine Ebene nach vorne"),
        ops::Z::Back => t("In den Hintergrund"),
        ops::Z::Backward => t("Eine Ebene nach hinten"),
    };
    change(app, ui, label, |tx, _, _, shapes| ops::reorder(tx, shapes, z));
}

/// Gruppieren
pub(crate) fn group(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = selected(app);
    let ow = owner(app);
    let (Some(o), Some(ow)) = (app.opened.as_mut(), ow) else { return };
    if shapes.len() < 2 {
        return;
    }
    if let Ok(g) = edit_doc::edit(o, t("Gruppieren"), true, |tx, eng| ops::group(tx, eng, ow, &shapes)) {
        app.se.select(&[key_of(g)]);
    }
    ui.repass();
}

/// Gruppierung aufheben: (JD21) Visio's warning first when the groups are instances of masters or
/// formulas name them (groups_cmds.rs)
pub(crate) fn ungroup(app: &mut Diagrammer, ui: &mut Ui) {
    crate::groups_cmds::ungroup(app, ui);
}

// the box of the selection on the page (drawing units): x0, y0, x1, y1
fn sel_box(app: &Diagrammer) -> Option<[f64; 4]> {
    let geo = app.model.geo?;
    let mut b: Option<[f64; 4]> = None;
    for s in app.model.shapes.iter().filter(|s| app.se.sel.contains(&s.key)) {
        let (p, q) = (geo.page([s.bbox[0], s.bbox[3]]), geo.page([s.bbox[2], s.bbox[1]]));
        let r = [p[0], p[1], q[0], q[1]];
        b = Some(match b {
            None => r,
            Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
        });
    }
    b
}

/// Um 90° nach links (dir 1) / rechts (-1) drehen, about the middle of the selection
pub(crate) fn rotate90(app: &mut Diagrammer, ui: &mut Ui, dir: f64) {
    let Some(b) = sel_box(app) else { return };
    let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let label = if dir > 0.0 { t("Um 90° nach links drehen") } else { t("Um 90° nach rechts drehen") };
    change(app, ui, label, |tx, eng, ow, shapes| {
        ops::rotate_shapes(tx, eng, shapes, c, dir * core::f64::consts::FRAC_PI_2)?;
        edit_canvas::follow(tx, eng, ow, shapes)
    });
}

/// Horizontal / vertikal spiegeln
pub(crate) fn flip(app: &mut Diagrammer, ui: &mut Ui, horizontal: bool) {
    let label = if horizontal { t("Horizontal spiegeln") } else { t("Vertikal spiegeln") };
    change(app, ui, label, |tx, eng, ow, shapes| {
        ops::flip_shapes(tx, eng, shapes, horizontal)?;
        edit_canvas::follow(tx, eng, ow, shapes)
    });
}

/// Ausrichten: the selected shapes to the anchor (the first selected), 0 left, 1 centre,
/// 2 right, 3 top, 4 middle, 5 bottom
pub(crate) fn align(app: &mut Diagrammer, ui: &mut Ui, how: u8) {
    let Some(geo) = app.model.geo else { return };
    let page = match app.model.page {
        Some(p) => p,
        None => return,
    };
    let boxes: Vec<(ShapeRef, [f64; 4])> = app
        .model
        .shapes
        .iter()
        .filter(|s| app.se.sel.contains(&s.key))
        .map(|s| {
            let (p, q) = (geo.page([s.bbox[0], s.bbox[3]]), geo.page([s.bbox[2], s.bbox[1]]));
            (edit_canvas::shape_of(page, s.key), [p[0], p[1], q[0], q[1]])
        })
        .collect();
    let anchor = boxes.iter().find(|(s, _)| key_of(*s) == app.se.primary).or(boxes.first()).map(|x| x.1);
    let Some(a) = anchor else { return };
    let target = |b: &[f64; 4]| -> [f64; 2] {
        match how {
            0 => [a[0] - b[0], 0.0],
            1 => [(a[0] + a[2]) * 0.5 - (b[0] + b[2]) * 0.5, 0.0],
            2 => [a[2] - b[2], 0.0],
            3 => [0.0, a[3] - b[3]],
            4 => [0.0, (a[1] + a[3]) * 0.5 - (b[1] + b[3]) * 0.5],
            _ => [0.0, a[1] - b[1]],
        }
    };
    let moves: Vec<(ShapeRef, [f64; 2])> = boxes.iter().map(|(s, b)| (*s, target(b))).filter(|(_, d)| d[0] != 0.0 || d[1] != 0.0).collect();
    run_moves(app, ui, t("Ausrichten"), moves);
}

/// Horizontal / vertikal verteilen: equal gaps between the first and the last shape
pub(crate) fn distribute(app: &mut Diagrammer, ui: &mut Ui, horizontal: bool) {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let mut boxes: Vec<(ShapeRef, [f64; 4])> = app
        .model
        .shapes
        .iter()
        .filter(|s| app.se.sel.contains(&s.key))
        .map(|s| {
            let (p, q) = (geo.page([s.bbox[0], s.bbox[3]]), geo.page([s.bbox[2], s.bbox[1]]));
            (edit_canvas::shape_of(page, s.key), [p[0], p[1], q[0], q[1]])
        })
        .collect();
    if boxes.len() < 3 {
        return;
    }
    let ax = if horizontal { 0 } else { 1 };
    boxes.sort_by(|a, b| (a.1[ax] + a.1[ax + 2]).partial_cmp(&(b.1[ax] + b.1[ax + 2])).unwrap_or(core::cmp::Ordering::Equal));
    let first = boxes[0].1;
    let last = boxes[boxes.len() - 1].1;
    let total: f64 = boxes.iter().map(|(_, b)| b[ax + 2] - b[ax]).sum();
    let gap = ((last[ax + 2] - first[ax]) - total) / (boxes.len() - 1) as f64;
    let mut at = first[ax];
    let mut moves = Vec::new();
    for (s, b) in &boxes {
        let d = at - b[ax];
        if d.abs() > 1e-9 {
            moves.push((*s, if horizontal { [d, 0.0] } else { [0.0, d] }));
        }
        at += b[ax + 2] - b[ax] + gap;
    }
    run_moves(app, ui, if horizontal { t("Horizontal verteilen") } else { t("Vertikal verteilen") }, moves);
}

fn run_moves(app: &mut Diagrammer, ui: &mut Ui, label: &str, moves: Vec<(ShapeRef, [f64; 2])>) {
    let ow = owner(app);
    let (Some(o), Some(ow)) = (app.opened.as_mut(), ow) else { return };
    if moves.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, label, true, |tx, eng| {
        let mut all = Vec::new();
        for (s, d) in &moves {
            ops::move_shapes(tx, eng, &[*s], *d)?;
            all.push(*s);
        }
        edit_canvas::follow(tx, eng, ow, &all)
    });
    ui.repass();
}

/// Verbinder › Rechtwinklig, Gerade, Gebogen: the style of the selected connectors
pub(crate) fn conn_style(app: &mut Diagrammer, ui: &mut Ui, style: diagram_core::glue::Style) {
    let label = match style {
        diagram_core::glue::Style::Straight => t("Gerade"),
        diagram_core::glue::Style::Curved => t("Gebogen"),
        _ => t("Rechtwinklig"),
    };
    change(app, ui, label, |tx, eng, _, shapes| {
        let conns: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| diagram_core::glue::is_connector(tx.doc(), s)).collect();
        diagram_core::glue::set_style(tx, eng, &conns, style)
    });
}

/// Verbinder zurücksetzen / neu routen: the selected connectors drawn anew with few bends
pub(crate) fn conn_reset(app: &mut Diagrammer, ui: &mut Ui) {
    change(app, ui, t("Verbinder zurücksetzen"), |tx, eng, _, shapes| {
        let conns: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| diagram_core::glue::is_connector(tx.doc(), s)).collect();
        diagram_core::glue::reset(tx, eng, &conns)
    });
}

/// Text bearbeiten (F2)
pub(crate) fn edit_text(app: &mut Diagrammer, ui: &mut Ui) {
    if let (1, Some(&k)) = (app.se.sel.len(), app.se.sel.iter().next()) {
        crate::edit_text::begin(app, ui, k, None);
    }
}
