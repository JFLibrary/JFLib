// layout/adjust -- (JD21) small adjustments of connected shapes: Automatische Ausrichtung und
// Abstände and Automatischer Abstand (CN-14, RS-25), Diagramm drehen (RS-25), Von
// Seitenumbrüchen verschieben and Seitenumbrüche vermeiden (PR-03); the rules in layout.rs
//
//   align_space(doc, owner, only, align, space, spacing) -> moves   each child (downstream over
//                       a connector) lined up with its parent and set at the spacing from it,
//                       its siblings on that side at the spacing from each other, everything
//                       further downstream carried along
//   turn(doc, owner, only, Turn, spacing) -> moves   each connected part (a selection: one)
//                       turned or flipped about the middle of its box, the shapes upright;
//                       distances grow where shapes would come too close
//   off_breaks(doc, owner, only, &xs, &ys, margin) -> moves   shapes across a page break
//                       moved to the side where more of them lies, `margin` from the break
//
// The moves are applied with layout::move_nodes (members and callouts along) and the
// connectors follow with layout::reroute, all in the caller's transaction.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

use super::layered::acyclic;
use super::{components, fixed, links, outermost, plain_2d, top, Comp, Depth, EPS};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::ops;

// the connected shapes of a page: placeable ones (fixed ones too, they stay), their boxes,
// the connections Begin -> End
struct Net {
    shapes: Vec<ShapeRef>,
    boxes: Vec<[f64; 4]>,
    fixed: Vec<bool>,
    edges: Vec<(usize, usize)>,
}

fn net(doc: &Document, owner: Owner) -> Net {
    let (lk, glued) = links(doc, owner);
    let mut n = Net { shapes: Vec::new(), boxes: Vec::new(), fixed: Vec::new(), edges: Vec::new() };
    for s in doc.top_shapes(owner) {
        if !plain_2d(doc, s) || outermost(doc, s) != s {
            continue;
        }
        let obj = super::num(doc, SheetRef::Shape(s), "ObjType").unwrap_or(0.0) as i64;
        if obj & 4 != 0 || !(obj & 1 != 0 || (obj & 3 == 0 && glued.contains(&s))) {
            continue;
        }
        let Some(b) = ops::bounds(doc, &[s]) else { continue };
        n.shapes.push(s);
        n.boxes.push(b);
        n.fixed.push(fixed(doc, s));
    }
    let index: BTreeMap<ShapeRef, usize> = n.shapes.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (a, b) in lk {
        let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) else { continue };
        if i != j && seen.insert((i, j)) {
            n.edges.push((i, j));
        }
    }
    n
}

// the shapes `only` stands for (empty: none)
fn scope(doc: &Document, only: &[ShapeRef]) -> BTreeSet<ShapeRef> {
    only.iter().map(|&s| outermost(doc, top(doc, s))).collect()
}

fn centre(b: &[f64; 4]) -> [f64; 2] {
    [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5]
}

fn size(b: &[f64; 4]) -> [f64; 2] {
    [b[2] - b[0], b[3] - b[1]]
}

/// Automatische Ausrichtung und Abstände (`align` and `space`), Automatischer Abstand (`space`
/// alone), automatisch ausrichten (`align` alone) on the connected shapes of `owner` (`only`
/// not empty: the selected ones adjust, what lies downstream of them is carried along) ->
/// the moves of the shapes (see the file header)
pub fn align_space(doc: &Document, owner: Owner, only: &[ShapeRef], align: bool, space: bool, spacing: [f64; 2]) -> Vec<(ShapeRef, [f64; 2])> {
    let net = net(doc, owner);
    let n = net.shapes.len();
    if n < 2 || (!align && !space) {
        return Vec::new();
    }
    let sel = scope(doc, only);
    let in_scope: Vec<bool> = net.shapes.iter().map(|s| sel.is_empty() || sel.contains(s)).collect();
    // the flow without cycles, in the order of the page: top to bottom, left to right
    let ext: Vec<[f64; 2]> = net.boxes.iter().map(size).collect();
    let orig: Vec<[f64; 2]> = net.boxes.iter().map(|b| {
        let c = centre(b);
        [c[0], -c[1]]
    })
    .collect();
    let comp = Comp { ext: &ext, orig: &orig, edges: &net.edges, gap: [0.0; 2], step: [0.0; 2], depth: Depth::Medium };
    let dag = acyclic(&comp);
    let ko = super::layered::key_order(&comp);
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in &dag {
        kids[a].push(b);
        indeg[b] += 1;
    }
    let mut kr = vec![0usize; n];
    for (k, &i) in ko.iter().enumerate() {
        kr[i] = k;
    }
    for k in kids.iter_mut() {
        k.sort_by_key(|&b| kr[b]);
    }
    // a walk from the sources: each shape's parent is the first one reaching it
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut seen = vec![false; n];
    let mut walk: Vec<usize> = Vec::new();
    for r in ko.iter().copied().filter(|&i| indeg[i] == 0).chain(ko.iter().copied()) {
        if seen[r] {
            continue;
        }
        seen[r] = true;
        let mut q = VecDeque::from([r]);
        while let Some(v) = q.pop_front() {
            walk.push(v);
            for &w in &kids[v] {
                if !seen[w] {
                    seen[w] = true;
                    parent[w] = Some(v);
                    q.push_back(w);
                }
            }
        }
    }
    let mut delta = vec![[0.0f64; 2]; n];
    let at = |delta: &[[f64; 2]], i: usize| {
        let c = centre(&net.boxes[i]);
        [c[0] + delta[i][0], c[1] + delta[i][1]]
    };
    // each shape's children in the order of the walk
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &c in &walk {
        if let Some(p) = parent[c] {
            children[p].push(c);
        }
    }
    for &p in &walk {
        let ch = core::mem::take(&mut children[p]);
        // carried along with the parent
        for &c in &ch {
            if !net.fixed[c] {
                delta[c] = delta[p];
            }
        }
        let cp = at(&delta, p);
        let ep = ext[p];
        // the children by the side of the parent they lie on: (axis, sign)
        let mut sides: Vec<((usize, i32), Vec<usize>)> = Vec::new();
        for &c in &ch {
            let cc = at(&delta, c);
            let d = [cc[0] - cp[0], cc[1] - cp[1]];
            let m = if d[0].abs() >= d[1].abs() { 0 } else { 1 };
            let sg = if d[m] < 0.0 { -1 } else { 1 };
            match sides.iter_mut().find(|s| s.0 == (m, sg)) {
                Some(s) => s.1.push(c),
                None => sides.push(((m, sg), vec![c])),
            }
        }
        for ((m, sg), mut group) in sides {
            let x = 1 - m;
            group.sort_by(|&a, &b| at(&delta, a)[x].total_cmp(&at(&delta, b)[x]).then(a.cmp(&b)));
            // the child most in line with the parent
            let first = (0..group.len()).min_by(|&a, &b| (at(&delta, group[a])[x] - cp[x]).abs().total_cmp(&(at(&delta, group[b])[x] - cp[x]).abs()).then(a.cmp(&b))).unwrap_or(0);
            let mut new: Vec<[f64; 2]> = group.iter().map(|&c| at(&delta, c)).collect();
            for (k, &c) in group.iter().enumerate() {
                if net.fixed[c] || !in_scope[c] {
                    continue;
                }
                if space {
                    new[k][m] = cp[m] + sg as f64 * ((ep[m] + ext[c][m]) * 0.5 + spacing[m]);
                }
                let near = (new[k][x] - cp[x]).abs() <= (ep[x] + ext[c][x]) * 0.5 + EPS;
                if align && k == first && near {
                    new[k][x] = cp[x];
                }
            }
            if space && group.len() > 1 {
                // the siblings on this side at the spacing from each other, from the one in line
                for k in first + 1..group.len() {
                    let c = group[k];
                    if !net.fixed[c] && in_scope[c] {
                        new[k][x] = new[k - 1][x] + (ext[group[k - 1]][x] + ext[c][x]) * 0.5 + spacing[x];
                    }
                }
                for k in (0..first).rev() {
                    let c = group[k];
                    if !net.fixed[c] && in_scope[c] {
                        new[k][x] = new[k + 1][x] - (ext[group[k + 1]][x] + ext[c][x]) * 0.5 - spacing[x];
                    }
                }
            }
            for (k, &c) in group.iter().enumerate() {
                if net.fixed[c] {
                    continue;
                }
                let cc = at(&delta, c);
                delta[c][0] += new[k][0] - cc[0];
                delta[c][1] += new[k][1] - cc[1];
            }
        }
    }
    (0..n)
        .filter(|&i| !net.fixed[i] && (delta[i][0].abs() > EPS || delta[i][1].abs() > EPS))
        .map(|i| (net.shapes[i], delta[i]))
        .collect()
}

/// Diagramm drehen: VisLayoutDirection's four (0 right, 1 left, 2 vertical, 3 horizontal)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Turn {
    /// 90 degrees clockwise
    Right,
    /// 90 degrees anticlockwise
    Left,
    /// top and bottom exchanged
    FlipV,
    /// left and right exchanged
    FlipH,
}

impl Turn {
    fn apply(self, d: [f64; 2]) -> [f64; 2] {
        match self {
            Turn::Right => [d[1], -d[0]],
            Turn::Left => [-d[1], d[0]],
            Turn::FlipV => [d[0], -d[1]],
            Turn::FlipH => [-d[0], d[1]],
        }
    }
}

/// Diagramm drehen: every connected part of two or more shapes of `owner` (`only` not empty:
/// the selected shapes as one part) turned or flipped about the middle of its box; the shapes
/// keep their own orientation, fixed ones stay; after a turn the distances along an axis grow
/// where two shapes would come closer than `spacing` on both axes -> the moves
pub fn turn(doc: &Document, owner: Owner, only: &[ShapeRef], how: Turn, spacing: [f64; 2]) -> Vec<(ShapeRef, [f64; 2])> {
    let net = net(doc, owner);
    let n = net.shapes.len();
    let sel = scope(doc, only);
    let groups: Vec<Vec<usize>> = if sel.is_empty() {
        components(n, &net.edges).into_iter().filter(|g| g.len() > 1).collect()
    } else {
        vec![(0..n).filter(|&i| sel.contains(&net.shapes[i])).collect()]
    };
    let mut out = Vec::new();
    for g in groups {
        let g: Vec<usize> = g.into_iter().filter(|&i| !net.fixed[i]).collect();
        if g.is_empty() {
            continue;
        }
        let bx = g.iter().fold(super::EMPTY, |m, &i| super::grow(m, net.boxes[i]));
        let c = centre(&bx);
        let rel: Vec<[f64; 2]> = g.iter().map(|&i| {
            let p = centre(&net.boxes[i]);
            how.apply([p[0] - c[0], p[1] - c[1]])
        })
        .collect();
        // a turn: stretch an axis where two shapes would come too close
        let mut k = [1.0f64, 1.0];
        if matches!(how, Turn::Right | Turn::Left) {
            for a in 0..g.len() {
                for b in a + 1..g.len() {
                    let (sa, sb) = (size(&net.boxes[g[a]]), size(&net.boxes[g[b]]));
                    let need = [(sa[0] + sb[0]) * 0.5 + spacing[0], (sa[1] + sb[1]) * 0.5 + spacing[1]];
                    let (pa, pb) = (centre(&net.boxes[g[a]]), centre(&net.boxes[g[b]]));
                    // two that were that close already stay as they are
                    if (pa[0] - pb[0]).abs() < need[0] - EPS && (pa[1] - pb[1]).abs() < need[1] - EPS {
                        continue;
                    }
                    let d = [rel[b][0] - rel[a][0], rel[b][1] - rel[a][1]];
                    if d[0].abs() * k[0] >= need[0] - EPS || d[1].abs() * k[1] >= need[1] - EPS {
                        continue;
                    }
                    if d[0].abs() >= d[1].abs() && d[0].abs() > EPS {
                        k[0] = k[0].max(need[0] / d[0].abs());
                    } else if d[1].abs() > EPS {
                        k[1] = k[1].max(need[1] / d[1].abs());
                    }
                }
            }
        }
        for (j, &i) in g.iter().enumerate() {
            let p = centre(&net.boxes[i]);
            let to = [c[0] + rel[j][0] * k[0], c[1] + rel[j][1] * k[1]];
            let d = [to[0] - p[0], to[1] - p[1]];
            if d[0].abs() > EPS || d[1].abs() > EPS {
                out.push((net.shapes[i], d));
            }
        }
    }
    out
}

// one axis: the box [a, b] across a break of `lines` -> the move off it (None: none needed or
// it cannot get off: wider than the space between two breaks)
fn off_axis(a: f64, b: f64, lines: &[f64], margin: f64) -> Option<f64> {
    let crosses = |a: f64, b: f64| lines.iter().any(|&l| a + EPS < l && l < b - EPS);
    let &l = lines.iter().find(|&&l| a + EPS < l && l < b - EPS)?;
    // the narrowest sheet between two breaks must hold it
    let mut sorted = lines.to_vec();
    sorted.sort_by(|x, y| x.total_cmp(y));
    let sheet = sorted.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > EPS).fold(f64::INFINITY, f64::min);
    if b - a + 2.0 * margin > sheet + EPS {
        return None;
    }
    let to_low = l - margin - b;
    let to_high = l + margin - a;
    let (first, second) = if l - a >= b - l { (to_low, to_high) } else { (to_high, to_low) };
    [first, second].into_iter().find(|&d| !crosses(a + d, b + d))
}

/// Von Seitenumbrüchen verschieben: the shapes of `owner` (`only` not empty: those; members
/// move with their containers, fixed shapes stay) that lie across a page break (`xs`: the
/// vertical lines, `ys`: the horizontal ones, drawing units) moved to the side where more of
/// them lies, `margin` from the break -> the moves
pub fn off_breaks(doc: &Document, owner: Owner, only: &[ShapeRef], xs: &[f64], ys: &[f64], margin: f64) -> Vec<(ShapeRef, [f64; 2])> {
    let sel = scope(doc, only);
    let mut out = Vec::new();
    for s in doc.top_shapes(owner) {
        if !plain_2d(doc, s) || outermost(doc, s) != s || fixed(doc, s) || !(sel.is_empty() || sel.contains(&s)) {
            continue;
        }
        let Some(b) = ops::bounds(doc, &[s]) else { continue };
        let d = [off_axis(b[0], b[2], xs, margin).unwrap_or(0.0), off_axis(b[1], b[3], ys, margin).unwrap_or(0.0)];
        if d[0].abs() > EPS || d[1].abs() > EPS {
            out.push((s, d));
        }
    }
    out
}
