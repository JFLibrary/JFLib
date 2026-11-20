// stencil_overview -- (JD13) every master of one of JFDiagrammer's stencils on one sheet: for
// looking at a whole stencil while its masters are drawn and before the user's approval
//
//   diagramiotest --write-stencil-previews DIR   writes DIR/<folder>-alle.png for every stencil
//                                                folder (besides the sheets of stencil_structs.rs
//                                                and stencil_shapes.rs)
//
// Six columns of 62 mm; each master at its own size (shrunk to fit its cell when larger), its
// German name under it, hidden masters last with "(verborgen)".  The stencils come from the
// table build.rs makes (templates::SOURCES), so a new folder or master needs no line here.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::instance::{MasterTexts, StencilTexts};
use diagram_core::{CellAddr, Document, MasterRef, Owner, ShapeRef, SheetRef};
use diagram_io::stencil;
use diagram_io::vsdx::Measure;

use crate::stencil_structs::{none, put, render_page_at, sheet_doc};
use crate::templates::{files_of, SOURCES};
use crate::out;

const COL: f64 = 62.0;
const ROW: f64 = 58.0;
const COLS: usize = 6;
const MM: f64 = 1.0 / 25.4;

// a master's size in mm (its master shape's Width and Height)
fn size_of(doc: &Document, m: MasterRef) -> (f64, f64) {
    let Some(slot) = doc.master(m).and_then(|x| x.master_shape()) else { return (20.0, 20.0) };
    let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
    let num = |n: &str| -> f64 {
        let Some(sym) = doc.sym(n) else { return 0.0 };
        let r = doc.resolve(sheet, &CellAddr::single(sym));
        r.cached.or(r.cell).and_then(|c| c.value.num()).unwrap_or(0.0) / MM
    };
    (num("Width"), num("Height"))
}

/// DIR/<folder>-alle.png for every stencil folder -> 0 when all were written
pub fn write_all(dir: &str) -> i32 {
    let mut folders: Vec<&'static str> = SOURCES.iter().filter(|s| s.0 == "stencils").map(|s| s.1).collect();
    folders.dedup();
    let mut ok = true;
    for f in folders {
        ok &= write(dir, f);
    }
    if ok { 0 } else { 1 }
}

fn write(dir: &str, folder: &'static str) -> bool {
    let files = files_of("stencils", folder);
    if files("stencil.txt").is_none() {
        return true;
    }
    let st = match stencil::build(&files, Measure::Metric) {
        Ok(b) => b.doc,
        Err(e) => {
            out(&format!("error: stencils/{folder}/{e}\n"));
            return false;
        }
    };
    let Some(standard) = stencil::build(&files_of("stencils", "standard"), Measure::Metric).ok().map(|b| b.doc) else { return false };
    let hidden = st.sym("Hidden");
    let is_hidden = |m: MasterRef| st.master(m).and_then(|x| hidden.and_then(|s| diagram_core::attr(&x.header.attrs, s))).is_some_and(|v| st.str(v) == "1");
    let mut masters: Vec<MasterRef> = st.masters().filter(|&m| !is_hidden(m)).collect();
    masters.extend(st.masters().filter(|&m| is_hidden(m)));
    if masters.is_empty() {
        return true;
    }
    let rows = masters.len().div_ceil(COLS).max(1);
    let (pw, ph) = (COLS as f64 * COL + 16.0, rows as f64 * ROW + 22.0);
    let Some((mut doc, page)) = sheet_doc(pw, ph) else { return false };
    {
        let mut tx = doc.transaction("overview");
        let title = format!("{} ({folder}): {} Master", StencilTexts::of(&st).name(0), masters.len());
        put(&mut tx, &standard, page, "Text box", (pw * 0.5, ph - 7.0), (pw - 16.0, 8.0), Some(&title), &none);
        for (k, &m) in masters.iter().enumerate() {
            let (c, r) = (k % COLS, k / COLS);
            let cx = 8.0 + COL * (c as f64 + 0.5);
            let top = ph - 18.0 - ROW * r as f64;
            let (w, h) = size_of(&st, m);
            // larger than its cell: shrunk, the aspect kept
            let f = (52.0 / w.max(1e-3)).min(38.0 / h.max(1e-3)).min(1.0);
            let size = if f < 1.0 { (w * f, h * f) } else { (0.0, 0.0) };
            let texts = MasterTexts::of(&st, m);
            let one_d = size_of(&st, m).1 < 1e-6;
            let at = (cx, top - 22.0);
            let size = if one_d { (0.0, 0.0) } else { size };
            put(&mut tx, &st, page, &texts.name_u, at, size, None, &none);
            let mut name = String::from(texts.name(0));
            if is_hidden(m) {
                name.push_str(" (verborgen)");
            }
            put(&mut tx, &standard, page, "Text box", (cx, top - 48.0), (COL - 4.0, 8.0), Some(&name), &none);
        }
        let _ = tx.commit();
    }
    render_page_at(&mut doc, page, &format!("{dir}/{folder}-alle.png"), 110.0)
}
