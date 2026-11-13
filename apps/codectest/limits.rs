// limits -- codectest --limits: the decoders' budgets against crafted files, built here, that
// must be refused quickly (instead of allocating gigabytes or decoding thousands of tiles), and
// valid files of the same shape that still decode
//
//   HEIF  grid tiles larger than tile 0 (HEVC, AV1), thousands of tiles for a small image, an
//         item's picture beyond max_pixels (HEVC SPS, AV1 full sequence header with
//         frame_size_override_flag), one tile item referenced four times; valid: a grid with
//         alpha from the HEIC writer, grids of distinct tiles
//   TIFF  tiles that dwarf the image (1 x 1 with 65536 x 65520 tiles, 1 x 4096 with 65536 x 16
//         tiles); valid: a 20 x 20 image in one 256 x 256 tile, 64 x 48 in 16 x 16 tiles
//   PDN   a layer referenced twice, two layers sharing a pixel block, eight layers sharing their
//         properties (a long name); valid: the same documents unpatched
//
// One line per check: "ok   NAME (MS ms)" or "FAIL NAME: ..."; exit code 1 when one fails.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, Surface};
use img_core::io::decode_ex;
use img_core::{ERR_HDR, ERR_LIMIT, ImgDec, ImgMeta, ImgOpt};
use img_pdn::{DocRef, LayerRef};

const SLOW_US: u64 = 5_000_000; // a check that takes longer fails

struct Run {
    fails: u32,
}

impl Run {
    /// f -> result; it must be `want` and come within SLOW_US
    fn check(&mut self, name: &str, want: i32, f: impl FnOnce() -> i32) {
        let t0 = plat::time_us();
        let r = f();
        let ms = (plat::time_us() - t0) / 1000;
        if r != want || ms * 1000 > SLOW_US {
            self.fails += 1;
            plat::write_stdout(format!("FAIL {name}: result {r} (want {want}), {ms} ms\n").as_bytes());
        } else {
            plat::write_stdout(format!("ok   {name} ({ms} ms)\n").as_bytes());
        }
    }
}

/// run every check -> exit code
pub fn run() -> i32 {
    let mut r = Run { fails: 0 };
    heif_checks(&mut r);
    tiff(&mut r);
    pdn(&mut r);
    if r.fails != 0 {
        plat::write_stdout(format!("FAIL {} limit checks\n", r.fails).as_bytes());
        return 1;
    }
    0
}

// ================================================================== HEIF / AVIF

fn bx(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(body.len() + 8);
    v.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    v.extend_from_slice(ty);
    v.extend_from_slice(body);
    v
}

fn full(ty: &[u8; 4], ver: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![ver, 0, 0, 0];
    b.extend_from_slice(body);
    bx(ty, &b)
}

fn be16(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&(x as u16).to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// an item: id, type, its properties (1-based ipco indices), data
struct It {
    id: u32,
    ty: &'static [u8; 4],
    props: Vec<u8>,
    data: Vec<u8>,
}

fn it(id: u32, ty: &'static [u8; 4], props: &[u8], data: &[u8]) -> It {
    It { id, ty, props: props.to_vec(), data: data.to_vec() }
}

/// a HEIF file: ftyp, meta (hdlr pict, pitm = the first item, iinf / infe v2, iloc v1 with the
/// data in idat, iref dimg from -> to, iprp)
fn heif_file(brand: &[u8; 4], items: &[It], props: &[Vec<u8>], dimg: &[(u32, Vec<u32>)]) -> Vec<u8> {
    let mut f = bx(b"ftyp", &[&brand[..], &[0, 0, 0, 0], b"mif1", &brand[..]].concat());
    let mut hd = vec![0u8; 4];
    hd.extend_from_slice(b"pict");
    hd.extend_from_slice(&[0u8; 13]);
    let mut meta = full(b"hdlr", 0, &hd);
    let mut b = Vec::new();
    be16(&mut b, items[0].id);
    meta.extend(full(b"pitm", 0, &b));
    let mut b = Vec::new();
    be16(&mut b, items.len() as u32);
    for i in items {
        let mut e = Vec::new();
        be16(&mut e, i.id);
        be16(&mut e, 0);
        e.extend_from_slice(i.ty);
        e.push(0);
        b.extend(full(b"infe", 2, &e));
    }
    meta.extend(full(b"iinf", 0, &b));
    let mut b = vec![0x44, 0x00]; // 32-bit offsets / lengths, no base offset / index
    be16(&mut b, items.len() as u32);
    let mut idat = Vec::new();
    for i in items {
        be16(&mut b, i.id);
        be16(&mut b, 1); // construction_method 1 (idat)
        be16(&mut b, 0);
        be16(&mut b, 1);
        be32(&mut b, idat.len() as u32);
        be32(&mut b, i.data.len() as u32);
        idat.extend_from_slice(&i.data);
    }
    meta.extend(full(b"iloc", 1, &b));
    if !dimg.is_empty() {
        let mut b = Vec::new();
        for (from, to) in dimg {
            let mut r = Vec::new();
            be16(&mut r, *from);
            be16(&mut r, to.len() as u32);
            for &t in to {
                be16(&mut r, t);
            }
            b.extend(bx(b"dimg", &r));
        }
        meta.extend(full(b"iref", 0, &b));
    }
    let ipco = bx(b"ipco", &props.concat());
    let mut b = Vec::new();
    be32(&mut b, items.len() as u32);
    for i in items {
        be16(&mut b, i.id);
        b.push(i.props.len() as u8);
        for &p in &i.props {
            b.push(0x80 | p);
        }
    }
    meta.extend(bx(b"iprp", &[ipco, full(b"ipma", 0, &b)].concat()));
    meta.extend(bx(b"idat", &idat));
    f.extend(full(b"meta", 0, &meta));
    f
}

fn ispe(w: u32, h: u32) -> Vec<u8> {
    let mut b = Vec::new();
    be32(&mut b, w);
    be32(&mut b, h);
    full(b"ispe", 0, &b)
}

/// grid descriptor: rows x cols tiles, output w x h
fn grid(rows: u32, cols: u32, w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0, 0, (rows - 1) as u8, (cols - 1) as u8];
    be16(&mut b, w);
    be16(&mut b, h);
    b
}

/// a grey HEVC picture (4:0:0, img_hevc::encode): the hvcC property, the item data
fn hevc(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let y = vec![128u16; (w * h) as usize];
    let cfg = img_hevc::EncCfg { pl: [&y[..], &[], &[]], st: [w, 0, 0], w, h, chroma: 0, qp: 40, flags: 0, vui: 0, cancel: None };
    let o = img_hevc::encode(&cfg).unwrap_or_default();
    let mut c = vec![1u8];
    c.extend_from_slice(&o.ptl);
    c.extend_from_slice(&[0xF0, 0x00, 0xFC, 0xFC, 0xF8, 0xF8, 0, 0, 0x0F, 3]); // 4:0:0, 8 bits, 4-byte lengths
    for (t, n) in [(32u8, &o.nal[0]), (33, &o.nal[1]), (34, &o.nal[2])] {
        c.extend_from_slice(&[0x80 | t, 0, 1]);
        c.extend_from_slice(&(n.len() as u16).to_be_bytes());
        c.extend_from_slice(n);
    }
    let mut d = (o.nal[3].len() as u32).to_be_bytes().to_vec();
    d.extend_from_slice(&o.nal[3]);
    (bx(b"hvcC", &c), d)
}

/// a grey AV1 picture (4:0:0, img_av1::encode, reduced still picture header): the av1C
/// property, the item data (sequence header + frame)
fn av1(w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
    let y = vec![128u16; (w * h) as usize];
    let cfg = img_av1::EncIn {
        w,
        h,
        bd: 8,
        chroma: 0,
        qidx: 200,
        cicp: 0x020202,
        tile_sb: 0,
        flags: 0,
        cancel: None,
        planes: [&y[..], &[], &[]],
        strides: [w, 0, 0],
    };
    let o = img_av1::encode(&cfg).unwrap_or_default();
    let mut c = vec![0x81, (o.profile << 5 | o.level) as u8, 0x1C, 0]; // monochrome
    c.extend_from_slice(&o.data[..(o.seqlen as usize).min(o.data.len())]);
    (bx(b"av1C", &c), o.data)
}

/// MSB-first bits
#[derive(Default)]
struct Bw {
    v: Vec<u8>,
    n: u32,
}

impl Bw {
    fn put(&mut self, x: u32, bits: u32) {
        for i in (0..bits).rev() {
            if self.n == 0 {
                self.v.push(0);
            }
            if x >> i & 1 != 0 {
                *self.v.last_mut().unwrap() |= 0x80 >> self.n;
            }
            self.n = (self.n + 1) & 7;
        }
    }
}

fn obu(ty: u8, pay: &[u8]) -> Vec<u8> {
    let mut v = vec![ty << 3 | 2, pay.len() as u8]; // has_size_field, size < 128
    v.extend_from_slice(pay);
    v
}

/// AV1 with a full sequence header (16-bit frame sizes as libaom writes them, max 16 x 16) and
/// a key frame header: frame_size_override_flag with a 2048 x 2048 frame (fso) or the maximum;
/// no tile data
fn av1_full(fso: bool) -> (Vec<u8>, Vec<u8>) {
    let mut s = Bw::default();
    s.put(0, 5); // seq_profile, still_picture, reduced_still_picture_header
    s.put(0, 7); // timing_info_present, initial_display_delay_present, operating_points_cnt_minus_1
    s.put(0, 17); // operating_point_idc[ 0 ], seq_level_idx[ 0 ]
    s.put(15, 4);
    s.put(15, 4);
    s.put(15, 16);
    s.put(15, 16);
    s.put(0, 9); // frame ids, 128 x 128, filter intra, edge filter, inter tools, order hint
    s.put(3, 2); // screen content tools / integer mv chosen per frame
    s.put(0, 3); // superres, cdef, restoration
    s.put(0, 7); // 8 bits, colour, no description, studio range, chroma position, uv delta q
    s.put(0, 1); // film_grain_params_present
    s.put(1, 1); // trailing bit
    let seq = obu(1, &s.v);
    let mut f = Bw::default();
    f.put(1, 4); // show_existing_frame 0, KEY_FRAME, show_frame 1
    f.put(0, 2); // disable_cdf_update, allow_screen_content_tools
    f.put(fso as u32, 1);
    if fso {
        f.put(2047, 16);
        f.put(2047, 16);
    }
    f.put(0, 32);
    f.put(0, 32);
    let mut d = seq.clone();
    d.extend(obu(3, &f.v));
    let mut c = vec![0x81, 0, 0x0C, 0]; // 4:2:0
    c.extend(seq);
    (bx(b"av1C", &c), d)
}

fn heif_dec(file: &[u8], maxpx: u64) -> i32 {
    let dec = ImgDec { maxpx, ..Default::default() };
    let mut sf = Surface::default();
    let mut meta = ImgMeta::default();
    img_heif::decode(file, &mut sf, &dec, &mut meta)
}

/// a valid decode of the expected size -> 0 (else the error, -1 for another size)
fn heif_ok(file: &[u8], maxpx: u64, w: i32, h: i32) -> i32 {
    let dec = ImgDec { maxpx, ..Default::default() };
    let mut sf = Surface::default();
    let mut meta = ImgMeta::default();
    match img_heif::decode(file, &mut sf, &dec, &mut meta) {
        0 if sf.w == w && sf.h == h => 0,
        0 => -1,
        e => e,
    }
}

/// a grid file: grid item 1 (rows x cols, output w x h), tile items 2.. with (property, data)
/// and the tile references (indices into tiles)
fn gridfile(brand: &[u8; 4], rows: u32, cols: u32, w: u32, h: u32, tiles: &[(Vec<u8>, Vec<u8>)], refs: &[usize]) -> Vec<u8> {
    let mut props = vec![ispe(w, h)];
    let mut items = vec![it(1, b"grid", &[1], &grid(rows, cols, w, h))];
    let ty: &'static [u8; 4] = if brand == b"avif" { b"av01" } else { b"hvc1" };
    for (k, (p, d)) in tiles.iter().enumerate() {
        props.push(p.clone());
        items.push(it(k as u32 + 2, ty, &[props.len() as u8], d));
    }
    let to = refs.iter().map(|&k| k as u32 + 2).collect();
    heif_file(brand, &items, &props, &[(1, to)])
}

fn heif_checks(r: &mut Run) {
    // valid: the HEIC writer's grid (two tiles across) with an alpha grid
    let mut sf = Surface::new(600, 300, FMT_ARGB);
    for (i, p) in sf.px.chunks_exact_mut(4).enumerate() {
        p.copy_from_slice(&[(i * 7) as u8, (i * 3) as u8, (i >> 4) as u8, 0x80 | i as u8]);
    }
    let mut file = Vec::new();
    let e = img_heif::encode(&sf, &ImgOpt::default(), &mut file);
    r.check("heic writer grid with alpha decodes", 0, || if e != 0 { e } else { heif_ok(&file, 0, 600, 300) });
    r.check("heic writer grid decodes with max_pixels = its size", 0, || heif_ok(&file, 600 * 300, 600, 300));
    // HEVC grids
    let t64 = hevc(64, 64);
    let f = gridfile(b"heic", 2, 2, 128, 128, &[t64.clone(), t64.clone(), t64.clone(), t64.clone()], &[0, 1, 2, 3]);
    r.check("heic grid of distinct tiles decodes", 0, || heif_ok(&f, 0, 128, 128));
    let (t16, t512) = (hevc(16, 16), hevc(512, 512));
    let f = gridfile(b"heic", 1, 2, 32, 16, &[t16.clone(), t512.clone()], &[0, 1]);
    r.check("heic grid tile larger than tile 0", ERR_LIMIT, || heif_dec(&f, 0));
    let t8 = hevc(8, 8);
    let f = gridfile(b"heic", 64, 64, 512, 512, &[t8], &[0; 4096]);
    r.check("heic grid of 4096 tiles for 512 x 512", ERR_LIMIT, || heif_dec(&f, 0));
    // an item whose stream holds a larger picture than its ispe
    let f = heif_file(b"heic", &[it(1, b"hvc1", &[1, 2], &t512.1)], &[t512.0.clone(), ispe(16, 16)], &[]);
    r.check("heic 16 x 16 item with a 512 x 512 SPS, max_pixels 16384", ERR_LIMIT, || heif_dec(&f, 16384));
    r.check("heic 16 x 16 item with a 512 x 512 SPS, default limit", 0, || heif_ok(&f, 0, 16, 16));
    // one tile item behind every reference: its data fed four times
    let mut fill = Vec::new();
    for _ in 0..3000 {
        fill.extend_from_slice(&100u32.to_be_bytes());
        fill.extend_from_slice(&[39 << 1, 1]); // prefix SEI (ignored by the decoder)
        fill.extend_from_slice(&[0x80; 98]);
    }
    fill.extend_from_slice(&t16.1);
    let tf = (t16.0.clone(), fill);
    let f = gridfile(b"heic", 2, 2, 32, 32, &[tf.clone()], &[0, 0, 0, 0]);
    r.check("heic grid: one 300 KB tile item referenced 4 times", ERR_LIMIT, || heif_dec(&f, 0));
    let f = gridfile(b"heic", 2, 2, 32, 32, &[tf.clone(), tf.clone(), tf.clone(), tf], &[0, 1, 2, 3]);
    r.check("heic grid: four 300 KB tile items", 0, || heif_ok(&f, 0, 32, 32));
    // AV1 grids
    let a64 = av1(64, 64);
    let f = gridfile(b"avif", 2, 2, 128, 128, &[a64.clone(), a64.clone(), a64.clone(), a64], &[0, 1, 2, 3]);
    r.check("avif grid of distinct tiles decodes", 0, || heif_ok(&f, 0, 128, 128));
    let f = gridfile(b"avif", 1, 2, 32, 16, &[av1(16, 16), av1(256, 256)], &[0, 1]);
    r.check("avif grid tile larger than tile 0", ERR_LIMIT, || heif_dec(&f, 0));
    // a full sequence header (max 16 x 16) whose frame header overrides the size: 2048 x 2048
    let (c, d) = av1_full(true);
    let f = heif_file(b"avif", &[it(1, b"av01", &[1, 2], &d)], &[c, ispe(16, 16)], &[]);
    r.check("avif full sequence header, 2048 x 2048 frame, max_pixels 65536", ERR_LIMIT, || heif_dec(&f, 65536));
    let (c, d) = av1_full(false);
    let f = heif_file(b"avif", &[it(1, b"av01", &[1, 2], &d)], &[c, ispe(16, 16)], &[]);
    r.check("avif full sequence header, 16 x 16 frame: not a limit error", 0, || match heif_dec(&f, 65536) {
        ERR_LIMIT => ERR_LIMIT,
        _ => 0,
    });
}

// ================================================================== TIFF

/// a little-endian TIFF: 8-bit grey, uncompressed, tiles tw x th with the given offsets'
/// data (one chunk per entry of `chunks`)
fn tiff_file(w: u32, h: u32, tw: u32, th: u32, chunks: &[Vec<u8>]) -> Vec<u8> {
    let n = chunks.len() as u32;
    let ntag = 10u32;
    let ifd_end = 8 + 2 + 12 * ntag + 4;
    let arrays = if n > 1 { 8 * n } else { 0 };
    let mut data_off = ifd_end + arrays;
    let mut offs = Vec::new();
    for c in chunks {
        offs.push(data_off);
        data_off += c.len() as u32;
    }
    let cnts: Vec<u32> = chunks.iter().map(|c| c.len() as u32).collect();
    let mut v = b"II*\0".to_vec();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&(ntag as u16).to_le_bytes());
    let mut tag = |t: u16, ty: u16, cnt: u32, val: u32| {
        v.extend_from_slice(&t.to_le_bytes());
        v.extend_from_slice(&ty.to_le_bytes());
        v.extend_from_slice(&cnt.to_le_bytes());
        v.extend_from_slice(&val.to_le_bytes());
    };
    tag(256, 4, 1, w);
    tag(257, 4, 1, h);
    tag(258, 3, 1, 8);
    tag(259, 3, 1, 1);
    tag(262, 3, 1, 1);
    tag(277, 3, 1, 1);
    tag(322, 4, 1, tw);
    tag(323, 4, 1, th);
    if n > 1 {
        tag(324, 4, n, ifd_end);
        tag(325, 4, n, ifd_end + 4 * n);
    } else {
        tag(324, 4, 1, offs[0]);
        tag(325, 4, 1, cnts[0]);
    }
    v.extend_from_slice(&0u32.to_le_bytes());
    if n > 1 {
        for x in offs.iter().chain(cnts.iter()) {
            v.extend_from_slice(&x.to_le_bytes());
        }
    }
    for c in chunks {
        v.extend_from_slice(c);
    }
    v
}

fn tiff_dec(file: &[u8], w: i32, h: i32) -> i32 {
    match decode_ex(file, None, None) {
        Ok(sf) if sf.w == w && sf.h == h => 0,
        Ok(_) => -1,
        Err(e) => e,
    }
}

fn tiff(r: &mut Run) {
    let f = tiff_file(1, 1, 65536, 65520, &[vec![0x55]]);
    r.check("tiff 1 x 1 with 65536 x 65520 tiles", ERR_LIMIT, || tiff_dec(&f, 1, 1));
    let f = tiff_file(1, 4096, 65536, 16, &vec![vec![0x55]; 256]);
    r.check("tiff 1 x 4096 with 65536 x 16 tiles", ERR_LIMIT, || tiff_dec(&f, 1, 4096));
    let f = tiff_file(20, 20, 256, 256, &[vec![0x80; 65536]]);
    r.check("tiff 20 x 20 in one 256 x 256 tile decodes", 0, || tiff_dec(&f, 20, 20));
    let f = tiff_file(64, 48, 16, 16, &vec![vec![0x40; 256]; 12]);
    r.check("tiff 64 x 48 in 16 x 16 tiles decodes", 0, || tiff_dec(&f, 64, 48));
}

// ================================================================== PDN

/// a .pdn of n 3 x 2 layers (the first named name0), written by img_pdn
fn pdn_file(n: usize, name0: &[u8]) -> Vec<u8> {
    let mut sf = Surface::new(3, 2, FMT_ARGB);
    sf.px.fill(0x7F);
    let layers = (0..n)
        .map(|i| LayerRef { sf: &sf, name: if i == 0 { name0 } else { b"L" }, opacity: 255, visible: 1, blend: 0, isbg: (i == 0) as u8, meta: &[] })
        .collect();
    let doc = DocRef { w: 3, h: 2, unit: 2, dpux: 96.0, dpuy: 96.0, layers, nvc: &[], icc: &[] };
    img_pdn::encode_ex(&doc, None, 0).unwrap_or_default()
}

/// the offsets where pat starts
fn find(v: &[u8], pat: &[u8]) -> Vec<usize> {
    (0..v.len().saturating_sub(pat.len() - 1)).filter(|&i| v[i..i + pat.len()] == *pat).collect()
}

/// decode -> 0 with that many layers, else the error (-1: another count)
fn pdn_dec(file: &[u8], layers: usize) -> i32 {
    match img_pdn::decode_ex(file, None, 0) {
        Ok(doc) if doc.layers.len() == layers => 0,
        Ok(_) => -1,
        Err(e) => e,
    }
}

fn pdn(r: &mut Run) {
    let base = pdn_file(2, b"Background");
    r.check("pdn 2 layers decode", 0, || pdn_dec(&base, 2));
    // the layer list (ArraySingleObject: 10 id, capacity 4, then 09 id per layer) names layer 0 twice
    let mut f = base.clone();
    if let Some(&i) = find(&f, &[4, 0, 0, 0, 9]).iter().find(|&&i| i >= 5 && f[i - 5] == 0x10 && f.get(i + 9) == Some(&9)) {
        let id0: [u8; 4] = f[i + 5..i + 9].try_into().unwrap();
        f[i + 10..i + 14].copy_from_slice(&id0);
    }
    r.check("pdn layer referenced twice", ERR_HDR, || pdn_dec(&f, 2));
    // the two Surfaces (width 3, height 2, stride 12, 09 scan0) name one MemoryBlock
    let mut f = base.clone();
    let s = find(&f, &[3, 0, 0, 0, 2, 0, 0, 0, 12, 0, 0, 0, 9]);
    if s.len() == 2 {
        let mb0: [u8; 4] = f[s[0] + 13..s[0] + 17].try_into().unwrap();
        f[s[1] + 13..s[1] + 17].copy_from_slice(&mb0);
    }
    r.check("pdn two layers share a pixel block", ERR_HDR, || pdn_dec(&f, 2));
    // eight BitmapLayers (09 properties, 09 surface, 00, width, height, 09 Layer+properties) share
    // the LayerProperties of layer 0 with its 100 KB name
    let name = vec![b'n'; 100_000];
    let base = pdn_file(8, &name);
    r.check("pdn 8 layers, a 100 KB name, decode", 0, || pdn_dec(&base, 8));
    let mut f = base.clone();
    let bl: Vec<usize> = find(&f, &[0, 3, 0, 0, 0, 2, 0, 0, 0, 9]).into_iter().filter(|&i| i >= 10 && f[i - 5] == 9 && f[i - 10] == 9).collect();
    if bl.len() == 8 {
        let lp0: [u8; 4] = f[bl[0] + 10..bl[0] + 14].try_into().unwrap();
        for &i in &bl[1..] {
            f[i + 10..i + 14].copy_from_slice(&lp0);
        }
    }
    r.check("pdn 8 layers share the properties with a 100 KB name", ERR_LIMIT, || pdn_dec(&f, 8));
}
