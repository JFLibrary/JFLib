// Supported paragraph style graphs in standard WordprocessingML.
use crate::{Error, Node, Document, Paragraph, ParagraphFormat, ParagraphStyle, TextFormat, esc, format, parse, style, style_name, unsupported, package::{FIXED, W}};
use alloc::{string::String, vec::Vec};
use doc_core::StyleDefinition;

fn original() -> &'static str { FIXED.iter().find(|(name, _)| *name == "word/styles.xml").unwrap().1 }
pub(crate) fn write(d: &Document) -> Result<String, Error> {
    let old = original();
    let defaults_end = old.find("</w:docDefaults>").ok_or(Error::Invalid)? + "</w:docDefaults>".len();
    let mut out = String::from(&old[..defaults_end]);
    for id in d.style_ids() {
        let s = d.style_definition(id).ok_or(Error::Invalid)?;
        out.push_str(&format!("<w:style w:type=\"paragraph\" w:styleId=\"{}\"{}><w:name w:val=\"{}\"/>",
            style_name(id), if id == ParagraphStyle::Normal { " w:default=\"1\"" } else { "" }, esc(&s.name)?));
        if let Some(base) = s.based_on { out.push_str(&format!("<w:basedOn w:val=\"{}\"/>", style_name(base))); }
        out.push_str("<w:qFormat/><w:pPr>");
        out.push_str(&format::write_properties(s.paragraph));
        if let Some(level) = s.outline_level {
            if level < 9 { out.push_str("<w:keepNext/>"); }
            out.push_str(&format!("<w:outlineLvl w:val=\"{level}\"/>"));
        }
        out.push_str("</w:pPr>");
        out.push_str(&format::write_run(s.text));
        out.push_str("</w:style>");
    }
    out.push_str(&crate::table_styles::write(d)?);
    out.push_str("</w:styles>");
    Ok(out)
}
fn value(n: &Node) -> Result<&str, Error> {
    if n.name.ns != W || !n.children.is_empty() || !n.text.is_empty() || n.attrs.len() != 1 { return Err(unsupported(&n.name.local)); }
    n.attr(W, "val").ok_or(Error::Invalid)
}
pub(crate) fn read(bytes: &[u8]) -> Result<Vec<StyleDefinition>, Error> {
    let root = parse(bytes)?;
    let old = parse(original().as_bytes())?;
    if root == old { return Ok(Vec::new()); }
    crate::container(&root, W, "styles")?;
    if root.children.len() > 265 || root.children.first() != old.children.first() { return Err(unsupported("Dokumentstandardformat")); }
    let defaults = Document::default();
    let mut out = Vec::new();
    let mut ids = alloc::collections::BTreeSet::new();
    for node in root.children.iter().skip(1) {
        if node.is(W,"style") && node.attr(W,"type")==Some("table") { continue; }
        if !node.is(W, "style") || !node.text.is_empty() || node.attr(W, "type") != Some("paragraph")
            || node.attrs.iter().any(|(n, _)| n.ns != W || !["type", "styleId", "default"].contains(&n.local.as_str())) {
            return Err(unsupported("Formatvorlagentyp"));
        }
        let id = style(node.attr(W, "styleId").ok_or(Error::Invalid)?)?;
        if !ids.insert(id) { return Err(Error::Invalid); }
        if node.attr(W, "default").is_some_and(|v| v != "1" || id != ParagraphStyle::Normal) { return Err(unsupported("Standardformatvorlage")); }
        let mut s = StyleDefinition { id, name: String::new(), based_on: None, text: TextFormat::default(), paragraph: ParagraphFormat::default(), outline_level: None };
        let mut seen = alloc::collections::BTreeSet::new();
        for child in &node.children {
            if child.name.ns != W || !seen.insert(&child.name.local) { return Err(Error::Invalid); }
            match child.name.local.as_str() {
                "name" => s.name = String::from(value(child)?),
                "basedOn" => s.based_on = Some(style(value(child)?)?),
                "next" if value(child)? == "Normal" => (),
                "qFormat" => { crate::container(child, W, "qFormat")?; if !child.children.is_empty() { return Err(Error::Invalid); } },
                "rPr" => s.text = format::read_run(child)?,
                "pPr" => {
                    let mut props = child.clone();
                    let mut keep = false;
                    for p in &child.children {
                        if p.is(W, "outlineLvl") {
                            if s.outline_level.is_some() { return Err(Error::Invalid); }
                            s.outline_level = Some(value(p)?.parse().map_err(|_| Error::Invalid)?);
                        } else if p.is(W, "keepNext") {
                            if keep || !p.children.is_empty() || !p.text.is_empty() || !p.attrs.is_empty() { return Err(unsupported("Absatzzusammenhalt")); }
                            keep = true;
                        }
                    }
                    if keep && !s.outline_level.is_some_and(|n| n < 9) { return Err(unsupported("Absatzzusammenhalt")); }
                    props.children.retain(|p| !p.is(W, "outlineLvl") && !p.is(W, "keepNext"));
                    if props.children.iter().any(|p| p.is(W, "pStyle")) { return Err(Error::Invalid); }
                    let mut p = Paragraph { id: 1, style: id, format: ParagraphFormat::default(), content: Vec::new() };
                    format::read_paragraph(&props, &mut p)?;
                    s.paragraph = p.format;
                },
                _ => return Err(unsupported(&child.name.local)),
            }
        }
        if defaults.style_definition(id).as_ref() != Some(&s) { out.push(s); }
    }
    // A missing standard definition is not silently filled in for imported packages.
    if [ParagraphStyle::Normal, ParagraphStyle::Title, ParagraphStyle::Heading1, ParagraphStyle::Heading2].iter().any(|id| !ids.contains(id)) { return Err(Error::Invalid); }
    Ok(out)
}
