// The licences an app's build must ship: those of every package it is built from (cargo metadata,
// the normal dependencies from the app on), each in <package>-<version>/ -- the package's
// LICENSE*, COPYING*, NOTICE* files, its licenses/ folder and the notices of the fonts and icons
// in its assets/ (a style's OFL-*.txt, *LICENSE*; build_rs.py's own_style_notices).

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use crate::cargo::{self, Project};
use crate::json::{self, Value};

fn tree(from: &Path, rel: &str, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = format!("{rel}/{}", e.file_name().to_string_lossy());
        if p.is_dir() {
            tree(&p, &name, out)?;
        } else {
            out.push((name, fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?));
        }
    }
    Ok(())
}

fn is_notice(name: &str, in_assets: bool) -> bool {
    let up = name.to_uppercase();
    if in_assets {
        up.contains("LICENSE") || up.starts_with("OFL") || up.starts_with("NOTICE") || up.starts_with("COPYING")
    } else {
        up.starts_with("LICENSE") || up.starts_with("COPYING") || up.starts_with("NOTICE")
    }
}

/// the notices of the packages the app is built from for `target` -> (<package>-<version>/<path>,
/// contents), sorted
pub(crate) fn files(p: &Project, target: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let out = cargo::command()
        .args(["metadata", "--format-version", "1", "--filter-platform", target])
        .current_dir(&p.package.dir)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed".to_string());
    }
    let meta = json::parse(&String::from_utf8_lossy(&out.stdout))?;
    let packages: HashMap<&str, &Value> = meta.get("packages").map(Value::arr).unwrap_or_default().iter().map(|v| (v.s("id"), v)).collect();
    let nodes: HashMap<&str, &Value> = meta
        .get("resolve")
        .and_then(|r| r.get("nodes"))
        .map(Value::arr)
        .unwrap_or_default()
        .iter()
        .map(|n| (n.s("id"), n))
        .collect();
    let mut seen = HashSet::new();
    let mut todo = VecDeque::from([p.package.id.as_str()]);
    while let Some(id) = todo.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        for d in nodes.get(id).and_then(|n| n.get("deps")).map(Value::arr).unwrap_or_default() {
            let normal = d.get("dep_kinds").map(Value::arr).unwrap_or_default().iter().any(|k| k.get("kind").is_none_or(Value::is_null));
            if normal {
                todo.push_back(d.s("pkg"));
            }
        }
    }
    let mut files = Vec::new();
    for id in seen {
        let Some(pkg) = packages.get(id) else { continue };
        let Some(dir) = Path::new(pkg.s("manifest_path")).parent() else { continue };
        let folder = format!("{}-{}", pkg.s("name"), pkg.s("version"));
        let mut names = HashSet::new();
        for (sub, in_assets) in [(dir.to_path_buf(), false), (dir.join("assets"), true)] {
            let Ok(entries) = fs::read_dir(&sub) else { continue };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name().to_string_lossy().to_string();
                let path = e.path();
                if path.is_file() && is_notice(&name, in_assets) && names.insert(name.clone()) {
                    files.push((format!("{folder}/{name}"), fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?));
                } else if path.is_dir() && !in_assets && name.eq_ignore_ascii_case("licenses") {
                    tree(&path, &format!("{folder}/licenses"), &mut files)?;
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// the notices of the packages the app is built from for `target` into `dest` -> how many packages
pub(crate) fn copy(p: &Project, target: &str, dest: &Path) -> Result<usize, String> {
    let files = files(p, target)?;
    let mut folders = HashSet::new();
    for (rel, data) in &files {
        let to = dest.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&to, data).map_err(|e| format!("{}: {e}", to.display()))?;
        folders.insert(rel.split('/').next().unwrap_or_default().to_string());
    }
    Ok(folders.len())
}
