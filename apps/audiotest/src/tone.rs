// tone -- the tone generator's fill (plat::audio::Stream::open_fill): sine or square wave,
// frequency and volume shared with the UI through atomics, changes glide over one buffer
// (no clicks), the phase runs on

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

pub const WAVE_SINE: u32 = 0;
pub const WAVE_SQUARE: u32 = 1;

/// what the generator plays (set from the UI thread)
pub struct Gen {
    freq: AtomicU32,
    vol: AtomicU32,
    wave: AtomicU32,
}

impl Gen {
    pub fn new(freq: f32, vol: f32, wave: u32) -> Arc<Gen> {
        Arc::new(Gen { freq: AtomicU32::new(freq.to_bits()), vol: AtomicU32::new(vol.to_bits()), wave: AtomicU32::new(wave) })
    }

    pub fn set(&self, freq: f32, vol: f32, wave: u32) {
        self.freq.store(freq.to_bits(), Ordering::Relaxed);
        self.vol.store(vol.to_bits(), Ordering::Relaxed);
        self.wave.store(wave, Ordering::Relaxed);
    }
}

const TABLE: usize = 1024;

/// the fill of a stereo stream at rate
pub fn fill(g: Arc<Gen>, rate: u32) -> impl FnMut(&mut [f32]) -> bool + Send {
    // one period of a sine, sampled (the last entry repeats the first: interpolation)
    let table: Vec<f32> = (0..=TABLE).map(|i| rt::x87::sin(2.0 * core::f64::consts::PI * i as f64 / TABLE as f64) as f32).collect();
    let mut phase = 0.0f64;
    let (mut f0, mut v0) = (0.0f32, 0.0f32);
    move |buf: &mut [f32]| {
        let f1 = f32::from_bits(g.freq.load(Ordering::Relaxed)).clamp(20.0, 20_000.0);
        let v1 = f32::from_bits(g.vol.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let wave = g.wave.load(Ordering::Relaxed);
        if f0 == 0.0 {
            f0 = f1;
        }
        let n = (buf.len() / 2).max(1) as f32;
        for (i, fr) in buf.chunks_exact_mut(2).enumerate() {
            let k = (i + 1) as f32 / n;
            let (f, v) = (f0 + (f1 - f0) * k, v0 + (v1 - v0) * k);
            let s = if wave == WAVE_SQUARE {
                // a softened square (quieter: it is loud by nature)
                if phase < 0.5 { 0.5 } else { -0.5 }
            } else {
                let x = phase * TABLE as f64;
                let j = x as usize;
                let t = (x - j as f64) as f32;
                table[j] + (table[j + 1] - table[j]) * t
            };
            fr[0] = s * v;
            fr[1] = s * v;
            phase += f as f64 / rate as f64;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
        f0 = f1;
        v0 = v1;
        true
    }
}
