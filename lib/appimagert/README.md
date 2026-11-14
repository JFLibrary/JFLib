# jflib-appimagert

The AppImage runtime of [JFLib](https://jflib.org)'s Linux packages: the small program at the
front of every AppImage that `cargo jflib package --formats appimage` makes. An AppImage is one
file that holds a whole app; this program starts when the user runs that file, unpacks the app
and hands over to it.

## Do you need this crate?

Not in your code. It is on crates.io so that `cargo jflib package` can build the runtime for an
app outside JFLib's repository: it makes a stub crate whose only line is `appimagert::main!();`,
depends on `jflib-appimagert` in the same version as the app's `jflib`, and builds it as one
static Linux program without libc. The app's files follow it in the same file as a SquashFS image.
All of JFLib's crates share one version and are released together.

## What it does

Unlike the usual AppImage runtime it does not mount the image, so the user's system needs no
FUSE. The first start unpacks the image into the user's cache folder
(`$XDG_CACHE_HOME/appimage-run/`), into a folder named after the AppImage file and a hash of its
image, so the first start of a large app takes as long as unpacking it. It unpacks into a
temporary folder and renames it when done, so parallel starts and broken runs never leave a half
folder in use. Later starts of the same file use that folder again, and folders of other versions
with the same file name that were not used for 7 days are removed. Then the app's `AppRun` takes
over the process, with `APPIMAGE`, `APPDIR`, `ARGV0` and `OWD` set as the upstream runtime sets
them.

`cargo jflib package` gives the finished file the layout of an AppImage of type 2, so AppImage
tools recognise it: the magic bytes in the ELF header, the update information and room for an
embedded GPG signature, which the runtime reads back for its options. The runtime reads SquashFS
4.0 images compressed with zstd or lz4 and checks every position and size in the image, so a
damaged file gives an error, not a crash. It understands the usual options:

- `--appimage-extract [PATTERN]`: unpack into `./squashfs-root`
- `--appimage-extract-and-run`: the same as a plain start, since this runtime always unpacks
- `--appimage-offset`, `--appimage-updateinformation`, `--appimage-signature`
- `--appimage-portable-home`, `--appimage-portable-config`: a home or config folder next to the
  AppImage file for the app
- `--appimage-version`, `--appimage-help`
- `--appimage-mount` is refused: this runtime does not mount

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-appimagert/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [Packaging: AppImage](https://github.com/JFLibrary/JFLib/blob/main/docs/PACKAGING.md#appimage):
  what goes into the image, the runtime, signing
- [cargo-jflib](https://crates.io/crates/cargo-jflib), which builds the packages
- [docs.rs](https://docs.rs/jflib-appimagert)

## License

MIT or Apache-2.0, at your choice.
