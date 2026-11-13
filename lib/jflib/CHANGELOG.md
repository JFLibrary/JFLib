# Changelog of jflib

What changed in JFLib, seen from an app that depends on `jflib`: the facade's own changes and
the headlines of its crates. Each crate keeps a changelog of its own with the details, linked
below; the [repository's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md)
has everything, down to the test apps and tools. All of JFLib's crates share one version and are
released together. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- Three crates join the facade: `jflib::render3d`, a 3D engine on the GPU layer with physically
  based materials, shadows, lights, occlusion culling and detail levels
  ([jflib-render3d](https://docs.rs/crate/jflib-render3d/latest/source/CHANGELOG.md));
  `jflib::gltf`, glTF 2.0 import of `.gltf` and `.glb` files
  ([jflib-gltf](https://docs.rs/crate/jflib-gltf/latest/source/CHANGELOG.md)); and
  `jflib::render2d`, a 2D renderer for CAD drawings with lines, arcs, curves, fills, hatches,
  dashes, blocks and transparency ([jflib-render2d](https://docs.rs/crate/jflib-render2d/latest/source/CHANGELOG.md)).
- NetLab is an official style: `jflib::style_netlab`, and `JFLIB_STYLE=netlab` makes it
  `jflib::style` ([jflib-style-netlab](https://docs.rs/crate/jflib-style-netlab/latest/source/CHANGELOG.md)).
- The feature `unwind` (it implies `std`, and both profiles say `panic = "unwind"`): a panic the
  app catches with `std::panic::catch_unwind` leaves it running, on any thread; one nobody
  catches still ends it with JFLib's crash report
  ([jflib-rt](https://docs.rs/crate/jflib-rt/latest/source/CHANGELOG.md)).
- macOS and iOS on Apple silicon: apps run natively, in windows through AppKit and UIKit, with
  the system's services, VoiceOver and, on the Mac, their menus in the menu bar
  ([jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md),
  [jflib-ui](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md)); the GPU layer has a
  Metal backend ([jflib-gpu](https://docs.rs/crate/jflib-gpu/latest/source/CHANGELOG.md)),
  updates replace a Mac app bundle ([jflib-update](https://docs.rs/crate/jflib-update/latest/source/CHANGELOG.md)),
  and `cargo jflib mac` and `cargo jflib package --formats dmg` make a Mac app and its disk image
  ([cargo-jflib](https://docs.rs/crate/cargo-jflib/latest/source/CHANGELOG.md)).
- Windows and Linux on ARM64, with crash reports that show the call chain and, on Linux, plugins
  and loadable modules
  ([jflib-rt](https://docs.rs/crate/jflib-rt/latest/source/CHANGELOG.md),
  [jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- The UI: views of GPU content beside the widgets (`ui.gpu_view`), phones and tablets
  (`ui.form_factor()`), what an app that hosts web content needs (canvases that take the
  keyboard, raw key events, input methods for fields it draws, more mouse cursors, popups at any
  point, a web page's accessibility tree), several files from one file dialog, `ui.calendar`,
  dock groups and syntax highlighting of JavaScript, CSS, HTML and WAT
  ([jflib-ui](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md)).
- The platform layer: an app can register as a web browser (`browser=1` in `PACKAGE`) and ask
  for its default status, print jobs of several pages (`plat::PrintDoc`) and drop effects per
  target ([jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- Smaller additions: ISO weeks in [jflib-datetime](https://docs.rs/crate/jflib-datetime/latest/source/CHANGELOG.md),
  limits of one's own for the XML reader in [jflib-base](https://docs.rs/crate/jflib-base/latest/source/CHANGELOG.md),
  a compute queue of its own on Vulkan in [jflib-gpu](https://docs.rs/crate/jflib-gpu/latest/source/CHANGELOG.md),
  SPIR-V to the Metal Shading Language in [jflib-shader](https://docs.rs/crate/jflib-shader/latest/source/CHANGELOG.md),
  and in [cargo-jflib](https://docs.rs/crate/cargo-jflib/latest/source/CHANGELOG.md) a style of
  the app's own in `package` and `test --features` and `--dir`.
- Every crate has a README of its own, the page crates.io and docs.rs show, and a CHANGELOG.md
  in its package.

### Changed

- `plat::dl_sym` and `plat::dl_close` are `unsafe fn`
  ([jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- The software math that replaces the x87 unit outside x86-64 is within about half an ulp
  ([jflib-rt](https://docs.rs/crate/jflib-rt/latest/source/CHANGELOG.md)).
- Large frames reach the screen faster: blits and the present step copy in bands on several
  threads ([jflib-gfx](https://docs.rs/crate/jflib-gfx/latest/source/CHANGELOG.md),
  [jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)), and an unchanged
  accessibility tree is not built again ([jflib-ui](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md)).
- `cargo jflib package` packages an app with std for the system it runs on by default
  ([cargo-jflib](https://docs.rs/crate/cargo-jflib/latest/source/CHANGELOG.md)).

### Fixed

- Programs link with Rust 1.99 again: with 0.1.0 every program on Windows failed with
  "unresolved external symbol wcslen" ([jflib-rt](https://docs.rs/crate/jflib-rt/latest/source/CHANGELOG.md)).
- Deadlocks in channels and async tasks, races in the worker pool, a crash report that could
  hang, and calls on the UI thread's state from other threads, which panic now instead of racing
  ([jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- A backup could lose its new chunks to a garbage collection on another thread
  ([jflib-cas](https://docs.rs/crate/jflib-cas/latest/source/CHANGELOG.md)); long formatted
  documents are saved again ([jflib-doc-io](https://docs.rs/crate/jflib-doc-io/latest/source/CHANGELOG.md)).
- Text boxes with the same id in two windows no longer share their caret and undo, and several
  inputs that made the UI panic are handled ([jflib-ui](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md)).
- `cargo jflib package` builds its stubs from the app's own JFLib checkout and always with
  `panic = "abort"`, and the Windows setup removes every file it installed
  ([cargo-jflib](https://docs.rs/crate/cargo-jflib/latest/source/CHANGELOG.md),
  [jflib-installer](https://docs.rs/crate/jflib-installer/latest/source/CHANGELOG.md)).

### Security

- The Markdown, HTML and rich-text views open only http, https and mailto links by themselves
  and read no images from network or device paths
  ([jflib-ui](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md)); `plat::open_url`
  takes only absolute URLs, so a path can no longer start a program
  ([jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- Hostile files are bounded: JPEG XL, JPEG XR, HEIF, TIFF, paint.net and SVG images, and Ogg
  Vorbis sounds can no longer make a small file allocate gigabytes or decode for minutes
  ([jflib-img-jxl](https://docs.rs/crate/jflib-img-jxl/latest/source/CHANGELOG.md),
  [jflib-img-jxr](https://docs.rs/crate/jflib-img-jxr/latest/source/CHANGELOG.md),
  [jflib-img-heif](https://docs.rs/crate/jflib-img-heif/latest/source/CHANGELOG.md),
  [jflib-img-tiff](https://docs.rs/crate/jflib-img-tiff/latest/source/CHANGELOG.md),
  [jflib-img-pdn](https://docs.rs/crate/jflib-img-pdn/latest/source/CHANGELOG.md),
  [jflib-img-svg](https://docs.rs/crate/jflib-img-svg/latest/source/CHANGELOG.md),
  [jflib-plat](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md)).
- The packet dissector no longer panics on packets from the network
  ([jflib-netpkt](https://docs.rs/crate/jflib-netpkt/latest/source/CHANGELOG.md)).

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
