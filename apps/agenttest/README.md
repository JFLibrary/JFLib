# agenttest

Native self-check executable for the portable `agent_core` library. It exercises bounded
JSON parsing/encoding, schemas, activation, capability and modal checks, stale revisions,
shared GUI/JSON/MCP handlers, batch preflight and rollback, task cancellation, resource
chunks, isolated application state and both supported MCP protocol eras. A headless
runtime probe verifies one exit callback, immediate control revocation, and refusal of
both reactivation and semantic mutations after shutdown.

Run from `RustLib`:

```powershell
python build_rs.py app agenttest --target both --out build/agent-core
./build/agent-core/win/agenttest.exe
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/agent-core/linux/agenttest
```

The default test uses an in-memory counter host and opens no GUI window, pipe, socket or
standard-input control endpoint. All output is an ordinary test summary; the tested
protocol adapter itself has no standard-stream access. Stream-framing checks include
split UTF-8, CRLF, EOF, oversize records and recovery after a rejected record.

Explicit integration modes `--ipc-server`, `--agent-json` and `--agent-mcp` are used by
the cross-process tests. The temporary IPC server exits after 20 seconds; `--long-title`
exercises conversion of a long Unicode display name into a bounded native endpoint slug.

```powershell
python apps/agenttest/tests/ipc_check.py --exe build/agent-core/win/agenttest.exe
python apps/agenttest/tests/bridge_check.py
python build_rs.py app gallery --target both --out build/agent-core
python build_rs.py app Kritzel --target both --out build/agent-core
python apps/agenttest/tests/gui_ipc_check.py --bin-dir build/agent-core/win
```

Run the same Python scripts under Linux with `--exe build/agent-core/linux/agenttest` and
`--bin-dir build/agent-core/linux`. The GUI test briefly starts real Gallery/Kritzel windows
with automatic shutdown, verifies that normal app launches expose no control endpoint,
checks CLI discovery, widget inspection, PNG resources, and Kritzel resize/fill/undo. It
uses only new in-memory documents. Windows launches request hidden windows.

`bridge_check.py` starts an ephemeral loopback listener and emulates the browser side to
exercise token/origin checks, request correlation, bounded queues, timeout/disconnect
behavior and CLI forwarding. Real browser rendering and input are tested by the common
web host separately.

The additional `word_regressions.py --exe .../jfWord[.exe]` checks semantic failures,
dynamic command availability, stale revisions and modal protection without touching the
system clipboard. `browser_bridge.mjs BUILD_DIR chromium` (or `firefox`) exercises a real
CLI-to-browser connection, Unicode document edits, both MCP revisions, session pressure,
explicit close and revocation. It uses the repository's common web host, local builds and
an available Playwright installation (`PLAYWRIGHT_MODULE` or Node's module resolution, see
`tools/tests/playwright.mjs`); no npm packages are downloaded by the test.
