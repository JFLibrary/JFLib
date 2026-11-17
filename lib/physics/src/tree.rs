// tree -- the dynamic AABB tree of a world's bodies: leaves with (fattened) boxes, inserted where the
// growth of the surface is smallest, kept balanced by rotations on the way up (the scheme of Box2D's
// b2DynamicTree, written anew).  A moving body keeps its leaf while its box stays inside the fattened
// one.  Deterministic: the same sequence of inserts, moves and removes gives the same tree.

use crate::math::{Aabb, Vec3, inverse_dir, ray_box};
use alloc::vec::Vec;

const NULL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct Node {
    aabb: Aabb,
    /// the parent, or the next free node
    parent: u32,
    child: [u32; 2],
    /// leaves 0, free nodes -1
    height: i32,
    item: u32,
}

/// a dynamic AABB tree over items (u32)
#[derive(Clone, Debug, Default)]
pub struct DynamicTree {
    nodes: Vec<Node>,
    root: u32,
    free: u32,
    leaves: u32,
}

/// a traversal stack: 64 entries on the stack of the caller, more on the heap only when a tree is deeper
struct Stack<T: Copy> {
    fixed: [T; 64],
    len: usize,
    more: Vec<T>,
}

impl<T: Copy> Stack<T> {
    fn new(first: T) -> Stack<T> {
        Stack { fixed: [first; 64], len: 1, more: Vec::new() }
    }

    #[inline]
    fn push(&mut self, v: T) {
        if self.len < 64 {
            self.fixed[self.len] = v;
            self.len += 1;
        } else {
            self.more.push(v);
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<T> {
        if let Some(v) = self.more.pop() {
            return Some(v);
        }
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.fixed[self.len])
    }
}

impl DynamicTree {
    /// an empty tree
    pub fn new() -> DynamicTree {
        DynamicTree { nodes: Vec::new(), root: NULL, free: NULL, leaves: 0 }
    }

    fn alloc(&mut self) -> u32 {
        if self.free != NULL {
            let i = self.free;
            self.free = self.nodes[i as usize].parent;
            return i;
        }
        self.nodes.push(Node { aabb: Aabb::EMPTY, parent: NULL, child: [NULL; 2], height: 0, item: NULL });
        (self.nodes.len() - 1) as u32
    }

    fn release(&mut self, i: u32) {
        let n = &mut self.nodes[i as usize];
        n.parent = self.free;
        n.height = -1;
        n.child = [NULL; 2];
        self.free = i;
    }

    #[inline]
    fn is_leaf(&self, i: u32) -> bool {
        self.nodes[i as usize].child[0] == NULL
    }

    /// a new leaf for `item` with box `aabb` -> its id
    pub fn insert(&mut self, aabb: Aabb, item: u32) -> u32 {
        let leaf = self.alloc();
        let n = &mut self.nodes[leaf as usize];
        n.aabb = aabb;
        n.item = item;
        n.height = 0;
        n.child = [NULL; 2];
        n.parent = NULL;
        self.insert_leaf(leaf);
        self.leaves += 1;
        leaf
    }

    /// remove a leaf
    pub fn remove(&mut self, leaf: u32) {
        self.remove_leaf(leaf);
        self.release(leaf);
        self.leaves -= 1;
    }

    /// give a leaf a new box (it keeps its id)
    pub fn update(&mut self, leaf: u32, aabb: Aabb) {
        self.remove_leaf(leaf);
        self.nodes[leaf as usize].aabb = aabb;
        self.insert_leaf(leaf);
    }

    /// the box of a leaf
    pub fn aabb(&self, leaf: u32) -> Aabb {
        self.nodes[leaf as usize].aabb
    }

    /// the number of leaves
    pub fn len(&self) -> usize {
        self.leaves as usize
    }

    /// no leaves
    pub fn is_empty(&self) -> bool {
        self.leaves == 0
    }

    /// the height of the root (0 for one leaf or none)
    pub fn height(&self) -> u32 {
        if self.root == NULL { 0 } else { self.nodes[self.root as usize].height as u32 }
    }

    fn insert_leaf(&mut self, leaf: u32) {
        if self.root == NULL {
            self.root = leaf;
            self.nodes[leaf as usize].parent = NULL;
            return;
        }
        let lb = self.nodes[leaf as usize].aabb;
        let mut index = self.root;
        while !self.is_leaf(index) {
            let n = self.nodes[index as usize];
            let (c1, c2) = (n.child[0], n.child[1]);
            let area = n.aabb.half_area();
            let combined = n.aabb.union(&lb).half_area();
            let cost = 2.0 * combined;
            let inherit = 2.0 * (combined - area);
            let child_cost = |c: u32| {
                let cb = self.nodes[c as usize].aabb;
                let u = lb.union(&cb).half_area();
                if self.is_leaf(c) { u + inherit } else { u - cb.half_area() + inherit }
            };
            let cost1 = child_cost(c1);
            let cost2 = child_cost(c2);
            if cost < cost1 && cost < cost2 {
                break;
            }
            index = if cost1 < cost2 { c1 } else { c2 };
        }
        let sibling = index;
        let old_parent = self.nodes[sibling as usize].parent;
        let np = self.alloc();
        let sb = self.nodes[sibling as usize];
        {
            let n = &mut self.nodes[np as usize];
            n.parent = old_parent;
            n.aabb = lb.union(&sb.aabb);
            n.height = sb.height + 1;
            n.child = [sibling, leaf];
            n.item = NULL;
        }
        if old_parent != NULL {
            let op = &mut self.nodes[old_parent as usize];
            if op.child[0] == sibling {
                op.child[0] = np;
            } else {
                op.child[1] = np;
            }
        } else {
            self.root = np;
        }
        self.nodes[sibling as usize].parent = np;
        self.nodes[leaf as usize].parent = np;
        self.fix_upwards(self.nodes[leaf as usize].parent);
    }

    fn fix_upwards(&mut self, mut index: u32) {
        while index != NULL {
            index = self.balance(index);
            let n = self.nodes[index as usize];
            let (a, b) = (self.nodes[n.child[0] as usize], self.nodes[n.child[1] as usize]);
            let m = &mut self.nodes[index as usize];
            m.height = 1 + a.height.max(b.height);
            m.aabb = a.aabb.union(&b.aabb);
            index = m.parent;
        }
    }

    fn remove_leaf(&mut self, leaf: u32) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }
        let parent = self.nodes[leaf as usize].parent;
        let grand = self.nodes[parent as usize].parent;
        let pc = self.nodes[parent as usize].child;
        let sibling = if pc[0] == leaf { pc[1] } else { pc[0] };
        if grand != NULL {
            let g = &mut self.nodes[grand as usize];
            if g.child[0] == parent {
                g.child[0] = sibling;
            } else {
                g.child[1] = sibling;
            }
            self.nodes[sibling as usize].parent = grand;
            self.release(parent);
            self.fix_upwards(grand);
        } else {
            self.root = sibling;
            self.nodes[sibling as usize].parent = NULL;
            self.release(parent);
        }
    }

    fn set_child(&mut self, parent: u32, old: u32, new: u32) {
        if parent == NULL {
            self.root = new;
            return;
        }
        let p = &mut self.nodes[parent as usize];
        if p.child[0] == old {
            p.child[0] = new;
        } else {
            p.child[1] = new;
        }
    }

    /// an AVL rotation at `ia` when its children's heights differ by more than one -> the node now at its place
    fn balance(&mut self, ia: u32) -> u32 {
        let a = self.nodes[ia as usize];
        if a.child[0] == NULL || a.height < 2 {
            return ia;
        }
        let (ib, ic) = (a.child[0], a.child[1]);
        let (b, c) = (self.nodes[ib as usize], self.nodes[ic as usize]);
        let bal = c.height - b.height;
        if bal > 1 {
            // C up
            let (iff, ig) = (c.child[0], c.child[1]);
            let (f, g) = (self.nodes[iff as usize], self.nodes[ig as usize]);
            self.nodes[ic as usize].child[0] = ia;
            self.nodes[ic as usize].parent = a.parent;
            self.nodes[ia as usize].parent = ic;
            self.set_child(a.parent, ia, ic);
            if f.height > g.height {
                self.nodes[ic as usize].child[1] = iff;
                self.nodes[ia as usize].child[1] = ig;
                self.nodes[ig as usize].parent = ia;
                let aa = b.aabb.union(&g.aabb);
                let ah = 1 + b.height.max(g.height);
                self.nodes[ia as usize].aabb = aa;
                self.nodes[ia as usize].height = ah;
                self.nodes[ic as usize].aabb = aa.union(&f.aabb);
                self.nodes[ic as usize].height = 1 + ah.max(f.height);
            } else {
                self.nodes[ic as usize].child[1] = ig;
                self.nodes[ia as usize].child[1] = iff;
                self.nodes[iff as usize].parent = ia;
                let aa = b.aabb.union(&f.aabb);
                let ah = 1 + b.height.max(f.height);
                self.nodes[ia as usize].aabb = aa;
                self.nodes[ia as usize].height = ah;
                self.nodes[ic as usize].aabb = aa.union(&g.aabb);
                self.nodes[ic as usize].height = 1 + ah.max(g.height);
            }
            return ic;
        }
        if bal < -1 {
            // B up
            let (id, ie) = (b.child[0], b.child[1]);
            let (d, e) = (self.nodes[id as usize], self.nodes[ie as usize]);
            self.nodes[ib as usize].child[0] = ia;
            self.nodes[ib as usize].parent = a.parent;
            self.nodes[ia as usize].parent = ib;
            self.set_child(a.parent, ia, ib);
            if d.height > e.height {
                self.nodes[ib as usize].child[1] = id;
                self.nodes[ia as usize].child[0] = ie;
                self.nodes[ie as usize].parent = ia;
                let aa = c.aabb.union(&e.aabb);
                let ah = 1 + c.height.max(e.height);
                self.nodes[ia as usize].aabb = aa;
                self.nodes[ia as usize].height = ah;
                self.nodes[ib as usize].aabb = aa.union(&d.aabb);
                self.nodes[ib as usize].height = 1 + ah.max(d.height);
            } else {
                self.nodes[ib as usize].child[1] = ie;
                self.nodes[ia as usize].child[0] = id;
                self.nodes[id as usize].parent = ia;
                let aa = c.aabb.union(&d.aabb);
                let ah = 1 + c.height.max(d.height);
                self.nodes[ia as usize].aabb = aa;
                self.nodes[ia as usize].height = ah;
                self.nodes[ib as usize].aabb = aa.union(&e.aabb);
                self.nodes[ib as usize].height = 1 + ah.max(e.height);
            }
            return ib;
        }
        ia
    }

    /// call `f(item)` for every leaf whose box touches `bx`
    pub fn query(&self, bx: &Aabb, mut f: impl FnMut(u32)) {
        if self.root == NULL {
            return;
        }
        let mut stack = Stack::new(self.root);
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if !n.aabb.intersects(bx) {
                continue;
            }
            if n.child[0] == NULL {
                f(n.item);
            } else {
                stack.push(n.child[1]);
                stack.push(n.child[0]);
            }
        }
    }

    /// the leaves whose box, grown by `ext`, the ray o + t d enters at t <= tmax, nearer subtrees first;
    /// `f(item, tmax)` returns the new tmax -> the last tmax
    pub fn ray(&self, o: Vec3, d: Vec3, ext: Vec3, mut tmax: f64, mut f: impl FnMut(u32, f64) -> f64) -> f64 {
        if self.root == NULL {
            return tmax;
        }
        let inv = inverse_dir(d);
        let enter = |n: &Node, tmax: f64| ray_box(o, inv, n.aabb.min - ext, n.aabb.max + ext, tmax);
        let Some(t0) = enter(&self.nodes[self.root as usize], tmax) else {
            return tmax;
        };
        let mut stack = Stack::new((self.root, t0));
        while let Some((i, te)) = stack.pop() {
            if te > tmax {
                continue;
            }
            let n = &self.nodes[i as usize];
            if n.child[0] == NULL {
                tmax = f(n.item, tmax);
                continue;
            }
            let (l, r) = (n.child[0], n.child[1]);
            let tl = enter(&self.nodes[l as usize], tmax);
            let tr = enter(&self.nodes[r as usize], tmax);
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    if b < a {
                        stack.push((l, a));
                        stack.push((r, b));
                    } else {
                        stack.push((r, b));
                        stack.push((l, a));
                    }
                }
                (Some(a), None) => stack.push((l, a)),
                (None, Some(b)) => stack.push((r, b)),
                (None, None) => {}
            }
        }
        tmax
    }

    /// (tests) the tree is consistent: parents, heights, boxes holding their children, the leaf count -> an
    /// error text
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.root == NULL {
            return if self.leaves == 0 { Ok(()) } else { Err("leaves without a root") };
        }
        if self.nodes[self.root as usize].parent != NULL {
            return Err("the root has a parent");
        }
        let mut leaves = 0u32;
        let mut stack: Vec<u32> = alloc::vec![self.root];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if n.child[0] == NULL {
                if n.height != 0 {
                    return Err("a leaf's height is not 0");
                }
                leaves += 1;
                continue;
            }
            let (a, b) = (&self.nodes[n.child[0] as usize], &self.nodes[n.child[1] as usize]);
            if a.parent != i || b.parent != i {
                return Err("a child's parent is wrong");
            }
            if n.height != 1 + a.height.max(b.height) {
                return Err("a height is wrong");
            }
            if !n.aabb.contains(&a.aabb) || !n.aabb.contains(&b.aabb) {
                return Err("a box does not hold its children");
            }
            stack.push(n.child[0]);
            stack.push(n.child[1]);
        }
        if leaves != self.leaves {
            return Err("the leaf count is wrong");
        }
        Ok(())
    }
}
