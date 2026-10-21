// Shared paragraph-model adapters for range edits, ruler state and tab-stop discovery.
use super::*;
use doc_core::{PageSettings, ParagraphFormat, TabAlignment, TabLeader, TabStop, TabStops};

pub(super) static TABS: Schema = Schema::Array { min: 0, max: 64, items: &Schema::Object { properties: &[
    property("position", &Schema::Integer { min: -31680, max: 31680 }, true),
    property("alignment", &Schema::String { min: 1, max: 7, choices: &["left", "center", "right", "decimal", "bar", "clear"] }, true),
    property("leader", &Schema::String { min: 1, max: 10, choices: &["none", "dot", "hyphen", "underscore", "heavy", "middleDot"] }, false),
], additional: false } };
pub(super) static TAB_RANGE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("start", &U64, true), property("end", &U64, true), property("tabs", &TABS, true)], additional: false };
pub(super) static PARAGRAPH_RANGE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("start", &U64, true), property("end", &U64, true), property("format", &style_actions::PARAGRAPH, true)], additional: false };
pub(super) static PAGE: Schema = Schema::Object { properties: &[
    property("width", &Schema::Unsigned { min: 2880, max: 31680 }, true), property("height", &Schema::Unsigned { min: 2880, max: 31680 }, true),
    property("left", &Schema::Unsigned { min: 0, max: 31680 }, true), property("right", &Schema::Unsigned { min: 0, max: 31680 }, true),
    property("top", &Schema::Unsigned { min: 0, max: 31680 }, true), property("bottom", &Schema::Unsigned { min: 0, max: 31680 }, true),
], additional: false };
pub(super) fn tabs(input: &Value) -> Result<TabStops, ActionError> {
    let mut tabs = TabStops::default();
    for v in input.as_array().ok_or_else(|| invalid("Tabs must be an array"))? {
        let position = v.get("position").and_then(Value::as_i64).ok_or_else(|| invalid("Missing tab position"))? as i32;
        if tabs.as_slice().iter().any(|s| s.position == position) { return Err(invalid("Duplicate tab position")); }
        let alignment = TabAlignment::from_name(&text(v, "alignment")).ok_or_else(|| invalid("Unknown tab alignment"))?;
        let leader = TabLeader::from_name(v.get("leader").and_then(Value::as_str).unwrap_or("none")).ok_or_else(|| invalid("Unknown tab leader"))?;
        tabs.set(TabStop { position, alignment, leader }).map_err(|e| invalid(format!("Invalid tab: {e:?}")))?;
    }
    Ok(tabs)
}
pub(super) fn tabs_json(tabs: TabStops) -> Value {
    Value::Array(tabs.as_slice().iter().map(|s| Value::object([("position", Value::i64(s.position.into())), ("alignment", Value::string(s.alignment.name())), ("leader", Value::string(s.leader.name()))])).collect())
}
pub(super) fn parse(input: &Value) -> Result<ParagraphFormat, ActionError> {
    let s = Value::object([("name", Value::string("Paragraph")), ("paragraph", input.clone())]);
    // Reuse the style adapter's property-patch semantics, including explicit defaults.
    Ok(style_actions::definition(&Document::default(), &s)?.paragraph)
}
pub(super) fn page(input: &Value) -> PageSettings {
    PageSettings { width: id(input, "width") as u32, height: id(input, "height") as u32, left: id(input, "left") as u32,
        right: id(input, "right") as u32, top: id(input, "top") as u32, bottom: id(input, "bottom") as u32 }
}
pub(super) fn page_json(page: PageSettings) -> Value {
    Value::object([("width", Value::u64(page.width.into())), ("height", Value::u64(page.height.into())), ("left", Value::u64(page.left.into())),
        ("right", Value::u64(page.right.into())), ("top", Value::u64(page.top.into())), ("bottom", Value::u64(page.bottom.into()))])
}
