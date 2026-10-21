// ops_jpeg -- codectest batch ops 20..39 (JPEG)  [apps/codectest/src/jpeg_ops.s]
//
//   op 20 jpeg_decode                  -> u32 w, u32 h, w*h PARGB u32 (like op 6)
//   op 21 jpeg_encode  arg = quality, data = u32 w, h, fmt, packed pixels -> file bytes
//   op 22 jpeg_decode                  -> u32 w, u32 h, u32 fmt | flags << 8, u64 FNV-1a of the rows
//   op 23 decode bench arg = repetitions -> u32 w, u32 h, u64 microseconds (all repetitions)
//   op 24 encode bench arg = quality | repetitions << 8, data as op 21
//                                      -> u64 microseconds (all repetitions), u64 file length
//   op 25 jpeg_decode_fl     arg = JDF_* flags, data = JPEG -> surface record
//   op 26 jpeg_decode_abbrev arg = JDF_* flags, data = u32 tlen + tables + image stream
//                                      -> surface record
//   op 27 jpeg_encode_ex     arg = quality, data = {u32 chroma, unit, xd, yd} + surface
//                                      {w, h, fmt, pixels} -> file bytes
//
// Surface records (ops 25, 26): {result, {u32 w, h, fmt | flags << 8, packed rows}}.

use alloc::vec::Vec;

use gfx::{Surface, FMT_A8};
use img_core::{ERR_NOMEM, ERR_TRUNC};

use crate::{u32_at, Ct};

const OP_JDEC: u32 = 20;
const OP_JENC: u32 = 21;
const OP_JHASH: u32 = 22;
const OP_JBDEC: u32 = 23;
const OP_JBENC: u32 = 24;
const OP_JDECFL: u32 = 25;
const OP_JABBREV: u32 = 26;
const OP_JENCEX: u32 = 27;

/// hash_fnv64(data, seed): FNV-1a 64 (seed 0 = offset basis)
fn fnv64(d: &[u8], seed: u64) -> u64 {
    let mut h = if seed == 0 { 0xCBF2_9CE4_8422_2325 } else { seed };
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// row bytes of a surface in the records (4 per pixel, 1 for FMT_A8)
fn row_bytes(sf: &Surface) -> usize {
    sf.w as usize * if sf.fmt == FMT_A8 { 1 } else { 4 }
}

/// .jo_mksurf: a new surface from {u32 w, h, fmt, packed rows}; None when the data does not
/// hold all rows or the size is invalid.  Unlike crate::surface_in the length is checked
/// before the allocation (the assembly's surf_alloc fails cleanly for 65536 x 65536; a Rust
/// allocation of that size would abort)
fn surface_in(d: &[u8]) -> Option<Surface> {
    if d.len() < 12 {
        return None;
    }
    let (w, h, fmt) = (u32_at(d, 0), u32_at(d, 4), u32_at(d, 8) as u8);
    if w > 65536 || h > 65536 {
        return None; // surf_alloc fails
    }
    let rb = w as usize * if fmt == FMT_A8 { 1 } else { 4 };
    if rb as u64 * h as u64 + 12 > d.len() as u64 {
        return None;
    }
    let mut sf = Surface::new(w as i32, h as i32, fmt);
    let st = sf.stride as usize;
    for y in 0..h as usize {
        sf.px[y * st..y * st + rb].copy_from_slice(&d[12 + y * rb..12 + (y + 1) * rb]);
    }
    Some(sf)
}

/// ct_surface_out of a decode result; a failed decode records 12 zero bytes (the assembly
/// writes its uninitialised surface record there)
fn surface_out(ct: &mut Ct, r: Result<Surface, i32>) {
    match r {
        Ok(sf) => ct.surface_out(0, &sf),
        Err(e) => ct.record(e, &[0; 12]),
    }
}

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_JDEC | OP_JHASH => match img_jpeg::decode(data) {
            Err(e) => ct.record(e, &[]),
            Ok(sf) if op == OP_JHASH => {
                let mut h = 0u64;
                let (rb, st) = (row_bytes(&sf), sf.stride as usize);
                for y in 0..sf.h as usize {
                    h = fnv64(&sf.px[y * st..y * st + rb], h);
                }
                let mut out = Vec::new();
                out.extend_from_slice(&(sf.w as u32).to_le_bytes());
                out.extend_from_slice(&(sf.h as u32).to_le_bytes());
                out.extend_from_slice(&(sf.fmt as u32 | (sf.flags as u32) << 8).to_le_bytes());
                out.extend_from_slice(&h.to_le_bytes());
                ct.record(0, &out);
            }
            Ok(sf) => {
                let mut out = Vec::with_capacity(sf.w as usize * sf.h as usize * 4 + 8);
                out.extend_from_slice(&(sf.w as u32).to_le_bytes());
                out.extend_from_slice(&(sf.h as u32).to_le_bytes());
                let (rb, st) = (sf.w as usize * 4, sf.stride as usize);
                for y in 0..sf.h as usize {
                    out.extend_from_slice(&sf.px[y * st..y * st + rb]);
                }
                ct.record(0, &out);
            }
        },
        OP_JENC => match surface_in(data) {
            None => ct.record(-1, &[]),
            Some(sf) => match img_jpeg::encode(&sf, arg as i32) {
                Some(f) => ct.record(0, &f),
                None => ct.record(ERR_NOMEM, &[]),
            },
        },
        OP_JBDEC => {
            let reps = arg.max(1);
            let t0 = plat::time_us();
            let mut last = None;
            for _ in 0..reps {
                match img_jpeg::decode(data) {
                    Ok(sf) => last = Some(sf),
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = plat::time_us() - t0;
            let sf = last.unwrap_or_else(|| Surface::new(0, 0, 0));
            let mut out = Vec::new();
            out.extend_from_slice(&(sf.w as u32).to_le_bytes());
            out.extend_from_slice(&(sf.h as u32).to_le_bytes());
            out.extend_from_slice(&us.to_le_bytes());
            ct.record(0, &out);
        }
        OP_JBENC => {
            let Some(sf) = surface_in(data) else {
                ct.record(-1, &[]);
                return true;
            };
            let reps = (arg >> 8).max(1);
            let q = (arg & 0xFF) as i32;
            let t0 = plat::time_us();
            let mut len = 0u64;
            for _ in 0..reps {
                match img_jpeg::encode(&sf, q) {
                    Some(f) => len = f.len() as u64,
                    None => {
                        ct.record(ERR_NOMEM, &[]);
                        return true;
                    }
                }
            }
            let us = plat::time_us() - t0;
            let mut out = Vec::new();
            out.extend_from_slice(&us.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            ct.record(0, &out);
        }
        OP_JDECFL => surface_out(ct, img_jpeg::decode_fl(data, arg)),
        OP_JABBREV => {
            if data.len() < 4 {
                ct.record(ERR_TRUNC, &[]);
                return true;
            }
            let tl = u32_at(data, 0) as usize;
            if tl > data.len() - 4 {
                ct.record(-100, &[]);
                return true;
            }
            surface_out(ct, img_jpeg::decode_abbrev(&data[4..4 + tl], &data[4 + tl..], arg));
        }
        OP_JENCEX => {
            if data.len() < 16 {
                ct.record(-100, &[]);
                return true;
            }
            let Some(sf) = surface_in(&data[16..]) else {
                ct.record(-100, &[]);
                return true;
            };
            let cfg = img_jpeg::EncCfg {
                chroma: u32_at(data, 0),
                unit: u32_at(data, 4),
                xd: u32_at(data, 8),
                yd: u32_at(data, 12),
            };
            match img_jpeg::encode_ex(&sf, arg as i32, Some(&cfg)) {
                Some(f) => ct.record(0, &f),
                None => ct.record(ERR_NOMEM, &[]),
            }
        }
        _ => return false,
    }
    true
}
