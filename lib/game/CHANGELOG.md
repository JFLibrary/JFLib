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
