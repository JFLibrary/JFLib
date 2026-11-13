# jflib-agent-core

The core of [JFLib](https://jflib.org)'s agent interface: the part that lets scripts, other
programs and AI agents operate an app through the same semantic actions its own menus and
shortcuts run, over JSON-RPC or the Model Context Protocol (MCP). An app describes its actions
once, with schemas for their input and output; a dispatcher checks every call against them
before the app's handler runs. The crate does no input or output of its own, opens no endpoint
and contains no language model: transports hand it complete lines and send back what it returns,
and nothing is reachable from outside until the app switches control on. It is plain Rust,
without the standard library and without a line of `unsafe` code.

## Do you need this crate?

An app with a window mostly meets it through [`jflib-ui`](https://crates.io/crates/jflib-ui). The
UI runtime already offers seven general actions to every app (`ui.state`, `ui.command`,
`ui.focus`, `ui.click`, `ui.type`, `ui.key` and `ui.render`), opens a local endpoint when the app
is started with `--agent` or calls `ui.set_agent_enabled(true)`, and runs a session over standard
input and output with `--agent-json` or `--agent-mcp`. The app adds its own actions through
`App::actions` and `App::action`, with the types of this crate. You use the crate directly to
declare those actions, or to put the dispatcher behind a transport of your own.

It also holds the strict, bounded JSON that other JFLib crates build on (`agent_core::json`), and
`agent_core::jmap`, which maps records and string enums to JSON and back
(`record! { pub struct P { pub a: String, pub n: u32 } }`) so that older and newer programs read
each other's data.

## Declaring actions

```rust
use jflib::agent_core::{ActionSpec, schema::{self, Property, Schema}};

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

An action name is 1 to 128 ASCII letters, digits, `.`, `_` and `-`; names and the optional numeric
command ids must be unique. The capability bits are opaque here and come from JFLib's platform
layer. A `Host` implements `actions`, `revision`, `capabilities` and `execute`, the same handler
the GUI calls; optional methods add availability checks (a modal dialog, an empty selection),
inspection, transactions, tasks and resources. `Dispatcher::execute_local` and `execute_command`
let the app's own menus and shortcuts go through the same schema, capability, availability and
output checks without an endpoint or a revision. An `RpcSession` per connection turns protocol
lines into calls:

```rust
use jflib::agent_core::{Dispatcher, Protocol, RpcSession};

let mut dispatcher = Dispatcher::new();              // starts disabled
dispatcher.set_enabled(true);                        // the app's explicit decision
let mut session = RpcSession::new(Protocol::Json, "Notes", "1.0");
if let Some(reply) = session.handle(&dispatcher, &mut notes, line) {
    // one newline-terminated JSON-RPC response for the transport to send
}
```

Disabling the dispatcher rejects calls at once; the native transport of
[`jflib-agent-io`](https://crates.io/crates/jflib-agent-io) then also closes its connections and
its listener. An action can need the user's confirmation (`ActionSpec::confirm()`) or be closed
to external control (`ActionSpec::local_only()`), and a host may only tighten that
(`Host::access`): in an app with a window, the `agent.confirm` and `agent.deny` lines of its
permission manifest do. Refused calls come back as `forbidden` or `confirmation_required`.

## The contracts

- **Revisions.** An external call of a `write` action must name the revision it read last
  (`expectedRevision`), unless the action turns that off (`revision_required`). The dispatcher
  compares it before the handler runs, so a stale request changes nothing. The host owns the
  revision and must move it only when its state changes; results always report the host's
  current revision.
- **Batches.** `actions/batch` runs up to 128 calls in order and reports each one; it stops at
  the first error unless asked to go on. An atomic batch checks every action, schema, capability,
  revision and the shared transaction domain before it begins, all its revisions refer to the
  state at its start, and its calls must complete synchronously. On a rollback the host must
  restore both its state and its revision and commit the batch as one undo step; a host that
  cannot promise that keeps `supports_transaction` false.
- **Tasks.** Long work returns an explicit task id. The host drives a `TaskTable` (up to 1024
  tasks) at safe interruption points and checks its state again before committing; a cancel
  request is `cancelling` until a checkpoint turns it into `cancelled`.
- **Resources** are opaque ids that the app owns, read in chunks of up to 64 KiB, never file
  paths. There is no action that runs a shell command.

## JSON control protocol 1

One UTF-8 JSON-RPC 2.0 object per line, the same on every transport. A JSON-RPC array is not a
batch here; `actions/batch` is. Every successful result carries `protocolVersion: "1"`, and a
request may name that version in `params.protocolVersion`; other versions are refused.

```json
{"jsonrpc":"2.0","id":1,"method":"actions/list","params":{"query":"text","limit":20}}
{"jsonrpc":"2.0","id":2,"method":"actions/describe","params":{"action":"document.text.insert"}}
{"jsonrpc":"2.0","id":3,"method":"actions/execute","params":{"action":"document.text.insert","input":{"text":"Example"},"expectedRevision":12}}
```

| Method | Parameters |
| --- | --- |
| `agent/info` | none |
| `actions/list` | optional `query`, `cursor`, `limit` (1 to 100) |
| `actions/describe` | `action` |
| `actions/execute` | `action`, optional `input`, `expectedRevision` |
| `actions/batch` | `calls` (1 to 128), optional `atomic`, `stopOnError` |
| `state/inspect` | what the app's inspection defines (filters, pages, `sinceRevision`) |
| `tasks/get`, `tasks/cancel` | `id` |
| `resources/read` | `uri`, optional byte `offset`, `limit` (1 to 65536) |
| `view/render` | the arguments of the registered `ui.render` action |

The library never guesses a document, a layer or a selection from screen coordinates: what
`state/inspect` reports is the app's to define.

## MCP

The MCP adapter follows the public
[2026-07-28 stdio transport](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio),
[schema](https://modelcontextprotocol.io/specification/2026-07-28/schema) and
[tool contracts](https://modelcontextprotocol.io/specification/2026-07-28/server/tools).
Requests of revision 2026-07-28 carry `io.modelcontextprotocol/protocolVersion` and the client's
capabilities in `params._meta`; `server/discover` lists the supported version, and the client's
identity is never used to authorize anything. The older revision 2025-11-25 is supported through
`initialize` and `notifications/initialized`. Nine stable tools cover search, describe, execute,
batch, inspect, task status and cancel, reading resources and rendering; the app's own actions
are found on demand rather than listed as tools. Errors of a tool are results with `isError`,
protocol errors stay JSON-RPC errors, and structured content comes with its text form. Small
resources can also be read with the standard `resources/read`. No subscriptions, sampling,
elicitation or requests from server to client are offered.

## Bounded JSON

`json::parse` checks UTF-8, escapes, surrogate pairs, the number grammar and unique keys, and by
default accepts at most 1 MiB, 32 levels of nesting, 16384 values, 4096 members per container and
256 KiB per string (`parse_with_limits` sets others). Numbers keep their exact token, so an integer
written with a fraction or an exponent still converts exactly, and 64-bit ids and revisions are
never rounded. Encoding applies the same bounds and writes no line breaks, which keeps one message
on one line; `stringify_pretty` indents for files people read.

## Transports and clients

[`jflib-agent-io`](https://crates.io/crates/jflib-agent-io) carries the protocols over standard
input and output and over a local endpoint that only the same user can reach (a named pipe on
Windows, a Unix socket on Linux, macOS and iOS); no TCP port is opened. On an endpoint opened for
JSON, the first valid request decides the protocol of its connection: an MCP version envelope or a
legacy initialization selects MCP, anything else keeps JSON protocol 1, and later requests cannot
switch.

JFLib's repository has a command-line client,
[`tools/jfgui-agent.py`](https://github.com/JFLibrary/JFLib/blob/main/tools/jfgui-agent.py) (Python
with its standard library only), that lists running instances, calls their actions, starts an app
headless with `--agent-json` or `--agent-mcp`, and acts as an MCP stdio gateway to an instance
that is already enabled. It passes arguments as a list and never evaluates shell text.

For a JFLib app in the browser, `jfgui-agent.py browser-bridge --origin ORIGIN` starts an optional
bridge that is not needed for normal use. It listens only on `127.0.0.1`, prints its address, a
fresh 256-bit token and the origin to stderr, holds no app state and offers no shell or file
actions; the activated browser app attaches to it explicitly. It checks the exact origin and the
bearer token, serves one browser, queues at most 64 requests of up to 1 MiB each, handles at most
16 HTTP connections at once, and drops a browser that has not polled for 60 seconds. A timeout
never claims that a change was cancelled or rolled back: inspect the state before retrying.

| Endpoint | Contract |
| --- | --- |
| `POST /connect` | `{app}` returns `{client, protocolVersion:"1", maxMessageBytes}` |
| `GET /poll?client=ID` | up to 20 seconds, returns `{requests:[{id,session,request}]}`, at most eight |
| `POST /result` | `{client,id,response}` returns `{accepted:true}`; the response must match the delivered id |
| `POST /disconnect` | `{client}` closes the browser connection and fails outstanding requests |
| `POST /rpc` | for the client: `{session,request}` returns the browser's JSON-RPC response, at most 30 seconds |
| `POST /session/close` | for the client: `{session}` returns `{closed:true}` once the browser released the session |

The browser keeps a protocol state per session, at most 16; stateless sessions can expire when
the table is full, initialized legacy MCP sessions stay until they are closed.

## Platforms

Every platform JFLib supports, the browser included: the crate makes no calls to the operating
system.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::agent_core`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-agent-core/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [The agent interface and the browser port](https://github.com/JFLibrary/JFLib/blob/main/docs/AGENT_WEB.md)
  (in German) and [agent rules in an app's permissions](https://github.com/JFLibrary/JFLib/blob/main/docs/PERMISSIONS.md#agent-control)
- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#2-skeleton): how an app is
  written and started
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#agent_core): every
  public item of `agent_core`, also on [docs.rs](https://docs.rs/jflib-agent-core)

## License

MIT or Apache-2.0, at your choice.
