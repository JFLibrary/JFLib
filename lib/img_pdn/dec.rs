// dec -- the .pdn reader                                     [src/img/pdn.s, decoding]
//
//   decode(data, dec) / decode_ex(data, dec, flags) -> PdnDoc     pdn_decode / pdn_decode_ex
//   probe(data, meta)                  pdn_probe: header scan for img_probe
//   decode_flat(data, sf, dec, meta)   pdn_decode_flat: img_decode_ex's entry (the composite)
//
// Objects are looked up by member name with fallbacks (3.x: userMetaData
// NameValueCollection, the blend mode from the UserBlendOps class, inline MemoryBlock
// chunks).  Deferred payloads with several chunks are decoded on the thread pool (the
// assembly's Z1 px_par): pass 1 walks the chunk headers in file order exactly like the
// sequential loop and records them, pass 2 decodes the records with plat::parallel_for (one
// inflate state and scratch buffer per thread); the result is the error of the first chunk
// in file order that fails, else the header error that ended pass 1 -- what the sequential
// loop returns.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering};

use gfx::FMT_ARGB;
use img_core::gzip::{gzip_decode, gzip_decode_st, gzip_isize};
use img_core::inflate::InflateState;
use img_core::meta::{Blob, dims_check, mem_check, meta_blob, surf_alloc};
use img_core::{
    ERR_CANCEL, ERR_CHUNK, ERR_DIMS, ERR_HDR, ERR_NOMEM, ERR_TRUNC, ERR_UNSUPPORTED, IMG_MAXDIM, IMG_MAXPX_DEF, IMGD_ICC,
    IMGD_PROBE, IMGF_PDN, IMF_ALPHA, IMF_LOSSLESS, IMM_RGB, IMR_CM, IMR_INCH, IMR_NONE, ImgDec, ImgMeta, Surface,
};
use rt::sync::SpinLock;

use crate::nrbf::{NRT_BYTE, NRVK_OBJ, NRVK_PRIM, Nrbf};
use crate::{PDN_MAXLAYERS, PDND_PROBE, PDNB_XOR, PDU_CM, PDU_INCH, PdnDoc, PdnLayer, attr, attr_int, b64dec, cstr, flatten_owned, utoa};

const PDN_MAXPAD: u64 = 64; // accepted Surface stride padding beyond width * 4
const XD_MAXSCR: u64 = 16_777_216; // larger chunks: sequential decode

// file LayerBlendMode 0..13 -> PDNB_* (= BM_*): identity  [pdn_bm_map]
static BM_MAP: [u8; 14] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
static BO_NAMES: [&[u8]; 14] = [
    b"PaintDotNet.UserBlendOps+NormalBlendOp",
    b"PaintDotNet.UserBlendOps+MultiplyBlendOp",
    b"PaintDotNet.UserBlendOps+AdditiveBlendOp",
    b"PaintDotNet.UserBlendOps+ColorBurnBlendOp",
    b"PaintDotNet.UserBlendOps+ColorDodgeBlendOp",
    b"PaintDotNet.UserBlendOps+ReflectBlendOp",
    b"PaintDotNet.UserBlendOps+GlowBlendOp",
    b"PaintDotNet.UserBlendOps+OverlayBlendOp",
    b"PaintDotNet.UserBlendOps+DifferenceBlendOp",
    b"PaintDotNet.UserBlendOps+NegationBlendOp",
    b"PaintDotNet.UserBlendOps+LightenBlendOp",
    b"PaintDotNet.UserBlendOps+DarkenBlendOp",
    b"PaintDotNet.UserBlendOps+ScreenBlendOp",
    b"PaintDotNet.UserBlendOps+XorBlendOp",
];
const C_DOCUMENT: &[u8] = b"PaintDotNet.Document";
const C_MEMBLOCK: &[u8] = b"PaintDotNet.MemoryBlock";
const C_LPROPS: &[u8] = b"PaintDotNet.Layer+LayerProperties";
const EXIF_TAG: &[u8] = b"$exif.tag";

#[inline]
fn be32(d: &[u8], p: usize) -> u32 {
    u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

// CHECK_SPAN: off + n within len
#[inline]
fn span(off: usize, n: u64, len: usize) -> bool {
    off <= len && ((len - off) as u64) >= n
}

// one thread's state of the parallel chunk decoder (XDS_*)
#[derive(Default)]
struct XdSlot {
    st: Option<Box<InflateState>>,
    scr: Vec<u8>,
}

// the decode context (pdn_decode_ex's PX frame)
struct Px<'a, 'd> {
    data: &'a [u8], // the NRBF data (file or decompressed buffer)
    nb: Nrbf<'a>,
    dec: ImgDec<'d>,
    flags: u32,
    pay: Vec<u64>, // payload offset + 1 of deferred MemoryBlocks, per object
    items: i32,    // layer array object
    w: u32,
    h: u32,
    n: u32,
    seen: Vec<u8>,                // chunk-seen bitmap
    scr: Vec<u8>,                 // chunk scratch (stride conversion)
    st: Option<Box<InflateState>>, // sequential chunks (the assembly: a temporary state each)
    slots: Vec<SpinLock<XdSlot>>, // parallel decoder, made on first use (PX_PD)
}

/// read a .pdn: every layer  [pdn_decode]
pub fn decode(data: &[u8], dec: Option<&ImgDec>) -> Result<PdnDoc, i32> {
    decode_ex(data, dec, 0)
}

/// pdn_decode; flags PDND_PROBE: only size, resolution and metadata (no layers)
/// [pdn_decode_ex]
pub fn decode_ex(data: &[u8], dec: Option<&ImgDec>, flags: u32) -> Result<PdnDoc, i32> {
    let mut d = dec.copied().unwrap_or_default();
    if d.maxpx == 0 {
        d.maxpx = IMG_MAXPX_DEF;
    }
    let mut doc = PdnDoc::default();
    let own = unwrap(data, &mut doc, &d)?;
    let nd: &[u8] = match &own {
        Unwrapped::Plain(o) => &data[*o..],
        Unwrapped::Own(v) => v,
    };
    let nb = Nrbf::parse(nd, d.maxmem)?;
    let mut px = Px {
        data: nd,
        nb,
        dec: d,
        flags,
        pay: Vec::new(),
        items: -1,
        w: 0,
        h: 0,
        n: 0,
        seen: Vec::new(),
        scr: Vec::new(),
        st: None,
        slots: Vec::new(),
    };
    px.document(&mut doc)?;
    Ok(doc)
}

enum Unwrapped {
    Plain(usize), // the NRBF stream starts at this offset of the file
    Own(Vec<u8>), // decompressed (legacy variants)
}

// find the NRBF stream, keep the XML header in doc.xml, decompress the legacy variants
// [px_unwrap]
fn unwrap(data: &[u8], doc: &mut PdnDoc, dec: &ImgDec) -> Result<Unwrapped, i32> {
    let n = data.len();
    if n < 7 {
        if n < 2 {
            return Err(ERR_TRUNC);
        }
    } else if &data[..4] == b"PDN3" {
        let xl = data[4] as usize | (data[5] as usize) << 8 | (data[6] as usize) << 16;
        if xl + 9 > n {
            return Err(ERR_TRUNC);
        }
        doc.xml = data[7..7 + xl].to_vec();
        let tag = u16::from_le_bytes([data[7 + xl], data[8 + xl]]);
        if tag == 0x0100 {
            return Ok(Unwrapped::Plain(xl + 9)); // 00 01: plain NRBF
        }
        if tag != 0x8B1F {
            return Err(ERR_HDR);
        }
        return gunzip(&data[7 + xl..], dec).map(Unwrapped::Own);
    }
    if data[0] != 0x1F || data[1] != 0x8B {
        return Err(ERR_HDR);
    }
    gunzip(data, dec).map(Unwrapped::Own)
}

// decompress the gzip member src (its size from the ISIZE trailer)  [px_gunzip]
fn gunzip(src: &[u8], dec: &ImgDec) -> Result<Vec<u8>, i32> {
    let n = gzip_isize(src)? as u64;
    if n == 0 || n > src.len() as u64 * 1032 + 1024 {
        return Err(ERR_HDR); // deflate expands at most 1032:1
    }
    let r = mem_check(n, dec);
    if r != 0 {
        return Err(r);
    }
    let mut buf = Vec::new();
    if buf.try_reserve_exact(n as usize).is_err() {
        return Err(ERR_NOMEM); // ISIZE comes from the file: fail like mem_alloc
    }
    buf.resize(n as usize, 0);
    let (got, _) = gzip_decode(src, &mut buf)?;
    if got as u64 != n {
        return Err(ERR_HDR);
    }
    Ok(buf)
}

impl<'a> Px<'a, '_> {
    // Document object -> doc (size, metadata, layers)  [px_document]
    fn document(&mut self, doc: &mut PdnDoc) -> Result<(), i32> {
        let root = self.nb.root();
        if !self.nb.is(root, C_DOCUMENT) {
            return Err(ERR_HDR);
        }
        let w = self.nb.get_int(root, b"width").ok_or(ERR_HDR)?;
        let h = self.nb.get_int(root, b"height").ok_or(ERR_HDR)?;
        // 1..65535 (64-bit values checked before truncation)
        if w.wrapping_sub(1) > (IMG_MAXDIM - 1) as u64 || h.wrapping_sub(1) > (IMG_MAXDIM - 1) as u64 {
            return Err(ERR_DIMS);
        }
        let (w, h) = (w as u32, h as u32);
        let r = dims_check(w, h, &self.dec);
        if r != 0 {
            return Err(r);
        }
        self.w = w;
        self.h = h;
        doc.w = w;
        doc.h = h;
        // resolution defaults: inch, 96
        doc.unit = PDU_INCH;
        doc.dpux = 96.0;
        doc.dpuy = 96.0;
        doc.meta = Some(ImgMeta {
            fmt: IMGF_PDN,
            w,
            h,
            bits: 8,
            model: IMM_RGB,
            orient: 1,
            channels: 4,
            flags: IMF_ALPHA | IMF_LOSSLESS,
            frames: 1,
            ..Default::default()
        });
        // document metadata (+ EXIF resolution / ICC)
        let nvc = self.kvblob(Some(&mut *doc), root);
        doc.nvc = nvc;
        resmeta(doc);
        // layers
        let ll = self.nb.get_obj(root, b"layers");
        if ll < 0 {
            return Err(ERR_HDR);
        }
        let mut items = self.nb.get_obj(ll, b"ArrayList+_items");
        if items < 0 {
            items = self.nb.get_obj(ll, b"_items");
            if items < 0 {
                return Err(ERR_HDR);
            }
        }
        self.items = items;
        let n = match self.nb.get_int(ll, b"ArrayList+_size") {
            Some(v) => v,
            None => self.nb.get_int(ll, b"_size").ok_or(ERR_HDR)?,
        };
        let cnt = self.nb.count(items);
        if cnt < 0 {
            return Err(ERR_HDR);
        }
        if n.wrapping_sub(1) > PDN_MAXLAYERS as u64 - 1 || n > cnt as u64 {
            return Err(ERR_HDR);
        }
        self.n = n as u32;
        if self.flags & PDND_PROBE != 0 {
            return Ok(());
        }
        // memory budget: every layer is w * h * 4 bytes
        let r = mem_check(w as u64 * h as u64 * 4 * n, &self.dec);
        if r != 0 {
            return Err(r);
        }
        doc.layers = Vec::with_capacity(n as usize);
        for _ in 0..n {
            doc.layers.push(PdnLayer::default());
        }
        self.payloads()?;
        for i in 0..self.n {
            if self.dec.cancelled() {
                return Err(ERR_CANCEL);
            }
            let mut l = core::mem::take(&mut doc.layers[i as usize]);
            let r = self.layer(i, &mut l);
            doc.layers[i as usize] = l;
            r?;
        }
        Ok(())
    }

    // the metadata items of object `holder` (userMetadataItems KeyValuePair[] or the 3.x
    // userMetaData NameValueCollection) as a blob {u32 klen, key, u32 vlen, value}*; with a
    // doc: also read resolution / ICC items into it  [px_kvblob]
    fn kvblob(&self, mut doc: Option<&mut PdnDoc>, holder: i32) -> Vec<u8> {
        let nb = &self.nb;
        let mut blob = Vec::new();
        let arr = nb.get_obj(holder, b"userMetadataItems");
        if arr >= 0 {
            let cnt = nb.count(arr);
            if cnt < 0 {
                return blob;
            }
            for i in 0..cnt as u32 {
                let e = nb.elem_obj(arr, i);
                if e < 0 {
                    continue;
                }
                let Some(key) = nb.get_str(e, b"key") else { continue };
                let val = nb.get_str(e, b"value");
                kvput(&mut blob, doc.as_deref_mut(), key, val);
            }
            return blob;
        }
        // 3.x NameValueCollection: Keys (String[]) and Values (Object[] of ArrayList / String)
        let nvc = nb.get_obj(holder, b"userMetaData");
        if nvc < 0 {
            return blob;
        }
        let keys = nb.get_obj(nvc, b"Keys");
        if keys < 0 {
            return blob;
        }
        let vals = nb.get_obj(nvc, b"Values");
        if vals < 0 {
            return blob;
        }
        let nk = nb.count(keys);
        if nk < 0 {
            return blob;
        }
        let nv = nb.count(vals);
        if nv < 0 {
            return blob;
        }
        let n = nk.min(nv) as u32;
        for i in 0..n {
            let ko = nb.elem_obj(keys, i);
            if ko < 0 {
                continue;
            }
            let Some(key) = nb.str_(ko) else { continue };
            let vo = nb.elem_obj(vals, i);
            let val = if vo < 0 {
                None
            } else {
                match nb.str_(vo) {
                    Some(s) => Some(s),
                    None => {
                        // ArrayList: first element of _items
                        let it = nb.get_obj(vo, b"_items");
                        if it < 0 {
                            None
                        } else {
                            let e0 = nb.elem_obj(it, 0);
                            if e0 < 0 { None } else { nb.str_(e0) }
                        }
                    }
                }
            };
            kvput(&mut blob, doc.as_deref_mut(), key, val);
        }
        blob
    }

    // walk the deferred MemoryBlock payloads after MessageEnd (validating their chunk
    // structure) and note where each one starts (pay[object] = offset + 1)  [px_payloads]
    fn payloads(&mut self) -> Result<(), i32> {
        let nb = &self.nb;
        let nobj = nb.nobj();
        self.pay = vec![0u64; nobj as usize + 1];
        let mut p = nb.end();
        let d = self.data;
        let len = d.len();
        for i in 0..nobj as i32 {
            if !nb.is(i, C_MEMBLOCK) || nb.get_int(i, b"deferred").unwrap_or(0) == 0 {
                continue;
            }
            let bl = nb.get_int(i, b"length64").ok_or(ERR_HDR)?;
            if bl > 1 << 40 {
                return Err(ERR_HDR); // also negative lengths
            }
            self.pay[i as usize] = p as u64 + 1;
            if !span(p, 5, len) {
                return Err(ERR_TRUNC);
            }
            if d[p] > 1 {
                return Err(ERR_HDR);
            }
            let ch = be32(d, p + 1) as u64;
            if ch == 0 {
                return Err(ERR_HDR);
            }
            p += 5;
            // chunks = ceil(length / chunk); each needs >= 8 bytes
            let mut chunks = (bl + ch - 1) / ch;
            if chunks > ((len - p) >> 3) as u64 {
                return Err(ERR_TRUNC);
            }
            while chunks > 0 {
                if !span(p, 8, len) {
                    return Err(ERR_TRUNC);
                }
                let n = be32(d, p + 4) as u64;
                p += 8;
                if !span(p, n, len) {
                    return Err(ERR_TRUNC);
                }
                p += n as usize;
                chunks -= 1;
            }
        }
        Ok(())
    }

    // layer i of the document into l  [px_layer]
    fn layer(&mut self, i: u32, l: &mut PdnLayer) -> Result<(), i32> {
        let nb = &self.nb;
        let obj = nb.elem_obj(self.items, i);
        if obj < 0 {
            return Err(ERR_HDR);
        }
        // LayerProperties: "Layer+properties", else "properties" of that class
        let mut props = nb.get_obj(obj, b"Layer+properties");
        if props < 0 {
            props = nb.get_obj(obj, b"properties");
            if props >= 0 && !nb.is(props, C_LPROPS) {
                props = -1;
            }
        }
        // defaults
        l.opacity = 255;
        l.visible = 1;
        l.blend = 0;
        l.isbg = 0;
        // name
        let name = if props >= 0 { nb.get_str(props, b"name").unwrap_or(&[]) } else { &[] };
        l.name = cstr(name).to_vec();
        let mut mode_set = false;
        if props >= 0 {
            if let Some(v) = nb.get_int(props, b"visible") {
                l.visible = (v != 0) as u8;
            }
            if let Some(v) = nb.get_int(props, b"isBackground") {
                l.isbg = (v != 0) as u8;
            }
            if let Some(v) = nb.get_int(props, b"opacity") {
                l.opacity = (v as i64).clamp(0, 255) as u8;
            }
            l.meta = self.kvblob(None, props);
            // blend mode: LayerProperties.blendMode (enum struct or Int32)
            let bm = match nb.member(props, b"blendMode") {
                Some(v) if v.kind == NRVK_PRIM => Some(v.val),
                Some(v) if v.kind == NRVK_OBJ => nb.get_int(v.val as u32 as i32, b"value__"),
                _ => None,
            };
            if let Some(m) = bm {
                let m = if m > PDNB_XOR as u64 { 0 } else { m }; // unknown values -> Normal
                l.blend = BM_MAP[m as usize];
                mode_set = true;
            }
        }
        if !mode_set {
            // 3.x: BitmapLayerProperties.blendOp class name
            let bp = nb.get_obj(obj, b"properties");
            if bp >= 0 {
                let bo = nb.get_obj(bp, b"blendOp");
                if bo >= 0
                    && let Some(k) = BO_NAMES.iter().position(|n| nb.is(bo, n))
                {
                    l.blend = k as u8;
                }
            }
        }
        // surface
        let sobj = nb.get_obj(obj, b"surface");
        if sobj < 0 {
            return Err(ERR_HDR);
        }
        let sw = nb.get_int(sobj, b"width").ok_or(ERR_HDR)?;
        if sw != self.w as u64 {
            return Err(ERR_HDR);
        }
        let sh = nb.get_int(sobj, b"height").ok_or(ERR_HDR)?;
        if sh != self.h as u64 {
            return Err(ERR_HDR);
        }
        let stride = nb.get_int(sobj, b"stride").ok_or(ERR_HDR)?;
        let row = self.w as u64 * 4; // w * 4 <= stride <= w * 4 + PDN_MAXPAD
        if stride.wrapping_sub(row) > PDN_MAXPAD {
            return Err(ERR_HDR);
        }
        let mb = nb.get_obj(sobj, b"scan0");
        if mb < 0 || !nb.is(mb, C_MEMBLOCK) {
            return Err(ERR_HDR);
        }
        if nb.get_int(mb, b"hasParent").unwrap_or(0) != 0 {
            return Err(ERR_UNSUPPORTED);
        }
        let bl = nb.get_int(mb, b"length64").ok_or(ERR_HDR)?;
        if stride.wrapping_mul(self.h as u64) != bl {
            return Err(ERR_HDR);
        }
        // pixels
        l.sf = surf_alloc(self.w, self.h, FMT_ARGB, &self.dec)?;
        let r = self.pixels(&mut l.sf, mb, stride);
        if r != 0 { Err(r) } else { Ok(()) }
    }

    // pixels of MemoryBlock mb (file row stride fstride; its length fstride * h was checked
    // by the caller) into sf  [px_pixels]
    fn pixels(&mut self, sf: &mut Surface, mb: i32, fstride: u64) -> i32 {
        let total = fstride * self.h as u64;
        let direct = fstride == sf.stride as u64;
        let d = self.data;
        let len = d.len();
        if self.nb.get_int(mb, b"deferred").unwrap_or(0) == 0 {
            return self.inline(sf, mb, fstride, total);
        }
        // deferred payload (structure walked by payloads)
        let p1 = self.pay[mb as usize];
        if p1 == 0 {
            return ERR_HDR;
        }
        let mut p = (p1 - 1) as usize;
        if !span(p, 5, len) {
            return ERR_TRUNC;
        }
        let fmt = d[p];
        let chunk = be32(d, p + 1) as u64;
        if chunk == 0 {
            return ERR_HDR;
        }
        p += 5;
        let count = (total + chunk - 1) / chunk;
        let nseen = count.div_ceil(8) as usize;
        if self.seen.len() < nseen {
            self.seen = vec![0u8; nseen];
        } else {
            self.seen[..nseen].fill(0);
        }
        if count >= 2 && plat::thr_count() != 0 {
            // several chunks: on the thread pool
            if let Some(r) = self.par(sf, fstride, p, total, chunk, fmt, direct, count) {
                return r;
            }
        }
        if !direct {
            let n = chunk.min(total) as usize;
            if self.scr.len() < n {
                self.scr = vec![0u8; n];
            }
        }
        let w4 = sf.w as usize * 4;
        let hh = sf.h as usize;
        let st = self.st.get_or_insert_with(InflateState::new);
        for _ in 0..count {
            if self.dec.cancelled() {
                return ERR_CANCEL;
            }
            if !span(p, 8, len) {
                return ERR_TRUNC;
            }
            let idx = be32(d, p) as u64;
            let size = be32(d, p + 4) as u64;
            p += 8;
            if !span(p, size, len) {
                return ERR_TRUNC;
            }
            let size = size as usize;
            if idx >= count {
                return ERR_HDR;
            }
            let (byte, bit) = ((idx >> 3) as usize, 1u8 << (idx & 7));
            if self.seen[byte] & bit != 0 {
                return ERR_HDR;
            }
            self.seen[byte] |= bit;
            let off = idx * chunk; // < total
            let exp = (total - off).min(chunk) as usize;
            let dst: &mut [u8] = if direct {
                &mut sf.px[off as usize..off as usize + exp]
            } else {
                &mut self.scr[..exp]
            };
            let src = &d[p..p + size];
            if fmt == 0 {
                match gzip_decode_st(Some(&mut **st), src, dst) {
                    Err(e) => return e,
                    Ok((n, _)) if n != exp => return ERR_CHUNK,
                    _ => {}
                }
            } else {
                if size != exp {
                    return ERR_CHUNK;
                }
                dst.copy_from_slice(src);
            }
            if !direct {
                let stride = sf.stride as usize;
                let px = &mut sf.px;
                scatter(w4, hh, fstride, off, &self.scr[..exp], |y, x, b| {
                    px[y * stride + x..y * stride + x + b.len()].copy_from_slice(b);
                });
            }
            p += size;
        }
        0
    }

    // inline chunks: chunk0, chunk1, ... (byte arrays) or pointerData  [px_pixels .pp_inline]
    fn inline(&self, sf: &mut Surface, mb: i32, fstride: u64, total: u64) -> i32 {
        let nb = &self.nb;
        let w4 = sf.w as usize * 4;
        let hh = sf.h as usize;
        let stride = sf.stride as usize;
        let mut off = 0u64;
        let mut k = 0u32;
        let mut name = Vec::with_capacity(16);
        loop {
            name.clear();
            name.extend_from_slice(b"chunk");
            utoa(k, &mut name);
            let mut o = nb.get_obj(mb, &name);
            if o < 0 {
                if k != 0 {
                    break;
                }
                o = nb.get_obj(mb, b"pointerData");
                if o < 0 {
                    break;
                }
                k = 0x7FFF_FFFE; // one piece only
            }
            let Some((data, t)) = nb.prim_data(o) else { return ERR_HDR };
            if t != NRT_BYTE {
                return ERR_HDR;
            }
            let n = nb.count(o);
            if n < 0 || n as u64 > total - off {
                return ERR_HDR;
            }
            let px = &mut sf.px;
            scatter(w4, hh, fstride, off, &data[..n as usize], |y, x, b| {
                px[y * stride + x..y * stride + x + b.len()].copy_from_slice(b);
            });
            off += n as u64;
            k += 1;
            if k >= 0x7FFF_FFFF {
                break;
            }
        }
        if off != total { ERR_HDR } else { 0 }
    }

    // the deferred chunks on the thread pool; None = decode sequentially (the scratch
    // buffers would not fit the budget)  [px_par]
    #[allow(clippy::too_many_arguments)]
    fn par(&mut self, sf: &mut Surface, fstride: u64, mut p: usize, total: u64, chunk: u64, fmt: u8, direct: bool, count: u64) -> Option<i32> {
        let scrsz = if direct { 0 } else { chunk.min(total) };
        if scrsz > XD_MAXSCR {
            return None;
        }
        if self.slots.is_empty() {
            let n = (plat::thr_count() as usize).min(plat::THR_MAX) + 1;
            self.slots = (0..n).map(|_| SpinLock::new(XdSlot::default())).collect();
        }
        let nslot = self.slots.len();
        // the extra scratch buffers must fit the budget (else sequentially)
        if mem_check(scrsz * (nslot as u64 - 1), &self.dec) != 0 {
            return None;
        }
        // pass 1: the headers in file order
        let d = self.data;
        let len = d.len();
        let mut recs: Vec<(usize, usize, u64)> = Vec::with_capacity(count as usize);
        let mut herr = 0;
        while (recs.len() as u64) < count {
            if self.dec.cancelled() {
                herr = ERR_CANCEL;
                break;
            }
            if !span(p, 8, len) {
                herr = ERR_TRUNC;
                break;
            }
            let idx = be32(d, p) as u64;
            let size = be32(d, p + 4) as u64;
            p += 8;
            if !span(p, size, len) {
                herr = ERR_TRUNC;
                break;
            }
            if idx >= count {
                herr = ERR_HDR;
                break;
            }
            let (byte, bit) = ((idx >> 3) as usize, 1u8 << (idx & 7));
            if self.seen[byte] & bit != 0 {
                herr = ERR_HDR;
                break;
            }
            self.seen[byte] |= bit;
            recs.push((p, size as usize, idx));
            p += size as usize;
        }
        // pass 2: decode the recorded chunks in parallel
        if !recs.is_empty() {
            let res: Vec<AtomicI32> = (0..recs.len()).map(|_| AtomicI32::new(0)).collect();
            let w4 = sf.w as usize * 4;
            let hh = sf.h as usize;
            let stride = sf.stride as usize;
            let dec = &self.dec;
            let slots = &self.slots;
            let recs = &recs;
            if direct {
                let parts: Vec<SpinLock<&mut [u8]>> = sf.px[..total as usize].chunks_mut(chunk as usize).map(SpinLock::new).collect();
                let item = |k: usize, _: &plat::JobCtl| {
                    let (pos, size, idx) = recs[k];
                    let e = if dec.cancelled() {
                        ERR_CANCEL
                    } else {
                        let mut s = slots[plat::thr_self().min(nslot - 1)].lock();
                        let mut dst = parts[idx as usize].lock();
                        chunk_dec(&mut s.st, fmt, &d[pos..pos + size], &mut dst[..])
                    };
                    res[k].store(e, Ordering::Relaxed);
                };
                let _ = plat::parallel_for(recs.len(), &item);
            } else {
                let rows: Vec<SpinLock<&mut [u8]>> = sf.px.chunks_mut(stride).take(hh).map(SpinLock::new).collect();
                let item = |k: usize, _: &plat::JobCtl| {
                    let (pos, size, idx) = recs[k];
                    let e = if dec.cancelled() {
                        ERR_CANCEL
                    } else {
                        let off = idx * chunk; // < total: index < count
                        let exp = (total - off).min(chunk) as usize;
                        let mut g = slots[plat::thr_self().min(nslot - 1)].lock();
                        let s = &mut *g;
                        if s.scr.len() < scrsz as usize {
                            s.scr = vec![0u8; scrsz as usize];
                        }
                        let e = chunk_dec(&mut s.st, fmt, &d[pos..pos + size], &mut s.scr[..exp]);
                        if e == 0 {
                            scatter(w4, hh, fstride, off, &s.scr[..exp], |y, x, b| {
                                rows[y].lock()[x..x + b.len()].copy_from_slice(b);
                            });
                        }
                        e
                    };
                    res[k].store(e, Ordering::Relaxed);
                };
                let _ = plat::parallel_for(recs.len(), &item);
            }
            for r in &res {
                let e = r.load(Ordering::Relaxed);
                if e != 0 {
                    return Some(e);
                }
            }
        }
        Some(herr)
    }
}

// decode one chunk into dst (its expected size): gzip member (format 0) or raw bytes; the
// gzip errors, ERR_CHUNK for a wrong size  [px_pitem]
fn chunk_dec(st: &mut Option<Box<InflateState>>, fmt: u8, src: &[u8], dst: &mut [u8]) -> i32 {
    if fmt == 0 {
        let s = st.get_or_insert_with(InflateState::new);
        match gzip_decode_st(Some(&mut **s), src, dst) {
            Err(e) => e,
            Ok((n, _)) if n != dst.len() => ERR_CHUNK,
            _ => 0,
        }
    } else if src.len() != dst.len() {
        ERR_CHUNK
    } else {
        dst.copy_from_slice(src);
        0
    }
}

// the bytes [off, off + src.len()) of an image whose rows are fstride bytes apart (the first
// w4 bytes of a row are pixels, the rest padding) into rows 0..h: put(row, byte, bytes)
// [px_scatter]
fn scatter(w4: usize, h: usize, fstride: u64, off: u64, src: &[u8], mut put: impl FnMut(usize, usize, &[u8])) {
    let mut row = off / fstride;
    let mut col = off % fstride;
    let mut s = 0usize;
    let mut n = src.len() as u64;
    while n > 0 && row < h as u64 {
        let take = (fstride - col).min(n);
        if col < w4 as u64 {
            let m = (w4 as u64 - col).min(take) as usize;
            put(row as usize, col as usize, &src[s..s + m]);
        }
        s += take as usize;
        n -= take;
        row += 1;
        col = 0;
    }
}

// append the item {u32 klen, key, u32 vlen, value} (None value = empty); with a doc:
// px_exif as well  [px_kvput]
fn kvput(blob: &mut Vec<u8>, doc: Option<&mut PdnDoc>, key: &[u8], val: Option<&[u8]>) {
    let v = val.unwrap_or(&[]);
    blob.extend_from_slice(&(key.len() as u32).to_le_bytes());
    blob.extend_from_slice(key);
    blob.extend_from_slice(&(v.len() as u32).to_le_bytes());
    blob.extend_from_slice(v);
    if let (Some(doc), Some(v)) = (doc, val) {
        exif(doc, key, v);
    }
}

// EXIF items 296 / 282 / 283 (resolution) and 34675 (ICC profile) of the document metadata
// [px_exif]
fn exif(doc: &mut PdnDoc, key: &[u8], val: &[u8]) {
    if key.len() < 9 || &key[..9] != EXIF_TAG {
        return;
    }
    let tag = attr_int(val, b"id=\"");
    let Some(v) = attr(val, b"value=\"") else { return };
    match tag {
        34675 => {
            let Some(m) = doc.meta.as_mut() else { return };
            let mut buf = vec![0u8; (v.len() >> 2) * 3 + 3]; // 3/4 of the base64 length
            let n = b64dec(v, &mut buf);
            if n > 0 {
                meta_blob(m, Blob::Icc, &buf[..n as usize]);
            }
        }
        296 | 282 | 283 => {
            let mut b = [0u8; 16];
            let n = b64dec(v, &mut b);
            if n < 0 {
                return;
            }
            if tag == 296 {
                if n < 2 {
                    return;
                }
                let u = u16::from_le_bytes([b[0], b[1]]) as u32;
                if u.wrapping_sub(1) <= 2 {
                    doc.unit = u; // EXIF 1 none, 2 inch, 3 cm = PDU_* 1..3
                }
                return;
            }
            if n < 8 {
                return;
            }
            let num = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            let den = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
            if den == 0 {
                return;
            }
            let x = num as f64 / den as f64;
            if tag == 283 {
                doc.dpuy = x;
            } else {
                doc.dpux = x;
            }
        }
        _ => {}
    }
}

// doc.unit / doc.dpu* -> doc.meta dpix / dpiy / resunit  [px_resmeta]
fn resmeta(doc: &mut PdnDoc) {
    let (unit, mut x, mut y) = (doc.unit, doc.dpux, doc.dpuy);
    let Some(m) = doc.meta.as_mut() else { return };
    match unit {
        PDU_INCH => m.resunit = IMR_INCH,
        PDU_CM => {
            x *= 2.54;
            y *= 2.54;
            m.resunit = IMR_CM;
        }
        _ => {
            m.resunit = IMR_NONE;
            return;
        }
    }
    x *= 65536.0;
    y *= 65536.0;
    // values come from u32 / u32: no NaN
    m.dpix = rt::fx::cvttd64(rt::fx::minsd(x, 4294967295.0)) as u32;
    m.dpiy = rt::fx::cvttd64(rt::fx::minsd(y, 4294967295.0)) as u32;
}

/// "PDN3" header: width / height from the XML, 8-bit straight BGRA; legacy gzip files are
/// recognised without a size  [pdn_probe]
pub fn probe(data: &[u8], m: &mut ImgMeta) -> bool {
    let n = data.len();
    if n < 2 {
        return false;
    }
    if data[0] == 0x1F && data[1] == 0x8B {
        return true;
    }
    if n < 7 || &data[..4] != b"PDN3" {
        return false;
    }
    let mut xl = data[4] as usize | (data[5] as usize) << 8 | (data[6] as usize) << 16;
    if xl > n - 7 {
        xl = n - 7; // truncated header: scan what is there
    }
    let xml = &data[7..7 + xl];
    let w = attr_int(xml, b"width=\"");
    if w >= 0 {
        m.w = w as u32;
        let h = attr_int(xml, b"height=\"");
        if h >= 0 {
            m.h = h as u32;
        }
    }
    m.bits = 8;
    m.model = IMM_RGB;
    m.channels = 4;
    m.flags |= IMF_ALPHA | IMF_LOSSLESS;
    true
}

/// img_decode_ex's entry: the composite of the visible layers (FMT_ARGB); IMGD_PROBE:
/// metadata only; IMGD_ICC: the profile of the document metadata  [pdn_decode_flat]
pub fn decode_flat(data: &[u8], out: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    let probe = dec.flags & IMGD_PROBE != 0;
    let mut doc = match decode_ex(data, Some(dec), if probe { PDND_PROBE } else { 0 }) {
        Ok(d) => d,
        Err(e) => return e,
    };
    meta.w = doc.w;
    meta.h = doc.h;
    meta.bits = 8;
    meta.model = IMM_RGB;
    meta.channels = 4;
    meta.flags |= IMF_ALPHA | IMF_LOSSLESS;
    if let Some(dm) = &doc.meta {
        meta.dpix = dm.dpix;
        meta.dpiy = dm.dpiy;
        meta.resunit = dm.resunit;
        if dec.flags & IMGD_ICC != 0 && !dm.icc.is_empty() {
            meta_blob(meta, Blob::Icc, &dm.icc);
        }
    }
    if probe {
        return 0;
    }
    match flatten_owned(&mut doc) {
        Ok(sf) => {
            *out = sf;
            0
        }
        Err(e) => e,
    }
}
