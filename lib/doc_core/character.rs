// Portable character properties shared by editing, styles and interchange.

/// Word's sixteen highlight colors plus an explicit removal of inherited highlighting.
/// `None` on TextFormat inherits; `Some(Highlight::None)` clears it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Highlight {
    #[default]
    None,
    Yellow, Green, Cyan, Magenta, Blue, Red, DarkBlue, DarkCyan,
    DarkGreen, DarkMagenta, DarkRed, DarkYellow, DarkGray, LightGray, Black, White,
}
impl Highlight {
    /// Stable palette order; these are document colors independent of the application theme.
    pub const ALL: [Self; 17] = [Self::None, Self::Yellow, Self::Green, Self::Cyan,
        Self::Magenta, Self::Blue, Self::Red, Self::DarkBlue, Self::DarkCyan,
        Self::DarkGreen, Self::DarkMagenta, Self::DarkRed, Self::DarkYellow,
        Self::DarkGray, Self::LightGray, Self::Black, Self::White];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none", Self::Yellow => "yellow", Self::Green => "green",
            Self::Cyan => "cyan", Self::Magenta => "magenta", Self::Blue => "blue",
            Self::Red => "red", Self::DarkBlue => "darkBlue", Self::DarkCyan => "darkCyan",
            Self::DarkGreen => "darkGreen", Self::DarkMagenta => "darkMagenta",
            Self::DarkRed => "darkRed", Self::DarkYellow => "darkYellow",
            Self::DarkGray => "darkGray", Self::LightGray => "lightGray",
            Self::Black => "black", Self::White => "white",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == name)
    }
    /// Opaque RGB; absence means that the paper remains visible.
    pub fn rgb(self) -> Option<u32> {
        Some(match self {
            Self::None => return None, Self::Yellow => 0xffff00, Self::Green => 0x00ff00,
            Self::Cyan => 0x00ffff, Self::Magenta => 0xff00ff, Self::Blue => 0x0000ff,
            Self::Red => 0xff0000, Self::DarkBlue => 0x000080, Self::DarkCyan => 0x008080,
            Self::DarkGreen => 0x008000, Self::DarkMagenta => 0x800080,
            Self::DarkRed => 0x800000, Self::DarkYellow => 0x808000,
            Self::DarkGray => 0x808080, Self::LightGray => 0xc0c0c0,
            Self::Black => 0, Self::White => 0xffffff,
        })
    }
    /// Exact palette match, never an approximate conversion of imported colors.
    pub fn from_rgb(rgb: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.rgb() == Some(rgb))
    }
}

/// Mutually exclusive script positions; Baseline explicitly clears inherited positioning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextPositioning {
    #[default]
    Baseline,
    Superscript,
    Subscript,
}
impl TextPositioning {
    pub fn name(self) -> &'static str {
        match self { Self::Baseline => "baseline", Self::Superscript => "superscript", Self::Subscript => "subscript" }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match name { "baseline" => Some(Self::Baseline), "superscript" => Some(Self::Superscript),
            "subscript" => Some(Self::Subscript), _ => None }
    }
}
