// vss -- the binary formats of Visio 2003-2010 (.vss stencil; .vsd drawing and .vst template
// alike) read into diagram_core's model, import only (DF-30; JD17).  Nothing is ever written in
// these formats (NO-12).
//
//   vss::read(&bytes, DocKind::Stencil, &vsdx::Limits::VSDX) -> Result<vsdx::Loaded, vsdx::Error>
//                       one file; the caller says by its extension what it is (kind_of)
//   vss::kind_of("vss") -> Some(DocKind::Stencil)     (vss vsd vst)
//   vss::sniff(&bytes) -> bool         a compound file with a VisioDocument stream
//
// Microsoft has published no specification of these formats.  This reader comes from our own
// analysis of files (the container is the openly specified compound file, archive::cfb); no
// code of other implementations is used.  What it found:
//   container      a compound file with the stream VisioDocument (and the property sets
//                  SummaryInformation and DocumentSummaryInformation)
//   VisioDocument  a header and a tree of streams, some compressed (stream.rs)
//   objects        chunk streams: a style, a sheet or a shape is a chunk at level 0, its sections
//                  are lists (chunks with child IDs) at level 1, their rows chunks at level 2; a
//                  row holds its cells as fixed fields (a number is a unit byte and a double), a
//                  byte 1, a mask of the cells that hold a value of their own, then blocks: the
//                  formulas (cells.rs, formula.rs)
//   names          a list of strings; masters, styles and named rows point into it
// The objects become the XML of Visio 2003-2010 (the format vdx.rs reads, whose element and cell
// names Microsoft's schema reference documents), and vdx::read builds the model from that: so
// both formats share every rule of the model, and what vdx.rs keeps or leaves out holds here too.
//
// Every file is untrusted input, bounded like a VSDX file (vsdx::Limits): the compound file by
// Limits::zip.archive_bytes, each stream after decompression by Limits::zip.part_bytes, all of
// them by Limits::zip.total_bytes (LZSS cannot grow anything more than nine-fold), the tree of
// streams by its depth and by each stream being used once, chunks by Limits::items, the XML made
// from them by Limits::zip.part_bytes, and then everything vdx::read bounds.  Never a panic, no
// recursion deeper than the format's fixed levels.

mod cells;
mod formula;
mod out;
mod stream;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use archive::cfb;
use diagram_core::DocKind;

use crate::vsdx::{Error, Limits, Loaded, Warning};
use out::Out;
use stream::{u16at, u32at, Body, Chunk, Node, R};

// the types of the streams this reader uses
const T_COLORS: u32 = 0x16;
const T_STYLES: u32 = 0x1A;
const T_STENCILS: u32 = 0x1D;
const T_MASTER: u32 = 0x1E;
const T_DOC_SHEET: u32 = 0x31;
const T_NAMES: u32 = 0x32;
const T_NAME: u32 = 0x33;
const T_FACES: u32 = 0xD8;
const T_FACE: u32 = 0xD7;
const T_NAME_INDEX: u32 = 0xC9;
const T_PAGE_SHEET: u32 = 0x46;
const T_GROUP: u32 = 0x47;
const T_SHAPE: u32 = 0x48;
const T_GUIDE: u32 = 0x4D;
const T_FOREIGN: u32 = 0x4E;
const T_STYLE: u32 = 0x4A;
const T_PROMPT: u32 = 0x0A;

/// the kind of document an extension of the binary formats stands for ("vss", "vsd", "vst";
/// ASCII case ignored)
pub fn kind_of(ext: &str) -> Option<DocKind> {
    match ext.to_ascii_lowercase().as_str() {
        "vsd" => Some(DocKind::Drawing),
        "vss" => Some(DocKind::Stencil),
        "vst" => Some(DocKind::Template),
        _ => None,
    }
}

/// whether `bytes` are a file of these formats: a compound file whose stream VisioDocument starts
/// with Visio's signature (the directory is read, no stream but the start of that one)
pub fn sniff(bytes: &[u8]) -> bool {
    if !cfb::sniff(bytes) {
        return false;
    }
    let Ok(f) = cfb::File::read(bytes, cfb::Limits::default()) else { return false };
    let Some(i) = f.find(&["VisioDocument"]) else { return false };
    f.stream(i).is_ok_and(|s| s.starts_with(stream::SIGNATURE))
}

/// read a binary file of Visio 2003-2010 (version 11) as a document of `kind` (see the module
/// comment); `limits` bound everything (vsdx::Limits::VSDX for real files)
pub fn read(bytes: &[u8], kind: DocKind, limits: &Limits) -> Result<Loaded, Error> {
    let (xml, own, own_more) = convert(bytes, kind, limits)?;
    let mut loaded = crate::vdx::read(xml.as_bytes(), kind, limits)?;
    // our own warnings first: they say what the conversion left out
    let mut w = own;
    let mut more = own_more;
    for x in loaded.warnings.drain(..) {
        if w.len() < limits.warnings {
            w.push(x);
        } else {
            more += 1;
        }
    }
    loaded.warnings = w;
    loaded.more_warnings += more;
    Ok(loaded)
}

/// the XML of Visio 2003-2010 (a .vdx, .vsx or .vtx by `kind`) that a binary file becomes before
/// vdx::read builds the model from it, with the warnings of the conversion (what it left out);
/// for checks and for opening the conversion in other programs
pub fn to_vdx(bytes: &[u8], kind: DocKind, limits: &Limits) -> Result<(String, Vec<Warning>), Error> {
    convert(bytes, kind, limits).map(|(x, w, _)| (x, w))
}

fn convert(bytes: &[u8], kind: DocKind, limits: &Limits) -> Result<(String, Vec<Warning>, usize), Error> {
    if bytes.len() > limits.zip.archive_bytes {
        return Err(Error::Limit("file"));
    }
    let cl = cfb::Limits { file_bytes: limits.zip.archive_bytes, stream_bytes: limits.zip.part_bytes, entries: 4096 };
    let file = cfb::File::read(bytes, cl).map_err(|e| match e {
        archive::Error::Limit => Error::Limit("file"),
        _ => Error::Binary { offset: 0, what: "no compound file" },
    })?;
    let ix = file.find(&["VisioDocument"]).ok_or(Error::NotVisio)?;
    let doc = file.stream(ix).map_err(|e| match e {
        archive::Error::Limit => Error::Limit("file"),
        _ => Error::Binary { offset: 0, what: "the VisioDocument stream" },
    })?;
    let tree = stream::tree(&doc, limits)?;
    let mut b = Bin { lim: limits, out: Out::new(limits.zip.part_bytes), names: Vec::new(), faces: Vec::new(), warnings: Vec::new(), more: 0, chunks: 0 };
    b.document(&tree.root, kind)?;
    let Bin { out, warnings, more, .. } = b;
    Ok((out.s, warnings, more))
}

/// a chunk with the chunks nested in it (by level)
pub(super) struct Obj {
    pub c: Chunk,
    pub kids: Vec<Obj>,
}

/// the chunks of a stream nested by their levels: a chunk belongs to the nearest chunk before it
/// with a lower level
pub(super) fn nest(chunks: &[Chunk]) -> Vec<Obj> {
    let mut top: Vec<Obj> = Vec::new();
    // the open chain: indices down from the last top-level object
    for &c in chunks {
        let mut list = &mut top;
        loop {
            let deeper = matches!(list.last(), Some(o) if o.c.level < c.level);
            if !deeper {
                break;
            }
            list = &mut list.last_mut().expect("checked").kids;
        }
        list.push(Obj { c, kids: Vec::new() });
    }
    top
}

// the reading of one file
pub(super) struct Bin<'l> {
    lim: &'l Limits,
    pub out: Out,
    // the strings of the name list, by their index
    names: Vec<String>,
    // the font names, by their index
    faces: Vec<String>,
    warnings: Vec<Warning>,
    more: usize,
    // chunks read so far
    chunks: usize,
}

impl Bin<'_> {
    pub fn warn(&mut self, what: String) {
        if self.warnings.iter().any(|w| matches!(w, Warning::LeftOut { what: x } if *x == what)) {
            return;
        }
        if self.warnings.len() < self.lim.warnings {
            self.warnings.push(Warning::LeftOut { what });
        } else {
            self.more += 1;
        }
    }

    /// the chunks of a chunk stream, nested
    fn objects(&mut self, node: &Node) -> R<(Vec<Obj>, Vec<u8>)> {
        let Some(s) = node.chunk_bytes() else { return Ok((Vec::new(), Vec::new())) };
        let left = self.lim.items.saturating_sub(self.chunks);
        let cs = stream::chunks(s, left)?;
        self.chunks += cs.len();
        Ok((nest(&cs), s.to_vec()))
    }

    /// a string of the name list
    pub fn name(&self, ix: u32) -> Option<&str> {
        self.names.get(ix as usize).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// a font name by its index
    pub fn face(&self, ix: u32) -> Option<&str> {
        self.faces.get(ix as usize).map(String::as_str).filter(|s| !s.is_empty())
    }

    fn document(&mut self, root: &Node, kind: DocKind) -> R<()> {
        self.read_names(root);
        self.out.raw("<?xml version='1.0' encoding='utf-8'?>\n<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core' xml:space='preserve'>")?;
        let _ = kind;
        self.colors(root)?;
        self.face_names(root)?;
        self.styles(root)?;
        self.doc_sheet(root)?;
        self.masters(root)?;
        self.out.raw("</VisioDocument>")?;
        Ok(())
    }

    // ---- the lists of the document

    // the name list: each name a stream of [u32 uses][UTF-16, NUL-terminated] (a compressed one
    // has its length word before)
    fn read_names(&mut self, root: &Node) {
        let Some(list) = root.kid(T_NAMES) else { return };
        let Body::List { items, .. } = &list.body else { return };
        let count = items.iter().map(|(i, _)| i + 1).max().unwrap_or(0);
        let mut names = alloc::vec![String::new(); count.min(self.lim.items)];
        for (i, n) in items {
            if n.ty != T_NAME || *i >= names.len() {
                continue;
            }
            if let Some(b) = n.blob() {
                names[*i] = utf16z(b.get(4..).unwrap_or(&[]));
            }
        }
        self.names = names;
    }

    // the colour table: [u8][u8][u16 count][count x RGBA]
    fn colors(&mut self, root: &Node) -> R<()> {
        let Some(b) = root.kid(T_COLORS).and_then(Node::blob) else { return Ok(()) };
        let n = u16at(b, 2).unwrap_or(0) as usize;
        self.out.raw("<Colors>")?;
        for i in 0..n {
            let Some(c) = b.get(4 + 4 * i..8 + 4 * i) else { break };
            self.out.open("ColorEntry");
            self.out.attr("IX", &format!("{i}"));
            self.out.attr("RGB", &format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]));
            self.out.end_empty()?;
        }
        self.out.raw("</Colors>")
    }

    // the fonts: each a stream of [u32 flags][32 UTF-16 name][4 x u32 Unicode ranges]
    // [2 x u32 character sets][10 bytes Panose]
    fn face_names(&mut self, root: &Node) -> R<()> {
        let Some(list) = root.kid(T_FACES) else { return Ok(()) };
        let Body::List { items, .. } = &list.body else { return Ok(()) };
        self.out.raw("<FaceNames>")?;
        let mut faces = Vec::new();
        for (i, n) in items {
            if n.ty != T_FACE {
                continue;
            }
            let Some(b) = n.blob() else { continue };
            let flags = u32at(b, 0).unwrap_or(0);
            let name = utf16z(b.get(4..68).unwrap_or(&[]));
            while faces.len() < *i {
                faces.push(String::new());
            }
            faces.push(name.clone());
            if name.is_empty() {
                continue;
            }
            let ranges: Vec<String> = (0..4).map(|k| format!("{}", u32at(b, 68 + 4 * k).unwrap_or(0) as i32)).collect();
            let sets: Vec<String> = (0..2).map(|k| format!("{}", u32at(b, 84 + 4 * k).unwrap_or(0) as i32)).collect();
            let panose: Vec<String> = (0..10).map(|k| format!("{}", b.get(92 + k).copied().unwrap_or(0))).collect();
            self.out.open("FaceName");
            self.out.attr("ID", &format!("{i}"));
            self.out.attr("Name", &name);
            self.out.attr("UnicodeRanges", &ranges.join(" "));
            self.out.attr("CharSets", &sets.join(" "));
            self.out.attr("Panos", &panose.join(" "));
            self.out.attr("Flags", &format!("{flags}"));
            self.out.end_empty()?;
        }
        self.faces = faces;
        self.out.raw("</FaceNames>")
    }

    // the styles: a chunk stream of style objects
    fn styles(&mut self, root: &Node) -> R<()> {
        let Some(node) = root.kid(T_STYLES) else { return Ok(()) };
        let (objs, s) = self.objects(node)?;
        // the stream's own object (type 0x1A) holds the styles and their name index
        let Some(top) = objs.iter().find(|o| o.c.ty == T_STYLES) else { return Ok(()) };
        let mut names: Vec<(u32, u32, u32)> = Vec::new();
        for k in &top.kids {
            if k.c.ty == T_NAME_INDEX {
                names.extend(name_index(k.c.data(&s)));
            }
        }
        self.out.raw("<StyleSheets>")?;
        for o in top.kids.iter().filter(|o| o.c.ty == T_STYLE) {
            let id = o.c.id;
            let (name, name_u) = names.iter().find(|n| n.2 == id).map_or((None, None), |n| (Some(n.0), Some(n.1)));
            self.out.open("StyleSheet");
            self.out.attr("ID", &format!("{id}"));
            if let Some(n) = name_u.and_then(|i| self.name(i)).map(String::from) {
                self.out.attr("NameU", &n);
            }
            if let Some(n) = name.and_then(|i| self.name(i)).map(String::from) {
                self.out.attr("Name", &n);
            }
            let refs = cells::style_refs(o, &s);
            cells::refs_attrs(&mut self.out, refs);
            self.out.close()?;
            self.sheet(o, &s)?;
            self.out.end("StyleSheet")?;
        }
        self.out.raw("</StyleSheets>")
    }

    // the document sheet: an object of its own stream
    fn doc_sheet(&mut self, root: &Node) -> R<()> {
        let Some(node) = root.kid(T_DOC_SHEET) else { return Ok(()) };
        let (objs, s) = self.objects(node)?;
        let Some(o) = objs.iter().find(|o| o.c.ty == 0x4F) else { return Ok(()) };
        self.out.open("DocumentSheet");
        cells::refs_attrs(&mut self.out, cells::style_refs(o, &s));
        self.out.close()?;
        self.sheet(o, &s)?;
        self.out.end("DocumentSheet")
    }

    // ---- masters

    fn masters(&mut self, root: &Node) -> R<()> {
        let Some(list) = root.kid(T_STENCILS) else { return Ok(()) };
        let Body::List { items, order } = &list.body else { return Ok(()) };
        // the name index of the masters: (Name, NameU, pointer index)
        let mut names: Vec<(u32, u32, u32)> = Vec::new();
        for (_, n) in items.iter().filter(|(_, n)| n.ty == T_NAME_INDEX) {
            if let Some(b) = n.blob() {
                names.extend(name_index_blob(b));
            }
        }
        // the masters in the order of the list (gaps and unknown indices left out)
        let mut seq: Vec<usize> = order.iter().map(|&o| o as usize).collect();
        if seq.is_empty() {
            seq = items.iter().filter(|(_, n)| n.ty == T_MASTER).map(|(i, _)| *i).collect();
        }
        self.out.raw("<Masters>")?;
        for ix in seq {
            let Some((_, node)) = items.iter().find(|(i, n)| *i == ix && n.ty == T_MASTER) else { continue };
            let (name, name_u) = names.iter().find(|n| n.2 as usize == ix).map_or((None, None), |n| (Some(n.0), Some(n.1)));
            self.master(node, ix as u32, name.and_then(|i| self.name(i)).map(String::from), name_u.and_then(|i| self.name(i)).map(String::from))?;
        }
        self.out.raw("</Masters>")
    }

    fn master(&mut self, node: &Node, id: u32, name: Option<String>, name_u: Option<String>) -> R<()> {
        let Body::List { items, .. } = &node.body else { return Ok(()) };
        self.out.open("Master");
        self.out.attr("ID", &format!("{id}"));
        if let Some(n) = &name_u {
            self.out.attr("NameU", n);
        }
        if let Some(n) = name.as_ref().or(name_u.as_ref()) {
            self.out.attr("Name", n);
        }
        // the prompt: the first plain stream of type 0x0A (index 2)
        if let Some(b) = items.iter().find(|(i, n)| *i == 2 && n.ty == T_PROMPT).and_then(|(_, n)| n.blob()) {
            let p = utf16z(b);
            if !p.is_empty() {
                self.out.attr("Prompt", &p);
            }
        }
        self.out.close()?;
        // the page sheet (index 0)
        if let Some((_, ps)) = items.iter().find(|(_, n)| n.ty == T_PAGE_SHEET) {
            let (objs, s) = self.objects(ps)?;
            if let Some(o) = objs.iter().find(|o| o.c.ty == T_PAGE_SHEET) {
                self.out.open("PageSheet");
                cells::refs_attrs(&mut self.out, cells::style_refs(o, &s));
                self.out.close()?;
                self.sheet(o, &s)?;
                self.out.end("PageSheet")?;
            }
        }
        // the shapes: each a stream at the index that is its ID; a shape names its parent group
        let mut shapes: Vec<(u32, u32, Vec<Obj>, Vec<u8>)> = Vec::new();
        for (i, n) in items {
            if !matches!(n.ty, T_GROUP | T_SHAPE | T_GUIDE | T_FOREIGN) {
                continue;
            }
            let (objs, s) = self.objects(n)?;
            let parent = objs.first().map_or(0, |o| cells::shape_parent(o, &s));
            shapes.push((*i as u32, parent, objs, s));
        }
        self.out.raw("<Shapes>")?;
        let tops: Vec<usize> = (0..shapes.len()).filter(|&k| shapes[k].1 == 0 || !shapes.iter().any(|x| x.0 == shapes[k].1)).collect();
        // children in the order of their group's list; a stack instead of recursion
        let mut done = alloc::vec![false; shapes.len()];
        for t in tops {
            self.shape_tree(&shapes, t, &mut done)?;
        }
        self.out.raw("</Shapes>")?;
        self.out.end("Master")
    }

    // a shape and its subshapes (groups nest as deep as the file has them: a stack of its own)
    fn shape_tree(&mut self, shapes: &[(u32, u32, Vec<Obj>, Vec<u8>)], top: usize, done: &mut [bool]) -> R<()> {
        enum Step {
            Open(usize),
            Close,
        }
        let mut stack = alloc::vec![Step::Open(top)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Close => {
                    self.out.raw("</Shapes></Shape>")?;
                }
                Step::Open(k) => {
                    if done[k] {
                        continue;
                    }
                    done[k] = true;
                    let (id, _, objs, s) = &shapes[k];
                    let Some(o) = objs.first() else { continue };
                    let kids: Vec<usize> = cells::group_children(o, s)
                        .into_iter()
                        .filter_map(|cid| shapes.iter().position(|x| x.0 == cid && x.1 == *id))
                        .chain((0..shapes.len()).filter(|&j| shapes[j].1 == *id))
                        .collect();
                    self.shape_open(*id, o, s)?;
                    let mut uniq: Vec<usize> = Vec::new();
                    for j in kids {
                        if !uniq.contains(&j) && !done[j] {
                            uniq.push(j);
                        }
                    }
                    if uniq.is_empty() {
                        self.out.end("Shape")?;
                    } else {
                        self.out.raw("<Shapes>")?;
                        stack.push(Step::Close);
                        for j in uniq.into_iter().rev() {
                            stack.push(Step::Open(j));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // a shape's element and its sheet, without its subshapes and its end
    fn shape_open(&mut self, id: u32, o: &Obj, s: &[u8]) -> R<()> {
        self.out.open("Shape");
        self.out.attr("ID", &format!("{id}"));
        let ty = match o.c.ty {
            T_GROUP => "Group",
            T_GUIDE => "Guide",
            T_FOREIGN => "Foreign",
            _ => "Shape",
        };
        self.out.attr("Type", ty);
        cells::refs_attrs(&mut self.out, cells::style_refs(o, s));
        self.out.close()?;
        self.sheet(o, s)
    }

    /// the sections of an object (shape, style, sheet) as the XML has them
    fn sheet(&mut self, o: &Obj, s: &[u8]) -> R<()> {
        cells::sheet(self, o, s)
    }
}

/// a NUL-terminated (or not) UTF-16 string
pub(super) fn utf16z(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|x| u16::from_le_bytes([x[0], x[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

// a name index chunk's data: [u32 count] then count x (Name, NameU, object, u8)
fn name_index(d: &[u8]) -> Vec<(u32, u32, u32)> {
    let n = u32at(d, 0).unwrap_or(0) as usize;
    (0..n.min(d.len() / 13)).filter_map(|k| Some((u32at(d, 4 + 13 * k)?, u32at(d, 8 + 13 * k)?, u32at(d, 12 + 13 * k)?))).collect()
}

// the same in a plain stream (after the length word of a compressed one)
fn name_index_blob(b: &[u8]) -> Vec<(u32, u32, u32)> {
    name_index(b)
}
