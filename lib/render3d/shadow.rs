// shadow -- the sun's cascaded shadow maps (G5c; docs/RENDER3D.md "Schatten"): one Depth32Float array of
// CASCADES_MAX layers owned by the Renderer, group 2 of the main pass with a linear comparison sampler, and the
// cascades' geometry on the CPU in f64.
//
// A cascade covers a slice of the camera's view (splits between uniform and logarithmic) by the bounding
// sphere of the slice: its radius depends only on the camera's field of view and the splits, so turning the
// camera changes nothing in the map's scale; its centre is snapped to the map's texel grid in the light's
// basis, in world coordinates (f64, not relative to the origin), so moving the camera moves the map by whole
// texels only.  Light space: z towards the sun, x and y from a fixed reference (world y, or x for a sun
// overhead).  Depth is reverse Z (the near plane, towards the sun, at 1), the planes of the visibility
// test are the box's four sides and its far side; there is no near plane: casters between the sun and the
// cascade stay in the list and are flattened onto the near plane by the vertex shader (pancaking).
//
// The map stays in Layout::Sampled between frames: a cascade's pass takes its layer from Sampled to
// DepthTarget and back (the pass's own barriers: the last frame's fragment reads before the depth writes, the
// depth writes before this frame's fragment reads).

use alloc::vec::Vec;

use gpu::*;

use crate::math::{self, V3};

/// the most cascades of the sun
pub const CASCADES_MAX: usize = 4;

/// the sun's shadows: cascaded shadow maps (Scene::set_shadows)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadows {
    /// 1 ..= CASCADES_MAX
    pub cascades: u32,
    /// texels a side of each cascade's map (16 ..= 8192)
    pub size: u32,
    /// how far along the view shadows reach (the last cascade's far end)
    pub distance: f64,
    /// how the slices are split: 0 uniform, 1 logarithmic
    pub split: f64,
    /// how far towards the sun beyond a cascade's sphere casters keep their depth; nearer the sun they are
    /// flattened onto the near plane and still cast (a triangle crossing that plane gets a wrong depth)
    pub reach: f64,
}

impl Shadows {
    /// 4 cascades of 2048 texels up to 100 from the eye, split 0.75, reach 100
    pub const DEFAULT: Shadows = Shadows { cascades: 4, size: 2048, distance: 100.0, split: 0.75, reach: 100.0 };

    pub(crate) fn check(&self) -> Result<()> {
        let fine = (1..=CASCADES_MAX as u32).contains(&self.cascades) && (16..=8192).contains(&self.size) && self.distance.is_finite() && self.distance > 0.0
            && self.split.is_finite() && (0.0..=1.0).contains(&self.split) && self.reach.is_finite() && self.reach >= 0.0;
        if fine { Ok(()) } else { Err(GpuError::Invalid("Shadows: 1 ..= 4 cascades, 16 ..= 8192 texels, distance > 0, split 0 ..= 1, reach >= 0")) }
    }
}

/// a cascade as the CPU computed it for a frame (Scene::cascades)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cascade {
    /// the slice of the view it covers (distances along the camera's forward)
    pub near: f64,
    pub far: f64,
    /// its sphere's centre after snapping (world coordinates), its radius, one texel in world units
    pub center: [f64; 3],
    pub radius: f64,
    pub texel: f64,
    /// the light's basis: x and y of the map, z towards the sun
    pub axes: [[f64; 3]; 3],
    /// the depth range along z (world, absolute): far side, near side
    pub z_far: f64,
    pub z_near: f64,
}

/// the light's basis for a sun direction (towards the sun, any length)
pub(crate) fn light_axes(dir: V3) -> [V3; 3] {
    let l = math::norm(dir);
    let up = if l[1].abs() < 0.99 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let x = math::norm(math::cross(up, l));
    [x, math::cross(l, x), l]
}

/// the cascades of a camera: slices, spheres, snapped centres
pub(crate) fn cascades(cam: &crate::Camera, sun: V3, s: &Shadows) -> Vec<Cascade> {
    let axes = light_axes(sun);
    let n = s.cascades as usize;
    let near = cam.near;
    let far = s.distance.max(near * 1.0001);
    let ty = math::tan(cam.fov_y * 0.5);
    let tx = ty * cam.aspect;
    let k2 = tx * tx + ty * ty;
    let mut out = Vec::with_capacity(n);
    let split = |i: usize| -> f64 {
        if i == 0 {
            return near;
        }
        if i == n {
            return far;
        }
        let t = i as f64 / n as f64;
        s.split * near * math::pow(far / near, t) + (1.0 - s.split) * (near + (far - near) * t)
    };
    for i in 0..n {
        let (a, b) = (split(i), split(i + 1));
        // the smallest sphere around the slice with its centre on the view's axis
        let mut c = (a + b) * (1.0 + k2) * 0.5;
        let r2 = if c >= b {
            c = b;
            b * b * k2
        } else {
            (c - a) * (c - a) + a * a * k2
        };
        let radius = math::sqrt(r2);
        let texel = 2.0 * radius / s.size as f64;
        let mid = math::add(cam.eye, math::mul(cam.forward, c));
        // snapped to whole texels in the light's x and y, in world coordinates
        let snap = |v: f64| math::floor(v / texel + 0.5) * texel;
        let (cx, cy, cz) = (snap(math::dot(mid, axes[0])), snap(math::dot(mid, axes[1])), math::dot(mid, axes[2]));
        let center = math::add(math::add(math::mul(axes[0], cx), math::mul(axes[1], cy)), math::mul(axes[2], cz));
        out.push(Cascade { near: a, far: b, center, radius, texel, axes, z_far: cz - radius, z_near: cz + radius + s.reach });
    }
    out
}

impl Cascade {
    /// the 44 floats of a view's uniform block relative to origin (as Camera::view_block): an orthographic
    /// view_proj, the planes (four sides, the far side; no near plane), the centre in place of the eye
    pub(crate) fn view_block(&self, origin: V3) -> [f32; 44] {
        let [x, y, l] = self.axes;
        let r = self.radius;
        let (ox, oy, oz) = (math::dot(origin, x) - math::dot(self.center, x), math::dot(origin, y) - math::dot(self.center, y), math::dot(origin, l));
        let depth = self.z_near - self.z_far;
        let rows = [[x[0] / r, x[1] / r, x[2] / r, ox / r], [y[0] / r, y[1] / r, y[2] / r, oy / r],
                    [l[0] / depth, l[1] / depth, l[2] / depth, (oz - self.z_far) / depth], [0.0, 0.0, 0.0, 1.0]];
        let mut out = [0.0f32; 44];
        for c in 0..4 {
            for (k, row) in rows.iter().enumerate() {
                out[c * 4 + k] = row[c] as f32;
            }
        }
        // inside: dot(n, p) + d >= 0, a little outwards for what rounding to f32 costs
        let plane = |n: V3, d: f64| -> [f32; 4] { [n[0] as f32, n[1] as f32, n[2] as f32, (d + d.abs() * 5e-7 + r * 1e-6) as f32] };
        let planes = [plane(x, ox + r), plane(math::mul(x, -1.0), r - ox), plane(y, oy + r), plane(math::mul(y, -1.0), r - oy), plane(l, oz - self.z_far), [0.0, 0.0, 0.0, 1.0]];
        for (k, p) in planes.iter().enumerate() {
            out[16 + k * 4..20 + k * 4].copy_from_slice(p);
        }
        let e = math::sub(self.center, origin);
        out[40] = e[0] as f32;
        out[41] = e[1] as f32;
        out[42] = e[2] as f32;
        out
    }

    /// draw.wgsl's Cascade: rows to (u, v, depth) of the map relative to origin, a texel, depth per unit
    pub(crate) fn receiver(&self, origin: V3) -> [f32; 16] {
        let [x, y, l] = self.axes;
        let r = self.radius;
        let (ox, oy, oz) = (math::dot(origin, x) - math::dot(self.center, x), math::dot(origin, y) - math::dot(self.center, y), math::dot(origin, l));
        let depth = self.z_near - self.z_far;
        let u = [0.5 * x[0] / r, 0.5 * x[1] / r, 0.5 * x[2] / r, 0.5 * ox / r + 0.5];
        let v = [-0.5 * y[0] / r, -0.5 * y[1] / r, -0.5 * y[2] / r, -0.5 * oy / r + 0.5];
        let d = [l[0] / depth, l[1] / depth, l[2] / depth, (oz - self.z_far) / depth];
        let mut out = [0.0f32; 16];
        for k in 0..4 {
            out[k] = u[k] as f32;
            out[4 + k] = v[k] as f32;
            out[8 + k] = d[k] as f32;
        }
        out[12] = self.texel as f32;
        out[13] = (1.0 / depth) as f32;
        out
    }
}

/// the renderer's shadow map and group 2
pub(crate) struct ShadowMap {
    pub tex: Texture,
    pub layers: Vec<TextureView>,
    array: TextureView,
    pub size: u32,
    sampler: Sampler,
    pub layout: BindGroupLayout,
    pub group: BindGroup,
}

impl ShadowMap {
    pub fn new(dev: &Device) -> Result<ShadowMap> {
        let layout = dev.create_bind_group_layout(&[
            Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::FRAGMENT },
            Binding { binding: 1, kind: BindingKind::Sampler, stages: Stages::FRAGMENT },
        ])?;
        let sampler = dev.create_sampler(&SamplerDesc { compare: Some(Compare::GreaterEqual), mip: Filter::Nearest, ..SamplerDesc::LINEAR })?;
        let (tex, layers, array) = ShadowMap::texture(dev, 1)?;
        let group = ShadowMap::make_group(dev, &layout, &array, &sampler)?;
        Ok(ShadowMap { tex, layers, array, size: 1, sampler, layout, group })
    }

    /// a map of size x size x CASCADES_MAX, cleared and left in Layout::Sampled (its own submission)
    fn texture(dev: &Device, size: u32) -> Result<(Texture, Vec<TextureView>, TextureView)> {
        let tex = dev.create_texture(&TextureDesc { width: size, height: size, depth: 1, layers: CASCADES_MAX as u32, mips: 1, format: Format::Depth32Float,
                                                    usage: TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED, cube: false })?;
        let mut layers = Vec::with_capacity(CASCADES_MAX);
        for k in 0..CASCADES_MAX as u32 {
            layers.push(tex.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: 0, mips: 1, base_layer: k, layers: 1 })?);
        }
        let array = tex.create_view(&ViewDesc { dim: ViewDim::D2Array, base_mip: 0, mips: 1, base_layer: 0, layers: 0 })?;
        let mut e = dev.encoder()?;
        for v in &layers {
            e.begin_render(&RenderPassDesc { color: &[], depth: Some(DepthAttachment { view: v, load: Load::Clear, clear: crate::DEPTH_CLEAR, store: true,
                                                                                        before: Layout::Undefined, after: Layout::Sampled }) });
            e.end_render();
        }
        dev.queue().submit([e.finish()?])?;
        Ok((tex, layers, array))
    }

    fn make_group(dev: &Device, layout: &BindGroupLayout, array: &TextureView, sampler: &Sampler) -> Result<BindGroup> {
        dev.create_bind_group(&BindGroupDesc { layout, entries: &[BindEntry { binding: 0, resource: Resource::Texture(array) }, BindEntry { binding: 1, resource: Resource::Sampler(sampler) }] })
    }

    /// a map of this size (a new one, the old one goes when the GPU is done with it)
    pub fn ensure(&mut self, dev: &Device, size: u32) -> Result<()> {
        if size == self.size {
            return Ok(());
        }
        let (tex, layers, array) = ShadowMap::texture(dev, size)?;
        self.group = ShadowMap::make_group(dev, &self.layout, &array, &self.sampler)?;
        self.tex = tex;
        self.layers = layers;
        self.array = array;
        self.size = size;
        Ok(())
    }
}
