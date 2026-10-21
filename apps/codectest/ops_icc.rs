// ops_icc -- codectest ops 320..339: colour management               [apps/codectest/src/icc_ops.s]
//
//   op 320 xform     data = {u32 slen, dlen, intent, flags, npx} + src + dst + npx BGRA
//                    arg = row length (0 = all pixels in one call) -> result ICX_* kind
//                    (or ERR_*), data = the converted pixels
//   op 321 inplace   same, converted in place (src == dst)
//   op 322 builtin   arg = ICC_BI_* -> result length (-1 unknown id), data = profile
//   op 323 describe  data = profile, arg = cap -> result bytes, data = text (with its NUL)
//   op 324 bench     like 320, arg = repetitions -> result microseconds, data = u32 kind
//   op 325 monitor   arg = window (-1 primary monitor) -> result 1 found / 0, data =
//                    {u32 len, u64 key} + the first 128 bytes (header only)
//   op 326 build     like 320 without pixels -> result kind / ERR_* (fuzzing)
//   (empty slen / dlen = the built-in sRGB; a malformed record -> -100)
//
// The LUT grids are built on the worker pool (img_core::icc_xform::set_parallel).

use alloc::vec;
use alloc::vec::Vec;

use img_core::icc_xform::{icc_builtin, icc_describe, set_parallel, IccXform};

use crate::{Ct, u32_at};

const IH_SIZE: usize = 20; // record header bytes
const R_BAD: i32 = -100;

/// plat::parallel_for for the LUT grids of img_core (which does not depend on plat)
fn par(n: usize, f: &(dyn Fn(usize) + Sync)) {
    let _ = plat::parallel_for(n, &|i, _| f(i));
}

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    set_parallel(par);
    match op {
        320 | 321 | 324 | 326 => xform(ct, op, arg, data),
        322 => match icc_builtin(arg) {
            Some(b) => ct.record(b.len() as i32, b),
            None => ct.record(-1, &[]),
        },
        323 => {
            // output capacity arg (1..4096)
            if arg.wrapping_sub(1) > 4095 {
                ct.record(R_BAD, &[]);
            } else {
                let mut buf = vec![0u8; arg as usize];
                let n = icc_describe(data, &mut buf);
                ct.record(n as i32, &buf[..n + 1]);
            }
        }
        325 => {
            let w = arg as i32;
            let key = plat::monitor_key(w);
            let icc = plat::monitor_icc(w);
            let len = icc.as_ref().map_or(0, |b| b.len());
            let mut d = Vec::with_capacity(12 + 128);
            d.extend_from_slice(&(len as u32).to_le_bytes());
            d.extend_from_slice(&key.to_le_bytes());
            if let Some(b) = &icc {
                d.extend_from_slice(&b[..len.min(128)]);
            }
            ct.record(icc.is_some() as i32, &d);
        }
        _ => return false,
    }
    true
}

// ops 320 / 321 / 324 / 326  [icc_op .io_x]
fn xform(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) {
    if data.len() < IH_SIZE {
        return ct.record(R_BAD, &[]);
    }
    let slen = u32_at(data, 0) as usize;
    let dlen = u32_at(data, 4) as usize;
    let npx = u32_at(data, 16) as usize;
    let pend = IH_SIZE + slen + dlen;
    if pend > data.len() || pend + npx * 4 > data.len() {
        return ct.record(R_BAD, &[]);
    }
    let src = (slen != 0).then(|| &data[IH_SIZE..IH_SIZE + slen]);
    let dst = (dlen != 0).then(|| &data[IH_SIZE + slen..pend]);
    let px = &data[pend..pend + npx * 4];
    let xf = match IccXform::build(src, dst, u32_at(data, 8), u32_at(data, 12)) {
        Ok(xf) => xf,
        Err(e) => return ct.record(e, &[]),
    };
    match op {
        326 => ct.record(xf.kind() as i32, &[]),
        324 => {
            let mut out = vec![0u8; npx * 4];
            let t0 = plat::time_us();
            for _ in 0..arg {
                xf.row(px, &mut out);
            }
            let us = plat::time_us() - t0;
            ct.record(us as i32, &xf.kind().to_le_bytes());
        }
        _ => {
            // rows of arg pixels (0 = one call); in place: a copy of the input
            let mut out = vec![0u8; npx * 4];
            if op == 321 {
                out.copy_from_slice(px);
            }
            let mut i = 0;
            while i < npx {
                let mut n = npx - i;
                if arg != 0 && n > arg as usize {
                    n = arg as usize;
                }
                let o = &mut out[i * 4..(i + n) * 4];
                if op == 321 {
                    xf.row_in_place(o);
                } else {
                    xf.row(&px[i * 4..(i + n) * 4], o);
                }
                i += n;
            }
            ct.record(xf.kind() as i32, &out);
        }
    }
}
