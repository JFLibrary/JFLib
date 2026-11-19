// stencil_icon -- a master's icon for the Shapes panel, rendered from the master itself (JD9,
// SH-G03)
//
//   master_icon(&mut doc, master) -> Result<Vec<u8>, EditError>   a 32x32 ICO with 16 colours
//   rasterize(&drawing, box) -> Vec<u8>                           the same from a display list
//
// The icon is the kind desktop Visio writes into a stencil: one ICO image of 32 by 32 pixels,
// 4 bits per pixel with its own palette of 16 colours, and an AND mask for transparency --
// ICONDIR (6 bytes), one ICONDIRENTRY (16), BITMAPINFOHEADER (40: width 32, height 64 for the
// colour and the mask bitmap, 1 plane, 4 bits), the palette (16 x BGRA), the colour bitmap (32
// rows of 16 bytes) and the mask (32 rows of 4 bytes), rows bottom up: 766 bytes.
//
// How: an instance of the master goes onto the stencil's page inside a transaction, the formula
// engine computes it, render::build makes its display list, and the transaction is dropped, so
// the page stays empty.  The list is drawn with 4 x 4 samples a pixel: fills even-odd, lines at
// least one pixel wide with round ends, text as a block per character (no fonts here),
// fitted into the 32 x 32 square with a margin of 2 pixels and centred (a shape that draws
// only text, like a text box, is fitted to its text).  Pixels at least a
// quarter covered are opaque, blended over white; the colours are reduced to at most 15 by
// median cut.  Palette entry 0 is black: the transparent pixels point at it, so that the screen
// shows through (AND 1, XOR 0).

use alloc::vec::Vec;

use diagram_core::formula::{math, Engine};
use diagram_core::geometry::{Path, Seg};
use diagram_core::render::{self, ApproxMeasure, Drawing, ItemKind, Options, Paint};
use diagram_core::{CellAddr, Document, EditError, MasterRef, Owner, SheetRef, Sym};

const N: usize = 32;
const SS: usize = 4;
const G: usize = N * SS;
const MARGIN: f64 = 2.0;

/// the icon of master `m` (see the module comment); the document is unchanged afterwards
pub(crate) fn master_icon(doc: &mut Document, m: MasterRef) -> Result<Vec<u8>, EditError> {
    let page = doc.pages().next().ok_or(EditError::NoPage)?;
    let mut tx = doc.transaction("icon");
    let shape = tx.add_instance(Owner::Page(page), m, 0.0, 0.0)?;
    let sheets: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
    Engine::new().recalc(&mut tx, &sheets)?;
    let d = tx.doc();
    let drawing = render::build(d, page, &mut ApproxMeasure, &Options { background: false, hidden_layers: true });
    let num = |name: Sym| d.resolve(SheetRef::Shape(shape), &CellAddr::single(name)).num().unwrap_or(0.0);
    let (w, h) = match (d.sym("Width"), d.sym("Height")) {
        (Some(ws), Some(hs)) => (num(ws), num(hs)),
        _ => (0.0, 0.0),
    };
    let xf = d.page_xform(shape);
    let corners = [xf.apply([0.0, 0.0]), xf.apply([w, 0.0]), xf.apply([0.0, h]), xf.apply([w, h])];
    let mut bx = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for c in corners {
        bx = [bx[0].min(c[0]), bx[1].min(c[1]), bx[2].max(c[0]), bx[3].max(c[1])];
    }
    let ico = rasterize(&drawing, bx);
    drop(tx);
    Ok(ico)
}

// premultiplied RGBA samples
struct Canvas {
    px: Vec<[f64; 4]>,
    mask: Vec<bool>,
    k: f64,
    ox: f64,
    oy: f64,
    x0: f64,
    y1: f64,
}

fn argb(c: u32) -> [f64; 4] {
    let a = ((c >> 24) & 255) as f64 / 255.0;
    [((c >> 16) & 255) as f64 / 255.0 * a, ((c >> 8) & 255) as f64 / 255.0 * a, (c & 255) as f64 / 255.0 * a, a]
}

impl Canvas {
    // page inches -> sample space (y down)
    fn map(&self, p: [f64; 2]) -> [f64; 2] {
        [(p[0] - self.x0) * self.k + self.ox, (self.y1 - p[1]) * self.k + self.oy]
    }

    fn clear_mask(&mut self) {
        self.mask.iter_mut().for_each(|m| *m = false);
    }

    // composite the masked samples with colour c (premultiplied)
    fn paint(&mut self, c: [f64; 4]) {
        for (p, &m) in self.px.iter_mut().zip(self.mask.iter()) {
            if m {
                let inv = 1.0 - c[3];
                *p = [c[0] + p[0] * inv, c[1] + p[1] * inv, c[2] + p[2] * inv, c[3] + p[3] * inv];
            }
        }
    }

    // polylines of a path in sample space (each sub-path closed when `close` or it is closed)
    fn polylines(&self, path: &Path, close: bool) -> Vec<Vec<[f64; 2]>> {
        let mut out = Vec::new();
        for sub in &path.subs {
            let mut pts = alloc::vec![self.map(sub.start)];
            let mut cur = sub.start;
            for s in &sub.segs {
                match *s {
                    Seg::Line(p) => pts.push(self.map(p)),
                    Seg::Cubic(a, b, p) => {
                        let (m0, m1, m2, m3) = (self.map(cur), self.map(a), self.map(b), self.map(p));
                        let len = dist(m0, m1) + dist(m1, m2) + dist(m2, m3);
                        let n = if len.is_finite() { (math::ceil(len / 2.0) as usize).clamp(1, 64) } else { 1 };
                        for i in 1..=n {
                            let t = i as f64 / n as f64;
                            let u = 1.0 - t;
                            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                            pts.push([
                                w[0] * m0[0] + w[1] * m1[0] + w[2] * m2[0] + w[3] * m3[0],
                                w[0] * m0[1] + w[1] * m1[1] + w[2] * m2[1] + w[3] * m3[1],
                            ]);
                        }
                    }
                }
                cur = s.end();
            }
            if (close || sub.closed) && pts.len() > 1 {
                let first = pts[0];
                pts.push(first);
            }
            if pts.iter().all(|p| p[0].is_finite() && p[1].is_finite()) {
                out.push(pts);
            }
        }
        out
    }

    // the mask: samples inside the polygons (even-odd)
    fn fill_mask(&mut self, polys: &[Vec<[f64; 2]>]) {
        let mut xs: Vec<f64> = Vec::new();
        for row in 0..G {
            let y = row as f64 + 0.5;
            xs.clear();
            for poly in polys {
                for w in poly.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                        xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            for pair in xs.chunks_exact(2) {
                let from = math::ceil(pair[0] - 0.5).max(0.0) as usize;
                let to = math::ceil(pair[1] - 0.5).clamp(0.0, G as f64) as usize;
                for col in from..to.min(G) {
                    self.mask[row * G + col] = true;
                }
            }
        }
    }

    // the mask: samples within half a line width of the polylines
    fn stroke_mask(&mut self, polys: &[Vec<[f64; 2]>], hw: f64) {
        for poly in polys {
            for w in poly.windows(2) {
                let (a, b) = (w[0], w[1]);
                let lo = [a[0].min(b[0]) - hw, a[1].min(b[1]) - hw];
                let hi = [a[0].max(b[0]) + hw, a[1].max(b[1]) + hw];
                let (c0, c1) = (math::floor(lo[0]).max(0.0) as usize, (math::ceil(hi[0]).max(0.0) as usize).min(G));
                let (r0, r1) = (math::floor(lo[1]).max(0.0) as usize, (math::ceil(hi[1]).max(0.0) as usize).min(G));
                for row in r0..r1 {
                    for col in c0..c1 {
                        let p = [col as f64 + 0.5, row as f64 + 0.5];
                        if seg_dist(p, a, b) <= hw {
                            self.mask[row * G + col] = true;
                        }
                    }
                }
            }
        }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    math::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, [a[0] + t * d[0], a[1] + t * d[1]])
}

// the characters of a text item as blocks in page inches: a block per character that is not a
// space, as wide as 70 % of its share of the run, 0.7 em high from the baseline -- text without
// fonts, as a greeked word
fn glyphs(t: &render::TextItem) -> Vec<([[f64; 2]; 4], u32)> {
    let mut out = Vec::new();
    for run in &t.runs {
        let n = run.text.chars().count();
        if n == 0 || !(run.width > 0.0) {
            continue;
        }
        let cell = run.width / n as f64;
        for (i, c) in run.text.chars().enumerate() {
            if c.is_whitespace() {
                continue;
            }
            let x0 = run.at[0] + cell * (i as f64 + 0.15);
            let x1 = x0 + cell * 0.7;
            let (y0, y1) = (run.at[1], run.at[1] + 0.7 * run.size);
            out.push(([t.xform.apply([x0, y0]), t.xform.apply([x1, y0]), t.xform.apply([x1, y1]), t.xform.apply([x0, y1])], run.color));
        }
    }
    out
}

/// the icon of a display list, fitted to `bx` (x0, y0, x1, y1 in page inches, the shape's box)
/// and the items; a shape that draws nothing but text is fitted to its text
pub(crate) fn rasterize(drawing: &Drawing, bx: [f64; 4]) -> Vec<u8> {
    let union = |a: [f64; 4], b: [f64; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
    let drawn = drawing.items.iter().filter(|i| !matches!(i.kind, ItemKind::Text(_))).map(|i| i.bbox).reduce(union);
    let text: Vec<[f64; 4]> = drawing
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Text(t) => Some(glyphs(t)),
            _ => None,
        })
        .flatten()
        .map(|(q, _)| {
            let xs = q.iter().map(|p| p[0]);
            let ys = q.iter().map(|p| p[1]);
            [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)]
        })
        .collect();
    let mut b = match (drawn, text.iter().copied().reduce(union)) {
        (Some(d), _) => union(bx, d),
        (None, Some(t)) => t,
        (None, None) => bx,
    };
    if !b.iter().all(|v| v.is_finite()) || b[2] < b[0] || b[3] < b[1] {
        b = [0.0, 0.0, 1.0, 1.0];
    }
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    let room = (N as f64 - 2.0 * MARGIN) * SS as f64;
    let k = match (bw > 1e-9, bh > 1e-9) {
        (true, true) => (room / bw).min(room / bh),
        (true, false) => room / bw,
        (false, true) => room / bh,
        (false, false) => 1.0,
    };
    let mut cv = Canvas {
        px: alloc::vec![[0.0; 4]; G * G],
        mask: alloc::vec![false; G * G],
        k,
        ox: (G as f64 - bw * k) / 2.0,
        oy: (G as f64 - bh * k) / 2.0,
        x0: b[0],
        y1: b[3],
    };
    for item in &drawing.items {
        cv.clear_mask();
        match &item.kind {
            ItemKind::Fill { path, paint } => {
                let c = match paint {
                    Paint::Solid(c) => argb(*c),
                    Paint::Hatch { color, .. } => {
                        let x = argb(*color);
                        [x[0] * 0.5, x[1] * 0.5, x[2] * 0.5, x[3] * 0.5]
                    }
                    Paint::Gradient(g) => argb(g.at(0.5)),
                };
                let polys = cv.polylines(path, true);
                cv.fill_mask(&polys);
                cv.paint(c);
            }
            ItemKind::Stroke { path, stroke } => {
                let polys = cv.polylines(path, false);
                let hw = (stroke.width * k).max(SS as f64) / 2.0;
                cv.stroke_mask(&polys, hw);
                cv.paint(argb(stroke.color));
            }
            ItemKind::Text(t) => {
                // a block per character, at least a pixel high, in the run's colour
                for (quad, color) in glyphs(t) {
                    cv.clear_mask();
                    let mut q: Vec<[f64; 2]> = quad.iter().map(|&p| cv.map(p)).collect();
                    let (top, bottom) = (q.iter().map(|p| p[1]).fold(f64::MAX, f64::min), q.iter().map(|p| p[1]).fold(f64::MIN, f64::max));
                    if bottom - top < SS as f64 {
                        let grow = (SS as f64 - (bottom - top)) / 2.0;
                        let mid = (top + bottom) / 2.0;
                        for p in q.iter_mut() {
                            p[1] = if p[1] <= mid { p[1] - grow } else { p[1] + grow };
                        }
                    }
                    let first = q[0];
                    q.push(first);
                    if q.iter().all(|p| p[0].is_finite() && p[1].is_finite()) {
                        cv.fill_mask(&[q]);
                        cv.paint(argb(color));
                    }
                }
            }
            ItemKind::Image(_) => {}
        }
    }
    // pixels: the average of their samples, opaque from a quarter, over white
    let mut pixels: Vec<Option<[u8; 3]>> = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            let mut s = [0.0; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = cv.px[(y * SS + sy) * G + x * SS + sx];
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                }
            }
            let n = (SS * SS) as f64;
            let (r, g, bl, a) = (s[0] / n, s[1] / n, s[2] / n, s[3] / n);
            if a < 0.25 {
                pixels.push(None);
            } else {
                let ch = |v: f64| math::round(((v + (1.0 - a)) * 255.0).clamp(0.0, 255.0)) as u8;
                pixels.push(Some([ch(r), ch(g), ch(bl)]));
            }
        }
    }
    ico(&pixels)
}

// colours and how often they occur, sorted by colour
fn histogram(pixels: &[Option<[u8; 3]>]) -> Vec<([u8; 3], u32)> {
    let mut v: Vec<[u8; 3]> = pixels.iter().flatten().copied().collect();
    v.sort_unstable();
    let mut out: Vec<([u8; 3], u32)> = Vec::new();
    for c in v {
        match out.last_mut() {
            Some((l, n)) if *l == c => *n += 1,
            _ => out.push((c, 1)),
        }
    }
    out
}

// at most `max` colours for the histogram: the colours themselves when few, else median cut
fn palette(hist: &[([u8; 3], u32)], max: usize) -> Vec<[u8; 3]> {
    if hist.len() <= max {
        let mut v: Vec<([u8; 3], u32)> = hist.to_vec();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        return v.into_iter().map(|(c, _)| c).collect();
    }
    let mut boxes: Vec<Vec<([u8; 3], u32)>> = alloc::vec![hist.to_vec()];
    while boxes.len() < max {
        // the box with the widest channel range (among those with two colours or more)
        let mut best: Option<(usize, usize, u8)> = None;
        for (i, bx) in boxes.iter().enumerate() {
            if bx.len() < 2 {
                continue;
            }
            for ch in 0..3 {
                let lo = bx.iter().map(|c| c.0[ch]).min().unwrap_or(0);
                let hi = bx.iter().map(|c| c.0[ch]).max().unwrap_or(0);
                let range = hi - lo;
                if best.is_none_or(|(_, _, r)| range > r) {
                    best = Some((i, ch, range));
                }
            }
        }
        let Some((i, ch, _)) = best else { break };
        let mut bx = boxes.remove(i);
        bx.sort_by(|a, b| a.0[ch].cmp(&b.0[ch]).then(a.0.cmp(&b.0)));
        let total: u64 = bx.iter().map(|c| c.1 as u64).sum();
        let mut acc = 0u64;
        let mut cut = 1;
        for (j, c) in bx.iter().enumerate() {
            acc += c.1 as u64;
            if acc * 2 >= total {
                cut = (j + 1).clamp(1, bx.len() - 1);
                break;
            }
        }
        let rest = bx.split_off(cut);
        boxes.insert(i.min(boxes.len()), rest);
        boxes.insert(i.min(boxes.len()), bx);
    }
    boxes
        .iter()
        .map(|bx| {
            let n: u64 = bx.iter().map(|c| c.1 as u64).sum::<u64>().max(1);
            let avg = |ch: usize| ((bx.iter().map(|c| c.0[ch] as u64 * c.1 as u64).sum::<u64>() + n / 2) / n) as u8;
            [avg(0), avg(1), avg(2)]
        })
        .collect()
}

fn nearest(pal: &[[u8; 3]], c: [u8; 3]) -> usize {
    let mut best = (0usize, u32::MAX);
    for (i, p) in pal.iter().enumerate() {
        let d: u32 = (0..3).map(|k| (p[k] as i32 - c[k] as i32).unsigned_abs().pow(2)).sum();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

// the ICO file of 32 x 32 pixels (None: transparent), 4 bits per pixel
fn ico(pixels: &[Option<[u8; 3]>]) -> Vec<u8> {
    // entry 0 black (the transparent pixels' colour), then up to 15 colours
    let hist = histogram(pixels);
    let without_black: Vec<([u8; 3], u32)> = hist.iter().copied().filter(|(c, _)| *c != [0, 0, 0]).collect();
    let mut pal = alloc::vec![[0u8, 0, 0]];
    pal.extend(palette(&without_black, 15).into_iter().filter(|c| *c != [0, 0, 0]));
    pal.truncate(16);
    let mut out = Vec::with_capacity(766);
    let le16 = |o: &mut Vec<u8>, v: u16| o.extend_from_slice(&v.to_le_bytes());
    let le32 = |o: &mut Vec<u8>, v: u32| o.extend_from_slice(&v.to_le_bytes());
    // ICONDIR
    le16(&mut out, 0);
    le16(&mut out, 1);
    le16(&mut out, 1);
    // ICONDIRENTRY
    out.extend_from_slice(&[N as u8, N as u8, 16, 0]);
    le16(&mut out, 1);
    le16(&mut out, 4);
    let bytes_in_res = 40 + 64 + (N * N / 2) + (N * N / 8);
    le32(&mut out, bytes_in_res as u32);
    le32(&mut out, 22);
    // BITMAPINFOHEADER
    le32(&mut out, 40);
    le32(&mut out, N as u32);
    le32(&mut out, 2 * N as u32);
    le16(&mut out, 1);
    le16(&mut out, 4);
    le32(&mut out, 0);
    le32(&mut out, ((N * N / 2) + (N * N / 8)) as u32);
    le32(&mut out, 0);
    le32(&mut out, 0);
    le32(&mut out, 0);
    le32(&mut out, 0);
    // the palette, BGR0
    for i in 0..16 {
        let c = pal.get(i).copied().unwrap_or([0, 0, 0]);
        out.extend_from_slice(&[c[2], c[1], c[0], 0]);
    }
    // the colour bitmap, bottom row first, two pixels a byte (the left one high)
    for y in (0..N).rev() {
        for x in (0..N).step_by(2) {
            let ix = |x: usize| pixels.get(y * N + x).copied().flatten().map_or(0, |c| nearest(&pal, c)) as u8;
            out.push((ix(x) << 4) | ix(x + 1));
        }
    }
    // the AND mask: 1 = transparent
    for y in (0..N).rev() {
        for byte in 0..N / 8 {
            let mut v = 0u8;
            for bit in 0..8 {
                let x = byte * 8 + bit;
                if pixels.get(y * N + x).copied().flatten().is_none() {
                    v |= 0x80 >> bit;
                }
            }
            out.push(v);
        }
    }
    out
}
