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

pub fn shape_of(g: &Geo) -> Shape<'_> {
    match g {
        Geo::Line(a, b) => Shape::Line { a: *a, b: *b },
        Geo::Poly(v, closed) => Shape::Polyline { points: v, closed: *closed },
        Geo::Arc(c, r, a0, sweep) => Shape::Arc { center: *c, radius: *r, start: *a0, sweep: *sweep },
        Geo::Circle(c, r) => Shape::Circle { center: *c, radius: *r },
    }
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
        s.insert(layers[it.layer], st, &shape_of(&it.geo))?;
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
}

/// the images the browser is compared with Vulkan on: caps, joins, hairlines, units, overlap, every case of the tests
pub fn shots() -> Vec<Shot> {
    let mut v = Vec::new();
    let mut add = |name: String, w: u32, h: u32, d: Drawing, cam: Camera, alpha: Option<u32>| v.push(Shot { name, w, h, d, cam, alpha });
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
const DEEP_AT: [f64; 2] = [-9_876_543.21, 6_543_210.987];
const DEEP_K: f64 = 1e-6;

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
