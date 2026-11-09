// lighttest -- render3dtest's tests of point and spot lights (G5c), on Voll and on a device limited to Basis,
// against the ray caster (cpu.rs: KHR_lights_punctual's falloff, range window and cone): point lights with and
// without a range, spot lights, 64 lights at once, the light itself in Rgba16Float, nothing beyond a range, a
// changed list against a scene built anew, what goes up, the same at 1e7, what Scene::set_lights refuses; and
// KHR_lights_punctual in lib/gltf against files written here: what is read, where the nodes put the lights, the
// file drawn against the ray caster (from the description, not from the import), truncated, corrupted and broken
// files, the limit.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, Light, LightKind, Scene, Sun, Tonemap};

use crate::common::*;
use crate::cpu::{self, RefLight};
use crate::pbr::{Lit, build_lit, compare_hdr, read_hdr};
use crate::tests::{T, on_levels, out, ref_camera, shot};

pub fn lights(t: &mut T) {
    gltf_lights(t);
    on_levels(t, &mut |t, dev, level| { let _ = lights_on(t, dev, level); });
}

/// a light for the reference
pub fn ref_light(l: &Light) -> RefLight {
    let k = l.intensity as f64;
    let spot = match l.kind {
        LightKind::Point => None,
        LightKind::Spot { inner, outer } => Some((cpu::norm(l.direction), inner as f64, outer as f64)),
    };
    RefLight { pos: l.pos, color: [l.color[0] as f64 * k, l.color[1] as f64 * k, l.color[2] as f64 * k], range: l.range.map(|r| r as f64), spot }
}

fn build_lights(t: &mut T, dev: &Device, format: Format, at: [f64; 3], lights: &[Light]) -> Option<(Lit, Camera)> {
    let (geos, items) = light_scene(at);
    let cam = light_camera(at, GALLERY_W as f64 / GALLERY_H as f64);
    let mut l = build_lit(t, dev, format, &light_sun(), &geos, &[], &items, &cam)?;
    l.s.set_origin(at);
    t.res(l.s.set_lights(lights), "set_lights")?;
    l.rs.lights = lights.iter().map(ref_light).collect();
    Some((l, cam))
}

fn lights_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let set = light_set([0.0; 3]);
    let (mut l, cam) = build_lights(t, dev, Format::Rgba8Unorm, [0.0; 3], &set)?;
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "lights")?;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: two point lights (one with range 4), two spot lights, a dim sun: {}", c.text()));
    let bytes = l.s.upload_info().bytes;
    let _ = shot(t, dev, &mut off, &mut l.r, &mut l.s, "lights, idle")?;
    let idle = l.s.upload_info().bytes;
    t.ok(idle == 192 + 480 && bytes >= 192 + 480 + 4 * 64, &format!("{level}: the lights go up when they change: {bytes} bytes with them, {idle} in the next frame"));
    // the light itself
    let mut hdr = t.res(Off::new(dev, w, h, Format::Rgba16Float), "HDR")?;
    let (mut lh, _) = build_lights(t, dev, Format::Rgba16Float, [0.0; 3], &set)?;
    lh.s.set_tonemap(Tonemap::None);
    lh.rs.tonemap = false;
    let id = t.res(hdr.submit(&mut lh.r, &mut lh.s), "HDR")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let himg = t.res(read_hdr(&hdr), "read")?;
    let hc = compare_hdr(&lh.rs, &ref_camera(&cam), w, h, &himg, 2e-3, 3e-3);
    t.ok(hc.bad == 0, &format!("{level}: the same light in Rgba16Float, within 2e-3 + 3e-3 x: {}", hc.text()));
    // nothing beyond a range: with and without the light of range 4 the same wherever the point is farther
    let without: Vec<Light> = set.iter().enumerate().filter(|(i, _)| *i != 1).map(|(_, l)| *l).collect();
    let (mut lw, _) = build_lights(t, dev, Format::Rgba8Unorm, [0.0; 3], &without)?;
    let img_w = shot(t, dev, &mut off, &mut lw.r, &mut lw.s, "without")?;
    let rc = ref_camera(&cam);
    let (mut beyond, mut within) = (0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let a = ((y * w + x) * 4) as usize;
            if img[a..a + 3] == img_w[a..a + 3] {
                continue;
            }
            let d = rc.ray(x as f64 + 0.5, y as f64 + 0.5, w, h);
            if let Some(hit) = l.rs.trace(rc.eye, d, rc.near, rc.far) {
                let p = [rc.eye[0] + d[0] * hit.z, rc.eye[1] + d[1] * hit.z, rc.eye[2] + d[2] * hit.z];
                let q = set[1].pos;
                let dist = cpu::sqrt((p[0] - q[0]) * (p[0] - q[0]) + (p[1] - q[1]) * (p[1] - q[1]) + (p[2] - q[2]) * (p[2] - q[2]));
                if dist > 4.0 + 0.05 {
                    beyond += 1;
                } else {
                    within += 1;
                }
            }
        }
    }
    t.ok(beyond == 0 && within > 100, &format!("{level}: the light of range 4 changes {within} pixels within its range and {beyond} beyond"));
    // 64 lights at once
    let mut rng = Rng(64);
    let many: Vec<Light> = (0..64).map(|i| {
        let pos = [rng.range(-7.0, 7.0), rng.range(0.3, 3.0), rng.range(-7.0, 3.0)];
        let color = [rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32, rng.range(0.2, 1.0) as f32];
        if i % 4 == 3 {
            Light { kind: LightKind::Spot { inner: 0.2, outer: 0.6 }, pos, direction: [rng.range(-0.5, 0.5), -1.0, rng.range(-0.5, 0.5)], color, intensity: 3.0, range: Some(6.0) }
        } else {
            Light::point(pos, color, 0.8, if i % 2 == 0 { Some(rng.range(1.5, 5.0) as f32) } else { None })
        }
    }).collect();
    t.res(l.s.set_lights(&many), "set_lights")?;
    l.rs.lights = many.iter().map(ref_light).collect();
    let img = shot(t, dev, &mut off, &mut l.r, &mut l.s, "64 lights")?;
    let c = cpu::compare_tol(&l.rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: 64 lights (points with and without range, spots), set on the running scene: {}", c.text()));
    let (mut f, _) = build_lights(t, dev, Format::Rgba8Unorm, [0.0; 3], &many)?;
    let fresh = shot(t, dev, &mut off, &mut f.r, &mut f.s, "64 lights anew")?;
    let (n, m) = diff(&img, &fresh);
    t.ok(n == 0, &format!("{level}: the changed list against a scene built anew with it: {n} pixels differ (largest {m})"));
    // far away: lights and scene at 1e7, the origin there
    let far = [1e7, 5e5, -1e7];
    let (mut lf, _) = build_lights(t, dev, Format::Rgba8Unorm, far, &light_set(far))?;
    let img_far = shot(t, dev, &mut off, &mut lf.r, &mut lf.s, "lights at 1e7")?;
    let (mut l0, _) = build_lights(t, dev, Format::Rgba8Unorm, [0.0; 3], &set)?;
    let img0 = shot(t, dev, &mut off, &mut l0.r, &mut l0.s, "lights at 0")?;
    let (n, m) = diff(&img0, &img_far);
    t.ok(n == 0, &format!("{level}: at 0 and at 1e7, the origin there: {n} pixels differ (largest {m})"));
    // what set_lights refuses (and keeps the list it had)
    let inv = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    let base = set[2];
    let bad = [
        Light { intensity: -1.0, ..base },
        Light { range: Some(0.0), ..base },
        Light { kind: LightKind::Spot { inner: 0.5, outer: 0.5 }, ..base },
        Light { kind: LightKind::Spot { inner: 0.1, outer: 1.7 }, ..base },
        Light { pos: [f64::NAN, 0.0, 0.0], ..base },
        Light { color: [1.0, -0.1, 1.0], ..base },
        Light { direction: [0.0; 3], ..base },
    ];
    let mut refused = 0;
    for b in bad {
        refused += inv(l0.s.set_lights(&[set[0], b])) as u32;
    }
    t.ok(refused == 7 && l0.s.lights() == &set[..], &format!("{level}: set_lights refuses {refused} of 7 broken lights and keeps its list"));
    Some(())
}

// ---------------------------------------------------------------- KHR_lights_punctual in lib/gltf

/// a quaternion from an axis and an angle, f64
fn q(axis: [f64; 3], angle: f64) -> [f64; 4] {
    let a = cpu::norm(axis);
    let (s, c) = cpu::sincos(angle * 0.5);
    [a[0] * s, a[1] * s, a[2] * s, c]
}

fn num(v: f64) -> String {
    format!("{v:?}")
}

/// the lights' JSON and nodes of the test file: (lights, light nodes as (translation, rotation, light, parent))
fn file_lights() -> (String, Vec<([f64; 3], [f64; 4], usize, Option<usize>)>) {
    let lights = String::from(concat!(
        r#"[{"name":"red","type":"point","color":[1.0,0.3,0.2],"intensity":8.0},"#,
        r#"{"type":"point","color":[0.3,1.0,0.4],"intensity":6.0,"range":4.0},"#,
        r#"{"name":"cone","type":"spot","color":[1.0,1.0,0.9],"intensity":40.0,"range":12.0,"spot":{"innerConeAngle":0.3,"outerConeAngle":0.5}},"#,
        r#"{"type":"directional","color":[0.31,0.31,0.31],"intensity":1.0},"#,
        r#"{"type":"spot","color":[0.3,0.4,1.0],"intensity":30.0,"spot":{"outerConeAngle":0.35}}]"#));
    // the spot at (0, 5, 2) turned from -z to straight down; the blue spot below a parent node (which carries a
    // translation); the red light twice
    let down = q([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let nodes = vec![
        ([-3.0, 1.5, 1.0], [0.0, 0.0, 0.0, 1.0], 0, None),
        ([2.0, 0.8, -1.0], [0.0, 0.0, 0.0, 1.0], 1, None),
        ([0.0, 5.0, 2.0], down, 2, None),
        ([0.0, 0.0, 0.0], q([1.0, 0.0, 0.0], -1.2), 3, None),
        ([1.0, 1.0, 0.0], q([0.0, 0.0, 1.0], 0.6), 4, Some(1)),
        ([-3.0, 1.5, -5.0], [0.0, 0.0, 0.0, 1.0], 0, None),
    ];
    (lights, nodes)
}

/// a glTF file: a ground plate, the lights above; `lights` the extension's lights array, `required` names it in
/// extensionsRequired
fn light_file(lights: &str, nodes: &[([f64; 3], [f64; 4], usize, Option<usize>)], required: bool) -> Vec<u8> {
    let g = plate();
    let mut bin = Vec::new();
    for p in &g.pos {
        for v in p {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    for n in &g.nrm {
        for v in n {
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    for i in &g.idx {
        bin.extend_from_slice(&(*i as u16).to_le_bytes());
    }
    let nv = g.pos.len();
    let mut node_json = vec![format!(r#"{{"mesh":0,"rotation":{:?},"scale":[9.0,7.0,1.0],"translation":[0.0,0.0,-2.0]}}"#, q([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2))];
    // node 1: the parent of light nodes that name it
    node_json.push(String::from(r#"{"translation":[3.0,2.0,-3.0],"children":[]}"#));
    let mut children_of_1 = Vec::new();
    let mut roots = vec![0usize, 1];
    for (k, (t, r, light, parent)) in nodes.iter().enumerate() {
        let i = node_json.len();
        node_json.push(format!(r#"{{"translation":[{},{},{}],"rotation":[{},{},{},{}],"extensions":{{"KHR_lights_punctual":{{"light":{light}}}}}}}"#,
                               num(t[0]), num(t[1]), num(t[2]), num(r[0]), num(r[1]), num(r[2]), num(r[3])));
        match parent {
            Some(1) => children_of_1.push(i),
            _ => roots.push(i),
        }
        let _ = k;
    }
    node_json[1] = format!(r#"{{"translation":[3.0,2.0,-3.0],"children":{children_of_1:?}}}"#);
    let req = if required { r#""extensionsRequired":["KHR_lights_punctual"],"# } else { "" };
    let json = format!(concat!(r#"{{"asset":{{"version":"2.0"}},"extensionsUsed":["KHR_lights_punctual"],{}"extensions":{{"KHR_lights_punctual":{{"lights":{}}}}},"#,
                               r#""buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}],"#,
                               r#""bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{}}},{{"buffer":0,"byteOffset":{},"byteLength":{}}},{{"buffer":0,"byteOffset":{},"byteLength":{}}}],"#,
                               r#""accessors":[{{"bufferView":0,"componentType":5126,"count":{},"type":"VEC3","min":[-1,-1,0],"max":[1,1,0]}},{{"bufferView":1,"componentType":5126,"count":{},"type":"VEC3"}},{{"bufferView":2,"componentType":5123,"count":{},"type":"SCALAR"}}],"#,
                               r#""materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[0.8,0.8,0.8,1.0],"metallicFactor":0.0,"roughnessFactor":0.8}}}}],"#,
                               r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1}},"indices":2,"material":0}}]}}],"#,
                               r#""nodes":[{}],"scenes":[{{"nodes":{:?}}}],"scene":0}}"#),
                       req, lights, bin.len(), gltf::base64_encode(&bin), nv * 12, nv * 12, nv * 12, nv * 24, g.idx.len() * 2, nv, nv, g.idx.len(), node_json.join(","), roots);
    json.into_bytes()
}

fn load(bytes: &[u8], limits: &gltf::Limits) -> core::result::Result<gltf::Document, gltf::Error> {
    gltf::load(bytes, &mut |_| None, limits)
}

fn gltf_lights(t: &mut T) {
    let (lights, nodes) = file_lights();
    let file = light_file(&lights, &nodes, false);
    let lim = gltf::Limits::DEFAULT;
    let doc = match load(&file, &lim) {
        Ok(d) => d,
        Err(e) => {
            t.ok(false, &format!("glTF lights: the file loads: {e}"));
            return;
        }
    };
    // what is read
    let kinds: Vec<gltf::LightKind> = doc.lights.iter().map(|l| l.kind).collect();
    let want = [gltf::LightKind::Point, gltf::LightKind::Point, gltf::LightKind::Spot { inner: 0.3, outer: 0.5 }, gltf::LightKind::Directional,
                gltf::LightKind::Spot { inner: 0.0, outer: 0.35 }];
    let values = doc.lights.len() == 5 && kinds == want && doc.lights[0].name == "red" && doc.lights[0].color == [1.0, 0.3, 0.2] && doc.lights[0].intensity == 8.0
        && doc.lights[0].range.is_none() && doc.lights[1].range == Some(4.0) && doc.lights[2].range == Some(12.0) && doc.lights[3].intensity == 1.0;
    t.ok(values, &format!("glTF lights: 5 lights read as written (types, names, colours, intensities, ranges, cone angles, defaults): {:?}", doc.lights.iter().map(|l| (l.kind, l.intensity, l.range)).collect::<Vec<_>>()));
    // where the nodes put them: the world matrix composed here, the red light twice, the blue spot under a parent
    let placed = doc.lights_of(None);
    let mut worst = 0.0f64;
    let mut order = Vec::new();
    for (k, (tr, r, light, parent)) in nodes.iter().enumerate() {
        let local = gltf::trs_matrix(*tr, *r, [1.0; 3]);
        let world = if parent.is_some() { gltf::mat_mul(&gltf::trs_matrix([3.0, 2.0, -3.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]), &local) } else { local };
        let Some(p) = placed.iter().find(|x| x.node == 2 + k) else { continue };
        order.push(p.light);
        for i in 0..16 {
            worst = worst.max((p.world[i] - world[i]).abs());
        }
        if p.light != *light {
            worst = f64::MAX;
        }
    }
    let spot = placed.iter().find(|p| p.light == 2).map(|p| p.direction()).unwrap_or([0.0; 3]);
    t.ok(placed.len() == 6 && worst < 1e-12 && (spot[1] + 1.0).abs() < 1e-12, &format!("glTF lights: {} placed by their nodes (one light twice, one under a parent), world matrices {worst:.1e} from those composed here; the spot turned down shines {spot:?}", placed.len()));
    let req = light_file(&lights, &nodes, true);
    t.ok(load(&req, &lim).is_ok(), "glTF lights: KHR_lights_punctual in extensionsRequired loads");
    // truncated: every length of the text
    let mut loaded = 0;
    for n in 0..file.len() {
        loaded += load(&file[..n], &lim).is_ok() as u32;
    }
    t.ok(loaded == 0, &format!("glTF lights: {} truncated files: {loaded} load", file.len()));
    // corrupted: 1 to 6 bytes changed at random
    let mut rng = Rng(77);
    let (mut ok, mut err) = (0, 0);
    for _ in 0..600 {
        let mut b = file.clone();
        for _ in 0..1 + rng.below(6) {
            let at = rng.below(b.len() as u32) as usize;
            b[at] = rng.next() as u8;
        }
        match load(&b, &lim) {
            Ok(d) => {
                ok += 1;
                // whatever loads holds lights that keep the rules
                let _ = d.lights_of(None);
            }
            Err(_) => err += 1,
        }
    }
    t.ok(ok + err == 600, &format!("glTF lights: 600 files with 1 to 6 bytes changed: {ok} load, {err} errors, no panic"));
    // one rule broken each
    let breaks: [(&str, &str, &str); 17] = [
        (r#""type":"point","color":[1.0,0.3,0.2]"#, r#""color":[1.0,0.3,0.2]"#, "a light without type"),
        (r#""name":"red","type":"point""#, r#""name":"red","type":"area""#, "an unknown type"),
        (r#""type":"point","color":[1.0,0.3,0.2]"#, r#""type":7,"color":[1.0,0.3,0.2]"#, "a type that is no string"),
        (r#""color":[1.0,0.3,0.2]"#, r#""color":[1.0,0.3]"#, "two colour channels"),
        (r#""color":[1.0,0.3,0.2]"#, r#""color":[1.0,-0.3,0.2]"#, "a negative colour"),
        (r#""intensity":8.0"#, r#""intensity":-8.0"#, "a negative intensity"),
        (r#""intensity":8.0"#, r#""intensity":1e999"#, "an infinite intensity"),
        (r#""intensity":8.0"#, r#""intensity":"8""#, "an intensity as text"),
        (r#""range":4.0"#, r#""range":0"#, "range 0"),
        (r#""range":4.0"#, r#""range":-4.0"#, "a negative range"),
        (r#""innerConeAngle":0.3,"outerConeAngle":0.5"#, r#""innerConeAngle":0.5,"outerConeAngle":0.5"#, "inner = outer"),
        (r#""innerConeAngle":0.3,"outerConeAngle":0.5"#, r#""innerConeAngle":0.3,"outerConeAngle":1.7"#, "outer beyond pi / 2"),
        (r#""innerConeAngle":0.3,"outerConeAngle":0.5"#, r#""innerConeAngle":-0.1,"outerConeAngle":0.5"#, "a negative inner angle"),
        (r#","spot":{"innerConeAngle":0.3,"outerConeAngle":0.5}"#, "", "a spot light without spot"),
        (r#"{"KHR_lights_punctual":{"light":4}}"#, r#"{"KHR_lights_punctual":{"light":9}}"#, "a node naming a light the file lacks"),
        (r#"{"KHR_lights_punctual":{"light":4}}"#, r#"{"KHR_lights_punctual":{"light":-1}}"#, "a negative light index"),
        (r#""KHR_lights_punctual":{"lights":["#, r#""KHR_lights_punctual":{"lights":{"x":0},"y":["#, "lights as an object"),
    ];
    let text = String::from_utf8(file.clone()).unwrap_or_default();
    let mut fails = Vec::new();
    for (from, to, what) in breaks {
        if !text.contains(from) {
            fails.push(format!("{what}: the pattern is not in the file"));
            continue;
        }
        let b = text.replacen(from, to, 1);
        match load(b.as_bytes(), &lim) {
            Err(gltf::Error::Invalid(_)) | Err(gltf::Error::Json(_)) => {}
            r => fails.push(format!("{what}: {:?}", r.map(|_| "loads"))),
        }
    }
    t.ok(fails.is_empty(), &format!("glTF lights: 17 files with one rule broken each are Invalid: {fails:?}"));
    let small = gltf::Limits { lights: 4, ..lim };
    t.ok(matches!(load(&file, &small), Err(gltf::Error::Limit(_))), "glTF lights: 5 lights against a limit of 4: Limit");
    // upload: the point and spot lights for Scene::set_lights, the directional one as a sun
    let Some(g) = crate::tests::setup(t) else { return };
    for (level, max_level) in [("Voll", Level::Voll), ("Basis", Level::Basis)] {
        if max_level == Level::Voll && g.adapter.level() != Level::Voll {
            continue;
        }
        let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc { max_level }), "device") else { return };
        let _ = drawn(t, &dev, level, &doc, &nodes);
    }
}

/// the file drawn: its plate and lights through upload against the ray caster built from the description
fn drawn(t: &mut T, dev: &Device, level: &str, doc: &gltf::Document, nodes: &[([f64; 3], [f64; 4], usize, Option<usize>)]) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(render3d::Renderer::new(dev, &render3d::RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    let imp = t.res(render3d::gltf::upload(&mut r, doc, None), "gltf::upload")?;
    t.ok(imp.lights.len() == 5 && imp.suns.len() == 1, &format!("{level}: upload: {} point and spot lights, {} sun(s)", imp.lights.len(), imp.suns.len()));
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let cam = light_camera([0.0; 3], w as f64 / h as f64);
    s.set_camera(&cam);
    let sun = Sun { ambient: [0.04; 3], ..*imp.suns.first()? };
    s.set_sun(&sun);
    for i in &imp.instances {
        t.res(s.add(*i), "add")?;
    }
    t.res(s.set_lights(&imp.lights), "set_lights")?;
    let img = shot(t, dev, &mut off, &mut r, &mut s, "glTF lights")?;
    // the reference from the description: the sun from the directional node's +z, the lights from their nodes
    let dir_q = nodes[3].1;
    let zcol = gltf::trs_matrix([0.0; 3], dir_q, [1.0; 3]);
    let mut rs = cpu::RefScene::new([zcol[8], zcol[9], zcol[10]], [0.31; 3], [0.04; 3]);
    let ground = Spec { mesh: 0, material: 0, pos: [0.0, 0.0, -2.0], rot: quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2), scale: [9.0, 7.0, 1.0] };
    rs.add_look(&plate(), &ground, cpu::Look { metallic: 0.0, roughness: 0.8, ..cpu::Look::color([0.8, 0.8, 0.8]) });
    let described = [(8.0, [1.0, 0.3, 0.2], None, None), (6.0, [0.3, 1.0, 0.4], Some(4.0), None), (40.0, [1.0, 1.0, 0.9], Some(12.0), Some((0.3, 0.5))),
                     (0.0, [0.0; 3], None, None), (30.0, [0.3, 0.4, 1.0], None, Some((0.0, 0.35)))];
    for (tr, rot, light, parent) in nodes {
        let (k, c, range, cone) = described[*light];
        if *light == 3 {
            continue;
        }
        let pos = if parent.is_some() { [tr[0] + 3.0, tr[1] + 2.0, tr[2] - 3.0] } else { *tr };
        let m = gltf::trs_matrix([0.0; 3], *rot, [1.0; 3]);
        let axis = cpu::norm([-m[8], -m[9], -m[10]]);
        rs.lights.push(RefLight { pos, color: [c[0] * k, c[1] * k, c[2] * k], range, spot: cone.map(|(i, o)| (axis, i, o)) });
    }
    let c = cpu::compare_tol(&rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 1);
    t.ok(c.fine(), &format!("{level}: the file's lights drawn (its sun, two points, two spots, one light twice) against the ray caster from the description: {}", c.text()));
    out(&format!("  {level}: upload's notes: {:?}", imp.notes));
    Some(())
}
