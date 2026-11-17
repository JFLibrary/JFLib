// bvh -- the fixed bounding volume hierarchy of a triangle mesh: binned SAH, built once
//
// Boxes in f32 (a mesh's vertices are f32, so the box of a triangle is exact in f32 and so is every
// union).  A node is 32 bytes; the two children of an inner node sit next to each other.  The build splits
// a node where the surface area heuristic over 16 bins per axis is cheapest (cost of a node visit 1, of a
// triangle 1); a node becomes a leaf when that is not cheaper than testing its triangles (at most
// `max_leaf`).  Past depth 40 it splits at the median along the widest axis, which bounds the depth for
// any input.  Every choice has a tie-break by axis, bin or index: the same tree on every target.

use crate::math::{Aabb, Vec3};
use alloc::vec;
use alloc::vec::Vec;

/// a node of a [`crate::TriMesh`]'s tree: an inner node when `count` is 0 (children at `first` and
/// `first + 1`), else a leaf with the triangles `first .. first + count`
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BvhNode {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub first: u32,
    pub count: u32,
}

impl BvhNode {
    #[inline]
    pub(crate) fn aabb(&self) -> Aabb {
        Aabb::new(Vec3::from(self.min), Vec3::from(self.max))
    }

    #[inline]
    pub fn is_leaf(&self) -> bool {
        self.count > 0
    }
}

/// the deepest path the traversals need room for (median splits past SAH_DEPTH keep any build below it)
pub(crate) const STACK: usize = 128;
const SAH_DEPTH: u32 = 40;
const BINS: usize = 16;

#[derive(Clone, Copy)]
struct Bin {
    min: [f32; 3],
    max: [f32; 3],
    n: u32,
}

const EMPTY_BIN: Bin = Bin { min: [f32::INFINITY; 3], max: [f32::NEG_INFINITY; 3], n: 0 };

#[inline]
fn grow(b: &mut Bin, lo: &[f32; 3], hi: &[f32; 3]) {
    for a in 0..3 {
        if lo[a] < b.min[a] {
            b.min[a] = lo[a];
        }
        if hi[a] > b.max[a] {
            b.max[a] = hi[a];
        }
    }
}

#[inline]
fn half_area(lo: &[f32; 3], hi: &[f32; 3]) -> f64 {
    if hi[0] < lo[0] {
        return 0.0;
    }
    let d = [(hi[0] - lo[0]) as f64, (hi[1] - lo[1]) as f64, (hi[2] - lo[2]) as f64];
    d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
}

/// build over the boxes (min, max) of the primitives -> the nodes and the order of the primitives
/// (leaf ranges index into it)
pub(crate) fn build(boxes: &[([f32; 3], [f32; 3])], max_leaf: usize) -> (Vec<BvhNode>, Vec<u32>) {
    let n = boxes.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    if n == 0 {
        return (Vec::new(), order);
    }
    let mut nodes: Vec<BvhNode> = Vec::with_capacity(2 * n / max_leaf.max(1) + 1);
    nodes.push(BvhNode { min: [0.0; 3], max: [0.0; 3], first: 0, count: 0 });
    // centroids times 2 (min + max, exact in f64)
    let cent: Vec<[f64; 3]> = boxes
        .iter()
        .map(|(lo, hi)| [lo[0] as f64 + hi[0] as f64, lo[1] as f64 + hi[1] as f64, lo[2] as f64 + hi[2] as f64])
        .collect();
    let mut work: Vec<(u32, usize, usize, u32)> = vec![(0, 0, n, 0)];
    let mut bin_of: Vec<u8> = vec![0; n];
    while let Some((ni, s, e, depth)) = work.pop() {
        let mut nb = EMPTY_BIN;
        let mut clo = [f64::INFINITY; 3];
        let mut chi = [f64::NEG_INFINITY; 3];
        for &i in &order[s..e] {
            let (lo, hi) = &boxes[i as usize];
            grow(&mut nb, lo, hi);
            let c = &cent[i as usize];
            for a in 0..3 {
                if c[a] < clo[a] {
                    clo[a] = c[a];
                }
                if c[a] > chi[a] {
                    chi[a] = c[a];
                }
            }
        }
        let count = e - s;
        let leaf = |nodes: &mut Vec<BvhNode>| {
            nodes[ni as usize] = BvhNode { min: nb.min, max: nb.max, first: s as u32, count: count as u32 };
        };
        if count == 1 {
            leaf(&mut nodes);
            continue;
        }
        let mut mid = 0usize;
        if depth < SAH_DEPTH {
            // binned SAH: the cheapest of 15 planes on each axis
            let node_area = half_area(&nb.min, &nb.max);
            let mut best: Option<(f64, usize, usize)> = None; // (cost, axis, first bin on the right)
            for a in 0..3 {
                let ext = chi[a] - clo[a];
                if !(ext > 0.0) {
                    continue;
                }
                let k = BINS as f64 / ext;
                let mut bins = [EMPTY_BIN; BINS];
                for &i in &order[s..e] {
                    let mut b = ((cent[i as usize][a] - clo[a]) * k) as usize;
                    if b >= BINS {
                        b = BINS - 1;
                    }
                    let (lo, hi) = &boxes[i as usize];
                    grow(&mut bins[b], lo, hi);
                    bins[b].n += 1;
                }
                // areas and counts left of each plane, then sweep from the right
                let mut left_area = [0.0f64; BINS];
                let mut left_n = [0u32; BINS];
                let mut acc = EMPTY_BIN;
                for p in 1..BINS {
                    let b = bins[p - 1];
                    if b.n > 0 {
                        grow(&mut acc, &b.min, &b.max);
                    }
                    acc.n += b.n;
                    left_area[p] = half_area(&acc.min, &acc.max);
                    left_n[p] = acc.n;
                }
                let mut acc = EMPTY_BIN;
                for p in (1..BINS).rev() {
                    let b = bins[p];
                    if b.n > 0 {
                        grow(&mut acc, &b.min, &b.max);
                    }
                    acc.n += b.n;
                    if left_n[p] == 0 || acc.n == 0 {
                        continue;
                    }
                    let cost = left_area[p] * left_n[p] as f64 + half_area(&acc.min, &acc.max) * acc.n as f64;
                    if best.is_none_or(|bb| cost < bb.0) {
                        best = Some((cost, a, p));
                    }
                }
            }
            match best {
                Some((cost, a, p)) => {
                    let split_cost = node_area + cost;
                    let leaf_cost = node_area * count as f64;
                    if count <= max_leaf && leaf_cost <= split_cost {
                        leaf(&mut nodes);
                        continue;
                    }
                    let k = BINS as f64 / (chi[a] - clo[a]);
                    for &i in &order[s..e] {
                        let mut b = ((cent[i as usize][a] - clo[a]) * k) as usize;
                        if b >= BINS {
                            b = BINS - 1;
                        }
                        bin_of[i as usize] = b as u8;
                    }
                    // partition in place: bins < p to the left
                    let (mut l, mut r) = (s, e);
                    while l < r {
                        if (bin_of[order[l] as usize] as usize) < p {
                            l += 1;
                        } else {
                            r -= 1;
                            order.swap(l, r);
                        }
                    }
                    mid = l;
                }
                None => {
                    // every centroid at one point
                    if count <= max_leaf {
                        leaf(&mut nodes);
                        continue;
                    }
                    mid = s + count / 2;
                }
            }
        }
        if mid <= s || mid >= e {
            if count <= max_leaf {
                leaf(&mut nodes);
                continue;
            }
            // the median along the widest axis of the centroids (keys made unique by the index)
            let mut a = 0;
            for k in 1..3 {
                if chi[k] - clo[k] > chi[a] - clo[a] {
                    a = k;
                }
            }
            order[s..e].sort_unstable_by(|&x, &y| {
                let (cx, cy) = (cent[x as usize][a], cent[y as usize][a]);
                cx.partial_cmp(&cy).unwrap_or(core::cmp::Ordering::Equal).then(x.cmp(&y))
            });
            mid = s + count / 2;
        }
        let first = nodes.len() as u32;
        nodes.push(BvhNode { min: [0.0; 3], max: [0.0; 3], first: 0, count: 0 });
        nodes.push(BvhNode { min: [0.0; 3], max: [0.0; 3], first: 0, count: 0 });
        nodes[ni as usize] = BvhNode { min: nb.min, max: nb.max, first, count: 0 };
        work.push((first + 1, mid, e, depth + 1));
        work.push((first, s, mid, depth + 1));
    }
    (nodes, order)
}

/// the depth of the tree (1 for a single leaf, 0 for none)
pub(crate) fn depth(nodes: &[BvhNode]) -> u32 {
    if nodes.is_empty() {
        return 0;
    }
    let mut best = 0;
    let mut stack: Vec<(u32, u32)> = vec![(0, 1)];
    while let Some((i, d)) = stack.pop() {
        let n = &nodes[i as usize];
        if d > best {
            best = d;
        }
        if n.count == 0 {
            stack.push((n.first, d + 1));
            stack.push((n.first + 1, d + 1));
        }
    }
    best
}
