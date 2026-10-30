// theme_app -- app themes: the active style's tokens overridden from a small text file
//
//   Theme::from_ini(text) -> Result<Theme, ThemeError>   parse a theme (the format below)
//   Theme::load(path) -> Result<Theme, ThemeError>       ... from a file (app://, config://, a
//                              path); relative font files are taken from the theme's folder
//   theme.to_ini() -> String / theme.save(path) -> bool  the text form of what the theme sets
//   token_index(name) -> Option<usize>   the T_* index (token / 4) of a token name ("bg")
//   TOKEN_NAMES, TS_NAMES      the names of the tokens and of the text styles
//
// The format is INI (base::ini):
//   [theme]     name = the theme's name
//   [colors]    TOKEN = #RRGGBB | #AARRGGBB for light and dark; [light], [dark] for one of
//               them, [contrast] for high contrast (on top of the scheme's colours).  TOKEN is
//               a T_* name in lower case (bg, card, ctrl_stroke, text2, accent_text, ...);
//               accent is the colour the style derives its accent ramp from (as accent_set;
//               high contrast: the accent fills)
//   [metrics]   radius = DIP | PERCENT%   rounded corners that are not pills (0: square)
//               spacing = PERCENT%        padding and gaps of boxes and grid boxes
//               control_height = DIP      buttons, text boxes, combo boxes, drop-down and split
//                                         buttons (not compact or large ones)
//   [text]      family = FILE | installed family   (family.semibold, family.bold: their faces;
//               else from the family: a file gets a synthetic bold, a variable one its wght);
//               family.axes = TAG=VALUE, ...   axis values of a variable family (wdth=87.5,
//               opsz=12; weights still come from the faces' 400 / 600 / 700, opsz else 11 pt);
//               STYLE.size = DIP (24.8), STYLE.line = DIP, STYLE.weight = regular | semibold |
//               bold | 100..900 for STYLE in caption, body, body_strong, subtitle, title,
//               title_large, display
// Errors name the line (in the UI language): an unknown section or key, a line without '=',
// a key before the first section or twice in one section, a value out of range.  A theme is
// applied whole or not at all (Ui::set_theme).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::ini::{Ini, Item};
use base::num::f64_parse;

use crate::consts::*;
use crate::i18n::{fill, tr};

/// Theme::colors / accent index: the light theme
pub const THEME_LIGHT: usize = 0;
/// ... the dark theme
pub const THEME_DARK: usize = 1;
/// ... high contrast
pub const THEME_CONTRAST: usize = 2;

/// the T_* tokens by name, in token order (index = token / 4); theme files write them in
/// lower case
pub const TOKEN_NAMES: [&str; T_COUNT] = [
    "BG", "BG_ALT", "LAYER", "LAYER_STROKE", "CARD", "CARD_STROKE", "CTRL", "CTRL_HOVER", "CTRL_PRESSED", "CTRL_DISABLED",
    "CTRL_STROKE", "CTRL_STROKE2", "STRONG_STROKE", "SUBTLE", "SUBTLE_HOVER", "SUBTLE_PRESSED", "SUBTLE_DISABLED", "TEXT",
    "TEXT2", "TEXT3", "TEXT_DISABLED", "ACCENT", "ACCENT_HOVER", "ACCENT_PRESSED", "ACCENT_DISABLED", "ON_ACCENT",
    "ON_ACCENT_DIS", "ACCENT_TEXT", "FOCUS_OUT", "FOCUS_IN", "FLYOUT", "FLYOUT_STROKE", "SMOKE", "DIVIDER", "INPUT_ACTIVE",
    "CRIT", "OK", "WARN", "INFO", "CRIT_BG", "OK_BG", "WARN_BG", "INFO_BG", "CLOSE_HOVER", "SHADOW", "SCROLL", "TRACK",
    "KNOB", "ALT_FILL", "STRONG_FILL",
];

/// the TS_* text styles by name (theme files: body.size, title.weight)
pub const TS_NAMES: [&str; TS_COUNT as usize] = ["caption", "body", "body_strong", "subtitle", "title", "title_large", "display"];

const SECTIONS: [&str; 7] = ["theme", "colors", "light", "dark", "contrast", "metrics", "text"];
const MODES: [&str; 3] = ["light", "dark", "contrast"];

/// the T_* index of a token name (any case, "T_" in front allowed)
pub fn token_index(name: &str) -> Option<usize> {
    let n = name.strip_prefix("T_").or_else(|| name.strip_prefix("t_")).unwrap_or(name);
    TOKEN_NAMES.iter().position(|t| t.eq_ignore_ascii_case(n))
}

/// rounded corners of a theme: every corner radius that is not a pill becomes this many DIP,
/// or the style's radius scaled by this percentage
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radius {
    Dip(u16),
    Percent(u16),
}

/// a text style of a theme: size in DIP (24.8), line height in DIP, weight 100..900; None =
/// the style's (a size alone scales the style's line height with it)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub size_fx: Option<u16>,
    pub line: Option<u16>,
    pub weight: Option<u16>,
}

/// an app theme (see the module header); Default = nothing overridden
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    /// colour tokens per THEME_LIGHT / _DARK / _CONTRAST (index token / 4), 0xAARRGGBB
    pub colors: [[Option<u32>; T_COUNT]; 3],
    /// the accent per mode, 0xRRGGBB: the style derives its accent ramp from it
    pub accent: [Option<u32>; 3],
    pub radius: Option<Radius>,
    /// box padding and gaps in percent (100: the app's values)
    pub spacing: u16,
    /// DIP; 0 = the style's
    pub control_height: u16,
    pub text: [TextStyle; TS_COUNT as usize],
    /// "" = the style's faces; a font file (VFS path) or an installed family
    pub family: String,
    pub family_semibold: String,
    pub family_bold: String,
    /// axis values of a variable family (tag, user units); empty: its defaults
    pub family_axes: Vec<([u8; 4], f32)>,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            name: String::new(),
            colors: [[None; T_COUNT]; 3],
            accent: [None; 3],
            radius: None,
            spacing: 100,
            control_height: 0,
            text: [TextStyle::default(); TS_COUNT as usize],
            family: String::new(),
            family_semibold: String::new(),
            family_bold: String::new(),
            family_axes: Vec::new(),
        }
    }
}

/// why a theme could not be read or applied: the line of the file (0: not a line) and the
/// message in the UI language ("Zeile 4: ...")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeError {
    pub line: u32,
    pub msg: String,
}

impl core::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl ThemeError {
    // a message of the library (tr) with its arguments; the line number comes first
    fn at(line: usize, text: &'static str, args: &[&str]) -> ThemeError {
        let n = format!("{}", line);
        let mut all: Vec<&str> = Vec::with_capacity(args.len() + 1);
        all.push(&n);
        all.extend_from_slice(args);
        ThemeError { line: line as u32, msg: fill(tr(text), &all) }
    }

    /// an error that belongs to no line (a file that cannot be read, a font)
    pub(crate) fn general(text: &'static str, arg: &str) -> ThemeError {
        ThemeError { line: 0, msg: fill(tr(text), &[arg]) }
    }
}

const E_SECTION: &str = "Zeile {}: unbekannter Abschnitt [{}]";
const E_KEY: &str = "Zeile {}: unbekannter Schlüssel „{}“ in [{}]";
const E_TWICE: &str = "Zeile {}: „{}“ steht zweimal in [{}]";
const E_LINE: &str = "Zeile {}: „Schlüssel = Wert“ erwartet";
const E_OUTSIDE: &str = "Zeile {}: „{}“ steht vor dem ersten Abschnitt";
const E_COLOR: &str = "Zeile {}: „{}“ ist keine Farbe (#RRGGBB oder #AARRGGBB)";
const E_NUMBER: &str = "Zeile {}: „{}“ ist keine Zahl von {} bis {}";
const E_WEIGHT: &str = "Zeile {}: „{}“ ist keine Schriftstärke (regular, semibold, bold, 100 bis 900)";
const E_AXES: &str = "Zeile {}: „{}“ sind keine Achsenwerte (wght=650, wdth=87.5 …)";
pub(crate) const E_READ: &str = "Theme-Datei nicht lesbar: {}";
pub(crate) const E_FONT: &str = "Schrift nicht ladbar: {}";

/// #RGB, #RRGGBB (opaque) or #AARRGGBB -> 0xAARRGGBB
pub(crate) fn parse_color(v: &str) -> Option<u32> {
    let h = v.strip_prefix('#')?;
    if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let x = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        3 => {
            let (r, g, b) = ((x >> 8) & 15, (x >> 4) & 15, x & 15);
            Some(0xFF00_0000 | (r * 17) << 16 | (g * 17) << 8 | b * 17)
        }
        6 => Some(0xFF00_0000 | x),
        8 => Some(x),
        _ => None,
    }
}

fn color_text(c: u32) -> String {
    if c >> 24 == 0xFF { format!("#{:06X}", c & 0xFF_FFFF) } else { format!("#{:08X}", c) }
}

// a whole number lo..=hi (optionally with a trailing '%' when pct)
fn parse_num(v: &str, lo: u32, hi: u32, pct: bool) -> Option<(u32, bool)> {
    let (t, is_pct) = match v.strip_suffix('%') {
        Some(t) if pct => (t.trim_end(), true),
        _ => (v, false),
    };
    let n: u32 = t.parse().ok()?;
    (lo..=hi).contains(&n).then_some((n, is_pct))
}

// a size in DIP with decimals -> 24.8
fn parse_size(v: &str) -> Option<u16> {
    let (x, used) = f64_parse(v.as_bytes());
    if used == 0 || used != v.len() || !(4.0..=200.0).contains(&x) {
        return None;
    }
    Some((x * 256.0 + 0.5) as u16)
}

fn parse_weight(v: &str) -> Option<u16> {
    match v.to_ascii_lowercase().as_str() {
        "regular" | "normal" => Some(400),
        "medium" => Some(500),
        "semibold" => Some(600),
        "bold" => Some(700),
        _ => v.parse::<u16>().ok().filter(|w| (100..=900).contains(w)),
    }
}

// "wdth=87.5, opsz=12" (also "wdth 87.5", tags of 1..4 characters, quoted or not) ->
// [(tag, value)]; None: not that form
pub(crate) fn parse_axes(v: &str) -> Option<Vec<([u8; 4], f32)>> {
    let mut out = Vec::new();
    for item in v.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (t, x) = item.split_once(['=', ' '])?;
        let t = t.trim().trim_matches(['"', '\'']);
        let x = x.trim();
        let (val, used) = f64_parse(x.as_bytes());
        if t.is_empty() || t.len() > 4 || !t.bytes().all(|c| (0x21..0x7F).contains(&c)) || used == 0 || used != x.len() {
            return None;
        }
        let mut tag = [b' '; 4];
        tag[..t.len()].copy_from_slice(t.as_bytes());
        out.push((tag, val as f32));
    }
    (!out.is_empty()).then_some(out)
}

fn axes_text(axes: &[([u8; 4], f32)]) -> String {
    let items: Vec<String> = axes
        .iter()
        .map(|(t, v)| format!("{}={}", String::from_utf8_lossy(t).trim_end(), base::num::f64_format(*v as f64)))
        .collect();
    items.join(", ")
}

fn weight_text(w: u16) -> String {
    match w {
        400 => String::from("regular"),
        600 => String::from("semibold"),
        700 => String::from("bold"),
        _ => format!("{}", w),
    }
}

fn size_text(fx: u16) -> String {
    if fx & 255 == 0 { format!("{}", fx >> 8) } else { base::num::f64_format(fx as f64 / 256.0) }
}

impl Theme {
    /// parse a theme file's text (module header); every error names its line
    pub fn from_ini(text: &str) -> Result<Theme, ThemeError> {
        let ini = Ini::parse(text.as_bytes());
        let mut th = Theme::default();
        let mut sec: Option<usize> = None; // index into SECTIONS
        let mut seen: Vec<String> = Vec::new(); // the keys of the current section
        for item in ini.items() {
            match item {
                Item::Section { line, name } => {
                    let Some(i) = SECTIONS.iter().position(|s| s.eq_ignore_ascii_case(name)) else {
                        return Err(ThemeError::at(line, E_SECTION, &[name]));
                    };
                    sec = Some(i);
                    seen.clear();
                }
                Item::Bad { line, .. } => return Err(ThemeError::at(line, E_LINE, &[])),
                Item::Key { line, key, value, .. } => {
                    let Some(s) = sec else { return Err(ThemeError::at(line, E_OUTSIDE, &[key])) };
                    let low = key.to_ascii_lowercase();
                    if seen.contains(&low) {
                        return Err(ThemeError::at(line, E_TWICE, &[key, SECTIONS[s]]));
                    }
                    seen.push(low);
                    th.set_key(SECTIONS[s], key, value, line)?;
                }
            }
        }
        Ok(th)
    }

    // one key of a section
    fn set_key(&mut self, sec: &str, key: &str, v: &str, line: usize) -> Result<(), ThemeError> {
        let unknown = || ThemeError::at(line, E_KEY, &[key, sec]);
        let num = |lo: u32, hi: u32| ThemeError::at(line, E_NUMBER, &[v, &format!("{}", lo), &format!("{}", hi)]);
        match sec {
            "theme" => {
                if !key.eq_ignore_ascii_case("name") {
                    return Err(unknown());
                }
                self.name = String::from(v);
            }
            "colors" | "light" | "dark" | "contrast" => {
                let modes: &[usize] = match sec {
                    "colors" => &[THEME_LIGHT, THEME_DARK],
                    "light" => &[THEME_LIGHT],
                    "dark" => &[THEME_DARK],
                    _ => &[THEME_CONTRAST],
                };
                let accent = key.eq_ignore_ascii_case("accent");
                let tok = if accent { None } else { Some(token_index(key).ok_or_else(unknown)?) };
                let c = parse_color(v).ok_or_else(|| ThemeError::at(line, E_COLOR, &[v]))?;
                for &m in modes {
                    match tok {
                        Some(t) => self.colors[m][t] = Some(c),
                        None => self.accent[m] = Some(c & 0xFF_FFFF),
                    }
                }
            }
            "metrics" => match key.to_ascii_lowercase().as_str() {
                "radius" => {
                    self.radius = Some(match v.strip_suffix('%') {
                        Some(_) => Radius::Percent(parse_num(v, 0, 400, true).ok_or_else(|| num(0, 400))?.0 as u16),
                        None => Radius::Dip(parse_num(v, 0, 64, false).ok_or_else(|| num(0, 64))?.0 as u16),
                    });
                }
                "spacing" => self.spacing = parse_num(v, 25, 400, true).ok_or_else(|| num(25, 400))?.0 as u16,
                "control_height" => self.control_height = parse_num(v, 12, 96, false).ok_or_else(|| num(12, 96))?.0 as u16,
                _ => return Err(unknown()),
            },
            _ => {
                // [text]
                let low = key.to_ascii_lowercase();
                match low.as_str() {
                    "family" => self.family = String::from(v),
                    "family.semibold" => self.family_semibold = String::from(v),
                    "family.bold" => self.family_bold = String::from(v),
                    "family.axes" => self.family_axes = parse_axes(v).ok_or_else(|| ThemeError::at(line, E_AXES, &[v]))?,
                    _ => {
                        let (ts, what) = low.split_once('.').ok_or_else(unknown)?;
                        let i = TS_NAMES.iter().position(|n| *n == ts).ok_or_else(unknown)?;
                        let t = &mut self.text[i];
                        match what {
                            "size" => t.size_fx = Some(parse_size(v).ok_or_else(|| num(4, 200))?),
                            "line" => t.line = Some(parse_num(v, 4, 300, false).ok_or_else(|| num(4, 300))?.0 as u16),
                            "weight" => t.weight = Some(parse_weight(v).ok_or_else(|| ThemeError::at(line, E_WEIGHT, &[v]))?),
                            _ => return Err(unknown()),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// read and parse a theme file; relative font files then name files in its folder
    pub fn load(path: &str) -> Result<Theme, ThemeError> {
        let data = plat::read_file(path).ok_or_else(|| ThemeError::general(E_READ, path))?;
        let text = core::str::from_utf8(&data).map_err(|_| ThemeError::general(E_READ, path))?;
        let mut th = Theme::from_ini(text)?;
        let dir = match path.rfind(['/', '\\']) {
            Some(i) => &path[..i + 1],
            None => "",
        };
        for f in [&mut th.family, &mut th.family_semibold, &mut th.family_bold] {
            if !dir.is_empty() && is_file(f) && !is_absolute(f) {
                *f = format!("{}{}", dir, f);
            }
        }
        Ok(th)
    }

    /// the theme as the text of a theme file (only what it sets)
    pub fn to_ini(&self) -> String {
        String::from_utf8(self.ini().to_bytes()).unwrap_or_default()
    }

    /// write the theme file (missing folders are made) -> written
    pub fn save(&self, path: &str) -> bool {
        self.ini().save(path)
    }

    fn ini(&self) -> Ini {
        let mut ini = Ini::parse(b"# JFLib app theme (docs/STYLES.md, \"App themes\")\n");
        if !self.name.is_empty() {
            ini.set("theme", "name", &self.name);
        }
        for (m, sec) in MODES.iter().enumerate() {
            if let Some(a) = self.accent[m] {
                ini.set(sec, "accent", &color_text(a | 0xFF00_0000));
            }
            for (i, c) in self.colors[m].iter().enumerate() {
                if let Some(c) = c {
                    ini.set(sec, &TOKEN_NAMES[i].to_ascii_lowercase(), &color_text(*c));
                }
            }
        }
        match self.radius {
            Some(Radius::Dip(d)) => {
                ini.set("metrics", "radius", &format!("{}", d));
            }
            Some(Radius::Percent(p)) => {
                ini.set("metrics", "radius", &format!("{}%", p));
            }
            None => {}
        }
        if self.spacing != 100 {
            ini.set("metrics", "spacing", &format!("{}%", self.spacing));
        }
        if self.control_height != 0 {
            ini.set("metrics", "control_height", &format!("{}", self.control_height));
        }
        for (k, v) in [("family", &self.family), ("family.semibold", &self.family_semibold), ("family.bold", &self.family_bold)] {
            if !v.is_empty() {
                ini.set("text", k, v);
            }
        }
        if !self.family_axes.is_empty() {
            ini.set("text", "family.axes", &axes_text(&self.family_axes));
        }
        for (i, t) in self.text.iter().enumerate() {
            if let Some(s) = t.size_fx {
                ini.set("text", &format!("{}.size", TS_NAMES[i]), &size_text(s));
            }
            if let Some(l) = t.line {
                ini.set("text", &format!("{}.line", TS_NAMES[i]), &format!("{}", l));
            }
            if let Some(w) = t.weight {
                ini.set("text", &format!("{}.weight", TS_NAMES[i]), &weight_text(w));
            }
        }
        ini
    }

    /// whether the theme changes text (a family or a text style)
    pub(crate) fn has_text(&self) -> bool {
        !self.family.is_empty() || self.text.iter().any(|t| *t != TextStyle::default())
    }
}

/// a font file rather than an installed family: a scheme, a folder or a font extension
pub(crate) fn is_file(spec: &str) -> bool {
    let l = spec.to_ascii_lowercase();
    l.contains("://") || l.contains('/') || l.contains('\\') || [".ttf", ".otf", ".ttc", ".otc"].iter().any(|e| l.ends_with(e))
}

fn is_absolute(f: &str) -> bool {
    let b = f.as_bytes();
    f.contains("://") || f.starts_with('/') || f.starts_with('\\') || (b.len() > 2 && b[1] == b':')
}
