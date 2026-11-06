// The MSIX package of an app (tools/pkg_msix.py of the JFLib repository, docs/PACKAGING.md
// "MSIX"): the default Windows style's program, the licences, the logos and AppxManifest.xml,
// packed by MakeAppx.exe of the Windows SDK into NAME-VERSION-x64.msix; NAME.appinstaller when
// PACKAGE appinstaller_url= says where both are published.  Unsigned: Add-AppxPackage needs a
// trusted signature (signing comes with the next stage of cargo jflib).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::package::{self, App};
use crate::pkgmeta::{self, xml};
use crate::png;

/// lib/plat/win_autostart.rs asks StartupTask.GetAsync for this id
const STARTUP_TASK_ID: &str = "Autostart";
const LOGOS: [(&str, usize, usize); 4] = [("StoreLogo.png", 50, 50), ("Square44x44Logo.png", 44, 44), ("Square150x150Logo.png", 150, 150),
                                          ("Wide310x150Logo.png", 310, 150)];
/// URL schemes Windows keeps for itself
const RESERVED_SCHEMES: [&str; 50] = [
    "application.manifest", "application.reference", "batfile", "blob", "cmd", "com", "comfile", "craplog", "dll", "drvfile", "exefile",
    "explorer.exe", "fax", "file", "hlp", "hta", "htafile", "http", "https", "inffile", "insfile", "jar", "javascript", "jsefile", "lnk",
    "mailto", "ms-accountpictureprovider", "ms-appdata", "ms-appx", "ms-autoplay", "msi", "ms-excel", "msisam", "ms-powerpoint",
    "ms-settings", "ms-word", "regfile", "res", "scrfile", "scriptletfile", "shell", "shtml", "sysfile", "tel", "vbefile", "vbscript",
    "vbsfile", "windowsmediacenterapp", "wsffile", "wshfile"];

/// MakeAppx.exe: jfgui_MAKEAPPX, PATH, the newest x64 one of the Windows Kits
fn makeappx() -> Option<PathBuf> {
    if let Some(p) = env::var_os("jfgui_MAKEAPPX") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    if let Some(p) = env::split_paths(&env::var_os("PATH").unwrap_or_default()).map(|d| d.join("makeappx.exe")).find(|p| p.is_file()) {
        return Some(p);
    }
    let kits = PathBuf::from(env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:\\Program Files (x86)".into())).join("Windows Kits/10/bin");
    let key = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).map(|n| n.split('.').map(|v| v.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>());
    fs::read_dir(kits).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("x64/makeappx.exe").is_file()).max_by_key(key)
        .map(|p| p.join("x64/makeappx.exe"))
}

/// a version as the four numbers of an MSIX identity (the fourth is the Store's: 0)
fn msix_version(v: &str) -> Result<String, String> {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let nums: Vec<u32> = core.split('.').take(3).map(|n| n.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| format!("version {v:?}: no MSIX version"))?;
    if nums.is_empty() || nums.iter().any(|&n| n > 65535) {
        return Err(format!("version {v:?}: no MSIX version (up to three numbers 0..65535)"));
    }
    let mut n = nums;
    n.resize(3, 0);
    Ok(format!("{}.{}.{}.0", n[0], n[1], n[2]))
}

impl App<'_> {
    /// PACKAGE msix_name, else the app id ('_' as '-'): 3..50 of [A-Za-z0-9.-]
    fn identity_name(&self) -> Result<String, String> {
        let name = self.meta.get("msix_name").map(String::from).map_or_else(|| pkgmeta::app_id(&self.meta, &self.name).map(|a| a.replace('_', "-")), Ok)?;
        if !(3..=50).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(format!("MSIX identity name {name:?}: 3..50 letters, digits, '.', '-' (PACKAGE msix_name=)"));
        }
        Ok(name)
    }

    /// the Publisher (the signing certificate's subject): jfgui_MSIX_PUBLISHER, PACKAGE
    /// msix_publisher, else CN=<publisher>
    fn publisher(&self) -> Result<String, String> {
        let p = env::var("jfgui_MSIX_PUBLISHER").ok().filter(|p| !p.is_empty()).or_else(|| self.meta.get("msix_publisher").map(String::from))
            .unwrap_or_else(|| format!("CN={}", self.meta.or("publisher", &self.display)));
        let ok = ["CN=", "O=", "OU=", "L=", "S=", "C=", "E=", "STREET=", "DC=", "SERIALNUMBER="].iter().any(|k| p.starts_with(k) && p.len() > k.len());
        if ok { Ok(p) } else { Err(format!("MSIX publisher {p:?}: a distinguished name like CN=Name, O=Firm, C=DE")) }
    }

    fn appx_manifest(&self) -> Result<String, String> {
        let perms = pkgmeta::permissions(&self.p.package.dir);
        let keys: Vec<&str> = perms.iter().flatten().map(|(k, _)| k.as_str()).collect();
        let caps: Vec<&str> = match &perms {
            None => vec!["internetClient", "internetClientServer", "privateNetworkClientServer"],
            Some(_) => {
                let mut c = Vec::new();
                if keys.contains(&"net.connect") {
                    c.push("internetClient");
                }
                if keys.contains(&"net.listen") {
                    c.extend(["internetClientServer", "privateNetworkClientServer"]);
                }
                c
            }
        };
        let (display, desc) = (xml(&self.display), xml(self.summary()));
        let exe = format!("{}.exe", self.name);
        let mut ext = String::new();
        let mut seen = Vec::new();
        for t in pkgmeta::filetypes(&self.meta)? {
            for e in &t.exts {
                if seen.contains(e) {
                    return Err(format!("MSIX: .{e} is in two entries of PACKAGE filetypes= (a package declares each extension once)"));
                }
                seen.push(e.clone());
            }
            let name: String = t.exts[0].chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
            ext += &format!("        <uap:Extension Category=\"windows.fileTypeAssociation\">\n          <uap:FileTypeAssociation Name=\"{}\">\n            \
                             <uap:DisplayName>{}</uap:DisplayName>\n            <uap:SupportedFileTypes>\n{}            </uap:SupportedFileTypes>\n          \
                             </uap:FileTypeAssociation>\n        </uap:Extension>\n",
                            xml(&name), xml(&t.desc),
                            t.exts.iter().map(|e| format!("              <uap:FileType ContentType=\"{}\">.{e}</uap:FileType>\n", xml(&t.mime))).collect::<String>());
        }
        for s in pkgmeta::schemes(&self.meta)? {
            if RESERVED_SCHEMES.contains(&s.as_str()) || s.len() < 2 || s.len() > 39 {
                return Err(format!("MSIX: URL scheme {s:?} is reserved by Windows or has a bad length (2..39)"));
            }
            ext += &format!("        <uap:Extension Category=\"windows.protocol\">\n          <uap:Protocol Name=\"{}\">\n            \
                             <uap:DisplayName>{display}</uap:DisplayName>\n          </uap:Protocol>\n        </uap:Extension>\n", xml(&s));
        }
        if perms.is_none() || keys.contains(&"autostart") {
            ext += &format!("        <desktop:Extension Category=\"windows.startupTask\" Executable=\"{}\" EntryPoint=\"Windows.FullTrustApplication\">\n          \
                             <desktop:StartupTask TaskId=\"{STARTUP_TASK_ID}\" Enabled=\"false\" DisplayName=\"{display}\"/>\n        </desktop:Extension>\n",
                            xml(&exe));
        }
        let alias = self.meta.or("msix_alias", "");
        if !alias.is_empty() {
            let stem = alias.strip_suffix(".exe").unwrap_or("");
            if stem.is_empty() || stem.len() > 60 || !stem.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
                return Err(format!("MSIX: msix_alias {alias:?}: a file name ending in .exe"));
            }
            ext += &format!("        <uap3:Extension Category=\"windows.appExecutionAlias\" Executable=\"{}\" EntryPoint=\"Windows.FullTrustApplication\">\n          \
                             <uap3:AppExecutionAlias>\n            <desktop:ExecutionAlias Alias=\"{}\"/>\n          </uap3:AppExecutionAlias>\n        \
                             </uap3:Extension>\n", xml(&exe), xml(alias));
        }
        Ok(format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- written by cargo jflib package from PACKAGE -->\n\
             <Package xmlns=\"http://schemas.microsoft.com/appx/manifest/foundation/windows10\"\n         \
             xmlns:uap=\"http://schemas.microsoft.com/appx/manifest/uap/windows10\"\n         \
             xmlns:uap3=\"http://schemas.microsoft.com/appx/manifest/uap/windows10/3\"\n         \
             xmlns:desktop=\"http://schemas.microsoft.com/appx/manifest/desktop/windows10\"\n         \
             xmlns:rescap=\"http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities\"\n         \
             IgnorableNamespaces=\"uap uap3 desktop rescap\">\n  <Identity Name=\"{}\" Publisher=\"{}\"\n            Version=\"{}\" ProcessorArchitecture=\"x64\"/>\n  \
             <Properties>\n    <DisplayName>{display}</DisplayName>\n    <PublisherDisplayName>{}</PublisherDisplayName>\n    \
             <Logo>Assets\\StoreLogo.png</Logo>\n    <Description>{desc}</Description>\n  </Properties>\n  <Dependencies>\n    \
             <TargetDeviceFamily Name=\"Windows.Desktop\" MinVersion=\"10.0.17763.0\" MaxVersionTested=\"10.0.26100.0\"/>\n  </Dependencies>\n  \
             <Resources>\n    <Resource Language=\"{}\"/>\n  </Resources>\n  <Applications>\n    \
             <Application Id=\"App\" Executable=\"{}\" EntryPoint=\"Windows.FullTrustApplication\">\n      \
             <uap:VisualElements DisplayName=\"{display}\" Description=\"{desc}\" BackgroundColor=\"transparent\"\n                          \
             Square150x150Logo=\"Assets\\Square150x150Logo.png\" Square44x44Logo=\"Assets\\Square44x44Logo.png\">\n        \
             <uap:DefaultTile Wide310x150Logo=\"Assets\\Wide310x150Logo.png\"/>\n      </uap:VisualElements>\n{}    </Application>\n  </Applications>\n  \
             <Capabilities>\n{}    <rescap:Capability Name=\"runFullTrust\"/>\n  </Capabilities>\n</Package>\n",
            xml(&self.identity_name()?), xml(&self.publisher()?), msix_version(&self.version)?, xml(self.meta.or("publisher", &self.display)),
            xml(self.meta.or("language", "de-DE")), xml(&exe),
            if ext.is_empty() { String::new() } else { format!("      <Extensions>\n{ext}      </Extensions>\n") },
            caps.iter().map(|c| format!("    <Capability Name=\"{c}\"/>\n")).collect::<String>()))
    }

    /// the icon (h x h, at most w) centred on a transparent w x h canvas
    fn logo(&self, w: usize, h: usize) -> Result<Vec<u8>, String> {
        let icon = png::decode(&self.icon(w.min(h))?)?;
        let mut px = vec![0u8; w * h * 4];
        let (x0, y0) = ((w - icon.w) / 2, (h - icon.h) / 2);
        for y in 0..icon.h {
            let row = &icon.px[y * icon.w * 4..(y + 1) * icon.w * 4];
            px[((y0 + y) * w + x0) * 4..((y0 + y) * w + x0 + icon.w) * 4].copy_from_slice(row);
        }
        Ok(png::encode(&png::Image { w, h, px }))
    }

    pub(crate) fn msix(&self) -> Result<(), String> {
        println!("[win] msix");
        let style = self.default_style("win");
        let d = self.work.join("msix").join(style);
        let _ = fs::remove_dir_all(&d);
        package::write(&d.join(format!("{}.exe", self.name)), &package::read(&self.exe(style, "win"))?)?;
        for (rel, data) in self.notices("win")? {
            package::write(&d.join("licenses").join(rel), data)?;
        }
        for (name, w, h) in LOGOS {
            package::write(&d.join("Assets").join(name), &if w == h { self.icon(w)? } else { self.logo(w, h)? })?;
        }
        package::write(&d.join("AppxManifest.xml"), self.appx_manifest()?.as_bytes())?;
        let Some(tool) = makeappx() else {
            self.skip("msix", &format!("MakeAppx.exe not found (the Windows SDK, or jfgui_MAKEAPPX); only the folder {}", d.display()));
            return Ok(());
        };
        let out = self.dist.join(format!("{}-{}-x64.msix", self.name, self.version));
        let r = Command::new(&tool).args(["pack", "/o", "/d"]).arg(&d).arg("/p").arg(&out).stdin(Stdio::null()).output()
            .map_err(|e| format!("{}: {e}", tool.display()))?;
        if !r.status.success() {
            return Err(format!("MakeAppx pack:\n{}{}", String::from_utf8_lossy(&r.stdout), String::from_utf8_lossy(&r.stderr)));
        }
        self.note("msix: unsigned; Add-AppxPackage needs a trusted signature (the Store takes it unsigned and signs it itself)");
        self.made(out.clone())?;
        let base = self.meta.or("appinstaller_url", "").trim_end_matches('/');
        if base.is_empty() {
            self.note("msix: no PACKAGE appinstaller_url=, so no .appinstaller (updates through App Installer)");
            return Ok(());
        }
        if !(base.starts_with("http://") || base.starts_with("https://")) || base.contains(char::is_whitespace) {
            return Err(format!("PACKAGE appinstaller_url {base:?}: an http(s) address of a folder"));
        }
        let hours = self.meta.or("appinstaller_hours", "0");
        if hours.parse::<u8>().is_err() {
            return Err(format!("PACKAGE appinstaller_hours {hours:?}: 0..255"));
        }
        let v = msix_version(&self.version)?;
        let file = out.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- written by cargo jflib package from PACKAGE -->\n\
             <AppInstaller xmlns=\"http://schemas.microsoft.com/appx/appinstaller/2018\" Version=\"{v}\" Uri=\"{}\">\n  \
             <MainPackage Name=\"{}\" Publisher=\"{}\"\n               Version=\"{v}\" ProcessorArchitecture=\"x64\" Uri=\"{}\"/>\n  \
             <UpdateSettings>\n    <OnLaunch HoursBetweenUpdateChecks=\"{hours}\" ShowPrompt=\"true\" UpdateBlocksActivation=\"false\"/>\n    \
             <AutomaticBackgroundTask/>\n  </UpdateSettings>\n</AppInstaller>\n",
            xml(&format!("{base}/{}.appinstaller", self.name)), xml(&self.identity_name()?), xml(&self.publisher()?), xml(&format!("{base}/{file}")));
        let ai = self.dist.join(format!("{}.appinstaller", self.name));
        package::write(&ai, text.as_bytes())?;
        self.made(ai)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(msix_version("0.1.0").unwrap(), "0.1.0.0");
        assert_eq!(msix_version("2.5").unwrap(), "2.5.0.0");
        assert_eq!(msix_version("1.2.3-beta.1").unwrap(), "1.2.3.0");
        assert!(msix_version("70000.0.0").is_err());
    }
}
