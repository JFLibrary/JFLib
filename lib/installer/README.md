# jflib-installer

The setup program of [JFLib](https://jflib.org)'s Windows packages. When `cargo jflib package`
packs an app for Windows, it builds this crate into a small program and appends the app to it:
one build of the app per style, a preview picture of each, and further files such as the
licences of the crates inside. The result is the `NAME_Setup.exe` the app's users run.

## Do you need this crate?

Not in your code. It is on crates.io so that `cargo jflib package` can build the setup for an app
outside JFLib's repository: it makes a stub crate whose only line is `installer::main!();`,
depends on `jflib-installer` in the same version as the app's `jflib`, and builds it like the
app. The setup runs on Windows only. All of JFLib's crates share one version and are released
together.

## What the setup does

When the app was packed with more than one style, the user first picks the look, each style with
its description and a preview. An app with one style, as `cargo jflib package` packs it by
default, skips this page. Then the user chooses who the app is for:

- **Just me** (the default) installs into `%LOCALAPPDATA%\Programs\<app>` and needs no administrator
  rights.
- **All users of this computer** installs into `C:\Program Files\<app>`, with the shortcuts on the
  common desktop and Start menu and the registry keys below `HKEY_LOCAL_MACHINE`. A setup without
  administrator rights starts itself again with them (the UAC prompt) and carries on when that is
  done.

The same page has the folder and the shortcuts: on the desktop, in the Start menu, and the entry in
Windows' installed apps. The last page can start the app. An app that is a web browser can also
become the default browser there; Windows lets no program take that itself, so the setup opens the
app's page in Windows' Settings, where the user picks it.

The setup installs the chosen build, the app's further files, the shortcuts and the uninstall
entry. It registers the file types and URL schemes the app's `PACKAGE` declares and gives the
program, its shortcuts and its file types the app's AppUserModelID, so that Windows groups them
under one taskbar button. Running the setup again installs another style over the old one, and
the app's settings stay.

It also copies itself, without the app, as `uninstall.exe`. Uninstalling removes exactly what the
setup recorded in `install.ini` next to the program: every file it installed, the folders that
held them once they are empty, the shortcuts, the registry keys of the file types and the
uninstall entry. Files the user put there stay.

The pages speak the user's language: German, English, French, Spanish, Italian, Dutch, Polish,
Arabic or Hebrew, otherwise English. The styles' names and descriptions come along when their
`INFO` files have them in that language.

## Without questions

Package managers, deployment tools and scripts install and remove the app without a window:

```text
App_Setup.exe --quiet [--all-users] [--dir PATH] [--style ID] [--no-desktop] [--no-start-menu]
              [--no-uninstall-entry] [--launch] [--log FILE] [--lang CODE]
uninstall.exe --uninstall --quiet [--log FILE]
```

The exit code is 0 when it is done, 1 when it failed, 2 for a wrong command line (an unknown
option or style) and 740 for an installation for all users, or the removal of one, without
administrator rights. The reason goes to standard output and into the log. The uninstall entry
carries this command as its `QuietUninstallString`. Without `--quiet`, the same options preset the
pages.

## What's new in 0.2.1

Windows: URL schemes are registered as `"<exe>" -- "%1"`, so a quote in a link can no longer add
options to the program. Apps registered before keep the old command until they are registered again.
Everything is in the [changelog](https://docs.rs/crate/jflib-installer/latest/source/CHANGELOG.md).

## Documentation

- [Packaging](https://github.com/JFLibrary/JFLib/blob/main/docs/PACKAGING.md#windows-setupexe):
  what the setup installs, its options, its test mode and the layout of its payload
- [cargo-jflib](https://crates.io/crates/cargo-jflib), which builds the packages
- [docs.rs](https://docs.rs/jflib-installer)

## License

MIT or Apache-2.0, at your choice.
