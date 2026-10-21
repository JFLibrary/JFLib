// ent -- JPEG XL entropy decoding (dec_ans.cc, ans_common.cc, dec_huffman.cc,
// dec_context_map.cc; model: tools/jxl_ref.py read_histogram .. Reader)  [src/img/jxl_ent.s]
//
//   je_read(jb, nctx, nolz77) -> Jc / ERR_*   DecodeHistograms: context map, hybrid uint
//                                             configs, ANS alias tables or prefix codes
//   je_ctxmap(jb, n, out) -> 0 / ERR_*        DecodeContextMap into out[n]
//   Jr::new(jc, jb, dist_mult) -> Jr / ERR_*  symbol reader: reads the ANS state, allocates
//                                             the LZ77 window
//   jr.read(jb, ctx) -> u32                   ReadHybridUint( ctx ) through the context map
//   jr.clustered(jb, h) -> u32                ReadHybridUintClustered
//   jr.fin() -> bool                          ANS final state ok
//   ej_alias(counts, len, log_alpha, table)   InitAliasTable (range 4096; the writer uses it)
//
// ANS: 12-bit tables, alias entries (cutoff, right, freq0, offset, freq1).  Prefix codes:
// canonical (Brotli order, bits read LSB first, code built MSB first), 10-bit lookup table
// plus a bit-by-bit fallback for longer codes.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_NOMEM};

use crate::bits::Jb;
use crate::tab;

const ANS_SIG: u32 = 0x13 << 16;
const LZ_WINBITS: u32 = 20;
const LZ_MASK: u64 = (1 << LZ_WINBITS) - 1;
const JP_FASTBITS: u32 = 10;

static D_LZMIN: [u32; 8] = [224, 0, 512, 0, 4096, 0, 8, 15];
static D_LZLEN: [u32; 8] = [3, 0, 4, 0, 5, 2, 9, 8];
static CLORDER: [u8; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
static CLHUFF: [u8; 32] = [2, 0, 2, 4, 2, 3, 3, 2, 2, 0, 2, 4, 2, 3, 4, 1, 2, 0, 2, 4, 2, 3, 3, 2, 2, 0, 2, 4, 2, 3, 4, 5];

/// an alias table entry (8 bytes in the assembly)
#[derive(Clone, Copy, Default)]
pub struct Ae {
    pub cutoff: u8,
    pub right: u8,
    pub freq0: u16,
    pub offset: u16,
    pub freq1: u16,
}

/// a prefix code (JP)
pub struct Jp {
    pub single: i32, // the only symbol, -1 = real code
    pub maxlen: u32,
    pub firstc: [u32; 16],
    pub firsti: [u32; 16],
    pub count: [u32; 16],
    pub nsym: u32,
    pub fast: [u32; 1024], // sym << 8 | len (len 0 = longer than 10 bits)
    pub sorted: Vec<u16>,  // symbols by (length, symbol)
}

impl Jp {
    fn new(nsorted: usize) -> Box<Jp> {
        Box::new(Jp {
            single: 0,
            maxlen: 0,
            firstc: [0; 16],
            firsti: [0; 16],
            count: [0; 16],
            nsym: 0,
            fast: [0; 1024],
            sorted: vec![0; nsorted],
        })
    }
}

/// a histogram code (JC)
#[derive(Default)]
pub struct Jc {
    pub lz77: u32,
    pub lzmin: u32,
    pub lzlen: u32,
    pub lzcfg: [u8; 4],
    pub lzctx: u32,
    pub nctx: u32,
    pub nhist: u32,
    pub prefix: u32,
    pub loga: u32,
    pub cmap: Vec<u8>,
    pub cfg: Vec<[u8; 4]>,
    pub ans: Vec<Ae>,      // ANS: nhist << loga entries
    pub pfx: Vec<Box<Jp>>, // prefix codes
}

/// ej_varlen: VarLenUint8 (nbits 3) / VarLenUint16 (nbits 4)
fn varlen(jb: &mut Jb, nbits: u32) -> u32 {
    if jb.f(1) == 0 {
        return 0;
    }
    let n = jb.f(nbits);
    if n == 0 {
        return 1;
    }
    jb.f(n).wrapping_add(1u32.wrapping_shl(n))
}

/// ej_ceillog2: 0 for x <= 1
#[inline]
pub fn ceillog2(x: u32) -> u32 {
    if x <= 1 { 0 } else { 32 - (x - 1).leading_zeros() }
}

/// ej_uintcfg: HybridUintConfig
fn uintcfg(jb: &mut Jb, loga: u32) -> Result<[u8; 4], i32> {
    let split = jb.f(ceillog2(loga + 1));
    let (mut msb, mut lsb) = (0u32, 0u32);
    if split != loga {
        msb = jb.f(ceillog2(split + 1));
        if msb > split {
            return Err(ERR_CODE);
        }
        lsb = jb.f(ceillog2(split - msb + 1));
    }
    if msb + lsb > split || split > 31 {
        return Err(ERR_CODE);
    }
    Ok([split as u8, msb as u8, lsb as u8, 0])
}

/// ej_histogram: ReadHistogram into counts -> length (1..256) / ERR_CODE
fn histogram(jb: &mut Jb, counts: &mut [u32; 260]) -> Result<u32, i32> {
    counts.fill(0);
    if jb.f(1) != 0 {
        // simple code: 1 or 2 symbols
        let n = jb.f(1) + 1;
        let s0 = varlen(jb, 3);
        let mut max = s0;
        if n == 1 {
            counts[s0 as usize] = 4096;
            return Ok(s0 + 1);
        }
        let s1 = varlen(jb, 3);
        if s1 == s0 {
            return Err(ERR_CODE);
        }
        if s1 > max {
            max = s1;
        }
        let c0 = jb.f(12);
        counts[s0 as usize] = c0;
        counts[s1 as usize] = 4096u32.wrapping_sub(c0);
        return Ok(max + 1);
    }
    if jb.f(1) != 0 {
        // flat
        let len = varlen(jb, 3) + 1;
        let (q, r) = (4096 / len, 4096 % len);
        for i in 0..len {
            counts[i as usize] = q + (i < r) as u32;
        }
        return Ok(len);
    }
    let mut log = 0u32;
    while log < 3 && jb.f(1) != 0 {
        log += 1;
    }
    let shift = (jb.f(log) | (1 << log)).wrapping_sub(1);
    if shift > 13 {
        return Err(ERR_CODE);
    }
    let length = varlen(jb, 3) + 3;
    if length > 256 {
        return Err(ERR_CODE);
    }
    // logcounts (an RLE entry keeps lc 13 | same << 8)
    let mut i = 0u32;
    let mut omit_log: i32 = -1;
    let mut omit_pos: i32 = -1;
    while i < length {
        let pk = (jb.peek64() & 127) as usize;
        let bits = tab::HISTHUFF[pk * 2] as u32;
        let lc = tab::HISTHUFF[pk * 2 + 1] as u32;
        jb.pos = jb.pos.wrapping_add(bits as u64);
        if lc == 13 {
            let rle = varlen(jb, 3);
            counts[i as usize] = 13 | (rle + 5) << 8;
            i = i.wrapping_add(rle).wrapping_add(4);
            continue;
        }
        counts[i as usize] = lc;
        if lc as i32 > omit_log {
            omit_log = lc as i32;
            omit_pos = i as i32;
        }
        i += 1;
    }
    if omit_pos < 0 {
        return Err(ERR_CODE);
    }
    // counts from the logcounts (in place, left to right)
    let mut total: u32 = 0;
    let mut prev: u32 = 0;
    let mut numsame: u32 = 0;
    for i in 0..length as usize {
        let v = counts[i];
        let same = v >> 8;
        if same != 0 {
            numsame = same - 1;
            prev = if i != 0 { counts[i - 1] } else { 0 };
        }
        if numsame != 0 {
            counts[i] = prev;
            numsame -= 1;
            total = total.wrapping_add(prev);
            continue;
        }
        let lc = v & 255;
        if i as i32 == omit_pos || lc == 0 {
            counts[i] = 0;
        } else if lc == 1 {
            counts[i] = 1;
            total = total.wrapping_add(1);
        } else {
            let cm1 = lc - 1;
            let mut bc = shift as i32 - ((12 - cm1 as i32) >> 1);
            if bc > cm1 as i32 {
                bc = cm1 as i32;
            }
            if bc < 0 {
                bc = 0;
            }
            let bits = jb.f(bc as u32);
            let c = (bits << (cm1 - bc as u32)).wrapping_add(1 << cm1);
            counts[i] = c;
            total = total.wrapping_add(c);
        }
    }
    let r = 4096i32.wrapping_sub(total as i32);
    if r <= 0 {
        return Err(ERR_CODE);
    }
    counts[omit_pos as usize] = r as u32;
    Ok(length)
}

/// ej_alias: InitAliasTable (range 4096) of counts[..len] into table (1 << loga entries)
pub fn ej_alias(counts: &mut [u32], len: u32, loga: u32, table: &mut [Ae]) -> i32 {
    let mut len = len as usize;
    while len > 0 && counts[len - 1] == 0 {
        len -= 1;
    }
    if len == 0 {
        counts[0] = 4096;
        len = 1;
    }
    let tsize = 1usize << loga;
    if len > tsize {
        return ERR_CODE;
    }
    let esize = 4096u32 >> loga;
    let mut sum = 0u32;
    let mut single: i32 = -1;
    for (i, &c) in counts.iter().enumerate().take(len) {
        sum = sum.wrapping_add(c);
        if c == 4096 {
            single = i as i32;
        }
    }
    if sum != 4096 {
        return ERR_CODE;
    }
    if single >= 0 {
        for (i, e) in table.iter_mut().enumerate().take(tsize) {
            *e = Ae { cutoff: 0, right: single as u8, freq0: 0, offset: (esize * i as u32) as u16, freq1: 4096 };
        }
        return 0;
    }
    let mut cut = [0u32; 256];
    let mut right = [0u32; 256];
    let mut offs = [0u32; 256];
    let mut under = [0u16; 256];
    let mut over = [0u16; 256];
    let (mut nu, mut no) = (0usize, 0usize);
    for i in 0..tsize {
        let c = if i < len { counts[i] } else { 0 };
        cut[i] = c;
        if c < esize {
            under[nu] = i as u16;
            nu += 1;
        } else if c > esize {
            over[no] = i as u16;
            no += 1;
        }
    }
    while no != 0 {
        if nu == 0 {
            return ERR_CODE;
        }
        no -= 1;
        let o = over[no] as usize;
        nu -= 1;
        let u = under[nu] as usize;
        let by = esize.wrapping_sub(cut[u]);
        cut[o] = cut[o].wrapping_sub(by);
        right[u] = o as u32;
        offs[u] = cut[o];
        if cut[o] < esize {
            under[nu] = o as u16;
            nu += 1;
        } else if cut[o] > esize {
            over[no] = o as u16;
            no += 1;
        }
    }
    for i in 0..tsize {
        let c = cut[i];
        let (r, o, cc) = if c == esize { (i as u32, 0u32, 0u32) } else { (right[i], offs[i].wrapping_sub(c), c) };
        table[i] = Ae {
            cutoff: cc as u8,
            right: r as u8,
            offset: o as u16,
            freq0: if i < len { counts[i] as u16 } else { 0 },
            freq1: if (r as usize) < len { counts[r as usize] as u16 } else { 0 },
        };
    }
    0
}

/// ej_pbuild: canonical code from lengths[..n]
fn pbuild(lengths: &[u8], n: u32, jp: &mut Jp) -> i32 {
    jp.nsym = n;
    jp.firstc = [0; 16];
    jp.firsti = [0; 16];
    jp.count = [0; 16];
    jp.single = 0; // (cleared with the arrays)
    jp.maxlen = 0;
    let mut nz = 0u32;
    let mut last: i32 = -1;
    let mut maxl = 0u32;
    for i in 0..n as usize {
        let l = lengths[i] as u32;
        if l == 0 {
            continue;
        }
        if l > 15 {
            return ERR_CODE;
        }
        jp.count[l as usize] += 1;
        nz += 1;
        last = i as i32;
        if l > maxl {
            maxl = l;
        }
    }
    if nz <= 1 {
        jp.single = if last < 0 { 0 } else { last };
        return 0;
    }
    jp.single = -1;
    jp.maxlen = maxl;
    let (mut code, mut idx) = (0u32, 0u32);
    for l in 1..16 {
        code <<= 1;
        jp.firstc[l] = code;
        jp.firsti[l] = idx;
        code = code.wrapping_add(jp.count[l]);
        idx = idx.wrapping_add(jp.count[l]);
    }
    let mut o = 0usize;
    for l in 1..=maxl {
        for i in 0..n as usize {
            if lengths[i] as u32 == l {
                if o >= jp.sorted.len() {
                    jp.sorted.resize(o + 16, 0);
                }
                jp.sorted[o] = i as u16;
                o += 1;
            }
        }
    }
    jp.fast = [0; 1024];
    for l in 1..=JP_FASTBITS.min(15) {
        let mut cnt = jp.count[l as usize];
        let mut si = jp.firsti[l as usize];
        let mut c = jp.firstc[l as usize];
        while cnt != 0 {
            let e = (jp.sorted.get(si as usize).copied().unwrap_or(0) as u32) << 8 | l;
            let mut r = 0u32;
            let mut d = c;
            for _ in 0..l {
                r = r << 1 | (d & 1);
                d >>= 1;
            }
            let step = 1u32 << l;
            while r < 1024 {
                jp.fast[r as usize] = e;
                r += step;
            }
            si += 1;
            c = c.wrapping_add(1);
            cnt -= 1;
        }
    }
    0
}

/// ej_psym: symbol / -1 invalid code
#[inline]
fn psym(jp: &Jp, jb: &mut Jb) -> i32 {
    if jp.single >= 0 {
        return jp.single;
    }
    let mut bits = jb.peek64();
    let e = jp.fast[(bits & 1023) as usize];
    let len = e & 255;
    if len != 0 {
        jb.pos = jb.pos.wrapping_add(len as u64);
        return (e >> 8) as i32;
    }
    let mut code = 0u32;
    let mut l = 1u32;
    loop {
        code = code.wrapping_mul(2).wrapping_add((bits & 1) as u32);
        bits >>= 1;
        let d = code.wrapping_sub(jp.firstc[l as usize]);
        if d < jp.count[l as usize] {
            jb.pos = jb.pos.wrapping_add(l as u64);
            let k = d.wrapping_add(jp.firsti[l as usize]) as usize;
            return jp.sorted.get(k).copied().unwrap_or(0) as i32;
        }
        l += 1;
        if l > jp.maxlen || l >= 16 {
            return -1;
        }
    }
}

/// ej_prefix: read one prefix code of the alphabet size
fn prefix(jb: &mut Jb, alpha: u32) -> Result<Box<Jp>, i32> {
    let mut jp = Jp::new(alpha as usize + 16);
    let mut lengths = vec![0u8; alpha as usize + 16];
    let sel = jb.f(2);
    if sel == 1 {
        // simple code
        let maxbits = if alpha > 1 { 32 - (alpha - 1).leading_zeros() } else { 0 };
        let mut n = jb.f(2) + 1;
        let mut s = [0u32; 4];
        for k in 0..n as usize {
            s[k] = jb.f(maxbits);
            if s[k] >= alpha {
                return Err(ERR_CODE);
            }
        }
        for a in 0..n as usize {
            for b in a + 1..n as usize {
                if s[a] == s[b] {
                    return Err(ERR_CODE);
                }
            }
        }
        if n == 4 {
            n += jb.f(1);
        }
        match n {
            1 => {
                jp.single = s[0] as i32;
                jp.nsym = alpha;
                return Ok(jp);
            }
            2 => {
                lengths[s[0] as usize] = 1;
                lengths[s[1] as usize] = 1;
            }
            3 => {
                lengths[s[0] as usize] = 1;
                lengths[s[1] as usize] = 2;
                lengths[s[2] as usize] = 2;
            }
            4 => {
                for &v in &s {
                    lengths[v as usize] = 2;
                }
            }
            _ => {
                lengths[s[0] as usize] = 1;
                lengths[s[1] as usize] = 2;
                lengths[s[2] as usize] = 3;
                lengths[s[3] as usize] = 3;
            }
        }
        let r = pbuild(&lengths, alpha, &mut jp);
        return if r != 0 { Err(r) } else { Ok(jp) };
    }
    // code length code: 18 lengths, static 4-bit code
    let mut cl = [0u8; 24];
    let mut space: i32 = 32;
    let mut ncodes = 0u32;
    let mut idx = sel;
    while idx < 18 && space > 0 {
        let pk = (jb.peek64() & 15) as usize;
        let bits = CLHUFF[pk * 2] as u32;
        let v = CLHUFF[pk * 2 + 1] as u32;
        jb.pos = jb.pos.wrapping_add(bits as u64);
        cl[CLORDER[idx as usize] as usize] = v as u8;
        if v != 0 {
            space -= (32 >> v) as i32;
            ncodes += 1;
        }
        idx += 1;
    }
    if ncodes != 1 && space != 0 {
        return Err(ERR_CODE);
    }
    let r = pbuild(&cl, 18, &mut jp);
    if r != 0 {
        return Err(r);
    }
    // symbol lengths
    let mut sym = 0u32;
    let mut prev = 8u32;
    let mut repeat = 0u32;
    let mut repeat_len = 0u32;
    let mut space: i32 = 32768;
    while sym < alpha && space > 0 {
        let v = psym(&jp, jb);
        if v < 0 {
            return Err(ERR_CODE);
        }
        let v = v as u32;
        if v < 16 {
            repeat = 0;
            lengths[sym as usize] = v as u8;
            sym += 1;
            if v != 0 {
                prev = v;
                space -= (32768 >> v) as i32;
            }
            continue;
        }
        let extra = v - 14;
        let new_len = if v == 16 { prev } else { 0 };
        if new_len != repeat_len {
            repeat = 0;
            repeat_len = new_len;
        }
        let old = repeat;
        if repeat != 0 {
            repeat = (repeat - 2) << extra;
        }
        repeat = repeat.wrapping_add(jb.f(extra)).wrapping_add(3);
        let mut delta = repeat.wrapping_sub(old);
        if sym.wrapping_add(delta) > alpha {
            return Err(ERR_CODE);
        }
        while delta != 0 {
            lengths[sym as usize] = repeat_len as u8;
            sym += 1;
            delta -= 1;
            if repeat_len != 0 {
                space -= 1 << (15 - repeat_len);
            }
        }
    }
    if space != 0 {
        return Err(ERR_CODE);
    }
    let r = pbuild(&lengths, alpha, &mut jp);
    if r != 0 { Err(r) } else { Ok(jp) }
}

/// je_ctxmap: DecodeContextMap into out[..n]
pub fn je_ctxmap(jb: &mut Jb, n: u32, out: &mut [u8]) -> i32 {
    let n = n as usize;
    if jb.f(1) != 0 {
        let bits = jb.f(2);
        for o in out.iter_mut().take(n) {
            *o = jb.f(bits) as u8;
        }
    } else {
        let mtf = jb.f(1);
        let jc = match je_read(jb, 1, n <= 2) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let mut jr = match Jr::new(&jc, jb, 0) {
            Ok(r) => r,
            Err(e) => return e,
        };
        for o in out.iter_mut().take(n) {
            let v = jr.read(jb, 0);
            if v >= 256 {
                return ERR_CODE;
            }
            *o = v as u8;
        }
        if !jr.fin() {
            return ERR_CODE;
        }
        if mtf != 0 {
            let mut l = [0u8; 256];
            for (i, v) in l.iter_mut().enumerate() {
                *v = i as u8;
            }
            for o in out.iter_mut().take(n) {
                let k = *o as usize;
                let v = l[k];
                *o = v;
                l.copy_within(0..k, 1);
                l[0] = v;
            }
        }
    }
    // every cluster 0 .. max must occur
    let mut seen = [false; 256];
    let mut max = 0usize;
    for &v in out.iter().take(n) {
        seen[v as usize] = true;
        if v as usize > max {
            max = v as usize;
        }
    }
    if seen[..=max].iter().all(|&s| s) { 0 } else { ERR_CODE }
}

/// je_read: DecodeHistograms for nctx contexts
pub fn je_read(jb: &mut Jb, nctx: u32, nolz77: bool) -> Result<Jc, i32> {
    let mut jc = Jc::default();
    let mut nctx = nctx;
    jc.lz77 = jb.f(1);
    if jc.lz77 != 0 {
        if nolz77 {
            return Err(ERR_CODE);
        }
        jc.lzmin = jb.u32f(&D_LZMIN);
        jc.lzlen = jb.u32f(&D_LZLEN);
        nctx = nctx.wrapping_add(1);
        jc.lzcfg = uintcfg(jb, 8)?;
    }
    jc.nctx = nctx;
    let mut cmap: Vec<u8> = Vec::new();
    if cmap.try_reserve_exact(nctx as usize + 16).is_err() {
        return Err(ERR_NOMEM);
    }
    cmap.resize(nctx as usize + 16, 0);
    if nctx > 1 {
        let r = je_ctxmap(jb, nctx, &mut cmap);
        if r != 0 {
            return Err(r);
        }
    }
    let mut max = 0u32;
    for &v in cmap.iter().take(nctx.max(1) as usize) {
        max = max.max(v as u32);
    }
    jc.nhist = max + 1;
    jc.lzctx = cmap[(nctx as usize).wrapping_sub(1).min(cmap.len() - 1)] as u32;
    jc.cmap = cmap;
    jc.prefix = jb.f(1);
    jc.loga = if jc.prefix != 0 { 15 } else { jb.f(2) + 5 };
    let mut cfg = Vec::with_capacity(jc.nhist as usize);
    for _ in 0..jc.nhist {
        cfg.push(uintcfg(jb, jc.loga)?);
    }
    jc.cfg = cfg;
    if jc.prefix != 0 {
        let mut sizes = Vec::with_capacity(jc.nhist as usize);
        for _ in 0..jc.nhist {
            sizes.push(varlen(jb, 4) + 1);
        }
        let mut pfx = Vec::with_capacity(jc.nhist as usize);
        for &s in &sizes {
            if s <= 1 {
                let mut jp = Jp::new(16);
                jp.single = 0;
                pfx.push(jp);
            } else {
                pfx.push(prefix(jb, s)?);
            }
        }
        jc.pfx = pfx;
    } else {
        let per = 1usize << jc.loga;
        let mut tabs: Vec<Ae> = Vec::new();
        if tabs.try_reserve_exact(per * jc.nhist as usize).is_err() {
            return Err(ERR_NOMEM);
        }
        tabs.resize(per * jc.nhist as usize, Ae::default());
        let mut counts = [0u32; 260];
        for h in 0..jc.nhist as usize {
            let len = histogram(jb, &mut counts)?;
            if len > per as u32 {
                return Err(ERR_CODE);
            }
            let r = ej_alias(&mut counts, len, jc.loga, &mut tabs[h * per..(h + 1) * per]);
            if r != 0 {
                return Err(r);
            }
        }
        jc.ans = tabs;
    }
    Ok(jc)
}

/// the symbol reader (JR)
pub struct Jr<'c> {
    pub jc: &'c Jc,
    pub state: u32,
    pub loge: u32,
    pub win: Vec<u32>, // LZ77 window (1 << 20) or empty
    pub ncopy: u64,
    pub cpos: u64,
    pub ndec: u64,
    pub nspec: u32,
    pub spec: [i32; 120],
}

impl<'c> Jr<'c> {
    /// jr_init: the ANS state, the LZ77 window and special distances (dist_mult != 0)
    pub fn new(jc: &'c Jc, jb: &mut Jb, dist_mult: u32) -> Result<Jr<'c>, i32> {
        let mut jr = Jr { jc, state: ANS_SIG, loge: 0, win: Vec::new(), ncopy: 0, cpos: 0, ndec: 0, nspec: 0, spec: [0; 120] };
        if jc.prefix == 0 {
            jr.state = jb.f(32);
            jr.loge = 12u32.wrapping_sub(jc.loga);
        }
        if jc.lz77 != 0 {
            let mut w: Vec<u32> = Vec::new();
            if w.try_reserve_exact(1 << LZ_WINBITS).is_err() {
                return Err(ERR_NOMEM);
            }
            w.resize(1 << LZ_WINBITS, 0);
            jr.win = w;
            if dist_mult != 0 {
                jr.nspec = 120;
                for k in 0..120 {
                    let dx = tab::SPECIAL[k * 2] as i32;
                    let dy = tab::SPECIAL[k * 2 + 1] as i32;
                    let d = dy.wrapping_mul(dist_mult as i32).wrapping_add(dx);
                    jr.spec[k] = if d < 1 { 1 } else { d };
                }
            }
        }
        Ok(jr)
    }

    /// jr_final: the ANS state is the signature again
    pub fn fin(&self) -> bool {
        self.state == ANS_SIG
    }

    /// jr_token: one entropy coded token of histogram h
    #[inline]
    fn token(&mut self, jb: &mut Jb, h: u32) -> u32 {
        let jc = self.jc;
        if jc.prefix != 0 {
            let v = match jc.pfx.get(h as usize) {
                Some(jp) => psym(jp, jb),
                None => -1,
            };
            if v < 0 {
                jb.err = ERR_CODE;
                return 0;
            }
            return v as u32;
        }
        let st = self.state;
        let res = st & 4095;
        let i = res >> self.loge;
        let pos = res & ((1u32 << self.loge) - 1);
        let k = ((h as usize) << jc.loga) + i as usize;
        let e = jc.ans.get(k).copied().unwrap_or_default();
        let (freq, off, sym) = if pos < e.cutoff as u32 {
            (e.freq0 as u32, pos, i)
        } else {
            (e.freq1 as u32, e.offset as u32 + pos, e.right as u32)
        };
        let mut s = (st >> 12).wrapping_mul(freq).wrapping_add(off);
        if s < 0x10000 {
            s = s << 16 | jb.f(16);
        }
        self.state = s;
        sym
    }

    /// jr_clustered: ReadHybridUintClustered
    pub fn clustered(&mut self, jb: &mut Jb, h: u32) -> u32 {
        let jc = self.jc;
        if !self.win.is_empty() {
            if self.ncopy != 0 {
                return self.copy();
            }
        }
        let tok = self.token(jb, h);
        if !self.win.is_empty() && tok >= jc.lzmin {
            let t = tok - jc.lzmin;
            self.ncopy = hybrid(jb, jc.lzcfg, t).wrapping_add(jc.lzlen) as u64;
            let dt = self.token(jb, jc.lzctx);
            let dc = jc.cfg.get(jc.lzctx as usize).copied().unwrap_or([0; 4]);
            let d = hybrid(jb, dc, dt);
            let mut d = if d < self.nspec { self.spec[d as usize] as u32 as u64 } else { (d - self.nspec).wrapping_add(1) as u64 };
            if d > self.ndec {
                d = self.ndec;
            }
            if d > 1 << LZ_WINBITS {
                d = 1 << LZ_WINBITS;
            }
            self.cpos = self.ndec.wrapping_sub(d);
            if d == 0 {
                let n = self.ncopy.min(1 << LZ_WINBITS) as usize;
                self.win[..n].fill(0);
            }
            return self.copy();
        }
        let c = jc.cfg.get(h as usize).copied().unwrap_or([0; 4]);
        let v = hybrid(jb, c, tok);
        if !self.win.is_empty() {
            self.win[(self.ndec & LZ_MASK) as usize] = v;
            self.ndec = self.ndec.wrapping_add(1);
        }
        v
    }

    #[inline]
    fn copy(&mut self) -> u32 {
        self.ncopy = self.ncopy.wrapping_sub(1);
        let v = self.win[(self.cpos & LZ_MASK) as usize];
        self.cpos = self.cpos.wrapping_add(1);
        self.win[(self.ndec & LZ_MASK) as usize] = v;
        self.ndec = self.ndec.wrapping_add(1);
        v
    }

    /// jr_read: ReadHybridUint( ctx ) through the context map
    #[inline]
    pub fn read(&mut self, jb: &mut Jb, ctx: u32) -> u32 {
        let h = self.jc.cmap.get(ctx as usize).copied().unwrap_or(0) as u32;
        self.clustered(jb, h)
    }
}

/// jr_hybrid: HybridUintConfig::Read (cfg: split exponent, msb, lsb)
#[inline]
pub fn hybrid(jb: &mut Jb, cfg: [u8; 4], token: u32) -> u32 {
    let se = cfg[0] as u32;
    let msb = cfg[1] as u32;
    let lsb = cfg[2] as u32;
    let split = 1u32 << (se & 31);
    if token < split {
        return token;
    }
    let nbits = ((token - split) >> ((msb + lsb) & 31)).wrapping_add(se).wrapping_sub(msb + lsb) & 31;
    let low = token & ((1u32 << (lsb & 31)).wrapping_sub(1));
    let t = token >> (lsb & 31);
    let bits = jb.f(nbits) as u64;
    let hi = 1u32 << (msb & 31);
    let mut d = (hi | (hi.wrapping_sub(1) & t)) as u64;
    d = d << nbits | bits;
    d = d << lsb | low as u64;
    d as u32
}
