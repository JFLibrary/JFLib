// modeltool -- the feedback loop for modelling in JFScript (docs/GEOM.md, GAME.md stage G7g): a script describes a
// model, lib/geom builds the meshes, modeltool checks them, draws them from fixed cameras and writes a report
//
//   modeltool FILE.jfs [--out DIR] [--param NAME=VALUE ...] [--size PX] [--no-grid] [--no-images] [--force]
//        run the script, check the model, write DIR/report.json, DIR/model.glb, DIR/front.png, side.png, top.png,
//        iso.png and DIR/sheet.png (the four in one); DIR defaults to FILE without .jfs plus ".out".  An unchanged
//        script with the same parameters and modules is not built again (the report's key; --force builds anyway).
//        Exit code 0: built and every check passed; 1: the script failed; 3: built, but a check failed; 2: usage.
//   modeltool --report FILE.jfs [--param ...]    only the report, to stdout (no files, no pictures)
//   modeltool --api                              the script's functions with their parameters
//   modeltool --check [--quick] | --test NAME    lib/geom against independent references, the examples' reports
//                                                (tests/*.uis); groups: prims ops normals checks poly gltf script examples
//   modeltool --checksum                         FNV-1a over the bits of every example's meshes and a fixed set of
//                                                operations: the same on every target
//   modeltool --examples DIR [--size PX]         the built-in examples with pictures into DIR/NAME/
//   modeltool --watch FILE.jfs [--param ...]     the observation mode: a window with the model, built again when
//                                                the script or a module is saved (watch.rs, watch_ui.rs)
//   modeltool --version | --help
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod checksum;
mod examples;
mod export;
mod host;
mod json;
mod refs;
#[cfg(not(target_arch = "wasm32"))]
mod render;
mod run;
mod selftest;
mod selftest_csg;
mod watch;
#[cfg(not(target_arch = "wasm32"))]
mod watch_ui;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use json::J;

pub const APP_VERSION: &str = rt::app_version!();

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn err(s: &str) {
    plat::write_stderr(s.as_bytes());
}

/// what a build writes besides the report
pub struct Options {
    pub size: u32,
    pub no_grid: bool,
    pub no_images: bool,
    pub force: bool,
}

fn usage() -> i32 {
    out("Aufruf: modeltool DATEI.jfs [--out ORDNER] [--param NAME=WERT ...] [--size PX] [--no-grid] [--no-images] [--force]\n");
    out("        modeltool --report DATEI.jfs [--param ...] | --api | --check [--quick] | --test NAME | --checksum\n");
    out("        modeltool --examples ORDNER [--size PX] | --watch DATEI.jfs [--param ...] | --version | --help\n");
    out("Schreibt ORDNER/report.json, model.glb, front.png, side.png, top.png, iso.png und sheet.png (vorn | rechts über oben | schräg).\n");
    out("Exit-Code 0: gebaut und alle Prüfungen bestanden; 1: Fehler im Skript; 3: gebaut, aber eine Prüfung fehlgeschlagen; 2: Aufruf.\n");
    0
}

/// the sources the problems point into: the script and its modules
fn sources_of(run: &run::Run, src: &str) -> Vec<(String, String)> {
    let mut v = alloc::vec![(run.file.clone(), String::from(src))];
    v.extend(run.imports.iter().cloned());
    v
}

/// the console summary of a built model
fn summary(r: &run::Run, a: &run::Analysis, out_dir: Option<&str>, grid: Option<f64>, render_error: Option<&str>) -> String {
    let w = &a.whole;
    let s = w.size();
    let mut t = format!(
        "{}: {} - {} Teile, {} Materialien, {} Dreiecke, {} Ecken; {} x {} x {} m (B x H x T); Fläche {} m², Volumen {} m³\n",
        r.file,
        if a.ok() { "ok" } else { "PRÜFUNG FEHLGESCHLAGEN" },
        r.model.parts.len(),
        r.model.materials.len(),
        w.triangles,
        w.vertices,
        geom::fnum(round3(s.x)),
        geom::fnum(round3(s.y)),
        geom::fnum(round3(s.z)),
        geom::fnum(round3(w.area)),
        geom::fnum(round3(w.volume)),
    );
    let fnd = run::findings(&r.model, a);
    if fnd.is_empty() {
        let mut ok = String::from("  geprüft: geschlossen, Normalen außen, keine entarteten Dreiecke");
        if let Some(b) = r.model.budget {
            ok.push_str(&format!(", Budget {} eingehalten", b));
        }
        if !r.model.expects.is_empty() {
            ok.push_str(", Maße wie erwartet");
        }
        t.push_str(&ok);
        t.push('\n');
    } else {
        for f in &fnd {
            t.push_str(&format!("  FEHLER: {}\n", f));
        }
    }
    if !r.output.is_empty() {
        t.push_str("  Ausgabe des Skripts:\n");
        for l in r.output.lines() {
            t.push_str(&format!("    {}\n", l));
        }
    }
    if let Some(d) = out_dir {
        if let Some(e) = render_error {
            t.push_str(&format!("  keine Bilder: {}\n", e));
        } else if let Some(g) = grid {
            t.push_str(&format!("  Bilder (Gitter {} m): {}sheet.png (vorn | rechts / oben | schräg), front.png, side.png, top.png, iso.png\n", geom::fnum(g), d));
        }
        t.push_str(&format!("  Bericht: {}report.json; glTF: {}model.glb\n", d, d));
    }
    t
}

fn round3(v: f64) -> f64 {
    geom::math::floor(v * 1000.0 + 0.5) / 1000.0
}

/// the timing of a run as the report's last line
fn timing(r: &run::Run, a: Option<&run::Analysis>, render_us: u64, export_us: u64, total_us: u64) -> J {
    let ms = |us: u64| J::Num(us as f64 / 1000.0);
    J::obj(alloc::vec![
        ("script_ms", ms(r.script_us)),
        ("geometry_ms", ms(r.host_us)),
        ("checks_ms", ms(a.map_or(0, |a| a.check_us))),
        ("render_ms", ms(render_us)),
        ("export_ms", ms(export_us)),
        ("total_ms", ms(total_us)),
        ("fuel", J::Int(r.fuel as i64)),
    ])
}

/// whether DIR's report is of this key with these options and every file is there: (ok, its findings)
fn cached(dir: &str, src: &str, params: &[(String, String)], o: &Options) -> Option<(bool, Vec<String>)> {
    let text = plat::read_file(&format!("{dir}report.json"))?;
    let big = usize::MAX / 2;
    let limits = agent_core::json::Limits { bytes: big, depth: 32, values: big, string_bytes: big, members: big };
    let v = agent_core::json::parse_with_limits(&text, limits).ok()?;
    let key = v.get("key")?.as_str()?;
    // the modules as they are now
    let mut imports = Vec::new();
    for i in v.get("imports")?.as_array()? {
        let p = i.get("path")?.as_str()?;
        let s = String::from_utf8(plat::read_file(p)?).ok()?;
        imports.push((String::from(p), s));
    }
    if run::cache_key(src, params, &imports) != key {
        return None;
    }
    let views = v.get("views")?;
    let size = views.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
    let grid = views.get("grid_on").and_then(|s| s.as_bool()).unwrap_or(false);
    let images = views.get("images").and_then(|s| s.as_bool()).unwrap_or(false);
    if (images, size as u32, grid) != (!o.no_images, if o.no_images { 0 } else { o.size }, !o.no_grid && !o.no_images) {
        return None;
    }
    let mut names = alloc::vec!["model.glb"];
    if images {
        names.extend_from_slice(&["sheet.png", "front.png", "side.png", "top.png", "iso.png"]);
    }
    if names.iter().any(|n| plat::file_exists(&format!("{dir}{n}")) != 1) {
        return None;
    }
    let findings = v.get("findings")?.as_array()?.iter().filter_map(|f| f.as_str().map(String::from)).collect();
    Some((v.get("ok")?.as_bool()?, findings))
}

/// build one script into `dir` (ending in "/"): report, glTF, pictures
fn build(file: &str, src: &str, dir: &str, params: &[(String, String)], o: &Options, loader: &mut dyn FnMut(&str) -> Result<String, String>) -> i32 {
    let t0 = plat::time_us();
    let r = run::run_script(file, src, params, loader);
    if !plat::mkdir(dir.trim_end_matches('/')) {
        err(&format!("modeltool: kann den Ordner {} nicht anlegen\n", dir));
        return 2;
    }
    if !r.ok() {
        let sources = sources_of(&r, src);
        for p in &r.problems {
            out(&run::format_problem(p, &sources));
        }
        let rep = run::report(&r, None, None, Some(timing(&r, None, 0, 0, plat::time_us().saturating_sub(t0))));
        let _ = plat::write_file(&format!("{dir}report.json"), json::write(&rep).as_bytes());
        out(&format!("  Bericht: {}report.json\n", dir));
        return 1;
    }
    let a = run::analyse(&r.model);
    let te = plat::time_us();
    let glb = export::glb(&r.model);
    let export_us = plat::time_us().saturating_sub(te);
    match &glb {
        Ok(b) => {
            if !plat::write_file(&format!("{dir}model.glb"), b) {
                err(&format!("modeltool: kann {}model.glb nicht schreiben\n", dir));
            }
        }
        Err(e) => err(&format!("modeltool: {}\n", e)),
    }
    let tr = plat::time_us();
    let (pics, render_error) = if o.no_images { (None, None) } else { pictures(&r.model, o) };
    let render_us = plat::time_us().saturating_sub(tr);
    let mut files: Vec<(&str, J)> = alloc::vec![("report", J::str("report.json")), ("glb", J::str("model.glb"))];
    let mut views: Vec<(&str, J)> = alloc::vec![("images", J::Bool(pics.is_some())), ("size", J::Int(if o.no_images { 0 } else { o.size as i64 })), ("grid_on", J::Bool(!o.no_grid && !o.no_images))];
    let mut grid = None;
    if let Some((list, sheet, g, adapter)) = &pics {
        let _ = plat::write_file(&format!("{dir}sheet.png"), sheet);
        files.push(("sheet", J::str("sheet.png")));
        for (n, png) in list {
            let _ = plat::write_file(&format!("{dir}{n}.png"), png);
            files.push((n, J::Str(format!("{n}.png"))));
        }
        views.push(("grid", J::Num(*g)));
        views.push(("sheet", J::str("front | side over top | iso")));
        views.push(("cameras", J::str("front: from +Z; side: from +X; top: from +Y, front at the bottom; the three at one scale; iso: from front right above")));
        views.push(("adapter", J::str(adapter)));
        grid = Some(*g);
    }
    if let Some(e) = &render_error {
        views.push(("error", J::str(e)));
    }
    let outputs = run::Outputs { files: J::obj(files), views: J::obj(views) };
    let rep = run::report(&r, Some(&a), Some(&outputs), Some(timing(&r, Some(&a), render_us, export_us, plat::time_us().saturating_sub(t0))));
    if !plat::write_file(&format!("{dir}report.json"), json::write(&rep).as_bytes()) {
        err(&format!("modeltool: kann {}report.json nicht schreiben\n", dir));
        return 2;
    }
    out(&summary(&r, &a, Some(dir), grid, render_error.as_deref()));
    if a.ok() { 0 } else { 3 }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::type_complexity)]
fn pictures(model: &host::Model, o: &Options) -> (Option<(Vec<(&'static str, Vec<u8>)>, Vec<u8>, f64, String)>, Option<String>) {
    match render::pictures(model, o.size, o.no_grid) {
        Ok(p) => (Some((p.views, p.sheet, p.grid, p.adapter)), None),
        Err(e) => (None, Some(e)),
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::type_complexity)]
fn pictures(_: &host::Model, _: &Options) -> (Option<(Vec<(&'static str, Vec<u8>)>, Vec<u8>, f64, String)>, Option<String>) {
    (None, Some(String::from("im Browser zeichnet modeltool nicht")))
}

fn run_main() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    let has = |n: &str| args.iter().any(|a| a == n);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if has("--help") || has("-h") {
        return usage();
    }
    if has("--version") || has("-V") {
        out(&format!("modeltool {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    if has("--api") {
        for e in host::API {
            out(&format!("{}({})\n    {}\n", e.name, e.params, e.doc));
        }
        out("Materialien: ");
        out(&host::PALETTE.iter().map(|p| p.0).collect::<Vec<_>>().join(", "));
        out("\n");
        return 0;
    }
    if has("--checksum") {
        let (sum, n) = checksum::run();
        out(&format!("modeltool checksum={:016x} results={}\n", sum, n));
        return 0;
    }
    let quick = has("--quick");
    if let Some(name) = value("--test") {
        let names: Vec<&str> = name.split(',').filter(|n| !n.is_empty()).collect();
        return selftest::run(&names, quick);
    }
    if has("--check") || args.is_empty() || args.iter().all(|a| a == "--quick" || a == "--script" || a.ends_with(".uis")) {
        return selftest::run(selftest::ALL, quick);
    }
    let size = value("--size").and_then(|v| v.parse::<u32>().ok()).unwrap_or(512).clamp(64, 4096);
    let o = Options { size, no_grid: has("--no-grid"), no_images: has("--no-images"), force: has("--force") };
    if let Some(dir) = value("--examples") {
        let dir = format!("{}/", dir.trim_end_matches(['/', '\\']));
        let mut worst = 0;
        for ex in examples::ALL {
            let mut loader = examples::loader;
            let code = build(&format!("{}.jfs", ex.name), ex.source, &format!("{dir}{}/", ex.name), &[], &o, &mut loader);
            worst = worst.max(code);
        }
        return worst;
    }
    // parameters
    let mut params: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    let mut file = None;
    while i < args.len() {
        let a = &args[i];
        if a == "--param" || a == "-p" {
            let Some(kv) = args.get(i + 1) else {
                err("modeltool: --param braucht NAME=WERT\n");
                return 2;
            };
            let Some((k, v)) = kv.split_once('=') else {
                err(&format!("modeltool: --param {}: NAME=WERT erwartet\n", kv));
                return 2;
            };
            params.retain(|(n, _)| n != k);
            params.push((k.to_string(), v.to_string()));
            i += 2;
            continue;
        }
        if a == "--out" || a == "--size" || a == "--script" {
            i += 2;
            continue;
        }
        if a == "--watch" {
            match args.get(i + 1) {
                Some(f) if file.is_none() => file = Some(f.clone()),
                Some(f) => {
                    err(&format!("modeltool: zwei Skripte ({} und {}); eins je Aufruf\n", file.as_deref().unwrap_or(""), f));
                    return 2;
                }
                None => {
                    err("modeltool: --watch braucht eine Datei\n");
                    return 2;
                }
            }
            i += 2;
            continue;
        }
        if a.starts_with("--") {
            if !matches!(a.as_str(), "--no-grid" | "--no-images" | "--force" | "--report" | "--verify-dirty") {
                err(&format!("modeltool: unbekannte Option {}\n", a));
                return 2;
            }
        } else if file.is_none() {
            file = Some(a.clone());
        } else {
            err(&format!("modeltool: zwei Skripte ({} und {}); eins je Aufruf\n", file.as_deref().unwrap_or(""), a));
            return 2;
        }
        i += 1;
    }
    let Some(file) = file else {
        return usage();
    };
    let file = file.replace('\\', "/");
    if has("--watch") {
        #[cfg(not(target_arch = "wasm32"))]
        return watch_ui::run(&file, &params);
        #[cfg(target_arch = "wasm32")]
        {
            err("modeltool: im Browser gibt es den Beobachtungsmodus nicht\n");
            return 2;
        }
    }
    let Some(bytes) = plat::read_file(&file) else {
        err(&format!("modeltool: kann {} nicht lesen\n", file));
        return 2;
    };
    let Ok(src) = String::from_utf8(bytes) else {
        err(&format!("modeltool: {} ist kein UTF-8\n", file));
        return 2;
    };
    let dir = run::script_dir(&file);
    let mut loader = |p: &str| run::file_loader(&dir, p);
    if has("--report") {
        let r = run::run_script(&file, &src, &params, &mut loader);
        if !r.ok() {
            let sources = sources_of(&r, &src);
            for p in &r.problems {
                err(&run::format_problem(p, &sources));
            }
            out(&json::write(&run::report(&r, None, None, None)));
            return 1;
        }
        let a = run::analyse(&r.model);
        out(&json::write(&run::report(&r, Some(&a), None, None)));
        return if a.ok() { 0 } else { 3 };
    }
    let dir = match value("--out") {
        Some(d) => format!("{}/", d.trim_end_matches(['/', '\\'])),
        None => format!("{}.out/", file.strip_suffix(".jfs").unwrap_or(&file)),
    };
    if !o.force {
        if let Some((ok, findings)) = cached(&dir, &src, &params, &o) {
            out(&format!("{}: {} - unverändert seit dem letzten Lauf (derselbe Schlüssel), nicht neu gebaut\n", file, if ok { "ok" } else { "PRÜFUNG FEHLGESCHLAGEN" }));
            for f in &findings {
                out(&format!("  FEHLER: {}\n", f));
            }
            out(&format!("  Bericht: {}report.json\n", dir));
            return if ok { 0 } else { 3 };
        }
    }
    build(&file, &src, &dir, &params, &o, &mut loader)
}

// a window for --watch: ui's start (its languages, the app's identity) everywhere but in the browser
#[cfg(not(target_arch = "wasm32"))]
ui::main!(run_main());
#[cfg(target_arch = "wasm32")]
rt::main!(run_main());
