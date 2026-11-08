// PERMISSIONS, PACKAGE, assets/ and res/app.ico of the app, and the linker arguments (lib/jflib_build)
fn main() {
    jflib_build::Build::app().part_of("permcli", "..").run();
}
