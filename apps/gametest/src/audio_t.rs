// audio_t -- (G7b part 2) the group "audio": lib/game's spatial audio against closed forms (the attenuation, the
// equal-power pan) and, sample for sample, through plat::audio's own mixer without a device (plat::audio::MixTest:
// the code play() mixes with).  The signals are made here (ramps, sines); no speaker, no file.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use game::audio::{gains, Attenuation, AudioStats, Emitter, Rolloff, SpatialAudio, PAN_RADIUS};
use game::{Pose, Vec3, View, World};
use plat::audio::{MixTest, Sound};

use crate::Ctx;

pub fn run(c: &mut Ctx) {
    attenuation(c);
    pan(c);
    mixing(c);
    lifecycle(c);
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn attenuation(c: &mut Ctx) {
    let inv = Attenuation::DEFAULT; // near 1, far 50, rolloff 1
    let cases = [(0.0, 1.0), (0.5, 1.0), (1.0, 1.0), (2.0, 0.5), (4.0, 0.25), (10.0, 0.1), (50.0, 0.02), (500.0, 0.02)];
    for (d, want) in cases {
        let g = inv.gain(d);
        c.check(close(g, want, 1e-15), || format!("inverse, near 1, far 50: gain at {d} m = {g}, not {want}"));
    }
    let inv2 = Attenuation { near: 2.0, far: 20.0, rolloff: Rolloff::Inverse(0.5) };
    let g = inv2.gain(6.0);
    c.check(close(g, 2.0 / (2.0 + 0.5 * 4.0), 1e-15), || format!("inverse, rolloff 0.5: {g}"));
    let lin = Attenuation::linear(2.0, 10.0);
    for (d, want) in [(1.0, 1.0), (2.0, 1.0), (6.0, 0.5), (8.0, 0.25), (10.0, 0.0), (12.0, 0.0)] {
        let g = lin.gain(d);
        c.check(close(g, want, 1e-15), || format!("linear 2 .. 10: gain at {d} m = {g}, not {want}"));
    }
    c.check(lin.gain(f64::NAN) == 0.0 && Attenuation { near: 0.0, far: -1.0, rolloff: Rolloff::Linear }.gain(5.0) == 0.0, || String::from("NaN and nonsense limits"));
    // monotonic: never louder further away
    let mut ok = true;
    for a in [inv, inv2, lin] {
        let mut last = 2.0;
        for k in 0..2000 {
            let g = a.gain(k as f64 * 0.05);
            ok &= g <= last && (0.0..=1.0).contains(&g);
            last = g;
        }
    }
    c.check(ok, || String::from("the gain falls (or stays) with the distance, within 0 ..= 1"));
}

/// a listener at the origin looking down -Z (right is +X)
fn ahead() -> View {
    View { eye: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, -1.0), up: Vec3::Y }
}

fn pan(c: &mut Ctx) {
    let att = Attenuation::linear(1.0, 11.0);
    let l = ahead();
    let h = core::f64::consts::FRAC_1_SQRT_2;
    // (where, volume) -> (pan, left / gain, right / gain, gain)
    let cases: [(Vec3, f64, f64, f64, f64, f64); 7] = [
        (Vec3::new(0.0, 0.0, -5.0), 1.0, 0.0, h, h, 0.6),
        (Vec3::new(0.0, 0.0, 5.0), 1.0, 0.0, h, h, 0.6),        // behind: as in front (stereo)
        (Vec3::new(5.0, 0.0, 0.0), 1.0, 1.0, 0.0, 1.0, 0.6),
        (Vec3::new(-5.0, 0.0, 0.0), 0.5, -1.0, 1.0, 0.0, 0.3),
        (Vec3::new(3.0, 0.0, -4.0), 1.0, 0.6, 0.309_016_994_374_947_4, 0.951_056_516_295_153_6, 0.6),
        (Vec3::new(0.0, 5.0, 0.0), 1.0, 0.0, h, h, 0.6),        // above: the middle
        (Vec3::new(0.25, 0.0, 0.0), 1.0, 0.25 / PAN_RADIUS, 0.382_683_432_365_089_8, 0.923_879_532_511_286_7, 1.0), // near the head: drawn to the middle
    ];
    for (at, vol, p, cl, cr, g) in cases {
        let x = gains(&l, at, vol, &att);
        let ok = close(x.pan, p, 1e-12) && close(x.gain, g, 1e-12) && close(x.left as f64, g * cl, 1e-7) && close(x.right as f64, g * cr, 1e-7)
            && close(x.left as f64 * x.left as f64 + x.right as f64 * x.right as f64, g * g, 1e-6);
        c.check(ok, || format!("gains at {at:?}, volume {vol}: {x:?} (want pan {p}, {} / {})", g * cl, g * cr));
    }
    // the listener turned about: right and left swap
    let back = View { eye: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, 1.0), up: Vec3::Y };
    let (a, b) = (gains(&l, Vec3::new(4.0, 0.0, -3.0), 1.0, &att), gains(&back, Vec3::new(4.0, 0.0, -3.0), 1.0, &att));
    c.check(a.left == b.right && a.right == b.left && close(a.pan, -b.pan, 1e-15), || format!("turned about: {a:?} / {b:?}"));
    // the listener moved: what counts is the source relative to the eye
    let moved = View { eye: Vec3::new(10.0, 2.0, -3.0), ..ahead() };
    let m = gains(&moved, Vec3::new(13.0, 2.0, -7.0), 1.0, &att);
    let r = gains(&l, Vec3::new(3.0, 0.0, -4.0), 1.0, &att);
    c.check(m == r, || format!("relative to the eye: {m:?} / {r:?}"));
    c.check(gains(&l, Vec3::ZERO, 1.0, &att).pan == 0.0 && gains(&l, Vec3::new(1.0, 0.0, 0.0), f64::NAN, &att).gain == 0.0, || String::from("at the eye: the middle; NaN volume: silent"));
}

/// a mono ramp of n samples (distinct values, exact in f32)
fn ramp(n: usize) -> Sound {
    Sound::new(48_000, 1, (0..n).map(|k| ((k % 1000) as f32 - 500.0) / 1024.0).collect()).expect("sound")
}

/// out against the expected frames: frames [from, from + n) of the sounds `parts` (samples, left gain, right
/// gain) summed in this order and clamped, as the mixer does -> frames that differ
fn expect(out: &[f32], parts: &[(&[f32], usize, f32, f32)]) -> usize {
    let mut bad = 0;
    for (i, o) in out.chunks_exact(2).enumerate() {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for &(s, from, gl, gr) in parts {
            let v = s.get(from + i).copied().unwrap_or(0.0);
            l += v * gl;
            r += v * gr;
        }
        if o[0] != l.clamp(-1.0, 1.0) || o[1] != r.clamp(-1.0, 1.0) {
            bad += 1;
        }
    }
    bad
}

fn mixing(c: &mut Ctx) {
    let snd = ramp(4800);
    let s = snd.samples().to_vec();
    let mut mix = MixTest::new();
    let mut sa = SpatialAudio::new();
    sa.set_listener(&ahead());
    let att = Attenuation::linear(1.0, 11.0);
    let mut w = World::new();
    let e = w.spawn_with((Pose::at(Vec3::new(3.0, 0.0, -4.0)), Emitter::new(snd.clone()).with_attenuation(att)));
    let st = sa.update(&mut w, 1.0, &mut mix);
    c.check(st == AudioStats { emitters: 1, started: 1, audible: 1, ..AudioStats::default() } && mix.voices() == 1, || format!("started: {st:?}"));
    let g = gains(&ahead(), Vec3::new(3.0, 0.0, -4.0), 1.0, &att);
    let out = mix.render(480);
    let bad = expect(&out, &[(&s, 0, g.left, g.right)]);
    c.check(bad == 0, || format!("the first buffer: {bad} of 480 frames differ (gains {} / {})", g.left, g.right));
    // the source moves to the other side: the next buffer has the new gains
    if let Some(mut p) = w.get_mut::<Pose>(e) {
        p.teleport(game::Transform::at(Vec3::new(-3.0, 0.0, -4.0)));
    }
    let _ = sa.update(&mut w, 1.0, &mut mix);
    let g2 = gains(&ahead(), Vec3::new(-3.0, 0.0, -4.0), 1.0, &att);
    let out = mix.render(480);
    let bad = expect(&out, &[(&s, 480, g2.left, g2.right)]);
    c.check(bad == 0 && g2.left == g.right && g2.right == g.left, || format!("moved to the left: {bad} frames differ"));
    // the listener turns about: the sides swap back
    sa.set_listener(&View { eye: Vec3::ZERO, forward: Vec3::new(0.0, 0.0, 1.0), up: Vec3::Y });
    let _ = sa.update(&mut w, 1.0, &mut mix);
    let out = mix.render(480);
    let bad = expect(&out, &[(&s, 960, g.left, g.right)]);
    c.check(bad == 0, || format!("the listener turned: {bad} frames differ"));
    sa.set_listener(&ahead());
    // beyond far: silent, still playing
    if let Some(mut p) = w.get_mut::<Pose>(e) {
        p.teleport(game::Transform::at(Vec3::new(0.0, 0.0, -20.0)));
    }
    let st = sa.update(&mut w, 1.0, &mut mix);
    let out = mix.render(480);
    c.check(out.iter().all(|&v| v == 0.0) && st.audible == 0 && st.emitters == 1, || format!("beyond far: silent ({st:?})"));
    // a second emitter and the master volume: the voices summed in their order
    if let Some(mut p) = w.get_mut::<Pose>(e) {
        p.teleport(game::Transform::at(Vec3::new(1.0, 0.0, -1.0)));
    }
    let tone: Vec<f32> = (0..4800).map(|k| physics::math::sin(k as f64 * 0.05) as f32 * 0.5).collect();
    let snd2 = Sound::new(48_000, 1, tone.clone()).expect("sound");
    let e2 = w.spawn_with((Pose::at(Vec3::new(-2.0, 1.0, 2.0)), Emitter::new(snd2).with_attenuation(Attenuation::DEFAULT).with_volume(0.8)));
    sa.master = 0.5;
    let st = sa.update(&mut w, 1.0, &mut mix);
    let ga = gains(&ahead(), Vec3::new(1.0, 0.0, -1.0), 0.5, &att);
    // (the volume and the master are f32: their product as update makes it)
    let gb = gains(&ahead(), Vec3::new(-2.0, 1.0, 2.0), 0.8f32 as f64 * 0.5f32 as f64, &Attenuation::DEFAULT);
    let out = mix.render(480);
    let bad = expect(&out, &[(&s, 1920, ga.left, ga.right), (&tone, 0, gb.left, gb.right)]);
    c.check(bad == 0 && st.started == 1 && st.emitters == 2 && mix.voices() == 2, || format!("two voices, master 0.5: {bad} frames differ ({st:?})"));
    sa.master = 1.0;
    // the entity goes: its sound stops
    w.despawn(e2);
    let _ = sa.update(&mut w, 1.0, &mut mix);
    let g3 = gains(&ahead(), Vec3::new(1.0, 0.0, -1.0), 1.0, &att);
    let out = mix.render(480);
    let bad = expect(&out, &[(&s, 2400, g3.left, g3.right)]);
    c.check(bad == 0 && mix.voices() == 1, || format!("despawned: its voice gone ({} voices, {bad} frames differ)", mix.voices()));
    // a stereo sound keeps its channels
    let st2: Vec<f32> = (0..960).flat_map(|k| [k as f32 / 2048.0, -(k as f32) / 4096.0]).collect();
    let mut mix2 = MixTest::new();
    let mut w2 = World::new();
    w2.spawn_with((Pose::at(Vec3::new(2.0, 0.0, -2.0)), Emitter::new(Sound::new(48_000, 2, st2.clone()).expect("sound")).with_attenuation(att)));
    let _ = sa.update(&mut w2, 1.0, &mut mix2);
    let gs = gains(&ahead(), Vec3::new(2.0, 0.0, -2.0), 1.0, &att);
    let out = mix2.render(960);
    let bad = out.chunks_exact(2).enumerate().filter(|(i, o)| o[0] != st2[2 * i] * gs.left || o[1] != st2[2 * i + 1] * gs.right).count();
    c.check(bad == 0, || format!("a stereo sound: {bad} frames differ"));
    // a sound at another rate goes through the mixer's resampler: its level after the gains
    let t24: Vec<f32> = (0..24_000).map(|k| physics::math::sin(k as f64 * core::f64::consts::TAU * 440.0 / 24_000.0) as f32 * 0.5).collect();
    let mut mix3 = MixTest::new();
    let mut w3 = World::new();
    w3.spawn_with((Pose::at(Vec3::new(0.0, 0.0, -3.0)), Emitter::new(Sound::new(24_000, 1, t24).expect("sound")).with_attenuation(Attenuation::linear(1.0, 5.0))));
    let _ = sa.update(&mut w3, 1.0, &mut mix3);
    let out = mix3.render(24_000);
    let gl = gains(&ahead(), Vec3::new(0.0, 0.0, -3.0), 1.0, &Attenuation::linear(1.0, 5.0)).left as f64;
    let lv = plat::audio::level(&out[4800..], 2, 0, 48_000, 440.0) as f64;
    c.check(close(lv, 0.5 * gl, 0.01), || format!("24 kHz through the resampler: level {lv}, want {}", 0.5 * gl));
}

fn lifecycle(c: &mut Ctx) {
    let snd = ramp(1000);
    let s = snd.samples().to_vec();
    let mut mix = MixTest::new();
    let mut sa = SpatialAudio::new();
    sa.set_listener(&ahead());
    let att = Attenuation::linear(1.0, 11.0);
    let mut w = World::new();
    let once = w.spawn_with((Pose::at(Vec3::new(0.0, 0.0, -2.0)), Emitter::new(snd.clone()).with_attenuation(att)));
    let looped = w.spawn_with((Pose::at(Vec3::new(0.0, 0.0, -2.0)), Emitter::new(snd.clone()).with_attenuation(att).looping()));
    let _ = sa.update(&mut w, 1.0, &mut mix);
    let _ = mix.render(1000); // both end within this buffer
    let _ = mix.render(16); // (the mixer says so with the next buffer)
    let st = sa.update(&mut w, 1.0, &mut mix);
    let fin = w.get::<Emitter>(once).is_some_and(|e| e.finished()) && w.get::<Emitter>(looped).is_some_and(|e| e.playing());
    c.check(st.ended == 2 && st.started == 1 && fin, || format!("ended: the one-shot finished, the loop again ({st:?})"));
    let g = gains(&ahead(), Vec3::new(0.0, 0.0, -2.0), 1.0, &att);
    let out = mix.render(500);
    let bad = expect(&out, &[(&s, 0, g.left, g.right)]);
    c.check(bad == 0, || format!("the loop from its start: {bad} frames differ"));
    // restart and stop
    if let Some(mut e) = w.get_mut::<Emitter>(once) {
        e.restart();
    }
    let st = sa.update(&mut w, 1.0, &mut mix);
    c.check(st.started == 1, || format!("restarted: {st:?}"));
    if let Some(mut e) = w.get_mut::<Emitter>(looped) {
        e.stop();
    }
    let _ = mix.render(16);
    let _ = sa.update(&mut w, 1.0, &mut mix);
    let stopped = w.get::<Emitter>(looped).is_some_and(|e| e.finished() && !e.playing());
    c.check(stopped, || String::from("stopped: finished, not playing"));
    // a sound at a point: its gains follow the listener, it ends
    let mut mix2 = MixTest::new();
    let mut empty = World::new();
    let at = Vec3::new(4.0, 0.0, 3.0);
    let r = sa.play_at(&snd, at, 1.0, att, &mut mix2);
    let g0 = gains(&ahead(), at, 1.0, &att);
    let out = mix2.render(400);
    let bad = expect(&out, &[(&s, 0, g0.left, g0.right)]);
    c.check(r.is_ok() && bad == 0, || format!("play_at: {bad} frames differ"));
    let lis = View { eye: Vec3::new(8.0, 0.0, 3.0), ..ahead() };
    sa.set_listener(&lis);
    let st = sa.update(&mut empty, 1.0, &mut mix2);
    let g1 = gains(&lis, at, 1.0, &att);
    let out = mix2.render(400);
    let bad = expect(&out, &[(&s, 400, g1.left, g1.right)]);
    c.check(bad == 0 && st.shots == 1 && g1.pan < 0.0, || format!("play_at follows the listener: {bad} frames differ ({st:?})"));
    let _ = mix2.render(400);
    let _ = mix2.render(16);
    let st = sa.update(&mut empty, 1.0, &mut mix2);
    c.check(st.shots == 0 && st.ended == 1, || format!("play_at ended: {st:?}"));
    // the same run twice: the same samples (no clock, no threads)
    let run = || {
        let mut m = MixTest::new();
        let mut a = SpatialAudio::new();
        let mut w = World::new();
        let e = w.spawn_with((Pose::at(Vec3::new(1.0, 0.0, -1.0)), Emitter::new(ramp(3000)).with_attenuation(Attenuation::DEFAULT).looping()));
        let mut all = Vec::new();
        for k in 0..20 {
            a.set_listener(&View { eye: Vec3::new(k as f64 * 0.1, 0.0, 0.0), ..ahead() });
            if let Some(mut p) = w.get_mut::<Pose>(e) {
                p.teleport(game::Transform::at(Vec3::new(1.0 - k as f64 * 0.2, 0.0, -1.0 - k as f64 * 0.1)));
            }
            let _ = a.update(&mut w, 1.0, &mut m);
            all.extend(m.render(256));
        }
        all.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x100_0000_01b3))
    };
    let (h1, h2) = (run(), run());
    c.check(h1 == h2, || format!("the same run twice: {h1:016x} / {h2:016x}"));
}
