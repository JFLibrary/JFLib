# jflib-ui

The GUI of [JFLib](https://jflib.org): windows, widgets, layout, menus and dialogs, and everything
that makes an app feel at home on its system, from screen readers to the system's file dialog.
It works in immediate mode and needs no standard library.

## Immediate mode

You don't build a tree of widget objects and keep it in step with your data. Your app describes
its window in one function, `frame`, and the library calls it whenever something happened:

```rust
impl App for Notes {
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(24), 12);
        ui.text(TS_TITLE, "Notes");
        ui.textbox(ID_TEXT, Some("Write something"), &mut self.text, 0);
        if ui.accent_button(ID_SAVE, "Save") & EV_ACT != 0 {
            ui.cmd_run(CMD_SAVE, 0);            // file work runs after the frame
        }
        ui.box_end();
    }
}
```

Your data stays yours. The library keeps only the small things a widget needs between frames,
such as the focus, a scroll position or a running animation. After every click or key the frame
is built again, so the window always shows the current state and there is nothing to update by
hand. Work that takes longer goes through commands, which reach `App::command` between frames, or
runs as an async task on the UI thread (`ui.spawn`).

## What's in it

The widgets an app needs: buttons, check boxes, sliders, text and number boxes, lists, grids and
trees, tabs, menus and toolbars, a navigation pane, docking panels and tool windows, charts, a
date picker. Larger parts are built in as well: a rich-text editor, a code editor with syntax
highlighting and folding, a Markdown and HTML viewer, a terminal, a node-and-edge diagram view.
Around them come several windows, the system's file dialogs (or the library's own), drag and drop
with other programs, the clipboard, global shortcuts, the tray, notifications, languages,
right-to-left layout and sharp output at any scale.

The look comes from a style crate: WinUI, the classic Windows look, KDE Plasma, macOS or NetLab,
each light and dark, and an app can switch between them while it runs. Every widget reports its
role, name and state to screen readers (UI Automation, AT-SPI, VoiceOver, TalkBack and an ARIA
tree in the browser), and everything can be reached with the keyboard.

Testing is part of it. Every app can render itself without a window system (`--shot out.png`)
and run test scripts that click, type and compare pixels (`cargo jflib test`). A live inspector
shows in a running app which line of your code drew a widget.

## Where it runs

Windows, Linux (Wayland and X11), macOS, iOS, Android and the browser, from the same code. On a
phone or a tablet the same app adapts: no caption buttons, a menu button instead of a menu bar
that does not fit, windows and dialogs that stay inside the screen.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), as `jflib::ui`.
`cargo install cargo-jflib` and `cargo jflib new hello` set up an app that runs at once. All of
JFLib's crates share one version and are released together.

## What's new in 0.2.0

Apps now run natively on macOS and iOS; on macOS their menus move into the system's menu bar. The
new `ui.form_factor()` tells phones and tablets from desktops, and the title bar, menu bar and
status bar adapt to them. A window can show GPU content next to its widgets (`ui.gpu_view`, for
example a 3D scene of `jflib-render3d`). Apps that host web content get what they need: canvases
that take the keyboard, raw key events, input methods for fields they draw themselves, more mouse
cursors, popups at any point and a web page's accessibility tree with tables and landmarks. The
file dialog chooses several files at once, `ui.calendar` shows a month in place, and dock panels
can move as a group. Among the fixes: text boxes with the same id in two windows no longer share
their caret and undo, the document views no longer open network or device paths for images, and
several inputs that made the UI panic are handled. Everything is in the
[changelog](https://docs.rs/crate/jflib-ui/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written,
  from the first window to accessibility and tests
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#ui): every public
  item of `ui`, also on [docs.rs](https://docs.rs/jflib-ui)

## License

MIT or Apache-2.0, at your choice.
