# Changelog of jflib-datastore

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- Secondary indexes (lib/datastore/index.rs): Store::add_index(coll, name, Index) builds one from the records and every commit keeps it up to date; Index::field("owner.id") takes the value at a path of members (an array there gives a key for each element), Index::by(|id, value| keys) computes them, .unique() refuses every transaction that would give two records one key before anything is written, and an index over records that already share one (StoreError::Conflict). Key (Bool < Int < Str; Key::of: strings, booleans and integral numbers, exactly), find, find_one, range (any RangeBounds of keys, Order::Asc or Desc, a limit) and count_key read through them, drop_index and index_names manage them; an index that was not added is StoreError::NoIndex. Indexes are not stored: they are added after each open
- Encryption at rest (lib/datastore/seal.rs): Store::open_encrypted(dir, options, &key) opens or creates a store whose snapshots and log frames are sealed with a 32-byte key (XChaCha20-Poly1305; keys derived with BLAKE3, a key id in every snapshot), Journal::open_encrypted seals each line, bound to its journal and month. A wrong key, an encrypted store opened without one and one in the clear opened with one are StoreError::Key, never cut away by Options::repair; a changed frame is Corrupt, a frame in the clear in an encrypted log is refused. Store::rekey(Some(&key) | None) encrypts a store, changes its key or decrypts it (a snapshot under the new key, a new log, the files under the old key removed; a crash in between leaves what the new key opens), Journal::rekey(dir, from, to) rewrites the journals and is repeated after an interruption; Store::encrypted and Journal::encrypted say which they are
- On Android too: `jflib` takes the crate there now that `plat::fs` has the files (docs/ANDROID.md, AN3); docs.rs builds it for `aarch64-linux-android`

### Changed

- The values are `formats::json::Value` of jflib-formats; the crate no longer depends on jflib-agent-core. `Value` is the same type as `agent_core::json::Value` (that path is a re-export now), so code that names it keeps compiling.

### Fixed

- A torn last log frame whose payload holds the bytes `BCW1` is cut like any torn frame: only a whole valid frame behind a damaged one makes it damaged (a bare magic made the store refuse to open).
- On Windows, a second compact() with no transaction in between failed with Io (cannot open the log): it opened the log that was open already, and Windows opens a file for writing once. The log that is open is kept now

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
