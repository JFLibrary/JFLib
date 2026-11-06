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

MIT or Apache-2.0, at your choice.
