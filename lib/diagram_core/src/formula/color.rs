// color -- colours as formulas see them: indices of the document's colour table and RGB, the
// HSL scale of 0..240, and the tint and shade functions (FX-08)
//
//   Color::Index(n) / Color::Rgb(r, g, b)     V writes an index as a number, RGB as "#rrggbb"
//   Color::parse("#4d1e1a")
//   rgb_to_hsl(r, g, b) -> (h, s, l)          0..240 each, Windows' ColorRGBToHLS (HLSMAX 240)
//   hsl_to_rgb(h, s, l)                       its inverse (ColorHLSToRGB), integer rounding
//   tint / shade / tone / blend / mso_tint / mso_shade
//
// The 24 built-in indices are Visio's default colour table; a document's Colors element
// (ColorEntry IX RGB) overrides them, and indices past 23 exist only there.

use alloc::string::String;

/// a colour value
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Color {
    /// an index into the document's colour table (0 black, 1 white ...)
    Index(u32),
    /// red, green, blue
    Rgb(u8, u8, u8),
}

/// Visio's default colour table (indices 0..23)
pub const DEFAULT_COLORS: [(u8, u8, u8); 24] = [
    (0, 0, 0), (255, 255, 255), (255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0), (255, 0, 255),
    (0, 255, 255), (128, 0, 0), (0, 128, 0), (0, 0, 128), (128, 128, 0), (128, 0, 128), (0, 128, 128),
    (192, 192, 192), (230, 230, 230), (205, 205, 205), (179, 179, 179), (154, 154, 154), (128, 128, 128),
    (102, 102, 102), (77, 77, 77), (51, 51, 51), (26, 26, 26),
];

impl Color {
    /// "#rrggbb" (any case) as an RGB colour
    pub fn parse(text: &str) -> Option<Color> {
        let h = text.strip_prefix('#')?;
        if h.len() != 6 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// "#rrggbb" in lower case, as Visio writes V
    pub fn hex(r: u8, g: u8, b: u8) -> String {
        const D: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(7);
        s.push('#');
        for c in [r, g, b] {
            s.push(D[(c >> 4) as usize] as char);
            s.push(D[(c & 15) as usize] as char);
        }
        s
    }
}

pub(crate) const HLSMAX: i32 = 240;
const RGBMAX: i32 = 255;
const UNDEFINED: i32 = HLSMAX * 2 / 3;

/// RGB -> hue, saturation, luminosity on 0..240 (Windows ColorRGBToHLS)
pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (i32, i32, i32) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let cmax = r.max(g).max(b);
    let cmin = r.min(g).min(b);
    let l = ((cmax + cmin) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    if cmax == cmin {
        return (UNDEFINED, 0, l);
    }
    let s = if l <= HLSMAX / 2 {
        ((cmax - cmin) * HLSMAX + (cmax + cmin) / 2) / (cmax + cmin)
    } else {
        ((cmax - cmin) * HLSMAX + (2 * RGBMAX - cmax - cmin) / 2) / (2 * RGBMAX - cmax - cmin)
    };
    let d = cmax - cmin;
    let rd = ((cmax - r) * (HLSMAX / 6) + d / 2) / d;
    let gd = ((cmax - g) * (HLSMAX / 6) + d / 2) / d;
    let bd = ((cmax - b) * (HLSMAX / 6) + d / 2) / d;
    let mut h = if r == cmax {
        bd - gd
    } else if g == cmax {
        HLSMAX / 3 + rd - bd
    } else {
        2 * HLSMAX / 3 + gd - rd
    };
    if h < 0 {
        h += HLSMAX;
    }
    if h > HLSMAX {
        h -= HLSMAX;
    }
    (h, s, l)
}

fn hue_to_rgb(n1: i32, n2: i32, mut hue: i32) -> i32 {
    if hue < 0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    if hue < HLSMAX / 6 {
        n1 + ((n2 - n1) * hue + HLSMAX / 12) / (HLSMAX / 6)
    } else if hue < HLSMAX / 2 {
        n2
    } else if hue < HLSMAX * 2 / 3 {
        n1 + ((n2 - n1) * (HLSMAX * 2 / 3 - hue) + HLSMAX / 12) / (HLSMAX / 6)
    } else {
        n1
    }
}

/// hue, saturation, luminosity on 0..240 -> RGB (Windows ColorHLSToRGB); out-of-range
/// arguments are clamped
pub fn hsl_to_rgb(h: i32, s: i32, l: i32) -> (u8, u8, u8) {
    let h = h.rem_euclid(HLSMAX + 1);
    let s = s.clamp(0, HLSMAX);
    let l = l.clamp(0, HLSMAX);
    if s == 0 {
        // grey: rounded (Visio; Windows truncates here)
        let v = ((l * RGBMAX + HLSMAX / 2) / HLSMAX) as u8;
        return (v, v, v);
    }
    let m2 = if l <= HLSMAX / 2 { (l * (HLSMAX + s) + HLSMAX / 2) / HLSMAX } else { l + s - (l * s + HLSMAX / 2) / HLSMAX };
    let m1 = 2 * l - m2;
    let c = |x: i32| ((x * RGBMAX + HLSMAX / 2) / HLSMAX).clamp(0, 255) as u8;
    (c(hue_to_rgb(m1, m2, h + HLSMAX / 3)), c(hue_to_rgb(m1, m2, h)), c(hue_to_rgb(m1, m2, h - HLSMAX / 3)))
}

fn clamp_u8(x: f64) -> u8 {
    let r = super::math::round(x);
    if r <= 0.0 {
        0
    } else if r >= 255.0 {
        255
    } else {
        r as u8
    }
}

/// TINT(color, amount): luminosity raised by `amount` (0..240 scale, truncated; negative
/// lowers it)
pub fn tint(rgb: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let (h, s, l) = rgb_to_hsl(rgb.0, rgb.1, rgb.2);
    hsl_to_rgb(h, s, l + super::math::trunc(amount).clamp(-32767.0, 32767.0) as i32)
}

/// SHADE(color, amount): luminosity lowered by `amount`
pub fn shade(rgb: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    tint(rgb, -amount)
}

/// TONE(color, amount): saturation lowered by `amount` (negative raises it)
pub fn tone(rgb: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let (h, s, l) = rgb_to_hsl(rgb.0, rgb.1, rgb.2);
    hsl_to_rgb(h, s - super::math::trunc(amount) as i32, l)
}

/// BLEND(a, b, part): `part` (0..1) of b mixed into a
pub fn blend(a: (u8, u8, u8), b: (u8, u8, u8), part: f64) -> (u8, u8, u8) {
    let t = part.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| clamp_u8(x as f64 * (1.0 - t) + y as f64 * t);
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// RGB (0..255) -> hue, luminosity, saturation as fractions (Python's colorsys)
pub fn rgb_to_hls_f(rgb: (u8, u8, u8)) -> (f64, f64, f64) {
    let (r, g, b) = (rgb.0 as f64 / 255.0, rgb.1 as f64 / 255.0, rgb.2 as f64 / 255.0);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, l, 0.0);
    }
    let d = mx - mn;
    let s = if l <= 0.5 { d / (mx + mn) } else { d / (2.0 - mx - mn) };
    let (rc, gc, bc) = ((mx - r) / d, (mx - g) / d, (mx - b) / d);
    let h = if r == mx {
        bc - gc
    } else if g == mx {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    (super::math::modulus(h / 6.0, 1.0), l, s)
}

fn hls_part(m1: f64, m2: f64, h: f64) -> f64 {
    let h = super::math::modulus(h, 1.0);
    if h < 1.0 / 6.0 {
        m1 + (m2 - m1) * h * 6.0
    } else if h < 0.5 {
        m2
    } else if h < 2.0 / 3.0 {
        m1 + (m2 - m1) * (2.0 / 3.0 - h) * 6.0
    } else {
        m1
    }
}

/// hue, luminosity, saturation as fractions -> RGB fractions (Python's colorsys)
pub fn hls_to_rgb_f(h: f64, l: f64, s: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let m2 = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let m1 = 2.0 * l - m2;
    (hls_part(m1, m2, h + 1.0 / 3.0), hls_part(m1, m2, h), hls_part(m1, m2, h - 1.0 / 3.0))
}

/// MSOTINT(color, amount): Office's tint in per cent on the luminosity of a fractional HLS,
/// positive towards white, negative towards black, channels truncated
pub fn mso_tint(rgb: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    let p = amount.clamp(-100.0, 100.0) / 100.0;
    let (h, l, s) = rgb_to_hls_f(rgb);
    let l2 = if p >= 0.0 { l + p * (1.0 - l) } else { l * (1.0 + p) };
    let (r, g, b) = hls_to_rgb_f(h, l2, s);
    let c = |x: f64| super::math::trunc(255.0 * x + 1e-9).clamp(0.0, 255.0) as u8;
    (c(r), c(g), c(b))
}

/// MSOSHADE(color, amount): the opposite of MSOTINT
pub fn mso_shade(rgb: (u8, u8, u8), amount: f64) -> (u8, u8, u8) {
    mso_tint(rgb, -amount)
}
