// g7ftest -- render3dtest --test sides, blend, remove (G7f; docs/RENDER3D.md "Doppelseitige Materialien", "Durchsichtiges",
// "Entfernen"), on Voll and on a device limited to Basis.
//
//   sides   double-sided materials against the ray caster (plates seen from behind, MASK, mirrored, a normal map, open
//           boxes seen into; single-sided ones culled), their lists, a material switched between the two, with and without
//           the occlusion test; their back faces lit by the sun against exact shadow rays
//   blend   blended instances against the ray caster laying them back to front (plates out of their order, a double-sided
//           cube, a mirrored one, a texture's alpha, over the clear colour, behind the wall), the sorted list read back,
//           with and without the occlusion test, the depth buffer as without them, no shadows, a switch to opaque and back;
//           600 instances (the sort over two tiles) against the ray caster, 3000 (four tiles and the steps between them)
//           sorted; detail levels of blended balls
//   remove  meshes, materials and textures removed: ids invalid after, places and ranges used again, what is refused (a
//           scene's instances, a dropped scene, detail levels, a material's texture), a removal between prepare and record,
//           a slot's generations spent; churn against the same contents in a fresh renderer; a random sequence of 400 steps
//           against a model, its images against fresh renderers built from the model

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{AlphaClass, Camera, Instance, InstanceId, Lod, Material, MaterialDesc, MaterialId, MeshId, Renderer, Scene, Sun, TextureData, TextureId, TextureRef,
               Transform};

use crate::common::*;
use crate::cpu::{self, RefScene, RefTexture};
use crate::tests::{T, on_levels, out, read_depth, ref_camera, shot};
use crate::tex::{Built, look_of_desc};

/// a described scene (common.rs) in a fresh renderer (linear, Rgba8Unorm) and its reference
fn build_desc(t: &mut T, dev: &Device, sun: &Sun, d: &Described, cam: &Camera) -> Option<Built> {
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let (s, ids) = t.res(described_in(&mut r, d, cam, sun), "the scene")?;
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    for (n, px, srgb) in &d.1 {
        rs.textures.push(RefTexture::new(*n, px, *srgb));
    }
    for (spec, m) in &d.2 {
        rs.add_look(&d.0[spec.mesh as usize], spec, look_of_desc(m));
    }
    Some(Built { r, s, rs, ids })
}

/// the scene's frame against the same items built anew (0 pixels apart) and against that scene's ray caster
#[allow(clippy::too_many_arguments)]
fn judge(t: &mut T, dev: &Device, off: &mut Off, b: &mut Built, d: &Described, cam: &Camera, tol: u8, what: &str) -> Option<Vec<u8>> {
    let img = shot(t, dev, off, &mut b.r, &mut b.s, what)?;
    let mut fresh = build_desc(t, dev, &Sun::DEFAULT, d, cam)?;
    let want = shot(t, dev, off, &mut fresh.r, &mut fresh.s, what)?;
    let (n, m) = diff(&img, &want);
    let c = cpu::compare_tol(&fresh.rs, &ref_camera(cam), off.w, off.h, CLEAR8, &img, None, tol);
    t.ok(n == 0 && c.fine(), &format!("{what}: against the scene built anew {n} pixels differ (largest {m}); against the ray caster {}", c.text()));
    Some(img)
}

// ---------------------------------------------------------------- double-sided materials

pub fn sides(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = sides_on(t, dev, level);
        let _ = sides_shadows_on(t, dev, level);
    });
}

fn sides_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = sides_camera(w as f64 / h as f64);
    let mut d = sides_scene();
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut b = build_desc(t, dev, &Sun::DEFAULT, &d, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "sides")?;
    let c = cpu::compare(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None);
    t.ok(c.fine() && c.objects_seen == 7,
         &format!("{level}: plates from behind (single-sided culled; double-sided, MASK, mirrored, a normal map), open boxes, against the ray caster: {}", c.text()));
    // the lists: double-sided instances in lists of their own (plate as it is: two; plate MASK; plate mirrored; the cup)
    let (lists, kinds) = (b.s.lists(), b.s.list_kinds());
    let mut have: Vec<(u32, bool, bool, bool, u32)> = lists.iter().zip(kinds.iter()).filter(|(l, _)| l.count > 0)
        .map(|(l, k)| (l.mesh.0, l.mirrored, l.class == AlphaClass::Mask, k.double_sided, l.count)).collect();
    have.sort();
    let want = vec![(0, false, false, false, 2), (0, false, false, true, 2), (0, false, true, true, 1), (0, true, false, true, 1), (1, false, false, false, 1),
                    (1, false, false, true, 1)];
    t.ok(have == want && b.r.double_sided(MaterialId(2)) && !b.r.double_sided(MaterialId(1)),
         &format!("{level}: lists (mesh, mirrored, MASK, double-sided, instances) {have:?}"));
    // the green plate single-sided (it disappears: its back is to the camera), and double-sided again
    let mut m = d.2[2].1;
    m.double_sided = false;
    t.res(b.r.set_material_desc(MaterialId(2), &m), "set_material_desc")?;
    d.2[2].1 = m;
    let gone = judge(t, dev, &mut off, &mut b, &d, &cam, 1, &format!("{level}: the green plate made single-sided (set_material_desc)"))?;
    let (n, _) = diff(&img, &gone);
    m.double_sided = true;
    t.res(b.r.set_material_desc(MaterialId(2), &m), "set_material_desc")?;
    d.2[2].1 = m;
    let back = shot(t, dev, &mut off, &mut b.r, &mut b.s, "double again")?;
    let (n2, m2) = diff(&img, &back);
    t.ok(n > 0 && n2 == 0, &format!("{level}: single-sided it is gone ({n} pixels differ), double-sided again: {n2} pixels differ (largest {m2})"));
    // set_material keeps the material double-sided
    let plain = d.2[2].1.material;
    t.res(b.r.set_material(MaterialId(2), &plain), "set_material")?;
    let kept = shot(t, dev, &mut off, &mut b.r, &mut b.s, "set_material")?;
    let (n3, _) = diff(&img, &kept);
    t.ok(n3 == 0 && b.r.double_sided(MaterialId(2)), &format!("{level}: set_material keeps it double-sided: {n3} pixels differ"));
    // the occlusion test draws them in both phases alike
    b.s.set_occlusion(false);
    let flat = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without the occlusion test")?;
    b.s.set_occlusion(true);
    let (n4, m4) = diff(&img, &flat);
    t.ok(n4 == 0, &format!("{level}: without the occlusion test: {n4} pixels differ (largest {m4})"));
    Some(())
}

/// double-sided plates whose back faces the sun lights, over a ground, with the sun's shadows: against exact shadow rays
/// (the bias of a back face goes along its turned normal: else it would shadow itself)
fn sides_shadows_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (SHADOW_W, SHADOW_H);
    let cam = Camera::look_at([0.5, 3.0, 11.0], [0.0, -0.5, 0.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let mut d = sides_scene();
    // (no back plate: a ground under them instead)
    d.2[0] = (Spec { mesh: 0, material: 0, pos: [0.0, -3.0, -2.0], rot: quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2), scale: [12.0, 9.0, 1.0] },
              MaterialDesc::new(Material { roughness: 0.9, ..Material::color(0.75, 0.75, 0.7) }));
    // (the normal map's plate without it: seen this steeply a pixel's centre can lie on a nearest texel's edge, which this
    // comparison has no tolerance for; the plain image above checks the map)
    d.2[7].1.material.normal_texture = None;
    let sun = Sun { direction: [0.3, 0.6, 0.7], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.3; 3] };
    let mut b = build_desc(t, dev, &sun, &d, &cam)?;
    t.res(b.s.set_shadows(Some(&render3d::Shadows { cascades: 2, size: 2048, distance: 30.0, ..render3d::Shadows::DEFAULT })), "set_shadows")?;
    b.rs.shadows = true;
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "sides with shadows")?;
    let c = cpu::compare_shadows(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, 1);
    let fine = c.bad == 0 && c.edges_bad == 0 && (c.inside as f64) <= 0.0005 * c.on_objects as f64;
    t.ok(fine, &format!("{level}: back faces lit by the sun, with its shadows, against exact shadow rays: {}", c.text()));
    if !fine {
        let rc = ref_camera(&cam);
        for (x, y) in c.first.map(|f| (f.0, f.1)).into_iter().chain(c.first_inside.map(|f| (f.0, f.1))) {
            for (dx, dy) in [(0i32, 0i32), (-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (px, py) = ((x as i32 + dx) as u32, (y as i32 + dy) as u32);
                let hit = b.rs.trace(rc.eye, rc.ray(px as f64 + 0.5, py as f64 + 0.5, w, h), rc.near, rc.far);
                out(&format!("    {px}, {py}: got {:?}, ray {:?}", crate::tests::pixel_of(&img, w, px, py), hit.map(|v| (v.inst, v.tri, v.back, v.vis, v.rgb))));
            }
        }
    }
    Some(())
}

// ---------------------------------------------------------------- blended instances

pub fn blend(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = blend_on(t, dev, level);
        let _ = blend_many_on(t, dev, level);
        let _ = blend_lods_on(t, dev, level);
        let _ = gltf_on(t, dev, level);
    });
}

/// glTF's BLEND and doubleSided through gltf::upload: the classes, no notes, the image against the ray caster built from the
/// description (not from the import)
fn gltf_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    use crate::gltfgen::{Enc, Form, GMat, GNode, GPrim, GScene};
    let enc = Enc { idx: Some(5123), uv: Some(5126), normals: true, interleaved: false, sparse: None };
    let mat = |factor: [f32; 4], blend: bool, double: bool| GMat { factor, metallic: 0.0, roughness: 0.6, emissive: [0.0; 3], tex: [None; 5], mask: None, blend, double };
    let mats = vec![mat([0.6, 0.6, 0.65, 1.0], false, false), mat([0.9, 0.2, 0.1, 0.5], true, false), mat([0.2, 0.9, 0.3, 1.0], false, true),
                    mat([0.9, 0.7, 0.2, 0.4], true, true)];
    let geos = [plate(), cube()];
    let meshes = vec![vec![GPrim { geo: plate(), enc, material: Some(0) }], vec![GPrim { geo: plate(), enc, material: Some(1) }],
                      vec![GPrim { geo: plate(), enc, material: Some(2) }], vec![GPrim { geo: cube(), enc, material: Some(3) }]];
    let geo_of = [0usize, 0, 0, 1];
    let node = |t: [f64; 3], r: [f64; 4], s: [f64; 3], mesh: usize| GNode { t, r, s, matrix: false, mesh: Some(mesh), children: Vec::new() };
    let q = |axis: [f64; 3], a: f64| { let x = quat(axis, a); [x[0] as f64, x[1] as f64, x[2] as f64, x[3] as f64] };
    let nodes = vec![node([0.0, 0.0, -5.0], [0.0, 0.0, 0.0, 1.0], [7.0, 4.0, 1.0], 0), node([-1.5, 0.3, 0.0], [0.0, 0.0, 0.0, 1.0], [1.6, 1.2, 1.0], 1),
                     node([1.8, 0.5, -1.0], q([0.0, 1.0, 0.0], 3.0), [1.2; 3], 2), node([0.2, -0.6, -2.0], q(cpu::norm([0.3, 1.0, 0.2]), 0.7), [0.9; 3], 3)];
    let gs = GScene { meshes, mats: mats.clone(), texs: Vec::new(), images: Vec::new(), nodes, scenes: vec![vec![0, 1, 2, 3]], scene: 0 };
    let (glb, _) = gs.write(Form::Glb);
    let Ok(doc) = gltf::load(&glb, &mut |_| None, &gltf::Limits::DEFAULT) else {
        t.ok(false, &format!("{level}: the glTF file loads"));
        return None;
    };
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let imp = t.res(render3d::gltf::upload(&mut r, &doc, None), "gltf::upload")?;
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = Camera::look_at([0.5, 1.0, 8.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    for i in &imp.instances {
        t.res(s.add(*i), "add")?;
    }
    let classes: Vec<(AlphaClass, bool)> = imp.materials.iter().map(|&m| (r.alpha_class(m), r.double_sided(m))).collect();
    let want = vec![(AlphaClass::Opaque, false), (AlphaClass::Blend, false), (AlphaClass::Opaque, true), (AlphaClass::Blend, true)];
    let quiet = !imp.notes.iter().any(|n| n.contains("BLEND") || n.contains("double"));
    t.ok(classes == want && quiet, &format!("{level}: glTF BLEND and doubleSided: classes {classes:?}; notes {:?}", imp.notes));
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let img = shot(t, dev, &mut off, &mut r, &mut s, "glTF")?;
    let sun = Sun::DEFAULT;
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    let worlds = gs.worlds();
    for (k, n) in gs.nodes.iter().enumerate() {
        let m = n.mesh?;
        let g = &mats[m];
        let look = cpu::Look { factor: g.factor.map(|v| v as f64), roughness: g.roughness as f64, metallic: 0.0, double_sided: g.double, blend: g.blend, ..cpu::Look::DULL };
        rs.add_matrix(&geos[geo_of[m]], &worlds[k], look);
    }
    let c = cpu::compare_tol(&rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 2);
    t.ok(c.fine() && s.blend_drawn() && s.list_kinds().iter().any(|k| k.double_sided),
         &format!("{level}: the file drawn against the ray caster from the description (within 2): {}", c.text()));
    Some(())
}

/// the blended instances' list after a frame: (vertices per instance, instances, the entries)
fn read_blend(t: &mut T, dev: &Device, s: &Scene) -> Option<(u32, u32, Vec<[u32; 4]>)> {
    let b = s.blend_buffer();
    let mut w = vec![0u32; b.size() as usize / 4];
    t.res(dev.queue().read_buffer(b, 0, &mut w), "read the blended list")?;
    let n = w[1].min((w.len() as u32 - 4) / 4);
    let items = (0..n as usize).map(|k| [w[4 + k * 4], w[5 + k * 4], w[6 + k * 4], w[7 + k * 4]]).collect();
    Some((w[0], w[1], items))
}

/// the entries are sorted: ascending key, then slot
fn sorted(items: &[[u32; 4]]) -> bool {
    items.windows(2).all(|p| (p[0][0], p[0][1]) < (p[1][0], p[1][1]))
}

/// the slots of the blended items whose sphere the camera sees (not within a hair of a face), farthest first by the
/// distance of the sphere's centre (f64)
fn blend_in_view(r: &Renderer, d: &Described, ids: &[InstanceId], cam: &Camera) -> (Vec<u32>, u32) {
    let rc = ref_camera(cam);
    let mut v: Vec<(f64, u32)> = Vec::new();
    let mut doubt = 0;
    for (k, (sp, m)) in d.2.iter().enumerate() {
        if !m.blend {
            continue;
        }
        let (c, rad) = r.cull_sphere(MeshId(sp.mesh)).unwrap_or(([0.0; 3], 1.0));
        let (wc, wr) = cpu::world_sphere(c, rad, sp.pos, sp.rot, sp.scale);
        match rc.sphere_test(wc, wr, [0.0; 3]) {
            cpu::Seen::Yes => {
                let e = [wc[0] - cam.eye[0], wc[1] - cam.eye[1], wc[2] - cam.eye[2]];
                v.push((cpu::dot(e, e), ids[k].slot));
            }
            cpu::Seen::Doubt => doubt += 1,
            cpu::Seen::No => {}
        }
    }
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(core::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
    (v.into_iter().map(|x| x.1).collect(), doubt)
}

fn blend_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = blend_camera(w as f64 / h as f64);
    let mut d = blend_scene();
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut b = build_desc(t, dev, &Sun::DEFAULT, &d, &cam)?;
    let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "blend")?;
    let c = cpu::compare_tol(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 2);
    t.ok(c.fine(), &format!("{level}: blended plates out of their order, a double-sided cube, a mirrored one, a texture's alpha, over the clear colour, \
                            behind the wall, against the ray caster laying them back to front (within 2): {}", c.text()));
    // the sorted list: with the occlusion test without the plate behind the wall (index 7)
    let (all, doubt) = blend_in_view(&b.r, &d, &b.ids, &cam);
    let hidden = b.ids[7].slot;
    let want: Vec<u32> = all.iter().copied().filter(|&s| s != hidden).collect();
    let (k, n, items) = read_blend(t, dev, &b.s)?;
    let got: Vec<u32> = items.iter().map(|e| e[1]).collect();
    t.ok(k == 36 && got == want && sorted(&items) && doubt == 0 && all.contains(&hidden),
         &format!("{level}: the blended list with the occlusion test: {n} instances, {k} vertices each (the cube's 36), back to front {got:?}, expected {want:?} \
                   (the plate behind the wall left out)"));
    let flags: Vec<u32> = items.iter().map(|e| e[3]).collect();
    let mirrored_double = items.iter().any(|e| e[3] & 4 != 0) && items.iter().any(|e| e[3] & 2 != 0);
    t.ok(mirrored_double, &format!("{level}: the entries' flags (2 double-sided, 4 mirrored): {flags:?}"));
    // without it: the same image, every blended instance in view
    b.s.set_occlusion(false);
    let flat = shot(t, dev, &mut off, &mut b.r, &mut b.s, "without the occlusion test")?;
    let (_, n2, items2) = read_blend(t, dev, &b.s)?;
    let got2: Vec<u32> = items2.iter().map(|e| e[1]).collect();
    b.s.set_occlusion(true);
    let (nd, md) = diff(&img, &flat);
    t.ok(nd == 0 && got2 == all && sorted(&items2), &format!("{level}: without the occlusion test: {nd} pixels differ (largest {md}); {n2} blended instances, back to front"));
    // the depth buffer as without the blended instances: they write none
    let depth = read_depth(t, dev, &off)?;
    let opaque: Described = (d.0.clone(), d.1.clone(), d.2.iter().filter(|x| !x.1.blend).cloned().collect());
    let mut o = build_desc(t, dev, &Sun::DEFAULT, &opaque, &cam)?;
    let mut off2 = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let without = shot(t, dev, &mut off2, &mut o.r, &mut o.s, "without them")?;
    let depth2 = read_depth(t, dev, &off2)?;
    let same = depth.iter().zip(depth2.iter()).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
    let (nw, _) = diff(&img, &without);
    t.ok(same == 0 && nw > 1000, &format!("{level}: the depth buffer as without the blended instances: {same} of {} values differ; the image {nw} pixels", depth.len()));
    // the green plate opaque (set_material_desc) and blended again
    let mut m = d.2[9].1;
    m.blend = false;
    t.res(b.r.set_material_desc(MaterialId(9), &m), "set_material_desc")?;
    t.ok(b.r.alpha_class(MaterialId(9)) == AlphaClass::Opaque, &format!("{level}: Renderer::alpha_class follows"));
    d.2[9].1 = m;
    judge(t, dev, &mut off, &mut b, &d, &cam, 2, &format!("{level}: the green plate made opaque (set_material_desc)"))?;
    m.blend = true;
    t.res(b.r.set_material_desc(MaterialId(9), &m), "set_material_desc")?;
    d.2[9].1 = m;
    let again = shot(t, dev, &mut off, &mut b.r, &mut b.s, "blended again")?;
    let (na, ma) = diff(&img, &again);
    t.ok(na == 0 && b.r.alpha_class(MaterialId(9)) == AlphaClass::Blend, &format!("{level}: blended again: {na} pixels differ (largest {ma})"));
    // what is refused: blended with an alpha_cutoff
    let mut bad = MaterialDesc::new(Material { alpha_cutoff: Some(0.5), ..Material::color(1.0, 1.0, 1.0) });
    bad.blend = true;
    t.ok(matches!(b.r.add_material_desc(&bad), Err(GpuError::Invalid(_))), &format!("{level}: blended with an alpha_cutoff: Invalid"));
    // no shadows: with the sun's shadows, every blended item at alpha 0 draws the image of the opaque items alone
    let shadows = render3d::Shadows { cascades: 2, size: 1024, distance: 40.0, ..render3d::Shadows::DEFAULT };
    let mut clear = d.clone();
    for x in clear.2.iter_mut().filter(|x| x.1.blend) {
        x.1.material.base_color[3] = 0.0;
        // (and a texture's alpha too)
        x.1.material.base_color_texture = None;
    }
    let mut c0 = build_desc(t, dev, &Sun::DEFAULT, &clear, &cam)?;
    t.res(c0.s.set_shadows(Some(&shadows)), "set_shadows")?;
    let zero = shot(t, dev, &mut off, &mut c0.r, &mut c0.s, "alpha 0, shadows")?;
    let mut o2 = build_desc(t, dev, &Sun::DEFAULT, &opaque, &cam)?;
    t.res(o2.s.set_shadows(Some(&shadows)), "set_shadows")?;
    let none = shot(t, dev, &mut off, &mut o2.r, &mut o2.s, "opaque alone, shadows")?;
    let mut c1 = build_desc(t, dev, &Sun::DEFAULT, &d, &cam)?;
    t.res(c1.s.set_shadows(Some(&shadows)), "set_shadows")?;
    let seen = shot(t, dev, &mut off, &mut c1.r, &mut c1.s, "blended, shadows")?;
    let (nz, mz) = diff(&zero, &none);
    let (ns, _) = diff(&seen, &none);
    t.ok(nz == 0 && ns > 1000, &format!("{level}: with the sun's shadows, blended items at alpha 0 against the opaque items alone: {nz} pixels differ (largest {mz}): \
                                        they cast no shadow; at their alpha {ns} pixels differ"));
    Some(())
}

/// many blended instances: 600 against the ray caster (the sort over two tiles), 3000 sorted, spread wider so that about
/// half lie outside the view (room for 4096: the sort's work follows the count, the passes beyond it end at once)
fn blend_many_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = Camera::look_at([0.0, 0.0, 14.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    for (n, image) in [(600u32, true), (3000, false)] {
        let mut rng = Rng(5150 + n);
        let mut items = vec![(Spec { mesh: 0, material: 0, pos: [0.0, 0.0, -12.0], rot: [0.0, 0.0, 0.0, 1.0], scale: [14.0, 8.0, 1.0] },
                              MaterialDesc::new(Material::color(0.6, 0.6, 0.65)))];
        let q = |r: &mut Rng, a: f64, b: f64| (r.range(a, b) * 1024.0) as i64 as f64 / 1024.0;
        for _ in 0..n {
            let mut m = MaterialDesc::new(Material { base_color: [rng.range(0.1, 1.0) as f32, rng.range(0.1, 1.0) as f32, rng.range(0.1, 1.0) as f32, rng.range(0.25, 0.75) as f32],
                                                     roughness: 0.6, ..Material::color(1.0, 1.0, 1.0) });
            m.blend = true;
            m.double_sided = rng.below(3) == 0;
            let mesh = rng.below(2);
            let s = rng.range(0.15, 0.5) as f32;
            let mut scale = [s, s, s];
            if rng.below(4) == 0 {
                scale[0] = -s;
            }
            let axis = cpu::norm([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0) + 0.01]);
            let wide = if image { 1.0 } else { 6.0 };
            items.push((Spec { mesh, material: 0, pos: [q(&mut rng, -9.0 * wide, 9.0 * wide), q(&mut rng, -5.0, 5.0), q(&mut rng, -10.0, 4.0)], rot: quat(axis, rng.range(0.0, 6.0)),
                               scale }, m));
        }
        let d: Described = (vec![plate(), cube()], Vec::new(), items);
        let mut b = build_desc(t, dev, &Sun::DEFAULT, &d, &cam)?;
        b.s.set_occlusion(false);
        let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
        let img = shot(t, dev, &mut off, &mut b.r, &mut b.s, "many blended")?;
        let (want, doubt) = blend_in_view(&b.r, &d, &b.ids, &cam);
        let (_, count, items) = read_blend(t, dev, &b.s)?;
        let got: Vec<u32> = items.iter().map(|e| e[1]).collect();
        // (an instance within a hair of a face may or may not be there)
        let fine = sorted(&items) && (got == want || doubt > 0 && want.iter().all(|s| got.contains(s)));
        t.ok(fine, &format!("{level}: {n} blended instances: {count} in the list (CPU {} + {doubt} at a face; room {}), sorted back to front: {}", want.len(),
                            (b.s.blend_buffer().size() - 16) / 16, sorted(&items)));
        if image {
            let c = cpu::compare_tol(&b.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 3);
            t.ok(c.fine(), &format!("{level}: {n} blended instances against the ray caster (within 3: every layer rounds to 8 bits): {}", c.text()));
        }
    }
    Some(())
}

/// blended balls with detail levels: each entry's draw record is the list of the mesh the CPU picks (and nothing of
/// those it leaves out)
fn blend_lods_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    use crate::lodtest::{LEVELS, geos as lod_geos, pick, size_of, specs};
    let geos = lod_geos();
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    for g in &geos {
        t.res(r.add_mesh(&g.data()), "add_mesh")?;
    }
    for c in &gallery_colors(12) {
        let mut m = MaterialDesc::new(Material { base_color: [c[0], c[1], c[2], 0.5], ..Material::color(1.0, 1.0, 1.0) });
        m.blend = true;
        t.res(r.add_material_desc(&m), "add_material_desc")?;
    }
    t.res(r.set_lods(MeshId(0), &LEVELS), "set_lods")?;
    let (w, h) = (GALLERY_W, GALLERY_H);
    let cam = Camera::look_at([0.0, 1.0, 8.0], [0.0, 0.5, -100.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let list = specs(&r, &cam, 1500);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    s.set_occlusion(false);
    for sp in &list {
        t.res(s.add(sp.instance()), "add")?;
    }
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    shot(t, dev, &mut off, &mut r, &mut s, "blended levels")?;
    let (_, _, items) = read_blend(t, dev, &s)?;
    let lists = s.lists();
    let mut by_slot = vec![None; list.len()];
    for e in &items {
        if let (Some(x), Some(l)) = (by_slot.get_mut(e[1] as usize), lists.iter().find(|l| l.record == e[2])) {
            *x = Some((l.mesh.0, l.mirrored));
        }
    }
    let rc = ref_camera(&cam);
    let (mut right, mut wrong, mut seen) = (0, 0, [0u32; 4]);
    for (i, sp) in list.iter().enumerate() {
        let (size, wc, wr) = size_of(&r, sp, &cam);
        let v = rc.sphere_test(wc, wr, [0.0; 3]);
        if v == cpu::Seen::Doubt {
            continue;
        }
        let mirrored = (sp.scale[0] as f64) * (sp.scale[1] as f64) * (sp.scale[2] as f64) < 0.0;
        let want = if v == cpu::Seen::Yes { pick(sp.mesh, size).map(|m| (m, mirrored)) } else { None };
        if want == by_slot[i] {
            right += 1;
            if let Some((m, _)) = want {
                seen[m as usize] += 1;
            }
        } else {
            wrong += 1;
        }
    }
    t.ok(wrong == 0 && seen.iter().all(|&n| n > 0) && sorted(&items),
         &format!("{level}: 1500 blended balls with detail levels: the pick (mesh, side) per entry against the CPU: {right} right, {wrong} wrong; per mesh {seen:?}"));
    Some(())
}

// ---------------------------------------------------------------- removing meshes, materials, textures

pub fn remove(t: &mut T) {
    on_levels(t, &mut |t, dev, level| {
        let _ = grown_records(t, dev, level);
        let _ = remove_basics(t, dev, level);
        let _ = churn_on(t, dev, level);
        let _ = random_on(t, dev, level);
    });
}

fn inv<V>(r: gpu::Result<V>) -> bool {
    matches!(r, Err(GpuError::Invalid(_)))
}

/// (a bug found by the random sequence, older than G7f) detail levels make list_first grow first (9 lists, 8 levels), then a
/// 17th list makes the draw records grow alone: the culling must count into the new ones
fn grown_records(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let cam = Camera::look_at([0.0, 0.0, 16.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, 16.0 / 9.0, 0.1, f64::INFINITY);
    let build = |t: &mut T, lists: u32| -> Option<(Renderer, Scene)> {
        let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
        for k in 0..20u32 {
            t.res(r.add_mesh(&prism(3 + k, 0.5).data()), "add_mesh")?;
        }
        let m = t.res(r.add_material(&Material::color(0.8, 0.5, 0.3)), "add_material")?;
        let levels: Vec<Lod> = (1..=8u32).map(|k| Lod { mesh: Some(MeshId(k)), below: 0.4 / k as f32 }).collect();
        t.res(r.set_lods(MeshId(0), &levels), "set_lods")?;
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        s.set_camera(&cam);
        for k in 0..lists {
            let mesh = if k == 0 { 0 } else { 8 + k };
            t.res(s.add(Instance { mesh: MeshId(mesh), material: m, transform: Transform::at([(k % 5) as f64 * 2.5 - 5.0, (k / 5) as f64 * 2.5 - 2.5, 0.0]) }), "add")?;
        }
        Some((r, s))
    };
    let mut off = t.res(Off::new(dev, 160, 90, Format::Rgba8Unorm), "target")?;
    // a frame with 9 lists (the base and its levels' lists), then 9 more lists
    let (mut r, mut s) = build(t, 1)?;
    shot(t, dev, &mut off, &mut r, &mut s, "9 lists")?;
    let m = MaterialId(0);
    for k in 1..10u32 {
        t.res(s.add(Instance { mesh: MeshId(8 + k), material: m, transform: Transform::at([(k % 5) as f64 * 2.5 - 5.0, (k / 5) as f64 * 2.5 - 2.5, 0.0]) }), "add")?;
    }
    let got = shot(t, dev, &mut off, &mut r, &mut s, "18 lists")?;
    let (mut fr, mut fs) = build(t, 10)?;
    let want = shot(t, dev, &mut off, &mut fr, &mut fs, "18 lists, fresh")?;
    let (n, mx) = diff(&got, &want);
    let drawn = got.chunks_exact(4).filter(|p| p[0..3] != CLEAR8).count();
    t.ok(n == 0 && drawn > 500 && s.lists().len() > 16,
         &format!("{level}: detail levels grew list_first first, then {} lists: against a fresh scene {n} pixels differ (largest {mx}), {drawn} pixels drawn", s.lists().len()));
    Some(())
}

fn remove_basics(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let geos = [cube(), prism(5, 0.5), ball(6, 8)];
    let mut m = Vec::new();
    for g in &geos {
        m.push(t.res(r.add_mesh(&g.data()), "add_mesh")?);
    }
    let first = r.mesh_info(m[1])?;
    t.ok(m == vec![MeshId(0), MeshId(1), MeshId(2)], &format!("{level}: a renderer that removes nothing numbers its meshes 0, 1, 2: {m:?}"));
    let mat = t.res(r.add_material(&Material::color(0.5, 0.6, 0.7)), "add_material")?;
    // a scene uses mesh 1: refused; a second scene uses it, the first lets go: refused; the second dropped: removed
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let a = t.res(s.add(Instance { mesh: m[1], material: mat, transform: Transform::at([0.0; 3]) }), "add")?;
    t.ok(inv(r.remove_mesh(m[1])), &format!("{level}: a mesh a scene's instance uses: Invalid"));
    {
        let mut s2 = t.res(Scene::new(&r), "Scene::new")?;
        t.res(s2.add(Instance { mesh: m[1], material: mat, transform: Transform::at([3.0, 0.0, 0.0]) }), "add")?;
        t.res(s.remove(a), "remove")?;
        t.ok(inv(r.remove_mesh(m[1])), &format!("{level}: still used by a second scene: Invalid"));
        t.ok(inv(r.remove_material(mat)), &format!("{level}: a material a scene's instance uses: Invalid"));
    }
    t.ok(r.remove_mesh(m[1]).is_ok() && r.mesh_info(m[1]).is_none() && r.mesh_count() == 2,
         &format!("{level}: the second scene dropped: removed, mesh_info None, {} meshes", r.mesh_count()));
    t.ok(inv(r.remove_mesh(m[1])), &format!("{level}: removed twice: Invalid"));
    t.ok(inv(s.add(Instance { mesh: m[1], material: mat, transform: Transform::at([0.0; 3]) }).map(|_| ())), &format!("{level}: Scene::add of a removed mesh: Invalid"));
    let keep = t.res(s.add(Instance { mesh: m[0], material: mat, transform: Transform::at([0.0; 3]) }), "add")?;
    t.ok(inv(s.set_mesh(keep, m[1])), &format!("{level}: set_mesh to a removed mesh: Invalid"));
    t.ok(inv(r.set_lods(m[0], &[Lod { mesh: Some(m[1]), below: 0.1 }])), &format!("{level}: set_lods with a removed mesh: Invalid"));
    // its place and its ranges again (the GPU done with the frames before: none here)
    t.res(dev.wait_idle(), "wait_idle")?;
    let again = t.res(r.add_mesh(&geos[1].data()), "add_mesh")?;
    let info = r.mesh_info(again)?;
    t.ok(again == MeshId(1 | 1 << 24) && info.base_vertex == first.base_vertex && info.first_index == first.first_index && r.mesh_info(m[1]).is_none(),
         &format!("{level}: the next mesh of that size: id {:#x} (the place again, generation 1), vertices from {} (were {}), indices from {} (were {}); the old id stays invalid",
                  again.0, info.base_vertex, first.base_vertex, info.first_index, first.first_index));
    // a detail level of another mesh: refused until that one lets go
    t.res(r.set_lods(m[0], &[Lod { mesh: Some(again), below: 0.1 }]), "set_lods")?;
    t.ok(inv(r.remove_mesh(again)), &format!("{level}: a detail level of another mesh: Invalid"));
    t.res(r.set_lods(m[0], &[]), "set_lods")?;
    t.ok(r.remove_mesh(again).is_ok(), &format!("{level}: once that mesh has no levels: removed"));
    // a mesh with levels goes with them
    let lv = t.res(r.add_mesh(&geos[2].data()), "add_mesh")?;
    t.res(r.set_lods(m[2], &[Lod { mesh: Some(lv), below: 0.1 }]), "set_lods")?;
    t.ok(r.remove_mesh(m[2]).is_ok() && r.remove_mesh(lv).is_ok(), &format!("{level}: a mesh with a level goes, then the level"));
    // textures: refused while a material names one; its layer again; the arrays do not grow
    let px = noise_rgba(256, 256, 31);
    let tx = t.res(r.add_texture(&TextureData { width: 256, height: 256, rgba: &px, srgb: true, nearest: false }), "add_texture")?;
    let layer = r.texture_info(tx)?.layer;
    let tm = t.res(r.add_material(&Material::textured(TextureRef { texture: tx, sampling: render3d::Sampling::LINEAR })), "add_material")?;
    t.ok(inv(r.remove_texture(tx)), &format!("{level}: a texture a material names: Invalid"));
    t.res(r.set_material(tm, &Material::color(1.0, 0.0, 0.0)), "set_material")?;
    let bytes = r.texture_bytes();
    t.ok(r.remove_texture(tx).is_ok() && r.texture_info(tx).is_none() && r.texture_count() == 0, &format!("{level}: set_material without it: removed"));
    t.ok(inv(r.add_material(&Material::textured(TextureRef { texture: tx, sampling: render3d::Sampling::LINEAR })).map(|_| ())),
         &format!("{level}: a material of a removed texture: Invalid"));
    let tx2 = t.res(r.add_texture(&TextureData { width: 256, height: 256, rgba: &px, srgb: true, nearest: false }), "add_texture")?;
    t.ok(r.texture_info(tx2).is_some_and(|i| i.layer == layer) && r.texture_bytes() == bytes && tx2 == TextureId(1 << 24),
         &format!("{level}: the next texture of its class: layer {:?} (was {layer}), {} bytes of arrays (were {bytes}), id {:#x}", r.texture_info(tx2).map(|i| i.layer),
                  r.texture_bytes(), tx2.0));
    // a material's removal lets its textures go
    let tm2 = t.res(r.add_material(&Material::textured(TextureRef { texture: tx2, sampling: render3d::Sampling::LINEAR })), "add_material")?;
    t.ok(inv(r.remove_texture(tx2)), &format!("{level}: named again: Invalid"));
    t.ok(r.remove_material(tm2).is_ok() && r.remove_texture(tx2).is_ok() && inv(r.set_material(tm2, &Material::color(1.0, 1.0, 1.0))),
         &format!("{level}: the material removed: the texture goes; set_material of the removed material: Invalid"));
    // a removal between prepare and record: the frame is refused, the next prepare takes it
    let cam = Camera::look_at([0.0, 2.0, 8.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, 1.0, 0.1, f64::INFINITY);
    s.set_camera(&cam);
    let mut off = t.res(Off::new(dev, 64, 64, Format::Rgba8Unorm), "target")?;
    shot(t, dev, &mut off, &mut r, &mut s, "before")?;
    let spare = t.res(r.add_mesh(&geos[2].data()), "add_mesh")?;
    t.res(s.prepare(&mut r), "prepare")?;
    t.res(r.remove_mesh(spare), "remove_mesh")?;
    if let Some(mut e) = t.res(dev.encoder(), "encoder") {
        let ca = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: CLEAR, store: true, before: Layout::CopySrc, after: Layout::CopySrc };
        let da = DepthAttachment { view: off.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true, before: Layout::Sampled, after: Layout::Sampled };
        t.ok(inv(r.record(&mut e, &s, ca, Some(da))), &format!("{level}: a removal between prepare and record: record = Invalid"));
    }
    let next = shot(t, dev, &mut off, &mut r, &mut s, "after").is_some();
    t.ok(next, &format!("{level}: the next prepare takes it"));
    // a place's generations spent: 255 uses again, then a new place (a fresh renderer: its first place)
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let g = plate();
    let mut ids = Vec::new();
    for _ in 0..257 {
        let id = t.res(r.add_mesh(&g.data()), "add_mesh")?;
        ids.push(id.0);
        t.res(r.remove_mesh(id), "remove_mesh")?;
    }
    let slot0 = ids[0] & 0xff_ffff;
    let gens: Vec<u32> = ids.iter().map(|v| v >> 24).collect();
    let fine = ids[..256].iter().all(|v| v & 0xff_ffff == slot0) && gens[..256].windows(2).all(|w| w[1] == w[0] + 1) && gens[255] == 255 && ids[256] & 0xff_ffff != slot0;
    t.ok(fine, &format!("{level}: a place used 256 times (generations {} .. {}), then a new place ({:#x})", gens[0], gens[255], ids[256]));
    Some(())
}

/// the churn (common.rs): after removals and new contents in the places they left, the same image as those contents in a
/// fresh renderer; the pools did not grow for them
fn churn_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let mut s = t.res(churn_gallery(&mut r, Format::Rgba8Unorm), "the churn")?;
    let mut off = t.res(Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm), "target")?;
    let got = shot(t, dev, &mut off, &mut r, &mut s, "the churn")?;
    let mut f = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let mut fs = t.res(churn_fresh(&mut f), "fresh")?;
    let want = shot(t, dev, &mut off, &mut f, &mut fs, "fresh")?;
    let (n, m) = diff(&got, &want);
    let blended = want.chunks_exact(4).filter(|p| p[0..3] != CLEAR8).count();
    let pools = r.pool_use();
    t.ok(n == 0 && blended > 1000 && r.mesh_count() == 4 && r.texture_count() == 2,
         &format!("{level}: after removals, the new contents against them in a fresh renderer: {n} pixels differ (largest {m}); pools (end, room, holes): {pools:?}"));
    Some(())
}

/// what the model expects of a step, and what happened
fn step_ok(t: &mut T, fails: &mut u32, step: usize, what: &str, want: bool, got: gpu::Result<()>) {
    let fine = match got {
        Ok(()) => want,
        Err(GpuError::Invalid(_)) => !want,
        Err(_) => false,
    };
    if !fine {
        *fails += 1;
        if *fails <= 5 {
            out(&format!("    step {step}: {what}: expected {}, got {got:?}", if want { "Ok" } else { "Invalid" }));
        }
    }
    let _ = t;
}

/// the model's contents in a fresh renderer: (renderer, scene)
struct Model {
    meshes: Vec<(MeshId, u32)>,
    textures: Vec<(TextureId, u32)>,
    materials: Vec<(MaterialId, [f32; 3], Option<TextureId>)>,
    instances: Vec<(InstanceId, MeshId, MaterialId, usize)>,
    lods: Vec<(MeshId, MeshId)>,
    dead_meshes: Vec<MeshId>,
    dead_textures: Vec<TextureId>,
    dead_materials: Vec<MaterialId>,
}

const CELLS: usize = 72;

fn cell_pos(c: usize) -> [f64; 3] {
    [(c % 12) as f64 * 1.4 - 7.7, (c / 12) as f64 * 1.4 - 3.5, -(((c * 7) % 5) as f64)]
}

fn mesh_of(k: u32) -> Geo {
    if k == 0 { cube() } else { prism(2 + k, 0.4 + k as f32 * 0.05) }
}

impl Model {
    fn fresh(&self, t: &mut T, dev: &Device, cam: &Camera) -> Option<(Renderer, Scene)> {
        let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
        let mut mesh = Vec::new();
        for (id, k) in &self.meshes {
            mesh.push((*id, t.res(r.add_mesh(&mesh_of(*k).data()), "add_mesh")?));
        }
        let mut tex = Vec::new();
        for (id, seed) in &self.textures {
            let px = noise_rgba(128, 128, *seed);
            tex.push((*id, t.res(r.add_texture(&TextureData { width: 128, height: 128, rgba: &px, srgb: true, nearest: false }), "add_texture")?));
        }
        let mut mat = Vec::new();
        for (id, c, tx) in &self.materials {
            let mut m = Material::color(c[0], c[1], c[2]);
            m.base_color_texture = tx.map(|x| TextureRef { texture: tex.iter().find(|y| y.0 == x).map_or(x, |y| y.1), sampling: render3d::Sampling::LINEAR });
            mat.push((*id, t.res(r.add_material(&m), "add_material")?));
        }
        let to = |v: &[(MeshId, MeshId)], id: MeshId| v.iter().find(|x| x.0 == id).map_or(id, |x| x.1);
        for (base, lv) in &self.lods {
            t.res(r.set_lods(to(&mesh, *base), &[Lod { mesh: Some(to(&mesh, *lv)), below: 0.05 }]), "set_lods")?;
        }
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        s.set_camera(cam);
        for (_, me, ma, c) in &self.instances {
            let material = mat.iter().find(|x| x.0 == *ma).map_or(*ma, |x| x.1);
            t.res(s.add(Instance { mesh: to(&mesh, *me), material, transform: Transform { pos: cell_pos(*c), rot: quat(cpu::norm([1.0, 1.0, 0.3]), 0.6), scale: [0.45; 3] } }),
                  "add")?;
        }
        Some((r, s))
    }
}

/// a random sequence of 400 steps against a model: every answer (Ok or Invalid), the counts, live and dead ids; every 40
/// steps the image against the model's contents in a fresh renderer
fn random_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (160u32, 90u32);
    let cam = Camera::look_at([0.0, 0.0, 14.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0], 1.0, w as f64 / h as f64, 0.1, f64::INFINITY);
    let mut r = t.res(Renderer::new(dev, &linear_desc(Format::Rgba8Unorm)), "Renderer::new")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    s.set_camera(&cam);
    let mut md = Model { meshes: Vec::new(), textures: Vec::new(), materials: Vec::new(), instances: Vec::new(), lods: Vec::new(), dead_meshes: Vec::new(),
                         dead_textures: Vec::new(), dead_materials: Vec::new() };
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut rng = Rng(9001);
    let mut fails = 0u32;
    let (mut images, mut images_bad, mut counted) = (0u32, 0u32, [0u32; 12]);
    let mut free_cells: Vec<usize> = (0..CELLS).rev().collect();
    for step in 0..400usize {
        let op = rng.below(12);
        counted[op as usize] += 1;
        match op {
            0 => {
                let k = rng.below(10);
                if let Some(id) = t.res(r.add_mesh(&mesh_of(k).data()), "add_mesh") {
                    md.meshes.push((id, k));
                }
            }
            1 => {
                // a live mesh, or now and then a dead one
                let id = if !md.dead_meshes.is_empty() && rng.below(5) == 0 { md.dead_meshes[rng.below(md.dead_meshes.len() as u32) as usize] }
                         else if md.meshes.is_empty() { continue } else { md.meshes[rng.below(md.meshes.len() as u32) as usize].0 };
                let live = md.meshes.iter().any(|x| x.0 == id);
                let used = md.instances.iter().any(|x| x.1 == id) || md.lods.iter().any(|x| x.1 == id && x.0 != id);
                let want = live && !used;
                step_ok(t, &mut fails, step, "remove_mesh", want, r.remove_mesh(id));
                if want {
                    md.meshes.retain(|x| x.0 != id);
                    md.lods.retain(|x| x.0 != id);
                    md.dead_meshes.push(id);
                }
            }
            2 => {
                let seed = 700 + step as u32;
                let px = noise_rgba(128, 128, seed);
                if let Some(id) = t.res(r.add_texture(&TextureData { width: 128, height: 128, rgba: &px, srgb: true, nearest: false }), "add_texture") {
                    md.textures.push((id, seed));
                }
            }
            3 => {
                let id = if !md.dead_textures.is_empty() && rng.below(5) == 0 { md.dead_textures[rng.below(md.dead_textures.len() as u32) as usize] }
                         else if md.textures.is_empty() { continue } else { md.textures[rng.below(md.textures.len() as u32) as usize].0 };
                let live = md.textures.iter().any(|x| x.0 == id);
                let want = live && !md.materials.iter().any(|x| x.2 == Some(id));
                step_ok(t, &mut fails, step, "remove_texture", want, r.remove_texture(id));
                if want {
                    md.textures.retain(|x| x.0 != id);
                    md.dead_textures.push(id);
                }
            }
            4 | 6 => {
                // a new material, or a row changed (now and then of a dead material, or naming a dead texture)
                let c = [rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32];
                let tx = if rng.below(2) == 0 { None } else if !md.dead_textures.is_empty() && rng.below(6) == 0 {
                    Some(md.dead_textures[rng.below(md.dead_textures.len() as u32) as usize])
                } else if md.textures.is_empty() { None } else { Some(md.textures[rng.below(md.textures.len() as u32) as usize].0) };
                let tex_live = tx.is_none_or(|x| md.textures.iter().any(|y| y.0 == x));
                let mut m = Material::color(c[0], c[1], c[2]);
                m.base_color_texture = tx.map(|x| TextureRef { texture: x, sampling: render3d::Sampling::LINEAR });
                if op == 4 {
                    let got = r.add_material(&m);
                    let id = got.as_ref().ok().copied();
                    step_ok(t, &mut fails, step, "add_material", tex_live, got.map(|_| ()));
                    if let (true, Some(id)) = (tex_live, id) {
                        md.materials.push((id, c, tx));
                    }
                } else {
                    let id = if !md.dead_materials.is_empty() && rng.below(5) == 0 { md.dead_materials[rng.below(md.dead_materials.len() as u32) as usize] }
                             else if md.materials.is_empty() { continue } else { md.materials[rng.below(md.materials.len() as u32) as usize].0 };
                    let live = md.materials.iter().any(|x| x.0 == id);
                    let want = live && tex_live;
                    step_ok(t, &mut fails, step, "set_material", want, r.set_material(id, &m));
                    if want {
                        if let Some(x) = md.materials.iter_mut().find(|x| x.0 == id) {
                            *x = (id, c, tx);
                        }
                    }
                }
            }
            5 => {
                let id = if !md.dead_materials.is_empty() && rng.below(5) == 0 { md.dead_materials[rng.below(md.dead_materials.len() as u32) as usize] }
                         else if md.materials.is_empty() { continue } else { md.materials[rng.below(md.materials.len() as u32) as usize].0 };
                let live = md.materials.iter().any(|x| x.0 == id);
                let want = live && !md.instances.iter().any(|x| x.2 == id);
                step_ok(t, &mut fails, step, "remove_material", want, r.remove_material(id));
                if want {
                    md.materials.retain(|x| x.0 != id);
                    md.dead_materials.push(id);
                }
            }
            7 | 8 => {
                // an instance of a (sometimes dead) mesh and material
                let Some(&cell) = free_cells.last() else { continue };
                let me = if !md.dead_meshes.is_empty() && rng.below(6) == 0 { md.dead_meshes[rng.below(md.dead_meshes.len() as u32) as usize] }
                         else if md.meshes.is_empty() { continue } else { md.meshes[rng.below(md.meshes.len() as u32) as usize].0 };
                let ma = if !md.dead_materials.is_empty() && rng.below(6) == 0 { md.dead_materials[rng.below(md.dead_materials.len() as u32) as usize] }
                         else if md.materials.is_empty() { continue } else { md.materials[rng.below(md.materials.len() as u32) as usize].0 };
                let want = md.meshes.iter().any(|x| x.0 == me) && md.materials.iter().any(|x| x.0 == ma);
                let got = s.add(Instance { mesh: me, material: ma, transform: Transform { pos: cell_pos(cell), rot: quat(cpu::norm([1.0, 1.0, 0.3]), 0.6), scale: [0.45; 3] } });
                let id = got.as_ref().ok().copied();
                step_ok(t, &mut fails, step, "Scene::add", want, got.map(|_| ()));
                if let (true, Some(id)) = (want, id) {
                    free_cells.pop();
                    md.instances.push((id, me, ma, cell));
                }
            }
            9 => {
                if md.instances.is_empty() {
                    continue;
                }
                let k = rng.below(md.instances.len() as u32) as usize;
                if rng.below(2) == 0 {
                    let (id, _, _, cell) = md.instances.swap_remove(k);
                    step_ok(t, &mut fails, step, "Scene::remove", true, s.remove(id));
                    free_cells.push(cell);
                } else {
                    let me = if !md.dead_meshes.is_empty() && rng.below(4) == 0 { md.dead_meshes[rng.below(md.dead_meshes.len() as u32) as usize] }
                             else if md.meshes.is_empty() { continue } else { md.meshes[rng.below(md.meshes.len() as u32) as usize].0 };
                    let want = md.meshes.iter().any(|x| x.0 == me);
                    step_ok(t, &mut fails, step, "Scene::set_mesh", want, s.set_mesh(md.instances[k].0, me));
                    if want {
                        md.instances[k].1 = me;
                    }
                }
            }
            10 => {
                // a detail level, or none
                if md.meshes.len() < 2 {
                    continue;
                }
                let base = md.meshes[rng.below(md.meshes.len() as u32) as usize].0;
                if rng.below(3) == 0 {
                    step_ok(t, &mut fails, step, "set_lods none", true, r.set_lods(base, &[]));
                    md.lods.retain(|x| x.0 != base);
                } else {
                    let lv = if !md.dead_meshes.is_empty() && rng.below(5) == 0 { md.dead_meshes[rng.below(md.dead_meshes.len() as u32) as usize] }
                             else { md.meshes[rng.below(md.meshes.len() as u32) as usize].0 };
                    let want = md.meshes.iter().any(|x| x.0 == lv);
                    step_ok(t, &mut fails, step, "set_lods", want, r.set_lods(base, &[Lod { mesh: Some(lv), below: 0.05 }]));
                    if want {
                        md.lods.retain(|x| x.0 != base);
                        md.lods.push((base, lv));
                    }
                }
            }
            _ => {
                // a frame (they record: the next removal needs a submission after it)
                shot(t, dev, &mut off, &mut r, &mut s, "a frame")?;
            }
        }
        // the counts, and every id the model knows: live ones answer, dead ones do not
        let counts = r.mesh_count() as usize == md.meshes.len() && r.material_count() as usize == md.materials.len() && r.texture_count() as usize == md.textures.len()
            && s.len() as usize == md.instances.len();
        let ids = md.meshes.iter().all(|x| r.mesh_info(x.0).is_some()) && md.dead_meshes.iter().all(|x| r.mesh_info(*x).is_none())
            && md.textures.iter().all(|x| r.texture_info(x.0).is_some()) && md.dead_textures.iter().all(|x| r.texture_info(*x).is_none());
        if !(counts && ids) {
            fails += 1;
            if fails <= 5 {
                out(&format!("    step {step}: counts {counts}, ids {ids}: meshes {} / {}, materials {} / {}, textures {} / {}, instances {} / {}", r.mesh_count(),
                             md.meshes.len(), r.material_count(), md.materials.len(), r.texture_count(), md.textures.len(), s.len(), md.instances.len()));
            }
        }
        if step % 40 == 39 {
            let got = shot(t, dev, &mut off, &mut r, &mut s, "the model's scene")?;
            let (mut fr, mut fs) = md.fresh(t, dev, &cam)?;
            let want = shot(t, dev, &mut off, &mut fr, &mut fs, "the model's contents, fresh")?;
            let (n, _) = diff(&got, &want);
            images += 1;
            if n > 0 {
                images_bad += 1;
                out(&format!("    step {step}: {n} pixels differ from a fresh renderer"));

            }
        }
    }
    let pools = r.pool_use();
    t.ok(fails == 0 && images_bad == 0,
         &format!("{level}: 400 random steps (per kind {counted:?}) against a model: {fails} wrong; {images} images against fresh renderers, {images_bad} differ; \
                   in the end {} meshes, {} materials, {} textures, {} instances, {} dead ids tried; pools (end, room, holes) {pools:?}",
                  md.meshes.len(), md.materials.len(), md.textures.len(), md.instances.len(), md.dead_meshes.len() + md.dead_textures.len() + md.dead_materials.len()));
    Some(())
}
