# jflib-datastore

A small embedded database for programs built on [JFLib](https://jflib.org): collections of JSON
records, kept in memory and made durable by a write-ahead log and snapshots, so that a crash or a
power cut never loses a transaction that was confirmed and never leaves half of one behind. It
is meant for one server or service process that keeps its users, jobs or settings in a folder of
its own, without an external database. It is plain Rust, without the standard library and
without a line of `unsafe` code.

## How it is used

A `Store` lives in a folder. Each collection maps an id to a JSON value
(`agent_core::json::Value` of [`jflib-agent-core`](https://crates.io/crates/jflib-agent-core)),
and reads return copies of the values, so nothing is held across a call. Changes run as
transactions: the closure of `write` sees the current data and records puts and deletes, which
then become durable and visible together, or not at all when it returns an error.

```rust
use jflib::agent_core::json::Value;
use jflib::datastore::{Store, StoreError};

let store = Store::open("data://db")?;                      // one process at a time
store.put("users", "ada", Value::object([("name", Value::string("Ada")), ("admin", Value::Bool(true))]))?;

store.write(|data, tx| {                                    // both changes or none
    if !data.get("users").is_some_and(|users| users.contains_key("bob")) {
        tx.put("users", "bob", Value::object([("name", Value::string("Bob"))]));
    }
    tx.delete("sessions", "expired");
    Ok::<(), StoreError>(())
})?;

let admins = store.filter("users", |_, v| v.get("admin").and_then(Value::as_bool) == Some(true));
```

`get`, `list`, `ids`, `count` and `filter` read, `update` changes one record in place, and every
transaction gets a sequence number. The store can be shared between threads; a lock file keeps a
second process out (`StoreError::Locked`). Values are stored as plain JSON, so secrets should be
sealed by the caller, for example with [`jflib-crypto`](https://crates.io/crates/jflib-crypto),
before they go in.

For logs that only grow, such as an audit trail or the log of a session, `Journal` appends JSON
lines to one file per name and month, reads a month or the newest entries, and removes old
months on request. A crash during an append costs at most the partial last line, which readers
skip.

## What happens on disk

Each transaction is one frame of the write-ahead log, protected by a BLAKE3 checksum and written
and synced before it becomes visible. When the log has grown past a size (16 MiB unless
`Options` says otherwise), the store writes a snapshot of all collections and starts a new log.
Opening loads the newest snapshot and replays the frames after it. A frame torn by a crash at the
very end of the log is cut off; other damage, in the log or in the newest snapshot, stops the
open with `StoreError::Corrupt`, unless `Options::repair` allows cutting the log at the damage or
falling back to an older snapshot. `recovery()` reports what was found and repaired. A record
may be up to 16 MiB of JSON, a transaction up to 64 MiB.

## Platforms

The store needs the file system of JFLib's platform layer, so it is there on Windows, Linux,
macOS and iOS. In the browser and on Android `jflib` leaves it out. Folder names may be paths of
the operating system or the names with a scheme that the platform layer knows, such as `data://`.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::datastore`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-datastore/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [The building blocks of network programs](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#datastore): every public
  item of `datastore`, also on [docs.rs](https://docs.rs/jflib-datastore)

## License

MIT or Apache-2.0, at your choice.
