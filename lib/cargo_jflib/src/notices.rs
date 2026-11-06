// The licences an app's build must ship: those of every package it is built from (cargo metadata,
// the normal dependencies from the app on), each in licenses/<package>-<version>/ -- the package's
// LICENSE*, COPYING*, NOTICE* files and its licenses/ folder.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use crate::cargo::{self, Project};
use crate::json::{self, Value};

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for e in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_tree(&p, &to.join(e.file_name()))?;
        } else {
            fs::copy(&p, to.join(e.file_name())).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    Ok(())
}

/// the notices of the packages the app is built from for `target` into `dest` -> how many packages
pub(crate) fn copy(p: &Project, target: &str, dest: &Path) -> Result<usize, String> {
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
    let mut n = 0;
    for id in seen {
        let Some(pkg) = packages.get(id) else { continue };
        let Some(dir) = Path::new(pkg.s("manifest_path")).parent() else { continue };
        let to = dest.join(format!("{}-{}", pkg.s("name"), pkg.s("version")));
        let mut any = false;
        for e in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
            let name = e.file_name().to_string_lossy().to_uppercase();
            let path = e.path();
            if path.is_file() && (name.starts_with("LICENSE") || name.starts_with("COPYING") || name.starts_with("NOTICE")) {
                fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                fs::copy(&path, to.join(e.file_name())).map_err(|e| format!("{}: {e}", path.display()))?;
                any = true;
            } else if path.is_dir() && name == "LICENSES" {
                copy_tree(&path, &to.join("licenses"))?;
                any = true;
            }
        }
        n += any as usize;
    }
    Ok(n)
}
