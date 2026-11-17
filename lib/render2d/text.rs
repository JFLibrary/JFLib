// text -- fonts, text styles and the layout of text (G6c; docs/RENDER2D.md "Text"): what a scene draws of a text, and
// what the CPU path of G6e (lib/gfx outlines of the same layout) and picking in G6d (each text's extent) take from it.
//
// Fonts are TrueType or OpenType (also a collection's face) from their bytes: lib/text parses them (outlines, metrics),
// lib/text_shape shapes (HarfRust: kerning, ligatures, marks, Arabic joining, Indic reordering; Unicode bidi and script
// runs; fallback fonts per run that the first font cannot show).  A text style is DXF's STYLE: a font, fallback fonts,
// a fixed height or none, a width factor, an oblique angle; heights in drawing units, or in pixels for annotation that
// keeps its size on the screen.
//
// A layout is in em units of the text's frame (x along the baseline, y up; one em = the font's units per em): its lines
// (baseline origin, glyphs in visual order with their pens), the bars small text is drawn as, its ink box; and the frame
// itself: origin + m · e maps a point e of the frame to the drawing (or, for a style in pixels, to pixels from the origin).
// Heights are cap heights, as DXF's TEXT and MTEXT measure them (the em is height / cap height).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use gpu::{GpuError, Result};

use crate::math::{sincos, sqrt};
use crate::msdf::{self, Seg};
use crate::scene::Unit;

/// a font of a scene (Scene::add_font)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// a text style of a scene (Scene::add_text_style)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextStyleId(pub u32);

/// the most fallback fonts of a text style
pub const FALLBACK_MAX: usize = 4;

/// AutoCAD's single line spacing: baselines 5/3 of the text height apart (MText::line_spacing multiplies it)
pub const LINE_SPACING: f64 = 5.0 / 3.0;

/// DXF's STYLE: how texts of it look.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: FontId,
    /// fonts for what `font` lacks, tried in this order per run of one script (the first `fallbacks`)
    pub fallback: [FontId; FALLBACK_MAX],
    pub fallbacks: u32,
    /// a fixed cap height (0: each text says its own), in `unit`s
    pub height: f64,
    /// how much wider than the font (1: as it is)
    pub width: f64,
    /// how far the glyphs lean, radians from upright (positive: to the right)
    pub oblique: f64,
    /// Drawing: heights in drawing units, the text zooms with the drawing; Px: pixels, the text keeps its size on the screen
    pub unit: Unit,
}

impl TextStyle {
    /// a font upright, its own width, heights per text, in drawing units
    pub const fn new(font: FontId) -> TextStyle {
        TextStyle { font, fallback: [FontId(0); FALLBACK_MAX], fallbacks: 0, height: 0.0, width: 1.0, oblique: 0.0, unit: Unit::Drawing }
    }

    /// the same with fallback fonts (at most FALLBACK_MAX)
    pub fn with_fallbacks(mut self, fonts: &[FontId]) -> TextStyle {
        self.fallbacks = fonts.len().min(FALLBACK_MAX) as u32;
        for (d, f) in self.fallback.iter_mut().zip(fonts) {
            *d = *f;
        }
        self
    }

    pub(crate) fn check(&self, fonts: &Fonts) -> Result<()> {
        let fine = |v: f64| v.is_finite() && v.abs() < 1e30;
        if !fonts.has(self.font) || self.fallbacks as usize > FALLBACK_MAX || !self.fallback[..self.fallbacks as usize].iter().all(|f| fonts.has(*f)) {
            return Err(GpuError::Invalid("TextStyle: fonts the scene has, at most FALLBACK_MAX fallbacks"));
        }
        if !(fine(self.height) && self.height >= 0.0 && fine(self.width) && self.width > 0.0 && self.oblique.is_finite() && self.oblique.abs() < 1.4) {
            return Err(GpuError::Invalid("TextStyle: a height not negative, a width factor above 0, an oblique angle within 80 degrees"));
        }
        Ok(())
    }
}

/// DXF TEXT's horizontal alignment (group 72)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HAlign {
    Left,
    Center,
    Right,
    /// between `at` and `at2`: the height follows the width (the width factor stays)
    Aligned,
    /// the centre of the baseline and the middle of all the text's ink, descenders too
    Middle,
    /// between `at` and `at2`: the width factor follows (the height stays)
    Fit,
}

/// DXF TEXT's vertical alignment (group 73): the baseline, the font's descender, half the cap height, the cap height
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VAlign {
    Baseline,
    Bottom,
    Middle,
    Top,
}

/// MTEXT's attachment point (group 71, 1 to 9): where `at` lies on the text's block
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attach {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// a single line of text (DXF's TEXT).  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    /// one line (control characters are left out)
    pub text: &'a str,
    pub style: TextStyleId,
    /// the alignment point: the baseline's start for Left / Baseline (DXF 10), else the point the alignment names (DXF
    /// 11); Aligned and Fit run the baseline from `at` (DXF 10) to `at2` (DXF 11)
    pub at: [f64; 2],
    pub at2: [f64; 2],
    /// the cap height (None: the style's), in the style's unit
    pub height: Option<f64>,
    /// radians, counter-clockwise (Aligned and Fit: the direction from at to at2)
    pub rotation: f64,
    /// the width factor and the oblique angle (None: the style's; DXF 41, 51)
    pub width: Option<f64>,
    pub oblique: Option<f64>,
    pub halign: HAlign,
    pub valign: VAlign,
    /// DXF 71: mirrored in x (backward), mirrored in y (upside down)
    pub backward: bool,
    pub upside_down: bool,
}

impl<'a> Text<'a> {
    /// left on the baseline at `at`, unturned, the style's height, width and oblique angle
    pub const fn new(text: &'a str, style: TextStyleId, at: [f64; 2]) -> Text<'a> {
        Text { text, style, at, at2: at, height: None, rotation: 0.0, width: None, oblique: None, halign: HAlign::Left, valign: VAlign::Baseline, backward: false, upside_down: false }
    }
}

/// text of several lines (the basics of DXF's MTEXT: explicit breaks, a wrapping width, line spacing, the nine attachment
/// points, rotation; its inline formatting codes are G6e's, which maps them onto runs).  Made with `new` and changed field
/// by field (it may grow: runs with their own font, height and colour)
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct MText<'a> {
    /// lines broken at '\n'
    pub text: &'a str,
    pub style: TextStyleId,
    pub at: [f64; 2],
    /// the cap height (None: the style's), in the style's unit
    pub height: Option<f64>,
    /// wrap lines longer than this (0: no wrapping), in the style's unit
    pub width: f64,
    pub rotation: f64,
    pub attach: Attach,
    /// times LINE_SPACING times the height between baselines (DXF 44)
    pub line_spacing: f64,
}

impl<'a> MText<'a> {
    /// top left at `at`, no wrapping, single spacing
    pub const fn new(text: &'a str, style: TextStyleId, at: [f64; 2]) -> MText<'a> {
        MText { text, style, at, height: None, width: 0.0, rotation: 0.0, attach: Attach::TopLeft, line_spacing: 1.0 }
    }
}

/// a glyph of a layout
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub font: FontId,
    pub glyph: u32,
    /// its origin, em units from its line's origin
    pub pen: [f64; 2],
    /// its advance (em)
    pub advance: f64,
    /// the byte where its cluster starts in the text
    pub cluster: u32,
    /// a mark (no advance: an accent or vowel sign over its base): it has no cell of its own
    pub mark: bool,
    /// whitespace or no outline: no ink, no bar
    pub blank: bool,
}

/// a line of a layout: its baseline's origin (em, in the frame), its glyphs (a range of Layout::glyphs) and bars, its
/// advance width (em)
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub origin: [f64; 2],
    pub glyphs: core::ops::Range<u32>,
    pub bars: core::ops::Range<u32>,
    pub width: f64,
}

/// a word drawn as one bar when its text is small (em, from its line's origin): from its ink's left to its right, from the
/// baseline to the x-height, `ink` the share of that box its glyphs cover
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    pub ink: f64,
}

/// a text laid out: in em units of its frame, and the frame
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// where the frame's origin lies (drawing units) and its map from em units: origin + (m[0] x + m[1] y, m[2] x + m[3] y)
    /// (a style in pixels: pixels from the origin, y up, before the camera's rotation)
    pub origin: [f64; 2],
    pub m: [f64; 4],
    pub px: bool,
    pub glyphs: Vec<Placed>,
    pub lines: Vec<Line>,
    pub bars: Vec<Bar>,
    /// the box of the glyphs' ink (em, in the frame; empty text: None)
    pub ink: Option<[f64; 4]>,
    /// the box of the lines: advances, descender to cap height (em, in the frame)
    pub cell: [f64; 4],
    /// the em in drawing units (or pixels): what the zoom decides by
    pub em: f64,
    /// the x-height (em) the bars stand on
    pub x_height: f64,
}

impl Layout {
    /// a point of the frame (em) in the drawing (a style in pixels: pixels from the origin)
    pub fn to_drawing(&self, e: [f64; 2]) -> [f64; 2] {
        let m = &self.m;
        [self.origin[0] + m[0] * e[0] + m[1] * e[1], self.origin[1] + m[2] * e[0] + m[3] * e[1]]
    }

    /// the corners of a box of the frame in the drawing (counter-clockwise from its lower left in the frame)
    pub fn quad_of(&self, b: [f64; 4]) -> [[f64; 2]; 4] {
        [self.to_drawing([b[0], b[1]]), self.to_drawing([b[2], b[1]]), self.to_drawing([b[2], b[3]]), self.to_drawing([b[0], b[3]])]
    }

    /// the text's extent in the drawing: its ink box's corners (picking, G6d), or the cell's where it has no ink
    pub fn quad(&self) -> [[f64; 2]; 4] {
        self.quad_of(self.ink.unwrap_or(self.cell))
    }

    /// the axis-aligned box of quad() (drawing units)
    pub fn bounds(&self) -> [f64; 4] {
        let q = self.quad();
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in q {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        b
    }
}

/// what a scene knows of a glyph: its contours (em), the box of its control hulls, the area it fills
#[derive(Clone, Debug)]
pub struct GlyphShape {
    pub contours: Vec<Vec<Seg>>,
    pub bbox: Option<[f64; 4]>,
    pub area: f64,
}

struct FontData {
    tt: Box<text::ttf::TtFont>,
    shaper: text_shape::Engine,
    upem: f64,
    /// em units: cap height, x-height, ascender, descender (negative)
    caph: f64,
    xh: f64,
    asc: f64,
    desc: f64,
    shapes: BTreeMap<u32, GlyphShape>,
}

/// the fonts of a scene (or of anyone laying out text: the CPU path of G6e)
#[derive(Default)]
pub struct Fonts {
    fonts: Vec<FontData>,
}

/// a font's metrics in em units (Fonts::metrics)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    pub units_per_em: f64,
    pub cap_height: f64,
    pub x_height: f64,
    pub ascender: f64,
    pub descender: f64,
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts { fonts: Vec::new() }
    }

    /// a font from TrueType or OpenType bytes (face: the font of a collection, else 0)
    pub fn add(&mut self, data: &[u8], face: u32) -> Result<FontId> {
        let mut ol = text::outline::Outline::default();
        let serial = 0x7d_0000 + self.fonts.len() as u64;
        let tt = text::ttf::TtFont::open(data.to_vec(), face, &mut ol, serial).map_err(|_| GpuError::Invalid("Fonts::add: not a TrueType or OpenType font this parser takes"))?;
        let shaper = text_shape::Engine::new(data, face).ok_or(GpuError::Invalid("Fonts::add: the shaper does not take this font"))?;
        let upem = tt.upem.max(1) as f64;
        let m = tt.fm;
        let caph = if m.caph > 0 { m.caph as f64 / upem } else if m.asc > 0 { 0.7 * m.asc as f64 / upem } else { 0.7 };
        let xh = if m.xh > 0 { m.xh as f64 / upem } else { 0.5 * caph / 0.7 };
        let asc = if m.asc > 0 { m.asc as f64 / upem } else { caph };
        let desc = if m.desc < 0 { m.desc as f64 / upem } else { -(m.desc.abs() as f64) / upem };
        self.fonts.push(FontData { tt, shaper, upem, caph, xh, asc, desc, shapes: BTreeMap::new() });
        Ok(FontId(self.fonts.len() as u32 - 1))
    }

    pub fn has(&self, f: FontId) -> bool {
        (f.0 as usize) < self.fonts.len()
    }

    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    pub fn metrics(&self, f: FontId) -> Option<FontMetrics> {
        let d = self.fonts.get(f.0 as usize)?;
        Some(FontMetrics { units_per_em: d.upem, cap_height: d.caph, x_height: d.xh, ascender: d.asc, descender: d.desc })
    }

    /// the glyph for a character (0: the font lacks it)
    pub fn glyph_of(&self, f: FontId, c: char) -> u32 {
        self.fonts.get(f.0 as usize).map_or(0, |d| d.tt.glyph(c as u32))
    }

    /// a glyph's contours as shape left them in the cache (the CPU path, G6e1: every glyph of a laid-out text is there)
    pub(crate) fn shape_cached(&self, f: FontId, g: u32) -> Option<&GlyphShape> {
        self.fonts.get(f.0 as usize)?.shapes.get(&g)
    }

    /// a glyph's contours in em units (y up) and what is known of them (cached)
    pub fn shape(&mut self, f: FontId, g: u32) -> Option<&GlyphShape> {
        let d = self.fonts.get_mut(f.0 as usize)?;
        if !d.shapes.contains_key(&g) {
            let mut ol = text::outline::Outline::default();
            let contours = if d.tt.prep(g, &mut ol) { msdf::contours_of(&ol.pts, &ol.tags, &ol.ends, 1.0 / d.upem) } else { Vec::new() };
            let bbox = msdf::bounds(&contours);
            let area = msdf::ink_area(&contours);
            d.shapes.insert(g, GlyphShape { contours, bbox, area });
        }
        d.shapes.get(&g)
    }
}

/// a shaped run: per glyph (font, glyph, advance em, offset em, cluster)
struct Shaped {
    font: FontId,
    glyph: u32,
    advance: f64,
    off: [f64; 2],
    cluster: u32,
}

fn shape_str(fonts: &Fonts, st: &TextStyle, s: &str) -> Result<Vec<Shaped>> {
    let ids: Vec<FontId> = core::iter::once(st.font).chain(st.fallback[..st.fallbacks as usize].iter().copied()).collect();
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let primary = &fonts.fonts[st.font.0 as usize].shaper;
    let fbs: Vec<&text_shape::Engine> = ids[1..].iter().map(|f| &fonts.fonts[f.0 as usize].shaper).collect();
    let g = primary.shape_with_fallback(s, &fbs).ok_or(GpuError::Invalid("Text: the shaper refused this text"))?;
    let mut out = Vec::with_capacity(g.len());
    for x in g {
        let font = ids.get(x.font).copied().unwrap_or(st.font);
        let up = fonts.fonts[font.0 as usize].upem;
        out.push(Shaped { font, glyph: x.id, advance: x.advance as f64 / up, off: [x.x as f64 / up, x.y as f64 / up], cluster: x.cluster as u32 });
    }
    Ok(out)
}

fn clean(s: &str) -> alloc::string::String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// one line placed: its glyphs from pen 0 on, its advance width
fn place(fonts: &mut Fonts, st: &TextStyle, s: &str, cluster0: u32, out: &mut Vec<Placed>) -> Result<f64> {
    let sh = shape_str(fonts, st, s)?;
    let mut x = 0.0f64;
    for g in &sh {
        let c = s.get(g.cluster as usize..).and_then(|r| r.chars().next()).unwrap_or(' ');
        let empty = fonts.shape(g.font, g.glyph).is_none_or(|sh| sh.bbox.is_none());
        out.push(Placed {
            font: g.font,
            glyph: g.glyph,
            pen: [x + g.off[0], g.off[1]],
            advance: g.advance,
            cluster: cluster0 + g.cluster,
            mark: g.advance == 0.0 && !empty,
            blank: c.is_whitespace() || empty,
        });
        x += g.advance;
    }
    Ok(x)
}

/// the bars of a line's glyphs (words: runs of glyphs with ink between blanks), x-height xh
fn bars_of(fonts: &mut Fonts, glyphs: &[Placed], xh: f64, out: &mut Vec<Bar>) {
    let mut cur: Option<Bar> = None;
    let mut area = 0.0;
    let flush = |cur: &mut Option<Bar>, area: &mut f64, out: &mut Vec<Bar>| {
        if let Some(mut b) = cur.take() {
            let box_area = (b.x1 - b.x0) * (b.y1 - b.y0);
            b.ink = if box_area > 0.0 { (*area / box_area).clamp(0.05, 1.0) } else { 1.0 };
            out.push(b);
        }
        *area = 0.0;
    };
    for g in glyphs {
        if g.blank {
            flush(&mut cur, &mut area, out);
            continue;
        }
        let Some(sh) = fonts.shape(g.font, g.glyph) else { continue };
        let Some(bb) = sh.bbox else { continue };
        let (x0, x1) = (g.pen[0] + bb[0], g.pen[0] + bb[2]);
        area += sh.area;
        cur = Some(match cur {
            Some(b) => Bar { x0: b.x0.min(x0), x1: b.x1.max(x1), ..b },
            None => Bar { x0, x1, y0: 0.0, y1: xh, ink: 1.0 },
        });
    }
    flush(&mut cur, &mut area, out);
}

fn ink_of(fonts: &mut Fonts, lay: &Layout) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for l in &lay.lines {
        for g in &lay.glyphs[l.glyphs.start as usize..l.glyphs.end as usize] {
            let Some(bb) = fonts.shape(g.font, g.glyph).and_then(|s| s.bbox) else { continue };
            let o = [l.origin[0] + g.pen[0], l.origin[1] + g.pen[1]];
            let q = [o[0] + bb[0], o[1] + bb[1], o[0] + bb[2], o[1] + bb[3]];
            b = Some(match b {
                Some(a) => [a[0].min(q[0]), a[1].min(q[1]), a[2].max(q[2]), a[3].max(q[3])],
                None => q,
            });
        }
    }
    b
}

/// the frame's map: em -> drawing (or pixels): s R(rotation) diag(back, upside) [[w, tan(oblique)], [0, 1]], as
/// [m00, m01, m10, m11]
fn frame(s: f64, dir: [f64; 2], w: f64, oblique: f64, backward: bool, upside_down: bool) -> [f64; 4] {
    let (so, co) = sincos(oblique);
    let t = so / co;
    let (bx, by) = (if backward { -1.0 } else { 1.0 }, if upside_down { -1.0 } else { 1.0 });
    // F = [[bx w, bx t], [0, by]]
    let f = [bx * w, bx * t, 0.0, by];
    let (c, sn) = (dir[0], dir[1]);
    [s * (c * f[0] - sn * f[2]), s * (c * f[1] - sn * f[3]), s * (sn * f[0] + c * f[2]), s * (sn * f[1] + c * f[3])]
}

fn height_of(st: &TextStyle, h: Option<f64>) -> Result<f64> {
    let v = if st.height > 0.0 { st.height } else { h.unwrap_or(0.0) };
    if !(v.is_finite() && v > 0.0 && v < 1e30) {
        return Err(GpuError::Invalid("Text: a cap height above 0 (the text's, or a fixed one of its style)"));
    }
    Ok(v)
}

/// a single line laid out (DXF TEXT)
pub fn layout_text(fonts: &mut Fonts, styles: &[TextStyle], t: &Text) -> Result<Layout> {
    let st = *styles.get(t.style.0 as usize).ok_or(GpuError::Invalid("Text: a text style the scene does not have"))?;
    let fine = |p: [f64; 2]| p.iter().all(|v| v.is_finite() && v.abs() < 1e30);
    if !(fine(t.at) && fine(t.at2) && t.rotation.is_finite() && t.rotation.abs() < 1e15) {
        return Err(GpuError::Invalid("Text: finite points below 1e30 and a finite rotation"));
    }
    let mut w = t.width.unwrap_or(st.width);
    let obl = t.oblique.unwrap_or(st.oblique);
    if !(w.is_finite() && w > 0.0 && w < 1e30 && obl.is_finite() && obl.abs() < 1.4) {
        return Err(GpuError::Invalid("Text: a width factor above 0 and an oblique angle within 80 degrees"));
    }
    let h = height_of(&st, t.height)?;
    let fm = fonts.metrics(st.font).ok_or(GpuError::Invalid("Text: a font the scene does not have"))?;
    let mut lay = Layout { px: st.unit == Unit::Px, x_height: fm.x_height, ..Layout::default() };
    let s = clean(t.text);
    let width = place(fonts, &st, &s, 0, &mut lay.glyphs)?;
    let mut bars = Vec::new();
    bars_of(fonts, &lay.glyphs, fm.x_height, &mut bars);
    lay.lines.push(Line { origin: [0.0, 0.0], glyphs: 0..lay.glyphs.len() as u32, bars: 0..bars.len() as u32, width });
    lay.bars = bars;
    lay.ink = ink_of(fonts, &lay);
    lay.cell = [0.0, fm.descender, width, fm.cap_height];
    let mut em = h / fm.cap_height;
    let mut dir = { let (sn, c) = sincos(t.rotation); [c, sn] };
    // the alignment point in em (before the frame's width factor, oblique angle and mirrors)
    let mut ax = match t.halign {
        HAlign::Left | HAlign::Aligned | HAlign::Fit => 0.0,
        HAlign::Center | HAlign::Middle => 0.5 * width,
        HAlign::Right => width,
    };
    let mut ay = match t.valign {
        VAlign::Baseline => 0.0,
        VAlign::Bottom => fm.descender,
        VAlign::Middle => 0.5 * fm.cap_height,
        VAlign::Top => fm.cap_height,
    };
    if t.halign == HAlign::Middle {
        ay = lay.ink.map_or(0.5 * fm.cap_height, |b| 0.5 * (b[1] + b[3]));
    }
    if matches!(t.halign, HAlign::Aligned | HAlign::Fit) {
        let d = [t.at2[0] - t.at[0], t.at2[1] - t.at[1]];
        let l = sqrt(d[0] * d[0] + d[1] * d[1]);
        if l > 0.0 && width > 0.0 {
            dir = [d[0] / l, d[1] / l];
            if t.halign == HAlign::Aligned {
                em = l / (width * w);
            } else {
                w = l / (width * em);
            }
        }
        ax = 0.0;
        ay = 0.0;
    }
    lay.em = em;
    lay.m = frame(em, dir, w, obl, t.backward, t.upside_down);
    let m = &lay.m;
    lay.origin = [t.at[0] - (m[0] * ax + m[1] * ay), t.at[1] - (m[2] * ax + m[3] * ay)];
    Ok(lay)
}

/// text of several lines laid out (the basics of DXF MTEXT)
pub fn layout_mtext(fonts: &mut Fonts, styles: &[TextStyle], t: &MText) -> Result<Layout> {
    let st = *styles.get(t.style.0 as usize).ok_or(GpuError::Invalid("MText: a text style the scene does not have"))?;
    let fine = |v: f64| v.is_finite() && v.abs() < 1e30;
    if !(fine(t.at[0]) && fine(t.at[1]) && t.rotation.is_finite() && t.rotation.abs() < 1e15 && fine(t.width) && t.width >= 0.0 && fine(t.line_spacing) && t.line_spacing > 0.0) {
        return Err(GpuError::Invalid("MText: a finite point and rotation, a width not negative, a line spacing above 0"));
    }
    let h = height_of(&st, t.height)?;
    let fm = fonts.metrics(st.font).ok_or(GpuError::Invalid("MText: a font the scene does not have"))?;
    let em = h / fm.cap_height;
    let wrap = if t.width > 0.0 { t.width / (em * st.width) } else { f64::INFINITY };
    let step = LINE_SPACING * fm.cap_height * t.line_spacing;
    let mut lay = Layout { px: st.unit == Unit::Px, x_height: fm.x_height, em, ..Layout::default() };
    let mut start = 0usize;
    let text = t.text;
    let mut k = 0u32;
    let mut paras = Vec::new();
    for (i, c) in text.char_indices() {
        if c == '\n' {
            paras.push(start..i);
            start = i + 1;
        }
    }
    paras.push(start..text.len());
    for pr in paras {
        let para = &text[pr.clone()];
        for lr in wrap_lines(fonts, &st, para, wrap)? {
            let s = clean(&para[lr.clone()]);
            let g0 = lay.glyphs.len() as u32;
            let width = place(fonts, &st, &s, (pr.start + lr.start) as u32, &mut lay.glyphs)?;
            let b0 = lay.bars.len() as u32;
            let mut bars = Vec::new();
            bars_of(fonts, &lay.glyphs[g0 as usize..], fm.x_height, &mut bars);
            lay.bars.extend_from_slice(&bars);
            let x = match t.attach {
                Attach::TopLeft | Attach::MiddleLeft | Attach::BottomLeft => 0.0,
                Attach::TopCenter | Attach::MiddleCenter | Attach::BottomCenter => -0.5 * width,
                _ => -width,
            };
            lay.lines.push(Line { origin: [x, -(k as f64) * step], glyphs: g0..lay.glyphs.len() as u32, bars: b0..lay.bars.len() as u32, width });
            k += 1;
        }
    }
    let n = k.max(1) as f64;
    let top = fm.cap_height;
    let bottom = -(n - 1.0) * step + fm.descender;
    let widest = lay.lines.iter().map(|l| l.width).fold(0.0f64, f64::max);
    let x0 = lay.lines.iter().map(|l| l.origin[0]).fold(0.0f64, f64::min);
    lay.cell = [x0, bottom, x0 + widest.max(0.0), top];
    lay.ink = ink_of(fonts, &lay);
    let ay = match t.attach {
        Attach::TopLeft | Attach::TopCenter | Attach::TopRight => top,
        Attach::MiddleLeft | Attach::MiddleCenter | Attach::MiddleRight => 0.5 * (top + bottom),
        _ => bottom,
    };
    let (sn, c) = sincos(t.rotation);
    lay.m = frame(em, [c, sn], st.width, st.oblique, false, false);
    let m = &lay.m;
    lay.origin = [t.at[0] - m[1] * ay, t.at[1] - m[3] * ay];
    Ok(lay)
}

/// a paragraph's lines (byte ranges) wrapped at `wrap` em: greedy at the spaces in logical order, from the advances of
/// the paragraph shaped once; a word longer than a line stands alone
fn wrap_lines(fonts: &mut Fonts, st: &TextStyle, para: &str, wrap: f64) -> Result<Vec<core::ops::Range<usize>>> {
    if !wrap.is_finite() || para.is_empty() {
        return Ok(alloc::vec![0..para.len()]);
    }
    let sh = shape_str(fonts, st, para)?;
    // the advance at each byte of the logical text (a cluster's glyphs at its first byte)
    let mut adv = alloc::vec![0.0f64; para.len() + 1];
    for g in &sh {
        if let Some(a) = adv.get_mut(g.cluster as usize) {
            *a += g.advance;
        }
    }
    let mut lines = Vec::new();
    let (mut line_start, mut line_w) = (0usize, 0.0f64);
    let mut i = 0usize;
    let bytes: Vec<(usize, char)> = para.char_indices().collect();
    while i < bytes.len() {
        // a word and the spaces after it
        let ws = bytes[i].0;
        let mut j = i;
        let mut word_w = 0.0;
        while j < bytes.len() && !bytes[j].1.is_whitespace() {
            word_w += adv[bytes[j].0];
            j += 1;
        }
        let mut sp_w = 0.0;
        while j < bytes.len() && bytes[j].1.is_whitespace() {
            sp_w += adv[bytes[j].0];
            j += 1;
        }
        if line_w > 0.0 && line_w + word_w > wrap {
            lines.push(line_start..ws);
            line_start = ws;
            line_w = 0.0;
        }
        line_w += word_w + sp_w;
        i = j;
    }
    lines.push(line_start..para.len());
    // (the spaces a line ends in are left out of it)
    for r in &mut lines {
        let s = &para[r.clone()];
        r.end = r.start + s.trim_end().len();
    }
    Ok(lines)
}
