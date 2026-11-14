# jflib-agent-io

The transports of [JFLib](https://jflib.org)'s agent interface. Where
[`jflib-agent-core`](https://crates.io/crates/jflib-agent-core) turns JSON-RPC and MCP requests into
calls of an app's actions without touching the outside world, this crate carries those requests
in and the answers out: over standard input and output, and over a local endpoint that only the
user running the app can reach. Nothing here opens by itself; every transport starts only when
the app asks for it. It is plain Rust, without the standard library and without a line of
`unsafe` code.

## Do you need this crate?

An app with a window does not call it directly: the UI runtime of
[`jflib-ui`](https://crates.io/crates/jflib-ui) opens the local endpoint when the app is started
with `--agent` or calls `ui.set_agent_enabled(true)`, closes it again when control is switched
off, and serves `--agent-json` and `--agent-mcp` over standard input and output. The crate is for
programs that host the agent interface themselves, a console tool or a service, and for anyone
writing a transport of their own.

## Standard input and output

`serve_stdio` reads requests line by line from standard input, hands each one to a closure, which
normally calls `RpcSession::handle` of agent_core, and writes only the protocol's responses to
standard output; problems with the streams go to standard error. It returns when the input ends
(0) or cannot be read (1), with the exit code to use.

```rust
use jflib::agent_core::{Dispatcher, Protocol, RpcSession};

let mut dispatcher = Dispatcher::new();
dispatcher.set_enabled(true);
let mut session = RpcSession::new(Protocol::Mcp, "Notes", "1.0");
let code = jflib::agent_io::serve_stdio(|line| session.handle(&dispatcher, &mut notes, line));
```

A request may be up to 1 MiB (`LINE_LIMIT`); a longer one is discarded as a whole and answered
with a JSON-RPC error, and the line after it is read normally. `Framer` does this framing for any
other stream of bytes, a pipe or a socket of your own.

## The local endpoint

`ipc::Server::bind` opens an endpoint for one running instance of an app, and `poll`, called from
the app's event loop, does a bounded amount of work without ever blocking: it accepts up to eight
connections, answers a few requests per connection and turn, and reads nothing more from a client
until it has taken its previous answers. A connection whose unread answers pile up beyond 2 MiB is
closed. When the dispatcher is disabled, the next `poll` closes the endpoint and every connection;
`close` does it at once. Each connection keeps its own protocol state, and on an endpoint opened
for JSON the first valid request that carries an MCP version (or a legacy MCP initialization)
switches that connection to MCP for good. `endpoint()` gives the address to advertise and
`connected()` the number of clients, for a visible indicator in the app.

The endpoint is private to the user on every system:

- **Windows**: a named pipe with a unique name, a security descriptor that grants access only to
  the user of the process, and remote clients refused.
- **Linux and macOS**: a Unix socket with mode 0600 in `/tmp/jfgui-agent-<uid>`, a folder that
  must be a real directory owned by the user with mode 0700; the peer's user id is checked on
  every connection (`SO_PEERCRED` on Linux, `getpeereid` on macOS).
- **iOS**: the same in the app's own temporary folder.

No TCP port is ever opened. Several instances can be active at once, each with its own address,
derived from the app's name as a short ASCII slug; the full name stays in the protocol's metadata.
Dropping the server removes its socket. In the browser and on Android the endpoint does not exist;
there the app's host carries control messages instead, and `serve_stdio` is missing in the
browser.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::agent_io`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

The local endpoint now exists on macOS and iOS too, as a Unix socket in a private folder with the
peer's user checked by `getpeereid`. The
[changelog](https://docs.rs/crate/jflib-agent-io/latest/source/CHANGELOG.md) says the same.

## Documentation

- [The agent interface and the browser port](https://github.com/JFLibrary/JFLib/blob/main/docs/AGENT_WEB.md)
  (in German), and the protocols in the
  [README of jflib-agent-core](https://crates.io/crates/jflib-agent-core)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#agent_io): every public
  item of `agent_io`, also on [docs.rs](https://docs.rs/jflib-agent-io)

## License

MIT or Apache-2.0, at your choice.
