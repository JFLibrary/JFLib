// manifest -- update.json: parse, check the signature, pick the package of this system,
// compare versions
//
//   Manifest::parse(bytes) -> Result<Manifest, String>   (no signature check)
//   verify(bytes, sig_text, keys) -> bool   the Base64 Ed25519 signature of update.json.sig
//   target() -> "windows-x86_64" | "linux-x86_64" | "linux-aarch64" | ...
//   newer(a, b) -> bool                     "1.10.0" > "1.9.2", "2.0.0" > "2.0.0-rc1"

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Value};

/// the largest update.json read
pub const MAX_MANIFEST: usize = 1 << 20;
/// the largest update.json.sig read
pub const MAX_SIG: usize = 4096;

/// what a package of the manifest is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// the program file itself
    Exe,
    /// a JFA1 bundle (tools/assetpack.py) of files relative to the program's folder
    Bundle,
    /// an installer (.deb, setup): downloaded and checked, installed by the app or the user
    Package,
}

impl Kind {
    /// "exe", "bundle" or "package"
    pub fn name(self) -> &'static str {
        match self {
            Kind::Exe => "exe",
            Kind::Bundle => "bundle",
            Kind::Package => "package",
        }
    }

    /// the kind of a name (name())
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "exe" => Some(Kind::Exe),
            "bundle" => Some(Kind::Bundle),
            "package" => Some(Kind::Package),
            _ => None,
        }
    }
}

/// one file of a manifest
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// the system it is for (target())
    pub target: String,
    pub kind: Kind,
    /// as written in the manifest: relative to update.json's address, or absolute
    pub url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// a parsed update.json
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// the app it is for (Config::app)
    pub app: String,
    pub version: String,
    /// unix seconds (0: not given)
    pub date: i64,
    /// unix seconds after which the manifest is refused (0: never)
    pub expires: i64,
    /// release notes for the user
    pub notes: String,
    pub files: Vec<Package>,
}

impl Manifest {
    /// parse update.json (format 1; the signature is not checked here) -> the reason when
    /// it is malformed
    pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
        let v = json::parse(bytes).map_err(|e| format!("kein gültiges JSON ({} bei Byte {})", e.kind, e.offset))?;
        if v.get("format").and_then(Value::as_u64) != Some(1) {
            return Err(String::from("unbekanntes Format (erwartet \"format\": 1)"));
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        let app = text("app").filter(|s| !s.is_empty()).ok_or("\"app\" fehlt")?;
        let version = text("version").filter(|s| version_ok(s)).ok_or("\"version\" fehlt oder ist ungültig")?;
        let int = |k: &str| -> Result<i64, String> {
            match v.get(k) {
                None => Ok(0),
                Some(x) => x.as_i64().ok_or_else(|| format!("\"{}\" ist keine ganze Zahl", k)),
            }
        };
        let (date, expires) = (int("date")?, int("expires")?);
        let notes = text("notes").unwrap_or_default();
        let list = v.get("files").and_then(Value::as_array).ok_or("\"files\" fehlt")?;
        let mut files = Vec::with_capacity(list.len());
        for (i, f) in list.iter().enumerate() {
            files.push(package(f).map_err(|e| format!("files[{}]: {}", i, e))?);
        }
        Ok(Manifest { app, version, date, expires, notes, files })
    }

    /// the package for a target (the first one)
    pub fn package(&self, target: &str) -> Option<&Package> {
        self.files.iter().find(|p| p.target == target)
    }
}

fn package(f: &Value) -> Result<Package, String> {
    let text = |k: &str| f.get(k).and_then(Value::as_str);
    let target = text("target").filter(|s| !s.is_empty()).ok_or("\"target\" fehlt")?;
    let kind = Kind::parse(text("kind").unwrap_or("exe")).ok_or("unbekannte Art (\"kind\": exe, bundle, package)")?;
    let url = text("url").filter(|s| !s.is_empty() && !s.chars().any(|c| c.is_control() || c == ' ')).ok_or("\"url\" fehlt")?;
    let size = f.get("size").and_then(Value::as_u64).ok_or("\"size\" fehlt")?;
    let hash = text("sha256").and_then(crypto::unhex).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok());
    let sha256 = hash.ok_or("\"sha256\" fehlt oder hat nicht 64 Hex-Zeichen")?;
    Ok(Package { target: String::from(target), kind, url: String::from(url), size, sha256 })
}

// a version: digits, letters, '.', '-', '+' (no separators of the state file)
fn version_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-._".contains(&b))
}

/// whether sig_text (update.json.sig: the Base64 Ed25519 signature, blanks around it ignored)
/// signs bytes with one of keys
pub fn verify(bytes: &[u8], sig_text: &[u8], keys: &[[u8; 32]]) -> bool {
    let Ok(text) = core::str::from_utf8(sig_text) else { return false };
    let Some(sig) = crypto::b64::decode(text.trim()).and_then(|b| <[u8; 64]>::try_from(b.as_slice()).ok()) else {
        return false;
    };
    keys.iter().any(|k| crypto::ed25519::verify(k, bytes, &sig))
}

/// the system this program runs on, as the manifest names it: "windows-x86_64",
/// "linux-x86_64", "linux-aarch64", "android-aarch64", "web-wasm32"
pub fn target() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "web-wasm32"
    } else if cfg!(target_os = "android") {
        if cfg!(target_arch = "x86_64") { "android-x86_64" } else { "android-aarch64" }
    } else if cfg!(windows) {
        if cfg!(target_arch = "aarch64") { "windows-aarch64" } else { "windows-x86_64" }
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// a is newer than b: the numbers before a '-' compared part by part ("1.10" > "1.9",
/// missing parts are 0, a leading 'v' is ignored); with equal numbers a release is newer than
/// a pre-release ("2.0.0" > "2.0.0-rc1"), pre-releases compare by SemVer's rules (numeric
/// parts as numbers, lower than text parts; more parts win); "+build" is ignored; a version
/// that does not parse is never newer and nothing is newer than it
pub fn newer(a: &str, b: &str) -> bool {
    use core::cmp::Ordering;
    fn key(v: &str) -> Option<(Vec<u64>, Option<&str>)> {
        let v = v.trim().trim_start_matches('v');
        let v = v.split_once('+').map_or(v, |(c, _)| c);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        if pre.is_some_and(|p| p.split('.').any(|i| i.is_empty() || !i.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))) {
            return None;
        }
        let nums = core.split('.').map(|p| if p.bytes().all(|c| c.is_ascii_digit()) { p.parse().ok() } else { None }).collect::<Option<Vec<u64>>>()?;
        Some((nums, pre))
    }
    fn pre_cmp(p: &str, q: &str) -> Ordering {
        let (mut pi, mut qi) = (p.split('.'), q.split('.'));
        loop {
            let ord = match (pi.next(), qi.next()) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(x), Some(y)) => match (x.bytes().all(|c| c.is_ascii_digit()), y.bytes().all(|c| c.is_ascii_digit())) {
                    (true, true) => (x.len().cmp(&y.len())).then(x.cmp(y)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => x.cmp(y),
                },
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
    }
    let (Some(ka), Some(kb)) = (key(a), key(b)) else { return false };
    for i in 0..ka.0.len().max(kb.0.len()) {
        let (x, y) = (ka.0.get(i).copied().unwrap_or(0), kb.0.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    match (ka.1, kb.1) {
        (None, Some(_)) => true,
        (Some(p), Some(q)) => pre_cmp(p, q) == Ordering::Greater,
        _ => false,
    }
}
