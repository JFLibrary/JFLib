// hashring -- consistent hashing: a ring of tokens with virtual nodes per member; a key belongs
// to the member of the first token at or after its hash (going round at the end), and its
// preference list is the next n distinct members from there (sharded stores, load balancers
// with sticky keys).  Adding or removing a member moves only the keys of its own tokens.
//
//   hash64(bytes) -> u64                    FNV-1a (64 bit) with a final mix; the same on every
//                                           target
//   Ring::new() / add(member, key, vnodes) / remove(&member) / members() / tokens()
//   owner(h) -> Option<&M>                  the member of the first token at or after h
//   preference(h, n) -> Vec<M>              the first n distinct members from h on
//   preference_by(h, n, ok)                 the same, skipping members ok() refuses
//   shares() -> Vec<(M, u32)>               each member's part of the ring in parts per million
//
// The tokens of a member come from its key and the vnode number ("10.0.4.11#3"), so every
// device that knows the members builds the same ring.

use alloc::format;
use alloc::vec::Vec;

/// FNV-1a over the bytes, then the finaliser of SplitMix64 (spreads close keys over the ring)
pub fn hash64(b: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let mut z = h;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// a ring of tokens; M names a member (an address, a backend)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ring<M: Ord + Clone> {
    /// (token, member), sorted
    tokens: Vec<(u64, M)>,
    members: Vec<M>,
}

impl<M: Ord + Clone> Default for Ring<M> {
    fn default() -> Self {
        Ring::new()
    }
}

impl<M: Ord + Clone> Ring<M> {
    pub fn new() -> Ring<M> {
        Ring { tokens: Vec::new(), members: Vec::new() }
    }
    /// adds a member with `vnodes` tokens made from `key` (a member that is there already gets
    /// its tokens again)
    pub fn add(&mut self, m: M, key: &str, vnodes: u32) {
        self.remove(&m);
        for i in 0..vnodes.max(1) {
            self.tokens.push((hash64(format!("{}#{}", key, i).as_bytes()), m.clone()));
        }
        self.tokens.sort();
        self.members.push(m);
        self.members.sort();
    }
    /// removes a member and its tokens
    pub fn remove(&mut self, m: &M) -> bool {
        let before = self.members.len();
        self.members.retain(|x| x != m);
        self.tokens.retain(|t| &t.1 != m);
        self.members.len() != before
    }
    /// the members, sorted
    pub fn members(&self) -> &[M] {
        &self.members
    }
    /// the tokens with their members, sorted by token
    pub fn tokens(&self) -> &[(u64, M)] {
        &self.tokens
    }
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
    /// the index of the first token at or after h (0 after the last)
    fn start(&self, h: u64) -> usize {
        let i = self.tokens.partition_point(|t| t.0 < h);
        if i == self.tokens.len() { 0 } else { i }
    }
    /// the member a hash belongs to
    pub fn owner(&self, h: u64) -> Option<&M> {
        if self.tokens.is_empty() {
            return None;
        }
        Some(&self.tokens[self.start(h)].1)
    }
    /// the first `n` distinct members from h on, in ring order
    pub fn preference(&self, h: u64, n: usize) -> Vec<M> {
        self.preference_by(h, n, |_| true)
    }
    /// the first `n` distinct members from h on that `ok` accepts
    pub fn preference_by(&self, h: u64, n: usize, ok: impl Fn(&M) -> bool) -> Vec<M> {
        let mut out: Vec<M> = Vec::new();
        if self.tokens.is_empty() || n == 0 {
            return out;
        }
        let s = self.start(h);
        for k in 0..self.tokens.len() {
            let m = &self.tokens[(s + k) % self.tokens.len()].1;
            if !out.contains(m) && ok(m) {
                out.push(m.clone());
                if out.len() == n {
                    break;
                }
            }
        }
        out
    }
    /// the part of the hash space each member owns, in parts per million (sorted by member)
    pub fn shares(&self) -> Vec<(M, u32)> {
        let n = self.tokens.len();
        let mut out: Vec<(M, u32)> = self.members.iter().map(|m| (m.clone(), 0)).collect();
        if n == 0 {
            return out;
        }
        let mut arcs: Vec<u128> = alloc::vec![0; self.members.len()];
        for i in 0..n {
            // the arc that ends at token i: from the token before it (exclusive)
            let prev = if i == 0 { self.tokens[n - 1].0 } else { self.tokens[i - 1].0 };
            let cur = self.tokens[i].0;
            let len = if n == 1 { 1u128 << 64 } else { cur.wrapping_sub(prev) as u128 };
            if let Ok(k) = self.members.binary_search(&self.tokens[i].1) {
                arcs[k] += len;
            }
        }
        for (k, a) in arcs.iter().enumerate() {
            out[k].1 = ((a * 1_000_000) >> 64) as u32;
        }
        out
    }
}
