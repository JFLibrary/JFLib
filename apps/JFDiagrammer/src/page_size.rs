// page_size -- (JD8 rest) Entwurf › Seite einrichten: Ausrichtung (Hochformat, Querformat),
// Größe (paper sizes with their measures in the region's unit, Größe an Zeichnung anpassen,
// Weitere Seitenformate), Automatisch anpassen; Zeichnungsmaßstab; the automatic size after a
// gesture (FEATURES RW-01 .. RW-03, PG-04, PG-05, PR-07)
//
//   SIZES                     the paper sizes of the menu (commands SIZE_*, 0x1800 ..)
//   orientation(app, ui, landscape)   Hochformat / Querformat of the page shown
//   size(app, ui, id)         a size of the menu for the page shown
//   size_note(id)             the measures a size row shows ("21 × 29,7 cm", "8.5 × 11 in")
//   fit_drawing(app, ui)      Größe an Zeichnung anpassen
//   auto_size(app, ui)        Automatisch anpassen on / off
//   checked(app, id)          the orientation, the size and the automatic size of the page shown
//   after_gesture(tx, eng)    edit_doc::edit after a finished gesture: each page the gesture
//                             changed whose automatic size is on grows or shrinks in tiles of
//                             its printed sheet -> where the view must move (pages::sync)
//
// The model's side is diagram_core::pagesetup (the cells, Visio's meanings and the tiles are
// described and tested there).  What the app adds:
// * The orientation also turns the print setup's paper (Datei › Drucken shows it), as Visio's
//   Ausrichtung sets PrintPageOrientation.
// * A size of the menu is the drawing page's; the printer paper stays (Visio's Page Setup:
//   the page size "does not directly affect the printed page", Step by Step, chapter 3).
//   Annahme: the size keeps the page's orientation (the menu lists portrait sizes).
// * Größe an Zeichnung anpassen leaves the page's print margins (Page*Margin, 0.25 in by
//   default) around the drawing -- Annahme: Visio's margin is not documented.
// * Automatisch anpassen turned on fits the page to its drawing in tiles at once (Annahme);
//   afterwards every finished gesture does (never a file being opened, PLAN.md "Fallen").
// * The measures follow the region (File › Optionen › Sprache): inches for en-US, centimetres
//   otherwise, as Visio's menu shows "21 cm x 29,7 cm".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::{pagesetup, EditError, Owner, PageRef, SheetRef, Transaction};
use ui::i18n;
use ui::*;

use crate::commands::{self, *};
use crate::print_layout::{Fit, PrintSetup};
use crate::viewer::DIP;
use crate::Diagrammer;

/// the paper sizes of Entwurf › Größe: command, width and height in mm (portrait)
pub(crate) static SIZES: [(u32, f64, f64); 9] = [
    (SIZE_LETTER, 215.9, 279.4),
    (SIZE_LEGAL, 215.9, 355.6),
    (SIZE_TABLOID, 279.4, 431.8),
    (SIZE_EXECUTIVE, 184.15, 266.7),
    (SIZE_A5, 148.0, 210.0),
    (SIZE_A4, 210.0, 297.0),
    (SIZE_A3, 297.0, 420.0),
    (SIZE_B5, 182.0, 257.0),
    (SIZE_B4, 257.0, 364.0),
];

fn mm(v: f64) -> f64 {
    v / 25.4
}

/// whether the command is one of this module's
pub(crate) fn owns(id: u32) -> bool {
    matches!(id, PORTRAIT | LANDSCAPE | FIT_DRAWING | MORE_SIZES | AUTO_SIZE | DRAWING_SCALE) || SIZES.iter().any(|s| s.0 == id)
}

/// whether the command can run now (a drawing is open)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    !owns(id) || crate::pages::shown(app).is_some()
}

// the page shown and the drawing
fn target(app: &mut Diagrammer) -> Option<(PageRef, &mut crate::viewer::Opened)> {
    let p = crate::pages::shown(app)?;
    Some((p, app.opened.as_mut()?))
}

/// Hochformat (landscape = false) / Querformat
pub(crate) fn orientation(app: &mut Diagrammer, ui: &mut Ui, landscape: bool) {
    let Some((page, o)) = target(app) else { return };
    let label = commands::label(if landscape { LANDSCAPE } else { PORTRAIT });
    let r = crate::edit_doc::edit(o, label, true, |tx, eng| pagesetup::set_orientation(tx, eng, page, landscape));
    if r == Ok(true) {
        app.print.setup.landscape = landscape;
        refit(app);
    }
    ui.repass();
}

/// a size of the menu
pub(crate) fn size(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some(&(_, w, h)) = SIZES.iter().find(|s| s.0 == id) else { return };
    let Some((page, o)) = target(app) else { return };
    let r = crate::edit_doc::edit(o, commands::label(PAGE_SIZE), true, |tx, eng| pagesetup::set_paper_size(tx, eng, page, [mm(w), mm(h)]));
    if r == Ok(true) {
        refit(app);
    }
    ui.repass();
}

/// Größe an Zeichnung anpassen: the page around its shapes with the print margins
pub(crate) fn fit_drawing(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((page, o)) = target(app) else { return };
    let margins = PrintSetup::from_page(&o.doc, page).margins;
    let r = crate::edit_doc::edit(o, commands::label(FIT_DRAWING), true, |tx, eng| pagesetup::fit_to_drawing(tx, eng, page, margins));
    if r == Ok(true) {
        refit(app);
    }
    ui.repass();
}

/// Automatisch anpassen: on (the page fitted in tiles at once) or off
pub(crate) fn auto_size(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((page, o)) = target(app) else { return };
    let on = !pagesetup::auto_size(&o.doc, page);
    let r = crate::edit_doc::edit(o, commands::label(AUTO_SIZE), true, |tx, eng| {
        pagesetup::set_auto_size(tx, page, on)?;
        if on {
            let t = tiles(tx.doc(), page);
            pagesetup::grow(tx, eng, page, &t)?;
        }
        Ok(())
    });
    if r.is_ok() {
        refit(app);
    }
    ui.repass();
}

// a page of another size: fit it into the window
fn refit(app: &mut Diagrammer) {
    app.canvas.request_fit();
    app.fit_again = true;
}

/// toggles and checks: the orientation, the size and the automatic size of the page shown
pub(crate) fn checked(app: &Diagrammer, id: u32) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), crate::pages::shown(app)) else { return false };
    let doc = &o.doc;
    match id {
        PORTRAIT => !pagesetup::landscape(doc, page),
        LANDSCAPE => pagesetup::landscape(doc, page),
        AUTO_SIZE => pagesetup::auto_size(doc, page),
        _ => match SIZES.iter().find(|s| s.0 == id) {
            Some(&(_, w, h)) => {
                let s = pagesetup::scale(doc, page);
                let [pw, ph] = pagesetup::size(doc, page);
                let (a, b) = (pw * s, ph * s);
                let near = |x: f64, y: f64| (x - y).abs() < mm(0.5);
                (near(a, mm(w)) && near(b, mm(h))) || (near(a, mm(h)) && near(b, mm(w)))
            }
            None => false,
        },
    }
}

/// the measures of a size row in the region's unit (None: not a size)
pub(crate) fn size_note(id: u32) -> Option<String> {
    let &(_, w, h) = SIZES.iter().find(|s| s.0 == id)?;
    let loc = i18n::locale();
    let dec = loc.decimal();
    // up to two decimals, no trailing zeros, the region's decimal sign
    let fmt = |v: f64| {
        let s = format!("{:.2}", v);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        String::from(s).replace('.', dec.encode_utf8(&mut [0; 4]))
    };
    Some(if loc.tag() == "en-US" { format!("{} × {} in", fmt(mm(w)), fmt(mm(h))) } else { format!("{} × {} cm", fmt(w / 10.0), fmt(h / 10.0)) })
}

/// the tiles of a page's automatic size: its paper in the print orientation (the page's own
/// when the print setup says "same as printer"), the margins, the print zoom
pub(crate) fn tiles(doc: &diagram_core::Document, page: PageRef) -> pagesetup::Tiles {
    let mut setup = PrintSetup::from_page(doc, page);
    let po = doc.sym("PrintPageOrientation").and_then(|c| doc.resolve(SheetRef::Page(page), &diagram_core::CellAddr::single(c)).num());
    if !matches!(po.map(|v| v as i64), Some(1) | Some(2)) {
        setup.landscape = pagesetup::landscape(doc, page);
    }
    let (w, h) = setup.paper_size();
    let zoom = match setup.fit {
        Fit::Percent(z) => z,
        Fit::Sheets(..) => 1.0,
    };
    let m = setup.margins; // left, top, right, bottom
    pagesetup::Tiles::new([w, h], m, zoom)
}

/// after a finished gesture (edit_doc::edit, inside its transaction): the automatic size of each
/// page whose shapes it changed -> (page, how far the drawing moved in world units) for the view
pub(crate) fn after_gesture(tx: &mut Transaction, eng: &mut Engine) -> Result<Vec<(PageRef, [f32; 2])>, EditError> {
    let mut pages: Vec<PageRef> = Vec::new();
    for s in tx.sheets() {
        if let SheetRef::Shape(r) = s {
            if let Owner::Page(p) = r.owner {
                if !pages.contains(&p) {
                    pages.push(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    for p in pages {
        if !pagesetup::auto_size(tx.doc(), p) {
            continue;
        }
        let t = tiles(tx.doc(), p);
        let h0 = pagesetup::size(tx.doc(), p)[1];
        if let Some(d) = pagesetup::grow(tx, eng, p, &t)? {
            let s = pagesetup::scale(tx.doc(), p);
            let h1 = pagesetup::size(tx.doc(), p)[1];
            // world x of the drawing moves by the shift, world y (down from the page's top) by
            // what the page grew at its top
            out.push((p, [(d[0] * s * DIP) as f32, ((h1 - h0 - d[1]) * s * DIP) as f32]));
        }
    }
    Ok(out)
}
