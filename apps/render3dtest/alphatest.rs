// alphatest -- render3dtest --test alpha: lists per alpha class (docs/RENDER3D.md "Listen je Alpha-Modus"), on Voll and
// on a device limited to Basis.  Instances switch between an opaque and a MASK material (Scene::set_material, and a
// material's own class through Renderer::set_material), mirrored MASK instances turn to the other side: every image
// against the scene built anew and against the ray caster, the lists per mesh, side and class, the draw records laid
// out by width of indices, class and side; a change after prepare shows with the next one.  Detail levels with MASK
// materials: the pick per instance (mesh, side, class) against the CPU, the levels' lists of the same side and class,
// the image against the ray caster.  Shadows: a MASK plate switched to opaque and back against exact shadow rays, opaque
// casters alike with and without set_shadow_mask.  The occlusion test behind a MASK wall: with it = without it over a
// flight, against the ray caster, both classes drawn by phase 2.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{AlphaClass, Camera, Material, MaterialId, MeshId, Mip, Renderer, Sampling, Scene, TextureData, TextureId, TextureRef, Transform, Wrap};

use crate::common::*;
use crate::cpu::{self, RefScene, RefTexture};
use crate::tests::{T, on_levels, ref_camera, shot, sun_of};
use crate::tex::{Built, Tex, build, build_with, look_of, plate_uv};

pub fn alpha(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = switch_on(t, dev, level);
        let _ = lods_on(t, dev, level);
        let _ = shadows_on(t, dev, level);
        let _ = occlusion_on(t, dev, level);
    });
}

/// nearest, without levels (exact against the ray caster, also on small far objects)
const HOLES: Sampling = Sampling { mag_nearest: true, min_nearest: true, mip: Mip::None, wrap_u: Wrap::Repeat, wrap_v: Wrap::Repeat };

/// white, square holes: alpha 30 in a checker of 16 texels, 230 elsewhere
fn holes() -> Vec<u8> {
    (0..128u32 * 128).flat_map(|i| { let (x, y) = (i % 128, i / 128); [255u8, 255, 255, if (x / 16 + y / 16) % 2 == 0 { 30 } else { 230 }] }).collect()
}

/// white, four square windows (alpha 30) of 32 texels at 16 .. 48 and 80 .. 112 in both directions, 230 elsewhere: a
/// quarter open, and the same alpha on both sides of the texture's seams (a ball's uv wraps round)
fn windows() -> Vec<u8> {
    let open = |v: u32| ((v + 16) / 32) % 2 == 1;
    (0..128u32 * 128).flat_map(|i| { let (x, y) = (i % 128, i / 128); [255u8, 255, 255, if open(x) && open(y) { 30 } else { 230 }] }).collect()
}

/// a colour times the texture of holes (texture 0): MASK (cutoff 0.5) or opaque (the holes do not show)
fn holed(r: f32, g: f32, b: f32, mask: bool) -> Material {
    Material { base_color_texture: Some(TextureRef { texture: TextureId(0), sampling: HOLES }), alpha_cutoff: if mask { Some(0.5) } else { None }, ..Material::color(r, g, b) }
}

/// (on a failure) what the rays around the first wrong pixel hit: instance, triangle, colour
fn explain(rs: &RefScene, cam: &Camera, w: u32, h: u32, img: &[u8], c: &cpu::Compared) {
    let Some((x, y, _, _)) = c.first else { return };
    let rc = ref_camera(cam);
    for (dx, dy) in [(0.0, 0.0), (-0.015625, 0.0), (0.015625, 0.0), (0.0, -0.015625), (0.0, 0.015625), (-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let hit = rs.trace(rc.eye, rc.ray(x as f64 + 0.5 + dx, y as f64 + 0.5 + dy, w, h), rc.near, rc.far);
        let (px, py) = ((x as f64 + dx) as u32, (y as f64 + dy) as u32);
        crate::tests::out(&format!("    at {dx}, {dy}: got {:?}, ray {:?}", crate::tests::pixel_of(img, w, px.min(w - 1), py.min(h - 1)),
                                   hit.map(|v| (v.inst, v.tri, v.u, v.v, v.rgb))));
    }
}

fn mirrored(sp: &Spec) -> bool {
    (sp.scale[0] as f64) * (sp.scale[1] as f64) * (sp.scale[2] as f64) < 0.0
}

fn is_mask(c: AlphaClass) -> bool {
    c == AlphaClass::Mask
}

/// the draw records lie by width of indices, class and side (each combination side by side: one multi-draw), and are
/// 0 .. lists
fn laid_out(r: &Renderer, s: &Scene) -> bool {
    let mut l = s.lists();
    l.sort_by_key(|x| x.record);
    let keys: Vec<(bool, bool, bool)> = l.iter().map(|x| (r.mesh_info(x.mesh).is_some_and(|m| m.wide), is_mask(x.class), x.mirrored)).collect();
    keys.windows(2).all(|w| w[0] <= w[1]) && l.iter().enumerate().all(|(k, x)| x.record == k as u32)
}

/// the lists with instances (mesh, mirrored, MASK, count) against those the items ask for
fn lists_as(t: &mut T, r: &Renderer, s: &Scene, items: &[(Spec, Material)], what: &str) {
    let mut want: Vec<(u32, bool, bool, u32)> = Vec::new();
    for (sp, m) in items {
        let key = (sp.mesh, mirrored(sp), m.alpha_cutoff.is_some());
        match want.iter_mut().find(|x| (x.0, x.1, x.2) == key) {
            Some(x) => x.3 += 1,
            None => want.push((key.0, key.1, key.2, 1)),
        }
    }
    want.sort();
    let mut have: Vec<(u32, bool, bool, u32)> = s.lists().iter().filter(|l| l.count > 0).map(|l| (l.mesh.0, l.mirrored, is_mask(l.class), l.count)).collect();
    have.sort();
    let laid = laid_out(r, s);
    let note = if want == have { String::new() } else { format!(", expected {want:?}") };
    t.ok(want == have && laid, &format!("{what}: lists (mesh, mirrored, MASK, instances) {have:?}{note}; draw records by width, class and side: {laid}"));
}

/// the scene's frame against the same items built anew: 0 pixels apart, and against that scene's ray caster
#[allow(clippy::too_many_arguments)]
fn judge(t: &mut T, dev: &Device, off: &mut Off, geos: &[Geo], cam: &Camera, b: &mut Built, items: &[(Spec, Material)], what: &str) -> Option<Vec<u8>> {
    let img = shot(t, dev, off, &mut b.r, &mut b.s, what)?;
    let mut fresh = build(t, dev, geos, &[Tex::exact(128, holes(), true)], items, cam)?;
    let want = shot(t, dev, off, &mut fresh.r, &mut fresh.s, what)?;
    let (n, m) = diff(&img, &want);
    let c = cpu::compare(&fresh.rs, &ref_camera(cam), off.w, off.h, CLEAR8, &img, None);
    t.ok(n == 0 && c.fine(), &format!("{what}: against the scene built anew {n} pixels differ (largest {m}); against the ray caster {}", c.text()));
    lists_as(t, &b.r, &b.s, items, what);
    Some(img)
}

// ---------------------------------------------------------------- an instance switching classes, mirrored MASK

fn switch_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let aspect = w as f64 / h as f64;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let geos = [cube(), plate()];
    let spin = quat(cpu::norm([1.0, 2.0, 0.5]), 0.9);
    let at = |mesh: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material: 0, pos, rot, scale };
    let mut items = vec![
        // 0: switches between opaque and MASK
        (at(0, [-4.5, 1.6, 0.0], spin, [1.2; 3]), holed(0.9, 0.5, 0.3, false)),
        // 1: mirrored, MASK
        (at(0, [-1.5, 1.6, 0.0], spin, [-1.2, 1.2, 1.2]), holed(0.3, 0.7, 0.9, true)),
        // 2: MASK; its material changes its class
        (at(0, [1.5, 1.6, 0.0], spin, [1.2, 1.2, 1.2]), holed(0.8, 0.8, 0.3, true)),
        // 3: mirrored, opaque
        (at(0, [4.5, 1.6, 0.0], spin, [1.2, -1.2, 1.2]), holed(0.6, 0.4, 0.9, false)),
        // 4, 5: plates, MASK, the second mirrored
        (at(1, [-3.0, -1.6, 1.0], quat([0.0, 1.0, 0.0], 0.4), [1.6, 1.4, 1.0]), holed(0.9, 0.9, 0.9, true)),
        (at(1, [3.0, -1.6, 1.0], quat([0.0, 1.0, 0.0], -0.4), [-1.6, 1.4, 1.0]), holed(0.9, 0.6, 0.6, true)),
        // 6: the plate behind
        (at(1, [0.0, 0.0, -3.0], [0.0, 0.0, 0.0, 1.0], [9.0, 5.0, 1.0]), Material::color(0.2, 0.9, 0.4)),
    ];
    let cam = Camera::look_at([0.5, 1.0, 11.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    let mut b = build(t, dev, &geos, &[Tex::exact(128, holes(), true)], &items, &cam)?;
    let img0 = shot(t, dev, &mut off, &mut b.r, &mut b.s, "classes")?;
    let c = cpu::compare(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img0, None);
    let green = img0.chunks_exact(4).filter(|p| p[1] > 120 && p[0] < 90 && p[2] < 120).count();
    t.ok(c.fine() && c.objects_seen == 7, &format!("{level}: opaque and MASK cubes and plates, mirrored and not, against the ray caster: {}", c.text()));
    lists_as(t, &b.r, &b.s, &items, &format!("{level}: the first frame"));
    // (the original materials are 0 .. 6, one per item)
    let orig = items.clone();
    // Scene::set_material: 0 and the mirrored 3 to MASK, the mirrored 1 to opaque
    let m0 = t.res(b.r.add_material(&holed(0.9, 0.5, 0.3, true)), "add_material")?;
    let m3 = t.res(b.r.add_material(&holed(0.6, 0.4, 0.9, true)), "add_material")?;
    let m1 = t.res(b.r.add_material(&holed(0.3, 0.7, 0.9, false)), "add_material")?;
    for (k, m) in [(0usize, m0), (3, m3), (1, m1)] {
        t.res(b.s.set_material(b.ids[k], m), "set_material")?;
    }
    items[0].1.alpha_cutoff = Some(0.5);
    items[3].1.alpha_cutoff = Some(0.5);
    items[1].1.alpha_cutoff = None;
    let img1 = judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: 0 and the mirrored 3 to MASK, the mirrored 1 to opaque (Scene::set_material)"))?;
    let (n, _) = diff(&img0, &img1);
    t.ok(n > 0, &format!("{level}: the switch shows: {n} pixels differ from the first frame; the plate behind through the holes: {green} pixels before"));
    // and back
    for k in [0usize, 3, 1] {
        t.res(b.s.set_material(b.ids[k], MaterialId(k as u32)), "set_material")?;
    }
    items = orig.clone();
    let img2 = judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: and back"))?;
    let (n, m) = diff(&img0, &img2);
    t.ok(n == 0, &format!("{level}: back: against the first frame {n} pixels differ (largest {m})"));
    // a material's own class (Renderer::set_material): its instances move
    let mut m2 = items[2].1;
    m2.alpha_cutoff = None;
    t.res(b.r.set_material(MaterialId(2), &m2), "set_material")?;
    t.ok(b.r.alpha_class(MaterialId(2)) == AlphaClass::Opaque && m2.alpha_class() == AlphaClass::Opaque, &format!("{level}: Renderer::alpha_class follows the row"));
    items[2].1 = m2;
    judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: material 2 made opaque (Renderer::set_material): its instance moves"))?;
    m2.alpha_cutoff = Some(0.5);
    t.res(b.r.set_material(MaterialId(2), &m2), "set_material")?;
    items[2].1 = m2;
    let img3 = judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: and MASK again"))?;
    let (n, m) = diff(&img0, &img3);
    t.ok(n == 0, &format!("{level}: MASK again: against the first frame {n} pixels differ (largest {m})"));
    // the MASK instances to the other side (set_transform) and back
    for k in [1usize, 2, 4, 5] {
        items[k].0.scale[2] = -items[k].0.scale[2];
        let sp = items[k].0;
        t.res(b.s.set_transform(b.ids[k], &Transform { pos: sp.pos, rot: sp.rot, scale: sp.scale }), "set_transform")?;
    }
    let img4 = judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: the MASK instances turned to the other side (set_transform)"))?;
    // a class change after prepare: the frame shows what prepare sent, the next prepare takes the change
    t.res(b.s.prepare(&mut b.r), "prepare")?;
    t.res(b.s.set_material(b.ids[0], m0), "set_material after prepare")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::CopySrc, after: Layout::CopySrc };
    let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true, before: Layout::Sampled, after: Layout::Sampled };
    let recorded = b.r.record(&mut e, &b.s, ca, Some(da));
    t.ok(recorded.is_ok(), &format!("{level}: a class changed after prepare: record takes the frame ({recorded:?})"));
    e.copy_texture_to_buffer(TextureRegion::mip(&off.color, 0), &off.rb, DataLayout { offset: 0, bytes_per_row: off.stride, rows_per_image: 0 });
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    b.s.submitted(id);
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let got = t.res(off.read(), "read")?;
    let (n, m) = diff(&img4, &got);
    t.ok(n == 0, &format!("{level}: that frame shows what prepare sent: {n} pixels differ (largest {m})"));
    items[0].1.alpha_cutoff = Some(0.5);
    judge(t, dev, &mut off, &geos, &cam, &mut b, &items, &format!("{level}: the next frame takes the change"))?;
    Some(())
}

// ---------------------------------------------------------------- detail levels with MASK materials

fn lods_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    use crate::lodtest::{LEVELS, geos as lod_geos, pick, read, size_of, specs};
    let geos = lod_geos();
    let colors = gallery_colors(12);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let px = windows();
    t.res(r.add_texture(&TextureData { width: 128, height: 128, rgba: &px, srgb: true, nearest: true }), "add_texture")?;
    // a third MASK (windows in the texture)
    let mats: Vec<Material> = colors.iter().enumerate().map(|(i, c)| if i % 3 == 0 { holed(c[0], c[1], c[2], true) } else { Material::color(c[0], c[1], c[2]) }).collect();
    for m in &mats {
        t.res(r.add_material(m), "add_material")?;
    }
    t.res(r.set_lods(MeshId(0), &LEVELS), "set_lods")?;
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = Camera::look_at([0.0, 1.0, 8.0], [0.0, 0.5, -100.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let list = specs(&r, &cam, 1500);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_sun(&sun_of());
    s.set_camera(&cam);
    for sp in &list {
        t.res(s.add(sp.instance()), "add")?;
    }
    // the pick per instance against the CPU: mesh, side and class (without the occlusion test: everything in the view)
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
    let (mut right, mut wrong, mut masked) = (0, 0, [0u32; 3]);
    for (i, sp) in list.iter().enumerate() {
        let (size, wc, wr) = size_of(&r, sp, &cam);
        let seen = rc.sphere_test(wc, wr, [0.0; 3]);
        if seen == cpu::Seen::Doubt {
            continue;
        }
        let mask = mats[sp.material as usize].alpha_cutoff.is_some();
        let want = if seen == cpu::Seen::Yes { pick(sp.mesh, size).map(|m| (m, mirrored(sp), mask)) } else { None };
        let have = listed[i].map(|l| (infos[l as usize].mesh.0, infos[l as usize].mirrored, is_mask(infos[l as usize].class)));
        if want == have { right += 1 } else { wrong += 1 }
        if let Some((m @ 0..=2, _, true)) = want {
            masked[m as usize] += 1;
        }
    }
    t.ok(wrong == 0 && masked.iter().all(|&n| n > 0),
         &format!("{level}: {} balls (a third MASK, a third mirrored), the pick (mesh, side, class) against the CPU: {right} right, {wrong} wrong; MASK in the view per level: {masked:?}", list.len()));
    // every list with instances has the lists of its levels, of its side and class, with room for them
    let mut missing = 0;
    let mut bases = 0;
    for base in infos.iter().filter(|l| l.mesh.0 == 0 && l.count > 0) {
        bases += 1;
        for lv in [1u32, 2] {
            if !infos.iter().any(|l| l.mesh.0 == lv && l.mirrored == base.mirrored && l.class == base.class && l.room >= base.count + l.count) {
                missing += 1;
            }
        }
    }
    t.ok(bases == 4 && missing == 0, &format!("{level}: {bases} lists of the ball with levels (2 sides x 2 classes): their levels' lists of the same side and class, room for them: {missing} missing"));
    // the image with the occlusion test against the ray caster with the CPU's pick
    s.set_occlusion(true);
    shot(t, dev, &mut off, &mut r, &mut s, "lods")?;
    let img = shot(t, dev, &mut off, &mut r, &mut s, "lods")?;
    let sun = sun_of();
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    rs.textures.push(RefTexture::new(128, &px, true));
    for sp in &list {
        let (size, _, _) = size_of(&r, sp, &cam);
        if let Some(m) = pick(sp.mesh, size) {
            rs.add_look(&geos[m as usize], &Spec { mesh: m, ..*sp }, look_of(&mats[sp.material as usize]));
        }
    }
    // (within 2: at a window's edge on a ball a few pixels across the colour changes by 2 within 1/64 of a pixel; with 1
    // one edge pixel fails by 2, solid pixels stay within 1)
    let c = cpu::compare_tol(&rs, &rc, w, h, CLEAR8, &img, None, 2);
    t.ok(c.fine(), &format!("{level}: detail levels with MASK, the occlusion test, second frame, against the ray caster with the same pick (within 2): {}", c.text()));
    explain(&rs, &cam, w, h, &img, &c);
    Some(())
}

// ---------------------------------------------------------------- MASK's holes in the shadows

fn shadows_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (geos, pics, mut items) = shadow_scene([0.0; 3]);
    let texs = || pics.iter().map(|(n, px, srgb)| Tex::exact(*n, px.clone(), *srgb)).collect::<Vec<Tex>>();
    let cam = shadow_camera([0.0; 3]);
    let mut off = t.res(Off::new(dev, SHADOW_W, SHADOW_H, Format::Rgba8Unorm), "target")?;
    let built = |t: &mut T, items: &[(Spec, Material)]| -> Option<Built> {
        let mut b = build_with(t, dev, &shadow_sun(), &geos, &texs(), items, &cam)?;
        t.res(b.s.set_shadows(Some(&SHADOWS)), "set_shadows")?;
        b.rs.shadows = true;
        Some(b)
    };
    let rays = |t: &mut T, rs: &RefScene, img: &[u8], what: &str| {
        let c = cpu::compare_shadows(rs, &ref_camera(&cam), SHADOW_W, SHADOW_H, CLEAR8, img, 1);
        let fine = c.bad == 0 && c.edges_bad == 0 && (c.inside as f64) <= 0.0005 * c.on_objects as f64;
        t.ok(fine, &format!("{what}: {}", c.text()));
        c
    };
    let mut b = built(t, &items)?;
    let with_holes = shot(t, dev, &mut off, &mut b.r, &mut b.s, "shadows")?;
    let base = rays(t, &b.rs, &with_holes, &format!("{level}: the shadow scene (a MASK plate) against exact shadow rays"));
    // the plate (the last item) to an opaque material of the same texture: full shadows, like the rays of an opaque plate
    let k = items.len() - 1;
    let mut solid = items[k].1;
    solid.alpha_cutoff = None;
    let mo = t.res(b.r.add_material(&solid), "add_material")?;
    t.res(b.s.set_material(b.ids[k], mo), "set_material")?;
    items[k].1 = solid;
    let full = shot(t, dev, &mut off, &mut b.r, &mut b.s, "plate opaque")?;
    let mut fresh = built(t, &items)?;
    let want = shot(t, dev, &mut off, &mut fresh.r, &mut fresh.s, "plate opaque, anew")?;
    let (n, m) = diff(&full, &want);
    let c = rays(t, &fresh.rs, &full, &format!("{level}: the plate switched to opaque (Scene::set_material) against the rays of an opaque plate"));
    let (d, _) = diff(&full, &with_holes);
    t.ok(n == 0 && d > 0, &format!("{level}: and against the scene built anew {n} pixels differ (largest {m}); {d} differ from the plate with holes (rays: {} on the other side, with holes {})",
                                   c.at_edges + c.inside, base.at_edges + base.inside));
    // opaque casters only: the same image with and without set_shadow_mask (they never take fs_shadow)
    b.r.set_shadow_mask(false);
    let no_fs = shot(t, dev, &mut off, &mut b.r, &mut b.s, "no fs_shadow")?;
    b.r.set_shadow_mask(true);
    let (n, m) = diff(&full, &no_fs);
    t.ok(n == 0, &format!("{level}: only opaque casters: with and without set_shadow_mask {n} pixels differ (largest {m})"));
    // back to MASK: the holes again
    t.res(b.s.set_material(b.ids[k], MaterialId(k as u32)), "set_material")?;
    let again = shot(t, dev, &mut off, &mut b.r, &mut b.s, "MASK again")?;
    let (n, m) = diff(&with_holes, &again);
    t.ok(n == 0, &format!("{level}: the plate MASK again: against the first frame {n} pixels differ (largest {m})"));
    Some(())
}

// ---------------------------------------------------------------- the occlusion test behind a MASK wall

fn occlusion_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    // a wall of 32 x 16 with windows of 4 x 4 (the texture twice across), small cubes behind it and before it
    let geos = [plate_uv([0.0, 2.0], [0.0, 1.0]), cube()];
    let colors = gallery_colors(12);
    let mut mats = vec![holed(0.7, 0.7, 0.75, true)];
    mats.extend(colors.iter().enumerate().map(|(i, c)| holed(c[0], c[1], c[2], i % 3 == 0)));
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    let px = windows();
    t.res(r.add_texture(&TextureData { width: 128, height: 128, rgba: &px, srgb: true, nearest: true }), "add_texture")?;
    for m in &mats {
        t.res(r.add_material(m), "add_material")?;
    }
    let mut specs = vec![Spec { mesh: 0, material: 0, pos: [0.0, 0.0, -10.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [16.0, 8.0, 1.0] }];
    let mut rng = Rng(2026);
    for k in 0..1600 {
        let z = if k < 1500 { rng.range(-40.0, -12.0) } else { rng.range(-7.0, 4.0) };
        let s = rng.range(0.2, 0.45) as f32;
        let mut scale = [s; 3];
        if rng.below(3) == 0 {
            scale[rng.below(3) as usize] *= -1.0;
        }
        let q = |r: &mut Rng, a: f64, b: f64| (r.range(a, b) * 1024.0) as i64 as f64 / 1024.0;
        let axis = cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0) + 0.01]);
        specs.push(Spec { mesh: 1, material: 1 + rng.below(12), pos: [q(&mut rng, -26.0, 26.0), q(&mut rng, -13.0, 13.0), z], rot: quat(axis, rng.range(0.0, 6.0)), scale });
    }
    let (mut a, mut b) = (t.res(Scene::new(&r), "Scene::new")?, t.res(Scene::new(&r), "Scene::new")?);
    b.set_occlusion(false);
    for s in [&mut a, &mut b] {
        s.set_sun(&sun_of());
        for sp in &specs {
            t.res(s.add(sp.instance()), "add")?;
        }
    }
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let (mut same, mut late_mask, mut late_opaque) = (0, 0u32, 0u32);
    for f in 0..6 {
        let x = -3.0 + f as f64 * 1.2;
        let cam = Camera::look_at([x, 0.5, 12.0], [x * 0.5, 0.0, -20.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
        a.set_camera(&cam);
        b.set_camera(&cam);
        let ia = shot(t, dev, &mut off, &mut r, &mut a, "with occlusion")?;
        let (slots, early, late) = crate::occl::drawn(t, dev, &a)?;
        for &slot in &slots[early as usize..] {
            match specs.get(slot as usize).map(|sp| mats[sp.material as usize].alpha_cutoff.is_some()) {
                Some(true) => late_mask += 1,
                Some(false) => late_opaque += 1,
                None => {}
            }
        }
        let ib = shot(t, dev, &mut off, &mut r, &mut b, "without")?;
        let (all, _, _) = crate::occl::drawn(t, dev, &b)?;
        let (n, _) = diff(&ia, &ib);
        same += (n == 0) as u32;
        if f == 3 {
            let sun = sun_of();
            let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
            rs.textures.push(RefTexture::new(128, &px, true));
            for sp in &specs {
                rs.add_look(&geos[sp.mesh as usize], sp, look_of(&mats[sp.material as usize]));
            }
            let c = cpu::compare(&rs, &ref_camera(&cam), w, h, CLEAR8, &ia, None);
            t.ok(c.fine(), &format!("{level}: behind a MASK wall with the occlusion test, frame 4, against the ray caster: {}", c.text()));
            t.ok((early + late) < all.len() as u32, &format!("{level}: the wall hides: {} of {} in the view drawn ({late} by phase 2)", early + late, all.len()));
        }
    }
    t.ok(same == 6 && late_mask > 0 && late_opaque > 0,
         &format!("{level}: six frames of a flight behind a MASK wall: {same} of 6 the same with and without the occlusion test; phase 2 drew {late_opaque} opaque and {late_mask} MASK instances"));
    Some(())
}
