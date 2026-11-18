// export -- the model as glTF: one mesh with one primitive per part, its material by its factors (the
// script's sRGB colour as glTF's linear baseColorFactor), one node per part named like it, one scene named
// like the model; written by lib/gltf's write_document.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::host::Model;

/// sRGB 0 ..= 1 to linear (glTF's and render3d's colours are linear); rt's portable pow, the same everywhere
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { pow((c + 0.055) / 1.055, 2.4) }
}

#[cfg(not(target_arch = "wasm32"))]
fn pow(x: f64, y: f64) -> f64 {
    rt::portable::math::pow(x, y)
}

#[cfg(target_arch = "wasm32")]
fn pow(x: f64, y: f64) -> f64 {
    rt::x87::pow(x, y)
}

/// the model as a glTF document
pub fn document(model: &Model) -> gltf::Document {
    let mut doc = gltf::Document {
        version: String::from("2.0"),
        generator: format!("JFLib modeltool {}", crate::APP_VERSION),
        ..gltf::Document::default()
    };
    for m in &model.materials {
        let c = m.color.map(|v| srgb_to_linear(v) as f32);
        doc.materials.push(gltf::Material {
            name: m.name.clone(),
            base_color_factor: [c[0], c[1], c[2], 1.0],
            metallic_factor: m.metallic as f32,
            roughness_factor: m.roughness as f32,
            ..gltf::Material::default()
        });
    }
    for (i, p) in model.parts.iter().enumerate() {
        doc.meshes.push(gltf::Mesh {
            name: p.name.clone(),
            primitives: vec![gltf::Primitive {
                positions: p.mesh.positions_f32(),
                normals: p.mesh.normals_f32(),
                uvs: p.mesh.uvs_f32(),
                indices: p.mesh.indices.clone(),
                material: Some(p.material),
                flat_normals: false,
                has_uvs: true,
            }],
        });
        doc.nodes.push(gltf::Node {
            name: p.name.clone(),
            mesh: Some(i),
            children: Vec::new(),
            parent: None,
            local: gltf::IDENTITY,
            world: gltf::IDENTITY,
            light: None,
        });
    }
    doc.scenes.push(gltf::SceneDef { name: model.name.clone(), nodes: (0..model.parts.len()).collect() });
    doc.scene = Some(0);
    doc
}

/// the model as a .glb
pub fn glb(model: &Model) -> Result<Vec<u8>, String> {
    gltf::write_document(&document(model)).map_err(|e| format!("glTF: {:?}", e))
}
