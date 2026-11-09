// texture -- the textures of a Renderer (G5b; docs/RENDER3D.md "Texturen"): one texture array per size class
// and colour space, chosen per instance through its material in the fragment shader (a switch over the five
// classes), one repeating trilinear sampler for everything.  The same binding model on Vulkan Voll, Basis and
// WebGPU, from the same WGSL: lib/wgsl has no binding arrays and WebGPU none in its core.
//
// Classes are squares of 128, 256, 512, 1024 and 2048 texels with every mip level.  A texture of another
// size goes into the class of its next power of two (128 at least, 2048 at most), resampled on the CPU:
// nearest for textures drawn with a nearest filter (a small one is repeated texel for texel), else
// bilinear upwards and a box downwards, in linear light for sRGB.  The mip levels are 2 x 2 box filters on
// the CPU, sRGB decoded by a table, averaged, encoded again (exactly rounded through a table of thresholds).
// Wrap modes and nearest filtering happen in the shader (draw.wgsl: wrap_coord), exactly, because the
// classes are powers of two.
//
// An array grows by doubling its layers: a new texture, the old layers copied through a buffer, the old one
// dropped once the GPU is done with it.  Uploads go through Queue::write_texture (staged before the next
// submission, after a barrier from Sampled: frames still reading the array finish first).

use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::math;

/// the size classes: 128 << k texels square
pub const CLASSES: usize = 5;
pub const CLASS_MIN: u32 = 128;
pub const CLASS_MAX: u32 = CLASS_MIN << (CLASSES - 1);
/// the texture word of a material slot without a texture
pub(crate) const NO_TEXTURE: u32 = u32::MAX;
/// bytes of the buffer a growing array's layers pass through, at most (a 2048 layer is about 22 MB)
const GROW_CHUNK: u64 = 64 << 20;

/// what add_texture takes: rows from the top, 4 bytes per texel, r g b a with straight alpha
#[derive(Clone, Copy, Debug)]
pub struct TextureData<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    /// colour (base colour, emissive): sRGB; data (metallic-roughness, normals, occlusion): linear
    pub srgb: bool,
    /// it is drawn with a nearest filter: a resampling to its class keeps the texels (else bilinear)
    pub nearest: bool,
}

/// a texture of a Renderer
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureId(pub u32);

/// where a texture lies
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureInfo {
    /// its class (0 ..), the class's size, its layer in the class's array
    pub class: u32,
    pub size: u32,
    pub layer: u32,
    pub srgb: bool,
    /// the size it was given; resampled: it was not the class's size
    pub width: u32,
    pub height: u32,
    pub resampled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    Clamp,
    Mirror,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mip {
    /// level 0 only
    None,
    Nearest,
    Linear,
}

/// how a material samples a texture (glTF's samplers)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampling {
    pub mag_nearest: bool,
    pub min_nearest: bool,
    pub mip: Mip,
    pub wrap_u: Wrap,
    pub wrap_v: Wrap,
}

impl Sampling {
    /// repeating, trilinear: the sampler's own path in the shader
    pub const LINEAR: Sampling = Sampling { mag_nearest: false, min_nearest: false, mip: Mip::Linear, wrap_u: Wrap::Repeat, wrap_v: Wrap::Repeat };
    /// repeating, nearest texels of the nearest level
    pub const NEAREST: Sampling = Sampling { mag_nearest: true, min_nearest: true, mip: Mip::Nearest, wrap_u: Wrap::Repeat, wrap_v: Wrap::Repeat };

    /// the mode byte of a texture word: wrap u (bits 0-1), wrap v (2-3), mag nearest (4), min nearest (5),
    /// mip (6-7: none, nearest, linear); draw.wgsl reads it
    pub(crate) fn bits(&self) -> u32 {
        let w = |w: Wrap| match w {
            Wrap::Repeat => 0,
            Wrap::Clamp => 1,
            Wrap::Mirror => 2,
        };
        let mip = match self.mip {
            Mip::None => 0,
            Mip::Nearest => 1,
            Mip::Linear => 2,
        };
        w(self.wrap_u) | w(self.wrap_v) << 2 | (self.mag_nearest as u32) << 4 | (self.min_nearest as u32) << 5 | mip << 6
    }
}

/// a texture of a material: which, sampled how (uv set 0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureRef {
    pub texture: TextureId,
    pub sampling: Sampling,
}

// ---------------------------------------------------------------- sRGB

/// sRGB as tables: the linear value of each code, the linear values where codes 1 ..= 255 begin, and the
/// code at k / 4096 (encode starts there and steps up past the starts below its value: one step at most)
pub(crate) struct Srgb {
    pub decode: [f32; 256],
    pub starts: [f32; 255],
    near: [u8; 4097],
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { math::pow((c + 0.055) / 1.055, 2.4) }
}

impl Srgb {
    pub fn new() -> Srgb {
        let mut s = Srgb { decode: [0.0; 256], starts: [0.0; 255], near: [0; 4097] };
        for k in 0..256 {
            s.decode[k] = to_linear(k as f64 / 255.0) as f32;
        }
        for k in 0..255 {
            s.starts[k] = to_linear((k as f64 + 0.5) / 255.0) as f32;
        }
        for k in 0..4097 {
            let v = k as f32 / 4096.0;
            s.near[k] = s.starts.partition_point(|&t| t <= v) as u8;
        }
        s
    }

    /// the code of a linear value, rounded to the nearest: the number of starts at or below it
    pub fn encode(&self, v: f32) -> u8 {
        let k = (v * 4096.0).clamp(0.0, 4096.0) as usize;
        let mut c = self.near[k] as usize;
        // (v * 4096 may round up into the next bin)
        while c > 0 && self.starts[c - 1] > v {
            c -= 1;
        }
        while c < 255 && self.starts[c] <= v {
            c += 1;
        }
        c as u8
    }
}

// ---------------------------------------------------------------- the CPU's part: class, resampling, mips

/// the class of a texture of w x h: its next power of two, 128 ..= 2048
pub fn class_of(w: u32, h: u32) -> u32 {
    let s = w.max(h).max(1).checked_next_power_of_two().unwrap_or(CLASS_MAX).clamp(CLASS_MIN, CLASS_MAX);
    s.trailing_zeros() - CLASS_MIN.trailing_zeros()
}

/// floor for the resampler's coordinates (no libm)
fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// the taps of one axis from n texels to m: (first source texel, weights)
fn taps(n: u32, m: u32, nearest: bool) -> Vec<(u32, Vec<f32>)> {
    let r = n as f64 / m as f64;
    (0..m)
        .map(|i| {
            if nearest {
                ((((i as f64 + 0.5) * r) as u32).min(n - 1), vec![1.0])
            } else if m >= n {
                // bilinear, the edges clamped
                let x = (i as f64 + 0.5) * r - 0.5;
                let x0 = floor(x);
                let f = (x - x0) as f32;
                let a = (x0.max(0.0) as u32).min(n - 1);
                let b = ((x0 + 1.0).max(0.0) as u32).min(n - 1);
                if a == b { (a, vec![1.0]) } else { (a, vec![1.0 - f, f]) }
            } else {
                // a box: the source texels the target texel covers, by how much
                let (lo, hi) = (i as f64 * r, (i as f64 + 1.0) * r);
                let first = floor(lo) as u32;
                let mut w = Vec::new();
                let mut k = first;
                while (k as f64) < hi && k < n {
                    let cover = (hi.min(k as f64 + 1.0) - lo.max(k as f64)) / r;
                    w.push(cover as f32);
                    k += 1;
                }
                (first, w)
            }
        })
        .collect()
}

/// the texels resampled to size x size; sRGB in linear light
pub(crate) fn fit(d: &TextureData, size: u32, srgb: &Srgb) -> Vec<u8> {
    let (w, h) = (d.width, d.height);
    if (w, h) == (size, size) {
        return d.rgba[..(size * size * 4) as usize].to_vec();
    }
    let (tx, ty) = (taps(w, size, d.nearest), taps(h, size, d.nearest));
    if d.nearest {
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for (y, _) in &ty {
            for (x, _) in &tx {
                let at = ((y * w + x) * 4) as usize;
                out.extend_from_slice(&d.rgba[at..at + 4]);
            }
        }
        return out;
    }
    let lin = |c: u8, k: usize| if d.srgb && k < 3 { srgb.decode[c as usize] } else { c as f32 / 255.0 };
    // rows first: h x size, then columns
    let mut rows = vec![0f32; (h * size * 4) as usize];
    for y in 0..h as usize {
        for (x, (first, wts)) in tx.iter().enumerate() {
            for k in 0..4 {
                let mut acc = 0.0;
                for (j, wt) in wts.iter().enumerate() {
                    acc += wt * lin(d.rgba[((y * w as usize) + *first as usize + j) * 4 + k], k);
                }
                rows[(y * size as usize + x) * 4 + k] = acc;
            }
        }
    }
    let mut out = vec![0u8; (size * size * 4) as usize];
    for (y, (first, wts)) in ty.iter().enumerate() {
        for x in 0..size as usize {
            for k in 0..4 {
                let mut acc = 0.0;
                for (j, wt) in wts.iter().enumerate() {
                    acc += wt * rows[((*first as usize + j) * size as usize + x) * 4 + k];
                }
                out[(y * size as usize + x) * 4 + k] = if d.srgb && k < 3 { srgb.encode(acc) } else { (acc.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
            }
        }
    }
    out
}

/// the next level of an n x n level: 2 x 2 boxes, sRGB in linear light
pub(crate) fn half(level: &[u8], n: u32, srgb_on: bool, srgb: &Srgb) -> Vec<u8> {
    let m = (n / 2).max(1);
    let n = n as usize;
    let mut out = vec![0u8; m as usize * m as usize * 4];
    for y in 0..m as usize {
        for x in 0..m as usize {
            let at = |dx: usize, dy: usize| ((y * 2 + dy).min(n - 1) * n + (x * 2 + dx).min(n - 1)) * 4;
            let p = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
            for k in 0..4 {
                let o = (y * m as usize + x) * 4 + k;
                out[o] = if srgb_on && k < 3 {
                    let s: f32 = p.iter().map(|&a| srgb.decode[level[a + k] as usize]).sum();
                    srgb.encode(s * 0.25)
                } else {
                    let s: u32 = p.iter().map(|&a| level[a + k] as u32).sum();
                    ((s + 2) / 4) as u8
                };
            }
        }
    }
    out
}

/// the texels at the class's size and every level below
fn levels(d: &TextureData, srgb: &Srgb) -> Vec<Vec<u8>> {
    let size = CLASS_MIN << class_of(d.width, d.height);
    let mut out = vec![fit(d, size, srgb)];
    let mut n = size;
    while n > 1 {
        let next = half(&out[out.len() - 1], n, d.srgb, srgb);
        out.push(next);
        n /= 2;
    }
    out
}

/// what add_texture uploads, without a device (tools, measurements): the texels resampled to their class's
/// size, then each mip level down to 1 x 1; rows of 4 bytes per texel.  Empty when the data is too short
pub fn mip_chain(d: &TextureData) -> Vec<Vec<u8>> {
    if d.width == 0 || d.height == 0 || d.width > 1 << 15 || d.height > 1 << 15 || (d.rgba.len() as u64) < d.width as u64 * d.height as u64 * 4 {
        return Vec::new();
    }
    levels(d, &Srgb::new())
}

// ---------------------------------------------------------------- the GPU's part: arrays per class

/// one class's array in one colour space
struct ClassArray {
    tex: Option<Texture>,
    view: TextureView,
    used: u32,
    cap: u32,
}

/// a renderer's textures: the arrays, the sampler, the bind group of draw.wgsl's group 1
pub(crate) struct Textures {
    /// [srgb as index][class]
    arrays: [[ClassArray; CLASSES]; 2],
    pub layout: BindGroupLayout,
    sampler: Sampler,
    pub group: BindGroup,
    pub info: Vec<TextureInfo>,
    pub srgb: Srgb,
    max_layers: u32,
}

fn format_of(srgb: bool) -> Format {
    if srgb { Format::Rgba8Srgb } else { Format::Rgba8Unorm }
}

fn array_view(t: &Texture) -> Result<TextureView> {
    t.create_view(&ViewDesc { dim: ViewDim::D2Array, base_mip: 0, mips: 0, base_layer: 0, layers: 0 })
}

/// a 1 x 1 array for a class without textures (every binding must hold one)
fn placeholder(dev: &Device, srgb: bool) -> Result<ClassArray> {
    let t = dev.create_texture(&TextureDesc { width: 1, height: 1, depth: 1, layers: 1, mips: 1, format: format_of(srgb),
                                              usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST, cube: false })?;
    dev.queue().write_texture(&TextureRegion::mip(&t, 0), &[255u8; 4], DataLayout::default(), Layout::Undefined, Layout::Sampled)?;
    Ok(ClassArray { view: array_view(&t)?, tex: Some(t), used: 0, cap: 0 })
}

impl Textures {
    pub fn new(dev: &Device) -> Result<Textures> {
        let mut bindings = Vec::new();
        for b in 0..(2 * CLASSES as u32) {
            bindings.push(Binding { binding: b, kind: BindingKind::Texture, stages: Stages::FRAGMENT });
        }
        bindings.push(Binding { binding: 2 * CLASSES as u32, kind: BindingKind::Sampler, stages: Stages::FRAGMENT });
        let layout = dev.create_bind_group_layout(&bindings)?;
        let sampler = dev.create_sampler(&SamplerDesc { address: [AddressMode::Repeat; 3], ..SamplerDesc::LINEAR })?;
        let mk = |srgb| -> Result<[ClassArray; CLASSES]> {
            Ok([placeholder(dev, srgb)?, placeholder(dev, srgb)?, placeholder(dev, srgb)?, placeholder(dev, srgb)?, placeholder(dev, srgb)?])
        };
        let arrays = [mk(false)?, mk(true)?];
        let group = Textures::make_group(dev, &layout, &arrays, &sampler)?;
        Ok(Textures { arrays, layout, sampler, group, info: Vec::new(), srgb: Srgb::new(), max_layers: dev.limits().max_texture_layers.clamp(1, 1 << 16) })
    }

    fn make_group(dev: &Device, layout: &BindGroupLayout, arrays: &[[ClassArray; CLASSES]; 2], sampler: &Sampler) -> Result<BindGroup> {
        let mut entries = Vec::new();
        // bindings 0 .. 4 sRGB, 5 .. 9 linear
        for (s, srgb) in [true, false].iter().enumerate() {
            for c in 0..CLASSES {
                entries.push(BindEntry { binding: (s * CLASSES + c) as u32, resource: Resource::Texture(&arrays[*srgb as usize][c].view) });
            }
        }
        entries.push(BindEntry { binding: 2 * CLASSES as u32, resource: Resource::Sampler(sampler) });
        dev.create_bind_group(&BindGroupDesc { layout, entries: &entries })
    }

    /// a texture into its class's array, with every mip level
    pub fn add(&mut self, dev: &Device, d: &TextureData) -> Result<TextureId> {
        if d.width == 0 || d.height == 0 || d.width > 1 << 15 || d.height > 1 << 15 {
            return Err(GpuError::Invalid("add_texture: 1 ..= 32768 texels a side"));
        }
        if (d.rgba.len() as u64) < d.width as u64 * d.height as u64 * 4 {
            return Err(GpuError::Invalid("add_texture: fewer bytes than width x height x 4"));
        }
        if self.info.len() >= u32::MAX as usize - 1 {
            return Err(GpuError::Invalid("add_texture: too many textures"));
        }
        let class = class_of(d.width, d.height);
        let size = CLASS_MIN << class;
        let (s, c) = (d.srgb as usize, class as usize);
        if self.arrays[s][c].used >= self.max_layers {
            return Err(GpuError::Unsupported("add_texture: more textures of one size class than the device's array layers"));
        }
        let mips = class + 8;
        if self.arrays[s][c].used == self.arrays[s][c].cap {
            self.grow(dev, s, c, size, mips)?;
        }
        let layer = self.arrays[s][c].used;
        let Some(tex) = self.arrays[s][c].tex.clone() else { return Err(GpuError::Invalid("add_texture: no array")) };
        for (mip, level) in levels(d, &self.srgb).iter().enumerate() {
            let n = (size >> mip).max(1);
            let region = TextureRegion { texture: &tex, mip: mip as u32, origin: [0; 3], size: [n, n, 1], base_layer: layer, layers: 1 };
            dev.queue().write_texture(&region, level, DataLayout::default(), Layout::Sampled, Layout::Sampled)?;
        }
        self.arrays[s][c].used += 1;
        self.info.push(TextureInfo { class, size, layer, srgb: d.srgb, width: d.width, height: d.height, resampled: (d.width, d.height) != (size, size) });
        Ok(TextureId(self.info.len() as u32 - 1))
    }

    /// twice the layers (4 at first): a new array, the old layers copied across through a buffer
    fn grow(&mut self, dev: &Device, s: usize, c: usize, size: u32, mips: u32) -> Result<()> {
        let (used, old_cap) = (self.arrays[s][c].used, self.arrays[s][c].cap);
        let cap = if old_cap == 0 { 4 } else { old_cap.saturating_mul(2) }.min(self.max_layers);
        let t = dev.create_texture(&TextureDesc { width: size, height: size, depth: 1, layers: cap, mips, format: format_of(s == 1),
                                                  usage: TextureUsage::SAMPLED | TextureUsage::COPY_DST | TextureUsage::COPY_SRC, cube: false })?;
        let old = if used > 0 { self.arrays[s][c].tex.clone() } else { None };
        // rows of 256 bytes at least (WebGPU's copies want them), every level's place aligned to 256
        let row = |l: u32| ((size >> l).max(1) * 4).div_ceil(256) * 256;
        let place: Vec<u64> = (0..mips).scan(0u64, |at, l| { let p = *at; *at += row(l) as u64 * (size >> l).max(1) as u64; Some(p) }).collect();
        let per_layer = place[mips as usize - 1] + row(mips - 1) as u64;
        let chunk = (GROW_CHUNK / per_layer).clamp(1, used.max(1) as u64) as u32;
        let buf = match old {
            Some(_) => Some(dev.create_buffer(&BufferDesc { size: per_layer * chunk as u64, usage: BufferUsage::NONE, memory: MemoryKind::Device })?),
            None => None,
        };
        let mut e = dev.encoder()?;
        e.texture_barrier(&t, Layout::Undefined, Layout::CopyDst);
        if let (Some(old), Some(buf)) = (&old, &buf) {
            // (frames still reading the old array finish first)
            e.texture_barrier(old, Layout::Sampled, Layout::CopySrc);
            let mut first = 0;
            while first < used {
                let n = chunk.min(used - first);
                let side = |l: u32| (size >> l).max(1);
                let lay = |l: u32| DataLayout { offset: place[l as usize] * chunk as u64, bytes_per_row: row(l), rows_per_image: side(l) };
                let region = |tex, l: u32| TextureRegion { texture: tex, mip: l, origin: [0; 3], size: [side(l), side(l), 1], base_layer: first, layers: n };
                for l in 0..mips {
                    e.copy_texture_to_buffer(region(old, l), buf, lay(l));
                }
                e.barrier(Access::COPY_WRITE, Access::COPY_READ);
                for l in 0..mips {
                    e.copy_buffer_to_texture(buf, lay(l), region(&t, l));
                }
                // (the next chunk writes the buffer again)
                e.barrier(Access::COPY_READ, Access::COPY_WRITE);
                first += n;
            }
        }
        e.texture_barrier(&t, Layout::CopyDst, Layout::Sampled);
        dev.queue().submit([e.finish()?])?;
        self.arrays[s][c] = ClassArray { view: array_view(&t)?, tex: Some(t), used, cap };
        self.group = Textures::make_group(dev, &self.layout, &self.arrays, &self.sampler)?;
        Ok(())
    }

    /// the texture word of a material slot: layer | class << 16 | mode << 20 | linear << 28
    pub fn word(&self, r: &Option<TextureRef>) -> Result<u32> {
        let Some(r) = r else { return Ok(NO_TEXTURE) };
        let Some(i) = self.info.get(r.texture.0 as usize) else { return Err(GpuError::Invalid("Material: a texture the renderer does not have")) };
        Ok(i.layer | i.class << 16 | r.sampling.bits() << 20 | (!i.srgb as u32) << 28)
    }

    /// bytes of GPU memory the arrays take (placeholders not counted)
    pub fn bytes(&self) -> u64 {
        let mut total = 0;
        for side in &self.arrays {
            for (c, a) in side.iter().enumerate() {
                let size = (CLASS_MIN << c) as u64;
                total += a.cap as u64 * size * size * 4 * 4 / 3;
            }
        }
        total
    }

    /// (tests) the array of a class and colour space, None while it has no texture
    pub fn array(&self, class: u32, srgb: bool) -> Option<&Texture> {
        let a = self.arrays[srgb as usize].get(class as usize)?;
        if a.cap > 0 { a.tex.as_ref() } else { None }
    }
}
