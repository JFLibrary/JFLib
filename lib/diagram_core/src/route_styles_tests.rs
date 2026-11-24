// tests of the routing styles and of shapes connectors may cross (cargo test -p
// jflib-diagram-core --lib route_styles); the fixture is shared with route_edit's tests
//
//   1. the codes both ways, the style a connector routes in
//   2. every style with a direction in every direction: the sides the ends use, the bends
//   3. simple styles ignore obstacles, straight ones draw one line
//   4. ShapePermeableX / Y: a way straight through a shape, only in its direction

use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::formula::Engine;
use crate::glue::{self, Target};
use crate::{CellAddr, DocKind, Document, Owner, Page, PageHeader, PageRef, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, ShapeRef, SheetRef, Unit, Value};

pub(crate) fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

pub(crate) fn near(p: [f64; 2], q: [f64; 2]) -> bool {
    close(p[0], q[0]) && close(p[1], q[1])
}

pub(crate) fn assert_path(got: &[[f64; 2]], want: &[[f64; 2]]) {
    assert!(got.len() == want.len() && got.iter().zip(want).all(|(a, b)| near(*a, *b)), "path {got:?}, not {want:?}");
}

pub(crate) fn upright_or_level(path: &[[f64; 2]]) -> bool {
    path.windows(2).all(|w| close(w[0][0], w[1][0]) || close(w[0][1], w[1][1]))
}

// the direction of a piece as a unit axis vector
pub(crate) fn heading(p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    if dx.abs() >= dy.abs() { [dx.signum(), 0.0] } else { [0.0, dy.signum()] }
}

pub(crate) struct Fx {
    pub doc: Document,
    pub page: PageRef,
    pub owner: Owner,
    pub eng: Engine,
}

pub(crate) fn fx() -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    Fx { doc, page, owner: Owner::Page(page), eng: Engine::new() }
}

impl Fx {
    // a w x h rectangle around `at` (geometry only: dynamic glue docks on its outline)
    pub fn rect(&mut self, id: u32, at: [f64; 2], size: [f64; 2]) -> ShapeRef {
        let doc = &mut self.doc;
        let mut d = ShapeData::new(ShapeKind::Shape);
        let (w, h) = (size[0], size[1]);
        for (n, v) in [("PinX", at[0]), ("PinY", at[1]), ("Width", w), ("Height", h), ("LocPinX", w * 0.5), ("LocPinY", h * 0.5), ("Angle", 0.0)] {
            let c = doc.intern(n);
            d.sheet.cells.push(crate::Cell::constant(c, Value::Num(v), None));
        }
        let mut geo = Section::new(SecKey::one(crate::names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let kind = if i == 0 { crate::names::MOVE_TO } else { crate::names::LINE_TO };
            let mut r = Row::new(RowKey::Index(i as u32 + 1));
            r.kind = Some(kind);
            r.cells = vec![crate::Cell::constant(crate::names::X, Value::Num(p[0]), None), crate::Cell::constant(crate::names::Y, Value::Num(p[1]), None)];
            geo.rows.push(r);
        }
        d.sheet.sections.push(geo);
        let tree = &mut self.doc.page_mut(self.page).unwrap().shapes;
        let slot = tree.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner, slot }
    }

    // a cell of a shape or (None) of the page, one transaction
    pub fn set(&mut self, s: Option<ShapeRef>, cell: &str, v: f64) {
        let sheet = s.map_or(SheetRef::Page(self.page), SheetRef::Shape);
        let mut tx = self.doc.transaction("Zelle");
        let a = tx.single(cell);
        tx.set_constant(sheet, &a, Value::Num(v), None).unwrap();
        tx.commit();
    }

    // a dynamic connector from shape a to shape b (glued to the whole shapes), routed
    pub fn connect(&mut self, a: ShapeRef, b: ShapeRef) -> ShapeRef {
        let (pa, pb) = (glue::centre(&self.doc, a), glue::centre(&self.doc, b));
        let mut tx = self.doc.transaction("Verbinden");
        let c = glue::new_connector(&mut tx, &mut self.eng, self.owner, (pa, Target::Shape(a)), (pb, Target::Shape(b))).unwrap();
        tx.commit().unwrap();
        c
    }

    // route these connectors again as a gesture would (glue::update)
    pub fn update(&mut self, conns: &[ShapeRef]) {
        let mut tx = self.doc.transaction("Neu");
        glue::update(&mut tx, &mut self.eng, conns).unwrap();
        tx.commit();
    }

    // move a shape's pin, then update what is glued to it and what its box now crosses
    pub fn move_to(&mut self, s: ShapeRef, at: [f64; 2]) {
        let mut tx = self.doc.transaction("Verschieben");
        let sheet = SheetRef::Shape(s);
        let (px, py) = (CellAddr::single(crate::names::PIN_X), CellAddr::single(crate::names::PIN_Y));
        tx.set_length(sheet, &px, at[0], Unit::Inches).unwrap();
        tx.set_length(sheet, &py, at[1], Unit::Inches).unwrap();
        self.eng.recalc_cells(&mut tx, &[(sheet, px), (sheet, py)]).unwrap();
        let mut deps = glue::dependents(tx.doc(), self.owner, &[s]);
        if let Some(b) = glue::page_box(tx.doc(), s) {
            for c in tx.doc().top_shapes(self.owner).collect::<Vec<_>>() {
                if !deps.contains(&c) && glue::is_connector(tx.doc(), c) && glue::path(tx.doc(), c).windows(2).any(|w| route::hits_box(w[0], w[1], &b)) {
                    deps.push(c);
                }
            }
        }
        glue::update(&mut tx, &mut self.eng, &deps).unwrap();
        tx.commit();
    }

    pub fn path(&self, c: ShapeRef) -> Vec<[f64; 2]> {
        glue::path(&self.doc, c)
    }

    pub fn value(&self, s: ShapeRef, cell: &str) -> Option<f64> {
        let a = CellAddr::single(self.doc.sym(cell)?);
        self.doc.resolve(SheetRef::Shape(s), &a).num()
    }
}

// the page's RouteStyle, the connector routed again
fn with_page_style(f: &mut Fx, code: f64, c: ShapeRef) -> Vec<[f64; 2]> {
    f.set(None, "RouteStyle", code);
    f.update(&[c]);
    f.path(c)
}

// ---- 1. codes

#[test]
fn codes_both_ways() {
    for code in 1..=22 {
        let s = RouteStyle::from_code(code).unwrap();
        assert_eq!(s.code(), code, "{s:?}");
        assert_eq!(s.kind.directed(), s.dir.is_some(), "{s:?}");
    }
    assert_eq!(RouteStyle::from_code(0), None);
    assert_eq!(RouteStyle::from_code(23), None);
    assert_eq!(RouteStyle::from_code(5), Some(RouteStyle { kind: Kind::Flowchart, dir: Some(Dir::Down) }));
    assert_eq!(RouteStyle::from_code(13), Some(RouteStyle { kind: Kind::Flowchart, dir: Some(Dir::Left) }));
    assert_eq!(RouteStyle::from_code(19), Some(RouteStyle { kind: Kind::Simple, dir: Some(Dir::Up) }));
    assert_eq!(RouteStyle::new(Kind::Tree, None).code(), 7);
    assert_eq!(RouteStyle::new(Kind::Tree, Some(Dir::Left)).code(), 15);
    assert_eq!(RouteStyle::new(Kind::Network, Some(Dir::Up)), RouteStyle { kind: Kind::Network, dir: None });
    assert!(Kind::CentreToCentre.straight() && Kind::Network.straight() && !Kind::Simple.straight());
}

#[test]
fn the_connector_overrides_the_page() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 6.0], [1.0, 1.0]);
    let b = f.rect(2, [4.0, 2.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert_eq!(of(&f.doc, c), RouteStyle::RIGHT_ANGLE);
    // right angles: an L from A's bottom to B's left side (cheaper than a Z)
    assert_path(&f.path(c), &[[2.0, 5.5], [2.0, 2.0], [3.5, 2.0]]);
    // the page's flowchart top to bottom: bottom to top, a Z bending halfway
    let p = with_page_style(&mut f, 5.0, c);
    assert_eq!(of(&f.doc, c), RouteStyle::from_code(5).unwrap());
    assert_path(&p, &[[2.0, 5.5], [2.0, 4.0], [4.0, 4.0], [4.0, 2.5]]);
    // the connector's own right angles win
    f.set(Some(c), "ShapeRouteStyle", 1.0);
    f.update(&[c]);
    assert_eq!(of(&f.doc, c), RouteStyle::RIGHT_ANGLE);
    assert_path(&f.path(c), &[[2.0, 5.5], [2.0, 2.0], [3.5, 2.0]]);
    assert!(!dynamics_off(&f.doc, f.owner));
    f.set(None, "DynamicsOff", 1.0);
    assert!(dynamics_off(&f.doc, f.owner));
}

// ---- 2. styles with a direction

// shape B a step along `d` from A and to its side, a connector A -> B in `code`: the first
// piece leaves A along d, the last one arrives travelling along d
#[test]
fn every_direction_names_the_sides() {
    let dirs: [(Dir, [f64; 2]); 4] = [(Dir::Down, [0.0, -1.0]), (Dir::Right, [1.0, 0.0]), (Dir::Up, [0.0, 1.0]), (Dir::Left, [-1.0, 0.0])];
    for kind in [Kind::OrgChart, Kind::Flowchart, Kind::Tree, Kind::Simple] {
        for (dir, d) in dirs {
            let mut f = fx();
            let a = f.rect(1, [10.0, 10.0], [1.0, 1.0]);
            // 4 along d, 2 across it
            let across = [d[1].abs() * 2.0, d[0].abs() * 2.0];
            let b = f.rect(2, [10.0 + d[0] * 4.0 + across[0], 10.0 + d[1] * 4.0 + across[1]], [1.0, 1.0]);
            let c = f.connect(a, b);
            let style = RouteStyle::new(kind, Some(dir));
            let p = with_page_style(&mut f, style.code() as f64, c);
            let n = p.len();
            assert!(upright_or_level(&p), "{style:?}: {p:?}");
            assert_eq!(heading(p[0], p[1]), d, "{style:?} leaves A: {p:?}");
            assert_eq!(heading(p[n - 2], p[n - 1]), d, "{style:?} arrives at B: {p:?}");
            assert_eq!(route::bends(&p), 2, "{style:?}: {p:?}");
        }
    }
}

#[test]
fn org_chart_halfway_tree_early() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 6.0], [1.0, 1.0]);
    let b = f.rect(2, [4.0, 2.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert_path(&with_page_style(&mut f, 3.0, c), &[[2.0, 5.5], [2.0, 4.0], [4.0, 4.0], [4.0, 2.5]]);
    // the tree bends one clearance (1/8 in) under A
    assert_path(&with_page_style(&mut f, 7.0, c), &[[2.0, 5.5], [2.0, 5.375], [4.0, 5.375], [4.0, 2.5]]);
    // left to right: one clearance right of A
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 4.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert_path(&with_page_style(&mut f, 8.0, c), &[[2.5, 2.0], [2.625, 2.0], [2.625, 4.0], [5.5, 4.0]]);
}

#[test]
fn flowchart_keeps_to_the_flow_only_along_it() {
    // side by side: the cheapest sides (straight across)
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 2.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert_path(&with_page_style(&mut f, 5.0, c), &[[2.5, 2.0], [5.5, 2.0]]);
    // the organization chart keeps to bottom and top: down, across, up over B, down into it
    let p = with_page_style(&mut f, 3.0, c);
    let n = p.len();
    assert_eq!(heading(p[0], p[1]), [0.0, -1.0], "{p:?}");
    assert_eq!(heading(p[n - 2], p[n - 1]), [0.0, -1.0], "{p:?}");
    assert!(near(p[0], [2.0, 1.5]) && near(p[n - 1], [6.0, 2.5]), "{p:?}");
}

// ---- 3. simple and straight

#[test]
fn simple_styles_ignore_obstacles() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 8.0], [1.0, 1.0]);
    let b = f.rect(2, [2.0, 2.0], [1.0, 1.0]);
    // a wall between them
    let _w = f.rect(3, [2.0, 5.0], [3.0, 0.5]);
    let c = f.connect(a, b);
    // the organization chart goes round the wall
    let p = with_page_style(&mut f, 3.0, c);
    assert!(route::bends(&p) >= 2, "{p:?}");
    // simple: straight down through it
    assert_path(&with_page_style(&mut f, 17.0, c), &[[2.0, 7.5], [2.0, 2.5]]);
    // simple horizontal-vertical / vertical-horizontal: an L, through the wall as well
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [5.0, 5.0], [1.0, 1.0]);
    let _w = f.rect(3, [3.5, 2.0], [0.5, 0.5]);
    let c = f.connect(a, b);
    assert_path(&with_page_style(&mut f, 21.0, c), &[[2.5, 2.0], [5.0, 2.0], [5.0, 4.5]]);
    assert_path(&with_page_style(&mut f, 22.0, c), &[[2.0, 2.5], [2.0, 5.0], [4.5, 5.0]]);
}

#[test]
fn straight_styles_draw_one_line() {
    for code in [2.0, 9.0, 16.0] {
        let mut f = fx();
        let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
        let b = f.rect(2, [6.0, 2.0], [1.0, 1.0]);
        let _w = f.rect(3, [4.0, 2.0], [0.5, 0.5]);
        let c = f.connect(a, b);
        assert_path(&with_page_style(&mut f, code, c), &[[2.5, 2.0], [5.5, 2.0]]);
        assert_eq!(glue::style(&f.doc, c), glue::Style::Straight, "{code}");
    }
}

// ---- 4. permeable shapes

#[test]
fn permeable_shapes_let_ways_through_one_way() {
    let mut f = fx();
    let a = f.rect(1, [1.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [7.0, 2.0], [1.0, 1.0]);
    let w = f.rect(3, [4.0, 2.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    // round the shape in the middle
    assert!(route::bends(&f.path(c)) >= 2, "{:?}", f.path(c));
    assert_eq!(pass(&f.doc, w), 0);
    // vertically permeable: still round it
    f.set(Some(w), "ShapePermeableY", 1.0);
    assert_eq!(pass(&f.doc, w), route::PASS_Y);
    f.update(&[c]);
    assert!(route::bends(&f.path(c)) >= 2, "{:?}", f.path(c));
    // horizontally permeable: straight through
    f.set(Some(w), "ShapePermeableY", 0.0);
    f.set(Some(w), "ShapePermeableX", 1.0);
    assert_eq!(pass(&f.doc, w), route::PASS_X);
    f.update(&[c]);
    assert_path(&f.path(c), &[[1.5, 2.0], [6.5, 2.0]]);
    // upright: the same the other way round
    let mut f = fx();
    let a = f.rect(1, [2.0, 1.0], [1.0, 1.0]);
    let b = f.rect(2, [2.0, 7.0], [1.0, 1.0]);
    let w = f.rect(3, [2.0, 4.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert!(route::bends(&f.path(c)) >= 2, "{:?}", f.path(c));
    f.set(Some(w), "ShapePermeableX", 1.0);
    f.update(&[c]);
    assert!(route::bends(&f.path(c)) >= 2, "{:?}", f.path(c));
    f.set(Some(w), "ShapePermeableY", 1.0);
    f.update(&[c]);
    assert_path(&f.path(c), &[[2.0, 1.5], [2.0, 6.5]]);
}

#[test]
fn a_layout_may_place_shapes_on_permeable_ones() {
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 2.0], [1.0, 1.0]);
    // not glued, not placeable: it stays and is kept clear
    let w = f.rect(3, [4.0, 6.0], [1.0, 1.0]);
    let _c = f.connect(a, b);
    assert_eq!(crate::layout::diagram(&f.doc, f.owner, &[]).obstacles.len(), 1);
    f.set(Some(w), "ShapePermeablePlace", 1.0);
    assert!(permeable_place(&f.doc, w));
    assert_eq!(crate::layout::diagram(&f.doc, f.owner, &[]).obstacles.len(), 0);
}

#[test]
fn a_permeable_shape_is_no_place_to_turn() {
    // B lies above the permeable shape's right part: the way may not turn up inside it
    let ports = [Port { at: [0.0, 0.0], dir: Some([1.0, 0.0]), own: None }];
    let goal = [Port { at: [3.0, 3.0], dir: Some([0.0, -1.0]), own: None }];
    let wall = [[1.0, -1.0, 5.0, 1.0]];
    let opts = Opts::default();
    let (_, _, p) = route::best_through(&ports, &goal, &wall, &[route::PASS_X], &opts);
    for w in p.windows(2) {
        assert!(!route::hits_pass(w[0], w[1], &wall[0], route::PASS_X), "{p:?}");
    }
    // without permeability the way goes round the whole box
    let (_, _, q) = route::best_through(&ports, &goal, &wall, &[], &opts);
    assert!(q.windows(2).all(|w| !route::hits_box(w[0], w[1], &wall[0])), "{q:?}");
    assert!(route::hits_pass([0.0, 0.0], [6.0, 0.0], &wall[0], route::PASS_Y));
    assert!(!route::hits_pass([0.0, 0.0], [6.0, 0.0], &wall[0], route::PASS_X));
    assert!(!route::hits_pass([3.0, -2.0], [3.0, 2.0], &wall[0], route::PASS_Y));
}
