// scene -- a scene's entities, layers, styles and camera, and what goes up of them (lib.rs has the overview,
// docs/RENDER2D.md the reasons)
//
// On the CPU: one Ent per entity with its f64 geometry, one LayerSlot per layer.  On the GPU: one record of 64 bytes
// per segment (common.wgsl Record) in one pool.  Every layer owns a region of the pool, whole pages of 256 records:
// its entities' records lie there in the order of insertion, each entity's together, so the pages of the visible
// layers in their draw order are the painter's order the visibility keeps (cull.wgsl).  An insert appends at its
// layer's end; an update in place when its records fit what the entity holds (alloc), else the layer is laid out
// anew with room for it (repack); a remove leaves free records (zeros), and a layer whose free records outgrow its
// live ones is laid out anew without them.  A region that is too small, a repack, a new or removed layer: a new
// pool, the GPU copies what stays across (no copy within one buffer: WebGPU forbids it), only changed entities'
// records come from the CPU.
//
// What a frame sends (prepare, then Renderer::record copies it inside the frame's commands, behind the barrier
// against the frame before): the view; the styles when they changed; the visible layers' pages and table when they
// changed; the changed records, in runs (few runs: a copy each) or through the scatter shader (many).  A prepare
// that never reached a submission: the next sends everything again.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::math::{sincos, split};
use crate::{Renderer, Tag, CHANGE_WORDS, DRAW_BYTES, LAYERS_MAX, PAGE, RECORD_BYTES, RECORD_WORDS, STAMPS, STYLE_BYTES, STYLE_WORDS, VIEW_BYTES};

/// what a scene shows: the point of the drawing at the target's middle, drawing units per pixel, and the rotation
/// (radians, counter-clockwise: the drawing turns that way on the screen).  The y axis points up on the screen
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub center: [f64; 2],
    pub scale: f64,
    pub rotation: f64,
}

impl Camera {
    pub const fn new(center: [f64; 2], scale: f64) -> Camera {
        Camera { center, scale, rotation: 0.0 }
    }

    /// a point of the drawing in pixels of a target of `size` (x right, y down from its top left corner; a pixel's
    /// centre at + 0.5)
    pub fn to_screen(&self, p: [f64; 2], size: (u32, u32)) -> [f64; 2] {
        let (s, c) = sincos(self.rotation);
        let (x, y) = (p[0] - self.center[0], p[1] - self.center[1]);
        let (rx, ry) = ((c * x - s * y) / self.scale, (s * x + c * y) / self.scale);
        [0.5 * size.0 as f64 + rx, 0.5 * size.1 as f64 - ry]
    }

    /// the point of the drawing under a pixel position of a target of `size`
    pub fn to_drawing(&self, q: [f64; 2], size: (u32, u32)) -> [f64; 2] {
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = ((q[0] - 0.5 * size.0 as f64) * self.scale, (0.5 * size.1 as f64 - q[1]) * self.scale);
        [self.center[0] + c * rx + s * ry, self.center[1] - s * rx + c * ry]
    }

    /// the drawing follows a pointer that moved by (dx, dy) pixels (a drag)
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = (dx * self.scale, -dy * self.scale);
        self.center = [self.center[0] - (c * rx + s * ry), self.center[1] - (-s * rx + c * ry)];
    }

    /// zoom in by `factor` (below 1: out) about the pixel position q of a target of `size`: the point under q stays
    pub fn zoom_at(&mut self, q: [f64; 2], factor: f64, size: (u32, u32)) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let p = self.to_drawing(q, size);
        self.scale /= factor;
        let (s, c) = sincos(self.rotation);
        let (rx, ry) = ((q[0] - 0.5 * size.0 as f64) * self.scale, (0.5 * size.1 as f64 - q[1]) * self.scale);
        self.center = [p[0] - (c * rx + s * ry), p[1] - (-s * rx + c * ry)];
    }

    fn check(&self) -> Result<()> {
        let fine = self.center.iter().all(|v| v.is_finite() && v.abs() < 1e30) && self.scale.is_finite() && self.scale > 0.0 && self.rotation.is_finite();
        if fine { Ok(()) } else { Err(GpuError::Invalid("Camera: a finite centre, a scale above 0, a finite rotation")) }
    }

    /// common.wgsl's View for a target of `size`: the centre as hi + lo and what that leaves of it, the scale, the
    /// rotation, the size
    fn words(&self, size: (u32, u32), all: bool) -> [f32; 16] {
        let (hx, lx) = split(self.center[0]);
        let (hy, ly) = split(self.center[1]);
        let rx = ((self.center[0] - hx as f64) - lx as f64) as f32;
        let ry = ((self.center[1] - hy as f64) - ly as f64) as f32;
        let (s, c) = sincos(self.rotation);
        [hx, hy, lx, ly, rx, ry, (1.0 / self.scale) as f32, self.scale as f32, c as f32, s as f32, size.0 as f32, size.1 as f32, all as u32 as f32, 0.0, 0.0, 0.0]
    }
}

/// how a style's width is measured
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    /// pixels of the target, whatever the zoom
    Px,
    /// units of the drawing: the stroke grows and shrinks with the zoom
    Drawing,
}

/// how an open end ends
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cap {
    /// at the end point
    Butt,
    /// a half circle around it
    Round,
    /// half the width beyond it
    Square,
}

/// how two segments of a polyline meet on the outer side of the turn
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Join {
    /// the outer edges meet, up to the miter limit; beyond it a bevel
    Miter,
    /// a circle around the vertex
    Round,
    /// a straight cut between the outer corners
    Bevel,
}

/// a stroke's look.  Made with `new` and changed field by field (it may grow: dash patterns come)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// 0xAARRGGBB: sRGB codes as the target shows them, straight alpha
    pub color: u32,
    /// in `unit`s; 0 (or anything thinner than a pixel): a hairline, 1 px wide
    pub width: f32,
    pub unit: Unit,
    pub cap: Cap,
    pub join: Join,
    /// the longest miter over the width (lib/gfx's and SVG's ratio); at least 1
    pub miter_limit: f32,
}

impl Style {
    /// a hairline of a colour: width 0, butt caps, miter joins with the limit 4
    pub const fn new(color: u32) -> Style {
        Style { color, width: 0.0, unit: Unit::Px, cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0 }
    }

    fn check(&self) -> Result<()> {
        if self.width.is_finite() && self.width >= 0.0 && self.miter_limit.is_finite() && self.miter_limit >= 1.0 {
            Ok(())
        } else {
            Err(GpuError::Invalid("Style: a finite width, not negative, and a miter limit of at least 1"))
        }
    }

    /// common.wgsl's Style
    fn words(&self) -> [u32; STYLE_WORDS] {
        let unit = match self.unit {
            Unit::Px => 0,
            Unit::Drawing => 1,
        };
        let cap = match self.cap {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        };
        let join = match self.join {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
        };
        [self.color, self.width.to_bits(), unit | cap << 4 | join << 8, self.miter_limit.to_bits(), 0, 0, 0, 0]
    }
}

/// a style of a Scene
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleId(pub u32);

/// a layer's settings.  Made with `new` and changed field by field (it may grow)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub visible: bool,
    /// layers are drawn by this, lowest first; equal ones in the order they were added
    pub order: i32,
}

impl Layer {
    pub const fn new(order: i32) -> Layer {
        Layer { visible: true, order }
    }
}

/// a layer of a Scene; dead after remove_layer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId {
    pub slot: u32,
    era: u32,
}

/// an entity of a Scene; dead after remove, also when its place has gone to another one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId {
    pub slot: u32,
    era: u32,
}

/// what an entity is (more kinds come: arcs, curves, fills, text)
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum Shape<'a> {
    Line { a: [f64; 2], b: [f64; 2] },
    /// at least one point; closed: a segment from the last back to the first, joined at every point
    Polyline { points: &'a [[f64; 2]], closed: bool },
}

/// what prepare sent for this frame
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadInfo {
    /// records that came from the CPU (changed entities, free places), and in how many runs
    pub records: u32,
    pub runs: u32,
    /// they went through the scatter shader (many runs), else a copy per run
    pub scatter: bool,
    /// a new pool (a layout changed), and the records the GPU copied across into it
    pub rebuild: bool,
    pub moved: u64,
    /// the visible layers' pages and table went up
    pub tables: bool,
    /// everything prepare wrote for the GPU, the view included
    pub bytes: u64,
}

/// what a finished frame measured (Scene::set_stats)
#[derive(Clone, Copy, Debug)]
pub struct FrameStats {
    /// counts the scene's frames from 0
    pub frame: u64,
    /// records the visibility found in the target
    pub visible: u64,
    /// the GPU's clock (ms) at the start, after the uploads, after the visibility, after the draw (None: no
    /// timestamps here)
    pub stamps: Option<[f64; 4]>,
}

#[derive(Clone)]
enum Geom {
    Line([[f64; 2]; 2]),
    Poly(Vec<[f64; 2]>, bool),
}

#[derive(Clone)]
struct Ent {
    live: bool,
    era: u32,
    layer: u32,
    style: u32,
    /// its first record in its layer's region, the records it holds there, the records its geometry has
    at: u32,
    alloc: u32,
    count: u32,
    /// its records go up with the next prepare
    dirty: bool,
    geom: Geom,
}

/// a place of the layer region nobody holds
const NONE: u32 = u32::MAX;

struct LayerSlot {
    live: bool,
    era: u32,
    visible: bool,
    order: i32,
    /// when it was added: equal orders draw in this order
    seq: u64,
    /// its region in the pool (records), and how much of it is laid out (used) or free inside that (holes)
    base: u32,
    cap: u32,
    used: u32,
    holes: u32,
    /// who holds each laid-out record (NONE: free)
    owners: Vec<u32>,
    /// lay it out anew with the next prepare: entities that grew out of what they hold, free records dropped
    repack: bool,
    /// laid out anew by this prepare: its free places need no zeros
    repacked: bool,
}

/// the bind groups of the scene's buffers (made again when one of them is new)
pub(crate) struct Groups {
    pub count: BindGroup,
    pub scan: BindGroup,
    pub write: BindGroup,
    pub draw: BindGroup,
}

/// a frame in flight's own data
pub(crate) struct FrameSlot {
    pub stage: Buffer,
    changes: Option<Buffer>,
    pub scatter_group: Option<BindGroup>,
    runs: Option<Buffer>,
    pub readback: Option<Buffer>,
    last: Option<(SubmitId, u64)>,
}

/// what the last prepare left for Renderer::record
#[derive(Default)]
pub(crate) struct Uploads {
    /// the old pool and the copies (source, destination, bytes) into the new one
    pub rebuild: Option<(Buffer, Vec<(u64, u64, u64)>)>,
    pub styles: Option<(Buffer, u64)>,
    /// the pages (bytes) and the layer table (bytes) behind them
    pub tables: Option<(Buffer, u64, u64)>,
    /// record runs: (source, destination, bytes)
    pub runs: Option<(Buffer, Vec<(u64, u64, u64)>)>,
    /// rows in the frame slot's change buffer
    pub scatter: u32,
}

fn storage(dev: &Device, size: u64, more: BufferUsage) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::STORAGE | more, memory: MemoryKind::Device })
}

fn entry(binding: u32, buffer: &Buffer) -> BindEntry<'_> {
    BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } }
}

fn upload(dev: &Device, size: u64) -> Result<Buffer> {
    dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::NONE, memory: MemoryKind::Upload })
}

fn pages(n: u32) -> u32 {
    n.div_ceil(PAGE) * PAGE
}

/// entities, layers, styles and the camera of one scene of a Renderer
pub struct Scene {
    dev: Device,
    pub(crate) tag: Arc<Tag>,
    max_records: u32,
    max_entries: u32,
    styles: Vec<Style>,
    styles_dirty: bool,
    layers: Vec<LayerSlot>,
    free_layers: Vec<u32>,
    layer_seq: u64,
    ents: Vec<Ent>,
    free_ents: Vec<u32>,
    live: u32,
    dirty: Vec<u32>,
    /// free places to clear: (layer, its era, first, records)
    freed: Vec<(u32, u32, u32, u32)>,
    /// a region is too small, a layer came or went: a new pool
    layout_dirty: bool,
    camera: Camera,
    pub(crate) size: (u32, u32),
    culling: bool,
    pub(crate) stats_on: bool,
    pub(crate) pool: Buffer,
    pool_cap: u32,
    pub(crate) styles_buf: Buffer,
    styles_cap: u32,
    pub(crate) order_buf: Buffer,
    masks: Buffer,
    counts: Buffer,
    offsets: Buffer,
    entries_cap: u32,
    pub(crate) layers_buf: Buffer,
    pub(crate) lists_buf: Buffer,
    pub(crate) draws_buf: Buffer,
    layers_cap: u32,
    visible: Buffer,
    visible_cap: u32,
    pub(crate) view_ubo: Buffer,
    pub(crate) groups: Option<Groups>,
    pub(crate) frames: Vec<FrameSlot>,
    pub(crate) ts: Option<Timestamps>,
    /// the tables as they went up last, the visible layers in draw order
    last_order: Vec<u32>,
    last_layers: Vec<[u32; 4]>,
    drawn: Vec<u32>,
    pub(crate) entries: u32,
    pub(crate) vis_layers: u32,
    pub(crate) up: Uploads,
    upload_info: UploadInfo,
    frame: u64,
    pub(crate) slot: usize,
    pub(crate) prepared: bool,
    /// a prepare has run since the last submission (what it sent has not reached the GPU)
    sent: bool,
    /// the scene changed after prepare
    pub(crate) stale: bool,
    waited: bool,
    /// the pool was ever filled (else the first prepare sends everything)
    filled: bool,
    finished: Vec<FrameStats>,
}

impl Scene {
    pub fn new(r: &Renderer) -> Result<Scene> {
        let dev = r.dev.clone();
        let none = BufferUsage::NONE;
        let (entries_cap, layers_cap) = (64u32, 16u32);
        let mut frames = Vec::new();
        for _ in 0..r.desc.frames_in_flight {
            frames.push(FrameSlot { stage: upload(&dev, VIEW_BYTES)?, changes: None, scatter_group: None, runs: None, readback: None, last: None });
        }
        Ok(Scene {
            tag: r.tag.clone(),
            max_records: r.max_records,
            max_entries: r.max_entries,
            styles: Vec::new(),
            styles_dirty: false,
            layers: Vec::new(),
            free_layers: Vec::new(),
            layer_seq: 0,
            ents: Vec::new(),
            free_ents: Vec::new(),
            live: 0,
            dirty: Vec::new(),
            freed: Vec::new(),
            layout_dirty: true,
            camera: Camera::new([0.0, 0.0], 1.0),
            size: (0, 0),
            culling: true,
            stats_on: false,
            pool: storage(&dev, PAGE as u64 * RECORD_BYTES, none)?,
            pool_cap: PAGE,
            styles_buf: storage(&dev, 16 * STYLE_BYTES, none)?,
            styles_cap: 16,
            order_buf: storage(&dev, entries_cap as u64 * 4, none)?,
            masks: storage(&dev, entries_cap as u64 * 32, none)?,
            counts: storage(&dev, entries_cap as u64 * 4, none)?,
            offsets: storage(&dev, entries_cap as u64 * 4, none)?,
            entries_cap,
            layers_buf: storage(&dev, layers_cap as u64 * 16, none)?,
            lists_buf: storage(&dev, layers_cap as u64 * 16, none)?,
            draws_buf: storage(&dev, layers_cap as u64 * DRAW_BYTES, BufferUsage::INDIRECT)?,
            layers_cap,
            visible: storage(&dev, PAGE as u64 * 4, none)?,
            visible_cap: PAGE,
            view_ubo: dev.create_buffer(&BufferDesc { size: VIEW_BYTES, usage: BufferUsage::UNIFORM, memory: MemoryKind::Device })?,
            groups: None,
            frames,
            ts: None,
            last_order: Vec::new(),
            last_layers: Vec::new(),
            drawn: Vec::new(),
            entries: 0,
            vis_layers: 0,
            up: Uploads::default(),
            upload_info: UploadInfo::default(),
            frame: 0,
            slot: 0,
            prepared: false,
            sent: false,
            stale: false,
            waited: false,
            filled: false,
            finished: Vec::new(),
            dev,
        })
    }

    fn touch(&mut self) {
        if self.prepared {
            self.stale = true;
        }
    }

    // ------------------------------------------------------------ styles

    /// a style for entities; the next frame shows it
    pub fn add_style(&mut self, s: &Style) -> Result<StyleId> {
        s.check()?;
        self.touch();
        self.styles.push(*s);
        self.styles_dirty = true;
        Ok(StyleId(self.styles.len() as u32 - 1))
    }

    /// a style changed: every entity of it shows the change with the next frame (nothing else goes up)
    pub fn set_style(&mut self, id: StyleId, s: &Style) -> Result<()> {
        s.check()?;
        let Some(old) = self.styles.get_mut(id.0 as usize) else { return Err(GpuError::Invalid("set_style: no such style")) };
        *old = *s;
        self.styles_dirty = true;
        self.touch();
        Ok(())
    }

    pub fn style(&self, id: StyleId) -> Option<Style> {
        self.styles.get(id.0 as usize).copied()
    }

    // ------------------------------------------------------------ layers

    /// a new layer, empty, on top of those of its order
    pub fn add_layer(&mut self, l: &Layer) -> Result<LayerId> {
        self.touch();
        let seq = self.layer_seq;
        self.layer_seq += 1;
        let fresh = LayerSlot { live: true, era: 0, visible: l.visible, order: l.order, seq, base: 0, cap: 0, used: 0, holes: 0, owners: Vec::new(), repack: false, repacked: false };
        let slot = match self.free_layers.pop() {
            Some(s) => {
                let era = self.layers[s as usize].era;
                self.layers[s as usize] = LayerSlot { era, ..fresh };
                s
            }
            None => {
                self.layers.push(fresh);
                self.layers.len() as u32 - 1
            }
        };
        // (no region until it holds something)
        Ok(LayerId { slot, era: self.layers[slot as usize].era })
    }

    fn layer_index(&self, id: LayerId) -> Result<usize> {
        match self.layers.get(id.slot as usize) {
            Some(l) if l.live && l.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this layer was removed")),
        }
    }

    /// shown or hidden, its place in the draw order (nothing of its entities goes up)
    pub fn set_layer(&mut self, id: LayerId, l: &Layer) -> Result<()> {
        let i = self.layer_index(id)?;
        self.touch();
        let s = &mut self.layers[i];
        if s.order != l.order {
            // (a new order: on top of the layers of that order)
            s.seq = self.layer_seq;
            self.layer_seq += 1;
        }
        s.visible = l.visible;
        s.order = l.order;
        Ok(())
    }

    pub fn layer(&self, id: LayerId) -> Result<Layer> {
        let l = &self.layers[self.layer_index(id)?];
        Ok(Layer { visible: l.visible, order: l.order })
    }

    /// the layer goes, with every entity on it
    pub fn remove_layer(&mut self, id: LayerId) -> Result<()> {
        let i = self.layer_index(id)?;
        self.touch();
        let owners = core::mem::take(&mut self.layers[i].owners);
        let mut last = NONE;
        for o in owners {
            if o != NONE && o != last && self.ents[o as usize].live {
                let e = &mut self.ents[o as usize];
                e.live = false;
                e.era = e.era.wrapping_add(1);
                self.free_ents.push(o);
                self.live -= 1;
            }
            last = o;
        }
        let l = &mut self.layers[i];
        l.live = false;
        l.era = l.era.wrapping_add(1);
        l.used = 0;
        l.holes = 0;
        l.cap = 0;
        // (its region stays unused until the pool is laid out again)
        self.free_layers.push(i as u32);
        Ok(())
    }

    /// the visible layers in the order the last prepare drew them (the lists' order: lists_buffer)
    pub fn drawn_layers(&self) -> Vec<LayerId> {
        self.drawn.iter().map(|&s| LayerId { slot: s, era: self.layers[s as usize].era }).collect()
    }

    // ------------------------------------------------------------ entities

    fn check_points(p: &[[f64; 2]]) -> Result<()> {
        if p.is_empty() {
            return Err(GpuError::Invalid("Shape: at least one point"));
        }
        if p.iter().all(|q| q[0].is_finite() && q[1].is_finite() && q[0].abs() < 1e30 && q[1].abs() < 1e30) {
            Ok(())
        } else {
            Err(GpuError::Invalid("Shape: finite points, below 1e30"))
        }
    }

    fn geom(s: &Shape) -> Result<Geom> {
        match *s {
            Shape::Line { a, b } => {
                Scene::check_points(&[a, b])?;
                Ok(Geom::Line([a, b]))
            }
            Shape::Polyline { points, closed } => {
                Scene::check_points(points)?;
                Ok(Geom::Poly(points.to_vec(), closed))
            }
        }
    }

    fn mark(&mut self, slot: u32) {
        let e = &mut self.ents[slot as usize];
        if !e.dirty {
            e.dirty = true;
            self.dirty.push(slot);
        }
    }

    /// a new entity at the end of its layer (drawn over everything there)
    pub fn insert(&mut self, layer: LayerId, style: StyleId, s: &Shape) -> Result<EntityId> {
        let li = self.layer_index(layer)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        let geom = Scene::geom(s)?;
        let count = record_count(&geom);
        self.touch();
        let slot = match self.free_ents.pop() {
            Some(f) => f,
            None => {
                self.ents.push(Ent { live: false, era: 0, layer: 0, style: 0, at: 0, alloc: 0, count: 0, dirty: false, geom: Geom::Line([[0.0; 2]; 2]) });
                self.ents.len() as u32 - 1
            }
        };
        let l = &mut self.layers[li];
        let at = l.used;
        l.used += count;
        l.owners.resize(l.used as usize, slot);
        if l.used > l.cap {
            self.layout_dirty = true;
        }
        let e = &mut self.ents[slot as usize];
        e.live = true;
        e.layer = li as u32;
        e.style = style.0;
        e.at = at;
        e.alloc = count;
        e.count = count;
        e.geom = geom;
        let era = e.era;
        self.live += 1;
        self.mark(slot);
        Ok(EntityId { slot, era })
    }

    fn get(&self, id: EntityId) -> Result<usize> {
        match self.ents.get(id.slot as usize) {
            Some(e) if e.live && e.era == id.era => Ok(id.slot as usize),
            _ => Err(GpuError::Invalid("Scene: this entity was removed")),
        }
    }

    /// new geometry; the entity keeps its place in the draw order
    pub fn update(&mut self, id: EntityId, s: &Shape) -> Result<()> {
        let i = self.get(id)?;
        let geom = Scene::geom(s)?;
        let count = record_count(&geom);
        self.touch();
        let (li, at, alloc) = (self.ents[i].layer as usize, self.ents[i].at, self.ents[i].alloc);
        if count > alloc {
            let l = &mut self.layers[li];
            if at + alloc == l.used {
                // the last of its layer: it grows in place
                l.used = at + count;
                l.owners.resize(l.used as usize, i as u32);
                if l.used > l.cap {
                    self.layout_dirty = true;
                }
                self.ents[i].alloc = count;
            } else {
                // room in the middle: the layer is laid out anew, with room to grow again
                l.repack = true;
                self.ents[i].alloc = count.max(2 * alloc);
            }
        }
        let e = &mut self.ents[i];
        e.geom = geom;
        e.count = count;
        self.mark(i as u32);
        Ok(())
    }

    /// another style: its records go up again
    pub fn set_entity_style(&mut self, id: EntityId, style: StyleId) -> Result<()> {
        let i = self.get(id)?;
        if style.0 as usize >= self.styles.len() {
            return Err(GpuError::Invalid("Scene: a style it does not have"));
        }
        self.touch();
        self.ents[i].style = style.0;
        self.mark(i as u32);
        Ok(())
    }

    /// the entity goes; its records become free places (and its id dead)
    pub fn remove(&mut self, id: EntityId) -> Result<()> {
        let i = self.get(id)?;
        self.touch();
        let e = &mut self.ents[i];
        e.live = false;
        e.era = e.era.wrapping_add(1);
        let (li, at, n) = (e.layer as usize, e.at, e.alloc);
        let l = &mut self.layers[li];
        for o in &mut l.owners[at as usize..(at + n) as usize] {
            *o = NONE;
        }
        l.holes += n;
        if !l.repack && l.holes >= 1024 && l.holes * 2 > l.used {
            l.repack = true;
        }
        self.freed.push((li as u32, l.era, at, n));
        self.free_ents.push(i as u32);
        self.live -= 1;
        Ok(())
    }

    /// entities alive
    pub fn len(&self) -> u32 {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// (tests, tools) an entity's records as the last prepare laid them out: its first record's place in the pool
    /// and the records of its geometry (a polyline of n points: n - 1, closed n; a point: 1)
    pub fn locate(&self, id: EntityId) -> Option<(u32, u32)> {
        let i = self.get(id).ok()?;
        let e = &self.ents[i];
        Some((self.layers[e.layer as usize].base + e.at, e.count))
    }

    pub fn set_camera(&mut self, c: &Camera) -> Result<()> {
        c.check()?;
        self.camera = *c;
        Ok(())
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// false: every live record of a visible layer is drawn (a test's comparison, a debugging aid)
    pub fn set_culling(&mut self, on: bool) {
        self.culling = on;
    }

    /// FrameStats of each frame: the records found visible (a small copy per frame), the GPU's times
    pub fn set_stats(&mut self, on: bool) {
        self.stats_on = on;
    }

    /// what the last prepare sent
    pub fn upload_info(&self) -> UploadInfo {
        self.upload_info
    }

    /// (tests, tools) after a frame's visibility: per visible layer in draw order (drawn_layers) its first place in the
    /// visible list and its count (16 bytes each); the visible list (record places in the pool); the indirect draws
    pub fn lists_buffer(&self) -> &Buffer {
        &self.lists_buf
    }

    pub fn visible_buffer(&self) -> &Buffer {
        &self.visible
    }

    pub fn draws_buffer(&self) -> &Buffer {
        &self.draws_buf
    }

    /// (tests, tools) the pool of records (common.wgsl Record, 64 bytes each)
    pub fn pool_buffer(&self) -> &Buffer {
        &self.pool
    }

    // ------------------------------------------------------------ a frame

    /// wait for the frame that last used this frame's slot, so its data can be written; prepare does it too.  The
    /// browser cannot wait, and needs not: its writes go in queue order
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
        let mut lists = vec![0u32; rb.size() as usize / 4];
        if rb.read(0, &mut lists).is_err() {
            return;
        }
        let n = (self.vis_layers as usize).min(lists.len() / 4);
        let visible: u64 = (0..n).map(|k| lists[k * 4 + 1] as u64).sum();
        let stamps = self.ts.as_ref().and_then(|ts| ts.read_ms(slot as u32 * STAMPS, STAMPS).ok()).map(|v| [v[0], v[1], v[2], v[3]]);
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

    /// a layer to lay out anew: its live entities in their order, side by side, each with what it holds; the copies
    /// (old place, new place, records) of those whose records stay
    fn repack_layer(&mut self, li: usize) -> Vec<(u32, u32, u32)> {
        let owners = core::mem::take(&mut self.layers[li].owners);
        let mut fresh = Vec::with_capacity(owners.len());
        let mut moves: Vec<(u32, u32, u32)> = Vec::new();
        let mut k = 0usize;
        while k < owners.len() {
            let o = owners[k];
            if o == NONE {
                k += 1;
                continue;
            }
            let e = &self.ents[o as usize];
            // (its records start here, k is its old place)
            let (old, n_old, n_new, dirty) = (k as u32, (owners[k..].iter().take_while(|&&x| x == o).count()) as u32, e.alloc, e.dirty);
            let at = fresh.len() as u32;
            fresh.resize(fresh.len() + n_new as usize, o);
            if !dirty {
                match moves.last_mut() {
                    Some(m) if m.0 + m.2 == old && m.1 + m.2 == at => m.2 += n_old,
                    _ => moves.push((old, at, n_old)),
                }
            }
            self.ents[o as usize].at = at;
            k += n_old as usize;
        }
        let l = &mut self.layers[li];
        l.used = fresh.len() as u32;
        l.owners = fresh;
        l.holes = 0;
        l.repack = false;
        l.repacked = true;
        moves
    }

    /// lay out what has to be laid out anew; a new pool when a region changes.  full: no copies, everything goes up
    fn layout(&mut self, full: bool) -> Result<()> {
        for l in &mut self.layers {
            l.repacked = false;
        }
        let repack = self.layers.iter().any(|l| l.live && l.repack);
        if !(self.layout_dirty || repack || full) {
            return Ok(());
        }
        // (old place, new place, records) per layer, local to its region
        let mut local: Vec<Option<Vec<(u32, u32, u32)>>> = vec![None; self.layers.len()];
        for li in 0..self.layers.len() {
            if self.layers[li].live && (self.layers[li].repack || (full && self.layers[li].holes > 0)) {
                local[li] = Some(self.repack_layer(li));
            }
        }
        // the regions: every live layer's, grown (doubling) where it is too small, packed in slot order
        let mut base = 0u64;
        let mut moves = Vec::new();
        for (li, l) in self.layers.iter_mut().enumerate() {
            if !l.live {
                continue;
            }
            if l.used == 0 && !l.repacked {
                // (nothing to hold: no region)
                l.cap = 0;
                continue;
            }
            let cap = if l.used > l.cap { pages(l.used.max(l.cap.saturating_mul(2)).max(PAGE)) } else if l.repacked && l.cap > 4 * l.used + PAGE { pages(2 * l.used).max(PAGE) } else { l.cap.max(PAGE) };
            // (what its old region held: a layer without one held nothing; records added since are sent anyway)
            if !full && self.filled && l.cap > 0 {
                let (ob, nb) = (l.base as u64, base);
                match &local[li] {
                    Some(m) => moves.extend(m.iter().map(|&(o, n, c)| ((ob + o as u64) * RECORD_BYTES, (nb + n as u64) * RECORD_BYTES, c as u64 * RECORD_BYTES))),
                    None if l.used > 0 => moves.push((ob * RECORD_BYTES, nb * RECORD_BYTES, l.used.min(l.cap) as u64 * RECORD_BYTES)),
                    None => {}
                }
            }
            l.base = base as u32;
            l.cap = cap;
            base += cap as u64;
        }
        let need = base.max(PAGE as u64);
        if need > self.max_records as u64 {
            return Err(GpuError::Unsupported("Scene: more records than this device's bindings and one dispatch reach"));
        }
        let old = core::mem::replace(&mut self.pool, storage(&self.dev, need * RECORD_BYTES, BufferUsage::NONE)?);
        self.pool_cap = need as u32;
        self.upload_info.rebuild = true;
        self.upload_info.moved = moves.iter().map(|m| m.2 / RECORD_BYTES).sum();
        self.up.rebuild = Some((old, moves));
        self.groups = None;
        for f in &mut self.frames {
            f.scatter_group = None;
        }
        self.layout_dirty = false;
        self.filled = true;
        Ok(())
    }

    /// the buffers of the visibility big enough (new ones: new bind groups)
    fn grow(&mut self) -> Result<()> {
        let dev = self.dev.clone();
        let none = BufferUsage::NONE;
        let entries = self.last_order.len() as u32;
        if entries > self.max_entries {
            return Err(GpuError::Unsupported("Scene: more pages in the visible layers than one dispatch reaches"));
        }
        if entries > self.entries_cap {
            self.entries_cap = entries.max(self.entries_cap * 2);
            let n = self.entries_cap as u64;
            self.order_buf = storage(&dev, n * 4, none)?;
            self.masks = storage(&dev, n * 32, none)?;
            self.counts = storage(&dev, n * 4, none)?;
            self.offsets = storage(&dev, n * 4, none)?;
            self.groups = None;
        }
        let layers = self.last_layers.len() as u32;
        if layers > self.layers_cap {
            self.layers_cap = layers.max(self.layers_cap * 2);
            let n = self.layers_cap as u64;
            self.layers_buf = storage(&dev, n * 16, none)?;
            self.lists_buf = storage(&dev, n * 16, none)?;
            self.draws_buf = storage(&dev, n * DRAW_BYTES, BufferUsage::INDIRECT)?;
            self.groups = None;
        }
        if self.pool_cap > self.visible_cap {
            self.visible_cap = self.pool_cap;
            self.visible = storage(&dev, self.visible_cap as u64 * 4, none)?;
            self.groups = None;
        }
        if self.styles.len() as u32 > self.styles_cap {
            self.styles_cap = (self.styles.len() as u32).max(self.styles_cap * 2);
            self.styles_buf = storage(&dev, self.styles_cap as u64 * STYLE_BYTES, none)?;
            self.styles_dirty = true;
            self.groups = None;
        }
        Ok(())
    }

    fn make_groups(&mut self, r: &Renderer) -> Result<()> {
        if self.groups.is_some() {
            return Ok(());
        }
        let dev = &self.dev;
        let g = |layout: &BindGroupLayout, entries: &[BindEntry]| dev.create_bind_group(&BindGroupDesc { layout, entries });
        self.groups = Some(Groups {
            count: g(&r.count_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.order_buf), entry(4, &self.masks),
                                         entry(5, &self.counts)])?,
            scan: g(&r.scan_layout, &[entry(5, &self.counts), entry(6, &self.layers_buf), entry(7, &self.offsets), entry(8, &self.lists_buf), entry(9, &self.draws_buf)])?,
            write: g(&r.write_layout, &[entry(3, &self.order_buf), entry(4, &self.masks), entry(7, &self.offsets), entry(10, &self.visible)])?,
            draw: g(&r.draw_layout, &[entry(0, &self.view_ubo), entry(1, &self.pool), entry(2, &self.styles_buf), entry(3, &self.visible), entry(4, &self.lists_buf)])?,
        });
        for f in &mut self.frames {
            f.scatter_group = None;
        }
        Ok(())
    }

    /// the visible layers' pages in draw order and their table
    fn tables(&mut self) -> (Vec<u32>, Vec<[u32; 4]>) {
        let mut vis: Vec<usize> = (0..self.layers.len()).filter(|&i| self.layers[i].live && self.layers[i].visible).collect();
        vis.sort_by_key(|&i| (self.layers[i].order, self.layers[i].seq));
        let mut order = Vec::new();
        let mut table = Vec::with_capacity(vis.len());
        for &i in &vis {
            let l = &self.layers[i];
            let first = order.len() as u32;
            let p0 = l.base / PAGE;
            order.extend(p0..p0 + l.used.div_ceil(PAGE));
            table.push([first, order.len() as u32 - first, 0, 0]);
        }
        self.drawn = vis.iter().map(|&i| i as u32).collect();
        (order, table)
    }

    /// the changed records and free places, by their place in the pool
    fn changed_records(&mut self) -> Vec<(u32, [u32; RECORD_WORDS])> {
        let mut rows: Vec<(u32, [u32; RECORD_WORDS])> = Vec::new();
        for (li, era, at, n) in core::mem::take(&mut self.freed) {
            let l = &self.layers[li as usize];
            // (a layer gone or laid out anew has no such places any more)
            if !l.live || l.era != era || l.repacked {
                continue;
            }
            for k in 0..n {
                rows.push((l.base + at + k, [0; RECORD_WORDS]));
            }
        }
        let mut words = Vec::new();
        for slot in core::mem::take(&mut self.dirty) {
            let e = &mut self.ents[slot as usize];
            e.dirty = false;
            if !e.live {
                continue;
            }
            let e = &self.ents[slot as usize];
            words.clear();
            records(&e.geom, e.style, &mut words);
            words.resize(e.alloc as usize, [0; RECORD_WORDS]);
            let at = self.layers[e.layer as usize].base + e.at;
            for (k, w) in words.iter().enumerate() {
                rows.push((at + k as u32, *w));
            }
        }
        // (the later of two rows for one place wins: a removed entity's place taken in the same frame by none other,
        // since free places are not handed out again; sorted stably)
        rows.sort_by_key(|r| r.0);
        rows.dedup_by(|b, a| {
            if a.0 == b.0 {
                *a = *b;
                true
            } else {
                false
            }
        });
        rows
    }

    /// everything goes up again (a prepare that never reached a submission, the first frame)
    fn resend_all(&mut self) {
        self.dirty.clear();
        self.freed.clear();
        for i in 0..self.ents.len() {
            self.ents[i].dirty = self.ents[i].live;
            if self.ents[i].live {
                self.dirty.push(i as u32);
            }
        }
        self.styles_dirty = true;
        self.last_order.clear();
        self.last_layers.clear();
        self.last_layers.push([u32::MAX; 4]);
    }

    /// the frame's data for Renderer::record: waits for this slot's last frame, sends what changed.  size: the
    /// target's in pixels
    pub fn prepare(&mut self, r: &mut Renderer, size: (u32, u32)) -> Result<()> {
        if !Arc::ptr_eq(&self.tag, &r.tag) {
            return Err(GpuError::Invalid("Scene::prepare: another Renderer than its own"));
        }
        if size.0 == 0 || size.1 == 0 {
            return Err(GpuError::Invalid("Scene::prepare: a target of at least 1 x 1"));
        }
        self.wait_slot()?;
        let full = self.sent || !self.filled;
        if full {
            self.resend_all();
        }
        self.sent = true;
        self.prepared = false;
        self.stale = false;
        let slot = (self.frame % self.frames.len() as u64) as usize;
        self.slot = slot;
        self.size = size;
        self.up = Uploads::default();
        self.upload_info = UploadInfo::default();
        self.layout(full)?;
        let (order, table) = self.tables();
        if table.len() as u32 > LAYERS_MAX {
            return Err(GpuError::Unsupported("Scene: more than 65535 visible layers"));
        }
        let tables_new = order != self.last_order || table != self.last_layers;
        self.last_order = order;
        self.last_layers = table;
        self.grow()?;
        // (new buffers hold nothing yet: the tables go up again)
        let tables_new = tables_new || self.groups.is_none();
        self.make_groups(r)?;
        self.entries = self.last_order.len() as u32;
        self.vis_layers = self.last_layers.len() as u32;
        let dev = self.dev.clone();
        let mut bytes = VIEW_BYTES;
        if self.styles_dirty && !self.styles.is_empty() {
            let words: Vec<[u32; STYLE_WORDS]> = self.styles.iter().map(|s| s.words()).collect();
            let size = words.len() as u64 * STYLE_BYTES;
            let up = upload(&dev, size)?;
            up.write(0, &words)?;
            self.up.styles = Some((up, size));
            bytes += size;
        }
        self.styles_dirty = false;
        if tables_new || self.upload_info.rebuild {
            let (ob, lb) = (self.last_order.len() as u64 * 4, self.last_layers.len() as u64 * 16);
            let up = upload(&dev, ob + lb)?;
            up.write(0, &self.last_order)?;
            up.write(ob, &self.last_layers)?;
            self.up.tables = Some((up, ob, lb));
            self.upload_info.tables = true;
            bytes += ob + lb;
        }
        // the records: runs of neighbouring places, a copy each when they are few, else the scatter shader
        let rows = self.changed_records();
        let mut runs = 0u32;
        for (k, row) in rows.iter().enumerate() {
            if k == 0 || rows[k - 1].0 + 1 != row.0 {
                runs += 1;
            }
        }
        self.upload_info.records = rows.len() as u32;
        self.upload_info.runs = runs;
        if !rows.is_empty() {
            let f = &mut self.frames[slot];
            if runs <= 256 {
                let size = rows.len() as u64 * RECORD_BYTES;
                if f.runs.as_ref().is_none_or(|b| b.size() < size) {
                    f.runs = Some(upload(&dev, size.next_power_of_two().max(1 << 14))?);
                }
                let Some(buf) = &f.runs else { return Err(GpuError::Invalid("Scene::prepare")) };
                let words: Vec<[u32; RECORD_WORDS]> = rows.iter().map(|r| r.1).collect();
                buf.write(0, &words)?;
                let mut list: Vec<(u64, u64, u64)> = Vec::with_capacity(runs as usize);
                for (k, row) in rows.iter().enumerate() {
                    match list.last_mut() {
                        Some(m) if k > 0 && rows[k - 1].0 + 1 == row.0 => m.2 += RECORD_BYTES,
                        _ => list.push((k as u64 * RECORD_BYTES, row.0 as u64 * RECORD_BYTES, RECORD_BYTES)),
                    }
                }
                self.up.runs = Some((buf.clone(), list));
                bytes += size;
            } else {
                let size = rows.len() as u64 * CHANGE_WORDS as u64 * 4;
                if f.changes.as_ref().is_none_or(|b| b.size() < size) {
                    f.changes = Some(dev.create_buffer(&BufferDesc { size: size.next_power_of_two().max(1 << 14), usage: BufferUsage::STORAGE, memory: MemoryKind::Upload })?);
                    f.scatter_group = None;
                }
                let Some(buf) = &f.changes else { return Err(GpuError::Invalid("Scene::prepare")) };
                let words: Vec<[u32; CHANGE_WORDS]> = rows.iter().map(|r| {
                    let mut w = [0u32; CHANGE_WORDS];
                    w[0] = r.0;
                    w[4..].copy_from_slice(&r.1);
                    w
                }).collect();
                buf.write(0, &words)?;
                if f.scatter_group.is_none() {
                    f.scatter_group = Some(dev.create_bind_group(&BindGroupDesc { layout: &r.scatter_layout, entries: &[entry(0, buf), entry(1, &self.pool)] })?);
                }
                self.up.scatter = rows.len() as u32;
                self.upload_info.scatter = true;
                bytes += size;
            }
        }
        if self.stats_on {
            if self.ts.is_none() && dev.features().timestamps {
                self.ts = Some(dev.create_timestamps(STAMPS * self.frames.len() as u32)?);
            }
            let size = self.layers_cap as u64 * 16;
            let f = &mut self.frames[slot];
            if f.readback.as_ref().is_none_or(|b| b.size() < size) {
                f.readback = Some(dev.create_buffer(&BufferDesc { size, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?);
            }
        }
        self.frames[slot].stage.write(0, &self.camera.words(size, !self.culling))?;
        self.upload_info.bytes = bytes;
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
        self.up = Uploads::default();
    }
}

/// the records an entity's geometry has: a segment between neighbouring points (repeated points count once), a closed
/// polyline's last back to its first; a single point: one (a dot, drawn by round and square caps)
fn record_count(g: &Geom) -> u32 {
    match g {
        Geom::Line(_) => 1,
        Geom::Poly(p, closed) => {
            let mut n = distinct(p);
            if *closed && n >= 2 && p[0] == p[p.len() - 1] {
                n -= 1;
            }
            if n <= 1 { 1 } else if *closed { n as u32 } else { n as u32 - 1 }
        }
    }
}

/// points that differ from the one before (a closed polyline's last equal to its first counts once)
fn distinct(p: &[[f64; 2]]) -> usize {
    let mut n = 0;
    for k in 0..p.len() {
        if k == 0 || p[k] != p[k - 1] {
            n += 1;
        }
    }
    n
}

/// common.wgsl's Records of an entity
fn records(g: &Geom, style: u32, out: &mut Vec<[u32; RECORD_WORDS]>) {
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let closed = match g {
        Geom::Line(p) => {
            pts.extend_from_slice(p);
            false
        }
        Geom::Poly(p, c) => {
            pts.extend_from_slice(p);
            *c
        }
    };
    pts.dedup();
    if closed && pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
        pts.pop();
    }
    let n = pts.len();
    let b = |v: f32| v.to_bits();
    // ends: 1 the previous point is the polyline's first (a cap there), 2 the next point its last
    let rec = |a: [f64; 2], e: [f64; 2], prev: Option<[f64; 2]>, next: Option<[f64; 2]>, ends: u32| -> [u32; RECORD_WORDS] {
        let (ax, alx) = split(a[0]);
        let (ay, aly) = split(a[1]);
        let (ex, elx) = split(e[0]);
        let (ey, ely) = split(e[1]);
        // (the same differences in f64 rounded once: a segment's own vector is exactly the negated previous vector of
        // the next one and the next vector of the one before, so both compute their join's bisector bit for bit)
        let pv = prev.map_or([0.0f32; 2], |p| [(p[0] - a[0]) as f32, (p[1] - a[1]) as f32]);
        let nv = next.map_or([0.0f32; 2], |p| [(p[0] - e[0]) as f32, (p[1] - e[1]) as f32]);
        let ov = [(e[0] - a[0]) as f32, (e[1] - a[1]) as f32];
        [b(ax), b(ay), b(alx), b(aly), b(ex), b(ey), b(elx), b(ely), b(pv[0]), b(pv[1]), b(nv[0]), b(nv[1]), style + 1, b(ov[0]), b(ov[1]), ends]
    };
    if n == 1 {
        out.push(rec(pts[0], pts[0], None, None, 0));
        return;
    }
    let segs = if closed { n } else { n - 1 };
    for k in 0..segs {
        let a = pts[k];
        let e = pts[(k + 1) % n];
        let prev = if closed || k > 0 { Some(pts[(k + n - 1) % n]) } else { None };
        let next = if closed || k + 1 < segs { Some(pts[(k + 2) % n]) } else { None };
        let ends = if closed { 0 } else { (k == 1) as u32 | ((segs >= 2 && k + 2 == segs) as u32) << 1 };
        out.push(rec(a, e, prev, next, ends));
    }
}
