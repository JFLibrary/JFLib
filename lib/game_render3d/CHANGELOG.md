# Changelog of jflib-game-render3d

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The bridge from lib/game to render3d (G7b part 1): `MeshRef` (a mesh and a material), `Bridge::sync` (new MeshRefs added, switched meshes and materials set, gone entities removed, changed poses written at the frame's alpha: a pose between two ticks every frame, a standing one once; `SyncStats`), `follow` (the scene's origin follows the camera in steps of 512 m), `retire_mesh`, `retire_material` and `collect` (`Renderer::remove_mesh` / `remove_material` once no instance of the bridge uses them), `reset` (a new scene after a lost device), `clear`; `camera` (a game `View` as `render3d::Camera`), `transform`. A crate of its own so that a game's server never links render3d (Cargo unifies features over one build). docs/GAMECORE.md

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
