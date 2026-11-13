// What every package reads from an app (tools/pkgmeta.py of the JFLib repository): PACKAGE next
// to Cargo.toml (key=value, '#' comments), its file types and URL schemes, the reverse-DNS id,
// the .desktop entry and the shared-mime-info file, the icons in res/.

use std::fs;
use std::path::Path;

use crate::png;

pub(crate) struct Meta(pub(crate) Vec<(String, String)>);

impl Meta {
    /// PACKAGE in `dir` (none: empty)
    pub(crate) fn read(dir: &Path) -> Meta {
        Meta(fs::read_to_string(dir.join("PACKAGE")).unwrap_or_default().lines().map(str::trim).filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect())
    }

    /// the value of `key` (the last line wins)
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub(crate) fn or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }
}

fn all(s: &str, ok: impl Fn(char) -> bool) -> bool {
    !s.is_empty() && s.chars().all(ok)
}

/// the reverse-DNS id: PACKAGE app_id=, else org.example.NAME; three or more parts of letters,
/// digits and '_', none starting with a digit (the Flatpak and D-Bus rules)
pub(crate) fn app_id(meta: &Meta, name: &str) -> Result<String, String> {
    let id = meta.get("app_id").map(String::from).unwrap_or_else(|| {
        format!("org.example.{}", name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
    });
    let parts: Vec<&str> = id.split('.').collect();
    let ok = parts.len() >= 3 && id.len() <= 255
        && parts.iter().all(|p| all(p, |c| c.is_ascii_alphanumeric() || c == '_') && !p.starts_with(|c: char| c.is_ascii_digit()));
    if ok { Ok(id) } else { Err(format!("PACKAGE app_id: {id:?} is no reverse-DNS id (org.example.App: three or more parts, letters, digits, '_')")) }
}

pub(crate) struct FileType {
    pub exts: Vec<String>,
    pub mime: String,
    pub desc: String,
    /// the app's own type (a shared-mime-info entry in the .deb)
    pub own: bool,
}

/// PACKAGE filetypes=: "ext[,ext..]|mime/type|description[|own]" entries separated by ';'
pub(crate) fn filetypes(meta: &Meta) -> Result<Vec<FileType>, String> {
    let mut out = Vec::new();
    for entry in meta.or("filetypes", "").split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let f: Vec<&str> = entry.split('|').map(str::trim).collect();
        let exts: Vec<String> = f[0].split(',').map(|e| e.trim().trim_start_matches('.').to_lowercase()).collect();
        let mime_ok = |m: &str| {
            let t = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || ".+-".contains(c);
            m.split_once('/').is_some_and(|(a, b)| all(a, t) && all(b, t))
        };
        let ok = (3..=4).contains(&f.len()) && (f.len() == 3 || f[3] == "own")
            && exts.iter().all(|e| e.len() <= 32 && all(e, |c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_+.-".contains(c)))
            && mime_ok(f[1]) && !f[2].is_empty() && !f[2].contains(['"', '<', '>', '&']);
        if !ok {
            return Err(format!("PACKAGE filetypes: bad entry {entry:?} (ext[,ext..]|mime/type|description[|own])"));
        }
        out.push(FileType { exts, mime: f[1].to_string(), desc: f[2].to_string(), own: f.len() == 4 });
    }
    Ok(out)
}

/// PACKAGE schemes=: the URL schemes the app opens (blanks, ',' or ';' between them)
pub(crate) fn schemes(meta: &Meta) -> Result<Vec<String>, String> {
    let out: Vec<String> = meta.or("schemes", "").split([';', ',', ' ']).filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
    for s in &out {
        if s.len() > 32 || !s.starts_with(|c: char| c.is_ascii_lowercase()) || !all(s, |c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+.-".contains(c)) {
            return Err(format!("PACKAGE schemes: bad scheme {s:?}"));
        }
    }
    Ok(out)
}

/// PACKAGE browser=: the app is a web browser (1; 0 or missing: not) -- it needs http and https
/// in schemes=
pub(crate) fn browser(meta: &Meta) -> Result<bool, String> {
    match meta.get("browser") {
        None | Some("0") => Ok(false),
        Some("1") => {
            let s = schemes(meta)?;
            if ["http", "https"].iter().all(|w| s.iter().any(|x| x == w)) {
                Ok(true)
            } else {
                Err("PACKAGE browser=1: schemes= must list http and https".to_string())
            }
        }
        Some(v) => Err(format!("PACKAGE browser: {v:?} is neither 1 nor 0")),
    }
}

/// PACKAGE categories= (default Utility;), a browser's with Network;WebBrowser; (the desktops
/// list their web browsers by them)
pub(crate) fn categories(meta: &Meta) -> Result<String, String> {
    let mut c = meta.or("categories", "Utility;").to_string();
    if browser(meta)? {
        for need in ["Network", "WebBrowser"] {
            if !c.split(';').any(|x| x.trim() == need) {
                if !c.is_empty() && !c.ends_with(';') {
                    c.push(';');
                }
                c += need;
                c.push(';');
            }
        }
    }
    Ok(c)
}

/// the shared-mime-info XML of the app's own file types
pub(crate) fn mime_package(types: &[&FileType]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n");
    for t in types {
        s += &format!("  <mime-type type=\"{}\">\n    <comment>{}</comment>\n", t.mime, t.desc);
        for e in &t.exts {
            s += &format!("    <glob pattern=\"*.{e}\"/>\n");
        }
        s += "  </mime-type>\n";
    }
    s + "</mime-info>\n"
}

/// the [Desktop Entry] of the app: `exe` on the Exec line with %U (schemes) or %F, the MIME types;
/// `wm_class`: StartupWMClass (JFLib's window class is the name the program was started by)
pub(crate) fn desktop_entry(meta: &Meta, display: &str, exe: &str, icon: &str, wm_class: Option<&str>) -> Result<String, String> {
    let (types, urls) = (filetypes(meta)?, schemes(meta)?);
    let mut mimes: Vec<String> = Vec::new();
    for m in types.iter().map(|t| t.mime.clone()).chain(urls.iter().map(|s| format!("x-scheme-handler/{s}"))) {
        if !mimes.contains(&m) {
            mimes.push(m);
        }
    }
    Ok(format!("[Desktop Entry]\nType=Application\nName={display}\nGenericName={}\nComment={}\nExec={exe} {}\nIcon={icon}\nTerminal=false\n\
                Categories={}\n{}",
               meta.or("generic", display), meta.or("summary", ""), if urls.is_empty() { "%F" } else { "%U" }, categories(meta)?,
               if mimes.is_empty() { String::new() } else { format!("MimeType={};\n", mimes.join(";")) })
        + &wm_class.map(|w| format!("StartupWMClass={w}\n")).unwrap_or_default())
}

/// PERMISSIONS next to Cargo.toml as (key, words) in file order; None without one (the app may
/// then do everything).  An `ask` line counts as the rule it asks for; reasons and comments go
pub(crate) fn permissions(dir: &Path) -> Option<Vec<(String, Vec<String>)>> {
    let text = fs::read_to_string(dir.join("PERMISSIONS")).ok()?;
    Some(text.lines().filter_map(|line| {
        let cut = [line.find('#'), line.find('"')].into_iter().flatten().min().unwrap_or(line.len());
        let mut words: Vec<String> = line[..cut].split_whitespace().map(String::from).collect();
        if words.first().is_some_and(|w| w == "ask") {
            words.remove(0);
        }
        (!words.is_empty()).then(|| (words.remove(0), words))
    }).collect())
}

pub(crate) fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// seconds since 1970 -> (year, month, day, weekday 0 = Sunday)
pub(crate) fn civil(secs: u64) -> (i64, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d, (days + 4).rem_euclid(7) as u32)
}

/// SOURCE_DATE_EPOCH, else now (seconds since 1970)
pub(crate) fn now() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// what the AppStream metainfo says of the app
pub(crate) struct About<'a> {
    pub display: &'a str,
    pub summary: &'a str,
    pub license: &'a str,
    pub homepage: &'a str,
    pub version: &'a str,
}

/// the AppStream metainfo (component type desktop-application) of the app
pub(crate) fn metainfo(meta: &Meta, about: &About, aid: &str, binary: &str) -> Result<String, String> {
    let (y, m, d, _) = civil(now());
    let dev_id = aid.split('.').take(2).collect::<Vec<_>>().join(".");
    let summary = about.summary.trim_end_matches('.');
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<component type=\"desktop-application\">\n  <id>{aid}</id>\n  \
        <metadata_license>CC0-1.0</metadata_license>\n  <project_license>{}</project_license>\n  <name>{}</name>\n  <summary>{}</summary>\n  \
        <description>\n    <p>{}</p>\n  </description>\n  <developer id=\"{dev_id}\">\n    <name>{}</name>\n  </developer>\n  \
        <launchable type=\"desktop-id\">{aid}.desktop</launchable>\n",
        xml(about.license), xml(about.display), xml(summary), xml(meta.or("description", summary)), xml(meta.or("publisher", about.display)));
    if !about.homepage.is_empty() {
        s += &format!("  <url type=\"homepage\">{}</url>\n", xml(about.homepage));
    }
    s += &format!("  <provides>\n    <binary>{binary}</binary>\n");
    for t in filetypes(meta)? {
        s += &format!("    <mediatype>{}</mediatype>\n", t.mime);
    }
    s += &format!("  </provides>\n  <content_rating type=\"oars-1.1\"/>\n  <releases>\n    <release version=\"{}\" date=\"{y:04}-{m:02}-{d:02}\"/>\n  \
                   </releases>\n</component>\n", xml(about.version));
    Ok(s)
}

/// the icon in size x size as PNG: res/app_<size>.png, else the next larger res/app_N.png scaled
/// down, else a placeholder (and `placeholder` is set)
pub(crate) fn icon(dir: &Path, size: usize, placeholder: &mut bool) -> Result<Vec<u8>, String> {
    let res = dir.join("res");
    let exact = res.join(format!("app_{size}.png"));
    if exact.is_file() {
        return fs::read(&exact).map_err(|e| format!("{}: {e}", exact.display()));
    }
    let mut have: Vec<(usize, std::path::PathBuf)> = fs::read_dir(&res).into_iter().flatten().flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            let s = n.strip_prefix("app_")?.strip_suffix(".png")?.parse().ok()?;
            Some((s, e.path()))
        })
        .filter(|(s, _)| *s >= size)
        .collect();
    have.sort();
    if let Some((_, big)) = have.first() {
        let img = png::decode(&fs::read(big).map_err(|e| format!("{}: {e}", big.display()))?).map_err(|e| format!("{}: {e}", big.display()))?;
        return Ok(png::encode(&png::downscale(&img, size, size)));
    }
    *placeholder = true;
    let p = png::placeholder(256.max(size));
    Ok(png::encode(&if p.w == size { p } else { png::downscale(&p, size, size) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_keys() {
        let m = Meta(vec![("filetypes".into(), "hal,hl|application/x-hallo|Hallo-Datei|own; txt|text/plain|Text".into()),
                          ("schemes".into(), "hallo, web+hallo".into()), ("display".into(), "Hallo".into())]);
        let t = filetypes(&m).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t[0].own && t[0].exts == ["hal", "hl"]);
        assert_eq!(schemes(&m).unwrap(), ["hallo", "web+hallo"]);
        let d = desktop_entry(&m, "Hallo", "hallo", "hallo", None).unwrap();
        assert_eq!(civil(0), (1970, 1, 1, 4));
        assert_eq!(civil(1_793_923_200), (2026, 11, 6, 5)); // a Friday
        assert!(d.contains("Exec=hallo %U\n") && d.contains("MimeType=application/x-hallo;text/plain;x-scheme-handler/hallo;x-scheme-handler/web+hallo;\n"));
        assert_eq!(app_id(&m, "hallo-2").unwrap(), "org.example.hallo_2");
        assert!(filetypes(&Meta(vec![("filetypes".into(), "x|bad|y".into())])).is_err());
        assert!(app_id(&Meta(vec![("app_id".into(), "org.9x.y".into())]), "a").is_err());
    }

    #[test]
    fn a_browser() {
        let m = Meta(vec![("schemes".into(), "http https".into()), ("filetypes".into(), "html,htm|text/html|HTML-Dokument".into()),
                          ("categories".into(), "Network".into()), ("browser".into(), "1".into())]);
        assert!(browser(&m).unwrap());
        let d = desktop_entry(&m, "Netz", "netz", "netz", None).unwrap();
        assert!(d.contains("\nCategories=Network;WebBrowser;\n") && d.contains("MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n")
            && d.contains("Exec=netz %U\n"), "{d}");
        assert!(!browser(&Meta(vec![])).unwrap() && categories(&Meta(vec![])).unwrap() == "Utility;");
        assert!(browser(&Meta(vec![("schemes".into(), "https".into()), ("browser".into(), "1".into())])).unwrap_err().contains("http and https"));
        assert!(browser(&Meta(vec![("browser".into(), "ja".into())])).is_err());
    }
}
