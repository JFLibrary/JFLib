# jflib-archive

ZIP archives in memory for [JFLib](https://jflib.org), read and written in plain Rust, without the
standard library and without a line of `unsafe` code. It is made for files that come from
elsewhere: everything an archive claims is checked before it is believed.

## What it does

`archive::read(bytes, limits)` returns the parts of a ZIP archive, each with its name and its
bytes. It reads the classic ZIP format with stored and DEFLATE-compressed parts and is strict about
everything else. It refuses encrypted archives, archives split over several disks and ZIP64, and
it refuses names that are absolute, climb out with `..`, contain a backslash, a colon or a control
character, or collide with another name on a file system that ignores case. Duplicate parts,
records that overlap, data hidden between or after the parts and a wrong CRC are errors too.
Sizes are checked against `Limits` (number of parts, bytes per part, bytes in all, size of the
archive) before anything is decompressed or allocated; the defaults allow 512 parts of up to
8 MiB and 32 MiB in all. Nothing is ever written to disk.

`archive::write(entries, limits)` writes an archive of stored parts with UTF-8 names and a fixed
date, so the same parts always give the same bytes.

`ZipFs` shows an archive as a read-only folder tree in JFLib's file namespace. Mounted under a
scheme of its own, its files open with the ordinary file functions of `plat`:

```rust
use alloc::sync::Arc;
use jflib::{archive, plat};

// checked and unpacked once
if let Ok(zip) = archive::ZipFs::new(&bytes, archive::Limits::default()) {
    plat::vfs::mount("pack", Arc::new(zip));
    let wood = plat::read_file("pack://textures/wood.png");   // Option<Vec<u8>>
}
```

JFLib's DOCX support is built on this crate.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::archive`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-archive/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Files and the file namespace in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  and [mounting providers such as `ZipFs`](https://github.com/JFLibrary/JFLib/blob/main/docs/VFS.md#mounting-your-own-providers)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#archive): every public
  item of `archive`, also on [docs.rs](https://docs.rs/jflib-archive)

## License

MIT or Apache-2.0, at your choice.
