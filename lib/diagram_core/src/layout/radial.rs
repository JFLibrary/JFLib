// layout/radial -- (JD21) the tree-like and round styles of Seitenlayout neu anordnen: compact
// tree, radial, circular (the rules in layout.rs's header)
//
//   compact(&comp) -> middles    an outline: the nodes in the order of a depth-first walk
//                                along the flow, each level stepping aside by half its
//                                parent's extent plus half the spacing
//   radial(&comp) -> middles     the root (most connections) in the middle, the others on
//                                circles in wedges by the leaves of their subtrees, the first
//                                at the top, clockwise
//   circular(&comp) -> middles   a ring in the order of a walk along the connections
//
// The middles are in the frame (u across, v along; radial and circular: u right, v down).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use super::layered::{acyclic, key_order};
use super::{ceil_to, Comp};
use crate::formula::math::{sincos, sqrt};

const TAU: f64 = core::f64::consts::PI * 2.0;
const QUARTER: f64 = core::f64::consts::FRAC_PI_2;

// the rank of each node in the key order
fn key_ranks(c: &Comp) -> (Vec<usize>, Vec<usize>) {
    let ko = key_order(c);
    let mut kr = vec![0usize; ko.len()];
    for (k, &i) in ko.iter().enumerate() {
        kr[i] = k;
    }
    (ko, kr)
}

/// a compact tree (see the file header): the spanning tree of a walk from the sources, a node
/// per row along the flow, children stepping aside
pub(super) fn compact(c: &Comp) -> Vec<[f64; 2]> {
    let n = c.ext.len();
    if n <= 1 {
        return vec![[0.0, 0.0]; n];
    }
    let (ko, kr) = key_ranks(c);
    let dag = acyclic(c);
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in &dag {
        kids[a].push(b);
        indeg[b] += 1;
    }
    for k in kids.iter_mut() {
        k.sort_by_key(|&b| kr[b]);
    }
    // a depth-first walk: the rows in its order, the indent from the parent
    let mut seen = vec![false; n];
    let mut ustart = vec![0.0; n];
    let mut out = vec![[0.0; 2]; n];
    let mut prev: Option<usize> = None;
    let roots: Vec<usize> = ko.iter().copied().filter(|&i| indeg[i] == 0).chain(ko.iter().copied()).collect();
    for r in roots {
        if seen[r] {
            continue;
        }
        seen[r] = true;
        ustart[r] = 0.0;
        let mut stack = vec![r];
        while let Some(v) = stack.pop() {
            let e = c.ext[v];
            let vc = match prev {
                None => e[1] * 0.5,
                Some(p) => out[p][1] + ceil_to((c.ext[p][1] + e[1]) * 0.5 + c.gap[1], c.step[1]),
            };
            out[v] = [ustart[v] + e[0] * 0.5, vc];
            prev = Some(v);
            let indent = ceil_to(e[0] * 0.5 + c.gap[0] * 0.5, c.step[0]);
            for &k in kids[v].iter().rev() {
                if !seen[k] {
                    seen[k] = true;
                    ustart[k] = ustart[v] + indent;
                    stack.push(k);
                }
            }
        }
    }
    out
}

// the neighbours of each node either way, each once, in the key order
fn neighbours(c: &Comp, kr: &[usize]) -> Vec<Vec<usize>> {
    let n = c.ext.len();
    let mut nb: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in c.edges {
        if a < n && b < n && a != b {
            if !nb[a].contains(&b) {
                nb[a].push(b);
            }
            if !nb[b].contains(&a) {
                nb[b].push(a);
            }
        }
    }
    for l in nb.iter_mut() {
        l.sort_by_key(|&x| kr[x]);
    }
    nb
}

// the diameter of the circle around a node's box
fn size(e: [f64; 2]) -> f64 {
    sqrt(e[0] * e[0] + e[1] * e[1])
}

// the radius at which two neighbours on a circle `delta` apart keep `need` between their middles
fn radius_for(need: f64, delta: f64) -> f64 {
    let (s, _) = sincos(delta * 0.5);
    if s > 1e-9 { need / (2.0 * s) } else { 0.0 }
}

// where a node goes on a circle: angle 0 at the top, clockwise; v points down
fn on_circle(r: f64, angle: f64) -> [f64; 2] {
    let (s, co) = sincos(QUARTER - angle);
    [r * co, -r * s]
}

/// radial (see the file header)
pub(super) fn radial(c: &Comp) -> Vec<[f64; 2]> {
    let n = c.ext.len();
    if n <= 1 {
        return vec![[0.0, 0.0]; n];
    }
    let (ko, kr) = key_ranks(c);
    let nb = neighbours(c, &kr);
    // the root: the node with the most connections; among those one without incoming ones,
    // then the one that reaches the most, then the first
    let mut out_e: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in c.edges {
        if a < n && b < n && a != b && !out_e[a].contains(&b) {
            out_e[a].push(b);
            indeg[b] += 1;
        }
    }
    let reach = |s: usize| {
        let mut seen = vec![false; n];
        seen[s] = true;
        let mut q = VecDeque::from([s]);
        let mut k = 0;
        while let Some(v) = q.pop_front() {
            k += 1;
            for &w in &out_e[v] {
                if !seen[w] {
                    seen[w] = true;
                    q.push_back(w);
                }
            }
        }
        k
    };
    let scored: Vec<(usize, bool, usize, usize)> = ko.iter().map(|&s| (nb[s].len(), indeg[s] == 0, reach(s), s)).collect();
    let root = scored.iter().max_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(kr[b.3].cmp(&kr[a.3]))).map_or(0, |t| t.3);
    // the tree of a walk from the root over every connection
    let mut depth = vec![usize::MAX; n];
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut walk = Vec::new();
    depth[root] = 0;
    let mut q = VecDeque::from([root]);
    while let Some(v) = q.pop_front() {
        walk.push(v);
        for &w in &nb[v] {
            if depth[w] == usize::MAX {
                depth[w] = depth[v] + 1;
                kids[v].push(w);
                q.push_back(w);
            }
        }
    }
    // the leaves under each node
    let mut leaves = vec![1.0f64; n];
    for &v in walk.iter().rev() {
        if !kids[v].is_empty() {
            leaves[v] = kids[v].iter().map(|&k| leaves[k]).sum();
        }
    }
    // the wedges: each node's children share its wedge by their leaves; the root's first child
    // in the middle of the top
    let first = kids[root].first().map_or(0.0, |&k| TAU * leaves[k] / leaves[root]);
    let mut wedge = vec![(-first * 0.5, TAU); n];
    let mut angle = vec![0.0f64; n];
    for &v in &walk {
        let (a0, span) = wedge[v];
        let mut a = a0;
        for &k in &kids[v] {
            let s = span * leaves[k] / leaves[v];
            wedge[k] = (a, s);
            angle[k] = a + s * 0.5;
            a += s;
        }
    }
    // the radius of each depth: neighbours on it apart (their circles), its boxes clear of the
    // boxes inside it -- at first by their extents along the radius, then out in small steps
    // while one still comes too close
    // (on a grid the middles move by up to half a step each)
    let gap = c.gap[0].max(c.gap[1]) + 2.0 * c.step[0].max(c.step[1]);
    let top = walk.iter().map(|&v| depth[v]).max().unwrap_or(0);
    let mut radius = vec![0.0f64; top + 1];
    let mut out = vec![[0.0; 2]; n];
    // half a box's extent along the direction of `a` (an angle from the top, clockwise)
    let along = |v: usize, a: f64| {
        let (s, co) = sincos(a);
        (c.ext[v][0] * s.abs() + c.ext[v][1] * co.abs()) * 0.5
    };
    let boxed = |p: [f64; 2], e: [f64; 2]| [p[0] - e[0] * 0.5, p[1] - e[1] * 0.5, p[0] + e[0] * 0.5, p[1] + e[1] * 0.5];
    let apart = |a: [f64; 4], b: [f64; 4]| {
        let dx = (b[0] - a[2]).max(a[0] - b[2]).max(0.0);
        let dy = (b[1] - a[3]).max(a[1] - b[3]).max(0.0);
        sqrt(dx * dx + dy * dy) >= gap - 1e-9
    };
    let mut placed: Vec<[f64; 4]> = vec![boxed(out[root], c.ext[root])];
    for d in 1..=top {
        let mut ring: Vec<usize> = walk.iter().copied().filter(|&v| depth[v] == d).collect();
        ring.sort_by(|&a, &b| angle[a].total_cmp(&angle[b]).then(a.cmp(&b)));
        let inner = walk.iter().filter(|&&v| depth[v] + 1 == d).flat_map(|&u| ring.iter().map(move |&v| (u, v))).map(|(u, v)| along(u, angle[v])).fold(0.0, f64::max);
        let outer = ring.iter().map(|&v| along(v, angle[v])).fold(0.0, f64::max);
        let mut r = radius[d - 1] + inner + outer + gap;
        if ring.len() > 1 {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                let delta = if i + 1 == ring.len() { TAU - (angle[a] - angle[b]) } else { angle[b] - angle[a] };
                let need = (size(c.ext[a]) + size(c.ext[b])) * 0.5 + gap;
                r = r.max(radius_for(need, delta));
            }
        }
        let step = (gap * 0.5).max(1e-3);
        for _ in 0..400 {
            if ring.iter().all(|&v| placed.iter().all(|&b| apart(boxed(on_circle(r, angle[v]), c.ext[v]), b))) {
                break;
            }
            r += step;
        }
        radius[d] = r;
        for &v in &ring {
            out[v] = on_circle(r, angle[v]);
            placed.push(boxed(out[v], c.ext[v]));
        }
    }
    out
}

/// circular (see the file header)
pub(super) fn circular(c: &Comp) -> Vec<[f64; 2]> {
    let n = c.ext.len();
    if n <= 1 {
        return vec![[0.0, 0.0]; n];
    }
    let (ko, kr) = key_ranks(c);
    let nb = neighbours(c, &kr);
    // a depth-first walk from the first node: neighbours end up side by side
    let mut seen = vec![false; n];
    let mut ring = Vec::with_capacity(n);
    for &s in &ko {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            ring.push(v);
            for &w in nb[v].iter().rev() {
                if !seen[w] {
                    seen[w] = true;
                    stack.push(w);
                }
            }
        }
    }
    // (on a grid the middles move by up to half a step each)
    let gap = c.gap[0].max(c.gap[1]) + 2.0 * c.step[0].max(c.step[1]);
    let delta = TAU / n as f64;
    let mut r = 0.0f64;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        r = r.max(radius_for((size(c.ext[a]) + size(c.ext[b])) * 0.5 + gap, delta));
    }
    let mut out = vec![[0.0; 2]; n];
    for (i, &v) in ring.iter().enumerate() {
        out[v] = on_circle(r, delta * i as f64);
    }
    out
}
