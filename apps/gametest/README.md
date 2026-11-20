# Spielkern-Test

The tests and the determinism checksum of [lib/game](../../lib/game/README.md) and its bridge to render3d,
[lib/game_render3d](../../lib/game_render3d/README.md) ([GAMECORE.md](../../docs/GAMECORE.md)), and of the
netcode `game::net` ([NETCODE.md](../../docs/NETCODE.md)): on a simulated link with bots, and on real sockets on
127.0.0.1 (`--loopback`). `--check` runs in the console; without arguments it opens the probe window the scripts drive (headless): ui's real input path
into a `game::Input`. It builds for Windows, Linux and macOS (on Linux against glibc: the bridge needs the GPU
drivers) and for the browser, where `tests/web.mjs` runs its console paths in Node (the checksums, the groups but the
bridge).

## Run

```sh
python build_rs.py test gametest                     # tests/*.uis on Windows and Linux
python build_rs.py run gametest -- --test input      # one group (world tick input camera bridge netcodec lagcomp
                                                     # room netsim bots events leave smooth threads assets audio),
                                                     # --quick: a tenth
python build_rs.py run gametest -- --checksum        # the determinism checksum of the core
python build_rs.py run gametest -- --netsum          # the netcode's: a recorded session on the simulated link
python build_rs.py run gametest -- --loopback        # the netcode on real sockets (--secs, --udp, --ws)
build/release/win/gametest.exe --netbench            # the netcode's measurements with 16 bots (docs/NETCODE.md)
build/release/win/gametest.exe --roomsbench          # its last part: 8 rooms of 16 bots, one thread and side by side
build/win/gametest.exe --script apps/gametest/tests/input.uis
```

The checksums must be the same on every target:

```sh
wsl.exe -d Ubuntu-24.04 -- build/linux/gametest --checksum     # Linux (and --netsum)
python build_rs.py run gametest --target mac -- --checksum     # on a Mac
python build_rs.py app gametest --target web
node apps/gametest/tests/web.mjs build                         # WebAssembly in Node: --netsum (or -- --checksum ...)
```

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the arguments, the counters of a test group, the random numbers (xorshift64*) |
| `src/world_t.rs` | entities, generations, columns and their order, versions, 200 000 random steps against a model, a million entities, the canonical order |
| `src/tick_t.rs` | the fixed rate against floor(t x rate / 10^9), the cap, the alpha, systems in order, poses settled and interpolated |
| `src/asset_t.rs` | (G7b part 2) Assets step by step (Frames) and on threads (Background), the glTF and PNG kinds, models on the GPU (Models, sync_models) against a scene built from the documents' matrices |
| `src/audio_t.rs` | (G7b part 2) spatial audio: attenuation and pan against closed forms, sample for sample through plat's mixer without a device (MixTest) |
| `src/input_t.rs` | names, bindings, the map, the settings file, the pointer lock's click and end, frames against ticks, the wheel, capture, plat's events, the factor per platform |
| `src/camera_t.rs` | the camera basis against `Quat::from_yaw_pitch`, the limits, walking, the follow camera against a wall, free flight |
| `src/bridge.rs` | the bridge against a render3d scene built anew, offscreen: 0 pixels different, only changes written, the origin at 10^7, retire and collect, reset, 60 random frames |
| `src/checksum.rs` | the determinism harness |
| `src/net_codec.rs` | the netcode's parts against their own references: the bit stream and its codes, the quantisation grids, snapshot deltas against a plain full encoding, the budget's cut, malformed bytes, input packets and messages, the input buffer across the wrap, the predictor against a replay, the clocks |
| `src/net_lag.rs` | lag compensation: view times, the rewind against a list of every tick, hitscan against a ray-capsule intersection and slabs written here, the arena's shot (a hit where the shooter saw the target) |
| `src/net_room.rs` | room codes, links, names; join, refusals, leave, a reused id, closing rooms and silent connections, the join's full state (WebSocket, ticket, FULL) with netchan events written by hand |
| `src/net_session.rs` | a networked session: the server with the arena and bots over the simulated link, the server's state per tick and every client's first prediction kept for the checks |
| `src/net_sim.rs` | the group netsim (clear, typical, rough, storm; the tick wrap, the join's full state fetched and filled in, leaving and joining, 70 000 snapshots past the sequence wrap), the group bots (16 bots against the limits), `--netbench`, `--netsum` |
| `src/net_loop.rs` | `--loopback`: the netcode on real sockets, UDP and WebSocket clients, a UDP client's fetch over its own WebSocket |
| `src/net_event.rs` | part 2 of the netcode: the groups events (outbox and inbox against a model channel, sessions whose room talks to all, one and all but one), leave, smooth, threads (rooms side by side against one thread, bit for bit; room codes) and `--roomsbench` |
| `src/probe.rs` | the window of the scripts: canvas keys, buttons, raw motion and the lock into an Input, ticks on the virtual clock |
| `tests/check.uis` | `--check` |
| `tests/input.uis` | the lock's click fires nothing, keys by place (a French keyboard), the motion's angle, Esc pauses, a click plays on |
| `tests/tick.uis` | 60 ticks a second on the virtual clock, a key held over 30 ticks, a tap in one tick, no ticks while paused |
| `tests/loopback.uis` | `--loopback --secs 5`: four bots over UDP, four over WebSocket, five seconds on 127.0.0.1 (not in the browser) |
| `tests/web.mjs` | the WebAssembly builds in Node without a browser: `--netsum` unless other arguments follow `--` |
| `PERMISSIONS` | `net.listen` and `net.connect` on localhost (`--loopback`); the tests compute, keep their settings in `mem://` and print |
| `PACKAGE`, `TARGETS`, `GLIBC`, `STYLE` | name, version, app id; the targets; glibc on Linux; the probe's style |
