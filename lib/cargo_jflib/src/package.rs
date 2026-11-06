// cargo jflib package: the app as a setup for Windows and a .deb for Linux, every style built in
// (JFLIB_STYLE), the look picked at installation -- tools/package.py of the JFLib repository
// (docs/PACKAGING.md) for an app outside it.
//
//   target/jflib-package/NAME_Setup.exe            the setup stub (jflib-installer, built here like
//                                                  the app) + the payload: every style's program,
//                                                  a preview of each, the licences
//   target/jflib-package/DEB_VERSION_amd64.deb     every style in /usr/lib/DEB/, debconf asks for
//                                                  the look, update-alternatives links /usr/bin/DEB
//
// Inputs: PACKAGE next to Cargo.toml (display, version, publisher, summary, description, generic,
// categories, section, deb_name, maintainer, default_win, default_linux, styles, filetypes,
// schemes, app_id), res/app.ico and res/app_{48,128,256}.png.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cargo::{self, Project};
use crate::debfile::{self, Tree};
use crate::deflate;
use crate::notices;
use crate::pkgmeta::{self, Meta};
use crate::png;

const WIN: &str = "x86_64-pc-windows-msvc";
const LINUX: &str = "x86_64-unknown-linux-gnu";
const STYLES: [&str; 4] = ["winui", "classic", "kde", "macos"];
const PREVIEW: &str = "1100x700";
const THUMB: (usize, usize) = (480, 305);

const PK_MANIFEST: u32 = 1;
const PK_STYLE: u32 = 2;
const PK_PREVIEW: u32 = 3;
const PK_FILE: u32 = 5;
const PF_ZLIB: u32 = 1;
const NAME_MAX: usize = 47; // the payload's names and labels (apps/installer/payload.rs)

fn read(p: &Path) -> Result<Vec<u8>, String> {
    fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn write(p: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fs::write(p, data).map_err(|e| format!("{}: {e}", p.display()))
}

/// write only when the contents differ (a file the build script watches keeps its time)
fn write_changed(p: &Path, data: &[u8]) -> Result<(), String> {
    if fs::read(p).ok().as_deref() == Some(data) { Ok(()) } else { write(p, data) }
}

fn cut(s: &str, max: usize) -> &str {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

struct Opts {
    package: Option<String>,
    styles: Option<String>,
    formats: String,
}

fn options(args: &[String]) -> Result<Opts, String> {
    let (mut styles, mut formats, mut no_win, mut no_deb, mut rest) = (None, "setup,deb".to_string(), false, false, Vec::new());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--styles" | "--formats" => {
                let v = args.get(i + 1).cloned().ok_or_else(|| format!("{} needs a value", args[i]))?;
                if args[i] == "--styles" { styles = Some(v) } else { formats = v }
                i += 1;
            }
            "--no-win" => no_win = true,
            "--no-deb" => no_deb = true,
            a => rest.push(a.to_string()),
        }
        i += 1;
    }
    let o = crate::opts(&rest)?;
    if o.target.is_some() || o.release || o.modes.is_some() || o.serve.is_some() || !o.free.is_empty() {
        return Err("package takes -p, --styles, --formats, --no-win and --no-deb (it always builds --release)".to_string());
    }
    let mut list: Vec<&str> = formats.split(',').map(str::trim).filter(|f| !f.is_empty()).collect();
    if let Some(bad) = list.iter().find(|f| !["setup", "deb"].contains(f)) {
        return Err(format!("--formats: unknown {bad} (known: setup, deb)"));
    }
    list.retain(|f| !(no_win && *f == "setup" || no_deb && *f == "deb"));
    Ok(Opts { package: o.package, styles, formats: list.join(",") })
}

/// a style's shown name and description (INFO of its crate)
fn style_info(styles: &BTreeMap<String, PathBuf>, style: &str) -> (String, String) {
    let text = styles.get(style).and_then(|d| fs::read_to_string(d.join("INFO")).ok()).unwrap_or_default();
    let mut lines = text.lines();
    (lines.next().unwrap_or(style).to_string(), lines.next().unwrap_or("").to_string())
}

/// the app built for `triple` with `style` -> its program
fn build(p: &Project, triple: &str, style: &str) -> Result<PathBuf, String> {
    let mut cmd = cargo::command();
    cmd.current_dir(&p.package.dir).env("JFLIB_STYLE", style).args(["build", "--release", "-p", &p.package.name, "--target", triple]);
    let programs: Vec<PathBuf> = cargo::build(&mut cmd)?.into_iter().filter(|a| a.package_id == p.package.id).filter_map(|a| a.executable).collect();
    programs.iter().find(|e| e.file_stem().is_some_and(|s| s == p.package.name.as_str())).or(programs.first()).cloned()
        .ok_or_else(|| format!("{}: cargo built no program for {triple}", p.package.name))
}

/// a preview of the program as the setup shows it: a screenshot, scaled down
fn preview(exe: &Path, work: &Path, style: &str) -> Result<Vec<u8>, String> {
    let shot = work.join(format!("preview_{style}.png"));
    let _ = fs::remove_file(&shot);
    let out = Command::new(exe).arg("--shot").arg(&shot).args(["--size", PREVIEW, "--theme", "light", "--anim", "off"])
        .current_dir(work).stdin(Stdio::null()).output().map_err(|e| format!("{}: {e}", exe.display()))?;
    if !shot.is_file() {
        let why = String::from_utf8_lossy(&out.stderr).trim().chars().rev().take(400).collect::<String>().chars().rev().collect::<String>();
        return Err(format!("{} --shot wrote no picture ({}){}", exe.display(), out.status, if why.is_empty() { String::new() } else { format!(": {why}") }));
    }
    let img = png::decode(&read(&shot)?)?;
    Ok(png::encode(&png::downscale(&img, THUMB.0, THUMB.1)))
}

/// the [profile.*] sections of the workspace's Cargo.toml (the stub is built like the app)
fn profiles(root: &Path) -> String {
    let text = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            inside = t.starts_with("[profile.");
        }
        if inside {
            out += line;
            out.push('\n');
        }
    }
    if out.is_empty() {
        out = "[profile.release]\npanic = \"abort\"\nopt-level = 2\ncodegen-units = 1\nlto = \"fat\"\ndebug = 0\nstrip = \"symbols\"\n".to_string();
    }
    out
}

/// the setup stub: jflib-installer in a project of its own below the target folder, built for
/// Windows like the app (its profiles, its Cargo.lock, its .cargo/config.toml) -> the program
fn stub(p: &Project, version: &str) -> Result<Vec<u8>, String> {
    let dir = p.target_dir.join("jflib-package").join("setup-stub");
    let manifest = dir.join("Cargo.toml");
    write_changed(&manifest, format!(
        "# made by cargo jflib package: the setup stub of the app's Windows package (jflib-installer)\n\
         [package]\nname = \"installer\"\nversion = \"{version}\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\njflib-installer = \"={version}\"\n\n[build-dependencies]\njflib-build = \"={version}\"\n\n{}\n\
         # its own workspace, inside the app's folder or not\n[workspace]\n", profiles(&p.workspace_root)).as_bytes())?;
    write_changed(&dir.join("build.rs"), b"fn main() {\n    jflib_build::app();\n}\n")?;
    write_changed(&dir.join("src/main.rs"), b"#![no_std]\n#![no_main]\n\ninstaller::main!();\n")?;
    // the versions the app was built with, on every run: the crates compiled for it are taken
    // again, and a stub's old lock cannot hold a crate the app's sources have replaced since
    let lock = dir.join("Cargo.lock");
    if p.workspace_root.join("Cargo.lock").is_file() {
        fs::copy(p.workspace_root.join("Cargo.lock"), &lock).map_err(|e| format!("{}: {e}", lock.display()))?;
    } else {
        let _ = fs::remove_file(&lock);
    }
    let inst = cargo::packages(&p.package.dir, &manifest)?.into_iter().find(|r| r.name == "jflib-installer")
        .ok_or("the setup stub resolves no jflib-installer")?;
    write_changed(&dir.join("res/app.ico"), &read(&inst.dir.join("res/app.ico"))?)?;
    let mut cmd = cargo::command();
    cmd.current_dir(&p.package.dir).arg("build").arg("--release").args(["--target", WIN]).arg("--manifest-path").arg(&manifest)
        .arg("--target-dir").arg(&p.target_dir);
    let exe = cargo::build(&mut cmd)?.into_iter().filter_map(|a| a.executable).next().ok_or("cargo built no setup stub")?;
    read(&exe)
}

/// one entry of the payload: kind, flags, name, label, data, its size unpacked
type Entry = (u32, u32, String, String, Vec<u8>, usize);

/// the payload the stub reads -> its bytes
fn payload(entries: &[Entry]) -> Vec<u8> {
    let mut out = b"AIPK".to_vec();
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let mut at = 8 + 128 * entries.len() as u64;
    for (kind, flags, name, label, data, size) in entries {
        for v in [*kind, data.len() as u32, *size as u32, *flags] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        for s in [name, label] {
            let mut b = cut(s, NAME_MAX).as_bytes().to_vec();
            b.resize(48, 0);
            out.extend(b);
        }
        at += data.len() as u64;
    }
    for e in entries {
        out.extend_from_slice(&e.4);
    }
    out
}

struct App<'a> {
    p: &'a Project,
    meta: Meta,
    name: String,
    display: String,
    version: String,
    styles: Vec<String>,
    /// the style crates' folders (their INFO)
    infos: BTreeMap<String, PathBuf>,
    /// the version of jflib the app resolves to: the stub's crates must have it
    jflib: String,
    work: PathBuf,
    dist: PathBuf,
}

impl App<'_> {
    fn exe(&self, style: &str, target: &str) -> PathBuf {
        self.work.join(style).join(target).join(if target == "win" { format!("{}.exe", self.name) } else { self.name.clone() })
    }

    fn default_style(&self, key: &str) -> &str {
        self.meta.get(key).filter(|d| self.styles.iter().any(|s| s == d)).unwrap_or(&self.styles[0])
    }

    fn setup(&self, previews: &BTreeMap<String, Vec<u8>>) -> Result<PathBuf, String> {
        println!("[win] setup");
        let stub = stub(self.p, &self.jflib)?;
        let mut man = vec![format!("app={}", self.name), format!("display={}", self.display), format!("version={}", self.version),
                           format!("publisher={}", self.meta.or("publisher", "")), format!("default={}", self.default_style("default_win"))];
        for s in &self.styles {
            man.push(format!("desc_{s}={}", style_info(&self.infos, s).1));
        }
        pkgmeta::filetypes(&self.meta)?;
        if let Some(t) = self.meta.get("filetypes").filter(|t| !t.is_empty()) {
            man.push(format!("filetypes={t}"));
        }
        let schemes = pkgmeta::schemes(&self.meta)?;
        if !schemes.is_empty() {
            man.push(format!("schemes={}", schemes.join(" ")));
        }
        man.push(format!("app_id={}", pkgmeta::app_id(&self.meta, &self.name)?));
        let mtext = (man.join("\n") + "\n").into_bytes();
        let mut entries = vec![(PK_MANIFEST, PF_ZLIB, "manifest".to_string(), String::new(), deflate::zlib(&mtext), mtext.len())];
        for s in &self.styles {
            let label = style_info(&self.infos, s).0;
            let exe = read(&self.exe(s, "win"))?;
            entries.push((PK_STYLE, PF_ZLIB, s.clone(), label.clone(), deflate::zlib(&exe), exe.len()));
            if let Some(png) = previews.get(s) {
                entries.push((PK_PREVIEW, 0, s.clone(), label, png.clone(), png.len()));
            }
        }
        // the licences in licenses/; one whose path is longer than a payload name goes, with its
        // path as a heading, into licenses/<package>.txt (or licenses/MORE-LICENSES.txt)
        let mut combined: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (rel, data) in notices::files(self.p, WIN)? {
            let name = format!("licenses/{rel}");
            if name.len() <= NAME_MAX {
                entries.push((PK_FILE, PF_ZLIB, name, String::new(), deflate::zlib(&data), data.len()));
                continue;
            }
            let folder = format!("licenses/{}.txt", rel.split('/').next().unwrap_or_default());
            let into = if folder.len() <= NAME_MAX { folder } else { "licenses/MORE-LICENSES.txt".to_string() };
            let text = combined.entry(into).or_default();
            text.extend(format!("==== {rel} ====\n\n").into_bytes());
            text.extend_from_slice(&data);
            text.extend_from_slice(b"\n\n");
        }
        for (name, data) in combined {
            entries.push((PK_FILE, PF_ZLIB, name, String::new(), deflate::zlib(&data), data.len()));
        }
        let pay = payload(&entries);
        let mut out = stub.clone();
        out.extend_from_slice(&pay);
        out.extend_from_slice(b"ASMINST1");
        out.extend_from_slice(&(stub.len() as u64).to_le_bytes());
        out.extend_from_slice(&(pay.len() as u64).to_le_bytes());
        let path = self.dist.join(format!("{}_Setup.exe", self.name));
        write(&path, &out)?;
        println!("[win] {}: {} bytes ({} styles, stub {}, payload {})", path.display(), out.len(), self.styles.len(), stub.len(), pay.len());
        Ok(path)
    }

    fn deb(&self) -> Result<PathBuf, String> {
        println!("[linux] deb");
        let pkg = self.meta.get("deb_name").map(String::from).unwrap_or_else(|| self.name.to_lowercase().replace('_', "-"));
        if pkg.len() < 2 || !pkg.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !pkg.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c)) {
            return Err(format!("{pkg:?} is no Debian package name: set deb_name= in PACKAGE (a-z, 0-9, + . -)"));
        }
        let (ver, display) = (&self.version, &self.display);
        let default = self.default_style("default_linux");
        let (mut data, mut control) = (Tree::new(), Tree::new());
        let mut size = 0;
        let mut glibc = false;
        for s in &self.styles {
            let exe = read(&self.exe(s, "linux"))?;
            glibc |= has_interpreter(&exe);
            size += exe.len();
            data.insert(format!("usr/lib/{pkg}/{}-{s}", self.name), exe);
        }
        for (rel, d) in notices::files(self.p, LINUX)? {
            size += d.len();
            data.insert(format!("usr/share/doc/{pkg}/licenses/{rel}"), d);
        }
        data.insert(format!("usr/share/applications/{pkg}.desktop"), pkgmeta::desktop_entry(&self.meta, display, &pkg, &pkg)?.into_bytes());
        let types = pkgmeta::filetypes(&self.meta)?;
        let own: Vec<_> = types.iter().filter(|t| t.own).collect();
        if !own.is_empty() {
            data.insert(format!("usr/share/mime/packages/{pkg}.xml"), pkgmeta::mime_package(&own).into_bytes());
        }
        let mut placeholder = false;
        for sz in [48, 128, 256] {
            data.insert(format!("usr/share/icons/hicolor/{sz}x{sz}/apps/{pkg}.png"), pkgmeta::icon(&self.p.package.dir, sz, &mut placeholder)?);
        }
        if placeholder {
            println!("  (res/app_256.png is missing: a placeholder icon)");
        }
        let maintainer = self.meta.get("maintainer").map(String::from).or_else(|| self.p.package.authors.first().cloned())
            .unwrap_or_else(|| format!("{} <nobody@localhost>", self.meta.or("publisher", display)));
        let desc = self.meta.or("description", "");
        control.insert("control".into(), format!(
            "Package: {pkg}\nVersion: {ver}\nArchitecture: amd64\nSection: {}\nPriority: optional\nMaintainer: {maintainer}\n\
             Installed-Size: {}\nDepends: debconf (>= 0.5) | debconf-2.0{}Description: {}\n{} \
             Enthaelt das Programm in {} Stilen; die Auswahl erfolgt bei der Installation.\n",
            self.meta.or("section", "utils"), size / 1024 + 64,
            if glibc { ", libc6\nRecommends: libvulkan1, mesa-vulkan-drivers\n" } else { "\n" },
            self.meta.or("summary", display), if desc.is_empty() { String::new() } else { format!(" {desc}\n .\n") }, self.styles.len()).into_bytes());
        let labels: Vec<String> = self.styles.iter().map(|s| style_info(&self.infos, s).0).collect();
        control.insert("templates".into(), format!(
            "Template: {pkg}/style\nType: select\nChoices-C: {}\nChoices: {}\nDefault: {default}\nDescription: Aussehen von {display}:\n \
             Der gewählte Stil ist fest in das Programm eingebaut. Ändern lässt er sich\n später mit „sudo dpkg-reconfigure {pkg}“.\n",
            self.styles.join(", "), labels.join(", ")).into_bytes());
        control.insert("config".into(), format!(
            "#!/bin/sh\nset -e\n. /usr/share/debconf/confmodule\ndb_input high {pkg}/style || true\ndb_go || true\nexit 0\n").into_bytes());
        let alts: Vec<String> = self.styles.iter().map(|s| format!("  update-alternatives --install /usr/bin/{pkg} {pkg} /usr/lib/{pkg}/{}-{s} {}",
                                                                   self.name, if s == default { 50 } else { 10 })).collect();
        control.insert("postinst".into(), format!(
            "#!/bin/sh\nset -e\n. /usr/share/debconf/confmodule\nif [ \"$1\" = \"configure\" ]; then\n{}\n  RET={default}\n  db_get {pkg}/style || true\n  \
             [ -x \"/usr/lib/{pkg}/{name}-$RET\" ] || RET={default}\n  update-alternatives --set {pkg} \"/usr/lib/{pkg}/{name}-$RET\"\n  \
             if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q || true; fi\n{}  \
             if command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi\n\
             fi\ndb_stop\nexit 0\n",
            alts.join("\n"),
            if own.is_empty() { "" } else { "  if command -v update-mime-database >/dev/null 2>&1; then update-mime-database /usr/share/mime || true; fi\n" },
            name = self.name).into_bytes());
        let rm: Vec<String> = self.styles.iter().map(|s| format!("  update-alternatives --remove {pkg} /usr/lib/{pkg}/{}-{s} || true", self.name)).collect();
        control.insert("prerm".into(), format!(
            "#!/bin/sh\nset -e\nif [ \"$1\" = \"remove\" ] || [ \"$1\" = \"deconfigure\" ]; then\n{}\nfi\nexit 0\n", rm.join("\n")).into_bytes());
        control.insert("postrm".into(), b"#!/bin/sh\nset -e\nif [ \"$1\" = \"purge\" ] && [ -e /usr/share/debconf/confmodule ]; then\n  \
            . /usr/share/debconf/confmodule\n  db_purge\nfi\nexit 0\n".to_vec());
        let mtime = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.parse().ok())
            .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
        let out = debfile::deb(&control, &data, mtime);
        let path = self.dist.join(format!("{pkg}_{ver}_amd64.deb"));
        write(&path, &out)?;
        println!("[linux] {}: {} bytes ({} styles)", path.display(), out.len(), self.styles.len());
        Ok(path)
    }
}

/// a program that names an interpreter (PT_INTERP): built against glibc, a dynamic program
fn has_interpreter(elf: &[u8]) -> bool {
    let u16_at = |at: usize| elf.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let (Some(phoff), Some(size), Some(count)) = (elf.get(0x20..0x28).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()) as usize),
                                                  u16_at(0x36), u16_at(0x38)) else { return false };
    elf.starts_with(b"\x7fELF") && (0..count).any(|k| elf.get(phoff + k * size..phoff + k * size + 4) == Some(&3u32.to_le_bytes()[..]))
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let o = options(args)?;
    if o.formats.is_empty() {
        return Err("nothing to package: --formats, --no-win and --no-deb leave no format".to_string());
    }
    let p = cargo::project(o.package.as_deref())?;
    let meta = Meta::read(&p.package.dir);
    let name = p.package.name.clone();
    let styles: Vec<String> = o.styles.as_deref().or(meta.get("styles")).unwrap_or("winui")
        .split([',', ' ']).filter(|s| !s.is_empty()).map(String::from).collect();
    if let Some(bad) = styles.iter().find(|s| !STYLES.contains(&s.as_str())) {
        return Err(format!("style {bad}: the styles are {}", STYLES.join(", ")));
    }
    let (want_setup, want_deb) = (o.formats.contains("setup"), o.formats.contains("deb"));
    let host = if cfg!(windows) { Some("win") } else if cfg!(target_os = "linux") { Some("linux") } else { None };
    let mut targets: Vec<&str> = Vec::new();
    if want_setup {
        targets.push("win");
    }
    if want_deb || (want_setup && host == Some("linux")) {
        targets.push("linux"); // on Linux the previews come from the Linux build
    }
    // the style crates (their INFO) and the version of jflib, which the stub's crates must have
    let (mut infos, mut jflib) = (BTreeMap::new(), None);
    for r in cargo::packages(&p.package.dir, &p.package.dir.join("Cargo.toml"))? {
        if let Some(s) = r.name.strip_prefix("jflib-style-") {
            infos.insert(s.to_string(), r.dir);
        } else if r.name == "jflib" {
            jflib = Some(r.version);
        }
    }
    let jflib = jflib.ok_or_else(|| format!("{name} does not depend on the crate jflib"))?;
    let app = App {
        p: &p,
        version: meta.get("version").map(String::from).unwrap_or_else(|| p.package.version.clone()),
        display: meta.get("display").map(String::from).unwrap_or_else(|| name.clone()),
        meta,
        name,
        styles,
        infos,
        jflib,
        work: p.target_dir.join("jflib-package").join("work").join(&p.package.name),
        dist: p.target_dir.join("jflib-package"),
    };
    for s in &app.styles {
        for &t in &targets {
            println!("[{s}] build {t}");
            let exe = build(&p, if t == "win" { WIN } else { LINUX }, s)?;
            write(&app.exe(s, t), &read(&exe)?)?;
        }
    }
    let mut made = Vec::new();
    if want_setup {
        let mut previews = BTreeMap::new();
        for s in &app.styles {
            match host.filter(|h| targets.contains(h)) {
                Some(h) => match preview(&app.exe(s, h), &app.work, s) {
                    Ok(png) => {
                        previews.insert(s.clone(), png);
                    }
                    Err(e) => println!("  (no preview of {s}: {e})"),
                },
                None => println!("  (no preview of {s}: this machine runs neither build)"),
            }
        }
        made.push(app.setup(&previews)?);
    }
    if want_deb {
        made.push(app.deb()?);
    }
    println!("packaged: {}", made.iter().map(|m| m.display().to_string()).collect::<Vec<_>>().join(", "));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_layout() {
        let pay = payload(&[(PK_MANIFEST, PF_ZLIB, "manifest".into(), String::new(), vec![1, 2, 3], 9),
                            (PK_FILE, 0, "x".repeat(60), String::new(), vec![4], 1)]);
        assert_eq!(&pay[..4], b"AIPK");
        assert_eq!(pay.len(), 8 + 2 * 128 + 4);
        assert_eq!(u64::from_le_bytes(pay[24..32].try_into().unwrap()), 8 + 256);
        assert_eq!(pay[8 + 128 + 32 + 47], 0); // the name cut to 47 bytes
        assert!(has_interpreter(&[]).eq(&false));
    }
}
