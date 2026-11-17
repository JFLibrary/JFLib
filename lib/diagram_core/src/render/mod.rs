// render -- a page as a display list (JD4, milestone "viewer"): what is drawn where, in which
// order, with which look; the drawing itself is someone else's (diagram_view on render2d and
// gfx, later the exports of diagram_io)
//
//   render::build(&doc, page, &mut measure, &Options) -> Drawing
//   Drawing { width, height, scale, items, fonts, notes }   in paper inches, y up, the origin at
//                                   the page's bottom left corner; items in painter's order
//   drawing.query(rect, &mut out)   the items whose boxes touch a rectangle (a grid over the
//                                   page: culling and hit tests without looking at every item)
//   Item { shape, top, bbox, role, kind }   role: Shape | Arrow | Shadow | Text; kind: Fill { path, paint } | Stroke { path, stroke } |
//                                   Text(TextItem) | Image(ImageItem)
//   Paint::Solid | Hatch | Gradient, Stroke { color, width, dash, cap, join }
//   Measure / Face / ApproxMeasure  (text.rs) what the text layout asks of fonts
//   ThemeInfo                       (theme.rs) colours and fonts of a theme part
//   style::{dash, fill_pattern, arrow, PALETTE}   the tables behind lines and fills
//
// Everything comes from the cached values (decision: the viewer shows what the file cached,
// the formula engine of JD6 is not needed).  Per page: its background page first (and theirs),
// then its shapes in z-order; a group draws its children and its own geometry and text behind
// them (DisplayMode 1) or in front (2, the default; 0 hides the group's own geometry).  Per
// shape: the shadow (ShdwPattern above 0, offset by ShapeShdwOffsetX/Y), the fill of all
// closed sub-paths of the Geometry sections without NoFill (even-odd), a Foreign shape's
// picture, the lines of the sections without NoLine with their line ends, the text.  Not
// drawn: deleted subshapes (Del, ID 4294967295, no Type), guides, sections with NoShow,
// shapes whose layers are all hidden.  A layer's colour (Color not 255) colours its members.
//
// Values Visio caches as "Themed" (the Theme style and a few shapes) are resolved against the
// page's theme: the theme part whose schemeEnum the page's ThemeIndex names (colours by
// ColorSchemeIndex, fonts by FontSchemeIndex), the QuickStyle colour indices of the shape, the
// variation of VariationColorIndex; the rest falls back to plain values (solid, 0.75 pt,
// no gradient, no shadow).  On a page with a drawing scale, geometry is in drawing units and
// line weights, text sizes and margins are in page units: the display list is in paper
// inches (geometry times PageScale / DrawingScale).

mod index;
pub mod style;
mod text;
mod theme;

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::geometry::{round_corners, Path, Seg, Sub, Xform};
use crate::names;
use crate::shape::{ForeignPayload, ShapeKind};
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::strings::Sym;

pub use index::Grid;
pub use text::{ApproxMeasure, Face, Measure, TextItem, TextRun};
pub use theme::{no_theme_color, ThemeInfo};

/// what build draws
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// draw the page's background pages under it
    pub background: bool,
    /// draw shapes on hidden layers too
    pub hidden_layers: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { background: true, hidden_layers: false }
    }
}

/// a page as items to draw
#[derive(Clone, Debug, Default)]
pub struct Drawing {
    pub page: Option<PageRef>,
    /// the page's size in paper inches
    pub width: f64,
    pub height: f64,
    /// paper inches per drawing inch (PageScale / DrawingScale)
    pub scale: f64,
    /// the page's colour (0xRRGGBB)
    pub color: u32,
    pub items: Vec<Item>,
    /// the font families the texts ask for
    pub fonts: BTreeSet<String>,
    /// what was left out or drawn only roughly, with counts
    pub notes: Vec<(Note, u32)>,
    grid: Grid,
}

impl Drawing {
    /// the items whose boxes touch `rect` (x0, y0, x1, y1, paper inches), ascending
    pub fn query(&self, rect: [f64; 4], out: &mut Vec<u32>) {
        self.grid.query(rect, out);
    }

    /// the box of all items (None: nothing to draw)
    pub fn bounds(&self) -> Option<[f64; 4]> {
        self.items.iter().map(|i| i.bbox).reduce(union)
    }

    fn note(&mut self, n: Note) {
        match self.notes.iter_mut().find(|(k, _)| *k == n) {
            Some((_, c)) => *c += 1,
            None => self.notes.push((n, 1)),
        }
    }
}

/// what the viewer does not draw (yet), or draws only roughly
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Note {
    /// a picture in a format the drawing side does not decode (EMF, WMF, OLE): a placeholder
    Metafile,
    /// a Foreign shape without picture data
    NoPicture,
    /// a double or triple line (CompoundType): drawn single
    CompoundLine,
    /// a line gradient: drawn in the first stop's colour
    LineGradient,
    /// 3D, bevel, glow, reflection, soft edges, sketch: left out
    Effects,
    /// a line pattern of a master (LinePattern above 23): drawn solid
    CustomLinePattern,
    /// a guide: not drawn
    Guide,
    /// a shadow drawn without blur, oblique type or scale
    SimpleShadow,
}

/// one thing to draw
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// the shape it belongs to
    pub shape: ShapeRef,
    /// that shape's top-level group (or itself): what a click selects
    pub top: ShapeRef,
    /// its box (paper inches), lines with half their width
    pub bbox: [f64; 4],
    /// what it is part of
    pub role: Role,
    pub kind: ItemKind,
}

/// what an item draws of its shape
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// the geometry, its fill and its lines, or the picture
    Shape,
    /// a line end
    Arrow,
    Shadow,
    /// the text, its background, its underlines
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// closed sub-paths filled even-odd
    Fill { path: Path, paint: Paint },
    Stroke { path: Path, stroke: Stroke },
    Text(Box<TextItem>),
    Image(Box<ImageItem>),
}

/// how an area is filled
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// 0xAARRGGBB
    Solid(u32),
    /// parallel lines in a colour (the background is an item of its own before it): angle
    /// (radians, counter-clockwise) and spacing (inches) per family
    Hatch { color: u32, lines: Vec<(f64, f64)> },
    Gradient(Box<Gradient>),
}

/// a gradient: linear from `from` (position 0) to `to` (1), or radial around `from` with the
/// radius |to - from|; stops ascending
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub radial: bool,
    pub from: [f64; 2],
    pub to: [f64; 2],
    /// (position 0..1, 0xAARRGGBB)
    pub stops: Vec<(f64, u32)>,
}

impl Gradient {
    /// the colour at position t (0 .. 1), straight alpha
    pub fn at(&self, t: f64) -> u32 {
        let s = &self.stops;
        let Some(first) = s.first() else { return 0 };
        if t <= first.0 {
            return first.1;
        }
        for w in s.windows(2) {
            if t <= w[1].0 {
                let d = w[1].0 - w[0].0;
                let k = if d > 0.0 { (t - w[0].0) / d } else { 1.0 };
                return mix(w[0].1, w[1].1, k);
            }
        }
        s.last().map_or(0, |l| l.1)
    }
}

/// how a line ends and turns
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cap {
    Round,
    Butt,
    Square,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Join {
    Round,
    Miter,
    Bevel,
}

/// a line's look
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// 0xAARRGGBB
    pub color: u32,
    /// inches; 0: a hairline (one pixel)
    pub width: f64,
    /// lengths on and off in inches (empty: solid)
    pub dash: Vec<f64>,
    pub cap: Cap,
    pub join: Join,
}

/// a picture: its pixels' rectangle (0..1 by 0..1 of the picture, y up) mapped to the page,
/// clipped to the shape's box
#[derive(Clone, Debug, PartialEq)]
pub struct ImageItem {
    pub xform: Xform,
    /// the shape's box on the page (the picture is cut there)
    pub clip: [[f64; 2]; 4],
    pub source: ImageSource,
    /// what ForeignData says it is (CompressionType, ForeignType), lower case: "png", "jpeg",
    /// "emf", "wmf", "bmp", "gif", "tiff" ... ("" unknown); the drawing side looks at the bytes
    pub format: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImageSource {
    /// a part of the package: Document::parts, by name
    Part(Sym),
    /// base64 as the file wrote it
    Inline(Box<str>),
}

/// the display list of a page
pub fn build(doc: &Document, page: PageRef, measure: &mut dyn Measure, opts: &Options) -> Drawing {
    let mut b = Builder::new(doc, measure, *opts);
    b.drawing.page = Some(page);
    let ps = SheetRef::Page(page);
    let (w, h) = (b.num_sym(ps, names::PAGE_WIDTH).unwrap_or(8.5), b.num_sym(ps, names::PAGE_HEIGHT).unwrap_or(11.0));
    let scale = page_scale(&b, ps);
    b.drawing.width = w * scale;
    b.drawing.height = h * scale;
    b.drawing.scale = scale;
    b.drawing.color = 0xFFFFFF;
    // background pages first, the farthest one first
    let mut chain = alloc::vec![page];
    if opts.background {
        let mut cur = page;
        while let Some(bg) = doc.background_of(cur) {
            if chain.contains(&bg) || chain.len() > 64 {
                break;
            }
            chain.push(bg);
            cur = bg;
        }
    }
    for &p in chain.iter().rev() {
        b.page(p);
    }
    let mut d = core::mem::take(&mut b.drawing);
    d.items = core::mem::take(&mut b.items);
    d.fonts = core::mem::take(&mut b.fonts);
    let bounds = d.bounds().map_or([0.0, 0.0, d.width, d.height], |bb| union(bb, [0.0, 0.0, d.width, d.height]));
    d.grid = Grid::build(&d.items, bounds);
    d
}

fn page_scale(b: &Builder, ps: SheetRef) -> f64 {
    let page = b.num_sym(ps, names::PAGE_SCALE).filter(|v| *v > 0.0 && v.is_finite());
    let drawing = b.num_sym(ps, names::DRAWING_SCALE).filter(|v| *v > 0.0 && v.is_finite());
    match (page, drawing) {
        (Some(p), Some(d)) if (p / d).is_finite() && p / d > 1e-9 => p / d,
        _ => 1.0,
    }
}

struct Layer {
    visible: bool,
    color: Option<u32>,
}

pub(crate) struct Builder<'d, 'm> {
    doc: &'d Document,
    measure: &'m mut dyn Measure,
    opts: Options,
    drawing: Drawing,
    items: Vec<Item>,
    fonts: BTreeSet<String>,
    /// themes parsed once (index of doc.themes)
    themes: Vec<Option<ThemeInfo>>,
    /// the page's theme and variation
    theme: Option<ThemeInfo>,
    variation: usize,
    layers: Vec<(u32, Layer)>,
    /// paper inches per drawing inch of the page being built
    scale: f64,
    /// the top-level shape being built
    top: ShapeRef,
    /// the page's shadow offset (inches)
    shadow: [f64; 2],
}

impl<'d, 'm> Builder<'d, 'm> {
    fn new(doc: &'d Document, measure: &'m mut dyn Measure, opts: Options) -> Builder<'d, 'm> {
        let themes = doc.themes.iter().map(|t| ThemeInfo::parse(&t.data)).collect();
        Builder {
            doc,
            measure,
            opts,
            drawing: Drawing::default(),
            items: Vec::new(),
            fonts: BTreeSet::new(),
            themes,
            theme: None,
            variation: 0,
            layers: Vec::new(),
            scale: 1.0,
            top: ShapeRef { owner: Owner::Page(PageRef(0)), slot: 0 },
            shadow: [0.125, -0.125],
        }
    }

    // ---- cells

    fn sym(&self, name: &str) -> Option<Sym> {
        self.doc.sym(name)
    }

    fn num_sym(&self, sheet: SheetRef, c: Sym) -> Option<f64> {
        self.doc.resolve(sheet, &CellAddr::single(c)).num().filter(|v| v.is_finite())
    }

    fn num(&self, sheet: SheetRef, name: &str) -> Option<f64> {
        self.num_sym(sheet, self.sym(name)?)
    }

    fn val(&self, sheet: SheetRef, name: &str) -> Value {
        match self.sym(name) {
            Some(c) => self.doc.resolve(sheet, &CellAddr::single(c)).value(),
            None => Value::Absent,
        }
    }

    fn row_val(&self, sheet: SheetRef, sec: SecKey, key: RowKey, name: &str) -> Value {
        match self.sym(name) {
            Some(c) => self.doc.resolve(sheet, &CellAddr::row(sec, key, c)).value(),
            None => Value::Absent,
        }
    }

    fn row_num(&self, sheet: SheetRef, sec: SecKey, key: RowKey, name: &str) -> Option<f64> {
        self.row_val(sheet, sec, key, name).num().filter(|v| v.is_finite())
    }

    fn text_cell(&self, sheet: SheetRef, sec: SecKey, key: RowKey, name: &str) -> Option<String> {
        match self.row_val(sheet, sec, key, name) {
            Value::Text(s) => Some(String::from(self.doc.str(s))),
            _ => None,
        }
    }

    /// whether a single cell's value is "Themed"
    fn themed(&self, sheet: SheetRef, name: &str) -> bool {
        matches!(self.val(sheet, name), Value::Text(s) if self.doc.str(s).eq_ignore_ascii_case("Themed"))
    }

    /// a colour index: the document's Colors table, else Visio's 24 colours
    fn palette(&self, i: u32) -> u32 {
        let doc = self.doc;
        doc.colors.iter().find(|c| c.ix == i).and_then(|c| parse_color(doc.str(c.rgb))).unwrap_or_else(|| style::PALETTE.get(i as usize).copied().unwrap_or(0))
    }

    /// a colour cell (single, or of a row): "#RRGGBB", an index, "Themed" (the QuickStyle
    /// colour `quick` names, else `quick_default`), else `fallback`; 0xRRGGBB
    fn color_cell(&self, sheet: SheetRef, row: Option<(SecKey, RowKey)>, name: &str, quick_default: u32, fallback: u32) -> u32 {
        let v = match row {
            Some((sec, key)) => self.row_val(sheet, sec, key, name),
            None => self.val(sheet, name),
        };
        match v {
            Value::Num(n) if n >= 0.0 => self.palette(n as u32),
            Value::Text(s) => {
                let t = self.doc.str(s);
                if t.eq_ignore_ascii_case("Themed") {
                    let q = match name {
                        "LineColor" => "QuickStyleLineColor",
                        "FillForegnd" | "FillBkgnd" => "QuickStyleFillColor",
                        "ShdwForegnd" => "QuickStyleShadowColor",
                        _ => "QuickStyleFontColor",
                    };
                    self.theme_color(sheet, q, quick_default)
                } else {
                    parse_color(t).unwrap_or(fallback)
                }
            }
            _ => fallback,
        }
    }

    fn theme_color(&self, sheet: SheetRef, quick: &str, default: u32) -> u32 {
        let idx = self.num(sheet, quick).map_or(default, |v| v as u32);
        let var = self.num(sheet, "VariationColorIndex").filter(|v| *v < 4.0).map_or(self.variation, |v| v as usize);
        match &self.theme {
            Some(t) => t.quick_color(idx, var).unwrap_or_else(|| t.color(4)),
            None => no_theme_color(idx),
        }
    }

    // ---- pages and shapes

    fn page(&mut self, page: PageRef) {
        let doc = self.doc;
        let ps = SheetRef::Page(page);
        self.scale = page_scale(self, ps);
        // the theme: by relationship, else the schemeEnum of ThemeIndex (colours of
        // ColorSchemeIndex, fonts of FontSchemeIndex)
        self.theme = self.page_theme(page);
        self.variation = self.num(ps, "VariationColorIndex").filter(|v| (0.0..4.0).contains(v)).map_or(0, |v| v as usize);
        self.shadow = [self.num(ps, "ShdwOffsetX").unwrap_or(0.125), self.num(ps, "ShdwOffsetY").unwrap_or(-0.125)];
        self.layers = doc
            .layers(page)
            .into_iter()
            .filter_map(|l| {
                let RowKey::Index(ix) = l.key() else { return None };
                let visible = l.num("Visible").is_none_or(|v| v != 0.0);
                let color = match l.get("Color").value() {
                    Value::Num(n) if n >= 0.0 && n != 255.0 => Some(self.palette(n as u32)),
                    Value::Text(s) => parse_color(doc.str(s)),
                    _ => None,
                }
                .map(|c| with_alpha(c, l.num("ColorTrans").unwrap_or(0.0)));
                Some((ix, Layer { visible, color }))
            })
            .collect();
        let m = Xform::scale(self.scale, self.scale);
        let tops: Vec<ShapeRef> = doc.top_shapes(Owner::Page(page)).collect();
        for r in tops {
            self.top = r;
            self.shape(r, &m, 0);
        }
    }

    fn page_theme(&self, page: PageRef) -> Option<ThemeInfo> {
        let doc = self.doc;
        if let Some(t) = doc.page(page).and_then(|p| p.theme).and_then(|i| self.themes.get(i as usize).cloned().flatten()) {
            return Some(t);
        }
        let ps = SheetRef::Page(page);
        let idx = self.num(ps, "ThemeIndex").map(|v| v as u32)?;
        let find = |id: u32, slot: usize| {
            self.themes.iter().flatten().find(|t| t.scheme_ids[slot] == Some(id)).or_else(|| self.themes.iter().flatten().find(|t| t.has_scheme(id)))
        };
        let mut t = find(idx, 3)?.clone();
        if let Some(c) = self.num(ps, "ColorSchemeIndex").and_then(|v| find(v as u32, 0)) {
            t.colors = c.colors;
            t.variations = c.variations.clone();
            t.background = c.background.or(t.background);
        }
        if let Some(f) = self.num(ps, "FontSchemeIndex").and_then(|v| find(v as u32, 1)) {
            t.major_font = f.major_font.clone();
            t.minor_font = f.minor_font.clone();
        }
        Some(t)
    }

    /// whether a shape is shown by its layers, and the colour a layer gives it
    fn layer_state(&self, r: ShapeRef) -> (bool, Option<u32>) {
        let members = self.doc.layer_members(r);
        if members.is_empty() || self.layers.is_empty() {
            return (true, None);
        }
        let mut visible = false;
        let mut known = false;
        let mut color = None;
        for m in members {
            match self.layers.iter().find(|(ix, _)| *ix == m) {
                Some((_, l)) => {
                    known = true;
                    visible |= l.visible;
                    if color.is_none() {
                        color = l.color;
                    }
                }
                None => visible = true,
            }
        }
        (visible || !known || self.opts.hidden_layers, color)
    }

    fn shape(&mut self, r: ShapeRef, parent: &Xform, depth: u32) {
        let doc = self.doc;
        let Some(s) = doc.shape(r) else { return };
        let header = s.header();
        if header.del || depth > 256 {
            return;
        }
        match header.kind {
            ShapeKind::Unspecified => return,
            ShapeKind::Guide => {
                self.drawing.note(Note::Guide);
                return;
            }
            _ => {}
        }
        let sheet = SheetRef::Shape(r);
        let m = doc.local_xform(sheet).then(parent);
        let (visible, layer_color) = self.layer_state(r);
        let children: Vec<ShapeRef> = s.children().collect();
        if header.kind == ShapeKind::Group || !children.is_empty() {
            let mode = self.num(sheet, "DisplayMode").unwrap_or(2.0) as u32;
            if visible && mode == 1 {
                self.own(r, &m, layer_color, true);
            }
            for c in children {
                self.shape(c, &m, depth + 1);
            }
            if visible && mode != 1 {
                self.own(r, &m, layer_color, mode != 0);
            }
        } else if visible {
            self.own(r, &m, layer_color, true);
        }
    }

    /// a shape's own drawing: shadow, fill, picture, lines, line ends, text
    fn own(&mut self, r: ShapeRef, m: &Xform, layer_color: Option<u32>, geometry: bool) {
        let doc = self.doc;
        let sheet = SheetRef::Shape(r);
        let w = self.num_sym(sheet, names::WIDTH).unwrap_or(0.0);
        let h = self.num_sym(sheet, names::HEIGHT).unwrap_or(0.0);
        if geometry {
            let sections = doc.geometry_paths(sheet, w, h);
            let mut fill = Path::default();
            let mut line = Path::default();
            for g in sections.iter().filter(|g| !g.no_show) {
                if !g.no_fill {
                    fill.subs.extend(g.path.subs.iter().filter(|s| s.closed).cloned());
                }
                if !g.no_line {
                    line.subs.extend(g.path.subs.iter().cloned());
                }
            }
            let rounding = self.num_sym(sheet, names::ROUNDING).unwrap_or(0.0);
            if rounding > 0.0 {
                fill = round_corners(&fill, rounding);
                line = round_corners(&line, rounding);
            }
            let fill = fill.map(m);
            let line = line.map(m);
            if self.effects(sheet) {
                self.drawing.note(Note::Effects);
            }
            self.shadow_of(r, sheet, &fill, &line);
            if !fill.is_empty() {
                self.fill(r, sheet, &fill, m, w, h, layer_color);
            }
            if doc.shape(r).is_some_and(|s| s.header().kind == ShapeKind::Foreign) {
                self.picture(r, sheet, m, w, h);
            }
            if !line.is_empty() {
                self.lines(r, sheet, line, layer_color);
            }
        }
        self.text(r, m, w, h, layer_color);
    }

    fn effects(&self, sheet: SheetRef) -> bool {
        ["BevelTopType", "GlowSize", "ReflectionSize", "SoftEdgesSize", "SketchEnabled"].iter().any(|c| self.num(sheet, c).is_some_and(|v| v > 0.0))
    }

    fn shadow_of(&mut self, r: ShapeRef, sheet: SheetRef, fill: &Path, line: &Path) {
        let pattern = self.num(sheet, "ShdwPattern").unwrap_or(0.0);
        if pattern <= 0.0 || (fill.is_empty() && line.is_empty()) {
            return;
        }
        let kind = self.num(sheet, "ShapeShdwType").unwrap_or(0.0);
        let mut off = [self.num(sheet, "ShapeShdwOffsetX").unwrap_or(0.0), self.num(sheet, "ShapeShdwOffsetY").unwrap_or(0.0)];
        if kind == 0.0 || (off[0] == 0.0 && off[1] == 0.0) {
            off = self.shadow;
        }
        if kind > 1.0 || self.num(sheet, "ShapeShdwBlur").is_some_and(|v| v > 0.0) || self.num(sheet, "ShapeShdwScaleFactor").is_some_and(|v| v != 1.0 && v != 0.0) {
            self.drawing.note(Note::SimpleShadow);
        }
        let color = self.color_cell(sheet, None, "ShdwForegnd", 0, 0x000000);
        let color = with_alpha(color, self.num(sheet, "ShdwForegndTrans").unwrap_or(0.0));
        let t = Xform::translate(off[0], off[1]);
        if !fill.is_empty() {
            self.push_fill(r, Role::Shadow, fill.map(&t), Paint::Solid(color));
        } else {
            let st = self.line_style(sheet, None);
            if let Some(mut st) = st {
                st.color = color;
                self.push_stroke(r, Role::Shadow, line.map(&t), st);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fill(&mut self, r: ShapeRef, sheet: SheetRef, fill: &Path, m: &Xform, w: f64, h: f64, layer_color: Option<u32>) {
        let pattern = if self.themed(sheet, "FillPattern") { 1 } else { self.num(sheet, "FillPattern").unwrap_or(1.0) as u32 };
        let mut fg = self.color_cell(sheet, None, "FillForegnd", 100, 0xFFFFFF);
        let mut bg = self.color_cell(sheet, None, "FillBkgnd", 100, 0xFFFFFF);
        if let Some(c) = layer_color {
            fg = c & 0xFF_FFFF;
            bg = c & 0xFF_FFFF;
        }
        let fg = with_alpha(fg, self.num(sheet, "FillForegndTrans").unwrap_or(0.0));
        let bg = with_alpha(bg, self.num(sheet, "FillBkgndTrans").unwrap_or(0.0));
        // the shape's box on the paper, for gradients
        let local_box = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
        if self.num(sheet, "FillGradientEnabled").is_some_and(|v| v != 0.0) && pattern != 0 {
            if let Some(g) = self.gradient(sheet, m, &local_box, fill) {
                self.push_fill(r, Role::Shape, fill.clone(), Paint::Gradient(Box::new(g)));
                return;
            }
        }
        match style::fill_pattern(pattern) {
            style::FillKind::None => {}
            style::FillKind::Solid => {
                if fg >> 24 != 0 {
                    self.push_fill(r, Role::Shape, fill.clone(), Paint::Solid(fg));
                }
            }
            style::FillKind::Hatch(fams) => {
                if bg >> 24 != 0 {
                    self.push_fill(r, Role::Shape, fill.clone(), Paint::Solid(bg));
                }
                let lines = fams.iter().map(|&(deg, sp)| (deg * core::f64::consts::PI / 180.0, sp)).collect();
                self.push_fill(r, Role::Shape, fill.clone(), Paint::Hatch { color: fg, lines });
            }
            style::FillKind::Legacy(l) => {
                let bb = bbox_of(&local_box.map(|p| m.apply(p)));
                let g = match l {
                    style::Legacy::Linear(deg, mirrored) => {
                        let (s, c) = crate::geometry::sincos(deg * core::f64::consts::PI / 180.0);
                        let (from, to) = axis_through(bb, [c, s]);
                        let stops = if mirrored { alloc::vec![(0.0, bg), (0.5, fg), (1.0, bg)] } else { alloc::vec![(0.0, fg), (1.0, bg)] };
                        Gradient { radial: false, from, to, stops }
                    }
                    style::Legacy::Radial(fx, fy) => {
                        let c = [bb[0] + (bb[2] - bb[0]) * fx, bb[1] + (bb[3] - bb[1]) * fy];
                        let r = [[bb[0], bb[1]], [bb[2], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]]].iter().map(|p| dist(*p, c)).fold(0.0, f64::max);
                        Gradient { radial: true, from: c, to: [c[0] + r, c[1]], stops: alloc::vec![(0.0, fg), (1.0, bg)] }
                    }
                };
                self.push_fill(r, Role::Shape, fill.clone(), Paint::Gradient(Box::new(g)));
            }
        }
    }

    /// the FillGradient section's stops along FillGradientDir / FillGradientAngle
    fn gradient(&self, sheet: SheetRef, m: &Xform, local_box: &[[f64; 2]; 4], fill: &Path) -> Option<Gradient> {
        let sec = SecKey::one(self.sym("FillGradient")?);
        let mut stops = Vec::new();
        for row in self.doc.rows(sheet, sec) {
            let pos = self.row_num(sheet, sec, row.key, "GradientStopPosition").unwrap_or(0.0).clamp(0.0, 1.0);
            let c = self.color_cell(sheet, Some((sec, row.key)), "GradientStopColor", 100, 0xFFFFFF);
            let trans = self.row_num(sheet, sec, row.key, "GradientStopColorTrans").unwrap_or(0.0);
            stops.push((pos, with_alpha(c, trans)));
        }
        if stops.is_empty() {
            return None;
        }
        stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        let dir = self.num(sheet, "FillGradientDir").unwrap_or(0.0) as u32;
        let angle = self.num(sheet, "FillGradientAngle").unwrap_or(core::f64::consts::FRAC_PI_2);
        let rotate = self.num(sheet, "RotateGradientWithShape").is_none_or(|v| v != 0.0);
        // the box the gradient spans: the shape's, turned with it or upright on the page
        let pts = local_box.map(|p| m.apply(p));
        let bb = if rotate { bbox_of(&pts) } else { fill.hull().unwrap_or(bbox_of(&pts)) };
        if dir == 0 {
            // DrawingML: the angle turns clockwise from left -> right on the screen (y down)
            let (s, c) = crate::geometry::sincos(angle);
            let mut d = [c, -s];
            if rotate {
                d = m.apply_vec(d);
                let l = dist(d, [0.0, 0.0]);
                if l > 0.0 {
                    d = [d[0] / l, d[1] / l];
                }
            }
            let (from, to) = axis_through(bbox_of(&pts), d);
            Some(Gradient { radial: false, from, to, stops })
        } else {
            // radial (and the rectangular and path kinds, drawn radial) from the centre or a corner
            let f = match dir {
                2 => [0.0, 1.0],
                3 => [1.0, 1.0],
                4 => [0.0, 0.0],
                5 => [1.0, 0.0],
                _ => [0.5, 0.5],
            };
            let c = [bb[0] + (bb[2] - bb[0]) * f[0], bb[1] + (bb[3] - bb[1]) * f[1]];
            let r = pts.iter().map(|p| dist(*p, c)).fold(0.0, f64::max);
            Some(Gradient { radial: true, from: c, to: [c[0] + r, c[1]], stops })
        }
    }

    /// a Foreign shape's picture in its box (ImgOffsetX/Y, ImgWidth/Height), cut at the shape's box
    fn picture(&mut self, r: ShapeRef, sheet: SheetRef, m: &Xform, w: f64, h: f64) {
        let doc = self.doc;
        // the picture data: the shape's own, else its master shape's
        let mut cur = Some(r);
        let mut fd = None;
        let mut depth = 0;
        while let (Some(c), true) = (cur, depth < 64) {
            if let Some(f) = doc.tree(c.owner).and_then(|t| t.get(c.slot)).and_then(|d| d.foreign.as_deref()) {
                fd = Some(f);
                break;
            }
            cur = doc.master_shape_of(c);
            depth += 1;
        }
        let Some(fd) = fd else {
            self.drawing.note(Note::NoPicture);
            return;
        };
        let source = match &fd.payload {
            ForeignPayload::Part(p) => ImageSource::Part(*p),
            ForeignPayload::Inline(b) => ImageSource::Inline(b.clone()),
            ForeignPayload::None => {
                self.drawing.note(Note::NoPicture);
                return;
            }
        };
        let attr = |name: &str| fd.attrs.iter().find(|(k, _)| doc.str(*k) == name).map(|(_, v)| doc.str(*v).to_ascii_lowercase());
        let format = match (attr("CompressionType"), attr("ForeignType")) {
            (Some(c), _) if !c.is_empty() && c != "none" => c,
            (_, Some(t)) if t == "enhmetafile" => String::from("emf"),
            (_, Some(t)) if t == "metafile" => String::from("wmf"),
            (_, Some(t)) if t == "bitmap" => String::from("bmp"),
            _ => String::new(),
        };
        if matches!(format.as_str(), "emf" | "wmf") || attr("ForeignType").is_some_and(|t| t == "object") {
            self.drawing.note(Note::Metafile);
        }
        let ox = self.num(sheet, "ImgOffsetX").unwrap_or(0.0);
        let oy = self.num(sheet, "ImgOffsetY").unwrap_or(0.0);
        let iw = self.num(sheet, "ImgWidth").filter(|v| *v > 0.0).unwrap_or(w);
        let ih = self.num(sheet, "ImgHeight").filter(|v| *v > 0.0).unwrap_or(h);
        let xform = Xform::scale(iw, ih).then(&Xform::translate(ox, oy)).then(m);
        let clip = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|p| m.apply(p));
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|p| xform.apply(p));
        let bb = intersect(bbox_of(&corners), bbox_of(&clip));
        self.items.push(Item { shape: r, top: self.top, bbox: bb, role: Role::Shape, kind: ItemKind::Image(Box::new(ImageItem { xform, clip, source, format })) });
    }

    /// LinePattern, LineWeight, LineColor (+Trans), LineCap; None: no line
    fn line_style(&mut self, sheet: SheetRef, layer_color: Option<u32>) -> Option<Stroke> {
        let pattern = if self.themed(sheet, "LinePattern") { 1 } else { self.num(sheet, "LinePattern").unwrap_or(1.0) as u32 };
        if pattern == 0 {
            return None;
        }
        let weight = if self.themed(sheet, "LineWeight") {
            let k = self.num(sheet, "QuickStyleLineMatrix").map_or(1, |v| v as usize).max(1);
            self.theme.as_ref().and_then(|t| t.line_widths.get(k - 1).copied()).unwrap_or(0.75 / 72.0)
        } else {
            self.num_sym(sheet, names::LINE_WEIGHT).unwrap_or(0.75 / 72.0).max(0.0)
        };
        let mut color = self.color_cell(sheet, None, "LineColor", 100, 0x000000);
        if let Some(c) = layer_color {
            color = c & 0xFF_FFFF;
        }
        let color = with_alpha(color, self.num(sheet, "LineColorTrans").unwrap_or(0.0));
        let cap = match self.num(sheet, "LineCap").unwrap_or(0.0) as u32 {
            1 => Cap::Butt,
            2 => Cap::Square,
            _ => Cap::Round,
        };
        let join = if cap == Cap::Round { Join::Round } else { Join::Miter };
        let dash = match style::dash(pattern) {
            Some(d) => {
                let unit = weight.max(1.0 / 72.0);
                d.iter().map(|v| v * unit).collect()
            }
            None => {
                if pattern > 23 {
                    self.drawing.note(Note::CustomLinePattern);
                }
                Vec::new()
            }
        };
        if self.num(sheet, "CompoundType").is_some_and(|v| v > 0.0) {
            self.drawing.note(Note::CompoundLine);
        }
        if self.num(sheet, "LineGradientEnabled").is_some_and(|v| v != 0.0) {
            self.drawing.note(Note::LineGradient);
        }
        Some(Stroke { color, width: weight, dash, cap, join })
    }

    fn lines(&mut self, r: ShapeRef, sheet: SheetRef, mut line: Path, layer_color: Option<u32>) {
        let Some(st) = self.line_style(sheet, layer_color) else { return };
        // line ends on the first sub-path's start and the last one's end, if they are open
        let size = |b: &Self, c: &str| b.num(sheet, c).unwrap_or(2.0).clamp(0.0, 6.0) as u32;
        let begin = self.num(sheet, "BeginArrow").unwrap_or(0.0) as u32;
        let end = self.num(sheet, "EndArrow").unwrap_or(0.0) as u32;
        let mut ends: Vec<(style::Arrow, [f64; 2], [f64; 2])> = Vec::new();
        let n = line.subs.len();
        if begin > 0 && n > 0 && !line.subs[0].closed {
            if let Some(a) = style::arrow(begin, size(self, "BeginArrowSize"), st.width) {
                let sub = line.subs[0].reversed();
                if let Some((tip, dir, trimmed)) = end_of(&sub, a.setback) {
                    line.subs[0] = trimmed.reversed();
                    ends.push((a, tip, dir));
                }
            }
        }
        if end > 0 && n > 0 && !line.subs[n - 1].closed {
            if let Some(a) = style::arrow(end, size(self, "EndArrowSize"), st.width) {
                if let Some((tip, dir, trimmed)) = end_of(&line.subs[n - 1], a.setback) {
                    line.subs[n - 1] = trimmed;
                    ends.push((a, tip, dir));
                }
            }
        }
        self.push_stroke(r, Role::Shape, line, st.clone());
        for (a, tip, dir) in ends {
            let at = Xform { a: dir[0], b: dir[1], c: -dir[1], d: dir[0], e: tip[0], f: tip[1] };
            if !a.fill.is_empty() {
                self.push_fill(r, Role::Arrow, a.fill.map(&at), Paint::Solid(st.color));
            }
            if !a.stroke.is_empty() {
                let s = Stroke { dash: Vec::new(), join: Join::Miter, ..st.clone() };
                self.push_stroke(r, Role::Arrow, a.stroke.map(&at), s);
            }
        }
    }

    pub(super) fn push_fill(&mut self, r: ShapeRef, role: Role, path: Path, paint: Paint) {
        let Some(bb) = path.hull() else { return };
        self.items.push(Item { shape: r, top: self.top, bbox: bb, role, kind: ItemKind::Fill { path, paint } });
    }

    pub(super) fn push_stroke(&mut self, r: ShapeRef, role: Role, path: Path, stroke: Stroke) {
        let Some(bb) = path.hull() else { return };
        let g = stroke.width.max(1.0 / 96.0) * if stroke.join == Join::Miter { 2.0 } else { 0.5 } + 1.0 / 96.0;
        let bb = [bb[0] - g, bb[1] - g, bb[2] + g, bb[3] + g];
        self.items.push(Item { shape: r, top: self.top, bbox: bb, role, kind: ItemKind::Stroke { path, stroke } });
    }
}

/// the end of a sub-path for a line end: the tip, the direction it points to, and the sub-path
/// shortened by `setback`
fn end_of(sub: &Sub, setback: f64) -> Option<([f64; 2], [f64; 2], Sub)> {
    let last = *sub.segs.last()?;
    let prev = if sub.segs.len() >= 2 { sub.segs[sub.segs.len() - 2].end() } else { sub.start };
    let tip = last.end();
    let from = match last {
        Seg::Line(_) => prev,
        Seg::Cubic(a, b, p) => {
            if dist(b, p) > 1e-9 {
                b
            } else if dist(a, p) > 1e-9 {
                a
            } else {
                prev
            }
        }
    };
    let d = [tip[0] - from[0], tip[1] - from[1]];
    let l = dist(d, [0.0, 0.0]);
    if !(l > 0.0) {
        return None;
    }
    let dir = [d[0] / l, d[1] / l];
    let mut out = sub.clone();
    if setback > 0.0 {
        trim_end(&mut out, setback);
    }
    Some((tip, dir, out))
}

/// shorten a sub-path's end by `len` (along its last pieces)
fn trim_end(sub: &mut Sub, mut len: f64) {
    while len > 0.0 && !sub.segs.is_empty() {
        let n = sub.segs.len();
        let prev = if n >= 2 { sub.segs[n - 2].end() } else { sub.start };
        match sub.segs[n - 1] {
            Seg::Line(p) => {
                let l = dist(prev, p);
                if l > len {
                    let k = (l - len) / l;
                    sub.segs[n - 1] = Seg::Line([prev[0] + (p[0] - prev[0]) * k, prev[1] + (p[1] - prev[1]) * k]);
                    return;
                }
                if n == 1 {
                    // keep a stub so the line stays where it was
                    sub.segs[0] = Seg::Line(crate::geometry::lerp(prev, p, 0.001));
                    return;
                }
                len -= l;
                sub.segs.pop();
            }
            Seg::Cubic(a, b, p) => {
                // the parameter where the length from the end reaches len (sampled)
                let mut pts = [[0.0; 2]; 33];
                for (i, q) in pts.iter_mut().enumerate() {
                    *q = crate::geometry::cubic_at(prev, a, b, p, i as f64 / 32.0);
                }
                let mut acc = 0.0;
                let mut t = 0.0;
                for i in (0..32).rev() {
                    let d = dist(pts[i], pts[i + 1]);
                    if acc + d >= len {
                        t = (i as f64 + (1.0 - (len - acc) / d.max(1e-12))) / 32.0;
                        break;
                    }
                    acc += d;
                }
                if t <= 0.0 && n > 1 {
                    len -= acc;
                    sub.segs.pop();
                    continue;
                }
                // de Casteljau: the part from 0 to t
                let t = t.max(0.001);
                let ab = crate::geometry::lerp(prev, a, t);
                let bc = crate::geometry::lerp(a, b, t);
                let cd = crate::geometry::lerp(b, p, t);
                let abc = crate::geometry::lerp(ab, bc, t);
                let bcd = crate::geometry::lerp(bc, cd, t);
                let e = crate::geometry::lerp(abc, bcd, t);
                sub.segs[n - 1] = Seg::Cubic(ab, abc, e);
                return;
            }
        }
    }
}

/// the gradient axis of direction d (unit) across a box: from the box's lowest projection to
/// its highest
fn axis_through(bb: [f64; 4], d: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let c = [(bb[0] + bb[2]) * 0.5, (bb[1] + bb[3]) * 0.5];
    let half = ((bb[2] - bb[0]) * d[0].abs() + (bb[3] - bb[1]) * d[1].abs()) * 0.5;
    ([c[0] - d[0] * half, c[1] - d[1] * half], [c[0] + d[0] * half, c[1] + d[1] * half])
}

/// "#RRGGBB" (also "RGB(r,g,b)") -> 0xRRGGBB
pub fn parse_color(t: &str) -> Option<u32> {
    let t = t.trim();
    if let Some(h) = t.strip_prefix('#') {
        return (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok()).flatten();
    }
    let inner = t.strip_prefix("RGB(").or_else(|| t.strip_prefix("rgb("))?.strip_suffix(')')?;
    let mut c = 0u32;
    let mut n = 0;
    for p in inner.split(',') {
        let v: f64 = p.trim().parse().ok()?;
        c = c << 8 | (v.clamp(0.0, 255.0) as u32);
        n += 1;
    }
    (n == 3).then_some(c)
}

/// 0xRRGGBB with a transparency 0 .. 1 -> 0xAARRGGBB
pub fn with_alpha(rgb: u32, trans: f64) -> u32 {
    let a = ((1.0 - trans.clamp(0.0, 1.0)) * 255.0 + 0.5) as u32;
    (a.min(255) << 24) | (rgb & 0xFF_FFFF)
}

fn mix(a: u32, b: u32, k: f64) -> u32 {
    let k = k.clamp(0.0, 1.0);
    let mut out = 0u32;
    for s in [24, 16, 8, 0] {
        let x = ((a >> s) & 255) as f64;
        let y = ((b >> s) & 255) as f64;
        out |= (((x + (y - x) * k) + 0.5) as u32).min(255) << s;
    }
    out
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    crate::geometry::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

pub(crate) fn bbox_of(pts: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in pts {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

pub(crate) fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]).max(a[0].max(b[0])), a[3].min(b[3]).max(a[1].max(b[1]))]
}
