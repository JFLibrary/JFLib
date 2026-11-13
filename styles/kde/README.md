# jflib-style-kde

The look of KDE Plasma's Breeze for apps built with [JFLib](https://jflib.org). A style decides
how every widget of JFLib's UI is drawn; the app's code stays the same whichever style it gets.

## What it looks like

Light buttons with a grey outline and small rounded corners. Hover, press and the keyboard focus
outline a control in the highlight blue, and default and checked buttons are tinted with it.
The window decoration is Breeze's: chevron caption buttons that show a round plate under the
pointer, a red one behind the close button. Dialogs put their buttons on the right with the
default first. Text is set in DejaVu Sans. Like every JFLib style it has a light and a dark variant,
takes the system's accent colour unless the app sets its own, and works on every system JFLib
runs on, not only on a Plasma desktop.

## How an app gets it

Through the one dependency [`jflib`](https://crates.io/crates/jflib). `jflib::style` is the
style the environment variable `JFLIB_STYLE` names when the app is built (WinUI when it names
none), so `JFLIB_STYLE=kde cargo run` builds an app with this look. The app hands it to
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
ui.styles_add(&[&jflib::style_winui::STYLE, &jflib::style_kde::STYLE]);
ui.set_style("kde");
```

`cargo jflib package --styles winui,kde` builds the app once per style, and its setup lets the
person installing it choose. Of this crate `jflib` promises only `STYLE`. All of JFLib's crates
share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-style-kde/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  and [app themes and the style at run time](https://docs.rs/jflib/latest/jflib/guide/index.html#34-app-themes-and-the-style-at-run-time)
  in JFLib's guide
- [How styles work](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md), and how to
  write one
- [The style contract](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#style-contract)
  of `ui`, and this crate on [docs.rs](https://docs.rs/jflib-style-kde)

## License

MIT or Apache-2.0, at your choice. The DejaVu fonts it embeds are under the Bitstream Vera
license (`assets/DejaVu-LICENSE.txt`); its icons (MIT) come from
[`jflib-style-assets`](https://crates.io/crates/jflib-style-assets).
