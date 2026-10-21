// vicon -- vector icons from SVG path data, for styles (Style::paint_icon) and app icon hooks
//
//   paint_svg_icon(g, art, x, y, size_fx, color, stroke) -> drawn
//        art: SVG path data on a 24 x 24 grid; several layers are separated by '|', a layer
//        that starts with 'F' is filled (non-zero rule), the others are stroked with round
//        caps and joins `stroke` grid units wide (1.75 is a good outline weight); 'E' fills
//        with the even-odd rule (cut-outs: a mark inside a filled disc).  The icon's
//        em box has its top-left corner at (x, y) target pixels and is size_fx (24.8) pixels
//        square.  A layer that starts with 'A' is drawn in the accent colour (second colour).
//   paint_svg_icon2(.., color, accent, stroke)   the same with an accent colour for 'A' layers
//
// Icons are geometry, not font glyphs: crisp at every size and dpi, and free of the icon
// font's licence.  Parsing is cheap (a few dozen commands per icon).
//
// The layer prefix (layer_head, shared with symbol.rs): 'A', 'F', 'E', '#n' (a palette role
// of a multi-colour symbol, ignored here) and 'S<w>' (this layer's stroke width in grid
// units instead of `stroke`) in any order before the first path command.

use gfx::pathdata::PData;
use gfx::xform::Matrix;

use crate::cmd::Gfx;

/// the prefix of an icon or symbol layer
#[derive(Clone, Copy, Default)]
pub(crate) struct LayerHead {
    /// 'A': the accent colour
    pub accent: bool,
    /// 0 stroked, 1 filled with the non-zero rule ('F'), 2 with the even-odd rule ('E')
    pub fill: u8,
    /// '#n': palette role 0..9
    pub role: Option<u8>,
    /// 'S<w>': stroke width in grid units
    pub stroke: Option<f32>,
}

/// split a layer into its prefix and its path data: 'A', 'F', 'E', '#n' and 'S<w>' in any
/// order before the first path command; blanks between them are skipped
pub(crate) fn layer_head(layer: &str) -> (LayerHead, &str) {
    let b = layer.as_bytes();
    let mut h = LayerHead::default();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'A' => {
                h.accent = true;
                i += 1;
            }
            b'F' => {
                h.fill = 1;
                i += 1;
            }
            b'E' => {
                h.fill = 2;
                i += 1;
            }
            b'#' if i + 1 < b.len() && b[i + 1].is_ascii_digit() => {
                h.role = Some(b[i + 1] - b'0');
                i += 2;
            }
            b'S' => {
                let s = i + 1;
                let mut e = s;
                while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'.') {
                    e += 1;
                }
                match layer[s..e].parse::<f32>() {
                    Ok(w) if e > s && w.is_finite() && w >= 0.0 => h.stroke = Some(w),
                    _ => break, // not a width: the path parser reports it
                }
                i = e;
            }
            _ => break,
        }
    }
    (h, &layer[i..])
}

/// paint an icon from 24-grid SVG path data (see the module comment)
pub fn paint_svg_icon(g: &mut Gfx, art: &str, x: i32, y: i32, size_fx: i32, color: u32, stroke: f32) -> bool {
    paint_svg_icon2(g, art, x, y, size_fx, color, color, stroke)
}

/// paint_svg_icon with a second (accent) colour for the layers that start with 'A'
#[allow(clippy::too_many_arguments)]
pub fn paint_svg_icon2(g: &mut Gfx, art: &str, x: i32, y: i32, size_fx: i32, color: u32, accent: u32, stroke: f32) -> bool {
    if size_fx <= 0 {
        return true;
    }
    let s = size_fx as f64 / 256.0 / 24.0;
    let m = Matrix { a: s, b: 0.0, c: x as f64, d: 0.0, e: s, f: y as f64 };
    let mut pd = PData::new();
    for layer in art.split('|') {
        let (h, d) = layer_head(layer.trim());
        pd.clear();
        let (err, _) = pd.parse(d.as_bytes());
        if err != 0 {
            continue;
        }
        let path = &mut g.text.path;
        path.reset();
        path.set_origin(0, 0);
        if pd.emit(path, Some(&m)) != 0 {
            continue;
        }
        g.dc.set_color(if h.accent { accent } else { color });
        if h.fill != 0 {
            path.fill(&mut g.text.ras, g.dc, g.sf, if h.fill == 2 { gfx::RULE_EVENODD } else { gfx::RULE_NONZERO });
        } else {
            // at least ~1 px wide so small icons stay visible
            let w = ((h.stroke.unwrap_or(stroke) as f64 * s).max(1.0) * 256.0) as i32;
            path.stroke(&mut g.text.ras, g.dc, g.sf, w, gfx::JOIN_ROUND | gfx::CAP_ROUND << 4);
        }
    }
    true
}
