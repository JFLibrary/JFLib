// t_texture -- gputest's tests of textures in shaders, each against a CPU reference (main.rs
// lists them); the shaders are shader::samples, built with lib/shader
//
//   textures   fetch      OpImageFetch of every texel of every mip / layer in a compute shader:
//                         rgba8unorm (5 mips), r32float (4 mips), r32float 2D array, r32uint,
//                         r32sint, rgba32uint, depth32float -- exact
//              sample     OpImageSampleExplicitLod in a compute shader, nearest: a 2D array
//                         (layer = RNE(z) clamped), a cube (every texel of every face through
//                         its direction, also scaled), a cube array -- exact
//              storage    OpImageWrite then OpImageRead (and OpImageQuerySize) of rgba8unorm,
//                         rgba16float, r32float, rgba32float, r32uint, rgba32uint, r32sint; the
//                         texture's bytes read back; read-write r32float / r32uint / r32sint --
//                         exact
//              fragment   a texture + sampler in a fragment shader: nearest at explicit LOD 0
//                         .. 4 and with implicit LOD (mip selection from derivatives) -- exact;
//                         bilinear at LOD 0 and trilinear at LOD 1.5 -- within LINEAR_TOL
//              compare    a depth texture with a comparison sampler (less, greater-equal),
//                         OpImageSampleDrefExplicitLod and ...ImplicitLod -- exact; linear
//                         (PCF) -- within LINEAR_TOL
//              queries    OpImageQuerySizeLod at mips 0 / 1, OpImageQueryLevels of 2D, 2D array,
//                         cube, cube array, depth and a view of mips 1 .. 3; OpImageQuerySize of
//                         a storage texture -- exact
//              bindless   (Voll) textures and samplers of a Bindless table indexed non-uniformly
//                         and sampled (nearest and linear)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use shader::samples::{self, SampleDim, Texel};
use shader::Fmt;

use crate::t_compute::readback;
use crate::{T, out, run_wait, setup};

/// the tolerance of linear filtering: the GPU's filter weights have subTexelPrecisionBits (8
/// on this GPU: steps of 1/256) and its own rounding; every sample here sits on quarter texels,
/// whose weights are exact, so the difference is the filter's arithmetic only
pub const LINEAR_TOL: f32 = 1.0 / 256.0;

fn bytes_of_words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// a texture with every (mip, layer) written by data(mip, layer) (tightly packed rows), in
/// Layout::Sampled
/// the iOS simulator's Metal has no cube arrays
const NO_CUBE_ARRAYS: bool = cfg!(target_abi = "sim");

fn texture(t: &mut T, dev: &Device, desc: TextureDesc, data: &dyn Fn(u32, u32) -> Vec<u8>, what: &str) -> Option<Texture> {
    let tex = t.res(dev.create_texture(&desc), &format!("{what}: texture"))?;
    let q = dev.queue();
    for m in 0..desc.mips {
        let (w, h) = ((desc.width >> m).max(1), (desc.height >> m).max(1));
        for l in 0..desc.layers {
            let region = TextureRegion { texture: &tex, mip: m, origin: [0; 3], size: [w, h, 1], base_layer: l, layers: 1 };
            t.res(q.write_texture(&region, &data(m, l), DataLayout::default(), Layout::Undefined, Layout::Sampled),
                  &format!("{what}: write_texture"))?;
        }
    }
    Some(tex)
}

/// a compute pipeline and its bind group layout
fn cpipe(t: &mut T, dev: &Device, words: &[u32], bindings: &[Binding], push: u32, what: &str) -> Option<(BindGroupLayout, ComputePipeline)> {
    if let Err(e) = shader::val::validate(words) {
        t.fail(&format!("{what}: shader::val: {}", e[0]));
        return None;
    }
    let m = t.res(ShaderModule::from_spirv(dev, words), &format!("{what}: module"))?;
    let bgl = t.res(dev.create_bind_group_layout(bindings), &format!("{what}: layout"))?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: push }), "layout")?;
    let p = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }),
                  &format!("{what}: pipeline"))?;
    Some((bgl, p))
}

fn bind(k: u32, kind: BindingKind, stages: Stages) -> Binding {
    Binding { binding: k, kind, stages }
}

fn buf(t: &mut T, dev: &Device, bytes: u64, usage: BufferUsage) -> Option<Buffer> {
    t.res(dev.create_buffer(&BufferDesc { size: bytes.max(16), usage: BufferUsage::STORAGE | usage, memory: MemoryKind::Device }),
          "buffer")
}

fn whole(b: &Buffer) -> Resource<'_> {
    Resource::Buffer { buffer: b, offset: 0, size: 0 }
}

/// the first mismatch of got against want (u32 words) as text, "" when equal
fn first_diff(got: &[u32], want: &[u32]) -> String {
    if got.len() < want.len() {
        return format!(" (only {} of {} words)", got.len(), want.len());
    }
    match (0..want.len()).find(|&k| got[k] != want[k]) {
        None => String::new(),
        Some(k) => format!(": word {k} = {:#x} ({}), expected {:#x} ({})", got[k], f32::from_bits(got[k]), want[k], f32::from_bits(want[k])),
    }
}

// ---------------------------------------------------------------- fetch

struct FetchCase {
    name: &'static str,
    desc: TextureDesc,
    kind: Texel,
    depth: bool,
    /// the texel bytes of (mip, layer, x, y)
    bytes: fn(u32, u32, u32, u32) -> Vec<u8>,
    /// what textureLoad gives for them (4 words; None: not compared)
    want: fn(u32, u32, u32, u32) -> [Option<u32>; 4],
}

fn rgba8_texel(m: u32, x: u32, y: u32) -> [u8; 4] {
    [(x * 15 + m) as u8, (y * 15 + m) as u8, (((x ^ y) * 13 + m * 50) & 255) as u8, (255 - m * 40) as u8]
}

fn unorm(b: u8) -> u32 {
    (b as f32 / 255.0).to_bits()
}

/// an rgba8unorm channel as a shader reads it: k / 255 within one unit in the last place.  The
/// correctly rounded quotient is what NVIDIA gives; Mesa's llvmpipe and Intel's storage reads
/// multiply by 1 / 255 and land next to it for some k, which Vulkan allows.  Every other format
/// stays exact.
fn unorm_near(got: u32, want: u32) -> bool {
    got.abs_diff(want) <= 1
}

/// words not equal to the reference but next to it (what unorm_near lets through)
fn next_to(got: &[u32], want: &[u32]) -> usize {
    got.iter().zip(want).filter(|(g, w)| g != w).count()
}

fn fetch_cases() -> Vec<FetchCase> {
    let s = TextureUsage::SAMPLED | TextureUsage::COPY_DST;
    vec![
        FetchCase {
            name: "rgba8unorm, 16 x 16, 5 mips",
            desc: TextureDesc { mips: 5, ..TextureDesc::d2(16, 16, Format::Rgba8Unorm, s) },
            kind: Texel::Float,
            depth: false,
            bytes: |m, _, x, y| rgba8_texel(m, x, y).to_vec(),
            want: |m, _, x, y| rgba8_texel(m, x, y).map(|b| Some(unorm(b))),
        },
        FetchCase {
            name: "r32float, 8 x 8, 4 mips",
            desc: TextureDesc { mips: 4, ..TextureDesc::d2(8, 8, Format::R32Float, s) },
            kind: Texel::Float,
            depth: false,
            bytes: |m, _, x, y| ((m * 1000 + y * 8 + x) as f32 + 0.5).to_le_bytes().to_vec(),
            want: |m, _, x, y| [Some(((m * 1000 + y * 8 + x) as f32 + 0.5).to_bits()), Some(0), Some(0), Some(1.0f32.to_bits())],
        },
        FetchCase {
            name: "r32float 2D array, 8 x 8 x 3 layers, 2 mips",
            desc: TextureDesc { layers: 3, mips: 2, ..TextureDesc::d2(8, 8, Format::R32Float, s) },
            kind: Texel::Float,
            depth: false,
            bytes: |m, l, x, y| ((l * 100 + m * 1000 + y * 8 + x) as f32).to_le_bytes().to_vec(),
            want: |m, l, x, y| [Some(((l * 100 + m * 1000 + y * 8 + x) as f32).to_bits()), Some(0), Some(0), Some(1.0f32.to_bits())],
        },
        FetchCase {
            name: "r32uint, 8 x 8",
            desc: TextureDesc::d2(8, 8, Format::R32Uint, s),
            kind: Texel::Uint,
            depth: false,
            bytes: |_, _, x, y| (0xC0DE_0000u32 | y << 8 | x).to_le_bytes().to_vec(),
            want: |_, _, x, y| [Some(0xC0DE_0000 | y << 8 | x), Some(0), Some(0), Some(1)],
        },
        FetchCase {
            name: "r32sint, 8 x 8",
            desc: TextureDesc::d2(8, 8, Format::R32Sint, s),
            kind: Texel::Sint,
            depth: false,
            bytes: |_, _, x, y| (-((y * 8 + x) as i32) - 77).to_le_bytes().to_vec(),
            want: |_, _, x, y| [Some((-((y * 8 + x) as i32) - 77) as u32), Some(0), Some(0), Some(1)],
        },
        FetchCase {
            name: "rgba32uint, 4 x 4",
            desc: TextureDesc::d2(4, 4, Format::Rgba32Uint, s),
            kind: Texel::Uint,
            depth: false,
            bytes: |_, _, x, y| bytes_of_words(&[x, y, x * y, 0xFFFF_FFFF - x]),
            want: |_, _, x, y| [Some(x), Some(y), Some(x * y), Some(0xFFFF_FFFF - x)],
        },
        FetchCase {
            name: "depth32float, 8 x 8 (the depth in x)",
            desc: TextureDesc::d2(8, 8, Format::Depth32Float, s),
            kind: Texel::Float,
            depth: true,
            bytes: |_, _, x, y| ((y * 8 + x) as f32 / 64.0).to_le_bytes().to_vec(),
            want: |_, _, x, y| [Some(((y * 8 + x) as f32 / 64.0).to_bits()), None, None, None],
        },
    ]
}

fn fetch(t: &mut T, dev: &Device) {
    for c in fetch_cases() {
        let d = c.desc;
        let bytes = c.bytes;
        let data = move |m: u32, l: u32| -> Vec<u8> {
            let (w, h) = ((d.width >> m).max(1), (d.height >> m).max(1));
            (0..h).flat_map(|y| (0..w).flat_map(move |x| bytes(m, l, x, y))).collect()
        };
        let Some(tex) = texture(t, dev, d, &data, c.name) else { continue };
        let words = samples::cs_fetch(3, c.kind, d.layers > 1, c.depth);
        let Some((bgl, pipe)) = cpipe(t, dev, &words, &[bind(0, BindingKind::Texture, Stages::COMPUTE),
                                                          bind(1, BindingKind::StorageBuffer, Stages::COMPUTE)],
                                     samples::FETCH_PUSH_SIZE, c.name) else { continue };
        let total: u32 = (0..d.mips).map(|m| ((d.width >> m).max(1)) * ((d.height >> m).max(1)) * d.layers * 4).sum();
        let Some(ob) = buf(t, dev, total as u64 * 4, BufferUsage::NONE) else { return };
        let Some(rb) = readback(t, dev, total as u64 * 4) else { return };
        let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
            BindEntry { binding: 0, resource: Resource::Texture(tex.view()) }, BindEntry { binding: 1, resource: whole(&ob) }] }),
                             "bind group") else { continue };
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &bg);
        let mut base = 0u32;
        for m in 0..d.mips {
            let (w, h) = ((d.width >> m).max(1), (d.height >> m).max(1));
            e.push_constants(0, &[w, h, d.layers, m, base]);
            e.dispatch(w.div_ceil(8), h.div_ceil(8), d.layers);
            // (each mip has its own range; the validation layer looks at the whole buffer)
            e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_WRITE);
            base += w * h * d.layers * 4;
        }
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        e.copy_buffer(&ob, 0, &rb, 0, total as u64 * 4);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        if run_wait(t, dev, e.finish(), c.name).is_none() {
            continue;
        }
        let mut got = vec![0u32; total as usize];
        if t.res(rb.read(0, &mut got), "read").is_none() {
            continue;
        }
        let unorm_format = d.format == Format::Rgba8Unorm;
        let (mut k, mut bad, mut beside) = (0usize, None, 0);
        for m in 0..d.mips {
            let (w, h) = ((d.width >> m).max(1), (d.height >> m).max(1));
            for l in 0..d.layers {
                for y in 0..h {
                    for x in 0..w {
                        let want = (c.want)(m, l, x, y);
                        for (ci, wv) in want.iter().enumerate() {
                            if let Some(wv) = wv {
                                let g = got[k + ci];
                                beside += (g != *wv) as u32;
                                if bad.is_none() && g != *wv && !(unorm_format && unorm_near(g, *wv)) {
                                    bad = Some(format!("mip {m} layer {l} ({x}, {y}) component {ci}: {g:#x}, expected {wv:#x}"));
                                }
                            }
                        }
                        k += 4;
                    }
                }
            }
        }
        let how = if unorm_format { format!("k / 255 ({beside} values one unit in the last place off)") } else { String::from("exact") };
        t.ok(bad.is_none(), &format!("fetch {}: every texel of every mip and layer {how}{}", c.name,
                                     bad.map_or(String::new(), |b| format!(" -- {b}"))));
    }
}

// ---------------------------------------------------------------- sample (compute)

/// a cube direction to the centre of texel (i, j) of face f (n x n), scaled by k
pub fn cube_dir(f: u32, i: u32, j: u32, n: u32, k: f32) -> [f32; 3] {
    let sc = 2.0 * (i as f32 + 0.5) / n as f32 - 1.0;
    let tc = 2.0 * (j as f32 + 0.5) / n as f32 - 1.0;
    let d = match f {
        0 => [1.0, -tc, -sc],
        1 => [-1.0, -tc, sc],
        2 => [sc, 1.0, tc],
        3 => [sc, -1.0, -tc],
        4 => [sc, -tc, 1.0],
        _ => [-sc, -tc, -1.0],
    };
    d.map(|v| v * k)
}

/// the face and texel nearest sampling of a cube (n x n faces) picks for direction d
/// (Vulkan's cube map face selection: the major axis, then sc / tc of its face)
pub fn cube_texel(d: [f32; 3], n: u32) -> (u32, u32, u32) {
    let [x, y, z] = d;
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (f, sc, tc, ma) = if ax >= ay && ax >= az {
        if x >= 0.0 { (0, -z, -y, ax) } else { (1, z, -y, ax) }
    } else if ay >= az {
        if y >= 0.0 { (2, x, z, ay) } else { (3, x, -z, ay) }
    } else if z >= 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    let s = 0.5 * (sc / ma + 1.0);
    let t = 0.5 * (tc / ma + 1.0);
    let i = (shader::bench::floor(s * n as f32) as i32).clamp(0, n as i32 - 1) as u32;
    let j = (shader::bench::floor(t * n as f32) as i32).clamp(0, n as i32 - 1) as u32;
    (f, i, j)
}

/// run cs_sample(dim) on tex with sampler at the (coordinate, lod) points -> the vec4 results
fn sample_points(t: &mut T, dev: &Device, dim: SampleDim, view: &TextureView, smp: &Sampler, pts: &[([f32; 4], f32)],
                 what: &str) -> Option<Vec<[f32; 4]>> {
    let words = samples::cs_sample(3, dim);
    let (bgl, pipe) = cpipe(t, dev, &words, &[bind(0, BindingKind::Texture, Stages::COMPUTE), bind(1, BindingKind::Sampler, Stages::COMPUTE),
                                              bind(2, BindingKind::StorageBufferRead, Stages::COMPUTE),
                                              bind(3, BindingKind::StorageBuffer, Stages::COMPUTE)], 4, what)?;
    let n = pts.len() as u32;
    let mut at: Vec<f32> = Vec::new();
    for (c, l) in pts {
        at.extend_from_slice(c);
        at.extend_from_slice(&[*l, 0.0, 0.0, 0.0]);
    }
    let ib = buf(t, dev, at.len() as u64 * 4, BufferUsage::NONE)?;
    t.res(dev.queue().write_buffer(&ib, 0, &at), "write coordinates")?;
    let ob = buf(t, dev, n as u64 * 16, BufferUsage::NONE)?;
    let rb = readback(t, dev, n as u64 * 16)?;
    let bg = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Texture(view) }, BindEntry { binding: 1, resource: Resource::Sampler(smp) },
        BindEntry { binding: 2, resource: whole(&ib) }, BindEntry { binding: 3, resource: whole(&ob) }] }), "bind group")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &[n]);
    e.dispatch(n.div_ceil(64), 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&ob, 0, &rb, 0, n as u64 * 16);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    run_wait(t, dev, e.finish(), what)?;
    let mut got = vec![0f32; n as usize * 4];
    t.res(rb.read(0, &mut got), "read")?;
    Some(got.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect())
}

/// round to nearest, ties to even
fn rne(x: f32) -> f32 {
    let f = shader::bench::floor(x);
    let d = x - f;
    if d > 0.5 || (d == 0.5 && (f as i64) % 2 != 0) { f + 1.0 } else { f }
}

fn sample(t: &mut T, dev: &Device) {
    let s = TextureUsage::SAMPLED | TextureUsage::COPY_DST;
    let Some(nearest) = t.res(dev.create_sampler(&SamplerDesc::NEAREST), "sampler") else { return };
    // 2D array, 4 x 4 x 4 layers: value = layer * 16 + y * 4 + x
    let ad = TextureDesc { layers: 4, ..TextureDesc::d2(4, 4, Format::R32Float, s) };
    let adata = |_: u32, l: u32| -> Vec<u8> { (0..16u32).flat_map(|k| ((l * 16 + k) as f32).to_le_bytes()).collect() };
    if let Some(arr) = texture(t, dev, ad, &adata, "2D array") {
        let layers_plain = [0.0f32, 1.0, 2.4, 2.6, 3.0, 5.0, -1.0, 0.49];
        let layers_ties = [0.5f32, 1.5, 2.5];
        for (zs, what) in [(&layers_plain[..], "layer = round(z) clamped to 0 .. 3"), (&layers_ties[..], "ties of z: round to even (RNE)")] {
            let mut pts = Vec::new();
            let mut want = Vec::new();
            for &z in zs {
                for k in [0u32, 5, 10, 15] {
                    let (x, y) = (k % 4, k / 4);
                    pts.push(([(x as f32 + 0.5) / 4.0, (y as f32 + 0.5) / 4.0, z, 0.0], 0.0));
                    let l = rne(z).clamp(0.0, 3.0) as u32;
                    want.push((l * 16 + k) as f32);
                }
            }
            if let Some(got) = sample_points(t, dev, SampleDim::D2Array, arr.view(), &nearest, &pts, "2D array") {
                let bad = (0..want.len()).find(|&i| got[i][0] != want[i]);
                t.ok(bad.is_none(), &format!("sample 2D array (nearest, LOD 0): {what}{}", bad.map_or(String::new(),
                    |i| format!(": z = {} gave {}, expected {}", pts[i].0[2], got[i][0], want[i]))));
            }
        }
    }
    // cube 4 x 4: value = face * 100 + y * 10 + x; every texel through its direction, unit and scaled
    let n = 4u32;
    let cd = TextureDesc { layers: 6, cube: true, ..TextureDesc::d2(n, n, Format::R32Float, s) };
    let cdata = |_: u32, f: u32| -> Vec<u8> { (0..16u32).flat_map(|k| ((f * 100 + (k / 4) * 10 + k % 4) as f32).to_le_bytes()).collect() };
    if let Some(cube) = texture(t, dev, cd, &cdata, "cube") {
        for scale in [1.0f32, 2.5, 0.125] {
            let mut pts = Vec::new();
            let mut want = Vec::new();
            for f in 0..6 {
                for j in 0..n {
                    for i in 0..n {
                        let d = cube_dir(f, i, j, n, scale);
                        let (ff, ii, jj) = cube_texel(d, n);
                        // the CPU face selection must give the texel the direction was made for
                        if (ff, ii, jj) != (f, i, j) {
                            t.fail(&format!("cube_texel({d:?}) = {:?}, expected {:?}", (ff, ii, jj), (f, i, j)));
                        }
                        pts.push(([d[0], d[1], d[2], 0.0], 0.0));
                        want.push((f * 100 + j * 10 + i) as f32);
                    }
                }
            }
            if let Some(got) = sample_points(t, dev, SampleDim::Cube, cube.view(), &nearest, &pts, "cube") {
                let bad = (0..want.len()).find(|&i| got[i][0] != want[i]);
                t.ok(bad.is_none(), &format!("sample cube (nearest): every texel of the 6 faces through its direction x {scale}{}",
                    bad.map_or(String::new(), |i| format!(": {:?} gave {}, expected {}", pts[i].0, got[i][0], want[i]))));
            }
        }
    }
    // cube array, two cubes of 2 x 2: value = cube * 1000 + face * 100 + y * 10 + x
    let ca = TextureDesc { layers: 12, cube: true, ..TextureDesc::d2(2, 2, Format::R32Float, s) };
    let cadata = |_: u32, l: u32| -> Vec<u8> {
        (0..4u32).flat_map(|k| (((l / 6) * 1000 + (l % 6) * 100 + (k / 2) * 10 + k % 2) as f32).to_le_bytes()).collect()
    };
    if NO_CUBE_ARRAYS {
        out("  SKIP sample cube array: the simulator's Metal has none");
    } else if let Some(cubes) = texture(t, dev, ca, &cadata, "cube array") {
        let mut pts = Vec::new();
        let mut want = Vec::new();
        for c in 0..2u32 {
            for f in 0..6 {
                for k in 0..4u32 {
                    let d = cube_dir(f, k % 2, k / 2, 2, 1.0);
                    pts.push(([d[0], d[1], d[2], c as f32], 0.0));
                    want.push((c * 1000 + f * 100 + (k / 2) * 10 + k % 2) as f32);
                }
            }
        }
        if let Some(got) = sample_points(t, dev, SampleDim::CubeArray, cubes.view(), &nearest, &pts, "cube array") {
            let bad = (0..want.len()).find(|&i| got[i][0] != want[i]);
            t.ok(bad.is_none(), &format!("sample cube array (nearest): every texel of 2 cubes{}",
                bad.map_or(String::new(), |i| format!(": {:?} gave {}, expected {}", pts[i].0, got[i][0], want[i]))));
        }
    }
}

// ---------------------------------------------------------------- storage

fn gpu_format(f: Fmt) -> Option<Format> {
    Some(match f {
        Fmt::Rgba8 => Format::Rgba8Unorm,
        Fmt::Rgba16f => Format::Rgba16Float,
        Fmt::R32f => Format::R32Float,
        Fmt::Rgba32f => Format::Rgba32Float,
        Fmt::R32ui => Format::R32Uint,
        Fmt::Rgba32ui => Format::Rgba32Uint,
        Fmt::R32i => Format::R32Sint,
        _ => return None,
    })
}

const STORAGE: [(Fmt, &str); 8] = [(Fmt::Rgba8, "rgba8unorm"), (Fmt::Rgba16f, "rgba16float"), (Fmt::R32f, "r32float"),
    (Fmt::Rgba32f, "rgba32float"), (Fmt::R32ui, "r32uint"), (Fmt::Rgba32ui, "rgba32uint"), (Fmt::R32i, "r32sint"),
    (Fmt::Rgba32i, "rgba32sint")];

fn storage(t: &mut T, dev: &Device) {
    let (w, h) = (16u32, 12u32);
    for (fmt, name) in STORAGE {
        let Some(format) = gpu_format(fmt) else {
            out(&format!("  note: storage {name}: lib/gpu has no such Format, skipped"));
            continue;
        };
        let usage = TextureUsage::STORAGE | TextureUsage::COPY_SRC;
        let tex = match dev.create_texture(&TextureDesc::d2(w, h, format, usage)) {
            Ok(x) => x,
            Err(GpuError::Unsupported(why)) => {
                out(&format!("  note: storage {name}: {why}"));
                continue;
            }
            Err(e) => {
                t.fail(&format!("storage {name}: texture: {e}"));
                continue;
            }
        };
        let st = [bind(0, BindingKind::StorageTexture, Stages::COMPUTE)];
        let Some((wbgl, wpipe)) = cpipe(t, dev, &samples::cs_storage_write(3, fmt), &st, 8, name) else { continue };
        let Some((rbgl, rpipe)) = cpipe(t, dev, &samples::cs_storage_read(3, fmt),
                                        &[st[0], bind(1, BindingKind::StorageBuffer, Stages::COMPUTE)], 8, name) else { continue };
        let texel_words = samples::storage_texel(fmt, 0, 0).len() as u32;
        let raw_bytes = (w * h * texel_words * 4) as u64;
        let out_words = w * h * 4 + 2;
        let Some(ob) = buf(t, dev, out_words as u64 * 4, BufferUsage::NONE) else { return };
        let Some(rb) = readback(t, dev, out_words as u64 * 4) else { return };
        let Some(rb_raw) = readback(t, dev, raw_bytes) else { return };
        let Some(wbg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &wbgl, entries: &[
            BindEntry { binding: 0, resource: Resource::StorageTexture(tex.view()) }] }), "bind group") else { continue };
        let Some(rbg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &rbgl, entries: &[
            BindEntry { binding: 0, resource: Resource::StorageTexture(tex.view()) }, BindEntry { binding: 1, resource: whole(&ob) }] }),
                              "bind group") else { continue };
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
        e.texture_barrier(&tex, Layout::Undefined, Layout::General);
        e.set_compute_pipeline(&wpipe);
        e.set_bind_group(0, &wbg);
        e.push_constants(0, &[w, h]);
        e.dispatch(w.div_ceil(8), h.div_ceil(8), 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ);
        e.set_compute_pipeline(&rpipe);
        e.set_bind_group(0, &rbg);
        e.push_constants(0, &[w, h]);
        e.dispatch(w.div_ceil(8), h.div_ceil(8), 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        e.copy_buffer(&ob, 0, &rb, 0, out_words as u64 * 4);
        e.texture_barrier(&tex, Layout::General, Layout::CopySrc);
        e.copy_texture_to_buffer(TextureRegion::mip(&tex, 0), &rb_raw, DataLayout::default());
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        if run_wait(t, dev, e.finish(), name).is_none() {
            continue;
        }
        let mut raw = vec![0u32; (raw_bytes / 4) as usize];
        let mut got = vec![0u32; out_words as usize];
        if t.res(rb_raw.read(0, &mut raw), "read").is_none() || t.res(rb.read(0, &mut got), "read").is_none() {
            continue;
        }
        let want_raw: Vec<u32> = (0..h).flat_map(|y| (0..w).flat_map(move |x| samples::storage_texel(fmt, x, y))).collect();
        t.ok(raw == want_raw, &format!("storage {name}: textureStore of {w} x {h} texels, the texture's bytes{}", first_diff(&raw, &want_raw)));
        let mut want: Vec<u32> = (0..h).flat_map(|y| (0..w).flat_map(move |x| samples::storage_read(fmt, x, y))).collect();
        want.extend_from_slice(&[w, h]);
        // rgba8unorm: k / 255 (unorm_near), the sizes after it exact
        let texels = want.len() - 2;
        let near = fmt == Fmt::Rgba8 && got.len() == want.len() && got[texels..] == want[texels..]
            && got[..texels].iter().zip(&want[..texels]).all(|(g, w)| unorm_near(*g, *w));
        let beside = if near { format!(" ({} values one unit in the last place off k / 255)", next_to(&got, &want)) } else { first_diff(&got, &want) };
        t.ok(got == want || near, &format!("storage {name}: textureLoad of every texel, textureDimensions = ({w}, {h}){beside}"));
    }
    // read-write: v -> v * 2 + 1 / v * 3 + 1 / v - 5
    for (fmt, name) in [(Fmt::R32f, "r32float"), (Fmt::R32ui, "r32uint"), (Fmt::R32i, "r32sint")] {
        let Some(format) = gpu_format(fmt) else { continue };
        let Some(tex) = t.res(dev.create_texture(&TextureDesc::d2(w, h, format, TextureUsage::STORAGE | TextureUsage::COPY_SRC)),
                              "texture") else { continue };
        let st = [bind(0, BindingKind::StorageTexture, Stages::COMPUTE)];
        let Some((wbgl, wpipe)) = cpipe(t, dev, &samples::cs_storage_write(3, fmt), &st, 8, name) else { continue };
        let Some((rwbgl, rwpipe)) = cpipe(t, dev, &samples::cs_storage_rw(3, fmt), &st, 8, name) else { continue };
        let Some(rb) = readback(t, dev, (w * h * 4) as u64) else { return };
        let Some(wbg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &wbgl, entries: &[
            BindEntry { binding: 0, resource: Resource::StorageTexture(tex.view()) }] }), "bind group") else { continue };
        let Some(rwbg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &rwbgl, entries: &[
            BindEntry { binding: 0, resource: Resource::StorageTexture(tex.view()) }] }), "bind group") else { continue };
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
        e.texture_barrier(&tex, Layout::Undefined, Layout::General);
        e.set_compute_pipeline(&wpipe);
        e.set_bind_group(0, &wbg);
        e.push_constants(0, &[w, h]);
        e.dispatch(w.div_ceil(8), h.div_ceil(8), 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ | Access::COMPUTE_WRITE);
        e.set_compute_pipeline(&rwpipe);
        e.set_bind_group(0, &rwbg);
        e.push_constants(0, &[w, h]);
        e.dispatch(w.div_ceil(8), h.div_ceil(8), 1);
        e.texture_barrier(&tex, Layout::General, Layout::CopySrc);
        e.copy_texture_to_buffer(TextureRegion::mip(&tex, 0), &rb, DataLayout::default());
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        if run_wait(t, dev, e.finish(), name).is_none() {
            continue;
        }
        let mut got = vec![0u32; (w * h) as usize];
        if t.res(rb.read(0, &mut got), "read").is_none() {
            continue;
        }
        let want: Vec<u32> = (0..h).flat_map(|y| (0..w).map(move |x| samples::storage_rw_ref(fmt, samples::storage_texel(fmt, x, y)[0]))).collect();
        t.ok(got == want, &format!("storage {name} read-write: textureLoad, change, textureStore in one shader{}", first_diff(&got, &want)));
    }
}

// ---------------------------------------------------------------- fragment shaders

/// a render target of s x s Rgba32Float, a draw of fs over it, the pixels read back
struct Frag {
    pipe: RenderPipeline,
    bgl: BindGroupLayout,
}

fn frag_pipeline(t: &mut T, dev: &Device, fs_words: &[u32], what: &str) -> Option<Frag> {
    for w in [&samples::vs_fullscreen(3)[..], fs_words] {
        if let Err(e) = shader::val::validate(w) {
            t.fail(&format!("{what}: shader::val: {}", e[0]));
            return None;
        }
    }
    let vs = t.res(ShaderModule::from_spirv(dev, &samples::vs_fullscreen(3)), "vertex shader")?;
    let fs = t.res(ShaderModule::from_spirv(dev, fs_words), "fragment shader")?;
    let bgl = t.res(dev.create_bind_group_layout(&[bind(0, BindingKind::Texture, Stages::FRAGMENT),
                                                   bind(1, BindingKind::Sampler, Stages::FRAGMENT)]), "layout")?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: samples::FS_PUSH_SIZE }),
                   "layout")?;
    let pipe = t.res(dev.create_render_pipeline(&RenderPipelineDesc {
        layout: &pl,
        vertex: ShaderStage::new(&vs, "main"),
        fragment: Some(ShaderStage::new(&fs, "main")),
        buffers: &[],
        topology: Topology::Triangles,
        cull: Cull::None,
        front_cw: false,
        targets: &[ColorTarget { format: Format::Rgba32Float, blend: None, write_mask: 0xF }],
        depth: None,
    }), &format!("{what}: pipeline"))?;
    Some(Frag { pipe, bgl })
}

/// draw over an s x s target: uv = frag_coord * scale + bias, value (lod / dref), mode -> the pixels
#[allow(clippy::too_many_arguments)]
fn draw(t: &mut T, dev: &Device, f: &Frag, view: &TextureView, smp: &Sampler, s: u32, scale: f32, bias: f32, value: f32,
        mode: u32, what: &str) -> Option<Vec<[f32; 4]>> {
    let target = t.res(dev.create_texture(&TextureDesc::d2(s, s, Format::Rgba32Float, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)),
                       "target")?;
    let rb = readback(t, dev, (s * s * 16) as u64)?;
    let bg = t.res(dev.create_bind_group(&BindGroupDesc { layout: &f.bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Texture(view) }, BindEntry { binding: 1, resource: Resource::Sampler(smp) }] }),
                   "bind group")?;
    let push: [u32; 6] = [scale.to_bits(), scale.to_bits(), bias.to_bits(), bias.to_bits(), value.to_bits(), mode];
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: target.view(), load: Load::Clear, clear: [9.0, 9.0, 9.0, 9.0], store: true,
                                   before: Layout::Undefined, after: Layout::CopySrc }],
        depth: None,
    });
    e.set_render_pipeline(&f.pipe);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &push);
    e.draw(3, 1, 0, 0);
    e.end_render();
    e.copy_texture_to_buffer(TextureRegion::mip(&target, 0), &rb, DataLayout::default());
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    run_wait(t, dev, e.finish(), what)?;
    let mut px = vec![0f32; (s * s * 4) as usize];
    t.res(rb.read(0, &mut px), "read")?;
    Some(px.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect())
}

/// the bilinear filter of mip `m` (w x h, clamp to edge) at uv, texels by texel(x, y)
fn bilinear(texel: &dyn Fn(u32, u32) -> [f32; 4], w: u32, h: u32, u: f32, v: f32) -> [f32; 4] {
    let (x, y) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
    let (x0, y0) = (shader::bench::floor(x), shader::bench::floor(y));
    let (a, b) = (x - x0, y - y0);
    let c = |i: f32, n: u32| (i as i32).clamp(0, n as i32 - 1) as u32;
    let (i0, i1, j0, j1) = (c(x0, w), c(x0 + 1.0, w), c(y0, h), c(y0 + 1.0, h));
    let (t00, t10, t01, t11) = (texel(i0, j0), texel(i1, j0), texel(i0, j1), texel(i1, j1));
    let mut o = [0.0f32; 4];
    for k in 0..4 {
        o[k] = (1.0 - a) * (1.0 - b) * t00[k] + a * (1.0 - b) * t10[k] + (1.0 - a) * b * t01[k] + a * b * t11[k];
    }
    o
}

fn max_err(got: &[[f32; 4]], want: &[[f32; 4]], comps: usize) -> f32 {
    got.iter().zip(want).flat_map(|(g, w)| (0..comps).map(move |k| (g[k] - w[k]).abs())).fold(0.0, f32::max)
}

fn fragment(t: &mut T, dev: &Device) {
    let s16 = TextureUsage::SAMPLED | TextureUsage::COPY_DST;
    let d = TextureDesc { mips: 5, ..TextureDesc::d2(16, 16, Format::Rgba8Unorm, s16) };
    let data = |m: u32, _: u32| -> Vec<u8> {
        let n = 16 >> m;
        (0..n).flat_map(|y| (0..n).flat_map(move |x| rgba8_texel(m, x, y))).collect()
    };
    let Some(tex) = texture(t, dev, d, &data, "fragment texture") else { return };
    let texel = |m: u32, x: u32, y: u32| rgba8_texel(m, x, y).map(|b| b as f32 / 255.0);
    let Some(f) = frag_pipeline(t, dev, &samples::fs_sample(3), "fs_sample") else { return };
    let Some(nearest) = t.res(dev.create_sampler(&SamplerDesc::NEAREST), "sampler") else { return };
    let Some(bilin) = t.res(dev.create_sampler(&SamplerDesc { mip: Filter::Nearest, ..SamplerDesc::LINEAR }), "sampler") else { return };
    let Some(trilin) = t.res(dev.create_sampler(&SamplerDesc::LINEAR), "sampler") else { return };
    // nearest: explicit LOD L and implicit LOD on a target of the mip's size: the mip's texels
    for (mode, mname) in [(0u32, "textureSampleLevel (explicit LOD)"), (1, "textureSample (implicit LOD: mip from derivatives)")] {
        let mut bad = None;
        let mut beside = 0;
        for l in 0..5u32 {
            let s = 16 >> l;
            let Some(px) = draw(t, dev, &f, tex.view(), &nearest, s, 1.0 / s as f32, 0.0, l as f32, mode, mname) else { return };
            for y in 0..s {
                for x in 0..s {
                    let want = texel(l, x, y);
                    let got = px[(y * s + x) as usize];
                    beside += (0..4).filter(|&c| got[c] != want[c]).count();
                    if bad.is_none() && !(0..4).all(|c| unorm_near(got[c].to_bits(), want[c].to_bits())) {
                        bad = Some(format!("mip {l} ({x}, {y}) = {got:?}, expected {want:?}"));
                    }
                }
            }
        }
        t.ok(bad.is_none(), &format!("fragment {mname}, nearest: mips 0 .. 4 = k / 255 ({beside} values one unit in the last place off){}",
                                     bad.map_or(String::new(), |b| format!(" -- {b}"))));
    }
    // bilinear at LOD 0, a quarter texel off the centres (16 x 16 target)
    if let Some(px) = draw(t, dev, &f, tex.view(), &bilin, 16, 1.0 / 16.0, 0.25 / 16.0, 0.0, 0, "bilinear") {
        let want: Vec<[f32; 4]> = (0..256u32).map(|k| {
            let (x, y) = (k % 16, k / 16);
            bilinear(&|i, j| texel(0, i, j), 16, 16, (x as f32 + 0.75) / 16.0, (y as f32 + 0.75) / 16.0)
        }).collect();
        let e = max_err(&px, &want, 4);
        t.ok(e <= LINEAR_TOL, &format!("fragment bilinear at LOD 0, quarter-texel offsets: max error {e:.2e} (tolerance {LINEAR_TOL:.2e})"));
    }
    // trilinear at LOD 1.5 (8 x 8 target at the centres of mip 1, quarter texels of mip 2)
    if let Some(px) = draw(t, dev, &f, tex.view(), &trilin, 8, 1.0 / 8.0, 0.0, 1.5, 0, "trilinear") {
        let want: Vec<[f32; 4]> = (0..64u32).map(|k| {
            let (u, v) = (((k % 8) as f32 + 0.5) / 8.0, ((k / 8) as f32 + 0.5) / 8.0);
            let a = bilinear(&|i, j| texel(1, i, j), 8, 8, u, v);
            let b = bilinear(&|i, j| texel(2, i, j), 4, 4, u, v);
            [0, 1, 2, 3].map(|c| 0.5 * a[c] + 0.5 * b[c])
        }).collect();
        let e = max_err(&px, &want, 4);
        t.ok(e <= LINEAR_TOL, &format!("fragment trilinear at LOD 1.5 (mips 1 and 2 half each): max error {e:.2e} (tolerance {LINEAR_TOL:.2e})"));
    }
}

fn compare(t: &mut T, dev: &Device) {
    let n = 8u32;
    let depth = |x: u32, y: u32| (y * n + x) as f32 / 64.0 + 1.0 / 128.0;
    let data = |_: u32, _: u32| -> Vec<u8> { (0..n * n).flat_map(|k| depth(k % n, k / n).to_le_bytes()).collect() };
    let Some(tex) = texture(t, dev, TextureDesc::d2(n, n, Format::Depth32Float, TextureUsage::SAMPLED | TextureUsage::COPY_DST), &data,
                            "depth texture") else { return };
    let Some(f) = frag_pipeline(t, dev, &samples::fs_compare(3), "fs_compare") else { return };
    for (cmp, cname, pass) in [(Compare::Less, "less", (|r: f32, d: f32| r < d) as fn(f32, f32) -> bool),
                               (Compare::GreaterEqual, "greater-equal", |r: f32, d: f32| r >= d)] {
        let Some(smp) = t.res(dev.create_sampler(&SamplerDesc { compare: Some(cmp), ..SamplerDesc::NEAREST }), "sampler") else { return };
        for (mode, mname) in [(0u32, "textureSampleCompareLevel"), (1, "textureSampleCompare")] {
            let mut bad = None;
            for dref in [0.0f32, 0.3, 0.5, 23.0 / 64.0 + 1.0 / 128.0, 1.0] {
                let Some(px) = draw(t, dev, &f, tex.view(), &smp, n, 1.0 / n as f32, 0.0, dref, mode, mname) else { return };
                for k in 0..n * n {
                    let want = pass(dref, depth(k % n, k / n)) as u32 as f32;
                    if bad.is_none() && px[k as usize][0] != want {
                        bad = Some(format!("dref {dref} texel {k}: {}, expected {want}", px[k as usize][0]));
                    }
                }
            }
            t.ok(bad.is_none(), &format!("compare {cname}, nearest, {mname}: every texel, 5 references exact{}",
                                         bad.map_or(String::new(), |b| format!(" -- {b}"))));
        }
    }
    // linear comparison (PCF): the weighted mean of the four comparisons
    let Some(smp) = t.res(dev.create_sampler(&SamplerDesc { compare: Some(Compare::Less), mip: Filter::Nearest, ..SamplerDesc::LINEAR }),
                          "sampler") else { return };
    let dref = 0.5f32;
    if let Some(px) = draw(t, dev, &f, tex.view(), &smp, n, 1.0 / n as f32, 0.25 / n as f32, dref, 0, "pcf") {
        let want: Vec<[f32; 4]> = (0..n * n).map(|k| {
            let r = bilinear(&|i, j| [(dref < depth(i, j)) as u32 as f32; 4], n, n, ((k % n) as f32 + 0.75) / n as f32,
                             ((k / n) as f32 + 0.75) / n as f32);
            [r[0], 0.0, 0.0, 1.0]
        }).collect();
        let e = max_err(&px, &want, 1);
        t.ok(e <= LINEAR_TOL, &format!("compare less, linear (PCF), quarter-texel offsets: max error {e:.2e} (tolerance {LINEAR_TOL:.2e})"));
    }
}

// ---------------------------------------------------------------- queries

fn queries(t: &mut T, dev: &Device) {
    if NO_CUBE_ARRAYS {
        out("  SKIP queries: one of the textures is a cube array");
        return;
    }
    let s = TextureUsage::SAMPLED;
    let descs = [
        TextureDesc { mips: 5, ..TextureDesc::d2(16, 8, Format::Rgba8Unorm, s) },
        TextureDesc { layers: 3, mips: 2, ..TextureDesc::d2(8, 8, Format::R32Float, s) },
        TextureDesc { layers: 6, mips: 3, cube: true, ..TextureDesc::d2(8, 8, Format::Rgba8Unorm, s) },
        TextureDesc { layers: 12, mips: 2, cube: true, ..TextureDesc::d2(4, 4, Format::Rgba8Unorm, s) },
        TextureDesc { mips: 2, ..TextureDesc::d2(8, 4, Format::Depth32Float, s) },
    ];
    let mut texs = Vec::new();
    for d in descs {
        let Some(x) = t.res(dev.create_texture(&d), "texture") else { return };
        texs.push(x);
    }
    let Some(view) = t.res(texs[0].create_view(&ViewDesc { dim: ViewDim::D2, base_mip: 1, mips: 3, base_layer: 0, layers: 1 }), "view")
    else { return };
    let Some(stex) = t.res(dev.create_texture(&TextureDesc::d2(6, 10, Format::R32Float, TextureUsage::STORAGE)), "storage texture")
    else { return };
    let mut bindings: Vec<Binding> = (0..samples::QUERY_SAMPLED).map(|k| bind(k, BindingKind::Texture, Stages::COMPUTE)).collect();
    bindings.push(bind(samples::QUERY_SAMPLED, BindingKind::StorageTexture, Stages::COMPUTE));
    bindings.push(bind(samples::QUERY_SAMPLED + 1, BindingKind::StorageBuffer, Stages::COMPUTE));
    let Some((bgl, pipe)) = cpipe(t, dev, &samples::cs_query(3), &bindings, 0, "cs_query") else { return };
    let words = 12 * samples::QUERY_SAMPLED + 4;
    let Some(ob) = buf(t, dev, words as u64 * 4, BufferUsage::NONE) else { return };
    let Some(rb) = readback(t, dev, words as u64 * 4) else { return };
    let mut entries: Vec<BindEntry> = Vec::new();
    for (k, x) in texs.iter().enumerate() {
        entries.push(BindEntry { binding: k as u32, resource: Resource::Texture(x.view()) });
    }
    entries.push(BindEntry { binding: 5, resource: Resource::Texture(&view) });
    entries.push(BindEntry { binding: samples::QUERY_SAMPLED, resource: Resource::StorageTexture(stex.view()) });
    entries.push(BindEntry { binding: samples::QUERY_SAMPLED + 1, resource: whole(&ob) });
    let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &entries }), "bind group") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    for x in &texs {
        e.texture_barrier(x, Layout::Undefined, Layout::Sampled);
    }
    e.texture_barrier(&stex, Layout::Undefined, Layout::General);
    e.fill_buffer(&ob, 0, words as u64 * 4, 0);
    e.barrier(Access::COPY_WRITE, Access::COMPUTE_WRITE);
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.dispatch(1, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&ob, 0, &rb, 0, words as u64 * 4);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "queries").is_none() {
        return;
    }
    let mut got = vec![0u32; words as usize];
    if t.res(rb.read(0, &mut got), "read").is_none() {
        return;
    }
    // (size at mip 0, size at mip 1, levels) as the descriptions give them (cube arrays: cubes)
    let want: [([u32; 3], [u32; 3], u32); 6] = [
        ([16, 8, 0], [8, 4, 0], 5),
        ([8, 8, 3], [4, 4, 3], 2),
        ([8, 8, 0], [4, 4, 0], 3),
        ([4, 4, 2], [2, 2, 2], 2),
        ([8, 4, 0], [4, 2, 0], 2),
        ([8, 4, 0], [4, 2, 0], 3),
    ];
    let names = ["2D 16 x 8, 5 mips", "2D array 8 x 8 x 3, 2 mips", "cube 8, 3 mips", "cube array 4 x 2 cubes, 2 mips",
                 "depth 8 x 4, 2 mips", "view of mips 1 .. 3 of the 2D"];
    for (b, (s0, s1, lv)) in want.iter().enumerate() {
        let g = &got[12 * b..12 * b + 12];
        let ok = g[0..3] == s0[..] && g[4..7] == s1[..] && g[8] == *lv;
        t.ok(ok, &format!("queries {}: textureDimensions(t, 0) = {:?}, (t, 1) = {:?}, textureNumLevels = {} (got {:?} / {:?} / {})",
                          names[b], s0, s1, lv, &g[0..3], &g[4..7], g[8]));
    }
    let g = &got[12 * samples::QUERY_SAMPLED as usize..];
    t.ok(g[0..2] == [6, 10], &format!("queries storage r32float 6 x 10: textureDimensions = {:?} (OpImageQuerySize)", &g[0..2]));
}

// ---------------------------------------------------------------- bindless

fn bindless(t: &mut T, dev: &Device) {
    if !dev.features().bindless {
        out(&format!("  SKIP bindless textures: level {:?}", dev.level()));
        return;
    }
    let Some(table) = t.res(dev.create_bindless(&BindlessDesc { textures: 8, samplers: 4, buffers: 0, storage_textures: 0 }), "table")
    else { return };
    // three 2 x 2 rgba8 textures, distinct per slot; samplers 0 nearest, 1 linear
    let px = |s: u32, k: u32| -> [u8; 4] { [(s * 80 + k * 16) as u8, (255 - s * 60 - k * 8) as u8, (k * 60) as u8, 255] };
    let mut texs = Vec::new();
    for s in 0..3u32 {
        let data = move |_: u32, _: u32| -> Vec<u8> { (0..4u32).flat_map(|k| px(s, k)).collect() };
        let Some(x) = texture(t, dev, TextureDesc::d2(2, 2, Format::Rgba8Unorm, TextureUsage::SAMPLED | TextureUsage::COPY_DST),
                              &data, "bindless texture") else { return };
        t.res(table.set_texture(s, x.view()), "set_texture");
        texs.push(x);
    }
    let Some(nearest) = t.res(dev.create_sampler(&SamplerDesc::NEAREST), "sampler") else { return };
    let Some(linear) = t.res(dev.create_sampler(&SamplerDesc::LINEAR), "sampler") else { return };
    t.res(table.set_sampler(0, &nearest), "set_sampler");
    t.res(table.set_sampler(1, &linear), "set_sampler");
    let words = samples::cs_bindless_sample();
    if let Err(e) = shader::val::validate(&words) {
        t.fail(&format!("cs_bindless_sample: shader::val: {}", e[0]));
        return;
    }
    let Some(m) = t.res(ShaderModule::from_spirv(dev, &words), "module") else { return };
    let Some(obgl) = t.res(dev.create_bind_group_layout(&[bind(0, BindingKind::StorageBuffer, Stages::COMPUTE)]), "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[table.layout(), &obgl], push_constants: 20 }), "layout")
    else { return };
    let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }),
                           "pipeline") else { return };
    let n = 12u32;
    let Some(ob) = buf(t, dev, n as u64 * 16, BufferUsage::NONE) else { return };
    let Some(rb) = readback(t, dev, n as u64 * 16) else { return };
    let Some(obg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &obgl, entries: &[BindEntry { binding: 0, resource: whole(&ob) }] }),
                          "bind group") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, table.group());
    e.set_bind_group(1, &obg);
    e.push_constants(0, &[n, 3, 2, 0.5f32.to_bits(), 0.5f32.to_bits()]);
    e.dispatch(1, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&ob, 0, &rb, 0, n as u64 * 16);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "bindless sample").is_none() {
        return;
    }
    let mut got = vec![0f32; n as usize * 4];
    if t.res(rb.read(0, &mut got), "read").is_none() {
        return;
    }
    // at (0.5, 0.5) of 2 x 2: nearest takes texel (1, 1), linear the mean of the four
    let mut err = 0.0f32;
    for i in 0..n {
        let (s, smp) = (i % 3, i % 2);
        let want: [f32; 4] = if smp == 0 {
            px(s, 3).map(|b| b as f32 / 255.0)
        } else {
            let mut o = [0.0f32; 4];
            for k in 0..4 {
                for c in 0..4 {
                    o[c] += px(s, k)[c] as f32 / 255.0 / 4.0;
                }
            }
            o
        };
        for c in 0..4 {
            err = err.max((got[i as usize * 4 + c] - want[c]).abs());
        }
    }
    t.ok(err <= LINEAR_TOL, &format!("bindless: {n} invocations sample texs[i % 3] with smps[i % 2] (NonUniform): max error {err:.2e}"));
}

pub fn textures(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    fetch(t, dev);
    sample(t, dev);
    storage(t, dev);
    fragment(t, dev);
    compare(t, dev);
    queries(t, dev);
    bindless(t, dev);
}
