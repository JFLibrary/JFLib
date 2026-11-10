// srgbtest -- (G5e2) render3d encodes sRGB for 8-bit targets without sRGB (docs/RENDER3D.md "sRGB"), on both levels:
//
//   - the gallery's ten images with the shader's encoding (Rgba8Unorm) against the same images in Rgba32Float, encoded on
//     the CPU by the definition; Bgra8Unorm = Rgba8Unorm; against an sRGB target (the hardware encodes); the linear
//     ones (encode_srgb false) against the float images rounded, as before G5e2
//   - the small scene of uigputest's views (five shapes on a floor) against the ray caster in sRGB codes
//   - which formats encode

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Material, Renderer, RendererDesc, Scene};

use crate::common::*;
use crate::cpu::{self, RefScene};
use crate::tests::{gallery_image_in, on_levels, out, ref_camera, shot, sun_of, GALLERY_NAMES, T};

/// (pixels that differ, the largest difference of a channel, pixels by their largest difference: 1, 2, more)
fn cmp(a: &[u8], b: &[u8]) -> (u32, u8, [u32; 3]) {
    let (mut n, mut m, mut hist) = (0u32, 0u8, [0u32; 3]);
    if a.len() != b.len() {
        return (u32::MAX, 255, hist);
    }
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = (0..4).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
        if d > 0 {
            n += 1;
            m = m.max(d);
            hist[(d as usize - 1).min(2)] += 1;
        }
    }
    (n, m, hist)
}

/// an Rgba32Float image as 8-bit codes: encoded by the sRGB definition, or rounded linear (alpha as a unorm target
/// stores it)
fn to8(f: &[u8], srgb: bool) -> Vec<u8> {
    let mut o = Vec::with_capacity(f.len() / 4);
    for (k, c) in f.chunks_exact(4).enumerate() {
        let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64;
        o.push(if srgb && k % 4 < 3 { cpu::srgb_encode(v) } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 });
    }
    o
}

pub fn srgb(t: &mut T) {
    // which formats the shader encodes
    let enc = |f: Format| RendererDesc::new(f).encodes_srgb();
    let fine = enc(Format::Rgba8Unorm) && enc(Format::Bgra8Unorm) && !enc(Format::Rgba8Srgb) && !enc(Format::Bgra8Srgb) && !enc(Format::Rgba16Float)
        && !enc(Format::Rgba32Float) && !enc(Format::Rgb10a2Unorm) && !linear_desc(Format::Rgba8Unorm).encodes_srgb();
    t.ok(fine, "the shader encodes for Rgba8Unorm and Bgra8Unorm, not for sRGB, float or 10-bit formats, nor with encode_srgb false");
    on_levels(t, &mut |t, dev, level| {
        let _ = gallery_on(t, dev, level);
        let _ = rays_on(t, dev, level);
    });
}

fn gallery_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let enc = gallery_image_in(t, dev, RendererDesc::new(Format::Rgba8Unorm))?;
    let bgra = gallery_image_in(t, dev, RendererDesc::new(Format::Bgra8Unorm))?;
    let hw = gallery_image_in(t, dev, RendererDesc::new(Format::Rgba8Srgb))?;
    let lin = gallery_image_in(t, dev, linear_desc(Format::Rgba8Unorm))?;
    let flt = gallery_image_in(t, dev, RendererDesc::new(Format::Rgba32Float))?;
    let px = GALLERY_W * GALLERY_H;
    let (mut all_enc, mut all_hw, mut all_lin, mut sum_changed) = ([0u32; 3], [0u32; 3], [0u32; 3], 0u64);
    for (i, name) in GALLERY_NAMES.iter().chain(LATER.iter()).enumerate() {
        let want = to8(&flt[i], true);
        let (n, m, h) = cmp(&enc[i], &want);
        t.ok(m <= 1, &format!("{level}, {name}: the shader's sRGB against the float image encoded on the CPU: {n} of {px} pixels differ (by 1 / 2 / more: {} / {} / {}), largest {m}",
                              h[0], h[1], h[2]));
        let swapped: Vec<u8> = bgra[i].chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let (nb, mb, _) = cmp(&swapped, &enc[i]);
        t.ok(nb == 0, &format!("{level}, {name}: Bgra8Unorm = Rgba8Unorm: {nb} pixels differ (largest {mb})"));
        let (nh, mh, hh) = cmp(&hw[i], &enc[i]);
        t.ok(mh <= 1, &format!("{level}, {name}: an sRGB target (the hardware encodes) against the shader: {nh} pixels differ (by 1 / 2 / more: {} / {} / {}), largest {mh}",
                               hh[0], hh[1], hh[2]));
        let (nl, ml, hl) = cmp(&lin[i], &to8(&flt[i], false));
        t.ok(ml <= 1, &format!("{level}, {name}: encode_srgb false (linear) against the float image rounded: {nl} pixels differ, largest {ml}"));
        let (nc, _, _) = cmp(&enc[i], &lin[i]);
        t.ok(nc > px / 4, &format!("{level}, {name}: the encoding changes {nc} of {px} pixels against the linear image"));
        sum_changed += nc as u64;
        for k in 0..3 {
            all_enc[k] += h[k];
            all_hw[k] += hh[k];
            all_lin[k] += hl[k];
        }
    }
    out(&format!("  {level}: ten images, pixels off by 1 / 2 / more: shader against CPU {} / {} / {}, hardware against shader {} / {} / {}, linear against \
                  float {} / {} / {}; the encoding changed {sum_changed} of {} pixels", all_enc[0], all_enc[1], all_enc[2], all_hw[0], all_hw[1], all_hw[2], all_lin[0],
                 all_lin[1], all_lin[2], px as u64 * 10));
    Some(())
}

/// the small scene of uigputest's views (view.rs world(), sun()): five shapes on a floor, against the rays in sRGB
fn rays_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (240u32, 150u32);
    let floor = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let spin = quat(cpu::norm([1.0, 2.0, 0.5]), 0.9);
    let at = |mesh: u32, material: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material, pos, rot, scale };
    let geos = vec![cube(), ball(12, 18), plate()];
    let colors = [[1.0f32, 0.5, 0.25], [0.25, 1.0, 0.5], [0.5, 0.25, 1.0], [0.85, 0.85, 0.8]];
    let specs = [at(2, 3, [0.0, -1.0, 0.0], floor, [6.0, 6.0, 1.0]), at(0, 0, [-2.2, 0.0, 0.0], spin, [1.0; 3]), at(1, 1, [2.2, 0.0, 0.5], none, [1.2; 3]),
                 at(0, 2, [0.0, -0.4, -2.5], none, [0.6; 3]), at(1, 0, [0.4, 0.1, 2.2], none, [0.5, 1.1, 0.5])];
    let sun = sun_of();
    for (k, a) in [0.3f64, 2.1, 4.0].iter().enumerate() {
        let (s, c) = cpu::sincos(*a);
        let cam = Camera::look_at([7.5 * s, 4.0, 7.5 * c], [0.0; 3], [0.0, 1.0, 0.0], 0.9, w as f64 / h as f64, 0.1, f64::INFINITY);
        let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
        for g in &geos {
            t.res(r.add_mesh(&g.data()), "add_mesh")?;
        }
        for c in &colors {
            t.res(r.add_material(&Material::color(c[0], c[1], c[2])), "add_material")?;
        }
        let mut sc = t.res(Scene::new(&r), "Scene::new")?;
        sc.set_sun(&sun);
        sc.set_camera(&cam);
        let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
        rs.srgb = true;
        for spec in &specs {
            t.res(sc.add(spec.instance()), "Scene::add")?;
            rs.add(&geos[spec.mesh as usize], spec, colors[spec.material as usize]);
        }
        let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
        let img = shot(t, dev, &mut off, &mut r, &mut sc, "five shapes")?;
        let clear = [cpu::srgb_encode(CLEAR[0] as f64), cpu::srgb_encode(CLEAR[1] as f64), cpu::srgb_encode(CLEAR[2] as f64)];
        let c = cpu::compare(&rs, &ref_camera(&cam), w, h, clear, &img, None);
        t.ok(c.fine(), &format!("{level}, five shapes, view {k}: the shader's sRGB against the ray caster in sRGB codes: {}", c.text()));
        // (the clear colour is linear too: encoded by record)
        t.ok(img[..3] == clear, &format!("{level}: the clear colour encoded: {:?} for {:?}", &img[..3], CLEAR));
    }
    Some(())
}
