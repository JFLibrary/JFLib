# jflib-cas

A content-addressed, deduplicating and encrypted chunk store for backups, part of
[JFLib](https://jflib.org). Files are cut into chunks by their content, every chunk is stored once
however many files and backups contain it, and the data, its indexes and the snapshots that
describe each backup are encrypted and authenticated. Even the ids and the boundaries of the
chunks depend on the repository's key, so without it they tell nothing about the files inside. It
is plain Rust, without the standard library and without a line of `unsafe` code.

## How it is used

A repository is a folder. `Repo::create` sets one up with its options, `Repo::open` opens it with
one of its keys. A backup writes the data of its files through a `Writer`, describes what it saved
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
plain data. Several writers can run at once; one process holds a repository at a time.

## What is inside

Chunk boundaries come from content-defined chunking (FastCDC), so an insertion early in a file
moves one boundary and leaves the rest of the file's chunks, and their deduplication, intact. The
average chunk size is a power of two chosen at creation, 1 MiB by default. A chunk's id is a keyed
BLAKE3 hash of its content, and the table that drives the chunker is derived from the repository
key as well, so neither ids nor chunk sizes give anything away without it.

Chunks are compressed with LZ4 or zstd when that saves something, sealed with XChaCha20-Poly1305
and collected in pack files with sealed indexes; a pack repeats its index at the end, so a lost
index file can be rebuilt. A backup commits in order, packs, then indexes, then the snapshot by an
atomic rename, each synced to disk, so a crash leaves at most packs that nothing references.

Every key comes from one random repository key. The repository's `repo.json` keeps it wrapped
several times: for the server's master key, for each passphrase (Argon2id with 64 MiB) and for a recovery
kit (an X25519 sealed box), so any of them opens the repository and a lost passphrase is not the
end. Snapshots can be made immutable for a number of days, during which `delete_snapshot` refuses
them; the period can be raised but never lowered.

`check` verifies every snapshot, its signature and its references, and with `read_data` also reads,
decrypts and compares every stored chunk. `gc` removes what no snapshot references any more:
empty packs are deleted, packs less than 60 % in use are rewritten with their live chunks.

## Platforms

The store needs the file system of JFLib's platform layer, so it is there on Windows, Linux,
macOS and iOS. In the browser and on Android `jflib` leaves it out.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::cas`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.0

A backup's new chunks are safe between the end of its writer and its snapshot. A gc running in
another thread could start in exactly that gap and remove them as unreferenced, and the snapshot
was then saved without its data. `Writer::finish_unsaved` now keeps gc out until the snapshot is
saved through the `Unsaved` it returns, and `Repo::save_snapshot` waits for a running gc and
refuses a snapshot whose chunks a gc has removed. Before it sweeps, gc rebuilds lost index files
from the packs and stops when a referenced chunk might still be in a pack it cannot read.
`save_snapshot` refuses a key the repository does not trust (`CasError::UntrustedSigner`): it
added such a key to the trusted signers by itself, so the signature check of `load_snapshot`
never refused a snapshot saved through the API.
Everything is in the [changelog](https://docs.rs/crate/jflib-cas/latest/source/CHANGELOG.md).

## Documentation

- [The building blocks of network programs](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#cas): every public item
  of `cas`, also on [docs.rs](https://docs.rs/jflib-cas)

## License

MIT or Apache-2.0, at your choice.
