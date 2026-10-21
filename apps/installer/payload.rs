// payload -- the setup's payload: reads its own exe, finds the trailer, validates the entry
// table and loads manifest, styles and previews (layout: apps/installer/inc/inst.inc, writer:
// tools/package.py)                                            [apps/installer/src/payload.s]
//
//   pl_load(st) -> Ok / Err (st.err holds the reason)
//   pl_inflate(st, entry) -> Some(PE_USIZE bytes) or None
//   kv_get(text, key, cap) -> Some(value) -- value of a "key=value" line (None if missing)
//   sty_find(st, name) -> style index or None
//   signature(bytes) / stub(st) -- a signed setup (Authenticode): the certificate table follows
//                     the trailer; the uninstaller is the stub without the setup's signature

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;

use crate::{Inst, PATH_CAP, Sty, str_cap};

const MAGIC_TRAILER: &[u8; 8] = b"ASMINST1";
const MAGIC_PAYLOAD: &[u8; 4] = b"AIPK";

// entry record (PE_*): 128 bytes
const PE_SIZE: usize = 128;
const PE_NAME: usize = 32;
const PE_LABEL: usize = 80;
pub(crate) const PK_MANIFEST: u32 = 1;
pub(crate) const PK_STYLE: u32 = 2;
pub(crate) const PK_PREVIEW: u32 = 3;
pub(crate) const PK_FILE: u32 = 5;
const PF_ZLIB: u32 = 1;
const PAY_MAX_ENT: u32 = 256;
const PAY_MAX_USIZE: u32 = 0x1000_0000;
pub(crate) const STY_MAX: usize = 8;

/// one PE record of the entry table
pub(crate) struct Entry {
    pub(crate) kind: u32,
    pub(crate) csize: u32,
    pub(crate) usize_: u32,
    pub(crate) flags: u32,
    pub(crate) off: u64,
    /// PE_NAME (at most 47 bytes)
    pub(crate) name: String,
    /// PE_LABEL (at most 47 bytes)
    pub(crate) label: String,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u32_at(b, o) as u64 | (u32_at(b, o + 4) as u64) << 32
}

/// a NUL-terminated char[48] field (byte 47 forced to NUL, as the asm does)
fn cstr48(b: &[u8]) -> String {
    let f = &b[..47];
    let n = f.iter().position(|&c| c == 0).unwrap_or(47);
    String::from_utf8_lossy(&f[..n]).into_owned()
}

const ERR_NOREAD: &str = "Die Setup-Datei konnte nicht gelesen werden.";
const ERR_BAD: &str = "Das Setup enthält keine gültigen Installationsdaten (beschädigt oder unvollständig).";

/// the payload of the own exe -> Ok, or Err with st.err set  [pl_load]
pub(crate) fn pl_load(st: &mut Inst) -> Result<(), ()> {
    let Some(path) = plat::inst::exe_path() else {
        st.err = String::from(ERR_NOREAD);
        return Err(());
    };
    st.selfpath = path;
    let Some(data) = plat::read_file(&st.selfpath) else {
        st.err = String::from(ERR_NOREAD);
        return Err(());
    };
    st.self_ = data;
    if pl_parse(st).is_err() {
        st.err = String::from(ERR_BAD);
        return Err(());
    }
    Ok(())
}

/// the PE security directory of an Authenticode-signed exe: (file offset of the certificate
/// table, offset of the directory entry); None when unsigned.  signtool puts the table at the
/// end of the file, after the setup's payload and trailer (padded to 8 bytes).
pub(crate) fn signature(b: &[u8]) -> Option<(usize, usize)> {
    if b.len() < 0x40 || &b[..2] != b"MZ" {
        return None;
    }
    let pe = u32_at(b, 0x3c) as usize;
    if pe.checked_add(24 + 112 + 8 * 5)? > b.len() || &b[pe..pe + 4] != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    let dirs = match u16::from_le_bytes([b[opt], b[opt + 1]]) {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return None,
    };
    if u32_at(b, dirs - 4) < 5 {
        return None;
    }
    let e = dirs + 4 * 8; // IMAGE_DIRECTORY_ENTRY_SECURITY
    let (off, size) = (u32_at(b, e) as usize, u32_at(b, e + 4) as usize);
    (off != 0 && size != 0 && off.checked_add(size)? == b.len()).then_some((off, e))
}

/// the setup without its payload and without its signature (that one covers the whole file):
/// the uninstaller
pub(crate) fn stub(st: &Inst) -> Vec<u8> {
    let mut b = st.self_[..st.payoff].to_vec();
    if let Some((_, e)) = signature(&st.self_) {
        if e + 8 <= b.len() {
            b[e..e + 8].fill(0);
        }
    }
    b
}

// trailer, header, entry table, kinds, manifest
fn pl_parse(st: &mut Inst) -> Result<(), ()> {
    // the trailer ends the file, or ends before the certificate table of a signed setup
    // (followed by up to 7 bytes of padding there)
    let end = signature(&st.self_).map_or(st.self_.len(), |(off, _)| off) as u64;
    if end < 24 + 8 {
        return Err(());
    }
    let mut found = None;
    for pad in 0..8u64 {
        let Some(t) = end.checked_sub(24 + pad).map(|t| t as usize) else { break };
        if &st.self_[t..t + 8] != MAGIC_TRAILER {
            continue;
        }
        let (off, size) = (u64_at(&st.self_, t + 8), u64_at(&st.self_, t + 16));
        if off < end && size < end && off.wrapping_add(size).wrapping_add(24) == end - pad {
            found = Some((off, size));
            break;
        }
    }
    let Some((off, size)) = found else { return Err(()) };
    st.payoff = off as usize;
    st.paysz = size as usize;
    // header
    if size < 8 {
        return Err(());
    }
    let pay = &st.self_[st.payoff..st.payoff + st.paysz];
    if &pay[..4] != MAGIC_PAYLOAD {
        return Err(());
    }
    let count = u32_at(pay, 4);
    if count > PAY_MAX_ENT {
        return Err(());
    }
    if count as u64 * PE_SIZE as u64 + 8 > size {
        return Err(());
    }
    // every entry inside the payload, names terminated
    let mut ents = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let r = &pay[8 + i * PE_SIZE..8 + (i + 1) * PE_SIZE];
        let e = Entry {
            kind: u32_at(r, 0),
            csize: u32_at(r, 4),
            usize_: u32_at(r, 8),
            flags: u32_at(r, 12),
            off: u64_at(r, 16),
            name: cstr48(&r[PE_NAME..]),
            label: cstr48(&r[PE_LABEL..]),
        };
        if e.off > size || e.csize as u64 > size - e.off || e.usize_ > PAY_MAX_USIZE {
            return Err(());
        }
        ents.push(e);
    }
    st.ents = ents;
    // kinds
    for i in 0..st.ents.len() {
        match st.ents[i].kind {
            PK_MANIFEST => {
                if st.man.is_none() {
                    st.man = Some(pl_inflate(st, i).ok_or(())?);
                }
            }
            PK_STYLE => {
                if st.sty.len() < STY_MAX {
                    st.sty.push(Sty { ent: i, sf: None, desc: String::new() });
                }
            }
            PK_PREVIEW => {
                let Some(k) = sty_find(st, &st.ents[i].name) else { continue };
                if k >= STY_MAX || st.sty[k].sf.is_some() {
                    continue;
                }
                let Some(png) = pl_inflate(st, i) else { continue };
                let mut sf = Surface::default();
                if img_png::dec::decode(&png, &mut sf) == 0 {
                    st.sty[k].sf = Some(Rc::new(sf));
                }
            }
            _ => {}
        }
    }
    // manifest
    if st.sty.is_empty() {
        return Err(());
    }
    let man = st.man.take().ok_or(())?;
    st.app = kv_get(&man, "app", 64).ok_or(())?;
    st.disp = kv_get(&man, "display", 128).unwrap_or_else(|| str_cap(&st.app, 128));
    st.ver = kv_get(&man, "version", 32).unwrap_or_default();
    st.pub_ = kv_get(&man, "publisher", 96).unwrap_or_default();
    st.defsty = kv_get(&man, "default", 48).unwrap_or_default();
    st.types = kv_get(&man, "filetypes", 4096).unwrap_or_default();
    st.schemes = kv_get(&man, "schemes", 512).unwrap_or_default();
    // descriptions: desc_<style id>
    for k in 0..st.sty.len() {
        let key = str_cap(&(String::from("desc_") + &st.ents[st.sty[k].ent].name), 64);
        st.sty[k].desc = kv_get(&man, &key, 192).unwrap_or_default();
    }
    st.man = Some(man);
    Ok(())
}

/// style index of a style id or None  [sty_find]
pub(crate) fn sty_find(st: &Inst, name: &str) -> Option<usize> {
    st.sty.iter().take(STY_MAX).position(|s| st.ents[s.ent].name == name)
}

/// the data of entry i (inflated when PF_ZLIB) -> PE_USIZE bytes or None  [pl_inflate]
pub(crate) fn pl_inflate(st: &Inst, i: usize) -> Option<Vec<u8>> {
    let e = &st.ents[i];
    let n = e.usize_ as usize;
    let at = st.payoff + e.off as usize;
    let src = &st.self_[at..at + e.csize as usize];
    if e.flags & PF_ZLIB != 0 {
        let mut dst = alloc::vec![0u8; n];
        match img_core::inflate::zlib_inflate(src, &mut dst) {
            Ok(got) if got == n => Some(dst),
            _ => None,
        }
    } else if e.csize as usize == n {
        Some(src.to_vec())
    } else {
        None
    }
}

/// the value of the first "key=value" line (up to the line end, cut to cap - 1 bytes); None
/// when the key is missing  [kv_get]
pub(crate) fn kv_get(text: &[u8], key: &str, cap: usize) -> Option<String> {
    let k = key.as_bytes();
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    let text = &text[..end];
    let mut p = 0;
    while p < text.len() {
        let line = &text[p..];
        if line.len() > k.len() && &line[..k.len()] == k && line[k.len()] == b'=' {
            let v = &line[k.len() + 1..];
            let n = v.iter().position(|&c| c == b'\n' || c == b'\r').unwrap_or(v.len());
            return Some(str_cap(&String::from_utf8_lossy(&v[..n]), cap));
        }
        // the next line
        match line.iter().position(|&c| c == b'\n') {
            Some(q) => p += q + 1,
            None => break,
        }
    }
    None
}

/// the folder part of a path (up to the last '\'), None without one  [un_load]
pub(crate) fn parent_dir(path: &str) -> Option<String> {
    path.rfind('\\').map(|i| String::from(&path[..i]))
}

/// a path of at most PATH_CAP - 1 bytes
pub(crate) fn path_cap(s: &str) -> String {
    str_cap(s, PATH_CAP)
}
