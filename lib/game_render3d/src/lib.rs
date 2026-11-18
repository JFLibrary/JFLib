// game_render3d -- the bridge from the game core (lib/game) to render3d: entities with a mesh are
// instances of a render3d scene, only what changed is written, the scene's origin follows the camera
//
//   MeshRef            the component: a render3d mesh and material (an entity with one is drawn, at its
//                      Pose, or at the origin without one)
//   Bridge::sync       the world into the scene: new MeshRefs are added (Scene::add), changed ones
//                      switched (set_mesh, set_material), gone ones removed (Scene::remove), poses that
//                      changed written (set_transform) at the frame's alpha (Pose::interpolate); a pose
//                      between two ticks is written every frame, a standing one once
//   Bridge::follow     the scene's origin (Scene::set_origin) follows the camera's eye in steps of
//                      origin_step metres (512 unless set): f32 on the GPU stays fine near the camera
//   Bridge::retire_* / collect   meshes and materials the game is done with go from the renderer
//                      (Renderer::remove_mesh / remove_material) once no instance of the bridge uses them
//   camera, transform  a game View as render3d::Camera, a game Transform as render3d::Transform
//
// Why a crate of its own and not a module of lib/game behind a feature: a game's server is built from the
// same workspace as its client, and Cargo unifies a crate's features over everything one `cargo build`
// builds (build_rs.py builds an app and its BINS together).  A feature "render3d" of lib/game would put
// render3d, gpu and their link directives into the server's program; a crate the server does not name
// never enters its graph.  The library has no features besides std either (docs/CRATES.md).
//
// What the bridge reads: the versions of the MeshRef and Pose columns (SparseSet::changed_since), the
// MeshRef column's removals (a sweep over its own entries only when something left), World::pose_mark
// (the poses that moved in the latest tick: between two states, written every frame).  Per frame that is
// a scan of one u64 per pose while something moved in the latest tick (none when no column changed), plus
// the work for what changed.  The bridge keeps per entity index its instance and the transform it wrote
// last: a write that would change nothing is left out.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use game::{Entity, Pose, View, World};
use gpu::{GpuError, Result};
use render3d::{Camera, Instance, InstanceId, MaterialId, MeshId, Renderer, Scene};

/// what an entity looks like: a mesh and a material of the renderer the scene belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshRef {
    /// the mesh (Renderer::add_mesh)
    pub mesh: MeshId,
    /// the material (Renderer::add_material)
    pub material: MaterialId,
}

/// what a sync did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// instances added (Scene::add)
    pub added: u32,
    /// instances removed (Scene::remove)
    pub removed: u32,
    /// meshes or materials switched (Scene::set_mesh, set_material)
    pub switched: u32,
    /// transforms written (Scene::set_transform)
    pub moved: u32,
    /// poses looked at (changed since the last sync, or between two ticks)
    pub visited: u32,
}

/// the origin's step unless set (metres): f32 has a step of 0.06 mm at 1 km from the origin
pub const ORIGIN_STEP: f64 = 512.0;

#[derive(Clone, Copy, Debug)]
struct Slot {
    entity: Entity,
    inst: InstanceId,
    mesh: MeshId,
    material: MaterialId,
    /// the transform written last
    t: render3d::Transform,
}

/// the world's entities with a MeshRef as instances of one scene (see the module header)
#[derive(Debug)]
pub struct Bridge {
    /// by entity index
    slots: Vec<Option<Slot>>,
    live: u32,
    /// the MeshRef column's version and removals at the last sync
    mesh_seen: u64,
    mesh_removals: u64,
    /// the Pose column's version at the last sync
    pose_seen: u64,
    /// instances per mesh and material (by id)
    mesh_use: BTreeMap<u32, u32>,
    material_use: BTreeMap<u32, u32>,
    retired_meshes: Vec<MeshId>,
    retired_materials: Vec<MaterialId>,
    origin_step: f64,
}

impl Default for Bridge {
    fn default() -> Self {
        Bridge::new()
    }
}

/// a game transform as render3d's (the position stays f64)
pub fn transform(t: &game::Transform) -> render3d::Transform {
    render3d::Transform { pos: t.pos.to_array(), rot: t.rot.to_f32(), scale: t.scale }
}

/// a game view through a lens: vertical field of view (radians), width / height, near, far
/// (f64::INFINITY: none)
pub fn camera(v: &View, fov_y: f64, aspect: f64, near: f64, far: f64) -> Camera {
    Camera { eye: v.eye.to_array(), forward: v.forward.to_array(), up: v.up.to_array(), fov_y, aspect, near, far }
}

fn count(m: &mut BTreeMap<u32, u32>, id: u32, up: bool) {
    let n = m.entry(id).or_insert(0);
    if up {
        *n += 1;
    } else {
        *n = n.saturating_sub(1);
        if *n == 0 {
            m.remove(&id);
        }
    }
}

impl Bridge {
    /// no instances yet
    pub fn new() -> Bridge {
        Bridge {
            slots: Vec::new(),
            live: 0,
            mesh_seen: 0,
            mesh_removals: 0,
            pose_seen: 0,
            mesh_use: BTreeMap::new(),
            material_use: BTreeMap::new(),
            retired_meshes: Vec::new(),
            retired_materials: Vec::new(),
            origin_step: ORIGIN_STEP,
        }
    }

    /// instances the bridge holds
    pub fn len(&self) -> u32 {
        self.live
    }

    /// none
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// the instance of entity e
    pub fn instance(&self, e: Entity) -> Option<InstanceId> {
        match self.slots.get(e.index() as usize) {
            Some(Some(s)) if s.entity == e => Some(s.inst),
            _ => None,
        }
    }

    /// instances using mesh m
    pub fn mesh_users(&self, m: MeshId) -> u32 {
        self.mesh_use.get(&m.0).copied().unwrap_or(0)
    }

    /// instances using material m
    pub fn material_users(&self, m: MaterialId) -> u32 {
        self.material_use.get(&m.0).copied().unwrap_or(0)
    }

    fn drop_slot(&mut self, index: usize, scene: &mut Scene) -> Result<()> {
        if let Some(s) = self.slots[index].take() {
            self.live -= 1;
            count(&mut self.mesh_use, s.mesh.0, false);
            count(&mut self.material_use, s.material.0, false);
            scene.remove(s.inst)?;
        }
        Ok(())
    }

    /// the world into the scene for a frame at `alpha` (FixedTick::alpha; Pose::interpolate) -> what it
    /// did.  The scene is the one the bridge has filled so far (another one after reset).  An error of
    /// render3d (a mesh or material it does not have, a removed one) ends the sync there; the next one goes
    /// on from the same place
    pub fn sync(&mut self, world: &World, scene: &mut Scene, alpha: f64) -> Result<SyncStats> {
        let mut st = SyncStats::default();
        let meshes = world.column::<MeshRef>();
        let poses = world.column::<Pose>();
        let at = |e: Entity| poses.and_then(|p| p.get(e)).map_or(game::Transform::IDENTITY, |p| p.interpolate(alpha));

        // gone: entities that left the MeshRef column (despawned, the component removed), only when it
        // counts removals it had not counted at the last sync
        let removals = meshes.map_or(0, |m| m.removals());
        if removals != self.mesh_removals || (meshes.is_none() && self.live > 0) {
            for i in 0..self.slots.len() {
                if let Some(s) = self.slots[i] {
                    if !meshes.is_some_and(|m| m.contains(s.entity)) {
                        self.drop_slot(i, scene)?;
                        st.removed += 1;
                    }
                }
            }
            self.mesh_removals = removals;
        }

        // new and changed MeshRefs (a column whose version did not move has none: no scan)
        if let Some(meshes) = meshes.filter(|m| m.version() > self.mesh_seen) {
            for (e, m) in meshes.changed_since(self.mesh_seen) {
                let i = e.index() as usize;
                if self.slots.len() <= i {
                    self.slots.resize(i + 1, None);
                }
                if let Some(s) = self.slots[i] {
                    if s.entity != e {
                        // (an older generation's instance the sweep did not see: it goes)
                        self.drop_slot(i, scene)?;
                        st.removed += 1;
                    }
                }
                match &mut self.slots[i] {
                    Some(s) => {
                        if s.mesh != m.mesh {
                            scene.set_mesh(s.inst, m.mesh)?;
                            count(&mut self.mesh_use, s.mesh.0, false);
                            count(&mut self.mesh_use, m.mesh.0, true);
                            s.mesh = m.mesh;
                            st.switched += 1;
                        }
                        if s.material != m.material {
                            scene.set_material(s.inst, m.material)?;
                            count(&mut self.material_use, s.material.0, false);
                            count(&mut self.material_use, m.material.0, true);
                            s.material = m.material;
                            st.switched += 1;
                        }
                    }
                    None => {
                        let t = transform(&at(e));
                        let inst = scene.add(Instance { mesh: m.mesh, material: m.material, transform: t })?;
                        self.slots[i] = Some(Slot { entity: e, inst, mesh: m.mesh, material: m.material, t });
                        self.live += 1;
                        count(&mut self.mesh_use, m.mesh.0, true);
                        count(&mut self.material_use, m.material.0, true);
                        st.added += 1;
                    }
                }
            }
            self.mesh_seen = meshes.version();
        }

        // poses: changed since the last sync, or moved in the latest tick (between two states: every frame)
        let since = self.pose_seen.min(world.pose_mark());
        if let Some(poses) = poses.filter(|p| p.version() > since) {
            for (e, p) in poses.changed_since(since) {
                st.visited += 1;
                if let Some(Some(s)) = self.slots.get_mut(e.index() as usize) {
                    if s.entity == e {
                        let t = transform(&p.interpolate(alpha));
                        if t != s.t {
                            scene.set_transform(s.inst, &t)?;
                            s.t = t;
                            st.moved += 1;
                        }
                    }
                }
            }
            self.pose_seen = poses.version();
        }
        Ok(st)
    }

    /// the scene went with its device (GpuContent::detach): forget every instance, the next sync adds
    /// everything to the new scene
    pub fn reset(&mut self) {
        let step = self.origin_step;
        let (rm, rt) = (core::mem::take(&mut self.retired_meshes), core::mem::take(&mut self.retired_materials));
        *self = Bridge::new();
        self.origin_step = step;
        (self.retired_meshes, self.retired_materials) = (rm, rt);
    }

    /// every instance of the bridge goes from the scene (the next sync adds them again)
    pub fn clear(&mut self, scene: &mut Scene) -> Result<()> {
        for i in 0..self.slots.len() {
            self.drop_slot(i, scene)?;
        }
        let step = self.origin_step;
        let (rm, rt) = (core::mem::take(&mut self.retired_meshes), core::mem::take(&mut self.retired_materials));
        *self = Bridge::new();
        self.origin_step = step;
        (self.retired_meshes, self.retired_materials) = (rm, rt);
        Ok(())
    }

    /// the game is done with mesh m: collect removes it from the renderer once no instance of the bridge
    /// uses it (no entity may get a MeshRef with it afterwards)
    pub fn retire_mesh(&mut self, m: MeshId) {
        if !self.retired_meshes.contains(&m) {
            self.retired_meshes.push(m);
        }
    }

    /// the game is done with material m (see retire_mesh)
    pub fn retire_material(&mut self, m: MaterialId) {
        if !self.retired_materials.contains(&m) {
            self.retired_materials.push(m);
        }
    }

    /// meshes and materials retired and not used by any instance of the bridge go from the renderer
    /// (Renderer::remove_mesh, remove_material; after a sync, before the next prepare) -> how many went.
    /// One the renderer refuses (another scene uses it, a mesh takes it as a detail level) stays retired
    /// for the next call; the first refusal is the error when nothing went
    pub fn collect(&mut self, r: &mut Renderer) -> Result<u32> {
        let mut gone = 0;
        let mut refused: Option<GpuError> = None;
        let mut keep = Vec::new();
        for m in core::mem::take(&mut self.retired_meshes) {
            if self.mesh_users(m) > 0 {
                keep.push(m);
                continue;
            }
            match r.remove_mesh(m) {
                Ok(()) => gone += 1,
                Err(e) => {
                    refused.get_or_insert(e);
                    keep.push(m);
                }
            }
        }
        self.retired_meshes = keep;
        let mut keep = Vec::new();
        for m in core::mem::take(&mut self.retired_materials) {
            if self.material_users(m) > 0 {
                keep.push(m);
                continue;
            }
            match r.remove_material(m) {
                Ok(()) => gone += 1,
                Err(e) => {
                    refused.get_or_insert(e);
                    keep.push(m);
                }
            }
        }
        self.retired_materials = keep;
        match refused {
            Some(e) if gone == 0 => Err(e),
            _ => Ok(gone),
        }
    }

    /// meshes and materials retired and still waiting (in use)
    pub fn retired(&self) -> (usize, usize) {
        (self.retired_meshes.len(), self.retired_materials.len())
    }

    /// the step the origin moves in (metres, a power of two keeps positions exact; at least 1)
    pub fn set_origin_step(&mut self, step: f64) {
        if step.is_finite() && step >= 1.0 {
            self.origin_step = step;
        }
    }

    /// the origin's step
    pub fn origin_step(&self) -> f64 {
        self.origin_step
    }

    /// the scene's origin follows the eye: once the eye is more than one step from the origin on an
    /// axis, the origin moves to the eye rounded to the step (Scene::set_origin uploads every instance
    /// once) -> it moved
    pub fn follow(&mut self, scene: &mut Scene, eye: [f64; 3]) -> bool {
        let o = scene.origin();
        let step = self.origin_step;
        if !(eye[0].is_finite() && eye[1].is_finite() && eye[2].is_finite()) {
            return false;
        }
        if (0..3).all(|k| (eye[k] - o[k]).abs() <= step) {
            return false;
        }
        let snap = |v: f64| round(v / step) * step;
        scene.set_origin([snap(eye[0]), snap(eye[1]), snap(eye[2])]);
        true
    }
}

/// x rounded to the nearest whole number, halves away from 0 (core has no f64::round without std)
fn round(x: f64) -> f64 {
    if !(x.abs() < 4.0e15) {
        return x;
    }
    let t = x as i64 as f64;
    let d = x - t;
    if d >= 0.5 {
        t + 1.0
    } else if d <= -0.5 {
        t - 1.0
    } else {
        t
    }
}
