// view -- a page of a diagram as a render2d scene plus what the scene cannot hold
//
//   PageView::new(&doc, page, &mut fonts, &opts)   the display list (diagram_core::render,
//                                    texts measured with the fonts drawn), the scene made of
//                                    it, pictures decoded
//   view.draw(dc, sf, r, &cam)       the page on the CPU into the rectangle r of a surface
//                                    (render2d's CPU path; gradients and pictures in between)
//   view.render(w, h) -> Surface     the page fitted into w x h pixels on its colour (XRGB)
//   view.fit(size, margin_px) -> Camera   the camera showing the whole page
//   view.drawing() / substitutions() / scene()
//
// The scene: render2d layers in painter's order.  A run of items render2d can draw (solid and
// hatched fills, lines with dashes, caps and joins, texts) lies on one layer; a gradient fill or
// a picture ends the run and is drawn by gfx between two layers (direct.rs).  On the CPU each
// layer is drawn on its own while the others are hidden, so the order holds.  On a GPU
// (Scene::attach, the app's choice) the same scene shows everything but those items.

use alloc::rc::Rc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::geometry::{Path, Seg, Sub};
use diagram_core::render::{self, Drawing, Face, ItemKind, Measure, Options, Paint, Stroke};
use diagram_core::{Document, PageRef};
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_XRGB};
use render2d::{Camera, Cap, FontId, HatchLine, Join, Layer, LayerId, Scene, Shape, Style, Text, TextStyle, TextStyleId, Unit};

use crate::direct::{self, Target};
use crate::fonts::{sample_width, FontSet, SAMPLE};
use crate::image::{self, Decoded};

/// how far a synthetic italic leans (radians)
pub const SYNTHETIC_ITALIC: f64 = 0.2;

#[derive(Clone, Copy)]
struct Resolved {
    font: FontId,
    style: TextStyleId,
    /// width factor (the missing font's width over the substitute's)
    width: f64,
    oblique: f64,
}

enum Run {
    Layer(LayerId),
    /// an item index of the drawing
    Direct(usize),
}

/// a page ready to draw
pub struct PageView {
    drawing: Drawing,
    scene: Scene,
    runs: Vec<Run>,
    pictures: BTreeMap<usize, Decoded>,
    /// families that were drawn with another font
    substituted: Vec<String>,
    /// items render2d refused (invalid geometry)
    refused: u32,
    ras: Raster,
}

/// the fonts as the display list's Measure, adding them to the scene as they are needed
struct Fonts<'a> {
    scene: &'a mut Scene,
    set: &'a mut FontSet,
    faces: BTreeMap<(String, bool, bool), Option<Resolved>>,
    /// registered font bytes -> its id in the scene and its text style
    loaded: Vec<(Rc<[u8]>, u32, FontId, TextStyleId)>,
    widths: BTreeMap<(String, bool, bool, String), f64>,
    substituted: Vec<String>,
}

impl Fonts<'_> {
    fn resolve(&mut self, face: &Face) -> Option<Resolved> {
        let key = (face.family.to_ascii_lowercase(), face.bold, face.italic);
        if let Some(r) = self.faces.get(&key) {
            return *r;
        }
        let r = self.resolve_new(face);
        self.faces.insert(key, r);
        r
    }

    fn resolve_new(&mut self, face: &Face) -> Option<Resolved> {
        let (f, synth, substituted) = self.set.pick(face.family, face.bold, face.italic)?;
        let (font, style) = match self.loaded.iter().find(|l| Rc::ptr_eq(&l.0, &f.data) && l.1 == f.index) {
            Some(l) => (l.2, l.3),
            None => {
                let font = self.scene.add_font(&f.data, f.index).ok()?;
                let style = self.scene.add_text_style(&TextStyle::new(font)).ok()?;
                self.loaded.push((f.data.clone(), f.index, font, style));
                (font, style)
            }
        };
        let mut width = 1.0;
        if substituted {
            if !self.substituted.iter().any(|s| s.eq_ignore_ascii_case(face.family)) {
                self.substituted.push(String::from(face.family));
            }
            if let Some(target) = sample_width(face.family, face.bold) {
                let own = self.raw_advance(font, style, SAMPLE);
                if own > 0.0 {
                    width = (target / own).clamp(0.7, 1.3);
                }
            }
        }
        Some(Resolved { font, style, width, oblique: if synth { SYNTHETIC_ITALIC } else { 0.0 } })
    }

    /// the advance of a line at an em of 1 (render2d's layout)
    fn raw_advance(&mut self, font: FontId, style: TextStyleId, text: &str) -> f64 {
        let Some(fm) = self.scene.fonts().metrics(font) else { return 0.0 };
        let styles = [TextStyle::new(font)];
        let mut t = Text::new(text, TextStyleId(0), [0.0, 0.0]);
        t.height = Some(fm.cap_height.max(1e-6));
        let _ = style;
        match render2d::layout_text(self.scene.fonts(), &styles, &t) {
            Ok(l) => l.lines.first().map_or(0.0, |x| x.width),
            Err(_) => 0.0,
        }
    }
}

impl Measure for Fonts<'_> {
    fn advance(&mut self, face: &Face, text: &str) -> f64 {
        let Some(r) = self.resolve(face) else { return render::ApproxMeasure.advance(face, text) };
        let key = (String::from(face.family), face.bold, face.italic, String::from(text));
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let w = self.raw_advance(r.font, r.style, text) * r.width;
        if self.widths.len() < 200_000 {
            self.widths.insert(key, w);
        }
        w
    }

    fn metrics(&mut self, face: &Face) -> (f64, f64) {
        match self.resolve(face).and_then(|r| self.scene.fonts().metrics(r.font)) {
            Some(m) => (m.ascender, -m.descender),
            None => (0.9, 0.25),
        }
    }
}

impl PageView {
    /// the page as a scene without a device
    pub fn new(doc: &Document, page: PageRef, fonts: &mut FontSet, opts: &Options) -> PageView {
        let mut scene = Scene::new_cpu();
        let (drawing, faces, substituted) = {
            let mut f = Fonts { scene: &mut scene, set: fonts, faces: BTreeMap::new(), loaded: Vec::new(), widths: BTreeMap::new(), substituted: Vec::new() };
            let d = render::build(doc, page, &mut f, opts);
            (d, f.faces, f.substituted)
        };
        let mut v = PageView { drawing, scene, runs: Vec::new(), pictures: BTreeMap::new(), substituted, refused: 0, ras: Raster::default() };
        v.fill_scene(doc, &faces);
        v
    }

    pub fn drawing(&self) -> &Drawing {
        &self.drawing
    }

    pub fn scene(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// the font families drawn with a substitute (TX-09: shown to the user)
    pub fn substitutions(&self) -> &[String] {
        &self.substituted
    }

    /// items render2d did not take (geometry it calls invalid)
    pub fn refused(&self) -> u32 {
        self.refused
    }

    fn fill_scene(&mut self, doc: &Document, faces: &BTreeMap<(String, bool, bool), Option<Resolved>>) {
        let mut styles: BTreeMap<StyleKey, render2d::StyleId> = BTreeMap::new();
        let mut layer: Option<LayerId> = None;
        let mut order = 0i32;
        let items = core::mem::take(&mut self.drawing.items);
        for (i, it) in items.iter().enumerate() {
            let direct = match &it.kind {
                ItemKind::Fill { paint: Paint::Gradient(_), .. } => true,
                ItemKind::Image(img) => {
                    let pic = image::bytes(doc, &img.source).map_or(Decoded::Placeholder("?"), |b| image::decode(&b));
                    self.pictures.insert(i, pic);
                    true
                }
                _ => false,
            };
            if direct {
                self.runs.push(Run::Direct(i));
                layer = None;
                continue;
            }
            let l = match layer {
                Some(l) => l,
                None => {
                    let Ok(l) = self.scene.add_layer(&Layer::new(order)) else { continue };
                    order += 1;
                    self.runs.push(Run::Layer(l));
                    layer = Some(l);
                    l
                }
            };
            let ok = match &it.kind {
                ItemKind::Fill { path, paint } => {
                    let style = match paint {
                        Paint::Solid(c) => StyleKey::fill(*c),
                        Paint::Hatch { color, lines } => StyleKey::hatch(*color, lines),
                        Paint::Gradient(_) => continue,
                    };
                    let Some(sid) = style_id(&mut self.scene, &mut styles, style) else { continue };
                    insert_fill(&mut self.scene, l, sid, path)
                }
                ItemKind::Stroke { path, stroke } => {
                    let Some(sid) = style_id(&mut self.scene, &mut styles, StyleKey::stroke(stroke)) else { continue };
                    insert_stroke(&mut self.scene, l, sid, path)
                }
                ItemKind::Text(t) => {
                    let ang = t.xform.angle();
                    let mut ok = true;
                    for run in &t.runs {
                        let key = (run.family.to_ascii_lowercase(), run.bold, run.italic);
                        let Some(Some(r)) = faces.get(&key) else { continue };
                        let Some(fm) = self.scene.fonts().metrics(r.font) else { continue };
                        let Some(sid) = style_id(&mut self.scene, &mut styles, StyleKey::fill(run.color)) else { continue };
                        let mut tx = Text::new(&run.text, r.style, t.xform.apply(run.at));
                        tx.height = Some(run.size * fm.cap_height);
                        tx.rotation = ang;
                        tx.width = Some(run.scale_x * r.width);
                        tx.oblique = Some(r.oblique);
                        if self.scene.insert(l, sid, &Shape::Text(tx)).is_err() {
                            ok = false;
                        }
                    }
                    ok
                }
                ItemKind::Image(_) => true,
            };
            if !ok {
                self.refused += 1;
            }
        }
        self.drawing.items = items;
        // every layer hidden but while it is drawn (a single one stays shown)
        if self.layer_count() > 1 {
            for r in &self.runs {
                if let Run::Layer(l) = r {
                    if let Ok(mut s) = self.scene.layer(*l) {
                        s.visible = false;
                        let _ = self.scene.set_layer(*l, &s);
                    }
                }
            }
        }
    }

    fn layer_count(&self) -> usize {
        self.runs.iter().filter(|r| matches!(r, Run::Layer(_))).count()
    }

    /// the camera that shows the whole page in a target of `size`, `margin` pixels around it
    pub fn fit(&self, size: (u32, u32), margin: f64) -> Camera {
        let (w, h) = (self.drawing.width.max(1e-6), self.drawing.height.max(1e-6));
        let sx = w / (size.0 as f64 - 2.0 * margin).max(1.0);
        let sy = h / (size.1 as f64 - 2.0 * margin).max(1.0);
        Camera::new([w * 0.5, h * 0.5], sx.max(sy))
    }

    /// the page on the CPU into the rectangle r (x, y, w, h relative to dc's origin) of sf,
    /// over what is there (the app paints the page's paper first)
    pub fn draw(&mut self, dc: &mut Dc, sf: &mut Surface, r: [i32; 4], cam: &Camera) {
        if r[2] <= 0 || r[3] <= 0 || self.scene.set_camera(cam).is_err() {
            return;
        }
        let many = self.layer_count() > 1;
        let size = (r[2] as u32, r[3] as u32);
        for k in 0..self.runs.len() {
            match self.runs[k] {
                Run::Layer(l) => {
                    if many {
                        set_visible(&mut self.scene, l, true);
                    }
                    self.scene.draw_cpu(dc, sf, r);
                    if many {
                        set_visible(&mut self.scene, l, false);
                    }
                }
                Run::Direct(i) => {
                    dc.clip_push(r[0], r[1], r[2], r[3]);
                    let at = [(r[0] + dc.ox) as f64, (r[1] + dc.oy) as f64];
                    let mut t = Target { dc, sf: &mut *sf, at, size, cam: *cam };
                    if t.dc.clip[2] > t.dc.clip[0] && t.dc.clip[3] > t.dc.clip[1] {
                        match &self.drawing.items[i].kind {
                            ItemKind::Fill { path, paint: Paint::Gradient(g) } => direct::fill_gradient(&mut t, &mut self.ras, path, g),
                            ItemKind::Image(img) => match self.pictures.get(&i) {
                                Some(Decoded::Picture(p)) => direct::picture(&mut t, &mut self.ras, p, &img.xform, &img.clip),
                                _ => direct::placeholder(&mut t, &mut self.ras, &img.xform, &img.clip),
                            },
                            _ => {}
                        }
                    }
                    dc.clip_pop();
                }
            }
        }
    }

    /// the page fitted into w x h pixels (a margin of 2 % around it) on the canvas grey, the
    /// page in its colour: an XRGB surface
    pub fn render(&mut self, w: u32, h: u32) -> Surface {
        let mut sf = Surface::new(w as i32, h as i32, FMT_XRGB);
        let mut dc = Dc::new(w as i32, h as i32);
        dc.begin(&sf);
        dc.clear(&mut sf, 0xFFEC_EEF1);
        let margin = (w.min(h) as f64 * 0.02).max(2.0);
        let cam = self.fit((w, h), margin);
        // the paper
        let (a, b) = (cam.to_screen([0.0, self.drawing.height], (w, h)), cam.to_screen([self.drawing.width, 0.0], (w, h)));
        let px = |v: f64| crate::floor(v + 0.5) as i32;
        let (x0, y0, x1, y1) = (px(a[0]), px(a[1]), px(b[0]), px(b[1]));
        dc.set_color(0xFF00_0000 | self.drawing.color);
        dc.fill(&mut sf, x0, y0, (x1 - x0).max(0), (y1 - y0).max(0));
        self.draw(&mut dc, &mut sf, [0, 0, w as i32, h as i32], &cam);
        dc.end();
        sf
    }
}

fn set_visible(s: &mut Scene, l: LayerId, v: bool) {
    if let Ok(mut x) = s.layer(l) {
        x.visible = v;
        let _ = s.set_layer(l, &x);
    }
}

/// what a render2d style is made of (deduplicated; numbers by their bits, so that the key orders)
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StyleKey {
    color: u32,
    width: u64,
    cap: u8,
    join: u8,
    dash: Vec<u64>,
    hatch: Vec<(u64, u64)>,
}

impl StyleKey {
    fn fill(color: u32) -> StyleKey {
        StyleKey { color, width: 0, cap: 0, join: 0, dash: Vec::new(), hatch: Vec::new() }
    }

    fn hatch(color: u32, lines: &[(f64, f64)]) -> StyleKey {
        StyleKey { hatch: lines.iter().take(render2d::HATCH_MAX).map(|&(a, s)| (a.to_bits(), s.to_bits())).collect(), ..StyleKey::fill(color) }
    }

    fn stroke(s: &Stroke) -> StyleKey {
        let cap = match s.cap {
            render::Cap::Round => 0,
            render::Cap::Butt => 1,
            render::Cap::Square => 2,
        };
        let join = match s.join {
            render::Join::Round => 0,
            render::Join::Miter => 1,
            render::Join::Bevel => 2,
        };
        let dash = s.dash.iter().take(render2d::DASH_MAX).map(|v| v.to_bits()).collect();
        StyleKey { color: s.color, width: s.width.to_bits(), cap, join, dash, hatch: Vec::new() }
    }

    fn style(&self) -> Style {
        let mut s = Style::new(self.color);
        let width = f64::from_bits(self.width);
        if width > 0.0 {
            s.width = width as f32;
            s.unit = Unit::Drawing;
        }
        s.cap = match self.cap {
            0 => Cap::Round,
            1 => Cap::Butt,
            _ => Cap::Square,
        };
        s.join = match self.join {
            0 => Join::Round,
            1 => Join::Miter,
            _ => Join::Bevel,
        };
        let dash: Vec<f64> = self.dash.iter().map(|v| f64::from_bits(*v)).collect();
        if !dash.is_empty() && dash.iter().sum::<f64>() > 0.0 {
            let pat: Vec<f32> = dash.iter().map(|v| *v as f32).collect();
            s = s.dashed(&pat, Unit::Drawing, 0.0);
        }
        if !self.hatch.is_empty() {
            let fams: Vec<HatchLine> = self.hatch.iter().map(|&(a, sp)| HatchLine::new(f64::from_bits(a), f64::from_bits(sp))).collect();
            s = s.hatched(&fams, 1.0);
        }
        s
    }
}

fn style_id(scene: &mut Scene, styles: &mut BTreeMap<StyleKey, render2d::StyleId>, key: StyleKey) -> Option<render2d::StyleId> {
    if let Some(id) = styles.get(&key) {
        return Some(*id);
    }
    let id = scene.add_style(&key.style()).ok()?;
    styles.insert(key, id);
    Some(id)
}

/// a sub-path as render2d's points: a polyline when it is all straight, else cubic pieces (a
/// straight piece as a cubic along it); closed drops a last point equal to the first
fn points(sub: &Sub) -> (Vec<[f64; 2]>, bool) {
    let straight = sub.segs.iter().all(|s| matches!(s, Seg::Line(_)));
    let mut pts = alloc::vec![sub.start];
    let mut prev = sub.start;
    for s in &sub.segs {
        match *s {
            Seg::Line(p) => {
                if straight {
                    pts.push(p);
                } else {
                    pts.push(lerp(prev, p, 1.0 / 3.0));
                    pts.push(lerp(prev, p, 2.0 / 3.0));
                    pts.push(p);
                }
            }
            Seg::Cubic(a, b, p) => {
                pts.push(a);
                pts.push(b);
                pts.push(p);
            }
        }
        prev = s.end();
    }
    if straight && sub.closed && pts.len() > 2 && pts.first() == pts.last() {
        pts.pop();
    }
    (pts, straight)
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn insert_fill(scene: &mut Scene, l: LayerId, sid: render2d::StyleId, path: &Path) -> bool {
    let subs: Vec<(Vec<[f64; 2]>, bool)> = path.subs.iter().filter(|s| !s.segs.is_empty()).map(points).collect();
    let shapes: Vec<Shape> = subs.iter().map(|(p, straight)| if *straight { Shape::Polyline { points: p, closed: true } } else { Shape::Cubic { points: p, closed: true } }).collect();
    if shapes.is_empty() {
        return true;
    }
    let rings: Vec<&[Shape]> = shapes.iter().map(core::slice::from_ref).collect();
    scene.insert(l, sid, &Shape::Fill { rings: &rings }).is_ok()
}

fn insert_stroke(scene: &mut Scene, l: LayerId, sid: render2d::StyleId, path: &Path) -> bool {
    let mut ok = true;
    for sub in path.subs.iter().filter(|s| !s.segs.is_empty()) {
        let (p, straight) = points(sub);
        let shape = if straight { Shape::Polyline { points: &p, closed: sub.closed } } else { Shape::Cubic { points: &p, closed: sub.closed } };
        ok &= scene.insert(l, sid, &shape).is_ok();
    }
    ok
}
