# jflib-update

Signed updates for apps built with [JFLib](https://jflib.org). An app checks a channel for a newer
version, downloads it in the background, puts it in place at the next start and goes back to the
old version on its own when the new one does not start. What protects an update is an Ed25519
signature with a key built into the app, not the connection, so the channel can be any web
server.

## How it is used

The app describes itself in an `update::Config`: its id, the version that runs, the address of
the channel's `update.json` and the public keys of its publisher. `update::startup` comes first
in `main`, before the UI, and `update::healthy` once the new version has shown that it works:

```rust
const UPDATE_KEYS: &[&str] = &["<the publisher's public key, 64 hex digits>"];

fn update_config() -> update::Config {
    update::Config::new("Kritzel", rt::app_version!(), "https://example.org/kritzel/stable/update.json", UPDATE_KEYS)
}

ui::main!({
    // puts a downloaded update in place and restarts into it, or rolls back a version that failed
    if let update::Startup::RolledBack(v) = update::startup(&update_config()) {
        // version v did not start and the old one is back: tell the user
    }
    ui::launch(&DESC, App::default(), &style::STYLE)
});
```

An `update::Updater` does the checking and downloading. `start_check()` and `start_download()`
run on a thread of their own and wake the UI when they are done, and `status()` gives the phase
(available, downloading with its progress, ready, failed) for a dialog or a status line. The same
steps exist as blocking calls for console programs. A downloaded update waits until the next
start, or `apply()` puts it in place at once and restarts the program.

When an update arrives, the updater checks the signature of the manifest, that it names this app
and has not expired, that its version is newer than the running one and was never rolled back,
and the size and SHA-256 of the file; right before the update is put in place it checks the
signature, the version and the file once more. A manifest older than one the app has already
seen is refused, so a mirror cannot push the app back to an older version. Downloads that break
off continue where they stopped.

A new version runs on trial until it calls `healthy()`. A start that finds the trial still open
puts the old files back and restarts the old version, which learns of it through
`Startup::RolledBack`. That version is never offered again.

## Ways a program is installed

The same calls follow the way the program came to the user (`update::install`):

- **A plain program**: the channel's `exe` entry replaces the program file, a `bundle` entry the
  program with its modules and further programs, and a `package` entry (a setup, a `.deb`) is only
  downloaded and checked for the user to install.
- **An AppImage**: the whole `.AppImage` file is replaced. A signed AppImage only takes an update
  signed with a key of its own.
- **A macOS app bundle**: the whole `NAME.app` is replaced in one step, after its code signature
  was checked against the installed one.
- **Flatpak**: the update comes from the app's Flatpak repository through the Flatpak portal.
- **MSIX and Snap**: the package system updates the app, and a check says so.

Putting files in place needs a folder the user can write. An app with a `PERMISSIONS` manifest
needs `fs.write $EXE/**` for that (an AppImage: `fs.write $APPIMAGE/**`), and its `net.connect`
rules apply to every download.

`update::fetch` is the updater's HTTP(S) for an app's other downloads, for example course
material or brushes, with a size and a SHA-256 the caller knows.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), as `jflib::update`, on
Windows, Linux and macOS; in the browser and on Android `jflib` leaves it out.
`cargo jflib keygen FILE` makes the signing key and prints the public key for the app, and
`cargo jflib package --update DIR` writes a signed channel per style next to the packages. All of
JFLib's crates share one version and are released together.

## What's new in 0.2.0

Programs on macOS can update themselves now: a channel's new kind `app` replaces the app bundle as
a whole, checked against the installed bundle's signature, with trial and rollback as everywhere
else. `update::target()` names macOS and iOS (a Mac reported itself as `linux-aarch64`), and an
updated program on a Mac keeps its executable bit. Everything is in the
[changelog](https://docs.rs/crate/jflib-update/latest/source/CHANGELOG.md).

## Documentation

- [Updates](https://docs.rs/jflib/latest/jflib/guide/index.html#18-updates), in JFLib's guide
- [How updates work](https://github.com/JFLibrary/JFLib/blob/main/docs/UPDATES.md): the channel,
  what is checked, trial and rollback, each kind of installation
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#update), also on
  [docs.rs](https://docs.rs/jflib-update)

## License

MIT or Apache-2.0, at your choice.
