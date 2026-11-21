// render -- the model's pictures from fixed cameras, offscreen through render3d: front (looking along -Z), side
// (from +X, looking along -X: the right side), top (from +Y, the front at the bottom of the picture), the three
// at one scale like the views of a drawing, and a three-quarter view (from front right above), each with a grid
// behind the model whose spacing the report names; and the four as one sheet (front | side over top |
// three-quarter).  The three plan views are orthographic (render3d's Camera::ortho, G7f) with outlines from the depth
// buffer (render3d's Outline): faces of one colour that lie parallel at different depths, which a plan view shows as
// one area, get a line where their depth jumps (creases only from a slope change of 3, so the grid's thin slabs stay
// without); the three-quarter view stays perspective.  Every picture is drawn at twice the size and averaged down
// (render3d has no MSAA).  Pictures are not part of any checksum: GPUs differ in the last bit.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use geom::{Mesh, Vec3};
use gpu::*;
use render3d::{Camera, Instance, Material, MaterialId, MeshData, MeshId, Outline, Renderer, RendererDesc, Scene, Sun, Transform};

use crate::export::srgb_to_linear;
use crate::host::Model;

/// drawn at this many times the size, averaged down
const SUPER: u32 = 2;

/// the background, linear rgb (an sRGB target would encode it; Rgba8Unorm takes it as it is)
pub const CLEAR: [f32; 4] = [0.78, 0.81, 0.86, 1.0];

/// the plan views' lines, `width` pixels wide (the pictures: two, drawn at twice the size; the window: one), dark, on
/// depth jumps and on creases from a slope change of 3
pub fn plan_outline(width: u32) -> Outline {
    let mut o = Outline::new(width, [0.03, 0.035, 0.05, 1.0]);
    o.crease = 3.0;
    o
}

/// the field of view of the three-quarter view
pub const ISO_FOV: f64 = 20.0;

/// the light of every view: a sun from the right above the front, ambient 0.32
pub const SUN: Sun = Sun { direction: [0.45, 0.75, 0.6], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.32; 3] };

/// the colours of the grid: plate, lines, the lines through the origin (sRGB)
pub const GRID_COLORS: [[f64; 3]; 3] = [[0.86, 0.87, 0.89], [0.55, 0.57, 0.62], [0.20, 0.27, 0.45]];

/// where a view looks from
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// one of the plan views (VIEWS 0 front, 1 side, 2 top): orthographic, with outlines
    Plan(usize),
    /// the three-quarter view of the pictures
    Iso,
    /// around the model's middle: yaw degrees from the front towards the right, pitch up, zoom (1: the model fills
    /// the view as in the three-quarter view)
    Orbit { yaw: f64, pitch: f64, zoom: f64 },
}

/// a view's camera, whether it is a plan view, its grid (plate, lines, axis lines) and the grid's spacing
pub struct Setup {
    pub cam: Camera,
    pub plan: bool,
    pub grid: (Mesh, Mesh, Mesh),
    pub step: f64,
}

/// the box around every part of the model (a point at the origin for none)
pub fn bounds_of(model: &Model) -> geom::Aabb {
    let mut b: Option<geom::Aabb> = None;
    for p in &model.parts {
        if let Some(pb) = p.mesh.bounds() {
            b = Some(match b {
                Some(x) => x.union(pb),
                None => pb,
            });
        }
    }
    b.unwrap_or(geom::Aabb::point(Vec3::ZERO))
}

/// The camera and grid of a view of a model with bounds `b`, `px` pixels high, `aspect` wide for each high: the
/// plan views at one scale (the largest extent), the plate behind the model; the three-quarter and the orbit view
/// in perspective over a ground grid.
pub fn setup(b: geom::Aabb, look: &Look, px: u32, aspect: f64) -> Setup {
    let (c, s) = (b.center(), b.size());
    let diag = s.length().max(1e-3);
    // one grid spacing for all views: the largest extent
    let extent = s.x.max(s.y).max(s.z).max(1e-3) * 1.25;
    let step = grid_spacing(extent);
    let gap = diag * 0.02;
    let aspect = if aspect > 0.0 && aspect.is_finite() { aspect } else { 1.0 };
    match *look {
        Look::Plan(i) => {
            let (_, dir, up) = VIEWS[i.min(2)];
            let (n, up) = (Vec3::from(dir), Vec3::from(up));
            // u right, v up on the screen, n towards the camera
            let u = n.cross(up) * -1.0;
            let v = up;
            // one scale for the three plan views: the largest extent of the model, orthographic (2 half high; a
            // view narrower than high grows so the width holds it)
            let half = s.x.max(s.y).max(s.z).max(1e-3) * 0.5 * 1.15;
            let h = 2.0 * half * if aspect < 1.0 { 1.0 / aspect } else { 1.0 };
            // the plate behind the model (the far side along n), larger than the view
            let back = b.min.dot(n).min(b.max.dot(n)) - gap;
            let back = if i == 2 { back.min(-gap) } else { back };
            let reach = half * 1.6 * aspect.max(1.0 / aspect);
            let line = h / px.max(1) as f64 * 2.5;
            let grid = grid(c, u, v, n, back, reach, step, line);
            let eye = c + n * diag;
            let dist = (eye - c).length();
            // (from the eye's plane, a diagonal before the model's middle, to beyond the plate behind it)
            let cam = Camera::ortho_at(eye.into(), c.into(), up.into(), h, aspect, 0.0, dist + diag * 2.0 + gap * 4.0);
            Setup { cam, plan: true, grid, step }
        }
        Look::Iso | Look::Orbit { .. } => {
            let (yaw, pitch, zoom) = match *look {
                Look::Orbit { yaw, pitch, zoom } => (yaw, pitch.clamp(-89.0, 89.0), zoom),
                _ => (35.0, 28.0, 1.0),
            };
            let (sa, ca) = geom::math::sin_cos_deg(yaw);
            let (se, ce) = geom::math::sin_cos_deg(pitch);
            let d = Vec3::new(sa * ce, se, ca * ce);
            let radius = diag * 0.5 * 1.08;
            // the narrower of the two fields of view holds the model
            let half_fov = geom::math::sin_cos_deg(ISO_FOV * 0.5);
            let tan_y = half_fov.0 / half_fov.1;
            let tan_min = if aspect < 1.0 { tan_y * aspect } else { tan_y };
            let sin_min = if aspect < 1.0 { tan_min / geom::math::sqrt(1.0 + tan_min * tan_min) } else { half_fov.0 };
            let dist = radius / sin_min * if zoom > 0.0 && zoom.is_finite() { zoom } else { 1.0 };
            // the ground under the model
            let ground = b.min.y.min(0.0) - gap;
            let grid = grid(Vec3::new(c.x, 0.0, c.z), Vec3::X, -Vec3::Z, Vec3::Y, ground, diag * 6.0 * zoom.max(1.0), step, diag / (px.max(1) as f64) * 2.5);
            let eye = c + d * dist;
            let near = (dist - diag * 2.0).max(dist * 1e-3);
            let cam = Camera::look_at(eye.into(), c.into(), [0.0, 1.0, 0.0], ISO_FOV * core::f64::consts::PI / 180.0, aspect, near, f64::INFINITY);
            Setup { cam, plan: false, grid, step }
        }
    }
}

/// the four views: name, direction from the model to the camera, the screen's up
pub const VIEWS: [(&str, [f64; 3], [f64; 3]); 4] =
    [("front", [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), ("side", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), ("top", [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]), ("iso", [0.0; 3], [0.0, 1.0, 0.0])];

/// the pictures of a model
pub struct Pictures {
    /// (view name, PNG)
    pub views: Vec<(&'static str, Vec<u8>)>,
    /// the four views as one PNG
    pub sheet: Vec<u8>,
    /// the grid's spacing in metres
    pub grid: f64,
    /// the GPU that drew them
    pub adapter: String,
}

/// an offscreen colour and depth target with a read-back buffer
struct Off {
    w: u32,
    h: u32,
    stride: u32,
    color: Texture,
    depth: Texture,
    rb: Buffer,
    used: bool,
}

impl Off {
    fn new(dev: &Device, w: u32, h: u32) -> Result<Off> {
        let stride = (w * 4).div_ceil(256) * 256;
        Ok(Off {
            w,
            h,
            stride,
            color: dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            depth: dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * h as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            used: false,
        })
    }

    /// draw the scene, wait for the GPU, return the pixels (RGBA, rows without padding)
    fn shot(&mut self, r: &mut Renderer, s: &mut Scene) -> Result<Vec<u8>> {
        s.prepare(r)?;
        let dev = r.device().clone();
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: CLEAR, store: true,
                                      before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        let depth = DepthAttachment { view: self.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true,
                                      before: if self.used { Layout::Sampled } else { Layout::Undefined }, after: Layout::Sampled };
        r.record(&mut e, s, color, Some(depth))?;
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        s.submitted(id);
        self.used = true;
        if !dev.wait(id, 20_000_000_000)? {
            return Err(GpuError::Unsupported("the GPU did not finish within 20 s"));
        }
        let mut raw = vec![0u8; self.rb.size() as usize];
        self.rb.read(0, &mut raw)?;
        let mut out = Vec::with_capacity((self.w * self.h * 4) as usize);
        for y in 0..self.h as usize {
            let at = y * self.stride as usize;
            out.extend_from_slice(&raw[at..at + (self.w * 4) as usize]);
        }
        Ok(out)
    }
}

/// a box between two corners as a mesh
fn slab(lo: Vec3, hi: Vec3) -> Option<Mesh> {
    let s = hi - lo;
    let m = geom::cuboid(s.x, s.y, s.z).ok()?;
    Some(m.translated(Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5)))
}

/// the grid spacing for a view `extent` metres wide: 1, 2 or 5 times a power of ten, at most 12 lines
pub fn grid_spacing(extent: f64) -> f64 {
    let mut s = 1e-3;
    loop {
        for k in [1.0, 2.0, 5.0] {
            if extent / (s * k) <= 12.0 {
                return s * k;
            }
        }
        s *= 10.0;
        if s > 1e6 {
            return s;
        }
    }
}

/// the grid of a view: a plate across `u` and `v` (unit axes) at `w` along the third axis `n` (facing +n),
/// lines every `step` (axis lines darker), `half` metres around `c`; (plate, lines, axis lines)
fn grid(c: Vec3, u: Vec3, v: Vec3, n: Vec3, w: f64, half: f64, step: f64, line: f64) -> (Mesh, Mesh, Mesh) {
    // coordinates along u, v of the centre
    let (cu, cv) = (c.dot(u), c.dot(v));
    let at = |pu: f64, pv: f64, pn: f64| u * pu + v * pv + n * pn;
    let corners = |a: Vec3, b: Vec3| (a.min(b), a.max(b));
    let (lo, hi) = corners(at(cu - half, cv - half, w - line * 0.5), at(cu + half, cv + half, w));
    let plate = slab(lo, hi).unwrap_or_default();
    let (mut lines, mut axes) = (Vec::new(), Vec::new());
    let first = geom::math::floor((cu - half) / step) as i64;
    let last = geom::math::floor((cu + half) / step) as i64 + 1;
    for k in first..=last {
        let p = k as f64 * step;
        let (lo, hi) = corners(at(p - line * 0.5, cv - half, w), at(p + line * 0.5, cv + half, w + line * 0.5));
        if let Some(m) = slab(lo, hi) {
            if k == 0 { axes.push(m) } else { lines.push(m) }
        }
    }
    let first = geom::math::floor((cv - half) / step) as i64;
    let last = geom::math::floor((cv + half) / step) as i64 + 1;
    for k in first..=last {
        let p = k as f64 * step;
        let (lo, hi) = corners(at(cu - half, p - line * 0.5, w), at(cu + half, p + line * 0.5, w + line * 0.5));
        if let Some(m) = slab(lo, hi) {
            if k == 0 { axes.push(m) } else { lines.push(m) }
        }
    }
    let merge = |v: &Vec<Mesh>| Mesh::merge(&v.iter().collect::<Vec<_>>()).unwrap_or_default();
    (plate, merge(&lines), merge(&axes))
}

pub fn upload(r: &mut Renderer, m: &Mesh) -> Result<Option<MeshId>> {
    if m.triangle_count() == 0 {
        return Ok(None);
    }
    let (p, n, uv) = (m.positions_f32(), m.normals_f32(), m.uvs_f32());
    Ok(Some(r.add_mesh(&MeshData { positions: &p, normals: &n, uvs: &uv, indices: &m.indices })?))
}

pub fn material(r: &mut Renderer, c: [f64; 3], roughness: f64, metallic: f64) -> Result<MaterialId> {
    let mut m = Material::color(srgb_to_linear(c[0]) as f32, srgb_to_linear(c[1]) as f32, srgb_to_linear(c[2]) as f32);
    m.roughness = roughness as f32;
    m.metallic = metallic as f32;
    r.add_material(&m)
}

/// 2 x 2 box filter
fn downsample(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (ow, oh) = (w / SUPER, h / SUPER);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            for ch in 0..4 {
                let mut s = 0u32;
                for dy in 0..SUPER {
                    for dx in 0..SUPER {
                        s += px[(((y * SUPER + dy) * w + x * SUPER + dx) * 4 + ch) as usize] as u32;
                    }
                }
                out[((y * ow + x) * 4 + ch) as usize] = ((s + SUPER * SUPER / 2) / (SUPER * SUPER)) as u8;
            }
        }
    }
    out
}

/// RGBA rows as a PNG (img_png takes B, G, R, x)
pub fn png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut bgrx = Vec::with_capacity(rgba.len());
    for p in rgba.chunks_exact(4) {
        bgrx.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    img_png::encode(&bgrx, w as usize, h as usize, w as usize * 4, false)
}

/// Draw the model's four views at `size` pixels a side (with the grid unless `no_grid`).  Err with the
/// reason when there is no GPU or drawing fails.
pub fn pictures(model: &Model, size: u32, no_grid: bool) -> core::result::Result<Pictures, String> {
    let inst = gpu::Instance::with(&InstanceDesc { app_name: "modeltool", validate: false }).map_err(|e| format!("keine GPU: {e}"))?;
    let adapter = inst.adapter().map_err(|e| format!("keine GPU: {e}"))?;
    let name = adapter.info().name.clone();
    let dev = adapter.create_device(&DeviceDesc::DEFAULT).map_err(|e| format!("GPU: {e}"))?;
    draw(&dev, model, size, no_grid).map(|(views, sheet, grid)| Pictures { views, sheet, grid, adapter: name }).map_err(|e| format!("Zeichnen: {e}"))
}

#[allow(clippy::type_complexity)]
fn draw(dev: &Device, model: &Model, size: u32, no_grid: bool) -> Result<(Vec<(&'static str, Vec<u8>)>, Vec<u8>, f64)> {
    let px = size * SUPER;
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm))?;
    r.set_size(px, px)?;
    let mut off = Off::new(dev, px, px)?;
    let mats: Vec<MaterialId> = model.materials.iter().map(|m| material(&mut r, m.color, m.roughness, m.metallic)).collect::<Result<_>>()?;
    let mut parts = Vec::with_capacity(model.parts.len());
    for p in &model.parts {
        if let Some(id) = upload(&mut r, &p.mesh)? {
            parts.push((id, mats[p.material]));
        }
    }
    let plate_mat = material(&mut r, GRID_COLORS[0], 0.95, 0.0)?;
    let line_mat = material(&mut r, GRID_COLORS[1], 0.95, 0.0)?;
    let axis_mat = material(&mut r, GRID_COLORS[2], 0.95, 0.0)?;
    let b = bounds_of(model);
    let sun = SUN;
    let mut views = Vec::new();
    for (i, (vname, _, _)) in VIEWS.iter().enumerate() {
        let mut scene = Scene::new(&r)?;
        for &(m, mat) in &parts {
            scene.add(Instance { mesh: m, material: mat, transform: Transform::at([0.0; 3]) })?;
        }
        let look = if i < 3 { Look::Plan(i) } else { Look::Iso };
        let st = setup(b, &look, px, 1.0);
        if !no_grid {
            for (m, mat) in [(&st.grid.0, plate_mat), (&st.grid.1, line_mat), (&st.grid.2, axis_mat)] {
                if let Some(id) = upload(&mut r, m)? {
                    scene.add(Instance { mesh: id, material: mat, transform: Transform::at([0.0; 3]) })?;
                }
            }
        }
        if st.plan {
            scene.set_outline(Some(&plan_outline(SUPER)))?;
        }
        scene.set_camera(&st.cam);
        scene.set_sun(&sun);
        let big = off.shot(&mut r, &mut scene)?;
        views.push((*vname, downsample(&big, px, px)));
    }
    let step = setup(b, &Look::Iso, px, 1.0).step;
    // the sheet: front | side over top | iso, a 4-pixel seam
    let seam = 4u32;
    let sw = size * 2 + seam;
    let mut sheet = vec![60u8; (sw * sw * 4) as usize];
    for (i, (_, img)) in views.iter().enumerate() {
        let (ox, oy) = ((i as u32 % 2) * (size + seam), (i as u32 / 2) * (size + seam));
        for y in 0..size {
            let src = &img[(y * size * 4) as usize..((y + 1) * size * 4) as usize];
            let at = (((oy + y) * sw + ox) * 4) as usize;
            sheet[at..at + src.len()].copy_from_slice(src);
        }
    }
    let pngs = views.iter().map(|(n, img)| (*n, png(img, size, size))).collect();
    Ok((pngs, png(&sheet, sw, sw), step))
}
