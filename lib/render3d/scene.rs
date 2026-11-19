// scene -- a scene's instances, camera and sun (lib.rs has the overview, docs/RENDER3D.md the reasons)
//
// On the CPU: one Slot per instance with its f64 position; free places on a list, used again first.
// On the GPU, per slot: spheres (16 bytes, read by cull), list_of (4 bytes, read by cull only for what
// is visible), instances (64 bytes, the finished matrix, read by the vertex shader).  Per list (one per
// mesh, side and alpha class in use: a mirrored instance, its scales' product negative, turns its faces round and
// is drawn by the pipelines whose front faces are clockwise; an instance of a MASK material is drawn by the pipelines
// with a discard): a draw record, a first place in the visible list.  Which list an instance is in is settled by
// prepare (the class is its material's, which the renderer holds).  The draw records lie by width of indices, class
// and side, so each combination is one multi-draw; a list's record is the fourth word of its entry in list_first.
// Capacities double, a list keeps room for its instances, so frames with instances coming and going rewrite no list
// layout.
//
// What a frame sends (prepare, then Renderer::record copies it inside the frame's commands, behind the
// barrier against the frame before): always the view and the lights of this frame's slot; the changed
// slots through the scatter shader (a run of (slot, record) in the slot's own Upload buffer); more than
// 4096 changes and an eighth of the slots at once, a new origin, new capacities: everything, from a
// temporary buffer.  Queue::write_buffer would not do for this: its copies run ahead of the frame's
// commands without a barrier against the frames still on the GPU.
//
// (G7f) Removal (store.rs): a scene counts its instances per mesh and material and tells the renderer's shared table
// when it has the first or no longer any of one (and when it goes); lists belong to a mesh's slot, so a removed mesh's
// lists serve the next mesh in that slot.  Lists per side, class and faces: double-sided materials have lists of their
// own (drawn without culling).  Blended instances (class Blend) have lists for their mesh like the others, without room in
// the visible lists: the camera's culling puts them into one list for all of them (`blend`), sort.wgsl sorts it, one draw
// draws it (lib.rs draw_blend).

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;


use gpu::*;

use crate::math::{self, V3};
use crate::particles::{self, Emitter, EmitterId, PARTICLE_BYTES};
use crate::post::{Antialias, OUTLINE_BYTES, Outline};
use crate::shadow::{self, CASCADES_MAX, Cascade, Shadows};
use crate::store::{Use, give, give_all, take};
use crate::{ALPHA_CLASSES, AlphaClass, BLEND_AT, BLEND_HEAD, BLEND_ITEM, CASCADE_AT, CASCADE_STRIDE, CHANGE_WORDS, Counts, DRAW_BYTES, INSTANCE_WORDS, Kind, MaterialId,
            LIGHTS_AT, MeshId, PFRAME_AT, PFRAME_BYTES, PHEAD_AT, POST_AT, Renderer, SORT_TILE, STAGE_BYTES, STAMPS, VM_AT, slot};

/// where an instance is: position in the world (f64), rotation (quaternion x, y, z, w), scale per axis.
/// Negative scales mirror: an odd number of them turns the winding round, the instance goes to its mesh's
/// lists of mirrored instances and keeps its outside in front.  0 on some of the axes leaves faces without
/// a normal, they come out black
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

    /// the determinant is negative: the faces turn round
    pub fn mirrored(&self) -> bool {
        (self.scale[0] as f64) * (self.scale[1] as f64) * (self.scale[2] as f64) < 0.0
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

/// a right-handed camera, perspective or (G7f) orthographic; reverse Z
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f64; 3],
    /// unit vectors: where it looks, and up (at right angles)
    pub forward: [f64; 3],
    pub up: [f64; 3],
    /// the vertical field of view in radians (perspective), width / height
    pub fov_y: f64,
    pub aspect: f64,
    /// distances along forward; (G7f) an orthographic camera's near may be 0 or below
    pub near: f64,
    /// f64::INFINITY: no far plane (perspective only: an orthographic camera needs a finite one)
    pub far: f64,
    /// (G7f) Some(h): orthographic, the view h world units high and h * aspect wide, fov_y unused (top, front and side
    /// views, editors); depth falls linearly from 1 at near to 0 at far.  Culling, the occlusion test, detail levels (the
    /// size of an instance is its sphere's diameter over h, at any distance) and the sun's shadows (the cascades split the
    /// view's box uniformly) work as for a perspective camera.  None: perspective
    pub ortho: Option<f64>,
}

impl Camera {
    /// at eye looking at target; `up` says which way is up on the screen
    pub fn look_at(eye: [f64; 3], target: [f64; 3], up: [f64; 3], fov_y: f64, aspect: f64, near: f64, far: f64) -> Camera {
        let f = math::norm(math::sub(target, eye));
        let r = math::norm(math::cross(f, up));
        Camera { eye, forward: f, up: math::cross(r, f), fov_y, aspect, near, far, ortho: None }
    }

    /// (G7f) an orthographic camera at eye looking at target, the view `height` world units high; near and far along
    /// the view (far finite)
    pub fn ortho_at(eye: [f64; 3], target: [f64; 3], up: [f64; 3], height: f64, aspect: f64, near: f64, far: f64) -> Camera {
        Camera { ortho: Some(height), ..Camera::look_at(eye, target, up, 1.0, aspect, near, far) }
    }

    /// (G7f) Scene::prepare: what an orthographic camera needs (a height above 0, a finite far plane beyond near)
    pub(crate) fn check(&self) -> Result<()> {
        match self.ortho {
            Some(h) if !(h.is_finite() && h > 0.0 && self.near.is_finite() && self.far.is_finite() && self.far > self.near && self.aspect > 0.0) => {
                Err(GpuError::Invalid("Camera: an orthographic camera needs a height above 0, finite near and far, far beyond near"))
            }
            _ => Ok(()),
        }
    }

    /// the 60 floats of the view's uniform block, relative to origin: view_proj, the six planes (inside:
    /// dot(n, p) + d >= 0; left, right, bottom, top, near, far), the eye, (G5d) the projection's scales for the
    /// occlusion test (clip x = fx x, clip y = fy y, depth = zz + zw / z in the camera's space; G7f orthographic: depth =
    /// zz + zw z), the eye and fy for the detail levels, (G7f) forward and whether the view is orthographic, whether the
    /// detail levels' camera is.  depth: (a, b), the depth becomes a depth + b (G7f: (0.5, 0) for the world with a viewmodel,
    /// (0.5, 0.5) for the viewmodel; 0.5 is an exact power of two, the order of depths stays; (1, 0): as it is)
    pub(crate) fn view_block(&self, origin: V3, depth: (f64, f64)) -> [f32; 60] {
        if let Some(h) = self.ortho {
            return self.ortho_block(origin, h, depth);
        }
        let e = math::sub(self.eye, origin);
        let (f, u) = (self.forward, self.up);
        let r = math::cross(f, u);
        let fy = 1.0 / math::tan(self.fov_y * 0.5);
        let fx = fy / self.aspect;
        // clip.z from the distance along forward (z): near / z at infinity, near (far - z) / (z (far - near))
        let (n, fa) = (self.near, self.far);
        let (zz, zw) = if fa.is_finite() { (-n / (fa - n), n * fa / (fa - n)) } else { (0.0, n) };
        let (zz, zw) = (zz * depth.0 + depth.1, zw * depth.0);
        // rows of the view matrix: x along r, y along u, z along forward
        let view = [[r[0], r[1], r[2], -math::dot(r, e)], [u[0], u[1], u[2], -math::dot(u, e)], [f[0], f[1], f[2], -math::dot(f, e)]];
        let mut out = [0.0f32; 60];
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
        out[44] = fx as f32;
        out[45] = fy as f32;
        out[46] = zz as f32;
        out[47] = zw as f32;
        out[48..52].copy_from_slice(&[e[0] as f32, e[1] as f32, e[2] as f32, fy as f32]);
        out[52..55].copy_from_slice(&[f[0] as f32, f[1] as f32, f[2] as f32]);
        out
    }

    /// (G7f) view_block of an orthographic camera h high: clip x = sx x, clip y = sy y, depth = zz + zw z (1 at near, 0 at
    /// far), w = 1; the planes are the box's six sides
    fn ortho_block(&self, origin: V3, h: f64, depth: (f64, f64)) -> [f32; 60] {
        let e = math::sub(self.eye, origin);
        let (f, u) = (self.forward, self.up);
        let r = math::cross(f, u);
        let sy = 2.0 / h;
        let sx = sy / self.aspect;
        let (n, fa) = (self.near, self.far);
        let (zz, zw) = (fa / (fa - n) * depth.0 + depth.1, -1.0 / (fa - n) * depth.0);
        let view = [[r[0], r[1], r[2], -math::dot(r, e)], [u[0], u[1], u[2], -math::dot(u, e)], [f[0], f[1], f[2], -math::dot(f, e)]];
        let mut out = [0.0f32; 60];
        for c in 0..4 {
            let w = if c == 3 { 1.0 } else { 0.0 };
            out[c * 4] = (sx * view[0][c]) as f32;
            out[c * 4 + 1] = (sy * view[1][c]) as f32;
            out[c * 4 + 2] = (zw * view[2][c] + zz * w) as f32;
            out[c * 4 + 3] = w as f32;
        }
        // inside: dot(m, p) + d >= 0 with m along an axis and d = at - dot(m, e) (a little outwards for what f32 rounds)
        let plane = |m: V3, at: f64| -> [f32; 4] {
            let a = -math::dot(m, e);
            [m[0] as f32, m[1] as f32, m[2] as f32, (a + at + (a.abs() + at.abs()) * 5e-7) as f32]
        };
        let (hw, hh) = (1.0 / sx, 1.0 / sy);
        let planes = [plane(r, hw), plane(math::mul(r, -1.0), hw), plane(u, hh), plane(math::mul(u, -1.0), hh), plane(f, -n), plane(math::mul(f, -1.0), fa)];
        for (k, p) in planes.iter().enumerate() {
            out[16 + k * 4..20 + k * 4].copy_from_slice(p);
        }
        out[40..43].copy_from_slice(&[e[0] as f32, e[1] as f32, e[2] as f32]);
        out[44..48].copy_from_slice(&[sx as f32, sy as f32, zz as f32, zw as f32]);
        // (the detail levels: an instance's size is r sy at any distance)
        out[48..52].copy_from_slice(&[e[0] as f32, e[1] as f32, e[2] as f32, sy as f32]);
        out[52..56].copy_from_slice(&[f[0] as f32, f[1] as f32, f[2] as f32, 1.0]);
        out[56] = 1.0;
        out
    }
}

/// (G7f) the viewmodel's lens (Scene::set_viewmodel): the instances of Scene::add_viewmodel (a weapon before the camera) are
/// drawn with this field of view and depth range after the world, in front of all of it (the world's depth takes 0 ..= 0.5,
/// the viewmodel's 0.5 ..= 1), lit by the scene's sun, shadows, lights and environment.  Made with `new` and changed field
/// by field (it may grow)
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Viewmodel {
    /// the vertical field of view in radians
    pub fov_y: f64,
    /// its own near and far planes (finite, 0 < near < far), distances along the camera's forward
    pub near: f64,
    pub far: f64,
}

impl Viewmodel {
    pub const fn new(fov_y: f64, near: f64, far: f64) -> Viewmodel {
        Viewmodel { fov_y, near, far }
    }

    fn check(&self) -> Result<()> {
        let fine = self.fov_y.is_finite() && self.fov_y > 0.0 && self.fov_y < 3.1 && self.near.is_finite() && self.near > 0.0 && self.far.is_finite() && self.far > self.near;
        if fine { Ok(()) } else { Err(GpuError::Invalid("Viewmodel: 0 < fov_y < pi, 0 < near < far, finite")) }
    }

    /// the camera of the viewmodel's view: the scene's eye and axes, its own lens (always perspective)
    fn camera(&self, c: &Camera) -> Camera {
        Camera { fov_y: self.fov_y, near: self.near, far: self.far, ortho: None, ..*c }
    }
}

/// one sun and an ambient light, linear, in the units of KHR_lights_punctual: a white matte surface facing
/// the sun shows color / pi, one under the ambient light alone shows ambient (before the exposure)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// towards the sun, any length
    pub direction: [f64; 3],
    /// illuminance per channel (lux)
    pub color: [f32; 3],
    /// the radiance of a uniform environment around the scene
    pub ambient: [f32; 3],
}

impl Sun {
    /// a white matte face towards the sun shows 0.75 + 0.25 (0.75 pi lux, ambient 0.25)
    pub const DEFAULT: Sun = Sun { direction: [0.3, 0.8, 0.5], color: [0.75 * core::f32::consts::PI; 3], ambient: [0.25; 3] };
}

/// (G5c) a point or spot light (KHR_lights_punctual): no shadows
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Point,
    /// cone angles from the axis in radians, 0 <= inner < outer <= pi / 2: full inside inner, nothing beyond outer
    Spot { inner: f32, outer: f32 },
}

/// (G5c) a point or spot light of a Scene
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub pos: [f64; 3],
    /// a spot light's axis, the way it shines (any length but 0)
    pub direction: [f64; 3],
    /// linear colour, and the intensity in candela: color * intensity / d^2 reaches a surface d away
    pub color: [f32; 3],
    pub intensity: f32,
    /// where it ends, KHR_lights_punctual's window clamp(1 - (d / range)^4, 0, 1); None: never
    pub range: Option<f32>,
}

impl Light {
    /// a point light of colour times intensity at pos
    pub const fn point(pos: [f64; 3], color: [f32; 3], intensity: f32, range: Option<f32>) -> Light {
        Light { kind: LightKind::Point, pos, direction: [0.0, 0.0, -1.0], color, intensity, range }
    }

    fn check(&self) -> Result<()> {
        let finite = self.pos.iter().chain(self.direction.iter()).all(|v| v.is_finite()) && self.color.iter().all(|v| v.is_finite() && *v >= 0.0)
            && self.intensity.is_finite() && self.intensity >= 0.0 && self.range.is_none_or(|r| r.is_finite() && r > 0.0);
        let cone = match self.kind {
            LightKind::Point => true,
            LightKind::Spot { inner, outer } => inner >= 0.0 && inner < outer && outer <= core::f32::consts::FRAC_PI_2 && math::len(self.direction) > 0.0,
        };
        if finite && cone { Ok(()) } else { Err(GpuError::Invalid("Light: finite values, colour and intensity not negative, range above 0, 0 <= inner < outer <= pi / 2")) }
    }

    /// draw.wgsl's Punctual relative to origin: position and range (0: none), colour times intensity and the
    /// kind (1: spot), the axis, the cone as scale and offset of cos (KHR_lights_punctual)
    fn words(&self, origin: V3) -> [f32; 16] {
        let p = math::sub(self.pos, origin);
        let c = self.color;
        let k = self.intensity;
        let (spot, d, scale, offset) = match self.kind {
            LightKind::Point => (0.0, [0.0; 3], 0.0, 0.0),
            LightKind::Spot { inner, outer } => {
                let (ci, co) = (math::sincos(inner as f64).1, math::sincos(outer as f64).1);
                let scale = 1.0 / (ci - co).max(0.001);
                (1.0, math::norm(self.direction), scale, -co * scale)
            }
        };
        [p[0] as f32, p[1] as f32, p[2] as f32, self.range.unwrap_or(0.0), c[0] * k, c[1] * k, c[2] * k, spot, d[0] as f32, d[1] as f32, d[2] as f32, 0.0,
         scale as f32, offset as f32, 0.0, 0.0]
    }
}

/// bytes of a light on the GPU
pub(crate) const PUNCTUAL_BYTES: u64 = 64;

/// what the fragment shader does with the light it computed (times the exposure)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tonemap {
    /// Khronos PBR Neutral: base colours stay up to 0.76, highlights are compressed towards white
    Neutral,
    /// none: linear values, clamped by the target's format (HDR targets such as Rgba16Float keep them)
    None,
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

/// (G7f) what ListInfo cannot say (it is not #[non_exhaustive]): more about a list, in the order of Scene::lists
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListKind {
    /// its instances' materials are double-sided: drawn without culling, back faces lit with their normal turned round
    pub double_sided: bool,
}

/// a list of the scene
#[derive(Clone, Copy, Debug)]
pub struct ListInfo {
    pub mesh: MeshId,
    /// its instances are mirrored (drawn with clockwise front faces)
    pub mirrored: bool,
    /// the alpha class of its instances' materials (drawn with or without a discard)
    pub class: AlphaClass,
    /// the instances of this mesh, side and class, and the room for them in the visible list (G7f: none for blended
    /// instances, they go into the blended instances' list)
    pub count: u32,
    pub capacity: u32,
    /// (G5d) the instances that may land here: its own and those of meshes that take it as a detail level
    pub room: u32,
    /// its first place in the visible list
    pub first: u32,
    /// its draw record (draws_buffer, late_buffers, cascade_buffers: 5 u32 each); the records lie by width of indices,
    /// class and side
    pub record: u32,
}

/// what a finished frame measured (Scene::set_stats)
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// counts the scene's frames from 0
    pub frame: u64,
    /// instances the compute shader found visible (with the occlusion test: drawn in phase 1 or 2)
    pub visible: u64,
    /// (G5d) of them, those phase 2 of the occlusion test drew (0 without it)
    pub late: u64,
    /// the GPU's clock (ms) at the start, after the uploads, after the camera's culling (phase 1), after the frame
    /// (no timestamps here: None)
    pub stamps: Option<[f64; 4]>,
    /// (G5c) per cascade of the sun, ms: its culling, its depth pass (0 for cascades not drawn)
    pub cascades: [[f64; 2]; CASCADES_MAX],
    /// (G5c) the main render pass, ms (G5d: both phases' passes)
    pub draw: f64,
    /// (G5d) the occlusion test, ms: building the pyramid, phase 2's culling, phase 2's pass (0 without it)
    pub occlusion: [f64; 3],
}

#[derive(Clone, Copy)]
pub(crate) struct Slot {
    t: Transform,
    mesh: u32,
    material: u32,
    /// the list prepare settled (NO_LIST: none yet), how often the place has been left (the ids of those before are
    /// dead), alive, among the changes
    list: u32,
    era: u32,
    live: bool,
    dirty: bool,
    mirrored: bool,
    /// (G7f) the viewmodel's (its transform in the camera's space)
    vm: bool,
}

pub(crate) struct List {
    /// (G7f) its mesh's slot, and the mesh's id (the last one seen there)
    mesh: u32,
    id: u32,
    pub mirrored: bool,
    pub class: AlphaClass,
    /// (G7f) for double-sided materials
    pub double: bool,
    /// (G7f) of the viewmodel's instances (drawn by its view only)
    pub vm: bool,
    pub count: u32,
    /// (G5d) its own instances and those that may pick its mesh as a detail level
    pub room: u32,
    cap: u32,
    pub first: u32,
    /// its mesh's indices have 32 bits
    pub wide: bool,
    /// its draw record
    pub record: u32,
}

/// a slot in no list (none settled yet)
const NO_LIST: u32 = u32::MAX;

/// a run of draw records side by side with the same width of indices, side and alpha class: one multi-draw on Voll.
/// room: one of its lists has room (instances may land there)
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    pub first: u32,
    pub count: u32,
    pub wide: bool,
    pub mirrored: bool,
    pub class: AlphaClass,
    pub double: bool,
    /// (G7f) the viewmodel's
    pub vm: bool,
    pub room: bool,
}

/// (G7f) the blended instances' list: draw record and entries (BLEND_HEAD + cap * BLEND_ITEM bytes, cap a power of two
/// and a multiple of SORT_TILE), its group for the draw and for the sort
pub(crate) struct BlendList {
    pub buf: Buffer,
    pub cap: u32,
    pub render: Option<BindGroup>,
    pub sort: Option<BindGroup>,
}

/// one view's data of a frame: its uniform block, its lists, its bind groups for culling and drawing.  The
/// camera's, one per shadow cascade (G5c), phase 2 of the occlusion test (G5d; the camera's uniform block); the
/// layout of the lists is the scene's
pub(crate) struct ViewLists {
    pub ubo: Buffer,
    pub draws: Buffer,
    pub visible: Buffer,
    pub group: BindGroup,
    pub render: BindGroup,
}

/// a frame in flight's own data
pub(crate) struct FrameSlot {
    pub stage: Buffer,
    /// (G5c) the point and spot lights of a frame that sends them
    pub punctual: Option<Buffer>,
    changes: Option<Buffer>,
    pub scatter_group: Option<BindGroup>,
    pub readback: Option<Buffer>,
    last: Option<(SubmitId, u64)>,
    /// (G7f) the particles born for this frame
    pub spawn: Option<Buffer>,
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
    /// the lists in the order of their draw records, and the runs of those records (Run)
    pub(crate) order: Vec<u32>,
    pub(crate) runs: Vec<Run>,
    /// the list of each mesh's slot per side (as it is, mirrored), alpha class, (G7f) faces (single, double-sided) and
    /// layer (the world, the viewmodel)
    list_of_mesh: Vec<[[[[u32; 2]; 2]; ALPHA_CLASSES]; 2]>,
    /// (G7f) its instances per slot of a mesh and of a material (store.rs)
    mesh_use: Vec<Use>,
    material_use: Vec<Use>,
    /// (G7f) the renderer's removals and new meshes in old slots, as the last prepare saw them
    pub(crate) seen_removals: u32,
    seen_mesh_gen: u32,
    /// (G7f) the blended instances: their list, it is drawn this frame (a list of them has room), vertices per instance
    pub(crate) blend: BlendList,
    pub(crate) blend_on: bool,
    blend_k: u32,
    blend_need: u32,
    lists_dirty: bool,
    /// the renderer's count of materials that changed their class, as the last prepare saw it
    seen_classes: u32,
    live: u32,
    camera: Camera,
    sun: Sun,
    shadows: Option<Shadows>,
    cascade_info: Vec<Cascade>,
    /// (G5c) the renderer's environment times this instead of Sun::ambient
    environment: Option<f32>,
    /// (G5c) the point and spot lights; their buffer, its room, whether they go up with the next frame
    punctual: Vec<Light>,
    pub(crate) punctual_buf: Buffer,
    punctual_cap: u32,
    punctual_dirty: bool,
    /// lights this frame copies from its slot's buffer
    pub(crate) punctual_up: u32,
    exposure: f32,
    tonemap: Tonemap,
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
    /// per list (first place in the visible list, its first detail level, their number), then (G5d) the detail levels
    /// (threshold, list): 16 bytes each, info_len of them in use, room for info_cap
    pub(crate) list_first: Buffer,
    pub(crate) info_len: u32,
    info_cap: u32,
    seen_lods: u32,
    pub(crate) view: ViewLists,
    /// (G5d) phase 2 of the camera's occlusion test: its lists (the camera's uniform block)
    pub(crate) late: ViewLists,
    /// (G5d) a bit per slot, phase 2 found it hidden; whether this frame clears them all (a new buffer, every slot
    /// sent anew), whether the buffer is new; the occlusion test on
    pub(crate) hidden: Buffer,
    pub(crate) reset_hidden: bool,
    hidden_fresh: bool,
    pub(crate) occlusion: bool,
    /// (G5c) one per cascade of the sun, made when shadows want them
    pub(crate) cascades: Vec<ViewLists>,
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
    /// (G7f) the viewmodel's lens, its view (made with the first frame that has one), its slots, the camera their records
    /// were computed with
    viewmodel: Option<Viewmodel>,
    pub(crate) vm: Option<ViewLists>,
    vm_slots: Vec<u32>,
    vm_camera: Option<Camera>,
    /// (G7f) outlines and anti-aliasing
    pub(crate) outline: Option<Outline>,
    pub(crate) antialias: Antialias,
    /// (G7f) the particles: their emitters, the clock, the ring and its list
    emitters: Vec<EmitterSlot>,
    time: f64,
    pub(crate) particles: Option<Particles>,
}

/// (G7f) an emitter of a scene: its description, its particles' next serial number, how often its place was left, alive
#[derive(Clone, Copy)]
struct EmitterSlot {
    e: Emitter,
    serial: u32,
    era: u32,
    live: bool,
}

/// (G7f) a scene's particles: the ring of records (cap of them) and the frame's list (room for cap, a power of two of the
/// sort's tiles), their groups; the next place of the ring, the records born since the last prepare (from place `first`),
/// those the last prepare sent (sent again when its frame never went out), the clock's epoch and the last death
pub(crate) struct Particles {
    pub cap: u32,
    pub buf: Buffer,
    pub list: BlendList,
    pub cs_group: Option<BindGroup>,
    pub render: Option<BindGroup>,
    /// the ring is new: cleared by the next frame
    pub fresh: bool,
    head: u32,
    pending: Vec<[u32; 16]>,
    first: u32,
    last: Vec<[u32; 16]>,
    last_first: u32,
    /// this frame's copies into the ring: (first place, records, where in the frame's upload buffer)
    pub copies: [(u32, u32, u64); 2],
    epoch: f64,
    alive_until: f64,
    pub frame_ubo: Buffer,
}

fn storage(dev: &Device, size: u64, more: BufferUsage) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE | more, memory: MemoryKind::Device })
}

fn entry(binding: u32, buffer: &Buffer) -> BindEntry<'_> {
    BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } }
}

#[allow(clippy::too_many_arguments)]
fn cull_group(dev: &Device, r: &Renderer, ubo: &Buffer, spheres: &Buffer, list_of: &Buffer, first: &Buffer, draws: &Buffer, visible: &Buffer, hidden: &Buffer,
              blend: &Buffer) -> Result<BindGroup> {
    dev.create_bind_group(&BindGroupDesc {
        layout: &r.cull_layout,
        entries: &[entry(0, ubo), entry(1, spheres), entry(2, list_of), entry(3, first), entry(4, draws), entry(5, visible), entry(6, hidden), entry(7, blend)],
    })
}

/// (G7f) a blended instances' list of room for cap of them
fn blend_buf(dev: &Device, cap: u32) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size: BLEND_HEAD + cap as u64 * BLEND_ITEM, usage: BufferUsage::STORAGE | BufferUsage::INDIRECT, memory: MemoryKind::Device })
}

#[allow(clippy::too_many_arguments)]
fn render_group(dev: &Device, r: &Renderer, ubo: &Buffer, instances: &Buffer, lights: &Buffer, visible: &Buffer, punctual: &Buffer) -> Result<BindGroup> {
    dev.create_bind_group(&BindGroupDesc {
        layout: &r.draw_layout,
        entries: &[entry(0, ubo), entry(1, instances), entry(2, lights), entry(3, &r.material_buf), entry(4, visible),
                   entry(5, &r.vertices.buf), entry(6, punctual)],
    })
}

/// the buffers of a view of the scene's sizes, and its groups (ubo: another view's uniform block, else its own)
#[allow(clippy::too_many_arguments)]
fn view_lists(dev: &Device, r: &Renderer, list_cap: u32, visible_cap: u32, spheres: &Buffer, list_of: &Buffer, list_first: &Buffer, instances: &Buffer,
              lights: &Buffer, punctual: &Buffer, hidden: &Buffer, blend: &Buffer, ubo: Option<&Buffer>) -> Result<ViewLists> {
    let ubo = match ubo {
        Some(u) => u.clone(),
        None => dev.create_buffer(&BufferDesc { size: crate::VIEW_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?,
    };
    let draws = storage(dev, list_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?;
    let visible = storage(dev, visible_cap as u64 * 4, BufferUsage::NONE)?;
    let group = cull_group(dev, r, &ubo, spheres, list_of, list_first, &draws, &visible, hidden, blend)?;
    let render = render_group(dev, r, &ubo, instances, lights, &visible, punctual)?;
    Ok(ViewLists { ubo, draws, visible, group, render })
}

/// the record of a slot: its bounding sphere relative to origin, safe against what f32 rounds (nothing
/// visible may go missing), and its instance words.  (G7f) A viewmodel's slot: its transform in the camera's space (x
/// right, y up, -z forward) taken to the world
fn records(s: &Slot, r: &Renderer, origin: V3, cam: &Camera) -> ([f32; 4], [u32; INSTANCE_WORDS]) {
    // (G5d) the sphere grown to hold the mesh's detail levels
    let (mc, mr) = r.bounds[slot(s.mesh)];
    let t = &s.t;
    let rot = math::rotation(t.rot);
    let (sx, sy, sz) = (t.scale[0] as f64, t.scale[1] as f64, t.scale[2] as f64);
    let mut p = math::sub(t.pos, origin);
    let mut c = [rot[0], rot[1], rot[2]];
    if s.vm {
        let (f, u) = (cam.forward, cam.up);
        let rr = math::cross(f, u);
        let to_world = |v: V3| math::add(math::add(math::mul(rr, v[0]), math::mul(u, v[1])), math::mul(f, -v[2]));
        p = math::add(math::sub(cam.eye, origin), to_world(t.pos));
        c = [to_world(rot[0]), to_world(rot[1]), to_world(rot[2])];
    }
    // rows of rotation * scale, then the place
    let row = |k: usize| [(c[0][k] * sx) as f32, (c[1][k] * sy) as f32, (c[2][k] * sz) as f32, p[k] as f32];
    let (r0, r1, r2) = (row(0), row(1), row(2));
    // (a mirror's sign stays in the matrix: m * (n / s^2) = r * (n / s), the inverse transpose)
    let inv = |v: f64| if v != 0.0 { (1.0 / (v * v)) as f32 } else { 0.0 };
    let b = |v: f32| v.to_bits();
    let words = [b(r0[0]), b(r0[1]), b(r0[2]), b(r0[3]), b(r1[0]), b(r1[1]), b(r1[2]), b(r1[3]), b(r2[0]), b(r2[1]), b(r2[2]), b(r2[3]),
                 b(inv(sx)), b(inv(sy)), b(inv(sz)), slot(s.material) as u32];
    let center = math::add(p, [c[0][0] * sx * mc[0] + c[1][0] * sy * mc[1] + c[2][0] * sz * mc[2],
                               c[0][1] * sx * mc[0] + c[1][1] * sy * mc[1] + c[2][1] * sz * mc[2],
                               c[0][2] * sx * mc[0] + c[1][2] * sy * mc[1] + c[2][2] * sz * mc[2]]);
    let big = center[0].abs().max(center[1].abs()).max(center[2].abs());
    let radius = mr * sx.abs().max(sy.abs()).max(sz.abs()) * (1.0 + 2e-6) + big * 5e-7;
    ([center[0] as f32, center[1] as f32, center[2] as f32, radius as f32], words)
}

/// the sphere of a free slot: its first plane test rejects it
const FREE_SPHERE: [f32; 4] = [0.0, 0.0, 0.0, -f32::MAX];

/// (G5d) the bits of hidden slots: a word per 32
fn hidden_bytes(cap: u32) -> u64 {
    cap.div_ceil(32) as u64 * 4
}

impl Scene {
    pub fn new(r: &Renderer) -> Result<Scene> {
        let dev = r.dev.clone();
        let (cap, list_cap, visible_cap) = (1024u32, 16u32, 1024u32);
        let none = BufferUsage::NONE;
        let spheres = storage(&dev, cap as u64 * 16, none)?;
        let list_of = storage(&dev, cap as u64 * 4, none)?;
        let instances = storage(&dev, cap as u64 * 64, none)?;
        let lights = dev.create_buffer(&BufferDesc { size: crate::LIGHTS_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?;
        let template = dev.create_buffer(&BufferDesc { size: list_cap as u64 * DRAW_BYTES, usage: none, memory: MemoryKind::Device })?;
        let list_first = storage(&dev, list_cap as u64 * 16, none)?;
        let punctual_cap = 16u32;
        let punctual_buf = storage(&dev, punctual_cap as u64 * PUNCTUAL_BYTES, none)?;
        let hidden = storage(&dev, hidden_bytes(cap), none)?;
        let blend = BlendList { buf: blend_buf(&dev, SORT_TILE)?, cap: SORT_TILE, render: None, sort: None };
        let view = view_lists(&dev, r, list_cap, visible_cap, &spheres, &list_of, &list_first, &instances, &lights, &punctual_buf, &hidden, &blend.buf, None)?;
        let late = view_lists(&dev, r, list_cap, visible_cap, &spheres, &list_of, &list_first, &instances, &lights, &punctual_buf, &hidden, &blend.buf, Some(&view.ubo))?;
        let mut frames = Vec::new();
        for _ in 0..r.desc.frames_in_flight {
            let stage = dev.create_buffer(&BufferDesc { size: STAGE_BYTES, usage: none, memory: MemoryKind::Upload })?;
            frames.push(FrameSlot { stage, punctual: None, changes: None, scatter_group: None, readback: None, last: None, spawn: None });
        }
        Ok(Scene {
            counts: r.counts.clone(),
            slots: Vec::new(),
            free: Vec::new(),
            dirty: Vec::new(),
            full_dirty: false,
            lists: Vec::new(),
            order: Vec::new(),
            runs: Vec::new(),
            list_of_mesh: Vec::new(),
            mesh_use: Vec::new(),
            material_use: Vec::new(),
            seen_removals: r.removals,
            seen_mesh_gen: r.mesh_gen,
            blend,
            blend_on: false,
            blend_k: 0,
            blend_need: 0,
            lists_dirty: false,
            seen_classes: r.class_gen,
            live: 0,
            camera: Camera::look_at([0.0, 0.0, 5.0], [0.0; 3], [0.0, 1.0, 0.0], 1.0, 1.0, 0.1, f64::INFINITY),
            sun: Sun::DEFAULT,
            shadows: None,
            cascade_info: Vec::new(),
            environment: None,
            punctual: Vec::new(),
            punctual_buf,
            punctual_cap,
            punctual_dirty: false,
            punctual_up: 0,
            exposure: 1.0,
            tonemap: Tonemap::Neutral,
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
            info_len: 0,
            info_cap: list_cap,
            seen_lods: r.lod_gen,
            view,
            late,
            hidden,
            reset_hidden: false,
            hidden_fresh: true,
            occlusion: true,
            cascades: Vec::new(),
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
            viewmodel: None,
            vm: None,
            vm_slots: Vec::new(),
            vm_camera: None,
            outline: None,
            antialias: Antialias::None,
            emitters: Vec::new(),
            time: 0.0,
            particles: None,
            dev,
        })
    }

    // ------------------------------------------------------------ instances

    fn check_transform(t: &Transform) -> Result<()> {
        let fine = t.pos.iter().all(|v| v.is_finite()) && t.rot.iter().all(|v| v.is_finite()) && t.scale.iter().all(|v| v.is_finite());
        if !fine {
            return Err(GpuError::Invalid("Transform: finite values"));
        }
        if t.rot == [0.0; 4] {
            return Err(GpuError::Invalid("Transform: a quaternion of length 0"));
        }
        Ok(())
    }

    /// (G7f) the list of a mesh (its id), side, alpha class, faces and layer (the world's, the viewmodel's)
    fn list_for(&mut self, mesh: u32, mirrored: bool, class: AlphaClass, double: bool, vm: bool) -> u32 {
        let m = slot(mesh);
        if self.list_of_mesh.len() <= m {
            self.list_of_mesh.resize(m + 1, [[[[NO_LIST; 2]; 2]; ALPHA_CLASSES]; 2]);
        }
        let (side, c, d, v) = (mirrored as usize, class.index(), double as usize, vm as usize);
        if self.list_of_mesh[m][side][c][d][v] == NO_LIST {
            self.list_of_mesh[m][side][c][d][v] = self.lists.len() as u32;
            self.lists.push(List { mesh: m as u32, id: mesh, mirrored, class, double, vm, count: 0, room: 0, cap: 0, first: 0, wide: false, record: 0 });
            self.lists_dirty = true;
        }
        let l = self.list_of_mesh[m][side][c][d][v];
        self.lists[l as usize].id = mesh;
        l
    }

    /// every changed slot into the list of its mesh, side and its material's alpha class (out of the one it was in);
    /// when a material changed its class, every slot is looked at, and those that move go up again.  (The room of the
    /// lists follows: fit_lists)
    fn settle_lists(&mut self, r: &Renderer) {
        let all = self.seen_classes != r.class_gen;
        self.seen_classes = r.class_gen;
        let n = if all { self.slots.len() } else { self.dirty.len() };
        for k in 0..n {
            let i = if all { k } else { self.dirty[k] as usize };
            let s = self.slots[i];
            if !s.live {
                if s.list != NO_LIST {
                    self.lists[s.list as usize].count -= 1;
                    self.slots[i].list = NO_LIST;
                }
                continue;
            }
            let k = r.kind_of(s.material);
            if s.list != NO_LIST {
                let l = &self.lists[s.list as usize];
                if l.mesh as usize == slot(s.mesh) && l.mirrored == s.mirrored && l.class == k.class && l.double == k.double && l.vm == s.vm {
                    continue;
                }
                self.lists[s.list as usize].count -= 1;
            }
            let to = self.list_for(s.mesh, s.mirrored, k.class, k.double, s.vm);
            self.lists[to as usize].count += 1;
            self.slots[i].list = to;
            // (list_of changes: up again)
            if all {
                self.mark(i as u32);
            }
        }
    }

    /// (G5d) every list's room: its own instances and those of meshes that may pick it as a detail level (lists of
    /// detail levels are made here, of the same side and alpha class); capacities double when the room does not fit
    fn fit_lists(&mut self, r: &Renderer) {
        for l in &mut self.lists {
            l.room = l.count;
        }
        for i in 0..self.lists.len() {
            let (mesh, mirrored, class, double, count) = (self.lists[i].mesh, self.lists[i].mirrored, self.lists[i].class, self.lists[i].double, self.lists[i].count);
            let vm = self.lists[i].vm;
            if count == 0 {
                continue;
            }
            for k in 0..r.lods[mesh as usize].len() {
                if let Some(t) = r.lods[mesh as usize][k].mesh {
                    let to = self.list_for(t.0, mirrored, class, double, vm) as usize;
                    self.lists[to].room += count;
                }
            }
        }
        // (G7f) blended instances have no room in the visible lists: they are counted for their own list
        let mut blended = 0u64;
        for l in &mut self.lists {
            if l.class == AlphaClass::Blend {
                blended += l.count as u64;
                continue;
            }
            if l.room > l.cap {
                l.cap = l.room.max(l.cap * 2).max(16);
                self.lists_dirty = true;
            }
        }
        // (G7f) the vertices of the largest blended mesh that may be drawn (the draw takes that many per instance)
        self.blend_k = self.lists.iter().filter(|l| l.class == AlphaClass::Blend && l.room > 0).map(|l| r.meshes[l.mesh as usize].indices).max().unwrap_or(0);
        self.blend_on = self.blend_k > 0;
        self.blend_need = blended.min(u32::MAX as u64) as u32;
    }

    fn mark(&mut self, slot: u32) {
        let s = &mut self.slots[slot as usize];
        if !s.dirty {
            s.dirty = true;
            self.dirty.push(slot);
        }
    }

    /// a new instance, on a place that came free if there is one (its list: with the next prepare)
    pub fn add(&mut self, i: Instance) -> Result<InstanceId> {
        self.add_in(i, false)
    }

    /// (G7f) an instance of the viewmodel (Scene::set_viewmodel; without one it is not drawn): its transform is in the camera's
    /// space (x right, y up, -z forward, as glTF's cameras), it follows the camera, is drawn with the viewmodel's lens in
    /// front of the world, lit by the scene's sun, shadows and lights (it casts no shadows and hides nothing from the
    /// occlusion test).  set_transform, set_material, set_mesh and remove work as for the others.  Its material may not be
    /// blended (prepare: Invalid)
    pub fn add_viewmodel(&mut self, i: Instance) -> Result<InstanceId> {
        self.add_in(i, true)
    }

    fn add_in(&mut self, i: Instance, vm: bool) -> Result<InstanceId> {
        Scene::check_transform(&i.transform)?;
        // (G7f) the ids alive, and counted: a mesh or material a scene uses is not removed
        take(&self.counts, &mut self.mesh_use, Kind::Mesh, i.mesh.0)?;
        if let Err(e) = take(&self.counts, &mut self.material_use, Kind::Material, i.material.0) {
            give(&self.counts, &mut self.mesh_use, Kind::Mesh, i.mesh.0);
            return Err(e);
        }
        self.stale = true;
        let mirrored = i.transform.mirrored();
        let slot = match self.free.pop() {
            Some(f) => f,
            None => {
                let f = self.slots.len() as u32;
                self.slots.push(Slot { t: i.transform, mesh: 0, material: 0, list: NO_LIST, era: 0, live: false, dirty: false, mirrored: false, vm: false });
                f
            }
        };
        // (a place that came free keeps its old list until prepare settles it)
        let s = &mut self.slots[slot as usize];
        s.t = i.transform;
        s.mesh = i.mesh.0;
        s.material = i.material.0;
        s.live = true;
        s.mirrored = mirrored;
        s.vm = vm;
        let era = s.era;
        if vm {
            self.vm_slots.push(slot);
        }
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

    /// the instance goes; its place comes free (and its list loses it with the next prepare)
    pub fn remove(&mut self, id: InstanceId) -> Result<()> {
        let i = self.get(id)?;
        let (mesh, material) = (self.slots[i].mesh, self.slots[i].material);
        give(&self.counts, &mut self.mesh_use, Kind::Mesh, mesh);
        give(&self.counts, &mut self.material_use, Kind::Material, material);
        let s = &mut self.slots[i];
        s.live = false;
        s.era = s.era.wrapping_add(1);
        if s.vm {
            self.vm_slots.retain(|&x| x != id.slot);
        }
        self.free.push(id.slot);
        self.live -= 1;
        self.mark(id.slot);
        Ok(())
    }

    /// a new transform; one that mirrors where the old did not (or the other way round) moves the instance
    /// to its mesh's list of the other side, as set_mesh does (with the next prepare)
    pub fn set_transform(&mut self, id: InstanceId, t: &Transform) -> Result<()> {
        Scene::check_transform(t)?;
        let i = self.get(id)?;
        let mirrored = t.mirrored();
        if mirrored != self.slots[i].mirrored {
            self.stale = true;
            self.slots[i].mirrored = mirrored;
        }
        self.slots[i].t = *t;
        self.mark(id.slot);
        Ok(())
    }

    pub fn transform(&self, id: InstanceId) -> Result<Transform> {
        Ok(self.slots[self.get(id)?].t)
    }

    /// a material of another alpha class moves the instance to the list of that class with the next prepare (until then
    /// a frame shows the old material: it goes up with prepare too)
    pub fn set_material(&mut self, id: InstanceId, m: MaterialId) -> Result<()> {
        let i = self.get(id)?;
        take(&self.counts, &mut self.material_use, Kind::Material, m.0)?;
        give(&self.counts, &mut self.material_use, Kind::Material, self.slots[i].material);
        self.slots[i].material = m.0;
        self.mark(id.slot);
        Ok(())
    }

    /// the instance's list of the new mesh, with the next prepare
    pub fn set_mesh(&mut self, id: InstanceId, m: MeshId) -> Result<()> {
        let i = self.get(id)?;
        take(&self.counts, &mut self.mesh_use, Kind::Mesh, m.0)?;
        give(&self.counts, &mut self.mesh_use, Kind::Mesh, self.slots[i].mesh);
        self.stale = true;
        self.slots[i].mesh = m.0;
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

    /// what the light is multiplied by before the tone mapper (1 at first; not finite: 1)
    pub fn set_exposure(&mut self, e: f32) {
        self.exposure = if e.is_finite() { e } else { 1.0 };
    }

    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    /// Tonemap::Neutral at first
    pub fn set_tonemap(&mut self, t: Tonemap) {
        self.tonemap = t;
    }

    /// (G5c) the sun's shadows, None (at first) for none; Invalid for values out of range
    pub fn set_shadows(&mut self, s: Option<&Shadows>) -> Result<()> {
        if let Some(s) = s {
            s.check()?;
        }
        self.shadows = s.copied();
        Ok(())
    }

    pub fn shadows(&self) -> Option<Shadows> {
        self.shadows
    }

    /// (G5c) Some(k): the renderer's environment (Renderer::set_environment) times k lights the scene in place of
    /// Sun::ambient (until it has one, Sun::ambient does); None (at first): Sun::ambient
    pub fn set_environment(&mut self, k: Option<f32>) {
        self.environment = k.filter(|v| v.is_finite() && *v >= 0.0);
    }

    /// (G5c) the point and spot lights (all of them: they go up with the next frame); Invalid for values out of
    /// range, and nothing changes then
    pub fn set_lights(&mut self, lights: &[Light]) -> Result<()> {
        for l in lights {
            l.check()?;
        }
        self.punctual.clear();
        self.punctual.extend_from_slice(lights);
        self.punctual_dirty = true;
        Ok(())
    }

    pub fn lights(&self) -> &[Light] {
        &self.punctual
    }

    /// (G5c; tests, tools) the cascades the last prepare computed
    pub fn cascades(&self) -> &[Cascade] {
        &self.cascade_info
    }

    /// (G5c; tests, tools) a cascade's draw records and visible list after a frame's culling
    pub fn cascade_buffers(&self, k: usize) -> Option<(&Buffer, &Buffer)> {
        self.cascades.get(k).map(|v| (&v.draws, &v.visible))
    }

    /// the point of the world that is (0, 0, 0) on the GPU.  Keep it near the camera: f32 has seven
    /// digits.  Moving it uploads every instance once
    pub fn set_origin(&mut self, origin: [f64; 3]) {
        if origin != self.origin {
            self.origin = origin;
            self.full_dirty = true;
            self.punctual_dirty = true;
            // (G7f) the particles die: their places are relative to it
            self.clear_particles();
        }
    }

    pub fn origin(&self) -> [f64; 3] {
        self.origin
    }

    /// false: every live instance is drawn (a test's comparison, a debugging aid)
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// (G5d) the camera's occlusion test against a depth pyramid in two phases (docs/RENDER3D.md "Verdeckung"): true
    /// at first.  It runs when the depth attachment can be sampled (the renderer's own can) and culling is on
    pub fn set_occlusion(&mut self, on: bool) {
        self.occlusion = on;
    }

    pub fn occlusion(&self) -> bool {
        self.occlusion
    }

    /// (G5d; tests, tools) phase 2's draw records and visible list after a frame (laid out as draws_buffer and
    /// visible_buffer, which hold phase 1's with the occlusion test)
    pub fn late_buffers(&self) -> (&Buffer, &Buffer) {
        (&self.late.draws, &self.late.visible)
    }

    /// FrameStats of each frame: the instances found visible (a small copy per frame), the GPU's times
    pub fn set_stats(&mut self, on: bool) {
        self.stats_on = on;
    }

    /// what the last prepare sent
    pub fn upload_info(&self) -> UploadInfo {
        self.upload
    }

    /// the lists as the last prepare settled them, in order (list i is what list_of holds for its instances; its draw
    /// record: ListInfo::record)
    pub fn lists(&self) -> Vec<ListInfo> {
        self.lists.iter().map(|l| ListInfo { mesh: MeshId(l.id), mirrored: l.mirrored, class: l.class, count: l.count, capacity: l.cap, room: l.room, first: l.first,
                                             record: l.record }).collect()
    }

    /// (G7f) more about the lists, in the order of lists(): double-sided or not
    pub fn list_kinds(&self) -> Vec<ListKind> {
        self.lists.iter().map(|l| ListKind { double_sided: l.double }).collect()
    }

    /// (G7f; tests, tools) the blended instances' list after a frame: a VkDrawIndirectCommand (vertices per instance, the
    /// instances), then per instance 4 u32: the bits of !(distance squared from the eye; f32), its slot, the draw record of
    /// its list (lists().record), the list's flags (1: 32-bit indices, 2: double-sided, 4: mirrored); sorted, farthest
    /// first.  Its draw: blend_drawn()
    pub fn blend_buffer(&self) -> &Buffer {
        &self.blend.buf
    }

    /// (G7f) the last prepare found blended instances that may be drawn (a list of them has room)
    pub fn blend_drawn(&self) -> bool {
        self.blend_on
    }

    /// (tests, tools) the camera's draw records after a frame's culling: 5 u32 per list at ListInfo::record,
    /// instance_count second
    pub fn draws_buffer(&self) -> &Buffer {
        &self.view.draws
    }

    /// (tests, tools) the camera's visible list after a frame's culling: slots, a list's at ListInfo::first
    pub fn visible_buffer(&self) -> &Buffer {
        &self.view.visible
    }

    // ------------------------------------------------------------ (G7f) viewmodel, outlines, anti-aliasing

    /// (G7f) the viewmodel's lens (None at first: its instances are not drawn, the world's depth is as it was); Invalid for
    /// values out of range.  With one the world's depth takes 0 ..= 0.5 (halved: the image stays the same but where two
    /// depths tie, as GPUs interpolate depth in steps of their own), the
    /// viewmodel's 0.5 ..= 1
    pub fn set_viewmodel(&mut self, v: Option<&Viewmodel>) -> Result<()> {
        if let Some(v) = v {
            v.check()?;
        }
        self.viewmodel = v.copied();
        Ok(())
    }

    pub fn viewmodel(&self) -> Option<Viewmodel> {
        self.viewmodel
    }

    /// (G7f; tests, tools) the viewmodel's draw records and visible list after a frame (as draws_buffer, visible_buffer)
    pub fn viewmodel_buffers(&self) -> Option<(&Buffer, &Buffer)> {
        self.vm.as_ref().map(|v| (&v.draws, &v.visible))
    }

    /// (G7f) lines on the edges of the depth buffer, over the opaque image (None at first); Invalid for values out of
    /// range.  They need a depth attachment with TextureUsage::SAMPLED (the renderer's own has it): record says Invalid
    /// otherwise
    pub fn set_outline(&mut self, o: Option<&Outline>) -> Result<()> {
        if let Some(o) = o {
            o.check()?;
        }
        self.outline = o.copied();
        Ok(())
    }

    pub fn outline(&self) -> Option<Outline> {
        self.outline
    }

    /// (G7f) anti-aliasing over the finished image (Antialias::None at first)
    pub fn set_antialias(&mut self, a: Antialias) {
        self.antialias = a;
    }

    pub fn antialias(&self) -> Antialias {
        self.antialias
    }

    // ------------------------------------------------------------ (G7f) particles

    /// (G7f) room for this many particles (0, at first: none); a new capacity starts with none alive.  Unsupported beyond what
    /// a binding reaches
    pub fn set_particles(&mut self, capacity: u32) -> Result<()> {
        if self.particles.as_ref().map_or(0, |p| p.cap) == capacity {
            return Ok(());
        }
        if capacity == 0 {
            self.particles = None;
            return Ok(());
        }
        if capacity as u64 * PARTICLE_BYTES > self.dev.limits().max_storage_binding as u64 || capacity > 1 << 22 {
            return Err(GpuError::Unsupported("Scene::set_particles: more than a binding reaches or one dispatch (at most 2^22)"));
        }
        let none = BufferUsage::NONE;
        let room = capacity.checked_next_power_of_two().unwrap_or(1 << 24).max(SORT_TILE);
        let buf = storage(&self.dev, capacity as u64 * PARTICLE_BYTES, none)?;
        let list = BlendList { buf: blend_buf(&self.dev, room)?, cap: room, render: None, sort: None };
        let frame_ubo = self.dev.create_buffer(&BufferDesc { size: PFRAME_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?;
        self.particles = Some(Particles { cap: capacity, buf, list, cs_group: None, render: None, fresh: true, head: 0, pending: Vec::new(), first: 0, last: Vec::new(),
                                          last_first: 0, copies: [(0, 0, 0); 2], epoch: self.time, alive_until: self.time, frame_ubo });
        Ok(())
    }

    /// (G7f) the particles' room (0: none)
    pub fn particle_capacity(&self) -> u32 {
        self.particles.as_ref().map_or(0, |p| p.cap)
    }

    /// (G7f) the particles' clock in seconds (the game's time; 0 at first): births and ages are read from it, so the same
    /// times give the same frames.  It should not go back
    pub fn set_time(&mut self, t: f64) {
        if !t.is_finite() {
            return;
        }
        self.time = t;
        // (the clock counts microseconds in 32 bits on the GPU: a new epoch when nothing lives)
        if let Some(p) = &mut self.particles {
            if t >= p.alive_until && t - p.epoch > 1800.0 {
                p.epoch = t;
                p.fresh = true;
                p.pending.clear();
            }
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// (G7f) an emitter: what its particles look like and how they move; Invalid for values out of range
    pub fn add_emitter(&mut self, e: &Emitter) -> Result<EmitterId> {
        e.check()?;
        if let Some(k) = self.emitters.iter().position(|x| !x.live) {
            let x = &mut self.emitters[k];
            *x = EmitterSlot { e: *e, serial: 0, era: x.era, live: true };
            return Ok(EmitterId::new(k as u32, x.era));
        }
        self.emitters.push(EmitterSlot { e: *e, serial: 0, era: 0, live: true });
        Ok(EmitterId::new(self.emitters.len() as u32 - 1, 0))
    }

    fn emitter(&self, id: EmitterId) -> Result<usize> {
        match self.emitters.get(id.slot as usize) {
            Some(x) if x.live && x.era == id.era() => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this emitter was removed")),
        }
    }

    /// (G7f) a new description (its next particles take it; the living keep theirs); its serial numbers go on
    pub fn set_emitter(&mut self, id: EmitterId, e: &Emitter) -> Result<()> {
        e.check()?;
        let k = self.emitter(id)?;
        self.emitters[k].e = *e;
        Ok(())
    }

    /// (G7f) the emitter goes (its living particles stay until they die)
    pub fn remove_emitter(&mut self, id: EmitterId) -> Result<()> {
        let k = self.emitter(id)?;
        let x = &mut self.emitters[k];
        x.live = false;
        x.era = x.era.wrapping_add(1);
        Ok(())
    }

    /// (G7f) `count` particles of an emitter, born now (Scene::set_time) at a point in a cone around a direction (the world's,
    /// or for a viewmodel emitter the camera's space).  They take the next places of the ring, the oldest are overwritten.
    /// Invalid without room for particles (set_particles) or for values not finite
    pub fn emit(&mut self, id: EmitterId, at: [f64; 3], dir: [f64; 3], count: u32) -> Result<()> {
        let k = self.emitter(id)?;
        if !(at.iter().chain(dir.iter()).all(|v| v.is_finite())) {
            return Err(GpuError::Invalid("Scene::emit: finite values"));
        }
        let origin = self.origin;
        let time = self.time;
        let Some(p) = &mut self.particles else { return Err(GpuError::Invalid("Scene::emit: no room for particles (Scene::set_particles)")) };
        let x = &mut self.emitters[k];
        let e = x.e;
        let aim = particles::Aim::new(&e, dir);
        let place = if e.viewmodel { at } else { math::sub(at, origin) };
        let birth = particles::micros(time - p.epoch);
        // (more than the ring holds: only the last ones would stay)
        let skip = count.saturating_sub(p.cap);
        if skip > 0 {
            p.pending.clear();
        }
        if p.pending.is_empty() {
            p.first = (p.head + skip) % p.cap;
        }
        for j in skip..count {
            p.pending.push(particles::spawn(&e, &aim, x.serial.wrapping_add(j), place, birth));
        }
        x.serial = x.serial.wrapping_add(count);
        p.head = ((p.head as u64 + count as u64) % p.cap as u64) as u32;
        if p.pending.len() > p.cap as usize {
            let drop = p.pending.len() - p.cap as usize;
            p.pending.drain(..drop);
            p.first = ((p.first as u64 + drop as u64) % p.cap as u64) as u32;
        }
        p.alive_until = p.alive_until.max(time + e.life[1] as f64);
        Ok(())
    }

    /// (G7f) every particle dies now
    pub fn clear_particles(&mut self) {
        if let Some(p) = &mut self.particles {
            p.fresh = true;
            p.pending.clear();
            p.last.clear();
            p.alive_until = self.time;
        }
    }

    /// (G7f; tests, tools) the particles' list after a frame: a VkDrawIndirectCommand (6 vertices per particle, the particles
    /// alive), then per particle 4 u32: its key (farthest first), its place in the ring, 0, 0; sorted.  None without room
    pub fn particle_buffer(&self) -> Option<&Buffer> {
        self.particles.as_ref().map(|p| &p.list.buf)
    }

    /// (G7f; tests, tools) the ring of records (particles.rs: 16 u32 each; particles::motion reads one)
    pub fn particle_records(&self) -> Option<&Buffer> {
        self.particles.as_ref().map(|p| &p.buf)
    }

    /// (G7f; tests, tools) the epoch of the particles' clock: a record's birth counts microseconds from it
    pub fn particle_epoch(&self) -> f64 {
        self.particles.as_ref().map_or(0.0, |p| p.epoch)
    }

    // ------------------------------------------------------------ a frame

    /// wait for the frame that last used this frame's slot, so its data can be written (the window's
    /// acquire has usually done so).  prepare does it too; call it first to tell the wait from the work.
    /// The browser cannot wait, and needs not: its writes go in queue order
    pub fn wait_slot(&mut self) -> Result<()> {
        if self.waited {
            return Ok(());
        }
        let slot = (self.frame % self.frames.len() as u64) as usize;
        if let Some((id, f)) = self.frames[slot].last {
            let done = self.dev.wait(id, 10_000_000_000)?;
            if !done && !cfg!(target_arch = "wasm32") {
                // not marked as waited: the next call waits again (it went on writing into the
                // slot the GPU still used)
                return Err(GpuError::Timeout);
            }
            if done {
                self.collect(slot, f);
            }
            self.frames[slot].last = None;
        }
        self.waited = true;
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
        // phase 1's records, phase 2's in the second half, (G7f) the blended instances' draw last (drawn with phase 2)
        let words = draws.len() - (BLEND_HEAD / 4) as usize;
        let half = words / 2;
        let n = self.lists.len().min(half / 5);
        let blended = draws[words + 1] as u64;
        let early: u64 = (0..n).map(|i| draws[i * 5 + 1] as u64).sum();
        let late: u64 = (0..n).map(|i| draws[half + i * 5 + 1] as u64).sum::<u64>() + blended;
        let v = self.ts.as_ref().and_then(|ts| ts.read_ms(slot as u32 * STAMPS, STAMPS).ok());
        let mut cascades = [[0.0; 2]; CASCADES_MAX];
        let (mut draw, mut occlusion) = (0.0, [0.0; 3]);
        if let Some(v) = &v {
            // stamps: 0 start, 1 uploads, 2 the camera's culling, 3 + k cascade k's culling, 7 + k its pass, 11 the
            // (phase 1) main pass, 12 the pyramid, 13 phase 2's culling, 14 its pass
            for (k, c) in cascades.iter_mut().enumerate() {
                *c = [v[3 + k] - v[2 + k], v[7 + k] - v[6 + k]];
            }
            occlusion = [v[12] - v[11], v[13] - v[12], v[14] - v[13]];
            draw = v[11] - v[10] + occlusion[2];
        }
        let stamps = v.map(|v| [v[0], v[1], v[2], v[14]]);
        self.finished.push(FrameStats { frame, visible: early + late, late, stamps, cascades, draw, occlusion });
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
        let res = self.grow_buffers(r);
        if res.is_err() {
            // a buffer that could not be made: the capacities raised before it no longer fit
            // the buffers there are -- they all start again at the next frame (everything made
            // anew and uploaded whole)
            (self.cap, self.list_cap, self.info_cap, self.visible_cap, self.punctual_cap) = (0, 0, 0, 0, 0);
            self.full_dirty = true;
            self.lists_dirty = true;
            self.punctual_dirty = true;
        }
        res
    }

    fn grow_buffers(&mut self, r: &Renderer) -> Result<()> {
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
            // (G5d) (cleared with the full upload that follows)
            self.hidden = storage(&dev, hidden_bytes(self.cap), none)?;
            self.full_dirty = true;
            groups = true;
        }
        let lists = self.lists.len() as u32;
        let mut draws_new = false;
        if lists > self.list_cap {
            self.list_cap = lists.max(self.list_cap * 2);
            self.template = dev.create_buffer(&BufferDesc { size: self.list_cap as u64 * DRAW_BYTES, usage: none, memory: MemoryKind::Device })?;
            draws_new = true;
            self.lists_dirty = true;
            // (G7f) the culling's groups bind the draw records: new ones with them.  Until G7f only a list_first that grew at
            // the same time made them anew; once detail levels had grown it earlier, the culling counted into the old
            // records and the frame drew nothing (found by the random sequence of the test `remove`)
            groups = true;
        }
        // (G5d) the lists' records and their detail levels
        let info = lists + self.lists.iter().map(|l| r.lods[l.mesh as usize].len() as u32).sum::<u32>();
        if info > self.info_cap {
            self.info_cap = info.max(self.info_cap * 2);
            self.list_first = storage(&dev, self.info_cap as u64 * 16, none)?;
            self.lists_dirty = true;
            groups = true;
        }
        let room: u64 = self.lists.iter().map(|l| l.cap as u64).sum();
        if room > u32::MAX as u64 {
            return Err(GpuError::Unsupported("Scene: the visible lists have grown too large"));
        }
        // (G7f) room for every blended instance in the blended instances' list (a power of two of tiles: the sort's)
        let mut blend_new = false;
        if self.blend_need > self.blend.cap {
            let cap = self.blend_need.checked_next_power_of_two().ok_or(GpuError::Unsupported("Scene: too many blended instances"))?.max(SORT_TILE);
            self.blend = BlendList { buf: blend_buf(&dev, cap)?, cap, render: None, sort: None };
            blend_new = true;
            groups = true;
        }
        let mut visible_new = false;
        if room as u32 > self.visible_cap {
            self.visible_cap = (room as u32).max(self.visible_cap.saturating_mul(2));
            visible_new = true;
            groups = true;
        }
        // (G5c) room for the point and spot lights
        let mut punctual_new = false;
        if self.punctual.len() as u32 > self.punctual_cap {
            self.punctual_cap = (self.punctual.len() as u32).max(self.punctual_cap * 2);
            self.punctual_buf = storage(&dev, self.punctual_cap as u64 * PUNCTUAL_BYTES, none)?;
            self.punctual_dirty = true;
            punctual_new = true;
        }
        // (G5c) a view per cascade of the sun's shadows
        let want = self.shadows.map_or(0, |s| s.cascades as usize);
        while self.cascades.len() < want {
            let v = view_lists(&dev, r, self.list_cap, self.visible_cap, &self.spheres, &self.list_of, &self.list_first, &self.instances, &self.lights, &self.punctual_buf,
                               &self.hidden, &self.blend.buf, None)?;
            self.cascades.push(v);
        }
        // (G7f) the viewmodel's view
        let mut vm_new = false;
        if self.viewmodel.is_some() && self.vm.is_none() {
            self.vm = Some(view_lists(&dev, r, self.list_cap, self.visible_cap, &self.spheres, &self.list_of, &self.list_first, &self.instances, &self.lights,
                                      &self.punctual_buf, &self.hidden, &self.blend.buf, None)?);
            vm_new = true;
        }
        let (list_cap, visible_cap) = (self.list_cap, self.visible_cap);
        for v in [&mut self.view, &mut self.late].into_iter().chain(self.cascades.iter_mut()).chain(self.vm.iter_mut()) {
            if draws_new {
                v.draws = storage(&dev, list_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?;
            }
            if visible_new {
                v.visible = storage(&dev, visible_cap as u64 * 4, BufferUsage::NONE)?;
            }
            if groups {
                v.group = cull_group(&dev, r, &v.ubo, &self.spheres, &self.list_of, &self.list_first, &v.draws, &v.visible, &self.hidden, &self.blend.buf)?;
            }
            if groups || punctual_new || self.seen_materials != r.bindings_gen {
                v.render = render_group(&dev, r, &v.ubo, &self.instances, &self.lights, &v.visible, &self.punctual_buf)?;
            }
        }
        // (G7f) the blended instances' groups: their draw (the camera's uniform block, instances, lights, materials, their
        // list, vertices, the lists' draw records, both index pools) and their sort, when something in them changed
        if self.blend_on && (self.blend.render.is_none() || groups || draws_new || punctual_new || self.seen_materials != r.bindings_gen) {
            self.blend.render = Some(dev.create_bind_group(&BindGroupDesc {
                layout: &r.blend_layout,
                entries: &[entry(0, &self.view.ubo), entry(1, &self.instances), entry(2, &self.lights), entry(3, &r.material_buf), entry(5, &r.vertices.buf),
                           entry(6, &self.punctual_buf), entry(7, &self.blend.buf), entry(8, &self.late.draws), entry(9, &r.indices16.buf), entry(10, &r.indices32.buf)],
            })?);
        }
        if self.blend_on && (self.blend.sort.is_none() || blend_new) {
            self.blend.sort = Some(dev.create_bind_group(&BindGroupDesc { layout: &r.sort_layout, entries: &[entry(0, &self.blend.buf)] })?);
        }
        // (G7f) the particles' groups: their motion (the frame, the ring, the list), their sort, their draw (the camera's and
        // the viewmodel's uniform blocks, the lights)
        if let Some(p) = &mut self.particles {
            if p.cs_group.is_none() {
                p.cs_group = Some(dev.create_bind_group(&BindGroupDesc { layout: &r.post.particles_cs_layout, entries: &[entry(0, &p.frame_ubo), entry(1, &p.buf), entry(2, &p.list.buf)] })?);
                p.list.sort = Some(dev.create_bind_group(&BindGroupDesc { layout: &r.sort_layout, entries: &[entry(0, &p.list.buf)] })?);
                p.render = None;
            }
            if p.render.is_none() || vm_new {
                let vm_ubo = self.vm.as_ref().map_or(&self.view.ubo, |v| &v.ubo);
                p.render = Some(dev.create_bind_group(&BindGroupDesc {
                    layout: &r.post.particles_layout,
                    entries: &[entry(0, &p.frame_ubo), entry(1, &p.buf), entry(3, &self.view.ubo), entry(4, vm_ubo), entry(5, &self.lights), entry(6, &p.list.buf)],
                })?);
            }
        }
        if groups {
            for f in &mut self.frames {
                f.scatter_group = None;
            }
        }
        self.seen_materials = r.bindings_gen;
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
        let n = self.lists.len();
        for l in &mut self.lists {
            l.wide = r.meshes[l.mesh as usize].wide;
            // (G7f) the id in its slot now (a removed mesh's list serves the next one there)
            l.id = r.mesh_ids.id(l.mesh as usize);
        }
        // the draw records by width of indices, alpha class and side: each combination's lists side by side (one
        // multi-draw per side and class in the main pass, and the shadow passes' pipelines take both sides)
        self.order.clear();
        self.order.extend(0..n as u32);
        let lists = &self.lists;
        // ((G7f) the viewmodel's after the world's: their runs apart)
        self.order.sort_by_key(|&i| {
            let l = &lists[i as usize];
            (l.vm, l.wide, l.class.index(), l.double, l.mirrored, i)
        });
        for (k, &i) in self.order.iter().enumerate() {
            self.lists[i as usize].record = k as u32;
        }
        let mut places: Vec<[u32; 4]> = Vec::with_capacity(n);
        let mut levels: Vec<[u32; 4]> = Vec::new();
        let mut draws = vec![[0u32; 5]; n];
        for l in &mut self.lists {
            let m = &r.meshes[l.mesh as usize];
            l.first = first;
            // (G5d) its detail levels: threshold and list (u32::MAX: not drawn), after the lists: the lists of the
            // same side and class
            let lods = &r.lods[l.mesh as usize];
            // (G7f) the number of levels and the list's flags (cull.wgsl: 32-bit indices, double-sided, mirrored, blended)
            let flags = (l.vm as u32) << 27 | (l.wide as u32) << 28 | (l.double as u32) << 29 | (l.mirrored as u32) << 30 | ((l.class == AlphaClass::Blend) as u32) << 31;
            places.push([first, (n + levels.len()) as u32, lods.len() as u32 | flags, l.record]);
            for d in lods {
                let to = d.mesh.map_or(NO_LIST, |t| self.list_of_mesh.get(slot(t.0)).map_or(NO_LIST, |x| x[l.mirrored as usize][l.class.index()][l.double as usize][l.vm as usize]));
                levels.push([d.below.to_bits(), to, 0, 0]);
            }
            // (Basis: no first instance is sure to work: the list's start is an offset of the vertex buffer)
            draws[l.record as usize] = [m.indices, 0, m.first_index, m.base_vertex, if r.multi { first } else { 0 }];
            first += l.cap;
        }
        places.extend_from_slice(&levels);
        self.info_len = places.len() as u32;
        self.runs.clear();
        for &i in &self.order {
            let l = &self.lists[i as usize];
            match self.runs.last_mut() {
                Some(r) if r.wide == l.wide && r.mirrored == l.mirrored && r.class == l.class && r.double == l.double && r.vm == l.vm => r.count += 1,
                _ => self.runs.push(Run { first: l.record, count: 1, wide: l.wide, mirrored: l.mirrored, class: l.class, double: l.double, vm: l.vm, room: false }),
            }
        }
        let info = self.info_len as u64 * 16;
        let size = info + n as u64 * DRAW_BYTES;
        let up = self.dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Upload })?;
        up.write(0, &places)?;
        up.write(info, &draws)?;
        self.upload.lists = true;
        self.upload.bytes += size;
        self.lists_up = Some(up);
        Ok(())
    }

    /// which runs have room this frame (rooms change with every prepare, the runs only with the lists' layout)
    fn mark_runs(&mut self) {
        let mut at = 0usize;
        for run in &mut self.runs {
            let lists = &self.order[at..at + run.count as usize];
            run.room = lists.iter().any(|&i| self.lists[i as usize].room > 0);
            at += run.count as usize;
        }
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
        let (origin, cam) = (self.origin, self.camera);
        for (k, chunk) in self.slots.chunks_mut(CHUNK).enumerate() {
            spheres.clear();
            lists.clear();
            words.clear();
            for s in chunk.iter_mut() {
                s.dirty = false;
                if s.live {
                    let (sphere, w) = records(s, r, origin, &cam);
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
        // (G5d) every slot counts as seen again
        self.reset_hidden = true;
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
        let (origin, cam) = (self.origin, self.camera);
        for &i in &self.dirty {
            let s = &mut self.slots[i as usize];
            s.dirty = false;
            let mut row = [0u32; CHANGE_WORDS];
            row[0] = i;
            if s.live {
                let (sphere, w) = records(s, r, origin, &cam);
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
                entries: &[entry(0, buf), entry(1, &self.spheres), entry(2, &self.list_of), entry(3, &self.instances), entry(4, &self.hidden)],
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
        // (G7f) an orthographic camera's values
        self.camera.check()?;
        self.wait_slot()?;
        // (G7f) a removed mesh, or a new one in an old slot: the lists' draw records again
        if self.seen_mesh_gen != r.mesh_gen {
            self.seen_mesh_gen = r.mesh_gen;
            self.lists_dirty = true;
        }
        self.seen_removals = r.removals;
        // (G5d) new detail levels: new spheres and new tables of the lists
        if self.seen_lods != r.lod_gen {
            self.seen_lods = r.lod_gen;
            self.full_dirty = true;
            self.lists_dirty = true;
        }
        // the last prepare never reached a submission, or failed half way: what it sent is lost
        if self.sent {
            self.full_dirty = true;
            self.lists_dirty = true;
            self.punctual_dirty = true;
            r.resend_materials();
            // (G7f) the particles it sent are born again
            if let Some(p) = &mut self.particles {
                if !p.last.is_empty() {
                    let mut again = core::mem::take(&mut p.last);
                    p.first = p.last_first;
                    again.append(&mut p.pending);
                    let cap = p.cap as usize;
                    if again.len() > cap {
                        let drop = again.len() - cap;
                        again.drain(..drop);
                        p.first = ((p.first as u64 + drop as u64) % p.cap as u64) as u32;
                    }
                    p.pending = again;
                }
            }
        }
        // (G7f) the viewmodel's instances follow the camera: their records again when it moved
        if self.viewmodel.is_some() && !self.vm_slots.is_empty() && self.vm_camera != Some(self.camera) {
            self.vm_camera = Some(self.camera);
            for k in 0..self.vm_slots.len() {
                let i = self.vm_slots[k];
                self.mark(i);
            }
        }
        self.sent = true;
        self.prepared = false;
        self.stale = false;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        self.slot = slot;
        self.upload = UploadInfo::default();
        // (G5d) a new buffer of bits is cleared by the frame that first uses it (and by every full upload)
        self.reset_hidden = core::mem::take(&mut self.hidden_fresh);
        self.full_up = None;
        self.changed = 0;
        self.upload.bytes += r.prepare_materials()?;
        self.settle_lists(r);
        // (G7f) the viewmodel draws no blended instances
        if self.lists.iter().any(|l| l.vm && l.class == AlphaClass::Blend && l.count > 0) {
            return Err(GpuError::Invalid("Scene::prepare: an instance of the viewmodel has a blended material"));
        }
        self.fit_lists(r);
        self.grow(r)?;
        self.upload_lists(r)?;
        self.mark_runs();
        // this device's stats need a read-back per frame and timestamps
        if self.stats_on {
            if self.ts.is_none() && self.dev.features().timestamps {
                self.ts = Some(self.dev.create_timestamps(STAMPS * self.frames.len() as u32)?);
            }
            // (G5d) phase 1's records, then phase 2's, (G7f) then the blended instances' draw
            let size = 2 * self.list_cap as u64 * DRAW_BYTES + BLEND_HEAD;
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
        let tm = if self.tonemap == Tonemap::Neutral { 1.0 } else { 0.0 };
        let mut lights = [0.0f32; crate::LIGHTS_BYTES as usize / 4];
        lights[..12].copy_from_slice(&[d[0] as f32, d[1] as f32, d[2] as f32, self.exposure, s.color[0], s.color[1], s.color[2], tm, s.ambient[0], s.ambient[1], s.ambient[2], 0.0]);
        // (G5c) the sun's cascades
        self.cascade_info = match &self.shadows {
            Some(sh) => {
                r.shadow.ensure(&self.dev, sh.size)?;
                shadow::cascades(&self.camera, s.direction, sh)
            }
            None => Vec::new(),
        };
        lights[12] = self.cascade_info.len() as f32;
        lights[13] = self.shadows.map_or(1.0, |sh| sh.size as f32);
        // (G5c) the environment: on (its strength), its cube's top level, the harmonics
        if let (Some(k), Some(env)) = (self.environment, &r.env) {
            lights[80] = k.max(1e-30);
            lights[81] = (env.levels.len() - 1) as f32;
            for (i, c) in env.sh.iter().enumerate() {
                lights[84 + i * 4..87 + i * 4].copy_from_slice(c);
            }
        }
        // (G5c) the point and spot lights, when they changed (or the origin did, or what went up was lost)
        lights[14] = self.punctual.len() as f32;
        self.punctual_up = 0;
        if self.punctual_dirty && !self.punctual.is_empty() {
            let n = self.punctual.len();
            let bytes = n as u64 * PUNCTUAL_BYTES;
            let f = &mut self.frames[slot];
            if f.punctual.as_ref().is_none_or(|b| b.size() < bytes) {
                f.punctual = Some(self.dev.create_buffer(&BufferDesc { size: bytes.next_power_of_two().max(1024), usage: BufferUsage::NONE, memory: MemoryKind::Upload })?);
            }
            let words: Vec<[f32; 16]> = self.punctual.iter().map(|l| l.words(self.origin)).collect();
            if let Some(b) = &f.punctual {
                b.write(0, &words)?;
            }
            self.punctual_up = n as u32;
            self.upload.bytes += bytes;
        }
        self.punctual_dirty = false;
        // (G7f) the particles born since the last prepare, into this frame's upload buffer
        self.prepare_particles(slot)?;
        let f = &self.frames[slot];
        // (G5d) the cascades take the camera's detail levels; (G7f) with a viewmodel the world's depth takes 0 ..= 0.5
        let depth = if self.viewmodel.is_some() { (0.5, 0.0) } else { (1.0, 0.0) };
        let view = self.camera.view_block(self.origin, depth);
        let lod = [view[48], view[49], view[50], view[51]];
        for (k, c) in self.cascade_info.iter().enumerate() {
            lights[16 + k * 16..32 + k * 16].copy_from_slice(&c.receiver(self.origin));
            f.stage.write(CASCADE_AT + k as u64 * CASCADE_STRIDE, &c.view_block(self.origin, lod, view[56]))?;
        }
        f.stage.write(0, &view)?;
        f.stage.write(LIGHTS_AT, &lights)?;
        // (G7f) the blended instances' draw: the largest blended mesh's vertices per instance, none counted yet
        if self.blend_on {
            f.stage.write(BLEND_AT, &[self.blend_k, 0u32, 0, 0])?;
        }
        // (G7f) the viewmodel's view: the camera's eye and axes, its lens, its depth 0.5 ..= 1
        let vm_cam = self.viewmodel.map(|v| v.camera(&self.camera));
        if let Some(c) = &vm_cam {
            f.stage.write(VM_AT, &c.view_block(self.origin, (0.5, 0.5)))?;
            self.upload.bytes += crate::VIEW_BYTES;
        }
        // (G7f) the outline's uniform block: colour, width, crease, the lenses (post.wgsl Outline)
        if let Some(o) = &self.outline {
            let lens = |b: &[f32; 60]| [b[46], b[47], b[45], b[55]];
            let world = lens(&view);
            let vm = vm_cam.map_or([0.0; 4], |c| lens(&c.view_block(self.origin, (0.5, 0.5))));
            let mut words = [0.0f32; OUTLINE_BYTES as usize / 4];
            words[..4].copy_from_slice(&o.color);
            words[4..8].copy_from_slice(&[o.width as f32, o.crease, if vm_cam.is_some() { 1.0 } else { 0.0 }, 0.0]);
            words[8..12].copy_from_slice(&world);
            words[12..16].copy_from_slice(&vm);
            f.stage.write(POST_AT, &words)?;
        }
        // (G7f) the particles' frame (particles.wgsl Frame) and their list's head (6 vertices per particle, none counted)
        if let Some(p) = &self.particles {
            let e = math::sub(self.camera.eye, self.origin);
            let (fw, u) = (self.camera.forward, self.camera.up);
            let rr = math::cross(fw, u);
            let now = particles::micros(self.time - p.epoch);
            let v = |a: V3, w: f64| [(a[0] as f32).to_bits(), (a[1] as f32).to_bits(), (a[2] as f32).to_bits(), (w as f32).to_bits()];
            let mut words = [0u32; PFRAME_BYTES as usize / 4];
            words[..4].copy_from_slice(&[now, p.cap, self.camera.ortho.is_some() as u32, 0]);
            words[4..8].copy_from_slice(&v(e, 0.0));
            words[8..12].copy_from_slice(&v(fw, self.camera.near));
            words[12..16].copy_from_slice(&v(rr, 0.0));
            words[16..20].copy_from_slice(&v(u, 0.0));
            f.stage.write(PFRAME_AT, &words)?;
            f.stage.write(PHEAD_AT, &[6u32, 0, 0, 0])?;
        }
        self.upload.bytes += crate::VIEW_BYTES * (1 + self.cascade_info.len() as u64) + crate::LIGHTS_BYTES;
        self.prepared = true;
        Ok(())
    }

    /// (G7f) the particles born since the last prepare into this frame's upload buffer, and where they go in the ring (two
    /// runs at most: its end, then its start)
    fn prepare_particles(&mut self, slot: usize) -> Result<()> {
        let Some(p) = &mut self.particles else { return Ok(()) };
        p.copies = [(0, 0, 0); 2];
        p.last.clear();
        if p.pending.is_empty() {
            return Ok(());
        }
        let n = p.pending.len() as u64;
        let bytes = n * PARTICLE_BYTES;
        let f = &mut self.frames[slot];
        if f.spawn.as_ref().is_none_or(|b| b.size() < bytes) {
            f.spawn = Some(self.dev.create_buffer(&BufferDesc { size: bytes.next_power_of_two().max(4096), usage: BufferUsage::NONE, memory: MemoryKind::Upload })?);
        }
        if let Some(b) = &f.spawn {
            b.write(0, &p.pending)?;
        }
        let first = p.first;
        let run = (p.cap - first).min(n as u32);
        p.copies[0] = (first, run, 0);
        if (run as u64) < n {
            p.copies[1] = (0, n as u32 - run, run as u64 * PARTICLE_BYTES);
        }
        p.last_first = first;
        p.last = core::mem::take(&mut p.pending);
        self.upload.bytes += bytes;
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
        // (G7f) the frame cleared a new ring and took the particles born for it
        if let Some(p) = &mut self.particles {
            p.fresh = false;
            p.last.clear();
        }
    }
}

/// (G7f) a scene that goes uses no mesh or material any more
impl Drop for Scene {
    fn drop(&mut self) {
        give_all(&self.counts, &self.mesh_use, &self.material_use);
    }
}
