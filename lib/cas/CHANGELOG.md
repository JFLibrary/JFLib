# Changelog of jflib-cas

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The storage is a trait: cas::Store (lib/cas/store.rs) keeps a repository's objects by kind (store::Kind: Config, Pack, Index, Snapshot) and name, the way S3, SFTP or WebDAV keep files -- location, init (a new repository; NotEmpty), lock (one holder; InUse, a StoreLock guard), get, get_range (with a default that reads the whole object), put, delete, list with sizes; implemented for Box and Arc of a store. Its rules: a put is atomic and durable, a delete stays done, reads see every put before them, the lock keeps out holders of other processes and machines. cas::Folder is the store in a folder (the layout as before, also on vfs mounts such as mem://); Repo::create_in, Repo::open_in, Repo::store and peek_id_in take any store, Repo::create, Repo::open and peek_id a folder as before. store::name_ok says which names a store is asked for

### Changed

- JSON (pack indexes, snapshots, the repository's config) goes through jflib-formats (`formats::json`); the crate no longer depends on jflib-agent-core. Its API is unchanged.
- Repairing a lost index file (repair_index, and gc before it sweeps) reads the index from the pack's end in two ranges instead of the whole pack. A pack the index names but the store does not have is Corrupt ("pack ... missing"), it was Io; a snapshot id with a NUL is refused like the other invalid ids (NotFound), and snapshot_ids leaves out names that cannot be ids. Repo::open reads repo.json again once it holds the lock, and Repo::create takes the lock before it writes repo.json

### Fixed

- gc deleted every pack when the snapshots folder could not be read (a refused read_dir: the rights, a network share gone): it took the folder for one without snapshots. It now stops with the error, and check fails too (snapshot_ids alone still answers an empty list). gc also keeps a pack whose index file it cannot delete, where it deleted the pack anyway: the index left behind named chunks that were gone, and on the next open it could win over the copies a repack had moved
- `Folder`: a pack name whose first two bytes are not whole letters is refused instead of panicking

### Security

- A repo.json without its MAC no longer opens (`Corrupt`). It was sealed again as genuine, so whoever could write it could change signers, retention and keys; every published version writes the MAC.

## [0.2.1] - 2026-11-16

No changes in this crate. It was released with the others because all of JFLib's crates move together.

## [0.2.0] - 2026-11-14

### Added

- `Writer::finish_unsaved() -> (WriteStats, Unsaved)` and `Unsaved::save_snapshot`: finish a
  writer and save its snapshot with gc kept out in between. Dropping the `Unsaved` gives the
  backup up and lets gc run again.

### Changed

- `Repo::save_snapshot` and `Unsaved::save_snapshot` refuse a key the repository does not trust with the new `CasError::UntrustedSigner`. They added the key to the trusted signers by themselves, so `load_snapshot`'s signature check never refused a snapshot saved through the API and `trust_signer` changed nothing. Trusted are `CreateOptions::signer` and the keys given to `trust_signer`; a program that saves with another key calls `trust_signer` first. The list is protected by the repository key: it keeps out other keys and mistakes, not someone who can open the repository

### Fixed

- A backup's chunks are safe between the end of its writer and its snapshot. A gc in another
  thread that waited for the writer ran in exactly that gap, removed the new chunks as
  unreferenced, and the snapshot was stored without its data. Backups where a gc can run beside
  them finish with `finish_unsaved`; `Repo::save_snapshot` waits for a running gc and refuses
  (`CasError::NotFound`) a snapshot whose chunks a gc has removed.
- `gc` rebuilds lost index files from the packs' trailers before it sweeps, as `repair_index`
  does: a pack whose index file was lost went as one nobody uses, together with the chunks of its
  snapshots. It refuses (`CasError::Corrupt`) and removes nothing while a referenced chunk is
  missing and a pack without an index and without a readable trailer is left.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
