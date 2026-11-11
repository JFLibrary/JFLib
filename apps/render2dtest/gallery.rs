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
    v
}

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
    let reach = hw * bound + 1.0;
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

/// an entity of a random scene of the visibility test: its id, points, closed, style
pub struct VisEnt {
    pub id: EntityId,
    pub pts: Vec<[f64; 2]>,
    pub closed: bool,
    pub style: Style,
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
        let id = s.insert(layers[rng.below(3) as usize], sid, &Shape::Polyline { points: &pts, closed })?;
        ents.push(VisEnt { id, pts, closed, style: st });
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
