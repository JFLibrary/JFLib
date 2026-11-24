# Netz-Test

The tests, the fuzzing and the measurements of the transport for games
([TRANSPORT.md](../../docs/TRANSPORT.md)): netio's WebSocket, the browser's WebSocket in plat and
[lib/netchan](../../lib/netchan/README.md)'s channel over UDP and WebSocket. A console program
without a window; it builds for Windows, Linux, macOS and the browser. Every server it starts
listens on the loopback addresses only (its PERMISSIONS name `localhost`).

## Run

```sh
python build_rs.py test nettest                         # tests/check.uis: --check (all groups)
python build_rs.py run nettest -- --test sim            # one group: codec ws sim reliable udp wschan fuzz;
                                                        # --quick: a tenth
python build_rs.py app nettest --release
build/release/win/nettest.exe --bench --n 2000          # the numbers of TRANSPORT.md
```

The browser test: the page of the web build connects to `nettest --serve` (which serves the page,
an echo WebSocket on `/ws` and the channel on `/chan`, UDP on the same port number), echoes and
measures; then `nettest --client` reaches the same server over UDP and the WebSocket.

```sh
python build_rs.py app nettest
python build_rs.py app nettest --target web --web-mode scalar
node apps/nettest/tests/web.mjs build --engine=chromium,firefox
```

`tools/tests/playwright.mjs` loads Playwright (`PLAYWRIGHT_MODULE`, `PLAYWRIGHT_BROWSERS_PATH`).

Linux: `wsl.exe -d Ubuntu-24.04 -- build/linux/nettest --check` (static, no libc). On a Mac:
`python build_rs.py run nettest --target mac -- --check`.

## Groups

| Group | What |
| --- | --- |
| codec | netio's frame codec: lengths at their boundaries, masks at every offset, bytes one at a time, controls between fragments, every close code, the decoder's bound |
| ws | netio's WebSocket on loopback: echo (text, binary, empty, 1 MiB), fragments both ways, ping / pong, the close both ways, subprotocols, every malformed frame with its close code, the limits (frame, message, fragments, rates, idle, ping), handshake errors on both sides, server stop, 32 clients at once |
| sim | netchan's protocol on a simulated network with a virtual clock: loss, duplicates, reordering, delay, jitter; the ledger of what was sent, arrived and acknowledged; past the wrap of the sequence numbers; reliable streams; handshake under loss; caps, protocol id, rate limits, forged cookies and tokens, answers never larger than their request; the seal over UDP: no payload readable on the wire, every changed byte and forged packet dropped and counted, replays, keys without a shared secret, the static key, a server in the middle |
| reliable | netchan's reliable channel on the same simulated network: a ledger of the messages each side sent against what came out of the other, in order and exactly once, under loss, duplicates and reordering, past the wrap of the chunk numbers; the pace within the server's default limits; Size and Backlog; over a stream; peers that break the protocol |
| udp | the channel over UDP on loopback (IPv4 and IPv6): echo, acks, reliable messages there and back, disconnect, server stop, timeouts, caps, the protocol id, the app's permissions |
| wschan | the channel over WebSocket on loopback (reliable messages there and back too), UDP and WebSocket clients on one server, netchan's WebSocket client against netio's echo server |
| fuzz | a fixed-seed random corpus (about 2.1 million cases): the frame decoder, the packet parser, the server's and the client's protocol, one connection's acks |

## Files

| File | Content |
| --- | --- |
| [src/main.rs](src/main.rs) | arguments, the groups, the counters |
| [src/codec.rs](src/codec.rs), [src/wstest.rs](src/wstest.rs), [src/raw.rs](src/raw.rs) | netio's WebSocket; raw.rs is a client by hand for malformed frames and requests |
| [src/sim.rs](src/sim.rs) | the simulated network and the channel's checks on it |
| [src/reltest.rs](src/reltest.rs) | the reliable channel's checks on the simulated network |
| [src/chantest.rs](src/chantest.rs) | the channel on real loopback sockets |
| [src/fuzz.rs](src/fuzz.rs) | the fuzzing |
| [src/bench.rs](src/bench.rs) | round-trip time and throughput |
| [src/serve.rs](src/serve.rs), [src/browser.rs](src/browser.rs), [tests/web.mjs](tests/web.mjs) | the browser test: server, page, Playwright |
