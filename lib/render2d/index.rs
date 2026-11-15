// index -- an R-tree of boxes in f64 (G6d, decision 30; docs/RENDER2D.md "Auswahl und Fang"): the spatial index of a
// scene's entities and of each block's.  No external crate.
//
//   RTree::bulk(items)                 Sort-Tile-Recursive packing: full leaves, a new scene in one go
//   insert(item, box), remove(item), update(item, box)     kept in step with the scene (pick.rs: Scene::sync_index)
//   query(box, f)                      every item whose box meets the box; within(box, inside, partial): those under a node
//                                      wholly inside the box without looking at their boxes (window selection)
//
// A node holds up to M boxes with their children (an inner node) or items (a leaf).  Inserting goes down the child whose
// box grows least (ties: the smaller one) and splits an overfull node along the axis and place that leave the two halves
// overlapping least (then the smaller sum of areas: R*'s split, without its reinsertion).  Removing drops the item from its
// leaf (`leaf_of` finds it); a node left with fewer than MIN entries leaves the tree and its items go in again; the boxes
// above shrink.  An update within its leaf's box stays in place, else it is a remove and an insert.  Boxes are
// [x0, y0, x1, y1], closed (a box of a point has x0 == x1).

use alloc::vec::Vec;

/// the most entries of a node, the fewest (but the root's)
pub(crate) const M: usize = 16;
const MIN: usize = 6;
const NIL: u32 = u32::MAX;

pub(crate) type Bx = [f64; 4];

#[derive(Clone)]
struct Node {
    bx: [Bx; M],
    kid: [u32; M],
    n: u8,
    leaf: bool,
    parent: u32,
}

impl Node {
    fn empty(leaf: bool) -> Node {
        Node { bx: [[0.0; 4]; M], kid: [NIL; M], n: 0, leaf, parent: NIL }
    }

    fn cover(&self) -> Bx {
        union_all(&self.bx[..self.n as usize])
    }
}

pub(crate) fn union(a: &Bx, b: &Bx) -> Bx {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn union_all(b: &[Bx]) -> Bx {
    let mut u = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for x in b {
        u = union(&u, x);
    }
    u
}

fn area(b: &Bx) -> f64 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

/// a box's half perimeter: what an area of 0 (a line, a point) still tells apart
fn margin(b: &Bx) -> f64 {
    (b[2] - b[0]).max(0.0) + (b[3] - b[1]).max(0.0)
}

pub(crate) fn meets(a: &Bx, b: &Bx) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// b inside a
pub(crate) fn holds(a: &Bx, b: &Bx) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && b[2] <= a[2] && b[3] <= a[3]
}

fn overlap(a: &Bx, b: &Bx) -> f64 {
    let w = a[2].min(b[2]) - a[0].max(b[0]);
    let h = a[3].min(b[3]) - a[1].max(b[1]);
    if w < 0.0 || h < 0.0 { 0.0 } else { w * h }
}

/// what the tree costs (IndexInfo)
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stats {
    pub nodes: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Clone, Default)]
pub(crate) struct RTree {
    nodes: Vec<Node>,
    free: Vec<u32>,
    root: u32,
    /// levels below the root (0: the root is a leaf)
    height: u32,
    /// per item: its leaf (NIL: not in the tree); empty for a tree that never changes after bulk (a block's)
    leaf_of: Vec<u32>,
    len: u32,
}

impl RTree {
    pub fn new() -> RTree {
        RTree { nodes: Vec::new(), free: Vec::new(), root: NIL, height: 0, leaf_of: Vec::new(), len: 0 }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn stats(&self) -> Stats {
        let live = self.nodes.len() - self.free.len();
        Stats {
            nodes: live as u32,
            height: if self.root == NIL { 0 } else { self.height + 1 },
            bytes: (self.nodes.capacity() * core::mem::size_of::<Node>() + self.leaf_of.capacity() * 4 + self.free.capacity() * 4) as u64,
        }
    }

    fn alloc(&mut self, n: Node) -> u32 {
        match self.free.pop() {
            Some(k) => {
                self.nodes[k as usize] = n;
                k
            }
            None => {
                self.nodes.push(n);
                self.nodes.len() as u32 - 1
            }
        }
    }

    fn set_leaf(&mut self, item: u32, leaf: u32, track: bool) {
        if !track {
            return;
        }
        let i = item as usize;
        if i >= self.leaf_of.len() {
            self.leaf_of.resize(i + 1, NIL);
        }
        self.leaf_of[i] = leaf;
    }

    /// a new tree of these items (Sort-Tile-Recursive: sorted by x into vertical slices, each by y into full leaves, the
    /// same again for the nodes above).  track: items can be removed and updated later (leaf_of kept)
    pub fn bulk(mut items: Vec<(Bx, u32)>, track: bool) -> RTree {
        let mut t = RTree::new();
        t.len = items.len() as u32;
        if items.is_empty() {
            return t;
        }
        let mut leaf = true;
        loop {
            let n = items.len();
            let pages = n.div_ceil(M);
            let slices = (sqrt_usize(pages)).max(1);
            let per = slices.max(1) * M;
            let cx = |b: &Bx| b[0] + b[2];
            let cy = |b: &Bx| b[1] + b[3];
            items.sort_unstable_by(|a, b| cx(&a.0).total_cmp(&cx(&b.0)));
            let mut out = Vec::with_capacity(pages);
            for slice in items.chunks_mut(per) {
                slice.sort_unstable_by(|a, b| cy(&a.0).total_cmp(&cy(&b.0)));
                for run in slice.chunks(M) {
                    let mut node = Node::empty(leaf);
                    for (k, (bx, id)) in run.iter().enumerate() {
                        node.bx[k] = *bx;
                        node.kid[k] = *id;
                    }
                    node.n = run.len() as u8;
                    let cov = node.cover();
                    let at = t.alloc(node);
                    for &(_, id) in run {
                        if leaf {
                            t.set_leaf(id, at, track);
                        } else {
                            t.nodes[id as usize].parent = at;
                        }
                    }
                    out.push((cov, at));
                }
            }
            if out.len() == 1 {
                t.root = out[0].1;
                break;
            }
            items = out;
            leaf = false;
            t.height += 1;
        }
        t
    }

    /// every item whose box meets q: f(item, its box); f returns false to stop
    pub fn query(&self, q: &Bx, f: &mut dyn FnMut(u32, &Bx) -> bool) {
        if self.root == NIL {
            return;
        }
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        stack.push(self.root);
        while let Some(k) = stack.pop() {
            let n = &self.nodes[k as usize];
            for j in 0..n.n as usize {
                if meets(&n.bx[j], q) {
                    if n.leaf {
                        if !f(n.kid[j], &n.bx[j]) {
                            return;
                        }
                    } else {
                        stack.push(n.kid[j]);
                    }
                }
            }
        }
    }

    /// the items under q: those of a node wholly inside q go to `inside` without a look at their boxes, the others whose box
    /// meets q to `partial` (with the box; the caller decides)
    pub fn within(&self, q: &Bx, inside: &mut dyn FnMut(u32), partial: &mut dyn FnMut(u32, &Bx)) {
        if self.root == NIL {
            return;
        }
        let mut stack: Vec<(u32, bool)> = Vec::with_capacity(64);
        stack.push((self.root, false));
        while let Some((k, all)) = stack.pop() {
            let n = &self.nodes[k as usize];
            for j in 0..n.n as usize {
                let wholly = all || holds(q, &n.bx[j]);
                if !wholly && !meets(&n.bx[j], q) {
                    continue;
                }
                if n.leaf {
                    if wholly { inside(n.kid[j]) } else { partial(n.kid[j], &n.bx[j]) }
                } else {
                    stack.push((n.kid[j], wholly));
                }
            }
        }
    }

    /// a new item (it must not be in the tree)
    pub fn insert(&mut self, item: u32, bx: Bx) {
        self.len += 1;
        if self.root == NIL {
            let mut n = Node::empty(true);
            n.bx[0] = bx;
            n.kid[0] = item;
            n.n = 1;
            self.root = self.alloc(n);
            self.height = 0;
            let r = self.root;
            self.set_leaf(item, r, true);
            return;
        }
        self.place(bx, item, 0);
    }

    /// an entry (an item at level 0, a node at a higher level) into a node of that level: down the least enlargement, then
    /// split upwards where a node overflows
    fn place(&mut self, bx: Bx, id: u32, level: u32) {
        let mut k = self.root;
        let mut depth = self.height;
        while depth > level {
            let n = &self.nodes[k as usize];
            let mut best = 0usize;
            let (mut grow, mut size) = (f64::MAX, f64::MAX);
            for j in 0..n.n as usize {
                let a0 = area(&n.bx[j]);
                let u = union(&n.bx[j], &bx);
                let g = area(&u) - a0 + 1e-300 * (margin(&u) - margin(&n.bx[j]));
                if g < grow || (g == grow && a0 < size) {
                    best = j;
                    grow = g;
                    size = a0;
                }
            }
            let next = n.kid[best];
            let nb = union(&self.nodes[k as usize].bx[best], &bx);
            self.nodes[k as usize].bx[best] = nb;
            k = next;
            depth -= 1;
        }
        self.add_entry(k, bx, id);
    }

    fn add_entry(&mut self, k: u32, bx: Bx, id: u32) {
        let leaf = self.nodes[k as usize].leaf;
        let n = self.nodes[k as usize].n as usize;
        if n < M {
            let node = &mut self.nodes[k as usize];
            node.bx[n] = bx;
            node.kid[n] = id;
            node.n += 1;
            if leaf {
                self.set_leaf(id, k, true);
            } else {
                self.nodes[id as usize].parent = k;
            }
            self.grow_up(k);
            return;
        }
        // overfull: M + 1 entries into two nodes
        let mut all: Vec<(Bx, u32)> = (0..M).map(|j| (self.nodes[k as usize].bx[j], self.nodes[k as usize].kid[j])).collect();
        all.push((bx, id));
        let (a, b) = split(all);
        let parent = self.nodes[k as usize].parent;
        let mut na = Node::empty(leaf);
        for (j, e) in a.iter().enumerate() {
            na.bx[j] = e.0;
            na.kid[j] = e.1;
        }
        na.n = a.len() as u8;
        na.parent = parent;
        self.nodes[k as usize] = na;
        let mut nb = Node::empty(leaf);
        for (j, e) in b.iter().enumerate() {
            nb.bx[j] = e.0;
            nb.kid[j] = e.1;
        }
        nb.n = b.len() as u8;
        nb.parent = parent;
        let k2 = self.alloc(nb);
        for (node, list) in [(k, &a), (k2, &b)] {
            for e in list {
                if leaf {
                    self.set_leaf(e.1, node, true);
                } else {
                    self.nodes[e.1 as usize].parent = node;
                }
            }
        }
        let (ca, cb) = (self.nodes[k as usize].cover(), self.nodes[k2 as usize].cover());
        if parent == NIL {
            // a new root above both
            let mut r = Node::empty(false);
            r.bx[0] = ca;
            r.kid[0] = k;
            r.bx[1] = cb;
            r.kid[1] = k2;
            r.n = 2;
            let rk = self.alloc(r);
            self.nodes[k as usize].parent = rk;
            self.nodes[k2 as usize].parent = rk;
            self.root = rk;
            self.height += 1;
            return;
        }
        let p = &mut self.nodes[parent as usize];
        if let Some(j) = (0..p.n as usize).find(|&j| p.kid[j] == k) {
            p.bx[j] = ca;
        }
        self.add_entry(parent, cb, k2);
    }

    /// node k's box in its parent again, and so on up while it changes
    fn grow_up(&mut self, mut k: u32) {
        loop {
            let parent = self.nodes[k as usize].parent;
            if parent == NIL {
                return;
            }
            let c = self.nodes[k as usize].cover();
            let p = &mut self.nodes[parent as usize];
            let Some(j) = (0..p.n as usize).find(|&j| p.kid[j] == k) else { return };
            if p.bx[j] == c {
                return;
            }
            p.bx[j] = c;
            k = parent;
        }
    }

    /// the item leaves the tree (nothing when it is not in it)
    pub fn remove(&mut self, item: u32) {
        let Some(&leaf) = self.leaf_of.get(item as usize) else { return };
        if leaf == NIL {
            return;
        }
        self.leaf_of[item as usize] = NIL;
        self.len -= 1;
        let n = &mut self.nodes[leaf as usize];
        let Some(j) = (0..n.n as usize).find(|&j| n.kid[j] == item) else { return };
        let last = n.n as usize - 1;
        n.bx[j] = n.bx[last];
        n.kid[j] = n.kid[last];
        n.n -= 1;
        self.condense(leaf);
    }

    /// after a removal from node k: underfull nodes leave the tree (their items go in again), the boxes above shrink
    fn condense(&mut self, mut k: u32) {
        let mut orphans: Vec<(Bx, u32)> = Vec::new();
        loop {
            let parent = self.nodes[k as usize].parent;
            if parent == NIL {
                break;
            }
            if (self.nodes[k as usize].n as usize) < MIN {
                // k goes: out of its parent, its items (all leaves below it) saved
                let p = &mut self.nodes[parent as usize];
                if let Some(j) = (0..p.n as usize).find(|&j| p.kid[j] == k) {
                    let last = p.n as usize - 1;
                    p.bx[j] = p.bx[last];
                    p.kid[j] = p.kid[last];
                    p.n -= 1;
                }
                self.collect(k, &mut orphans);
            } else {
                let c = self.nodes[k as usize].cover();
                let p = &mut self.nodes[parent as usize];
                if let Some(j) = (0..p.n as usize).find(|&j| p.kid[j] == k) {
                    p.bx[j] = c;
                }
            }
            k = parent;
        }
        // the root: an inner one with one child gives way to it; an empty one leaves an empty tree
        loop {
            let r = &self.nodes[self.root as usize];
            if !r.leaf && r.n == 1 {
                let child = r.kid[0];
                let old = self.root;
                self.nodes[old as usize].n = 0;
                self.free.push(old);
                self.root = child;
                self.nodes[child as usize].parent = NIL;
                self.height -= 1;
            } else if !r.leaf && r.n == 0 {
                let old = self.root;
                self.free.push(old);
                self.root = NIL;
                self.height = 0;
                break;
            } else {
                break;
            }
        }
        if self.root != NIL && self.nodes[self.root as usize].leaf && self.nodes[self.root as usize].n == 0 && orphans.is_empty() {
            self.free.push(self.root);
            self.root = NIL;
            self.height = 0;
        }
        for (bx, id) in orphans {
            self.len -= 1;
            self.insert(id, bx);
        }
    }

    /// node k and everything below it to the free list; its items into out
    fn collect(&mut self, k: u32, out: &mut Vec<(Bx, u32)>) {
        let mut stack = alloc::vec![k];
        while let Some(x) = stack.pop() {
            let n = self.nodes[x as usize].clone();
            for j in 0..n.n as usize {
                if n.leaf {
                    out.push((n.bx[j], n.kid[j]));
                    if let Some(l) = self.leaf_of.get_mut(n.kid[j] as usize) {
                        *l = NIL;
                    }
                } else {
                    stack.push(n.kid[j]);
                }
            }
            self.nodes[x as usize].n = 0;
            self.free.push(x);
        }
    }

    /// the item's box changed (it is inserted when it is not in the tree)
    pub fn update(&mut self, item: u32, bx: Bx) {
        let Some(&leaf) = self.leaf_of.get(item as usize) else {
            self.insert(item, bx);
            return;
        };
        if leaf == NIL {
            self.insert(item, bx);
            return;
        }
        // within the leaf's box as its parent holds it: in place, the boxes above shrink or stay
        let parent = self.nodes[leaf as usize].parent;
        let held = if parent == NIL {
            true
        } else {
            let p = &self.nodes[parent as usize];
            (0..p.n as usize).find(|&j| p.kid[j] == leaf).is_some_and(|j| holds(&p.bx[j], &bx))
        };
        if held {
            let n = &mut self.nodes[leaf as usize];
            if let Some(j) = (0..n.n as usize).find(|&j| n.kid[j] == item) {
                n.bx[j] = bx;
            }
            self.shrink_up(leaf);
            return;
        }
        self.remove(item);
        self.insert(item, bx);
    }

    /// node k's box in its parent again (smaller or the same), and so on up while it changes
    fn shrink_up(&mut self, k: u32) {
        self.grow_up(k);
    }

    /// (tests) the tree's invariants: every box holds what is below it, parents point back, leaf_of agrees, the levels are
    /// even, no node but the root is underfull; the items it holds
    pub fn check(&self) -> Result<u32, &'static str> {
        if self.root == NIL {
            return if self.len == 0 { Ok(0) } else { Err("an empty root but items counted") };
        }
        let mut count = 0u32;
        let mut stack = alloc::vec![(self.root, self.height)];
        while let Some((k, lvl)) = stack.pop() {
            let n = &self.nodes[k as usize];
            if n.leaf != (lvl == 0) {
                return Err("a leaf at an inner level or an inner node at the bottom");
            }
            if k != self.root && (n.n as usize) < MIN {
                return Err("an underfull node");
            }
            for j in 0..n.n as usize {
                if n.leaf {
                    count += 1;
                    if !self.leaf_of.is_empty() && self.leaf_of.get(n.kid[j] as usize) != Some(&k) {
                        return Err("leaf_of does not point at the item's leaf");
                    }
                } else {
                    let c = &self.nodes[n.kid[j] as usize];
                    if c.parent != k {
                        return Err("a child that does not point at its parent");
                    }
                    if !holds(&n.bx[j], &c.cover()) {
                        return Err("a box that does not hold its child");
                    }
                    stack.push((n.kid[j], lvl - 1));
                }
            }
        }
        if count != self.len {
            return Err("the items counted differ from those held");
        }
        Ok(count)
    }
}

/// M + 1 entries into two of at least MIN: along the axis whose sorted splits have the least summed margins, the split of
/// least overlap (then least area)
fn split(mut all: Vec<(Bx, u32)>) -> (Vec<(Bx, u32)>, Vec<(Bx, u32)>) {
    let n = all.len();
    let mut best_axis = 0;
    let mut best_margin = f64::MAX;
    for axis in 0..2 {
        for key in 0..2 {
            all.sort_unstable_by(|a, b| a.0[axis + 2 * key].total_cmp(&b.0[axis + 2 * key]));
            let mut m = 0.0;
            for cut in MIN..=n - MIN {
                m += margin(&union_all_e(&all[..cut])) + margin(&union_all_e(&all[cut..]));
            }
            if m < best_margin {
                best_margin = m;
                best_axis = axis + 2 * key;
            }
        }
    }
    all.sort_unstable_by(|a, b| a.0[best_axis].total_cmp(&b.0[best_axis]));
    let mut best = MIN;
    let (mut bo, mut ba) = (f64::MAX, f64::MAX);
    for cut in MIN..=n - MIN {
        let (l, r) = (union_all_e(&all[..cut]), union_all_e(&all[cut..]));
        let (o, a) = (overlap(&l, &r), area(&l) + area(&r));
        if o < bo || (o == bo && a < ba) {
            best = cut;
            bo = o;
            ba = a;
        }
    }
    let right = all.split_off(best);
    (all, right)
}

fn union_all_e(b: &[(Bx, u32)]) -> Bx {
    let mut u = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for x in b {
        u = union(&u, &x.0);
    }
    u
}

fn sqrt_usize(n: usize) -> usize {
    let mut r = crate::math::sqrt(n as f64) as usize;
    while r * r < n {
        r += 1;
    }
    r
}
