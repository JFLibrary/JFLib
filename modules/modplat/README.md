# jflib-modplat

The platform layer inside a loadable module of [JFLib](https://jflib.org): the small part of
[`jflib-plat`](https://crates.io/crates/jflib-plat) that the library crates compiled into a module
call, served by the program that loaded the module.

## What a loadable module is

A loadable module is a library file, a `.dll` on Windows or a `.so` on Linux, that a JFLib program
loads while it runs, for a large piece of work it does not want to carry in its own executable.
The one module there is today is JFLib's JPEG XL writer, `jxlenc`. A module is built without the
standard library, imports nothing from the system and gets everything it needs from its host.
[`jflib-modrt`](https://crates.io/crates/jflib-modrt) explains the rest: how a module starts, how
memory and panics pass between the two sides, and how a module is built.

## What this crate does

A module takes this crate as `plat`, next to `jflib-modrt` as `rt`, and the library crates in it
compile against it unchanged. It offers what they use of `plat`:

- `parallel_for(count, &f)` runs `f(index, job)` for every index on the host's worker pool and the
  calling thread, the same loop `plat`'s pool runs in an app; nesting is allowed, and an item can
  stop the rest with `JobCtl::cancel`.
- `thr_count()` and `thr_self()` are the host pool's, so arrays with one slot per thread work as
  they do in the host.
- The JPEG XL writer's module functions (`jxlenc_ready` and its relatives) answer that there is no
  module to load: inside a module there is none.

## Who needs it

Hardly anyone yet. Loadable modules are built and tested inside JFLib's repository, but not yet
for programs outside it, and this crate's interface is no part of what JFLib's version number
promises
([VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md#the-crates-on-cratesio)).
It is on crates.io because the crates that can be built into a module name it as their `plat`
under `--cfg jflib_module`. It is not part of the `jflib` facade. All of JFLib's crates share one
version and are released together.

## Platforms

Windows and Linux, on x86-64 and ARM64: the systems where JFLib's programs load modules.

## What's new in 0.2.0

The loop items that the host calls use the calling convention of plugins and modules
(`rt::plugin_abi!` of `jflib-modrt`), so modules with parallel loops run on ARM64 too; on x86-64
nothing changes. Everything is in the
[changelog](https://docs.rs/crate/jflib-modplat/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#platmodabirs): the
  records of host and module, among them the host's `parallel_for`
- [docs.rs](https://docs.rs/jflib-modplat)

## License

MIT or Apache-2.0, at your choice.
