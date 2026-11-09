// tex -- render3dtest's tests of textures and mirrored instances (G5b), on Voll and on a device limited to
// Basis, against the ray caster (cpu.rs): nearest texels, levels and wrap modes exactly, sRGB and the mip
// levels against its own box filter (also read back from the arrays), trilinear filtering within a stated
// tolerance, the five size classes, many textures in one list, arrays growing, resampled sizes, MASK, and
// mirrored instances in lists of their own.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Material, MeshId, Mip, Renderer, RendererDesc, Sampling, Scene, TextureData, TextureId, TextureRef, Transform, Wrap};

use crate::common::*;
use crate::cpu::{self, Look, RWrap, RefSampling, RefScene, RefTexture};
use crate::gltfgen::picture;
use crate::tests::{T, on_levels, ref_camera, shot, sun_of};

/// a texture of a test: what the renderer gets, and what the reference holds (at the class's size)
pub struct Tex {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub srgb: bool,
    pub nearest: bool,
    /// the texels at the class's size (the renderer's resampling, done again here)
    pub reference: Vec<u8>,
}

impl Tex {
    /// a texture of a class's size: the reference is itself
    pub fn exact(size: u32, rgba: Vec<u8>, srgb: bool) -> Tex {
        Tex { w: size, h: size, reference: rgba.clone(), rgba, srgb, nearest: false }
    }
}

/// random texels (fast for the large classes)
pub fn noise(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut r = Rng(seed);
    (0..w * h).flat_map(|_| { let v = r.next(); [v as u8, (v >> 8) as u8, (v >> 16) as u8, 255] }).collect()
}

pub fn ref_sampling(s: Sampling) -> RefSampling {
    let w = |w: Wrap| match w {
        Wrap::Repeat => RWrap::Repeat,
        Wrap::Clamp => RWrap::Clamp,
        Wrap::Mirror => RWrap::Mirror,
    };
    RefSampling { mag_nearest: s.mag_nearest, min_nearest: s.min_nearest, mip: match s.mip { Mip::None => 0, Mip::Nearest => 1, Mip::Linear => 2 }, wrap: [w(s.wrap_u), w(s.wrap_v)] }
}

/// what the reference makes of a material (texture ids are the reference's texture indices)
pub fn look_of(m: &Material) -> Look {
    let f = m.base_color;
    let tex = |t: Option<TextureRef>| t.map(|t| (t.texture.0 as usize, ref_sampling(t.sampling)));
    let e = m.emissive;
    Look { factor: [f[0] as f64, f[1] as f64, f[2] as f64, f[3] as f64], texture: tex(m.base_color_texture), cutoff: m.alpha_cutoff.map(|c| c as f64),
           metallic: m.metallic as f64, roughness: m.roughness as f64, mr_texture: tex(m.metallic_roughness_texture), normal_texture: tex(m.normal_texture),
           normal_scale: m.normal_scale as f64, occlusion_texture: tex(m.occlusion_texture), occlusion: m.occlusion_strength as f64,
           emissive: [e[0] as f64, e[1] as f64, e[2] as f64], emissive_texture: tex(m.emissive_texture) }
}

pub struct Built {
    pub r: Renderer,
    pub s: Scene,
    pub rs: RefScene,
    pub ids: Vec<render3d::InstanceId>,
}

/// meshes, textures, one material per item, a scene and its reference
pub fn build(t: &mut T, dev: &Device, geos: &[Geo], texs: &[Tex], items: &[(Spec, Material)], cam: &Camera) -> Option<Built> {
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let sun = sun_of();
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for x in texs {
        t.res(r.add_texture(&TextureData { width: x.w, height: x.h, rgba: &x.rgba, srgb: x.srgb, nearest: x.nearest }), "add_texture")?;
        let size = render3d::CLASS_MIN << render3d::class_of(x.w, x.h);
        rs.textures.push(RefTexture::new(size, &x.reference, x.srgb));
    }
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_sun(&sun);
    s.set_camera(cam);
    let mut ids = Vec::new();
    for (spec, m) in items {
        let id = t.res(r.add_material(m), "add_material")?;
        let mut sp = *spec;
        sp.material = id.0;
        ids.push(t.res(s.add(sp.instance()), "Scene::add")?);
        rs.add_look(&geos[spec.mesh as usize], &sp, look_of(m));
    }
    Some(Built { r, s, rs, ids })
}

fn check(t: &mut T, b: &Built, cam: &Camera, off: &Off, img: &[u8], tol: u8, what: &str) -> cpu::Compared {
    let c = cpu::compare_tol(&b.rs, &ref_camera(cam), off.w, off.h, CLEAR8, img, None, tol);
    t.ok(c.fine(), &format!("{what}: {}", c.text()));
    c
}

/// a square -1 ..= 1 in the xy plane facing +z, uv from (u0, v0) to (u1, v1)
pub fn plate_uv(u: [f32; 2], v: [f32; 2]) -> Geo {
    let mut g = plate();
    g.uv = vec![[u[0], v[0]], [u[1], v[0]], [u[1], v[1]], [u[0], v[1]]];
    g
}

fn textured(texture: u32, sampling: Sampling) -> Material {
    Material::textured(TextureRef { texture: TextureId(texture), sampling })
}

const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn textures(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = textures_on(t, dev, level); });
}

fn textures_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let pic = picture(128, 128, 5, &|_, _| 255);
    let near_cam = |d: f64| Camera::look_at([0.0, 0.0, d], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let plate = Spec { mesh: 0, material: 0, pos: [0.0; 3], rot: NO_TURN, scale: [4.5, 2.5, 1.0] };
    let geos = [plate_uv([-0.5, 1.5], [-0.25, 0.75])];
    let samp = |mag_nearest, min_nearest, mip, wrap_u, wrap_v| Sampling { mag_nearest, min_nearest, mip, wrap_u, wrap_v };

    // the wrap modes, magnified: nearest exactly, bilinear to within 2 (the hardware's weights have 8 bits)
    for (name, wu, wv, nearest) in [("repeat x clamp", Wrap::Repeat, Wrap::Clamp, true), ("clamp x mirror", Wrap::Clamp, Wrap::Mirror, true),
                                    ("mirror x repeat", Wrap::Mirror, Wrap::Repeat, true), ("clamp x mirror, bilinear", Wrap::Clamp, Wrap::Mirror, false),
                                    ("repeat, bilinear", Wrap::Repeat, Wrap::Repeat, false)] {
        let cam = near_cam(5.0);
        let s = samp(nearest, nearest, Mip::Nearest, wu, wv);
        let mut b = build(t, dev, &geos, &[Tex::exact(128, pic.clone(), true)], &[(plate, textured(0, s))], &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "wraps")?;
        check(t, &b, &cam, &off, &img, if nearest { 1 } else { 2 }, &format!("{level}: sRGB 128, magnified, {name}"));
    }
    // levels: rho 2.6 and 5.2 (lambda 1.38, 2.38): the nearest level's texels exactly, its box filter's
    for (d, lvl) in [(15.0, 1), (30.0, 2)] {
        let cam = near_cam(d);
        let s = samp(true, true, Mip::Nearest, Wrap::Repeat, Wrap::Repeat);
        let mut b = build(t, dev, &geos, &[Tex::exact(128, pic.clone(), true)], &[(plate, textured(0, s))], &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "levels")?;
        check(t, &b, &cam, &off, &img, 1, &format!("{level}: sRGB 128 at level {lvl}, nearest texels of the nearest level"));
        let s = samp(false, true, Mip::Linear, Wrap::Clamp, Wrap::Repeat);
        let mut b = build(t, dev, &geos, &[Tex::exact(128, pic.clone(), true)], &[(plate, textured(0, s))], &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "levels")?;
        check(t, &b, &cam, &off, &img, 2, &format!("{level}: around level {lvl}, nearest texels of two levels mixed, clamped"));
    }
    // the levels in the array against the reference's box filter, byte for byte
    {
        let mut b = build(t, dev, &geos, &[Tex::exact(128, pic.clone(), true), Tex::exact(128, pic.clone(), false)], &[], &near_cam(5.0))?;
        let _ = shot(t, dev, &mut off, &mut b.r, &mut b.s, "upload")?;
        for srgb in [true, false] {
            let (diff1, worse) = read_levels(t, dev, &b.r, 0, srgb, 0, &b.rs.textures[!srgb as usize])?;
            t.ok(worse == 0 && diff1 * 1000 <= 128 * 128 * 4, &format!("{level}: the 8 levels of a 128 {} texture read back against a box filter of its own: {diff1} bytes differ by 1, {worse} by more",
                                                                  if srgb { "sRGB" } else { "linear" }));
        }
    }
    // trilinear at a grazing angle: the sampler's own path (repeat), and done in the shader (clamp, mirror)
    {
        let ground = [plate_uv([0.0, 6.0], [0.0, 24.0])];
        let flat = Spec { mesh: 0, material: 0, pos: [0.0, -1.0, -12.0], rot: quat([1.0, 0.0, 0.0], -1.45), scale: [6.0, 14.0, 1.0] };
        let cam = Camera::look_at([0.0, 0.6, 2.0], [0.0, -0.4, -10.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
        for (name, s, tol) in [("repeat, trilinear (the sampler's)", Sampling::LINEAR, 6), ("clamp x repeat, trilinear (in the shader)", samp(false, false, Mip::Linear, Wrap::Clamp, Wrap::Repeat), 6),
                               ("mirror, bilinear of the nearest level", samp(false, false, Mip::Nearest, Wrap::Mirror, Wrap::Mirror), 6)] {
            let mut b = build(t, dev, &ground, &[Tex::exact(128, pic.clone(), true)], &[(flat, textured(0, s))], &cam)?;
            let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "trilinear")?;
            check(t, &b, &cam, &off, &img, tol, &format!("{level}: a floor at a grazing angle, {name}, within {tol}"));
        }
    }
    // every class, magnified, exactly
    for k in 0..render3d::CLASSES as u32 {
        let size = render3d::CLASS_MIN << k;
        let span = 0.8 * 296.0 / size as f32;
        let g = [plate_uv([0.1, 0.1 + span], [0.2, 0.2 + span * 0.55])];
        let cam = near_cam(5.0);
        let mut b = build(t, dev, &g, &[Tex::exact(size, noise(size, size, 40 + k), k % 2 == 0)], &[(plate, textured(0, Sampling::NEAREST))], &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "classes")?;
        let info = b.r.texture_info(TextureId(0))?;
        check(t, &b, &cam, &off, &img, 1, &format!("{level}: class {k} ({size}, {}), layer {}, nearest, magnified", if info.srgb { "sRGB" } else { "linear" }, info.layer));
    }
    // 40 instances in one list, each with its material and texture (4 x 4, repeated to 128): the arrays grow
    let cubes = [cube()];
    let mut texs = Vec::new();
    let mut items = Vec::new();
    for i in 0..40u32 {
        let small = noise(4, 4, 100 + i);
        texs.push(Tex { w: 4, h: 4, reference: nearest_resample(&small, 4, 4, 128), rgba: small, srgb: i % 3 != 0, nearest: true });
        let (x, y) = ((i % 8) as f64 * 2.6 - 9.1, (i / 8) as f64 * 2.6 - 5.2);
        items.push((Spec { mesh: 0, material: 0, pos: [x, y, 0.0], rot: quat(cpu::norm([1.0, 0.7, 0.2]), 0.5 + i as f64 * 0.1), scale: [0.9; 3] }, textured(i, Sampling::NEAREST)));
    }
    let cam = Camera::look_at([0.0, 0.0, 16.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut b = build(t, dev, &cubes, &texs, &items, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "40 textures")?;
    t.ok(b.s.lists().len() == 1, &format!("{level}: 40 instances of one mesh with 40 materials and textures: {} list", b.s.lists().len()));
    check(t, &b, &cam, &off, &img, 1, &format!("{level}: 40 cubes, 40 textures (sRGB and linear), one list"));
    // more textures: the arrays grow, the image stays
    let before = b.r.texture_bytes();
    for i in 0..90u32 {
        let x = noise(4 + i % 5, 4, 300 + i);
        t.res(b.r.add_texture(&TextureData { width: 4 + i % 5, height: 4, rgba: &x, srgb: i % 2 == 0, nearest: true }), "add_texture")?;
    }
    let img2 = shot(t, dev, &mut off, &mut b.r, &mut b.s, "grown")?;
    let (n, m) = diff(&img, &img2);
    t.ok(n == 0, &format!("{level}: 90 textures more (arrays from {} to {} MiB): {n} pixels differ (largest {m})", before >> 20, b.r.texture_bytes() >> 20));
    // sizes that are no class: bilinear (sRGB in linear light) and nearest resampling, against its own
    {
        let odd = picture(100, 60, 21, &|_, _| 255);
        let odd2 = noise(48, 20, 22);
        let texs = [Tex { w: 100, h: 60, reference: bilinear_resample(&odd, 100, 60, 128, true), rgba: odd, srgb: true, nearest: false },
                    Tex { w: 48, h: 20, reference: nearest_resample(&odd2, 48, 20, 128), rgba: odd2, srgb: false, nearest: true }];
        for (i, name) in ["100 x 60, bilinear", "48 x 20, nearest"].iter().enumerate() {
            let cam = near_cam(5.0);
            let g = [plate_uv([0.0, 1.0], [0.0, 1.0])];
            let sp = Spec { scale: [2.4, 2.4, 1.0], ..plate };
            let mut b = build(t, dev, &g, &texs[i..i + 1], &[(sp, textured(0, Sampling::NEAREST))], &cam)?;
            let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "resampled")?;
            check(t, &b, &cam, &off, &img, 2, &format!("{level}: {name} resampled to 128 (against its own resampling, within 2)"));
        }
        let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
        let wide = noise(4096, 64, 23);
        let id = t.res(r.add_texture(&TextureData { width: 4096, height: 64, rgba: &wide, srgb: true, nearest: false }), "add_texture")?;
        let info = r.texture_info(id)?;
        t.ok(info.size == 2048 && info.class == 4 && info.resampled, &format!("{level}: 4096 x 64 goes to class {} ({}), resampled {}", info.class, info.size, info.resampled));
    }
    // MASK: holes in the alpha, a plate behind; a factor's alpha below the cutoff hides all
    {
        let holes = picture(128, 128, 31, &|x, y| if ((x / 8) + (y / 8)) % 3 == 0 { 40 } else { 220 });
        let g = [plate_uv([0.0, 1.5], [0.0, 0.75]), crate::common::plate()];
        let cam = near_cam(5.0);
        let back = Spec { mesh: 1, material: 0, pos: [0.0, 0.0, -1.0], rot: NO_TURN, scale: [6.0, 4.0, 1.0] };
        let mut front = textured(0, Sampling::NEAREST);
        front.alpha_cutoff = Some(0.5);
        let items = [(plate, front), (back, Material::color(0.2, 0.9, 0.4))];
        let mut b = build(t, dev, &g, &[Tex::exact(128, holes.clone(), true)], &items, &cam)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "mask")?;
        let c = check(t, &b, &cam, &off, &img, 1, &format!("{level}: MASK, cutoff 0.5: the plate behind through the holes"));
        let green = img.chunks_exact(4).filter(|p| p[1] > 120 && p[0] < 60 && p[2] < 90).count();
        t.ok(green > 1000 && c.objects_seen == 2, &format!("{level}: MASK: {green} pixels of the plate behind seen"));
        front.base_color[3] = 0.4;
        let mut b2 = build(t, dev, &g, &[Tex::exact(128, holes, true)], &[(plate, front), (back, Material::color(0.2, 0.9, 0.4))], &cam)?;
        let img2 = shot(t, dev, &mut off, &mut b2.r, &mut b2.s, "mask")?;
        let mut b3 = build(t, dev, &g, &[], &[(back, Material::color(0.2, 0.9, 0.4))], &cam)?;
        let img3 = shot(t, dev, &mut off, &mut b3.r, &mut b3.s, "mask")?;
        let (n, m) = diff(&img2, &img3);
        t.ok(n == 0, &format!("{level}: MASK with the factor's alpha 0.4 < 0.5: against the plate behind alone {n} pixels differ (largest {m})"));
    }
    // what add_texture and materials refuse
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    let inv = |x: gpu::Result<()>| matches!(x, Err(GpuError::Invalid(_)));
    t.ok(inv(r.add_texture(&TextureData { width: 0, height: 4, rgba: &[], srgb: true, nearest: false }).map(|_| ())), &format!("{level}: a texture 0 wide: Invalid"));
    t.ok(inv(r.add_texture(&TextureData { width: 4, height: 4, rgba: &[0; 60], srgb: true, nearest: false }).map(|_| ())), &format!("{level}: 60 bytes for 4 x 4: Invalid"));
    t.ok(inv(r.add_material(&textured(0, Sampling::LINEAR)).map(|_| ())), &format!("{level}: a material with a texture the renderer lacks: Invalid"));
    let mut m = Material::color(1.0, 1.0, 1.0);
    m.alpha_cutoff = Some(-0.5);
    t.ok(inv(r.add_material(&m).map(|_| ())), &format!("{level}: a negative alpha cutoff: Invalid"));
    Some(())
}

/// the levels of a layer of a class's array read back, against a reference texture: (bytes off by 1, more)
fn read_levels(t: &mut T, dev: &Device, r: &Renderer, class: u32, srgb: bool, layer: u32, want: &RefTexture) -> Option<(u32, u32)> {
    let tex = r.texture_array(class, srgb)?;
    t.res(dev.wait_idle(), "wait_idle")?;
    let size = render3d::CLASS_MIN << class;
    let levels = tex.mips();
    let row = |l: u32| ((size >> l).max(1) * 4).div_ceil(256) * 256;
    let place: Vec<u64> = (0..levels).scan(0u64, |at, l| { let p = *at; *at += row(l) as u64 * (size >> l).max(1) as u64; Some(p) }).collect();
    let total = place[levels as usize - 1] + row(levels - 1) as u64;
    let rb = t.res(dev.create_buffer(&BufferDesc { size: total, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "read-back")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.texture_barrier(tex, Layout::Sampled, Layout::CopySrc);
    for l in 0..levels {
        let n = (size >> l).max(1);
        e.copy_texture_to_buffer(TextureRegion { texture: tex, mip: l, origin: [0; 3], size: [n, n, 1], base_layer: layer, layers: 1 }, &rb,
                                 DataLayout { offset: place[l as usize], bytes_per_row: row(l), rows_per_image: n });
    }
    e.texture_barrier(tex, Layout::CopySrc, Layout::Sampled);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut raw = vec![0u8; total as usize];
    t.res(rb.read(0, &mut raw), "read")?;
    let (mut one, mut more) = (0, 0);
    for l in 0..levels as usize {
        let n = (size as usize >> l).max(1);
        for y in 0..n {
            for x in 0..n {
                let got = &raw[place[l] as usize + y * row(l as u32) as usize + x * 4..][..4];
                let w = want.levels[l][y * n + x];
                for k in 0..4 {
                    match got[k].abs_diff(w[k]) {
                        0 => {}
                        1 => one += 1,
                        _ => more += 1,
                    }
                }
            }
        }
    }
    Some((one, more))
}

/// nearest resampling to size x size, written here: texel i of the target takes floor((i + 0.5) n / size)
pub fn nearest_resample(src: &[u8], w: u32, h: u32, size: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let sy = (((y as f64 + 0.5) * h as f64 / size as f64) as u32).min(h - 1);
        for x in 0..size {
            let sx = (((x as f64 + 0.5) * w as f64 / size as f64) as u32).min(w - 1);
            let at = ((sy * w + sx) * 4) as usize;
            out.extend_from_slice(&src[at..at + 4]);
        }
    }
    out
}

/// bilinear resampling upwards to size x size (texel centres, edges clamped; sRGB in linear light), here
pub fn bilinear_resample(src: &[u8], w: u32, h: u32, size: u32, srgb: bool) -> Vec<u8> {
    let lin = |c: u8, k: usize| if srgb && k < 3 { cpu::srgb_decode(c as f64 / 255.0) } else { c as f64 / 255.0 };
    let at = |x: i64, y: i64, k: usize| lin(src[((y.clamp(0, h as i64 - 1) * w as i64 + x.clamp(0, w as i64 - 1)) * 4) as usize + k], k);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let fy = (y as f64 + 0.5) * h as f64 / size as f64 - 0.5;
        let (y0, b) = (cpu::floor(fy), fy - cpu::floor(fy));
        for x in 0..size {
            let fx = (x as f64 + 0.5) * w as f64 / size as f64 - 0.5;
            let (x0, a) = (cpu::floor(fx), fx - cpu::floor(fx));
            for k in 0..4 {
                let (x0, y0) = (x0 as i64, y0 as i64);
                let v = (at(x0, y0, k) * (1.0 - a) + at(x0 + 1, y0, k) * a) * (1.0 - b) + (at(x0, y0 + 1, k) * (1.0 - a) + at(x0 + 1, y0 + 1, k) * a) * b;
                out.push(if srgb && k < 3 { cpu::srgb_encode(v) } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- mirrored instances

pub fn mirror(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = mirror_on(t, dev, level); });
}

fn mirror_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let geos = [cube(), prism(5, 0.6)];
    let spin = quat(cpu::norm([1.0, 2.0, 0.5]), 0.9);
    let spin2 = quat(cpu::norm([0.3, 1.0, 0.2]), 2.2);
    let tex = Tex::exact(128, noise(128, 128, 77), true);
    let cubetex = textured(0, Sampling::NEAREST);
    let at = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let items = [
        (at(0, [-4.5, 1.6, 0.0], spin, [1.0, 1.0, 1.0]), cubetex),
        (at(0, [-1.5, 1.6, 0.0], spin, [-1.0, 1.0, 1.0]), cubetex),
        (at(0, [1.5, 1.6, 0.0], spin, [1.0, -1.2, 0.8]), cubetex),
        (at(0, [4.5, 1.6, 0.0], spin, [-1.0, -1.0, -1.0]), cubetex),
        (at(1, [-4.5, -1.6, 0.0], spin2, [1.0, 1.0, 1.0]), Material::color(0.9, 0.5, 0.2)),
        (at(1, [-1.5, -1.6, 0.0], spin2, [-1.0, -1.0, 1.0]), Material::color(0.9, 0.5, 0.2)),
        (at(1, [1.5, -1.6, 0.0], spin2, [0.8, 1.0, -1.3]), Material::color(0.2, 0.6, 0.9)),
        (at(1, [4.5, -1.6, 0.0], spin2, [-1.0, 1.0, 1.0]), Material::color(0.2, 0.6, 0.9)),
    ];
    let cam = Camera::look_at([0.5, 1.0, 11.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut b = build(t, dev, &geos, &[tex], &items, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "mirrored")?;
    check(t, &b, &cam, &off, &img, 1, &format!("{level}: mirrored cubes and prisms (odd and even numbers of negative scales) beside unmirrored ones"));
    let lists = b.s.lists();
    let desc: Vec<(u32, bool, u32)> = lists.iter().map(|l| (l.mesh.0, l.mirrored, l.count)).collect();
    t.ok(desc == [(0, false, 1), (0, true, 3), (1, false, 2), (1, true, 2)], &format!("{level}: lists per mesh and side (mesh, mirrored, instances): {desc:?}"));
    // flipped at run time: each instance to its other side and back, against scenes built anew
    let ids = b.ids.clone();
    let mut specs: Vec<(Spec, Material)> = items.to_vec();
    for (k, id) in ids.iter().enumerate() {
        let mut sp = specs[k].0;
        sp.scale[2] = -sp.scale[2];
        specs[k].0 = sp;
        let tr = Transform { pos: sp.pos, rot: sp.rot, scale: sp.scale };
        t.res(b.s.set_transform(*id, &tr), "set_transform")?;
    }
    let flipped = shot(t, dev, &mut off, &mut b.r, &mut b.s, "flipped")?;
    let mut fresh = build(t, dev, &geos, &[Tex::exact(128, noise(128, 128, 77), true)], &specs, &cam)?;
    let want = shot(t, dev, &mut off, &mut fresh.r, &mut fresh.s, "flipped anew")?;
    let (n, m) = diff(&flipped, &want);
    t.ok(n == 0, &format!("{level}: every instance turned to its other side with set_transform: against a scene built anew {n} pixels differ (largest {m})"));
    check(t, &fresh, &cam, &off, &want, 1, &format!("{level}: the turned scene against the ray caster"));
    // from inside a mirrored cube: its faces look away
    let inside = Camera::look_at([0.0; 3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut b = build(t, dev, &geos, &[], &[(at(0, [0.0; 3], NO_TURN, [-3.0, 3.0, 3.0]), Material::color(1.0, 1.0, 1.0))], &inside)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "inside")?;
    t.ok(img.chunks_exact(4).all(|p| p[0..3] == CLEAR8), &format!("{level}: from inside a mirrored cube: back faces culled, only the clear colour"));
    // visibility of mirrored instances: their spheres by the absolute scales
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let mat = t.res(r.add_material(&Material::color(0.5, 0.7, 0.9)), "add_material")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut rng = Rng(8);
    let vcam = Camera::look_at([5.0, 3.0, 20.0], [-30.0, 10.0, -100.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.25, f64::INFINITY);
    s.set_camera(&vcam);
    let rc = ref_camera(&vcam);
    let (mut truth, mut ids) = (Vec::new(), Vec::new());
    for _ in 0..20_000 {
        let sc = rng.range(0.2, 2.5) as f32;
        let sign = |r: &mut Rng| if r.below(2) == 0 { -1.0f32 } else { 1.0 };
        let scale = [sc * sign(&mut rng), sc * rng.range(0.5, 1.0) as f32 * sign(&mut rng), sc * sign(&mut rng)];
        let mesh = rng.below(2);
        let pos = [rng.range(-200.0, 200.0), rng.range(-200.0, 200.0), rng.range(-200.0, 200.0)];
        let rot = quat(cpu::norm([rng.range(-1.0, 1.0), 1.0, rng.range(-1.0, 1.0)]), rng.range(0.0, 6.0));
        let id = t.res(s.add(render3d::Instance { mesh: MeshId(mesh), material: mat, transform: Transform { pos, rot, scale } }), "add")?;
        ids.push((id.slot, mesh));
        let info = r.mesh_info(MeshId(mesh))?;
        let (c, rad) = cpu::world_sphere(info.center, info.radius, pos, rot, scale);
        truth.push((id.slot, rc.sphere_test(c, rad, [0.0; 3])));
    }
    let seen = crate::tests::read_lists(t, dev, &mut s, &mut off, &mut r, &ids)?;
    crate::tests::judge(t, &format!("{level}: 20 000 instances, about half of them mirrored, 4 lists"), &seen, &truth);
    Some(())
}
