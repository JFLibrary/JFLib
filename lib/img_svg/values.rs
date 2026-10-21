// values -- the attribute and property values of SVG: numbers, lengths, colours, paints,
// transforms, viewBox, preserveAspectRatio, point lists, dash arrays
//
// Every parser returns None for a value it cannot read; the caller then ignores the
// declaration (the property keeps its inherited / initial value), as CSS does.  Numbers are
// finite and at most NUM_MAX in magnitude.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::xform::{sincos, Matrix};
use img_core::xform::mul;

pub(crate) const NUM_MAX: f64 = 1.0e9;
const DEG: f64 = 0.017453292519943295;

#[inline]
pub(crate) fn blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 12)
}

/// a number at s[*i..] (blanks before it skipped): [+-] digits [. digits] [e [+-] digits]
pub(crate) fn number(s: &[u8], i: &mut usize) -> Option<f64> {
    let mut j = *i;
    while j < s.len() && blank(s[j]) {
        j += 1;
    }
    let start = j;
    if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
        j += 1;
    }
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < s.len() && s[j] == b'.' {
        let k = j + 1;
        let mut m = k;
        while m < s.len() && s[m].is_ascii_digit() {
            m += 1;
        }
        if m > k || digits > 0 {
            digits += m - k;
            j = m;
        }
    }
    if digits == 0 {
        return None;
    }
    // an exponent only when digits follow ("1em" is a number and a unit)
    if j < s.len() && s[j] | 32 == b'e' {
        let mut k = j + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        let k0 = k;
        while k < s.len() && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > k0 && k - k0 <= 4 {
            j = k;
        }
    }
    let t = core::str::from_utf8(&s[start..j]).ok()?;
    let v: f64 = t.parse().ok()?;
    if !(v.abs() <= NUM_MAX) {
        return None;
    }
    *i = j;
    Some(v)
}

/// blanks and at most one comma
pub(crate) fn sep(s: &[u8], i: &mut usize) {
    let mut comma = false;
    while *i < s.len() {
        let c = s[*i];
        if c == b',' && !comma {
            comma = true;
        } else if !blank(c) {
            return;
        }
        *i += 1;
    }
}

/// a whole value that is one number (blanks around it)
pub(crate) fn num_only(v: &str) -> Option<f64> {
    let s = v.as_bytes();
    let mut i = 0;
    let n = number(s, &mut i)?;
    if s[i..].iter().all(|&c| blank(c)) { Some(n) } else { None }
}

/// every number of a list (blanks / commas between); None when anything else is in it
pub(crate) fn numbers(v: &str) -> Option<Vec<f64>> {
    let s = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        sep(s, &mut i);
        if i >= s.len() {
            return Some(out);
        }
        out.push(number(s, &mut i)?);
    }
}

// ------------------------------------------------------------------ lengths

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Unit {
    Px,
    Em,
    Ex,
    Pct,
}

/// a length: value and unit (absolute units are converted to px at parse time)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Len {
    pub v: f64,
    pub u: Unit,
}

impl Len {
    pub const ZERO: Len = Len { v: 0.0, u: Unit::Px };
    pub fn px(v: f64) -> Len {
        Len { v, u: Unit::Px }
    }
    /// in user units: em / ex relative to font size fs, % of `of`
    pub fn get(self, fs: f64, of: f64) -> f64 {
        match self.u {
            Unit::Px => self.v,
            Unit::Em => self.v * fs,
            Unit::Ex => self.v * fs * 0.5,
            Unit::Pct => self.v * of * 0.01,
        }
    }
}

// a unit after a number
fn unit(s: &[u8], i: &mut usize) -> Option<(f64, Unit)> {
    let rest = &s[*i..];
    let take = |n: usize, i: &mut usize| *i += n;
    let lower = |k: usize| rest.get(..k).map(|r| r.to_ascii_lowercase());
    if rest.first() == Some(&b'%') {
        take(1, i);
        return Some((1.0, Unit::Pct));
    }
    if let Some(u) = lower(2) {
        let f = match u.as_slice() {
            b"px" => Some((1.0, Unit::Px)),
            b"pt" => Some((96.0 / 72.0, Unit::Px)),
            b"pc" => Some((16.0, Unit::Px)),
            b"mm" => Some((96.0 / 25.4, Unit::Px)),
            b"cm" => Some((96.0 / 2.54, Unit::Px)),
            b"in" => Some((96.0, Unit::Px)),
            b"em" => Some((1.0, Unit::Em)),
            b"ex" => Some((1.0, Unit::Ex)),
            _ => None,
        };
        if f.is_some() {
            take(2, i);
            return f;
        }
    }
    Some((1.0, Unit::Px))
}

/// a length at s[*i..]
pub(crate) fn length_at(s: &[u8], i: &mut usize) -> Option<Len> {
    let v = number(s, i)?;
    let (f, u) = unit(s, i)?;
    Some(Len { v: v * f, u })
}

/// a value that is one length
pub(crate) fn length(v: &str) -> Option<Len> {
    let s = v.as_bytes();
    let mut i = 0;
    let l = length_at(s, &mut i)?;
    if s[i..].iter().all(|&c| blank(c)) { Some(l) } else { None }
}

/// a list of lengths (stroke-dasharray): None when one is invalid
pub(crate) fn lengths(v: &str) -> Option<Vec<Len>> {
    let s = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        sep(s, &mut i);
        if i >= s.len() {
            return Some(out);
        }
        out.push(length_at(s, &mut i)?);
        if out.len() > 256 {
            return None;
        }
    }
}

// ------------------------------------------------------------------ colours and paints

/// a colour: straight 0xAARRGGBB, or currentColor
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Color {
    Argb(u32),
    Current,
}

/// a fill / stroke paint
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Paint {
    None,
    Color(Color),
    /// a paint server (gradient) by id, with the fallback for a missing / unusable one
    Url(String, Option<Color>),
}

fn hexval(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

fn clamp255(v: f64) -> u32 {
    if !(v > 0.0) {
        0
    } else if v >= 255.0 {
        255
    } else {
        (v + 0.5) as u32
    }
}

// the arguments of rgb() / hsl(): numbers or percentages, separated by commas or blanks,
// an optional "/ alpha"
fn func_args(s: &str) -> Option<Vec<(f64, bool)>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && (blank(b[i]) || b[i] == b',' || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let v = number(b, &mut i)?;
        let pct = b.get(i) == Some(&b'%');
        if pct {
            i += 1;
        } else if b.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
            // "deg" after a hue
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
        }
        out.push((v, pct));
        if out.len() > 4 {
            return None;
        }
    }
    Some(out)
}

/// floor without libm (values beyond 2^52 are integers already)
pub(crate) fn floor(x: f64) -> f64 {
    if !(x.abs() < 4.0e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

fn hsl_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let h = (h - 360.0 * floor(h / 360.0)) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hm = h - 2.0 * floor(h * 0.5);
    let x = c * (1.0 - (hm - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c * 0.5;
    ((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
}

/// a colour value: #rgb, #rgba, #rrggbb, #rrggbbaa, rgb()/rgba(), hsl()/hsla(), a CSS colour
/// name, transparent, currentColor (case-insensitive)
pub(crate) fn color(v: &str) -> Option<Color> {
    let t = v.trim();
    let b = t.as_bytes();
    if b.first() == Some(&b'#') {
        let h = &b[1..];
        let d: Option<Vec<u32>> = h.iter().map(|&c| hexval(c)).collect();
        let d = d?;
        let argb = match d.len() {
            3 => 0xFF00_0000 | (d[0] * 17) << 16 | (d[1] * 17) << 8 | d[2] * 17,
            4 => (d[3] * 17) << 24 | (d[0] * 17) << 16 | (d[1] * 17) << 8 | d[2] * 17,
            6 => 0xFF00_0000 | (d[0] << 4 | d[1]) << 16 | (d[2] << 4 | d[3]) << 8 | (d[4] << 4 | d[5]),
            8 => (d[6] << 4 | d[7]) << 24 | (d[0] << 4 | d[1]) << 16 | (d[2] << 4 | d[3]) << 8 | (d[4] << 4 | d[5]),
            _ => return None,
        };
        return Some(Color::Argb(argb));
    }
    let lower = t.to_ascii_lowercase();
    if lower == "currentcolor" {
        return Some(Color::Current);
    }
    if lower == "transparent" {
        return Some(Color::Argb(0));
    }
    if let Some(open) = lower.find('(') {
        let fname = lower[..open].trim();
        let close = lower.rfind(')')?;
        if close < open || !lower[close + 1..].trim().is_empty() {
            return None;
        }
        let args = func_args(&lower[open + 1..close])?;
        if args.len() < 3 {
            return None;
        }
        let alpha = match args.get(3) {
            Some(&(a, pct)) => clamp255(if pct { a * 2.55 } else { a * 255.0 }),
            None => 255,
        };
        let (r, g, bl) = match fname {
            "rgb" | "rgba" => {
                let ch = |(v, pct): (f64, bool)| if pct { v * 2.55 } else { v };
                (ch(args[0]), ch(args[1]), ch(args[2]))
            }
            "hsl" | "hsla" => {
                let s = (args[1].0 * 0.01).clamp(0.0, 1.0);
                let l = (args[2].0 * 0.01).clamp(0.0, 1.0);
                hsl_rgb(args[0].0, s, l)
            }
            _ => return None,
        };
        return Some(Color::Argb(alpha << 24 | clamp255(r) << 16 | clamp255(g) << 8 | clamp255(bl)));
    }
    named(&lower).map(|rgb| Color::Argb(0xFF00_0000 | rgb))
}

/// a paint: none, a colour, url(#id) [fallback]
pub(crate) fn paint(v: &str) -> Option<Paint> {
    let t = v.trim();
    if t.eq_ignore_ascii_case("none") {
        return Some(Paint::None);
    }
    if t.as_bytes().len() >= 4 && t.as_bytes()[..4].eq_ignore_ascii_case(b"url(") {
        let close = t.find(')')?;
        let inner = t[4..close].trim().trim_matches(|c| c == '"' || c == '\'');
        let id = inner.strip_prefix('#')?;
        let rest = t[close + 1..].trim();
        let fb = if rest.is_empty() {
            None
        } else if rest.eq_ignore_ascii_case("none") {
            Some(Color::Argb(0))
        } else {
            Some(color(rest)?)
        };
        return Some(Paint::Url(String::from(id), fb));
    }
    color(t).map(Paint::Color)
}

/// the id of url(#id) (clip-path, href fragments "#id")
pub(crate) fn url_id(v: &str) -> Option<&str> {
    let t = v.trim();
    if t.as_bytes().len() >= 4 && t.as_bytes()[..4].eq_ignore_ascii_case(b"url(") {
        let close = t.find(')')?;
        let inner = t[4..close].trim().trim_matches(|c| c == '"' || c == '\'');
        return inner.strip_prefix('#');
    }
    None
}

/// an opacity: a number (clamped to 0..1) or a percentage
pub(crate) fn opacity(v: &str) -> Option<f64> {
    let l = length(v)?;
    let o = match l.u {
        Unit::Pct => l.v * 0.01,
        Unit::Px => l.v,
        _ => return None,
    };
    Some(o.clamp(0.0, 1.0))
}

// ------------------------------------------------------------------ transforms

fn tan(a: f64) -> f64 {
    let (s, c) = sincos(a);
    if c.abs() < 1e-12 { 0.0 } else { s / c }
}

/// a transform list: matrix() translate() scale() rotate() skewX() skewY(), applied left to
/// right (the left one outermost); None when malformed
pub(crate) fn transform(v: &str) -> Option<Matrix> {
    let s = v.as_bytes();
    let mut m = Matrix::IDENTITY;
    let mut i = 0;
    loop {
        while i < s.len() && (blank(s[i]) || s[i] == b',') {
            i += 1;
        }
        if i >= s.len() {
            return Some(m);
        }
        let n0 = i;
        while i < s.len() && s[i].is_ascii_alphabetic() {
            i += 1;
        }
        let name = &s[n0..i];
        while i < s.len() && blank(s[i]) {
            i += 1;
        }
        if s.get(i) != Some(&b'(') {
            return None;
        }
        i += 1;
        let mut a = [0f64; 6];
        let mut n = 0;
        loop {
            sep(s, &mut i);
            if s.get(i) == Some(&b')') {
                i += 1;
                break;
            }
            if n >= 6 {
                return None;
            }
            a[n] = number(s, &mut i)?;
            n += 1;
        }
        let t = match (name, n) {
            (b"matrix", 6) => Matrix { a: a[0], b: a[2], c: a[4], d: a[1], e: a[3], f: a[5] },
            (b"translate", 1) => Matrix { c: a[0], ..Matrix::IDENTITY },
            (b"translate", 2) => Matrix { c: a[0], f: a[1], ..Matrix::IDENTITY },
            (b"scale", 1) => Matrix { a: a[0], e: a[0], ..Matrix::IDENTITY },
            (b"scale", 2) => Matrix { a: a[0], e: a[1], ..Matrix::IDENTITY },
            (b"rotate", 1) | (b"rotate", 3) => {
                let (sn, cs) = sincos(a[0] * DEG);
                let r = Matrix { a: cs, b: -sn, c: 0.0, d: sn, e: cs, f: 0.0 };
                if n == 3 {
                    let t1 = Matrix { c: a[1], f: a[2], ..Matrix::IDENTITY };
                    let t2 = Matrix { c: -a[1], f: -a[2], ..Matrix::IDENTITY };
                    mul(&mul(&t1, &r), &t2)
                } else {
                    r
                }
            }
            (b"skewX", 1) => Matrix { b: tan(a[0] * DEG), ..Matrix::IDENTITY },
            (b"skewY", 1) => Matrix { d: tan(a[0] * DEG), ..Matrix::IDENTITY },
            _ => return None,
        };
        m = mul(&m, &t);
    }
}

/// viewBox="x y w h" (w, h > 0)
pub(crate) fn view_box(v: &str) -> Option<[f64; 4]> {
    let n = numbers(v)?;
    if n.len() != 4 || !(n[2] > 0.0) || !(n[3] > 0.0) {
        return None;
    }
    Some([n[0], n[1], n[2], n[3]])
}

/// preserveAspectRatio: align (None = "none") as (x, y) 0 min / 1 mid / 2 max, slice
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Par {
    pub align: Option<(u8, u8)>,
    pub slice: bool,
}

impl Default for Par {
    fn default() -> Par {
        Par { align: Some((1, 1)), slice: false }
    }
}

pub(crate) fn par(v: &str) -> Par {
    let mut p = Par::default();
    let mut words = v.split_ascii_whitespace();
    let mut w = words.next();
    if w == Some("defer") {
        w = words.next();
    }
    let Some(a) = w else { return p };
    let axis = |s: &[u8]| match s {
        b"Min" => Some(0u8),
        b"Mid" => Some(1),
        b"Max" => Some(2),
        _ => None,
    };
    let b = a.as_bytes();
    if a == "none" {
        p.align = None;
    } else if b.len() == 8 && b[0] == b'x' && b[4] == b'Y' {
        match (axis(&b[1..4]), axis(&b[5..8])) {
            (Some(x), Some(y)) => p.align = Some((x, y)),
            _ => return Par::default(),
        }
    } else {
        return Par::default();
    }
    p.slice = words.next() == Some("slice");
    p
}

/// the matrix that maps viewBox vb into the viewport (x, y, w, h) by par
pub(crate) fn view_matrix(vb: [f64; 4], vp: [f64; 4], p: Par) -> Matrix {
    let mut sx = vp[2] / vb[2];
    let mut sy = vp[3] / vb[3];
    let (mut tx, mut ty) = (vp[0] - vb[0] * sx, vp[1] - vb[1] * sy);
    if let Some((ax, ay)) = p.align {
        let s = if p.slice { sx.max(sy) } else { sx.min(sy) };
        sx = s;
        sy = s;
        let ex = vp[2] - vb[2] * s;
        let ey = vp[3] - vb[3] * s;
        tx = vp[0] - vb[0] * s + ex * ax as f64 * 0.5;
        ty = vp[1] - vb[1] * s + ey * ay as f64 * 0.5;
    }
    Matrix { a: sx, b: 0.0, c: tx, d: 0.0, e: sy, f: ty }
}

// ------------------------------------------------------------------ named colours

static NAMED: [(&str, u32); 148] = [
    ("aliceblue", 0xF0F8FF), ("antiquewhite", 0xFAEBD7), ("aqua", 0x00FFFF), ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF), ("beige", 0xF5F5DC), ("bisque", 0xFFE4C4), ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD), ("blue", 0x0000FF), ("blueviolet", 0x8A2BE2), ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887), ("cadetblue", 0x5F9EA0), ("chartreuse", 0x7FFF00), ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50), ("cornflowerblue", 0x6495ED), ("cornsilk", 0xFFF8DC), ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF), ("darkblue", 0x00008B), ("darkcyan", 0x008B8B), ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9), ("darkgreen", 0x006400), ("darkgrey", 0xA9A9A9), ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B), ("darkolivegreen", 0x556B2F), ("darkorange", 0xFF8C00), ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000), ("darksalmon", 0xE9967A), ("darkseagreen", 0x8FBC8F), ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F), ("darkslategrey", 0x2F4F4F), ("darkturquoise", 0x00CED1), ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493), ("deepskyblue", 0x00BFFF), ("dimgray", 0x696969), ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF), ("firebrick", 0xB22222), ("floralwhite", 0xFFFAF0), ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF), ("gainsboro", 0xDCDCDC), ("ghostwhite", 0xF8F8FF), ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520), ("gray", 0x808080), ("green", 0x008000), ("greenyellow", 0xADFF2F),
    ("grey", 0x808080), ("honeydew", 0xF0FFF0), ("hotpink", 0xFF69B4), ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082), ("ivory", 0xFFFFF0), ("khaki", 0xF0E68C), ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5), ("lawngreen", 0x7CFC00), ("lemonchiffon", 0xFFFACD), ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080), ("lightcyan", 0xE0FFFF), ("lightgoldenrodyellow", 0xFAFAD2), ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90), ("lightgrey", 0xD3D3D3), ("lightpink", 0xFFB6C1), ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA), ("lightskyblue", 0x87CEFA), ("lightslategray", 0x778899), ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE), ("lightyellow", 0xFFFFE0), ("lime", 0x00FF00), ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6), ("magenta", 0xFF00FF), ("maroon", 0x800000), ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD), ("mediumorchid", 0xBA55D3), ("mediumpurple", 0x9370DB), ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE), ("mediumspringgreen", 0x00FA9A), ("mediumturquoise", 0x48D1CC), ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970), ("mintcream", 0xF5FFFA), ("mistyrose", 0xFFE4E1), ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD), ("navy", 0x000080), ("oldlace", 0xFDF5E6), ("olive", 0x808000),
    ("olivedrab", 0x6B8E23), ("orange", 0xFFA500), ("orangered", 0xFF4500), ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA), ("palegreen", 0x98FB98), ("paleturquoise", 0xAFEEEE), ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5), ("peachpuff", 0xFFDAB9), ("peru", 0xCD853F), ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD), ("powderblue", 0xB0E0E6), ("purple", 0x800080), ("rebeccapurple", 0x663399),
    ("red", 0xFF0000), ("rosybrown", 0xBC8F8F), ("royalblue", 0x4169E1), ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072), ("sandybrown", 0xF4A460), ("seagreen", 0x2E8B57), ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D), ("silver", 0xC0C0C0), ("skyblue", 0x87CEEB), ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xFFFAFA), ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4), ("tan", 0xD2B48C), ("teal", 0x008080), ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347), ("turquoise", 0x40E0D0), ("violet", 0xEE82EE), ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF), ("whitesmoke", 0xF5F5F5), ("yellow", 0xFFFF00), ("yellowgreen", 0x9ACD32),
];

fn named(lower: &str) -> Option<u32> {
    NAMED.iter().find(|(n, _)| *n == lower).map(|e| e.1)
}
