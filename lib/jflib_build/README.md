# jflib-build

The build script of every [JFLib](https://jflib.org) app. One line in `build.rs` hands the app's
permission manifest, its package data, its built-in files and its icon to the program, and gives
the linker what JFLib's own runtime needs. When the program starts, `ui::main!` (or `rt::main!`
in a command-line tool) finds everything in place.

## How it is used

`cargo jflib new` (from [`cargo-jflib`](https://crates.io/crates/cargo-jflib)) sets an app up
with it. By hand, it is a build-dependency next to [`jflib`](https://crates.io/crates/jflib), in
the same version:

```toml
[dependencies]
jflib = "0.2"

[build-dependencies]
jflib-build = "0.2"
```

```rust
// build.rs of a program with a window
fn main() {
    jflib_build::app();
}
```

`jflib_build::console()` does the same for a command-line program; on Windows it gets the console
subsystem, so a shell waits for it and shows its output. An app made of several programs, each a
package of its own, keeps its files in one folder that every program names:
`jflib_build::Build::console().part_of("MyTool", "..").run()` gives that program the app's name
and the files in `..`.

## What it hands to the program

Everything is optional and lies next to the project's `Cargo.toml`:

| File | What the program gets |
| --- | --- |
| `PERMISSIONS` | the rules of what the app may do: files, programs, URL schemes, network hosts and more. `ui::main!` and `rt::main!` install them before the app's own code runs; without the file everything is allowed. A `PERMISSIONS.<package>` wins over the app's file for that one program. |
| `PACKAGE` | the shown name, the app id, the categories, file types and URL schemes that `--register` registers, and `browser=1` for a web browser. The same file holds the keys `cargo jflib package` reads. |
| `assets/` | the files the program reads as `app://NAME`, built into it as one bundle; a file is stored LZ4-compressed when that saves at least a tenth |
| `res/app.ico` | the program's icon on Windows |

The build checks all of it and stops with a message that names the problem: a wrong line of
`PERMISSIONS` by its number, a file type, URL scheme or app id that does not fit, or a `version=`
in `PACKAGE` that differs from `Cargo.toml`. The app's version is always `Cargo.toml`'s, and the
program reads it with `rt::app_version!()`. In a program with the standard library (the feature
`std` of `jflib`) the build warns that `PERMISSIONS` covers JFLib's own calls only.

The build script watches only the files that exist. After adding one of them to a project that had
none, run `cargo clean -p NAME` once.

## What it gives the linker

A JFLib program starts without a C runtime, so the linker has to be told how to put it together.
On Windows that is the entry point, the window or console subsystem and the icon. On Linux it is
one static file, or with `--cfg jfgui_glibc` a program against glibc, which a window needs for the
GPU drivers; the libraries to link against are made here, so the build machine needs no glibc.
For the browser it is the exports of the WebAssembly module. macOS, iOS and Android need nothing
extra. A program with `std` gets the C runtime's entry point and libraries instead, and on Linux
it is built on Linux, through the system's `cc`. The linker's map of a Windows or Linux program
lands in the build script's `OUT_DIR` as `program.map`; JFLib's crash reports name offsets into
it.

What a build script cannot set, the linker and the relocation model, comes from the project's
`.cargo/config.toml`. The template of `jflib` has it, and an error here names what is missing.
Build Linux programs with `--target x86_64-unknown-linux-gnu` (or the ARM64 triple), on Linux too,
so that those flags reach the program and not the build scripts.

## Its promise

`jflib-build` is one of the three crates whose interface JFLib's version number covers, next to
`jflib` and `cargo-jflib`: `app()`, `console()`, `Build::app()`, `Build::console()`,
`.part_of()`, `.run()` and the files it reads. All of JFLib's crates share one version and are
released together; keep `jflib-build` at the version of `jflib`.

## What's new in 0.2.0

The build script accepts macOS and iOS, where Apple's linker needs nothing extra. `PACKAGE` takes
`browser=1` for an app that is a web browser, which needs `http` and `https` among its
`schemes=`. For programs against glibc on ARM64 Linux, the libraries made for linking also carry
`getauxval`, through which the runtime reads the page size there. When the Linux flags are
missing, the error says to build with `--target`, on Linux too. Everything is in the
[changelog](https://docs.rs/crate/jflib-build/latest/source/CHANGELOG.md).

## Documentation

- [The skeleton of an app in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#2-skeleton),
  and the [README of `jflib`](https://crates.io/crates/jflib), which names every file of the template
- [PERMISSIONS.md](https://github.com/JFLibrary/JFLib/blob/main/docs/PERMISSIONS.md): the rules
  and their syntax; [PACKAGING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/PACKAGING.md#inputs):
  the keys of `PACKAGE`
- [docs.rs](https://docs.rs/jflib-build)

## License

MIT or Apache-2.0, at your choice.
