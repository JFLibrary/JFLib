// canvas -- drawing on one page: paths, paint, transforms, clipping, images, text, links
//
// The canvas writes the page's content stream.  Graphics state that only matters when
// something is painted (colours, alpha, line width, caps, joins, miter limit, dash, text mode)
// is kept as wanted and written just before the paint operator that needs it, so a run of
// paths in the same style writes it once.  Transforms (`cm`) are written at once; a path
// still open when the transform changes keeps where it was drawn.  Path construction is
// buffered until it is painted, so no state operator ever lands inside a path object.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::geom::{FillRule, LineCap, LineJoin, Matrix, Path, Rgb, Seg, TextMode};
use crate::obj::{self, num};
use crate::shading::{self, Alpha, Hatch, Prepared, Shading};
use crate::{Dest, Document, FontId, Glyph, ImageId, Link, LinkTarget, PageId};

/// where a canvas puts (0, 0) and which way y grows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// the top left corner of the page, y downwards (as gfx, the screen and page layout)
    TopLeft,
    /// the bottom left corner, y upwards (PDF's own page space)
    BottomLeft,
}

/// alpha in thousandths
pub(crate) fn alpha_key(a: f64) -> u16 {
    if a.is_finite() { (a.clamp(0.0, 1.0) * 1000.0 + 0.5) as u16 } else { 1000 }
}

/// two alphas in thousandths multiplied
fn alpha_mul(a: u16, b: u16) -> u16 {
    ((a as u32 * b as u32 + 500) / 1000) as u16
}

#[derive(Clone, Debug, PartialEq)]
struct Paint {
    fill: Rgb,
    stroke: Rgb,
    fill_alpha: u16,
    stroke_alpha: u16,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    dash: Vec<f64>,
    phase: f64,
    text_mode: TextMode,
}

impl Paint {
    /// PDF's initial graphics state
    fn initial() -> Paint {
        Paint {
            fill: Rgb::BLACK,
            stroke: Rgb::BLACK,
            fill_alpha: 1000,
            stroke_alpha: 1000,
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: Vec::new(),
            phase: 0.0,
            text_mode: TextMode::Fill,
        }
    }
}

/// drawing on one page (Document::canvas).  Coordinates are points (1/72 inch) from the
/// origin the canvas was made with; what it draws goes into the page when it is dropped, with
/// every save it left open restored.  Several canvases on one page draw one after the other.
pub struct Canvas<'a> {
    doc: &'a mut Document,
    page: usize,
    out: Vec<u8>,
    ctm: Matrix,
    want: Paint,
    have: Paint,
    stack: Vec<(Matrix, Paint, Paint)>,
    path: Path,
}

impl<'a> Canvas<'a> {
    pub(crate) fn new(doc: &'a mut Document, page: usize, origin: Origin) -> Canvas<'a> {
        let h = doc.pages[page].height;
        let mut c = Canvas {
            doc,
            page,
            out: Vec::new(),
            ctm: Matrix::IDENTITY,
            want: Paint::initial(),
            have: Paint::initial(),
            stack: Vec::new(),
            path: Path::new(),
        };
        c.out.extend_from_slice(b"q\n");
        if origin == Origin::TopLeft {
            c.write_cm(&Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, h));
            c.ctm = Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, h);
        }
        c
    }

    /// the page this canvas draws on
    pub fn page(&self) -> PageId {
        PageId(self.page as u32)
    }

    /// width and height of the page in points
    pub fn page_size(&self) -> (f64, f64) {
        let p = &self.doc.pages[self.page];
        (p.width, p.height)
    }

    /// the current transform from this canvas's coordinates to PDF page space (points from the
    /// bottom left, y up)
    pub fn ctm(&self) -> Matrix {
        self.ctm
    }

    /// the document (fonts, metrics, shaping) while drawing
    pub fn document(&self) -> &Document {
        self.doc
    }

    // ------------------------------------------------------------------ state

    /// saves the whole state (transform, clip, colours, alpha, stroke and text settings)
    pub fn save(&mut self) {
        self.out.extend_from_slice(b"q\n");
        self.stack.push((self.ctm, self.want.clone(), self.have.clone()));
    }

    /// back to the state of the matching save (none open: nothing happens)
    pub fn restore(&mut self) {
        let Some((ctm, want, have)) = self.stack.pop() else { return };
        self.out.extend_from_slice(b"Q\n");
        self.move_path(&ctm);
        self.ctm = ctm;
        self.want = want;
        self.have = have;
    }

    /// how many saves are open
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// m first, then the current transform (singular or not finite: ignored)
    pub fn transform(&mut self, m: Matrix) {
        if !m.is_finite() || m.invert().is_none() {
            return;
        }
        let ctm = m.then(&self.ctm);
        self.move_path(&ctm);
        self.ctm = ctm;
        self.write_cm(&m);
    }

    pub fn translate(&mut self, tx: f64, ty: f64) {
        self.transform(Matrix::translate(tx, ty));
    }

    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.transform(Matrix::scale(sx, sy));
    }

    /// radians: counter-clockwise where y points up, clockwise where it points down
    pub fn rotate(&mut self, angle: f64) {
        self.transform(Matrix::rotate(angle));
    }

    /// an open path keeps its place on the page when the transform becomes `ctm`
    fn move_path(&mut self, ctm: &Matrix) {
        if self.path.is_empty() || *ctm == self.ctm {
            return;
        }
        if let Some(inv) = ctm.invert() {
            let m = self.ctm.then(&inv);
            self.path.transform(&m);
        }
    }

    fn write_cm(&mut self, m: &Matrix) {
        let o = &mut self.out;
        for v in [m.a, m.b, m.c, m.d, m.e, m.f] {
            num(o, v);
            o.push(b' ');
        }
        o.extend_from_slice(b"cm\n");
    }

    pub fn set_fill_color(&mut self, c: Rgb) {
        self.want.fill = c.clamped();
    }

    pub fn set_stroke_color(&mut self, c: Rgb) {
        self.want.stroke = c.clamped();
    }

    /// opacity of fills, text fills and images, 0 to 1 (kept in thousandths)
    pub fn set_fill_alpha(&mut self, a: f64) {
        self.want.fill_alpha = alpha_key(a);
    }

    /// opacity of strokes, 0 to 1 (kept in thousandths)
    pub fn set_stroke_alpha(&mut self, a: f64) {
        self.want.stroke_alpha = alpha_key(a);
    }

    /// in the coordinates at the time of the stroke (0: the thinnest line the device draws)
    pub fn set_line_width(&mut self, w: f64) {
        if w.is_finite() && w >= 0.0 {
            self.want.width = w;
        }
    }

    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.want.cap = cap;
    }

    pub fn set_line_join(&mut self, join: LineJoin) {
        self.want.join = join;
    }

    /// the longest miter, as a multiple of the line width (at least 1)
    pub fn set_miter_limit(&mut self, limit: f64) {
        if limit.is_finite() && limit >= 1.0 {
            self.want.miter = limit;
        }
    }

    /// dashes: lengths on, off, on, ... from `phase` into the pattern; an empty list (or one
    /// with a negative length or only zeros) draws solid lines
    pub fn set_dash(&mut self, pattern: &[f64], phase: f64) {
        let ok = pattern.iter().all(|v| v.is_finite() && *v >= 0.0) && pattern.iter().any(|v| *v > 0.0);
        if ok && pattern.len() <= 64 {
            self.want.dash = pattern.to_vec();
            self.want.phase = if phase.is_finite() { phase } else { 0.0 };
        } else {
            self.want.dash.clear();
            self.want.phase = 0.0;
        }
    }

    pub fn set_text_mode(&mut self, mode: TextMode) {
        self.want.text_mode = mode;
    }

    fn sync_alpha(&mut self) {
        let (f, s) = (self.want.fill_alpha, self.want.stroke_alpha);
        if (f, s) == (self.have.fill_alpha, self.have.stroke_alpha) {
            return;
        }
        let next = self.doc.alphas.len();
        let i = *self.doc.alpha_ids.entry((f, s)).or_insert(next);
        if i == next {
            self.doc.alphas.push((f, s));
        }
        let page = &mut self.doc.pages[self.page];
        page.gstates.insert(i);
        if f < 1000 || s < 1000 {
            page.transparency = true;
        }
        let _ = core::fmt::Write::write_fmt(&mut obj::Fmt(&mut self.out), format_args!("/G{i} gs\n"));
        self.have.fill_alpha = f;
        self.have.stroke_alpha = s;
    }

    fn sync_fill(&mut self) {
        if self.want.fill != self.have.fill {
            let c = self.want.fill;
            self.color(c, b"rg\n");
            self.have.fill = c;
        }
        self.sync_alpha();
    }

    fn sync_stroke(&mut self) {
        if self.want.stroke != self.have.stroke {
            let c = self.want.stroke;
            self.color(c, b"RG\n");
            self.have.stroke = c;
        }
        let o = &mut self.out;
        if self.want.width != self.have.width {
            num(o, self.want.width);
            o.extend_from_slice(b" w\n");
            self.have.width = self.want.width;
        }
        if self.want.cap != self.have.cap {
            o.extend_from_slice(match self.want.cap {
                LineCap::Butt => b"0 J\n",
                LineCap::Round => b"1 J\n",
                LineCap::Square => b"2 J\n",
            });
            self.have.cap = self.want.cap;
        }
        if self.want.join != self.have.join {
            o.extend_from_slice(match self.want.join {
                LineJoin::Miter => b"0 j\n",
                LineJoin::Round => b"1 j\n",
                LineJoin::Bevel => b"2 j\n",
            });
            self.have.join = self.want.join;
        }
        if self.want.miter != self.have.miter {
            num(o, self.want.miter);
            o.extend_from_slice(b" M\n");
            self.have.miter = self.want.miter;
        }
        if self.want.dash != self.have.dash || self.want.phase != self.have.phase {
            obj::nums(o, &self.want.dash);
            o.push(b' ');
            num(o, self.want.phase);
            o.extend_from_slice(b" d\n");
            self.have.dash = self.want.dash.clone();
            self.have.phase = self.want.phase;
        }
        self.sync_alpha();
    }

    fn color(&mut self, c: Rgb, op: &[u8]) {
        let o = &mut self.out;
        num(o, c.r);
        o.push(b' ');
        num(o, c.g);
        o.push(b' ');
        num(o, c.b);
        o.push(b' ');
        o.extend_from_slice(op);
    }

    // ------------------------------------------------------------------ paths

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.path.move_to(x, y);
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        self.path.line_to(x, y);
    }

    pub fn curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) {
        self.path.curve_to(x1, y1, x2, y2, x, y);
    }

    pub fn quad_to(&mut self, x1: f64, y1: f64, x: f64, y: f64) {
        self.path.quad_to(x1, y1, x, y);
    }

    pub fn close(&mut self) {
        self.path.close();
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.path.rect(x, y, w, h);
    }

    pub fn rounded_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64) {
        self.path.rounded_rect(x, y, w, h, r);
    }

    pub fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) {
        self.path.ellipse(cx, cy, rx, ry);
    }

    pub fn circle(&mut self, cx: f64, cy: f64, r: f64) {
        self.path.circle(cx, cy, r);
    }

    /// Path::arc on the canvas's path
    pub fn arc(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, start: f64, sweep: f64) {
        self.path.arc(cx, cy, rx, ry, start, sweep);
    }

    /// a prepared path's subpaths, in the current coordinates
    pub fn append_path(&mut self, p: &Path) {
        self.path.append(p);
    }

    fn write_path(&mut self) -> bool {
        if self.path.is_empty() {
            return false;
        }
        let o = &mut self.out;
        for s in &self.path.segs {
            match *s {
                Seg::Move(x, y) => pts(o, &[x, y], b"m\n"),
                Seg::Line(x, y) => pts(o, &[x, y], b"l\n"),
                Seg::Cubic(a, b, c, d, e, f) => pts(o, &[a, b, c, d, e, f], b"c\n"),
                Seg::Rect(x, y, w, h) => pts(o, &[x, y, w, h], b"re\n"),
                Seg::Close => o.extend_from_slice(b"h\n"),
            }
        }
        self.path.clear();
        true
    }

    /// fills the path and starts a new one
    pub fn fill(&mut self, rule: FillRule) {
        if self.path.is_empty() {
            return;
        }
        self.sync_fill();
        self.write_path();
        self.out.extend_from_slice(if rule == FillRule::EvenOdd { b"f*\n" } else { b"f\n" });
    }

    /// strokes the path and starts a new one
    pub fn stroke(&mut self) {
        if self.path.is_empty() {
            return;
        }
        self.sync_stroke();
        self.write_path();
        self.out.extend_from_slice(b"S\n");
    }

    /// fills, then strokes the path, and starts a new one
    pub fn fill_stroke(&mut self, rule: FillRule) {
        if self.path.is_empty() {
            return;
        }
        self.sync_fill();
        self.sync_stroke();
        self.write_path();
        self.out.extend_from_slice(if rule == FillRule::EvenOdd { b"B*\n" } else { b"B\n" });
    }

    /// intersects the clip with the path (until the matching restore) and starts a new path
    pub fn clip(&mut self, rule: FillRule) {
        if self.write_path() {
            self.out.extend_from_slice(if rule == FillRule::EvenOdd { b"W* n\n" } else { b"W n\n" });
        }
    }

    /// drops the path without painting it
    pub fn end_path(&mut self) {
        self.path.clear();
    }

    pub fn fill_path(&mut self, p: &Path, rule: FillRule) {
        self.path.clear();
        self.path.append(p);
        self.fill(rule);
    }

    pub fn stroke_path(&mut self, p: &Path) {
        self.path.clear();
        self.path.append(p);
        self.stroke();
    }

    // ------------------------------------------------------------------ gradients and hatches

    /// paints the gradient over the whole current clip (`sh`), its coordinates in the current user
    /// space, times the canvas's fill alpha.  Degenerate geometry paints the last stop's colour
    /// (axial from = to, radial with both radii 0: everywhere; two equal circles: that disk); no
    /// stops, coordinates that are not finite or a negative radius: nothing.  The path being built
    /// is kept.
    pub fn shade(&mut self, s: &Shading) {
        let prepared = shading::prepare(s);
        if matches!(prepared, Prepared::Invalid) {
            return;
        }
        let kept = core::mem::take(&mut self.path);
        let (area, bb) = self.page_area();
        self.scoped(|c| match prepared {
            Prepared::Invalid => {}
            Prepared::Solid(col, a) => {
                c.path = area;
                c.paint_solid(col, a, FillRule::NonZero);
            }
            Prepared::Disk(at, r, col, a) => {
                c.path.circle(at[0], at[1], r);
                c.paint_solid(col, a, FillRule::NonZero);
            }
            Prepared::Shade(dict, alpha) => c.paint_shading(dict, alpha, bb),
        });
        self.path = kept;
    }

    /// fills the path with the gradient (q, clip with `rule`, sh, Q) and starts a new path, as
    /// fill() does; what shade() does with degenerate and invalid gradients, within the path (an
    /// invalid one drops the path)
    pub fn fill_shading(&mut self, rule: FillRule, s: &Shading) {
        if self.path.is_empty() {
            return;
        }
        let prepared = shading::prepare(s);
        if matches!(prepared, Prepared::Invalid) {
            self.path.clear();
            return;
        }
        // the soft mask's group covers the path (with a margin against edge antialiasing)
        let bb = match self.path_bbox() {
            Some(b) => {
                let m = (b[2] - b[0]).max(b[3] - b[1]) * 0.05 + 1.0;
                [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
            }
            None => self.page_area().1,
        };
        self.scoped(|c| match prepared {
            Prepared::Invalid => c.path.clear(),
            Prepared::Solid(col, a) => c.paint_solid(col, a, rule),
            Prepared::Disk(at, r, col, a) => {
                c.clip(rule);
                c.path.circle(at[0], at[1], r);
                c.paint_solid(col, a, FillRule::NonZero);
            }
            Prepared::Shade(dict, alpha) => {
                c.clip(rule);
                c.paint_shading(dict, alpha, bb);
            }
        });
    }

    /// fills the path with the hatch's lines (q, clip with `rule`, the lines stroked, Q) and starts
    /// a new path, as fill() does.  Lines at least as wide as their spacing fill the path solid;
    /// an angle, spacing, width or origin that is not finite (or a spacing or width <= 0), a path
    /// with coordinates that are not finite or more than 100 000 lines: nothing, the path dropped
    pub fn fill_hatch(&mut self, rule: FillRule, h: &Hatch) {
        if self.path.is_empty() {
            return;
        }
        let ok = h.angle.is_finite() && h.spacing.is_finite() && h.spacing > 0.0 && h.width.is_finite() && h.width > 0.0 && h.origin.iter().all(|v| v.is_finite());
        let (true, Some(bb)) = (ok, self.path_bbox()) else {
            self.path.clear();
            return;
        };
        let color = h.color.clamped();
        let a = alpha_key(h.alpha);
        if h.width >= h.spacing {
            self.scoped(|c| c.paint_solid(color, a, rule));
            return;
        }
        let lines = match shading::hatch_lines(h, bb) {
            Some(l) if !l.is_empty() => l,
            _ => {
                self.path.clear();
                return;
            }
        };
        self.scoped(|c| {
            c.clip(rule);
            c.want.stroke = color;
            c.want.width = h.width;
            c.want.cap = LineCap::Butt;
            c.want.join = c.have.join;
            c.want.miter = c.have.miter;
            c.want.dash.clear();
            c.want.phase = 0.0;
            c.want.stroke_alpha = alpha_mul(c.want.fill_alpha, a);
            c.sync_stroke();
            for l in &lines {
                pts(&mut c.out, &[l[0], l[1]], b"m\n");
                pts(&mut c.out, &[l[2], l[3]], b"l\n");
            }
            c.out.extend_from_slice(b"S\n");
        });
    }

    /// f between q and Q; the state the canvas knows is what it was before
    fn scoped(&mut self, f: impl FnOnce(&mut Self)) {
        let (want, have) = (self.want.clone(), self.have.clone());
        self.out.extend_from_slice(b"q\n");
        f(self);
        self.out.extend_from_slice(b"Q\n");
        self.want = want;
        self.have = have;
    }

    /// fills the path in one colour, its alpha times the fill alpha
    fn paint_solid(&mut self, col: Rgb, a: u16, rule: FillRule) {
        self.want.fill = col;
        self.want.fill_alpha = alpha_mul(self.want.fill_alpha, a);
        self.fill(rule);
    }

    /// `sh` with its alpha: a constant (times the fill alpha) or a soft mask over `bb`
    fn paint_shading(&mut self, dict: Vec<u8>, alpha: Alpha, bb: [f64; 4]) {
        let i = self.doc.add_shading(dict);
        match alpha {
            Alpha::Const(a) => {
                self.want.fill_alpha = alpha_mul(self.want.fill_alpha, a);
                self.sync_alpha();
            }
            Alpha::Mask(gray) => {
                self.sync_alpha();
                let g = self.doc.add_shading(gray);
                let m = self.doc.add_mask(g, bb);
                let page = &mut self.doc.pages[self.page];
                page.masks.insert(m);
                page.transparency = true;
                // the group's space is the user space at this gs, the same as the shading's below
                let _ = core::fmt::Write::write_fmt(&mut obj::Fmt(&mut self.out), format_args!("/M{m} gs\n"));
            }
        }
        self.doc.pages[self.page].shadings.insert(i);
        let _ = core::fmt::Write::write_fmt(&mut obj::Fmt(&mut self.out), format_args!("/Sh{i} sh\n"));
    }

    /// the bounding box of the path's points and control points; None for an empty path or one
    /// with coordinates that are not finite
    fn path_bbox(&self) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut finite = true;
        let mut add = |x: f64, y: f64| {
            finite &= x.is_finite() && y.is_finite();
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        };
        for s in &self.path.segs {
            match *s {
                Seg::Move(x, y) | Seg::Line(x, y) => add(x, y),
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    add(x1, y1);
                    add(x2, y2);
                    add(x, y);
                }
                Seg::Rect(x, y, w, h) => {
                    add(x, y);
                    add(x + w, y + h);
                }
                Seg::Close => {}
            }
        }
        (finite && b.iter().all(|v| v.is_finite())).then_some(b)
    }

    /// the page in the current user space: its outline and bounding box
    fn page_area(&self) -> (Path, [f64; 4]) {
        let (w, h) = self.page_size();
        let inv = self.ctm.invert().unwrap_or(Matrix::IDENTITY);
        let c = [inv.apply(0.0, 0.0), inv.apply(w, 0.0), inv.apply(w, h), inv.apply(0.0, h)];
        let mut p = Path::new();
        p.move_to(c[0].0, c[0].1);
        for q in &c[1..] {
            p.line_to(q.0, q.1);
        }
        p.close();
        let mut b = [c[0].0, c[0].1, c[0].0, c[0].1];
        for q in &c[1..] {
            b = [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)];
        }
        (p, b)
    }

    // ------------------------------------------------------------------ images

    /// the image upright (as it looks on the page) in the rectangle x, y, w, h of the current
    /// coordinates
    pub fn image(&mut self, img: ImageId, x: f64, y: f64, w: f64, h: f64) {
        let m = if self.ctm.det() < 0.0 { Matrix::new(w, 0.0, 0.0, -h, x, y + h) } else { Matrix::new(w, 0.0, 0.0, h, x, y) };
        self.image_transform(img, m);
    }

    /// the image's unit square (row 0 along y = 1, column 0 along x = 0) mapped by m into the
    /// current coordinates
    pub fn image_transform(&mut self, img: ImageId, m: Matrix) {
        let i = img.0 as usize;
        if i >= self.doc.images.len() || !m.is_finite() || m.invert().is_none() {
            return;
        }
        self.sync_alpha();
        let page = &mut self.doc.pages[self.page];
        page.images.insert(i);
        if self.doc.images[i].has_alpha() {
            page.transparency = true;
        }
        self.out.extend_from_slice(b"q\n");
        self.write_cm(&m);
        let _ = core::fmt::Write::write_fmt(&mut obj::Fmt(&mut self.out), format_args!("/Im{i} Do\nQ\n"));
    }

    // ------------------------------------------------------------------ text

    /// one line of text shaped with the font (kerning, ligatures, marks, bidi), its baseline
    /// starting at (x, y), `size` the em in points; upright as it looks on the page.  Returns
    /// the advance along the baseline (0 when nothing was drawn).
    pub fn text(&mut self, font: FontId, size: f64, x: f64, y: f64, s: &str) -> f64 {
        self.text_fallback(&[font], size, x, y, s)
    }

    /// the same with fallback fonts: a run of one script the first font cannot show comes from
    /// the first of the others that can
    pub fn text_fallback(&mut self, fonts: &[FontId], size: f64, x: f64, y: f64, s: &str) -> f64 {
        let glyphs = self.doc.shape(fonts, s);
        self.glyphs(fonts, size, x, y, &glyphs, s)
    }

    /// shaped glyphs (in visual order; `Glyph::font` indexes `fonts`) from the
    /// pen at (x, y); `text` is the shaped string (each glyph's cluster is a byte range of it)
    /// for search and copy.  Returns the advance.
    pub fn glyphs(&mut self, fonts: &[FontId], size: f64, x: f64, y: f64, glyphs: &[Glyph], text: &str) -> f64 {
        if !(size.is_finite() && size > 0.0 && x.is_finite() && y.is_finite()) || glyphs.is_empty() {
            return 0.0;
        }
        let ids: Vec<usize> = fonts.iter().map(|f| f.0 as usize).collect();
        if ids.is_empty() || ids.iter().any(|&i| i >= self.doc.fonts.len()) {
            return 0.0;
        }
        match self.want.text_mode {
            TextMode::Fill => self.sync_fill(),
            TextMode::Stroke => self.sync_stroke(),
            TextMode::FillStroke => {
                self.sync_fill();
                self.sync_stroke();
            }
            TextMode::Invisible => {}
        }
        let o = &mut self.out;
        o.extend_from_slice(b"BT\n");
        if self.want.text_mode != self.have.text_mode {
            o.extend_from_slice(match self.want.text_mode {
                TextMode::Fill => b"0 Tr\n",
                TextMode::Stroke => b"1 Tr\n",
                TextMode::FillStroke => b"2 Tr\n",
                TextMode::Invisible => b"3 Tr\n",
            });
            self.have.text_mode = self.want.text_mode;
        }
        // text space: x along the baseline, y up as seen on the page
        let flip = if self.ctm.det() < 0.0 { -1.0 } else { 1.0 };
        pts(o, &[1.0, 0.0, 0.0, flip, x, y], b"Tm\n");
        let mut run = TextRun { out: Vec::new(), open: false, in_str: false };
        let mut cur_font = usize::MAX;
        let (mut lx, mut ly) = (0.0f64, 0.0f64); // line start (Td)
        let (mut tx, mut ty) = (0.0f64, 0.0f64); // where PDF's text position is
        let mut pen = 0.0f64;
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new(); // clusters mapped in this call
        let mut used_fonts: Vec<usize> = Vec::new();
        for g in glyphs {
            let Some(&fi) = ids.get(g.font) else { continue };
            let f = &mut self.doc.fonts[fi];
            let upem = f.upem as f64;
            let scale = size / upem;
            let gid = g.id;
            let gx = pen + g.x as f64 * scale;
            let gy = g.y as f64 * scale;
            pen += g.advance as f64 * scale;
            if gid >= f.glyphs as u32 {
                continue;
            }
            if fi != cur_font {
                run.flush(o);
                let _ = core::fmt::Write::write_fmt(&mut obj::Fmt(o), format_args!("/F{fi} "));
                num(o, size);
                o.extend_from_slice(b" Tf\n");
                cur_font = fi;
                if !used_fonts.contains(&fi) {
                    used_fonts.push(fi);
                }
            }
            let dx = gx - tx;
            if (gy - ty).abs() > 1e-9 || (!run.open && dx.abs() > 1e-9) {
                run.flush(o);
                pts(o, &[gx - lx, gy - ly], b"Td\n");
                lx = gx;
                ly = gy;
                tx = gx;
                ty = gy;
            } else if dx.abs() > 1e-9 {
                // TJ: a number moves left by number / 1000 em
                run.adjust(-dx * 1000.0 / size);
                tx = gx;
            }
            // ToUnicode: the first glyph of a cluster stands for its text
            let cl = (g.cluster.min(text.len()), g.end.min(text.len()));
            let unicode = if gid != 0 && cl.0 < cl.1 && text.is_char_boundary(cl.0) && text.is_char_boundary(cl.1) && seen.insert(cl) {
                Some(String::from(&text[cl.0..cl.1]))
            } else {
                None
            };
            run.glyph(gid as u16);
            tx += f.advance(gid as u16) as f64 * scale;
            f.mark(gid as u16, unicode);
        }
        run.flush(o);
        o.extend_from_slice(b"ET\n");
        for fi in used_fonts {
            self.doc.pages[self.page].fonts.insert(fi);
        }
        pen
    }

    // ------------------------------------------------------------------ links

    /// a link on the rectangle x, y, w, h (its box on the page when turned) to a URI
    pub fn link_uri(&mut self, x: f64, y: f64, w: f64, h: f64, uri: &str) {
        let rect = self.page_box(x, y, w, h);
        self.doc.pages[self.page].links.push(Link { rect, target: LinkTarget::Uri(String::from(uri)) });
    }

    /// a link on the rectangle x, y, w, h to a place in the document
    pub fn link_dest(&mut self, x: f64, y: f64, w: f64, h: f64, dest: Dest) {
        let rect = self.page_box(x, y, w, h);
        self.doc.pages[self.page].links.push(Link { rect, target: LinkTarget::Dest(dest) });
    }

    /// a destination showing the point (x, y) of this canvas at the top left of the window,
    /// the zoom unchanged
    pub fn dest(&self, x: f64, y: f64) -> Dest {
        let (px, py) = self.ctm.apply(x, y);
        Dest::Xyz { page: self.page(), left: px, top: py, zoom: None }
    }

    fn page_box(&self, x: f64, y: f64, w: f64, h: f64) -> [f64; 4] {
        let c = [self.ctm.apply(x, y), self.ctm.apply(x + w, y), self.ctm.apply(x + w, y + h), self.ctm.apply(x, y + h)];
        let mut b = [c[0].0, c[0].1, c[0].0, c[0].1];
        for p in &c[1..] {
            b[0] = b[0].min(p.0);
            b[1] = b[1].min(p.1);
            b[2] = b[2].max(p.0);
            b[3] = b[3].max(p.1);
        }
        b
    }
}

impl Drop for Canvas<'_> {
    fn drop(&mut self) {
        for _ in 0..=self.stack.len() {
            self.out.extend_from_slice(b"Q\n");
        }
        let out = core::mem::take(&mut self.out);
        self.doc.pages[self.page].content.extend_from_slice(&out);
    }
}

/// numbers separated by spaces, then the operator
fn pts(o: &mut Vec<u8>, v: &[f64], op: &[u8]) {
    for x in v {
        num(o, *x);
        o.push(b' ');
    }
    o.extend_from_slice(op);
}

/// the array of one TJ
struct TextRun {
    out: Vec<u8>,
    open: bool,
    in_str: bool,
}

impl TextRun {
    fn glyph(&mut self, gid: u16) {
        if !self.open {
            self.out.push(b'[');
            self.open = true;
        }
        if !self.in_str {
            self.out.push(b'<');
            self.in_str = true;
        }
        const H: &[u8; 16] = b"0123456789ABCDEF";
        for b in gid.to_be_bytes() {
            self.out.push(H[(b >> 4) as usize]);
            self.out.push(H[(b & 15) as usize]);
        }
    }

    fn adjust(&mut self, n: f64) {
        if self.in_str {
            self.out.push(b'>');
            self.in_str = false;
        }
        self.out.push(b' ');
        num(&mut self.out, n);
        self.out.push(b' ');
    }

    fn flush(&mut self, o: &mut Vec<u8>) {
        if !self.open {
            return;
        }
        if self.in_str {
            self.out.push(b'>');
        }
        self.out.extend_from_slice(b"] TJ\n");
        o.extend_from_slice(&self.out);
        self.out.clear();
        self.open = false;
        self.in_str = false;
    }
}
