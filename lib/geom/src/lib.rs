// geom -- the modelling kernel of the game engine (docs/GEOM.md, GAME.md stage G7g): indexed triangle meshes
// built from primitives, profiles and operations, checked, the same bits on every target
//
//   Mesh           positions, normals, UV (f64), indices and points (the topology: a hard edge does not
//                  open a surface, touching parts stay apart); transformed, translated, rotated (degrees,
//                  exact at multiples of 90), scaled, mirrored, flipped, merge, array, radial, weld; flat,
//                  smooth (angle threshold), uv_box (box projection in metres); bounds, area, volume,
//                  fingerprint; check() -> Report
//   build          cuboid, cylinder, cone, sphere, torus, stairs, arch (standing on y = 0, centred over
//                  the origin), extrude (front-view profile along Z), prism (floor plan up along Y), sweep
//                  (profile along a path, mitred corners, rotation-minimising frames), lathe (profile about Y)
//   poly           2D profiles: area, cleaning, triangulation (ear clipping), rect, circle, arc, rounded_rect
//   Solid          convex cells as brushes: from_convex, union, difference, intersection (exact decisions:
//                  points are where three planes meet, sides are signs of determinants), to_mesh (closed,
//                  corners rounded once at the end), bevelled / bevel (chamfer or round edges and corners)
//   exact          expansions (Shewchuk), orient2d, the side of a plane at a meeting of three planes,
//                  the correctly rounded quotient
//   Report         closed, manifold, outward, degenerate triangles, normals, bounds, area, volume, Euler
//                  characteristic, components; json()
//   math           Vec3, Aabb, Affine in f64; sqrt correctly rounded, sin / cos / atan2 from rt's portable
//                  math, sin_cos_deg exact at multiples of 90 degrees
//
// Metres, right-handed, Y up, faces counter-clockwise from outside (render3d, lib/physics, glTF).  A script
// describes a model (apps/modeltool binds JFScript), this crate does the geometry: it has a plain Rust API and
// knows no script.  Deterministic: only + - * / and correctly rounded square roots in f64, trigonometry from
// rt's portable math, sorting with unique keys, no unordered containers, no threads, so the same calls give the
// same mesh bits on every target (the checksum of apps/modeltool).  Error texts are German, like JFScript's: they
// end up in front of whoever wrote the script.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

/// the generators: primitives, extrude, prism, sweep, lathe
pub mod build;
mod check;
mod csg;
/// exact arithmetic: expansions, orientation, planes meeting in a point, correctly rounded quotients
pub mod exact;
/// Vec3, Aabb, Affine in f64 and the scalar functions, the same bits on every target
pub mod math;
mod mesh;
mod normals;
/// 2D profiles: area, orientation, triangulation and the profiles of the modelling API
pub mod poly;

pub use build::{DEFAULT_SMOOTH_DEG, arch, cone, cuboid, cylinder, extrude, lathe, prism, sphere, stairs, sweep, torus};
pub use check::{DEGENERATE_RATIO, NORMAL_TOLERANCE, Report, fnum};
pub use csg::{MAX_CELLS, Plane, Solid, WELD_RATIO, bevel, difference, intersection, union};
pub use math::{Aabb, Affine, Axis, Vec3};
pub use mesh::{Fnv, Mesh};

use alloc::string::String;

/// The most triangles a mesh may have (2^24): a limit against runaway parameters, far above a game model.
pub const MAX_TRIANGLES: usize = 1 << 24;

/// The most segments of a round primitive, a lathe or stairs.
pub const MAX_SEGMENTS: usize = 4096;

/// The most copies of an array.
pub const MAX_COPIES: usize = 100_000;

/// Why an operation made no mesh; the text (German) says what to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// a parameter out of range: a length not above 0, a segment count, an arch that does not fit
    Param(String),
    /// a profile with too few points, no area, crossing edges or one that cannot be triangulated
    Profile(String),
    /// a path with too few points or one that turns back on itself
    Path(String),
    /// more triangles, points or copies than the limits allow
    TooBig(String),
    /// arrays of a mesh that do not fit together (from_triangles, validate)
    Invalid(String),
}

impl Error {
    /// the text
    pub fn message(&self) -> &str {
        match self {
            Error::Param(s) | Error::Profile(s) | Error::Path(s) | Error::TooBig(s) | Error::Invalid(s) => s,
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(self.message())
    }
}
