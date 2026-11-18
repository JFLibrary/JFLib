// roundtrip -- writing the VSDX family (JD5): read -> write -> read gives the same model, the
// second write the same bytes; new documents of the three kinds; hostile models that must not
// crash the writer; the local files (context/files) when they are there, nothing written to disk
//
//   roundtrip/samples   the self-built files of the six formats (samples::file)
//   roundtrip/new       new drawing, template and stencil (vsdx::new_document) in both measures,
//                       written in every format; a drawing with own shapes, connectors, a master
//                       instance, a group and a picture; a stencil with own masters
//   roundtrip/hostile   models the reader never makes: extras at places that do not exist,
//                       broken raw XML, attributes the writer writes itself, characters XML
//                       cannot carry, parts named twice or not at all, macros, missing pictures,
//                       odd numbers, deep groups, stale relationships, tight ZIP limits
//   roundtrip/corpus    Zeichnung2, Zeichnung 3, Favoriten: only numbers printed
//
//   diagramiotest --write-probe DIR   the files for a Visio check: a new empty drawing, a new
//                                     drawing with own shapes and connectors, a new stencil,
//                                     and a round-trip copy of each local file (to DIR only)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{DocKind, Document, ForeignPayload, Owner};
use diagram_io::vsdx::{self, Format, Limits, Loaded, Measure, WriteOptions};

use crate::build;
use crate::dump::{dump, first_difference};
use crate::{out, H};

/// read -> write -> read -> write: the two models and the two writes must agree; the first
/// model and the first write for more checks
pub fn round(h: &mut H, name: &str, bytes: &[u8], opts: &WriteOptions) -> Option<(Loaded, Vec<u8>)> {
    let l1 = match vsdx::read(bytes, &Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            h.check(name, false, format!("read: {e}"));
            return None;
        }
    };
    let b1 = match vsdx::write(&l1.doc, opts) {
        Ok(b) => b,
        Err(e) => {
            h.check(name, false, format!("write: {e}"));
            return None;
        }
    };
    let l2 = match vsdx::read(&b1, &Limits::VSDX) {
        Ok(l) => l,
        Err(e) => {
            h.check(name, false, format!("read back: {e}"));
            return None;
        }
    };
    let (d1, d2) = (dump(&l1.doc), dump(&l2.doc));
    let b2 = vsdx::write(&l2.doc, opts).unwrap_or_default();
    let same = d1 == d2;
    let what = if same { format!("{} bytes, model and second write equal", b1.len()) } else { first_difference(&d1, &d2) };
    h.check(name, same && b1 == b2 && l2.warnings.len() <= l1.warnings.len(), what);
    Some((l1, b1))
}

fn samples(h: &mut H) {
    for ext in ["vsdx", "vstx", "vssx", "vsdm", "vstm", "vssm"] {
        let bytes = crate::samples::file(ext);
        round(h, &format!("roundtrip/samples {ext}"), &bytes, &WriteOptions::DEFAULT);
    }
}

// ---- new documents

fn s(doc: &Document, sym: diagram_core::Sym) -> &str {
    doc.str(sym)
}

// a document written, read back, and written again from what was read: the same bytes
fn new_round(h: &mut H, name: &str, doc: &Document, opts: &WriteOptions) -> Option<Loaded> {
    let b1 = match vsdx::write(doc, opts) {
        Ok(b) => b,
        Err(e) => {
            h.check(name, false, format!("write: {e}"));
            return None;
        }
    };
    let again = vsdx::write(doc, opts).unwrap_or_default();
    h.check(&format!("{name} deterministic"), again == b1, format!("{} bytes", b1.len()));
    let (l, b2) = round(h, &format!("{name} round trip"), &b1, opts)?;
    h.check(&format!("{name} second write"), b2 == b1, format!("{} / {} bytes", b1.len(), b2.len()));
    // nothing of Visio's own content in a new file
    let text = String::from_utf8_lossy(&b1).into_owned();
    h.check(&format!("{name} own content"), !text.contains("Calibri") && !text.contains("Office Theme"), String::from("no Calibri, no Office theme"));
    Some(l)
}

// whether a document has visio/windows.xml with its content type, named from the main part
fn windows_part(d: &Document) -> bool {
    let part = d.parts.iter().any(|p| s(d, p.name) == "/visio/windows.xml" && s(d, p.content_type) == "application/vnd.ms-visio.windows+xml");
    let rel = d.rels.iter().any(|r| {
        s(d, r.source) == "/visio/document.xml"
            && s(d, r.kind) == "http://schemas.microsoft.com/visio/2010/relationships/windows"
            && r.part.is_some_and(|p| s(d, p) == "/visio/windows.xml")
    });
    part && rel
}

fn new_docs(h: &mut H) {
    for kind in [DocKind::Drawing, DocKind::Template, DocKind::Stencil] {
        for measure in [Measure::Metric, Measure::Us] {
            let name = format!("roundtrip/new {} {measure:?}", build::kind_name(kind));
            let doc = vsdx::new_document(kind, measure);
            let Some(l) = new_round(h, &name, &doc, &WriteOptions::DEFAULT) else { continue };
            let d = &l.doc;
            let styles: Vec<(u32, String)> = d.styles().filter_map(|r| d.style(r)).map(|st| (st.header.id, String::from(st.header.name_u.map_or("", |n| s(d, n))))).collect();
            let page = d.pages().next().and_then(|p| d.page(p));
            let part_ok = page.is_some_and(|p| p.part.is_some() == (kind != DocKind::Stencil));
            let width = page.and_then(|p| p.sheet.cell(d.sym("PageWidth")?)).and_then(|c| c.value.num()).unwrap_or(0.0);
            let want_w = if measure == Measure::Metric { 297.0 / 25.4 } else { 11.0 };
            let theme = d.themes.first().is_some_and(|t| s(d, t.part) == "/visio/theme/theme1.xml" && t.data.windows(13).any(|w| w == b"name=\"NetLab\""));
            let main_rel = d.rels.iter().any(|r| s(d, r.source).is_empty() && s(d, r.kind) == "http://schemas.microsoft.com/visio/2010/relationships/document");
            h.check(
                &format!("{name} content"),
                l.warnings.is_empty()
                    && l.format == Format { kind, macros: false }
                    && styles.iter().map(|x| x.1.as_str()).collect::<Vec<_>>() == ["No Style", "Normal", "Connector", "Guide", "Text Only"]
                    && d.page_count() == 1
                    && part_ok
                    && (width - want_w).abs() < 1e-12
                    && d.colors.len() == 24
                    && d.faces.len() == 1
                    && theme
                    && main_rel
                    && d.parts.len() == 3
                    && windows_part(d),
                format!("{} styles, page width {width:.4} in, {} warnings", styles.len(), l.warnings.len()),
            );
        }
    }
    // a document without a windows part (Visio refuses such a file): the writer adds one, named
    // from the main part; read back it is a part like any other and the second write is the same
    for kind in [DocKind::Drawing, DocKind::Stencil] {
        let mut doc = vsdx::new_document(kind, Measure::Metric);
        let win = doc.sym("/visio/windows.xml");
        doc.parts.retain(|p| Some(p.name) != win);
        let name = format!("roundtrip/new {} without windows.xml", build::kind_name(kind));
        if let Some(l) = new_round(h, &name, &doc, &WriteOptions::DEFAULT) {
            h.check(&format!("{name} added"), windows_part(&l.doc) && l.warnings.is_empty(), format!("{} parts", l.doc.parts.len()));
        }
    }
    // every format: a drawing as macro drawing, template and macro template; a stencil as macro stencil
    let drawing = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let stencil = build::stencil(Measure::Metric);
    for (doc, ext) in [(&drawing, "vsdm"), (&drawing, "vstx"), (&drawing, "vstm"), (&stencil, "vssm"), (&stencil, "vssx")] {
        let format = Format::from_extension(ext);
        let opts = WriteOptions { format, ..WriteOptions::DEFAULT };
        let name = format!("roundtrip/new as .{ext}");
        if let Some(l) = new_round(h, &name, doc, &opts) {
            h.check(&format!("{name} format"), Some(l.format) == format, format!("{:?}", l.format));
        }
    }
    // own shapes, connectors, a master instance, a group, a picture
    let doc = build::drawing(Measure::Metric, Some(build::png()));
    if let Some(l) = new_round(h, "roundtrip/new shapes", &doc, &WriteOptions::DEFAULT) {
        let d = &l.doc;
        let page = d.pages().next();
        let shapes = page.map_or(0, |p| d.all_shapes(Owner::Page(p)).count());
        let connects = page.and_then(|p| d.page(p)).map_or(0, |p| p.connects.len());
        let pic = page.and_then(|p| {
            let t = &d.page(p)?.shapes;
            t.iter().filter_map(|x| t.get(x)).find_map(|x| x.foreign.as_ref().map(|f| f.payload.clone()))
        });
        let page_part = page.and_then(|p| d.page(p)).and_then(|p| p.part).map(|p| String::from(s(d, p))).unwrap_or_default();
        let from_page: Vec<&str> = d.rels.iter().filter(|r| s(d, r.source) == page_part).map(|r| s(d, r.kind)).collect();
        h.check(
            "roundtrip/new shapes content",
            l.warnings.is_empty()
                && shapes == 10
                && connects == 4
                && d.masters().count() == 1
                && matches!(pic, Some(ForeignPayload::Part(p)) if s(d, p) == "/visio/media/image1.png")
                && from_page.iter().any(|k| k.ends_with("/master"))
                && from_page.iter().any(|k| k.ends_with("/image")),
            format!("{shapes} shapes, {connects} connects, page part rels {from_page:?}"),
        );
    }
    let doc = build::stencil(Measure::Metric);
    if let Some(l) = new_round(h, "roundtrip/new stencil masters", &doc, &WriteOptions::DEFAULT) {
        let d = &l.doc;
        let names: Vec<&str> = d.masters().filter_map(|m| d.master(m)).filter_map(|m| m.header.name_u).map(|n| s(d, n)).collect();
        let parts = d.masters().filter_map(|m| d.master(m)).all(|m| m.part.is_some() && m.shapes.len() == 1);
        h.check("roundtrip/new stencil masters content", l.warnings.is_empty() && names == ["Kasten", "Ellipse", "Rahmen"] && parts && l.format.kind == DocKind::Stencil, format!("{names:?}"));
    }
}

// ---- the local files

// the local files: --corpus DIR, else the first apps/JFDiagrammer/context/files above the
// current folder (as corpus.rs looks for them)
pub fn corpus_folder(arg: Option<&str>) -> Option<String> {
    if let Some(d) = arg {
        return Some(String::from(d));
    }
    let mut dir = plat::cwd().replace('\\', "/");
    for _ in 0..12 {
        let cand = format!("{dir}/apps/JFDiagrammer/context/files");
        if plat::fs::read_dir(&cand).is_some() {
            return Some(cand);
        }
        match dir.rsplit_once('/') {
            Some((up, _)) if !up.is_empty() => dir = String::from(up),
            _ => break,
        }
    }
    None
}

pub const CORPUS: [&str; 3] = ["Zeichnung2.vsdx", "Zeichnung 3.vsdx", "Favoriten.vssx"];

// read -> write -> read of the local files: only sizes, times and the result are printed,
// nothing is written to disk
fn corpus(h: &mut H, arg: Option<&str>) {
    let Some(dir) = corpus_folder(arg) else {
        out("skip roundtrip/corpus: no apps/JFDiagrammer/context/files above the current folder (--corpus DIR)\n");
        return;
    };
    for f in CORPUS {
        let Some(bytes) = plat::read_file(&format!("{dir}/{f}")) else {
            out(&format!("skip roundtrip/corpus: {f} is missing\n"));
            continue;
        };
        let t0 = plat::time_us();
        if let Some((_, b1)) = round(h, &format!("roundtrip/corpus {f}"), &bytes, &WriteOptions::DEFAULT) {
            let ms = plat::time_us().wrapping_sub(t0) / 1000;
            out(&format!("     {f}: {} KB read, {} KB written, two rounds in {ms} ms\n", bytes.len() / 1024, b1.len() / 1024));
        }
    }
}

pub fn run(h: &mut H, corpus_dir: Option<&str>) {
    if h.on("roundtrip/samples") {
        samples(h);
    }
    if h.on("roundtrip/new") {
        new_docs(h);
    }
    if h.on("roundtrip/hostile") {
        crate::hostile_write::run(h);
    }
    if h.on("roundtrip/corpus") {
        corpus(h, corpus_dir);
    }
}

/// --write-probe DIR: the files for the Visio check
pub fn write_probe(dir: &str, corpus_dir: Option<&str>) -> i32 {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |name: &str, doc: &Document| match vsdx::write(doc, &WriteOptions::DEFAULT) {
        Ok(b) => files.push((String::from(name), b)),
        Err(e) => out(&format!("error: {name}: {e}\n")),
    };
    add("neu-leer.vsdx", &vsdx::new_document(DocKind::Drawing, Measure::Metric));
    add("neu-shapes.vsdx", &build::drawing(Measure::Metric, Some(build::png())));
    add("neu-schablone.vssx", &build::stencil(Measure::Metric));
    if let Some(cdir) = corpus_folder(corpus_dir) {
        for f in CORPUS {
            let Some(bytes) = plat::read_file(&format!("{cdir}/{f}")) else { continue };
            match vsdx::read(&bytes, &Limits::VSDX) {
                Ok(l) => add(&format!("rundlauf-{f}"), &l.doc),
                Err(e) => out(&format!("error: {f}: {e}\n")),
            }
        }
    }
    let mut status = 0;
    for (name, bytes) in files {
        let path = format!("{dir}/{name}");
        if plat::write_file(&path, &bytes) {
            out(&format!("wrote {path} ({} KB)\n", bytes.len() / 1024));
        } else {
            out(&format!("error: cannot write {path}\n"));
            status = 1;
        }
    }
    status
}
