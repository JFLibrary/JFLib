// Document font names and style inheritance. No platform or UI dependencies.
use crate::{Document, Error, Paragraph, ParagraphFormat, ParagraphStyle, ResolvedFormat, TextColor, TextFormat};
use alloc::{string::String, vec::Vec};

/// A validated UTF-8 family name, retained even when the renderer must substitute a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontFamily { bytes: [u8; 96], len: u8 }
impl FontFamily {
    /// Family names contain 1–96 UTF-8 bytes, without controls or surrounding whitespace.
    pub fn new(name: &str) -> Result<Self, Error> {
        if name.is_empty() || name.len() > 96 || name.trim() != name || name.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
        crate::check_text(name)?;
        let mut value = Self { bytes: [0; 96], len: name.len() as u8 };
        value.bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(value)
    }
    /// Original family name, without a style suffix.
    pub fn as_str(&self) -> &str { core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("") }
}
impl Default for FontFamily {
    fn default() -> Self { Self::new("DejaVu Sans").unwrap() }
}

/// One paragraph style. Absent character/paragraph properties inherit from `based_on`.
/// Built-in definitions can be replaced using the same identity. Level 9 means body text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleDefinition {
    pub id: ParagraphStyle,
    pub name: String,
    pub based_on: Option<ParagraphStyle>,
    pub text: TextFormat,
    pub paragraph: ParagraphFormat,
    pub outline_level: Option<u8>,
}
impl StyleDefinition {
    /// Create an otherwise inherited style, initially based on Normal.
    pub fn new(id: ParagraphStyle, name: &str) -> Self {
        Self { id, name: String::from(name), based_on: Some(ParagraphStyle::Normal),
            text: TextFormat::default(), paragraph: ParagraphFormat::default(), outline_level: None }
    }
}

impl Document {
    // Remap complete style graphs before joining rich selections. A conflicting definition
    // receives a new identity; destination styles are never overwritten by paste.
    pub(crate) fn import_fragment_styles(&mut self, incoming: &mut Document) -> Result<(), Error> {
        let mut mapping = alloc::collections::BTreeMap::new();
        let mut pending = incoming.style_ids();
        while !pending.is_empty() {
            let before = pending.len();
            let mut next = Vec::new();
            for id in pending {
                let mut s = incoming.style_definition(id).ok_or(Error::Missing)?;
                if let Some(base) = s.based_on {
                    if let Some(mapped) = mapping.get(&base) { s.based_on = Some(*mapped); }
                    else { next.push(id); continue; }
                }
                let destination = if self.style_definition(id).as_ref() == Some(&s) { id } else {
                    // Reuse an identical named definition independently of its allocated ID.
                    let same = self.styles.iter().find(|other| other.based_on == s.based_on && other.text == s.text
                        && other.paragraph == s.paragraph && other.outline_level == s.outline_level && other.name == s.name);
                    if let Some(other) = same { other.id } else {
                        s.id = self.next_style_id();
                        let name = s.name.clone();
                        let mut suffix = 2;
                        while self.style_ids().iter().any(|id| self.style_definition(*id).is_some_and(|d| d.name.eq_ignore_ascii_case(&s.name))) {
                            s.name = alloc::format!("{name} ({suffix})"); suffix += 1;
                        }
                        let id = s.id;
                        self.styles.push(s);
                        id
                    }
                };
                mapping.insert(id, destination);
            }
            if next.len() == before { return Err(Error::Invalid); }
            pending = next;
        }
        for p in incoming.paragraphs.iter_mut().chain(incoming.notes.iter_mut().flat_map(|n| &mut n.paragraphs)).chain(incoming.tables.iter_mut().flat_map(|t| &mut t.rows).flat_map(|r| &mut r.cells).flat_map(|c| &mut c.paragraphs)) {
            p.style = *mapping.get(&p.style).ok_or(Error::Missing)?;
        }
        Ok(())
    }
    /// Explicit definition, or the original built-in definition when unmodified.
    pub fn style_definition(&self, id: ParagraphStyle) -> Option<StyleDefinition> {
        if let Some(s) = self.styles.iter().find(|s| s.id == id) { return Some(s.clone()); }
        let (name, size, bold, color, level) = match id {
            ParagraphStyle::Normal => ("Standard", None, None, None, Some(9)),
            ParagraphStyle::Title => ("Titel", Some(48), None, Some(0x176b70), Some(9)),
            ParagraphStyle::Heading1 => ("Überschrift 1", Some(32), Some(true), Some(0x176b70), Some(0)),
            ParagraphStyle::Heading2 => ("Überschrift 2", Some(26), Some(true), None, Some(1)),
            ParagraphStyle::Named(_) => return None,
        };
        let mut s = StyleDefinition::new(id, name);
        if id == ParagraphStyle::Normal { s.based_on = None; }
        s.text.size_half_points = size;
        s.text.bold = bold;
        s.text.color = color.map(TextColor::Rgb);
        s.outline_level = level;
        Some(s)
    }
    /// Built-in and custom styles, in stable gallery order.
    pub fn style_ids(&self) -> Vec<ParagraphStyle> {
        let mut ids = alloc::vec![ParagraphStyle::Normal, ParagraphStyle::Title, ParagraphStyle::Heading1, ParagraphStyle::Heading2];
        ids.extend(self.styles.iter().filter_map(|s| matches!(s.id, ParagraphStyle::Named(_)).then_some(s.id)));
        ids
    }
    /// Next unused custom identity. Style identities occupy a separate namespace.
    pub fn next_style_id(&self) -> ParagraphStyle {
        let mut id = 1;
        while self.styles.iter().any(|s| s.id == ParagraphStyle::Named(id)) { id += 1; }
        ParagraphStyle::Named(id)
    }
    fn style_chain(&self, id: ParagraphStyle) -> Result<Vec<StyleDefinition>, Error> {
        let mut chain: Vec<StyleDefinition> = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next {
            if chain.iter().any(|s| s.id == id) { return Err(Error::Invalid); }
            if chain.len() >= 32 { return Err(Error::Limit); }
            let s = self.style_definition(id).ok_or(Error::Missing)?;
            next = s.based_on;
            chain.push(s);
        }
        Ok(chain)
    }
    /// Resolve style inheritance followed by direct character formatting.
    pub fn resolve_text(&self, direct: TextFormat, style: ParagraphStyle) -> ResolvedFormat {
        self.resolve_text_base(TextFormat::default(), direct, style)
    }
    pub(crate) fn resolve_text_base(&self, mut f: TextFormat, direct: TextFormat, style: ParagraphStyle) -> ResolvedFormat {
        if let Ok(chain) = self.style_chain(style) {
            for s in chain.iter().rev() { f = f.overlay(s.text); }
        }
        f.overlay(direct).resolve(ParagraphStyle::Normal)
    }
    /// Resolve inherited paragraph properties followed by per-property direct overrides.
    pub fn resolve_paragraph(&self, p: &Paragraph) -> ParagraphFormat {
        let mut f = ParagraphFormat::default();
        if let Ok(chain) = self.style_chain(p.style) {
            for s in chain.iter().rev() { f = s.paragraph.over(f); }
        }
        p.format.over(f)
    }
    /// Outline level for navigation and heading pagination; body text has no level.
    pub fn outline_level(&self, style: ParagraphStyle) -> Option<u8> {
        self.style_chain(style).ok()?.iter().find_map(|s| s.outline_level).filter(|n| *n < 9)
    }
    pub(crate) fn validate_styles(&self) -> Result<(), Error> {
        if self.styles.len() > 128 { return Err(Error::Limit); }
        for (i, s) in self.styles.iter().enumerate() {
            if s.name.is_empty() || s.name.trim() != s.name || s.name.len() > 128 || s.name.chars().any(char::is_control)
                || matches!(s.id, ParagraphStyle::Named(0 | 0xffff_ffff))
                || s.outline_level.is_some_and(|n| n > 9)
                || self.styles[..i].iter().any(|other| other.id == s.id || other.name.eq_ignore_ascii_case(&s.name)) {
                return Err(Error::Invalid);
            }
            s.text.validate()?;
            crate::check_text(&s.name)?;
            s.paragraph.validate_overrides()?;
        }
        let mut names: Vec<String> = Vec::new();
        for id in self.style_ids() {
            let chain = self.style_chain(id)?;
            let name = &chain[0].name;
            if names.iter().any(|other| other.eq_ignore_ascii_case(name)) { return Err(Error::Invalid); }
            names.push(name.clone());
            let mut paragraph = ParagraphFormat::default();
            for s in chain.iter().rev() { paragraph = s.paragraph.over(paragraph); }
            paragraph.validate(self.page)?;
        }
        Ok(())
    }
}
