// run -- a script from source to a checked model: compile (an unknown name is an error before the run), run in
// a VM with limits, finish the parts (box UV in metres), measure them (geom's checks per part and for the whole
// model, the script's expectations, the budget) and write the report.  Errors carry the file, line and
// column, the source line with a caret and the calls, because an AI reads them and has to find the place;
// the warnings carry the same, and a failed rule of the script (model.expect_size, model.budget) names the line
// that states it.  The warnings are first those of JFScript's language services on the script and each module it
// imported (ls::check with the host's description, host::api(): a member a mesh or a vec does not have, a name
// of a namespace that is not there, a division of two ints), then the host's in the order they came (cx.warn:
// the script went on).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use geom::{Fnv, Mesh, Report, Vec3};
use script::{Api, Limits, Status, Value, Vm, Warning};

use crate::host::{self, Model, ModelHost};
use crate::json::J;

/// the limits of a script: 64 MiB heap, 200 million steps
pub const LIMITS: Limits = Limits { heap: 64 << 20, stack: 100_000, depth: 500, fuel: 200_000_000, output: 256 << 10 };

/// an error of the compiler or the run
#[derive(Clone, Debug)]
pub struct Problem {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
    /// "in name (file:line:col)", innermost first
    pub trace: Vec<String>,
}

/// what a run gives
pub struct Run {
    pub file: String,
    pub model: Model,
    pub problems: Vec<Problem>,
    /// the language services' warnings on the script and its modules, then the host's (cx.warn) in the order they
    /// came; at most script::MAX_WARNINGS
    pub warnings: Vec<Warning>,
    /// how many more there were (not kept)
    pub warnings_dropped: u64,
    /// what the script printed
    pub output: String,
    /// (path, source) of the modules it imported
    pub imports: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    /// the cache key: SHA-256 (hex, 32 digits) of the tool's version, the script, the parameters and the modules
    pub key: String,
    pub fuel: u64,
    pub script_us: u64,
    pub host_us: u64,
    /// how many meshes the run made, and the most the script held at once (the others were freed)
    pub meshes: u64,
    pub meshes_peak: usize,
}

impl Run {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// a parameter's text as a script value: int, float, true / false, else str
fn param_value(vm: &mut Vm, s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::Int(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        if f.is_finite() {
            return Value::Float(f);
        }
    }
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => vm.new_str(s).unwrap_or(Value::Nil),
    }
}

/// the text with CR LF line ends as LF
pub fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// the cache key of a script, its parameters and its modules
pub fn cache_key(src: &str, params: &[(String, String)], imports: &[(String, String)]) -> String {
    let mut text = String::new();
    // (2: the report has warnings and the rules' places since JFScript S1; 3: meshes and vectors are host objects
    // since S2, and the language services' warnings are in the report; a report of before is built again)
    text.push_str("modeltool-key-3\n");
    text.push_str(crate::APP_VERSION);
    text.push('\n');
    text.push_str(rt::VERSION);
    // line ends as LF: a checkout with CRLF gives the same key
    text.push_str("\n--script\n");
    text.push_str(&lf(src));
    let mut p: Vec<&(String, String)> = params.iter().collect();
    p.sort();
    for (k, v) in p {
        text.push_str(&format!("\n--param {}={}", k, v));
    }
    for (path, s) in imports {
        text.push_str(&format!("\n--import {}\n", path));
        text.push_str(&lf(s));
    }
    let h = crypto::sha2::sha256(text.as_bytes());
    h[..16].iter().map(|b| format!("{:02x}", b)).collect()
}

/// `p` without "." and "..", "/" between the parts, the root ("/", "C:/") kept: the way JFScript normalises the
/// path of an import (its module.rs), so the two compare
pub fn normalize(p: &str) -> String {
    let p = p.replace('\\', "/");
    let b = p.as_bytes();
    let (root, rest) = if p.starts_with('/') {
        ("/", &p[1..])
    } else if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        if b.get(2) == Some(&b'/') { (&p[..3], &p[3..]) } else { (&p[..2], &p[2..]) }
    } else {
        ("", &p[..])
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&l) if l != ".." => {
                    parts.pop();
                }
                _ if !root.is_empty() => {}
                _ => parts.push(".."),
            },
            c => parts.push(c),
        }
    }
    format!("{}{}", root, parts.join("/"))
}

/// the folder of a script, normalised ("" for a script in the working directory)
pub fn script_dir(file: &str) -> String {
    let f = normalize(file);
    match f.rfind('/') {
        Some(i) => String::from(&f[..i + 1]),
        None => String::new(),
    }
}

/// Whether a script in folder `dir` (script_dir) may import `path` (normalised by JFScript, relative to the
/// working directory): only files in its folder and below.  Err with the reason for the message.
pub fn import_allowed(dir: &str, path: &str) -> Result<(), String> {
    let p = normalize(path);
    let inside = if dir.is_empty() {
        let b = p.as_bytes();
        !(p.starts_with('/') || (b.len() >= 2 && b[1] == b':') || p == ".." || p.starts_with("../"))
    } else {
        p.len() > dir.len() && p.starts_with(dir)
    };
    if inside && !p.split('/').any(|c| c == "..") {
        Ok(())
    } else {
        Err(format!(
            "liegt außerhalb des Ordners des Skripts ({}); import liest nur Dateien in diesem Ordner und darunter",
            if dir.is_empty() { "." } else { dir.trim_end_matches('/') }
        ))
    }
}

/// A module in the script's folder or below (paths come normalised and relative to the working directory;
/// import_allowed has refused any outside the folder): no link on the way from the folder to the file, so a link
/// inside it cannot lead out.
pub fn file_loader(dir: &str, path: &str) -> Result<String, String> {
    // (the browser has no file system and no links)
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rest = path.strip_prefix(dir).unwrap_or(path);
        let mut at = String::from(dir);
        for part in rest.split('/') {
            at.push_str(part);
            if plat::fs::lstat(&at).is_some_and(|m| m.is_symlink()) {
                return Err(format!("{} ist eine Verknüpfung; import folgt keinen Verknüpfungen (sie könnten aus dem Ordner des Skripts führen)", at));
            }
            at.push('/');
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = dir;
    match plat::read_file(path) {
        Some(b) => String::from_utf8(b).map_err(|_| String::from("die Datei ist kein UTF-8")),
        None => Err(String::from("die Datei gibt es nicht")),
    }
}

/// The warnings of JFScript's language services on a script or module (ls::check with the host's description):
/// a member a mesh or a vec does not have, a name of a namespace that is not there (with the closest one), a
/// division of two ints, a top-level let read before it ran.  Its errors are compile_with's and stop the run
/// before.
pub fn ls_warnings(api: &Api, file: &str, src: &str) -> Vec<Warning> {
    script::ls::check(src, api)
        .into_iter()
        .filter(|d| !d.error)
        .map(|d| Warning { file: String::from(file), line: d.line, col: d.col, msg: d.msg })
        .collect()
}

/// Compile and run the script `src` (named `file` in messages) with its parameters; `loader` reads a module
/// for import(path) (the path relative to the script).  A module outside the script's folder is refused
/// before the loader sees it (import_allowed).
pub fn run_script(file: &str, src: &str, params: &[(String, String)], loader: &mut dyn FnMut(&str) -> Result<String, String>) -> Run {
    let t0 = plat::time_us();
    let mut run = Run {
        file: String::from(file),
        model: Model::default(),
        problems: Vec::new(),
        warnings: Vec::new(),
        warnings_dropped: 0,
        output: String::new(),
        imports: Vec::new(),
        params: params.to_vec(),
        key: String::new(),
        fuel: 0,
        script_us: 0,
        host_us: 0,
        meshes: 0,
        meshes_peak: 0,
    };
    let api = host::api();
    let prog = match script::compile_with(file, src, &api.names()) {
        Ok(p) => p,
        Err(diags) => {
            for d in diags {
                let f = if d.file.is_empty() { String::from(file) } else { d.file };
                run.problems.push(Problem { file: f, line: d.line, col: d.col, message: d.msg, trace: Vec::new() });
            }
            run.key = cache_key(src, params, &[]);
            return run;
        }
    };
    let mut vm = Vm::new(&prog, LIMITS);
    if let Ok(m) = vm.new_map() {
        for (k, v) in params {
            let val = param_value(&mut vm, v);
            if let Ok(key) = vm.new_str(k) {
                let _ = vm.map_set(m, key, val);
            }
        }
        vm.set_global("params", m);
    }
    let dir = script_dir(file);
    let mut guarded = |path: &str| -> Result<String, String> {
        import_allowed(&dir, path)?;
        loader(path)
    };
    let mut host = ModelHost::new(&api, &mut guarded);
    loop {
        match vm.run(&mut host, 10_000_000) {
            Status::Done(_) => break,
            Status::Yielded | Status::Paused(_) => continue,
            Status::Error(e) => {
                let f = if e.file.is_empty() { String::from(file) } else { e.file.clone() };
                let trace = e
                    .trace
                    .iter()
                    .map(|t| {
                        let tf = if t.file.is_empty() { file } else { t.file.as_str() };
                        let at = format!("{}:{}:{}", tf, t.line, t.col);
                        if t.func.is_empty() { format!("im Hauptprogramm ({})", at) } else { format!("in {} ({})", t.func, at) }
                    })
                    .collect();
                run.problems.push(Problem { file: f, line: e.line, col: e.col, message: e.msg, trace });
                break;
            }
        }
    }
    run.fuel = vm.call_fuel();
    run.output = vm.take_output();
    run.host_us = host.host_us;
    run.imports = core::mem::take(&mut host.imports);
    run.model = core::mem::take(&mut host.model);
    (run.meshes, run.meshes_peak) = (host.made, host.peak);
    drop(host);
    // the language services' warnings on the script and its modules, then the host's up to the end or the error,
    // at most script::MAX_WARNINGS together; the places of the rules; each with the file named as errors name it
    let mut warnings = ls_warnings(&api, file, src);
    for (path, s) in &run.imports {
        warnings.extend(ls_warnings(&api, path, s));
    }
    warnings.extend(vm.take_warnings());
    run.warnings_dropped = vm.warnings_dropped();
    if warnings.len() > script::MAX_WARNINGS {
        run.warnings_dropped += (warnings.len() - script::MAX_WARNINGS) as u64;
        warnings.truncate(script::MAX_WARNINGS);
    }
    run.warnings = warnings;
    for w in run.warnings.iter_mut().filter(|w| w.file.is_empty()) {
        w.file = String::from(file);
    }
    let rules = run.model.expects.iter_mut().map(|e| &mut e.at).chain(core::iter::once(&mut run.model.budget_at));
    for at in rules.filter(|at| at.file.is_empty() && at.line > 0) {
        at.file = String::from(file);
    }
    if run.problems.is_empty() && run.model.parts.is_empty() {
        run.problems.push(Problem {
            file: String::from(file),
            line: 0,
            col: 0,
            message: String::from("Das Skript hat dem Modell nichts hinzugefügt: model.add(mesh, material) fehlt"),
            trace: Vec::new(),
        });
    }
    // the parts' UV in metres where they ended up
    let s = run.model.uv_scale;
    for p in run.model.parts.iter_mut() {
        p.mesh = p.mesh.uv_box(s);
    }
    run.key = cache_key(src, params, &run.imports);
    run.script_us = plat::time_us().saturating_sub(t0).saturating_sub(run.host_us);
    run
}

/// "file:line:col: KIND: message" (without a line: "file: KIND: message"), then the source line with a caret
/// under the column when `sources` has the file
fn format_place(kind: &str, file: &str, line: u32, col: u32, message: &str, sources: &[(String, String)]) -> String {
    let mut s = if line > 0 { format!("{}:{}:{}: {}: {}\n", file, line, col, kind, message) } else { format!("{}: {}: {}\n", file, kind, message) };
    if line > 0 {
        if let Some((_, src)) = sources.iter().find(|(f, _)| *f == file) {
            if let Some(text) = src.lines().nth(line as usize - 1) {
                let text = text.replace('\t', " ");
                let no = format!("{}", line);
                s.push_str(&format!("  {} | {}\n", no, text.trim_end()));
                let caret: String = core::iter::repeat_n(' ', (col as usize).saturating_sub(1)).collect();
                s.push_str(&format!("  {} | {}^\n", " ".repeat(no.len()), caret));
            }
        }
    }
    s
}

/// a problem as the console shows it: "file:line:col: Fehler: message", the source line with a caret, the calls
pub fn format_problem(p: &Problem, sources: &[(String, String)]) -> String {
    let mut s = format_place("Fehler", &p.file, p.line, p.col, &p.message, sources);
    for t in &p.trace {
        s.push_str(&format!("  {}\n", t));
    }
    s
}

/// how many warnings the console and the window show in full (the report has all)
pub const WARNINGS_SHOWN: usize = 10;

/// The warnings as the console shows them, each like an error ("file:line:col: Warnung: message", the source
/// line with a caret), at most WARNINGS_SHOWN; then one line with how many more there are.
pub fn format_warnings(run: &Run, sources: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = run.warnings.iter().take(WARNINGS_SHOWN).map(|w| format_place("Warnung", &w.file, w.line, w.col, &w.msg, sources)).collect();
    let more = (run.warnings.len() - out.len()) as u64;
    let lost = run.warnings_dropped;
    if lost > 0 {
        out.push(format!("  ... und {} weitere Warnungen ({} im Bericht; {} nicht aufgehoben, höchstens {} je Lauf)\n", more + lost, more, lost, script::MAX_WARNINGS));
    } else if more > 0 {
        out.push(format!("  ... und {} weitere Warnungen (alle im Bericht)\n", more));
    }
    out
}

/// the measurements of a model
pub struct Analysis {
    /// the whole model (every part merged)
    pub whole: Report,
    pub parts: Vec<Report>,
    /// per expectation: met, the actual size
    pub expects: Vec<(bool, Vec3)>,
    pub within_budget: Option<bool>,
    /// FNV-1a over every part's mesh bits and material
    pub fingerprint: u64,
    pub check_us: u64,
}

impl Analysis {
    /// every check passed, every expectation met, within the budget
    pub fn ok(&self) -> bool {
        self.whole.ok() && self.parts.iter().all(|p| p.ok()) && self.expects.iter().all(|e| e.0) && self.within_budget != Some(false)
    }
}

/// measure a model
pub fn analyse(model: &Model) -> Analysis {
    let t0 = plat::time_us();
    let parts: Vec<Report> = model.parts.iter().map(|p| p.mesh.check()).collect();
    let refs: Vec<&Mesh> = model.parts.iter().map(|p| &p.mesh).collect();
    let whole = Mesh::merge(&refs).unwrap_or_default().check();
    let size = whole.size();
    let expects = model
        .expects
        .iter()
        .map(|e| {
            let d = [size.x - e.size[0], size.y - e.size[1], size.z - e.size[2]];
            (d.iter().zip(&e.size).all(|(x, want)| want.is_nan() || geom::math::abs(*x) <= e.tolerance), size)
        })
        .collect();
    let mut h = Fnv::new();
    for p in &model.parts {
        h.u(p.mesh.fingerprint());
        h.u(p.material as u64);
    }
    Analysis {
        whole,
        parts,
        expects,
        within_budget: model.budget.map(|b| (model.parts.iter().map(|p| p.mesh.triangle_count() as u64).sum::<u64>()) <= b),
        fingerprint: h.0,
        check_us: plat::time_us().saturating_sub(t0),
    }
}

/// a German one-line summary of what failed, for the console and the report; the first finding about the surface
/// says where it lies first and in which part
pub fn findings(model: &Model, a: &Analysis) -> Vec<String> {
    let mut out = Vec::new();
    let w = &a.whole;
    let place = a.parts.iter().enumerate().find_map(|(i, r)| r.problem_at.map(|p| (i, p))).map(|(i, p)| {
        format!(", zuerst bei ({}, {}, {}) in Teil \"{}\"", geom::fnum(p.x), geom::fnum(p.y), geom::fnum(p.z), model.parts[i].name)
    });
    if w.open_edges > 0 {
        out.push(format!("nicht geschlossen: {} offene Kanten", w.open_edges));
    }
    if w.nonmanifold_edges > 0 {
        out.push(format!("{} Kanten mit mehr als zwei Dreiecken", w.nonmanifold_edges));
    }
    if w.flipped_edges > 0 {
        out.push(format!("Umlaufsinn uneinheitlich: {} Kanten mit zwei gleich laufenden Dreiecken", w.flipped_edges));
    }
    if w.nonmanifold_points > 0 {
        out.push(format!("{} Punkte, an denen sich Teile nur berühren", w.nonmanifold_points));
    }
    if w.inverted_components > 0 {
        out.push(format!("{} geschlossene Teile stehen innen nach außen (Normalen nach innen)", w.inverted_components));
    }
    if w.degenerate > 0 {
        out.push(format!("{} entartete Dreiecke (ohne Fläche)", w.degenerate));
    }
    if w.bad_normals > 0 {
        out.push(format!("{} Normalen falsch (nicht Länge 1 oder gegen die Fläche)", w.bad_normals));
    }
    if let (Some(first), Some(p)) = (out.first_mut(), place) {
        first.push_str(&p);
    }
    for (i, p) in a.parts.iter().enumerate() {
        if !p.ok() && w.ok() {
            out.push(format!("Teil {} ({}) besteht die Prüfungen nicht", i + 1, model.parts[i].name));
        }
    }
    for (e, (ok, size)) in model.expects.iter().zip(&a.expects) {
        if !ok {
            let want = |v: f64| if v.is_nan() { String::from("beliebig") } else { geom::fnum(v) };
            out.push(format!(
                "{}Maße {} x {} x {} m erwartet (Toleranz {}), sind {} x {} x {} m",
                rule_at(&e.at), want(e.size[0]), want(e.size[1]), want(e.size[2]), geom::fnum(e.tolerance),
                geom::fnum(size.x), geom::fnum(size.y), geom::fnum(size.z)
            ));
        }
    }
    if a.within_budget == Some(false) {
        out.push(format!("{}über dem Budget: {} Dreiecke, erlaubt {}", rule_at(&model.budget_at), w.triangles, model.budget.unwrap_or(0)));
    }
    out
}

/// "haus.jfs:71: " before a finding about a rule of the script (nothing when its place is not known)
fn rule_at(at: &script::Position) -> String {
    if at.line > 0 { format!("{}: ", host::place(at)) } else { String::new() }
}

/// "#rrggbb" of an sRGB colour
pub fn hex(c: [f64; 3]) -> String {
    let b = |v: f64| (geom::math::floor(v.clamp(0.0, 1.0) * 255.0 + 0.5)) as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

/// a geom report as JSON
pub fn report_json(r: &Report) -> J {
    let b = r.bounds.unwrap_or(geom::Aabb::point(Vec3::ZERO));
    J::obj(alloc::vec![
        ("ok", J::Bool(r.ok())),
        ("closed", J::Bool(r.closed())),
        ("outward", J::Bool(r.outward())),
        ("vertices", J::Int(r.vertices as i64)),
        ("triangles", J::Int(r.triangles as i64)),
        ("min", J::vec3(b.min)),
        ("max", J::vec3(b.max)),
        ("size", J::vec3(r.size())),
        ("area", J::Num(r.area)),
        ("volume", J::Num(r.volume)),
        ("components", J::Int(r.components as i64)),
        ("euler", J::Int(r.euler)),
        ("open_edges", J::Int(r.open_edges as i64)),
        ("nonmanifold_edges", J::Int(r.nonmanifold_edges as i64)),
        ("flipped_edges", J::Int(r.flipped_edges as i64)),
        ("nonmanifold_points", J::Int(r.nonmanifold_points as i64)),
        ("inverted_components", J::Int(r.inverted_components as i64)),
        ("degenerate", J::Int(r.degenerate as i64)),
        ("bad_normals", J::Int(r.bad_normals as i64)),
    ])
}

/// what the report adds when images and files were written
pub struct Outputs {
    pub files: J,
    pub views: J,
}

/// The report: the part every run of the same script gives (tool, key, script, ok, errors, warnings, findings,
/// params, model, checks, parts, the rules with their places and results, materials, output, imports,
/// fingerprint) and, when given, the files, the views and the timing, each on one line at the end.
pub fn report(run: &Run, a: Option<&Analysis>, outputs: Option<&Outputs>, timing: Option<J>) -> J {
    let mut f: Vec<(&str, J)> = Vec::new();
    f.push(("tool", J::Str(format!("modeltool {}", crate::APP_VERSION))));
    f.push(("script", J::str(&run.file)));
    f.push(("key", J::str(&run.key)));
    let ok = run.ok() && a.is_some_and(|a| a.ok());
    f.push(("ok", J::Bool(ok)));
    let errors: Vec<J> = run
        .problems
        .iter()
        .map(|p| {
            J::obj(alloc::vec![
                ("file", J::str(&p.file)),
                ("line", J::Int(p.line as i64)),
                ("col", J::Int(p.col as i64)),
                ("message", J::str(&p.message)),
                ("trace", J::Arr(p.trace.iter().map(|t| J::str(t)).collect())),
            ])
            .compact()
        })
        .collect();
    f.push(("errors", J::Arr(errors)));
    let warnings: Vec<J> = run
        .warnings
        .iter()
        .map(|w| {
            J::obj(alloc::vec![("file", J::str(&w.file)), ("line", J::Int(w.line as i64)), ("col", J::Int(w.col as i64)), ("message", J::str(&w.msg))]).compact()
        })
        .collect();
    f.push(("warnings", J::Arr(warnings)));
    if run.warnings_dropped > 0 {
        f.push(("warnings_dropped", J::Int(run.warnings_dropped as i64)));
    }
    let fnd = a.map(|a| findings(&run.model, a)).unwrap_or_default();
    f.push(("findings", J::Arr(fnd.iter().map(|s| J::str(s)).collect())));
    f.push(("params", J::Obj(run.params.iter().map(|(k, v)| (k.clone(), J::str(v))).collect())));
    if let Some(a) = a {
        let m = &run.model;
        let w = &a.whole;
        let b = w.bounds.unwrap_or(geom::Aabb::point(Vec3::ZERO));
        f.push((
            "model",
            J::obj(alloc::vec![
                ("name", J::str(&m.name)),
                ("parts", J::Int(m.parts.len() as i64)),
                ("materials", J::Int(m.materials.len() as i64)),
                ("vertices", J::Int(w.vertices as i64)),
                ("triangles", J::Int(w.triangles as i64)),
                ("budget", m.budget.map_or(J::Null, |b| J::Int(b as i64))),
                ("within_budget", a.within_budget.map_or(J::Null, J::Bool)),
                ("min", J::vec3(b.min)),
                ("max", J::vec3(b.max)),
                ("size", J::vec3(w.size())),
                ("center", J::vec3(b.center())),
                ("area", J::Num(w.area)),
                ("volume", J::Num(w.volume)),
                ("uv_scale", J::Num(m.uv_scale)),
            ]),
        ));
        f.push(("checks", report_json(w)));
        let parts: Vec<J> = m
            .parts
            .iter()
            .zip(&a.parts)
            .map(|(p, r)| {
                let b = r.bounds.unwrap_or(geom::Aabb::point(Vec3::ZERO));
                J::obj(alloc::vec![
                    ("name", J::str(&p.name)),
                    ("material", J::str(&m.materials[p.material].name)),
                    ("ok", J::Bool(r.ok())),
                    ("closed", J::Bool(r.closed())),
                    ("outward", J::Bool(r.outward())),
                    ("triangles", J::Int(r.triangles as i64)),
                    ("vertices", J::Int(r.vertices as i64)),
                    ("min", J::vec3(b.min)),
                    ("size", J::vec3(r.size())),
                    ("area", J::Num(r.area)),
                    ("volume", J::Num(r.volume)),
                    ("degenerate", J::Int(r.degenerate as i64)),
                ])
                .compact()
            })
            .collect();
        f.push(("parts", J::Arr(parts)));
        // the rules of the script, each with where it states it: the sizes, then the budget
        let mut expects: Vec<J> = m
            .expects
            .iter()
            .zip(&a.expects)
            .map(|(e, (ok, size))| {
                J::obj(alloc::vec![
                    ("rule", J::str("size")),
                    ("file", J::str(&e.at.file)),
                    ("line", J::Int(e.at.line as i64)),
                    ("expected", J::nums(&e.size)),
                    ("actual", J::vec3(*size)),
                    ("tolerance", J::Num(e.tolerance)),
                    ("ok", J::Bool(*ok)),
                ])
                .compact()
            })
            .collect();
        if let (Some(b), Some(ok)) = (m.budget, a.within_budget) {
            expects.push(
                J::obj(alloc::vec![
                    ("rule", J::str("budget")),
                    ("file", J::str(&m.budget_at.file)),
                    ("line", J::Int(m.budget_at.line as i64)),
                    ("expected", J::Int(b as i64)),
                    ("actual", J::Int(w.triangles as i64)),
                    ("ok", J::Bool(ok)),
                ])
                .compact(),
            );
        }
        f.push(("expect", J::Arr(expects)));
        let mats: Vec<J> = m
            .materials
            .iter()
            .map(|x| {
                J::obj(alloc::vec![
                    ("name", J::str(&x.name)),
                    ("color", J::Str(hex(x.color))),
                    ("roughness", J::Num(x.roughness)),
                    ("metallic", J::Num(x.metallic)),
                ])
                .compact()
            })
            .collect();
        f.push(("materials", J::Arr(mats)));
        f.push(("fingerprint", J::Str(format!("{:016x}", a.fingerprint))));
    }
    f.push(("output", J::str(&run.output)));
    let imports: Vec<J> = run
        .imports
        .iter()
        .map(|(p, s)| {
            let h = crypto::sha2::sha256(lf(s).as_bytes());
            J::obj(alloc::vec![("path", J::str(p)), ("sha256", J::Str(h.iter().map(|b| format!("{:02x}", b)).collect()))]).compact()
        })
        .collect();
    f.push(("imports", J::Arr(imports)));
    if let Some(o) = outputs {
        f.push(("files", o.files.clone().compact()));
        f.push(("views", o.views.clone().compact()));
    }
    if let Some(t) = timing {
        f.push(("timing", t.compact()));
    }
    J::obj(f)
}

/// the report's lines without those that change between runs and options ("files", "views", "timing"),
/// trailing commas dropped: what selftest compares
pub fn stable_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !(l.starts_with("  \"files\"") || l.starts_with("  \"views\"") || l.starts_with("  \"timing\"")))
        .map(|l| l.trim_end().trim_end_matches(',').to_string())
        .collect()
}
