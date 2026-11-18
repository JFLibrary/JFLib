// write -- a Document as a .glb (G7g): meshes with positions, normals and one uv set, materials by their
// factors, nodes with their local transforms, scenes.  What load reads back is the same Document (the
// numbers are written as the shortest text that parses to the same value).  Not written: textures,
// images and samplers, lights; a Document with any of them is Error::Unsupported rather than written
// without them.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Number, Value};

use crate::{AlphaMode, Document, Error, IDENTITY, Result, write_glb};

/// a finite number as JSON (an f32 value written through its exact f64, so it reads back bit for bit)
fn number(v: f64, what: &str) -> Result<Value> {
    if !v.is_finite() {
        return Err(Error::Invalid(format!("{what}: not a finite number")));
    }
    // Display of f64 is the shortest text that parses back to the same value, never with an exponent
    match Number::parse(&format!("{}", v + 0.0)) {
        Some(n) => Ok(Value::Number(n)),
        None => Err(Error::Invalid(format!("{what}: no JSON number"))),
    }
}

fn numbers(v: &[f64], what: &str) -> Result<Value> {
    let mut out = Vec::with_capacity(v.len());
    for &x in v {
        out.push(number(x, what)?);
    }
    Ok(Value::Array(out))
}

fn uint(v: usize) -> Value {
    Value::u64(v as u64)
}

fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (String::from(k), v)).collect())
}

/// the binary buffer and its views and accessors as they are built
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    /// a view of `bytes` (padded to 4) with target 34962 (vertices) or 34963 (indices); its index
    fn view(&mut self, bytes: &[u8], target: u64) -> usize {
        while self.data.len() % 4 != 0 {
            self.data.push(0);
        }
        let offset = self.data.len();
        self.data.extend_from_slice(bytes);
        self.views.push(obj(alloc::vec![
            ("buffer", uint(0)),
            ("byteOffset", uint(offset)),
            ("byteLength", uint(bytes.len())),
            ("target", Value::u64(target)),
        ]));
        self.views.len() - 1
    }

    /// an accessor of floats (`N` per element) with min and max when asked; its index
    fn floats<const N: usize>(&mut self, data: &[[f32; N]], min_max: bool, what: &str) -> Result<usize> {
        let mut bytes = Vec::with_capacity(data.len() * 4 * N);
        let (mut lo, mut hi) = ([f64::INFINITY; N], [f64::NEG_INFINITY; N]);
        for e in data {
            for (k, &v) in e.iter().enumerate() {
                if !v.is_finite() {
                    return Err(Error::Invalid(format!("{what}: not a finite number")));
                }
                bytes.extend_from_slice(&v.to_le_bytes());
                lo[k] = lo[k].min(v as f64);
                hi[k] = hi[k].max(v as f64);
            }
        }
        let view = self.view(&bytes, 34962);
        let ty = match N {
            2 => "VEC2",
            3 => "VEC3",
            _ => "VEC4",
        };
        let mut fields = alloc::vec![
            ("bufferView", uint(view)),
            ("componentType", Value::u64(5126)),
            ("count", uint(data.len())),
            ("type", Value::string(ty)),
        ];
        if min_max && !data.is_empty() {
            fields.push(("min", numbers(&lo, what)?));
            fields.push(("max", numbers(&hi, what)?));
        }
        self.accessors.push(obj(fields));
        Ok(self.accessors.len() - 1)
    }

    /// an accessor of triangle indices: u16 when every index fits, else u32; its index
    fn indices(&mut self, idx: &[u32]) -> usize {
        let small = idx.iter().all(|&i| i < 65_535);
        let mut bytes = Vec::with_capacity(idx.len() * if small { 2 } else { 4 });
        for &i in idx {
            if small {
                bytes.extend_from_slice(&(i as u16).to_le_bytes());
            } else {
                bytes.extend_from_slice(&i.to_le_bytes());
            }
        }
        let view = self.view(&bytes, 34963);
        self.accessors.push(obj(alloc::vec![
            ("bufferView", uint(view)),
            ("componentType", Value::u64(if small { 5123 } else { 5125 })),
            ("count", uint(idx.len())),
            ("type", Value::string("SCALAR")),
        ]));
        self.accessors.len() - 1
    }
}

/// The document as a .glb: meshes (positions with min and max, normals, TEXCOORD_0 when the primitive has
/// UV, indices as u16 or u32), materials (factors, alpha mode and cutoff, double-sided), nodes (name, mesh,
/// children, the local matrix unless it is the identity), scenes and the default scene; `asset.generator`
/// is the document's.  Err(Invalid) for arrays that do not fit (an index out of range, normals or UV of
/// another length, a number that is not finite, a reference to a mesh, material or node that is not
/// there), Err(Unsupported) for textures, images or lights, which this writer leaves to later.
pub fn write_document(doc: &Document) -> Result<Vec<u8>> {
    if !doc.textures.is_empty() || !doc.images.is_empty() || !doc.lights.is_empty() {
        return Err(Error::Unsupported(String::from("write_document: textures, images and lights are not written yet")));
    }
    let mut bin = Bin { data: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let mut meshes = Vec::with_capacity(doc.meshes.len());
    for (mi, m) in doc.meshes.iter().enumerate() {
        let mut prims = Vec::with_capacity(m.primitives.len());
        for (pi, p) in m.primitives.iter().enumerate() {
            let what = format!("meshes[{mi}].primitives[{pi}]");
            let n = p.positions.len();
            if p.normals.len() != n || p.uvs.len() != n {
                return Err(Error::Invalid(format!("{what}: normals and uvs must be as many as positions")));
            }
            if p.indices.len() % 3 != 0 || p.indices.iter().any(|&i| i as usize >= n) {
                return Err(Error::Invalid(format!("{what}: indices must be whole triangles within the positions")));
            }
            if p.material.is_some_and(|x| x >= doc.materials.len()) {
                return Err(Error::Invalid(format!("{what}: material out of range")));
            }
            let pos = bin.floats(&p.positions, true, &what)?;
            let nrm = bin.floats(&p.normals, false, &what)?;
            let mut attrs = alloc::vec![("POSITION", uint(pos)), ("NORMAL", uint(nrm))];
            if p.has_uvs {
                attrs.push(("TEXCOORD_0", uint(bin.floats(&p.uvs, false, &what)?)));
            }
            let mut fields = alloc::vec![("attributes", obj(attrs)), ("indices", uint(bin.indices(&p.indices)))];
            if let Some(mat) = p.material {
                fields.push(("material", uint(mat)));
            }
            fields.push(("mode", Value::u64(4)));
            prims.push(obj(fields));
        }
        let mut fields = Vec::new();
        if !m.name.is_empty() {
            fields.push(("name", Value::string(m.name.clone())));
        }
        fields.push(("primitives", Value::Array(prims)));
        meshes.push(obj(fields));
    }
    let mut materials = Vec::with_capacity(doc.materials.len());
    for (i, m) in doc.materials.iter().enumerate() {
        let what = format!("materials[{i}]");
        if m.base_color_texture.is_some() || m.metallic_roughness_texture.is_some() || m.normal_texture.is_some()
            || m.occlusion_texture.is_some() || m.emissive_texture.is_some()
        {
            return Err(Error::Unsupported(format!("{what}: textures are not written yet")));
        }
        let f4: Vec<f64> = m.base_color_factor.iter().map(|&v| v as f64).collect();
        let pbr = obj(alloc::vec![
            ("baseColorFactor", numbers(&f4, &what)?),
            ("metallicFactor", number(m.metallic_factor as f64, &what)?),
            ("roughnessFactor", number(m.roughness_factor as f64, &what)?),
        ]);
        let mut fields = Vec::new();
        if !m.name.is_empty() {
            fields.push(("name", Value::string(m.name.clone())));
        }
        fields.push(("pbrMetallicRoughness", pbr));
        if m.emissive_factor != [0.0; 3] {
            let e: Vec<f64> = m.emissive_factor.iter().map(|&v| v as f64).collect();
            fields.push(("emissiveFactor", numbers(&e, &what)?));
        }
        match m.alpha_mode {
            AlphaMode::Opaque => {}
            AlphaMode::Mask => {
                fields.push(("alphaMode", Value::string("MASK")));
                fields.push(("alphaCutoff", number(m.alpha_cutoff as f64, &what)?));
            }
            AlphaMode::Blend => fields.push(("alphaMode", Value::string("BLEND"))),
        }
        if m.double_sided {
            fields.push(("doubleSided", Value::Bool(true)));
        }
        materials.push(obj(fields));
    }
    let mut nodes = Vec::with_capacity(doc.nodes.len());
    for (i, n) in doc.nodes.iter().enumerate() {
        let what = format!("nodes[{i}]");
        let mut fields = Vec::new();
        if !n.name.is_empty() {
            fields.push(("name", Value::string(n.name.clone())));
        }
        if let Some(m) = n.mesh {
            if m >= doc.meshes.len() {
                return Err(Error::Invalid(format!("{what}: mesh out of range")));
            }
            fields.push(("mesh", uint(m)));
        }
        if !n.children.is_empty() {
            if n.children.iter().any(|&c| c >= doc.nodes.len()) {
                return Err(Error::Invalid(format!("{what}: child out of range")));
            }
            fields.push(("children", Value::Array(n.children.iter().map(|&c| uint(c)).collect())));
        }
        if n.local != IDENTITY {
            fields.push(("matrix", numbers(&n.local, &what)?));
        }
        nodes.push(obj(fields));
    }
    let mut scenes = Vec::with_capacity(doc.scenes.len());
    for (i, s) in doc.scenes.iter().enumerate() {
        if s.nodes.iter().any(|&n| n >= doc.nodes.len()) {
            return Err(Error::Invalid(format!("scenes[{i}]: node out of range")));
        }
        let mut fields = Vec::new();
        if !s.name.is_empty() {
            fields.push(("name", Value::string(s.name.clone())));
        }
        fields.push(("nodes", Value::Array(s.nodes.iter().map(|&n| uint(n)).collect())));
        scenes.push(obj(fields));
    }
    let generator = if doc.generator.is_empty() { String::from("JFLib gltf") } else { doc.generator.clone() };
    let mut root = alloc::vec![("asset", obj(alloc::vec![("version", Value::string("2.0")), ("generator", Value::string(generator))]))];
    if let Some(s) = doc.scene.filter(|&s| s < doc.scenes.len()) {
        root.push(("scene", uint(s)));
    }
    let nonempty = |v: Vec<Value>| if v.is_empty() { None } else { Some(Value::Array(v)) };
    for (key, v) in [("scenes", scenes), ("nodes", nodes), ("meshes", meshes), ("materials", materials)] {
        if let Some(a) = nonempty(v) {
            root.push((key, a));
        }
    }
    while bin.data.len() % 4 != 0 {
        bin.data.push(0);
    }
    if !bin.data.is_empty() {
        root.push(("buffers", Value::Array(alloc::vec![obj(alloc::vec![("byteLength", uint(bin.data.len()))])])));
        root.push(("bufferViews", Value::Array(bin.views)));
        root.push(("accessors", Value::Array(bin.accessors)));
    }
    // the document is ours, not untrusted input: no limits but the nesting (at most 5 levels here)
    let big = usize::MAX / 2;
    let limits = json::Limits { bytes: big, depth: 32, values: big, string_bytes: big, members: big };
    let text = json::stringify_with_limits(&obj(root), limits).map_err(|e| Error::Invalid(format!("JSON: {} at {}", e.kind, e.offset)))?;
    Ok(write_glb(text.as_bytes(), if bin.data.is_empty() { None } else { Some(&bin.data) }))
}
