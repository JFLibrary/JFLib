# Physik-Test

The tests, the determinism checksum and the measurements of [lib/physics](../../lib/physics/README.md)
([PHYSICS.md](../../docs/PHYSICS.md)). A console program without a window; it builds for Windows,
Linux, the browser, Android and macOS.

## Run

```sh
python build_rs.py test physicstest                       # tests/check.uis: --check on Windows and Linux
python build_rs.py run physicstest -- --test seams        # one group (math hull ray distance cast tunnel
                                                          # seams bvh world fuzz edge), --quick: a tenth
python build_rs.py run physicstest -- --checksum          # the determinism checksum
python build_rs.py app physicstest --release
build/release/win/physicstest.exe --bench --runs 3        # the numbers of PHYSICS.md
```

The checksum must be the same on every target:

```sh
wsl.exe -d Ubuntu-24.04 -- build/linux/physicstest --checksum           # Linux (static, no libc)
python build_rs.py app physicstest --target web
node apps/physicstest/tests/web.mjs build                               # the WebAssembly builds in Node
python build_rs.py app physicstest --target android --android-abi x86_64
python tools/android_console.py physicstest --serial emulator-6200 -- --checksum
```

On a Mac: `python build_rs.py run physicstest --target mac -- --checksum`.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the arguments, the counters of a test group, the random numbers (xorshift64*) |
| `src/tests.rs` | the test groups of `--check` |
| `src/refs.rs` | the references: closest points, polytope distances by brute force, penetration by separating axes, first contact by bisection on the convex gap |
| `src/scene.rs` | the meshes: hilly ground, boxes, steps, ramps, a flat grid |
| `src/checksum.rs` | the determinism harness |
| `src/bench.rs` | the measurements |
| `tests/check.uis` | the test `build_rs.py test` runs |
| `tests/web.mjs` | the WebAssembly builds in Node |
| `PERMISSIONS` | nothing: the program only computes and prints |
| `PACKAGE`, `TARGETS` | name, version, app id; the targets |
