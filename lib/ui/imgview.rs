// imgview -- SVG pictures and the image viewer (ROADMAP §6)
//
//   ui.svg(id, &doc, w, h) -> EV        an img_svg::Svg drawn sharp at its layout size: w / h in
//        DIP (0 and 0: the intrinsic size, one 0: from the aspect ratio); currentColor is the
//        text colour (icons follow the theme); clickable when id != 0; K_IMAGE record with
//        the document's <title>.  Text in the SVG is set with the UI's fonts (SvgText).
//   ui.image_view(id, &mut st, img, flags) -> EV    a zoomable, scrollable view of a bitmap
//        (ViewImage::Bitmap(&Rc<Surface>, version)) or an SVG (ViewImage::Svg(&Rc<Svg>)),
//        filling its parent unless next_size says otherwise.  EV_ACT when the zoom or the
//        position changed (st.zoom, st.cx / cy, st.fit), EV_HOVER, EV_FOCUS, EV_DOUBLE.
//        Mouse: the wheel zooms at the pointer (IV_WHEEL_SCROLL: the wheel scrolls, Ctrl +
//        wheel zooms; Shift + wheel scrolls sideways), a left or middle drag pans, Ctrl + drag
//        draws a rectangle to zoom into, a double click switches between 100 % (at the
//        pointer) and fit.  Keys (focused): arrows pan an eighth of the view, PgUp / PgDn a
//        page, Home / End to the top / bottom; + / - (with or without Ctrl) zoom in steps,
//        Ctrl+0 fits, Ctrl+1 shows 100 %.  Scroll bars at the right and bottom edges when the
//        image is larger than the view (IV_NO_BARS: none).
//        Drawing: bitmaps through mip levels (2 x 2 box) and bilinear sampling below 400 %,
//        nearest neighbour above (IV_SMOOTH: bilinear at every zoom), a pixel grid from 800 %
//        (IV_NO_GRID: none); SVGs are drawn again at the zoom (only the visible part,
//        currentColor black, IV_TEXT_COLOR: the text colour).  Transparent parts over a
//        checkerboard (IV_NO_CHECKER: the view's background); the theme decides its colours
//        (light, dark, high contrast: plain background).
//   ImageViewState { zoom, cx, cy, fit, label }: zoom 1.0 = 100 % (a bitmap pixel on one
//        screen pixel, an SVG at its intrinsic size in DIP); (cx, cy) the image point at the
//        view's centre; fit: the zoom follows the view.  st.set_fit() / set_actual() /
//        zoom_to(rect) / zoom_by(factor) / zoom_pct().  A new image (another Rc) fits.
//   SvgText { text, fonts }: img_svg::TextHook on ui's text engine; SvgFonts picks the faces:
//        families registered with ui.font_register, "monospace" = ui.mono_font_set's font,
//        everything else the style's fonts (regular / semibold / bold; italic slanted).
//
// Ids: id (the view: focus, K_IMAGE with st.label, the SVG's title or "Bild"; value = zoom %),
// part(id, 1) / part(id, 2) the vertical / horizontal scroll bar.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::path::Path;
use gfx::xform::Matrix;
use gfx::{Surface, FMT_ARGB, FMT_PARGB};
use img_core::mip::MipChain;
use img_core::resample::{RS_BILINEAR, RS_NEAREST};
use img_core::xform::{affine, XF_TRANSPARENT};
use img_svg::{FontSpec, Opts, Svg, TextHook};
use text::{Gp, Text, FONT_BOLD, FONT_REG, FONT_SEMI};

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::i18n::tr;
use crate::keys::*;
use crate::Ui;

/// image_view flags
pub const IV_WHEEL_SCROLL: u32 = 1; // the wheel scrolls, Ctrl + wheel zooms
pub const IV_NO_GRID: u32 = 2; // no pixel grid at high zoom
pub const IV_SMOOTH: u32 = 4; // bilinear at every zoom (no visible pixels)
pub const IV_NO_BARS: u32 = 8; // no scroll bars
pub const IV_NO_CHECKER: u32 = 16; // transparency over the view background
pub const IV_TEXT_COLOR: u32 = 32; // an SVG's currentColor is the text colour (else black)

const ZOOM_MIN: f64 = 0.01;
const ZOOM_MAX: f64 = 64.0;
// the steps of + / - (and of the zoom box of apps)
const STEPS: [f64; 26] = [
    0.01, 0.02, 0.03, 0.05, 0.08, 0.12, 0.17, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0,
    32.0, 48.0, 64.0,
];
const NEAREST_FROM: f64 = 4.0; // device px per image px: pixels become visible
const GRID_FROM: f64 = 8.0;
const SVG_CACHE: usize = 64; // Draw objects of ui.svg kept

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

// FNV-1a over u64 words (the draw objects' identity)
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xCBF2_9CE4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

// ------------------------------------------------------------------ fonts for SVG text

/// the faces SvgText may use (taken from the Ui at frame time)
#[derive(Clone, Default, Debug)]
pub struct SvgFonts {
    /// (family lower case, weight, italic, font id) of ui.font_register
    pub faces: Vec<(String, u32, bool, u32)>,
    /// "monospace"
    pub mono: Option<u32>,
}

/// an img_svg::TextHook on a text engine
pub struct SvgText<'a> {
    pub text: &'a mut Text,
    pub fonts: &'a SvgFonts,
}

impl SvgText<'_> {
    // (font id, slant synthetically)
    fn pick(&self, f: &FontSpec) -> (u32, bool) {
        for fam in f.family.split(',') {
            let fam = fam.trim().trim_matches(|c| c == '"' || c == '\'').to_lowercase();
            if fam == "monospace" {
                if let Some(m) = self.fonts.mono.filter(|&m| self.text.valid(m)) {
                    return (m, f.italic);
                }
            }
            let best = self
                .fonts
                .faces
                .iter()
                .filter(|x| x.0 == fam && self.text.valid(x.3))
                .min_by_key(|x| (x.2 != f.italic) as u32 * 10000 + (x.1 as i32 - f.weight as i32).unsigned_abs());
            if let Some(b) = best {
                return (b.3, f.italic && !b.2);
            }
        }
        let font = if f.weight >= 600 {
            FONT_BOLD
        } else if f.weight >= 500 {
            FONT_SEMI
        } else {
            FONT_REG
        };
        (font, f.italic)
    }

    // advance (px) of s; with a path: the outlines appended (baseline at x, y)
    fn run(&mut self, s: &str, f: &FontSpec, px: f32, mut out: Option<(&mut Path, f32, f32)>) -> f32 {
        let (font, slant) = self.pick(f);
        let t = &mut *self.text;
        let n0 = out.as_mut().map_or(0, |o| o.0.pts_mut().len());
        let mut pen = 0.0f32;
        let mut prev: Option<(u32, u32)> = None;
        for ch in s.chars() {
            let cp = ch as u32;
            let mut ff = font;
            if !t.has(font, cp) {
                if let Some(alt) = t.fallback_for(cp) {
                    ff = alt;
                }
            }
            let g = t.glyph(ff, cp);
            let upem = t.upem(ff).max(1) as f32;
            let sx = px / upem;
            if let Some((pf, pg)) = prev {
                if pf == ff {
                    pen += t.kern(ff, pg, g) as f32 * sx;
                }
            }
            if let Some((path, x, y)) = out.as_mut() {
                t.gp = Gp { x: *x + pen, y: *y, sx, sy: sx, dx: 0, dy: 0 };
                core::mem::swap(&mut t.path, *path);
                t.glyph_path(ff, g);
                core::mem::swap(&mut t.path, *path);
            }
            pen += t.adv(ff, g) as f32 * sx;
            prev = Some((ff, g));
        }
        // a synthetic slant (12 degrees) about the baseline
        if slant {
            if let Some((path, _, y)) = out {
                for p in path.pts_mut()[n0..].iter_mut() {
                    p.0 += (y - p.1) * 0.21;
                }
            }
        }
        pen
    }
}

impl TextHook for SvgText<'_> {
    fn outline(&mut self, path: &mut Path, s: &str, font: &FontSpec, px: f32, x: f32, y: f32) -> f32 {
        self.run(s, font, px, Some((path, x, y)))
    }
    fn advance(&mut self, s: &str, font: &FontSpec, px: f32) -> f32 {
        self.run(s, font, px, None)
    }
}

// ------------------------------------------------------------------ ui.svg

/// the shared state: the Draw objects of ui.svg by (document, size, colour)
#[derive(Default)]
pub(crate) struct S {
    svgs: Vec<(u64, Rc<SvgDraw>)>,
    fonts: Option<Rc<SvgFonts>>,
}

/// an SVG drawn at render time into its rectangle (the rectangle is the viewport)
pub(crate) struct SvgDraw {
    pub(crate) doc: Rc<Svg>,
    pub(crate) color: u32,
    pub(crate) fonts: Rc<SvgFonts>,
}

impl Draw for SvgDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let (x, y) = (r[0] + g.dc.ox, r[1] + g.dc.oy);
        let clip = g.dc.clip;
        let mut th = SvgText { text: &mut *g.text, fonts: &self.fonts };
        let mut o = Opts { color: self.color, text: Some(&mut th) };
        self.doc.render_rect(g.sf, x as f64, y as f64, r[2] as f64, r[3] as f64, Some(clip), &mut o);
    }
}

impl Ui {
    /// the faces for SVG text (registered families, the monospace font); shared until the
    /// registered fonts change
    pub fn svg_fonts(&mut self) -> Rc<SvgFonts> {
        let mut faces: Vec<(String, u32, bool, u32)> = Vec::new();
        for fam in self.font_registered() {
            for w in [100, 200, 300, 400, 500, 600, 700, 800, 900] {
                for it in [false, true] {
                    if let Some(id) = self.font_registered_face(&fam, w, it) {
                        faces.push((fam.to_lowercase(), w, it, id));
                    }
                }
            }
        }
        let mono = self.m.term.mono;
        if let Some(f) = &self.m.svg.fonts {
            if f.faces == faces && f.mono == mono {
                return f.clone();
            }
        }
        let f = Rc::new(SvgFonts { faces, mono });
        self.m.svg.fonts = Some(f.clone());
        f
    }

    /// ui_svg(id, doc, w, h) -> EV: an SVG at w x h DIP (see the module header of imgview.rs)
    pub fn svg(&mut self, id: u32, doc: &Rc<Svg>, w: i32, h: i32) -> u32 {
        let (iw, ih) = doc.size();
        let (w, h) = match (w > 0, h > 0) {
            (true, true) => (w as f64, h as f64),
            (true, false) => (w as f64, w as f64 * ih / iw),
            (false, true) => (h as f64 * iw / ih, h as f64),
            (false, false) => (iw, ih),
        };
        let pw = self.dp((w + 0.5).clamp(1.0, 1.0e5) as i32);
        let ph = self.dp((h + 0.5).clamp(1.0, 1.0e5) as i32);
        let r = self.place(pw, ph);
        let disabled = self.wp.ctx.disabled_depth != 0;
        let color = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
        let fonts = self.svg_fonts();
        let mut hh = Fnv::new();
        hh.u64(Rc::as_ptr(doc) as usize as u64);
        hh.u64(color as u64);
        hh.u64(Rc::as_ptr(&fonts) as usize as u64);
        let key = hh.0;
        let s = &mut self.m.svg;
        // forget the objects of documents nobody else holds
        s.svgs.retain(|(_, d)| Rc::strong_count(&d.doc) > 1);
        let d = match s.svgs.iter().find(|(k, _)| *k == key) {
            Some((_, d)) => d.clone(),
            None => {
                if s.svgs.len() >= SVG_CACHE {
                    s.svgs.remove(0);
                }
                let d = Rc::new(SvgDraw { doc: doc.clone(), color, fonts });
                s.svgs.push((key, d.clone()));
                d
            }
        };
        let obj: DrawRef = d;
        self.ltr_begin(r[0], r[2]);
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(obj), 0, key as u32);
        self.ltr_end();
        let ev = if id != 0 { self.behave(id, 0) } else { 0 };
        let title = doc.title();
        let label = if title.is_empty() { None } else { Some(title) };
        if id != 0 || label.is_some() {
            self.sem(K_IMAGE, id, label, 0);
        }
        ev
    }
}

// ------------------------------------------------------------------ the image viewer

/// what ui.image_view shows
#[derive(Clone, Copy)]
pub enum ViewImage<'a> {
    /// a bitmap (FMT_PARGB, FMT_ARGB or FMT_XRGB) and its version (bump it when the
    /// pixels change)
    Bitmap(&'a Rc<Surface>, u32),
    Svg(&'a Rc<Svg>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Gesture {
    #[default]
    Idle,
    /// panning since the pointer was at (mx, my) and the centre at (cx, cy)
    Pan { mx: i32, my: i32, cx: f64, cy: f64, middle: bool },
    /// a zoom rectangle from (x, y) px
    Rect { x: i32, y: i32 },
}

/// the app-owned state of an image view (see the module header of imgview.rs)
#[derive(Clone)]
pub struct ImageViewState {
    /// 1.0 = 100 %
    pub zoom: f64,
    /// the image point (image px) at the centre of the view
    pub cx: f64,
    pub cy: f64,
    /// the zoom follows the view: the whole image, centred
    pub fit: bool,
    /// the accessible name (empty: the SVG's title, else "Bild")
    pub label: String,
    key: usize,
    ver: u32,
    // a PARGB copy of an XRGB bitmap and the mip levels
    conv: Option<Rc<Surface>>,
    mips: Option<Rc<MipChain>>,
    g: Gesture,
    draw: Option<(u64, DrawRef)>,
    // the last frame: view size (px), device px per image px at zoom 1
    view: [i32; 2],
    unit: f64,
}

impl Default for ImageViewState {
    fn default() -> ImageViewState {
        ImageViewState {
            zoom: 1.0,
            cx: 0.0,
            cy: 0.0,
            fit: true,
            label: String::new(),
            key: 0,
            ver: 0,
            conv: None,
            mips: None,
            g: Gesture::Idle,
            draw: None,
            view: [0, 0],
            unit: 1.0,
        }
    }
}

impl core::fmt::Debug for ImageViewState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageViewState").field("zoom", &self.zoom).field("cx", &self.cx).field("cy", &self.cy).field("fit", &self.fit).finish()
    }
}

impl ImageViewState {
    /// the zoom in percent (rounded)
    pub fn zoom_pct(&self) -> i32 {
        (self.zoom * 100.0 + 0.5) as i32
    }

    /// fit mode: the whole image in the view
    pub fn set_fit(&mut self) {
        self.fit = true;
    }

    /// 100 %, the centre kept
    pub fn set_actual(&mut self) {
        self.fit = false;
        self.zoom = 1.0;
    }

    /// multiply the zoom (about the view's centre)
    pub fn zoom_by(&mut self, f: f64) {
        if f > 0.0 {
            self.fit = false;
            self.zoom = (self.zoom * f).clamp(ZOOM_MIN, ZOOM_MAX);
        }
    }

    /// show the image rectangle x, y, w, h (image px) as large as the view allows
    pub fn zoom_to(&mut self, r: [f64; 4]) {
        let (vw, vh) = (self.view[0] as f64, self.view[1] as f64);
        if !(r[2] > 0.0 && r[3] > 0.0) || vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let u = if self.unit > 0.0 { self.unit } else { 1.0 };
        self.fit = false;
        self.zoom = (vw / (r[2] * u)).min(vh / (r[3] * u)).clamp(ZOOM_MIN, ZOOM_MAX);
        self.cx = r[0] + r[2] * 0.5;
        self.cy = r[1] + r[3] * 0.5;
    }

    // the next step of the zoom list in direction dir (+1 / -1)
    fn step(&mut self, dir: i32) {
        let z = self.zoom;
        let next = if dir > 0 { STEPS.iter().copied().find(|&s| s > z * 1.001) } else { STEPS.iter().rev().copied().find(|&s| s < z * 0.999) };
        if let Some(n) = next {
            self.fit = false;
            self.zoom = n;
        }
    }

    // zoom to z keeping the image point under the view point (px, py) relative to the view's
    // centre in place
    fn zoom_at(&mut self, z: f64, dx: f64, dy: f64) {
        let s0 = self.zoom * self.unit;
        let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
        let s1 = z * self.unit;
        if !(s0 > 0.0 && s1 > 0.0) {
            return;
        }
        let (px, py) = (self.cx + dx / s0, self.cy + dy / s0);
        self.fit = false;
        self.zoom = z;
        self.cx = px - dx / s1;
        self.cy = py - dy / s1;
    }
}

/// the view's drawing (render time)
struct IvDraw {
    img: IvImg,
    mips: Option<Rc<MipChain>>,
    s: f64,
    ox: f64,
    oy: f64,
    iw: f64,
    ih: f64,
    flags: u32,
    bg: u32,
    checker: Option<(i32, u32, u32)>,
    grid: u32,
    color: u32,
    fonts: Rc<SvgFonts>,
}

enum IvImg {
    Bitmap(Rc<Surface>),
    Svg(Rc<Svg>),
}

impl IvDraw {
    // the bitmap through the mips and the sampler into the rows y0 .. y1 of x0 .. x1
    fn bitmap(&self, g: &mut Gfx, src0: &Surface, a: [i32; 4]) {
        let mut src = src0;
        let mut s = self.s;
        let mut level = 1;
        if let Some(m) = &self.mips {
            while s < 0.5 {
                match m.level(level) {
                    Some(l) => {
                        src = l;
                        s *= 2.0;
                        level += 1;
                    }
                    None => break,
                }
            }
        }
        let sx = src.w as f64 / src0.w.max(1) as f64; // level px per image px
        let sy = src.h as f64 / src0.h.max(1) as f64;
        let filter = if self.s >= NEAREST_FROM && self.flags & IV_SMOOTH == 0 { RS_NEAREST } else { RS_BILINEAR };
        let w = a[2] - a[0];
        const BAND: i32 = 64;
        let mut tile = Surface::new(w, BAND, FMT_PARGB);
        if tile.px.is_empty() {
            return;
        }
        let stride = g.sf.stride as usize;
        let mut y = a[1];
        while y < a[3] {
            let n = (a[3] - y).min(BAND);
            // tile (tx, ty) = device (tx + a[0], ty + y) -> level px
            let m = Matrix { a: sx / self.s, b: 0.0, c: (a[0] as f64 - self.ox) * sx / self.s, d: 0.0, e: sy / self.s, f: (y as f64 - self.oy) * sy / self.s };
            if affine(src, &mut tile, &m, filter, XF_TRANSPARENT, Some([0, 0, w, n])) != 0 {
                return;
            }
            for r in 0..n {
                let to = r as usize * tile.stride as usize;
                let trow = &tile.px[to..to + w as usize * 4];
                let d = (y + r) as usize * stride + a[0] as usize * 4;
                let Some(drow) = g.sf.px.get_mut(d..d + w as usize * 4) else { continue };
                match self.checker {
                    Some((cell, c0, c1)) => {
                        let cfg = gfx::straight::ViewCfg { cell, c0, c1 };
                        gfx::straight::view_row_pm(drow, trow, a[0] - self.ox as i32, y + r - self.oy as i32, &cfg);
                    }
                    None => gfx::blend::blit_over(drow, trow, 255),
                }
            }
            y += n;
        }
    }

    // the checkerboard under an SVG
    fn checker_fill(&self, g: &mut Gfx, a: [i32; 4]) {
        let Some((cell, c0, c1)) = self.checker else { return };
        let cfg = gfx::straight::ViewCfg { cell, c0, c1 };
        let stride = g.sf.stride as usize;
        for y in a[1]..a[3] {
            let d = y as usize * stride + a[0] as usize * 4;
            let Some(row) = g.sf.px.get_mut(d..d + (a[2] - a[0]) as usize * 4) else { continue };
            for (i, px) in row.chunks_exact_mut(4).enumerate() {
                let c = gfx::straight::view_bg(a[0] + i as i32 - self.ox as i32, y - self.oy as i32, &cfg);
                px.copy_from_slice(&c.to_le_bytes());
            }
        }
    }

    // pixel boundaries at high zoom
    fn grid(&self, g: &mut Gfx, a: [i32; 4]) {
        let (dx, dy) = (g.dc.ox, g.dc.oy);
        g.dc.set_color(self.grid);
        let i0 = ((a[0] as f64 - self.ox) / self.s).max(0.0) as i64;
        let i1 = (((a[2] as f64 - self.ox) / self.s) as i64 + 1).min(self.iw as i64);
        for i in i0..=i1 {
            let x = (self.ox + i as f64 * self.s).rnd() as i32;
            if x >= a[0] && x < a[2] {
                g.dc.fill(g.sf, x - dx, a[1] - dy, 1, a[3] - a[1]);
            }
        }
        let j0 = ((a[1] as f64 - self.oy) / self.s).max(0.0) as i64;
        let j1 = (((a[3] as f64 - self.oy) / self.s) as i64 + 1).min(self.ih as i64);
        for j in j0..=j1 {
            let y = (self.oy + j as f64 * self.s).rnd() as i32;
            if y >= a[1] && y < a[3] {
                g.dc.fill(g.sf, a[0] - dx, y - dy, a[2] - a[0], 1);
            }
        }
    }
}

impl Draw for IvDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let (dx, dy) = (g.dc.ox, g.dc.oy);
        let clip = g.dc.clip;
        let view = [r[0] + dx, r[1] + dy, r[0] + dx + r[2], r[1] + dy + r[3]];
        let k = [clip[0].max(view[0]), clip[1].max(view[1]), clip[2].min(view[2]), clip[3].min(view[3])];
        if k[2] <= k[0] || k[3] <= k[1] {
            return;
        }
        // the view's background
        g.dc.set_color(self.bg);
        g.dc.fill(g.sf, k[0] - dx, k[1] - dy, k[2] - k[0], k[3] - k[1]);
        // the image's pixels inside the clip
        let ir = [
            self.ox.max(-1.0e9) as i64,
            self.oy.max(-1.0e9) as i64,
            (self.ox + self.iw * self.s).min(1.0e9).ceil_i64(),
            (self.oy + self.ih * self.s).min(1.0e9).ceil_i64(),
        ];
        let a = [
            (ir[0].max(k[0] as i64)) as i32,
            (ir[1].max(k[1] as i64)) as i32,
            (ir[2].min(k[2] as i64)) as i32,
            (ir[3].min(k[3] as i64)) as i32,
        ];
        if a[2] <= a[0] || a[3] <= a[1] {
            return;
        }
        match &self.img {
            IvImg::Bitmap(sf) => {
                self.bitmap(g, sf, a);
                if self.s >= GRID_FROM && self.flags & IV_NO_GRID == 0 {
                    self.grid(g, a);
                }
            }
            IvImg::Svg(doc) => {
                self.checker_fill(g, a);
                let mut th = SvgText { text: &mut *g.text, fonts: &self.fonts };
                let mut o = Opts { color: self.color, text: Some(&mut th) };
                let m = Matrix { a: self.s, b: 0.0, c: self.ox, d: 0.0, e: self.s, f: self.oy };
                doc.render(g.sf, &m, Some(a), &mut o);
            }
        }
    }
}

// rounding without libm
trait Round {
    fn ceil_i64(self) -> i64;
    fn rnd(self) -> f64;
}

impl Round for f64 {
    fn ceil_i64(self) -> i64 {
        let t = self as i64;
        if (t as f64) < self { t + 1 } else { t }
    }
    // to the nearest integer (halves up); beyond 2^52 unchanged
    fn rnd(self) -> f64 {
        if !(self.abs() < 4.0e15) {
            return self;
        }
        let v = self + 0.5;
        let t = v as i64 as f64;
        if t > v { t - 1.0 } else { t }
    }
}

impl Ui {
    // an unconsumed key vk whose modifiers & mask == need: consumed (arrows as drawn: the
    // view is not mirrored)
    fn iv_key(&mut self, vk: u32, need: u32, mask: u32) -> bool {
        let rtl = self.rtl;
        for k in self.wp.ctx.keyq.iter_mut() {
            if (*k as i32) < 0 {
                continue;
            }
            let mods = (*k >> 16) & 0x7FFF;
            let mut v = *k & 0xFFFF;
            if rtl && mods & (MOD_CTRL | MOD_ALT) == 0 {
                v = match v {
                    VK_LEFT => VK_RIGHT,
                    VK_RIGHT => VK_LEFT,
                    v => v,
                };
            }
            if v == vk && mods & mask == need {
                *k |= 0x8000_0000;
                return true;
            }
        }
        false
    }

    /// ui_image_view(id, st, img, flags) -> EV: the image viewer (see the module header of
    /// imgview.rs)
    pub fn image_view(&mut self, id: u32, st: &mut ImageViewState, img: ViewImage, flags: u32) -> u32 {
        let _ = self.take_flags();
        if self.wp.lay.next_w == 0 && self.wp.lay.next_h == 0 && self.wp.lay.next_rect_set == 0 {
            self.next_size(-1, -1);
        }
        let r = self.place(0, 0);
        let dpi = self.wp.rt.dpi;
        // the image: its size, the device px per image px at 100 %, a new one fits
        let (key, ver, iw, ih, unit) = match img {
            ViewImage::Bitmap(sf, v) => (Rc::as_ptr(sf) as usize, v, sf.w.max(0) as f64, sf.h.max(0) as f64, 1.0),
            ViewImage::Svg(d) => {
                let (w, h) = d.size();
                (Rc::as_ptr(d) as usize, 0, w, h, dpi as f64 / 96.0)
            }
        };
        if key != st.key {
            if st.key != 0 {
                st.fit = true;
            }
            st.key = key;
            st.ver = ver.wrapping_add(1); // the bitmap's derived data below is built again
            st.g = Gesture::Idle;
        }
        if let ViewImage::Bitmap(sf, v) = img {
            if st.ver != v {
                st.ver = v;
                st.conv = None;
                st.mips = None;
                if sf.fmt != FMT_PARGB && sf.fmt != FMT_ARGB && sf.w > 0 && sf.h > 0 {
                    // XRGB (opaque) -> PARGB
                    let mut c = Surface::new(sf.w, sf.h, FMT_PARGB);
                    let n = c.px.len();
                    if n > 0 && c.stride == sf.stride && sf.px.len() >= n {
                        c.px.copy_from_slice(&sf.px[..n]);
                        for p in c.px.chunks_exact_mut(4) {
                            p[3] = 255;
                        }
                        st.conv = Some(Rc::new(c));
                    }
                }
            }
        } else {
            st.conv = None;
            st.mips = None;
        }
        st.unit = unit;
        st.view = [r[2].max(0), r[3].max(0)];
        let (vw, vh) = (r[2].max(1) as f64, r[3].max(1) as f64);
        let has = iw > 0.0 && ih > 0.0;
        let fitz = if has { (vw / (iw * unit)).min(vh / (ih * unit)).clamp(ZOOM_MIN, ZOOM_MAX) } else { 1.0 };
        let view0 = (st.zoom, st.cx, st.cy, st.fit);
        // ---- input
        let mut cur = CUR_MOVE;
        if matches!(st.g, Gesture::Rect { .. }) || self.mods & MOD_CTRL != 0 {
            cur = CUR_CROSS;
        }
        let ev = self.behave(id, HB_FOCUS | HB_MOVES | cur << HB_CURSOR_SHIFT);
        let state = self.wp.ctx.last_state;
        let (mx, my) = (self.rtl_map(self.wp.ctx.mx, r[0], r[2]), self.wp.ctx.my);
        let (cxv, cyv) = (r[0] as f64 + vw * 0.5, r[1] as f64 + vh * 0.5);
        if st.fit {
            st.zoom = fitz;
            st.cx = iw * 0.5;
            st.cy = ih * 0.5;
        }
        st.zoom = st.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if has {
            // the wheel
            let c = &self.wp.ctx;
            let (wx, wy) = if ev & EV_HOVER != 0 { (c.wheel_x, c.wheel_y) } else { (0, 0) };
            if wx != 0 || wy != 0 {
                let ctrl = self.mods & MOD_CTRL != 0;
                let shift = self.mods & MOD_SHIFT != 0;
                let zoom = if flags & IV_WHEEL_SCROLL != 0 { ctrl } else { !shift };
                let s = st.zoom * unit;
                let notch = self.dp(48) as f64;
                if zoom && wy != 0 {
                    let f = rt::x87::pow(1.25, wy as f64 / 120.0);
                    st.zoom_at(st.zoom * f, mx as f64 + 0.5 - cxv, my as f64 + 0.5 - cyv);
                } else {
                    // wheel up shows the upper part (Shift: the left part)
                    let (mut ddx, mut ddy) = (wx as f64 / 120.0 * notch, -(wy as f64) / 120.0 * notch);
                    if shift {
                        ddx += ddy;
                        ddy = 0.0;
                    }
                    if self.rtl {
                        ddx = -ddx;
                    }
                    st.fit = false;
                    st.cx += ddx / s;
                    st.cy += ddy / s;
                }
                if zoom && wx != 0 {
                    st.fit = false;
                    st.cx += wx as f64 / 120.0 * notch / s;
                }
                let c = &mut self.wp.ctx;
                c.wheel_x = 0;
                c.wheel_y = 0;
            }
            // presses: pan (left, middle), zoom rectangle (Ctrl + left)
            let c = &self.wp.ctx;
            let hot = c.hot == id;
            let s = st.zoom * unit;
            if hot && c.pressed & (1 << BTN_LEFT) != 0 && c.active == id {
                st.g = if self.mods & MOD_CTRL != 0 { Gesture::Rect { x: mx, y: my } } else { Gesture::Pan { mx, my, cx: st.cx, cy: st.cy, middle: false } };
            } else if hot && c.pressed & (1 << BTN_MIDDLE) != 0 {
                st.g = Gesture::Pan { mx, my, cx: st.cx, cy: st.cy, middle: true };
            }
            let c = &self.wp.ctx;
            match st.g {
                Gesture::Pan { mx: x0, my: y0, cx, cy, middle } => {
                    let held = if middle { c.mbtn & (1 << BTN_MIDDLE) != 0 } else { c.active == id && c.mbtn & (1 << BTN_LEFT) != 0 };
                    if held {
                        if mx != x0 || my != y0 {
                            st.fit = false;
                            st.cx = cx - (mx - x0) as f64 / s;
                            st.cy = cy - (my - y0) as f64 / s;
                        }
                    } else {
                        st.g = Gesture::Idle;
                    }
                }
                Gesture::Rect { x, y } => {
                    if !(c.active == id && c.mbtn & (1 << BTN_LEFT) != 0) {
                        st.g = Gesture::Idle;
                        let lim = self.dp(4);
                        if (mx - x).abs() > lim && (my - y).abs() > lim {
                            // view px -> image px
                            let ox = cxv - st.cx * s;
                            let oy = cyv - st.cy * s;
                            let (x0, x1) = (x.min(mx) as f64, x.max(mx) as f64);
                            let (y0, y1) = (y.min(my) as f64, y.max(my) as f64);
                            st.zoom_to([(x0 - ox) / s, (y0 - oy) / s, (x1 - x0) / s, (y1 - y0) / s]);
                        }
                    }
                }
                Gesture::Idle => {}
            }
            // a double click: 100 % at the pointer, else fit
            if ev & EV_DOUBLE != 0 {
                st.g = Gesture::Idle;
                if st.fit || (st.zoom - 1.0).abs() > 1e-9 {
                    st.zoom_at(1.0, mx as f64 + 0.5 - cxv, my as f64 + 0.5 - cyv);
                } else {
                    st.fit = true;
                }
            }
            // keys
            if self.wp.ctx.focus == id {
                let s = st.zoom * unit;
                let (px, py) = (vw / 8.0 / s, vh / 8.0 / s);
                let mut moved = false;
                for (vk, ddx, ddy) in [(VK_LEFT, -px, 0.0), (VK_RIGHT, px, 0.0), (VK_UP, 0.0, -py), (VK_DOWN, 0.0, py)] {
                    if self.iv_key(vk, 0, MOD_CTRL | MOD_ALT) {
                        st.cx += ddx;
                        st.cy += ddy;
                        moved = true;
                    }
                }
                let page = vh * 0.9 / s;
                if self.iv_key(VK_PRIOR, 0, MOD_CTRL | MOD_ALT) {
                    st.cy -= page;
                    moved = true;
                }
                if self.iv_key(VK_NEXT, 0, MOD_CTRL | MOD_ALT) {
                    st.cy += page;
                    moved = true;
                }
                if self.iv_key(VK_HOME, 0, MOD_ALT) {
                    st.cy = f64::MIN / 4.0;
                    moved = true;
                }
                if self.iv_key(VK_END, 0, MOD_ALT) {
                    st.cy = f64::MAX / 4.0;
                    moved = true;
                }
                if moved {
                    st.fit = false;
                    self.wp.ctx.focus_visible = true;
                }
                for (vk, dir) in [(VK_OEM_PLUS, 1), (VK_ADD, 1), (VK_OEM_MINUS, -1), (VK_SUBTRACT, -1)] {
                    if self.iv_key(vk, 0, MOD_ALT) {
                        st.step(dir);
                    }
                }
                if self.iv_key(VK_0, MOD_CTRL, MOD_CTRL | MOD_ALT) || self.iv_key(VK_NUMPAD0, MOD_CTRL, MOD_CTRL | MOD_ALT) {
                    st.fit = true;
                }
                if self.iv_key(VK_1, MOD_CTRL, MOD_CTRL | MOD_ALT) || self.iv_key(VK_NUMPAD0 + 1, MOD_CTRL, MOD_CTRL | MOD_ALT) {
                    st.set_actual();
                }
            }
        }
        if st.fit {
            st.zoom = fitz;
            st.cx = iw * 0.5;
            st.cy = ih * 0.5;
        }
        // the centre inside the image (or the image centred when it fits)
        let s = st.zoom * unit;
        let clampc = |c: f64, len: f64, v: f64| -> f64 {
            let half = v * 0.5 / s;
            if len * s <= v { len * 0.5 } else { c.clamp(half, len - half) }
        };
        if has && s > 0.0 {
            st.cx = clampc(st.cx, iw, vw);
            st.cy = clampc(st.cy, ih, vh);
        }
        // ---- drawing
        let ox = (cxv - st.cx * s).rnd();
        let oy = (cyv - st.cy * s).rnd();
        if let ViewImage::Bitmap(sf, _) = img {
            // the mip levels when the image is shown below 50 %
            let base: &Surface = st.conv.as_deref().unwrap_or(sf);
            if s < 0.5 && st.mips.is_none() && base.w > 1 && base.h > 1 {
                if let Some(mut m) = MipChain::alloc(base.w, base.h, 0) {
                    if m.update(base, None) == 0 {
                        st.mips = Some(Rc::new(m));
                    }
                }
            }
        }
        let dark = self.theme_dark;
        let hc = self.high_contrast();
        let bg = self.color(if hc { T_BG } else { T_BG_ALT });
        let checker = if flags & IV_NO_CHECKER != 0 || hc {
            None
        } else if dark {
            Some((self.dp(8), 0x2B2B2B, 0x3A3A3A))
        } else {
            Some((self.dp(8), 0xFFFFFF, 0xE3E3E3))
        };
        let grid = if hc {
            self.color(T_TEXT) & 0xFF_FFFF | 0x9000_0000
        } else if dark {
            0x60FF_FFFF
        } else {
            0x5000_0000
        };
        let color = if flags & IV_TEXT_COLOR != 0 { self.color(T_TEXT) } else { 0xFF00_0000 };
        let fonts = self.svg_fonts();
        let mut h = Fnv::new();
        h.u64(key as u64 | (st.ver as u64) << 48);
        h.u64(s.to_bits());
        h.u64(ox.to_bits());
        h.u64(oy.to_bits());
        h.u64(flags as u64 | (bg as u64) << 32);
        h.u64(checker.map_or(0, |c| c.0 as u64 | (c.1 as u64) << 16 | (c.2 as u64) << 40));
        h.u64(grid as u64 | (color as u64) << 32);
        h.u64(Rc::as_ptr(&fonts) as usize as u64 ^ st.mips.as_ref().map_or(0, |m| Rc::as_ptr(m) as usize as u64));
        let hash = h.0;
        let obj = match &st.draw {
            Some((k, d)) if *k == hash => d.clone(),
            _ => {
                let image = match img {
                    ViewImage::Bitmap(sf, _) => IvImg::Bitmap(st.conv.clone().unwrap_or_else(|| sf.clone())),
                    ViewImage::Svg(d) => IvImg::Svg(d.clone()),
                };
                let d: DrawRef = Rc::new(IvDraw {
                    img: image,
                    mips: st.mips.clone(),
                    s,
                    ox,
                    oy,
                    iw,
                    ih,
                    flags,
                    bg,
                    checker,
                    grid,
                    color,
                    fonts,
                });
                st.draw = Some((hash, d.clone()));
                d
            }
        };
        self.ltr_begin(r[0], r[2]);
        self.clip_push(r[0], r[1], r[2], r[3]);
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(obj), 0, hash as u32);
        // scroll bars (over the image; their hit boxes after the view's)
        if has && flags & IV_NO_BARS == 0 {
            let sty = self.style;
            let t = sty.scrollbar_thickness(self);
            let (cw, ch) = ((iw * s).ceil_i64().min(i32::MAX as i64 / 2) as i32, (ih * s).ceil_i64().min(i32::MAX as i64 / 2) as i32);
            let (bw, bh) = (r[2], r[3]);
            let vneed = ch > bh;
            let hneed = cw > bw;
            if vneed {
                let mut pos = (st.cy * s - vh * 0.5).rnd() as i32;
                self.next_rect_px(r[0] + r[2] - t, r[1], t, if hneed { bh - t } else { bh });
                if self.scrollbar(part(id, 1), &mut pos, ch, bh, true) & EV_ACT != 0 {
                    st.fit = false;
                    st.cy = (pos as f64 + vh * 0.5) / s;
                }
            }
            if hneed {
                let mut pos = (st.cx * s - vw * 0.5).rnd() as i32;
                self.next_rect_px(r[0], r[1] + r[3] - t, if vneed { bw - t } else { bw }, t);
                if self.scrollbar(part(id, 2), &mut pos, cw, bw, false) & EV_ACT != 0 {
                    st.fit = false;
                    st.cx = (pos as f64 + vw * 0.5) / s;
                }
            }
        }
        // the zoom rectangle
        if let Gesture::Rect { x, y } = st.g {
            let (x0, y0) = (x.min(mx), y.min(my));
            let (w, hh) = ((x - mx).abs(), (y - my).abs());
            let acc = self.color(T_ACCENT);
            self.draw_color(acc & 0xFF_FFFF | 0x3000_0000);
            self.draw_rect(x0, y0, w, hh);
            self.draw_color(acc);
            self.draw_rect(x0, y0, w, 1);
            self.draw_rect(x0, y0 + hh - 1, w, 1);
            self.draw_rect(x0, y0, 1, hh);
            self.draw_rect(x0 + w - 1, y0, 1, hh);
        }
        self.clip_pop();
        self.ltr_end();
        // focus ring, the record
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state;
        self.focus_ring_draw(ev);
        let label = if !st.label.is_empty() {
            st.label.clone()
        } else {
            match img {
                ViewImage::Svg(d) if !d.title().is_empty() => String::from(d.title()),
                _ => String::from(tr("Bild")),
            }
        };
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = state;
        let changed = (st.zoom, st.cx, st.cy, st.fit) != view0;
        if changed {
            self.repass();
        }
        self.sem(K_IMAGE, id, Some(&label), st.zoom_pct());
        if changed { ev | EV_ACT } else { ev }
    }
}
