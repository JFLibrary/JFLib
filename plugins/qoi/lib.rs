// qoi -- example file type plugin "QOI" (Quite OK Image format) in Rust
//                                                         [apps/PaintCraft/plugins/qoi/qoi.s]
//
//   python build_rs.py plugin qoi  -> build/<target>/plugins/qoi.dll / .so  (FileTypes folder)
//
// Decoder + encoder (qoiformat.org, specification 1.0): header "qoif", width / height (big
// endian), channels 3 / 4, colour space; chunks QOI_OP_RGB / RGBA / INDEX / DIFF / LUMA / RUN;
// end marker 7 x 00 + 01.  The decoder never reads beyond the data (a short file ends early
// with the last pixel, like the reference decoder); its surface comes from PA_SURFALLOC
// (Decoder::alloc, with the limits of the decode options).  The encoder writes 4 channels into
// a PA_ALLOC block (Block) the host frees.  Files written are byte-identical to the assembly
// plugin's.  One difference: the assembly decoder needs rows written back to back (width a
// multiple of 16, else ERR_UNSUPPORTED); this one writes each row at the surface's stride, so
// any width decodes.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

const QOI_HDR: usize = 14;
const QOI_PAD: usize = 8;
const OP_INDEX: u8 = 0x00;
const OP_DIFF: u8 = 0x40;
const OP_LUMA: u8 = 0x80;
const OP_RUN: u8 = 0xC0;
const OP_RGB: u8 = 0xFE;
const OP_RGBA: u8 = 0xFF;

struct Qoi;

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

// index slot of a pixel: (r * 3 + g * 5 + b * 7 + a * 11) % 64
fn hash(px: u32) -> usize {
    let (a, r, g, b) = (px >> 24, (px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF);
    ((r * 3 + g * 5 + b * 7 + a * 11) & 63) as usize
}

impl FileType for Qoi {
    const PROBE: bool = true;
    const DECODE: bool = true;
    const ENCODE: bool = true;

    // data starts with a QOI header
    fn probe(data: &[u8], meta: Option<&mut ImgMeta>) -> bool {
        if data.len() < QOI_HDR || data[..4] != *b"qoif" {
            return false;
        }
        if let Some(m) = meta {
            m.w = be32(data, 4);
            m.h = be32(data, 8);
            m.channels = data[12];
            m.bits = 8;
            m.model = IMM_RGB;
            m.flags |= IMF_LOSSLESS;
        }
        true
    }

    // straight FMT_ARGB
    fn decode(data: &[u8], d: &mut Decoder<'_>) -> Result<(), i32> {
        let len = data.len();
        if len < QOI_HDR + QOI_PAD {
            return Err(ERR_TRUNC);
        }
        if data[..4] != *b"qoif" || !(3..=4).contains(&data[12]) {
            return Err(ERR_HDR);
        }
        let (w, h) = (be32(data, 4), be32(data, 8));
        if w == 0 || h == 0 || w > 65535 || h > 65535 {
            return Err(ERR_DIMS);
        }
        d.alloc(w as i32, h as i32)?;
        let mut index = [0u32; 64];
        let mut px: u32 = 0xFF00_0000; // current pixel 0xAARRGGBB
        let mut run = 0u32;
        let mut p = QOI_HDR; // read position
        let end = len - QOI_PAD; // end of the chunks
        for y in 0..h as i32 {
            for out in d.row_mut(y).iter_mut() {
                if run > 0 {
                    run -= 1;
                } else if p < end {
                    // (no chunks left: repeat the last pixel)
                    let b1 = data[p];
                    p += 1;
                    match b1 {
                        OP_RGB => {
                            if p + 3 > len {
                                return Err(ERR_TRUNC); // the host frees the surface
                            }
                            px = (px & 0xFF00_0000) | (data[p] as u32) << 16 | (data[p + 1] as u32) << 8 | data[p + 2] as u32;
                            p += 3;
                        }
                        OP_RGBA => {
                            if p + 4 > len {
                                return Err(ERR_TRUNC);
                            }
                            px = (data[p + 3] as u32) << 24
                                | (data[p] as u32) << 16
                                | (data[p + 1] as u32) << 8
                                | data[p + 2] as u32;
                            p += 4;
                        }
                        _ => match b1 & 0xC0 {
                            OP_INDEX => px = index[(b1 & 63) as usize],
                            OP_DIFF => {
                                let dr = ((b1 >> 4) & 3) as i32 - 2;
                                let dg = ((b1 >> 2) & 3) as i32 - 2;
                                let db = (b1 & 3) as i32 - 2;
                                px = add(px, dr, dg, db);
                            }
                            OP_LUMA => {
                                if p >= len {
                                    return Err(ERR_TRUNC);
                                }
                                let vg = (b1 & 0x3F) as i32 - 32;
                                let b2 = data[p] as i32;
                                p += 1;
                                px = add(px, vg - 8 + (b2 >> 4), vg, vg - 8 + (b2 & 15));
                            }
                            _ => run = (b1 & 0x3F) as u32, // OP_RUN
                        },
                    }
                    index[hash(px)] = px;
                }
                *out = px;
            }
        }
        Ok(())
    }

    // 4 channels, colour space 0 (sRGB with linear alpha)
    fn encode(img: &Image<'_>, _opt: &ImgOpt) -> Result<Block, i32> {
        let (w, h) = (img.width() as u32, img.height() as u32);
        let total = w.wrapping_mul(h) as u64; // pixels
        let mut o = Block::new((total * 5) as usize + QOI_HDR + QOI_PAD).ok_or(ERR_NOMEM)?;
        o.extend(b"qoif");
        o.extend(&w.to_be_bytes());
        o.extend(&h.to_be_bytes());
        o.extend(&[4, 0]);
        let mut index = [0u32; 64];
        let mut prev: u32 = 0xFF00_0000;
        let mut run = 0u32;
        let mut done = 0u64;
        for y in 0..h as i32 {
            for &px in img.row(y) {
                done += 1;
                if px == prev {
                    // a run of the previous pixel
                    run += 1;
                    if run == 62 || done == total {
                        o.push(OP_RUN + (run - 1) as u8);
                        run = 0;
                    }
                    continue;
                }
                if run > 0 {
                    o.push(OP_RUN + (run - 1) as u8);
                    run = 0;
                }
                let k = hash(px);
                let [b, g, r, a] = px.to_le_bytes();
                if index[k] == px {
                    o.push(OP_INDEX | k as u8);
                } else {
                    index[k] = px;
                    if (px ^ prev) & 0xFF00_0000 != 0 {
                        o.extend(&[OP_RGBA, r, g, b, a]);
                    } else {
                        // vr, vg, vb as signed byte differences
                        let [pb, pg, pr, _] = prev.to_le_bytes();
                        let vr = r.wrapping_sub(pr) as i8 as i32;
                        let vg = g.wrapping_sub(pg) as i8 as i32;
                        let vb = b.wrapping_sub(pb) as i8 as i32;
                        let (vgr, vgb) = (vr - vg, vb - vg);
                        if (-2..=1).contains(&vr) && (-2..=1).contains(&vg) && (-2..=1).contains(&vb) {
                            o.push(OP_DIFF | ((vr + 2) << 4 | (vg + 2) << 2 | (vb + 2)) as u8);
                        } else if (-32..=31).contains(&vg) && (-8..=7).contains(&vgr) && (-8..=7).contains(&vgb) {
                            o.extend(&[OP_LUMA | (vg + 32) as u8, ((vgr + 8) << 4 | (vgb + 8)) as u8]);
                        } else {
                            o.extend(&[OP_RGB, r, g, b]);
                        }
                    }
                }
                prev = px;
            }
        }
        o.extend(&[0, 0, 0, 0, 0, 0, 0, 1]); // end marker
        Ok(o)
    }
}

// add(px, dr, dg, db) -- r += dr, g += dg, b += db modulo 256, alpha kept
fn add(px: u32, dr: i32, dg: i32, db: i32) -> u32 {
    let r = (px >> 16).wrapping_add(dr as u32) & 0xFF;
    let g = (px >> 8).wrapping_add(dg as u32) & 0xFF;
    let b = px.wrapping_add(db as u32) & 0xFF;
    (px & 0xFF00_0000) | r << 16 | g << 8 | b
}

static FT_LIST: [FtDesc; 1] = [FtDesc::new::<Qoi>(c"QOI", c"qoi", Some(c"image/qoi"), IFC_ALPHA | IFC_LOSSLESS)];
static INFO: PluginInfo = PluginInfo::new(c"QOI-Format (Beispiel)", c"1.0")
    .author(c"jfguiLib")
    .url(c"https://qoiformat.org")
    .file_types(&FT_LIST);

// keeps the API table (decode / encode need it): an older host is refused
fn query(api: &'static PnApi) -> Option<&'static PluginInfo> {
    if api.size < PA_SIZE_V1 {
        return None;
    }
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
