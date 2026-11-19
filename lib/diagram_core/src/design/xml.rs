// xml -- a design as a theme part (ECMA-376 Part 1, 20.1.6, with Visio's extensions)
//
// Element order: as ECMA-376 orders the DrawingML elements, and as Visio writes its
// extensions -- clrScheme: schemeID, bkgnd, variationClrSchemeLst; fontScheme: schemeID;
// themeElements: fmtConnectorScheme, themeScheme, fmtSchemeEx, fmtConnectorSchemeEx,
// fillStyles, lineStyles, fontStylesGroup, variationStyleSchemeLst.  Each extension element
// declares the vt namespace itself, as Visio does.  Visio's line style lists have one entry
// more than the DrawingML lists they extend: the first stands for matrix 0 (the root style's
// values: no line ends, solid, no rounding), matrix i reads entry i.

use alloc::format;
use alloc::string::String;

use super::{uri, Cap, Clr, Design, Effect, Line, Mod, Scheme, NS_VT};

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const ZERO_GUID: &str = "{00000000-0000-0000-0000-000000000000}";

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

fn color(o: &mut String, c: &Clr) {
    let (open, mods) = match c {
        Clr::Rgb(v, m) => (format!("a:srgbClr val=\"{:06X}\"", v & 0xFF_FFFF), m),
        Clr::Scheme(s, m) => (format!("a:schemeClr val=\"{}\"", s.name()), m),
    };
    let tag = if matches!(c, Clr::Rgb(..)) { "a:srgbClr" } else { "a:schemeClr" };
    if mods.is_empty() {
        o.push_str(&format!("<{open}/>"));
        return;
    }
    o.push_str(&format!("<{open}>"));
    for m in mods {
        let (n, v) = match *m {
            Mod::LumMod(v) => ("lumMod", v),
            Mod::LumOff(v) => ("lumOff", v),
            Mod::SatMod(v) => ("satMod", v),
            Mod::Alpha(v) => ("alpha", v),
        };
        o.push_str(&format!("<a:{n} val=\"{v}\"/>"));
    }
    o.push_str(&format!("</{tag}>"));
}

fn solid(o: &mut String, c: &Clr) {
    o.push_str("<a:solidFill>");
    color(o, c);
    o.push_str("</a:solidFill>");
}

fn line(o: &mut String, l: &Line) {
    let cap = match l.cap {
        Cap::Round => "rnd",
        Cap::Square => "sq",
        Cap::Flat => "flat",
    };
    o.push_str(&format!("<a:ln w=\"{}\" cap=\"{cap}\" cmpd=\"sng\" algn=\"ctr\">", l.width));
    solid(o, &l.color);
    let dash = if l.pattern >= 2 { "dash" } else { "solid" };
    o.push_str(&format!("<a:prstDash val=\"{dash}\"/><a:round/></a:ln>"));
}

fn effect(o: &mut String, e: &Effect) {
    match &e.shadow {
        None => o.push_str("<a:effectStyle><a:effectLst/></a:effectStyle>"),
        Some(s) => {
            o.push_str(&format!(
                "<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"{}\" dist=\"{}\" dir=\"{}\" algn=\"tl\" rotWithShape=\"0\">",
                s.blur, s.dist, s.dir
            ));
            color(o, &s.color);
            o.push_str("</a:outerShdw></a:effectLst></a:effectStyle>");
        }
    }
}

// the DrawingML lists of a scheme (fmtScheme or vt:fmtConnectorScheme)
fn lists(o: &mut String, s: &Scheme) {
    o.push_str("<a:fillStyleLst>");
    for f in &s.fills {
        solid(o, &f.color);
    }
    o.push_str("</a:fillStyleLst><a:lnStyleLst>");
    for l in &s.lines {
        line(o, l);
    }
    o.push_str("</a:lnStyleLst><a:effectStyleLst>");
    for e in &s.effects {
        effect(o, e);
    }
    o.push_str("</a:effectStyleLst><a:bgFillStyleLst>");
    for f in &s.backgrounds {
        solid(o, &f.color);
    }
    o.push_str("</a:bgFillStyleLst>");
}

fn ext_open(o: &mut String, uri: &str, el: &str, attrs: &str) {
    o.push_str(&format!("<a:ext uri=\"{uri}\"><vt:{el} xmlns:vt=\"{NS_VT}\"{attrs}>"));
}

fn scheme_id(o: &mut String, uri: &str, id: u32) {
    o.push_str(&format!("<a:ext uri=\"{uri}\"><vt:schemeID xmlns:vt=\"{NS_VT}\" schemeEnum=\"{id}\" schemeGUID=\"{ZERO_GUID}\"/></a:ext>"));
}

fn wrapped_id(o: &mut String, uri: &str, el: &str, id: u32) {
    ext_open(o, uri, el, "");
    o.push_str(&format!("<vt:schemeID schemeEnum=\"{id}\" schemeGUID=\"{ZERO_GUID}\"/></vt:{el}></a:ext>"));
}

fn line_ex(o: &mut String, lines: &[Line]) {
    // the entry of matrix 0 first, then one a style
    let root = Line::solid(75, super::Clr::ph());
    for l in core::iter::once(&root).chain(lines) {
        o.push_str(&format!(
            "<vt:lineStyle><vt:lineEx rndg=\"{}\" start=\"{}\" startSize=\"{}\" end=\"{}\" endSize=\"{}\" pattern=\"{}\"/></vt:lineStyle>",
            l.rounding, l.begin, l.begin_size, l.end, l.end_size, l.pattern
        ));
    }
}

fn fonts(o: &mut String, el: &str, s: &Scheme) {
    o.push_str(&format!("<vt:{el}>"));
    for f in &s.fonts {
        o.push_str(&format!("<vt:fontProps style=\"{}\"><vt:color>", f.style));
        color(o, &f.color);
        o.push_str("</vt:color></vt:fontProps>");
    }
    o.push_str(&format!("</vt:{el}>"));
}

const SCHEME: [&str; 12] = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];

/// the theme part of a design
pub(super) fn write(d: &Design) -> String {
    let name = esc(&d.name);
    let mut o = String::with_capacity(16 * 1024);
    o.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n");
    o.push_str(&format!("<a:theme xmlns:a=\"{NS_A}\" name=\"{name}\"><a:themeElements>"));
    // colours
    o.push_str(&format!("<a:clrScheme name=\"{name}\">"));
    for (i, el) in SCHEME.iter().enumerate() {
        o.push_str(&format!("<a:{el}><a:srgbClr val=\"{:06X}\"/></a:{el}>", d.colors[i] & 0xFF_FFFF));
    }
    o.push_str("<a:extLst>");
    scheme_id(&mut o, uri::COLOR_ID, d.id);
    ext_open(&mut o, uri::BACKGROUND, "bkgnd", "");
    color(&mut o, &Clr::rgb(d.background));
    o.push_str("</vt:bkgnd></a:ext>");
    ext_open(&mut o, uri::VARIANT_COLORS, "variationClrSchemeLst", "");
    for v in &d.variants {
        let mono = if v.monotone { " monotone=\"1\"" } else { "" };
        o.push_str(&format!("<vt:variationClrScheme{mono}>"));
        for (k, c) in v.colors.iter().enumerate() {
            o.push_str(&format!("<vt:varColor{n}><a:srgbClr val=\"{c:06X}\"/></vt:varColor{n}>", n = k + 1, c = c & 0xFF_FFFF));
        }
        o.push_str("</vt:variationClrScheme>");
    }
    o.push_str("</vt:variationClrSchemeLst></a:ext></a:extLst></a:clrScheme>");
    // fonts
    o.push_str(&format!("<a:fontScheme name=\"{name}\">"));
    for (el, face) in [("majorFont", &d.major_font), ("minorFont", &d.minor_font)] {
        o.push_str(&format!("<a:{el}><a:latin typeface=\"{}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{el}>", esc(face)));
    }
    o.push_str("<a:extLst>");
    scheme_id(&mut o, uri::FONT_ID, d.id);
    o.push_str("</a:extLst></a:fontScheme>");
    // the format scheme of shapes
    o.push_str(&format!("<a:fmtScheme name=\"{}\">", esc(&d.shapes.name)));
    lists(&mut o, &d.shapes);
    o.push_str("</a:fmtScheme><a:extLst>");
    // Visio's extensions
    ext_open(&mut o, uri::CONNECTOR_SCHEME, "fmtConnectorScheme", &format!(" name=\"{}\"", esc(&d.connectors.name)));
    lists(&mut o, &d.connectors);
    o.push_str("</vt:fmtConnectorScheme></a:ext>");
    wrapped_id(&mut o, uri::THEME_ID, "themeScheme", d.id);
    wrapped_id(&mut o, uri::FORMAT_ID, "fmtSchemeEx", d.id);
    wrapped_id(&mut o, uri::CONNECTOR_ID, "fmtConnectorSchemeEx", d.id);
    ext_open(&mut o, uri::FILL_STYLES, "fillStyles", "");
    for f in &d.shapes.fills {
        o.push_str(&format!("<vt:fillProps pattern=\"{}\"/>", f.pattern));
    }
    o.push_str("</vt:fillStyles></a:ext>");
    ext_open(&mut o, uri::LINE_STYLES, "lineStyles", "");
    o.push_str("<vt:fmtConnectorSchemeLineStyles>");
    line_ex(&mut o, &d.connectors.lines);
    o.push_str("</vt:fmtConnectorSchemeLineStyles><vt:fmtSchemeLineStyles>");
    line_ex(&mut o, &d.shapes.lines);
    o.push_str("</vt:fmtSchemeLineStyles></vt:lineStyles></a:ext>");
    ext_open(&mut o, uri::FONT_STYLES, "fontStylesGroup", "");
    fonts(&mut o, "connectorFontStyles", &d.connectors);
    fonts(&mut o, "fontStyles", &d.shapes);
    o.push_str("</vt:fontStylesGroup></a:ext>");
    ext_open(&mut o, uri::VARIANT_STYLES, "variationStyleSchemeLst", "");
    for v in &d.variants {
        o.push_str(&format!("<vt:variationStyleScheme embellishment=\"{}\">", v.embellishment));
        for s in &v.styles {
            o.push_str(&format!("<vt:varStyle fillIdx=\"{}\" lineIdx=\"{}\" effectIdx=\"{}\" fontIdx=\"{}\"/>", s.fill, s.line, s.effect, s.font));
        }
        o.push_str("</vt:variationStyleScheme>");
    }
    o.push_str("</vt:variationStyleSchemeLst></a:ext>");
    o.push_str("</a:extLst></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>");
    o
}
