// pose -- where an entity is: position (f64), rotation, scale; now and at the end of the tick before,
// so that a frame between two ticks shows a state between the two
//
//   Transform         pos (Vec3, f64 as in render3d), rot (Quat, x y z w), scale ([f32; 3] as in
//                     render3d); lerp: positions and scales straight, rotations by nlerp
//   Pose              now (what the systems write) and prev (the state when the tick began);
//                     interpolate(alpha) for a frame, teleport for a jump without the in-between
//   nlerp(a, b, t)    the normalised straight blend of two rotations on the shorter way
//
// The world settles the poses at every tick (World::begin_tick -> settle): a pose written in the tick
// before gets prev = now.  Only the poses with a newer version than the mark of the tick before are
// touched, so a world of standing things costs a scan of their versions.  Everything is + - * / and the
// correctly rounded square root of lib/physics: the same bits on every target.

use physics::math::sqrt;
use physics::{Quat, Vec3};

use crate::sparse::SparseSet;

/// a placement: rotate (rot), scale (scale, along the rotated axes), move (pos)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// the position in the world (metres)
    pub pos: Vec3,
    /// the rotation, a unit quaternion (x, y, z, w)
    pub rot: Quat,
    /// the scale per axis (render3d's; negative values mirror)
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

impl Transform {
    /// at the origin, not rotated, scale 1
    pub const IDENTITY: Transform = Transform { pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: [1.0; 3] };

    /// at pos, not rotated, scale 1
    pub const fn at(pos: Vec3) -> Transform {
        Transform { pos, rot: Quat::IDENTITY, scale: [1.0; 3] }
    }

    /// at pos with rotation rot, scale 1
    pub const fn new(pos: Vec3, rot: Quat) -> Transform {
        Transform { pos, rot, scale: [1.0; 3] }
    }

    /// the same with another scale
    pub const fn scaled(self, scale: [f32; 3]) -> Transform {
        Transform { scale, ..self }
    }

    /// the state t of the way from self (0) to b (1): positions and scales straight, the rotation by
    /// nlerp; t = 0 and t = 1 give self and b exactly
    pub fn lerp(&self, b: &Transform, t: f64) -> Transform {
        if t <= 0.0 {
            return *self;
        }
        if t >= 1.0 {
            return *b;
        }
        let s = |x: f32, y: f32| (x as f64 + (y as f64 - x as f64) * t) as f32;
        Transform {
            pos: Vec3::new(self.pos.x + (b.pos.x - self.pos.x) * t, self.pos.y + (b.pos.y - self.pos.y) * t, self.pos.z + (b.pos.z - self.pos.z) * t),
            rot: nlerp(self.rot, b.rot, t),
            scale: [s(self.scale[0], b.scale[0]), s(self.scale[1], b.scale[1]), s(self.scale[2], b.scale[2])],
        }
    }
}

/// the straight blend of two unit quaternions, normalised, on the shorter of the two ways (b's sign
/// flipped when a . b < 0); t = 0 and 1 give a and b (or -b) exactly up to the normalisation
pub fn nlerp(a: Quat, b: Quat, t: f64) -> Quat {
    let d = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    let b = if d < 0.0 { Quat::new(-b.x, -b.y, -b.z, -b.w) } else { b };
    let q = Quat::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t, a.w + (b.w - a.w) * t);
    let l = sqrt(q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w);
    if l > 0.0 { Quat::new(q.x / l, q.y / l, q.z / l, q.w / l) } else { a }
}

/// where an entity is now and where it was when the tick began: the component the world interpolates
/// (World::begin_tick) and the bridge to render3d draws
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pose {
    /// the state the systems write
    pub now: Transform,
    /// the state at the start of the tick (World::begin_tick sets it from now)
    pub prev: Transform,
}

impl Pose {
    /// standing at t (prev = now)
    pub const fn new(t: Transform) -> Pose {
        Pose { now: t, prev: t }
    }

    /// standing at pos, not rotated
    pub const fn at(pos: Vec3) -> Pose {
        Pose::new(Transform::at(pos))
    }

    /// a jump to t without the frames in between (prev = now = t)
    pub fn teleport(&mut self, t: Transform) {
        self.now = t;
        self.prev = t;
    }

    /// now differs from prev: frames show it between the two
    pub fn moving(&self) -> bool {
        self.now != self.prev
    }

    /// the state a frame shows at alpha (FixedTick::alpha: 0 at the start of the tick, towards 1 at its
    /// end): prev.lerp(now, alpha)
    pub fn interpolate(&self, alpha: f64) -> Transform {
        if self.now == self.prev { self.now } else { self.prev.lerp(&self.now, alpha) }
    }
}

/// (World::begin_tick) every pose written after version `mark` gets prev = now, stamped with a new
/// version (what a frame shows changes once more: from the blend to now)
pub(crate) fn settle(poses: &mut SparseSet<Pose>, mark: u64) {
    // (a pose with prev != now was written since the last settle; one written with prev == now already,
    // such as a new one, keeps its version, newer than `mark`: the bridge sees it anyway)
    for (_, mut p) in poses.changed_mut_since(mark) {
        if p.prev != p.now {
            let now = p.now;
            p.prev = now;
        }
    }
}
