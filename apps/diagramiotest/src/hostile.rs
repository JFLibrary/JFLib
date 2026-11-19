// hostile -- VSDX packages that must be refused with the right error, quickly, without a panic
// and without allocating what they claim; controls of the same shape must still read
//
//   xml     broken syntax, entities, DOCTYPE, encodings, wrong roots, depth, attributes
//   limits  shapes, cells, rows, sections, formula and text length; large drawings in time
//   refs    cycles of styles, masters, background pages; references that name nothing (warnings)
//   rels    wrong relationships and content types, parts used twice, macros, malformed IDs
//   zip     ZIP bombs and lying archives through lib/archive

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::ForeignPayload;
use diagram_io::vsdx::{self, Error, Limits, Warning};

use crate::pkg::{self, minimal, shape, Pkg, CT_DRAWING, CT_PAGE, DECL, NS};
use crate::{zipkit, H};

const PAGE: &str = "visio/pages/page1.xml";
const DOC: &str = "visio/document.xml";
const PAGES: &str = "visio/pages/pages.xml";

/// the outcome of reading, short
fn res(b: &[u8], lim: &Limits) -> String {
    match vsdx::read(b, lim) {
        Ok(l) if l.warnings.is_empty() => String::from("ok"),
        Ok(l) => format!("ok+{}", l.warnings.len() + l.more_warnings),
        Err(Error::Xml { error, .. }) => format!("Xml {error:?}"),
        Err(Error::Structure { what, .. }) => format!("Structure: {what}"),
        Err(Error::Limit(w)) => format!("Limit {w}"),
        Err(Error::Cycle { what, .. }) => format!("Cycle {what}"),
        Err(e) => format!("{e:?}"),
    }
}

fn s(x: &str) -> String {
    String::from(x)
}

/// a page part with these contents (PageContents inner XML)
fn page_with(inner: &str) -> Pkg {
    let mut p = minimal("", "");
    p.set(PAGE, format!("{DECL}<PageContents {NS}>{inner}</PageContents>"));
    p
}

/// a minimal drawing with a page part replaced by raw bytes
fn page_raw(bytes: Vec<u8>) -> Pkg {
    let mut p = minimal("", "");
    p.set(PAGE, bytes);
    p
}

pub fn run(h: &mut H) {
    if h.on("xml") {
        xml(h);
    }
    if h.on("limits") {
        limits(h);
    }
    if h.on("refs") {
        refs(h);
    }
    if h.on("rels") {
        rels(h);
    }
    if h.on("zip") {
        zip(h);
    }
}

fn q(h: &mut H, name: &str, p: &Pkg, want: &str) {
    let b = p.bytes();
    h.quick(name, s(want), || res(&b, &Limits::VSDX));
}

// ---- xml

fn xml(h: &mut H) {
    let l = Limits::VSDX;
    q(h, "xml/control", &minimal(&shape(1), ""), "ok");
    q(h, "xml/unclosed", &page_with("<Shapes><Shape ID='1'>"), "Xml Syntax");
    q(h, "xml/mismatch", &page_with("<Shapes></Shape>"), "Xml Syntax");
    q(h, "xml/entity", &page_with("<Shapes><Shape ID='1'><Cell N='A' V='&bogus;'/></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/entity-no-semicolon", &page_with("<Shapes><Shape ID='1'><Cell N='A' V='a &amp b'/></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/char-ref-0", &page_with("<Shapes><Shape ID='1'><Text>&#0;</Text></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/char-ref-surrogate", &page_with("<Shapes><Shape ID='1'><Text>&#xD800;</Text></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/char-ref-huge", &page_with("<Shapes><Shape ID='1'><Text>&#99999999999;</Text></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/char-ref-ok", &page_with("<Shapes><Shape ID='1'><Text>&#x41;&#66;&#x1F600;</Text></Shape></Shapes>"), "ok");
    q(h, "xml/control-char", &page_with("<Shapes><Shape ID='1'><Text>a\u{1}b</Text></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/cdata-end-in-text", &page_with("<Shapes><Shape ID='1'><Text>a]]>b</Text></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/cdata", &page_with("<Shapes><Shape ID='1'><Text><![CDATA[<not markup>]]></Text></Shape></Shapes>"), "ok");
    q(h, "xml/duplicate-attribute", &page_with("<Shapes><Shape ID='1'><Cell N='A' N='B'/></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/unquoted", &page_with("<Shapes><Shape ID=1/></Shapes>"), "Xml Syntax");
    q(h, "xml/no-space-between-attributes", &page_with("<Shapes><Shape ID='1'Type='Shape'/></Shapes>"), "Xml Syntax");
    q(h, "xml/lt-in-attribute", &page_with("<Shapes><Shape ID='1'><Cell N='A' F='1<2'/></Shape></Shapes>"), "Xml Syntax");
    q(h, "xml/unbound-prefix", &page_with("<Shapes><x:Shape ID='1'/></Shapes>"), "Xml Syntax");
    q(h, "xml/rebind-xml-prefix", &page_with("<Shapes xmlns:xml='urn:x'/>"), "Xml Syntax");
    let mut p = minimal("", "");
    p.set(PAGE, format!("{DECL}<PageContents {NS}/><PageContents {NS}/>"));
    q(h, "xml/two-roots", &p, "Xml Syntax");
    p.set(PAGE, format!("{DECL}<PageContents {NS}/>text after"));
    q(h, "xml/text-after-root", &p, "Xml Syntax");
    p.set(PAGE, format!("{DECL}<PageContents {NS}/>  \r\n<!-- a comment --> <?pi x?>"));
    q(h, "xml/misc-after-root", &p, "ok");
    p.set(PAGE, Vec::new());
    q(h, "xml/empty-part", &p, "Xml Syntax");
    p.set(PAGE, format!("<?xml version='1.0' encoding='ISO-8859-1'?><PageContents {NS}/>"));
    q(h, "xml/encoding-latin1", &p, "Xml Unsupported");
    p.set(PAGE, format!("<!-- first --><?xml version='1.0'?><PageContents {NS}/>"));
    q(h, "xml/declaration-not-first", &p, "Xml Syntax");
    let mut bad = format!("{DECL}<PageContents {NS}><Shapes><Shape ID='1' NameU='").into_bytes();
    bad.extend_from_slice(&[0xC3, 0x28]);
    bad.extend_from_slice(b"'/></Shapes></PageContents>");
    q(h, "xml/invalid-utf8", &page_raw(bad), "Xml Unsupported");
    q(h, "xml/utf32", &page_raw(alloc::vec![0xFF, 0xFE, 0, 0, b'<', 0, 0, 0]), "Xml Unsupported");
    let mut odd16 = alloc::vec![0xFF, 0xFE];
    odd16.extend_from_slice(b"<\0P\0x");
    q(h, "xml/utf16-odd", &page_raw(odd16), "Xml Unsupported");
    // DOCTYPE (a billion-laughs entity tree): never expanded, refused as a whole
    let mut p = minimal("", "");
    let laughs = "<!DOCTYPE VisioDocument [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;\">]>";
    p.set(DOC, format!("{DECL}{laughs}<VisioDocument {NS}><DocumentSheet NameU='&c;'/></VisioDocument>"));
    q(h, "xml/doctype", &p, "Xml Unsupported");
    p.set(DOC, format!("{DECL}<!DOCTYPE x SYSTEM 'file:///etc/passwd'><VisioDocument {NS}/>"));
    q(h, "xml/doctype-external", &p, "Xml Unsupported");
    // roots
    let mut p = minimal("", "");
    p.set(PAGE, format!("{DECL}<PageContentz {NS}/>"));
    q(h, "xml/wrong-root", &p, "Structure: another root element");
    p.set(PAGE, format!("{DECL}<PageContents xmlns='urn:not-visio'/>"));
    q(h, "xml/root-other-namespace", &p, "Structure: another root element");
    let mut p = minimal("", "");
    p.set(DOC, format!("{DECL}<Pages {NS}/>"));
    q(h, "xml/document-root", &p, "Structure: another root element");
    // depth: groups and unknown elements far beyond Limits::depth, refused at the limit
    let deep = 100_000;
    let mut x = String::with_capacity(deep * 40);
    for i in 0..deep {
        x.push_str(&format!("<Shape ID='{}' Type='Group'><Shapes>", i + 1));
    }
    let p = page_with(&format!("<Shapes>{x}"));
    q(h, "xml/depth-groups", &p, "Xml Depth");
    // (so regular that it is denser than 200:1: read with the ratio DEFLATE allows)
    let b = page_with(&format!("<Shapes><Shape ID='1'><Cell N='A'>{}</Cell></Shape></Shapes>", "<a>".repeat(1_000_000))).bytes();
    h.quick("xml/depth-unknown", s("Xml Depth"), || res(&b, &loose()));
    // deep, but within the limit: 120 nested groups load, the tree is 120 levels deep
    let n = 120;
    let mut x = String::new();
    for i in 0..n {
        x.push_str(&format!("<Shape ID='{}' Type='Group'><Shapes>", i + 1));
    }
    x.push_str(&shape(9999));
    for _ in 0..n {
        x.push_str("</Shapes></Shape>");
    }
    let b = page_with(&format!("<Shapes>{x}</Shapes>")).bytes();
    let r = vsdx::read(&b, &l);
    let depth = r.as_ref().ok().and_then(|l| {
        let pg = l.doc.pages().next().and_then(|p| l.doc.page(p))?;
        let leaf = pg.shapes.by_id(9999)?;
        let mut d = 0;
        let mut cur = Some(leaf);
        while let Some(c) = cur {
            cur = pg.shapes.parent(c);
            d += 1;
        }
        Some(d)
    });
    h.check("xml/depth-within", depth == Some(n + 1), format!("{depth:?} levels"));
    // the reader does not recurse: 5000 levels of groups with Limits::depth raised, on the main
    // thread's stack (a recursive reader overflowed it at 500)
    let n = 5000;
    let mut x = String::new();
    for i in 0..n {
        x.push_str(&format!("<Shape ID='{}' Type='Group'><Shapes>", i + 1));
    }
    for _ in 0..n {
        x.push_str("</Shapes></Shape>");
    }
    let b = page_with(&format!("<Shapes>{x}</Shapes>")).bytes();
    let deep = Limits { depth: 1_000_000, zip: archive::Limits { ratio: 1032, ..l.zip }, ..l };
    h.quick("xml/depth-no-recursion", s("ok"), || res(&b, &deep));
    // attributes
    let many: String = (0..300).map(|i| format!(" a{i}='1'")).collect();
    q(h, "xml/attributes", &page_with(&format!("<Shapes><Shape ID='1'{many}/></Shapes>")), "Xml Attributes");
    let dup: String = (0..200).map(|i| format!(" a{}='1'", i % 199)).collect();
    q(h, "xml/attributes-duplicate-many", &page_with(&format!("<Shapes><Shape ID='1'{dup}/></Shapes>")), "Xml Syntax");
    // many elements with many attributes: no quadratic duplicate check
    let attrs: String = (0..250).map(|i| format!(" u{i}='{i}'")).collect();
    let cells: String = (0..2000).map(|i| format!("<Cell N='C{i}'{attrs}/>")).collect();
    let b = page_with(&format!("<Shapes><Shape ID='1'>{cells}</Shape></Shapes>")).bytes();
    let t0 = plat::time_us();
    let r = res(&b, &l);
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("xml/attributes-many-elements", r == "ok" && ms < 10_000, format!("{r} for 2000 cells x 250 attributes ({} KB) in {ms} ms", b.len() / 1024));
    // namespace declarations: at most 64 in scope (a name is looked up through them)
    let decls = |n: usize| -> String { (0..n).map(|i| format!("<x:e{i} xmlns:p{i}='urn:{i}' xmlns:x='urn:x'>")).collect() };
    let closes = |n: usize| -> String { (0..n).rev().map(|i| format!("</x:e{i}>")).collect() };
    q(h, "xml/namespaces-in-scope", &page_with(&format!("<Shapes><Shape ID='1'><Cell N='A'>{}{}</Cell></Shape></Shapes>", decls(31), closes(31))), "ok");
    q(h, "xml/namespaces-too-many", &page_with(&format!("<Shapes><Shape ID='1'><Cell N='A'>{}{}</Cell></Shape></Shapes>", decls(40), closes(40))), "Xml Attributes");
    // names, processing instructions, comments, reserved namespaces, character references
    q(h, "xml/name-semicolon", &page_with("<Shapes><Sh;ape ID='1'/></Shapes>"), "Xml Syntax");
    q(h, "xml/name-paren", &page_with("<Shapes><Shape I(D='1'/></Shapes>"), "Xml Syntax");
    q(h, "xml/name-control", &page_with("<Shapes><Sha\u{7}pe ID='1'/></Shapes>"), "Xml Syntax");
    q(h, "xml/pi-xml-target", &page_with("<?XML x?><Shapes/>"), "Xml Syntax");
    q(h, "xml/pi-ok", &page_with("<?probe data?><Shapes/>"), "ok");
    q(h, "xml/comment-dashes", &page_with("<!-- a -- b --><Shapes/>"), "Xml Syntax");
    q(h, "xml/xmlns-xml-own", &page_with("<Shapes xmlns:xml='http://www.w3.org/XML/1998/namespace'/>"), "ok");
    q(h, "xml/bind-xmlns-uri", &page_with("<Shapes xmlns:p='http://www.w3.org/2000/xmlns/'/>"), "Xml Syntax");
    q(h, "xml/char-ref-leading-zeros", &page_with("<Shapes><Shape ID='1'><Text>&#x000000000041;&#0000000000066;</Text></Shape></Shapes>"), "ok");
    // a Rel with a namespace declaration of its own, the strict relationships namespace
    let mut p = minimal(&shape(1), "");
    p.edit(PAGES, "<Rel r:id='rId1'/>", "<Rel xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' r:id='rId1'/>");
    let b = p.bytes();
    let n = vsdx::read(&b, &l).map(|x| (x.doc.pages().next().and_then(|r| x.doc.page(r)).map_or(0, |pg| pg.shapes.len()), x.warnings.len()));
    h.check("xml/rel-with-declaration", n == Ok((1, 0)), format!("{n:?}"));
    let mut p = minimal(&shape(1), "");
    p.edit(PAGES, "xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships'", "xmlns:r='http://purl.oclc.org/ooxml/officeDocument/relationships'");
    let b = p.bytes();
    let n = vsdx::read(&b, &l).map(|x| (x.doc.pages().next().and_then(|r| x.doc.page(r)).map_or(0, |pg| pg.shapes.len()), x.warnings.len()));
    h.check("xml/strict-rel-namespace", n == Ok((1, 0)), format!("{n:?}"));
    // a Rel with content: kept as written, the page has no contents then (two warnings)
    let mut p = minimal(&shape(1), "");
    p.edit(PAGES, "<Rel r:id='rId1'/>", "<Rel r:id='rId1'><x:ext xmlns:x='urn:x'/></Rel>");
    q(h, "xml/rel-with-content", &p, "ok+2");
    // a second Shapes list: kept as an unknown element
    let b = page_with(&format!("<Shapes>{}</Shapes><Shapes>{}</Shapes>", shape(1), shape(2))).bytes();
    let n = vsdx::read(&b, &l).map(|x| {
        let pg = x.doc.pages().next().and_then(|r| x.doc.page(r));
        pg.map_or((0, 0), |pg| (pg.shapes.len(), pg.contents_extras.items().iter().filter(|e| matches!(e.item, diagram_core::ExtraItem::Element(_))).count()))
    });
    h.check("xml/second-shapes", n == Ok((1, 1)), format!("{n:?}"));
}

// ---- limits

fn limits(h: &mut H) {
    let small = Limits { shapes: 100, cells: 1000, rows: 10, sections: 10, formula_bytes: 1000, text_bytes: 1000, warnings: 5, ..Limits::VSDX };
    let shapes = |n: u32| -> String { (1..=n).map(shape).collect() };
    let b = minimal(&shapes(100), "").bytes();
    h.quick("limits/shapes-at", s("ok"), || res(&b, &small));
    let b = minimal(&shapes(101), "").bytes();
    h.quick("limits/shapes", s("Limit shapes"), || res(&b, &small));
    let cells: String = (0..1001).map(|i| format!("<Cell N='C{i}' V='1'/>")).collect();
    let b = minimal(&format!("<Shape ID='1'>{cells}</Shape>"), "").bytes();
    h.quick("limits/cells", s("Limit cells"), || res(&b, &small));
    let rows: String = (0..11).map(|i| format!("<Row IX='{i}'/>")).collect();
    let b = minimal(&format!("<Shape ID='1'><Section N='Geometry'>{rows}</Section></Shape>"), "").bytes();
    h.quick("limits/rows", s("Limit rows"), || res(&b, &small));
    let secs: String = (0..11).map(|_| String::from("<Section N='Geometry'/>")).collect();
    let b = minimal(&format!("<Shape ID='1'>{secs}</Shape>"), "").bytes();
    h.quick("limits/sections", s("Limit sections"), || res(&b, &small));
    let f = "1+".repeat(500) + "1";
    let b = minimal(&format!("<Shape ID='1'><Cell N='A' V='1' F='{f}'/></Shape>"), "").bytes();
    h.quick("limits/formula", s("Limit formula"), || res(&b, &small));
    let v = "x".repeat(1001);
    let b = minimal(&format!("<Shape ID='1'><Cell N='A' V='{v}' U='STR'/></Shape>"), "").bytes();
    h.quick("limits/value", s("Limit formula"), || res(&b, &small));
    let t = "y".repeat(1001);
    let b = minimal(&format!("<Shape ID='1'><Text>{t}</Text></Shape>"), "").bytes();
    h.quick("limits/text", s("Limit text"), || res(&b, &small));
    let t = "<cp IX='0'/>".repeat(100);
    let b = minimal(&format!("<Shape ID='1'><Text>{t}</Text></Shape>"), "").bytes();
    h.quick("limits/text-marks", s("Limit text"), || res(&b, &small));
    // warnings: at most Limits::warnings kept, the rest counted
    let dangling: String = (1..=20).map(|i| format!("<Shape ID='{i}' Master='99'/>")).collect();
    let b = minimal(&dangling, "").bytes();
    let r = vsdx::read(&b, &small);
    let n = r.as_ref().map(|l| (l.warnings.len(), l.more_warnings)).unwrap_or((0, 0));
    h.check("limits/warnings", n == (5, 15), format!("{n:?}"));
    // pages, masters and styles; everything else kept (connects, unknown items ...)
    let few = Limits { objects: 10, items: 50, ..Limits::VSDX };
    let mut p = minimal("", "");
    p.set(DOC, styles_doc(&(0..11).map(|i| format!("<StyleSheet ID='{i}'/>")).collect::<String>()));
    let b = p.bytes();
    h.quick("limits/objects", s("Limit objects"), || res(&b, &few));
    let unknown: String = (0..60).map(|i| format!(" u{i}='1'")).collect();
    let b = minimal(&format!("<Shape ID='1'{unknown}/>"), "").bytes();
    h.quick("limits/items-attributes", s("Limit items"), || res(&b, &few));
    let connects: String = (0..60).map(|_| String::from("<Connect FromSheet='1' ToSheet='1'/>")).collect();
    let b = minimal(&shape(1), &connects).bytes();
    h.quick("limits/items-connects", s("Limit items"), || res(&b, &few));
    // floods with the default limits: empty pages, dangling connects (warnings kept to 100)
    let mut p = minimal("", "");
    let pages: String = (0..200_000).map(|i| format!("<Page ID='{i}'/>")).collect();
    p.set(PAGES, format!("{DECL}<Pages {NS}>{pages}</Pages>"));
    p.unrel(PAGES, "rId1");
    p.drop(PAGE);
    let b = p.bytes();
    let t0 = plat::time_us();
    let r = res(&b, &loose());
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("limits/page-flood", r == "Limit objects" && ms < 10_000, format!("{r} for 200000 empty pages in {ms} ms"));
    let dangling: String = (0..50_000).map(|i| format!("<Connect FromSheet='{}' ToSheet='{}'/>", 100_000 + i, 200_000 + i)).collect();
    let b = minimal(&shape(1), &dangling).bytes();
    let t0 = plat::time_us();
    let r = vsdx::read(&b, &loose());
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    let n = r.as_ref().map(|x| (x.warnings.len(), x.more_warnings)).map_err(|e| e.clone());
    h.check("limits/warning-flood", n == Ok((100, 99_900)) && ms < 10_000, format!("{n:?} in {ms} ms"));
    // many content type defaults: archive::opc's limits (30000 elements) and its linear search
    let mut p = minimal(&shape(1), "");
    let mut types = format!("{DECL}<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Default Extension='rels' ContentType='application/vnd.openxmlformats-package.relationships+xml'/><Default Extension='xml' ContentType='application/xml'/>");
    for i in 0..29_000 {
        types.push_str(&format!("<Default Extension='e{i}' ContentType='a/b'/>"));
    }
    for (n, t) in [(DOC, CT_DRAWING), (PAGES, pkg::CT_PAGES), (PAGE, CT_PAGE)] {
        types.push_str(&format!("<Override PartName='/{n}' ContentType='{t}'/>"));
    }
    types.push_str("</Types>");
    p.types_override = Some(types.clone());
    let b = p.bytes();
    let t0 = plat::time_us();
    let r = res(&b, &Limits::VSDX);
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("limits/content-type-defaults", r == "ok" && ms < 10_000, format!("{r} for 29000 defaults in {ms} ms"));
    let more = types.replacen("</Types>", &(29_000..31_000).map(|i| format!("<Default Extension='e{i}' ContentType='a/b'/>")).collect::<String>(), 1) + "</Types>";
    p.types_override = Some(more);
    let b = p.bytes();
    h.quick("limits/content-type-defaults-over", s("Package(Xml(Limit))"), || res(&b, &Limits::VSDX));
    // large drawings within the default limits, in reasonable time (no quadratic step)
    timed(h, "limits/100k-shapes", &minimal(&shapes(100_000), ""), 100_000, |l| {
        l.doc.pages().next().and_then(|p| l.doc.page(p)).map_or(0, |p| p.shapes.len())
    });
    let cells: String = (0..300_000).map(|i| format!("<Cell N='C{}' V='{i}'/>", i % 5000)).collect();
    timed(h, "limits/300k-cells-one-sheet", &minimal(&format!("<Shape ID='1'>{cells}</Shape>"), ""), 300_000, |l| {
        l.doc.pages().next().and_then(|p| l.doc.page(p)).and_then(|p| p.shapes.get(p.shapes.top()[0])).map_or(0, |d| d.sheet.cells.len())
    });
    let secs: String = (0..100_000).map(|_| String::from("<Section N='Geometry'><Row T='MoveTo'/></Section>")).collect();
    timed(h, "limits/100k-sections-one-sheet", &minimal(&format!("<Shape ID='1'>{secs}</Shape>"), ""), 99_999, |l| {
        l.doc.pages().next().and_then(|p| l.doc.page(p)).and_then(|p| p.shapes.get(p.shapes.top()[0])).and_then(|d| d.sheet.sections.last()).map_or(0, |s| s.index as usize)
    });
    let connects: String = (0..50_000).map(|i| format!("<Connect FromSheet='1' ToSheet='{}' FromCell='BeginX' ToCell='PinX'/>", 2 + i % 2)).collect();
    timed(h, "limits/50k-connects", &minimal(&format!("{}{}{}", shape(1), shape(2), shape(3)), &connects), 50_000, |l| {
        l.doc.pages().next().and_then(|p| l.doc.page(p)).map_or(0, |p| p.connects.len())
    });
    // 2000 pages, each a part of its own
    let mut p = minimal("", "");
    let mut list = String::new();
    for i in 0..2000u32 {
        list.push_str(&format!("<Page ID='{i}' NameU='Page-{i}'><Rel r:id='rId{i}'/></Page>"));
        p.rel(PAGES, &format!("rId{i}"), "page", &format!("p{i}.xml"));
        p.part(&format!("visio/pages/p{i}.xml"), CT_PAGE, format!("<PageContents {NS}><Shapes>{}</Shapes></PageContents>", shape(1)));
    }
    p.unrel(PAGES, "rId1");
    p.rel(PAGES, "rId1", "page", "p1.xml");
    p.drop(PAGE);
    p.set(PAGES, format!("{DECL}<Pages {NS}>{list}</Pages>"));
    timed(h, "limits/2000-pages", &p, 2000, |l| l.doc.page_count());
}

/// Limits::VSDX with the ratio DEFLATE allows: the generated drawings repeat one element and are
/// denser than the 200:1 a real file reaches
fn loose() -> Limits {
    Limits { zip: archive::Limits { ratio: 1032, ..Limits::VSDX.zip }, ..Limits::VSDX }
}

fn timed(h: &mut H, name: &str, p: &Pkg, want: usize, count: impl Fn(&vsdx::Loaded) -> usize) {
    let b = p.bytes();
    let t0 = plat::time_us();
    let r = vsdx::read(&b, &loose());
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    let n = r.as_ref().map(&count);
    h.check(name, n.as_ref().is_ok_and(|&n| n == want) && ms < 20_000, format!("{:?} ({} KB packed) in {ms} ms", n, b.len() / 1024));
}

// ---- refs

fn styles_doc(sheets: &str) -> String {
    format!("{DECL}<VisioDocument {NS}><StyleSheets>{sheets}</StyleSheets></VisioDocument>")
}

/// a drawing with masters (masters.xml inner XML, the master parts' Shapes inner XML)
fn with_masters(masters: &[(u32, &str)], page_shapes: &str) -> Pkg {
    let mut p = minimal(page_shapes, "");
    let mut list = String::new();
    for (i, (id, shapes)) in masters.iter().enumerate() {
        list.push_str(&format!("<Master ID='{id}' NameU='M{id}'><Rel r:id='rId{}'/></Master>", i + 1));
        p.rel("visio/masters/masters.xml", &format!("rId{}", i + 1), "master", &format!("master{}.xml", i + 1));
        p.part(&format!("visio/masters/master{}.xml", i + 1), pkg::CT_MASTER, format!("{DECL}<MasterContents {NS}><Shapes>{shapes}</Shapes></MasterContents>"));
    }
    p.rel(DOC, "rId2", "masters", "masters/masters.xml");
    p.part("visio/masters/masters.xml", pkg::CT_MASTERS, format!("{DECL}<Masters {NS}>{list}</Masters>"));
    p
}

fn refs(h: &mut H) {
    let mut p = minimal(&shape(1), "");
    p.set(DOC, styles_doc("<StyleSheet ID='0'/><StyleSheet ID='3' LineStyle='4'/><StyleSheet ID='4' LineStyle='3'/>"));
    q(h, "refs/style-cycle", &p, "Cycle style");
    p.set(DOC, styles_doc("<StyleSheet ID='0'/><StyleSheet ID='3' FillStyle='4'/><StyleSheet ID='4' FillStyle='5'/><StyleSheet ID='5' FillStyle='3'/>"));
    q(h, "refs/style-cycle-fill", &p, "Cycle style");
    p.set(DOC, styles_doc("<StyleSheet ID='0' LineStyle='0' FillStyle='0' TextStyle='0'/><StyleSheet ID='3' LineStyle='0'/>"));
    q(h, "refs/style-self", &p, "ok");
    p.set(DOC, styles_doc("<StyleSheet ID='0'/><StyleSheet ID='0'/>"));
    q(h, "refs/style-duplicate", &p, "Structure: two styles with one ID");
    // a long chain is no cycle
    let chain: String = (0..5000).map(|i| format!("<StyleSheet ID='{}' LineStyle='{}'/>", i + 1, i)).collect();
    p.set(DOC, styles_doc(&format!("<StyleSheet ID='0'/>{chain}")));
    q(h, "refs/style-chain", &p, "ok");
    // masters through instances
    q(h, "refs/master-cycle", &with_masters(&[(2, "<Shape ID='5' Master='3'/>"), (3, "<Shape ID='5' Master='2'/>")], ""), "Cycle master");
    q(h, "refs/master-self", &with_masters(&[(2, "<Shape ID='5' Type='Group'><Shapes><Shape ID='6' Master='2'/></Shapes></Shape>")], ""), "Cycle master");
    q(h, "refs/master-nested-ok", &with_masters(&[(2, "<Shape ID='5' Master='3'/>"), (3, "<Shape ID='5'/>")], "<Shape ID='1' Master='2'/>"), "ok");
    let ring: Vec<(u32, String)> = (0..300u32).map(|i| (i + 2, format!("<Shape ID='5' Master='{}'/>", 2 + (i + 1) % 300))).collect();
    let ring: Vec<(u32, &str)> = ring.iter().map(|(i, s)| (*i, s.as_str())).collect();
    q(h, "refs/master-ring-300", &with_masters(&ring, ""), "Cycle master");
    // background pages
    let mut p = minimal("", "");
    p.set(PAGES, format!("{DECL}<Pages {NS}><Page ID='0' BackPage='3'><Rel r:id='rId1'/></Page><Page ID='3' Background='1' BackPage='0'/></Pages>"));
    q(h, "refs/background-cycle", &p, "Cycle background");
    p.set(PAGES, format!("{DECL}<Pages {NS}><Page ID='0' Background='1' BackPage='0'><Rel r:id='rId1'/></Page></Pages>"));
    q(h, "refs/background-self", &p, "Cycle background");
    p.set(PAGES, format!("{DECL}<Pages {NS}><Page ID='0' NameU='A'><Rel r:id='rId1'/></Page><Page ID='0' NameU='B'/></Pages>"));
    q(h, "refs/page-duplicate", &p, "Structure: two pages with one ID");
    // references that name nothing: kept, reported
    let mut p = with_masters(&[(2, "<Shape ID='5'/>")], "<Shape ID='1' Master='99'/><Shape ID='2' LineStyle='77'/><Shape ID='3' Master='2' Type='Group'><Shapes><Shape ID='4' MasterShape='42'/></Shapes></Shape>");
    p.edit(PAGE, "</Connects>", "<Connect FromSheet='1' ToSheet='55'/></Connects>");
    p.edit(PAGES, "<Page ID='0' NameU='Page-1'>", "<Page ID='0' NameU='Page-1' BackPage='12'>");
    p.edit("visio/masters/masters.xml", "</Masters>", "<Master ID='7' NameU='Empty'/></Masters>");
    let b = p.bytes();
    let r = vsdx::read(&b, &Limits::VSDX);
    let w = r.as_ref().map(|l| l.warnings.clone()).unwrap_or_default();
    let has = |f: &dyn Fn(&Warning) -> bool| w.iter().any(f);
    let ok = has(&|x| matches!(x, Warning::MissingMaster { shape: 1, master: 99, .. }))
        && has(&|x| matches!(x, Warning::MissingStyle { style: 77, .. }))
        && has(&|x| matches!(x, Warning::MissingMasterShape { shape: 4, master_shape: 42, .. }))
        && has(&|x| matches!(x, Warning::MissingShape { shape: 55, .. }))
        && has(&|x| matches!(x, Warning::MissingBackPage { page: 0, back_page: 12 }))
        && has(&|x| matches!(x, Warning::NoContents { what: "master", id: 7 }))
        && w.len() == 6;
    h.check("refs/dangling", ok, format!("{w:?}"));
    // a picture outside the package: kept as a relationship, never fetched
    let mut p = minimal("<Shape ID='1' Type='Foreign'><ForeignData ForeignType='Bitmap'><Rel r:id='rId7'/></ForeignData></Shape>", "");
    p.rel_external(PAGE, "rId7", "image", "file:///C:/secret.png");
    let b = p.bytes();
    let r = vsdx::read(&b, &Limits::VSDX);
    let ok = r.as_ref().is_ok_and(|l| {
        let pg = l.doc.pages().next().and_then(|x| l.doc.page(x));
        let fd = pg.and_then(|pg| pg.shapes.get(pg.shapes.top()[0])).and_then(|d| d.foreign.as_deref());
        fd.is_some_and(|f| f.payload == ForeignPayload::None && f.extras.items().len() == 1)
            && matches!(l.warnings.as_slice(), [Warning::ExternalPicture { shape: 1, .. }])
            && l.doc.rels.iter().any(|r| r.external && l.doc.str(r.target) == "file:///C:/secret.png")
    });
    h.check("refs/external-picture", ok, format!("{:?}", r.as_ref().map(|l| l.warnings.clone()).map_err(|e| e.clone())));
}

// ---- rels

fn rels(h: &mut H) {
    let base = || minimal(&shape(1), "");
    let mut p = base();
    p.edit(PAGES, "r:id='rId1'", "r:id='rId9'");
    q(h, "rels/unknown-rid", &p, "Structure: an r:id that names no relationship");
    let mut p = base();
    p.rel(PAGES, "rId2", "master", "page1.xml");
    p.edit(PAGES, "r:id='rId1'", "r:id='rId2'");
    q(h, "rels/rid-of-other-kind", &p, "Structure: an r:id that names a relationship of another kind");
    let mut p = base();
    p.part(PAGES, "application/xml", p.data(PAGES));
    q(h, "rels/pages-wrong-content-type", &p, "Structure: a part of the wrong content type for its relationship");
    let mut p = base();
    p.part(PAGE, pkg::CT_MASTER, p.data(PAGE));
    q(h, "rels/page-wrong-content-type", &p, "Structure: a part of the wrong content type for its relationship");
    let mut p = base();
    p.rel(DOC, "rId2", "pages", "pages/pages.xml");
    q(h, "rels/two-pages-parts", &p, "Structure: more than one relationship of a kind that is single");
    let mut p = base();
    p.rel(PAGES, "rId2", "page", "page1.xml");
    p.edit(PAGES, "</Pages>", "<Page ID='1'><Rel r:id='rId2'/></Page></Pages>");
    q(h, "rels/part-used-twice", &p, "Structure: a part used twice");
    let mut p = base();
    p.rel(PAGES, "rId2", "page", "../../visio/document.xml");
    p.edit(PAGES, "</Pages>", "<Page ID='1'><Rel r:id='rId2'/></Page></Pages>");
    q(h, "rels/page-is-document", &p, "Structure: a part of the wrong content type for its relationship");
    let p = minimal("<Shape ID='1' Type='Foreign'><ForeignData><Rel r:id='rId5'/></ForeignData></Shape>", "");
    q(h, "rels/foreign-unknown-rid", &p, "Structure: an r:id that names no relationship");
    let mut p = base();
    p.rel(DOC, "rId3", "theme", "theme/theme1.xml");
    p.part("visio/theme/theme1.xml", "application/xml", "<a/>");
    q(h, "rels/theme-not-a-theme", &p, "Structure: a theme relationship to a part that is no theme");
    let mut p = base();
    p.rel(DOC, "rId3", "windows", "missing.xml");
    q(h, "rels/missing-target", &p, "Package(Invalid)");
    let mut p = base();
    p.unrel("", "rId1");
    p.rel_external("", "rId1", "document", "https://example.org/document.xml");
    q(h, "rels/external-main", &p, "Structure: an external main part");
    let mut p = base();
    p.unrel("", "rId1");
    p.rel_external("", "rId1", "officeDocument", "https://example.org/document.xml");
    q(h, "rels/external-office-main", &p, "Package(Invalid)");
    let mut p = base();
    p.unrel("", "rId1");
    p.rel("", "rId1", "officeDocument", "visio/document.xml");
    q(h, "rels/office-document-main", &p, "ok");
    let mut p = base();
    p.rel("", "rId2", "document", "visio/pages/pages.xml");
    q(h, "rels/two-main-parts", &p, "Structure: more than one main part");
    let mut p = base();
    p.unrel("", "rId1");
    p.rel("", "rId1", "core-properties", "visio/document.xml");
    q(h, "rels/no-main", &p, "NotVisio");
    let mut p = base();
    p.part(DOC, "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", p.data(DOC));
    q(h, "rels/docx-main", &p, "NotVisio");
    let mut p = Pkg::new();
    p.rel("", "rId1", "document", DOC);
    p.part(DOC, CT_DRAWING, format!("{DECL}<VisioDocument {NS}/>"));
    q(h, "rels/no-pages", &p, "ok");
    // an unused page part: kept as a part, reported
    let mut p = base();
    p.rel(PAGES, "rId2", "page", "page2.xml");
    p.part("visio/pages/page2.xml", CT_PAGE, format!("<PageContents {NS}/>"));
    q(h, "rels/unused-page-part", &p, "ok+1");
    // macros: only in a macro-enabled file
    let mut p = base();
    p.part("visio/vbaProject.bin", "application/vnd.ms-office.vbaProject", alloc::vec![1u8; 16]);
    p.rel(DOC, "rId7", "vbaProject", "vbaProject.bin");
    q(h, "rels/macros-in-vsdx", &p, "Macros");
    let mut p = base();
    p.part("visio/vbaProject.bin", "application/octet-stream", alloc::vec![1u8; 16]);
    q(h, "rels/macro-part-name-in-vsdx", &p, "Macros");
    let mut p = base();
    p.part("visio/x.bin", "application/vnd.ms-office.vbaProject", alloc::vec![1u8; 16]);
    q(h, "rels/macro-content-type-in-vsdx", &p, "Macros");
    let mut p = base();
    p.part("visio/vbaProject.bin", "application/vnd.ms-office.vbaProject", alloc::vec![1u8; 16]);
    p.rel(DOC, "rId7", "vbaProject", "vbaProject.bin");
    p.part(DOC, "application/vnd.ms-visio.drawing.macroEnabled.main+xml", p.data(DOC));
    q(h, "rels/macros-in-vsdm", &p, "ok");
    let mut p = base();
    p.part("visio/other.xml", "application/vnd.ms-visio.drawing.macroEnabled.main+xml", "<x/>");
    q(h, "rels/macro-main-type-elsewhere", &p, "Macros");
    // malformed IDs and attributes
    for (name, shapes, want) in [
        ("rels/id-letters", "<Shape ID='x1'/>", "Structure: a Shape ID that is no number"),
        ("rels/id-negative", "<Shape ID='-1'/>", "Structure: a Shape ID that is no number"),
        ("rels/id-overflow", "<Shape ID='99999999999'/>", "Structure: a Shape ID that is no number"),
        ("rels/id-space", "<Shape ID=' 1'/>", "Structure: a Shape ID that is no number"),
        ("rels/id-missing", "<Shape Type='Shape'/>", "Structure: a Shape without ID"),
        ("rels/id-duplicate", "<Shape ID='1'/><Shape ID='1'/>", "Structure: two shapes with one ID"),
        ("rels/id-duplicate-in-group", "<Shape ID='1' Type='Group'><Shapes><Shape ID='1'/></Shapes></Shape>", "Structure: two shapes with one ID"),
        ("rels/cell-without-n", "<Shape ID='1'><Cell V='1'/></Shape>", "Structure: a Cell without N"),
        ("rels/section-without-n", "<Shape ID='1'><Section IX='0'/></Shape>", "Structure: a Section without N"),
        ("rels/ix-letters", "<Shape ID='1'><Section N='Geometry' IX='a'/></Shape>", "Structure: a Section IX that is no number"),
        ("rels/del-maybe", "<Shape ID='1' Del='maybe'/>", "Structure: a Shape Del that is no flag"),
        ("rels/master-letters", "<Shape ID='1' Master='two'/>", "Structure: a Master that is no number"),
    ] {
        q(h, name, &minimal(shapes, ""), want);
    }
    q(h, "rels/connect-without-to", &minimal(&shape(1), "<Connect FromSheet='1'/>"), "Structure: a Connect without FromSheet or ToSheet");
    q(h, "rels/connect-part-letters", &minimal(&shape(1), "<Connect FromSheet='1' ToSheet='1' ToPart='x'/>"), "Structure: a ToPart that is no number");
    let mut p = base();
    p.edit(PAGES, "<Page ID='0'", "<Page ID='zero'");
    q(h, "rels/page-id-letters", &p, "Structure: a Page ID that is no number");
    let mut p = base();
    p.edit(PAGES, "<Page ID='0' NameU='Page-1'>", "<Page NameU='Page-1'>");
    q(h, "rels/page-without-id", &p, "Structure: a Page without ID");
    let mut p = with_masters(&[(2, "<Shape ID='5'/>"), (2, "<Shape ID='5'/>")], "");
    q(h, "rels/master-duplicate", &p, "Structure: two masters with one ID");
    p.drop("[Content_Types].xml");
    let mut entries = p.entries();
    entries.retain(|e| e.name != "[Content_Types].xml");
    let b = archive::write(&entries, archive::Limits::default()).unwrap_or_default();
    h.quick("rels/no-content-types", s("Package(Invalid)"), || res(&b, &Limits::VSDX));
}

// ---- zip

fn zip(h: &mut H) {
    let l = Limits::VSDX;
    // a DEFLATE bomb: 64 MiB of spaces in a page part, far denser than 200:1
    let mut p = minimal("", "");
    let mut big = format!("{DECL}<PageContents {NS}>").into_bytes();
    big.resize(big.len() + (64 << 20), b' ');
    big.extend_from_slice(b"</PageContents>");
    p.set(PAGE, big);
    let b = p.bytes();
    h.quick("zip/bomb-ratio", s("Package(Zip(Limit))"), || res(&b, &l));
    let loose = Limits { zip: archive::Limits { ratio: 1032, ..l.zip }, ..l };
    let t0 = plat::time_us();
    let r = res(&b, &loose);
    h.check("zip/bomb-ratio-control", r == "ok", format!("{r} with ratio 1032 ({} KB packed, {} ms)", b.len() / 1024, plat::time_us().wrapping_sub(t0) / 1000));
    // part, total and archive sizes, number of parts
    let p = pkg::sample(CT_DRAWING);
    let b = p.bytes();
    let tight = |z: archive::Limits| Limits { zip: z, ..l };
    h.quick("zip/entries", s("Package(Zip(Limit))"), || res(&b, &tight(archive::Limits { entries: 10, ..l.zip })));
    h.quick("zip/part-bytes", s("Package(Zip(Limit))"), || res(&b, &tight(archive::Limits { part_bytes: 1000, ..l.zip })));
    h.quick("zip/total-bytes", s("Package(Zip(Limit))"), || res(&b, &tight(archive::Limits { total_bytes: 4000, ..l.zip })));
    h.quick("zip/archive-bytes", s("Package(Zip(Limit))"), || res(&b, &tight(archive::Limits { archive_bytes: 1000, ..l.zip })));
    h.quick("zip/control", s("ok"), || res(&b, &l));
    // lying archives (the JD1 builder): a size no record has, central records sharing one local
    // record, a wrong CRC, a cut archive, no archive at all
    let entries = minimal(&shape(1), "").entries();
    let mut recs: Vec<zipkit::Rec> = entries.iter().map(|e| zipkit::rec(&e.name, &e.data, true)).collect();
    let good = zipkit::build(&recs, None);
    h.quick("zip/kit-control", s("ok"), || res(&good, &l));
    let last = recs.len() - 1;
    recs[last].size = 3 << 30;
    let b = zipkit::build(&recs, None);
    h.quick("zip/size-lie", true, || res(&b, &l).starts_with("Package(Zip("));
    let mut recs: Vec<zipkit::Rec> = entries.iter().map(|e| zipkit::rec(&e.name, &e.data, true)).collect();
    let quoted: Vec<usize> = (0..recs.len()).chain(core::iter::repeat_n(last, 2000)).collect();
    let b = zipkit::build(&recs, Some(&quoted));
    h.quick("zip/overlap-bomb", s("Package(Zip(Invalid))"), || res(&b, &l));
    recs[last].crc ^= 1;
    let b = zipkit::build(&recs, None);
    h.quick("zip/crc", s("Package(Zip(Checksum))"), || res(&b, &l));
    let cut = &good[..good.len() / 2];
    h.quick("zip/cut", true, || res(cut, &l).starts_with("Package(Zip("));
    h.quick("zip/empty", s("Package(Zip(Invalid))"), || res(&[], &l));
    let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    h.quick("zip/noise", s("Package(Zip(Invalid))"), || res(&noise, &l));
}
