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

The browser with threads needs the Rust sources once: `rustup component add rust-src`. Without
them `web` builds the other two modes.

MIT or Apache-2.0, at your choice.
