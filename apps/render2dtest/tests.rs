// tests -- render2dtest's tests on Vulkan (main.rs lists them)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, Join, Layer, LayerId, Renderer, RendererDesc, Scene, Shape, Style, Unit};

use crate::refimg::{self, Drawing, Geo, Item, Tol};

/// the test state
pub struct T {
    pub fails: u32,
    pub validate: bool,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

impl T {
    pub fn ok(&mut self, cond: bool, what: &str) -> bool {
        if cond {
            out(&format!("  ok   {what}"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {what}"));
        }
        cond
    }

    pub fn res<V>(&mut self, r: gpu::Result<V>, what: &str) -> Option<V> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.ok(false, &format!("{what}: {e}"));
                None
            }
        }
    }
}

/// instance and adapter of a run
pub struct Gpu {
    /// (held so the adapter's instance lives as long)
    #[allow(dead_code)]
    pub inst: Instance,
    pub adapter: Adapter,
}

/// None (SKIP, not a failure) without Vulkan or a usable GPU
pub fn setup(t: &mut T) -> Option<Gpu> {
    let inst = match Instance::with(&InstanceDesc { app_name: "render2dtest", validate: t.validate }) {
        Ok(i) => i,
        Err(e @ (GpuError::NoLoader | GpuError::NoAdapter | GpuError::Disabled)) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.ok(false, &format!("Instance::with: {e}"));
            return None;
        }
    };
    if t.validate && !inst.validating() {
        out("note: validation layer not available, running without it");
    }
    let adapter = match inst.adapter() {
        Ok(a) => a,
        Err(e @ (GpuError::NoAdapter | GpuError::Unsupported(_))) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.ok(false, &format!("Instance::adapter: {e}"));
            return None;
        }
    };
    Some(Gpu { inst, adapter })
}

/// a test on the adapter's own level and on a device limited to Basis
pub fn on_levels(t: &mut T, f: &mut dyn FnMut(&mut T, &Device, &str)) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() == Level::Voll {
        if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") {
            f(t, &d, "Voll");
        }
    }
    if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") {
        f(t, &d, "Basis");
    }
}

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

    /// the last image: rows of B, G, R, A
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

/// submit, wait, read the image
pub fn shot(t: &mut T, dev: &Device, off: &mut Off, r: &mut Renderer, s: &mut Scene, what: &str) -> Option<Vec<u8>> {
    let id = t.res(off.submit(r, s), what)?;
    if !t.res(dev.wait(id, 10_000_000_000), what)? {
        t.ok(false, &format!("{what}: the GPU did not finish within 10 s"));
        return None;
    }
    t.res(off.read(), what)
}

/// a test drawing in a scene: its layers
pub struct Built {
    pub layers: Vec<LayerId>,
}

pub fn shape_of(g: &Geo) -> Shape<'_> {
    match g {
        Geo::Line(a, b) => Shape::Line { a: *a, b: *b },
        Geo::Poly(v, closed) => Shape::Polyline { points: v, closed: *closed },
    }
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

/// a drawing on the GPU in a new scene: its image
pub fn gpu_image(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, d: &Drawing, cam: &Camera, what: &str) -> Option<Vec<u8>> {
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(build(&mut s, d), what)?;
    t.res(s.set_camera(cam), what)?;
    shot(t, dev, off, r, &mut s, what)
}

/// optional dumps of images for a person to look at: R2D_DUMP=DIR (raw B, G, R, A rows; tools convert them)
pub fn dump(name: &str, img: &[u8], w: u32, h: u32) {
    if let Some(dir) = plat::env("R2D_DUMP") {
        let mut v = Vec::with_capacity(img.len() + 8);
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(img);
        let _ = plat::write_file(&format!("{dir}/{name}.bgra"), &v);
    }
}

/// a drawing against lib/gfx
pub fn against_ref(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, d: &Drawing, cam: &Camera, tol: &Tol, what: &str) -> Option<refimg::Cmp> {
    let img = gpu_image(t, dev, r, off, d, cam, what)?;
    let reference = refimg::draw(d, cam, (off.w, off.h), off.bg);
    let c = refimg::compare(&img, &reference, off.w, off.bg, tol.small);
    let name: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    dump(&format!("{name}_gpu"), &img, off.w, off.h);
    dump(&format!("{name}_ref"), &reference, off.w, off.h);
    t.ok(c.fine(tol), &format!("{what}: against lib/gfx: {}", c.text()));
    Some(c)
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

const CAPS: [Cap; 3] = [Cap::Butt, Cap::Round, Cap::Square];
const JOINS: [Join; 3] = [Join::Miter, Join::Bevel, Join::Round];
const INK: [u32; 4] = [0xFF20_3040, 0xFFB0_3020, 0xFF20_7040, 0xFF30_40C0];

// ---------------------------------------------------------------- strokes against lib/gfx

fn caps(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = caps_on(t, dev, level); });
}

fn caps_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for cap in CAPS {
        for width in [1.0f32, 2.5, 6.0, 15.0] {
            // a star of 12 segments: every angle a cap meets the pixel grid at
            let mut d = Drawing::one_layer();
            let (cx, cy) = (160.3, 120.6);
            for k in 0..12 {
                let a = k as f64 * core::f64::consts::TAU / 12.0 + 0.1;
                let (s, c) = render2d::math::sincos(a);
                let (r0, r1) = (18.0 + width as f64, 100.0);
                d.add(style(INK[k % 4], width, cap, Join::Miter), Geo::Line([cx + c * r0, cy + s * r0], [cx + c * r1, cy + s * r1]));
            }
            // and points: a dot of a round cap, a square of a square cap, nothing of a butt cap
            d.add(style(INK[1], width.max(4.0), cap, Join::Miter), Geo::Poly(vec![[20.5, 20.5]], false));
            let tol = if cap == Cap::Round { Tol::round(26) } else { Tol::STRAIGHT };
            against_ref(t, dev, &mut r, &mut off, &d, &cam, &tol, &format!("{level}: {cap:?} caps, width {width}"))?;
        }
    }
    Some(())
}

fn joins(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = joins_on(t, dev, level); });
}

fn joins_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (360u32, 260u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for join in JOINS {
        for width in [1.0f32, 3.0, 9.0, 18.0] {
            let mut d = Drawing::one_layer();
            // turns from blunt to sharp, both ways (a zigzag), a closed triangle and a closed square
            d.add(style(INK[0], width, Cap::Butt, join), Geo::Poly(vec![[20.0, 40.0], [80.0, 30.0], [120.0, 70.0], [150.0, 20.0], [175.0, 75.0], [190.0, 22.0]], false));
            d.add(style(INK[1], width, Cap::Round, join), Geo::Poly(vec![[215.0, 30.0], [340.0, 45.0], [230.0, 60.0], [335.0, 80.0]], false));
            d.add(style(INK[2], width, Cap::Butt, join), Geo::Poly(vec![[30.0, 120.0], [150.0, 140.0], [60.0, 220.0]], true));
            d.add(style(INK[3], width, Cap::Square, join), Geo::Poly(vec![[200.0, 120.0], [320.0, 120.3], [320.0, 230.0], [200.5, 230.0]], true));
            // nearly straight: the joins of a flattened curve
            let arc: Vec<[f64; 2]> = (0..24).map(|k| {
                let (s, c) = render2d::math::sincos(k as f64 * 0.05);
                [180.0 + 140.0 * c - 140.0, 180.0 + 60.0 * s]
            }).collect();
            d.add(style(INK[0], width, Cap::Butt, join), Geo::Poly(arc, false));
            against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(40), &format!("{level}: {join:?} joins, width {width}"))?;
        }
    }
    // the miter limit: a turn whose miter is 3.9 and one whose miter is 4.1 widths long, limits 4 and 10
    for limit in [4.0f32, 10.0] {
        let mut d = Drawing::one_layer();
        for (k, ratio) in [3.9f64, 4.1, 8.0, 12.0].iter().enumerate() {
            // 1 / cos(phi / 2) = ratio: phi the turn
            let phi = 2.0 * acos(1.0 / ratio);
            let (s, c) = render2d::math::sincos(core::f64::consts::PI - phi);
            let x = 40.0 + 80.0 * k as f64;
            let mut st = style(INK[k % 4], 8.0, Cap::Butt, Join::Miter);
            st.miter_limit = limit;
            d.add(st, Geo::Poly(vec![[x - 30.0, 200.0], [x, 200.0], [x + 70.0 * c, 200.0 - 70.0 * s]], false));
        }
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::STRAIGHT, &format!("{level}: miter limit {limit}"))?;
    }
    Some(())
}

/// acos for the tests (no libm): Newton on cos
fn acos(x: f64) -> f64 {
    let mut a = core::f64::consts::FRAC_PI_2 * (1.0 - x);
    for _ in 0..60 {
        let (s, c) = render2d::math::sincos(a);
        if s.abs() < 1e-12 {
            break;
        }
        a += (c - x) / s;
    }
    a
}

fn hairlines(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = hairlines_on(t, dev, level); });
}

fn hairlines_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (300u32, 300u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for (width, what) in [(0.0f32, "width 0"), (0.4, "width 0.4 (drawn as 1)"), (1.0, "width 1")] {
        let mut d = Drawing::one_layer();
        // 36 angles, the start at fractions of a pixel
        for k in 0..36 {
            let a = k as f64 * core::f64::consts::TAU / 36.0 + 0.013;
            let (s, c) = render2d::math::sincos(a);
            let (cx, cy) = (150.0 + 0.1 * (k % 10) as f64, 150.0 + 0.07 * (k % 7) as f64);
            d.add(style(INK[k % 4], width, Cap::Butt, Join::Miter), Geo::Line([cx + 20.0 * c, cy + 20.0 * s], [cx + 140.0 * c, cy + 140.0 * s]));
        }
        // axis-aligned on pixel centres and on pixel edges
        d.add(style(INK[1], width, Cap::Butt, Join::Miter), Geo::Line([10.0, 290.5], [290.0, 290.5]));
        d.add(style(INK[2], width, Cap::Butt, Join::Miter), Geo::Line([10.0, 285.0], [290.0, 285.0]));
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::STRAIGHT, &format!("{level}: hairlines, {what}"))?;
    }
    Some(())
}

fn units(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = units_on(t, dev, level); });
}

fn units_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    // a drawing in metres: px widths stay, unit widths follow the zoom
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
    for (scale, rot) in [(0.1f64, 0.0f64), (0.05, 0.0), (0.02, 0.4), (0.2, -1.2)] {
        let mut cam = Camera::new([0.3, -0.2], scale);
        cam.rotation = rot;
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(24), &format!("{level}: widths in units and pixels, scale {scale}, rotation {rot}"))?;
    }
    Some(())
}

fn overlap(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = overlap_on(t, dev, level); });
}

fn overlap_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for join in JOINS {
        for alpha in [0x80u32, 0x40] {
            let c = (alpha << 24) | 0x0020_40C0;
            let mut d = Drawing::one_layer();
            // a sharp corner (20 degrees), a hairpin, a blunt one, a closed square
            d.add(style(c, 14.0, Cap::Butt, join), Geo::Poly(vec![[20.0, 40.0], [200.0, 60.0], [30.0, 95.0]], false));
            d.add(style(c, 10.0, Cap::Round, join), Geo::Poly(vec![[30.0, 140.0], [180.0, 150.0], [40.0, 160.0]], false));
            d.add(style(c, 12.0, Cap::Square, join), Geo::Poly(vec![[230.0, 30.0], [290.0, 120.0], [230.0, 200.0]], false));
            d.add(style(c, 9.0, Cap::Butt, join), Geo::Poly(vec![[60.0, 190.0], [180.0, 190.0], [180.0, 230.0], [60.0, 230.0]], true));
            let what = format!("{level}: translucent ({alpha:#x}) polylines with {join:?} joins at sharp corners");
            let Some(img) = gpu_image(t, dev, &mut r, &mut off, &d, &cam, &what) else { return None };
            let reference = refimg::draw(&d, &cam, (w, h), BG);
            let tol = Tol::corners(16);
            let cmp = refimg::compare(&img, &reference, w, BG, tol.small);
            t.ok(cmp.fine(&tol), &format!("{what}: against lib/gfx (blended once): {}", cmp.text()));
            // no pixel darker than the colour over the background once: blended twice would be
            let once = |v: u32, b: u32| (b as f64 + (v as f64 - b as f64) * alpha as f64 / 255.0 + 0.5) as i32;
            let target = [once(0xC0, BG & 255), once(0x40, (BG >> 8) & 255), once(0x20, (BG >> 16) & 255)];
            let mut twice = 0u32;
            for p in img.chunks_exact(4) {
                // (every channel of the colour lies below the background's: twice pulls it further down)
                if (0..3).any(|k| (p[k] as i32) < target[k] - 2) {
                    twice += 1;
                }
            }
            t.ok(twice == 0, &format!("{what}: no pixel beyond one blend of the colour: {twice}"));
        }
    }
    Some(())
}

// ---------------------------------------------------------------- layers and order

fn layers(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = layers_on(t, dev, level); });
}

fn layers_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (256u32, 192u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    // three layers out of order, one hidden; translucent bars crossing in every layer
    let mut d = Drawing { layers: vec![(true, 5), (true, -2), (false, 0), (true, 1)], items: Vec::new() };
    let mut rng = Rng(7);
    for k in 0..60 {
        let layer = rng.below(4) as usize;
        let a = [rng.range(10.0, 246.0), rng.range(10.0, 182.0)];
        let b = [rng.range(10.0, 246.0), rng.range(10.0, 182.0)];
        let c = ((0x60 + rng.below(0x9F)) << 24) | (rng.next() & 0x00FF_FFFF);
        let st = style(c, rng.range(2.0, 14.0) as f32, CAPS[k % 3], JOINS[k % 3]);
        let geo = if k % 3 == 0 { Geo::Poly(vec![a, [rng.range(10.0, 246.0), rng.range(10.0, 182.0)], b], false) } else { Geo::Line(a, b) };
        d.items.push(Item { layer, style: st, geo });
    }
    against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(150), &format!("{level}: 60 translucent entities on four layers (one hidden), orders 5, -2, 1"))?;
    // the same scene, its layers turned around and shown, through set_layer
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let b = t.res(build(&mut s, &d), "build")?;
    t.res(s.set_camera(&cam), "camera")?;
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "first frame")?;
    let mut d2 = d.clone();
    d2.layers = vec![(true, 0), (true, 9), (true, 3), (true, 3)];
    for (k, &(v, o)) in d2.layers.iter().enumerate() {
        let mut l = Layer::new(o);
        l.visible = v;
        t.res(s.set_layer(b.layers[k], &l), "set_layer")?;
    }
    let img = shot(t, dev, &mut off, &mut r, &mut s, "after set_layer")?;
    let info = s.upload_info();
    let reference = refimg::draw(&d2, &cam, (w, h), BG);
    let tol = Tol::corners(150);
    let cmp = refimg::compare(&img, &reference, w, BG, tol.small);
    t.ok(cmp.fine(&tol), &format!("{level}: the layers ordered anew and all shown (set_layer): against lib/gfx: {}", cmp.text()));
    t.ok(info.records == 0 && !info.rebuild, &format!("{level}: set_layer sends no records ({} records, rebuild {})", info.records, info.rebuild));
    Some(())
}

// ---------------------------------------------------------------- Voll and Basis

fn levels(t: &mut T) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() != Level::Voll {
        out("SKIP: the adapter has no level Voll");
        return;
    }
    let (w, h) = (300u32, 220u32);
    let cam = pixel_camera(w, h);
    let mut d = Drawing { layers: vec![(true, 1), (true, 0), (true, 2)], items: Vec::new() };
    let mut rng = Rng(3);
    for k in 0..300 {
        let a = [rng.range(0.0, 300.0), rng.range(0.0, 220.0)];
        let pts: Vec<[f64; 2]> = (0..1 + rng.below(6)).map(|_| [a[0] + rng.range(-40.0, 40.0), a[1] + rng.range(-40.0, 40.0)]).collect();
        let c = ((0x50 + rng.below(0xB0)) << 24) | (rng.next() & 0x00FF_FFFF);
        d.items.push(Item { layer: k % 3, style: style(c, rng.range(0.0, 9.0) as f32, CAPS[k % 3], JOINS[(k / 3) % 3]), geo: Geo::Poly(pts, k % 5 == 0) });
    }
    let mut imgs = Vec::new();
    for max_level in [Level::Voll, Level::Basis] {
        let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc { max_level }), "device") else { return };
        let Some(mut off) = t.res(Off::new(&dev, w, h), "target") else { return };
        let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        out(&format!("  ({max_level:?}: multi-draw {})", r.multi_draw()));
        let Some(img) = gpu_image(t, &dev, &mut r, &mut off, &d, &cam, "levels") else { return };
        imgs.push(img);
    }
    let n = imgs[0].chunks_exact(4).zip(imgs[1].chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
    t.ok(n == 0, &format!("Voll and Basis: 300 entities on three layers, {n} pixels differ"));
}

// ---------------------------------------------------------------- errors

fn errors(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let bad = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    t.ok(matches!(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Srgb)), Err(GpuError::Invalid(_))), "an sRGB target is refused");
    let mut d0 = RendererDesc::new(Format::Bgra8Unorm);
    d0.frames_in_flight = 4;
    t.ok(matches!(Renderer::new(&dev, &d0), Err(GpuError::Invalid(_))), "4 frames in flight are refused");
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
    let mut st = Style::new(0xFF00_0000);
    st.width = -1.0;
    t.ok(matches!(s.add_style(&st), Err(GpuError::Invalid(_))), "a negative width is refused");
    st.width = 1.0;
    st.miter_limit = 0.5;
    t.ok(matches!(s.add_style(&st), Err(GpuError::Invalid(_))), "a miter limit below 1 is refused");
    let Some(sid) = t.res(s.add_style(&Style::new(0xFF00_0000)), "add_style") else { return };
    let Some(l) = t.res(s.add_layer(&Layer::new(0)), "add_layer") else { return };
    t.ok(matches!(s.insert(l, render2d::StyleId(9), &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), Err(GpuError::Invalid(_))), "a style the scene does not have is refused");
    t.ok(matches!(s.insert(l, sid, &Shape::Line { a: [f64::NAN, 0.0], b: [1.0; 2] }), Err(GpuError::Invalid(_))), "a point that is not finite is refused");
    t.ok(matches!(s.insert(l, sid, &Shape::Polyline { points: &[], closed: false }), Err(GpuError::Invalid(_))), "a polyline without points is refused");
    let Some(e) = t.res(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), "insert") else { return };
    t.ok(s.remove(e).is_ok() && bad(s.remove(e)), "a removed entity's id is dead");
    let Some(e2) = t.res(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), "insert") else { return };
    t.ok(e2.slot == e.slot && bad(s.update(e, &Shape::Line { a: [0.0; 2], b: [2.0; 2] })), "its place goes to the next one, the old id stays dead");
    t.ok(s.remove_layer(l).is_ok() && bad(s.remove(e2)) && matches!(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), Err(GpuError::Invalid(_))),
         "a removed layer takes its entities with it, its id is dead");
    t.ok(matches!(s.set_camera(&Camera::new([0.0; 2], 0.0)), Err(GpuError::Invalid(_))), "a camera of scale 0 is refused");
    // record before prepare, a target of another size, a change after prepare
    let Some(mut off) = t.res(Off::new(&dev, 64, 48), "target") else { return };
    let Some(enc) = t.res(dev.encoder(), "encoder") else { return };
    let mut e = enc;
    let color = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc };
    t.ok(bad(r.record(&mut e, &s, color)), "record before prepare is refused");
    drop(e);
    let Some(()) = t.res(s.prepare(&mut r, (32, 32)), "prepare") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    t.ok(bad(r.record(&mut e, &s, color)), "a target of another size than prepare's is refused");
    drop(e);
    let Some(()) = t.res(s.prepare(&mut r, (64, 48)), "prepare") else { return };
    let Some(l2) = t.res(s.add_layer(&Layer::new(0)), "add_layer") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    t.ok(bad(r.record(&mut e, &s, color)), "a change after prepare: record is refused until the next prepare");
    drop(e);
    let _ = l2;
    // an empty scene: the background only (also after prepares that never reached a frame)
    let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s, "empty") else { return };
    let bgc = [(BG & 255) as u8, (BG >> 8) as u8, (BG >> 16) as u8];
    t.ok(img.chunks_exact(4).all(|p| p[..3] == bgc), "an empty scene: the background");
    let Some(s2) = t.res(Scene::new(&r), "Scene::new") else { return };
    let Some(r2) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    let color = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc };
    t.ok(bad(r2.record(&mut e, &s2, color)), "a scene of another renderer is refused");
    drop(e);
    t.res(dev.wait_idle(), "wait_idle");
}

// ---------------------------------------------------------------- the shaders

/// (the passes that subtract the camera's hi / lo, the visibility and the vertex shader, are compiled exact: NoContraction
/// on every float operation, so that the driver neither fuses nor reassociates the two-sums of common.wgsl, which it
/// did: the lo part was lost (docs/RENDER2D.md "Koordinaten"))
fn compile_shaders() -> Option<Vec<(&'static str, Vec<u32>)>> {
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let exact = wgsl::CompileOptions { exact: true, ..opt };
    let mut outv = Vec::new();
    for (src, list, o) in [(render2d::CULL_WGSL, &[("COUNT", "count"), ("SCAN", "scan"), ("WRITE", "write")][..], &exact), (render2d::SCATTER_WGSL, &[("SCATTER", "main")][..], &opt),
                           (render2d::STROKE_WGSL, &[("VS", "vs")][..], &exact), (render2d::STROKE_WGSL, &[("FS", "fs")][..], &opt)] {
        match wgsl::compile(src, o) {
            Ok(o) => {
                for (name, entry) in list {
                    outv.push((*name, o.entry(entry)?.words.clone()));
                }
            }
            Err(d) => {
                for x in d.iter().take(8) {
                    out(&format!("FAIL {}: {x}", list[0].0));
                }
                return None;
            }
        }
    }
    Some(outv)
}

fn gen_shaders(path: &str) -> i32 {
    let Some(v) = compile_shaders() else { return 1 };
    let mut s = String::from("// written by render2dtest --gen-shaders from common.wgsl + cull.wgsl, scatter.wgsl, stroke.wgsl (shader::BASIS, no robustness; COUNT, SCAN, WRITE and VS exact); do not edit\n");
    for (name, w) in &v {
        s.push_str(&format!("pub const {name}: &[u32] = &[\n"));
        for row in w.chunks(10) {
            s.push_str("    ");
            for x in row {
                s.push_str(&format!("{x:#x}, "));
            }
            s.truncate(s.trim_end().len());
            s.push('\n');
        }
        s.push_str("];\n");
    }
    if plat::write_file(path, s.as_bytes()) {
        out(&format!("{path}: {}", v.iter().map(|x| format!("{} {} words", x.0, x.1.len())).collect::<Vec<_>>().join(", ")));
        0
    } else {
        out(&format!("FAIL cannot write {path}"));
        1
    }
}

fn shaders(t: &mut T) {
    let Some(v) = compile_shaders() else {
        t.ok(false, "the WGSL compiles");
        return;
    };
    let table: [(&str, &[u32]); 6] = [("COUNT", render2d::COUNT_SPV), ("SCAN", render2d::SCAN_SPV), ("WRITE", render2d::WRITE_SPV), ("SCATTER", render2d::SCATTER_SPV),
                                      ("VS", render2d::VS_SPV), ("FS", render2d::FS_SPV)];
    for (name, embedded) in table {
        let made = v.iter().find(|x| x.0 == name).map_or(&[][..], |x| &x.1[..]);
        t.ok(made == embedded, &format!("{name}: {} words embedded, as wgsl::compile makes them (if not: render2dtest --gen-shaders lib/render2d/shaders_spv.rs)", embedded.len()));
    }
}

pub fn run() -> i32 {
    let args = plat::args();
    let validate = args.iter().any(|a| a == "--validate");
    if validate {
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    if let Some(p) = plat::arg_value("--gen-shaders") {
        return gen_shaders(&p);
    }
    if args.iter().any(|a| a == "--demo") {
        return crate::demo::launch();
    }
    if let Some(n) = plat::arg_value("--bench").and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0) {
        return crate::bench::bench(n);
    }
    let Some(test) = plat::arg_value("--test") else {
        plat::write_stderr(b"render2dtest: --test NAME [--validate], --bench N, --demo, --gen-shaders FILE (see apps/render2dtest/main.rs)\n");
        return 2;
    };
    let mut t = T { fails: 0, validate };
    let list: [(&str, fn(&mut T)); 12] = [("shaders", shaders), ("caps", caps), ("joins", joins), ("hairlines", hairlines), ("units", units), ("overlap", overlap),
                                          ("layers", layers), ("changes", crate::scenes::changes), ("visibility", crate::scenes::visibility),
                                          ("coords", crate::scenes::coords), ("levels", levels), ("errors", errors)];
    let mut ran = false;
    for (name, f) in list {
        if test == "all" || test == name {
            out(&format!("== {name}{}", if validate { " (validation)" } else { "" }));
            f(&mut t);
            ran = true;
        }
    }
    if test == "window" {
        out(&format!("== window{}", if validate { " (validation)" } else { "" }));
        return crate::demo::window_test(validate);
    }
    if !ran {
        plat::write_stderr(format!("render2dtest: no test {test}\n").as_bytes());
        return 2;
    }
    if validate {
        let (e, w) = gpu::validation_messages();
        t.ok(e == 0, &format!("validation layer: {e} errors, {w} warnings"));
    }
    if t.fails > 0 {
        out(&format!("{} check(s) failed", t.fails));
        1
    } else {
        out("all checks passed");
        0
    }
}

