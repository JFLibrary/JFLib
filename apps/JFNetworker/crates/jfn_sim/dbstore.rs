// dbstore -- what the database service (db.rs) stores and how it finds it: modelled tables with only the
// touched rows real, their versions, the plan of a statement with its pages, and the buffer pool
//
//   TableSpec                    what a table says of itself (name, rows, row_bytes, key, dist, columns,
//                                indexes); shared by every clone of the engine
//   Table                        a table at run time: spec, indexes, touched rows (Row, Version), what
//                                moved in its indexes
//   Table::derived(col, key)     the value of an untouched row: from its key and the table's seed, so a
//                                table of a billion rows holds nothing
//   Table::find(...) -> Found    the rows of a condition at a snapshot, in closed form or by jumps: no step
//                                per row that does not match
//   Table::draw(rng) / key_of_rank(rank)   a key by the table's distribution (uniform, zipf); ranks are
//                                scattered over the keys, so hot rows do not share their pages
//   Layout                       page size and fan-out; rows per page, depth of a tree
//   Plan, Cost, plan(...)        which way a statement goes and what it touches: EXPLAIN and the charge
//                                come from this one place
//   Pool                         the buffer pool: LRU over at most SLOTS tracked pages
//   Zipf                         integer table of a zipf distribution: the same on every target
//
// Untouched rows: a column's value is (key - 1 + off) mod distinct, off from the seed, the table and the
// column; a text column writes it "name-N".  So the rows of a value have a closed form, and counting or
// jumping to the next match needs no walk.  A column thus runs along the key: nothing here models values
// independent of the key.
//
// Touched rows: a row that was written has its versions (Row): what changed against the derived values, in
// order, the newest last; a version has the commit stamp of its transaction (0: open) and its log position.
// Their number is bounded (db.rs "touched_max"): past it the oldest clean row, which no lock and no snapshot
// needs, goes back to its derived values, and that is counted (forgotten): a limit of the simulator, not of
// a database.  Figures of time stay right; the values of such a row are lost.
//
// Pages: a table has a heap (rows in key order, heap page = (key - 1) / rows per page) and a tree per index,
// the primary one over the key.  A statement goes through the upper levels of a tree, its leaf and the heap.
// The upper levels of a tree stay in the pool once read (Pool::upper), since they are the hottest pages
// there are.  A long scan of a big table (more pages than a quarter of the pool) does not push out what the
// pool holds, as real engines read those through a small ring: its hits are the share of the table in the
// pool at that moment.
//
// The pool is honest about its granularity: at most SLOTS pages are tracked.  A pool of up to SLOTS pages
// is an exact LRU.  A larger one tracks 1 page in R (by a hash of the page, R = pool / SLOTS) in an LRU of
// pool / R slots, the way hit ratio curves are sampled; an access to a page that is not tracked is a hit at
// the rate the tracked ones of its class (leaves of trees, heap pages) have had lately, by an accumulator, not by
// chance.  What the pool holds is then counted in steps of R pages.  A tree with few leaves (up to SMALL_TREE)
// keeps them like its upper levels in such a pool: each of them is far hotter than a heap page, and a sample
// could miss them all.  The cost of a clone stays that of SLOTS entries for any pool size.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use base::rng::Pcg32;

/// pages a pool tracks at most
pub const SLOTS: u64 = 16_384;
/// the most pages one statement walks through the pool one by one; beyond it the share in the pool decides
const TOUCH_MAX: u64 = 4096;
/// in a sampled pool the leaves of a tree this small stay, like its upper levels
const SMALL_TREE: u64 = 4096;
/// the bytes of an index entry (key and pointer), for fan-out and the size of an index
const ENTRY_BYTES: u64 = 16;

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// a value of a column
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Val {
    Int(i64),
    Text(String),
}

impl Val {
    /// as JSON
    pub fn json(&self, out: &mut String) {
        match self {
            Val::Int(v) => out.push_str(&format!("{}", v)),
            Val::Text(t) => {
                out.push('"');
                for c in t.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
        }
    }
}

/// how a table's keys are asked for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dist {
    Uniform,
    Zipf,
}

/// a column besides the key
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColDef {
    pub name: String,
    pub text: bool,
    /// how many values it has (a column that says nothing: as many as the table has rows)
    pub distinct: u64,
    off: u64,
}

/// what a table says of itself
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSpec {
    pub name: String,
    pub key: String,
    pub rows: u64,
    pub row_bytes: u64,
    pub dist: Dist,
    /// the exponent of zipf in thousandths
    pub zipf_s: u32,
    pub cols: Vec<ColDef>,
    pub indexes: Vec<String>,
}

impl TableSpec {
    /// a column by name: 0 the key, then the others from 1
    pub fn col(&self, name: &str) -> Option<usize> {
        if name.eq_ignore_ascii_case(&self.key) {
            return Some(0);
        }
        self.cols.iter().position(|c| c.name.eq_ignore_ascii_case(name)).map(|i| i + 1)
    }
    pub fn col_name(&self, c: usize) -> &str {
        if c == 0 { &self.key } else { self.cols.get(c - 1).map_or("", |x| x.name.as_str()) }
    }
    pub fn is_text(&self, c: usize) -> bool {
        c > 0 && self.cols.get(c - 1).is_some_and(|x| x.text)
    }
    /// the offsets of its columns follow from the seed and the names
    pub fn seeded(mut self, seed: u64) -> TableSpec {
        let t = base::hashring::hash64(self.name.as_bytes()) ^ seed;
        let rows = self.rows.max(1);
        for c in self.cols.iter_mut() {
            if c.distinct == 0 || c.distinct > rows {
                c.distinct = rows;
            }
            c.off = mix(t ^ base::hashring::hash64(c.name.as_bytes())) % c.distinct;
        }
        self
    }
}

/// a column of a config: {"name": "balance", "type": "int", "distinct": 1000} or just its name
pub fn column(name: &str, text: bool, distinct: u64) -> ColDef {
    ColDef { name: String::from(name), text, distinct, off: 0 }
}

/// page size and fan-out
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub page: u64,
    /// entries of a page of a tree
    pub fanout: u64,
}

impl Layout {
    pub fn new(page: u64, fanout: u64) -> Layout {
        Layout { page, fanout: if fanout >= 2 { fanout } else { (page / ENTRY_BYTES).max(2) } }
    }
    /// rows of a heap page
    pub fn rows_per_page(&self, row_bytes: u64) -> u64 {
        (self.page / row_bytes.max(1)).max(1)
    }
    /// pages of `n` rows
    pub fn heap_pages(&self, n: u64, row_bytes: u64) -> u64 {
        n.div_ceil(self.rows_per_page(row_bytes))
    }
    /// leaf pages of a tree of `n` entries
    pub fn leaves(&self, n: u64) -> u64 {
        n.div_ceil(self.fanout).max(1)
    }
    /// levels of a tree of `n` entries: log to the fan-out, the leaves included
    pub fn depth(&self, n: u64) -> u64 {
        let (mut x, mut d) = (self.leaves(n), 1);
        while x > 1 {
            x = x.div_ceil(self.fanout);
            d += 1;
        }
        d
    }
    /// all pages of a tree of `n` entries
    pub fn tree_pages(&self, n: u64) -> u64 {
        let (mut x, mut sum) = (self.leaves(n), 0);
        loop {
            sum += x;
            if x <= 1 {
                return sum;
            }
            x = x.div_ceil(self.fanout);
        }
    }
}

// ---------------------------------------------------------------- zipf

/// the rank of a draw: exact up to rank 64, then buckets 3 % wide with a uniform choice inside.  Integer
/// arithmetic, so the same bytes on every target
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zipf {
    n: u64,
    starts: Vec<u64>,
    cum: Vec<u64>,
}

/// log2(y) in 32.32 for y >= 1 (bit by bit)
fn log2_fx(y: u64) -> u64 {
    let ip = 63 - y.leading_zeros() as u64;
    let mut x: u128 = ((y as u128) << 32) >> ip;
    let mut frac: u64 = 0;
    for i in (0..32).rev() {
        x = (x * x) >> 32;
        if x >= (2u128 << 32) {
            x >>= 1;
            frac |= 1 << i;
        }
    }
    (ip << 32) | frac
}

/// 2^-x in 0.62 for x in 32.32
fn pow2_neg(x: u64, roots: &[u128; 32]) -> u128 {
    let ip = (x >> 32).min(126);
    let f = x & 0xFFFF_FFFF;
    let mut v: u128 = 1 << 62;
    for (i, r) in roots.iter().enumerate() {
        if f & (1 << (31 - i)) != 0 {
            v = (v * *r) >> 62;
        }
    }
    v >> ip
}

impl Zipf {
    /// `n` ranks with the exponent s / 1000
    pub fn new(n: u64, s: u32) -> Zipf {
        // 2^(-1/2), 2^(-1/4), ... in 0.62
        let mut roots = [0u128; 32];
        let mut r: u128 = 1 << 62;
        // (one half first: 2^-1, and each root of it)
        let half: u128 = 1 << 61;
        r = crate::cli::fmt::isqrt(half * r);
        roots[0] = r;
        for i in 1..32 {
            r = crate::cli::fmt::isqrt(r << 62);
            roots[i] = r;
        }
        let n = n.max(1);
        let (mut starts, mut weights) = (Vec::new(), Vec::new());
        let mut a = 1u64;
        while a <= n {
            let b = (a + (a / 32).max(1)).min(n + 1);
            let mid = (log2_fx(a) + log2_fx(b - 1)) / 2;
            let x = ((mid as u128 * s as u128) / 1000) as u64;
            starts.push(a);
            weights.push(pow2_neg(x, &roots) * (b - a) as u128);
            a = b;
        }
        let total: u128 = weights.iter().sum::<u128>().max(1);
        let mut cum = Vec::with_capacity(weights.len());
        let mut sum = 0u128;
        for w in weights {
            sum += w;
            // of 2^63 (the shifts keep it within 128 bits)
            cum.push(((sum << 22) / (total >> 41).max(1)) as u64);
        }
        Zipf { n, starts, cum }
    }
    /// a rank from 1 from two random numbers
    pub fn rank(&self, u: u64, v: u32) -> u64 {
        let x = u >> 1;
        let i = self.cum.partition_point(|c| *c <= x).min(self.starts.len().saturating_sub(1));
        let a = self.starts.get(i).copied().unwrap_or(1);
        let b = self.starts.get(i + 1).copied().unwrap_or(self.n + 1);
        a + (v as u64 * (b - a)) / (1u64 << 32)
    }
    /// the share of a rank's bucket per rank, in parts of 2^63: what a test compares with the formula
    pub fn share(&self, rank: u64) -> u64 {
        let i = self.starts.partition_point(|s| *s <= rank).saturating_sub(1);
        let a = self.starts.get(i).copied().unwrap_or(1);
        let b = self.starts.get(i + 1).copied().unwrap_or(self.n + 1);
        let from = if i == 0 { 0 } else { self.cum[i - 1] };
        (self.cum.get(i).copied().unwrap_or(0) - from) / (b - a).max(1)
    }
    pub fn bytes(&self) -> usize {
        (self.starts.len() + self.cum.len()) * 8
    }
}

// ---------------------------------------------------------------- rows

/// a version of a touched row
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// its commit stamp; 0: its transaction is open
    pub ts: u64,
    pub txn: u64,
    /// the log position of the change
    pub lsn: u64,
    pub deleted: bool,
    /// the columns that differ from the derived values
    pub vals: Vec<(u16, Val)>,
}

/// a touched row
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// the newest last
    pub vers: Vec<Version>,
    /// written since the bound's hand last came by
    pub refd: bool,
}

/// who reads: the stamp of its snapshot, its own transaction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snap {
    pub ts: u64,
    pub txn: u64,
}

/// a condition on a column, in the table's terms
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RCond {
    /// values from .. to, both included
    Int { col: usize, lo: i64, hi: i64 },
    /// one text; derived: the number of a derived value it can be ("name-17")
    Text { col: usize, val: String, derived: Option<i64> },
}

impl RCond {
    pub fn col(&self) -> usize {
        match self {
            RCond::Int { col, .. } | RCond::Text { col, .. } => *col,
        }
    }
}

/// what a table holds at run time
#[derive(Clone, Debug)]
pub struct Table {
    pub spec: Arc<TableSpec>,
    pub no: u16,
    /// the columns with an index, the key's not among them
    pub indexes: Vec<usize>,
    pub rows: BTreeMap<u64, Row>,
    /// rows whose indexed value differs from the derived one: (column, value) -> keys (a stale entry
    /// is found out by reading the row)
    moved: BTreeMap<(u16, Val), BTreeSet<u64>>,
    /// the highest key so far
    pub next_key: u64,
    zipf: Option<Arc<Zipf>>,
    scatter: (u64, u64),
    /// the hand of the bound on touched rows
    pub hand: u64,
}

/// the rows of a condition
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// their keys, at most as many as were wanted
    pub keys: Vec<u64>,
    /// how many match (with a LIMIT: how many were needed)
    pub matches: u64,
    /// how many rows were looked at
    pub examined: u64,
}

impl Table {
    pub fn new(spec: TableSpec, no: u16, seed: u64) -> Table {
        let spec = spec.seeded(seed);
        let indexes = spec.indexes.iter().filter_map(|n| spec.col(n)).filter(|c| *c != 0).collect();
        let zipf = (spec.dist == Dist::Zipf).then(|| Arc::new(Zipf::new(spec.rows, spec.zipf_s)));
        let n = spec.rows.max(1);
        // an offset and a multiplier without a common factor with the rows: ranks spread over the keys
        let t = mix(seed ^ base::hashring::hash64(spec.name.as_bytes()));
        let mut a = if n > 2 { (t % (n - 1)) + 1 } else { 1 };
        while gcd(a, n) != 1 {
            a = a % n + 1;
        }
        let next_key = spec.rows;
        Table { spec: Arc::new(spec), no, indexes, rows: BTreeMap::new(), moved: BTreeMap::new(), next_key, zipf, scatter: (a, mix(t) % n), hand: 0 }
    }
    /// a value of an untouched row, as a number
    fn base_int(&self, c: usize, key: u64) -> i64 {
        if c == 0 {
            return key as i64;
        }
        let d = &self.spec.cols[c - 1];
        (((key - 1) as u128 + d.off as u128) % d.distinct.max(1) as u128) as i64
    }
    /// a value of an untouched row
    pub fn derived(&self, c: usize, key: u64) -> Val {
        let n = self.base_int(c, key);
        if self.spec.is_text(c) { Val::Text(format!("{}-{}", self.spec.col_name(c), n)) } else { Val::Int(n) }
    }
    /// the number inside a derived text of a column
    pub fn derived_of_text(&self, c: usize, text: &str) -> Option<i64> {
        let n = text.strip_prefix(self.spec.col_name(c))?.strip_prefix('-')?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        n.parse().ok()
    }
    /// the key of a rank of the table's distribution
    pub fn key_of_rank(&self, rank: u64) -> u64 {
        let n = self.spec.rows.max(1) as u128;
        (((rank - 1) as u128 * self.scatter.0 as u128 + self.scatter.1 as u128) % n) as u64 + 1
    }
    /// a key by the table's distribution
    pub fn draw(&self, rng: &mut Pcg32) -> u64 {
        match &self.zipf {
            Some(z) => {
                let (u, v) = (rng.next_u64(), rng.next_u32());
                self.key_of_rank(z.rank(u, v))
            }
            None => (((rng.next_u64() >> 1) as u128 * self.spec.rows.max(1) as u128) >> 63) as u64 + 1,
        }
    }
    /// the zipf table (a test's closed form)
    pub fn zipf(&self) -> Option<&Zipf> {
        self.zipf.as_deref()
    }
    /// the version a reader sees: Ok(None): the derived row; Err: no row for it
    fn seen<'a>(&'a self, key: u64, s: Snap) -> Result<Option<&'a Version>, ()> {
        let base = key >= 1 && key <= self.spec.rows;
        let Some(r) = self.rows.get(&key) else {
            return if base { Ok(None) } else { Err(()) };
        };
        for v in r.vers.iter().rev() {
            if (v.ts == 0 && s.txn != 0 && v.txn == s.txn) || (v.ts != 0 && v.ts <= s.ts) {
                return if v.deleted { Err(()) } else { Ok(Some(v)) };
            }
        }
        if base { Ok(None) } else { Err(()) }
    }
    /// the newest committed version (or this transaction's own), for a write under its lock
    fn newest<'a>(&'a self, key: u64, txn: u64) -> Result<Option<&'a Version>, ()> {
        self.seen(key, Snap { ts: u64::MAX, txn })
    }
    /// the stamp of the newest committed version of a row (0: untouched)
    pub fn newest_ts(&self, key: u64) -> u64 {
        self.rows.get(&key).and_then(|r| r.vers.iter().rev().find(|v| v.ts != 0)).map_or(0, |v| v.ts)
    }
    /// a value of a row
    pub fn value(&self, key: u64, ver: Option<&Version>, c: usize) -> Val {
        if c == 0 {
            return Val::Int(key as i64);
        }
        if let Some(v) = ver.and_then(|v| v.vals.iter().find(|x| x.0 as usize == c)) {
            return v.1.clone();
        }
        self.derived(c, key)
    }
    fn holds(&self, key: u64, ver: Option<&Version>, c: &RCond) -> bool {
        match c {
            RCond::Int { col, lo, hi } => {
                if let Some(v) = ver.and_then(|v| v.vals.iter().find(|x| x.0 as usize == *col)) {
                    return matches!(&v.1, Val::Int(n) if n >= lo && n <= hi);
                }
                let n = self.base_int(*col, key);
                n >= *lo && n <= *hi
            }
            RCond::Text { col, val, derived } => {
                if let Some(v) = ver.and_then(|v| v.vals.iter().find(|x| x.0 as usize == *col)) {
                    return matches!(&v.1, Val::Text(t) if t == val);
                }
                *derived == Some(self.base_int(*col, key))
            }
        }
    }
    fn holds_all(&self, key: u64, ver: Option<&Version>, cs: &[RCond]) -> bool {
        cs.iter().all(|c| self.holds(key, ver, c))
    }
    /// the row with the newest committed values holds the conditions still (a write that waited)
    pub fn holds_now(&self, key: u64, txn: u64, cs: &[RCond]) -> bool {
        match self.newest(key, txn) {
            Ok(v) => self.holds_all(key, v, cs),
            Err(()) => false,
        }
    }
    /// the row exists for a reader
    pub fn exists(&self, key: u64, s: Snap) -> bool {
        self.seen(key, s).is_ok()
    }
    /// its values for a reader, the key first
    pub fn image(&self, key: u64, s: Snap) -> Option<Vec<Val>> {
        let v = self.seen(key, s).ok()?;
        Some((0..=self.spec.cols.len()).map(|c| self.value(key, v, c)).collect())
    }
    /// the smallest key from `k` on whose derived row can hold this one condition
    fn jump(&self, k: u64, c: &RCond) -> Option<u64> {
        let (col, a, b) = match c {
            RCond::Int { col, lo, hi } => (*col, *lo, *hi),
            RCond::Text { col, derived, .. } => {
                let n = (*derived)?;
                (*col, n, n)
            }
        };
        if col == 0 {
            if (b as u64) < k || b < 0 {
                return None;
            }
            return Some(k.max(a.max(0) as u64));
        }
        let d = self.spec.cols[col - 1].distinct.max(1);
        let (a, b) = (a.max(0) as u64, (b.min((d - 1).min(i64::MAX as u64) as i64)).max(-1));
        if b < 0 || a > b as u64 {
            return None;
        }
        let b = b as u64;
        let r = (((k - 1) as u128 + self.spec.cols[col - 1].off as u128) % d as u128) as u64;
        if r >= a && r <= b {
            return Some(k);
        }
        Some(k + (a + d - r) % d)
    }
    /// the next key of the table's derived rows that holds them all
    fn next_base(&self, mut k: u64, hi: u64, cs: &[RCond]) -> Option<u64> {
        let hi = hi.min(self.spec.rows);
        'walk: while k <= hi {
            for c in cs {
                let n = self.jump(k, c)?;
                if n > k {
                    k = n;
                    continue 'walk;
                }
            }
            return Some(k);
        }
        None
    }
    /// how many of the derived rows from `lo` to `hi` have the value in [a, b] of one column
    fn count_col(&self, c: &RCond, lo: u64, hi: u64) -> u64 {
        let (col, a, b) = match c {
            RCond::Int { col, lo, hi } => (*col, *lo, *hi),
            RCond::Text { col, derived, .. } => match derived {
                Some(n) => (*col, *n, *n),
                None => return 0,
            },
        };
        let d = self.spec.cols[col - 1].distinct.max(1) as u128;
        let off = self.spec.cols[col - 1].off as u128;
        let a = a.max(0) as u128;
        if b < 0 || a >= d {
            return 0;
        }
        let b = (b as u128).min(d - 1);
        if a > b {
            return 0;
        }
        let width = b - a + 1;
        // how many of the positions 0 .. y have their rest modulo d in [a, b]
        let g = |y: u128| (y / d) * width + (y % d).saturating_sub(a).min(width);
        // (the keys lo - 1 .. hi - 1 at their offset)
        (g(off + hi as u128) - g(off + lo as u128 - 1)) as u64
    }
    /// how many rows from `lo` to `hi` hold the conditions for a reader
    pub fn count(&self, lo: u64, hi: u64, cs: &[RCond], s: Snap) -> u64 {
        if lo > hi {
            return 0;
        }
        let top = hi.min(self.spec.rows);
        let mut n = if lo > top {
            0
        } else {
            match cs {
                [] => top - lo + 1,
                [one] => self.count_col(one, lo, top),
                _ => {
                    let (mut k, mut c) = (lo, 0);
                    while let Some(x) = self.next_base(k, top, cs) {
                        c += 1;
                        k = x + 1;
                    }
                    c
                }
            }
        };
        for (k, _) in self.rows.range(lo..=hi) {
            let base = *k <= self.spec.rows && self.holds_all(*k, None, cs);
            let real = match self.seen(*k, s) {
                Ok(v) => self.holds_all(*k, v, cs),
                Err(()) => false,
            };
            if base && !real {
                n -= 1;
            } else if real && !base {
                n += 1;
            }
        }
        n
    }
    /// rows in key order: from `lo` to `hi`, up to `want`
    pub fn by_key(&self, lo: u64, hi: u64, cs: &[RCond], want: u64, desc: bool, s: Snap) -> Vec<u64> {
        let mut out = Vec::new();
        if want == 0 || lo > hi {
            return out;
        }
        if desc {
            // (one by one: what it looks at it pays for)
            let mut k = hi.min(self.next_key.max(self.spec.rows));
            while k >= lo.max(1) && (out.len() as u64) < want {
                if let Ok(v) = self.seen(k, s)
                    && self.holds_all(k, v, cs)
                {
                    out.push(k);
                }
                k -= 1;
            }
            return out;
        }
        let mut k = lo.max(1);
        while k <= hi {
            let b = self.next_base(k, hi, cs);
            let t = if self.rows.is_empty() { None } else { self.rows.range(k..=hi).next().map(|x| *x.0) };
            let Some(c) = (match (b, t) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, None) => x,
                (None, y) => y,
            }) else {
                break;
            };
            let ok = if t == Some(c) {
                match self.seen(c, s) {
                    Ok(v) => self.holds_all(c, v, cs),
                    Err(()) => false,
                }
            } else {
                true
            };
            if ok {
                out.push(c);
                if out.len() as u64 >= want {
                    break;
                }
            }
            k = c + 1;
        }
        out
    }
    /// rows of an indexed value: in the order of the values, then the keys, up to `want`; the rows it
    /// looked at
    pub fn by_index(&self, c: &RCond, rest: &[RCond], want: u64, desc: bool, s: Snap) -> (Vec<u64>, u64) {
        let (col, a, b) = match c {
            RCond::Int { col, lo, hi } => (*col, *lo, *hi),
            RCond::Text { col, derived: Some(n), .. } => (*col, *n, *n),
            RCond::Text { col, .. } => (*col, 0, -1),
        };
        let d = self.spec.cols[col - 1].distinct.max(1);
        let (a, b) = (a.max(0), b.min((d - 1).min(i64::MAX as u64) as i64));
        let mut found: Vec<(Val, u64)> = Vec::new();
        let mut looked = 0u64;
        let off = self.spec.cols[col - 1].off;
        let rows = self.spec.rows;
        let mut one = |v: i64, found: &mut Vec<(Val, u64)>| {
            // the keys of a value: the first at key - 1 = v - off (mod d), then every d
            let k0 = ((v as u128 + d as u128 - off as u128 % d as u128) % d as u128) as u64;
            let mut k = k0 + 1;
            while k <= rows && (found.len() as u64) < want {
                looked += 1;
                let ok = match self.seen(k, s) {
                    Ok(ver) => self.holds(k, ver, c) && self.holds_all(k, ver, rest),
                    Err(()) => false,
                };
                if ok {
                    found.push((self.derived(col, k), k));
                }
                k = k.saturating_add(d);
            }
        };
        if a <= b {
            if desc {
                let mut v = b;
                while v >= a && (found.len() as u64) < want {
                    one(v, &mut found);
                    v -= 1;
                }
            } else {
                let mut v = a;
                while v <= b && (found.len() as u64) < want {
                    one(v, &mut found);
                    v += 1;
                }
            }
        }
        // rows whose value is not the derived one
        let (from, to) = match c {
            RCond::Int { lo, hi, .. } => ((col as u16, Val::Int(*lo)), (col as u16, Val::Int(*hi))),
            RCond::Text { val, .. } => ((col as u16, Val::Text(val.clone())), (col as u16, Val::Text(val.clone()))),
        };
        if from <= to {
            for (key, set) in self.moved.range(from..=to) {
                for k in set {
                    if found.iter().any(|x| x.1 == *k) {
                        continue;
                    }
                    looked += 1;
                    if let Ok(ver) = self.seen(*k, s)
                        && self.holds(*k, ver, c)
                        && self.holds_all(*k, ver, rest)
                    {
                        found.push((key.1.clone(), *k));
                    }
                }
            }
        }
        found.sort();
        if desc {
            found.reverse();
        }
        found.truncate(want.min(usize::MAX as u64) as usize);
        (found.into_iter().map(|x| x.1).collect(), looked)
    }
    /// a new version of a row, open until its transaction ends; `gone`: a deletion
    pub fn write(&mut self, key: u64, txn: u64, lsn: u64, sets: Vec<(u16, Val)>, gone: bool) {
        let cur: Vec<(u16, Val)> = match self.newest(key, txn) {
            Ok(Some(v)) => v.vals.clone(),
            _ => Vec::new(),
        };
        let mut vals = cur;
        for (c, v) in sets {
            // an indexed column that leaves its derived value is found through `moved`
            if self.indexes.contains(&(c as usize)) {
                self.moved.entry((c, v.clone())).or_default().insert(key);
            }
            match vals.iter_mut().find(|x| x.0 == c) {
                Some(x) => x.1 = v,
                None => vals.push((c, v)),
            }
        }
        let r = self.rows.entry(key).or_default();
        r.refd = true;
        // a second write of its own replaces the first
        match r.vers.last_mut() {
            Some(l) if l.ts == 0 && l.txn == txn => {
                l.vals = vals;
                l.deleted = gone;
                l.lsn = lsn;
            }
            _ => r.vers.push(Version { ts: 0, txn, lsn, deleted: gone, vals }),
        }
    }
    /// an index came: the rows that already left their derived value are found through `moved`
    pub fn index_added(&mut self, c: usize) {
        let keys: Vec<(u64, Val)> = self.rows.iter().filter_map(|(k, r)| r.vers.iter().rev().find_map(|v| v.vals.iter().find(|x| x.0 as usize == c)).map(|x| (*k, x.1.clone()))).collect();
        for (k, v) in keys {
            self.moved.entry((c as u16, v)).or_default().insert(k);
        }
    }
    /// its open version commits with this stamp
    pub fn commit_row(&mut self, key: u64, txn: u64, ts: u64) {
        if let Some(v) = self.rows.get_mut(&key).and_then(|r| r.vers.last_mut())
            && v.ts == 0
            && v.txn == txn
        {
            v.ts = ts;
        }
    }
    /// its open version goes; a row left without versions is untouched again
    pub fn undo_row(&mut self, key: u64, txn: u64) {
        let Some(r) = self.rows.get_mut(&key) else { return };
        if r.vers.last().is_some_and(|v| v.ts == 0 && v.txn == txn) {
            r.vers.pop();
        }
        if r.vers.is_empty() {
            self.rows.remove(&key);
        }
    }
    /// versions no reader needs go: `horizon` the oldest snapshot still open, `durable` what a crash keeps
    pub fn prune_row(&mut self, key: u64, horizon: u64, durable: u64) -> usize {
        let Some(r) = self.rows.get_mut(&key) else { return 0 };
        // (a hot row between flushes of "relaxed": nothing can go yet, and no walk to find that out)
        if r.vers.len() < 2 || r.vers[r.vers.len() - 2].lsn > durable {
            return 0;
        }
        // the newest version every reader sees and a crash keeps: what is older than it goes
        let Some(i) = r.vers.iter().rposition(|v| v.ts != 0 && v.ts <= horizon && v.lsn <= durable) else {
            return 0;
        };
        if i == 0 {
            return 0;
        }
        r.vers.drain(..i);
        i
    }
    /// a crash: what the log did not keep goes, and so does what was open
    pub fn crash_row(&mut self, key: u64, durable: u64) -> u64 {
        let Some(r) = self.rows.get_mut(&key) else { return 0 };
        let before = r.vers.len() as u64;
        r.vers.retain(|v| v.ts != 0 && v.lsn <= durable);
        let gone = before - r.vers.len() as u64;
        if r.vers.is_empty() {
            self.rows.remove(&key);
        }
        gone
    }
    /// past the bound: the next clean row goes back to its derived values; false when none can go now
    pub fn forget_one(&mut self, horizon: u64, durable: u64, locked: &dyn Fn(u16, u64) -> bool) -> bool {
        let no = self.no;
        // (a hand over the rows: its first round clears the marks)
        for _ in 0..1024 {
            let Some(k) = self.rows.range(self.hand..).next().map(|x| *x.0).or_else(|| self.rows.keys().next().copied()) else {
                return false;
            };
            self.hand = k + 1;
            let Some(r) = self.rows.get_mut(&k) else { continue };
            if r.refd {
                r.refd = false;
                continue;
            }
            if r.vers.len() == 1 && r.vers[0].ts != 0 && r.vers[0].ts <= horizon && r.vers[0].lsn <= durable && !locked(no, k) {
                self.rows.remove(&k);
                return true;
            }
        }
        false
    }
    /// the bytes its touched rows hold (Service::footprint)
    pub fn bytes(&self) -> usize {
        let mut b = core::mem::size_of::<Table>() + self.zipf.as_ref().map_or(0, |z| z.bytes() / Arc::strong_count(z).max(1));
        for r in self.rows.values() {
            b += 48 + r.vers.iter().map(|v| 48 + v.vals.iter().map(|x| 32 + if let Val::Text(t) = &x.1 { t.len() } else { 0 }).sum::<usize>()).sum::<usize>();
        }
        b + self.moved.values().map(|s| 64 + s.len() * 16).sum::<usize>()
    }
    /// the bytes of its heap and its indexes on the disk
    pub fn disk_bytes(&self, l: &Layout) -> u64 {
        let mut pages = l.heap_pages(self.spec.rows, self.spec.row_bytes) + l.tree_pages(self.spec.rows);
        for _ in &self.indexes {
            pages += l.tree_pages(self.spec.rows);
        }
        pages * l.page
    }
}

// ---------------------------------------------------------------- the plan

/// which way a statement goes
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// the primary index, keys from .. to
    Key { lo: u64, hi: u64 },
    /// another index over this condition
    Index { cond: RCond },
    /// every page of the heap
    Scan,
}

/// what a statement's way touches
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// levels of a tree above the leaves (one path)
    pub upper: u64,
    pub leaves: u64,
    pub heap: u64,
    pub examined: u64,
    /// the heap pages are neighbours (a key range, a scan), not scattered
    pub seq: bool,
}

impl Cost {
    pub fn pages(&self) -> u64 {
        self.upper + self.leaves + self.heap
    }
}

/// what a pool's walk gave
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touch {
    pub hits: u64,
    /// misses of scattered pages, of neighbours (read together)
    pub miss_rand: u64,
    pub miss_seq: u64,
    pub heap_hits: u64,
    pub heap_miss: u64,
}

impl Touch {
    pub fn misses(&self) -> u64 {
        self.miss_rand + self.miss_seq
    }
}

/// the way of a statement: the best of the primary index, an index and the scan by the rows they touch; an
/// ORDER BY needs its index
pub fn best_access(t: &Table, cs: &[RCond], order: Option<usize>) -> (Access, &'static str) {
    let (mut lo, mut hi) = (1u64, u64::MAX);
    let mut on_key = false;
    for c in cs {
        if let RCond::Int { col: 0, lo: a, hi: b } = c {
            on_key = true;
            lo = lo.max((*a).max(0) as u64);
            hi = hi.min((*b).max(0) as u64);
        }
    }
    let rows = t.spec.rows.max(1);
    if order == Some(0) || (on_key && order.is_none()) {
        if on_key {
            return (Access::Key { lo, hi }, if lo == hi { "Gleichheit auf dem Schlüssel: der Baum führt zur Zeile." } else { "Bereich auf dem Schlüssel: der Baum führt zum Anfang, dann der Reihe nach." });
        }
        return (Access::Key { lo: 1, hi: u64::MAX }, "ORDER BY über den Schlüssel: die Zeilen liegen in seiner Reihenfolge.");
    }
    let mut best: Option<(u64, &RCond)> = None;
    for c in cs {
        if !t.indexes.contains(&c.col()) || order.is_some_and(|o| o != c.col()) {
            continue;
        }
        let n = match c {
            RCond::Int { col, lo, hi } => {
                let d = t.spec.cols[*col - 1].distinct.max(1);
                let (a, b) = ((*lo).max(0) as u64, (*hi).max(-1).min((d - 1) as i64));
                if b < 0 || a > b as u64 { 0 } else { (b as u64 - a + 1).saturating_mul(rows.div_ceil(d)) }
            }
            RCond::Text { col, .. } => rows.div_ceil(t.spec.cols[*col - 1].distinct.max(1)),
        };
        if best.is_none_or(|b| n < b.0) {
            best = Some((n, c));
        }
    }
    if let Some((_, c)) = best {
        return (Access::Index { cond: c.clone() }, "Es gibt einen Index auf dieser Spalte: Baum, dann die Zeilen.");
    }
    if let Some(o) = order
        && t.indexes.contains(&o)
    {
        let cond = if t.spec.is_text(o) { RCond::Int { col: o, lo: 0, hi: i64::MAX } } else { RCond::Int { col: o, lo: i64::MIN, hi: i64::MAX } };
        return (Access::Index { cond }, "ORDER BY über diesen Index: er liefert die Zeilen in seiner Reihenfolge.");
    }
    (Access::Scan, if cs.is_empty() { "Keine Bedingung: alle Seiten der Tabelle." } else { "Auf diesen Spalten liegt kein Index: alle Seiten der Tabelle." })
}

/// what a way touches when it looked at so many rows and found so many
pub fn cost_of(t: &Table, l: &Layout, a: &Access, examined: u64, found: u64) -> Cost {
    let rb = t.spec.row_bytes;
    let rows = t.spec.rows.max(t.next_key);
    match a {
        Access::Key { .. } => Cost { upper: l.depth(rows) - 1, leaves: examined.max(1).div_ceil(l.fanout), heap: if found == 0 { 0 } else { l.heap_pages(examined.max(1), rb) }, examined, seq: true },
        Access::Index { .. } => Cost { upper: l.depth(rows) - 1, leaves: examined.max(1).div_ceil(l.fanout), heap: examined.min(l.heap_pages(rows, rb)), examined, seq: false },
        Access::Scan => Cost { upper: 0, leaves: 0, heap: l.heap_pages(examined, rb), examined, seq: true },
    }
}

/// a page's name in the pool: table, object (0 the heap, 1 the primary tree, then the other trees), level
fn page_key(table: u16, obj: u16, level: u16, page: u64) -> u64 {
    ((table as u64 & 0xFFF) << 51) | ((obj as u64 & 0x3F) << 45) | ((level as u64 & 0xF) << 41) | (page & 0x1FF_FFFF_FFFF)
}

/// a statement's way through the pool
pub fn walk(p: &mut Pool, t: &Table, l: &Layout, a: &Access, c: &Cost, keys: &[u64], write: bool) -> Touch {
    let mut o = Touch::default();
    let tn = t.no;
    let rpp = l.rows_per_page(t.spec.row_bytes);
    let (obj, first_leaf): (u16, u64) = match a {
        Access::Key { .. } => (1, keys.first().map_or(0, |k| (*k - 1) / l.fanout)),
        Access::Index { cond } => {
            let slot = t.indexes.iter().position(|x| *x == cond.col()).unwrap_or(0) as u16 + 2;
            let d = t.spec.cols[cond.col() - 1].distinct.max(1);
            let at = keys.first().map_or(0, |k| ((t.base_int(cond.col(), *k) as u64) * (t.spec.rows / d).max(1)) / l.fanout);
            (slot, at)
        }
        Access::Scan => (0, 0),
    };
    // the upper levels of its tree
    let mut up = first_leaf;
    for lvl in 1..=c.upper {
        up /= l.fanout;
        if p.upper(page_key(tn, obj, lvl as u16, up)) {
            o.hits += 1;
        } else {
            o.miss_rand += 1;
        }
    }
    // its leaves, neighbours
    for i in 0..c.leaves {
        if i >= TOUCH_MAX {
            let left = c.leaves - i;
            o.hits += left / 2;
            o.miss_seq += left - left / 2;
            break;
        }
        let leaf = page_key(tn, obj, 0, first_leaf + i);
        let hit = if p.rate > 1 && l.leaves(t.spec.rows.max(t.next_key)) <= SMALL_TREE { p.upper(leaf) } else { p.access(leaf, false) };
        if hit {
            o.hits += 1;
        } else if i == 0 {
            o.miss_rand += 1;
        } else {
            o.miss_seq += 1;
        }
    }
    let heap_of = |k: u64| (k - 1) / rpp;
    let heap = |p: &mut Pool, pg: u64, o: &mut Touch, seq: bool| {
        if p.access(page_key(tn, 0, 0, pg), write) {
            o.hits += 1;
            o.heap_hits += 1;
        } else {
            o.heap_miss += 1;
            if seq {
                o.miss_seq += 1;
            } else {
                o.miss_rand += 1;
            }
        }
    };
    match a {
        Access::Key { .. } => {
            let first = keys.first().map_or(0, |k| heap_of(*k));
            for i in 0..c.heap.min(TOUCH_MAX) {
                heap(p, first + i, &mut o, i > 0);
            }
            share(p, t, c.heap.saturating_sub(TOUCH_MAX), &mut o);
        }
        Access::Index { .. } => {
            let mut done = 0;
            let mut last = u64::MAX;
            for k in keys.iter().take(TOUCH_MAX as usize) {
                let pg = heap_of(*k);
                if pg != last {
                    heap(p, pg, &mut o, false);
                    done += 1;
                    last = pg;
                }
            }
            share(p, t, c.heap.saturating_sub(done), &mut o);
        }
        Access::Scan => {
            let pages = l.heap_pages(t.spec.rows.max(t.next_key), t.spec.row_bytes);
            // a big table goes through a small ring: it pushes nothing out
            if pages > p.cap() / 4 || c.heap > TOUCH_MAX * 8 {
                share(p, t, c.heap, &mut o);
            } else {
                for pg in 0..c.heap {
                    heap(p, pg, &mut o, pg > 0);
                }
            }
        }
    }
    o
}

// the rest of a long walk: the share of the table in the pool decides, without a step per page
fn share(p: &Pool, t: &Table, n: u64, o: &mut Touch) {
    if n == 0 {
        return;
    }
    let held = p.held_of(t.no);
    let total = p.heap_pages_of(t.no).max(1);
    let hits = ((n as u128 * held.min(total) as u128) / total as u128) as u64;
    o.hits += hits;
    o.heap_hits += hits;
    o.miss_seq += n - hits;
    o.heap_miss += n - hits;
}

// ---------------------------------------------------------------- the pool

const NIL: u32 = u32::MAX;
/// the mark of a dirty page in a slot's key
const DIRTY: u64 = 1 << 63;

/// the buffer pool: see the head of the file
#[derive(Clone, Debug)]
pub struct Pool {
    /// its size in pages
    cap: u64,
    /// one page in this many is tracked
    pub rate: u64,
    /// tracked pages at most
    slots: usize,
    keys: Vec<u64>,
    prev: Vec<u32>,
    next: Vec<u32>,
    head: u32,
    tail: u32,
    /// open addressing: slot + 1, 0 free
    table: Vec<u32>,
    /// the upper levels of trees it holds
    uppers: BTreeSet<u64>,
    /// tracked heap pages by table
    held: BTreeMap<u16, u64>,
    /// heap pages of the tables, for the share of a long walk
    heaps: BTreeMap<u16, u64>,
    /// parts per million of hits of the tracked pages lately, the accumulators for the others: by class
    /// (0 leaves of trees, 1 heap pages)
    rate_ppm: [u32; 2],
    acc: [u32; 2],
    dirty: u64,
    /// dirty pages it pushed out and did not write yet (tracked ones; the caller writes them)
    pub evicted_dirty: u64,
    pub hits: u64,
    pub misses: u64,
}

impl Pool {
    pub fn new(cap_pages: u64) -> Pool {
        let cap = cap_pages.max(1);
        let rate = cap.div_ceil(SLOTS).max(1);
        Pool { cap, rate, slots: (cap / rate).max(1) as usize, keys: Vec::new(), prev: Vec::new(), next: Vec::new(), head: NIL, tail: NIL, table: alloc::vec![0; 64], uppers: BTreeSet::new(), held: BTreeMap::new(), heaps: BTreeMap::new(), rate_ppm: [0; 2], acc: [0; 2], dirty: 0, evicted_dirty: 0, hits: 0, misses: 0 }
    }
    pub fn cap(&self) -> u64 {
        self.cap
    }
    /// the tables' heap sizes
    pub fn set_heap(&mut self, table: u16, pages: u64) {
        self.heaps.insert(table, pages);
    }
    pub fn heap_pages_of(&self, table: u16) -> u64 {
        self.heaps.get(&table).copied().unwrap_or(0)
    }
    /// pages of a table's heap it holds
    pub fn held_of(&self, table: u16) -> u64 {
        self.held.get(&table).copied().unwrap_or(0) * self.rate
    }
    /// the pages it holds (in steps of `rate`)
    pub fn used(&self) -> u64 {
        (self.keys.len() as u64 * self.rate + self.uppers.len() as u64).min(self.cap)
    }
    /// its dirty pages (in steps of `rate`)
    pub fn dirty_pages(&self) -> u64 {
        self.dirty * self.rate
    }
    /// it starts empty (a restart, a kill)
    pub fn clear(&mut self) {
        let heaps = core::mem::take(&mut self.heaps);
        *self = Pool::new(self.cap);
        self.heaps = heaps;
    }
    /// another size in the running system: what fits stays
    pub fn resize(&mut self, cap_pages: u64) {
        let old: Vec<u64> = self.order_oldest_first();
        let uppers = core::mem::take(&mut self.uppers);
        let (heaps, hits, misses, rate_ppm) = (core::mem::take(&mut self.heaps), self.hits, self.misses, self.rate_ppm);
        *self = Pool::new(cap_pages);
        (self.heaps, self.hits, self.misses, self.rate_ppm, self.uppers) = (heaps, hits, misses, rate_ppm, uppers);
        for k in old {
            let id = k & !DIRTY;
            if self.tracked(id) && self.find(id).is_none() {
                self.insert(k);
            }
        }
    }
    fn order_oldest_first(&self) -> Vec<u64> {
        let mut v = Vec::with_capacity(self.keys.len());
        let mut i = self.tail;
        while i != NIL {
            v.push(self.keys[i as usize]);
            i = self.prev[i as usize];
        }
        v
    }
    fn tracked(&self, id: u64) -> bool {
        self.rate == 1 || mix(id) % self.rate == 0
    }
    fn bucket(&self, id: u64) -> usize {
        (mix(id ^ 0x5bd1_e995) as usize) & (self.table.len() - 1)
    }
    fn find(&self, id: u64) -> Option<u32> {
        let mut b = self.bucket(id);
        loop {
            let s = self.table[b];
            if s == 0 {
                return None;
            }
            if self.keys[s as usize - 1] & !DIRTY == id {
                return Some(s - 1);
            }
            b = (b + 1) & (self.table.len() - 1);
        }
    }
    fn table_put(&mut self, id: u64, slot: u32) {
        let mut b = self.bucket(id);
        while self.table[b] != 0 {
            b = (b + 1) & (self.table.len() - 1);
        }
        self.table[b] = slot + 1;
    }
    fn table_del(&mut self, id: u64) {
        let mask = self.table.len() - 1;
        let mut b = self.bucket(id);
        loop {
            let s = self.table[b];
            if s == 0 {
                return;
            }
            if self.keys[s as usize - 1] & !DIRTY == id {
                break;
            }
            b = (b + 1) & mask;
        }
        // backward shift, so that no marks of deleted entries are needed
        let mut hole = b;
        let mut i = (b + 1) & mask;
        loop {
            let s = self.table[i];
            if s == 0 {
                break;
            }
            let home = self.bucket(self.keys[s as usize - 1] & !DIRTY);
            let between = if hole <= i { hole < home && home <= i } else { hole < home || home <= i };
            if !between {
                self.table[hole] = s;
                hole = i;
            }
            i = (i + 1) & mask;
        }
        self.table[hole] = 0;
    }
    fn unlink(&mut self, s: u32) {
        let (p, n) = (self.prev[s as usize], self.next[s as usize]);
        if p != NIL {
            self.next[p as usize] = n;
        } else {
            self.head = n;
        }
        if n != NIL {
            self.prev[n as usize] = p;
        } else {
            self.tail = p;
        }
    }
    fn push_front(&mut self, s: u32) {
        self.prev[s as usize] = NIL;
        self.next[s as usize] = self.head;
        if self.head != NIL {
            self.prev[self.head as usize] = s;
        }
        self.head = s;
        if self.tail == NIL {
            self.tail = s;
        }
    }
    fn heap_table_of(id: u64) -> Option<u16> {
        // (object 0: a heap page)
        if (id >> 45) & 0x3F == 0 { Some(((id >> 51) & 0xFFF) as u16) } else { None }
    }
    /// takes a tracked page in, the oldest out when it is full
    fn insert(&mut self, key: u64) {
        let id = key & !DIRTY;
        if self.keys.len() >= self.slots {
            let s = self.tail;
            let old = self.keys[s as usize];
            self.table_del(old & !DIRTY);
            self.unlink(s);
            if old & DIRTY != 0 {
                self.dirty -= 1;
                self.evicted_dirty += 1;
            }
            if let Some(t) = Pool::heap_table_of(old & !DIRTY)
                && let Some(n) = self.held.get_mut(&t)
            {
                *n = n.saturating_sub(1);
            }
            self.keys[s as usize] = key;
            self.push_front(s);
            self.table_put(id, s);
        } else {
            let s = self.keys.len() as u32;
            self.keys.push(key);
            self.prev.push(NIL);
            self.next.push(NIL);
            self.push_front(s);
            // the table stays at most half full
            if self.keys.len() * 2 > self.table.len() {
                let size = self.table.len() * 2;
                self.table = alloc::vec![0; size];
                for i in 0..self.keys.len() {
                    let x = self.keys[i] & !DIRTY;
                    self.table_put(x, i as u32);
                }
            } else {
                self.table_put(id, s);
            }
        }
        if key & DIRTY != 0 {
            self.dirty += 1;
        }
        if let Some(t) = Pool::heap_table_of(id) {
            *self.held.entry(t).or_insert(0) += 1;
        }
    }
    /// a page is used: true when it was there; a tracked miss takes it in
    pub fn access(&mut self, id: u64, write: bool) -> bool {
        let class = Pool::heap_table_of(id).is_some() as usize;
        let hit = if !self.tracked(id) {
            // what the tracked pages of its class have had lately
            self.acc[class] += self.rate_ppm[class];
            if self.acc[class] >= 1_000_000 {
                self.acc[class] -= 1_000_000;
                true
            } else {
                false
            }
        } else {
            let hit = match self.find(id) {
                Some(s) => {
                    self.unlink(s);
                    self.push_front(s);
                    if write && self.keys[s as usize] & DIRTY == 0 {
                        self.keys[s as usize] |= DIRTY;
                        self.dirty += 1;
                    }
                    true
                }
                None => {
                    self.insert(if write { id | DIRTY } else { id });
                    false
                }
            };
            // the mean of about the last thousand
            self.rate_ppm[class] = ((self.rate_ppm[class] as u64 * 1023 + if hit { 1_000_000 } else { 0 }) / 1024) as u32;
            hit
        };
        if hit {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        hit
    }
    /// an upper level of a tree is used: it stays once it is read
    pub fn upper(&mut self, id: u64) -> bool {
        let hit = !self.uppers.insert(id);
        if hit {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        hit
    }
    /// a checkpoint wrote what was dirty
    pub fn clean_all(&mut self) {
        for k in self.keys.iter_mut() {
            *k &= !DIRTY;
        }
        self.dirty = 0;
    }
    /// the bytes of its structure (Service::footprint)
    pub fn bytes(&self) -> usize {
        self.keys.len() * 16 + self.table.len() * 4 + self.uppers.len() * 16 + core::mem::size_of::<Pool>()
    }
}
