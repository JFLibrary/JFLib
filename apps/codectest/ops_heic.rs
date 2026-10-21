// ops_heic -- codectest ops 160..179: HEIC (img_heif, img_hevc)    [apps/codectest/src/heic_ops.s]
//
//   160 OP_HEVCDEC  data = HEVC Annex B byte stream; arg = HVF_* (bits 0..7) | CTU limit << 8:
//                   records {u32 w, h, bitDepthY, bitDepthC, chroma_format_idc; Y, Cb, Cr of
//                   the conformance window as u16 samples} (Hevc::annexb / finish)
//   161 OP_HEICTIME data = HEIC file; arg = repetitions (low 16 bits, 0 = 1) | IMGD flags << 16:
//                   img_heif::decode that many times, records u64 microseconds
//   162 OP_HEVCENC  data = {u32 w, h, chroma, qp, flags (HEF_*), vui} + u8 planes (Y, Cb, Cr
//                   at their sizes): img_hevc::encode; records 4 x {u32 length, NAL unit}
//                   (VPS, SPS, PPS, slice) + the 12 PTL bytes

use alloc::vec::Vec;

use gfx::Surface;
use img_core::{ImgDec, ImgMeta, ERR_TRUNC};
use img_hevc::{EncCfg, Hevc};

use crate::{u32_at, Ct};

const OP_HEVCDEC: u32 = 160;
const OP_HEICTIME: u32 = 161;
const OP_HEVCENC: u32 = 162;

/// run op (one record via ct.record); false = unknown op  [heic_op]
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        OP_HEVCDEC => hevcdec(ct, arg, data),
        OP_HEICTIME => {
            let mut reps = arg & 0xFFFF;
            if reps == 0 {
                reps = 1;
            }
            let dec = ImgDec { flags: arg >> 16, ..Default::default() };
            let t0 = plat::time_us();
            let mut res;
            loop {
                let mut sf = Surface::default();
                let mut meta = ImgMeta::default();
                res = img_heif::decode(data, &mut sf, &dec, &mut meta);
                drop(sf);
                if res != 0 {
                    break;
                }
                reps -= 1;
                if reps == 0 {
                    break;
                }
            }
            let t = plat::time_us() - t0;
            ct.record(res, &t.to_le_bytes());
        }
        OP_HEVCENC => hevcenc(ct, data),
        _ => return false,
    }
    true
}

/// 160: decode an Annex B stream, record the planes of the conformance window
fn hevcdec(ct: &mut Ct, arg: u32, data: &[u8]) {
    let mut hv = Hevc::new(arg & 255, arg >> 8);
    let r = hv.annexb(data);
    if r != 0 {
        ct.record(r, &[]);
        return;
    }
    let pic = match hv.finish() {
        Ok(p) => p,
        Err(e) => {
            ct.record(e, &[]);
            return;
        }
    };
    let mut out = Vec::new();
    for v in [pic.w, pic.h, pic.bd, pic.bdc, pic.chroma] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for c in 0..3 {
        if c != 0 && pic.chroma == 0 {
            break;
        }
        // window in component samples
        let (mut x0, mut y0, mut w, mut h) = (pic.x0, pic.y0, pic.w, pic.h);
        if c != 0 {
            x0 /= pic.subw;
            w /= pic.subw;
            y0 /= pic.subh;
            h /= pic.subh;
        }
        let pw = pic.pw[c] as usize;
        for r in 0..h as usize {
            let o = (y0 as usize + r) * pw + x0 as usize;
            for &s in &pic.planes[c][o..o + w as usize] {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
    }
    ct.record(0, &out);
}

/// 162: encode u8 planes (as u16 samples), record the NAL units and the PTL bytes
fn hevcenc(ct: &mut Ct, data: &[u8]) {
    if data.len() < 24 {
        ct.record(ERR_TRUNC, &[]);
        return;
    }
    let (w, h, chroma) = (u32_at(data, 0), u32_at(data, 4), u32_at(data, 8));
    let (qp, flags, vui) = (u32_at(data, 12), u32_at(data, 16), u32_at(data, 20));
    // u8 planes -> u16 planes
    let mut planes: [Vec<u16>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut st = [0u32; 3];
    let mut src = &data[24..];
    for c in 0..3 {
        let (mut pw, mut ph) = (w, h);
        if c != 0 {
            if chroma == 0 {
                break;
            }
            if chroma == 1 {
                pw >>= 1;
                ph >>= 1;
            }
        }
        st[c] = pw;
        let n = pw.wrapping_mul(ph) as usize; // samples of the plane
        if n > src.len() {
            ct.record(ERR_TRUNC, &[]);
            return;
        }
        planes[c] = src[..n].iter().map(|&b| b as u16).collect();
        src = &src[n..];
    }
    let cfg = EncCfg {
        pl: [&planes[0], &planes[1], &planes[2]],
        st,
        w,
        h,
        chroma,
        qp,
        flags,
        vui,
        cancel: None,
    };
    match img_hevc::encode(&cfg) {
        Ok(o) => {
            let mut out = Vec::new();
            for n in &o.nal {
                out.extend_from_slice(&(n.len() as u32).to_le_bytes());
                out.extend_from_slice(n);
            }
            out.extend_from_slice(&o.ptl);
            ct.record(0, &out);
        }
        Err(e) => ct.record(e, &[]),
    }
}
