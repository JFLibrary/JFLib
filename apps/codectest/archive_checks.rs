// archive_checks -- codectest --archive: lib/archive's ZIP reader and writer and archive::opc
// against archives and packages built here (a few bytes to a few MB): hostile ones must be
// refused with the right error, quickly and without allocating what they claim; controls of the
// same shape must still read; everything written must read back as it was.  Exit code 1 when a
// check fails.
//
//   zip_rt       round trips: stored (byte for byte the layout of the 0.2 writer), DEFLATE at
//                every level, empty archives, ZipFs, data descriptors
//   zip_names    names: `..`, absolute, empty segments, `\`, `:`, `%`, control characters,
//                1025 bytes, duplicates (exact, ASCII case, Unicode case), non-UTF-8, a local
//                name other than the central one; a name 512 levels deep still reads
//   zip_dir      broken directories: truncation, end record, offsets, counts, disks, gaps,
//                overlapping records (a "quoted" overlap bomb), CRC, encryption, methods,
//                versions, local fields that disagree, bad DEFLATE data
//   zip_limits   Limits: entries, part and total size, archive size, the ratio of a DEFLATE part,
//                sizes no DEFLATE stream reaches, a 16 MiB bomb, a count far beyond the archive
//   zip64        ZIP64: written only when needed (65535 parts, sizes and offsets from a lowered
//                threshold), read from foreign layouts (.NET, Java, data descriptors), and
//                hostile ZIP64 records
//   opc          packages: round trip, unknown parts kept byte for byte, name equivalence and
//                percent-escapes, the exact profile, edits, hostile relationships and content
//                types, macros, XML limits, 20000 parts
//
// --only NAME runs the groups whose names start with NAME.  (build_rs.py test codectest runs it
// through tests/archive.uis; --script is ignored)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::opc::{self, Package, Relationship, Rules};
use archive::{Entry, Error, Limits, Method};
use img_core::checksum::crc32;
use img_core::deflate::deflate_raw;

/// a refusal must come within this many microseconds
const QUICK_US: u64 = 1_000_000;
const M32: u32 = 0xFFFF_FFFF;

struct H {
    ok: u32,
    failed: u32,
    only: Option<String>,
}

impl H {
    fn on(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|o| name.starts_with(o.as_str()))
    }
    fn check(&mut self, name: &str, cond: bool, what: String) {
        if cond {
            self.ok += 1;
            plat::write_stdout(format!("ok   {name}: {what}\n").as_bytes());
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {name}: {what}\n").as_bytes());
        }
    }
    /// f() must give `want`, within QUICK_US
    fn quick<T: PartialEq + core::fmt::Debug>(&mut self, name: &str, want: T, f: impl FnOnce() -> T) {
        let t0 = plat::time_us();
        let r = f();
        let us = plat::time_us().wrapping_sub(t0);
        let what = format!("{r:?} (want {want:?}) in {} ms", us / 1000);
        self.check(name, r == want && us < QUICK_US, what);
    }
}

// ------------------------------------------------------------------ helpers

fn entry(name: &str, data: &[u8]) -> Entry {
    Entry { name: String::from(name), data: data.to_vec() }
}
fn read_err(z: &[u8], limits: Limits) -> Result<usize, Error> {
    archive::read(z, limits).map(|e| e.len())
}
fn big() -> Limits {
    Limits { entries: 200_000, part_bytes: 1 << 30, total_bytes: 1 << 30, archive_bytes: 1 << 30, ..Limits::default() }
}
/// a deterministic byte stream that DEFLATE cannot shrink
fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 11) as u8
        })
        .collect()
}
/// text that DEFLATE shrinks well
fn text(n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    let mut i = 0u32;
    while v.len() < n {
        v.extend_from_slice(format!("<Cell N=\"Width\" V=\"{}\" U=\"MM\"/>\n", i % 977).as_bytes());
        i += 1;
    }
    v.truncate(n);
    v
}
fn p16(z: &mut Vec<u8>, v: u16) {
    z.extend_from_slice(&v.to_le_bytes());
}
fn p32(z: &mut Vec<u8>, v: u32) {
    z.extend_from_slice(&v.to_le_bytes());
}
fn p64(z: &mut Vec<u8>, v: u64) {
    z.extend_from_slice(&v.to_le_bytes());
}
fn set16(z: &mut [u8], p: usize, v: u16) {
    z[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn set32(z: &mut [u8], p: usize, v: u32) {
    z[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn set64(z: &mut [u8], p: usize, v: u64) {
    z[p..p + 8].copy_from_slice(&v.to_le_bytes());
}
fn at16(z: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([z[p], z[p + 1]])
}
fn at32(z: &[u8], p: usize) -> u32 {
    u32::from_le_bytes([z[p], z[p + 1], z[p + 2], z[p + 3]])
}
fn contains(z: &[u8], sig: u32) -> bool {
    z.windows(4).any(|w| w == sig.to_le_bytes())
}

// ------------------------------------------------------------------ a ZIP builder for crafted archives

#[derive(Clone, Copy, PartialEq)]
enum Desc {
    No,
    Narrow,
    NarrowSig,
    Wide,
}
/// one record as it is to sit in the archive
#[derive(Clone)]
struct Rec {
    name: Vec<u8>,
    body: Vec<u8>, // the payload as stored
    size: u64,
    crc: u32,
    method: u16,
    flags: u16,
    version: u16,
    desc: Desc,
    local64: bool,      // local header: sizes as marks and a ZIP64 field
    local64_real: bool, // local header: real sizes and a ZIP64 field anyway (.NET)
    cd64: bool,         // central record: sizes and offset as marks and a ZIP64 field
    gap: usize,         // junk before the local header
}
fn rec(name: &str, data: &[u8], deflate: bool) -> Rec {
    let body = if deflate { deflate_raw(data, 6) } else { data.to_vec() };
    Rec {
        name: name.as_bytes().to_vec(),
        body,
        size: data.len() as u64,
        crc: crc32(0, data),
        method: if deflate { 8 } else { 0 },
        flags: 0x800,
        version: 20,
        desc: Desc::No,
        local64: false,
        local64_real: false,
        cd64: false,
        gap: 0,
    }
}
#[derive(Clone, Copy, PartialEq)]
enum End {
    Plain,
    Z64Marks, // ZIP64 end records, the classic one all marks (Java with 65535+ entries)
    Z64Real,  // ZIP64 end records, the classic one with the real values (always-ZIP64 writers)
}
struct Built {
    z: Vec<u8>,
    locals: Vec<usize>,
    cds: Vec<usize>,
    cd: usize,
    end: usize, // the classic end record
}
fn build(recs: &[Rec], end: End, junk_before_cd: usize, comment: &[u8]) -> Built {
    let mut z = Vec::new();
    let mut locals = Vec::new();
    for r in recs {
        z.resize(z.len() + r.gap, 0xAA);
        locals.push(z.len());
        let packed = r.body.len() as u64;
        let d = r.desc != Desc::No;
        let field = r.local64 || r.local64_real;
        p32(&mut z, 0x04034b50);
        p16(&mut z, r.version);
        p16(&mut z, r.flags);
        p16(&mut z, r.method);
        p16(&mut z, 0);
        p16(&mut z, 33);
        p32(&mut z, if d { 0 } else { r.crc });
        p32(&mut z, if r.local64 { M32 } else if d { 0 } else { packed as u32 });
        p32(&mut z, if r.local64 { M32 } else if d { 0 } else { r.size as u32 });
        p16(&mut z, r.name.len() as u16);
        p16(&mut z, if field { 20 } else { 0 });
        z.extend_from_slice(&r.name);
        if field {
            p16(&mut z, 1);
            p16(&mut z, 16);
            p64(&mut z, if d { 0 } else { r.size });
            p64(&mut z, if d { 0 } else { packed });
        }
        z.extend_from_slice(&r.body);
        match r.desc {
            Desc::No => {}
            Desc::Narrow | Desc::NarrowSig => {
                if r.desc == Desc::NarrowSig {
                    p32(&mut z, 0x08074b50);
                }
                p32(&mut z, r.crc);
                p32(&mut z, packed as u32);
                p32(&mut z, r.size as u32);
            }
            Desc::Wide => {
                p32(&mut z, 0x08074b50);
                p32(&mut z, r.crc);
                p64(&mut z, packed);
                p64(&mut z, r.size);
            }
        }
    }
    z.resize(z.len() + junk_before_cd, 0xBB);
    let cd = z.len();
    let mut cds = Vec::new();
    for (i, r) in recs.iter().enumerate() {
        cds.push(z.len());
        let packed = r.body.len() as u64;
        p32(&mut z, 0x02014b50);
        p16(&mut z, if r.cd64 { 45 } else { 20 });
        p16(&mut z, r.version);
        p16(&mut z, r.flags);
        p16(&mut z, r.method);
        p16(&mut z, 0);
        p16(&mut z, 33);
        p32(&mut z, r.crc);
        p32(&mut z, if r.cd64 { M32 } else { packed as u32 });
        p32(&mut z, if r.cd64 { M32 } else { r.size as u32 });
        p16(&mut z, r.name.len() as u16);
        p16(&mut z, if r.cd64 { 28 } else { 0 });
        p16(&mut z, 0);
        p16(&mut z, 0);
        p16(&mut z, 0);
        p32(&mut z, 0);
        p32(&mut z, if r.cd64 { M32 } else { locals[i] as u32 });
        z.extend_from_slice(&r.name);
        if r.cd64 {
            p16(&mut z, 1);
            p16(&mut z, 24);
            p64(&mut z, r.size);
            p64(&mut z, packed);
            p64(&mut z, locals[i] as u64);
        }
    }
    let cdsize = z.len() - cd;
    let n = recs.len();
    if end != End::Plain {
        let rec64 = z.len();
        p32(&mut z, 0x06064b50);
        p64(&mut z, 44);
        p16(&mut z, 45);
        p16(&mut z, 45);
        p32(&mut z, 0);
        p32(&mut z, 0);
        p64(&mut z, n as u64);
        p64(&mut z, n as u64);
        p64(&mut z, cdsize as u64);
        p64(&mut z, cd as u64);
        p32(&mut z, 0x07064b50);
        p32(&mut z, 0);
        p64(&mut z, rec64 as u64);
        p32(&mut z, 1);
    }
    let marks = end == End::Z64Marks;
    let eocd = z.len();
    p32(&mut z, 0x06054b50);
    p16(&mut z, 0);
    p16(&mut z, 0);
    p16(&mut z, if marks { 0xFFFF } else { n as u16 });
    p16(&mut z, if marks { 0xFFFF } else { n as u16 });
    p32(&mut z, if marks { M32 } else { cdsize as u32 });
    p32(&mut z, if marks { M32 } else { cd as u32 });
    p16(&mut z, comment.len() as u16);
    z.extend_from_slice(comment);
    Built { z, locals, cds, cd, end: eocd }
}
fn plain(recs: &[Rec]) -> Built {
    build(recs, End::Plain, 0, b"")
}
/// the stored layout of the 0.2 writer, built independently: `write` must keep it byte for byte
fn layout_0_2(entries: &[Entry]) -> Vec<u8> {
    let recs: Vec<Rec> = entries.iter().map(|e| rec(&e.name, &e.data, false)).collect();
    plain(&recs).z
}

// ------------------------------------------------------------------ zip_rt

fn zip_rt(t: &mut H) {
    let sample = vec![
        entry("doc/readme.txt", b"zip text"),
        entry("dir/\u{e4}rger.xml", &text(5000)),
        entry("empty", b""),
        entry("bin/noise.bin", &noise(3000, 7)),
    ];
    let a = archive::write(&sample, Limits::default());
    let b = archive::write(&sample, Limits::default());
    t.check("zip_rt/stored", a.as_ref().ok().and_then(|z| archive::read(z, Limits::default()).ok()) == Some(sample.clone()), String::from("write -> read gives the parts back"));
    t.check("zip_rt/stored_same_bytes", a.is_ok() && a == b && a.as_deref().ok() == Some(&layout_0_2(&sample)[..]), String::from("deterministic, the 0.2 layout byte for byte"));

    let parts = vec![entry("text.xml", &text(200_000)), entry("noise.bin", &noise(4096, 3)), entry("empty.txt", b""), entry("one", b"x")];
    let z = archive::write_with(&parts, Limits::default(), Method::Deflate(6)).unwrap_or_default();
    let z2 = archive::write_with(&parts, Limits::default(), Method::Deflate(6)).unwrap_or_default();
    let back = archive::read(&z, Limits::default());
    t.check("zip_rt/deflate", back.as_ref().ok() == Some(&parts), format!("{} bytes for {} of parts, read back", z.len(), 200_000 + 4096 + 1));
    t.check("zip_rt/deflate_same_bytes", !z.is_empty() && z == z2, String::from("DEFLATE output is deterministic"));
    // methods: the text is DEFLATE and smaller, noise, empty and one byte stay stored
    let mut methods = Vec::new();
    let mut p = 0;
    for e in &parts {
        let (method, packed) = (at16(&z, p + 8), at32(&z, p + 18) as usize);
        methods.push((method, packed));
        p += 30 + e.name.len() + packed;
    }
    t.check(
        "zip_rt/deflate_methods",
        methods.len() == 4 && methods[0].0 == 8 && methods[0].1 < 20_000 && methods[1] == (0, 4096) && methods[2] == (0, 0) && methods[3] == (0, 1),
        format!("(method, stored bytes) {methods:?}"),
    );
    for level in [0u8, 1, 4, 6, 9, 200] {
        let z = archive::write_with(&parts, Limits::default(), Method::Deflate(level));
        let ok = z.as_ref().ok().and_then(|z| archive::read(z, Limits::default()).ok()) == Some(parts.clone());
        t.check("zip_rt/level", ok, format!("level {level} round trip ({} bytes)", z.map(|z| z.len()).unwrap_or(0)));
    }
    let empty = archive::write(&[], Limits::default()).unwrap_or_default();
    t.check("zip_rt/empty", empty.len() == 22 && archive::read(&empty, Limits::default()) == Ok(Vec::new()), String::from("an archive without parts is the end record alone"));
    let empty = archive::write_with(&[], Limits::default(), Method::Deflate(6)).unwrap_or_default();
    t.check("zip_rt/empty_deflate", archive::read(&empty, Limits::default()) == Ok(Vec::new()), String::from("also with DEFLATE"));

    match archive::ZipFs::new(&z, Limits::default()) {
        Ok(fs) => {
            use plat::vfs::Provider;
            t.check("zip_rt/zipfs", fs.read("text.xml").as_deref() == Some(&parts[0].data[..]) && fs.read("noise.bin").as_deref() == Some(&parts[1].data[..]), String::from("ZipFs reads DEFLATE parts"));
        }
        Err(e) => t.check("zip_rt/zipfs", false, format!("ZipFs::new: {e:?}")),
    }
    // data descriptors: with and without signature, stored and DEFLATE
    for (desc, label) in [(Desc::Narrow, "narrow"), (Desc::NarrowSig, "narrow+sig"), (Desc::Wide, "wide")] {
        for deflate in [false, true] {
            let mut r = rec("a/b.xml", &text(3000), deflate);
            r.desc = desc;
            r.flags |= 8;
            if desc == Desc::Wide {
                r.local64_real = true;
                r.version = 45;
            }
            let z = plain(&[r, rec("c.txt", b"after", false)]).z;
            t.check("zip_rt/descriptor", read_err(&z, Limits::default()) == Ok(2), format!("{label} descriptor, deflate {deflate}"));
        }
    }
    let mut r = rec("a.xml", &text(3000), true);
    r.desc = Desc::NarrowSig;
    r.flags |= 8;
    r.crc ^= 1;
    let mut z = plain(&[r]).z;
    let cd = at32(&z, z.len() - 6) as usize;
    let crc_at = cd + 16;
    let crc = at32(&z, crc_at) ^ 1;
    set32(&mut z, crc_at, crc);
    t.check("zip_rt/descriptor_crc", read_err(&z, Limits::default()) == Err(Error::Invalid), String::from("a descriptor's CRC that differs from the central one"));
}

// ------------------------------------------------------------------ zip_names

fn zip_names(t: &mut H) {
    let long = "x".repeat(1025);
    let bad = ["../x", "/abs", "a//b", "a/./b", "a/../b", "a/", "C:x", "a\\b", "ctl\u{1}x", "nel\u{85}x", "", long.as_str(), "a%41", "a%2e%2e/x"];
    for name in bad {
        t.check("zip_names/write", archive::write(&[entry(name, b"x")], Limits::default()) == Err(Error::Invalid), format!("write refuses {name:?}"));
        t.check("zip_names/read", read_err(&plain(&[rec(name, b"x", false)]).z, Limits::default()) == Err(Error::Invalid), format!("read refuses {name:?}"));
    }
    for (a, b) in [("same.txt", "same.txt"), ("Readme.TXT", "readme.txt"), ("\u{c4}.txt", "\u{e4}.txt"), ("dir/K", "dir/\u{212a}")] {
        t.check("zip_names/dup_write", archive::write(&[entry(a, b"1"), entry(b, b"2")], Limits::default()) == Err(Error::Invalid), format!("write refuses {a:?} + {b:?}"));
        let z = plain(&[rec(a, b"1", false), rec(b, b"2", false)]).z;
        t.check("zip_names/dup_read", read_err(&z, Limits::default()) == Err(Error::Invalid), format!("read refuses {a:?} + {b:?}"));
    }
    // depth: 512 levels in 1023 bytes are a name like any other, in a ZipFs too
    let deep = format!("{}a", "a/".repeat(511));
    let z = archive::write(&[entry(&deep, b"deep"), entry("top.txt", b"top")], Limits::default()).unwrap_or_default();
    let fs_ok = archive::ZipFs::new(&z, Limits::default()).is_ok_and(|fs| {
        use plat::vfs::Provider;
        fs.read(&deep).as_deref() == Some(&b"deep"[..])
    });
    t.quick("zip_names/deep", (Ok(2), true), || (read_err(&z, Limits::default()), fs_ok));
    let mut r = rec("gr\u{fc}n.txt", b"x", false);
    r.flags = 0;
    t.check("zip_names/no_utf8_flag", read_err(&plain(&[r]).z, Limits::default()) == Err(Error::Unsupported), String::from("non-ASCII name without the UTF-8 flag"));
    let mut r = rec("abc.txt", b"x", false);
    r.name = vec![b'a', 0xC3, b'(', b'.', b't'];
    t.check("zip_names/bad_utf8", read_err(&plain(&[r]).z, Limits::default()) == Err(Error::Invalid), String::from("a name that is not UTF-8"));
    let mut built = plain(&[rec("aaa.txt", b"x", false)]);
    let at = built.locals[0] + 30;
    built.z[at] = b'b';
    t.check("zip_names/local_differs", read_err(&built.z, Limits::default()) == Err(Error::Invalid), String::from("a local name other than the central one"));
    t.check("zip_names/percent_opc", archive::opc::Package::read(&plain(&[rec("a%20b.xml", b"x", false)]).z, Limits::default(), Rules::DEFAULT).err() == Some(opc::Error::Invalid), String::from("'%' passes the ZIP layer of a package (refused later: no content types)"));
}

// ------------------------------------------------------------------ zip_dir

fn zip_dir(t: &mut H) {
    let parts = vec![rec("a.txt", b"first part", false), rec("b/c.xml", &text(4000), true), rec("d.bin", &noise(500, 9), false)];
    let good = plain(&parts);
    t.check("zip_dir/control", read_err(&good.z, Limits::default()) == Ok(3), String::from("the crafted control archive reads"));
    let z = &good.z;
    let mut all_err = true;
    for cut in (0..z.len()).step_by(7).chain([z.len() - 1, z.len() - 21, z.len() - 22]) {
        all_err &= archive::read(&z[..cut], Limits::default()).is_err();
    }
    t.check("zip_dir/truncated", all_err, format!("every cut of {} bytes refused", z.len()));
    t.check("zip_dir/short", read_err(b"PK\x05\x06", Limits::default()) == Err(Error::Invalid), String::from("shorter than an end record"));
    t.check("zip_dir/noise", read_err(&noise(5000, 1), Limits::default()) == Err(Error::Invalid), String::from("no end record"));
    let with_comment = build(&parts, End::Plain, 0, b"a comment");
    t.check("zip_dir/comment", read_err(&with_comment.z, Limits::default()) == Ok(3), String::from("an archive comment"));
    let mut c = with_comment.z.clone();
    c.push(0);
    t.check("zip_dir/after_comment", read_err(&c, Limits::default()) == Err(Error::Invalid), String::from("a byte after the comment"));
    let mut c = good.z.clone();
    c.extend_from_slice(b"trailing");
    t.check("zip_dir/trailing", read_err(&c, Limits::default()) == Err(Error::Invalid), String::from("data after the end record"));

    let e = good.end;
    let patched = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut z = good.z.clone();
        f(&mut z);
        read_err(&z, Limits::default())
    };
    t.check("zip_dir/cd_offset", patched(&|z| set32(z, e + 16, good.cd as u32 + 1)) == Err(Error::Invalid), String::from("central directory one byte off"));
    t.check("zip_dir/cd_size", patched(&|z| { let n = at32(z, e + 12) - 1; set32(z, e + 12, n) }) == Err(Error::Invalid), String::from("central directory size one short"));
    t.check("zip_dir/count_more", patched(&|z| { set16(z, e + 8, 4); set16(z, e + 10, 4) }) == Err(Error::Invalid), String::from("one entry more than there are"));
    t.check("zip_dir/count_less", patched(&|z| { set16(z, e + 8, 2); set16(z, e + 10, 2) }) == Err(Error::Invalid), String::from("one entry less than there are"));
    t.check("zip_dir/count_disk", patched(&|z| set16(z, e + 8, 2)) == Err(Error::Unsupported), String::from("entries on this disk differ from all entries"));
    t.check("zip_dir/disks", patched(&|z| set16(z, e + 4, 1)) == Err(Error::Unsupported), String::from("a multi-disk archive"));
    t.check("zip_dir/count_65535", patched(&|z| { set16(z, e + 8, 0xFFFF); set16(z, e + 10, 0xFFFF) }) == Err(Error::Unsupported), String::from("65535 entries without ZIP64 records"));
    t.check("zip_dir/part_disk", patched(&|z| set16(z, good.cds[1] + 34, 1)) == Err(Error::Unsupported) && patched(&|z| set16(z, good.cds[1] + 34, 0xFFFF)) == Err(Error::Unsupported), String::from("a part on another disk (also 0xFFFF without a ZIP64 field)"));
    t.check("zip_dir/cd_sig",patched(&|z| z[good.cds[1]] = b'X') == Err(Error::Invalid), String::from("a broken central record signature"));
    t.check("zip_dir/local_sig", patched(&|z| z[good.locals[1]] = b'X') == Err(Error::Invalid), String::from("a broken local header signature"));
    t.check("zip_dir/same_local", patched(&|z| set32(z, good.cds[2] + 42, good.locals[0] as u32)) == Err(Error::Invalid), String::from("two central records, one local header"));
    t.check("zip_dir/crc", patched(&|z| { let c = at32(z, good.cds[0] + 16) ^ 4; set32(z, good.cds[0] + 16, c); set32(z, good.locals[0] + 14, c) }) == Err(Error::Checksum), String::from("a CRC that does not match the data"));
    t.check("zip_dir/local_crc", patched(&|z| z[good.locals[0] + 14] ^= 1) == Err(Error::Invalid), String::from("local CRC other than the central one"));
    t.check("zip_dir/local_size", patched(&|z| z[good.locals[0] + 22] ^= 1) == Err(Error::Invalid), String::from("local size other than the central one"));
    t.check("zip_dir/local_method", patched(&|z| set16(z, good.locals[1] + 8, 0)) == Err(Error::Invalid), String::from("local method other than the central one"));
    t.check("zip_dir/local_flags", patched(&|z| set16(z, good.locals[1] + 6, 0)) == Err(Error::Invalid), String::from("local flags other than the central ones"));
    for (flag, what) in [(1u16, "encrypted"), (0x41, "strong encryption"), (0x2000, "masked local headers")] {
        t.check("zip_dir/flags", patched(&|z| { set16(z, good.cds[0] + 8, 0x800 | flag); set16(z, good.locals[0] + 6, 0x800 | flag) }) == Err(Error::Unsupported), String::from(what));
    }
    for method in [1u16, 12, 14, 93, 99] {
        t.check("zip_dir/method", patched(&|z| { set16(z, good.cds[0] + 10, method); set16(z, good.locals[0] + 8, method) }) == Err(Error::Unsupported), format!("method {method}"));
    }
    for (v, want) in [(46u16, Err(Error::Unsupported)), (63, Err(Error::Unsupported)), (45, Ok(3)), (10, Ok(3))] {
        t.check("zip_dir/version", patched(&|z| { set16(z, good.cds[0] + 6, v); set16(z, good.locals[0] + 4, v) }) == want, format!("version needed {v}: {want:?}"));
    }
    // gaps: before the first record, between records (data hidden there), before the directory
    let mut gap = parts.clone();
    gap[0].gap = 5;
    t.check("zip_dir/gap_first", read_err(&plain(&gap).z, Limits::default()) == Err(Error::Unsupported), String::from("bytes before the first record"));
    let mut gap = parts.clone();
    gap[2].gap = 64;
    t.check("zip_dir/gap_between", read_err(&plain(&gap).z, Limits::default()) == Err(Error::Unsupported), String::from("bytes hidden between records"));
    t.check("zip_dir/gap_cd", read_err(&build(&parts, End::Plain, 9, b"").z, Limits::default()) == Err(Error::Invalid), String::from("bytes before the central directory"));
    // an overlap bomb: the first record's stored data quotes the second record whole
    let inner = rec("kernel.bin", &vec![0u8; 100_000], true);
    let inner_built = plain(&[inner.clone()]);
    let quoted = inner_built.z[..inner_built.cd].to_vec();
    let outer = rec("outer.bin", &quoted, false);
    let mut bomb = plain(&[outer.clone(), inner.clone()]);
    // point the second central record into the first record's data
    let inside = bomb.locals[0] + 30 + outer.name.len();
    set32(&mut bomb.z, bomb.cds[1] + 42, inside as u32);
    t.quick("zip_dir/overlap", Err(Error::Invalid), || read_err(&bomb.z, big()));
    // DEFLATE data that is broken or longer than the stream
    let mut r = rec("x.xml", &text(5000), true);
    let n = r.body.len();
    r.body[n / 2] ^= 0x55;
    t.check("zip_dir/deflate_broken", archive::read(&plain(&[r]).z, Limits::default()).is_err(), String::from("a corrupted DEFLATE stream"));
    let mut r = rec("x.xml", &text(5000), true);
    r.body.extend_from_slice(b"tail");
    t.check("zip_dir/deflate_tail", read_err(&plain(&[r]).z, Limits::default()) == Err(Error::Invalid), String::from("bytes after the end of the DEFLATE stream"));
    let mut r = rec("x.txt", b"stored data", false);
    r.size += 1;
    t.check("zip_dir/stored_size", read_err(&plain(&[r]).z, Limits::default()) == Err(Error::Invalid), String::from("stored part whose sizes differ"));
    t.check("zip_dir/archive_bytes", read_err(&good.z, Limits { archive_bytes: good.z.len() - 1, ..Limits::default() }) == Err(Error::Limit), String::from("archive one byte over archive_bytes"));
}

// ------------------------------------------------------------------ zip_limits

fn zip_limits(t: &mut H) {
    let many: Vec<Entry> = (0..513).map(|i| entry(&format!("p/{i}.txt"), b"x")).collect();
    t.check("zip_limits/entries_write", archive::write(&many, Limits::default()) == Err(Error::Limit), String::from("513 parts, default limits"));
    let z = archive::write(&many, big()).unwrap_or_default();
    t.quick("zip_limits/entries_read", Err(Error::Limit), || read_err(&z, Limits::default()));
    t.check("zip_limits/entries_ok", read_err(&z, Limits { entries: 513, ..Limits::default() }) == Ok(513), String::from("513 parts with entries = 513"));
    let nine = vec![entry("big.bin", &vec![1u8; 9 << 20])];
    t.check("zip_limits/part_write", archive::write(&nine, Limits { archive_bytes: 64 << 20, ..Limits::default() }) == Err(Error::Limit), String::from("a 9 MiB part, default part_bytes"));
    let z = archive::write_with(&nine, big(), Method::Deflate(1)).unwrap_or_default();
    t.quick("zip_limits/part_read", Err(Error::Limit), || read_err(&z, Limits::default()));
    let five: Vec<Entry> = (0..5).map(|i| entry(&format!("{i}.bin"), &vec![i as u8; 7 << 20])).collect();
    t.check("zip_limits/total_write", archive::write_with(&five, Limits::default(), Method::Deflate(6)) == Err(Error::Limit), String::from("5 x 7 MiB, default total_bytes"));
    let z = archive::write_with(&five, big(), Method::Deflate(6)).unwrap_or_default();
    t.quick("zip_limits/total_read", Err(Error::Limit), || read_err(&z, Limits::default()));
    t.check("zip_limits/total_ok", read_err(&z, big()) == Ok(5), format!("the same {} bytes with larger limits", z.len()));
    t.check("zip_limits/archive_write", archive::write(&[entry("a", &[0; 100])], Limits { archive_bytes: 100, ..Limits::default() }) == Err(Error::Limit), String::from("archive over archive_bytes"));

    // the ratio: 4 MiB of zeros take about 4 KB (about 1000:1)
    let zeros = vec![entry("zeros.bin", &vec![0u8; 4 << 20])];
    let z = archive::write_with(&zeros, Limits::default(), Method::Deflate(9)).unwrap_or_default();
    t.check("zip_limits/ratio_default", read_err(&z, Limits::default()) == Ok(1), format!("4 MiB in {} bytes with the default ratio", z.len()));
    t.quick("zip_limits/ratio_low", Err(Error::Limit), || read_err(&z, Limits { ratio: 100, ..Limits::default() }));
    let small = vec![entry("zeros.bin", &vec![0u8; 900 << 10])];
    let z = archive::write_with(&small, Limits::default(), Method::Deflate(9)).unwrap_or_default();
    t.check("zip_limits/ratio_floor", read_err(&z, Limits { ratio: 10, ..Limits::default() }) == Ok(1), String::from("900 KiB parts are not checked against the ratio"));
    // sizes no DEFLATE stream reaches: refused before the buffer is allocated
    let mut b = plain(&[rec("claim.bin", &text(1000), true)]);
    for p in [b.cds[0] + 24, b.locals[0] + 22] {
        set32(&mut b.z, p, 0xFFFF_FFFE);
    }
    let huge = Limits { part_bytes: usize::MAX / 2, total_bytes: usize::MAX / 2, ..big() };
    t.quick("zip_limits/impossible", Err(Error::Invalid), || read_err(&b.z, huge));
    // a 16 MiB bomb
    let bomb = vec![entry("bomb.bin", &vec![0u8; 16 << 20])];
    let z = archive::write_with(&bomb, big(), Method::Deflate(9)).unwrap_or_default();
    t.quick("zip_limits/bomb_default", Err(Error::Limit), || read_err(&z, Limits::default()));
    t.quick("zip_limits/bomb_ratio", Err(Error::Limit), || read_err(&z, Limits { ratio: 100, ..big() }));
    t.check("zip_limits/bomb_allowed", read_err(&z, big()) == Ok(1), format!("{} bytes inflate to 16 MiB within larger limits", z.len()));
    // an end record that claims 60000 entries in a tiny archive
    let mut z = archive::write(&[entry("a", b"x")], Limits::default()).unwrap_or_default();
    let e = z.len() - 22;
    set16(&mut z, e + 8, 60_000);
    set16(&mut z, e + 10, 60_000);
    t.quick("zip_limits/count_default", Err(Error::Limit), || read_err(&z, Limits::default()));
    t.quick("zip_limits/count_big", Err(Error::Invalid), || read_err(&z, big()));
}

// ------------------------------------------------------------------ zip64

fn zip64(t: &mut H) {
    let parts = vec![entry("a.txt", b"small"), entry("b.xml", &text(3000)), entry("c.bin", &noise(1000, 5)), entry("d/e.txt", b"")];
    for method in [Method::Stored, Method::Deflate(6)] {
        let z = archive::write_zip64_from(&parts, Limits::default(), method, 0).unwrap_or_default();
        t.check(
            "zip64/all",
            archive::read(&z, Limits::default()).as_ref().ok() == Some(&parts) && contains(&z, 0x06064b50) && contains(&z, 0x07064b50),
            format!("{method:?}: every size and offset through ZIP64, {} bytes, read back", z.len()),
        );
        let z = archive::write_zip64_from(&parts, Limits::default(), method, 200).unwrap_or_default();
        t.check("zip64/mixed", archive::read(&z, Limits::default()).as_ref().ok() == Some(&parts), format!("{method:?}: ZIP64 from 200 bytes on, read back"));
        let plain = archive::write_with(&parts, Limits::default(), method).unwrap_or_default();
        t.check("zip64/not_needed", !contains(&plain, 0x06064b50) && !contains(&plain, 0x07064b50) && at16(&plain, 4) == 20, format!("{method:?}: no ZIP64 record, version 2.0"));
    }
    // 65535 parts need the ZIP64 end records, 65534 do not
    for n in [65_534usize, 65_535, 66_000] {
        let many: Vec<Entry> = (0..n).map(|i| entry(&format!("{:05x}", i), b"")).collect();
        let limits = Limits { entries: 70_000, ..big() };
        let t0 = plat::time_us();
        let z = archive::write(&many, limits).unwrap_or_default();
        let back = archive::read(&z, limits);
        let ms = plat::time_us().wrapping_sub(t0) / 1000;
        let z64 = contains(&z[z.len().saturating_sub(120)..], 0x07064b50);
        t.check(
            "zip64/count",
            back.as_ref().is_ok_and(|b| b.len() == n && b[n - 1].name == many[n - 1].name) && z64 == (n >= 65_535) && ms < 20_000,
            format!("{n} parts: ZIP64 end {z64}, read back in {ms} ms"),
        );
    }
    // foreign layouts
    let mut net = rec("net.xml", &text(2000), true);
    net.local64_real = true;
    net.version = 45;
    t.check("zip64/foreign_net", read_err(&plain(&[net, rec("b", b"x", false)]).z, Limits::default()) == Ok(2), String::from(".NET: ZIP64 field with real local sizes"));
    let mut full = rec("full.xml", &text(2000), true);
    (full.local64, full.cd64, full.version) = (true, true, 45);
    let full_b = plain(&[full.clone(), rec("b", b"x", false)]);
    t.check("zip64/foreign_marks", read_err(&full_b.z, Limits::default()) == Ok(2), String::from("sizes and offset through ZIP64 fields"));
    let mut wide = rec("w.xml", &text(2000), true);
    (wide.desc, wide.flags, wide.local64_real, wide.version) = (Desc::Wide, 0x808, true, 45);
    t.check("zip64/foreign_descriptor", read_err(&plain(&[wide]).z, Limits::default()) == Ok(1), String::from("a 64-bit data descriptor"));
    let recs = [rec("a", b"1", false), rec("b", &text(500), true)];
    t.check("zip64/end_real", read_err(&build(&recs, End::Z64Real, 0, b"").z, Limits::default()) == Ok(2), String::from("ZIP64 end records with real values in the classic one"));
    t.check("zip64/end_marks", read_err(&build(&recs, End::Z64Marks, 0, b"c").z, Limits::default()) == Ok(2), String::from("ZIP64 end records, the classic one all marks"));
    // hostile ZIP64
    let mut r = rec("m.xml", b"data", false);
    r.version = 45;
    let mut b = plain(&[r.clone()]);
    set32(&mut b.z, b.cds[0] + 24, M32);
    t.check("zip64/mark_no_field", read_err(&b.z, Limits::default()) == Err(Error::Invalid), String::from("a mark without a ZIP64 field"));
    let mut b = full_b.z.clone();
    let field = full_b.cds[0] + 46 + full.name.len();
    set16(&mut b, field + 2, 16);
    t.check("zip64/field_short", archive::read(&b, Limits::default()).is_err(), String::from("a ZIP64 field too short for its marks"));
    let mut b = full_b.z.clone();
    set64(&mut b, field + 4, 1 << 62);
    t.quick("zip64/size_huge", Err(Error::Limit), || read_err(&b, Limits::default()));
    let mut b = full_b.z.clone();
    set64(&mut b, field + 20, 1 << 40);
    t.quick("zip64/offset_huge", Err(Error::Invalid), || read_err(&b, Limits::default()));
    let mut b = full_b.z.clone();
    set64(&mut b, full_b.locals[0] + 30 + full.name.len() + 4, 7);
    t.check("zip64/local_disagrees", read_err(&b, Limits::default()) == Err(Error::Invalid), String::from("local ZIP64 size other than the central one"));
    let end = build(&recs, End::Z64Marks, 0, b"");
    let rec64 = end.end - 20 - 56;
    let mut b = end.z.clone();
    set64(&mut b, rec64 + 32, 1 << 40);
    set64(&mut b, rec64 + 24, 1 << 40);
    t.quick("zip64/count_huge", Err(Error::Limit), || read_err(&b, Limits::default()));
    let mut b = end.z.clone();
    set64(&mut b, rec64 + 48, 1 << 63);
    t.quick("zip64/cd_huge", Err(Error::Invalid), || read_err(&b, Limits::default()));
    let mut b = end.z.clone();
    set32(&mut b, rec64 + 16, 1);
    t.check("zip64/disk", read_err(&b, Limits::default()) == Err(Error::Unsupported), String::from("a ZIP64 end record on another disk"));
    let mut b = end.z.clone();
    set64(&mut b, end.end - 20 + 8, 3);
    t.check("zip64/locator_wrong", read_err(&b, Limits::default()) == Err(Error::Unsupported), String::from("a locator that leads nowhere: the classic record's marks, Unsupported as before ZIP64"));
    let real = build(&recs, End::Z64Real, 0, b"");
    let mut b = real.z.clone();
    set16(&mut b, real.end + 10, 1);
    set16(&mut b, real.end + 8, 1);
    t.check("zip64/end_disagrees", read_err(&b, Limits::default()) == Err(Error::Invalid), String::from("classic count other than the ZIP64 one"));
}

// ------------------------------------------------------------------ opc

const VSD_MAIN: &str = "application/vnd.ms-visio.drawing.main+xml";
const PAGES: &str = "application/vnd.ms-visio.pages+xml";
const PAGE: &str = "application/vnd.ms-visio.page+xml";
const R: &str = "http://schemas.microsoft.com/visio/2010/relationships";

fn rel(id: &str, kind: &str, target: &str) -> Relationship {
    Relationship { id: String::from(id), kind: String::from(kind), target: String::from(target), external: false }
}
/// a small VSDX-like package
fn sample_package() -> Package {
    let mut p = Package::new(Rules::DEFAULT);
    let ct = p.content_types_mut();
    let _ = ct.set_default("rels", opc::RELS_TYPE);
    let _ = ct.set_default("xml", "application/xml");
    let _ = ct.set_default("bin", "application/octet-stream");
    let parts = [
        ("visio/document.xml", VSD_MAIN, text(3000)),
        ("visio/pages/pages.xml", PAGES, text(500)),
        ("visio/pages/page1.xml", PAGE, text(20_000)),
        ("visio/embeddings/blob.bin", "application/octet-stream", noise(2000, 11)),
        ("customXml/item1.xml", "application/xml", text(100)),
        ("visio/unknownExt.dat", "application/vnd.example.unknown", noise(300, 2)),
        ("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml", text(200)),
    ];
    for (name, ct, data) in parts {
        let _ = p.set_part(name, ct, data);
    }
    let _ = p.set_relationships(
        "",
        &[
            rel("rId1", opc::OFFICE_DOCUMENT, "visio/document.xml"),
            rel("rId2", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml"),
        ],
    );
    let _ = p.set_relationships("visio/document.xml", &[rel("rId1", &format!("{R}/pages"), "pages/pages.xml"), rel("rId2", "http://example.com/unknown", "../customXml/item1.xml")]);
    let mut link = rel("rId2", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink", "file:///C:/Windows/win.ini");
    link.external = true;
    let _ = p.set_relationships("visio/pages/pages.xml", &[rel("rId1", &format!("{R}/page"), "page1.xml"), link]);
    p
}
fn zip_of(p: &Package) -> Vec<u8> {
    p.write(Limits::default(), Method::Deflate(6)).unwrap_or_default()
}
/// the package's ZIP items with one item's text changed
fn mutate(z: &[u8], name: &str, before: &str, after: &str) -> Vec<Entry> {
    let mut e = archive::read(z, Limits::default()).unwrap_or_default();
    if let Some(x) = e.iter_mut().find(|e| e.name == name) {
        let s = String::from_utf8_lossy(&x.data).replacen(before, after, 1);
        x.data = s.into_bytes();
    }
    e
}
fn opc_err(e: Vec<Entry>, rules: Rules) -> Option<opc::Error> {
    Package::from_entries(e, rules).err()
}

fn opc_checks(t: &mut H) {
    let p = sample_package();
    let z = p.write(Limits::default(), Method::Deflate(6));
    t.check("opc/write", z.is_ok(), format!("a new package written: {:?}", z.as_ref().map(|z| z.len())));
    let z = z.unwrap_or_default();
    let r = Package::read(&z, Limits::default(), Rules::DEFAULT);
    t.check("opc/read", r.is_ok(), format!("and read: {:?}", r.as_ref().err()));
    let Ok(q) = r else { return };
    t.check("opc/main", q.main_part() == Some("visio/document.xml"), format!("main part {:?}", q.main_part()));
    t.check("opc/types", q.content_type("visio/pages/page1.xml") == Some(PAGE) && q.content_type("visio/embeddings/blob.bin") == Some("application/octet-stream") && q.content_type("VISIO/UNKNOWNEXT.DAT") == Some("application/vnd.example.unknown"), String::from("content types by override and default, case-insensitive"));
    let rels = q.relationships("visio/pages/pages.xml").unwrap_or_default();
    t.check("opc/relationships", rels.len() == 2 && rels[1].external && rels[1].target == "file:///C:/Windows/win.ini" && q.resolve("visio/pages/pages.xml", &rels[0].target) == Ok("visio/pages/page1.xml"), format!("{rels:?}"));
    t.check("opc/resolve_up", q.resolve("visio/document.xml", "../customXml/item1.xml") == Ok("customXml/item1.xml"), String::from("'..' within the package"));
    let same = q.write(Limits::default(), Method::Deflate(6)).unwrap_or_default();
    t.check("opc/rewrite_same", same == z, String::from("read -> write unchanged gives the same bytes"));
    t.check("opc/zip_layer", archive::read(&z, Limits::default()).is_ok_and(|e| e[0].name == opc::CONTENT_TYPES), String::from("[Content_Types].xml first, plain ZIP"));

    // unknown parts are kept as they are when a known part changes
    let mut edit = q.clone();
    let before: Vec<Entry> = q.entries().to_vec();
    let _ = edit.replace("visio/pages/page1.xml", text(999));
    let z2 = edit.write(Limits::default(), Method::Stored).unwrap_or_default();
    let after = archive::read(&z2, Limits::default()).unwrap_or_default();
    let kept = before.iter().filter(|e| e.name != "visio/pages/page1.xml").all(|e| after.iter().any(|a| a == e));
    t.check("opc/preserve", kept && after.len() == before.len(), format!("{} of {} items unchanged byte for byte, [Content_Types].xml included", after.len() - 1, before.len()));

    // names: equivalence, percent-escapes
    let mut n = sample_package();
    let _ = n.set_part("visio/media/image 1.png", "image/png", vec![1, 2, 3]);
    let _ = n.set_part("visio/media/%C3%A4.png", "image/png", vec![4]);
    let _ = n.set_relationships("visio/pages/page1.xml", &[rel("rId1", "http://x/image", "../media/IMAGE%201.PNG"), rel("rId2", "http://x/image", "../media/\u{e4}.png"), rel("rId3", "http://x/image", "../media/%c3%a4.png"), rel("rId4", "http://x/image", "../media/%2E/image%201.png")]);
    let zn = n.write(Limits::default(), Method::Deflate(6));
    t.check("opc/names_write", zn.is_ok(), format!("escaped and spaced names written: {:?}", zn.as_ref().err()));
    let zn = zn.unwrap_or_default();
    match Package::read(&zn, Limits::default(), Rules::DEFAULT) {
        Ok(q) => {
            let rels = q.relationships("visio/pages/page1.xml").unwrap_or_default();
            let names: Vec<_> = rels.iter().map(|r| q.resolve("visio/pages/page1.xml", &r.target)).collect();
            t.check("opc/names_resolve", names == [Ok("visio/media/image 1.png"), Ok("visio/media/%C3%A4.png"), Ok("visio/media/%C3%A4.png"), Ok("visio/media/image 1.png")], format!("{names:?}"));
        }
        Err(e) => t.check("opc/names_resolve", false, format!("read: {e:?}")),
    }
    t.check("opc/names_exact", Package::read(&zn, Limits::default(), Rules { exact_names: true, ..Rules::DEFAULT }).err() == Some(opc::Error::Zip(Error::Invalid)), String::from("the exact profile refuses '%' in ZIP names"));
    t.check("opc/normalize", opc::normalize("/a/%7euser/\u{e4} b%3a").as_deref() == Ok("a/~user/%C3%A4%20b%3A") && opc::same_part("Visio/A%20B.xml", "visio/a b.XML") && !opc::same_part("a.xml", "b.xml"), String::from("canonical names, equivalence"));
    for bad in ["a/%2F", "a%5cb", "%zz", "a%", "a/%2E/b", "a/%2e%2e/b", "a//b", "a\\b", "a/\u{1}b"] {
        t.check("opc/normalize_bad", opc::normalize(bad) == Err(opc::Error::Address), format!("{bad:?} refused"));
    }

    // hostile packages: the XML of one item changed
    let z = zip_of(&sample_package());
    let main_rel = "Target=\"visio/document.xml\"";
    let cases: Vec<(&str, &str, String, opc::Error)> = vec![
        ("_rels/.rels", main_rel, String::from("Target=\"../visio/document.xml\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, String::from("Target=\"visio/missing.xml\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, String::from("Target=\"%2Fvisio/document.xml\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"visio%5cdocument.xml\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"visio/%zzdocument.xml\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"http://example.com/document.xml\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"visio/document.xml#frag\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"visio/document.xml?q=1\""), opc::Error::Address),
        ("_rels/.rels", main_rel, String::from("Target=\"%2E%2E/%2E%2E/visio/document.xml\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, String::from("Target=\"/\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, String::from("Target=\"\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, format!("{main_rel} TargetMode=\"External\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, format!("{main_rel} TargetMode=\"Unclear\""), opc::Error::Invalid),
        ("_rels/.rels", main_rel, format!("{main_rel} Extra=\"1\""), opc::Error::Invalid),
        ("_rels/.rels", "Id=\"rId2\"", String::from("Id=\"rId1\""), opc::Error::Invalid),
        ("_rels/.rels", "Id=\"rId2\"", String::from("Id=\"\""), opc::Error::Invalid),
        ("_rels/.rels", "</Relationships>", format!("<Relationship Id=\"rId9\" Type=\"{}\" Target=\"visio/document.xml\"/></Relationships>", opc::OFFICE_DOCUMENT), opc::Error::Invalid),
        ("_rels/.rels", "</Relationships>", String::from("<Relationship Id=\"rId9\" Type=\"http://schemas.microsoft.com/office/2006/relationships/vbaProject\" Target=\"visio/document.xml\"/></Relationships>"), opc::Error::MacroRelationship),
        ("_rels/.rels", "<Relationships", String::from("<!DOCTYPE r><Relationships"), opc::Error::Xml(base::xml::Error::Unsupported)),
        ("_rels/.rels", "<Relationships xmlns", String::from("<Relations xmlns"), opc::Error::Xml(base::xml::Error::Syntax)),
        ("[Content_Types].xml", VSD_MAIN, String::from("application/vnd.ms-visio.drawing.macroEnabled.main+xml"), opc::Error::ContentType),
        ("[Content_Types].xml", VSD_MAIN, String::new(), opc::Error::ContentType),
        ("[Content_Types].xml", "<Types xmlns", String::from("<Typez xmlns"), opc::Error::Xml(base::xml::Error::Syntax)),
        ("[Content_Types].xml", "PartName=\"/visio/document.xml\"", String::from("PartName=\"/visio/missing.xml\""), opc::Error::Invalid),
        ("[Content_Types].xml", "PartName=\"/visio/document.xml\"", String::from("PartName=\"visio/document.xml\""), opc::Error::Invalid),
        ("[Content_Types].xml", "PartName=\"/visio/document.xml\"", String::from("PartName=\"/visio/./document.xml\""), opc::Error::Invalid),
        ("[Content_Types].xml", "Extension=\"bin\"", String::from("Extension=\"b.n\""), opc::Error::Invalid),
        ("[Content_Types].xml", "Extension=\"bin\"", String::from("Extension=\"XML\""), opc::Error::Invalid),
        ("[Content_Types].xml", "Extension=\"bin\"", String::from("Extension=\"bun\""), opc::Error::Invalid),
        ("[Content_Types].xml", "<Default Extension=\"bin\"", String::from("<Defaults Extension=\"bin\""), opc::Error::Invalid),
        ("[Content_Types].xml", "</Types>", String::from("<Override PartName=\"/VISIO/DOCUMENT.XML\" ContentType=\"x/y\"/></Types>"), opc::Error::Invalid),
        ("visio/_rels/document.xml.rels", "../customXml/item1.xml", String::from("../../customXml/item1.xml"), opc::Error::Invalid),
    ];
    for (name, before, after, want) in &cases {
        let got = opc_err(mutate(&z, name, before, after), Rules::DEFAULT);
        t.check("opc/hostile", got.as_ref() == Some(want), format!("{name}: {before:?} -> {after:?}: {got:?}"));
    }
    let root = mutate(&z, "[Content_Types].xml", "<Types xmlns=", "<Types a=\"1\" xmlns=");
    t.check("opc/hostile_root", opc_err(root, Rules::DEFAULT) == Some(opc::Error::Root(String::from("Types"))), String::from("attributes on <Types>"));
    let root = mutate(&z, "_rels/.rels", "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">", "<Relationships xmlns=\"urn:other\">");
    t.check("opc/hostile_ns", opc_err(root, Rules::DEFAULT) == Some(opc::Error::Root(String::from("Relationships"))), String::from("relationships in another namespace"));
    let entries = archive::read(&z, Limits::default()).unwrap_or_default();
    let without = |name: &str| entries.iter().filter(|e| e.name != name).cloned().collect::<Vec<_>>();
    t.check("opc/no_types", opc_err(without(opc::CONTENT_TYPES), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("no [Content_Types].xml"));
    t.check("opc/no_package_rels", opc_err(without(opc::PACKAGE_RELS), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("no _rels/.rels"));
    let with = |extra: Entry| {
        let mut e = entries.clone();
        e.push(extra);
        e
    };
    t.check("opc/orphan_rels", opc_err(with(entry("visio/_rels/nothing.xml.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>")), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("relationships of a part that does not exist"));
    t.check("opc/stray_rels", opc_err(with(entry("visio/stray.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>")), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("a .rels item outside a _rels folder"));
    t.check("opc/no_type", opc_err(with(entry("visio/x.unknownext", b"1")), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("a part without a content type"));
    t.check("opc/vba_part", opc_err(with(entry("visio/vbaProject.bin", b"1")), Rules::DEFAULT) == Some(opc::Error::MacroPart), String::from("vbaProject.bin whatever its content type"));
    t.check("opc/vba_allowed", opc_err(with(entry("visio/vbaProject.bin", b"1")), Rules { macros: true, ..Rules::DEFAULT }).is_none(), String::from("accepted with Rules::macros"));
    t.check("opc/dup_equivalent", opc_err(with(entry("visio/Pages/PAGE1.xml", b"1")), Rules::DEFAULT).is_some() && opc_err(with(entry("visio/pages/page%31.xml", b"1")), Rules::DEFAULT) == Some(opc::Error::Invalid), String::from("two names for one part"));
    t.check("opc/escaped_slash", opc_err(with(entry("visio/a%2Fb.xml", b"1")), Rules::DEFAULT) == Some(opc::Error::Address), String::from("an escaped '/' in a ZIP name"));
    let macro_ct = mutate(&z, "[Content_Types].xml", VSD_MAIN, "application/vnd.ms-visio.drawing.macroEnabled.main+xml");
    t.check("opc/macro_allowed", opc_err(macro_ct, Rules { macros: true, ..Rules::DEFAULT }).is_none(), String::from("a macro-enabled main part with Rules::macros"));
    // case and the exact profile
    let case = mutate(&z, "[Content_Types].xml", "PartName=\"/visio/document.xml\"", "PartName=\"/Visio/Document.XML\"");
    t.check("opc/case_equivalent", opc_err(case.clone(), Rules::DEFAULT).is_none(), String::from("an override in other case names the part"));
    t.check("opc/case_exact", opc_err(case, Rules { exact_names: true, ..Rules::DEFAULT }) == Some(opc::Error::Invalid), String::from("not in the exact profile"));
    let case = mutate(&z, "_rels/.rels", main_rel, "Target=\"VISIO/document.xml\"");
    t.check("opc/case_target", opc_err(case.clone(), Rules::DEFAULT).is_none() && opc_err(case, Rules { exact_names: true, ..Rules::DEFAULT }) == Some(opc::Error::Invalid), String::from("a target in other case: equivalent, not exact"));
    let pct = mutate(&z, "_rels/.rels", main_rel, "Target=\"visio/docu%6Dent.xml\"");
    t.check("opc/percent_target", opc_err(pct.clone(), Rules::DEFAULT).is_none() && opc_err(pct, Rules { exact_names: true, ..Rules::DEFAULT }) == Some(opc::Error::Address), String::from("an escaped target: resolved, refused by the exact profile"));
    // XML limits
    let mut many = String::from("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    for i in 0..40_000 {
        many.push_str(&format!("<Relationship Id=\"r{i}\" Type=\"http://x/y\" Target=\"page1.xml\"/>"));
    }
    many.push_str("</Relationships>");
    let big_rels = with(entry("visio/pages/_rels/page1.xml.rels", many.as_bytes()));
    t.quick("opc/xml_nodes", Some(opc::Error::Xml(base::xml::Error::Limit)), || opc_err(big_rels.clone(), Rules::DEFAULT));
    let roomy = Rules { xml: base::xml::Limits { bytes: 8 << 20, nodes: 50_000, depth: 48 }, ..Rules::DEFAULT };
    t.check("opc/xml_roomy", opc_err(big_rels, roomy).is_none(), String::from("40000 relationships with larger XML limits"));
    let deep = format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}{}</Relationships>", "<a>".repeat(100), "</a>".repeat(100));
    t.quick("opc/xml_depth", Some(opc::Error::Xml(base::xml::Error::Limit)), || opc_err(with(entry("visio/pages/_rels/page1.xml.rels", deep.as_bytes())), Rules::DEFAULT));

    // edits
    let mut e = sample_package();
    t.check("opc/remove", e.remove_part("visio/pages/page1.xml") && e.part("visio/pages/_rels/page1.xml.rels").is_none() && e.write(Limits::default(), Method::Stored).err() == Some(opc::Error::Invalid), String::from("removing a part leaves its relationship dangling: write refuses"));
    let _ = e.set_relationships("visio/pages/pages.xml", &[]);
    t.check("opc/remove_fixed", e.part("visio/pages/_rels/pages.xml.rels").is_none() && e.write(Limits::default(), Method::Stored).is_ok(), String::from("an empty relationship list removes the part, the package writes again"));
    t.check("opc/set_part_bad", e.set_part("visio/../x.xml", "a/b", vec![]).is_err() && e.set_part("[Content_Types].xml", "a/b", vec![]).is_err() && e.set_part("a.xml", "", vec![]) == Err(opc::Error::ContentType), String::from("bad names and empty content types"));
    let mut fresh = Package::new(Rules::DEFAULT);
    let _ = fresh.set_part("main.xml", "application/xml", text(10));
    t.check("opc/no_rels", fresh.write(Limits::default(), Method::Stored).err() == Some(opc::Error::Invalid), String::from("a package needs its relationships"));

    // a large package: 20000 pages (20001 relationships fit the XML reader's 30000 elements)
    let t0 = plat::time_us();
    let mut large = sample_package();
    let mut list = vec![rel("rId1", &format!("{R}/page"), "page1.xml")];
    for i in 0..20_000 {
        let name = format!("page{}.xml", i + 2);
        let _ = large.set_part(&format!("visio/pages/{name}"), PAGE, format!("<PageContents n=\"{i}\"/>").into_bytes());
        list.push(rel(&format!("rId{}", i + 2), &format!("{R}/page"), &name));
    }
    let _ = large.set_relationships("visio/pages/pages.xml", &list);
    let limits = Limits { entries: 30_000, ..big() };
    let written = large.write(limits, Method::Deflate(1));
    let back = written.as_ref().ok().map(|z| Package::read(z, limits, Rules::DEFAULT).map(|p| p.entries().len()));
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    t.check("opc/large", back == Some(Ok(20_011)) && ms < 20_000, format!("20000 pages built, written, read: {back:?} items in {ms} ms ({:?})", written.as_ref().err()));
}

/// run every group -> exit code
pub fn run(only: Option<&str>) -> i32 {
    let mut t = H { ok: 0, failed: 0, only: only.map(String::from) };
    if t.on("zip_rt") {
        zip_rt(&mut t);
    }
    if t.on("zip_names") {
        zip_names(&mut t);
    }
    if t.on("zip_dir") {
        zip_dir(&mut t);
    }
    if t.on("zip_limits") {
        zip_limits(&mut t);
    }
    if t.on("zip64") {
        zip64(&mut t);
    }
    if t.on("opc") {
        opc_checks(&mut t);
    }
    plat::write_stdout(format!("archive: {} ok, {} failed\n", t.ok, t.failed).as_bytes());
    if t.failed == 0 { 0 } else { 1 }
}
