// `cargo jflib mac [-p PKG] [--release] [--run [-- ARGS]]` -- the app as NAME.app for macOS
// (docs/APPLE.md AP7c): the program for aarch64-apple-darwin, Info.plist from PACKAGE (display,
// version, app_id, filetypes= as document types -- "own" ones exported as the app's types --,
// schemes= as URL types), the icon as NAME.icns from res/app_*.png, signed ad hoc; with `sandbox`
// in PERMISSIONS the App Sandbox and the entitlements its rules need.  The same bundle as the JFLib
// repository's tools/apple.py mac_bundle.  On a Mac only (Apple's SDK, codesign).  --run starts it
// through `open` (in the logged-in session, also from SSH) and waits for it.  jfgui_MAC_IDENTITY
// names a certificate of the keychain ("Developer ID Application: NAME (TEAM)"): then the bundle is
// signed with it, the hardened runtime and a secure time stamp instead of ad hoc (sign).  dmg puts a
// bundle on a compressed disk image beside a link to /Applications (cargo jflib package --formats dmg).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cargo;
use crate::pkgmeta::{self, Meta, xml};

pub(crate) const TRIPLE: &str = "aarch64-apple-darwin";
pub(crate) const MAC_MIN: &str = "13.0";
/// (ICNS type, PNG size): up to the largest res/app_*.png (an icon is never scaled up)
const ICNS: [(&[u8; 4], usize); 11] = [(b"icp4", 16), (b"icp5", 32), (b"ic11", 32), (b"icp6", 64), (b"ic12", 64), (b"ic07", 128),
                                       (b"ic13", 256), (b"ic08", 256), (b"ic14", 512), (b"ic09", 512), (b"ic10", 1024)];

pub(crate) fn run_cmd(args: &[String]) -> Result<(), String> {
    let (mut run, mut rest, mut app_args) = (false, Vec::new(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--run" => run = true,
            "--" => {
                app_args.extend(it.by_ref().cloned());
            }
            _ => rest.push(a.clone()),
        }
    }
    if !cfg!(target_os = "macos") {
        return Err("cargo jflib mac runs on a Mac (Apple's SDK and codesign)".into());
    }
    let o = crate::opts(&rest)?;
    let p = cargo::project(o.package.as_deref())?;
    let mut cmd = cargo::command();
    cmd.args(["build", "-p", &p.package.name, "--target", TRIPLE]).env("MACOSX_DEPLOYMENT_TARGET", MAC_MIN);
    if o.release {
        cmd.arg("--release");
    }
    let exe = cargo::build(&mut cmd)?.into_iter().filter(|a| a.package_id == p.package.id).find_map(|a| a.executable)
        .ok_or("cargo built no program")?;
    let out = p.target_dir.join("jflib-mac");
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let app = bundle(&p.package.dir, &p.package.version, &exe, &out)?;
    println!("{}", app.display());
    if run {
        let st = Command::new("open").arg("-W").arg("-n").arg(&app).arg("--args").args(&app_args).status()
            .map_err(|e| format!("open: {e}"))?;
        if !st.success() {
            return Err(format!("open {} failed", app.display()));
        }
    }
    Ok(())
}

/// the program exe as out/NAME.app: Contents/MacOS/NAME, Info.plist, Resources/NAME.icns, signed
pub(crate) fn bundle(dir: &Path, version: &str, exe: &Path, out: &Path) -> Result<PathBuf, String> {
    let name = exe.file_name().and_then(|n| n.to_str()).ok_or("the program has no name")?.to_string();
    let app = out.join(format!("{name}.app"));
    let _ = fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
    fs::copy(exe, macos.join(&name)).map_err(|e| format!("{}: {e}", exe.display()))?;
    let meta = Meta::read(dir);
    let icns = icns(dir)?;
    if let Some(i) = &icns {
        let res = contents.join("Resources");
        fs::create_dir_all(&res).map_err(|e| e.to_string())?;
        fs::write(res.join(format!("{name}.icns")), i).map_err(|e| e.to_string())?;
    }
    fs::write(contents.join("Info.plist"), info_plist(&meta, &name, version, icns.is_some())?).map_err(|e| e.to_string())?;
    fs::write(contents.join("PkgInfo"), "APPL????").map_err(|e| e.to_string())?;
    let ent = entitlements(pkgmeta::permissions(dir).as_deref());
    let ent_file = out.join(format!("{name}.entitlements"));
    if let Some(e) = &ent {
        fs::write(&ent_file, e).map_err(|e| e.to_string())?;
    }
    let signed = sign(&app, ent.as_ref().map(|_| ent_file.as_path()));
    let _ = fs::remove_file(&ent_file);
    signed?;
    Ok(app)
}

/// the certificate of jfgui_MAC_IDENTITY (None: ad hoc)
fn identity() -> Option<String> {
    std::env::var("jfgui_MAC_IDENTITY").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// sign a bundle or a disk image: ad hoc, or with jfgui_MAC_IDENTITY, the hardened runtime and a time
/// stamp (what notarization requires; a disk image gets neither runtime nor entitlements)
pub(crate) fn sign(path: &Path, entitlements: Option<&Path>) -> Result<(), String> {
    let ident = identity();
    let image = path.extension().is_some_and(|e| e == "dmg");
    let mut cmd = Command::new("codesign");
    cmd.args(["-f", "-s", ident.as_deref().unwrap_or("-")]);
    cmd.arg(if ident.is_some() { "--timestamp" } else { "--timestamp=none" });
    if ident.is_some() && !image {
        cmd.args(["--options", "runtime"]);
    }
    if let (Some(e), false) = (entitlements, image) {
        cmd.arg("--entitlements").arg(e);
    }
    let st = cmd.arg(path).status().map_err(|e| format!("codesign: {e}"))?;
    if !st.success() {
        return Err(format!("codesign {} failed", path.display()));
    }
    Ok(())
}

/// the link to /Applications the user drags the app onto
fn link_applications(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink("/Applications", dir.join("Applications")).map_err(|e| format!("the link to /Applications: {e}"));
    #[cfg(not(unix))]
    {
        let _ = dir;
        Err("a dmg is made on a Mac".to_string())
    }
}

/// the bundle on a compressed disk image (hdiutil, UDZO) beside a link to /Applications, signed like
/// the bundle when there is an identity
pub(crate) fn dmg(app: &Path, out: &Path, volume: &str) -> Result<(), String> {
    let stage = out.with_extension("dmg-stage");
    let _ = fs::remove_dir_all(&stage);
    let dir = stage.join(volume);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = app.file_name().ok_or("the bundle has no name")?;
    let run = |cmd: &mut Command, what: &str| -> Result<(), String> {
        let st = cmd.status().map_err(|e| format!("{what}: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("{what} failed")) }
    };
    let made = (|| {
        run(Command::new("ditto").arg(app).arg(dir.join(name)), "ditto")?; // keeps the signature and the modes
        link_applications(&dir)?;
        let _ = fs::remove_file(out);
        run(Command::new("hdiutil").args(["create", "-volname", volume, "-srcfolder"]).arg(&dir)
            .args(["-fs", "HFS+", "-format", "UDZO", "-ov", "-quiet"]).arg(out), "hdiutil create")
    })();
    let _ = fs::remove_dir_all(&stage);
    made?;
    if identity().is_some() {
        sign(out, None)?;
    }
    Ok(())
}

/// the icon as an ICNS file of PNG entries, None without res/app_*.png
fn icns(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let largest = fs::read_dir(dir.join("res")).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_prefix("app_")?.strip_suffix(".png")?.parse::<usize>().ok())
        .max();
    let Some(largest) = largest else { return Ok(None) };
    let mut body = Vec::new();
    for (ty, size) in ICNS.iter().filter(|(_, s)| *s <= largest) {
        let mut placeholder = false;
        let png = pkgmeta::icon(dir, *size, &mut placeholder)?;
        body.extend_from_slice(*ty);
        body.extend_from_slice(&(8 + png.len() as u32).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(Some(out))
}

fn string(k: &str, v: &str) -> String {
    format!("\t<key>{}</key>\n\t<string>{}</string>\n", xml(k), xml(v))
}

fn array(items: &[String]) -> String {
    let mut s = String::from("<array>");
    for i in items {
        s.push_str(&format!("<string>{}</string>", xml(i)));
    }
    s + "</array>"
}

/// Info.plist from PACKAGE (see the file header)
fn info_plist(meta: &Meta, name: &str, version: &str, icon: bool) -> Result<String, String> {
    let aid = pkgmeta::app_id(meta, name)?;
    let display = meta.or("display", name);
    let version = meta.or("version", version);
    let short: String = display.chars().take(15).collect();
    let mut d = String::new();
    for (k, v) in [("CFBundleExecutable", name), ("CFBundleIdentifier", aid.as_str()), ("CFBundleName", short.as_str()),
                   ("CFBundleDisplayName", display), ("CFBundlePackageType", "APPL"), ("CFBundleInfoDictionaryVersion", "6.0"),
                   ("CFBundleShortVersionString", version), ("CFBundleVersion", version), ("LSMinimumSystemVersion", MAC_MIN),
                   ("NSPrincipalClass", "NSApplication")] {
        d.push_str(&string(k, v));
    }
    d.push_str("\t<key>NSHighResolutionCapable</key>\n\t<true/>\n\t<key>CFBundleSupportedPlatforms</key>\n\t<array><string>MacOSX</string></array>\n");
    if let Some(p) = meta.get("publisher") {
        d.push_str(&string("NSHumanReadableCopyright", p));
    }
    if icon {
        d.push_str(&string("CFBundleIconFile", name));
    }
    let types = pkgmeta::filetypes(meta)?;
    if !types.is_empty() {
        d.push_str("\t<key>CFBundleDocumentTypes</key>\n\t<array>\n");
        for t in &types {
            d.push_str(&format!(
                "\t\t<dict><key>CFBundleTypeName</key><string>{}</string><key>CFBundleTypeRole</key><string>Editor</string>\
                 <key>CFBundleTypeExtensions</key>{}<key>CFBundleTypeMIMETypes</key>{}<key>LSHandlerRank</key><string>{}</string>{}</dict>\n",
                xml(&t.desc), array(&t.exts), array(std::slice::from_ref(&t.mime)), if t.own { "Owner" } else { "Alternate" },
                if t.own { format!("<key>LSItemContentTypes</key>{}", array(&[format!("{aid}.{}", t.exts[0])])) } else { String::new() }));
        }
        d.push_str("\t</array>\n");
        let own: Vec<_> = types.iter().filter(|t| t.own).collect();
        if !own.is_empty() {
            d.push_str("\t<key>UTExportedTypeDeclarations</key>\n\t<array>\n");
            for t in own {
                d.push_str(&format!(
                    "\t\t<dict><key>UTTypeIdentifier</key><string>{}</string><key>UTTypeDescription</key><string>{}</string>\
                     <key>UTTypeConformsTo</key><array><string>public.data</string></array><key>UTTypeTagSpecification</key>\
                     <dict><key>public.filename-extension</key>{}<key>public.mime-type</key>{}</dict></dict>\n",
                    xml(&format!("{aid}.{}", t.exts[0])), xml(&t.desc), array(&t.exts), array(std::slice::from_ref(&t.mime))));
            }
            d.push_str("\t</array>\n");
        }
    }
    let schemes = pkgmeta::schemes(meta)?;
    if !schemes.is_empty() {
        d.push_str(&format!("\t<key>CFBundleURLTypes</key>\n\t<array><dict><key>CFBundleURLName</key><string>{}</string>\
                             <key>CFBundleURLSchemes</key>{}</dict></array>\n", xml(&aid), array(&schemes)));
    }
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{d}</dict>\n</plist>\n"))
}

/// the App Sandbox's entitlements of PERMISSIONS -- only with `sandbox` (tools/apple.py
/// mac_entitlements has the same rules)
fn entitlements(perms: Option<&[(String, Vec<String>)]>) -> Option<String> {
    let perms = perms?;
    if !perms.iter().any(|(k, _)| k == "sandbox") {
        return None;
    }
    let mut keys: Vec<String> = vec!["com.apple.security.app-sandbox".into()];
    let (mut home_rw, mut home_ro, mut abs_rw, mut abs_ro) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let own = ["$CONFIG", "$APPDATA", "$APPCACHE", "$APPLOG", "$TEMP", "$EXE"];
    let folders = [("$DOWNLOADS", "files.downloads"), ("$PICTURES", "assets.pictures"), ("$MUSIC", "assets.music"), ("$VIDEOS", "assets.movies")];
    for (key, words) in perms {
        match key.as_str() {
            "net.connect" => keys.push("com.apple.security.network.client".into()),
            "net.listen" => keys.push("com.apple.security.network.server".into()),
            "fs.user" => keys.push("com.apple.security.files.user-selected.read-write".into()),
            "print" => keys.push("com.apple.security.print".into()),
            "location" => keys.push("com.apple.security.personal-information.location".into()),
            "fs.read" | "fs.write" | "plugins" => {
                let rw = key == "fs.write";
                for w in words {
                    // the folder above the glob
                    let base = match w.rfind('/') {
                        Some(k) if w[k + 1..].contains('*') => &w[..k],
                        _ => w.as_str(),
                    };
                    let top = base.split('/').next().unwrap_or("");
                    if own.contains(&top) {
                        continue;
                    }
                    if let Some((_, f)) = folders.iter().find(|(v, _)| *v == top) {
                        keys.push(format!("com.apple.security.{f}.read-{}", if rw { "write" } else { "only" }));
                    } else if ["$HOME", "$DOCUMENTS", "$DESKTOP"].contains(&top) {
                        let rel = base.replacen("$HOME", "", 1).replacen("$DOCUMENTS", "/Documents", 1).replacen("$DESKTOP", "/Desktop", 1);
                        (if rw { &mut home_rw } else { &mut home_ro }).push(format!("{}/", rel.trim_end_matches('/')));
                    } else if base.starts_with('/') {
                        (if rw { &mut abs_rw } else { &mut abs_ro }).push(format!("{}/", base.trim_end_matches('/')));
                    }
                }
            }
            _ => {}
        }
    }
    keys.sort();
    keys.dedup();
    let mut d = String::new();
    for k in &keys {
        // a read-only folder entitlement is dropped where the read-write one is there
        if k.ends_with(".read-only") && keys.contains(&k.replace(".read-only", ".read-write")) {
            continue;
        }
        d.push_str(&format!("\t<key>{}</key>\n\t<true/>\n", xml(k)));
    }
    for (k, mut v) in [("home-relative-path.read-write", home_rw), ("home-relative-path.read-only", home_ro),
                       ("absolute-path.read-write", abs_rw), ("absolute-path.read-only", abs_ro)] {
        if !v.is_empty() {
            v.sort();
            v.dedup();
            d.push_str(&format!("\t<key>com.apple.security.temporary-exception.files.{k}</key>\n\t{}\n", array(&v)));
        }
    }
    Some(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                  \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{d}</dict>\n</plist>\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_and_entitlements() {
        let m = Meta::read(Path::new("/nonexistent"));
        let p = info_plist(&m, "hallo", "1.2.3", true).unwrap();
        assert!(p.contains("<key>CFBundleIdentifier</key>\n\t<string>org.example.hallo</string>")
            && p.contains("<key>CFBundleShortVersionString</key>\n\t<string>1.2.3</string>") && p.contains("<key>CFBundleIconFile</key>"));
        let perms = vec![("sandbox".to_string(), vec![]), ("net.connect".into(), vec!["*".into()]), ("fs.write".into(), vec!["$DOWNLOADS/**".into()]),
                         ("fs.read".into(), vec!["$HOME/Projekte/**".into(), "$TEMP/x/**".into()])];
        let e = entitlements(Some(&perms)).unwrap();
        assert!(e.contains("com.apple.security.app-sandbox") && e.contains("network.client") && e.contains("files.downloads.read-write"));
        assert!(e.contains("home-relative-path.read-only</key>\n\t<array><string>/Projekte/</string></array>") && !e.contains("$TEMP"));
        assert!(entitlements(Some(&[("net.connect".into(), vec![])])).is_none() && entitlements(None).is_none());
    }
}
