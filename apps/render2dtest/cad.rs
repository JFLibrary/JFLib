// cad -- the measuring scene: what a CAD drawing looks like, not a random walk.  A town plan in metres at surveyed
// coordinates (E 500 000, N 5 400 000): blocks in a grid with kerbs and pavements, streets as long polylines, parcels,
// buildings with double walls, inner walls, doors, windows, stairs, furniture and hatching, trees, labels written with
// short strokes, long utility lines through the town, contour lines of many short segments.  With arcs and dashes
// (G6a2b, not `plain`): doors swing in arcs, trees, columns and manholes are circles, the streets' axes dash-dotted in
// metres, plot borders and hidden edges dashed in pixels; each of these on layers of its own, as a CAD drawing keeps
// them (render2d draws such layers with its second pipeline).  With fills (G6b1, Mode::Fills): buildings filled, their
// walls (the band between the outer and the inner outline) hatched, every fourth street edge a spline, every fifth plot
// border a closed Bézier with rounded corners, an elliptic island at every crossing (filled, its outline an ellipse), every
// third tree crown an ellipse; the loose hatching lines go.  With blocks (G6b2, Mode::Blocks, the default): doors, windows
// (scaled unevenly to their length), stairs, furniture (unevenly to its size), trees (oval ones: an ellipse, flattened per
// zoom band), manholes (a ring and a filled cover), columns, lamps along the streets and the labels' characters (twenty
// glyphs) are blocks, each inserted wherever the drawing had its strokes; the rest as in Mode::Fills.  Deterministic: the
// same numbers give the same drawing on every machine and in the browser; Mode::Plain: the drawing of G6a2a, Mode::Arcs
// that of G6a2b, Mode::Fills that of G6b1, entity for entity.
//
//   build(&mut scene, records, seed, styled, mode) -> Cad   about `records` drawn records in the overview (an arc's head is
//                                                           none; a fill's and a curve's as flattened for the overview; an
//                                                           insert's its block's)
//   views(&cad, size)                                       the overview (all of it), a district, the working zoom
//   estate(&mut scene, inserts, styled) -> Cad              a block of about 1000 records (a house) inserted many times

use alloc::vec;
use alloc::vec::Vec;

use gpu::Result;
use render2d::math::sincos;
use render2d::{BlockId, Camera, Cap, EntityId, HatchLine, Insert, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Unit, ARC_STEP};

use crate::gallery::Rng;

/// where the drawing lies (metres): a surveyed coordinate, so the hi / lo pairs are in use
pub const ORIGIN: [f64; 2] = [500_000.0, 5_400_000.0];

/// the drawing of a G6 stage: G6a2a (lines only), G6a2b (arcs and dashes), G6b1 (fills, hatches, curves too), G6b2 (blocks;
/// Expanded: the same drawing, every insert's entities as entities of their own: what instancing costs)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Plain,
    Arcs,
    Fills,
    Blocks,
    Expanded,
}

/// an entity of a block as the CPU keeps it (Mode::Expanded draws inserts as entities of their own)
#[derive(Clone)]
enum Part {
    Line([f64; 2], [f64; 2]),
    Poly(Vec<[f64; 2]>, bool),
    Arc([f64; 2], f64, f64, f64),
    Circle([f64; 2], f64),
    Ellipse([f64; 2], [f64; 2], f64),
    Disc([f64; 2], f64),
}

/// what build made
#[derive(Clone, Debug)]
pub struct Cad {
    pub segments: u32,
    pub entities: u32,
    /// fills (of them hatched) and their records, curves and theirs (Mode::Fills)
    pub fills: u32,
    pub hatched: u32,
    pub fill_records: u32,
    pub curves: u32,
    pub curve_records: u32,
    /// of them arcs and circles, their pieces; dashed entities, their records
    pub arcs: u32,
    pub arc_records: u32,
    pub dashed: u32,
    pub dashed_records: u32,
    /// the drawing's size (metres), its lower left corner at ORIGIN
    pub width: f64,
    pub height: f64,
    /// (Mode::Blocks) blocks, inserts and their drawn records; the trees' inserts (moved by the measurement)
    pub blocks: u32,
    pub inserts: u32,
    pub insert_records: u32,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub trees: Vec<(EntityId, Insert)>,
}

struct Pen {
    l: LayerId,
    s: StyleId,
    /// its style has a dash pattern (asked once: Scene::style copies a Style, some 450 bytes since the hatches)
    dashed: bool,
}

/// the layers of the drawing in draw order, with their styles
struct Pens {
    contour: Pen,
    utility: Pen,
    street: Pen,
    centre: Pen,
    kerb: Pen,
    parcel: Pen,
    hatch: Pen,
    outer: Pen,
    inner: Pen,
    detail: Pen,
    furniture: Pen,
    tree: Pen,
    label: Pen,
    /// (not plain) doors' swings, trees' crowns, columns, manholes, hidden edges
    extra: Option<Extra>,
    /// (Mode::Fills) buildings' fills, hatched walls, islands' fills and outlines
    g6b: Option<G6b>,
    /// (Mode::Blocks) the blocks and the lamps' layer
    blocks: Option<Blocks>,
}

/// a block of the drawing, the records an insert of it draws in the overview, its entities (Part, style)
#[derive(Clone, Copy)]
struct Blk {
    id: BlockId,
    records: u32,
    def: usize,
}

struct Blocks {
    door: Blk,
    window: Blk,
    stair: Blk,
    table: Blk,
    tree: Blk,
    oval: Blk,
    manhole: Blk,
    column: Blk,
    lamp: Blk,
    glyphs: Vec<Blk>,
    lamps: Pen,
    /// any style: an insert's style gives nothing by block here
    any: StyleId,
    defs: Vec<Vec<(StyleId, Part)>>,
}

struct G6b {
    building: Pen,
    walls: Pen,
    island: Pen,
    isle_edge: Pen,
}

struct Extra {
    doors: Pen,
    crowns: Pen,
    columns: Pen,
    manholes: Pen,
    hidden: Pen,
}

struct Sink<'a> {
    s: &'a mut Scene,
    left: u32,
    ents: u32,
    arcs: u32,
    arc_records: u32,
    dashed: u32,
    dashed_records: u32,
    fills: u32,
    hatched: u32,
    fill_records: u32,
    curves: u32,
    curve_records: u32,
    inserts: u32,
    insert_records: u32,
    trees: Vec<(EntityId, Insert)>,
    /// (Mode::Expanded) the blocks' entities: each insert's as entities of their own
    expand: Option<Vec<Vec<(StyleId, Part)>>>,
}

impl Sink<'_> {
    /// an insert of a block (its records counted as an insert draws them); dropped when they do not fit
    fn ins(&mut self, l: LayerId, st: StyleId, b: Blk, ins: Insert) -> Result<Option<EntityId>> {
        let n = b.records * ins.columns * ins.rows;
        if self.left < n {
            return Ok(None);
        }
        self.left -= n;
        self.ents += 1;
        self.inserts += 1;
        self.insert_records += n;
        if let Some(defs) = &self.expand {
            // (its entities through its transform: the inserts of this drawing turn, move and scale evenly but for windows
            // and tables, which hold lines only)
            let (sn, cs) = sincos(ins.rotation);
            let (sx, sy) = (ins.scale[0], ins.scale[1]);
            let at = |p: [f64; 2]| [ins.at[0] + cs * sx * p[0] - sn * sy * p[1], ins.at[1] + sn * sx * p[0] + cs * sy * p[1]];
            for (ps, part) in &defs[b.def] {
                match part {
                    Part::Line(a, e) => self.s.insert(l, *ps, &Shape::Line { a: at(*a), b: at(*e) })?,
                    Part::Poly(v, c) => self.s.insert(l, *ps, &Shape::Polyline { points: &v.iter().map(|q| at(*q)).collect::<Vec<_>>(), closed: *c })?,
                    Part::Arc(c, r, a0, sw) => self.s.insert(l, *ps, &Shape::Arc { center: at(*c), radius: r * sx, start: ins.rotation + a0, sweep: *sw })?,
                    Part::Circle(c, r) => self.s.insert(l, *ps, &Shape::Circle { center: at(*c), radius: r * sx })?,
                    Part::Ellipse(c, rr, a0) => self.s.insert(l, *ps, &Shape::Ellipse { center: at(*c), radii: [rr[0] * sx, rr[1] * sx], rotation: ins.rotation + a0, start: 0.0,
                                                                                         sweep: core::f64::consts::TAU })?,
                    Part::Disc(c, r) => {
                        let ring = [Shape::Circle { center: at(*c), radius: r * sx }];
                        self.s.insert(l, *ps, &Shape::Fill { rings: &[&ring] })?
                    }
                };
            }
            return Ok(None);
        }
        let id = self.s.insert(l, st, &Shape::Insert(ins))?;
        Ok(Some(id))
    }

    /// a fill or a curve: its records as flattened for the camera (the overview's, set before); dropped when they do not fit
    fn flat(&mut self, p: &Pen, sh: &Shape, fill: bool) -> Result<()> {
        if self.left == 0 {
            return Ok(());
        }
        let id = self.s.insert(p.l, p.s, sh)?;
        let n = self.s.locate(id).map_or(0, |x| x.1);
        if n > self.left {
            self.s.remove(id)?;
            self.left = 0;
            return Ok(());
        }
        self.left -= n;
        self.ents += 1;
        if fill {
            self.fills += 1;
            self.fill_records += n;
            self.hatched += self.s.style(p.s).is_some_and(|st| st.hatches > 0) as u32;
        } else {
            self.curves += 1;
            self.curve_records += n;
            self.count_dashed(p, n);
        }
        Ok(())
    }

    /// (a polyline's records; dropped when the count is reached, cut short if only part fits)
    fn poly(&mut self, p: &Pen, pts: &[[f64; 2]], closed: bool) -> Result<()> {
        if self.left == 0 || pts.len() < 2 {
            return Ok(());
        }
        let n = if closed { pts.len() } else { pts.len() - 1 } as u32;
        let ins = n.min(self.left);
        if n > self.left {
            let keep = self.left as usize + 1;
            self.s.insert(p.l, p.s, &Shape::Polyline { points: &pts[..keep.min(pts.len())], closed: false })?;
            self.left = 0;
        } else {
            self.s.insert(p.l, p.s, &Shape::Polyline { points: pts, closed })?;
            self.left -= n;
        }
        self.ents += 1;
        self.count_dashed(p, ins);
        Ok(())
    }

    fn line(&mut self, p: &Pen, a: [f64; 2], b: [f64; 2]) -> Result<()> {
        if self.left == 0 {
            return Ok(());
        }
        self.s.insert(p.l, p.s, &Shape::Line { a, b })?;
        self.left -= 1;
        self.ents += 1;
        self.count_dashed(p, 1);
        Ok(())
    }

    fn count_dashed(&mut self, p: &Pen, n: u32) {
        if p.dashed {
            self.dashed += 1;
            self.dashed_records += n;
        }
    }

    /// an arc (a circle: sweep TAU), its pieces counted; dropped when they do not fit
    fn arc(&mut self, p: &Pen, c: [f64; 2], r: f64, a0: f64, sweep: f64) -> Result<()> {
        let n = ((sweep.abs() / ARC_STEP - 1e-9).max(0.0) as u32 + 1).min(16);
        if self.left < n {
            return Ok(());
        }
        let shape = if sweep.abs() >= core::f64::consts::TAU { Shape::Circle { center: c, radius: r } } else { Shape::Arc { center: c, radius: r, start: a0, sweep } };
        self.s.insert(p.l, p.s, &shape)?;
        self.left -= n;
        self.ents += 1;
        self.arcs += 1;
        self.arc_records += n;
        self.count_dashed(p, n);
        Ok(())
    }
}

fn pens(s: &mut Scene, styled: bool, mode: Mode) -> Result<Pens> {
    let plain = mode == Mode::Plain;
    let fills = mode == Mode::Fills || mode == Mode::Blocks || mode == Mode::Expanded;
    let mut order = 0;
    // (a dash pattern: lengths, its unit, its phase; in the plain drawing none)
    let mut pen = |s: &mut Scene, color: u32, width: f32, unit: Unit, cap: Cap, join: Join, dash: (&[f32], Unit)| -> Result<Pen> {
        let l = s.add_layer(&Layer::new(order))?;
        order += 1;
        let mut st = Style::new(color);
        if styled {
            st.width = width;
            st.unit = unit;
            st.cap = cap;
            st.join = join;
        }
        if !plain {
            st = st.dashed(dash.0, dash.1, 0.0);
        }
        Ok(Pen { l, s: s.add_style(&st)?, dashed: st.dashes > 0 })
    };
    let (b, r, m) = (Cap::Butt, Cap::Round, Join::Miter);
    let jr = Join::Round;
    let solid: (&[f32], Unit) = (&[], Unit::Px);
    let contour = pen(s, 0xFF9F_B39A, 0.0, Unit::Px, b, m, solid)?;
    let utility = pen(s, 0xFFC0_6070, 2.0, Unit::Px, b, m, solid)?;
    // (G6b1: the islands at the crossings under the streets)
    let island = if fills { Some(Pen { l: s.add_layer(&Layer::new(-2))?, s: s.add_style(&Style::new(0xFFB8_D8A0))?, dashed: false }) } else { None };
    let street = pen(s, 0xFF55_5555, 3.0, Unit::Px, b, m, solid)?;
    // (axes dash-dotted in metres, plot borders dashed in pixels)
    let centre = pen(s, 0xA060_6060, 0.0, Unit::Px, b, m, (&[6.0, 1.5, 0.8, 1.5], Unit::Drawing))?;
    let kerb = pen(s, 0xFF70_7070, 1.0, Unit::Px, b, jr, solid)?;
    let parcel = pen(s, 0xFFA8_9070, 0.0, Unit::Px, b, m, (&[9.0, 3.0, 2.0, 3.0], Unit::Px))?;
    // (G6b1: the buildings' fills over the plots, under their walls)
    let building = if fills { Some(Pen { l: s.add_layer(&Layer::new(-1))?, s: s.add_style(&Style::new(0xFFE6_DCC8))?, dashed: false }) } else { None };
    let hatch = pen(s, 0x6030_4050, 0.0, Unit::Px, b, m, solid)?;
    let mut p = Pens {
        contour,
        utility,
        street,
        centre,
        kerb,
        parcel,
        hatch,
        outer: pen(s, 0xFF20_2830, 0.3, Unit::Drawing, b, m, solid)?,
        inner: pen(s, 0xFF40_4850, 0.12, Unit::Drawing, b, m, solid)?,
        detail: pen(s, 0xFF50_60A0, 0.0, Unit::Px, b, m, solid)?,
        furniture: pen(s, 0xFF90_9090, 0.0, Unit::Px, b, m, solid)?,
        tree: pen(s, 0xFF30_8040, 0.0, Unit::Px, r, jr, solid)?,
        label: pen(s, 0xFF20_2020, 1.0, Unit::Px, r, jr, solid)?,
        extra: None,
        g6b: None,
        blocks: None,
    };
    if !plain {
        p.extra = Some(Extra {
            doors: pen(s, 0xFF50_60A0, 0.0, Unit::Px, b, m, solid)?,
            crowns: pen(s, 0xFF30_8040, 0.0, Unit::Px, r, jr, solid)?,
            columns: pen(s, 0xFF20_2830, 0.08, Unit::Drawing, b, m, solid)?,
            manholes: pen(s, 0xFF60_6060, 1.0, Unit::Px, b, m, solid)?,
            hidden: pen(s, 0xFF60_6878, 0.0, Unit::Px, b, m, (&[5.0, 3.0], Unit::Px))?,
        });
    }
    if let (Some(building), Some(island)) = (building, island) {
        // the walls hatched at 45 degrees, a line every 0.25 m (ANSI31's look), hairlines; the islands' outlines
        let mut wall = Style::new(0xFF40_4850);
        wall = wall.hatched(&[HatchLine::new(core::f64::consts::FRAC_PI_4, 0.25)], 0.0);
        let walls = Pen { l: s.add_layer(&Layer::new(order))?, s: s.add_style(&wall)?, dashed: false };
        let mut e = Style::new(0xFF70_8060);
        if styled {
            e.width = 1.0;
        }
        let isle_edge = Pen { l: s.add_layer(&Layer::new(order + 1))?, s: s.add_style(&e)?, dashed: false };
        // (the walls with the loose hatching's order, added after it: drawn after it, before the outlines)
        let mut l = Layer::new(0);
        l.order = s.layer(p.hatch.l)?.order;
        s.set_layer(walls.l, &l)?;
        p.g6b = Some(G6b { building, walls, island, isle_edge });
    }
    if mode == Mode::Blocks || mode == Mode::Expanded {
        p.blocks = Some(blocks(s, &p, styled, &mut order)?);
    }
    Ok(p)
}

/// the blocks of Mode::Blocks: each in its own units (a door of radius 1, a window and a table of 1 by 1, a tree of radius
/// 1, ...), its entities with the styles their strokes had on their layers
fn blocks(s: &mut Scene, p: &Pens, styled: bool, order: &mut i32) -> Result<Blocks> {
    let x = p.extra.as_ref().ok_or(gpu::GpuError::Invalid("cad: blocks come with arcs"))?;
    let h = core::f64::consts::FRAC_PI_2;
    let cover = s.add_style(&Style::new(0xFF80_8080))?;
    let mut ls = Style::new(0xFF90_7020);
    if styled {
        ls.width = 1.0;
    }
    let ls = s.add_style(&ls)?;
    let (d, f, cr, tr) = (p.detail.s, p.furniture.s, x.crowns.s, p.tree.s);
    let cross = |v: &mut Vec<(StyleId, Part)>| {
        v.push((tr, Part::Line([-0.5, 0.0], [0.5, 0.0])));
        v.push((tr, Part::Line([0.0, -0.5], [0.0, 0.5])));
    };
    let mut defs: Vec<Vec<(StyleId, Part)>> = Vec::new();
    // a door: its leaf and its swing (5 drawn records)
    defs.push(vec![(d, Part::Line([0.0, 0.0], [1.0, 0.0])), (x.doors.s, Part::Arc([0.0, 0.0], 1.0, 0.0, h))]);
    // a window: three lines 1 long, 0.1 apart (scaled to its length along x only)
    defs.push((0..3).map(|j| (d, Part::Line([-0.5, j as f64 * 0.1 - 0.1], [0.5, j as f64 * 0.1 - 0.1]))).collect());
    // a stair: eleven steps
    defs.push((0..11).map(|j| (d, Part::Line([j as f64 * 0.26, 0.0], [j as f64 * 0.26, 1.1]))).collect());
    // a table: a square of 1 (scaled to its size)
    defs.push(vec![(f, Part::Poly(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], true))]);
    // trees: a crown of radius 1 and its cross; an oval one (an ellipse: flattened per zoom band)
    let mut tree = vec![(cr, Part::Circle([0.0, 0.0], 1.0))];
    cross(&mut tree);
    defs.push(tree);
    let mut oval = vec![(cr, Part::Ellipse([0.0, 0.0], [1.0, 0.7], 0.0))];
    cross(&mut oval);
    defs.push(oval);
    // a manhole: its ring and its cover (a fill); a column
    defs.push(vec![(x.manholes.s, Part::Circle([0.0, 0.0], 0.4)), (cover, Part::Disc([0.0, 0.0], 0.25))]);
    defs.push(vec![(x.columns.s, Part::Circle([0.0, 0.0], 0.2))]);
    // a lamp: a ring and four rays
    let mut lamp = vec![(ls, Part::Circle([0.0, 0.0], 0.3))];
    for k in 0..4 {
        let (sn, cs) = sincos(k as f64 * h + 0.25 * h);
        lamp.push((ls, Part::Line([0.4 * cs, 0.4 * sn], [0.7 * cs, 0.7 * sn])));
    }
    defs.push(lamp);
    // twenty glyphs: a stroke of four to six pieces in a box of 1 by 1.6 (the labels' characters)
    let mut rng = Rng(77);
    for _ in 0..20 {
        let n = 4 + rng.below(3) as usize;
        defs.push(vec![(p.label.s, Part::Poly((0..n).map(|_| [rng.range(0.0, 1.0), rng.range(0.0, 1.6)]).collect(), false))]);
    }
    let mut ids = Vec::with_capacity(defs.len());
    for def in &defs {
        let b = s.add_block(&render2d::Block::new())?;
        for (st, part) in def {
            with_part(part, |sh| s.insert_in(b, *st, sh))?;
        }
        ids.push(b);
    }
    let b = |k: usize, records| Blk { id: ids[k], records, def: k };
    let glyphs = (9..29).map(|k| b(k, match &defs[k][0].1 { Part::Poly(v, _) => v.len() as u32 - 1, _ => 1 })).collect();
    let lamps = Pen { l: s.add_layer(&Layer::new(*order))?, s: ls, dashed: false };
    *order += 1;
    Ok(Blocks { door: b(0, 5), window: b(1, 3), stair: b(2, 11), table: b(3, 4), tree: b(4, 18), oval: b(5, 26), manhole: b(6, 26), column: b(7, 16), lamp: b(8, 20), glyphs, lamps, any: p.detail.s,
                defs })
}

/// f with the shape of a block's entity
fn with_part<R>(part: &Part, f: impl FnOnce(&Shape) -> R) -> R {
    let tau = core::f64::consts::TAU;
    match part {
        Part::Line(a, b) => f(&Shape::Line { a: *a, b: *b }),
        Part::Poly(v, c) => f(&Shape::Polyline { points: v, closed: *c }),
        Part::Arc(c, r, a0, sw) => f(&Shape::Arc { center: *c, radius: *r, start: *a0, sweep: *sw }),
        Part::Circle(c, r) => f(&Shape::Circle { center: *c, radius: *r }),
        Part::Ellipse(c, rr, rot) => f(&Shape::Ellipse { center: *c, radii: *rr, rotation: *rot, start: 0.0, sweep: tau }),
        Part::Disc(c, r) => {
            let ring = [Shape::Circle { center: *c, radius: *r }];
            f(&Shape::Fill { rings: &[&ring] })
        }
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> [[f64; 2]; 4] {
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

/// a footprint: a rectangle with notches on its sides, counter-clockwise
fn footprint(rng: &mut Rng, x: f64, y: f64, w: f64, h: f64, out: &mut Vec<[f64; 2]>) {
    out.clear();
    out.push([x, y]);
    // the bottom side, the right side, the top, the left, each with a notch or not
    let mut side = |rng: &mut Rng, a: [f64; 2], b: [f64; 2], n: [f64; 2]| {
        if rng.below(2) == 0 {
            let (t0, t1) = (rng.range(0.2, 0.45), rng.range(0.55, 0.8));
            let d = rng.range(0.6, 1.8);
            let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (q0, q1) = (p(t0), p(t1));
            out.push(q0);
            out.push([q0[0] + n[0] * d, q0[1] + n[1] * d]);
            out.push([q1[0] + n[0] * d, q1[1] + n[1] * d]);
            out.push(q1);
        }
        out.push(b);
    };
    side(rng, [x, y], [x + w, y], [0.0, 1.0]);
    side(rng, [x + w, y], [x + w, y + h], [-1.0, 0.0]);
    side(rng, [x + w, y + h], [x, y + h], [0.0, -1.0]);
    side(rng, [x, y + h], [x, y], [1.0, 0.0]);
    out.pop();
}

fn parcel(k: &mut Sink, rng: &mut Rng, p: &Pens, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
    // the building
    let m = rng.range(1.2, 3.0);
    let (bx, by, bw, bh) = (x + m, y + m, w - 2.0 * m, h - 2.0 * m);
    let mut pts = Vec::new();
    footprint(rng, bx, by, bw, bh, &mut pts);
    let inset: Vec<[f64; 2]> = pts.iter().map(|q| [q[0] + (bx + 0.5 * bw - q[0]).signum() * 0.3, q[1] + (by + 0.5 * bh - q[1]).signum() * 0.3]).collect();
    if let Some(g) = &p.g6b {
        // the building filled, its walls (between both outlines) hatched
        let (o, i) = ([Shape::Polyline { points: &pts, closed: true }], [Shape::Polyline { points: &inset, closed: true }]);
        k.flat(&g.building, &Shape::Fill { rings: &[&o] }, true)?;
        k.flat(&g.walls, &Shape::Fill { rings: &[&o, &i] }, true)?;
    }
    k.poly(&p.outer, &pts, true)?;
    k.poly(&p.inner, &inset, true)?;
    // inner walls
    for _ in 0..6 + rng.below(6) {
        let (px, py) = (rng.range(bx + 0.5, bx + bw - 0.5), rng.range(by + 0.5, by + bh - 0.5));
        let l = rng.range(1.5, 6.0);
        if rng.below(2) == 0 {
            k.line(&p.inner, [px, py], [(px + l).min(bx + bw - 0.3), py])?;
        } else {
            k.line(&p.inner, [px, py], [px, (py + l).min(by + bh - 0.3)])?;
        }
    }
    // doors: the leaf and its swing (plain: six short segments; else an arc of a quarter turn)
    for _ in 0..3 + rng.below(4) {
        let (px, py) = (rng.range(bx + 1.0, bx + bw - 1.0), rng.range(by + 1.0, by + bh - 1.0));
        let r = rng.range(0.75, 1.0);
        let a0 = rng.below(4) as f64 * core::f64::consts::FRAC_PI_2;
        if let (Some(b), Some(x)) = (&p.blocks, &p.extra) {
            let mut ins = Insert::new(b.door.id, [px, py]);
            ins.rotation = a0;
            ins.scale = [r, r];
            k.ins(x.doors.l, b.any, b.door, ins)?;
            continue;
        }
        let (s, c) = sincos(a0);
        k.line(&p.detail, [px, py], [px + r * c, py + r * s])?;
        if let Some(x) = &p.extra {
            k.arc(&x.doors, [px, py], r, a0, core::f64::consts::FRAC_PI_2)?;
        } else {
            let arc: Vec<[f64; 2]> = (0..7).map(|j| {
                let (s, c) = sincos(a0 + j as f64 / 6.0 * core::f64::consts::FRAC_PI_2);
                [px + r * c, py + r * s]
            }).collect();
            k.poly(&p.detail, &arc, false)?;
        }
    }
    // windows on the outer walls: three short lines each
    for _ in 0..8 + rng.below(7) {
        let t = rng.range(0.1, 0.9);
        let (px, py) = if rng.below(2) == 0 { (bx + t * bw, if rng.below(2) == 0 { by } else { by + bh }) } else { (if rng.below(2) == 0 { bx } else { bx + bw }, by + t * bh) };
        let l = rng.range(0.6, 1.4);
        if let Some(b) = &p.blocks {
            // (a window scaled to its length, along x only)
            let mut ins = Insert::new(b.window.id, [px, py]);
            ins.scale = [l, 1.0];
            k.ins(p.detail.l, b.any, b.window, ins)?;
            continue;
        }
        for j in 0..3 {
            let off = j as f64 * 0.1 - 0.1;
            k.line(&p.detail, [px - 0.5 * l, py + off], [px + 0.5 * l, py + off])?;
        }
    }
    // a stair
    if rng.below(3) == 0 {
        let (px, py) = (rng.range(bx + 1.0, bx + bw - 3.5), rng.range(by + 1.0, by + bh - 2.5));
        if let Some(b) = &p.blocks {
            k.ins(p.detail.l, b.any, b.stair, Insert::new(b.stair.id, [px, py]))?;
        } else {
            for j in 0..11 {
                k.line(&p.detail, [px + j as f64 * 0.26, py], [px + j as f64 * 0.26, py + 1.1])?;
            }
        }
    }
    // furniture
    for _ in 0..3 + rng.below(6) {
        let (fw, fh) = (rng.range(0.5, 2.0), rng.range(0.5, 1.2));
        let (fx, fy) = (rng.range(bx + 0.5, bx + bw - 2.5), rng.range(by + 0.5, by + bh - 1.5));
        if let Some(b) = &p.blocks {
            // (a table scaled to its size, unevenly)
            let mut ins = Insert::new(b.table.id, [fx, fy]);
            ins.scale = [fw, fh];
            k.ins(p.furniture.l, b.any, b.table, ins)?;
            continue;
        }
        let r = rect(fx, fy, fw, fh);
        k.poly(&p.furniture, &r, true)?;
    }
    // hatching over part of it: diagonal lines of a few metres (G6b1: the walls are hatched instead)
    let (hx, hy) = (rng.range(bx, bx + 0.4 * bw), rng.range(by, by + 0.4 * bh));
    for j in 0..8 + rng.below(13) {
        let t = j as f64 * 0.45;
        let l = rng.range(2.0, 9.0).min(bw * 0.6).min(bh * 0.6);
        if p.g6b.is_none() {
            k.line(&p.hatch, [hx + t, hy], [hx + t + l, hy + l])?;
        }
    }
    if let Some(x) = &p.extra {
        // columns in a row in one building of four; an edge hidden above (a canopy over the entrance) in every second
        if rng.below(4) == 0 {
            let (cy, n) = (by + 0.5 * bh, 3 + rng.below(3));
            for j in 0..n {
                let cx = bx + (j as f64 + 0.5) / n as f64 * bw;
                if let Some(b) = &p.blocks {
                    k.ins(x.columns.l, b.any, b.column, Insert::new(b.column.id, [cx, cy]))?;
                } else {
                    k.arc(&x.columns, [cx, cy], 0.2, 0.0, core::f64::consts::TAU)?;
                }
            }
        }
        if rng.below(2) == 0 {
            let (cw, ch) = (rng.range(1.5, 4.0).min(bw - 1.0), rng.range(1.0, 2.5));
            let cx = rng.range(bx + 0.5, bx + bw - 0.5 - cw);
            k.poly(&x.hidden, &rect(cx, by - 0.4 * ch, cw, ch), true)?;
        }
    }
    Ok(())
}

fn block(k: &mut Sink, rng: &mut Rng, p: &Pens, x0: f64, y0: f64, w: f64, h: f64) -> Result<()> {
    // kerb and pavement: closed polylines with rounded corners
    for inset in [0.0, 2.5] {
        let (x, y, bw, bh) = (x0 + inset, y0 + inset, w - 2.0 * inset, h - 2.0 * inset);
        let r = 4.0 - inset * 0.8;
        let mut pts = Vec::with_capacity(16);
        for (cx, cy, a0) in [(x + bw - r, y + r, -core::f64::consts::FRAC_PI_2), (x + bw - r, y + bh - r, 0.0), (x + r, y + bh - r, core::f64::consts::FRAC_PI_2), (x + r, y + r, core::f64::consts::PI)] {
            for j in 0..4 {
                let (s, c) = sincos(a0 + j as f64 / 3.0 * core::f64::consts::FRAC_PI_2);
                pts.push([cx + r * c, cy + r * s]);
            }
        }
        k.poly(&p.kerb, &pts, true)?;
    }
    // parcels in two rows
    let cols = 6 + rng.below(5);
    let (ix, iy, iw, ih) = (x0 + 3.0, y0 + 3.0, w - 6.0, h - 6.0);
    let pw = iw / cols as f64;
    for row in 0..2 {
        let py = iy + row as f64 * 0.5 * ih;
        for c in 0..cols {
            let px = ix + c as f64 * pw;
            if p.g6b.is_some() && (row as u32 * cols + c) % 5 == 0 {
                // (a plot with rounded corners: a closed cubic Bézier, corners of radius 1.5 m)
                let (x0, y0, x1, y1, rr) = (px, py, px + pw, py + 0.5 * ih, 1.5);
                let kk = 0.5523 * rr;
                let b = [[x0 + rr, y0], [x1 - rr, y0], [x1 - rr + kk, y0], [x1, y0 + rr - kk], [x1, y0 + rr], [x1, y1 - rr], [x1, y1 - rr + kk], [x1 - rr + kk, y1], [x1 - rr, y1], [x0 + rr, y1],
                         [x0 + rr - kk, y1], [x0, y1 - rr + kk], [x0, y1 - rr], [x0, y0 + rr], [x0, y0 + rr - kk], [x0 + rr - kk, y0], [x0 + rr, y0]];
                // (straight sides as Bézier pieces too: their controls on the side)
                let mut pts = Vec::with_capacity(25);
                pts.push(b[0]);
                for side in 0..4 {
                    let (a, e) = (b[side * 4], b[side * 4 + 1]);
                    pts.extend_from_slice(&[[a[0] + (e[0] - a[0]) / 3.0, a[1] + (e[1] - a[1]) / 3.0], [a[0] + (e[0] - a[0]) * 2.0 / 3.0, a[1] + (e[1] - a[1]) * 2.0 / 3.0], e]);
                    pts.extend_from_slice(&[b[side * 4 + 2], b[side * 4 + 3], b[side * 4 + 4]]);
                }
                k.flat(&p.parcel, &Shape::Cubic { points: &pts, closed: true }, false)?;
            } else {
                k.poly(&p.parcel, &rect(px, py, pw, 0.5 * ih), true)?;
            }
            parcel(k, rng, p, px, py, pw, 0.5 * ih)?;
        }
    }
    // trees: a crown (plain: twelve-sided; else a circle) and its cross, along the pavement
    for _ in 0..8 + rng.below(9) {
        let (tx, ty) = (rng.range(x0 - 4.0, x0 + w + 4.0), if rng.below(2) == 0 { y0 - 3.0 } else { y0 + h + 3.0 });
        let r = rng.range(1.5, 4.0);
        let oval = p.g6b.is_some() && rng.below(3) == 0;
        if let (Some(b), Some(x)) = (&p.blocks, &p.extra) {
            // (a tree scaled to its crown, an oval one turned too)
            let blk = if oval { b.oval } else { b.tree };
            let mut ins = Insert::new(blk.id, [tx, ty]);
            ins.scale = [r, r];
            if oval {
                ins.rotation = rng.range(0.0, 3.0);
            }
            if let Some(id) = k.ins(x.crowns.l, b.any, blk, ins)? {
                k.trees.push((id, ins));
            }
            continue;
        }
        if let (Some(x), true) = (&p.extra, oval) {
            // (an oval crown)
            let rot = rng.range(0.0, 3.0);
            k.flat(&x.crowns, &Shape::Ellipse { center: [tx, ty], radii: [r, 0.7 * r], rotation: rot, start: 0.0, sweep: core::f64::consts::TAU }, false)?;
        } else if let Some(x) = &p.extra {
            k.arc(&x.crowns, [tx, ty], r, 0.0, core::f64::consts::TAU)?;
        } else {
            let crown: Vec<[f64; 2]> = (0..12).map(|j| {
                let (s, c) = sincos(j as f64 / 12.0 * core::f64::consts::TAU);
                [tx + r * c, ty + r * s]
            }).collect();
            k.poly(&p.tree, &crown, true)?;
        }
        k.line(&p.tree, [tx - 0.5 * r, ty], [tx + 0.5 * r, ty])?;
        k.line(&p.tree, [tx, ty - 0.5 * r], [tx, ty + 0.5 * r])?;
    }
    // labels: a character is one stroke of a few short pieces in a box of 1 by 1.6 m
    for _ in 0..6 + rng.below(5) {
        let (mut lx, ly) = (rng.range(x0 + 4.0, x0 + w - 12.0), rng.range(y0 + 4.0, y0 + h - 4.0));
        for _ in 0..4 + rng.below(5) {
            if let Some(b) = &p.blocks {
                // (a character: one of the twenty glyphs)
                let g = b.glyphs[rng.below(b.glyphs.len() as u32) as usize];
                k.ins(p.label.l, b.any, g, Insert::new(g.id, [lx, ly]))?;
                lx += 1.4;
                continue;
            }
            let n = 4 + rng.below(3) as usize;
            let ch: Vec<[f64; 2]> = (0..n).map(|_| [lx + rng.range(0.0, 1.0), ly + rng.range(0.0, 1.6)]).collect();
            k.poly(&p.label, &ch, false)?;
            lx += 1.4;
        }
    }
    Ok(())
}

/// a polyline along x from x0 to x1 that wanders by amp around y, a point every `step`
fn contour(rng: &mut Rng, x0: f64, x1: f64, y: f64, amp: f64, step: f64, pts: &mut Vec<[f64; 2]>) {
    pts.clear();
    let (mut x, mut v) = (x0, 0.0f64);
    let mut yy = y;
    while x < x1 {
        pts.push([x, yy]);
        v = (v + rng.range(-0.25, 0.25) * amp).clamp(-amp, amp);
        yy += v;
        x += step;
    }
}

/// a drawing of exactly `n` segments (fewer entities than a random walk has: a CAD drawing has long lines and many small
/// details).  styled: the widths of a plan (streets 3 px, walls in metres, ...), else every stroke a hairline
pub fn build(s: &mut Scene, n: u32, seed: u32, styled: bool, mode: Mode) -> Result<Cad> {
    let p = pens(s, styled, mode)?;
    let mut rng = Rng(seed);
    let (pitch_x, pitch_y) = (140.0, 100.0);
    // about 2500 segments a block; contour lines, streets and utilities take the rest
    let blocks = (0.88 * n as f64 / 2500.0) as u32 + 1;
    let cols = ((render2d::math::sqrt(1.27 * blocks as f64) + 0.999) as u32).max(1);
    let rows = blocks.div_ceil(cols);
    let (width, height) = (cols as f64 * pitch_x + 20.0, rows as f64 * pitch_y + 20.0);
    // (fills and curves are flattened for the camera they come with: the overview's at full HD)
    if mode == Mode::Fills || mode == Mode::Blocks || mode == Mode::Expanded {
        s.set_camera(&views(&Cad { segments: n, entities: 0, fills: 0, hatched: 0, fill_records: 0, curves: 0, curve_records: 0, arcs: 0, arc_records: 0, dashed: 0, dashed_records: 0, width, height,
                                   blocks: 0, inserts: 0, insert_records: 0, trees: Vec::new() },
                            (1920, 1080))[0].cam)?;
    }
    let expand = if mode == Mode::Expanded { p.blocks.as_ref().map(|b| b.defs.clone()) } else { None };
    let mut k = Sink { s, left: n, ents: 0, arcs: 0, arc_records: 0, dashed: 0, dashed_records: 0, fills: 0, hatched: 0, fill_records: 0, curves: 0, curve_records: 0, inserts: 0, insert_records: 0,
                       trees: Vec::new(), expand };
    let at = |x: f64, y: f64| [ORIGIN[0] + x, ORIGIN[1] + y];
    // streets: both edges and the centre line of each, long polylines with a point per block
    let road = 12.0;
    for c in 0..=cols {
        let x = 10.0 + c as f64 * pitch_x - 0.5 * road;
        for (dx, pen) in [(0.0, &p.street), (0.5 * road, &p.centre), (road, &p.street)] {
            let pts: Vec<[f64; 2]> = (0..=rows).map(|r| at(x + dx + rng.range(-0.2, 0.2), 10.0 + r as f64 * pitch_y)).collect();
            if p.g6b.is_some() && c % 4 == 1 && dx != 0.5 * road && pts.len() >= 4 {
                // (a street edge as a spline through its points' neighbourhood: a clamped cubic B-spline)
                k.flat(pen, &Shape::Spline { degree: 3, points: &pts, weights: &[], knots: &[], closed: false }, false)?;
            } else {
                k.poly(pen, &pts, false)?;
            }
        }
    }
    for r in 0..=rows {
        let y = 10.0 + r as f64 * pitch_y - 0.5 * road;
        for (dy, pen) in [(0.0, &p.street), (0.5 * road, &p.centre), (road, &p.street)] {
            let pts: Vec<[f64; 2]> = (0..=cols).map(|c| at(10.0 + c as f64 * pitch_x, y + dy + rng.range(-0.2, 0.2))).collect();
            k.poly(pen, &pts, false)?;
        }
    }
    // islands at the crossings (G6b1): an ellipse, filled and outlined
    if let Some(g) = &p.g6b {
        for c in 0..=cols {
            for r in 0..=rows {
                let ctr = at(10.0 + c as f64 * pitch_x, 10.0 + r as f64 * pitch_y);
                let rot = rng.range(0.0, 3.1);
                let e = Shape::Ellipse { center: ctr, radii: [3.2, 2.0], rotation: rot, start: 0.0, sweep: core::f64::consts::TAU };
                k.flat(&g.island, &Shape::Fill { rings: &[&[e]] }, true)?;
                k.flat(&g.isle_edge, &e, false)?;
            }
        }
    }
    // manholes on the streets (not plain): a circle every 35 m along each street's axis (blocks: a ring and its cover; a
    // lamp every 25 m beside the street)
    if let Some(x) = &p.extra {
        for c in 0..=cols {
            for j in 0..(rows as f64 * pitch_y / 35.0) as u32 {
                let m = at(10.0 + c as f64 * pitch_x, 12.0 + j as f64 * 35.0);
                if let Some(b) = &p.blocks {
                    k.ins(x.manholes.l, b.any, b.manhole, Insert::new(b.manhole.id, m))?;
                } else {
                    k.arc(&x.manholes, m, 0.4, 0.0, core::f64::consts::TAU)?;
                }
            }
            if let Some(b) = &p.blocks {
                for j in 0..(rows as f64 * pitch_y / 25.0) as u32 {
                    let l = at(10.0 + c as f64 * pitch_x + 0.5 * road + 1.0, 20.0 + j as f64 * 25.0);
                    k.ins(b.lamps.l, b.any, b.lamp, Insert::new(b.lamp.id, l))?;
                }
            }
        }
    }
    // long utility lines through the town, straight: a few hundred metres to its full size
    for _ in 0..n / 400 {
        let a = [rng.range(0.0, width), rng.range(0.0, height)];
        let l = rng.range(150.0, 0.5 * (width + height));
        let (sn, cs) = sincos(rng.range(0.0, core::f64::consts::TAU));
        let b = [(a[0] + l * cs).clamp(0.0, width), (a[1] + l * sn).clamp(0.0, height)];
        k.line(&p.utility, at(a[0], a[1]), at(b[0], b[1]))?;
    }
    // the blocks
    for r in 0..rows {
        for c in 0..cols {
            if r * cols + c >= blocks {
                break;
            }
            let (x0, y0) = (10.0 + c as f64 * pitch_x + 0.5 * road + 4.0, 10.0 + r as f64 * pitch_y + 0.5 * road + 4.0);
            // (the block's content in drawing coordinates: moved by the origin on its way out of the sink)
            block(&mut k, &mut rng, &p, ORIGIN[0] + x0, ORIGIN[1] + y0, pitch_x - road - 8.0, pitch_y - road - 8.0)?;
        }
    }
    // whatever is left: contour lines over the whole of it, a point every few metres
    let mut pts = Vec::new();
    let mut y = 0.0;
    while k.left > 0 {
        let (start, end, step) = (rng.range(0.0, 0.2 * width), width - rng.range(0.0, 0.2 * width), rng.range(2.0, 6.0));
        contour(&mut rng, start, end, y, 0.8, step, &mut pts);
        let moved: Vec<[f64; 2]> = pts.iter().map(|q| at(q[0], q[1])).collect();
        k.poly(&p.contour, &moved, false)?;
        y += rng.range(0.5, 5.0);
        if y > height {
            y -= height;
        }
    }
    let blocks = p.blocks.as_ref().map_or(0, |b| 9 + b.glyphs.len() as u32);
    Ok(Cad { segments: n, entities: k.ents, fills: k.fills, hatched: k.hatched, fill_records: k.fill_records, curves: k.curves, curve_records: k.curve_records, arcs: k.arcs, arc_records: k.arc_records,
             dashed: k.dashed, dashed_records: k.dashed_records, width, height, blocks, inserts: k.inserts, insert_records: k.insert_records, trees: k.trees })
}

/// a housing estate: one block of about 1000 records (a house: its outline, walls, doors swinging in arcs, windows,
/// furniture, a stair) inserted `inserts` times in a grid, each turned a little; far more records than the pool would hold
/// as plain records (G6b2's measurement of a large block inserted many times)
pub fn estate(s: &mut Scene, inserts: u32, styled: bool) -> Result<Cad> {
    let mut st = |c: u32, w: f32, u: Unit| -> Result<StyleId> {
        let mut x = Style::new(c);
        if styled {
            x.width = w;
            x.unit = u;
        }
        s.add_style(&x)
    };
    let (outer, inner, detail, furn, ist) = (st(0xFF20_2830, 0.3, Unit::Drawing)?, st(0xFF40_4850, 0.12, Unit::Drawing)?, st(0xFF50_60A0, 0.0, Unit::Px)?, st(0xFF90_9090, 0.0, Unit::Px)?,
                                             st(0xFF00_0000, 0.0, Unit::Px)?);
    let house = s.add_block(&render2d::Block::new())?;
    let mut rng = Rng(42);
    let mut records = 0u32;
    // the outline and its inset, 18 by 12 m
    let o = [[0.0, 0.0], [18.0, 0.0], [18.0, 12.0], [0.0, 12.0]];
    let i = [[0.3, 0.3], [17.7, 0.3], [17.7, 11.7], [0.3, 11.7]];
    s.insert_in(house, outer, &Shape::Polyline { points: &o, closed: true })?;
    s.insert_in(house, inner, &Shape::Polyline { points: &i, closed: true })?;
    records += 8;
    while records < 1000 {
        match rng.below(6) {
            0 => {
                // an inner wall
                let (x, y, l) = (rng.range(1.0, 17.0), rng.range(1.0, 11.0), rng.range(1.0, 5.0));
                let b = if rng.below(2) == 0 { [(x + l).min(17.7), y] } else { [x, (y + l).min(11.7)] };
                s.insert_in(house, inner, &Shape::Line { a: [x, y], b })?;
                records += 1;
            }
            1 => {
                // a door: its leaf and its swing
                let (x, y, r) = (rng.range(1.0, 17.0), rng.range(1.0, 11.0), rng.range(0.75, 1.0));
                let a0 = rng.below(4) as f64 * core::f64::consts::FRAC_PI_2;
                let (sn, cs) = sincos(a0);
                s.insert_in(house, detail, &Shape::Line { a: [x, y], b: [x + r * cs, y + r * sn] })?;
                s.insert_in(house, detail, &Shape::Arc { center: [x, y], radius: r, start: a0, sweep: core::f64::consts::FRAC_PI_2 })?;
                records += 5;
            }
            2 | 3 => {
                // furniture
                let (x, y, w, h) = (rng.range(0.5, 16.0), rng.range(0.5, 10.5), rng.range(0.5, 2.0), rng.range(0.5, 1.2));
                s.insert_in(house, furn, &Shape::Polyline { points: &rect(x, y, w, h), closed: true })?;
                records += 4;
            }
            4 => {
                // a window on the outline
                let t = rng.range(0.1, 0.9);
                let (x, y, horiz) = if rng.below(2) == 0 { (t * 18.0, if rng.below(2) == 0 { 0.0 } else { 12.0 }, true) } else { (if rng.below(2) == 0 { 0.0 } else { 18.0 }, t * 12.0, false) };
                for j in 0..3 {
                    let off = j as f64 * 0.1 - 0.1;
                    let (a, b) = if horiz { ([x - 0.6, y + off], [x + 0.6, y + off]) } else { ([x + off, y - 0.6], [x + off, y + 0.6]) };
                    s.insert_in(house, detail, &Shape::Line { a, b })?;
                }
                records += 3;
            }
            _ => {
                // a stair's steps
                let (x, y) = (rng.range(1.0, 14.0), rng.range(1.0, 9.0));
                for j in 0..11 {
                    s.insert_in(house, detail, &Shape::Line { a: [x + j as f64 * 0.26, y], b: [x + j as f64 * 0.26, y + 1.1] })?;
                }
                records += 11;
            }
        }
    }
    let l = s.add_layer(&Layer::new(0))?;
    let side = (render2d::math::sqrt(inserts as f64) + 0.999) as u32;
    let mut trees = Vec::new();
    for k in 0..inserts {
        let (c, r) = (k % side, k / side);
        let mut ins = Insert::new(house, [ORIGIN[0] + c as f64 * 25.0 + rng.range(0.0, 2.0), ORIGIN[1] + r as f64 * 20.0 + rng.range(0.0, 2.0)]);
        ins.rotation = rng.range(-0.1, 0.1);
        let id = s.insert(l, ist, &Shape::Insert(ins))?;
        trees.push((id, ins));
    }
    let (width, height) = (side as f64 * 25.0 + 20.0, side as f64 * 20.0 + 20.0);
    Ok(Cad { segments: records * inserts, entities: inserts, fills: 0, hatched: 0, fill_records: 0, curves: 0, curve_records: 0, arcs: 0, arc_records: 0, dashed: 0, dashed_records: 0, width, height,
             blocks: 1, inserts, insert_records: records * inserts, trees })
}

/// a named view of the drawing in a target of `size`
pub struct View {
    pub name: &'static str,
    pub cam: Camera,
}

/// the whole drawing, a district (a sixteenth of the area), the working zoom (10 cm per pixel at full HD: two blocks)
pub fn views(c: &Cad, size: (u32, u32)) -> [View; 3] {
    let centre = [ORIGIN[0] + 0.5 * c.width, ORIGIN[1] + 0.5 * c.height];
    let all = (c.width / size.0 as f64).max(c.height / size.1 as f64) * 1.02;
    let work = 0.1 * 1920.0 / size.0 as f64;
    [
        View { name: "overview", cam: Camera::new(centre, all) },
        View { name: "district", cam: Camera::new([centre[0] + 0.13 * c.width, centre[1] - 0.09 * c.height], 0.25 * all) },
        View { name: "working zoom", cam: Camera::new([centre[0] + 0.13 * c.width, centre[1] - 0.09 * c.height], work) },
    ]
}
