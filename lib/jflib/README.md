# jflib

A GUI library and everything it stands on, without the standard library: windows and controls,
text, images in many formats, the GPU, files, the network, documents, and a runtime of its own.
This one dependency brings all of it; a program links only what it uses.

`docs/GUIDE.md` in this package explains how an app is written, `docs/API.md` lists every public
item. Both lie below `~/.cargo/registry/src/` once Cargo has downloaded the crate.

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
cargo jflib package           # a setup for Windows and a .deb, every style inside
                              #   (--formats all: also MSIX, AppImage, RPM, Flatpak)
```

`cargo jflib new` writes the template that lies in this package as `template/`. Without the tool,
copy it (`cp -r ~/.cargo/registry/src/*/jflib-0.1.*/template hallo`, then rename `Cargo.toml.in`
to `Cargo.toml`).

| File | What it is for |
| --- | --- |
| `Cargo.toml` | `jflib`, and `jflib-build` for the build script; the profiles below |
| `build.rs` | `jflib_build::app()`, or `jflib_build::console()` for a command-line program |
| `.cargo/config.toml` | the linker (rust-lld, it comes with rustup) and what Linux needs (with std only `+crt-static` for Windows) |
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
jflib = { features = ["std"], version = "0.1" }
```

Then `src/lib.rs` has no `#![no_std]`, `src/main.rs` is `extern crate hallo; fn main() {
jflib::plat::std_start() }`, `.cargo/config.toml` only links the C runtime in on Windows
(`+crt-static`) and `PERMISSIONS` covers JFLib's own calls only. Linux programs with std are built
on Linux (cc, glibc); in the browser std has no clock, files or threads and there is no threads
mode; on Android `std::fs` needs the app's real folder (`docs/GUIDE.md`, "An app with std").

Don't add a second window loop or UI crate (winit, eframe, egui, SDL): `ui::launch` is the whole
stack, not one crate of several as with egui and eframe. Don't run an async runtime (tokio) on the
UI thread: JFLib runs `async` blocks itself (`ui.spawn`); a crate that needs tokio gets it on a
thread of its own, with results back through `plat::wake()`.

## The profiles

Cargo takes no profile from a dependency, so every project needs these in its `Cargo.toml`.
Without `panic = "abort"` the build stops at `ui::main!` with a message that says so.

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
| Windows | `cargo build` | rust-lld; the program links without import libraries, the build scripts need what Rust's msvc toolchain always needs (the Visual Studio Build Tools) |
| Linux | `cargo build --target x86_64-unknown-linux-gnu` | a window links against glibc (the GPU); a command-line program can be one static file (`.cargo/config.toml`) |
| Browser | `cargo jflib web` | scalar, SIMD and with threads, the page around them and the licences in `target/jflib-web/NAME/`; threads need `rustup component add rust-src` |
| Android | `cargo jflib android` | an APK for `arm64-v8a` (or `--abi x86_64` for the emulator) in `target/jflib-android/`; needs the Android SDK, the NDK, a JDK and `rustup target add aarch64-linux-android`; the app's code in `src/lib.rs` as the template has it |

The style: `jflib::style` is WinUI unless the environment variable `JFLIB_STYLE` names another
(`classic`, `kde`, `macos`). All four are always there under their own names for the switch at
run time (`jflib::style_kde::STYLE`).

## Good to know

- **A file added later:** after adding `PERMISSIONS`, `PACKAGE`, `assets/` or `res/app.ico` to
  a project that had none, run `cargo clean -p NAME` once. The build script watches only what
  exists; watching a missing file would compile the program again on every build.
- **Always `--target` for Linux, on Linux too:** without it Cargo hands the target's flags to
  the build scripts and proc macros as well, and the static relocation model breaks them. With it
  they are linked by cc as usual and only the program by rust-lld. Without the two linker flags
  cc links the program too.
- **Paths in a release program:** panic messages name the source files with the build
  machine's paths. `RUSTFLAGS="--remap-path-prefix=$HOME=~"` removes them.
- **Versions:** the crates below `jflib` (`jflib-ui`, `jflib-plat`, ...) always come in the
  same version and change together. Only the way through `jflib` is promised, together with
  `jflib-build` and `cargo jflib` (docs/VERSIONING.md, "The crates on crates.io").
- **Your own system calls:** JFLib checks a program's file, network and process access against
  its `PERMISSIONS` in its platform layer. A crate that calls the system itself goes around it;
  only the `sandbox` line confines it, as far as the operating system enforces the manifest
  (docs/PERMISSIONS.md, "OS sandbox").

## License

MIT or Apache-2.0, at your choice. Parts carry third-party licenses in addition, named in each
crate's manifest and its `licenses/` folder.
