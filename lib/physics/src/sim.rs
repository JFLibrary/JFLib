// sim -- rigid bodies: a world of static, kinematic and dynamic bodies that move under gravity, forces,
// contacts and joints, step by step, with the same bits on every target
//
// A step: kinematic targets become velocities; every awake body looks for others near it in the trees of
// the collision World (its box grown by how far it can move in this step), each pair gets its manifolds
// (manifold.rs) and the impulses of the last step's points that lie within 2 cm of the new ones; a sleeping
// body that an awake one would touch wakes with its island.  Then the substeps of the soft solver
// (solver.rs, joint.rs), restitution, the new poses (into the World, so queries and the character controller
// see them), a sweep of fast bodies against what does not move (they stop where they first touch it, so they
// do not pass through a wall between two steps), sleep: an island (bodies linked by contacts and joints) whose
// bodies all moved slower than `sleep_speed` for `time_to_sleep` sleeps until something touches it.
//
// Determinism: everything in slot order, pairs sorted, no hash containers, no threads; the same bits on every
// target as the queries (checked by physicstest's checksums).

use crate::joint::{Joint, JointError, JointId, JointKind, JointRt};
use crate::manifold::{self, Geom, MAX_POINTS};
use crate::mass::MassProps;
use crate::math::{Iso, Mat3, Quat, Vec3, max, min, sqrt};
use crate::query;
use crate::shape::Shape;
use crate::solver::{self, CPoint, Constraint, NONE, SBody, Soft};
use crate::world::{BodyId, World};
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

/// how a body moves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// never (a map, a wall); moved only by [`Simulation::set_iso`]
    Static,
    /// by its velocity, which only the caller sets (a door, a platform, a lift); pushes dynamic bodies, nothing
    /// pushes it
    Kinematic,
    /// by gravity, forces, contacts and joints
    Dynamic,
}

/// What [`Simulation::add`] makes a body from.  `RigidBody::dynamic(shape, iso)` and the `with_` methods
/// for the rest.
#[derive(Clone, Debug)]
pub struct RigidBody {
    /// how it moves
    pub kind: BodyKind,
    /// its shape (a mesh only for a static or kinematic body)
    pub shape: Shape,
    /// where the shape's frame is
    pub iso: Iso,
    /// m/s, of the centre of mass
    pub linear_velocity: Vec3,
    /// rad/s, world frame
    pub angular_velocity: Vec3,
    /// kg/m³ (1000: water), for the mass from the shape
    pub density: f64,
    /// the mass as given instead of from the shape and density (in the shape's frame)
    pub mass: Option<MassProps>,
    /// Coulomb friction; two bodies use the square root of the product of theirs (default 0.6)
    pub friction: f64,
    /// how much of the approach speed comes back, 0 to 1; two bodies use the larger (default 0)
    pub restitution: f64,
    /// rolling resistance: the torque that brakes a rolling body is this times the normal force times its
    /// smallest radius; two bodies use the larger (default 0: a ball rolls on until damping stops it)
    pub rolling_resistance: f64,
    /// 1/s: the velocity loses this fraction per second (default 0)
    pub linear_damping: f64,
    /// 1/s (default 0.05)
    pub angular_damping: f64,
    /// how much of the simulation's gravity acts on it (default 1)
    pub gravity_scale: f64,
    /// the layers the body is in (also what queries filter by)
    pub layers: u32,
    /// the layers it collides with: two bodies collide when each one's mask has a layer of the other
    pub mask: u32,
    /// it reports overlaps ([`ContactEvent::SensorBegin`]) and pushes nothing
    pub sensor: bool,
    /// it is swept against every body that is not a bullet, not only against what does not move (for small
    /// fast projectiles)
    pub bullet: bool,
    /// it may fall asleep (default true)
    pub can_sleep: bool,
    /// gyroscopic forces: a body tumbling about an unstable axis flips as it does in space (Bullet's implicit
    /// step; costs a little, default off)
    pub gyroscopic: bool,
    /// it never turns (a figure's capsule pushed around as a dynamic body)
    pub fixed_rotation: bool,
    /// it starts awake (default true)
    pub awake: bool,
    /// free for the caller (the World's body has it too)
    pub user: u64,
}

impl RigidBody {
    /// a body of `kind` with the defaults
    pub fn new(kind: BodyKind, shape: Shape, iso: Iso) -> RigidBody {
        RigidBody {
            kind,
            shape,
            iso,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            density: 1000.0,
            mass: None,
            friction: 0.6,
            restitution: 0.0,
            rolling_resistance: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            layers: 1,
            mask: u32::MAX,
            sensor: false,
            bullet: false,
            can_sleep: true,
            gyroscopic: false,
            fixed_rotation: false,
            awake: true,
            user: 0,
        }
    }

    /// a body that never moves
    pub fn fixed(shape: Shape, iso: Iso) -> RigidBody {
        RigidBody::new(BodyKind::Static, shape, iso)
    }

    /// a body moved by gravity, forces, contacts and joints
    pub fn dynamic(shape: Shape, iso: Iso) -> RigidBody {
        RigidBody::new(BodyKind::Dynamic, shape, iso)
    }

    /// a body moved by its velocity only
    pub fn kinematic(shape: Shape, iso: Iso) -> RigidBody {
        RigidBody::new(BodyKind::Kinematic, shape, iso)
    }

    /// the velocity of the centre of mass at the start (m/s)
    pub fn with_velocity(mut self, v: Vec3) -> RigidBody {
        self.linear_velocity = v;
        self
    }

    /// the angular velocity at the start (rad/s, world frame)
    pub fn with_angular_velocity(mut self, w: Vec3) -> RigidBody {
        self.angular_velocity = w;
        self
    }

    /// the density the mass comes from (kg/m³)
    pub fn with_density(mut self, density: f64) -> RigidBody {
        self.density = density;
        self
    }

    /// the mass as given (in the shape's frame) instead of from the density
    pub fn with_mass(mut self, mass: MassProps) -> RigidBody {
        self.mass = Some(mass);
        self
    }

    /// the friction coefficient
    pub fn with_friction(mut self, friction: f64) -> RigidBody {
        self.friction = friction;
        self
    }

    /// how much of an approach speed comes back (0 to 1)
    pub fn with_restitution(mut self, restitution: f64) -> RigidBody {
        self.restitution = restitution;
        self
    }

    /// the rolling resistance coefficient (0: none)
    pub fn with_rolling_resistance(mut self, c: f64) -> RigidBody {
        self.rolling_resistance = c;
        self
    }

    /// the linear and angular damping (1/s)
    pub fn with_damping(mut self, linear: f64, angular: f64) -> RigidBody {
        self.linear_damping = linear;
        self.angular_damping = angular;
        self
    }

    /// how much of the gravity acts on it
    pub fn with_gravity_scale(mut self, scale: f64) -> RigidBody {
        self.gravity_scale = scale;
        self
    }

    /// the layers it is in and those it collides with
    pub fn with_layers(mut self, layers: u32, mask: u32) -> RigidBody {
        self.layers = layers;
        self.mask = mask;
        self
    }

    /// a value for the caller (also the World body's `user`)
    pub fn with_user(mut self, user: u64) -> RigidBody {
        self.user = user;
        self
    }

    /// reports overlaps, pushes nothing
    pub fn sensor(mut self) -> RigidBody {
        self.sensor = true;
        self
    }

    /// swept against every body, not only the static ones
    pub fn bullet(mut self) -> RigidBody {
        self.bullet = true;
        self
    }

    /// never falls asleep
    pub fn sleepless(mut self) -> RigidBody {
        self.can_sleep = false;
        self
    }

    /// with gyroscopic forces
    pub fn gyroscopic(mut self) -> RigidBody {
        self.gyroscopic = true;
        self
    }

    /// never turns
    pub fn fixed_rotation(mut self) -> RigidBody {
        self.fixed_rotation = true;
        self
    }

    /// starts asleep (until something touches it)
    pub fn asleep(mut self) -> RigidBody {
        self.awake = false;
        self
    }
}

/// why [`Simulation::add`] refused a body
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyError {
    /// a mesh can be static or kinematic, not dynamic
    DynamicMesh,
    /// no valid mass: the density is not positive, or the given mass properties are not those of a body
    Mass,
    /// the pose, a velocity or a parameter is not finite
    NotFinite,
}

/// How the simulation steps.  The defaults suit bodies from centimetres to tens of metres at 60 steps a
/// second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParams {
    /// m/s²
    pub gravity: Vec3,
    /// solver substeps per step (1 to 64; default 4)
    pub substeps: u32,
    /// how stiff contacts push apart (Hz; at most a quarter of the substep rate; default 30)
    pub contact_hertz: f64,
    /// their damping ratio (default 10)
    pub contact_damping_ratio: f64,
    /// how stiff joints hold (Hz; at most half the substep rate; default 60)
    pub joint_hertz: f64,
    /// their damping ratio (default 2)
    pub joint_damping_ratio: f64,
    /// the fastest an overlap is pushed apart (m/s; default 3)
    pub max_push_speed: f64,
    /// slower approaches do not bounce (m/s; default 1)
    pub restitution_threshold: f64,
    /// this much overlap is left alone, so resting contacts stay (m; default 0.005)
    pub linear_slop: f64,
    /// contact points are made this far before touching, plus how far the bodies can move in the step
    /// (m; default 0.02)
    pub speculative_distance: f64,
    /// no body moves faster (m/s; default 1000)
    pub max_speed: f64,
    /// may islands fall asleep (default true)
    pub sleep: bool,
    /// how long an island must be slow to fall asleep (s; default 0.5)
    pub time_to_sleep: f64,
    /// slow: every point of every body of the island slower than this (m/s; default 0.05)
    pub sleep_speed: f64,
    /// sweep fast bodies against what does not move (default true)
    pub continuous: bool,
}

impl Default for SimParams {
    fn default() -> SimParams {
        SimParams {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            substeps: 4,
            contact_hertz: 30.0,
            contact_damping_ratio: 10.0,
            joint_hertz: 60.0,
            joint_damping_ratio: 2.0,
            max_push_speed: 3.0,
            restitution_threshold: 1.0,
            linear_slop: 0.005,
            speculative_distance: 0.02,
            max_speed: 1000.0,
            sleep: true,
            time_to_sleep: 0.5,
            sleep_speed: 0.05,
            continuous: true,
        }
    }
}

/// what a body is now
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    /// how it moves
    pub kind: BodyKind,
    /// the shape's frame
    pub iso: Iso,
    /// the centre of mass (world)
    pub center: Vec3,
    /// of the centre of mass (m/s)
    pub linear_velocity: Vec3,
    /// rad/s, world frame
    pub angular_velocity: Vec3,
    /// kg (0 for static and kinematic bodies)
    pub mass: f64,
    /// awake (it moves in the next step)
    pub awake: bool,
}

/// a point of a [`ContactManifold`]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ContactPoint {
    /// world, between the two surfaces, at the start of the last step
    pub point: Vec3,
    /// the gap then (negative: overlap)
    pub separation: f64,
    /// the impulse along the normal in the last substep (N s; divided by the substep: the force)
    pub normal_impulse: f64,
}

/// the contact of two bodies along one normal
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactManifold {
    /// the first body (the convex one when the other is a mesh)
    pub a: BodyId,
    /// the second body
    pub b: BodyId,
    /// unit, from a towards b
    pub normal: Vec3,
    /// with a mesh: its lowest triangle of the manifold; else [`crate::NO_TRIANGLE`]
    pub triangle: u32,
    /// how many of `points` are used
    pub count: usize,
    /// the points (the first `count`)
    pub points: [ContactPoint; MAX_POINTS],
}

/// what happened between bodies in a step
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContactEvent {
    /// a and b started to touch: where, the normal from a towards b, how fast they approached (m/s, along
    /// the normal: for the loudness of a sound)
    Begin { a: BodyId, b: BodyId, point: Vec3, normal: Vec3, speed: f64 },
    /// a and b no longer touch (also when one of them was removed: then its id is dead)
    End { a: BodyId, b: BodyId },
    /// a body entered a sensor
    SensorBegin { sensor: BodyId, other: BodyId },
    /// a body left a sensor (or one of them was removed)
    SensorEnd { sensor: BodyId, other: BodyId },
}

/// counters of the last step
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SimStats {
    /// all bodies
    pub bodies: usize,
    /// awake dynamic and moving kinematic bodies
    pub awake: usize,
    /// manifolds (also of islands asleep)
    pub manifolds: usize,
    /// their points
    pub points: usize,
    /// joints
    pub joints: usize,
    /// pairs whose manifolds were computed
    pub pairs: usize,
    /// islands asleep
    pub sleeping_islands: usize,
    /// bodies a sweep stopped
    pub swept: usize,
}

#[derive(Clone, Copy, Debug)]
struct Rb {
    kind: BodyKind,
    mass: MassProps,
    inv_mass: f64,
    inv_inertia: Mat3,
    center: Vec3,
    rot: Quat,
    v: Vec3,
    w: Vec3,
    force: Vec3,
    torque: Vec3,
    friction: f64,
    restitution: f64,
    rolling: f64,
    linear_damping: f64,
    angular_damping: f64,
    gravity_scale: f64,
    mask: u32,
    sensor: bool,
    bullet: bool,
    can_sleep: bool,
    gyroscopic: bool,
    awake: bool,
    sleep_time: f64,
    group: u32,
    extent_min: f64,
    extent_max: f64,
    target: Option<Iso>,
}

impl Rb {
    #[inline]
    fn moves(&self) -> bool {
        self.kind != BodyKind::Static
    }

    #[inline]
    fn dynamic(&self) -> bool {
        self.kind == BodyKind::Dynamic
    }

    fn iso(&self) -> Iso {
        Iso::new(self.center - self.rot.rotate(self.mass.center), self.rot)
    }

    fn inv_inertia_world(&self) -> Mat3 {
        if self.rot.is_identity() { self.inv_inertia } else { self.inv_inertia.rotated(self.rot) }
    }

    /// how far a point of the body can move in `dt`
    fn reach(&self, dt: f64) -> f64 {
        if !self.awake || !self.moves() {
            return 0.0;
        }
        (self.v.length() + self.w.length() * self.extent_max) * dt
    }
}

#[derive(Clone)]
struct JSlot {
    generation: u32,
    rt: Option<JointRt>,
}

/// what a pair of bodies does
#[derive(Clone, Copy, PartialEq, Eq)]
enum PairKind {
    Nothing,
    Sensor,
    Contact,
}

/// The rigid bodies of a game, and the collision [`World`] they live in (for rays, casts and the character
/// controller; [`Simulation::world`]).  Bodies are added and removed here, not in the World.  A clone is a
/// snapshot: it steps on to the same bits as the original (the impulses kept for the next step and the
/// islands asleep included), for a client that rolls back and steps again.
#[derive(Clone)]
pub struct Simulation {
    world: World,
    rbs: Vec<Option<Rb>>,
    joints: Vec<JSlot>,
    free_joints: Vec<u32>,
    /// the slot pairs (lower first) a joint keeps from colliding
    no_collide: Vec<(u32, u32)>,
    /// the members of each sleeping island
    groups: Vec<Vec<u32>>,
    free_groups: Vec<u32>,
    /// the manifolds of the last step, in the order they were made (the order of the solver)
    constraints: Vec<Constraint>,
    /// their indices sorted by (a, b, key): a constraint is a kilobyte, so it is not moved into order
    order: Vec<u32>,
    /// overlapping (sensor, other) slots, sorted
    sensors: Vec<(u32, u32)>,
    /// touching pairs (as oriented in the constraints), sorted
    touching: Vec<(u32, u32)>,
    events: Vec<ContactEvent>,
    pending: Vec<ContactEvent>,
    params: SimParams,
    stats: SimStats,
}

impl Default for Simulation {
    fn default() -> Simulation {
        Simulation::new()
    }
}

/// the largest distance from `c` to a point of the shape and the smallest from it to the surface
fn extents(shape: &Shape, c: Vec3) -> (f64, f64) {
    let b = shape.aabb(&Iso::IDENTITY);
    let mut far = 0.0f64;
    for k in 0..8 {
        let p = Vec3::new(
            if k & 1 != 0 { b.max.x } else { b.min.x },
            if k & 2 != 0 { b.max.y } else { b.min.y },
            if k & 4 != 0 { b.max.z } else { b.min.z },
        );
        far = max(far, (p - c).length());
    }
    let near = match shape {
        Shape::Sphere { radius } | Shape::Capsule { radius, .. } => *radius,
        Shape::Box { half } => min(half.x, min(half.y, half.z)),
        Shape::Hull(h) => h.planes().iter().map(|(n, d)| d - n.dot(c)).fold(f64::INFINITY, min),
        Shape::Mesh(_) => 0.0,
    };
    let near = if near.is_finite() { max(near, 0.0) } else { 0.0 };
    (min(far, f64::MAX), near)
}

/// the gyroscopic torque -w x I w over a substep, as Jolt applies it: an explicit step of the angular
/// momentum in the body's frame, scaled back to its magnitude (so the step adds no energy)
fn gyroscopic(w: Vec3, q: Quat, inertia: &Mat3, inv_inertia: &Mat3, h: f64) -> Vec3 {
    let wb = q.inverse_rotate(w);
    let l = inertia.mul_vec(wb);
    let nl = l - wb.cross(l) * h;
    let (ll, nn) = (l.length_sq(), nl.length_sq());
    let nl = if nn > 0.0 { nl * sqrt(ll / nn) } else { Vec3::ZERO };
    q.rotate(inv_inertia.mul_vec(nl))
}

/// what each solver body needs in the substeps
#[derive(Clone, Copy)]
struct Extra {
    dynamic: bool,
    damp_v: f64,
    damp_w: f64,
    /// gravity times its scale plus force / mass
    accel: Vec3,
    /// inverse inertia times torque
    alpha: Vec3,
    gyro: bool,
    rot0: Quat,
    inertia: Mat3,
    inv_inertia: Mat3,
}

impl Simulation {
    /// an empty simulation with the default parameters (gravity 9.81 m/s² down -Y)
    pub fn new() -> Simulation {
        Simulation::with_params(SimParams::default())
    }

    /// an empty simulation with these parameters
    pub fn with_params(params: SimParams) -> Simulation {
        Simulation {
            world: World::new(),
            rbs: Vec::new(),
            joints: Vec::new(),
            free_joints: Vec::new(),
            no_collide: Vec::new(),
            groups: Vec::new(),
            free_groups: Vec::new(),
            constraints: Vec::new(),
            order: Vec::new(),
            sensors: Vec::new(),
            touching: Vec::new(),
            events: Vec::new(),
            pending: Vec::new(),
            params,
            stats: SimStats::default(),
        }
    }

    /// the parameters
    pub fn params(&self) -> &SimParams {
        &self.params
    }

    /// change the parameters (they apply from the next step)
    pub fn set_params(&mut self, params: SimParams) {
        self.params = params;
    }

    /// the collision world of the bodies: rays, casts, overlaps and the character controller run on it
    pub fn world(&self) -> &World {
        &self.world
    }

    /// the number of bodies
    pub fn len(&self) -> usize {
        self.world.len()
    }

    /// no bodies
    pub fn is_empty(&self) -> bool {
        self.world.is_empty()
    }

    fn rb(&self, id: BodyId) -> Option<&Rb> {
        self.world.body(id)?;
        self.rbs.get(id.index() as usize)?.as_ref()
    }

    fn rb_mut(&mut self, id: BodyId) -> Option<&mut Rb> {
        self.world.body(id)?;
        self.rbs.get_mut(id.index() as usize)?.as_mut()
    }

    /// add a body -> its id (also its id in [`Simulation::world`])
    pub fn add(&mut self, body: RigidBody) -> Result<BodyId, BodyError> {
        let b = &body;
        let fin = b.iso.pos.is_finite()
            && b.iso.rot.is_finite()
            && b.linear_velocity.is_finite()
            && b.angular_velocity.is_finite()
            && b.friction.is_finite()
            && b.restitution.is_finite()
            && b.rolling_resistance.is_finite()
            && b.linear_damping.is_finite()
            && b.angular_damping.is_finite()
            && b.gravity_scale.is_finite();
        if !fin {
            return Err(BodyError::NotFinite);
        }
        let dynamic = b.kind == BodyKind::Dynamic;
        if dynamic && !b.shape.is_convex() {
            return Err(BodyError::DynamicMesh);
        }
        let mass = if dynamic {
            let m = match b.mass {
                Some(m) => m,
                None => b.shape.mass_properties(b.density).ok_or(BodyError::Mass)?,
            };
            if !m.is_valid() {
                return Err(BodyError::Mass);
            }
            m
        } else {
            MassProps::new(0.0, Vec3::ZERO, Mat3::ZERO)
        };
        let inv_inertia = if dynamic && !b.fixed_rotation { mass.inertia.inverse().unwrap_or(Mat3::ZERO) } else { Mat3::ZERO };
        let rot = b.iso.rot.normalize();
        let iso = Iso::new(b.iso.pos, rot);
        let (extent_max, extent_min) = extents(&b.shape, mass.center);
        let id = self.world.add_with_layers(b.shape.clone(), iso, b.layers);
        self.world.set_user(id, b.user);
        let moves = b.kind != BodyKind::Static;
        let rb = Rb {
            kind: b.kind,
            mass,
            inv_mass: if dynamic { 1.0 / mass.mass } else { 0.0 },
            inv_inertia,
            center: iso.transform_point(mass.center),
            rot,
            v: if moves { b.linear_velocity } else { Vec3::ZERO },
            w: if moves && !b.fixed_rotation { b.angular_velocity } else { Vec3::ZERO },
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            friction: max(b.friction, 0.0),
            restitution: crate::math::clamp(b.restitution, 0.0, 1.0),
            rolling: max(b.rolling_resistance, 0.0),
            linear_damping: max(b.linear_damping, 0.0),
            angular_damping: max(b.angular_damping, 0.0),
            gravity_scale: b.gravity_scale,
            mask: b.mask,
            sensor: b.sensor,
            bullet: b.bullet,
            can_sleep: b.can_sleep,
            gyroscopic: b.gyroscopic,
            awake: moves && (b.awake || b.kind == BodyKind::Kinematic),
            sleep_time: 0.0,
            group: NONE,
            extent_min,
            extent_max,
            target: None,
        };
        let i = id.index() as usize;
        if self.rbs.len() <= i {
            self.rbs.resize(i + 1, None);
        }
        self.rbs[i] = Some(rb);
        if dynamic && !b.awake {
            // asleep from the start: an island of its own
            self.put_to_sleep(&[id.index()]);
        }
        Ok(id)
    }

    /// remove a body, its joints and its contacts -> false when the id is dead.  Bodies it touched wake up.
    pub fn remove(&mut self, id: BodyId) -> bool {
        if self.rb(id).is_none() {
            return false;
        }
        let slot = id.index();
        self.wake_slot(slot);
        // the joints on it
        let dead: Vec<JointId> = self.joints().filter(|(_, j)| j.a == id || j.b == id).map(|(jid, _)| jid).collect();
        for j in dead {
            self.remove_joint(j);
        }
        // its contacts: the others wake, the events say so
        let mut others: Vec<u32> = Vec::new();
        let mut ends: Vec<(u32, u32)> = Vec::new();
        for c in &self.constraints {
            if c.a == slot || c.b == slot {
                others.push(if c.a == slot { c.b } else { c.a });
            }
        }
        for &(a, b) in &self.touching {
            if a == slot || b == slot {
                ends.push((a, b));
            }
        }
        for (a, b) in ends {
            let (ia, ib) = (self.world.at(a).map(|x| x.0), self.world.at(b).map(|x| x.0));
            if let (Some(a), Some(b)) = (ia, ib) {
                self.pending.push(ContactEvent::End { a, b });
            }
        }
        let sens: Vec<(u32, u32)> = self.sensors.iter().copied().filter(|&(s, o)| s == slot || o == slot).collect();
        for (s, o) in sens {
            if let (Some((si, _)), Some((oi, _))) = (self.world.at(s), self.world.at(o)) {
                self.pending.push(ContactEvent::SensorEnd { sensor: si, other: oi });
            }
        }
        self.constraints.retain(|c| c.a != slot && c.b != slot);
        self.order = sort_order(&self.constraints);
        self.touching.retain(|&(a, b)| a != slot && b != slot);
        self.sensors.retain(|&(s, o)| s != slot && o != slot);
        for o in others {
            self.wake_slot(o);
        }
        self.rbs[slot as usize] = None;
        self.world.remove(id);
        true
    }

    /// is the id alive
    pub fn contains(&self, id: BodyId) -> bool {
        self.rb(id).is_some()
    }

    /// what the body is now
    pub fn state(&self, id: BodyId) -> Option<BodyState> {
        let r = self.rb(id)?;
        Some(BodyState { kind: r.kind, iso: r.iso(), center: r.center, linear_velocity: r.v, angular_velocity: r.w, mass: r.mass.mass, awake: r.awake })
    }

    /// where its shape's frame is
    pub fn iso(&self, id: BodyId) -> Option<Iso> {
        self.rb(id).map(|r| r.iso())
    }

    /// the velocity of the centre of mass (m/s) and the angular velocity (rad/s)
    pub fn velocity(&self, id: BodyId) -> Option<(Vec3, Vec3)> {
        self.rb(id).map(|r| (r.v, r.w))
    }

    /// the velocity of the body's point that is at `point` now
    pub fn velocity_at(&self, id: BodyId, point: Vec3) -> Option<Vec3> {
        self.rb(id).map(|r| r.v + r.w.cross(point - r.center))
    }

    /// the mass properties (in the shape's frame; mass 0 for static and kinematic bodies)
    pub fn mass(&self, id: BodyId) -> Option<MassProps> {
        self.rb(id).map(|r| r.mass)
    }

    /// is it awake (a static body never is)
    pub fn is_awake(&self, id: BodyId) -> bool {
        self.rb(id).is_some_and(|r| r.awake)
    }

    /// put the body somewhere else (teleport) -> false when the id is dead.  It wakes, and so does what touched
    /// it.
    pub fn set_iso(&mut self, id: BodyId, iso: Iso) -> bool {
        if !iso.pos.is_finite() || !iso.rot.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        r.rot = iso.rot.normalize();
        r.center = Iso::new(iso.pos, r.rot).transform_point(r.mass.center);
        r.target = None;
        let now = r.iso();
        self.world.set_iso(id, now);
        self.wake_touching(id.index());
        self.wake_slot(id.index());
        true
    }

    /// set the velocities (m/s, rad/s) -> false when the id is dead; a static body has none
    pub fn set_velocity(&mut self, id: BodyId, linear: Vec3, angular: Vec3) -> bool {
        if !linear.is_finite() || !angular.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if !r.moves() {
            return true;
        }
        r.v = linear;
        r.w = if r.inv_inertia == Mat3::ZERO && r.dynamic() { Vec3::ZERO } else { angular };
        self.wake_slot(id.index());
        true
    }

    /// an impulse (N s) at a point (world) of a dynamic body -> false when the id is dead
    pub fn apply_impulse(&mut self, id: BodyId, impulse: Vec3, point: Vec3) -> bool {
        if !impulse.is_finite() || !point.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if r.dynamic() {
            r.v += impulse * r.inv_mass;
            r.w += r.inv_inertia_world().mul_vec((point - r.center).cross(impulse));
            self.wake_slot(id.index());
        }
        true
    }

    /// an angular impulse (N m s) on a dynamic body
    pub fn apply_angular_impulse(&mut self, id: BodyId, impulse: Vec3) -> bool {
        if !impulse.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if r.dynamic() {
            r.w += r.inv_inertia_world().mul_vec(impulse);
            self.wake_slot(id.index());
        }
        true
    }

    /// a force (N) at a point (world) of a dynamic body during the next step (forces add up; a step clears
    /// them)
    pub fn apply_force(&mut self, id: BodyId, force: Vec3, point: Vec3) -> bool {
        if !force.is_finite() || !point.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if r.dynamic() {
            r.force += force;
            r.torque += (point - r.center).cross(force);
            self.wake_slot(id.index());
        }
        true
    }

    /// a torque (N m) on a dynamic body during the next step
    pub fn apply_torque(&mut self, id: BodyId, torque: Vec3) -> bool {
        if !torque.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if r.dynamic() {
            r.torque += torque;
            self.wake_slot(id.index());
        }
        true
    }

    /// move a kinematic body to `target` over the next step of `dt` seconds: its velocities are set so it
    /// gets there (what it touches on the way is pushed), and it ends exactly there.  After that step it keeps
    /// those velocities until they are set again.
    pub fn move_kinematic(&mut self, id: BodyId, target: Iso, dt: f64) -> bool {
        if !target.pos.is_finite() || !target.rot.is_finite() || !(dt > 0.0) {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        if r.kind != BodyKind::Kinematic {
            return false;
        }
        let t = Iso::new(target.pos, target.rot.normalize());
        r.v = (t.transform_point(r.mass.center) - r.center) / dt;
        r.w = t.rot.mul(r.rot.conjugate()).to_scaled_axis() / dt;
        r.target = Some(t);
        r.awake = true;
        true
    }

    /// wake a body and its island
    pub fn wake(&mut self, id: BodyId) -> bool {
        if self.rb(id).is_none() {
            return false;
        }
        self.wake_slot(id.index());
        true
    }

    /// set friction, restitution and rolling resistance
    pub fn set_material(&mut self, id: BodyId, friction: f64, restitution: f64, rolling_resistance: f64) -> bool {
        if !friction.is_finite() || !restitution.is_finite() || !rolling_resistance.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        r.friction = max(friction, 0.0);
        r.restitution = crate::math::clamp(restitution, 0.0, 1.0);
        r.rolling = max(rolling_resistance, 0.0);
        true
    }

    /// set the layers the body is in and those it collides with
    pub fn set_layers(&mut self, id: BodyId, layers: u32, mask: u32) -> bool {
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        r.mask = mask;
        self.world.set_layers(id, layers);
        self.wake_slot(id.index());
        true
    }

    /// set the gravity scale
    pub fn set_gravity_scale(&mut self, id: BodyId, scale: f64) -> bool {
        if !scale.is_finite() {
            return false;
        }
        let Some(r) = self.rb_mut(id) else {
            return false;
        };
        r.gravity_scale = scale;
        self.wake_slot(id.index());
        true
    }

    // ------------------------------------------------------------ joints

    /// a joint between a and b whose frame is `frame` (world) at their poses now -> its id
    pub fn add_joint(&mut self, a: BodyId, b: BodyId, frame: Iso, kind: JointKind) -> Result<JointId, JointError> {
        let (Some(ia), Some(ib)) = (self.iso(a), self.iso(b)) else {
            return Err(JointError::DeadBody);
        };
        let frame = Iso::new(frame.pos, frame.rot.normalize());
        let joint = Joint { a, b, frame_a: ia.inv_mul(&frame), frame_b: ib.inv_mul(&frame), kind, collide_connected: false, user: 0 };
        self.add_joint_local(joint)
    }

    /// a distance joint between a's point `pa` and b's point `pb` (world, now) -> its id
    pub fn add_distance_joint(&mut self, a: BodyId, b: BodyId, pa: Vec3, pb: Vec3, kind: JointKind) -> Result<JointId, JointError> {
        let (Some(ia), Some(ib)) = (self.iso(a), self.iso(b)) else {
            return Err(JointError::DeadBody);
        };
        let joint = Joint {
            a,
            b,
            frame_a: Iso::at(ia.inverse_transform_point(pa)),
            frame_b: Iso::at(ib.inverse_transform_point(pb)),
            kind,
            collide_connected: false,
            user: 0,
        };
        self.add_joint_local(joint)
    }

    /// a joint with its frames given in the bodies' own frames -> its id
    pub fn add_joint_local(&mut self, joint: Joint) -> Result<JointId, JointError> {
        let (Some(ra), Some(rb)) = (self.rb(joint.a), self.rb(joint.b)) else {
            return Err(JointError::DeadBody);
        };
        if joint.a == joint.b {
            return Err(JointError::SameBody);
        }
        if !ra.dynamic() && !rb.dynamic() {
            return Err(JointError::NothingMoves);
        }
        let joint = joint.checked()?;
        let index = match self.free_joints.pop() {
            Some(i) => i,
            None => {
                self.joints.push(JSlot { generation: 0, rt: None });
                (self.joints.len() - 1) as u32
            }
        };
        let slot = &mut self.joints[index as usize];
        slot.rt = Some(JointRt::new(joint));
        let id = JointId { index, generation: slot.generation };
        self.wake_slot(joint.a.index());
        self.wake_slot(joint.b.index());
        self.rebuild_no_collide();
        Ok(id)
    }

    /// remove a joint -> false when the id is dead.  Its bodies wake.
    pub fn remove_joint(&mut self, id: JointId) -> bool {
        let Some(s) = self.joints.get_mut(id.index as usize) else {
            return false;
        };
        if s.generation != id.generation {
            return false;
        }
        let Some(rt) = s.rt.take() else {
            return false;
        };
        s.generation = s.generation.wrapping_add(1);
        self.free_joints.push(id.index);
        self.wake_slot(rt.ia);
        self.wake_slot(rt.ib);
        self.rebuild_no_collide();
        true
    }

    fn joint_rt(&self, id: JointId) -> Option<&JointRt> {
        let s = self.joints.get(id.index as usize)?;
        if s.generation != id.generation {
            return None;
        }
        s.rt.as_ref()
    }

    /// the joint
    pub fn joint(&self, id: JointId) -> Option<&Joint> {
        self.joint_rt(id).map(|r| &r.joint)
    }

    /// change a joint's kind (limits, motor, spring) or whether its bodies collide -> false when the id is
    /// dead or the new joint is not valid; its bodies wake
    pub fn set_joint(&mut self, id: JointId, kind: JointKind, collide_connected: bool) -> bool {
        let Some(s) = self.joints.get_mut(id.index as usize) else {
            return false;
        };
        if s.generation != id.generation {
            return false;
        }
        let Some(rt) = s.rt.as_mut() else {
            return false;
        };
        let mut j = rt.joint;
        j.kind = kind;
        j.collide_connected = collide_connected;
        let Ok(j) = j.checked() else {
            return false;
        };
        let same_kind = core::mem::discriminant(&rt.joint.kind) == core::mem::discriminant(&j.kind);
        rt.joint = j;
        if !same_kind {
            rt.acc = Default::default();
        }
        let (ia, ib) = (rt.ia, rt.ib);
        self.wake_slot(ia);
        self.wake_slot(ib);
        self.rebuild_no_collide();
        true
    }

    /// every joint with its id, by index
    pub fn joints(&self) -> impl Iterator<Item = (JointId, &Joint)> + '_ {
        self.joints.iter().enumerate().filter_map(|(i, s)| s.rt.as_ref().map(|r| (JointId { index: i as u32, generation: s.generation }, &r.joint)))
    }

    /// a hinge's angle (rad), a slider's position (m), a distance joint's length (m), a ball joint's twist
    /// (rad), now
    pub fn joint_position(&self, id: JointId) -> Option<f64> {
        let rt = self.joint_rt(id)?;
        let mut r = *rt;
        let (a, b) = (self.rb(rt.joint.a)?, self.rb(rt.joint.b)?);
        r.prepare((a.center, a.rot, a.mass.center), (b.center, b.rot, b.mass.center), rt.h);
        Some(r.position())
    }

    /// the force (N) and torque (N m) the joint applied to b in the last step (world; a got the opposite):
    /// for breakable joints and for measuring
    pub fn joint_force(&self, id: JointId) -> Option<(Vec3, Vec3)> {
        self.joint_rt(id).map(|r| r.reaction())
    }

    fn rebuild_no_collide(&mut self) {
        let mut v: Vec<(u32, u32)> = Vec::new();
        for s in &self.joints {
            if let Some(r) = &s.rt {
                if !r.joint.collide_connected {
                    v.push((min_u(r.ia, r.ib), max_u(r.ia, r.ib)));
                }
            }
        }
        v.sort_unstable();
        v.dedup();
        self.no_collide = v;
    }

    // ------------------------------------------------------------ sleeping

    /// wake slot i and every body of its sleeping island
    fn wake_slot(&mut self, i: u32) -> Vec<u32> {
        let Some(Some(r)) = self.rbs.get(i as usize) else {
            return Vec::new();
        };
        if r.awake || !r.moves() {
            return Vec::new();
        }
        let members = if r.group != NONE {
            let g = r.group;
            let m = core::mem::take(&mut self.groups[g as usize]);
            self.free_groups.push(g);
            m
        } else {
            vec![i]
        };
        for &m in &members {
            if let Some(Some(r)) = self.rbs.get_mut(m as usize) {
                r.awake = true;
                r.sleep_time = 0.0;
                r.group = NONE;
            }
        }
        members
    }

    /// wake what touches slot i (after a teleport)
    fn wake_touching(&mut self, i: u32) {
        let others: Vec<u32> = self.constraints.iter().filter(|c| c.a == i || c.b == i).map(|c| if c.a == i { c.b } else { c.a }).collect();
        for o in others {
            self.wake_slot(o);
        }
    }

    fn put_to_sleep(&mut self, members: &[u32]) {
        let g = match self.free_groups.pop() {
            Some(g) => g,
            None => {
                self.groups.push(Vec::new());
                (self.groups.len() - 1) as u32
            }
        };
        for &m in members {
            if let Some(Some(r)) = self.rbs.get_mut(m as usize) {
                r.awake = false;
                r.v = Vec3::ZERO;
                r.w = Vec3::ZERO;
                r.sleep_time = 0.0;
                r.group = g;
            }
        }
        self.groups[g as usize] = members.to_vec();
    }

    // ------------------------------------------------------------ the step's parts

    /// what a and b do with each other
    fn pair_kind(&self, a: u32, b: u32) -> PairKind {
        let (Some(Some(ra)), Some(Some(rb))) = (self.rbs.get(a as usize), self.rbs.get(b as usize)) else {
            return PairKind::Nothing;
        };
        let (Some((_, ba)), Some((_, bb))) = (self.world.at(a), self.world.at(b)) else {
            return PairKind::Nothing;
        };
        if ba.layers & rb.mask == 0 || bb.layers & ra.mask == 0 {
            return PairKind::Nothing;
        }
        if !ba.shape.is_convex() && !bb.shape.is_convex() {
            return PairKind::Nothing;
        }
        if ra.sensor || rb.sensor {
            if ra.sensor && rb.sensor {
                return PairKind::Nothing;
            }
            if !ra.moves() && !rb.moves() {
                return PairKind::Nothing;
            }
            return PairKind::Sensor;
        }
        if !ra.dynamic() && !rb.dynamic() {
            return PairKind::Nothing;
        }
        if self.no_collide.binary_search(&(min_u(a, b), max_u(a, b))).is_ok() {
            return PairKind::Nothing;
        }
        PairKind::Contact
    }

    /// the constraint of a pair (a first: convex) from its geometry, with the impulses of the last step's points
    /// nearby, made in place at the end of `out` (`prev_order`: the last step's constraints sorted)
    #[allow(clippy::too_many_arguments)]
    fn push_constraint(&self, out: &mut Vec<Constraint>, a: u32, b: u32, g: &Geom, margins: (f64, f64), prev: &[Constraint], prev_order: &[u32]) {
        let (Some(Some(ra)), Some(Some(rb))) = (self.rbs.get(a as usize), self.rbs.get(b as usize)) else {
            return;
        };
        let n = g.normal;
        let tangent = solver::tangents(n);
        out.push(Constraint {
            a,
            b,
            sa: NONE,
            sb: NONE,
            key: g.key,
            normal: n,
            tangent,
            friction: sqrt(ra.friction * rb.friction),
            restitution: max(ra.restitution, rb.restitution),
            count: 0,
            points: [CPoint::default(); MAX_POINTS],
            rolling: 0.0,
            rolling_impulse: Vec3::ZERO,
            rolling_mass: Mat3::ZERO,
            rigid_other: false,
            deep: 0.0,
        });
        let Some(c) = out.last_mut() else {
            return;
        };
        let roll = max(ra.rolling, rb.rolling);
        if roll > 0.0 {
            // the lever: the smaller radius of what rolls
            let r = match (ra.dynamic(), rb.dynamic()) {
                (true, true) => min(ra.extent_min, rb.extent_min),
                (true, false) => ra.extent_min,
                _ => rb.extent_min,
            };
            c.rolling = roll * r;
        }
        let lo = prev_order.partition_point(|&k| (prev[k as usize].a, prev[k as usize].b) < (a, b));
        let hi = prev_order.partition_point(|&k| (prev[k as usize].a, prev[k as usize].b) <= (a, b));
        let match_d2 = {
            let d = 4.0 * self.params.linear_slop;
            d * d
        };
        let mut rolling_impulse = Vec3::ZERO;
        for mp in &g.points[..g.count] {
            let ra_w = mp.point - ra.center;
            let local_a = ra.rot.inverse_rotate(ra_w);
            // the cores' points under the contact: the point lies midway between the two surfaces
            let half = mp.separation * 0.5;
            let ta = mp.point - n * (half + margins.0) - ra.center;
            let tb = mp.point + n * (half + margins.1) - rb.center;
            let p = &mut c.points[c.count];
            *p = CPoint { point: mp.point, ra: ra_w, rb: mp.point - rb.center, ta, tb, local_a, separation: mp.separation, ..CPoint::default() };
            let mut best = match_d2;
            for &k in &prev_order[lo..hi] {
                let oc = &prev[k as usize];
                if oc.normal.dot(n) < 0.95 {
                    continue;
                }
                for op in &oc.points[..oc.count] {
                    let d2 = (op.local_a - local_a).length_sq();
                    if d2 < best {
                        best = d2;
                        if roll > 0.0 {
                            rolling_impulse = oc.rolling_impulse;
                        }
                        p.normal_impulse = op.normal_impulse;
                        let f = oc.tangent[0] * op.tangent_impulse[0] + oc.tangent[1] * op.tangent_impulse[1];
                        p.tangent_impulse = [f.dot(tangent[0]), f.dot(tangent[1])];
                    }
                }
            }
            c.count += 1;
        }
        if roll > 0.0 {
            c.rolling_impulse = rolling_impulse;
        }
        if c.count == 0 {
            out.pop();
        }
    }

    /// find the pairs of the awake bodies and their manifolds; wakes the islands they touch (`active` grows
    /// by them) -> the new constraints, sensor overlaps
    fn collide(&mut self, dt: f64, active: &mut Vec<u32>) -> (Vec<Constraint>, Vec<u32>, Vec<(u32, u32)>) {
        let spec0 = max(self.params.speculative_distance, 0.0);
        let prev = core::mem::take(&mut self.constraints);
        let prev_order = core::mem::take(&mut self.order);
        let mut out: Vec<Constraint> = Vec::with_capacity(prev.len() + 64);
        let mut sensors: Vec<(u32, u32)> = Vec::new();
        let mut done: BTreeSet<(u32, u32)> = BTreeSet::new();
        let mut geoms: Vec<Geom> = Vec::new();
        let mut scratch = manifold::Scratch::new();
        let mut start = 0usize;
        let mut pairs = 0usize;
        loop {
            let mut cand: Vec<(u32, u32)> = Vec::new();
            for &i in &active[start..] {
                let Some((_, body)) = self.world.at(i) else {
                    continue;
                };
                let Some(Some(r)) = self.rbs.get(i as usize) else {
                    continue;
                };
                let reach = min(r.reach(dt), max(r.extent_max, spec0));
                let bx = body.shape.aabb(&body.iso).expanded(spec0 + reach);
                self.world.query_box(&bx, |j| {
                    if j != i {
                        cand.push((min_u(i, j), max_u(i, j)));
                    }
                });
            }
            start = active.len();
            cand.sort_unstable();
            cand.dedup();
            for (x, y) in cand {
                if !done.insert((x, y)) {
                    continue;
                }
                let kind = self.pair_kind(x, y);
                if kind == PairKind::Nothing {
                    continue;
                }
                let (Some((_, bx)), Some((_, by))) = (self.world.at(x), self.world.at(y)) else {
                    continue;
                };
                // the convex one first
                let (a, b, ba, bb) = if bx.shape.is_convex() { (x, y, bx, by) } else { (y, x, by, bx) };
                if kind == PairKind::Sensor {
                    if query::intersects(&ba.shape, &ba.iso, &bb.shape, &bb.iso) {
                        let sa = self.rbs[a as usize].is_some_and(|r| r.sensor);
                        sensors.push(if sa { (a, b) } else { (b, a) });
                    }
                    continue;
                }
                pairs += 1;
                let (Some(Some(r_a)), Some(Some(r_b))) = (self.rbs.get(a as usize), self.rbs.get(b as usize)) else {
                    continue;
                };
                let reach = min(r_a.reach(dt), max(r_a.extent_max, spec0)) + min(r_b.reach(dt), max(r_b.extent_max, spec0));
                let spec = spec0 + reach;
                geoms.clear();
                match &bb.shape {
                    Shape::Mesh(m) => manifold::mesh_pair(&ba.shape, &ba.iso, m, &bb.iso, spec, &mut scratch, &mut geoms),
                    _ => {
                        if let Some(g) = manifold::convex_pair(&ba.shape, &ba.iso, &bb.shape, &bb.iso, spec, &mut scratch) {
                            geoms.push(g);
                        }
                    }
                }
                if geoms.iter().all(|g| g.count == 0) {
                    continue;
                }
                let margins = (ba.shape.margin(), bb.shape.margin());
                for g in &geoms {
                    self.push_constraint(&mut out, a, b, g, margins, &prev, &prev_order);
                }
                // a sleeping body this pair touches wakes with its island
                for s in [a, b] {
                    if self.rbs[s as usize].is_some_and(|r| r.dynamic() && !r.awake) {
                        let woke = self.wake_slot(s);
                        active.extend_from_slice(&woke);
                    }
                }
            }
            if active.len() == start {
                break;
            }
        }
        active.sort_unstable();
        active.dedup();
        // the manifolds of pairs that were not looked at (asleep, or against what does not move) stay
        let is_active = |s: u32, rbs: &Vec<Option<Rb>>| rbs.get(s as usize).and_then(|r| r.as_ref()).is_some_and(|r| r.awake && r.moves());
        for c in prev {
            if !is_active(c.a, &self.rbs) && !is_active(c.b, &self.rbs) && self.rbs.get(c.a as usize).is_some_and(|r| r.is_some()) && self.rbs.get(c.b as usize).is_some_and(|r| r.is_some()) {
                out.push(c);
            }
        }
        let order = sort_order(&out);
        for &(s, o) in &self.sensors {
            if !is_active(s, &self.rbs) && !is_active(o, &self.rbs) {
                sensors.push((s, o));
            }
        }
        sensors.sort_unstable();
        sensors.dedup();
        self.stats.pairs = pairs;
        (out, order, sensors)
    }

    /// advance the simulation by `dt` seconds (a fixed step, 1/60 s for example); the events of the step are
    /// in [`Simulation::events`] afterwards
    pub fn step(&mut self, dt: f64) {
        self.events.clear();
        self.events.append(&mut self.pending);
        if !(dt > 0.0 && dt.is_finite()) {
            return;
        }
        let p = self.params;
        let substeps = p.substeps.clamp(1, 64);
        let h = dt / substeps as f64;
        let inv_h = 1.0 / h;
        self.stats.swept = 0;

        // the bodies that move in this step
        let mut active: Vec<u32> = Vec::new();
        for (i, r) in self.rbs.iter().enumerate() {
            if let Some(r) = r {
                if r.awake && r.moves() {
                    active.push(i as u32);
                }
            }
        }
        let (mut cons, order, sensors) = self.collide(dt, &mut active);

        // the solver's bodies
        let mut sidx = vec![NONE; self.rbs.len()];
        let mut sb: Vec<SBody> = Vec::with_capacity(active.len() + 2);
        let mut extra: Vec<Extra> = Vec::with_capacity(active.len());
        for &i in &active {
            let r = self.rbs[i as usize].as_ref().copied();
            let Some(r) = r else {
                continue;
            };
            sidx[i as usize] = sb.len() as u32;
            let dynamic = r.dynamic();
            let inv_i = if dynamic { r.inv_inertia_world() } else { Mat3::ZERO };
            sb.push(SBody { v: r.v, w: r.w, dp: Vec3::ZERO, dq: Quat::IDENTITY, inv_mass: r.inv_mass, inv_i });
            extra.push(Extra {
                dynamic,
                damp_v: 1.0 / (1.0 + h * r.linear_damping),
                damp_w: 1.0 / (1.0 + h * r.angular_damping),
                accel: if dynamic { p.gravity * r.gravity_scale + r.force * r.inv_mass } else { Vec3::ZERO },
                alpha: if dynamic { inv_i.mul_vec(r.torque) } else { Vec3::ZERO },
                gyro: dynamic && r.gyroscopic && r.inv_inertia != Mat3::ZERO,
                rot0: r.rot,
                inertia: r.mass.inertia,
                inv_inertia: r.inv_inertia,
            });
        }
        // the two static bodies of solver::pair
        sb.push(SBody::STATIC);
        sb.push(SBody::STATIC);
        // the constraints that are solved: those with a body in the solver
        let mut solve_idx: Vec<usize> = Vec::new();
        // in the order of the pairs (a, b), not of their making: islands woken in the middle of the search would
        // otherwise come last, and a heap that wakes its parts in turn never fell asleep
        for &k in &order {
            let k = k as usize;
            let c = &mut cons[k];
            c.sa = sidx.get(c.a as usize).copied().unwrap_or(NONE);
            c.sb = sidx.get(c.b as usize).copied().unwrap_or(NONE);
            if c.sa == NONE && c.sb == NONE {
                continue;
            }
            let ma = if c.sa == NONE { 0.0 } else { sb[c.sa as usize].inv_mass };
            let mb = if c.sb == NONE { 0.0 } else { sb[c.sb as usize].inv_mass };
            c.rigid_other = ma == 0.0 || mb == 0.0;
            c.deep = 0.0;
            if c.rigid_other {
                // the moving body's size: the one that has a mass
                let mover = if ma > 0.0 { c.a } else { c.b };
                if let Some(Some(r)) = self.rbs.get(mover as usize) {
                    c.deep = max(0.25 * r.extent_min, 2.0 * p.linear_slop);
                }
            }
            solver::prepare(c, &sb);
            solve_idx.push(k);
        }
        // the contacts against what does not move last, so their answer stands at the end of each pass
        solve_idx.sort_by_key(|&k| cons[k].rigid_other);
        // the joints that are solved
        let mut jidx: Vec<usize> = Vec::new();
        for (k, s) in self.joints.iter_mut().enumerate() {
            let Some(rt) = s.rt.as_mut() else {
                continue;
            };
            rt.sa = sidx.get(rt.ia as usize).copied().unwrap_or(NONE);
            rt.sb = sidx.get(rt.ib as usize).copied().unwrap_or(NONE);
            if rt.sa == NONE && rt.sb == NONE {
                continue;
            }
            let (Some(Some(a)), Some(Some(b))) = (self.rbs.get(rt.ia as usize), self.rbs.get(rt.ib as usize)) else {
                continue;
            };
            rt.prepare((a.center, a.rot, a.mass.center), (b.center, b.rot, b.mass.center), h);
            jidx.push(k);
        }
        let contact_hz = min(p.contact_hertz, 0.25 * inv_h);
        let soft = Soft::new(contact_hz, p.contact_damping_ratio, h);
        let soft_rigid = Soft::new(2.0 * contact_hz, p.contact_damping_ratio, h);
        let joint_soft = Soft::new(min(p.joint_hertz, 0.5 * inv_h), p.joint_damping_ratio, h);
        let max_push = max(p.max_push_speed, 0.0);
        let slop = 0.0 * max(p.linear_slop, 0.0);
        let max_w = 0.25 * core::f64::consts::PI * inv_h;

        for _ in 0..substeps {
            // velocities
            for (s, e) in sb.iter_mut().zip(extra.iter()) {
                if !e.dynamic {
                    continue;
                }
                let mut v = s.v * e.damp_v + e.accel * h;
                let mut w = s.w;
                if e.gyro {
                    w = gyroscopic(w, s.dq.mul(e.rot0), &e.inertia, &e.inv_inertia, h);
                }
                w = w * e.damp_w + e.alpha * h;
                let vv = v.length_sq();
                if vv > p.max_speed * p.max_speed {
                    v = v * (p.max_speed / sqrt(vv));
                }
                let ww = w.length_sq();
                if ww > max_w * max_w {
                    w = w * (max_w / sqrt(ww));
                }
                s.v = v;
                s.w = w;
            }
            for &k in &jidx {
                if let Some(rt) = self.joints[k].rt.as_mut() {
                    rt.warm_start(&mut sb);
                }
            }
            for &k in &solve_idx {
                solver::warm_start(&cons[k], &mut sb);
            }
            for &k in &jidx {
                if let Some(rt) = self.joints[k].rt.as_mut() {
                    rt.solve(&mut sb, &joint_soft, inv_h, true);
                }
            }
            for &k in &solve_idx {
                let c = &mut cons[k];
                let s = if c.rigid_other { &soft_rigid } else { &soft };
                solver::solve(c, &mut sb, s, inv_h, max_push, slop, true);
            }
            // positions: no body moves faster than max_speed or turns more than an eighth of a turn in a substep,
            // also when the constraints asked for more (conflicting joints on a light body with long levers)
            for (s, e) in sb.iter_mut().zip(extra.iter()) {
                if e.dynamic {
                    let vv = s.v.length_sq();
                    if vv > p.max_speed * p.max_speed {
                        s.v = s.v * (p.max_speed / sqrt(vv));
                    }
                    let ww = s.w.length_sq();
                    if ww > max_w * max_w {
                        s.w = s.w * (max_w / sqrt(ww));
                    }
                }
                s.dp += s.v * h;
                s.dq = Quat::from_scaled_axis(s.w * h).mul(s.dq).normalize();
            }
            for &k in &jidx {
                if let Some(rt) = self.joints[k].rt.as_mut() {
                    rt.solve(&mut sb, &joint_soft, inv_h, false);
                }
            }
            for &k in &solve_idx {
                let c = &mut cons[k];
                let s = if c.rigid_other { &soft_rigid } else { &soft };
                solver::solve(c, &mut sb, s, inv_h, max_push, slop, false);
            }
        }
        for &k in &solve_idx {
            solver::restitution(&mut cons[k], &mut sb, max(p.restitution_threshold, 0.0));
        }

        // the new poses
        let mut swept = 0usize;
        for (k, &i) in active.iter().enumerate() {
            let Some(mut r) = self.rbs[i as usize] else {
                continue;
            };
            let Some((id, body)) = self.world.at(i) else {
                continue;
            };
            let s = sb[k];
            let c0 = r.center;
            r.v = s.v;
            r.w = s.w;
            r.center += s.dp;
            r.rot = s.dq.mul(r.rot).normalize();
            if let Some(t) = r.target.take() {
                r.center = t.transform_point(r.mass.center);
                r.rot = t.rot;
            }
            let motion = r.center - c0;
            let ml = motion.length();
            if r.dynamic() && p.continuous && !r.sensor && ml > 0.0 {
                let rbs = &self.rbs;
                let nc = &self.no_collide;
                let (mask, layers, bullet) = (r.mask, body.layers, r.bullet);
                let pred = |j: u32, other: &crate::world::Body, bullets: bool| {
                    if j == i || other.layers & mask == 0 {
                        return false;
                    }
                    let Some(Some(o)) = rbs.get(j as usize) else {
                        return false;
                    };
                    if o.sensor || layers & o.mask == 0 || nc.binary_search(&(min_u(i, j), max_u(i, j))).is_ok() {
                        return false;
                    }
                    !o.dynamic() || (bullets && bullet && !o.bullet)
                };
                // the sweep of a fast body against what does not move (a bullet: also against what does):
                // when it moved farther than its speculative contacts reached
                if r.bullet || ml > max(r.extent_max, max(p.speculative_distance, 0.0)) {
                    let iso0 = Iso::new(c0 - r.rot.rotate(r.mass.center), r.rot);
                    let hit = self.world.cast_where(&body.shape, &iso0, motion, |j, o| pred(j, o, true));
                    if let Some((_, h)) = hit {
                        if h.fraction > 0.0 && h.fraction < 1.0 {
                            // a slop off the surface (along its normal, so a body that grazes it keeps its
                            // way): the next step meets it with a gap, which its speculative contact
                            // closes exactly
                            r.center = c0 + motion * h.fraction + h.normal * max(p.linear_slop, 0.0);
                            swept += 1;
                        }
                    }
                }
                // the centre of mass never passes through what does not move: a body that turned into the
                // ground faster than its contacts could follow still has its centre on the right side, so
                // its contacts push it out (a two-sided triangle would otherwise push it on through)
                let motion = r.center - c0;
                let ml = motion.length();
                if ml > 0.0 {
                    if let Some((_, h)) = self.world.ray_cast_where(c0, motion, 1.0, |j, o| pred(j, o, false)) {
                        if h.t > 0.0 && h.t < 1.0 {
                            r.center = c0 + motion * max(h.t - max(p.linear_slop, 0.0) / ml, 0.0);
                            swept += 1;
                        }
                    }
                }
            }
            r.force = Vec3::ZERO;
            r.torque = Vec3::ZERO;
            let iso = r.iso();
            self.rbs[i as usize] = Some(r);
            self.world.set_iso(id, iso);
        }
        self.stats.swept = swept;

        // sleep
        self.sleep(dt, &active, &cons, &sidx);

        // events
        self.contact_events(&cons, &order, &sensors);
        self.constraints = cons;
        self.order = order;
        self.sensors = sensors;
        self.update_stats(&active);
    }

    /// islands that were slow long enough fall asleep
    fn sleep(&mut self, dt: f64, active: &[u32], cons: &[Constraint], sidx: &[u32]) {
        let p = self.params;
        let n = active.len();
        let mut kin_moving = vec![false; n];
        for (k, &i) in active.iter().enumerate() {
            let Some(r) = self.rbs[i as usize].as_mut() else {
                continue;
            };
            match r.kind {
                BodyKind::Dynamic => {
                    let speed = r.v.length() + r.w.length() * r.extent_max;
                    if p.sleep && r.can_sleep && speed < p.sleep_speed {
                        r.sleep_time += dt;
                    } else {
                        r.sleep_time = 0.0;
                    }
                }
                BodyKind::Kinematic => {
                    if r.v == Vec3::ZERO && r.w == Vec3::ZERO {
                        r.awake = false;
                    } else {
                        kin_moving[k] = true;
                    }
                }
                BodyKind::Static => {}
            }
        }
        if !p.sleep {
            return;
        }
        // islands over the dynamic bodies of this step
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                let g = parent[parent[x as usize] as usize];
                parent[x as usize] = g;
                x = g;
            }
            x
        }
        let unite = |parent: &mut Vec<u32>, x: u32, y: u32| {
            let (rx, ry) = (find(parent, x), find(parent, y));
            if rx != ry {
                let (lo, hi) = if rx < ry { (rx, ry) } else { (ry, rx) };
                parent[hi as usize] = lo;
            }
        };
        let dyn_s = |s: u32, rbs: &Vec<Option<Rb>>| s != NONE && rbs[active[s as usize] as usize].is_some_and(|r| r.dynamic());
        for c in cons {
            let (sa, sb) = (sidx.get(c.a as usize).copied().unwrap_or(NONE), sidx.get(c.b as usize).copied().unwrap_or(NONE));
            if sa == NONE && sb == NONE {
                continue;
            }
            // a body on a moving kinematic one stays awake
            for (x, y) in [(sa, sb), (sb, sa)] {
                if x != NONE && y != NONE && kin_moving[y as usize] {
                    if let Some(r) = self.rbs[active[x as usize] as usize].as_mut() {
                        r.sleep_time = 0.0;
                    }
                }
            }
            if dyn_s(sa, &self.rbs) && dyn_s(sb, &self.rbs) {
                unite(&mut parent, sa, sb);
            }
        }
        for s in &self.joints {
            if let Some(rt) = &s.rt {
                let (sa, sb) = (sidx.get(rt.ia as usize).copied().unwrap_or(NONE), sidx.get(rt.ib as usize).copied().unwrap_or(NONE));
                if dyn_s(sa, &self.rbs) && dyn_s(sb, &self.rbs) {
                    unite(&mut parent, sa, sb);
                }
            }
        }
        let mut min_time = vec![f64::INFINITY; n];
        for k in 0..n {
            let i = active[k];
            let Some(r) = self.rbs[i as usize].as_ref() else {
                continue;
            };
            if !r.dynamic() {
                continue;
            }
            let root = find(&mut parent, k as u32) as usize;
            min_time[root] = min(min_time[root], r.sleep_time);
        }
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); n];
        for k in 0..n {
            let i = active[k];
            if self.rbs[i as usize].is_some_and(|r| r.dynamic()) {
                let root = find(&mut parent, k as u32) as usize;
                if min_time[root] >= p.time_to_sleep {
                    members[root].push(i);
                }
            }
        }
        for m in members {
            if !m.is_empty() {
                self.put_to_sleep(&m);
            }
        }
    }

    fn contact_events(&mut self, cons: &[Constraint], order: &[u32], sensors: &[(u32, u32)]) {
        let slop = self.params.linear_slop;
        // touching pairs now (by pair), with the point that approached fastest
        let mut now: Vec<((u32, u32), Vec3, Vec3, f64)> = Vec::new();
        for &k in order {
            let c = &cons[k as usize];
            let mut touch = false;
            let mut best: Option<(Vec3, f64)> = None;
            for p in &c.points[..c.count] {
                if p.max_normal_impulse > 0.0 || p.separation <= slop {
                    touch = true;
                    let speed = max(-p.approach, 0.0);
                    if best.is_none_or(|b| speed > b.1) {
                        best = Some((p.point, speed));
                    }
                }
            }
            if touch {
                let (pt, sp) = best.unwrap_or((Vec3::ZERO, 0.0));
                match now.last_mut() {
                    Some(last) if last.0 == (c.a, c.b) => {
                        if sp > last.3 {
                            *last = ((c.a, c.b), pt, c.normal, sp);
                        }
                    }
                    _ => now.push(((c.a, c.b), pt, c.normal, sp)),
                }
            }
        }
        let ids = |s: u32| self.world.at(s).map(|x| x.0);
        let mut events: Vec<ContactEvent> = Vec::new();
        for (pair, pt, n, sp) in &now {
            if self.touching.binary_search(pair).is_err() {
                if let (Some(a), Some(b)) = (ids(pair.0), ids(pair.1)) {
                    events.push(ContactEvent::Begin { a, b, point: *pt, normal: *n, speed: *sp });
                }
            }
        }
        let now_pairs: Vec<(u32, u32)> = now.iter().map(|x| x.0).collect();
        for pair in &self.touching {
            if now_pairs.binary_search(pair).is_err() {
                if let (Some(a), Some(b)) = (ids(pair.0), ids(pair.1)) {
                    events.push(ContactEvent::End { a, b });
                }
            }
        }
        for pair in sensors {
            if self.sensors.binary_search(pair).is_err() {
                if let (Some(s), Some(o)) = (ids(pair.0), ids(pair.1)) {
                    events.push(ContactEvent::SensorBegin { sensor: s, other: o });
                }
            }
        }
        for pair in &self.sensors {
            if sensors.binary_search(pair).is_err() {
                if let (Some(s), Some(o)) = (ids(pair.0), ids(pair.1)) {
                    events.push(ContactEvent::SensorEnd { sensor: s, other: o });
                }
            }
        }
        self.events.extend(events);
        self.touching = now_pairs;
    }

    fn update_stats(&mut self, active: &[u32]) {
        let mut s = SimStats { pairs: self.stats.pairs, swept: self.stats.swept, ..SimStats::default() };
        s.bodies = self.world.len();
        s.awake = active.iter().filter(|&&i| self.rbs[i as usize].is_some_and(|r| r.awake)).count();
        s.manifolds = self.constraints.len();
        s.points = self.constraints.iter().map(|c| c.count).sum();
        s.joints = self.joints.iter().filter(|j| j.rt.is_some()).count();
        s.sleeping_islands = self.groups.iter().filter(|g| !g.is_empty()).count();
        self.stats = s;
    }

    /// the events of the last step (contacts that began and ended, sensors entered and left)
    pub fn events(&self) -> &[ContactEvent] {
        &self.events
    }

    /// the counters of the last step
    pub fn stats(&self) -> SimStats {
        self.stats
    }

    /// the manifolds of the last step (also of islands asleep), by body
    pub fn manifolds(&self) -> impl Iterator<Item = ContactManifold> + '_ {
        self.order.iter().filter_map(|&k| {
            let c = &self.constraints[k as usize];
            let (a, _) = self.world.at(c.a)?;
            let (b, _) = self.world.at(c.b)?;
            let mut m = ContactManifold { a, b, normal: c.normal, triangle: c.key, count: c.count, points: [ContactPoint::default(); MAX_POINTS] };
            for (k, p) in c.points[..c.count].iter().enumerate() {
                m.points[k] = ContactPoint { point: p.point, separation: p.separation, normal_impulse: p.normal_impulse };
            }
            Some(m)
        })
    }
}

/// the indices of constraints sorted by (a, b, key) (unique: a pair's manifolds have different keys)
fn sort_order(cons: &[Constraint]) -> Vec<u32> {
    let mut keys: Vec<(u32, u32, u32, u32)> = cons.iter().enumerate().map(|(i, c)| (c.a, c.b, c.key, i as u32)).collect();
    keys.sort_unstable();
    keys.into_iter().map(|k| k.3).collect()
}

#[inline]
fn min_u(a: u32, b: u32) -> u32 {
    if a < b { a } else { b }
}

#[inline]
fn max_u(a: u32, b: u32) -> u32 {
    if a > b { a } else { b }
}
