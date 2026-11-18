// camera -- camera controllers in f64: first person (yaw and pitch, the pitch held within a limit),
// follow (an orbit around a target at a distance, pulled in front of what is in the way) and free fly
//
//   View              eye, forward, up (unit vectors at right angles): what a renderer needs besides the
//                     lens (the bridge makes a render3d::Camera of it)
//   FirstPerson       eye, yaw, pitch; look(dx, dy) from Motion actions, walk_dir for moving on the
//                     ground, view()
//   Follow            target, yaw, pitch, distance (min..max); look, zoom, eye, view, view_in (a sphere
//                     cast through a physics World from the target to the eye)
//   FreeFly           eye, yaw, pitch, speed; look, fly(forward, right, up, dt), view
//   basis(yaw, pitch) (forward, right, up) of a turn; wrap_angle into (-pi, pi]
//
// Conventions as lib/physics (Quat::from_yaw_pitch): right-handed, Y up; yaw 0 and pitch 0 look along -Z;
// yaw turns about +Y (positive: to the left), pitch about the turned +X (positive: up).  look(dx, dy)
// takes what the mouse did, right and down positive (the values of Motion actions), and turns right and
// down: yaw -= dx, pitch -= dy.  sin and cos are physics::math's (rt's portable ones): the same bits on
// every target, as the server's simulation needs.

use physics::math::{atan2, clamp, sincos, sqrt};
use physics::{Filter, Iso, Quat, Shape, Vec3, World as PhysicsWorld};

/// the pitch limit unless set: 89 degrees (straight up or down would leave the yaw undefined)
pub const PITCH_LIMIT: f64 = 89.0 * core::f64::consts::PI / 180.0;

/// what a camera sees from: the eye, the unit vectors forward and up (at right angles)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// the eye's position (metres, f64)
    pub eye: Vec3,
    /// where it looks
    pub forward: Vec3,
    /// up on the screen
    pub up: Vec3,
}

impl View {
    /// from eye to target, `up` saying which way is up (None: target at the eye, or up along the line)
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Option<View> {
        let f = (target - eye).try_normalize()?;
        let r = f.cross(up).try_normalize()?;
        Some(View { eye, forward: f, up: r.cross(f) })
    }

    /// right on the screen (forward x up)
    pub fn right(&self) -> Vec3 {
        self.forward.cross(self.up)
    }
}

/// an angle in (-pi, pi]
pub fn wrap_angle(a: f64) -> f64 {
    let tau = core::f64::consts::TAU;
    if !a.is_finite() {
        return 0.0;
    }
    let mut r = a - tau * round(a / tau);
    if r <= -core::f64::consts::PI {
        r += tau;
    } else if r > core::f64::consts::PI {
        r -= tau;
    }
    r
}

/// x rounded to the nearest whole number, halves away from 0 (core has no f64::round without std)
pub(crate) fn round(x: f64) -> f64 {
    if !(x.abs() < 4.0e15) {
        return x;
    }
    let t = x as i64 as f64;
    let d = x - t;
    if d >= 0.5 {
        t + 1.0
    } else if d <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// (forward, right, up) of a camera turned by yaw and pitch (see the module header): the columns of
/// Quat::from_yaw_pitch(yaw, pitch) applied to -Z, +X and +Y
pub fn basis(yaw: f64, pitch: f64) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = sincos(yaw);
    let (sp, cp) = sincos(pitch);
    let forward = Vec3::new(-sy * cp, sp, -cy * cp);
    let right = Vec3::new(cy, 0.0, -sy);
    let up = Vec3::new(sy * sp, cp, cy * sp);
    (forward, right, up)
}

/// yaw and pitch of a direction (the inverse of basis' forward; None for a zero vector)
pub fn yaw_pitch_of(dir: Vec3) -> Option<(f64, f64)> {
    let d = dir.try_normalize()?;
    let flat = sqrt(d.x * d.x + d.z * d.z);
    Some((atan2(-d.x, -d.z), atan2(d.y, flat)))
}

/// a first-person camera: at the eye, turned by yaw and pitch
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstPerson {
    /// the eye (a figure's position plus its eye height)
    pub eye: Vec3,
    /// the turn about +Y (radians, positive to the left), kept in (-pi, pi]
    pub yaw: f64,
    /// the turn up and down (radians, positive up), kept within pitch_limit
    pub pitch: f64,
    /// the largest pitch up and down (PITCH_LIMIT unless set)
    pub pitch_limit: f64,
}

impl FirstPerson {
    /// at eye, looking along -Z
    pub fn new(eye: Vec3) -> FirstPerson {
        FirstPerson { eye, yaw: 0.0, pitch: 0.0, pitch_limit: PITCH_LIMIT }
    }

    /// turn by what the mouse did (radians, right and down positive: the values of Motion actions)
    pub fn look(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.yaw = wrap_angle(self.yaw - dx);
            self.pitch = clamp(self.pitch - dy, -self.pitch_limit, self.pitch_limit);
        }
    }

    /// where it looks
    pub fn forward(&self) -> Vec3 {
        basis(self.yaw, self.pitch).0
    }

    /// the rotation of the camera (-Z forward, +Y up)
    pub fn rotation(&self) -> Quat {
        Quat::from_yaw_pitch(self.yaw, self.pitch)
    }

    /// a direction on the ground (y 0) from two Axis values: forward along the yaw, right across it; its
    /// length at most 1 (diagonal walking is not faster)
    pub fn walk_dir(&self, forward: f64, right: f64) -> Vec3 {
        walk_dir(self.yaw, forward, right)
    }

    /// the view from the eye
    pub fn view(&self) -> View {
        let (forward, _, up) = basis(self.yaw, self.pitch);
        View { eye: self.eye, forward, up }
    }
}

/// (FirstPerson::walk_dir) a direction on the ground from the yaw and two Axis values, length at most 1
pub fn walk_dir(yaw: f64, forward: f64, right: f64) -> Vec3 {
    let (sy, cy) = sincos(yaw);
    let v = Vec3::new(-sy * forward + cy * right, 0.0, -cy * forward - sy * right);
    let l = v.length();
    if l > 1.0 { v / l } else { v }
}

/// a third-person camera: an orbit around a target, at a distance
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    /// what it looks at (a figure's head)
    pub target: Vec3,
    /// the turn about +Y (radians), kept in (-pi, pi]
    pub yaw: f64,
    /// the turn up and down (positive: looking up, so the eye sinks), kept in pitch_min ..= pitch_max
    pub pitch: f64,
    /// the eye's distance from the target, kept in min_distance ..= max_distance
    pub distance: f64,
    /// the nearest the eye comes
    pub min_distance: f64,
    /// the farthest
    pub max_distance: f64,
    /// the lowest pitch (looking down most: the eye above the target)
    pub pitch_min: f64,
    /// the highest pitch
    pub pitch_max: f64,
}

impl Follow {
    /// around target at distance (kept in 0.5 ..= 50), slightly from above (pitch -0.3)
    pub fn new(target: Vec3, distance: f64) -> Follow {
        let mut f = Follow { target, yaw: 0.0, pitch: -0.3, distance, min_distance: 0.5, max_distance: 50.0, pitch_min: -1.4, pitch_max: 1.0 };
        f.distance = clamp(distance, f.min_distance, f.max_distance);
        f
    }

    /// turn around the target by what the mouse did (radians, right and down positive)
    pub fn look(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.yaw = wrap_angle(self.yaw - dx);
            self.pitch = clamp(self.pitch - dy, self.pitch_min, self.pitch_max);
        }
    }

    /// nearer (notches > 0, the wheel up) or farther: 10 % of the distance a notch
    pub fn zoom(&mut self, notches: f64) {
        if notches.is_finite() {
            let mut d = self.distance;
            let k = if notches > 0.0 { 0.9 } else { 1.0 / 0.9 };
            let mut n = notches.abs();
            while n >= 1.0 {
                d *= k;
                n -= 1.0;
            }
            d *= 1.0 + (k - 1.0) * n;
            self.distance = clamp(d, self.min_distance, self.max_distance);
        }
    }

    /// the eye: distance behind the target along the view
    pub fn eye(&self) -> Vec3 {
        self.target - basis(self.yaw, self.pitch).0 * self.distance
    }

    /// the view from the eye to the target
    pub fn view(&self) -> View {
        let (forward, _, up) = basis(self.yaw, self.pitch);
        View { eye: self.target - forward * self.distance, forward, up }
    }

    /// the view with the eye pulled in front of what is in the way: a sphere of `radius` cast from the
    /// target towards the eye through `world` (the figure's own body left out by `filter`) -> the view and
    /// the distance used (at least min_distance, nearer still when the target itself sits in something)
    pub fn view_in(&self, world: &PhysicsWorld, filter: &Filter, radius: f64) -> (View, f64) {
        let (forward, _, up) = basis(self.yaw, self.pitch);
        let motion = forward * -self.distance;
        let mut d = self.distance;
        if radius > 0.0 && d > 0.0 {
            if let Some((_, hit)) = world.cast(&Shape::sphere(radius), &Iso::at(self.target), motion, filter) {
                d = (self.distance * hit.fraction - radius * 0.1).max(0.0);
            }
        }
        (View { eye: self.target - forward * d, forward, up }, d)
    }
}

/// a free camera: flies where it looks
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FreeFly {
    /// the eye
    pub eye: Vec3,
    /// the turn about +Y (radians), kept in (-pi, pi]
    pub yaw: f64,
    /// the turn up and down, kept within pitch_limit
    pub pitch: f64,
    /// the largest pitch (PITCH_LIMIT unless set)
    pub pitch_limit: f64,
    /// metres a second at full input
    pub speed: f64,
}

impl FreeFly {
    /// at eye, looking along -Z, 10 m/s
    pub fn new(eye: Vec3) -> FreeFly {
        FreeFly { eye, yaw: 0.0, pitch: 0.0, pitch_limit: PITCH_LIMIT, speed: 10.0 }
    }

    /// turn by what the mouse did (radians, right and down positive)
    pub fn look(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.yaw = wrap_angle(self.yaw - dx);
            self.pitch = clamp(self.pitch - dy, -self.pitch_limit, self.pitch_limit);
        }
    }

    /// fly for dt seconds: forward along the view (pitch included), right across it, up along +Y; each
    /// input in -1 ..= 1, the three together at most speed
    pub fn fly(&mut self, forward: f64, right: f64, up: f64, dt: f64) {
        let (f, r, _) = basis(self.yaw, self.pitch);
        let v = f * forward + r * right + Vec3::Y * up;
        let l = v.length();
        let v = if l > 1.0 { v / l } else { v };
        if v.is_finite() && dt.is_finite() {
            self.eye += v * (self.speed * dt);
        }
    }

    /// the view from the eye
    pub fn view(&self) -> View {
        let (forward, _, up) = basis(self.yaw, self.pitch);
        View { eye: self.eye, forward, up }
    }
}
