// gltf -- a jflib-gltf Document into a Renderer (G5b; docs/GLTF.md): each primitive as a mesh, each image as a
// texture (once per image, colour space and filter: base colour and emissive sRGB, the others linear), each
// material as a row, and the instances of a scene with their world matrices taken apart (gltf::decompose)
// for Scene::add.  What the engine cannot show yet is counted in notes: BLEND drawn opaque, double-sided
// materials drawn from the front, uv sets other than 0, world matrices with shear (drawn without it).
//
//   let doc = ::gltf::load(&bytes, &mut |uri| read(dir, uri), &::gltf::Limits::DEFAULT)?;
//   let imported = render3d::gltf::upload(&mut renderer, &doc, None)?;
//   for i in &imported.instances { scene.add(*i)?; }

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ::gltf as g;
use gpu::*;

use crate::{Instance, Material, MaterialId, MeshData, MeshId, Mip, Renderer, Sampling, TextureData, TextureId, TextureRef, Transform, Wrap};

/// what upload made of a Document
#[derive(Clone, Debug, Default)]
pub struct Imported {
    /// the renderer's mesh of each primitive: meshes[mesh][primitive]
    pub meshes: Vec<Vec<MeshId>>,
    /// the renderer's material of each of the document's, and glTF's default material for primitives without
    pub materials: Vec<MaterialId>,
    pub default_material: Option<MaterialId>,
    /// textures uploaded
    pub textures: u32,
    /// the scene's instances, for Scene::add
    pub instances: Vec<Instance>,
    /// instances whose world matrix has shear, which a Transform cannot hold (the largest |cos| between
    /// two axes above 1e-4); drawn without it
    pub sheared: u32,
    /// what the engine leaves out (once each)
    pub notes: Vec<String>,
}

fn sampling(s: &g::Sampler) -> Sampling {
    let wrap = |w: g::Wrap| match w {
        g::Wrap::Repeat => Wrap::Repeat,
        g::Wrap::ClampToEdge => Wrap::Clamp,
        g::Wrap::MirroredRepeat => Wrap::Mirror,
    };
    Sampling {
        mag_nearest: s.mag == g::Filter::Nearest,
        min_nearest: s.min == g::Filter::Nearest,
        mip: match s.mip {
            None => Mip::None,
            Some(g::Filter::Nearest) => Mip::Nearest,
            Some(g::Filter::Linear) => Mip::Linear,
        },
        wrap_u: wrap(s.wrap_s),
        wrap_v: wrap(s.wrap_t),
    }
}

/// the document's meshes, textures, materials, and the instances of `scene` (None: the document's)
pub fn upload(r: &mut Renderer, doc: &g::Document, scene: Option<usize>) -> Result<Imported> {
    let mut out = Imported::default();
    let note = |out: &mut Imported, s: String| {
        if !out.notes.contains(&s) {
            out.notes.push(s);
        }
    };
    for m in &doc.meshes {
        let mut ids = Vec::with_capacity(m.primitives.len());
        for p in &m.primitives {
            ids.push(r.add_mesh(&MeshData { positions: &p.positions, normals: &p.normals, uvs: &p.uvs, indices: &p.indices })?);
        }
        out.meshes.push(ids);
    }
    // textures by (image, sRGB, nearest): an image used both ways goes up twice
    let mut uploaded: BTreeMap<(usize, bool, bool), TextureId> = BTreeMap::new();
    let mut tex = |r: &mut Renderer, out: &mut Imported, t: &Option<g::TextureRef>, srgb: bool| -> Result<Option<TextureRef>> {
        let Some(t) = t else { return Ok(None) };
        let Some(texture) = doc.textures.get(t.texture) else { return Ok(None) };
        let Some(image) = texture.image.and_then(|i| doc.images.get(i).map(|im| (i, im))) else { return Ok(None) };
        if image.1.width == 0 {
            return Ok(None);
        }
        if t.tex_coord != 0 {
            note(out, String::from("textures on uv sets other than 0 are drawn with uv set 0"));
        }
        let nearest = texture.sampler.mag == g::Filter::Nearest;
        let id = match uploaded.get(&(image.0, srgb, nearest)) {
            Some(id) => *id,
            None => {
                let id = r.add_texture(&TextureData { width: image.1.width, height: image.1.height, rgba: &image.1.rgba, srgb, nearest })?;
                uploaded.insert((image.0, srgb, nearest), id);
                out.textures += 1;
                id
            }
        };
        Ok(Some(TextureRef { texture: id, sampling: sampling(&texture.sampler) }))
    };
    for m in &doc.materials {
        let mut mat = Material::color(1.0, 1.0, 1.0);
        mat.base_color = m.base_color_factor;
        mat.metallic = m.metallic_factor;
        mat.roughness = m.roughness_factor;
        mat.emissive = m.emissive_factor;
        mat.normal_scale = m.normal_scale;
        mat.occlusion_strength = m.occlusion_strength;
        mat.base_color_texture = tex(r, &mut out, &m.base_color_texture, true)?;
        mat.metallic_roughness_texture = tex(r, &mut out, &m.metallic_roughness_texture, false)?;
        mat.normal_texture = tex(r, &mut out, &m.normal_texture, false)?;
        mat.occlusion_texture = tex(r, &mut out, &m.occlusion_texture, false)?;
        mat.emissive_texture = tex(r, &mut out, &m.emissive_texture, true)?;
        match m.alpha_mode {
            g::AlphaMode::Opaque => {}
            g::AlphaMode::Mask => mat.alpha_cutoff = Some(m.alpha_cutoff),
            g::AlphaMode::Blend => note(&mut out, String::from("BLEND materials are drawn opaque")),
        }
        if m.double_sided {
            note(&mut out, String::from("double-sided materials are drawn from the front only"));
        }
        if mat.normal_texture.is_some() {
            note(&mut out, String::from("normal maps take the triangle's tangent frame from derivatives, not MikkTSpace (docs/RENDER3D.md)"));
        }
        out.materials.push(r.add_material(&mat)?);
    }
    // (a Document's fields are public: one built by hand is checked here, not trusted)
    let insts = doc.instances(scene);
    for i in &insts {
        let Some(prim) = doc.meshes.get(i.mesh).and_then(|m| m.primitives.get(i.primitive)) else {
            return Err(GpuError::Invalid("gltf::upload: an instance of a primitive the document lacks"));
        };
        let Some(&mesh) = out.meshes.get(i.mesh).and_then(|m| m.get(i.primitive)) else {
            return Err(GpuError::Invalid("gltf::upload: an instance of a primitive the document lacks"));
        };
        if !i.world.iter().all(|v| v.is_finite()) {
            return Err(GpuError::Invalid("gltf::upload: a world matrix that is not finite"));
        }
        let material = match prim.material {
            Some(m) => *out.materials.get(m).ok_or(GpuError::Invalid("gltf::upload: a primitive's material the document lacks"))?,
            None => match out.default_material {
                Some(d) => d,
                None => {
                    let d = r.add_material(&Material { metallic: 1.0, ..Material::color(1.0, 1.0, 1.0) })?;
                    out.default_material = Some(d);
                    d
                }
            },
        };
        let trs = g::decompose(&i.world);
        if trs.shear > 1e-4 {
            out.sheared += 1;
        }
        let q = trs.rotation;
        let transform = Transform { pos: trs.translation, rot: [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32],
                                    scale: [trs.scale[0] as f32, trs.scale[1] as f32, trs.scale[2] as f32] };
        out.instances.push(Instance { mesh, material, transform });
    }
    if out.sheared > 0 {
        let s = format!("{} instances with shear in their world matrix, drawn without it", out.sheared);
        note(&mut out, s);
    }
    Ok(out)
}
