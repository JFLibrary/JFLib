// Files appended to a zip (the APK aapt2 writes) without touching what is in it: its entries stay
// byte for byte (aapt2 decides how resources.arsc is stored), the new ones are stored uncompressed
// (the native library must be anyway; zipalign then aligns them).

use std::fs;
use std::path::Path;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// CRC-32 (IEEE) as zip wants it
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

/// append `files` (name in the zip, contents) to the zip at `path`, stored
pub(crate) fn append(path: &Path, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    let bad = || format!("{}: not a zip this can extend", path.display());
    let zip = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // the end of central directory record: the last "PK\5\6" with room for its 22 bytes
    let eocd = (0..=zip.len().saturating_sub(22)).rev().find(|&i| zip[i..].starts_with(b"PK\x05\x06")).ok_or_else(bad)?;
    let count = u16_at(&zip, eocd + 10).ok_or_else(bad)? as usize;
    let cd_size = u32_at(&zip, eocd + 12).ok_or_else(bad)? as usize;
    let cd_at = u32_at(&zip, eocd + 16).ok_or_else(bad)? as usize;
    let central = zip.get(cd_at..cd_at + cd_size).ok_or_else(bad)?.to_vec();
    let mut out = zip[..cd_at].to_vec();
    let mut more = Vec::new();
    for (name, data) in files {
        let at = u32::try_from(out.len()).map_err(|_| bad())?;
        let (crc, size, n) = (crc32(data), data.len() as u32, name.len() as u16);
        let header = |sig: &[u8; 4], central: bool| {
            let mut h = sig.to_vec();
            if central {
                h.extend_from_slice(&20u16.to_le_bytes()); // made by
            }
            for v in [20u16, 0x0800, 0, 0, 0x21] {
                h.extend_from_slice(&v.to_le_bytes()); // needed, UTF-8 names, stored, time, date (1980-01-01)
            }
            for v in [crc, size, size] {
                h.extend_from_slice(&v.to_le_bytes());
            }
            h.extend_from_slice(&n.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // extra
            if central {
                for v in [0u16, 0, 0] {
                    h.extend_from_slice(&v.to_le_bytes()); // comment, disk, internal attributes
                }
                h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                h.extend_from_slice(&at.to_le_bytes());
            }
            h.extend_from_slice(name.as_bytes());
            h
        };
        out.extend(header(b"PK\x03\x04", false));
        out.extend_from_slice(data);
        more.extend(header(b"PK\x01\x02", true));
    }
    let new_cd = u32::try_from(out.len()).map_err(|_| bad())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&more);
    let total = u16::try_from(count + files.len()).map_err(|_| bad())?;
    out.extend_from_slice(b"PK\x05\x06");
    for v in [0u16, 0, total, total] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&((central.len() + more.len()) as u32).to_le_bytes());
    out.extend_from_slice(&new_cd.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// the names in the zip at `path` (its central directory; ZIP64 too, as MakeAppx writes)
pub(crate) fn names(path: &Path) -> Result<Vec<String>, String> {
    let bad = || format!("{}: not a zip", path.display());
    let zip = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let eocd = (0..=zip.len().saturating_sub(22)).rev().find(|&i| zip[i..].starts_with(b"PK\x05\x06")).ok_or_else(bad)?;
    let mut count = u16_at(&zip, eocd + 10).ok_or_else(bad)? as usize;
    let mut at = u32_at(&zip, eocd + 16).ok_or_else(bad)? as usize;
    // ZIP64: a locator just before the record points at the ZIP64 end record with the real values
    if (count == 0xFFFF || at == 0xFFFF_FFFF) && eocd >= 20 && zip[eocd - 20..].starts_with(b"PK\x06\x07") {
        let rec = u64_at(&zip, eocd - 20 + 8).ok_or_else(bad)? as usize;
        if !zip.get(rec..).is_some_and(|z| z.starts_with(b"PK\x06\x06")) {
            return Err(bad());
        }
        count = u64_at(&zip, rec + 32).ok_or_else(bad)? as usize;
        at = u64_at(&zip, rec + 48).ok_or_else(bad)? as usize;
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !zip.get(at..).is_some_and(|z| z.starts_with(b"PK\x01\x02")) {
            return Err(bad());
        }
        let (n, x, c) = (u16_at(&zip, at + 28).ok_or_else(bad)? as usize, u16_at(&zip, at + 30).ok_or_else(bad)? as usize,
                         u16_at(&zip, at + 32).ok_or_else(bad)? as usize);
        out.push(String::from_utf8_lossy(zip.get(at + 46..at + 46 + n).ok_or_else(bad)?).to_string());
        at += 46 + n + x + c;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_the_check_string() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn append_to_an_empty_zip() {
        let dir = std::env::temp_dir().join(format!("cargo-jflib-zip-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.zip");
        fs::write(&p, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        append(&p, &[("x/y.txt".into(), b"hallo".to_vec())]).unwrap();
        let z = fs::read(&p).unwrap();
        assert!(z.starts_with(b"PK\x03\x04"));
        assert_eq!(u16_at(&z, z.len() - 12), Some(1)); // one entry
        assert!(z.windows(5).any(|w| w == b"hallo"));
        assert_eq!(names(&p).unwrap(), ["x/y.txt"]);
        fs::remove_dir_all(&dir).ok();
    }
}
