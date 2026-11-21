// symbol_sheet -- (JD13) every SVG of a folder on one sheet, drawn the way JFDiagrammer draws
// stencil masters (diagram_io::stencil + diagram_view): a look at the shared network symbols
// (apps/JFDiagrammer/symbols) while they are drawn
//
//   diagramiotest --symbol-sheet DIR OUT.png [--cols N]
//
// Each SVG becomes a master of a stencil built in memory: a symbol of the 64 x 48 grid is
// 24 x 18 mm, anything else (rack fronts in mm) is drawn at 1:20 on paper.  The masters are
// placed in a grid with their file name below.  A file the stencil tool refuses stops the run
// with file, line and reason.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::instance::{self, MasterTexts};
use diagram_core::render::Options;
use diagram_core::{CellAddr, DocKind, SheetRef, Text, Unit, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::out;

const MM: f64 = 1.0 / 25.4;

// the viewBox of an SVG text (width, height)
fn view_box(t: &str) -> Option<(f64, f64)> {
    let i = t.find("viewBox=\"")? + 9;
    let rest = &t[i..];
    let v: Vec<f64> = rest[..rest.find('"')?].split([' ', ',']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    (v.len() == 4 && v[2] > 0.0 && v[3] > 0.0).then(|| (v[2], v[3]))
}

/// diagramiotest --symbol-sheet DIR OUT.png [--cols N]
pub fn write(dir: &str, path: &str, cols: usize) -> i32 {
    let Some(entries) = plat::fs::read_dir(dir) else {
        out(&format!("error: cannot read {dir}\n"));
        return 1;
    };
    let mut names: Vec<String> = entries.into_iter().map(|e| e.name).filter(|n| n.ends_with(".svg")).collect();
    names.sort();
    if names.is_empty() {
        out(&format!("error: no SVG in {dir}\n"));
        return 1;
    }
    let mut files: Vec<(String, String)> = Vec::new();
    let mut sizes: Vec<(f64, f64)> = Vec::new();
    let mut stems: Vec<String> = Vec::new();
    for (i, n) in names.iter().enumerate() {
        let Some(bytes) = plat::read_file(&format!("{dir}/{n}")) else {
            out(&format!("error: cannot read {dir}/{n}\n"));
            return 1;
        };
        let t = String::from_utf8_lossy(&bytes).into_owned();
        let Some((vw, vh)) = view_box(&t) else {
            out(&format!("error: {n}: no viewBox\n"));
            return 1;
        };
        // the 64 x 48 grid: 24 x 18 mm; else millimetres at 1:20
        let (w, h) = if (vw - 64.0).abs() < 1e-9 && (vh - 48.0).abs() < 1e-9 { (24.0, 18.0) } else { (vw / 20.0, vh / 20.0) };
        let stem = String::from(n.trim_end_matches(".svg"));
        let master = format!(
            "master: S{i}\nname.de: {stem}\nname.en: {stem}\nname.es: {stem}\nname.it: {stem}\nname.fr: {stem}\nprompt.de: -\nprompt.en: -\nprompt.es: -\nprompt.it: -\nprompt.fr: -\nsvg: {n}\n\n[Shape]\nWidth = {w} mm\nHeight = {h} mm\n"
        );
        files.push((format!("s{i}.master"), master));
        files.push((n.clone(), t));
        sizes.push((w, h));
        stems.push(stem);
    }
    let list: Vec<String> = (0..names.len()).map(|i| format!("s{i}")).collect();
    files.push((String::from("stencil.txt"), format!("stencil: Sheet\nfile: sheet\nname.de: S\nname.en: S\nname.es: S\nname.it: S\nname.fr: S\ncategory: network\nmasters: {}\n", list.join(", "))));
    let lookup = |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone());
    let built = match diagram_io::stencil::build(&lookup, Measure::Metric) {
        Ok(b) => b.doc,
        Err(e) => {
            out(&format!("error: {e}\n"));
            return 1;
        }
    };
    // the grid: cells 40 mm wide, as high as the tallest symbol + 14 mm for the name
    let cols = cols.max(1);
    let rows = names.len().div_ceil(cols);
    let cell_w = 40.0_f64.max(sizes.iter().map(|s| s.0).fold(0.0, f64::max) + 8.0);
    let cell_h = sizes.iter().map(|s| s.1).fold(0.0, f64::max) + 14.0;
    let (pw, ph) = (cols as f64 * cell_w + 10.0, rows as f64 * cell_h + 10.0);
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let Some(page) = doc.pages().next() else { return 1 };
    {
        let mut tx = doc.transaction("sheet");
        for (n, v) in [("PageWidth", pw), ("PageHeight", ph)] {
            let a = CellAddr::single(tx.intern(n));
            let _ = tx.set_constant(SheetRef::Page(page), &a, Value::Num(v * MM), Some(Unit::Millimeters));
        }
        for (i, stem) in stems.iter().enumerate() {
            let Some(m) = built.masters().find(|&m| MasterTexts::of(&built, m).name_u == format!("S{i}")) else { continue };
            let Ok(local) = instance::import_master(&mut tx, &built, m) else { continue };
            let (c, r) = (i % cols, i / cols);
            let x = 5.0 + (c as f64 + 0.5) * cell_w;
            let y = ph - 5.0 - r as f64 * cell_h - (cell_h - 14.0) * 0.5 - 2.0;
            let Ok(s) = instance::place(&mut tx, page, local, x * MM, y * MM, 0) else { continue };
            // the name below the symbol
            let _ = tx.set_text(s, Some(Text::plain(stem)));
            let h = sizes[i].1;
            for (n, v) in [("TxtPinY", -1.0), ("TxtLocPinY", 5.0), ("TxtHeight", 5.0), ("TxtWidth", cell_w - 2.0)] {
                let a = CellAddr::single(tx.intern(n));
                let _ = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v * MM), Some(Unit::Millimeters));
            }
            let _ = h;
        }
        // nothing to compute: the instances inherit the values the build computed for the masters
        // (as the Shapes panel's previews in JFDiagrammer do)
        let _ = tx.commit();
    }
    let mut fonts = crate::render::fonts();
    let mut v = PageView::new(&mut doc, page, &mut fonts, &Options::default());
    let ro = raster::RasterOptions { dpi: 200.0, ..Default::default() };
    let Some(img) = raster::render(&mut v, &ro) else {
        out("error: cannot render\n");
        return 1;
    };
    let ok = plat::write_file(path, &crate::render::png(&img));
    out(&format!("{} {path} ({} symbols, {} x {})\n", if ok { "wrote" } else { "error: cannot write" }, names.len(), img.w, img.h));
    if ok { 0 } else { 1 }
}
