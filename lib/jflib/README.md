# jflib

[JFLib](https://jflib.org) is a GUI library and everything it stands on, without the standard
library: windows and controls, text, images in many formats, the GPU, files, the network,
documents, and a runtime of its own. This one dependency brings all of it; a program links only
what it uses. The same app runs on Windows, Linux, macOS, iOS, Android and in the browser.

`docs/GUIDE.md` in this package explains how an app is written, `docs/API.md` lists every public
item. Both lie below `~/.cargo/registry/src/` once Cargo has downloaded the crate; the guide is
also on docs.rs as [`jflib::guide`](https://docs.rs/jflib/latest/jflib/guide/index.html), the
API reference [on GitHub](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md).

## Quick start

```sh
cargo install cargo-jflib
cargo jflib new hallo         # or new --std hallo: an app with std (below)
cd hallo
cargo run
cargo jflib test              # runs tests/*.uis
cargo jflib watch             # hot reload while you edit
cargo jflib web --serve       # the app in the browser
cargo jflib android --install # the app on a phone (Android SDK and NDK)
cargo jflib mac --run         # on a Mac: the app as NAME.app
cargo jflib package           # a setup for Windows and a .deb, every style inside
                              #   (--formats all: also MSIX, AppImage, RPM, Flatpak;
                              #   on a Mac --formats dmg)
```

`cargo jflib new` writes the template that lies in this package as `template/`. Without the tool,
copy it (`cp -r ~/.cargo/registry/src/*/jflib-0.2.*/template hallo`, then rename `Cargo.toml.in`
to `Cargo.toml`).

| File | What it is for |
| --- | --- |
| `Cargo.toml` | `jflib`, and `jflib-build` for the build script; the profiles below |
| `build.rs` | `jflib_build::app()`, or `jflib_build::console()` for a command-line program |
| `.cargo/config.toml` | the linker for Windows (rust-lld, it comes with rustup); Linux needs nothing (with std only `+crt-static` for Windows) |
| `src/lib.rs` | the app: a `ui::App` and `ui::main!` |
| `src/main.rs` | the program, it only links the app, so the same code builds for the browser |
| `PERMISSIONS` | what the app may do; without the file everything |
| `PACKAGE` | its name, id and file types for `--register` |
| `assets/` (optional) | files the app reads as `app://NAME` |
| `res/app.ico` (optional) | its icon on Windows |
| `tests/smoke.uis` | a test script, run with `--script` |

Names come from the crate in code: `use jflib::{style, ui};`, then `ui::Ui`, `plat::read_file`.

## Apps with std

JFLib is `no_std` and brings everything an app needs from one hand. Write apps `no_std` as well
where you can, and look in `docs/API.md` before adding a crate. An app that needs a crate with
`std` turns on the feature `std` (`cargo jflib new --std` writes such an app):

```toml
jflib = { features = ["std"], version = "0.2" }
```

Then `src/lib.rs` has no `#![no_std]`, `src/main.rs` is `extern crate hallo; fn main() {
jflib::plat::std_start() }`, `.cargo/config.toml` only links the C runtime in on Windows
(`+crt-static`) and `PERMISSIONS` covers JFLib's own calls only. Linux programs with std are built
on Linux (cc, glibc), macOS and iOS ones on a Mac; in the browser std has no clock, files or
threads and there is no threads mode; on Android `std::fs` needs the app's real folder
(`docs/GUIDE.md`, "An app with std").

Don't add a second window loop or UI crate (winit, eframe, egui, SDL): `ui::launch` is the whole
stack, not one crate of several as with egui and eframe. Don't run an async runtime (tokio) on the
UI thread: JFLib runs `async` blocks itself (`ui.spawn`); a crate that needs tokio gets it on a
thread of its own, with results back through `plat::wake()`.

## The profiles

Cargo takes no profile from a dependency, so every project needs these in its `Cargo.toml`.
Without `panic = "abort"` the build stops at `ui::main!` with a message that says so. An app with
std that must survive panics of its own code turns on the feature `unwind` instead of `std` and
writes `panic = "unwind"` in both profiles: a panic it catches with `std::panic::catch_unwind`
leaves it running, one nobody catches ends it with JFLib's crash report (`docs/GUIDE.md`, "An app
with std").

```toml
[profile.dev]
panic = "abort"
opt-level = 1
debug = 0
debug-assertions = false
overflow-checks = false

[profile.release]
panic = "abort"
opt-level = 2
codegen-units = 1
lto = "fat"
debug = 0
strip = "symbols"
```

## Targets

| Target | Build | Notes |
| --- | --- | --- |
| Windows | `cargo build`; ARM64: `--target aarch64-pc-windows-msvc` | rust-lld; the program links without import libraries, the build scripts need what Rust's msvc toolchain always needs (the Visual Studio Build Tools) |
| Linux | `cargo build`, on ARM64 Linux too | a window links against glibc (the GPU) through cc, as every Rust program; a command-line program can be one static file (`.cargo/config.toml`, then with `--target`); from Windows `cargo jflib package` builds the Linux packages |
| macOS | `cargo build` on a Mac; `cargo jflib mac` makes `NAME.app` | Apple silicon; Apple's linker needs nothing in `.cargo/config.toml` |
| iOS | `cargo build --target aarch64-apple-ios` (or `aarch64-apple-ios-sim`) on a Mac | devices and the simulator; `cargo jflib` makes no iOS app yet |
| Browser | `cargo jflib web` | scalar, SIMD and with threads, the page around them and the licences in `target/jflib-web/NAME/`; threads need `rustup component add rust-src` |
| Android | `cargo jflib android` | an APK for `arm64-v8a` (or `--abi x86_64` for the emulator) in `target/jflib-android/`; needs the Android SDK, the NDK, a JDK and `rustup target add aarch64-linux-android`; the app's code in `src/lib.rs` as the template has it. Threads, the network (`netio`), files (`datastore`, `cas`), printing and the system's fonts work as on the desktop; `plat::process` and `plat::service` answer that they are not supported, `update` is missing (apps update through their store) |

The style: `jflib::style` is WinUI unless the environment variable `JFLIB_STYLE` names another
(`classic`, `kde`, `macos`, `netlab`). All five are always there under their own names for the
switch at run time (`jflib::style_kde::STYLE`).

## Good to know

- **A file added later:** after adding `PERMISSIONS`, `PACKAGE`, `assets/` or `res/app.ico` to
  a project that had none, run `cargo clean -p NAME` once. The build script watches only what
  exists; watching a missing file would compile the program again on every build.
- **Linux needs no flags:** a program with a window links against glibc through cc, as the
  build scripts do, so `cargo run` is all it takes there too. Only one static file (a
  command-line program) takes `-C target-feature=+crt-static` and `-C relocation-model=static`
  in `.cargo/config.toml`, and is then always built with `--target x86_64-unknown-linux-gnu`,
  also on Linux: without it Cargo hands these flags to the build scripts and proc macros as well.
  A project made from a template up to 0.2.0 drops the `[target.*-unknown-linux-gnu]` tables of
  its `.cargo/config.toml` (with them it still builds with `--target`).
- **Paths in a release program:** panic messages name the source files with the build
  machine's paths. `RUSTFLAGS="--remap-path-prefix=$HOME=~"` removes them.
- **Versions:** the crates below `jflib` (`jflib-ui`, `jflib-plat`, ...) always come in the
  same version and change together. Only the way through `jflib` is promised, together with
  `jflib-build` and `cargo jflib` (docs/VERSIONING.md, "The crates on crates.io").
- **Your own system calls:** JFLib checks a program's file, network and process access against
  its `PERMISSIONS` in its platform layer. A crate that calls the system itself goes around it;
  only the `sandbox` line confines it, as far as the operating system enforces the manifest
  (docs/PERMISSIONS.md, "OS sandbox").

## What's new in 0.2.1

The template's `.cargo/config.toml` has no Linux tables any more: `cargo run` builds a JFLib app on
Linux as on Windows and macOS, without `--target`. A project made from an older template drops the
`[target.*-unknown-linux-gnu]` tables of its `.cargo/config.toml` (with them it still builds with
`--target`). Through its crates 0.2.1 also brings Intel Macs, a round of security fixes in `plat`
and the other crates, and fixes in `ui`, `base`, `netio` and others. Everything is in the
[changelog](https://docs.rs/crate/jflib/latest/source/CHANGELOG.md).

## License

MIT or Apache-2.0, at your choice. Parts carry third-party licenses in addition, named in each
crate's manifest and its `licenses/` folder.
