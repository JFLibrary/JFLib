// cargo jflib package: the app's installable packages, every style built in (JFLIB_STYLE) --
// tools/package.py of the JFLib repository (docs/PACKAGING.md) for an app outside it.
//
//   target/jflib-package/NAME_Setup.exe            setup: the stub (jflib-installer, built here like
//                                                  the app) + every style's program, a preview of
//                                                  each, the licences; the look picked at installation
//   target/jflib-package/DEB_VERSION_amd64.deb     deb: every style in /usr/lib/DEB/, debconf asks
//   target/jflib-package/NAME-VERSION-STYLE-x86_64.AppImage   appimage: one per style (linuxpkg.rs)
//   target/jflib-package/rpm/                      rpm: the .spec and its tarball, the .rpm by rpmbuild
//   target/jflib-package/flatpak/, ID.flatpak      flatpak: the manifest, the bundle by flatpak-builder
//   target/jflib-package/NAME-VERSION-x64.msix     msix: by MakeAppx (msix.rs)
//
// Inputs: PACKAGE next to Cargo.toml (README.md of cargo-jflib), PERMISSIONS (Flatpak's
// finish-args, MSIX's capabilities), res/app.ico and res/app_{48,128,256}.png.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cargo::{self, Project};
use crate::debfile::{self, Tree};
use crate::deflate;
use crate::notices;
use crate::pkgmeta::{self, Meta};
use crate::png;
use crate::sign::Signing;

pub(crate) const WIN: &str = "x86_64-pc-windows-msvc";
pub(crate) const LINUX: &str = "x86_64-unknown-linux-gnu";
const STYLES: [&str; 4] = ["winui", "classic", "kde", "macos"];
const PREVIEW: &str = "1100x700";
const THUMB: (usize, usize) = (480, 305);
/// the formats and the system each one packages
const FORMATS: [(&str, &str); 7] =
    [("setup", "win"), ("msix", "win"), ("deb", "linux"), ("appimage", "linux"), ("rpm", "linux"), ("flatpak", "linux"), ("dmg", "mac")];

const PK_MANIFEST: u32 = 1;
const PK_STYLE: u32 = 2;
const PK_PREVIEW: u32 = 3;
const PK_FILE: u32 = 5;
const PF_ZLIB: u32 = 1;
const NAME_MAX: usize = 47; // the payload's names and labels (lib/installer/payload.rs)

pub(crate) fn read(p: &Path) -> Result<Vec<u8>, String> {
    fs::read(p).map_err(|e| format!("{}: {e}", p.display()))
}

pub(crate) fn write(p: &Path, data: &[u8]) -> Result<(), String> {
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
    formats: Vec<&'static str>,
    /// every signature required (--sign)
    sign: bool,
    /// --repo DIR: a flat apt repository with the .deb
    repo: Option<PathBuf>,
    /// --update DIR [--update-url URL] [--update-key FILE]: a signed update channel per style
    update: Option<PathBuf>,
    update_url: String,
    update_key: Option<String>,
}

fn options(args: &[String]) -> Result<Opts, String> {
    let (mut styles, mut formats, mut no_win, mut no_deb, mut rest) = (None, "setup,deb".to_string(), false, false, Vec::new());
    let (mut sign, mut repo, mut update, mut update_url, mut update_key) = (false, None, None, String::new(), None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            flag @ ("--styles" | "--formats" | "--repo" | "--update" | "--update-url" | "--update-key") => {
                let v = args.get(i + 1).cloned().ok_or_else(|| format!("{flag} needs a value"))?;
                match flag {
                    "--styles" => styles = Some(v),
                    "--formats" => formats = v,
                    "--repo" => repo = Some(PathBuf::from(v)),
                    "--update" => update = Some(PathBuf::from(v)),
                    "--update-url" => update_url = v,
                    _ => update_key = Some(v),
                }
                i += 1;
            }
            "--no-win" => no_win = true,
            "--no-deb" => no_deb = true,
            "--sign" => sign = true,
            a => rest.push(a.to_string()),
        }
        i += 1;
    }
    let o = crate::opts(&rest)?;
    if o.target.is_some() || o.release || o.modes.is_some() || o.serve.is_some() || !o.free.is_empty() {
        return Err("package takes -p, --styles, --formats, --no-win, --no-deb, --sign, --repo, --update, --update-url and --update-key \
                    (it always builds --release)".to_string());
    }
    let known: Vec<&str> = FORMATS.iter().map(|f| f.0).collect();
    // (all: the formats of Windows and Linux; a dmg is made on a Mac, named by itself)
    let wanted: Vec<String> = if formats.trim() == "all" { FORMATS.iter().filter(|f| f.1 != "mac").map(|f| f.0.to_string()).collect() }
                              else { formats.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect() };
    if let Some(bad) = wanted.iter().find(|f| !known.contains(&f.as_str())) {
        return Err(format!("--formats: unknown {bad} (known: {}, all)", known.join(", ")));
    }
    // in the order they are made; --no-win / --no-deb drop every Windows / Linux format
    let formats = FORMATS.iter().filter(|(f, sys)| wanted.iter().any(|w| w == f) && !(no_win && *sys == "win" || no_deb && *sys == "linux"))
        .map(|f| f.0).collect();
    Ok(Opts { package: o.package, styles, formats, sign, repo, update, update_url, update_key })
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
    if triple == crate::mac::TRIPLE {
        cmd.env("MACOSX_DEPLOYMENT_TARGET", crate::mac::MAC_MIN);
    }
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

/// the [profile.*] sections of the workspace's Cargo.toml (a stub is built like the app)
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

/// a program of JFLib's that a package carries or packaging needs, made from its crates
pub(crate) struct Stub {
    /// the program's name (package and bin)
    pub name: &'static str,
    /// its crates (the first one's res/app.ico is its icon)
    pub deps: &'static [&'static str],
    /// src/main.rs
    pub source: &'static str,
    /// a command-line program (Windows: the console subsystem)
    pub console: bool,
    pub triple: &'static str,
    /// one static Linux file, whatever the app's .cargo/config.toml says (built in a target
    /// folder of its own: other rustflags would rebuild the app's crates)
    pub linux_static: bool,
}

pub(crate) const INSTALLER: Stub = Stub { name: "installer", deps: &["jflib-installer"], source: "#![no_std]\n#![no_main]\n\ninstaller::main!();\n",
                                          console: false, triple: WIN, linux_static: false };
pub(crate) const APPIMAGERT: Stub = Stub { name: "appimagert", deps: &["jflib-appimagert"], source: "#![no_std]\n#![no_main]\n\nappimagert::main!();\n",
                                           console: false, triple: LINUX, linux_static: true };
/// Ed25519 keys and signatures of an update channel with lib/crypto (cargo-jflib has no curve
/// arithmetic of its own), for the machine it runs on
pub(crate) const SIGNER: Stub = Stub { name: "jflib-sign", deps: &["jflib-rt", "jflib-plat", "jflib-crypto"], source: include_str!("signer.rs.in"),
                                       console: true, triple: if cfg!(windows) { WIN } else if cfg!(target_os = "macos") { crate::mac::TRIPLE } else { LINUX },
                                       linux_static: false };

/// a stub: its crates in a project of its own below the target folder, built like the app (its
/// profiles, its Cargo.lock, its .cargo/config.toml) -> the program
pub(crate) fn stub(p: &Project, version: &str, s: &Stub) -> Result<PathBuf, String> {
    let dir = p.target_dir.join("jflib-package").join(format!("{}-stub", s.name));
    let manifest = dir.join("Cargo.toml");
    let deps: String = s.deps.iter().map(|d| format!("{d} = \"={version}\"\n")).collect();
    write_changed(&manifest, format!(
        "# made by cargo jflib: {} ({})\n\
         [package]\nname = \"{}\"\nversion = \"{version}\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\n{deps}\n[build-dependencies]\njflib-build = \"={version}\"\n\n{}\n\
         # its own workspace, inside the app's folder or not\n[workspace]\n", s.name, s.deps.join(", "), s.name, profiles(&p.workspace_root)).as_bytes())?;
    write_changed(&dir.join("build.rs"), format!("fn main() {{\n    jflib_build::{}();\n}}\n", if s.console { "console" } else { "app" }).as_bytes())?;
    write_changed(&dir.join("src/main.rs"), s.source.as_bytes())?;
    // the versions the app was built with, on every run: the crates compiled for it are taken
    // again, and a stub's old lock cannot hold a crate the app's sources have replaced since
    let lock = dir.join("Cargo.lock");
    if p.workspace_root.join("Cargo.lock").is_file() {
        fs::copy(p.workspace_root.join("Cargo.lock"), &lock).map_err(|e| format!("{}: {e}", lock.display()))?;
    } else {
        let _ = fs::remove_file(&lock);
    }
    let krate = cargo::packages(&p.package.dir, &manifest)?.into_iter().find(|r| r.name == s.deps[0])
        .ok_or_else(|| format!("{}: resolves no {}", manifest.display(), s.deps[0]))?;
    let icon = krate.dir.join("res/app.ico");
    if icon.is_file() {
        write_changed(&dir.join("res/app.ico"), &read(&icon)?)?;
    }
    let mut cmd = cargo::command();
    cmd.current_dir(&p.package.dir).arg("build").arg("--release").args(["--target", s.triple]).arg("--manifest-path").arg(&manifest);
    if s.triple == crate::mac::TRIPLE {
        cmd.env("MACOSX_DEPLOYMENT_TARGET", crate::mac::MAC_MIN);
    }
    if s.linux_static {
        cmd.arg("--target-dir").arg(p.target_dir.join("jflib-package").join("static-target"));
        // these flags instead of every target.*.rustflags of the configuration
        cmd.env("CARGO_ENCODED_RUSTFLAGS", ["-C", "linker=rust-lld", "-C", "linker-flavor=ld.lld", "-C", "relocation-model=static",
                                            "-C", "target-feature=+crt-static"].join("\u{1f}"));
    } else {
        cmd.arg("--target-dir").arg(&p.target_dir);
    }
    cargo::build(&mut cmd)?.into_iter().filter_map(|a| a.executable).next().ok_or_else(|| format!("cargo built no {}", s.name))
}

/// files by their path: the licences of the crates
pub(crate) type Files = Vec<(String, Vec<u8>)>;

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

pub(crate) struct App<'a> {
    pub p: &'a Project,
    pub meta: Meta,
    pub name: String,
    pub display: String,
    pub version: String,
    pub styles: Vec<String>,
    /// the style crates' folders (their INFO)
    infos: BTreeMap<String, PathBuf>,
    /// the version of jflib the app resolves to: a stub's crates must have it
    pub jflib: String,
    pub work: PathBuf,
    pub dist: PathBuf,
    notices: [OnceCell<Files>; 2],
    /// what was made, what was left out and why, what to know
    pub made: RefCell<Vec<PathBuf>>,
    pub skipped: RefCell<Vec<String>>,
    pub notes: RefCell<Vec<String>>,
    /// the systems built for ("win", "linux")
    pub built: Vec<&'static str>,
    pub signing: OnceCell<Signing>,
    /// the .deb and its control file (the apt repository, the update channel)
    pub deb_made: RefCell<Option<(PathBuf, String)>>,
    /// the AppImage of each style (the update channel)
    pub appimages: RefCell<BTreeMap<String, PathBuf>>,
    /// (macOS) per style: the .dmg and the app bundle on it
    pub dmgs: RefCell<BTreeMap<String, (PathBuf, PathBuf)>>,
}

impl App<'_> {
    pub(crate) fn exe(&self, style: &str, target: &str) -> PathBuf {
        self.work.join(style).join(target).join(if target == "win" { format!("{}.exe", self.name) } else { self.name.clone() })
    }

    pub(crate) fn default_style(&self, target: &str) -> &str {
        let key = if target == "win" { "default_win" } else { "default_linux" };
        self.meta.get(key).filter(|d| self.styles.iter().any(|s| s == d)).unwrap_or(&self.styles[0])
    }

    /// the licences of every crate the app is built from for `target` (<package>-<version>/<file>)
    pub(crate) fn notices(&self, target: &str) -> Result<&[(String, Vec<u8>)], String> {
        let cell = &self.notices[(target == "linux") as usize];
        if cell.get().is_none() {
            let _ = cell.set(notices::files(self.p, if target == "win" { WIN } else { LINUX })?);
        }
        Ok(cell.get().map(Vec::as_slice).unwrap_or_default())
    }

    /// the package name of the Linux formats: PACKAGE deb_name=, else the program's in lower case
    pub(crate) fn pkg(&self) -> Result<String, String> {
        let pkg = self.meta.get("deb_name").map(String::from).unwrap_or_else(|| self.name.to_lowercase().replace('_', "-"));
        if pkg.len() < 2 || !pkg.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !pkg.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c)) {
            return Err(format!("{pkg:?} is no Linux package name: set deb_name= in PACKAGE (a-z, 0-9, + . -)"));
        }
        Ok(pkg)
    }

    /// PACKAGE summary=, else Cargo.toml's description, else the shown name
    pub(crate) fn summary(&self) -> &str {
        self.meta.get("summary").or(Some(self.p.package.description.as_str()).filter(|d| !d.is_empty())).unwrap_or(&self.display)
    }

    pub(crate) fn license(&self) -> &str {
        self.meta.get("license").or(Some(self.p.package.license.as_str()).filter(|l| !l.is_empty())).unwrap_or("LicenseRef-proprietary")
    }

    pub(crate) fn homepage(&self) -> &str {
        self.meta.get("homepage").unwrap_or(&self.p.package.homepage)
    }

    /// the Linux program needs the system's glibc (a program with a window: the GPU drivers)
    pub(crate) fn glibc(&self) -> Result<bool, String> {
        Ok(has_interpreter(&read(&self.exe(&self.styles[0], "linux"))?))
    }

    pub(crate) fn icon(&self, size: usize) -> Result<Vec<u8>, String> {
        let mut placeholder = false;
        let png = pkgmeta::icon(&self.p.package.dir, size, &mut placeholder)?;
        if placeholder {
            self.note("res/app_256.png is missing: a placeholder icon");
        }
        Ok(png)
    }

    pub(crate) fn sign(&self) -> &Signing {
        self.signing.get().expect("signing is set up before the formats")
    }

    pub(crate) fn note(&self, text: &str) {
        if !self.notes.borrow().iter().any(|n| n == text) {
            println!("  ({text})");
            self.notes.borrow_mut().push(text.to_string());
        }
    }

    pub(crate) fn skip(&self, format: &str, why: &str) {
        println!("  [{format}] not made: {why}");
        self.skipped.borrow_mut().push(format!("{format}: {why}"));
    }

    pub(crate) fn made(&self, path: PathBuf) -> Result<(), String> {
        let size = fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
        println!("  {}: {size} bytes", path.display());
        self.made.borrow_mut().push(path);
        Ok(())
    }

    fn setup(&self, previews: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
        println!("[win] setup");
        let stub = read(&stub(self.p, &self.jflib, &INSTALLER)?)?;
        let mut man = vec![format!("app={}", self.name), format!("display={}", self.display), format!("version={}", self.version),
                           format!("publisher={}", self.meta.or("publisher", "")), format!("default={}", self.default_style("win"))];
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
        for (rel, data) in self.notices("win")? {
            let name = format!("licenses/{rel}");
            if name.len() <= NAME_MAX {
                entries.push((PK_FILE, PF_ZLIB, name, String::new(), deflate::zlib(data), data.len()));
                continue;
            }
            let folder = format!("licenses/{}.txt", rel.split('/').next().unwrap_or_default());
            let into = if folder.len() <= NAME_MAX { folder } else { "licenses/MORE-LICENSES.txt".to_string() };
            let text = combined.entry(into).or_default();
            text.extend(format!("==== {rel} ====\n\n").into_bytes());
            text.extend_from_slice(data);
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
        println!("  {} styles, stub {} bytes, payload {}", self.styles.len(), stub.len(), pay.len());
        // the setup finds its payload before the signature (lib/installer/payload.rs)
        self.sign().windows(std::slice::from_ref(&path), Some(&format!("{} Setup", self.display)))?;
        self.made(path)
    }

    fn deb(&self) -> Result<(), String> {
        println!("[linux] deb");
        let pkg = self.pkg()?;
        let (ver, display) = (&self.version, &self.display);
        let default = self.default_style("linux");
        let (mut data, mut control) = (Tree::new(), Tree::new());
        let mut size = 0;
        for s in &self.styles {
            let exe = read(&self.exe(s, "linux"))?;
            size += exe.len();
            data.insert(format!("usr/lib/{pkg}/{}-{s}", self.name), exe);
        }
        for (rel, d) in self.notices("linux")? {
            size += d.len();
            data.insert(format!("usr/share/doc/{pkg}/licenses/{rel}"), d.clone());
        }
        data.insert(format!("usr/share/applications/{pkg}.desktop"), pkgmeta::desktop_entry(&self.meta, display, &pkg, &pkg, None)?.into_bytes());
        let types = pkgmeta::filetypes(&self.meta)?;
        let own: Vec<_> = types.iter().filter(|t| t.own).collect();
        if !own.is_empty() {
            data.insert(format!("usr/share/mime/packages/{pkg}.xml"), pkgmeta::mime_package(&own).into_bytes());
        }
        for sz in [48, 128, 256] {
            data.insert(format!("usr/share/icons/hicolor/{sz}x{sz}/apps/{pkg}.png"), self.icon(sz)?);
        }
        let maintainer = self.meta.get("maintainer").map(String::from).or_else(|| self.p.package.authors.first().cloned())
            .unwrap_or_else(|| format!("{} <nobody@localhost>", self.meta.or("publisher", display)));
        let desc = self.meta.or("description", "");
        control.insert("control".into(), format!(
            "Package: {pkg}\nVersion: {ver}\nArchitecture: amd64\nSection: {}\nPriority: optional\nMaintainer: {maintainer}\n\
             Installed-Size: {}\nDepends: debconf (>= 0.5) | debconf-2.0{}Description: {}\n{} \
             Enthaelt das Programm in {} Stilen; die Auswahl erfolgt bei der Installation.\n",
            self.meta.or("section", "utils"), size / 1024 + 64,
            if self.glibc()? { ", libc6\nRecommends: libvulkan1, mesa-vulkan-drivers\n" } else { "\n" },
            self.summary(), if desc.is_empty() { String::new() } else { format!(" {desc}\n .\n") }, self.styles.len()).into_bytes());
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
        let out = debfile::deb(&control, &data, pkgmeta::now());
        let path = self.dist.join(format!("{pkg}_{ver}_amd64.deb"));
        write(&path, &out)?;
        *self.deb_made.borrow_mut() = Some((path.clone(), String::from_utf8_lossy(&control["control"]).to_string()));
        self.made(path)
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
    let wants = |f: &str| o.formats.contains(&f);
    if wants("dmg") && !cfg!(target_os = "macos") {
        return Err("a dmg is made on a Mac (Apple's hdiutil and codesign): there --formats dmg".to_string());
    }
    let host = if cfg!(windows) { Some("win") } else if cfg!(target_os = "linux") { Some("linux") } else { None };
    let mut targets: Vec<&str> = Vec::new();
    if wants("setup") || wants("msix") {
        targets.push("win");
    }
    if ["deb", "appimage", "rpm", "flatpak"].iter().any(|f| wants(f)) || (wants("setup") && host == Some("linux")) {
        targets.push("linux"); // on Linux the previews come from the Linux build
    }
    // a program with std links against the C runtime of the system it is built for, so it is built
    // on that system (JFLib's own runtime builds Linux programs on Windows and back)
    if p.package.std {
        for (system, formats, shown) in [("win", "setup, msix", "Windows"), ("linux", "deb, appimage, rpm, flatpak", "Linux")] {
            if targets.contains(&system) && host != Some(system) {
                return Err(format!(
                    "{name} has std (jflib's feature std): its {formats} are made on {shown} itself; here --formats \
                     with the formats of this system"
                ));
            }
        }
    }
    // the style crates (their INFO) and the version of jflib, which the stubs' crates must have
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
        notices: [OnceCell::new(), OnceCell::new()],
        made: RefCell::new(Vec::new()),
        skipped: RefCell::new(Vec::new()),
        notes: RefCell::new(Vec::new()),
        built: targets.clone(),
        signing: OnceCell::new(),
        deb_made: RefCell::new(None),
        appimages: RefCell::new(BTreeMap::new()),
        dmgs: RefCell::new(BTreeMap::new()),
    };
    if o.repo.is_some() && !wants("deb") {
        return Err("--repo is an apt repository of the .deb: it needs the format deb".to_string());
    }
    for s in &app.styles {
        for &t in &targets {
            println!("[{s}] build {t}");
            let exe = build(&p, if t == "win" { WIN } else { LINUX }, s)?;
            write(&app.exe(s, t), &read(&exe)?)?;
        }
    }
    let linux_signed = ["appimage", "flatpak"].iter().any(|f| wants(f)) || o.repo.is_some();
    let _ = app.signing.set(Signing::new(o.sign, targets.contains(&"win"), linux_signed, &app.work)?);
    if targets.contains(&"win") {
        // every style's program, before the setup and the MSIX carry it
        let exes: Vec<PathBuf> = app.styles.iter().map(|s| app.exe(s, "win")).collect();
        app.sign().windows(&exes, Some(&app.display))?;
    }
    for f in &o.formats {
        match *f {
            "setup" => {
                let mut previews = BTreeMap::new();
                for s in &app.styles {
                    match host.filter(|h| targets.contains(h)) {
                        Some(h) => match preview(&app.exe(s, h), &app.work, s) {
                            Ok(png) => {
                                previews.insert(s.clone(), png);
                            }
                            Err(e) => app.note(&format!("no preview of {s}: {e}")),
                        },
                        None => app.note(&format!("no preview of {s}: this machine runs neither build")),
                    }
                }
                app.setup(&previews)?;
            }
            "msix" => app.msix()?,
            "deb" => app.deb()?,
            "appimage" => app.appimages()?,
            "rpm" => app.rpm()?,
            "flatpak" => app.flatpak()?,
            "dmg" => {
                for s in &app.styles {
                    println!("[{s}] build mac");
                    let exe = build(&p, crate::mac::TRIPLE, s)?;
                    let dir = app.work.join(s).join("mac");
                    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                    let bundle = crate::mac::bundle(&p.package.dir, &app.version, &exe, &dir)?;
                    let file = if app.styles.len() > 1 { format!("{}-{}-{s}-arm64.dmg", app.name, app.version) }
                               else { format!("{}-{}-arm64.dmg", app.name, app.version) };
                    let out = app.dist.join(file);
                    crate::mac::dmg(&bundle, &out, &app.display)?;
                    app.dmgs.borrow_mut().insert(s.clone(), (out.clone(), bundle));
                    app.made(out)?;
                }
            }
            _ => {}
        }
    }
    if let Some(repo) = &o.repo {
        app.apt_repo(repo)?;
    }
    if let Some(update) = &o.update {
        app.update_channel(update, &o.update_url, o.update_key.as_deref())?;
    }
    println!("made: {}", app.made.borrow().iter().map(|m| m.display().to_string()).collect::<Vec<_>>().join(", "));
    for s in app.skipped.borrow().iter() {
        println!("not made: {s}");
    }
    for n in app.notes.borrow().iter() {
        println!("note: {n}");
    }
    app.sign().report();
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
        assert!(!has_interpreter(&[]));
    }

    #[test]
    fn formats() {
        let o = options(&["--formats".into(), "all".into(), "--no-deb".into()]).unwrap();
        assert_eq!(o.formats, ["setup", "msix"]);
        let o = options(&["--formats".into(), "flatpak,setup".into()]).unwrap();
        assert_eq!(o.formats, ["setup", "flatpak"]);
        assert!(options(&["--formats".into(), "snap".into()]).is_err());
    }
}
