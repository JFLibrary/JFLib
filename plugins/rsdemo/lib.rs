// rsdemo -- example plugin with two effects, showing the parts of the plugin crate the ports of
// the assembly examples do not use: a prepare step with its aux (Effect::PREPARE), PNAPI
// parallel_for, hash32, a seed property and the alloc crate on the host's allocator (PnAlloc).
// (No assembly counterpart; plugins/tests/rsdemo.uis checks it against a Python model.)
//
//   python build_rs.py plugin rsdemo  -> build/<target>/plugins/rsdemo.dll / .so  (Effects)
//
// "Kontrast strecken" (Korrekturen): prepare finds the range lo..hi of every colour channel over
// the visible pixels (alpha > 0) -- row bands on the host's workers, one result per band, reduced
// in band order -- and builds a table per channel that maps lo..hi to 0..255 (with "Kanäle
// einzeln" off: one common range), mixed with the identity by "Stärke" %; render looks the
// pixels up.  "Körnung" (Effekte ▸ Rauschen): adds -n..n to r, g, b, n from PA_HASH32(x, y,
// seed, 0) -- the same noise for every ROI split, thread count and host.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use plugin::*;

#[global_allocator]
static ALLOC: PnAlloc = PnAlloc;

// ------------------------------------------------------------------ Kontrast strecken

struct Stretch;

/// the tables of the run: r, g, b
struct Lut {
    t: [[u8; 256]; 3],
}

const BANDS: usize = 32;

impl Effect for Stretch {
    type Aux = Lut;
    const PREPARE: bool = true;

    fn prepare(fx: &Fx<'_>) -> Result<Option<Lut>, i32> {
        let per_channel = fx.param_bool(0);
        let strength = fx.param_int(1).clamp(0, 100) as u32;
        let h = fx.height() as usize;
        let n = BANDS.min(h.max(1));
        // per band: lo r, g, b, hi r, g, b
        let stats: Vec<[AtomicU32; 6]> = (0..n).map(|_| [255, 255, 255, 0, 0, 0].map(AtomicU32::new)).collect();
        parallel_for(n, &|i, job| {
            let mut s = [255u32, 255, 255, 0, 0, 0];
            for y in i * h / n..(i + 1) * h / n {
                if job.is_cancelled() {
                    return;
                }
                for &p in fx.src_row(y as i32) {
                    if p >> 24 == 0 {
                        continue;
                    }
                    for (c, sh) in [16, 8, 0].into_iter().enumerate() {
                        let v = (p >> sh) & 0xFF;
                        s[c] = s[c].min(v);
                        s[c + 3] = s[c + 3].max(v);
                    }
                }
            }
            for (k, v) in s.into_iter().enumerate() {
                stats[i][k].store(v, Ordering::Relaxed);
            }
        })?;
        if fx.cancelled() {
            return Err(ERR_CANCEL);
        }
        let mut r = [255u32, 255, 255, 0, 0, 0];
        for b in &stats {
            for k in 0..3 {
                r[k] = r[k].min(b[k].load(Ordering::Relaxed));
                r[k + 3] = r[k + 3].max(b[k + 3].load(Ordering::Relaxed));
            }
        }
        if !per_channel {
            let (lo, hi) = (r[0].min(r[1]).min(r[2]), r[3].max(r[4]).max(r[5]));
            r = [lo, lo, lo, hi, hi, hi];
        }
        let mut lut = Lut { t: [[0; 256]; 3] };
        for c in 0..3 {
            let (lo, hi) = (r[c], r[c + 3]);
            for v in 0..256u32 {
                let st = if hi <= lo {
                    v // nothing visible / one value: identity
                } else if v <= lo {
                    0
                } else if v >= hi {
                    255
                } else {
                    ((v - lo) * 255 + (hi - lo) / 2) / (hi - lo)
                };
                lut.t[c][v as usize] = ((v * (100 - strength) + st * strength + 50) / 100) as u8;
            }
        }
        Ok(Some(lut))
    }

    fn render(fx: &mut Fx<'_>, aux: Option<&Lut>) -> Result<(), i32> {
        let lut = aux.ok_or(ERR_FAIL)?;
        for y in fx.y0..fx.y1 {
            if fx.cancelled() {
                return Err(ERR_CANCEL);
            }
            let (src, dst) = fx.rows(y);
            for (d, &p) in dst.iter_mut().zip(src) {
                let [b, g, r, a] = p.to_le_bytes();
                *d = u32::from_le_bytes([lut.t[2][b as usize], lut.t[1][g as usize], lut.t[0][r as usize], a]);
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ Körnung

struct Grain;

impl Effect for Grain {
    type Aux = ();

    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        let n = fx.param_int(0).clamp(0, 100) as i32;
        let seed = fx.param_int(1) as u32;
        let x0 = fx.x0;
        for y in fx.y0..fx.y1 {
            if fx.cancelled() {
                return Err(ERR_CANCEL);
            }
            let (src, dst) = fx.rows(y);
            for (i, (d, &p)) in dst.iter_mut().zip(src).enumerate() {
                let h = hash32((x0 + i as i32) as u32, y as u32, seed, 0);
                let k = (((h & 0xFFFF) * (2 * n as u32 + 1)) >> 16) as i32 - n;
                let ch = |v: u32| ((v & 0xFF) as i32 + k).clamp(0, 255) as u32;
                *d = (p & 0xFF00_0000) | ch(p >> 16) << 16 | ch(p >> 8) << 8 | ch(p);
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ descriptors

static STRETCH_PROPS: [FxProp; 2] =
    [FxProp::boolean(0, c"Kanäle einzeln", false), FxProp::int(1, c"Stärke", 0.0, 100.0, 100.0)];
static FX_STRETCH: FxDesc = FxDesc::new::<Stretch>(c"Kontrast strecken", FXF_ADJ, FXC_ADJUST, &STRETCH_PROPS);
static GRAIN_PROPS: [FxProp; 2] = [FxProp::int(0, c"Menge", 0.0, 100.0, 25.0), FxProp::seed(1, c"Zufall")];
static FX_GRAIN: FxDesc = FxDesc::new::<Grain>(c"Körnung", FXF_SEED, FXC_NOISE, &GRAIN_PROPS);
static FX_LIST: [&FxDesc; 2] = [&FX_STRETCH, &FX_GRAIN];
static INFO: PluginInfo = PluginInfo::new(c"Rust-Beispiele", c"1.0")
    .author(c"jfguiLib")
    .url(c"plugins/rsdemo")
    .effects(&FX_LIST);

// keeps the API table (prepare / render use parallel_for, hash32 and the allocator)
fn query(api: &'static PnApi) -> Option<&'static PluginInfo> {
    if api.size < PA_SIZE_V1 {
        return None;
    }
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
