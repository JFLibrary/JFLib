// pbr -- render3dtest's tests of the shading (G5c), on Voll and on a device limited to Basis, against the ray
// caster (cpu.rs: glTF 2.0 Appendix B in f64): materials from dull to metallic, the metallic-roughness texture,
// normal maps (turned, mirrored instances, mirrored uv, on a ball; and which way they lean), emission,
// occlusion (on the ambient light only), exposure and both tone mappers; in an Rgba16Float target without tone
// mapping the light itself, and a white furnace (energy conservation).  On the CPU alone: the BRDF's albedo
// by importance sampling, and how far Karis' fit of the split-sum's second sum lies from the integral.

use alloc::format;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Material, Renderer, RendererDesc, Sampling, Scene, Sun, TextureData, TextureId, TextureRef, Tonemap};

use crate::common::*;
use crate::cpu::{self, RefScene, RefTexture};
use crate::tests::{T, on_levels, out, ref_camera, shot, sun_of};
use crate::tex::{Tex, look_of, noise, plate_uv};

const NO_TURN: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

pub fn pbr(t: &mut T) {
    brdf_on_cpu(t);
    on_levels(t, &mut |t, dev, level| { let _ = pbr_on(t, dev, level); });
}

/// a scene of the tests: meshes, textures, one material per item, with its reference
pub struct Lit {
    pub r: Renderer,
    pub s: Scene,
    pub rs: RefScene,
}

/// meshes, textures (as the reference holds them), a material per item, a sun, the target's format
#[allow(clippy::too_many_arguments)]
pub fn build_lit(t: &mut T, dev: &Device, format: Format, sun: &Sun, geos: &[Geo], texs: &[Tex], items: &[(Spec, Material)], cam: &Camera) -> Option<Lit> {
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(format)), "Renderer::new")?;
    for g in geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for x in texs {
        t.res(r.add_texture(&TextureData { width: x.w, height: x.h, rgba: &x.rgba, srgb: x.srgb, nearest: x.nearest }), "add_texture")?;
        let size = render3d::CLASS_MIN << render3d::class_of(x.w, x.h);
        rs.textures.push(RefTexture::new(size, &x.reference, x.srgb));
    }
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_sun(sun);
    s.set_camera(cam);
    for (spec, m) in items {
        let id = t.res(r.add_material(m), "add_material")?;
        let mut sp = *spec;
        sp.material = id.0;
        t.res(s.add(sp.instance()), "Scene::add")?;
        rs.add_look(&geos[spec.mesh as usize], &sp, look_of(m));
    }
    Some(Lit { r, s, rs })
}

fn tex(id: u32, sampling: Sampling) -> Option<TextureRef> {
    Some(TextureRef { texture: TextureId(id), sampling })
}

fn check(t: &mut T, l: &Lit, cam: &Camera, off: &Off, img: &[u8], tol: u8, what: &str) -> cpu::Compared {
    let c = cpu::compare_tol(&l.rs, &ref_camera(cam), off.w, off.h, CLEAR8, img, None, tol);
    t.ok(c.fine(), &format!("{what}: {}", c.text()));
    if let Some((x, y, _, _)) = c.first {
        let rc = ref_camera(cam);
        if let Some(hit) = l.rs.trace(rc.eye, rc.ray(x as f64 + 0.5, y as f64 + 0.5, off.w, off.h), rc.near, rc.far) {
            let uv = l.rs.uv_of(&hit);
            out(&format!("    first wrong pixel: uv x 128 = {:.6}, {:.6}; linear {:?} rgb {:?}", uv[0] * 128.0, uv[1] * 128.0, hit.linear, hit.rgb));
        }
    }
    c
}

// ---------------------------------------------------------------- HDR

/// an IEEE half as f32
pub fn half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 31) as u32, (h & 1023) as u32);
    let bits = if e == 0 {
        if m == 0 {
            s << 31
        } else {
            // subnormal: m 2^-24
            return if s == 1 { -(m as f32) * (1.0 / 16_777_216.0) } else { m as f32 * (1.0 / 16_777_216.0) };
        }
    } else if e == 31 {
        s << 31 | 0x7F80_0000 | m << 13
    } else {
        s << 31 | (e + 112) << 23 | m << 13
    };
    f32::from_bits(bits)
}

/// the pixels of an Rgba16Float target as f32
pub fn read_hdr(off: &Off) -> gpu::Result<Vec<[f32; 4]>> {
    let raw = off.read()?;
    Ok(raw.chunks_exact(8).map(|p| {
        let h = |k: usize| half(u16::from_le_bytes([p[k * 2], p[k * 2 + 1]]));
        [h(0), h(1), h(2), h(3)]
    }).collect())
}

/// what comparing light with the rays found: solid pixels (no edge), of them wrong, the largest relative error
#[derive(Default)]
pub struct HdrCompared {
    pub solid: u32,
    pub on_objects: u32,
    pub bad: u32,
    pub largest: f64,
    pub first: Option<(u32, u32, [f32; 3], [f64; 3])>,
    /// over the object pixels: the least and the largest channel
    pub min: f64,
    pub max: f64,
}

/// light at solid pixels against the rays: |got - want| <= abs + rel * want (half floats hold 11 bits)
pub fn compare_hdr(sc: &RefScene, cam: &cpu::RefCamera, w: u32, h: u32, img: &[[f32; 4]], abs: f64, rel: f64) -> HdrCompared {
    let mut out = HdrCompared { min: f64::MAX, max: f64::MIN, ..HdrCompared::default() };
    let shoot = |x: f64, y: f64| sc.trace(cam.eye, cam.ray(x, y, w, h), cam.near, cam.far);
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let c = shoot(cx, cy);
            let e = 1.0 / 64.0;
            let around = [shoot(cx - e, cy), shoot(cx + e, cy), shoot(cx, cy - e), shoot(cx, cy + e)];
            if !around.iter().all(|a| a.map(|v| v.inst) == c.map(|v| v.inst)) {
                continue;
            }
            out.solid += 1;
            let want = c.map_or([CLEAR[0] as f64, CLEAR[1] as f64, CLEAR[2] as f64], |v| v.linear);
            let got = img[(y * w + x) as usize];
            if c.is_some() {
                out.on_objects += 1;
                for k in 0..3 {
                    out.min = out.min.min(got[k] as f64);
                    out.max = out.max.max(got[k] as f64);
                }
            }
            // (the GPU snaps vertices to 1/256 of a pixel: where the light changes fast across the pixel, near a
            // ball's silhouette under a highlight, a quarter of what 1/64 of a pixel changes is allowed too)
            let steep = |k: usize| around.iter().flatten().map(|a| (a.linear[k] - want[k]).abs()).fold(0.0, f64::max) * 0.25;
            let mut wrong = false;
            for k in 0..3 {
                let d = (got[k] as f64 - want[k]).abs();
                out.largest = out.largest.max(d / want[k].abs().max(1e-3));
                if d > abs + rel * want[k].abs() + steep(k) {
                    wrong = true;
                }
            }
            if wrong {
                out.bad += 1;
                out.first.get_or_insert((x, y, [got[0], got[1], got[2]], want));
            }
        }
    }
    out
}

impl HdrCompared {
    pub fn text(&self) -> alloc::string::String {
        format!("{} solid ({} on objects, channels {:.4} ..= {:.4}), largest relative difference {:.2e}, {} wrong{}", self.solid, self.on_objects, self.min, self.max, self.largest, self.bad,
                self.first.map_or(alloc::string::String::new(), |f| format!(" (first at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3)))
    }
}

// ---------------------------------------------------------------- on the CPU

/// the i-th of n points of the Hammersley set
fn hammersley(i: u32, n: u32) -> (f64, f64) {
    (i as f64 / n as f64, i.reverse_bits() as f64 / 4_294_967_296.0)
}

/// the BRDF's albedo for white (metallic: F = 1, all specular; else a white dielectric) and the split-sum's
/// second sum (A, B) at roughness r and n.v = nv, by GGX importance sampling of the half vector
fn integrate(r: f64, nv: f64, metallic: bool) -> (f64, f64, f64) {
    let n = [0.0, 0.0, 1.0];
    let v = [cpu::sqrt(1.0 - nv * nv), 0.0, nv];
    let alpha = (r * r).max(0.001);
    let a2 = alpha * alpha;
    let count = 1 << 16;
    let (mut a, mut b) = (0.0, 0.0);
    for i in 0..count {
        let (e1, e2) = hammersley(i, count);
        // the half vector by D(h) (n.h) as its density
        let cos_t = cpu::sqrt((1.0 - e2) / (1.0 + (a2 - 1.0) * e2));
        let sin_t = cpu::sqrt(1.0 - cos_t * cos_t);
        let (sp, cp) = cpu::sincos(e1 * core::f64::consts::TAU);
        let hv = [sin_t * cp, sin_t * sp, cos_t];
        let vh = v[0] * hv[0] + v[1] * hv[1] + v[2] * hv[2];
        let l = [2.0 * vh * hv[0] - v[0], 2.0 * vh * hv[1] - v[1], 2.0 * vh * hv[2] - v[2]];
        let nl = l[2];
        if nl <= 0.0 || vh <= 0.0 {
            continue;
        }
        // f n.l / pdf with pdf(l) = D n.h / (4 v.h): the specular part without F is 4 V n.l v.h / n.h
        let vis = 1.0 / ((nl + cpu::sqrt(a2 + (1.0 - a2) * nl * nl)) * (nv + cpu::sqrt(a2 + (1.0 - a2) * nv * nv)));
        let w = 4.0 * vis * nl * vh / cos_t;
        let fc = { let x = 1.0 - vh; x * x * x * x * x };
        a += (1.0 - fc) * w;
        b += fc * w;
        let _ = n;
    }
    a /= count as f64;
    b /= count as f64;
    let albedo = if metallic {
        a + b
    } else {
        // a white dielectric: its specular lobe (f0 0.04) plus the diffuse part, (1 - F) / pi by cosine
        // sampling (the reference's brdf, without the specular term)
        let spec = 0.04 * a + b;
        let mut diff = 0.0;
        let m = 256;
        for i in 0..m * m {
            let (e1, e2) = hammersley(i, m * m);
            let (sp, cp) = cpu::sincos(e1 * core::f64::consts::TAU);
            let ct = cpu::sqrt(1.0 - e2);
            let st = cpu::sqrt(e2);
            let l = [st * cp, st * sp, ct];
            let full = cpu::brdf(n, v, l, [1.0; 3], [0.04; 3], a2)[0];
            let hv = cpu::norm([l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
            let vh = (v[0] * hv[0] + v[1] * hv[1] + v[2] * hv[2]).clamp(0.0, 1.0);
            let f = 0.04 + 0.96 * { let x = 1.0 - vh; x * x * x * x * x };
            let nh = hv[2].clamp(0.0, 1.0);
            let t = nh * nh * (a2 - 1.0) + 1.0;
            let spec_only = f * (a2 / (core::f64::consts::PI * t * t)) * (1.0 / ((ct + cpu::sqrt(a2 + (1.0 - a2) * ct * ct)) * (nv + cpu::sqrt(a2 + (1.0 - a2) * nv * nv)))) * ct;
            // cosine sampling: pdf = n.l / pi
            diff += (full - spec_only) * core::f64::consts::PI / ct;
        }
        spec + diff / (m * m) as f64
    };
    (albedo, a, b)
}

fn brdf_on_cpu(t: &mut T) {
    let (mut worst_metal, mut table, mut karis) = (0.0f64, (0.0f64, 0.0, 0.0), (0.0f64, 0.0, 0.0));
    let mut loss = (1.0f64, 0.0);
    // on the table's points and between them (roughness 0.05 and 0.37 lie between)
    for r in [0.0, 0.05, 0.25, 0.37, 0.5, 0.75, 1.0] {
        for nv in [1.0, 0.7, 0.45, 0.3, 0.05] {
            let (albedo, a, b) = integrate(r, nv, true);
            worst_metal = worst_metal.max(albedo);
            if albedo < loss.0 {
                loss = (albedo, r);
            }
            let (fa, fb) = cpu::env_ab(r, nv);
            let e = (fa - a).abs().max((fb - b).abs());
            if e > table.0 {
                table = (e, r, nv);
            }
            let (ka, kb) = cpu::karis_ab(r, nv);
            let e = (ka - a).abs().max((kb - b).abs());
            if e > karis.0 {
                karis = (e, r, nv);
            }
        }
    }
    t.ok(worst_metal <= 1.0 + 1e-3, &format!("CPU: the BRDF's albedo, white metal (F = 1), roughness 0 ..= 1, n.v 0.05 ..= 1: at most {worst_metal:.4} (least {:.3} at roughness {}: single scattering)", loss.0, loss.1));
    // glTF's sample BRDF weighs the diffuse part by 1 - F(v.h), which is near 1 for most directions while
    // the specular lobe reflects more and more towards grazing angles: it is not energy conserving there
    let mut line = alloc::string::String::new();
    let mut fine = true;
    for nv in [1.0, 0.7, 0.3, 0.05] {
        let worst = [0.0, 0.25, 0.5, 1.0].iter().map(|&r| integrate(r, nv, false).0).fold(0.0, f64::max);
        line.push_str(&format!(" n.v {nv}: {worst:.3};"));
        fine &= nv < 0.5 || worst <= 1.0 + 3e-3;
    }
    t.ok(fine, &format!("CPU: the BRDF's albedo, white dielectric (glTF Appendix B), the most over roughness 0 ..= 1 by n.v:{line} (at most 1 from n.v 0.5 on, within the integration's noise of about 1e-3)"));
    t.ok(table.0 < 0.01, &format!("CPU: the engine's table of the split-sum's second sum against the reference's own integral: largest difference {:.4} at roughness {}, n.v {}", table.0, table.1, table.2));
    out(&format!("  note Karis' analytic fit against the same integral: largest difference {:.3} at roughness {}, n.v {}", karis.0, karis.1, karis.2));
}

// ---------------------------------------------------------------- on the GPU


fn pbr_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let sun = Sun { direction: [0.5, 0.7, 0.6], ..sun_of() };
    let ball = || ball(24, 32);

    // materials: dielectric and metallic, rough to smooth
    let geos = [ball()];
    let cam = Camera::look_at([0.0, 0.0, 10.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let colors = [[0.9f32, 0.3, 0.2], [0.2, 0.7, 0.3], [0.3, 0.4, 0.9], [0.95, 0.95, 0.95]];
    let mut items = Vec::new();
    for (row, metallic) in [(0, 0.0f32), (1, 1.0)] {
        for (col, rough) in [0.25f32, 0.5, 0.75, 1.0].iter().enumerate() {
            let c = colors[col];
            let m = Material { metallic, roughness: *rough, ..Material::color(c[0], c[1], c[2]) };
            items.push((Spec { mesh: 0, material: 0, pos: [col as f64 * 2.5 - 3.75, 1.3 - row as f64 * 2.6, 0.0], rot: NO_TURN, scale: [1.1; 3] }, m));
        }
    }
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &sun, &geos, &[], &items, &cam)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "materials")?;
    check(t, &l, &cam, &off, &img, 1, &format!("{level}: 8 balls, dielectric and metallic, roughness 0.25 ..= 1 (PBR Neutral)"));
    // exposure, no tone mapping (the 8-bit target clamps)
    l.s.set_exposure(0.6);
    l.s.set_tonemap(Tonemap::None);
    l.rs.exposure = 0.6;
    l.rs.tonemap = false;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "exposure")?;
    check(t, &l, &cam, &off, &img, 1, &format!("{level}: the same, exposure 0.6, no tone mapping"));

    // the light itself: Rgba16Float, no tone mapping
    let mut hdr = t.res(Off::new(dev, w, h, Format::Rgba16Float), "HDR target")?;
    let mut l = build_lit(t, dev, Format::Rgba16Float, &sun, &geos, &[], &items, &cam)?;
    l.s.set_tonemap(Tonemap::None);
    l.rs.tonemap = false;
    let id = t.res(hdr.submit(&mut l.r, &mut l.s), "HDR")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let img = t.res(read_hdr(&hdr), "read HDR")?;
    let c = compare_hdr(&l.rs, &ref_camera(&cam), w, h, &img, 2e-3, 2e-3);
    t.ok(c.bad == 0 && c.on_objects > 1000, &format!("{level}: the 8 balls' light in Rgba16Float, within 2e-3 + 2e-3 x: {}", c.text()));

    // white furnace: a uniform white environment, no sun; dielectric white = 1, metal white = A + B <= 1
    let furnace = Sun { direction: [0.0, 1.0, 0.0], color: [0.0; 3], ambient: [1.0; 3] };
    let mut fitems = Vec::new();
    for (row, metallic) in [(0, 0.0f32), (1, 1.0)] {
        for (col, rough) in [0.0f32, 0.25, 0.5, 0.75, 1.0].iter().enumerate() {
            let m = Material { metallic, roughness: *rough, ..Material::color(1.0, 1.0, 1.0) };
            fitems.push((Spec { mesh: 0, material: 0, pos: [col as f64 * 2.4 - 4.8, 1.3 - row as f64 * 2.6, 0.0], rot: NO_TURN, scale: [1.1; 3] }, m));
        }
    }
    for (name, which) in [("dielectric", 0..5), ("metallic", 5..10)] {
        let mut l = build_lit(t, dev, Format::Rgba16Float, &furnace, &geos, &[], &fitems[which], &cam)?;
        l.s.set_tonemap(Tonemap::None);
        l.rs.tonemap = false;
        let id = t.res(hdr.submit(&mut l.r, &mut l.s), "furnace")?;
        t.res(dev.wait(id, 10_000_000_000), "wait")?;
        let img = t.res(read_hdr(&hdr), "read HDR")?;
        let c = compare_hdr(&l.rs, &ref_camera(&cam), w, h, &img, 2e-3, 2e-3);
        let fine = if name == "dielectric" { c.min >= 0.998 && c.max <= 1.002 } else { c.max <= 1.002 };
        t.ok(c.bad == 0 && fine, &format!("{level}: white furnace, white {name} balls, roughness 0 ..= 1: {}", c.text()));
    }

    // the metallic-roughness texture (g roughness from 0.25, b metallic), nearest
    let mut mr = noise(128, 128, 61);
    for p in mr.chunks_exact_mut(4) {
        p[1] = 64 + (p[1] as u32 * 191 / 255) as u8;
    }
    let base = crate::gltfgen::picture(128, 128, 62, &|_, _| 255);
    let plate = Spec { mesh: 0, material: 0, pos: [0.0; 3], rot: NO_TURN, scale: [4.5, 2.5, 1.0] };
    let pgeo = [plate_uv([0.0, 1.0], [1.0, 0.0])];
    let near = Camera::look_at([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let m = Material { metallic: 1.0, roughness: 1.0, base_color_texture: tex(0, Sampling::NEAREST), metallic_roughness_texture: tex(1, Sampling::NEAREST), ..Material::color(1.0, 1.0, 1.0) };
    let sun_low = Sun { direction: [0.2, 0.3, 1.0], ..sun_of() };
    let texs = [Tex::exact(128, base.clone(), true), Tex::exact(128, mr, false)];
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &sun_low, &pgeo, &texs, &[(plate, m)], &near)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "mr")?;
    check(t, &l, &near, &off, &img, 1, &format!("{level}: a metallic-roughness texture, nearest"));

    // normal maps: a plate as it is, turned by 90 degrees, mirrored, with mirrored u; a ball
    let bumps = bump_map(128, 71);
    let ngeos = [plate_uv([0.0, 1.0], [1.0, 0.0]), plate_uv([1.0, 0.0], [1.0, 0.0]), ball()];
    let turn = quat([0.0, 0.0, 1.0], core::f64::consts::FRAC_PI_2);
    let nm = |scale: f32| Material { normal_texture: tex(0, Sampling::NEAREST), normal_scale: scale, roughness: 0.5, ..Material::color(0.8, 0.7, 0.6) };
    // (on the balls bilinear on level 0: nearest texels or levels would make every texel's and level's edge a
    // place where the GPU's f32 and the reference's f64 may pick different ones)
    let bilinear = Sampling { mag_nearest: false, min_nearest: false, mip: render3d::Mip::None, ..Sampling::LINEAR };
    let nb = |scale: f32| Material { normal_texture: tex(0, bilinear), ..nm(scale) };
    let at = |mesh: u32, x: f64, y: f64, rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos: [x, y, 0.0], rot, scale };
    let nitems = [
        (at(0, -4.4, 1.2, NO_TURN, [1.1; 3]), nm(1.0)),
        (at(0, -1.5, 1.2, turn, [1.1; 3]), nm(0.8)),
        (at(0, 1.5, 1.2, NO_TURN, [-1.1, 1.1, 1.1]), nm(1.0)),
        (at(1, 4.4, 1.2, NO_TURN, [1.1; 3]), nm(1.0)),
        (at(2, -2.0, -1.4, quat(cpu::norm([1.0, 1.0, 0.0]), 0.7), [1.2; 3]), nb(1.0)),
        (at(2, 2.0, -1.4, NO_TURN, [-1.2, 1.2, 1.2]), nb(1.5)),
    ];
    let ncam = Camera::look_at([0.0, 0.0, 9.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let sun_side = Sun { direction: [-0.6, 0.5, 0.6], ..sun_of() };
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &sun_side, &ngeos, &[Tex::exact(128, bumps.clone(), false)], &nitems[..4], &ncam)?;
    let plates = shot(t, dev, &mut off, &mut l.r, &mut l.s, "normal maps")?;
    check(t, &l, &ncam, &off, &plates, 1, &format!("{level}: normal maps, nearest: plates as they are, turned by 90 degrees (scale 0.8), mirrored, with mirrored u"));
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &sun_side, &ngeos, &[Tex::exact(128, bumps.clone(), false)], &nitems[4..], &ncam)?;
    let balls = shot(t, dev, &mut off, &mut l.r, &mut l.s, "normal maps")?;
    check(t, &l, &ncam, &off, &balls, 2, &format!("{level}: normal maps, bilinear (within 2: the hardware's weights have 8 bits): balls, turned, mirrored with scale 1.5"));
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &sun_side, &ngeos, &[Tex::exact(128, bumps.clone(), false)], &nitems, &ncam)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "normal maps")?;
    let flat_items: Vec<(Spec, Material)> = nitems.iter().map(|(s, m)| (*s, Material { normal_texture: None, ..*m })).collect();
    let mut f = build_lit(t, dev, Format::Rgba8Unorm, &sun_side, &ngeos, &[], &flat_items, &ncam)?;
    let flat = shot(t, dev, &mut off, &mut f.r, &mut f.s, "no normal maps")?;
    let (n, _) = diff(&img, &flat);
    t.ok(n > 5000, &format!("{level}: the normal maps change {n} pixels"));

    // which way a normal map leans: +y in the image (v decreasing) is up on a plate whose image stands
    // upright; under a sun from above it is brighter than a flat one, leaning down darker
    let lean = |g: u8| -> Vec<u8> { (0..128 * 128).flat_map(|_| [128u8, g, 238, 255]).collect() };
    let upright = [plate_uv([0.0, 1.0], [1.0, 0.0])];
    let above = Sun { direction: [0.0, 0.8, 0.6], ..sun_of() };
    let mut means = Vec::new();
    for g in [191u8, 128, 64] {
        let mut l = build_lit(t, dev, Format::Rgba8Unorm, &above, &upright, &[Tex::exact(128, lean(g), false)], &[(plate, nm(1.0))], &near)?;
        let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "lean")?;
        let px = &img[((h / 2 * w + w / 2) * 4) as usize..][..3];
        means.push(px[0] as u32 + px[1] as u32 + px[2] as u32);
    }
    t.ok(means[0] > means[1] && means[1] > means[2], &format!("{level}: a normal map leaning to +y (up in the image) is lit from above: brightness {} > flat {} > leaning down {}", means[0], means[1], means[2]));

    // emission (sRGB texture times the factor) in the dark; occlusion on the ambient light only
    let dark = Sun { direction: [0.0, 1.0, 0.0], color: [0.0; 3], ambient: [0.0; 3] };
    let m = Material { emissive: [1.5, 0.8, 0.4], emissive_texture: tex(0, Sampling::NEAREST), ..Material::color(0.5, 0.5, 0.5) };
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &dark, &pgeo, &[Tex::exact(128, base.clone(), true)], &[(plate, m)], &near)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "emission")?;
    check(t, &l, &near, &off, &img, 1, &format!("{level}: emission: factor times an sRGB texture, no light"));
    let occ = noise(128, 128, 81);
    let m = Material { occlusion_texture: tex(1, Sampling::NEAREST), occlusion_strength: 0.7, base_color_texture: tex(0, Sampling::NEAREST), roughness: 0.6, ..Material::color(1.0, 1.0, 1.0) };
    let ambient = Sun { direction: [0.3, 0.4, 1.0], color: [0.4 * core::f32::consts::PI; 3], ambient: [0.6; 3] };
    let texs = [Tex::exact(128, base.clone(), true), Tex::exact(128, occ.clone(), false)];
    let mut l = build_lit(t, dev, Format::Rgba8Unorm, &ambient, &pgeo, &texs, &[(plate, m)], &near)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "occlusion")?;
    check(t, &l, &near, &off, &img, 1, &format!("{level}: occlusion, strength 0.7, sun and ambient light"));
    let sun_only = Sun { ambient: [0.0; 3], ..ambient };
    let mut a = build_lit(t, dev, Format::Rgba8Unorm, &sun_only, &pgeo, &texs, &[(plate, m)], &near)?;
    let with = shot(t, dev, &mut off, &mut a.r, &mut a.s, "occlusion, sun only")?;
    let mut b = build_lit(t, dev, Format::Rgba8Unorm, &sun_only, &pgeo, &texs, &[(plate, Material { occlusion_texture: None, ..m })], &near)?;
    let without = shot(t, dev, &mut off, &mut b.r, &mut b.s, "no occlusion, sun only")?;
    let (n, mx) = diff(&with, &without);
    t.ok(n == 0, &format!("{level}: occlusion leaves the sun's light alone: with and without the texture {n} pixels differ (largest {mx})"));
    Some(())
}
