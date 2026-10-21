// ops_webp -- codectest ops 120..139: WebP (img_webp)  [apps/codectest/src/webp_ops.s]
//
//   op 120 time img_decode_ex   data = file, arg = repetitions | IMGD_* flags << 16
//                               -> {u32 microseconds per call, u32 w, u32 h} (result: error)
//   op 121 vp8l_build_table     data = {u32 n, u32 root_bits, u8 lengths[n]} (n <= 2328)
//                               -> result = entries (0 invalid), bytes = u32 table[entries]
//   op 122 vp8l_decode          data = VP8L chunk payload -> surface record
//   op 123 webp_alpha           data = {u32 w, u32 h} + ALPH payload -> u8 plane[w * h]
//   op 124 vp8_decode           data = VP8 chunk payload -> surface record
//   op 125 webp_parse           data = file -> Info scalars {u32 cw, ch, flags, kind, iw, ih,
//                               fx, fy, frames, loops, dur, bg, alpha, anim, vp8x; u64 ilen,
//                               iavail, alen, icclen, exiflen, xmplen}
//   op 126 vp8l_enc_argb        data = {u32 w, h, effort, flags} + u32 pixels[w * h]
//                               -> result, VP8L stream (flags = Cfg flags incl. test bits;
//                               bit 31 of flags: the cancel flag is set from the start)
//   op 127 vp8_enc_argb         data = {u32 w, h, effort, flags, quality, preset} + pixels
//                               -> result, VP8 stream
//   op 128 webp_enc_alpha       data = {u32 w, h, effort, flags} + pixels -> result, ALPH payload
//   op 129 time img_encode      data = IMGOPT (64 bytes, pointers ignored) + surface record,
//                               arg = repetitions -> {u32 microseconds per call, u32 file size}
//
// A failed decode of op 122 / 124 records 12 zero bytes (the cleared surface record).

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::AtomicU32;

use crate::{Ct, surface_in, u32_at};
use img_core::io::{decode_ex, encode};
use img_core::{ImgDec, ImgOpt};
use img_webp::{Cfg, container, vp8, vp8_enc, vp8l, vp8l_enc};

const OP_WTIME: u32 = 120;
const OP_WBUILD: u32 = 121;
const OP_WVP8L: u32 = 122;
const OP_WALPHA: u32 = 123;
const OP_WVP8: u32 = 124;
const OP_WPARSE: u32 = 125;
const OP_WEL: u32 = 126;
const OP_WEV: u32 = 127;
const OP_WEA: u32 = 128;
const OP_WETIME: u32 = 129;
const WB_MAXN: u32 = 2328; // largest alphabet (vp8l_build_table)
const WB_TAB: usize = 8192; // table entries (>= any valid table)
const IO_SIZE: usize = 64;
const R_BAD: i32 = -100;

fn reps(arg: u32) -> u32 {
    match arg & 0xFFFF {
        0 => 1,
        n => n,
    }
}

/// run op (one record via ct.record); false = unknown op  [webp_op]
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_WTIME => {
            let n = reps(arg);
            let dec = ImgDec { flags: arg >> 16, ..Default::default() };
            let t0 = plat::time_us();
            let (mut w, mut h) = (0u32, 0u32);
            for _ in 0..n {
                match decode_ex(data, Some(&dec), None) {
                    Ok(sf) => (w, h) = (sf.w as u32, sf.h as u32),
                    Err(e) => {
                        ct.record(e, &[]);
                        return true;
                    }
                }
            }
            let us = ((plat::time_us() - t0) / n as u64) as u32;
            let mut rec = Vec::with_capacity(12);
            for v in [us, w, h] {
                rec.extend_from_slice(&v.to_le_bytes());
            }
            ct.record(0, &rec);
        }
        OP_WBUILD => {
            if data.len() < 8 {
                ct.record(R_BAD, &[]);
                return true;
            }
            let (n, rb) = (u32_at(data, 0), u32_at(data, 4));
            if n.wrapping_sub(1) > WB_MAXN - 1 || rb.wrapping_sub(1) > 7 || n as usize + 8 > data.len() {
                ct.record(R_BAD, &[]);
                return true;
            }
            let mut tab = vec![0u32; WB_TAB];
            let e = vp8l::build_table(&data[8..8 + n as usize], rb, Some(&mut tab));
            let mut out = Vec::with_capacity(e as usize * 4);
            for v in &tab[..e as usize] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            ct.record(e as i32, &out);
        }
        OP_WVP8L | OP_WVP8 => {
            let dec = ImgDec::default();
            let r = if op == OP_WVP8L { vp8l::decode(data, Some(&dec)) } else { vp8::decode(data, Some(&dec)) };
            match r {
                Ok(sf) => ct.surface_out(0, &sf),
                Err(e) => ct.record(e, &[0; 12]),
            }
        }
        OP_WALPHA => {
            if data.len() < 8 {
                ct.record(R_BAD, &[]);
                return true;
            }
            let (w, h) = (u32_at(data, 0), u32_at(data, 4));
            if w.wrapping_sub(1) >= 16383 || h.wrapping_sub(1) >= 16383 {
                ct.record(R_BAD, &[]);
                return true;
            }
            let n = w as usize * h as usize;
            let mut plane = vec![0u8; n];
            let dec = ImgDec::default();
            let r = container::alpha(&data[8..], &mut plane, w, h, Some(&dec));
            ct.record(r, if r == 0 { &plane } else { &[] });
        }
        OP_WPARSE => {
            let (r, i) = container::parse(data);
            let mut rec = Vec::with_capacity(15 * 4 + 6 * 8);
            for v in [i.cw, i.ch, i.flags, i.kind, i.iw, i.ih, i.fx, i.fy, i.frames, i.loops, i.dur, i.bg, i.alpha, i.anim, i.vp8x] {
                rec.extend_from_slice(&v.to_le_bytes());
            }
            for v in [i.ilen, i.iavail, i.alen, i.icclen, i.exiflen, i.xmplen] {
                rec.extend_from_slice(&v.to_le_bytes());
            }
            ct.record(r, &rec);
        }
        OP_WEL | OP_WEV | OP_WEA => {
            // {w, h, effort, flags [, quality, preset]} + pixels
            let hl = if op == OP_WEV { 24 } else { 16 };
            if data.len() < hl {
                ct.record(R_BAD, &[]);
                return true;
            }
            let (w, h) = (u32_at(data, 0), u32_at(data, 4));
            if w.wrapping_sub(1) > 16383 || h.wrapping_sub(1) > 16383 {
                ct.record(R_BAD, &[]);
                return true;
            }
            let n = w as usize * h as usize;
            if data.len() < hl + n * 4 {
                ct.record(R_BAD, &[]);
                return true;
            }
            let cancel = AtomicU32::new(1);
            let mut fl = u32_at(data, 12);
            let mut cfg = Cfg { effort: u32_at(data, 8), ..Default::default() };
            if fl & 0x8000_0000 != 0 {
                fl &= !0x8000_0000; // test: bit 31 = cancelled from the start
                cfg.cancel = Some(&cancel);
            }
            cfg.flags = fl;
            if op == OP_WEV {
                cfg.quality = u32_at(data, 16);
                cfg.preset = u32_at(data, 20);
            }
            let px: Vec<u32> = data[hl..hl + n * 4].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            let r = match op {
                OP_WEL => vp8l_enc::enc_argb_ex(&px, w, h, &cfg),
                OP_WEV => vp8_enc::enc_argb(&px, w, h, &cfg).map(|v| (0, v)),
                _ => container::enc_alpha(&px, w, h, &cfg).map(|v| (0, v)),
            };
            match r {
                Ok((res, v)) => ct.record(res, &v),
                Err(e) => ct.record(e, &[]),
            }
        }
        OP_WETIME => {
            if data.len() < IO_SIZE {
                ct.record(R_BAD, &[]);
                return true;
            }
            let opt = ImgOpt::from_bytes(&data[..IO_SIZE]);
            let Some(sf) = surface_in(&data[IO_SIZE..]) else {
                ct.record(-1, &[]); // ct_surface_in failed
                return true;
            };
            let n = reps(arg);
            let t0 = plat::time_us();
            let mut size = 0u32;
            for _ in 0..n {
                match encode(&sf, Some(&opt)) {
                    Ok(f) => size = f.len() as u32,
                    Err(e) => {
                        ct.record(e, &[0; 8]);
                        return true;
                    }
                }
            }
            let us = ((plat::time_us() - t0) / n as u64) as u32;
            let mut rec = Vec::with_capacity(8);
            rec.extend_from_slice(&us.to_le_bytes());
            rec.extend_from_slice(&size.to_le_bytes());
            ct.record(0, &rec);
        }
        _ => return false,
    }
    true
}
