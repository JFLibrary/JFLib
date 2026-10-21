# Shared agent control core

`agent_core` is a `no_std` / `alloc` library with no external crates, GUI dependencies,
operating-system calls or embedded model. `Host` supplies an app's real semantic handlers;
`Dispatcher` applies the same contracts to GUI commands and external calls. Opening any
endpoint is a separate, explicit choice made by `agent_io` / the app runtime.

## Registering an application

```rust
static INSERT: Schema = Schema::Object {
    properties: &[Property { name: "text", schema: &schema::STRING, required: true }],
    additional: false,
};
static ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        capabilities: 0,
        undo: true,
        transaction: Some("document"),
        command_id: Some(100),
        ..ActionSpec::write("document.text.insert", "Insert text at the selection")
            .schemas(&INSERT, &schema::OBJECT)
    },
];
```

Implement `Host::actions`, `revision`, `capabilities` and `execute`. Optional methods add
availability/modal checks, inspection, transactions, asynchronous tasks and resource
access. `Dispatcher::execute_local` and `execute_command` use the same schema, capability,
availability, implementation and output checks as external execution; trusted GUI calls
do not require an external endpoint or an expected revision.

The dispatcher starts **disabled**. `set_enabled(true)` is an explicit host decision.
Disabling it rejects calls immediately; the IPC adapter also closes all peer connections
and removes its listener on the next poll (or immediately through `Server::close`).

An action name is 1–128 ASCII letters/digits/`.`/`_`/`-`; names and optional command IDs
must be unique. Capability bits are opaque to this library and come from `plat`.
Availability failures contain a concrete machine-readable code and explanation.

## Revision and transaction contracts

- Mutating `ActionSpec::write` actions require `expectedRevision` externally. The check
  happens before calling the host. A stale request has no mutation side effects.
- The host owns revision advancement, undo history and domain state. Returned revisions
  are always read from the host, never trusted from a result constructor.
- Ordered non-atomic batches report each attempted result. They stop on error by default;
  callers may request continuation. A call's expected revision refers to its execution time.
- Atomic batches preflight all actions/schemas/capabilities/revisions and transaction
  domains before `begin_transaction`. Every call's expected revision refers to the batch's
  initial state. Calls must complete synchronously within an atomic transaction.
- The host must restore **both state and revision** on rollback, include all affected
  document/selection/undo state in its snapshot, and commit one undoable operation.
  A host that cannot guarantee this must leave `supports_transaction` false.
- Deferred jobs use explicit task IDs. Hosts drive a `TaskTable` at safe interruption
  points and must recheck relevant state/revisions before committing asynchronous work.
  A cancellation request first becomes `cancelling`, then `cancelled` at a checkpoint.

## JSON control protocol 1

Transport framing is one UTF-8 JSON-RPC 2.0 object per line. Native stdio, named pipes,
Unix sockets and browser messages share the same dispatcher. JSON-RPC arrays are not a
transport batch: use `actions/batch` for the defined ordering/transaction behavior.

```json
{"jsonrpc":"2.0","id":1,"method":"actions/list","params":{"query":"text","limit":20}}
{"jsonrpc":"2.0","id":2,"method":"actions/describe","params":{"action":"document.text.insert"}}
{"jsonrpc":"2.0","id":3,"method":"actions/execute","params":{"action":"document.text.insert","input":{"text":"Example"},"expectedRevision":12}}
```

Every successful control result includes `protocolVersion: "1"`. Supplying an optional
`params.protocolVersion` explicitly selects revision 1; unsupported versions are errors.
The unversioned spelling remains fixed to revision 1, rather than silently selecting a
future protocol. Methods:

| Method | Parameters |
| --- | --- |
| `agent/info` | none |
| `actions/list` | optional `query`, `cursor`, `limit` (1–100) |
| `actions/describe` | `action` |
| `actions/execute` | `action`, optional `input`, `expectedRevision` |
| `actions/batch` | `calls` (1–128), optional `atomic`, `stopOnError` |
| `state/inspect` | host-defined query/filter/pagination/revision fields |
| `tasks/get`, `tasks/cancel` | `id` |
| `resources/read` | `uri`, optional byte `offset`, `limit` (1–65536) |
| `view/render` | arguments of the registered `ui.render` action |

Inspection/delta semantics come from the app/runtime adapter; the generic library never
pretends to infer a document, image layer or selection from screen coordinates. Resources
are opaque app-owned identifiers with separately read payload chunks, not unrestricted
filesystem paths. There is no shell-execution action.

The strict JSON parser validates UTF-8, escapes, surrogate pairs, numeric grammar and
unique object keys. Numbers retain their exact token and integer conversion is exact even
for exponent/fraction spellings of integral values. Defaults: 1 MiB encoded, 32 nesting
levels, 16384 values, 4096 members per container and 256 KiB per decoded string. Encoding
uses the same bounds and emits no literal line breaks. Clients using floating-point-only
JSON libraries must preserve large integer IDs/revisions without rounding.

## MCP versions and catalog

The adapter follows the public [2026-07-28 stdio transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio),
[schema](https://modelcontextprotocol.io/specification/2026-07-28/schema) and
[tool contracts](https://modelcontextprotocol.io/specification/2026-07-28/server/tools).

- Modern revision **2026-07-28** requires `params._meta` with
  `io.modelcontextprotocol/protocolVersion` and per-request `clientCapabilities`.
  Client identity is optional and never used for authorization. `server/discover` returns
  modern supported versions and all modern results include `resultType: "complete"`.
- Legacy revision **2025-11-25** supports `initialize` followed by
  `notifications/initialized`. Other legacy offers receive a counteroffer of 2025-11-25.
  Modern requests do not inherit the legacy negotiation state.
- Nine stable tools provide search, describe, execute, batch, inspect, task status/cancel,
  resource reading and rendering. Semantic app commands are discovered on demand.
- Tool execution errors are structured `isError` results; protocol errors remain JSON-RPC
  errors. Structured content is accompanied by serialized text for compatibility.
- Small resources also support standard `resources/read`; large resources are explicitly
  chunked with `jfgui.resources.read`. Tasks are application-owned IDs, not a claim to
  implement a separately negotiated MCP task extension.
- Handlers finish synchronously or return an explicit deferred task. MCP cancellation of
  an already completed request has no effect; deferred work is cancelled through its task
  action. No subscriptions, sampling, elicitation, or server-to-client requests are advertised.

`RpcSession` produces protocol strings only. It has no stdout/stderr APIs. Native stdio
transports write only these strings to stdout and send diagnostics to stderr.

## Native local connections and CLI

`agent_io::ipc::Server` is nonblocking and bounds work/output per UI tick. Its listener is
created only after explicit activation. Each connection owns framing and protocol state.
The first valid request's explicit MCP version envelope (or legacy initialization)
selects MCP for that connection; otherwise it remains JSON protocol 1. Later requests
cannot switch the selected protocol.

Windows uses a unique local named pipe with a protected DACL granting access only to the
process token's exact user SID, with remote clients rejected. Linux uses a verified mode
0700 directory `/tmp/jfgui-agent-UID`, a mode 0600 Unix socket, and `SO_PEERCRED` checks.
Dropping the listener removes its owned socket; stale sockets after crashes are ignored by
discovery. No TCP listener is opened. Both modes permit multiple explicitly active instances.
Endpoint slugs are derived from display names, bounded and ASCII-only; spaces, Unicode and
long window titles remain intact in protocol metadata and cannot prevent activation.

The shared standard-library CLI is `python tools/jfgui-agent.py`:

```text
jfgui-agent.py list
jfgui-agent.py actions ENDPOINT --query document
jfgui-agent.py describe ENDPOINT document.text.replace
jfgui-agent.py call ENDPOINT document.text.replace --input-file request.json --expected-revision 12
jfgui-agent.py inspect ENDPOINT --input-json '{"sinceRevision":12}'
jfgui-agent.py stdio ENDPOINT --protocol mcp
jfgui-agent.py headless APP --protocol json
```

`stdio` is a protocol-preserving MCP gateway to an already enabled native instance.
`headless` starts an app's explicit `--agent-json` / `--agent-mcp` mode using an argument
array, without shell evaluation. Browser transport and browser bridging are separate host
adapters and are not provided by this native IPC module.

Run `apps/agenttest` for core contracts and `apps/agenttest/tests/ipc_check.py --exe ...`
for independent cross-process, current-user isolation, DACL/permissions, instance and
CLI/MCP-gateway checks on each native operating system.
`apps/agenttest/tests/gui_ipc_check.py --bin-dir ...` also exercises real Gallery/Kritzel
event loops, disabled-by-default endpoints, discovery, PNG resources and graphics undo.

## Explicit browser bridge

The optional bridge is separate from normal browser operation. Start it only when an
external CLI/MCP client is meant to control an explicitly activated browser app:

```text
python tools/jfgui-agent.py browser-bridge --origin http://127.0.0.1:8765
python tools/jfgui-agent.py browser-call --url http://127.0.0.1:PORT --token TOKEN actions/list
python tools/jfgui-agent.py browser-stdio --url http://127.0.0.1:PORT --token TOKEN
```

`browser-bridge` binds only `127.0.0.1`, chooses an available port by default, and generates
a fresh 256-bit capability token. It prints `{url, token, origin}` to **stderr**. It has no
app state, shell actions or file actions; all calls go to the activated browser's same
Rust dispatcher. The app must explicitly attach using this configuration. Exact-origin
CORS, a bearer token, one attached browser, bounded message/queue sizes, connection limits,
and deadlines apply. Native bridge clients bypass environment HTTP proxies and reject
redirects so capability tokens stay on loopback.

Browser requests include `Authorization: Bearer TOKEN` and the chosen browser `Origin`:

| Endpoint | Contract |
| --- | --- |
| `POST /connect` | `{app}` → `{client, protocolVersion:"1", maxMessageBytes}` |
| `GET /poll?client=ID` | Up to 20 seconds → `{requests:[{id,session,request}]}`, maximum eight items |
| `POST /result` | `{client,id,response}` → `{accepted:true}`; response must match the delivered RPC ID |
| `POST /disconnect` | `{client}` closes the browser connection and fails outstanding requests |
| `POST /rpc` | CLI: `{session,request}` → the raw browser JSON-RPC response, maximum 30 seconds |
| `POST /session/close` | CLI: `{session}` → `{closed:true}` after explicit browser-session release |

The browser forwards each envelope to `jfguiAgent.request(request,{session})`. Distinct
agent sessions have distinct Rust `RpcSession` state. Notifications are acknowledged to
the bridge with `response:null`, and produce no MCP/JSON stdout response. A timeout never
claims a mutation was cancelled or rolled back: callers must inspect state before retrying.
The queue holds at most 64 requests, each at most 1 MiB; at most 16 HTTP connections are
processed concurrently. A browser with no polls/results for 60 seconds expires when the
bridge next checks it. There is no HTTP listener when the bridge command is not running.

Close jobs are delivered as `{id,session,closeSession:true}` and acknowledged with a null
response after `jfguiAgent.closeSession(session)`. `browser-stdio` releases its session
on EOF. `browser-call` reuses `cli-json` (or `cli-mcp` for explicit MCP metadata) unless a
session is supplied. The browser keeps at most 16 protocol sessions; least-recently-used
stateless JSON/current-MCP entries can expire, while initialized legacy MCP sessions stay
until explicit close or revocation. A full table of negotiated legacy sessions returns a
correlated error without executing the request. Malformed envelopes do not reserve slots.

`apps/agenttest/tests/bridge_check.py` checks actual loopback HTTP, auth, exact-origin CORS,
request/response correlation, separate sessions, bounded queues, disconnects, timeouts,
strict JSON and the CLI stdio bridge. The test emulates the browser side; rendering/input
browser tests belong to the common web host's own validation.
`apps/agenttest/tests/browser_bridge.mjs` additionally verifies the real Python CLI through
the HTTP bridge and browser worker into jfWord's Rust dispatcher in Chromium and Firefox,
including document edits, both MCP revisions, more than 16 requests and session cleanup.
