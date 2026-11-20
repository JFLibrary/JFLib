# Changelog of modeltool

What changed in this app. The app has its own version (`version=` in `PACKAGE`, docs/VERSIONING.md); the
library's changes are in the [library's changelog](../../CHANGELOG.md). The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- `import()` reads modules only from the script's folder and below (a path outside it is refused before any loader sees it, and the file loader follows no link on the way); modeltool starts through `ui::main!` outside the browser.

### Added

- G7g part 2 (docs/GEOM.md): Boolean operations `geom.union`, `geom.difference`, `geom.intersection` on convex solids and on results of earlier ones (a result keeps its convex cells through move, place, center, rotations, scale and mirror); `m.bevel(breite, segmente, achse)` chamfers or rounds edges and corners of a convex solid; the observation mode `--watch FILE` (a window with the model in a gpu_view, the pictures' views and a free view, built again when the script or a module is saved, errors above the view); a finding about the surface names where it lies first and the part; the example `turm.jfs`; the checksum folds the example and nine Boolean and bevel operations (`0fae175a31a0f15e` on Windows, Linux, macOS and in Node); the test groups `csg`, `bevel`, `watch` and the headless scripts `tests/watch.uis`, `tests/watch_fehler.uis`.
- The first version (G7g part 1, docs/GEOM.md): a JFScript script describes a model through the host functions `geom.*` (box, cylinder, cone, sphere, torus, stairs, arch, extrude, prism, sweep, lathe, merge), the methods of a mesh (move, place, center, rotate_x / y / z, rotate, scale, mirror, flip, weld, array, radial, flat, smooth, bounds, area, volume, check), `profile.*`, `vec.*` and `model.*` (add, material, name, budget, expect_size, uv_scale) with the parameters of `--param`; lib/geom builds it. modeltool checks the model and each part, draws front, side and top at one scale and a three-quarter view offscreen through render3d with a grid (twice the size, averaged down), writes the PNGs, a sheet, `report.json` and `model.glb`; the SHA-256 of script, parameters and modules is the cache key. Errors name file, line and column and show the source line with a caret. `--report`, `--api`, `--check` (8 groups), `--checksum`, `--examples`; five reference scripts with their sheets and reports.
