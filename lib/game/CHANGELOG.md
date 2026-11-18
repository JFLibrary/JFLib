# Changelog of jflib-game

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The game core (G7b part 1): `World` (entities with a generation: a stale id is refused or finds nothing; components in dense columns per type, `SparseSet` with a paged index, a version per change and `changed_since`, `Mut` stamps only on a write; `spawn_with`, `despawn`, `insert`, `remove`, `get`, `get_mut`, `column`, `column_mut`, `pair_mut`, `sort_columns` for the canonical order; no archetypes, no scheduler, no hash maps), `Pose` (a `Transform` in f64 now and at the start of the tick, `interpolate(alpha)`, `teleport`; `World::begin_tick` settles only the poses written in the tick before), `FixedTick` (60 Hz unless set, an exact accumulator in nanoseconds times the rate, a cap on catch-up steps, `alpha`, time from outside), `Systems` (functions in a fixed order), `Game` (world, resources, systems, clock). `Input`: `ActionMap` with Button, Axis and Motion actions bound to keys by their place (`Source::Key(KC_*)`), mouse buttons, the wheel and the raw motion; `MouseSettings` (sensitivity in steps of 0.022 degrees a count, Quake's and Source's m_yaw; inversion, a factor per platform: `platform_motion_factor`, `FACTOR_*`); the click that asks for the pointer lock fires nothing, the end of the lock is the game's pause; presses counted per frame and per tick (`tick() -> TickInput`); capture for a settings page; `plat_event` for a game with its own window; `store` / `load` in a `base::Ini` (the free part of `base::Settings`). Cameras in f64: `FirstPerson`, `Follow` (an orbit, `view_in` pulls the eye in front of walls through lib/physics), `FreeFly`, `View`. docs/GAMECORE.md

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
- The netcode (G7e part 1, `game::net`, over lib/netchan): `NetServer` and the trait `RoomSim` (rooms by code, a fixed tick, every player's input of the tick or the last one repeated, snapshots every two ticks as deltas against what each client holds: the newest it confirmed over UDP, the last sent over a WebSocket, the map's baseline before; quantised fields, changes ordered by an accumulated priority and cut to 1200 bytes since netchan does not fragment; a join's full state that does not fit a datagram is fetched over a WebSocket of its own with a ticket); `NetClient` (ticks ahead of the server on a clock steered by the margin of its inputs, inputs sent with the ones the server may lack, prediction and reconciliation with `Movement` and `Predictor`, the others interpolated 100 ms behind the newest snapshot); lag compensation (`HitHistory`, `view_time`, `hitscan`: the others rewound to what the shooter saw, at most 200 ms); `RoomCode`, room links, names; the bit stream, quantisation grids, the messages; `sim::SimNet` (netchan's cores on a virtual clock with delay, jitter, loss and reordering); `link::serve` and `NetLink` on real sockets; a reference game (`walker::Walker`, `arena::Arena`, `bot::Bot`). Everything a client predicts computes the same bits as the server. docs/NETCODE.md

