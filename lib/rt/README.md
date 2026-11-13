# jflib-rt

The runtime at the very bottom of [JFLib](https://jflib.org). A program built on JFLib does not
need Rust's standard library, and this crate is the reason why: it brings what std would
otherwise bring, the memory allocator, `memcpy` and its relatives, the program's entry point and
what happens when something goes wrong. Every other JFLib crate stands on it.

## Do you need this crate?

Usually not directly. An app depends on [`jflib`](https://crates.io/crates/jflib), which brings
this crate along and makes it available as `jflib::rt`. All of JFLib's crates share one version
and are released together, so there is nothing to pick and nothing to keep in step by hand.

## What it does

**It starts the program.** `ui::main!` (for apps with a window) and `rt::main!` (for command-line
tools) end up here. rt defines the entry symbol and hands what the build put into the program, its
name, its permission manifest and its built-in assets, to the platform layer before your code
runs. There is no C runtime underneath. Memory comes in pages straight from the operating system,
and on Linux the result is a static executable that talks to the kernel through system calls.

**It ends the program cleanly when something goes wrong.** A panic and a real crash (an access
violation, an illegal instruction, a division by zero) end the same way: one report on stderr
that says what happened and where, then the app's own crash hook if it set one
(`rt::crash::set_hook`, a good place to save a backup of the open document), then exit code 3.
The crash path never allocates, because the crashing thread may be holding the allocator's lock.

**It keeps numbers the same everywhere.** JFLib is meant to compute the same pixels and the same
text on every machine. rt holds the few operations where that does not come for free:
float-to-integer conversions and rounding exactly as SSE does them (`rt::fx`), and the
extended-precision steps that turn a double into decimal text and back, plus `pow`, `sin`,
`cos`, `exp2`, `log2` and `atan2` (`rt::x87`). On x86-64 those run on the processor's x87 unit.
On ARM64, Android, Apple systems and in the browser a software version takes over that gives the
same bits on every processor and stays within about half an ulp.

**And a few small things the rest needs:** the processor's features (`rt::cpu`), a spin lock
(`rt::sync::SpinLock`), JFLib's version (`rt::VERSION`), the app's own version
(`rt::app_version!()`) and the calling convention of plugins and loadable modules
(`rt::plugin_abi!`).

## With the standard library

An app may still use std, for example to use a crate that needs it (`jflib` with the feature
`std`). Then std brings the allocator, the entry point and the panic handler, and rt steps
aside. `rt::panic_hook()` makes std's panics end with JFLib's crash report all the same. With the
feature `unwind`, a panic the app catches with `std::panic::catch_unwind` leaves the program
running, and `rt::panic_take()` tells what it said. The guide explains it under
[An app with std](https://docs.rs/jflib/latest/jflib/guide/index.html#an-app-with-std).

## Platforms

Windows (x86-64 and ARM64), Linux (x86-64 and ARM64), macOS and iOS on Apple silicon, Android,
and the browser (wasm32).

## What's new in 0.2.0

rt now runs on macOS and iOS, on Linux for ARM64 and on Windows for ARM64. Programs link again
with Rust 1.99: its LLVM turns some loops into calls to `wcslen`, so with 0.1.0 every program on
Windows failed to link. `SpinLock::try_lock` no longer releases a lock it failed to take. The
software math used outside x86-64 is now accurate to about half an ulp instead of up to 118 ulp.
Everything is in the [changelog](https://docs.rs/crate/jflib-rt/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#rt): every public
  item of `rt`, also on [docs.rs](https://docs.rs/jflib-rt)

## License

MIT or Apache-2.0, at your choice.
