// net -- the netcode (docs/GAME.md G7e, docs/NETCODE.md): the server decides, clients predict their own
// figure and see the others interpolated, hits are lag-compensated; over lib/netchan (UDP or WebSocket)
//
//   server     NetServer and the trait RoomSim: rooms by code, the fixed tick, inputs per tick, snapshots
//              as deltas against what each client holds, cut to the budget by priority, the full state of
//              a join over a WebSocket when it does not fit a datagram
//   client     NetClient: joins, runs ahead of the server (clock::SteeredClock), predicts and reconciles
//              (predict), decodes snapshots and shows the others behind the newest (clock::InterpClock)
//   snapshot   NetState, Snapshot, Schema; diff, select, encode, decode
//   input      NetInput, the redundant input packets, the server's InputBuffer
//   lagcomp    HitHistory, view_time, hitscan
//   room       RoomCode, links, names
//   proto      the messages
//   quant      the grids: positions, angles, axes, rotations
//   bits       the bit stream
//   sim        a simulated network for tests (netchan's cores on a virtual clock)
//   link       real sockets: serve() for a server's loop, NetLink for a client
//   walker, arena, bot   the reference movement, game and players of the tests (G7h replaces them)
//
// Everything a client predicts runs the same bits as the server: the movement state is integers on a
// grid, the movement uses + - * /, sqrt and lib/physics' sin, cos and atan2, nothing iterates a hash map,
// nothing reads a clock (time comes from outside).

/// the arena: the reference game of the netcode's tests
pub mod arena;
/// the bit stream of the messages
pub mod bits;
/// bots: players without a person
pub mod bot;
/// the netcode's client
pub mod client;
/// the client's clocks
pub mod clock;
/// inputs and their redundancy, the server's input buffer
pub mod input;
/// lag compensation and hitscan
pub mod lagcomp;
/// real sockets
pub mod link;
/// prediction and reconciliation
pub mod predict;
/// the messages
pub mod proto;
/// quantisation
pub mod quant;
/// rooms, codes, links, names
pub mod room;
/// the netcode's server
pub mod server;
/// a simulated network
pub mod sim;
/// snapshots and their delta encoding
pub mod snapshot;
/// the reference movement
pub mod walker;

pub use client::{ClientOptions, ClientStats, NetClient, Phase, Remote};
pub use input::NetInput;
pub use predict::{Movement, Outcome, Predictor};
pub use room::RoomCode;
pub use server::{LinkStats, NetServer, RoomSim, ServerOptions, ServerStats};
pub use snapshot::{tick_diff, Ent, NetState, Schema, Snapshot};
