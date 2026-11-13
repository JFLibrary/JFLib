# jflib-style-netlab

NetLab, a sober and timeless look for apps built with [JFLib](https://jflib.org), first made for
[JFNetworker](https://github.com/JFLibrary/JFNetworker). A style decides how every widget of
JFLib's UI is drawn; the app's code stays the same whichever style it gets.

## What it looks like

Three flat grey layers, window, panel and field, without gradients or translucency. Every
control, card and dock group has a 1 DIP frame, corners are small (3 DIP on controls, 4 on
flyouts and dialogs, none on dock panels and grids), and only flyouts and dialogs cast a soft
shadow. The selection is a light tint of the accent with an accent frame, the keyboard focus a
thin accent ring just outside the control, and tabs carry an accent underline. It is compact:
controls are 28 DIP high, grid rows 22 DIP, text is Inter at 13 DIP. Its own accent is
"Signalblau" (#1F5FA8 light, #6FA3E0 dark). Thin scroll tracks stay visible, and views that
scroll only vertically keep their room free beside the content. Grids, navigation panes, dock
groups and the other data and shell parts have painters of their own, and many icons are its
own vector drawings. Like every JFLib style it has a light and a dark variant and works on every
system JFLib runs on.

## How an app gets it

Through the one dependency [`jflib`](https://crates.io/crates/jflib). `jflib::style` is the
style the environment variable `JFLIB_STYLE` names when the app is built (WinUI when it names
none), so `JFLIB_STYLE=netlab cargo run` builds an app with this look. The app hands it to
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
ui.styles_add(&[&jflib::style_netlab::STYLE, &jflib::style_winui::STYLE]);
ui.set_style("netlab");
```

`cargo jflib package --styles netlab,winui` builds the app once per style, and its setup lets
the person installing it choose. Of this crate `jflib` promises only `STYLE`. All of JFLib's
crates share one version and are released together.

## What's new in 0.2.0

NetLab is now one of JFLib's official styles: `jflib` brings it like the other four, as
`jflib::style_netlab`, as `jflib::style` with `JFLIB_STYLE=netlab`, and in packages with
`cargo jflib package --styles netlab`. Version 0.1.0 was published on its own, and an app had to
name the crate itself; such an app can drop that dependency now, and if it keeps it, it must use
the same version as `jflib`. Views that scroll only vertically keep the room of the visible
scroll bar free, so their content no longer runs under it, and the manifest now names the
licences of the fonts and icons the crate carries. Everything is in the
[changelog](https://docs.rs/crate/jflib-style-netlab/latest/source/CHANGELOG.md).

## Documentation

- [Styles, theme and fonts](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  and [app themes and the style at run time](https://docs.rs/jflib/latest/jflib/guide/index.html#34-app-themes-and-the-style-at-run-time)
  in JFLib's guide
- [How styles work](https://github.com/JFLibrary/JFLib/blob/main/docs/STYLES.md), and how to
  write one
- [The style contract](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#style-contract)
  of `ui`, and this crate on [docs.rs](https://docs.rs/jflib-style-netlab)

## License

MIT or Apache-2.0, at your choice. The crate carries its own copies of the Inter fonts, under
the SIL Open Font License 1.1 (`assets/OFL-Inter.txt`), and of an icon font whose motifs come
from WordCraft under the MIT license (`LICENSE-icons.txt`).
