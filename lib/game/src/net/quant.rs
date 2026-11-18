// quant -- quantisation: the netcode keeps what it predicts and sends on fixed grids, so the server's
// state, what it sends and what a client rebuilds are the same integers
//
//   POS_STEPS        steps a metre (1024: 0.98 mm, exact in f64 because it is a power of two)
//   pos_q / pos_f    a coordinate to its step and back (round half away from 0); vec_q / vec_f for three
//   ANGLE_STEPS      a turn in 65 536 steps (0.0055 degrees); angle_q (wraps), angle_f in (-pi, pi]
//   pitch_q / pitch_f  a pitch in i16 steps of the same size (+-90 degrees fit)
//   axis_q / axis_f  an axis value -1 ..= 1 as -127 ..= 127
//   rot_q / rot_f    a rotation by its smallest three components (which one is left out, 2 bits, and three
//                    of ROT_BITS each): at most about 0.07 degrees at ROT_BITS = 12
//   round_i64        rounding without std (core has no f64::round)
//
// A grid of 2^-10 m reaches +-2 097 km in an i32.  Whatever a client predicts (the movement state of its
// figure) stays on the grid on both sides after every step (docs/NETCODE.md, "Bitgleich"), so the state a
// snapshot carries is the server's state exactly, not an approximation of it.

use physics::math::sqrt;
use physics::{Quat, Vec3};

/// steps a metre of the position grid
pub const POS_STEPS: f64 = 1024.0;
/// steps a full turn of an angle
pub const ANGLE_STEPS: f64 = 65536.0;
/// bits of each of the three components of rot_q
pub const ROT_BITS: u32 = 12;

/// x rounded to the nearest whole number, halves away from 0 (saturating at the ends of i64; NaN is 0)
pub fn round_i64(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let t = x as i64;
    let d = x - t as f64;
    if d >= 0.5 {
        t.saturating_add(1)
    } else if d <= -0.5 {
        t.saturating_sub(1)
    } else {
        t
    }
}

/// a coordinate in steps of POS_STEPS (saturating at the ends of i32)
pub fn pos_q(x: f64) -> i32 {
    round_i64(x * POS_STEPS).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// the coordinate of a step
pub fn pos_f(q: i32) -> f64 {
    q as f64 / POS_STEPS
}

/// a vector in steps
pub fn vec_q(v: Vec3) -> [i32; 3] {
    [pos_q(v.x), pos_q(v.y), pos_q(v.z)]
}

/// the vector of steps
pub fn vec_f(q: [i32; 3]) -> Vec3 {
    Vec3::new(pos_f(q[0]), pos_f(q[1]), pos_f(q[2]))
}

/// an angle in radians as a step of ANGLE_STEPS a turn (any angle: it wraps)
pub fn angle_q(a: f64) -> u16 {
    if !a.is_finite() {
        return 0;
    }
    let tau = core::f64::consts::TAU;
    let turns = a / tau;
    let frac = turns - turns as i64 as f64; // (-1, 1)
    round_i64(frac * ANGLE_STEPS) as i32 as u16
}

/// the angle of a step in (-pi, pi]
pub fn angle_f(q: u16) -> f64 {
    let s = q as i16 as f64;
    let a = s * (core::f64::consts::TAU / ANGLE_STEPS);
    if q == 0x8000 { core::f64::consts::PI } else { a }
}

/// a pitch in radians as an i16 step of the same size as angle_q (clamped to +-pi/2)
pub fn pitch_q(p: f64) -> i16 {
    if !p.is_finite() {
        return 0;
    }
    let half = core::f64::consts::FRAC_PI_2;
    let p = p.clamp(-half, half);
    round_i64(p * (ANGLE_STEPS / core::f64::consts::TAU)) as i16
}

/// the pitch of a step
pub fn pitch_f(q: i16) -> f64 {
    q as f64 * (core::f64::consts::TAU / ANGLE_STEPS)
}

/// an axis value -1 ..= 1 as -127 ..= 127
pub fn axis_q(v: f64) -> i8 {
    if !v.is_finite() {
        return 0;
    }
    round_i64(v.clamp(-1.0, 1.0) * 127.0) as i8
}

/// the axis value of a step (-128 counts as -127)
pub fn axis_f(q: i8) -> f64 {
    (q.max(-127)) as f64 / 127.0
}

/// a rotation as (index of the largest component, its three others in ROT_BITS each, signed so that the
/// largest is positive); a quaternion and its negative give the same
pub fn rot_q(q: Quat) -> [i32; 4] {
    let c = [q.x, q.y, q.z, q.w];
    let mut big = 0;
    for i in 1..4 {
        if c[i].abs() > c[big].abs() {
            big = i;
        }
    }
    let sign = if c[big] < 0.0 { -1.0 } else { 1.0 };
    let scale = ((1i64 << (ROT_BITS - 1)) - 1) as f64 / core::f64::consts::FRAC_1_SQRT_2;
    let lim = (1i64 << (ROT_BITS - 1)) - 1;
    let mut out = [big as i32, 0, 0, 0];
    let mut k = 1;
    for (i, v) in c.iter().enumerate() {
        if i != big {
            out[k] = round_i64(v * sign * scale).clamp(-lim, lim) as i32;
            k += 1;
        }
    }
    out
}

/// the rotation of rot_q (the left-out component from the unit length); a bad index gives the identity
pub fn rot_f(r: [i32; 4]) -> Quat {
    let big = r[0];
    if !(0..4).contains(&big) {
        return Quat::IDENTITY;
    }
    let scale = core::f64::consts::FRAC_1_SQRT_2 / ((1i64 << (ROT_BITS - 1)) - 1) as f64;
    let a = r[1] as f64 * scale;
    let b = r[2] as f64 * scale;
    let c = r[3] as f64 * scale;
    let rest = 1.0 - a * a - b * b - c * c;
    let d = if rest > 0.0 { sqrt(rest) } else { 0.0 };
    let q = match big {
        0 => Quat::new(d, a, b, c),
        1 => Quat::new(a, d, b, c),
        2 => Quat::new(a, b, d, c),
        _ => Quat::new(a, b, c, d),
    };
    q.normalize()
}
