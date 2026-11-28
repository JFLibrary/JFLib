// tests of layout (cargo test -p jflib-diagram-core --lib layout)
//
//   1. Visio's placement codes both ways
//   2. arrange on built diagrams: every style and direction (levels follow Begin -> End, no
//      overlaps, the spacing kept), cycles, separate parts, crossings, hierarchy rows aligned,
//      compact tree, radial, circular, the grid, obstacles, the start corner, determinism
//   3. relayout on a page: connectors glued and routed, fixed and unconnected shapes stay, a
//      container's members move with it, undo restores everything, the page's cells
//   4. align_space, turn, off_breaks, enlarge_page
//
// Every document is built here; nothing is read from files.

use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::cell::Cell;
use crate::document::DocKind;
use crate::glue::Target;
use crate::names;
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::{Change, Page, PageHeader, ShapeData};

const S: f64 = 0.25;

fn opts(style: Style, dir: Dir) -> Opts {
    Opts { place: Place::new(style, dir), spacing: [S, S], depth: Depth::Medium, grid: None }
}

// a diagram of boxes w x h around the given middles, with connections
fn dia(boxes: &[([f64; 2], [f64; 2])], edges: &[(usize, usize)]) -> Diagram {
    let owner = Owner::Page(PageRef(0));
    Diagram {
        nodes: (0..boxes.len() as u32).map(|slot| ShapeRef { owner, slot }).collect(),
        boxes: boxes.iter().map(|(c, s)| [c[0] - s[0] * 0.5, c[1] - s[1] * 0.5, c[0] + s[0] * 0.5, c[1] + s[1] * 0.5]).collect(),
        edges: edges.to_vec(),
        obstacles: Vec::new(),
    }
}

fn placed(d: &Diagram, at: &[[f64; 2]]) -> Vec<[f64; 4]> {
    d.boxes
        .iter()
        .zip(at)
        .map(|(b, c)| {
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            [c[0] - w * 0.5, c[1] - h * 0.5, c[0] + w * 0.5, c[1] + h * 0.5]
        })
        .collect()
}

// no two boxes come closer than `gap` on both axes
fn assert_apart(bx: &[[f64; 4]], gap: f64) {
    for i in 0..bx.len() {
        for j in i + 1..bx.len() {
            let (a, b) = (bx[i], bx[j]);
            let dx = (b[0] - a[2]).max(a[0] - b[2]);
            let dy = (b[1] - a[3]).max(a[1] - b[3]);
            assert!(dx >= gap - 1e-6 || dy >= gap - 1e-6, "boxes {i} {a:?} and {j} {b:?} closer than {gap}");
        }
    }
}

// round styles keep their gaps as distances between the boxes, not along an axis
fn assert_clear(style: Style, bx: &[[f64; 4]], gap: f64) {
    if !matches!(style, Style::Radial | Style::Circular) {
        return assert_apart(bx, gap);
    }
    for i in 0..bx.len() {
        for j in i + 1..bx.len() {
            let (a, b) = (bx[i], bx[j]);
            let dx = (b[0] - a[2]).max(a[0] - b[2]).max(0.0);
            let dy = (b[1] - a[3]).max(a[1] - b[3]).max(0.0);
            assert!(crate::formula::math::sqrt(dx * dx + dy * dy) >= gap - 1e-6, "{style:?}: boxes {i} {a:?} and {j} {b:?} closer than {gap}");
        }
    }
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

// a messy flowchart: start, step, decision, two branches, a loop back, the end
fn flowchart() -> Diagram {
    dia(
        &[
            ([3.0, 1.0], [1.0, 0.5]),
            ([0.5, 4.0], [1.2, 0.6]),
            ([5.0, 3.0], [1.0, 1.0]),
            ([1.0, 0.5], [1.0, 0.5]),
            ([4.0, 5.5], [1.0, 0.5]),
            ([2.0, 2.2], [0.8, 0.5]),
        ],
        &[(0, 1), (1, 2), (2, 3), (2, 4), (3, 5), (4, 5), (4, 1)],
    )
}

// ---- 1. codes

#[test]
fn place_codes_both_ways() {
    for code in (1..=14).chain(16..=27) {
        let p = Place::from_code(code).expect("known code");
        assert_eq!(p.code(), code, "{p:?}");
    }
    assert_eq!(Place::from_code(0), None);
    assert_eq!(Place::from_code(15), None);
    assert_eq!(Place::from_code(1).unwrap(), Place::new(Style::Flowchart, Dir::Down));
    assert_eq!(Place::from_code(17).unwrap(), Place::new(Style::Hierarchy, Dir::Down));
    assert_eq!(Place::from_code(22).unwrap().align, Align::Start);
    assert_eq!(Place::from_code(7).unwrap(), Place::new(Style::CompactTree, Dir::Down));
    assert_eq!(Place::new(Style::Radial, Dir::Left).code(), 3);
    for c in [0, 1, 2, 3] {
        assert_eq!(Depth::from_code(Depth::from_code(c).code()), Depth::from_code(c));
    }
    assert_eq!(Depth::from_code(0), Depth::Medium);
}

// ---- 2. arrange

#[test]
fn flowchart_follows_the_connectors_in_every_direction() {
    for dir in [Dir::Down, Dir::Up, Dir::Right, Dir::Left] {
        let d = flowchart();
        let at = arrange(&d, &opts(Style::Flowchart, dir));
        let bx = placed(&d, &at);
        assert_apart(&bx, S);
        let v = dir.vector();
        // every connection but the loop back runs with the flow, a level or more apart
        for &(a, b) in &d.edges {
            if (a, b) == (4, 1) {
                continue;
            }
            let along = dot([at[b][0] - at[a][0], at[b][1] - at[a][1]], v);
            assert!(along > 0.5, "{dir:?}: {a} -> {b} runs {along}");
        }
        // the loop back runs against it
        assert!(dot([at[1][0] - at[4][0], at[1][1] - at[4][1]], v) < 0.0);
        // levels: the start alone at the top, the spacing between the levels
        let lv = |i: usize| dot(at[i], v);
        assert!((1..6).all(|i| lv(i) > lv(0)));
        let half = |i: usize| if dir.vertical() { (bx[i][3] - bx[i][1]) * 0.5 } else { (bx[i][2] - bx[i][0]) * 0.5 };
        assert!(close(lv(1) - half(1) - (lv(0) + half(0)), S), "{dir:?}: level gap {}", lv(1) - half(1) - lv(0) - half(0));
    }
}

#[test]
fn a_chain_runs_straight() {
    let d = dia(&[([0.0, 0.0], [1.0, 0.5]), ([3.0, 2.0], [1.0, 0.5]), ([1.0, 5.0], [1.0, 0.5]), ([4.0, 1.0], [1.0, 0.5])], &[(0, 1), (1, 2), (2, 3)]);
    let at = arrange(&d, &opts(Style::Flowchart, Dir::Down));
    for i in 1..4 {
        assert!(close(at[i][0], at[0][0]), "{at:?}");
        assert!(close(at[i - 1][1] - at[i][1], 0.5 + S), "{at:?}");
    }
    // the start corner of the shapes' box stays: top left
    let bx = placed(&d, &at);
    let left = bx.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
    let topy = bx.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max);
    assert!(close(left, -0.5) && close(topy, 5.25), "{left} {topy}");
}

#[test]
fn a_parent_sits_over_its_branches() {
    let d = dia(&[([2.0, 5.0], [1.0, 0.5]), ([0.0, 3.0], [1.0, 0.5]), ([4.0, 3.0], [1.0, 0.5]), ([2.0, 1.0], [1.0, 0.5])], &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    let at = arrange(&d, &opts(Style::Flowchart, Dir::Down));
    assert!(close(at[0][0], (at[1][0] + at[2][0]) * 0.5), "{at:?}");
    assert!(close(at[3][0], at[0][0]));
    assert!(close(at[2][0] - at[1][0], 1.0 + S));
    assert!(at[1][0] < at[2][0], "the order they had");
}

#[test]
fn crossings_are_taken_out() {
    // two on top, two below, connected across: the lower ones change places
    let d = dia(&[([0.0, 3.0], [1.0, 0.5]), ([2.0, 3.0], [1.0, 0.5]), ([0.0, 0.0], [1.0, 0.5]), ([2.0, 0.0], [1.0, 0.5])], &[(0, 3), (1, 2), (0, 2)]);
    for depth in [Depth::Shallow, Depth::Medium, Depth::Deep] {
        let mut o = opts(Style::Flowchart, Dir::Down);
        o.depth = depth;
        let at = arrange(&d, &o);
        // 0 -> 3 and 1 -> 2 do not cross: the order below follows the order above
        assert_eq!(at[0][0] < at[1][0], at[3][0] < at[2][0], "{depth:?}: {at:?}");
        assert_apart(&placed(&d, &at), S);
    }
}

#[test]
fn cycles_and_loops_lay_out() {
    let d = dia(&[([0.0, 0.0], [1.0, 0.5]), ([2.0, 0.0], [1.0, 0.5]), ([4.0, 0.0], [1.0, 0.5]), ([6.0, 0.0], [1.0, 0.5])], &[(0, 1), (1, 2), (2, 0), (2, 3), (3, 2)]);
    for style in [Style::Flowchart, Style::Hierarchy, Style::CompactTree, Style::Radial, Style::Circular] {
        let at = arrange(&d, &opts(style, Dir::Down));
        assert_eq!(at.len(), 4);
        assert!(at.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
        assert_clear(style, &placed(&d, &at), S);
    }
    let at = arrange(&d, &opts(Style::Flowchart, Dir::Down));
    // the first shape starts the walk: it stays on top
    assert!((1..4).all(|i| at[i][1] < at[0][1]), "{at:?}");
}

#[test]
fn separate_parts_follow_each_other_across_the_flow() {
    // two chains and a lone shape, the lone one leftmost
    let d = dia(
        &[([5.0, 5.0], [1.0, 0.5]), ([5.0, 3.0], [1.0, 0.5]), ([9.0, 5.0], [1.0, 0.5]), ([9.0, 3.0], [1.0, 0.5]), ([1.0, 4.0], [0.6, 0.6])],
        &[(0, 1), (2, 3)],
    );
    for style in [Style::Flowchart, Style::Hierarchy, Style::Radial, Style::Circular] {
        let at = arrange(&d, &opts(style, Dir::Down));
        let bx = placed(&d, &at);
        assert_clear(style, &bx, S);
        assert!(at[4][0] < at[0][0] && at[0][0] < at[2][0], "{style:?}: {at:?}");
        // the parts start at the top of the shapes' box
        let topy = bx.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max);
        let part_top = |a: usize, b: usize| bx[a][3].max(bx[b][3]);
        assert!(close(bx[4][3], topy) && close(part_top(0, 1), topy) && close(part_top(2, 3), topy), "{style:?}");
        // and touch at the spacing
        let left = bx[0][0].min(bx[1][0]);
        assert!(close(left - bx[4][2], S), "{style:?}: {}", left - bx[4][2]);
    }
    // a compact tree's parts follow each other along the flow
    let at = arrange(&d, &opts(Style::CompactTree, Dir::Down));
    assert_apart(&placed(&d, &at), S);
}

#[test]
fn hierarchy_rows_align() {
    // a root with three children, one of them with a child
    let d = dia(
        &[([2.0, 5.0], [2.0, 0.5]), ([0.0, 3.0], [1.0, 0.5]), ([2.0, 3.0], [1.0, 0.5]), ([4.0, 3.0], [1.0, 0.5]), ([0.0, 1.0], [1.5, 0.5])],
        &[(0, 1), (0, 2), (0, 3), (1, 4)],
    );
    for (code, want) in [(16, 0usize), (17, 1), (18, 2)] {
        let mut o = opts(Style::Hierarchy, Dir::Down);
        o.place = Place::from_code(code).unwrap();
        let at = arrange(&d, &o);
        let bx = placed(&d, &at);
        assert_apart(&bx, S);
        let rows: [&[usize]; 3] = [&[0], &[1, 2, 3], &[4]];
        let edge = |r: &[usize]| {
            let lo = r.iter().map(|&i| bx[i][0]).fold(f64::INFINITY, f64::min);
            let hi = r.iter().map(|&i| bx[i][2]).fold(f64::NEG_INFINITY, f64::max);
            [lo, (lo + hi) * 0.5, hi][want]
        };
        assert!(rows.iter().all(|r| close(edge(r), edge(rows[1]))), "{code}: {bx:?}");
        // the middle row at the spacing
        assert!(close(bx[2][0] - bx[1][2], S) && close(bx[3][0] - bx[2][2], S));
    }
    // left to right, aligned at the top
    let mut o = opts(Style::Hierarchy, Dir::Right);
    o.place = Place::from_code(22).unwrap();
    let at = arrange(&d, &o);
    let bx = placed(&d, &at);
    assert_apart(&bx, S);
    assert!(close(bx[0][3], bx[1][3]) && close(bx[4][3], bx[1][3]), "{bx:?}");
    assert!(at[1][0] > at[0][0] && at[4][0] > at[1][0]);
}

#[test]
fn compact_tree_is_an_outline() {
    // root, two children, the first with a child
    let d = dia(&[([0.0, 5.0], [1.0, 0.5]), ([3.0, 3.0], [1.0, 0.5]), ([0.0, 3.0], [1.0, 0.5]), ([5.0, 1.0], [1.0, 0.5])], &[(0, 1), (0, 2), (2, 3)]);
    let at = arrange(&d, &opts(Style::CompactTree, Dir::Down));
    let bx = placed(&d, &at);
    assert_apart(&bx, S);
    // rows top to bottom in the order of the walk: root, the left child and its child, the right child
    assert!(at[0][1] > at[2][1] && at[2][1] > at[3][1] && at[3][1] > at[1][1], "{at:?}");
    assert!(close(at[0][1] - at[2][1], 0.5 + S));
    // each level steps right by half its parent's width and half the spacing
    let indent = 0.5 + S * 0.5;
    assert!(close(bx[2][0] - bx[0][0], indent) && close(bx[1][0] - bx[0][0], indent) && close(bx[3][0] - bx[2][0], indent), "{bx:?}");
    // up and to the left: the mirror image
    let mut o = opts(Style::CompactTree, Dir::Up);
    o.place.then = Dir::Left;
    assert_eq!(o.place.code(), 11);
    let at = arrange(&d, &o);
    assert!(at[0][1] < at[2][1] && at[2][1] < at[3][1], "{at:?}");
    assert!(at[2][0] < at[0][0]);
    for code in 7..=14 {
        let mut o = opts(Style::CompactTree, Dir::Down);
        o.place = Place::from_code(code).unwrap();
        assert_apart(&placed(&d, &arrange(&d, &o)), S);
    }
}

#[test]
fn radial_puts_the_root_in_the_middle() {
    let mut boxes = vec![([5.0, 5.0], [1.0, 1.0])];
    let mut edges = Vec::new();
    for k in 0..6 {
        boxes.push(([k as f64, 0.0], [1.0, 0.5]));
        edges.push((0, k + 1));
    }
    boxes.push(([9.0, 9.0], [0.5, 0.5]));
    edges.push((1, 7));
    let d = dia(&boxes, &edges);
    let at = arrange(&d, &opts(Style::Radial, Dir::Down));
    assert_clear(Style::Radial, &placed(&d, &at), S);
    let r = |i: usize| crate::formula::math::sqrt((at[i][0] - at[0][0]).powi(2) + (at[i][1] - at[0][1]).powi(2));
    for k in 2..7 {
        assert!(close(r(k), r(1)), "the children on one circle: {} {}", r(k), r(1));
    }
    assert!(r(7) > r(1) + 0.5, "the grandchild further out");
    // the first child straight above the root
    assert!(close(at[1][0], at[0][0]) && at[1][1] > at[0][1], "{at:?}");
}

#[test]
fn circular_puts_them_on_a_ring() {
    let n = 7;
    let d = dia(&(0..n).map(|k| ([k as f64 * 2.0, (k % 3) as f64], [1.0, 0.6])).collect::<Vec<_>>(), &(0..n).map(|k| (k, (k + 1) % n)).collect::<Vec<_>>());
    let at = arrange(&d, &opts(Style::Circular, Dir::Down));
    assert_clear(Style::Circular, &placed(&d, &at), S);
    let c = [at.iter().map(|p| p[0]).sum::<f64>() / n as f64, at.iter().map(|p| p[1]).sum::<f64>() / n as f64];
    let r: Vec<f64> = at.iter().map(|p| crate::formula::math::sqrt((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2))).collect();
    assert!(r.iter().all(|&x| close(x, r[0])), "{r:?}");
    // the first of the walk straight above the middle
    let top = at.iter().copied().fold([0.0, f64::NEG_INFINITY], |m: [f64; 2], p| if p[1] > m[1] { p } else { m });
    assert!(close(top[0], c[0]), "{top:?} {c:?}");
}

#[test]
fn the_grid_takes_the_middles() {
    let d = flowchart();
    for style in [Style::Flowchart, Style::Hierarchy, Style::CompactTree, Style::Radial, Style::Circular] {
        let mut o = opts(style, Dir::Down);
        o.grid = Some([0.2, 0.1]);
        let at = arrange(&d, &o);
        assert_clear(style, &placed(&d, &at), S);
        for p in &at {
            let (kx, ky) = (p[0] / 0.2, p[1] / 0.1);
            assert!((kx - kx.round()).abs() < 1e-6 && (ky - ky.round()).abs() < 1e-6, "{style:?}: {p:?}");
        }
    }
}

#[test]
fn obstacles_are_kept_clear() {
    let mut d = dia(&[([0.0, 5.0], [1.0, 0.5]), ([0.0, 3.0], [1.0, 0.5]), ([0.0, 1.0], [1.0, 0.5])], &[(0, 1), (1, 2)]);
    // where the chain would go, and a frame around all (no obstacle)
    d.obstacles.push([-0.2, 3.5, 0.2, 4.0]);
    d.obstacles.push([-5.0, -5.0, 10.0, 10.0]);
    let at = arrange(&d, &opts(Style::Flowchart, Dir::Down));
    let mut bx = placed(&d, &at);
    bx.push(d.obstacles[0]);
    assert_apart(&bx, S);
    assert!(at[0][0] > 0.2, "pushed across past the obstacle: {at:?}");
}

#[test]
fn the_same_drawing_gives_the_same_layout() {
    let d = flowchart();
    for style in [Style::Flowchart, Style::Hierarchy, Style::CompactTree, Style::Radial, Style::Circular] {
        let o = opts(style, Dir::Down);
        assert_eq!(arrange(&d, &o), arrange(&d.clone(), &o));
    }
}

#[test]
fn a_large_drawing_stays_fast_and_clear() {
    // a grid-like graph of 300 shapes
    let n = 300;
    let boxes: Vec<([f64; 2], [f64; 2])> = (0..n).map(|k| ([(k % 17) as f64 * 1.3, (k / 17) as f64 * 0.9], [1.0, 0.5])).collect();
    let mut edges = Vec::new();
    for k in 0..n {
        if k + 1 < n && k % 17 != 16 {
            edges.push((k, k + 1));
        }
        if k + 17 < n && k % 5 == 0 {
            edges.push((k, k + 17));
        }
    }
    let d = dia(&boxes, &edges);
    for style in [Style::Flowchart, Style::Hierarchy] {
        let at = arrange(&d, &opts(style, Dir::Down));
        assert_apart(&placed(&d, &at), S);
    }
}

// ---- 3. on a page

struct Fx {
    doc: Document,
    page: PageRef,
    owner: Owner,
    eng: Engine,
    next: u32,
}

fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new(), next: 1 }
}

impl Fx {
    // an upright rectangle w x h around `at` with its outline
    fn rect(&mut self, at: [f64; 2], size: [f64; 2]) -> ShapeRef {
        let doc = &mut self.doc;
        let (w, h) = (size[0], size[1]);
        let mut d = ShapeData::new(ShapeKind::Shape);
        let num = |doc: &mut Document, n: &str, v: f64| Cell::constant(doc.intern(n), Value::Num(v), None);
        d.sheet.cells = vec![num(doc, "PinX", at[0]), num(doc, "PinY", at[1]), num(doc, "Width", w), num(doc, "Height", h), num(doc, "LocPinX", w * 0.5), num(doc, "LocPinY", h * 0.5)];
        let mut geo = Section::new(SecKey::one(names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let mut r = Row::new(RowKey::Index(i as u32 + 1));
            r.kind = Some(if i == 0 { names::MOVE_TO } else { names::LINE_TO });
            r.cells = vec![Cell::constant(names::X, Value::Num(p[0]), None), Cell::constant(names::Y, Value::Num(p[1]), None)];
            geo.rows.push(r);
        }
        d.sheet.sections.push(geo);
        // after the ids connectors took
        let id = self.doc.all_shapes(self.owner).filter_map(|s| self.doc.shape(s).map(|x| x.id())).max().unwrap_or(0).max(self.next) + 1;
        self.next = id;
        let slot = self.doc.page_mut(self.page).unwrap().shapes.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    fn edit<T>(&mut self, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> (T, Option<Change>) {
        let mut tx = self.doc.transaction("test");
        let r = f(&mut tx, &mut self.eng).expect("edit");
        (r, tx.commit())
    }

    // a dynamic connector from a to b
    fn connect(&mut self, a: ShapeRef, b: ShapeRef) -> ShapeRef {
        let (pa, pb) = (ops::frame(&self.doc, a).centre(), ops::frame(&self.doc, b).centre());
        let owner = self.owner;
        self.edit(|tx, eng| glue::new_connector(tx, eng, owner, (pa, Target::Shape(a)), (pb, Target::Shape(b)))).0
    }

    fn set(&mut self, s: ShapeRef, cell: &str, v: f64) {
        self.edit(|tx, _| {
            let a = tx.single(cell);
            tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v), None)
        });
    }

    fn mid(&self, s: ShapeRef) -> [f64; 2] {
        ops::frame(&self.doc, s).centre()
    }

    fn bounds(&self, s: ShapeRef) -> [f64; 4] {
        ops::bounds(&self.doc, &[s]).unwrap()
    }

    // every shape's sheet and the Connect entries
    fn snapshot(&self) -> (Vec<crate::Sheet>, Vec<crate::Connect>) {
        let sheets = self.doc.all_shapes(self.owner).map(|s| self.doc.shape(s).unwrap().sheet().clone()).collect();
        (sheets, self.doc.page(self.page).unwrap().connects.clone())
    }
}

// a little flowchart on a page: four steps, a decision, a loop back; a title box apart
fn page_flowchart(f: &mut Fx) -> Vec<ShapeRef> {
    let s: Vec<ShapeRef> = [[4.0, 1.0], [1.0, 3.0], [6.0, 4.0], [2.0, 6.0], [5.0, 7.0]].iter().map(|&p| f.rect(p, [1.0, 0.5])).collect();
    for (a, b) in [(0, 1), (1, 2), (2, 3), (2, 4), (4, 1)] {
        f.connect(s[a], s[b]);
    }
    s
}

#[test]
fn relayout_moves_the_connected_shapes_and_routes_their_connectors() {
    let mut f = fx();
    let s = page_flowchart(&mut f);
    let title = f.rect([9.0, 9.0], [2.0, 0.4]);
    let title_at = f.mid(title);
    let before = f.snapshot();
    // only the connected shapes are nodes; the title stays and is kept clear
    let d = diagram(&f.doc, f.owner, &[]);
    assert_eq!(d.nodes, s);
    assert_eq!(d.edges.len(), 5);
    assert_eq!(d.obstacles.len(), 1);
    let owner = f.owner;
    let o = opts(Style::Flowchart, Dir::Down);
    let (moved, change) = f.edit(|tx, eng| relayout(tx, eng, owner, &[], &o, Routing::Follow));
    assert!(!moved.is_empty());
    assert_eq!(f.mid(title), title_at);
    // the flow top to bottom along Begin -> End (the loop back excepted)
    let y: Vec<f64> = s.iter().map(|&x| f.mid(x)[1]).collect();
    assert!(y[0] > y[1] && y[1] > y[2] && y[2] > y[3] && y[2] > y[4], "{y:?}");
    let mut bx: Vec<[f64; 4]> = s.iter().map(|&x| f.bounds(x)).collect();
    bx.push(f.bounds(title));
    assert_apart(&bx, S);
    // the connectors still glued, their ends on the shapes they are glued to
    for c in f.doc.top_shapes(f.owner).collect::<Vec<_>>() {
        if !glue::is_connector(&f.doc, c) {
            continue;
        }
        let [a, b] = glue::glued(&f.doc, c);
        let (Target::Shape(a), Target::Shape(b)) = (a, b) else { panic!("glue kept") };
        let (p, q) = glue::ends(&f.doc, c).unwrap();
        let on = |pt: [f64; 2], s: ShapeRef| {
            let b = f.bounds(s);
            pt[0] >= b[0] - 1e-6 && pt[0] <= b[2] + 1e-6 && pt[1] >= b[1] - 1e-6 && pt[1] <= b[3] + 1e-6
        };
        assert!(on(p, a) && on(q, b), "connector ends {p:?} {q:?}");
    }
    // one undo step takes it all back
    let mut change = change.unwrap();
    assert!(change.undo(&mut f.doc));
    f.eng.refresh(&f.doc, &change.sheets());
    assert_eq!(f.snapshot(), before);
    assert!(change.redo(&mut f.doc));
}

#[test]
fn relayout_applies_a_routing_style() {
    let mut f = fx();
    page_flowchart(&mut f);
    let owner = f.owner;
    let o = opts(Style::Flowchart, Dir::Right);
    f.edit(|tx, eng| relayout(tx, eng, owner, &[], &o, Routing::Apply { route: 2, ext: 1 }));
    let conns: Vec<ShapeRef> = f.doc.top_shapes(f.owner).filter(|&c| glue::is_connector(&f.doc, c)).collect();
    assert_eq!(conns.len(), 5);
    for c in conns {
        assert_eq!(glue::style(&f.doc, c), glue::Style::Straight);
        assert_eq!(glue::path(&f.doc, c).len(), 2);
    }
}

#[test]
fn fixed_and_unplaceable_shapes_stay() {
    let mut f = fx();
    let s = page_flowchart(&mut f);
    f.set(s[2], "ShapeFixedCode", 1.0);
    f.set(s[4], "ObjType", 4.0);
    let lone = f.rect([1.0, 9.0], [1.0, 1.0]);
    let (fixed_at, never_at, lone_at) = (f.mid(s[2]), f.mid(s[4]), f.mid(lone));
    let owner = f.owner;
    let o = opts(Style::Flowchart, Dir::Down);
    f.edit(|tx, eng| relayout(tx, eng, owner, &[], &o, Routing::Follow));
    assert_eq!(f.mid(s[2]), fixed_at);
    assert_eq!(f.mid(s[4]), never_at);
    assert_eq!(f.mid(lone), lone_at);
    // the moved ones keep clear of the fixed one and of the lone one
    let bx: Vec<[f64; 4]> = [s[0], s[1], s[2], s[3], lone].iter().map(|&x| f.bounds(x)).collect();
    assert_apart(&bx, S);
    // a placeable shape (ObjType 1) without connectors is laid out on its own
    f.set(lone, "ObjType", 1.0);
    assert!(diagram(&f.doc, f.owner, &[]).nodes.contains(&lone));
}

#[test]
fn a_selection_lays_out_alone() {
    let mut f = fx();
    let s = page_flowchart(&mut f);
    let rest: Vec<[f64; 2]> = [s[0], s[4]].iter().map(|&x| f.mid(x)).collect();
    let owner = f.owner;
    let o = opts(Style::Flowchart, Dir::Right);
    let sel = [s[1], s[2], s[3]];
    f.edit(|tx, eng| relayout(tx, eng, owner, &sel, &o, Routing::Follow));
    assert_eq!(f.mid(s[0]), rest[0]);
    assert_eq!(f.mid(s[4]), rest[1]);
    assert!(f.mid(s[1])[0] < f.mid(s[2])[0] && f.mid(s[2])[0] < f.mid(s[3])[0]);
    assert_apart(&s.iter().map(|&x| f.bounds(x)).collect::<Vec<_>>(), S);
}

#[test]
fn a_container_moves_with_its_members() {
    let mut f = fx();
    let c = f.rect([3.0, 3.0], [3.0, 2.0]);
    let m = f.rect([3.0, 3.0], [1.0, 0.5]);
    let out = f.rect([8.0, 8.0], [1.0, 0.5]);
    f.edit(|tx, eng| {
        let k = tx.intern("Container");
        tx.set_user(SheetRef::Shape(c), "msvStructureType", Value::Text(k), Some(Unit::Str))?;
        structure::add_members(tx, eng, c, &[m], None)
    });
    assert_eq!(structure::members(&f.doc, c), vec![m]);
    // a connection from the outside shape to the member: the container is the node
    f.connect(out, m);
    let d = diagram(&f.doc, f.owner, &[]);
    assert_eq!(d.nodes, vec![c, out]);
    assert_eq!(d.edges, vec![(1, 0)]);
    let off = [f.mid(m)[0] - f.mid(c)[0], f.mid(m)[1] - f.mid(c)[1]];
    let owner = f.owner;
    let o = opts(Style::Flowchart, Dir::Down);
    f.edit(|tx, eng| relayout(tx, eng, owner, &[], &o, Routing::Follow));
    assert!(f.mid(out)[1] > f.mid(c)[1], "the outside shape first");
    let now = [f.mid(m)[0] - f.mid(c)[0], f.mid(m)[1] - f.mid(c)[1]];
    assert!(close(now[0], off[0]) && close(now[1], off[1]), "{off:?} -> {now:?}");
    assert_eq!(structure::members(&f.doc, c), vec![m]);
}

#[test]
fn store_writes_the_page_cells() {
    let mut f = fx();
    let page = f.page;
    let mut o = opts(Style::Hierarchy, Dir::Right);
    o.place.align = Align::End;
    o.depth = Depth::Deep;
    o.spacing = [0.5, 0.4];
    let (changed, _) = f.edit(|tx, eng| store(tx, eng, page, &o, Some((6, 2)), Some(true)));
    assert!(changed);
    let back = Opts::from_page(&f.doc, page);
    assert_eq!(back.place, o.place);
    assert_eq!(back.depth, Depth::Deep);
    assert!(close(back.spacing[0], 0.5) && close(back.spacing[1], 0.4));
    let n = |c: &str| num(&f.doc, SheetRef::Page(page), c).unwrap();
    assert_eq!((n("PlaceStyle"), n("RouteStyle"), n("LineRouteExt"), n("ResizePage")), (24.0, 6.0, 2.0, 1.0));
    // the same again changes nothing
    let (changed, c) = f.edit(|tx, eng| store(tx, eng, page, &o, Some((6, 2)), Some(true)));
    assert!(!changed && c.is_none());
}

#[test]
fn enlarge_page_holds_the_drawing() {
    let mut f = fx();
    let page = f.page;
    f.edit(|tx, _| {
        for (c, v) in [("PageWidth", 4.0), ("PageHeight", 3.0)] {
            let a = tx.single(c);
            tx.set_constant(SheetRef::Page(page), &a, Value::Num(v), None)?;
        }
        Ok(())
    });
    let a = f.rect([-1.0, 1.0], [1.0, 1.0]);
    let b = f.rect([5.0, 2.5], [1.0, 1.0]);
    let c = f.connect(a, b);
    let ends = glue::ends(&f.doc, c).unwrap();
    let (d, _) = f.edit(|tx, eng| enlarge_page(tx, eng, page, 0.25));
    assert_eq!(d, Some([1.75, 0.0]));
    let [w, h] = crate::pagesetup::size(&f.doc, page);
    assert!(close(w, 7.5) && close(h, 3.25), "{w} {h}");
    assert!(close(f.bounds(a)[0], 0.25) && close(f.bounds(b)[2], 7.25));
    // the connector glued to the whole shapes moved along (its _WALKGLUE ends have no formula
    // value of the engine's)
    let now = glue::ends(&f.doc, c).unwrap();
    assert!(close(now.0[0], ends.0[0] + 1.75) && close(now.1[0], ends.1[0] + 1.75) && close(now.0[1], ends.0[1]), "{ends:?} -> {now:?}");
    let path = glue::path(&f.doc, c);
    assert!(close(path[0][0], now.0[0]) && close(path[path.len() - 1][0], now.1[0]), "{path:?}");
    let (d, _) = f.edit(|tx, eng| enlarge_page(tx, eng, page, 0.25));
    assert_eq!(d, None);
}

// ---- 4. adjustments

#[test]
fn align_space_lines_children_up_with_their_parents() {
    let mut f = fx();
    let a = f.rect([2.0, 6.0], [1.0, 0.5]);
    let b = f.rect([2.3, 4.0], [1.0, 0.5]);
    let c = f.rect([2.6, 2.0], [1.0, 0.5]);
    let d = f.rect([5.0, 5.8], [1.0, 0.5]);
    f.connect(a, b);
    f.connect(b, c);
    f.connect(a, d);
    // the whole page: b in line under a at the spacing, c under b; d to the right of a
    let moves = align_space(&f.doc, f.owner, &[], true, true, [S, S]);
    let owner = f.owner;
    f.edit(|tx, eng| {
        let moved = move_nodes(tx, eng, owner, &moves)?;
        reroute(tx, eng, owner, &moved, Routing::Follow, false)
    });
    let (pa, pb, pc, pd) = (f.mid(a), f.mid(b), f.mid(c), f.mid(d));
    assert!(close(pb[0], pa[0]) && close(pa[1] - pb[1], 0.5 + S), "{pa:?} {pb:?}");
    assert!(close(pc[0], pb[0]) && close(pb[1] - pc[1], 0.5 + S), "{pc:?}");
    assert!(close(pd[1], pa[1]) && close(pd[0] - pa[0], 1.0 + S), "{pd:?}");
    assert!(align_space(&f.doc, f.owner, &[], true, true, [S, S]).is_empty(), "nothing left to do");
}

#[test]
fn align_space_of_a_selection_carries_what_is_downstream() {
    let mut f = fx();
    let a = f.rect([2.0, 6.0], [1.0, 0.5]);
    let b = f.rect([2.3, 3.0], [1.0, 0.5]);
    let c = f.rect([2.9, 1.0], [1.0, 0.5]);
    f.connect(a, b);
    f.connect(b, c);
    let moves = align_space(&f.doc, f.owner, &[b], true, true, [S, S]);
    let mb = moves.iter().find(|m| m.0 == b).unwrap().1;
    let mc = moves.iter().find(|m| m.0 == c).unwrap().1;
    assert!(close(mb[0], -0.3) && close(mb[1], 6.0 - 0.5 - S - 3.0), "{mb:?}");
    assert_eq!(mb, mc, "carried, not aligned itself");
    assert!(!moves.iter().any(|m| m.0 == a));
    // spacing alone leaves the line across as it is
    let moves = align_space(&f.doc, f.owner, &[b], false, true, [S, S]);
    let mb = moves.iter().find(|m| m.0 == b).unwrap().1;
    assert!(close(mb[0], 0.0) && close(mb[1], 6.0 - 0.5 - S - 3.0));
    // siblings on one side at the spacing from each other, the one in line with the parent kept
    let mut f = fx();
    let p = f.rect([4.0, 6.0], [1.0, 0.5]);
    let k: Vec<ShapeRef> = [[3.0, 3.5], [4.1, 4.0], [6.0, 3.6]].iter().map(|&x| f.rect(x, [1.0, 0.5])).collect();
    for &x in &k {
        f.connect(p, x);
    }
    let moves = align_space(&f.doc, f.owner, &[], true, true, [S, S]);
    let owner = f.owner;
    f.edit(|tx, eng| move_nodes(tx, eng, owner, &moves));
    let m: Vec<[f64; 2]> = k.iter().map(|&x| f.mid(x)).collect();
    assert!(close(m[1][0], 4.0), "in line: {m:?}");
    assert!(close(m[1][0] - m[0][0], 1.0 + S) && close(m[2][0] - m[1][0], 1.0 + S), "{m:?}");
    assert!(m.iter().all(|q| close(q[1], 6.0 - 0.5 - S)));
}

#[test]
fn turn_keeps_shapes_upright_and_apart() {
    let mut f = fx();
    // a row left to right, tall shapes close together
    let s: Vec<ShapeRef> = [[1.0, 5.0], [1.8, 5.0], [2.6, 5.0]].iter().map(|&p| f.rect(p, [0.5, 2.0])).collect();
    f.connect(s[0], s[1]);
    f.connect(s[1], s[2]);
    let lone = f.rect([9.0, 9.0], [1.0, 1.0]);
    let moves = turn(&f.doc, f.owner, &[], Turn::Right, [S, S]);
    assert!(!moves.iter().any(|m| m.0 == lone), "a shape on its own is no diagram");
    let owner = f.owner;
    f.edit(|tx, eng| move_nodes(tx, eng, owner, &moves));
    let m: Vec<[f64; 2]> = s.iter().map(|&x| f.mid(x)).collect();
    // clockwise: left to right becomes top to bottom, about the middle
    assert!(m.iter().all(|p| close(p[0], 1.8)), "{m:?}");
    assert!(m[0][1] > m[1][1] && m[1][1] > m[2][1]);
    assert!(close(m[1][1], 5.0));
    assert!(close(m[0][1] - m[1][1], 2.0 + S), "stretched to the spacing: {m:?}");
    assert!(s.iter().all(|&x| close(f.bounds(x)[2] - f.bounds(x)[0], 0.5)), "upright");
    // flipped left and right
    let mut f = fx();
    let s: Vec<ShapeRef> = [[1.0, 1.0], [3.0, 2.0]].iter().map(|&p| f.rect(p, [1.0, 0.5])).collect();
    f.connect(s[0], s[1]);
    let moves = turn(&f.doc, f.owner, &[], Turn::FlipH, [S, S]);
    let owner = f.owner;
    f.edit(|tx, eng| move_nodes(tx, eng, owner, &moves));
    assert_eq!((f.mid(s[0]), f.mid(s[1])), ([3.0, 1.0], [1.0, 2.0]));
}

#[test]
fn off_breaks_moves_shapes_to_their_larger_side() {
    let mut f = fx();
    let a = f.rect([4.8, 2.0], [1.0, 0.5]);
    let b = f.rect([5.4, 6.0], [1.0, 0.5]);
    let wide = f.rect([5.0, 9.0], [12.0, 0.5]);
    let low = f.rect([1.0, 3.0], [1.0, 1.0]);
    let moves = off_breaks(&f.doc, f.owner, &[], &[5.0, 10.0], &[3.2], 0.1);
    let get = |s: ShapeRef| moves.iter().find(|m| m.0 == s).map(|m| m.1);
    assert!(close(get(a).unwrap()[0], 5.0 - 0.1 - 5.3));
    assert!(close(get(b).unwrap()[0], 5.0 + 0.1 - 4.9));
    assert_eq!(get(wide), None, "wider than a sheet: stays");
    // across the horizontal break, more of it below
    assert!(close(get(low).unwrap()[1], 3.2 - 0.1 - 3.5));
    assert!(off_breaks(&f.doc, f.owner, &[a], &[5.0], &[], 0.1).iter().all(|m| m.0 == a));
}
