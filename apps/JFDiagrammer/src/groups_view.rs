// groups_view -- (JD21) groups on the canvas: Gruppe öffnen (SP-15) as an isolated view of the
// group on the canvas, and how the edit layer treats groups (SS-17): SelectMode 1 and 2,
// DontMoveChildren, snapping and gluing to members (IsSnapTarget), typing into a group
// (IsTextEditTarget), a double click that opens the group (EventDblClick = OPENGROUPWIN()),
// shapes dropped or moved onto a group (IsDropTarget, IsDropSource)
//
//   scope(app) -> Option<ShapeRef>     the group open on the page shown
//   open(app, ui, g) / close(app, ui)  Gruppe öffnen / schließen: closing fits the group's box
//                                      around its members (Ausrichtungsfeld aktualisieren, SP-16)
//                                      as one undo step when it changes
//   bar(app, ui)                       above the canvas while a group is open: its name and the
//                                      button Gruppe schließen (panes::center)
//   overlay(app, ui)                   the veil over the page outside the open group
//   tops / open_too / flags / snaps    what edit_canvas::model lists: the open group's members as
//                                      top-level shapes, the members of "members first" groups and
//                                      of groups that are snap targets, their flags
//   hit(..)                            edit_canvas's hit test with the open group and SelectMode 2
//   text(app, ui, key, ch) -> bool     a double click or typing on a group
//   moved / adopt                      after a move or a new shape: into the open group, onto a
//                                      group that takes drops
//
// Why an isolated view on the canvas and not Visio's own window: JFDiagrammer has one window
// (also in the browser and on Android), the Shapes panel, the rulers, the zoom and every command
// stay as they are, and scripts and screen readers see the same canvas.  While a group is open its
// members are the canvas' top-level shapes (selected by one click, marquee, Ctrl+A, Tab), shapes
// drawn, dropped from the Shapes panel or moved from the page into the group's frame join it, and
// the page around it is dimmed but stays usable (a page shape dragged into the frame joins the
// group, as when it is dragged into Visio's group window).  The group's box follows its members
// when the view closes (Visio leaves that to Ausrichtungsfeld aktualisieren; FEATURES SP-15 asks
// for it).

use alloc::vec::Vec;

use diagram_core::groups::{self, SelectMode};
use diagram_core::{ops, Document, EditError, Owner, PageRef, ShapeRef, Transaction};
use ui::i18n::{t, tf};
use ui::*;

use crate::edit_canvas::{self, key_of, Geo};
use crate::{edit_doc, Diagrammer};

const ID_BAR: u32 = 0x7A10_0000;

/// the group open on the page shown (None: none, or it is gone)
pub(crate) fn scope(app: &Diagrammer) -> Option<ShapeRef> {
    let (serial, page, g) = app.groups.view?;
    let o = app.opened.as_ref()?;
    let tab = o.tabs.get(app.page_index())?;
    (o.serial == serial && tab.page == page && groups::is_group(&o.doc, g)).then_some(g)
}

// a value that changes the edit layer's model version with the open group
pub(crate) fn ver_salt(scope: Option<ShapeRef>) -> u32 {
    scope.map_or(0, |g| (g.slot.wrapping_add(1)).wrapping_mul(0x9E37_79B1))
}

/// Gruppe öffnen: the group's members on their own (an open group inside it: that one)
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui, g: ShapeRef) {
    let Some(o) = app.opened.as_ref() else { return };
    let Some(tab) = o.tabs.get(app.page_index()) else { return };
    if !groups::is_group(&o.doc, g) {
        return;
    }
    let name = edit_canvas::shape_label(&o.doc, g);
    app.groups.view = Some((o.serial, tab.page, g));
    app.se.select(&[]);
    app.model.geo = None;
    ui.announce(&tf("Gruppe „{}“ geöffnet", &[&name]), LIVE_POLITE);
    ui.repass();
}

/// Gruppe schließen: the group's box around its members (one undo step when it changes), the
/// group selected
pub(crate) fn close(app: &mut Diagrammer, ui: &mut Ui) {
    let g = scope(app);
    app.groups.view = None;
    app.model.geo = None;
    if let (Some(g), Some(o)) = (g, app.opened.as_mut()) {
        let owner = g.owner;
        let _ = edit_doc::edit(o, t("Ausrichtungsfeld aktualisieren"), true, |tx, eng| {
            if diagram_core::shapeops::update_alignment_box(tx, eng, g)? {
                edit_canvas::follow(tx, eng, owner, &[g])?;
            }
            Ok(())
        });
        app.se.select(&[key_of(g)]);
    }
    ui.repass();
}

/// the bar above the canvas while a group is open
pub(crate) fn bar(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(g) = scope(app) else {
        app.groups.view = None;
        return;
    };
    let name = app.opened.as_ref().map(|o| edit_canvas::shape_label(&o.doc, g)).unwrap_or_default();
    ui.next_size(-1, 30);
    ui.box_begin(0, BOX_H | BOX_LAYER, 4);
    ui.next_size(-1, 26);
    ui.text(TS_BODY, &tf("Gruppe „{}“ geöffnet: Shapes hineinziehen oder bearbeiten", &[&name]));
    ui.next_size(0, 26);
    if ui.button(ID_BAR + 1, t("Gruppe schließen")) & EV_ACT != 0 {
        close(app, ui);
    }
    ui.box_end();
}

// the open group's box and its members' on the page
fn view_box(doc: &Document, g: ShapeRef) -> Option<[f64; 4]> {
    let mut all = alloc::vec![g];
    all.extend(doc.shape(g)?.children());
    ops::bounds(doc, &all)
}

/// the veil over the page around the open group, a frame around it; (ST-03) while shapes are
/// dragged, the edge of a container they would sit on
pub(crate) fn overlay(app: &mut Diagrammer, ui: &mut Ui) {
    edges(app, ui);
    let (Some(g), Some(geo), Some(o)) = (scope(app), app.model.geo, app.opened.as_ref()) else { return };
    let Some(b) = view_box(&o.doc, g) else { return };
    let vp = app.canvas.vp;
    let r = vp.rect;
    let (a, z) = (vp.to_screen(geo.world([b[0], b[3]])), vp.to_screen(geo.world([b[2], b[1]])));
    let pad = ui.dp(6) as f32;
    let (x0, y0, x1, y1) = ((a[0] - pad) as i32, (a[1] - pad) as i32, (z[0] + pad) as i32, (z[1] + pad) as i32);
    ui.clip_push(r[0], r[1], r[2], r[3]);
    let veil = (ui.color(T_BG) & 0x00FF_FFFF) | 0xA000_0000;
    ui.draw_color(veil);
    let (rx1, ry1) = (r[0] + r[2], r[1] + r[3]);
    ui.draw_rect(r[0], r[1], r[2], (y0 - r[1]).max(0));
    ui.draw_rect(r[0], y1, r[2], (ry1 - y1).max(0));
    ui.draw_rect(r[0], y0, (x0 - r[0]).max(0), (y1 - y0).max(0));
    ui.draw_rect(x1, y0, (rx1 - x1).max(0), (y1 - y0).max(0));
    ui.draw_color(ui.color(T_ACCENT) | 0xFF00_0000);
    ui.draw_frame(x0, y0, x1 - x0, y1 - y0, 0, ui.dp(1).max(1));
    ui.clip_pop();
}

/// the shapes the edit layer lists as top-level: the page's, the open group's members instead of
/// the group
pub(crate) fn tops(doc: &Document, owner: Owner, scope: Option<ShapeRef>) -> Vec<ShapeRef> {
    let mut v: Vec<ShapeRef> = doc.top_shapes(owner).filter(|&s| Some(s) != scope).collect();
    if let Some(g) = scope {
        v.extend(doc.shape(g).map(|s| s.children().collect::<Vec<_>>()).unwrap_or_default());
    }
    v
}

/// the groups whose members the edit layer lists (`open`: the selected groups and the groups
/// around the selection) with those of open_too, without the open group (its members are listed
/// as top-level shapes)
pub(crate) fn extend_open(open: &mut Vec<ShapeRef>, doc: &Document, tops: &[ShapeRef], scope: Option<ShapeRef>) {
    open.extend(open_too(doc, tops));
    if let Some(g) = scope {
        open.retain(|&x| x != g);
    }
}

/// the groups among `tops` whose members the edit layer lists too: "members first" groups (and
/// such groups inside them), groups that let shapes snap and glue to their members
pub(crate) fn open_too(doc: &Document, tops: &[ShapeRef]) -> Vec<ShapeRef> {
    let mut out = Vec::new();
    let mut stack: Vec<ShapeRef> = tops.iter().copied().filter(|&s| groups::is_group(doc, s)).collect();
    let mut depth = 0;
    while let Some(g) = stack.pop() {
        let p = groups::props(doc, g);
        if p.select_mode == SelectMode::MembersFirst || (p.snap_target && tops.contains(&g)) {
            out.push(g);
            if p.select_mode == SelectMode::MembersFirst && depth < 64 {
                depth += 1;
                stack.extend(doc.shape(g).map(|s| s.children().filter(|&k| groups::is_group(doc, k)).collect::<Vec<_>>()).unwrap_or_default());
            }
        }
    }
    out
}

/// the edit layer's flags of a member `r` (listed with its group, not the open group's): it
/// cannot be dragged (DontMoveChildren), a second click selects its group (members first), it is
/// listed for snapping only (a group that is selected as a whole: SelectMode 0)
pub(crate) fn flags(doc: &Document, r: ShapeRef, scope: Option<ShapeRef>) -> u32 {
    let Some(g) = doc.shape(r).and_then(|s| s.parent()) else { return 0 };
    if Some(g) == scope {
        return 0;
    }
    let p = groups::props(doc, g);
    let mut f = 0;
    if p.dont_move_children {
        f |= SEF_LOCK_MOVE;
    }
    match p.select_mode {
        SelectMode::MembersFirst => f |= SEF_UP,
        SelectMode::GroupOnly => f |= SEF_NOSELECT,
        SelectMode::GroupFirst => {}
    }
    f
}

/// whether member `r`'s connection points are glue targets: its group lets shapes snap to its
/// members (IsSnapTarget)
pub(crate) fn snaps(doc: &Document, r: ShapeRef) -> bool {
    doc.shape(r).and_then(|s| s.parent()).is_some_and(|g| groups::props(doc, g).snap_target)
}

/// edit_canvas's hit test with the groups' rules: while a group is open its members are what a
/// click reaches (the page beside it too, not the group itself); a "members first" group hands
/// the click to the member under the pointer unless it is the selection itself
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit(doc: &Document, d: &diagram_core::render::Drawing, geo: &Geo, page: PageRef, p: [f32; 2], tol: f32, within: u64, scope: Option<ShapeRef>, sel: &[u64]) -> u64 {
    let mut k = 0;
    if within == 0 {
        if let Some(g) = scope {
            k = edit_canvas::hit_at(doc, d, geo, page, p, tol, key_of(g));
            if k == 0 {
                let top = edit_canvas::hit_at(doc, d, geo, page, p, tol, 0);
                if top == key_of(g) {
                    return 0;
                }
                k = top;
            }
        }
    }
    if k == 0 {
        k = edit_canvas::hit_at(doc, d, geo, page, p, tol, within);
    }
    // members first: the member under the pointer (unless the group is what is selected)
    for _ in 0..64 {
        let s = edit_canvas::shape_of(page, k);
        if k == 0 || sel == [k] || !groups::is_group(doc, s) || groups::props(doc, s).select_mode != SelectMode::MembersFirst || Some(s) == scope {
            break;
        }
        let m = edit_canvas::hit_at(doc, d, geo, page, p, tol, k);
        if m == 0 {
            break;
        }
        k = m;
    }
    k
}

/// a double click (ch None) or typing on shape `key`: a group whose double click opens it opens;
/// a group that hands its text on (IsTextEditTarget off) has its top member edited -> handled
pub(crate) fn text(app: &mut Diagrammer, ui: &mut Ui, key: u64, ch: Option<char>) -> bool {
    let Some(page) = app.model.page else { return false };
    let s = edit_canvas::shape_of(page, key);
    let Some(o) = app.opened.as_ref() else { return false };
    if ch.is_none() && groups::is_group(&o.doc, s) && groups::double_click_opens(&o.doc, s) {
        open(app, ui, s);
        return true;
    }
    let to = groups::text_target(&o.doc, s);
    if to != s {
        app.text_req = Some((key_of(to), ch, false));
        ui.repass();
        return true;
    }
    false
}

/// after a move released (`done`): page shapes moved into the open group's frame join it; else
/// shapes that may (IsDropSource) join a group that takes drops (IsDropTarget) they lie on
pub(crate) fn moved(tx: &mut Transaction, eng: &mut diagram_core::formula::Engine, owner: Owner, scope: Option<ShapeRef>, shapes: &[ShapeRef], done: bool) -> Result<(), EditError> {
    if !done {
        return Ok(());
    }
    match scope {
        Some(g) => {
            let into: Vec<ShapeRef> = {
                let doc = tx.doc();
                let f = ops::frame(doc, g);
                shapes
                    .iter()
                    .copied()
                    .filter(|&s| doc.shape(s).is_some_and(|x| x.parent().is_none()))
                    .filter(|&s| {
                        let Some(b) = ops::bounds(doc, &[s]) else { return false };
                        let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
                        f.local(mid).is_some_and(|q| q[0] >= 0.0 && q[1] >= 0.0 && q[0] <= f.w && q[1] <= f.h)
                    })
                    .collect()
            };
            if !into.is_empty() {
                groups::add_to_group(tx, eng, g, &into)?;
                edit_canvas::follow(tx, eng, owner, &[g])?;
            }
        }
        None => {
            let took = groups::dropped(tx, eng, owner, shapes)?;
            if !took.is_empty() {
                edit_canvas::follow(tx, eng, owner, &took)?;
            }
        }
    }
    Ok(())
}

/// a new shape (drawn, dropped from the Shapes panel): while a group is open it joins it, else
/// it joins a group that takes drops when it may
pub(crate) fn adopt(tx: &mut Transaction, eng: &mut diagram_core::formula::Engine, owner: Owner, scope: Option<ShapeRef>, shapes: &[ShapeRef]) -> Result<(), EditError> {
    match scope {
        Some(g) => {
            groups::add_to_group(tx, eng, g, shapes)?;
            edit_canvas::follow(tx, eng, owner, &[g])
        }
        None => moved(tx, eng, owner, None, shapes, true),
    }
}

// (ST-03) the feedback of a boundary member: while shapes are dragged, the edge of each container
// they would sit on is drawn thick in the accent colour (Visio highlights the one edge)
fn edges(app: &Diagrammer, ui: &mut Ui) {
    use diagram_core::structure::{self, edge, Edge};
    if app.structs.dragging.is_empty() || !app.se.busy() {
        return;
    }
    let (Some(geo), Some(page), Some(o)) = (app.model.geo, app.model.page, app.opened.as_ref()) else { return };
    let doc = &o.doc;
    let owner = Owner::Page(page);
    let shapes: Vec<ShapeRef> = app.structs.dragging.iter().map(|&k| edit_canvas::shape_of(page, k)).collect();
    let moving = structure::carried(doc, &shapes);
    let vp = app.canvas.vp;
    let r = vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    ui.draw_color(ui.color(T_ACCENT) | 0xFF00_0000);
    let w = (ui.dp(4).max(3) * 256) as i32;
    for &s in &shapes {
        let inside = structure::drop_targets(doc, owner, s, &moving);
        let now = structure::containers_of(doc, s);
        for (c, e) in edge::targets(doc, owner, s, &moving) {
            // a plain member moved partly out stays one (structure::settle)
            if inside.contains(&c) || (now.contains(&c) && edge::edge_of(doc, s, c).is_none()) {
                continue;
            }
            let b = structure::shape_box(doc, c);
            let (p, q) = match e {
                Edge::Left => ([b[0], b[1]], [b[0], b[3]]),
                Edge::Right => ([b[2], b[1]], [b[2], b[3]]),
                Edge::Top => ([b[0], b[3]], [b[2], b[3]]),
                Edge::Bottom => ([b[0], b[1]], [b[2], b[1]]),
                Edge::None => continue,
            };
            let (p, q) = (vp.to_screen(geo.world(p)), vp.to_screen(geo.world(q)));
            ui.draw_line((p[0] * 256.0) as i32, (p[1] * 256.0) as i32, (q[0] * 256.0) as i32, (q[1] * 256.0) as i32, w);
        }
    }
    ui.clip_pop();
}
