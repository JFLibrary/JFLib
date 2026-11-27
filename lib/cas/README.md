# jflib-cas

A content-addressed, deduplicating and encrypted chunk store for backups, part of
[JFLib](https://jflib.org). Files are cut into chunks by their content, every chunk is stored once
however many files and backups contain it, and the data, its indexes and the snapshots that
describe each backup are encrypted and authenticated. Even the ids and the boundaries of the
chunks depend on the repository's key, so without it they tell nothing about the files inside. It
is plain Rust, without the standard library and without a line of `unsafe` code.

## How it is used

A repository is a folder, or any other place that keeps objects by name (see below).
`Repo::create` sets one up with its options, `Repo::open` opens it with one of its keys. A backup writes the data of its files through a `Writer`, describes what it saved
in a `Snapshot` and saves that; a restore loads a snapshot and reads the chunks back.

```rust
use jflib::cas::{Node, NodeKind, Repo, Snapshot, Unlock};

let repo = Repo::open("D:/Backups/repo", Unlock::Passphrase(passphrase))?;
let mut w = repo.writer();
let (chunks, size) = w.put_stream(&mut |buf: &mut [u8]| read_more(buf), &|| false)?;
let (_stats, unsaved) = w.finish_unsaved()?;               // a gc waits until the snapshot is saved
let file = Node { name: "D:/Docs/report.pdf".into(), kind: NodeKind::File, size, chunks, ..Node::default() };
let snap = Snapshot { id: "2026-11-13-0800".into(), created: now, roots: vec![file], ..Snapshot::default() };
unsaved.save_snapshot(&snap, &signer)?;                    // signed with a crypto::ed25519::Keypair
```

Directories are stored as tree chunks of their entries, so an unchanged folder produces the same
tree chunk and is stored only once. Snapshots are signed: the key given at creation
(`CreateOptions::signer`) and the ones added with `trust_signer` are trusted, `save_snapshot`
refuses any other key with `CasError::UntrustedSigner`, and `load_snapshot` returns a snapshot
only after checking its signature against the trusted keys. The list of trusted keys is
protected by the repository key, so it keeps out other keys and mistakes, not someone who can
open the repository: that can call `trust_signer` too. `read_chunk` and `read_tree` give back the
plain data. Several writers can run at once; one holder (a process, through `Store::lock`) has
a repository at a time.

## Where it is stored

A repository consists of objects of four kinds: its config, packs of sealed chunks, the index of
each pack and the snapshots. The `Store` trait keeps them by kind and name, the way S3, SFTP or
WebDAV keep files: `get`, `get_range`, `put`, `delete`, `list`, `init` for a new repository and
`lock`. `Repo::create(path, ..)` and `Repo::open(path, ..)` use `Folder`, the store in a folder of
the file system (a path of JFLib's file layer, so also `mem://` or another mount);
`Repo::create_in(store, ..)` and `Repo::open_in(store, ..)` take any other.

```rust
use jflib::cas::store::{Kind, Store, StoreLock};
use jflib::cas::{CasResult, Repo, Unlock};

struct Bucket { /* a client of an object store */ }

impl Store for Bucket {
    fn location(&self) -> String { "s3://backups/office".into() }
    fn init(&self) -> CasResult<()> { /* NotEmpty when the prefix holds anything */ }
    fn lock(&self) -> CasResult<StoreLock> { /* a lock object; InUse while another holds it */ }
    fn get(&self, kind: Kind, name: &str) -> CasResult<Option<Vec<u8>>> { /* GET */ }
    fn get_range(&self, kind: Kind, name: &str, off: u64, len: usize) -> CasResult<Vec<u8>> { /* GET with Range */ }
    fn put(&self, kind: Kind, name: &str, data: &[u8]) -> CasResult<()> { /* PUT */ }
    fn delete(&self, kind: Kind, name: &str) -> CasResult<()> { /* DELETE */ }
    fn list(&self, kind: Kind) -> CasResult<Vec<(String, u64)>> { /* LIST of the kind's prefix */ }
}

let repo = Repo::open_in(Bucket { /* .. */ }, Unlock::Master(&key))?;
```

A store keeps four rules. A `put` is atomic and durable: once it returns, the whole object is
there, also after a crash, and a failed one leaves the old object or none. A `delete` that
returned stays done, and deleting what is not there is fine. `get` and `list` see every `put`
that returned before them. And `lock` keeps every other holder out, also one in another process
or on another machine, until its guard is dropped. Reads of chunks and the repair of a lost index
ask for ranges, so a store far away sends only what is needed; `get_range` has a default that
reads the whole object.

Nothing that comes back from a store is trusted. Packs, indexes and snapshots are sealed and
`repo.json` carries a MAC, so a store that changes or swaps objects is found out; a rollback to
older objects of the same repository is not. Names and sizes in a listing are checked before
they are used. What a store does see: the names and sizes of the objects and when they come and
go. Pack and index names are random, snapshot names are the ids given to `save_snapshot`.

## What is inside

Chunk boundaries come from content-defined chunking (FastCDC), so an insertion early in a file
moves one boundary and leaves the rest of the file's chunks, and their deduplication, intact. The
average chunk size is a power of two chosen at creation, 1 MiB by default. A chunk's id is a keyed
BLAKE3 hash of its content, and the table that drives the chunker is derived from the repository
key as well, so neither ids nor chunk sizes give anything away without it.

Chunks are compressed with LZ4 or zstd when that saves something, sealed with XChaCha20-Poly1305
and collected in pack files with sealed indexes; a pack repeats its index at the end, so a lost
index file can be rebuilt. A backup commits in order, packs, then indexes, then the snapshot, each
written atomically and durably, so a crash leaves at most packs that nothing references.

Every key comes from one random repository key. The repository's `repo.json` keeps it wrapped
several times: for the server's master key, for each passphrase (Argon2id with 64 MiB) and for a recovery
kit (an X25519 sealed box), so any of them opens the repository and a lost passphrase is not the
end. Snapshots can be made immutable for a number of days, during which `delete_snapshot` refuses
them; the period can be raised but never lowered.

`check` verifies every snapshot, its signature and its references, and with `read_data` also reads,
decrypts and compares every stored chunk. `gc` removes what no snapshot references any more:
empty packs are deleted, packs less than 60 % in use are rewritten with their live chunks. It
stops, with nothing removed, when the snapshots cannot be listed or one of them does not open.

## Platforms

The crate needs JFLib's platform layer for its random numbers, locks and the `Folder` store, so it
is there on Windows, Linux, macOS, iOS and Android. In the browser `jflib` leaves it out.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::cas`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

Nothing in this crate. It is released again as 0.2.1 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-cas/latest/source/CHANGELOG.md) says the same.

## Documentation

- [The building blocks of network programs](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#cas): every public item
  of `cas`, also on [docs.rs](https://docs.rs/jflib-cas)

## License

MIT or Apache-2.0, at your choice.
