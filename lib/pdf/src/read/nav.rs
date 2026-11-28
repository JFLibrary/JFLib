// What a document holds besides its pages' content (ISO 32000-1, 12): the outline
// (bookmarks), destinations (explicit and named, through the /Dests dictionary or the name
// tree), actions, annotations with links, the fields of an interactive form with their values
// and signatures (read, not verified), embedded files and page labels.  Trees and lists are
// walked with a set of the objects seen and a budget, so a loop or a huge tree ends.

use super::object::{text_string, Dict, Object, Ref, Stream};
use super::{parse_date, Document, Error};
use crate::Date;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// how a destination shows its page (12.3.2.2); None keeps what the viewer has
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Xyz { left: Option<f64>, top: Option<f64>, zoom: Option<f64> },
    Fit,
    FitH(Option<f64>),
    FitV(Option<f64>),
    /// left, bottom, right, top
    FitR([f64; 4]),
    FitB,
    FitBH(Option<f64>),
    FitBV(Option<f64>),
}

/// a place in the document: a page (counted from 0) and how it is shown
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dest {
    pub page: usize,
    pub view: View,
}

/// what an outline item or a link does (12.6.4)
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    GoTo(Dest),
    /// a web address or other URI
    Uri(String),
    /// a page of another PDF file (GoToR): the file, the page when given as a number
    GoToFile { file: String, page: Option<usize> },
    /// start another file or program (Launch)
    Launch(String),
    /// a viewer command (Named: NextPage, PrevPage, FirstPage, LastPage, ...)
    Named(String),
    /// any other kind of action (JavaScript, SubmitForm, ...), by its /S
    Other(String),
}

/// an item of the outline, with its children
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub title: String,
    pub action: Option<Action>,
    /// its children are shown
    pub open: bool,
    pub children: Vec<Outline>,
}

/// an annotation of a page
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    /// /Subtype: Link, Text, Highlight, Widget, FileAttachment, ...
    pub subtype: String,
    /// x0, y0, x1, y1 in page space
    pub rect: [f64; 4],
    /// /Contents: the text of a note or comment
    pub contents: Option<String>,
    /// /T: the author of a markup annotation
    pub author: Option<String>,
    pub modified: Option<Date>,
    /// a link's action (its /A, or its /Dest as GoTo)
    pub action: Option<Action>,
    pub obj: Option<Ref>,
}

/// a form field's kind (12.7.4)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    CheckBox,
    Radio,
    PushButton,
    ComboBox,
    ListBox,
    Signature,
    /// a field without /FT
    Unknown,
}

/// a field's signature as the file states it (12.8): the signature is not verified
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    /// /Filter and /SubFilter: Adobe.PPKLite, adbe.pkcs7.detached, ETSI.CAdES.detached, ...
    pub filter: String,
    pub sub_filter: String,
    /// /Name, /Reason, /Location, /ContactInfo as given
    pub name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub time: Option<Date>,
    /// the byte ranges of the file the signature covers: (start, length) pairs
    pub byte_range: Vec<(u64, u64)>,
    /// the signature (a PKCS #7 / CMS structure for the usual kinds)
    pub contents: Vec<u8>,
    /// the ranges cover the whole file but the signature itself (later changes make it false)
    pub covers_file: bool,
}

/// a field of the interactive form
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// the full name: the partial names of its parents and its own, with dots
    pub name: String,
    pub kind: FieldKind,
    /// the value: text, the chosen option(s), a button's state ("Off" when not checked)
    pub value: Vec<String>,
    /// /Ff
    pub flags: u32,
    pub read_only: bool,
    pub required: bool,
    /// the options of a choice field, as shown
    pub options: Vec<String>,
    /// the widgets: the page (when known) and the rectangle
    pub widgets: Vec<(Option<usize>, [f64; 4])>,
    /// for a signature field with a value
    pub signature: Option<Signature>,
}

/// a file embedded in the document (7.11.4)
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    /// the file name (/UF, else /F)
    pub name: String,
    pub description: Option<String>,
    /// /Subtype of the file stream: a MIME type ("text/xml")
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub modified: Option<Date>,
    /// /AFRelationship (PDF/A-3, ZUGFeRD: Data, Source, Alternative, ...)
    pub relationship: Option<String>,
    /// the data: Document::decode(&stream)
    pub stream: Option<Stream>,
}

/// trees and lists end after this many nodes
const BUDGET: usize = 1 << 20;

fn rect_of(o: &Object) -> [f64; 4] {
    o.as_rect().unwrap_or([0.0; 4])
}

impl Document {
    fn text_of(&self, d: &Dict, k: &str) -> Option<String> {
        match self.key(d, k) {
            Object::String(b) => Some(text_string(&b)),
            Object::Name(n) => Some(n.to_text()),
            _ => None,
        }
    }

    /// an explicit destination [page /XYZ left top zoom] or a named one
    pub(crate) fn dest(&self, o: &Object) -> Option<Dest> {
        self.dest_d(o, 0)
    }

    fn dest_d(&self, o: &Object, depth: u32) -> Option<Dest> {
        if depth > 4 {
            return None;
        }
        match self.resolve(o) {
            Object::Array(a) => {
                let page = match a.first()? {
                    Object::Ref(r) => self.page_of(*r)?,
                    // some writers number the page in a local destination
                    Object::Int(n) => usize::try_from(*n).ok().filter(|&n| n < self.page_count())?,
                    _ => return None,
                };
                let n = |i: usize| a.get(i).and_then(Object::as_f64);
                let view = match a.get(1).and_then(Object::as_name).unwrap_or(b"Fit") {
                    b"XYZ" => View::Xyz { left: n(2), top: n(3), zoom: n(4).filter(|&z| z > 0.0) },
                    b"FitH" => View::FitH(n(2)),
                    b"FitV" => View::FitV(n(2)),
                    b"FitR" => View::FitR([n(2).unwrap_or(0.0), n(3).unwrap_or(0.0), n(4).unwrap_or(0.0), n(5).unwrap_or(0.0)]),
                    b"FitB" => View::FitB,
                    b"FitBH" => View::FitBH(n(2)),
                    b"FitBV" => View::FitBV(n(2)),
                    _ => View::Fit,
                };
                Some(Dest { page, view })
            }
            Object::Dict(d) => self.dest_d(d.get("D")?, depth + 1),
            Object::Name(n) => self.dest_d(&self.named_dest_obj(&n.0)?, depth + 1),
            Object::String(s) => self.dest_d(&self.named_dest_obj(&s)?, depth + 1),
            _ => None,
        }
    }

    fn named_dest_obj(&self, name: &[u8]) -> Option<Object> {
        let cat = self.catalog();
        // PDF 1.1: a dictionary of names
        if let Object::Dict(d) = self.key(&cat, "Dests") {
            if let Some(v) = d.get_bytes(name) {
                return Some(v.clone());
            }
        }
        let names = self.dict(cat.get("Names")?);
        self.name_tree_get(names.get("Dests")?, name)
    }

    /// a named destination ("chapter1")
    pub fn named_dest(&self, name: &str) -> Option<Dest> {
        self.dest(&self.named_dest_obj(name.as_bytes())?)
    }

    /// the value of `key` in a name tree (7.9.6), through /Limits
    pub(crate) fn name_tree_get(&self, root: &Object, key: &[u8]) -> Option<Object> {
        let mut seen = BTreeSet::new();
        let mut node = root.clone();
        for _ in 0..64 {
            if let Some(r) = node.as_ref() {
                if !seen.insert(r) {
                    return None;
                }
            }
            let d = self.dict(&node);
            if let Object::Array(a) = self.key(&d, "Names") {
                let mut i = 0;
                while i + 1 < a.len() {
                    if let Object::String(k) = self.resolve(&a[i]) {
                        if k == key {
                            return Some(a[i + 1].clone());
                        }
                    }
                    i += 2;
                }
                return None;
            }
            let Object::Array(kids) = self.key(&d, "Kids") else { return None };
            let mut next = None;
            for k in &kids {
                let kd = self.dict(k);
                match self.key(&kd, "Limits") {
                    Object::Array(l) if l.len() >= 2 => {
                        let (lo, hi) = (self.resolve(&l[0]), self.resolve(&l[1]));
                        if let (Some(lo), Some(hi)) = (lo.as_bytes(), hi.as_bytes()) {
                            if lo <= key && key <= hi {
                                next = Some(k.clone());
                                break;
                            }
                        }
                    }
                    // a kid without limits: look inside
                    _ => {
                        if let Some(v) = self.name_tree_get_flat(k, key, &mut seen) {
                            return Some(v);
                        }
                    }
                }
            }
            node = next?;
        }
        None
    }

    fn name_tree_get_flat(&self, root: &Object, key: &[u8], seen: &mut BTreeSet<Ref>) -> Option<Object> {
        let mut found = None;
        self.walk_tree(root, "Names", seen, &mut |k, v| {
            if found.is_none() && k.as_bytes() == Some(key) {
                found = Some(v.clone());
            }
        });
        found
    }

    /// every (key, value) of a name or number tree (`leaf` "Names" or "Nums"), in order
    fn walk_tree(&self, root: &Object, leaf: &str, seen: &mut BTreeSet<Ref>, f: &mut dyn FnMut(&Object, &Object)) {
        let mut stack = alloc::vec![root.clone()];
        let mut n = 0;
        while let Some(node) = stack.pop() {
            n += 1;
            if n > BUDGET {
                return;
            }
            if let Some(r) = node.as_ref() {
                if !seen.insert(r) {
                    continue;
                }
            }
            let d = self.dict(&node);
            if let Object::Array(a) = self.key(&d, leaf) {
                for pair in a.chunks(2) {
                    if let [k, v] = pair {
                        f(&self.resolve(k), v);
                    }
                }
            }
            if let Object::Array(kids) = self.key(&d, "Kids") {
                stack.extend(kids.into_iter().rev());
            }
        }
    }

    pub(crate) fn action(&self, o: &Object) -> Option<Action> {
        let d = self.dict(o);
        let s = d.name("S")?.to_vec();
        Some(match s.as_slice() {
            b"GoTo" => Action::GoTo(self.dest(d.get("D")?)?),
            b"URI" => {
                let u = self.key(&d, "URI");
                let b = u.as_bytes()?;
                // a URI is 7-bit ASCII; some writers put UTF-8 there
                Action::Uri(String::from_utf8_lossy(b).into_owned())
            }
            b"GoToR" | b"GoToE" => Action::GoToFile {
                file: self.file_name(&self.key(&d, "F")).unwrap_or_default(),
                page: match self.key(&d, "D") {
                    Object::Array(a) => a.first().and_then(Object::as_int).and_then(|n| usize::try_from(n).ok()),
                    _ => None,
                },
            },
            b"Launch" => Action::Launch(self.file_name(&self.key(&d, "F")).unwrap_or_default()),
            b"Named" => Action::Named(d.name("N").map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default()),
            other => Action::Other(String::from_utf8_lossy(other).into_owned()),
        })
    }

    /// a file specification (7.11): a string, or a dictionary with /UF or /F
    fn file_name(&self, o: &Object) -> Option<String> {
        match o {
            Object::String(b) => Some(text_string(b)),
            Object::Dict(d) => self.text_of(d, "UF").or_else(|| self.text_of(d, "F")).or_else(|| self.text_of(d, "Unix")).or_else(|| self.text_of(d, "DOS")),
            _ => None,
        }
    }

    /// the outline (bookmarks)
    pub fn outlines(&self) -> Vec<Outline> {
        let root = self.dict(&self.catalog().get("Outlines").cloned().unwrap_or_default());
        let mut seen = BTreeSet::new();
        let mut budget = BUDGET;
        self.outline_level(root.get("First"), &mut seen, &mut budget, 0)
    }

    fn outline_level(&self, first: Option<&Object>, seen: &mut BTreeSet<Ref>, budget: &mut usize, depth: u32) -> Vec<Outline> {
        let mut out = Vec::new();
        let mut cur = first.cloned();
        while let Some(o) = cur.take() {
            if *budget == 0 || depth > 64 {
                break;
            }
            *budget -= 1;
            match o.as_ref() {
                Some(r) if !seen.insert(r) => break,
                None => break, // items are indirect objects
                _ => {}
            }
            let d = self.dict(&o);
            if d.is_empty() {
                break;
            }
            let action = match d.get("Dest") {
                Some(dest) => self.dest(dest).map(Action::GoTo),
                None => d.get("A").and_then(|a| self.action(a)),
            };
            let children = self.outline_level(d.get("First"), seen, budget, depth + 1);
            out.push(Outline {
                title: self.text_of(&d, "Title").unwrap_or_default(),
                action,
                open: d.get("Count").map(|c| self.resolve(c)).and_then(|c| c.as_int()).is_some_and(|c| c > 0),
                children,
            });
            cur = d.get("Next").cloned();
        }
        out
    }

    /// the annotations of a page
    pub fn annotations(&self, page: usize) -> Result<Vec<Annotation>, Error> {
        let p = self.page(page)?;
        let Object::Array(list) = self.key(&p.dict, "Annots") else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for a in list.iter().take(BUDGET) {
            let d = self.dict(a);
            let Some(sub) = d.name("Subtype") else { continue };
            let action = match d.get("A") {
                Some(act) => self.action(act),
                None => d.get("Dest").and_then(|x| self.dest(x)).map(Action::GoTo),
            };
            out.push(Annotation {
                subtype: String::from_utf8_lossy(sub).into_owned(),
                rect: rect_of(&self.key(&d, "Rect")),
                contents: self.text_of(&d, "Contents"),
                author: self.text_of(&d, "T"),
                modified: self.text_of(&d, "M").and_then(|t| parse_date(&t)),
                action,
                obj: a.as_ref(),
            });
        }
        Ok(out)
    }

    /// the links of a page: (rectangle, action)
    pub fn links(&self, page: usize) -> Result<Vec<([f64; 4], Action)>, Error> {
        Ok(self.annotations(page)?.into_iter().filter(|a| a.subtype == "Link").filter_map(|a| Some((a.rect, a.action?))).collect())
    }

    /// the fields of the interactive form (the AcroForm), terminal fields only
    pub fn fields(&self) -> Vec<Field> {
        let form = self.dict(&self.catalog().get("AcroForm").cloned().unwrap_or_default());
        let Object::Array(roots) = self.key(&form, "Fields") else { return Vec::new() };
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut annot_page: Option<alloc::collections::BTreeMap<Ref, usize>> = None;
        // (field, parent's full name, inherited FT, Ff, V); what is inherited is shared by the
        // kids (a /T of a megabyte with a million kids copied it for each before anything was
        // checked), and the names and values all fields give stay within max_stream bytes
        type Up = (Object, Rc<str>, Option<Rc<[u8]>>, Option<u32>, Option<Rc<Object>>, u32);
        let mut stack: Vec<Up> = roots.into_iter().rev().map(|o| (o, Rc::from(""), None, None, None, 0)).collect();
        let mut n = 0;
        let mut text_left = self.max_stream;
        while let Some((o, parent, ft, ff, v, depth)) = stack.pop() {
            n += 1;
            if n > BUDGET || depth > 64 {
                break;
            }
            if let Some(r) = o.as_ref() {
                if !seen.insert(r) {
                    continue;
                }
            }
            let d = self.dict(&o);
            if d.is_empty() {
                continue;
            }
            let name: Rc<str> = match self.text_of(&d, "T") {
                Some(t) if parent.is_empty() => Rc::from(t),
                Some(t) => Rc::from(format!("{parent}.{t}")),
                None => parent.clone(),
            };
            if name.len() > text_left {
                break;
            }
            let ft = d.name("FT").map(Rc::from).or(ft);
            let ff = self.key(&d, "Ff").as_int().map(|f| f as u32).or(ff);
            let v = d.get("V").map(|x| Rc::new(self.resolve(x))).or(v);
            // kids that are fields (they have /T); kids without are this field's widgets
            let kids = match self.key(&d, "Kids") {
                Object::Array(a) => a,
                _ => Vec::new(),
            };
            let field_kids: Vec<Object> = kids.iter().filter(|k| self.dict(k).contains("T")).cloned().collect();
            if !field_kids.is_empty() {
                for k in field_kids.into_iter().rev() {
                    if k.as_ref().is_some_and(|r| seen.contains(&r)) {
                        continue;
                    }
                    stack.push((k, name.clone(), ft.clone(), ff, v.clone(), depth + 1));
                }
                continue;
            }
            let flags = ff.unwrap_or(0);
            let kind = match ft.as_deref() {
                Some(b"Tx") => FieldKind::Text,
                Some(b"Btn") if flags & (1 << 16) != 0 => FieldKind::PushButton,
                Some(b"Btn") if flags & (1 << 15) != 0 => FieldKind::Radio,
                Some(b"Btn") => FieldKind::CheckBox,
                Some(b"Ch") if flags & (1 << 17) != 0 => FieldKind::ComboBox,
                Some(b"Ch") => FieldKind::ListBox,
                Some(b"Sig") => FieldKind::Signature,
                _ => FieldKind::Unknown,
            };
            let mut value = Vec::new();
            let mut signature = None;
            match v.as_deref() {
                Some(Object::String(b)) => value.push(text_string(b)),
                Some(Object::Name(nm)) => value.push(nm.to_text()),
                Some(Object::Array(a)) => {
                    for x in a {
                        if let Object::String(b) = self.resolve(x) {
                            value.push(text_string(&b));
                        }
                    }
                }
                Some(Object::Dict(sd)) if kind == FieldKind::Signature => signature = Some(self.signature(sd)),
                _ => {}
            }
            let options = match self.key(&d, "Opt") {
                Object::Array(a) => a
                    .iter()
                    .filter_map(|x| match self.resolve(x) {
                        Object::String(b) => Some(text_string(&b)),
                        // [export shown]
                        Object::Array(p) => p.get(1).or(p.first()).and_then(|e| self.resolve(e).as_bytes().map(text_string)),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let mut widgets = Vec::new();
            let wlist: Vec<Object> = if kids.is_empty() { alloc::vec![o.clone()] } else { kids };
            for w in &wlist {
                let wd = self.dict(w);
                if !wd.contains("Rect") {
                    continue;
                }
                let page = wd.get("P").and_then(Object::as_ref).and_then(|r| self.page_of(r)).or_else(|| {
                    let map = annot_page.get_or_insert_with(|| self.annot_pages());
                    map.get(&w.as_ref()?).copied()
                });
                widgets.push((page, rect_of(&self.key(&wd, "Rect"))));
            }
            let size = name.len() + value.iter().chain(&options).map(String::len).sum::<usize>();
            if size > text_left {
                break;
            }
            text_left -= size;
            out.push(Field { name: String::from(&*name), kind, value, flags, read_only: flags & 1 != 0, required: flags & 2 != 0, options, widgets, signature });
        }
        out
    }

    /// the page of each annotation that /Annots lists (for widgets without /P)
    fn annot_pages(&self) -> alloc::collections::BTreeMap<Ref, usize> {
        let mut m = alloc::collections::BTreeMap::new();
        for (i, p) in self.pages.iter().enumerate() {
            if let Object::Array(a) = self.key(&p.dict, "Annots") {
                for r in a.iter().filter_map(Object::as_ref) {
                    m.entry(r).or_insert(i);
                }
            }
        }
        m
    }

    fn signature(&self, d: &Dict) -> Signature {
        let name = |k: &str| d.name(k).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
        let br: Vec<i64> = match self.key(d, "ByteRange") {
            Object::Array(a) => a.iter().filter_map(Object::as_int).collect(),
            _ => Vec::new(),
        };
        let byte_range: Vec<(u64, u64)> = br.chunks_exact(2).filter_map(|p| Some((u64::try_from(p[0]).ok()?, u64::try_from(p[1]).ok()?))).collect();
        let len = self.data.len() as u64;
        let covers_file = byte_range.len() == 2 && byte_range[0].0 == 0 && byte_range[1].0.checked_add(byte_range[1].1) == Some(len) && byte_range[0].1 <= byte_range[1].0;
        Signature {
            filter: name("Filter"),
            sub_filter: name("SubFilter"),
            name: self.text_of(d, "Name"),
            reason: self.text_of(d, "Reason"),
            location: self.text_of(d, "Location"),
            time: self.text_of(d, "M").and_then(|t| parse_date(&t)),
            byte_range,
            contents: self.key(d, "Contents").as_bytes().map(<[u8]>::to_vec).unwrap_or_default(),
            covers_file,
        }
    }

    /// the embedded files: the /EmbeddedFiles name tree, then file attachment annotations
    pub fn attachments(&self) -> Vec<Attachment> {
        let mut out = Vec::new();
        let cat = self.catalog();
        let names = self.dict(&cat.get("Names").cloned().unwrap_or_default());
        let mut specs: Vec<Object> = Vec::new();
        if let Some(root) = names.get("EmbeddedFiles") {
            let mut seen = BTreeSet::new();
            self.walk_tree(root, "Names", &mut seen, &mut |_, v| specs.push(v.clone()));
        }
        for i in 0..self.page_count() {
            if let Ok(list) = self.annotations(i) {
                for a in list.into_iter().filter(|a| a.subtype == "FileAttachment") {
                    if let Some(r) = a.obj {
                        if let Some(fs) = self.dict(&Object::Ref(r)).get("FS") {
                            specs.push(fs.clone());
                        }
                    }
                }
            }
        }
        let mut seen = BTreeSet::new();
        for s in specs {
            if let Some(r) = s.as_ref() {
                if !seen.insert(r) {
                    continue;
                }
            }
            let d = self.dict(&s);
            let ef = self.dict(&d.get("EF").cloned().unwrap_or_default());
            let stream = match ef.get("UF").or_else(|| ef.get("F")).map(|o| self.resolve(o)) {
                Some(Object::Stream(st)) => Some(*st),
                _ => None,
            };
            let params = stream.as_ref().map(|st| self.dict(&st.dict.get("Params").cloned().unwrap_or_default())).unwrap_or_default();
            out.push(Attachment {
                name: self.file_name(&Object::Dict(d.clone())).unwrap_or_default(),
                description: self.text_of(&d, "Desc"),
                mime: stream.as_ref().and_then(|st| st.dict.name("Subtype")).map(|n| String::from_utf8_lossy(n).into_owned()),
                size: self.key(&params, "Size").as_int().and_then(|n| u64::try_from(n).ok()),
                modified: self.text_of(&params, "ModDate").and_then(|t| parse_date(&t)),
                relationship: d.name("AFRelationship").map(|n| String::from_utf8_lossy(n).into_owned()),
                stream,
            });
        }
        out
    }

    /// a page's label (12.4.2: "iv", "A-3"); None when the document sets no labels
    pub fn page_label(&self, page: usize) -> Option<String> {
        if page >= self.page_count() {
            return None;
        }
        let root = self.catalog().get("PageLabels")?.clone();
        let mut best: Option<(usize, Dict)> = None;
        let mut seen = BTreeSet::new();
        self.walk_tree(&root, "Nums", &mut seen, &mut |k, v| {
            if let Some(start) = k.as_int().and_then(|n| usize::try_from(n).ok()) {
                if start <= page && best.as_ref().is_none_or(|b| b.0 <= start) {
                    best = Some((start, self.dict(v)));
                }
            }
        });
        let (start, d) = best?;
        let n = ((page - start) as i64).saturating_add(d.int("St").unwrap_or(1).max(1));
        let prefix = self.text_of(&d, "P").unwrap_or_default();
        let num = match d.name("S") {
            Some(b"D") => format!("{n}"),
            Some(b"R") => roman(n).to_uppercase(),
            Some(b"r") => roman(n),
            Some(b"A") => letters(n).to_uppercase(),
            Some(b"a") => letters(n),
            _ => String::new(),
        };
        Some(prefix + &num)
    }
}

fn roman(mut n: i64) -> String {
    let mut s = String::new();
    if !(1..=100_000).contains(&n) {
        return format!("{n}");
    }
    for (v, t) in [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")] {
        while n >= v {
            s.push_str(t);
            n -= v;
        }
    }
    s
}

/// a, b, ..., z, aa, bb, ... (12.4.2)
fn letters(n: i64) -> String {
    if !(1..=10_000).contains(&n) {
        return format!("{n}");
    }
    let c = (b'a' + ((n - 1) % 26) as u8) as char;
    core::iter::repeat_n(c, ((n - 1) / 26 + 1) as usize).collect()
}
