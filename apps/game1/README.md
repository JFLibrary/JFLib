# Spiel 1 (game1)

The first game of the JFLib game engine ([docs/GAME.md](../../docs/GAME.md), stage G7h; the game's own document:
[docs/GAME1.md](../../docs/GAME1.md)): a fast cartoon multiplayer shooter, free for all with one hitscan weapon, in
the browser and on the desktop. "Spiel 1" and `game1` are working names; the user picks the real name later. Part 1
is the playable core: one map, one weapon, rooms by code, bots, the HUD, the start and pause menus, settings.

## Run

```sh
python build_rs.py run game1                       # the window: practice against bots, or join a server
python build_rs.py app game1-server                # the server (UDP and WebSocket, 127.0.0.1 unless --bind)
build/win/game1-server.exe --bots 4                # a server whose rooms have four bots
python build_rs.py app game1 --target web          # the browser build, served by the server:
build/win/game1-server.exe --web build/web/game1   #   http://127.0.0.1:27960/
python build_rs.py test game1                      # tests/check.uis, loopback.uis, shot.uis
build/win/game1.exe --sums                         # the determinism checksums (the same on every target)
build/win/game1.exe --bench                        # the measurements (release build)
node apps/game1/tests/web.mjs -- --sums            # the checksums in the browser's engine (Node)
python apps/game1/tools/content.py [--check]       # the map and the figure from JFScript again (byte for byte)
```

Controls: click into the view to play (that click does not shoot), W A S D by their place on the keyboard, the mouse
looks, left button fires, R reloads, Space jumps (held: on every landing), Ctrl or C crouches, Shift walks, Tab holds
the scoreboard, Esc pauses (the pointer lock ends).

## Files

| File | What it is |
|---|---|
| `crates/game1_core/` | the game without a GPU, shared with `apps/game1-server`: `rules.rs` (all numbers), `map.rs` (the `.g1m` format, the map's physics world), `movement.rs` (the character controller and the weapon behind `game::net::Movement`), `sim.rs` (the room: `RoomSim`), `bot.rs` (server-side bots), `host.rs` (the server's loop on real sockets) |
| `content/*.jfs` | the map (`werft.jfs`) and the figure (`figur.jfs`) in JFScript for `apps/modeltool`; `*.g1m` the baked files the game reads |
| `tools/content.py` | modeltool + `game1 --bake` for every script; `--check` rebuilds and compares the bytes |
| `src/app.rs` | the window: menus, HUD, settings |
| `src/play.rs`, `src/local.rs` | a client while it plays (prediction, effects, HUD state); the practice mode's server in this process |
| `src/scene.rs`, `src/look.rs` | render3d in a `ui.gpu_view`: the toon look C, figures from rigid parts, the weapon in front of the camera, particles |
| `src/bake.rs` | modeltool's `.glb` into `.g1m` |
| `src/t_*.rs`, `src/bench.rs`, `src/loopback.rs` | the test groups, the measurements, real sockets on 127.0.0.1 |
| `tests/` | the scripts `build_rs.py test game1` runs; `web.mjs` runs the console paths in Node |
