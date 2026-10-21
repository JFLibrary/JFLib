// ops_gif -- codectest ops 40..59: GIF and the quantizer  [apps/codectest/src/gif_ops.s]
//
//   op 40 gif_decode       data = GIF file -> u32 w, u32 h, w*h PARGB u32 (like op 6)
//   op 41 gif_encode       data = u32 w, h, fmt, packed pixels (like op 8) -> GIF file
//   op 42 img_quantize     data = as op 41, arg = max_colors | dither << 16
//                          -> i32 colours, i32 transparent index, u32 pal[256], w*h indices
//   op 43 time gif_decode  data = GIF file, arg = repetitions -> u32 us per call, u32 w, u32 h
//   op 44 time gif_encode  data = as op 41, arg = repetitions -> u32 us per call, u32 bytes
//   op 45 img_quantize_ex  arg = max_colors, data = u32 cfg + surface (any format, surface_in)
//                          -> i32 colours, i32 transparent index, u32 pal[256], w*h indices
//   op 46 gif_encode_q     arg = qcfg, data = surface (surface_in) -> GIF file
//   op 47 gif_decode_fl    arg = JDF_* flags, data = GIF -> surface_out record

use alloc::vec::Vec;

use crate::{Ct, surface_in};
use gfx::{FMT_A8, Surface};
use img_core::{ERR_NOMEM, quantize};

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        40 => {
            let mut sf = Surface::default();
            match img_gif::decode(data, &mut sf) {
                0 => put_image(ct, &sf),
                e => ct.record(e, &[]),
            }
        }
        41 => match go_surface(data) {
            Some(sf) => match img_gif::encode(&sf) {
                Some(f) => ct.record(0, &f),
                None => ct.record(ERR_NOMEM, &[]),
            },
            None => ct.record(ERR_NOMEM, &[]),
        },
        42 => match go_surface(data) {
            Some(sf) => quant_record(ct, &sf, |sf, pal, idx| quantize::quantize(sf, (arg & 0xFFFF) as i32, arg >> 16, pal, idx)),
            None => ct.record(ERR_NOMEM, &[]),
        },
        43 => {
            let reps = arg.max(1);
            let t0 = plat::time_us();
            let mut rec = [0u32; 3];
            for _ in 0..reps {
                let mut sf = Surface::default();
                let e = img_gif::decode(data, &mut sf);
                if e != 0 {
                    ct.record(e, &[]);
                    return true;
                }
                rec[1] = sf.w as u32;
                rec[2] = sf.h as u32;
            }
            rec[0] = ((plat::time_us() - t0) / reps as u64) as u32;
            ct.record(0, &words(&rec));
        }
        44 => {
            let Some(sf) = go_surface(data) else {
                ct.record(ERR_NOMEM, &[]);
                return true;
            };
            let reps = arg.max(1);
            let t0 = plat::time_us();
            let mut len = 0usize;
            for _ in 0..reps {
                match img_gif::encode(&sf) {
                    Some(f) => len = f.len(),
                    None => {
                        ct.record(ERR_NOMEM, &[]);
                        return true;
                    }
                }
            }
            let us = ((plat::time_us() - t0) / reps as u64) as u32;
            ct.record(0, &words(&[us, len as u32, 0]));
        }
        45 => {
            let sf = if data.len() < 4 { None } else { surface_in(&data[4..]) };
            match sf {
                Some(sf) => {
                    let cfg = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    quant_record(ct, &sf, |sf, pal, idx| quantize::quantize_ex(sf, arg as i32, cfg, pal, idx))
                }
                None => ct.record(ERR_NOMEM, &[]),
            }
        }
        46 => match surface_in(data) {
            Some(sf) => match img_gif::encode_q(&sf, arg) {
                Some(f) => ct.record(0, &f),
                None => ct.record(ERR_NOMEM, &[]),
            },
            None => ct.record(ERR_NOMEM, &[]),
        },
        47 => {
            let mut sf = Surface::default();
            let e = img_gif::decode_fl(data, &mut sf, arg);
            ct.surface_out(e, &sf);
        }
        _ => return false,
    }
    true
}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

// .go_qrec: {i32 colours, i32 transparent index, u32 pal[256], w*h indices}
fn quant_record(ct: &mut Ct, sf: &Surface, q: impl Fn(&Surface, &mut [u32; 256], &mut [u8]) -> (u32, i32)) {
    let n = sf.w as usize * sf.h as usize;
    let mut pal = [0u32; 256];
    let mut idx = alloc::vec![0u8; n];
    let (c, t) = q(sf, &mut pal, &mut idx);
    let mut rec = Vec::with_capacity(8 + 1024 + n);
    rec.extend_from_slice(&c.to_le_bytes());
    rec.extend_from_slice(&t.to_le_bytes());
    rec.extend_from_slice(&words(&pal));
    rec.extend_from_slice(&idx);
    ct.record(0, &rec);
}

// go_put_image: record {0, 8 + w*h*4}, w, h and the pixel rows
fn put_image(ct: &mut Ct, sf: &Surface) {
    let (w, h) = (sf.w as usize, sf.h as usize);
    let mut rec = Vec::with_capacity(8 + w * h * 4);
    rec.extend_from_slice(&(w as u32).to_le_bytes());
    rec.extend_from_slice(&(h as u32).to_le_bytes());
    for y in 0..h {
        let o = y * sf.stride as usize;
        rec.extend_from_slice(&sf.px[o..o + w * 4]);
    }
    ct.record(0, &rec);
}

// go_surface: surface from {u32 w, h, fmt <= FMT_A8, packed rows} (4 bytes per pixel, 1 for
// FMT_A8); w, h 1..32768
fn go_surface(d: &[u8]) -> Option<Surface> {
    if d.len() < 12 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
    let (w, h, fmt) = (u(0), u(4), u(8));
    if fmt > FMT_A8 as u32 || w == 0 || h == 0 || w > 32768 || h > 32768 {
        return None;
    }
    let row = w as usize * if fmt == FMT_A8 as u32 { 1 } else { 4 };
    if d.len() < 12 + row * h as usize {
        return None;
    }
    let mut sf = Surface::new(w as i32, h as i32, fmt as u8);
    copy_rows(&mut sf, &d[12..], row);
    Some(sf)
}

fn copy_rows(sf: &mut Surface, src: &[u8], row: usize) {
    let stride = sf.stride as usize;
    for y in 0..sf.h as usize {
        sf.px[y * stride..y * stride + row].copy_from_slice(&src[y * row..y * row + row]);
    }
}
