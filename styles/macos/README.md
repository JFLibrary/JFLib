# jflib-style-macos

The look of macOS for apps built with [JFLib](https://jflib.org). A style decides how every
widget of JFLib's UI is drawn; the app's code stays the same whichever style it gets.

## What it looks like

macOS Sonoma's: a grey window, white push buttons with a hairline and a darker bottom edge, a
blue default button and a blue selection, pop-up buttons with the blue chevron square, scroll
bars that float over the content, and the three traffic-light buttons at the left of a title bar
with a centred title. Dialogs put their buttons on the right with the default last. JFLib Sans
stands in for SF Pro, and the icons are Microsoft's Fluent UI System Icons. Like every JFLib
style it has a light and a dark variant and takes the system's accent colour unless the app sets
its own.

The style is a look, not the port to the Mac: JFLib apps run natively on macOS in any style,
and this one works on every system JFLib runs on.

## How an app gets it

Through the one dependency [`jflib`](https://crates.io/crates/jflib). `jflib::style` is the
style the environment variable `JFLIB_STYLE` names when the app is built (WinUI when it names
none), so `JFLIB_STYLE=macos cargo run` builds an app with this look. The app hands it to
`ui::launch`, as the template of `cargo jflib new` does:

```rust
use jflib::{style, ui};

ui::main!({
    ui::launch(&DESC, Notes::default(), &style::STYLE)
});
```

All five of JFLib's styles are also there under their own names, so an app can let its user
switch while it runs:

```rust
ui.styles_add(&[&jflib::style_winui::STYLE, &jflib::style_macos::STYLE]);
ui.set_style("macos");
```

`cargo jflib package --styles winui,macos` builds the app once per style, and its setup lets the
person installing it choose. Of this crate `jflib` promises only `STYLE`. All of JFLib's crates
share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-style-macos/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  and [app themes and the style at run time](https://docs.rs/jflib/latest/jflib/guide/index.html#34-app-themes-and-the-style-at-run-time)
  in JFLib's guide
- [How styles work](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md), and how to
  write one
- [The style contract](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#style-contract)
  of `ui`, and this crate on [docs.rs](https://docs.rs/jflib-style-macos)

## License

MIT or Apache-2.0, at your choice. Its fonts (SIL Open Font License 1.1) and icons (MIT) come
from [`jflib-style-assets`](https://crates.io/crates/jflib-style-assets), whose license texts
this crate carries in `assets/` as well.
