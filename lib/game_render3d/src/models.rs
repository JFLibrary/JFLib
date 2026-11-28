// models -- (G7b part 2) assets for render3d: glTF models and images through lib/game's Assets, on the GPU once
// ready, and entities that draw a model (docs/GAMECORE.md "Assets")
//
//   Model (an Asset)           a .glb / .gltf as lib/gltf reads it (its buffers and images through the VFS beside
//                              it), decoded on the loader's thread
//   Image (an Asset)           a PNG or JPEG as straight rgba rows
//   ModelRef (a component)     an entity draws the model of its handle at its Pose, every instance of the model's
//                              scene (Bridge::sync_models); nothing while the model loads or failed
//   Models                     the models on a Renderer: update(&assets, &changes, &mut renderer) puts what became
//                              Ready on the GPU (meshes, textures, materials) and takes off what was unloaded
//                              (Renderer::remove_mesh / remove_material / remove_texture; what instances still use
//                              comes off with the next update, waiting()); parts(id)
//   Textures                   the same for images: texture(id) -> TextureId once it is up
//   Bridge::sync_models        entities with a ModelRef as instances: they appear when their model is up, follow
//                              their poses (only what changed), go with the entity, the component or the model
//
// A model's lights and cameras are not placed (render3d::gltf::upload does that for a whole scene); a node's shear
// is dropped as there.  An entity's transform and a part's compose as translation, rotation and per-axis scale
// (exact for uniform scales; a non-uniform scale under a rotated part would need shear).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::assets::{Asset, AssetId, Assets, Change, Handle};
use game::{Entity, Pose, World};
use gpu::{GpuError, Result};
use render3d::{Instance, InstanceId, Material, MaterialDesc, MaterialId, MeshData, MeshId, Mip, Renderer, Sampling, Scene, TextureData, TextureId,
               TextureRef, Wrap};

use crate::{Bridge, SyncStats};

/// a glTF model (an Asset): lib/gltf's Document
pub struct Model {
    pub doc: ::gltf::Document,
}

impl Asset for Model {
    fn decode(bytes: &[u8], files: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> core::result::Result<Self, String> {
        ::gltf::load(bytes, files, &::gltf::Limits::DEFAULT).map(|doc| Model { doc }).map_err(|e| format!("{e}"))
    }
}

/// an image (an Asset): straight rgba rows
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// the largest image an Image asset takes (pixels)
pub const IMAGE_PIXELS: u64 = 1 << 26;

impl Asset for Image {
    fn decode(data: &[u8], _files: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> core::result::Result<Self, String> {
        use img_core::{ImgMeta, JDF_NO_ORIENT, JDF_STRAIGHT};
        let ok = |w: u64, h: u64| w > 0 && h > 0 && w.saturating_mul(h) <= IMAGE_PIXELS;
        let sf = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            let (w, h) = match data.get(16..24) {
                Some(b) => (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64, u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as u64),
                None => return Err(String::from("a PNG cut off before its size")),
            };
            if !ok(w, h) {
                return Err(format!("a PNG of {w} x {h} pixels"));
            }
            let mut sf = img_core::Surface::default();
            let r = img_png::dec::decode_lim(data, &mut sf, JDF_STRAIGHT, w * h * 12 + h + 65536);
            if r != 0 {
                return Err(format!("the PNG does not decode (error {r})"));
            }
            sf
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            let mut m = ImgMeta::default();
            if img_jpeg::meta(data, &mut m, 0) != 0 {
                return Err(String::from("a JPEG without a readable frame header"));
            }
            if !ok(m.w as u64, m.h as u64) {
                return Err(format!("a JPEG of {} x {} pixels", m.w, m.h));
            }
            img_jpeg::decode_fl_max(data, JDF_STRAIGHT | JDF_NO_ORIENT, m.w.max(1), m.h.max(1)).map_err(|e| format!("the JPEG does not decode (error {e})"))?
        } else {
            return Err(String::from("neither PNG nor JPEG"));
        };
        let (w, h, stride) = (sf.w.max(0) as usize, sf.h.max(0) as usize, sf.stride.max(0) as usize);
        if w == 0 || h == 0 || stride < w * 4 || sf.px.len() < stride * (h - 1) + w * 4 {
            return Err(String::from("the decoder returned no usable pixels"));
        }
        // bytes b, g, r, a -> r, g, b, a
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for p in sf.px[y * stride..y * stride + w * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        Ok(Image { width: w as u32, height: h as u32, rgba })
    }
}

/// what an entity looks like: the model of a handle (all instances of its default scene), at its Pose
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRef {
    pub model: Handle<Model>,
}

/// a part of a model on the GPU: one instance of its scene
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub mesh: MeshId,
    pub material: MaterialId,
    /// where the scene puts it (model space)
    pub local: render3d::Transform,
}

/// a model on the GPU
struct Uploaded {
    /// which upload of Models it is (a reload makes another: the bridge builds its entities anew)
    serial: u64,
    parts: Vec<Part>,
    meshes: Vec<MeshId>,
    materials: Vec<MaterialId>,
    textures: Vec<TextureId>,
}

/// what an update of Models did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelStats {
    pub uploaded: u32,
    pub removed: u32,
    /// models on the GPU now, their parts
    pub models: u32,
    pub parts: u32,
}

/// the models of an Assets<Model> on one renderer (see the module header)
#[derive(Default)]
pub struct Models {
    up: BTreeMap<AssetId, Uploaded>,
    /// counts every change of what is up (the bridge looks again when it moved)
    version: u64,
    /// the ready models of the store were not all put up yet (a new renderer)
    fresh: bool,
    /// what the renderer did not take off yet
    waiting: Waiting,
}

fn sampling(s: &::gltf::Sampler) -> Sampling {
    let wrap = |w: ::gltf::Wrap| match w {
        ::gltf::Wrap::Repeat => Wrap::Repeat,
        ::gltf::Wrap::ClampToEdge => Wrap::Clamp,
        ::gltf::Wrap::MirroredRepeat => Wrap::Mirror,
    };
    Sampling {
        mag_nearest: s.mag == ::gltf::Filter::Nearest,
        min_nearest: s.min == ::gltf::Filter::Nearest,
        mip: match s.mip {
            None => Mip::None,
            Some(::gltf::Filter::Nearest) => Mip::Nearest,
            Some(::gltf::Filter::Linear) => Mip::Linear,
        },
        wrap_u: wrap(s.wrap_s),
        wrap_v: wrap(s.wrap_t),
    }
}

/// a document on the renderer, everything it made recorded (render3d::gltf::upload's way, which does not say which
/// textures it made)
fn upload(r: &mut Renderer, doc: &::gltf::Document) -> Result<Uploaded> {
    let mut u = Uploaded { serial: 0, parts: Vec::new(), meshes: Vec::new(), materials: Vec::new(), textures: Vec::new() };
    let r2 = &mut *r;
    let result = (|| -> Result<()> {
        let mut prims: Vec<Vec<MeshId>> = Vec::new();
        for m in &doc.meshes {
            let mut ids = Vec::new();
            for p in &m.primitives {
                let id = r2.add_mesh(&MeshData { positions: &p.positions, normals: &p.normals, uvs: &p.uvs, indices: &p.indices })?;
                u.meshes.push(id);
                ids.push(id);
            }
            prims.push(ids);
        }
        let mut made: BTreeMap<(usize, bool, bool), TextureId> = BTreeMap::new();
        let mut materials = Vec::new();
        for m in &doc.materials {
            let mut tex = |r: &mut Renderer, u: &mut Uploaded, t: &Option<::gltf::TextureRef>, srgb: bool| -> Result<Option<TextureRef>> {
                let Some(t) = t else { return Ok(None) };
                let Some(texture) = doc.textures.get(t.texture) else { return Ok(None) };
                let Some((i, im)) = texture.image.and_then(|i| doc.images.get(i).map(|im| (i, im))) else { return Ok(None) };
                if im.width == 0 {
                    return Ok(None);
                }
                let nearest = texture.sampler.mag == ::gltf::Filter::Nearest;
                let id = match made.get(&(i, srgb, nearest)) {
                    Some(id) => *id,
                    None => {
                        let id = r.add_texture(&TextureData { width: im.width, height: im.height, rgba: &im.rgba, srgb, nearest })?;
                        u.textures.push(id);
                        made.insert((i, srgb, nearest), id);
                        id
                    }
                };
                Ok(Some(TextureRef { texture: id, sampling: sampling(&texture.sampler) }))
            };
            let mut mat = Material::color(1.0, 1.0, 1.0);
            mat.base_color = m.base_color_factor;
            mat.metallic = m.metallic_factor;
            mat.roughness = m.roughness_factor;
            mat.emissive = m.emissive_factor;
            mat.normal_scale = m.normal_scale;
            mat.occlusion_strength = m.occlusion_strength;
            mat.base_color_texture = tex(r2, &mut u, &m.base_color_texture, true)?;
            mat.metallic_roughness_texture = tex(r2, &mut u, &m.metallic_roughness_texture, false)?;
            mat.normal_texture = tex(r2, &mut u, &m.normal_texture, false)?;
            mat.occlusion_texture = tex(r2, &mut u, &m.occlusion_texture, false)?;
            mat.emissive_texture = tex(r2, &mut u, &m.emissive_texture, true)?;
            let mut d = MaterialDesc::new(mat);
            match m.alpha_mode {
                ::gltf::AlphaMode::Opaque => {}
                ::gltf::AlphaMode::Mask => d.material.alpha_cutoff = Some(m.alpha_cutoff),
                ::gltf::AlphaMode::Blend => d.blend = true,
            }
            d.double_sided = m.double_sided;
            let id = r2.add_material_desc(&d)?;
            u.materials.push(id);
            materials.push(id);
        }
        let mut default_material = None;
        for i in doc.instances(None) {
            let (Some(prim), Some(&mesh)) = (doc.meshes.get(i.mesh).and_then(|m| m.primitives.get(i.primitive)), prims.get(i.mesh).and_then(|m| m.get(i.primitive))) else {
                return Err(GpuError::Invalid("game_render3d: an instance of a primitive the model lacks"));
            };
            if !i.world.iter().all(|v| v.is_finite()) {
                return Err(GpuError::Invalid("game_render3d: a world matrix that is not finite"));
            }
            let material = match prim.material {
                Some(m) => *materials.get(m).ok_or(GpuError::Invalid("game_render3d: a material the model lacks"))?,
                None => match default_material {
                    Some(d) => d,
                    None => {
                        let d = r2.add_material(&Material { metallic: 1.0, ..Material::color(1.0, 1.0, 1.0) })?;
                        u.materials.push(d);
                        default_material = Some(d);
                        d
                    }
                },
            };
            let trs = ::gltf::decompose(&i.world);
            let q = trs.rotation;
            let local = render3d::Transform { pos: trs.translation, rot: [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32],
                                              scale: [trs.scale[0] as f32, trs.scale[1] as f32, trs.scale[2] as f32] };
            u.parts.push(Part { mesh, material, local });
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(u),
        Err(e) => {
            // (what went up of it comes off again)
            remove(r, &u);
            Err(e)
        }
    }
}

fn remove(r: &mut Renderer, u: &Uploaded) {
    Waiting::default().remove(r, &u.materials, &u.meshes, &u.textures);
}

/// what came off a store but not yet off the renderer, which refused it: instances of a scene still used it (the
/// bridge's, which sync_models drops after the update) or a material of the app named a texture.  Each update tries
/// again; dropped, the ids were lost and the GPU memory with them
#[derive(Default)]
struct Waiting {
    materials: Vec<MaterialId>,
    meshes: Vec<MeshId>,
    textures: Vec<TextureId>,
}

impl Waiting {
    /// these off the renderer (materials first: they hold textures); what it refuses and still has waits
    fn remove(&mut self, r: &mut Renderer, materials: &[MaterialId], meshes: &[MeshId], textures: &[TextureId]) {
        for &m in materials {
            if r.remove_material(m).is_err() && r.has_material(m) {
                self.materials.push(m);
            }
        }
        for &m in meshes {
            if r.remove_mesh(m).is_err() && r.mesh_info(m).is_some() {
                self.meshes.push(m);
            }
        }
        for &t in textures {
            if r.remove_texture(t).is_err() && r.texture_info(t).is_some() {
                self.textures.push(t);
            }
        }
    }

    /// the next try
    fn retry(&mut self, r: &mut Renderer) {
        if self.len() > 0 {
            let old = core::mem::take(self);
            self.remove(r, &old.materials, &old.meshes, &old.textures);
        }
    }

    fn len(&self) -> usize {
        self.materials.len() + self.meshes.len() + self.textures.len()
    }
}

impl Models {
    pub fn new() -> Models {
        Models { up: BTreeMap::new(), version: 0, fresh: true, waiting: Waiting::default() }
    }

    /// once a frame, after Assets::update (its changes) and before the bridge: what became Ready goes up, what was
    /// unloaded comes off.  The first update on a renderer (and the first after reset) puts every Ready model up.
    /// A model the renderer refuses counts as not up (its entities show nothing); the first error is returned after
    /// everything else was done.  What the renderer will not take off yet (the bridge's instances still use it until
    /// sync_models) comes off with the next update
    pub fn update(&mut self, assets: &Assets<Model>, changes: &[Change], r: &mut Renderer) -> Result<ModelStats> {
        let mut st = ModelStats::default();
        let mut first_err = None;
        let mut want: Vec<AssetId> = Vec::new();
        self.waiting.retry(r);
        if self.fresh {
            self.fresh = false;
            want.extend(assets.ids().into_iter().filter(|id| assets.get_id(*id).is_some()));
        }
        for c in changes {
            match *c {
                Change::Ready(id) => {
                    // (a reload: the old one comes off first)
                    if let Some(u) = self.up.remove(&id) {
                        self.waiting.remove(r, &u.materials, &u.meshes, &u.textures);
                        st.removed += 1;
                        self.version += 1;
                    }
                    want.push(id);
                }
                Change::Unloaded(id) | Change::Failed(id) => {
                    if let Some(u) = self.up.remove(&id) {
                        self.waiting.remove(r, &u.materials, &u.meshes, &u.textures);
                        st.removed += 1;
                        self.version += 1;
                    }
                }
            }
        }
        for id in want {
            if self.up.contains_key(&id) {
                continue;
            }
            let Some(m) = assets.get_id(id) else { continue };
            match upload(r, &m.doc) {
                Ok(mut u) => {
                    self.version += 1;
                    u.serial = self.version;
                    self.up.insert(id, u);
                    st.uploaded += 1;
                }
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        st.models = self.up.len() as u32;
        st.parts = self.up.values().map(|u| u.parts.len() as u32).sum();
        match first_err {
            Some(e) => Err(e),
            None => Ok(st),
        }
    }

    /// the parts of a model that is up
    pub fn parts(&self, id: AssetId) -> Option<&[Part]> {
        self.up.get(&id).map(|u| &u.parts[..])
    }

    /// which upload a model that is up came from (another one after a reload or a new renderer)
    pub fn serial(&self, id: AssetId) -> Option<u64> {
        self.up.get(&id).map(|u| u.serial)
    }

    /// the meshes, materials and textures a model made on the renderer
    pub fn made(&self, id: AssetId) -> Option<(&[MeshId], &[MaterialId], &[TextureId])> {
        self.up.get(&id).map(|u| (&u.meshes[..], &u.materials[..], &u.textures[..]))
    }

    /// counts every change of what is up
    pub fn version(&self) -> u64 {
        self.version
    }

    /// models up now
    pub fn len(&self) -> usize {
        self.up.len()
    }

    pub fn is_empty(&self) -> bool {
        self.up.is_empty()
    }

    /// meshes, materials and textures of models that came off and wait for the renderer to let them go (the next
    /// update, once no instance uses them)
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// the renderer went with its device: forget what was up (the next update puts every Ready model up again)
    pub fn reset(&mut self) {
        self.up.clear();
        self.waiting = Waiting::default();
        self.fresh = true;
        self.version += 1;
    }
}

/// the images of an Assets<Image> on one renderer (sRGB colour textures unless set otherwise)
#[derive(Default)]
pub struct Textures {
    up: BTreeMap<AssetId, TextureId>,
    fresh: bool,
    /// the images are data, not colour (normal maps, masks): linear
    pub linear: bool,
    pub nearest: bool,
    /// textures the renderer did not take off yet (a material still names them)
    waiting: Waiting,
}

impl Textures {
    pub fn new() -> Textures {
        Textures { up: BTreeMap::new(), fresh: true, linear: false, nearest: false, waiting: Waiting::default() }
    }

    /// once a frame after Assets::update: Ready images up, unloaded ones off -> textures put up now.  A texture that
    /// a material still names stays on the renderer until the material lets it go; each update tries again
    pub fn update(&mut self, assets: &Assets<Image>, changes: &[Change], r: &mut Renderer) -> Result<u32> {
        let mut want: Vec<AssetId> = Vec::new();
        self.waiting.retry(r);
        if self.fresh {
            self.fresh = false;
            want.extend(assets.ids().into_iter().filter(|id| assets.get_id(*id).is_some()));
        }
        for c in changes {
            match *c {
                Change::Ready(id) => {
                    if let Some(t) = self.up.remove(&id) {
                        self.waiting.remove(r, &[], &[], &[t]);
                    }
                    want.push(id);
                }
                Change::Unloaded(id) | Change::Failed(id) => {
                    if let Some(t) = self.up.remove(&id) {
                        self.waiting.remove(r, &[], &[], &[t]);
                    }
                }
            }
        }
        let mut n = 0;
        for id in want {
            if self.up.contains_key(&id) {
                continue;
            }
            let Some(im) = assets.get_id(id) else { continue };
            let t = r.add_texture(&TextureData { width: im.width, height: im.height, rgba: &im.rgba, srgb: !self.linear, nearest: self.nearest })?;
            self.up.insert(id, t);
            n += 1;
        }
        Ok(n)
    }

    /// the texture of an image once it is up
    pub fn texture(&self, id: AssetId) -> Option<TextureId> {
        self.up.get(&id).copied()
    }

    /// textures of images that came off and wait for the renderer to let them go
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// the renderer went: forget every texture (the next update puts them up again)
    pub fn reset(&mut self) {
        self.up.clear();
        self.waiting = Waiting::default();
        self.fresh = true;
    }
}

/// an entity's model on the scene: its instances, the model's version they were made of
#[derive(Clone, Debug)]
pub(crate) struct ModelSlot {
    entity: Entity,
    asset: AssetId,
    /// Models::serial of the upload its instances show
    serial: u64,
    insts: Vec<InstanceId>,
    /// the entity's transform written last
    t: render3d::Transform,
}

/// a part at an entity's transform
fn compose(e: &render3d::Transform, p: &render3d::Transform) -> render3d::Transform {
    let rot = game::Quat::from_f32(e.rot);
    let local = game::Vec3::new(p.pos[0] * e.scale[0] as f64, p.pos[1] * e.scale[1] as f64, p.pos[2] * e.scale[2] as f64);
    let d = rot.rotate(local);
    let r = rot.mul(game::Quat::from_f32(p.rot)).normalize();
    render3d::Transform {
        pos: [e.pos[0] + d.x, e.pos[1] + d.y, e.pos[2] + d.z],
        rot: r.to_f32(),
        scale: [e.scale[0] * p.scale[0], e.scale[1] * p.scale[1], e.scale[2] * p.scale[2]],
    }
}

impl Bridge {
    /// an entity's instances leave the scene (all it can remove; the first error after them)
    pub(crate) fn drop_model(&mut self, index: usize, scene: &mut Scene) -> Result<()> {
        let mut r = Ok(());
        if let Some(s) = self.models[index].take() {
            for i in s.insts {
                if let Err(e) = scene.remove(i) {
                    r = r.and(Err(e));
                }
            }
        }
        r
    }

    // entity e's model into the scene (its parts as instances at t), all or none
    fn put_model(&mut self, e: Entity, m: &ModelRef, t: render3d::Transform, models: &Models, scene: &mut Scene, st: &mut SyncStats) -> Result<()> {
        let i = e.index() as usize;
        if self.models.len() <= i {
            self.models.resize(i + 1, None);
        }
        if self.models[i].as_ref().is_some_and(|s| s.entity == e && s.asset == m.model.id() && models.serial(s.asset) == Some(s.serial)) {
            return Ok(());
        }
        if let Some(s) = &self.models[i] {
            st.removed += s.insts.len() as u32;
            self.drop_model(i, scene)?;
        }
        let (Some(parts), Some(serial)) = (models.parts(m.model.id()), models.serial(m.model.id())) else { return Ok(()) };
        let mut insts = Vec::with_capacity(parts.len());
        for p in parts {
            match scene.add(Instance { mesh: p.mesh, material: p.material, transform: compose(&t, &p.local) }) {
                Ok(id) => insts.push(id),
                Err(err) => {
                    // (the parts added so far go again: nothing held them)
                    for id in insts {
                        let _ = scene.remove(id);
                    }
                    return Err(err);
                }
            }
        }
        st.added += insts.len() as u32;
        self.models[i] = Some(ModelSlot { entity: e, asset: m.model.id(), serial, insts, t });
        Ok(())
    }

    /// the entities with a ModelRef into the scene for a frame at `alpha` (see the module header): an entity whose
    /// model is up (Models) gets an instance per part, at its Pose composed with the part's place; a model that is
    /// not up shows nothing; a pose that changed moves every part.  Instances it adds are the bridge's (clear,
    /// reset); the counts as sync's, an entity render3d refuses left out as there
    pub fn sync_models(&mut self, world: &World, scene: &mut Scene, alpha: f64, models: &Models) -> Result<SyncStats> {
        let mut st = SyncStats::default();
        let refs = world.column::<ModelRef>();
        let poses = world.column::<Pose>();
        let at = |e: Entity| crate::transform(&poses.and_then(|p| p.get(e)).map_or(game::Transform::IDENTITY, |p| p.interpolate(alpha)));
        let all = models.version() != self.models_version;
        // a slot still right: its entity has a ModelRef of its asset, whose upload it shows
        let valid = |s: &ModelSlot| refs.and_then(|m| m.get(s.entity)).is_some_and(|m| m.model.id() == s.asset) && models.serial(s.asset) == Some(s.serial);
        // gone or stale: entities that left the column, models that came off or were put up anew
        let removals = refs.map_or(0, |m| m.removals());
        if all || removals != self.model_removals || (refs.is_none() && self.models.iter().any(|s| s.is_some())) {
            for i in 0..self.models.len() {
                if let Some(s) = &self.models[i] {
                    if !valid(s) {
                        st.removed += s.insts.len() as u32;
                        if self.drop_model(i, scene).is_err() {
                            st.failed += 1;
                        }
                    }
                }
            }
            self.model_removals = removals;
        }
        // new and changed ModelRefs (all of them when what is up changed), then the ones refused at the last sync
        let mut retry = core::mem::take(&mut self.model_retry);
        if let Some(refs) = refs.filter(|m| all || m.version() > self.model_seen) {
            let since = if all { 0 } else { self.model_seen };
            for (e, m) in refs.changed_since(since) {
                retry.retain(|x| *x != e);
                if self.put_model(e, m, at(e), models, scene, &mut st).is_err() {
                    st.failed += 1;
                    self.model_retry.push(e);
                }
            }
            self.model_seen = refs.version();
        }
        for e in retry {
            if let Some(m) = refs.and_then(|c| c.get(e)) {
                if self.put_model(e, m, at(e), models, scene, &mut st).is_err() {
                    st.failed += 1;
                    self.model_retry.push(e);
                }
            }
        }
        self.models_version = models.version();
        // poses that changed (or moved in the latest tick): every part of the entity's model
        let since = self.model_pose_seen.min(world.pose_mark());
        if let Some(poses) = poses.filter(|p| p.version() > since) {
            for (e, p) in poses.changed_since(since) {
                let Some(Some(s)) = self.models.get_mut(e.index() as usize) else { continue };
                if s.entity != e {
                    continue;
                }
                st.visited += 1;
                let t = crate::transform(&p.interpolate(alpha));
                if t == s.t {
                    continue;
                }
                let Some(parts) = models.parts(s.asset) else { continue };
                let mut ok = true;
                for (inst, part) in s.insts.iter().zip(parts.iter()) {
                    if scene.set_transform(*inst, &compose(&t, &part.local)).is_err() {
                        ok = false;
                        break;
                    }
                    st.moved += 1;
                }
                if ok {
                    s.t = t;
                } else {
                    st.failed += 1;
                }
            }
            self.model_pose_seen = poses.version();
        }
        Ok(st)
    }

    /// entities drawn through sync_models now
    pub fn model_entities(&self) -> u32 {
        self.models.iter().filter(|s| s.is_some()).count() as u32
    }
}
