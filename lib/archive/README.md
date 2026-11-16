# jflib-archive

ZIP archives in memory for [JFLib](https://jflib.org), read and written in plain Rust, without the
standard library and without a line of `unsafe` code. It is made for files that come from
elsewhere: everything an archive claims is checked before it is believed.

## What it does

`archive::read(bytes, limits)` returns the parts of a ZIP archive, each with its name and its
bytes. It reads stored and DEFLATE-compressed parts, ZIP64 included, and is strict about
everything else. It refuses encrypted archives and archives split over several disks, and it
refuses names that are absolute, climb out with `..`, contain a backslash, a colon, a `%` or a
control character, or collide with another name on a file system that ignores case. Duplicate
parts, records that overlap, data hidden between or after the parts, sizes no DEFLATE stream can
reach and a wrong CRC are errors too. Sizes are checked against `Limits` (number of parts, bytes
per part, bytes in all, size of the archive, how densely a part may be packed) before anything is
decompressed or allocated; the defaults allow 512 parts of up to 8 MiB and 32 MiB in all, and
readers of large packages pass larger limits with a lower `ratio` against ZIP bombs. Nothing is
ever written to disk.

`archive::write(entries, limits)` writes an archive of stored parts with UTF-8 names and a fixed
date, so the same parts always give the same bytes; `archive::write_with(entries, limits,
Method::Deflate(6))` compresses them, just as deterministically. ZIP64 records appear only when an
archive needs them (65535 parts or more, 4 GiB or more).

`archive::opc` adds the Open Packaging Conventions that DOCX, XLSX, PPTX and VSDX are built on: a
`Package` with its parts, their content types (`[Content_Types].xml`) and relationships
(`_rels/*.rels`), all checked when it is read. Part names compare the way the conventions say,
without regard to case and with percent-escapes understood; targets never leave the package and
external ones are never followed. Parts the reader does not know are kept: changed parts are
replaced, everything else is written back as it was.

```rust
use jflib::archive::{self, opc};

let package = opc::Package::read(&bytes, archive::Limits::default(), opc::Rules::DEFAULT)?;
let main = package.main_part();                               // Some("visio/document.xml")
let pages = package.relationships("visio/document.xml")?;     // ids, types, targets
```

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

## What's new in 0.2.1

Nothing in this crate. It is released again as 0.2.1 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-archive/latest/source/CHANGELOG.md) says the
same.

## Documentation

- [Files and the file namespace in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  and [mounting providers such as `ZipFs`](https://github.com/JFLibrary/JFLib/blob/main/docs/VFS.md#mounting-your-own-providers)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#archive): every public
  item of `archive`, also on [docs.rs](https://docs.rs/jflib-archive)

## License

MIT or Apache-2.0, at your choice.
