// layout/layered -- (JD21) the layered styles of Seitenlayout neu anordnen: flowchart and
// hierarchy (the steps in layout.rs's header)
//
//   layout(&comp, hierarchy) -> middles   a connected part in the frame (u across, v along the
//                                         flow); hierarchy: Some(alignment of the rows)
//
// Every order is decided by positions and indices (stable sorts, f64::total_cmp), so the same
// part always gives the same layout.

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use super::{ceil_to, Align, Comp, Depth};

// a layered graph: the part's nodes first, then the dummy points of long connections
struct Layers {
    real: usize,
    /// extents (u, v); a dummy's are 0
    ext: Vec<[f64; 2]>,
    /// where each point lay across the flow before (dummies: between their ends)
    orig_u: Vec<f64>,
    rank: Vec<usize>,
    /// neighbours in the level above (towards the flow's start) and below
    up: Vec<Vec<usize>>,
    down: Vec<Vec<usize>>,
    /// the points of each level in their order
    levels: Vec<Vec<usize>>,
    /// each point's index in its level
    pos: Vec<usize>,
}

/// the middles of a part laid out in levels (see the file header)
pub(super) fn layout(c: &Comp, hierarchy: Option<Align>) -> Vec<[f64; 2]> {
    let n = c.ext.len();
    if n <= 1 {
        return vec![[0.0, 0.0]; n];
    }
    let dag = acyclic(c);
    let rank = levels(n, &dag, &key_order(c));
    let mut g = build(c, &dag, &rank);
    order(&mut g, c.depth);
    let v = along(&g, c);
    let u = match hierarchy {
        Some(a) => rows(&g, c, a),
        None => balanced(&g, c),
    };
    (0..n).map(|i| [u[i], v[g.rank[i]]]).collect()
}

// the nodes by where they lay: along the flow, then across, then index
pub(super) fn key_order(c: &Comp) -> Vec<usize> {
    let mut o: Vec<usize> = (0..c.ext.len()).collect();
    o.sort_by(|&a, &b| c.orig[a][1].total_cmp(&c.orig[b][1]).then(c.orig[a][0].total_cmp(&c.orig[b][0])).then(a.cmp(&b)));
    o
}

// the connections without cycles: a depth-first walk from the sources (in the order the nodes
// lay), a connection back to a node on the walk reversed; each pair once
pub(super) fn acyclic(c: &Comp) -> Vec<(usize, usize)> {
    let n = c.ext.len();
    let ko = key_order(c);
    let mut kr = vec![0usize; n];
    for (k, &i) in ko.iter().enumerate() {
        kr[i] = k;
    }
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in c.edges {
        if a < n && b < n && a != b && !out[a].contains(&b) {
            out[a].push(b);
            indeg[b] += 1;
        }
    }
    for o in out.iter_mut() {
        o.sort_by_key(|&b| kr[b]);
    }
    let mut state = vec![0u8; n];
    let mut dag: BTreeSet<(usize, usize)> = BTreeSet::new();
    let starts: Vec<usize> = ko.iter().copied().filter(|&i| indeg[i] == 0).chain(ko.iter().copied()).collect();
    for s in starts {
        if state[s] != 0 {
            continue;
        }
        state[s] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        while let Some(&(v, k)) = stack.last() {
            if k < out[v].len() {
                let last = stack.len() - 1;
                stack[last].1 += 1;
                let w = out[v][k];
                match state[w] {
                    0 => {
                        dag.insert((v, w));
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    // back to a node on the walk: reversed for the layout
                    1 => {
                        dag.insert((w, v));
                    }
                    _ => {
                        dag.insert((v, w));
                    }
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    dag.into_iter().collect()
}

// the level of each node: the longest path from a source; a source then moves down to just
// above its nearest successor
fn levels(n: usize, dag: &[(usize, usize)], ko: &[usize]) -> Vec<usize> {
    let mut kr = vec![0usize; n];
    for (k, &i) in ko.iter().enumerate() {
        kr[i] = k;
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in dag {
        succ[a].push(b);
        indeg[b] += 1;
    }
    let sources: Vec<bool> = indeg.iter().map(|&d| d == 0).collect();
    let mut ready: BTreeSet<(usize, usize)> = (0..n).filter(|&i| indeg[i] == 0).map(|i| (kr[i], i)).collect();
    let mut rank = vec![0usize; n];
    while let Some((k, v)) = ready.iter().next().copied() {
        ready.remove(&(k, v));
        for &s in &succ[v] {
            rank[s] = rank[s].max(rank[v] + 1);
            indeg[s] -= 1;
            if indeg[s] == 0 {
                ready.insert((kr[s], s));
            }
        }
    }
    for v in 0..n {
        if sources[v] {
            if let Some(m) = succ[v].iter().map(|&s| rank[s]).min() {
                rank[v] = m.saturating_sub(1);
            }
        }
    }
    let lo = rank.iter().copied().min().unwrap_or(0);
    rank.iter().map(|r| r - lo).collect()
}

// the layered graph: dummy points on connections over more than one level, the levels sorted
// by where their points lay across the flow
fn build(c: &Comp, dag: &[(usize, usize)], rank: &[usize]) -> Layers {
    let n = c.ext.len();
    let mut g = Layers {
        real: n,
        ext: c.ext.to_vec(),
        orig_u: c.orig.iter().map(|p| p[0]).collect(),
        rank: rank.to_vec(),
        up: vec![Vec::new(); n],
        down: vec![Vec::new(); n],
        levels: Vec::new(),
        pos: Vec::new(),
    };
    for &(a, b) in dag {
        let (ra, rb) = (g.rank[a], g.rank[b]);
        let mut prev = a;
        for r in ra + 1..rb {
            let t = (r - ra) as f64 / (rb - ra) as f64;
            let d = g.ext.len();
            g.ext.push([0.0, 0.0]);
            g.orig_u.push(g.orig_u[a] + (g.orig_u[b] - g.orig_u[a]) * t);
            g.rank.push(r);
            g.up.push(Vec::new());
            g.down.push(Vec::new());
            g.down[prev].push(d);
            g.up[d].push(prev);
            prev = d;
        }
        g.down[prev].push(b);
        g.up[b].push(prev);
    }
    let top = g.rank.iter().copied().max().unwrap_or(0);
    g.levels = vec![Vec::new(); top + 1];
    for i in 0..g.ext.len() {
        g.levels[g.rank[i]].push(i);
    }
    let ou = g.orig_u.clone();
    for l in g.levels.iter_mut() {
        l.sort_by(|&a, &b| ou[a].total_cmp(&ou[b]).then(a.cmp(&b)));
    }
    g.pos = vec![0; g.ext.len()];
    renumber(&mut g);
    g
}

fn renumber(g: &mut Layers) {
    for l in &g.levels {
        for (i, &x) in l.iter().enumerate() {
            g.pos[x] = i;
        }
    }
}

// the crossings between two neighbouring levels: the connections sorted by their upper end,
// the inversions of their lower ends counted with a Fenwick tree
fn crossings_between(g: &Layers, k: usize) -> usize {
    let mut e: Vec<(usize, usize)> = Vec::new();
    for &a in &g.levels[k] {
        for &b in &g.down[a] {
            e.push((g.pos[a], g.pos[b]));
        }
    }
    e.sort_unstable();
    let m = g.levels.get(k + 1).map_or(0, |l| l.len());
    let mut tree = vec![0usize; m + 1];
    let mut count = 0;
    for (seen, &(_, b)) in e.iter().enumerate() {
        // the ones seen so far ending at or before b
        let mut i = b + 1;
        let mut le = 0;
        while i > 0 {
            le += tree[i];
            i -= i & i.wrapping_neg();
        }
        count += seen - le;
        let mut i = b + 1;
        while i <= m {
            tree[i] += 1;
            i += i & i.wrapping_neg();
        }
    }
    count
}

fn crossings(g: &Layers) -> usize {
    (0..g.levels.len().saturating_sub(1)).map(|k| crossings_between(g, k)).sum()
}

// one level sorted by the barycentres of its neighbours on one side (a point without any keeps
// its index)
fn reorder(g: &mut Layers, k: usize, upward: bool) {
    let mut keyed: Vec<(f64, usize, usize)> = g.levels[k]
        .iter()
        .map(|&x| {
            let nb = if upward { &g.up[x] } else { &g.down[x] };
            let b = if nb.is_empty() { g.pos[x] as f64 } else { nb.iter().map(|&y| g.pos[y] as f64).sum::<f64>() / nb.len() as f64 };
            (b, g.pos[x], x)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    g.levels[k] = keyed.into_iter().map(|t| t.2).collect();
    for (i, &x) in g.levels[k].iter().enumerate() {
        g.pos[x] = i;
    }
}

// the crossings of u's connections with v's when u lies left of v (both in one level)
fn pair_crossings(g: &Layers, u: usize, v: usize) -> usize {
    let mut c = 0;
    for nb in [&g.up, &g.down] {
        for &a in &nb[u] {
            for &b in &nb[v] {
                if g.pos[a] > g.pos[b] {
                    c += 1;
                }
            }
        }
    }
    c
}

// neighbours exchanged where that crosses less
fn transpose(g: &mut Layers) {
    for _ in 0..4 {
        let mut better = false;
        for k in 0..g.levels.len() {
            for i in 0..g.levels[k].len().saturating_sub(1) {
                let (u, v) = (g.levels[k][i], g.levels[k][i + 1]);
                if pair_crossings(g, v, u) < pair_crossings(g, u, v) {
                    g.levels[k].swap(i, i + 1);
                    g.pos[u] = i + 1;
                    g.pos[v] = i;
                    better = true;
                }
            }
        }
        if !better {
            break;
        }
    }
}

// the order in the levels: sweeps down and up (and exchanges when deep), the order with the
// fewest crossings kept
fn order(g: &mut Layers, depth: Depth) {
    let (sweeps, exchange) = match depth {
        Depth::Shallow => (2, false),
        Depth::Medium => (8, false),
        Depth::Deep => (24, true),
    };
    let mut best = g.levels.clone();
    let mut fewest = crossings(g);
    let n = g.levels.len();
    for s in 0..sweeps {
        if fewest == 0 {
            break;
        }
        if s % 2 == 0 {
            for k in 1..n {
                reorder(g, k, true);
            }
        } else {
            for k in (0..n.saturating_sub(1)).rev() {
                reorder(g, k, false);
            }
        }
        if exchange {
            transpose(g);
        }
        let c = crossings(g);
        if c < fewest {
            fewest = c;
            best = g.levels.clone();
        }
    }
    g.levels = best;
    renumber(g);
}

// the levels along the flow: each as thick as its thickest node, the spacing between them
fn along(g: &Layers, c: &Comp) -> Vec<f64> {
    let thick: Vec<f64> = g.levels.iter().map(|l| l.iter().filter(|&&x| x < g.real).map(|&x| g.ext[x][1]).fold(0.0, f64::max)).collect();
    let mut v = vec![0.0; thick.len()];
    for k in 1..thick.len() {
        v[k] = v[k - 1] + ceil_to(thick[k - 1] * 0.5 + c.gap[1] + thick[k] * 0.5, c.step[1]);
    }
    v
}

// the distance two neighbours of a level keep between their middles (dummies keep less)
fn separation(g: &Layers, c: &Comp, a: usize, b: usize) -> f64 {
    let dummies = (a >= g.real) as u32 + (b >= g.real) as u32;
    let gap = c.gap[0] / [1.0, 2.0, 4.0][dummies as usize];
    ceil_to((g.ext[a][0] + g.ext[b][0]) * 0.5 + gap, c.step[0])
}

// hierarchy: each level's nodes packed in their order, the rows aligned with the widest one
fn rows(g: &Layers, c: &Comp, align: Align) -> Vec<f64> {
    let mut u = vec![0.0; g.ext.len()];
    let mut widths: Vec<(Vec<usize>, f64)> = Vec::new();
    for l in &g.levels {
        let reals: Vec<usize> = l.iter().copied().filter(|&x| x < g.real).collect();
        let mut at = 0.0;
        for (i, &x) in reals.iter().enumerate() {
            if i == 0 {
                at = g.ext[x][0] * 0.5;
            } else {
                at += separation(g, c, reals[i - 1], x);
            }
            u[x] = at;
        }
        let w = reals.last().map_or(0.0, |&x| at + g.ext[x][0] * 0.5);
        widths.push((reals, w));
    }
    let widest = widths.iter().map(|w| w.1).fold(0.0, f64::max);
    for (reals, w) in &widths {
        let off = match align {
            Align::Start => 0.0,
            Align::Centre => (widest - w) * 0.5,
            Align::End => widest - w,
        };
        for &x in reals {
            u[x] += off;
        }
    }
    u
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let m = v.len() / 2;
    Some(if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) * 0.5 })
}

// the points of a level as near the wanted places as their order and separations allow, the
// squared distances weighted (pool adjacent violators on x - offset)
fn fit(xs: &mut [f64], want: &[f64], w: &[f64], sep: &[f64]) {
    let m = xs.len();
    if m == 0 {
        return;
    }
    let mut off = vec![0.0; m];
    for i in 1..m {
        off[i] = off[i - 1] + sep[i - 1];
    }
    // blocks: (first index, sum of weights, weighted sum of the targets)
    let mut blocks: Vec<(usize, f64, f64)> = Vec::new();
    for i in 0..m {
        blocks.push((i, w[i], w[i] * (want[i] - off[i])));
        while blocks.len() >= 2 {
            let b = blocks[blocks.len() - 1];
            let a = blocks[blocks.len() - 2];
            if a.2 / a.1 > b.2 / b.1 {
                blocks.pop();
                let last = blocks.len() - 1;
                blocks[last].1 += b.1;
                blocks[last].2 += b.2;
            } else {
                break;
            }
        }
    }
    for (k, b) in blocks.iter().enumerate() {
        let end = blocks.get(k + 1).map_or(m, |n| n.0);
        let y = b.2 / b.1;
        for i in b.0..end {
            xs[i] = y + off[i];
        }
    }
}

// flowchart: each point as near the median of its neighbours (above, then below, by turns) as
// the level's order and the spacing allow; dummies weigh less (they only keep a lane free for a
// long connection, the router lays its way): the shapes of a chain stay in line
fn balanced(g: &Layers, c: &Comp) -> Vec<f64> {
    let mut u = vec![0.0; g.ext.len()];
    let seps: Vec<Vec<f64>> = g.levels.iter().map(|l| l.windows(2).map(|p| separation(g, c, p[0], p[1])).collect()).collect();
    for (l, s) in g.levels.iter().zip(&seps) {
        let total: f64 = s.iter().sum();
        let mut at = -total * 0.5;
        for (i, &x) in l.iter().enumerate() {
            if i > 0 {
                at += s[i - 1];
            }
            u[x] = at;
        }
    }
    let n = g.levels.len();
    let rounds = match c.depth {
        Depth::Shallow => 2,
        Depth::Medium => 6,
        Depth::Deep => 12,
    };
    let pass = |u: &mut Vec<f64>, k: usize, sides: &[&Vec<Vec<usize>>]| {
        let l = &g.levels[k];
        let want: Vec<f64> = l
            .iter()
            .map(|&x| {
                let nb: Vec<f64> = sides.iter().flat_map(|s| s[x].iter().map(|&y| u[y])).collect();
                median(nb).unwrap_or(u[x])
            })
            .collect();
        let w: Vec<f64> = l.iter().map(|&x| if x < g.real { 1.0 } else { 0.25 }).collect();
        let mut xs: Vec<f64> = l.iter().map(|&x| u[x]).collect();
        fit(&mut xs, &want, &w, &seps[k]);
        for (&x, v) in l.iter().zip(xs) {
            u[x] = v;
        }
    };
    for _ in 0..rounds {
        for k in 1..n {
            pass(&mut u, k, &[&g.up]);
        }
        for k in (0..n.saturating_sub(1)).rev() {
            pass(&mut u, k, &[&g.down]);
        }
    }
    // last: both sides at once, from the top
    for k in 0..n {
        pass(&mut u, k, &[&g.up, &g.down]);
    }
    u
}
