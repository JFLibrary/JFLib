# Spiel-Demo

A first-person scene on the game core ([lib/game](../../lib/game/README.md)), its bridge to render3d
([lib/game_render3d](../../lib/game_render3d/README.md)) and render3d in a `ui.gpu_view` with `AF_GPU_3D`
([GAMECORE.md](../../docs/GAMECORE.md), "Die Demo"): a ground, crates in a ring, a wall, steps, four boxes
turning above the walker's head. It builds for Windows, Linux, macOS and the browser.

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
