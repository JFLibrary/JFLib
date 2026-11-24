# jflib-datastore

A small embedded database for programs built on [JFLib](https://jflib.org): collections of JSON
records, kept in memory and made durable by a write-ahead log and snapshots, so that a crash or a
power cut never loses a transaction that was confirmed and never leaves half of one behind.
Secondary indexes find records by a field, and the files can be encrypted. It
is meant for one server or service process that keeps its users, jobs or settings in a folder of
its own, without an external database. It is plain Rust, without the standard library and
without a line of `unsafe` code.

## How it is used

A `Store` lives in a folder. Each collection maps an id to a JSON value
(`formats::json::Value` of [`jflib-formats`](https://crates.io/crates/jflib-formats)),
and reads return copies of the values, so nothing is held across a call. Changes run as
transactions: the closure of `write` sees the current data and records puts and deletes, which
then become durable and visible together, or not at all when it returns an error.

```rust
use jflib::formats::json::Value;
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
second process out (`StoreError::Locked`).

## Indexes

`filter` looks at every record of a collection. An index finds them by a key instead: the value
of a field (`Index::field`, a path such as `"owner.id"`; an array there gives a key for each
element) or keys computed from the record (`Index::by`). Keys are strings, booleans and integral
numbers, compared exactly; a record without the field is not in the index. A unique index
refuses every transaction that would give two records the same key, before anything is written
(`StoreError::Conflict`), and so does adding one over records that already share a key.

```rust
use jflib::datastore::{Index, Key, Order};

store.add_index("users", "email", Index::field("email").unique())?;
store.add_index("ops", "started", Index::field("started"))?;

let ada = store.find_one("users", "email", &Key::from("ada@example.org"))?;
let newest = store.range("ops", "started", .., Order::Desc, 50)?;    // the 50 newest
let today = store.range("ops", "started", Key::from(midnight).., Order::Asc, usize::MAX)?;
```

Indexes live in memory beside the data and every commit keeps them up to date. They are not
stored: a program adds them after each open, and they are built from the records then, in one
pass. `find`, `find_one`, `range` and `count_key` read through them; an index that was not added
is `StoreError::NoIndex`.

## Encryption

`Store::open_encrypted(dir, options, &key)` opens or creates a store whose snapshots and log
frames are sealed with a 32-byte key (XChaCha20-Poly1305), and `Journal::open_encrypted` does the
same for each line of a journal. The key should come from the system's secret store, which
JFLib's platform layer reaches with `plat::secret::secret_get` and `plat::secret::secret_set`
(an app names `secrets` in its permission manifest for that).

```rust
let key: [u8; 32] = match plat::secret::secret_get("my-app", "db-key") {
    Some(k) if k.len() == 32 => k.try_into().unwrap(),
    _ => {
        let k: [u8; 32] = plat::random_bytes();
        plat::secret::secret_set("my-app", "db-key", &k)?;
        k
    }
};
let store = Store::open_encrypted("data://db", Options::default(), &key)?;
```

A store sealed with another key, an encrypted store opened without one and a store in the clear
opened with one are all `StoreError::Key`, which `Options::repair` never cuts away. Every change
of a sealed file is found: a frame of the log, a snapshot, a line moved to another journal or
month. What encryption cannot find is someone who can write the folder removing or putting back
whole files: an older snapshot, the end of a log, lines of a journal.

`rekey(Some(&key))` encrypts a store that was in the clear or gives it another key, and
`rekey(None)` decrypts it. It writes a snapshot under the new key, starts a new log and removes
every file under the old key. A crash in between leaves the new snapshot, which the new key
opens. The old files are removed, not wiped: on an SSD nothing promises that their blocks are
gone. `Journal::rekey(dir, from, to)` rewrites the journals file by file and is run again with
the same keys after an interruption.

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

An encrypted store seals the snapshot and the payload of each frame; a frame's checksum stays
outside the seal, so a torn frame is still found and cut without the key. A new encrypted store
writes a snapshot at once: its key id tells a wrong key from damage.

## Platforms

The store needs the file system of JFLib's platform layer, so it is there on Windows, Linux,
macOS and iOS. In the browser and on Android `jflib` leaves it out. Folder names may be paths of
the operating system or the names with a scheme that the platform layer knows, such as `data://`.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::datastore`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

Nothing in this crate. It is released again as 0.2.1 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-datastore/latest/source/CHANGELOG.md) says the
same.

## Documentation

- [The building blocks of network programs](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#datastore): every public
  item of `datastore`, also on [docs.rs](https://docs.rs/jflib-datastore)

## License

MIT or Apache-2.0, at your choice.
