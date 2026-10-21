// font -- embedded SGF1 fonts (tools/ttf2bin.py, ASMLIB/docs/FONTS.md), the font header and
// FONTMETRICS records shared by every font kind                       [src/text/font.s]
//
//   Font::parse(data)         the 64-byte header, the fast codepoint table (None: not SGF1)
//   glyph(cp) -> id           fallback glyph when unmapped      lookup(cp) -> Option<id>
//   adv(g), kern(g1, g2)      font units      glyph_box(g) -> [xmin, ymin, xmax, ymax]
//   path(g, gp, path)         append the outline (TrueType quadratics, composites)
//   outline(g, dx, dy, ol)    the outline into the outline buffer (sgf_outline, outline.s)

use gfx::path::Path;

use crate::outline::Outline;

pub const FAST_CP: u32 = 384;
pub const SGF1_MAGIC: u32 = 0x3146_4753;

// SGF1 header flags (SG_FLAGS)
pub const SGH_ICON: u16 = 1;
pub const SGH_KERN: u16 = 2;
pub const SGH_RUNTIME: u16 = 0x100;
pub const SGH_COLR: u16 = 0x200; // COLR layers (text runs draw them layer by layer)
pub const SGH_CBDT: u16 = 0x400; // CBDT bitmaps (PARGB glyph cache entries)

// font_style flags
pub const FS_SYN_BOLD: u32 = 1;
pub const FS_SYN_ITALIC: u32 = 2;
pub const FS_SHARP: u32 = 4;
/// Classic screen typography: round kerning and advances to whole device pixels.
pub const FS_CLASSIC: u32 = 8;
pub const FS_MASK: u32 = 15;

// text_decor flags
pub const TD_UNDERLINE: u32 = 1;
pub const TD_STRIKEOUT: u32 = 2;

// font_name ids (OpenType name IDs)
pub const FN_FAMILY: u32 = 1;
pub const FN_STYLE: u32 = 2;
pub const FN_FULL: u32 = 4;
pub const FN_TYPO_FAMILY: u32 = 16;
pub const FN_TYPO_STYLE: u32 = 17;

// FONTMETRICS byte offsets (font_metric)
pub const FM_UPEM: u32 = 0;
pub const FM_ASC: u32 = 4;
pub const FM_DESC: u32 = 8;
pub const FM_GAP: u32 = 12;
pub const FM_TYPO_ASC: u32 = 16;
pub const FM_TYPO_DESC: u32 = 20;
pub const FM_TYPO_GAP: u32 = 24;
pub const FM_WIN_ASC: u32 = 28;
pub const FM_WIN_DESC: u32 = 32;
pub const FM_XH: u32 = 36;
pub const FM_CAPH: u32 = 40;
pub const FM_UL_POS: u32 = 44;
pub const FM_UL_THICK: u32 = 48;
pub const FM_ST_POS: u32 = 52;
pub const FM_ST_THICK: u32 = 56;
pub const FM_WEIGHT: u32 = 60;
pub const FM_FLAGS: u32 = 64;
pub const FM_NGLYPHS: u32 = 68;
pub const FM_ADVMAX: u32 = 72;
pub const FM_WIDTH: u32 = 76;
pub const FM_SIZE: u32 = 80;

// FONTMETRICS flags
pub const FMF_ITALIC: i32 = 1;
pub const FMF_BOLD: i32 = 2;
pub const FMF_OBLIQUE: i32 = 4;
pub const FMF_COLOR: i32 = 8;
pub const FMF_CFF: i32 = 16;
pub const FMF_VAR: i32 = 32;
pub const FMF_SYMBOL: i32 = 64;
pub const FMF_MONO: i32 = 128;
pub const FMF_TYPO: i32 = 256;
pub const FMF_EMBED: i32 = 512;
pub const FMF_SYN: i32 = 1024;
pub const FMF_BITMAP: i32 = 2048;
pub const FMF_KERN: i32 = 4096;

/// the SGF1-compatible header every font kind has (font_data): what gc_scale and the
/// text metrics read.  i16 fields sign-extended, upem / nglyphs zero-extended.
#[derive(Clone, Copy, Default)]
pub struct Hdr {
    pub flags: u16,
    pub upem: i32,
    pub asc: i32,
    pub desc: i32,
    pub gap: i32,
    pub caph: i32,
    pub xh: i32,
    pub ulpos: i32,
    pub ulthick: i32,
    pub nglyphs: u32,
    pub baseid: u32, // derived fonts: base font id + 1 (0 = none)
}

/// FONTMETRICS (font.inc): all values in font units (y up)
#[derive(Clone, Copy, Default)]
pub struct Metrics {
    pub upem: i32,
    pub asc: i32,
    pub desc: i32,
    pub gap: i32,
    pub typo_asc: i32,
    pub typo_desc: i32,
    pub typo_gap: i32,
    pub win_asc: i32,
    pub win_desc: i32,
    pub xh: i32,
    pub caph: i32,
    pub ul_pos: i32,
    pub ul_thick: i32,
    pub st_pos: i32,
    pub st_thick: i32,
    pub weight: i32,
    pub flags: i32,
    pub nglyphs: i32,
    pub advmax: i32,
    pub width: i32,
}

impl Metrics {
    /// the field at byte offset FM_* (0 for an invalid field)  [font_metric]
    pub fn field(&self, off: u32) -> i32 {
        if off > FM_SIZE - 4 || off & 3 != 0 {
            return 0;
        }
        self.as_array()[(off / 4) as usize]
    }

    pub fn as_array(&self) -> [i32; 20] {
        [
            self.upem, self.asc, self.desc, self.gap, self.typo_asc, self.typo_desc, self.typo_gap,
            self.win_asc, self.win_desc, self.xh, self.caph, self.ul_pos, self.ul_thick, self.st_pos,
            self.st_thick, self.weight, self.flags, self.nglyphs, self.advmax, self.width,
        ]
    }
}

/// the glyph transform (font.s g_gp): origin in float pixels, pixels per font unit,
/// composite offset in font units
#[derive(Clone, Copy, Default)]
pub struct Gp {
    pub x: f32,
    pub y: f32,
    pub sx: f32,
    pub sy: f32,
    pub dx: i32,
    pub dy: i32,
}

pub struct Font {
    data: &'static [u8],
    pub flags: u16,
    pub upem: i32,
    pub asc: i32,
    pub desc: i32,
    pub gap: i32,
    pub caph: i32,
    pub xh: i32,
    pub ulpos: i32,
    pub ulthick: i32,
    pub nglyphs: u32,
    nranges: u32,
    kern_r: u32,
    off_ranges: usize,
    off_glyphs: usize,
    off_outlines: usize,
    off_kern: usize,
    pub fallback: u32,
    fmap: [u16; FAST_CP as usize],
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

fn i16_at(d: &[u8], o: usize) -> i32 {
    i16::from_le_bytes([d[o], d[o + 1]]) as i32
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl Font {
    /// font_register's parse: None when the data is not SGF1
    pub fn parse(data: &'static [u8]) -> Option<Font> {
        if data.len() < 64 || u32_at(data, 0) != SGF1_MAGIC {
            return None;
        }
        let mut f = Font {
            data,
            flags: u16_at(data, 6),
            upem: u16_at(data, 8) as i32,
            asc: i16_at(data, 10),
            desc: i16_at(data, 12),
            gap: i16_at(data, 14),
            caph: i16_at(data, 16),
            xh: i16_at(data, 18),
            ulpos: i16_at(data, 20),
            ulthick: i16_at(data, 22),
            nglyphs: u16_at(data, 24) as u32,
            nranges: u16_at(data, 26) as u32,
            kern_r: u16_at(data, 30) as u32,
            off_ranges: u32_at(data, 32) as usize,
            off_glyphs: u32_at(data, 36) as usize,
            off_outlines: u32_at(data, 40) as usize,
            off_kern: u32_at(data, 44) as usize,
            fallback: u16_at(data, 48) as u32,
            fmap: [0; FAST_CP as usize],
        };
        for cp in 0..FAST_CP {
            f.fmap[cp as usize] = f.glyph_slow(cp) as u16;
        }
        Some(f)
    }

    pub fn new(data: &'static [u8]) -> Font {
        Font::parse(data).expect("not an SGF1 font")
    }

    pub fn hdr(&self) -> Hdr {
        Hdr {
            flags: self.flags,
            upem: self.upem,
            asc: self.asc,
            desc: self.desc,
            gap: self.gap,
            caph: self.caph,
            xh: self.xh,
            ulpos: self.ulpos,
            ulthick: self.ulthick,
            nglyphs: self.nglyphs,
            baseid: 0,
        }
    }

    /// binary search over the ranges {u32 firstCp, u16 count, u16 firstGlyph}  [font_lookup]
    pub fn lookup(&self, cp: u32) -> Option<u32> {
        let (mut lo, mut hi) = (0u32, self.nranges);
        while lo < hi {
            let mid = (lo + hi) >> 1;
            let o = self.off_ranges + mid as usize * 8;
            let first = u32_at(self.data, o);
            if cp < first {
                hi = mid;
                continue;
            }
            let count = u16_at(self.data, o + 4) as u32;
            if cp >= first.wrapping_add(count) {
                lo = mid + 1;
                continue;
            }
            return Some((cp - first).wrapping_add(u16_at(self.data, o + 6) as u32));
        }
        None
    }

    fn glyph_slow(&self, cp: u32) -> u32 {
        self.lookup(cp).unwrap_or(self.fallback)
    }

    pub fn glyph(&self, cp: u32) -> u32 {
        if cp < FAST_CP { self.fmap[cp as usize] as u32 } else { self.glyph_slow(cp) }
    }

    fn rec(&self, g: u32) -> Option<usize> {
        if g >= self.nglyphs { None } else { Some(self.off_glyphs + g as usize * 16) }
    }

    pub fn adv(&self, g: u32) -> i32 {
        self.rec(g).map_or(0, |r| u16_at(self.data, r) as i32)
    }

    /// kern(l, r) = matrix[lclass[l] * kernR + rclass[r]] * 4 font units
    pub fn kern(&self, g1: u32, g2: u32) -> i32 {
        if self.flags & SGH_KERN == 0 || g1 >= self.nglyphs || g2 >= self.nglyphs {
            return 0;
        }
        let n = self.nglyphs as usize;
        let k = self.off_kern;
        let lc = self.data[k + g1 as usize] as u32;
        let rc = self.data[k + n + g2 as usize] as u32;
        if lc == 0 || rc == 0 {
            return 0;
        }
        let m = (lc * self.kern_r + rc) as usize;
        (self.data[k + 2 * n + m] as i8 as i32) << 2
    }

    pub fn glyph_box(&self, g: u32) -> Option<[i32; 4]> {
        let r = self.rec(g)?;
        if u16_at(self.data, r + 10) == 0 {
            return None;
        }
        Some([
            i16_at(self.data, r + 2),
            i16_at(self.data, r + 4),
            i16_at(self.data, r + 6),
            i16_at(self.data, r + 8),
        ])
    }

    // gp_point: font units + composite offset -> float pixels (y up -> y down)
    fn point(gp: &Gp, fx: i32, fy: i32) -> (f32, f32) {
        let x = (fx.wrapping_add(gp.dx)) as f32 * gp.sx + gp.x;
        let y = gp.y - (fy.wrapping_add(gp.dy)) as f32 * gp.sy;
        (x, y)
    }

    /// glyph_path: append the outline transformed by gp (implied on-curve midpoints,
    /// composites recursively)
    pub fn path(&self, g: u32, gp: &mut Gp, path: &mut Path, depth: u32) {
        let Some(r) = self.rec(g) else { return };
        let n = u16_at(self.data, r + 10) as usize;
        if n == 0 || depth > 64 {
            return;
        }
        let ol = self.off_outlines + u32_at(self.data, r + 12) as usize;
        if n & 0x8000 != 0 {
            for c in 0..n & 0x7FFF {
                let o = ol + c * 8;
                let (sdx, sdy) = (gp.dx, gp.dy);
                gp.dx = gp.dx.wrapping_add(i16_at(self.data, o + 2));
                gp.dy = gp.dy.wrapping_add(i16_at(self.data, o + 4));
                self.path(u16_at(self.data, o) as u32, gp, path, depth + 1);
                gp.dx = sdx;
                gp.dy = sdy;
            }
            return;
        }
        let pts = ol + ((n + n + 3) & !3);
        let on = |i: usize| u16_at(self.data, pts + i * 4) & 1 != 0;
        let load = |i: usize| {
            let x = i16_at(self.data, pts + i * 4) >> 1;
            let y = i16_at(self.data, pts + i * 4 + 2);
            Self::point(gp, x, y)
        };
        let mut first = 0usize;
        for c in 0..n {
            let last = u16_at(self.data, ol + c * 2) as usize;
            if last < first {
                first = last + 1;
                continue;
            }
            let count = last - first + 1;
            // start: the first on-curve point of first / last, else their midpoint
            let (start, mut next, mut left);
            if on(first) {
                start = load(first);
                next = first + 1;
                left = count - 1;
            } else if on(last) {
                start = load(last);
                next = first;
                left = count - 1;
            } else {
                let a = load(last);
                let b = load(first);
                start = ((b.0 + a.0) * 0.5, (b.1 + a.1) * 0.5);
                next = first;
                left = count;
            }
            path.move_f(start.0, start.1);
            let mut pending: Option<(f32, f32)> = None;
            while left > 0 {
                left -= 1;
                let i = next;
                next += 1;
                let p = load(i);
                if on(i) {
                    match pending.take() {
                        Some(c) => path.quad_f(c.0, c.1, p.0, p.1),
                        None => path.line_f(p.0, p.1),
                    }
                } else {
                    if let Some(c) = pending {
                        // two off-curve points: quad to their midpoint
                        path.quad_f(c.0, c.1, (p.0 + c.0) * 0.5, (p.1 + c.1) * 0.5);
                    }
                    pending = Some(p);
                }
            }
            if let Some(c) = pending {
                path.quad_f(c.0, c.1, start.0, start.1);
            }
            path.close();
            first = last + 1;
        }
    }

    /// append glyph g (composites resolved) to the outline buffer: points in font units +
    /// (dx, dy), one contour per SGF1 contour  [sgf_outline]
    pub fn outline(&self, g: u32, dx: i32, dy: i32, depth: u32, ol: &mut Outline) {
        if depth > 16 {
            return;
        }
        let Some(r) = self.rec(g) else { return };
        let n = u16_at(self.data, r + 10) as usize;
        if n == 0 {
            return;
        }
        let o = self.off_outlines + u32_at(self.data, r + 12) as usize;
        if n & 0x8000 != 0 {
            for c in 0..n & 0x7FFF {
                let e = o + c * 8;
                let cdx = i16_at(self.data, e + 2).wrapping_add(dx);
                let cdy = i16_at(self.data, e + 4).wrapping_add(dy);
                self.outline(u16_at(self.data, e) as u32, cdx, cdy, depth + 1, ol);
            }
            return;
        }
        let pts = o + ((n + n + 3) & !3);
        let mut pt = 0usize;
        for c in 0..n {
            let last = u16_at(self.data, o + c * 2) as usize;
            while pt <= last {
                let v = i16_at(self.data, pts + pt * 4);
                let tag = (v & 1) as u8;
                let x = (v >> 1).wrapping_add(dx) as f32;
                let y = i16_at(self.data, pts + pt * 4 + 2).wrapping_add(dy) as f32;
                ol.point(x, y, tag);
                pt += 1;
            }
            ol.close();
        }
    }
}
