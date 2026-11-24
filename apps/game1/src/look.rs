// look -- how game 1 shows its figures, without a GPU: the figure baked from content/figur.jfs, its rigid parts posed
// (legs swing about the hips, arms, hands and weapon turn with the view's pitch about the shoulder, the head
// follows half of it, crouching squashes the legs and lowers the rest), the weapon in front of the camera (with
// bobbing and a kick), the muzzle in the world and in the camera's space, the players' colours
//
//   FIGURE           the baked figure (.g1m)
//   Pose             what a frame knows of a figure: feet, yaw, pitch, crouched, on the ground, walk phase, dead
//   part_transform   a part's transform in the world (game::Transform: position, rotation, scale)
//   VM_LAYOUT        the weapon's size and place in front of the camera (one of VM_VARIANTS), right and down on
//                    every shape of window
//   viewmodel        a viewmodel part's transform in the camera's space; viewmodel_point the muzzle there,
//                    vm_to_world the point of the world shown at the same place (the own tracer's start)
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

/// the viewmodel's lens (render3d::Viewmodel's vertical field of view): its own, the game's field of view does not
/// change the weapon
pub const VM_FOV: f64 = 1.0;
/// how far in front of the eye the weapon's anchor sits (m)
const VM_DEPTH: f64 = 0.62;
/// below this width / height the weapon's measure is the window's width (a narrow or tall window: it shrinks), at
/// and above it the height
const VM_NARROW: f64 = 4.0 / 3.0;
/// the scale the motions' amplitudes (bobbing, kick, lowering) were tuned at: they grow and shrink with the weapon
pub const VM_TUNED: f64 = 0.55;

/// the size and place of the weapon in front of the camera: its scale, and where its anchor (the middle of the
/// weapon's box) sits on the screen, `right` from the right edge and `bottom` above the lower edge, both in screen
/// heights (on a window narrower than 4:3 in widths / (4/3): it shrinks with the width and stays in the corner)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VmLayout {
    pub scale: f64,
    pub right: f64,
    pub bottom: f64,
}

/// the variants of 2026-11-24, 0.6, 0.7 and 0.8 times the weapon of G7h part 1 (scale 0.55, anchored in the
/// camera's space at x 0.15, y -0.16: on 16:9 a little right of and below the crosshair), each placed so that its
/// box ends a little below the lower edge and on 16:9 to 21:9 left of the ammunition (0.21 to 0.22 screen heights
/// from the right edge on 16:9; docs/GAME1.md, "Die Waffe vor der Kamera"); VM_LAYOUT is the one the game draws
pub const VM_VARIANTS: [(&str, VmLayout); 3] = [
    ("A", VmLayout { scale: 0.33, right: 0.36, bottom: 0.12 }),
    ("B", VmLayout { scale: 0.385, right: 0.39, bottom: 0.14 }),
    ("C", VmLayout { scale: 0.44, right: 0.42, bottom: 0.17 }),
];
pub const VM_LAYOUT: VmLayout = VM_VARIANTS[1].1;

/// a sane width / height (16:9 for none)
fn aspect_or(aspect: f64) -> f64 {
    if aspect.is_finite() && aspect > 0.0 { aspect } else { 16.0 / 9.0 }
}

/// the weapon's measure on a window of this width / height, in its heights: 1, or less on a narrow one
pub fn vm_fit(aspect: f64) -> f64 {
    (aspect_or(aspect) / VM_NARROW).min(1.0)
}

/// the weapon's scale on a window of this width / height
pub fn vm_scale(aspect: f64) -> f64 {
    VM_LAYOUT.scale * vm_fit(aspect)
}

fn tan_half(fov: f64) -> f64 {
    let (s, c) = physics::math::sincos(fov * 0.5);
    s / c
}

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

/// where the anchor rests in the camera's space on a window of this width / height (VmLayout): the lens shows a
/// point at depth d with x / (d tan(fov/2)) of the half height and of aspect half heights across
fn vm_rest(aspect: f64) -> Vec3 {
    let a = aspect_or(aspect);
    let k = vm_fit(a);
    let t = tan_half(VM_FOV) * VM_DEPTH;
    Vec3::new((a - 2.0 * VM_LAYOUT.right * k) * t, (2.0 * VM_LAYOUT.bottom * k - 1.0) * t, -VM_DEPTH)
}

/// the viewmodel's transform (and its scale in f64): see viewmodel
fn vm_transform(anchor: Vec3, v: &VmState, aspect: f64) -> (Transform, f64) {
    let scale = vm_scale(aspect);
    let m = scale / VM_TUNED;
    let (s, c) = physics::math::sincos(v.bob);
    let bob = Vec3::new(s * 0.012 * v.bob_amount, -(c * c) * 0.010 * v.bob_amount, 0.0) * m;
    let back = Vec3::new(0.0, -0.10 * v.reload, 0.06 * v.kick) * m;
    let rx = rot_x(0.10 * v.kick - 0.6 * v.reload);
    let ry = Quat::from_axis_angle(Vec3::Y, core::f64::consts::PI);
    // on a window wider than 16:9 the anchor lies further off the axis: the weapon turns outward by as much more
    // (about its anchor, around the camera's up), so the eye sees the same side of it as on 16:9 and its back does
    // not spread towards the ammunition
    let rest = vm_rest(aspect);
    let wide = physics::math::atan2(rest.x, VM_DEPTH) - physics::math::atan2(vm_rest(16.0 / 9.0).x, VM_DEPTH);
    let turn = Quat::from_axis_angle(Vec3::Y, -wide.max(0.0));
    let rot = turn.mul(ry.mul(rx)).normalize();
    // p_cam = rest + bob + back + rot (s (q - anchor))
    (Transform { pos: rest + bob + back - rot.rotate(anchor * scale), rot, scale: [scale as f32; 3] }, scale)
}

/// a viewmodel part's transform in the camera's space (x right, y up, -z forward) on a window of this width /
/// height; anchor: the weapon's middle (VmLayout: right and down in every shape of window)
pub fn viewmodel(anchor: Vec3, v: &VmState, aspect: f64) -> Transform {
    vm_transform(anchor, v, aspect).0
}

/// a point of the model (q) in the camera's space through the viewmodel's transform
pub fn viewmodel_point(anchor: Vec3, v: &VmState, aspect: f64, q: Vec3) -> Vec3 {
    let (t, scale) = vm_transform(anchor, v, aspect);
    t.pos + t.rot.rotate(q * scale)
}

/// where the weapon in front of the camera points (the model's forward, +Z, in the camera's space): its muzzle
/// flash's direction
pub fn viewmodel_dir(anchor: Vec3, v: &VmState, aspect: f64) -> Vec3 {
    vm_transform(anchor, v, aspect).0.rot.rotate(Vec3::new(0.0, 0.0, 1.0))
}

/// a point of the viewmodel's camera space (seen through its lens VM_FOV) -> the point of the world that the
/// camera at eye, turned by yaw and pitch, with the vertical field of view fov shows at the same place of the
/// screen, at the same depth: where the own tracer starts (the muzzle on the screen, on every window and field of
/// view)
pub fn vm_to_world(p: Vec3, eye: Vec3, yaw: f64, pitch: f64, fov: f64) -> Vec3 {
    let (dir, right, up) = game::camera::basis(yaw, pitch);
    let k = tan_half(fov) / tan_half(VM_FOV);
    eye + right * (p.x * k) + up * (p.y * k) + dir * -p.z
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
