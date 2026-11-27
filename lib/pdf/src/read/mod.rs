// pdf::read -- a PDF reader: the objects of a file, its pages, information, outlines, links and
// other annotations, form fields, attachments, page labels, the text of each page with where
// each glyph stands, and the pages drawn as pixels (render/).  PDF 1.0 to 2.0: classic cross-reference tables and cross-reference
// streams, object streams, incremental updates, hybrid files; files whose table is missing or
// wrong are rebuilt from their objects, as the common readers do.  Encrypted files with the
// standard security handler open with their password, or without one when only the owner
// password is set (RC4 40 to 128 bit, AES-128, AES-256).
//
//   let doc = read::Document::open(bytes)?;            // or open_with(bytes, &Options { password, .. })
//   for i in 0..doc.page_count() {
//       let text = doc.text(i)?;                       // the page's text in reading order
//       let size = doc.page(i)?.size();                // points, after /Rotate
//   }
//   let info = doc.info();                             // pdf::Info: title, author, dates, ...
//   for o in doc.outlines() { .. }                     // bookmarks with their Action
//   let px = doc.render(0, &RenderOptions { dpi: 150.0, ..Default::default() })?;  // gfx::Surface
//
//   Document     open / open_with, version, is_encrypted / permissions / opened_as_owner,
//                was_repaired, trailer / catalog, get(Ref) / resolve, decode(Stream) /
//                decode_until_image, page_count / page / pages / contents, info / metadata /
//                language, outlines, named_dest, annotations / links, fields, attachments,
//                page_label, page_text (glyphs with positions) / text, page_pixels / render
//                (RenderOptions: dpi, region, background, annotations, fonts: a FontSource,
//                SystemFonts by default)
//   Object       Null, Bool, Int, Real, String, Name, Array, Dict, Stream, Ref; text_string
//   Ops          the operators of a content stream with their operands (content::Op)
//
// Untrusted input: a damaged or hostile file gives an Error or less than it claims, never a
// panic, a hang or memory without end.  Nesting, the objects of one parse, the depth of
// references, page trees, outlines and form XObjects are bounded; every decoded stream is at
// most Options::max_stream bytes.  Permissions are reported, not enforced: what a program
// allows its user to do with the document is its own decision.
//
// Not here: the image filters for decode_until_image (it stops before DCT, JPX, JBIG2 and
// CCITTFax; drawing decodes DCT, CCITTFax and JPX), JBIG2 images at all, the predefined CMaps
// of East Asian fonts other than Identity and the Unicode ones, verifying signatures, changing
// and saving documents.

mod cmap;
mod content;
mod crypt;
mod filter;
mod font;
mod lex;
mod nav;
mod object;
mod render;
pub(crate) mod tables;
mod text;
mod xref;

pub use content::{Op, Ops};
pub use nav::{Action, Annotation, Attachment, Dest, Field, FieldKind, Outline, Signature, View};
pub use object::{text_string, Dict, Name, Object, Ref, Stream};
pub use render::{FontQuery, FontSource, RenderOptions, SystemFonts};
pub use text::{PageText, TextItem};

use crate::{Date, Info};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{OnceCell, RefCell};
use xref::Entry;

/// how a document is opened
#[derive(Clone, Debug)]
pub struct Options {
    /// the user or the owner password ("" for files without one)
    pub password: String,
    /// the largest decoded stream, in bytes (a content stream, an image, an attachment)
    pub max_stream: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { password: String::new(), max_stream: 256 << 20 }
    }
}

/// why a document or a part of it cannot be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// not a PDF file: no header and no objects
    NotPdf,
    /// the file is damaged where it was needed
    Damaged(&'static str),
    /// encrypted, and the password is missing or wrong
    Password,
    /// something this reader does not do (a security handler other than the standard one, an
    /// image filter)
    Unsupported(&'static str),
    /// more than the Options allow
    Limit(&'static str),
    /// a page number past the last page
    NoPage,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotPdf => f.write_str("not a PDF file"),
            Error::Damaged(s) => write!(f, "damaged: {s}"),
            Error::Password => f.write_str("the password is missing or wrong"),
            Error::Unsupported(s) => write!(f, "not supported: {s}"),
            Error::Limit(s) => write!(f, "limit: {s}"),
            Error::NoPage => f.write_str("no such page"),
        }
    }
}

impl core::error::Error for Error {}

/// a page: its boxes (points, PDF page space with y up), rotation and dictionaries
#[derive(Clone, Debug)]
pub struct Page {
    /// the page object
    pub obj: Option<Ref>,
    /// x0, y0, x1, y1
    pub media_box: [f64; 4],
    /// the visible part (the media box when the page sets none)
    pub crop_box: [f64; 4],
    /// clockwise when shown: 0, 90, 180 or 270
    pub rotate: u16,
    /// the size of a unit in points (PDF 1.6; usually 1)
    pub user_unit: f64,
    /// the page dictionary, and its resources (inherited ones included)
    pub dict: Dict,
    pub resources: Dict,
}

impl Page {
    /// width and height of the crop box in points as the page is shown (after /Rotate)
    pub fn size(&self) -> (f64, f64) {
        let w = (self.crop_box[2] - self.crop_box[0]) * self.user_unit;
        let h = (self.crop_box[3] - self.crop_box[1]) * self.user_unit;
        if self.rotate % 180 == 90 { (h, w) } else { (w, h) }
    }
}

struct ObjStm {
    data: Vec<u8>,
    first: usize,
    /// (object number, offset after /First)
    index: Vec<(u32, usize)>,
}

/// at most this many references in a row (a reference to a reference ...)
const MAX_CHAIN: u32 = 32;
/// object streams and /Length references inside each other
const MAX_LOAD_DEPTH: u32 = 8;
/// the most pages a document may have
const MAX_PAGES: usize = 1 << 20;

/// a PDF document opened for reading
pub struct Document {
    data: Vec<u8>,
    xref: xref::Xref,
    scan: OnceCell<BTreeMap<u32, (usize, u16)>>,
    ends: OnceCell<lex::Ends>,
    objstms: RefCell<BTreeMap<u32, Option<Rc<ObjStm>>>>,
    crypt: Option<crypt::Crypt>,
    encrypt_ref: Option<Ref>,
    pages: Vec<Page>,
    page_index: BTreeMap<Ref, usize>,
    version: (u8, u8),
    max_stream: usize,
    /// what drawing keeps across pages (fonts, the system's font list)
    render: render::Cache,
}

impl Document {
    /// open a document without a password
    pub fn open(data: Vec<u8>) -> Result<Document, Error> {
        Document::open_with(data, &Options::default())
    }

    pub fn open_with(data: Vec<u8>, opt: &Options) -> Result<Document, Error> {
        let head = lex::find(&data[..data.len().min(1024)], b"%PDF-", 0);
        let version = head
            .map(|h| {
                let d = |i: usize| data.get(h + i).filter(|c| c.is_ascii_digit()).map(|c| c - b'0');
                (d(5).unwrap_or(1), d(7).unwrap_or(4))
            })
            .unwrap_or((1, 4));
        let x = xref::load(&data);
        if x.entries.is_empty() && head.is_none() {
            return Err(Error::NotPdf);
        }
        let mut doc = Document {
            data,
            xref: x,
            scan: OnceCell::new(),
            ends: OnceCell::new(),
            objstms: RefCell::new(BTreeMap::new()),
            crypt: None,
            encrypt_ref: None,
            pages: Vec::new(),
            page_index: BTreeMap::new(),
            version,
            max_stream: opt.max_stream,
            render: render::Cache::default(),
        };
        doc.setup_crypt(&opt.password)?;
        // a catalog that does not load: rebuild from the objects
        if !doc.xref.repaired && doc.catalog().is_empty() {
            let x = xref::scan(&doc.data);
            doc.xref = x;
            doc.objstms.borrow_mut().clear();
            doc.setup_crypt(&opt.password)?;
        }
        if doc.xref.repaired {
            doc.add_objstm_entries();
            if doc.catalog().is_empty() {
                // no trailer named it: the last catalog among the objects
                let nums: Vec<u32> = doc.xref.entries.keys().copied().collect();
                let found = nums.into_iter().rev().take(1 << 20).find(|&n| doc.get(Ref { num: n, generation: 0 }).as_dict().is_some_and(|d| d.is("Type", "Catalog")));
                if let Some(n) = found {
                    doc.xref.trailer.insert("Root", Object::Ref(Ref { num: n, generation: 0 }));
                }
            }
        }
        if let Some(v) = doc.catalog().name("Version") {
            if v.len() >= 3 && v[0].is_ascii_digit() && v[2].is_ascii_digit() && (v[0] - b'0', v[2] - b'0') > doc.version {
                doc.version = (v[0] - b'0', v[2] - b'0');
            }
        }
        doc.collect_pages();
        if doc.pages.is_empty() && doc.catalog().is_empty() {
            return Err(Error::Damaged("no catalog and no pages"));
        }
        Ok(doc)
    }

    fn setup_crypt(&mut self, password: &str) -> Result<(), Error> {
        self.crypt = None;
        self.encrypt_ref = None;
        let Some(e) = self.xref.trailer.get("Encrypt").cloned() else { return Ok(()) };
        if e.is_null() {
            return Ok(());
        }
        self.encrypt_ref = e.as_ref();
        let enc = match self.resolve(&e) {
            Object::Dict(d) => d,
            _ => return Err(Error::Damaged("the encryption dictionary")),
        };
        let id0 = match self.xref.trailer.get("ID") {
            Some(Object::Array(a)) => a.first().and_then(Object::as_bytes).map(<[u8]>::to_vec).unwrap_or_default(),
            _ => Vec::new(),
        };
        self.crypt = Some(crypt::Crypt::open(&enc, &id0, password)?);
        Ok(())
    }

    /// a scanned file: the objects of its object streams are known once they can be decoded
    fn add_objstm_entries(&mut self) {
        let list = core::mem::take(&mut self.xref.objstms);
        for s in list {
            let Some(st) = self.objstm(s, 0) else { continue };
            for (i, &(num, _)) in st.index.iter().enumerate() {
                self.xref.entries.entry(num).or_insert((Entry::InStream(s, i as u32), 0));
            }
        }
    }

    /// the version of the header, or of the catalog's /Version when that is newer
    pub fn version(&self) -> (u8, u8) {
        self.version
    }

    pub fn is_encrypted(&self) -> bool {
        self.crypt.is_some()
    }

    /// the permission bits /P of an encrypted file (ISO 32000-1, Table 22: bit 3 print, 4
    /// modify, 5 copy text and graphics, 6 annotate, 9 fill in forms, 10 extract for
    /// accessibility, 11 assemble, 12 print in high quality; counted from 1)
    pub fn permissions(&self) -> Option<u32> {
        self.crypt.as_ref().map(|c| c.permissions)
    }

    /// the revision of the standard security handler (2 to 6) of an encrypted file
    pub fn encryption_revision(&self) -> Option<u8> {
        self.crypt.as_ref().map(|c| c.revision)
    }

    /// the password given was the owner password
    pub fn opened_as_owner(&self) -> bool {
        self.crypt.as_ref().is_some_and(|c| c.owner)
    }

    /// the cross-reference was rebuilt from the objects of the file
    pub fn was_repaired(&self) -> bool {
        self.xref.repaired
    }

    pub fn trailer(&self) -> &Dict {
        &self.xref.trailer
    }

    /// the document catalog (empty when there is none)
    pub fn catalog(&self) -> Dict {
        match self.xref.trailer.get("Root") {
            Some(r) => self.dict(r),
            None => Dict::new(),
        }
    }

    /// the raw bytes of the file
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    fn scan(&self) -> &BTreeMap<u32, (usize, u16)> {
        self.scan.get_or_init(|| xref::scan_objects(&self.data))
    }

    /// an indirect object (Null when it is missing, as ISO 32000 has it)
    pub fn get(&self, r: Ref) -> Object {
        self.load(r, 0)
    }

    fn load(&self, r: Ref, depth: u32) -> Object {
        if depth > MAX_LOAD_DEPTH {
            return Object::Null;
        }
        match self.xref.entries.get(&r.num).copied() {
            Some((Entry::At(off), _)) => {
                if let Some(o) = self.at(off, r, depth) {
                    return o;
                }
                if !self.xref.repaired {
                    if let Some(&(o2, _)) = self.scan().get(&r.num) {
                        if o2 != off {
                            return self.at(o2, r, depth).unwrap_or_default();
                        }
                    }
                }
                Object::Null
            }
            Some((Entry::InStream(s, i), _)) => self.in_stream(s, i, r.num, depth).unwrap_or_default(),
            _ if self.xref.repaired => Object::Null,
            // an object a damaged table left out
            _ => match self.scan().get(&r.num) {
                Some(&(off, _)) => self.at(off, r, depth).unwrap_or_default(),
                None => Object::Null,
            },
        }
    }

    fn at(&self, off: usize, r: Ref, depth: u32) -> Option<Object> {
        let ends = |from: usize| self.ends.get_or_init(|| lex::Ends::of(&self.data)).after(from);
        let (got, mut obj) = lex::indirect(&self.data, off, &mut |lr| self.load(lr, depth + 1).as_int(), &ends)?;
        if got.num != r.num {
            return None;
        }
        if let Some(c) = &self.crypt {
            if self.encrypt_ref != Some(r) {
                c.strings_in(r, &mut obj, 0);
            }
        }
        Some(obj)
    }

    fn objstm(&self, num: u32, depth: u32) -> Option<Rc<ObjStm>> {
        if let Some(v) = self.objstms.borrow().get(&num) {
            return v.clone();
        }
        let built = (|| {
            let o = self.load(Ref { num, generation: 0 }, depth + 1);
            let s = o.as_stream()?;
            let (data, img) = self.decode_d(s, depth + 1).ok()?;
            if img.is_some() {
                return None;
            }
            let n = s.dict.int("N").unwrap_or(0).max(0) as usize;
            let first = usize::try_from(s.dict.int("First")?).ok()?;
            let mut lx = lex::Lexer::new(&data, 0);
            let mut index = Vec::new();
            while index.len() < n && lx.pos < first.min(data.len()) {
                let (lex::Tok::Int(a), lex::Tok::Int(b)) = (lx.next(), lx.next()) else { break };
                let (Ok(a), Ok(b)) = (u32::try_from(a), usize::try_from(b)) else { break };
                index.push((a, b));
            }
            Some(Rc::new(ObjStm { data, first, index }))
        })();
        self.objstms.borrow_mut().insert(num, built.clone());
        built
    }

    fn in_stream(&self, s: u32, i: u32, num: u32, depth: u32) -> Option<Object> {
        let st = self.objstm(s, depth)?;
        let off = match st.index.get(i as usize) {
            Some(&(n, off)) if n == num => off,
            _ => st.index.iter().find(|e| e.0 == num)?.1,
        };
        let mut lx = lex::Lexer::new(&st.data, st.first.checked_add(off)?);
        lx.object(true).ok()
    }

    /// follow references until an object that is none (Null after too many)
    pub fn resolve(&self, o: &Object) -> Object {
        let mut cur = match o {
            Object::Ref(r) => self.get(*r),
            o => return o.clone(),
        };
        for _ in 0..MAX_CHAIN {
            match cur {
                Object::Ref(r) => cur = self.get(r),
                _ => return cur,
            }
        }
        Object::Null
    }

    /// a dictionary (a stream's), resolved; empty when it is none
    pub(crate) fn dict(&self, o: &Object) -> Dict {
        match self.resolve(o) {
            Object::Dict(d) => d,
            Object::Stream(s) => s.dict,
            _ => Dict::new(),
        }
    }

    /// the value under `key`, resolved
    pub(crate) fn key(&self, d: &Dict, key: &str) -> Object {
        d.get(key).map(|o| self.resolve(o)).unwrap_or_default()
    }

    /// a stream's data with all its filters applied (an image filter is Error::Unsupported)
    pub fn decode(&self, s: &Stream) -> Result<Vec<u8>, Error> {
        match self.decode_d(s, 0)? {
            (d, None) => Ok(d),
            (_, Some(_)) => Err(Error::Unsupported("an image filter (decode_until_image)")),
        }
    }

    /// a stream's data with its filters applied up to an image filter: the data, and the image
    /// filter with its parameters (DCTDecode is a JPEG file then, JPXDecode a JPEG 2000 one)
    pub fn decode_until_image(&self, s: &Stream) -> Result<(Vec<u8>, Option<(Name, Dict)>), Error> {
        self.decode_d(s, 0)
    }

    fn decode_d(&self, s: &Stream, depth: u32) -> Result<(Vec<u8>, Option<(Name, Dict)>), Error> {
        let raw = self.data.get(s.start..s.end).ok_or(Error::Damaged("a stream outside the file"))?;
        // the filters and their parameters may be references
        let mut fd = Dict::new();
        for k in ["Filter", "DecodeParms"] {
            if let Some(o) = s.dict.get(k) {
                let v = match self.resolve_d(o, depth) {
                    Object::Array(a) => Object::Array(a.iter().map(|e| self.resolve_d(e, depth)).collect()),
                    v => v,
                };
                fd.insert(k, v);
            }
        }
        let chain = filter::chain(&fd);
        let mut data = raw.to_vec();
        if let (Some(c), Some(r)) = (&self.crypt, s.owner) {
            let exempt = s.dict.is("Type", "XRef") || (!c.encrypt_metadata && s.dict.is("Type", "Metadata"));
            if !exempt {
                let cf = match chain.first() {
                    Some((n, p)) if n == "Crypt" => Some(p.name("Name").unwrap_or(b"Identity")),
                    _ => None,
                };
                data = c.stream(r, &data, cf, s.dict.is("Type", "EmbeddedFile"));
            }
        }
        filter::apply(&chain, data, self.max_stream)
    }

    fn resolve_d(&self, o: &Object, depth: u32) -> Object {
        match o {
            Object::Ref(r) => match self.load(*r, depth + 1) {
                Object::Ref(_) => Object::Null,
                v => v,
            },
            o => o.clone(),
        }
    }

    fn collect_pages(&mut self) {
        #[derive(Clone, Default)]
        struct Inh {
            resources: Option<Object>,
            media: Option<Object>,
            crop: Option<Object>,
            rotate: Option<i64>,
        }
        let cat = self.catalog();
        let mut pages = Vec::new();
        let mut seen = BTreeSet::new();
        let mut stack: Vec<(Object, Inh, u32)> = Vec::new();
        if let Some(root) = cat.get("Pages") {
            stack.push((root.clone(), Inh::default(), 0));
        }
        let budget = self.xref.entries.len().saturating_mul(2).max(64);
        let mut nodes = 0usize;
        while let Some((node, inh, depth)) = stack.pop() {
            nodes += 1;
            if nodes > budget || pages.len() >= MAX_PAGES || depth > 256 {
                break;
            }
            let r = node.as_ref();
            if let Some(r) = r {
                if !seen.insert(r) {
                    continue; // a loop in the tree, or a page listed twice
                }
            }
            let d = self.dict(&node);
            if d.is_empty() {
                continue;
            }
            let mut inh = inh;
            if let Some(v) = d.get("Resources") {
                inh.resources = Some(v.clone());
            }
            if let Some(v) = d.get("MediaBox") {
                inh.media = Some(v.clone());
            }
            if let Some(v) = d.get("CropBox") {
                inh.crop = Some(v.clone());
            }
            if let Some(v) = d.get("Rotate").map(|o| self.resolve(o)).and_then(|o| o.as_int()) {
                inh.rotate = Some(v);
            }
            let kids = match self.key(&d, "Kids") {
                Object::Array(a) => Some(a),
                _ => None,
            };
            let is_page = d.is("Type", "Page") || (kids.is_none() && !d.is("Type", "Pages"));
            if !is_page {
                for k in kids.unwrap_or_default().into_iter().rev() {
                    stack.push((k, inh.clone(), depth + 1));
                }
                continue;
            }
            pages.push((r, d, inh));
        }
        // a tree that leads nowhere: the page objects of the file, in order
        if pages.is_empty() {
            let nums: Vec<u32> = self.xref.entries.keys().copied().collect();
            for num in nums.into_iter().take(1 << 20) {
                let r = Ref { num, generation: self.xref.entries.get(&num).map(|e| e.1).unwrap_or(0) };
                let d = self.dict(&Object::Ref(r));
                if d.is("Type", "Page") {
                    let inh = Inh { resources: d.get("Resources").cloned(), ..Inh::default() };
                    pages.push((Some(r), d, inh));
                }
            }
        }
        for (r, d, inh) in pages {
            let rect = |o: &Option<Object>| o.as_ref().map(|o| self.resolve(o)).and_then(|o| o.as_rect());
            let media = rect(&inh.media).filter(|b| b[2] > b[0] && b[3] > b[1]).unwrap_or([0.0, 0.0, 612.0, 792.0]);
            let crop = rect(&inh.crop)
                .map(|c| [c[0].max(media[0]), c[1].max(media[1]), c[2].min(media[2]), c[3].min(media[3])])
                .filter(|c| c[2] > c[0] && c[3] > c[1])
                .unwrap_or(media);
            let rotate = (inh.rotate.unwrap_or(0).rem_euclid(360) / 90 * 90) as u16;
            let user_unit = d.num("UserUnit").filter(|&u| u > 0.0 && u < 1e4).unwrap_or(1.0);
            let resources = inh.resources.as_ref().map(|o| self.dict(o)).unwrap_or_default();
            if let Some(r) = r {
                self.page_index.insert(r, self.pages.len());
            }
            self.pages.push(Page { obj: r, media_box: media, crop_box: crop, rotate, user_unit, dict: d, resources });
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page(&self, i: usize) -> Result<&Page, Error> {
        self.pages.get(i).ok_or(Error::NoPage)
    }

    /// the page a page object is (for destinations)
    pub fn page_of(&self, r: Ref) -> Option<usize> {
        self.page_index.get(&r).copied()
    }

    /// the content of a page: its content streams decoded and joined (a stream that cannot be
    /// decoded adds nothing)
    pub fn contents(&self, i: usize) -> Result<Vec<u8>, Error> {
        let p = self.page(i)?;
        let mut out = Vec::new();
        let parts = match self.key(&p.dict, "Contents") {
            Object::Array(a) => a,
            Object::Stream(s) => alloc::vec![Object::Stream(s)],
            _ => Vec::new(),
        };
        for part in parts {
            if let Object::Stream(s) = self.resolve(&part) {
                if let Ok(d) = self.decode(&s) {
                    if out.len() + d.len() > self.max_stream {
                        return Err(Error::Limit("the page's content is larger than max_stream"));
                    }
                    out.extend_from_slice(&d);
                    out.push(b'\n');
                }
            }
        }
        Ok(out)
    }

    /// the document information (the Info dictionary; dates as the file writes them)
    pub fn info(&self) -> Info {
        let d = match self.xref.trailer.get("Info") {
            Some(o) => self.dict(o),
            None => Dict::new(),
        };
        let s = |k: &str| match self.key(&d, k) {
            Object::String(b) => Some(text_string(&b)),
            _ => None,
        };
        Info {
            title: s("Title"),
            author: s("Author"),
            subject: s("Subject"),
            keywords: s("Keywords"),
            creator: s("Creator"),
            producer: s("Producer"),
            created: s("CreationDate").and_then(|t| parse_date(&t)),
            modified: s("ModDate").and_then(|t| parse_date(&t)),
        }
    }

    /// the XMP metadata of the document (the catalog's /Metadata stream)
    pub fn metadata(&self) -> Option<Vec<u8>> {
        match self.key(&self.catalog(), "Metadata") {
            Object::Stream(s) => self.decode(&s).ok(),
            _ => None,
        }
    }

    /// the natural language of the document (/Lang: "de-DE")
    pub fn language(&self) -> Option<String> {
        match self.key(&self.catalog(), "Lang") {
            Object::String(b) => Some(text_string(&b)),
            _ => None,
        }
    }
}

/// a date of a PDF file (7.9.4: "D:YYYYMMDDHHmmSSOHH'mm'"; every part after the year may be
/// missing, as may "D:")
pub fn parse_date(s: &str) -> Option<Date> {
    let s = s.trim();
    let s = s.strip_prefix("D:").unwrap_or(s).as_bytes();
    let num = |i: usize, n: usize| -> Option<u32> {
        let p = s.get(i..i + n)?;
        p.iter().all(u8::is_ascii_digit).then(|| p.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32))
    };
    let year = num(0, 4)?;
    let mut parts = [1u32, 1, 0, 0, 0];
    let mut i = 4;
    for p in parts.iter_mut() {
        let Some(v) = num(i, 2) else { break };
        *p = v;
        i += 2;
    }
    let [month, day, hour, minute, second] = parts;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut offset = 0i32;
    match s.get(i) {
        Some(&c @ (b'+' | b'-')) => {
            let h = num(i + 1, 2).unwrap_or(0) as i32;
            let m = s.get(i + 3).filter(|&&c| c == b'\'').map_or_else(|| num(i + 3, 2), |_| num(i + 4, 2)).unwrap_or(0) as i32;
            offset = (h * 60 + m).min(1439) * if c == b'-' { -1 } else { 1 };
        }
        _ => {}
    }
    Some(Date { year: year as u16, month: month as u8, day: day as u8, hour: hour as u8, minute: minute as u8, second: second.min(59) as u8, offset_minutes: offset as i16 })
}
