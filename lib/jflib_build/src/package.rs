// PACKAGE: what ui::main! registers on --register (lib/plat/assoc.rs), checked as tools/pkgmeta.py
// checks it for the packages; app_package in build_rs.py writes the same text.

use std::collections::HashMap;

/// "key=value" lines, '#' comments; a later key wins (kv_file)
fn kv(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn chars(s: &str, ok: impl Fn(u8) -> bool) -> bool {
    !s.is_empty() && s.bytes().all(ok)
}

/// filetypes= -> "ext[,ext..]|mime|description[|own]" entries, ';' between them
fn filetypes(meta: &HashMap<String, String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for entry in meta.get("filetypes").map_or("", String::as_str).split(';').map(str::trim) {
        if entry.is_empty() {
            continue;
        }
        let f: Vec<&str> = entry.split('|').map(str::trim).collect();
        let exts: Vec<String> = f[0].split(',').map(|e| e.trim().trim_start_matches('.').to_lowercase()).collect();
        let mime = f.get(1).and_then(|m| m.split_once('/'));
        let mime_char = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b".+-".contains(&c);
        let ok = (3..=4).contains(&f.len())
            && (f.len() == 3 || f[3] == "own")
            && exts.iter().all(|e| e.len() <= 32 && chars(e, |c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_+.-".contains(&c)))
            && mime.is_some_and(|(a, b)| chars(a, mime_char) && chars(b, mime_char))
            && !f[2].is_empty()
            && !f[2].contains(['"', '<', '>', '&']);
        if !ok {
            return Err(format!("PACKAGE filetypes: bad entry '{entry}' (ext[,ext..]|mime/type|description[|own])"));
        }
        out.push(format!("{}|{}|{}{}", exts.join(","), f[1], f[2], if f.len() == 4 { "|own" } else { "" }));
    }
    Ok(out)
}

/// schemes= -> the URL schemes the app opens (blanks, ',' or ';' between them)
fn schemes(meta: &HashMap<String, String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in meta.get("schemes").map_or("", String::as_str).split([';', ',', ' ']).filter(|s| !s.is_empty()) {
        let s = s.to_lowercase();
        let ok = s.len() <= 32
            && s.as_bytes()[0].is_ascii_lowercase()
            && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"+.-".contains(&c));
        if !ok {
            return Err(format!("PACKAGE schemes: bad scheme '{s}'"));
        }
        out.push(s);
    }
    Ok(out)
}

/// app_id=, else org.jflib.<name>: three or more parts of letters, digits and '_', none starting
/// with a digit (the Flatpak and D-Bus rules)
fn app_id(meta: &HashMap<String, String>, name: &str) -> Result<String, String> {
    let aid = match meta.get("app_id").filter(|a| !a.is_empty()) {
        Some(a) => a.clone(),
        None => format!("org.jflib.{}", name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>()),
    };
    let parts: Vec<&str> = aid.split('.').collect();
    let part = |p: &str| {
        p.bytes().next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    };
    if parts.len() < 3 || aid.len() > 255 || !parts.iter().all(|p| part(p)) {
        return Err(format!(
            "PACKAGE app_id: '{aid}' is no reverse-DNS id (org.example.App: three or more parts, letters, digits, '_')"
        ));
    }
    Ok(aid)
}

/// PACKAGE -> the text ui::main! gets (JFGUI_PACKAGE); its version= must be Cargo's
pub(crate) fn boot_text(text: &str, name: &str, version: &str) -> Result<String, String> {
    let meta = kv(text);
    if let Some(v) = meta.get("version") {
        if v != version {
            return Err(format!("PACKAGE: version={v} is not the version {version} of Cargo.toml: delete the line, Cargo.toml's counts"));
        }
    }
    let types = filetypes(&meta)?;
    let schemes = schemes(&meta)?;
    let aid = app_id(&meta, name)?;
    let get = |k: &str, d: &str| meta.get(k).cloned().unwrap_or_else(|| d.to_string());
    Ok(format!(
        "display={}\napp_id={aid}\ncategories={}\nfiletypes={}\nschemes={}\n",
        get("display", name),
        get("categories", "Utility;"),
        types.join("; "),
        schemes.join(" ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_of_build_rs() {
        let pkg = "# x\ndisplay=Hallo Welt\nversion=1.2.3\nfiletypes=.HLO,hlx|application/x-hallo|Hallo-Datei|own; txt|text/plain|Text\nschemes=Hallo, web+x\n";
        assert_eq!(
            boot_text(pkg, "hallo-app", "1.2.3").unwrap(),
            "display=Hallo Welt\napp_id=org.jflib.hallo_app\ncategories=Utility;\nfiletypes=hlo,hlx|application/x-hallo|Hallo-Datei|own; txt|text/plain|Text\nschemes=hallo web+x\n"
        );
        assert!(boot_text("version=1.0.0", "a", "1.2.3").unwrap_err().contains("not the version 1.2.3"));
        assert!(boot_text("filetypes=x|text|y", "a", "1.0.0").unwrap_err().contains("bad entry 'x|text|y'"));
        assert!(boot_text("schemes=1x", "a", "1.0.0").unwrap_err().contains("bad scheme '1x'"));
        assert!(boot_text("app_id=org.x", "a", "1.0.0").unwrap_err().contains("no reverse-DNS id"));
        assert!(boot_text("app_id=org.example.App", "a", "1.0.0").unwrap().contains("app_id=org.example.App\n"));
    }
}
