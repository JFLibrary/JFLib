// bidi -- lines with right-to-left characters in their visual order, also without set_shaping
//
//   has_rtl(s)                       s has right-to-left characters (what set_bidi lays out)
//   set_bidi(on) -> previous         (default on) a line with right-to-left characters (Hebrew,
//                                    Arabic, Syriac, Thaana, N'Ko, ... or the marks RLM, RLE,
//                                    RLO, RLI) is laid out by the Unicode bidi algorithm while
//                                    shaping is off; off: every line in logical order (text
//                                    that is in visual order already)
//
// The line's directional runs come in visual order (text_shape::visual_runs; the line's
// direction is that of its first strong character).  A right-to-left run's characters from a
// TrueType / OpenType font (the fallback font of Hebrew or Arabic, usually) are shaped with
// it (HarfRust: Arabic joining, marks, ligatures, mirrored brackets); the others are drawn
// one grapheme cluster after the other as run draws them, right to left, brackets mirrored.
// Left-to-right runs are drawn as run draws them.  Draw, measure, caret_x, hit and
// selection_spans all read the same layout (shaping.rs: the line laid out last, with the 128
// bidi lines before it).  Lines without right-to-left characters keep run's metrics and
// pixels.

use alloc::string::String;
use alloc::vec::Vec;

use crate::shaping::{Line, Placed, LINE_BIDI};
use crate::{segment, Text};

const LINES: usize = 128; // earlier bidi layouts kept (a window full of right-to-left labels)
const ENGINES: usize = 8; // shaping engines kept (fonts of right-to-left runs)
const NONE: u32 = u32::MAX;

/// a character that is right to left (strong R / AL) or opens a right-to-left level
pub(crate) fn rtl_char(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF
        | 0x200F | 0x202B | 0x202E | 0x2067)
}

// Bidi_Mirroring_Glyph pairs: the brackets, quotes and relations a right-to-left run shows
// mirrored
const MIRRORED: [(u32, u32); 26] = [
    (0x28, 0x29), (0x3C, 0x3E), (0x5B, 0x5D), (0x7B, 0x7D), (0xAB, 0xBB), (0x2039, 0x203A), (0x2045, 0x2046),
    (0x207D, 0x207E), (0x208D, 0x208E), (0x2208, 0x220B), (0x2264, 0x2265), (0x2266, 0x2267), (0x2282, 0x2283),
    (0x2286, 0x2287), (0x2329, 0x232A), (0x27E6, 0x27E7), (0x27E8, 0x27E9), (0x3008, 0x3009), (0x300A, 0x300B),
    (0x300C, 0x300D), (0x300E, 0x300F), (0x3010, 0x3011), (0x3014, 0x3015), (0xFF08, 0xFF09), (0xFF3B, 0xFF3D),
    (0xFF5B, 0xFF5D),
];

fn mirror(cp: u32) -> u32 {
    for (a, b) in MIRRORED {
        if cp == a {
            return b;
        }
        if cp == b {
            return a;
        }
    }
    cp
}

/// Whether s has right-to-left characters (Hebrew, Arabic, ... or the marks RLM, RLE, RLO,
/// RLI): a line with them is laid out in its visual order (set_bidi). Views that place their
/// carets by logical prefixes draw such text cluster by cluster to keep the logical order.
pub fn has_rtl(s: &[u8]) -> bool {
    s.iter().any(|&c| c >= 0xD6) && s.utf8_chunks().any(|c| c.valid().chars().any(rtl_char))
}

// the carets and character spans of the cluster a..b drawn from left to right (24.8; rtl: its
// start is on the right); the characters of a ligature share it evenly (as shape_line)
fn cluster_carets(l: &mut Line, text: &str, a: usize, b: usize, left: i32, right: i32, rtl: bool) {
    let (begin, end) = if rtl { (right, left) } else { (left, right) };
    l.carets.push((a, begin));
    let chars: Vec<usize> = text[a..b].char_indices().map(|(o, _)| a + o).collect();
    let n = chars.len().max(1) as i64;
    for (k, &index) in chars.iter().enumerate() {
        let x0 = begin + ((end - begin) as i64 * k as i64 / n) as i32;
        let x1 = begin + ((end - begin) as i64 * (k as i64 + 1) / n) as i32;
        l.segments.push((index, chars.get(k + 1).copied().unwrap_or(b), x0.min(x1), x0.max(x1)));
        if k > 0 {
            l.carets.push((index, x0));
        }
    }
    l.carets.push((b, end));
}

impl Text {
    /// Lay out lines with right-to-left characters in their visual order while shaping is off
    /// (default on). Returns the previous state.
    pub fn set_bidi(&mut self, on: bool) -> bool {
        !core::mem::replace(&mut self.shaping.bidi_off, !on)
    }

    // (shaping.rs) the line laid out last is line; a bidi layout it replaces is kept
    pub(crate) fn line_set(&mut self, line: Line) {
        let old = core::mem::replace(&mut self.shaping.line, line);
        if old.kind == LINE_BIDI {
            if self.shaping.lines.len() >= LINES {
                self.shaping.lines.remove(0);
            }
            self.shaping.lines.push(old);
        }
    }

    // (shaping.rs: shape_line while shaping is off) true: the line has right-to-left
    // characters and is laid out in shaping.line
    pub(crate) fn bidi_line(&mut self, bytes: &[u8], limit: Option<usize>) -> bool {
        if self.shaping.bidi_off {
            return false;
        }
        let n = limit.unwrap_or(bytes.len()).min(bytes.len());
        let n = bytes[..n].iter().position(|b| *b == 0 || *b == b'\n').unwrap_or(n);
        let b = &bytes[..n];
        if !has_rtl(b) {
            return false;
        }
        // a UTF-8 sequence cut at the end ends the line (as run); other broken text: run
        let text = match core::str::from_utf8(b) {
            Ok(t) => t,
            Err(e) if e.error_len().is_none() => match core::str::from_utf8(&b[..e.valid_up_to()]) {
                Ok(t) => t,
                Err(_) => return false,
            },
            Err(_) => return false,
        };
        let (font, size, reg) = (self.font, self.size, self.serial);
        let same = |l: &Line| l.kind == LINE_BIDI && l.font == font && l.size == size && l.reg == reg && l.text == text;
        if same(&self.shaping.line) {
            return true;
        }
        if let Some(i) = self.shaping.lines.iter().position(same) {
            let l = self.shaping.lines.remove(i);
            self.line_set(l);
            return true;
        }
        let mut l = self.bidi_layout(text);
        l.reg = self.serial; // fallback fonts it loaded count
        self.line_set(l);
        true
    }

    // the font of a character as run picks it: the run's font when it has it, else the first
    // of the fallback chain that has it (NONE: none has it)
    fn bidi_font(&mut self, cp: u32) -> u32 {
        if self.has(self.font, cp) {
            self.font
        } else if self.fallback_active() {
            self.fallback_for(cp).unwrap_or(NONE)
        } else {
            NONE
        }
    }

    // no-break, narrow and other spaces of number formats: a plain space when missing (as run)
    fn bidi_space(&self, cp: u32) -> u32 {
        if matches!(cp, 0xA0 | 0x2000..=0x200A | 0x202F | 0x205F) && !self.has(self.font, cp) { 0x20 } else { cp }
    }

    fn bidi_layout(&mut self, text: &str) -> Line {
        let font = self.font;
        let classic = self.classic_metrics(font);
        let spacing = |v: i32| if classic { v.saturating_add(128) & !255 } else { v };
        let mut l = Line { kind: LINE_BIDI, text: String::from(text), font, size: self.size, ..Line::default() };
        let mut pen = 0i32;
        for (run, rtl) in text_shape::visual_runs(text) {
            // the run's grapheme clusters with the font of their first character; one that no
            // font has takes the font of the cluster before it (at the start: after it)
            let bounds = segment::graphemes(&text[run.clone()]);
            let mut clusters: Vec<(usize, usize, u32)> = Vec::new();
            for w in bounds.windows(2) {
                let (a, b) = (run.start + w[0], run.start + w[1]);
                let cp = text[a..b].chars().next().map_or(0x20, |c| c as u32);
                let f = self.bidi_font(self.bidi_space(cp));
                clusters.push((a, b, f));
            }
            let first = clusters.iter().map(|c| c.2).find(|&f| f != NONE).unwrap_or(font);
            let mut last = first;
            for c in clusters.iter_mut() {
                if c.2 == NONE {
                    c.2 = last;
                }
                last = c.2;
            }
            // the font runs: clusters of one font, in visual order
            let mut segs: Vec<(usize, usize, u32)> = Vec::new();
            for &(a, b, f) in &clusters {
                match segs.last_mut() {
                    Some(s) if s.2 == f => s.1 = b,
                    _ => segs.push((a, b, f)),
                }
            }
            if rtl {
                segs.reverse();
            }
            let mut prev: Option<(u32, u32)> = None; // the glyph to the left (kerning)
            for (sa, sb, f) in segs {
                if rtl
                    && let Some(glyphs) = self.bidi_shape(f, &text[sa..sb])
                {
                    let mut i = 0;
                    while i < glyphs.len() {
                        let first = glyphs[i];
                        let left = pen;
                        while i < glyphs.len() && glyphs[i].cluster == first.cluster {
                            let g = glyphs[i];
                            l.glyphs.push(Placed { font: f, id: g.id, x: pen.saturating_add(self.units_fx_of(f, g.x)), y: self.units_fx_of(f, g.y) });
                            pen = pen.saturating_add(spacing(self.units_fx_of(f, g.advance)));
                            i += 1;
                        }
                        cluster_carets(&mut l, text, sa + first.cluster, sa + first.end, left, pen, first.rtl);
                    }
                    prev = None;
                    continue;
                }
                // one cluster after the other (right to left in a right-to-left run), each
                // character in the font run picks for it
                let mut cl: Vec<(usize, usize)> = clusters.iter().filter(|c| c.0 >= sa && c.1 <= sb).map(|c| (c.0, c.1)).collect();
                if rtl {
                    cl.reverse();
                }
                for (ca, cb) in cl {
                    let left = pen;
                    for ch in text[ca..cb].chars() {
                        let cp = self.bidi_space(ch as u32);
                        if cp == 0xAD {
                            continue; // SOFT HYPHEN: invisible
                        }
                        let cp = if rtl && self.has(f, mirror(cp)) { mirror(cp) } else { cp };
                        let cf = if self.has(f, cp) {
                            f
                        } else {
                            match self.bidi_font(cp) {
                                NONE if crate::fallback::never(cp) => continue, // a control no font has: nothing
                                NONE => f,
                                other => other,
                            }
                        };
                        let g = self.glyph(cf, cp);
                        if let Some((pf, pg)) = prev
                            && pf == cf
                        {
                            let k = self.kern(cf, pg, g);
                            if k != 0 {
                                pen = pen.wrapping_add(spacing(self.units_fx_of(cf, k)));
                            }
                        }
                        l.glyphs.push(Placed { font: cf, id: g, x: pen, y: 0 });
                        pen = pen.wrapping_add(spacing(self.units_fx_of(cf, self.adv(cf, g))));
                        prev = Some((cf, g));
                    }
                    cluster_carets(&mut l, text, ca, cb, left, pen, rtl);
                }
            }
        }
        if l.carets.is_empty() {
            l.carets.push((0, 0));
        }
        l.carets.sort_by_key(|c| c.0);
        l.carets.dedup_by_key(|c| c.0);
        l.width = pen;
        l
    }

    // text (a right-to-left font run) shaped with font f: None when f is no TrueType /
    // OpenType font or shaping fails
    fn bidi_shape(&mut self, f: u32, text: &str) -> Option<Vec<text_shape::Glyph>> {
        self.shape_face(f)?;
        let serial = self.shape_serial(f);
        let i = match self.shaping.engines.iter().position(|e| e.0 == f && e.1 == serial) {
            Some(i) => i,
            None => {
                let e = self.shape_engine(f)?;
                if self.shaping.engines.len() >= ENGINES {
                    self.shaping.engines.remove(0);
                }
                self.shaping.engines.push((f, serial, e));
                self.shaping.engines.len() - 1
            }
        };
        self.shaping.engines[i].2.shape_run(text, true)
    }
}
