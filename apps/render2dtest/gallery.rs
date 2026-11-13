// gallery -- what both paths draw and judge: the Vulkan tests (tests.rs, scenes.rs, --dump) and the browser (web.rs,
// tests/engine_web.mjs read its console).  The drawings, the offscreen target, and what a test computes on the CPU
// from an image or from the visible lists; nothing here waits for the GPU (the browser cannot).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::math::{sincos, sqrt};
use render2d::{Camera, Cap, EntityId, Join, Layer, LayerId, Renderer, Scene, Shape, Style, Unit};

// ---------------------------------------------------------------- a test drawing

/// an entity of a test drawing
#[derive(Clone)]
pub enum Geo {
    Line([f64; 2], [f64; 2]),
    Poly(Vec<[f64; 2]>, bool),
    /// centre, radius, start, sweep (Shape::Arc)
    Arc([f64; 2], f64, f64, f64),
    /// centre, radius (Shape::Circle)
    Circle([f64; 2], f64),
    /// Bézier pieces (Shape::Quadratic, Shape::Cubic): points, degree (2 or 3), closed
    Bez(Vec<[f64; 2]>, u32, bool),
    /// Shape::Ellipse: centre, radii, rotation, start, sweep
    Ellipse([f64; 2], [f64; 2], f64, f64, f64),
    /// Shape::Spline: degree, points, weights, knots, closed
    Spline(u32, Vec<[f64; 2]>, Vec<f64>, Vec<f64>, bool),
    /// Shape::Fill: its rings, each of pieces end to end
    Fill(Vec<Vec<Geo>>),
}

#[derive(Clone)]
pub struct Item {
    pub layer: usize,
    pub style: Style,
    pub geo: Geo,
}

/// layers (visible, order) and the entities in the order of insertion
#[derive(Clone, Default)]
pub struct Drawing {
    pub layers: Vec<(bool, i32)>,
    pub items: Vec<Item>,
}

impl Drawing {
    pub fn one_layer() -> Drawing {
        Drawing { layers: vec![(true, 0)], items: Vec::new() }
    }

    pub fn add(&mut self, style: Style, geo: Geo) {
        self.items.push(Item { layer: 0, style, geo });
    }
}

/// the shape of a geometry other than a fill (with_shape takes fills too)
pub fn shape_of(g: &Geo) -> Shape<'_> {
    match g {
        Geo::Line(a, b) => Shape::Line { a: *a, b: *b },
        Geo::Poly(v, closed) => Shape::Polyline { points: v, closed: *closed },
        Geo::Arc(c, r, a0, sweep) => Shape::Arc { center: *c, radius: *r, start: *a0, sweep: *sweep },
        Geo::Circle(c, r) => Shape::Circle { center: *c, radius: *r },
        Geo::Bez(v, 2, closed) => Shape::Quadratic { points: v, closed: *closed },
        Geo::Bez(v, _, closed) => Shape::Cubic { points: v, closed: *closed },
        Geo::Ellipse(c, r, rot, a0, sweep) => Shape::Ellipse { center: *c, radii: *r, rotation: *rot, start: *a0, sweep: *sweep },
        Geo::Spline(d, v, w, k, closed) => Shape::Spline { degree: *d, points: v, weights: w, knots: k, closed: *closed },
        Geo::Fill(_) => Shape::Polyline { points: &[], closed: false },
    }
}

/// f with the geometry's shape (a fill's rings borrowed from here)
pub fn with_shape<R>(g: &Geo, f: impl FnOnce(&Shape) -> R) -> R {
    if let Geo::Fill(rings) = g {
        let shapes: Vec<Vec<Shape>> = rings.iter().map(|r| r.iter().map(shape_of).collect()).collect();
        let refs: Vec<&[Shape]> = shapes.iter().map(|v| &v[..]).collect();
        return f(&Shape::Fill { rings: &refs });
    }
    f(&shape_of(g))
}

/// a test drawing in a scene: its layers
pub struct Built {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub layers: Vec<LayerId>,
}

pub fn build(s: &mut Scene, d: &Drawing) -> Result<Built> {
    let mut layers = Vec::new();
    for &(visible, order) in &d.layers {
        let mut l = Layer::new(order);
        l.visible = visible;
        layers.push(s.add_layer(&l)?);
    }
    for it in &d.items {
        let st = s.add_style(&it.style)?;
        with_shape(&it.geo, |sh| s.insert(layers[it.layer], st, sh))?;
    }
    Ok(Built { layers })
}

pub fn style(color: u32, width: f32, cap: Cap, join: Join) -> Style {
    let mut s = Style::new(color);
    s.width = width;
    s.cap = cap;
    s.join = join;
    s
}

/// a camera with the drawing's units as pixels: (0, 0) at the target's top left, y up (so rows count down from h)
pub fn pixel_camera(w: u32, h: u32) -> Camera {
    Camera::new([0.5 * w as f64, 0.5 * h as f64], 1.0)
}

pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let w = ((self.0 >> ((self.0 >> 28) + 4)) ^ self.0).wrapping_mul(277_803_737);
        (w >> 22) ^ w
    }

    /// in [0, 1), 24 bits
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 8) as f64 / 16_777_216.0
    }

    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }

    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

pub const CAPS: [Cap; 3] = [Cap::Butt, Cap::Round, Cap::Square];
pub const JOINS: [Join; 3] = [Join::Miter, Join::Bevel, Join::Round];
pub const INK: [u32; 4] = [0xFF20_3040, 0xFFB0_3020, 0xFF20_7040, 0xFF30_40C0];

// ---------------------------------------------------------------- the offscreen target

/// the background of the stroke tests (sRGB codes): a light grey
pub const BG: u32 = 0xFFE8_E4DC;

pub fn bg_clear(bg: u32) -> [f32; 4] {
    let c = |s: u32| ((bg >> s) & 255) as f32 / 255.0;
    [c(16), c(8), c(0), 1.0]
}

/// an offscreen target (Bgra8Unorm, as a ui view) with the read-back of its image
pub struct Off {
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub color: Texture,
    pub rb: Buffer,
    pub bg: u32,
    used: bool,
}

impl Off {
    pub fn new(dev: &Device, w: u32, h: u32) -> Result<Off> {
        let stride = (w * 4).div_ceil(256) * 256;
        Ok(Off {
            w,
            h,
            stride,
            color: dev.create_texture(&TextureDesc::d2(w, h, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            bg: BG,
            used: false,
        })
    }

    /// one frame of the scene, with the image copied to the read-back
    pub fn submit(&mut self, r: &mut Renderer, s: &mut Scene) -> Result<SubmitId> {
        s.prepare(r, (self.w, self.h))?;
        let dev = r.device().clone();
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: bg_clear(self.bg), store: true,
                                      before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        r.record(&mut e, s, color)?;
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        s.submitted(id);
        self.used = true;
        Ok(id)
    }

    /// the last image: rows of B, G, R, A (the browser: NotReady until the GPU has finished)
    pub fn read(&self) -> Result<Vec<u8>> {
        let mut raw = vec![0u8; self.stride as usize * self.h as usize];
        self.rb.read(0, &mut raw)?;
        let mut v = Vec::with_capacity((self.w * self.h * 4) as usize);
        for y in 0..self.h as usize {
            v.extend_from_slice(&raw[y * self.stride as usize..y * self.stride as usize + self.w as usize * 4]);
        }
        Ok(v)
    }
}

// ---------------------------------------------------------------- the drawings of the tests

/// a star of 12 segments (every angle a cap meets the pixel grid at) and a dot (of a round cap; a square of a square
/// cap, nothing of a butt cap)
pub fn caps_drawing(cap: Cap, width: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    let (cx, cy) = (160.3, 120.6);
    for k in 0..12 {
        let a = k as f64 * core::f64::consts::TAU / 12.0 + 0.1;
        let (s, c) = sincos(a);
        let (r0, r1) = (18.0 + width as f64, 100.0);
        d.add(style(INK[k % 4], width, cap, Join::Miter), Geo::Line([cx + c * r0, cy + s * r0], [cx + c * r1, cy + s * r1]));
    }
    d.add(style(INK[1], width.max(4.0), cap, Join::Miter), Geo::Poly(vec![[20.5, 20.5]], false));
    d
}

/// turns from blunt to sharp, both ways (a zigzag), a closed triangle and a closed square, the nearly straight joins
/// of a flattened curve
pub fn joins_drawing(join: Join, width: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    d.add(style(INK[0], width, Cap::Butt, join), Geo::Poly(vec![[20.0, 40.0], [80.0, 30.0], [120.0, 70.0], [150.0, 20.0], [175.0, 75.0], [190.0, 22.0]], false));
    d.add(style(INK[1], width, Cap::Round, join), Geo::Poly(vec![[215.0, 30.0], [340.0, 45.0], [230.0, 60.0], [335.0, 80.0]], false));
    d.add(style(INK[2], width, Cap::Butt, join), Geo::Poly(vec![[30.0, 120.0], [150.0, 140.0], [60.0, 220.0]], true));
    d.add(style(INK[3], width, Cap::Square, join), Geo::Poly(vec![[200.0, 120.0], [320.0, 120.3], [320.0, 230.0], [200.5, 230.0]], true));
    let arc: Vec<[f64; 2]> = (0..24).map(|k| {
        let (s, c) = sincos(k as f64 * 0.05);
        [180.0 + 140.0 * c - 140.0, 180.0 + 60.0 * s]
    }).collect();
    d.add(style(INK[0], width, Cap::Butt, join), Geo::Poly(arc, false));
    d
}

/// acos for the tests (no libm): Newton on cos
fn acos(x: f64) -> f64 {
    let mut a = core::f64::consts::FRAC_PI_2 * (1.0 - x);
    for _ in 0..60 {
        let (s, c) = sincos(a);
        if s.abs() < 1e-12 {
            break;
        }
        a += (c - x) / s;
    }
    a
}

/// a turn whose miter is 3.9 widths long, 4.1, 8 and 12
pub fn miter_drawing(limit: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    for (k, ratio) in [3.9f64, 4.1, 8.0, 12.0].iter().enumerate() {
        // 1 / cos(phi / 2) = ratio: phi the turn
        let phi = 2.0 * acos(1.0 / ratio);
        let (s, c) = sincos(core::f64::consts::PI - phi);
        let x = 40.0 + 80.0 * k as f64;
        let mut st = style(INK[k % 4], 8.0, Cap::Butt, Join::Miter);
        st.miter_limit = limit;
        d.add(st, Geo::Poly(vec![[x - 30.0, 200.0], [x, 200.0], [x + 70.0 * c, 200.0 - 70.0 * s]], false));
    }
    d
}

/// 36 angles, the start at fractions of a pixel; axis-aligned on pixel centres and on pixel edges
pub fn hairlines_drawing(width: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    for k in 0..36 {
        let a = k as f64 * core::f64::consts::TAU / 36.0 + 0.013;
        let (s, c) = sincos(a);
        let (cx, cy) = (150.0 + 0.1 * (k % 10) as f64, 150.0 + 0.07 * (k % 7) as f64);
        d.add(style(INK[k % 4], width, Cap::Butt, Join::Miter), Geo::Line([cx + 20.0 * c, cy + 20.0 * s], [cx + 140.0 * c, cy + 140.0 * s]));
    }
    d.add(style(INK[1], width, Cap::Butt, Join::Miter), Geo::Line([10.0, 290.5], [290.0, 290.5]));
    d.add(style(INK[2], width, Cap::Butt, Join::Miter), Geo::Line([10.0, 285.0], [290.0, 285.0]));
    d
}

/// a drawing in metres: px widths stay, unit widths follow the zoom
pub fn units_drawing() -> Drawing {
    let mut d = Drawing::one_layer();
    let unit = |c: u32, wdt: f32, u: Unit, cap: Cap, join: Join| {
        let mut s = style(c, wdt, cap, join);
        s.unit = u;
        s
    };
    d.add(unit(INK[0], 0.5, Unit::Drawing, Cap::Round, Join::Round), Geo::Poly(vec![[-10.0, -5.0], [0.0, 6.0], [8.0, -4.0], [12.0, 3.0]], false));
    d.add(unit(INK[1], 3.0, Unit::Px, Cap::Square, Join::Miter), Geo::Poly(vec![[-12.0, 4.0], [-2.0, -6.0], [10.0, 7.0]], false));
    d.add(unit(INK[2], 0.2, Unit::Drawing, Cap::Butt, Join::Bevel), Geo::Poly(vec![[-6.0, -6.0], [6.0, -6.0], [6.0, 6.0], [-6.0, 6.0]], true));
    d.add(unit(INK[3], 0.0, Unit::Drawing, Cap::Butt, Join::Miter), Geo::Line([-13.0, 0.0], [13.0, 0.5]));
    d
}

pub const UNIT_VIEWS: [(f64, f64); 4] = [(0.1, 0.0), (0.05, 0.0), (0.02, 0.4), (0.2, -1.2)];

pub fn units_camera(scale: f64, rotation: f64) -> Camera {
    let mut cam = Camera::new([0.3, -0.2], scale);
    cam.rotation = rotation;
    cam
}

/// a sharp corner (20 degrees), a hairpin, a blunt one, a closed square, translucent
pub fn overlap_drawing(join: Join, alpha: u32) -> Drawing {
    let c = (alpha << 24) | 0x0020_40C0;
    let mut d = Drawing::one_layer();
    d.add(style(c, 14.0, Cap::Butt, join), Geo::Poly(vec![[20.0, 40.0], [200.0, 60.0], [30.0, 95.0]], false));
    d.add(style(c, 10.0, Cap::Round, join), Geo::Poly(vec![[30.0, 140.0], [180.0, 150.0], [40.0, 160.0]], false));
    d.add(style(c, 12.0, Cap::Square, join), Geo::Poly(vec![[230.0, 30.0], [290.0, 120.0], [230.0, 200.0]], false));
    d.add(style(c, 9.0, Cap::Butt, join), Geo::Poly(vec![[60.0, 190.0], [180.0, 190.0], [180.0, 230.0], [60.0, 230.0]], true));
    d
}

/// pixels of an image more than one blend of the overlap test's colour away from the background (blended twice)
pub fn blended_twice(img: &[u8], alpha: u32) -> u32 {
    let once = |v: u32, b: u32| (b as f64 + (v as f64 - b as f64) * alpha as f64 / 255.0 + 0.5) as i32;
    let target = [once(0xC0, BG & 255), once(0x40, (BG >> 8) & 255), once(0x20, (BG >> 16) & 255)];
    let mut twice = 0u32;
    for p in img.chunks_exact(4) {
        // (every channel of the colour lies below the background's: twice pulls it further down)
        if (0..3).any(|k| (p[k] as i32) < target[k] - 2) {
            twice += 1;
        }
    }
    twice
}

/// many short polylines of random turns, segments of 0.4 to 6 pixels
pub fn short_drawing(seed: u32, width: f32, join: Join) -> Drawing {
    let mut d = Drawing::one_layer();
    let mut rng = Rng(seed);
    for k in 0..120 {
        let (mut x, mut y) = (rng.range(4.0, 316.0), rng.range(4.0, 236.0));
        let n = 2 + rng.below(9) as usize;
        let mut pts = vec![[x, y]];
        for _ in 1..n {
            x = (x + rng.range(-6.0, 6.0)).clamp(2.0, 318.0);
            y = (y + rng.range(-6.0, 6.0)).clamp(2.0, 238.0);
            pts.push([x, y]);
        }
        d.add(style(INK[k % 4], width, CAPS[k % 3], join), Geo::Poly(pts, k % 7 == 0 && n >= 3));
    }
    d
}

/// circles of many radii (under a pixel to beyond the target, centres at fractions of a pixel) and open arcs of every
/// direction and sweep (a sliver, three pieces exactly, nearly a full turn, clockwise), with `cap` and `width`
pub fn arcs_drawing(cap: Cap, width: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    let st = |k: usize| style(INK[k % 4], width, cap, Join::Miter);
    for (k, (c, r)) in [([40.3, 200.6], 0.8), ([70.5, 200.2], 3.0), ([120.25, 195.7], 12.3), ([210.6, 180.1], 41.0), ([150.0, -170.0], 260.0)].into_iter().enumerate() {
        d.add(st(k), Geo::Circle(c, r));
    }
    let pi8 = core::f64::consts::PI / 8.0;
    for (k, (c, r, a0, sw)) in [([60.2, 90.4], 30.0, 0.3, 1.2), ([140.7, 90.1], 25.0, 2.0, -2.5), ([235.3, 80.9], 35.0, -1.0, 5.9), ([295.6, 200.3], 18.0, 0.7, 0.05),
                                 ([285.1, 60.8], 40.0, -0.4, 3.0 * pi8)].into_iter().enumerate() {
        d.add(st(k + 1), Geo::Arc(c, r, a0, sw));
    }
    d
}

/// translucent rings and an arc: no pixel where two of an arc's pieces meet may blend twice
pub fn arcs_overlap_drawing(alpha: u32) -> Drawing {
    let c = (alpha << 24) | 0x0020_40C0;
    let mut d = Drawing::one_layer();
    d.add(style(c, 10.0, Cap::Butt, Join::Miter), Geo::Circle([90.3, 120.6], 60.0));
    d.add(style(c, 3.0, Cap::Butt, Join::Miter), Geo::Circle([90.3, 120.6], 20.0));
    d.add(style(c, 14.0, Cap::Round, Join::Miter), Geo::Arc([245.6, 120.2], 70.0, 0.4, 4.0));
    d.add(style(c, 6.0, Cap::Square, Join::Miter), Geo::Arc([245.6, 120.2], 30.0, 2.0, -5.0));
    d
}

/// a style with a dash pattern
pub fn dashed(color: u32, width: f32, cap: Cap, join: Join, pat: &[f32], unit: Unit, phase: f32) -> Style {
    style(color, width, cap, join).dashed(pat, unit, phase)
}

/// dash patterns: lines at many angles (two patterns, phases), an odd pattern (it runs twice), dots (length 0), a zigzag
/// whose dashes run through some of its vertices and not others, closed polylines on and off at their first point, a
/// dashed arc and circle
pub fn dash_drawing(cap: Cap, join: Join, width: f32) -> Drawing {
    let mut d = Drawing::one_layer();
    let st = |k: usize, pat: &[f32], phase: f32| dashed(INK[k % 4], width, cap, join, pat, Unit::Px, phase);
    for k in 0..8 {
        let (s, c) = sincos(k as f64 * 0.41 + 0.1);
        let (cx, cy) = (90.3, 180.6);
        let pat: &[f32] = if k % 2 == 0 { &[12.0, 6.0] } else { &[9.0, 4.0, 2.0, 4.0] };
        d.add(st(k, pat, 2.5 * k as f32 + 0.37), Geo::Line([cx + 18.0 * c, cy + 18.0 * s], [cx + 85.0 * c, cy + 85.0 * s]));
    }
    d.add(st(1, &[10.0, 4.0, 3.0], 0.0), Geo::Line([190.0, 250.5], [350.0, 244.0]));
    d.add(st(2, &[0.0, 7.0], 1.37), Geo::Line([190.0, 230.5], [350.0, 230.5]));
    d.add(st(3, &[22.0, 7.0], 3.0), Geo::Poly(vec![[190.0, 200.0], [230.0, 160.0], [260.0, 208.0], [300.0, 152.0], [345.0, 204.0]], false));
    d.add(st(0, &[16.0, 5.0], 0.0), Geo::Poly(vec![[20.0, 70.0], [120.0, 70.5], [120.0, 20.0], [20.5, 20.0]], true));
    d.add(st(1, &[16.0, 5.0], 18.0), Geo::Poly(vec![[140.0, 70.0], [230.0, 65.0], [180.0, 15.0]], true));
    d.add(st(2, &[14.0, 5.0], 0.0), Geo::Arc([300.0, 70.0], 40.0, 0.2, 2.9));
    d.add(st(3, &[10.0, 4.0], 3.0), Geo::Circle([300.0, 70.0], 22.0));
    d
}

/// horizontal dashed lines of a period of `period` px (a flat tone below DASH_FLAT, exact from DASH_EXACT, mixed between)
pub fn flat_drawing(period: f32, width: f32, cap: Cap) -> Drawing {
    let mut d = Drawing::one_layer();
    for k in 0..6 {
        let y = 20.3 + 30.0 * k as f64;
        let duty = 0.2 + 0.12 * k as f32;
        d.add(dashed(INK[k % 4], width, cap, Join::Miter, &[duty * period, (1.0 - duty) * period], Unit::Px, 0.0), Geo::Line([10.0, y], [310.0, y]));
    }
    d
}

/// the units drawing with dash patterns in drawing units (the lines' widths as there)
pub fn units_dash_drawing() -> Drawing {
    let mut d = units_drawing();
    for (k, it) in d.items.iter_mut().enumerate() {
        let pat: &[f32] = if k % 2 == 0 { &[1.5, 0.6] } else { &[0.9, 0.4, 0.2, 0.4] };
        it.style = it.style.dashed(pat, Unit::Drawing, 0.3 * k as f32 + 0.07);
    }
    d.add(dashed(INK[1], 2.0, Cap::Butt, Join::Miter, &[2.0, 1.0], Unit::Drawing, 0.0), Geo::Arc([1.0, -1.0], 8.0, 0.3, 4.0));
    d
}

// ---------------------------------------------------------------- curves, fills, hatches (G6b1)

/// curves of every kind in 360 x 270 px (units = px): cubic pieces (an S, a loop, a sharp turn), quadratic pieces closed,
/// ellipses (rotated, a full one, an arc), splines (clamped degree 2 and 3, periodic and closed, a rational circle of degree
/// 2, explicit knots not clamped); a dash pattern when pat is not empty
pub fn curves_drawing(cap: Cap, join: Join, width: f32, pat: &[f32]) -> Drawing {
    let mut d = Drawing::one_layer();
    let st = |k: usize| style(INK[k % 4], width, cap, join).dashed(pat, Unit::Px, 1.3 * k as f32);
    d.add(st(0), Geo::Bez(vec![[20.0, 60.0], [60.0, 10.0], [90.0, 110.0], [130.0, 40.0], [150.0, 10.0], [170.0, 90.0], [200.0, 50.0]], 3, false));
    d.add(st(1), Geo::Bez(vec![[220.0, 30.0], [360.0, 120.0], [200.0, 120.0], [330.0, 20.0]], 3, false));
    d.add(st(2), Geo::Bez(vec![[20.0, 140.0], [120.0, 200.0], [20.0, 200.0], [110.0, 140.5]], 3, false));
    d.add(st(3), Geo::Bez(vec![[150.0, 140.0], [190.0, 120.0], [230.0, 150.0], [250.0, 200.0], [190.0, 210.0], [140.0, 190.0], [150.0, 140.0]], 2, true));
    d.add(st(0), Geo::Ellipse([300.0, 185.0], [45.0, 18.0], 0.5, 0.0, core::f64::consts::TAU));
    d.add(st(1), Geo::Ellipse([300.0, 185.0], [30.0, 12.0], -0.3, 0.4, 3.9));
    d.add(st(2), Geo::Spline(2, vec![[20.0, 250.0], [40.0, 215.0], [70.0, 260.0], [100.0, 220.0], [120.0, 255.0]], vec![], vec![], false));
    d.add(st(3), Geo::Spline(3, vec![[140.0, 230.0], [160.0, 260.0], [190.0, 220.0], [210.0, 262.0], [230.0, 225.0], [245.0, 250.0]], vec![], vec![], false));
    d.add(st(0), Geo::Spline(3, vec![[262.0, 235.0], [290.0, 225.0], [300.0, 250.0], [280.0, 265.0], [258.0, 255.0]], vec![], vec![], true));
    // (a rational quadratic spline: a circle of radius 14, nine points, weights 1 and 1 / sqrt 2)
    let s2 = core::f64::consts::FRAC_1_SQRT_2;
    let (cx, cy, rr) = (334.0, 246.0, 14.0);
    let ring: Vec<[f64; 2]> = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0], [1.0, -1.0], [1.0, 0.0]].iter().map(|p| [cx + rr * p[0], cy + rr * p[1]]).collect();
    d.add(st(1), Geo::Spline(2, ring, vec![1.0, s2, 1.0, s2, 1.0, s2, 1.0, s2, 1.0], vec![0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 4.0], false));
    d.add(st(2), Geo::Spline(3, vec![[255.0, 130.0], [275.0, 155.0], [300.0, 125.0], [325.0, 158.0], [345.0, 128.0], [355.0, 150.0]], vec![], vec![0.0, 0.5, 1.0, 2.0, 3.5, 4.0, 5.0, 6.0, 6.5, 8.0], false));
    d
}

/// the curve cases: caps, joins, widths, dash patterns
pub const CURVE_CASES: [(Cap, Join, f32, &[f32]); 5] = [(Cap::Butt, Join::Miter, 1.0, &[]), (Cap::Round, Join::Round, 3.0, &[]), (Cap::Square, Join::Bevel, 7.0, &[]), (Cap::Butt, Join::Miter, 2.0, &[12.0, 5.0]),
                                                       (Cap::Round, Join::Round, 3.0, &[9.0, 4.0, 1.0, 4.0])];

fn ring(v: &[[f64; 2]]) -> Vec<Geo> {
    vec![Geo::Poly(v.to_vec(), true)]
}

/// fills in 360 x 270 px: a square with a hole and an island in it, two triangles touching at a corner, a hole touching its
/// outer ring, collinear and repeated points with a notch, a circle with an elliptic hole, a ring of a Bézier piece and an
/// arc, a polygon with many points (a flattened wave); of the colour's alpha
pub fn fills_drawing(alpha: u32) -> Drawing {
    let mut d = Drawing::one_layer();
    // (translucent: the colour of blended_twice; the fills do not overlap)
    let st = |k: usize| style((alpha << 24) | if alpha < 0xFF { 0x0020_40C0 } else { INK[k % 4] & 0x00FF_FFFF }, 0.0, Cap::Butt, Join::Miter);
    d.add(st(0), Geo::Fill(vec![ring(&[[20.5, 20.3], [140.2, 20.3], [140.2, 120.7], [20.5, 120.7]]), ring(&[[45.0, 45.0], [115.0, 45.0], [115.0, 100.0], [45.0, 100.0]]),
                                ring(&[[65.3, 60.2], [95.0, 60.2], [80.1, 85.6]])]));
    d.add(st(1), Geo::Fill(vec![ring(&[[160.0, 20.0], [230.0, 20.0], [195.0, 75.0]]), ring(&[[195.0, 75.0], [240.0, 120.0], [150.0, 120.0]])]));
    d.add(st(2), Geo::Fill(vec![ring(&[[250.0, 20.0], [345.0, 20.0], [345.0, 110.0], [250.0, 110.0]]), ring(&[[297.5, 20.0], [330.0, 60.0], [270.0, 60.0]])]));
    d.add(st(3), Geo::Fill(vec![ring(&[[20.0, 140.0], [50.0, 140.0], [50.0, 140.0], [80.0, 140.0], [110.0, 140.0], [110.0, 200.0], [80.3, 200.0], [65.0, 150.0], [50.0, 200.0], [20.0, 200.0],
                                       [20.0, 170.0]])]));
    d.add(st(0), Geo::Fill(vec![vec![Geo::Circle([175.0, 185.0], 48.0)], vec![Geo::Ellipse([180.0, 183.0], [25.0, 12.0], 0.6, 0.0, core::f64::consts::TAU)]]));
    d.add(st(1), Geo::Fill(vec![vec![Geo::Bez(vec![[240.0, 140.0], [300.0, 115.0], [345.0, 195.0]], 2, false), Geo::Arc([305.0, 210.0], 40.0, 0.0, core::f64::consts::PI)]]));
    let wave: Vec<[f64; 2]> = (0..=60).map(|k| [20.0 + 1.5 * k as f64, 230.0 + 9.0 * sincos(k as f64 * 0.37).0]).chain([[110.0, 262.0], [20.0, 262.0]]).collect();
    d.add(st(2), Geo::Fill(vec![ring(&wave)]));
    d
}

/// a self-intersecting star and a ring crossing another: even-odd, approximately (decision 26)
pub fn fills_crossing_drawing() -> Drawing {
    let mut d = Drawing::one_layer();
    let star: Vec<[f64; 2]> = (0..5).map(|k| {
        let (s, c) = sincos(-core::f64::consts::FRAC_PI_2 + k as f64 * 4.0 * core::f64::consts::PI / 5.0);
        [90.3 + 70.0 * c, 120.6 + 70.0 * s]
    }).collect();
    d.add(style(0xFF20_4080, 0.0, Cap::Butt, Join::Miter), Geo::Fill(vec![ring(&star)]));
    d.add(style(0xFF80_4020, 0.0, Cap::Butt, Join::Miter), Geo::Fill(vec![ring(&[[200.0, 60.0], [300.0, 60.0], [300.0, 160.0], [200.0, 160.0]]), ring(&[[250.0, 110.0], [330.0, 110.0], [330.0, 200.0], [250.0, 200.0]])]));
    d
}

/// hatched fills in 360 x 270 px, the lines `spacing` px apart: one family at 45 degrees, crossed lines, a dashed family
/// (shifted line to line) with dots; widths 1 and 2 px
pub fn hatch_drawing(spacing: f64) -> Drawing {
    let mut d = Drawing::one_layer();
    let st = |k: usize, fam: &[render2d::HatchLine], w: f32| style(INK[k % 4], 0.0, Cap::Butt, Join::Miter).hatched(fam, w);
    let q = core::f64::consts::FRAC_PI_4;
    let mut a = render2d::HatchLine::new(q, spacing);
    a.base = [3.3, 1.7];
    d.add(st(0, &[a], 1.0), Geo::Fill(vec![ring(&[[20.5, 20.3], [160.2, 20.3], [160.2, 120.7], [20.5, 120.7]]), ring(&[[60.0, 50.0], [120.0, 50.0], [90.0, 100.0]])]));
    let b0 = render2d::HatchLine::new(0.1, spacing * 1.3);
    let b1 = render2d::HatchLine::new(0.1 + core::f64::consts::FRAC_PI_2, spacing * 1.7);
    d.add(st(1, &[b0, b1], 1.0), Geo::Fill(vec![vec![Geo::Circle([265.0, 75.0], 55.0)]]));
    let mut c = render2d::HatchLine::new(-0.5, spacing * 1.5).dashed(&[4.0 * spacing, 1.5 * spacing, 0.0, 1.5 * spacing]);
    c.shift = 2.1 * spacing;
    c.base = [100.0, 200.0];
    d.add(st(2, &[c], 2.0), Geo::Fill(vec![vec![Geo::Ellipse([110.0, 200.0], [90.0, 50.0], 0.2, 0.0, core::f64::consts::TAU)]]));
    d.add(st(3, &[a, b1], 1.0), Geo::Fill(vec![ring(&[[230.0, 150.0], [345.0, 165.0], [320.0, 255.0], [215.0, 240.0]])]));
    d
}

/// spacings of the hatch tests (px): exact, mixed, flat
pub const HATCH_SPACINGS: [f64; 3] = [7.0, 3.0, 1.5];

/// one image both paths draw
pub struct Shot {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub d: Drawing,
    pub cam: Camera,
    /// the translucent images: the alpha of their colour (blended_twice)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub alpha: Option<u32>,
    /// (G6b2) a drawing of blocks instead of `d`
    pub b: Option<BlockDrawing>,
}

impl Shot {
    /// its drawing in a scene
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn build(&self, s: &mut Scene) -> Result<()> {
        match &self.b {
            Some(b) => build_blocks(s, b).map(|_| ()),
            None => build(s, &self.d).map(|_| ()),
        }
    }
}

/// the images the browser is compared with Vulkan on: caps, joins, hairlines, units, overlap, every case of the tests
pub fn shots() -> Vec<Shot> {
    let mut v = Vec::new();
    let mut add = |name: String, w: u32, h: u32, d: Drawing, cam: Camera, alpha: Option<u32>| v.push(Shot { name, w, h, d, cam, alpha, b: None });
    for cap in CAPS {
        for width in [1.0f32, 2.5, 6.0, 15.0] {
            add(format!("caps_{cap:?}_{width}"), 320, 240, caps_drawing(cap, width), pixel_camera(320, 240), None);
        }
    }
    for join in JOINS {
        for width in [1.0f32, 3.0, 9.0, 18.0] {
            add(format!("joins_{join:?}_{width}"), 360, 260, joins_drawing(join, width), pixel_camera(360, 260), None);
        }
    }
    for limit in [4.0f32, 10.0] {
        add(format!("miter_{limit}"), 360, 260, miter_drawing(limit), pixel_camera(360, 260), None);
    }
    for (k, width) in [0.0f32, 0.4, 1.0].iter().enumerate() {
        add(format!("hairlines_{k}"), 300, 300, hairlines_drawing(*width), pixel_camera(300, 300), None);
    }
    for (k, (scale, rot)) in UNIT_VIEWS.iter().enumerate() {
        add(format!("units_{k}"), 320, 240, units_drawing(), units_camera(*scale, *rot), None);
    }
    // short segments and sharp turns (the labels and details of a plan seen from afar): the near-join pieces of every kind
    for (k, (seed, width, join)) in [(1u32, 0.0f32, Join::Miter), (2, 1.0, Join::Miter), (3, 3.0, Join::Round), (4, 1.0, Join::Bevel), (5, 2.0, Join::Round), (6, 0.0, Join::Bevel)].into_iter().enumerate() {
        add(format!("short_{k}"), 320, 240, short_drawing(seed, width, join), pixel_camera(320, 240), None);
    }
    for join in JOINS {
        for alpha in [0x80u32, 0x40] {
            add(format!("overlap_{join:?}_{alpha:x}"), 320, 240, overlap_drawing(join, alpha), pixel_camera(320, 240), Some(alpha));
        }
    }
    // arcs and circles, translucent ones (their pieces' joints), dash patterns in pixels and in drawing units, flat tones
    for cap in CAPS {
        for width in [1.0f32, 3.0, 9.0] {
            add(format!("arcs_{cap:?}_{width}"), 320, 240, arcs_drawing(cap, width), pixel_camera(320, 240), None);
        }
    }
    for alpha in [0x80u32, 0x40] {
        add(format!("arcs_overlap_{alpha:x}"), 320, 240, arcs_overlap_drawing(alpha), pixel_camera(320, 240), Some(alpha));
    }
    for (cap, join, width) in DASH_CASES {
        add(format!("dashes_{cap:?}_{join:?}_{width}"), 360, 270, dash_drawing(cap, join, width), pixel_camera(360, 270), None);
    }
    for (k, (scale, rot)) in UNIT_VIEWS.iter().enumerate() {
        add(format!("units_dash_{k}"), 320, 240, units_dash_drawing(), units_camera(*scale, *rot), None);
    }
    for period in FLAT_PERIODS {
        add(format!("flat_{period}"), 320, 200, flat_drawing(period, 2.0, Cap::Butt), pixel_camera(320, 200), None);
    }
    // curves, fills (translucent: blended once), hatches (G6b1)
    for (k, (cap, join, width, pat)) in CURVE_CASES.into_iter().enumerate() {
        add(format!("curves_{k}"), 360, 270, curves_drawing(cap, join, width, pat), pixel_camera(360, 270), None);
    }
    for alpha in [0xFFu32, 0x80] {
        add(format!("fills_{alpha:x}"), 360, 270, fills_drawing(alpha), pixel_camera(360, 270), if alpha < 0xFF { Some(alpha) } else { None });
    }
    add("fills_crossing".into(), 360, 270, fills_crossing_drawing(), pixel_camera(360, 270), None);
    for sp in HATCH_SPACINGS {
        add(format!("hatch_{sp}"), 360, 270, hatch_drawing(sp), pixel_camera(360, 270), None);
    }
    // blocks inserted in every way, transparency (G6b2)
    for (name, b) in [("blocks", blocks_drawing(false)), ("blocks_uneven", blocks_drawing(true)), ("transparency", transparency_drawing([0, 40, 90]))] {
        v.push(Shot { name: name.into(), w: 360, h: 270, d: Drawing::default(), cam: pixel_camera(360, 270), alpha: None, b: Some(b) });
    }
    v
}

/// the dash test's styles: caps, joins, widths
pub const DASH_CASES: [(Cap, Join, f32); 6] = [(Cap::Butt, Join::Miter, 1.0), (Cap::Butt, Join::Miter, 3.0), (Cap::Round, Join::Round, 3.0), (Cap::Square, Join::Bevel, 2.0),
                                              (Cap::Round, Join::Round, 8.0), (Cap::Butt, Join::Bevel, 6.0)];
/// periods of the flat tone test (px): flat, mixed, exact
pub const FLAT_PERIODS: [f32; 3] = [1.5, 3.0, 6.0];

// ---------------------------------------------------------------- large coordinates

pub const FAR_POINTS: [[f64; 2]; 3] = [[1.0e7, 1.0e7], [-9_999_999.5, 3_141_592.653_589_793], [9_876_543.210_987, -9_999_000.0]];
pub const FAR_SIZE: (u32, u32) = (240, 160);

/// a drawing of every kind of stroke around a point
pub fn sample(at: [f64; 2]) -> Drawing {
    let mut d = Drawing::one_layer();
    let p = |x: f64, y: f64| [at[0] + x, at[1] + y];
    d.add(style(0xFF20_3040, 6.0, Cap::Round, Join::Round), Geo::Poly(vec![p(-80.0, -50.0), p(-20.0, 40.0), p(30.0, -30.0), p(90.0, 45.0)], false));
    d.add(style(0x9030_8040, 11.0, Cap::Square, Join::Miter), Geo::Poly(vec![p(-90.0, 10.0), p(80.0, 20.0), p(-60.0, 35.0)], false));
    d.add(style(0xFF80_2020, 0.0, Cap::Butt, Join::Miter), Geo::Line(p(-100.0, -60.0), p(100.0, 55.5)));
    let mut st = style(0xC020_40C0, 0.8, Cap::Butt, Join::Bevel);
    st.unit = Unit::Drawing;
    d.add(st, Geo::Poly(vec![p(-40.0, -40.0), p(40.0, -40.0), p(40.0, 30.0), p(-40.0, 30.0)], true));
    d
}

/// the camera of `sample` around a point
pub fn sample_camera(at: [f64; 2]) -> Camera {
    let mut c = Camera::new([at[0] + 0.25, at[1] - 0.5], 0.9);
    c.rotation = 0.3;
    c
}

/// two images of the same drawing: pixels that differ, largest difference
pub fn differ(a: &[u8], b: &[u8]) -> (u32, u8) {
    let (mut n, mut m) = (0u32, 0u8);
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if a[..3] != b[..3] {
            n += 1;
            for k in 0..3 {
                m = m.max(a[k].abs_diff(b[k]));
            }
        }
    }
    (n, m)
}

pub const DEEP_SIZE: (u32, u32) = (200, 160);
pub const DEEP_STEPS: u32 = 24;
pub const DEEP_AT: [f64; 2] = [-9_876_543.21, 6_543_210.987];
pub const DEEP_K: f64 = 1e-6;

/// (a, b): lines through the view, short ones (ends inside), long ones (ends 0.3 and 1e7 units away)
pub fn deep_lines() -> [([f64; 2], [f64; 2]); 6] {
    let (at, k) = (DEEP_AT, DEEP_K);
    [
        ([at[0] - 40.0 * k, at[1] - 50.0 * k], [at[0] - 35.0 * k, at[1] + 55.0 * k]),
        ([at[0] + 20.3 * k, at[1] - 0.3], [at[0] + 32.9 * k, at[1] + 0.3]),
        ([at[0] + 61.0 * k - 1.0e7 * 0.02, at[1] - 1.0e7], [at[0] + 61.0 * k + 1.0e7 * 0.02, at[1] + 1.0e7]),
        ([at[0] - 0.7, at[1] + 30.25 * k - 0.7 * 0.03], [at[0] + 0.7, at[1] + 30.25 * k + 0.7 * 0.03]),
        ([at[0] - 1.0e7, at[1] - 41.6 * k - 0.005], [at[0] + 1.0e7, at[1] - 41.6 * k + 0.005]),
        ([at[0] - 70.0 * k, at[1] + 5.0 * k], [at[0] - 10.0 * k, at[1] + 62.0 * k]),
    ]
}

/// white lines on black, 2.5 px wide
pub fn deep_drawing() -> Drawing {
    let mut d = Drawing::one_layer();
    for (a, b) in deep_lines() {
        d.add(style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter), Geo::Line(a, b));
    }
    d
}

/// pans of 0.37 px and of a few pixels, with a rotation now and then
pub fn deep_camera(step: u32) -> Camera {
    let mut c = Camera::new([DEEP_AT[0] + step as f64 * 0.37 * DEEP_K, DEEP_AT[1] - step as f64 * 0.23 * DEEP_K], DEEP_K);
    c.rotation = if step >= 16 { 0.25 } else { 0.0 };
    c
}

/// what the deep zoom found over its frames
#[derive(Clone, Copy, Default)]
pub struct DeepStats {
    pub worst: f64,
    pub checks: u32,
    pub lost: u32,
    pub per_line: [f64; 6],
}

impl DeepStats {
    pub fn fine(&self) -> bool {
        self.checks > 300 && self.lost == 0 && self.worst <= 1.0 / 16.0
    }

    pub fn text(&self) -> String {
        format!("1e-6 units per pixel near ({}, {}), 24 pans of 0.37 px and a rotation, six lines up to 2e7 long: {} rows, the largest distance from where the line belongs {:.4} px \
                 (1/16 = 0.0625; per line {}), {} rows without the line", DEEP_AT[0], DEEP_AT[1], self.checks, self.worst,
                self.per_line.iter().map(|v| format!("{v:.4}")).collect::<Vec<_>>().join(", "), self.lost)
    }
}

/// the share of a pixel inside a half plane whose edge lies s from its centre (outward unit normal m), in f64: what
/// stroke.wgsl's cov() computes
fn box_cov(s: f64, m: [f64; 2]) -> f64 {
    let (a, b) = (m[0].abs().min(m[1].abs()), m[0].abs().max(m[1].abs()));
    let x = -s;
    let outer = 0.5 * (a + b);
    if x >= outer {
        return 1.0;
    }
    if x <= -outer {
        return 0.0;
    }
    if a < 1e-9 {
        return (0.5 + x / b).clamp(0.0, 1.0);
    }
    let inner = 0.5 * (b - a);
    if x < -inner {
        let t = x + outer;
        return t * t / (2.0 * a * b);
    }
    if x > inner {
        let t = outer - x;
        return 1.0 - t * t / (2.0 * a * b);
    }
    0.5 + x / b
}

/// one frame of the deep zoom: each line's place from the coverage of its rows against where it belongs (f64)
pub fn deep_judge(img: &[u8], c: &Camera, st: &mut DeepStats) {
    let (dw, dh) = DEEP_SIZE;
    let lines = deep_lines();
    let cov = |x: usize, y: usize| img[(y * dw as usize + x) * 4 + 1] as f64 / 255.0;
    for (li, (a, b)) in lines.into_iter().enumerate() {
        let (sa, sb) = (c.to_screen(a, (dw, dh)), c.to_screen(b, (dw, dh)));
        let vertical = (sb[1] - sa[1]).abs() > (sb[0] - sa[0]).abs();
        // rows (or columns) inside the view and away from the ends
        for j in (12..(if vertical { dh } else { dw }) as usize - 12).step_by(9) {
            let y = j as f64 + 0.5;
            let (u0, v0, u1, v1) = if vertical { (sa[0], sa[1], sb[0], sb[1]) } else { (sa[1], sa[0], sb[1], sb[0]) };
            // (between the ends, 3 px away from them: a butt end crosses its last rows)
            if (y - v0) * (y - v1) > 0.0 || (y - v0).abs() < 3.0 || (y - v1).abs() < 3.0 {
                continue;
            }
            let want = u0 + (y - v0) * (u1 - u0) / (v1 - v0);
            let lim = if vertical { dw } else { dh } as f64;
            if want < 8.0 || want > lim - 8.0 {
                continue;
            }
            // (no other line within 3 px of the window: it would pull the centroid)
            let near = lines.iter().enumerate().any(|(lj, &(oa, ob))| {
                if lj == li {
                    return false;
                }
                let (pa, pb) = (c.to_screen(oa, (dw, dh)), c.to_screen(ob, (dw, dh)));
                let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
                let l = sqrt(dx * dx + dy * dy);
                (-6..=6).any(|o| {
                    let (x, y) = if vertical { (want + o as f64, y) } else { (y, want + o as f64) };
                    ((x - pa[0]) * dy - (y - pa[1]) * dx).abs() / l < 1.25 + 3.0
                })
            });
            if near {
                continue;
            }
            // the centroid of the window's coverage, against that of the exact coverage of the line there (the strip's
            // share of each pixel in f64, as cov() in stroke.wgsl: binning the coverage biases a centroid by up to
            // 0.05 px, the same for both)
            let (dx, dy) = (sb[0] - sa[0], sb[1] - sa[1]);
            let l = sqrt(dx * dx + dy * dy);
            let n = [-dy / l, dx / l];
            let (mut sum, mut mom, mut esum, mut emom) = (0.0, 0.0, 0.0, 0.0);
            for i in (want as i64 - 6).max(0) as usize..((want as i64 + 7) as usize).min(lim as usize) {
                let (px, py) = if vertical { (i, j) } else { (j, i) };
                let c = cov(px, py);
                sum += c;
                mom += c * (i as f64 + 0.5);
                let v = (px as f64 + 0.5 - sa[0]) * n[0] + (py as f64 + 0.5 - sa[1]) * n[1];
                let e = (box_cov(v - 1.25, n) + box_cov(-v - 1.25, n) - 1.0).max(0.0);
                esum += e;
                emom += e * (i as f64 + 0.5);
            }
            st.checks += 1;
            if sum < 1.0 {
                st.lost += 1;
                continue;
            }
            let want = emom / esum;
            st.worst = st.worst.max((mom / sum - want).abs());
            st.per_line[li] = st.per_line[li].max((mom / sum - want).abs());
        }
    }
}

// ---------------------------------------------------------------- visibility

/// what the CPU's exact test says of a record
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seen {
    Yes,
    Doubt,
    No,
}

/// cull.wgsl's test in f64: the box around the segment, grown by the stroke's reach (a miter's bound only where the
/// record has a neighbour: joined); the GPU adds a margin for f32's rounding, within it a record may go either way
pub fn exact(a: [f64; 2], b: [f64; 2], st: &Style, joined: bool, cam: &Camera, size: (u32, u32)) -> Seen {
    exact_grown(a, b, st, joined, 0.0, cam, size)
}

/// exact with the reach grown by `extra` px
#[allow(clippy::too_many_arguments)]
pub fn exact_grown(a: [f64; 2], b: [f64; 2], st: &Style, joined: bool, extra: f64, cam: &Camera, size: (u32, u32)) -> Seen {
    let (s, c) = sincos(cam.rotation);
    let px = |p: [f64; 2]| {
        let (x, y) = (p[0] - cam.center[0], p[1] - cam.center[1]);
        [(c * x - s * y) / cam.scale, -(s * x + c * y) / cam.scale]
    };
    let (p0, p1) = (px(a), px(b));
    let wpx = match st.unit {
        Unit::Drawing => st.width as f64 / cam.scale,
        _ => st.width as f64,
    };
    let hw = (0.5 * wpx).max(0.5);
    let bound = if st.join == Join::Miter && joined { sqrt((st.miter_limit as f64 * st.miter_limit as f64 - 1.0).max(0.0)).max(1.0) } else { 1.0 };
    let reach = hw * bound + extra + 1.0;
    let len0 = |v: [f64; 2]| sqrt(v[0] * v[0] + v[1] * v[1]);
    let margin = 1e-3 + 4e-6 * (len0(p0) + len0(p1));
    let half = [0.5 * size.0 as f64, 0.5 * size.1 as f64];
    let dv = [p1[0] - p0[0], p1[1] - p0[1]];
    let len = len0(dv);
    let d = if len > 0.0 { [dv[0] / len, dv[1] / len] } else { [1.0, 0.0] };
    let n = [-d[1], d[0]];
    let cc = [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])];
    let e = 0.5 * len + reach;
    let dot = |u: [f64; 2], v: [f64; 2]| u[0] * v[0] + u[1] * v[1];
    let sep = [
        cc[0].abs() - (half[0] + e * d[0].abs() + reach * n[0].abs()),
        cc[1].abs() - (half[1] + e * d[1].abs() + reach * n[1].abs()),
        dot(cc, d).abs() - (e + half[0] * d[0].abs() + half[1] * d[1].abs()),
        dot(cc, n).abs() - (reach + half[0] * n[0].abs() + half[1] * n[1].abs()),
    ];
    let m = sep.iter().fold(f64::MIN, |a, &b| a.max(b));
    if m <= 0.0 {
        Seen::Yes
    } else if m > 2.0 * margin + 0.01 {
        Seen::No
    } else {
        Seen::Doubt
    }
}

/// an entity of a random scene of the visibility test: its id, points, closed, style; or an arc (centre, radius, start,
/// sweep)
pub struct VisEnt {
    pub id: EntityId,
    pub pts: Vec<[f64; 2]>,
    pub closed: bool,
    pub style: Style,
    pub arc: Option<([f64; 2], f64, f64, f64)>,
}

pub const VIS_SIZE: (u32, u32) = (200, 150);
pub const VIS_CASES: [(u32, u32, f64); 4] = [(1, 5000, 600.0), (2, 20000, 3000.0), (3, 3000, 250.0), (4, 8000, 1.0e6)];

/// a random scene of the visibility test (fixed seed): rotated, three layers, 12 styles, every 17th entity removed
pub fn vis_scene(r: &Renderer, seed: u32, n: u32, spread: f64) -> Result<(Scene, Camera, Vec<VisEnt>)> {
    let mut rng = Rng(seed);
    let mut s = Scene::new(r)?;
    let layers: Vec<_> = (0..3).map(|k| s.add_layer(&Layer::new(k as i32))).collect::<Result<Vec<_>>>()?;
    let mut cam = Camera::new([rng.range(-100.0, 100.0), rng.range(-100.0, 100.0)], rng.range(0.4, 2.5) * spread / 600.0);
    cam.rotation = rng.range(-3.0, 3.0);
    s.set_camera(&cam)?;
    let mut styles = Vec::new();
    for k in 0..12 {
        let mut st = style(0xFF00_0000 | rng.next(), rng.range(0.0, 12.0) as f32, [Cap::Butt, Cap::Round, Cap::Square][k % 3], [Join::Miter, Join::Bevel, Join::Round][(k / 3) % 3]);
        if k % 4 == 3 {
            st.unit = Unit::Drawing;
            st.width = (rng.range(0.0, 0.02) * spread) as f32;
        }
        st.miter_limit = rng.range(1.0, 12.0) as f32;
        styles.push((s.add_style(&st)?, st));
    }
    let mut ents = Vec::new();
    for _ in 0..n {
        let a = [cam.center[0] + rng.range(-spread, spread), cam.center[1] + rng.range(-spread, spread)];
        let m = 2 + rng.below(4) as usize;
        let mut pts = vec![a];
        for _ in 1..m {
            let q = pts[pts.len() - 1];
            let step = spread * 0.05;
            pts.push([q[0] + rng.range(-step, step), q[1] + rng.range(-step, step)]);
        }
        let (sid, st) = styles[rng.below(12) as usize];
        let closed = rng.below(5) == 0;
        let layer = layers[rng.below(3) as usize];
        if ents.len() % 6 == 5 {
            // an arc (every twelfth a circle) of up to a tenth of the spread
            let r = rng.range(0.001, 0.1) * spread;
            let a0 = rng.range(-4.0, 4.0);
            let sw = if ents.len() % 12 == 11 { core::f64::consts::TAU } else { rng.range(-6.0, 6.0) };
            let id = s.insert(layer, sid, &Shape::Arc { center: a, radius: r, start: a0, sweep: sw })?;
            ents.push(VisEnt { id, pts: Vec::new(), closed: false, style: st, arc: Some((a, r, a0, sw)) });
            continue;
        }
        let id = s.insert(layer, sid, &Shape::Polyline { points: &pts, closed })?;
        ents.push(VisEnt { id, pts, closed, style: st, arc: None });
    }
    // a few removed: their places must not show up
    for k in (0..ents.len()).step_by(17) {
        s.remove(ents[k].id)?;
    }
    Ok((s, cam, ents))
}

/// the visible lists of a frame against the exact test on the CPU
#[derive(Clone, Copy, Default, Debug)]
pub struct VisResult {
    pub visible: usize,
    pub yes: u32,
    pub doubt: u32,
    pub missing: u32,
    pub extra: u32,
    pub twice: usize,
    pub foreign: usize,
    pub unordered: usize,
}

impl VisResult {
    pub fn fine(&self) -> bool {
        self.missing == 0 && self.extra == 0 && self.twice == 0 && self.foreign == 0 && self.unordered == 0
    }

    pub fn text(&self) -> String {
        format!("GPU {} visible, CPU {} (+{} within a hair of an edge): {} missing, {} too many, {} doubled, {} free or unknown, {} out of order", self.visible, self.yes, self.doubt, self.missing,
                self.extra, self.twice, self.foreign, self.unordered)
    }
}

/// lists: per visible layer (first, count, ..); vis: the visible list.  None: an entity of the wrong size
pub fn vis_judge(s: &Scene, lists: &[[u32; 4]], vis: &[u32], ents: &[VisEnt], cam: &Camera, size: (u32, u32)) -> Option<VisResult> {
    let mut seen = Vec::new();
    let mut res = VisResult::default();
    for l in lists {
        let part = &vis[l[0] as usize..(l[0] + l[1]) as usize];
        res.unordered += part.windows(2).filter(|w| w[0] >= w[1]).count();
        seen.extend_from_slice(part);
    }
    seen.sort_unstable();
    res.twice = seen.windows(2).filter(|w| w[0] == w[1]).count();
    res.visible = seen.len();
    let mut known = Vec::new();
    for (k, e) in ents.iter().enumerate() {
        if k % 17 == 0 {
            continue;
        }
        let (first, count) = s.locate(e.id)?;
        if let Some((c, r, a0, sw)) = e.arc {
            // an arc: its head (never drawn: unknown if listed) and its pieces, their ends as scene.rs computes them
            let n = ((sw.abs() / render2d::ARC_STEP - 1e-9).max(0.0) as u32 + 1).min(16);
            if n + 1 != count {
                return None;
            }
            let sw = if sw.abs() >= core::f64::consts::TAU { core::f64::consts::TAU.copysign(sw) } else { sw };
            let at = |t: f64| {
                let (sn, co) = sincos(t);
                [c[0] + r * co, c[1] + r * sn]
            };
            for j in 0..n {
                let pa = at(a0 + sw * j as f64 / n as f64);
                let pb = if sw.abs() >= core::f64::consts::TAU && j + 1 == n { at(a0) } else { at(a0 + sw * (j + 1) as f64 / n as f64) };
                let v = exact_arc(pa, pb, r, &e.style, cam, size);
                let place = first + 1 + j;
                known.push(place);
                let got = seen.binary_search(&place).is_ok();
                match v {
                    Seen::Yes => {
                        res.yes += 1;
                        res.missing += !got as u32;
                    }
                    Seen::Doubt => res.doubt += 1,
                    Seen::No => res.extra += got as u32,
                }
            }
            continue;
        }
        let m = e.pts.len();
        let segs = if e.closed { m } else { m - 1 };
        if segs as u32 != count {
            return None;
        }
        for j in 0..segs {
            let v = exact(e.pts[j], e.pts[(j + 1) % m], &e.style, segs > 1, cam, size);
            let place = first + j as u32;
            known.push(place);
            let got = seen.binary_search(&place).is_ok();
            match v {
                Seen::Yes => {
                    res.yes += 1;
                    res.missing += !got as u32;
                }
                Seen::Doubt => res.doubt += 1,
                Seen::No => res.extra += got as u32,
            }
        }
    }
    known.sort_unstable();
    res.foreign = seen.iter().filter(|p| known.binary_search(p).is_err()).count();
    Some(res)
}

// ---------------------------------------------------------------- arcs and dashes at large coordinates

/// arcs, circles and dashed strokes around a point (coords: the same at the origin and far away)
pub fn arc_sample(at: [f64; 2]) -> Drawing {
    let mut d = Drawing::one_layer();
    let p = |x: f64, y: f64| [at[0] + x, at[1] + y];
    d.add(style(0xFF20_3040, 5.0, Cap::Round, Join::Round), Geo::Arc(p(-30.0, -10.0), 45.0, 0.4, 2.2));
    d.add(style(0x9030_8040, 9.0, Cap::Square, Join::Miter), Geo::Circle(p(40.0, 10.0), 28.0));
    d.add(dashed(0xFF80_2020, 2.0, Cap::Butt, Join::Miter, &[9.0, 4.0], Unit::Px, 1.5), Geo::Circle(p(-50.0, 20.0), 30.0));
    d.add(dashed(0xC020_40C0, 0.9, Cap::Round, Join::Round, &[7.0, 3.0, 1.0, 3.0], Unit::Drawing, 2.0),
          Geo::Poly(vec![p(-90.0, -50.0), p(-20.0, -45.0), p(10.0, 40.0), p(80.0, -40.0)], false));
    d
}

/// circles at the deep zoom (DEEP_AT, 1e-6 units per pixel): their centres and radii, all passing through the view: one
/// of radius 1e7 across it (its top in the view), one of 1e7 at a slant, one of 3e5, a small one of 40 px, and an open
/// arc of 1e7 whose ends lie far away
pub fn deep_circles() -> [([f64; 2], f64); 5] {
    let (at, k) = (DEEP_AT, DEEP_K);
    let r = 1.0e7;
    let s = 0.6;
    let c = sqrt(1.0 - s * s);
    [
        ([at[0] + 10.0 * k, at[1] + 50.25 * k - r], r),
        ([at[0] - 60.0 * k + r * c, at[1] - 10.0 * k - r * s], r),
        ([at[0] + 40.0 * k - 3.0e5 * s, at[1] - 45.0 * k + 3.0e5 * c], 3.0e5),
        ([at[0] - 30.0 * k, at[1] + 10.0 * k], 33.3 * k),
        ([at[0] + 70.0 * k, at[1] - 30.0 * k - r], r),
    ]
}

/// white rings on black, 2.5 px wide (the last an arc of 0.2 rad, its ends 1e6 units away)
pub fn deep_arc_drawing() -> Drawing {
    let mut d = Drawing::one_layer();
    for (k, (c, r)) in deep_circles().into_iter().enumerate() {
        let st = style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter);
        if k == 4 {
            d.add(st, Geo::Arc(c, r, core::f64::consts::FRAC_PI_2 - 0.1, 0.2));
        } else {
            d.add(st, Geo::Circle(c, r));
        }
    }
    d
}

/// the share of a pixel at screen point q in the ring (half width h) of a circle (c, r): what fs_arc computes, in f64 (the
/// ring cut by its tangents at the pixel)
fn ring_cov(q: [f64; 2], c: [f64; 2], r: f64, h: f64, cam: &Camera, size: (u32, u32)) -> f64 {
    let p = cam.to_drawing(q, size);
    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
    let l = sqrt(dx * dx + dy * dy);
    let dist = (l - r) / cam.scale;
    let (s, co) = sincos(cam.rotation);
    let m = if l > 0.0 { [(co * dx - s * dy) / l, -(s * dx + co * dy) / l] } else { [1.0, 0.0] };
    (box_cov(dist - h, m) + box_cov(-dist - h, [-m[0], -m[1]]) - 1.0).max(0.0)
}

/// one frame of the deep zoom of arcs: each ring's place from the coverage of rows and columns that cross it (at right
/// angles within 45 degrees) against where it belongs (the centroid of ring_cov in f64)
pub fn deep_arc_judge(img: &[u8], cam: &Camera, st: &mut DeepStats) {
    let (dw, dh) = DEEP_SIZE;
    let circles = deep_circles();
    let h = 1.25;
    let cov = |x: usize, y: usize| img[(y * dw as usize + x) * 4 + 1] as f64 / 255.0;
    // the signed distance (px) of a screen point from a circle
    let dist = |q: [f64; 2], c: [f64; 2], r: f64| {
        let p = cam.to_drawing(q, (dw, dh));
        let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
        (sqrt(dx * dx + dy * dy) - r) / cam.scale
    };
    for (ci, &(c, r)) in circles.iter().enumerate() {
        for rows in [true, false] {
            let (n_lines, n_along) = if rows { (dh, dw) } else { (dw, dh) };
            for j in (12..n_lines as usize - 12).step_by(9) {
                let at = |i: f64| if rows { [i, j as f64 + 0.5] } else { [j as f64 + 0.5, i] };
                // the crossings along this line: sign changes of the distance, refined by halving
                let mut prev = dist(at(0.5), c, r);
                for i in 1..n_along as usize {
                    let x1 = i as f64 + 0.5;
                    let cur = dist(at(x1), c, r);
                    if (prev < 0.0) == (cur < 0.0) {
                        prev = cur;
                        continue;
                    }
                    let (mut lo, mut hi) = (x1 - 1.0, x1);
                    for _ in 0..50 {
                        let mid = 0.5 * (lo + hi);
                        if (dist(at(mid), c, r) < 0.0) == (prev < 0.0) {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    prev = cur;
                    let want = 0.5 * (lo + hi);
                    if want < 10.0 || want > n_along as f64 - 10.0 {
                        continue;
                    }
                    // (crossed at right angles within 45 degrees: the ring's direction from the distance's slope)
                    let slope = (dist(at(want + 0.01), c, r) - dist(at(want - 0.01), c, r)) / 0.02;
                    if slope.abs() < 0.707 {
                        continue;
                    }
                    // (an arc: away from its ends; no other ring within 4 px of the window)
                    let q = at(want);
                    if ci == 4 {
                        let p = cam.to_drawing(q, (dw, dh));
                        let a = (p[1] - c[1]).atan2_approx(p[0] - c[0]);
                        if (a - core::f64::consts::FRAC_PI_2).abs() > 0.09 {
                            continue;
                        }
                    }
                    let crowded = circles.iter().enumerate().any(|(cj, &(c2, r2))| cj != ci && (-7..=7).any(|o| dist(at(want + o as f64), c2, r2).abs() < h + 4.0));
                    if crowded {
                        continue;
                    }
                    let (mut sum, mut mom, mut esum, mut emom) = (0.0, 0.0, 0.0, 0.0);
                    for i in (want as i64 - 6).max(0) as usize..((want as i64 + 7) as usize).min(n_along as usize) {
                        let (px, py) = if rows { (i, j) } else { (j, i) };
                        let g = cov(px, py);
                        sum += g;
                        mom += g * (i as f64 + 0.5);
                        let e = ring_cov(at(i as f64 + 0.5), c, r, h, cam, (dw, dh));
                        esum += e;
                        emom += e * (i as f64 + 0.5);
                    }
                    st.checks += 1;
                    if sum < 1.0 {
                        st.lost += 1;
                        continue;
                    }
                    let err = (mom / sum - emom / esum).abs();
                    st.worst = st.worst.max(err);
                    st.per_line[ci] = st.per_line[ci].max(err);
                }
            }
        }
    }
}

/// atan2 for the tests (no libm)
trait Atan2 {
    fn atan2_approx(self, x: f64) -> f64;
}

impl Atan2 for f64 {
    fn atan2_approx(self, x: f64) -> f64 {
        let y = self;
        let r = sqrt(x * x + y * y);
        if r == 0.0 {
            return 0.0;
        }
        let a = acos((x / r).clamp(-1.0, 1.0));
        if y < 0.0 { -a } else { a }
    }
}

impl DeepStats {
    pub fn arc_text(&self) -> String {
        format!("arcs at 1e-6 units per pixel near ({}, {}), 24 pans of 0.37 px and a rotation, rings of radius 1e7 (two), 3e5, 33 px and an arc of 1e7: {} crossings, \
                 the largest distance from where the ring belongs {:.4} px (1/16 = 0.0625; per ring {}), {} without the ring", DEEP_AT[0], DEEP_AT[1], self.checks, self.worst,
                self.per_line[..5].iter().map(|v| format!("{v:.4}")).collect::<Vec<_>>().join(", "), self.lost)
    }

    pub fn fine_arcs(&self) -> bool {
        self.checks > 200 && self.lost == 0 && self.worst <= 1.0 / 16.0
    }
}

// ---------------------------------------------------------------- curves, fills and hatches far away (G6b1)

/// curves, a fill with a hole and a hatched fill around a point (coords: the same at the origin and far away)
pub fn curve_sample(at: [f64; 2]) -> Drawing {
    let mut d = Drawing::one_layer();
    let p = |x: f64, y: f64| [at[0] + x, at[1] + y];
    d.add(style(0xFF20_3040, 3.0, Cap::Round, Join::Round), Geo::Bez(vec![p(-90.0, -40.0), p(-40.0, 50.0), p(10.0, -60.0), p(80.0, 30.0)], 3, false));
    d.add(dashed(0xFF80_2020, 2.0, Cap::Butt, Join::Miter, &[8.0, 3.0], Unit::Px, 1.0), Geo::Ellipse(p(20.0, 10.0), [60.0, 25.0], 0.4, 0.0, core::f64::consts::TAU));
    d.add(style(0xC020_40C0, 1.5, Cap::Square, Join::Bevel), Geo::Spline(3, vec![p(-80.0, 40.0), p(-50.0, 70.0), p(-10.0, 30.0), p(30.0, 65.0), p(70.0, 45.0)], vec![], vec![], false));
    d.add(style(0x9030_8040, 0.0, Cap::Butt, Join::Miter), Geo::Fill(vec![ring(&[p(-100.0, -70.0), p(-20.0, -70.0), p(-20.0, -5.0), p(-100.0, -5.0)]), vec![Geo::Circle(p(-60.0, -38.0), 18.0)]]));
    let mut h = render2d::HatchLine::new(0.7, 6.0);
    h.base = p(1.3, -2.1);
    d.add(style(0xFF40_2080, 0.0, Cap::Butt, Join::Miter).hatched(&[h], 1.0), Geo::Fill(vec![vec![Geo::Ellipse(p(55.0, -40.0), [40.0, 22.0], -0.2, 0.0, core::f64::consts::TAU)]]));
    d
}

/// a curve near DEEP_AT at 1e-6 units per pixel, the curve's true points near the view (drawing units, fine): an ellipse of
/// radii 1e7 and 6e6, turned, its point at the angle 0.9 at DEEP_AT; a cubic Bézier whose controls lie 1e6 away, through
/// the view
pub fn deep_curves() -> (Drawing, Vec<Vec<[f64; 2]>>) {
    let (at, k) = (DEEP_AT, DEEP_K);
    let mut d = Drawing::one_layer();
    let st = style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter);
    // the ellipse: centre such that its point at t0 = 0.9 lies at DEEP_AT + (5, 12) px
    let (rx, ry, rot, t0) = (1.0e7, 6.0e6, 0.3, 0.9);
    let (sr, cr) = sincos(rot);
    let ell = |c: [f64; 2], t: f64| {
        let (s, co) = sincos(t);
        let (x, y) = (rx * co, ry * s);
        [c[0] + cr * x - sr * y, c[1] + sr * x + cr * y]
    };
    let o = ell([0.0, 0.0], t0);
    let c = [at[0] + 5.0 * k - o[0], at[1] + 12.0 * k - o[1]];
    d.add(st, Geo::Ellipse(c, [rx, ry], rot, 0.0, core::f64::consts::TAU));
    // (the true points: the angle within +-0.5e-10 of t0, in f64)
    let fine_e: Vec<[f64; 2]> = (0..=400).map(|j| ell(c, t0 + (j as f64 - 200.0) * 2.5e-13)).collect();
    // the Bézier: through DEEP_AT - (30, 20) px at t = 0.5 (its controls 1e6 away), a cubic in x
    let q = [at[0] - 30.0 * k, at[1] - 20.0 * k];
    let ctl = [[q[0] - 1.0e6, q[1] - 3.0e5], [q[0] - 2.0e5, q[1] + 6.0e5], [q[0] + 4.0e5, q[1] - 5.0e5], [q[0] + 1.2e6, q[1] + 2.0e5]];
    // (the curve at 0.5 is the controls' (1, 3, 3, 1) / 8: moved so that it passes through q)
    let mid = [(ctl[0][0] + 3.0 * ctl[1][0] + 3.0 * ctl[2][0] + ctl[3][0]) / 8.0, (ctl[0][1] + 3.0 * ctl[1][1] + 3.0 * ctl[2][1] + ctl[3][1]) / 8.0];
    let ctl: Vec<[f64; 2]> = ctl.iter().map(|p| [p[0] + q[0] - mid[0], p[1] + q[1] - mid[1]]).collect();
    d.add(st, Geo::Bez(ctl.clone(), 3, false));
    let bez = |t: f64| {
        let s = 1.0 - t;
        let (a, b, cc, dd) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
        [a * ctl[0][0] + b * ctl[1][0] + cc * ctl[2][0] + dd * ctl[3][0], a * ctl[0][1] + b * ctl[1][1] + cc * ctl[2][1] + dd * ctl[3][1]]
    };
    let fine_b: Vec<[f64; 2]> = (0..=400).map(|j| bez(0.5 + (j as f64 - 200.0) * 1.0e-12)).collect();
    (d, vec![fine_e, fine_b])
}

/// the signed distance (px) of a screen point from a fine polyline of drawing points
fn fine_dist(q: [f64; 2], line: &[[f64; 2]], cam: &Camera, size: (u32, u32)) -> f64 {
    let p = cam.to_drawing(q, size);
    let mut best = (f64::MAX, 1.0);
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let ll = dx * dx + dy * dy;
        let t = if ll > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / ll).clamp(0.0, 1.0) } else { 0.0 };
        let (ex, ey) = (p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
        let dd = ex * ex + ey * ey;
        if dd < best.0 {
            best = (dd, if dx * (p[1] - a[1]) - dy * (p[0] - a[0]) > 0.0 { 1.0 } else { -1.0 });
        }
    }
    best.1 * sqrt(best.0) / cam.scale
}

/// one frame of the deep zoom of curves: each curve's place from rows and columns that cross it (at right angles within
/// 45 degrees) against where its true points put it (the centroid of the exact strip coverage in f64), as deep_arc_judge
pub fn deep_curve_judge(img: &[u8], cam: &Camera, fine: &[Vec<[f64; 2]>], st: &mut DeepStats) {
    let (dw, dh) = DEEP_SIZE;
    let h = 1.25;
    let cov = |x: usize, y: usize| img[(y * dw as usize + x) * 4 + 1] as f64 / 255.0;
    for (ci, line) in fine.iter().enumerate() {
        let dist = |q: [f64; 2]| fine_dist(q, line, cam, (dw, dh));
        for rows in [true, false] {
            let (n_lines, n_along) = if rows { (dh, dw) } else { (dw, dh) };
            for j in (12..n_lines as usize - 12).step_by(9) {
                let at = |i: f64| if rows { [i, j as f64 + 0.5] } else { [j as f64 + 0.5, i] };
                let mut prev = dist(at(0.5));
                for i in 1..n_along as usize {
                    let x1 = i as f64 + 0.5;
                    let cur = dist(at(x1));
                    if (prev < 0.0) == (cur < 0.0) || (prev.abs() > 4.0 && cur.abs() > 4.0) {
                        prev = cur;
                        continue;
                    }
                    let (mut lo, mut hi) = (x1 - 1.0, x1);
                    for _ in 0..50 {
                        let mid = 0.5 * (lo + hi);
                        if (dist(at(mid)) < 0.0) == (prev < 0.0) {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    prev = cur;
                    let want = 0.5 * (lo + hi);
                    if want < 10.0 || want > n_along as f64 - 10.0 {
                        continue;
                    }
                    let slope = (dist(at(want + 0.01)) - dist(at(want - 0.01))) / 0.02;
                    if slope.abs() < 0.707 {
                        continue;
                    }
                    let crowded = fine.iter().enumerate().any(|(cj, l2)| cj != ci && (-7..=7).any(|o| fine_dist(at(want + o as f64), l2, cam, (dw, dh)).abs() < h + 4.0));
                    if crowded {
                        continue;
                    }
                    let (mut sum, mut mom, mut esum, mut emom) = (0.0, 0.0, 0.0, 0.0);
                    for i in (want as i64 - 6).max(0) as usize..((want as i64 + 7) as usize).min(n_along as usize) {
                        let (px, py) = if rows { (i, j) } else { (j, i) };
                        let g = cov(px, py);
                        sum += g;
                        mom += g * (i as f64 + 0.5);
                        // (the exact strip at the pixel: its distance and the line's normal there in pixels)
                        let q = at(i as f64 + 0.5);
                        let dd = dist(q);
                        let (gx, gy) = ((dist([q[0] + 0.01, q[1]]) - dist([q[0] - 0.01, q[1]])) / 0.02, (dist([q[0], q[1] + 0.01]) - dist([q[0], q[1] - 0.01])) / 0.02);
                        let gl = sqrt(gx * gx + gy * gy).max(1e-12);
                        let m = [gx / gl, gy / gl];
                        let e = (box_cov(dd - h, m) + box_cov(-dd - h, [-m[0], -m[1]]) - 1.0).max(0.0);
                        esum += e;
                        emom += e * (i as f64 + 0.5);
                    }
                    st.checks += 1;
                    if sum < 1.0 {
                        st.lost += 1;
                        continue;
                    }
                    let err = (mom / sum - emom / esum).abs();
                    st.worst = st.worst.max(err);
                    st.per_line[ci] = st.per_line[ci].max(err);
                }
            }
        }
    }
}

/// the hatch at 1e-6 units per pixel near DEEP_AT: a fill of 2e7 by 2e7 units hatched with horizontal lines 25 px apart
/// (base 1e7 away), and its outline's right edge (x = DEEP_AT.x + 31.3 px) in the view, solid or hatched
pub fn deep_hatch(solid: bool) -> Drawing {
    let (at, k) = (DEEP_AT, DEEP_K);
    let mut d = Drawing::one_layer();
    let x1 = at[0] + 31.3 * k;
    let sq = ring(&[[x1 - 2.0e7, at[1] - 1.0e7], [x1, at[1] - 1.0e7], [x1, at[1] + 1.0e7], [x1 - 2.0e7, at[1] + 1.0e7]]);
    let mut h = render2d::HatchLine::new(0.0, 25.0 * k);
    h.base = [at[0] - 1.0e7, at[1] + 3.3 * k];
    let st = if solid { style(0xFFFF_FFFF, 0.0, Cap::Butt, Join::Miter) } else { style(0xFFFF_FFFF, 0.0, Cap::Butt, Join::Miter).hatched(&[h], 2.5) };
    d.add(st, Geo::Fill(vec![sq]));
    d
}

/// one frame of the deep hatch: the lines' places (their centroids per column, left of the edge) and, in the solid frame,
/// the edge's place (per row: the left of a window plus its coverage summed), against f64
pub fn deep_hatch_judge(hatched: &[u8], solid: &[u8], cam: &Camera, lines: &mut DeepStats, edge: &mut DeepStats) {
    let (dw, dh) = DEEP_SIZE;
    let (at, k) = (DEEP_AT, DEEP_K);
    let g = |img: &[u8], x: usize, y: usize| img[(y * dw as usize + x) * 4 + 1] as f64 / 255.0;
    // (a line of the drawing on the screen: where it crosses a column or a row, the camera turned or not)
    let cross = |a: [f64; 2], b: [f64; 2], along: f64, rows: bool| {
        let (sa, sb) = (cam.to_screen(a, (dw, dh)), cam.to_screen(b, (dw, dh)));
        let (i, o) = if rows { (1, 0) } else { (0, 1) };
        sa[o] + (along - sa[i]) * (sb[o] - sa[o]) / (sb[i] - sa[i])
    };
    let xe = at[0] + 31.3 * k;
    let ex = cross([xe, at[1] - 50.0 * k], [xe, at[1] + 50.0 * k], 0.5 * dh as f64, true);
    // the lines: y of line j where base.y + j 25 k crosses the view
    for col in (10..(ex as usize).saturating_sub(12)).step_by(7) {
        for j in -10i64..=10 {
            let yl = at[1] + 3.3 * k + j as f64 * 25.0 * k;
            let want = cross([at[0] - 50.0 * k, yl], [at[0] + 50.0 * k, yl], col as f64 + 0.5, false);
            // (away from the outline, which the camera's turn tilts)
            if !(want > 8.0 && want < dh as f64 - 8.0) || col as f64 + 0.5 > cross([xe, at[1] - 50.0 * k], [xe, at[1] + 50.0 * k], want, true) - 8.0 {
                continue;
            }
            // (the line's strip, 2.5 px wide, on the screen: its normal; the exact share of each pixel, as deep_judge)
            let (sa, sb) = (cam.to_screen([at[0] - 50.0 * k, yl], (dw, dh)), cam.to_screen([at[0] + 50.0 * k, yl], (dw, dh)));
            let (dx, dy) = (sb[0] - sa[0], sb[1] - sa[1]);
            let l = sqrt(dx * dx + dy * dy);
            let m = [-dy / l, dx / l];
            let (mut sum, mut mom, mut esum, mut emom) = (0.0, 0.0, 0.0, 0.0);
            for y in (want as i64 - 5).max(0) as usize..((want as i64 + 6) as usize).min(dh as usize) {
                let c = g(hatched, col, y);
                sum += c;
                mom += c * (y as f64 + 0.5);
                let v = (col as f64 + 0.5 - sa[0]) * m[0] + (y as f64 + 0.5 - sa[1]) * m[1];
                let e = (box_cov(v - 1.25, m) + box_cov(-v - 1.25, [-m[0], -m[1]]) - 1.0).max(0.0);
                esum += e;
                emom += e * (y as f64 + 0.5);
            }
            lines.checks += 1;
            if sum < 1.0 {
                lines.lost += 1;
                continue;
            }
            let e = (mom / sum - emom / esum).abs();
            lines.worst = lines.worst.max(e);
            lines.per_line[0] = lines.per_line[0].max(e);
        }
    }
    // the edge: per row, the left of a window of 16 px around it plus the window's coverage
    if ex > 20.0 && ex < dw as f64 - 20.0 {
        for row in (8..dh as usize - 8).step_by(5) {
            let exr = cross([xe, at[1] - 50.0 * k], [xe, at[1] + 50.0 * k], row as f64 + 0.5, true);
            let x0 = exr as usize - 8;
            let s: f64 = (x0..x0 + 16).map(|x| g(solid, x, row)).sum();
            edge.checks += 1;
            let e = (x0 as f64 + s - exr).abs();
            edge.worst = edge.worst.max(e);
            edge.per_line[1] = edge.per_line[1].max(e);
        }
    }
}

// ---------------------------------------------------------------- the dash pattern's place far away

/// the cases of the phase test: a polyline at DEEP_AT whose vertex lies in the view, its segments 1e7 units long, looked at
/// at its vertex and in the middle of its first segment, the pattern in pixels and in drawing units (also one of a period
/// of a million pixels), and a dashed circle of radius 1e7 at a joint of its pieces and in the middle of a piece
pub const PHASE_SIZE: (u32, u32) = (200, 120);
pub const PHASE_CASES: u32 = 7;

/// a case's drawing, its camera at pan `step`, whether its stroke is an arc, its pattern (dash units) and phase
pub struct PhaseCase {
    pub name: &'static str,
    pub d: Drawing,
    pub cam: Camera,
}

const PHASE_K: f64 = 1e-6;

fn phase_vertex() -> [f64; 2] {
    [DEEP_AT[0] + 30.0 * PHASE_K, DEEP_AT[1] - 20.0 * PHASE_K]
}

/// the polyline of the phase test: from 1e7 - 0.123 units left of the vertex to it, then 1e7 up
fn phase_poly() -> Vec<[f64; 2]> {
    let v = phase_vertex();
    vec![[v[0] - (1.0e7 - 0.123), v[1]], v, [v[0], v[1] + 1.0e7]]
}

/// the circle of the phase test: radius 1e7, its top at the vertex
fn phase_circle() -> ([f64; 2], f64) {
    let v = phase_vertex();
    ([v[0], v[1] - 1.0e7], 1.0e7)
}

pub fn phase_case(k: u32, step: u32) -> PhaseCase {
    let v = phase_vertex();
    let (pat, unit, phase, name): (&[f32], Unit, f32, &str) = match k {
        0 | 1 => (&[6.0, 4.0], Unit::Px, 0.3, if k == 0 { "a polyline's vertex, pattern in px" } else { "the middle of a segment 1e7 long, pattern in px" }),
        2 | 3 => (&[6.0e-6, 4.0e-6], Unit::Drawing, 0.3e-6, if k == 2 { "a polyline's vertex, pattern in units" } else { "the middle of a segment 1e7 long, pattern in units" }),
        4 => (&[0.4, 0.6], Unit::Drawing, 0.0, "a period of a million pixels"),
        _ => (&[6.0, 4.0], Unit::Px, 0.3, if k == 5 { "a circle of radius 1e7 at a joint of its pieces" } else { "a circle of radius 1e7 in the middle of a piece" }),
    };
    let st = dashed(0xFFFF_FFFF, 2.0, Cap::Butt, Join::Miter, pat, unit, phase);
    let mut d = Drawing::one_layer();
    let centre = match k {
        0 | 2 => [v[0] - 40.0 * PHASE_K, v[1] + 25.0 * PHASE_K],
        1 | 3 => [v[0] - 5.0e6 + 0.37, v[1]],
        4 => {
            // (where a boundary of the pattern lies: a length of a whole period plus 0.4 from the start, near the middle)
            let l1 = 1.0e7 - 0.123;
            let l = 5.0e6 + 0.4;
            [v[0] - l1 + l + 37.0 * PHASE_K, v[1]]
        }
        5 => v,
        _ => {
            let (c, r) = phase_circle();
            let (s, co) = sincos(core::f64::consts::FRAC_PI_2 + core::f64::consts::PI / 16.0);
            [c[0] + r * co, c[1] + r * s]
        }
    };
    if k < 5 {
        d.add(st, Geo::Poly(phase_poly(), false));
    } else {
        let (c, r) = phase_circle();
        d.add(st, Geo::Circle(c, r));
    }
    let mut cam = Camera::new([centre[0] + step as f64 * 0.37 * PHASE_K, centre[1] - step as f64 * 0.11 * PHASE_K], PHASE_K);
    if k == 6 {
        // (a circle's direction there: the camera turned with it, so that the ring runs along the rows)
        cam.rotation = -core::f64::consts::PI / 16.0;
    }
    PhaseCase { name, d, cam }
}

fn floor_f(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// the dashes' place in a frame of a phase case: along each straight stretch of the stroke in the view (rows or columns
/// at right angles to it, 3 px away from the vertex), the share of the stroke's width a pixel line holds against the
/// pattern's share on over that pixel, the pattern's place from the length along the stroke in f64 (as the CPU puts it
/// in the records and the style): their largest difference is how far a dash's end lies from where it belongs (px)
pub fn phase_judge(img: &[u8], case: &PhaseCase, k: u32, worst: &mut f64, checks: &mut u32) {
    let (w, h) = PHASE_SIZE;
    let cam = &case.cam;
    let st = &case.d.items[0].style;
    // the pattern as the style puts it (scene.rs Style::words): its entries' ends from the period's start as f32, the period
    // the last of them, its periods per drawing unit in f64, the phase in periods as f32; the place at a length l along the
    // stroke frac(l k + p0) periods (dash units: times the period).  (At 1e12 periods the twelfth digit of the period decides
    // where a dash lies: the same numbers, or none of it would agree)
    let n = st.dashes as usize;
    let m = if n % 2 == 1 { 2 * n } else { n };
    let mut ends = Vec::with_capacity(m);
    let mut sum = 0.0f64;
    for k in 0..m {
        sum += st.dash[k % n] as f64;
        ends.push(sum as f32 as f64);
    }
    let period = ends[m - 1];
    let kappa = 1.0 / (period * if st.dash_unit == Unit::Px { cam.scale } else { 1.0 });
    let p0 = {
        let v = st.dash_phase as f64 / period;
        (v - floor_f(v)) as f32 as f64
    };
    let place = |l: f64| {
        let v = l * kappa + p0;
        (v - floor_f(v)) * period
    };
    // the share on over [a, b] of the period (0 <= a <= b < 2 periods)
    let share = |a: f64, b: f64| -> f64 {
        let on_to = |x: f64| -> f64 {
            let (mut s, mut c0, mut x) = (0.0, 0.0, x);
            if x >= period {
                s += (0..m).step_by(2).map(|k| ends[k] - if k == 0 { 0.0 } else { ends[k - 1] }).sum::<f64>();
                x -= period;
            }
            for (k, &c1) in ends.iter().enumerate() {
                if k % 2 == 0 {
                    s += (x.min(c1) - c0).max(0.0);
                }
                c0 = c1;
            }
            s
        };
        if b > a { (on_to(b) - on_to(a)) / (b - a) } else { 0.0 }
    };
    let g = |x: usize, y: usize| img[(y * w as usize + x) * 4 + 1] as f64 / 255.0;
    let pts = phase_poly();
    let l1 = sqrt((pts[1][0] - pts[0][0]) * (pts[1][0] - pts[0][0]) + (pts[1][1] - pts[0][1]) * (pts[1][1] - pts[0][1]));
    let sv = cam.to_screen(pts[1], (w, h));
    // the stretches: horizontal or not, where the stroke lies across them (px), the length along the stroke at a pixel
    // line's two sides
    let mut stretches: Vec<(bool, f64)> = Vec::new();
    if k < 5 {
        stretches.push((true, sv[1]));
        stretches.push((false, sv[0]));
    } else {
        // (the ring's row: where it crosses the view's middle, panned with the camera)
        let (c, r) = phase_circle();
        let a0 = core::f64::consts::FRAC_PI_2 + if k == 6 { core::f64::consts::PI / 16.0 } else { 0.0 };
        let (s0, c0) = sincos(a0);
        stretches.push((true, cam.to_screen([c[0] + r * c0, c[1] + r * s0], (w, h))[1]));
    }
    for (horiz, across) in stretches {
        let span = if horiz { h } else { w } as f64;
        if !(across > 4.0 && across < span - 4.0) {
            continue;
        }
        let len_at = |x: f64| -> Option<f64> {
            if k >= 5 {
                // r times the angle, measured from the camera centre's own angle (exact) plus the small rest
                let (c, r) = phase_circle();
                let a0 = core::f64::consts::FRAC_PI_2 + if k == 6 { core::f64::consts::PI / 16.0 } else { 0.0 };
                let (s0, c0) = sincos(a0);
                let p = cam.to_drawing([x, across], (w, h));
                let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                let l = sqrt(dx * dx + dy * dy);
                let sn = (c0 * dy - s0 * dx) / l;
                return Some(r * (a0 + sn + sn * sn * sn / 6.0));
            }
            // (the first segment runs to the right, the second upwards: from the vertex's place on the screen)
            let along = if horiz { sv[0] } else { sv[1] };
            if x > along - 3.0 {
                return None;
            }
            Some(if horiz { l1 - (along - x) * cam.scale } else { l1 + (along - x) * cam.scale })
        };
        for i in 0..(if horiz { w } else { h }) as usize {
            let (Some(la), Some(lb)) = (len_at(i as f64), len_at(i as f64 + 1.0)) else { continue };
            let (la, lb) = if la <= lb { (la, lb) } else { (lb, la) };
            let (pa, mut pb) = (place(la), place(lb));
            if pb < pa {
                pb += period;
            }
            let want = share(pa, pb);
            let mut got = 0.0;
            for j in (across - 3.0) as usize..(across + 4.0) as usize {
                got += if horiz { g(i, j) } else { g(j, i) };
            }
            got /= 2.0;
            *checks += 1;
            *worst = worst.max((got - want).abs());
        }
    }
}

// ---------------------------------------------------------------- visibility of arcs

/// cull.wgsl's test of an arc's piece in f64: the box around its chord (from a to b), grown by the stroke's reach and the
/// piece's sagitta (r the radius); Doubt within the GPU's margin for f32
pub fn exact_arc(a: [f64; 2], b: [f64; 2], r: f64, st: &Style, cam: &Camera, size: (u32, u32)) -> Seen {
    let px = |p: [f64; 2]| {
        let s = cam.to_screen(p, size);
        [s[0] - 0.5 * size.0 as f64, s[1] - 0.5 * size.1 as f64]
    };
    let (p0, p1) = (px(a), px(b));
    let rp = r / cam.scale;
    let c = 0.5 * sqrt((p1[0] - p0[0]) * (p1[0] - p0[0]) + (p1[1] - p0[1]) * (p1[1] - p0[1]));
    let sag = c * c / (rp + sqrt((rp * rp - c * c).max(0.0)));
    // (cull.wgsl: a square cap's corner, the tangent at the end turned away from the chord)
    let wpx = match st.unit {
        Unit::Drawing => st.width as f64 / cam.scale,
        _ => st.width as f64,
    };
    let cap = if st.cap == Cap::Square { 0.18 * (0.5 * wpx).max(0.5) } else { 0.0 };
    let mut st2 = *st;
    st2.join = Join::Bevel;
    exact_grown(a, b, &st2, false, sag + cap, cam, size)
}

/// vs_arc's quads of a circle of radius rp px and half width h in a target of `size`, computed again on the CPU (f64,
/// the same formulas): the pixel centres they cover inside the target, summed over the pieces (what the fragment shader
/// runs for), against its ring with the smoothed edge (2 pi r (2 h + 2)) and its bounding square.  The circle's centre
/// lies in the target's middle, a circle larger than the target passes through the middle
#[cfg(not(target_arch = "wasm32"))]
pub fn arc_quad_area(rp: f64, h: f64, size: (u32, u32)) -> (f64, f64, f64) {
    let (w, ht) = (size.0 as f64, size.1 as f64);
    let half = [0.5 * w, 0.5 * ht];
    let big = rp > sqrt((half[0] + h + 3.0) * (half[0] + h + 3.0) + (half[1] + h + 3.0) * (half[1] + h + 3.0)) + h + 1.0;
    let c = if big { [half[0], half[1] + rp] } else { [half[0], half[1]] };
    let n = 16;
    let step = core::f64::consts::TAU / n as f64;
    // the angles at which the ring can meet the target and its margin (vs_arc: only for a circle much larger than it)
    let (mut lo, mut hi) = (-1e30f64, 1e30f64);
    if big {
        let tf_angle = -core::f64::consts::FRAC_PI_2;
        let (s, co) = sincos(tf_angle);
        let f = [c[0] + rp * co, c[1] + rp * s];
        let tf = [-s, co];
        let mg = [half[0] + h + 3.0, half[1] + h + 3.0];
        let (mut xmin, mut xmax) = (1e30f64, -1e30f64);
        for k in 0..4 {
            let q = [half[0] + if k & 1 != 0 { mg[0] } else { -mg[0] }, half[1] + if k & 2 != 0 { mg[1] } else { -mg[1] }];
            let x = (q[0] - f[0]) * tf[0] + (q[1] - f[1]) * tf[1];
            xmin = xmin.min(x);
            xmax = xmax.max(x);
        }
        let asin = |v: f64| -> f64 {
            let v = v.clamp(-1.0, 1.0);
            core::f64::consts::FRAC_PI_2 - acos(v)
        };
        let (ri, ro) = ((rp - h - 1.0).max(1.0), rp + h + 1.0);
        lo = tf_angle + asin(xmin / ri).min(asin(xmin / ro));
        hi = tf_angle + asin(xmax / ri).max(asin(xmax / ro));
    }
    let mut count = 0.0;
    for k in 0..n {
        let (mut b0, mut b1) = (-core::f64::consts::PI + k as f64 * step, -core::f64::consts::PI + (k + 1) as f64 * step);
        b0 = b0.max(lo);
        b1 = b1.min(hi);
        if b0 > b1 {
            continue;
        }
        let al = 0.5 * (b1 - b0);
        let (sal, _) = sincos(0.5 * al);
        let (_, cal) = sincos(al);
        let dout = ((h + 1.0) + 2.0 * rp * sal * sal) / cal;
        let din = -(h + 1.0).min(rp);
        let corner = |b: f64, dr: f64, e: f64| {
            let (s, co) = sincos(b);
            let p = [c[0] + (rp + dr) * co, c[1] + (rp + dr) * s];
            [p[0] - e * s, p[1] + e * co]
        };
        let q = [corner(b0, din, -1.0), corner(b1, din, 1.0), corner(b1, dout, 1.0), corner(b0, dout, -1.0)];
        // the pixel centres inside the quad (convex) and the target
        let (mut x0, mut x1, mut y0, mut y1) = (1e30f64, -1e30f64, 1e30f64, -1e30f64);
        for p in &q {
            x0 = x0.min(p[0]);
            x1 = x1.max(p[0]);
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
        let inside = |p: [f64; 2]| {
            let mut sgn = 0.0;
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                let cr = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                if cr != 0.0 {
                    if sgn == 0.0 {
                        sgn = cr.signum();
                    } else if cr.signum() != sgn {
                        return false;
                    }
                }
            }
            true
        };
        for y in (y0.max(0.0) as i64)..=(y1.min(ht - 1.0) as i64) {
            for x in (x0.max(0.0) as i64)..=(x1.min(w - 1.0) as i64) {
                if inside([x as f64 + 0.5, y as f64 + 0.5]) {
                    count += 1.0;
                }
            }
        }
    }
    let ring = if big {
        // (the part of the ring in the target: its width across)
        w * (2.0 * h + 2.0)
    } else {
        core::f64::consts::TAU * rp * (2.0 * h + 2.0)
    };
    let side = 2.0 * (rp + h + 1.0);
    (count, ring, side * side)
}

// ---------------------------------------------------------------- blocks (G6b2)

/// an insert of a test drawing: a block (its place in BlockDrawing::blocks, an earlier one when nested) and the insert's
/// numbers (render2d::Insert)
#[derive(Clone, Copy, Debug)]
pub struct BIns {
    pub block: usize,
    pub at: [f64; 2],
    pub rotation: f64,
    pub scale: [f64; 2],
    pub columns: u32,
    pub rows: u32,
    pub spacing: [f64; 2],
}

impl BIns {
    pub fn new(block: usize, at: [f64; 2]) -> BIns {
        BIns { block, at, rotation: 0.0, scale: [1.0, 1.0], columns: 1, rows: 1, spacing: [0.0, 0.0] }
    }

    pub fn turned(mut self, rotation: f64, scale: [f64; 2]) -> BIns {
        self.rotation = rotation;
        self.scale = scale;
        self
    }

    pub fn array(mut self, columns: u32, rows: u32, spacing: [f64; 2]) -> BIns {
        self.columns = columns;
        self.rows = rows;
        self.spacing = spacing;
        self
    }

    /// its copies' transforms (render2d's order: rows, then columns), from its block's units (less its base) to where it
    /// sits (less `origin`): (t, m) with world = t + m local
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn xfs(&self, origin: [f64; 2]) -> Vec<Aff> {
        let (s, c) = sincos(self.rotation);
        let m = [[c * self.scale[0], -s * self.scale[1]], [s * self.scale[0], c * self.scale[1]]];
        let mut v = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.columns {
                let (dx, dy) = (col as f64 * self.spacing[0], row as f64 * self.spacing[1]);
                v.push(Aff { t: [self.at[0] - origin[0] + (c * dx - s * dy), self.at[1] - origin[1] + (s * dx + c * dy)], m });
            }
        }
        v
    }
}

/// an affine map of the tests: world = t + m p
#[derive(Clone, Copy, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct Aff {
    pub t: [f64; 2],
    pub m: [[f64; 2]; 2],
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl Aff {
    pub fn lin(&self, v: [f64; 2]) -> [f64; 2] {
        [self.m[0][0] * v[0] + self.m[0][1] * v[1], self.m[1][0] * v[0] + self.m[1][1] * v[1]]
    }

    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let v = self.lin(p);
        [self.t[0] + v[0], self.t[1] + v[1]]
    }

    pub fn then(&self, o: &Aff) -> Aff {
        let (a, b) = (&self.m, &o.m);
        Aff { t: self.apply(o.t), m: [[a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]], [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]]] }
    }

    pub fn det(&self) -> f64 {
        self.m[0][0] * self.m[1][1] - self.m[0][1] * self.m[1][0]
    }

    /// a rotation, mirrored or not, times an even scale: (scale, angle, mirrored)
    pub fn similar(&self) -> Option<(f64, f64, bool)> {
        let m = &self.m;
        let n = m[0][0].abs() + m[0][1].abs() + m[1][0].abs() + m[1][1].abs();
        let mir = self.det() < 0.0;
        let off = if mir { (m[0][0] + m[1][1]).abs() + (m[0][1] - m[1][0]).abs() } else { (m[0][0] - m[1][1]).abs() + (m[0][1] + m[1][0]).abs() };
        if off > 1e-9 * n {
            return None;
        }
        Some((sqrt(m[0][0] * m[0][0] + m[1][0] * m[1][0]), atan2(m[1][0], m[0][0]), mir))
    }
}

/// atan2 in f64 for the tests (no libm): an octant, then the half-angle step twice and the series
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let pi = core::f64::consts::PI;
    let (ax, ay) = (x.abs(), y.abs());
    let (z, swap) = if ay > ax { (ax / ay, true) } else { (ay / ax, false) };
    let mut t = z;
    for _ in 0..2 {
        t /= 1.0 + sqrt(1.0 + t * t);
    }
    let t2 = t * t;
    let (mut term, mut sum) = (t, 0.0);
    for k in 0..30 {
        sum += term / (2 * k + 1) as f64;
        term *= -t2;
    }
    let mut a = 4.0 * sum;
    if swap {
        a = 0.5 * pi - a;
    }
    if x < 0.0 {
        a = pi - a;
    }
    if y < 0.0 { -a } else { a }
}

/// a block's entity or an entity on a layer: a geometry, or an insert
#[derive(Clone)]
pub enum BGeo {
    G(Geo),
    Ins(BIns),
}

/// a block of a test drawing: its base point, its entities in their order
#[derive(Clone)]
pub struct BDef {
    pub base: [f64; 2],
    pub items: Vec<(Style, BGeo)>,
}

/// layers (visible, order, transparency), blocks, the entities on the layers (layer, style, what)
#[derive(Clone, Default)]
pub struct BlockDrawing {
    pub layers: Vec<(bool, i32, u32)>,
    pub blocks: Vec<BDef>,
    pub items: Vec<(usize, Style, BGeo)>,
}

/// what build_blocks made
pub struct BuiltBlocks {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub layers: Vec<LayerId>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub blocks: Vec<render2d::BlockId>,
    /// per item on a layer its entity
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub items: Vec<EntityId>,
    /// per block per item its entity
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub block_items: Vec<Vec<EntityId>>,
}

fn insert_of(b: &BIns, blocks: &[render2d::BlockId]) -> render2d::Insert {
    let mut i = render2d::Insert::new(blocks[b.block], b.at);
    i.rotation = b.rotation;
    i.scale = b.scale;
    i.columns = b.columns;
    i.rows = b.rows;
    i.spacing = b.spacing;
    i
}

pub fn with_bshape<R>(g: &BGeo, blocks: &[render2d::BlockId], f: impl FnOnce(&Shape) -> R) -> R {
    match g {
        BGeo::G(g) => with_shape(g, f),
        BGeo::Ins(b) => f(&Shape::Insert(insert_of(b, blocks))),
    }
}

/// a block drawing in a scene: its layers, its blocks (in order: a nested insert names an earlier one), its entities
pub fn build_blocks(s: &mut Scene, d: &BlockDrawing) -> Result<BuiltBlocks> {
    let mut layers = Vec::new();
    for &(visible, order, transparency) in &d.layers {
        let mut l = Layer::new(order);
        l.visible = visible;
        l.transparency = transparency;
        layers.push(s.add_layer(&l)?);
    }
    let mut blocks = Vec::new();
    let mut block_items = Vec::new();
    for b in &d.blocks {
        let mut def = render2d::Block::new();
        def.base = b.base;
        let id = s.add_block(&def)?;
        blocks.push(id);
        let mut ids = Vec::new();
        for (st, g) in &b.items {
            let sid = s.add_style(st)?;
            ids.push(with_bshape(g, &blocks, |sh| s.insert_in(id, sid, sh))?);
        }
        block_items.push(ids);
    }
    let mut items = Vec::new();
    for (l, st, g) in &d.items {
        let sid = s.add_style(st)?;
        items.push(with_bshape(g, &blocks, |sh| s.insert(layers[*l], sid, sh))?);
    }
    Ok(BuiltBlocks { layers, blocks, items, block_items })
}

/// a colour's alpha times (100 - p) / 100 as the GPU does it (block.wgsl ink)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn ink(color: u32, p: u32) -> u32 {
    if p == 0 {
        return color;
    }
    let a = color >> 24;
    (color & 0x00FF_FFFF) | (((a * (100 - p) + 50) / 100) << 24)
}

/// an entity's style inside an insert whose style by block is `ins`: what it takes by block, its transparency into its
/// alpha (by layer: `layer_t`), its widths and patterns in units scaled with the insert (sqrt |det|), its hatch through
/// the transform
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn style_in(own: &Style, ins: Option<&Style>, xf: &Aff, base: [f64; 2], layer_t: u32) -> Style {
    let mut s = *own;
    if let Some(b) = ins {
        if own.by_block & render2d::BY_BLOCK_COLOR != 0 {
            s.color = b.color;
        }
        if own.by_block & render2d::BY_BLOCK_WIDTH != 0 {
            s.width = b.width;
            s.unit = b.unit;
        }
        if own.by_block & render2d::BY_BLOCK_DASH != 0 {
            s.dash = b.dash;
            s.dashes = b.dashes;
            s.dash_unit = b.dash_unit;
            s.dash_phase = b.dash_phase;
        }
        if own.transparency == render2d::Transparency::ByBlock {
            s.transparency = b.transparency;
        }
    }
    s.by_block = 0;
    let p = match s.transparency {
        render2d::Transparency::Percent(p) => p,
        render2d::Transparency::ByLayer => layer_t,
        _ => 0,
    };
    s.color = ink(s.color, p);
    s.transparency = render2d::Transparency::Percent(0);
    let sw = sqrt(xf.det().abs());
    if s.unit == Unit::Drawing {
        s.width = (s.width as f64 * sw) as f32;
    }
    if s.dash_unit == Unit::Drawing {
        for k in 0..s.dashes as usize {
            s.dash[k] = (s.dash[k] as f64 * sw) as f32;
        }
        s.dash_phase = (s.dash_phase as f64 * sw) as f32;
    }
    for k in 0..s.hatches as usize {
        let h = s.hatch[k];
        let (sa, ca) = sincos(h.angle);
        let d = xf.lin([ca, sa]);
        let l = sqrt(d[0] * d[0] + d[1] * d[1]);
        let du = [d[0] / l, d[1] / l];
        let nn = [-du[1], du[0]];
        let v = xf.lin([-sa * h.spacing, ca * h.spacing]);
        let (a, c) = (v[0] * nn[0] + v[1] * nn[1], v[0] * du[0] + v[1] * du[1]);
        let mut nh = render2d::HatchLine::new(atan2(du[1], du[0]), a.abs());
        nh.base = xf.apply([h.base[0] - base[0], h.base[1] - base[1]]);
        nh.shift = a.signum() * (c + l * h.shift);
        nh.dash = h.dash;
        for x in &mut nh.dash {
            *x *= l;
        }
        nh.dashes = h.dashes;
        s.hatch[k] = nh;
    }
    s
}

/// a circular or elliptic arc (centre, the map of the unit circle to it, start, sweep) as a rational quadratic spline:
/// spans of at most a quarter turn, the control points through `xf`
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn conic_spline(c: [f64; 2], a: [[f64; 2]; 2], start: f64, sweep: f64, xf: &Aff) -> Geo {
    let n = ((sweep.abs() / core::f64::consts::FRAC_PI_2 - 1e-9) as usize + 1).max(1);
    let d = sweep / n as f64;
    let w = sincos(0.5 * d).1;
    let at = |u: [f64; 2]| xf.apply([c[0] + a[0][0] * u[0] + a[0][1] * u[1], c[1] + a[1][0] * u[0] + a[1][1] * u[1]]);
    let mut pts = Vec::new();
    let mut ws = Vec::new();
    let mut knots = vec![0.0, 0.0, 0.0];
    for k in 0..n {
        let t0 = start + d * k as f64;
        let (s0, c0) = sincos(t0);
        let (sm, cm) = sincos(t0 + 0.5 * d);
        if k == 0 {
            pts.push(at([c0, s0]));
            ws.push(1.0);
        }
        // (the middle control point: where the tangents at both ends meet, 1 / cos(d / 2) out)
        pts.push(at([cm / w, sm / w]));
        ws.push(w);
        let (s1, c1) = sincos(t0 + d);
        pts.push(at([c1, s1]));
        ws.push(1.0);
        let kk = (k + 1) as f64;
        if k + 1 < n {
            knots.extend_from_slice(&[kk, kk]);
        } else {
            knots.extend_from_slice(&[kk, kk, kk]);
        }
    }
    let closed = sweep.abs() >= core::f64::consts::TAU - 1e-12;
    Geo::Spline(2, pts, ws, knots, closed)
}

/// a geometry of a block (in its units, less its base) through a transform
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn geo_through(g: &Geo, xf: &Aff, base: [f64; 2]) -> Geo {
    let p = |q: &[f64; 2]| xf.apply([q[0] - base[0], q[1] - base[1]]);
    let local = |q: &[f64; 2]| [q[0] - base[0], q[1] - base[1]];
    match g {
        Geo::Line(a, b) => Geo::Line(p(a), p(b)),
        Geo::Poly(v, c) => Geo::Poly(v.iter().map(p).collect(), *c),
        Geo::Arc(c, r, a0, sw) => match xf.similar() {
            Some((s, th, mir)) => {
                let sg = if mir { -1.0 } else { 1.0 };
                Geo::Arc(p(c), r * s, th + sg * a0, sg * sw)
            }
            None => conic_spline(local(c), [[*r, 0.0], [0.0, *r]], *a0, *sw, xf),
        },
        Geo::Circle(c, r) => match xf.similar() {
            // (a full turn starting where the circle's angle 0 went: its dash pattern starts there)
            Some((s, th, mir)) => Geo::Arc(p(c), r * s, th, if mir { -core::f64::consts::TAU } else { core::f64::consts::TAU }),
            None => conic_spline(local(c), [[*r, 0.0], [0.0, *r]], 0.0, core::f64::consts::TAU, xf),
        },
        Geo::Ellipse(c, r, rot, a0, sw) => match xf.similar() {
            Some((s, th, mir)) => {
                let sg = if mir { -1.0 } else { 1.0 };
                Geo::Ellipse(p(c), [r[0] * s, r[1] * s], th + sg * rot, sg * a0, sg * sw)
            }
            None => {
                let (sr, cr) = sincos(*rot);
                conic_spline(local(c), [[cr * r[0], -sr * r[1]], [sr * r[0], cr * r[1]]], *a0, *sw, xf)
            }
        },
        Geo::Bez(v, d, c) => Geo::Bez(v.iter().map(p).collect(), *d, *c),
        Geo::Spline(d, v, w, k, c) => Geo::Spline(*d, v.iter().map(p).collect(), w.clone(), k.clone(), *c),
        Geo::Fill(rings) => Geo::Fill(rings.iter().map(|r| r.iter().map(|g| geo_through(g, xf, base)).collect()).collect()),
    }
}

/// the drawing a block drawing shows: every insert's entities through its transforms (depth first, in their order), their
/// styles as the GPU resolves them (by block, transparency into the alpha, widths in units scaled), on the insert's layer
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn expand_blocks(d: &BlockDrawing) -> Drawing {
    let mut out = Drawing { layers: d.layers.iter().map(|l| (l.0, l.1)).collect(), items: Vec::new() };
    fn walk(d: &BlockDrawing, b: usize, xf: &Aff, ins: &Style, layer: usize, out: &mut Drawing) {
        let def = &d.blocks[b];
        let lt = d.layers[layer].2;
        for (st, g) in &def.items {
            match g {
                BGeo::G(g) => out.items.push(Item { layer, style: style_in(st, Some(ins), xf, def.base, lt), geo: geo_through(g, xf, def.base) }),
                BGeo::Ins(n) => {
                    // (a nested insert's style takes what it takes by block from its parent's)
                    let mut s2 = *st;
                    if st.by_block != 0 || st.transparency == render2d::Transparency::ByBlock {
                        let mut t = *st;
                        if st.by_block & render2d::BY_BLOCK_COLOR != 0 {
                            t.color = ins.color;
                        }
                        if st.by_block & render2d::BY_BLOCK_WIDTH != 0 {
                            t.width = ins.width;
                            t.unit = ins.unit;
                        }
                        if st.by_block & render2d::BY_BLOCK_DASH != 0 {
                            t.dash = ins.dash;
                            t.dashes = ins.dashes;
                            t.dash_unit = ins.dash_unit;
                            t.dash_phase = ins.dash_phase;
                        }
                        if st.transparency == render2d::Transparency::ByBlock {
                            t.transparency = ins.transparency;
                        }
                        s2 = t;
                    }
                    for x2 in n.xfs(def.base) {
                        walk(d, n.block, &xf.then(&x2), &s2, layer, out);
                    }
                }
            }
        }
    }
    for (l, st, g) in &d.items {
        match g {
            BGeo::G(g) => {
                let ident = Aff { t: [0.0, 0.0], m: [[1.0, 0.0], [0.0, 1.0]] };
                out.items.push(Item { layer: *l, style: style_in(st, None, &ident, [0.0, 0.0], d.layers[*l].2), geo: g.clone() });
            }
            BGeo::Ins(b) => {
                for xf in b.xfs([0.0, 0.0]) {
                    walk(d, b.block, &xf, st, *l, &mut out);
                }
            }
        }
    }
    out
}

/// a style by block: everything taken from the insert's
pub fn by_block(mut st: Style) -> Style {
    st.by_block = render2d::BY_BLOCK_COLOR | render2d::BY_BLOCK_WIDTH | render2d::BY_BLOCK_DASH;
    st.transparency = render2d::Transparency::ByBlock;
    st
}

/// the block test's drawing: three blocks (strokes, arcs and dash patterns; curves, fills and a hatch; one nesting both),
/// inserted moved, turned, scaled evenly and unevenly, mirrored, as arrays, nested; styles by block.  uneven: with the
/// inserts that scale unevenly (their dash patterns follow the block's own lengths, which the expanded drawing cannot show:
/// none in them)
pub fn blocks_drawing(uneven: bool) -> BlockDrawing {
    let mut d = BlockDrawing { layers: vec![(true, 0, 0), (true, 1, 0)], blocks: Vec::new(), items: Vec::new() };
    let g = BGeo::G;
    // A: strokes and arcs, a dash pattern in pixels and one in units, a width in units, by block colour and width
    let mut wide = style(INK[0], 0.6, Cap::Round, Join::Round);
    wide.unit = Unit::Drawing;
    let mut a = BDef { base: [10.0, 5.0], items: Vec::new() };
    a.items.push((wide, g(Geo::Poly(vec![[0.0, 0.0], [24.0, 4.0], [12.0, 20.0], [30.0, 26.0]], false))));
    a.items.push((style(INK[1], 3.0, Cap::Butt, Join::Miter), g(Geo::Arc([18.0, 12.0], 9.0, 0.3, 2.4))));
    a.items.push((style(INK[2], 1.0, Cap::Square, Join::Bevel), g(Geo::Circle([6.0, 18.0], 5.5))));
    let mut bb = style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter);
    bb.by_block = render2d::BY_BLOCK_COLOR | render2d::BY_BLOCK_WIDTH;
    a.items.push((bb, g(Geo::Poly(vec![[2.0, 28.0], [28.0, 30.0], [26.0, 8.0]], false))));
    if !uneven {
        a.items.push((dashed(INK[3], 2.0, Cap::Butt, Join::Miter, &[7.0, 3.0], Unit::Px, 1.0), g(Geo::Poly(vec![[0.0, 34.0], [30.0, 34.0], [30.0, 40.0]], false))));
        a.items.push((dashed(0xFF60_3080, 1.5, Cap::Round, Join::Round, &[3.0, 1.5], Unit::Drawing, 0.5), g(Geo::Circle([36.0, 20.0], 4.0))));
    }
    d.blocks.push(a);
    // B: curves, a fill with a hole, a hatched fill, an ellipse
    let mut b = BDef { base: [0.0, 0.0], items: Vec::new() };
    b.items.push((style(INK[1], 2.0, Cap::Round, Join::Round), g(Geo::Bez(vec![[0.0, 0.0], [10.0, 25.0], [25.0, -10.0], [35.0, 15.0]], 3, false))));
    b.items.push((style(INK[0], 1.0, Cap::Butt, Join::Miter), g(Geo::Ellipse([18.0, 30.0], [14.0, 6.0], 0.4, 0.0, core::f64::consts::TAU))));
    let sq = |x: f64, y: f64, s: f64| vec![Geo::Poly(vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]], true)];
    b.items.push((style(0xFF70_A060, 0.0, Cap::Butt, Join::Miter), g(Geo::Fill(vec![sq(2.0, 40.0, 22.0), vec![Geo::Circle([13.0, 51.0], 6.0)]]))));
    let hst = style(0xFF30_3060, 0.0, Cap::Butt, Join::Miter).hatched(&[render2d::HatchLine::new(0.5, 2.5), render2d::HatchLine::new(-0.9, 4.0).dashed(&[3.0, 1.5])], 1.0);
    b.items.push((hst, g(Geo::Fill(vec![vec![Geo::Ellipse([40.0, 50.0], [10.0, 7.0], 0.2, 0.0, core::f64::consts::TAU)]]))));
    d.blocks.push(b);
    // C: A and B nested (A turned and by block, B as an array), a line of its own between them
    let mut c = BDef { base: [5.0, 5.0], items: Vec::new() };
    c.items.push((by_block(style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter)), BGeo::Ins(BIns::new(0, [5.0, 5.0]).turned(0.5, [0.8, 0.8]))));
    c.items.push((style(INK[3], 2.0, Cap::Round, Join::Round), g(Geo::Line([0.0, 0.0], [60.0, 10.0]))));
    c.items.push((style(INK[2], 0.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(1, [40.0, 0.0]).turned(0.0, [0.5, 0.5]).array(2, 1, [30.0, 0.0]))));
    d.blocks.push(c);
    // the inserts
    let ins = |c: u32| {
        let mut s = style(c, 2.5, Cap::Butt, Join::Miter);
        s.transparency = render2d::Transparency::Percent(0);
        s
    };
    d.items.push((0, style(0xFF90_9090, 1.0, Cap::Butt, Join::Miter), g(Geo::Line([5.0, 5.0], [355.0, 265.0]))));
    d.items.push((0, ins(0xFFC0_2020), BGeo::Ins(BIns::new(0, [20.0, 20.0]))));
    d.items.push((0, ins(0xFF20_A020), BGeo::Ins(BIns::new(0, [120.0, 30.0]).turned(0.7, [1.0, 1.0]))));
    d.items.push((1, ins(0xFF2020C0), BGeo::Ins(BIns::new(0, [230.0, 20.0]).turned(-0.3, [1.6, 1.6]))));
    d.items.push((1, ins(0xFFA0_6000), BGeo::Ins(BIns::new(0, [120.0, 90.0]).turned(0.0, [-1.0, 1.0]))));
    d.items.push((1, ins(0xFF00_8080), BGeo::Ins(BIns::new(0, [175.0, 90.0]).turned(2.2, [1.2, -1.2]))));
    d.items.push((0, ins(0xFF80_0080), BGeo::Ins(BIns::new(1, [200.0, 110.0]).turned(0.25, [1.1, 1.1]))));
    d.items.push((0, ins(0xFF40_4040), BGeo::Ins(BIns::new(1, [300.0, 190.0]).turned(3.0, [-0.9, 0.9]))));
    d.items.push((1, ins(0xFF80_8000), BGeo::Ins(BIns::new(2, [10.0, 150.0]).turned(0.15, [1.3, 1.3]))));
    d.items.push((0, ins(0xFF60_6060), BGeo::Ins(BIns::new(0, [10.0, 225.0]).turned(0.0, [0.5, 0.5]).array(4, 2, [24.0, 22.0]))));
    if uneven {
        d.items.push((0, ins(0xFFB0_4000), BGeo::Ins(BIns::new(0, [230.0, 170.0]).turned(0.4, [2.0, 0.7]))));
        d.items.push((1, ins(0xFF00_6090), BGeo::Ins(BIns::new(1, [290.0, 40.0]).turned(-0.5, [0.6, 1.4]))));
        d.items.push((1, ins(0xFF90_0040), BGeo::Ins(BIns::new(2, [150.0, 200.0]).turned(0.0, [1.0, -0.6]))));
    }
    d
}

/// layers of 0, 40 and 90 % transparency; entities by layer, of their own 30 % and 0 %, opaque and translucent colours,
/// strokes, arcs, dash patterns, fills and a hatch; an insert whose entities take theirs by block from its style (60 %, and
/// by layer); the expected image is the same drawing with the transparency multiplied into each colour's alpha
pub fn transparency_drawing(layer_t: [u32; 3]) -> BlockDrawing {
    let mut d = BlockDrawing { layers: vec![(true, 0, layer_t[0]), (true, 1, layer_t[1]), (true, 2, layer_t[2])], blocks: Vec::new(), items: Vec::new() };
    let tr = |mut s: Style, t: render2d::Transparency| {
        s.transparency = t;
        s
    };
    let g = BGeo::G;
    let mut b = BDef { base: [0.0, 0.0], items: Vec::new() };
    b.items.push((tr(style(0xFF20_3080, 6.0, Cap::Round, Join::Round), render2d::Transparency::ByBlock), g(Geo::Poly(vec![[0.0, 0.0], [40.0, 30.0], [0.0, 50.0]], false))));
    b.items.push((tr(style(0xC080_2020, 0.0, Cap::Butt, Join::Miter), render2d::Transparency::ByBlock), g(Geo::Fill(vec![vec![Geo::Circle([50.0, 20.0], 15.0)]]))));
    b.items.push((tr(style(0xFF20_8020, 3.0, Cap::Butt, Join::Miter), render2d::Transparency::Percent(20)), g(Geo::Line([0.0, 60.0], [60.0, 60.0]))));
    d.blocks.push(b);
    for (l, y) in [(0usize, 20.0f64), (1, 100.0), (2, 180.0)] {
        d.items.push((l, tr(style(0xFF20_3040, 8.0, Cap::Butt, Join::Miter), render2d::Transparency::ByLayer), g(Geo::Poly(vec![[10.0, y], [150.0, y + 50.0], [20.0, y + 60.0]], false))));
        d.items.push((l, tr(style(0x9030_8040, 5.0, Cap::Square, Join::Bevel), render2d::Transparency::ByLayer), g(Geo::Line([5.0, y + 40.0], [170.0, y]))));
        d.items.push((l, tr(style(0xFF80_2020, 3.0, Cap::Butt, Join::Miter), render2d::Transparency::Percent(30)), g(Geo::Arc([90.0, y + 25.0], 30.0, 0.2, 4.5))));
        d.items.push((l, tr(dashed(0xFF20_2080, 2.0, Cap::Round, Join::Round, &[8.0, 4.0], Unit::Px, 0.0), render2d::Transparency::Percent(0)), g(Geo::Line([10.0, y + 70.0], [170.0, y + 72.0]))));
        d.items.push((l, tr(style(0xFFE0_A040, 0.0, Cap::Butt, Join::Miter), render2d::Transparency::ByLayer), g(Geo::Fill(vec![vec![Geo::Poly(vec![[180.0, y], [260.0, y + 10.0], [230.0, y + 60.0]], true)]]))));
        let hs = style(0xFF20_4060, 0.0, Cap::Butt, Join::Miter).hatched(&[render2d::HatchLine::new(0.7, 4.0)], 1.0);
        d.items.push((l, tr(hs, render2d::Transparency::ByLayer), g(Geo::Fill(vec![vec![Geo::Circle([200.0, y + 30.0], 22.0)]]))));
        // the insert: its entities by block 60 %, and (the second) by its style's layer
        d.items.push((l, tr(style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), render2d::Transparency::Percent(60)), BGeo::Ins(BIns::new(0, [270.0, y]))));
        d.items.push((l, tr(style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), render2d::Transparency::ByLayer), BGeo::Ins(BIns::new(0, [300.0, y + 10.0]).turned(0.3, [0.8, 0.8]))));
    }
    d
}

/// the deep zoom's lines as a block (G6b2): moved there (scale 1), or in units 1e5 smaller, the insert scaling them up
/// (turned by rot, mirrored); the lines land where deep_lines puts them
pub fn deep_insert_drawing(scale: f64, rot: f64, mirror: bool) -> BlockDrawing {
    let t0 = [-9_876_543.0, 6_543_211.0];
    let (sn, cs) = sincos(rot);
    let sy = if mirror { -scale } else { scale };
    let local = |p: [f64; 2]| {
        let (dx, dy) = (p[0] - t0[0], p[1] - t0[1]);
        [(cs * dx + sn * dy) / scale, (-sn * dx + cs * dy) / sy]
    };
    let lines = BDef { base: [0.0, 0.0], items: deep_lines().iter().map(|&(a, b)| (style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter), BGeo::G(Geo::Line(local(a), local(b))))).collect() };
    BlockDrawing { layers: vec![(true, 0, 0)], blocks: vec![lines], items: vec![(0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, t0).turned(rot, [scale, sy])))] }
}
