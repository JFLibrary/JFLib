// Fixed parts of jfWord's initial editable OPC profile; no executable or external relations.
use alloc::{format, string::String};
use doc_core::NoteKind;
pub(crate) const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub(crate) const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
pub(crate) const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
pub(crate) const XML: &str = "http://www.w3.org/XML/1998/namespace";
pub(crate) const B: &str = "http://schemas.openxmlformats.org/officeDocument/2006/bibliography";
pub(crate) const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
pub(crate) const FIXED: &[(&str, &str)] = &[
    (
        "[Content_Types].xml",
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/customXml/itemProps1.xml" ContentType="application/vnd.openxmlformats-officedocument.customXmlProperties+xml"/></Types>"#,
    ),
    (
        "_rels/.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
    ),
    (
        "word/_rels/document.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/></Relationships>"#,
    ),
    (
        "customXml/_rels/item1.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps" Target="itemProps1.xml"/></Relationships>"#,
    ),
    (
        "customXml/itemProps1.xml",
        r#"<ds:datastoreItem ds:itemID="{3A684A17-8584-4521-8F46-D15CEEE14396}" xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs><ds:schemaRef ds:uri="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"/></ds:schemaRefs></ds:datastoreItem>"#,
    ),
    (
        "word/styles.xml",
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="DejaVu Sans" w:hAnsi="DejaVu Sans"/><w:sz w:val="22"/><w:lang w:val="de-DE"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:rPr><w:sz w:val="48"/><w:color w:val="176B70"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:color w:val="176B70"/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style></w:styles>"#,
    ),
];

// Only note-bearing documents extend the original fixed metadata profile.
pub(crate) fn metadata(name: &str, base: &str, notes: [bool; 2]) -> String {
    let mut out = String::from(base);
    for (index, kind) in crate::notes::KINDS.into_iter().enumerate() {
        if !notes[index] {
            continue;
        }
        let tag = crate::notes::name(kind);
        if name == "[Content_Types].xml" {
            let extra = format!(
                "<Override PartName=\"/word/{tag}s.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{tag}s+xml\"/></Types>"
            );
            out = out.replacen("</Types>", &extra, 1);
        } else if name == "word/_rels/document.xml.rels" {
            let id = if kind == NoteKind::Footnote { 3 } else { 4 };
            let extra = format!(
                "<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{tag}s\" Target=\"{tag}s.xml\"/></Relationships>"
            );
            out = out.replacen("</Relationships>", &extra, 1);
        }
    }
    if notes.iter().any(|present| *present) {
        if name == "[Content_Types].xml" {
            out = out.replacen("</Types>", "<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/></Types>", 1);
        } else if name == "word/_rels/document.xml.rels" {
            out = out.replacen("</Relationships>", "<Relationship Id=\"rId5\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/></Relationships>", 1);
        }
    }
    out
}
