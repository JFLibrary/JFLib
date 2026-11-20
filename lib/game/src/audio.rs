// audio -- (G7b part 2) spatial audio over plat::audio's mixer: a listener (the camera), sounds on entities and at
// points, the volume by distance (near, far, a rolloff), the pan by direction (docs/GAMECORE.md "Raumklang")
//
//   Attenuation { near, far, rolloff }   the gain by distance: 1 up to near; Linear falls to 0 at far, Inverse
//                                  (OpenAL's inverse distance, clamped) near / (near + rolloff (d - near)) up to far
//                                  and stays there beyond it
//   gains(&View, at, volume, &att) -> Gains   left and right gain of a sound at `at` for a listener: equal power
//                                  (left = cos, right = sin of (pan + 1) pi / 4, so -3 dB each in the middle), pan
//                                  the sine of the direction's angle to the right (front and back sound the same:
//                                  stereo), drawn to the middle within PAN_RADIUS of the listener
//   Emitter (a component)          a sound on an entity, played at its Pose (interpolated at the frame's alpha)
//   SpatialAudio                   set_listener(&View), update(&mut World, alpha, &mut dyn Mixer) once a frame:
//                                  starts the emitters' voices, sets every voice's gains; play_at(sound, at, ..):
//                                  a sound at a point (a shot, a hit) until it ends
//   Mixer                          where voices play: Speakers (plat::audio's shared mixer, play_gains), or
//                                  plat::audio::MixTest (the same mixer without a device: tests, sample by sample)
//
// No Doppler: plat's mixer plays a voice at its rate (a pitch per voice would need its resampler steered).  A voice's
// gains change at the mixer's next buffer (about 20 ms), without a ramp.  Like the rest of lib/game the gains are the
// same bits on every target: + - x / and lib/physics' math (its square root, rt's portable sine and cosine).

use alloc::vec::Vec;

use crate::camera::View;
use crate::pose::Pose;
use crate::world::World;
use physics::Vec3;

pub use plat::audio::{AudioError, Sound, Voice};

/// a source this close to the listener drifts to the middle (metres): no jump from left to right through the head
pub const PAN_RADIUS: f64 = 0.5;

/// how the gain falls beyond near
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rolloff {
    /// to 0 at far
    Linear,
    /// near / (near + rolloff (d - near)), d clamped to far (OpenAL's AL_INVERSE_DISTANCE_CLAMPED)
    Inverse(f64),
}

/// the gain by distance (metres)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attenuation {
    pub near: f64,
    pub far: f64,
    pub rolloff: Rolloff,
}

impl Attenuation {
    /// near 1 m, far 50 m, inverse with rolloff 1 (half the gain at 2 m, a fiftieth at 50 m and beyond)
    pub const DEFAULT: Attenuation = Attenuation { near: 1.0, far: 50.0, rolloff: Rolloff::Inverse(1.0) };

    /// linear from near to far
    pub const fn linear(near: f64, far: f64) -> Attenuation {
        Attenuation { near, far, rolloff: Rolloff::Linear }
    }

    /// the gain at distance d (0 ..= 1)
    pub fn gain(&self, d: f64) -> f64 {
        let near = if self.near.is_finite() && self.near > 1e-3 { self.near } else { 1e-3 };
        let far = if self.far.is_finite() && self.far > near { self.far } else { near };
        if d.is_nan() {
            return 0.0;
        }
        if d <= near {
            return 1.0;
        }
        match self.rolloff {
            Rolloff::Linear => {
                if d >= far {
                    0.0
                } else {
                    (far - d) / (far - near)
                }
            }
            Rolloff::Inverse(r) => {
                let r = if r.is_finite() && r >= 0.0 { r } else { 1.0 };
                let dc = if d < far { d } else { far };
                near / (near + r * (dc - near))
            }
        }
    }
}

impl Default for Attenuation {
    fn default() -> Self {
        Attenuation::DEFAULT
    }
}

/// what a listener hears of a source
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gains {
    pub left: f32,
    pub right: f32,
    /// the distance (metres), the pan (-1 left .. 1 right), the gain before the pan (volume x attenuation)
    pub distance: f64,
    pub pan: f64,
    pub gain: f64,
}

/// the gains of a sound at `at` with `volume` (0 ..= 1) for the listener (see the module header)
pub fn gains(listener: &View, at: Vec3, volume: f64, att: &Attenuation) -> Gains {
    let d = at - listener.eye;
    let distance = d.length();
    let mut pan = 0.0;
    if distance > 1e-9 {
        let right = listener.right();
        let rl = right.length();
        if rl > 1e-12 {
            pan = (d.dot(right) / (distance * rl)).clamp(-1.0, 1.0);
        }
        if distance < PAN_RADIUS {
            pan *= distance / PAN_RADIUS;
        }
    }
    let volume = if volume.is_nan() { 0.0 } else { volume.clamp(0.0, 1.0) };
    let gain = volume * att.gain(distance);
    let (s, c) = physics::math::sincos((pan + 1.0) * core::f64::consts::FRAC_PI_4);
    Gains { left: (gain * c) as f32, right: (gain * s) as f32, distance, pan, gain }
}

/// where voices play
pub trait Mixer {
    /// start `sound` with these gains (they hold from its first sample on)
    fn play(&mut self, sound: &Sound, left: f32, right: f32) -> Result<Voice, AudioError>;
}

/// plat::audio's shared mixer (the speakers; an app with a manifest needs `audio.output`)
pub struct Speakers;

impl Mixer for Speakers {
    fn play(&mut self, sound: &Sound, left: f32, right: f32) -> Result<Voice, AudioError> {
        plat::audio::play_gains(sound, left, right)
    }
}

impl Mixer for plat::audio::MixTest {
    fn play(&mut self, sound: &Sound, left: f32, right: f32) -> Result<Voice, AudioError> {
        let v = plat::audio::MixTest::play(self, sound);
        v.set_gains(left, right);
        Ok(v)
    }
}

enum Play {
    /// not started yet
    Waiting,
    Playing(Voice),
    /// ended (not looping), stopped, or refused by the mixer
    Done,
}

/// a sound on an entity (a component): it plays at the entity's Pose (the origin without one) from the next
/// SpatialAudio::update on; looping: started again when it ends
pub struct Emitter {
    pub sound: Sound,
    pub volume: f32,
    pub att: Attenuation,
    pub looping: bool,
    play: Play,
    /// the gains set last
    last: Option<Gains>,
}

impl Emitter {
    /// once, at volume 1, with the default attenuation
    pub fn new(sound: Sound) -> Emitter {
        Emitter { sound, volume: 1.0, att: Attenuation::DEFAULT, looping: false, play: Play::Waiting, last: None }
    }

    pub fn with_volume(mut self, v: f32) -> Emitter {
        self.volume = v;
        self
    }

    pub fn with_attenuation(mut self, a: Attenuation) -> Emitter {
        self.att = a;
        self
    }

    pub fn looping(mut self) -> Emitter {
        self.looping = true;
        self
    }

    /// its voice plays now
    pub fn playing(&self) -> bool {
        matches!(&self.play, Play::Playing(v) if v.playing())
    }

    /// it has ended (or was stopped, or the mixer refused it)
    pub fn finished(&self) -> bool {
        matches!(self.play, Play::Done)
    }

    /// play it again from the start at the next update
    pub fn restart(&mut self) {
        if let Play::Playing(v) = &self.play {
            v.stop();
        }
        self.play = Play::Waiting;
    }

    /// end it now
    pub fn stop(&mut self) {
        if let Play::Playing(v) = &self.play {
            v.stop();
        }
        self.play = Play::Done;
    }

    /// what the last update set
    pub fn gains(&self) -> Option<Gains> {
        self.last
    }
}

impl Drop for Emitter {
    fn drop(&mut self) {
        // (an entity that goes takes its sound along)
        if let Play::Playing(v) = &self.play {
            v.stop();
        }
    }
}

/// a sound at a point (play_at)
struct Shot {
    voice: Voice,
    at: Vec3,
    volume: f32,
    att: Attenuation,
}

/// what an update did
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioStats {
    /// emitters looked at, voices started, voices that ended, sounds at points still playing, voices with a gain
    /// above 0, starts the mixer refused
    pub emitters: u32,
    pub started: u32,
    pub ended: u32,
    pub shots: u32,
    pub audible: u32,
    pub refused: u32,
}

/// the listener and the voices of a game's sounds (see the module header)
pub struct SpatialAudio {
    listener: View,
    /// every gain times this (0 ..= 1)
    pub master: f32,
    shots: Vec<Shot>,
}

impl Default for SpatialAudio {
    fn default() -> Self {
        SpatialAudio::new()
    }
}

impl SpatialAudio {
    /// a listener at the origin looking down -Z
    pub fn new() -> SpatialAudio {
        SpatialAudio { listener: View { eye: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, -1.0), up: Vec3::Y }, master: 1.0, shots: Vec::new() }
    }

    /// where the listener is and looks (the camera's View)
    pub fn set_listener(&mut self, v: &View) {
        self.listener = *v;
    }

    pub fn listener(&self) -> &View {
        &self.listener
    }

    fn gains_at(&self, at: Vec3, volume: f32, att: &Attenuation) -> Gains {
        let master = if self.master.is_nan() { 0.0 } else { self.master.clamp(0.0, 1.0) };
        gains(&self.listener, at, volume as f64 * master as f64, att)
    }

    /// a sound at a point, once (a shot, a hit): its gains follow the listener until it ends
    pub fn play_at(&mut self, sound: &Sound, at: Vec3, volume: f32, att: Attenuation, mixer: &mut dyn Mixer) -> Result<(), AudioError> {
        let g = self.gains_at(at, volume, &att);
        let voice = mixer.play(sound, g.left, g.right)?;
        self.shots.push(Shot { voice, at, volume, att });
        Ok(())
    }

    /// once a frame, after the listener is set: the emitters of `world` (their voices started, their gains at the
    /// poses interpolated at `alpha`), the sounds at points -> what it did
    pub fn update(&mut self, world: &mut World, alpha: f64, mixer: &mut dyn Mixer) -> AudioStats {
        let mut st = AudioStats::default();
        // (the gains need &self while the column is borrowed: a copy of what they read)
        let me = SpatialAudio { listener: self.listener, master: self.master, shots: Vec::new() };
        if world.column::<Emitter>().is_some_and(|c| !c.entities().is_empty()) {
            let (emitters, poses) = world.pair_mut::<Emitter, Pose>();
            for (e, em) in emitters.iter_mut() {
                let em = em.untracked();
                st.emitters += 1;
                let at = poses.get(e).map_or(Vec3::ZERO, |p| p.interpolate(alpha).pos);
                let g = me.gains_at(at, em.volume, &em.att);
                if let Play::Playing(v) = &em.play {
                    if !v.playing() {
                        st.ended += 1;
                        em.play = if em.looping { Play::Waiting } else { Play::Done };
                    }
                }
                match &em.play {
                    Play::Waiting => match mixer.play(&em.sound, g.left, g.right) {
                        Ok(v) => {
                            em.play = Play::Playing(v);
                            st.started += 1;
                        }
                        Err(_) => {
                            em.play = Play::Done;
                            st.refused += 1;
                        }
                    },
                    Play::Playing(v) => {
                        if em.last.is_none_or(|l| (l.left, l.right) != (g.left, g.right)) {
                            v.set_gains(g.left, g.right);
                        }
                    }
                    Play::Done => {}
                }
                if matches!(em.play, Play::Playing(_)) {
                    em.last = Some(g);
                    if g.left > 0.0 || g.right > 0.0 {
                        st.audible += 1;
                    }
                }
            }
        }
        // the sounds at points
        let mut k = 0;
        while k < self.shots.len() {
            let s = &self.shots[k];
            if !s.voice.playing() {
                self.shots.swap_remove(k);
                st.ended += 1;
                continue;
            }
            let g = me.gains_at(s.at, s.volume, &s.att);
            s.voice.set_gains(g.left, g.right);
            if g.left > 0.0 || g.right > 0.0 {
                st.audible += 1;
            }
            k += 1;
        }
        st.shots = self.shots.len() as u32;
        st
    }

    /// every sound at a point ends now (the emitters keep theirs: Emitter::stop)
    pub fn stop_all(&mut self) {
        for s in self.shots.drain(..) {
            s.voice.stop();
        }
    }
}
