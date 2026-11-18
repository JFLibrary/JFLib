// examples -- the reference scripts built into the program (examples/*.jfs) and their reports
// (examples/out/NAME/report.json): --examples builds them with pictures, --check compares their reports, and
// --checksum folds their meshes.  Built in, so the browser's build (no files) runs them too.

use alloc::string::String;

/// a reference script, its modules and the report it must give
pub struct Example {
    pub name: &'static str,
    pub source: &'static str,
    /// the report of `modeltool --examples examples/out` (selftest compares it without files, views and timing)
    pub report: &'static str,
}

/// the reference scripts
pub const ALL: &[Example] = &[
    Example { name: "kiste", source: include_str!("../examples/kiste.jfs"), report: include_str!("../examples/out/kiste/report.json") },
    Example { name: "treppe", source: include_str!("../examples/treppe.jfs"), report: include_str!("../examples/out/treppe/report.json") },
    Example { name: "tuer", source: include_str!("../examples/tuer.jfs"), report: include_str!("../examples/out/tuer/report.json") },
    Example { name: "haus", source: include_str!("../examples/haus.jfs"), report: include_str!("../examples/out/haus/report.json") },
    Example { name: "saeule", source: include_str!("../examples/saeule.jfs"), report: include_str!("../examples/out/saeule/report.json") },
];

/// the modules the examples import
pub const MODULES: &[(&str, &str)] = &[("haus_teile.jfs", include_str!("../examples/haus_teile.jfs"))];

/// import() for the built-in examples
pub fn loader(path: &str) -> Result<String, String> {
    MODULES.iter().find(|m| m.0 == path).map(|m| String::from(m.1)).ok_or_else(|| String::from("die Datei gibt es nicht"))
}
