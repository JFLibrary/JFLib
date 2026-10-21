// ops_dds -- codectest batch ops 100..119: DDS and the BC block codecs  [apps/codectest/src/dds_ops.s]
//
//   op 100 decode_block      arg = bcfmt, data = n blocks -> 0 / ERR_*, n * 64 bytes (16 BGRA
//                            pixels per block, row-major)
//   op 101 bc6h_decode_half  arg = signed, data = n blocks of 16 bytes -> n * 128 bytes
//                            (16 x r, g, b, a IEEE halves)
//   op 102 decode            arg = bcfmt, data = {u32 w, h} + blocks -> surface record
//   op 103 time decode       arg = bcfmt | reps << 8, data as op 102 -> u32 us per call, w, h
//   op 104 time decode_ex    arg = reps, data = file (IMGD_STRAIGHT) -> u32 us per call, w, h
//   op 105 decode_to         arg = bcfmt, data as op 102, cancel flag set -> result code
//   op 106 decode_ex         arg = frame, data = file; IMGD_PROBE | IMGD_STRAIGHT (arg bit 31
//                            clear) or a full decode (bit 31 set) -> the raw IMGMETA (128)
//   op 107 encode_block      arg = bcfmt | cfg << 8, data = n x 64 bytes (16 straight BGRA
//                            pixels per block) -> n blocks
//   op 108 bc encode         arg = bcfmt | cfg << 8, data = surface record -> result, blocks
//   op 109 time bc encode    arg as 108, data = u32 reps + surface -> u32 us per call, u32 bytes
//   op 110 encode_to         arg as 108, data = surface, cancel flag set -> result code
//   op 111 synthetic encode  data = IMGOPT (64) + {u32 w, h, pattern, reps}: FMT_ARGB surface
//                            (pattern 0 smooth + noise, opaque; 1 random bytes; | 256: the
//                            cancel flag is set) -> io::encode: result, {u64 len, u32 us per
//                            call, u32 FNV-64 low}
//   op 112 time io::encode   arg = reps, data = IMGOPT + surface -> {u32 us per call, u32 len}
//
// Records with times (103, 104, 109, 111, 112) differ from the assembly by nature.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU32;

use gfx::{FMT_ARGB, Surface};
use img_core::{ERR_HDR, ERR_NOMEM, ERR_UNSUPPORTED, IMGD_PROBE, IMGD_STRAIGHT, ImgDec, ImgMeta, ImgOpt};

use crate::{Ct, surface_in, u32_at};

const OP_BCBLOCK: u32 = 100;
const OP_BC6HALF: u32 = 101;
const OP_BCDEC: u32 = 102;
const OP_BCDEC_T: u32 = 103;
const OP_DECEX_T: u32 = 104;
const OP_BCCANCEL: u32 = 105;
const OP_RAWMETA: u32 = 106;
const OP_BCENC_BLK: u32 = 107;
const OP_BCENC: u32 = 108;
const OP_BCENC_T: u32 = 109;
const OP_BCENC_CAN: u32 = 110;
const OP_ENC_SYN: u32 = 111;
const OP_ENC_T: u32 = 112;
const IO_SIZE: usize = 64;

/// hash_fnv64(data, 0): FNV-1a 64
fn fnv64(d: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// a zeroed surface, None when the size is invalid or the memory is not there (surf_alloc)
fn try_surface(w: u32, h: u32, fmt: u8) -> Option<Surface> {
    if w > 65536 || h > 65536 {
        return None;
    }
    let stride = ((w as usize * 4) + 63) & !63;
    let bytes = (stride * h as usize).max(64);
    let mut px = Vec::new();
    px.try_reserve_exact(bytes).ok()?;
    px.resize(bytes, 0);
    Some(Surface { px, w: w as i32, h: h as i32, stride: stride as i32, fmt, flags: gfx::SFF_OWNED, opac: 255, blend: 0 })
}

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    let cfg = arg >> 8;
    match op {
        OP_BCBLOCK | OP_BC6HALF => {
            let bs = if op == OP_BC6HALF { 16 } else { img_dds::block_bytes(arg) };
            if bs == 0 {
                ct.record(ERR_UNSUPPORTED, &[]);
                return true;
            }
            if data.len() % bs != 0 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let n = data.len() / bs;
            if op == OP_BC6HALF {
                let mut out = vec![0u8; n * 128];
                let mut h = [0u16; 64];
                for (b, o) in data.chunks_exact(16).zip(out.chunks_exact_mut(128)) {
                    img_dds::bc6h_decode_half(b, arg != 0, &mut h);
                    for (k, v) in h.iter().enumerate() {
                        o[k * 2..k * 2 + 2].copy_from_slice(&v.to_le_bytes());
                    }
                }
                ct.record(0, &out);
            } else {
                let mut out = vec![0u8; n * 64];
                for (b, o) in data.chunks_exact(bs).zip(out.chunks_exact_mut(64)) {
                    img_dds::decode_block(arg, b, o, 16);
                }
                ct.record(0, &out);
            }
        }
        OP_BCDEC => {
            if data.len() < 8 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            match img_dds::decode(&data[8..], u32_at(data, 0), u32_at(data, 4), arg) {
                Ok(sf) => ct.surface_out(0, &sf),
                Err(e) => ct.surface_out(e, &Surface::default()),
            }
        }
        OP_BCDEC_T => {
            if data.len() < 8 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let reps = (arg >> 8).max(1);
            let fmt = arg & 0xFF;
            let mut rec = [0u8; 12];
            let t0 = plat::time_us();
            for _ in 0..reps {
                match img_dds::decode(&data[8..], u32_at(data, 0), u32_at(data, 4), fmt) {
                    Ok(sf) => {
                        rec[4..8].copy_from_slice(&(sf.w as u32).to_le_bytes());
                        rec[8..12].copy_from_slice(&(sf.h as u32).to_le_bytes());
                    }
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = (plat::time_us() - t0) / reps as u64;
            rec[0..4].copy_from_slice(&(us as u32).to_le_bytes());
            ct.record(0, &rec);
        }
        OP_DECEX_T => {
            let reps = arg.max(1);
            let dec = ImgDec { flags: IMGD_STRAIGHT, ..Default::default() };
            let mut rec = [0u8; 12];
            let t0 = plat::time_us();
            for _ in 0..reps {
                match img_core::io::decode_ex(data, Some(&dec), None) {
                    Ok(sf) => {
                        rec[4..8].copy_from_slice(&(sf.w as u32).to_le_bytes());
                        rec[8..12].copy_from_slice(&(sf.h as u32).to_le_bytes());
                    }
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = (plat::time_us() - t0) / reps as u64;
            rec[0..4].copy_from_slice(&(us as u32).to_le_bytes());
            ct.record(0, &rec);
        }
        OP_BCCANCEL => {
            if data.len() < 8 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let (w, h) = (u32_at(data, 0), u32_at(data, 4));
            let Some(mut sf) = try_surface(w, h, FMT_ARGB) else {
                ct.record(ERR_NOMEM, &[]);
                return true;
            };
            let flag = AtomicU32::new(1);
            let st = sf.stride as usize;
            let r = img_dds::decode_to(&data[8..], arg, &mut sf.px, st, w, h, Some(&flag));
            ct.record(r, &[]);
        }
        OP_RAWMETA => {
            let flags = if arg & 0x8000_0000 != 0 { IMGD_STRAIGHT } else { IMGD_PROBE | IMGD_STRAIGHT };
            let dec = ImgDec { flags, frame: arg & 0x7FFF_FFFF, ..Default::default() };
            let mut meta = ImgMeta::default();
            let r = match img_core::io::decode_ex(data, Some(&dec), Some(&mut meta)) {
                Ok(_) => 0,
                Err(e) => e,
            };
            ct.record(r, &meta.to_bytes());
        }
        OP_BCENC_BLK => {
            let fmt = arg & 0xFF;
            let bs = img_dds::block_bytes(fmt);
            if bs == 0 {
                ct.record(ERR_UNSUPPORTED, &[]);
                return true;
            }
            if data.len() & 63 != 0 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let n = data.len() >> 6;
            let mut out = vec![0u8; n * bs];
            for (px, o) in data.chunks_exact(64).zip(out.chunks_exact_mut(bs)) {
                let mut p = [0u8; 64];
                p.copy_from_slice(px);
                let r = img_dds::encode_block(fmt, &p, o, cfg);
                if r != 0 {
                    ct.record(r, &[]);
                    return true;
                }
            }
            ct.record(0, &out);
        }
        OP_BCENC => {
            let Some(sf) = surface_in(data) else {
                ct.record(ERR_HDR, &[]);
                return true;
            };
            match img_dds::bc_encode(&sf, arg & 0xFF, cfg) {
                Ok(b) => ct.record(0, &b),
                Err(e) => ct.record(e, &[]),
            }
        }
        OP_BCENC_T => {
            if data.len() < 4 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let reps = u32_at(data, 0).max(1);
            let Some(sf) = surface_in(&data[4..]) else {
                ct.record(ERR_HDR, &[]);
                return true;
            };
            let t0 = plat::time_us();
            let mut len = 0;
            for _ in 0..reps {
                match img_dds::bc_encode(&sf, arg & 0xFF, cfg) {
                    Ok(b) => len = b.len(),
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = (plat::time_us() - t0) / reps as u64;
            let mut rec = [0u8; 8];
            rec[0..4].copy_from_slice(&(us as u32).to_le_bytes());
            rec[4..8].copy_from_slice(&(len as u32).to_le_bytes());
            ct.record(0, &rec);
        }
        OP_BCENC_CAN => {
            let Some(sf) = surface_in(data) else {
                ct.record(ERR_HDR, &[]);
                return true;
            };
            let n = img_dds::encoded_size(sf.w as u32, sf.h as u32, arg & 0xFF) as usize;
            let mut buf = Vec::new();
            if buf.try_reserve_exact(n + 16).is_err() {
                ct.record(ERR_NOMEM, &[]);
                return true;
            }
            buf.resize(n + 16, 0);
            let flag = AtomicU32::new(1);
            let r = img_dds::encode_to(&sf, arg & 0xFF, cfg, &mut buf[..n], Some(&flag));
            ct.record(r, &[]);
        }
        OP_ENC_SYN => {
            if data.len() < IO_SIZE + 16 {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let flag = AtomicU32::new(1);
            let mut opt = ImgOpt::from_bytes(&data[..IO_SIZE]);
            let reps = u32_at(data, IO_SIZE + 12).max(1);
            let pat = u32_at(data, IO_SIZE + 8);
            let Some(mut sf) = try_surface(u32_at(data, IO_SIZE), u32_at(data, IO_SIZE + 4), FMT_ARGB) else {
                ct.record(ERR_NOMEM, &[]);
                return true;
            };
            pattern(&mut sf, pat & 255);
            if pat & 256 != 0 {
                opt.cancel = Some(&flag); // pattern | 256: the cancel flag is set
            }
            let t0 = plat::time_us();
            let mut buf = Vec::new();
            for _ in 0..reps {
                match img_core::io::encode(&sf, Some(&opt)) {
                    Ok(b) => buf = b,
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = (plat::time_us() - t0) / reps as u64;
            let mut rec = [0u8; 16];
            rec[0..8].copy_from_slice(&(buf.len() as u64).to_le_bytes());
            rec[8..12].copy_from_slice(&(us as u32).to_le_bytes());
            rec[12..16].copy_from_slice(&(fnv64(&buf) as u32).to_le_bytes());
            ct.record(0, &rec);
        }
        OP_ENC_T => {
            if data.len() < IO_SIZE {
                ct.record(ERR_HDR, &[]);
                return true;
            }
            let reps = arg.max(1);
            let opt = ImgOpt::from_bytes(&data[..IO_SIZE]);
            let Some(sf) = surface_in(&data[IO_SIZE..]) else {
                ct.record(ERR_HDR, &[]);
                return true;
            };
            let t0 = plat::time_us();
            let mut len = 0;
            for _ in 0..reps {
                match img_core::io::encode(&sf, Some(&opt)) {
                    Ok(b) => len = b.len(),
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = (plat::time_us() - t0) / reps as u64;
            let mut rec = [0u8; 8];
            rec[0..4].copy_from_slice(&(us as u32).to_le_bytes());
            rec[4..8].copy_from_slice(&(len as u32).to_le_bytes());
            ct.record(0, &rec);
        }
        _ => return false,
    }
    true
}

/// fill a 32-bit surface: 0 = smooth gradients + a little noise (opaque), 1 = random bytes
/// [do_pattern]
fn pattern(sf: &mut Surface, pat: u32) {
    let (w, h, st) = (sf.w as u32, sf.h as u32, sf.stride as usize);
    for y in 0..h {
        for x in 0..w {
            let mut v = x.wrapping_mul(0x9E37_79B1).wrapping_add(y.wrapping_mul(0x85EB_CA6B));
            v ^= v >> 15;
            v = v.wrapping_mul(0x2C1B_3C6D);
            v ^= v >> 12; // hash
            if pat == 0 {
                let noise = v & 15;
                let b = (x + y) & 255;
                let g = (((x >> 2) ^ (y >> 3)) + noise) & 255;
                let r = (y & 127) + noise;
                v = b | g << 8 | r << 16 | 0xFF00_0000;
            }
            let o = y as usize * st + x as usize * 4;
            sf.px[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}
