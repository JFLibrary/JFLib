# jflib-style-winui

The look of Windows 11 for apps built with [JFLib](https://jflib.org), and JFLib's default style.
A style decides how every widget of JFLib's UI is drawn; the app's code stays the same whichever
style it gets.

## What it looks like

WinUI 3's Fluent design: rounded controls with a hairline border whose bottom edge is a little
darker, surfaces in Mica's colours, and the accent colour on accent buttons, checked boxes,
switches that are on, sliders and progress bars, and as a small pill beside the selected row of
a list. Text is set in
JFLib Sans, a subset of Selawik, and the icons are Microsoft's Fluent UI System Icons. Like every
JFLib style it has a light and a dark variant, takes the system's accent colour unless the app
sets its own, and works on every system JFLib runs on, not only on Windows.

## How an app gets it

Through the one dependency [`jflib`](https://crates.io/crates/jflib). `jflib::style` is the
style the environment variable `JFLIB_STYLE` names when the app is built, and this one when it
names none. The app hands it to `ui::launch`, as the template of `cargo jflib new` does:

```rust
use jflib::{style, ui};

ui::main!({
    ui::launch(&DESC, Notes::default(), &style::STYLE)
});
```

All five of JFLib's styles are also there under their own names, so an app can let its user
switch while it runs:

```rust
ui.styles_add(&[&jflib::style_winui::STYLE, &jflib::style_kde::STYLE]);
ui.set_style("winui");
```

`cargo jflib package --styles winui,kde` builds the app once per style, and its setup lets the
person installing it choose. Of this crate `jflib` promises only `STYLE`. All of JFLib's crates
share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-style-winui/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  and [app themes and the style at run time](https://docs.rs/jflib/latest/jflib/guide/index.html#34-app-themes-and-the-style-at-run-time)
  in JFLib's guide
- [How styles work](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md), and how to
  write one
- [The style contract](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#style-contract)
  of `ui`, and this crate on [docs.rs](https://docs.rs/jflib-style-winui)

## License

MIT or Apache-2.0, at your choice. Its fonts (SIL Open Font License 1.1) and icons (MIT) come
from [`jflib-style-assets`](https://crates.io/crates/jflib-style-assets), whose license texts
this crate carries in `assets/` as well.
