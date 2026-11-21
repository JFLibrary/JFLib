// cmds_myshapes -- (JD8 rest) Zu meinen Shapes hinzufügen from the shape's context menu (SL-07,
// ZF-12): the selected shapes become a master of the favourites (the tiles' own menu adds a
// stencil's master, stencils.rs)
//
//   add(app, ui)          the selection as one master at the end of Favoriten, saved; the
//                         panel shows the favourites
//   available(app)        shapes are selected
//
// How: a master needs the drawing's styles, theme and masters its shapes use; instance::
// import_master carries all that from one document into another.  So the shapes first become a
// master of a copy of the drawing (ops::copy / paste into the new master, several shapes grouped,
// moved so that their box starts at the master page's origin, the master page as large as the
// box), and that master is imported into the favourites.  The drawing itself does not change.
// The master's name is the shape's name (its text when it has no name of its own), else Shape.

use alloc::string::String;

use diagram_core::formula::Engine;
use diagram_core::{instance, ops, CellAddr, Document, EditError, MasterRef, Owner, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::{edit_canvas, stencils, Diagrammer};

/// shapes are selected
pub(crate) fn available(app: &Diagrammer) -> bool {
    app.opened.is_some() && !app.se.sel.is_empty()
}

// the master's name: the shape's own name without ".ID", else its text's first line
fn name_for(doc: &Document, s: ShapeRef) -> (String, String) {
    let sh = doc.shape(s);
    let strip = |n: &str| -> String {
        match n.rsplit_once('.') {
            Some((stem, id)) if !stem.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => String::from(stem),
            _ => String::from(n),
        }
    };
    let name = sh.and_then(|x| x.header().name).map(|n| strip(doc.str(n)));
    let name_u = sh.and_then(|x| x.header().name_u).map(|n| strip(doc.str(n)));
    let text = sh.and_then(|x| x.text()).map(|t| t.chars()).map(|c| String::from(c.lines().next().unwrap_or("").trim()));
    let generic = |n: &str| n.starts_with("Sheet") || n.is_empty();
    let shown = match (&name, &text) {
        (Some(n), _) if !generic(n) => n.clone(),
        (_, Some(tx)) if !tx.is_empty() => tx.chars().take(40).collect(),
        _ => String::from(t("Shape")),
    };
    let univ = match name_u {
        Some(n) if !generic(&n) => n,
        _ => String::from("Shape"),
    };
    (shown, univ)
}

// the shapes as a master of `tmp` (a copy of the drawing) -> the master
fn make_master(tmp: &mut Document, src: &Document, shapes: &[ShapeRef], name: &str, name_u: &str) -> Result<MasterRef, EditError> {
    let mut eng = Engine::new();
    let mut tx = tmp.transaction("master");
    let m = tx.add_master(name, name_u)?;
    let clip = ops::copy(src, shapes);
    let mut made = ops::paste(&mut tx, &mut eng, Owner::Master(m), &clip, [0.0, 0.0])?;
    if made.len() > 1 {
        made = alloc::vec![ops::group(&mut tx, &mut eng, Owner::Master(m), &made)?];
    }
    let b = ops::bounds(tx.doc(), &made).ok_or(EditError::NoShape)?;
    ops::move_shapes(&mut tx, &mut eng, &made, [-b[0], -b[1]])?;
    let sheet = SheetRef::Master(m);
    for (cell, v) in [("PageWidth", b[2] - b[0]), ("PageHeight", b[3] - b[1])] {
        let a: CellAddr = tx.single(cell);
        tx.set_constant(sheet, &a, Value::Num(v.max(0.01)), Some(Unit::Inches))?;
    }
    tx.commit();
    Ok(m)
}

/// Zu meinen Shapes hinzufügen
pub(crate) fn add(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = edit_canvas::selected(app);
    let Some(o) = app.opened.as_ref() else { return };
    let Some(&first) = shapes.first() else { return };
    let (name, name_u) = name_for(&o.doc, first);
    let mut tmp = o.doc.clone();
    let m = match make_master(&mut tmp, &o.doc, &shapes, &name, &name_u) {
        Ok(m) => m,
        Err(_) => return,
    };
    let fav = stencils::favorites(app, ui);
    let Some(l) = app.stencils.loaded.iter_mut().find(|l| l.key == fav) else { return };
    let mut tx = l.doc.transaction(t("Zu meinen Shapes hinzufügen"));
    let ok = instance::import_master(&mut tx, &tmp, m).is_ok();
    if ok {
        tx.commit();
        l.rebuild_entries();
        l.dirty = true;
    } else {
        return;
    }
    stencils::save(app, ui, fav);
    app.stencils.show(fav);
    app.panes.show(crate::panes::P_SHAPES);
    ui.repass();
}
