// enc -- the .pdn writer                                     [src/img/pdn.s, encoding]
//
//   encode(doc, opt) / encode_ex(doc, opt, flags) -> file bytes   pdn_encode / pdn_encode_ex
//   encode_flat(sf, opt, out)          pdn_encode_flat: img_encode's entry (one-layer document)
//
// The writer reproduces paint.net's object graph and id numbering (the class records, library
// names and version come from the template block of pdn.s, see template.rs); pixel chunks of
// 256 KiB are gzip members (deflate level IO_EFFORT, default 1) with XFL 0 / OS 10 as
// paint.net writes them, compressed in batches on the thread pool (the assembly's Z1
// pe_pchunk: one deflate state and gather buffer per thread) and written in chunk order --
// the same bytes as one gzip member per chunk.  A full chunk of zero bytes reuses the first
// such member (identical bytes, compressed once).  Layer blend modes go to
// LayerProperties.blendMode; BitmapLayerProperties.blendOp is a NormalBlendOp as in every
// paint.net 5.1 file seen, or with PDNE_BLENDOP the UserBlendOps class of the mode.  The
// metadata items EXIF 296 / 282 / 283 are rewritten from the unit / dots per unit, 34675 from
// the ICC profile; every other item is kept.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use gfx::{FMT_ARGB, FMT_PARGB};
use img_core::deflate::DeflateState;
use img_core::gzip::gzip_encode_to;
use img_core::meta::get_row;
use img_core::resample::{RS_FANT, resize};
use img_core::{
    ERR_CANCEL, ERR_DIMS, ERR_LIMIT, ERR_NOMEM, ERR_UNSUPPORTED, IMG_MAXDIM, IOF_EMBED_ICC, ImgOpt, Surface,
};
use rt::sync::SpinLock;

use crate::nrbf::Nw;
use crate::template;
use crate::{
    DocRef, LayerRef, PDN_MAXLAYERS, PDNB_XOR, PDNE_BLENDOP, PDU_INCH, attr_int, b64enc, b64len, cstr, flatten, utoa,
};

const PDN_CHUNK: usize = 262_144; // MemoryBlock chunk size paint.net writes
const PDN_CHUNK_SHIFT: u32 = 18;
const PDN_THUMB: u32 = 256; // thumbnail: longer side
const PZ_MAXBATCH: usize = 64;
const PZ_OSTRIDE: usize = PDN_CHUNK + PDN_CHUNK / 512 + 128; // deflate_bound(PDN_CHUNK)
const PZF_ZERO: u32 = 1; // a full chunk of zero bytes
const PZF_CACHED: u32 = 2; // ... written as the cached member

const EXIF_TAG: &[u8] = b"$exif.tag";

// per-layer object ids (EI_*)
#[derive(Clone, Copy, Default)]
struct Ei {
    bl: i32,
    blp: i32,
    s: i32,
    lp: i32,
    bo: i32,
    mb: i32,
    kv: i32,
    kvnew: bool,
}

// one thread's state of the parallel chunk compressor: deflate state, and for layers that
// need conversion the gather buffer and the cached converted row (PS_*)
#[derive(Default)]
struct PzSlot {
    st: Option<Box<DeflateState>>,
    scr: Vec<u8>,
    row: Vec<u8>,
    rowkey: Option<(usize, usize)>, // (layer, y) of the cached row
}

// the parallel encode context (PZ record), made on first use
struct Pz {
    nslot: usize,
    batch: usize,
    slots: Vec<SpinLock<PzSlot>>,
    out: Vec<SpinLock<Vec<u8>>>, // batch output: PZ_OSTRIDE each
    res: Vec<(AtomicI64, AtomicU32)>, // {bytes / ERR_*, PZF_*}
}

// the encode context (pdn_encode_ex's EC frame)
struct Ec<'d, 'a> {
    doc: &'d DocRef<'a>,
    flags: u32, // PDNE_*
    level: i32,
    cancel: Option<&'d AtomicU32>,
    w: u32,
    h: u32,
    n: u32,
    nw: Nw,
    ids: Vec<Ei>,
    empty: i32,         // id of the shared empty metadata array (0 = none yet)
    genp: [Vec<u8>; 4], // generated values: EXIF 296, 282, 283, 34675
    kmax: i32,          // largest "$exif.tagN" index in doc.nvc (-1 none)
    thumb: Vec<u8>,
    zgz: Option<Vec<u8>>, // gzip member of PDN_CHUNK zero bytes (cached)
    pz: Option<Pz>,
}

/// write the document as a .pdn file (opt: IO_EFFORT = deflate level 1..9, default 1)
/// [pdn_encode]
pub fn encode(doc: &DocRef, opt: Option<&ImgOpt>) -> Result<Vec<u8>, i32> {
    encode_ex(doc, opt, 0)
}

/// pdn_encode with PDNE_* flags; errors: ERR_DIMS (sizes, a layer of another size, no
/// layers), ERR_UNSUPPORTED (surface format), ERR_LIMIT (XML header > 16 MB, the composite
/// over the default pixel budget), ERR_CANCEL  [pdn_encode_ex]
pub fn encode_ex(doc: &DocRef, opt: Option<&ImgOpt>, flags: u32) -> Result<Vec<u8>, i32> {
    let mut level = 1;
    let mut cancel = None;
    if let Some(o) = opt {
        cancel = o.cancel;
        if o.effort.wrapping_sub(1) <= 8 {
            level = o.effort as i32;
        }
    }
    // validate the document
    let (w, h, n) = (doc.w, doc.h, doc.layers.len() as u32);
    if w.wrapping_sub(1) > IMG_MAXDIM - 1 || h.wrapping_sub(1) > IMG_MAXDIM - 1 || n.wrapping_sub(1) > PDN_MAXLAYERS - 1
    {
        return Err(ERR_DIMS);
    }
    for l in &doc.layers {
        if l.sf.w as u32 != w || l.sf.h as u32 != h || l.sf.px.is_empty() {
            return Err(ERR_DIMS);
        }
        if l.sf.fmt > FMT_ARGB {
            return Err(ERR_UNSUPPORTED);
        }
    }
    let mut ec = Ec {
        doc,
        flags,
        level,
        cancel,
        w,
        h,
        n,
        nw: Nw::new(),
        ids: vec![Ei::default(); n as usize],
        empty: 0,
        genp: Default::default(),
        kmax: -1,
        thumb: Vec::new(),
        zgz: None,
        pz: None,
    };
    ec.genmeta();
    ec.thumb()?;
    ec.xml()?;
    ec.nrbf()?;
    for i in 0..n as usize {
        ec.payload(i)?;
    }
    if ec.nw.err != 0 {
        return Err(ec.nw.err);
    }
    ec.nw.take().ok_or(ERR_NOMEM)
}

impl Ec<'_, '_> {
    #[inline]
    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }

    // the generated metadata values (EXIF 296 / 282 / 283 from the unit / dots per unit,
    // 34675 from the ICC profile) and kmax = the largest "$exif.tagN" index of doc.nvc
    // [pe_genmeta]
    fn genmeta(&mut self) {
        let d = self.doc;
        // unit
        let unit = if d.unit.wrapping_sub(1) <= 2 { d.unit } else { PDU_INCH };
        self.genp[0] = genitem(b"<exif id=\"296\" len=\"2\" type=\"3\" value=\"", &unit.to_le_bytes()[..2]);
        // resolutions
        for (k, x) in [d.dpux, d.dpuy].into_iter().enumerate() {
            let (num, den) = rational(x);
            let mut b = [0u8; 8];
            b[..4].copy_from_slice(&num.to_le_bytes());
            b[4..].copy_from_slice(&den.to_le_bytes());
            let pre: &[u8] = if k == 0 {
                b"<exif id=\"282\" len=\"8\" type=\"5\" value=\""
            } else {
                b"<exif id=\"283\" len=\"8\" type=\"5\" value=\""
            };
            self.genp[1 + k] = genitem(pre, &b);
        }
        if !d.icc.is_empty() {
            // "<exif id="34675" len="N" type="7" value="
            let mut v = Vec::with_capacity(b64len(d.icc.len()) + 64);
            v.extend_from_slice(b"<exif id=\"34675\" len=\"");
            utoa(d.icc.len() as u32, &mut v);
            v.extend_from_slice(b"\" type=\"7\" value=\"");
            b64push(d.icc, &mut v);
            v.extend_from_slice(b"\" />");
            self.genp[3] = v;
        }
        self.kmax = kmax(d.nvc);
    }

    // the flattened image scaled to fit 256 x 256 (area average, RS_FANT) as a PNG
    // [pe_thumb]
    fn thumb(&mut self) -> Result<(), i32> {
        let comp = flatten(self.doc)?;
        // thumbnail size: the longer side 256, the other floor(side * 256 / longer)
        let (mut tw, mut th) = (self.w, self.h);
        let long = tw.max(th);
        if long > PDN_THUMB {
            tw = ((tw << 8) / long).max(1);
            th = ((th << 8) / long).max(1);
        }
        let mut t = Surface::new(tw as i32, th as i32, FMT_PARGB);
        if t.px.is_empty() {
            return Err(ERR_NOMEM);
        }
        let r = resize(&comp, &mut t, RS_FANT, 0);
        if r != 0 {
            return Err(r);
        }
        self.thumb = img_png::enc::encode_classic(&t, 6).ok_or(ERR_NOMEM)?;
        Ok(())
    }

    // "PDN3", u24 length and the XML header with the thumbnail  [pe_xml]
    fn xml(&mut self) -> Result<(), i32> {
        let nw = &mut self.nw;
        nw.bytes(b"PDN3\0\0\0"); // + 3 length bytes (patched)
        let mut s = Vec::with_capacity(160);
        s.extend_from_slice(b"<pdnImage width=\"");
        utoa(self.w, &mut s);
        s.extend_from_slice(b"\" height=\"");
        utoa(self.h, &mut s);
        s.extend_from_slice(b"\" layers=\"");
        utoa(self.n, &mut s);
        s.extend_from_slice(b"\" savedWithVersion=\"");
        s.extend_from_slice(template::VERSTR);
        s.extend_from_slice(b"\"><custom><thumb png=\"");
        nw.bytes(&s);
        let Some(dst) = nw.reserve(b64len(self.thumb.len())) else { return Err(nw.err) };
        b64enc(&self.thumb, dst);
        nw.bytes(b"\" /></custom></pdnImage>");
        if nw.err != 0 {
            return Err(nw.err);
        }
        let xl = nw.buf.len() - 7;
        if xl > 0xFF_FFFF {
            return Err(ERR_LIMIT);
        }
        nw.buf[4] = xl as u8;
        nw.buf[5] = (xl >> 8) as u8;
        nw.buf[6] = (xl >> 16) as u8;
        nw.bytes(&[0, 1]); // 00 01
        if nw.err != 0 { Err(nw.err) } else { Ok(()) }
    }

    // the blend mode 0..13 of layer l for the blendOp class (always Normal unless
    // PDNE_BLENDOP)  [pe_layer_blend]
    fn layer_blend(&self, l: &LayerRef) -> usize {
        if self.flags & PDNE_BLENDOP == 0 || l.blend > PDNB_XOR { 0 } else { l.blend as usize }
    }

    // the KeyValuePair<string,string>[] record `id` with the items of the blob (BinaryArray,
    // rank 1, element type from the template)  [pe_kvarr]
    fn kvarr(&mut self, id: i32, blob: &[u8], with_gen: bool) {
        let n = items(&mut self.nw, false, &self.genp, self.kmax, blob, with_gen);
        let mut h = [0u8; 14];
        h[0] = 7;
        h[1..5].copy_from_slice(&id.to_le_bytes());
        h[5] = 0;
        h[6..10].copy_from_slice(&1u32.to_le_bytes());
        h[10..14].copy_from_slice(&n.to_le_bytes());
        self.nw.bytes(&h);
        self.nw.bytes(&template::KVP_TAIL);
        items(&mut self.nw, true, &self.genp, self.kmax, blob, with_gen);
    }

    // the NRBF object graph, ids and record order exactly as .NET's BinaryFormatter writes
    // paint.net's Document  [pe_nrbf]
    fn nrbf(&mut self) -> Result<(), i32> {
        let doc = self.doc;
        let (w, h, n) = (self.w, self.h, self.n);
        self.nw.header(1);
        let id = self.nw.id(); // Document = 1
        self.nw.class(&template::DOC, id);
        self.nw.u8(0); // isDisposed
        let ll = self.nw.id();
        self.nw.ref_(ll);
        self.nw.i32(w);
        self.nw.i32(h);
        let ver = self.nw.id();
        self.nw.ref_(ver);
        let cnt = items(&mut self.nw, false, &self.genp, self.kmax, doc.nvc, true);
        let docarr = self.nw.id();
        if cnt == 0 {
            self.empty = docarr; // the shared empty array
        }
        self.nw.ref_(docarr);
        // LayerList
        self.nw.class(&template::LL, ll);
        self.nw.id(); // parent: a reference consumes an id
        self.nw.ref_(1);
        let itm = self.nw.id();
        self.nw.ref_(itm);
        self.nw.i32(n);
        self.nw.i32(n);
        // Version
        self.nw.class(&template::VER, ver);
        self.nw.bytes(&template::VERSION);
        // document metadata array
        self.kvarr(docarr, doc.nvc, true);
        // ArrayList buffer: capacity 4, 8, 16, ...
        let mut cap = 4u32;
        while cap < n {
            cap += cap;
        }
        self.nw.u8(16);
        self.nw.i32(itm as u32);
        self.nw.i32(cap);
        for i in 0..n as usize {
            let id = self.nw.id();
            self.ids[i].bl = id;
            self.nw.ref_(id);
        }
        self.nw.nulls(cap.wrapping_sub(n) as i32);
        // BitmapLayer records
        for i in 0..n as usize {
            self.nw.class(&template::BL, self.ids[i].bl);
            let blp = self.nw.id();
            self.ids[i].blp = blp;
            self.nw.ref_(blp);
            let s = self.nw.id();
            self.ids[i].s = s;
            self.nw.ref_(s);
            self.nw.u8(0);
            self.nw.i32(w);
            self.nw.i32(h);
            let lp = self.nw.id();
            self.ids[i].lp = lp;
            self.nw.ref_(lp);
        }
        // per layer: BitmapLayerProperties, Surface, LayerProperties
        for i in 0..n as usize {
            let l = doc.layers[i];
            let b = self.layer_blend(&l);
            self.nw.class(template::BLP[b], self.ids[i].blp);
            let bo = self.nw.id();
            self.ids[i].bo = bo;
            self.nw.ref_(bo);
            // Surface
            self.nw.class(&template::SURF, self.ids[i].s);
            self.nw.i32(w);
            self.nw.i32(h);
            self.nw.i32(w << 2);
            let mb = self.nw.id();
            self.ids[i].mb = mb;
            self.nw.ref_(mb);
            // LayerProperties
            self.nw.class(&template::LP, self.ids[i].lp);
            self.nw.str_(cstr(l.name));
            let cnt = items(&mut self.nw, false, &self.genp, self.kmax, l.meta, false);
            self.ids[i].kvnew = true;
            if cnt != 0 {
                self.ids[i].kv = self.nw.id();
            } else if self.empty != 0 {
                self.ids[i].kv = self.empty;
                self.ids[i].kvnew = false;
                self.nw.id(); // consumed by the reference
            } else {
                let id = self.nw.id();
                self.empty = id;
                self.ids[i].kv = id;
            }
            self.nw.ref_(self.ids[i].kv);
            self.nw.u8((l.visible != 0) as u32);
            self.nw.u8((l.isbg != 0) as u32);
            self.nw.u8(l.opacity as u32);
            let id = self.nw.id().wrapping_neg();
            self.nw.class(&template::BM, id);
            self.nw.i32(if l.blend > PDNB_XOR { 0 } else { l.blend as u32 });
        }
        // per layer: UserBlendOp, MemoryBlock, (new) metadata array
        for i in 0..n as usize {
            let l = doc.layers[i];
            let b = self.layer_blend(&l);
            self.nw.class(template::BLENDOP[b], self.ids[i].bo);
            self.nw.class(&template::MB, self.ids[i].mb);
            self.nw.i64((w as u64 * 4) * h as u64);
            self.nw.u8(0); // hasParent
            self.nw.u8(1); // deferred
            if self.ids[i].kvnew {
                self.kvarr(self.ids[i].kv, l.meta, false);
            }
        }
        self.nw.end();
        if self.nw.err != 0 { Err(self.nw.err) } else { Ok(()) }
    }

    // the parallel encode context: a slot per thread (thr_count() + 1), the batch output
    // buffers and the results  [pe_pz]
    fn pz(&mut self) -> &Pz {
        self.pz.get_or_insert_with(|| {
            let nslot = (plat::thr_count() as usize).min(plat::THR_MAX) + 1; // workers + the caller
            let batch = (nslot * 2).min(PZ_MAXBATCH); // two chunks per thread and batch
            Pz {
                nslot,
                batch,
                slots: (0..nslot).map(|_| SpinLock::new(PzSlot::default())).collect(),
                out: (0..batch).map(|_| SpinLock::new(vec![0u8; PZ_OSTRIDE])).collect(),
                res: (0..batch).map(|_| (AtomicI64::new(0), AtomicU32::new(0))).collect(),
            }
        })
    }

    // the deferred MemoryBlock of layer i: format 0, chunk size 256 KiB (big endian), chunks
    // 0, 1, ... as gzip members, compressed in batches on the thread pool and written here in
    // index order  [pe_payload]
    fn payload(&mut self, i: usize) -> Result<(), i32> {
        let l = self.doc.layers[i];
        let sf = l.sf;
        let w4 = self.w as usize * 4;
        let total = w4 * self.h as usize;
        // direct: FMT_ARGB without row padding
        let direct = sf.fmt == FMT_ARGB && sf.stride as usize == w4;
        self.nw.u8(0);
        self.nw.be32(PDN_CHUNK as u32);
        let level = self.level;
        let cancel = self.cancel;
        self.pz();
        let chunks = (total + PDN_CHUNK - 1) >> PDN_CHUNK_SHIFT;
        let mut base = 0usize;
        let len0 = self.nw.buf.len(); // the output before the first member
        while base < chunks {
            if self.nw.err != 0 {
                return Err(self.nw.err);
            }
            if self.cancelled() {
                return Err(ERR_CANCEL);
            }
            let pz = self.pz.as_ref().unwrap_or_else(|| unreachable!());
            let nb = (chunks - base).min(pz.batch);
            let zgz = self.zgz.is_some(); // (only written between batches)
            let nslot = pz.nslot;
            let item = |j: usize, _: &plat::JobCtl| {
                let (rb, rf) = &pz.res[j];
                rb.store(ERR_CANCEL as i64, Ordering::Relaxed);
                rf.store(0, Ordering::Relaxed);
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
                    return;
                }
                let off = (base + j) << PDN_CHUNK_SHIFT;
                let n = (total - off).min(PDN_CHUNK);
                let mut g = pz.slots[plat::thr_self().min(nslot - 1)].lock();
                let PzSlot { st, scr, row, rowkey } = &mut *g;
                let src: &[u8] = if direct {
                    &sf.px[off..off + n]
                } else {
                    if scr.is_empty() {
                        *scr = vec![0u8; PDN_CHUNK];
                        *row = vec![0u8; w4];
                        *rowkey = None;
                    }
                    gather(sf, i, off, n, w4, scr, row, rowkey);
                    &scr[..n]
                };
                // a full chunk of zero bytes (transparent area): the cached member if there
                let zero = n == PDN_CHUNK && iszero(src);
                if zero && zgz {
                    rb.store(0, Ordering::Relaxed);
                    rf.store(PZF_ZERO | PZF_CACHED, Ordering::Relaxed);
                    return;
                }
                let s = st.get_or_insert_with(DeflateState::new);
                let mut o = pz.out[j].lock();
                match gzip_encode_to(Some(&mut **s), src, level, &mut o[..]) {
                    Err(e) => rb.store(e as i64, Ordering::Relaxed),
                    Ok(m) => {
                        o[8] = 0; // XFL 0, OS 10: as paint.net writes them
                        o[9] = 0x0A;
                        rb.store(m as i64, Ordering::Relaxed);
                        if zero {
                            rf.store(PZF_ZERO, Ordering::Relaxed);
                        }
                    }
                }
            };
            let _ = plat::parallel_for(nb, &item);
            // the members in index order
            for j in 0..nb {
                let r = pz.res[j].0.load(Ordering::Relaxed);
                let fl = pz.res[j].1.load(Ordering::Relaxed);
                if r < 0 {
                    return Err(r as i32); // the first failing chunk (ERR_CANCEL)
                }
                let mut hd = [0u8; 8];
                hd[..4].copy_from_slice(&((base + j) as u32).to_be_bytes());
                if fl & PZF_CACHED != 0 {
                    let z = self.zgz.as_deref().unwrap_or(&[]);
                    hd[4..].copy_from_slice(&(z.len() as u32).to_be_bytes());
                    self.nw.bytes(&hd);
                    self.nw.bytes(z);
                    continue;
                }
                let o = pz.out[j].lock();
                let m = &o[..r as usize];
                if fl & PZF_ZERO != 0 && self.zgz.is_none() {
                    self.zgz = Some(m.to_vec()); // the first zero chunk: keep its member
                }
                hd[4..].copy_from_slice(&(r as u32).to_be_bytes());
                self.nw.bytes(&hd);
                self.nw.bytes(m);
            }
            if base == 0 && nb < chunks && self.nw.err == 0 {
                // perf-inflate: room for the rest of the layer at the ratio of this first
                // batch (+2 %), so that the output grows once instead of doubling (every
                // doubling copies the file so far into new pages); a failed reservation only
                // leaves the doubling
                let raw = nb << PDN_CHUNK_SHIFT;
                let out = (self.nw.buf.len() - len0) as u64;
                let rest = (total - raw) as u64;
                let est = out * rest / raw as u64 + rest / 50 + 8 * (chunks - nb) as u64 + 65536;
                let free = (self.nw.buf.capacity() - self.nw.buf.len()) as u64;
                if est > free && est <= usize::MAX as u64 / 2 {
                    let _ = self.nw.buf.try_reserve_exact(est as usize);
                }
            }
            base += nb;
        }
        if self.nw.err != 0 { Err(self.nw.err) } else { Ok(()) }
    }
}

// bytes [off, off + n) of layer sf's pixels as straight BGRA rows of w4 bytes (no padding)
// into scr (n <= PDN_CHUNK); other formats through a cached converted row  [pe_gather]
#[allow(clippy::too_many_arguments)]
fn gather(sf: &Surface, li: usize, off: usize, n: usize, w4: usize, scr: &mut [u8], row: &mut [u8], rowkey: &mut Option<(usize, usize)>) {
    let mut y = off / w4;
    let mut x = off % w4;
    let mut d = 0usize;
    let mut left = n;
    let ss = sf.stride as usize;
    while left > 0 {
        let take = (w4 - x).min(left);
        if sf.fmt == FMT_ARGB {
            scr[d..d + take].copy_from_slice(&sf.px[y * ss + x..y * ss + x + take]);
        } else {
            if *rowkey != Some((li, y)) {
                get_row(sf, y, row, FMT_ARGB);
                *rowkey = Some((li, y));
            }
            scr[d..d + take].copy_from_slice(&row[x..x + take]);
        }
        d += take;
        left -= take;
        y += 1;
        x = 0;
    }
}

// the n bytes at p are all zero (n a multiple of 64)  [pdn_iszero]
fn iszero(p: &[u8]) -> bool {
    for blk in p.chunks(4096) {
        let mut acc = 0u64;
        for c in blk.chunks_exact(8) {
            acc |= u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        }
        if acc != 0 {
            return false;
        }
    }
    true
}

// append the base64 of src
fn b64push(src: &[u8], v: &mut Vec<u8>) {
    let l = v.len();
    v.resize(l + b64len(src.len()), 0);
    b64enc(src, &mut v[l..]);
}

// prefix + base64 of bytes + `" />`  [pe_genitem]
fn genitem(prefix: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(prefix.len() + 16);
    v.extend_from_slice(prefix);
    b64push(bytes, &mut v);
    v.extend_from_slice(b"\" />");
    v
}

// dots per unit -> numerator / denominator: 96.0 -> 96 / 1, 37.795 -> 37795 / 1000 (4
// decimals, trailing zeros removed); values outside (0, 400000] or NaN -> 96 / 1
// [pe_rational]
fn rational(x: f64) -> (u32, u32) {
    if !(x > 0.0) || x > 400000.0 {
        return (96, 1);
    }
    let mut v = rt::fx::cvtd64(x * 10000.0) as u64; // rounded (MXCSR nearest)
    let mut den = 10000u32;
    while den != 1 {
        if v % 10 != 0 {
            break;
        }
        v /= 10;
        den /= 10;
    }
    let num = v as u32;
    if num == 0 { (96, 1) } else { (num, den) }
}

// the items {u32 klen, key, u32 vlen, value} of a metadata blob (stops where one does not fit)
fn blob_items(blob: &[u8], mut f: impl FnMut(&[u8], &[u8]) -> bool) {
    let mut p = 0usize;
    let len = blob.len();
    loop {
        if len - p < 4 {
            return;
        }
        let kl = u32::from_le_bytes([blob[p], blob[p + 1], blob[p + 2], blob[p + 3]]) as usize;
        p += 4;
        if len - p < kl {
            return;
        }
        let key = &blob[p..p + kl];
        p += kl;
        if len - p < 4 {
            return;
        }
        let vl = u32::from_le_bytes([blob[p], blob[p + 1], blob[p + 2], blob[p + 3]]) as usize;
        p += 4;
        if len - p < vl {
            return;
        }
        let val = &blob[p..p + vl];
        p += vl;
        if !f(key, val) {
            return;
        }
    }
}

// the largest N of the keys "$exif.tagN..." in the blob (-1 none)  [pe_kmax]
fn kmax(blob: &[u8]) -> i32 {
    let mut km = -1i32;
    blob_items(blob, |key, _| {
        if key.len() < 10 || &key[..9] != EXIF_TAG {
            return true;
        }
        let mut v = 0u32; // digits after "$exif.tag"
        let mut k = 9;
        while k < key.len() {
            let dg = key[k].wrapping_sub(b'0');
            if dg > 9 {
                break;
            }
            if v >= 100_000_000 {
                return true; // absurd index: ignore the key
            }
            v = v * 10 + dg as u32;
            k += 1;
        }
        if k != 9 && v as i32 > km {
            km = v as i32;
        }
        true
    });
    km
}

// the generated-value slot 0..3 of an item that is the EXIF tag 296 / 282 / 283 / 34675
// with a generated value, else None  [pe_genidx]
fn genidx(genp: &[Vec<u8>; 4], key: &[u8], val: &[u8]) -> Option<usize> {
    if key.len() < 9 || &key[..9] != EXIF_TAG {
        return None;
    }
    let s = match attr_int(val, b"id=\"") {
        296 => 0,
        282 => 1,
        283 => 2,
        34675 => 3,
        _ => return None,
    };
    if genp[s].is_empty() { None } else { Some(s) }
}

// write: a KeyValuePair struct {key, value} (inline, negative id)  [pe_kv]
fn kv(nw: &mut Nw, write: bool, key: &[u8], val: &[u8]) {
    if !write {
        return;
    }
    let id = nw.id().wrapping_neg();
    nw.class(&template::KVP, id);
    nw.str_(key);
    nw.str_(val);
}

// walk the metadata blob -> number of items; with_gen: EXIF items 296 / 282 / 283 / 34675 get the
// generated values (later duplicates of the same tag are dropped) and the generated values
// that were not in the blob are appended as "$exif.tagN[0]" (N = kmax + 1 ...); write: emit
// every item as a KeyValuePair struct  [pe_items]
fn items(nw: &mut Nw, write: bool, genp: &[Vec<u8>; 4], kmax: i32, blob: &[u8], with_gen: bool) -> u32 {
    let mut count = 0u32;
    let mut done = 0u32;
    blob_items(blob, |key, val| {
        let mut v = val;
        if with_gen && let Some(s) = genidx(genp, key, val) {
            if done & 1 << s != 0 {
                return true; // second item with that tag: dropped
            }
            done |= 1 << s;
            v = &genp[s];
        }
        kv(nw, write, key, v);
        count += 1;
        true
    });
    if with_gen {
        let mut next = kmax.wrapping_add(1) as u32;
        let mut key = Vec::with_capacity(32);
        for (s, g) in genp.iter().enumerate() {
            if g.is_empty() || done & 1 << s != 0 {
                continue;
            }
            key.clear();
            key.extend_from_slice(EXIF_TAG);
            utoa(next, &mut key);
            key.extend_from_slice(b"[0]");
            next = next.wrapping_add(1);
            kv(nw, write, &key, g);
            count += 1;
        }
    }
    count
}

/// img_encode's entry: sf as the background layer of a one-layer document (resolution from
/// opt.meta dpix / dpiy, ICC profile with IOF_EMBED_ICC)  [pdn_encode_flat]
pub fn encode_flat(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let mut doc = DocRef {
        w: sf.w as u32,
        h: sf.h as u32,
        unit: 0,
        dpux: 0.0,
        dpuy: 0.0,
        layers: vec![LayerRef { sf, name: b"Background", opacity: 255, visible: 1, blend: 0, isbg: 1, meta: &[] }],
        nvc: &[],
        icc: &[],
    };
    if let Some(m) = opt.meta {
        if opt.flags & IOF_EMBED_ICC != 0 {
            doc.icc = &m.icc;
        }
        if m.dpix != 0 && m.dpiy != 0 {
            doc.unit = PDU_INCH;
            doc.dpux = m.dpix as f64 / 65536.0;
            doc.dpuy = m.dpiy as f64 / 65536.0;
        }
    }
    match encode_ex(&doc, Some(opt), 0) {
        Ok(v) => {
            *out = v;
            0
        }
        Err(e) => e,
    }
}
