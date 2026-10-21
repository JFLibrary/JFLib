// Document formatting in typographic units; independent of pixels, GUI styles and codecs.
use crate::{Error, FontFamily, Highlight, ParagraphStyle, TextPositioning};

/// Direct WordprocessingML text color; automatic remains distinct from inherited color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColor {
    Auto,
    /// Opaque RGB (0x000000..=0xFFFFFF), without an alpha channel.
    Rgb(u32),
}

/// Explicit character overrides. None inherits the paragraph style; false can disable bold headings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFormat {
    pub font_family: Option<FontFamily>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<TextColor>,
    pub size_half_points: Option<u16>,
    pub highlight: Option<Highlight>,
    pub positioning: Option<TextPositioning>,
}
impl TextFormat {
    /// Keep only equal, effective properties when summarizing a selection. A missing
    /// property in the result means mixed formatting, not an instruction to change text.
    pub fn common(self, other: Self) -> Self {
        Self {
            font_family: self.font_family.filter(|v| Some(*v) == other.font_family),
            bold: self.bold.filter(|v| Some(*v) == other.bold),
            italic: self.italic.filter(|v| Some(*v) == other.italic),
            underline: self.underline.filter(|v| Some(*v) == other.underline),
            strike: self.strike.filter(|v| Some(*v) == other.strike),
            color: self.color.filter(|v| Some(*v) == other.color),
            size_half_points: self.size_half_points.filter(|v| Some(*v) == other.size_half_points),
            highlight: self.highlight.filter(|v| Some(*v) == other.highlight),
            positioning: self.positioning.filter(|v| Some(*v) == other.positioning),
        }
    }
    /// Capture the original built-in defaults. For a document's named or modified styles,
    /// use `Document::resolve_text(...).explicit()` to include the document's inheritance.
    pub fn explicit(self, style: ParagraphStyle) -> Self {
        let resolved = self.resolve(style);
        Self {
            font_family: Some(resolved.font_family),
            bold: Some(resolved.bold), italic: Some(resolved.italic),
            underline: Some(resolved.underline), strike: Some(resolved.strike),
            color: Some(TextColor::Rgb(resolved.color & 0xffffff)),
            size_half_points: Some(resolved.size_half_points),
            highlight: Some(resolved.highlight), positioning: Some(resolved.positioning),
        }
    }
    /// Apply only properties present in `direct`, preserving all other overrides.
    pub fn overlay(self, direct: Self) -> Self {
        Self { font_family: direct.font_family.or(self.font_family), bold: direct.bold.or(self.bold),
            italic: direct.italic.or(self.italic), underline: direct.underline.or(self.underline),
            strike: direct.strike.or(self.strike), color: direct.color.or(self.color),
            size_half_points: direct.size_half_points.or(self.size_half_points),
            highlight: direct.highlight.or(self.highlight), positioning: direct.positioning.or(self.positioning) }
    }
    /// Supported size range: 4–144 points, in half-point increments.
    pub fn validate(self) -> Result<(), Error> {
        if matches!(self.color, Some(TextColor::Rgb(n)) if n > 0xffffff)
            || self
                .size_half_points
                .is_some_and(|n| !(8..=288).contains(&n))
        {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    /// Resolve the original built-in defaults. Use `Document::resolve_text` for style graphs.
    pub fn resolve(self, style: ParagraphStyle) -> ResolvedFormat {
        let (size, bold, color) = match style {
            ParagraphStyle::Normal | ParagraphStyle::Named(_) => (22, false, 0xff20272b),
            ParagraphStyle::Title => (48, false, 0xff176b70),
            ParagraphStyle::Heading1 => (32, true, 0xff176b70),
            ParagraphStyle::Heading2 => (26, true, 0xff20272b),
        };
        ResolvedFormat {
            font_family: self.font_family.unwrap_or_default(),
            bold: self.bold.unwrap_or(bold),
            italic: self.italic.unwrap_or(false),
            underline: self.underline.unwrap_or(false),
            strike: self.strike.unwrap_or(false),
            size_half_points: self.size_half_points.unwrap_or(size),
            highlight: self.highlight.unwrap_or_default(), positioning: self.positioning.unwrap_or_default(),
            color: match self.color {
                Some(TextColor::Auto) => 0xff000000,
                Some(TextColor::Rgb(rgb)) => 0xff000000 | rgb,
                None => color,
            },
        }
    }
}
/// Fully resolved attributes consumed by layout and rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedFormat {
    pub font_family: FontFamily,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub size_half_points: u16,
    pub color: u32,
    pub highlight: Highlight,
    pub positioning: TextPositioning,
}
impl ResolvedFormat {
    /// Capture all resolved properties as direct formatting for cross-style transfers.
    pub fn explicit(self) -> TextFormat {
        TextFormat { font_family: Some(self.font_family), bold: Some(self.bold), italic: Some(self.italic),
            underline: Some(self.underline), strike: Some(self.strike), color: Some(TextColor::Rgb(self.color & 0xffffff)),
            size_half_points: Some(self.size_half_points), highlight: Some(self.highlight), positioning: Some(self.positioning) }
    }
    /// Shared typographic policy: scripts use two thirds of the nominal size, rounded to
    /// half points. The nominal size remains unchanged in the document and file formats.
    pub fn display_size_half_points(self) -> u16 {
        if self.positioning == TextPositioning::Baseline { self.size_half_points }
        else { ((u32::from(self.size_half_points) * 2 + 1) / 3).max(1) as u16 }
    }
    /// Baseline displacement in twips, positive downwards. GUI zoom never enters this value.
    pub fn baseline_shift(self) -> i32 {
        let em = i32::from(self.size_half_points) * 10;
        match self.positioning { TextPositioning::Baseline => 0,
            TextPositioning::Superscript => -em / 3, TextPositioning::Subscript => em / 5 }
    }
}
/// Horizontal alignment of a paragraph's lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}
/// Paragraph overrides. Distances are twips (1/20 point); line spacing is in 1/240 lines.
#[derive(Clone, Copy, Debug, Eq)]
pub struct ParagraphFormat {
    /// Explicit default-valued overrides: alignment, before, after, line, left, right,
    /// first-line, page break, tabs (bits 0..8). Non-default values are always overrides.
    pub explicit: u16,
    pub alignment: Alignment,
    pub before: u16,
    pub after: u16,
    pub line: u16,
    pub left: u16,
    pub right: u16,
    /// Additional first-line indent in twips; negative values create a hanging indent.
    /// Applies once per paragraph, including paragraphs that continue on another page.
    pub first_line: i32,
    pub page_break_before: bool,
    /// Custom tab declarations, merged by position through the style hierarchy (bit 8).
    pub tabs: crate::TabStops,
}
impl PartialEq for ParagraphFormat {
    fn eq(&self, other: &Self) -> bool {
        self.mask() == other.mask() && self.alignment == other.alignment && self.before == other.before
            && self.after == other.after && self.line == other.line && self.left == other.left
            && self.right == other.right && self.first_line == other.first_line && self.page_break_before == other.page_break_before
            && self.tabs == other.tabs
    }
}
impl Default for ParagraphFormat {
    fn default() -> Self {
        Self {
            explicit: 0,
            alignment: Alignment::Left,
            before: 0,
            after: 160,
            line: 276,
            left: 0,
            right: 0,
            first_line: 0,
            page_break_before: false,
            tabs: Default::default(),
        }
    }
}
impl ParagraphFormat {
    /// Retain only selected properties as an explicit patch (bits follow `explicit`).
    pub fn select(self, mask: u16) -> Self {
        let mut f = Self { explicit: mask, ..Self::default() };
        if mask & 1 != 0 { f.alignment = self.alignment; }
        if mask & 2 != 0 { f.before = self.before; }
        if mask & 4 != 0 { f.after = self.after; }
        if mask & 8 != 0 { f.line = self.line; }
        if mask & 16 != 0 { f.left = self.left; }
        if mask & 32 != 0 { f.right = self.right; }
        if mask & 64 != 0 { f.first_line = self.first_line; }
        if mask & 128 != 0 { f.page_break_before = self.page_break_before; }
        if mask & 256 != 0 { f.tabs = self.tabs; }
        f
    }
    /// Merge a property patch without making untouched inherited values explicit.
    pub fn overlay(self, patch: Self) -> Self {
        let mut f = patch.over(self);
        f.explicit = self.mask() | patch.mask();
        f.tabs = self.tabs.merge(patch.tabs, false);
        f
    }
    /// The properties this value overrides. This retains compatibility with existing
    /// callers that construct non-default properties with a struct update.
    pub fn mask(self) -> u16 {
        let d = Self::default();
        let values = [self.alignment != d.alignment, self.before != d.before, self.after != d.after,
            self.line != d.line, self.left != d.left, self.right != d.right,
            self.first_line != d.first_line, self.page_break_before != d.page_break_before, !self.tabs.is_empty()];
        values.iter().enumerate().fold(self.explicit, |m, (i, set)| m | if *set { 1 << i } else { 0 })
    }
    /// Apply this set of overrides to an inherited paragraph format.
    pub fn over(self, mut base: Self) -> Self {
        let m = self.mask();
        if m & 1 != 0 { base.alignment = self.alignment; }
        if m & 2 != 0 { base.before = self.before; }
        if m & 4 != 0 { base.after = self.after; }
        if m & 8 != 0 { base.line = self.line; }
        if m & 16 != 0 { base.left = self.left; }
        if m & 32 != 0 { base.right = self.right; }
        if m & 64 != 0 { base.first_line = self.first_line; }
        if m & 128 != 0 { base.page_break_before = self.page_break_before; }
        if m & 256 != 0 { base.tabs = base.tabs.merge(self.tabs, true); }
        base.explicit = 0;
        base
    }
    pub(crate) fn validate_overrides(self) -> Result<(), Error> {
        self.tabs.validate()?;
        if self.explicit & !511 != 0 || self.before > 2880
            || self.after > 2880
            || !(240..=720).contains(&self.line)
            || self.left > 31680 || self.right > 31680
            || !(-31680..=31680).contains(&self.first_line)
        { Err(Error::Invalid) } else { Ok(()) }
    }
    /// Validate resolved spacing and ensure that the paragraph retains a usable text area.
    pub fn validate(self, page: PageSettings) -> Result<(), Error> {
        self.validate_overrides()?;
        if self.left as u32 + self.right as u32 + 360 > page.content_width()
            || i64::from(page.left) + i64::from(self.left) + i64::from(self.first_line) < 0
            || i64::from(self.left) + i64::from(self.first_line) + i64::from(self.right) + 360
                > i64::from(page.content_width())
        {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
}
/// One document-wide page setup in twips. A4 portrait with 25 mm margins by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSettings {
    pub width: u32,
    pub height: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}
impl Default for PageSettings {
    fn default() -> Self {
        Self {
            width: 11906,
            height: 16838,
            top: 1417,
            right: 1417,
            bottom: 1417,
            left: 1417,
        }
    }
}
impl PageSettings {
    /// Remaining line width after page margins; saturating for unvalidated public models.
    pub fn content_width(self) -> u32 {
        self.width
            .saturating_sub(self.left.saturating_add(self.right))
    }
    /// Remaining text height after page margins.
    pub fn content_height(self) -> u32 {
        self.height
            .saturating_sub(self.top.saturating_add(self.bottom))
    }
    /// Keep arithmetic bounded and at least two inches available for text in each direction.
    pub fn validate(self) -> Result<(), Error> {
        if !(2880..=31680).contains(&self.width)
            || !(2880..=31680).contains(&self.height)
            || self.content_width() < 2880
            || self.content_height() < 2880
        {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
}
