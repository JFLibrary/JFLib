// vdx -- (JD16) the XML formats of 2003-2010 (diagram_io::vdx) and the macro parts
//
//   run(h)        group vdx: the self-written drawing, stencil and bitmap (vdx_sample.rs) read
//                 into the model (cells with formula and value, sections, fonts by name, rows,
//                 geometry, text, pictures as parts, masters, pages, properties, windows), written
//                 as .vsdx / .vssx and read back unchanged; hostile files and the limits; the
//                 macro project of .vsdm, .vssm, .vstm (and a .vdx's VBProjectData) passed
//                 through unchanged, never into a format without macros; props.rs
//   write(dir)    --write-vdx DIR: the files for the Visio counter-check (jd16-*.vdx/.vsx and
//                 what JFDiagrammer writes from them)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    CellAddr, Cell, DocKind, Document, ExtraItem, ForeignPayload, Formula, Owner, Page, Part, RowKey, SecKey, ShapeKind,
    ShapeRef, SheetRef, TextItem, Value,
};
use diagram_io::props::{self, Custom};
use diagram_io::vdx;
use diagram_io::vsdx::{self, Error, Format, Limits, Loaded, Warning, WriteError, WriteOptions};

use crate::dump::{dump, first_difference};
use crate::vdx_sample::{self as sample, b64};
use crate::{out, H};

fn read(text: &str, kind: DocKind) -> Result<Loaded, Error> {
    vdx::read(text.as_bytes(), kind, &Limits::VSDX)
}

fn page_of(doc: &Document, id: u32) -> Option<&Page> {
    doc.page_by_id(id).and_then(|p| doc.page(p))
}

fn shape(doc: &Document, page: u32, id: u32) -> Option<ShapeRef> {
    doc.page_by_id(page).and_then(|p| doc.shape_by_id(Owner::Page(p), id))
}

// a single cell of a shape
fn cell<'d>(doc: &'d Document, r: ShapeRef, name: &str) -> Option<&'d Cell> {
    let n = doc.sym(name)?;
    doc.shape(r)?.sheet().cell(n)
}

// a cell of a row of a section of a shape
fn row_cell<'d>(doc: &'d Document, r: ShapeRef, sec: &str, index: u32, row: RowKey, name: &str) -> Option<&'d Cell> {
    let key = SecKey { name: doc.sym(sec)?, index };
    let n = doc.sym(name)?;
    doc.shape(r)?.sheet().section(key)?.row(row)?.cell(n)
}

fn named(doc: &Document, s: &str) -> RowKey {
    RowKey::Named(doc.sym(s).unwrap_or_default())
}

fn text_of(v: Value, doc: &Document) -> String {
    match v {
        Value::Text(s) => String::from(doc.str(s)),
        Value::Num(x) => format!("{x}"),
        Value::Absent => String::from("-"),
    }
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

// the namespace attributes the writer puts on the roots of the pages, masters and their parts
const ROOT_NS: &str = "{Element @\"xmlns\"=\"http://schemas.microsoft.com/office/visio/2012/main\"; Element @\"xmlns:r\"=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"; Element @\"xml:space\"=\"preserve\"; }";

// the dump without what only a package has: relationships, the parts of pages and masters, the
// namespace attributes of the parts' roots
fn content(doc: &Document) -> String {
    let d = dump(doc);
    let mut out = String::new();
    for line in d.lines() {
        let line = line.trim_end();
        if line.starts_with("rel ") || line == format!("pagesx{ROOT_NS}") || line == format!("mastersx{ROOT_NS}") {
            continue;
        }
        let stripped = line.replace(&format!(" cx{ROOT_NS}"), "");
        let line = stripped.as_str();
        let line = match line.find(" part ") {
            Some(i) if line.starts_with("page ") || line.starts_with("master ") => {
                // the part name is the last field before the style refs
                let rest = &line[i + 6..];
                let after = rest.find(' ').map_or("", |j| &rest[j..]);
                format!("{}{}", &line[..i], after)
            }
            _ => String::from(line),
        };
        let line = match line.find(" attrs[") {
            Some(i) if line.starts_with("kind ") => {
                let end = line[i..].find(']').map_or(line.len(), |j| i + j + 1);
                format!("{}{}", &line[..i], &line[end..])
            }
            _ => line,
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

pub fn run(h: &mut H) {
    drawing(h);
    stencil(h);
    bitmap(h);
    hostile(h);
    macros(h);
    props_checks(h);
    window_checks(h);
}

// ---- the drawing window (DF-10)

fn window_checks(h: &mut H) {
    use diagram_io::template::{set_stencil_windows, stencil_windows, StencilWindow};
    use diagram_io::windows::{drawing_window, set_drawing_window, DrawingWindow};
    // a new drawing: its windows part has no drawing window yet
    let mut doc = vsdx::new_document(DocKind::Drawing, vsdx::Measure::Metric);
    h.check("vdx/window/new", drawing_window(&doc).is_none(), String::from("none"));
    let w = DrawingWindow {
        page: Some(0),
        view_scale: Some(1.5),
        center: Some((4.25, 3.5)),
        show_rulers: Some(true),
        show_grid: Some(false),
        show_page_breaks: Some(false),
        show_guides: Some(true),
        show_connection_points: None,
    };
    set_drawing_window(&mut doc, &w);
    set_stencil_windows(&mut doc, &[StencilWindow { document: String::from("jfd_basic_m.vssx"), active: true }]);
    // the stencil windows stay when the drawing window changes, and the other way round
    let mut w2 = w.clone();
    w2.view_scale = Some(0.75);
    set_drawing_window(&mut doc, &w2);
    let bytes = vsdx::write(&doc, &WriteOptions::DEFAULT).ok();
    let back = bytes.as_ref().and_then(|b| vsdx::read(b, &Limits::VSDX).ok());
    let got = back.as_ref().and_then(|l| drawing_window(&l.doc));
    let st = back.as_ref().map(|l| stencil_windows(&l.doc)).unwrap_or_default();
    h.check(
        "vdx/window/set",
        got.as_ref() == Some(&w2) && st.len() == 1 && st[0].document == "jfd_basic_m.vssx" && back.as_ref().is_some_and(|l| l.warnings.is_empty()),
        format!("{got:?}, {} stencils", st.len()),
    );
    // a foreign window part: its other windows and their children stay as written
    let text = sample::drawing();
    if let Ok(l) = read(&text, DocKind::Drawing) {
        let mut doc = l.doc;
        let mut w3 = drawing_window(&doc).unwrap_or_default();
        w3.page = Some(1);
        set_drawing_window(&mut doc, &w3);
        let part = doc.parts.iter().find(|p| doc.str(p.content_type) == "application/vnd.ms-vsdx.windows+xml" || doc.str(p.name) == "/visio/windows.xml").map(|p| String::from_utf8_lossy(&p.data).into_owned()).unwrap_or_default();
        h.check(
            "vdx/window/foreign",
            drawing_window(&doc).and_then(|x| x.page) == Some(1)
                && part.contains("WindowLeft='-8'")
                && part.contains("<TabSplitterPos>0.5</TabSplitterPos>")
                && part.contains("Document='jd16-schablone.vsx'")
                && stencil_windows(&doc).len() == 1,
            part.chars().take(300).collect(),
        );
    }
}

// ---- the drawing

fn drawing(h: &mut H) {
    let text = sample::drawing();
    h.check("vdx/sniff", vdx::sniff(text.as_bytes()) && !vdx::sniff(b"PK\x03\x04") && !vdx::sniff(b"<VisioDocument/>"), String::from("the drawing is one, a ZIP and a root without namespace are none"));
    h.check(
        "vdx/kind_of",
        vdx::kind_of("VDX") == Some(DocKind::Drawing) && vdx::kind_of("vsx") == Some(DocKind::Stencil) && vdx::kind_of("vtx") == Some(DocKind::Template) && vdx::kind_of("vsdx").is_none(),
        String::from("vdx vsx vtx"),
    );
    let l = match read(&text, DocKind::Drawing) {
        Ok(l) => l,
        Err(e) => {
            h.check("vdx/drawing", false, format!("{e}"));
            return;
        }
    };
    let doc = &l.doc;
    let tab = Warning::LeftOut { what: String::from("Tab") };
    h.check("vdx/drawing", l.format == Format { kind: DocKind::Drawing, macros: false } && l.warnings == [tab] && l.more_warnings == 0, format!("{:?} {:?}", l.format, l.warnings));
    // the document's lists
    let faces: Vec<String> = doc.faces.iter().map(|f| f.attrs.iter().map(|(k, v)| format!("{}={}", doc.str(*k), doc.str(*v))).collect::<Vec<_>>().join(" ")).collect();
    h.check(
        "vdx/lists",
        doc.colors.len() == 24
            && doc.str(doc.colors[2].rgb) == "#FF0000"
            && faces.len() == 2
            && faces[0].starts_with("NameU=Arial ")
            && faces[0].contains("Panose=2 11 6 4")
            && !faces[0].contains("ID="),
        format!("colors {}, faces {faces:?}", doc.colors.len()),
    );
    let st = &doc.settings;
    let items: Vec<String> = st.items.iter().map(|(k, v)| format!("{}={}", doc.str(*k), doc.str(*v))).collect();
    h.check(
        "vdx/settings",
        st.top_page == Some(0) && st.default_text_style == Some(1) && st.default_guide_style == Some(3) && items.first().map(String::as_str) == Some("GlueSettings=9") && items.contains(&String::from("SnapAngles=")) && st.extras.items().is_empty(),
        format!("{items:?}, {} extras", st.extras.items().len()),
    );
    // styles: based on, the No Style's font by name
    let normal = doc.style_by_id(1).and_then(|s| doc.style(s));
    let font0 = doc.style_by_id(0).and_then(|s| doc.style(s)).and_then(|s| {
        let key = SecKey { name: doc.sym("Character")?, index: 0 };
        s.sheet.section(key)?.row(RowKey::Index(0))?.cell(doc.sym("Font")?).map(|c| c.value)
    });
    h.check(
        "vdx/styles",
        doc.styles().count() == 4 && normal.is_some_and(|s| s.header.based_on.line == Some(0) && doc.str(s.header.name_u.unwrap_or_default()) == "Normal") && font0.is_some_and(|v| text_of(v, doc) == "Arial"),
        format!("{} styles, No Style's font {:?}", doc.styles().count(), font0.map(|v| text_of(v, doc))),
    );
    let ds = doc.doc_sheet();
    let version = doc.sym("User").and_then(|u| ds.sheet.section(SecKey { name: u, index: 0 })).and_then(|s| s.row(named(doc, "Version"))).and_then(|r| r.cell(doc.sym("Value")?)).map(|c| text_of(c.value, doc));
    let lang = ds.sheet.cell(doc.sym("DocLangID").unwrap_or_default()).and_then(|c| c.value.num());
    h.check("vdx/docsheet", version.as_deref() == Some("JD16") && lang == Some(1031.0) && ds.styles.line == Some(0), format!("User.Version {version:?}, DocLangID {lang:?}"));
    // pages
    let p0 = page_of(doc, 0);
    let p1 = page_of(doc, 1);
    let layer = p0.and_then(|p| {
        let key = SecKey { name: doc.sym("Layer")?, index: 0 };
        p.sheet.section(key)?.row(RowKey::Index(0))?.cell(doc.sym("NameUniv")?).map(|c| text_of(c.value, doc))
    });
    let pw = p0.and_then(|p| p.sheet.cell(doc.sym("PageWidth")?)).and_then(|c| c.value.num());
    h.check(
        "vdx/pages",
        doc.page_count() == 2
            && p0.is_some_and(|p| p.header.back_page == Some(1) && !p.header.background && p.part.is_none() && p.connects.len() == 2)
            && p1.is_some_and(|p| p.header.background)
            && close(pw, 297.0 / 25.4)
            && layer.as_deref() == Some("Connector"),
        format!("{} pages, width {pw:?}, layer {layer:?}", doc.page_count()),
    );
    // the master
    let m = doc.master_by_id(2).and_then(|m| doc.master(m));
    let icon = m.and_then(|m| m.icon).map(|s| String::from(doc.str(s)));
    let want_icon: String = b64(&sample::icon()).chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let prompt = m.and_then(|m| m.header.attrs.iter().find(|(k, _)| doc.str(*k) == "Prompt")).map(|(_, v)| String::from(doc.str(*v)));
    let cost = m.and_then(|m| {
        let s = m.master_shape()?;
        let d = m.shapes.get(s)?;
        let key = SecKey { name: doc.sym("Property")?, index: 0 };
        let row = d.sheet.section(key)?.row(named(doc, "Cost"))?;
        Some((row.local_name.map(|n| String::from(doc.str(n))), row.cell(doc.sym("Label")?).map(|c| text_of(c.value, doc))))
    });
    h.check(
        "vdx/master",
        doc.masters().count() == 1 && icon.as_deref() == Some(want_icon.as_str()) && prompt.is_some() && cost == Some((Some(String::from("Kosten")), Some(String::from("Kosten")))),
        format!("icon {} chars, prompt {prompt:?}, Cost {cost:?}", icon.as_ref().map_or(0, |i| i.len())),
    );
    // shape 1: the instance
    if let Some(s1) = shape(doc, 0, 1) {
        let pin = cell(doc, s1, "PinX");
        let w = cell(doc, s1, "Width");
        // a currency (Unit CUR) stays text in the model, as in VSDX
        let cost = row_cell(doc, s1, "Property", 0, named(doc, "Cost"), "Value").map(|c| text_of(c.value, doc));
        let text = doc.shape(s1).and_then(|s| s.text()).map(|t| t.chars());
        h.check(
            "vdx/instance",
            doc.shape(s1).is_some_and(|s| s.header().master == Some(2))
                && pin.is_some_and(|c| close(c.value.num(), 60.0 / 25.4) && c.formula == Formula::Absent && c.unit.is_some())
                && w.is_some_and(|c| c.formula == Formula::Inherited && close(c.value.num(), 30.0 / 25.4))
                && cost.as_deref() == Some("12.5")
                && text.as_deref() == Some("Prüfen & weiter"),
            format!("PinX {:?}, Width {:?}, Cost {cost:?}, text {text:?}", pin.map(|c| c.value), w.map(|c| c.formula)),
        );
    } else {
        h.check("vdx/instance", false, String::from("no shape 1"));
    }
    // shape 2: character rows with font names, gone cells left out, tabs, rows by name
    if let Some(s2) = shape(doc, 0, 2) {
        let f0 = row_cell(doc, s2, "Character", 0, RowKey::Index(0), "Font").map(|c| text_of(c.value, doc));
        let f1 = row_cell(doc, s2, "Character", 0, RowKey::Index(1), "Font").map(|c| text_of(c.value, doc));
        let locale = row_cell(doc, s2, "Character", 0, RowKey::Index(0), "Locale");
        let tabs = doc.sym("Tabs").and_then(|t| doc.shape(s2)?.sheet().section(SecKey { name: t, index: 0 }).map(|s| (s.rows.len(), s.rows.first().map_or(0, |r| r.cells.len()))));
        let conn = doc.sym("Connection").and_then(|c| doc.shape(s2)?.sheet().section(SecKey { name: c, index: 0 })?.rows.first().map(|r| r.kind.map(|k| String::from(doc.str(k)))));
        let act = row_cell(doc, s2, "Actions", 0, named(doc, "Row_1"), "Menu").map(|c| text_of(c.value, doc));
        let link = doc.sym("Hyperlink").and_then(|n| doc.shape(s2)?.sheet().section(SecKey { name: n, index: 0 })?.row(named(doc, "Link")).map(|r| r.local_name.map(|l| String::from(doc.str(l)))));
        let user = row_cell(doc, s2, "User", 0, named(doc, "Faktor"), "Value").map(|c| (c.formula.expr().map(|e| String::from(doc.str(e))), c.value.num()));
        let scratch = row_cell(doc, s2, "Scratch", 0, RowKey::Index(0), "X").map(|c| c.formula.expr().map(|e| String::from(doc.str(e))));
        let items = doc.shape(s2).and_then(|s| s.text()).map(|t| t.items.clone()).unwrap_or_default();
        let marks = matches!(items.as_slice(), [TextItem::Cp(0), TextItem::Chars(a), TextItem::Cp(1), TextItem::Chars(b)] if &**a == "Normal " && &**b == "fett und rot");
        let geo = doc.sym("Geometry").and_then(|g| doc.shape(s2)?.sheet().section(SecKey { name: g, index: 0 }).map(|s| (s.rows.len(), s.rows.get(1).and_then(|r| r.kind).map(|k| String::from(doc.str(k))), s.cells.len())));
        h.check(
            "vdx/rows",
            f0.as_deref() == Some("Arial")
                && f1.as_deref() == Some("Times New Roman")
                && locale.is_none()
                && tabs == Some((1, 0))
                && conn == Some(Some(String::from("Connection")))
                && act.as_deref() == Some("Rechteck &markieren")
                && link == Some(Some(String::from("Verweis")))
                && user == Some((Some(String::from("Width/Height")), Some(2.0)))
                && scratch == Some(Some(String::from("Width*0.25")))
                && marks
                && geo == Some((5, Some(String::from("LineTo")), 4)),
            format!("fonts {f0:?} {f1:?}, Locale {}, tabs {tabs:?}, connection {conn:?}, action {act:?}, link {link:?}, user {user:?}, scratch {scratch:?}, text {items:?}, geometry {geo:?}", locale.is_some()),
        );
    } else {
        h.check("vdx/rows", false, String::from("no shape 2"));
    }
    // shape 3: an ellipse row, a field with its cached text kept as XML
    if let Some(s3) = shape(doc, 0, 3) {
        let kind = doc.sym("Geometry").and_then(|g| doc.shape(s3)?.sheet().section(SecKey { name: g, index: 0 })?.row(RowKey::Index(1))?.kind).map(|k| String::from(doc.str(k)));
        let field = row_cell(doc, s3, "Field", 0, RowKey::Index(0), "Format").map(|c| text_of(c.value, doc));
        let items = doc.shape(s3).and_then(|s| s.text()).map(|t| t.items.clone()).unwrap_or_default();
        let fld = items.iter().any(|i| matches!(i, TextItem::Other(x) if doc.str(*x) == "<fld IX='0'>40 mm</fld>"));
        h.check("vdx/ellipse", kind.as_deref() == Some("Ellipse") && field.as_deref() == Some("0.## u") && fld, format!("{kind:?}, format {field:?}, text {items:?}"));
    }
    // the group, the connector
    let group = shape(doc, 0, 4).and_then(|g| doc.shape(g)).map(|g| (g.header().kind, g.children().count()));
    let sub = shape(doc, 0, 6).and_then(|s| doc.shape(s)?.parent()).and_then(|p| doc.shape(p)).map(|p| p.id());
    let begin = shape(doc, 0, 8).and_then(|c| cell(doc, c, "BeginX")).map(|c| (c.formula.expr().map(|e| String::from(doc.str(e))), c.value.num()));
    h.check(
        "vdx/group",
        group == Some((ShapeKind::Group, 2)) && sub == Some(4) && begin.as_ref().is_some_and(|b| b.0.as_deref() == Some("PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))") && close(b.1, 75.0 / 25.4)),
        format!("group {group:?}, parent of 6 {sub:?}, BeginX {begin:?}"),
    );
    // the picture: a part of its own with the PNG's bytes
    let pic = shape(doc, 0, 9).and_then(|s| doc.shape(s)?.data().foreign.as_ref().map(|f| f.payload.clone()));
    let part = match &pic {
        Some(ForeignPayload::Part(n)) => doc.parts.iter().find(|p| p.name == *n),
        _ => None,
    };
    h.check(
        "vdx/picture",
        part.is_some_and(|p| doc.str(p.name) == "/visio/media/image1.png" && doc.str(p.content_type) == "image/png" && p.data == sample::png()),
        format!("{:?}", part.map(|p| (doc.str(p.name), doc.str(p.content_type), p.data.len()))),
    );
    // properties, windows, header and footer
    let p = props::read(doc);
    let custom: Vec<String> = p.custom.iter().map(|c| format!("{}:{}={}", c.name, c.kind, c.value)).collect();
    h.check(
        "vdx/props",
        p.title == "JD16-Gegenprobe"
            && p.subject == "Ältere XML-Formate"
            && p.creator == "JFDiagrammer-Test"
            && p.company == "JFLib"
            && p.manager == "Leitung"
            && p.description.starts_with("Eine von Hand")
            && p.hyperlink_base == "https://example.org/"
            && p.created == "2026-11-23T10:00:00Z"
            && p.modified == "2026-11-23T11:00:00Z"
            && custom == ["_VPID_ALTERNATENAMES:lpwstr=", "BuildNumberCreated:i4=671351309", "BuildNumberEdited:i4=671351309", "TimeEdited:filetime=2026-11-23T10:30:00Z", "Projekt:lpwstr=JD16", "Seiten:r8=2", "IsMetric:bool=true"],
        format!("{} / {} / {custom:?}", p.title, p.created),
    );
    let stencils = diagram_io::template::stencil_windows(doc);
    let win = diagram_io::windows::drawing_window(doc);
    h.check(
        "vdx/windows",
        stencils.len() == 1 && stencils[0].document == "jd16-schablone.vsx" && stencils[0].active && win.as_ref().is_some_and(|w| w.page == Some(0) && w.view_scale == Some(1.0) && w.show_grid == Some(true)),
        format!("{:?}, {win:?}", stencils.iter().map(|s| &s.document).collect::<Vec<_>>()),
    );
    let hf = doc.extras.items().iter().any(|e| matches!(e.item, ExtraItem::Element(x) if doc.str(x).starts_with("<HeaderFooter HeaderFooterColor='#000000'><HeaderMargin Unit='MM'>")));
    h.check("vdx/headerfooter", hf && doc.extras.items().len() == 1, format!("{} extras", doc.extras.items().len()));
    // written as .vsdx and read back: the same content, no warnings, stable bytes
    round_trip(h, "vdx/drawing-vsdx", doc, Format { kind: DocKind::Drawing, macros: false });
    // a template (.vtx) is the same format
    let t = read(&text, DocKind::Template);
    h.check("vdx/template", t.as_ref().is_ok_and(|l| l.doc.kind == DocKind::Template && l.format.kind == DocKind::Template), format!("{:?}", t.map(|l| l.format)));
}

// written in `format` and read back: no warnings, the same content, the second write the same
// bytes
fn round_trip(h: &mut H, name: &str, doc: &Document, format: Format) -> Option<Vec<u8>> {
    let opts = WriteOptions { format: Some(format), ..WriteOptions::DEFAULT };
    let bytes = match vsdx::write(doc, &opts) {
        Ok(b) => b,
        Err(e) => {
            h.check(name, false, format!("write: {e}"));
            return None;
        }
    };
    let back = match vsdx::read(&bytes, &Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            h.check(name, false, format!("read back: {e}"));
            return None;
        }
    };
    let again = vsdx::write(&back.doc, &opts).ok();
    let (a, mut b) = (content(doc), content(&back.doc));
    // the writer gives a document without windows part one of its own (desktop Visio wants it)
    if !a.contains("part \"/visio/windows.xml\"") {
        b = b.lines().filter(|l| !l.starts_with("part \"/visio/windows.xml\"")).map(|l| format!("{l}\n")).collect();
    }
    if a != b {
        if let Some(dir) = plat::arg_value("--dump-dir") {
            let stem = name.replace('/', "-");
            let _ = plat::write_file(&format!("{dir}/{stem}-a.txt"), a.as_bytes());
            let _ = plat::write_file(&format!("{dir}/{stem}-b.txt"), b.as_bytes());
        }
    }
    h.check(
        name,
        back.warnings.is_empty() && back.format == format && a == b && again.as_deref() == Some(bytes.as_slice()),
        format!("{} bytes, {:?} warnings, {}", bytes.len(), back.warnings, if a == b { String::from("same content") } else { first_difference(&a, &b) }),
    );
    Some(bytes)
}

// ---- the stencil and the bitmap

fn stencil(h: &mut H) {
    let l = match read(&sample::stencil(), DocKind::Stencil) {
        Ok(l) => l,
        Err(e) => {
            h.check("vdx/stencil", false, format!("{e}"));
            return;
        }
    };
    let doc = &l.doc;
    let names: Vec<String> = doc.masters().filter_map(|m| doc.master(m)).map(|m| String::from(doc.str(m.header.name.unwrap_or_default()))).collect();
    let arrow_1d = doc.master_by_id(4).and_then(|m| doc.master(m)).and_then(|m| m.master_shape().and_then(|s| m.shapes.get(s))).is_some_and(|d| d.sheet.cell(doc.sym("EndX").unwrap_or_default()).is_some());
    h.check(
        "vdx/stencil",
        l.format.kind == DocKind::Stencil && l.warnings.is_empty() && names == ["Kasten", "Kreis", "Pfeil"] && arrow_1d && doc.page_count() == 1,
        format!("{names:?}, {:?}", l.warnings),
    );
    round_trip(h, "vdx/stencil-vssx", doc, Format { kind: DocKind::Stencil, macros: false });
}

fn bitmap(h: &mut H) {
    let l = read(&sample::bitmap(), DocKind::Drawing);
    let part = l.as_ref().ok().and_then(|l| l.doc.parts.iter().find(|p| l.doc.str(p.content_type) == "image/bmp").map(|p| p.data.clone()));
    let dib = sample::dib();
    // the file header in front: "BM", the size, the pixels after header and no palette
    let ok = part.as_ref().is_some_and(|b| {
        b.len() == dib.len() + 14 && b.starts_with(b"BM") && u32::from_le_bytes([b[2], b[3], b[4], b[5]]) as usize == b.len() && u32::from_le_bytes([b[10], b[11], b[12], b[13]]) == 54 && b[14..] == dib[..]
    });
    h.check("vdx/bitmap", ok, format!("{:?}", part.as_ref().map(|b| b.len())));
    if let Ok(l) = &l {
        let decoded = part.as_deref().and_then(img_core_decode);
        h.check("vdx/bitmap-decodes", decoded == Some((8, 8)), format!("{decoded:?}"));
        round_trip(h, "vdx/bitmap-vsdx", &l.doc, Format { kind: DocKind::Drawing, macros: false });
    }
}

// the size of a BMP as the viewer decodes it
fn img_core_decode(b: &[u8]) -> Option<(u32, u32)> {
    match diagram_view::image::decode(b) {
        diagram_view::image::Decoded::Picture(p) => Some((p.w, p.h)),
        _ => None,
    }
}

// ---- hostile files and limits

fn hostile(h: &mut H) {
    let good = sample::drawing();
    let lim = Limits::VSDX;
    let quick = |h: &mut H, name: &str, text: &str, want: Result<(), Error>, lim: &Limits| {
        h.quick(name, want, || vdx::read(text.as_bytes(), DocKind::Drawing, lim).map(|_| ()));
    };
    let xml = |e| Err(Error::Xml { part: String::from("VDX"), line: 1, error: e });
    quick(h, "vdx/hostile/empty", "", xml(vsdx::XmlError::Syntax), &lim);
    quick(h, "vdx/hostile/doctype", "<!DOCTYPE VisioDocument [<!ENTITY a 'b'>]><VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core'/>", xml(vsdx::XmlError::Unsupported), &lim);
    quick(h, "vdx/hostile/vsdx-root", "<VisioDocument xmlns='http://schemas.microsoft.com/office/visio/2012/main'/>", Err(Error::NotVisio), &lim);
    quick(h, "vdx/hostile/no-namespace", "<VisioDocument/>", Err(Error::NotVisio), &lim);
    quick(h, "vdx/hostile/other-root", "<Pages xmlns='http://schemas.microsoft.com/visio/2003/core'/>", Err(Error::NotVisio), &lim);
    quick(h, "vdx/hostile/minimal", "<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core'/>", Ok(()), &lim);
    // an entity that is not defined, a broken tag, text after the root
    let r = vdx::read(b"<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core'>&x;</VisioDocument>", DocKind::Drawing, &lim);
    h.check("vdx/hostile/entity", matches!(r, Err(Error::Xml { .. })), format!("{:?}", r.map(|_| ())));
    let cut = &good[..good.len() / 2];
    let r = vdx::read(cut.as_bytes(), DocKind::Drawing, &lim);
    h.check("vdx/hostile/cut", r.is_err(), format!("half a file: {:?}", r.map(|_| ())));
    let r = vdx::read(format!("{good}<x/>").as_bytes(), DocKind::Drawing, &lim);
    h.check("vdx/hostile/after-root", r.is_err(), format!("{:?}", r.map(|_| ())));
    // UTF-16 with BOM reads, UTF-32 does not
    let mut u16le = alloc::vec![0xFFu8, 0xFE];
    for u in good.replace("encoding='utf-8'", "encoding='utf-16'").encode_utf16() {
        u16le.extend_from_slice(&u.to_le_bytes());
    }
    let r = vdx::read(&u16le, DocKind::Drawing, &lim);
    h.check("vdx/hostile/utf16", r.as_ref().is_ok_and(|l| l.doc.page_count() == 2) && vdx::sniff(&u16le), format!("{:?}", r.map(|l| l.doc.page_count())));
    let r = vdx::read(&[0xFF, 0xFE, 0, 0, b'<', 0, 0, 0], DocKind::Drawing, &lim);
    h.check("vdx/hostile/utf32", matches!(r, Err(Error::Xml { error: vsdx::XmlError::Unsupported, .. })), format!("{:?}", r.map(|_| ())));
    // structure: two shapes with one ID, a shape without ID, an IX that is no number
    let dup = good.replace("<Shape ID='3' ", "<Shape ID='2' ");
    let r = read(&dup, DocKind::Drawing);
    h.check("vdx/hostile/duplicate-id", matches!(r, Err(Error::Structure { what: "two shapes with one ID", .. })), format!("{:?}", r.map(|_| ())));
    let r = read(&good.replace("<Shape ID='3' ", "<Shape "), DocKind::Drawing);
    h.check("vdx/hostile/no-id", matches!(r, Err(Error::Structure { what: "a Shape without ID", .. })), format!("{:?}", r.map(|_| ())));
    let r = read(&good.replace("<Char IX='1'>", "<Char IX='x'>"), DocKind::Drawing);
    h.check("vdx/hostile/ix", matches!(r, Err(Error::Structure { .. })), format!("{:?}", r.map(|_| ())));
    let r = read(&good.replace("<Page ID='1' ", "<Page ID='0' "), DocKind::Drawing);
    h.check("vdx/hostile/duplicate-page", matches!(r, Err(Error::Structure { what: "two pages with one ID", .. })), format!("{:?}", r.map(|_| ())));
    // cycles: styles, background pages, masters in each other
    let r = read(&good.replace("<StyleSheet ID='0' NameU='No Style' Name='Kein Stil'>", "<StyleSheet ID='0' NameU='No Style' Name='Kein Stil' LineStyle='1'>"), DocKind::Drawing);
    h.check("vdx/hostile/style-cycle", matches!(r, Err(Error::Cycle { what: "style", .. })), format!("{:?}", r.map(|_| ())));
    let r = read(&good.replace("Name='Hintergrund-1' Background='1'>", "Name='Hintergrund-1' Background='1' BackPage='0'>").replace("Name='Seite-1' BackPage='1'", "Name='Seite-1' Background='1' BackPage='1'"), DocKind::Drawing);
    h.check("vdx/hostile/background-cycle", matches!(r, Err(Error::Cycle { what: "background", .. })), format!("{:?}", r.map(|_| ())));
    let st = sample::stencil();
    let in_box = "<Shape ID='5' NameU='Box' Name='Kasten' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>";
    let in_circle = "<Shape ID='5' NameU='Circle' Name='Kreis' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>";
    let cyc = st.replacen(in_box, &format!("{in_box}<Shapes><Shape ID='6' Type='Shape' Master='3'/></Shapes>"), 1).replacen(in_circle, &format!("{in_circle}<Shapes><Shape ID='6' Type='Shape' Master='2'/></Shapes>"), 1);
    let r = read(&cyc, DocKind::Stencil);
    h.check("vdx/hostile/master-cycle", matches!(r, Err(Error::Cycle { what: "master", .. })), format!("{:?}", r.map(|_| ())));
    // references that name nothing: warnings
    let r = read(&good.replace("Master='2'>", "Master='77'>"), DocKind::Drawing);
    h.check("vdx/hostile/missing-master", r.as_ref().is_ok_and(|l| l.warnings.iter().any(|w| matches!(w, Warning::MissingMaster { master: 77, .. }))), format!("{:?}", r.map(|l| l.warnings)));
    // what has no place: left out, warned
    let extra = good.replace(
        "</VisioDocument>",
        "<VBProjectData>bm8gT0xFIGZpbGU=</VBProjectData><SolutionXML Name='x'><a/></SolutionXML><DataConnections/></VisioDocument>",
    );
    let extra = extra.replace("<Title>", "<PreviewPicture Size='4'>AAAAAA==</PreviewPicture><Title>").replace("<Text>A</Text>", "<Text>A</Text><Foo xmlns='urn:x'/>");
    let r = read(&extra, DocKind::Drawing);
    let what: Vec<String> = r.as_ref().map(|l| l.warnings.iter().filter_map(|w| if let Warning::LeftOut { what } = w { Some(what.clone()) } else { None }).collect()).unwrap_or_default();
    h.check(
        "vdx/hostile/left-out",
        r.as_ref().is_ok_and(|l| !l.doc.macros) && what == ["PreviewPicture", "Tab", "Foo", "VBProjectData", "SolutionXML", "DataConnections"],
        format!("{what:?}"),
    );
    // a picture whose base64 is broken or no picture: left out, the shape stays
    let png = b64(&sample::png());
    for (name, bad) in [("vdx/hostile/base64", String::from("@@@@")), ("vdx/hostile/not-a-picture", b64(b"just some text, no picture"))] {
        let r = read(&good.replace(&png, &bad), DocKind::Drawing);
        let ok = r.as_ref().is_ok_and(|l| {
            let s = shape(&l.doc, 0, 9).and_then(|s| l.doc.shape(s));
            s.is_some_and(|s| s.data().foreign.as_ref().is_some_and(|f| f.payload == ForeignPayload::None)) && l.warnings.contains(&Warning::LeftOut { what: String::from("ForeignData") })
        });
        h.check(name, ok, format!("{:?}", r.map(|l| l.warnings)));
    }
    // limits: every count, the formula and text length, the file and a picture
    let small = |f: fn(&mut Limits)| {
        let mut l = Limits::VSDX;
        f(&mut l);
        l
    };
    let cases: [(&str, Limits, &str); 9] = [
        ("shapes", small(|l| l.shapes = 5), "shapes"),
        ("cells", small(|l| l.cells = 100), "cells"),
        ("rows", small(|l| l.rows = 10), "rows"),
        ("sections", small(|l| l.sections = 5), "sections"),
        ("objects", small(|l| l.objects = 3), "objects"),
        ("items", small(|l| l.items = 10), "items"),
        ("formula", small(|l| l.formula_bytes = 20), "formula"),
        ("text", small(|l| l.text_bytes = 10), "text"),
        ("file", small(|l| l.zip.part_bytes = 1000), "file"),
    ];
    for (name, lim, what) in cases {
        quick(h, &format!("vdx/limits/{name}"), &good, Err(Error::Limit(what)), &lim);
    }
    // a picture larger than a part may be: the file limit
    let big = good.replace(&png, &b64(&alloc::vec![0x89u8; 6000]));
    let mut pl = Limits::VSDX;
    pl.zip.part_bytes = good.len() + 6000;
    quick(h, "vdx/limits/picture", &big, Err(Error::Limit("file")), &pl);
    // depth: groups in groups, 100 levels read (no recursion), 300 are past Limits::depth
    for (levels, ok) in [(100usize, true), (300, false)] {
        let mut s = String::from("<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core'><Pages><Page ID='0'><Shapes>");
        for i in 0..levels {
            s.push_str(&format!("<Shape ID='{}' Type='Group'><Shapes>", i + 1));
        }
        for _ in 0..levels {
            s.push_str("</Shapes></Shape>");
        }
        s.push_str("</Shapes></Page></Pages></VisioDocument>");
        let r = read(&s, DocKind::Drawing);
        let good = if ok { r.as_ref().is_ok_and(|l| page_of(&l.doc, 0).is_some_and(|p| p.shapes.iter().count() == levels)) } else { matches!(r, Err(Error::Xml { error: vsdx::XmlError::Depth, .. })) };
        h.check("vdx/limits/depth", good, format!("{levels} levels: {:?}", r.map(|_| ())));
    }
    // many shapes in reasonable time; many warnings: only Limits::warnings kept
    let mut s = String::from("<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core'><Pages><Page ID='0'><Shapes>");
    for i in 0..20000 {
        s.push_str(&format!("<Shape ID='{}' Type='Shape' Master='9'><XForm><PinX>{}</PinX><PinY>1</PinY></XForm><Tabs IX='0'><Tab IX='0'/></Tabs></Shape>", i + 1, i));
    }
    s.push_str("</Shapes></Page></Pages></VisioDocument>");
    let t0 = plat::time_us();
    let r = read(&s, DocKind::Drawing);
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check(
        "vdx/limits/many",
        r.as_ref().is_ok_and(|l| l.warnings.len() == 100 && l.more_warnings == 40000 - 100) && ms < 5000,
        format!("20000 shapes in {ms} ms, {:?} warnings", r.map(|l| (l.warnings.len(), l.more_warnings))),
    );
}

// ---- macro parts

fn macros(h: &mut H) {
    // a .vsdm, .vssm, .vstm: the project passes through unchanged
    for ext in ["vsdm", "vssm", "vstm"] {
        let bytes = crate::samples::file(ext);
        let l = match vsdx::read(&bytes, &Limits::VSDX) {
            Ok(l) => l,
            Err(e) => {
                h.check("vdx/macros", false, format!("{ext}: {e}"));
                continue;
            }
        };
        let format = l.format;
        let out1 = vsdx::write(&l.doc, &WriteOptions::DEFAULT);
        let back = out1.as_ref().ok().and_then(|b| vsdx::read(b, &Limits::VSDX).ok());
        let project = |doc: &Document| doc.parts.iter().find(|p| doc.str(p.name) == "/visio/vbaProject.bin").map(|p| (p.data.clone(), String::from(doc.str(p.content_type))));
        let rel = |doc: &Document| doc.rels.iter().any(|r| doc.str(r.kind).ends_with("/vbaProject") && doc.str(r.target).ends_with("vbaProject.bin"));
        let ok = back.as_ref().is_some_and(|b| b.format == format && format.macros && project(&b.doc) == project(&l.doc) && project(&l.doc).is_some_and(|p| p.0 == [0xCC; 64]) && rel(&b.doc));
        h.check("vdx/macros/pass", ok, format!("{ext}: {format:?} -> {:?}", back.as_ref().map(|b| b.format)));
        // never into a format without macros; without the project the drawing writes
        let plain = Format { kind: format.kind, macros: false };
        let r = vsdx::write(&l.doc, &WriteOptions { format: Some(plain), ..WriteOptions::DEFAULT });
        h.check("vdx/macros/refused", r == Err(WriteError::Macros), format!("{ext} as {}: {:?}", plain.extension(), r.as_ref().map(|b| b.len())));
        let mut doc = l.doc.clone();
        let removed = diagram_io::vsdx::remove_macros(&mut doc);
        let r = vsdx::write(&doc, &WriteOptions { format: Some(plain), ..WriteOptions::DEFAULT });
        let back = r.as_ref().ok().and_then(|b| vsdx::read(b, &Limits::VSDX).ok());
        h.check(
            "vdx/macros/removed",
            removed && !doc.macros && back.is_some_and(|b| !b.format.macros && project(&b.doc).is_none()),
            format!("{ext} without its project as {}", plain.extension()),
        );
    }
    // a .vdx's macro project (an OLE file) becomes vbaProject.bin: written as .vsdm unchanged
    let mut ole = alloc::vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.extend((0..600u32).map(|i| (i * 7 % 251) as u8));
    let text = sample::drawing().replace("</VisioDocument>", &format!("<VBProjectData>{}</VBProjectData></VisioDocument>", b64(&ole)));
    match read(&text, DocKind::Drawing) {
        Ok(l) => {
            let ok = l.format == Format { kind: DocKind::Drawing, macros: true } && l.doc.macros;
            h.check("vdx/macros/vdx", ok, format!("{:?}", l.format));
            if let Some(b) = round_trip(h, "vdx/macros/vdx-vsdm", &l.doc, l.format) {
                let back = vsdx::read(&b, &Limits::VSDX).ok();
                let data = back.as_ref().and_then(|b| b.doc.parts.iter().find(|p| b.doc.str(p.name) == "/visio/vbaProject.bin").map(|p| p.data.clone()));
                h.check("vdx/macros/vdx-bytes", data.as_deref() == Some(ole.as_slice()), format!("{:?}", data.map(|d| d.len())));
            }
            let r = vsdx::write(&l.doc, &WriteOptions { format: Some(Format { kind: DocKind::Drawing, macros: false }), ..WriteOptions::DEFAULT });
            h.check("vdx/macros/vdx-refused", r == Err(WriteError::Macros), String::from("as .vsdx: refused"));
        }
        Err(e) => h.check("vdx/macros/vdx", false, format!("{e}")),
    }
}

// ---- document properties

fn props_checks(h: &mut H) {
    let mut doc = vsdx::new_document(DocKind::Drawing, vsdx::Measure::Metric);
    let p = props::read(&doc);
    h.check("vdx/props/new", p.application == "JFLib" && p.title.is_empty() && p.custom.is_empty(), format!("{p:?}"));
    // unchanged: no part touched
    let before: Vec<Part> = doc.parts.clone();
    props::write(&mut doc, &p);
    h.check("vdx/props/unchanged", doc.parts == before, String::from("the parts keep their bytes"));
    // set and read back, written and read again
    let mut q = p.clone();
    q.title = String::from("Titel <mit> & Zeichen");
    q.creator = String::from("Jemand");
    q.company = String::from("Firma");
    q.manager = String::from("Chefin");
    q.hyperlink_base = String::from("https://example.org/");
    q.created = String::from("2026-11-23T12:00:00Z");
    q.custom.push(Custom { name: String::from("Projekt"), kind: String::from("lpwstr"), value: String::from("JD16") });
    props::write(&mut doc, &q);
    let r = props::read(&doc);
    let bytes = vsdx::write(&doc, &WriteOptions::DEFAULT).ok();
    let back = bytes.as_ref().and_then(|b| vsdx::read(b, &Limits::VSDX).ok()).map(|l| props::read(&l.doc));
    h.check("vdx/props/set", r == q && back.as_ref() == Some(&q), format!("{r:?}"));
    // app.xml keeps what it had
    let app = doc.parts.iter().position(|p| doc.str(p.content_type) == props::CT_APP);
    if let Some(i) = app {
        doc.parts[i].data = Vec::from(
            &b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Template>a.vstx</Template><Application>Microsoft Visio</Application><TitlesOfParts><vt:vector size=\"1\" baseType=\"lpstr\"><vt:lpstr>Seite-1</vt:lpstr></vt:vector></TitlesOfParts><Company>Alt</Company><AppVersion>16.0000</AppVersion></Properties>"[..],
        );
    }
    let mut q2 = props::read(&doc);
    q2.company = String::from("Neu & besser");
    q2.manager = String::from("M");
    props::write(&mut doc, &q2);
    let text = app.map(|i| String::from_utf8_lossy(&doc.parts[i].data).into_owned()).unwrap_or_default();
    h.check(
        "vdx/props/app",
        props::read(&doc) == q2 && text.contains("<TitlesOfParts><vt:vector size=\"1\" baseType=\"lpstr\"><vt:lpstr>Seite-1</vt:lpstr></vt:vector></TitlesOfParts><Company>Neu &amp; besser</Company><AppVersion>16.0000</AppVersion><Manager>M</Manager></Properties>"),
        text,
    );
    // no custom properties: the part goes
    let mut q3 = props::read(&doc);
    q3.custom.clear();
    props::write(&mut doc, &q3);
    h.check("vdx/props/custom-removed", !doc.parts.iter().any(|p| doc.str(p.content_type) == props::CT_CUSTOM), String::from("custom.xml removed"));
    let _ = (CellAddr::single, SheetRef::Document);
}

/// --write-vdx DIR: the self-written files and what JFDiagrammer's writer makes of them
pub fn write(dir: &str) -> i32 {
    let files = [("jd16-zeichnung", sample::drawing(), DocKind::Drawing), ("jd16-schablone", sample::stencil(), DocKind::Stencil), ("jd16-bitmap", sample::bitmap(), DocKind::Drawing)];
    for (stem, text, kind) in files {
        let ext = if kind == DocKind::Stencil { "vsx" } else { "vdx" };
        let path = format!("{dir}/{stem}.{ext}");
        if !plat::write_file(&path, text.as_bytes()) {
            out(&format!("error: cannot write {path}\n"));
            return 1;
        }
        out(&format!("wrote {path}\n"));
        let l = match vdx::read(text.as_bytes(), kind, &Limits::VSDX) {
            Ok(l) => l,
            Err(e) => {
                out(&format!("error: {stem}: {e}\n"));
                return 1;
            }
        };
        let bytes = match vsdx::write(&l.doc, &WriteOptions::DEFAULT) {
            Ok(b) => b,
            Err(e) => {
                out(&format!("error: {stem}: {e}\n"));
                return 1;
            }
        };
        let path = format!("{dir}/{stem}-aus-vdx.{}", l.format.extension());
        if !plat::write_file(&path, &bytes) {
            out(&format!("error: cannot write {path}\n"));
            return 1;
        }
        out(&format!("wrote {path}\n"));
        // the first page of a drawing as JFDiagrammer draws it
        if let Some(page) = l.doc.foreground_pages().next().filter(|_| kind == DocKind::Drawing) {
            let mut fonts = crate::render::fonts();
            let mut v = diagram_view::PageView::new(&l.doc, page, &mut fonts, &diagram_core::render::Options::default());
            let d = v.drawing();
            let w = 1400u32;
            let hh = ((w as f64 * d.height / d.width.max(1e-6)) + 0.5) as u32;
            let img = v.render(w, hh.clamp(16, 4 * w));
            let path = format!("{dir}/{stem}-so-zeichnet-jfdiagrammer.png");
            if plat::write_file(&path, &crate::render::png(&img)) {
                out(&format!("wrote {path}\n"));
            }
        }
    }
    0
}
