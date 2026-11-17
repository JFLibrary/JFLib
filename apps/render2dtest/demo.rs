// demo -- render2d in a view of a window (ui.gpu_view): a drawing to pan by dragging with the middle or right button (a
// finger: any drag) and to zoom with the wheel at the pointer; G6d: the object snap under the pointer (its marker), the
// entity under it highlighted, a click picks (Shift: adds or takes away), a drag with the left button selects by a window
// (to the right) or a crossing (to the left), grips on what is selected, Esc clears.  The connection to render2d lives
// here, in the app (View2d, a GpuContent): lib/ui knows nothing of it, as with render3d (docs/RENDER2D.md "In der
// Oberfläche").  G6e1: the scene is made without a device and lives on when the device goes (attach, detach); without one
// (jfgui_GPU=0, no Vulkan or WebGPU, before the first frame, a lost device) the view paints it through the CPU path
// (Scene::draw_cpu), picks, snaps and highlights as on the GPU.
//
//   render2dtest --demo           the demo (in the browser and on Android the app is this)
//   render2dtest --test window    a real window (once): the demo, a pan and a zoom through the same functions the
//                                 pointer calls, the view in the framebuffer against the same scene offscreen
//   tests/demo.uis                the demo headless: drag and wheel through the script, the view's pixels

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use gpu::{ColorAttachment, CommandEncoder, Device, GpuError, Load, SubmitId};
use render2d::{Camera, Cap, EntityId, Grip, Join, Layer, Region, Renderer, RendererDesc, Scene, SelectMode, Shape, SnapKind, SnapOptions, Space, Style, Unit};
use ui::*;

/// the demo's object snaps: aperture (px) and kinds; a pick's tolerance (px)
pub const APERTURE: f64 = 10.0;
pub const SNAPS: u32 = render2d::SNAP_END | render2d::SNAP_MID | render2d::SNAP_CEN | render2d::SNAP_QUA | render2d::SNAP_INT | render2d::SNAP_NEA | render2d::SNAP_INS;
pub const PICK_PX: f64 = 3.0;

pub const VIEW_ID: u32 = 900;
/// the view's background (sRGB codes)
pub const PAPER: u32 = 0xFFF4_F1EA;
/// the window and where the view lies in it (below a title row)
pub const DESC: AppDesc = AppDesc { title: "render2dtest", flags: AF_RESIZABLE | AF_GPU_3D, w: 720, h: 512, min_w: 200, min_h: 150, ..AppDesc::NONE };
pub const TOP: i32 = 32;

/// the demo's view: the scene (made without a device; on the device lib/ui hands over while there is one, with a Renderer
/// there), and the camera
pub struct View2d {
    pub cam: Camera,
    pub size: (u32, u32),
    pub dev: Option<Device>,
    pub r: Option<Renderer>,
    /// (a RefCell: GpuContent::paint has &self, the CPU path needs the scene mutable: its index, text's decided scale)
    pub s: RefCell<Scene>,
    pub draws: u32,
    pub attaches: u32,
    /// (G6e1) frames painted on the CPU, the last one's time (us)
    pub paints: Cell<u32>,
    pub paint_us: Cell<u64>,
    // ---- G6d
    /// the pointer over the view (px), the snap there, the entity under it, what is selected, a selection box's first corner
    pub pointer: Option<[f64; 2]>,
    pub snap: Option<(SnapKind, [f64; 2])>,
    pub hover: Option<EntityId>,
    pub selected: Vec<EntityId>,
    pub boxa: Option<[f64; 2]>,
    // ---- G6e2: a DXF drawing (render2dtest --dxf PATH)
    /// what it became (its groups: the render2d entities of one DXF entity are picked and highlighted together)
    pub dxf: Option<render2d::dxf::Loaded>,
    /// the box to show once the view knows its size (the drawing's extents)
    pub fit: Option<[f64; 4]>,
    /// the title's text
    pub title: Option<alloc::string::String>,
}

impl View2d {
    pub fn new() -> Rc<RefCell<View2d>> {
        let mut s = Scene::new_cpu();
        // (the drawing's values are all valid: fill refuses none of them)
        let _ = fill(&mut s);
        View2d::of(s)
    }

    /// a view of a scene
    pub fn of(s: Scene) -> Rc<RefCell<View2d>> {
        Rc::new(RefCell::new(View2d { cam: Camera::new([0.0, 0.0], 1.0), size: (0, 0), dev: None, r: None, s: RefCell::new(s), draws: 0, attaches: 0, paints: Cell::new(0), paint_us: Cell::new(0),
                                      pointer: None, snap: None, hover: None, selected: Vec::new(), boxa: None, dxf: None, fit: None, title: None }))
    }

    /// (G6e2) the render2d entities an entity stands for: its DXF entity's, else itself
    pub fn group(&self, id: EntityId) -> Vec<EntityId> {
        match self.dxf.as_ref().and_then(|l| l.group_of(id).map(|g| l.members(g).to_vec())) {
            Some(v) if !v.is_empty() => v,
            _ => alloc::vec![id],
        }
    }

    /// (G6e2) the camera that shows a box in the view's size, with a margin of 4 %
    pub fn fit_to(&mut self, b: [f64; 4]) {
        if self.size.0 == 0 || self.size.1 == 0 {
            return;
        }
        let (w, h) = ((b[2] - b[0]).max(1e-9), (b[3] - b[1]).max(1e-9));
        let scale = (w / self.size.0 as f64).max(h / self.size.1 as f64) * 1.08;
        self.cam = Camera::new([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], scale);
    }

    /// (G6d) the pointer moved (px of the view; None: it left): the snap and the entity under it, their highlight
    pub fn hover_at(&mut self, at: Option<[f64; 2]>) {
        self.pointer = at;
        let (cam, size) = (self.cam, self.size);
        let s = self.s.get_mut();
        let _ = s.set_camera(&cam);
        let (snap, hover) = match at {
            Some(q) if size.0 > 0 => {
                let p = cam.to_drawing(q, size);
                (s.snap(p, &SnapOptions::new(APERTURE, SNAPS)).map(|x| (x.kind, x.point)), s.pick(p, PICK_PX).first().map(|h| h.target.entity))
            }
            _ => (None, None),
        };
        self.snap = snap;
        if hover != self.hover {
            let old = self.hover.map(|h| self.group(h)).unwrap_or_default();
            let new = hover.map(|h| self.group(h)).unwrap_or_default();
            let s = self.s.get_mut();
            for h in old {
                let keep = if self.selected.contains(&h) { render2d::HL_SELECTED } else { 0 };
                let _ = s.set_highlight(h, keep);
            }
            for h in new {
                let b = s.highlight(h) | render2d::HL_HOVER;
                let _ = s.set_highlight(h, b);
            }
            self.hover = hover;
        }
    }

    /// (G6d) a click at a point of the view: the entity under it selected (add: Shift, taken away when it was), nothing
    /// there: the selection cleared (not with Shift)
    pub fn click(&mut self, q: [f64; 2], add: bool) {
        let (cam, size) = (self.cam, self.size);
        let s = self.s.get_mut();
        let _ = s.set_camera(&cam);
        let hit = s.pick(cam.to_drawing(q, size), PICK_PX).first().map(|h| h.target.entity);
        let mut sel = if add { self.selected.clone() } else { Vec::new() };
        if let Some(h) = hit {
            if add && sel.contains(&h) {
                let g = self.group(h);
                sel.retain(|x| !g.contains(x));
            } else if !sel.contains(&h) {
                sel.push(h);
            }
        }
        self.select_set(sel);
    }

    /// (G6d) a selection box from a to b (px of the view): a window to the right, a crossing to the left
    pub fn select_box(&mut self, a: [f64; 2], b: [f64; 2], add: bool) {
        let (cam, size) = (self.cam, self.size);
        let s = self.s.get_mut();
        let _ = s.set_camera(&cam);
        let corners = [cam.to_drawing(a, size), cam.to_drawing([b[0], a[1]], size), cam.to_drawing(b, size), cam.to_drawing([a[0], b[1]], size)];
        let mode = if render2d::box_is_window(a, b) { SelectMode::Window } else { SelectMode::Crossing };
        let got = s.select(Region::Polygon(&corners), mode);
        let mut sel = if add { self.selected.clone() } else { Vec::new() };
        for g in got {
            if !sel.contains(&g) {
                sel.push(g);
            }
        }
        self.select_set(sel);
    }

    /// (G6d) what is selected now: the highlights follow (the hover stays on top)
    pub fn select_set(&mut self, sel: Vec<EntityId>) {
        // (G6e2) a DXF entity's parts together
        let mut all: Vec<EntityId> = Vec::new();
        for id in sel {
            for g in self.group(id) {
                if !all.contains(&g) {
                    all.push(g);
                }
            }
        }
        let sel = all;
        let s = self.s.get_mut();
        for &old in &self.selected {
            if !sel.contains(&old) {
                let b = s.highlight(old) & !render2d::HL_SELECTED;
                let _ = s.set_highlight(old, b);
            }
        }
        for &new in &sel {
            let b = s.highlight(new) | render2d::HL_SELECTED;
            let _ = s.set_highlight(new, b);
        }
        self.selected = sel;
    }

    /// (G6d) the overlay of this frame: the selection box, the selection's grips (hovered under the pointer), the snap's
    /// marker
    fn overlay(&self, s: &mut Scene) {
        let (pointer, boxa, snap) = (self.pointer, self.boxa, self.snap);
        let (cam, size) = (self.cam, self.size);
        let sel = self.selected.clone();
        let mut grips: Vec<([f64; 2], Grip)> = Vec::new();
        for id in sel {
            for g in s.grip_points(id) {
                let at = cam.to_screen(g, size);
                let near = pointer.is_some_and(|p| (p[0] - at[0]).abs() <= 0.5 * render2d::GRIP_PX && (p[1] - at[1]).abs() <= 0.5 * render2d::GRIP_PX);
                grips.push((g, if near { Grip::Hover } else { Grip::Cold }));
            }
        }
        let ov = s.overlay();
        ov.clear();
        if let (Some(a), Some(p)) = (boxa, pointer) {
            ov.selection_box(a, p, Space::Px);
        }
        for (g, state) in grips {
            ov.grip(g, Space::Drawing, state);
        }
        if let (Some((kind, at)), None) = (snap, boxa) {
            ov.marker(kind, at, Space::Drawing);
        }
    }

    /// what a drag does: the drawing follows the pointer
    pub fn drag(&mut self, dx: f64, dy: f64) {
        self.cam.pan(dx, dy);
    }

    /// what the wheel does at a point of the view (px): 120 units a notch, a notch zooms by 2^(1/4)
    pub fn wheel(&mut self, x: f64, y: f64, units: i32) {
        let f = pow2(units as f64 / 480.0);
        self.cam.zoom_at([x, y], f, self.size);
    }
}

/// 2^x for the zoom (no libm)
fn pow2(x: f64) -> f64 {
    let k = if x < 0.0 { (x - 1.0) as i64 } else { x as i64 };
    let r = (x - k as f64) * core::f64::consts::LN_2;
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..20 {
        term *= r / n as f64;
        sum += term;
    }
    sum * f64::from_bits(((k + 1023).clamp(1, 2046) as u64) << 52)
}

fn style(color: u32, width: f32, unit: Unit, cap: Cap, join: Join) -> Style {
    let mut s = Style::new(color);
    s.width = width;
    s.unit = unit;
    s.cap = cap;
    s.join = join;
    s
}

/// the demo's drawing (units: millimetres): a grid, an axis cross, a part with holes, translucent shapes, a spiral
/// whose width grows with the zoom, and a line a kilometre long
pub fn fill(s: &mut Scene) -> gpu::Result<()> {
    let grid = s.add_layer(&Layer::new(0))?;
    let parts = s.add_layer(&Layer::new(1))?;
    let marks = s.add_layer(&Layer::new(2))?;
    let fine = s.add_style(&style(0xFFC8_C4BA, 0.0, Unit::Px, Cap::Butt, Join::Miter))?;
    let coarse = s.add_style(&style(0xFFA8_A398, 1.0, Unit::Px, Cap::Butt, Join::Miter))?;
    for k in -40..=40 {
        let v = k as f64 * 10.0;
        let st = if k % 5 == 0 { coarse } else { fine };
        s.insert(grid, st, &Shape::Line { a: [v, -400.0], b: [v, 400.0] })?;
        s.insert(grid, st, &Shape::Line { a: [-400.0, v], b: [400.0, v] })?;
    }
    // the axes: a red vertical one through x = 0, a blue horizontal one through y = 50 (demo.uis looks at them)
    let red = s.add_style(&style(0xFFD0_2020, 9.0, Unit::Px, Cap::Butt, Join::Miter))?;
    let blue = s.add_style(&style(0xFF20_40C0, 5.0, Unit::Px, Cap::Butt, Join::Miter))?;
    s.insert(marks, red, &Shape::Line { a: [0.0, -400.0], b: [0.0, 400.0] })?;
    s.insert(marks, blue, &Shape::Line { a: [-400.0, 50.0], b: [400.0, 50.0] })?;
    // a part: its outline 0.7 mm wide, holes, a slot with round ends
    let outline = s.add_style(&style(0xFF20_2830, 0.7, Unit::Drawing, Cap::Butt, Join::Miter))?;
    let thin = s.add_style(&style(0xFF20_2830, 0.35, Unit::Drawing, Cap::Round, Join::Round))?;
    s.insert(parts, outline, &Shape::Polyline { points: &[[-150.0, -100.0], [120.0, -100.0], [150.0, -70.0], [150.0, 90.0], [-150.0, 90.0]], closed: true })?;
    for c in [[-110.0, -60.0], [110.0, -60.0], [-110.0, 50.0], [110.0, 50.0]] {
        let circle: Vec<[f64; 2]> = (0..48).map(|k| {
            let (sn, cs) = render2d::math::sincos(k as f64 * core::f64::consts::TAU / 48.0);
            [c[0] + 12.0 * cs, c[1] + 12.0 * sn]
        }).collect();
        s.insert(parts, thin, &Shape::Polyline { points: &circle, closed: true })?;
    }
    let slot = s.add_style(&style(0x9020_6040, 14.0, Unit::Drawing, Cap::Round, Join::Round))?;
    s.insert(parts, slot, &Shape::Line { a: [-40.0, -20.0], b: [40.0, -20.0] })?;
    // translucent zigzags over each other
    let zig = s.add_style(&style(0x80E0_8020, 6.0, Unit::Px, Cap::Square, Join::Miter))?;
    let zag = s.add_style(&style(0x8020_A0E0, 10.0, Unit::Px, Cap::Round, Join::Bevel))?;
    s.insert(parts, zig, &Shape::Polyline { points: &[[-130.0, 20.0], [-90.0, 70.0], [-50.0, 15.0], [-10.0, 75.0], [30.0, 10.0]], closed: false })?;
    s.insert(parts, zag, &Shape::Polyline { points: &[[-130.0, 60.0], [-80.0, 10.0], [-40.0, 65.0], [0.0, 5.0], [40.0, 60.0]], closed: false })?;
    // a spiral, 0.5 mm wide
    let spiral: Vec<[f64; 2]> = (0..400).map(|k| {
        let a = k as f64 * 0.08;
        let (sn, cs) = render2d::math::sincos(a);
        [260.0 + (2.0 + 2.2 * a) * cs, -20.0 + (2.0 + 2.2 * a) * sn]
    }).collect();
    let sp = s.add_style(&style(0xFF30_7040, 0.5, Unit::Drawing, Cap::Round, Join::Round))?;
    s.insert(parts, sp, &Shape::Polyline { points: &spiral, closed: false })?;
    // a line a kilometre long: zoom in far along it
    let long = s.add_style(&style(0xFF70_3090, 2.0, Unit::Px, Cap::Butt, Join::Miter))?;
    s.insert(marks, long, &Shape::Line { a: [-1.0e6, -300.0], b: [1.0e6, -200.0] })?;
    Ok(())
}

impl GpuContent for View2d {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
        // (the scene and what is selected stay: on the device everything goes up with the first frame)
        self.s.get_mut().attach(&r)?;
        self.r = Some(r);
        self.dev = Some(dev.clone());
        self.attaches += 1;
        Ok(())
    }

    fn detach(&mut self) {
        self.s.get_mut().detach();
        self.r = None;
        self.dev = None;
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()> {
        {
            self.size = size;
            self.overlay(&mut self.s.borrow_mut());
            let cam = self.cam;
            let (Some(r), s) = (self.r.as_mut(), self.s.get_mut()) else { return Err(GpuError::Invalid("View2d: not attached")) };
            s.set_camera(&cam)?;
            s.prepare(r, size)?;
        }
        self.draws += 1;
        let me: &'a mut View2d = self;
        let (Some(r), s) = (me.r.as_ref(), me.s.get_mut()) else { return Err(GpuError::Invalid("View2d: not attached")) };
        let s: &'a Scene = s;
        r.record(e, s, ColorAttachment { load: Load::Clear, clear: crate_clear(PAPER), ..target })
    }

    fn submitted(&mut self, id: SubmitId) {
        self.s.get_mut().submitted(id);
    }

    /// without a device: the same scene through the CPU path (paper, the drawing, the highlights and the overlay)
    fn paint(&self, g: &mut Gfx, r: [i32; 4]) {
        g.dc.set_color(PAPER);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        let Ok(mut s) = self.s.try_borrow_mut() else { return };
        if s.set_camera(&self.cam).is_err() {
            return;
        }
        self.overlay(&mut s);
        let t0 = plat::time_us();
        s.draw_cpu(g.dc, g.sf, r);
        self.paint_us.set(plat::time_us().saturating_sub(t0));
        self.paints.set(self.paints.get() + 1);
    }

    fn paints(&self) -> bool {
        true
    }
}

/// a colour as a clear value (sRGB codes)
pub fn crate_clear(c: u32) -> [f32; 4] {
    let k = |s: u32| ((c >> s) & 255) as f32 / 255.0;
    [k(16), k(8), k(0), 1.0]
}

/// the demo's window
pub struct Demo {
    pub view: Rc<RefCell<View2d>>,
    pub ver: u32,
    /// the pointer at the last drag step (a pan)
    last: Option<(i32, i32)>,
    /// (G6d) where the left button went down, and whether it has moved since (a selection box)
    press: Option<[f64; 2]>,
    boxing: bool,
    /// (the window test) its steps
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    test: Option<WindowTest>,
}

impl Demo {
    pub fn new() -> Demo {
        Demo::with(View2d::new())
    }

    /// (G6e2) the demo's window on another view (a DXF drawing)
    pub fn with(view: Rc<RefCell<View2d>>) -> Demo {
        Demo {
            view,
            ver: 1,
            last: None,
            press: None,
            boxing: false,
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            test: None,
        }
    }
}

pub fn view_rect(w: i32, h: i32) -> [i32; 4] {
    [0, TOP, w, (h - TOP).max(16)]
}

impl App for Demo {
    fn frame(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.fb().w, ui.fb().h);
        ui.draw_color(0xFF26_2A30);
        ui.draw_rect(0, 0, w, TOP);
        ui.draw_color(0xFFE8_ECF0);
        let c = self.view.borrow().cam;
        let (nsel, snap) = {
            let v = self.view.borrow();
            (v.selected.len(), v.snap.map(|s| s.0))
        };
        let title = self.view.borrow().title.clone();
        let line = match title {
            Some(t) => format!("{t} (1 px = {:.3e}; {nsel} gewählt{})", c.scale, snap.map(|k| format!(", Fang {k:?}")).unwrap_or_default()),
            None => format!("render2d: Mitte/rechts ziehen schiebt, Rad zoomt, Klick wählt, links ziehen: Fenster/Kreuzen (1 px = {:.3e} mm; {nsel} gewählt{})", c.scale,
                            snap.map(|k| format!(", Fang {k:?}")).unwrap_or_default()),
        };
        ui.draw_text(12, 22, &line, (13u32 << 16) | FONT_REG);
        let r = view_rect(w, h);
        // (the view's size for the pointer's mapping: also without a device, where no frame tells it)
        {
            let mut v = self.view.borrow_mut();
            v.size = (r[2].max(0) as u32, r[3].max(0) as u32);
            // (G6e2) a DXF drawing shown whole once the size is known
            if let (Some(b), true) = (v.fit, v.size.0 > 0 && v.size.1 > 0) {
                v.fit = None;
                v.fit_to(b);
                self.ver += 1;
            }
        }
        ui.next_rect_px(r[0], r[1], r[2], r[3]);
        let inp = ui.gpu_view(VIEW_ID, &self.view, self.ver).clone();
        // a drag with the middle or right button (a finger: any): the drawing follows the pointer; the wheel zooms at the
        // pointer (the view took this pass's version: another pass shows the new one, and the title with it)
        let ver = self.ver;
        let touch = inp.ptype == PTR_TOUCH;
        let pan = inp.btn & ((1 << BTN_MIDDLE) | (1 << BTN_RIGHT)) != 0 || (touch && inp.btn & (1 << BTN_LEFT) != 0);
        if pan {
            if let Some((x, y)) = self.last {
                if (inp.x, inp.y) != (x, y) {
                    self.view.borrow_mut().drag((inp.x - x) as f64, (inp.y - y) as f64);
                    self.ver += 1;
                }
            }
            self.last = Some((inp.x, inp.y));
        } else {
            self.last = None;
        }
        // (G6d) the left button: a click picks, a drag selects by a box; the pointer's snap and hover
        let at = [inp.x as f64 + 0.5, inp.y as f64 + 0.5];
        let over = inp.ev & EV_HOVER != 0 || inp.btn != 0;
        let add = inp.mods & MOD_SHIFT != 0;
        if !touch {
            if inp.pressed & (1 << BTN_LEFT) != 0 {
                self.press = Some(at);
                self.boxing = false;
            }
            if let (Some(p), true) = (self.press, inp.btn & (1 << BTN_LEFT) != 0) {
                if (at[0] - p[0]).abs() > 3.0 || (at[1] - p[1]).abs() > 3.0 {
                    self.boxing = true;
                }
                let mut v = self.view.borrow_mut();
                let want = if self.boxing { Some(p) } else { None };
                if v.boxa != want {
                    v.boxa = want;
                    self.ver += 1;
                }
            }
            if inp.released & (1 << BTN_LEFT) != 0 {
                if let Some(p) = self.press.take() {
                    let mut v = self.view.borrow_mut();
                    if self.boxing {
                        v.select_box(p, at, add);
                    } else {
                        v.click(at, add);
                    }
                    v.boxa = None;
                    self.boxing = false;
                    self.ver += 1;
                }
            }
        } else if inp.released & (1 << BTN_LEFT) != 0 && (inp.x - inp.press_x).abs() <= 3 && (inp.y - inp.press_y).abs() <= 3 {
            self.view.borrow_mut().click(at, false);
            self.ver += 1;
        }
        {
            let mut v = self.view.borrow_mut();
            let want = if over { Some(at) } else { None };
            if v.pointer != want {
                let before = (v.snap, v.hover);
                v.hover_at(want);
                if (v.snap, v.hover) != before || v.boxa.is_some() || !v.selected.is_empty() {
                    self.ver += 1;
                }
            }
        }
        if inp.wheel_y != 0 {
            self.view.borrow_mut().wheel(inp.x as f64 + 0.5, inp.y as f64 + 0.5, inp.wheel_y);
            self.ver += 1;
        }
        if self.ver != ver {
            ui.repass();
        }
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        if self.test.is_some() {
            self.test_step(ui);
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id == UIC_TIMER {
            ui.invalidate();
            return true;
        }
        // (G6d) Esc: nothing selected
        if id == UIC_ESCAPE {
            self.view.borrow_mut().select_set(Vec::new());
            self.ver += 1;
            ui.invalidate();
            return true;
        }
        false
    }
}

/// the demo: a window, or (in a browser, on Android) the app itself
pub fn launch() -> i32 {
    ui::launch(&DESC, Demo::new(), &style::STYLE)
}

// ---------------------------------------------------------------- the window test

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
#[derive(Default)]
struct WindowTest {
    pc: u32,
    since: u64,
    fails: u32,
    done: bool,
    /// the offscreen image before the pan
    before: Vec<u8>,
}

#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), allow(dead_code))]
fn say(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
impl Demo {
    fn ok(&mut self, cond: bool, what: &str) {
        if let Some(t) = self.test.as_mut() {
            if !cond {
                t.fails += 1;
            }
        }
        say(&format!("{} {what}", if cond { "  ok  " } else { "FAIL" }));
    }

    /// the view's pixels in the framebuffer (rows of B, G, R, x)
    fn fb_view(ui: &mut Ui, r: [i32; 4]) -> Vec<u8> {
        ui.gpu_settle();
        let fb = ui.fb();
        let mut v = Vec::with_capacity((r[2] * r[3] * 4) as usize);
        for y in r[1]..r[1] + r[3] {
            let at = y as usize * fb.stride as usize + r[0] as usize * 4;
            v.extend_from_slice(&fb.px[at..at + r[2] as usize * 4]);
        }
        v
    }

    /// the view's scene offscreen on its device with its renderer: what render2d draws without lib/ui
    fn offscreen(&self) -> Option<Vec<u8>> {
        let mut v = self.view.borrow_mut();
        let (w, h) = v.size;
        let cam = v.cam;
        let View2d { dev, r, s, .. } = &mut *v;
        let (dev, r, s) = (dev.clone()?, r.as_mut()?, s.get_mut());
        s.set_camera(&cam).ok()?;
        let mut off = crate::gallery::Off::new(&dev, w, h).ok()?;
        off.bg = PAPER;
        let id = off.submit(r, s).ok()?;
        if !dev.wait(id, 10_000_000_000).ok()? {
            return None;
        }
        off.read().ok()
    }

    /// the view in the framebuffer against offscreen -> the offscreen image
    fn check_view(&mut self, ui: &mut Ui, what: &str) -> Vec<u8> {
        let r = view_rect(ui.fb().w, ui.fb().h);
        let got = Demo::fb_view(ui, r);
        let Some(want) = self.offscreen() else {
            self.ok(false, &format!("{what}: no offscreen image (the view has no device?)"));
            return Vec::new();
        };
        let (mut n, mut m) = (0u32, 0u8);
        if got.len() != want.len() {
            n = u32::MAX;
        } else {
            for (p, q) in got.chunks_exact(4).zip(want.chunks_exact(4)) {
                if p[..3] != q[..3] {
                    n += 1;
                    for k in 0..3 {
                        m = m.max(p[k].abs_diff(q[k]));
                    }
                }
            }
        }
        let size = self.view.borrow().size;
        self.ok(n == 0, &format!("{what}: the view in the framebuffer ({} x {}) against render2d offscreen: {n} pixels differ (largest {m})", size.0, size.1));
        want
    }

    fn test_step(&mut self, ui: &mut Ui) {
        let now = plat::time_us();
        let (pc, since) = match &self.test {
            Some(t) => (t.pc, t.since),
            None => return,
        };
        let after = |ms: u64| now >= since + ms * 1000;
        let next = |d: &mut Demo| {
            if let Some(t) = d.test.as_mut() {
                t.pc += 1;
                t.since = plat::time_us();
            }
        };
        ui.timer(1, 30);
        match pc {
            0 => {
                let i = ui.gpu_view_info(0, VIEW_ID).unwrap_or_default();
                if i.device && i.frames >= 1 && after(300) {
                    say(&format!("  view {:?} on {:?}, output {}", i.size, i.level, i.output));
                    self.ok(i.device && i.attaches == 1, &format!("the view has a device ({} attach)", i.attaches));
                    let img = self.check_view(ui, "the first frame");
                    let paper = [(PAPER & 255) as u8, (PAPER >> 8) as u8, (PAPER >> 16) as u8];
                    let ink = img.chunks_exact(4).filter(|p| p[..3] != paper).count();
                    self.ok(ink > 1000, &format!("the drawing is there: {ink} pixels not paper"));
                    if let Some(t) = self.test.as_mut() {
                        t.before = img;
                    }
                    next(self);
                } else if after(15_000) {
                    self.ok(false, &format!("no frame of the view within 15 s ({:?})", i.error));
                    if let Some(t) = self.test.as_mut() {
                        t.done = true;
                    }
                    ui.quit();
                }
            }
            1 => {
                // a pan as the pointer gives it: 40 px right, 25 px down
                let p = [12.0, 34.0];
                let (cam0, size) = (self.view.borrow().cam, self.view.borrow().size);
                self.view.borrow_mut().drag(40.0, 25.0);
                let cam1 = self.view.borrow().cam;
                let (a, b) = (cam0.to_screen(p, size), cam1.to_screen(p, size));
                self.ok((b[0] - a[0] - 40.0).abs() < 1e-9 && (b[1] - a[1] - 25.0).abs() < 1e-9, &format!("a drag of (40, 25) px moves the drawing by ({:.6}, {:.6})", b[0] - a[0], b[1] - a[1]));
                self.ver += 1;
                next(self);
            }
            2 => {
                if after(300) {
                    let img = self.check_view(ui, "after the pan");
                    let before = self.test.as_ref().map_or(Vec::new(), |t| t.before.clone());
                    // the image moved: 40 px right and 25 px down, the same pixels where both show the drawing
                    let size = self.view.borrow().size;
                    let (w, h) = (size.0 as usize, size.1 as usize);
                    let (mut same, mut all) = (0usize, 0usize);
                    if img.len() == before.len() && !img.is_empty() {
                        for y in 30..h - 30 {
                            for x in 50..w - 50 {
                                let (p, q) = ((y * w + x) * 4, ((y - 25) * w + x - 40) * 4);
                                all += 1;
                                same += (img[p..p + 3] == before[q..q + 3]) as usize;
                            }
                        }
                    }
                    self.ok(all > 0 && same * 100 >= all * 99, &format!("the image moved with the drag: {same} of {all} pixels as before, 40 px right and 25 px down"));
                    next(self);
                }
            }
            3 => {
                // two notches of the wheel at a point of the view
                let q = [301.5, 187.5];
                let (cam0, size) = (self.view.borrow().cam, self.view.borrow().size);
                let p = cam0.to_drawing(q, size);
                self.view.borrow_mut().wheel(q[0], q[1], 240);
                let cam1 = self.view.borrow().cam;
                let s = cam1.to_screen(p, size);
                self.ok((s[0] - q[0]).abs() < 1e-6 && (s[1] - q[1]).abs() < 1e-6 && (cam0.scale / cam1.scale - core::f64::consts::SQRT_2).abs() < 1e-9,
                        &format!("the wheel zooms by {:.4} about the pointer: the point under it stays ({:.2e} px away)", cam0.scale / cam1.scale, ((s[0] - q[0]).abs()).max((s[1] - q[1]).abs())));
                self.ver += 1;
                next(self);
            }
            4 => {
                if after(300) {
                    let _ = self.check_view(ui, "after the zoom");
                    let i = ui.gpu_view_info(0, VIEW_ID).unwrap_or_default();
                    say(&format!("  {} frames of the view, {} read-backs, output {}", i.frames, i.reads, i.output));
                    if let Some(t) = self.test.as_mut() {
                        t.done = true;
                    }
                    ui.quit();
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
/// render2dtest --test window: the demo in a real window, shown without activation, no real input
pub fn window_test(validate: bool) -> i32 {
    plat::setenv("jfgui_STATE", Some("0"));
    plat::setenv("jfgui_TEST_NOACTIVATE", Some("1"));
    if validate {
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    let mut d = Demo::new();
    d.test = Some(WindowTest { since: plat::time_us(), ..Default::default() });
    let code = ui::run_window(&DESC, &mut d, &style::STYLE);
    if code != 0 {
        return code;
    }
    let t = d.test.take().unwrap_or_default();
    if validate {
        let (e, w) = gpu::validation_messages();
        say(&format!("{} validation layer: {e} errors, {w} warnings", if e == 0 { "  ok  " } else { "FAIL" }));
        if e != 0 {
            return 1;
        }
    }
    if !t.done {
        say("FAIL the test did not end");
        return 1;
    }
    if t.fails > 0 {
        say(&format!("{} check(s) failed", t.fails));
        return 1;
    }
    say("all checks passed");
    0
}
