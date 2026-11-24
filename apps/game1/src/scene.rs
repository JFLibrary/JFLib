// scene -- what game 1 draws: render3d in a ui.gpu_view (or offscreen for the tests and the measurement).  The look
// the user chose (variant C, docs/GAME.md G7f): the cartoon material with four soft bands and no outlines, the rim
// light on figures and props only, FXAA; the weapon in front of the camera (render3d's viewmodel), muzzle flashes,
// tracers, sparks and puffs as particles
//
//   Frame      what the game hands over each frame: the eye and view, the figures' poses, the viewmodel's state
//   Fx         effects waiting for the next draw (particles are born on the CPU when the frame is drawn)
//   View       the GpuContent: attach builds the renderer, the map's meshes and instances, the figure's parts and
//              materials (a jacket per colour), the emitters; draw syncs the figures' instances (one per part,
//              created when a figure appears, removed when it goes), the viewmodel, emits the effects, records

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

use game::{FirstPerson, Vec3};
use game1_core::map::{Map, ModelData, MAT_RIM, PART_DRAW};
use gpu::{ColorAttachment, CommandEncoder, Device, SubmitId};
use render3d::{Antialias, Emitter, EmitterId, InstanceId, Material, MaterialDesc, MaterialId, MeshData, MeshId, ParticleBlend, ParticleShape, Renderer, RendererDesc, Scene, Shadows, Sun, Toon, Viewmodel};

use crate::look::{self, Group, Pose, VmState, PALETTE};

/// what a frame shows
#[derive(Clone, Debug)]
pub struct Frame {
    pub eye: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    /// the vertical field of view (radians)
    pub fov: f64,
    /// the other figures: net id and pose
    pub figures: Vec<(u16, Pose)>,
    pub vm: VmState,
    /// the weapon in front of the camera (not while dead, not in a free view)
    pub show_vm: bool,
    /// the game's time in seconds (the particles' clock)
    pub time: f64,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { eye: Vec3::new(0.0, 6.0, 26.0), yaw: 0.0, pitch: -0.2, fov: 1.25, figures: Vec::new(), vm: VmState::default(), show_vm: false, time: 0.0 }
    }
}

/// an effect for the next draw
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fx {
    /// the own muzzle flash (in front of the camera)
    MuzzleOwn,
    /// another figure's muzzle flash
    Muzzle { at: Vec3, dir: Vec3 },
    /// a tracer from a muzzle to where the shot ended
    Tracer { from: Vec3, to: Vec3 },
    /// the own shot's tracer: from the muzzle in front of the camera (where the screen shows it) to where it ended
    TracerOwn { to: Vec3 },
    /// a shot ended on the map: sparks and dust along the surface's normal
    Impact { at: Vec3, normal: Vec3 },
    /// a figure was hit: sparks at its chest
    Hit { at: Vec3 },
    /// a figure was ended: a puff in its colour
    Poof { at: Vec3, color: [f32; 3] },
}

/// the toon look C: four soft bands, a rim on figures and props
fn toon(rim: bool) -> Toon {
    let mut t = Toon::new(4);
    t.softness = 0.5;
    t.rim = if rim { 0.2 } else { 0.0 };
    t.rim_width = if rim { 0.35 } else { 0.0 };
    t
}

fn material(c: [f32; 3], rim: bool) -> MaterialDesc {
    let mut d = MaterialDesc::new(Material::color(c[0], c[1], c[2]));
    d.toon = Some(toon(rim));
    d
}

fn mesh_of(r: &mut Renderer, pos: &[[f32; 3]], nrm: &[[f32; 3]], idx: &[u32]) -> gpu::Result<MeshId> {
    let uv = alloc::vec![[0.0f32; 2]; pos.len()];
    r.add_mesh(&MeshData { positions: pos, normals: nrm, uvs: &uv, indices: idx })
}

struct Emitters {
    muzzle_vm: EmitterId,
    /// the own muzzle flash as made (for the weapon's scale VM_TUNED) and the scale it was last set for
    muzzle_vm_e: Emitter,
    muzzle_vm_scale: f64,
    muzzle: EmitterId,
    tracer: EmitterId,
    sparks: EmitterId,
    dust: EmitterId,
    poof: EmitterId,
    tracer_e: Emitter,
    poof_e: Emitter,
}

/// the figure's parts on the GPU
struct FigParts {
    /// mesh, material (None: the jacket), group, in the viewmodel
    parts: Vec<(MeshId, Option<MaterialId>, Group, bool)>,
    jackets: Vec<MaterialId>,
}

struct Gpu {
    r: Renderer,
    s: Scene,
    fig: FigParts,
    figs: BTreeMap<u16, Vec<InstanceId>>,
    vm: Vec<InstanceId>,
    em: Emitters,
}

/// what the last frames cost (the HUD's numbers, the measurement)
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawStats {
    pub frames: u64,
    pub instances: usize,
}

/// the view's content (see the module header)
pub struct View {
    pub frame: Frame,
    pub fx: Vec<Fx>,
    pub map: Arc<Map>,
    pub figure: ModelData,
    pub anchor: Vec3,
    pub muzzle: Vec3,
    pub stats: DrawStats,
    /// a sky colour (the clear colour, codes of the target)
    pub sky: [f64; 4],
    gpu: Option<Gpu>,
}

impl View {
    pub fn new(map: Arc<Map>) -> View {
        let figure = ModelData::decode(look::FIGURE).unwrap_or_default();
        let (anchor, muzzle) = look::weapon_points(&figure);
        View { frame: Frame::default(), fx: Vec::new(), map, figure, anchor, muzzle, stats: DrawStats::default(), sky: [0.19, 0.40, 0.62, 1.0], gpu: None }
    }

    /// the renderer and scene, to measure
    pub fn parts_mut(&mut self) -> Option<(&mut Renderer, &mut Scene)> {
        self.gpu.as_mut().map(|g| (&mut g.r, &mut g.s))
    }

    fn build(&self, dev: &Device) -> gpu::Result<Gpu> {
        let mut r = Renderer::new(dev, &RendererDesc::new(ui::GPU_VIEW_FORMAT))?;
        let mut s = Scene::new(&r)?;
        // the map: every drawn part an instance at the origin
        let mut mats = Vec::new();
        for m in &self.map.data.materials {
            mats.push(r.add_material_desc(&material(m.color, m.flags & MAT_RIM != 0))?);
        }
        for p in self.map.data.parts.iter().filter(|p| p.flags & PART_DRAW != 0 && !p.indices.is_empty()) {
            let mesh = mesh_of(&mut r, &p.positions, &p.normals, &p.indices)?;
            s.add(render3d::Instance { mesh, material: mats[p.material as usize], transform: render3d::Transform::at([0.0; 3]) })?;
        }
        // the figure: its parts and materials, a jacket per colour
        let mut fmats = Vec::new();
        for m in &self.figure.materials {
            fmats.push(r.add_material_desc(&material(m.color, true))?);
        }
        let mut jackets = Vec::new();
        for c in PALETTE {
            jackets.push(r.add_material_desc(&material(c, true))?);
        }
        let mut parts = Vec::new();
        for p in self.figure.parts.iter().filter(|p| p.flags & PART_DRAW != 0 && !p.indices.is_empty()) {
            let mesh = mesh_of(&mut r, &p.positions, &p.normals, &p.indices)?;
            let jacket = self.figure.materials.get(p.material as usize).is_some_and(|m| m.name == "jacke");
            parts.push((mesh, if jacket { None } else { Some(fmats[p.material as usize]) }, look::group(&p.name), look::in_viewmodel(&p.name)));
        }
        // the weapon in front of the camera (the own jacket: the first colour)
        s.set_viewmodel(Some(&Viewmodel::new(look::VM_FOV, 0.02, 4.0)))?;
        let hidden = render3d::Transform { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], scale: [0.0; 3] };
        let mut vm = Vec::new();
        for (mesh, mat, _, in_vm) in &parts {
            if *in_vm {
                vm.push(s.add_viewmodel(render3d::Instance { mesh: *mesh, material: mat.unwrap_or(jackets[0]), transform: hidden })?);
            }
        }
        // light, shadows, anti-aliasing
        s.set_sun(&Sun { direction: [0.45, 0.85, 0.30], color: [2.7, 2.5, 2.25], ambient: [0.30, 0.34, 0.42] });
        s.set_shadows(Some(&Shadows::for_level(dev.level())))?;
        s.set_antialias(Antialias::Fxaa);
        s.set_particles(8192)?;
        let em = emitters(&mut s)?;
        Ok(Gpu { r, s, fig: FigParts { parts, jackets }, figs: BTreeMap::new(), vm, em })
    }

    /// the figures and the viewmodel into the scene, the effects as particles (a window of this width / height)
    fn sync(&mut self, aspect: f64) -> gpu::Result<()> {
        let Some(g) = self.gpu.as_mut() else { return Ok(()) };
        let f = &self.frame;
        // figures that went
        let gone: Vec<u16> = g.figs.keys().copied().filter(|id| !f.figures.iter().any(|x| x.0 == *id)).collect();
        for id in gone {
            if let Some(v) = g.figs.remove(&id) {
                for i in v {
                    g.s.remove(i)?;
                }
            }
        }
        for (id, pose) in &f.figures {
            if !g.figs.contains_key(id) {
                let jacket = g.fig.jackets[*id as usize % g.fig.jackets.len()];
                let mut v = Vec::new();
                for (mesh, mat, grp, _) in &g.fig.parts {
                    let t = game_render3d::transform(&look::part_transform(*grp, pose));
                    v.push(g.s.add(render3d::Instance { mesh: *mesh, material: mat.unwrap_or(jacket), transform: t })?);
                }
                g.figs.insert(*id, v);
            } else if let Some(v) = g.figs.get(id) {
                for (k, (_, _, grp, _)) in g.fig.parts.iter().enumerate() {
                    g.s.set_transform(v[k], &game_render3d::transform(&look::part_transform(*grp, pose)))?;
                }
            }
        }
        // the viewmodel
        let t = if f.show_vm { game_render3d::transform(&look::viewmodel(self.anchor, &f.vm, aspect)) } else { render3d::Transform { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], scale: [0.0; 3] } };
        for i in &g.vm {
            g.s.set_transform(*i, &t)?;
        }
        // the effects
        g.s.set_time(f.time);
        // the muzzle in front of the camera and where it points; the point of the world the screen shows there (the
        // own tracer's start)
        let muzzle_cam = look::viewmodel_point(self.anchor, &f.vm, aspect, self.muzzle);
        let muzzle_dir = look::viewmodel_dir(self.anchor, &f.vm, aspect);
        let muzzle_world = look::vm_to_world(muzzle_cam, f.eye, f.yaw, f.pitch, f.fov);
        for fx in self.fx.drain(..) {
            let v = |p: Vec3| [p.x, p.y, p.z];
            let tracer = match fx {
                Fx::MuzzleOwn => {
                    // the flash grows and shrinks with the weapon
                    let scale = look::vm_scale(aspect);
                    if scale != g.em.muzzle_vm_scale {
                        let m = (scale / look::VM_TUNED) as f32;
                        let mut e = g.em.muzzle_vm_e;
                        e.size = [e.size[0] * m, e.size[1] * m];
                        e.radius *= m;
                        e.speed = [e.speed[0] * m, e.speed[1] * m];
                        g.s.set_emitter(g.em.muzzle_vm, &e)?;
                        g.em.muzzle_vm_scale = scale;
                    }
                    g.s.emit(g.em.muzzle_vm, v(muzzle_cam), v(muzzle_dir), 10)?;
                    None
                }
                Fx::Muzzle { at, dir } => {
                    g.s.emit(g.em.muzzle, v(at), v(dir), 10)?;
                    None
                }
                Fx::Tracer { from, to } => Some((from, to)),
                Fx::TracerOwn { to } => Some((muzzle_world, to)),
                Fx::Impact { at, normal } => {
                    g.s.emit(g.em.sparks, v(at), v(normal), 8)?;
                    g.s.emit(g.em.dust, v(at), v(normal), 5)?;
                    None
                }
                Fx::Hit { at } => {
                    g.s.emit(g.em.sparks, v(at), [0.0, 1.0, 0.0], 14)?;
                    None
                }
                Fx::Poof { at, color } => {
                    g.em.poof_e.color = [[color[0], color[1], color[2], 0.9], [color[0] * 0.6, color[1] * 0.6, color[2] * 0.6, 0.0]];
                    g.s.set_emitter(g.em.poof, &g.em.poof_e)?;
                    g.s.emit(g.em.poof, v(at), [0.0, 1.0, 0.0], 40)?;
                    None
                }
            };
            if let Some((from, to)) = tracer {
                let d = to - from;
                let len = d.length();
                if len > 1.0 {
                    let life = (len / 260.0) as f32;
                    g.em.tracer_e.life = [life, life];
                    g.s.set_emitter(g.em.tracer, &g.em.tracer_e)?;
                    g.s.emit(g.em.tracer, v(from), v(d * (1.0 / len)), 1)?;
                }
            }
        }
        self.stats.instances = g.s.len() as usize;
        Ok(())
    }

    /// set the camera, sync, prepare (a frame of size w x h follows with record)
    pub fn prepare(&mut self, w: u32, h: u32) -> gpu::Result<()> {
        let aspect = w as f64 / h.max(1) as f64;
        self.sync(aspect)?;
        let f = &self.frame;
        let mut fp = FirstPerson::new(f.eye);
        fp.yaw = f.yaw;
        fp.pitch = f.pitch;
        let cam = game_render3d::camera(&fp.view(), f.fov, aspect, 0.05, f64::INFINITY);
        if let Some(g) = self.gpu.as_mut() {
            g.r.set_size(w, h)?;
            g.s.set_camera(&cam);
            g.s.prepare(&mut g.r)?;
        }
        self.stats.frames += 1;
        Ok(())
    }

    /// record the prepared frame
    pub fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, depth: Option<gpu::DepthAttachment<'a>>) -> gpu::Result<()> {
        let Some(g) = &self.gpu else { return Err(gpu::GpuError::Invalid("not attached")) };
        let c = self.sky;
        g.r.record(e, &g.s, ColorAttachment { clear: [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32], ..target }, depth)
    }
}

fn emitters(s: &mut Scene) -> gpu::Result<Emitters> {
    let mut m = Emitter::new(11);
    m.viewmodel = true;
    m.life = [0.035, 0.07];
    m.speed = [0.3, 1.5];
    m.spread = 0.7;
    m.radius = 0.01;
    m.size = [0.07, 0.015];
    m.color = [[3.0, 2.0, 0.7, 1.0], [1.5, 0.5, 0.1, 0.0]];
    let muzzle_vm = s.add_emitter(&m)?;
    let mut w = m;
    w.seed = 12;
    w.viewmodel = false;
    w.size = [0.28, 0.05];
    let muzzle = s.add_emitter(&w)?;
    let mut t = Emitter::new(13);
    t.shape = ParticleShape::Streak;
    t.stretch = 0.012;
    t.speed = [260.0, 260.0];
    t.size = [0.035, 0.035];
    t.color = [[4.0, 3.6, 2.2, 0.9], [4.0, 3.0, 1.4, 0.5]];
    let tracer = s.add_emitter(&t)?;
    let mut k = Emitter::new(14);
    k.shape = ParticleShape::Streak;
    k.stretch = 0.03;
    k.life = [0.15, 0.35];
    k.speed = [3.0, 8.0];
    k.spread = 1.0;
    k.size = [0.035, 0.01];
    k.gravity = 9.0;
    k.color = [[5.0, 3.0, 1.0, 1.0], [2.0, 0.6, 0.1, 0.0]];
    let sparks = s.add_emitter(&k)?;
    let mut d = Emitter::new(15);
    d.blend = ParticleBlend::Alpha;
    d.life = [0.35, 0.7];
    d.speed = [0.3, 1.2];
    d.spread = 1.2;
    d.size = [0.15, 0.55];
    d.drag = 2.0;
    d.color = [[0.62, 0.6, 0.57, 0.55], [0.62, 0.6, 0.57, 0.0]];
    let dust = s.add_emitter(&d)?;
    let mut p = Emitter::new(16);
    p.blend = ParticleBlend::Alpha;
    p.life = [0.5, 0.9];
    p.speed = [1.0, 3.0];
    p.spread = core::f32::consts::PI;
    p.radius = 0.3;
    p.size = [0.35, 0.85];
    p.gravity = -1.0;
    p.drag = 3.0;
    let poof = s.add_emitter(&p)?;
    Ok(Emitters { muzzle_vm, muzzle_vm_e: m, muzzle_vm_scale: look::VM_TUNED, muzzle, tracer, sparks, dust, poof, tracer_e: t, poof_e: p })
}

impl ui::GpuContent for View {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        self.gpu = None;
        self.gpu = Some(self.build(dev)?);
        Ok(())
    }

    fn detach(&mut self) {
        self.gpu = None;
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, (w, h): (u32, u32)) -> gpu::Result<()> {
        self.prepare(w, h)?;
        let me: &'a Self = self;
        me.record(e, target, None)
    }

    fn submitted(&mut self, id: SubmitId) {
        if let Some(g) = &mut self.gpu {
            g.s.submitted(id);
        }
    }
}

impl View {
    /// attach outside a gpu_view (offscreen: tests, the measurement)
    pub fn attach_to(&mut self, dev: &Device) -> gpu::Result<()> {
        ui::GpuContent::attach(self, dev)
    }

    /// the frame went to the queue (offscreen)
    pub fn submitted_as(&mut self, id: SubmitId) {
        ui::GpuContent::submitted(self, id)
    }
}
