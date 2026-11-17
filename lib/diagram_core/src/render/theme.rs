// theme -- what the viewer needs of a DrawingML theme with Visio's extensions (JD4)
//
//   ThemeInfo::parse(bytes)          colours (dk1, lt1, dk2, lt2, accent1-6, hlink, folHlink),
//                                    the variation colours (vt:variationClrScheme, 7 each), the
//                                    background (vt:bkgnd), the major and minor Latin fonts, the
//                                    line widths of the format scheme, and the scheme IDs
//                                    (vt:schemeID schemeEnum) a page's ThemeIndex and
//                                    ColorSchemeIndex cells point at
//   ThemeInfo::quick_color(index, variation)   a QuickStyle colour index -> RGB
//
// Pages do not name their theme by a relationship: Visio keeps all themes at the document and
// a page picks one by the schemeEnum of its ThemeIndex (the theme), ColorSchemeIndex (the
// colours) and FontSchemeIndex (the fonts) cells (JD3, finding).  The viewer draws the cached
// values; a theme is needed only where a value is literally "Themed" (the Theme style and a few
// shapes) and for the theme's background.  A small tolerant scanner reads the part: elements,
// attributes, nothing else; anything it does not understand is skipped.

use alloc::string::String;
use alloc::vec::Vec;

/// the parts of a theme the viewer uses
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ThemeInfo {
    /// a:theme name
    pub name: String,
    /// 0xRRGGBB of dk1, lt1, dk2, lt2, accent1 .. accent6, hlink, folHlink (None: not given)
    pub colors: [Option<u32>; 12],
    /// the variation colour schemes, 7 colours each
    pub variations: Vec<[Option<u32>; 7]>,
    /// vt:bkgnd
    pub background: Option<u32>,
    /// the Latin typefaces of the major (headings) and minor (body) fonts
    pub major_font: String,
    pub minor_font: String,
    /// a:lnStyleLst widths in inches (the line style matrix 1, 2, 3 ...)
    pub line_widths: Vec<f64>,
    /// the schemeEnum of the colour scheme, the font scheme, the format scheme and the theme
    /// as a whole (vt:themeScheme), in that order; every other schemeID found after them
    pub scheme_ids: [Option<u32>; 4],
    pub other_ids: Vec<u32>,
}

/// a QuickStyle colour index for "no theme": dark 1 black, light 1 white, accents Office-like
const NO_THEME: [u32; 12] = [0x000000, 0xFFFFFF, 0x44546A, 0xE7E6E6, 0x5B9BD5, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x4472C4, 0x70AD47, 0x0563C1, 0x954F72];

impl ThemeInfo {
    /// the theme of a theme part (None: not a theme)
    pub fn parse(data: &[u8]) -> Option<ThemeInfo> {
        let text = core::str::from_utf8(data).ok()?;
        let mut t = ThemeInfo::default();
        let mut stack: Vec<String> = Vec::new();
        let mut seen_theme = false;
        let mut var: Option<[Option<u32>; 7]> = None;
        let mut sc = Scanner { s: text, pos: 0 };
        while let Some(tok) = sc.next() {
            match tok {
                Tok::Open { name, attrs, empty } => {
                    let local = local(name);
                    let parent = stack.last().map(|s| local_str(s)).unwrap_or("");
                    match local {
                        "theme" => {
                            seen_theme = true;
                            t.name = attr(attrs, "name").map(unescape).unwrap_or_default();
                        }
                        "variationClrScheme" => var = Some([None; 7]),
                        "srgbClr" | "sysClr" => {
                            let v = if local == "srgbClr" { attr(attrs, "val") } else { attr(attrs, "lastClr") };
                            if let Some(c) = v.and_then(hex) {
                                if let Some(i) = scheme_slot(parent) {
                                    if stack.iter().any(|s| local_str(s) == "clrScheme") && t.colors[i].is_none() {
                                        t.colors[i] = Some(c);
                                    }
                                } else if let Some(i) = parent.strip_prefix("varColor").and_then(|n| n.parse::<usize>().ok()) {
                                    if let (Some(v), true) = (var.as_mut(), (1..=7).contains(&i)) {
                                        v[i - 1] = Some(c);
                                    }
                                } else if parent == "bkgnd" && t.background.is_none() {
                                    t.background = Some(c);
                                }
                            }
                        }
                        "latin" => {
                            let face = attr(attrs, "typeface").map(unescape).unwrap_or_default();
                            match parent {
                                "majorFont" if t.major_font.is_empty() => t.major_font = face,
                                "minorFont" if t.minor_font.is_empty() => t.minor_font = face,
                                _ => {}
                            }
                        }
                        "ln" if parent == "lnStyleLst" => {
                            let w = attr(attrs, "w").and_then(|w| w.parse::<f64>().ok()).unwrap_or(9525.0);
                            t.line_widths.push(w / 914_400.0);
                        }
                        "schemeID" => {
                            if let Some(id) = attr(attrs, "schemeEnum").and_then(|v| v.parse::<u32>().ok()) {
                                let slot = if stack.iter().any(|s| local_str(s) == "clrScheme") {
                                    Some(0)
                                } else if stack.iter().any(|s| local_str(s) == "fontScheme") {
                                    Some(1)
                                } else if stack.iter().any(|s| local_str(s) == "fmtScheme") {
                                    Some(2)
                                } else if parent == "themeScheme" {
                                    Some(3)
                                } else {
                                    None
                                };
                                match slot {
                                    Some(i) if t.scheme_ids[i].is_none() => t.scheme_ids[i] = Some(id),
                                    _ => t.other_ids.push(id),
                                }
                            }
                        }
                        _ => {}
                    }
                    if !empty {
                        stack.push(String::from(name));
                    }
                }
                Tok::Close(name) => {
                    if local(name) == "variationClrScheme" {
                        if let Some(v) = var.take() {
                            t.variations.push(v);
                        }
                    }
                    // pop to the matching element (tolerates a missing end tag)
                    if let Some(i) = stack.iter().rposition(|s| s == name) {
                        stack.truncate(i);
                    }
                }
            }
            if stack.len() > 256 {
                return None;
            }
        }
        seen_theme.then_some(t)
    }

    /// whether the theme's own scheme (vt:themeScheme), or else any of its schemes, has this ID
    pub fn has_scheme(&self, id: u32) -> bool {
        self.scheme_ids.contains(&Some(id)) || self.other_ids.contains(&id)
    }

    /// a scheme colour 0 .. 11 (dk1, lt1, dk2, lt2, accent1-6, hlink, folHlink)
    pub fn color(&self, i: usize) -> u32 {
        self.colors.get(i).copied().flatten().unwrap_or(NO_THEME[i.min(11)])
    }

    /// a QuickStyle colour index (QuickStyleLineColor, ...FillColor, ...FontColor): 0 dark 1,
    /// 1 light 1, 2 to 7 accent 1 to 6, 8 dark 2, 9 light 2, 100 to 106 the variation colours
    /// 1 to 7 of `variation`, 200 the background
    pub fn quick_color(&self, index: u32, variation: usize) -> Option<u32> {
        Some(match index {
            0 => self.color(0),
            1 => self.color(1),
            2..=7 => self.color(index as usize + 2),
            8 => self.color(2),
            9 => self.color(3),
            100..=106 => {
                let v = self.variations.get(variation).or(self.variations.first())?;
                v[(index - 100) as usize].unwrap_or(self.color(4))
            }
            200 => self.background.unwrap_or(0xFFFFFF),
            _ => return None,
        })
    }
}

/// QuickStyle colours without a theme
pub fn no_theme_color(index: u32) -> u32 {
    match index {
        0 => NO_THEME[0],
        1 => NO_THEME[1],
        2..=7 => NO_THEME[index as usize + 2],
        200 => 0xFFFFFF,
        _ => NO_THEME[4],
    }
}

fn scheme_slot(name: &str) -> Option<usize> {
    Some(match name {
        "dk1" => 0,
        "lt1" => 1,
        "dk2" => 2,
        "lt2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        "hlink" => 10,
        "folHlink" => 11,
        _ => return None,
    })
}

fn hex(v: &str) -> Option<u32> {
    let v = v.trim().trim_start_matches('#');
    (v.len() == 6).then(|| u32::from_str_radix(v, 16).ok()).flatten()
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn local_str(s: &str) -> &str {
    local(s)
}

fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let end = after[1..].find(q)?;
        let value = &after[1..1 + end];
        if key == name || local(key) == name {
            return Some(value);
        }
        rest = &after[end + 2..];
    }
    None
}

/// the five predefined entities and numeric character references
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail.find(';').filter(|&k| k <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

enum Tok<'a> {
    Open { name: &'a str, attrs: &'a str, empty: bool },
    Close(&'a str),
}

struct Scanner<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            let rest = &self.s[self.pos..];
            let lt = rest.find('<')?;
            let start = self.pos + lt;
            let tail = &self.s[start..];
            if tail.starts_with("<!--") {
                self.pos = start + tail.find("-->").map_or(tail.len(), |e| e + 3);
                continue;
            }
            if tail.starts_with("<![CDATA[") {
                self.pos = start + tail.find("]]>").map_or(tail.len(), |e| e + 3);
                continue;
            }
            if tail.starts_with("<?") || tail.starts_with("<!") {
                self.pos = start + tail.find('>').map_or(tail.len(), |e| e + 1);
                continue;
            }
            // the end of the tag, quotes respected
            let bytes = tail.as_bytes();
            let mut i = 1;
            let mut quote = 0u8;
            while i < bytes.len() {
                let b = bytes[i];
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    break;
                }
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }
            self.pos = start + i + 1;
            let inner = &tail[1..i];
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Tok::Close(name.trim()));
            }
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(x) => (x, true),
                None => (inner, false),
            };
            let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            return Some(Tok::Open { name: &inner[..name_end], attrs: &inner[name_end..], empty });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_of_our_own_parses() {
        let xml = r#"<?xml version="1.0"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Probe &amp; Test">
 <a:themeElements>
  <a:clrScheme name="P"><a:dk1><a:sysClr val="windowText" lastClr="101010"/></a:dk1><a:lt1><a:srgbClr val="FEFEFE"/></a:lt1>
   <a:accent1><a:srgbClr val="112233"/></a:accent1>
   <a:extLst><a:ext uri="x"><vt:schemeID xmlns:vt="v" schemeEnum="41" schemeGUID="{0}"/></a:ext>
   <a:ext uri="y"><vt:bkgnd xmlns:vt="v"><a:srgbClr val="F0F0F0"/></vt:bkgnd></a:ext>
   <a:ext uri="z"><vt:variationClrSchemeLst xmlns:vt="v"><vt:variationClrScheme><vt:varColor1><a:srgbClr val="AA0000"/></vt:varColor1></vt:variationClrScheme>
   <vt:variationClrScheme><vt:varColor2><a:srgbClr val="00BB00"/></vt:varColor2></vt:variationClrScheme></vt:variationClrSchemeLst></a:ext></a:extLst>
  </a:clrScheme>
  <a:fontScheme name="F"><a:majorFont><a:latin typeface="Head"/></a:majorFont><a:minorFont><a:latin typeface="Body"/></a:minorFont></a:fontScheme>
  <a:fmtScheme name="M"><a:lnStyleLst><a:ln w="9525"/><a:ln w="19050"><a:solidFill/></a:ln></a:lnStyleLst></a:fmtScheme>
 </a:themeElements>
 <a:extLst><a:ext uri="t"><vt:themeScheme xmlns:vt="v"><vt:schemeID schemeEnum="34"/></vt:themeScheme></a:ext></a:extLst>
</a:theme>"#;
        let t = ThemeInfo::parse(xml.as_bytes()).unwrap();
        assert_eq!(t.name, "Probe & Test");
        assert_eq!(t.colors[0], Some(0x101010));
        assert_eq!(t.colors[1], Some(0xFEFEFE));
        assert_eq!(t.colors[4], Some(0x112233));
        assert_eq!(t.background, Some(0xF0F0F0));
        assert_eq!(t.variations.len(), 2);
        assert_eq!(t.quick_color(100, 0), Some(0xAA0000));
        assert_eq!(t.quick_color(101, 1), Some(0x00BB00));
        assert_eq!(t.quick_color(2, 0), Some(0x112233));
        assert_eq!((t.major_font.as_str(), t.minor_font.as_str()), ("Head", "Body"));
        assert_eq!(t.line_widths.len(), 2);
        assert!((t.line_widths[1] - 19050.0 / 914_400.0).abs() < 1e-12);
        assert_eq!(t.scheme_ids, [Some(41), None, None, Some(34)]);
        assert!(t.has_scheme(34) && t.has_scheme(41) && !t.has_scheme(40));
        assert!(ThemeInfo::parse(b"<x/>").is_none());
    }
}
