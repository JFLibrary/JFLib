# cargo-jflib

What Cargo does not do for a [JFLib](https://github.com/JFLibrary/JFLib) app. Install it once:

```sh
cargo install cargo-jflib
```

| Command | What it does |
| --- | --- |
| `cargo jflib new [--std] NAME` | a new app from the template of the crate `jflib`: a window, its tests, the settings for Windows and Linux; `--std`: the same app with the standard library (`jflib`'s feature `std`), so it may use `std` crates; its `Cargo.toml` says how it catches panics instead (the feature `unwind`, `panic = "unwind"`) |
| `cargo jflib test [--features F,..] [--no-default-features] [--dir DIR]` | builds the app (on Linux always with `--target`, this machine's when none is given: the template's flags must not reach the build scripts), then runs every `tests/*.uis` with `--script`; a line `#!only windows linux ..` among a script's first lines runs it on these systems only (`target_os` of `--target`, else of this machine); `--features` and `--no-default-features` go to the build, `--dir DIR` runs `DIR/*.uis` instead (relative to the package's folder) |
| `cargo jflib watch [-- ARGS]` | hot reload: each save of the project's files builds the app again and restarts it in its session, the input replayed so it comes back where it was; a failed build keeps the running version and shows the error in its inspector |
| `cargo jflib web [--serve]` | the app for the browser in `target/jflib-web/NAME/`: scalar, SIMD and threads, the host page, the licences; `--serve` serves it with the headers threads need |
| `cargo jflib package [--styles winui,kde] [--formats setup,deb,..\|all] [--sign] [--repo DIR] [--update DIR]` | the app with every style built in, in `target/jflib-package/`: by default `NAME_Setup.exe` for Windows (a preview of each style to pick from, folder, shortcuts, file types, an uninstaller) and `DEB_VERSION_amd64.deb` for Linux (debconf asks for the look), an app with jflib's feature `std` only the one of the system it is made on; with `--formats` also `msix`, `appimage` (one per style), `rpm` and `flatpak`; the licences of every crate inside |
| `cargo jflib keygen FILE` | a key for the signed update channel (`package --update`) and its public key for the app |
| `cargo jflib android [--abi arm64-v8a\|x86_64] [--release] [--install]` | the app as an APK in `target/jflib-android/NAME-ABI-debug.apk`: the code for one ABI, its Java and C bridge, the licences; signed with a debug key it makes itself, `--release` with yours; `--install` hands it to `adb` |
| `cargo jflib mac [--release] [--run [-- ARGS]]` | on a Mac: the app as `target/jflib-mac/NAME.app` for Apple Silicon: `Info.plist` from `PACKAGE` (`display`, `version`, `app_id`, `filetypes` as document types, `schemes` as URL types), the icon from `res/app_*.png` as `NAME.icns`, signed ad hoc; with `sandbox` in `PERMISSIONS` the App Sandbox and the entitlements its rules need; `--run` starts it |

The browser with threads needs the Rust sources once: `rustup component add rust-src`. Without
them `web` builds the other two modes. An app with std has no threads mode (rustup ships no std
with atomics), and `package` makes its packages on their own system: `setup` and `msix` on
Windows, the Linux formats on Linux.

Android needs the Android SDK (build-tools, `platforms;android-36`), the NDK, a JDK 17 or newer and
the Rust target (`rustup target add aarch64-linux-android x86_64-linux-android`). The SDK is found
through `ANDROID_SDK_ROOT`, `ANDROID_HOME` or where Android Studio puts it, the NDK as the newest
below the SDK or `ANDROID_NDK_HOME`. The manifest comes from `PACKAGE` (`app_id`, `display`,
`version`) and `PERMISSIONS` (location), unless the app has its own `android/AndroidManifest.xml`;
`android/res/` is compiled in when it is there. A release key: `jfgui_ANDROID_KEYSTORE`,
`jfgui_ANDROID_KEYSTORE_PASS`, `jfgui_ANDROID_KEY_ALIAS` and `jfgui_ANDROID_KEY_PASS`.

`package` builds the app with `--release` once per style (`JFLIB_STYLE`) for Windows and for
Linux, both from any machine: the Linux program links without a Linux machine, the setup stub
(the crate `jflib-installer`) and the AppImage runtime (`jflib-appimagert`, one static file) are
built next to the app with its `[profile.release]` and Cargo.lock, always with `panic = "abort"` (they are
programs without std, also for an app with the feature `unwind`); where the app has JFLib's crates
from paths (an app in JFLib's workspace), the stubs take them from there too, and when its `jflib` comes
from a path (a checkout under `[patch.crates-io]`), the stubs take all their crates from that checkout's
workspace, `jflib-installer` included, which the app itself does not depend on. The previews are screenshots of the
program this machine runs (`--shot`).

| Format | Made by | Needs |
| --- | --- | --- |
| `setup`, `deb` | `cargo jflib` itself | nothing more |
| `appimage` | `cargo jflib` itself: the AppDir as SquashFS (LZ4) behind the runtime | nothing more; the runtime extracts the image on the first start (no FUSE) |
| `msix` | MakeAppx of the Windows SDK (`jfgui_MAKEAPPX` names another) | Windows; without it only the package's folder; signed like the setup |
| `rpm` | the `.spec` and its tarball; `rpmbuild` makes the `.rpm` | Linux, on Windows a WSL distribution with `rpmbuild` |
| `flatpak` | the manifest and its files; `flatpak-builder` makes `ID.flatpak` | Linux or WSL with `flatpak-builder` and the runtime `org.freedesktop.Platform//26.08` |

On Windows the Linux tools run in the WSL distribution `JFLIB_WSL_DISTRO`, else in the first
one that has the tool. What it reads from `PACKAGE` next to Cargo.toml:

| Key | Use |
| --- | --- |
| `display`, `version`, `publisher` | the shown name (default: the package's name), the version (default: Cargo.toml's), the publisher in Windows' list of apps |
| `styles`, `default_win`, `default_linux` | the styles to build in (default `winui`; `--styles` wins) and the one preselected; besides the library's (winui, classic, kde, macos, netlab) a style of the app's own: a crate of its dependencies with `[package.metadata.jflib] style = "NAME"` (and an `INFO` file: its name and description for the setup), which the app uses itself -- it is built without `JFLIB_STYLE` |
| `summary`, `description`, `generic`, `categories` | the .desktop entry and the .deb's description |
| `deb_name`, `section`, `maintainer` | the .deb's name (default: the package's name), its section (`utils`) and maintainer (default: Cargo.toml's first author) |
| `filetypes`, `schemes` | the file types (`ext[,ext]\|mime/type\|description[\|own]`, `;` between them) and URL schemes the app opens, registered by the setup and the .deb |
| `app_id` | the reverse-DNS id (default `org.example.NAME`): the .desktop file of the AppImage and Flatpak, AppStream, the MSIX identity |
| `homepage`, `license` | AppStream, the .spec (default: Cargo.toml's) |
| `appimage_update` | the AppImage's update information (`.upd_info`, e.g. `zsync\|https://...`) |
| `flatpak_runtime`, `flatpak_args` | another runtime than `org.freedesktop.Platform//26.08`, more finish-args |
| `msix_name`, `msix_publisher`, `msix_alias`, `language` | the MSIX identity (default: the app id, `CN=<publisher>`), an execution alias, the package language (`de-DE`) |
| `appinstaller_url`, `appinstaller_hours` | where the .msix and an `.appinstaller` file are published (App Installer updates from there) |

`PERMISSIONS` next to Cargo.toml sets the Flatpak's finish-args (the sandbox holes) and the
MSIX's capabilities; without one both allow everything.

### Signing, apt repository, update channel

`package` signs with what the environment names, and says what it left unsigned; `--sign`
makes every missing signature an error.

| Variable | Signs |
| --- | --- |
| `jfgui_SIGN_PFX` + `jfgui_SIGN_PFX_PASS`, or `jfgui_SIGN_THUMBPRINT` (a certificate in the store); `jfgui_SIGN_TIMESTAMP` (an RFC 3161 server) | Authenticode by signtool (`jfgui_SIGNTOOL`, else the Windows SDK's): every style's program, the setup, the MSIX (the certificate's subject must be the MSIX publisher) |
| `jfgui_GPG_KEY` (+ `jfgui_GPG_HOME`, `jfgui_GPG_PASSPHRASE` in Linux / WSL) | the AppImages (`.sha256_sig` / `.sig_key`, as `appimagetool --sign`), the apt repository, the Flatpak repository and bundle |

`--repo DIR` writes a flat apt repository with the `.deb` (`Packages`, `Release`, signed
`InRelease`, `Release.gpg` and the key `repo.gpg`):
`deb [signed-by=/etc/apt/keyrings/NAME.gpg] https://example.org/apt ./`.

`--update DIR` writes a signed update channel per style for JFLib's `update` module:
`DIR/STYLE/update.json` and `update.json.sig` with the style's Windows program, the `.deb` and
the style's AppImage (`--update-url URL`: absolute addresses; PACKAGE `update_app`,
`update_notes`, `update_expires_days`). The key comes from `--update-key FILE`,
`jfgui_UPDATE_KEY_FILE` or `jfgui_UPDATE_KEY`; `cargo jflib keygen FILE` makes one and prints
its public key for `update::Config::new(APP, VERSION, URL, &["PUBLIC KEY"])`. Keep the key out of
the repository: whoever has it can ship code to every installation. The signatures are made by
a small program built from JFLib's own crates (`jflib-crypto`).

The icons: `res/app.ico` for Windows, `res/app_48.png`, `app_128.png`, `app_256.png` for
Linux (a larger one is scaled down; without any a placeholder).

MIT or Apache-2.0, at your choice.
