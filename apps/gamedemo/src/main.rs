// gamedemo -- (G7b) a first-person demo of the game core: a ground, boxes, a few turning above them; WASD
// by where the keys sit, the mouse looks while the pointer is locked; a HUD over the view
//
//   click into the view      locks the pointer (that click fires nothing); the game plays
//   W A S D (their places)   walk, Shift (left) runs; the boxes stop the walker (a capsule cast
//                            through lib/physics, sliding along what it touches)
//   the mouse                looks (game::Input: raw motion x the platform's factor x sensitivity)
//   Esc                      the lock's: it ends, the game pauses (the simulation stands, the clock
//                            does not catch up); a click plays on
//
// The parts: lib/game (World with Pose components, Systems at 60 Hz, Input, FirstPerson), the bridge
// game_render3d (entities with a MeshRef as render3d instances, only changes written, the origin
// follows the camera), render3d in a ui.gpu_view with AF_GPU_3D, the HUD drawn over it by lib/ui (veils:
// lib/ui composes it on the GPU, G7b part 2).  The world lives in the view's content (View): the app's
// frame feeds the input and runs the ticks, the content's draw syncs the world into the scene at the
// frame's alpha.
//
// (G7b part 2) Four beacons are a glTF model (assets/beacon.gltf, app://, its texture a PNG inside it) loaded
// through lib/game's Assets in the background; the entities with its ModelRef show it once it is up
// (game_render3d's Models and Bridge::sync_models).  A turning box hums (an Emitter, a generated tone), heard
// through game::audio's SpatialAudio from the camera while the game plays (not in script runs).
//
// (G7b part 2) The window path measured (bench.rs, own.rs; docs/GAMECORE.md "Spielfenster"):
//   gamedemo --bench view [--present fifo|mailbox|immediate] [--fullscreen] [--limit HZ] [--n N] [--secs S]
//            path (a): this window, the gpu_view over all of it, the HUD over it with veils, with the read-back
//            (Ui::gpu_test_veil(false)) and without a HUD: input to the screen and the frames on the screen
//   gamedemo --bench own [--present ..] [--fullscreen] [--limit HZ] [--n N] [--secs S]
//            path (b): a window and a swapchain of its own, the HUD a texture over the 3D picture
//
//   python build_rs.py run gamedemo                  (Linux: --target linux; macOS on a Mac;
//   python build_rs.py test gamedemo                  the browser: build with --target web)
//   build/win/gamedemo.exe --script apps/gamedemo/tests/shot.uis
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod bench;
mod own;
mod probe;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use game::assets::{Assets, Change, Handle, Loading, State};
use game::audio::{Attenuation, Emitter, SpatialAudio, Speakers};
use game::input::platform_motion_factor;
use game::{ActionId, ActionKind, ActionMap, Bind, Entity, FirstPerson, Game, Input, Pose, Quat, Source, Step, TickInput, Transform, Vec3, World};
use game_render3d::{Bridge, MeshRef, Model, ModelRef, Models};
use gpu::{ColorAttachment, CommandEncoder, Device, GpuError, SubmitId};
use physics::{Filter, Iso, Shape, World as PhysicsWorld};
use plat::{KC_A, KC_D, KC_S, KC_SHIFT_LEFT, KC_W};
use render3d::{MaterialId, MeshData, MeshId, Renderer, RendererDesc, Shadows, Sun};
use ui::*;

use crate::probe::Probe;

/// the app's own version: version= of apps/gamedemo/PACKAGE
const APP_VERSION: &str = rt::app_version!();

const ID_VIEW: u32 = 100;
const CMD_QUIT: u32 = 1;
const TIMER_FRAME: u32 = 1;

static CMDS: [Command; 1] = [Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0)];

const DESC: AppDesc = AppDesc {
    title: "Spiel-Demo",
    flags: AF_RESIZABLE | AF_GPU_3D,
    w: 1024,
    h: 640,
    min_w: 320,
    min_h: 240,
    cmds: &CMDS,
    ..AppDesc::NONE
};

/// the eye above the walker's feet, the walker's capsule (half height, radius), speeds (m/s)
pub(crate) const EYE: f64 = 1.7;
const HALF: f64 = 0.6;
const RADIUS: f64 = 0.3;
const WALK: f64 = 4.5;
const RUN: f64 = 8.0;
/// the vertical field of view (radians)
pub(crate) const FOV: f64 = 1.15;
/// the sky (linear light)
pub(crate) const SKY: [f32; 4] = [0.62, 0.76, 0.92, 1.0];

/// a box turning about its y axis (radians a second)
#[derive(Clone, Copy)]
struct Spin(f64);

/// what the systems use besides the world
pub(crate) struct Res {
    pub(crate) input: Input,
    /// the input of the tick (taken before each step)
    tick: TickInput,
    fwd: ActionId,
    side: ActionId,
    run: ActionId,
    pub(crate) look_x: ActionId,
    pub(crate) look_y: ActionId,
    /// the camera's yaw: walking goes where it looks
    pub(crate) yaw: f64,
    pub(crate) walker: Entity,
    phys: PhysicsWorld,
}

/// the walker: forward and sideways by the tick's axes, stopped and slid by the boxes
fn sys_walk(w: &mut World, r: &mut Res, s: &Step) {
    let speed = if r.tick.down(r.run) { RUN } else { WALK };
    let dir = game::camera::walk_dir(r.yaw, r.tick.value(r.fwd), r.tick.value(r.side));
    let Some(mut p) = w.get_mut::<Pose>(r.walker) else { return };
    let mut pos = p.now.pos;
    let mut motion = dir * (speed * s.dt);
    let cap = Shape::capsule(HALF, RADIUS);
    let centre = Vec3::new(0.0, HALF + RADIUS, 0.0);
    for _ in 0..3 {
        if motion.length_sq() < 1e-18 {
            break;
        }
        match r.phys.cast(&cap, &Iso::at(pos + centre), motion, &Filter::ALL) {
            Some((_, hit)) => {
                // to the contact less a millimetre, the rest along the wall (level: no climbing)
                let len = motion.length();
                let go = (hit.fraction * len - 0.001).max(0.0) / len;
                pos += motion * go;
                let rest = motion * (1.0 - go);
                let n = Vec3::new(hit.normal.x, 0.0, hit.normal.z).normalize_or_zero();
                motion = rest - n * rest.dot(n);
            }
            None => {
                pos += motion;
                break;
            }
        }
    }
    if pos != p.now.pos {
        p.now.pos = pos;
    }
}

/// the turning boxes
fn sys_spin(w: &mut World, _: &mut Res, s: &Step) {
    let (poses, spins) = w.pair_mut::<Pose, Spin>();
    for (e, sp) in spins.iter() {
        if let Some(mut p) = poses.get_mut(e) {
            p.now.rot = p.now.rot.mul(Quat::from_axis_angle(Vec3::Y, sp.0 * s.dt)).normalize();
        }
    }
}

/// a box of the scene: centre, half extents, yaw, colour, turning
struct BoxDef {
    at: [f64; 3],
    half: [f64; 3],
    yaw: f64,
    color: usize,
    spin: f64,
}

fn boxes() -> Vec<BoxDef> {
    let mut v = Vec::new();
    // a ring of crates and pillars around the start
    for i in 0..12 {
        let a = i as f64 / 12.0 * core::f64::consts::TAU;
        let (s, c) = physics::math::sincos(a);
        let r = if i % 2 == 0 { 9.0 } else { 13.0 };
        let h = if i % 3 == 0 { 2.5 } else { 0.6 };
        v.push(BoxDef { at: [r * s, h, -r * c], half: [0.6, h, 0.6], yaw: a, color: i % 4, spin: 0.0 });
    }
    // a low wall, steps of crates
    v.push(BoxDef { at: [-4.0, 0.5, -3.0], half: [3.0, 0.5, 0.25], yaw: 0.3, color: 4, spin: 0.0 });
    for k in 0..4 {
        v.push(BoxDef { at: [4.0 + k as f64 * 0.9, 0.2 + k as f64 * 0.2, 2.0], half: [0.45, 0.2 + k as f64 * 0.2, 0.8], yaw: 0.0, color: 1 + k % 3, spin: 0.0 });
    }
    // turning boxes above the walkers' heads
    for k in 0..4 {
        let a = k as f64 * core::f64::consts::FRAC_PI_2 + 0.4;
        let (s, c) = physics::math::sincos(a);
        v.push(BoxDef { at: [5.0 * s, 3.2, -5.0 * c], half: [0.5, 0.5, 0.5], yaw: a, color: k, spin: 0.6 + 0.3 * k as f64 });
    }
    v
}

/// where the beacons stand (a model: ModelRef)
const BEACONS: [[f64; 3]; 4] = [[-6.0, 0.0, 1.0], [6.5, 0.0, -2.0], [-2.5, 0.0, -9.0], [2.0, 0.0, 9.5]];

/// the demo's world: the ground, the boxes (with their bodies in the physics world), the walker; the beacons when
/// a model is given, a hum on the first turning box when a sound is
pub(crate) fn build(meshes: &[MeshId], materials: &[MaterialId], beacon: Option<&Handle<Model>>, hum: Option<plat::audio::Sound>) -> Game<Res> {
    let mut map = ActionMap::new();
    let fwd = map.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]);
    let side = map.add("strafe", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0)]);
    let run = map.add("run", ActionKind::Button, &[Bind::key(KC_SHIFT_LEFT)]);
    let look_x = map.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]);
    let look_y = map.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY)]);
    let mut phys = PhysicsWorld::new();
    phys.add(Shape::cuboid(Vec3::new(40.0, 0.5, 40.0)), Iso::at(Vec3::new(0.0, -0.5, 0.0)));
    let res = Res { input: Input::new(map), tick: TickInput::default(), fwd, side, run, look_x, look_y, yaw: 0.0, walker: Entity::NONE, phys };
    let mut g = Game::new(res);
    g.systems.add("walk", sys_walk).add("spin", sys_spin);
    let w = &mut g.world;
    let ground = Transform::at(Vec3::new(0.0, -0.05, 0.0)).scaled([40.0, 0.05, 40.0]);
    w.spawn_with((Pose::new(ground), MeshRef { mesh: meshes[0], material: materials[5] }));
    let mut hum = hum;
    for b in boxes() {
        let rot = Quat::from_axis_angle(Vec3::Y, b.yaw);
        let t = Transform::new(Vec3::new(b.at[0], b.at[1], b.at[2]), rot).scaled([b.half[0] as f32, b.half[1] as f32, b.half[2] as f32]);
        let e = w.spawn_with((Pose::new(t), MeshRef { mesh: meshes[0], material: materials[b.color] }));
        if b.spin != 0.0 {
            w.insert(e, Spin(b.spin));
            if let Some(s) = hum.take() {
                w.insert(e, Emitter::new(s).with_volume(0.35).with_attenuation(Attenuation { near: 1.5, far: 30.0, ..Attenuation::DEFAULT }).looping());
            }
        } else {
            g.res.phys.add(Shape::cuboid(Vec3::new(b.half[0], b.half[1], b.half[2])), Iso::new(Vec3::new(b.at[0], b.at[1], b.at[2]), rot));
        }
    }
    if let Some(m) = beacon {
        for (k, at) in BEACONS.iter().enumerate() {
            let t = Transform::new(Vec3::new(at[0], at[1], at[2]), Quat::from_axis_angle(Vec3::Y, k as f64 * 0.7)).scaled([1.2; 3]);
            g.world.spawn_with((Pose::new(t), ModelRef { model: m.clone() }));
            g.res.phys.add(Shape::cuboid(Vec3::new(0.45, 0.1, 0.45)), Iso::at(Vec3::new(at[0], 0.1, at[2])));
        }
    }
    g.res.walker = g.world.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 6.0)),));
    g
}

/// the cube -1 ..= 1, four vertices per face
pub(crate) fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    let (mut p, mut n, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (f, (nn, u, v)) in faces.iter().enumerate() {
        for (a, b) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            p.push([nn[0] + a * u[0] + b * v[0], nn[1] + a * u[1] + b * v[1], nn[2] + a * u[2] + b * v[2]]);
            n.push(*nn);
            uv.push([(a + 1.0) * 0.5, (b + 1.0) * 0.5]);
        }
        let f = f as u32 * 4;
        idx.extend_from_slice(&[f, f + 1, f + 2, f, f + 2, f + 3]);
    }
    (p, n, uv, idx)
}

/// the colours: four for crates, the wall, the ground
pub(crate) const COLORS: [[f32; 3]; 6] = [[0.8, 0.35, 0.2], [0.25, 0.55, 0.85], [0.85, 0.75, 0.3], [0.4, 0.7, 0.35], [0.6, 0.6, 0.62], [0.45, 0.47, 0.42]];

/// the renderer, its scene and the bridge for the demo's world on a device: the meshes and materials as build()
/// numbers them (a fresh renderer counts 0, 1, 2, ..)
pub(crate) struct Gpu3d {
    pub(crate) r: Renderer,
    pub(crate) s: render3d::Scene,
}

impl Gpu3d {
    pub(crate) fn new(dev: &Device, format: gpu::Format) -> gpu::Result<Gpu3d> {
        let mut r = Renderer::new(dev, &RendererDesc::new(format))?;
        let (p, n, uv, idx) = cube();
        let mesh = r.add_mesh(&MeshData { positions: &p, normals: &n, uvs: &uv, indices: &idx })?;
        let mut materials = Vec::new();
        for c in COLORS {
            materials.push(r.add_material(&render3d::Material::color(c[0], c[1], c[2]))?);
        }
        if mesh != MeshId(0) || materials.iter().enumerate().any(|(i, m)| *m != MaterialId(i as u32)) {
            return Err(GpuError::Invalid("gamedemo: a fresh renderer numbered its meshes otherwise"));
        }
        let mut s = render3d::Scene::new(&r)?;
        s.set_sun(&Sun { direction: [0.35, 0.8, 0.45], color: [0.8 * core::f32::consts::PI; 3], ambient: [0.3; 3] });
        s.set_shadows(Some(&Shadows::for_level(dev.level())))?;
        Ok(Gpu3d { r, s })
    }

    /// the world at alpha into the scene for a target of w x h, seen through cam (the models of `assets` put up as
    /// they become ready, their entities drawn) -> transforms and instances written
    pub(crate) fn prepare(&mut self, bridge: &mut Bridge, world: &World, cam: &FirstPerson, alpha: f64, (w, h): (u32, u32), assets: &Assets<Model>, models: &mut Models,
                          changes: &[Change]) -> gpu::Result<u32> {
        self.r.set_size(w, h)?;
        let c = game_render3d::camera(&cam.view(), FOV, w as f64 / h.max(1) as f64, 0.05, f64::INFINITY);
        self.s.set_camera(&c);
        bridge.follow(&mut self.s, c.eye);
        let st = bridge.sync(world, &mut self.s, alpha)?;
        // (a model the renderer refuses shows nothing; a lost device ends the frame)
        if let Err(e @ (GpuError::DeviceLost | GpuError::OutOfMemory)) = models.update(assets, changes, &mut self.r) {
            return Err(e);
        }
        let sm = bridge.sync_models(world, &mut self.s, alpha, models)?;
        self.s.prepare(&mut self.r)?;
        Ok(st.added + st.moved + st.removed + st.switched + sm.added + sm.moved + sm.removed)
    }
}

/// the view's content: the game and what renders it on the view's device
pub(crate) struct View {
    pub(crate) game: Game<Res>,
    pub(crate) cam: FirstPerson,
    alpha: f64,
    g: Option<Gpu3d>,
    bridge: Bridge,
    /// what the last frame's sync did (the status bar)
    written: u32,
    /// (measurements) the probe, the frames drawn, the presses taken: the sky carries them (probe.rs)
    pub(crate) probe: Option<Rc<Probe>>,
    pub(crate) frame_no: u64,
    pub(crate) presses: u32,
    /// (G7b part 2) the models (the beacon), what their updates said since the last frame drawn, them on the GPU
    pub(crate) assets: Assets<Model>,
    pub(crate) changes: Vec<Change>,
    pub(crate) models: Models,
    pub(crate) beacon: Handle<Model>,
}

/// the beacon's model
pub(crate) const BEACON: &str = "app://beacon.gltf";

impl View {
    /// the world, the beacon loading as `loading` says (script runs: Frames, the same frames every run); with a hum
    pub(crate) fn new(loading: Loading, hum: Option<plat::audio::Sound>) -> View {
        // a fresh renderer numbers its meshes and materials 0, 1, 2, ..: the ids attach makes again
        let meshes = [MeshId(0)];
        let materials: Vec<MaterialId> = (0..COLORS.len() as u32).map(MaterialId).collect();
        let mut assets = Assets::new(loading);
        let beacon = assets.load(BEACON);
        let game = build(&meshes, &materials, Some(&beacon), hum);
        View {
            game,
            cam: FirstPerson::new(Vec3::new(0.0, EYE, 6.0)),
            alpha: 0.0,
            g: None,
            bridge: Bridge::new(),
            written: 0,
            probe: None,
            frame_no: 0,
            presses: 0,
            assets,
            changes: Vec::new(),
            models: Models::new(),
            beacon,
        }
    }

    /// once a frame: what the assets did since -> something to draw (a model came or went)
    pub(crate) fn assets_frame(&mut self) -> bool {
        let ch = self.assets.update();
        let any = !ch.is_empty();
        self.changes.extend(ch);
        any
    }

    /// the beacon's state, for the status bar
    pub(crate) fn beacon_text(&self) -> &'static str {
        match self.assets.state(&self.beacon) {
            State::Loading => "lädt",
            State::Ready => "bereit",
            State::Failed => "Fehler",
        }
    }

    /// the sky of the next frame (measurements: the probe's codes)
    pub(crate) fn sky(&mut self) -> [f32; 4] {
        let s = self.sky_next();
        self.drawn();
        s
    }

    /// the sky of the next frame without telling the probe (own.rs tells it once the frame is presented)
    pub(crate) fn sky_next(&mut self) -> [f32; 4] {
        match &self.probe {
            Some(_) => {
                self.frame_no += 1;
                probe::sky_clear(self.presses, self.frame_no)
            }
            None => SKY,
        }
    }

    /// (measurements) the frame of the last sky_next went to the GPU
    pub(crate) fn drawn(&self) {
        if let Some(p) = &self.probe {
            p.drawn(self.frame_no, self.presses);
        }
    }
}

impl GpuContent for View {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        self.g = Some(Gpu3d::new(dev, GPU_VIEW_FORMAT)?);
        self.bridge.reset();
        self.models.reset();
        Ok(())
    }

    fn detach(&mut self) {
        self.g = None;
        self.bridge.reset();
        self.models.reset();
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()> {
        let sky = self.sky();
        let changes = core::mem::take(&mut self.changes);
        if let Some(g) = self.g.as_mut() {
            self.written = g.prepare(&mut self.bridge, &self.game.world, &self.cam, self.alpha, size, &self.assets, &mut self.models, &changes)?;
        }
        let me: &'a Self = self;
        let Some(g) = &me.g else { return Err(GpuError::Invalid("not attached")) };
        g.r.record(e, &g.s, ColorAttachment { clear: sky, ..target }, None)
    }

    fn submitted(&mut self, id: SubmitId) {
        if let Some(g) = &mut self.g {
            g.s.submitted(id);
        }
    }
}

/// what the HUD shows (lib/ui draws it over the view: veils)
pub(crate) struct HudState {
    pub(crate) playing: bool,
    pub(crate) tick: u64,
    pub(crate) fps: f64,
    pub(crate) health: i32,
}

/// the HUD over the view's rectangle r (x, y, w, h): a crosshair, a bar with a number (bottom left), a panel with the
/// tick and the frames a second (top right); drawn on the base layer after the view (lib/ui: veils, gpu_out.rs)
pub(crate) fn hud(ui: &mut Ui, r: [i32; 4], h: &HudState) {
    let fx = |v: i32| v << 8;
    let (cx, cy) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
    if h.playing {
        ui.draw_color(0xF0FF_FFFF);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            ui.draw_line(fx(cx + dx * 5), fx(cy + dy * 5), fx(cx + dx * 14), fx(cy + dy * 14), fx(2));
        }
    }
    let (bx, by) = (r[0] + 24, r[1] + r[3] - 44);
    ui.draw_color(0x8010_1418);
    ui.draw_rrect(bx, by, 200, 16, 5);
    ui.draw_color(0xFF4C_D964);
    ui.draw_rrect(bx, by, 200 * h.health.clamp(0, 100) / 100, 16, 5);
    ui.draw_color(0xA000_0000);
    ui.draw_text(bx + 1, by - 7, &format!("{}", h.health), (26u32 << 16) | FONT_BOLD);
    ui.draw_color(0xFFFF_FFFF);
    ui.draw_text(bx, by - 8, &format!("{}", h.health), (26u32 << 16) | FONT_BOLD);
    let (pw, ph) = (190, 52);
    let (px, py) = (r[0] + r[2] - pw - 20, r[1] + 20);
    ui.draw_color(0xB01A_1E28);
    ui.draw_rrect(px, py, pw, ph, 8);
    ui.draw_color(0xFFE8_ECF0);
    ui.draw_text(px + 12, py + 22, &format!("Takt {}", h.tick), (15u32 << 16) | FONT_REG);
    ui.draw_text(px + 12, py + 42, &format!("{:.0} Bilder/s", h.fps), (15u32 << 16) | FONT_REG);
}

struct Demo {
    view: Rc<RefCell<View>>,
    /// (G7b part 2) the game's sound: None in script runs
    audio: Option<SpatialAudio>,
    ver: u32,
    /// the lock has been held once (the status text: "click to play" or "paused")
    played: bool,
    /// frames a second for the HUD: frames counted since t0
    fps: (u64, u32, f64),
}

impl App for Demo {
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(0, -1);
        ui.box_begin(0, BOX_V, 0);
        ui.next_size(0, -1);
        ui.next_flags(HB_KEYS);
        ui.canvas_label("Spiel");
        let inp = ui.gpu_view(ID_VIEW, &self.view, self.ver).clone();
        let r = ui.last_rect();
        let (playing, feet, yaw, pitch, tick, written, beacon) = {
            let mut guard = self.view.borrow_mut();
            let v = &mut *guard;
            let res = &mut v.game.res;
            res.input.set_lock(ui.pointer_lock_state());
            if inp.ev & EV_FOCUS != 0 {
                while let Some(k) = ui.key_event_take() {
                    res.input.key(k.code, k.down);
                }
            }
            res.input.buttons(inp.pressed, inp.released);
            let (dx, dy) = ui.pointer_motion();
            res.input.motion(dx, dy);
            if res.input.take_lock_request() {
                ui.pointer_lock(true);
            }
            res.input.take_pause();
            let playing = res.input.playing();
            let (lx, ly) = (res.input.motion_of(res.look_x), res.input.motion_of(res.look_y));
            v.cam.look(lx, ly);
            v.game.res.yaw = v.cam.yaw;
            if playing {
                // the ticks due: each takes the input first
                let n = v.game.clock.advance_to(plat::task::now_us().saturating_mul(1000));
                for _ in 0..n {
                    v.game.res.tick = v.game.res.input.tick();
                    v.game.step();
                }
                // the next frame through a timer (an animation's way: ui.invalidate in every frame stops after 8)
                ui.timer(TIMER_FRAME, 1);
            } else {
                // paused: the simulation stands, the clock forgets (no catch-up after the pause)
                v.game.clock.reset();
            }
            v.game.res.input.end_frame();
            v.alpha = v.game.alpha();
            let feet = v.game.world.get::<Pose>(v.game.res.walker).map_or(Vec3::ZERO, |p| p.interpolate(v.alpha).pos);
            v.cam.eye = feet + Vec3::new(0.0, EYE, 0.0);
            // the assets (a model that came draws a frame); the sound from the camera, silent in the pause
            let came = v.assets_frame();
            if let Some(sa) = &mut self.audio {
                sa.master = if playing { 1.0 } else { 0.0 };
                sa.set_listener(&v.cam.view());
                let alpha = v.alpha;
                let _ = sa.update(&mut v.game.world, alpha, &mut Speakers);
            }
            if playing || lx != 0.0 || ly != 0.0 || came {
                self.ver = self.ver.wrapping_add(1);
            }
            let deg = 180.0 / core::f64::consts::PI;
            (playing, feet, v.cam.yaw * deg, v.cam.pitch * deg, v.game.world.tick(), v.written, v.beacon_text())
        };
        self.played |= playing;
        // the frames a second (the HUD's panel), counted twice a second
        let now = plat::time_us();
        self.fps.1 += 1;
        if now >= self.fps.0 + 500_000 {
            self.fps.2 = self.fps.1 as f64 * 1e6 / now.saturating_sub(self.fps.0).max(1) as f64;
            (self.fps.0, self.fps.1) = (now, 0);
        }
        hud(ui, r, &HudState { playing, tick, fps: if playing { self.fps.2 } else { 0.0 }, health: 100 - (tick / 60 % 100) as i32 });
        ui.statusbar_begin();
        let hint = if playing {
            "Esc: Pause"
        } else if self.played {
            "Pause – klicken zum Weiterspielen"
        } else {
            "Klicken zum Spielen · WASD gehen, Umschalt rennen, Maus schaut"
        };
        ui.text(TS_CAPTION, hint);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Position {:.2} {:.2} · Blick {:.1} {:.1} · Takt {}", feet.x, feet.z, yaw, pitch, tick));
        // (what the bridge wrote in the view's last frame: in a window every frame, in a headless run when
        // the frame is rendered for a shot)
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("geschrieben {} · Modell {} · Faktor {} · Spiel-Demo {} · JFLib {}", written, beacon, platform_motion_factor(), APP_VERSION, ui::VERSION));
        ui.statusbar_end();
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            UIC_TIMER if arg == TIMER_FRAME => ui.invalidate(),
            _ => return false,
        }
        true
    }
}

/// the value after `name` on the command line
pub(crate) fn arg(name: &str) -> Option<String> {
    let a = plat::args();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned()
}

pub(crate) fn has(name: &str) -> bool {
    plat::args().iter().any(|x| x == name)
}

fn start() -> i32 {
    match arg("--bench").as_deref() {
        Some("view") => return bench::view(),
        Some("own") => return own::run(),
        Some(other) => {
            plat::write_stderr(format!("gamedemo: --bench view|own, not {other}\n").as_bytes());
            return 2;
        }
        None if bench::web_bench() => return bench::view(),
        None => {}
    }
    // script runs: the beacon decoded in the first frame (the same frames every run), no sound
    let script = has("--script");
    let loading = if script { Loading::Frames { budget_us: 1_000_000 } } else { Loading::default_for_target() };
    let hum = (!script).then(|| plat::audio::Sound::tone(110.0, 1200));
    let view = View::new(loading, hum);
    let audio = (!script).then(SpatialAudio::new);
    ui::launch(&DESC, Demo { view: Rc::new(RefCell::new(view)), audio, ver: 0, played: false, fps: (0, 0, 0.0) }, &style::STYLE)
}

ui::main!(start());
