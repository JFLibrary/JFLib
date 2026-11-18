// theme -- JFLib's own DrawingML theme "NetLab" for new documents (decision V3)
//
//   theme::netlab() -> the theme part's bytes
//
// Built from the NetLab style's colours (styles/netlab: palette.rs, data.rs, lib.rs) and its
// typeface, after ECMA-376 Part 1, 20.1.6 (theme, clrScheme, fontScheme, fmtScheme): flat as
// the style is -- solid fills, thin solid lines, one soft shadow as the strongest effect.
//
//   dark 1 / light 1      anthracite #1D2228 (text) / white
//   dark 2 / light 2      slate #4D5560 (secondary text) / window grey #ECEEF1
//   accent 1              Signalblau #1F5FA8, the style's accent
//   accent 2-6            the NetLab chart series: ochre #C9792B, teal #3D8C84, brick #B0453F,
//                         violet #7A5C99, olive #7F8C3A
//   hyperlink / followed  Signalblau / violet
//   fonts                 Inter for headings and body
//   lines                 0.75, 1 and 1.5 pt
//
// Visio's own extensions of a theme (variants, connector and line styles) hang under extLst
// entries whose identifiers [MS-VSDX] does not give; the theme leaves them out until they are
// known from a public source.  A document names its theme from the document part.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::blank::FONT;

/// the colour scheme: (element, RGB)
pub(crate) const COLORS: [(&str, &str); 12] = [
    ("dk1", "1D2228"),
    ("lt1", "FFFFFF"),
    ("dk2", "4D5560"),
    ("lt2", "ECEEF1"),
    ("accent1", "1F5FA8"),
    ("accent2", "C9792B"),
    ("accent3", "3D8C84"),
    ("accent4", "B0453F"),
    ("accent5", "7A5C99"),
    ("accent6", "7F8C3A"),
    ("hlink", "1F5FA8"),
    ("folHlink", "7A5C99"),
];

// line widths in EMU (12700 a point)
const LINES: [u32; 3] = [9525, 12700, 19050];

/// the theme part
pub(crate) fn netlab() -> Vec<u8> {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n");
    s.push_str("<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"NetLab\"><a:themeElements>");
    s.push_str("<a:clrScheme name=\"NetLab\">");
    for (e, rgb) in COLORS {
        s.push_str(&format!("<a:{e}><a:srgbClr val=\"{rgb}\"/></a:{e}>"));
    }
    s.push_str("</a:clrScheme><a:fontScheme name=\"NetLab\">");
    for f in ["majorFont", "minorFont"] {
        s.push_str(&format!("<a:{f}><a:latin typeface=\"{FONT}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{f}>"));
    }
    s.push_str("</a:fontScheme><a:fmtScheme name=\"NetLab\"><a:fillStyleLst>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"20000\"/></a:schemeClr></a:solidFill>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"><a:shade val=\"80000\"/></a:schemeClr></a:solidFill>");
    s.push_str("</a:fillStyleLst><a:lnStyleLst>");
    for w in LINES {
        s.push_str(&format!(
            "<a:ln w=\"{w}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>"
        ));
    }
    s.push_str("</a:lnStyleLst><a:effectStyleLst>");
    s.push_str("<a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle>");
    s.push_str("<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"38100\" dist=\"12700\" dir=\"5400000\" algn=\"t\" rotWithShape=\"0\"><a:srgbClr val=\"1D2228\"><a:alpha val=\"24000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>");
    s.push_str("</a:effectStyleLst><a:bgFillStyleLst>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/></a:schemeClr></a:solidFill>");
    s.push_str("<a:solidFill><a:schemeClr val=\"phClr\"><a:shade val=\"90000\"/></a:schemeClr></a:solidFill>");
    s.push_str("</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>");
    s.into_bytes()
}
