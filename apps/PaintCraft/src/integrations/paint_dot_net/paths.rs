//! Wine path interchange for the standard Z: root and C: prefix mappings.
extern crate alloc;
use alloc::{format, string::String};

pub(crate) fn to_windows(path: &str, cwd: &str) -> Result<String, &'static str> {
    if path.is_empty() || path.contains('\0') {
        return Err("Leerer oder ungültiger Plugin-Pfad.");
    }
    if path.as_bytes().get(1) == Some(&b':') {
        if !path.as_bytes()[0].is_ascii_alphabetic()
            || !matches!(path.as_bytes().get(2), Some(b'/' | b'\\'))
        {
            return Err("Wine benötigt einen absoluten Windows-Pfad.");
        }
        return Ok(path.replace('/', "\\"));
    }
    // Backslash is legal in a Unix name but cannot be represented by this mapping.
    if path.contains('\\') {
        return Err("Ein Linux-Pfad darf für Wine keinen Rückstrich enthalten.");
    }
    let absolute = if path.starts_with('/') {
        String::from(path)
    } else {
        format!("{cwd}/{path}")
    };
    if !absolute.starts_with('/') {
        return Err("Wine benötigt ein absolutes Arbeitsverzeichnis.");
    }
    Ok(format!("Z:{}", absolute.replace('/', "\\")))
}

pub(crate) fn to_native(path: &str, prefix: &str) -> Result<String, &'static str> {
    if path.contains('\0') || path.is_empty() {
        return Err("Ungültiger Wine-Katalogpfad.");
    }
    if path.starts_with('/') && !path.contains('\\') {
        return Ok(String::from(path));
    }
    if path.len() < 3 || path.as_bytes()[1] != b':' || !matches!(path.as_bytes()[2], b'\\' | b'/') {
        return Err("Der Plugin-Host lieferte keinen absoluten Dateipfad.");
    }
    let rest = path[2..].replace('\\', "/");
    match path.as_bytes()[0].to_ascii_uppercase() {
        b'Z' => Ok(rest),
        b'C' if prefix.starts_with('/') => {
            Ok(format!("{}/drive_c{rest}", prefix.trim_end_matches('/')))
        }
        _ => Err("Die Wine-Brücke unterstützt Katalogpfade auf C: und Z:."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_spaces_and_relative_paths() {
        assert_eq!(
            to_windows("Plugins/Öl & Farbe.dll", "/home/a b").unwrap(),
            "Z:\\home\\a b\\Plugins\\Öl & Farbe.dll"
        );
        assert_eq!(
            to_windows("C:/plugins/a.dll", "/").unwrap(),
            "C:\\plugins\\a.dll"
        );
        assert_eq!(
            to_native("Z:\\tmp\\Öl & Farbe.dll", "/prefix").unwrap(),
            "/tmp/Öl & Farbe.dll"
        );
        assert_eq!(
            to_native("c:\\Program Files\\paint.net", "/my prefix/").unwrap(),
            "/my prefix/drive_c/Program Files/paint.net"
        );
    }
    #[test]
    fn ambiguous_paths_are_rejected() {
        for s in ["", "C:relative", "a\0b", "dir\\file.dll"] {
            assert!(to_windows(s, "/home").is_err());
        }
        for s in [
            "",
            "D:\\plugin.dll",
            "relative",
            "\\\\server\\file",
            "Z:x",
            "Z:\\a\0",
        ] {
            assert!(to_native(s, "/prefix").is_err());
        }
    }
}
