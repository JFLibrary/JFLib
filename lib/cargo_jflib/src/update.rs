// What a package's users update from (tools/package.py --repo / --update, tools/update_manifest.py,
// docs/UPDATES.md):
//
//   --repo DIR       a flat apt repository: pool/*.deb, Packages(.gz), Release, and signed with
//                    jfgui_GPG_KEY InRelease, Release.gpg and the public key repo.gpg
//                    (deb [signed-by=/path/repo.gpg] URL ./)
//   --update DIR     a signed channel per style for lib/update: DIR/STYLE/update.json and
//                    update.json.sig with the style's Windows program (kind exe), the .deb (kind
//                    package) and its AppImage (kind appimage); the Ed25519 key from --update-key
//                    FILE, jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY ("JFUPDKEY1 <64 hex digits>")
//   cargo jflib keygen FILE    a new key and its public key for update::Config

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cargo;
use crate::deflate;
use crate::package::{self, App, SIGNER};
use crate::pkgmeta;
use crate::sha256::{base64, hex, sha256, unhex};

const KEY_TAG: &str = "JFUPDKEY1";

/// the seed of a key text ("JFUPDKEY1 <hex>" or the bare hex)
fn parse_key(text: &str) -> Result<String, String> {
    let t = text.trim();
    let t = t.strip_prefix(KEY_TAG).unwrap_or(t).trim();
    if unhex(t).is_some_and(|b| b.len() == 32) { Ok(t.to_lowercase()) } else { Err("the update key is no JFUPDKEY1 key (64 hex digits)".to_string()) }
}

/// the update key: the file given, jfgui_UPDATE_KEY_FILE, jfgui_UPDATE_KEY -> None without one
fn load_key(file: Option<&str>) -> Result<Option<String>, String> {
    if let Some(f) = file.map(String::from).or_else(|| env::var("jfgui_UPDATE_KEY_FILE").ok().filter(|v| !v.is_empty())) {
        let text = fs::read_to_string(&f).map_err(|e| format!("the update key {f}: {e}"))?;
        return parse_key(&text).map(Some);
    }
    env::var("jfgui_UPDATE_KEY").ok().filter(|v| !v.is_empty()).map(|t| parse_key(&t)).transpose()
}

/// run the signer (built once for this machine) -> its output
fn signer(exe: &Path, seed: Option<&str>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdin(Stdio::null());
    if let Some(s) = seed {
        cmd.env("JFLIB_SIGN_SEED", s);
    }
    let out = cmd.output().map_err(|e| format!("{}: {e}", exe.display()))?;
    if !out.status.success() {
        return Err(format!("jflib-sign: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o + "\""
}

impl App<'_> {
    pub(crate) fn apt_repo(&self, out: &Path) -> Result<(), String> {
        println!("[linux] apt repository {}", out.display());
        let Some((deb, control)) = self.deb_made.borrow().clone() else {
            return Err("--repo: no .deb was made".to_string());
        };
        let name = deb.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let data = package::read(&deb)?;
        package::write(&out.join("pool").join(&name), &data)?;
        let packages = format!("{}Filename: pool/{name}\nSize: {}\nSHA256: {}\n\n", control, data.len(), hex(&sha256(&data)));
        let gz = deflate::gzip(packages.as_bytes());
        let (y, m, d, wd) = pkgmeta::civil(pkgmeta::now());
        let secs = pkgmeta::now() % 86400;
        let date = format!("{}, {d:02} {} {y} {:02}:{:02}:{:02} UTC", ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][wd as usize],
                           ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m as usize - 1],
                           secs / 3600, secs / 60 % 60, secs % 60);
        let origin = self.meta.or("publisher", &self.display);
        let release = format!("Origin: {origin}\nLabel: {origin}\nDate: {date}\nArchitectures: amd64\nSHA256:\n {} {} Packages\n {} {} Packages.gz\n",
                              hex(&sha256(packages.as_bytes())), packages.len(), hex(&sha256(&gz)), gz.len());
        for f in ["InRelease", "Release.gpg", "repo.gpg"] {
            let _ = fs::remove_file(out.join(f));
        }
        package::write(&out.join("Packages"), packages.as_bytes())?;
        package::write(&out.join("Packages.gz"), &gz)?;
        package::write(&out.join("Release"), release.as_bytes())?;
        if self.sign().has_gpg() {
            self.sign().apt_release(out)?;
        } else {
            self.sign().skip(&format!("apt repository {}: unsigned (jfgui_GPG_KEY)", out.display()))?;
        }
        self.made(out.join("Release"))
    }

    pub(crate) fn update_channel(&self, out: &Path, base_url: &str, key_file: Option<&str>) -> Result<(), String> {
        println!("[update] channel {}", out.display());
        let Some(seed) = load_key(key_file)? else {
            return self.sign().skip("update channel: no signing key (--update-key FILE, jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY; \
                                     make one with cargo jflib keygen FILE)");
        };
        let exe = package::stub(self.p, &self.jflib, &SIGNER)?;
        let public = signer(&exe, Some(&seed), &["public"])?;
        let app = self.meta.or("update_app", &self.name);
        let expires_days: u64 = self.meta.or("update_expires_days", "0").parse().map_err(|_| "PACKAGE update_expires_days: a number of days")?;
        let now = pkgmeta::now();
        for style in &self.styles {
            let mut files: Vec<(&str, &str, PathBuf, String)> = Vec::new();
            if self.built.contains(&"win") {
                files.push(("windows-x86_64", "exe", self.exe(style, "win"), format!("{}-{}-{style}.exe", self.name, self.version)));
            }
            if let Some((deb, _)) = self.deb_made.borrow().as_ref() {
                files.push(("linux-x86_64", "package", deb.clone(), deb.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()));
            }
            if let Some(img) = self.appimages.borrow().get(style) {
                files.push(("linux-x86_64", "appimage", img.clone(), img.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()));
            }
            // macOS: the app bundle (an installed app replaces its bundle) and the .dmg
            if let Some((dmg, bundle)) = self.dmgs.borrow().get(style) {
                let jfa = self.work.join(format!("{}-{}-{style}-mac.jfa", self.name, self.version));
                package::write(&jfa, &jfa1(bundle)?)?;
                files.push(("macos-aarch64", "app", jfa.clone(), jfa.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()));
                files.push(("macos-aarch64", "package", dmg.clone(), dmg.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string()));
            }
            if files.is_empty() {
                continue;
            }
            let dir = out.join(style);
            let mut entries = Vec::new();
            for (target, kind, src, name) in &files {
                let data = package::read(src)?;
                package::write(&dir.join(name), &data)?;
                let url = if base_url.is_empty() { name.clone() } else { format!("{}/{style}/{name}", base_url.trim_end_matches('/')) };
                entries.push(format!("  {{\n   \"target\": {},\n   \"kind\": {},\n   \"url\": {},\n   \"size\": {},\n   \"sha256\": {}\n  }}",
                                     json_str(target), json_str(kind), json_str(&url), data.len(), json_str(&hex(&sha256(&data)))));
            }
            let manifest = format!("{{\n \"format\": 1,\n \"app\": {},\n \"version\": {},\n \"date\": {now},\n{} \"notes\": {},\n \"files\": [\n{}\n ]\n}}\n",
                                   json_str(app), json_str(&self.version),
                                   if expires_days > 0 { format!(" \"expires\": {},\n", now + expires_days * 86400) } else { String::new() },
                                   json_str(self.meta.or("update_notes", "")), entries.join(",\n"));
            let path = dir.join("update.json");
            package::write(&path, manifest.as_bytes())?;
            let sig = unhex(&signer(&exe, Some(&seed), &["sign", &path.to_string_lossy()])?).filter(|s| s.len() == 64)
                .ok_or("jflib-sign wrote no signature")?;
            package::write(&dir.join("update.json.sig"), format!("{}\n", base64(&sig)).as_bytes())?;
            self.sign().done.borrow_mut().push(format!("update channel ({style}): {} (key {}...)", path.display(), &public[..16.min(public.len())]));
            self.made(path)?;
        }
        Ok(())
    }
}

/// cargo jflib keygen FILE [-p PKG]: a new update key in FILE (never overwritten), its public key
pub(crate) fn keygen(args: &[String]) -> Result<(), String> {
    let o = crate::opts(args)?;
    let [file] = o.free.as_slice() else { return Err("keygen FILE: where the new key goes".to_string()) };
    let path = PathBuf::from(file);
    if path.exists() {
        return Err(format!("{file} exists (a key is never overwritten)"));
    }
    let p = cargo::project(o.package.as_deref())?;
    let jflib = cargo::packages(&p.package.dir, &p.package.dir.join("Cargo.toml"))?.into_iter().find(|r| r.name == "jflib")
        .map(|r| r.version).ok_or_else(|| format!("{} does not depend on the crate jflib", p.package.name))?;
    let exe = package::stub(&p, &jflib, &SIGNER)?;
    let out = signer(&exe, None, &["keygen"])?;
    let (seed, public) = out.split_once(' ').ok_or("jflib-sign keygen wrote no key")?;
    let mut f = fs::OpenOptions::new();
    f.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        f.mode(0o600);
    }
    use std::io::Write;
    f.open(&path).and_then(|mut h| h.write_all(format!("{KEY_TAG} {seed}\n").as_bytes())).map_err(|e| format!("{file}: {e}"))?;
    println!("the update key is in {file}: keep it out of the repository (whoever has it can ship code to every installation)");
    println!("its public key, for update::Config::new(APP, VERSION, URL, &[\"{public}\"]):\n{public}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_json() {
        let k = "JFUPDKEY1 00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        assert_eq!(parse_key(k).unwrap().len(), 64);
        assert!(parse_key("JFUPDKEY1 0011").is_err());
        assert_eq!(json_str("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }
}

/// a folder as a JFA1 bundle (tools/assetpack.py's format, every file stored): the files below dir,
/// '/' separated names relative to it, sorted bytewise, names starting with '.' left out
fn jfa1(dir: &Path) -> Result<Vec<u8>, String> {
    fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
        for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let e = e.map_err(|e| format!("{}: {e}", dir.display()))?;
            let (path, name) = (e.path(), e.file_name());
            let name = name.to_str().ok_or_else(|| format!("{}: the name is not UTF-8", path.display()))?;
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                collect(root, &path, out)?;
            } else if path.is_file() {
                let rel: Vec<&str> = path.strip_prefix(root).unwrap_or(&path).components().filter_map(|c| c.as_os_str().to_str()).collect();
                out.push((rel.join("/"), package::read(&path)?));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let n32 = |n: usize, what: &str| u32::try_from(n).map_err(|_| format!("{what}: larger than 4 GiB"));
    let pool: usize = files.iter().map(|f| f.0.len()).sum();
    let mut data_off = 12 + 20 * files.len() + pool;
    let mut out = b"JFA1".to_vec();
    out.extend_from_slice(&n32(files.len(), "the bundle")?.to_le_bytes());
    out.extend_from_slice(&n32(pool, "the names")?.to_le_bytes());
    let mut name_off = 0;
    for (name, data) in &files {
        let len = u16::try_from(name.len()).map_err(|_| format!("{name}: the name is too long"))?;
        out.extend_from_slice(&n32(name_off, "the names")?.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&[0, 0]); // stored
        out.extend_from_slice(&n32(data_off, "the bundle")?.to_le_bytes());
        out.extend_from_slice(&n32(data.len(), name)?.to_le_bytes());
        out.extend_from_slice(&n32(data.len(), name)?.to_le_bytes());
        name_off += name.len();
        data_off += data.len();
    }
    for (name, _) in &files {
        out.extend_from_slice(name.as_bytes());
    }
    for (_, data) in &files {
        out.extend_from_slice(data);
    }
    n32(out.len(), "the bundle")?;
    Ok(out)
}
