// gltftest -- render3dtest --test gltf: lib/gltf against files gltfgen.rs writes (nothing is downloaded).  The
// three forms of one scene read alike; what is read equals what was written (geometry bit for bit, flat
// normals computed here, materials, texture assignment, samplers, PNG pixels exactly and JPEG closely,
// world matrices composed here, the instances of the scene, decompose); truncated, corrupted and broken
// files are errors of the right kind, never a panic; and the scene drawn by the engine against the ray
// caster built from the description, on Voll and Basis.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ::gltf as g;
use gpu::*;
use render3d::{Renderer, RendererDesc, Scene};

use crate::common::*;
use crate::cpu::{self, Look, RWrap, RefSampling, RefScene, RefTexture};
use crate::gltfgen::{self as gg, Form, GScene};
use crate::tests::{T, on_levels, out, ref_camera, shot, sun_of};

fn load(bytes: &[u8], files: &[(String, Vec<u8>)], limits: &g::Limits) -> g::Result<g::Document> {
    g::load(bytes, &mut |uri: &str| files.iter().find(|f| f.0 == uri).map(|f| f.1.clone()), limits)
}

/// what the importer must make of a primitive as written: flat normals split the vertices
fn expected(p: &gg::GPrim) -> Geo {
    let mut geo = Geo { pos: p.geo.pos.clone(), nrm: p.geo.nrm.clone(), uv: p.geo.uv.clone(), idx: p.geo.idx.clone() };
    if let Some((i, v)) = p.enc.sparse {
        geo.pos[i as usize] = v;
    }
    geo.uv = match p.enc.uv {
        Some(ct) => geo.uv.iter().map(|u| gg::uv_as_read(*u, ct)).collect(),
        None => vec![[0.0; 2]; geo.pos.len()],
    };
    if p.enc.idx.is_none() {
        geo.idx = (0..geo.pos.len() as u32).collect();
    }
    if !p.enc.normals {
        let mut flat = Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
        for t in geo.idx.chunks_exact(3) {
            let v = |i: u32| { let p = geo.pos[i as usize]; [p[0] as f64, p[1] as f64, p[2] as f64] };
            let (a, b, c) = (v(t[0]), v(t[1]), v(t[2]));
            let n = cpu::norm([(b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]), (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
                               (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])]);
            for &i in t {
                flat.idx.push(flat.pos.len() as u32);
                flat.pos.push(geo.pos[i as usize]);
                flat.nrm.push([n[0] as f32, n[1] as f32, n[2] as f32]);
                flat.uv.push(geo.uv[i as usize]);
            }
        }
        geo = flat;
    }
    geo
}

pub fn gltf(t: &mut T) {
    let sc = gg::test_scene();
    let (ext, files) = sc.write(Form::External);
    let (data, _) = sc.write(Form::DataUri);
    let (glb, _) = sc.write(Form::Glb);
    let lim = g::Limits::DEFAULT;
    let t0 = plat::time_us();
    let a = load(&ext, &files, &lim);
    let ms = (plat::time_us() - t0) as f64 / 1000.0;
    let (b, c) = (load(&data, &[], &lim), load(&glb, &[], &lim));
    let (Ok(a), Ok(b), Ok(c)) = (a, b, c) else {
        t.ok(false, &format!("the three forms load: {:?}", [load(&ext, &files, &lim).err(), load(&data, &[], &lim).err(), load(&glb, &[], &lim).err()]));
        return;
    };
    t.ok(true, &format!(".gltf with {} files beside it ({} bytes, {ms:.1} ms), with data URIs ({} bytes), .glb ({} bytes): all three load", files.len(), ext.len(), data.len(), glb.len()));
    t.ok(a == b && a == c, "the three forms read alike, image names and all");
    contents(t, &sc, &a);
    decomposed(t, &sc, &a);
    broken(t, &sc, &glb, &ext, &files);
    on_levels(t, &mut |t, dev, level| { let _ = drawn(t, dev, level, &sc, &c); });
}

fn contents(t: &mut T, sc: &GScene, d: &g::Document) {
    t.ok(d.version == "2.0" && d.generator == "render3dtest gen.rs", &format!("asset: version {:?}, generator {:?}", d.version, d.generator));
    // geometry: positions, uv and indices bit for bit, flat normals to 1e-6
    let mut bad = Vec::new();
    let mut counts = (0, 0, 0);
    t.ok(d.meshes.len() == sc.meshes.len(), &format!("{} meshes", d.meshes.len()));
    for (mi, m) in sc.meshes.iter().enumerate() {
        let Some(dm) = d.meshes.get(mi) else { continue };
        if dm.primitives.len() != m.len() {
            bad.push(format!("mesh {mi}: {} primitives", dm.primitives.len()));
            continue;
        }
        for (pi, p) in m.iter().enumerate() {
            let want = expected(p);
            let got = &dm.primitives[pi];
            counts.0 += 1;
            counts.1 += got.positions.len();
            counts.2 += got.indices.len();
            let normals_close = got.normals.len() == want.nrm.len() && got.normals.iter().zip(&want.nrm).all(|(a, b)| (0..3).all(|k| (a[k] - b[k]).abs() <= 1e-6));
            if got.positions != want.pos || got.uvs != want.uv || got.indices != want.idx || !normals_close || got.material != p.material
                || got.flat_normals == p.enc.normals || got.has_uvs != p.enc.uv.is_some() {
                bad.push(format!("mesh {mi} primitive {pi}"));
            }
        }
    }
    t.ok(bad.is_empty(), &format!("{} primitives, {} vertices, {} indices as written (u8 / u16 / u32 indices, none, a stride of 32, normalized u8 and u16 uv, a sparse accessor, flat normals){}",
                                  counts.0, counts.1, counts.2, if bad.is_empty() { String::new() } else { format!(": wrong {bad:?}") }));
    // materials and which texture is where
    let mut wrong = Vec::new();
    for (i, m) in sc.mats.iter().enumerate() {
        let Some(dm) = d.materials.get(i) else { wrong.push(i); continue };
        let refs = [dm.base_color_texture, dm.metallic_roughness_texture, dm.normal_texture, dm.occlusion_texture, dm.emissive_texture];
        let tex_ok = (0..5).all(|k| refs[k].map(|r| r.texture) == m.tex[k] && refs[k].is_none_or(|r| r.tex_coord == 0));
        let mode_ok = match m.mask {
            Some(c) => dm.alpha_mode == g::AlphaMode::Mask && dm.alpha_cutoff == c,
            None => dm.alpha_mode == g::AlphaMode::Opaque,
        };
        if dm.base_color_factor != m.factor || dm.metallic_factor != m.metallic || dm.roughness_factor != m.roughness || dm.emissive_factor != m.emissive || !tex_ok || !mode_ok {
            wrong.push(i);
        }
    }
    t.ok(wrong.is_empty() && d.materials.len() == sc.mats.len(), &format!("{} materials: factors, the five textures, MASK and its cutoff (wrong: {wrong:?})", d.materials.len()));
    let filt = |c: u32| if c == 9728 { g::Filter::Nearest } else { g::Filter::Linear };
    let wrap = |c: u32| match c { 33071 => g::Wrap::ClampToEdge, 33648 => g::Wrap::MirroredRepeat, _ => g::Wrap::Repeat };
    let mut wrong = Vec::new();
    for (i, x) in sc.texs.iter().enumerate() {
        let s = x.sampler;
        let (min, mip) = match s[1] {
            9728 => (g::Filter::Nearest, None),
            9729 => (g::Filter::Linear, None),
            9984 => (g::Filter::Nearest, Some(g::Filter::Nearest)),
            9985 => (g::Filter::Linear, Some(g::Filter::Nearest)),
            9986 => (g::Filter::Nearest, Some(g::Filter::Linear)),
            _ => (g::Filter::Linear, Some(g::Filter::Linear)),
        };
        let want = g::Sampler { mag: filt(s[0]), min, mip, wrap_s: wrap(s[2]), wrap_t: wrap(s[3]) };
        if d.textures.get(i).is_none_or(|dt| dt.image != Some(x.image) || dt.sampler != want) {
            wrong.push(i);
        }
    }
    t.ok(wrong.is_empty(), &format!("{} textures: image and sampler (wrong: {wrong:?})", d.textures.len()));
    // images: PNG exactly, JPEG closely (quality 95)
    for (i, im) in sc.images.iter().enumerate() {
        let Some(di) = d.images.get(i) else { t.ok(false, &format!("image {i}")); continue };
        if im.jpeg {
            let mean = di.rgba.iter().zip(&im.rgba).enumerate().filter(|(k, _)| k % 4 != 3).map(|(_, (a, b))| a.abs_diff(*b) as u64).sum::<u64>() as f64 / (im.w * im.h * 3) as f64;
            t.ok(di.width == im.w && di.height == im.h && mean < 6.0, &format!("image {i}: JPEG {} x {}, mean difference {mean:.2} from what was encoded", di.width, di.height));
        } else {
            t.ok(di.width == im.w && di.height == im.h && di.rgba == im.rgba, &format!("image {i}: PNG {} x {} with alpha, every byte", di.width, di.height));
        }
    }
    // nodes: world matrices against those composed here; scenes; the instances in order
    let worlds = sc.worlds();
    let worst = (0..sc.nodes.len()).map(|i| d.nodes.get(i).map_or(f64::INFINITY, |n| (0..16).map(|k| (n.world[k] - worlds[i][k]).abs()).fold(0.0, f64::max))).fold(0.0, f64::max);
    t.ok(d.nodes.len() == sc.nodes.len() && worst < 1e-12, &format!("{} nodes (TRS and matrices, a chain 40 deep): world matrices within {worst:.1e} of those composed here", d.nodes.len()));
    t.ok(d.scenes.len() == 2 && d.scene == Some(1), &format!("{} scenes, the file's choice {:?}", d.scenes.len(), d.scene));
    let got: Vec<(usize, usize, usize)> = d.instances(None).iter().map(|i| (i.node, i.mesh, i.primitive)).collect();
    t.ok(got == sc.instances(), &format!("the scene's instances in order: {} (node, mesh, primitive)", got.len()));
    let other: Vec<usize> = d.instances(Some(0)).iter().map(|i| i.node).collect();
    t.ok(other == [sc.nodes.len() - 1], &format!("the other scene's instances: nodes {other:?}"));
}

fn decomposed(t: &mut T, sc: &GScene, d: &g::Document) {
    let mut worst = 0.0f64;
    let mut mirrored = Vec::new();
    for (i, n) in d.nodes.iter().enumerate() {
        let x = g::decompose(&n.world);
        let back = gg::trs(x.translation, x.rotation, x.scale);
        worst = worst.max((0..16).map(|k| (back[k] - n.world[k]).abs()).fold(0.0, f64::max));
        if x.scale[0] < 0.0 {
            mirrored.push(i);
        }
        worst = worst.max(x.shear);
    }
    t.ok(worst < 1e-9 && mirrored == [3], &format!("decompose: T * R * S again within {worst:.1e}, no shear; mirrored (x negative): nodes {mirrored:?}"));
    // a parent scaled unevenly with a turned child: shear, which decompose reports
    let parent = gg::trs([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0, 1.0, 1.0]);
    let (s, c) = cpu::sincos(0.4);
    let child = gg::trs([0.0; 3], [0.0, 0.0, s, c], [1.0; 3]);
    let x = g::decompose(&gg::mul4(&parent, &child));
    t.ok(x.shear > 0.1, &format!("decompose: an unevenly scaled parent of a turned child has shear {:.3}", x.shear));
    let _ = sc;
}

// ---------------------------------------------------------------- broken files

/// a valid file of one triangle, a data URI buffer: positions (36 bytes), u16 indices (6 + 2)
fn mini_json() -> String {
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for x in v {
            bin.extend_from_slice(&x.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    format!(concat!("{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0,\"translation\":[1,2,3]}}],",
                    "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1,\"mode\":4}}]}}],",
                    "\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},{{\"bufferView\":1,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}}],",
                    "\"bufferViews\":[{{\"buffer\":0,\"byteLength\":36}},{{\"buffer\":0,\"byteOffset\":36,\"byteLength\":6}}],",
                    "\"buffers\":[{{\"byteLength\":44,\"uri\":\"data:application/octet-stream;base64,{}\"}}]}}"), g::base64_encode(&bin))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Json,
    Invalid,
    Unsupported,
    Limit,
    Missing,
}

fn kind(e: &g::Error) -> Kind {
    match e {
        g::Error::Json(_) => Kind::Json,
        g::Error::Invalid(_) => Kind::Invalid,
        g::Error::Unsupported(_) => Kind::Unsupported,
        g::Error::Limit(_) => Kind::Limit,
        g::Error::Missing(_) => Kind::Missing,
    }
}

fn broken(t: &mut T, sc: &GScene, glb: &[u8], ext: &[u8], files: &[(String, Vec<u8>)]) {
    let lim = g::Limits::DEFAULT;
    // every prefix of the .glb's first 256 bytes and every 61st after: none loads
    let mut loaded = 0;
    let mut tried = 0;
    let mut n = 0;
    while n < glb.len() {
        tried += 1;
        loaded += load(&glb[..n], &[], &lim).is_ok() as u32;
        n += if n < 256 { 1 } else { 61 };
    }
    t.ok(loaded == 0, &format!("{tried} truncated .glb files ({} bytes whole): {loaded} load", glb.len()));
    // every prefix of the .gltf's JSON
    let mut loaded = 0;
    for n in 0..ext.len() {
        loaded += load(&ext[..n], files, &lim).is_ok() as u32;
    }
    t.ok(loaded == 0, &format!("{} truncated .gltf texts: {loaded} load", ext.len()));
    // bytes changed at random: errors or a document, never a panic (a panic would end the program here)
    let mut rng = Rng(2024);
    let (mut ok, mut err) = (0, 0);
    for k in 0..1200 {
        let (mut b, base) = if k % 2 == 0 { (glb.to_vec(), glb.len()) } else { (ext.to_vec(), ext.len()) };
        for _ in 0..1 + rng.below(6) {
            let at = if k % 4 < 2 { rng.below(base as u32) } else { rng.below((base as u32).min(4096)) } as usize;
            b[at] = rng.next() as u8;
        }
        match load(&b, files, &lim) {
            Ok(_) => ok += 1,
            Err(_) => err += 1,
        }
    }
    t.ok(ok + err == 1200, &format!("1200 files with 1 to 6 bytes changed at random (.glb and .gltf): {ok} still load, {err} are errors, no panic"));
    // one rule broken at a time
    let base = mini_json();
    let cases: Vec<(&str, String, Kind)> = vec![
        ("accessor beyond its view", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":4,\"type\":\"VEC3\"", 1), Kind::Invalid),
        ("a count of 10^12", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":1000000000000,\"type\":\"VEC3\"", 1), Kind::Limit),
        ("a count of 0", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":0,\"type\":\"VEC3\"", 1), Kind::Invalid),
        ("a negative count", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":-3,\"type\":\"VEC3\"", 1), Kind::Invalid),
        ("a view beyond its buffer", base.replacen("\"byteLength\":36}", "\"byteLength\":40000}", 1), Kind::Invalid),
        ("a byte stride of 2", base.replacen("\"byteLength\":36}", "\"byteLength\":36,\"byteStride\":2}", 1), Kind::Invalid),
        ("a byte stride of 8 for 12-byte elements", base.replacen("\"byteLength\":36}", "\"byteLength\":36,\"byteStride\":8}", 1), Kind::Invalid),
        ("an accessor of a view not there", base.replacen("\"bufferView\":1,", "\"bufferView\":9,", 1), Kind::Invalid),
        ("a mesh's accessor not there", base.replacen("\"POSITION\":0", "\"POSITION\":7", 1), Kind::Invalid),
        ("component type 5130", base.replacen("\"componentType\":5126", "\"componentType\":5130", 1), Kind::Invalid),
        ("POSITION as VEC2", base.replacen("\"type\":\"VEC3\"", "\"type\":\"VEC2\"", 1), Kind::Invalid),
        ("POSITION as normalized shorts", base.replacen("\"componentType\":5126", "\"componentType\":5122,\"normalized\":true", 1), Kind::Invalid),
        ("indices as floats", base.replacen("\"componentType\":5123", "\"componentType\":5126", 1), Kind::Invalid),
        ("four indices of triangles", base.replacen("\"componentType\":5123,\"count\":3", "\"componentType\":5123,\"count\":4", 1).replacen("\"byteOffset\":36,\"byteLength\":6", "\"byteOffset\":36,\"byteLength\":8", 1), Kind::Invalid),
        ("mode 7", base.replacen("\"mode\":4", "\"mode\":7", 1), Kind::Invalid),
        ("a material not there", base.replacen("\"mode\":4", "\"mode\":4,\"material\":3", 1), Kind::Invalid),
        ("a node of a mesh not there", base.replacen("{\"mesh\":0,", "{\"mesh\":2,", 1), Kind::Invalid),
        ("two nodes children of each other", base.replacen("\"nodes\":[{\"mesh\":0,\"translation\":[1,2,3]}]", "\"nodes\":[{\"mesh\":0,\"children\":[1]},{\"children\":[0]}]", 1), Kind::Invalid),
        ("a node its own child", base.replacen("{\"mesh\":0,", "{\"mesh\":0,\"children\":[0],", 1), Kind::Invalid),
        ("a node with two parents", base.replacen("\"nodes\":[{\"mesh\":0,\"translation\":[1,2,3]}]", "\"nodes\":[{\"mesh\":0,\"children\":[2]},{\"children\":[2]},{}]", 1), Kind::Invalid),
        ("a child not there", base.replacen("{\"mesh\":0,", "{\"mesh\":0,\"children\":[5],", 1), Kind::Invalid),
        ("a scene of a node that is not a root", base.replacen("\"nodes\":[{\"mesh\":0,\"translation\":[1,2,3]}]", "\"nodes\":[{\"mesh\":0,\"children\":[1]},{}]", 1).replacen("\"scenes\":[{\"nodes\":[0]}]", "\"scenes\":[{\"nodes\":[1]}]", 1), Kind::Invalid),
        ("the file's scene not there", base.replacen("\"scene\":0", "\"scene\":4", 1), Kind::Invalid),
        ("a translation of 1e999", base.replacen("[1,2,3]", "[1e999,2,3]", 1), Kind::Invalid),
        ("a translation as text", base.replacen("[1,2,3]", "[\"1\",2,3]", 1), Kind::Invalid),
        ("a rotation of length 0", base.replacen("\"translation\":[1,2,3]", "\"rotation\":[0,0,0,0]", 1), Kind::Invalid),
        ("a matrix whose last row is not 0 0 0 1", base.replacen("\"translation\":[1,2,3]", "\"matrix\":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,2]", 1), Kind::Invalid),
        ("Draco required", base.replacen("{\"asset\"", "{\"extensionsRequired\":[\"KHR_draco_mesh_compression\"],\"asset\"", 1), Kind::Unsupported),
        ("meshopt required", base.replacen("{\"asset\"", "{\"extensionsRequired\":[\"EXT_meshopt_compression\"],\"asset\"", 1), Kind::Unsupported),
        ("KTX2 / basisu required", base.replacen("{\"asset\"", "{\"extensionsRequired\":[\"KHR_texture_basisu\"],\"asset\"", 1), Kind::Unsupported),
        ("quantization required", base.replacen("{\"asset\"", "{\"extensionsRequired\":[\"KHR_mesh_quantization\"],\"asset\"", 1), Kind::Unsupported),
        ("glTF 1.0", base.replacen("\"version\":\"2.0\"", "\"version\":\"1.0\"", 1), Kind::Unsupported),
        ("no asset", base.replacen("\"asset\":{\"version\":\"2.0\"},", "", 1), Kind::Invalid),
        ("a buffer file not there", base.replacen("data:application/octet-stream;base64,", "missing.bin?", 1), Kind::Missing),
        ("a buffer one folder up", base.replacen("data:application/octet-stream;base64,", "../up/", 1), Kind::Unsupported),
        ("a buffer at an absolute path", base.replacen("data:application/octet-stream;base64,", "/root/", 1), Kind::Unsupported),
        ("a buffer on a drive", base.replacen("data:application/octet-stream;base64,", "C:/data/", 1), Kind::Unsupported),
        ("a buffer over http", base.replacen("data:application/octet-stream;base64,", "http://example.com/", 1), Kind::Unsupported),
        ("a broken %-escape", base.replacen("data:application/octet-stream;base64,", "a%zz", 1), Kind::Invalid),
        ("broken base64", base.replacen(";base64,", ";base64,*", 1), Kind::Invalid),
        ("a data URI without base64", base.replacen(";base64,", ",", 1), Kind::Unsupported),
        ("a buffer shorter than its byteLength", base.replacen("\"byteLength\":44", "\"byteLength\":48", 1), Kind::Invalid),
        ("more sparse values than elements", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":3,\"type\":\"VEC3\",\"sparse\":{\"count\":4,\"indices\":{\"bufferView\":1,\"componentType\":5123},\"values\":{\"bufferView\":0}}", 1), Kind::Invalid),
        ("sparse indices out of order", base.replacen("\"count\":3,\"type\":\"VEC3\"", "\"count\":3,\"type\":\"VEC3\",\"sparse\":{\"count\":2,\"indices\":{\"bufferView\":0,\"componentType\":5125},\"values\":{\"bufferView\":0}}", 1), Kind::Invalid),
        ("a texture of an image not there", base.replacen("{\"asset\"", "{\"textures\":[{\"source\":3}],\"asset\"", 1), Kind::Invalid),
        ("a material's texture not there", base.replacen("{\"asset\"", "{\"materials\":[{\"pbrMetallicRoughness\":{\"baseColorTexture\":{\"index\":2}}}],\"asset\"", 1), Kind::Invalid),
        ("alphaMode HALF", base.replacen("{\"asset\"", "{\"materials\":[{\"alphaMode\":\"HALF\"}],\"asset\"", 1), Kind::Invalid),
        ("a sampler's wrap 1234", base.replacen("{\"asset\"", "{\"samplers\":[{\"wrapS\":1234}],\"asset\"", 1), Kind::Invalid),
        ("an array, not an object", String::from("[1,2,3]"), Kind::Invalid),
        ("not JSON", String::from("a glTF? no"), Kind::Json),
        ("a key twice", base.replacen("{\"asset\"", "{\"scene\":0,\"asset\"", 1), Kind::Json),
    ];
    let mut wrong = Vec::new();
    match load(base.as_bytes(), &[], &lim) {
        Ok(d) => t.ok(d.meshes[0].primitives[0].positions.len() == 3 && d.nodes[0].world[12] == 1.0, "the small file loads: one triangle at (1, 2, 3)"),
        Err(e) => t.ok(false, &format!("the small file: {e}")),
    };
    for (name, json, want) in &cases {
        let got = load(json.as_bytes(), &[], &lim);
        match &got {
            Err(e) if kind(e) == *want => {}
            _ => wrong.push(format!("{name}: {:?}", got.as_ref().err())),
        }
    }
    t.ok(wrong.is_empty(), &format!("{} files with one rule broken: each the error it should be{}", cases.len(), if wrong.is_empty() { String::new() } else { format!(", not: {wrong:?}") }));
    if let Err(e) = load(cases[0].1.as_bytes(), &[], &lim) {
        out(&format!("    e.g. {e}"));
    }
    // images: a PNG claiming 70000 x 70000, a broken PNG, a GIF
    let img_case = |data: &[u8]| -> String {
        base.replacen("{\"asset\"", &format!("{{\"textures\":[{{\"source\":0}}],\"images\":[{{\"uri\":\"data:image/png;base64,{}\"}}],\"asset\"", g::base64_encode(data)), 1)
    };
    let small = vec![200u8; 4 * 4 * 4];
    let mut corrupt = gg::png(4, 4, &small);
    let l = corrupt.len();
    corrupt[l - 20] ^= 0xFF;
    let claims = gg::png_claiming(70000, 70000, 4, 4, &small);
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    let mut wrong = Vec::new();
    for (name, data, want) in [("a PNG claiming 70000 x 70000", claims, Kind::Limit), ("a PNG with a byte of its data changed", corrupt, Kind::Invalid), ("a GIF", gif, Kind::Unsupported)] {
        let got = load(img_case(&data).as_bytes(), &[], &lim);
        if !matches!(&got, Err(e) if kind(e) == want) {
            wrong.push(format!("{name}: {:?}", got.err()));
        }
    }
    t.ok(wrong.is_empty(), &format!("broken images: the PNG too large, broken, not PNG or JPEG are errors of their kind{}", if wrong.is_empty() { String::new() } else { format!(", not: {wrong:?}") }));
    // limits lowered
    let full = sc.write(Form::Glb).0;
    let mut wrong = Vec::new();
    let low = |f: &dyn Fn(&mut g::Limits)| { let mut l = g::Limits::DEFAULT; f(&mut l); l };
    let limits: [(&str, g::Limits); 8] = [
        ("nodes 10", low(&|l| l.nodes = 10)), ("depth 10", low(&|l| l.depth = 10)), ("instances 3", low(&|l| l.instances = 3)), ("image pixels 1000", low(&|l| l.image_pixels = 1000)),
        ("buffers 1000 bytes", low(&|l| l.buffer_bytes = 1000)), ("JSON 500 bytes", low(&|l| l.json_bytes = 500)), ("vertices 100", low(&|l| l.vertices = 100)), ("file 1000 bytes", low(&|l| l.file_bytes = 1000)),
    ];
    for (name, l) in &limits {
        let got = load(&full, &[], l);
        if !matches!(&got, Err(g::Error::Limit(_))) {
            wrong.push(format!("{name}: {:?}", got.err()));
        }
    }
    t.ok(wrong.is_empty(), &format!("{} limits lowered below the file: each a Limit error{}", limits.len(), if wrong.is_empty() { String::new() } else { format!(", not: {wrong:?}") }));
    // the .glb container
    let mut wrong = Vec::new();
    let mut cases: Vec<(&str, Vec<u8>, Kind)> = Vec::new();
    let mut v = full.clone();
    v[1] = b'L';
    t.ok(matches!(g::glb_chunks(&v), Err(g::Error::Invalid(_))) && matches!(load(&v, &[], &lim), Err(g::Error::Json(_))),
         "a .glb without its magic: glb_chunks says Invalid, load takes it for JSON and says so");
    let mut v = full.clone();
    v[4] = 1;
    cases.push(("version 1", v, Kind::Unsupported));
    let mut v = full.clone();
    v[8..12].copy_from_slice(&(full.len() as u32 + 4).to_le_bytes());
    cases.push(("the header longer than the file", v, Kind::Invalid));
    let mut v = full.clone();
    v[12..16].copy_from_slice(&(full.len() as u32).to_le_bytes());
    cases.push(("a chunk beyond the file", v, Kind::Invalid));
    let mut v = full.clone();
    v[16..20].copy_from_slice(&0x004E_4942u32.to_le_bytes());
    cases.push(("BIN first", v, Kind::Invalid));
    let json_len = u32::from_le_bytes([full[12], full[13], full[14], full[15]]) as usize;
    let mut v = full.clone();
    v[20 + json_len + 4..20 + json_len + 8].copy_from_slice(&0x4E4F_534Au32.to_le_bytes());
    cases.push(("a second JSON chunk", v, Kind::Invalid));
    for (name, data, want) in &cases {
        let got = load(data, &[], &lim);
        if !matches!(&got, Err(e) if kind(e) == *want) {
            wrong.push(format!("{name}: {:?}", got.err()));
        }
    }
    t.ok(wrong.is_empty(), &format!("{} broken .glb containers: errors of their kind{}", cases.len(), if wrong.is_empty() { String::new() } else { format!(", not: {wrong:?}") }));
    // strips, fans, points, a primitive without POSITION
    let strip = base.replacen("\"mode\":4", "\"mode\":5", 1).replacen("\"count\":3,\"type\":\"SCALAR\"", "\"count\":4,\"type\":\"SCALAR\"", 1).replacen("\"byteOffset\":36,\"byteLength\":6", "\"byteOffset\":36,\"byteLength\":8", 1);
    let fan = strip.replacen("\"mode\":5", "\"mode\":6", 1);
    let points = base.replacen("\"mode\":4", "\"mode\":0", 1);
    let nopos = base.replacen("\"POSITION\":0", "\"COLOR_0\":0", 1);
    // (no normals: flat ones, so the triangles' vertices come in order)
    let pos = |j: &str| load(j.as_bytes(), &[], &lim).ok().map(|d| d.meshes[0].primitives.first().map(|p| p.positions.clone()).unwrap_or_default());
    let (p0, p1, p2) = ([0.0f32, 0.0, 0.0], [1.0f32, 0.0, 0.0], [0.0f32, 1.0, 0.0]);
    t.ok(pos(&strip) == Some(vec![p0, p1, p2, p2, p1, p0]), &format!("a strip of 0 1 2 0: triangles 0 1 2, 2 1 0 ({:?})", pos(&strip)));
    t.ok(pos(&fan) == Some(vec![p0, p1, p2, p0, p2, p0]), &format!("a fan of 0 1 2 0: triangles 0 1 2, 0 2 0 ({:?})", pos(&fan)));
    let notes = |j: &str| load(j.as_bytes(), &[], &lim).map(|d| (d.meshes[0].primitives.len(), d.notes.len()));
    t.ok(matches!(notes(&points), Ok((0, n)) if n > 0), &format!("points are left out with a note: {:?}", notes(&points)));
    t.ok(matches!(notes(&nopos), Ok((0, n)) if n > 0), &format!("a primitive without POSITION is left out with notes: {:?}", notes(&nopos)));
}

// ---------------------------------------------------------------- drawn

/// the reference's sampling of a glTF sampler's codes
fn ref_sampler(s: [u32; 4]) -> RefSampling {
    let w = |c: u32| match c { 33071 => RWrap::Clamp, 33648 => RWrap::Mirror, _ => RWrap::Repeat };
    let (min_nearest, mip) = match s[1] {
        9728 => (true, 0),
        9729 => (false, 0),
        9984 => (true, 1),
        9985 => (false, 1),
        9986 => (true, 2),
        _ => (false, 2),
    };
    RefSampling { mag_nearest: s[0] == 9728, min_nearest, mip, wrap: [w(s[2]), w(s[3])] }
}

fn drawn(t: &mut T, dev: &Device, level: &str, sc: &GScene, doc: &g::Document) -> Option<()> {
    let (w, h) = (GALLERY_W, GALLERY_H);
    let mut off = t.res(Off::new(dev, w, h, Format::Rgba8Unorm), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new")?;
    let imp = t.res(render3d::gltf::upload(&mut r, doc, None), "gltf::upload")?;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let cam = gg::test_camera(w as f64 / h as f64);
    s.set_camera(&cam);
    let sun = sun_of();
    s.set_sun(&sun);
    for i in &imp.instances {
        t.res(s.add(*i), "Scene::add")?;
    }
    let img = shot(t, dev, &mut off, &mut r, &mut s, "glTF")?;
    // the reference from the description: textures at their class's size as this test makes them
    let mut rs = RefScene::new(sun.direction, sun.color.map(|v| v as f64), sun.ambient.map(|v| v as f64));
    // (each texture twice: sRGB at k for base colour and emission, linear at n + k for the others, as upload
    // puts an image used both ways up twice)
    let mut tex_of = Vec::new();
    let n = sc.texs.len();
    for srgb in [true, false] {
        for x in &sc.texs {
            let im = &sc.images[x.image];
            let px = if im.jpeg { img_jpeg::decode_fl(&gg::jpeg(im.w, im.h, &im.rgba), img_core::JDF_STRAIGHT).map(|sf| bgra_rows(&sf)).unwrap_or_default() } else { im.rgba.clone() };
            let size = render3d::CLASS_MIN << render3d::class_of(im.w, im.h);
            let reference = if im.w == size && im.h == size {
                px
            } else if x.sampler[0] == 9728 {
                crate::tex::nearest_resample(&px, im.w, im.h, size)
            } else {
                crate::tex::bilinear_resample(&px, im.w, im.h, size, srgb)
            };
            rs.textures.push(RefTexture::new(size, &reference, srgb));
            tex_of.push(ref_sampler(x.sampler));
        }
    }
    let worlds = sc.worlds();
    for (node, mesh, prim) in sc.instances() {
        let p = &sc.meshes[mesh][prim];
        let look = match p.material {
            Some(m) => {
                let gm = &sc.mats[m];
                let f = gm.factor;
                let srgb = |i: usize| gm.tex[i].map(|k| (k, tex_of[k]));
                let lin = |i: usize| gm.tex[i].map(|k| (n + k, tex_of[n + k]));
                let e = gm.emissive;
                Look { factor: [f[0] as f64, f[1] as f64, f[2] as f64, f[3] as f64], texture: srgb(0), cutoff: gm.mask.map(|c| c as f64), metallic: gm.metallic as f64,
                       roughness: gm.roughness as f64, mr_texture: lin(1), normal_texture: lin(2), normal_scale: 1.0, occlusion_texture: lin(3), occlusion: 1.0,
                       emissive: [e[0] as f64, e[1] as f64, e[2] as f64], emissive_texture: srgb(4) }
            }
            // (glTF's default material: white, metallic 1, roughness 1)
            None => Look { metallic: 1.0, ..Look::color([1.0, 1.0, 1.0]) },
        };
        rs.add_matrix(&expected(p), &worlds[node], look);
    }
    // (bilinear and trilinear textures in the scene: within 6, as in the texture tests)
    let c = cpu::compare_tol(&rs, &ref_camera(&cam), w, h, CLEAR8, &img, None, 6);
    t.ok(c.fine() && c.objects_seen as usize == imp.instances.len(), &format!("{level}: the scene drawn ({} instances, {} textures, a mirrored node, MASK) against the ray caster: {}", imp.instances.len(), imp.textures, c.text()));
    t.ok(imp.notes.is_empty() && imp.sheared == 0, &format!("{level}: upload's notes: {:?}", imp.notes));
    Some(())
}

/// a surface's b g r a rows as r g b a without padding
fn bgra_rows(sf: &img_core::Surface) -> Vec<u8> {
    let (w, h, st) = (sf.w as usize, sf.h as usize, sf.stride as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for p in sf.px[y * st..y * st + w * 4].chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out
}
