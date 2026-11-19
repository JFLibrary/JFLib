// game1_core -- game 1's logic without a GPU, shared by the client (apps/game1) and the server (apps/game1-server):
// the rules, the baked map, the movement behind game::net's Movement (lib/physics' CharacterController and the
// weapon's timing), the room (game::net's RoomSim: free-for-all, one hitscan weapon, lag compensation), the bots,
// and on the desktop the server's loop on real sockets.  It never names render3d or gpu: the server stays a static
// Linux program without libc (docs/GAME1.md, "Crates").
//
//   rules      the numbers: tick, buttons, weapon, health, the replicated entities, the protocol id
//   map        the .g1m format (models and maps baked from JFScript by modeltool), Map with its physics world
//   movement   MoveState, Mover (Movement)
//   sim        Match (RoomSim), Figure, Event
//   bot        Brain: a server-side player
//   host       the server on a netchan::Server (Windows, Linux, Apple)
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

/// server-side bots
pub mod bot;
/// the server on real sockets
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
pub mod host;
/// the baked maps and models
pub mod map;
/// a figure's movement and weapon
pub mod movement;
/// game 1's numbers
pub mod rules;
/// a room: free-for-all with one weapon
pub mod sim;
