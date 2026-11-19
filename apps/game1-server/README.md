# Spiel 1 Server (game1-server)

The server of [Spiel 1](../game1/README.md) ([docs/GAME1.md](../../docs/GAME1.md)): rooms by code (a client without
a code makes one), free for all, server-side bots, clients over UDP (desktop) and WebSocket (browser and desktop) on
the same port. One thread; a static Linux program without libc, also Windows. The game's logic is
`apps/game1/crates/game1_core`, the same code the clients predict with; this program never links render3d or gpu.

```sh
python build_rs.py app game1-server
build/win/game1-server.exe                          # 127.0.0.1:27960 (only this computer)
build/win/game1-server.exe --bots 6 --port 27960     # rooms with six bots while they have space
build/win/game1-server.exe --bind 0.0.0.0           # the LAN (the system's firewall may ask)
build/win/game1-server.exe --web build/web/game1    # also the browser build: http://127.0.0.1:27960/
build/win/game1-server.exe --check                  # the self-check (tests/smoke.uis)
```
