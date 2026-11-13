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

The user first picks the look: the styles the app was packed with, each with its description and
a preview. Then come the folder, by default `%LOCALAPPDATA%\Programs\<app>`, so that no
administrator rights are needed, and the options: a shortcut on the desktop, an entry in the
Start menu, an entry in Windows' installed apps, and starting the app when it is done. An app that
is a web browser also gets the option to become the default browser; Windows lets no program take
that itself, so the setup opens the app's page in Windows' Settings, where the user picks it.

The setup installs the chosen build, the app's further files, the shortcuts and the uninstall
entry, all for the current user. It registers the file types and URL schemes the app's `PACKAGE`
declares and gives the program, its shortcuts and its file types the app's AppUserModelID, so
that Windows groups them under one taskbar button. Running the setup again installs another
style over the old one, and the app's settings stay.

It also copies itself, without the app, as `uninstall.exe`. Uninstalling removes exactly what the
setup recorded in `install.ini` next to the program: every file it installed, the folders that
held them once they are empty, the shortcuts, the registry keys of the file types and the
uninstall entry. Files the user put there stay.

The pages are in German; only the default-browser option also speaks English. On the command
line `--dir PATH` and `--style ID` preset the folder and the look.

## What's new in 0.2.0

A setup of a web browser offers to make it the default browser and registers it the way Windows
expects one: through Default Programs, without taking the keys of `http` and `https` from the
system. Uninstalling now removes every file of a large installation, also beyond the 4096 bytes
at which `install.ini` used to cut the list, together with the folders that held them, and a
package may hold up to 65536 files instead of 256. A setup that cannot write `install.ini` fails
instead of leaving an installation nothing can remove. Everything is in the
[changelog](https://docs.rs/crate/jflib-installer/latest/source/CHANGELOG.md).

## Documentation

- [Packaging](https://github.com/JFLibrary/JFLib/blob/main/docs/PACKAGING.md#windows-setupexe):
  what the setup installs, its options, its test mode and the layout of its payload
- [cargo-jflib](https://crates.io/crates/cargo-jflib), which builds the packages
- [docs.rs](https://docs.rs/jflib-installer)

## License

MIT or Apache-2.0, at your choice.
