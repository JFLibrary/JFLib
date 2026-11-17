// hostile_write -- models the reader never makes, written: the writer must not crash, must not
// write a file the reader refuses, and says why when it cannot write (roundtrip/hostile)
//
//   extras at places their element does not have, beyond the last child, attributes the writer
//   writes itself; raw XML that is not well-formed where it stands; characters XML cannot carry;
//   pages claiming one part or a part name that is none; parts kept as bytes with bad names or
//   no content type; two themes with one name; a macro project in a format without macros; a
//   picture whose part is gone; deleted subshapes without ID; NaN, infinities and extreme
//   numbers; indexes that are not positions; groups 3000 deep; relationships that no longer
//   fit; namespaces bound elsewhere; dangling references; an empty document; tight ZIP limits

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    Cell, DocKind, Document, Extra, ExtraItem, ForeignData, ForeignPayload, Formula, Page, PageHeader, Part, Place,
    Relationship, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, Sym, Text, TextItem, Value, NO_ID,
};
use diagram_io::vsdx::{self, Format, Limits, Loaded, Measure, WriteError, WriteOptions};

use crate::build;
use crate::H;

// a reader for the deep groups
const DEEP: Limits = Limits { depth: 1 << 20, ..Limits::VSDX };

// write, then read back with `lim`: "ok", "ok+N" (warnings) or why not
fn outcome(doc: &Document, opts: &WriteOptions, lim: &Limits) -> String {
    match vsdx::write(doc, opts) {
        Ok(b) => match vsdx::read(&b, lim) {
            Ok(l) if l.warnings.is_empty() => String::from("ok"),
            Ok(l) => format!("ok+{}", l.warnings.len() + l.more_warnings),
            Err(e) => format!("unreadable: {e}"),
        },
        Err(WriteError::Xml { part, line }) => format!("Xml {part}:{line}"),
        Err(WriteError::Part { what, .. }) => format!("Part: {what}"),
        Err(WriteError::Macros) => String::from("Macros"),
        Err(WriteError::Package(e)) => format!("Package {e:?}"),
        Err(e) => format!("{e:?}"),
    }
}

fn written(h: &mut H, name: &str, want: &str, doc: &Document) {
    let got = outcome(doc, &WriteOptions::DEFAULT, &Limits::VSDX);
    h.check(name, got.starts_with(want), format!("{got} (want {want})"));
}

// write and read back (None: either failed)
fn back(doc: &Document, lim: &Limits) -> Option<Loaded> {
    vsdx::read(&vsdx::write(doc, &WriteOptions::DEFAULT).ok()?, lim).ok()
}

fn extra(place: Place, item: ExtraItem) -> Extra {
    Extra { place, item }
}

// a shape on the first page (under `parent`); its slot
fn add(doc: &mut Document, parent: Option<u32>, id: u32, d: ShapeData) -> Option<u32> {
    let pr = doc.pages().next()?;
    doc.page_mut(pr)?.shapes.insert(parent, id, d).ok()
}

// the shape with this ID on the first page of a document read back
fn shape_of(l: &Loaded, id: u32) -> Option<&ShapeData> {
    let pg = l.doc.pages().next().and_then(|p| l.doc.page(p))?;
    pg.shapes.by_id(id).and_then(|x| pg.shapes.get(x))
}

fn base() -> Document {
    build::drawing(Measure::Metric, None)
}

fn small(doc: &mut Document) -> ShapeData {
    build::box_shape(doc, 0.5, 0.5, 1.0, 1.0, "", false)
}

fn out_of_place(h: &mut H) {
    let mut doc = base();
    let (bar, v, id, n) = (doc.intern("<Bar/>"), doc.intern("x"), doc.intern("ID"), doc.intern("N"));
    let txt = doc.intern(" loose & <text> ");
    let mut d = build::box_shape(&mut doc, 0.5, 0.5, 1.0, 1.0, "x", false);
    for e in [
        extra(Place::InCell(999), ExtraItem::Element(bar)),
        extra(Place::AfterChild(10_000), ExtraItem::Element(bar)),
        extra(Place::InShapes(3), ExtraItem::Element(bar)),
        extra(Place::InConnects(0), ExtraItem::Text(txt)),
        extra(Place::Element, ExtraItem::Attribute { name: id, value: v }),
        extra(Place::Element, ExtraItem::Element(bar)),
        extra(Place::AfterChild(1), ExtraItem::Attribute { name: v, value: v }),
        extra(Place::InCell(0), ExtraItem::Attribute { name: n, value: v }),
        extra(Place::InCell(0), ExtraItem::Text(txt)),
    ] {
        d.extras.push(e);
    }
    add(&mut doc, None, 99, d);
    let first = doc.pages().next();
    if let Some(p) = first.and_then(|p| doc.page_mut(p)) {
        p.contents_extras.push(extra(Place::AfterChild(77), ExtraItem::Text(txt)));
        p.extras.push(extra(Place::InCell(5), ExtraItem::Element(bar)));
    }
    written(h, "roundtrip/hostile extras out of place", "ok", &doc);
}

fn raw_xml(h: &mut H) {
    for (raw, want) in [("<a>", "Xml"), ("</Shape>", "Xml"), ("<b:x/>", "Xml"), ("<a></b>", "Xml"), ("<!DOCTYPE x>", "Xml"), ("plain text", "ok")] {
        let mut doc = base();
        let x = doc.intern(raw);
        doc.extras.push(extra(Place::AfterChild(2), ExtraItem::Element(x)));
        written(h, &format!("roundtrip/hostile raw {raw:?}"), want, &doc);
    }
    let mut doc = base();
    let (x, v) = (doc.intern("bad name"), doc.intern("1"));
    doc.pages_extras.push(extra(Place::Element, ExtraItem::Attribute { name: x, value: v }));
    written(h, "roundtrip/hostile attribute name", "Xml", &doc);
    let mut doc = base();
    let t = doc.intern("<cp IX='0'>");
    let mut d = small(&mut doc);
    d.text = Some(Box::new(Text { items: alloc::vec![TextItem::Other(t)], extras: Default::default() }));
    add(&mut doc, None, 99, d);
    written(h, "roundtrip/hostile raw text mark", "Xml", &doc);
}

fn characters(h: &mut H) {
    let mut doc = base();
    let nasty = "a\u{1}b\u{ffff}c\rd\te\nf'g\"h<i>&j]]>k";
    let want = "a\u{fffd}b\u{fffd}c\rd\te\nf'g\"h<i>&j]]>k";
    let sym = doc.intern(nasty);
    let first = doc.pages().next();
    if let Some(p) = first.and_then(|p| doc.page_mut(p)) {
        p.header.name = Some(sym);
    }
    let mut sh = small(&mut doc);
    sh.text = Some(Box::new(Text { items: alloc::vec![TextItem::Chars(Box::from(nasty))], extras: Default::default() }));
    if let Some(c) = sh.sheet.cells.first_mut() {
        c.formula = Formula::Expr(sym);
    }
    add(&mut doc, None, 99, sh);
    let ok = back(&doc, &Limits::VSDX).is_some_and(|l| {
        let d = &l.doc;
        let name = d.pages().next().and_then(|p| d.page(p)).and_then(|p| p.header.name).map(|n| d.str(n));
        let shape = shape_of(&l, 99);
        let text = shape.and_then(|x| x.text.as_ref()).map(|t| t.chars());
        let f = shape.and_then(|x| x.sheet.cells.first()).and_then(|c| c.formula.expr()).map(|f| d.str(f));
        name == Some(want) && text.as_deref() == Some(want) && f == Some(want)
    });
    h.check("roundtrip/hostile characters", ok, String::from("U+0001 and U+FFFF as U+FFFD, CR, tab, line end, quotes and markup unchanged"));
}

fn names(h: &mut H) {
    // two pages claiming one part, a page part name that is no part name: new names, nothing lost
    let mut doc = base();
    let p1 = doc.intern("/visio/pages/page1.xml");
    let first = doc.pages().next();
    if let Some(p) = first.and_then(|p| doc.page_mut(p)) {
        p.part = Some(p1);
    }
    let pname = doc.intern("Page-2");
    let mut second = Page::new(PageHeader { id: 7, name_u: Some(pname), ..Default::default() });
    second.part = Some(p1);
    let _ = doc.insert_page(second);
    let mut third = Page::new(PageHeader { id: 8, ..Default::default() });
    third.part = Some(doc.intern("/visio/pages/a?b.xml"));
    let _ = doc.insert_page(third);
    let l = back(&doc, &Limits::VSDX);
    let parts: Vec<String> = l.as_ref().map_or(Vec::new(), |l| l.doc.pages().filter_map(|p| l.doc.page(p)).map(|p| String::from(p.part.map_or("-", |x| l.doc.str(x)))).collect());
    let shapes = l.as_ref().and_then(|l| l.doc.pages().next().and_then(|p| l.doc.page(p)).map(|p| p.shapes.len())).unwrap_or(0);
    h.check(
        "roundtrip/hostile page parts",
        parts == ["/visio/pages/page1.xml", "/visio/pages/page2.xml", "/visio/pages/page3.xml"] && shapes == 9,
        format!("{parts:?}, {shapes} shapes on page 1"),
    );
    // parts kept as bytes need valid names of their own and a content type
    for (name, ct, want) in [
        ("/bad?name.bin", "application/x-test", "Part"),
        ("/visio/document.xml", "application/xml", "Part"),
        ("/[Content_Types].xml", "application/xml", "Part"),
        ("/x/_rels/y.xml.rels", "application/xml", "Part"),
        ("/custom/empty.bin", "", "Part"),
        ("/custom/fine.bin", "application/x-test", "ok"),
    ] {
        let mut doc = base();
        let part = Part { name: doc.intern(name), content_type: doc.intern(ct), rel_type: Sym::EMPTY, data: alloc::vec![1, 2, 3] };
        doc.parts.push(part);
        written(h, &format!("roundtrip/hostile part {name:?}"), want, &doc);
    }
    // two themes with one name: the second gets a new one
    let mut doc = base();
    if let Some(t) = doc.themes.first().cloned() {
        doc.themes.push(t);
    }
    let n = back(&doc, &Limits::VSDX).map(|l| l.doc.themes.iter().map(|t| String::from(l.doc.str(t.part))).collect::<Vec<_>>());
    let want = ["/visio/theme/theme1.xml", "/visio/theme/theme2.xml"];
    h.check("roundtrip/hostile themes", n.as_ref().is_some_and(|v| v.iter().map(String::as_str).eq(want.iter().copied())), format!("{n:?}"));
}

fn macros_and_pictures(h: &mut H) {
    let mut doc = base();
    let part = Part {
        name: doc.intern("/visio/vbaProject.bin"),
        content_type: doc.intern("application/vnd.ms-office.vbaProject"),
        rel_type: doc.intern("http://schemas.microsoft.com/office/2006/relationships/vbaProject"),
        data: alloc::vec![0xCC; 16],
    };
    doc.parts.push(part);
    written(h, "roundtrip/hostile macros in .vsdx", "Macros", &doc);
    let opts = WriteOptions { format: Format::from_extension("vsdm"), ..WriteOptions::DEFAULT };
    let got = outcome(&doc, &opts, &Limits::VSDX);
    h.check("roundtrip/hostile macros in .vsdm", got == "ok", got);
    // a picture whose part is gone: the shape is written without its data
    let mut doc = base();
    let mut d = small(&mut doc);
    d.header.kind = ShapeKind::Foreign;
    let gone = doc.intern("/visio/media/gone.png");
    d.foreign = Some(Box::new(ForeignData { attrs: Vec::new(), payload: ForeignPayload::Part(gone), extras: Default::default() }));
    add(&mut doc, None, 99, d);
    let p = back(&doc, &Limits::VSDX).and_then(|l| shape_of(&l, 99).and_then(|d| d.foreign.as_ref()).map(|f| f.payload.clone()));
    h.check("roundtrip/hostile missing picture", p == Some(ForeignPayload::None), format!("{p:?}"));
}

fn numbers_and_ids(h: &mut H) {
    // deleted subshapes without ID, several of them
    let mut doc = base();
    for _ in 0..3 {
        let mut d = ShapeData::new(ShapeKind::Unspecified);
        d.header.del = true;
        add(&mut doc, None, NO_ID, d);
    }
    let n = back(&doc, &Limits::VSDX).and_then(|l| {
        let pg = l.doc.pages().next().and_then(|p| l.doc.page(p))?;
        Some(pg.shapes.iter().filter(|&x| pg.shapes.id(x) == Some(NO_ID)).count())
    });
    h.check("roundtrip/hostile NO_ID", n == Some(3), format!("{n:?} shapes with ID 4294967295"));
    // numbers: NaN and infinities come back as text, the rest as the same f64 (-0 as 0)
    let mut doc = base();
    let vals = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -0.0, 5e-324, 1e-5, 1.2345678901234568e20, 0.1 + 0.2, -123.456];
    let mut d = small(&mut doc);
    d.sheet.cells.clear();
    for (i, v) in vals.iter().enumerate() {
        let n = doc.intern(&format!("Odd{i}"));
        d.sheet.cells.push(Cell::constant(n, Value::Num(*v), None));
    }
    add(&mut doc, None, 99, d);
    let got: Vec<String> = back(&doc, &Limits::VSDX).map_or(Vec::new(), |l| {
        shape_of(&l, 99).map_or(Vec::new(), |sh| {
            sh.sheet
                .cells
                .iter()
                .map(|c| match c.value {
                    Value::Num(x) => format!("{x:?}"),
                    Value::Text(t) => format!("text {}", l.doc.str(t)),
                    Value::Absent => String::from("-"),
                })
                .collect()
        })
    });
    let want: Vec<String> = vals.iter().map(|v| if v.is_finite() { format!("{:?}", if *v == 0.0 { 0.0 } else { *v }) } else { format!("text {v}") }).collect();
    h.check("roundtrip/hostile numbers", got == want, format!("{got:?}"));
    // rows and sections whose index is not their position while ix_written says it is
    let mut doc = base();
    let mut d = small(&mut doc);
    if let Some(g) = d.sheet.sections.first_mut() {
        g.index = 3;
        g.ix_written = false;
        if let Some(r) = g.rows.first_mut() {
            r.key = RowKey::Index(5);
            r.ix_written = false;
        }
    }
    let user = doc.intern("User");
    let mut sec = Section::new(SecKey { name: user, index: 2 });
    sec.rows.push(Row::new(RowKey::Named(doc.intern("Row_1"))));
    d.sheet.sections.push(sec);
    add(&mut doc, None, 99, d);
    let keys = back(&doc, &Limits::VSDX).and_then(|l| {
        let sh = shape_of(&l, 99)?;
        let g = sh.sheet.sections.first()?;
        Some((g.index, g.rows.first()?.key, sh.sheet.sections.get(1)?.index))
    });
    h.check("roundtrip/hostile indexes", keys == Some((3, RowKey::Index(5), 2)), format!("{keys:?}"));
    // groups 3000 deep: no recursion in the writer
    let mut doc = base();
    let mut parent = None;
    for i in 0..3000u32 {
        let mut d = ShapeData::new(ShapeKind::Group);
        let px = doc.intern("PinX");
        d.sheet.cells.push(Cell::constant(px, Value::Num(i as f64), None));
        parent = add(&mut doc, parent, 1000 + i, d);
    }
    let t0 = plat::time_us();
    let n = back(&doc, &DEEP).and_then(|l| l.doc.pages().next().and_then(|p| l.doc.page(p)).map(|p| p.shapes.len()));
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("roundtrip/hostile deep groups", n == Some(9 + 3000), format!("{n:?} shapes in {ms} ms"));
}

fn rel(doc: &mut Document, source: &str, id: &str, kind: &str, target: &str, external: bool, part: Option<&str>) {
    let r = Relationship {
        source: doc.intern(source),
        id: doc.intern(id),
        kind: doc.intern(kind),
        target: doc.intern(target),
        external,
        part: part.map(|p| doc.intern(p)),
    };
    doc.rels.push(r);
}

fn relationships(h: &mut H) {
    // a gone source, a gone target, ids twice and empty, a pages relationship to the wrong part;
    // the external ones stay with unique ids
    let mut doc = base();
    let img = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    let link = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    rel(&mut doc, "/visio/gone.xml", "rId1", img, "media/a.png", false, Some("/visio/media/a.png"));
    rel(&mut doc, "/visio/document.xml", "rId1", img, "media/gone.png", false, Some("/visio/media/gone.png"));
    rel(&mut doc, "/visio/document.xml", "rId7", link, "https://example.org/x", true, None);
    rel(&mut doc, "/visio/document.xml", "rId7", link, "https://example.org/y", true, None);
    rel(&mut doc, "/visio/document.xml", "", link, "https://example.org/z", true, None);
    rel(&mut doc, "/visio/document.xml", "rId9", "http://schemas.microsoft.com/visio/2010/relationships/pages", "../docProps/app.xml", false, Some("/docProps/app.xml"));
    let l = back(&doc, &Limits::VSDX);
    let ext: Vec<(String, String)> =
        l.as_ref().map_or(Vec::new(), |l| l.doc.rels.iter().filter(|r| r.external).map(|r| (String::from(l.doc.str(r.id)), String::from(l.doc.str(r.target)))).collect());
    let ids: alloc::collections::BTreeSet<&String> = ext.iter().map(|x| &x.0).collect();
    h.check(
        "roundtrip/hostile relationships",
        l.as_ref().is_some_and(|l| l.warnings.is_empty()) && ext.len() == 3 && ids.len() == 3 && ext.first().is_some_and(|x| x.0 == "rId7"),
        format!("{ext:?} {}", if l.is_none() { outcome(&doc, &WriteOptions::DEFAULT, &Limits::VSDX) } else { String::new() }),
    );
    // the root's namespaces bound elsewhere: the Visio and relationship namespaces win
    let mut doc = base();
    let (x, xr, other, wrong) = (doc.intern("xmlns"), doc.intern("xmlns:r"), doc.intern("urn:other"), doc.intern("urn:wrong"));
    doc.attrs = alloc::vec![(x, other), (xr, wrong)];
    written(h, "roundtrip/hostile namespaces", "ok", &doc);
    // a theme index past the themes, a missing style and master: written, read with warnings
    let mut doc = base();
    let first = doc.pages().next();
    if let Some(p) = first.and_then(|p| doc.page_mut(p)) {
        p.theme = Some(5);
    }
    let mut d = small(&mut doc);
    d.header.styles.line = Some(77);
    d.header.master = Some(66);
    add(&mut doc, None, 99, d);
    written(h, "roundtrip/hostile dangling references", "ok+2", &doc);
    // an empty document; tight ZIP limits
    written(h, "roundtrip/hostile empty document", "ok", &Document::new(DocKind::Drawing));
    let doc = base();
    let opts = WriteOptions { zip: archive::Limits { entries: 3, ..Limits::VSDX.zip }, ..WriteOptions::DEFAULT };
    let got = outcome(&doc, &opts, &Limits::VSDX);
    h.check("roundtrip/hostile zip limits", got.starts_with("Package"), got);
}

pub fn run(h: &mut H) {
    out_of_place(h);
    raw_xml(h);
    characters(h);
    names(h);
    macros_and_pictures(h);
    numbers_and_ids(h);
    relationships(h);
}
