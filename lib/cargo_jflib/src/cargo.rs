// Cargo itself: the project's package and target folder (cargo metadata), a build and its artifacts
// (--message-format=json-render-diagnostics: the diagnostics go to the terminal as Cargo renders them).

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::json::{self, Value};

/// the cargo that started us (`cargo jflib`), else the one on the PATH
pub(crate) fn command() -> Command {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

pub(crate) struct Package {
    pub name: String,
    pub version: String,
    pub id: String,
    pub dir: PathBuf,
    /// the name of its library target, when it has one (an app's code for the browser)
    pub lib: Option<String>,
    /// Cargo.toml's authors
    pub authors: Vec<String>,
    /// Cargo.toml's description, license and homepage (empty: none)
    pub description: String,
    pub license: String,
    pub homepage: String,
    /// a program with std: its dependency jflib has the feature std
    pub std: bool,
}

pub(crate) struct Project {
    pub target_dir: PathBuf,
    /// where Cargo.lock lies
    pub workspace_root: PathBuf,
    pub package: Package,
}

fn run_json(cmd: &mut Command) -> Result<Value, String> {
    let out = cmd.stderr(Stdio::inherit()).output().map_err(|e| format!("cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?} failed"));
    }
    json::parse(&String::from_utf8_lossy(&out.stdout))
}

/// the package to work on: -p NAME, else the one whose Cargo.toml is in the current folder, else the
/// workspace's only member
pub(crate) fn project(package: Option<&str>) -> Result<Project, String> {
    let meta = run_json(command().args(["metadata", "--format-version", "1", "--no-deps"]))?;
    let here = env::current_dir().map_err(|e| e.to_string())?.join("Cargo.toml");
    let members = meta.get("packages").map(Value::arr).unwrap_or_default();
    let same = |a: &str, b: &Path| Path::new(a).canonicalize().ok() == b.canonicalize().ok();
    let pick = members
        .iter()
        .find(|p| package.is_some_and(|n| p.s("name") == n))
        .or_else(|| if package.is_none() { members.iter().find(|p| same(p.s("manifest_path"), &here)) } else { None })
        .or_else(|| if package.is_none() && members.len() == 1 { members.first() } else { None })
        .ok_or_else(|| match package {
            Some(n) => format!("no package {n} in this workspace"),
            None => "several packages here: name one with -p".to_string(),
        })?;
    let lib = pick
        .get("targets")
        .map(Value::arr)
        .unwrap_or_default()
        .iter()
        .find(|t| t.get("kind").map(Value::arr).unwrap_or_default().iter().any(|k| k.str() == Some("lib")))
        .map(|t| t.s("name").to_string());
    Ok(Project {
        target_dir: PathBuf::from(meta.s("target_directory")),
        workspace_root: PathBuf::from(meta.s("workspace_root")),
        package: Package {
            name: pick.s("name").to_string(),
            version: pick.s("version").to_string(),
            id: pick.s("id").to_string(),
            dir: Path::new(pick.s("manifest_path")).parent().map(Path::to_path_buf).unwrap_or_default(),
            lib,
            authors: pick.get("authors").map(Value::arr).unwrap_or_default().iter().filter_map(Value::str).map(String::from).collect(),
            description: pick.get("description").and_then(Value::str).unwrap_or("").to_string(),
            license: pick.get("license").and_then(Value::str).unwrap_or("").to_string(),
            homepage: pick.get("homepage").and_then(Value::str).unwrap_or("").to_string(),
            std: pick.get("dependencies").map(Value::arr).unwrap_or_default().iter().any(|d| {
                d.s("name") == "jflib" && d.get("features").map(Value::arr).unwrap_or_default().iter().any(|f| f.str() == Some("std"))
            }),
        },
    })
}

/// a package among those a project resolves to
pub(crate) struct Resolved {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
}

/// every package `manifest` resolves to (cargo metadata with the dependencies, run in `cwd` for
/// its .cargo/config.toml)
pub(crate) fn packages(cwd: &Path, manifest: &Path) -> Result<Vec<Resolved>, String> {
    let meta = run_json(command().current_dir(cwd).arg("metadata").arg("--format-version").arg("1").arg("--manifest-path").arg(manifest))?;
    Ok(meta.get("packages").map(Value::arr).unwrap_or_default().iter().map(|p| Resolved {
        name: p.s("name").to_string(),
        version: p.s("version").to_string(),
        dir: Path::new(p.s("manifest_path")).parent().map(Path::to_path_buf).unwrap_or_default(),
    }).collect())
}

/// what one build produced
pub(crate) struct Artifact {
    pub package_id: String,
    pub kinds: Vec<String>,
    pub filenames: Vec<PathBuf>,
    pub executable: Option<PathBuf>,
}

/// run a cargo build / rustc command (the JSON format is added here) -> its artifacts
pub(crate) fn build(cmd: &mut Command) -> Result<Vec<Artifact>, String> {
    cmd.arg("--message-format=json-render-diagnostics").stderr(Stdio::inherit()).stdout(Stdio::piped());
    let out = cmd.output().map_err(|e| format!("cargo: {e}"))?;
    if !out.status.success() {
        return Err("the build failed".to_string());
    }
    let mut artifacts = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.starts_with('{')) {
        let m = json::parse(line)?;
        if m.s("reason") != "compiler-artifact" {
            continue;
        }
        let strings = |key: &str| m.get(key).map(Value::arr).unwrap_or_default().iter().filter_map(Value::str).map(String::from).collect::<Vec<_>>();
        artifacts.push(Artifact {
            package_id: m.s("package_id").to_string(),
            kinds: m.get("target").and_then(|t| t.get("kind")).map(Value::arr).unwrap_or_default().iter().filter_map(Value::str).map(String::from).collect(),
            filenames: strings("filenames").into_iter().map(PathBuf::from).collect(),
            executable: m.get("executable").and_then(Value::str).map(PathBuf::from),
        });
    }
    Ok(artifacts)
}
