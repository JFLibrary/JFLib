// g7f2test -- render3dtest --test ortho, toon, outline, fxaa, viewmodel, particles (G7f part 2; docs/RENDER3D.md
// "Orthografische Kamera", "Cartoon-Material", "Umrisse", "Kantenglättung", "Waffe vor der Kamera", "Partikel"), on Voll
// and on a device limited to Basis.
//
//   ortho      an orthographic camera: the arena from the front, the top, the side and above against the ray caster with
//              parallel rays (and the depth buffer, linear in z); random scenes' lists against the exact test of the box;
//              spheres touching each of its six faces; the occlusion test = none over a flight, and it hides; detail levels
//              by size alone; the sun's shadows against exact shadow rays; blended plates sorted by depth
//   toon       the cartoon look against the ray caster: bands, softness, rim on flat faces and balls, a point light, the
//              environment's mean, MASK, double-sided and blended in the look, with the sun's shadows against exact rays;
//              what Toon refuses, set_material keeps the look
//   outline    the outline pass against the same formula on the CPU over the GPU's own depth and image (widths, colours,
//              an orthographic camera, with a viewmodel); against the ray caster's exact depth: lines where the geometry
//              has edges; refused without a sampled depth buffer
//   fxaa       FXAA against FXAA 3.11 written again on the CPU over the GPU's own image (linear, sRGB target, the
//              shader's sRGB); a plain image stays as it is; only edges change
//   viewmodel  a weapon in its own pass close to a wall: never in the wall, against two ray casters (the world's and the
//              weapon's lens); in the sun and in a roof's shadow (the world's shadow map at its place); turning the camera
//              under a sun overhead leaves it as it is; its lists apart from the world's and the cascades'; the world's depth
//              halved; refusals
//   particles  the records the same on every run and device; the list (alive, sorted) against the CPU's motion; the image
//              against the particles laid on the CPU over the GPU's image and depth without them (additive, alpha, streaks,
//              the viewmodel's); the ring's end, a lost frame, a new origin, refusals

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Antialias, Camera, Instance, Material, MaterialDesc, MaterialId, MeshId, Outline, Renderer, Scene, Sun, Toon, Transform};

use crate::common::*;
use crate::cpu::{self, RefCamera, RefScene};
use crate::tests::{T, on_levels, read_depth, ref_camera, shot};

/// a described scene (common.rs) in a fresh renderer of this format and its reference (the world's items)
struct Arena {
    r: Renderer,
    s: Scene,
    rs: RefScene,
}

fn reference(d: &Described, sun: &Sun) -> RefScene {
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for (spec, m) in &d.2 {
        rs.add_look(&d.0[spec.mesh as usize], spec, crate::tex::look_of_desc(m));
    }
    rs
}

fn arena(t: &mut T, dev: &Device, format: Format, d: &Described, cam: &Camera, sun: &Sun, vm: bool) -> Option<Arena> {
    let mut r = t.res(Renderer::new(dev, &linear_desc(format)), "Renderer::new")?;
    let (s, _) = t.res(arena_in(&mut r, d, cam, sun, vm), "the arena")?;
    Some(Arena { r, s, rs: reference(d, sun) })
}

fn read_words(t: &mut T, dev: &Device, b: &Buffer) -> Option<Vec<u32>> {
    let mut w = vec![0u32; b.size() as usize / 4];
    t.res(dev.queue().read_buffer(b, 0, &mut w), "read a buffer")?;
    Some(w)
}

// ---------------------------------------------------------------- orthographic camera

pub fn ortho(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = ortho_images_on(t, dev, level);
        let _ = ortho_visibility_on(t, dev, level);
        let _ = ortho_occlusion_on(t, dev, level);
        let _ = ortho_lods_on(t, dev, level);
        let _ = ortho_shadows_on(t, dev, level);
        let _ = ortho_blend_on(t, dev, level);
    });
}

fn ortho_images_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let a = w as f64 / h as f64;
    let d = arena_scene(None);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let views = [("from the front", Camera::ortho_at([0.0, 1.0, 20.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 5.0, a, 0.0, 40.0)),
                 ("from the top", Camera::ortho_at([0.0, 20.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, -1.0], 19.0, a, 0.5, 30.0)),
                 ("from the side", Camera::ortho_at([20.0, 1.5, -1.0], [0.0, 1.5, -1.0], [0.0, 1.0, 0.0], 7.0, a, 1.0, 40.0)),
                 ("from above, near plane behind the eye", Camera::ortho_at([3.1, 4.0, 5.2], [0.0, 0.5, -1.0], [0.0, 1.0, 0.0], 11.0, a, -15.0, 30.0))];
    for (name, cam) in views {
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, name)?;
        let depth = read_depth(t, dev, &off)?;
        let c = cpu::compare(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, Some(&depth));
        t.ok(c.fine() && c.depth_bad == 0 && c.objects_seen >= 6,
             &format!("{level}: the arena orthographic {name} against parallel rays: {}; depth (linear) largest relative error {:.1e}", c.text(), c.depth_err));
    }
    // the same view at 1e7 with the origin there: the same image
    let cam = arena_ortho(a);
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
    let near = shot(t, dev, &mut off, &mut b.r, &mut b.s, "at 0")?;
    let at = [1e7, 0.0, -1e7];
    let mut far = d.clone();
    for (sp, _) in &mut far.2 {
        sp.pos = [sp.pos[0] + at[0], sp.pos[1] + at[1], sp.pos[2] + at[2]];
    }
    let cam_far = Camera { eye: [cam.eye[0] + at[0], cam.eye[1] + at[1], cam.eye[2] + at[2]], ..cam };
    let mut bf = arena(t, dev, Format::Rgba8Unorm, &far, &cam_far, &Sun::DEFAULT, false)?;
    bf.s.set_origin(at);
    let away = shot(t, dev, &mut off, &mut bf.r, &mut bf.s, "at 1e7")?;
    let (n, m) = diff(&near, &away);
    t.ok(n <= w * h / 200 && m <= 2, &format!("{level}: orthographic at 0 and at 1e7 (origin there): {n} pixels differ (largest {m})"));
    // refused: no height, no far plane
    let mut bad = cam;
    bad.far = f64::INFINITY;
    b.s.set_camera(&bad);
    t.ok(matches!(b.s.prepare(&mut b.r), Err(GpuError::Invalid(_))), &format!("{level}: an orthographic camera without a far plane: Invalid"));
    bad = cam;
    bad.ortho = Some(0.0);
    b.s.set_camera(&bad);
    t.ok(matches!(b.s.prepare(&mut b.r), Err(GpuError::Invalid(_))), &format!("{level}: an orthographic camera 0 high: Invalid"));
    Some(())
}

fn ortho_visibility_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (128u32, 72u32);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let geos = [cube(), prism(7, 0.6), ball(6, 8)];
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let m = t.res(r.add_material(&Material::color(0.5, 0.75, 1.0)), "add_material")?;
    for (seed, n, spread) in [(21u32, 20_000u32, 200.0f64), (22, 5000, 40.0)] {
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        let mut rng = Rng(seed);
        let cam = Camera::ortho_at([rng.range(-20.0, 20.0), rng.range(-20.0, 20.0), rng.range(-20.0, 20.0)], [rng.range(-200.0, 200.0), rng.range(-200.0, 200.0), rng.range(-200.0, 200.0)],
                                   [0.0, 1.0, 0.0], rng.range(30.0, 120.0), aspect, rng.range(-50.0, 5.0), rng.range(80.0, 300.0));
        s.set_camera(&cam);
        let rc = ref_camera(&cam);
        let (mut truth, mut ids) = (Vec::new(), Vec::new());
        for _ in 0..n {
            let scale = rng.range(0.1, 3.0) as f32;
            let (mesh, pos) = (rng.below(3), [rng.range(-spread, spread), rng.range(-spread, spread), rng.range(-spread, spread)]);
            let rot = quat(cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), 1.0]), rng.range(0.0, 6.0));
            let sc = [scale, scale * rng.range(0.5, 1.0) as f32, scale];
            let id = t.res(s.add(Instance { mesh: MeshId(mesh), material: m, transform: Transform { pos, rot, scale: sc } }), "add")?;
            ids.push((id.slot, mesh));
            let info = r.mesh_info(MeshId(mesh))?;
            let (c, rad) = cpu::world_sphere(info.center, info.radius, pos, rot, sc);
            truth.push((id.slot, rc.sphere_test(c, rad, [0.0; 3])));
        }
        let seen = crate::tests::read_lists(t, dev, &mut s, &mut off, &mut r, &ids)?;
        crate::tests::judge(t, &format!("{level}: orthographic, seed {seed}: {n} instances"), &seen, &truth);
    }
    // balls touching each face of the box stay, 0.01 beyond go
    let cam = Camera::ortho_at([3.0, -2.0, 11.0], [-4.0, 5.0, -9.0], [0.0, 1.0, 0.0], 30.0, aspect, -5.0, 60.0);
    let rc = ref_camera(&cam);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    let info = r.mesh_info(MeshId(2))?;
    let rad = 1.5f64;
    let k = rad / info.radius;
    let faces: [(&str, [f64; 3], [f64; 3]); 6] = [("left", [1.0, 0.0, 0.0], [-rc.tx, 0.0, 20.0]), ("right", [-1.0, 0.0, 0.0], [rc.tx, 0.0, 20.0]),
                                                  ("bottom", [0.0, 1.0, 0.0], [0.0, -rc.ty, 20.0]), ("top", [0.0, -1.0, 0.0], [0.0, rc.ty, 20.0]),
                                                  ("near", [0.0, 0.0, 1.0], [0.0, 0.0, -5.0]), ("far", [0.0, 0.0, -1.0], [0.0, 0.0, 60.0])];
    let (mut ids, mut truth) = (Vec::new(), Vec::new());
    for (name, n, p) in faces {
        for (gap, want) in [(0.0, true), (0.01, false)] {
            let d = -(rad + gap);
            let c = [p[0] + n[0] * d, p[1] + n[1] * d, p[2] + n[2] * d];
            let world = [rc.eye[0] + rc.r[0] * c[0] + rc.u[0] * c[1] + rc.f[0] * c[2], rc.eye[1] + rc.r[1] * c[0] + rc.u[1] * c[1] + rc.f[1] * c[2],
                         rc.eye[2] + rc.r[2] * c[0] + rc.u[2] * c[1] + rc.f[2] * c[2]];
            let id = t.res(s.add(Instance { mesh: MeshId(2), material: m, transform: Transform { pos: world, rot: [0.0, 0.0, 0.0, 1.0], scale: [k as f32; 3] } }), "add")?;
            ids.push((id.slot, 2));
            truth.push((id.slot, name, want));
        }
    }
    let seen = crate::tests::read_lists(t, dev, &mut s, &mut off, &mut r, &ids)?;
    let wrong: Vec<String> = truth.iter().filter(|(slot, _, want)| seen.has(*slot) != *want).map(|(_, n, w)| format!("{n} {}", if *w { "touching" } else { "beyond" })).collect();
    t.ok(wrong.is_empty(), &format!("{level}: orthographic: balls touching each of the six faces in the list, 0.01 beyond culled; wrong: {wrong:?}"));
    Some(())
}

fn ortho_occlusion_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let cube_m = t.res(r.add_mesh(&cube().data()), "add_mesh")?;
    let mats: Vec<MaterialId> = (0..8).map(|k| r.add_material(&Material::color(0.3 + 0.08 * k as f32, 0.6, 0.9 - 0.1 * k as f32))).collect::<gpu::Result<_>>().ok()?;
    let (mut a, mut b) = (t.res(Scene::new(&r), "Scene::new")?, t.res(Scene::new(&r), "Scene::new")?);
    b.set_occlusion(false);
    a.set_stats(true);
    b.set_stats(true);
    // a wall, 3000 small cubes behind and before it
    let wall = Instance { mesh: cube_m, material: mats[0], transform: Transform { pos: [0.0, 0.0, 0.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [24.0, 14.0, 0.3] } };
    t.res(a.add(wall), "add")?;
    t.res(b.add(wall), "add")?;
    let mut rng = Rng(5);
    for _ in 0..3000 {
        let pos = [rng.range(-20.0, 20.0), rng.range(-12.0, 12.0), rng.range(-30.0, 6.0)];
        let i = Instance { mesh: cube_m, material: mats[rng.below(8) as usize], transform: Transform { pos, rot: quat([0.0, 1.0, 0.0], rng.range(0.0, 3.0)), scale: [0.2; 3] } };
        t.res(a.add(i), "add")?;
        t.res(b.add(i), "add")?;
    }
    let mut same = 0;
    let frames = 6;
    for f in 0..frames {
        let x = -3.0 + f as f64 * 1.2;
        let cam = Camera::ortho_at([x, 1.0, 20.0], [x * 0.5, 0.0, 0.0], [0.0, 1.0, 0.0], 14.0, aspect, 0.0, 60.0);
        a.set_camera(&cam);
        b.set_camera(&cam);
        let ia = shot(t, dev, &mut off, &mut r, &mut a, "with occlusion")?;
        let ib = shot(t, dev, &mut off, &mut r, &mut b, "without")?;
        same += (diff(&ia, &ib).0 == 0) as u32;
    }
    t.res(dev.wait_idle(), "wait")?;
    a.collect_stats();
    b.collect_stats();
    let (sa, sb) = (a.take_stats(), b.take_stats());
    let (va, vb) = (sa.last().map_or(0, |x| x.visible), sb.last().map_or(0, |x| x.visible));
    t.ok(same == frames && va * 2 < vb, &format!("{level}: orthographic: the occlusion test = none in {same} of {frames} frames; drawn {va} of {vb} in the view"));
    Some(())
}

fn ortho_lods_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let fine = t.res(r.add_mesh(&ball(16, 24).data()), "add_mesh")?;
    let mid = t.res(r.add_mesh(&ball(8, 12).data()), "add_mesh")?;
    let low = t.res(r.add_mesh(&ball(4, 6).data()), "add_mesh")?;
    t.res(r.set_lods(fine, &[render3d::Lod { mesh: Some(mid), below: 0.1 }, render3d::Lod { mesh: Some(low), below: 0.04 }]), "set_lods")?;
    let m = t.res(r.add_material(&Material::color(0.8, 0.7, 0.4)), "add_material")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    // (all of them drawn: the counts of the camera's lists are the picks)
    s.set_occlusion(false);
    // the view 20 high: a ball of radius k has the size k sy = k / 10 at any distance
    let cam = Camera::ortho_at([0.0, 0.0, 50.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 20.0, aspect, 0.0, 200.0);
    s.set_camera(&cam);
    let mut want = [0u32; 3];
    let mut rng = Rng(8);
    for k in 0..90 {
        let (scale, level_of) = match k % 3 {
            0 => (1.5, 0usize),
            1 => (0.7, 1),
            _ => (0.25, 2),
        };
        want[level_of] += 1;
        let pos = [rng.range(-12.0, 12.0), rng.range(-7.0, 7.0), rng.range(-120.0, 40.0)];
        t.res(s.add(Instance { mesh: fine, material: m, transform: Transform { pos, rot: [0.0, 0.0, 0.0, 1.0], scale: [scale as f32; 3] } }), "add")?;
    }
    let id = t.res(off.submit(&mut r, &mut s), "frame")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let draws = read_words(t, dev, s.draws_buffer())?;
    let mut got = [0u32; 3];
    for l in s.lists() {
        let k = if l.mesh == fine { 0 } else if l.mesh == mid { 1 } else { 2 };
        got[k] += draws[l.record as usize * 5 + 1];
    }
    t.ok(got == want, &format!("{level}: orthographic detail levels by size alone (at depths from -120 to 40): fine, middle, coarse {got:?}, want {want:?}"));
    Some(())
}

fn ortho_shadows_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let aspect = w as f64 / h as f64;
    let d = arena_scene(None);
    let cam = Camera::ortho_at([6.0, 7.0, 10.0], [0.0, 0.6, -1.0], [0.0, 1.0, 0.0], 9.0, aspect, 0.0, 40.0);
    let sun = shadow_sun();
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &sun, false)?;
    t.res(b.s.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 2048, distance: 30.0, ..render3d::Shadows::DEFAULT })), "set_shadows")?;
    b.rs.shadows = true;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "orthographic shadows")?;
    let c = cpu::compare_shadows(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, 1);
    let cs = b.s.cascades();
    t.ok(c.bad == 0 && c.edges_bad == 0 && c.inside * 1000 <= c.on_objects * 3 && c.shadowed > 1000 && cs.len() == 2,
         &format!("{level}: the sun's shadows under an orthographic camera (2 cascades, {:.2} .. {:.2} .. {:.2}) against exact shadow rays: {}", cs.first()?.near,
                  cs.first()?.far, cs.last()?.far, c.text()));
    Some(())
}

fn ortho_blend_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    // plates whose order by depth differs from their order by the distance to the eye (far to the side, near the eye's plane)
    let mut d: Described = (vec![plate()], Vec::new(), Vec::new());
    let s = |pos: [f64; 3]| Spec { mesh: 0, material: 0, pos, rot: [0.0, 0.0, 0.0, 1.0], scale: [2.0, 1.5, 1.0] };
    let m = |r: f32, g: f32, b: f32, a: f32| {
        let mut x = MaterialDesc::new(Material { base_color: [r, g, b, a], roughness: 0.6, ..Material::color(r, g, b) });
        x.blend = true;
        x
    };
    let mut opaque = MaterialDesc::new(Material::color(0.6, 0.6, 0.6));
    opaque.double_sided = false;
    d.2.push((Spec { mesh: 0, material: 0, pos: [0.0, 0.0, -20.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [20.0, 12.0, 1.0] }, opaque));
    d.2.push((s([-7.0, 0.5, -2.0]), m(0.9, 0.2, 0.1, 0.5)));
    d.2.push((s([-5.5, -0.5, -1.0]), m(0.2, 0.8, 0.2, 0.5)));
    d.2.push((s([0.0, 0.0, -6.0]), m(0.2, 0.3, 0.9, 0.5)));
    d.2.push((s([-6.2, 0.0, -3.0]), m(0.9, 0.9, 0.2, 0.5)));
    let cam = Camera::ortho_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 12.0, aspect, 0.0, 50.0);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (mut sc, ids) = t.res(described_in(&mut r, &d, &cam, &Sun::DEFAULT), "scene")?;
    sc.set_occlusion(false);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let img = shot(t, dev, &mut off, &mut r, &mut sc, "orthographic blend")?;
    let words = read_words(t, dev, sc.blend_buffer())?;
    let n = words[1] as usize;
    let order: Vec<u32> = (0..n).map(|k| words[5 + k * 4]).collect();
    // by depth along forward (-z), the farthest first
    let want = vec![ids[3].slot, ids[4].slot, ids[1].slot, ids[2].slot];
    let mut rs = reference(&d, &Sun::DEFAULT);
    rs.blend_axis = Some([0.0, 0.0, -1.0]);
    let c = cpu::compare_tol(&rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 2);
    t.ok(order == want && c.fine(), &format!("{level}: orthographic: blended plates sorted by depth {order:?} (want {want:?}); against the ray caster laying them so: {}", c.text()));
    Some(())
}

// ---------------------------------------------------------------- the cartoon look

pub fn toon(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = toon_on(t, dev, level);
        let _ = toon_kinds_on(t, dev, level);
    });
}

fn look(bands: u32, softness: f32, rim: f32, rim_width: f32) -> Toon {
    let mut k = Toon::new(bands);
    k.softness = softness;
    k.rim = rim;
    k.rim_width = rim_width;
    k
}

/// the arena with a look per item: hard steps only on flat faces (a ball's n.l passes every threshold, where f32 and f64
/// would part); soft ones, rims and many bands on the balls too
fn toon_arena() -> Described {
    let mut d = arena_scene(None);
    let flat = [look(2, 0.0, 0.0, 0.0), look(3, 0.3, 0.4, 0.3), look(4, 0.6, 0.2, 0.5), look(1, 0.0, 0.0, 0.0), look(8, 1.0, 0.3, 0.25), look(3, 0.0, 0.0, 0.0)];
    let smooth = [look(3, 0.3, 0.4, 0.3), look(4, 0.6, 0.2, 0.5), look(8, 1.0, 0.3, 0.25)];
    for (k, (sp, m)) in d.2.iter_mut().enumerate() {
        m.toon = Some(if sp.mesh == 2 { smooth[k % smooth.len()] } else { flat[k % flat.len()] });
    }
    d
}

fn toon_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let cam = arena_camera(w as f64 / h as f64);
    let d = toon_arena();
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "toon")?;
    let c = cpu::compare(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None);
    t.ok(c.fine() && c.objects_seen >= 19, &format!("{level}: the arena in six looks (1, 2, 3, 4, 8 bands, hard and soft, rims) against the ray caster: {}", c.text()));
    // the test can tell: the same image against PBR
    let pbr = reference(&arena_scene(None), &Sun::DEFAULT);
    let cp = cpu::compare(&pbr, &ref_camera(&cam), w, h, CLEAR8, &img, None);
    t.ok(cp.bad > 5000, &format!("{level}: the test can tell: against the arena in PBR {} pixels wrong", cp.bad));
    // the sun's shadows: in shadow as if turned away
    t.res(b.s.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 2048, distance: 30.0, ..render3d::Shadows::DEFAULT })), "set_shadows")?;
    b.rs.shadows = true;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "toon with shadows")?;
    let c = cpu::compare_shadows(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, 1);
    t.ok(c.bad == 0 && c.edges_bad == 0 && c.inside * 1000 <= c.on_objects * 3 && c.shadowed > 1000 && c.slivers * 1000 <= c.on_objects,
         &format!("{level}: the looks with the sun's shadows against exact shadow rays: {}", c.text()));
    t.res(b.s.set_shadows(None), "set_shadows")?;
    b.rs.shadows = false;
    // a point light and a spot light, the environment's mean as the ambient light
    let lights = [render3d::Light::point([1.5, 2.0, 2.5], [1.0, 0.8, 0.6], 6.0, Some(8.0)),
                  render3d::Light { kind: render3d::LightKind::Spot { inner: 0.2, outer: 0.5 }, pos: [-3.0, 3.5, 2.0], direction: [0.4, -1.0, -0.3], color: [0.5, 0.7, 1.0],
                                    intensity: 10.0, range: None }];
    t.res(b.s.set_lights(&lights), "set_lights")?;
    t.res(b.r.set_environment(&render3d::EnvironmentData::Sky(render3d::Sky::DEFAULT)), "set_environment")?;
    b.s.set_environment(Some(0.8));
    for l in &lights {
        let spot = match l.kind {
            render3d::LightKind::Spot { inner, outer } => Some((cpu::norm(l.direction), inner as f64, outer as f64)),
            _ => None,
        };
        b.rs.lights.push(cpu::RefLight { pos: l.pos, color: l.color.map(|c| c as f64 * l.intensity as f64), range: l.range.map(|r| r as f64), spot });
    }
    let env = b.r.environment()?;
    b.rs.env = Some(cpu::RefEnv { k: 0.8, sh: env.sh.map(|c| c.map(|v| v as f64)), size: 0, levels: Vec::new() });
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "toon with lights")?;
    let c = cpu::compare(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None);
    t.ok(c.fine(), &format!("{level}: the looks under a point and a spot light, the environment's mean as the ambient light: {}", c.text()));
    // set_material keeps the look; refusals
    let before = img;
    let m = d.2[1].1.material;
    t.res(b.r.set_material(MaterialId(1), &m), "set_material")?;
    let after = shot(t, dev, &mut off, &mut b.r, &mut b.s, "set_material")?;
    let (n, _) = diff(&before, &after);
    let mut bad = Vec::new();
    for k in [look(0, 0.0, 0.0, 0.0), look(17, 0.0, 0.0, 0.0), look(2, 1.5, 0.0, 0.0), look(2, 0.0, f32::NAN, 0.0), look(2, 0.0, 0.0, -0.1)] {
        let mut x = MaterialDesc::new(Material::color(1.0, 1.0, 1.0));
        x.toon = Some(k);
        if !matches!(b.r.add_material_desc(&x), Err(GpuError::Invalid(_))) {
            bad.push(k);
        }
    }
    t.ok(n == 0 && bad.is_empty(), &format!("{level}: set_material keeps the look ({n} pixels differ); out of range not refused: {bad:?}"));
    Some(())
}

/// the look on MASK (holes), double-sided (seen from behind) and blended plates, before a wall in the look
fn toon_kinds_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let k = look(3, 0.3, 0.3, 0.3);
    let tex = Some(render3d::TextureRef { texture: render3d::TextureId(0), sampling: NEAREST_ONLY });
    let desc = |m: Material, blend: bool, double: bool| {
        let mut x = MaterialDesc::new(m);
        x.toon = Some(k);
        x.blend = blend;
        x.double_sided = double;
        x
    };
    let s = |pos: [f64; 3], rot: [f32; 4]| Spec { mesh: 0, material: 0, pos, rot, scale: [1.6, 1.2, 1.0] };
    let turned = [0.0f32, 1.0, 0.0, 0.0];
    let d: Described = (vec![plate()], vec![(128, holes_rgba(), true)], vec![
        (Spec { mesh: 0, material: 0, pos: [0.0, 0.0, -4.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [9.0, 5.0, 1.0] }, desc(Material::color(0.6, 0.6, 0.7), false, false)),
        (s([-3.6, 0.0, 0.0], quat([0.0, 1.0, 0.0], 0.3)), desc(Material { base_color_texture: tex, alpha_cutoff: Some(0.5), ..Material::color(0.9, 0.6, 0.2) }, false, false)),
        (s([0.0, 0.0, 0.0], turned), desc(Material::color(0.3, 0.8, 0.4), false, true)),
        (s([3.6, 0.0, 0.5], quat([0.0, 1.0, 0.0], -0.3)), desc(Material { base_color: [0.2, 0.4, 0.95, 0.5], ..Material::color(0.2, 0.4, 0.95) }, true, false)),
    ]);
    let cam = Camera::look_at([0.5, 1.0, 9.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (mut sc, _) = t.res(described_in(&mut r, &d, &cam, &Sun::DEFAULT), "scene")?;
    let mut rs = reference(&d, &Sun::DEFAULT);
    rs.textures.push(cpu::RefTexture::new(128, &holes_rgba(), true));
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let img = shot(t, dev, &mut off, &mut r, &mut sc, "toon kinds")?;
    let c = cpu::compare_tol(&rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 2);
    t.ok(c.fine() && c.objects_seen == 3 && sc.blend_drawn(), &format!("{level}: the look on MASK, double-sided (from behind) and blended plates against the ray caster: {}", c.text()));
    Some(())
}

// ---------------------------------------------------------------- outlines

pub fn outline(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = outline_on(t, dev, level);
    });
}

/// a lens of the outline's uniform block as post.wgsl reads it: zz, zw (depth = zz + zw / z, orthographic zz + zw z), fy (sy),
/// orthographic; depth: the mapping a depth + b of the view (a viewmodel's world (0.5, 0), the viewmodel (0.5, 0.5))
pub fn lens_of(c: &Camera, depth: (f64, f64)) -> [f32; 4] {
    let (n, fa) = (c.near, c.far);
    match c.ortho {
        Some(h) => [(fa / (fa - n) * depth.0 + depth.1) as f32, (-1.0 / (fa - n) * depth.0) as f32, (2.0 / h) as f32, 1.0],
        None => {
            let fy = 1.0 / render3d::math::tan(c.fov_y * 0.5);
            let (zz, zw) = if fa.is_finite() { (-n / (fa - n), n * fa / (fa - n)) } else { (0.0, n) };
            [(zz * depth.0 + depth.1) as f32, (zw * depth.0) as f32, fy as f32, 0.0]
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// post.wgsl's fs_outline on the CPU in f32: the coverage of every pixel (times the colour's alpha)
pub fn outline_cover(depth: &[f32], w: u32, h: u32, o: &Outline, world: [f32; 4], vm: Option<[f32; 4]>) -> Vec<f32> {
    let at = |x: i32, y: i32| depth[(y.clamp(0, h as i32 - 1) as u32 * w + x.clamp(0, w as i32 - 1) as u32) as usize];
    let mut out = vec![0.0f32; (w * h) as usize];
    let wd = o.width as i32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let dc = at(x, y);
            if !(dc > 0.0) {
                continue;
            }
            let (l, r, u, d) = (at(x - wd, y), at(x + wd, y), at(x, y - wd), at(x, y + wd));
            let k = match vm {
                Some(v) if dc > 0.5 => v,
                _ => world,
            };
            let px = 2.0 / (k[2] * h as f32);
            let mut s = (dc - k[0]) * px;
            if k[3] != 0.0 {
                s = k[1].abs() * px;
            }
            s *= o.width as f32;
            let lx = (l + r - 2.0 * dc) / s;
            let ly = (u + d - 2.0 * dc) / s;
            let e = lx.abs().max(ly.abs());
            let tt = o.crease;
            let mut cover = smoothstep(tt, tt * 1.5 + 1e-3, e);
            const JUMP: f32 = 16.0;
            if (lx.abs() > JUMP && lx > 0.0 && ly.abs() <= JUMP) || (ly.abs() > JUMP && ly > 0.0 && lx.abs() <= JUMP) || (lx > JUMP && ly > JUMP) {
                cover = 0.0;
            }
            out[(y as u32 * w + x as u32) as usize] = cover * o.color[3].clamp(0.0, 1.0);
        }
    }
    out
}

/// the image with lines of the colour laid over by coverage (premultiplied, the target's 8 bits; linear target)
fn with_lines(base: &[u8], cover: &[f32], color: [f32; 4]) -> Vec<u8> {
    let mut out = base.to_vec();
    for (k, a) in cover.iter().enumerate() {
        for c in 0..3 {
            let v = color[c].max(0.0) * a + base[k * 4 + c] as f32 / 255.0 * (1.0 - a);
            out[k * 4 + c] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    out
}

/// the depth buffer as exact rays see it (reverse Z of the nearest opaque hit, 0 where none), through `map` (a view's
/// mapping a depth + b)
fn exact_depth(rs: &RefScene, cam: &RefCamera, w: u32, h: u32, map: (f64, f64)) -> Vec<f32> {
    let mut d = vec![0.0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            if let Some(hit) = rs.trace(cam.origin(cx, cy, w, h), cam.ray(cx, cy, w, h), cam.span().0, cam.span().1) {
                d[(y * w + x) as usize] = (cam.depth(hit.z) * map.0 + map.1) as f32;
            }
        }
    }
    d
}

/// lines (coverage above a half) of one mask with no line of the other within a pixel: (of a, of b), and a's lines
fn lines_apart(a: &[f32], b: &[f32], w: u32, h: u32) -> (u32, u32, u32) {
    let near = |m: &[f32], x: u32, y: u32| {
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                if xx >= 0 && yy >= 0 && (xx as u32) < w && (yy as u32) < h && m[(yy as u32 * w + xx as u32) as usize] > 0.5 {
                    return true;
                }
            }
        }
        false
    };
    let (mut na, mut nb, mut lines) = (0, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let k = (y * w + x) as usize;
            if a[k] > 0.5 {
                lines += 1;
                if !near(b, x, y) {
                    na += 1;
                }
            }
            if b[k] > 0.5 && !near(a, x, y) {
                nb += 1;
            }
        }
    }
    (na, nb, lines)
}

fn outline_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let toon = toon_variants()[1].1;
    let line = |w: u32, c: [f32; 4], crease: f32| {
        let mut o = Outline::new(w, c);
        o.crease = crease;
        o
    };
    let cases = [("1 px black, perspective", arena_camera(aspect), line(1, [0.0, 0.0, 0.0, 1.0], 0.5), false),
                 ("2 px dark red half covering", arena_camera(aspect), line(2, [0.5, 0.05, 0.05, 0.5], 0.5), false),
                 ("3 px, creases from 1.5", arena_camera(aspect), line(3, [0.1, 0.1, 0.3, 1.0], 1.5), false),
                 ("2 px, orthographic", arena_ortho(aspect), line(2, [0.0, 0.0, 0.0, 1.0], 0.5), false),
                 ("1 px, with the viewmodel", wall_camera(aspect), line(1, [0.0, 0.0, 0.0, 1.0], 0.5), true)];
    for (name, cam, o, vm) in cases {
        let d = arena_scene(Some(toon));
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, vm)?;
        let base = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without lines")?;
        let depth = read_depth(t, dev, &off)?;
        t.res(b.s.set_outline(Some(&o)), "set_outline")?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, name)?;
        let map = if vm { (0.5, 0.0) } else { (1.0, 0.0) };
        let vm_lens = vm.then(|| lens_of(&Camera { fov_y: weapon_lens().fov_y, near: weapon_lens().near, far: weapon_lens().far, ..cam }, (0.5, 0.5)));
        let cover = outline_cover(&depth, w, h, &o, lens_of(&cam, map), vm_lens);
        let want = with_lines(&base, &cover, o.color);
        let (n, m) = diff(&img, &want);
        let lines = cover.iter().filter(|&&a| a > 0.25 * o.color[3]).count();
        t.ok(m <= 1 && lines > 500, &format!("{level}: {name}: against post.wgsl's formula on the CPU over the GPU's depth and image: {n} pixels differ, largest {m}; {lines} pixels of lines"));
        // against the exact depth of rays: the lines where the geometry's edges are (the world; not with the viewmodel)
        if !vm {
            let exact = exact_depth(&b.rs, &ref_camera(&cam), w, h, map);
            let ce = outline_cover(&exact, w, h, &o, lens_of(&cam, map), None);
            let norm = |c: &[f32]| c.iter().map(|a| a / o.color[3]).collect::<Vec<f32>>();
            let (ga, ea, total) = lines_apart(&norm(&cover), &norm(&ce), w, h);
            t.ok(ga * 200 <= total && ea * 200 <= total, &format!("{level}: {name}: against the rays' exact depth: of {total} line pixels {ga} without one of the rays' within a pixel, {ea} of the rays' without one of the GPU's"));
        }
    }
    // without a depth buffer that can be sampled: refused
    let d = arena_scene(None);
    let cam = arena_camera(aspect);
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
    t.res(b.s.set_outline(Some(&Outline::new(1, [0.0, 0.0, 0.0, 1.0]))), "set_outline")?;
    let plain = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET)), "depth")?;
    t.res(b.s.prepare(&mut b.r), "prepare")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    let color = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::CopySrc, after: Layout::CopySrc };
    let depth = DepthAttachment { view: plain.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: false, before: Layout::Undefined, after: Layout::DepthTarget };
    let refused = matches!(b.r.record(&mut e, &b.s, color, Some(depth)), Err(GpuError::Invalid(_)));
    drop(e);
    let bad = [Outline::new(0, [0.0; 4]), Outline::new(9, [0.0; 4]), Outline::new(1, [f32::NAN, 0.0, 0.0, 1.0]), line(1, [0.0; 4], 0.0)];
    let checked = bad.iter().all(|o| matches!(b.s.set_outline(Some(o)), Err(GpuError::Invalid(_))));
    t.ok(refused && checked, &format!("{level}: outlines without a sampled depth buffer: Invalid ({refused}); widths 0 and 9, a colour not finite, crease 0: Invalid ({checked})"));
    Some(())
}

// ---------------------------------------------------------------- FXAA

pub fn fxaa(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = fxaa_on(t, dev, level);
    });
}

/// FXAA 3.11, quality preset 12, on the CPU in f32 as post.wgsl writes it: rgba rows (8 bits, linear: the values are light,
/// luma their square root; else the values as they are), bilinear taps with clamping.  And per pixel whether a decision lay
/// within 2e-4 of its threshold (the GPU filters 8-bit texels in its own precision: such a pixel may go either way)
pub fn fxaa_cpu(img: &[u8], w: u32, h: u32, linear: bool) -> (Vec<u8>, Vec<bool>) {
    let val = |c: u8| if linear { cpu::srgb_decode(c as f64 / 255.0) as f32 } else { c as f32 / 255.0 };
    let px: Vec<[f32; 4]> = img.chunks_exact(4).map(|p| [val(p[0]), val(p[1]), val(p[2]), p[3] as f32 / 255.0]).collect();
    let texel = |x: i32, y: i32| px[(y.clamp(0, h as i32 - 1) as u32 * w + x.clamp(0, w as i32 - 1) as u32) as usize];
    let luma = |c: [f32; 4]| {
        let y = c[0].clamp(0.0, 1.0) * 0.299 + c[1].clamp(0.0, 1.0) * 0.587 + c[2].clamp(0.0, 1.0) * 0.114;
        if linear { libm_sqrt(y) } else { y }
    };
    let tap = |u: f32, v: f32| {
        let (x, y) = (u * w as f32 - 0.5, v * h as f32 - 0.5);
        let (x0, y0) = (cpu::floor(x as f64) as f32, cpu::floor(y as f64) as f32);
        // (the hardware's filter weights: 8 bits of the texel's fraction, as Vulkan's minimum subtexel precision)
        let q8 = |v: f32| cpu::floor((v * 256.0 + 0.5) as f64) as f32 / 256.0;
        let (fx, fy) = (q8(x - x0), q8(y - y0));
        let (i, j) = (x0 as i32, y0 as i32);
        let (a, b, c, d) = (texel(i, j), texel(i + 1, j), texel(i, j + 1), texel(i + 1, j + 1));
        let mut o = [0.0f32; 4];
        for k in 0..4 {
            o[k] = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
        }
        o
    };
    let quality = |i: u32| match i {
        0 => 1.0f32,
        1 => 1.5,
        2 => 2.0,
        3 => 4.0,
        _ => 12.0,
    };
    let (rx, ry) = (1.0 / w as f32, 1.0 / h as f32);
    let mut out = Vec::with_capacity(img.len());
    let mut fragile = Vec::with_capacity((w * h) as usize);
    const EPS: f32 = 2e-4;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut frag = false;
            let mut near = |a: f32, b: f32| frag |= (a - b).abs() < EPS;
            let pos = [(x as f32 + 0.5) * rx, (y as f32 + 0.5) * ry];
            let center = texel(x, y);
            let lm = luma(center);
            let (ls, le, ln, lw) = (luma(texel(x, y + 1)), luma(texel(x + 1, y)), luma(texel(x, y - 1)), luma(texel(x - 1, y)));
            let range_max = ln.max(lw).max(le.max(ls)).max(lm);
            let range_min = ln.min(lw).min(le.min(ls)).min(lm);
            let range = range_max - range_min;
            near(range, 0.0833f32.max(range_max * 0.166));
            let result = if range < 0.0833f32.max(range_max * 0.166) {
                center
            } else {
                let (lnw, lse, lne, lsw) = (luma(texel(x - 1, y - 1)), luma(texel(x + 1, y + 1)), luma(texel(x + 1, y - 1)), luma(texel(x - 1, y + 1)));
                let (lns, lwe) = (ln + ls, lw + le);
                let (eh1, ev1) = (-2.0 * lm + lns, -2.0 * lm + lwe);
                let (lnese, lnwne) = (lne + lse, lnw + lne);
                let (eh2, ev2) = (-2.0 * le + lnese, -2.0 * ln + lnwne);
                let (lnwsw, lswse) = (lnw + lsw, lsw + lse);
                let (eh4, ev4) = (eh1.abs() * 2.0 + eh2.abs(), ev1.abs() * 2.0 + ev2.abs());
                let (eh3, ev3) = (-2.0 * lw + lnwsw, -2.0 * ls + lswse);
                let horz = eh3.abs() + eh4 >= ev3.abs() + ev4;
                near(eh3.abs() + eh4, ev3.abs() + ev4);
                let subpix_a = (lns + lwe) * 2.0 + lnwsw + lnese;
                let (mut lo_n, mut lo_s, mut sign) = (ln, ls, ry);
                if !horz {
                    lo_n = lw;
                    lo_s = le;
                    sign = rx;
                }
                let subpix_b = subpix_a * (1.0 / 12.0) - lm;
                let (gn, gs) = (lo_n - lm, lo_s - lm);
                let mut luma_nn = lo_n + lm;
                let luma_ss = lo_s + lm;
                let pair_n = gn.abs() >= gs.abs();
                near(gn.abs(), gs.abs());
                let gradient = gn.abs().max(gs.abs());
                if pair_n {
                    sign = -sign;
                }
                let subpix_c = (subpix_b.abs() / range).clamp(0.0, 1.0);
                let mut pos_b = pos;
                let mut off = [rx, 0.0];
                if horz {
                    pos_b[1] += sign * 0.5;
                } else {
                    pos_b[0] += sign * 0.5;
                    off = [0.0, ry];
                }
                let mut pos_n = [pos_b[0] - off[0], pos_b[1] - off[1]];
                let mut pos_p = [pos_b[0] + off[0], pos_b[1] + off[1]];
                let subpix_d = -2.0 * subpix_c + 3.0;
                let mut end_n = luma(tap(pos_n[0], pos_n[1]));
                let subpix_e = subpix_c * subpix_c;
                let mut end_p = luma(tap(pos_p[0], pos_p[1]));
                if !pair_n {
                    luma_nn = luma_ss;
                }
                let gscaled = gradient * 0.25;
                let luma_mm = lm - luma_nn * 0.5;
                let subpix_f = subpix_d * subpix_e;
                let mm_below = luma_mm < 0.0;
                end_n -= luma_nn * 0.5;
                end_p -= luma_nn * 0.5;
                let mut done_n = end_n.abs() >= gscaled;
                let mut done_p = end_p.abs() >= gscaled;
                near(end_n.abs(), gscaled);
                near(end_p.abs(), gscaled);
                near(luma_mm, 0.0);
                if !done_n {
                    pos_n = [pos_n[0] - off[0] * quality(1), pos_n[1] - off[1] * quality(1)];
                }
                if !done_p {
                    pos_p = [pos_p[0] + off[0] * quality(1), pos_p[1] + off[1] * quality(1)];
                }
                for i in 2..5 {
                    if done_n && done_p {
                        break;
                    }
                    if !done_n {
                        end_n = luma(tap(pos_n[0], pos_n[1])) - luma_nn * 0.5;
                        done_n = end_n.abs() >= gscaled;
                        near(end_n.abs(), gscaled);
                        if !done_n {
                            pos_n = [pos_n[0] - off[0] * quality(i), pos_n[1] - off[1] * quality(i)];
                        }
                    }
                    if !done_p {
                        end_p = luma(tap(pos_p[0], pos_p[1])) - luma_nn * 0.5;
                        done_p = end_p.abs() >= gscaled;
                        near(end_p.abs(), gscaled);
                        if !done_p {
                            pos_p = [pos_p[0] + off[0] * quality(i), pos_p[1] + off[1] * quality(i)];
                        }
                    }
                }
                let (dst_n, dst_p) = if horz { (pos[0] - pos_n[0], pos_p[0] - pos[0]) } else { (pos[1] - pos_n[1], pos_p[1] - pos[1]) };
                let good_n = (end_n < 0.0) != mm_below;
                let good_p = (end_p < 0.0) != mm_below;
                let span = dst_p + dst_n;
                let dst = dst_n.min(dst_p);
                let good = if dst_n < dst_p { good_n } else { good_p };
                near(if dst_n < dst_p { end_n } else { end_p }, 0.0);
                let subpix_g = subpix_f * subpix_f;
                let offset = -dst / span + 0.5;
                let shift = (if good { offset } else { 0.0 }).max(subpix_g * 0.75);
                let mut q = pos;
                if horz {
                    q[1] += shift * sign;
                } else {
                    q[0] += shift * sign;
                }
                tap(q[0], q[1])
            };
            for c in 0..4 {
                let v = result[c].clamp(0.0, 1.0) as f64;
                out.push(if linear && c < 3 { cpu::srgb_encode(v) } else { (v * 255.0 + 0.5) as u8 });
            }
            fragile.push(frag);
        }
    }
    (out, fragile)
}

/// the square root in f32 (the reference's own, rounded)
fn libm_sqrt(y: f32) -> f32 {
    cpu::sqrt(y as f64) as f32
}

/// pixels whose colour changed against a base: (count, how many lie where the base has no luma edge within a pixel)
fn changed_away_from_edges(base: &[u8], img: &[u8], w: u32, h: u32) -> (u32, u32) {
    let l = |x: u32, y: u32| {
        let k = ((y * w + x) * 4) as usize;
        base[k] as f32 * 0.299 + base[k + 1] as f32 * 0.587 + base[k + 2] as f32 * 0.114
    };
    let (mut n, mut away) = (0, 0);
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) * 4) as usize;
            if base[k..k + 3] == img[k..k + 3] {
                continue;
            }
            n += 1;
            let c = l(x, y);
            let mut edge = false;
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (1, -1), (-1, 1)] {
                let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                if xx >= 0 && yy >= 0 && (xx as u32) < w && (yy as u32) < h && (l(xx as u32, yy as u32) - c).abs() > 255.0 * 0.03 {
                    edge = true;
                }
            }
            away += !edge as u32;
        }
    }
    (n, away)
}

fn fxaa_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let aspect = w as f64 / h as f64;
    let cam = arena_camera(aspect);
    let d = arena_scene(None);
    // a linear target, an sRGB one (FXAA's luma from the light's square root), Bgra8Unorm with the shader's sRGB
    for (name, format, encode) in [("Rgba8Unorm, linear", Format::Rgba8Unorm, false), ("Rgba8Srgb", Format::Rgba8Srgb, true), ("Bgra8Unorm, the shader's sRGB", Format::Bgra8Unorm, true)] {
        let mut desc = render3d::RendererDesc::new(format);
        desc.encode_srgb = encode;
        let mut r = t.res(Renderer::new(dev, &desc), "Renderer::new")?;
        let (mut s, _) = t.res(arena_in(&mut r, &d, &cam, &Sun::DEFAULT, false), "the arena")?;
        t.res(s.set_outline(Some(&Outline::new(1, [0.0, 0.0, 0.0, 1.0]))), "set_outline")?;
        let mut off = t.res(Off::new(dev, w, h, format), "target")?;
        let mut base = shot(t, dev, &mut off, &mut r, &mut s, "without FXAA")?;
        s.set_antialias(Antialias::Fxaa);
        let mut img = shot(t, dev, &mut off, &mut r, &mut s, "FXAA")?;
        if format == Format::Bgra8Unorm {
            for p in base.chunks_exact_mut(4).chain(img.chunks_exact_mut(4)) {
                p.swap(0, 2);
            }
        }
        let (want, fragile) = fxaa_cpu(&base, w, h, format == Format::Rgba8Srgb);
        let (mut over, mut either) = (0u32, 0u32);
        let mut largest = 0u8;
        for ((p, q), f) in img.chunks_exact(4).zip(want.chunks_exact(4)).zip(fragile.iter()) {
            let dd = (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
            if *f {
                either += (dd > 1) as u32;
                continue;
            }
            over += (dd > 1) as u32;
            largest = largest.max(dd);
        }
        let fragile_n = fragile.iter().filter(|f| **f).count() as u32;
        let (changed, away) = changed_away_from_edges(&base, &img, w, h);
        t.ok(over * 2000 <= w * h && changed > 1000 && away == 0 && either * 50 <= w * h,
             &format!("{level}: FXAA, {name}: against FXAA 3.11 on the CPU over the GPU's image: {over} pixels off by more than 1 (largest {largest}); {fragile_n} with a \
                       decision within 2e-4 of its threshold ({either} of them off); {changed} pixels changed, {away} of them away from an edge"));
        s.set_antialias(Antialias::None);
        let again = shot(t, dev, &mut off, &mut r, &mut s, "FXAA off")?;
        let mut base2 = again.clone();
        if format == Format::Bgra8Unorm {
            for p in base2.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
        }
        let (n, _) = diff(&base, &base2);
        t.ok(n == 0, &format!("{level}: FXAA, {name}: switched off again the image is the one before: {n} pixels differ"));
    }
    // a plain image stays as it is: a wall filling the view
    let wall: Described = (vec![plate()], Vec::new(), vec![(Spec { mesh: 0, material: 0, pos: [0.0, 0.0, 0.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [50.0, 50.0, 1.0] },
                                                             MaterialDesc::new(Material::color(0.7, 0.4, 0.3)))]);
    let flat_cam = Camera::look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (mut s, _) = t.res(described_in(&mut r, &wall, &flat_cam, &Sun::DEFAULT), "wall")?;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let base = shot(t, dev, &mut off, &mut r, &mut s, "wall")?;
    s.set_antialias(Antialias::Fxaa);
    let img = shot(t, dev, &mut off, &mut r, &mut s, "wall FXAA")?;
    let (n, _) = diff(&base, &img);
    t.ok(n == 0, &format!("{level}: FXAA over a plain wall: {n} pixels differ"));
    Some(())
}

// ---------------------------------------------------------------- the viewmodel

pub fn viewmodel(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = viewmodel_on(t, dev, level);
        let _ = viewmodel_light_on(t, dev, level);
    });
}

/// v turned by the unit quaternion q (x, y, z, w)
fn turn(q: [f32; 4], v: [f64; 3]) -> [f64; 3] {
    let l = cpu::sqrt(q.iter().map(|x| *x as f64 * *x as f64).sum());
    let (x, y, z, w) = (q[0] as f64 / l, q[1] as f64 / l, q[2] as f64 / l, q[3] as f64 / l);
    let u = [x, y, z];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let c1 = cross(u, v);
    let t1 = [c1[0] + w * v[0], c1[1] + w * v[1], c1[2] + w * v[2]];
    let c2 = cross(u, t1);
    [v[0] + 2.0 * c2[0], v[1] + 2.0 * c2[1], v[2] + 2.0 * c2[2]]
}

/// the column-major matrix of a viewmodel's item in the world: the camera's axes (right, up, back) times its transform
fn vm_matrix(cam: &Camera, sp: &Spec) -> [f64; 16] {
    let f = cam.forward;
    let u = cam.up;
    let r = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
    let to_world = |v: [f64; 3]| [r[0] * v[0] + u[0] * v[1] - f[0] * v[2], r[1] * v[0] + u[1] * v[1] - f[1] * v[2], r[2] * v[0] + u[2] * v[1] - f[2] * v[2]];
    let mut m = [0.0; 16];
    for k in 0..3 {
        let mut e = [0.0; 3];
        e[k] = sp.scale[k] as f64;
        let c = to_world(turn(sp.rot, e));
        m[k * 4..k * 4 + 3].copy_from_slice(&c);
    }
    let p = to_world(sp.pos);
    m[12..15].copy_from_slice(&[cam.eye[0] + p[0], cam.eye[1] + p[1], cam.eye[2] + p[2]]);
    m[15] = 1.0;
    m
}

/// the weapon's reference: its items (rays see them, they cast nothing) and the world's (they only cast shadows)
fn weapon_reference(d: &Described, sun: &Sun, cam: &Camera) -> RefScene {
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for (spec, m) in &d.2 {
        rs.add_look_as(&d.0[spec.mesh as usize], spec, crate::tex::look_of_desc(m), false, true);
    }
    for (spec, c) in weapon() {
        let look = cpu::Look { roughness: 0.45, metallic: 0.3, toon: crate::tex::look_of_desc(&d.2[0].1).toon, ..cpu::Look::color(c) };
        rs.add_matrix_as(&d.0[spec.mesh as usize], &vm_matrix(cam, &spec), look, true, false);
    }
    rs
}

/// the image against the two ray casters: the weapon's rays (its lens) first, the world's where they miss.  Pixels whose
/// rays around the centre agree within 1 (tol); at edges any colour those rays see.  only_weapon: judge the weapon's
/// pixels alone.  (bad, solid pixels of the weapon, first bad)
#[allow(clippy::too_many_arguments)]
fn judge_layers(world: &RefScene, gun: &RefScene, cam: &Camera, w: u32, h: u32, img: &[u8], tol: u8, only_weapon: bool) -> (u32, u32, Option<(u32, u32, [u8; 3], [u8; 3])>) {
    let lens = weapon_lens();
    let wc = ref_camera(cam);
    let vc = RefCamera::new(cam.eye, cam.forward, cam.up, lens.fov_y, cam.aspect, lens.near, lens.far);
    let shoot = |x: f64, y: f64| -> (u32, Option<[u8; 3]>) {
        if let Some(hit) = gun.trace(vc.eye, vc.ray(x, y, w, h), vc.near, vc.far) {
            return (1000 + hit.inst as u32, Some(hit.rgb));
        }
        match world.trace(wc.eye, wc.ray(x, y, w, h), wc.near, wc.far) {
            Some(hit) => (hit.inst as u32, Some(hit.rgb)),
            None => (u32::MAX, None),
        }
    };
    let (mut bad, mut solid, mut first) = (0, 0, None);
    let e = 1.0 / 64.0;
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            let k = ((y * w + x) * 4) as usize;
            let got = [img[k], img[k + 1], img[k + 2]];
            let c = shoot(cx, cy);
            if only_weapon && (c.0 < 1000 || c.0 == u32::MAX) {
                continue;
            }
            let around = [shoot(cx - e, cy), shoot(cx + e, cy), shoot(cx, cy - e), shoot(cx, cy + e)];
            let rgb = |v: &(u32, Option<[u8; 3]>)| v.1.unwrap_or(CLEAR8);
            let near = |a: [u8; 3], b: [u8; 3]| (0..3).all(|i| a[i].abs_diff(b[i]) <= tol);
            let solid_px = around.iter().all(|a| a.0 == c.0 && near(rgb(a), rgb(&c)));
            if solid_px {
                solid += (c.0 >= 1000 && c.0 != u32::MAX) as u32;
                if !near(got, rgb(&c)) {
                    bad += 1;
                    first.get_or_insert((x, y, got, rgb(&c)));
                }
            } else if !(near(got, rgb(&c)) || around.iter().any(|a| near(got, rgb(a)))) {
                // (an edge: any colour between those its rays see)
                let lo = (0..3).map(|i| around.iter().chain([&c]).map(|a| rgb(a)[i]).min().unwrap_or(0)).collect::<Vec<u8>>();
                let hi = (0..3).map(|i| around.iter().chain([&c]).map(|a| rgb(a)[i]).max().unwrap_or(255)).collect::<Vec<u8>>();
                if !(0..3).all(|i| got[i] as i32 >= lo[i] as i32 - tol as i32 && got[i] as i32 <= hi[i] as i32 + tol as i32) {
                    bad += 1;
                    first.get_or_insert((x, y, got, rgb(&c)));
                }
            }
        }
    }
    (bad, solid, first)
}

fn viewmodel_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let aspect = w as f64 / h as f64;
    let cam = wall_camera(aspect);
    let d = arena_scene(None);
    let sun = Sun::DEFAULT;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &sun, true)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "viewmodel")?;
    let depth = read_depth(t, dev, &off)?;
    let gun = weapon_reference(&d, &sun, &cam);
    let (bad, solid, first) = judge_layers(&b.rs, &gun, &cam, w, h, &img, 1, false);
    // the weapon reaches into the wall: in the world its barrel's end lies behind the wall's face
    let tip = vm_matrix(&cam, &weapon()[1].0);
    let inside = tip[12] + tip[4] * 1.0 > 8.0 || tip[12] - tip[4] * 1.0 > 8.0;
    t.ok(bad == 0 && solid > 500 && inside, &format!("{level}: a weapon whose barrel reaches into the wall, drawn whole in front of it: against the weapon's and the world's rays \
                                                     {bad} pixels wrong of the weapon's {solid}{}", first.map_or(String::new(), |f| format!(" (first at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3))));
    // its lists apart: the world's view and the cascades' never hold it, its own view does
    let lists = b.s.lists();
    let draws = read_words(t, dev, b.s.draws_buffer())?;
    let (vd, _) = b.s.viewmodel_buffers()?;
    let vd = read_words(t, dev, vd)?;
    let (mut in_world, mut in_own, mut world_total) = (0, 0, 0);
    for l in &lists {
        let (a, v) = (draws[l.record as usize * 5 + 1], vd[l.record as usize * 5 + 1]);
        world_total += a;
        if l.mesh == MeshId(3) && l.count == 1 || l.mesh == MeshId(1) && l.capacity > 0 && v > 0 {
            in_own += v;
        }
        if v > 0 {
            in_world += a * 0;
        }
    }
    let own_total: u32 = lists.iter().map(|l| vd[l.record as usize * 5 + 1]).sum();
    t.ok(own_total == 3 && world_total > 5 && in_world == 0 && in_own == 3, &format!("{level}: the weapon's 3 items in its own view's lists ({own_total}), the world's view holds {world_total} others"));
    // the world's depth halved (exactly where the GPU's interpolation of depth keeps the full precision; NVIDIA's and Intel's
    // round in steps of their own: there within 1e-6), the world's image the same but for depth ties, the weapon's depth in
    // 0.5 ..= 1
    b.s.set_viewmodel(None).ok()?;
    let plain = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without the viewmodel")?;
    let pdepth = read_depth(t, dev, &off)?;
    let (mut halved, mut exact, mut weapon_px, mut wrong, mut other, mut most) = (0, 0, 0, 0, 0, 0.0f32);
    for k in 0..depth.len() {
        let same_px = img[k * 4..k * 4 + 3] == plain[k * 4..k * 4 + 3];
        if depth[k] > 0.5 {
            weapon_px += 1;
            wrong += (depth[k] > 1.0) as u32;
        } else {
            let half = pdepth[k] * 0.5;
            let near = (depth[k] - half).abs() <= 1e-6;
            halved += near as u32;
            exact += (depth[k] == half) as u32;
            most = most.max((depth[k] - half).abs());
            wrong += !near as u32;
            other += !same_px as u32;
        }
    }
    let fresh = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &sun, false)?;
    let mut fresh = fresh;
    let alone = shot(t, dev, &mut off, &mut fresh.r, &mut fresh.s, "the world alone")?;
    let (n, _) = diff(&plain, &alone);
    t.ok(wrong == 0 && other * 10_000 <= w * h && halved > 1000 && weapon_px > 500 && n == 0,
         &format!("{level}: the world's depth halved at {halved} pixels ({exact} of them exactly, the others within 1e-6; largest difference {most:.1e}), {wrong} not, the world's image the same but for depth ties \
                   ({other} pixels differ); the weapon's depth in 0.5 ..= 1 at {weapon_px}; set_viewmodel(None): the world alone ({n} pixels differ)"));
    // with and without the occlusion test; refusals
    t.res(b.s.set_viewmodel(Some(&weapon_lens())), "set_viewmodel")?;
    b.s.set_occlusion(false);
    let flat = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without the occlusion test")?;
    b.s.set_occlusion(true);
    let (n2, _) = diff(&img, &flat);
    let lens_bad = [render3d::Viewmodel::new(0.0, 0.1, 1.0), render3d::Viewmodel::new(1.0, 0.0, 1.0), render3d::Viewmodel::new(1.0, 0.5, 0.5), render3d::Viewmodel::new(1.0, 0.1, f64::INFINITY)];
    let refused = lens_bad.iter().all(|v| matches!(b.s.set_viewmodel(Some(v)), Err(GpuError::Invalid(_))));
    let mut glass = MaterialDesc::new(Material { base_color: [0.5, 0.5, 1.0, 0.5], ..Material::color(0.5, 0.5, 1.0) });
    glass.blend = true;
    let gm = t.res(b.r.add_material_desc(&glass), "add_material_desc")?;
    let id = t.res(b.s.add_viewmodel(Instance { mesh: MeshId(1), material: gm, transform: Transform::at([0.0, 0.0, -0.5]) }), "add_viewmodel")?;
    let blended = matches!(b.s.prepare(&mut b.r), Err(GpuError::Invalid(_)));
    t.res(b.s.remove(id), "remove")?;
    let back = shot(t, dev, &mut off, &mut b.r, &mut b.s, "after the refusals")?;
    let (n3, _) = diff(&img, &back);
    t.ok(n2 == 0 && refused && blended && n3 == 0, &format!("{level}: without the occlusion test {n2} pixels differ; lenses out of range Invalid ({refused}); a blended item of the \
                                                            weapon Invalid at prepare ({blended}); removed, the image as before ({n3} pixels differ)"));
    Some(())
}

/// the weapon lit by the scene: in a roof's shadow and in the sun (the world's shadow map at its place, against exact shadow
/// rays from its points); under a sun overhead, the camera turned about the vertical: the weapon the same
fn viewmodel_light_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let aspect = w as f64 / h as f64;
    let cam = wall_camera(aspect);
    let sun = Sun::DEFAULT;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let flat = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    for roof in [true, false] {
        let mut d = arena_scene(None);
        if roof {
            d.2.push((Spec { mesh: 0, material: 0, pos: [6.0, 3.0, 1.0], rot: flat, scale: [4.0, 6.0, 1.0] }, MaterialDesc::new(Material::color(0.5, 0.5, 0.5))));
        }
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &sun, true)?;
        t.res(b.s.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 2048, distance: 30.0, ..render3d::Shadows::DEFAULT })), "set_shadows")?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "weapon in light")?;
        let mut gun = weapon_reference(&d, &sun, &cam);
        gun.shadows = true;
        b.rs.shadows = true;
        let (bad, solid, first) = judge_layers(&b.rs, &gun, &cam, w, h, &img, 1, true);
        t.ok(bad == 0 && solid > 500, &format!("{level}: the weapon {}: against exact shadow rays from its points, {bad} of its {solid} pixels wrong{}",
                                               if roof { "under a roof (in its shadow)" } else { "in the sun" },
                                               first.map_or(String::new(), |f| format!(" (first at {}, {}: {:?}, expected {:?})", f.0, f.1, f.2, f.3))));
    }
    // a sun overhead, the camera turned by a quarter about the vertical: the weapon's pixels the same
    let up = Sun { direction: [0.0, 1.0, 0.0], ..Sun::DEFAULT };
    let d = arena_scene(None);
    let mut shots = Vec::new();
    for yaw in [0.0f64, core::f64::consts::FRAC_PI_2] {
        let (s, c) = cpu::sincos(yaw);
        let f = cam.forward;
        let turned = Camera { forward: [f[0] * c + f[2] * s, f[1], -f[0] * s + f[2] * c], ..cam };
        let turned = Camera::look_at(turned.eye, [turned.eye[0] + turned.forward[0], turned.eye[1] + turned.forward[1], turned.eye[2] + turned.forward[2]], [0.0, 1.0, 0.0],
                                     cam.fov_y, aspect, cam.near, cam.far);
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &turned, &up, true)?;
        let with = shot(t, dev, &mut off, &mut b.r, &mut b.s, "turned")?;
        b.s.set_viewmodel(None).ok()?;
        let without = shot(t, dev, &mut off, &mut b.r, &mut b.s, "turned, without")?;
        let mask: Vec<bool> = with.chunks_exact(4).zip(without.chunks_exact(4)).map(|(a, b)| a != b).collect();
        shots.push((with, mask));
    }
    let (mut differ, mut moved, mut largest) = (0, 0, 0u8);
    for k in 0..shots[0].1.len() {
        if shots[0].1[k] != shots[1].1[k] {
            moved += 1;
        } else if shots[0].1[k] {
            let dd = (0..3).map(|i| shots[0].0[k * 4 + i].abs_diff(shots[1].0[k * 4 + i])).max().unwrap_or(0);
            largest = largest.max(dd);
            differ += (dd > 1) as u32;
        }
    }
    let n = shots[0].1.iter().filter(|v| **v).count();
    t.ok(differ as usize * 1000 <= n && moved <= 20 && n > 500, &format!("{level}: a sun overhead, the camera turned a quarter: the weapon's {n} pixels stay ({moved} moved, {differ} off by more than 1, largest {largest})"));
    Some(())
}

// ---------------------------------------------------------------- particles

pub fn particles(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = particles_list_on(t, dev, level);
        let _ = particles_image_on(t, dev, level);
        let _ = particles_ring_on(t, dev, level);
    });
}

/// the ring's records and the list after a frame (count, entries)
fn read_particles(t: &mut T, dev: &Device, s: &Scene) -> Option<(Vec<[u32; 16]>, u32, Vec<[u32; 4]>)> {
    let rw = read_words(t, dev, s.particle_records()?)?;
    let recs = rw.chunks_exact(16).map(|c| c.try_into().unwrap_or([0; 16])).collect();
    let lw = read_words(t, dev, s.particle_buffer()?)?;
    let n = lw[1].min((lw.len() as u32 - 4) / 4);
    let items = (0..n as usize).map(|k| [lw[4 + k * 4], lw[5 + k * 4], lw[6 + k * 4], lw[7 + k * 4]]).collect();
    Some((recs, lw[1], items))
}

/// a record's age at a frame's clock, as the GPU takes it (a wrapped difference of microseconds in f32) and alive
fn age(rec: &[u32; 16], now: u32) -> (f64, bool) {
    let a = (now.wrapping_sub(rec[3]) as f32 * 1e-6) as f64;
    (a, (a as f32) < f32::from_bits(rec[7]))
}

/// a record's place in the world (relative to the origin) at its age: its own space, or the camera's for the viewmodel's
fn place(rec: &[u32; 16], t: f64, cam: &Camera, origin: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let (p, v) = render3d::particles::motion(rec, t);
    if rec[14] & 4 == 0 {
        return (p, v);
    }
    let f = cam.forward;
    let u = cam.up;
    let r = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
    let w = |a: [f64; 3]| [r[0] * a[0] + u[0] * a[1] - f[0] * a[2], r[1] * a[0] + u[1] * a[1] - f[1] * a[2], r[2] * a[0] + u[2] * a[1] - f[2] * a[2]];
    let e = [cam.eye[0] - origin[0], cam.eye[1] - origin[1], cam.eye[2] - origin[2]];
    let pw = w(p);
    ([e[0] + pw[0], e[1] + pw[1], e[2] + pw[2]], w(v))
}

fn particles_list_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let cam = arena_camera(aspect);
    let d = arena_scene(None);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
        t.res(arena_particles(&mut b.s, false, 2.0), "particles")?;
        shot(t, dev, &mut off, &mut b.r, &mut b.s, "particles")?;
        runs.push((read_particles(t, dev, &b.s)?, b));
    }
    let same = runs[0].0.0 == runs[1].0.0;
    let (b, (recs, count, items)) = {
        let (x, b) = runs.pop()?;
        (b, x)
    };
    // the list: those alive at the frame's clock, sorted by key; the keys' order the distances' (f64) but for near ties
    let now = (((b.s.time() - b.s.particle_epoch()) * 1e6) as u64 & 0xFFFF_FFFF) as u32;
    let alive: Vec<u32> = (0..recs.len() as u32).filter(|&k| age(&recs[k as usize], now).1).collect();
    let mut listed: Vec<u32> = items.iter().map(|e| e[1]).collect();
    listed.sort();
    let sorted = items.windows(2).all(|p| (p[0][0], p[0][1]) < (p[1][0], p[1][1]));
    let eye = [cam.eye[0], cam.eye[1], cam.eye[2]];
    let dist = |k: u32| {
        let r = &recs[k as usize];
        let (p, _) = place(r, age(r, now).0, &cam, [0.0; 3]);
        let dd = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        dd[0] * dd[0] + dd[1] * dd[1] + dd[2] * dd[2]
    };
    let out_of_order = items.windows(2).filter(|p| dist(p[0][1]) < dist(p[1][1]) * (1.0 - 1e-5)).count();
    t.ok(same && listed == alive && count as usize == alive.len() && sorted && out_of_order == 0 && alive.len() > 50,
         &format!("{level}: the same emits, the same records ({same}); the list: {count} alive (the CPU: {}), sorted {sorted}, {out_of_order} out of the distances' order",
                  alive.len()));
    Some(())
}

/// the particles laid over an image on the CPU, in the GPU's order: each a quad (the camera's axes, or along its velocity)
/// that a pixel's ray meets at (u, v), opacity 1 - u^2 - v^2 times its alpha, its depth tested against the image's
#[allow(clippy::too_many_arguments)]
fn splat(base: &[u8], depth: &[f32], w: u32, h: u32, cam: &Camera, vm: bool, recs: &[[u32; 16]], items: &[[u32; 4]], now: u32) -> Vec<u8> {
    let mut img: Vec<f64> = base.iter().map(|&v| v as f64).collect();
    let lens = weapon_lens();
    let wc = ref_camera(cam);
    let vc = RefCamera::new(cam.eye, cam.forward, cam.up, lens.fov_y, cam.aspect, lens.near, lens.far);
    let world_map = if vm { (0.5, 0.0) } else { (1.0, 0.0) };
    let half = |w: u32, hi: bool| render3d::from_half(if hi { (w >> 16) as u16 } else { w as u16 }) as f64;
    let f = cam.forward;
    let u = cam.up;
    let r = [f[1] * u[2] - f[2] * u[1], f[2] * u[0] - f[0] * u[2], f[0] * u[1] - f[1] * u[0]];
    for e in items {
        let rec = &recs[e[1] as usize];
        let (a, _) = age(rec, now);
        let life = f32::from_bits(rec[7]) as f64;
        let tt = (a / life).clamp(0.0, 1.0);
        let (p, vel) = place(rec, a, cam, [0.0; 3]);
        let size = half(rec[8], false) + (half(rec[8], true) - half(rec[8], false)) * tt;
        let hs = 0.5 * size;
        let mix = |x: f64, y: f64| x + (y - x) * tt;
        let color = [mix(half(rec[9], false), half(rec[11], false)), mix(half(rec[9], true), half(rec[11], true)), mix(half(rec[10], false), half(rec[12], false)),
                     mix(half(rec[10], true), half(rec[12], true))];
        let (mut ax, mut ay) = ([r[0] * hs, r[1] * hs, r[2] * hs], [u[0] * hs, u[1] * hs, u[2] * hs]);
        if rec[14] & 2 != 0 {
            let along = cpu::dot(vel, f);
            let across = [vel[0] - f[0] * along, vel[1] - f[1] * along, vel[2] - f[2] * along];
            let len = cpu::sqrt(cpu::dot(across, across));
            if len > 1e-6 {
                let aa = [across[0] / len, across[1] / len, across[2] / len];
                let stretch = half(rec[14], true);
                let l = hs + 0.5 * len * stretch;
                ax = [aa[0] * l, aa[1] * l, aa[2] * l];
                let b = [f[1] * aa[2] - f[2] * aa[1], f[2] * aa[0] - f[0] * aa[2], f[0] * aa[1] - f[1] * aa[0]];
                ay = [b[0] * hs, b[1] * hs, b[2] * hs];
            }
        }
        let vmp = rec[14] & 4 != 0;
        let (rc, map) = if vmp { (&vc, (0.5, 0.5)) } else { (&wc, world_map) };
        let n = [ax[1] * ay[2] - ax[2] * ay[1], ax[2] * ay[0] - ax[0] * ay[2], ax[0] * ay[1] - ax[1] * ay[0]];
        let (lx, ly) = (cpu::dot(ax, ax), cpu::dot(ay, ay));
        if lx <= 0.0 || ly <= 0.0 {
            continue;
        }
        let additive = rec[14] & 1 != 0;
        let c = cpu::neutral([color[0].max(0.0), color[1].max(0.0), color[2].max(0.0)]);
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                let o = rc.origin(cx, cy, w, h);
                let dir = rc.ray(cx, cy, w, h);
                let den = cpu::dot(dir, n);
                if den == 0.0 {
                    continue;
                }
                let s = cpu::dot([p[0] - o[0], p[1] - o[1], p[2] - o[2]], n) / den;
                let q = [o[0] + dir[0] * s - p[0], o[1] + dir[1] * s - p[1], o[2] + dir[2] * s - p[2]];
                let (uu, vv) = (cpu::dot(q, ax) / lx, cpu::dot(q, ay) / ly);
                if uu.abs() > 1.0 || vv.abs() > 1.0 || s <= rc.span().0 {
                    continue;
                }
                let dp = rc.depth(s) * map.0 + map.1;
                let k = (y * w + x) as usize;
                if !(dp as f32 > depth[k]) {
                    continue;
                }
                let al = (1.0 - uu * uu - vv * vv).max(0.0) * color[3].clamp(0.0, 1.0);
                let cover = if additive { 0.0 } else { al };
                for i in 0..3 {
                    let v = c[i] * al + img[k * 4 + i] / 255.0 * (1.0 - cover);
                    img[k * 4 + i] = cpu::floor(v.clamp(0.0, 1.0) * 255.0 + 0.5);
                }
            }
        }
    }
    img.iter().map(|&v| v as u8).collect()
}

fn particles_image_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let d = arena_scene(None);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    for (name, cam, vm) in [("in the arena", arena_camera(aspect), false), ("with the viewmodel's muzzle flash", wall_camera(aspect), true)] {
        let mut a = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, vm)?;
        let base = shot(t, dev, &mut off, &mut a.r, &mut a.s, "without particles")?;
        let depth = read_depth(t, dev, &off)?;
        let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, vm)?;
        t.res(arena_particles(&mut b.s, vm, 1.0), "particles")?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, name)?;
        let (recs, count, items) = read_particles(t, dev, &b.s)?;
        let now = (((b.s.time() - b.s.particle_epoch()) * 1e6) as u64 & 0xFFFF_FFFF) as u32;
        let want = splat(&base, &depth, w, h, &cam, vm, &recs, &items, now);
        let (mut over, mut largest) = (0u32, 0u8);
        for (p, q) in img.chunks_exact(4).zip(want.chunks_exact(4)) {
            let dd = (0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0);
            over += (dd > 2) as u32;
            largest = largest.max(dd);
        }
        let (touched, _) = diff(&base, &img);
        t.ok(over <= w * h / 1000 && touched > 50, &format!("{level}: {count} particles {name} against them laid on the CPU over the image without them: \
                                                             {over} pixels off by more than 2 (largest {largest}); {touched} pixels they touch"));
    }
    Some(())
}

fn particles_ring_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let cam = arena_camera(aspect);
    let d = arena_scene(None);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut b = arena(t, dev, Format::Rgba8Unorm, &d, &cam, &Sun::DEFAULT, false)?;
    let base = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without particles")?;
    let e = emitter(5, false, false, [5.0, 5.0], [0.5, 1.0], 1.0, [0.3, 0.3], [[0.8, 0.8, 0.8, 0.8]; 2], 0.0, 0.0);
    // refusals: no room yet, an emitter out of range, a removed one
    let id = t.res(b.s.add_emitter(&e), "add_emitter")?;
    let no_room = matches!(b.s.emit(id, [0.0; 3], [0.0, 1.0, 0.0], 1), Err(GpuError::Invalid(_)));
    let mut bad = e;
    bad.life = [0.0, 1.0];
    let bad_e = matches!(b.s.add_emitter(&bad), Err(GpuError::Invalid(_)));
    t.res(b.s.set_particles(64), "set_particles")?;
    let gone = t.res(b.s.add_emitter(&e), "add_emitter")?;
    t.res(b.s.remove_emitter(gone), "remove_emitter")?;
    let removed = matches!(b.s.emit(gone, [0.0; 3], [0.0, 1.0, 0.0], 1), Err(GpuError::Invalid(_)));
    // the ring's end: 100 at once leave the last 64, then 30 and 50 the newest 64
    b.s.set_time(1.0);
    t.res(b.s.emit(id, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 100), "emit")?;
    shot(t, dev, &mut off, &mut b.r, &mut b.s, "100 into 64")?;
    let (_, c1, _) = read_particles(t, dev, &b.s)?;
    b.s.set_time(1.1);
    t.res(b.s.emit(id, [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], 30), "emit")?;
    t.res(b.s.emit(id, [-1.0, 1.0, 0.0], [0.0, 1.0, 0.0], 50), "emit")?;
    // a frame prepared and never submitted: its particles come with the next
    t.res(b.s.prepare(&mut b.r), "prepare")?;
    b.s.set_time(1.2);
    shot(t, dev, &mut off, &mut b.r, &mut b.s, "30 and 50")?;
    let (recs, c2, _) = read_particles(t, dev, &b.s)?;
    let newest = recs.iter().filter(|r| r[3] == (((1.1 - b.s.particle_epoch()) * 1e6) as u64 as u32)).count();
    // a new origin and clear_particles: none; capacity 0: the image without them
    b.s.set_origin([0.0, 0.0, 1.0]);
    shot(t, dev, &mut off, &mut b.r, &mut b.s, "new origin")?;
    let (_, c3, _) = read_particles(t, dev, &b.s)?;
    b.s.set_origin([0.0; 3]);
    t.res(b.s.emit(id, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 10), "emit")?;
    b.s.clear_particles();
    shot(t, dev, &mut off, &mut b.r, &mut b.s, "cleared")?;
    let (_, c4, _) = read_particles(t, dev, &b.s)?;
    t.res(b.s.emit(id, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 10), "emit")?;
    t.res(b.s.set_particles(0), "set_particles")?;
    let none = shot(t, dev, &mut off, &mut b.r, &mut b.s, "no particles")?;
    let (n, _) = diff(&base, &none);
    t.ok(no_room && bad_e && removed && c1 == 64 && c2 == 64 && newest == 64 && c3 == 0 && c4 == 0 && n == 0,
         &format!("{level}: refusals ({no_room}, {bad_e}, {removed}); 100 into a ring of 64: {c1} alive; 30 and 50 more over a frame never submitted: {c2} alive, the newest 50 of \
                   them ({newest}, all of the last 64 born then); a new origin: {c3}; cleared: {c4}; no room: the image without them ({n} pixels differ)"));
    // a streak flying sideways is wider than tall on the screen
    t.res(b.s.set_particles(16), "set_particles")?;
    let mut st = emitter(9, true, true, [1.0, 1.0], [6.0, 6.0], 0.0, [0.08, 0.08], [[4.0, 4.0, 4.0, 1.0]; 2], 0.0, 0.0);
    st.stretch = 0.2;
    let sid = t.res(b.s.add_emitter(&st), "add_emitter")?;
    b.s.set_time(5.0);
    let flat_cam = Camera::look_at([0.0, 2.0, 10.0], [0.0, 2.0, 0.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    b.s.set_camera(&flat_cam);
    let empty = shot(t, dev, &mut off, &mut b.r, &mut b.s, "before the streak")?;
    t.res(b.s.emit(sid, [-1.0, 2.0, 0.0], [1.0, 0.0, 0.0], 1), "emit")?;
    b.s.set_time(5.1);
    let streak = shot(t, dev, &mut off, &mut b.r, &mut b.s, "streak")?;
    let (mut x0, mut x1, mut y0, mut y1) = (w, 0, h, 0);
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) * 4) as usize;
            if empty[k..k + 3] != streak[k..k + 3] {
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
        }
    }
    t.ok(x1 > x0 && (x1 - x0) > 3 * (y1 - y0 + 1), &format!("{level}: a streak flying sideways: {} x {} pixels", x1.saturating_sub(x0) + 1, y1.saturating_sub(y0) + 1));
    Some(())
}

// ---------------------------------------------------------------- the toon comparison sheet

/// RGBA rows as a PNG (img_png takes B, G, R, x)
fn png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut bgrx = Vec::with_capacity(rgba.len());
    for p in rgba.chunks_exact(4) {
        bgrx.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    img_png::encode(&bgrx, w as usize, h as usize, w as usize * 4, false)
}

/// render3dtest --toon-sheet DIR [--res WxH]: the arena (two figures, a crate, a ramp, a pillar, a ball) in the four looks of
/// common::toon_variants with their outlines and FXAA, the sun's shadows (2 cascades of 2048), as an sRGB image of WxH (960 x
/// 540) each: DIR/toon_A.png .. toon_D.png, and DIR/toon_sheet.png with all four (A B over C D), and the same scene in PBR
/// (DIR/toon_pbr.png) to compare
pub fn toon_sheet(dir: &str) -> i32 {
    let mut t = T { fails: 0, validate: false, inst: None };
    let Some(g) = crate::tests::setup(&mut t) else { return 0 };
    let (w, h) = plat::arg_value("--res").and_then(|r| {
        let mut p = r.split('x').filter_map(|x| x.parse::<u32>().ok());
        Some((p.next()?, p.next()?))
    }).unwrap_or((960, 540));
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return 1 };
    let cam = Camera::look_at([5.5, 3.2, 7.0], [-0.6, 0.8, -0.5], [0.0, 1.0, 0.0], 0.8, w as f64 / h as f64, 0.1, f64::INFINITY);
    let sun = Sun { direction: [0.45, 0.8, 0.35], ..Sun::DEFAULT };
    let mut shots = Vec::new();
    let variants = toon_variants();
    let looks: Vec<(String, Option<Toon>, Option<Outline>)> = variants.iter().map(|(n, k, o)| (String::from(*n), Some(*k), *o)).chain([(String::from("PBR"), None, None)]).collect();
    for (k, (name, toon, outline)) in looks.iter().enumerate() {
        let Some(mut r) = t.res(Renderer::new(&dev, &render3d::RendererDesc::new(Format::Rgba8Srgb)), "Renderer::new") else { return 1 };
        // (the rim on the figures and props, not on the ground and the walls: a plane seen at a grazing angle would glow)
        let mut d = arena_scene(*toon);
        for (_, m) in d.2.iter_mut().take(4) {
            if let Some(k) = &mut m.toon {
                k.rim = 0.0;
            }
        }
        let Some((mut s, _)) = t.res(arena_in(&mut r, &d, &cam, &sun, false), "arena") else { return 1 };
        let _ = s.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 2048, distance: 40.0, ..render3d::Shadows::DEFAULT }));
        let _ = s.set_outline(outline.as_ref());
        s.set_antialias(Antialias::Fxaa);
        let Some(mut off) = t.res(Off::new(&dev, w, h, Format::Rgba8Srgb), "target") else { return 1 };
        let Some(px) = shot(&mut t, &dev, &mut off, &mut r, &mut s, name) else { return 1 };
        let tag = if k < 4 { format!("toon_{}", (b'A' + k as u8) as char) } else { String::from("toon_pbr") };
        let file = format!("{dir}/{tag}.png");
        t.ok(plat::write_file(&file, &png(&px, w, h)), &format!("write {file}: {name}"));
        shots.push(px);
    }
    // the sheet: A B over C D, 8 pixels of white between
    let gap = 8u32;
    let (sw, sh) = (2 * w + 3 * gap, 2 * h + 3 * gap);
    let mut sheet = vec![255u8; (sw * sh * 4) as usize];
    for (k, px) in shots.iter().take(4).enumerate() {
        let (ox, oy) = (gap + (k as u32 % 2) * (w + gap), gap + (k as u32 / 2) * (h + gap));
        for y in 0..h {
            let from = (y * w * 4) as usize;
            let to = (((oy + y) * sw + ox) * 4) as usize;
            sheet[to..to + (w * 4) as usize].copy_from_slice(&px[from..from + (w * 4) as usize]);
        }
    }
    let file = format!("{dir}/toon_sheet.png");
    t.ok(plat::write_file(&file, &png(&sheet, sw, sh)), &format!("write {file}: A B over C D"));
    out_line(&format!("adapter: {}", g.adapter.info().name));
    (t.fails > 0) as i32
}

fn out_line(s: &str) {
    crate::tests::out(s);
}
