// read -- the JSON of a glTF file into a Document (lib.rs has the overview).  Every array index goes through
// get(), every byte range through checked arithmetic against the buffer it lies in; counts are charged to
// the limits before anything of that size is allocated.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::json::{self, Value};

use crate::*;

const FLOAT: u64 = 5126;
const UBYTE: u64 = 5121;
const USHORT: u64 = 5123;
const UINT: u64 = 5125;
const BYTE: u64 = 5120;
const SHORT: u64 = 5122;

/// the extensions a file may require: KHR_lights_punctual (G5c) only (each other one changes what the data means)
const SUPPORTED_REQUIRED: &[&str] = &["KHR_lights_punctual"];

struct View {
    buffer: usize,
    offset: usize,
    length: usize,
    stride: Option<usize>,
}

struct Ctx<'a> {
    root: &'a Value,
    buffers: Vec<Vec<u8>>,
    views: Vec<View>,
    limits: &'a Limits,
    notes: Vec<String>,
    vertices: u64,
    indices: u64,
}

fn arr<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    v.get(key).and_then(|x| x.as_array()).unwrap_or(&[])
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).map(String::from).unwrap_or_default()
}

/// a JSON number as f64 (None: absent; Err: not a finite number)
fn num(v: &Value, key: &str, what: &str) -> Result<Option<f64>> {
    match v.get(key) {
        None => Ok(None),
        Some(Value::Number(n)) => match n.as_str().parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(Some(x)),
            _ => invalid(format!("{what}.{key}: not a finite number")),
        },
        Some(_) => invalid(format!("{what}.{key}: not a number")),
    }
}

fn num_or(v: &Value, key: &str, what: &str, default: f64) -> Result<f64> {
    Ok(num(v, key, what)?.unwrap_or(default))
}

/// an index or a count (None: absent; Err: negative, a fraction, too large)
fn index(v: &Value, key: &str, what: &str) -> Result<Option<usize>> {
    match v.get(key) {
        None => Ok(None),
        Some(x) => match x.as_u64().and_then(|u| usize::try_from(u).ok()) {
            Some(u) => Ok(Some(u)),
            None => invalid(format!("{what}.{key}: not an unsigned integer")),
        },
    }
}

fn need(v: &Value, key: &str, what: &str) -> Result<usize> {
    match index(v, key, what)? {
        Some(u) => Ok(u),
        None => invalid(format!("{what}: no {key}")),
    }
}

fn floats<const N: usize>(v: &Value, key: &str, what: &str, default: [f64; N]) -> Result<[f64; N]> {
    let Some(x) = v.get(key) else { return Ok(default) };
    let Some(a) = x.as_array().filter(|a| a.len() == N) else { return invalid(format!("{what}.{key}: not an array of {N} numbers")) };
    let mut out = [0.0; N];
    for (i, e) in a.iter().enumerate() {
        out[i] = match e {
            Value::Number(n) => match n.as_str().parse::<f64>() {
                Ok(f) if f.is_finite() => f,
                _ => return invalid(format!("{what}.{key}[{i}]: not a finite number")),
            },
            _ => return invalid(format!("{what}.{key}[{i}]: not a number")),
        };
    }
    Ok(out)
}

fn f32s<const N: usize>(a: [f64; N]) -> [f32; N] {
    a.map(|v| v as f32)
}

impl Ctx<'_> {
    fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    fn charge(&mut self, vertices: u64, indices: u64) -> Result<()> {
        self.vertices = self.vertices.saturating_add(vertices);
        self.indices = self.indices.saturating_add(indices);
        if self.vertices > self.limits.vertices {
            return Err(Error::Limit(format!("more than {} vertices", self.limits.vertices)));
        }
        if self.indices > self.limits.indices {
            return Err(Error::Limit(format!("more than {} indices", self.limits.indices)));
        }
        Ok(())
    }

    /// an accessor's elements as f32 (normalized integers to 0 ..= 1 or -1 ..= 1), `comps` per element.
    /// types: the component types this attribute may have, (type, normalized)
    fn read_floats(&self, a: usize, kind: &str, comps: usize, types: &[(u64, bool)], what: &str) -> Result<Vec<f32>> {
        let (acc, ctype, count) = self.accessor(a, kind, self.limits.vertices, what)?;
        let normalized = acc.get("normalized").and_then(|v| v.as_bool()).unwrap_or(false);
        if !types.contains(&(ctype, normalized)) {
            return invalid(format!("{what}: accessor {a} has component type {ctype}{}, which this attribute may not have",
                                   if normalized { " (normalized)" } else { "" }));
        }
        let conv = |b: &[u8]| -> f32 {
            match ctype {
                FLOAT => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                UBYTE if normalized => b[0] as f32 / 255.0,
                USHORT if normalized => u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0,
                BYTE if normalized => (b[0] as i8 as f32 / 127.0).max(-1.0),
                SHORT if normalized => (i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).max(-1.0),
                UBYTE => b[0] as f32,
                USHORT => u16::from_le_bytes([b[0], b[1]]) as f32,
                BYTE => b[0] as i8 as f32,
                SHORT => i16::from_le_bytes([b[0], b[1]]) as f32,
                _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
            }
        };
        let mut out = vec![0f32; count.checked_mul(comps).ok_or_else(|| Error::Limit(format!("{what}: accessor {a} is too large")))?];
        self.elements(acc, a, ctype, count, comps, what, &mut |i, b| {
            let size = csize(ctype);
            for k in 0..comps {
                out[i * comps + k] = conv(&b[k * size..]);
            }
        })?;
        if out.iter().any(|v| !v.is_finite()) {
            return invalid(format!("{what}: accessor {a} holds a value that is not finite"));
        }
        Ok(out)
    }

    /// an index accessor's elements
    fn read_indices(&self, a: usize, what: &str) -> Result<Vec<u32>> {
        let (acc, ctype, count) = self.accessor(a, "SCALAR", self.limits.indices, what)?;
        if !matches!(ctype, UBYTE | USHORT | UINT) || acc.get("normalized").and_then(|v| v.as_bool()) == Some(true) {
            return invalid(format!("{what}: index accessor {a} has component type {ctype} (unsigned byte, short or int, not normalized)"));
        }
        let mut out = vec![0u32; count];
        self.elements(acc, a, ctype, count, 1, what, &mut |i, b| {
            out[i] = match ctype {
                UBYTE => b[0] as u32,
                USHORT => u16::from_le_bytes([b[0], b[1]]) as u32,
                _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            };
        })?;
        Ok(out)
    }

    /// accessor a: its object, component type, count (at most `max`: checked before anything is allocated),
    /// after checking its type
    fn accessor(&self, a: usize, kind: &str, max: u64, what: &str) -> Result<(&Value, u64, usize)> {
        let Some(acc) = arr(self.root, "accessors").get(a) else { return invalid(format!("{what}: no accessor {a}")) };
        let w = format!("accessor {a}");
        let t = acc.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if t != kind {
            return invalid(format!("{what}: {w} is {t:?}, not {kind}"));
        }
        let ctype = acc.get("componentType").and_then(|v| v.as_u64()).unwrap_or(0);
        if !matches!(ctype, FLOAT | UBYTE | USHORT | UINT | BYTE | SHORT) {
            return invalid(format!("{w}: component type {ctype}"));
        }
        let count = need(acc, "count", &w)?;
        if count == 0 {
            return invalid(format!("{w}: count 0"));
        }
        if count as u64 > max {
            return Err(Error::Limit(format!("{w}: {count} elements, the limit is {max}")));
        }
        Ok((acc, ctype, count))
    }

    /// visit each element's bytes: from its buffer view (stride), zeros without one, then the sparse values
    #[allow(clippy::too_many_arguments)]
    fn elements(&self, acc: &Value, a: usize, ctype: u64, count: usize, comps: usize, what: &str, f: &mut dyn FnMut(usize, &[u8])) -> Result<()> {
        let w = format!("{what}: accessor {a}");
        let elem = csize(ctype) * comps;
        if let Some(v) = index(acc, "bufferView", &w)? {
            let off = index(acc, "byteOffset", &w)?.unwrap_or(0);
            let (data, stride) = self.view_range(v, elem, &w)?;
            // the last element ends at off + stride * (count - 1) + elem
            let end = stride.checked_mul(count - 1).and_then(|x| x.checked_add(off)).and_then(|x| x.checked_add(elem));
            if end.is_none_or(|e| e > data.len()) {
                return invalid(format!("{w}: {count} elements of {elem} bytes every {stride} from {off} do not fit its view of {} bytes", data.len()));
            }
            for i in 0..count {
                let at = off + i * stride;
                f(i, &data[at..at + elem]);
            }
        } else {
            let zero = [0u8; 64];
            for i in 0..count {
                f(i, &zero[..elem]);
            }
        }
        if let Some(sp) = acc.get("sparse") {
            let n = need(sp, "count", &w)?;
            if n == 0 || n > count {
                return invalid(format!("{w}: sparse count {n} of {count}"));
            }
            let (Some(si), Some(sv)) = (sp.get("indices"), sp.get("values")) else { return invalid(format!("{w}: sparse without indices or values")) };
            let it = si.get("componentType").and_then(|v| v.as_u64()).unwrap_or(0);
            if !matches!(it, UBYTE | USHORT | UINT) {
                return invalid(format!("{w}: sparse index type {it}"));
            }
            let (idata, istride) = self.view_range(need(si, "bufferView", &w)?, csize(it), &w)?;
            let (vdata, vstride) = self.view_range(need(sv, "bufferView", &w)?, elem, &w)?;
            if istride != csize(it) || vstride != elem {
                return invalid(format!("{w}: sparse data in a view with a byte stride"));
            }
            let (io, vo) = (index(si, "byteOffset", &w)?.unwrap_or(0), index(sv, "byteOffset", &w)?.unwrap_or(0));
            let fits = |o: usize, size: usize, len: usize| n.checked_mul(size).and_then(|x| x.checked_add(o)).is_some_and(|e| e <= len);
            if !fits(io, csize(it), idata.len()) || !fits(vo, elem, vdata.len()) {
                return invalid(format!("{w}: sparse indices or values beyond their view"));
            }
            let mut last: Option<usize> = None;
            for k in 0..n {
                let b = &idata[io + k * csize(it)..];
                let i = match it {
                    UBYTE => b[0] as usize,
                    USHORT => u16::from_le_bytes([b[0], b[1]]) as usize,
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize,
                };
                if i >= count || last.is_some_and(|l| i <= l) {
                    return invalid(format!("{w}: sparse index {i} out of order or of range"));
                }
                last = Some(i);
                f(i, &vdata[vo + k * elem..vo + (k + 1) * elem]);
            }
        }
        Ok(())
    }

    /// a buffer view's bytes and the stride of elements of `elem` bytes in it
    fn view_range(&self, v: usize, elem: usize, what: &str) -> Result<(&[u8], usize)> {
        let Some(view) = self.views.get(v) else { return invalid(format!("{what}: no buffer view {v}")) };
        let stride = view.stride.unwrap_or(elem);
        if stride < elem {
            return invalid(format!("{what}: buffer view {v} has a byte stride of {stride}, an element has {elem} bytes"));
        }
        let buf = &self.buffers[view.buffer];
        Ok((&buf[view.offset..view.offset + view.length], stride))
    }
}

fn csize(ctype: u64) -> usize {
    match ctype {
        UBYTE | BYTE => 1,
        USHORT | SHORT => 2,
        _ => 4,
    }
}

/// the file's bytes for a URI (data or the closure's)
fn fetch(uri: &str, files: &mut dyn FnMut(&str) -> Option<Vec<u8>>, limits: &Limits) -> Result<Vec<u8>> {
    let b = match parse_uri(uri)? {
        Uri::Data(b) => b,
        Uri::File(path) => files(&path).ok_or_else(|| Error::Missing(String::from(short(&path))))?,
    };
    if b.len() as u64 > limits.file_bytes {
        return Err(Error::Limit(format!("{:?} has {} bytes, the limit is {}", short(uri), b.len(), limits.file_bytes)));
    }
    Ok(b)
}

pub(crate) fn document(json_bytes: &[u8], bin: Option<&[u8]>, files: &mut dyn FnMut(&str) -> Option<Vec<u8>>, limits: &Limits) -> Result<Document> {
    if json_bytes.len() > limits.json_bytes {
        return Err(Error::Limit(format!("the JSON has {} bytes, the limit is {}", json_bytes.len(), limits.json_bytes)));
    }
    let jl = json::Limits { bytes: limits.json_bytes, depth: 64, values: limits.json_values, string_bytes: limits.json_bytes, members: limits.json_values };
    let root = json::parse_with_limits(json_bytes, jl).map_err(|e| Error::Json(format!("{} at byte {}", e.kind, e.offset)))?;
    if root.as_object().is_none() {
        return invalid(String::from("the JSON is not an object"));
    }
    let Some(asset) = root.get("asset") else { return invalid(String::from("no asset")) };
    let version = text(asset, "version");
    if !version.starts_with("2.") {
        return Err(Error::Unsupported(format!("glTF version {:?} (2.x)", short(&version))));
    }
    for e in arr(&root, "extensionsRequired") {
        let name = e.as_str().unwrap_or("?");
        if !SUPPORTED_REQUIRED.contains(&name) {
            return Err(Error::Unsupported(format!("the required extension {}", short(name))));
        }
    }
    let mut cx = Ctx { root: &root, buffers: Vec::new(), views: Vec::new(), limits, notes: Vec::new(), vertices: 0, indices: 0 };
    // buffers: the first without a uri is a glb's BIN chunk
    let mut total = 0u64;
    for (i, b) in arr(&root, "buffers").iter().enumerate() {
        let w = format!("buffer {i}");
        let len = need(b, "byteLength", &w)?;
        let data = match b.get("uri").and_then(|u| u.as_str()) {
            Some(uri) => fetch(uri, files, limits)?,
            None if i == 0 && bin.is_some() => bin.map(<[u8]>::to_vec).unwrap_or_default(),
            None => return invalid(format!("{w}: no uri (and not a .glb's first buffer)")),
        };
        if data.len() < len {
            return invalid(format!("{w}: byteLength {len}, but {} bytes there", data.len()));
        }
        total = total.saturating_add(len as u64);
        if total > limits.buffer_bytes {
            return Err(Error::Limit(format!("buffers of more than {} bytes", limits.buffer_bytes)));
        }
        let mut data = data;
        data.truncate(len);
        cx.buffers.push(data);
    }
    for (i, v) in arr(&root, "bufferViews").iter().enumerate() {
        let w = format!("buffer view {i}");
        let buffer = need(v, "buffer", &w)?;
        let (offset, length) = (index(v, "byteOffset", &w)?.unwrap_or(0), need(v, "byteLength", &w)?);
        let Some(b) = cx.buffers.get(buffer) else { return invalid(format!("{w}: no buffer {buffer}")) };
        if offset.checked_add(length).is_none_or(|e| e > b.len()) {
            return invalid(format!("{w}: bytes {offset} + {length} beyond buffer {buffer} of {}", b.len()));
        }
        let stride = index(v, "byteStride", &w)?;
        if stride.is_some_and(|s| !(4..=252).contains(&s) || s % 4 != 0) {
            return invalid(format!("{w}: byteStride {} (4 ..= 252, a multiple of 4)", stride.unwrap_or(0)));
        }
        cx.views.push(View { buffer, offset, length, stride });
    }
    let images = read_images(&mut cx, files)?;
    let textures = read_textures(&mut cx, images.len())?;
    let materials = read_materials(&mut cx, textures.len())?;
    let meshes = read_meshes(&mut cx, materials.len())?;
    let lights = read_lights(&mut cx)?;
    let nodes = read_nodes(&mut cx, meshes.len(), lights.len())?;
    let (scenes, scene) = read_scenes(&mut cx, &nodes)?;
    let mut doc = Document {
        version,
        generator: text(asset, "generator"),
        meshes,
        materials,
        textures,
        images,
        nodes,
        scenes,
        scene,
        lights,
        notes: Vec::new(),
    };
    // a scene's instances are bounded before anyone asks for them
    for s in 0..doc.scenes.len().max(1) {
        let mut n = 0u64;
        let mut stack: Vec<usize> = match doc.scenes.get(s) {
            Some(sc) => sc.nodes.clone(),
            None => (0..doc.nodes.len()).filter(|&i| doc.nodes[i].parent.is_none()).collect(),
        };
        while let Some(i) = stack.pop() {
            let node = &doc.nodes[i];
            n += node.mesh.map_or(0, |m| doc.meshes[m].primitives.len() as u64);
            if n > limits.instances {
                return Err(Error::Limit(format!("scene {s} has more than {} instances", limits.instances)));
            }
            stack.extend_from_slice(&node.children);
        }
    }
    doc.notes = cx.notes;
    Ok(doc)
}

// ---------------------------------------------------------------- images, textures, materials

fn read_images(cx: &mut Ctx, files: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<Vec<Image>> {
    let root = cx.root;
    // which images a texture uses: only those are decoded
    let mut used = vec![false; arr(root, "images").len()];
    for t in arr(root, "textures") {
        if let Some(s) = index(t, "source", "texture")? {
            if let Some(u) = used.get_mut(s) {
                *u = true;
            }
        }
    }
    let mut out = Vec::new();
    let mut bytes = 0u64;
    for (i, im) in arr(root, "images").iter().enumerate() {
        let w = format!("image {i}");
        let name = text(im, "name");
        if !used[i] {
            cx.note(String::from("images no texture uses were not decoded"));
            out.push(Image { name, ..Image::default() });
            continue;
        }
        let data: Vec<u8> = match (im.get("uri").and_then(|u| u.as_str()), index(im, "bufferView", &w)?) {
            (Some(uri), _) => fetch(uri, files, cx.limits)?,
            (None, Some(v)) => {
                let Some(view) = cx.views.get(v) else { return invalid(format!("{w}: no buffer view {v}")) };
                cx.buffers[view.buffer][view.offset..view.offset + view.length].to_vec()
            }
            (None, None) => return invalid(format!("{w}: neither uri nor bufferView")),
        };
        let (width, height, rgba) = decode_image(&data, cx.limits, &w)?;
        bytes = bytes.saturating_add(rgba.len() as u64);
        if bytes > cx.limits.image_bytes {
            return Err(Error::Limit(format!("decoded images of more than {} bytes", cx.limits.image_bytes)));
        }
        out.push(Image { name, width, height, rgba });
    }
    Ok(out)
}

/// a PNG or JPEG (by its first bytes, not the mimeType) as straight rgba rows
fn decode_image(data: &[u8], limits: &Limits, what: &str) -> Result<(u32, u32, Vec<u8>)> {
    use img_core::{ImgMeta, JDF_NO_ORIENT, JDF_STRAIGHT};
    let pixels_ok = |w: u64, h: u64| w > 0 && h > 0 && w.saturating_mul(h) <= limits.image_pixels;
    let sf = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        // the size from IHDR before anything is allocated
        let (w, h) = match data.get(16..24) {
            Some(b) => (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64, u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as u64),
            None => return invalid(format!("{what}: a PNG cut off before its size")),
        };
        if !pixels_ok(w, h) {
            return Err(Error::Limit(format!("{what}: a PNG of {w} x {h} pixels (at most {} pixels)", limits.image_pixels)));
        }
        let mut sf = img_core::Surface::default();
        // (the decoder's raw rows: up to 8 bytes per pixel and a filter byte per row, plus the surface)
        let r = img_png::dec::decode_lim(data, &mut sf, JDF_STRAIGHT, w * h * 12 + h + 65536);
        if r != 0 {
            return invalid(format!("{what}: the PNG does not decode (error {r})"));
        }
        sf
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let mut m = ImgMeta::default();
        if img_jpeg::meta(data, &mut m, 0) != 0 {
            return invalid(format!("{what}: a JPEG without a readable frame header"));
        }
        if !pixels_ok(m.w as u64, m.h as u64) {
            return Err(Error::Limit(format!("{what}: a JPEG of {} x {} pixels (at most {} pixels)", m.w, m.h, limits.image_pixels)));
        }
        match img_jpeg::decode_fl_max(data, JDF_STRAIGHT | JDF_NO_ORIENT, m.w.max(1), m.h.max(1)) {
            Ok(sf) => sf,
            Err(e) => return invalid(format!("{what}: the JPEG does not decode (error {e})")),
        }
    } else {
        return Err(Error::Unsupported(format!("{what}: neither PNG nor JPEG")));
    };
    let (w, h, stride) = (sf.w.max(0) as usize, sf.h.max(0) as usize, sf.stride.max(0) as usize);
    if w == 0 || h == 0 || stride < w * 4 || sf.px.len() < stride * (h - 1) + w * 4 || !pixels_ok(w as u64, h as u64) {
        return invalid(format!("{what}: the decoder returned no usable pixels"));
    }
    // bytes b, g, r, a (FMT_ARGB) -> r, g, b, a
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for p in sf.px[y * stride..y * stride + w * 4].chunks_exact(4) {
            rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    Ok((w as u32, h as u32, rgba))
}

fn wrap(v: &Value, key: &str, what: &str) -> Result<Wrap> {
    Ok(match index(v, key, what)? {
        None | Some(10497) => Wrap::Repeat,
        Some(33071) => Wrap::ClampToEdge,
        Some(33648) => Wrap::MirroredRepeat,
        Some(x) => return invalid(format!("{what}.{key}: {x}")),
    })
}

fn read_textures(cx: &mut Ctx, images: usize) -> Result<Vec<Texture>> {
    let root = cx.root;
    let mut samplers = Vec::new();
    for (i, s) in arr(root, "samplers").iter().enumerate() {
        let w = format!("sampler {i}");
        let mag = match index(s, "magFilter", &w)? {
            None | Some(9729) => Filter::Linear,
            Some(9728) => Filter::Nearest,
            Some(x) => return invalid(format!("{w}.magFilter: {x}")),
        };
        let (min, mip) = match index(s, "minFilter", &w)? {
            None | Some(9987) => (Filter::Linear, Some(Filter::Linear)),
            Some(9728) => (Filter::Nearest, None),
            Some(9729) => (Filter::Linear, None),
            Some(9984) => (Filter::Nearest, Some(Filter::Nearest)),
            Some(9985) => (Filter::Linear, Some(Filter::Nearest)),
            Some(9986) => (Filter::Nearest, Some(Filter::Linear)),
            Some(x) => return invalid(format!("{w}.minFilter: {x}")),
        };
        samplers.push(Sampler { mag, min, mip, wrap_s: wrap(s, "wrapS", &w)?, wrap_t: wrap(s, "wrapT", &w)? });
    }
    let mut out = Vec::new();
    for (i, t) in arr(root, "textures").iter().enumerate() {
        let w = format!("texture {i}");
        let sampler = match index(t, "sampler", &w)? {
            None => Sampler::DEFAULT,
            Some(s) => *samplers.get(s).ok_or_else(|| Error::Invalid(format!("{w}: no sampler {s}")))?,
        };
        let image = index(t, "source", &w)?;
        if image.is_some_and(|s| s >= images) {
            return invalid(format!("{w}: no image {}", image.unwrap_or(0)));
        }
        if image.is_none() {
            cx.note(String::from("textures without a source (only an extension's image) are left out"));
        }
        out.push(Texture { name: text(t, "name"), image, sampler });
    }
    Ok(out)
}

fn texref(cx: &mut Ctx, v: &Value, key: &str, what: &str, textures: usize) -> Result<Option<TextureRef>> {
    let Some(t) = v.get(key) else { return Ok(None) };
    let w = format!("{what}.{key}");
    let texture = need(t, "index", &w)?;
    if texture >= textures {
        return invalid(format!("{w}: no texture {texture}"));
    }
    let tex_coord = index(t, "texCoord", &w)?.unwrap_or(0);
    if tex_coord > 0 {
        cx.note(String::from("textures on a uv set other than TEXCOORD_0 (render3d draws them with TEXCOORD_0)"));
    }
    Ok(Some(TextureRef { texture, tex_coord: tex_coord.min(u32::MAX as usize) as u32 }))
}

fn read_materials(cx: &mut Ctx, textures: usize) -> Result<Vec<Material>> {
    let root = cx.root;
    let mut out = Vec::new();
    for (i, m) in arr(root, "materials").iter().enumerate() {
        let w = format!("material {i}");
        let mut mat = Material { name: text(m, "name"), ..Material::default() };
        if let Some(p) = m.get("pbrMetallicRoughness") {
            let wp = format!("{w}.pbrMetallicRoughness");
            mat.base_color_factor = f32s(floats(p, "baseColorFactor", &wp, [1.0; 4])?);
            mat.metallic_factor = num_or(p, "metallicFactor", &wp, 1.0)? as f32;
            mat.roughness_factor = num_or(p, "roughnessFactor", &wp, 1.0)? as f32;
            mat.base_color_texture = texref(cx, p, "baseColorTexture", &wp, textures)?;
            mat.metallic_roughness_texture = texref(cx, p, "metallicRoughnessTexture", &wp, textures)?;
        }
        mat.normal_texture = texref(cx, m, "normalTexture", &w, textures)?;
        if let Some(n) = m.get("normalTexture") {
            mat.normal_scale = num_or(n, "scale", &w, 1.0)? as f32;
        }
        mat.occlusion_texture = texref(cx, m, "occlusionTexture", &w, textures)?;
        if let Some(o) = m.get("occlusionTexture") {
            mat.occlusion_strength = num_or(o, "strength", &w, 1.0)? as f32;
        }
        mat.emissive_texture = texref(cx, m, "emissiveTexture", &w, textures)?;
        mat.emissive_factor = f32s(floats(m, "emissiveFactor", &w, [0.0; 3])?);
        mat.alpha_mode = match m.get("alphaMode").map(|v| v.as_str()) {
            None | Some(Some("OPAQUE")) => AlphaMode::Opaque,
            Some(Some("MASK")) => AlphaMode::Mask,
            Some(Some("BLEND")) => AlphaMode::Blend,
            _ => return invalid(format!("{w}.alphaMode: OPAQUE, MASK or BLEND")),
        };
        mat.alpha_cutoff = num_or(m, "alphaCutoff", &w, 0.5)? as f32;
        if mat.alpha_cutoff < 0.0 {
            return invalid(format!("{w}.alphaCutoff: negative"));
        }
        mat.double_sided = m.get("doubleSided").and_then(|v| v.as_bool()).unwrap_or(false);
        out.push(mat);
    }
    Ok(out)
}

// ---------------------------------------------------------------- meshes

fn read_meshes(cx: &mut Ctx, materials: usize) -> Result<Vec<Mesh>> {
    let root = cx.root;
    let mut out = Vec::new();
    for (mi, m) in arr(root, "meshes").iter().enumerate() {
        let mut mesh = Mesh { name: text(m, "name"), primitives: Vec::new() };
        let prims = arr(m, "primitives");
        if prims.is_empty() {
            return invalid(format!("mesh {mi}: no primitives"));
        }
        for (pi, p) in prims.iter().enumerate() {
            let w = format!("mesh {mi} primitive {pi}");
            if let Some(prim) = read_primitive(cx, p, &w, materials)? {
                mesh.primitives.push(prim);
            }
        }
        out.push(mesh);
    }
    Ok(out)
}

fn read_primitive(cx: &mut Ctx, p: &Value, w: &str, materials: usize) -> Result<Option<Primitive>> {
    let mode = index(p, "mode", w)?.unwrap_or(4);
    match mode {
        4..=6 => {}
        0..=3 => {
            cx.note(String::from("points and lines are left out"));
            return Ok(None);
        }
        _ => return invalid(format!("{w}: mode {mode}")),
    }
    let material = index(p, "material", w)?;
    if material.is_some_and(|m| m >= materials) {
        return invalid(format!("{w}: no material {}", material.unwrap_or(0)));
    }
    let Some(attrs) = p.get("attributes").and_then(|a| a.as_object()) else { return invalid(format!("{w}: no attributes")) };
    let attr = |name: &str| -> Result<Option<usize>> {
        match attrs.iter().find(|(k, _)| k == name) {
            None => Ok(None),
            Some((_, v)) => match v.as_u64().and_then(|u| usize::try_from(u).ok()) {
                Some(u) => Ok(Some(u)),
                None => invalid(format!("{w}: attribute {name} is not an accessor index")),
            },
        }
    };
    for (k, _) in attrs {
        if !matches!(k.as_str(), "POSITION" | "NORMAL" | "TEXCOORD_0") {
            cx.note(format!("the attribute {} is not read", short(k)));
        }
    }
    let Some(pa) = attr("POSITION")? else {
        cx.note(String::from("primitives without POSITION are left out"));
        return Ok(None);
    };
    let pos = cx.read_floats(pa, "VEC3", 3, &[(FLOAT, false)], w)?;
    let n = pos.len() / 3;
    cx.charge(n as u64, 0)?;
    let nrm = match attr("NORMAL")? {
        Some(a) => Some(cx.read_floats(a, "VEC3", 3, &[(FLOAT, false)], w)?),
        None => None,
    };
    let uv = match attr("TEXCOORD_0")? {
        Some(a) => Some(cx.read_floats(a, "VEC2", 2, &[(FLOAT, false), (UBYTE, true), (USHORT, true)], w)?),
        None => None,
    };
    if nrm.as_ref().is_some_and(|v| v.len() / 3 != n) || uv.as_ref().is_some_and(|v| v.len() / 2 != n) {
        return invalid(format!("{w}: attributes of different counts"));
    }
    let raw = match index(p, "indices", w)? {
        Some(a) => {
            let v = cx.read_indices(a, w)?;
            if let Some(bad) = v.iter().find(|&&i| i as usize >= n) {
                return invalid(format!("{w}: index {bad} of {n} vertices"));
            }
            v
        }
        None => (0..n as u32).collect(),
    };
    // strips and fans as lists (a strip's odd triangles turn round to keep their winding); the list's
    // indices are charged before it is allocated (a strip triples them)
    let listed = if mode == 4 { raw.len() } else { raw.len().saturating_sub(2) * 3 };
    cx.charge(0, listed as u64)?;
    let idx: Vec<u32> = match mode {
        4 => {
            if raw.len() % 3 != 0 {
                return invalid(format!("{w}: {} indices of triangles, not a multiple of 3", raw.len()));
            }
            raw
        }
        5 => {
            let mut v = Vec::with_capacity(raw.len().saturating_sub(2) * 3);
            for i in 2..raw.len() {
                if i % 2 == 0 { v.extend_from_slice(&[raw[i - 2], raw[i - 1], raw[i]]) } else { v.extend_from_slice(&[raw[i - 1], raw[i - 2], raw[i]]) }
            }
            v
        }
        _ => {
            let mut v = Vec::with_capacity(raw.len().saturating_sub(2) * 3);
            for i in 2..raw.len() {
                v.extend_from_slice(&[raw[0], raw[i - 1], raw[i]]);
            }
            v
        }
    };
    if idx.is_empty() {
        cx.note(String::from("primitives without a triangle are left out"));
        return Ok(None);
    }
    let v3 =|v: &[f32], i: usize| [v[i * 3], v[i * 3 + 1], v[i * 3 + 2]];
    let v2 = |v: &[f32], i: usize| [v[i * 2], v[i * 2 + 1]];
    let prim = match nrm {
        Some(nv) => Primitive {
            positions: (0..n).map(|i| v3(&pos, i)).collect(),
            normals: (0..n).map(|i| v3(&nv, i)).collect(),
            uvs: (0..n).map(|i| uv.as_ref().map_or([0.0; 2], |u| v2(u, i))).collect(),
            indices: idx,
            material,
            flat_normals: false,
            has_uvs: uv.is_some(),
        },
        None => {
            // flat normals: three vertices of its own per triangle with the face's normal
            cx.charge(idx.len() as u64, 0)?;
            let mut out = Primitive { material, flat_normals: true, has_uvs: uv.is_some(), ..Primitive::default() };
            for t in idx.chunks_exact(3) {
                let (a, b, c) = (v3(&pos, t[0] as usize), v3(&pos, t[1] as usize), v3(&pos, t[2] as usize));
                let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let nn = [(e1[1] * e2[2] - e1[2] * e2[1]) as f64, (e1[2] * e2[0] - e1[0] * e2[2]) as f64, (e1[0] * e2[1] - e1[1] * e2[0]) as f64];
                let l = sqrt(nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]);
                // (a triangle of no area covers no pixel; its normal only must not be NaN)
                let face = if l > 0.0 { [(nn[0] / l) as f32, (nn[1] / l) as f32, (nn[2] / l) as f32] } else { [0.0, 0.0, 1.0] };
                for &i in t {
                    out.indices.push(out.positions.len() as u32);
                    out.positions.push(v3(&pos, i as usize));
                    out.normals.push(face);
                    out.uvs.push(uv.as_ref().map_or([0.0; 2], |u| v2(u, i as usize)));
                }
            }
            out
        }
    };
    Ok(Some(prim))
}

// ---------------------------------------------------------------- nodes and scenes

// ---------------------------------------------------------------- KHR_lights_punctual (G5c)

/// the file's lights: each checked (a type of the three, colour and intensity finite and not negative, a range
/// above 0, cone angles 0 <= inner < outer <= pi / 2), their number against Limits::lights
fn read_lights(cx: &mut Ctx) -> Result<Vec<Light>> {
    let Some(ext) = cx.root.get("extensions").and_then(|e| e.get("KHR_lights_punctual")) else { return Ok(Vec::new()) };
    if ext.as_object().is_none() {
        return invalid(String::from("extensions.KHR_lights_punctual: not an object"));
    }
    let list = match ext.get("lights") {
        None => return Ok(Vec::new()),
        Some(v) => match v.as_array() {
            Some(a) => a,
            None => return invalid(String::from("KHR_lights_punctual.lights: not an array")),
        },
    };
    if list.len() > cx.limits.lights {
        return Err(Error::Limit(format!("{} lights, the limit is {}", list.len(), cx.limits.lights)));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, v) in list.iter().enumerate() {
        let w = format!("light {i}");
        if v.as_object().is_none() {
            return invalid(format!("{w}: not an object"));
        }
        let color = floats(v, "color", &w, [1.0; 3])?;
        if color.iter().any(|c| *c < 0.0) {
            return invalid(format!("{w}.color: negative"));
        }
        let intensity = num_or(v, "intensity", &w, 1.0)?;
        if intensity < 0.0 {
            return invalid(format!("{w}.intensity: negative"));
        }
        let range = num(v, "range", &w)?;
        if range.is_some_and(|r| r <= 0.0) {
            return invalid(format!("{w}.range: not above 0"));
        }
        let kind = match v.get("type").and_then(|t| t.as_str()) {
            Some("directional") => {
                if range.is_some() {
                    cx.note(String::from("a directional light's range is ignored"));
                }
                LightKind::Directional
            }
            Some("point") => LightKind::Point,
            Some("spot") => {
                let Some(s) = v.get("spot").filter(|s| s.as_object().is_some()) else { return invalid(format!("{w}: a spot light without spot")) };
                let inner = num_or(s, "innerConeAngle", &w, 0.0)?;
                let outer = num_or(s, "outerConeAngle", &w, core::f64::consts::FRAC_PI_4)?;
                if !(inner >= 0.0 && inner < outer && outer <= core::f64::consts::FRAC_PI_2) {
                    return invalid(format!("{w}.spot: cone angles 0 <= inner < outer <= pi / 2"));
                }
                LightKind::Spot { inner: inner as f32, outer: outer as f32 }
            }
            Some(t) => return invalid(format!("{w}: the type {:?} (directional, point, spot)", short(t))),
            None => return invalid(format!("{w}: no type")),
        };
        out.push(Light { name: text(v, "name"), kind, color: f32s(color), intensity: intensity as f32, range: range.map(|r| r as f32) });
    }
    Ok(out)
}

fn read_nodes(cx: &mut Ctx, meshes: usize, lights: usize) -> Result<Vec<Node>> {
    let root = cx.root;
    let list = arr(root, "nodes");
    if list.len() > cx.limits.nodes {
        return Err(Error::Limit(format!("{} nodes, the limit is {}", list.len(), cx.limits.nodes)));
    }
    let mut nodes = Vec::with_capacity(list.len());
    for (i, v) in list.iter().enumerate() {
        let w = format!("node {i}");
        let mesh = index(v, "mesh", &w)?;
        if mesh.is_some_and(|m| m >= meshes) {
            return invalid(format!("{w}: no mesh {}", mesh.unwrap_or(0)));
        }
        let mut children = Vec::new();
        for c in arr(v, "children") {
            match c.as_u64().and_then(|u| usize::try_from(u).ok()) {
                Some(c) if c < list.len() => children.push(c),
                _ => return invalid(format!("{w}: a child that is no node")),
            }
        }
        let local = if v.get("matrix").is_some() {
            let m = floats(v, "matrix", &w, IDENTITY)?;
            if v.get("translation").is_some() || v.get("rotation").is_some() || v.get("scale").is_some() {
                cx.note(String::from("nodes with a matrix and TRS: the matrix counts"));
            }
            // (glTF: decomposable into T * R * S, so the last row is 0 0 0 1)
            if m[3].abs() > 1e-6 || m[7].abs() > 1e-6 || m[11].abs() > 1e-6 || (m[15] - 1.0).abs() > 1e-6 {
                return invalid(format!("{w}: a matrix whose last row is not 0 0 0 1"));
            }
            m
        } else {
            let t = floats(v, "translation", &w, [0.0; 3])?;
            let q = floats(v, "rotation", &w, [0.0, 0.0, 0.0, 1.0])?;
            let s = floats(v, "scale", &w, [1.0; 3])?;
            let l = sqrt(q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
            if l == 0.0 {
                return invalid(format!("{w}: a rotation of length 0"));
            }
            trs_matrix(t, [q[0] / l, q[1] / l, q[2] / l, q[3] / l], s)
        };
        // (G5c) the light it places
        let light = match v.get("extensions").and_then(|e| e.get("KHR_lights_punctual")) {
            None => None,
            Some(x) => match index(x, "light", &w)? {
                Some(l) if l < lights => Some(l),
                Some(l) => return invalid(format!("{w}: no light {l}")),
                None => return invalid(format!("{w}: KHR_lights_punctual without a light")),
            },
        };
        nodes.push(Node { name: text(v, "name"), mesh, children, parent: None, local, world: IDENTITY, light });
    }
    // parents: one at most; then the world matrices from the roots down, without recursion
    for i in 0..nodes.len() {
        for k in 0..nodes[i].children.len() {
            let c = nodes[i].children[k];
            if c == i || nodes[c].parent.is_some() {
                return invalid(format!("node {c}: a child of two nodes, or of itself"));
            }
            nodes[c].parent = Some(i);
        }
    }
    let mut seen = vec![false; nodes.len()];
    let mut stack: Vec<(usize, usize)> = (0..nodes.len()).filter(|&i| nodes[i].parent.is_none()).map(|i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        if depth > cx.limits.depth {
            return Err(Error::Limit(format!("nodes nested deeper than {}", cx.limits.depth)));
        }
        seen[i] = true;
        nodes[i].world = match nodes[i].parent {
            Some(p) => mat_mul(&nodes[p].world, &nodes[i].local),
            None => nodes[i].local,
        };
        for k in 0..nodes[i].children.len() {
            stack.push((nodes[i].children[k], depth + 1));
        }
    }
    if let Some(i) = seen.iter().position(|s| !s) {
        return invalid(format!("node {i} lies on a cycle of nodes"));
    }
    if nodes.iter().any(|n| n.world.iter().any(|v| !v.is_finite())) {
        return invalid(String::from("a node's world matrix is not finite"));
    }
    Ok(nodes)
}

fn read_scenes(cx: &mut Ctx, nodes: &[Node]) -> Result<(Vec<SceneDef>, Option<usize>)> {
    let root = cx.root;
    let mut out = Vec::new();
    for (i, s) in arr(root, "scenes").iter().enumerate() {
        let w = format!("scene {i}");
        let mut list = Vec::new();
        let mut taken = vec![false; nodes.len()];
        for n in arr(s, "nodes") {
            match n.as_u64().and_then(|u| usize::try_from(u).ok()) {
                Some(n) if n < nodes.len() && nodes[n].parent.is_none() && !taken[n] => {
                    taken[n] = true;
                    list.push(n);
                }
                _ => return invalid(format!("{w}: a node that is not a root of its own, or twice")),
            }
        }
        out.push(SceneDef { name: text(s, "name"), nodes: list });
    }
    let scene = match index(root, "scene", "the file")? {
        Some(s) if s < out.len() => Some(s),
        Some(s) => return invalid(format!("scene {s}: there is no such scene")),
        None if out.is_empty() => None,
        None => Some(0),
    };
    Ok((out, scene))
}
