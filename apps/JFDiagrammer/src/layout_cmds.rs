// layout_cmds -- (JD21) Entwurf › Layout › Seitenlayout neu anordnen (CN-13, RW-09) with its
// styles and the live preview while a row is pointed at; Start › Anordnen › Positionieren:
// Automatischer Abstand, Automatische Ausrichtung und Abstände (CN-14), Von Seitenumbrüchen
// verschieben, Seitenumbrüche vermeiden (PR-03), Diagramm drehen (RS-25)
//
//   LayoutCmds                  the state: the live preview, Seitenumbrüche vermeiden, the
//                               spacing of Abstandsoptionen, the dialogs (dlg_layout.rs)
//   place_of(id)                a row of the menu -> its placement (diagram_core::layout::Place)
//   relayout(app, ui, place, done)   the page (two or more shapes selected: those) laid out;
//                               done = false: as the live preview (edit_doc's preview)
//   auto(app, ui, align, space, done)   Automatische Ausrichtung und Abstände / Automatischer
//                               Abstand on the selection (none: the page)
//   off_breaks / turn           Von Seitenumbrüchen verschieben, Diagramm drehen
//   toggle_avoid / avoid_on     Seitenumbrüche vermeiden (kept in the settings)
//   breaks(doc, page)           the page breaks as the canvas draws them (drawing units)
//   available(app, id)          when the commands can run
//   row_event(app, ui, id, ev)  ribbon.rs: a menu row's events (pointed at: the preview)
//   after(app, ui)              every pass after the ribbon: a preview whose row is no longer
//                               pointed at goes
//
// Each command is one undo step (edit_doc::edit): the moves, what the shapes carry along
// (members, callouts), the containers settled, the connectors routed (layout::reroute), the
// page's PlaceStyle as Visio keeps it, Seite vergrößern when the page's ResizePage says so, and
// the shapes moved off the page breaks when Seitenumbrüche vermeiden is on (Visio: Auto Align
// and Auto Space; here also Re-Layout).  The rules of the layouts are diagram_core::layout's.

use alloc::vec::Vec;
use core::cell::Cell;

use diagram_core::layout::{self, Dir, Opts, Place, Routing, Style, Turn};
use diagram_core::{glue, CellAddr, Document, Owner, PageRef, ShapeRef, SheetRef};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::{edit_canvas, edit_doc, Diagrammer};

/// the state of the layout commands
#[derive(Default)]
pub(crate) struct LayoutCmds {
    /// the row shown as a live preview, whether a row was pointed at this pass
    preview: Option<u32>,
    hovered: bool,
    /// Seitenumbrüche vermeiden (None: not read from the settings yet)
    avoid: Cell<Option<bool>>,
    /// Abstandsoptionen: a spacing of its own (x, y; inches) instead of the page's
    pub spacing: Option<[f64; 2]>,
    /// whether the page has connected shapes: (drawing's version, page, answer)
    connected: Cell<Option<(u32, u32, PageRef, bool)>>,
    pub dlg: crate::dlg_layout::LayoutDialog,
}

/// the gallery's rows and their placements
pub(crate) fn place_of(id: u32) -> Option<Place> {
    let p = |style, dir| Some(Place::new(style, dir));
    match id {
        LAYOUT_FLOW_DOWN => p(Style::Flowchart, Dir::Down),
        LAYOUT_FLOW_UP => p(Style::Flowchart, Dir::Up),
        LAYOUT_FLOW_RIGHT => p(Style::Flowchart, Dir::Right),
        LAYOUT_FLOW_LEFT => p(Style::Flowchart, Dir::Left),
        LAYOUT_HIER_DOWN => p(Style::Hierarchy, Dir::Down),
        LAYOUT_HIER_UP => p(Style::Hierarchy, Dir::Up),
        LAYOUT_HIER_RIGHT => p(Style::Hierarchy, Dir::Right),
        LAYOUT_HIER_LEFT => p(Style::Hierarchy, Dir::Left),
        LAYOUT_TREE_DOWN_RIGHT => Place::from_code(7),
        LAYOUT_TREE_RIGHT_DOWN => Place::from_code(8),
        LAYOUT_TREE_DOWN_LEFT => Place::from_code(14),
        LAYOUT_TREE_LEFT_DOWN => Place::from_code(13),
        LAYOUT_RADIAL => p(Style::Radial, Dir::Down),
        LAYOUT_CIRCULAR => p(Style::Circular, Dir::Down),
        _ => None,
    }
}

// the commands with a live preview while their row is pointed at
fn previews(id: u32) -> bool {
    place_of(id).is_some() || matches!(id, AUTO_SPACE | AUTO_ALIGN_SPACE)
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num()).filter(|v| v.is_finite())
}

// the page shown and its owner
fn page(app: &Diagrammer) -> Option<PageRef> {
    app.opened.as_ref()?;
    app.model.page
}

// the selected 2D shapes when two or more are selected (a layout of the selection), else none
// (the page)
pub(crate) fn chosen(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let sel: Vec<ShapeRef> = edit_canvas::selected(app).into_iter().filter(|&s| !glue::is_one_d(&o.doc, s)).collect();
    if sel.len() > 1 { sel } else { Vec::new() }
}

// the selected shapes (any number) for the adjustments
fn selection(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    edit_canvas::selected(app).into_iter().filter(|&s| !glue::is_one_d(&o.doc, s)).collect()
}

/// the page breaks as the canvas draws them (edit_canvas::page_breaks: the paper less its
/// margins, from the left and from the bottom) in drawing units: (vertical lines' x,
/// horizontal lines' y); none when the page fits on its paper
pub(crate) fn breaks(doc: &Document, page: PageRef) -> (Vec<f64>, Vec<f64>) {
    let s = SheetRef::Page(page);
    let kind = num(doc, s, "PaperKind").unwrap_or(9.0) as i32;
    let (mut pw, mut ph) = match kind {
        1 => (8.5, 11.0),
        5 => (8.5, 14.0),
        8 => (297.0 / 25.4, 420.0 / 25.4),
        11 => (148.0 / 25.4, 210.0 / 25.4),
        _ => (210.0 / 25.4, 297.0 / 25.4),
    };
    if num(doc, s, "PrintPageOrientation").unwrap_or(1.0) as i32 == 2 {
        core::mem::swap(&mut pw, &mut ph);
    }
    let margin = |n: &str| num(doc, s, n).unwrap_or(0.25);
    let (uw, uh) = (pw - margin("PageLeftMargin") - margin("PageRightMargin"), ph - margin("PageTopMargin") - margin("PageBottomMargin"));
    let scale = diagram_core::pagesetup::scale(doc, page);
    let [w, h] = diagram_core::pagesetup::size(doc, page);
    let (gw, gh) = (w * scale, h * scale);
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    if !(uw > 0.1 && uh > 0.1) || (gw <= pw + 1e-6 && gh <= ph + 1e-6) {
        return (xs, ys);
    }
    let mut x = uw;
    while x < gw - 1e-6 && xs.len() < 1000 {
        xs.push(x / scale);
        x += uw;
    }
    let mut y = uh;
    while y < gh - 1e-6 && ys.len() < 1000 {
        ys.push(y / scale);
        y += uh;
    }
    (xs, ys)
}

// how far a shape keeps from a break: 1/8 inch on paper
fn break_margin(doc: &Document, page: PageRef) -> f64 {
    0.125 / diagram_core::pagesetup::scale(doc, page)
}

/// Seitenumbrüche vermeiden is on (kept in the settings, section [layout])
pub(crate) fn avoid_on(app: &Diagrammer) -> bool {
    if let Some(v) = app.lay.avoid.get() {
        return v;
    }
    let v = app.opts.ini().get("layout", "avoid_breaks").is_some_and(|s| s.trim() == "1");
    app.lay.avoid.set(Some(v));
    v
}

/// Seitenumbrüche vermeiden: on or off
pub(crate) fn toggle_avoid(app: &mut Diagrammer, _ui: &mut Ui) {
    let v = !avoid_on(app);
    app.lay.avoid.set(Some(v));
    app.opts.ini_mut().set("layout", "avoid_breaks", if v { "1" } else { "0" });
    app.opts.save();
}

// the spacing of the adjustments: Abstandsoptionen's, else the page's
pub(crate) fn spacing(app: &Diagrammer, doc: &Document, page: PageRef) -> [f64; 2] {
    app.lay.spacing.map(|s| {
        let k = diagram_core::pagesetup::scale(doc, page);
        [s[0] / k, s[1] / k]
    })
    .unwrap_or_else(|| Opts::from_page(doc, page).spacing)
}

// the shapes moved off the page breaks (inside the caller's transaction) when the toggle is on
fn keep_off_breaks(tx: &mut diagram_core::Transaction, eng: &mut diagram_core::formula::Engine, page: PageRef, shapes: &[ShapeRef]) -> Result<Vec<ShapeRef>, diagram_core::EditError> {
    let owner = Owner::Page(page);
    let (xs, ys) = breaks(tx.doc(), page);
    if (xs.is_empty() && ys.is_empty()) || shapes.is_empty() {
        return Ok(Vec::new());
    }
    let m = break_margin(tx.doc(), page);
    let moves = layout::off_breaks(tx.doc(), owner, shapes, &xs, &ys, m);
    layout::move_nodes(tx, eng, owner, &moves)
}

/// a layout of the page (or of the selected shapes) in `place` with the page's other settings
/// -> laid out; done = false: as the live preview
pub(crate) fn relayout(app: &mut Diagrammer, ui: &mut Ui, place: Place, done: bool) {
    let Some(page) = page(app) else { return };
    let only = chosen(app);
    let avoid = avoid_on(app);
    let Some(o) = app.opened.as_mut() else { return };
    let mut opts = Opts::from_page(&o.doc, page);
    opts.place = place;
    let label = t("Seitenlayout neu anordnen");
    let _ = edit_doc::edit(o, label, done, |tx, eng| {
        let owner = Owner::Page(page);
        if done {
            layout::store(tx, eng, page, &opts, None, None)?;
        }
        let moved = layout::relayout(tx, eng, owner, &only, &opts, Routing::Follow)?;
        finish(tx, eng, page, &opts, &moved, avoid, num(tx.doc(), SheetRef::Page(page), "ResizePage").is_some_and(|v| v != 0.0))
    });
    ui.repass();
}

/// after a layout (inside its transaction): off the page breaks, the page enlarged; the
/// connectors of what moved here follow
pub(crate) fn finish(tx: &mut diagram_core::Transaction, eng: &mut diagram_core::formula::Engine, page: PageRef, opts: &Opts, moved: &[ShapeRef], avoid: bool, enlarge: bool) -> Result<(), diagram_core::EditError> {
    let owner = Owner::Page(page);
    if avoid {
        let nodes: Vec<ShapeRef> = moved.iter().copied().filter(|&s| !glue::is_one_d(tx.doc(), s)).collect();
        let more = keep_off_breaks(tx, eng, page, &nodes)?;
        if !more.is_empty() {
            layout::reroute(tx, eng, owner, &more, Routing::Follow, false)?;
        }
    }
    if enlarge {
        layout::enlarge_page(tx, eng, page, opts.spacing[0].max(opts.spacing[1]))?;
    }
    Ok(())
}

/// Automatische Ausrichtung und Abstände (align and space), Automatischer Abstand (space) on
/// the selection (none: the page); done = false: as the live preview
pub(crate) fn auto(app: &mut Diagrammer, ui: &mut Ui, align: bool, space: bool, done: bool) {
    let Some(page) = page(app) else { return };
    let only = selection(app);
    let avoid = avoid_on(app);
    let gap = match app.opened.as_ref() {
        Some(o) => spacing(app, &o.doc, page),
        None => return,
    };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let moves = layout::align_space(&o.doc, owner, &only, align, space, gap);
    if moves.is_empty() && !avoid {
        return;
    }
    let label = if align { t("Automatische Ausrichtung und Abstände") } else { t("Automatischer Abstand") };
    let _ = edit_doc::edit(o, label, done, |tx, eng| {
        let moved = layout::move_nodes(tx, eng, owner, &moves)?;
        layout::reroute(tx, eng, owner, &moved, Routing::Follow, false)?;
        let opts = Opts::from_page(tx.doc(), page);
        finish(tx, eng, page, &opts, &moved, avoid, false)
    });
    ui.repass();
}

/// Von Seitenumbrüchen verschieben: the selected shapes (none: all) off the page breaks
pub(crate) fn off_breaks(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(page) = page(app) else { return };
    let only = selection(app);
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let (xs, ys) = breaks(&o.doc, page);
    let moves = layout::off_breaks(&o.doc, owner, &only, &xs, &ys, break_margin(&o.doc, page));
    if moves.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, t("Von Seitenumbrüchen verschieben"), true, |tx, eng| {
        let moved = layout::move_nodes(tx, eng, owner, &moves)?;
        layout::reroute(tx, eng, owner, &moved, Routing::Follow, false)
    });
    ui.repass();
}

/// Diagramm drehen: the connected shapes (the selection) turned or flipped, the shapes upright
pub(crate) fn turn(app: &mut Diagrammer, ui: &mut Ui, how: Turn) {
    let Some(page) = page(app) else { return };
    let only = selection(app);
    let avoid = avoid_on(app);
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let gap = Opts::from_page(&o.doc, page).spacing;
    let moves = layout::turn(&o.doc, owner, &only, how, gap);
    if moves.is_empty() {
        return;
    }
    let label = match how {
        Turn::Left => t("Diagramm um 90° nach links drehen"),
        Turn::Right => t("Diagramm um 90° nach rechts drehen"),
        Turn::FlipV => t("Diagramm vertikal spiegeln"),
        Turn::FlipH => t("Diagramm horizontal spiegeln"),
    };
    let _ = edit_doc::edit(o, label, true, |tx, eng| {
        let moved = layout::move_nodes(tx, eng, owner, &moves)?;
        layout::reroute(tx, eng, owner, &moved, Routing::Follow, false)?;
        let opts = Opts::from_page(tx.doc(), page);
        finish(tx, eng, page, &opts, &moved, avoid, false)
    });
    ui.repass();
}

// the page has two or more connected shapes (kept per version of the drawing)
fn connected(app: &Diagrammer) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return false };
    if let Some((serial, rev, p, v)) = app.lay.connected.get() {
        if serial == o.serial && rev == o.ed.rev && p == page {
            return v;
        }
    }
    let v = !layout::diagram(&o.doc, Owner::Page(page), &[]).edges.is_empty();
    app.lay.connected.set(Some((o.serial, o.ed.rev, page, v)));
    v
}

/// whether a command of this file can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let open = page(app).is_some();
    match id {
        id if place_of(id).is_some() => open,
        RELAYOUT | LAYOUT_MORE | SPACING_OPTIONS | AVOID_BREAKS => open,
        AUTO_SPACE | AUTO_ALIGN_SPACE | DIAGRAM_ROTATE_LEFT | DIAGRAM_ROTATE_RIGHT | DIAGRAM_FLIP_V | DIAGRAM_FLIP_H => open && connected(app),
        DISTRIBUTE_MORE => app.se.sel.len() > 2,
        MOVE_OFF_BREAKS => {
            let (Some(o), Some(p)) = (app.opened.as_ref(), app.model.page) else { return false };
            let (xs, ys) = breaks(&o.doc, p);
            !(xs.is_empty() && ys.is_empty())
        }
        _ => true,
    }
}

// run a command with a preview as the preview (done = false) or for real
fn run_as(app: &mut Diagrammer, ui: &mut Ui, id: u32, done: bool) {
    if let Some(p) = place_of(id) {
        relayout(app, ui, p, done);
    } else if id == AUTO_SPACE {
        auto(app, ui, false, true, done);
    } else if id == AUTO_ALIGN_SPACE {
        auto(app, ui, true, true, done);
    }
}

/// a row of the gallery: the layout for real
pub(crate) fn gallery(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    run_as(app, ui, id, true);
}

/// ribbon.rs, every menu row: pointed at, a row with a preview shows it on the drawing (the
/// option Livevorschau); chosen, the preview goes first (the command applies anew)
pub(crate) fn row_event(app: &mut Diagrammer, ui: &mut Ui, id: u32, ev: u32) {
    if !previews(id) {
        return;
    }
    if ev & EV_ACT != 0 {
        if app.lay.preview.take().is_some() {
            if let Some(o) = app.opened.as_mut() {
                edit_doc::cancel(o);
            }
        }
        return;
    }
    if ev & EV_HOVER != 0 && app.opts.get(&crate::options::LIVE) && crate::commands::usable(app, id) {
        app.lay.hovered = true;
        if app.lay.preview != Some(id) {
            app.lay.preview = Some(id);
            run_as(app, ui, id, false);
        }
    }
}

/// every pass after the ribbon: the preview goes when its row is no longer pointed at
pub(crate) fn after(app: &mut Diagrammer, ui: &mut Ui) {
    if !app.lay.hovered && app.lay.preview.take().is_some() {
        if let Some(o) = app.opened.as_mut() {
            edit_doc::cancel(o);
        }
        ui.repass();
    }
    app.lay.hovered = false;
}
