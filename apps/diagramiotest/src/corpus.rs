// corpus -- the local load tests: Zeichnung2.vsdx, Zeichnung 3.vsdx and Favoriten.vssx from
// apps/JFDiagrammer/context/files (gitignored, Microsoft content: read here, never copied, only
// numbers printed).  Skipped when the folder is not there.  The counts are the findings of
// FEATURES.md 1.6 (pages, masters, shapes by kind, sections by name, rows, cells, connects,
// themes, pictures, styles, texts) and of the analysis behind it (element counts of the three
// files together).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{Document, Formula, Sheet, ShapeKind, TextItem};
use diagram_io::vsdx::{self, Limits};

use crate::{out, H};

const FILES: [&str; 3] = ["Zeichnung2.vsdx", "Zeichnung 3.vsdx", "Favoriten.vssx"];

// the folder: --corpus DIR, else the first apps/JFDiagrammer/context/files above the current folder
// ((JD21) route.rs reads the connectors' cells there too)
pub(crate) fn folder(arg: Option<String>) -> Option<String> {
    if let Some(d) = arg {
        return Some(d);
    }
    let mut dir = plat::cwd().replace('\\', "/");
    for _ in 0..12 {
        let cand = format!("{dir}/apps/JFDiagrammer/context/files");
        if plat::fs::read_dir(&cand).is_some() {
            return Some(cand);
        }
        match dir.rsplit_once('/') {
            Some((up, _)) if !up.is_empty() => dir = String::from(up),
            _ => break,
        }
    }
    None
}

/// the numbers of one document
#[derive(Default, Debug, Clone)]
struct Counts {
    pages: usize,
    page_parts: usize,
    masters: usize,
    icons: usize,
    shapes: usize,
    kinds: BTreeMap<String, usize>,
    sections: BTreeMap<String, usize>,
    rows: usize,
    del_rows: usize,
    cells: usize,
    connects: usize,
    themes: usize,
    images: usize,
    foreign: usize,
    texts: usize,
    text_marks: usize,
    styles: usize,
    triggers: usize,
    refby: usize,
    colors: usize,
    faces: usize,
    windows: usize,
    stencil_windows: usize,
    geometry_rows: BTreeSet<String>,
    units: BTreeSet<String>,
    functions: BTreeSet<String>,
    warnings: usize,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        macro_rules! sum { ($($f:ident),*) => { $(self.$f += o.$f;)* } }
        sum!(pages, page_parts, masters, icons, shapes, rows, del_rows, cells, connects, themes, images, foreign, texts, text_marks, styles, triggers, refby, colors, faces, windows, stencil_windows, warnings);
        for (k, v) in &o.kinds {
            *self.kinds.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &o.sections {
            *self.sections.entry(k.clone()).or_default() += v;
        }
        self.geometry_rows.extend(o.geometry_rows.iter().cloned());
        self.units.extend(o.units.iter().cloned());
        self.functions.extend(o.functions.iter().cloned());
    }
}

// the function names of a formula: upper-case names right before '(' (in string literals too:
// SETF and friends evaluate them; FEATURES.md 1.6 counted them so, 59 names)
fn functions(f: &str, out: &mut BTreeSet<String>) {
    let b = f.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() && (i == 0 || !b[i - 1].is_ascii_uppercase() && !b[i - 1].is_ascii_digit()) {
            let s = i;
            while i < b.len() && (b[i].is_ascii_uppercase() || b[i].is_ascii_digit()) {
                i += 1;
            }
            if b.get(i) == Some(&b'(') {
                out.insert(String::from(&f[s..i]));
            }
            continue;
        }
        i += 1;
    }
}

// the RefBy elements kept in extras
fn refby(doc: &Document, e: &diagram_core::Extras) -> usize {
    e.items().iter().filter(|e| matches!(e.item, diagram_core::ExtraItem::Element(x) if doc.str(x).starts_with("<RefBy"))).count()
}

fn sheet(doc: &Document, sh: &Sheet, c: &mut Counts) {
    let cell = |x: &diagram_core::Cell, c: &mut Counts| {
        c.cells += 1;
        if let Some(u) = x.unit {
            c.units.insert(String::from(u.code().unwrap_or(match u {
                diagram_core::Unit::Other(s) => doc.str(s),
                _ => "?",
            })));
        }
        if let Formula::Expr(f) = x.formula {
            functions(doc.str(f), &mut c.functions);
        }
    };
    for x in &sh.cells {
        cell(x, c);
    }
    c.triggers += sh.triggers.len();
    c.refby += sh.triggers.iter().map(|t| doc.str(t.inner).matches("<RefBy").count()).sum::<usize>() + refby(doc, &sh.extras);
    for s in &sh.sections {
        c.refby += refby(doc, &s.extras);
        let name = doc.str(s.name);
        *c.sections.entry(String::from(name)).or_default() += 1;
        for x in &s.cells {
            cell(x, c);
        }
        for r in &s.rows {
            c.rows += 1;
            c.del_rows += r.del as usize;
            if name == "Geometry" {
                if let Some(k) = r.kind {
                    c.geometry_rows.insert(String::from(doc.str(k)));
                }
            }
            for x in &r.cells {
                cell(x, c);
            }
            c.refby += refby(doc, &r.extras);
        }
    }
}

fn count(l: &vsdx::Loaded) -> Counts {
    let doc = &l.doc;
    let mut c = Counts { pages: doc.page_count(), masters: doc.masters().count(), themes: doc.themes.len(), styles: doc.styles().count(), colors: doc.colors.len(), faces: doc.faces.len(), warnings: l.warnings.len() + l.more_warnings, ..Counts::default() };
    c.images = doc.parts.iter().filter(|p| doc.str(p.content_type).starts_with("image/png")).count();
    for p in &doc.parts {
        if doc.str(p.name).ends_with("/windows.xml") {
            let t = String::from_utf8_lossy(&p.data);
            c.windows = t.matches("<Window ").count();
            c.stencil_windows = t.matches("WindowType='Stencil'").count() + t.matches("WindowType=\"Stencil\"").count();
        }
    }
    sheet(doc, &doc.doc_sheet().sheet, &mut c);
    for s in doc.styles() {
        if let Some(st) = doc.style(s) {
            c.refby += refby(doc, &st.extras);
            sheet(doc, &st.sheet, &mut c);
        }
    }
    let mut trees = Vec::new();
    for m in doc.masters() {
        let Some(ms) = doc.master(m) else { continue };
        c.icons += ms.icon.is_some() as usize;
        c.connects += ms.connects.len();
        sheet(doc, &ms.sheet, &mut c);
        trees.push(&ms.shapes);
    }
    for p in doc.pages() {
        let Some(pg) = doc.page(p) else { continue };
        c.page_parts += pg.part.is_some() as usize;
        c.connects += pg.connects.len();
        sheet(doc, &pg.sheet, &mut c);
        trees.push(&pg.shapes);
    }
    for t in trees {
        for slot in t.iter() {
            let Some(d) = t.get(slot) else { continue };
            c.shapes += 1;
            let k = match d.header.kind {
                ShapeKind::Shape => "Shape",
                ShapeKind::Group => "Group",
                ShapeKind::Foreign => "Foreign",
                ShapeKind::Guide => "Guide",
                ShapeKind::Unspecified => "-",
                ShapeKind::Other(_) => "other",
            };
            *c.kinds.entry(String::from(k)).or_default() += 1;
            c.foreign += d.foreign.is_some() as usize;
            if let Some(tx) = &d.text {
                c.texts += 1;
                c.text_marks += tx.items.iter().filter(|i| matches!(i, TextItem::Cp(_) | TextItem::Pp(_) | TextItem::Tp(_))).count();
            }
            c.refby += refby(doc, &d.extras);
            sheet(doc, &d.sheet, &mut c);
        }
    }
    c
}

pub fn run(h: &mut H, arg: Option<String>) {
    let Some(dir) = folder(arg) else {
        out("skip corpus: no apps/JFDiagrammer/context/files above the current folder (--corpus DIR)\n");
        return;
    };
    let mut total = Counts::default();
    let mut per = Vec::new();
    for f in FILES {
        let Some(bytes) = plat::read_file(&format!("{dir}/{f}")) else {
            out(&format!("skip corpus: {f} is missing\n"));
            return;
        };
        let t0 = plat::time_us();
        let r = vsdx::read(&bytes, &Limits::VSDX);
        let ms = plat::time_us().wrapping_sub(t0) / 1000;
        match r {
            Ok(l) => {
                let c = count(&l);
                h.check("corpus/load", c.warnings == 0, format!("{f}: {} KB in {ms} ms, {} warnings", bytes.len() / 1024, c.warnings));
                total.add(&c);
                per.push(c);
            }
            Err(e) => {
                h.check("corpus/load", false, format!("{f}: {e}"));
                return;
            }
        }
    }
    let (z2, z3, fav) = (&per[0], &per[1], &per[2]);
    let row = |c: &Counts| (c.pages, c.page_parts, c.masters, c.themes, c.images, c.connects, c.styles, c.colors, c.windows, c.stencil_windows);
    out(&format!("     (pages, page parts, masters, themes, PNG, connects, styles, colours, windows, stencil windows)\n     Z2 {:?}\n     Z3 {:?}\n     Fav {:?}\n", row(z2), row(z3), row(fav)));
    // FEATURES.md 1.6 per file
    h.check("corpus/files", row(z2) == (1, 1, 15, 1, 3, 34, 8, 89, 8, 7) && row(z3) == (5, 5, 30, 3, 0, 132, 11, 59, 13, 12) && row(fav) == (1, 0, 29, 0, 0, 0, 6, 13, 0, 0),
        String::from("pages, masters, themes, PNG, connects, styles, colours, windows per file"));
    let t = &total;
    out(&format!(
        "     all: {} shapes {:?}, {} sections, {} rows ({} Del), {} cells, {} connects, {} texts ({} marks), {} pictures, {} icons, {} triggers, {} RefBy, {} fonts\n",
        t.shapes, t.kinds, t.sections.values().sum::<usize>(), t.rows, t.del_rows, t.cells, t.connects, t.texts, t.text_marks, t.foreign, t.icons, t.triggers, t.refby, t.faces
    ));
    out(&format!("     sections {:?}\n", t.sections));
    out(&format!("     {} geometry row kinds, {} units, {} functions\n", t.geometry_rows.len(), t.units.len(), t.functions.len()));
    let kinds = |k: &str| t.kinds.get(k).copied().unwrap_or(0);
    h.check("corpus/shapes", t.shapes == 1043 && kinds("Shape") == 718 && kinds("Group") == 303 && kinds("Foreign") == 3 && kinds("-") == 19,
        format!("{} shapes: 718 simple, 303 groups, 3 foreign, 19 deleted without Type", t.shapes));
    let want: [(&str, usize); 13] = [
        ("Actions", 4), ("Character", 325), ("Connection", 234), ("Control", 314), ("FillGradient", 11), ("Geometry", 2115), ("Layer", 12),
        ("LineGradient", 13), ("Paragraph", 29), ("Property", 60), ("Scratch", 3), ("Tabs", 3), ("User", 434),
    ];
    let secs_ok = t.sections.len() == 13 && want.iter().all(|(n, v)| t.sections.get(*n) == Some(v));
    h.check("corpus/sections", secs_ok && t.sections.values().sum::<usize>() == 3557, String::from("13 section kinds, 3557 sections"));
    h.check("corpus/rows-cells", t.rows == 12786 && t.del_rows == 117 && t.cells == 64762, format!("{} rows, {} cells", t.rows, t.cells));
    h.check("corpus/connects", t.connects == 166, format!("{}", t.connects));
    h.check("corpus/pages-masters", t.pages == 7 && t.page_parts == 6 && t.masters == 74 && t.icons == 74, format!("{} pages, {} masters", t.pages, t.masters));
    h.check("corpus/text-styles", t.texts == 175 && t.text_marks == 225 && t.styles == 25 && t.colors == 161 && t.faces == 6, format!("{} texts, {} marks", t.texts, t.text_marks));
    h.check("corpus/kept", t.foreign == 3 && t.triggers == 6 && t.refby == 8 && t.windows == 21, format!("{} pictures, {} triggers, {} RefBy", t.foreign, t.triggers, t.refby));
    h.check("corpus/kinds", t.geometry_rows.len() == 10 && t.units.len() == 17 && t.functions.len() == 59,
        format!("{} geometry rows, {} units, {} functions", t.geometry_rows.len(), t.units.len(), t.functions.len()));
}
