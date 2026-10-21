// modular -- JPEG XL modular image decoding (modular/encoding/*.cc; model: tools/jxl_ref.py
// WPHeader .. decode_channel)                                            [src/img/jxl_mod.s]
//
//   Mi / Mc / Mt               modular image, channel, transform (MI, MC, MT)
//   mi.insert(pos, w, h, hs, vs) / mi.delete(pos, n) / mi.resize(c, w, h)
//   jm_wphdr(jb) -> [u32; 11]  weighted predictor header
//   jm_tree(jb, limit) -> Ma   MA tree
//   jm_decode(jb, mi, stream, maxsize, gtree, gcode, undo, hdr) -> 0 / ERR_*
//                              ModularGenericDecompress: group header, transforms (meta
//                              applied), channels (those of at most maxsize samples per side
//                              after the meta channels), inverse transforms if undo
//   Wp                         weighted predictor state (wp_new, wp_predict, wp_update)
//   jm_predict(pred, nb, wpp)  PredictOne (palette deltas)
//
// Channel samples are i32 rows of w.  Arithmetic follows libjxl: properties in 32 bits
// (wrapping), predictions in 64 bits, the sample = wrap32(UnpackSigned(token) * multiplier +
// guess).

use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_HDR, ERR_LIMIT, ERR_NOMEM};

use crate::bits::Jb;
use crate::ent::{je_read, Jc, Jr};
use crate::frm::Jmh;
use crate::hdr::unpack_signed;

pub const JM_MAXPIX: u64 = 0x7FFF_FFFF;
pub const JM_MAXCH: u32 = 1 << 16;
pub const MTR_RCT: u32 = 0;
pub const MTR_PALETTE: u32 = 1;
pub const MTR_SQUEEZE: u32 = 2;

/// default weighted predictor header (p1C, p2C, p3Ca .. p3Ce, w[4])
pub const WH_DEF: [u32; 11] = [16, 10, 7, 7, 7, 0, 0, 13, 12, 12, 12];

static D_0123: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 0];
static D_BEGIN: [u32; 8] = [0, 3, 8, 6, 72, 10, 1096, 13];
static D_RCT: [u32; 8] = [6, 0, 0, 2, 2, 4, 10, 6];
static D_NUMC: [u32; 8] = [1, 0, 3, 0, 4, 0, 1, 13];
static D_NBCOL: [u32; 8] = [0, 8, 256, 10, 1280, 12, 5376, 16];
static D_NBDELTA: [u32; 8] = [0, 0, 1, 8, 257, 10, 1281, 16];
static D_NSQ: [u32; 8] = [0, 0, 1, 4, 9, 6, 41, 8];
static D_SQNC: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 4];
static D_NT: [u32; 8] = [0, 0, 1, 0, 2, 4, 18, 8];

pub static DIVLOOKUP: [u32; 64] = [
    16777216, 8388608, 5592405, 4194304, 3355443, 2796202, 2396745, 2097152, 1864135, 1677721, 1525201, 1398101,
    1290555, 1198372, 1118481, 1048576, 986895, 932067, 883011, 838860, 798915, 762600, 729444, 699050, 671088,
    645277, 621378, 599186, 578524, 559240, 541200, 524288, 508400, 493447, 479349, 466033, 453438, 441505, 430185,
    419430, 409200, 399457, 390167, 381300, 372827, 364722, 356962, 349525, 342392, 335544, 328965, 322638, 316551,
    310689, 305040, 299593, 294337, 289262, 284359, 279620, 275036, 270600, 266305, 262144,
];

/// a modular channel (MC)
#[derive(Clone, Default)]
pub struct Mc {
    pub w: u32,
    pub h: u32,
    pub hs: i32, // -1: palette / meta channel
    pub vs: i32,
    pub d: Vec<i32>, // rows of w (empty when w * h = 0)
}

impl Mc {
    /// the record part compared by jm_checkeq (w, h, hs, vs)
    #[inline]
    pub fn same(&self, o: &Mc) -> bool {
        self.w == o.w && self.h == o.h && self.hs == o.hs && self.vs == o.vs
    }
}

/// a modular transform (MT)
#[derive(Clone, Default)]
pub struct Mt {
    pub id: u32,
    pub begin: u32,
    pub rct: u32,
    pub numc: u32,
    pub nbcol: u32,
    pub nbdelta: u32,
    pub pred: u32,
    pub sq: Vec<[u32; 4]>, // per step: horizontal, in_place, begin_c, num_c
}

/// a modular image (MI)
#[derive(Default)]
pub struct Mi {
    pub w: u32,
    pub h: u32,
    pub bd: u32,
    pub nbmeta: u32,
    pub ch: Vec<Mc>,
    pub tr: Vec<Mt>,
}

/// a zeroed vector of n elements, ERR_NOMEM when it cannot be allocated
pub fn zvec<T: Clone + Default>(n: usize) -> Result<Vec<T>, i32> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(ERR_NOMEM);
    }
    if n.saturating_mul(core::mem::size_of::<T>()) >= 1 << 16 {
        // jxl-speed: the size can be allocated; large blocks come from the zeroed allocation
        // (fresh pages from the OS), so they are not cleared here in one serial pass (the
        // threads that fill them touch the pages)
        drop(v);
        return Ok(alloc::vec![T::default(); n]);
    }
    v.resize(n, T::default());
    Ok(v)
}

impl Mi {
    /// mi_new
    pub fn new(w: u32, h: u32, bd: u32) -> Mi {
        Mi { w, h, bd, nbmeta: 0, ch: Vec::with_capacity(8), tr: Vec::new() }
    }

    pub fn nch(&self) -> u32 {
        self.ch.len() as u32
    }

    /// mi_insert: new zeroed channel at pos
    pub fn insert(&mut self, pos: u32, w: u32, h: u32, hs: i32, vs: i32) -> i32 {
        if self.nch() >= JM_MAXCH {
            return ERR_LIMIT;
        }
        let pos = (pos as usize).min(self.ch.len());
        self.ch.insert(pos, Mc { w, h, hs, vs, d: Vec::new() });
        let n = w as u64 * h as u64;
        if n == 0 {
            return 0;
        }
        if n > JM_MAXPIX {
            return ERR_LIMIT;
        }
        match zvec::<i32>(n as usize) {
            Ok(v) => {
                self.ch[pos].d = v;
                0
            }
            Err(e) => e,
        }
    }

    /// mi_delete: remove n channels at pos
    pub fn delete(&mut self, pos: u32, n: u32) {
        let a = (pos as usize).min(self.ch.len());
        let b = (pos as usize + n as usize).min(self.ch.len());
        self.ch.drain(a..b);
    }

    /// mi_resize: new zeroed samples of w x h for channel c
    pub fn resize(&mut self, c: u32, w: u32, h: u32) -> i32 {
        let ch = &mut self.ch[c as usize];
        ch.d = Vec::new();
        ch.w = w;
        ch.h = h;
        let n = w as u64 * h as u64;
        if n == 0 {
            return 0;
        }
        if n > JM_MAXPIX {
            return ERR_LIMIT;
        }
        match zvec::<i32>(n as usize) {
            Ok(v) => {
                ch.d = v;
                0
            }
            Err(e) => e,
        }
    }

    /// jm_checkeq: channels c1..c2 exist, do not mix meta channels and have equal sizes and
    /// shifts
    pub fn checkeq(&self, c1: u32, c2: u32) -> i32 {
        let n = self.nch();
        if c1 > n || c2 >= n || c2 < c1 {
            return ERR_HDR;
        }
        if c1 < self.nbmeta && self.nbmeta <= c2 {
            return ERR_HDR;
        }
        let f = &self.ch[c1 as usize];
        for k in c1 + 1..=c2 {
            if !self.ch[k as usize].same(f) {
                return ERR_HDR;
            }
        }
        0
    }
}

/// jm_wphdr: weighted predictor header
pub fn jm_wphdr(jb: &mut Jb) -> [u32; 11] {
    let mut w = WH_DEF;
    if jb.f(1) != 0 {
        return w;
    }
    for v in w.iter_mut().take(7) {
        *v = jb.f(5);
    }
    for v in w.iter_mut().skip(7) {
        *v = jb.f(4);
    }
    w
}

/// an MA tree node (MN)
#[derive(Clone, Copy, Default)]
pub struct Mn {
    pub prop: i32,  // property, -1 = leaf
    pub split: i32, // split value
    pub l: u32,     // child taken when property > split; leaf: context
    pub r: u32,
    pub pred: u32, // leaf: predictor
    pub off: i32,  // leaf: offset
    pub mul: u32,  // leaf: multiplier
}

/// an MA tree (MA); empty nodes = none
#[derive(Default)]
pub struct Ma {
    pub maxprop: i32,
    pub wp: u32,
    pub nleaf: u32,
    pub nodes: Vec<Mn>,
}

/// jm_tree: MA tree with at most limit nodes
pub fn jm_tree(jb: &mut Jb, limit: u64) -> Result<Ma, i32> {
    let jc = je_read(jb, 6, false)?;
    let mut jr = Jr::new(&jc, jb, 0)?;
    let mut nodes: Vec<Mn> = Vec::with_capacity(64);
    let mut leaves = 0u32;
    let mut todo = 1u32;
    while todo != 0 {
        if nodes.len() as u64 > limit {
            return Err(ERR_CODE);
        }
        todo -= 1;
        let n = nodes.len() as u32;
        let v = jr.read(jb, 1);
        if v > 256 {
            return Err(ERR_CODE);
        }
        let prop = v as i32 - 1;
        if prop < 0 {
            let pred = jr.read(jb, 2);
            if pred >= 14 {
                return Err(ERR_CODE);
            }
            let off = unpack_signed(jr.read(jb, 3));
            let ml = jr.read(jb, 4);
            if ml >= 31 {
                return Err(ERR_CODE);
            }
            let mb = jr.read(jb, 5);
            if mb >= (1u32 << (31 - ml)).wrapping_sub(1) {
                return Err(ERR_CODE);
            }
            nodes.push(Mn { prop: -1, split: 0, l: leaves, r: 0, pred, off, mul: (mb + 1) << ml });
            leaves += 1;
        } else {
            let split = unpack_signed(jr.read(jb, 0));
            let l = n.wrapping_add(todo).wrapping_add(1);
            nodes.push(Mn { prop, split, l, r: l.wrapping_add(1), pred: 0, off: 0, mul: 1 });
            todo += 2;
        }
    }
    if !jr.fin() {
        return Err(ERR_CODE);
    }
    if jb.err != 0 {
        return Err(jb.err);
    }
    let mut maxprop = -1i32;
    let mut wp = 0u32;
    for nd in &nodes {
        if nd.prop > maxprop {
            maxprop = nd.prop;
        }
        if nd.prop == 15 || (nd.prop == -1 && nd.pred == 6) {
            wp = 1;
        }
    }
    Ok(Ma { maxprop, wp, nleaf: leaves, nodes })
}

/// jm_tread: Transform header
fn jm_tread(jb: &mut Jb, mt: &mut Mt) -> i32 {
    let id = jb.u32f(&D_0123);
    if id == 3 {
        return ERR_HDR;
    }
    mt.id = id;
    mt.rct = 6;
    mt.numc = 3;
    mt.nbcol = 256;
    if id == MTR_SQUEEZE {
        let nsq = jb.u32f(&D_NSQ);
        if nsq != 0 {
            let mut sq = Vec::with_capacity(nsq as usize);
            for _ in 0..nsq {
                let h = jb.f(1);
                let ip = jb.f(1);
                let b = jb.u32f(&D_BEGIN);
                let n = jb.u32f(&D_SQNC);
                sq.push([h, ip, b, n]);
            }
            mt.sq = sq;
        }
        return 0;
    }
    mt.begin = jb.u32f(&D_BEGIN);
    if id == MTR_RCT {
        let r = jb.u32f(&D_RCT);
        if r >= 42 {
            return ERR_HDR;
        }
        mt.rct = r;
        return 0;
    }
    mt.numc = jb.u32f(&D_NUMC);
    mt.nbcol = jb.u32f(&D_NBCOL);
    mt.nbdelta = jb.u32f(&D_NBDELTA);
    let p = jb.f(4);
    if p >= 14 {
        return ERR_HDR;
    }
    mt.pred = p;
    0
}

/// jm_defsqueeze: DefaultSqueezeParameters into mt
fn jm_defsqueeze(mi: &Mi, mt: &mut Mt) -> i32 {
    let nm = mi.nbmeta;
    let nch = mi.nch();
    if nch <= nm {
        return ERR_HDR;
    }
    let nbc = nch - nm;
    let mut sq = Vec::with_capacity(64);
    let c = &mi.ch[nm as usize];
    let (mut w, mut h) = (c.w, c.h);
    if nbc > 2 {
        let c1 = &mi.ch[nm as usize + 1];
        if c1.w == w && c1.h == h {
            sq.push([1, 0, nm + 1, 2]);
            sq.push([0, 0, nm + 1, 2]);
        }
    }
    if w <= h && h > 8 {
        sq.push([0, 1, nm, nbc]);
        h = (h + 1) >> 1;
    }
    while w > 8 || h > 8 {
        if w > 8 {
            sq.push([1, 1, nm, nbc]);
            w = (w + 1) >> 1;
        }
        if h > 8 {
            sq.push([0, 1, nm, nbc]);
            h = (h + 1) >> 1;
        }
    }
    mt.sq = sq;
    0
}

/// jm_meta: MetaApply
fn jm_meta(mi: &mut Mi, mt: &mut Mt) -> i32 {
    match mt.id {
        MTR_RCT => mi.checkeq(mt.begin, mt.begin.wrapping_add(2)),
        MTR_PALETTE => {
            let bc = mt.begin;
            let nc = mt.numc;
            if nc == 0 {
                return ERR_HDR;
            }
            let ec = bc.wrapping_add(nc).wrapping_sub(1);
            let r = mi.checkeq(bc, ec);
            if r != 0 {
                return r;
            }
            if bc < mi.nbmeta {
                if ec >= mi.nbmeta {
                    return ERR_HDR;
                }
                mi.nbmeta = mi.nbmeta.wrapping_add(2).wrapping_sub(nc);
            } else {
                mi.nbmeta += 1;
            }
            mi.delete(bc + 1, nc - 1);
            mi.insert(0, mt.nbcol.wrapping_add(mt.nbdelta), nc, -1, -1)
        }
        _ => {
            if mt.sq.is_empty() {
                let r = jm_defsqueeze(mi, mt);
                if r != 0 {
                    return r;
                }
            }
            for s in 0..mt.sq.len() {
                let st = mt.sq[s];
                let bc = st[2];
                let nc = st[3];
                if nc == 0 {
                    return ERR_HDR;
                }
                let ec = bc.wrapping_add(nc).wrapping_sub(1);
                if ec < bc || ec >= mi.nch() {
                    return ERR_HDR;
                }
                if bc < mi.nbmeta {
                    if ec >= mi.nbmeta || st[1] == 0 {
                        return ERR_HDR;
                    }
                    mi.nbmeta += nc;
                }
                let offset = if st[1] != 0 { ec + 1 } else { mi.nch() };
                for c in bc..=ec {
                    let ch = &mut mi.ch[c as usize];
                    let (w, h) = (ch.w, ch.h);
                    if w == 0 || h == 0 {
                        return ERR_HDR;
                    }
                    let (aw, ah, rw, rh);
                    if st[0] != 0 {
                        aw = (w + 1) >> 1;
                        ah = h;
                        rw = w - aw;
                        rh = h;
                        if ch.hs >= 0 {
                            ch.hs += 1;
                        }
                    } else {
                        ah = (h + 1) >> 1;
                        aw = w;
                        rw = w;
                        rh = h - ah;
                        if ch.vs >= 0 {
                            ch.vs += 1;
                        }
                    }
                    let (hs, vs) = (ch.hs, ch.vs);
                    let r = mi.resize(c, aw, ah);
                    if r != 0 {
                        return r;
                    }
                    let r = mi.insert(c - bc + offset, rw, rh, hs, vs);
                    if r != 0 {
                        return r;
                    }
                }
            }
            0
        }
    }
}

/// jm_decode: ModularGenericDecompress (see the module header); hdr receives the group
/// header (the caller's JMH, or a local one)
#[allow(clippy::too_many_arguments)]
pub fn jm_decode(
    jb: &mut Jb,
    mi: &mut Mi,
    stream: u32,
    maxsize: u32,
    gtree: Option<&Ma>,
    gcode: Option<&Jc>,
    undo: bool,
    hdr: Option<&mut Jmh>,
) -> i32 {
    let mut local = Jmh::default();
    let hdr = match hdr {
        Some(h) => h,
        None => &mut local,
    };
    if mi.ch.is_empty() {
        return 0;
    }
    hdr.global = jb.f(1);
    hdr.wp = jm_wphdr(jb);
    let nt = jb.u32f(&D_NT);
    if nt > 4096 {
        return ERR_CODE;
    }
    mi.tr = Vec::new();
    mi.tr.resize(nt as usize, Mt::default());
    for i in 0..nt as usize {
        let r = jm_tread(jb, &mut mi.tr[i]);
        if r != 0 {
            return r;
        }
    }
    if jb.err != 0 {
        return jb.err;
    }
    for i in 0..nt as usize {
        let mut mt = core::mem::take(&mut mi.tr[i]);
        let r = jm_meta(mi, &mut mt);
        mi.tr[i] = mt;
        if r != 0 {
            return r;
        }
    }
    // distance multiplier, any channel to decode, local tree size limit
    let mut dm = 0u32;
    let mut any = false;
    let mut limit: u64 = 1024;
    for (c, ch) in mi.ch.iter().enumerate() {
        if c as u32 >= mi.nbmeta && (ch.w > maxsize || ch.h > maxsize) {
            break;
        }
        limit = limit.wrapping_add(ch.w as u64 * ch.h as u64);
        if ch.w != 0 && ch.h != 0 {
            dm = dm.max(ch.w);
            any = true;
        }
    }
    if any {
        let ma_local;
        let jc_local;
        let (tree, code): (&Ma, &Jc) = if hdr.global == 0 {
            let lim = limit.min(1 << 20);
            ma_local = match jm_tree(jb, lim) {
                Ok(m) => m,
                Err(e) => return e,
            };
            jc_local = match je_read(jb, (ma_local.nodes.len() as u32 + 1) >> 1, false) {
                Ok(c) => c,
                Err(e) => return e,
            };
            (&ma_local, &jc_local)
        } else {
            match (gtree, gcode) {
                (Some(t), Some(c)) => (t, c),
                _ => return ERR_CODE,
            }
        };
        let mut jr = match Jr::new(code, jb, dm) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let wh = hdr.wp;
        for c in 0..mi.ch.len() {
            let ch = &mi.ch[c];
            if ch.w == 0 || ch.h == 0 {
                continue;
            }
            if c as u32 >= mi.nbmeta && (ch.w > maxsize || ch.h > maxsize) {
                break;
            }
            let r = jm_channel(&mut jr, jb, tree, &wh, mi, c, stream);
            if r != 0 {
                return r;
            }
        }
        if !jr.fin() {
            return ERR_CODE;
        }
        if jb.err != 0 {
            return jb.err;
        }
    }
    if undo {
        return crate::mtr::jm_undo(mi, &hdr.wp);
    }
    0
}

// ---------------------------------------------------------------- weighted predictor

/// weighted predictor state (WS)
pub struct Wp {
    wh: [u32; 11],
    xs: u32,
    n: u32,         // entries per array ((xs + 2) * 2)
    pred: i64,      // prediction before removing the extra bits
    pr: [i64; 4],   // sub-predictions
    err: Vec<i32>,  // true errors [n], then the 4 predictor error arrays [n] each
}

/// wp_errweight: 4 + ((maxweight * divlookup[x >> shift]) >> shift)
#[inline]
fn errweight(x: u32, maxw: u32) -> u32 {
    let b = 63 - (x as u64 + 1).leading_zeros() as i32;
    let sh = if b - 5 < 0 { 0 } else { (b - 5) as u32 };
    let v = DIVLOOKUP[((x >> sh) & 63) as usize];
    (v.wrapping_mul(maxw) >> sh).wrapping_add(4)
}

impl Wp {
    /// wp_new
    pub fn new(wh: &[u32; 11], xs: u32) -> Result<Wp, i32> {
        let n = (xs + 2) * 2;
        let err = zvec::<i32>(n as usize * 5)?;
        Ok(Wp { wh: *wh, xs, n, pred: 0, pr: [0; 4], err })
    }

    /// wp_predict: prediction (rounded, extra bits removed); nb = N, W, NE, NW, NN; props:
    /// property 15 is set
    pub fn predict(&mut self, x: u32, y: u32, nb: &[i32; 5], props: Option<&mut [i32]>) -> i64 {
        let xs = self.xs;
        let n = self.n as usize;
        let (cur, prev) = if y & 1 != 0 { (0u32, xs + 2) } else { (xs + 2, 0u32) };
        let pn = prev + x;
        let mut pne = pn;
        let mut pnw = pn;
        if x < xs.wrapping_sub(1) {
            pne += 1;
        }
        if x != 0 {
            pnw -= 1;
        }
        let (pn, pne, pnw) = (pn as usize, pne as usize, pnw as usize);
        let mut w = [0u32; 4];
        for (i, wi) in w.iter_mut().enumerate() {
            let pe = &self.err[n * (i + 1)..n * (i + 2)];
            let s = (pe[pn] as u32).wrapping_add(pe[pne] as u32).wrapping_add(pe[pnw] as u32);
            *wi = errweight(s, self.wh[7 + i]);
        }
        let nn = nb[0] as i64 * 8;
        let ww = nb[1] as i64 * 8;
        let ne = nb[2] as i64 * 8;
        let nw = nb[3] as i64 * 8;
        let nnn = nb[4] as i64 * 8;
        let tew = if x != 0 { self.err[(cur + x - 1) as usize] as i64 } else { 0 };
        let ten = self.err[pn] as i64;
        let tenw = self.err[pnw] as i64;
        let tene = self.err[pne] as i64;
        if let Some(p) = props {
            let mut pv = tew;
            let mut pa = pv.wrapping_abs();
            for t in [ten, tenw, tene] {
                let ta = t.wrapping_abs();
                if ta > pa {
                    pv = t;
                    pa = ta;
                }
            }
            p[15] = pv as i32;
        }
        let wh = &self.wh;
        let sumwn = ten.wrapping_add(tew);
        self.pr[0] = ww.wrapping_add(ne).wrapping_sub(nn);
        self.pr[1] = nn.wrapping_sub((sumwn.wrapping_add(tene)).wrapping_mul(wh[0] as i32 as i64) >> 5);
        self.pr[2] = ww.wrapping_sub((sumwn.wrapping_add(tenw)).wrapping_mul(wh[1] as i32 as i64) >> 5);
        let mut a = tenw.wrapping_mul(wh[2] as i32 as i64);
        a = a.wrapping_add(ten.wrapping_mul(wh[3] as i32 as i64));
        a = a.wrapping_add(tene.wrapping_mul(wh[4] as i32 as i64));
        a = a.wrapping_add(nnn.wrapping_sub(nn).wrapping_mul(wh[5] as i32 as i64));
        a = a.wrapping_add(nw.wrapping_sub(ww).wrapping_mul(wh[6] as i32 as i64));
        self.pr[3] = nn.wrapping_sub(a >> 5);
        // weighted average
        let tot = w[0].wrapping_add(w[1]).wrapping_add(w[2]).wrapping_add(w[3]);
        let lw = (31 - tot.leading_zeros()).wrapping_sub(4) & 31;
        let mut ws = 0u32;
        for wi in w.iter_mut() {
            *wi >>= lw;
            ws = ws.wrapping_add(*wi);
        }
        let mut sum = ((ws >> 1) as i64).wrapping_sub(1);
        for i in 0..4 {
            sum = sum.wrapping_add(self.pr[i].wrapping_mul(w[i] as i64));
        }
        let dl = DIVLOOKUP[(ws.wrapping_sub(1) & 63) as usize] as i64;
        let mut pred = sum.wrapping_mul(dl) >> 24;
        self.pred = pred;
        if ((ten ^ tew) | (ten ^ tenw)) <= 0 {
            let mx = ww.max(ne).max(nn);
            let mn = ww.min(ne).min(nn);
            if pred > mx {
                pred = mx;
            }
            if pred < mn {
                pred = mn;
            }
            self.pred = pred;
        }
        pred.wrapping_add(3) >> 3
    }

    /// wp_update: errors of the decoded value at (x, y)
    pub fn update(&mut self, val: i32, x: u32, y: u32) {
        let v = (val as i64) << 3;
        let xs2 = self.xs + 2;
        let (cur, prev) = if y & 1 != 0 { (0u32, xs2) } else { (xs2, 0u32) };
        let c = (cur + x) as usize;
        let p = (prev + x + 1) as usize;
        let n = self.n as usize;
        self.err[c] = self.pred.wrapping_sub(v) as i32;
        for i in 0..4 {
            let d = (self.pr[i].wrapping_sub(v).wrapping_abs().wrapping_add(3) >> 3) as i32;
            let o = n * (i + 1);
            self.err[o + c] = d;
            self.err[o + p] = self.err[o + p].wrapping_add(d);
        }
    }
}

/// clamped gradient (32 bit)
#[inline(always)]
pub fn cgrad(n: i32, w: i32, l: i32) -> i32 {
    let mn = if n > w { w } else { n };
    let mx = if n < w { w } else { n };
    let mut d = n.wrapping_add(w).wrapping_sub(l);
    if l > mx {
        d = mn;
    }
    if l < mn {
        d = mx;
    }
    d
}

/// truncating halving of a 64-bit sum
#[inline(always)]
fn half(v: i64) -> i64 {
    (v.wrapping_add((v as u64 >> 63) as i64)) >> 1
}

/// PredictOne: nb = left, top, topleft, topright, leftleft, toptop, toprightright; wpp =
/// the weighted predictor value (predictor 6)  [jm_predict]
#[inline]
pub fn jm_predict(pred: u32, nb: &[i32; 7], wpp: i64) -> i64 {
    let l = nb[0] as i64;
    let t = nb[1] as i64;
    let tl = nb[2] as i64;
    let tr = nb[3] as i64;
    match pred {
        0 => 0,
        1 => l,
        2 => t,
        3 => half(l.wrapping_add(t)),
        4 => {
            let p = l.wrapping_add(t).wrapping_sub(tl);
            let pa = p.wrapping_sub(l).wrapping_abs();
            let pb = p.wrapping_sub(t).wrapping_abs();
            if pa < pb { l } else { t }
        }
        5 => cgrad(nb[0], nb[1], nb[2]) as i64,
        6 => wpp,
        7 => tr,
        8 => tl,
        9 => nb[4] as i64,
        10 => half(l.wrapping_add(tl)),
        11 => half(tl.wrapping_add(t)),
        12 => half(t.wrapping_add(tr)),
        13 => {
            let mut a = t.wrapping_mul(6);
            a = a.wrapping_sub((nb[5] as i64).wrapping_mul(2));
            a = a.wrapping_add(l.wrapping_mul(7));
            a = a.wrapping_add(nb[4] as i64);
            a = a.wrapping_add(nb[6] as i64);
            a = a.wrapping_add(tr.wrapping_mul(3));
            a = a.wrapping_add(8);
            (a.wrapping_add((a >> 63) & 15)) >> 4
        }
        _ => 0,
    }
}

/// jm_channel: DecodeChannel of channel c with the MA tree
fn jm_channel(jr: &mut Jr, jb: &mut Jb, ma: &Ma, wh: &[u32; 11], mi: &mut Mi, c: usize, gid: u32) -> i32 {
    let w = mi.ch[c].w;
    let h = mi.ch[c].h;
    let wu = w as usize;
    let mut np = 16u32;
    let mp1 = ma.maxprop.wrapping_add(1);
    if mp1 > 16 {
        np = ((mp1 - 13) as u32 & !3).wrapping_add(16);
    }
    let nrefmax = np - 16;
    // reference channels: earlier channels of the same size and shifts
    let mut d = core::mem::take(&mut mi.ch[c].d);
    let mut refs: Vec<&[i32]> = Vec::new();
    {
        let me = &mi.ch[c];
        let mut k = c;
        while k != 0 {
            k -= 1;
            if refs.len() as u32 * 4 >= nrefmax {
                break;
            }
            if mi.ch[k].same(me) {
                refs.push(&mi.ch[k].d);
            }
        }
    }
    let mut ws = if ma.wp != 0 {
        match Wp::new(wh, w) {
            Ok(v) => Some(v),
            Err(e) => {
                mi.ch[c].d = d;
                return e;
            }
        }
    } else {
        None
    };
    let mut props = [0i32; 256];
    props[0] = c as i32;
    props[1] = gid as i32;
    let nodes = &ma.nodes[..];
    let mut wpp: i64 = 0;
    let mut res = 0;
    'rows: for y in 0..h {
        let ro = y as usize * wu;
        props[2] = y as i32;
        let mut p9: i32 = 0;
        for x in 0..w {
            let xi = x as usize;
            let i = ro + xi;
            let has_prev = y != 0;
            let left = if xi > 0 {
                d[i - 1]
            } else if has_prev {
                d[i - wu]
            } else {
                0
            };
            let top = if has_prev { d[i - wu] } else { left };
            let tl = if xi > 0 && has_prev { d[i - wu - 1] } else { left };
            let tr = if xi + 1 < wu && has_prev { d[i - wu + 1] } else { top };
            let ll = if xi > 1 { d[i - 2] } else { left };
            let tt = if y > 1 { d[i - 2 * wu] } else { top };
            let trr = if xi + 2 < wu && has_prev { d[i - wu + 2] } else { tr };
            props[3] = x as i32;
            props[4] = top.wrapping_abs();
            props[5] = left.wrapping_abs();
            props[6] = top;
            props[7] = left;
            props[8] = left.wrapping_sub(p9);
            p9 = left.wrapping_add(top).wrapping_sub(tl);
            props[9] = p9;
            props[10] = left.wrapping_sub(tl);
            props[11] = tl.wrapping_sub(top);
            props[12] = top.wrapping_sub(tr);
            props[13] = top.wrapping_sub(tt);
            props[14] = left.wrapping_sub(ll);
            if let Some(wsr) = ws.as_mut() {
                let nb = [top, left, tr, tl, tt];
                wpp = wsr.predict(x, y, &nb, Some(&mut props[..]));
            }
            for (k, rd) in refs.iter().enumerate() {
                let v = rd[i];
                let vl = if xi > 0 { rd[i - 1] } else { 0 };
                let (mut vt, mut vtl) = (vl, vl);
                if has_prev {
                    vt = rd[i - wu];
                    if xi > 0 {
                        vtl = rd[i - wu - 1];
                    }
                }
                let pv = cgrad(vl, vt, vtl);
                let o = 16 + 4 * k;
                props[o] = v.wrapping_abs();
                props[o + 1] = v;
                let dv = v.wrapping_sub(pv);
                props[o + 2] = dv.wrapping_abs();
                props[o + 3] = dv;
            }
            // MA tree
            let mut nd = &nodes[0];
            while nd.prop >= 0 {
                let v = props[nd.prop as usize & 255];
                let k = if v > nd.split { nd.l } else { nd.r };
                nd = &nodes[k as usize]; // complete by construction (jm_tree)
            }
            let l64 = left as i64;
            let t64 = top as i64;
            let tl64 = tl as i64;
            let tr64 = tr as i64;
            let g: i64 = match nd.pred {
                0 => 0,
                1 => l64,
                2 => t64,
                3 => half(l64.wrapping_add(t64)),
                4 => {
                    let p = l64.wrapping_add(t64).wrapping_sub(tl64);
                    let pa = p.wrapping_sub(l64).wrapping_abs();
                    let pb = p.wrapping_sub(t64).wrapping_abs();
                    if pa < pb { l64 } else { t64 }
                }
                5 => cgrad(left, top, tl) as i64,
                6 => wpp,
                7 => tr64,
                8 => tl64,
                9 => ll as i64,
                10 => half(l64.wrapping_add(tl64)),
                11 => half(tl64.wrapping_add(t64)),
                12 => half(t64.wrapping_add(tr64)),
                _ => {
                    let mut a = t64.wrapping_mul(6);
                    a = a.wrapping_sub((tt as i64).wrapping_mul(2));
                    a = a.wrapping_add(l64.wrapping_mul(7));
                    a = a.wrapping_add(ll as i64);
                    a = a.wrapping_add(trr as i64);
                    a = a.wrapping_add(tr64.wrapping_mul(3));
                    a = a.wrapping_add(8);
                    (a.wrapping_add((a >> 63) & 15)) >> 4
                }
            };
            let guess = g.wrapping_add(nd.off as i64);
            let tok = jr.read(jb, nd.l);
            let v = unpack_signed(tok).wrapping_mul(nd.mul as i32).wrapping_add(guess as i32);
            d[i] = v;
            if let Some(wsr) = ws.as_mut() {
                wsr.update(v, x, y);
            }
        }
        if jb.err != 0 {
            res = jb.err;
            break 'rows;
        }
    }
    drop(refs);
    mi.ch[c].d = d;
    res
}
