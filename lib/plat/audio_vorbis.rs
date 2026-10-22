// audio_vorbis -- (audio.rs) Ogg Vorbis files into f32 samples (Vorbis I specification)
//
//   decode(bytes) -> Result<(rate, channels, samples), AudioError>
//
// Ogg: the pages of the first logical stream (CRC-32 checked: a damaged page is Format; a file
// cut off ends with its last whole page), packets joined across pages, the last granule
// position trims the end.  Vorbis: the identification, comment (skipped) and
// setup headers -- codebooks (Huffman trees from the code lengths, VQ lookup types 1 and 2),
// floor type 1, residue types 0 / 1 / 2, mappings with channel coupling and submaps, modes --
// then each audio packet: floors, residues, inverse coupling, the floor curve, the inverse MDCT
// (a DCT-IV through a complex FFT of a quarter of the block), the power-complementary window
// and overlap-add.  Channels come out in WAVE order (Vorbis puts the centre second: L C R ..).  Floor type 0 (unused by encoders since 2002) is refused, as are files
// that would decode to more than 10 minutes (a Sound holds everything decoded; longer audio
// belongs in a Stream).  An audio packet that ends early decodes as far as it goes (the
// specification's rule); damaged headers are AudioError::Format.

use alloc::vec;
use alloc::vec::Vec;

use super::AudioError;

const FMT: AudioError = AudioError::Format;
const PI: f64 = core::f64::consts::PI;
const MAX_SECONDS: u64 = 600;

// ---------------------------------------------------------------- Ogg

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, e) in t.iter_mut().enumerate() {
        let mut r = (i as u32) << 24;
        for _ in 0..8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
        }
        *e = r;
    }
    t
}

/// the packets of the first logical stream and its last granule position (-1: none)
fn ogg_packets(b: &[u8]) -> Result<(Vec<Vec<u8>>, i64), AudioError> {
    let crc = crc_table();
    let mut packets = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut serial0: Option<u32> = None;
    let mut granule = -1i64;
    let mut pos = 0;
    while pos + 27 <= b.len() {
        if &b[pos..pos + 4] != b"OggS" || b[pos + 4] != 0 {
            return if packets.len() >= 3 { Ok((packets, granule)) } else { Err(FMT) };
        }
        let htype = b[pos + 5];
        let gp = i64::from_le_bytes(b[pos + 6..pos + 14].try_into().map_err(|_| FMT)?);
        let serial = u32::from_le_bytes(b[pos + 14..pos + 18].try_into().map_err(|_| FMT)?);
        let stored = u32::from_le_bytes(b[pos + 22..pos + 26].try_into().map_err(|_| FMT)?);
        let nseg = b[pos + 26] as usize;
        let table = b.get(pos + 27..pos + 27 + nseg).ok_or(FMT)?;
        let body_len: usize = table.iter().map(|&x| x as usize).sum();
        let end = pos + 27 + nseg + body_len;
        let Some(page) = b.get(pos..end) else { break }; // cut off: what came before
        let mut c = 0u32;
        for (i, &x) in page.iter().enumerate() {
            let x = if (22..26).contains(&i) { 0 } else { x };
            c = (c << 8) ^ crc[((c >> 24) as u8 ^ x) as usize];
        }
        if c != stored {
            return Err(FMT);
        }
        let first = serial0.is_none();
        if first {
            if htype & 2 == 0 {
                return Err(FMT);
            }
            serial0 = Some(serial);
        }
        if Some(serial) == serial0 {
            if htype & 1 == 0 {
                cur.clear(); // a packet that never ended
            }
            let mut at = 27 + nseg;
            for &l in table {
                cur.extend_from_slice(&page[at..at + l as usize]);
                at += l as usize;
                if l < 255 {
                    packets.push(core::mem::take(&mut cur));
                }
            }
            if gp != -1 {
                granule = gp;
            }
            if htype & 4 != 0 {
                break;
            }
        }
        pos = end;
    }
    Ok((packets, granule))
}

// ---------------------------------------------------------------- bits

struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(b: &'a [u8]) -> Bits<'a> {
        Bits { b, pos: 0 }
    }

    /// n <= 32 bits, least significant first (None: the packet ended)
    fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if self.pos + n as usize > self.b.len() * 8 {
            self.pos = self.b.len() * 8;
            return None;
        }
        let mut v = 0u64;
        let mut got = 0;
        while got < n {
            let byte = self.b[self.pos / 8] as u64;
            let off = (self.pos % 8) as u32;
            let take = (8 - off).min(n - got);
            v |= ((byte >> off) & ((1 << take) - 1)) << got;
            got += take;
            self.pos += take as usize;
        }
        Some(v as u32)
    }

    fn bit(&mut self) -> Option<bool> {
        self.read(1).map(|v| v != 0)
    }
}

fn ilog(x: u32) -> u32 {
    32 - x.leading_zeros()
}

fn float32_unpack(x: u32) -> f32 {
    let mantissa = (x & 0x1F_FFFF) as f64;
    let exp = ((x & 0x7FE0_0000) >> 21) as i32 - 788;
    let m = if x & 0x8000_0000 != 0 { -mantissa } else { mantissa };
    (m * pow2(exp)) as f32
}

fn pow2(e: i32) -> f64 {
    let mut r = 1.0f64;
    let (mut b, mut k) = (if e < 0 { 0.5 } else { 2.0 }, e.unsigned_abs());
    while k > 0 {
        if k & 1 != 0 {
            r *= b;
        }
        b *= b;
        k >>= 1;
    }
    r
}

// ---------------------------------------------------------------- codebooks

struct Codebook {
    dims: usize,
    entries: usize,
    /// the Huffman tree: [child for 0, child for 1]; >= 0 a node, < 0 the entry -(e + 1)
    tree: Vec<[i32; 2]>,
    /// the only entry of a one-entry book (its code has `single_len` bits)
    single: Option<(u32, u32)>,
    /// the VQ vectors (entries x dims), empty without a lookup
    vq: Vec<f32>,
}

impl Codebook {
    fn read(r: &mut Bits) -> Result<Codebook, AudioError> {
        if r.read(24) != Some(0x56_4342) {
            return Err(FMT);
        }
        let dims = r.read(16).ok_or(FMT)? as usize;
        let entries = r.read(24).ok_or(FMT)? as usize;
        if dims == 0 || entries == 0 {
            return Err(FMT);
        }
        let mut lens = vec![0u8; entries]; // 0: unused
        if r.bit().ok_or(FMT)? {
            // ordered
            let mut len = r.read(5).ok_or(FMT)? + 1;
            let mut cur = 0usize;
            while cur < entries {
                let n = r.read(ilog((entries - cur) as u32)).ok_or(FMT)? as usize;
                if cur + n > entries || len > 32 {
                    return Err(FMT);
                }
                lens[cur..cur + n].fill(len as u8);
                cur += n;
                len += 1;
            }
        } else {
            let sparse = r.bit().ok_or(FMT)?;
            for l in lens.iter_mut() {
                if !sparse || r.bit().ok_or(FMT)? {
                    *l = (r.read(5).ok_or(FMT)? + 1) as u8;
                }
            }
        }
        let (tree, single) = huffman(&lens)?;
        let lookup = r.read(4).ok_or(FMT)?;
        let mut vq = Vec::new();
        if lookup == 1 || lookup == 2 {
            let min = float32_unpack(r.read(32).ok_or(FMT)?);
            let delta = float32_unpack(r.read(32).ok_or(FMT)?);
            let bits = r.read(4).ok_or(FMT)? + 1;
            let seq = r.bit().ok_or(FMT)?;
            let nvals = if lookup == 1 { lookup1_values(entries, dims) } else { entries.checked_mul(dims).ok_or(FMT)? };
            if nvals == 0 || nvals > 1 << 20 || entries.checked_mul(dims).is_none_or(|n| n > 1 << 22) {
                return Err(FMT);
            }
            let mut mult = Vec::with_capacity(nvals);
            for _ in 0..nvals {
                mult.push(r.read(bits).ok_or(FMT)? as f32);
            }
            vq = vec![0.0f32; entries * dims];
            for e in 0..entries {
                let mut last = 0.0f32;
                let mut div = 1usize;
                for d in 0..dims {
                    let off = if lookup == 1 { (e / div) % nvals } else { e * dims + d };
                    let v = mult[off] * delta + min + last;
                    vq[e * dims + d] = v;
                    if seq {
                        last = v;
                    }
                    if lookup == 1 {
                        div = div.saturating_mul(nvals);
                    }
                }
            }
        } else if lookup != 0 {
            return Err(FMT);
        }
        Ok(Codebook { dims, entries, tree, single, vq })
    }

    /// one entry number (None: the packet ended or no such code)
    fn scalar(&self, r: &mut Bits) -> Option<usize> {
        if let Some((e, len)) = self.single {
            r.read(len)?;
            return Some(e as usize);
        }
        let mut node = 0usize;
        loop {
            let next = self.tree.get(node)?[r.read(1)? as usize];
            if next < 0 {
                return Some((-next - 1) as usize);
            }
            if next == 0 {
                return None;
            }
            node = next as usize;
        }
    }

    /// one VQ vector
    fn vector(&self, r: &mut Bits) -> Option<&[f32]> {
        let e = self.scalar(r)?;
        self.vq.get(e * self.dims..(e + 1) * self.dims)
    }
}

/// the largest r with r^dims <= entries
fn lookup1_values(entries: usize, dims: usize) -> usize {
    let mut r = 1usize;
    loop {
        let mut p = 1usize;
        let mut over = false;
        for _ in 0..dims {
            p = match p.checked_mul(r + 1) {
                Some(v) => v,
                None => {
                    over = true;
                    break;
                }
            };
        }
        if over || p > entries {
            return r;
        }
        r += 1;
    }
}

/// the decoding tree of the code lengths (codes given in entry order, each the lowest free
/// one of its length, as the specification builds them) -> (tree, a single entry)
fn huffman(lens: &[u8]) -> Result<(Vec<[i32; 2]>, Option<(u32, u32)>), AudioError> {
    let used: Vec<usize> = (0..lens.len()).filter(|&i| lens[i] != 0).collect();
    if used.len() == 1 {
        return Ok((Vec::new(), Some((used[0] as u32, lens[used[0]] as u32))));
    }
    // the lowest free code of each length, MSB-aligned in 32 bits (0: none)
    let mut avail = [0u64; 33];
    let mut tree: Vec<[i32; 2]> = vec![[0, 0]];
    for (n, &i) in used.iter().enumerate() {
        let len = lens[i] as usize;
        let code: u64 = if n == 0 {
            for (l, a) in avail.iter_mut().enumerate().take(len + 1).skip(1) {
                *a = 1u64 << (32 - l);
            }
            0
        } else {
            let mut z = len;
            while z > 0 && avail[z] == 0 {
                z -= 1;
            }
            if z == 0 {
                return Err(FMT); // over-full: no code left
            }
            let res = avail[z];
            avail[z] = 0;
            for (y, a) in avail.iter_mut().enumerate().take(len + 1).skip(z + 1) {
                *a = res + (1u64 << (32 - y));
            }
            res
        };
        // insert: the code's bits from the top
        let mut node = 0usize;
        for k in 0..len {
            let bit = ((code >> (31 - k)) & 1) as usize;
            if k + 1 == len {
                if tree[node][bit] != 0 {
                    return Err(FMT);
                }
                tree[node][bit] = -(i as i32) - 1;
            } else {
                let next = tree[node][bit];
                if next < 0 {
                    return Err(FMT);
                }
                if next == 0 {
                    tree.push([0, 0]);
                    let id = (tree.len() - 1) as i32;
                    tree[node][bit] = id;
                    node = id as usize;
                } else {
                    node = next as usize;
                }
            }
        }
    }
    Ok((tree, None))
}

// ---------------------------------------------------------------- setup

struct Floor1 {
    partition_class: Vec<u8>,
    class_dims: Vec<u8>,
    class_subs: Vec<u8>,
    class_master: Vec<u8>,
    sub_books: Vec<Vec<i32>>,
    mult: u32,
    xs: Vec<u32>,
    // the X list's order and neighbours (precomputed)
    sorted: Vec<usize>,
    low: Vec<usize>,
    high: Vec<usize>,
}

struct Residue {
    typ: u16,
    begin: usize,
    end: usize,
    psize: usize,
    classes: usize,
    classbook: usize,
    books: Vec<[i32; 8]>,
}

struct Mapping {
    coupling: Vec<(usize, usize)>,
    mux: Vec<usize>,
    floors: Vec<usize>,
    residues: Vec<usize>,
}

struct Mode {
    long: bool,
    mapping: usize,
}

struct Setup {
    books: Vec<Codebook>,
    floors: Vec<Floor1>,
    residues: Vec<Residue>,
    maps: Vec<Mapping>,
    modes: Vec<Mode>,
}

fn read_setup(p: &[u8], ch: usize) -> Result<Setup, AudioError> {
    let mut r = Bits::new(p);
    if r.read(8) != Some(5) || p.get(1..7) != Some(b"vorbis") {
        return Err(FMT);
    }
    r.pos = 7 * 8;
    let nbooks = r.read(8).ok_or(FMT)? + 1;
    let mut books = Vec::new();
    for _ in 0..nbooks {
        books.push(Codebook::read(&mut r)?);
    }
    let ntimes = r.read(6).ok_or(FMT)? + 1;
    for _ in 0..ntimes {
        if r.read(16) != Some(0) {
            return Err(FMT);
        }
    }
    let nb = books.len();
    let book = |v: u32| -> Result<usize, AudioError> { if (v as usize) < nb { Ok(v as usize) } else { Err(FMT) } };
    let nfloors = r.read(6).ok_or(FMT)? + 1;
    let mut floors = Vec::new();
    for _ in 0..nfloors {
        if r.read(16) != Some(1) {
            return Err(FMT); // floor 0 or an unknown type
        }
        let parts = r.read(5).ok_or(FMT)? as usize;
        let mut partition_class = Vec::with_capacity(parts);
        for _ in 0..parts {
            partition_class.push(r.read(4).ok_or(FMT)? as u8);
        }
        let nclasses = partition_class.iter().map(|&c| c as usize + 1).max().unwrap_or(0);
        let (mut class_dims, mut class_subs, mut class_master, mut sub_books) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..nclasses {
            class_dims.push((r.read(3).ok_or(FMT)? + 1) as u8);
            let subs = r.read(2).ok_or(FMT)? as u8;
            class_subs.push(subs);
            class_master.push(if subs != 0 { book(r.read(8).ok_or(FMT)?)? as u8 } else { 0 });
            let mut sb = Vec::new();
            for _ in 0..(1 << subs) {
                let v = r.read(8).ok_or(FMT)? as i32 - 1;
                if v >= nb as i32 {
                    return Err(FMT);
                }
                sb.push(v);
            }
            sub_books.push(sb);
        }
        let mult = r.read(2).ok_or(FMT)? + 1;
        let rangebits = r.read(4).ok_or(FMT)?;
        let mut xs = vec![0u32, 1 << rangebits];
        for &c in &partition_class {
            for _ in 0..class_dims[c as usize] {
                xs.push(r.read(rangebits).ok_or(FMT)?);
            }
        }
        if xs.len() > 65 {
            return Err(FMT);
        }
        let mut sorted: Vec<usize> = (0..xs.len()).collect();
        sorted.sort_by_key(|&i| xs[i]);
        if sorted.windows(2).any(|w| xs[w[0]] == xs[w[1]]) {
            return Err(FMT);
        }
        let (mut low, mut high) = (vec![0; xs.len()], vec![0; xs.len()]);
        for i in 2..xs.len() {
            let (mut lo, mut hi) = (0usize, 1usize);
            let (mut lv, mut hv) = (0u32, u32::MAX);
            for j in 0..i {
                if xs[j] < xs[i] && xs[j] >= lv {
                    lv = xs[j];
                    lo = j;
                }
                if xs[j] > xs[i] && xs[j] <= hv {
                    hv = xs[j];
                    hi = j;
                }
            }
            low[i] = lo;
            high[i] = hi;
        }
        floors.push(Floor1 { partition_class, class_dims, class_subs, class_master, sub_books, mult, xs, sorted, low, high });
    }
    let nres = r.read(6).ok_or(FMT)? + 1;
    let mut residues = Vec::new();
    for _ in 0..nres {
        let typ = r.read(16).ok_or(FMT)? as u16;
        if typ > 2 {
            return Err(FMT);
        }
        let begin = r.read(24).ok_or(FMT)? as usize;
        let end = r.read(24).ok_or(FMT)? as usize;
        let psize = r.read(24).ok_or(FMT)? as usize + 1;
        let classes = r.read(6).ok_or(FMT)? as usize + 1;
        let classbook = book(r.read(8).ok_or(FMT)?)?;
        let mut cascade = Vec::new();
        for _ in 0..classes {
            let low = r.read(3).ok_or(FMT)?;
            let high = if r.bit().ok_or(FMT)? { r.read(5).ok_or(FMT)? } else { 0 };
            cascade.push(high * 8 + low);
        }
        let mut rbooks = Vec::new();
        for c in cascade {
            let mut b = [-1i32; 8];
            for (j, slot) in b.iter_mut().enumerate() {
                if c & (1 << j) != 0 {
                    let v = book(r.read(8).ok_or(FMT)?)?;
                    if books[v].vq.is_empty() {
                        return Err(FMT);
                    }
                    *slot = v as i32;
                }
            }
            rbooks.push(b);
        }
        if books[classbook].entries == 0 {
            return Err(FMT);
        }
        residues.push(Residue { typ, begin, end, psize, classes, classbook, books: rbooks });
    }
    let nmaps = r.read(6).ok_or(FMT)? + 1;
    let mut maps = Vec::new();
    for _ in 0..nmaps {
        if r.read(16) != Some(0) {
            return Err(FMT);
        }
        let submaps = if r.bit().ok_or(FMT)? { r.read(4).ok_or(FMT)? as usize + 1 } else { 1 };
        let mut coupling = Vec::new();
        if r.bit().ok_or(FMT)? {
            let steps = r.read(8).ok_or(FMT)? + 1;
            let bits = ilog(ch as u32 - 1);
            for _ in 0..steps {
                let m = r.read(bits).ok_or(FMT)? as usize;
                let a = r.read(bits).ok_or(FMT)? as usize;
                if m == a || m >= ch || a >= ch {
                    return Err(FMT);
                }
                coupling.push((m, a));
            }
        }
        if r.read(2) != Some(0) {
            return Err(FMT);
        }
        let mut mux = vec![0usize; ch];
        if submaps > 1 {
            for m in mux.iter_mut() {
                *m = r.read(4).ok_or(FMT)? as usize;
                if *m >= submaps {
                    return Err(FMT);
                }
            }
        }
        let (mut fl, mut rs) = (Vec::new(), Vec::new());
        for _ in 0..submaps {
            r.read(8).ok_or(FMT)?;
            let f = r.read(8).ok_or(FMT)? as usize;
            let s = r.read(8).ok_or(FMT)? as usize;
            if f >= floors.len() || s >= residues.len() {
                return Err(FMT);
            }
            fl.push(f);
            rs.push(s);
        }
        maps.push(Mapping { coupling, mux, floors: fl, residues: rs });
    }
    let nmodes = r.read(6).ok_or(FMT)? + 1;
    let mut modes = Vec::new();
    for _ in 0..nmodes {
        let long = r.bit().ok_or(FMT)?;
        if r.read(16) != Some(0) || r.read(16) != Some(0) {
            return Err(FMT);
        }
        let mapping = r.read(8).ok_or(FMT)? as usize;
        if mapping >= maps.len() {
            return Err(FMT);
        }
        modes.push(Mode { long, mapping });
    }
    if r.bit() != Some(true) {
        return Err(FMT);
    }
    Ok(Setup { books, floors, residues, maps, modes })
}

// ---------------------------------------------------------------- floor 1

fn render_point(x0: i32, y0: i32, x1: i32, y1: i32, x: i32) -> i32 {
    let dy = y1 - y0;
    let adx = x1 - x0;
    let off = dy.abs() * (x - x0) / adx;
    if dy < 0 { y0 - off } else { y0 + off }
}

fn render_line(x0: i32, y0: i32, x1: i32, y1: i32, v: &mut [i32]) {
    let dy = y1 - y0;
    let adx = x1 - x0;
    if adx <= 0 {
        return;
    }
    let base = dy / adx;
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let ady = dy.abs() - base.abs() * adx;
    let (mut y, mut err) = (y0, 0);
    if let Some(s) = v.get_mut(x0 as usize) {
        *s = y;
    }
    for x in x0 + 1..x1 {
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        if let Some(s) = v.get_mut(x as usize) {
            *s = y;
        }
    }
}

impl Floor1 {
    /// the floor's Y values from the packet -> None: unused (no energy) or the packet ended
    fn decode(&self, r: &mut Bits, books: &[Codebook]) -> Option<Vec<i32>> {
        if !r.bit()? {
            return None;
        }
        let range = [256u32, 128, 86, 64][self.mult as usize - 1];
        let bits = ilog(range - 1);
        let mut y = vec![0i32; self.xs.len()];
        y[0] = r.read(bits)? as i32;
        y[1] = r.read(bits)? as i32;
        let mut off = 2;
        for &c in &self.partition_class {
            let c = c as usize;
            let (cdim, cbits) = (self.class_dims[c] as usize, self.class_subs[c] as u32);
            let csub = (1u32 << cbits) - 1;
            let mut cval = if cbits > 0 { books[self.class_master[c] as usize].scalar(r)? as u32 } else { 0 };
            for j in 0..cdim {
                let b = self.sub_books[c][(cval & csub) as usize];
                cval >>= cbits;
                y[off + j] = if b >= 0 { books[b as usize].scalar(r)? as i32 } else { 0 };
            }
            off += cdim;
        }
        Some(y)
    }

    /// the curve (n2 values of the inverse dB table) from decoded Y values
    fn curve(&self, y: &[i32], n2: usize, out: &mut [f32], table: &[f32; 256]) {
        let range = [256i32, 128, 86, 64][self.mult as usize - 1];
        let k = self.xs.len();
        let mut fy = vec![0i32; k];
        let mut step2 = vec![false; k];
        fy[0] = y[0];
        fy[1] = y[1];
        step2[0] = true;
        step2[1] = true;
        for i in 2..k {
            let (lo, hi) = (self.low[i], self.high[i]);
            let pred = render_point(self.xs[lo] as i32, fy[lo], self.xs[hi] as i32, fy[hi], self.xs[i] as i32);
            let val = y[i];
            let (highroom, lowroom) = (range - pred, pred);
            let room = if highroom < lowroom { highroom * 2 } else { lowroom * 2 };
            if val != 0 {
                step2[lo] = true;
                step2[hi] = true;
                step2[i] = true;
                fy[i] = if val >= room {
                    if highroom > lowroom { val - lowroom + pred } else { pred - val + highroom - 1 }
                } else if val & 1 == 1 {
                    pred - (val + 1) / 2
                } else {
                    pred + val / 2
                };
            } else {
                fy[i] = pred;
            }
        }
        let mut line = vec![0i32; n2];
        let m = self.mult as i32;
        let (mut lx, mut ly) = (0i32, fy[self.sorted[0]] * m);
        let (mut hx, mut hy) = (0i32, ly);
        for &i in &self.sorted[1..] {
            if step2[i] {
                hy = fy[i] * m;
                hx = self.xs[i] as i32;
                render_line(lx, ly, hx, hy, &mut line);
                lx = hx;
                ly = hy;
            }
        }
        if (hx as usize) < n2 {
            render_line(hx, hy, n2 as i32, hy, &mut line);
        }
        for (o, &v) in out.iter_mut().zip(&line) {
            *o *= table[v.clamp(0, 255) as usize];
        }
    }
}

// ---------------------------------------------------------------- residues

impl Residue {
    /// decode into the vectors (each n2 long) of the channels not marked `skip`
    fn decode(&self, r: &mut Bits, books: &[Codebook], vecs: &mut [Vec<f32>], skip: &[bool], n2: usize) {
        let ch = vecs.len();
        if self.typ == 2 {
            if skip.iter().all(|&s| s) {
                return;
            }
            let mut one = vec![vec![0.0f32; n2 * ch]];
            self.decode_inner(r, books, &mut one, &[false], n2 * ch, 1);
            for (i, &v) in one[0].iter().enumerate() {
                vecs[i % ch][i / ch] += v;
            }
            return;
        }
        self.decode_inner(r, books, vecs, skip, n2, self.typ);
    }

    fn decode_inner(&self, r: &mut Bits, books: &[Codebook], vecs: &mut [Vec<f32>], skip: &[bool], size: usize, format: u16) {
        let (begin, end) = (self.begin.min(size), self.end.min(size));
        if end <= begin {
            return;
        }
        let cb = &books[self.classbook];
        let per = cb.dims;
        let parts = (end - begin) / self.psize;
        let ch = vecs.len();
        let mut classes = vec![vec![0usize; parts + per]; ch];
        for pass in 0..8 {
            let mut p = 0;
            while p < parts {
                if pass == 0 {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let Some(mut t) = cb.scalar(r) else { return };
                        for i in (0..per).rev() {
                            classes[j][p + i] = t % self.classes;
                            t /= self.classes;
                        }
                    }
                }
                let mut i = 0;
                while i < per && p < parts {
                    for j in 0..ch {
                        if skip[j] {
                            continue;
                        }
                        let b = self.books[classes[j][p]][pass];
                        if b < 0 {
                            continue;
                        }
                        let book = &books[b as usize];
                        let off = begin + p * self.psize;
                        let v = &mut vecs[j];
                        if format == 0 {
                            let step = self.psize / book.dims;
                            for s in 0..step {
                                let Some(e) = book.vector(r) else { return };
                                for (d, &x) in e.iter().enumerate() {
                                    if let Some(o) = v.get_mut(off + s + d * step) {
                                        *o += x;
                                    }
                                }
                            }
                        } else {
                            let mut k = 0;
                            while k < self.psize {
                                let Some(e) = book.vector(r) else { return };
                                for &x in e {
                                    if let Some(o) = v.get_mut(off + k) {
                                        *o += x;
                                    }
                                    k += 1;
                                }
                            }
                        }
                    }
                    i += 1;
                    p += 1;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the inverse MDCT

/// the inverse MDCT of one block size: a DCT-IV of n/2 through a complex FFT of n/4
struct Imdct {
    n: usize,
    tw: Vec<(f64, f64)>,
    roots: Vec<(f64, f64)>,
    rev: Vec<usize>,
}

impl Imdct {
    fn new(n: usize) -> Imdct {
        let m = n / 2;
        let q = n / 4;
        let tw = (0..q).map(|k| rt::x87::sincos(-PI * (k as f64 + 0.125) / m as f64)).map(|(c, s)| (c, s)).collect();
        let roots = (0..q / 2).map(|k| rt::x87::sincos(-2.0 * PI * k as f64 / q as f64)).collect();
        let bits = q.trailing_zeros();
        let rev = (0..q).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - bits) }).collect();
        Imdct { n, tw, roots, rev }
    }

    /// x: n/2 coefficients -> out: n samples
    fn run(&self, x: &[f32], out: &mut [f32]) {
        let (n, m, q) = (self.n, self.n / 2, self.n / 4);
        // z[k] = (x[2k] + i x[m-1-2k]) * tw[k], in bit-reversed order
        let mut re = vec![0.0f64; q];
        let mut im = vec![0.0f64; q];
        for k in 0..q {
            let (a, b) = (x[2 * k] as f64, x[m - 1 - 2 * k] as f64);
            let (c, s) = self.tw[k];
            let j = self.rev[k];
            re[j] = a * c - b * s;
            im[j] = a * s + b * c;
        }
        // radix-2 FFT
        let mut len = 2;
        while len <= q {
            let half = len / 2;
            let stride = q / len;
            for start in (0..q).step_by(len) {
                for k in 0..half {
                    let (c, s) = self.roots[k * stride];
                    let (i, j) = (start + k, start + k + half);
                    let (tr, ti) = (re[j] * c - im[j] * s, re[j] * s + im[j] * c);
                    re[j] = re[i] - tr;
                    im[j] = im[i] - ti;
                    re[i] += tr;
                    im[i] += ti;
                }
            }
            len *= 2;
        }
        // the DCT-IV: y[2k] = Re(Z[k] tw[k]), y[m-1-2k] = -Im(Z[k] tw[k])
        let mut y = vec![0.0f32; m];
        for k in 0..q {
            let (c, s) = self.tw[k];
            let (a, b) = (re[k], im[k]);
            y[2 * k] = (a * c - b * s) as f32;
            y[m - 1 - 2 * k] = -(a * s + b * c) as f32;
        }
        // unfold: out[i] = Y[i + m/2] with Y's symmetries
        let h = m / 2;
        for i in 0..n {
            let t = i + h;
            out[i] = if t < m {
                y[t]
            } else if t < 2 * m {
                -y[2 * m - 1 - t]
            } else {
                -y[t - 2 * m]
            };
        }
    }
}

/// the slope of the window over len samples (rising)
fn slope(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let s = rt::x87::sin((i as f64 + 0.5) / len as f64 * PI / 2.0);
            rt::x87::sin(PI / 2.0 * s * s) as f32
        })
        .collect()
}

// ---------------------------------------------------------------- decoding

/// an Ogg Vorbis file -> (frames per second, channels, interleaved samples)
pub(crate) fn decode(b: &[u8]) -> Result<(u32, u16, Vec<f32>), AudioError> {
    let (packets, granule) = ogg_packets(b)?;
    if packets.len() < 3 {
        return Err(FMT);
    }
    // identification
    let id = &packets[0];
    if id.len() < 30 || id[0] != 1 || &id[1..7] != b"vorbis" || id[7..11] != [0, 0, 0, 0] {
        return Err(FMT);
    }
    let ch = id[11] as usize;
    let rate = u32::from_le_bytes(id[12..16].try_into().map_err(|_| FMT)?);
    let (bs0, bs1) = (1usize << (id[28] & 15), 1usize << (id[28] >> 4));
    if ch == 0 || ch > 8 || !(1000..=384_000).contains(&rate) || bs0 < 64 || bs1 > 8192 || bs0 > bs1 || id[29] & 1 == 0 {
        return Err(FMT);
    }
    if packets[1].first() != Some(&3) || packets[1].get(1..7) != Some(b"vorbis") {
        return Err(FMT);
    }
    let setup = read_setup(&packets[2], ch)?;
    let mode_bits = ilog(setup.modes.len() as u32 - 1);
    let table: [f32; 256] = core::array::from_fn(|i| rt::x87::exp2((i as f64 - 255.0) * 7.0 / 256.0 * 3.321_928_094_887_362) as f32);
    let imdct = [Imdct::new(bs0), Imdct::new(bs1)];
    let (short_slope, long_slope) = (slope(bs0 / 2), slope(bs1 / 2));
    let max_frames = (rate as u64 * MAX_SECONDS) as usize;
    // the Vorbis channel of each WAVE channel (spec 4.3.9)
    let order: &[usize] = match ch {
        3 => &[0, 2, 1],
        5 => &[0, 2, 1, 3, 4],
        6 => &[0, 2, 1, 5, 3, 4],
        7 => &[0, 2, 1, 6, 5, 3, 4],
        8 => &[0, 2, 1, 7, 5, 6, 3, 4],
        1 => &[0],
        2 => &[0, 1],
        _ => &[0, 1, 2, 3],
    };
    let mut out: Vec<f32> = Vec::new();
    let mut prev: Vec<Vec<f32>> = vec![Vec::new(); ch];
    let mut prev_n = 0usize;
    let mut coef: Vec<Vec<f32>> = vec![Vec::new(); ch];
    let mut pcm = vec![0.0f32; bs1];
    let mut cur: Vec<Vec<f32>> = vec![vec![0.0f32; bs1]; ch];
    for p in &packets[3..] {
        let mut r = Bits::new(p);
        if r.bit() != Some(false) {
            continue; // not an audio packet (or empty)
        }
        let Some(mode) = r.read(mode_bits).and_then(|m| setup.modes.get(m as usize)) else { continue };
        let n = if mode.long { bs1 } else { bs0 };
        let (prev_long, next_long) = if mode.long { (r.bit().unwrap_or(false), r.bit().unwrap_or(false)) } else { (false, false) };
        let map = &setup.maps[mode.mapping];
        let n2 = n / 2;
        // floors
        let mut floor_y: Vec<Option<Vec<i32>>> = Vec::with_capacity(ch);
        for c in 0..ch {
            let f = &setup.floors[map.floors[map.mux[c]]];
            floor_y.push(f.decode(&mut r, &setup.books));
        }
        // channels without energy, unless coupled with one that has
        let mut silent: Vec<bool> = floor_y.iter().map(|f| f.is_none()).collect();
        for &(m, a) in &map.coupling {
            if !silent[m] || !silent[a] {
                silent[m] = false;
                silent[a] = false;
            }
        }
        // residues per submap
        for v in coef.iter_mut() {
            v.clear();
            v.resize(n2, 0.0);
        }
        for (s, &res) in map.residues.iter().enumerate() {
            let chans: Vec<usize> = (0..ch).filter(|&c| map.mux[c] == s).collect();
            let mut vecs: Vec<Vec<f32>> = chans.iter().map(|&c| core::mem::take(&mut coef[c])).collect();
            let skip: Vec<bool> = chans.iter().map(|&c| silent[c]).collect();
            setup.residues[res].decode(&mut r, &setup.books, &mut vecs, &skip, n2);
            for (k, &c) in chans.iter().enumerate() {
                coef[c] = core::mem::take(&mut vecs[k]);
            }
        }
        // inverse coupling
        for &(m, a) in map.coupling.iter().rev() {
            for j in 0..n2 {
                let (mv, av) = (coef[m][j], coef[a][j]);
                let (nm, na) = if mv > 0.0 {
                    if av > 0.0 { (mv, mv - av) } else { (mv + av, mv) }
                } else if av > 0.0 {
                    (mv, mv + av)
                } else {
                    (mv - av, mv)
                };
                coef[m][j] = nm;
                coef[a][j] = na;
            }
        }
        // the floor curve, the inverse MDCT, the window
        let (ls, le, rs, re) = {
            let (mut ls, mut le, mut rs, mut re) = (0, n2, n2, n);
            if mode.long && !prev_long {
                ls = n / 4 - bs0 / 4;
                le = n / 4 + bs0 / 4;
            }
            if mode.long && !next_long {
                rs = n * 3 / 4 - bs0 / 4;
                re = n * 3 / 4 + bs0 / 4;
            }
            (ls, le, rs, re)
        };
        let (lslope, rslope) = (if le - ls == bs0 / 2 { &short_slope } else { &long_slope }, if re - rs == bs0 / 2 { &short_slope } else { &long_slope });
        for c in 0..ch {
            let w = &mut cur[c][..n];
            match &floor_y[c] {
                Some(y) => {
                    let f = &setup.floors[map.floors[map.mux[c]]];
                    f.curve(y, n2, &mut coef[c], &table);
                    imdct[mode.long as usize].run(&coef[c], &mut pcm[..n]);
                    for (i, o) in w.iter_mut().enumerate() {
                        let g = if i < ls || i >= re {
                            0.0
                        } else if i < le {
                            lslope[i - ls]
                        } else if i < rs {
                            1.0
                        } else {
                            rslope[re - 1 - i]
                        };
                        *o = pcm[i] * g;
                    }
                }
                None => w.fill(0.0),
            }
        }
        // overlap-add with the previous block: from its centre to this one's
        if prev_n > 0 {
            let len = prev_n / 4 + n / 4;
            if out.len() / ch + len > max_frames {
                return Err(FMT);
            }
            let shift = n as isize / 4 - prev_n as isize / 4;
            for k in 0..len {
                for &c in order {
                    let tp = prev_n / 2 + k;
                    let tc = k as isize + shift;
                    let a = if tp < prev_n { prev[c][tp] } else { 0.0 };
                    let b = if tc >= 0 && (tc as usize) < n { cur[c][tc as usize] } else { 0.0 };
                    out.push(a + b);
                }
            }
        }
        for c in 0..ch {
            prev[c].clear();
            prev[c].extend_from_slice(&cur[c][..n]);
        }
        prev_n = n;
    }
    if granule >= 0 && (granule as usize) < out.len() / ch {
        out.truncate(granule as usize * ch);
    }
    Ok((rate, ch as u16, out))
}
