// scene -- a scene's instances, camera and sun (lib.rs has the overview, docs/RENDER3D.md the reasons)
//
// On the CPU: one Slot per instance with its f64 position; free places on a list, used again first.
// On the GPU, per slot: spheres (16 bytes, read by cull), list_of (4 bytes, read by cull only for what
// is visible), instances (64 bytes, the finished matrix, read by the vertex shader).  Per list (one per
// mesh in use): a draw record, a first place in the visible list.  Capacities double, a list keeps room
// for its instances, so frames with instances coming and going rewrite no list layout.
//
// What a frame sends (prepare, then Renderer::record copies it inside the frame's commands, behind the
// barrier against the frame before): always the view and the lights of this frame's slot; the changed
// slots through the scatter shader (a run of (slot, record) in the slot's own Upload buffer); more than
// 4096 changes and an eighth of the slots at once, a new origin, new capacities: everything, from a
// temporary buffer.  Queue::write_buffer would not do for this: its copies run ahead of the frame's
// commands without a barrier against the frames still on the GPU.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use gpu::*;

use crate::math::{self, V3};
use crate::{CHANGE_WORDS, Counts, DRAW_BYTES, INSTANCE_WORDS, LIGHTS_AT, MaterialId, MeshId, Renderer};

/// where an instance is: position in the world (f64), rotation (quaternion x, y, z, w), scale per axis
/// (not negative: a mirrored instance would turn its faces inside out; 0 on some of the axes leaves
/// faces without a normal, they come out black)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub pos: [f64; 3],
    pub rot: [f32; 4],
    pub scale: [f32; 3],
}

impl Transform {
    pub const fn at(pos: [f64; 3]) -> Transform {
        Transform { pos, rot: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }
}

/// what Scene::add takes
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pub mesh: MeshId,
    pub material: MaterialId,
    pub transform: Transform,
}

/// an instance of a Scene; dead after remove, also when its place has gone to another one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceId {
    /// the place in the instance arrays
    pub slot: u32,
    era: u32,
}

/// a right-handed perspective camera; reverse Z
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f64; 3],
    /// unit vectors: where it looks, and up (at right angles)
    pub forward: [f64; 3],
    pub up: [f64; 3],
    /// the vertical field of view in radians, width / height
    pub fov_y: f64,
    pub aspect: f64,
    pub near: f64,
    /// f64::INFINITY: no far plane
    pub far: f64,
}

impl Camera {
    /// at eye looking at target; `up` says which way is up on the screen
    pub fn look_at(eye: [f64; 3], target: [f64; 3], up: [f64; 3], fov_y: f64, aspect: f64, near: f64, far: f64) -> Camera {
        let f = math::norm(math::sub(target, eye));
        let r = math::norm(math::cross(f, up));
        Camera { eye, forward: f, up: math::cross(r, f), fov_y, aspect, near, far }
    }

    /// the 44 floats of the view's uniform block, relative to origin: view_proj, the six planes (inside:
    /// dot(n, p) + d >= 0; left, right, bottom, top, near, far), the eye
    pub(crate) fn view_block(&self, origin: V3) -> [f32; 44] {
        let e = math::sub(self.eye, origin);
        let (f, u) = (self.forward, self.up);
        let r = math::cross(f, u);
        let fy = 1.0 / math::tan(self.fov_y * 0.5);
        let fx = fy / self.aspect;
        // clip.z from the distance along forward (z): near / z at infinity, near (far - z) / (z (far - near))
        let (n, fa) = (self.near, self.far);
        let (zz, zw) = if fa.is_finite() { (-n / (fa - n), n * fa / (fa - n)) } else { (0.0, n) };
        // rows of the view matrix: x along r, y along u, z along forward
        let view = [[r[0], r[1], r[2], -math::dot(r, e)], [u[0], u[1], u[2], -math::dot(u, e)], [f[0], f[1], f[2], -math::dot(f, e)]];
        let mut out = [0.0f32; 44];
        for c in 0..4 {
            let w = if c == 3 { 1.0 } else { 0.0 };
            out[c * 4] = (fx * view[0][c]) as f32;
            out[c * 4 + 1] = (fy * view[1][c]) as f32;
            out[c * 4 + 2] = (zz * view[2][c] + zw * w) as f32;
            out[c * 4 + 3] = view[2][c] as f32;
        }
        let (tx, ty) = (1.0 / fx, 1.0 / fy);
        let plane = |x: f64, y: f64, z: f64, at: f64| -> [f32; 4] {
            let d = math::add(math::add(math::mul(r, x), math::mul(u, y)), math::mul(f, z));
            let m = math::norm(d);
            let (a, b) = (-math::dot(m, e), at / math::len(d));
            // (a little outwards: what rounding to f32 costs here must not cost a visible object)
            [m[0] as f32, m[1] as f32, m[2] as f32, (a + b + (a.abs() + b.abs()) * 5e-7) as f32]
        };
        let planes = [plane(1.0, 0.0, tx, 0.0), plane(-1.0, 0.0, tx, 0.0), plane(0.0, 1.0, ty, 0.0), plane(0.0, -1.0, ty, 0.0), plane(0.0, 0.0, 1.0, -n),
                      if fa.is_finite() { plane(0.0, 0.0, -1.0, fa) } else { [0.0, 0.0, 0.0, 1.0] }];
        for (k, p) in planes.iter().enumerate() {
            out[16 + k * 4..20 + k * 4].copy_from_slice(p);
        }
        out[40] = e[0] as f32;
        out[41] = e[1] as f32;
        out[42] = e[2] as f32;
        out
    }
}

/// one sun and an ambient light (linear colours)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// towards the sun, any length
    pub direction: [f64; 3],
    pub color: [f32; 3],
    pub ambient: [f32; 3],
}

impl Sun {
    pub const DEFAULT: Sun = Sun { direction: [0.3, 0.8, 0.5], color: [0.75; 3], ambient: [0.25; 3] };
}

/// what prepare sent for this frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadInfo {
    /// slots through the scatter shader
    pub changed: u32,
    /// every slot, from a temporary buffer
    pub full: bool,
    /// the lists' layout changed
    pub lists: bool,
    /// everything prepare wrote for the GPU, view and lights included
    pub bytes: u64,
}

/// a list of the scene
#[derive(Clone, Copy, Debug)]
pub struct ListInfo {
    pub mesh: MeshId,
    /// the instances of this mesh, and the room for them in the visible list
    pub count: u32,
    pub capacity: u32,
    /// its first place in the visible list
    pub first: u32,
}

/// what a finished frame measured (Scene::set_stats)
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// counts the scene's frames from 0
    pub frame: u64,
    /// instances the compute shader found visible
    pub visible: u64,
    /// the GPU's clock (ms) at the start, after the uploads, after culling, after the render pass (no
    /// timestamps here: None)
    pub stamps: Option<[f64; 4]>,
}

#[derive(Clone, Copy)]
pub(crate) struct Slot {
    t: Transform,
    mesh: u32,
    material: u32,
    /// the list, how often the place has been left (the ids of those before are dead), alive, among the changes
    list: u32,
    era: u32,
    live: bool,
    dirty: bool,
}

pub(crate) struct List {
    mesh: u32,
    pub count: u32,
    cap: u32,
    pub first: u32,
    /// its mesh's indices have 32 bits
    pub wide: bool,
}

/// one view's data of a frame: its uniform block, its lists.  G5a has the camera's; shadow cascades (G5c)
/// and the second round of Hi-Z (G5d) add more of these (the layout of the lists is the scene's)
pub(crate) struct ViewLists {
    pub ubo: Buffer,
    pub draws: Buffer,
    pub visible: Buffer,
    pub group: BindGroup,
}

/// a frame in flight's own data
pub(crate) struct FrameSlot {
    pub stage: Buffer,
    changes: Option<Buffer>,
    pub scatter_group: Option<BindGroup>,
    pub readback: Option<Buffer>,
    last: Option<(SubmitId, u64)>,
}

/// instances, camera and sun of one scene of a Renderer
pub struct Scene {
    dev: Device,
    pub(crate) counts: Arc<Counts>,
    pub(crate) slots: Vec<Slot>,
    free: Vec<u32>,
    dirty: Vec<u32>,
    full_dirty: bool,
    pub(crate) lists: Vec<List>,
    /// runs of lists with the same width of indices: first, count, wide
    pub(crate) runs: Vec<(u32, u32, bool)>,
    list_of_mesh: Vec<u32>,
    lists_dirty: bool,
    live: u32,
    camera: Camera,
    sun: Sun,
    origin: V3,
    pub(crate) culling: bool,
    pub(crate) stats_on: bool,
    cap: u32,
    list_cap: u32,
    visible_cap: u32,
    pub(crate) spheres: Buffer,
    pub(crate) list_of: Buffer,
    pub(crate) instances: Buffer,
    pub(crate) lights: Buffer,
    pub(crate) template: Buffer,
    pub(crate) list_first: Buffer,
    pub(crate) view: ViewLists,
    pub(crate) render_group: BindGroup,
    pub(crate) frames: Vec<FrameSlot>,
    pub(crate) ts: Option<Timestamps>,
    seen_materials: u32,
    frame: u64,
    pub(crate) slot: usize,
    pub(crate) prepared: bool,
    /// a prepare has run since the last submission (what it sent has not reached the GPU)
    sent: bool,
    /// instances came or changed their mesh since prepare
    pub(crate) stale: bool,
    waited: bool,
    pub(crate) changed: u32,
    pub(crate) full_up: Option<Buffer>,
    pub(crate) lists_up: Option<Buffer>,
    upload: UploadInfo,
    finished: Vec<FrameStats>,
    /// the words of the changes, kept between frames
    scratch: Vec<[u32; CHANGE_WORDS]>,
}

fn storage(dev: &Device, size: u64, more: BufferUsage) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE | more, memory: MemoryKind::Device })
}

fn entry(binding: u32, buffer: &Buffer) -> BindEntry<'_> {
    BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } }
}

#[allow(clippy::too_many_arguments)]
fn cull_group(dev: &Device, r: &Renderer, ubo: &Buffer, spheres: &Buffer, list_of: &Buffer, first: &Buffer, draws: &Buffer, visible: &Buffer) -> Result<BindGroup> {
    dev.create_bind_group(&BindGroupDesc {
        layout: &r.cull_layout,
        entries: &[entry(0, ubo), entry(1, spheres), entry(2, list_of), entry(3, first), entry(4, draws), entry(5, visible)],
    })
}

fn render_group(dev: &Device, r: &Renderer, ubo: &Buffer, instances: &Buffer, lights: &Buffer, visible: &Buffer) -> Result<BindGroup> {
    dev.create_bind_group(&BindGroupDesc {
        layout: &r.draw_layout,
        entries: &[entry(0, ubo), entry(1, instances), entry(2, lights), entry(3, &r.material_buf), entry(4, visible),
                   entry(5, &r.vertices.buf)],
    })
}

/// the record of a slot: its bounding sphere relative to origin, safe against what f32 rounds (nothing
/// visible may go missing), and its instance words
fn records(s: &Slot, r: &Renderer, origin: V3) -> ([f32; 4], [u32; INSTANCE_WORDS]) {
    let m = &r.meshes[s.mesh as usize];
    let t = &s.t;
    let rot = math::rotation(t.rot);
    let (sx, sy, sz) = (t.scale[0] as f64, t.scale[1] as f64, t.scale[2] as f64);
    let p = math::sub(t.pos, origin);
    let c = [rot[0], rot[1], rot[2]];
    // rows of rotation * scale, then the place
    let row = |k: usize| [(c[0][k] * sx) as f32, (c[1][k] * sy) as f32, (c[2][k] * sz) as f32, p[k] as f32];
    let (r0, r1, r2) = (row(0), row(1), row(2));
    let inv = |v: f64| if v > 0.0 { (1.0 / (v * v)) as f32 } else { 0.0 };
    let b = |v: f32| v.to_bits();
    let words = [b(r0[0]), b(r0[1]), b(r0[2]), b(r0[3]), b(r1[0]), b(r1[1]), b(r1[2]), b(r1[3]), b(r2[0]), b(r2[1]), b(r2[2]), b(r2[3]),
                 b(inv(sx)), b(inv(sy)), b(inv(sz)), s.material];
    let mc = m.center;
    let center = math::add(p, [c[0][0] * sx * mc[0] + c[1][0] * sy * mc[1] + c[2][0] * sz * mc[2],
                               c[0][1] * sx * mc[0] + c[1][1] * sy * mc[1] + c[2][1] * sz * mc[2],
                               c[0][2] * sx * mc[0] + c[1][2] * sy * mc[1] + c[2][2] * sz * mc[2]]);
    let big = center[0].abs().max(center[1].abs()).max(center[2].abs());
    let radius = m.radius * sx.max(sy).max(sz) * (1.0 + 2e-6) + big * 5e-7;
    ([center[0] as f32, center[1] as f32, center[2] as f32, radius as f32], words)
}

/// the sphere of a free slot: its first plane test rejects it
const FREE_SPHERE: [f32; 4] = [0.0, 0.0, 0.0, -f32::MAX];

impl Scene {
    pub fn new(r: &Renderer) -> Result<Scene> {
        let dev = r.dev.clone();
        let (cap, list_cap, visible_cap) = (1024u32, 16u32, 1024u32);
        let none = BufferUsage::NONE;
        let spheres = storage(&dev, cap as u64 * 16, none)?;
        let list_of = storage(&dev, cap as u64 * 4, none)?;
        let instances = storage(&dev, cap as u64 * 64, none)?;
        let device_uniform = |size: u64| dev.create_buffer(&BufferDesc { size, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device });
        let (ubo, lights) = (device_uniform(crate::VIEW_BYTES)?, device_uniform(crate::LIGHTS_BYTES)?);
        let template = dev.create_buffer(&BufferDesc { size: list_cap as u64 * DRAW_BYTES, usage: none, memory: MemoryKind::Device })?;
        let list_first = storage(&dev, list_cap as u64 * 4, none)?;
        let draws = storage(&dev, list_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?;
        let visible = storage(&dev, visible_cap as u64 * 4, BufferUsage::NONE)?;
        let group = cull_group(&dev, r, &ubo, &spheres, &list_of, &list_first, &draws, &visible)?;
        let rg = render_group(&dev, r, &ubo, &instances, &lights, &visible)?;
        let mut frames = Vec::new();
        for _ in 0..r.desc.frames_in_flight {
            let stage = dev.create_buffer(&BufferDesc { size: LIGHTS_AT + crate::LIGHTS_BYTES, usage: none, memory: MemoryKind::Upload })?;
            frames.push(FrameSlot { stage, changes: None, scatter_group: None, readback: None, last: None });
        }
        Ok(Scene {
            counts: r.counts.clone(),
            slots: Vec::new(),
            free: Vec::new(),
            dirty: Vec::new(),
            full_dirty: false,
            lists: Vec::new(),
            runs: Vec::new(),
            list_of_mesh: Vec::new(),
            lists_dirty: false,
            live: 0,
            camera: Camera::look_at([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, 1.0, 0.1, f64::INFINITY),
            sun: Sun::DEFAULT,
            origin: [0.0; 3],
            culling: true,
            stats_on: false,
            cap,
            list_cap,
            visible_cap,
            spheres,
            list_of,
            instances,
            lights,
            template,
            list_first,
            view: ViewLists { ubo, draws, visible, group },
            render_group: rg,
            frames,
            ts: None,
            seen_materials: r.bindings_gen,
            frame: 0,
            slot: 0,
            prepared: false,
            sent: false,
            stale: false,
            waited: false,
            changed: 0,
            full_up: None,
            lists_up: None,
            upload: UploadInfo::default(),
            finished: Vec::new(),
            scratch: Vec::new(),
            dev,
        })
    }

    // ------------------------------------------------------------ instances

    fn check_transform(t: &Transform) -> Result<()> {
        let fine = t.pos.iter().all(|v| v.is_finite()) && t.rot.iter().all(|v| v.is_finite()) && t.scale.iter().all(|v| v.is_finite() && *v >= 0.0);
        if !fine {
            return Err(GpuError::Invalid("Transform: finite values and no negative scale (mirrored instances are not built)"));
        }
        if t.rot == [0.0; 4] {
            return Err(GpuError::Invalid("Transform: a quaternion of length 0"));
        }
        Ok(())
    }

    fn check_mesh(&self, m: MeshId) -> Result<()> {
        if m.0 < self.counts.meshes.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(GpuError::Invalid("Scene: a mesh its Renderer does not have"))
        }
    }

    fn check_material(&self, m: MaterialId) -> Result<()> {
        if m.0 < self.counts.materials.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(GpuError::Invalid("Scene: a material its Renderer does not have"))
        }
    }

    fn list_for(&mut self, mesh: u32) -> u32 {
        let m = mesh as usize;
        if self.list_of_mesh.len() <= m {
            self.list_of_mesh.resize(m + 1, u32::MAX);
        }
        if self.list_of_mesh[m] == u32::MAX {
            self.list_of_mesh[m] = self.lists.len() as u32;
            self.lists.push(List { mesh, count: 0, cap: 0, first: 0, wide: false });
            self.lists_dirty = true;
        }
        self.list_of_mesh[m]
    }

    fn enter_list(&mut self, mesh: u32) -> u32 {
        let l = self.list_for(mesh);
        let list = &mut self.lists[l as usize];
        list.count += 1;
        if list.count > list.cap {
            list.cap = (list.cap * 2).max(16);
            self.lists_dirty = true;
        }
        l
    }

    fn mark(&mut self, slot: u32) {
        let s = &mut self.slots[slot as usize];
        if !s.dirty {
            s.dirty = true;
            self.dirty.push(slot);
        }
    }

    /// a new instance, on a place that came free if there is one
    pub fn add(&mut self, i: Instance) -> Result<InstanceId> {
        Scene::check_transform(&i.transform)?;
        self.check_mesh(i.mesh)?;
        self.check_material(i.material)?;
        self.stale = true;
        let list = self.enter_list(i.mesh.0);
        let slot = match self.free.pop() {
            Some(f) => f,
            None => {
                let f = self.slots.len() as u32;
                self.slots.push(Slot { t: i.transform, mesh: 0, material: 0, list: 0, era: 0, live: false, dirty: false });
                f
            }
        };
        let s = &mut self.slots[slot as usize];
        s.t = i.transform;
        s.mesh = i.mesh.0;
        s.material = i.material.0;
        s.list = list;
        s.live = true;
        let era = s.era;
        self.live += 1;
        self.mark(slot);
        Ok(InstanceId { slot, era })
    }

    fn get(&self, id: InstanceId) -> Result<usize> {
        match self.slots.get(id.slot as usize) {
            Some(s) if s.live && s.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this instance was removed")),
        }
    }

    /// the instance goes; its place comes free
    pub fn remove(&mut self, id: InstanceId) -> Result<()> {
        let i = self.get(id)?;
        let s = &mut self.slots[i];
        s.live = false;
        s.era = s.era.wrapping_add(1);
        let l = s.list as usize;
        self.lists[l].count -= 1;
        self.free.push(id.slot);
        self.live -= 1;
        self.mark(id.slot);
        Ok(())
    }

    pub fn set_transform(&mut self, id: InstanceId, t: &Transform) -> Result<()> {
        Scene::check_transform(t)?;
        let i = self.get(id)?;
        self.slots[i].t = *t;
        self.mark(id.slot);
        Ok(())
    }

    pub fn transform(&self, id: InstanceId) -> Result<Transform> {
        Ok(self.slots[self.get(id)?].t)
    }

    pub fn set_material(&mut self, id: InstanceId, m: MaterialId) -> Result<()> {
        self.check_material(m)?;
        let i = self.get(id)?;
        self.slots[i].material = m.0;
        self.mark(id.slot);
        Ok(())
    }

    pub fn set_mesh(&mut self, id: InstanceId, m: MeshId) -> Result<()> {
        self.check_mesh(m)?;
        let i = self.get(id)?;
        self.stale = true;
        let old = self.slots[i].list as usize;
        self.lists[old].count -= 1;
        let list = self.enter_list(m.0);
        let s = &mut self.slots[i];
        s.mesh = m.0;
        s.list = list;
        self.mark(id.slot);
        Ok(())
    }

    /// instances alive
    pub fn len(&self) -> u32 {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    pub fn set_camera(&mut self, c: &Camera) {
        self.camera = *c;
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn set_sun(&mut self, s: &Sun) {
        self.sun = *s;
    }

    /// the point of the world that is (0, 0, 0) on the GPU.  Keep it near the camera: f32 has seven
    /// digits.  Moving it uploads every instance once
    pub fn set_origin(&mut self, origin: [f64; 3]) {
        if origin != self.origin {
            self.origin = origin;
            self.full_dirty = true;
        }
    }

    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    /// false: every live instance is drawn (a test's comparison, a debugging aid)
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// FrameStats of each frame: the instances found visible (a small copy per frame), the GPU's times
    pub fn set_stats(&mut self, on: bool) {
        self.stats_on = on;
    }

    /// what the last prepare sent
    pub fn upload_info(&self) -> UploadInfo {
        self.upload
    }

    /// the lists in order (list i = the i-th draw record of draws_buffer)
    pub fn lists(&self) -> Vec<ListInfo> {
        self.lists.iter().map(|l| ListInfo { mesh: MeshId(l.mesh), count: l.count, capacity: l.cap, first: l.first }).collect()
    }

    /// (tests, tools) the camera's draw records after a frame's culling: 5 u32 per list, instance_count second
    pub fn draws_buffer(&self) -> &Buffer {
        &self.view.draws
    }

    /// (tests, tools) the camera's visible list after a frame's culling: slots, a list's at ListInfo::first
    pub fn visible_buffer(&self) -> &Buffer {
        &self.view.visible
    }

    // ------------------------------------------------------------ a frame

    /// wait for the frame that last used this frame's slot, so its data can be written (the window's
    /// acquire has usually done so).  prepare does it too; call it first to tell the wait from the work.
    /// The browser cannot wait, and needs not: its writes go in queue order
    pub fn wait_slot(&mut self) -> Result<()> {
        if self.waited {
            return Ok(());
        }
        self.waited = true;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        if let Some((id, f)) = self.frames[slot].last {
            let done = self.dev.wait(id, 10_000_000_000)?;
            if !done && !cfg!(target_arch = "wasm32") {
                return Err(GpuError::Timeout);
            }
            if done {
                self.collect(slot, f);
            }
            self.frames[slot].last = None;
        }
        Ok(())
    }

    fn collect(&mut self, slot: usize, frame: u64) {
        if !self.stats_on {
            return;
        }
        let Some(rb) = &self.frames[slot].readback else { return };
        let mut draws = vec![0u32; rb.size() as usize / 4];
        if rb.read(0, &mut draws).is_err() {
            return;
        }
        let n = self.lists.len().min(draws.len() / 5);
        let visible = (0..n).map(|i| draws[i * 5 + 1] as u64).sum();
        let stamps = self.ts.as_ref().and_then(|ts| ts.read_ms(slot as u32 * 4, 4).ok()).map(|v| [v[0], v[1], v[2], v[3]]);
        self.finished.push(FrameStats { frame, visible, stamps });
    }

    /// the finished frames' stats since the last call
    pub fn take_stats(&mut self) -> Vec<FrameStats> {
        core::mem::take(&mut self.finished)
    }

    /// after Device::wait_idle: the stats of what was still in flight
    pub fn collect_stats(&mut self) {
        for slot in 0..self.frames.len() {
            if let Some((_, f)) = self.frames[slot].last.take() {
                self.collect(slot, f);
            }
        }
        self.finished.sort_by_key(|s| s.frame);
    }

    /// new capacities: new buffers, then every slot again
    fn grow(&mut self, r: &Renderer) -> Result<()> {
        let dev = self.dev.clone();
        let none = BufferUsage::NONE;
        let mut groups = false;
        let need = self.slots.len() as u32;
        if need > r.max_instances {
            return Err(GpuError::Unsupported("Scene: more instances than this device's bindings and one dispatch reach"));
        }
        if need > self.cap {
            self.cap = need.max(self.cap.saturating_mul(2)).min(r.max_instances);
            self.spheres = storage(&dev, self.cap as u64 * 16, none)?;
            self.list_of = storage(&dev, self.cap as u64 * 4, none)?;
            self.instances = storage(&dev, self.cap as u64 * 64, none)?;
            self.full_dirty = true;
            groups = true;
        }
        let lists = self.lists.len() as u32;
        if lists > self.list_cap {
            self.list_cap = lists.max(self.list_cap * 2);
            self.template = dev.create_buffer(&BufferDesc { size: self.list_cap as u64 * DRAW_BYTES, usage: none, memory: MemoryKind::Device })?;
            self.list_first = storage(&dev, self.list_cap as u64 * 4, none)?;
            self.view.draws = storage(&dev, self.list_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?;
            self.lists_dirty = true;
            groups = true;
        }
        let room: u64 = self.lists.iter().map(|l| l.cap as u64).sum();
        if room > u32::MAX as u64 {
            return Err(GpuError::Unsupported("Scene: the visible lists have grown too large"));
        }
        if room as u32 > self.visible_cap {
            self.visible_cap = (room as u32).max(self.visible_cap.saturating_mul(2));
            self.view.visible = storage(&dev, self.visible_cap as u64 * 4, BufferUsage::NONE)?;
            groups = true;
        }
        if groups {
            let v = &mut self.view;
            v.group = cull_group(&dev, r, &v.ubo, &self.spheres, &self.list_of, &self.list_first, &v.draws, &v.visible)?;
            for f in &mut self.frames {
                f.scatter_group = None;
            }
        }
        if groups || self.seen_materials != r.bindings_gen {
            self.render_group = render_group(&dev, r, &self.view.ubo, &self.instances, &self.lights, &self.view.visible)?;
            self.seen_materials = r.bindings_gen;
        }
        Ok(())
    }

    /// the lists' places and draw records, when their layout changed
    fn upload_lists(&mut self, r: &Renderer) -> Result<()> {
        self.lists_up = None;
        if !self.lists_dirty || self.lists.is_empty() {
            return Ok(());
        }
        self.lists_dirty = false;
        let mut first = 0u32;
        let mut places = Vec::with_capacity(self.lists.len());
        let mut draws = Vec::with_capacity(self.lists.len());
        for l in &mut self.lists {
            let m = &r.meshes[l.mesh as usize];
            l.first = first;
            l.wide = m.wide;
            places.push(first);
            // (Basis: no first instance is sure to work: the list's start is an offset of the vertex buffer)
            draws.push([m.indices, 0, m.first_index, m.base_vertex, if r.multi { first } else { 0 }]);
            first += l.cap;
        }
        self.runs.clear();
        for (i, l) in self.lists.iter().enumerate() {
            match self.runs.last_mut() {
                Some(r) if r.2 == l.wide => r.1 += 1,
                _ => self.runs.push((i as u32, 1, l.wide)),
            }
        }
        let n = self.lists.len() as u64;
        let size = n * (4 + DRAW_BYTES);
        let up = self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        up.write(0, &places)?;
        up.write(n * 4, &draws)?;
        self.upload.lists = true;
        self.upload.bytes += size;
        self.lists_up = Some(up);
        Ok(())
    }

    /// every slot, from a temporary buffer: spheres, list_of, instances
    fn upload_all(&mut self, r: &Renderer) -> Result<()> {
        let n = self.slots.len();
        if n == 0 {
            self.full_dirty = false;
            return Ok(());
        }
        const CHUNK: usize = 4096;
        let size = n as u64 * 84;
        let up = self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        let (mut spheres, mut lists, mut words) = (Vec::with_capacity(CHUNK), Vec::with_capacity(CHUNK), Vec::with_capacity(CHUNK));
        let origin = self.origin;
        for (k, chunk) in self.slots.chunks_mut(CHUNK).enumerate() {
            spheres.clear();
            lists.clear();
            words.clear();
            for s in chunk.iter_mut() {
                s.dirty = false;
                if s.live {
                    let (sphere, w) = records(s, r, origin);
                    spheres.push(sphere);
                    lists.push(s.list);
                    words.push(w);
                } else {
                    spheres.push(FREE_SPHERE);
                    lists.push(0);
                    words.push([0; INSTANCE_WORDS]);
                }
            }
            let at = (k * CHUNK) as u64;
            up.write(at * 16, &spheres)?;
            up.write(n as u64 * 16 + at * 4, &lists)?;
            up.write(n as u64 * 20 + at * 64, &words)?;
        }
        self.dirty.clear();
        self.full_dirty = false;
        self.upload.full = true;
        self.upload.bytes += size;
        self.full_up = Some(up);
        Ok(())
    }

    /// the changed slots, for the scatter shader
    fn upload_changes(&mut self, r: &Renderer, slot: usize) -> Result<()> {
        let k = self.dirty.len();
        self.changed = k as u32;
        if k == 0 {
            return Ok(());
        }
        let mut rows = core::mem::take(&mut self.scratch);
        rows.clear();
        let origin = self.origin;
        for &i in &self.dirty {
            let s = &mut self.slots[i as usize];
            s.dirty = false;
            let mut row = [0u32; CHANGE_WORDS];
            row[0] = i;
            if s.live {
                let (sphere, w) = records(s, r, origin);
                row[1] = s.list;
                for j in 0..4 {
                    row[4 + j] = sphere[j].to_bits();
                }
                row[8..8 + INSTANCE_WORDS].copy_from_slice(&w);
            } else {
                for j in 0..4 {
                    row[4 + j] = FREE_SPHERE[j].to_bits();
                }
            }
            rows.push(row);
        }
        self.dirty.clear();
        let bytes = k as u64 * CHANGE_WORDS as u64 * 4;
        let f = &mut self.frames[slot];
        if f.changes.as_ref().is_none_or(|b| b.size() < bytes) {
            let size = bytes.next_power_of_two().max(1 << 14);
            f.changes = Some(self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE, memory: MemoryKind::Upload })?);
            f.scatter_group = None;
        }
        let Some(buf) = &f.changes else { return Ok(()) };
        buf.write(0, &rows)?;
        if f.scatter_group.is_none() {
            f.scatter_group = Some(self.dev.create_bind_group(&BindGroupDesc {
                layout: &r.scatter_layout,
                entries: &[entry(0, buf), entry(1, &self.spheres), entry(2, &self.list_of), entry(3, &self.instances)],
            })?);
        }
        self.scratch = rows;
        self.upload.changed = k as u32;
        self.upload.bytes += bytes;
        Ok(())
    }

    /// the frame's data for Renderer::record: waits for this slot's last frame, sends what changed
    pub fn prepare(&mut self, r: &mut Renderer) -> Result<()> {
        if !Arc::ptr_eq(&self.counts, &r.counts) {
            return Err(GpuError::Invalid("Scene::prepare: another Renderer than its own"));
        }
        self.wait_slot()?;
        // the last prepare never reached a submission, or failed half way: what it sent is lost
        if self.sent {
            self.full_dirty = true;
            self.lists_dirty = true;
            r.resend_materials();
        }
        self.sent = true;
        self.prepared = false;
        self.stale = false;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        self.slot = slot;
        self.upload = UploadInfo::default();
        self.full_up = None;
        self.changed = 0;
        self.upload.bytes += r.prepare_materials()?;
        self.grow(r)?;
        self.upload_lists(r)?;
        // this device's stats need a read-back per frame and timestamps
        if self.stats_on {
            if self.ts.is_none() && self.dev.features().timestamps {
                self.ts = Some(self.dev.create_timestamps(4 * self.frames.len() as u32)?);
            }
            let size = self.list_cap as u64 * DRAW_BYTES;
            let f = &mut self.frames[slot];
            if f.readback.as_ref().is_none_or(|b| b.size() < size) {
                f.readback = Some(self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?);
            }
        }
        let k = self.dirty.len();
        if self.full_dirty || (k > 4096 && k * 8 > self.slots.len()) {
            self.upload_all(r)?;
        } else {
            self.upload_changes(r, slot)?;
        }
        let s = &self.sun;
        let d = math::norm(s.direction);
        let lights = [d[0] as f32, d[1] as f32, d[2] as f32, 0.0, s.color[0], s.color[1], s.color[2], 0.0, s.ambient[0], s.ambient[1], s.ambient[2], 0.0];
        let f = &self.frames[slot];
        f.stage.write(0, &self.camera.view_block(self.origin))?;
        f.stage.write(LIGHTS_AT, &lights)?;
        self.upload.bytes += crate::VIEW_BYTES + crate::LIGHTS_BYTES;
        self.prepared = true;
        Ok(())
    }

    /// the frame went to the GPU: Queue::submit's or Swapchain::present's answer
    pub fn submitted(&mut self, id: SubmitId) {
        let slot = self.slot;
        self.frames[slot].last = Some((id, self.frame));
        self.frame += 1;
        self.prepared = false;
        self.sent = false;
        self.waited = false;
    }
}
