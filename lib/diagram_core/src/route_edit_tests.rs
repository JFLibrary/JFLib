// tests of a connector's way by hand (cargo test -p jflib-diagram-core --lib route_edit)
//
//   1. the handles of a way
//   2. a piece moved across, an end piece with a jog, a vertex moved, snapping that removes bends
//   3. vertices added and removed
//   4. the way kept when its shapes move, laid anew when a shape is moved onto it, reset
//   5. straight ways, curved and locked connectors

use alloc::vec::Vec;

use super::*;
use crate::route_styles::tests::{assert_path, fx, near, upright_or_level, Fx, STUB};

// A at (2, 2), B at (6, 4), the page's simple left to right: a Z bending at x = 4 (JD21v: Visio's
// flowchart bends one stub after A)
fn z() -> (Fx, ShapeRef, ShapeRef, ShapeRef) {
    let (f, a, b, c) = z_in(18.0);
    assert_path(&f.path(c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    (f, a, b, c)
}

// the same in another style
fn z_in(code: f64) -> (Fx, ShapeRef, ShapeRef, ShapeRef) {
    let mut f = fx();
    f.set(None, "RouteStyle", code);
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 4.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    (f, a, b, c)
}

fn apply(f: &mut Fx, c: ShapeRef, e: Edit, snap: f64) -> bool {
    let mut tx = f.doc.transaction("Weg");
    let changed = edit(&mut tx, &mut f.eng, c, e, snap).unwrap();
    tx.commit();
    changed
}

// ---- 1. handles

#[test]
fn a_z_has_three_midpoints_and_two_vertices() {
    let (f, _, _, c) = z();
    let w = way(&f.doc, c).unwrap();
    assert!(w.right);
    let h = handles(&f.doc, c);
    let mids: Vec<&Handle> = h.iter().filter(|x| !x.bend).collect();
    let bends: Vec<&Handle> = h.iter().filter(|x| x.bend).collect();
    assert_eq!((mids.len(), bends.len()), (3, 2));
    assert!(near(mids[0].at, [3.25, 2.0]) && near(mids[0].half, [0.75, 0.0]) && !mids[0].free, "{mids:?}");
    assert!(near(mids[1].at, [4.0, 3.0]) && near(mids[1].half, [0.0, 1.0]) && mids[1].index == 1);
    assert!(near(bends[0].at, [4.0, 2.0]) && bends[0].index == 1 && near(bends[1].at, [4.0, 4.0]) && bends[1].index == 2);
}

// ---- 2. moves

#[test]
fn a_piece_moves_across_and_stays_upright() {
    let (mut f, _, _, c) = z();
    assert_eq!(f.value(c, "ConFixedCode"), None);
    // the pointer's y does not matter: the upright piece moves across only
    assert!(apply(&mut f, c, Edit::Piece(1, [4.6, 3.3]), 0.0));
    assert_path(&f.path(c), &[[2.5, 2.0], [4.6, 2.0], [4.6, 4.0], [5.5, 4.0]]);
    // the way is the user's now
    assert_eq!(f.value(c, "ConFixedCode"), Some(1.0));
    // past the stub at A it cannot go
    assert!(apply(&mut f, c, Edit::Piece(1, [1.0, 3.0]), 0.0));
    assert_path(&f.path(c), &[[2.5, 2.0], [2.5 + STUB, 2.0], [2.5 + STUB, 4.0], [5.5, 4.0]]);
}

#[test]
fn an_end_piece_moves_with_a_jog() {
    let (mut f, _, _, c) = z();
    assert!(apply(&mut f, c, Edit::Piece(0, [3.0, 2.5]), 0.0));
    let p = f.path(c);
    assert_path(&p, &[[2.5, 2.0], [2.5 + STUB, 2.0], [2.5 + STUB, 2.5], [4.0, 2.5], [4.0, 4.0], [5.5, 4.0]]);
    assert!(upright_or_level(&p));
    // the only piece of a straight connection: a detour with two jogs
    let mut f = fx();
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 2.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    assert_path(&f.path(c), &[[2.5, 2.0], [5.5, 2.0]]);
    assert!(apply(&mut f, c, Edit::Piece(0, [4.0, 3.0]), 0.0));
    let u = [[2.5, 2.0], [2.5 + STUB, 2.0], [2.5 + STUB, 3.0], [5.5 - STUB, 3.0], [5.5 - STUB, 2.0], [5.5, 2.0]];
    assert_path(&f.path(c), &u);
    // dragged back near the line it snaps there and the bends go
    assert!(apply(&mut f, c, Edit::Piece(2, [4.0, 2.03]), 0.05));
    assert_path(&f.path(c), &[[2.5, 2.0], [5.5, 2.0]]);
    // without snapping it stays a small step
    assert!(apply(&mut f, c, Edit::Piece(0, [4.0, 3.0]), 0.0));
    assert!(apply(&mut f, c, Edit::Piece(2, [4.0, 2.03]), 0.0));
    assert_eq!(f.path(c).len(), 6);
}

#[test]
fn a_vertex_moves_both_pieces() {
    let (mut f, _, _, c) = z();
    assert!(apply(&mut f, c, Edit::Bend(1, [4.5, 2.8]), 0.0));
    let p = f.path(c);
    assert_path(&p, &[[2.5, 2.0], [2.5 + STUB, 2.0], [2.5 + STUB, 2.8], [4.5, 2.8], [4.5, 4.0], [5.5, 4.0]]);
    assert!(p.iter().any(|q| near(*q, [4.5, 2.8])));
}

// ---- 3. add and remove

#[test]
fn vertices_come_and_go() {
    let (mut f, _, _, c) = z();
    // Ctrl + click on the first piece: a vertex splitting it
    assert!(apply(&mut f, c, Edit::Add([3.0, 2.01]), 0.0));
    assert_path(&f.path(c), &[[2.5, 2.0], [3.0, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    assert_eq!(handles(&f.doc, c).iter().filter(|h| h.bend).count(), 3);
    // dragged: a step there
    let w = way(&f.doc, c).unwrap();
    let step = move_bend(&w.pts, 1, [3.0, 2.6], true, 0.125, 0.0);
    assert_path(&step, &[[2.5, 2.0], [3.0, 2.0], [3.0, 2.6], [4.0, 2.6], [4.0, 4.0], [5.5, 4.0]]);
    // Del: the split goes
    assert!(apply(&mut f, c, Edit::Remove(1), 0.0));
    assert_path(&f.path(c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    // a bend of the Z cannot go: both its neighbours end at A and B
    assert!(!apply(&mut f, c, Edit::Remove(1), 0.0));
    assert_eq!(remove_bend(&w.pts, 0, true, 0.125), None);
    // a detour's bend goes with its neighbour: the jog flattens, then the rest
    let u = alloc::vec![[2.5, 2.0], [2.625, 2.0], [2.625, 3.0], [5.375, 3.0], [5.375, 2.0], [5.5, 2.0]];
    assert_path(&remove_bend(&u, 2, true, 0.125).unwrap(), &[[2.5, 2.0], [5.5, 2.0]]);
    // a vertex on an end or on another vertex is not added
    assert_eq!(add_bend(&u, [2.5, 2.0]), None);
    assert_eq!(add_bend(&u, [2.625, 3.0]), None);
}

// ---- 4. kept ways

// (an organization chart left to right: it routes around shapes, simple ones do not)
#[test]
fn a_way_by_hand_is_kept_and_laid_anew_when_crossed() {
    let (mut f, _, b, c) = z_in(4.0);
    assert!(apply(&mut f, c, Edit::Piece(1, [4.6, 3.0]), 0.0));
    // B goes up: the upright piece stays at x = 4.6 (freshly routed it would bend at 4)
    f.move_to(b, [6.0, 5.0]);
    assert_path(&f.path(c), &[[2.5, 2.0], [4.6, 2.0], [4.6, 5.0], [5.5, 5.0]]);
    // a shape moved onto the kept way: laid anew round it
    let w = f.rect(3, [10.0, 10.0], [0.6, 0.6]);
    f.move_to(w, [4.6, 3.5]);
    let wb = glue::page_box(&f.doc, w).unwrap();
    let p = f.path(c);
    assert!(p.windows(2).all(|s| !route::hits_box(s[0], s[1], &wb)), "{p:?}");
    assert!(!p.iter().any(|q| (q[0] - 4.6).abs() < 1e-9), "{p:?}");
    // never reroute: the way stays where it is, through the shape
    let (mut f, _, b, c) = z_in(4.0);
    assert!(apply(&mut f, c, Edit::Piece(1, [4.6, 3.0]), 0.0));
    f.set(Some(c), "ConFixedCode", 2.0);
    f.move_to(b, [6.0, 5.0]);
    let w = f.rect(3, [10.0, 10.0], [0.6, 0.6]);
    f.move_to(w, [4.6, 3.5]);
    assert_path(&f.path(c), &[[2.5, 2.0], [4.6, 2.0], [4.6, 5.0], [5.5, 5.0]]);
    // Verbinder zurücksetzen: 2 stays the user's choice, the way is routed anew
    let mut tx = f.doc.transaction("Zurücksetzen");
    reset(&mut tx, &mut f.eng, &[c]).unwrap();
    tx.commit();
    assert_eq!(f.value(c, "ConFixedCode"), Some(2.0));
    assert!(!f.path(c).iter().any(|q| (q[0] - 4.6).abs() < 1e-9));
}

#[test]
fn reset_frees_a_way_by_hand() {
    let (mut f, _, b, c) = z();
    assert!(apply(&mut f, c, Edit::Piece(1, [4.6, 3.0]), 0.0));
    let mut tx = f.doc.transaction("Zurücksetzen");
    reset(&mut tx, &mut f.eng, &[c]).unwrap();
    tx.commit();
    assert_eq!(f.value(c, "ConFixedCode"), Some(0.0));
    assert_path(&f.path(c), &[[2.5, 2.0], [4.0, 2.0], [4.0, 4.0], [5.5, 4.0]]);
    // free again: B's move routes it anew, bending halfway
    f.move_to(b, [7.0, 4.0]);
    assert_path(&f.path(c), &[[2.5, 2.0], [4.5, 2.0], [4.5, 4.0], [6.5, 4.0]]);
}

// ---- 5. straight, curved, locked

#[test]
fn a_straight_way_gets_vertices() {
    let mut f = fx();
    f.set(None, "RouteStyle", 2.0);
    let a = f.rect(1, [2.0, 2.0], [1.0, 1.0]);
    let b = f.rect(2, [6.0, 4.0], [1.0, 1.0]);
    let c = f.connect(a, b);
    // (JD21v) Visio's straight style: corner to corner
    assert_path(&f.path(c), &[[2.5, 2.5], [5.5, 3.5]]);
    let h = handles(&f.doc, c);
    assert!(h.len() == 1 && h[0].free && !way(&f.doc, c).unwrap().right, "{h:?}");
    // the midpoint dragged: a vertex there
    assert!(apply(&mut f, c, Edit::Piece(0, [4.0, 5.0]), 0.0));
    assert_path(&f.path(c), &[[2.5, 2.5], [4.0, 5.0], [5.5, 3.5]]);
    assert_eq!(f.value(c, "ConFixedCode"), Some(1.0));
    // B moves: the vertex stays, both ends dock towards it
    f.move_to(b, [6.0, 2.0]);
    let third = 1.0 / 3.0;
    assert_path(&f.path(c), &[[2.0 + third, 2.5], [4.0, 5.0], [6.0 - third, 2.5]]);
    // the vertex moves freely and goes with Del
    assert!(apply(&mut f, c, Edit::Bend(1, [4.0, 6.0]), 0.0));
    assert!(near(f.path(c)[1], [4.0, 6.0]));
    assert!(apply(&mut f, c, Edit::Remove(1), 0.0));
    assert_eq!(f.path(c).len(), 2);
}

#[test]
fn curved_and_locked_ways_have_no_handles() {
    let (mut f, _, _, c) = z();
    f.set(Some(c), "LockVtxEdit", 1.0);
    assert!(way(&f.doc, c).is_none() && handles(&f.doc, c).is_empty());
    f.set(Some(c), "LockVtxEdit", 0.0);
    assert!(way(&f.doc, c).is_some());
    f.set(Some(c), "ConLineRouteExt", 2.0);
    f.update(&[c]);
    assert!(way(&f.doc, c).is_none() && handles(&f.doc, c).is_empty());
    let mut tx = f.doc.transaction("Weg");
    assert!(edit(&mut tx, &mut f.eng, c, Edit::Remove(1), 0.0).is_err());
}

#[test]
fn pure_edits_keep_the_ends() {
    let pts = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [4.0, 2.0]];
    for (i, to) in [(0, [1.0, 1.0]), (1, [3.0, 1.0]), (2, [3.0, 1.0]), (1, [-5.0, 0.0])] {
        let p = move_piece(&pts, i, to, true, 0.125, 0.0);
        assert!(near(p[0], pts[0]) && near(*p.last().unwrap(), pts[3]) && upright_or_level(&p), "{i}: {p:?}");
    }
    for j in [1, 2] {
        let p = move_bend(&pts, j, [1.0, 3.0], true, 0.125, 0.0);
        assert!(near(p[0], pts[0]) && near(*p.last().unwrap(), pts[3]) && upright_or_level(&p), "{j}: {p:?}");
    }
    // a polyline: a vertex in, a vertex moved
    let p = move_piece(&pts, 0, [1.0, 1.0], false, 0.125, 0.0);
    assert_path(&p, &[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [2.0, 2.0], [4.0, 2.0]]);
    assert!(orthogonal(&pts) && !orthogonal(&p));
}
