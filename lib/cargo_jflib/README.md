# cargo-jflib

What Cargo does not do for a [JFLib](https://github.com/JFLibrary/JFLib) app. Install it once:

```sh
cargo install cargo-jflib
```

| Command | What it does |
| --- | --- |
| `cargo jflib new NAME` | a new app from the template of the crate `jflib`: a window, its tests, the settings for Windows and Linux |
| `cargo jflib test` | builds the app, then runs every `tests/*.uis` with `--script` |
| `cargo jflib web [--serve]` | the app for the browser in `target/jflib-web/NAME/`: scalar, SIMD and threads, the host page, the licences; `--serve` serves it with the headers threads need |
| `cargo jflib package [--styles winui,kde] [--formats setup,deb,..\|all]` | the app with every style built in, in `target/jflib-package/`: by default `NAME_Setup.exe` for Windows (a preview of each style to pick from, folder, shortcuts, file types, an uninstaller) and `DEB_VERSION_amd64.deb` for Linux (debconf asks for the look); with `--formats` also `msix`, `appimage` (one per style), `rpm` and `flatpak`; the licences of every crate inside |
| `cargo jflib android [--abi arm64-v8a\|x86_64] [--release] [--install]` | the app as an APK in `target/jflib-android/NAME-ABI-debug.apk`: the code for one ABI, its Java and C bridge, the licences; signed with a debug key it makes itself, `--release` with yours; `--install` hands it to `adb` |

The browser with threads needs the Rust sources once: `rustup component add rust-src`. Without
them `web` builds the other two modes.

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
built next to the app with its profiles and Cargo.lock. The previews are screenshots of the
program this machine runs (`--shot`).

| Format | Made by | Needs |
| --- | --- | --- |
| `setup`, `deb` | `cargo jflib` itself | nothing more |
| `appimage` | `cargo jflib` itself: the AppDir as SquashFS (LZ4) behind the runtime | nothing more; the runtime extracts the image on the first start (no FUSE) |
| `msix` | MakeAppx of the Windows SDK (`jfgui_MAKEAPPX` names another) | Windows; without it only the package's folder; unsigned |
| `rpm` | the `.spec` and its tarball; `rpmbuild` makes the `.rpm` | Linux, on Windows a WSL distribution with `rpmbuild` |
| `flatpak` | the manifest and its files; `flatpak-builder` makes `ID.flatpak` | Linux or WSL with `flatpak-builder` and the runtime `org.freedesktop.Platform//26.08` |

On Windows the Linux tools run in the WSL distribution `JFLIB_WSL_DISTRO`, else in the first
one that has the tool. What it reads from `PACKAGE` next to Cargo.toml:

| Key | Use |
| --- | --- |
| `display`, `version`, `publisher` | the shown name (default: the package's name), the version (default: Cargo.toml's), the publisher in Windows' list of apps |
| `styles`, `default_win`, `default_linux` | the styles to build in (default `winui`; `--styles` wins) and the one preselected |
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

The icons: `res/app.ico` for Windows, `res/app_48.png`, `app_128.png`, `app_256.png` for
Linux (a larger one is scaled down; without any a placeholder).

MIT or Apache-2.0, at your choice.
