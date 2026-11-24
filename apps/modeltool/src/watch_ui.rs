// watch_ui -- the observation mode's window: `modeltool --watch FILE [--param NAME=VALUE ...]` shows the model in a
// gpu_view and builds it again when the script or one of its modules is saved (watch.rs looks every 300 ms).  The
// same cameras, grid and light as the pictures (render.rs): the three plan views and the three-quarter view, and a
// free view about the model's middle (drag with the left button: turn; the wheel: nearer, farther).  An error in
// the script shows in a bar above the view with its place; the model of the last build that ran stays.  The
// host's warnings (the script went on) show in a bar of their own below it, each with its place.
//
//   F5 builds again by hand, Ctrl+Q ends.  Headless with --script (tests/watch.uis, tests/watch_fehler.uis).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use gpu::{ColorAttachment, CommandEncoder, Device, GpuError, SubmitId};
use render3d::{Instance, InstanceId, MaterialId, MeshId, Renderer, RendererDesc, Scene, Transform};
use ui::*;

use crate::host::Model;
use crate::render::{self, Look};
use crate::watch::Watched;

const ID_VIEW: u32 = 100;
const ID_LOOKS: [u32; 5] = [101, 102, 103, 104, 105];
const ID_GRID: u32 = 110;
const ID_REBUILD: u32 = 111;
// (an info bar's close button takes the next id)
const ID_ERR: u32 = 120;
const ID_FINDINGS: u32 = 130;
const ID_WARN: u32 = 140;
const CMD_QUIT: u32 = 1;
const CMD_REBUILD: u32 = 2;
const TIMER_POLL: u32 = 1;
/// how often the files are looked at
const POLL_MS: u32 = 300;

static CMDS: [Command; 2] = [Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0), Command::new(CMD_REBUILD, "&Neu bauen", 0, 0, VK_F5, 0)];

const DESC: AppDesc = AppDesc {
    title: "Modellier-Werkzeug",
    flags: AF_RESIZABLE | AF_GPU_3D,
    w: 1100,
    h: 760,
    min_w: 480,
    min_h: 360,
    cmds: &CMDS,
    ..AppDesc::NONE
};

/// the buttons of the views, and their names in the status bar
const LOOKS: [(&str, &str); 5] = [("Vorn", "vorn"), ("Rechts", "rechts"), ("Oben", "oben"), ("Schräg", "schräg"), ("Frei", "frei")];

/// the view's content: the model and the grid in a render3d scene on the view's device
struct View {
    model: Option<Rc<Model>>,
    /// changes with every new model
    model_gen: u32,
    look: Look,
    grid: bool,
    r: Option<Renderer>,
    s: Option<Scene>,
    /// the model's meshes with their materials (on the renderer), the generation they are of
    parts: Vec<(MeshId, MaterialId)>,
    mats: Vec<MaterialId>,
    uploaded: Option<u32>,
    grid_mats: Vec<MaterialId>,
    grid_meshes: Vec<MeshId>,
    insts: Vec<InstanceId>,
    /// what the scene shows: (model generation, look, grid, width, height)
    built: Option<(u32, Look, bool, u32, u32)>,
}

impl View {
    fn new() -> View {
        View { model: None, model_gen: 0, look: Look::Plan(0), grid: true, r: None, s: None, parts: Vec::new(), mats: Vec::new(), uploaded: None, grid_mats: Vec::new(), grid_meshes: Vec::new(), insts: Vec::new(), built: None }
    }

    /// the scene again: the model's meshes when it changed, the grid and camera for the look and the size
    fn rebuild(&mut self, w: u32, h: u32) -> gpu::Result<()> {
        let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) else { return Ok(()) };
        for id in self.insts.drain(..) {
            s.remove(id)?;
        }
        if self.uploaded != Some(self.model_gen) {
            for (m, _) in self.parts.drain(..) {
                r.remove_mesh(m)?;
            }
            for m in self.mats.drain(..) {
                r.remove_material(m)?;
            }
            if let Some(model) = &self.model {
                for m in &model.materials {
                    self.mats.push(render::material(r, m.color, m.roughness, m.metallic)?);
                }
                for p in &model.parts {
                    if let Some(id) = render::upload(r, &p.mesh)? {
                        self.parts.push((id, self.mats[p.material]));
                    }
                }
            }
            self.uploaded = Some(self.model_gen);
        }
        for m in self.grid_meshes.drain(..) {
            r.remove_mesh(m)?;
        }
        let b = self.model.as_ref().map_or(geom::Aabb { min: geom::Vec3::new(-0.5, 0.0, -0.5), max: geom::Vec3::new(0.5, 1.0, 0.5) }, |m| render::bounds_of(m));
        let st = render::setup(b, &self.look, h.max(1), w.max(1) as f64 / h.max(1) as f64);
        for &(m, mat) in &self.parts {
            self.insts.push(s.add(Instance { mesh: m, material: mat, transform: Transform::at([0.0; 3]) })?);
        }
        if self.grid {
            for (m, mat) in [(&st.grid.0, self.grid_mats[0]), (&st.grid.1, self.grid_mats[1]), (&st.grid.2, self.grid_mats[2])] {
                if let Some(id) = render::upload(r, m)? {
                    self.grid_meshes.push(id);
                    self.insts.push(s.add(Instance { mesh: id, material: mat, transform: Transform::at([0.0; 3]) })?);
                }
            }
        }
        s.set_outline(if st.plan { Some(render::plan_outline(1)) } else { None }.as_ref())?;
        s.set_camera(&st.cam);
        self.built = Some((self.model_gen, self.look, self.grid, w, h));
        Ok(())
    }
}

impl GpuContent for View {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let mut r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
        self.grid_mats = render::GRID_COLORS.iter().map(|&c| render::material(&mut r, c, 0.95, 0.0)).collect::<gpu::Result<_>>()?;
        let mut s = Scene::new(&r)?;
        s.set_sun(&render::SUN);
        (self.r, self.s) = (Some(r), Some(s));
        self.parts.clear();
        self.mats.clear();
        self.grid_meshes.clear();
        self.insts.clear();
        self.uploaded = None;
        self.built = None;
        Ok(())
    }

    fn detach(&mut self) {
        (self.s, self.r) = (None, None);
        self.built = None;
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, (w, h): (u32, u32)) -> gpu::Result<()> {
        if let Some(r) = self.r.as_mut() {
            r.set_size(w, h)?;
        }
        if self.built != Some((self.model_gen, self.look, self.grid, w, h)) {
            self.rebuild(w, h)?;
        }
        if let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) {
            s.prepare(r)?;
        }
        let me: &'a Self = self;
        let (Some(r), Some(s)) = (&me.r, &me.s) else { return Err(GpuError::Invalid("not attached")) };
        r.record(e, s, ColorAttachment { clear: render::CLEAR, ..target }, None)
    }

    fn submitted(&mut self, id: SubmitId) {
        if let Some(s) = &mut self.s {
            s.submitted(id);
        }
    }
}

struct WatchApp {
    w: Watched,
    view: Rc<RefCell<View>>,
    ver: u32,
    /// the view chosen: LOOKS 0..5
    sel: usize,
    yaw: f64,
    pitch: f64,
    zoom: f64,
    grid: i32,
    drag: Option<(i32, i32)>,
    armed: bool,
}

impl WatchApp {
    fn look(&self) -> Look {
        match self.sel {
            0..=2 => Look::Plan(self.sel),
            3 => Look::Iso,
            _ => Look::Orbit { yaw: self.yaw, pitch: self.pitch, zoom: self.zoom },
        }
    }

    /// the view follows the look and the grid
    fn show(&mut self) {
        let mut v = self.view.borrow_mut();
        v.look = self.look();
        v.grid = self.grid != 0;
        self.ver = self.ver.wrapping_add(1);
    }

    /// build again; a model that came out goes to the view
    fn rebuild(&mut self) {
        self.w.build();
        if let Some(m) = &self.w.state.model {
            let mut v = self.view.borrow_mut();
            if !v.model.as_ref().is_some_and(|old| Rc::ptr_eq(old, m)) {
                v.model = Some(m.clone());
                v.model_gen = v.model_gen.wrapping_add(1);
            }
        }
        self.ver = self.ver.wrapping_add(1);
    }

    /// to the free view, starting where the chosen view looks from
    fn free(&mut self) {
        if self.sel != 4 {
            (self.yaw, self.pitch) = match self.sel {
                0 => (0.0, 0.0),
                1 => (90.0, 0.0),
                2 => (0.0, 89.0),
                _ => (35.0, 28.0),
            };
            self.zoom = 1.0;
            self.sel = 4;
        }
    }
}

impl App for WatchApp {
    fn frame(&mut self, ui: &mut Ui) {
        if !self.armed {
            ui.timer(TIMER_POLL, POLL_MS);
            self.armed = true;
        }
        ui.next_size(0, -1);
        ui.box_begin(0, BOX_V, 0);
        // the views, the grid, building again
        ui.box_begin(0, BOX_H, 6);
        for (i, (label, _)) in LOOKS.iter().enumerate() {
            if ui.button(ID_LOOKS[i], label) & EV_ACT != 0 {
                if i == 4 {
                    self.free();
                } else {
                    self.sel = i;
                }
                self.show();
            }
        }
        ui.spacer(12);
        if ui.checkbox(ID_GRID, "Gitter", &mut self.grid) & EV_ACT != 0 {
            self.show();
        }
        ui.spacer(12);
        if ui.button(ID_REBUILD, "Neu bauen") & EV_ACT != 0 {
            self.rebuild();
        }
        ui.box_end();
        // an error of the script: its place and line; the model of the last build stays
        let st = &self.w.state;
        if !st.errors.is_empty() {
            let note = if st.model.is_some() { "zu sehen ist der letzte Bau, der lief" } else { "noch kein Modell" };
            ui.infobar(ID_ERR, IB_ERROR, "Fehler im Skript", note);
            for e in &st.errors {
                for (k, line) in e.iter().enumerate() {
                    // the place and message, the source line, the calls (the caret needs a fixed-width font)
                    if !line.trim_start().starts_with('|') {
                        ui.text(if k == 0 { TS_BODY_STRONG } else { TS_BODY | TX_SECONDARY }, line.trim_end());
                    }
                }
            }
        } else if !st.findings.is_empty() {
            ui.infobar(ID_FINDINGS, IB_WARNING, "Prüfung fehlgeschlagen", &st.findings.join("; "));
        }
        // the host's warnings: the script went on; each with its place and line, as the console shows them
        if st.warning_count > 0 {
            let n = st.warning_count;
            let title = if n == 1 { String::from("1 Warnung im Skript") } else { format!("{} Warnungen im Skript", n) };
            ui.infobar(ID_WARN, IB_WARNING, &title, "das Skript lief weiter");
            for w in &st.warnings {
                for (k, line) in w.iter().enumerate() {
                    if !line.trim_start().starts_with('|') {
                        ui.text(if k == 0 { TS_BODY_STRONG } else { TS_BODY | TX_SECONDARY }, line.trim_end());
                    }
                }
            }
        }
        // the model
        ui.next_size(0, -1);
        ui.canvas_label("Modell");
        let inp = ui.gpu_view(ID_VIEW, &self.view, self.ver).clone();
        let left = 1u32 << BTN_LEFT;
        if inp.pressed & left != 0 {
            self.drag = Some((inp.x, inp.y));
        }
        if inp.btn & left != 0 {
            if let Some((lx, ly)) = self.drag {
                let (dx, dy) = (inp.x - lx, inp.y - ly);
                if dx != 0 || dy != 0 {
                    self.free();
                    self.yaw -= dx as f64 * 0.4;
                    self.pitch = (self.pitch + dy as f64 * 0.4).clamp(-89.0, 89.0);
                    self.show();
                }
            }
            self.drag = Some((inp.x, inp.y));
        } else {
            self.drag = None;
        }
        if inp.wheel_y != 0 {
            self.free();
            let mut f = 1.0;
            let notches = inp.wheel_y / 120;
            for _ in 0..notches.unsigned_abs().max(1) {
                f *= if inp.wheel_y > 0 { 0.9 } else { 1.0 / 0.9 };
            }
            self.zoom = (self.zoom * f).clamp(0.05, 20.0);
            self.show();
        }
        // the state
        let st = &self.w.state;
        ui.statusbar_begin();
        let state = match st.ok {
            _ if !st.errors.is_empty() && st.model.is_none() => format!("{}: Fehler", self.w.file.rsplit('/').next().unwrap_or("")),
            _ if !st.errors.is_empty() => format!("{}: Fehler (gezeigt: der letzte Bau)", st.title),
            Some(true) => format!("{}: ok", st.title),
            Some(false) => format!("{}: Prüfung fehlgeschlagen", st.title),
            None => String::from("noch nicht gebaut"),
        };
        ui.text(TS_CAPTION, &state);
        if st.model.is_some() {
            ui.text(TS_CAPTION | TX_SECONDARY, &st.summary);
        }
        let look = if self.sel == 4 {
            format!("Ansicht: frei (Gier {}°, Nick {}°, Abstand {})", geom::fnum(round1(self.yaw)), geom::fnum(round1(self.pitch)), geom::fnum(round1(self.zoom * 10.0) / 10.0))
        } else {
            format!("Ansicht: {}", LOOKS[self.sel].1)
        };
        ui.text(TS_CAPTION | TX_SECONDARY, &look);
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Bau {}", st.builds));
        ui.text(TS_CAPTION | TX_TERTIARY, &format!("{} ms · {}", geom::fnum(round1(st.ms)), self.w.file));
        ui.statusbar_end();
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            CMD_REBUILD => {
                self.rebuild();
                ui.invalidate();
            }
            UIC_TIMER if arg == TIMER_POLL => {
                if self.w.changed() {
                    self.rebuild();
                    ui.invalidate();
                }
                ui.timer(TIMER_POLL, POLL_MS);
            }
            _ => return false,
        }
        true
    }
}

fn round1(v: f64) -> f64 {
    geom::math::floor(v * 10.0 + 0.5) / 10.0
}

/// open the window on `file` (built once before it shows)
pub fn run(file: &str, params: &[(String, String)]) -> i32 {
    let mut app = WatchApp { w: Watched::new(file, params), view: Rc::new(RefCell::new(View::new())), ver: 0, sel: 0, yaw: 0.0, pitch: 0.0, zoom: 1.0, grid: 1, drag: None, armed: false };
    app.rebuild();
    ui::launch(&DESC, app, &style::STYLE)
}
