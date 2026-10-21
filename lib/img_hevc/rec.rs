// rec -- HEVC intra decoder: intra prediction and residual reconstruction
//                                                                [src/img/hevc_rec.s]
//
//   intra(c, x0, y0, log2, mode)      prediction of one TB into its plane (8.4.4.2: reference
//                                     samples with z-scan availability and substitution,
//                                     filtering / strong smoothing, planar, DC, angular with
//                                     the DC / boundary filters)  [hvr_intra]
//   recon_tb(c, x0, y0, log2, ts)     coef (raster, i32) -> scaling (8.6.2 / 8.6.3, scaling
//                                     lists), transform skip / DST 4x4 / DCT, transquant
//                                     bypass, added to the predicted samples with clipping
//                                     [hvr_recon_tb]
//
// Plane coordinates are in samples of the component; availability works in luma units.  The
// encoder (enc.rs) predicts and reconstructs through the same functions.

use crate::dec::Hevc;

/// intraPredAngle by mode (0, 1 unused)
static ANGLE: [i8; 36] = [
    0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, //
    -32, -26, -21, -17, -13, -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32, 0,
];
/// invAngle (modes 11 .. 25)
static INVANG: [i16; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    -4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, //
    -1638, -4096, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
/// QpC for qPi 30 .. 43 (ChromaArrayType 1)
pub(crate) static QPC: [u8; 14] = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37];
static LSCALE: [u8; 6] = [40, 45, 51, 57, 64, 72];
/// transMatrix of the 4x4 DST (rows k)
static DST4: [i8; 16] = [29, 55, 74, 84, 74, 74, 0, -74, 84, -29, -74, 55, 55, -84, 74, -29];
/// raster (y * 4 + x) -> up-right diagonal index
static INV4: [u8; 16] = [0, 2, 5, 9, 1, 4, 8, 12, 3, 7, 11, 14, 6, 10, 13, 15];
/// raster (y * 8 + x) -> up-right diagonal index (8x8)
static INV8: [u8; 64] = [
    0, 2, 5, 9, 14, 20, 27, 35, //
    1, 4, 8, 13, 19, 26, 34, 42, //
    3, 7, 12, 18, 25, 33, 41, 48, //
    6, 11, 17, 24, 32, 40, 47, 53, //
    10, 16, 23, 31, 39, 46, 52, 57, //
    15, 22, 30, 38, 45, 51, 56, 60, //
    21, 29, 37, 44, 50, 55, 59, 62, //
    28, 36, 43, 49, 54, 58, 61, 63,
];
/// 32-point transMatrix (row k, column j)
pub(crate) static DCT32: [i8; 1024] = [
    64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, //
    64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, //
    90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4, //
    -4, -13, -22, -31, -38, -46, -54, -61, -67, -73, -78, -82, -85, -88, -90, -90, //
    90, 87, 80, 70, 57, 43, 25, 9, -9, -25, -43, -57, -70, -80, -87, -90, //
    -90, -87, -80, -70, -57, -43, -25, -9, 9, 25, 43, 57, 70, 80, 87, 90, //
    90, 82, 67, 46, 22, -4, -31, -54, -73, -85, -90, -88, -78, -61, -38, -13, //
    13, 38, 61, 78, 88, 90, 85, 73, 54, 31, 4, -22, -46, -67, -82, -90, //
    89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89, //
    89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89, //
    88, 67, 31, -13, -54, -82, -90, -78, -46, -4, 38, 73, 90, 85, 61, 22, //
    -22, -61, -85, -90, -73, -38, 4, 46, 78, 90, 82, 54, 13, -31, -67, -88, //
    87, 57, 9, -43, -80, -90, -70, -25, 25, 70, 90, 80, 43, -9, -57, -87, //
    -87, -57, -9, 43, 80, 90, 70, 25, -25, -70, -90, -80, -43, 9, 57, 87, //
    85, 46, -13, -67, -90, -73, -22, 38, 82, 88, 54, -4, -61, -90, -78, -31, //
    31, 78, 90, 61, 4, -54, -88, -82, -38, 22, 73, 90, 67, 13, -46, -85, //
    83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, //
    83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, //
    82, 22, -54, -90, -61, 13, 78, 85, 31, -46, -90, -67, 4, 73, 88, 38, //
    -38, -88, -73, -4, 67, 90, 46, -31, -85, -78, -13, 61, 90, 54, -22, -82, //
    80, 9, -70, -87, -25, 57, 90, 43, -43, -90, -57, 25, 87, 70, -9, -80, //
    -80, -9, 70, 87, 25, -57, -90, -43, 43, 90, 57, -25, -87, -70, 9, 80, //
    78, -4, -82, -73, 13, 85, 67, -22, -88, -61, 31, 90, 54, -38, -90, -46, //
    46, 90, 38, -54, -90, -31, 61, 88, 22, -67, -85, -13, 73, 82, 4, -78, //
    75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75, //
    75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75, //
    73, -31, -90, -22, 78, 67, -38, -90, -13, 82, 61, -46, -88, -4, 85, 54, //
    -54, -85, 4, 88, 46, -61, -82, 13, 90, 38, -67, -78, 22, 90, 31, -73, //
    70, -43, -87, 9, 90, 25, -80, -57, 57, 80, -25, -90, -9, 87, 43, -70, //
    -70, 43, 87, -9, -90, -25, 80, 57, -57, -80, 25, 90, 9, -87, -43, 70, //
    67, -54, -78, 38, 85, -22, -90, 4, 90, 13, -88, -31, 82, 46, -73, -61, //
    61, 73, -46, -82, 31, 88, -13, -90, -4, 90, 22, -85, -38, 78, 54, -67, //
    64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, //
    64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, //
    61, -73, -46, 82, 31, -88, -13, 90, -4, -90, 22, 85, -38, -78, 54, 67, //
    -67, -54, 78, 38, -85, -22, 90, 4, -90, 13, 88, -31, -82, 46, 73, -61, //
    57, -80, -25, 90, -9, -87, 43, 70, -70, -43, 87, 9, -90, 25, 80, -57, //
    -57, 80, 25, -90, 9, 87, -43, -70, 70, 43, -87, -9, 90, -25, -80, 57, //
    54, -85, -4, 88, -46, -61, 82, 13, -90, 38, 67, -78, -22, 90, -31, -73, //
    73, 31, -90, 22, 78, -67, -38, 90, -13, -82, 61, 46, -88, 4, 85, -54, //
    50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50, //
    50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50, //
    46, -90, 38, 54, -90, 31, 61, -88, 22, 67, -85, 13, 73, -82, 4, 78, //
    -78, -4, 82, -73, -13, 85, -67, -22, 88, -61, -31, 90, -54, -38, 90, -46, //
    43, -90, 57, 25, -87, 70, 9, -80, 80, -9, -70, 87, -25, -57, 90, -43, //
    -43, 90, -57, -25, 87, -70, -9, 80, -80, 9, 70, -87, 25, 57, -90, 43, //
    38, -88, 73, -4, -67, 90, -46, -31, 85, -78, 13, 61, -90, 54, 22, -82, //
    82, -22, -54, 90, -61, -13, 78, -85, 31, 46, -90, 67, 4, -73, 88, -38, //
    36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, //
    36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, //
    31, -78, 90, -61, 4, 54, -88, 82, -38, -22, 73, -90, 67, -13, -46, 85, //
    -85, 46, 13, -67, 90, -73, 22, 38, -82, 88, -54, -4, 61, -90, 78, -31, //
    25, -70, 90, -80, 43, 9, -57, 87, -87, 57, -9, -43, 80, -90, 70, -25, //
    -25, 70, -90, 80, -43, -9, 57, -87, 87, -57, 9, 43, -80, 90, -70, 25, //
    22, -61, 85, -90, 73, -38, -4, 46, -78, 90, -82, 54, -13, -31, 67, -88, //
    88, -67, 31, 13, -54, 82, -90, 78, -46, 4, 38, -73, 90, -85, 61, -22, //
    18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18, //
    18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18, //
    13, -38, 61, -78, 88, -90, 85, -73, 54, -31, 4, 22, -46, 67, -82, 90, //
    -90, 82, -67, 46, -22, -4, 31, -54, 73, -85, 90, -88, 78, -61, 38, -13, //
    9, -25, 43, -57, 70, -80, 87, -90, 90, -87, 80, -70, 57, -43, 25, -9, //
    -9, 25, -43, 57, -70, 80, -87, 90, -90, 87, -80, 70, -57, 43, -25, 9, //
    4, -13, 22, -31, 38, -46, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90, //
    90, -90, 88, -85, 82, -78, 73, -67, 61, -54, 46, -38, 31, -22, 13, -4,
];

/// |v|
#[inline(always)]
fn iabs(v: i32) -> i32 {
    if v < 0 { v.wrapping_neg() } else { v }
}

impl Hevc<'_> {
    // ================================================================== intra prediction
    /// hvr_intra: prediction of one TB of component c into its plane (see the file header)
    pub(crate) fn intra(&mut self, c: usize, x0: u32, y0: u32, log2: u32, mode: u32) {
        let n = 1usize << log2;
        let (bd, sw, sh) = if c == 0 { (self.s.bdy, 1, 1) } else { (self.s.bdc, self.s.subw as i32, self.s.subh as i32) };
        let max = (1i32 << bd) - 1;
        let c = c & 3;
        let pw = self.pw[c];
        let ph = self.ph[c];
        // ---- reference samples S[0 .. 4n]: S[k] = p[-1][2n-1-k] (k < 2n), S[2n] = p[-1][-1],
        //      S[2n+1+x] = p[x][-1]; -1 = not available
        let n4 = 4 * n;
        let mut cnt = 0u32; // available count
        {
            let pl = &self.pl[c];
            let x0i = x0 as i32;
            let y0i = y0 as i32;
            // avail per sample as the assembly, its map lookup cached per 4x4 block
            let (lw, lh) = (self.s.w, self.s.h);
            let mut lastkey = usize::MAX;
            let mut lastav = false;
            for k in 0..=n4 {
                let (xs, ys) = if k < 2 * n {
                    (x0i - 1, y0i + (2 * n as i32 - 1 - k as i32))
                } else {
                    (x0i + (k as i32 - 2 * n as i32 - 1), y0i - 1)
                };
                let lx = xs.wrapping_mul(sw);
                let ly = ys.wrapping_mul(sh);
                let av = if lx < 0 || ly < 0 || lx as u32 >= lw || ly as u32 >= lh {
                    false
                } else {
                    let key = self.mapidx(lx as u32, ly as u32);
                    if key != lastkey {
                        lastkey = key;
                        lastav = self.mdec[key] == self.u.sliceaddr && self.mtid[key] as u32 == self.u.tile;
                    }
                    lastav
                };
                let mut v = -1i32;
                if av && (xs as u32) < pw && (ys as u32) < ph {
                    v = pl[(ys as u32 * pw + xs as u32) as usize] as i32;
                    cnt += 1;
                }
                self.refs[k] = v;
            }
        }
        // substitution (8.4.4.2.2)
        if cnt == 0 {
            let v = 1i32 << (bd - 1);
            self.refs[..=n4].fill(v);
        } else {
            if self.refs[0] == -1 {
                let mut k = 0;
                loop {
                    k += 1;
                    if self.refs[k] != -1 {
                        break;
                    }
                }
                self.refs[0] = self.refs[k];
            }
            for k in 1..=n4 {
                if self.refs[k] == -1 {
                    self.refs[k] = self.refs[k - 1];
                }
            }
        }
        // ---- filtering (luma or 4:4:4 chroma; not DC, not 4x4)
        if (c == 0 || self.s.cat == 3) && self.s.isdis == 0 && mode != 1 && n != 4 {
            // minDistVerHor > intraHorVerDistThres[nTbS] (planar: always)
            let d = (mode as i32 - 26).unsigned_abs().min((mode as i32 - 10).unsigned_abs());
            let thr = match n {
                8 => 7,
                16 => 1,
                _ => 0,
            };
            if d > thr {
                let mut strong = false;
                // strong intra smoothing (luma 32x32)
                if self.s.sis != 0 && c == 0 && n == 32 {
                    let t = 1i32 << ((bd - 5) & 31); // threshold
                    let corner = self.refs[2 * n];
                    let top = self.refs[n4]; // p[2n-1][-1]
                    let mid = self.refs[3 * n]; // p[n-1][-1]
                    if iabs(corner + top - mid - mid) < t {
                        let left = self.refs[0]; // p[-1][2n-1]
                        let midl = self.refs[n]; // p[-1][n-1]
                        if iabs(corner + left - midl - midl) < t {
                            strong = true;
                            // bilinear: top p[x][-1] = ((63-x) pc + (x+1) p[63][-1] + 32) >> 6, left likewise
                            for x in 0..63i32 {
                                self.ref2[65 + x as usize] = ((63 - x) * corner + (x + 1) * top + 32) >> 6;
                                self.ref2[(63 - x) as usize] = ((63 - x) * corner + (x + 1) * left + 32) >> 6;
                            }
                            self.ref2[0] = left;
                            self.ref2[64] = corner;
                            self.ref2[128] = top;
                        }
                    }
                }
                if !strong {
                    self.ref2[0] = self.refs[0];
                    self.ref2[n4] = self.refs[n4];
                    for k in 1..n4 {
                        self.ref2[k] = (self.refs[k - 1] + self.refs[k + 1] + self.refs[k] * 2 + 2) >> 2;
                    }
                }
                self.refs[..=n4].copy_from_slice(&self.ref2[..=n4]);
            }
        }
        let n2 = 2 * n;
        if mode == 0 {
            // ---- planar
            let topn = self.refs[n2 + n + 1]; // top(n) = p[n][-1]
            let leftn = self.refs[n - 1]; // left(n) = p[-1][n] = S[n-1]
            let sh = log2 + 1;
            let ni = n as i32;
            for y in 0..n {
                let left = self.refs[n2 - 1 - y];
                for x in 0..n {
                    let top = self.refs[n2 + x + 1];
                    let v = (ni - 1 - x as i32) * left
                        + (x as i32 + 1) * topn
                        + top * (ni - 1 - y as i32)
                        + (y as i32 + 1) * leftn
                        + ni;
                    self.res[y * n + x] = v >> sh;
                }
            }
        } else if mode == 1 {
            // ---- DC
            let mut sum = 0u32;
            for x in 0..n {
                sum = sum.wrapping_add(self.refs[n2 + x + 1] as u32).wrapping_add(self.refs[n2 - 1 - x] as u32);
            }
            let dc = (sum.wrapping_add(n as u32) >> (log2 + 1)) as i32;
            self.res[..n * n].fill(dc);
            if c == 0 && n < 32 {
                // edge filters
                let l0 = self.refs[n2 - 1];
                let t0 = self.refs[n2 + 1];
                self.res[0] = (l0 + t0 + 2 + dc * 2) >> 2;
                let d3 = dc * 3;
                for k in 1..n {
                    self.res[k] = (self.refs[n2 + k + 1] + d3 + 2) >> 2;
                    self.res[k * n] = (self.refs[n2 - 1 - k] + d3 + 2) >> 2;
                }
            }
        } else {
            // ---- angular
            let ang = ANGLE[mode as usize % 36] as i32;
            let inv = INVANG[mode as usize % 36] as i32;
            let vert = mode >= 18; // d = 1 (vertical family), -1 (horizontal)
            // ref[x] (x = -n .. 2n) in ref2[32 + x]: S[2n + d x] (x >= 0), S[2n - d ((x inv + 128) >> 8)]
            for x in 0..=n2 {
                self.ref2[32 + x] = if vert { self.refs[n2 + x] } else { self.refs[n2 - x] };
            }
            if ang < 0 {
                let lim = (n as i32 * ang) >> 5; // (n * ang) >> 5
                if lim < -1 {
                    for x in lim..0 {
                        let v = (x * inv + 128) >> 8;
                        let k = if vert { n2 as i32 - v } else { n2 as i32 + v };
                        self.ref2[(32 + x) as usize] = self.refs[k as usize];
                    }
                }
            }
            for i in 0..n {
                let p = (i as i32 + 1) * ang;
                let idx = p >> 5; // iIdx
                let fact = p & 31; // iFact
                for j in 0..n {
                    let r = (32 + j as i32 + idx + 1) as usize;
                    let mut v = self.ref2[r];
                    if fact != 0 {
                        v = ((32 - fact) * v + fact * self.ref2[r + 1] + 16) >> 5;
                    }
                    // vertical: (y = i, x = j); horizontal: (y = j, x = i)
                    let o = if vert { i * n + j } else { j * n + i };
                    self.res[o] = v;
                }
            }
            // boundary filters of modes 26 / 10 (luma, nTbS < 32)
            if c == 0 && n < 32 {
                let corner = self.refs[n2];
                if mode == 26 {
                    let p0 = self.refs[n2 + 1]; // p[0][-1]
                    for y in 0..n {
                        let v = ((self.refs[n2 - 1 - y] - corner) >> 1) + p0;
                        self.res[y * n] = v.max(0).min(max);
                    }
                } else if mode == 10 {
                    let p0 = self.refs[n2 - 1]; // p[-1][0]
                    for x in 0..n {
                        let v = ((self.refs[n2 + x + 1] - corner) >> 1) + p0;
                        self.res[x] = v.max(0).min(max);
                    }
                }
            }
        }
        // store res (n x n) into the plane
        let pl = &mut self.pl[c];
        for y in 0..n {
            let py = y0 + y as u32;
            if py >= ph {
                break;
            }
            let row = (py * pw + x0) as usize;
            for x in 0..n {
                if x0 + x as u32 >= pw {
                    break;
                }
                pl[row + x] = self.res[y * n + x] as u16;
            }
        }
    }

    // ================================================================== reconstruction
    /// hvr_recon_tb: coef of component c -> residual -> added to the plane (see the file
    /// header); ts = transform_skip_flag
    pub(crate) fn recon_tb(&mut self, c: usize, x0: u32, y0: u32, log2: u32, ts: u32) {
        let n = 1usize << log2;
        let nn = n * n;
        let bd = if c == 0 { self.s.bdy } else { self.s.bdc };
        let bds = 20 - bd as i32; // second stage shift
        if self.u.byp != 0 {
            // transquant bypass: residual = coefficients
            self.res[..nn].copy_from_slice(&self.coef[..nn]);
        } else {
            // ---- qP
            let mut qp = self.u.qpy;
            if c == 0 {
                qp = qp.wrapping_add((self.s.bdy as i32 - 8) * 6);
            } else {
                let off = if c == 1 { self.p.cboff.wrapping_add(self.l.cboff) } else { self.p.croff.wrapping_add(self.l.croff) };
                let mut q = qp.wrapping_add(off); // qPi before clipping
                let bdoff = (self.s.bdc as i32 - 8) * 6; // QpBdOffsetC
                q = q.max(-bdoff).min(57);
                if self.s.cat == 1 {
                    if q >= 30 {
                        q = if q > 43 { q - 6 } else { QPC[(q - 30) as usize] as i32 };
                    }
                } else if q > 51 {
                    q = 51;
                }
                qp = q + bdoff;
            }
            let qpu = qp as u32;
            let ls = (LSCALE[(qpu % 6) as usize] as i64).wrapping_shl(qpu / 6); // levelScale << (qP / 6)
            // ---- scaling list (m = 16 when off, or transform skip of TBs > 4x4)
            let mut sl: Option<usize> = None; // offset of the list in the SPS / PPS lists
            let usepps = self.p.slpres != 0;
            if self.s.slen != 0 && (ts == 0 || n <= 4) {
                self.rw.rlog2 = log2;
                let size = log2 - 2; // sizeId
                let m = size * 6 + c as u32; // matrixId = cIdx (intra)
                sl = Some((m << 6) as usize);
                self.rw.rdc = -1;
                if size >= 2 {
                    let i = (m - 12) as usize;
                    self.rw.rdc = if usepps { self.p.sldc[i] } else { self.s.sldc[i] } as i32;
                }
            }
            // ---- scaling: d = Clip3(-32768, 32767, (c * m * ls + (1 << (bdShift - 1))) >> bdShift)
            let bdshift = bd + log2 - 5;
            let rnd = 1i64 << (bdshift - 1);
            let mut xmax = 0usize; // columns with a coefficient (last one + 1)
            let mut kmax = [0u8; 32]; // per column: rows with a coefficient (last one + 1)
            for i in 0..nn {
                let cv = self.coef[i] as i64;
                if cv == 0 {
                    continue;
                }
                // rows / columns that hold coefficients (the transform skips the rest)
                let y1 = ((i >> log2) + 1) as u8;
                let x = i & (n - 1) & 31;
                if y1 > kmax[x] {
                    kmax[x] = y1;
                }
                if x + 1 > xmax {
                    xmax = x + 1;
                }
                let m = match sl {
                    Some(o) => self.slfactor(i as u32, o, usepps) as i64,
                    None => 16,
                };
                let d = (cv.wrapping_mul(m).wrapping_mul(ls).wrapping_add(rnd)) >> bdshift;
                self.coef[i] = d.max(-32768).min(32767) as i32;
            }
            if ts != 0 {
                // transform skip: r = (d << tsShift) (rotated for 4x4 when enabled), then >> bdShift2
                let tsh = log2 + 5;
                let r2 = 1i32 << ((bds - 1) & 31);
                let rot = self.s.tsrot != 0 && n == 4;
                for i in 0..nn {
                    let k = if rot { 15 - i } else { i };
                    self.res[i] = (self.coef[k].wrapping_shl(tsh)).wrapping_add(r2) >> bds;
                }
            } else {
                // matrix rows: DST for 4x4 luma, else the DCT rows k * 32 / n
                let dst = n == 4 && c == 0;
                let mat: &[i8] = if dst { &DST4 } else { &DCT32 };
                let step = if dst { 4 } else { 32 * (32 / n) }; // matrix bytes per k
                // stage 1 (columns with coefficients): e[y][x] = Clip3(-32768, 32767,
                // (sum_k M[k][y] d[k][x] + 64) >> 7), k below the column's last coefficient
                // (the integer sums wrap like the assembly's; accumulating them row by row of
                // M gives the same values)
                for x in 0..xmax {
                    let km = kmax[x & 31] as usize;
                    let mut acc = [0i32; 32];
                    for k in 0..km {
                        let d = self.coef[k * n + x];
                        let row = &mat[k * step..k * step + n];
                        for (a, &m) in acc[..n].iter_mut().zip(row) {
                            *a = a.wrapping_add(d.wrapping_mul(m as i32));
                        }
                    }
                    for y in 0..n {
                        self.tmp[y * n + x] = (acc[y].wrapping_add(64) >> 7).max(-32768).min(32767);
                    }
                }
                // stage 2 (rows): r[y][x] = (sum_k M[k][x] e[y][k] + (1 << (bdShift2 - 1))) >>
                // bdShift2, k below the last column with coefficients
                let rnd2 = 1i32 << ((bds - 1) & 31);
                for y in 0..n {
                    let mut acc = [0i32; 32];
                    for k in 0..xmax {
                        let e = self.tmp[y * n + k];
                        let row = &mat[k * step..k * step + n];
                        for (a, &m) in acc[..n].iter_mut().zip(row) {
                            *a = a.wrapping_add(e.wrapping_mul(m as i32));
                        }
                    }
                    for (r, &a) in self.res[y * n..y * n + n].iter_mut().zip(&acc[..n]) {
                        *r = a.wrapping_add(rnd2) >> bds;
                    }
                }
            }
        }
        // ---- add the residual to the prediction, clip
        let max = (1i32 << bd) - 1;
        let c = c & 3;
        let pw = self.pw[c];
        let ph = self.ph[c];
        let pl = &mut self.pl[c];
        for y in 0..n {
            let py = y0 + y as u32;
            if py >= ph {
                break;
            }
            let row = (py * pw + x0) as usize;
            for x in 0..n {
                if x0 + x as u32 >= pw {
                    break;
                }
                let v = (pl[row + x] as i32).wrapping_add(self.res[y * n + x]);
                pl[row + x] = v.max(0).min(max) as u16;
            }
        }
    }

    /// slfactor: ScalingFactor (8.6.4.2) of raster index i of the TB size rw.rlog2 (DC of
    /// sizes 16 / 32: rw.rdc); o = offset of the list
    #[inline]
    fn slfactor(&self, i: u32, o: usize, pps: bool) -> u32 {
        let l2 = self.rw.rlog2;
        let x = i & ((1 << l2) - 1);
        let y = i >> l2;
        let list = if pps { &self.p.sl } else { &self.s.sl };
        if l2 == 2 {
            return list[o + INV4[((y << 2) + x) as usize & 15] as usize] as u32;
        }
        if i == 0 && self.rw.rdc >= 0 {
            return self.rw.rdc as u32;
        }
        let r = l2 - 3; // log2 ratio
        list[o + INV8[(((y >> r) << 3) + (x >> r)) as usize & 63] as usize] as u32
    }
}
