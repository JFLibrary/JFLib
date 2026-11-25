// vss -- (JD17) group vss: the binary stencils of Visio 2003-2010 (diagram_io::vss) and their
// container (archive::cfb)
//
//   run(h)            the checks below; the local ones are skipped without their folders
//   write_hostile(d)  --write-vss-hostile DIR: the synthetic files and their hostile variants
//                     (cfb-*.vss, stream-*.vss), to try them elsewhere
//   write_overview(d, corpus)   --write-vss-overview DIR [--corpus DIR]: every master of each local
//                     .vss on one sheet (DIR/<stem>-alle.png, as stencil_overview.rs draws the own
//                     stencils), the same for each .vssx of the reference folder
//                     (DIR/<stem>-visio-alle.png)
//
//   vss/sniff, vss/kind_of   vss::sniff and vss::kind_of
//   vss/cfb/...       archive::cfb::File::read (and every stream read) against the files of
//                     cfbkit.rs: version 3 and 4, a DIFAT, and each Hostile variant with the error
//                     it must give (Invalid, Limit, Unsupported); the limits
//   vss/container/... vss::read against the same files: Limit("file"), Binary { 0, .. }, NotVisio
//   vss/stream/...    the stream VisioDocument (vsskit.rs) in a sound container: header, version
//                     6, pointers outside, lists longer than their stream, nesting past 12 levels,
//                     a stream pointed at twice, LZSS bombs and the byte limits, the stream count
//   vss/chunks/...    chunk streams: a chunk longer than its stream, cut, wrong headers, lists, more
//                     chunks than Limits::items
//   vss/cut/...       every prefix of a synthetic file and of its VisioDocument stream, and
//                     mutated copies: Ok or Err, never a panic, each in time
//   vss/corpus/...    (local, DF-34) every .vss of apps/JFDiagrammer/context/files/SWDB_DE (or
//                     $JFD_VSS_CORPUS): read, counted, written as .vssx and read back (the same
//                     model, the same bytes on the second write), cut and mutated; nothing is
//                     copied, only numbers are printed
//   vss/compare/...   (local) each master of a corpus .vss drawn next to the same master (NameU)
//                     of the .vssx Visio saved from it ($JFD_VSS_REFERENCE, else
//                     Documents/JFLib-jd17-visio-check/referenz in the home folder), both framed
//                     by what Visio's master covers: the share of differing inked pixels (fills,
//                     colours and lines) and of the lines alone (dark pixels without a dark one
//                     within a pixel in the other picture); reports only until COMPARE_WANT is set
//
// Environment: JFD_VSS_TRACE prints every generated case before it is read (to find the input of
// a panic); JFD_VSS_DIFF_DIR writes the compared pictures there (outside the repository).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::cfb;
use diagram_core::formula::Engine;
use diagram_core::render::Options;
use diagram_core::{CellAddr, DocKind, Document, MasterRef, Owner, SheetRef, ShapeRef};
use diagram_io::vss;
use diagram_io::vsdx::{self, Error, Format, Limits, Loaded};
use diagram_view::{FontSet, PageView};
use gfx::Surface;

use crate::cfbkit::{self, Cfb, Hostile};
use crate::vsskit::{self as kit, Doc, Entry, Pack, Ptr, Stream};
use crate::{out, H, QUICK_US};

/// a master's picture matches its reference when at most this share of the inked pixels differs
/// (by more than render::FAR in a channel)
pub const COMPARE_TOLERANCE: f64 = 0.05;
/// the share of matching masters vss/compare wants; None: it only reports (the calling session
/// sets the threshold)
pub const COMPARE_WANT: Option<f64> = None;
/// the pictures' size in pixels (a square page around each master)
const COMPARE_PX: u32 = 200;

fn read(b: &[u8], lim: &Limits) -> Result<Loaded, Error> {
    vss::read(b, DocKind::Stencil, lim)
}

fn unit(b: &[u8], lim: &Limits) -> Result<(), Error> {
    read(b, lim).map(|_| ())
}

fn bin(offset: u64, what: &'static str) -> Result<(), Error> {
    Err(Error::Binary { offset, what })
}

const NO_CFB: Result<(), Error> = Err(Error::Binary { offset: 0, what: "no compound file" });
const BAD_DOC: Result<(), Error> = Err(Error::Binary { offset: 0, what: "the VisioDocument stream" });
const LIMIT_FILE: Result<(), Error> = Err(Error::Limit("file"));

fn trace() -> bool {
    plat::env("JFD_VSS_TRACE").is_some()
}

// pseudo-random bytes (xorshift), the same on every run
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed | 1);
    (0..n).map(|_| r.next() as u8).collect()
}

// the NameU and Name of each master, the shapes of each
fn masters(l: &Loaded) -> Vec<(String, String, usize)> {
    let doc = &l.doc;
    doc.masters()
        .filter_map(|m| doc.master(m))
        .map(|m| {
            let s = |x: Option<diagram_core::Sym>| x.map(|x| String::from(doc.str(x))).unwrap_or_default();
            (s(m.header.name_u), s(m.header.name), m.shapes.iter().count())
        })
        .collect()
}

fn res(r: &Result<Loaded, Error>) -> String {
    match r {
        Ok(l) => format!("ok: {} masters, {} warnings", l.doc.masters().count(), l.warnings.len() + l.more_warnings),
        Err(e) => format!("{e:?}"),
    }
}

/// the synthetic file of the container checks: VisioDocument (a stencil of two masters; `big`:
/// with 6000 bytes more, so in sectors of its own, else in the mini stream), a property set
/// stream, a storage with a small and a large stream, an empty stream
pub fn fixture(version: u16, big: bool, min_fat: usize) -> cfbkit::Built {
    let doc = if big { kit::with(vec![Stream::blob(0x99, &noise(6000, 7)).s()]) } else { kit::with(Vec::new()) };
    Cfb {
        version,
        kids: vec![
            cfbkit::stream("VisioDocument", &doc),
            cfbkit::stream("\u{5}SummaryInformation", &noise(150, 3)),
            cfbkit::storage("Sub", vec![cfbkit::stream("Inner", &noise(300, 5)), cfbkit::stream("Large", &noise(5000, 9))]),
            cfbkit::stream("Empty", &[]),
        ],
        min_fat,
        spare_entries: 1,
    }
    .build()
}

// every stream of a compound file read
fn probe(b: &[u8], lim: cfb::Limits) -> Result<Vec<(String, Vec<u8>)>, archive::Error> {
    let f = cfb::File::read(b, lim)?;
    let mut v = Vec::new();
    for (i, e) in f.entries().iter().enumerate() {
        if e.kind == cfb::Kind::Stream && !e.name.is_empty() {
            v.push((e.name.clone(), f.stream(i)?));
        }
    }
    Ok(v)
}

fn probe_unit(b: &[u8], lim: cfb::Limits) -> Result<(), archive::Error> {
    probe(b, lim).map(|_| ())
}

pub fn run(h: &mut H) {
    if h.on("vss/sniff") || h.on("vss/kind_of") {
        sniff(h);
    }
    if h.on("vss/cfb") {
        cfb_checks(h);
    }
    if h.on("vss/container") {
        container(h);
    }
    if h.on("vss/stream") {
        stream(h);
    }
    if h.on("vss/chunks") {
        chunks(h);
    }
    if h.on("vss/cut") {
        cut(h);
    }
    if h.on("vss/corpus") || h.on("vss/compare") {
        corpus(h);
    }
}

// ---- sniff, kind_of

fn sniff(h: &mut H) {
    let good = fixture(3, false, 1).bytes;
    let v4 = fixture(4, true, 1).bytes;
    let none = Cfb { kids: vec![cfbkit::stream("WordDocument", b"no Visio here")], ..Default::default() }.build().bytes;
    let mut wrong = kit::with(Vec::new());
    wrong[0] = b'W';
    let wrong = kit::file(&wrong);
    let cases: [(&str, &[u8], bool); 7] = [
        ("v3", &good, true),
        ("v4", &v4, true),
        ("no VisioDocument", &none, false),
        ("wrong signature in the stream", &wrong, false),
        ("empty", &[], false),
        ("zip", b"PK\x03\x04\x14\x00\x00\x00\x08\x00", false),
        ("cut header", &good[..300], false),
    ];
    let got: Vec<(&str, bool)> = cases.iter().map(|(n, b, _)| (*n, vss::sniff(b))).collect();
    h.check("vss/sniff", cases.iter().zip(&got).all(|(c, g)| c.2 == g.1), format!("{got:?}"));
    let k = [vss::kind_of("vss"), vss::kind_of("VSD"), vss::kind_of("Vst"), vss::kind_of("vssx"), vss::kind_of("vdx"), vss::kind_of("")];
    h.check(
        "vss/kind_of",
        k == [Some(DocKind::Stencil), Some(DocKind::Drawing), Some(DocKind::Template), None, None, None],
        format!("{k:?}"),
    );
}

// ---- the container: archive::cfb directly

// the expected outcome of a variant: (archive::cfb, vss::read); None: Ok or the error, both are
// acceptable (reported)
fn want(v: Hostile) -> (Option<Result<(), archive::Error>>, Option<Result<(), Error>>) {
    use archive::Error as A;
    let inv = Some(Err(A::Invalid));
    match v {
        Hostile::Version5 => (Some(Err(A::Unsupported)), Some(NO_CFB)),
        Hostile::FatLoop | Hostile::FatOutside | Hostile::FatShort | Hostile::FatSpecial | Hostile::StreamLonger | Hostile::MiniLoop | Hostile::MiniOutside | Hostile::MiniFatMissing => (inv, Some(BAD_DOC)),
        // the chain is as long as the stream before it loops: a reader may stop at the size
        Hostile::FatLoopEnd => (None, None),
        Hostile::StreamHuge | Hostile::RootHuge => (Some(Err(A::Limit)), Some(LIMIT_FILE)),
        Hostile::HighSizeV3 => (Some(Ok(())), Some(Ok(()))),
        _ => (inv, Some(NO_CFB)),
    }
}

fn cfb_checks(h: &mut H) {
    let lim = cfb::Limits::default();
    // controls: every stream back as written, by path and in the directory's order
    for (version, big, min_fat) in [(3u16, false, 1usize), (3, true, 1), (4, false, 1), (4, true, 1), (3, true, 110), (3, true, 238), (4, true, 110)] {
        let b = fixture(version, big, min_fat);
        let doc = if big { kit::with(vec![Stream::blob(0x99, &noise(6000, 7)).s()]) } else { kit::with(Vec::new()) };
        let name = format!("vss/cfb/control/v{version}{}{}", if big { "-big" } else { "" }, if b.difat.is_empty() { String::new() } else { format!("-difat{}", b.difat.len()) });
        let t0 = plat::time_us();
        let r = cfb::File::read(&b.bytes, lim);
        let us = plat::time_us().wrapping_sub(t0);
        let ok = r.as_ref().is_ok_and(|f| {
            let get = |p: &[&str]| f.find(p).and_then(|i| f.stream(i).ok());
            let kids: Vec<&str> = f.children(0).iter().map(|&i| f.entries()[i].name.as_str()).collect();
            get(&["VisioDocument"]).as_deref() == Some(doc.as_slice())
                && get(&["visiodocument"]).as_deref() == Some(doc.as_slice())
                && get(&["Sub", "Inner"]) == Some(noise(300, 5))
                && get(&["SUB", "large"]) == Some(noise(5000, 9))
                && get(&["\u{5}SummaryInformation"]) == Some(noise(150, 3))
                && get(&["Empty"]) == Some(Vec::new())
                && f.find(&["Inner"]).is_none()
                && f.find(&["Sub"]).is_some_and(|i| f.stream(i) == Err(archive::Error::Invalid))
                && kids == ["Sub", "Empty", "VisioDocument", "\u{5}SummaryInformation"]
        });
        h.check(&name, ok && us < QUICK_US, format!("{} bytes, {} FAT, {} DIFAT sectors: {:?} in {} ms", b.bytes.len(), b.fat.len(), b.difat.len(), r.as_ref().map(|f| f.entries().len()), us / 1000));
    }
    // the hostile variants, version 3 and 4
    for version in [3u16, 4] {
        for v in Hostile::ALL {
            let (big, min_fat) = v.needs();
            let mut b = fixture(version, big, min_fat);
            if !v.apply(&mut b) {
                continue;
            }
            let name = format!("vss/cfb/v{version}/{}", v.name());
            match want(v).0 {
                Some(w) => h.quick(&name, w, || probe_unit(&b.bytes, lim)),
                None => {
                    let r = probe(&b.bytes, lim);
                    let sound = match &r {
                        Ok(s) => s.iter().find(|x| x.0 == "VisioDocument").is_some_and(|x| x.1.starts_with(kit::SIGNATURE)),
                        Err(e) => *e == archive::Error::Invalid,
                    };
                    h.check(&name, sound, format!("tolerated either way: {:?}", r.map(|s| s.len())));
                }
            }
        }
    }
    // the limits
    let b = fixture(3, true, 1).bytes;
    let n = b.len();
    h.quick("vss/cfb/limits/file", Err(archive::Error::Limit), || probe_unit(&b, cfb::Limits { file_bytes: n - 1, ..lim }));
    h.quick("vss/cfb/limits/file-exact", Ok(()), || probe_unit(&b, cfb::Limits { file_bytes: n, ..lim }));
    h.quick("vss/cfb/limits/stream", Err(archive::Error::Limit), || probe_unit(&b, cfb::Limits { stream_bytes: 4999, ..lim }));
    h.quick("vss/cfb/limits/entries", Err(archive::Error::Limit), || probe_unit(&b, cfb::Limits { entries: 3, ..lim }));
    h.quick("vss/cfb/limits/entries-exact", Ok(()), || probe_unit(&b, cfb::Limits { entries: 7, ..lim }));
    // a directory far larger than the entries allowed: refused before it is believed
    let many: Vec<cfbkit::Node> = (0..5000).map(|i| cfbkit::stream(&format!("S{i}"), &[])).collect();
    let big = Cfb { kids: many, ..Default::default() }.build().bytes;
    h.quick("vss/cfb/limits/many-entries", Err(archive::Error::Limit), || probe_unit(&big, lim));
    // nothing but a header, a header of a larger file
    let mut head = fixture(3, false, 1).bytes;
    head.truncate(512);
    h.quick("vss/cfb/header-only", Err(archive::Error::Invalid), || probe_unit(&head, lim));
}

// ---- the container: vss::read

fn container(h: &mut H) {
    let lim = Limits::VSDX;
    for (version, big) in [(3u16, false), (3, true), (4, false), (4, true)] {
        let b = fixture(version, big, 1).bytes;
        let r = read(&b, &lim);
        let m = r.as_ref().map(masters).unwrap_or_default();
        h.check(&format!("vss/container/control/v{version}{}", if big { "-big" } else { "" }), m.len() == 2, format!("{} bytes: {}, {m:?}", b.len(), res(&r)));
    }
    for version in [3u16, 4] {
        for v in Hostile::ALL {
            let (big, min_fat) = v.needs();
            let mut b = fixture(version, big, min_fat);
            if !v.apply(&mut b) {
                continue;
            }
            let name = format!("vss/container/v{version}/{}", v.name());
            match want(v).1 {
                Some(w) => h.quick(&name, w, || unit(&b.bytes, &lim)),
                None => {
                    let r = unit(&b.bytes, &lim);
                    h.check(&name, r.is_ok() || r == BAD_DOC, format!("tolerated either way: {r:?}"));
                }
            }
        }
    }
    let file = |kids: Vec<cfbkit::Node>| Cfb { kids, ..Default::default() }.build().bytes;
    let doc = kit::with(Vec::new());
    h.quick("vss/container/not-cfb", NO_CFB, || unit(&b"Visio (TM) Drawing\r\n".repeat(40), &lim));
    h.quick("vss/container/empty", NO_CFB, || unit(&[], &lim));
    h.quick("vss/container/no-visiodocument", Err(Error::NotVisio), || unit(&file(vec![cfbkit::stream("WordDocument", &doc)]), &lim));
    h.quick("vss/container/only-in-storage", Err(Error::NotVisio), || unit(&file(vec![cfbkit::storage("Sub", vec![cfbkit::stream("VisioDocument", &doc)])]), &lim));
    h.quick("vss/container/storage", BAD_DOC, || unit(&file(vec![cfbkit::storage("VisioDocument", vec![cfbkit::stream("x", &doc)])]), &lim));
    h.quick("vss/container/lower-case", Ok(()), || unit(&file(vec![cfbkit::stream("visiodocument", &doc)]), &lim));
    h.quick("vss/container/empty-stream", bin(0, "no VisioDocument stream"), || unit(&file(vec![cfbkit::stream("VisioDocument", &[])]), &lim));
    // the limits: the file by Limits::zip.archive_bytes, the stream by part_bytes
    let b = fixture(3, true, 1).bytes;
    let mut small = lim;
    small.zip.archive_bytes = b.len() - 1;
    h.quick("vss/container/limits/file", LIMIT_FILE, || unit(&b, &small));
    small.zip.archive_bytes = b.len();
    h.quick("vss/container/limits/file-exact", Ok(()), || unit(&b, &small));
    let mut small = lim;
    small.zip.part_bytes = 4000;
    h.quick("vss/container/limits/stream", LIMIT_FILE, || unit(&b, &small));
}

// ---- the stream VisioDocument

fn stream(h: &mut H) {
    let lim = Limits::VSDX;
    let q = |h: &mut H, name: &str, doc: &[u8], want: Result<(), Error>, lim: &Limits| {
        let f = kit::file(doc);
        h.quick(name, want, || unit(&f, lim));
    };
    // the control: two masters, their names, prompts and a shape each
    let good = kit::with(Vec::new());
    let r = read(&kit::file(&good), &lim);
    let m = r.as_ref().map(masters).unwrap_or_default();
    let want = [(String::from("Box"), String::from("Kasten"), 1), (String::from("Circle"), String::from("Kreis"), 1)];
    h.check("vss/stream/control", m == want, format!("{} bytes: {}, {m:?}", good.len(), res(&r)));
    // the same tree stored three ways: the same model
    let dumps: Vec<String> = [Pack::None, Pack::Literals, Pack::Repeats]
        .iter()
        .map(|&p| {
            let mut root = kit::stencil(&[("Kasten", "Box"), ("Kreis", "Circle")]);
            repack(&mut root, p);
            read(&kit::file(&Doc::new(root).build()), &lim).map(|l| crate::dump::dump(&l.doc)).unwrap_or_else(|e| format!("{e}"))
        })
        .collect();
    h.check("vss/stream/lzss/ways", dumps[0] == dumps[1] && dumps[1] == dumps[2] && !dumps[0].is_empty(), format!("stored, literals, repeats: {} {} {} bytes of model", dumps[0].len(), dumps[1].len(), dumps[2].len()));
    // long names: references across the ring's end and into what they write
    let long: String = (0..3000).map(|i| char::from(b'a' + ((i / 7 + i % 3) % 26) as u8)).collect();
    let mut root = kit::stencil(&[(long.as_str(), "Long"), ("Kreis", "Circle")]);
    repack(&mut root, Pack::Repeats);
    let doc = Doc::new(root).build();
    let r = read(&kit::file(&doc), &lim);
    let m = r.as_ref().map(masters).unwrap_or_default();
    h.check("vss/stream/lzss/ring", m.first().is_some_and(|x| x.1 == long), format!("a name of {} characters in {} bytes of stream: {}", long.len(), doc.len(), res(&r)));
    // the header
    let mut d = good.clone();
    d[0] = b'X';
    q(h, "vss/stream/signature", &d, bin(0, "no VisioDocument stream"), &lim);
    q(h, "vss/stream/short", &good[..0x30], bin(0, "no VisioDocument stream"), &lim);
    for (v, what) in [(6u16, "version 6 (Visio 2000/2002)"), (12, "version"), (0, "version")] {
        let mut d = good.clone();
        d[0x1A..0x1C].copy_from_slice(&v.to_le_bytes());
        q(h, &format!("vss/stream/version-{v}"), &d, bin(0x1A, what), &lim);
    }
    let mut d = good.clone();
    let past = d.len() as u32 + 10;
    d[0x2C..0x30].copy_from_slice(&past.to_le_bytes());
    q(h, "vss/stream/trailer-outside", &d, bin(past as u64, "a pointer outside the stream"), &lim);
    let mut d = good.clone();
    d[0x2C..0x34].copy_from_slice(&[0xF0, 0xFF, 0xFF, 0xFF, 0xF0, 0xFF, 0xFF, 0xFF]);
    q(h, "vss/stream/trailer-overflow", &d, bin(0xFFFF_FFF0, "a pointer outside the stream"), &lim);
    // pointers and lists
    let far = Ptr { ty: 0x99, word: 0, offset: 0x7FFF_0000, length: 16, format: kit::F_PLAIN };
    q(h, "vss/stream/pointer-outside", &kit::with(vec![Entry::Ptr(far)]), bin(0x7FFF_0000, "a pointer outside the stream"), &lim);
    let raw_list = |b: &[u8]| kit::with(vec![Stream::raw(0x99, kit::F_LIST, b).s()]);
    let w = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let at = |d: &[u8]| -> u64 { find_raw(d) };
    let d = raw_list(&w(&[8, 0, 1000, 0]));
    q(h, "vss/stream/list-longer", &d, bin(at(&d), "a list longer than its stream"), &lim);
    let d = raw_list(&w(&[0, 0, 0, 0]));
    q(h, "vss/stream/list-head-zero", &d, bin(at(&d), "a list's head"), &lim);
    let d = raw_list(&w(&[0xFFFF, 0, 0, 0]));
    q(h, "vss/stream/list-head-beyond", &d, bin(at(&d), "a list's head"), &lim);
    let d = raw_list(&w(&[0xFFFF_FFFF, 0, 0, 0]));
    q(h, "vss/stream/list-head-max", &d, bin(at(&d), "a list's head"), &lim);
    let d = raw_list(&[1, 2]);
    q(h, "vss/stream/list-without-head", &d, bin(at(&d), "a list without head"), &lim);
    let d = raw_list(&w(&[8, 16, 0, 0]));
    q(h, "vss/stream/list-order", &d, bin(at(&d), "a list's order"), &lim);
    let d = raw_list(&w(&[8, 0xFFFF_FFFF, 0, 0]));
    q(h, "vss/stream/list-order-max", &d, bin(at(&d), "a list's order"), &lim);
    let d = raw_list(&w(&[8, 2, 0, 0, 7, 0xFFFF_FFFF]));
    q(h, "vss/stream/list-order-unknown", &d, Ok(()), &lim);
    q(h, "vss/stream/gaps", &kit::with(vec![Entry::Null, Entry::Null, Stream::blob(0x99, b"x").s(), Entry::Null]), Ok(()), &lim);
    let empty = Ptr { ty: 0x99, word: 0, offset: 0x40, length: 0, format: kit::F_PLAIN };
    q(h, "vss/stream/empty-twice", &kit::with(vec![Entry::Ptr(empty), Entry::Ptr(empty)]), Ok(()), &lim);
    let header = Ptr { ty: 0x99, word: 0, offset: 0, length: 0x40, format: kit::F_PLAIN };
    q(h, "vss/stream/into-header", &kit::with(vec![Entry::Ptr(header)]), Ok(()), &lim);
    // the same stream twice: refused, so no tree grows beyond the file
    let d = kit::with(vec![Stream::blob(0x99, b"twice, please").s(), Entry::Same(2)]);
    let r = unit(&kit::file(&d), &lim);
    h.check("vss/stream/twice", matches!(r, Err(Error::Binary { what: "a stream pointed at twice", .. })), format!("{r:?}"));
    // a list that is its own child (made-up pointer at itself): pointed at twice as well
    let mut d = kit::with(vec![Stream::list(0x99, vec![Entry::Ptr(Ptr::default())]).s()]);
    let (lo, ll) = find_list(&d);
    // the list's single pointer at 0x40 + 16 bytes into it
    let p = Ptr { ty: 0x99, word: 0, offset: lo, length: ll, format: kit::F_LIST };
    let pos = lo as usize + 16;
    d[pos..pos + 18].copy_from_slice(&p.bytes());
    let r = unit(&kit::file(&d), &lim);
    h.check("vss/stream/self", matches!(r, Err(Error::Binary { what: "a stream pointed at twice", .. })), format!("{r:?}"));
    // nesting: 12 levels below the trailer are read, 13 are refused (no deeper recursion)
    for (levels, ok) in [(12usize, true), (13, false), (40, false)] {
        let d = kit::with(vec![deep(levels)]);
        let r = unit(&kit::file(&d), &lim);
        let good = if ok { r.is_ok() } else { matches!(r, Err(Error::Binary { what: "pointers nested too deeply", .. })) };
        h.check(&format!("vss/stream/depth-{levels}"), good, format!("{r:?}"));
    }
    // LZSS: a bomb against small limits, the same within them; a cut reference ends the data
    let zip = |part: usize, total: usize| {
        let mut l = lim;
        l.zip.part_bytes = part;
        l.zip.total_bytes = total;
        l
    };
    let bomb = kit::with(vec![Stream::raw(0x99, kit::F_PLAIN | kit::COMPRESSED, &kit::bomb(4 << 20)).s()]);
    q(h, "vss/stream/bomb", &bomb, Err(Error::Limit("stream")), &zip(1 << 20, 64 << 20));
    q(h, "vss/stream/bomb-within", &bomb, Ok(()), &lim);
    let bombs: Vec<Entry> = (0..3).map(|_| Stream::raw(0x99, kit::F_PLAIN | kit::COMPRESSED, &kit::bomb(600 << 10)).s()).collect();
    q(h, "vss/stream/bombs-total", &kit::with(bombs), Err(Error::Limit("stream")), &zip(1 << 20, 1 << 20));
    let plain: Vec<Entry> = (0..3).map(|i| Stream::blob(0x99, &noise(4000, 11 + i)).s()).collect();
    q(h, "vss/stream/plain-total", &kit::with(plain), Err(Error::Limit("stream")), &zip(1 << 20, 10_000));
    let big_one = kit::with(vec![Stream::blob(0x99, &noise(20_000, 13)).s()]);
    let r = unit(&kit::file(&big_one), &zip(19_000 + big_one.len(), 64 << 20));
    h.check("vss/stream/plain-part", r.is_ok(), format!("a plain stream within part_bytes: {r:?}"));
    q(h, "vss/stream/lzss-cut", &kit::with(vec![Stream::raw(0x99, kit::F_PLAIN | kit::COMPRESSED, &[0x00, 0x05]).s()]), Ok(()), &lim);
    // noise under every format, packed and not: Ok or an error, in time
    let mut worst = 0u64;
    let mut seen = Vec::new();
    for (k, format) in [kit::F_PLAIN, kit::F_LIST, kit::F_CHUNKS, 0xC0, 0x80, 0x00].into_iter().enumerate() {
        for packed in [0, kit::COMPRESSED] {
            let f = kit::file(&kit::with(vec![Stream::raw(kit::T_STYLES, format | packed, &noise(3000, 17 + k as u64)).s()]));
            let t = plat::time_us();
            let r = unit(&f, &lim);
            worst = worst.max(plat::time_us().wrapping_sub(t));
            seen.push(match r {
                Ok(()) => String::from("ok"),
                Err(Error::Binary { what, .. }) => String::from(what),
                Err(e) => format!("{e:?}"),
            });
        }
    }
    h.check("vss/stream/noise", worst < QUICK_US, format!("noise as styles under 6 formats, stored and packed: {seen:?}, the slowest {} ms", worst / 1000));
    // more streams than Limits::objects + sections
    let mut few = lim;
    few.objects = 4;
    few.sections = 4;
    q(h, "vss/stream/streams", &good, Err(Error::Limit("objects")), &few);
}

// the offset of the first stream of type 0x99 (a pointer in the trailer list)
fn find_raw(d: &[u8]) -> u64 {
    find_list(d).0 as u64
}

// the (offset, length) of the trailer's last pointer
fn find_list(d: &[u8]) -> (u32, u32) {
    let u = |p: usize| u32::from_le_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]]);
    let (off, _len) = (u(0x2C) as usize, u(0x30));
    let count = u(off + 8) as usize;
    let p = off + 16 + (count - 1) * 18;
    (u(p + 8), u(p + 12))
}

// a chain of lists whose deepest stream lies `levels` below the trailer
fn deep(levels: usize) -> Entry {
    let mut s = Stream::blob(0x99, b"deep");
    for _ in 1..levels {
        s = Stream::list(0x99, vec![s.s()]);
    }
    s.s()
}

// every stream of the tree stored as `p`
fn repack(s: &mut Stream, p: Pack) {
    s.pack = p;
    if let kit::Body::List { items, .. } = &mut s.body {
        for e in items {
            if let Entry::S(c) = e {
                repack(c, p);
            }
        }
    }
}

// ---- chunk streams (as the styles' stream, which the reader parses first)

fn chunks(h: &mut H) {
    let lim = Limits::VSDX;
    let styles = |b: &[u8]| kit::file(&kit::with(vec![Stream::chunks(kit::T_STYLES, b).s()]));
    let q = |h: &mut H, name: &str, b: &[u8], want: Result<(), Error>, lim: &Limits| {
        let f = styles(b);
        h.quick(name, want, || unit(&f, lim));
    };
    let mut ok = kit::chunk(0x4A, 1, 0, &[0; 12]);
    ok.extend_from_slice(&kit::SEP);
    ok.extend(kit::list_chunk(0x68, 2, 1, 1, &[1, 2, 3], &[3]));
    ok.extend(kit::chunk(0x9B, 3, 2, &kit::xform(1.0, 1.0)));
    ok.extend_from_slice(&[0; 32]);
    q(h, "vss/chunks/control", &ok, Ok(()), &lim);
    q(h, "vss/chunks/longer", &kit::chunk_raw(0x4A, 1, 0, 1000, 0, 0, &[7; 10]), bin(0, "a chunk longer than its stream"), &lim);
    q(h, "vss/chunks/longer-max", &kit::chunk_raw(0x4A, 1, 1, 0xFFFF_FFFF, 0, 0, &[7; 10]), bin(0, "a chunk longer than its stream"), &lim);
    let mut later = ok.clone();
    let at = later.len() as u64;
    later.extend(kit::chunk_raw(0x4A, 9, 0, 50, 0, 0, &[1; 20]));
    q(h, "vss/chunks/longer-later", &later, bin(at, "a chunk longer than its stream"), &lim);
    q(h, "vss/chunks/cut", &[0x4A, 0, 0, 0, 1, 0, 0, 0, 0, 0], bin(0, "a cut chunk"), &lim);
    q(h, "vss/chunks/level", &kit::chunk(0x4A, 1, 4, &[]), bin(0, "a chunk's header"), &lim);
    q(h, "vss/chunks/type-zero", &kit::chunk_raw(0, 1, 0, 0, 0, 0, &[]), bin(0, "a chunk's header"), &lim);
    q(h, "vss/chunks/list", &kit::chunk_raw(0x4A, 1, 1, 0, 0, 0, &[100, 0, 0, 0, 0, 0, 0, 0]), bin(0, "a chunk's list"), &lim);
    q(h, "vss/chunks/list-ids", &kit::chunk_raw(0x4A, 1, 1, 4, 0, 0, &[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 4]), bin(0, "a chunk's list"), &lim);
    // levels that jump (0 to 3 and back) nest without trouble
    let mut jumps = Vec::new();
    for (i, lv) in [0u16, 3, 1, 3, 0, 2, 2, 0].iter().enumerate() {
        jumps.extend(kit::chunk(0x4A + i as u32, i as u32, *lv, &[1, 2]));
    }
    q(h, "vss/chunks/levels", &jumps, Ok(()), &lim);
    // more chunks than Limits::items
    let mut many = Vec::new();
    for i in 0..20 {
        many.extend(kit::chunk(0x4A, i, 0, &[1, 2, 3]));
    }
    let mut few = lim;
    few.items = 10;
    q(h, "vss/chunks/items", &many, Err(Error::Limit("items")), &few);
    // many chunks in time
    let mut lots = Vec::new();
    for i in 0..100_000 {
        lots.extend(kit::chunk(0x4A, i, (i % 3) as u16, &[1, 2, 3]));
    }
    let f = kit::file(&kit::with(vec![Stream::chunks(kit::T_STYLES, &lots).packed(Pack::Literals).s()]));
    let t0 = plat::time_us();
    let r = unit(&f, &lim);
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("vss/chunks/many", r.is_ok() && ms < 5000, format!("100000 chunks: {r:?} in {ms} ms"));
}

// ---- cut and mutated files

fn cut(h: &mut H) {
    let lim = Limits::VSDX;
    let tr = trace();
    let file = fixture(3, false, 1).bytes;
    let mut worst = 0u64;
    let mut oks = 0;
    let t0 = plat::time_us();
    for n in 0..=file.len() {
        if tr {
            out(&format!("trace vss/cut/file {n}\n"));
        }
        let t = plat::time_us();
        oks += read(&file[..n], &lim).is_ok() as usize;
        worst = worst.max(plat::time_us().wrapping_sub(t));
    }
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("vss/cut/file", worst < QUICK_US, format!("{} prefixes of the compound file: {oks} read, the slowest {} ms, all {ms} ms", file.len() + 1, worst / 1000));
    let doc = kit::with(Vec::new());
    let (mut worst, mut oks) = (0u64, 0);
    let t0 = plat::time_us();
    for n in 0..=doc.len() {
        if tr {
            out(&format!("trace vss/cut/stream {n}\n"));
        }
        let f = kit::file(&doc[..n]);
        let t = plat::time_us();
        oks += read(&f, &lim).is_ok() as usize;
        worst = worst.max(plat::time_us().wrapping_sub(t));
    }
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check("vss/cut/stream", worst < QUICK_US, format!("{} prefixes of the stream VisioDocument: {oks} read, the slowest {} ms, all {ms} ms", doc.len() + 1, worst / 1000));
    // mutations: one to four bytes changed (any value, a bit, 0x00, 0xFF), in the stream and in
    // the whole file
    let (s, w, o) = mutate(&doc, 4000, 0x5EED, kit::file, &lim, "vss/cut/mutate-stream");
    h.check("vss/cut/mutate-stream", w < QUICK_US, format!("4000 mutated streams: {o} read, the slowest {} ms, all {} ms", w / 1000, s / 1000));
    let (s, w, o) = mutate(&file, 3000, 0xF11E, |d| d.to_vec(), &lim, "vss/cut/mutate-file");
    h.check("vss/cut/mutate-file", w < QUICK_US, format!("3000 mutated files: {o} read, the slowest {} ms, all {} ms", w / 1000, s / 1000));
}

// `count` mutated copies of `base`, each wrapped by `wrap` and read -> (all us, slowest us, read)
fn mutate(base: &[u8], count: usize, seed: u64, wrap: impl Fn(&[u8]) -> Vec<u8>, lim: &Limits, name: &str) -> (u64, u64, usize) {
    let tr = trace();
    let mut rng = Rng(seed);
    let (mut all, mut worst, mut oks) = (0u64, 0u64, 0usize);
    for k in 0..count {
        let mut d = base.to_vec();
        for _ in 0..1 + rng.below(4) {
            let i = rng.below(d.len());
            d[i] = match rng.below(4) {
                0 => rng.next() as u8,
                1 => d[i] ^ (1 << rng.below(8)),
                2 => 0,
                _ => 0xFF,
            };
        }
        if tr {
            out(&format!("trace {name} {k}: {}\n", kit::hex(&d)));
        }
        let f = wrap(&d);
        let t = plat::time_us();
        oks += read(&f, lim).is_ok() as usize;
        let us = plat::time_us().wrapping_sub(t);
        all += us;
        worst = worst.max(us);
    }
    (all, worst, oks)
}

// ---- the local files

// the folder of the local .vss files
fn corpus_dir() -> Option<String> {
    plat::env("JFD_VSS_CORPUS").or_else(|| crate::render::find_up("apps/JFDiagrammer/context/files/SWDB_DE"))
}

// the folder of Visio's .vssx of the same files
fn reference_dir() -> Option<String> {
    let d = plat::env("JFD_VSS_REFERENCE").or_else(|| {
        let home = plat::env("USERPROFILE").or_else(|| plat::env("HOME"))?;
        Some(format!("{}/Documents/JFLib-jd17-visio-check/referenz", home.replace('\\', "/")))
    })?;
    let any = plat::fs::read_dir(&d)?.iter().any(|e| e.name.to_ascii_lowercase().ends_with(".vssx"));
    any.then_some(d)
}

fn files(dir: &str, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = plat::fs::read_dir(dir).unwrap_or_default().into_iter().map(|e| e.name).filter(|n| n.to_ascii_lowercase().ends_with(ext)).collect();
    v.sort();
    v
}

/// (masters, shapes, cells, sections, rows) of a document's masters
fn counts(doc: &Document) -> [usize; 5] {
    let mut c = [0usize; 5];
    let sheet = |s: &diagram_core::Sheet, c: &mut [usize; 5]| {
        c[2] += s.cells.len();
        c[3] += s.sections.len();
        for sec in &s.sections {
            c[2] += sec.cells.len();
            c[4] += sec.rows.len();
            for r in &sec.rows {
                c[2] += r.cells.len();
            }
        }
    };
    for m in doc.masters() {
        let Some(ms) = doc.master(m) else { continue };
        c[0] += 1;
        sheet(&ms.sheet, &mut c);
        for slot in ms.shapes.iter() {
            let Some(d) = ms.shapes.get(slot) else { continue };
            c[1] += 1;
            sheet(&d.sheet, &mut c);
        }
    }
    c
}

fn corpus(h: &mut H) {
    let Some(dir) = corpus_dir() else {
        out("skip vss/corpus: no apps/JFDiagrammer/context/files/SWDB_DE above the current folder ($JFD_VSS_CORPUS)\n");
        out("skip vss/compare: no corpus\n");
        return;
    };
    let names = files(&dir, ".vss");
    if names.is_empty() {
        out(&format!("skip vss/corpus: no .vss in {dir}\n"));
        return;
    }
    let refs = reference_dir();
    let mut fonts = crate::render::fonts();
    let lim = Limits::VSDX;
    for f in &names {
        let Some(bytes) = plat::read_file(&format!("{dir}/{f}")) else {
            h.check(&format!("vss/corpus/read/{f}"), false, String::from("cannot read the file"));
            continue;
        };
        let t0 = plat::time_us();
        let r = read(&bytes, &lim);
        let ms = plat::time_us().wrapping_sub(t0) / 1000;
        let reference = refs.as_ref().and_then(|d| plat::read_file(&format!("{d}/{}x", f))).and_then(|b| vsdx::read(&b, &lim).ok());
        if h.on("vss/corpus") {
            let l = match &r {
                Ok(l) => {
                    let c = counts(&l.doc);
                    let warn: Vec<String> = l.warnings.iter().take(6).map(|w| format!("{w:?}")).collect();
                    h.check(&format!("vss/corpus/read/{f}"), true, format!("{} KB in {ms} ms: {} masters, {} shapes, {} cells, {} sections, {} rows; {} warnings {warn:?}", bytes.len() / 1024, c[0], c[1], c[2], c[3], c[4], l.warnings.len() + l.more_warnings));
                    Some(l)
                }
                Err(e) => {
                    h.check(&format!("vss/corpus/read/{f}"), false, format!("{} KB in {ms} ms: {e}", bytes.len() / 1024));
                    None
                }
            };
            if let Some(l) = l {
                // the same masters (and shapes) as in Visio's .vssx of the file
                if let Some(rl) = &reference {
                    let (a, b) = (counts(&l.doc), counts(&rl.doc));
                    let mut na: Vec<String> = masters(l).into_iter().map(|m| m.0).collect();
                    let mut nb: Vec<String> = masters(rl).into_iter().map(|m| m.0).collect();
                    na.sort();
                    nb.sort();
                    h.check(&format!("vss/corpus/masters/{f}"), na == nb && a[1] == b[1], format!("masters {} / Visio {}, shapes {} / {}, cells {} / {}, rows {} / {}", a[0], b[0], a[1], b[1], a[2], b[2], a[4], b[4]));
                }
                // written as .vssx and read back: the same model, the same bytes again
                crate::vdx::round_trip(h, &format!("vss/corpus/vssx/{f}"), &l.doc, Format { kind: DocKind::Stencil, macros: false });
            }
            corpus_hostile(h, f, &bytes);
        }
        if h.on("vss/compare") {
            match (&r, &reference) {
                (Ok(l), Some(rl)) => compare(h, f, &l.doc, &rl.doc, &mut fonts),
                (_, None) => out(&format!("skip vss/compare/{f}: no {f}x in the reference folder ($JFD_VSS_REFERENCE)\n")),
                (Err(e), _) => out(&format!("skip vss/compare/{f}: the .vss does not read ({e})\n")),
            }
        }
    }
}

// a local file's stream VisioDocument cut and mutated (in a container of its own): never a panic
fn corpus_hostile(h: &mut H, f: &str, bytes: &[u8]) {
    let lim = Limits::VSDX;
    let doc = cfb::File::read(bytes, cfb::Limits::default()).ok().and_then(|c| c.find(&["VisioDocument"]).and_then(|i| c.stream(i).ok()));
    let Some(doc) = doc else {
        h.check(&format!("vss/corpus/cut/{f}"), false, String::from("no stream VisioDocument"));
        return;
    };
    let tr = trace();
    let (mut worst, mut oks) = (0u64, 0usize);
    let t0 = plat::time_us();
    let step = (doc.len() / 150).max(1);
    let mut n = 0;
    while n < doc.len() {
        if tr {
            out(&format!("trace vss/corpus/cut/{f} {n}\n"));
        }
        let c = kit::file(&doc[..n]);
        let t = plat::time_us();
        oks += read(&c, &lim).is_ok() as usize;
        worst = worst.max(plat::time_us().wrapping_sub(t));
        n += step;
    }
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    h.check(&format!("vss/corpus/cut/{f}"), worst < QUICK_US, format!("{} prefixes of {} bytes: {oks} read, the slowest {} ms, all {ms} ms", doc.len().div_ceil(step), doc.len(), worst / 1000));
    let (s, w, o) = mutate(&doc, 150, 0xC0FFEE ^ doc.len() as u64, kit::file, &lim, &format!("vss/corpus/mutate/{f}"));
    h.check(&format!("vss/corpus/mutate/{f}"), w < QUICK_US, format!("150 mutated streams: {o} read, the slowest {} ms, all {} ms", w / 1000, s / 1000));
}

// ---- pictures: each master of the .vss against the same master of Visio's .vssx

// a master's size in mm (its master shape's Width and Height)
fn size_mm(doc: &Document, m: MasterRef) -> (f64, f64) {
    let Some(slot) = doc.master(m).and_then(|x| x.master_shape()) else { return (0.0, 0.0) };
    let sheet = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
    let num = |n: &str| -> f64 {
        let Some(sym) = doc.sym(n) else { return 0.0 };
        let r = doc.resolve(sheet, &CellAddr::single(sym));
        r.cached.or(r.cell).and_then(|c| c.value.num()).unwrap_or(0.0) * 25.4
    };
    (num("Width"), num("Height"))
}

// the page of the first pass: the master's pin in its middle
const PROBE_MM: f64 = 400.0;

// a square drawing page of `side` mm with one instance of master `m` of `src` (its pin at `at`
// mm, its own size), calculated
fn placed(src: &Document, m: MasterRef, side: f64, at: (f64, f64)) -> Option<(Document, diagram_core::PageRef)> {
    use diagram_core::instance;
    let (mut doc, page) = crate::stencil_structs::sheet_doc(side, side)?;
    {
        let mut tx = doc.transaction("place");
        let local = instance::import_master(&mut tx, src, m).ok()?;
        instance::place(&mut tx, page, local, at.0 * MM, at.1 * MM, 0).ok()?;
        let _ = tx.commit();
    }
    {
        let mut tx = doc.transaction("recalc");
        let sheets: Vec<SheetRef> = tx.doc().all_shapes(Owner::Page(page)).map(SheetRef::Shape).collect();
        let _ = Engine::new().recalc(&mut tx, &sheets);
        let _ = tx.commit();
    }
    Some((doc, page))
}

// the box (inches: x0, y0, x1, y1) the drawn master covers with its pin in the middle of the
// first pass's page (cut to that page: an infinite line does not make it endless)
fn extent(src: &Document, m: MasterRef, fonts: &mut FontSet) -> Option<[f64; 4]> {
    let (doc, page) = placed(src, m, PROBE_MM, (PROBE_MM / 2.0, PROBE_MM / 2.0))?;
    let v = PageView::new(&doc, page, fonts, &Options::default());
    let edge = PROBE_MM * MM;
    v.drawing()
        .items
        .iter()
        .map(|i| [i.bbox[0].max(0.0), i.bbox[1].max(0.0), i.bbox[2].min(edge), i.bbox[3].min(edge)])
        .filter(|b| b[0].is_finite() && b[1].is_finite() && b[2] >= b[0] && b[3] >= b[1])
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

// master `m` of `src` drawn at COMPARE_PX on a square page around `frame` (a box of the first
// pass): the same frame for both pictures, so they cover the same paper
fn picture(src: &Document, m: MasterRef, frame: [f64; 4], fonts: &mut FontSet) -> Option<Surface> {
    let side_in = (frame[2] - frame[0]).max(frame[3] - frame[1]).max(0.05) * 1.15 + 0.1;
    let side = side_in * 25.4;
    let centre = ((frame[0] + frame[2]) * 12.7, (frame[1] + frame[3]) * 12.7);
    let at = (PROBE_MM / 2.0 + side / 2.0 - centre.0, PROBE_MM / 2.0 + side / 2.0 - centre.1);
    let (doc, page) = placed(src, m, side, at)?;
    let mut v = PageView::new(&doc, page, fonts, &Options::default());
    Some(v.render(COMPARE_PX, COMPARE_PX))
}

// (pixels differing by more than render::FAR, pixels inked in either picture)
fn ink_diff(a: &Surface, b: &Surface) -> (u64, u64) {
    let (mut far, mut ink) = (0u64, 0u64);
    let lim = crate::render::FAR as i32;
    for y in 0..a.h.min(b.h) as usize {
        let (ra, rb) = (&a.px[y * a.stride as usize..], &b.px[y * b.stride as usize..]);
        for x in 0..a.w.min(b.w) as usize {
            let (pa, pb) = (&ra[x * 4..x * 4 + 3], &rb[x * 4..x * 4 + 3]);
            if pa.iter().chain(pb).any(|&c| (c as i32) < 255 - lim) {
                ink += 1;
            }
            if (0..3).any(|k| (pa[k] as i32 - pb[k] as i32).abs() > lim) {
                far += 1;
            }
        }
    }
    (far, ink)
}

// the lines alone (fills and colours aside): (dark pixels of one picture with no dark pixel in the
// other within one pixel, dark pixels of both)
fn line_diff(a: &Surface, b: &Surface) -> (u64, u64) {
    let (w, hh) = (a.w.min(b.w) as usize, a.h.min(b.h) as usize);
    let dark = |s: &Surface, x: usize, y: usize| {
        let p = &s.px[y * s.stride as usize + x * 4..];
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 3 * 128
    };
    let near = |s: &Surface, x: usize, y: usize| {
        (y.saturating_sub(1)..(y + 2).min(hh)).any(|yy| (x.saturating_sub(1)..(x + 2).min(w)).any(|xx| dark(s, xx, yy)))
    };
    let (mut off, mut all) = (0u64, 0u64);
    for y in 0..hh {
        for x in 0..w {
            for (s, t) in [(a, b), (b, a)] {
                if dark(s, x, y) {
                    all += 1;
                    off += !near(t, x, y) as u64;
                }
            }
        }
    }
    (off, all)
}

fn compare(h: &mut H, f: &str, ours: &Document, theirs: &Document, fonts: &mut FontSet) {
    let dump = plat::env("JFD_VSS_DIFF_DIR");
    let stem = f.rsplit_once('.').map_or(f, |x| x.0);
    let (mut matched, mut lines, mut total, mut missing) = (0usize, 0usize, 0usize, Vec::new());
    for m in ours.masters() {
        let nu = diagram_core::instance::MasterTexts::of(ours, m).name_u;
        let Some(rm) = theirs.masters().find(|&x| diagram_core::instance::MasterTexts::of(theirs, x).name_u == nu) else {
            missing.push(nu);
            continue;
        };
        total += 1;
        let (w, hh) = size_mm(theirs, rm);
        // the frame: what Visio's master covers (ours when Visio's draws nothing)
        let Some(frame) = extent(theirs, rm, fonts).or_else(|| extent(ours, m, fonts)) else {
            out(&format!("info vss/compare/{stem}/{nu}: neither master draws anything\n"));
            continue;
        };
        let (a, b) = (picture(ours, m, frame, fonts), picture(theirs, rm, frame, fonts));
        let drawn = (a.is_some(), b.is_some());
        let (Some(a), Some(b)) = (a, b) else {
            out(&format!("info vss/compare/{stem}/{nu}: not placed (ours {}, Visio's {})\n", drawn.0, drawn.1));
            continue;
        };
        let (far, ink) = ink_diff(&a, &b);
        let share = far as f64 / ink.max(1) as f64;
        let (off, dark) = line_diff(&a, &b);
        let ok = share <= COMPARE_TOLERANCE;
        matched += ok as usize;
        lines += (off as f64 <= COMPARE_TOLERANCE * dark.max(1) as f64) as usize;
        out(&format!(
            "info vss/compare/{stem}/{nu}: {:.1} % of {ink} inked pixels differ, lines {:.1} % of {dark} (Width x Height {w:.1} x {hh:.1} mm, drawn {:.1} x {:.1} mm){}\n",
            share * 100.0,
            off as f64 * 100.0 / dark.max(1) as f64,
            (frame[2] - frame[0]) * 25.4,
            (frame[3] - frame[1]) * 25.4,
            if ok { ", stimmt" } else { "" }
        ));
        if let Some(d) = &dump {
            let safe: String = nu.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            let _ = plat::write_file(&format!("{d}/{stem}-{safe}-vss.png"), &crate::render::png(&a));
            let _ = plat::write_file(&format!("{d}/{stem}-{safe}-visio.png"), &crate::render::png(&b));
        }
    }
    let share = matched as f64 / total.max(1) as f64;
    let what = format!(
        "{matched} of {total} masters match (at most {:.0} % of their inked pixels differ): {:.0} %; their lines alone: {lines} of {total}; {} not in Visio's file {missing:?}",
        COMPARE_TOLERANCE * 100.0,
        share * 100.0,
        missing.len()
    );
    match COMPARE_WANT {
        Some(want) => h.check(&format!("vss/compare/{stem}"), share >= want && missing.is_empty(), what),
        None => out(&format!("info vss/compare/{stem}: {what} (reported only)\n")),
    }
}

// ---- --write-vss-overview DIR [--corpus DIR]

const OV_COL: f64 = 62.0;
const OV_ROW: f64 = 58.0;
const OV_COLS: usize = 6;
const MM: f64 = 1.0 / 25.4;

/// every master of each local .vss (and of each .vssx Visio saved from them) on one sheet
pub fn write_overview(dir: &str, corpus: Option<String>) -> i32 {
    let Some(src) = corpus.or_else(corpus_dir) else {
        out("no local .vss (apps/JFDiagrammer/context/files/SWDB_DE, $JFD_VSS_CORPUS or --corpus DIR)\n");
        return 1;
    };
    let Some(standard) = diagram_io::stencil::build(&crate::templates::files_of("stencils", "standard"), vsdx::Measure::Metric).ok().map(|b| b.doc) else {
        out("error: the standard stencil does not build\n");
        return 1;
    };
    let lim = Limits::VSDX;
    let mut rc = 0;
    let mut sets: Vec<(String, String, bool)> = files(&src, ".vss").into_iter().map(|f| (src.clone(), f, false)).collect();
    if let Some(r) = reference_dir() {
        sets.extend(files(&r, ".vssx").into_iter().map(|f| (r.clone(), f, true)));
    }
    for (folder, f, visio) in sets {
        let Some(bytes) = plat::read_file(&format!("{folder}/{f}")) else {
            out(&format!("error: cannot read {folder}/{f}\n"));
            rc = 1;
            continue;
        };
        let r = if visio { vsdx::read(&bytes, &lim) } else { read(&bytes, &lim) };
        let l = match r {
            Ok(l) => l,
            Err(e) => {
                out(&format!("error: {f}: {e}\n"));
                rc = 1;
                continue;
            }
        };
        let stem = f.rsplit_once('.').map_or(f.as_str(), |x| x.0);
        let path = format!("{dir}/{stem}{}-alle.png", if visio { "-visio" } else { "" });
        let title = format!("{f}{}: {} Master, {} Warnungen", if visio { " (Visio)" } else { "" }, l.doc.masters().count(), l.warnings.len() + l.more_warnings);
        if !overview(&l.doc, &standard, &title, &path) {
            rc = 1;
        }
    }
    rc
}

// the masters of `st` in columns, each at its own size (shrunk to its cell when larger), its name
// under it, hidden masters last with "(verborgen)"; placed by reference, so masters without NameU
// or with the same NameU show too
fn overview(st: &Document, standard: &Document, title: &str, path: &str) -> bool {
    use crate::stencil_structs::{none, put, render_page_at, set_single, sheet_doc};
    use diagram_core::instance::{self, MasterTexts};
    let hidden = st.sym("Hidden");
    let is_hidden = |m: MasterRef| st.master(m).and_then(|x| hidden.and_then(|s| diagram_core::attr(&x.header.attrs, s))).is_some_and(|v| st.str(v) == "1");
    let mut masters: Vec<MasterRef> = st.masters().filter(|&m| !is_hidden(m)).collect();
    masters.extend(st.masters().filter(|&m| is_hidden(m)));
    let rows = masters.len().div_ceil(OV_COLS).max(1);
    let (pw, ph) = (OV_COLS as f64 * OV_COL + 16.0, rows as f64 * OV_ROW + 22.0);
    let Some((mut doc, page)) = sheet_doc(pw, ph) else { return false };
    let mut failed = Vec::new();
    {
        let mut tx = doc.transaction("overview");
        put(&mut tx, standard, page, "Text box", (pw * 0.5, ph - 7.0), (pw - 16.0, 8.0), Some(title), &none);
        for (k, &m) in masters.iter().enumerate() {
            let (c, r) = (k % OV_COLS, k / OV_COLS);
            let cx = 8.0 + OV_COL * (c as f64 + 0.5);
            let top = ph - 18.0 - OV_ROW * r as f64;
            let (w, h) = size_mm(st, m);
            let f = (52.0 / w.max(1e-3)).min(38.0 / h.max(1e-3)).min(1.0);
            let texts = MasterTexts::of(st, m);
            let placed = instance::import_master(&mut tx, st, m).and_then(|local| instance::place(&mut tx, page, local, cx * MM, (top - 22.0) * MM, 0));
            match placed {
                Ok(s) => {
                    if f < 1.0 && h > 1e-6 {
                        set_single(&mut tx, SheetRef::Shape(s), "Width", w * f * MM, Some(diagram_core::Unit::Millimeters));
                        set_single(&mut tx, SheetRef::Shape(s), "Height", h * f * MM, Some(diagram_core::Unit::Millimeters));
                    }
                }
                Err(e) => failed.push(format!("{}: {e:?}", texts.name_u)),
            }
            let mut name = String::from(texts.name(0));
            if name.is_empty() {
                name = st.master(m).map_or(String::new(), |x| format!("#{}", x.header.id));
            }
            if is_hidden(m) {
                name.push_str(" (verborgen)");
            }
            put(&mut tx, standard, page, "Text box", (cx, top - 48.0), (OV_COL - 4.0, 8.0), Some(&name), &none);
        }
        let _ = tx.commit();
    }
    if !failed.is_empty() {
        out(&format!("{path}: {} masters not placed: {failed:?}\n", failed.len()));
    }
    render_page_at(&mut doc, page, path, 110.0)
}

// ---- --write-vss-hostile DIR

/// the synthetic files and their hostile variants as .vss files in `dir`
pub fn write_hostile(dir: &str) -> i32 {
    let mut n = 0;
    let mut put = |name: &str, b: &[u8]| -> bool {
        let p = format!("{dir}/{name}.vss");
        let ok = plat::write_file(&p, b);
        n += ok as usize;
        if !ok {
            out(&format!("error: cannot write {p}\n"));
        }
        ok
    };
    for (version, big) in [(3u16, false), (3, true), (4, true)] {
        if !put(&format!("ok-v{version}{}", if big { "-big" } else { "" }), &fixture(version, big, 1).bytes) {
            return 1;
        }
    }
    for v in Hostile::ALL {
        let (big, min_fat) = v.needs();
        let mut b = fixture(3, big, min_fat);
        if v.apply(&mut b) && !put(&format!("cfb-{}", v.name()), &b.bytes) {
            return 1;
        }
    }
    let good = kit::with(Vec::new());
    let mut v6 = good.clone();
    v6[0x1A] = 6;
    let streams: [(&str, Vec<u8>); 4] = [
        ("stream-version-6", v6),
        ("stream-depth-13", kit::with(vec![deep(13)])),
        ("stream-twice", kit::with(vec![Stream::blob(0x99, b"twice").s(), Entry::Same(2)])),
        ("stream-bomb", kit::with(vec![Stream::raw(0x99, kit::F_PLAIN | kit::COMPRESSED, &kit::bomb(4 << 20)).s()])),
    ];
    for (name, d) in &streams {
        if !put(name, &kit::file(d)) {
            return 1;
        }
    }
    out(&format!("wrote {n} files to {dir}\n"));
    0
}
