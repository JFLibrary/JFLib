// look -- how game 1 shows its figures, without a GPU: the figure baked from content/figur.jfs, its rigid parts posed
// (legs swing about the hips, arms, hands and weapon turn with the view's pitch about the shoulder, the head
// follows half of it, crouching squashes the legs and lowers the rest), the weapon in front of the camera (with
// bobbing and a kick), the muzzle in the world and in the camera's space, the players' colours
//
//   FIGURE           the baked figure (.g1m)
//   Pose             what a frame knows of a figure: feet, yaw, pitch, crouched, on the ground, walk phase, dead
//   part_transform   a part's transform in the world (game::Transform: position, rotation, scale)
//   viewmodel        a viewmodel part's transform in the camera's space; muzzle_cam the muzzle there
//   PALETTE          the jacket's colour per player (linear rgb)
//
// The figure looks along +Z (modeltool's front); yaw 0 looks along -Z, so a figure turns by yaw + pi.

use game::{Quat, Transform, Vec3};
use game1_core::map::ModelData;

/// the baked figure (content/figur.jfs)
pub const FIGURE: &[u8] = include_bytes!("../content/figur.g1m");

/// the jacket of each player (linear rgb), by net id
pub const PALETTE: [[f32; 3]; 8] = [
    [0.80, 0.25, 0.03],
    [0.04, 0.22, 0.80],
    [0.08, 0.52, 0.10],
    [0.42, 0.08, 0.62],
    [0.86, 0.62, 0.03],
    [0.03, 0.55, 0.52],
    [0.72, 0.05, 0.06],
    [0.86, 0.26, 0.48],
];

/// what a frame knows of a figure
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub feet: Vec3,
    pub yaw: f64,
    pub pitch: f64,
    pub crouched: bool,
    pub grounded: bool,
    /// the walk cycle (radians) and how strongly it swings (0 ..= 1)
    pub phase: f64,
    pub swing: f64,
}

/// which group a part belongs to: how it moves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    LegL,
    LegR,
    Body,
    Head,
    Arms,
}

/// the group of a part by its name
pub fn group(name: &str) -> Group {
    match name {
        "bein_l" => Group::LegL,
        "bein_r" => Group::LegR,
        "kopf" | "muetze" | "augen" => Group::Head,
        "arm_l" | "arm_r" | "haende" | "waffe" | "tank" => Group::Arms,
        _ => Group::Body,
    }
}

/// the parts drawn in front of the camera
pub fn in_viewmodel(name: &str) -> bool {
    matches!(name, "waffe" | "tank" | "haende")
}

const HIP_Y: f64 = 0.82;
const SHOULDER: Vec3 = Vec3 { x: 0.0, y: 1.3, z: 0.0 };
const NECK: Vec3 = Vec3 { x: 0.0, y: 1.42, z: 0.0 };
/// the crouched capsule is this much lower
const CROUCH_DROP: f64 = 0.6;

fn rot_x(a: f64) -> Quat {
    Quat::from_axis_angle(Vec3::X, a)
}

/// a part of group g of the figure in pose p: its world transform
pub fn part_transform(g: Group, p: &Pose) -> Transform {
    let mut scale = [1.0f32; 3];
    let (pivot, angle, drop) = match g {
        Group::LegL | Group::LegR => {
            let side = if g == Group::LegL { 1.0 } else { -1.0 };
            let swing = if p.grounded { physics::math::sin(p.phase) * 0.7 * p.swing * side } else { -0.35 };
            if p.crouched {
                scale[1] = ((HIP_Y - CROUCH_DROP) / HIP_Y) as f32;
            }
            (Vec3::new(0.0, HIP_Y, 0.0), swing, 0.0)
        }
        Group::Body => (Vec3::ZERO, 0.0, if p.crouched { CROUCH_DROP } else { 0.0 }),
        Group::Head => (NECK, -p.pitch * 0.4, if p.crouched { CROUCH_DROP } else { 0.0 }),
        Group::Arms => (SHOULDER, -p.pitch, if p.crouched { CROUCH_DROP } else { 0.0 }),
    };
    let rx = rot_x(angle);
    // local: pivot + Rx (S (q - pivot)) - drop = Rx (S q) + (pivot - Rx (S pivot)) - drop
    let sp = Vec3::new(pivot.x * scale[0] as f64, pivot.y * scale[1] as f64, pivot.z * scale[2] as f64);
    let local = pivot - rx.rotate(sp) - Vec3::new(0.0, drop, 0.0);
    let ry = Quat::from_axis_angle(Vec3::Y, p.yaw + core::f64::consts::PI);
    Transform { pos: p.feet + ry.rotate(local), rot: ry.mul(rx).normalize(), scale }
}

/// where a point of the figure's model (a part of group g) is in the world
pub fn model_point(g: Group, p: &Pose, q: Vec3) -> Vec3 {
    let t = part_transform(g, p);
    t.pos + t.rot.rotate(Vec3::new(q.x * t.scale[0] as f64, q.y * t.scale[1] as f64, q.z * t.scale[2] as f64))
}

/// the weapon in front of the camera: where its anchor (the middle of the weapon's box) sits in the camera's space,
/// and its size there
const VM_OFFSET: Vec3 = Vec3 { x: 0.15, y: -0.16, z: -0.62 };
const VM_SCALE: f64 = 0.55;

/// what moves the weapon in front of the camera
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VmState {
    /// the kick of the last shot (0 ..= 1, decays)
    pub kick: f64,
    /// the walk cycle and its strength
    pub bob: f64,
    pub bob_amount: f64,
    /// lowered while reloading (0 ..= 1)
    pub reload: f64,
}

/// a viewmodel part's transform in the camera's space (x right, y up, -z forward); anchor: the weapon's middle
pub fn viewmodel(anchor: Vec3, v: &VmState) -> Transform {
    let (s, c) = physics::math::sincos(v.bob);
    let bob = Vec3::new(s * 0.012 * v.bob_amount, -(c * c) * 0.010 * v.bob_amount, 0.0);
    let back = Vec3::new(0.0, -0.10 * v.reload, 0.06 * v.kick);
    let rx = rot_x(0.10 * v.kick - 0.6 * v.reload);
    let ry = Quat::from_axis_angle(Vec3::Y, core::f64::consts::PI);
    let rot = ry.mul(rx).normalize();
    // p_cam = offset + bob + back + rot (s (q - anchor))
    Transform { pos: VM_OFFSET + bob + back - rot.rotate(anchor * VM_SCALE), rot, scale: [VM_SCALE as f32; 3] }
}

/// a point of the model (q) in the camera's space through the viewmodel's transform
pub fn viewmodel_point(anchor: Vec3, v: &VmState, q: Vec3) -> Vec3 {
    let t = viewmodel(anchor, v);
    t.pos + t.rot.rotate(q * VM_SCALE)
}

/// the anchor of the viewmodel and the muzzle of a baked figure (model space)
pub fn weapon_points(m: &ModelData) -> (Vec3, Vec3) {
    let anchor = m.part("waffe").map_or(Vec3::new(0.12, 1.2, 0.5), |p| {
        let (lo, hi) = p.bounds();
        Vec3::new((lo[0] as f64 + hi[0] as f64) * 0.5, (lo[1] as f64 + hi[1] as f64) * 0.5, (lo[2] as f64 + hi[2] as f64) * 0.5)
    });
    let muzzle = m.marker("muendung").map_or(Vec3::new(-0.12, 1.23, 0.96), |k| k.centre());
    (anchor, muzzle)
}
