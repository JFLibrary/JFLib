# jflib-style-classic

The look of Windows 2000 and Office 97 for apps built with [JFLib](https://jflib.org). A style
decides how every widget of JFLib's UI is drawn; the app's code stays the same whichever style
it gets.

## What it looks like

Square controls with grey 3D bevels, raised when they rest and sunken when pressed, a dotted
focus rectangle, a navy selection, progress bars of navy blocks in a sunken frame and title bars
with a gradient. Text is set in DejaVu Sans Condensed with Tahoma's ascender and descender, so
lines and baselines sit where Tahoma puts them. The grey face and bevels come from the style's
own palette: an app theme changes the other colours, but classic buttons stay grey. Like every
JFLib style it has a light and a dark variant and works on every system JFLib runs on.

## How an app gets it

Through the one dependency [`jflib`](https://crates.io/crates/jflib). `jflib::style` is the
style the environment variable `JFLIB_STYLE` names when the app is built (WinUI when it names
none), so `JFLIB_STYLE=classic cargo run` builds an app with this look. The app hands it to
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
ui.styles_add(&[&jflib::style_winui::STYLE, &jflib::style_classic::STYLE]);
ui.set_style("classic");
```

`cargo jflib package --styles winui,classic` builds the app once per style, and its setup lets
the person installing it choose. Of this crate `jflib` promises only `STYLE`. All of JFLib's
crates share one version and are released together.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-style-classic/latest/source/CHANGELOG.md)
says the same.

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  and [app themes and the style at run time](https://docs.rs/jflib/latest/jflib/guide/index.html#34-app-themes-and-the-style-at-run-time)
  in JFLib's guide
- [How styles work](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md), and how to
  write one
- [The style contract](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#style-contract)
  of `ui`, and this crate on [docs.rs](https://docs.rs/jflib-style-classic)

## License

MIT or Apache-2.0, at your choice. The DejaVu fonts it embeds are under the Bitstream Vera
license (`assets/DejaVu-LICENSE.txt`); its icons (MIT) come from
[`jflib-style-assets`](https://crates.io/crates/jflib-style-assets).
