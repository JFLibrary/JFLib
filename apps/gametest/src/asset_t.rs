// asset_t -- (G7b part 2) the group "assets": lib/game's Assets step by step (Loading::Frames) and with threads
// (Background), the kinds of game_render3d (Model: glTF, Image: PNG), and the GPU side (Models, Textures,
// Bridge::sync_models) against a scene built directly from the documents' matrices.  The files lie in mem://
// (plat's VFS); the glTF files are made here (gltf::write_document, a .gltf written by hand), the PNG by img_png.
// Without Vulkan or a usable GPU the GPU part prints SKIP.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use game::assets::{sibling, Asset, AssetId, Assets, Bytes, Change, Loading, State};
use game::{Pose, Quat, Transform, Vec3, View, World};
use game_render3d::{Bridge, Image, Model, ModelRef, Models, Textures};
use gpu::{Access, Buffer, BufferDesc, BufferUsage, ColorAttachment, DataLayout, DepthAttachment, Device, DeviceDesc, Format, GpuError, Layout, Level, Load,
          MemoryKind, Texture, TextureDesc, TextureRegion, TextureUsage};
use render3d::{Camera, Renderer, RendererDesc, Scene};

use crate::{Ctx, out};

const DIR: &str = "mem://gametest/assets/";

fn path(name: &str) -> String {
    format!("{DIR}{name}")
}

fn put(name: &str, data: &[u8]) -> bool {
    plat::write_file(&path(name), data)
}

pub fn run(c: &mut Ctx) {
    siblings(c);
    frames(c);
    background(c);
    kinds(c);
    gpu_part(c);
}

fn siblings(c: &mut Ctx) {
    let cases: [(&str, &str, Option<&str>); 9] = [
        ("mem://a/b/model.gltf", "buf.bin", Some("mem://a/b/buf.bin")),
        ("mem://a/b/model.gltf", "tex/wood.png", Some("mem://a/b/tex/wood.png")),
        ("model.gltf", "buf.bin", Some("buf.bin")),
        ("C:\\games\\x.gltf", "y.bin", Some("C:\\games\\y.bin")),
        ("mem://a/model.gltf", "../secret.bin", None),
        ("mem://a/model.gltf", "/etc/passwd", None),
        ("mem://a/model.gltf", "app://other.bin", None),
        ("mem://a/model.gltf", "", None),
        ("mem://a/model.gltf", "x/../../y", None),
    ];
    for (p, rel, want) in cases {
        let got = sibling(p, rel);
        c.check(got.as_deref() == want, || format!("sibling({p}, {rel}) = {got:?}, not {want:?}"));
    }
}

/// a numbered file's bytes
fn data(i: usize) -> Vec<u8> {
    (0..(i * 37 % 300 + 1)).map(|k| (k * 7 + i * 13) as u8).collect()
}

fn frames(c: &mut Ctx) {
    for i in 0..8 {
        c.check(put(&format!("f{i}.bin"), &data(i)), || format!("mem:// f{i}.bin not written"));
    }
    let mut s: Assets<Bytes> = Assets::new(Loading::Frames { budget_us: 0 });
    let h = s.load(&path("f0.bin"));
    c.check(s.state(&h) == State::Loading && s.get(&h).is_none() && s.refs(&h) == 1 && s.len() == 1, || String::from("a load: Loading, no value, one reference"));
    let ch = s.update();
    c.check(ch == vec![Change::Ready(h.id())], || format!("the first update: {ch:?}"));
    c.check(s.state(&h) == State::Ready && s.get(&h).map(|b| &b.0[..]) == Some(&data(0)[..]), || String::from("Ready with the file's bytes"));
    // the same path: the same entry, another handle, no second read
    let h2 = s.load(&path("f0.bin"));
    c.check(h2 == h && s.refs(&h) == 2 && s.stats().decoded == 1, || format!("the same path: {:?} / {:?}, {} references", h2, h, s.refs(&h)));
    let ch = s.update();
    c.check(ch.is_empty() && s.stats().decoded == 1, || format!("nothing new: {ch:?}"));
    // budget 0: one decode per update, in the order asked
    let hs: Vec<_> = (1..6).map(|i| s.load(&path(&format!("f{i}.bin")))).collect();
    for (k, hk) in hs.iter().enumerate() {
        let ch = s.update();
        c.check(ch == vec![Change::Ready(hk.id())], || format!("budget 0, update {k}: {ch:?}"));
    }
    c.check(hs.iter().enumerate().all(|(k, hk)| s.get(hk).map(|b| &b.0[..]) == Some(&data(k + 1)[..])), || String::from("every file's bytes"));
    // a missing file fails, with its path in the error
    let miss = s.load(&path("none.bin"));
    let ch = s.update();
    c.check(ch == vec![Change::Failed(miss.id())] && s.state(&miss) == State::Failed && s.error(&miss).is_some_and(|e| e.contains("none.bin")),
            || format!("a missing file: {ch:?} {:?}", s.error(&miss)));
    // the last handle goes: unloaded at the next update, the id stale
    let id = h.id();
    drop(h);
    c.check(s.refs(&h2) == 1, || String::from("one handle left"));
    drop(h2);
    let ch = s.update();
    c.check(ch == vec![Change::Unloaded(id)] && s.state_of(id).is_none() && s.get_id(id).is_none(), || format!("unloaded: {ch:?}"));
    let again = s.load(&path("f0.bin"));
    c.check(again.id() != id && s.state(&again) == State::Loading, || format!("loaded again: a new entry ({:?} after {:?})", again.id(), id));
    let _ = s.update();
    c.check(s.get(&again).map(|b| &b.0[..]) == Some(&data(0)[..]), || String::from("loaded again: its bytes"));
    // linger: unreferenced, it stays two updates and is taken again without a read
    s.set_linger(2);
    let lid = again.id();
    let decoded = s.stats().decoded;
    drop(again);
    let ch1 = s.update();
    let ch2 = s.update();
    let back = s.load(&path("f0.bin"));
    c.check(ch1.is_empty() && ch2.is_empty() && back.id() == lid && s.stats().decoded == decoded && s.state(&back) == State::Ready,
            || format!("linger: {ch1:?} {ch2:?} {:?}/{:?}", back.id(), lid));
    drop(back);
    let n = (0..3).map(|_| s.update()).filter(|ch| ch.contains(&Change::Unloaded(lid))).count();
    c.check(n == 1 && s.state_of(lid).is_none(), || format!("linger 2: unloaded on the third update ({n})"));
    s.set_linger(0);
    // a handle dropped while it loads: never decoded
    let decoded = s.stats().decoded;
    let gone = s.load(&path("f6.bin"));
    let gid = gone.id();
    drop(gone);
    let ch = s.update();
    c.check(ch == vec![Change::Unloaded(gid)] && s.stats().decoded == decoded, || format!("dropped while loading: {ch:?}, decoded {}", s.stats().decoded - decoded));
    // made in code: Ready at once; its name is no path
    let made = s.insert("made in code", Bytes(vec![9, 9]));
    c.check(s.state(&made) == State::Ready && s.path(&made) == "made in code" && s.get(&made).map(|b| b.0.len()) == Some(2), || String::from("insert"));
    // reload: the file changed
    let r = s.load(&path("f7.bin"));
    let _ = s.update();
    let before = s.get(&r).map(|b| b.0.clone());
    c.check(put("f7.bin", b"changed"), || String::from("mem:// f7.bin not written again"));
    s.reload(&r);
    // (the value so far stays until the new one has come: it was gone at once)
    c.check(s.state(&r) == State::Loading && before.is_some() && s.get(&r).map(|b| b.0.clone()) == before, || format!("reload: Loading, the old value meanwhile: {:?}", s.get(&r)));
    let ch = s.update();
    c.check(ch == vec![Change::Ready(r.id())] && s.get(&r).map(|b| &b.0[..]) == Some(&b"changed"[..]), || format!("reload: {ch:?}"));
    // a reload that fails (the file went) keeps the value, Ready, without a change
    plat::delete(&path("f7.bin"));
    s.reload(&r);
    let ch = s.update();
    c.check(ch.is_empty() && s.state(&r) == State::Ready && s.get(&r).map(|b| &b.0[..]) == Some(&b"changed"[..]), || format!("a failed reload keeps the value: {ch:?} {:?} {:?}", s.state(&r), s.error(&r)));
    // stats
    let st = s.stats();
    c.check(st.loading == 0 && st.failed == 1 && st.ready as usize == s.len() - 1, || format!("stats {st:?}, {} entries", s.len()));
    drop(hs);
    drop(miss);
    drop(made);
    drop(r);
    let ch = s.update();
    c.check(ch.iter().all(|x| matches!(x, Change::Unloaded(_))) && s.is_empty(), || format!("all gone: {} left", s.len()));
}

fn background(c: &mut Ctx) {
    let n = c.n(60).max(6);
    for i in 0..n {
        put(&format!("g{i}.bin"), &data(i + 100));
    }
    let mut s: Assets<Bytes> = Assets::new(Loading::Background);
    let hs: Vec<_> = (0..n).map(|i| s.load(&path(&format!("g{i}.bin")))).collect();
    c.check(hs.iter().all(|h| s.state(h) == State::Loading), || String::from("Background: Loading until an update looks"));
    let mut ch = Vec::new();
    let idle = s.wait_idle(20_000_000, &mut ch);
    let ready = ch.iter().filter(|x| matches!(x, Change::Ready(_))).count();
    c.check(idle && ready == n, || format!("Background: {ready} of {n} Ready ({idle})"));
    c.check(hs.iter().enumerate().all(|(i, h)| s.get(h).map(|b| &b.0[..]) == Some(&data(i + 100)[..])), || String::from("Background: every file's bytes"));
    // every id once
    let mut ids: Vec<AssetId> = ch.iter().filter_map(|x| if let Change::Ready(id) = x { Some(*id) } else { None }).collect();
    ids.sort();
    ids.dedup();
    c.check(ids.len() == n, || format!("Background: {} different ids Ready", ids.len()));
    // dropped while their threads run: gone, and nothing reports them Ready afterwards
    let extra: Vec<_> = (0..n).map(|i| s.load(&path(&format!("g{i}.bin")).replace(".bin", ".bin"))).collect();
    c.check(extra.iter().zip(hs.iter()).all(|(a, b)| a == b), || String::from("Background: loads of paths that live are the same entries"));
    drop(extra);
    let fresh: Vec<AssetId> = (0..4).map(|i| {
        put(&format!("late{i}.bin"), &data(i));
        s.load(&path(&format!("late{i}.bin"))).id()
    }).collect();
    let ch = s.update();
    c.check(fresh.iter().all(|id| ch.contains(&Change::Unloaded(*id)) && s.state_of(*id).is_none()), || format!("Background: dropped while loading: {ch:?}"));
    plat::sleep_ms(50);
    let ch = s.update();
    c.check(!ch.iter().any(|x| matches!(x, Change::Ready(id) if fresh.contains(id))), || format!("Background: a cancelled one came back: {ch:?}"));
    drop(hs);
    let ch = s.update();
    c.check(ch.len() == n && s.is_empty(), || format!("Background: {} unloaded of {n}", ch.len()));
}

/// a cube of side 2 around 0 (24 vertices, 12 triangles)
fn cube_prim(material: Option<usize>) -> gltf::Primitive {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let mut p = gltf::Primitive { material, has_uvs: true, ..gltf::Primitive::default() };
    for (f, (nn, u, v)) in faces.iter().enumerate() {
        for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            p.positions.push([nn[0] + a * u[0] + b * v[0], nn[1] + a * u[1] + b * v[1], nn[2] + a * u[2] + b * v[2]]);
            p.normals.push(*nn);
            p.uvs.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        let f = f as u32 * 4;
        p.indices.extend_from_slice(&[f, f + 1, f + 2, f, f + 2, f + 3]);
    }
    p
}

/// a column-major matrix: translate t, turn about y by a, scale s
fn trs(t: [f64; 3], a: f64, s: f64) -> [f64; 16] {
    let (sn, cs) = physics::math::sincos(a);
    [cs * s, 0.0, -sn * s, 0.0, 0.0, s, 0.0, 0.0, sn * s, 0.0, cs * s, 0.0, t[0], t[1], t[2], 1.0]
}

/// a model of two cubes (two materials): a node at the origin, scaled 0.5, and one beside it, turned
fn doc_two_cubes() -> gltf::Document {
    let mut m0 = gltf::Material::default();
    m0.base_color_factor = [0.9, 0.3, 0.2, 1.0];
    let mut m1 = gltf::Material::default();
    m1.base_color_factor = [0.2, 0.5, 0.9, 1.0];
    let node = |mesh: usize, local: [f64; 16]| gltf::Node { name: String::new(), mesh: Some(mesh), children: Vec::new(), parent: None, local, world: local, light: None };
    gltf::Document {
        version: String::from("2.0"),
        generator: String::from("gametest"),
        meshes: vec![gltf::Mesh { name: String::from("a"), primitives: vec![cube_prim(Some(0))] }, gltf::Mesh { name: String::from("b"), primitives: vec![cube_prim(Some(1))] }],
        materials: vec![m0, m1],
        nodes: vec![node(0, trs([0.0, 0.5, 0.0], 0.0, 0.5)), node(1, trs([1.5, 0.75, 0.0], 0.6, 0.75))],
        scenes: vec![gltf::SceneDef { name: String::new(), nodes: vec![0, 1] }],
        scene: Some(0),
        ..gltf::Document::default()
    }
}

/// a model of one cube, unscaled (the second model of the GPU part)
fn doc_one_cube() -> gltf::Document {
    let mut m0 = gltf::Material::default();
    m0.base_color_factor = [0.3, 0.8, 0.3, 1.0];
    let local = trs([0.0, 1.0, 0.0], 0.3, 1.0);
    gltf::Document {
        version: String::from("2.0"),
        meshes: vec![gltf::Mesh { name: String::new(), primitives: vec![cube_prim(Some(0))] }],
        materials: vec![m0],
        nodes: vec![gltf::Node { name: String::new(), mesh: Some(0), children: Vec::new(), parent: None, local, world: local, light: None }],
        scenes: vec![gltf::SceneDef { name: String::new(), nodes: vec![0] }],
        scene: Some(0),
        ..gltf::Document::default()
    }
}

/// a .gltf with its buffer beside it: one triangle
fn tri_gltf() -> (String, Vec<u8>) {
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for x in v {
            bin.extend_from_slice(&x.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    bin.extend_from_slice(&[0, 0]);
    let json = String::from(concat!(
        "{\"asset\":{\"version\":\"2.0\"},\"buffers\":[{\"uri\":\"tri.bin\",\"byteLength\":44}],",
        "\"bufferViews\":[{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36},{\"buffer\":0,\"byteOffset\":36,\"byteLength\":6}],",
        "\"accessors\":[{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[0,0,0],\"max\":[1,1,0]},",
        "{\"bufferView\":1,\"componentType\":5123,\"count\":3,\"type\":\"SCALAR\"}],",
        "\"meshes\":[{\"primitives\":[{\"attributes\":{\"POSITION\":0},\"indices\":1}]}],",
        "\"nodes\":[{\"mesh\":0}],\"scenes\":[{\"nodes\":[0]}],\"scene\":0}"
    ));
    (json, bin)
}

/// a w x h PNG (RGB) and the rgba the decoder should give
fn png(w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
    let mut bgrx = Vec::new();
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = ((x * 40) as u8, (y * 50) as u8, ((x + y) * 20) as u8);
            bgrx.extend_from_slice(&[b, g, r, 0]);
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    (img_png::encode(&bgrx, w, h, w * 4, false), rgba)
}

fn kinds(c: &mut Ctx) {
    let doc = doc_two_cubes();
    let glb = gltf::write_document(&doc);
    c.check(glb.is_ok(), || format!("write_document: {:?}", glb.as_ref().err()));
    let Ok(glb) = glb else { return };
    put("two.glb", &glb);
    put("broken.glb", &glb[..glb.len() / 2]);
    let (json, bin) = tri_gltf();
    put("tri.gltf", json.as_bytes());
    put("tri.bin", &bin);
    put("escape.gltf", json.replace("tri.bin", "../tri.bin").as_bytes());
    let mut s: Assets<Model> = Assets::new(Loading::Frames { budget_us: 1_000_000 });
    let two = s.load(&path("two.glb"));
    let tri = s.load(&path("tri.gltf"));
    let broken = s.load(&path("broken.glb"));
    let escape = s.load(&path("escape.gltf"));
    let ch = s.update();
    c.check(ch.len() == 4, || format!("models: {ch:?}"));
    match s.get(&two) {
        Some(m) => {
            let inst = m.doc.instances(None);
            c.check(m.doc.meshes.len() == 2 && m.doc.meshes[0].primitives[0].positions == doc.meshes[0].primitives[0].positions && inst.len() == 2,
                    || format!("the .glb read back: {} meshes, {} instances", m.doc.meshes.len(), inst.len()));
        }
        None => {
            c.check(false, || format!("the .glb: {:?}", s.error(&two)));
        }
    }
    c.check(s.get(&tri).is_some_and(|m| m.doc.meshes.len() == 1 && m.doc.meshes[0].primitives[0].indices == [0, 1, 2] && m.doc.meshes[0].primitives[0].positions[2] == [0.0, 1.0, 0.0]),
            || format!("the .gltf with its buffer beside it: {:?}", s.error(&tri)));
    c.check(s.state(&broken) == State::Failed && s.state(&escape) == State::Failed, || format!("a broken .glb, a buffer outside its folder: {:?} / {:?}", s.error(&broken), s.error(&escape)));
    // images
    let (p, rgba) = png(7, 5);
    put("wood.png", &p);
    put("junk.png", b"not an image at all");
    let mut si: Assets<Image> = Assets::new(Loading::Frames { budget_us: 1_000_000 });
    let im = si.load(&path("wood.png"));
    let junk = si.load(&path("junk.png"));
    let _ = si.update();
    c.check(si.get(&im).is_some_and(|i| i.width == 7 && i.height == 5 && i.rgba == rgba), || format!("the PNG: {:?}", si.error(&im)));
    c.check(si.error(&junk).is_some_and(|e| e.contains("neither PNG nor JPEG")), || format!("junk: {:?}", si.error(&junk)));
    // the kinds decode as Asset::decode says, also without the store
    let direct = Image::decode(&p, &mut |_| None);
    c.check(direct.as_ref().is_ok_and(|i| i.rgba == rgba), || String::from("Image::decode"));
}

// ---------------------------------------------------------------- the GPU part

const W: u32 = 320;
const H: u32 = 180;

/// an offscreen target with the read-back of its image (as bridge.rs's)
struct Off {
    stride: u32,
    color: Texture,
    depth: Texture,
    rb: Buffer,
    used: bool,
}

impl Off {
    fn new(dev: &Device) -> gpu::Result<Off> {
        let stride = (W * 4).div_ceil(256) * 256;
        Ok(Off {
            stride,
            color: dev.create_texture(&TextureDesc::d2(W, H, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC))?,
            depth: dev.create_texture(&TextureDesc::d2(W, H, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED))?,
            rb: dev.create_buffer(&BufferDesc { size: stride as u64 * H as u64, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?,
            used: false,
        })
    }

    fn shot(&mut self, r: &mut Renderer, s: &mut Scene) -> gpu::Result<Vec<u8>> {
        s.prepare(r)?;
        let dev = r.device().clone();
        let mut e = dev.encoder()?;
        let color = ColorAttachment { view: self.color.view(), load: Load::Clear, clear: [0.2, 0.4, 0.6, 1.0], store: true,
                                      before: if self.used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
        let depth = DepthAttachment { view: self.depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: true,
                                      before: if self.used { Layout::Sampled } else { Layout::Undefined }, after: Layout::Sampled };
        r.record(&mut e, s, color, Some(depth))?;
        e.copy_texture_to_buffer(TextureRegion::mip(&self.color, 0), &self.rb, DataLayout { offset: 0, bytes_per_row: self.stride, rows_per_image: 0 });
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let id = dev.queue().submit([e.finish()?])?;
        s.submitted(id);
        self.used = true;
        if !dev.wait(id, 10_000_000_000)? {
            return Err(GpuError::Invalid("the GPU did not finish within 10 s"));
        }
        let mut raw = vec![0u8; self.rb.size() as usize];
        self.rb.read(0, &mut raw)?;
        let mut img = Vec::with_capacity((W * H * 4) as usize);
        for y in 0..H as usize {
            img.extend_from_slice(&raw[y * self.stride as usize..y * self.stride as usize + (W * 4) as usize]);
        }
        Ok(img)
    }
}

fn camera() -> Camera {
    let v = View::look_at(Vec3::new(2.0, 5.0, 12.0), Vec3::new(0.0, 0.5, 0.0), Vec3::Y).expect("view");
    game_render3d::camera(&v, 1.0, W as f64 / H as f64, 0.1, f64::INFINITY)
}

fn mat_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    m
}

/// an entity's transform as a matrix (a quaternion turned into its rotation, the scale per axis)
fn matrix(t: &Transform) -> [f64; 16] {
    let m = t.rot.to_mat3();
    let s = [t.scale[0] as f64, t.scale[1] as f64, t.scale[2] as f64];
    let (x, y, z) = (m.cols[0], m.cols[1], m.cols[2]);
    [x.x * s[0], x.y * s[0], x.z * s[0], 0.0, y.x * s[1], y.y * s[1], y.z * s[1], 0.0, z.x * s[2], z.y * s[2], z.z * s[2], 0.0, t.pos.x, t.pos.y, t.pos.z, 1.0]
}

/// the reference: every entity with a ModelRef whose model is up, each part at (entity matrix) x (node's world
/// matrix), taken apart by gltf::decompose (independent of the bridge's compose)
fn reference(r: &Renderer, w: &World, models: &Models, assets: &Assets<Model>, cam: &Camera) -> gpu::Result<Scene> {
    let mut s = Scene::new(r)?;
    if let Some(col) = w.column::<ModelRef>() {
        for (e, m) in col.iter() {
            let (Some(parts), Some(model)) = (models.parts(m.model.id()), assets.get(&m.model)) else { continue };
            let et = w.get::<Pose>(e).map_or(Transform::IDENTITY, |p| p.now);
            for (part, inst) in parts.iter().zip(model.doc.instances(None).iter()) {
                let trs = gltf::decompose(&mat_mul(&matrix(&et), &inst.world));
                let q = trs.rotation;
                let t = render3d::Transform { pos: trs.translation, rot: [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32], scale: trs.scale.map(|v| v as f32) };
                s.add(render3d::Instance { mesh: part.mesh, material: part.material, transform: t })?;
            }
        }
    }
    s.set_camera(cam);
    Ok(s)
}

/// pixels whose channels differ by more than 1, and the drawn ones
fn diff(a: &[u8], b: &[u8]) -> (u32, u32) {
    let d = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 1)).count() as u32;
    // (drawn: other than the clear colour, which the top left corner shows)
    let sky = [a[0], a[1], a[2]];
    let drawn = a.chunks_exact(4).filter(|p| p[..3] != sky).count() as u32;
    (d, drawn)
}

#[allow(clippy::too_many_arguments)]
fn same(c: &mut Ctx, off: &mut Off, r: &mut Renderer, s: &mut Scene, w: &World, models: &Models, assets: &Assets<Model>, what: &str, want_drawn: bool) {
    let cam = *s.camera();
    let a = off.shot(r, s);
    let b = reference(r, w, models, assets, &cam).and_then(|mut rs| off.shot(r, &mut rs));
    match (a, b) {
        (Ok(a), Ok(b)) => {
            let (d, drawn) = diff(&a, &b);
            c.check(d == 0 && (drawn > 300) == want_drawn, || format!("{what}: {d} pixels differ from the scene built from the matrices ({drawn} drawn)"));
        }
        (a, b) => {
            c.check(false, || format!("{what}: {:?} / {:?}", a.err(), b.err()));
        }
    }
}

fn gpu_part(c: &mut Ctx) {
    let inst = match gpu::Instance::with(&gpu::InstanceDesc { app_name: "gametest", validate: false }) {
        Ok(i) => i,
        Err(e) => {
            out(&format!("  assets: GPU part SKIP: {e}\n"));
            return;
        }
    };
    let Ok(adapter) = inst.adapter() else {
        out("  assets: GPU part SKIP: no adapter\n");
        return;
    };
    let level = if adapter.level() == Level::Voll { Level::Voll } else { Level::Basis };
    let r = adapter.create_device(&DeviceDesc { max_level: level }).and_then(|dev| gpu_tests(c, &dev));
    if let Err(e) = r {
        c.check(false, || format!("assets, GPU: {e}"));
    }
}

fn gpu_tests(c: &mut Ctx, dev: &Device) -> gpu::Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm))?;
    let mut off = Off::new(dev)?;
    let glb2 = gltf::write_document(&doc_two_cubes()).map_err(|_| GpuError::Invalid("write_document"))?;
    let glb1 = gltf::write_document(&doc_one_cube()).map_err(|_| GpuError::Invalid("write_document"))?;
    put("gpu_two.glb", &glb2);
    put("gpu_one.glb", &glb1);
    let mut assets: Assets<Model> = Assets::new(Loading::Background);
    let two = assets.load(&path("gpu_two.glb"));
    let one = assets.load(&path("gpu_one.glb"));
    let lost = assets.load(&path("gpu_none.glb"));
    let mut models = Models::new();
    let mut bridge = Bridge::new();
    let mut w = World::new();
    let ents: Vec<_> = (0..3)
        .map(|k| {
            let t = Transform::new(Vec3::new(-3.0 + 3.0 * k as f64, 0.0, -1.0 * k as f64), Quat::from_axis_angle(Vec3::Y, 0.4 * k as f64)).scaled([1.0 + 0.25 * k as f32; 3]);
            w.spawn_with((Pose::new(t), ModelRef { model: two.clone() }))
        })
        .collect();
    let with_lost = w.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 3.0)), ModelRef { model: lost.clone() }));
    let mut s = Scene::new(&r)?;
    s.set_camera(&camera());
    // before the models are up: nothing drawn
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.added == 0 && bridge.model_entities() == 0, || format!("before the model is up: {st:?}"));
    // up: three entities, two parts each; the one with the missing model shows nothing
    let mut ch = Vec::new();
    let idle = assets.wait_idle(20_000_000, &mut ch);
    let ms = models.update(&assets, &ch, &mut r)?;
    c.check(idle && ms.uploaded == 2 && ms.parts == 3 && assets.state(&lost) == State::Failed, || format!("models up: {ms:?}, {ch:?}"));
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.added == 6 && bridge.model_entities() == 3, || format!("entities get their meshes: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, &models, &assets, "three entities with the model of two cubes", true);
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.added == 0 && st.moved == 0 && st.removed == 0, || format!("nothing changed, nothing written: {st:?}"));
    // a pose moves: both parts of that entity
    if let Some(mut p) = w.get_mut::<Pose>(ents[1]) {
        p.teleport(Transform::new(Vec3::new(0.5, 1.0, 1.0), Quat::from_axis_angle(Vec3::Y, -0.7)).scaled([0.8; 3]));
    }
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.moved == 2 && st.added == 0, || format!("a pose moved: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, &models, &assets, "after the move", true);
    // another model: the entity's instances switch
    if let Some(mut m) = w.get_mut::<ModelRef>(ents[2]) {
        m.model = one.clone();
    }
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.removed == 2 && st.added == 1, || format!("another model: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, &models, &assets, "one entity with the other model", true);
    // despawned: its instances go
    w.despawn(ents[0]);
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    c.check(st.removed == 2 && bridge.model_entities() == 2, || format!("despawned: {st:?}"));
    same(c, &mut off, &mut r, &mut s, &w, &models, &assets, "after the despawn", true);
    // the model of two cubes: its last users go, it is unloaded, its meshes leave the renderer
    let made: Vec<_> = models.made(two.id()).map(|m| m.0.to_vec()).unwrap_or_default();
    w.despawn(ents[1]);
    w.despawn(with_lost);
    drop(two);
    drop(lost);
    let st = bridge.sync_models(&w, &mut s, 1.0, &models)?;
    let ch = assets.update();
    let ms = models.update(&assets, &ch, &mut r)?;
    c.check(st.removed == 2 && ms.removed == 1 && ms.models == 1 && made.len() == 2 && made.iter().all(|m| r.mesh_info(*m).is_none()),
            || format!("unloaded: {st:?} {ms:?} {ch:?}"));
    // a lost device: a new renderer, everything up again
    let mut r2 = Renderer::new(dev, &RendererDesc::new(Format::Rgba8Unorm))?;
    models.reset();
    bridge.reset();
    let mut s2 = Scene::new(&r2)?;
    s2.set_camera(&camera());
    let ms = models.update(&assets, &[], &mut r2)?;
    let st = bridge.sync_models(&w, &mut s2, 1.0, &models)?;
    c.check(ms.uploaded == 1 && st.added == 1, || format!("a new renderer: {ms:?} {st:?}"));
    same(c, &mut off, &mut r2, &mut s2, &w, &models, &assets, "on a new renderer", true);
    // clear: the bridge's instances go
    bridge.clear(&mut s2)?;
    same(c, &mut off, &mut r2, &mut s2, &World::new(), &models, &assets, "cleared", false);
    // images: up as textures, off when unloaded
    let (p, _) = png(16, 8);
    put("gpu_tex.png", &p);
    let mut images: Assets<Image> = Assets::new(Loading::Frames { budget_us: 1_000_000 });
    let mut tex = Textures::new();
    let im = images.load(&path("gpu_tex.png"));
    let ch = images.update();
    let n = tex.update(&images, &ch, &mut r2)?;
    let id = tex.texture(im.id());
    c.check(n == 1 && id.is_some_and(|t| r2.texture_info(t).is_some()), || format!("an image as a texture: {n}, {id:?}"));
    let iid = im.id();
    drop(im);
    let ch = images.update();
    let _ = tex.update(&images, &ch, &mut r2)?;
    c.check(tex.texture(iid).is_none() && id.is_some_and(|t| r2.texture_info(t).is_none()), || String::from("the image unloaded: its texture gone"));
    // an image unloaded while a material of the app still names its texture: the renderer refuses the removal then;
    // once the material went, the next update takes the texture off (it stayed for good)
    let im = images.load(&path("gpu_tex.png"));
    let ch = images.update();
    let _ = tex.update(&images, &ch, &mut r2)?;
    let named = tex.texture(im.id());
    let mat = match named {
        Some(t) => Some(r2.add_material(&render3d::Material { base_color_texture: Some(render3d::TextureRef { texture: t, sampling: render3d::Sampling::LINEAR }), ..render3d::Material::color(1.0, 1.0, 1.0) })?),
        None => None,
    };
    drop(im);
    let ch = images.update();
    let _ = tex.update(&images, &ch, &mut r2)?;
    let refused = named.is_some_and(|t| r2.texture_info(t).is_some());
    if let Some(m) = mat {
        r2.remove_material(m)?;
    }
    let _ = tex.update(&images, &[], &mut r2)?;
    c.check(mat.is_some() && refused && named.is_some_and(|t| r2.texture_info(t).is_none()), || format!("a texture a material still named comes off after the material: {named:?}, {mat:?}, refused {refused}"));
    // the documented order (Assets::update, Models::update, then sync_models): the update that unloads a model finds
    // the instances of its last entity still on the scene, which refuses the removal of its meshes and materials; the
    // next update takes them off (they stayed on the renderer for good)
    drop(w);
    let mut w3 = World::new();
    let e3 = w3.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 0.0)), ModelRef { model: one.clone() }));
    let mut b3 = Bridge::new();
    let mut s3 = Scene::new(&r2)?;
    let st = b3.sync_models(&w3, &mut s3, 1.0, &models)?;
    let (meshes, mats) = models.made(one.id()).map(|m| (m.0.to_vec(), m.1.len() as u32)).unwrap_or_default();
    let mc = r2.material_count();
    w3.despawn(e3);
    drop(one);
    let ch = assets.update();
    let ms = models.update(&assets, &ch, &mut r2)?;
    let st2 = b3.sync_models(&w3, &mut s3, 1.0, &models)?;
    let ch2 = assets.update();
    let _ = models.update(&assets, &ch2, &mut r2)?;
    let gone = !meshes.is_empty() && meshes.iter().all(|m| r2.mesh_info(*m).is_none()) && r2.material_count() + mats == mc;
    c.check(st.added == 1 && ms.removed == 1 && st2.removed == 1 && gone,
            || format!("unloaded in the documented order: its meshes and materials off the renderer a frame later: {st:?} {ms:?} {st2:?}, {} of {} meshes left, materials {} of {mc}",
                       meshes.iter().filter(|m| r2.mesh_info(**m).is_some()).count(), meshes.len(), r2.material_count()));
    sync_refused(c, &mut r2)
}

/// an entity render3d refuses (a removed mesh, a pose that is not finite) ended Bridge::sync there, and the next sync
/// ran into it again: the entities after it were never added or moved.  Now it is left out and tried again
fn sync_refused(c: &mut Ctx, r: &mut Renderer) -> gpu::Result<()> {
    use game_render3d::MeshRef;
    use render3d::{Material, MeshData};
    let p = cube_prim(None);
    let data = MeshData { positions: &p.positions, normals: &p.normals, uvs: &p.uvs, indices: &p.indices };
    let (mesh, gone) = (r.add_mesh(&data)?, r.add_mesh(&data)?);
    r.remove_mesh(gone)?;
    let material = r.add_material(&Material::color(0.5, 0.5, 0.5))?;
    let mut w = World::new();
    let a = w.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 0.0)), MeshRef { mesh: gone, material }));
    let b = w.spawn_with((Pose::at(Vec3::new(1.0, 0.0, 0.0)), MeshRef { mesh, material }));
    let nan = Transform::new(Vec3::new(f64::NAN, 0.0, 0.0), Quat::IDENTITY);
    let cc = w.spawn_with((Pose::new(nan), MeshRef { mesh, material }));
    let mut bridge = Bridge::new();
    let mut s = Scene::new(r)?;
    let first = bridge.sync(&w, &mut s, 1.0);
    let b_in = bridge.instance(b).is_some();
    if let Some(mut p) = w.get_mut::<Pose>(b) {
        p.teleport(Transform::new(Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY));
    }
    if let Some(mut p) = w.get_mut::<Pose>(cc) {
        p.teleport(Transform::new(Vec3::new(3.0, 0.0, 0.0), Quat::IDENTITY));
    }
    let second = bridge.sync(&w, &mut s, 1.0);
    c.check(b_in && first.as_ref().is_ok_and(|st| st.added == 1 && st.failed == 2) && bridge.instance(a).is_none(),
            || format!("a refused entity is left out, the others are added: {first:?}, b {b_in}"));
    c.check(second.as_ref().is_ok_and(|st| st.moved >= 1 && st.added == 1 && st.failed == 1) && bridge.instance(cc).is_some(),
            || format!("the next sync moves the others and adds the one whose pose became finite: {second:?}"));
    Ok(())
}
