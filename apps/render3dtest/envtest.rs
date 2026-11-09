// envtest -- render3dtest's tests of the environment's light (G5c), on Voll and on a device limited to Basis:
// on the CPU the harmonics of the procedural sky against the reference's own projection and the irradiance they
// stand for against its integral, the prefiltered levels against a brute-force GGX convolution; on the GPU
// balls and plates in the sky against the ray caster (which reads the cube's levels like texels and samples
// them as Vulkan defines cube maps), a white furnace from six white faces, a cube of the caller's faces, the
// environment switched off again, what set_environment refuses.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, EnvironmentData, Material, Renderer, RendererDesc, Sky, Sun, Tonemap};

use crate::common::*;
use crate::cpu::{self, RefEnv};
use crate::pbr::{Lit, build_lit, compare_hdr, read_hdr};
use crate::tests::{T, on_levels, out, ref_camera, shot};

pub fn env(t: &mut T) {
    env_on_cpu(t);
    on_levels(t, &mut |t, dev, level| { let _ = env_on(t, dev, level); });
}

/// the reference's view of a renderer's environment
pub fn ref_env(r: &Renderer, k: f64) -> Option<RefEnv> {
    let e = r.environment()?;
    Some(RefEnv { k, sh: e.sh.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]), size: e.size, levels: e.levels.clone() })
}

/// the reference's own spherical harmonics of a radiance function: a 64 x 64 grid per face by solid angle, times
/// the cosine lobe's factors over pi
fn project(f: &dyn Fn([f64; 3]) -> [f64; 3]) -> [[f64; 3]; 9] {
    let g = 64usize;
    let mut sh = [[0.0; 3]; 9];
    let mut total = 0.0;
    for t in 0..g * g {
        let (u, v) = (2.0 * ((t % g) as f64 + 0.5) / g as f64 - 1.0, 2.0 * ((t / g) as f64 + 0.5) / g as f64 - 1.0);
        let r2 = 1.0 + u * u + v * v;
        total += 6.0 * 4.0 / (r2 * cpu::sqrt(r2)) / (g * g) as f64;
    }
    let scale = 4.0 * core::f64::consts::PI / total;
    for face in 0..6 {
        for t in 0..g * g {
            let (u, v) = (2.0 * ((t % g) as f64 + 0.5) / g as f64 - 1.0, 2.0 * ((t / g) as f64 + 0.5) / g as f64 - 1.0);
            let r2 = 1.0 + u * u + v * v;
            let w = 4.0 / (r2 * cpu::sqrt(r2)) / (g * g) as f64 * scale;
            let d = cpu::norm(match face {
                0 => [1.0, -v, -u],
                1 => [-1.0, -v, u],
                2 => [u, 1.0, v],
                3 => [u, -1.0, -v],
                4 => [u, -v, 1.0],
                _ => [-u, -v, -1.0],
            });
            let (x, y, z) = (d[0], d[1], d[2]);
            let b = [0.282095, 0.488603 * y, 0.488603 * z, 0.488603 * x, 1.092548 * x * y, 1.092548 * y * z, 0.315392 * (3.0 * z * z - 1.0), 1.092548 * x * z,
                     0.546274 * (x * x - y * y)];
            let c = f(d);
            for k in 0..9 {
                for q in 0..3 {
                    sh[k][q] += c[q] * b[k] * w;
                }
            }
        }
    }
    let band = [1.0, 2.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 0.25, 0.25, 0.25, 0.25, 0.25];
    for k in 0..9 {
        for q in 0..3 {
            sh[k][q] *= band[k];
        }
    }
    sh
}

/// irradiance over pi at n by a cosine-weighted sum over the sphere (Hammersley, 16384 points)
fn irradiance(f: &dyn Fn([f64; 3]) -> [f64; 3], n: [f64; 3]) -> [f64; 3] {
    let up = if n[1].abs() < 0.99 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let tx = cpu::norm(cross(up, n));
    let ty = cross(n, tx);
    let count = 1u32 << 14;
    let mut sum = [0.0; 3];
    for i in 0..count {
        let (e1, e2) = (i as f64 / count as f64, i.reverse_bits() as f64 / 4_294_967_296.0);
        let (sp, cp) = cpu::sincos(e1 * core::f64::consts::TAU);
        let (ct, st) = (cpu::sqrt(1.0 - e2), cpu::sqrt(e2));
        let d = [tx[0] * st * cp + ty[0] * st * sp + n[0] * ct, tx[1] * st * cp + ty[1] * st * sp + n[1] * ct, tx[2] * st * cp + ty[2] * st * sp + n[2] * ct];
        let c = f(d);
        for q in 0..3 {
            sum[q] += c[q];
        }
    }
    // cosine sampling: E / pi is the mean
    [sum[0] / count as f64, sum[1] / count as f64, sum[2] / count as f64]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// the GGX convolution of a radiance function with N = V = R, by importance sampling with many points
fn prefiltered(f: &dyn Fn([f64; 3]) -> [f64; 3], n: [f64; 3], rough: f64) -> [f64; 3] {
    let alpha = (rough * rough).max(0.001);
    let a2 = alpha * alpha;
    let up = if n[1].abs() < 0.99 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let tx = cpu::norm(cross(up, n));
    let ty = cross(n, tx);
    let count = 1u32 << 13;
    let (mut sum, mut w) = ([0.0; 3], 0.0);
    for i in 0..count {
        let (e1, e2) = (i as f64 / count as f64, i.reverse_bits() as f64 / 4_294_967_296.0);
        let ct = cpu::sqrt((1.0 - e2) / (1.0 + (a2 - 1.0) * e2));
        let st = cpu::sqrt(1.0 - ct * ct);
        let (sp, cp) = cpu::sincos(e1 * core::f64::consts::TAU);
        let h = [tx[0] * st * cp + ty[0] * st * sp + n[0] * ct, tx[1] * st * cp + ty[1] * st * sp + n[1] * ct, tx[2] * st * cp + ty[2] * st * sp + n[2] * ct];
        let l = [2.0 * ct * h[0] - n[0], 2.0 * ct * h[1] - n[1], 2.0 * ct * h[2] - n[2]];
        let nl = l[0] * n[0] + l[1] * n[1] + l[2] * n[2];
        if nl <= 0.0 {
            continue;
        }
        let c = f(l);
        for q in 0..3 {
            sum[q] += c[q] * nl;
        }
        w += nl;
    }
    [sum[0] / w, sum[1] / w, sum[2] / w]
}

fn env_on_cpu(t: &mut T) {
    let Some(g) = crate::tests::setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new") else { return };
    let sky = Sky::DEFAULT;
    let t0 = plat::time_us();
    if t.res(r.set_environment(&EnvironmentData::Sky(sky)), "set_environment").is_none() {
        return;
    }
    let ms = (plat::time_us() - t0) as f64 / 1000.0;
    let Some(e) = r.environment() else { return };
    let f = |d: [f64; 3]| sky.radiance(d);
    // the harmonics against the reference's own projection
    let own = project(&f);
    let mut worst = 0.0f64;
    for k in 0..9 {
        for q in 0..3 {
            worst = worst.max((own[k][q] - e.sh[k][q] as f64).abs());
        }
    }
    t.ok(worst < 2e-3, &format!("CPU: the sky's 9 harmonics against the reference's own projection (64 x 64 a face): at most {worst:.1e} apart ({ms:.0} ms for set_environment)"));
    // what 9 harmonics leave out of the irradiance
    let mut rng = Rng(5);
    let mut rel = 0.0f64;
    for _ in 0..24 {
        let n = cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)]);
        let want = irradiance(&f, n);
        let env = RefEnv { k: 1.0, sh: e.sh.map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]), size: e.size, levels: Vec::new() };
        let got = env.irradiance(n);
        for q in 0..3 {
            rel = rel.max((got[q] - want[q]).abs() / want[q].max(1e-3));
        }
    }
    t.ok(rel < 0.05, &format!("CPU: the irradiance of 9 harmonics against its integral, 24 normals: at most {:.1} % apart", rel * 100.0));
    // the prefiltered levels against a brute-force convolution (8192 points) at texel centres
    let top = e.levels.len() - 1;
    let (mut worst, mut at) = (0.0f64, 0usize);
    for k in 1..=top {
        let n = (e.size >> k).max(1) as usize;
        for probe in 0..8 {
            let face = probe % 6;
            let (i, j) = ((probe * 7 + k) % n, (probe * 3 + 1) % n);
            let d = render3d::env_dir(face, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
            let want = prefiltered(&f, d, k as f64 / top as f64);
            let got = e.levels[k][face * n * n + j * n + i];
            for q in 0..3 {
                let x = (got[q] as f64 - want[q]).abs() / want[q].max(1e-3);
                if x > worst {
                    worst = x;
                    at = k;
                }
            }
        }
    }
    t.ok(worst < 0.03, &format!("CPU: the cube's levels 1 ..= {top} (128 points a texel) against a GGX convolution of 8192 points: at most {:.2} % apart (level {at})", worst * 100.0));
    let _ = out;
}

fn env_scene(at: [f64; 3]) -> (Vec<Geo>, Vec<(Spec, Material)>) {
    let geos = vec![ball(24, 32), plate()];
    let mut items = Vec::new();
    for (row, metallic) in [(0, 0.0f32), (1, 1.0)] {
        for (col, rough) in [0.0f32, 0.3, 0.6, 1.0].iter().enumerate() {
            let c = [[0.9f32, 0.9, 0.9], [0.9, 0.5, 0.3], [0.3, 0.8, 0.4], [0.5, 0.6, 0.95]][col];
            items.push((Spec { mesh: 0, material: 0, pos: [at[0] + col as f64 * 2.5 - 3.75, at[1] + 1.3 - row as f64 * 2.6, at[2]], rot: [0.0, 0.0, 0.0, 1.0], scale: [1.1; 3] },
                        Material { metallic, roughness: *rough, ..Material::color(c[0], c[1], c[2]) }));
        }
    }
    (geos, items)
}

fn build_env(t: &mut T, dev: &Device, format: Format, data: &EnvironmentData, sun: &Sun) -> Option<(Lit, Camera)> {
    let (geos, items) = env_scene([0.0; 3]);
    let cam = Camera::look_at([0.0, 0.0, 10.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, GALLERY_W as f64 / GALLERY_H as f64, 0.1, f64::INFINITY);
    let mut l = build_lit(t, dev, format, sun, &geos, &[], &items, &cam)?;
    t.res(l.r.set_environment(data), "set_environment")?;
    l.s.set_environment(Some(1.0));
    l.rs.env = ref_env(&l.r, 1.0);
    Some((l, cam))
}

fn env_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let dark = Sun { direction: [0.3, 0.8, 0.5], color: [0.0; 3], ambient: [0.0; 3] };
    let sky = EnvironmentData::Sky(Sky::DEFAULT);
    let (mut l, cam) = build_env(t, dev, Format::Rgba8Unorm, &sky, &dark)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "sky")?;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: 8 balls in the sky alone (roughness 0 ..= 1, dielectric and metallic): {}", c.text()));
    // with the sun
    let sunny = Sun { direction: [0.3, 0.8, 0.5], color: [2.0; 3], ambient: [0.0; 3] };
    l.s.set_sun(&sunny);
    l.rs.sun_dir = cpu::norm([0.3, 0.8, 0.5]);
    l.rs.sun = [2.0; 3];
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "sky and sun")?;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: the same with the sun: {}", c.text()));
    // the environment off again: as without
    l.s.set_environment(None);
    let off_img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "off")?;
    let (geos, items) = env_scene([0.0; 3]);
    let mut plain = build_lit(t, dev, Format::Rgba8Unorm, &sunny, &geos, &[], &items, &cam)?;
    let want = shot(t, dev, &mut off, &mut plain.r, &mut plain.s, "plain")?;
    let (n, m) = diff(&off_img, &want);
    t.ok(n == 0, &format!("{level}: set_environment(None): as a scene that never had one: {n} pixels differ (largest {m})"));
    // white furnace from six white faces
    let white = vec![1.0f32; 8 * 8 * 4];
    let faces = EnvironmentData::Faces { size: 8, rgba: [&white, &white, &white, &white, &white, &white] };
    let mut hdr = t.res(Off::new(dev, w, h, Format::Rgba16Float), "HDR")?;
    let white_items: Vec<(Spec, Material)> = env_scene([0.0; 3]).1.into_iter().map(|(s, m)| (s, Material { base_color: [1.0; 4], ..m })).collect();
    let mut l = build_lit(t, dev, Format::Rgba16Float, &dark, &env_scene([0.0; 3]).0, &[], &white_items, &cam)?;
    t.res(l.r.set_environment(&faces), "set_environment")?;
    l.s.set_environment(Some(1.0));
    l.s.set_tonemap(Tonemap::None);
    l.rs.env = ref_env(&l.r, 1.0);
    l.rs.tonemap = false;
    let id = t.res(hdr.submit(&mut l.r, &mut l.s), "furnace")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let himg = t.res(read_hdr(&hdr), "read")?;
    let c = compare_hdr(&l.rs, &ref_camera(&cam), w, h, &himg, 2e-3, 3e-3);
    t.ok(c.bad == 0 && c.max <= 1.002, &format!("{level}: white furnace from six white faces (white dielectric = 1, white metal = A + B): {}", c.text()));
    // the caller's faces: a smooth cube
    let n = 16u32;
    let mut data: Vec<Vec<f32>> = Vec::new();
    for f in 0..6 {
        let mut px = Vec::with_capacity((n * n * 4) as usize);
        for j in 0..n {
            for i in 0..n {
                let d = render3d::env_dir(f, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                px.extend_from_slice(&[(0.6 + 0.4 * d[0]) as f32, (0.6 + 0.4 * d[1]) as f32, (0.6 + 0.4 * d[2]) as f32, 1.0]);
            }
        }
        data.push(px);
    }
    let faces = EnvironmentData::Faces { size: n, rgba: [&data[0], &data[1], &data[2], &data[3], &data[4], &data[5]] };
    let (mut l, cam) = build_env(t, dev, Format::Rgba8Unorm, &faces, &dark)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "faces")?;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: a cube of the caller's faces (16 x 16, colour by direction): {}", c.text()));
    // what set_environment refuses
    let inv = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    let six = vec![1.0f32; 6 * 6 * 4];
    let mut nan = white.clone();
    nan[5] = f32::NAN;
    let refused = [inv(l.r.set_environment(&EnvironmentData::Faces { size: 6, rgba: [&six, &six, &six, &six, &six, &six] })),
                   inv(l.r.set_environment(&EnvironmentData::Faces { size: 8, rgba: [&white, &white, &nan, &white, &white, &white] })),
                   inv(l.r.set_environment(&EnvironmentData::Faces { size: 16, rgba: [&white, &white, &white, &white, &white, &white] }))];
    t.ok(refused.iter().all(|v| *v), &format!("{level}: set_environment refuses faces of 6 texels, a NaN, too few values: {refused:?}"));
    Some(())
}
