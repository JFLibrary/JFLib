// backend -- the checks of lib/wgsl's back end (wgsl::compile, docs/WGSL.md "Back-End")
//
//   spirv    every valid module -- the benchmark shaders, the valid cases of the check group
//            (modules without entry point through compile's library mode), the files of
//            tools/wgsl_corpus Tint accepts -- compiled at both levels: lib/shader's validator
//            accepts every module and the same input gives the same words; with the Vulkan
//            SDK (as shadertest finds it) spirv-val --target-env vulkan1.1 (Basis) / vulkan1.3
//            (Voll) accepts every module too
//   corpus   lib/wgsl's verdict on every corpus file against Tint's (expected.json, no Chrome):
//            check, compile, then per entry point the pipeline checks (Output::check_pipeline
//            with WebGPU's default limits and the file's `// constants:` line, check_interface
//            between the first vertex and each fragment entry point) -- valid, error or
//            pipeline-error; a file we accept and Tint rejects fails unless ALLOW_ACCEPT lists
//            it, a file we reject and Tint accepts fails unless KNOWN_REJECT lists it; on the
//            unif and diag files our warnings and infos must match Tint's (type and line); the
//            position of the first error is compared with Tint's on every error file
//   --spv FILE [--out DIR]   compile one file at both levels, write DIR/<file>.<entry>.1x.spv,
//            print lib/shader's verdict (a tool for the development)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{out, T};

const OUT: &str = "build/wgsltest";
const CORPUS: &str = "tools/wgsl_corpus";

/// corpus files we accept and Tint rejects: none (never accept what Chrome rejects)
const ALLOW_ACCEPT: &[&str] = &[];

/// corpus files we reject and Tint accepts (docs/WGSL.md "Abweichungen"): the spec's rule where
/// Chrome is more lenient
const KNOWN_REJECT: &[&str] = &[
    // `01f`: the spec's decimal float grammar has no leading zero before a suffix; Tint takes it
    "lex/err_float_leading_zero_suffix.wgsl",
    // over the minimum of spec 2.4, which Chrome exceeds (see check.rs, MAX_*)
    "limits/err_array_constructor_2048.wgsl",
    "limits/err_array_constructor_typed_2048.wgsl",
    "limits/err_case_selectors_1024.wgsl",
    "limits/err_case_selectors_in_one_clause_1024.wgsl",
    "limits/err_function_8196.wgsl",
    "limits/err_function_two_blocks.wgsl",
    "limits/err_nesting_array_16.wgsl",
    "limits/err_nesting_matrix_counts_two.wgsl",
    "limits/err_nesting_struct_16.wgsl",
    "limits/err_private_8196.wgsl",
    "limits/err_private_through_helper.wgsl",
    "limits/err_private_two_variables.wgsl",
    "limits/err_struct_members_1024.wgsl",
];

/// lib/shader's validator with the options the module needs
pub fn shader_val(words: &[u32], needs: &wgsl::spirv::Needs) -> Result<(), String> {
    let opt = shader::val::Options { uniform_standard_layout: needs.uniform_standard_layout };
    shader::val::validate_with(words, &opt)
        .map_err(|e| e.iter().take(4).map(|x| format!("{:?} @{}: {}", x.rule, x.at, x.msg)).collect::<Vec<_>>().join("; "))
}

/// --spv FILE: compile one file at both levels and write the modules
pub fn spv_file(path: &str) -> i32 {
    let Some(bytes) = plat::read_file(path) else {
        out(&format!("cannot read {}\n", path));
        return 2;
    };
    let src = String::from_utf8_lossy(&bytes);
    let dir = plat::arg_value("--out").unwrap_or_else(|| String::from(OUT));
    plat::mkdir(&dir);
    let stem = path.rsplit(['/', '\\']).next().unwrap_or(path).trim_end_matches(".wgsl");
    let mut code = 0;
    for minor in [3u32, 6] {
        let opt = wgsl::CompileOptions::new(minor);
        match wgsl::compile(&src, &opt).or_else(|_| wgsl::spirv::compile_library(&src, &opt)) {
            Ok(o) => {
                for e in o.entries {
                    let file = plat::fs::join(&dir, &format!("{}.{}.1{}.spv", stem, e.name, minor));
                    plat::write_file(&file, &shader::to_bytes(&e.words));
                    match shader_val(&e.words, &e.needs) {
                        Ok(()) => out(&format!("{}: ok ({} words)\n", file, e.words.len())),
                        Err(m) => {
                            out(&format!("{}: shader::val: {}\n", file, m));
                            code = 1;
                        }
                    }
                }
            }
            Err(d) => {
                for x in d {
                    out(&format!("{} (1.{}): {}\n", path, minor, x));
                }
                code = 1;
            }
        }
    }
    code
}

// ---------------------------------------------------------------- the Vulkan SDK

fn exe(dir: &str, name: &str) -> String {
    plat::fs::join(dir, &if cfg!(windows) { format!("{}.exe", name) } else { String::from(name) })
}

fn version_key(name: &str) -> Vec<u32> {
    name.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// spirv-val: --sdk DIR, %VULKAN_SDK%\Bin, the newest C:\VulkanSDK\*\Bin, PATH
fn find_val() -> Option<String> {
    let bin = if cfg!(windows) { "Bin" } else { "bin" };
    let mut tries: Vec<String> = Vec::new();
    if let Some(d) = plat::arg_value("--sdk") {
        tries.push(d);
    }
    if let Some(d) = plat::env("VULKAN_SDK") {
        tries.push(plat::fs::join(&d, bin));
    }
    if cfg!(windows) {
        if let Some(list) = plat::fs::read_dir("C:\\VulkanSDK") {
            let mut names: Vec<String> = list.into_iter().map(|e| e.name).collect();
            names.sort_by_key(|n| version_key(n));
            if let Some(n) = names.last() {
                tries.push(format!("C:\\VulkanSDK\\{}\\{}", n, bin));
            }
        }
    }
    if let Some(p) = plat::process::find("spirv-val") {
        let cut = p.rfind(['/', '\\']).unwrap_or(0);
        tries.push(String::from(&p[..cut]));
    }
    tries.into_iter().map(|d| exe(&d, "spirv-val")).find(|p| plat::file_exists(p) == 1)
}

/// run a tool -> (exit code, stdout + stderr)
fn tool(program: &str, args: &[&str]) -> Result<(i32, String), String> {
    let mut c = plat::process::Command { program, args, cwd: None, env: &[] }.spawn()
        .map_err(|e| format!("{} does not start (error {})", program, e))?;
    let mut text = Vec::new();
    let start = plat::time_us();
    let mut buf = [0u8; 8192];
    loop {
        let mut idle = true;
        while let Ok(Some(n)) = c.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            text.extend_from_slice(&buf[..n]);
            idle = false;
        }
        while let Ok(Some(n)) = c.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            text.extend_from_slice(&buf[..n]);
            idle = false;
        }
        if let Ok(Some(code)) = c.try_wait() {
            while let Ok(Some(n)) = c.read_stdout(&mut buf) {
                if n == 0 {
                    break;
                }
                text.extend_from_slice(&buf[..n]);
            }
            while let Ok(Some(n)) = c.read_stderr(&mut buf) {
                if n == 0 {
                    break;
                }
                text.extend_from_slice(&buf[..n]);
            }
            return Ok((code, String::from_utf8_lossy(&text).replace('\r', "")));
        }
        if plat::time_us() - start > 60_000_000 {
            c.terminate();
            return Err(format!("{} takes too long", program));
        }
        if idle {
            plat::sleep_ms(1);
        }
    }
}

// ---------------------------------------------------------------- sources

/// every .wgsl file of the corpus (path relative to it, text)
pub fn corpus_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(dirs) = plat::fs::read_dir(CORPUS) else { return out };
    let mut dirs: Vec<String> = dirs.into_iter().filter(|e| e.kind == plat::fs::FileKind::Dir).map(|e| e.name).collect();
    dirs.sort();
    for d in dirs {
        let Some(files) = plat::fs::read_dir(&plat::fs::join(CORPUS, &d)) else { continue };
        let mut names: Vec<String> = files.into_iter().map(|e| e.name).filter(|n| n.ends_with(".wgsl")).collect();
        names.sort();
        for n in names {
            if let Some(b) = plat::read_file(&plat::fs::join(&plat::fs::join(CORPUS, &d), &n)) {
                out.push((format!("{}/{}", d, n), String::from_utf8_lossy(&b).into_owned()));
            }
        }
    }
    out
}

/// Tint's verdict on every corpus file from expected.json: (path, verdict)
pub fn expected() -> Vec<(String, String)> {
    let Some(b) = plat::read_file(&plat::fs::join(CORPUS, "expected.json")) else { return Vec::new() };
    let s = String::from_utf8_lossy(&b).into_owned();
    let mut out = Vec::new();
    let Some(files) = s.find("\"files\"") else { return out };
    let mut at = files;
    // each entry: "path.wgsl": { "rule": ..., "verdict": "..."
    while let Some(k) = s[at..].find(".wgsl\": {") {
        let end = at + k + 5;
        let start = s[..end].rfind('"').map(|i| i + 1).unwrap_or(end);
        let path = String::from(&s[start..end]);
        let Some(v) = s[end..].find("\"verdict\": \"") else { break };
        let vs = end + v + 12;
        let ve = s[vs..].find('"').map(|i| vs + i).unwrap_or(vs);
        out.push((path, String::from(&s[vs..ve])));
        at = ve;
    }
    out
}

/// a message of Tint's from expected.json
#[derive(Clone, Default, Debug)]
pub struct TintMessage {
    pub kind: String,
    pub line: u32,
    pub col: u32,
    pub text: String,
}

impl TintMessage {
    /// a message of its own, not a note of the one before (notes also have the type info)
    pub fn is_main(&self) -> bool {
        self.kind != "info" || self.text.contains("uniform control flow") && self.text.contains("must only be called") || self.text.contains("to be uniform") || self.text.contains("code is unreachable")
    }
}

/// the JSON string at s[at..] (at the opening quote) and the place after it
fn json_string(s: &str, at: usize) -> (String, usize) {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = at + 1;
    while i < b.len() && b[i] != b'"' {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'u' => {
                    // (enough for the comparisons: the text is not shown)
                    out.push(b'?');
                    i += 4;
                }
                c => out.push(c),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    (String::from_utf8_lossy(&out).into_owned(), i + 1)
}

/// Tint's messages per corpus file from expected.json
pub fn expected_messages() -> Vec<(String, Vec<TintMessage>)> {
    let Some(b) = plat::read_file(&plat::fs::join(CORPUS, "expected.json")) else { return Vec::new() };
    let s = String::from_utf8_lossy(&b).into_owned();
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let Some(files) = s.find("\"files\"") else { return out };
    let mut at = files;
    while let Some(k) = s[at..].find(".wgsl\": {") {
        let end = at + k + 5;
        let start = s[..end].rfind('"').map(|i| i + 1).unwrap_or(end);
        let path = String::from(&s[start..end]);
        let next = s[end + 1..].find(".wgsl\": {").map(|n| end + 1 + n).unwrap_or(s.len());
        let mut msgs = Vec::new();
        if let Some(m) = s[end..next].find("\"messages\": [") {
            let mut i = end + m + 13;
            loop {
                while i < next && matches!(bytes[i], b' ' | b'\n' | b'\r' | b',') {
                    i += 1;
                }
                if i >= next || bytes[i] != b'{' {
                    break;
                }
                i += 1;
                let mut one = TintMessage::default();
                loop {
                    while i < next && matches!(bytes[i], b' ' | b'\n' | b'\r' | b',') {
                        i += 1;
                    }
                    if i >= next || bytes[i] == b'}' {
                        i += 1;
                        break;
                    }
                    let (key, n) = json_string(&s, i);
                    i = n + 1;
                    while i < next && bytes[i] == b' ' {
                        i += 1;
                    }
                    if bytes[i] == b'"' {
                        let (v, n) = json_string(&s, i);
                        i = n;
                        match key.as_str() {
                            "type" => one.kind = v,
                            "text" => one.text = v,
                            _ => {}
                        }
                    } else {
                        let st = i;
                        while i < next && !matches!(bytes[i], b',' | b'}' | b'\n' | b'\r') {
                            i += 1;
                        }
                        let v: u32 = s[st..i].trim().parse().unwrap_or(0);
                        match key.as_str() {
                            "lineNum" => one.line = v,
                            "linePos" => one.col = v,
                            _ => {}
                        }
                    }
                }
                msgs.push(one);
            }
        }
        out.push((path, msgs));
        at = end;
    }
    out
}

/// the `// constants: {"name": value, ...}` line of a corpus file
pub fn constants_line(src: &str) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for l in src.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("// constants:") else { continue };
        let body = rest.trim().trim_start_matches('{').trim_end_matches('}');
        for part in body.split(',') {
            let mut kv = part.splitn(2, ':');
            let (Some(k), Some(v)) = (kv.next(), kv.next()) else { continue };
            let k = k.trim().trim_matches('"');
            let v = v.trim();
            let x = match v {
                "true" => 1.0,
                "false" => 0.0,
                _ => v.parse::<f64>().unwrap_or(0.0),
            };
            out.push((String::from(k), x));
        }
    }
    out
}

/// our verdict on a corpus file: "valid", "error" or "pipeline-error" and why
pub fn verdict(src: &str) -> (&'static str, String) {
    let m = match wgsl::check(src) {
        Ok(m) => m,
        Err(d) => return ("error", d.first().map(|x| format!("{}", x)).unwrap_or_default()),
    };
    let consts = constants_line(src);
    let given: Vec<(&str, f64)> = consts.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let o = match wgsl::compile_module(&m, &wgsl::CompileOptions::new(shader::VOLL)) {
        Ok(o) => o,
        Err(d) => return ("error", d.first().map(|x| format!("compile: {}", x)).unwrap_or_default()),
    };
    let first_vs = o.entries.iter().find(|e| e.stage == wgsl::ir::Stage::Vertex).map(|e| e.name.clone());
    for e in &o.entries {
        if let Err(msg) = o.check_pipeline(&e.name, &given, &wgsl::Limits::WEBGPU) {
            return ("pipeline-error", format!("{}: {}", e.name, msg));
        }
        if e.stage == wgsl::ir::Stage::Fragment {
            if let Some(vs) = first_vs.as_deref().and_then(|n| o.entry(n)) {
                if let Err(msg) = wgsl::check_interface(&o.module, vs, e) {
                    return ("pipeline-error", format!("{}: {}", e.name, msg));
                }
            }
        }
    }
    ("valid", String::new())
}

// ---------------------------------------------------------------- the groups

/// the modules of the spirv group: (name, source)
fn sources() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for (n, s) in wgsl::bench::ALL {
        v.push((format!("bench_{}", n), String::from(s)));
    }
    for (n, s) in crate::coverage::SOURCES {
        v.push((format!("cover_{}", n), String::from(*s)));
    }
    for (i, (_, s)) in crate::checks::PASS.iter().enumerate() {
        v.push((format!("check_{}", i), String::from(*s)));
    }
    let exp = expected();
    for (p, s) in corpus_files() {
        if exp.iter().any(|(q, v)| *q == p && v != "error") && !KNOWN_REJECT.contains(&p.as_str()) {
            v.push((format!("corpus_{}", p.replace(['/', '.'], "_")), s));
        }
    }
    v
}

pub fn spirv(t: &mut T) {
    let srcs = sources();
    let val = find_val();
    let dir = plat::fs::join(OUT, "spv");
    if val.is_some() {
        plat::mkdir(OUT);
        plat::mkdir(&dir);
    }
    let (mut modules, mut ours, mut theirs, mut libs) = (0u32, 0u32, 0u32, 0u32);
    for (name, src) in &srcs {
        for minor in [shader::BASIS, shader::VOLL] {
            let opt = wgsl::CompileOptions::new(minor);
            let r = wgsl::compile(src, &opt);
            // modules without entry points: every function in a library module
            let r = match r {
                Ok(o) if o.entries.is_empty() => {
                    libs += 1;
                    wgsl::spirv::compile_library(src, &opt)
                }
                other => other,
            };
            let o = match r {
                Ok(o) => o,
                Err(d) => {
                    out(&format!("    {} 1.{}: {}\n", name, minor, d.first().map(|x| format!("{}", x)).unwrap_or_default()));
                    t.check(&format!("{} 1.{} compiles", name, minor), false);
                    continue;
                }
            };
            let again = wgsl::compile(src, &opt).ok().filter(|a| !a.entries.is_empty()).or_else(|| wgsl::spirv::compile_library(src, &opt).ok());
            let same = again.is_some_and(|a| a.entries.len() == o.entries.len() && a.entries.iter().zip(&o.entries).all(|(x, y)| x.words == y.words));
            t.check(&format!("{} 1.{}: deterministic", name, minor), same);
            for e in &o.entries {
                modules += 1;
                match shader_val(&e.words, &e.needs) {
                    Ok(()) => ours += 1,
                    Err(m) => {
                        out(&format!("    {} {} 1.{}: shader::val: {}\n", name, e.name, minor, m));
                        t.check(&format!("{} {} 1.{}: shader::val", name, e.name, minor), false);
                        continue;
                    }
                }
                t.ok += 1;
                if let Some(v) = &val {
                    let file = plat::fs::join(&dir, &format!("{}.{}.1{}.spv", name, e.name, minor));
                    plat::write_file(&file, &shader::to_bytes(&e.words));
                    let env = if minor >= 4 { "vulkan1.3" } else { "vulkan1.1" };
                    let mut args = alloc::vec!["--target-env", env];
                    if e.needs.uniform_standard_layout {
                        args.push("--uniform-buffer-standard-layout");
                    }
                    args.push(&file);
                    match tool(v, &args) {
                        Ok((0, _)) => {
                            theirs += 1;
                            t.ok += 1;
                        }
                        Ok((_, msg)) => {
                            let first = msg.lines().take(3).collect::<Vec<_>>().join(" | ");
                            out(&format!("    spirv-val {}: {}\n", file, first));
                            t.check(&format!("spirv-val {}", file), false);
                        }
                        Err(m) => t.check(&format!("spirv-val {}: {}", file, m), false),
                    }
                }
            }
        }
    }
    out(&format!("   spirv: {} sources ({} as libraries), {} modules: shader::val {} ok, spirv-val {}\n", srcs.len(), libs, modules, ours,
                 if val.is_some() { format!("{} ok", theirs) } else { String::from("not found (skipped)") }));
}

pub fn corpus(t: &mut T) {
    let exp = expected();
    let tint = expected_messages();
    let files = corpus_files();
    t.check("expected.json read", !exp.is_empty());
    t.eq("corpus files", files.len(), exp.len());
    let (mut agree, mut accept_reject, mut reject_accept, mut pipe_valid, mut pipe_err) = (0, 0, 0, 0, 0);
    let (mut pos_same, mut pos_all, mut warn_same, mut warn_all) = (0, 0, 0, 0);
    let mut lists: Vec<String> = Vec::new();
    for (path, src) in &files {
        let Some(want) = exp.iter().find(|(p, _)| p == path).map(|x| x.1.as_str()) else {
            t.check(&format!("{} in expected.json", path), false);
            continue;
        };
        let (got, why) = verdict(src);
        let msgs: &[TintMessage] = tint.iter().find(|(q, _)| q == path).map(|x| x.1.as_slice()).unwrap_or(&[]);
        let checked = wgsl::check(src);
        // the first error: the same place as Tint's?
        if let (Err(d), Some(first)) = (&checked, msgs.iter().find(|m| m.kind == "error")) {
            if let Some(e) = d.iter().find(|x| x.error) {
                pos_all += 1;
                if (e.line, e.col) == (first.line, first.col) {
                    pos_same += 1;
                } else if t.verbose {
                    out(&format!("    position {}: {}:{} here, {}:{} in Tint ({})\n", path, e.line, e.col, first.line, first.col, first.text));
                }
            }
        }
        // the warnings and infos on the uniformity and diagnostic files: type and line
        if path.starts_with("unif/") || path.starts_with("diag/") {
            let ours: &[wgsl::Diagnostic] = match &checked {
                Ok(m) => &m.warnings,
                Err(d) => d,
            };
            let mut a: Vec<(&str, u32)> = ours.iter().filter(|d| !d.error).map(|d| (if d.info { "info" } else { "warning" }, d.line)).collect();
            let mut b: Vec<(&str, u32)> = msgs.iter().filter(|m| m.is_main() && m.kind != "error").map(|m| (m.kind.as_str(), m.line)).collect();
            a.sort();
            b.sort();
            warn_all += 1;
            if a == b {
                warn_same += 1;
            }
            t.eq(&format!("{}: warnings and infos (type, line) like Tint's", path), a, b);
        }
        if got == want {
            agree += 1;
            t.ok += 1;
            continue;
        }
        match (got, want) {
            ("valid", "error") | ("pipeline-error", "error") => {
                accept_reject += 1;
                let allowed = ALLOW_ACCEPT.contains(&path.as_str());
                if !allowed {
                    out(&format!("    {}: we accept ({}), Tint rejects\n", path, got));
                }
                t.check(&format!("{}: accepted although Tint rejects (not in ALLOW_ACCEPT)", path), allowed);
            }
            ("error", _) => {
                reject_accept += 1;
                let known = KNOWN_REJECT.contains(&path.as_str());
                if !known {
                    out(&format!("    {}: we reject, Tint: {} -- {}\n", path, want, why));
                }
                t.check(&format!("{}: rejected although Tint accepts (not in KNOWN_REJECT)", path), known);
            }
            ("valid", "pipeline-error") => {
                pipe_valid += 1;
                lists.push(format!("{} (pipeline-error in Tint, valid here)", path));
            }
            _ => {
                pipe_err += 1;
                out(&format!("    {}: we say {} ({}), Tint {}\n", path, got, why, want));
                t.check(&format!("{}: {} here, {} in Tint", path, got, want), false);
            }
        }
    }
    for l in &lists {
        out(&format!("    {}\n", l));
    }
    // every listed exception still happens (the lists stay current)
    for p in ALLOW_ACCEPT.iter().chain(KNOWN_REJECT) {
        let Some((_, src)) = files.iter().find(|(q, _)| q == p) else {
            t.check(&format!("{}: listed but not in the corpus", p), false);
            continue;
        };
        let want = exp.iter().find(|(q, _)| q == p).map(|x| x.1.as_str()).unwrap_or("");
        let (got, _) = verdict(src);
        t.check(&format!("{}: still differs from Tint (or remove it from the list)", p), got != want);
    }
    out(&format!("   corpus: {} files, {} agree with Tint; we accept / Tint rejects {}, we reject / Tint accepts {}, \
                  pipeline errors only Tint finds {}, other {}\n", files.len(), agree, accept_reject, reject_accept, pipe_valid, pipe_err));
    out(&format!("   corpus: first error at Tint's place in {} of {} files; warnings and infos like Tint's in {} of {} unif / diag files\n", pos_same, pos_all, warn_same, warn_all));
    t.check("the first error is at Tint's place in at least 435 files", pos_same >= 435);
}
