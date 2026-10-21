// icc_xform -- ICC colour transforms                  [src/img/icc_xform.s, inc/icc.inc]
//
//   IccXform::build(src, dst, intent, flags) -> IccXform | ERR_*
//        src / dst = profile bytes (None = the built-in sRGB); intent ICC_* , flags
//        ICF_BPC / ICF_NOIDENT / ICF_LUT.  ERR_TRUNC / ERR_HDR (broken profile, bad tag),
//        ERR_UNSUPPORTED (CMYK / Lab / other device spaces, links, abstract, named colour,
//        unknown tag types, gray LUT profiles, singular colorants, intent > 3), ERR_LIMIT
//        (curv > 2^20 entries)  [icc_xform_build]
//   xf.row(src, dst)        straight BGRA pixels (8 bit), alpha unchanged; any thread, no
//                           allocation  [icc_xform_row]
//   xf.row_in_place(px)     the same with src == dst
//   xf.kind() -> ICX_*      [icc_xform_kind]
//   icc_builtin(id) -> profile bytes (static) | None   ICC_BI_*  [icc_builtin]
//   icc_describe(icc, buf) -> bytes   UTF-8 'desc' text, NUL-terminated  [icc_describe]
//   set_parallel(f)         the loop that builds LUT grids (plat::parallel_for in the apps;
//                           without one the grid planes are built one after the other)
//
// Profiles: RGB and gray, v2 and v4, classes mntr / scnr / prtr / spac.  Matrix/TRC
// (rXYZ gXYZ bXYZ + rTRC gTRC bTRC, gray kTRC; curv identity / gamma / table, para 0..4)
// and LUT tags (A2B0..2 / B2A0..2 as lut8 / lut16 / mAB / mBA; PCS XYZ or Lab).  Tag choice
// like LittleCMS: the intent's LUT tag (absolute: *1), else *0, else the matrix/TRC.
// Media white for the absolute intent: wtpt (D50 for v2 display profiles, like lcms).
//
// Paths (ICX_*): identical profiles -> copy; matrix/TRC on both sides -> ICX_MATRIX: per
// channel f32 linearisation tables (256), one 3x3 f32 matrix (dst^-1 * adapt * src), linear
// values clamped to 0..1, index round(sqrt(v) * 4096) into u8 inverse tables (4097 entries,
// built by inverting 8193 forward samples of the destination TRC) -- a simd::Kernel, 4 (SSE2)
// or 8 (AVX2) pixels per step, every lane computes what the assembly's SSE lane computes;
// a result that maps every channel value to itself becomes ICX_IDENTITY.  Otherwise
// ICX_LUT: the whole float64 pipeline (src -> PCS XYZ -> intent / BPC -> dst) sampled on a
// 33^3 grid whose node positions are (k/32)^2 per axis (dense near black), f32 per channel,
// tetrahedral interpolation in f32 (grid planes on the worker pool, see set_parallel).  For
// matrix/TRC (gray) destinations the grid holds the destination's LINEAR values, unclamped,
// and every pixel ends like the matrix path (clamp, sqrt index, inverse TRC table); for B2A
// destinations device values.  The tetrahedron is chosen without branches (a table over the
// three comparisons of the assembly's decision tree, the weights as max / median / min of
// the fractions): the same vertices and weights, no mispredictions on random pixels.
//
// The handle of the assembly becomes a value (dropping it frees it); the parsing and the
// float64 maths are in icc_eval, the built-in profiles and the description in icc_bi.
// Deviation: out of memory aborts (Vec) instead of ERR_NOMEM.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use rt::fx::{cvttd, maxsd, maxss, minsd, minss, sqrt, sqrtd};
use rt::sync::SpinLock;
use simd::{Kernel, Simd};

pub use crate::icc_bi::{icc_builtin, icc_describe};
use crate::icc::icc_check;
use crate::icc_eval::*;
use crate::ERR_UNSUPPORTED;

// ------------------------------------------------------------------ icc.inc
pub const ICC_PERCEPTUAL: u32 = 0; // rendering intents (ICC numbering)
pub const ICC_RELATIVE: u32 = 1; // relative colorimetric
pub const ICC_SATURATION: u32 = 2;
pub const ICC_ABSOLUTE: u32 = 3; // absolute colorimetric

pub const ICF_BPC: u32 = 1; // black point compensation (not with ICC_ABSOLUTE)
pub const ICF_NOIDENT: u32 = 2; // never take the identity shortcut (tests)
pub const ICF_LUT: u32 = 4; // always use the 3D LUT path (tests)

pub const ICX_IDENTITY: u32 = 0; // rows are copied
pub const ICX_MATRIX: u32 = 1; // linearisation tables + 3x3 matrix + inverse tables
pub const ICX_LUT: u32 = 2; // 33^3 3D LUT, tetrahedral interpolation

pub const ICC_BI_SRGB: u32 = 0; // sRGB IEC61966-2.1 compatible
pub const ICC_BI_A98: u32 = 1; // A98 RGB compatible (CSS Color 4 a98-rgb: D65, gamma 563/256)
pub const ICC_BI_P3: u32 = 2; // Display P3
pub const ICC_BI_ROMM: u32 = 3; // ROMM RGB (ISO 22028-2, D50, gamma 1.8)
pub const ICC_BI_GRAY22: u32 = 4; // gray, gamma 2.2, D50
pub const ICC_BI_COUNT: u32 = 5;

pub const ICC_MON_PRIMARY: i32 = -1; // plat::monitor_icc: the primary monitor

// ------------------------------------------------------------------ build context (XB)
const MODE_MATRIX: u32 = 1;
const MODE_LUT: u32 = 2;

const OUT_N: usize = INV_N + 1; // XF_OUT entries per channel
const GN: usize = 33; // grid nodes per axis
const GS_B: usize = 4; // node strides in f32 units: f32 B, G, R, 0 per node
const GS_G: usize = GN * 4;
const GS_R: usize = GN * GN * 4;
const GS_ALL: usize = GS_R + GS_G + GS_B;
const GRID_N: usize = GN * GN * GN * 4;
static AX_STRIDE: [usize; 3] = [GS_B, GS_G, GS_R];

/// everything the pipeline needs while a transform is built  [XB]
struct Xb<'a> {
    src: Pm<'a>,
    dst: Pm<'a>,
    ls: Lut<'a>,      // source A2B
    ld: Lut<'a>,      // destination B2A
    ms: [f64; 9],     // device linear -> XYZ
    md: [f64; 9],     // XYZ -> destination linear
    adja: [f64; 3],   // xyz' = a * xyz + b (absolute intent / BPC)
    adjb: [f64; 3],
    inv: Vec<f64>,    // inverse tables f64[3][INV_N + 1] (matrix destination)
    smode: u32,       // MODE_*
    dmode: u32,
    flags: u32,
}

/// the tables of a transform  [XF]
struct Tables {
    mb: [f32; 9],             // ICX_MATRIX: m00 m01 .. m22
    inp: [[i32; 256]; 3],     // f32 bits: linear values of R, G, B
    out: [[u8; OUT_N]; 3],    // device values of R, G, B at the sqrt-companded index
    ax_off: [[u32; 256]; 3],  // axes B, G, R: k * stride
    ax_f: [[f32; 256]; 3],    //   and the fraction inside the cell
    lmode: bool,              // ICX_LUT: the grid holds destination linear values (then
                              // clamp + out tables), else device values
    grid: Vec<i32>,           // ICX_LUT: f32 bits B, G, R, 0 per node
}

/// a colour transform between two profiles
pub struct IccXform {
    kind: u32,
    t: Box<Tables>,
}

pub use crate::ParallelFor;

/// the loop the LUT grids are built with = img_core::set_parallel (kept for its callers)
pub fn set_parallel(f: ParallelFor) {
    crate::set_parallel(f)
}

/// tags = A2B / B2A of a profile: the intent's tag (absolute: *1), else *0 (like lcms)
/// [lut_tag]
fn lut_tag<'a>(tags: &[Option<&'a [u8]>; 3], intent: u32) -> Option<&'a [u8]> {
    let i = if intent == ICC_ABSOLUTE { ICC_RELATIVE } else { intent };
    tags[i as usize].or(tags[0])
}

impl Xb<'_> {
    /// source device values (0..1) -> PCS XYZ, intent adjustment applied  [src_xyz]
    fn src_xyz(&self, v: &mut [f64; 3]) {
        if self.smode == MODE_LUT {
            lt_eval(&self.ls, v);
            pcs_decode(&self.ls, v);
        } else {
            curves3(&self.src.cv, v);
            mat_apply(&self.ms, v, false);
        }
        for i in 0..3 {
            v[i] = v[i] * self.adja[i] + self.adjb[i];
        }
    }

    /// PCS XYZ -> destination values: MODE_LUT device values 0..1 (B2A); MODE_MATRIX the
    /// destination's linear values (not clamped; the kernel ends the way)  [dst_dev]
    fn dst_dev(&self, v: &mut [f64; 3]) {
        if self.dmode == MODE_LUT {
            pcs_encode(&self.ld, v);
            clamp3(v);
            lt_eval(&self.ld, v);
            clamp3(v);
        } else {
            mat_apply(&self.md, v, false);
        }
    }

    /// grid nodes (i, 0..32, 0..32) into plane (GN * GN nodes)  [grid_job]
    fn grid_plane(&self, i: usize, plane: &mut [i32]) {
        let ri = (i * i) as i32 as f64 * D_I1024;
        for j in 0..GN {
            for k in 0..GN {
                let mut v = [ri, (j * j) as i32 as f64 * D_I1024, (k * k) as i32 as f64 * D_I1024];
                self.src_xyz(&mut v);
                self.dst_dev(&mut v);
                let o = (j * GN + k) * 4;
                plane[o] = (v[2] as f32).to_bits() as i32; // B, G, R, 0
                plane[o + 1] = (v[1] as f32).to_bits() as i32;
                plane[o + 2] = (v[0] as f32).to_bits() as i32;
                plane[o + 3] = 0;
            }
        }
    }
}

impl Tables {
    fn new() -> Box<Tables> {
        Box::new(Tables {
            mb: [0.0; 9],
            inp: [[0; 256]; 3],
            out: [[0; OUT_N]; 3],
            ax_off: [[0; 256]; 3],
            ax_f: [[0.0; 256]; 3],
            lmode: false,
            grid: Vec::new(),
        })
    }
}

impl IccXform {
    /// a transform from profile src to profile dst (None = the built-in sRGB)
    /// [icc_xform_build]
    pub fn build(src: Option<&[u8]>, dst: Option<&[u8]>, intent: u32, flags: u32) -> Result<IccXform, i32> {
        if intent > ICC_ABSOLUTE {
            return Err(ERR_UNSUPPORTED);
        }
        let srgb = icc_builtin(ICC_BI_SRGB).unwrap_or(&[]);
        let src = src.unwrap_or(srgb);
        let dst = dst.unwrap_or(srgb);
        let mut xf = IccXform { kind: ICX_IDENTITY, t: Tables::new() };
        // identical bytes: copy rows (still only valid profiles)
        if flags & (ICF_NOIDENT | ICF_LUT) == 0 && src == dst {
            let r = icc_check(src);
            if r < 0 {
                return Err(r as i32);
            }
            return Ok(xf);
        }
        let mut xb = Xb {
            src: pm_parse(src)?,
            dst: pm_parse(dst)?,
            ls: LUT0,
            ld: LUT0,
            ms: [0.0; 9],
            md: [0.0; 9],
            adja: [D_ONE; 3],
            adjb: [D_ZERO; 3],
            inv: Vec::new(),
            smode: 0,
            dmode: 0,
            flags,
        };
        // ---- source: the intent's A2B tag, else matrix/TRC (or gray kTRC)
        let stag = if xb.src.cs != CS_GRAY { lut_tag(&xb.src.a2b, intent) } else { None };
        if let Some(el) = stag {
            xb.ls = lt_parse(el, xb.src.pcs == PCS_LAB, 0)?;
            xb.smode = MODE_LUT;
        } else {
            if xb.src.flags & (PMF_MATRIX | PMF_GRAY) == 0 || xb.src.pcs != PCS_XYZ {
                return Err(ERR_UNSUPPORTED);
            }
            xb.smode = MODE_MATRIX;
            xb.ms = xb.src.mat;
        }
        // ---- destination: the intent's B2A tag, else the inverted matrix/TRC
        let dtag = if xb.dst.cs != CS_GRAY { lut_tag(&xb.dst.b2a, intent) } else { None };
        if let Some(el) = dtag {
            xb.ld = lt_parse(el, xb.dst.pcs == PCS_LAB, 1)?;
            xb.dmode = MODE_LUT;
        } else {
            if xb.dst.pcs != PCS_XYZ {
                return Err(ERR_UNSUPPORTED);
            }
            if xb.dst.flags & PMF_GRAY != 0 {
                // gray output = Y of the PCS (rows 0 1 0)
                xb.md[1] = D_ONE;
                xb.md[4] = D_ONE;
                xb.md[7] = D_ONE;
            } else {
                if xb.dst.flags & PMF_MATRIX == 0 {
                    return Err(ERR_UNSUPPORTED);
                }
                xb.md = mat3_inv(&xb.dst.mat).ok_or(ERR_UNSUPPORTED)?;
            }
            xb.dmode = MODE_MATRIX;
            let mut fw = vec![0f64; NF];
            let mut inv = vec![0f64; 3 * OUT_N];
            for (c, tab) in inv.chunks_mut(OUT_N).enumerate() {
                inv_build(&xb.dst.cv[c], tab, &mut fw);
            }
            xb.inv = inv;
        }
        // ---- intent adjustment: xyz' = a xyz + b
        if intent == ICC_ABSOLUTE {
            for i in 0..3 {
                let w = xb.dst.wtpt[i];
                if w > D_EPSW {
                    xb.adja[i] = xb.src.wtpt[i] / w;
                }
            }
        } else if flags & ICF_BPC != 0 {
            let mut bs = [0f64; 3];
            let mut bd = [0f64; 3];
            xb.src_xyz(&mut bs); // source black (ADJ = identity yet)
            if xb.dmode == MODE_LUT {
                if let Some(el) = lut_tag(&xb.dst.a2b, intent) {
                    if let Ok(lt2) = lt_parse(el, xb.dst.pcs == PCS_LAB, 0) {
                        lt_eval(&lt2, &mut bd);
                        pcs_decode(&lt2, &mut bd);
                    }
                }
            }
            // lcms: a = (bd - D50) / (bs - D50), b = -D50 (bd - bs) / (bs - D50)
            for i in 0..3 {
                let t = bs[i] - D_D50[i];
                if t.abs() > D_EPST {
                    xb.adja[i] = (bd[i] - D_D50[i]) / t;
                    xb.adjb[i] = D_ZERO - (bd[i] - bs[i]) * D_D50[i] / t;
                }
            }
        }
        // ---- path
        if flags & ICF_LUT == 0
            && xb.smode == MODE_MATRIX
            && xb.dmode == MODE_MATRIX
            && xb.adjb.iter().all(|&b| b == D_ZERO)
        {
            xf.matrix(&xb);
        } else {
            xf.lut(&xb);
        }
        Ok(xf)
    }

    /// ICX_IDENTITY / ICX_MATRIX / ICX_LUT  [icc_xform_kind]
    pub fn kind(&self) -> u32 {
        self.kind
    }

    /// dst.len() / 4 straight BGRA pixels from src (at least as long) to dst, alpha
    /// unchanged  [icc_xform_row]
    pub fn row(&self, src: &[u8], dst: &mut [u8]) {
        let n = dst.len() / 4;
        let src = &src[..n * 4];
        match self.kind {
            ICX_MATRIX => simd::dispatch(MatRows { t: &self.t, src: Some(src), dst: &mut dst[..n * 4] }),
            ICX_LUT => lut_rows(&self.t, Some(src), &mut dst[..n * 4]),
            _ => dst[..n * 4].copy_from_slice(src),
        }
    }

    /// row with src == dst: px.len() / 4 pixels converted in place
    pub fn row_in_place(&self, px: &mut [u8]) {
        let n = px.len() / 4;
        match self.kind {
            ICX_MATRIX => simd::dispatch(MatRows { t: &self.t, src: None, dst: &mut px[..n * 4] }),
            ICX_LUT => lut_rows(&self.t, None, &mut px[..n * 4]),
            _ => {}
        }
    }

    /// the ICX_MATRIX tables (or ICX_IDENTITY)  [xf_matrix]
    fn matrix(&mut self, xb: &Xb) {
        let t = &mut self.t;
        let mut s = [0f64; 9];
        for i in 0..9 {
            s[i] = xb.ms[i] * xb.adja[i / 3];
        }
        let m = mat3_mul(&xb.md, &s);
        for i in 0..9 {
            t.mb[i] = m[i] as f32;
        }
        // linearisation tables
        for c in 0..3 {
            for u in 0..256 {
                t.inp[c][u] = (cv_eval(&xb.src.cv[c], u as i32 as f64 * D_I255) as f32).to_bits() as i32;
            }
        }
        out_tabs(t, xb);
        self.kind = ICX_MATRIX;
        // identity? matrix ~ I and every channel value maps to itself
        if xb.flags & ICF_NOIDENT != 0 {
            return;
        }
        for i in 0..9 {
            if (m[i] - D_IDENT[i]).abs() > D_EPSID {
                return;
            }
        }
        for c in 0..3 {
            for u in 0..256 {
                let mut v = f32::from_bits(t.inp[c][u] as u32);
                v = minss(maxss(v, 0.0), 1.0);
                let i = rt::fx::cvt(sqrt(v) * 4096.0) as u32;
                if i as usize > INV_N || t.out[c][i as usize] as usize != u {
                    return;
                }
            }
        }
        self.kind = ICX_IDENTITY;
    }

    /// the ICX_LUT grid  [xf_lut]
    fn lut(&mut self, xb: &Xb) {
        let t = &mut self.t;
        // axis tables: node positions (k/32)^2, {k * stride, frac} per 8-bit value
        for a in 0..3 {
            for u in 0..256 {
                let x = u as i32 as f64 * D_I255;
                let x1024 = x * D_1024;
                let mut k = cvttd(sqrtd(x) * D_32);
                while k != 0 && (k * k) as f64 > x1024 {
                    k -= 1;
                }
                while (k as u32) < GN as u32 - 1 && !(((k + 1) * (k + 1)) as f64 > x1024) {
                    k += 1;
                }
                if k as u32 > GN as u32 - 2 {
                    k = GN as i32 - 2;
                }
                let f = (x1024 - (k * k) as f64) / (k * 2 + 1) as f64;
                t.ax_off[a][u] = (k as usize * AX_STRIDE[a]) as u32;
                t.ax_f[a][u] = f as f32;
            }
        }
        t.lmode = false;
        if xb.dmode == MODE_MATRIX {
            t.lmode = true;
            out_tabs(t, xb);
        }
        t.grid = grid_build(xb);
        self.kind = ICX_LUT;
    }
}

/// out = round(inv * 255) per channel (sqrt-companded index)  [xf_outtabs]
fn out_tabs(t: &mut Tables, xb: &Xb) {
    for c in 0..3 {
        let inv = &xb.inv[c * OUT_N..(c + 1) * OUT_N];
        for j in 0..OUT_N {
            let v = minsd(maxsd(inv[j], D_ZERO), D_ONE);
            t.out[c][j] = cvttd(v * D_255 + D_HALF) as u8;
        }
    }
}

/// the 33^3 grid: one plane per index of the outer loop, on the worker pool when there is
/// one (every node depends only on its position: the same bytes either way)
fn grid_build(xb: &Xb) -> Vec<i32> {
    const PLANE: usize = GN * GN * 4;
    let mut grid = vec![0i32; GRID_N];
    let par = crate::parallel();
    match par {
        Some(par) => {
            let planes: Vec<SpinLock<&mut [i32]>> = grid.chunks_mut(PLANE).map(SpinLock::new).collect();
            par(GN, &|i| {
                let mut p = planes[i].lock();
                xb.grid_plane(i, &mut p);
            });
        }
        None => {
            for (i, p) in grid.chunks_mut(PLANE).enumerate() {
                xb.grid_plane(i, p);
            }
        }
    }
    grid
}

// ------------------------------------------------------------------ kernels

const BLOCK: usize = 1024; // in place: bytes per block through a stack buffer

/// ICX_MATRIX rows: 4 (SSE2) / 8 (AVX2) pixels per step  [xf_row_mat]
struct MatRows<'a> {
    t: &'a Tables,
    src: Option<&'a [u8]>, // None: in place
    dst: &'a mut [u8],
}

impl Kernel for MatRows<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let k = MatK::new(s, self.t);
        match self.src {
            Some(sp) => k.rows(sp, self.dst),
            None => {
                let mut buf = [0u8; BLOCK];
                for d in self.dst.chunks_mut(BLOCK) {
                    let n = d.len();
                    buf[..n].copy_from_slice(d);
                    k.rows(&buf[..n], d);
                }
            }
        }
    }
}

/// the matrix kernel's constants in registers  [RM_QUAD]
struct MatK<'a, S: Simd> {
    s: S,
    t: &'a Tables,
    mb: [S::V; 9],
    k0: S::V,
    k1: S::V,
    k4096: S::V,
}

impl<'a, S: Simd> MatK<'a, S> {
    #[inline(always)]
    fn new(s: S, t: &'a Tables) -> Self {
        let mut mb = [s.zero(); 9];
        for i in 0..9 {
            mb[i] = s.splatf(t.mb[i]);
        }
        MatK { s, t, mb, k0: s.zero(), k1: s.splatf(1.0), k4096: s.splatf(4096.0) }
    }

    /// src -> dst (the same length), S::BYTES / 4 pixels per step, the tail through a
    /// zeroed buffer
    #[inline(always)]
    fn rows(&self, src: &[u8], dst: &mut [u8]) {
        let mut q = [0u8; 32];
        let mut sc = src.chunks_exact(S::BYTES);
        let mut dc = dst.chunks_exact_mut(S::BYTES);
        for (sp, dp) in (&mut sc).zip(&mut dc) {
            q[..S::BYTES].copy_from_slice(sp);
            self.step(&q, dp);
        }
        let (st, dt) = (sc.remainder(), dc.into_remainder());
        if !dt.is_empty() {
            q = [0u8; 32];
            q[..st.len()].copy_from_slice(st);
            let mut o = [0u8; 32];
            self.step(&q, &mut o[..S::BYTES]);
            dt.copy_from_slice(&o[..dt.len()]);
        }
    }

    /// S::BYTES / 4 pixels from q to d (S::BYTES bytes)
    #[inline(always)]
    fn step(&self, q: &[u8; 32], d: &mut [u8]) {
        const MAXL: usize = 8;
        let (s, t) = (self.s, self.t);
        let l = S::BYTES / 4;
        let mut rv = [0i32; MAXL];
        let mut gv = [0i32; MAXL];
        let mut bv = [0i32; MAXL];
        for k in 0..l {
            rv[k] = t.inp[0][q[4 * k + 2] as usize];
            gv[k] = t.inp[1][q[4 * k + 1] as usize];
            bv[k] = t.inp[2][q[4 * k] as usize];
        }
        let r = s.load_i32(&rv);
        let g = s.load_i32(&gv);
        let b = s.load_i32(&bv);
        let mut idx = [[0i32; MAXL]; 3];
        for (c, ix) in idx.iter_mut().enumerate() {
            let mut a = s.mulf(r, self.mb[c * 3]);
            a = s.addf(a, s.mulf(g, self.mb[c * 3 + 1]));
            a = s.addf(a, s.mulf(b, self.mb[c * 3 + 2]));
            a = s.maxf(a, self.k0);
            a = s.minf(a, self.k1);
            a = s.sqrtf(a);
            a = s.mulf(a, self.k4096);
            s.store_i32(ix, s.cvtr_f32_i32(a));
        }
        for k in 0..l {
            let v = (t.out[0][idx[0][k] as usize] as u32) << 16
                | (t.out[1][idx[1][k] as usize] as u32) << 8
                | t.out[2][idx[2][k] as usize] as u32
                | (q[4 * k + 3] as u32) << 24;
            d[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

/// ICX_LUT rows: tetrahedral interpolation per pixel (f32), then linear -> clamp -> out
/// tables (lmode) or device * 255  [xf_row_lut]
fn lut_rows(t: &Tables, src: Option<&[u8]>, dst: &mut [u8]) {
    match src {
        Some(sp) => {
            if t.lmode {
                lut_px::<true>(t, sp, dst)
            } else {
                lut_px::<false>(t, sp, dst)
            }
        }
        None => {
            let mut buf = [0u8; BLOCK];
            for d in dst.chunks_mut(BLOCK) {
                let n = d.len();
                buf[..n].copy_from_slice(d);
                if t.lmode {
                    lut_px::<true>(t, &buf[..n], d)
                } else {
                    lut_px::<false>(t, &buf[..n], d)
                }
            }
        }
    }
}

/// first and second vertex of the tetrahedron by (fR >= fG) | (fG >= fB) << 1 | (fR >= fB) << 2,
/// the paths of xf_row_lut's decision tree (the two impossible combinations as the tree
/// would take them)
static TET: [(u16, u16); 8] = [
    (GS_B as u16, (GS_B + GS_G) as u16), // B > G > R
    (GS_B as u16, (GS_B + GS_R) as u16), // B > R >= G
    (GS_G as u16, (GS_G + GS_B) as u16), // G >= B > R
    (GS_R as u16, (GS_R + GS_G) as u16), // R >= G >= B
    (GS_G as u16, (GS_G + GS_R) as u16), // G > R >= B
    (GS_R as u16, (GS_R + GS_B) as u16), // R >= B > G
    (GS_G as u16, (GS_G + GS_R) as u16), // G > R >= B
    (GS_R as u16, (GS_R + GS_G) as u16), // R >= G >= B
];

#[inline(always)]
fn lut_px<const LIN: bool>(t: &Tables, src: &[u8], dst: &mut [u8]) {
    let s = simd::Sse2;
    let k0 = s.zero();
    let k1 = s.splatf(1.0);
    let k4096 = s.splatf(4096.0);
    let ks = s.splatf(255.0);
    let grid = &t.grid[..];
    for (sp, dp) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let (b, g, r) = (sp[0] as usize, sp[1] as usize, sp[2] as usize);
        let (fb, fg, fr) = (t.ax_f[0][b], t.ax_f[1][g], t.ax_f[2][r]);
        let v0 = (t.ax_off[0][b] + t.ax_off[1][g] + t.ax_off[2][r]) as usize;
        // the tetrahedron: the assembly's decision tree (three comparisons) as a table of the
        // first and second vertex; the weights in that order are always the fractions in
        // descending order (max, median, min -- the same values, no branches)
        let (o1, o2) = TET[(fr >= fg) as usize | ((fg >= fb) as usize) << 1 | ((fr >= fb) as usize) << 2];
        let (o1, o2) = (o1 as usize, o2 as usize);
        let w1 = maxss(maxss(fr, fg), fb);
        let w2 = maxss(minss(fr, fg), minss(maxss(fr, fg), fb));
        let w3 = minss(minss(fr, fg), fb);
        let cell = &grid[v0..v0 + GS_ALL + 4];
        let c0 = s.load_i32(&cell[..4]);
        let c1 = s.load_i32(&cell[o1..o1 + 4]);
        let c2 = s.load_i32(&cell[o2..o2 + 4]);
        let c3 = s.load_i32(&cell[GS_ALL..]);
        let mut acc = s.addf(c0, s.mulf(s.subf(c1, c0), s.splatf(w1)));
        acc = s.addf(acc, s.mulf(s.subf(c2, c1), s.splatf(w2)));
        acc = s.addf(acc, s.mulf(s.subf(c3, c2), s.splatf(w3)));
        let v = if LIN {
            // linear: clamp, sqrt index, inverse TRC tables (like the matrix path)
            let mut x = s.maxf(acc, k0);
            x = s.minf(x, k1);
            x = s.sqrtf(x);
            x = s.mulf(x, k4096);
            let mut ix = [0i32; 4];
            s.store_i32(&mut ix, s.cvtr_f32_i32(x));
            t.out[2][ix[0] as usize] as u32
                | (t.out[1][ix[1] as usize] as u32) << 8
                | (t.out[0][ix[2] as usize] as u32) << 16
        } else {
            let x = s.cvtr_f32_i32(s.mulf(acc, ks));
            let x = s.packs32(x, x);
            let x = s.packus16(x, x);
            s.to_i32(x) as u32 & 0x00FFFFFF
        };
        dp.copy_from_slice(&(v | (sp[3] as u32) << 24).to_le_bytes());
    }
}
