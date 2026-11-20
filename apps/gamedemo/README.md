# Spiel-Demo

A first-person scene on the game core ([lib/game](../../lib/game/README.md)), its bridge to render3d
([lib/game_render3d](../../lib/game_render3d/README.md)) and render3d in a `ui.gpu_view` with `AF_GPU_3D`
([GAMECORE.md](../../docs/GAMECORE.md), "Die Demo"): a ground, crates in a ring, a wall, steps, four boxes
turning above the walker's head. It builds for Windows, Linux, macOS and the browser.

Since G7b part 2: a HUD over the view (lib/ui draws it, its cells composed on the GPU: veils), four beacons that are
a glTF model (`assets/beacon.gltf`, generated, its texture inside it) loaded through `game::assets` in the background,
and a turning box that hums through `game::audio` (heard from the camera while the game plays; not in script runs).

| Input | What it does |
|---|---|
| a click into the view | locks the pointer and plays; that click fires nothing |
| W A S D (by where they sit: Z Q S D on a French keyboard) | walk; the crates stop the walker and it slides along them |
| Shift (left) | run |
| the mouse | look (raw motion x the platform's factor x sensitivity 1) |
| Esc | belongs to the lock: it ends, the game pauses (the simulation stands, the clock does not catch up); a click plays on |

## Run

```sh
python build_rs.py run gamedemo                       # Windows; Linux: --target linux
python build_rs.py app gamedemo --target web          # the browser
build/win/gamedemo.exe --script apps/gamedemo/tests/shot.uis    # headless: walk, look, pause, build/gamedemo.png
```

Trying it by hand (G7b part 2): does one sensitivity turn about the same on Windows, Wayland, X11, macOS, Chrome
and Firefox? Does the lock come with one click and go with Esc, and does the click fire nothing?

## The window path measured (G7b part 2)

```sh
build/release/win/gamedemo.exe --bench view [--present fifo|mailbox|immediate] [--fullscreen] [--limit HZ] [--n N] [--secs S]
build/release/win/gamedemo.exe --bench own  [--present fifo|mailbox|immediate|all] [--fullscreen] [--limit HZ] [--n N] [--secs S]
node apps/gamedemo/tests/bench_web.mjs build      # the browser (after --target web): the frames the app draws
```

`view` is path (a): the gpu_view over the whole window, the HUD of lib/ui over it with veils, with the read-back of before
and without a HUD. `own` is path (b): a window and a swapchain of its own, the HUD a texture. A thread posts F9 presses
(25 .. 45 ms apart) and reads the screen where the sky is; the sky carries the presses and the frame's number. One
`RESULT` line per step: press -> the app took it -> a frame with it drawn -> on the screen, the frames drawn and the
frames on the screen (their intervals, those never shown, the delay from the draw). Windows only for the screen;
numbers and what they mean: [GAMECORE.md](../../docs/GAMECORE.md), "Spielfenster".
