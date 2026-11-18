// gamedemo -- (G7b) a first-person demo of the game core: a ground, boxes, a few turning above them; WASD
// by where the keys sit, the mouse looks while the pointer is locked
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
// follows the camera), render3d in a ui.gpu_view with AF_GPU_3D.  The world lives in the view's content
// (View): the app's frame feeds the input and runs the ticks, the content's draw syncs the world into
// the scene at the frame's alpha.
//
//   python build_rs.py run gamedemo                  (Linux: --target linux; macOS on a Mac;
//   python build_rs.py test gamedemo                  the browser: build with --target web)
//   build/win/gamedemo.exe --script apps/gamedemo/tests/shot.uis
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use game::input::platform_motion_factor;
use game::{ActionId, ActionKind, ActionMap, Bind, Entity, FirstPerson, Game, Input, Pose, Quat, Source, Step, TickInput, Transform, Vec3, World};
use game_render3d::{Bridge, MeshRef};
use gpu::{ColorAttachment, CommandEncoder, Device, GpuError, SubmitId};
use physics::{Filter, Iso, Shape, World as PhysicsWorld};
use plat::{KC_A, KC_D, KC_S, KC_SHIFT_LEFT, KC_W};
use render3d::{MaterialId, MeshData, MeshId, Renderer, RendererDesc, Shadows, Sun};
use ui::*;

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
const EYE: f64 = 1.7;
const HALF: f64 = 0.6;
const RADIUS: f64 = 0.3;
const WALK: f64 = 4.5;
const RUN: f64 = 8.0;
/// the vertical field of view (radians)
const FOV: f64 = 1.15;

/// a box turning about its y axis (radians a second)
#[derive(Clone, Copy)]
struct Spin(f64);

/// what the systems use besides the world
struct Res {
    input: Input,
    /// the input of the tick (taken before each step)
    tick: TickInput,
    fwd: ActionId,
    side: ActionId,
    run: ActionId,
    look_x: ActionId,
    look_y: ActionId,
    /// the camera's yaw: walking goes where it looks
    yaw: f64,
    walker: Entity,
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

/// the demo's world: the ground, the boxes (with their bodies in the physics world), the walker
fn build(meshes: &[MeshId], materials: &[MaterialId]) -> Game<Res> {
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
    for b in boxes() {
        let rot = Quat::from_axis_angle(Vec3::Y, b.yaw);
        let t = Transform::new(Vec3::new(b.at[0], b.at[1], b.at[2]), rot).scaled([b.half[0] as f32, b.half[1] as f32, b.half[2] as f32]);
        let e = w.spawn_with((Pose::new(t), MeshRef { mesh: meshes[0], material: materials[b.color] }));
        if b.spin != 0.0 {
            w.insert(e, Spin(b.spin));
        } else {
            g.res.phys.add(Shape::cuboid(Vec3::new(b.half[0], b.half[1], b.half[2])), Iso::new(Vec3::new(b.at[0], b.at[1], b.at[2]), rot));
        }
    }
    g.res.walker = g.world.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 6.0)),));
    g
}

/// the cube -1 ..= 1, four vertices per face
fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
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
const COLORS: [[f32; 3]; 6] = [[0.8, 0.35, 0.2], [0.25, 0.55, 0.85], [0.85, 0.75, 0.3], [0.4, 0.7, 0.35], [0.6, 0.6, 0.62], [0.45, 0.47, 0.42]];

/// the view's content: the game and what renders it on the view's device
struct View {
    game: Game<Res>,
    cam: FirstPerson,
    alpha: f64,
    r: Option<Renderer>,
    s: Option<render3d::Scene>,
    bridge: Bridge,
    /// what the last frame's sync did (the status bar)
    written: u32,
}

impl View {
    fn new() -> View {
        // a fresh renderer numbers its meshes and materials 0, 1, 2, ..: the ids attach makes again
        let meshes = [MeshId(0)];
        let materials: Vec<MaterialId> = (0..COLORS.len() as u32).map(MaterialId).collect();
        let game = build(&meshes, &materials);
        View { game, cam: FirstPerson::new(Vec3::new(0.0, EYE, 6.0)), alpha: 0.0, r: None, s: None, bridge: Bridge::new(), written: 0 }
    }
}

impl GpuContent for View {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let mut r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
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
        self.bridge.reset();
        (self.r, self.s) = (Some(r), Some(s));
        Ok(())
    }

    fn detach(&mut self) {
        (self.s, self.r) = (None, None);
        self.bridge.reset();
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, (w, h): (u32, u32)) -> gpu::Result<()> {
        if let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) {
            r.set_size(w, h)?;
            let cam = game_render3d::camera(&self.cam.view(), FOV, w as f64 / h.max(1) as f64, 0.05, f64::INFINITY);
            s.set_camera(&cam);
            self.bridge.follow(s, cam.eye);
            let st = self.bridge.sync(&self.game.world, s, self.alpha)?;
            self.written = st.added + st.moved + st.removed + st.switched;
            s.prepare(r)?;
        }
        let me: &'a Self = self;
        let (Some(r), Some(s)) = (&me.r, &me.s) else { return Err(GpuError::Invalid("not attached")) };
        r.record(e, s, ColorAttachment { clear: [0.62, 0.76, 0.92, 1.0], ..target }, None)
    }

    fn submitted(&mut self, id: SubmitId) {
        if let Some(s) = &mut self.s {
            s.submitted(id);
        }
    }
}

struct Demo {
    view: Rc<RefCell<View>>,
    ver: u32,
    /// the lock has been held once (the status text: "click to play" or "paused")
    played: bool,
}

impl App for Demo {
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(0, -1);
        ui.box_begin(0, BOX_V, 0);
        ui.next_size(0, -1);
        ui.next_flags(HB_KEYS);
        ui.canvas_label("Spiel");
        let inp = ui.gpu_view(ID_VIEW, &self.view, self.ver).clone();
        let (playing, feet, yaw, pitch, tick, written) = {
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
            if playing || lx != 0.0 || ly != 0.0 {
                self.ver = self.ver.wrapping_add(1);
            }
            let deg = 180.0 / core::f64::consts::PI;
            (playing, feet, v.cam.yaw * deg, v.cam.pitch * deg, v.game.world.tick(), v.written)
        };
        self.played |= playing;
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
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("geschrieben {} · Faktor {} · Spiel-Demo {} · JFLib {}", written, platform_motion_factor(), APP_VERSION, ui::VERSION));
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

ui::main!(ui::launch(&DESC, Demo { view: Rc::new(RefCell::new(View::new())), ver: 0, played: false }, &style::STYLE));
