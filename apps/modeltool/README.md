# Modellier-Werkzeug (modeltool)

The feedback loop for modelling in JFScript ([GEOM.md](../../docs/GEOM.md), stage G7g of
[GAME.md](../../docs/GAME.md)): a script describes a model from building blocks and parameters,
[lib/geom](../../lib/geom/README.md) builds the meshes, modeltool checks them, draws them from fixed
cameras and writes a machine-readable report and a glTF file. It is made for an AI that models with the
pictures as its only view, and for people who want to see what a script builds: `--watch` opens a window
that builds the model again whenever the script is saved. A console program with that one window; it
builds for Windows, Linux, macOS and the browser (the browser's build runs the checks and the checksum,
it draws no pictures and has no window).

## Run

```sh
python build_rs.py app modeltool
build/win/modeltool.exe apps/modeltool/examples/kiste.jfs              # -> apps/modeltool/examples/kiste.out/
build/win/modeltool.exe kiste.jfs --out out/kiste --param groesse=1.2  # parameters: params.get("groesse", 0.8)
build/win/modeltool.exe --report kiste.jfs                             # only the report, to stdout
build/win/modeltool.exe --watch kiste.jfs                              # a window: built again when saved
build/win/modeltool.exe --api                                          # the script's functions
python build_rs.py test modeltool                                      # tests/*.uis
build/win/modeltool.exe --checksum                                     # the determinism checksum
build/win/modeltool.exe --examples apps/modeltool/examples/out         # the examples' pictures and reports
```

A build writes into its folder (`--out`, else the script's name with `.out`): `report.json`, `model.glb`,
`front.png` (from +Z), `side.png` (from +X, the right side), `top.png` (from above, the front at the
bottom), `iso.png` (from front right above) and `sheet.png` with the four (front | side over top |
three-quarter). Front, side and top have one scale, like the views of a drawing; the grid's spacing is
in the report and the console line. Exit code 0: built and every check passed; 1: the script failed;
3: built, but a check failed (open, inside out, degenerate, over the budget, other dimensions than
`model.expect_size` says); 2: a wrong command line. A script that has not changed (with the same
parameters and modules) is not built again: the report's `key` is the SHA-256 of all of them
(`--force` builds anyway).

## A script

```js
// a table: a top on four legs
model.name("Tisch")
model.material("eiche", {color: "#b8864e", roughness: 0.7})
let h = params.get("hoehe", 0.75)
let platte = geom.box(1.2, 0.04, 0.8).move(0, h - 0.04, 0)
let bein = geom.cylinder(0.025, h - 0.04, 24).move(0.55, 0, 0.35)
let beine = geom.merge(bein, bein.mirror("x"))
model.add(platte, "eiche", "platte")
model.add(geom.merge(beine, beine.mirror("z")), "eiche", "beine")
model.budget(1000)
model.expect_size(1.2, h, 0.8)
```

Every primitive stands on y = 0, centred over the origin; metres, Y up, the front is +Z. Meshes never
change: `m.move(..)` gives a new one. Convex solids (and the results of earlier Boolean operations) go
into `geom.union`, `geom.difference` and `geom.intersection`, which give one closed solid (a window cut
out of a wall: `geom.difference(wand, fenster)`); `m.bevel(breite, segmente)` chamfers or rounds the edges
of a convex solid. `import()` reads modules only from the script's folder and below. `modeltool --api`
lists every function; [GEOM.md](../../docs/GEOM.md) describes them and what the checks mean.

## Watch

`modeltool --watch FILE.jfs [--param NAME=VALUE ...]` shows the model in a window and builds it again when
the script or one of its modules is saved: the three plan views and the three-quarter view of the
pictures, and a free view (drag with the left button turns, the wheel zooms). An error in the script shows
above the view with its place and line; the last model that was built stays. F5 builds by hand, Ctrl+Q
ends.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the command line, a build (report, glTF, pictures), the cache |
| `src/host.rs` | the script's functions (`geom.*`, `profile.*`, `vec.*`, `model.*`, methods of a mesh), their reference (`--api`), the materials |
| `src/run.rs` | compile, run, the errors with the source line, the measurements and the report |
| `src/render.rs` | the pictures offscreen through render3d; the cameras, grid and light the window shares |
| `src/watch.rs`, `src/watch_ui.rs` | the observation mode: the watched files and the builds; the window |
| `src/export.rs` | the model as glTF (lib/gltf's `write_document`) |
| `src/json.rs` | the report's JSON |
| `src/selftest.rs`, `src/selftest_csg.rs`, `src/refs.rs` | the checks of `--check` (the Boolean operations and bevels in their own file) and their independent references |
| `src/checksum.rs` | the determinism harness |
| `src/examples.rs`, `examples/` | the reference scripts (crate, stairs, door, house from modules, column, tower) and in `examples/out/` their sheets and reports |
| `tests/geom.uis`, `tests/csg.uis`, `tests/examples.uis` | the checks `build_rs.py test` runs |
| `tests/watch.uis`, `tests/watch_fehler.uis`, `tests/kaputt.jfs` | the window headless: the views, dragging, building again; a script with an error |
| `tests/web.mjs` | the WebAssembly builds in Node |
| `PACKAGE`, `TARGETS`, `GLIBC` | name, version, app id; the targets; linked against glibc on Linux (the GPU) |

JFScript comes from its own repository by git, as for editortest; the root `Cargo.toml` takes its
`jflib-agent-core` from this workspace.
