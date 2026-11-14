# Changelog of cargo-jflib

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

- `cargo jflib mac [--release] [--run [-- ARGS]]`: on a Mac, the app as
  `target/jflib-mac/NAME.app` for Apple silicon. `Info.plist` comes from `PACKAGE` (the shown
  name, the version, the app id, the file types as document types, the URL schemes as URL types)
  and names the languages of the app's catalogue (`src/locales/ui.tsv`, else German and English)
  as its localizations, so macOS and VoiceOver take the app's texts for what they are. The icon
  is made from `res/app_*.png` as `NAME.icns`, the bundle is signed ad hoc, and with `sandbox` in
  `PERMISSIONS` it gets the App Sandbox and the entitlements its rules need. `--run` starts it in
  the logged-in session, also from SSH, and waits for it.
- `cargo jflib package --formats dmg` (on a Mac): per style, the app bundle on a compressed disk
  image beside a link to `/Applications`. `--formats all` leaves it out.
- Signing for macOS: with `jfgui_MAC_IDENTITY` (a Developer ID certificate of the keychain) the
  Mac app and the disk image are signed with it, the hardened runtime and a secure time stamp
  instead of ad hoc. With `jfgui_MAC_NOTARY` (a profile of `xcrun notarytool store-credentials`)
  as well, `--formats dmg` has Apple notarize the app and then the image and staples both
  tickets, so a downloaded `.dmg` passes Gatekeeper.
- `package --update` puts the Mac app (kind `app`) and its `.dmg` (kind `package`) into each
  style's update channel, for `macos-aarch64`.
- `cargo jflib test --features F,..` (repeated, or comma- or space-separated; also `-F`),
  `--no-default-features` and `--dir DIR` (runs `DIR/*.uis` instead of `tests/*.uis`, relative
  to the package's folder), for the scripts of a build with other features. The other commands
  refuse these options instead of ignoring them.
- `cargo jflib test` runs a script with a line `#!only windows linux ..` among its first lines
  only on the systems it names (the `target_os` of `--target`, else of this machine) and counts
  the others as skipped.
- `package` builds a style of the app's own: `styles=` in `PACKAGE` (or `--styles`) may name a
  crate among the app's dependencies with `[package.metadata.jflib] style = "NAME"`, whose `INFO`
  file gives its name and description for the setup. It is built without `JFLIB_STYLE`.
- The style `netlab` is one of the library's styles `package` builds in (`--styles netlab`).
- `browser=1` in `PACKAGE` for an app that is a web browser (`schemes=` must name `http` and
  `https`): the setup registers it as a browser in place of keys for `http` and `https`, the
  Linux packages' `.desktop` entry gets the categories `Network;WebBrowser;`, and the Mac app
  declares `http` and `https` as the web's URL type and its file types as web documents with
  their system types. MSIX refuses it, as Windows reserves `http` and `https` for packages.

### Changed

- `cargo jflib new --std` writes into the app's `Cargo.toml` how an app catches panics: jflib's
  feature `unwind` in place of `std` and `panic = "unwind"` in both profiles.
- `package` without `--formats` packages an app with jflib's feature `std` in the default's
  format of the system it runs on (Windows: the setup, Linux: the `.deb`) instead of refusing it
  on Windows. An error about another system's formats names the ones to pass.
- A setup's payload holds up to 65536 entries (it was 256, too few for the licences of an app
  with std and many crates); `package` refuses more instead of writing a setup that cannot hold
  them.
- The host page of `cargo jflib web` shows the new mouse cursors as their CSS cursors and gives
  the browser's screen readers the rich accessibility tree: tables, rows and cells, landmarks,
  trees, live regions and announcements as ARIA roles and attributes.
- The Android host of `cargo jflib android` paints the status bar and the navigation bar in the
  colours at the app's edges, with icons to match, and gives TalkBack the rich accessibility
  tree: tables and trees as collections with their cells, headers and places, descriptions, a
  link's address, live regions, announcements and the further actions (expand and collapse,
  increment and decrement, select).

### Fixed

- `cargo jflib test` on Linux builds with this machine's `--target` when none is given: without
  it the template's flags reached the build scripts, which rust-lld could not link.
- `package` builds the setup stub and the AppImage runtime of an app inside JFLib's workspace
  from the workspace's crates (it took them from crates.io), and gives the stubs only the app's
  `[profile.release]` tables (its other profile tables were a Cargo warning each).
- When an app's `jflib` comes from a path (a JFLib checkout under `[patch.crates-io]`), `package`
  takes all the stubs' crates from that checkout, `jflib-installer` included, which the app does
  not depend on itself: before, the setup got `jflib-installer` from crates.io. A crate the app
  has from a path of its own keeps that path.
- `package` builds its stubs with `panic = "abort"` whatever the app's profiles say: the setup of
  an app with jflib's feature `unwind` and `panic = "unwind"` failed.
- A stub's manifest no longer gets the comment lines that follow the app's `[profile.release]`
  tables; only the tables are copied.
- On a Mac, `cargo jflib keygen` and the update channel's signer are built for macOS (they were
  built for Linux there).

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
