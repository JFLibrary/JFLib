// overlay -- an immediate-mode layer over the scene, made anew every frame (G6d, decision 24; docs/RENDER2D.md
// "Überlagerung"): rubber bands, window and crossing boxes, grips, snap markers.  A child of scene.rs.
//
// The app fills it between frames (Scene::overlay: clear, then lines, boxes, grips, markers in drawing units or in pixels);
// prepare maps everything into the target's pixels on the CPU in f64 (a drawing unit through the camera: at 1e7 and 1e-6
// units per pixel well within 1/1000 px), makes records (scene.rs records(): the same 64 bytes as every record), the
// styles, the visible list, the fills' edges in pixels (what count writes for the scene), all into the frame's own upload
// buffer, and Renderer::record copies them into the overlay's buffers under the barriers that cover the scene's uploads and
// draws them with fs_fill (strokes, arcs, dashes and fills: anti-aliased as everything else) after the scene's layers in the
// same render pass.  An empty overlay costs a test per frame: nothing goes up, nothing is drawn.  No culling: what lies
// beside the target the vertex shader clips away.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use super::{records, upload, Camera, Cap, FillG, FlatState, Geom, Join, Scene, SnapKind, Style, Unit, K_EDGE, RECORD_WORDS, STYLE_WORDS};
use crate::curve::{self, Curve};
use crate::fill;
use crate::{RECORD_BYTES, STYLE_BYTES, VIEW_BYTES};

/// what overlay coordinates are: drawing units (through the camera at prepare) or pixels of the target (x right, y down
/// from its top left corner)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    Drawing,
    Px,
}

/// a grip's state: AutoCAD's cold (unselected), hover, hot (selected)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grip {
    Cold,
    Hover,
    Hot,
}

/// a grip's side (px, odd: whole pixels around the pixel it sits on) and colours (cold, hover, hot, its outline)
pub const GRIP_PX: f64 = 9.0;
pub const GRIP_COLD: u32 = 0xFF1E_64FF;
pub const GRIP_HOVER: u32 = 0xFFFF_8C1A;
pub const GRIP_HOT: u32 = 0xFFE0_2020;
pub const GRIP_EDGE: u32 = 0xFF1A_1A1A;
/// a snap marker's size (px), its lines' width (px) and colour
pub const MARKER_PX: f64 = 12.0;
pub const MARKER_WIDTH: f32 = 2.0;
pub const MARKER_COLOR: u32 = 0xFFD8_9000;
/// AutoCAD's window box (solid, blue tint) and crossing box (dashed, green tint): outline, fill
pub const WINDOW_EDGE: u32 = 0xFF20_60FF;
pub const WINDOW_FILL: u32 = 0x3020_60FF;
pub const CROSSING_EDGE: u32 = 0xFF20_A040;
pub const CROSSING_FILL: u32 = 0x3020_C040;

#[derive(Clone)]
pub(super) enum Item {
    Line([f64; 2], [f64; 2], Space, Style),
    Poly(Vec<[f64; 2]>, bool, Space, Style),
    Arc([f64; 2], f64, f64, f64, Space, Style),
    Fill(Vec<[f64; 2]>, Space, u32),
    Box([f64; 2], [f64; 2], Space),
    Grip([f64; 2], Space, Grip),
    Marker(SnapKind, [f64; 2], Space),
}

/// the overlay of a scene: made anew every frame (Scene::overlay).  What it holds at prepare is drawn over the scene
#[derive(Clone, Default)]
pub struct Overlay {
    items: Vec<Item>,
}

/// what the last prepare made of the overlay (Scene::overlay_info).  It may grow
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OverlayInfo {
    pub items: u32,
    /// its records and those drawn, its styles; the bytes it sent
    pub records: u32,
    pub drawn: u32,
    pub styles: u32,
    pub bytes: u64,
}

impl Overlay {
    /// nothing in it
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// (the CPU path) what it holds
    pub(super) fn items(&self) -> &[Item] {
        &self.items
    }

    /// a line (a rubber band) in the style's look (widths and dashes in pixels or drawing units)
    pub fn line(&mut self, a: [f64; 2], b: [f64; 2], space: Space, style: &Style) {
        self.items.push(Item::Line(a, b, space, *style));
    }

    /// a polyline
    pub fn polyline(&mut self, points: &[[f64; 2]], closed: bool, space: Space, style: &Style) {
        if !points.is_empty() {
            self.items.push(Item::Poly(points.to_vec(), closed, space, *style));
        }
    }

    /// an arc (angles counter-clockwise from the x axis, in the drawing's sense: y up in drawing units, y down in pixels
    /// turns them clockwise on the screen)
    pub fn arc(&mut self, center: [f64; 2], radius: f64, start: f64, sweep: f64, space: Space, style: &Style) {
        if radius > 0.0 && sweep != 0.0 {
            self.items.push(Item::Arc(center, radius, start, sweep, space, *style));
        }
    }

    pub fn circle(&mut self, center: [f64; 2], radius: f64, space: Space, style: &Style) {
        self.arc(center, radius, 0.0, core::f64::consts::TAU, space, style);
    }

    /// a rectangle's outline (axis-aligned in its space)
    pub fn rect(&mut self, a: [f64; 2], b: [f64; 2], space: Space, style: &Style) {
        self.items.push(Item::Poly(vec![a, [b[0], a[1]], b, [a[0], b[1]]], true, space, *style));
    }

    /// a filled polygon (even-odd) of a colour (0xAARRGGBB)
    pub fn fill(&mut self, points: &[[f64; 2]], space: Space, color: u32) {
        if points.len() >= 3 {
            self.items.push(Item::Fill(points.to_vec(), space, color));
        }
    }

    pub fn fill_rect(&mut self, a: [f64; 2], b: [f64; 2], space: Space, color: u32) {
        self.items.push(Item::Fill(vec![a, [b[0], a[1]], b, [a[0], b[1]]], space, color));
    }

    /// AutoCAD's selection box from the first corner to the second, axis-aligned on the screen: a window (the second
    /// corner right of the first on the screen: solid outline, blue tint) or a crossing (left: dashed, green tint)
    pub fn selection_box(&mut self, first: [f64; 2], second: [f64; 2], space: Space) {
        self.items.push(Item::Box(first, second, space));
    }

    /// a grip: a square of GRIP_PX pixels on the pixel the point lies on
    pub fn grip(&mut self, at: [f64; 2], space: Space, state: Grip) {
        self.items.push(Item::Grip(at, space, state));
    }

    /// a snap marker: AutoCAD's symbol of the kind (square endpoint, triangle midpoint, circle centre, diamond quadrant, X
    /// intersection, the right angle of perpendicular, the circle and line of tangent, the hourglass of nearest, two squares
    /// of insertion), MARKER_PX pixels, on the pixel the point lies on
    pub fn marker(&mut self, kind: SnapKind, at: [f64; 2], space: Space) {
        self.items.push(Item::Marker(kind, at, space));
    }
}

/// a selection box is a window: its second corner (pixels) lies right of its first
pub fn box_is_window(first_px: [f64; 2], second_px: [f64; 2]) -> bool {
    second_px[0] >= first_px[0]
}

fn pen(color: u32, width: f32) -> Style {
    let mut s = Style::new(color);
    s.width = width;
    s
}

/// the overlay made into pixels: geometry in the overlay's own frame (target pixels with y up: (x, -y)) and styles in pixels
pub(super) struct Built {
    pub(super) geoms: Vec<(Geom, u32)>,
    pub(super) styles: Vec<Style>,
}

impl Built {
    fn style(&mut self, s: Style) -> u32 {
        match self.styles.iter().position(|x| *x == s) {
            Some(k) => k as u32,
            None => {
                self.styles.push(s);
                self.styles.len() as u32 - 1
            }
        }
    }
}

/// a style with its widths and dash lengths in drawing units turned into pixels
fn in_px(st: &Style, scale: f64) -> Style {
    let mut s = *st;
    if s.unit == Unit::Drawing {
        s.width = (s.width as f64 / scale) as f32;
        s.unit = Unit::Px;
    }
    if s.dash_unit == Unit::Drawing {
        for d in &mut s.dash {
            *d = (*d as f64 / scale) as f32;
        }
        s.dash_phase = (s.dash_phase as f64 / scale) as f32;
        s.dash_unit = Unit::Px;
    }
    s.hatches = 0;
    s.by_block = 0;
    s
}

pub(super) fn build(items: &[Item], cam: &Camera, size: (u32, u32)) -> Built {
    let mut b = Built { geoms: Vec::new(), styles: Vec::new() };
    // a point in the overlay's frame: target pixels, y up
    let px = |p: [f64; 2], sp: Space| -> [f64; 2] {
        let q = if sp == Space::Drawing { cam.to_screen(p, size) } else { p };
        [q[0], -q[1]]
    };
    // a point on its pixel's centre (grips and markers stay crisp)
    let snapped = |p: [f64; 2], sp: Space| -> [f64; 2] {
        let q = if sp == Space::Drawing { cam.to_screen(p, size) } else { p };
        [fl(q[0]) + 0.5, -(fl(q[1]) + 0.5)]
    };
    let tau = core::f64::consts::TAU;
    for it in items {
        match it {
            Item::Line(a, e, sp, st) => {
                let s = b.style(in_px(st, cam.scale));
                b.geoms.push((Geom::Line([px(*a, *sp), px(*e, *sp)]), s));
            }
            Item::Poly(p, closed, sp, st) => {
                let s = b.style(in_px(st, cam.scale));
                b.geoms.push((Geom::Poly(p.iter().map(|q| px(*q, *sp)).collect(), *closed), s));
            }
            Item::Arc(c, r, a0, sw, sp, st) => {
                let s = b.style(in_px(st, cam.scale));
                // (drawing units: turned by the camera's rotation, scaled; pixels: y down mirrors the angles)
                let (r2, a2, sw2) = if *sp == Space::Drawing { (r / cam.scale, a0 + cam.rotation, *sw) } else { (*r, -a0, -sw) };
                let sw2 = if sw2.abs() >= tau { tau.copysign(sw2) } else { sw2 };
                b.geoms.push((Geom::Arc(px(*c, *sp), r2, a2, sw2), s));
            }
            Item::Fill(p, sp, color) => {
                let s = b.style(Style::new(*color));
                let pts: Vec<[f64; 2]> = p.iter().map(|q| px(*q, *sp)).collect();
                b.geoms.push((fill_of(&pts), s));
            }
            Item::Box(a, e, sp) => {
                let (qa, qe) = (px(*a, *sp), px(*e, *sp));
                let window = box_is_window([qa[0], -qa[1]], [qe[0], -qe[1]]);
                let pts = vec![qa, [qe[0], qa[1]], qe, [qa[0], qe[1]]];
                let (edge, tint) = if window { (WINDOW_EDGE, WINDOW_FILL) } else { (CROSSING_EDGE, CROSSING_FILL) };
                let f = b.style(Style::new(tint));
                b.geoms.push((fill_of(&pts), f));
                let mut st = pen(edge, 1.0);
                if !window {
                    st = st.dashed(&[5.0, 3.0], Unit::Px, 0.0);
                }
                let s = b.style(st);
                b.geoms.push((Geom::Poly(pts, true), s));
            }
            Item::Grip(p, sp, state) => {
                let c = snapped(*p, *sp);
                let h = 0.5 * GRIP_PX;
                let color = match state {
                    Grip::Cold => GRIP_COLD,
                    Grip::Hover => GRIP_HOVER,
                    Grip::Hot => GRIP_HOT,
                };
                // the square as a stroke as wide as it is long (exact edges), its outline on the pixels at its border
                let mut fillst = pen(color, GRIP_PX as f32);
                fillst.cap = Cap::Butt;
                let s = b.style(fillst);
                b.geoms.push((Geom::Line([[c[0] - h, c[1]], [c[0] + h, c[1]]]), s));
                let e = b.style(pen(GRIP_EDGE, 1.0));
                let k = h - 0.5;
                b.geoms.push((Geom::Poly(vec![[c[0] - k, c[1] - k], [c[0] + k, c[1] - k], [c[0] + k, c[1] + k], [c[0] - k, c[1] + k]], true), e));
            }
            Item::Marker(kind, p, sp) => {
                let c = snapped(*p, *sp);
                let mut st = pen(MARKER_COLOR, MARKER_WIDTH);
                st.join = Join::Miter;
                let s = b.style(st);
                let h = 0.5 * MARKER_PX;
                let at = |x: f64, y: f64| [c[0] + x, c[1] - y];
                let mut poly = |v: Vec<[f64; 2]>, closed: bool| b.geoms.push((Geom::Poly(v, closed), s));
                match kind {
                    SnapKind::Endpoint => poly(vec![at(-h, -h), at(h, -h), at(h, h), at(-h, h)], true),
                    SnapKind::Midpoint => poly(vec![at(-h, h * 0.8), at(h, h * 0.8), at(0.0, -h)], true),
                    SnapKind::Quadrant => poly(vec![at(0.0, -h), at(h, 0.0), at(0.0, h), at(-h, 0.0)], true),
                    SnapKind::Intersection => {
                        poly(vec![at(-h, -h), at(h, h)], false);
                        poly(vec![at(-h, h), at(h, -h)], false);
                    }
                    SnapKind::Perpendicular => {
                        poly(vec![at(-h, -h), at(-h, h), at(h, h)], false);
                        poly(vec![at(-h, 0.0), at(0.0, 0.0), at(0.0, h)], false);
                    }
                    SnapKind::Nearest => poly(vec![at(-h, -h), at(h, -h), at(-h, h), at(h, h)], true),
                    SnapKind::Insertion => {
                        let k = 0.35 * h;
                        poly(vec![at(-h, -h), at(k, -h), at(k, k), at(-h, k)], true);
                        poly(vec![at(-k, -k), at(h, -k), at(h, h), at(-k, h)], true);
                    }
                    SnapKind::Tangent => {
                        b.geoms.push((Geom::Arc(c, 0.8 * h, 0.0, tau), s));
                        b.geoms.push((Geom::Poly(vec![at(-h, -0.8 * h), at(h, -0.8 * h)], false), s));
                    }
                    _ => b.geoms.push((Geom::Arc(c, h, 0.0, tau), s)),
                }
            }
        }
    }
    b
}

fn fl(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// a filled polygon in pixels as a fill (triangulated, its edges' neighbours within the band of a pixel)
fn fill_of(pts: &[[f64; 2]]) -> Geom {
    let ring = Curve::polyline(pts, false).ring();
    let mut prep = fill::prepare(&[pts.to_vec()]);
    prep.near(curve::pow2(1) * curve::HYST);
    let bounds = ring.bounds();
    Geom::Fill(Box::new(FillG { rings: vec![ring], prep, st: FlatState { band: 0, coarse: 0, window: None }, bounds }))
}

/// a fill edge in the target's pixels (from its top left), clipped to the target and 8 px around it (common.wgsl edge_px:
/// what count writes for the scene's fills), from the record's own hi / lo
fn edge_px(r: &[u32; RECORD_WORDS], size: (u32, u32)) -> [f32; 4] {
    let f = |k: usize| f32::from_bits(r[k]) as f64;
    // the overlay's points are (x, -y) of the target's pixels
    let (x0, y0) = (f(0) + f(2), -(f(1) + f(3)));
    let (x1, y1) = (f(4) + f(6), -(f(5) + f(7)));
    let (lo, hix, hiy) = (-8.0, size.0 as f64 + 8.0, size.1 as f64 + 8.0);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, d, a, b) in [(x0, dx, lo, hix), (y0, dy, lo, hiy)] {
        if d == 0.0 {
            if p < a || p > b {
                return [-1e6; 4];
            }
        } else {
            let (ta, tb) = ((a - p) / d, (b - p) / d);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        }
    }
    if t0 > t1 {
        return [-1e6; 4];
    }
    [(x0 + t0 * dx) as f32, (y0 + t0 * dy) as f32, (x0 + t1 * dx) as f32, (y0 + t1 * dy) as f32]
}

/// the overlay's buffers on the GPU (device memory; a frame's data come in by copies from its upload buffer)
pub(crate) struct OvGpu {
    pub view: Buffer,
    pub pool: Buffer,
    pub styles: Buffer,
    pub visible: Buffer,
    pub lists: Buffer,
    pub epx: Buffer,
    /// what bindings 5, 7 and 8 (hatches, instances) get: the overlay has none
    pub dummy: Buffer,
    pub caps: (u32, u32),
    pub group: Option<BindGroup>,
}

/// what record draws of the overlay this frame: the copies (upload, offset, destination, offset, bytes; destination: 0 view,
/// 1 pool, 2 styles, 3 visible, 4 lists, 5 epx) and the quads
pub(crate) struct OvUp {
    pub buf: Buffer,
    pub copies: Vec<(u64, u32, u64)>,
    pub quads: u32,
}

impl Scene {
    /// the overlay: what is drawn over the scene with the next frames (clear it and fill it again for each frame)
    pub fn overlay(&mut self) -> &mut Overlay {
        &mut self.ov
    }

    /// what the last prepare made of the overlay
    pub fn overlay_info(&self) -> OverlayInfo {
        self.ov_info
    }

    /// (prepare) the overlay into the frame's upload buffer; the bytes
    pub(crate) fn ov_prepare(&mut self, r: &crate::Renderer, slot: usize, size: (u32, u32)) -> Result<u64> {
        self.ov_info = OverlayInfo { items: self.ov.items.len() as u32, ..OverlayInfo::default() };
        if self.ov.items.is_empty() {
            return Ok(0);
        }
        let b = build(&self.ov.items, &self.camera, size);
        let mut recs: Vec<[u32; RECORD_WORDS]> = Vec::new();
        for (g, s) in &b.geoms {
            records(g, *s, [0.0, 0.0], &mut recs);
        }
        let visible: Vec<u32> = (0..recs.len() as u32).filter(|&k| recs[k as usize][14] != 0).collect();
        let epx: Vec<[f32; 4]> = recs.iter().map(|r| if (r[15] >> 8) & 15 == K_EDGE && r[14] != 0 { edge_px(r, size) } else { [-1e6; 4] }).collect();
        let styles: Vec<[u32; STYLE_WORDS]> = b.styles.iter().map(|s| s.words(1.0, 0)).collect();
        let cam = Camera::new([0.5 * size.0 as f64, -0.5 * size.1 as f64], 1.0);
        let view = cam.words(size, true, 1.0);
        let lists = [0u32, visible.len() as u32, 0, 0];
        let n = recs.len().max(1) as u32;
        let ns = styles.len().max(1) as u32;
        self.ov_info.records = recs.len() as u32;
        self.ov_info.drawn = visible.len() as u32;
        self.ov_info.styles = styles.len() as u32;
        // the device's buffers big enough (new ones: a new bind group)
        let need = self.ov_gpu.as_ref().is_none_or(|g| g.caps.0 < n || g.caps.1 < ns);
        if need {
            let (cn, cs) = self.ov_gpu.as_ref().map_or((0, 0), |g| g.caps);
            let (cn, cs) = (n.max(cn * 2).max(256), ns.max(cs * 2).max(16));
            let dev = &self.dev;
            let st = |size: u64| dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
            self.ov_gpu = Some(OvGpu {
                view: dev.create_buffer(&BufferDesc { size: VIEW_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?,
                pool: st(cn as u64 * RECORD_BYTES)?,
                styles: st(cs as u64 * STYLE_BYTES)?,
                visible: st(cn as u64 * 4)?,
                lists: st(16)?,
                epx: st(cn as u64 * 16)?,
                dummy: st(512)?,
                caps: (cn, cs),
                group: None,
            });
        }
        let Some(g) = self.ov_gpu.as_mut() else { return Err(GpuError::Invalid("Scene::prepare")) };
        if g.group.is_none() {
            let e = super::entry;
            g.group = Some(self.dev.create_bind_group(&BindGroupDesc {
                layout: &r.draw_layout,
                entries: &[e(0, &g.view), e(1, &g.pool), e(2, &g.styles), e(3, &g.visible), e(4, &g.lists), e(5, &g.dummy), e(6, &g.epx), e(7, &g.dummy), e(8, &g.dummy)],
            })?);
        }
        // the frame's upload buffer: each part 256-aligned
        let al = |x: u64| x.div_ceil(256) * 256;
        let parts: [(u32, u64); 6] = [(0, VIEW_BYTES), (1, recs.len() as u64 * RECORD_BYTES), (2, styles.len() as u64 * STYLE_BYTES), (3, visible.len() as u64 * 4), (4, 16), (5, epx.len() as u64 * 16)];
        let mut offs = [0u64; 6];
        let mut total = 0u64;
        for (k, &(_, bytes)) in parts.iter().enumerate() {
            offs[k] = total;
            total += al(bytes.max(4));
        }
        let f = &mut self.frames[slot];
        if f.ov.as_ref().is_none_or(|x| x.size() < total) {
            f.ov = Some(upload(&self.dev, total.next_power_of_two().max(4096))?);
        }
        let Some(buf) = f.ov.clone() else { return Err(GpuError::Invalid("Scene::prepare")) };
        buf.write(offs[0], &view)?;
        if !recs.is_empty() {
            buf.write(offs[1], &recs)?;
        }
        if !styles.is_empty() {
            buf.write(offs[2], &styles)?;
        }
        if !visible.is_empty() {
            buf.write(offs[3], &visible)?;
        }
        buf.write(offs[4], &lists)?;
        if !epx.is_empty() {
            buf.write(offs[5], &epx)?;
        }
        let copies: Vec<(u64, u32, u64)> = parts.iter().enumerate().filter(|(_, p)| p.1 > 0).map(|(k, p)| (offs[k], p.0, p.1)).collect();
        let bytes: u64 = parts.iter().map(|p| p.1).sum();
        self.ov_info.bytes = bytes;
        self.up.ov = Some(OvUp { buf, copies, quads: visible.len() as u32 });
        Ok(bytes)
    }
}
