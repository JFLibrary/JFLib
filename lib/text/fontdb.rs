// fontdb -- system font database: enumerate the installed fonts, group them into families,
// match styles, cache the result                                      [src/text/fontdb.s]
//
//   scan(flags) -> faces found      walk the font folders (recursively, depth <= 8) and read
//        only the table directory, name, OS/2 and head of every .ttf / .otf / .ttc / .otc;
//        files already in a loaded cache with the same size are not opened again (flags
//        FDB_RESCAN: ignore the cache).  Folders: jfgui_FONTDIRS (';' separated, on Linux
//        also ':'), else Windows %WINDIR%/Fonts and %LOCALAPPDATA%/Microsoft/Windows/Fonts;
//        Linux /usr/share/fonts, /usr/local/share/fonts, $XDG_DATA_HOME/fonts (else
//        ~/.local/share/fonts), ~/.fonts (directories are visited once: symlink loops)
//   scan_dirs(dirs, flags) -> faces found   the same in these folders only (tests, apps with
//        their own font folders)
//   resolve(conf, family, lang, weight, italic) -> (face, FS_SYN_*, family)   the first installed
//        of the FontConf's candidates (generic families, substitutes, the language's choices;
//        fontconf.rs), else sans-serif's; load_resolved(text, conf, ...) -> (font id, family)
//        (a variable face: its instance for weight / italic, load_var)
//   clear()                         forget the scan result (the cache stays)
//   family_count(), family_name(i), family_faces(i) -> &[FdFace], family_range(i)   families
//        sorted case-insensitively
//   face_count(), face(i)           every face, family order
//   find(family, weight, italic) -> (face index, FS_SYN_* flags to add)   family = legacy
//        family name (FN_FAMILY, like GDI), else a typographic family; closest weight,
//        italic before upright; synthetic bold / italic asked when the family lacks them
//   load(text, family, weight, italic) -> font id / ERR_*   find + load_file (+ style; a
//        variable face: load_var); each (face, style) is loaded once until the next scan
//   variations(face) -> Option<&FdVar>   a variable face's axes and named instances (fvar,
//        names from its name table; read from the file on first use, kept until the next scan)
//   var_values(face, weight, italic, size_pt, axes) -> (values, FS_SYN_* still needed)   the
//        axis values of a choice: the axes named, else weight -> wght, italic -> ital = 1 or
//        slnt = its most slanted value, the size in points -> opsz (clamped to each axis);
//        synthetic bold / italic only where no axis gives them
//   load_var(text, face, syn, weight, italic, size_pt, axes) -> font id / ERR_*   load_face +
//        Text::vary at var_values (a face that is not variable: load_face)
//   save_cache(path) / load_cache(path) -> 0 / -1
//
// Families group faces by the legacy family name (name ID 1); names prefer German, then
// English.  Cache file (little endian): "FDB1", u32 version 2, u32 files, u32 faces, u32
// pool bytes, u64 FNV-1a hash of everything after the 28-byte header; files {u32 path, u32
// size, u32 first face, u32 faces}; faces {u32 file, u32 face index, u16 weight, u16 flags,
// u32 family, u32 style, u32 full, u32 typo family, u32 0}; pool of NUL-terminated UTF-8
// strings (offsets above point into it).  Entries are validated by path and file size.

use alloc::string::String;
use alloc::vec::Vec;

use crate::font::{FN_FAMILY, FN_FULL, FN_STYLE, FN_TYPO_FAMILY, FS_SYN_BOLD, FS_SYN_ITALIC};
use crate::fontconf::{is_generic, FontConf};
use crate::ttf::{sfnt_name, Tb, ERR_FAIL, NAME_CAP};
use crate::var::{Var, VarAxis, VarInstance};
use crate::Text;

pub const FDB_RESCAN: u32 = 1;
pub const FDFF_ITALIC: u16 = 1;
pub const FDFF_BOLD: u16 = 2;
pub const FDFF_OBLIQUE: u16 = 4;
pub const FDFF_CFF: u16 = 8;
pub const FDFF_COLOR: u16 = 16;
pub const FDFF_VAR: u16 = 32;
pub const FDFF_SYMBOL: u16 = 64;

const DEPTH_MAX: u32 = 8;
const PATH_CAP: usize = 4096;
const HDR_READ: usize = 4096;
const TTC_MAX: u32 = 64;
const NT_MAX: u32 = 1024;
const NAME_MAX: u32 = 1 << 20;
const FILES_MAX: usize = 20000;
const LOADED_MAX: usize = 256;
const CACHE_MAGIC: u32 = 0x3142_4446; // "FDB1"
const CACHE_HDR: usize = 28;
const CACHE_VERSION: u32 = 2;
const VISIT_MAX: usize = 4096;
const TAG_TTCF: u32 = 0x7474_6366;
const TAG_TRUE: u32 = 0x7472_7565;
const TAG_OTTO: u32 = 0x4F54_544F;
const TAG_V1: u32 = 0x0001_0000;
const T_NAME: u32 = 0x6E61_6D65;
const T_OS2: u32 = 0x4F53_2F32;
const T_HEAD: u32 = 0x6865_6164;
const T_GLYF: u32 = 0x676C_7966;
const T_CFF: u32 = 0x4346_4620;
const T_COLR: u32 = 0x434F_4C52;
const T_CBDT: u32 = 0x4342_4454;
const T_FVAR: u32 = 0x6676_6172;
const FVAR_MAX: u32 = 1 << 20;
static NAME_WANT: [u32; 4] = [FN_FAMILY, FN_STYLE, FN_FULL, FN_TYPO_FAMILY];

/// a face of the database (FDFACE)
#[derive(Clone, Default)]
pub struct FdFace {
    pub path: String,
    pub style: String,   // FN_STYLE
    pub full: String,    // FN_FULL
    pub family: String,  // FN_FAMILY, the grouping key
    pub tfamily: String, // FN_TYPO_FAMILY
    pub face: u32,       // face index inside the file (TTC / OTC)
    pub weight: u16,     // usWeightClass
    pub flags: u16,      // FDFF_*
}

/// the variations of an installed variable face (FontDb::variations)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FdVar {
    /// fvar's axes (names from the face's name table)
    pub axes: Vec<VarAxis>,
    /// its named instances
    pub instances: Vec<VarInstance>,
}

// file record {path, size, first face, faces}
#[derive(Clone, Copy)]
struct FileRec {
    path: u32,
    size: u32,
    first: u32,
    n: u32,
}

// internal face record (strings: pool offsets)
#[derive(Clone, Copy, Default)]
struct FaceRec {
    file: u32,
    face: u32,
    weight: u16,
    flags: u16,
    fam: u32,
    style: u32,
    full: u32,
    tfam: u32,
}

struct Family {
    first: u32,
    n: u32,
}

#[derive(Default)]
struct Cache {
    files: Vec<FileRec>,
    faces: Vec<FaceRec>,
    pool: Vec<u8>,
    hash: Vec<u64>,
    by_hash: Vec<(u64, u32)>, // (path hash, file index) sorted
}

#[derive(Default)]
pub struct FontDb {
    pool: Vec<u8>,
    files: Vec<FileRec>,
    fi: Vec<FaceRec>,
    faces: Vec<FdFace>,
    fam: Vec<Family>,
    loaded: Vec<(usize, u32, i32)>, // (face, synthetic flags, font id)
    vars: Vec<(usize, Option<FdVar>)>, // (face, its variations): read on first use
    cache: Option<Cache>,
    use_cache: bool,
    visited: Vec<(u64, u64)>,
}

// fvar (and the name table for its names) of face `face` in a file
fn read_var(path: &str, face: u32) -> Option<FdVar> {
    let f = plat::File::open(path, plat::FO_READ)?;
    let mut h = [0u8; 16];
    if read_at(&f, 0, &mut h) < 12 {
        return None;
    }
    let dir = if be32(&h, 0) == TAG_TTCF {
        let mut o = [0u8; 4];
        if read_at(&f, 12 + face as u64 * 4, &mut o) != 4 {
            return None;
        }
        be32(&o, 0) as u64
    } else {
        0
    };
    let mut d = [0u8; 12];
    if read_at(&f, dir, &mut d) != 12 {
        return None;
    }
    let nt = be16(&d, 4).min(NT_MAX);
    let mut recs = alloc::vec![0u8; (nt * 16) as usize];
    if read_at(&f, dir + 12, &mut recs) != (nt * 16) as i64 {
        return None;
    }
    let (mut fvar, mut name) = ((0u32, 0u32), (0u32, 0u32));
    for i in 0..nt as usize {
        let r = &recs[i * 16..];
        match be32(r, 0) {
            T_FVAR => fvar = (be32(r, 8), be32(r, 12)),
            T_NAME => name = (be32(r, 8), be32(r, 12)),
            _ => {}
        }
    }
    let table = |(o, l): (u32, u32), max: u32| -> Vec<u8> {
        let mut b = alloc::vec![0u8; l.min(max) as usize];
        let n = read_at(&f, o as u64, &mut b);
        b.truncate(n.max(0) as usize);
        b
    };
    let fv = table(fvar, FVAR_MAX);
    let nm = table(name, NAME_MAX);
    let v = Var::parse(Tb(&fv), Tb(&[]), Tb(&[]), Tb(&[]), Tb(&[]), Tb(&[]), Tb(&nm))?;
    Some(FdVar { axes: v.axes, instances: v.instances })
}

/// FNV-1a 64 (hash_fnv64, seed 0 = the offset basis)
pub fn fnv64(d: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

// ASCII case-insensitive compare, bytes otherwise  [ci_cmp]
fn ci_cmp(a: &[u8], b: &[u8]) -> i32 {
    let mut i = 0;
    loop {
        let mut x = a.get(i).copied().unwrap_or(0) as i32;
        let mut y = b.get(i).copied().unwrap_or(0) as i32;
        if (x - b'A' as i32) as u32 <= 25 {
            x += 32;
        }
        if (y - b'A' as i32) as u32 <= 25 {
            y += 32;
        }
        let d = x - y;
        if d != 0 || y == 0 {
            return d;
        }
        i += 1;
    }
}

// a NUL-terminated pool string
fn pstr(pool: &[u8], off: u32) -> &[u8] {
    let s = &pool[off as usize..];
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

fn pool_put(pool: &mut Vec<u8>, s: &[u8]) -> u32 {
    let off = pool.len() as u32;
    pool.extend_from_slice(s);
    pool.push(0);
    off
}

fn ends_ci(s: &str, ext: &str) -> bool {
    s.len() >= ext.len() && s.as_bytes()[s.len() - ext.len()..].eq_ignore_ascii_case(ext.as_bytes())
}

fn be16(d: &[u8], o: usize) -> u32 {
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn read_at(f: &plat::File, off: u64, buf: &mut [u8]) -> i64 {
    match f.read_at(off, buf) {
        Some(n) => n as i64,
        None => -1,
    }
}

impl FontDb {
    pub fn new() -> FontDb {
        FontDb::default()
    }

    /// fontdb_clear(): forget the scan result (the cache stays)
    pub fn clear(&mut self) {
        self.pool = Vec::new();
        self.files = Vec::new();
        self.fi = Vec::new();
        self.faces = Vec::new();
        self.fam = Vec::new();
        self.loaded.clear();
        self.vars.clear();
    }

    pub fn family_count(&self) -> usize {
        self.fam.len()
    }

    /// the family's name ("" when i is out of range)
    pub fn family_name(&self, i: usize) -> &str {
        match self.fam.get(i) {
            Some(f) => &self.faces[f.first as usize].family,
            None => "",
        }
    }

    /// the indices (face(i), variations(i)) of family i's faces (empty when out of range)
    pub fn family_range(&self, i: usize) -> core::ops::Range<usize> {
        match self.fam.get(i) {
            Some(f) => f.first as usize..(f.first + f.n) as usize,
            None => 0..0,
        }
    }

    /// the faces of family i (empty when out of range)
    pub fn family_faces(&self, i: usize) -> &[FdFace] {
        match self.fam.get(i) {
            Some(f) => &self.faces[f.first as usize..(f.first + f.n) as usize],
            None => &[],
        }
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn face(&self, i: usize) -> Option<&FdFace> {
        self.faces.get(i)
    }

    // ------------------------------------------------------------------ scanning

    /// fontdb_scan(flags) -> number of faces
    pub fn scan(&mut self, flags: u32) -> usize {
        let mut ws: Vec<(String, u32)> = Vec::new();
        Self::roots(&mut ws);
        self.scan_from(ws, flags)
    }

    /// scan these folders instead of the system's -> number of faces
    pub fn scan_dirs(&mut self, dirs: &[&str], flags: u32) -> usize {
        let ws = dirs.iter().filter(|d| !d.is_empty()).map(|d| (String::from(*d), 0)).collect();
        self.scan_from(ws, flags)
    }

    fn scan_from(&mut self, mut ws: Vec<(String, u32)>, flags: u32) -> usize {
        self.clear();
        self.visited.clear();
        self.use_cache = flags & FDB_RESCAN == 0 && self.cache.as_ref().is_some_and(|c| !c.files.is_empty());
        while let Some((path, depth)) = ws.pop() {
            if !self.visit_once(&path) {
                continue;
            }
            // list the directory
            let mut ents: Vec<(String, bool, u32)> = Vec::new();
            if !plat::dir_list(&path, &mut |name, is_dir, size| ents.push((String::from(name), is_dir, size))) {
                continue;
            }
            for (name, is_dir, size) in ents {
                // path2 = path + '/' + name
                let mut p2 = path.clone();
                if p2.len() >= PATH_CAP - 2 {
                    continue; // no room for '/' + name
                }
                if !p2.ends_with('/') && !p2.ends_with('\\') {
                    p2.push('/');
                }
                p2.push_str(&name);
                if p2.len() >= PATH_CAP - 2 {
                    continue; // too long: skip
                }
                if is_dir {
                    if depth < DEPTH_MAX {
                        ws.push((p2, depth + 1));
                    }
                    continue;
                }
                if ![".ttf", ".otf", ".ttc", ".otc"].iter().any(|e| ends_ci(&p2, e)) {
                    continue;
                }
                if self.files.len() >= FILES_MAX {
                    continue;
                }
                self.add_file(&p2, size);
            }
        }
        self.finalize();
        self.faces.len()
    }

    // first visit of a directory (Linux: device + inode; Windows: always)  [fd_visit_once]
    fn visit_once(&mut self, path: &str) -> bool {
        let Some(id) = plat::dev_ino(path) else { return true };
        if self.visited.contains(&id) {
            return false;
        }
        if self.visited.len() < VISIT_MAX {
            self.visited.push(id);
        }
        true
    }

    // push the font folders onto the work stack (plat::policy::font_dirs: the same folders
    // the app's permissions always let it read), the first one on top  [fd_roots]
    fn roots(ws: &mut Vec<(String, u32)>) {
        for d in plat::policy::font_dirs().into_iter().rev() {
            ws.push((d, 0));
        }
    }

    // add a file record and its faces (from the cache or parsed)  [fd_add_file]
    fn add_file(&mut self, path: &str, size: u32) {
        let poff = pool_put(&mut self.pool, path.as_bytes());
        let first = self.fi.len() as u32;
        self.files.push(FileRec { path: poff, size, first, n: 0 });
        let file = self.files.len() as u32 - 1;
        if !(self.use_cache && self.from_cache(path, size, file)) {
            self.parse(path, file);
        }
        // faces of this file = records added since first
        let n = self.fi.len() as u32 - first;
        self.files[file as usize].n = n;
    }

    // copy the cached faces of (path, size)  [fd_from_cache]
    fn from_cache(&mut self, path: &str, size: u32, file: u32) -> bool {
        let Some(c) = &self.cache else { return false };
        let h = fnv64(path.as_bytes());
        let start = c.by_hash.partition_point(|e| e.0 < h);
        for &(eh, i) in &c.by_hash[start..] {
            if eh != h {
                break;
            }
            let (i, f) = (i as usize, &c.files[i as usize]);
            if c.hash[i] != h || f.size != size || pstr(&c.pool, f.path) != path.as_bytes() {
                continue;
            }
            for k in f.first..f.first + f.n {
                let mut r = c.faces[k as usize];
                r.file = file;
                // strings into the new pool
                r.fam = pool_put(&mut self.pool, pstr(&c.pool, r.fam));
                r.style = pool_put(&mut self.pool, pstr(&c.pool, r.style));
                r.full = pool_put(&mut self.pool, pstr(&c.pool, r.full));
                r.tfam = pool_put(&mut self.pool, pstr(&c.pool, r.tfam));
                self.fi.push(r);
            }
            return true;
        }
        false
    }

    // open the file and add every face  [fd_parse]
    fn parse(&mut self, path: &str, file: u32) {
        let Some(f) = plat::File::open(path, plat::FO_READ) else { return };
        let mut hdr = alloc::vec![0u8; HDR_READ];
        let n = read_at(&f, 0, &mut hdr);
        if n < 12 {
            return;
        }
        match be32(&hdr, 0) {
            TAG_TTCF => {
                let nf = be32(&hdr, 8).min(TTC_MAX);
                // offsets must lie in what was read
                if (nf * 4 + 12) as i64 > n {
                    return;
                }
                for i in 0..nf {
                    let dir = be32(&hdr, (12 + i * 4) as usize);
                    self.face_rec(&f, dir, i, file);
                }
            }
            TAG_V1 | TAG_TRUE | TAG_OTTO => self.face_rec(&f, 0, 0, file),
            _ => {}
        }
    }

    // parse one face's tables and add it  [fd_face]
    fn face_rec(&mut self, f: &plat::File, dir: u32, face: u32, file: u32) {
        let (mut name, mut os2, mut head) = ((0u32, 0u32), (0u32, 0u32), (0u32, 0u32));
        let mut flags = 0u16;
        let mut outl = false;
        // table directory
        let mut d = [0u8; 12];
        if read_at(f, dir as u64, &mut d) != 12 {
            return;
        }
        let nt = be16(&d, 4).min(NT_MAX);
        let mut recs = alloc::vec![0u8; (nt * 16) as usize];
        if read_at(f, dir as u64 + 12, &mut recs) != (nt * 16) as i64 {
            return;
        }
        for i in 0..nt as usize {
            let r = &recs[i * 16..];
            let (tag, off, len) = (be32(r, 0), be32(r, 8), be32(r, 12));
            match tag {
                T_NAME => name = (off, len),
                T_OS2 => os2 = (off, len),
                T_HEAD => head = (off, len),
                T_GLYF => outl = true,
                T_CFF => {
                    flags |= FDFF_CFF;
                    outl = true;
                }
                T_COLR => flags |= FDFF_COLOR,
                T_CBDT => {
                    outl = true;
                    flags |= FDFF_COLOR;
                }
                T_FVAR => flags |= FDFF_VAR,
                _ => {}
            }
        }
        if !outl || name.1 < 6 {
            return; // no usable outlines (bitmap-only, CFF2)
        }
        // head
        let mut mac_style = 0u32;
        if head.1 >= 54 {
            let mut h = [0u8; 54];
            if read_at(f, head.0 as u64, &mut h) == 54 {
                mac_style = be16(&h, 44);
            }
        }
        // OS/2 (first 96 bytes)
        let mut o = [0u8; 96];
        let mut os2n = os2.1.min(96) as i64;
        if os2n != 0 {
            os2n = read_at(f, os2.0 as u64, &mut o[..os2n as usize]);
            if os2n < 0 {
                return;
            }
        }
        // name table
        let nl = name.1.min(NAME_MAX) as usize;
        let mut nb = alloc::vec![0u8; nl];
        let got = read_at(f, name.0 as u64, &mut nb);
        if got <= 0 {
            return;
        }
        let tb = Tb(&nb[..got as usize]);
        let mut names: [Vec<u8>; 4] = Default::default();
        for (i, &id) in NAME_WANT.iter().enumerate() {
            names[i] = sfnt_name(tb, id, NAME_CAP);
        }
        if names[0].is_empty() {
            return; // no family name
        }
        if names[3].is_empty() {
            names[3] = names[0].clone();
        }
        // weight / style flags
        let mut weight = 400u32;
        if os2n >= 64 {
            weight = be16(&o, 4);
            if weight < 10 {
                weight *= 100;
            }
            if weight == 0 {
                weight = 400;
            }
            if weight > 1000 {
                weight = 1000;
            }
            let s = be16(&o, 62);
            if s & 1 != 0 {
                flags |= FDFF_ITALIC;
            }
            if s & 0x20 != 0 {
                flags |= FDFF_BOLD;
            }
            if s & 0x200 != 0 {
                flags |= FDFF_OBLIQUE;
            }
            if os2n >= 82 && be32(&o, 78) & 0x8000_0000 != 0 {
                flags |= FDFF_SYMBOL; // ulCodePageRange1 bit 31
            }
        } else {
            if mac_style & 1 != 0 {
                flags |= FDFF_BOLD;
                weight = 700;
            }
            if mac_style & 2 != 0 {
                flags |= FDFF_ITALIC;
            }
        }
        let mut r = FaceRec { file, face, weight: weight as u16, flags, ..Default::default() };
        r.fam = pool_put(&mut self.pool, &names[0]);
        r.style = pool_put(&mut self.pool, &names[1]);
        r.full = pool_put(&mut self.pool, &names[2]);
        r.tfam = pool_put(&mut self.pool, &names[3]);
        self.fi.push(r);
    }

    // ------------------------------------------------------------------ families

    // order of internal faces: family, weight, italic, style, file, face  [fd_cmp]
    fn cmp(&self, i: usize, j: usize) -> i32 {
        let (a, b) = (&self.fi[i], &self.fi[j]);
        let p = &self.pool;
        let c = ci_cmp(pstr(p, a.fam), pstr(p, b.fam));
        if c != 0 {
            return c;
        }
        let c = a.weight as i32 - b.weight as i32;
        if c != 0 {
            return c;
        }
        let m = FDFF_ITALIC | FDFF_OBLIQUE;
        let c = (a.flags & m) as i32 - (b.flags & m) as i32;
        if c != 0 {
            return c;
        }
        let c = ci_cmp(pstr(p, a.style), pstr(p, b.style));
        if c != 0 {
            return c;
        }
        let c = a.file.wrapping_sub(b.file) as i32;
        if c != 0 {
            return c;
        }
        a.face.wrapping_sub(b.face) as i32
    }

    // duplicates: same family, full name, weight and style flags  [fd_same]
    fn same(&self, i: usize, j: usize) -> bool {
        let (a, b) = (&self.fi[i], &self.fi[j]);
        let p = &self.pool;
        a.weight == b.weight
            && a.flags == b.flags
            && ci_cmp(pstr(p, a.fam), pstr(p, b.fam)) == 0
            && ci_cmp(pstr(p, a.full), pstr(p, b.full)) == 0
    }

    // sort (shell sort gaps 701 .. 1), drop duplicates, build the faces and families
    // [fd_finalize]
    fn finalize(&mut self) {
        let n = self.fi.len();
        if n == 0 {
            return;
        }
        let mut idx: Vec<usize> = (0..n).collect();
        for gap in [701usize, 301, 132, 57, 23, 10, 4, 1] {
            for i in gap..n {
                let t = idx[i];
                let mut j = i;
                while j >= gap && self.cmp(idx[j - gap], t) > 0 {
                    idx[j] = idx[j - gap];
                    j -= gap;
                }
                idx[j] = t;
            }
        }
        let mut prev: Option<usize> = None;
        let p = |o: u32| String::from_utf8_lossy(pstr(&self.pool, o)).into_owned();
        let mut faces: Vec<FdFace> = Vec::new();
        let mut fam: Vec<Family> = Vec::new();
        for &k in &idx {
            if let Some(pk) = prev
                && self.same(pk, k)
            {
                continue;
            }
            prev = Some(k);
            let r = self.fi[k];
            let fdf = FdFace {
                path: p(self.files[r.file as usize].path),
                style: p(r.style),
                full: p(r.full),
                family: p(r.fam),
                tfamily: p(r.tfam),
                face: r.face,
                weight: r.weight,
                flags: r.flags,
            };
            // family: new when the name differs (case-insensitively) from the previous face
            match fam.last_mut() {
                Some(f) if ci_cmp(faces[f.first as usize].family.as_bytes(), fdf.family.as_bytes()) == 0 => f.n += 1,
                _ => fam.push(Family { first: faces.len() as u32, n: 1 }),
            }
            faces.push(fdf);
        }
        self.faces = faces;
        self.fam = fam;
    }

    // ------------------------------------------------------------------ matching / loading

    /// fontdb_find(family, weight, italic) -> (face index, FS_SYN_* flags to add)
    pub fn find(&self, family: &str, weight: u32, italic: bool) -> Option<(usize, u32)> {
        let w = if weight == 0 { 400 } else { weight.min(1000) as i32 };
        for pass in 0..2 {
            let mut best = i32::MAX;
            let mut bi = None;
            for (i, f) in self.faces.iter().enumerate() {
                let name = if pass == 0 { &f.family } else { &f.tfamily };
                if ci_cmp(name.as_bytes(), family.as_bytes()) != 0 {
                    continue;
                }
                // score: italic mismatch 10000, weight distance x 2, direction tie-break
                let fi = f.flags & (FDFF_ITALIC | FDFF_OBLIQUE) != 0;
                let mut s = if fi != italic { 10000 } else { 0 };
                let fw = f.weight as i32;
                s += (fw - w).abs() * 2;
                // prefer heavier for w >= 500, lighter otherwise
                if (w >= 500 && fw < w) || (w < 500 && fw > w) {
                    s += 1;
                }
                if s < best {
                    best = s;
                    bi = Some(i);
                }
            }
            if let Some(i) = bi {
                let f = &self.faces[i];
                let mut syn = 0;
                if italic && f.flags & (FDFF_ITALIC | FDFF_OBLIQUE) == 0 {
                    syn |= FS_SYN_ITALIC;
                }
                if w >= 600 && f.weight < 600 {
                    syn |= FS_SYN_BOLD;
                }
                return Some((i, syn));
            }
        }
        None
    }

    /// the first installed candidate of conf for family in lang (generic families,
    /// substitutes, the language's choices), else sans-serif's -> (face index, FS_SYN_* flags,
    /// the family name found)
    pub fn resolve(&self, conf: &FontConf, family: &str, lang: &str, weight: u32, italic: bool) -> Option<(usize, u32, String)> {
        let mut tried = conf.candidates(family, lang);
        if !is_generic(family) {
            tried.extend(conf.candidates("sans-serif", lang));
        }
        for f in tried.iter().filter(|f| !is_generic(f)) {
            if let Some((i, syn)) = self.find(f, weight, italic) {
                return Some((i, syn, self.faces[i].family.clone()));
            }
        }
        None
    }

    /// resolve and load -> (font id / ERR_*, the family found)
    pub fn load_resolved(&mut self, text: &mut Text, conf: &FontConf, family: &str, lang: &str, weight: u32, italic: bool) -> (i32, String) {
        match self.resolve(conf, family, lang, weight, italic) {
            Some((fi, syn, fam)) => (self.load_var(text, fi, syn, weight, italic, None, &[]), fam),
            None => (ERR_FAIL, String::new()),
        }
    }

    /// fontdb_load(text, family, weight, italic) -> font id / ERR_*
    pub fn load(&mut self, text: &mut Text, family: &str, weight: u32, italic: bool) -> i32 {
        let Some((fi, syn)) = self.find(family, weight, italic) else { return ERR_FAIL };
        self.load_var(text, fi, syn, weight, italic, None, &[])
    }

    /// load face fi with the synthetic styles syn (FS_SYN_*) -> font id / ERR_*; each (face,
    /// style) once until the next scan
    pub fn load_face(&mut self, text: &mut Text, fi: usize, syn: u32) -> i32 {
        if fi >= self.faces.len() {
            return ERR_FAIL;
        }
        // loaded before?
        for &(f, s, id) in &self.loaded {
            if f == fi && s == syn && text.valid(id as u32) {
                return id;
            }
        }
        let face = &self.faces[fi];
        let mut id = text.load_file(&face.path, face.face);
        if id < 0 {
            return id;
        }
        if syn != 0 {
            id = text.style(id as u32, syn);
            if id < 0 {
                return id;
            }
        }
        if self.loaded.len() < LOADED_MAX {
            self.loaded.push((fi, syn, id));
        }
        id
    }

    // ------------------------------------------------------------------ variable faces

    /// the axes and named instances of face i (None: not a variable face, or its file cannot
    /// be read); read from the file on first use
    pub fn variations(&mut self, i: usize) -> Option<&FdVar> {
        let f = self.faces.get(i)?;
        if f.flags & FDFF_VAR == 0 {
            return None;
        }
        let k = match self.vars.iter().position(|v| v.0 == i) {
            Some(k) => k,
            None => {
                let v = read_var(&f.path, f.face);
                self.vars.push((i, v));
                self.vars.len() - 1
            }
        };
        self.vars[k].1.as_ref()
    }

    /// the axis values of a choice on variable face i (see the module header) -> (one value
    /// per axis, the FS_SYN_* flags still needed); None: not a variable face
    pub fn var_values(&mut self, i: usize, weight: u32, italic: bool, size_pt: Option<f32>, axes: &[([u8; 4], f32)]) -> Option<(Vec<f32>, u32)> {
        let face_weight = self.faces.get(i)?.weight as u32;
        let face_italic = self.faces.get(i)?.flags & (FDFF_ITALIC | FDFF_OBLIQUE) != 0;
        let v = self.variations(i)?;
        let w = if weight == 0 { 400 } else { weight.min(1000) };
        let mut vals: Vec<f32> = v.axes.iter().map(|a| a.def).collect();
        let mut syn = 0;
        let find = |t: &[u8; 4]| v.axes.iter().position(|a| &a.tag == t);
        // weight: the wght axis, else a synthetic bold like find()
        match find(b"wght") {
            Some(k) => {
                let a = &v.axes[k];
                vals[k] = (w as f32).clamp(a.min, a.max);
                if w >= 600 && a.max < 600.0 {
                    syn |= FS_SYN_BOLD;
                }
            }
            None if w >= 600 && face_weight < 600 => syn |= FS_SYN_BOLD,
            None => {}
        }
        // italic: ital = 1, else the most slanted slnt, else a synthetic italic
        if italic && !face_italic {
            match (find(b"ital"), find(b"slnt")) {
                (Some(k), _) if v.axes[k].max >= 1.0 => vals[k] = 1.0,
                (_, Some(k)) if v.axes[k].min < 0.0 => vals[k] = v.axes[k].min,
                _ => syn |= FS_SYN_ITALIC,
            }
        }
        // the optical size follows the size
        if let (Some(k), Some(pt)) = (find(b"opsz"), size_pt) {
            vals[k] = pt.clamp(v.axes[k].min, v.axes[k].max);
        }
        // the axes named win
        for &(t, x) in axes {
            if let Some(k) = find(&t) {
                vals[k] = if x.is_nan() { v.axes[k].def } else { x.clamp(v.axes[k].min, v.axes[k].max) };
                if &t == b"wght" {
                    syn &= !FS_SYN_BOLD;
                }
                if &t == b"ital" || &t == b"slnt" {
                    syn &= !FS_SYN_ITALIC;
                }
            }
        }
        Some((vals, syn))
    }

    /// load variable face fi at the axis values of a choice (var_values) with the synthetic
    /// styles syn asks for that no axis gives -> font id / ERR_*; a face that is not
    /// variable: load_face(fi, syn)
    #[allow(clippy::too_many_arguments)]
    pub fn load_var(&mut self, text: &mut Text, fi: usize, syn: u32, weight: u32, italic: bool, size_pt: Option<f32>, axes: &[([u8; 4], f32)]) -> i32 {
        let Some((vals, vsyn)) = self.var_values(fi, weight, italic, size_pt, axes) else { return self.load_face(text, fi, syn) };
        // the synthetic styles the variable face still needs (find asked for them without
        // knowing the axes)
        let syn = (syn & !(FS_SYN_BOLD | FS_SYN_ITALIC)) | vsyn;
        let id = self.load_face(text, fi, 0);
        if id < 0 {
            return id;
        }
        let tags: Vec<([u8; 4], f32)> = match self.variations(fi) {
            Some(v) => v.axes.iter().zip(&vals).map(|(a, &x)| (a.tag, x)).collect(),
            None => Vec::new(),
        };
        let id = text.vary(id as u32, &tags);
        if id < 0 || syn == 0 { id } else { text.style(id as u32, syn) }
    }

    // ------------------------------------------------------------------ cache

    /// fontdb_save_cache(path) -> 0 / -1
    pub fn save_cache(&self, path: &str) -> i32 {
        let mut b = Vec::with_capacity(CACHE_HDR + self.files.len() * 16 + self.fi.len() * 32 + self.pool.len());
        b.extend_from_slice(&CACHE_MAGIC.to_le_bytes());
        b.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        b.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        b.extend_from_slice(&(self.fi.len() as u32).to_le_bytes());
        b.extend_from_slice(&(self.pool.len() as u32).to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        for f in &self.files {
            for v in [f.path, f.size, f.first, f.n] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        for r in &self.fi {
            b.extend_from_slice(&r.file.to_le_bytes());
            b.extend_from_slice(&r.face.to_le_bytes());
            b.extend_from_slice(&r.weight.to_le_bytes());
            b.extend_from_slice(&r.flags.to_le_bytes());
            for v in [r.fam, r.style, r.full, r.tfam, 0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b.extend_from_slice(&self.pool);
        let h = fnv64(&b[CACHE_HDR..]);
        b[20..28].copy_from_slice(&h.to_le_bytes());
        if plat::write_file(path, &b) { 0 } else { -1 }
    }

    /// fontdb_load_cache(path) -> 0 / -1 (a malformed file is rejected as a whole)
    pub fn load_cache(&mut self, path: &str) -> i32 {
        self.cache = None;
        let Some(d) = plat::read_file(path) else { return -1 };
        if d.len() < CACHE_HDR || le32(&d, 0) != CACHE_MAGIC || le32(&d, 4) != CACHE_VERSION {
            return -1;
        }
        if fnv64(&d[CACHE_HDR..]) != u64::from_le_bytes(d[20..28].try_into().unwrap()) {
            return -1;
        }
        let nf = le32(&d, 8) as usize;
        let nfa = le32(&d, 12) as usize;
        let np = le32(&d, 16) as usize;
        if nf > FILES_MAX || nfa > FILES_MAX * 8 || np == 0 {
            return -1;
        }
        if nf * 16 + nfa * 32 + np + CACHE_HDR > d.len() {
            return -1;
        }
        let fo = CACHE_HDR;
        let ao = fo + nf * 16;
        let po = ao + nfa * 32;
        let pool = &d[po..po + np];
        if pool[np - 1] != 0 {
            return -1;
        }
        // validate every offset
        let mut c = Cache::default();
        for i in 0..nf {
            let o = fo + i * 16;
            let f = FileRec { path: le32(&d, o), size: le32(&d, o + 4), first: le32(&d, o + 8), n: le32(&d, o + 12) };
            let (end, carry) = f.first.overflowing_add(f.n);
            if f.path as usize >= np || carry || end as usize > nfa {
                return -1;
            }
            c.files.push(f);
        }
        for i in 0..nfa {
            let o = ao + i * 32;
            let r = FaceRec {
                file: le32(&d, o),
                face: le32(&d, o + 4),
                weight: u16::from_le_bytes([d[o + 8], d[o + 9]]),
                flags: u16::from_le_bytes([d[o + 10], d[o + 11]]),
                fam: le32(&d, o + 12),
                style: le32(&d, o + 16),
                full: le32(&d, o + 20),
                tfam: le32(&d, o + 24),
            };
            if r.file as usize >= nf
                || r.fam as usize >= np
                || r.style as usize >= np
                || r.full as usize >= np
                || r.tfam as usize >= np
            {
                return -1;
            }
            c.faces.push(r);
        }
        c.pool = pool.to_vec();
        // path hashes
        c.hash = c.files.iter().map(|f| fnv64(pstr(&c.pool, f.path))).collect();
        c.by_hash = c.hash.iter().enumerate().map(|(i, &h)| (h, i as u32)).collect();
        c.by_hash.sort_unstable();
        self.cache = Some(c);
        0
    }
}
