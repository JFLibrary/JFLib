# jflib-modrt

The runtime inside a loadable module of [JFLib](https://jflib.org). It is to a module what
[`jflib-rt`](https://crates.io/crates/jflib-rt) is to an app: the allocator, the panic handler,
`memcpy` and its relatives, and the few operations JFLib needs to compute the same numbers
everywhere. The difference is where they come from. A module brings no runtime of its own and
imports nothing from the system; everything it needs it gets from the program that loaded it.

## What a loadable module is

A loadable module is a library file, a `.dll` on Windows or a `.so` on Linux, that a JFLib program
loads while it runs, for a large piece of work it does not want to carry in its own executable.
The one module there is today is JFLib's JPEG XL writer, `jxlenc`: a program can leave the writer
out and load it from a file next to its executable on the first JPEG XL save, and the file it
writes is the same byte for byte as with the writer linked in.

A module is built without the standard library and links against nothing. It exports exactly one
function, `<name>_module`, which receives the host's table of functions and returns the module's
own. From then on the host's allocator is the module's allocator, so buffers move between the two
without a copy; the module's parallel loops run on the host's worker pool; its messages go out
through the host; and a panic inside it ends in the host's normal crash report. The host keeps
the module loaded for the rest of the process. The records both sides exchange are described in
JFLib's API reference under
[`plat/modabi.rs`](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#platmodabirs).

## What this crate does

A module is a `cdylib` crate built with `--cfg jflib_module` that takes this crate as `rt` and
[`jflib-modplat`](https://crates.io/crates/jflib-modplat) as `plat`. The library crates compiled
into it (`simd`, `gfx`, `img_core`, `img_jpeg`, `img_jxl`, `img_jxl_enc`) take the two crates
under the same names when that cfg is set, so their sources compile unchanged. For them, modrt
carries rt's float semantics, CPU feature bits, spin lock and extended-precision math, from the
same source files as rt, so the results inside a module equal those of the app. A module also
honours the host's `--simd sse2`.

For the module itself, `module_entry!(name, TABLE)` defines the one export, `host()` gives the
host's table, and `plugin_abi!` is the calling convention of the export and the records. For the
JPEG XL writer, `jxl_table::<W>()` builds the module's table from a type that implements
`JxlWriter`, which receives the request as a safe `JxlRequest`.

## Who needs it

Hardly anyone yet. Loadable modules are built and tested inside JFLib's repository, where `jxlenc`
lives (it is not published itself), but not yet for programs outside it, and this crate's
interface is no part of what JFLib's version number promises
([VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md#the-crates-on-cratesio)).
It is on crates.io because the crates that can be built into a module name it as their `rt` under
`--cfg jflib_module`. It is not part of the `jflib` facade. All of JFLib's crates share one version
and are released together.

## Platforms

Windows and Linux, on x86-64 and ARM64: the systems where JFLib's programs load modules.

## What's new in 0.2.0

Modules run on ARM64, on Linux and on Windows: modrt takes rt's ARM64 versions of the CPU
features, the float conversions and the math, and has `memcpy` and its relatives for ARM64. The
new `plugin_abi!` gives the export and the records the right calling convention on each processor,
the same as before on x86-64. `SpinLock::try_lock` no longer releases a lock another thread holds
when it fails. Everything is in the
[changelog](https://docs.rs/crate/jflib-modrt/latest/source/CHANGELOG.md).

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#platmodabirs): the
  records of host and module, and the host's side in
  [`plat/modhost.rs`](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#platmodhostrs)
- [The module `jxlenc`](https://github.com/JFLibrary/JFLib/blob/main/modules/jxlenc/lib.rs): a
  complete module in 44 lines
- [docs.rs](https://docs.rs/jflib-modrt)

## License

MIT or Apache-2.0, at your choice.
