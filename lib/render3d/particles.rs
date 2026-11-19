// particles -- (G7f) billboards from a buffer (docs/RENDER3D.md "Partikel"): emitters on the CPU, a ring of particles on
// the GPU, their motion in a compute shader (particles.wgsl), sorted back to front (sort.wgsl), one indirect draw.
//
// A particle is born on the CPU (Scene::emit): its randomness comes from a hash of the emitter's seed and the particle's
// serial number in that emitter, so the same calls give the same particles on every device and in every run.  Its record
// (64 bytes: place, birth in microseconds, velocity, life, sizes, colours, gravity and drag, flags) goes into the next places
// of the ring (the oldest are overwritten), copied inside the frame's commands.  Nothing changes it afterwards: the compute
// shader evaluates its motion in closed form at its age every frame (constant gravity along -y and a linear drag), and
// puts each living particle into the frame's list with the key of its distance; sort.wgsl sorts the list, one draw with six
// vertices per particle draws it.  No read-back, no state on the GPU besides the births.

use gpu::{GpuError, Result};

use crate::env::to_half;
use crate::math::{self, V3};

/// (G7f) how a particle is laid over the image (both through the premultiplied blend state: an additive particle returns
/// alpha 0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParticleBlend {
    /// adds its light (fire, sparks, muzzle flashes)
    Additive,
    /// covers what is behind by its alpha, premultiplied (smoke, dust)
    Alpha,
}

/// (G7f) the shape of a particle's billboard
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParticleShape {
    /// a soft disc facing the camera (opacity 1 - r^2 from the middle to the rim)
    Disc,
    /// the disc stretched along the particle's velocity on the screen by Emitter::stretch (sparks)
    Streak,
}

/// (G7f) an emitter of a Scene (Scene::add_emitter): what the particles it emits look like and how they move.  Made with
/// `new` and changed field by field (it may grow).  Ranges [a, b] are drawn uniformly per particle
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Emitter {
    /// the seed of its particles' randomness: particle k of this emitter (counted from its first emit) is the same in every
    /// run with the same calls
    pub seed: u32,
    pub blend: ParticleBlend,
    pub shape: ParticleShape,
    /// ParticleShape::Streak: how long the streak is per speed (seconds): its length is size + speed * stretch
    pub stretch: f32,
    /// its particles live in the viewmodel's space (Scene::add_viewmodel; Scene::emit takes the camera's space: x right, y
    /// up, -z forward) and are drawn with the viewmodel, in front of the world (a muzzle flash at the weapon); gravity
    /// acts along the camera's -y.  false: the world's
    pub viewmodel: bool,
    /// seconds a particle lives (above 0)
    pub life: [f32; 2],
    /// the speed at birth (world units per second), along a direction in a cone around Scene::emit's direction
    pub speed: [f32; 2],
    /// the cone's half angle in radians (0: along the direction, pi: any direction)
    pub spread: f32,
    /// particles are born within a ball of this radius around Scene::emit's point
    pub radius: f32,
    /// the billboard's width at birth and at death (world units), linear in between
    pub size: [f32; 2],
    /// linear colour (radiance, before the exposure) and opacity at birth and at death, linear in between
    pub color: [[f32; 4]; 2],
    /// acceleration downwards (along -y; negative: rising), world units per second squared
    pub gravity: f32,
    /// linear drag (1 / seconds): the velocity falls by e^(-drag t) towards gravity / drag
    pub drag: f32,
}

impl Emitter {
    /// additive white discs of 0.1 that fade out over a second, at speed 1 straight along the direction
    pub const fn new(seed: u32) -> Emitter {
        Emitter { seed, blend: ParticleBlend::Additive, shape: ParticleShape::Disc, stretch: 0.0, viewmodel: false, life: [1.0, 1.0], speed: [1.0, 1.0], spread: 0.0,
                  radius: 0.0, size: [0.1, 0.1], color: [[1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]], gravity: 0.0, drag: 0.0 }
    }

    pub(crate) fn check(&self) -> Result<()> {
        let fin = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let range = |r: [f32; 2]| r[0] <= r[1];
        let fine = fin(&self.life) && fin(&self.speed) && fin(&self.size) && fin(&self.color[0]) && fin(&self.color[1])
            && fin(&[self.stretch, self.spread, self.radius, self.gravity, self.drag]) && self.life[0] > 0.0 && range(self.life) && range(self.speed)
            && self.speed[0] >= 0.0 && self.size.iter().all(|&s| s >= 0.0) && self.stretch >= 0.0 && (0.0..=core::f32::consts::PI).contains(&self.spread)
            && self.radius >= 0.0 && self.drag >= 0.0 && self.life[1] <= 4000.0 && self.size.iter().all(|&s| s <= 60000.0) && self.gravity.abs() <= 60000.0
            && self.drag <= 60000.0 && self.stretch <= 60000.0 && self.color.iter().flatten().all(|&c| (0.0..=60000.0).contains(&c));
        if fine {
            Ok(())
        } else {
            Err(GpuError::Invalid("Emitter: finite values, 0 < life (at most 4000 s), ranges [a, b] with a <= b, speed, size, colour, radius, drag and stretch not negative, spread 0 ..= pi, halves (at most 60000)"))
        }
    }
}

/// (G7f) an emitter of a Scene; dead after Scene::remove_emitter
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmitterId {
    pub slot: u32,
    era: u32,
}

impl EmitterId {
    pub(crate) const fn new(slot: u32, era: u32) -> EmitterId {
        EmitterId { slot, era }
    }

    pub(crate) const fn era(&self) -> u32 {
        self.era
    }
}

/// bytes of a particle's record (particles.wgsl Particle)
pub(crate) const PARTICLE_BYTES: u64 = 64;
/// flags of a record: additive, a streak, in the viewmodel's space
pub(crate) const P_ADDITIVE: u32 = 1;
pub(crate) const P_STREAK: u32 = 2;
pub(crate) const P_VIEWMODEL: u32 = 4;

/// the hash of the randomness (a PCG step and its output permutation, as in PCG-RXS-M-XS)
pub fn hash(x: u32) -> u32 {
    let s = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277_803_737);
    (w >> 22) ^ w
}

/// a stream of uniform numbers in [0, 1) from a seed
struct Stream(u32);

impl Stream {
    fn next(&mut self) -> f64 {
        self.0 = hash(self.0);
        (self.0 >> 8) as f64 / (1u32 << 24) as f64
    }

    /// a point in the unit disc (rejection; after 32 tries the middle)
    fn disc(&mut self) -> (f64, f64) {
        for _ in 0..32 {
            let (x, y) = (2.0 * self.next() - 1.0, 2.0 * self.next() - 1.0);
            if x * x + y * y <= 1.0 {
                return (x, y);
            }
        }
        (0.0, 0.0)
    }

    /// a point in the unit ball (rejection; after 32 tries the middle)
    fn ball(&mut self) -> V3 {
        for _ in 0..32 {
            let p = [2.0 * self.next() - 1.0, 2.0 * self.next() - 1.0, 2.0 * self.next() - 1.0];
            if math::dot(p, p) <= 1.0 {
                return p;
            }
        }
        [0.0; 3]
    }
}

fn lerp(r: [f32; 2], t: f64) -> f64 {
    r[0] as f64 + (r[1] as f64 - r[0] as f64) * t
}

/// two f32 as two halves in a word (the first in the low half), as WGSL's pack2x16float
fn halves(a: f32, b: f32) -> u32 {
    to_half(a) as u32 | (to_half(b) as u32) << 16
}

/// what one emit computes once for all its particles: a unit direction, two axes at right angles to it, the cone's cosine
pub(crate) struct Aim {
    dir: V3,
    a: V3,
    b: V3,
    cos: f64,
}

impl Aim {
    pub(crate) fn new(e: &Emitter, dir: V3) -> Aim {
        let d = math::norm(dir);
        let d = if math::dot(d, d) > 0.0 { d } else { [0.0, 1.0, 0.0] };
        let other = if d[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
        let a = math::norm(math::cross(d, other));
        let b = math::cross(d, a);
        Aim { dir: d, a, b, cos: math::sincos(e.spread as f64).1 }
    }
}

/// particle `serial` of emitter e (its seed's stream), born at `at` (the world's position relative to the origin, or the
/// viewmodel's in the camera's space) at birth_us: its record (particles.wgsl Particle)
pub(crate) fn spawn(e: &Emitter, aim: &Aim, serial: u32, at: V3, birth_us: u32) -> [u32; 16] {
    let mut s = Stream(hash(e.seed ^ hash(serial.wrapping_mul(0x9E37_79B9))));
    let life = lerp(e.life, s.next());
    let speed = lerp(e.speed, s.next());
    // a direction in the cone: cos(theta) uniform in [cos(spread), 1], the azimuth from a point of the unit disc
    let (x, y) = s.disc();
    let r2 = x * x + y * y;
    let ct = 1.0 - r2 * (1.0 - aim.cos);
    let st = math::sqrt((1.0 - ct * ct).max(0.0));
    let (ux, uy) = if r2 > 0.0 { let r = math::sqrt(r2); (x / r, y / r) } else { (1.0, 0.0) };
    let d = math::add(math::mul(aim.dir, ct), math::add(math::mul(aim.a, st * ux), math::mul(aim.b, st * uy)));
    let off = math::mul(s.ball(), e.radius as f64);
    let p = math::add(at, off);
    let v = math::mul(d, speed);
    let flags = match e.blend {
        ParticleBlend::Additive => P_ADDITIVE,
        ParticleBlend::Alpha => 0,
    } | if e.shape == ParticleShape::Streak { P_STREAK } else { 0 } | if e.viewmodel { P_VIEWMODEL } else { 0 };
    let f = |v: f64| (v as f32).to_bits();
    let (c0, c1) = (e.color[0], e.color[1]);
    [f(p[0]), f(p[1]), f(p[2]), birth_us, f(v[0]), f(v[1]), f(v[2]), f(life), halves(e.size[0], e.size[1]), halves(c0[0], c0[1]), halves(c0[2], c0[3]),
     halves(c1[0], c1[1]), halves(c1[2], c1[3]), halves(e.gravity, e.drag), flags | (to_half(e.stretch) as u32) << 16, 0]
}

/// a point and its velocity of a record at age t (seconds), in its own space, as particles.wgsl's `motion` computes it:
/// gravity along -y, drag k: v(t) = a + (v0 - a) e^(-k t) with a = g / k, p(t) = p0 + a t + (v0 - a)(1 - e^(-k t)) / k; without
/// drag (k below 1e-4) p0 + v0 t + g t^2 / 2.  In f64 (tests and tools)
pub fn motion(rec: &[u32; 16], t: f64) -> (V3, V3) {
    let g = |i: usize| f32::from_bits(rec[i]) as f64;
    let half = |w: u32, hi: bool| crate::env::from_half(if hi { (w >> 16) as u16 } else { w as u16 }) as f64;
    let (p0, v0) = ([g(0), g(1), g(2)], [g(4), g(5), g(6)]);
    let (grav, k) = (half(rec[13], false), half(rec[13], true));
    let acc = [0.0, -grav, 0.0];
    if k > 1e-4 {
        let e = math::exp(-k * t);
        let a = math::mul(acc, 1.0 / k);
        let dv = math::sub(v0, a);
        (math::add(math::add(p0, math::mul(a, t)), math::mul(dv, (1.0 - e) / k)), math::add(a, math::mul(dv, e)))
    } else {
        (math::add(math::add(p0, math::mul(v0, t)), math::mul(acc, 0.5 * t * t)), math::add(v0, math::mul(acc, t)))
    }
}

/// the particles' clock in microseconds (wrapping after 71 minutes: ages are differences)
pub(crate) fn micros(t: f64) -> u32 {
    let us = t * 1e6;
    if us.is_finite() && us >= 0.0 { (us as u64 & 0xFFFF_FFFF) as u32 } else { 0 }
}
