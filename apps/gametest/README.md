# Spielkern-Test

The tests and the determinism checksum of [lib/game](../../lib/game/README.md) and its bridge to render3d,
[lib/game_render3d](../../lib/game_render3d/README.md) ([GAMECORE.md](../../docs/GAMECORE.md)). `--check` runs
in the console; without arguments it opens the probe window the scripts drive (headless): ui's real input path
into a `game::Input`. It builds for Windows, Linux and macOS (on Linux against glibc: the bridge needs the GPU
drivers).

## Run

```sh
python build_rs.py test gametest                     # tests/*.uis on Windows and Linux
python build_rs.py run gametest -- --test input      # one group (world tick input camera bridge), --quick: a tenth
python build_rs.py run gametest -- --checksum        # the determinism checksum
build/win/gametest.exe --script apps/gametest/tests/input.uis
```

The checksum must be the same on every target:

```sh
wsl.exe -d Ubuntu-24.04 -- build/linux/gametest --checksum     # Linux
python build_rs.py run gametest --target mac -- --checksum     # on a Mac
```

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the arguments, the counters of a test group, the random numbers (xorshift64*) |
| `src/world_t.rs` | entities, generations, columns and their order, versions, 200 000 random steps against a model, a million entities, the canonical order |
| `src/tick_t.rs` | the fixed rate against floor(t x rate / 10^9), the cap, the alpha, systems in order, poses settled and interpolated |
| `src/input_t.rs` | names, bindings, the map, the settings file, the pointer lock's click and end, frames against ticks, the wheel, capture, plat's events, the factor per platform |
| `src/camera_t.rs` | the camera basis against `Quat::from_yaw_pitch`, the limits, walking, the follow camera against a wall, free flight |
| `src/bridge.rs` | the bridge against a render3d scene built anew, offscreen: 0 pixels different, only changes written, the origin at 10^7, retire and collect, reset, 60 random frames |
| `src/checksum.rs` | the determinism harness |
| `src/probe.rs` | the window of the scripts: canvas keys, buttons, raw motion and the lock into an Input, ticks on the virtual clock |
| `tests/check.uis` | `--check` |
| `tests/input.uis` | the lock's click fires nothing, keys by place (a French keyboard), the motion's angle, Esc pauses, a click plays on |
| `tests/tick.uis` | 60 ticks a second on the virtual clock, a key held over 30 ticks, a tap in one tick, no ticks while paused |
| `PERMISSIONS` | nothing: the tests compute, keep their settings in `mem://` and print |
| `PACKAGE`, `TARGETS`, `GLIBC`, `STYLE` | name, version, app id; the targets; glibc on Linux; the probe's style |
