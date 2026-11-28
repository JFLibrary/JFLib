# Changelog of cargo-jflib

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- Flatpak: the permission key `gamepad` becomes `--device=input` (every input device in `/dev/input`, keyboards too; Flatpak 1.15.6 and newer, an older one refuses to build it), with a note.
- The setup manifest carries the styles' names and descriptions in other languages: styles/S/INFO's further lines name.<lang>= and desc.<lang>= become label_<style>.<lang>= and desc_<style>.<lang>= (cargo jflib package, tools/package.py)
- `cargo jflib android`: the manifest it writes has `android.permission.INTERNET` when the app has no PERMISSIONS or names `net.connect` or `net.listen` there; an own `android/AndroidManifest.xml` without it is refused with those rules (without it Android refuses every socket)
- The Android host prints (android/GalleryPrint.java, copied into the package as for the other host files)

### Changed

- The browser host it carries ticks by the app's next deadline (web/loader.js, worker.js): a view that draws through `ui.timer(id, 1)` runs at the display's rate instead of 30 frames/s; an app at rest or with a slow timer is woken as often as before
- Compresses with jflib-compress instead of its own DEFLATE and LZ4: the .deb's archives, the update channel and the source tarball with gzip at level 9, the setup's payload and the PNGs with zlib at level 9, the AppImage's SquashFS with lz4::compress_hc; its PNG reader with zlib::decompress, CRC-32 from compress. The bytes of the packages change, they get 0.2 to 0.3 % smaller

### Fixed

- Debian package: a pre-release version is written with `~` (`1.2.0~beta.1`, older than `1.2.0` for dpkg; with `-` it counted as newer), the description's lines are indented and a multi-line summary is joined (a line break ended the field and broke `control`); Android: `versionCode` is MAJOR * 1 000 000 + MINOR * 1000 + PATCH (with 10000 / 100, 1.0.100 had the code of 1.1.0), a version outside that is an error.
- The browser and Android hosts it carries: browser: a key that writes a punctuation mark gives the key code the native hosts give (+ , - . as VK_OEM_PLUS / COMMA / MINUS / PERIOD, the keypad as VK_NUMPAD*, others by the key's place) instead of its capital letter's code: typing "$" was Shift+Home and replaced the text before the caret, "." was Delete, "%" Shift+Left, "#" End; browser: a stored file that cannot be imported at the start (too big, no memory) is left out and named in the console; it stopped the whole start, so the app never started again on that origin; android: the keyboard gets the box's kind -- a password keyboard without personalised learning for TB_PASSWORD, a multi-line one whose Enter breaks the line for TB_MULTI, one line with Done (sent to the app as Enter, EV_COMMIT) otherwise -- and restarts when the focus goes to another kind of box; it was always a plain text field (Gboard learned passwords, no line break, Done never reached the app).

### Security

- jfgui_SIGN_PFX_PASS is taken out of the environment when cargo-jflib starts, so the processes it starts (cargo, linkers, gpg, the app under `run`) no longer inherit it. signtool still gets it on its command line (it has no other way); a certificate from the store (jfgui_SIGN_THUMBPRINT) needs none.

## [0.2.1] - 2026-11-16

### Added

- `cargo jflib mac` and `cargo jflib package --formats dmg` build one program for Apple Silicon and Intel Macs (both targets, joined by lipo; the Intel part needs `rustup target add x86_64-apple-darwin`), `cargo jflib mac --arch aarch64|x86_64` only one of them; the disk image is `NAME-VERSION-universal.dmg` (was `-arm64.dmg`), the update channel names it and the bundle for `macos-aarch64` and `macos-x86_64`; the helpers cargo jflib runs on the Mac itself are built for its own processor

### Changed

- `cargo jflib new`: the template's `.cargo/config.toml` has no Linux tables any more, `cargo run`
  builds the app on Linux too. `cargo jflib package` adds rust-lld itself for a Linux program built
  on another system; `test` and `watch` keep `--target` on Linux, so apps from older templates
  build too.

### Security

- `cargo jflib web --serve` serves only plain names below the bundle, and only to the Host 127.0.0.1 or localhost. On Windows `GET /C:/...` and `..\` reached any file, and a web page could read the bundle through DNS rebinding.
- cargo jflib android: --release refuses a manifest that sets android:debuggable, debug APKs are debuggable through aapt2 --debug-mode; a copied clipboard image is shared as a new file each time and earlier copies are deleted, so an app that kept a descriptor of an earlier image cannot read later ones
- The browser loader passes ?theme= only as a plain theme name and the worker drops arguments containing line breaks, so the page address can no longer add arguments or grant files

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
