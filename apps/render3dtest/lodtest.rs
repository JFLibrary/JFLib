// lodtest -- (G5d) render3dtest --test lods: detail levels (Renderer::set_lods, docs/RENDER3D.md "Detailstufen").  Balls
// of 16 x 24 segments take balls of 8 x 12 and 4 x 6 below a projected size and are left out below a third
// (contribution culling); a third of them mirrored, cubes without levels among them.  The pick of every instance
// against a reference on the CPU (sizes near a threshold are not placed); the image with the occlusion test against
// the ray caster drawing each instance with the mesh the CPU picked; without levels the image differs.  The shadow
// cascades take the camera's pick.  A level beyond its mesh grows the culling sphere.  What set_lods refuses.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Lod, Material, MeshId, Renderer, RendererDesc, Scene};

use crate::common::*;
use crate::cpu::{self, RefScene};
use crate::tests::{T, on_levels, ref_camera, shot, sun_of};

pub fn lods(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = lods_on(t, dev, level); });
}

/// the meshes: ball 16 x 24 (with levels), ball 8 x 12, ball 4 x 6, cube
fn geos() -> Vec<Geo> {
    vec![ball(16, 24), ball(8, 12), ball(4, 6), cube()]
}

const LEVELS: [Lod; 3] = [Lod { mesh: Some(MeshId(1)), below: 0.08 }, Lod { mesh: Some(MeshId(2)), below: 0.03 }, Lod { mesh: None, below: 0.008 }];

/// the CPU's pick: Some(mesh) or None (left out), from the size r fy / d of the culling sphere
fn pick(mesh: u32, size: f64) -> Option<u32> {
    if mesh != 0 {
        return Some(mesh);
    }
    let mut m = Some(0);
    for l in LEVELS {
        if size < l.below as f64 {
            m = l.mesh.map(|x| x.0);
        }
    }
    m
}

/// the size of an instance as the GPU sees it (the culling sphere), for the camera
fn size_of(r: &Renderer, sp: &Spec, cam: &Camera) -> (f64, [f64; 3], f64) {
    let (c, rad) = r.cull_sphere(MeshId(sp.mesh)).unwrap_or(([0.0; 3], 1.0));
    let (wc, wr) = cpu::world_sphere(c, rad, sp.pos, sp.rot, sp.scale);
    let d = cpu::sqrt(cpu::dot([wc[0] - cam.eye[0], wc[1] - cam.eye[1], wc[2] - cam.eye[2]], [wc[0] - cam.eye[0], wc[1] - cam.eye[1], wc[2] - cam.eye[2]]));
    let fy = 1.0 / (cpu::sincos(cam.fov_y * 0.5).0 / cpu::sincos(cam.fov_y * 0.5).1);
    (wr * fy / d.max(1e-30), wc, wr)
}

/// n instances in front of the camera, none within 2 % of a threshold
fn specs(r: &Renderer, cam: &Camera, n: usize) -> Vec<Spec> {
    let mut rng = Rng(808);
    let mut v = Vec::new();
    while v.len() < n {
        let d = rng.range(3.0, 160.0);
        let (x, y) = (rng.range(-0.6, 0.6) * d, rng.range(-0.35, 0.35) * d);
        let s = rng.range(0.3, 1.4) as f32;
        let mut scale = [s, s, s];
        if rng.below(3) == 0 {
            scale[rng.below(3) as usize] *= -1.0;
        }
        let mesh = if rng.below(8) == 0 { 3 } else { 0 };
        let axis = cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0) + 0.01]);
        let q = |v: f64| (v * 1024.0) as i64 as f64 / 1024.0;
        let sp = Spec { mesh, material: rng.below(12), pos: [q(cam.eye[0] + x), q(cam.eye[1] + y), q(cam.eye[2] - d)], rot: quat(axis, rng.range(0.0, 6.0)), scale };
        let (size, _, _) = size_of(r, &sp, cam);
        // (larger ones would hide the rest)
        if size > 0.16 || LEVELS.iter().any(|l| (size / l.below as f64 - 1.0).abs() < 0.02) {
            continue;
        }
        v.push(sp);
    }
    v
}

/// list index -> (mesh, mirrored), and the slots per list of a set of draw records and visible list
fn read(t: &mut T, dev: &Device, s: &Scene, draws: &Buffer, visible: &Buffer) -> Option<Vec<(u32, u32)>> {
    let lists = s.lists();
    let mut d = vec![0u32; draws.size() as usize / 4];
    let mut v = vec![0u32; visible.size() as usize / 4];
    t.res(dev.queue().read_buffer(draws, 0, &mut d), "read draws")?;
    t.res(dev.queue().read_buffer(visible, 0, &mut v), "read visible")?;
    let mut out = Vec::new();
    for (i, l) in lists.iter().enumerate() {
        let c = d[i * 5 + 1].min(l.capacity);
        for k in 0..c {
            out.push((v[(l.first + k) as usize], i as u32));
        }
    }
    Some(out)
}

fn lods_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let geos = geos();
    let colors = gallery_colors(12);
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    for c in &colors {
        t.res(r.add_material(&Material::color(c[0], c[1], c[2])), "add_material")?;
    }
    // what set_lods refuses
    let inv = |x: gpu::Result<()>| matches!(x, Err(GpuError::Invalid(_)));
    let rising = [Lod { mesh: Some(MeshId(1)), below: 0.03 }, Lod { mesh: Some(MeshId(2)), below: 0.08 }];
    let refused = [inv(r.set_lods(MeshId(0), &rising)), inv(r.set_lods(MeshId(9), &LEVELS)), inv(r.set_lods(MeshId(0), &[Lod { mesh: Some(MeshId(9)), below: 0.1 }])),
                   inv(r.set_lods(MeshId(0), &[Lod { mesh: None, below: f32::NAN }])), inv(r.set_lods(MeshId(0), &[Lod { mesh: None, below: 0.0 }])),
                   inv(r.set_lods(MeshId(0), &[Lod { mesh: None, below: 0.1 }; render3d::LODS_MAX + 1]))];
    t.ok(refused.iter().all(|&x| x) && r.lods(MeshId(0)).is_empty(), &format!("{level}: set_lods refuses {} of 6 (rising, unknown meshes, NaN, 0, too many)", refused.iter().filter(|&&x| x).count()));
    // a level that reaches beyond its mesh grows the culling sphere (a ball 2 to the side as the cube's level), and
    // shrinks it back without
    let mut aside = ball(4, 6);
    for p in aside.pos.iter_mut() {
        p[0] += 2.0;
    }
    let far = t.res(r.add_mesh(&aside.data()), "add_mesh")?;
    t.res(r.set_lods(MeshId(3), &[Lod { mesh: Some(far), below: 0.5 }]), "set_lods")?;
    let wide = r.cull_sphere(MeshId(3)).map_or(0.0, |x| x.1);
    let centre = r.mesh_info(far).map_or(0.0, |m| m.center[0]);
    t.res(r.set_lods(MeshId(3), &[]), "set_lods")?;
    let back = r.cull_sphere(MeshId(3)).map_or(0.0, |x| x.1);
    let want = (centre + r.mesh_info(far).map_or(0.0, |m| m.radius)).max(cpu::sqrt(3.0));
    t.ok((wide - want).abs() < 1e-12 && (back - cpu::sqrt(3.0)).abs() < 1e-6,
         &format!("{level}: a level 2 to the side grows the cube's culling sphere to {wide:.4} (its own {back:.4})"));
    t.res(r.set_lods(MeshId(0), &LEVELS), "set_lods")?;
    let grown = r.cull_sphere(MeshId(0)).map_or(0.0, |x| x.1);
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = Camera::look_at([0.0, 1.0, 8.0], [0.0, 0.5, -100.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let list = specs(&r, &cam, 1500);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_sun(&sun_of());
    s.set_camera(&cam);
    for sp in &list {
        t.res(s.add(sp.instance()), "add")?;
    }
    // the pick against the CPU (without the occlusion test: everything in the view is listed)
    s.set_occlusion(false);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    shot(t, dev, &mut off, &mut r, &mut s, "lods")?;
    let infos = s.lists();
    let got = read(t, dev, &s, s.draws_buffer(), s.visible_buffer())?;
    let rc = ref_camera(&cam);
    let mut listed = vec![None; list.len()];
    for &(slot, l) in &got {
        if let Some(x) = listed.get_mut(slot as usize) {
            *x = Some(l);
        }
    }
    let (mut right, mut wrong, mut per) = (0, 0, [0u32; 5]);
    for (i, sp) in list.iter().enumerate() {
        let (size, wc, wr) = size_of(&r, sp, &cam);
        let seen = rc.sphere_test(wc, wr, [0.0; 3]);
        if seen == cpu::Seen::Doubt {
            continue;
        }
        let want = if seen == cpu::Seen::Yes { pick(sp.mesh, size) } else { None };
        let mirrored = (sp.scale[0] as f64) * (sp.scale[1] as f64) * (sp.scale[2] as f64) < 0.0;
        let have = listed[i].map(|l| (infos[l as usize].mesh.0, infos[l as usize].mirrored));
        let ok = match (want, have) {
            (None, None) => true,
            (Some(m), Some((hm, hmir))) => m == hm && hmir == mirrored,
            _ => false,
        };
        if ok { right += 1 } else { wrong += 1 }
        if seen == cpu::Seen::Yes {
            per[want.map_or(4, |m| m as usize)] += 1;
        }
    }
    t.ok(wrong == 0, &format!("{level}: {} instances, the pick against the CPU: {right} right, {wrong} wrong (in the view: ball 16 x 24 {}, 8 x 12 {}, 4 x 6 {}, cube {}, left out {}); culling sphere {grown:.4}",
                              list.len(), per[0], per[1], per[2], per[3], per[4]));
    // the image with the occlusion test against the ray caster with the CPU's pick
    s.set_occlusion(true);
    for k in 0..2 {
        let img = shot(t, dev, &mut off, &mut r, &mut s, "lods")?;
        if k == 1 {
            let sun = sun_of();
            let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
            for sp in &list {
                let (size, _, _) = size_of(&r, sp, &cam);
                if let Some(m) = pick(sp.mesh, size) {
                    rs.add(&geos[m as usize], &Spec { mesh: m, ..*sp }, colors[sp.material as usize]);
                }
            }
            let c = cpu::compare(&rs, &rc, w, h, CLEAR8, &img, None);
            t.ok(c.fine(), &format!("{level}: with detail levels and the occlusion test, second frame, against the ray caster with the same pick: {}", c.text()));
            // without levels the image is another (the test can tell)
            t.res(r.set_lods(MeshId(0), &[]), "set_lods")?;
            let plain = shot(t, dev, &mut off, &mut r, &mut s, "no lods")?;
            let (n, m) = diff(&img, &plain);
            t.ok(n > 0, &format!("{level}: without detail levels {n} pixels differ (largest {m})"));
            t.res(r.set_lods(MeshId(0), &LEVELS), "set_lods")?;
            let again = shot(t, dev, &mut off, &mut r, &mut s, "lods again")?;
            let (n, m) = diff(&img, &again);
            t.ok(n == 0, &format!("{level}: the levels set again: {n} pixels differ (largest {m})"));
        }
    }
    // the shadow cascades take the camera's pick
    let sh = render3d::Shadows { cascades: 4, size: 1024, distance: 120.0, ..render3d::Shadows::DEFAULT };
    t.res(s.set_shadows(Some(&sh)), "set_shadows")?;
    s.set_occlusion(false);
    shot(t, dev, &mut off, &mut r, &mut s, "shadows")?;
    let infos = s.lists();
    let (mut bad, mut total) = (0, 0);
    for k in 0..4 {
        let Some((d, v)) = s.cascade_buffers(k) else { continue };
        let set = read(t, dev, &s, d, v)?;
        for &(slot, l) in &set {
            let sp = &list[slot as usize];
            let (size, _, _) = size_of(&r, sp, &cam);
            total += 1;
            if pick(sp.mesh, size) != Some(infos[l as usize].mesh.0) {
                bad += 1;
            }
        }
    }
    t.ok(bad == 0 && total > 0, &format!("{level}: the cascades' lists ({total} entries) take the camera's pick: {bad} not"));
    Some(())
}
