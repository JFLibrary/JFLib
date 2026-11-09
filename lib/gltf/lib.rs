// gltf -- glTF 2.0 import (G5b; docs/GLTF.md): .gltf (JSON with external files or data URIs) and .glb, read
// into a data model that needs no GPU: meshes of triangles with positions, normals and one uv set, materials
// with the five textures of pbrMetallicRoughness, decoded images (PNG and JPEG through img_png / img_jpeg),
// samplers, nodes with their world matrices in f64 and the instances a scene draws.  lib/render3d puts a
// Document on the GPU (render3d::gltf); a CAD or game tool can read the same without a device.
//
//   gltf::load(bytes, &mut |uri| Option<Vec<u8>>, &Limits) -> Result<Document>   .glb or .gltf by its first
//        bytes; the closure reads the files a .gltf names (relative paths, decoded; never called for data URIs)
//   doc.instances(scene) -> Vec<InstanceRef>      each node's mesh primitives with its world matrix
//   gltf::decompose(&matrix) -> Trs               translation, rotation, scale (a mirror: x negative), shear
//
// Files are untrusted input: every index, offset, length and count is checked, sizes and counts are limited
// (Limits), and a file that breaks a rule is an Err, never a panic.  A file whose extensionsRequired names
// anything (Draco, meshopt, KTX2 / basisu, quantization, texture transforms, ...) is Error::Unsupported; other
// extensions and what this model has no place for (animations, skins, cameras, lights, tangents, colours,
// further uv sets) are ignored.  URIs that leave the file's folder (absolute paths, "..", a scheme) are
// refused before the closure sees them.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

mod read;

/// what went wrong; the text says where
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// not JSON, or JSON that breaks the parser's limits (the byte offset in the text)
    Json(String),
    /// a rule of glTF or of a format inside it broken: an index or a range out of bounds, a wrong type, a cycle
    Invalid(String),
    /// valid, but not supported here: a required extension, an image format, a URI scheme
    Unsupported(String),
    /// a Limits value exceeded
    Limit(String),
    /// the closure had no file for this URI
    Missing(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::Json(s) => write!(f, "glTF: not valid JSON: {s}"),
            Error::Invalid(s) => write!(f, "glTF: invalid: {s}"),
            Error::Unsupported(s) => write!(f, "glTF: not supported: {s}"),
            Error::Limit(s) => write!(f, "glTF: over a limit: {s}"),
            Error::Missing(s) => write!(f, "glTF: missing file: {s}"),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

pub(crate) fn invalid<T>(s: String) -> Result<T> {
    Err(Error::Invalid(s))
}

/// the bounds a file must stay in (Limits::DEFAULT; raise them for large files you trust more)
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// bytes of the .glb / .gltf and of each file it names
    pub file_bytes: u64,
    /// bytes of the JSON part, and the values in it (objects, arrays, numbers, ...)
    pub json_bytes: usize,
    pub json_values: usize,
    /// bytes of all buffers together
    pub buffer_bytes: u64,
    /// vertices and indices of all primitives together, as the model holds them (flat normals split vertices)
    pub vertices: u64,
    pub indices: u64,
    /// nodes, and how deep they nest
    pub nodes: usize,
    pub depth: usize,
    /// instances of one scene (node x primitive)
    pub instances: u64,
    /// pixels of one image, bytes of all decoded images (4 per pixel)
    pub image_pixels: u64,
    pub image_bytes: u64,
}

impl Limits {
    pub const DEFAULT: Limits = Limits {
        file_bytes: 1 << 30,
        json_bytes: 256 << 20,
        json_values: 16 << 20,
        buffer_bytes: 1 << 30,
        vertices: 64 << 20,
        indices: 192 << 20,
        nodes: 1 << 20,
        depth: 1024,
        instances: 16 << 20,
        image_pixels: 8192 * 8192,
        image_bytes: 2 << 30,
    };
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// what a file holds, in the order of its arrays (indices of the file stay valid)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    /// asset.version ("2.0") and asset.generator
    pub version: String,
    pub generator: String,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub images: Vec<Image>,
    pub nodes: Vec<Node>,
    pub scenes: Vec<SceneDef>,
    /// the scene to show (the file's `scene`; else the first; None: no scenes, instances() takes every root)
    pub scene: Option<usize>,
    /// what was read but left out: points and lines, unused images, unknown attributes, ... (once each)
    pub notes: Vec<String>,
}

/// a mesh: primitives with a material each
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
}

/// triangles: one normal and one uv per position, three indices per triangle, counter-clockwise from outside.
/// Strips and fans become lists; points and lines are left out (a note)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
    /// the file had no normals: each triangle has its own three vertices with its face's normal
    pub flat_normals: bool,
    /// the file had TEXCOORD_0 (else every uv is 0)
    pub has_uvs: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Opaque,
    /// below alpha_cutoff: not drawn
    Mask,
    Blend,
}

/// a texture of a material: which one, which uv set (render3d takes 0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureRef {
    pub texture: usize,
    pub tex_coord: u32,
}

/// a material: pbrMetallicRoughness and the three others, with glTF's defaults for what the file leaves out
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// linear rgba; the texture is sRGB
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<TextureRef>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    /// linear: roughness in g, metallic in b
    pub metallic_roughness_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<TextureRef>,
    pub occlusion_strength: f32,
    /// linear rgb; the texture is sRGB
    pub emissive_factor: [f32; 3],
    pub emissive_texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl Default for Material {
    /// glTF's default material: white, metallic 1, roughness 1
    fn default() -> Material {
        Material {
            name: String::new(),
            base_color_factor: [1.0; 4],
            base_color_texture: None,
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_factor: [0.0; 3],
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    Repeat,
    ClampToEdge,
    MirroredRepeat,
}

/// a sampler; what the file leaves out is linear with linear mip levels, repeating
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sampler {
    pub mag: Filter,
    pub min: Filter,
    /// between mip levels; None: level 0 only (minFilter NEAREST or LINEAR)
    pub mip: Option<Filter>,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}

impl Sampler {
    pub const DEFAULT: Sampler =
        Sampler { mag: Filter::Linear, min: Filter::Linear, mip: Some(Filter::Linear), wrap_s: Wrap::Repeat, wrap_t: Wrap::Repeat };
}

/// an image with a sampler.  image None: the file names its image only through an extension it does not
/// require (the materials' references to it are dropped, a note)
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub name: String,
    pub image: Option<usize>,
    pub sampler: Sampler,
}

/// a decoded image: rows from the top, 4 bytes per pixel, r g b a with straight alpha; the file's colour
/// profile and gamma are ignored (glTF says so), a JPEG's EXIF orientation too.  Images no texture uses
/// are not decoded (width 0)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Image {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// a node: its own transform (column-major 4 x 4, f64), the product of those of its ancestors and itself
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub mesh: Option<usize>,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    pub local: [f64; 16],
    pub world: [f64; 16],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneDef {
    pub name: String,
    /// root nodes
    pub nodes: Vec<usize>,
}

/// a primitive drawn by a node
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceRef {
    pub node: usize,
    pub mesh: usize,
    pub primitive: usize,
    pub world: [f64; 16],
}

/// the identity, column-major
pub const IDENTITY: [f64; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

impl Document {
    /// the instances of a scene (None: Document::scene), depth first in the order of the file; without
    /// scenes those of every node without a parent.  Each node is visited once: a Document built by hand
    /// with a cycle of children ends too
    pub fn instances(&self, scene: Option<usize>) -> Vec<InstanceRef> {
        let roots: Vec<usize> = match scene.or(self.scene).and_then(|s| self.scenes.get(s)) {
            Some(s) => s.nodes.clone(),
            None if self.scenes.is_empty() => (0..self.nodes.len()).filter(|&n| self.nodes[n].parent.is_none()).collect(),
            None => Vec::new(),
        };
        let mut out = Vec::new();
        let mut stack: Vec<usize> = roots.into_iter().rev().collect();
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            let Some(node) = self.nodes.get(n) else { continue };
            if core::mem::replace(&mut seen[n], true) {
                continue;
            }
            if let Some(m) = node.mesh.and_then(|m| self.meshes.get(m).map(|x| (m, x))) {
                for p in 0..m.1.primitives.len() {
                    out.push(InstanceRef { node: n, mesh: m.0, primitive: p, world: node.world });
                }
            }
            stack.extend(node.children.iter().rev());
        }
        out
    }
}

/// read a .glb or a .gltf (by its first bytes); `files` gives the bytes of a relative URI the file names
pub fn load(bytes: &[u8], files: &mut dyn FnMut(&str) -> Option<Vec<u8>>, limits: &Limits) -> Result<Document> {
    if bytes.len() as u64 > limits.file_bytes {
        return Err(Error::Limit(format!("the file has {} bytes, the limit is {}", bytes.len(), limits.file_bytes)));
    }
    if bytes.starts_with(b"glTF") {
        let (json, bin) = glb_chunks(bytes)?;
        read::document(json, bin, files, limits)
    } else {
        read::document(bytes, None, files, limits)
    }
}

/// the JSON and the BIN chunk of a .glb
pub fn glb_chunks(b: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    let u32_at = |at: usize| -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) };
    let (Some(magic), Some(version), Some(length)) = (u32_at(0), u32_at(4), u32_at(8)) else {
        return invalid(String::from("glb: the 12-byte header is cut off"));
    };
    if magic != 0x4654_6C67 {
        return invalid(String::from("glb: no glTF magic"));
    }
    if version != 2 {
        return Err(Error::Unsupported(format!("glb version {version} (2 is glTF 2.0)")));
    }
    let length = length as usize;
    if length < 12 || length > b.len() {
        return invalid(format!("glb: the header says {length} bytes, the file has {}", b.len()));
    }
    let mut at = 12usize;
    let (mut json, mut bin) = (None, None);
    let mut first = true;
    while at < length {
        let (Some(len), Some(kind)) = (u32_at(at), u32_at(at + 4)) else {
            return invalid(format!("glb: a chunk header at {at} is cut off"));
        };
        let start = at + 8;
        let Some(end) = start.checked_add(len as usize).filter(|&e| e <= length) else {
            return invalid(format!("glb: the chunk at {at} says {len} bytes, beyond the file's end"));
        };
        match kind {
            0x4E4F_534A if first => json = Some(&b[start..end]),
            0x004E_4942 if json.is_some() && bin.is_none() => bin = Some(&b[start..end]),
            0x4E4F_534A => return invalid(String::from("glb: a second JSON chunk")),
            _ if first => return invalid(String::from("glb: the first chunk is not JSON")),
            _ => {} // unknown chunks are skipped
        }
        first = false;
        // (chunks are padded to 4 bytes; a file that is not still reads)
        at = end.saturating_add(3) & !3;
    }
    match json {
        Some(j) => Ok((j, bin)),
        None => invalid(String::from("glb: no JSON chunk")),
    }
}

/// a .glb of a JSON text and a binary buffer (tools and tests): chunks padded with spaces and zeros
pub fn write_glb(json: &[u8], bin: Option<&[u8]>) -> Vec<u8> {
    let pad = |n: usize| (n + 3) & !3;
    let total = 12 + 8 + pad(json.len()) + bin.map_or(0, |b| 8 + pad(b.len()));
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(pad(json.len()) as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(json);
    out.resize(out.len() + pad(json.len()) - json.len(), b' ');
    if let Some(b) = bin {
        out.extend_from_slice(&(pad(b.len()) as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(b);
        out.resize(out.len() + pad(b.len()) - b.len(), 0);
    }
    out
}

// ---------------------------------------------------------------- URIs

/// standard base64 (padding optional); None for anything else
pub fn base64_decode(s: &[u8]) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let body = match s {
        [rest @ .., b'=', b'='] => rest,
        [rest @ .., b'='] => rest,
        _ => s,
    };
    if body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    for chunk in body.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        let bytes = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    Some(out)
}

/// base64 of bytes (tools and tests: data URIs)
pub fn base64_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let v = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() { A[(v >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    s
}

/// what a URI of the file means: the bytes of a data URI, or the decoded relative path for the closure
pub(crate) enum Uri {
    Data(Vec<u8>),
    File(String),
}

pub(crate) fn parse_uri(uri: &str) -> Result<Uri> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let Some(comma) = rest.find(',') else { return invalid(String::from("a data URI without a comma")) };
        if !rest[..comma].ends_with(";base64") {
            return Err(Error::Unsupported(String::from("a data URI that is not base64")));
        }
        return match base64_decode(rest[comma + 1..].as_bytes()) {
            Some(b) => Ok(Uri::Data(b)),
            None => invalid(String::from("a data URI with broken base64")),
        };
    }
    // a scheme ("http:", "file:") before the first slash, an absolute path, a drive, a way up: refused
    let first_slash = uri.find('/').unwrap_or(uri.len());
    if uri[..first_slash].contains(':') {
        return Err(Error::Unsupported(format!("the URI scheme of {:?} (relative paths and data URIs only)", short(uri))));
    }
    let mut path = Vec::with_capacity(uri.len());
    let b = uri.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = |c: u8| (c as char).to_digit(16);
            match (b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c))) {
                (Some(h), Some(l)) => path.push((h * 16 + l) as u8),
                _ => return invalid(format!("a broken %-escape in the URI {:?}", short(uri))),
            }
            i += 3;
        } else {
            path.push(b[i]);
            i += 1;
        }
    }
    let Ok(path) = String::from_utf8(path) else { return invalid(format!("the URI {:?} is not UTF-8", short(uri))) };
    let outside = path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains(':') || path.contains('\0')
        || path.split(['/', '\\']).any(|seg| seg == "..");
    if outside {
        return Err(Error::Unsupported(format!("the URI {:?} leaves the file's folder", short(uri))));
    }
    Ok(Uri::File(path))
}

/// a long text cut for a message
pub(crate) fn short(s: &str) -> &str {
    match s.char_indices().nth(60) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

// ---------------------------------------------------------------- matrices

/// a square root in f64 (no libm in no_std)
pub fn sqrt(x: f64) -> f64 {
    if x <= 0.0 || x.is_nan() {
        return 0.0;
    }
    if x.is_infinite() {
        return x;
    }
    let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        r = 0.5 * (r + x / r);
    }
    r
}

/// a * b, column-major 4 x 4
pub fn mat_mul(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

/// the matrix of translation, rotation (unit quaternion x, y, z, w) and scale: T * R * S
pub fn trs_matrix(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> [f64; 16] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)],
        [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)],
        [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = IDENTITY;
    for c in 0..3 {
        for k in 0..3 {
            m[c * 4 + k] = r[c][k] * s[c];
        }
    }
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

/// a node matrix taken apart: translation, rotation (unit quaternion x, y, z, w), scale (a mirror makes x
/// negative), and how far the columns are from right angles (|cos| of the worst pair: 0 without shear, which
/// T * R * S cannot hold)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
    pub shear: f64,
}

pub fn decompose(m: &[f64; 16]) -> Trs {
    let col = |c: usize| [m[c * 4], m[c * 4 + 1], m[c * 4 + 2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let mut c = [col(0), col(1), col(2)];
    let mut s = [sqrt(dot(c[0], c[0])), sqrt(dot(c[1], c[1])), sqrt(dot(c[2], c[2]))];
    if dot(c[0], cross(c[1], c[2])) < 0.0 {
        s[0] = -s[0];
    }
    // unit columns; a column of length 0 is made from the other two (or the axis)
    for k in 0..3 {
        if s[k] != 0.0 {
            c[k] = [c[k][0] / s[k], c[k][1] / s[k], c[k][2] / s[k]];
        }
    }
    for k in 0..3 {
        if s[k] == 0.0 {
            let (a, b) = (c[(k + 1) % 3], c[(k + 2) % 3]);
            let n = cross(a, b);
            let l = sqrt(dot(n, n));
            c[k] = if l > 0.0 { [n[0] / l, n[1] / l, n[2] / l] } else { let mut e = [0.0; 3]; e[k] = 1.0; e };
        }
    }
    let shear = dot(c[0], c[1]).abs().max(dot(c[0], c[2]).abs()).max(dot(c[1], c[2]).abs());
    // the quaternion of the rotation matrix (columns c), by its largest diagonal term
    let (m00, m11, m22) = (c[0][0], c[1][1], c[2][2]);
    let tr = m00 + m11 + m22;
    let q = if tr > 0.0 {
        let k = sqrt(tr + 1.0) * 2.0;
        [(c[1][2] - c[2][1]) / k, (c[2][0] - c[0][2]) / k, (c[0][1] - c[1][0]) / k, 0.25 * k]
    } else if m00 > m11 && m00 > m22 {
        let k = sqrt(1.0 + m00 - m11 - m22) * 2.0;
        [0.25 * k, (c[1][0] + c[0][1]) / k, (c[2][0] + c[0][2]) / k, (c[1][2] - c[2][1]) / k]
    } else if m11 > m22 {
        let k = sqrt(1.0 + m11 - m00 - m22) * 2.0;
        [(c[1][0] + c[0][1]) / k, 0.25 * k, (c[2][1] + c[1][2]) / k, (c[2][0] - c[0][2]) / k]
    } else {
        let k = sqrt(1.0 + m22 - m00 - m11) * 2.0;
        [(c[2][0] + c[0][2]) / k, (c[2][1] + c[1][2]) / k, 0.25 * k, (c[0][1] - c[1][0]) / k]
    };
    let l = sqrt(q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
    let q = if l > 0.0 { [q[0] / l, q[1] / l, q[2] / l, q[3] / l] } else { [0.0, 0.0, 0.0, 1.0] };
    Trs { translation: [m[12], m[13], m[14]], rotation: q, scale: s, shear }
}
