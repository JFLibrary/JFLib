// espline -- the test-only spline writer: a frame's spline bundle from given quantized splines
// (libjxl enc_splines.cc EncodeSplines / QuantizedSplineEncoder::Tokenize; libjxl's API cannot
// write splines, so files with splines for the decoder's checks come from here)
//                                                                         [jxl-tools: no asm]
//
//   write(q, jw)              the bundle (histograms + tokens of the 6 spline contexts)
//   from_bytes(b) -> Quantized   the codectest op's description: u32 count, i32 quantization
//                             adjustment; per spline i32 x, y (start), u32 n, n x (i32 dx, dy)
//                             delta-deltas, 3 x 32 i32 colour DCT (X, Y, B), 32 i32 sigma DCT

use alloc::vec;
use alloc::vec::Vec;

use crate::eans::{self, pack_signed, HParams, Token};
use crate::ewr::Jw;
use crate::spline::{QSpline, Quantized, CTX_DCT, CTX_NPOINTS, CTX_NUM, CTX_POINTS, CTX_QADJ, CTX_START, NUM_CTX};

/// EncodeSplines
pub fn write(q: &Quantized, jw: &mut Jw) {
    let mut t: Vec<Token> = Vec::new();
    t.push(Token::new(CTX_NUM, q.splines.len() as u32 - 1));
    let (mut lx, mut ly) = (0i64, 0i64);
    for (i, &(x, y)) in q.start.iter().enumerate() {
        let (x, y) = (x as i64, y as i64);
        if i == 0 {
            t.push(Token::new(CTX_START, x as u32));
            t.push(Token::new(CTX_START, y as u32));
        } else {
            t.push(Token::new(CTX_START, pack_signed((x - lx) as i32)));
            t.push(Token::new(CTX_START, pack_signed((y - ly) as i32)));
        }
        lx = x;
        ly = y;
    }
    t.push(Token::new(CTX_QADJ, pack_signed(q.qadj)));
    for s in &q.splines {
        t.push(Token::new(CTX_NPOINTS, s.points.len() as u32));
        for &(dx, dy) in &s.points {
            t.push(Token::new(CTX_POINTS, pack_signed(dx as i32)));
            t.push(Token::new(CTX_POINTS, pack_signed(dy as i32)));
        }
        for c in 0..3 {
            for &v in &s.color[c] {
                t.push(Token::new(CTX_DCT, pack_signed(v)));
            }
        }
        for &v in &s.sigma {
            t.push(Token::new(CTX_DCT, pack_signed(v)));
        }
    }
    let mut st = vec![t];
    let code = eans::build(&HParams::default(), NUM_CTX as usize, &mut st, Some(jw));
    eans::write_tokens(&st[0], &code, jw);
}

/// the op's description (see the module header); None when it is malformed
pub fn from_bytes(b: &[u8]) -> Option<(Quantized, usize)> {
    let mut p = 0usize;
    let mut rd = || -> Option<i32> {
        let v = b.get(p..p + 4)?;
        p += 4;
        Some(i32::from_le_bytes([v[0], v[1], v[2], v[3]]))
    };
    let n = rd()? as u32;
    if n == 0 || n > 4096 {
        return None;
    }
    let qadj = rd()?;
    let mut q = Quantized { qadj, ..Default::default() };
    for _ in 0..n {
        let x = rd()?;
        let y = rd()?;
        q.start.push((x as f32, y as f32));
        let np = rd()? as u32;
        if np > 1 << 16 {
            return None;
        }
        let mut s = QSpline::default();
        for _ in 0..np {
            let dx = rd()? as i64;
            let dy = rd()? as i64;
            s.points.push((dx, dy));
        }
        for c in 0..3 {
            for i in 0..32 {
                s.color[c][i] = rd()?;
            }
        }
        for i in 0..32 {
            s.sigma[i] = rd()?;
        }
        q.splines.push(s);
    }
    Some((q, p))
}
