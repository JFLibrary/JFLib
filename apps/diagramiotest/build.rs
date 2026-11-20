// PERMISSIONS, PACKAGE, assets/ and res/app.ico of the app, and the linker arguments (lib/jflib_build);
// (JD13) the text sources of JFDiagrammer's stencils and templates as a table for src/templates.rs
// (OUT_DIR/jfd_sources.rs: every stencil and template folder there is, so a new one is tested
// without a line here)
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    jflib_build::app();
    jfd_sources();
}

// (kind, folder, file name, include_str!) for the .txt, .master, .starter and .svg files of
// ../JFDiagrammer/stencils/<folder> and ../JFDiagrammer/templates/<folder>
fn jfd_sources() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let app = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../JFDiagrammer");
    let mut table = String::from("// built by build.rs from ../JFDiagrammer/stencils and templates\n&[\n");
    for kind in ["stencils", "templates"] {
        let root = app.join(kind);
        println!("cargo:rerun-if-changed={}", root.display());
        let mut dirs: Vec<PathBuf> = fs::read_dir(&root).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        dirs.sort();
        for dir in &dirs {
            let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let mut files: Vec<PathBuf> = fs::read_dir(dir)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x, "txt" | "master" | "starter" | "svg")))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            for f in &files {
                let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
                let _ = writeln!(table, "    ({kind:?}, {folder:?}, {name:?}, include_str!({:?})),", f.display().to_string());
            }
        }
    }
    table.push_str("]\n");
    fs::write(out.join("jfd_sources.rs"), table).expect("jfd_sources.rs");
}
