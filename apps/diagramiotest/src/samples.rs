// samples -- the self-built files of the six formats read into the model: every field, extra,
// part and relationship where it belongs (pkg::sample is the drawing)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    CellAddr, DocKind, Document, ExtraItem, Extras, ForeignPayload, Formula, Origin, Owner, Place, RowKey, SecKey,
    ShapeKind, ShapeRef, SheetRef, TextItem, Unit, Value,
};
use diagram_io::vsdx::{self, Format, Limits, Loaded};

use crate::pkg::{self, Pkg, CT_DRAWING, DECL};
use crate::{out, H};

const MAIN_TYPES: [(&str, &str); 6] = [
    ("vsdx", "application/vnd.ms-visio.drawing.main+xml"),
    ("vstx", "application/vnd.ms-visio.template.main+xml"),
    ("vssx", "application/vnd.ms-visio.stencil.main+xml"),
    ("vsdm", "application/vnd.ms-visio.drawing.macroEnabled.main+xml"),
    ("vstm", "application/vnd.ms-visio.template.macroEnabled.main+xml"),
    ("vssm", "application/vnd.ms-visio.stencil.macroEnabled.main+xml"),
];

/// a stencil: masters only, a page without contents (as Visio writes stencils)
fn stencil(main_ct: &str) -> Pkg {
    let mut p = pkg::sample(main_ct);
    p.edit("visio/pages/pages.xml", "<Rel r:id='rId1'/></Page>", "</Page>");
    p.unrel("visio/pages/pages.xml", "rId1");
    p.drop("visio/pages/page1.xml");
    p.unrel("visio/pages/page1.xml", "rId1");
    p.unrel("visio/pages/page1.xml", "rId2");
    p.drop("visio/media/image1.png");
    p
}

/// a macro-enabled file: the sample with a macro project
fn with_macros(mut p: Pkg) -> Pkg {
    p.part("visio/vbaProject.bin", "application/vnd.ms-office.vbaProject", alloc::vec![0xCC; 64]);
    p.rel("visio/document.xml", "rId20", "vbaProject", "vbaProject.bin");
    p
}

/// the sample of a format
pub fn file(ext: &str) -> Vec<u8> {
    let ct = MAIN_TYPES.iter().find(|t| t.0 == ext).map_or(CT_DRAWING, |t| t.1);
    let p = if ext.starts_with("vss") { stencil(ct) } else { pkg::sample(ct) };
    let p = if ext.ends_with('m') { with_macros(p) } else { p };
    p.bytes()
}

/// --write-samples DIR
pub fn write(dir: &str) -> i32 {
    for (ext, _) in MAIN_TYPES {
        let path = format!("{dir}/probe.{ext}");
        if !plat::write_file(&path, &file(ext)) {
            out(&format!("error: cannot write {path}\n"));
            return 1;
        }
        out(&format!("wrote {path}\n"));
    }
    0
}

fn texts(doc: &Document, extras: &Extras) -> Vec<(Place, String)> {
    extras
        .items()
        .iter()
        .map(|e| {
            let t = match e.item {
                ExtraItem::Attribute { name, value } => format!("@{}={}", doc.str(name), doc.str(value)),
                ExtraItem::Element(x) => format!("<{}", doc.str(x)),
                ExtraItem::Text(x) => format!("\"{}", doc.str(x)),
            };
            (e.place, t)
        })
        .collect()
}

fn has(doc: &Document, extras: &Extras, place: Place, start: &str) -> bool {
    texts(doc, extras).iter().any(|(p, t)| *p == place && t.starts_with(start))
}

pub fn run(h: &mut H) {
    // the formats by content type and by extension
    for (ext, ct) in MAIN_TYPES {
        let f = Format::from_content_type(ct);
        h.check("samples/format", f.is_some_and(|f| f.extension() == ext && f.content_type() == ct) && Format::from_extension(&ext.to_ascii_uppercase()) == f,
            format!("{ext} {f:?}"));
    }
    h.check("samples/format", Format::from_content_type("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml").is_none(), String::from("DOCX is none"));
    // every format reads with its kind and macro flag
    for (ext, _) in MAIN_TYPES {
        let bytes = file(ext);
        let r = vsdx::read(&bytes, &Limits::VSDX);
        let want_kind = match &ext[..3] {
            "vsd" => DocKind::Drawing,
            "vst" => DocKind::Template,
            _ => DocKind::Stencil,
        };
        let macros = ext.ends_with('m');
        let ok = r.as_ref().is_ok_and(|l| {
            l.format.kind == want_kind
                && l.format.macros == macros
                && l.doc.kind == want_kind
                && l.doc.macros == macros
                && l.warnings.is_empty()
                && (!macros || l.doc.parts.iter().any(|p| l.doc.str(p.name) == "/visio/vbaProject.bin" && p.data == [0xCC; 64]))
        });
        let w = r.as_ref().map(|l| l.warnings.len()).unwrap_or(0);
        h.check("samples/kind", ok, format!("{ext}: {:?}, {w} warnings", r.as_ref().map(|l| l.format).map_err(|e| e.clone())));
    }
    let bytes = pkg::sample(CT_DRAWING).bytes();
    match vsdx::read(&bytes, &Limits::VSDX) {
        Ok(l) => drawing(h, &l),
        Err(e) => h.check("samples/drawing", false, format!("{e}")),
    }
    // the same drawing with UTF-16 and BOM parts, a prefixed Visio namespace and CR LF text
    let mut p = pkg::sample(CT_DRAWING);
    let pages = p.text("visio/pages/pages.xml").replace("encoding='utf-8'", "encoding='UTF-16'");
    let mut utf16 = alloc::vec![0xFFu8, 0xFE];
    for u in pages.encode_utf16() {
        utf16.extend_from_slice(&u.to_le_bytes());
    }
    p.set("visio/pages/pages.xml", utf16);
    let doc = p.text("visio/document.xml");
    let mut bom = alloc::vec![0xEFu8, 0xBB, 0xBF];
    bom.extend_from_slice(doc.as_bytes());
    p.set("visio/document.xml", bom);
    let m = format!("{DECL}<v:MasterContents xmlns:v='http://schemas.microsoft.com/office/visio/2012/main'><v:Shapes><v:Shape ID='5' Type='Shape'><v:Cell N='PinX' V='0.5'/></v:Shape></v:Shapes></v:MasterContents>");
    p.set("visio/masters/master1.xml", m);
    match vsdx::read(&p.bytes(), &Limits::VSDX) {
        Ok(l) => {
            let doc = &l.doc;
            let pages = doc.page_count();
            let m = doc.masters().next().and_then(|m| doc.master(m));
            let pin = m.and_then(|m| m.master_shape().and_then(|s| m.shapes.get(s))).and_then(|d| d.sheet.cell(doc.sym("PinX")?)).map(|c| c.value);
            h.check("samples/encodings", pages == 2 && pin == Some(Value::Num(0.5)) && doc.styles().count() == 3, format!("UTF-16 pages {pages}, prefixed master PinX {pin:?}"));
        }
        Err(e) => h.check("samples/encodings", false, format!("{e}")),
    }
}

fn drawing(h: &mut H, l: &Loaded) {
    let doc = &l.doc;
    let s = |t: &str| doc.sym(t);
    h.check("samples/drawing", l.warnings.is_empty() && l.more_warnings == 0, format!("{} warnings: {:?}", l.warnings.len(), l.warnings));
    // document.xml
    let st = &doc.settings;
    h.check("samples/settings",
        st.top_page == Some(0) && st.default_text_style == Some(3) && st.default_guide_style == Some(4)
            && st.attrs.len() == 1 && doc.str(st.attrs[0].0) == "Custom"
            && st.items.len() == 4 && doc.str(st.items[2].0) == "SnapAngles" && doc.str(st.items[2].1).is_empty()
            && has(doc, &st.extras, Place::AfterChild(4), "<<Odd a='1'><b/></Odd>"),
        format!("{} items, extras {:?}", st.items.len(), texts(doc, &st.extras)));
    h.check("samples/colors", doc.colors.len() == 2 && doc.str(doc.colors[1].rgb) == "#FFFFFF" && has(doc, &doc.colors_extras, Place::AfterChild(2), "<<ColorEntry IX='2'"),
        format!("{} colours, extras {:?}", doc.colors.len(), texts(doc, &doc.colors_extras)));
    h.check("samples/faces", doc.faces.len() == 1 && doc.faces[0].attrs.len() == 4, format!("{:?}", doc.faces.len()));
    let styles: Vec<u32> = doc.styles().filter_map(|r| doc.style(r)).map(|x| x.header.id).collect();
    let normal = doc.style_by_name_u("Normal").and_then(|r| doc.style(r));
    let guide = doc.style_by_id(4).and_then(|r| doc.style(r));
    h.check("samples/styles",
        styles == [0, 3, 4] && normal.is_some_and(|n| n.header.based_on.fill == Some(0) && n.sheet.cells.len() == 1)
            && guide.is_some_and(|g| g.header.attrs.len() == 1 && g.sheet.is_empty()),
        format!("{styles:?}"));
    let ds = doc.doc_sheet();
    let user = ds.sheet.section(SecKey::one(s("User").unwrap_or_default()));
    let key = user.and_then(|u| u.row(RowKey::Named(s("Key").unwrap_or_default())));
    let v007 = key.and_then(|r| r.cell(s("Value")?)).map(|c| (c.value, c.unit));
    let prompt = key.and_then(|r| r.cell(s("Prompt")?)).map(|c| c.formula);
    h.check("samples/doc-sheet",
        ds.styles.line == Some(0) && ds.attrs.len() == 2 && ds.sheet.cells.len() == 1
            && matches!(v007, Some((Value::Text(t), Some(Unit::Str))) if doc.str(t) == "007") && prompt == Some(Formula::NoFormula),
        format!("{v007:?} {prompt:?}"));
    let ex = texts(doc, &doc.extras);
    h.check("samples/doc-extras",
        ex.len() == 2 && ex[0].0 == Place::AfterChild(5) && ex[0].1.starts_with("<<EventList>") && ex[1].1 == "<<x:Foreign xmlns:x='urn:test'>kept &amp; sound</x:Foreign>"
            && doc.attrs.iter().any(|(n, _)| doc.str(*n) == "xmlns:r"),
        format!("{ex:?}"));
    // themes, parts, relationships
    h.check("samples/themes", doc.themes.len() == 1 && doc.str(doc.themes[0].part) == "/visio/theme/theme1.xml" && doc.themes[0].data.starts_with(b"<?xml"),
        format!("{} themes", doc.themes.len()));
    let parts: Vec<(String, String)> = doc.parts.iter().map(|p| (String::from(doc.str(p.name)), String::from(doc.str(p.rel_type)))).collect();
    let want = [
        "/docProps/core.xml", "/docProps/app.xml", "/visio/windows.xml", "/visio/comments.xml", "/visio/data/connections.xml",
        "/visio/data/recordsets.xml", "/visio/data/recordset1.xml", "/visio/solutions/solutions.xml", "/custom/thing.bin", "/orphan.dat",
        "/visio/media/image1.png",
    ];
    let names: Vec<&str> = parts.iter().map(|p| p.0.as_str()).collect();
    let orphan = parts.iter().find(|p| p.0 == "/orphan.dat").is_some_and(|p| p.1.is_empty());
    let thing = doc.parts.iter().find(|p| doc.str(p.name) == "/custom/thing.bin");
    let windows = parts.iter().find(|p| p.0 == "/visio/windows.xml").is_some_and(|p| p.1.ends_with("/windows"));
    h.check("samples/parts",
        names.len() == want.len() && want.iter().all(|w| names.contains(w)) && orphan && windows
            && thing.is_some_and(|t| t.data == [0, 1, 2, 3, 255] && doc.str(t.content_type) == "application/x-test"),
        format!("{names:?}"));
    let ext = doc.rels.iter().find(|r| r.external);
    h.check("samples/rels",
        doc.rels.len() == 19 && ext.is_some_and(|r| doc.str(r.target) == "https://example.org/never-fetched" && r.part.is_none() && doc.str(r.source) == "/visio/document.xml")
            && doc.rels.iter().any(|r| doc.str(r.source).is_empty() && r.part.is_some_and(|p| doc.str(p) == "/visio/document.xml")),
        format!("{} relationships", doc.rels.len()));
    // masters
    let m = doc.master_by_id(2).and_then(|r| doc.master(r));
    let ok = m.is_some_and(|m| {
        doc.str(m.header.name_u.unwrap_or_default()) == "Box"
            && m.header.base_id.is_some()
            && m.header.attrs.len() == 3
            && m.icon.is_some_and(|i| doc.str(i) == "QUJD\nREVG")
            && m.part.is_some_and(|p| doc.str(p) == "/visio/masters/master1.xml")
            && m.header.sheet_styles.line == Some(0)
            && has(doc, &m.sheet.extras, Place::Element, "@Extra=m")
            && has(doc, &m.extras, Place::AfterChild(3), "<<Future/>")
            && m.shapes.len() == 1
    });
    h.check("samples/master", ok, String::from("Box with icon, part, extras"));
    let ms = m.and_then(|m| m.master_shape().and_then(|s| m.shapes.get(s)));
    let geo = ms.and_then(|d| d.sheet.section(SecKey { name: s("Geometry").unwrap_or_default(), index: 0 }));
    let serial = ms.and_then(|d| d.sheet.section(SecKey::one(s("Property").unwrap_or_default()))).and_then(|p| p.row(RowKey::Named(s("Serial")?))).and_then(|r| r.cell(s("Value")?)).map(|c| c.value);
    h.check("samples/master-shape",
        geo.is_some_and(|g| g.ix_written && g.rows.len() == 2 && g.cells.len() == 1 && g.rows[1].key == RowKey::Index(2) && g.rows[1].kind == s("LineTo"))
            && matches!(serial, Some(Value::Text(t)) if doc.str(t) == "0042")
            && ms.is_some_and(|d| d.text.as_ref().is_some_and(|t| t.chars() == "Master text")),
        format!("{serial:?}"));
    // pages
    let pages: Vec<u32> = doc.pages().filter_map(|p| doc.page(p)).map(|p| p.header.id).collect();
    let p0r = doc.page_by_id(0);
    let p0 = p0r.and_then(|r| doc.page(r));
    let bg = doc.page_by_id(3).and_then(|r| doc.page(r));
    h.check("samples/pages",
        pages == [0, 3] && doc.foreground_pages().count() == 1 && bg.is_some_and(|b| b.header.background && b.shapes.is_empty())
            && p0.is_some_and(|p| {
                p.header.back_page == Some(3) && p.header.is_custom_name == Some(true) && p.header.attrs.len() == 2
                    && p.sheet.cells.len() == 2 && p.sheet.sections.len() == 1 && p.theme.is_none()
                    && p.part.is_some_and(|x| doc.str(x) == "/visio/pages/page1.xml")
            })
            && has(doc, &doc.pages_extras, Place::AfterChild(2), "<<Note>")
            && has(doc, &doc.pages_extras, Place::Element, "@xml:space=preserve"),
        format!("{pages:?}"));
    let Some(p0) = p0 else { return };
    let Some(p0r) = p0r else { return };
    h.check("samples/contents-extras",
        has(doc, &p0.contents_extras, Place::Element, "@Custom=c") && has(doc, &p0.contents_extras, Place::AfterChild(2), "<<Later/>"),
        format!("{:?}", texts(doc, &p0.contents_extras)));
    // shapes
    let t = &p0.shapes;
    let ids: Vec<u32> = t.iter().filter_map(|s| t.id(s)).collect();
    let kinds: Vec<ShapeKind> = t.iter().filter_map(|s| t.get(s)).map(|d| d.header.kind).collect();
    h.check("samples/shapes",
        ids == [1, 2, 3, 4, 6, 7, 8] && t.top().len() == 5
            && kinds == [ShapeKind::Shape, ShapeKind::Group, ShapeKind::Shape, ShapeKind::Foreign, ShapeKind::Shape, ShapeKind::Foreign, ShapeKind::Unspecified],
        format!("{ids:?}"));
    let s1 = t.by_id(1).and_then(|x| t.get(x));
    let Some(s1) = s1 else { return };
    let cell = |n: &str| s(n).and_then(|n| s1.sheet.cell(n)).copied();
    let angle = cell("Angle");
    let odd = cell("Odd");
    h.check("samples/cells",
        cell("PinX").is_some_and(|c| c.value == Value::Num(2.0) && c.formula == Formula::Absent && !c.stale)
            && cell("Width").is_some_and(|c| c.formula == Formula::Inherited)
            && angle.is_some_and(|c| c.unit == Some(Unit::Degrees) && c.error.is_some_and(|e| doc.str(e) == "#REF!"))
            && odd.is_some_and(|c| matches!(c.unit, Some(Unit::Other(u)) if doc.str(u) == "XYZ") && matches!(c.value, Value::Text(v) if doc.str(v) == "7"))
            && cell("LocPinX").is_some_and(|c| matches!(c.formula, Formula::Expr(f) if doc.str(f) == "Width*0.5")),
        format!("Angle {angle:?}"));
    h.check("samples/shape-extras",
        s1.header.master == Some(2)
            && has(doc, &s1.extras, Place::Element, "@Foo=bar")
            && has(doc, &s1.extras, Place::InCell(4), "<<RefBy T='Shape' ID='6'/>")
            && has(doc, &s1.extras, Place::AfterChild(7), "<<Bar baz='1'/>")
            && has(doc, &s1.extras, Place::AfterChild(7), "\"loose text")
            && s1.data[0].is_some_and(|d| doc.str(d) == "first"),
        format!("{:?}", texts(doc, &s1.extras)));
    let items = s1.text.as_ref().map(|t| t.items.clone()).unwrap_or_default();
    let ok = matches!(items.as_slice(), [TextItem::Chars(a), TextItem::Cp(0), TextItem::Chars(b), TextItem::Pp(1), TextItem::Chars(c), TextItem::Other(f), TextItem::Tp(0), TextItem::Other(x)]
        if &**a == "Hallo\n" && &**b == "Welt" && &**c == " & mehr" && doc.str(*f) == "<fld IX='0'>12</fld>" && doc.str(*x) == "<b:x xmlns:b='urn:t'/>");
    h.check("samples/text", ok, format!("{items:?}"));
    let g = t.by_id(2).and_then(|x| t.get(x));
    let geo: Vec<(u32, bool, usize)> = g.map(|g| g.sheet.sections.iter().filter(|x| doc.str(x.name) == "Geometry").map(|x| (x.index, x.ix_written, x.rows.len())).collect()).unwrap_or_default();
    let first = g.and_then(|g| g.sheet.sections.first());
    let user_del = g.is_some_and(|g| g.sheet.sections.iter().any(|x| doc.str(x.name) == "User" && x.del));
    h.check("samples/sections",
        geo == [(0, false, 2), (1, false, 0)]
            && first.is_some_and(|f| f.rows[0].key == RowKey::Index(0) && !f.rows[0].ix_written && f.rows[1].del && f.rows[1].key == RowKey::Index(1))
            && user_del
            && g.is_some_and(|g| g.sheet.triggers.len() == 1 && doc.str(g.sheet.triggers[0].inner) == "<RefBy T='Shape' ID='1'/>")
            && g.is_some_and(|g| has(doc, &g.extras, Place::InShapes(1), "<<Unknown/>")),
        format!("{geo:?}"));
    let s4 = t.by_id(4).and_then(|x| t.get(x)).and_then(|d| d.foreign.as_deref());
    let s7 = t.by_id(7).and_then(|x| t.get(x)).and_then(|d| d.foreign.as_deref());
    h.check("samples/foreign",
        s4.is_some_and(|f| f.attrs.len() == 3 && matches!(f.payload, ForeignPayload::Part(p) if doc.str(p) == "/visio/media/image1.png"))
            && s7.is_some_and(|f| matches!(&f.payload, ForeignPayload::Inline(b) if &**b == "iVBORw0KGgo="))
            && doc.parts.iter().any(|p| doc.str(p.name) == "/visio/media/image1.png" && p.data == pkg::PNG),
        format!("{:?}", s4.map(|f| &f.payload)));
    let s8 = t.by_id(8).and_then(|x| t.get(x));
    h.check("samples/deleted", s8.is_some_and(|d| d.header.del && d.header.kind == ShapeKind::Unspecified), String::from("Del shape kept"));
    let c = &p0.connects;
    h.check("samples/connects",
        c.len() == 2 && c[0].from_sheet == 6 && c[0].to_sheet == 1 && c[0].from_part == Some(9) && c[0].to_part == Some(3)
            && c[0].to_cell == s("PinX") && c[1].to_cell == s("Connections.X1") && has(doc, &c[1].extras, Place::Element, "@Glue=odd"),
        format!("{} connects", c.len()));
    // the model works on what was read: inheritance from the master shape, nothing recomputed
    let r1 = ShapeRef { owner: Owner::Page(p0r), slot: t.by_id(1).unwrap_or(0) };
    let height = doc.resolve(SheetRef::Shape(r1), &CellAddr::single(s("Height").unwrap_or_default()));
    let width = doc.resolve(SheetRef::Shape(r1), &CellAddr::single(s("Width").unwrap_or_default()));
    h.check("samples/inherit",
        height.origin == Origin::Master && height.num() == Some(0.5) && width.origin == Origin::Master
            && width.cached.is_some_and(|c| c.value == Value::Num(1.0) && c.formula == Formula::Inherited)
            && doc.shape(r1).is_some_and(|x| x.text().is_some_and(|t| t.chars().starts_with("Hallo"))),
        format!("Height {:?} from {:?}", height.num(), height.origin));
}
