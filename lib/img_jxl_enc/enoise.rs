// enoise -- photon noise for the new lossy writer: the noise LUT of a simulated camera sensor
// at an ISO setting, written into the frame (libjxl 0.11.1 enc_photon_noise.cc
// SimulatePhotonNoise, enc_noise.cc EncodeNoise; cjxl --photon_noise_iso)  [jxl-tools: no asm]
//
//   lut(w, h, iso) -> [f32; 8]     NoiseParams for an image of w x h pixels (a 35 mm sensor
//                                  of that resolution at ISO iso)
//   write(lut, jw)                 the 8 values, 10 bits each
//
// The decoder synthesizes the grain (noise.rs); the encoder only describes it, so a larger
// ISO costs no bits.  Off by default, as in libjxl.

use crate::ewr::Jw;

/// kOpsinAbsorbanceBias (Y)
const BIAS: f64 = 0.003_793_073_255_275_449_3;
const PHOTONS_PER_LXS_PER_UM2: f64 = 11260.0;
const QUANTUM_EFFICIENCY: f64 = 0.20;
const PRNU: f64 = 0.005;
const READ_NOISE: f64 = 3.0;
const SENSOR_AREA_UM2: f64 = 36000.0 * 24000.0;

fn cbrt(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let a = x.abs();
    // Newton from a bit-level estimate
    let mut y = f64::from_bits(a.to_bits() / 3 + 0x2A9F_7893_782D_A1CE);
    for _ in 0..6 {
        y = y - (y * y * y - a) / (3.0 * y * y);
    }
    if x < 0.0 { -y } else { y }
}

/// SimulatePhotonNoise
pub fn lut(w: u32, h: u32, iso: f32) -> [f32; 8] {
    let bias_cbrt = cbrt(BIAS);
    let h18 = 10.0 / iso as f64;
    let pixel_area = SENSOR_AREA_UM2 / (w as f64 * h as f64);
    let e18 = QUANTUM_EFFICIENCY * PHOTONS_PER_LXS_PER_UM2 * h18 * pixel_area;
    let mut out = [0f32; 8];
    for (i, o) in out.iter_mut().enumerate() {
        let scaled = i as f64 / (8.0 - 2.0);
        let y = 2.0 * scaled;
        let t = y - bias_cbrt;
        let linear = (t * t * t + BIAS).max(0.0);
        let e = e18 * (linear / 0.18);
        let noise = rt::fx::sqrtd(READ_NOISE * READ_NOISE + e + (PRNU * e) * (PRNU * e));
        let lin_noise = noise * (0.18 / e18);
        let c = cbrt(linear - BIAS);
        let deriv = (1.0 / 3.0) / (c * c);
        let opsin_noise = lin_noise * deriv;
        let v = opsin_noise / (0.22 * core::f64::consts::SQRT_2 * 1.13);
        *o = (if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }) as f32;
    }
    out
}

/// EncodeNoise: 8 x 10 bits (value * 1024, rounded)
pub fn write(lut: &[f32; 8], jw: &mut Jw) {
    for &v in lut {
        let q = (v * 1024.0 + 0.5) as i32;
        jw.bits(q.clamp(0, 1023) as u64, 10);
    }
}
