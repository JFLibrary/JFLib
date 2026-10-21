// Supported WordprocessingML run, paragraph and page properties; reject unrepresented data.
use crate::{Error, Node, container, package::W, style, style_name, unsupported};
use alloc::{collections::BTreeSet, format, string::String};
use doc_core::{Alignment, Highlight, PageSettings, Paragraph, ParagraphFormat, TextColor, TextFormat, TextPositioning};

fn empty(n: &Node) -> Result<(), Error> {
    if n.name.ns != W || !n.children.is_empty() || !n.text.is_empty() {
        Err(unsupported(&n.name.local))
    } else {
        Ok(())
    }
}
fn attrs(n: &Node, names: &[&str]) -> Result<(), Error> {
    empty(n)?;
    if n.attrs
        .iter()
        .any(|(name, _)| name.ns != W || !names.contains(&name.local.as_str()))
    {
        Err(unsupported(&n.name.local))
    } else {
        Ok(())
    }
}
fn number(n: &Node, name: &str) -> Result<u32, Error> {
    n.attr(W, name)
        .ok_or(Error::Invalid)?
        .parse()
        .map_err(|_| Error::Invalid)
}
fn small(n: &Node, name: &str) -> Result<u16, Error> {
    u16::try_from(number(n, name)?).map_err(|_| Error::Invalid)
}
fn on(n: &Node) -> Result<bool, Error> {
    attrs(n, &["val"])?;
    match n.attr(W, "val") {
        None | Some("1" | "true" | "on") => Ok(true),
        Some("0" | "false" | "off") => Ok(false),
        _ => Err(Error::Invalid),
    }
}
pub(crate) fn write_run(f: TextFormat) -> String {
    if f == TextFormat::default() {
        return String::new();
    }
    let mut out = String::from("<w:rPr>");
    if let Some(family) = f.font_family {
        let family = crate::esc(family.as_str()).unwrap_or_default();
        out.push_str(&format!("<w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\"/>"));
    }
    for (tag, value) in [("b", f.bold), ("i", f.italic), ("strike", f.strike)] {
        if let Some(value) = value {
            out.push_str(&format!(
                "<w:{tag} w:val=\"{}\"/>",
                if value { 1 } else { 0 }
            ));
        }
    }
    if let Some(color) = f.color {
        let value = match color {
            TextColor::Auto => String::from("auto"),
            TextColor::Rgb(rgb) => format!("{rgb:06X}"),
        };
        out.push_str(&format!("<w:color w:val=\"{value}\"/>"));
    }
    if let Some(value) = f.size_half_points {
        out.push_str(&format!("<w:sz w:val=\"{value}\"/>"));
    }
    if let Some(value) = f.underline {
        out.push_str(&format!(
            "<w:u w:val=\"{}\"/>",
            if value { "single" } else { "none" }
        ));
    }
    if let Some(value) = f.highlight {
        out.push_str(&format!("<w:highlight w:val=\"{}\"/>", value.name()));
    }
    if let Some(value) = f.positioning {
        out.push_str(&format!("<w:vertAlign w:val=\"{}\"/>", value.name()));
    }
    out.push_str("</w:rPr>");
    out
}
pub(crate) fn read_run(n: &Node) -> Result<TextFormat, Error> {
    container(n, W, "rPr")?;
    let mut f = TextFormat::default();
    let mut seen = BTreeSet::new();
    for c in &n.children {
        if !seen.insert(&c.name.local) {
            return Err(Error::Invalid);
        }
        match c.name.local.as_str() {
            "rFonts" => {
                attrs(c, &["ascii", "hAnsi"])?;
                let family = c.attr(W, "ascii").or_else(|| c.attr(W, "hAnsi")).ok_or(Error::Invalid)?;
                if c.attr(W, "hAnsi").is_some_and(|name| name != family) { return Err(unsupported("Gemischte Schriftzuordnung")); }
                f.font_family = Some(doc_core::FontFamily::new(family)?);
            },
            "b" => f.bold = Some(on(c)?),
            "i" => f.italic = Some(on(c)?),
            "strike" => f.strike = Some(on(c)?),
            "color" => {
                // Theme dependencies cannot be flattened to the fallback RGB value.
                attrs(c, &["val"])?;
                let value = c.attr(W, "val").ok_or(Error::Invalid)?;
                f.color = Some(if value == "auto" {
                    TextColor::Auto
                } else {
                    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(Error::Invalid);
                    }
                    TextColor::Rgb(u32::from_str_radix(value, 16).map_err(|_| Error::Invalid)?)
                });
            }
            "u" => {
                attrs(c, &["val"])?;
                f.underline = Some(match c.attr(W, "val") {
                    Some("single") => true,
                    Some("none") => false,
                    _ => return Err(unsupported("Unterstreichung")),
                });
            }
            "sz" => {
                attrs(c, &["val"])?;
                f.size_half_points = Some(small(c, "val")?);
            }
            "highlight" => {
                attrs(c, &["val"])?;
                f.highlight = Some(Highlight::from_name(c.attr(W, "val").ok_or(Error::Invalid)?).ok_or(Error::Invalid)?);
            }
            "vertAlign" => {
                attrs(c, &["val"])?;
                f.positioning = Some(TextPositioning::from_name(c.attr(W, "val").ok_or(Error::Invalid)?).ok_or(Error::Invalid)?);
            }
            _ => return Err(unsupported(&c.name.local)),
        }
    }
    f.validate()?;
    Ok(f)
}
pub(crate) fn write_paragraph(p: &Paragraph) -> String {
    let mut out = format!("<w:pPr><w:pStyle w:val=\"{}\"/>", style_name(p.style));
    out.push_str(&write_properties(p.format));
    out.push_str("</w:pPr>");
    out
}
pub(crate) fn write_properties(f: ParagraphFormat) -> String {
    let mut out = String::new();
    let mask = f.mask();
    if mask != 0 {
        let alignment = match f.alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "both",
        };
        if mask & 128 != 0 {
            out.push_str(&format!("<w:pageBreakBefore w:val=\"{}\"/>", u8::from(f.page_break_before)));
        }
        if !f.tabs.is_empty() {
            out.push_str("<w:tabs>");
            for s in f.tabs.as_slice() {
                out.push_str(&format!("<w:tab w:val=\"{}\" w:pos=\"{}\" w:leader=\"{}\"/>", s.alignment.name(), s.position, s.leader.name()));
            }
            out.push_str("</w:tabs>");
        }
        if mask & 14 != 0 {
            out.push_str("<w:spacing");
            for (bit, tag, value) in [(2, "before", f.before), (4, "after", f.after), (8, "line", f.line)] {
                if mask & bit != 0 { out.push_str(&format!(" w:{tag}=\"{value}\"")); }
            }
            if mask & 8 != 0 { out.push_str(" w:lineRule=\"auto\""); }
            out.push_str("/>");
        }
        if mask & 112 != 0 {
            out.push_str("<w:ind");
            for (bit, tag, value) in [(16, "left", i32::from(f.left)), (32, "right", i32::from(f.right)),
                (64, if f.first_line < 0 { "hanging" } else { "firstLine" }, f.first_line.abs())] {
                if mask & bit != 0 { out.push_str(&format!(" w:{tag}=\"{value}\"")); }
            }
            out.push_str("/>");
        }
        if mask & 1 != 0 { out.push_str(&format!("<w:jc w:val=\"{alignment}\"/>")); }
    }
    out
}
pub(crate) fn read_paragraph(n: &Node, p: &mut Paragraph) -> Result<(), Error> {
    container(n, W, "pPr")?;
    let mut seen = BTreeSet::new();
    for c in &n.children {
        if !seen.insert(&c.name.local) {
            return Err(Error::Invalid);
        }
        match c.name.local.as_str() {
            "pStyle" => {
                attrs(c, &["val"])?;
                p.style = style(c.attr(W, "val").ok_or(Error::Invalid)?)?;
            }
            "pageBreakBefore" => { p.format.page_break_before = on(c)?; p.format.explicit |= 128; },
            "tabs" => {
                container(c, W, "tabs")?;
                let mut positions = BTreeSet::new();
                for tab in &c.children {
                    if tab.name.local != "tab" { return Err(unsupported(&tab.name.local)); }
                    attrs(tab, &["val", "pos", "leader"])?;
                    let position = tab.attr(W, "pos").ok_or(Error::Invalid)?.parse().map_err(|_| Error::Invalid)?;
                    if !positions.insert(position) { return Err(Error::Invalid); }
                    let alignment = doc_core::TabAlignment::from_name(tab.attr(W, "val").ok_or(Error::Invalid)?).ok_or_else(|| unsupported("Tabulatorausrichtung"))?;
                    let leader = doc_core::TabLeader::from_name(tab.attr(W, "leader").unwrap_or("none")).ok_or_else(|| unsupported("Tabulatorfüllzeichen"))?;
                    p.format.tabs.set(doc_core::TabStop { position, alignment, leader })?;
                }
                // An empty w:tabs does not override inherited tabs.
            }
            "spacing" => {
                attrs(c, &["before", "after", "line", "lineRule"])?;
                if let Some(rule) = c.attr(W, "lineRule") {
                    if rule != "auto" {
                        return Err(unsupported("Zeilenabstand"));
                    }
                }
                if c.attr(W, "before").is_some() {
                    p.format.explicit |= 2;
                    p.format.before = small(c, "before")?;
                }
                if c.attr(W, "after").is_some() {
                    p.format.explicit |= 4;
                    p.format.after = small(c, "after")?;
                }
                if c.attr(W, "line").is_some() {
                    p.format.explicit |= 8;
                    p.format.line = small(c, "line")?;
                }
            }
            "ind" => {
                attrs(c, &["left", "right", "firstLine", "hanging"])?;
                // Preserve ambiguous/overlapping source properties through the package path.
                if c.attr(W, "firstLine").is_some() && c.attr(W, "hanging").is_some() {
                    return Err(unsupported("Überlagerte Erstzeileneinzüge"));
                }
                if c.attr(W, "left").is_some() {
                    p.format.explicit |= 16;
                    p.format.left = small(c, "left")?;
                }
                if c.attr(W, "right").is_some() {
                    p.format.explicit |= 32;
                    p.format.right = small(c, "right")?;
                }
                if c.attr(W, "firstLine").is_some() {
                    p.format.explicit |= 64;
                    p.format.first_line = i32::from(small(c, "firstLine")?);
                }
                if c.attr(W, "hanging").is_some() {
                    p.format.explicit |= 64;
                    p.format.first_line = -i32::from(small(c, "hanging")?);
                }
            }
            "jc" => {
                p.format.explicit |= 1;
                attrs(c, &["val"])?;
                p.format.alignment = match c.attr(W, "val") {
                    Some("left") => Alignment::Left,
                    Some("center") => Alignment::Center,
                    Some("right") => Alignment::Right,
                    Some("both") => Alignment::Justify,
                    _ => return Err(unsupported("Absatzausrichtung")),
                };
            }
            _ => return Err(unsupported(&c.name.local)),
        }
    }
    Ok(())
}
pub(crate) fn write_section(p: PageSettings) -> String {
    let orient = if p.width > p.height {
        " w:orient=\"landscape\""
    } else {
        ""
    };
    format!(
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"{orient}/><w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr>",
        p.width, p.height, p.top, p.right, p.bottom, p.left
    )
}
pub(crate) fn read_section(n: &Node) -> Result<PageSettings, Error> {
    container(n, W, "sectPr")?;
    if n.children.len() != 2 {
        return Err(unsupported("Abschnittslayout"));
    }
    let mut p = PageSettings::default();
    let mut seen = BTreeSet::new();
    for c in &n.children {
        if !seen.insert(&c.name.local) {
            return Err(Error::Invalid);
        }
        match c.name.local.as_str() {
            "pgSz" => {
                attrs(c, &["w", "h", "orient"])?;
                p.width = number(c, "w")?;
                p.height = number(c, "h")?;
                if let Some(orient) = c.attr(W, "orient") {
                    if orient
                        != if p.width > p.height {
                            "landscape"
                        } else {
                            "portrait"
                        }
                    {
                        return Err(unsupported("Seitenorientierung"));
                    }
                }
            }
            "pgMar" => {
                attrs(
                    c,
                    &[
                        "top", "right", "bottom", "left", "header", "footer", "gutter",
                    ],
                )?;
                p.top = number(c, "top")?;
                p.right = number(c, "right")?;
                p.bottom = number(c, "bottom")?;
                p.left = number(c, "left")?;
                if number(c, "header")? != 708
                    || number(c, "footer")? != 708
                    || number(c, "gutter")? != 0
                {
                    return Err(unsupported("Kopf-/Fußzeilenrand"));
                }
            }
            _ => return Err(unsupported(&c.name.local)),
        }
    }
    p.validate()?;
    Ok(p)
}
