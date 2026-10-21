// Semantic style adapters; the editor owns validation, inheritance and transactions.
use super::*;
use doc_core::{Alignment, Document, FontFamily, ParagraphFormat, StyleDefinition};
pub(super) static STYLE_KEY: Schema = Schema::String { min: 1, max: 40, choices: &[] };
pub(super) static PARAGRAPH: Schema = Schema::Object { properties: &[
    property("tabs", &paragraph_actions::TABS, false),
    property("alignment", &Schema::String { min: 1, max: 8, choices: &["left", "center", "right", "justify"] }, false),
    property("before", &Schema::Unsigned { min: 0, max: 2880 }, false),
    property("after", &Schema::Unsigned { min: 0, max: 2880 }, false),
    property("line", &Schema::Unsigned { min: 240, max: 720 }, false),
    property("left", &Schema::Unsigned { min: 0, max: 31680 }, false),
    property("right", &Schema::Unsigned { min: 0, max: 31680 }, false),
    property("firstLine", &Schema::Integer { min: -31680, max: 31680 }, false),
    property("pageBreakBefore", &BOOLEAN, false),
], additional: false };
pub(super) static STYLE_PUT: Schema = Schema::Object { properties: &[
    property("id", &STYLE_KEY, false), property("name", &Schema::String { min: 1, max: 128, choices: &[] }, true),
    property("basedOn", &Schema::String { min: 0, max: 40, choices: &[] }, false),
    property("format", &CHARACTER, false), property("paragraph", &PARAGRAPH, false),
    property("outlineLevel", &Schema::Unsigned { min: 0, max: 9 }, false),
], additional: false };
pub(super) fn key(style: ParagraphStyle) -> String {
    match style { ParagraphStyle::Normal => String::from("Normal"), ParagraphStyle::Title => String::from("Title"),
        ParagraphStyle::Heading1 => String::from("Heading1"), ParagraphStyle::Heading2 => String::from("Heading2"),
        ParagraphStyle::Named(n) => format!("jfStyle{n}") }
}
pub(super) fn parse_key(value: &str) -> Result<ParagraphStyle, ActionError> {
    Ok(match value { "Normal" => ParagraphStyle::Normal, "Title" => ParagraphStyle::Title,
        "Heading1" => ParagraphStyle::Heading1, "Heading2" => ParagraphStyle::Heading2,
        _ => ParagraphStyle::Named(value.strip_prefix("jfStyle").and_then(|s| s.parse::<u32>().ok())
            .filter(|n| *n != 0 && *n != u32::MAX).ok_or_else(|| invalid("Unknown style identity; use document.styles.list"))?) })
}
pub(super) fn character(f: &Value) -> Result<TextFormat, ActionError> {
    Ok(TextFormat {
        font_family: f.get("fontFamily").and_then(Value::as_str).map(FontFamily::new).transpose().map_err(|_| invalid("Invalid font family name"))?,
        bold: f.get("bold").and_then(Value::as_bool), italic: f.get("italic").and_then(Value::as_bool),
        underline: f.get("underline").and_then(Value::as_bool), strike: f.get("strike").and_then(Value::as_bool),
        size_half_points: f.get("sizeHalfPoints").and_then(Value::as_u64).map(|n| n as u16),
        color: f.get("color").and_then(Value::as_u64).map(|n| TextColor::Rgb(n as u32)),
        highlight: f.get("highlight").and_then(Value::as_str).map(|v| doc_core::Highlight::from_name(v).ok_or_else(|| invalid("Unknown highlight color"))).transpose()?,
        positioning: f.get("positioning").and_then(Value::as_str).map(|v| doc_core::TextPositioning::from_name(v).ok_or_else(|| invalid("Unknown text positioning"))).transpose()?,
    })
}
pub(super) fn definition(d: &Document, input: &Value) -> Result<StyleDefinition, ActionError> {
    let id = input.get("id").and_then(Value::as_str).map(parse_key).transpose()?.unwrap_or_else(|| d.next_style_id());
    let mut s = StyleDefinition::new(id, &text(input, "name"));
    s.based_on = match input.get("basedOn").and_then(Value::as_str) { Some("") => None, Some(name) => Some(parse_key(name)?), None if id == ParagraphStyle::Normal => None, None => Some(ParagraphStyle::Normal) };
    if let Some(f) = input.get("format") { s.text = character(f)?; }
    if let Some(p) = input.get("paragraph") {
        let f = &mut s.paragraph;
        if let Some(tabs) = p.get("tabs") { f.tabs = paragraph_actions::tabs(tabs)?; }
        for (name, bit, field) in [("before", 2, &mut f.before), ("after", 4, &mut f.after), ("line", 8, &mut f.line), ("left", 16, &mut f.left), ("right", 32, &mut f.right)] {
            if let Some(n) = p.get(name).and_then(Value::as_u64) { *field = n as u16; f.explicit |= bit; }
        }
        if let Some(n) = p.get("firstLine").and_then(Value::as_i64) { f.first_line = n as i32; f.explicit |= 64; }
        if let Some(a) = p.get("alignment").and_then(Value::as_str) { f.alignment = match a { "center" => Alignment::Center, "right" => Alignment::Right, "justify" => Alignment::Justify, _ => Alignment::Left }; f.explicit |= 1; }
        if let Some(v) = p.get("pageBreakBefore").and_then(Value::as_bool) { f.page_break_before = v; f.explicit |= 128; }
    }
    s.outline_level = input.get("outlineLevel").and_then(Value::as_u64).map(|n| n as u8);
    Ok(s)
}
pub(super) fn paragraph(f: ParagraphFormat) -> Value {
    let mask = f.mask();
    Value::object([
        ("tabs", paragraph_actions::tabs_json(f.tabs)),
        ("alignment", if mask & 1 != 0 { Value::string(match f.alignment { Alignment::Left => "left", Alignment::Center => "center", Alignment::Right => "right", Alignment::Justify => "justify" }) } else { Value::Null }),
        ("before", if mask & 2 != 0 { Value::u64(f.before.into()) } else { Value::Null }),
        ("after", if mask & 4 != 0 { Value::u64(f.after.into()) } else { Value::Null }),
        ("line", if mask & 8 != 0 { Value::u64(f.line.into()) } else { Value::Null }),
        ("left", if mask & 16 != 0 { Value::u64(f.left.into()) } else { Value::Null }),
        ("right", if mask & 32 != 0 { Value::u64(f.right.into()) } else { Value::Null }),
        ("firstLine", if mask & 64 != 0 { Value::i64(i64::from(f.first_line)) } else { Value::Null }),
        ("pageBreakBefore", if mask & 128 != 0 { Value::Bool(f.page_break_before) } else { Value::Null }),
    ])
}
pub(super) fn list(d: &Document, query: &Value) -> Value {
    let offset = id(query, "offset") as usize;
    let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100) as usize;
    let styles = d.style_ids();
    Value::object([("total", Value::u64(styles.len() as u64)),
        ("nextOffset", if offset.saturating_add(limit) < styles.len() { Value::u64((offset + limit) as u64) } else { Value::Null }),
        ("styles", Value::Array(styles.iter().skip(offset).take(limit).map(|id| {
            let s = d.style_definition(*id).unwrap();
            Value::object([("id", Value::string(key(*id))), ("name", Value::string(s.name)),
                ("basedOn", s.based_on.map(|base| Value::string(key(base))).unwrap_or(Value::Null)),
                ("format", format_json(s.text)), ("paragraph", paragraph(s.paragraph)),
                ("outlineLevel", s.outline_level.map(|n| Value::u64(n.into())).unwrap_or(Value::Null)),
                ("effectiveFormat", format_json(d.resolve_text(Default::default(), *id).explicit()))])
        }).collect()))])
}
