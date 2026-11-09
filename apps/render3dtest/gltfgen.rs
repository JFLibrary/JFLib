// gltfgen -- glTF files written by the tests themselves (nothing is downloaded): a scene described in Rust, written
// as .gltf with external files, as .gltf with data URIs and as .glb, with PNGs of its own (stored deflate
// blocks, its own CRC-32 and Adler-32) and JPEGs from img_jpeg.  The description is what the tests expect
// the importer to read: geometry as written, materials, textures, samplers, nodes whose world matrices the
// tests compose on their own.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::common::{Geo, Rng};

// ---------------------------------------------------------------- PNG

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let at = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let c = crc32(&out[at..]);
    out.extend_from_slice(&c.to_be_bytes());
}

/// a PNG of rgba rows (colour type 6, 8 bits), zlib with stored blocks; w and h as the header says (a test
/// may claim another size than the data has)
pub fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    png_claiming(w, h, w, h, rgba)
}

pub fn png_claiming(claim_w: u32, claim_h: u32, w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((h * (w * 4 + 1)) as usize);
    for y in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * w as usize * 4..(y + 1) * w as usize * 4]);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&claim_w.to_be_bytes());
    ihdr.extend_from_slice(&claim_h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// a JPEG of rgba rows (alpha dropped) at quality 95, by img_jpeg
pub fn jpeg(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut sf = img_core::Surface::new(w as i32, h as i32, gfx::FMT_XRGB);
    let st = sf.stride as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = (y * w as usize + x) * 4;
            sf.px[y * st + x * 4..y * st + x * 4 + 4].copy_from_slice(&[rgba[s + 2], rgba[s + 1], rgba[s], 255]);
        }
    }
    img_jpeg::encode(&sf, 95).unwrap_or_default()
}

/// smooth colours with some noise: w x h rgba, alpha from `alpha`
pub fn picture(w: u32, h: u32, seed: u32, alpha: &dyn Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut r = Rng(seed);
    let (a, b, c) = (r.range(0.5, 3.0), r.range(0.5, 3.0), r.range(0.0, 6.0));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (x as f64 / w as f64, y as f64 / h as f64);
            let (s1, _) = crate::cpu::sincos(u * a * 6.28 + c);
            let (s2, c2) = crate::cpu::sincos(v * b * 6.28 - c);
            let n = r.range(-24.0, 24.0);
            let ch = |base: f64| (base + n).clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&[ch(128.0 + 100.0 * s1), ch(128.0 + 100.0 * s2), ch(128.0 + 90.0 * c2 * s1), alpha(x, y)]);
        }
    }
    out
}

// ---------------------------------------------------------------- a scene in Rust

/// how a primitive is written
#[derive(Clone, Copy, Debug)]
pub struct Enc {
    /// index component type (None: not indexed)
    pub idx: Option<u32>,
    /// uv: float, or normalized u8 / u16; None: no TEXCOORD_0
    pub uv: Option<u32>,
    pub normals: bool,
    /// position, normal, uv in one view with a byte stride (float uv only)
    pub interleaved: bool,
    /// a sparse accessor moves this vertex of POSITION there
    pub sparse: Option<(u32, [f32; 3])>,
}

pub struct GPrim {
    pub geo: Geo,
    pub enc: Enc,
    pub material: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct GMat {
    pub factor: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    /// textures: base colour, metallic-roughness, normal, occlusion, emissive
    pub tex: [Option<usize>; 5],
    pub mask: Option<f32>,
}

/// a texture: an image and the sampler's glTF codes (mag, min, wrapS, wrapT)
#[derive(Clone, Copy, Debug)]
pub struct GTex {
    pub image: usize,
    pub sampler: [u32; 4],
}

pub struct GImage {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub jpeg: bool,
}

#[derive(Clone, Debug)]
pub struct GNode {
    pub t: [f64; 3],
    /// a unit quaternion x, y, z, w
    pub r: [f64; 4],
    pub s: [f64; 3],
    /// written as a matrix instead of TRS
    pub matrix: bool,
    pub mesh: Option<usize>,
    pub children: Vec<usize>,
}

pub struct GScene {
    pub meshes: Vec<Vec<GPrim>>,
    pub mats: Vec<GMat>,
    pub texs: Vec<GTex>,
    pub images: Vec<GImage>,
    pub nodes: Vec<GNode>,
    pub scenes: Vec<Vec<usize>>,
    pub scene: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// .gltf, a .bin and images beside it
    External,
    /// .gltf, everything in data URIs
    DataUri,
    /// .glb, the images in buffer views
    Glb,
}

/// T * R * S, column-major, from the definition
pub fn trs(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> [f64; 16] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    // the columns of R are R applied to the axes
    let col = |e: [f64; 3]| {
        // v + 2 u x (u x v + w v)
        let u = [x, y, z];
        let cr = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let inner = cr(u, e);
        let inner = [inner[0] + w * e[0], inner[1] + w * e[1], inner[2] + w * e[2]];
        let o = cr(u, inner);
        [e[0] + 2.0 * o[0], e[1] + 2.0 * o[1], e[2] + 2.0 * o[2]]
    };
    let (c0, c1, c2) = (col([1.0, 0.0, 0.0]), col([0.0, 1.0, 0.0]), col([0.0, 0.0, 1.0]));
    [c0[0] * s[0], c0[1] * s[0], c0[2] * s[0], 0.0, c1[0] * s[1], c1[1] * s[1], c1[2] * s[1], 0.0, c2[0] * s[2], c2[1] * s[2], c2[2] * s[2], 0.0,
     t[0], t[1], t[2], 1.0]
}

pub fn mul4(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            m[c * 4 + r] = s;
        }
    }
    m
}

/// a JSON number that reads back as the same f64 (Rust's shortest round trip)
fn num(v: f64) -> String {
    let s = format!("{v:?}");
    s.replace("inf", "1e999")
}

fn f32n(v: f32) -> String {
    num(v as f64)
}

/// the buffer under construction: bytes, views, accessors as JSON
struct Out {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

impl Out {
    fn align(&mut self) {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
    }

    fn view(&mut self, bytes: &[u8], stride: Option<u32>) -> usize {
        self.align();
        let at = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let s = stride.map_or(String::new(), |s| format!(",\"byteStride\":{s}"));
        self.views.push(format!("{{\"buffer\":0,\"byteOffset\":{at},\"byteLength\":{}{s}}}", bytes.len()));
        self.views.len() - 1
    }

    fn accessor(&mut self, view: usize, offset: usize, ctype: u32, normalized: bool, count: usize, kind: &str, extra: &str) -> usize {
        let n = if normalized { ",\"normalized\":true" } else { "" };
        self.accessors.push(format!("{{\"bufferView\":{view},\"byteOffset\":{offset},\"componentType\":{ctype}{n},\"count\":{count},\"type\":\"{kind}\"{extra}}}"));
        self.accessors.len() - 1
    }
}

fn minmax(p: &[[f32; 3]]) -> String {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in p {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    format!(",\"min\":[{},{},{}],\"max\":[{},{},{}]", f32n(lo[0]), f32n(lo[1]), f32n(lo[2]), f32n(hi[0]), f32n(hi[1]), f32n(hi[2]))
}

/// uv as written (normalized integers: rounded) and as the importer must read it back
pub fn uv_as_read(uv: [f32; 2], ctype: u32) -> [f32; 2] {
    match ctype {
        5121 => uv.map(|v| ((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8) as f32 / 255.0),
        5123 => uv.map(|v| ((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16) as f32 / 65535.0),
        _ => uv,
    }
}

impl GScene {
    /// the main file and the files beside it
    pub fn write(&self, form: Form) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        let mut o = Out { bin: Vec::new(), views: Vec::new(), accessors: Vec::new() };
        let mut meshes = Vec::new();
        for m in &self.meshes {
            let mut prims = Vec::new();
            for p in m {
                let g = &p.geo;
                let n = g.pos.len();
                let mut attrs = Vec::new();
                let fl = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
                let sparse = |o: &mut Out, s: Option<(u32, [f32; 3])>| -> String {
                    match s {
                        None => String::new(),
                        Some((i, v)) => {
                            let iv = o.view(&i.to_le_bytes(), None);
                            let vv = o.view(&fl(&v), None);
                            format!(",\"sparse\":{{\"count\":1,\"indices\":{{\"bufferView\":{iv},\"componentType\":5125}},\"values\":{{\"bufferView\":{vv}}}}}")
                        }
                    }
                };
                // (min and max of the positions as read, the sparse vertex included)
                let mut read_pos = g.pos.clone();
                if let Some((i, v)) = p.enc.sparse {
                    read_pos[i as usize] = v;
                }
                if p.enc.interleaved {
                    let mut bytes = Vec::with_capacity(n * 32);
                    for i in 0..n {
                        bytes.extend_from_slice(&fl(&g.pos[i]));
                        bytes.extend_from_slice(&fl(&g.nrm[i]));
                        bytes.extend_from_slice(&fl(&g.uv[i]));
                    }
                    let v = o.view(&bytes, Some(32));
                    let sp = sparse(&mut o, p.enc.sparse);
                    let a = o.accessor(v, 0, 5126, false, n, "VEC3", &format!("{}{sp}", minmax(&read_pos)));
                    attrs.push(format!("\"POSITION\":{a}"));
                    let a = o.accessor(v, 12, 5126, false, n, "VEC3", "");
                    attrs.push(format!("\"NORMAL\":{a}"));
                    let a = o.accessor(v, 24, 5126, false, n, "VEC2", "");
                    attrs.push(format!("\"TEXCOORD_0\":{a}"));
                } else {
                    let pv = o.view(&g.pos.iter().flat_map(|p| fl(p)).collect::<Vec<u8>>(), None);
                    let sp = sparse(&mut o, p.enc.sparse);
                    let a = o.accessor(pv, 0, 5126, false, n, "VEC3", &format!("{}{sp}", minmax(&read_pos)));
                    attrs.push(format!("\"POSITION\":{a}"));
                    if p.enc.normals {
                        let nv = o.view(&g.nrm.iter().flat_map(|p| fl(p)).collect::<Vec<u8>>(), None);
                        let a = o.accessor(nv, 0, 5126, false, n, "VEC3", "");
                        attrs.push(format!("\"NORMAL\":{a}"));
                    }
                    if let Some(ct) = p.enc.uv {
                        // a stride of 8 for normalized integers too: each element on its own 4 bytes
                        let mut bytes = Vec::with_capacity(n * 8);
                        for uv in &g.uv {
                            match ct {
                                5121 => {
                                    let q = uv.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                                    bytes.extend_from_slice(&[q[0], q[1], 0, 0, 0, 0, 0, 0]);
                                }
                                5123 => {
                                    let q = uv.map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
                                    bytes.extend_from_slice(&q[0].to_le_bytes());
                                    bytes.extend_from_slice(&q[1].to_le_bytes());
                                    bytes.extend_from_slice(&[0, 0, 0, 0]);
                                }
                                _ => bytes.extend_from_slice(&fl(uv)),
                            }
                        }
                        let v = o.view(&bytes, Some(8));
                        let a = o.accessor(v, 0, ct, ct != 5126, n, "VEC2", "");
                        attrs.push(format!("\"TEXCOORD_0\":{a}"));
                    }
                }
                let mut prim = format!("{{\"attributes\":{{{}}}", attrs.join(","));
                if let Some(ct) = p.enc.idx {
                    let bytes: Vec<u8> = match ct {
                        5121 => g.idx.iter().map(|&i| i as u8).collect(),
                        5123 => g.idx.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect(),
                        _ => g.idx.iter().flat_map(|&i| i.to_le_bytes()).collect(),
                    };
                    let v = o.view(&bytes, None);
                    let a = o.accessor(v, 0, ct, false, g.idx.len(), "SCALAR", "");
                    prim.push_str(&format!(",\"indices\":{a}"));
                }
                if let Some(m) = p.material {
                    prim.push_str(&format!(",\"material\":{m}"));
                }
                prim.push('}');
                prims.push(prim);
            }
            meshes.push(format!("{{\"primitives\":[{}]}}", prims.join(",")));
        }
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut images = Vec::new();
        for (i, im) in self.images.iter().enumerate() {
            let data = if im.jpeg { jpeg(im.w, im.h, &im.rgba) } else { png(im.w, im.h, &im.rgba) };
            let (mime, ext) = if im.jpeg { ("image/jpeg", "jpg") } else { ("image/png", "png") };
            images.push(match form {
                Form::External => {
                    // a space and a percent-escape in the name: the URI is decoded
                    let name = format!("tex {i}.{ext}");
                    files.push((name.clone(), data));
                    format!("{{\"name\":\"image{i}\",\"uri\":\"tex%20{i}.{ext}\"}}")
                }
                Form::DataUri => format!("{{\"name\":\"image{i}\",\"uri\":\"data:{mime};base64,{}\"}}", gltf::base64_encode(&data)),
                Form::Glb => {
                    let v = o.view(&data, None);
                    format!("{{\"name\":\"image{i}\",\"bufferView\":{v},\"mimeType\":\"{mime}\"}}")
                }
            });
        }
        let samplers: Vec<String> = self.texs.iter().map(|t| format!("{{\"magFilter\":{},\"minFilter\":{},\"wrapS\":{},\"wrapT\":{}}}", t.sampler[0], t.sampler[1], t.sampler[2], t.sampler[3])).collect();
        let textures: Vec<String> = self.texs.iter().enumerate().map(|(i, t)| format!("{{\"sampler\":{i},\"source\":{}}}", t.image)).collect();
        let mats: Vec<String> = self.mats.iter().map(|m| {
            let r = |k: usize| m.tex[k].map(|t| format!("{{\"index\":{t}}}"));
            let mut pbr = format!("\"baseColorFactor\":[{},{},{},{}],\"metallicFactor\":{},\"roughnessFactor\":{}", f32n(m.factor[0]), f32n(m.factor[1]), f32n(m.factor[2]),
                                  f32n(m.factor[3]), f32n(m.metallic), f32n(m.roughness));
            if let Some(t) = r(0) {
                pbr.push_str(&format!(",\"baseColorTexture\":{t}"));
            }
            if let Some(t) = r(1) {
                pbr.push_str(&format!(",\"metallicRoughnessTexture\":{t}"));
            }
            let mut s = format!("{{\"pbrMetallicRoughness\":{{{pbr}}},\"emissiveFactor\":[{},{},{}]", f32n(m.emissive[0]), f32n(m.emissive[1]), f32n(m.emissive[2]));
            for (k, key) in [(2, "normalTexture"), (3, "occlusionTexture"), (4, "emissiveTexture")] {
                if let Some(t) = r(k) {
                    s.push_str(&format!(",\"{key}\":{t}"));
                }
            }
            if let Some(c) = m.mask {
                s.push_str(&format!(",\"alphaMode\":\"MASK\",\"alphaCutoff\":{}", f32n(c)));
            }
            s.push('}');
            s
        }).collect();
        let nodes: Vec<String> = self.nodes.iter().map(|n| {
            let mut s = String::from("{");
            let mut parts = Vec::new();
            if n.matrix {
                let m = trs(n.t, n.r, n.s);
                parts.push(format!("\"matrix\":[{}]", m.iter().map(|v| num(*v)).collect::<Vec<_>>().join(",")));
            } else {
                parts.push(format!("\"translation\":[{},{},{}]", num(n.t[0]), num(n.t[1]), num(n.t[2])));
                parts.push(format!("\"rotation\":[{},{},{},{}]", num(n.r[0]), num(n.r[1]), num(n.r[2]), num(n.r[3])));
                parts.push(format!("\"scale\":[{},{},{}]", num(n.s[0]), num(n.s[1]), num(n.s[2])));
            }
            if let Some(m) = n.mesh {
                parts.push(format!("\"mesh\":{m}"));
            }
            if !n.children.is_empty() {
                parts.push(format!("\"children\":[{}]", n.children.iter().map(|c| format!("{c}")).collect::<Vec<_>>().join(",")));
            }
            s.push_str(&parts.join(","));
            s.push('}');
            s
        }).collect();
        let scenes: Vec<String> = self.scenes.iter().map(|s| format!("{{\"nodes\":[{}]}}", s.iter().map(|c| format!("{c}")).collect::<Vec<_>>().join(","))).collect();
        o.align();
        let buffer = match form {
            Form::External => {
                files.push((String::from("scene.bin"), o.bin.clone()));
                format!("{{\"byteLength\":{},\"uri\":\"scene.bin\"}}", o.bin.len())
            }
            Form::DataUri => format!("{{\"byteLength\":{},\"uri\":\"data:application/octet-stream;base64,{}\"}}", o.bin.len(), gltf::base64_encode(&o.bin)),
            Form::Glb => format!("{{\"byteLength\":{}}}", o.bin.len()),
        };
        let json = format!(
            "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"render3dtest gen.rs\"}},\"extensionsUsed\":[\"EXT_made_up\"],\"scene\":{},\"scenes\":[{}],\"nodes\":[{}],\"meshes\":[{}],\"materials\":[{}],\"textures\":[{}],\"samplers\":[{}],\"images\":[{}],\"accessors\":[{}],\"bufferViews\":[{}],\"buffers\":[{}]}}",
            self.scene, scenes.join(","), nodes.join(","), meshes.join(","), mats.join(","), textures.join(","), samplers.join(","), images.join(","),
            o.accessors.join(","), o.views.join(","), buffer);
        match form {
            Form::Glb => (gltf::write_glb(json.as_bytes(), Some(&o.bin)), files),
            _ => (json.into_bytes(), files),
        }
    }

    /// each node's world matrix, composed here
    pub fn worlds(&self) -> Vec<[f64; 16]> {
        let mut parent = vec![None; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            for &c in &n.children {
                parent[c] = Some(i);
            }
        }
        let mut out = vec![[0.0; 16]; self.nodes.len()];
        let mut done = vec![false; self.nodes.len()];
        for start in 0..self.nodes.len() {
            // the chain to a done node or a root, then down it
            let mut chain = vec![start];
            while let Some(p) = parent[*chain.last().unwrap_or(&start)] {
                if done[p] {
                    break;
                }
                chain.push(p);
            }
            for &i in chain.iter().rev() {
                if done[i] {
                    continue;
                }
                let n = &self.nodes[i];
                let local = trs(n.t, n.r, n.s);
                out[i] = match parent[i] {
                    Some(p) => mul4(&out[p], &local),
                    None => local,
                };
                done[i] = true;
            }
        }
        out
    }

    /// the instances of the scene, depth first: (node, mesh, primitive)
    pub fn instances(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.scenes[self.scene].iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            if let Some(m) = self.nodes[n].mesh {
                for p in 0..self.meshes[m].len() {
                    out.push((n, m, p));
                }
            }
            stack.extend(self.nodes[n].children.iter().rev());
        }
        out
    }
}

fn quat(axis: [f64; 3], angle: f64) -> [f64; 4] {
    let l = crate::cpu::norm(axis);
    let (s, c) = crate::cpu::sincos(angle * 0.5);
    [l[0] * s, l[1] * s, l[2] * s, c]
}

fn node(t: [f64; 3], r: [f64; 4], s: [f64; 3], mesh: Option<usize>, children: Vec<usize>) -> GNode {
    GNode { t, r, s, matrix: false, mesh, children }
}

/// the scene of the glTF tests: five meshes in every encoding the importer takes, five materials with PNG
/// and JPEG textures and every sampler mode, a hierarchy with TRS and matrices, a mirrored node, a chain 40
/// deep, two scenes
pub fn test_scene() -> GScene {
    use crate::common::{ball, cube, plate, prism};
    let noalpha = |_: u32, _: u32| 255u8;
    let holes = |x: u32, y: u32| if ((x / 16) + (y / 16)) % 2 == 0 { 255u8 } else { 0 };
    let images = vec![
        GImage { w: 128, h: 128, rgba: picture(128, 128, 11, &noalpha), jpeg: false },
        GImage { w: 128, h: 128, rgba: picture(128, 128, 12, &noalpha), jpeg: true },
        GImage { w: 64, h: 64, rgba: picture(64, 64, 13, &holes), jpeg: false },
        GImage { w: 32, h: 32, rgba: picture(32, 32, 14, &noalpha), jpeg: false },
    ];
    // (mag, min, wrapS, wrapT): nearest everything; trilinear clamped; nearest mip-nearest mirrored; linear repeat
    let texs = vec![
        GTex { image: 0, sampler: [9728, 9984, 10497, 10497] },
        GTex { image: 1, sampler: [9729, 9987, 33071, 33071] },
        GTex { image: 2, sampler: [9728, 9984, 33648, 10497] },
        GTex { image: 3, sampler: [9729, 9729, 10497, 33648] },
        GTex { image: 0, sampler: [9729, 9986, 10497, 33071] },
    ];
    let mat = |factor: [f32; 4], tex: [Option<usize>; 5], mask: Option<f32>| GMat { factor, metallic: 0.25, roughness: 0.75, emissive: [0.0; 3], tex, mask };
    let mats = vec![
        mat([1.0, 1.0, 1.0, 1.0], [Some(0), None, None, None, None], None),
        mat([0.9, 0.8, 1.0, 1.0], [Some(1), None, None, None, None], None),
        mat([0.3, 0.6, 0.9, 1.0], [None; 5], None),
        mat([1.0, 0.9, 0.8, 1.0], [Some(2), None, None, None, None], Some(0.5)),
        GMat { factor: [0.8, 0.8, 0.8, 1.0], metallic: 1.0, roughness: 0.5, emissive: [0.1, 0.2, 0.3], tex: [Some(3), Some(4), Some(3), Some(4), Some(0)], mask: None },
    ];
    let enc = |idx, uv, normals, interleaved| Enc { idx, uv, normals, interleaved, sparse: None };
    let mut big = plate();
    big.uv = big.uv.iter().map(|u| [u[0] * 0.75 + 0.125, u[1] * 0.75 + 0.125]).collect();
    let mut prism7 = prism(7, 0.6);
    for (i, u) in prism7.uv.iter_mut().enumerate() {
        *u = [(i % 4) as f32 * 0.25, (i % 3) as f32 * 0.3];
    }
    let meshes = vec![
        // 0: the cube, u16 indices, interleaved with a stride of 32
        vec![GPrim { geo: cube(), enc: enc(Some(5123), Some(5126), true, true), material: Some(0) }],
        // 1: a prism without normals (flat normals) and without indices, uv as normalized u8
        vec![GPrim { geo: unindexed(&prism7), enc: enc(None, Some(5121), false, false), material: Some(1) }],
        // 2: a plate, u8 indices, uv as normalized u16, a sparse accessor moving a corner
        vec![GPrim { geo: big, enc: Enc { sparse: Some((2, [1.25, 1.5, 0.25])), ..enc(Some(5121), Some(5123), true, false) }, material: Some(3) }],
        // 3: two primitives, u32 indices; the second without uv and without a material
        vec![GPrim { geo: ball(6, 10), enc: enc(Some(5125), Some(5126), true, false), material: Some(4) },
             GPrim { geo: prism(5, 0.3), enc: enc(Some(5125), None, true, false), material: None }],
        // 4: one colour, float uv in their own view
        vec![GPrim { geo: prism(9, 0.8), enc: enc(Some(5123), Some(5126), true, false), material: Some(2) }],
    ];
    let mut nodes = vec![
        // 0: the root: moved, turned, scaled unevenly
        node([0.5, -0.25, -2.0], quat([0.2, 1.0, 0.1], 0.6), [1.0, 1.0, 1.0], None, vec![1, 2, 3, 4, 5, 6]),
        node([-3.0, 1.0, 0.0], quat([1.0, 0.0, 0.0], 0.3), [1.2, 0.8, 1.0], Some(0), vec![]),
        // 2: a matrix
        GNode { matrix: true, ..node([3.0, 1.2, 0.5], quat([0.0, 1.0, 1.0], 1.1), [0.7, 0.7, 0.7], Some(1), vec![]) },
        // 3: mirrored in x
        node([0.0, -1.5, 1.0], quat([0.0, 0.0, 1.0], 0.25), [-1.0, 1.0, 1.0], Some(0), vec![]),
        node([0.0, 2.0, -1.0], quat([1.0, 0.2, 0.0], -0.7), [1.5, 1.5, 1.5], Some(2), vec![]),
        node([-1.0, -0.5, 2.5], quat([0.3, 0.3, 1.0], 2.0), [0.6, 0.6, 0.6], Some(3), vec![]),
        // 6: the head of a chain 40 deep (small turns), its end draws
        node([2.0, -2.0, 0.0], quat([0.0, 1.0, 0.0], 0.1), [1.0, 1.0, 1.0], None, vec![]),
    ];
    let mut last = 6;
    for k in 0..40 {
        let i = nodes.len();
        let mesh = if k == 39 { Some(4) } else { None };
        nodes.push(GNode { matrix: k % 2 == 1, ..node([0.0, 0.02, 0.0], quat([0.0, 0.0, 1.0], 0.01), [1.0, 1.0, 1.0], mesh, vec![]) });
        nodes[last].children.push(i);
        last = i;
    }
    // a second scene with a node of its own, and the first scene as the file's choice
    let other = nodes.len();
    nodes.push(node([0.0; 3], [0.0, 0.0, 0.0, 1.0], [2.0; 3], Some(4), vec![]));
    GScene { meshes, mats, texs, images, nodes, scenes: vec![vec![other], vec![0]], scene: 1 }
}

/// a mesh with three vertices of its own per triangle
pub fn unindexed(g: &Geo) -> Geo {
    let mut out = Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
    for &i in &g.idx {
        out.idx.push(out.pos.len() as u32);
        out.pos.push(g.pos[i as usize]);
        out.nrm.push(g.nrm[i as usize]);
        out.uv.push(g.uv[i as usize]);
    }
    out
}

/// the camera that sees the test scene
pub fn test_camera(aspect: f64) -> render3d::Camera {
    render3d::Camera::look_at([1.5, 2.0, 11.0], [0.0, 0.0, -1.5], [0.0, 1.0, 0.0], 0.9, aspect, 0.1, f64::INFINITY)
}
