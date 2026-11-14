# Changelog of jflib-cas

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Changed

- `Repo::save_snapshot` and `Unsaved::save_snapshot` refuse a key the repository does not trust with the new `CasError::UntrustedSigner`. They added the key to the trusted signers by themselves, so `load_snapshot`'s signature check never refused a snapshot saved through the API and `trust_signer` changed nothing. Trusted are `CreateOptions::signer` and the keys given to `trust_signer`; a program that saves with another key calls `trust_signer` first. The list is protected by the repository key: it keeps out other keys and mistakes, not someone who can open the repository

## [0.2.0] - 2026-11-13

### Added

- `Writer::finish_unsaved() -> (WriteStats, Unsaved)` and `Unsaved::save_snapshot`: finish a
  writer and save its snapshot with gc kept out in between. Dropping the `Unsaved` gives the
  backup up and lets gc run again.

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
