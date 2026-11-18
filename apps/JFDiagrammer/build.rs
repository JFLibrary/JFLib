// PERMISSIONS, PACKAGE, assets/ and res/app.ico of the app, and the linker arguments (lib/jflib_build);
// (JD9) the app's own steps in build_steps.rs: the built-in stencils (build_rs.py's derived builds
// for the browser, Android and other styles run them too)
mod build_steps;

fn main() {
    jflib_build::app();
    build_steps::run(std::path::Path::new("."));
}
