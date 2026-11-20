// game -- the core of the game engine (docs/GAME.md, G7b): a world of entities and components, a fixed
// tick, input actions, camera controllers; without a GPU, so a game's server runs it headless
//
//   World          entities with a generation (stale ids are refused), components in dense columns per
//                  type (SparseSet: sparse sets, versions of every change), spawn_with / despawn /
//                  insert / remove / get / get_mut / column / pair_mut; begin_tick settles the poses
//   Pose           where an entity is (Transform: f64 position, rotation, scale), now and at the start of
//                  the tick: a frame shows interpolate(alpha)
//   FixedTick      the fixed rate (60 Hz unless set), an exact accumulator over time from outside, a cap
//                  on catch-up steps, alpha; Systems (functions in a fixed order), Game (all of it)
//   Input          ActionMap (Button, Axis, Motion actions bound to keys by their place, mouse buttons,
//                  the wheel, the raw motion), MouseSettings (sensitivity, inversion, a factor per
//                  platform), the pointer lock's click and end; store / load in a settings file (base::Ini)
//   camera         FirstPerson, Follow (an orbit, pulled in front of walls through lib/physics), FreeFly;
//                  View (eye, forward, up)
//   assets         (G7b part 2) Assets<T>: files through the VFS decoded in the background (plat's blocking
//                  pool, or a budget per frame), Handle<T> with a state (Loading, Ready, Failed), counted
//                  references, unloading; the kinds are types (Asset::decode; glTF and images in game_render3d)
//   audio          (G7b part 2) SpatialAudio over plat::audio's mixer: a listener (the camera's View), Emitter
//                  components and sounds at points, the gain by distance (Attenuation), the pan by direction
//
// render3d is not a dependency: the bridge that puts entities with a mesh into a render3d scene is the
// crate game_render3d.  Everything here is deterministic: no unordered containers, no threads, no clock
// of its own (time comes from outside), floats only through + - * / and lib/physics' math (its correctly
// rounded square root, rt's portable sin and cos).  The one exception is assets: with Loading::Background
// an asset becomes Ready when its thread is done (what a frame shows, never what the simulation computes).
// docs/GAMECORE.md describes the design and the tests (apps/gametest).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

/// (G7b part 2) files decoded in the background behind handles: states, references, unloading
pub mod assets;
/// (G7b part 2) spatial audio: a listener, emitters, attenuation by distance, pan by direction
pub mod audio;
/// camera controllers: first person, follow, free fly
pub mod camera;
/// actions, bindings, mouse settings, the pointer lock's click and end
pub mod input;
/// the netcode: server, client, prediction, snapshots, lag compensation, rooms (docs/NETCODE.md)
pub mod net;
mod pose;
mod sparse;
mod tick;
mod world;

pub use assets::{Asset, AssetId, Assets, Handle, Loading};
pub use audio::{Attenuation, Emitter, SpatialAudio};
pub use camera::{FirstPerson, Follow, FreeFly, View};
pub use input::{ActionId, ActionKind, ActionMap, ActionState, Bind, Device, Input, MouseSettings, PadSettings, Source, TickInput};
pub use pose::{Pose, Transform, nlerp};
pub use sparse::{Mut, SparseSet};
pub use tick::{DEFAULT_MAX_STEPS, DEFAULT_RATE, FixedTick, Game, Step, System, Systems};
pub use world::{Bundle, Entities, Entity, World};

/// the vector and rotation types positions and turns are made of (lib/physics')
pub use physics::{Quat, Vec3};
