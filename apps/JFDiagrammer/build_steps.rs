// build_steps -- JFDiagrammer's own steps of its build script, beside jflib_build's (build.rs;
// build_rs.py's derived builds for the browser, Android and other styles call run() too, with the
// app's folder)
//
// (JD9) the built-in stencils: every stencils/<folder>/stencil.txt is built with
// diagram_io::stencil into OUT_DIR/<file>_m.vssx (metric) and <file>_u.vssx (US units), and
// OUT_DIR/stencils.rs lists them for src/stencils.rs (PLAN.md V2: own stencils are .vssx, made
// from the sources in the repository when the app is built; the format: stencils/README.md).
// A source error stops the build with file and line.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use diagram_io::vsdx::Measure;

/// the steps for the app in folder `app` (its Cargo.toml's folder)
pub fn run(app: &Path) {
    stencils(app);
}

// plat's start of a program with std (plat::std_start, linked in with the stencil builder's
// crates) refers to the jfgui_main a program's ui::main! defines; a build script has a main of
// its own, so this stand-in is never called (as ui::test_main! does for test binaries)
#[allow(dead_code)]
#[unsafe(no_mangle)]
fn jfgui_main() -> i32 {
    0
}

fn stencils(app: &Path) {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let root = app.join("stencils");
    println!("cargo:rerun-if-changed={}", root.display());
    // (JD13) the shared network symbols the masters name (svg: ../../symbols/...; later a submodule)
    println!("cargo:rerun-if-changed={}", app.join("symbols").display());
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("stencil.txt").is_file()).collect())
        .unwrap_or_default();
    dirs.sort();
    let mut table = String::from("// built by build_steps.rs from stencils/ (PLAN.md V2)\n&[\n");
    for dir in &dirs {
        let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let files = |name: &str| fs::read_to_string(dir.join(name)).ok();
        let mut stem = String::new();
        let mut info = (String::new(), String::new());
        for (measure, suffix) in [(Measure::Metric, "m"), (Measure::Us, "u")] {
            let (built, bytes) = diagram_io::stencil::build_vssx(&files, measure).unwrap_or_else(|e| panic!("stencils/{folder}/{e}"));
            let path = out.join(format!("{}_{suffix}.vssx", built.file));
            fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            stem = built.file.clone();
            info = (built.stencil.clone(), built.category.clone());
        }
        let _ = writeln!(
            table,
            "    Builtin {{ file: {stem:?}, stencil: {:?}, category: {:?}, metric: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{stem}_m.vssx\")), us: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{stem}_u.vssx\")) }},",
            info.0, info.1
        );
    }
    table.push_str("]\n");
    fs::write(out.join("stencils.rs"), table).expect("stencils.rs");
}
