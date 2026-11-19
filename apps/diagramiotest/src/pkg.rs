// pkg -- a small builder of VSDX packages for the tests: parts, content types and relationships
// written as given, also when they break the rules (the hostile tests need that)
//
//   let mut p = Pkg::new();
//   p.part("visio/document.xml", CT_DOC, xml);       a part with its content type (an override)
//   p.rel("", "rId1", "document", "visio/document.xml");   a relationship of a source
//   p.bytes() -> the ZIP (DEFLATE), [Content_Types].xml and the .rels parts made from the lists
//   p.edit(name, from, to) / set(name, data) / drop(name)   change a part afterwards
//
// sample() is a drawing written for these tests (generic names, nothing taken from a real file):
// styles, a document sheet, two pages (one a background page), a master with an icon, groups, a
// connector with Connect entries, text with marks, pictures in a part and inline, a theme,
// windows.xml, properties, comments, data recordsets, a solution, an unknown and an orphan part,
// an external relationship, and unknown attributes, elements and text in many places.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use archive::{Entry, Limits, Method};

pub const NS: &str = "xmlns='http://schemas.microsoft.com/office/visio/2012/main' \
                      xmlns:r='http://schemas.openxmlformats.org/officeDocument/2006/relationships' xml:space='preserve'";
pub const DECL: &str = "<?xml version='1.0' encoding='utf-8' ?>\r\n";
pub const CT_DRAWING: &str = "application/vnd.ms-visio.drawing.main+xml";
pub const CT_PAGES: &str = "application/vnd.ms-visio.pages+xml";
pub const CT_PAGE: &str = "application/vnd.ms-visio.page+xml";
pub const CT_MASTERS: &str = "application/vnd.ms-visio.masters+xml";
pub const CT_MASTER: &str = "application/vnd.ms-visio.master+xml";
pub const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
pub const CT_WINDOWS: &str = "application/vnd.ms-visio.windows+xml";

/// the full type URI of a short relationship name
pub fn rel_type(short: &str) -> String {
    match short {
        "officeDocument" | "theme" | "image" | "extended-properties" | "custom-properties" | "hyperlink" | "oleObject" => {
            format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{short}")
        }
        "core-properties" | "thumbnail" => format!("http://schemas.openxmlformats.org/package/2006/relationships/metadata/{short}"),
        "vbaProject" => String::from("http://schemas.microsoft.com/office/2006/relationships/vbaProject"),
        s if s.contains(':') => String::from(s),
        s => format!("http://schemas.microsoft.com/visio/2010/relationships/{s}"),
    }
}

struct Rel {
    source: String,
    id: String,
    kind: String,
    target: String,
    external: bool,
}

/// a package to build
pub struct Pkg {
    parts: Vec<(String, String, Vec<u8>)>,
    rels: Vec<Rel>,
    defaults: Vec<(String, String)>,
    /// the content types part as written instead of the generated one
    pub types_override: Option<String>,
}

impl Pkg {
    pub fn new() -> Pkg {
        Pkg {
            parts: Vec::new(),
            rels: Vec::new(),
            defaults: alloc::vec![
                (String::from("rels"), String::from("application/vnd.openxmlformats-package.relationships+xml")),
                (String::from("xml"), String::from("application/xml")),
            ],
            types_override: None,
        }
    }

    /// add (or replace) a part with its content type
    pub fn part(&mut self, name: &str, ct: &str, data: impl Into<Vec<u8>>) {
        let data = data.into();
        match self.parts.iter_mut().find(|p| p.0 == name) {
            Some(p) => {
                p.1 = String::from(ct);
                p.2 = data;
            }
            None => self.parts.push((String::from(name), String::from(ct), data)),
        }
    }

    /// a default content type for an extension
    pub fn default_type(&mut self, ext: &str, ct: &str) {
        self.defaults.push((String::from(ext), String::from(ct)));
    }

    /// a relationship of `source` ("" = the package) to `target`
    pub fn rel(&mut self, source: &str, id: &str, short: &str, target: &str) {
        self.rels.push(Rel { source: String::from(source), id: String::from(id), kind: rel_type(short), target: String::from(target), external: false });
    }

    /// an external relationship
    pub fn rel_external(&mut self, source: &str, id: &str, short: &str, target: &str) {
        self.rels.push(Rel { source: String::from(source), id: String::from(id), kind: rel_type(short), target: String::from(target), external: true });
    }

    /// remove every relationship of `source` with this id
    pub fn unrel(&mut self, source: &str, id: &str) {
        self.rels.retain(|r| !(r.source == source && r.id == id));
    }

    /// replace the bytes of a part
    pub fn set(&mut self, name: &str, data: impl Into<Vec<u8>>) {
        if let Some(p) = self.parts.iter_mut().find(|p| p.0 == name) {
            p.2 = data.into();
        }
    }

    /// change the text of a part (the first occurrence of `from`); false when it is not there
    pub fn edit(&mut self, name: &str, from: &str, to: &str) -> bool {
        let Some(p) = self.parts.iter_mut().find(|p| p.0 == name) else { return false };
        let Ok(text) = core::str::from_utf8(&p.2) else { return false };
        if !text.contains(from) {
            return false;
        }
        p.2 = text.replacen(from, to, 1).into_bytes();
        true
    }

    /// the text of a part
    pub fn text(&self, name: &str) -> String {
        self.parts.iter().find(|p| p.0 == name).map(|p| String::from_utf8_lossy(&p.2).into_owned()).unwrap_or_default()
    }

    /// the bytes of a part
    pub fn data(&self, name: &str) -> Vec<u8> {
        self.parts.iter().find(|p| p.0 == name).map(|p| p.2.clone()).unwrap_or_default()
    }

    /// remove a part
    pub fn drop(&mut self, name: &str) {
        self.parts.retain(|p| p.0 != name);
    }

    /// the parts and relationship parts as ZIP entries
    pub fn entries(&self) -> Vec<Entry> {
        let mut types = format!("{DECL}<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'>");
        for (e, t) in &self.defaults {
            types.push_str(&format!("<Default Extension='{e}' ContentType='{t}'/>"));
        }
        for (n, t, _) in &self.parts {
            if !t.is_empty() {
                types.push_str(&format!("<Override PartName='/{n}' ContentType='{t}'/>"));
            }
        }
        types.push_str("</Types>");
        let types = self.types_override.clone().unwrap_or(types);
        let mut out = alloc::vec![Entry { name: String::from("[Content_Types].xml"), data: types.into_bytes() }];
        let mut sources: Vec<&str> = Vec::new();
        for r in &self.rels {
            if !sources.contains(&r.source.as_str()) {
                sources.push(&r.source);
            }
        }
        for src in sources {
            let mut x = format!("{DECL}<Relationships xmlns='http://schemas.openxmlformats.org/package/2006/relationships'>");
            for r in self.rels.iter().filter(|r| r.source == src) {
                let mode = if r.external { " TargetMode='External'" } else { "" };
                x.push_str(&format!("<Relationship Id='{}' Type='{}' Target='{}'{mode}/>", r.id, r.kind, r.target));
            }
            x.push_str("</Relationships>");
            let name = match src.rsplit_once('/') {
                _ if src.is_empty() => String::from("_rels/.rels"),
                Some((d, f)) => format!("{d}/_rels/{f}.rels"),
                None => format!("_rels/{src}.rels"),
            };
            out.push(Entry { name, data: x.into_bytes() });
        }
        for (n, _, d) in &self.parts {
            out.push(Entry { name: n.clone(), data: d.clone() });
        }
        out
    }

    /// the package as a ZIP archive (DEFLATE)
    pub fn bytes(&self) -> Vec<u8> {
        let big = Limits { entries: 300_000, part_bytes: 1 << 30, total_bytes: 1 << 30, archive_bytes: 1 << 30, ..Limits::default() };
        archive::write_with(&self.entries(), big, Method::Deflate(6)).unwrap_or_default()
    }
}

// ---- the sample drawing

/// a minimal drawing: document.xml, pages.xml with one page, its part (shapes = `shapes`,
/// connects = `connects`), masters.xml without masters
pub fn minimal(shapes: &str, connects: &str) -> Pkg {
    let mut p = Pkg::new();
    p.rel("", "rId1", "document", "visio/document.xml");
    p.part("visio/document.xml", CT_DRAWING, format!("{DECL}<VisioDocument {NS}><StyleSheets><StyleSheet ID='0' NameU='No Style'/></StyleSheets></VisioDocument>"));
    p.rel("visio/document.xml", "rId1", "pages", "pages/pages.xml");
    p.part("visio/pages/pages.xml", CT_PAGES, format!("{DECL}<Pages {NS}><Page ID='0' NameU='Page-1'><PageSheet/><Rel r:id='rId1'/></Page></Pages>"));
    p.rel("visio/pages/pages.xml", "rId1", "page", "page1.xml");
    p.part("visio/pages/page1.xml", CT_PAGE, format!("{DECL}<PageContents {NS}><Shapes>{shapes}</Shapes><Connects>{connects}</Connects></PageContents>"));
    p
}

/// a plain shape with an ID and two cells
pub fn shape(id: u32) -> String {
    format!("<Shape ID='{id}' Type='Shape' LineStyle='0' FillStyle='0' TextStyle='0'><Cell N='PinX' V='{}'/><Cell N='PinY' V='1'/></Shape>", id % 97)
}

pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-picture-the-reader-keeps-bytes-";

/// the sample drawing (see the module comment); `main_ct` its main content type
pub fn sample(main_ct: &str) -> Pkg {
    let mut p = Pkg::new();
    p.default_type("png", "image/png");
    p.default_type("dat", "application/octet-stream");
    p.rel("", "rId1", "document", "visio/document.xml");
    p.rel("", "rId2", "core-properties", "docProps/core.xml");
    p.rel("", "rId3", "extended-properties", "docProps/app.xml");
    p.part("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml",
        format!("{DECL}<cp:coreProperties xmlns:cp='http://schemas.openxmlformats.org/package/2006/metadata/core-properties' xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:title>Probe</dc:title></cp:coreProperties>"));
    p.part("docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml",
        format!("{DECL}<Properties xmlns='http://schemas.openxmlformats.org/officeDocument/2006/extended-properties'><Application>diagramiotest</Application></Properties>"));
    p.part("visio/document.xml", main_ct, format!("{DECL}<VisioDocument {NS}>\r\n\
<DocumentSettings TopPage='0' DefaultTextStyle='3' DefaultLineStyle='3' DefaultFillStyle='3' DefaultGuideStyle='4' Custom='k'>\
<GlueSettings>9</GlueSettings><SnapSettings>65847</SnapSettings><SnapAngles/><ProtectStyles>0</ProtectStyles><Odd a='1'><b/></Odd></DocumentSettings>\
<Colors><ColorEntry IX='0' RGB='#000000'/><ColorEntry IX='1' RGB='#FFFFFF'/><ColorEntry IX='2' RGB='#FF0000' Tint='y'/></Colors>\
<FaceNames><FaceName NameU='Probe Sans' UnicodeRanges='1 2 3 4' CharSets='1 0' Flags='325'/></FaceNames>\
<StyleSheets>\
<StyleSheet ID='0' NameU='No Style' Name='Kein Stil'><Cell N='LineWeight' V='0.01' U='PT'/><Cell N='LineColor' V='0'/>\
<Section N='Character'><Row IX='0'><Cell N='Font' V='Themed' F='THEMEVAL()'/><Cell N='Size' V='0.1666666666666667' U='PT'/></Row></Section></StyleSheet>\
<StyleSheet ID='3' NameU='Normal' Name='Normal' LineStyle='0' FillStyle='0' TextStyle='0'><Cell N='FillForegnd' V='#4472c4' F='THEMEVAL(\"FillColor\",0)'/></StyleSheet>\
<StyleSheet ID='4' NameU='Guide' LineStyle='3' FillStyle='3' TextStyle='3' Hidden='0'/>\
</StyleSheets>\
<DocumentSheet NameU='TheDoc' Name='TheDoc' LineStyle='0' FillStyle='0' TextStyle='0'><Cell N='OutputFormat' V='0'/>\
<Section N='User'><Row N='Key'><Cell N='Value' V='007' U='STR'/><Cell N='Prompt' V='' F='No Formula'/></Row></Section></DocumentSheet>\
<EventList><EventItem ID='1' Action='1' EventCode='1' Enabled='1' Target='t' TargetArgs=''/></EventList>\
<x:Foreign xmlns:x='urn:test'>kept &amp; sound</x:Foreign>\
</VisioDocument>\r\n"));
    p.rel("visio/document.xml", "rId1", "masters", "masters/masters.xml");
    p.rel("visio/document.xml", "rId2", "pages", "pages/pages.xml");
    p.rel("visio/document.xml", "rId3", "windows", "windows.xml");
    p.rel("visio/document.xml", "rId4", "theme", "theme/theme1.xml");
    p.rel("visio/document.xml", "rId5", "comments", "comments.xml");
    p.rel("visio/document.xml", "rId6", "connections", "data/connections.xml");
    p.rel("visio/document.xml", "rId7", "recordsets", "data/recordsets.xml");
    p.rel("visio/document.xml", "rId8", "solutions", "solutions/solutions.xml");
    p.rel("visio/document.xml", "rId9", "urn:test:unknown-relationship", "../custom/thing.bin");
    p.rel_external("visio/document.xml", "rId10", "hyperlink", "https://example.org/never-fetched");
    p.part("visio/windows.xml", CT_WINDOWS, format!("{DECL}<Windows ClientWidth='800' ClientHeight='600' {NS}><Window ID='0' WindowType='Drawing' Page='0'/></Windows>"));
    p.part("visio/theme/theme1.xml", CT_THEME,
        format!("{DECL}<a:theme xmlns:a='http://schemas.openxmlformats.org/drawingml/2006/main' name='Probe'><a:themeElements/></a:theme>"));
    p.part("visio/comments.xml", "application/vnd.ms-visio.comments+xml",
        format!("{DECL}<Comments {NS}><AuthorList><AuthorEntry ID='0' Name='A'/></AuthorList><CommentList/></Comments>"));
    p.part("visio/data/connections.xml", "application/vnd.ms-visio.connections+xml", format!("{DECL}<DataConnections {NS}/>"));
    p.part("visio/data/recordsets.xml", "application/vnd.ms-visio.recordsets+xml", format!("{DECL}<DataRecordSets {NS}><DataRecordSet ID='1'><Rel r:id='rId1'/></DataRecordSet></DataRecordSets>"));
    p.rel("visio/data/recordsets.xml", "rId1", "recordset", "recordset1.xml");
    p.part("visio/data/recordset1.xml", "application/vnd.ms-visio.recordset+xml", "<xml>data</xml>");
    p.part("visio/solutions/solutions.xml", "application/vnd.ms-visio.solutions+xml", format!("{DECL}<Solutions {NS}/>"));
    p.part("custom/thing.bin", "application/x-test", alloc::vec![0u8, 1, 2, 3, 255]);
    p.part("orphan.dat", "", b"no relationship names me".to_vec());
    // masters
    p.part("visio/masters/masters.xml", CT_MASTERS, format!("{DECL}<Masters {NS}>\
<Master ID='2' NameU='Box' Name='Kasten' Prompt='a box' IconSize='1' BaseID='{{1A2B3C4D-0000-0000-0000-000000000001}}' UniqueID='{{1A2B3C4D-0000-0000-0000-000000000002}}' MatchByName='0'>\
<PageSheet LineStyle='0' FillStyle='0' TextStyle='0' Extra='m'><Cell N='PageWidth' V='1'/></PageSheet><Rel r:id='rId1'/><Icon>QUJD\nREVG</Icon><Future/></Master>\
</Masters>"));
    p.rel("visio/masters/masters.xml", "rId1", "master", "master1.xml");
    p.part("visio/masters/master1.xml", CT_MASTER, format!("{DECL}<MasterContents {NS}><Shapes>\
<Shape ID='5' NameU='Box' Type='Shape' LineStyle='3' FillStyle='3' TextStyle='3'>\
<Cell N='PinX' V='0.5' F='Width*0.5'/><Cell N='Width' V='1' U='MM'/><Cell N='Height' V='0.5'/>\
<Section N='Geometry' IX='0'><Cell N='NoFill' V='0'/><Row T='MoveTo' IX='1'><Cell N='X' V='0'/><Cell N='Y' V='0'/></Row>\
<Row T='LineTo' IX='2'><Cell N='X' V='1' F='Width'/><Cell N='Y' V='0'/></Row></Section>\
<Section N='Connection'><Row T='Connection' IX='0'><Cell N='X' V='0'/><Cell N='Y' V='0.25'/></Row></Section>\
<Section N='Property'><Row N='Serial'><Cell N='Value' V='0042' U='STR'/><Cell N='Label' V='Serial'/></Row></Section>\
<Text><cp IX='0'/>Master text</Text></Shape></Shapes></MasterContents>"));
    // pages
    p.part("visio/pages/pages.xml", CT_PAGES, format!("{DECL}<Pages {NS}>\
<Page ID='0' NameU='Page-1' Name='Blatt-1' ViewScale='1' ViewCenterX='4.25' BackPage='3' IsCustomName='1'>\
<PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><Cell N='PageWidth' V='8.5' U='IN'/><Cell N='PageHeight' V='11'/>\
<Section N='Layer'><Row IX='0'><Cell N='Name' V='Connectors'/><Cell N='Visible' V='1'/></Row></Section></PageSheet><Rel r:id='rId1'/></Page>\
<Page ID='3' NameU='Background-1' Background='1'><PageSheet/><Rel r:id='rId2'/></Page>\
<Note>pages.xml has room for more</Note></Pages>"));
    p.rel("visio/pages/pages.xml", "rId1", "page", "page1.xml");
    p.rel("visio/pages/pages.xml", "rId2", "page", "page2.xml");
    p.part("visio/pages/page2.xml", CT_PAGE, format!("{DECL}<PageContents {NS}/>"));
    p.part("visio/media/image1.png", "", PNG.to_vec());
    p.rel("visio/pages/page1.xml", "rId1", "image", "../media/image1.png");
    p.rel("visio/pages/page1.xml", "rId2", "master", "../masters/master1.xml");
    p.part("visio/pages/page1.xml", CT_PAGE, format!("{DECL}<PageContents {NS} Custom='c'>\r\n<Shapes>\r\n\
<Shape ID='1' NameU='Box.1' Type='Shape' Master='2' Foo='bar'>\
<Cell N='PinX' V='2'/><Cell N='Width' V='1' F='Inh'/><Cell N='Angle' V='0' U='DEG' E='#REF!'/><Cell N='Odd' V='7' U='XYZ'/>\
<Cell N='LocPinX' V='0.5' F='Width*0.5'><RefBy T='Shape' ID='6'/></Cell>\
<Text>Hallo\r\n<cp IX='0'/>Welt<pp IX='1'/> &amp; mehr<fld IX='0'>12</fld><tp IX='0'></tp><b:x xmlns:b='urn:t'/></Text>\
<Data1>first</Data1><Bar baz='1'/>loose text</Shape>\
<Shape ID='2' Type='Group' NameU='Group'>\
<Section N='Geometry'><Row T='MoveTo'><Cell N='X' V='0'/></Row><Row T='LineTo' Del='1'/></Section>\
<Section N='Geometry'><Cell N='NoShow' V='1'/></Section><Section N='User' Del='1'/>\
<Trigger N='Refresh'><RefBy T='Shape' ID='1'/></Trigger>\
<Shapes>\
<Shape ID='3' Type='Shape' MasterShape='5'><Cell N='PinY' V='1'/></Shape>\
<Unknown/>\
<Shape ID='4' Type='Foreign' NameU='Picture'><ForeignData ForeignType='Bitmap' CompressionType='PNG' ObjectWidth='1'><Rel r:id='rId1'/></ForeignData></Shape>\
</Shapes></Shape>\
<Shape ID='6' NameU='Dynamic connector' Type='Shape'><Cell N='BeginX' V='1'/><Cell N='EndX' V='3'/></Shape>\
<Shape ID='7' Type='Foreign'><ForeignData ForeignType='Bitmap' CompressionType='PNG'>iVBORw0KGgo=</ForeignData></Shape>\
<Shape ID='8' Del='1'/>\
</Shapes>\r\n<Connects><Connect FromSheet='6' FromCell='BeginX' FromPart='9' ToSheet='1' ToCell='PinX' ToPart='3'/>\
<Connect FromSheet='6' FromCell='EndX' FromPart='12' ToSheet='2' ToCell='Connections.X1' ToPart='100' Glue='odd'/></Connects>\
<Later/></PageContents>"));
    p
}
