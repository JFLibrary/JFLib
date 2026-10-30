// appimage -- an AppImage file as an update: the type-2 header, its embedded signature (the
// sections .sha256_sig and .sig_key appimagetool --sign writes) and whether a new file may
// follow the running one
//
//   is_type2(path) -> bool             ELF with "AI\x02" at byte 8
//   signature(path) -> Result<Signed, String>   No (no or empty sections), By(the signing key
//                                      of the embedded key block, its fingerprints), Err: a
//                                      signature that does not check
//   follows(old, new) -> Result<(), String>   the rule of an update: a signed AppImage only
//                                      takes one signed by a key of its own key block
//
// The signed text is the SHA-256 of the whole file, with the bytes of both sections taken as
// zeros, as 64 lower-case hex digits (appimagetool's digest; AppImageUpdate's validate checks
// the same).  The armored signature and key stand NUL-padded in their sections.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::pgp;

/// what an AppImage's signature sections say
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signed {
    /// not signed (no sections, or empty ones)
    No,
    /// signed by `signer` (a key of the embedded block, whose fingerprints are `keys`)
    By { signer: [u8; 20], keys: Vec<[u8; 20]> },
}

const SIG_SECTION: &str = ".sha256_sig";
const KEY_SECTION: &str = ".sig_key";
const CHUNK: usize = 1 << 16;

fn read_at(f: &plat::File, off: u64, n: usize) -> Option<Vec<u8>> {
    let mut b = alloc::vec![0u8; n];
    let mut got = 0;
    while got < n {
        match f.read_at(off + got as u64, &mut b[got..]) {
            Some(k) if k > 0 => got += k,
            _ => return None,
        }
    }
    Some(b)
}

/// ELF64 with the AppImage type-2 magic
pub fn is_type2(path: &str) -> bool {
    let Some(f) = plat::File::open(path, plat::FO_READ) else { return false };
    read_at(&f, 0, 16).is_some_and(|h| &h[..4] == b"\x7fELF" && h[4] == 2 && &h[8..11] == b"AI\x02")
}

/// the sections of an ELF64 little-endian file: (name, file offset, size)
fn sections(f: &plat::File) -> Result<Vec<(String, u64, u64)>, String> {
    let bad = || String::from("kein gültiger ELF-Kopf");
    let h = read_at(f, 0, 64).ok_or_else(bad)?;
    if &h[..4] != b"\x7fELF" || h[4] != 2 || h[5] != 1 {
        return Err(bad());
    }
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
    let u64_at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]));
    let (shoff, shsize, shnum, shstrndx) = (u64_at(&h, 0x28), u16_at(&h, 0x3A), u16_at(&h, 0x3C), u16_at(&h, 0x3E));
    if shsize < 64 || shnum == 0 || shnum > 4096 || shstrndx >= shnum {
        return Err(bad());
    }
    let table = read_at(f, shoff, (shsize * shnum) as usize).ok_or_else(bad)?;
    let head = |i: u64| &table[(i * shsize) as usize..(i * shsize) as usize + 64];
    let strs = head(shstrndx);
    let (soff, ssize) = (u64_at(strs, 24), u64_at(strs, 32));
    if ssize > 1 << 20 {
        return Err(bad());
    }
    let names = read_at(f, soff, ssize as usize).ok_or_else(bad)?;
    let mut out = Vec::new();
    for i in 0..shnum {
        let e = head(i);
        let at = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as usize;
        let name = names.get(at..).map(|n| &n[..n.iter().position(|&c| c == 0).unwrap_or(n.len())]).unwrap_or(&[]);
        out.push((String::from_utf8_lossy(name).into_owned(), u64_at(e, 24), u64_at(e, 32)));
    }
    Ok(out)
}

/// SHA-256 of the file with the given ranges read as zeros, as lower-case hex
fn digest(f: &plat::File, size: u64, zero: &[(u64, u64)]) -> Option<String> {
    let mut h = crypto::Sha256::new();
    let mut buf = alloc::vec![0u8; CHUNK];
    let mut off = 0u64;
    while off < size {
        let n = (size - off).min(CHUNK as u64) as usize;
        let got = f.read_at(off, &mut buf[..n])?;
        if got == 0 {
            return None;
        }
        for &(zo, zs) in zero {
            let (a, b) = (zo.max(off), (zo + zs).min(off + got as u64));
            if a < b {
                buf[(a - off) as usize..(b - off) as usize].fill(0);
            }
        }
        h.update(&buf[..got]);
        off += got as u64;
    }
    Some(crypto::hex(&h.finalize()))
}

// a section's text: up to the first NUL
fn section_text(f: &plat::File, off: u64, size: u64) -> Option<String> {
    let b = read_at(f, off, size.min(1 << 16) as usize)?;
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    Some(String::from_utf8_lossy(&b[..end]).trim().into())
}

/// the embedded signature of the AppImage at path
pub fn signature(path: &str) -> Result<Signed, String> {
    let f = plat::File::open(path, plat::FO_READ).ok_or_else(|| format!("{} kann nicht gelesen werden", path))?;
    let size = f.size().ok_or_else(|| format!("{} kann nicht gelesen werden", path))?;
    let secs = sections(&f)?;
    let find = |n: &str| secs.iter().find(|s| s.0 == n).map(|s| (s.1, s.2)).filter(|&(o, s)| o.checked_add(s).is_some_and(|e| e <= size));
    let (Some(sig), Some(key)) = (find(SIG_SECTION), find(KEY_SECTION)) else { return Ok(Signed::No) };
    let sig_text = section_text(&f, sig.0, sig.1).ok_or("die Signatur kann nicht gelesen werden")?;
    let key_text = section_text(&f, key.0, key.1).ok_or("der Schlüssel kann nicht gelesen werden")?;
    if sig_text.is_empty() {
        return Ok(Signed::No);
    }
    if key_text.is_empty() {
        return Err(String::from("signiert, aber ohne Schlüssel (.sig_key leer)"));
    }
    let keys = pgp::keys(&key_text)?;
    let d = digest(&f, size, &[sig, key]).ok_or("die Datei kann nicht gelesen werden")?;
    let signer = pgp::verify(&keys, &sig_text, d.as_bytes())?;
    Ok(Signed::By { signer: signer.fingerprint, keys: keys.iter().map(|k| k.fingerprint).collect() })
}

fn hex20(f: &[u8; 20]) -> String {
    f.iter().map(|b| format!("{:02X}", b)).collect()
}

/// whether the AppImage `new` may replace `old` (the running one): new is a type-2 AppImage,
/// its signature (if any) checks, and when old is signed, new is signed by a key of old's
/// key block -> Ok(new's signer fingerprint as hex, "" unsigned) or why not
pub fn follows(old: &str, new: &str) -> Result<String, String> {
    if !is_type2(new) {
        return Err(String::from("die neue Datei ist kein AppImage (Typ 2)"));
    }
    let n = signature(new).map_err(|e| format!("die Signatur des neuen AppImage stimmt nicht: {}", e))?;
    // a running file without a signature that checks: the signed manifest alone decides
    let o = signature(old).unwrap_or(Signed::No);
    match (o, n) {
        (Signed::By { keys, .. }, Signed::By { signer, .. }) if !keys.contains(&signer) => Err(format!(
            "das neue AppImage ist mit einem anderen Schlüssel signiert ({}) als das laufende",
            hex20(&signer)
        )),
        (Signed::By { .. }, Signed::No) => Err(String::from("das laufende AppImage ist signiert, das neue nicht")),
        (_, Signed::By { signer, .. }) => Ok(hex20(&signer)),
        (_, Signed::No) => Ok(String::new()),
    }
}
