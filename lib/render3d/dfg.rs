// dfg -- the second sum of the split-sum approximation for draw.wgsl's BRDF (G5c; docs/RENDER3D.md "PBR"): a
// table of (A, B) such that the specular light a white uniform environment reflects is f0 A + B, over n.v (x)
// and roughness (y), 32 x 32 points from 0 to 1, read bilinearly by the fragment shader from a storage buffer
// (no texture slot, the same exact weights on every path).  Made ahead by dfg_table (render3dtest --gen-dfg
// writes dfg_table.rs, --test shaders compares the embedded table with a fresh one): GGX importance sampling
// of the half vector, 4096 points of the Hammersley set per entry, in f64.  Karis' analytic fit was the
// first choice; for this BRDF's separable Smith term it lies up to 0.19 from the integral (measured by
// render3dtest --test pbr), the table within a few thousandths.

use alloc::vec::Vec;

use crate::math;

/// points of the table per axis
pub const DFG_N: usize = 32;
/// samples per entry
const SAMPLES: u32 = 4096;
/// draw.wgsl's ALPHA_MIN
const ALPHA_MIN: f64 = 0.001;

mod table {
    include!("dfg_table.rs");
}

/// the embedded table: entry j * DFG_N + i holds (A, B) at n.v = i / (DFG_N - 1), roughness j / (DFG_N - 1)
pub const DFG: [[f32; 2]; DFG_N * DFG_N] = {
    let mut out = [[0.0f32; 2]; DFG_N * DFG_N];
    let mut k = 0;
    while k < DFG_N * DFG_N {
        out[k] = [f32::from_bits(table::DFG_BITS[2 * k]), f32::from_bits(table::DFG_BITS[2 * k + 1])];
        k += 1;
    }
    out
};

/// the table computed (what dfg_table.rs holds; slow: some 4 million samples)
pub fn dfg_table() -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(DFG_N * DFG_N);
    let last = (DFG_N - 1) as f64;
    for j in 0..DFG_N {
        let r = j as f64 / last;
        let alpha = (r * r).max(ALPHA_MIN);
        let a2 = alpha * alpha;
        for i in 0..DFG_N {
            // (n.v = 0 exactly has no direction to see from)
            let nv = (i as f64 / last).max(1e-4);
            let v = [math::sqrt(1.0 - nv * nv), 0.0, nv];
            let (mut a, mut b) = (0.0, 0.0);
            for s in 0..SAMPLES {
                let e1 = s as f64 / SAMPLES as f64;
                let e2 = s.reverse_bits() as f64 / 4_294_967_296.0;
                // the half vector with density D(h) n.h
                let cos_t = math::sqrt((1.0 - e2) / (1.0 + (a2 - 1.0) * e2));
                let sin_t = math::sqrt(1.0 - cos_t * cos_t);
                let (sp, cp) = math::sincos(e1 * core::f64::consts::TAU);
                let h = [sin_t * cp, sin_t * sp, cos_t];
                let vh = math::dot(v, h);
                let nl = 2.0 * vh * h[2] - v[2];
                if nl <= 0.0 || vh <= 0.0 {
                    continue;
                }
                // f n.l / pdf(l) without F, pdf(l) = D n.h / (4 v.h): 4 V n.l v.h / n.h
                let vis = 1.0 / ((nl + math::sqrt(a2 + (1.0 - a2) * nl * nl)) * (nv + math::sqrt(a2 + (1.0 - a2) * nv * nv)));
                let w = 4.0 * vis * nl * vh / cos_t;
                let x = 1.0 - vh;
                let fc = x * x * x * x * x;
                a += (1.0 - fc) * w;
                b += fc * w;
            }
            out.push([(a / SAMPLES as f64) as f32, (b / SAMPLES as f64) as f32]);
        }
    }
    out
}
