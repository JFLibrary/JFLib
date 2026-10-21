// brotli -- Brotli decompression (RFC 7932) of the JPEG reconstruction data (jbrd box) of
// JPEG transcodes                                                        [jxl-plus: no asm]
//
//   decompress(data, max) -> (bytes, consumed)   the whole stream; ERR_CODE for a damaged
//                                                stream, ERR_LIMIT past max output bytes
//
// One pass over the whole input: bit reader, prefix codes (simple and complex), block switches,
// context maps (RLE + move to front), insert-and-copy commands, the distance ring buffer and
// the static dictionary with its 121 transforms (brotli_tab.rs, brotli_dict.bin).

use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_CODE, ERR_LIMIT};

use crate::brotli_tab as T;

static DICT: &[u8] = include_bytes!("brotli_dict.bin");

/// insert length codes: (offset, extra bits)
static INS: [(u32, u32); 24] = [
    (0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 1), (8, 1), (10, 2), (14, 2), (18, 3), (26, 3),
    (34, 4), (50, 4), (66, 5), (98, 5), (130, 6), (194, 7), (322, 8), (578, 9), (1090, 10), (2114, 12),
    (6210, 14), (22594, 24),
];
/// copy length codes: (offset, extra bits)
static COPY: [(u32, u32); 24] = [
    (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0), (10, 1), (12, 1), (14, 2), (18, 2),
    (22, 3), (30, 3), (38, 4), (54, 4), (70, 5), (102, 5), (134, 6), (198, 7), (326, 8), (582, 9),
    (1094, 10), (2118, 24),
];
/// the insert and copy code bases of the 11 command cells (symbol >> 6)
static CELLS: [(u32, u32, bool); 11] = [
    (0, 0, true),
    (0, 8, true),
    (0, 0, false),
    (0, 8, false),
    (8, 0, false),
    (8, 8, false),
    (0, 16, false),
    (16, 0, false),
    (8, 16, false),
    (16, 8, false),
    (16, 16, false),
];
/// the order of the code length code lengths
static CL_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// the fixed code of the code length code lengths (4 bits peeked): length, value
static CL_LEN: [u32; 16] = [2, 2, 2, 3, 2, 2, 2, 4, 2, 2, 2, 3, 2, 2, 2, 4];
static CL_VAL: [u8; 16] = [0, 4, 3, 2, 0, 4, 3, 1, 0, 4, 3, 2, 0, 4, 3, 5];

/// LSB-first bit reader
struct Br<'a> {
    d: &'a [u8],
    pos: usize, // bits
}

impl Br<'_> {
    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, i32> {
        if n == 0 {
            return Ok(0);
        }
        if self.pos + n as usize > self.d.len() * 8 {
            return Err(ERR_CODE);
        }
        let mut v = 0u32;
        for i in 0..n {
            let p = self.pos + i as usize;
            v |= (((self.d[p >> 3] >> (p & 7)) & 1) as u32) << i;
        }
        self.pos += n as usize;
        Ok(v)
    }
    /// up to n bits without consuming (zeros past the end)
    fn peek(&self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let p = self.pos + i as usize;
            if p >> 3 < self.d.len() {
                v |= (((self.d[p >> 3] >> (p & 7)) & 1) as u32) << i;
            }
        }
        v
    }
    fn skip(&mut self, n: u32) -> Result<(), i32> {
        self.pos += n as usize;
        if self.pos > self.d.len() * 8 { Err(ERR_CODE) } else { Ok(()) }
    }
    /// to the byte boundary; the padding bits must be zero
    fn align(&mut self) -> Result<(), i32> {
        let n = ((8 - (self.pos & 7)) & 7) as u32;
        if self.bits(n)? != 0 {
            return Err(ERR_CODE);
        }
        Ok(())
    }
    /// VarLenUint8
    fn varlen8(&mut self) -> Result<u32, i32> {
        if self.bits(1)? == 0 {
            return Ok(0);
        }
        let n = self.bits(3)?;
        if n == 0 { Ok(1) } else { Ok(self.bits(n)? + (1 << n)) }
    }
}

/// a canonical prefix code: codes read most significant bit first, as in deflate
struct Huff {
    count: [u16; 16],   // codes per length
    symbol: Vec<u16>,   // symbols by (length, value)
    single: Option<u16>, // a one-symbol code (no bits)
}

impl Huff {
    fn new(lens: &[u8]) -> Result<Huff, i32> {
        let mut count = [0u16; 16];
        for &l in lens {
            count[l as usize] += 1;
        }
        let used = lens.len() - count[0] as usize;
        if used == 0 {
            return Err(ERR_CODE);
        }
        if used == 1 {
            let s = lens.iter().position(|&l| l != 0).unwrap_or(0) as u16;
            return Ok(Huff { count, symbol: Vec::new(), single: Some(s) });
        }
        count[0] = 0;
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut symbol = vec![0u16; used];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Huff { count, symbol, single: None })
    }
    fn decode(&self, br: &mut Br) -> Result<u32, i32> {
        if let Some(s) = self.single {
            return Ok(s as u32);
        }
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= br.bits(1)? as i32;
            let c = self.count[len] as i32;
            if code - c < first {
                return Ok(self.symbol[(index + (code - first)) as usize] as u32);
            }
            index += c;
            first += c;
            first <<= 1;
            code <<= 1;
        }
        Err(ERR_CODE)
    }
}

/// a prefix code of an alphabet of `size` symbols
fn read_code(br: &mut Br, size: usize) -> Result<Huff, i32> {
    let mut lens = vec![0u8; size];
    let hskip = br.bits(2)?;
    if hskip == 1 {
        // simple code
        let mut bits = 0u32;
        let mut i = size - 1;
        while i != 0 {
            i >>= 1;
            bits += 1;
        }
        let nsym = br.bits(2)? as usize + 1;
        let mut syms = [0usize; 4];
        for s in syms.iter_mut().take(nsym) {
            *s = br.bits(bits)? as usize;
            if *s >= size {
                return Err(ERR_CODE);
            }
        }
        for a in 0..nsym {
            for b in a + 1..nsym {
                if syms[a] == syms[b] {
                    return Err(ERR_CODE);
                }
            }
        }
        let ls: &[u8] = match nsym {
            1 => &[1],
            2 => &[1, 1],
            3 => &[1, 2, 2],
            _ => {
                if br.bits(1)? == 0 {
                    &[2, 2, 2, 2]
                } else {
                    &[1, 2, 3, 3]
                }
            }
        };
        for k in 0..nsym {
            lens[syms[k]] = ls[k];
        }
        if nsym == 1 {
            return Ok(Huff { count: [0; 16], symbol: Vec::new(), single: Some(syms[0] as u16) });
        }
        return Huff::new(&lens);
    }
    // complex code: the code length code, then the code lengths
    let mut cl = [0u8; 18];
    let mut space = 32i32;
    let mut num = 0;
    for &o in CL_ORDER.iter().skip(hskip as usize) {
        let p = br.peek(4) as usize;
        br.skip(CL_LEN[p])?;
        let v = CL_VAL[p];
        cl[o] = v;
        if v != 0 {
            space -= 32 >> v;
            num += 1;
            if space <= 0 {
                break;
            }
        }
    }
    if !(num == 1 || space == 0) {
        return Err(ERR_CODE);
    }
    let clh = Huff::new(&cl)?;
    let mut sym = 0usize;
    let mut prev = 8u8;
    let mut repeat = 0usize;
    let mut repeat_len = 0u8;
    let mut space = 32768i32;
    while sym < size && space > 0 {
        let c = clh.decode(br)? as u8;
        if c < 16 {
            repeat = 0;
            lens[sym] = c;
            sym += 1;
            if c != 0 {
                prev = c;
                space -= 32768 >> c;
            }
        } else {
            let (eb, nl) = if c == 16 { (2, prev) } else { (3, 0) };
            if repeat_len != nl {
                repeat = 0;
                repeat_len = nl;
            }
            let old = repeat;
            if repeat > 0 {
                repeat -= 2;
                repeat <<= eb;
            }
            repeat += br.bits(eb)? as usize + 3;
            let delta = repeat - old;
            if sym + delta > size {
                return Err(ERR_CODE);
            }
            for l in lens.iter_mut().skip(sym).take(delta) {
                *l = repeat_len;
            }
            sym += delta;
            if repeat_len != 0 {
                space -= (delta as i32) * (32768 >> repeat_len);
            }
        }
    }
    if space != 0 {
        return Err(ERR_CODE);
    }
    Huff::new(&lens)
}

/// a context map of `size` entries over `ntrees` trees
fn read_cmap(br: &mut Br, size: usize, ntrees: usize) -> Result<Vec<u8>, i32> {
    let mut map = vec![0u8; size];
    if ntrees < 2 {
        return Ok(map);
    }
    let rlemax = if br.bits(1)? != 0 { br.bits(4)? as usize + 1 } else { 0 };
    let h = read_code(br, ntrees + rlemax)?;
    let mut i = 0;
    while i < size {
        let c = h.decode(br)? as usize;
        if c == 0 {
            map[i] = 0;
            i += 1;
        } else if c <= rlemax {
            let reps = (1usize << c) + br.bits(c as u32)? as usize;
            if i + reps > size {
                return Err(ERR_CODE);
            }
            i += reps; // (zeros)
        } else {
            map[i] = (c - rlemax) as u8;
            i += 1;
        }
    }
    if br.bits(1)? != 0 {
        // inverse move to front
        let mut mtf: Vec<u8> = (0..=255u8).collect();
        for v in map.iter_mut() {
            let idx = *v as usize;
            let val = mtf[idx];
            *v = val;
            mtf.copy_within(0..idx, 1);
            mtf[0] = val;
        }
    }
    Ok(map)
}

/// block switching of one category (literals, commands, distances)
struct Blocks {
    n: u32,
    types: Option<Huff>,
    lens: Option<Huff>,
    cur: u32,
    last: u32,
    second: u32,
    left: u32,
}

impl Blocks {
    fn read(br: &mut Br) -> Result<Blocks, i32> {
        let n = br.varlen8()? + 1;
        let mut b = Blocks { n, types: None, lens: None, cur: 0, last: 0, second: 1, left: u32::MAX };
        if n >= 2 {
            b.types = Some(read_code(br, n as usize + 2)?);
            let l = read_code(br, 26)?;
            b.left = block_len(br, &l)?;
            b.lens = Some(l);
        }
        Ok(b)
    }
    /// the next block's type when the current one is used up
    fn next(&mut self, br: &mut Br) -> Result<(), i32> {
        if self.left == 0 {
            let (Some(t), Some(l)) = (self.types.as_ref(), self.lens.as_ref()) else { return Err(ERR_CODE) };
            let c = t.decode(br)?;
            let mut ty = match c {
                0 => self.second,
                1 => self.last + 1,
                _ => c - 2,
            };
            if ty >= self.n {
                ty -= self.n;
            }
            self.second = self.last;
            self.last = ty;
            self.cur = ty;
            self.left = block_len(br, l)?;
        }
        self.left -= 1;
        Ok(())
    }
}

fn block_len(br: &mut Br, h: &Huff) -> Result<u32, i32> {
    let c = h.decode(br)? as usize;
    let (o, e) = T::BLOCK_LEN[c];
    Ok(o as u32 + br.bits(e as u32)?)
}

/// to upper case of the UTF-8 character at p (RFC 7932 ToUpperCase): its length
fn upper(p: &mut [u8]) -> usize {
    if p[0] < 0xC0 {
        if p[0].is_ascii_lowercase() {
            p[0] ^= 32;
        }
        return 1;
    }
    if p[0] < 0xE0 {
        if p.len() > 1 {
            p[1] ^= 32;
        }
        return 2;
    }
    if p.len() > 2 {
        p[2] ^= 5;
    }
    3
}

/// a static dictionary word with transform t appended to out
fn dict_word(out: &mut Vec<u8>, len: usize, idx: usize, t: usize) {
    let o = T::OFFSETS[len] as usize + idx * len;
    let mut w: Vec<u8> = DICT[o..o + len].to_vec();
    let [pre, ty, suf] = T::TRANSFORMS[t];
    let ty = ty as usize;
    let ps = |id: u8| -> &'static [u8] {
        let s = T::PREFIX_SUFFIX_MAP[id as usize] as usize;
        let n = T::PREFIX_SUFFIX[s] as usize;
        &T::PREFIX_SUFFIX[s + 1..s + 1 + n]
    };
    out.extend_from_slice(ps(pre));
    if (1..=9).contains(&ty) {
        w.truncate(len.saturating_sub(ty));
    } else if (12..=20).contains(&ty) {
        let k = (ty - 11).min(w.len());
        w.drain(..k);
    } else if ty == 10 && !w.is_empty() {
        upper(&mut w);
    } else if ty == 11 {
        let mut i = 0;
        while i < w.len() {
            i += upper(&mut w[i..]);
        }
    }
    out.extend_from_slice(&w);
    out.extend_from_slice(ps(suf));
}

/// decompress a whole Brotli stream -> (output, bytes of input used)
pub fn decompress(data: &[u8], max: usize) -> Result<(Vec<u8>, usize), i32> {
    let mut br = Br { d: data, pos: 0 };
    // window size
    let wbits = if br.bits(1)? == 0 {
        16
    } else {
        let n = br.bits(3)?;
        if n != 0 {
            17 + n
        } else {
            let m = br.bits(3)?;
            if m == 1 {
                return Err(ERR_CODE); // large window: not in RFC 7932
            }
            if m != 0 { 8 + m } else { 17 }
        }
    };
    let window = (1usize << wbits) - 16;
    let mut out: Vec<u8> = Vec::new();
    let mut dist_rb = [16usize, 15, 11, 4]; // the last distance last
    let mut rb_idx = 3usize; // index of the last distance
    loop {
        let last = br.bits(1)? != 0;
        if last && br.bits(1)? != 0 {
            break; // ISLASTEMPTY
        }
        let nib = br.bits(2)?;
        if nib == 3 {
            // metadata block
            if br.bits(1)? != 0 {
                return Err(ERR_CODE);
            }
            let nb = br.bits(2)?;
            let mut skip = 0usize;
            if nb > 0 {
                skip = br.bits(8 * nb)? as usize + 1;
            }
            br.align()?;
            br.skip(8 * skip as u32)?;
            if last {
                break;
            }
            continue;
        }
        let mlen = br.bits(4 * (nib + 4))? as usize + 1;
        if out.len() + mlen > max {
            return Err(ERR_LIMIT);
        }
        if !last && br.bits(1)? != 0 {
            // uncompressed
            br.align()?;
            let p = br.pos >> 3;
            if p + mlen > data.len() {
                return Err(ERR_CODE);
            }
            out.extend_from_slice(&data[p..p + mlen]);
            br.pos += 8 * mlen;
            continue;
        }
        let mut bl = Blocks::read(&mut br)?;
        let mut bi = Blocks::read(&mut br)?;
        let mut bd = Blocks::read(&mut br)?;
        let npostfix = br.bits(2)?;
        let ndirect = br.bits(4)? << npostfix;
        let mut cmode = vec![0u8; bl.n as usize];
        for m in cmode.iter_mut() {
            *m = br.bits(2)? as u8;
        }
        let ntl = br.varlen8()? as usize + 1;
        let cmap_l = read_cmap(&mut br, 64 * bl.n as usize, ntl)?;
        let ntd = br.varlen8()? as usize + 1;
        let cmap_d = read_cmap(&mut br, 4 * bd.n as usize, ntd)?;
        let mut lit_h = Vec::with_capacity(ntl);
        for _ in 0..ntl {
            lit_h.push(read_code(&mut br, 256)?);
        }
        let mut cmd_h = Vec::with_capacity(bi.n as usize);
        for _ in 0..bi.n {
            cmd_h.push(read_code(&mut br, 704)?);
        }
        let dsize = 16 + ndirect as usize + (48usize << npostfix);
        let mut dist_h = Vec::with_capacity(ntd);
        for _ in 0..ntd {
            dist_h.push(read_code(&mut br, dsize)?);
        }
        let mut left = mlen;
        while left > 0 {
            bi.next(&mut br)?;
            let cmd = cmd_h[bi.cur as usize].decode(&mut br)?;
            let (ib, cb, d0) = CELLS[(cmd >> 6) as usize];
            let ic = (ib + ((cmd >> 3) & 7)) as usize;
            let cc = (cb + (cmd & 7)) as usize;
            let ilen = INS[ic].0 as usize + br.bits(INS[ic].1)? as usize;
            let clen = COPY[cc].0 as usize + br.bits(COPY[cc].1)? as usize;
            for _ in 0..ilen {
                bl.next(&mut br)?;
                let n = out.len();
                let p1 = if n > 0 { out[n - 1] } else { 0 } as usize;
                let p2 = if n > 1 { out[n - 2] } else { 0 } as usize;
                let lut = &T::CONTEXT_LUT[(cmode[bl.cur as usize] as usize) << 9..];
                let ctx = (lut[p1] | lut[256 + p2]) as usize;
                let tree = cmap_l[bl.cur as usize * 64 + ctx] as usize;
                let lit = lit_h.get(tree).ok_or(ERR_CODE)?.decode(&mut br)?;
                out.push(lit as u8);
                left -= 1;
                if left == 0 {
                    break;
                }
            }
            if left == 0 {
                break;
            }
            let (dist, dcode) = if d0 {
                (dist_rb[rb_idx], 0usize)
            } else {
                bd.next(&mut br)?;
                let dctx = if clen > 4 { 3 } else { clen - 2 };
                let tree = cmap_d[bd.cur as usize * 4 + dctx] as usize;
                let dc = dist_h.get(tree).ok_or(ERR_CODE)?.decode(&mut br)? as usize;
                let d = if dc < 16 {
                    let r = |k: usize| dist_rb[(rb_idx + 4 - k) & 3] as i64;
                    static IDX: [usize; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
                    static OFF: [i64; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];
                    let v = r(IDX[dc]) + OFF[dc];
                    if v <= 0 {
                        return Err(ERR_CODE);
                    }
                    v as usize
                } else if dc < 16 + ndirect as usize {
                    dc - 15
                } else {
                    let x = dc - ndirect as usize - 16;
                    let nb = 1 + (x >> (npostfix + 1));
                    let hcode = x >> npostfix;
                    let lcode = x & ((1usize << npostfix) - 1);
                    let off = ((2 + (hcode & 1)) << nb) - 4;
                    ((off + br.bits(nb as u32)? as usize) << npostfix) + lcode + ndirect as usize + 1
                };
                (d, dc)
            };
            let maxd = out.len().min(window);
            if dist > maxd {
                // a static dictionary word
                if !(4..=24).contains(&clen) {
                    return Err(ERR_CODE);
                }
                let nb = T::SIZE_BITS[clen] as usize;
                let id = dist - maxd - 1;
                let idx = id & ((1usize << nb) - 1);
                let t = id >> nb;
                if t >= T::TRANSFORMS.len() || nb == 0 {
                    return Err(ERR_CODE);
                }
                let before = out.len();
                dict_word(&mut out, clen, idx, t);
                let n = out.len() - before;
                if n > left {
                    return Err(ERR_CODE);
                }
                left -= n;
            } else {
                if clen > left {
                    return Err(ERR_CODE);
                }
                if dcode != 0 {
                    rb_idx = (rb_idx + 1) & 3;
                    dist_rb[rb_idx] = dist;
                }
                let s = out.len() - dist;
                for k in 0..clen {
                    let v = out[s + k];
                    out.push(v);
                }
                left -= clen;
            }
        }
        if last {
            break;
        }
    }
    Ok((out, br.pos.div_ceil(8)))
}
