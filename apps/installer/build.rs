// the setup's icon (lib/installer/res/app.ico) and the linker arguments (lib/jflib_build)
fn main() {
    jflib_build::Build::app().part_of("installer", "../../lib/installer").run();
}
