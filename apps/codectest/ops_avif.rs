// ops_avif -- codectest ops 140..159: AVIF (avif.s, heic.s, av1*.s)   [apps/codectest/src/avif_ops.s]
//
//   140 OP_AV1DEC   data = AV1 OBU stream (low-overhead format); arg = AVS_* stage (bits 0..7) |
//                   1 << 8 no film grain: records {u32 w, h, BitDepth, subsampling_x, subsampling_y,
//                   mono_chrome; Y, U, V as u16 samples (chroma ( w + subX ) >> subX wide)}
//                   (Av1Dec::obus / finish)
//   141 OP_AV1ENC   data = {u32 w, h, bit depth, chroma (0 / 1 = 4:2:0 / 3), base_q_idx, CICP, tile
//                   side in superblocks, flags (the A1E order); Y, U, V as u16 samples (chroma
//                   (w + subX) >> subX wide)}: records the OBUs of img_av1::encode  (r9-av1enc)

use alloc::vec;
use alloc::vec::Vec;

use crate::{Ct, u32_at};
use img_av1::Av1Dec;
use img_core::ERR_TRUNC;

const OP_AV1DEC: u32 = 140;
const OP_AV1ENC: u32 = 141; // r9-av1enc

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_AV1DEC => {
            dec(ct, arg, data);
            true
        }
        OP_AV1ENC => {
            av1enc(ct, data); // r9-av1enc
            true
        }
        _ => false,
    }
}

// .ao_dec
fn dec(ct: &mut Ct, arg: u32, data: &[u8]) {
    let mut d = Av1Dec::new();
    d.set_stage(arg & 0xFF);
    d.set_nograin((arg >> 8) & 1);
    if let Err(e) = d.obus(data) {
        ct.record(e, &[]);
        return;
    }
    let pic = match d.finish() {
        Ok(p) => p,
        Err(e) => {
            ct.record(e, &[]);
            return;
        }
    };
    // chroma size
    let cw = (pic.w + pic.subx) >> pic.subx;
    let ch = (pic.h + pic.suby) >> pic.suby;
    let mut n = pic.w as usize * pic.h as usize;
    if pic.mono == 0 {
        n += 2 * cw as usize * ch as usize;
    }
    let mut out: Vec<u8> = vec![0; n * 2 + 24];
    for (i, v) in [pic.w, pic.h, pic.bd, pic.subx, pic.suby, pic.mono].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut o = 24;
    for p in 0..3 {
        let (w, h) = if p == 0 {
            (pic.w as usize, pic.h as usize)
        } else {
            if pic.mono != 0 {
                break;
            }
            (cw as usize, ch as usize)
        };
        let st = pic.strides[p] as usize;
        let pl = pic.planes[p];
        for y in 0..h {
            let d = &mut out[o..o + w * 2];
            for (b, &s) in d.chunks_exact_mut(2).zip(&pl[y * st..y * st + w]) {
                b.copy_from_slice(&s.to_le_bytes());
            }
            o += w * 2;
        }
    }
    ct.record(0, &out);
}

// ---- r9-av1enc: op 141 (.ao_enc)

// little-endian u16 samples of b
fn samples(b: &[u8]) -> Vec<u16> {
    b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

// op 141: av1_encode of the planes in data
fn av1enc(ct: &mut Ct, data: &[u8]) {
    if data.len() < 32 {
        ct.record(ERR_TRUNC, &[]);
        return;
    }
    let h8: [u32; 8] = core::array::from_fn(|i| u32_at(data, i * 4));
    let (w, h, chroma) = (h8[0] as u64, h8[1] as u64, h8[3]);
    let sub = (chroma == 1) as u32; // subX = subY
    let cw = (h8[0].wrapping_add(sub) >> sub) as u64;
    let chh = (h8[1].wrapping_add(sub) >> sub) as u64;
    let ybytes = w * h * 2;
    let cbytes = if chroma == 0 { 0 } else { cw * chh * 2 };
    let end = 32 + ybytes + 2 * cbytes;
    if end > data.len() as u64 {
        ct.record(ERR_TRUNC, &[]);
        return;
    }
    let (y0, u0) = (32usize, 32 + ybytes as usize);
    let v0 = u0 + cbytes as usize;
    let py = samples(&data[y0..u0]);
    let pu = samples(&data[u0..v0]);
    let pv = samples(&data[v0..v0 + cbytes as usize]);
    let cfg = img_av1::EncIn {
        w: h8[0],
        h: h8[1],
        bd: h8[2],
        chroma,
        qidx: h8[4],
        cicp: h8[5],
        tile_sb: h8[6],
        flags: h8[7],
        cancel: None,
        planes: [&py, &pu, &pv],
        strides: [h8[0], cw as u32, cw as u32],
    };
    match img_av1::encode(&cfg) {
        Ok(o) => ct.record(0, &o.data),
        Err(e) => ct.record(e, &[]),
    }
}
