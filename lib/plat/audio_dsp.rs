// audio_dsp -- (audio.rs) the stream's ring, sample-rate conversion, generated tones and the
// level of one frequency
//
//   Ring        one producer (the app), one consumer (the audio thread), lock-free: the samples
//               are AtomicU32 bits, the write index is published with Release after them
//   Resampler   windowed sinc between two rates: 8 zero crossings on each side at the lower
//               Nyquist frequency (16 taps upward, wider downward), cut-off at 95 %, Blackman
//               window, the kernel tabulated at 256 phases per source frame and interpolated
//   tone(..)    a sine with 5 ms fades;  level(..) the amplitude of one frequency (Goertzel with
//               a Hann window: a full-scale sine reads 1.0)

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// a ring of f32 samples for one producer and one consumer
pub(crate) struct Ring {
    buf: Box<[AtomicU32]>,
    mask: usize,
    w: AtomicUsize,
    r: AtomicUsize,
}

impl Ring {
    /// room for at least `samples` samples (rounded up to a power of two)
    pub(crate) fn new(samples: usize) -> Ring {
        let cap = samples.max(64).next_power_of_two();
        Ring { buf: (0..cap).map(|_| AtomicU32::new(0)).collect(), mask: cap - 1, w: AtomicUsize::new(0), r: AtomicUsize::new(0) }
    }

    pub(crate) fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// samples waiting
    pub(crate) fn len(&self) -> usize {
        self.w.load(Ordering::Acquire).wrapping_sub(self.r.load(Ordering::Acquire)).min(self.capacity())
    }

    /// producer: n samples made by f(i) (the caller checked the room) -> samples taken
    pub(crate) fn push_with(&self, n: usize, f: impl Fn(usize) -> f32) -> usize {
        let w = self.w.load(Ordering::Relaxed);
        let r = self.r.load(Ordering::Acquire);
        let n = n.min(self.capacity() - w.wrapping_sub(r).min(self.capacity()));
        for i in 0..n {
            self.buf[w.wrapping_add(i) & self.mask].store(f(i).to_bits(), Ordering::Relaxed);
        }
        self.w.store(w.wrapping_add(n), Ordering::Release);
        n
    }

    /// consumer: up to out.len() samples -> samples taken
    pub(crate) fn pop(&self, out: &mut [f32]) -> usize {
        let r = self.r.load(Ordering::Relaxed);
        let w = self.w.load(Ordering::Acquire);
        let n = out.len().min(w.wrapping_sub(r));
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = f32::from_bits(self.buf[r.wrapping_add(i) & self.mask].load(Ordering::Relaxed));
        }
        self.r.store(r.wrapping_add(n), Ordering::Release);
        n
    }

    /// producer: the position after the last sample written (for drop_until)
    pub(crate) fn mark(&self) -> usize {
        self.w.load(Ordering::Acquire)
    }

    /// consumer: drop what was written before mark (later samples stay)
    pub(crate) fn drop_until(&self, mark: usize) {
        let r = self.r.load(Ordering::Relaxed);
        let ahead = mark.wrapping_sub(r);
        if ahead > 0 && ahead <= self.capacity() {
            self.r.store(mark, Ordering::Release);
        }
    }
}

const PHASES: usize = 256;
const ZERO_CROSSINGS: f64 = 8.0;
const PI: f64 = core::f64::consts::PI;

/// sample-rate conversion of interleaved frames (one per stream or voice)
pub(crate) struct Resampler {
    ch: usize,
    step: f64,
    half: usize,
    table: Vec<f32>,
    hist: Vec<f32>,
    pos: f64,
    chunk: Vec<f32>,
}

impl Resampler {
    /// from `from` to `to` frames per second, ch channels
    pub(crate) fn new(from: u32, to: u32, ch: usize) -> Resampler {
        let step = from.max(1) as f64 / to.max(1) as f64;
        let cut = 0.95 * if step > 1.0 { 1.0 / step } else { 1.0 };
        let half = libm_ceil(ZERO_CROSSINGS / cut) as usize;
        let n = half * PHASES + 2;
        let mut table = Vec::with_capacity(n);
        for i in 0..n {
            let x = i as f64 / PHASES as f64;
            let t = x / half as f64;
            let k = if t >= 1.0 {
                0.0
            } else {
                let sinc = if x == 0.0 { 1.0 } else { rt::x87::sin(PI * cut * x) / (PI * cut * x) };
                let win = 0.42 + 0.5 * rt::x87::cos(PI * t) + 0.08 * rt::x87::cos(2.0 * PI * t);
                cut * sinc * win
            };
            table.push(k as f32);
        }
        let mut r = Resampler { ch: ch.max(1), step, half, table, hist: Vec::new(), pos: 0.0, chunk: Vec::new() };
        r.reset();
        r
    }

    /// forget the history (after a flush)
    pub(crate) fn reset(&mut self) {
        self.hist.clear();
        self.hist.resize(self.half * self.ch, 0.0);
        self.pos = self.half as f64;
    }

    // the kernel at distance x (source frames)
    fn kernel(&self, x: f64) -> f32 {
        let t = if x < 0.0 { -x } else { x } * PHASES as f64;
        let i = t as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let f = (t - i as f64) as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * f
    }

    /// fill out (whole frames at the target rate); src fills its buffer with source frames
    /// (zeros past its end) and returns how many of them are real -> real source frames used
    pub(crate) fn process(&mut self, out: &mut [f32], src: &mut dyn FnMut(&mut [f32]) -> usize) -> usize {
        let ch = self.ch;
        let mut real = 0;
        for o in out.chunks_exact_mut(ch) {
            let base = self.pos as usize;
            while self.hist.len() / ch <= base + self.half {
                self.chunk.clear();
                self.chunk.resize(256 * ch, 0.0);
                real += src(&mut self.chunk);
                self.hist.extend_from_slice(&self.chunk);
            }
            o.fill(0.0);
            let first = base + 1 - self.half;
            for j in first..=base + self.half {
                let k = self.kernel(self.pos - j as f64);
                if k != 0.0 {
                    let f = &self.hist[j * ch..j * ch + ch];
                    for (o, &s) in o.iter_mut().zip(f) {
                        *o += s * k;
                    }
                }
            }
            self.pos += self.step;
        }
        // drop the frames no later output needs
        let keep_from = (self.pos as usize + 1).saturating_sub(self.half);
        if keep_from > 0 {
            self.hist.drain(..keep_from * ch);
            self.pos -= keep_from as f64;
        }
        real
    }
}

// ceil for small positive values (no libm)
fn libm_ceil(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t < x { t + 1.0 } else { t }
}

/// a sine of freq Hz, ms long, at rate frames per second, amplitude amp, faded in and out
/// over 5 ms (mono samples)
pub(crate) fn tone(freq: f32, ms: u32, rate: u32, amp: f32) -> Vec<f32> {
    let n = (rate as u64 * ms as u64 / 1000) as usize;
    let mut v = vec![0.0f32; n];
    let (cw, sw) = rt::x87::sincos(2.0 * PI * freq as f64 / rate.max(1) as f64);
    let (mut s, mut c) = (0.0f64, 1.0f64);
    let fade = (rate as usize / 200).max(1).min(n / 2 + 1);
    for (i, x) in v.iter_mut().enumerate() {
        let g = if i < fade { i as f64 / fade as f64 } else if n - i <= fade { (n - i) as f64 / fade as f64 } else { 1.0 };
        *x = (s * g * amp as f64) as f32;
        let (s2, c2) = (s * cw + c * sw, c * cw - s * sw);
        s = s2;
        c = c2;
        if i % 1024 == 1023 {
            let m = rt::fx::sqrtd(s * s + c * c);
            s /= m;
            c /= m;
        }
    }
    v
}

/// the amplitude of the sine at freq in channel ch of interleaved frames (Goertzel with a
/// Hann window; a full-scale sine reads 1.0, other frequencies a little)
pub(crate) fn level(samples: &[f32], channels: usize, ch: usize, rate: u32, freq: f32) -> f32 {
    let channels = channels.max(1);
    let n = samples.len() / channels;
    if n < 2 || ch >= channels {
        return 0.0;
    }
    let w = 2.0 * PI * freq as f64 / rate.max(1) as f64;
    let coeff = 2.0 * rt::x87::cos(w);
    let (mut s1, mut s2, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
    // the Hann window by rotation (cos of 2 pi i / (n - 1))
    let (hc, hs) = rt::x87::sincos(2.0 * PI / (n - 1) as f64);
    let (mut ws, mut wc) = (0.0f64, 1.0f64);
    for i in 0..n {
        let win = 0.5 - 0.5 * wc;
        wsum += win;
        let s = samples[i * channels + ch] as f64 * win + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
        let (a, b) = (ws * hc + wc * hs, wc * hc - ws * hs);
        ws = a;
        wc = b;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    if power <= 0.0 || wsum <= 0.0 {
        return 0.0;
    }
    (2.0 * rt::fx::sqrtd(power) / wsum) as f32
}
