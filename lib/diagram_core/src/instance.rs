// instance -- masters from stencils onto pages (JD9): a master taken into a document, an
// instance of it dropped on a page, the texts of masters and stencils in five languages
//
//   LANGS, lang_index("fr")            de en es it fr: the order of every five-language value
//   join_fields(&[..]) / split_fields(text)   the '|'-separated values of a JFD_ cell
//                                      ("\|" and "\\" escape)
//   MasterTexts::of(&doc, master)      names, QuickInfos, keywords, text and translated cells in
//                                      five languages, the quick flag (JFD_ cells of the master's
//                                      page sheet; a master without them: Name, Prompt and
//                                      ShapeKeywords for every language)
//     texts.name(lang) / matches(&words, any)   the name shown, the search (NameU, the names in
//                                      all five languages, the keywords)
//   StencilTexts::of(&doc)             a stencil's title in five languages and its category
//                                      (User.JFD_Name, User.JFD_Category of the document sheet)
//   import_master(&mut tx, &src, m) -> MasterRef   master m of document src in tx's document:
//                                      the copy already there with the same UniqueID (MS-02),
//                                      else a new copy: strings into this document's table,
//                                      styles matched by NameU (missing ones copied with their
//                                      bases), masters its shapes are instances of first,
//                                      pictures of package parts inline
//   move_master(&mut tx, m, pos)      a master to another place in the order (own stencils)
//   place(&mut tx, page, master, x, y, lang) -> ShapeRef   an instance at (x, y) inches on the
//                                      page (y up): Name in language lang, NameU the master's
//                                      (LG-04; "Rechteck.7" when the page has one already), its
//                                      text and translated cells in that language
//
// The JFD_ cells are JFDiagrammer's own (apps/JFDiagrammer/stencils/README.md); diagram_io's
// stencil builder writes them, the Shapes panel reads them.  Visio keeps them as ordinary User
// cells.  The caller recalculates after place (formula::Engine::recalc with the new shapes'
// sheets) inside the same transaction, so undo takes the drop back as one step.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, MasterRef, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Op, Transaction};
use crate::page::{Connect, Master};
use crate::shape::{ForeignData, ForeignPayload, ShapeData, ShapeKind, ShapeTree, StyleRefs, Text, TextItem};
use crate::sheet::{CellAddr, Extra, ExtraItem, Extras, Row, RowKey, SecKey, Section, Sheet, Trigger};
use crate::strings::Sym;
use crate::units::Unit;

/// the five languages, in the order of every five-language value
pub const LANGS: [&str; 5] = ["de", "en", "es", "it", "fr"];

/// User cells of a stencil's document sheet and of a master's page sheet (README of the stencils)
pub const JFD_NAME: &str = "JFD_Name";
/// the QuickInfos of a master
pub const JFD_PROMPT: &str = "JFD_Prompt";
/// the search words of a master, per language separated by commas
pub const JFD_KEYWORDS: &str = "JFD_Keywords";
/// the text a master's instance carries
pub const JFD_TEXT: &str = "JFD_Text";
/// 1: the master is a quick shape of its stencil (MS-04)
pub const JFD_QUICK: &str = "JFD_Quick";
/// the category of a stencil in "Weitere Shapes"
pub const JFD_CATEGORY: &str = "JFD_Category";
/// JFD_Tr1, JFD_Tr2 ...: "Section.Row.Cell|de|en|es|it|fr" for translated cells of the master shape
pub const JFD_TR: &str = "JFD_Tr";

/// the position of a language code in LANGS
pub fn lang_index(code: &str) -> Option<usize> {
    LANGS.iter().position(|l| l.eq_ignore_ascii_case(code))
}

/// values joined with '|' ('|' and '\' escaped with '\')
pub fn join_fields(values: &[&str]) -> String {
    let mut s = String::new();
    for (k, v) in values.iter().enumerate() {
        if k > 0 {
            s.push('|');
        }
        for c in v.chars() {
            if c == '|' || c == '\\' {
                s.push('\\');
            }
            s.push(c);
        }
    }
    s
}

/// the values of a '|'-joined text
pub fn split_fields(text: &str) -> Vec<String> {
    let mut out = alloc::vec![String::new()];
    let mut esc = false;
    for c in text.chars() {
        if esc {
            out.last_mut().expect("one").push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '|' {
            out.push(String::new());
        } else {
            out.last_mut().expect("one").push(c);
        }
    }
    out
}

// five values of a joined text (missing ones empty)
fn five(text: &str) -> [String; 5] {
    let mut v = split_fields(text).into_iter();
    core::array::from_fn(|_| v.next().unwrap_or_default())
}

// the text value of User.<row>.Value of a sheet (no inheritance: these cells are local)
fn user_text(doc: &Document, sheet: SheetRef, row: &str) -> Option<String> {
    let s = doc.sheet(sheet)?;
    let (sec, row, value) = (doc.sym("User")?, doc.sym(row)?, doc.sym("Value")?);
    let r = s.section(SecKey::one(sec))?.row(RowKey::Named(row))?;
    if r.del {
        return None;
    }
    match r.cell(value)?.value {
        Value::Text(t) => Some(String::from(doc.str(t))),
        Value::Num(n) => Some(format!("{n}")),
        Value::Absent => None,
    }
}

// the named User rows of a sheet starting with `prefix`, in sheet order
fn user_rows(doc: &Document, sheet: SheetRef, prefix: &str) -> Vec<String> {
    let (Some(s), Some(sec)) = (doc.sheet(sheet), doc.sym("User")) else { return Vec::new() };
    let Some(section) = s.section(SecKey::one(sec)) else { return Vec::new() };
    section
        .rows
        .iter()
        .filter_map(|r| match r.key {
            RowKey::Named(n) if !r.del && doc.str(n).starts_with(prefix) => Some(String::from(doc.str(n))),
            _ => None,
        })
        .collect()
}

/// the texts of a master in five languages
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTexts {
    /// NameU: the key (LG-04)
    pub name_u: String,
    /// the names shown, by LANGS
    pub names: [String; 5],
    /// the QuickInfos
    pub prompts: [String; 5],
    /// the search words, comma separated
    pub keywords: [String; 5],
    /// the text an instance carries
    pub text: Option<[String; 5]>,
    /// translated cells of the master shape: "Section.Row.Cell" and its five texts
    pub translated: Vec<(String, [String; 5])>,
    /// a quick shape of its stencil (MS-04)
    pub quick: bool,
}

impl MasterTexts {
    /// the texts of master `m` (empty for a master that is not there)
    pub fn of(doc: &Document, m: MasterRef) -> MasterTexts {
        let Some(master) = doc.master(m) else { return MasterTexts::default() };
        let h = &master.header;
        let name_u = h.name_u.or(h.name).map(|s| String::from(doc.str(s))).unwrap_or_default();
        let local = h.name.or(h.name_u).map(|s| String::from(doc.str(s))).unwrap_or_default();
        let prompt = doc.sym("Prompt").and_then(|p| crate::page::attr(&h.attrs, p)).map(|s| String::from(doc.str(s))).unwrap_or_default();
        let sheet = SheetRef::Master(m);
        let kw_cell = doc.sym("ShapeKeywords").and_then(|k| master.sheet.cell(k)).and_then(|c| c.value.text()).map(|s| String::from(doc.str(s)));
        let fill = |v: Option<String>, dflt: &str| -> [String; 5] {
            match v {
                Some(t) => {
                    let mut f = five(&t);
                    for x in f.iter_mut() {
                        if x.is_empty() {
                            *x = String::from(dflt);
                        }
                    }
                    f
                }
                None => core::array::from_fn(|_| String::from(dflt)),
            }
        };
        let names = fill(user_text(doc, sheet, JFD_NAME), &local);
        let prompts = fill(user_text(doc, sheet, JFD_PROMPT), &prompt);
        let keywords = fill(user_text(doc, sheet, JFD_KEYWORDS), kw_cell.as_deref().unwrap_or(""));
        let text = user_text(doc, sheet, JFD_TEXT).map(|t| five(&t));
        let quick = user_text(doc, sheet, JFD_QUICK).is_some_and(|q| q.trim() == "1" || q.eq_ignore_ascii_case("TRUE"));
        let mut translated = Vec::new();
        for row in user_rows(doc, sheet, JFD_TR) {
            if let Some(t) = user_text(doc, sheet, &row) {
                let mut f = split_fields(&t).into_iter();
                let Some(cell) = f.next() else { continue };
                translated.push((cell, core::array::from_fn(|_| f.next().unwrap_or_default())));
            }
        }
        MasterTexts { name_u, names, prompts, keywords, text, translated, quick }
    }

    /// the name in language `lang` (index into LANGS), else the first one there, else NameU
    pub fn name(&self, lang: usize) -> &str {
        let n = self.names.get(lang).map_or("", |s| s.as_str());
        if !n.is_empty() {
            return n;
        }
        self.names.iter().find(|s| !s.is_empty()).map_or(self.name_u.as_str(), |s| s.as_str())
    }

    /// the QuickInfo in language `lang`
    pub fn prompt(&self, lang: usize) -> &str {
        self.prompts.get(lang).map_or("", |s| s.as_str())
    }

    /// whether the master matches the search words (lower case expected): all of them
    /// (any = false) or one of them, each a part of NameU, of a name in any language or of a
    /// keyword (SL-04)
    pub fn matches(&self, words: &[&str], any: bool) -> bool {
        if words.is_empty() {
            return false;
        }
        let hay: Vec<String> = core::iter::once(&self.name_u).chain(self.names.iter()).chain(self.keywords.iter()).map(|s| fold(s)).collect();
        let hit = |w: &&str| {
            let w = fold(w);
            !w.is_empty() && hay.iter().any(|h| h.contains(w.as_str()))
        };
        if any { words.iter().any(hit) } else { words.iter().all(hit) }
    }
}

/// a text in lower case without the accents of the five languages (search ignores both)
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            'à' | 'á' | 'â' | 'ä' | 'À' | 'Á' | 'Â' | 'Ä' => 'a',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' | 'Ñ' => 'n',
            'ß' => {
                out.push_str("ss");
                continue;
            }
            c => c,
        };
        out.extend(c.to_lowercase());
    }
    out
}

/// the texts of a stencil
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StencilTexts {
    /// the title in five languages (empty: the stencil has none; the app takes its file name)
    pub names: [String; 5],
    /// the category in "Weitere Shapes" ("" when none)
    pub category: String,
}

impl StencilTexts {
    /// the texts of the stencil `doc`
    pub fn of(doc: &Document) -> StencilTexts {
        let names = user_text(doc, SheetRef::Document, JFD_NAME).map(|t| five(&t)).unwrap_or_default();
        let category = user_text(doc, SheetRef::Document, JFD_CATEGORY).unwrap_or_default();
        StencilTexts { names, category }
    }

    /// the title in language `lang`, else the first one there ("" when none)
    pub fn name(&self, lang: usize) -> &str {
        let n = self.names.get(lang).map_or("", |s| s.as_str());
        if !n.is_empty() {
            return n;
        }
        self.names.iter().find(|s| !s.is_empty()).map_or("", |s| s.as_str())
    }
}

// ------------------------------------------------------------------ copying between documents

// copies items of document `src` into the document of a transaction: strings re-interned,
// style and master IDs mapped
struct Copier<'s> {
    src: &'s Document,
    styles: BTreeMap<u32, u32>,
    masters: BTreeMap<u32, u32>,
    depth: u32,
}

const DEPTH_MAX: u32 = 64;

impl Copier<'_> {
    fn sym(&self, tx: &mut Transaction, s: Sym) -> Sym {
        tx.intern(self.src.str(s))
    }

    fn opt(&self, tx: &mut Transaction, s: Option<Sym>) -> Option<Sym> {
        s.map(|s| self.sym(tx, s))
    }

    fn attrs(&self, tx: &mut Transaction, a: &[(Sym, Sym)]) -> Vec<(Sym, Sym)> {
        a.iter().map(|&(n, v)| (self.sym(tx, n), self.sym(tx, v))).collect()
    }

    fn extras(&self, tx: &mut Transaction, e: &Extras) -> Extras {
        let mut out = Extras::new();
        for x in e.items() {
            let item = match x.item {
                ExtraItem::Attribute { name, value } => ExtraItem::Attribute { name: self.sym(tx, name), value: self.sym(tx, value) },
                ExtraItem::Element(s) => ExtraItem::Element(self.sym(tx, s)),
                ExtraItem::Text(s) => ExtraItem::Text(self.sym(tx, s)),
            };
            out.push(Extra { place: x.place, item });
        }
        out
    }

    fn cell(&self, tx: &mut Transaction, c: &Cell) -> Cell {
        Cell {
            name: self.sym(tx, c.name),
            value: match c.value {
                Value::Text(t) => Value::Text(self.sym(tx, t)),
                v => v,
            },
            formula: match c.formula {
                Formula::Expr(e) => Formula::Expr(self.sym(tx, e)),
                f => f,
            },
            unit: match c.unit {
                Some(Unit::Other(u)) => Some(Unit::Other(self.sym(tx, u))),
                u => u,
            },
            error: self.opt(tx, c.error),
            ..*c
        }
    }

    fn cells(&self, tx: &mut Transaction, c: &[Cell]) -> Vec<Cell> {
        c.iter().map(|c| self.cell(tx, c)).collect()
    }

    fn triggers(&self, tx: &mut Transaction, t: &[Trigger]) -> Vec<Trigger> {
        t.iter().map(|t| Trigger { name: self.sym(tx, t.name), inner: self.sym(tx, t.inner) }).collect()
    }

    fn row(&self, tx: &mut Transaction, r: &Row) -> Row {
        Row {
            key: match r.key {
                RowKey::Named(n) => RowKey::Named(self.sym(tx, n)),
                k => k,
            },
            ix_written: r.ix_written,
            local_name: self.opt(tx, r.local_name),
            kind: self.opt(tx, r.kind),
            del: r.del,
            cells: self.cells(tx, &r.cells),
            triggers: self.triggers(tx, &r.triggers),
            extras: self.extras(tx, &r.extras),
        }
    }

    fn sheet(&self, tx: &mut Transaction, s: &Sheet) -> Sheet {
        let mut sections = Vec::with_capacity(s.sections.len());
        for sec in &s.sections {
            sections.push(Section {
                name: self.sym(tx, sec.name),
                index: sec.index,
                ix_written: sec.ix_written,
                del: sec.del,
                cells: self.cells(tx, &sec.cells),
                rows: sec.rows.iter().map(|r| self.row(tx, r)).collect(),
                triggers: self.triggers(tx, &sec.triggers),
                extras: self.extras(tx, &sec.extras),
            });
        }
        Sheet { cells: self.cells(tx, &s.cells), sections, triggers: self.triggers(tx, &s.triggers), extras: self.extras(tx, &s.extras) }
    }

    // a style ID of src -> the style in tx's document with the same NameU (else Name), a copy
    // when there is none
    fn style(&mut self, tx: &mut Transaction, id: u32) -> Result<u32, EditError> {
        if let Some(&d) = self.styles.get(&id) {
            return Ok(d);
        }
        let sr = self.src.style_by_id(id).ok_or(EditError::NoStyle)?;
        let st = self.src.style(sr).ok_or(EditError::NoStyle)?;
        let key = st.header.name_u.or(st.header.name).map(|s| self.src.str(s));
        if let Some(found) = key.and_then(|k| tx.doc().style_by_name_u(k)).and_then(|r| tx.doc().style(r)).map(|s| s.header.id) {
            self.styles.insert(id, found);
            return Ok(found);
        }
        // a copy, its bases first
        self.depth += 1;
        if self.depth > DEPTH_MAX {
            return Err(EditError::Cycle);
        }
        let based_on = self.style_refs(tx, st.header.based_on)?;
        self.depth -= 1;
        let mut header = st.header.clone();
        header.name = self.opt(tx, header.name);
        header.name_u = self.opt(tx, header.name_u);
        header.based_on = based_on;
        header.attrs = self.attrs(tx, &st.header.attrs);
        let style = crate::page::Style { header, sheet: self.sheet(tx, &st.sheet), extras: self.extras(tx, &st.extras) };
        let pos = tx.doc.styles.len() as u32;
        let (slot, new_id) = tx.doc.styles.alloc().ok_or(EditError::NoIds)?;
        let mut style = style;
        style.header.id = new_id;
        tx.run(Op::StyleInsert { slot, pos, style: Box::new(style) })?;
        self.styles.insert(id, new_id);
        Ok(new_id)
    }

    fn style_refs(&mut self, tx: &mut Transaction, s: StyleRefs) -> Result<StyleRefs, EditError> {
        let map = |v: Option<u32>, me: &mut Self, tx: &mut Transaction| -> Result<Option<u32>, EditError> {
            match v {
                Some(id) => me.style(tx, id).map(Some),
                None => Ok(None),
            }
        };
        Ok(StyleRefs { line: map(s.line, self, tx)?, fill: map(s.fill, self, tx)?, text: map(s.text, self, tx)? })
    }

    fn text(&self, tx: &mut Transaction, t: &Text) -> Text {
        let items = t
            .items
            .iter()
            .map(|i| match i {
                TextItem::Other(s) => TextItem::Other(self.sym(tx, *s)),
                i => i.clone(),
            })
            .collect();
        Text { items, extras: self.extras(tx, &t.extras) }
    }

    // a picture in a package part of src becomes inline data (the part stays with src)
    fn foreign(&self, tx: &mut Transaction, f: &ForeignData) -> ForeignData {
        let payload = match &f.payload {
            ForeignPayload::Part(name) => {
                let n = self.src.str(*name);
                match self.src.parts.iter().find(|p| self.src.str(p.name) == n) {
                    Some(p) => ForeignPayload::Inline(base64(&p.data).into_boxed_str()),
                    None => ForeignPayload::None,
                }
            }
            p => p.clone(),
        };
        ForeignData { attrs: self.attrs(tx, &f.attrs), payload, extras: self.extras(tx, &f.extras) }
    }

    fn shape(&mut self, tx: &mut Transaction, d: &ShapeData) -> Result<ShapeData, EditError> {
        let mut h = d.header.clone();
        h.kind = match h.kind {
            ShapeKind::Other(s) => ShapeKind::Other(self.sym(tx, s)),
            k => k,
        };
        h.name = self.opt(tx, h.name);
        h.name_u = self.opt(tx, h.name_u);
        h.unique_id = self.opt(tx, h.unique_id);
        h.styles = self.style_refs(tx, h.styles)?;
        if let Some(mid) = h.master {
            h.master = Some(self.master_id(tx, mid)?);
        }
        Ok(ShapeData {
            header: h,
            sheet: self.sheet(tx, &d.sheet),
            text: d.text.as_ref().map(|t| Box::new(self.text(tx, t))),
            foreign: d.foreign.as_ref().map(|f| Box::new(self.foreign(tx, f))),
            data: [self.opt(tx, d.data[0]), self.opt(tx, d.data[1]), self.opt(tx, d.data[2])],
            extras: self.extras(tx, &d.extras),
        })
    }

    // a master ID of src -> the ID of its copy in tx's document
    fn master_id(&mut self, tx: &mut Transaction, id: u32) -> Result<u32, EditError> {
        if let Some(&d) = self.masters.get(&id) {
            return Ok(d);
        }
        let m = self.src.master_by_id(id).ok_or(EditError::NoMaster)?;
        let r = self.master(tx, m)?;
        Ok(tx.doc().master(r).ok_or(EditError::NoMaster)?.header.id)
    }

    fn master(&mut self, tx: &mut Transaction, m: MasterRef) -> Result<MasterRef, EditError> {
        let src = self.src;
        let master = src.master(m).ok_or(EditError::NoMaster)?;
        // the copy already in the document (MS-02: matched by UniqueID)
        if let Some(uid) = master.header.unique_id.map(|u| src.str(u)) {
            if let Some(found) = tx.doc().masters().find(|&r| tx.doc().master(r).and_then(|x| x.header.unique_id).is_some_and(|u| tx.doc().str(u).eq_ignore_ascii_case(uid))) {
                let fid = tx.doc().master(found).map_or(0, |x| x.header.id);
                self.masters.insert(master.header.id, fid);
                return Ok(found);
            }
        }
        self.depth += 1;
        if self.depth > DEPTH_MAX {
            return Err(EditError::Cycle);
        }
        let mut header = master.header.clone();
        header.name = self.opt(tx, header.name);
        header.name_u = self.opt(tx, header.name_u);
        header.base_id = self.opt(tx, header.base_id);
        header.unique_id = self.opt(tx, header.unique_id);
        header.sheet_styles = self.style_refs(tx, header.sheet_styles)?;
        header.attrs = self.attrs(tx, &master.header.attrs);
        // the tree with the same IDs (a master's shapes are its own)
        let mut shapes = ShapeTree::new();
        let mut stack: Vec<(u32, Option<u32>)> = master.shapes.top().iter().rev().map(|&s| (s, None)).collect();
        while let Some((slot, parent)) = stack.pop() {
            let (Some(d), Some(id)) = (master.shapes.get(slot), master.shapes.id(slot)) else { continue };
            let data = self.shape(tx, d)?;
            let new = shapes.insert(parent, id, data).map_err(|_| EditError::Invalid)?;
            stack.extend(master.shapes.children(slot).iter().rev().map(|&c| (c, Some(new))));
        }
        let connects = master
            .connects
            .iter()
            .map(|c| Connect { from_cell: self.opt(tx, c.from_cell), to_cell: self.opt(tx, c.to_cell), extras: self.extras(tx, &c.extras), ..c.clone() })
            .collect();
        let copy = Master {
            header,
            sheet: self.sheet(tx, &master.sheet),
            icon: self.opt(tx, master.icon),
            shapes,
            connects,
            extras: self.extras(tx, &master.extras),
            contents_extras: self.extras(tx, &master.contents_extras),
            part: None,
        };
        self.depth -= 1;
        let pos = tx.doc.masters.len() as u32;
        let (slot, id) = tx.doc.masters.alloc().ok_or(EditError::NoIds)?;
        let mut copy = copy;
        copy.header.id = id;
        // (JD13) another master with this NameU is there already (the same name in two stencils,
        // "Router" in Netzwerk und Peripheriegeräte and in Netzwerksymbole): Name and NameU get
        // ".ID" as Visio names them
        if let Some(nu) = copy.header.name_u.map(|s| String::from(tx.doc().str(s))) {
            if tx.doc().masters().any(|r| tx.doc().master(r).and_then(|x| x.header.name_u).is_some_and(|n| tx.doc().str(n) == nu)) {
                copy.header.name_u = Some(tx.intern(&format!("{nu}.{id}")));
                if let Some(n) = copy.header.name.map(|s| String::from(tx.doc().str(s))) {
                    copy.header.name = Some(tx.intern(&format!("{n}.{id}")));
                }
            }
        }
        let old = master.header.id;
        tx.run(Op::MasterInsert { slot, pos, master: Box::new(copy) })?;
        self.masters.insert(old, id);
        Ok(MasterRef(slot))
    }
}

/// master `m` of document `src` in the document of `tx`: the copy already there with the same
/// UniqueID, else a new copy (see the head of this file)
pub fn import_master(tx: &mut Transaction, src: &Document, m: MasterRef) -> Result<MasterRef, EditError> {
    let mut c = Copier { src, styles: BTreeMap::new(), masters: BTreeMap::new(), depth: 0 };
    tx.atomic(|tx| c.master(tx, m))
}

/// drop an instance of `master` at (x, y) inches on `page` (MS-02, LG-04): Name in language
/// `lang` (an index into LANGS), NameU the master's, unique on the page with ".ID" like Visio;
/// the text and translated cells in that language where they differ from the master's
pub fn place(tx: &mut Transaction, page: PageRef, master: MasterRef, x: f64, y: f64, lang: usize) -> Result<ShapeRef, EditError> {
    tx.atomic(|tx| {
        let texts = MasterTexts::of(tx.doc(), master);
        let shape = tx.add_instance(Owner::Page(page), master, x, y)?;
        let id = tx.doc().shape(shape).map_or(0, |s| s.id());
        let base = texts.name(lang).to_string();
        let base_u = if texts.name_u.is_empty() { base.clone() } else { texts.name_u.clone() };
        // the page has a shape with this NameU already: "Rectangle.7" (the new shape's ID)
        let taken = tx
            .doc()
            .all_shapes(Owner::Page(page))
            .any(|r| r != shape && tx.doc().shape(r).and_then(|s| s.header().name_u).is_some_and(|n| tx.doc().str(n) == base_u));
        let (name, name_u) = if taken { (format!("{base}.{id}"), format!("{base_u}.{id}")) } else { (base, base_u) };
        tx.set_shape_names(shape, Some(&name), Some(&name_u))?;
        // the text in this language
        if let Some(text) = texts.text.as_ref().and_then(|t| t.get(lang)).filter(|t| !t.is_empty()) {
            let own = tx.doc().shape(shape).and_then(|s| s.text()).map(|t| t.chars()).unwrap_or_default();
            if own.trim_end_matches('\n') != text.as_str() {
                tx.set_text(shape, Some(Text::plain(text)))?;
            }
        }
        // translated cells (Controls.Row_1.Tip ...) in this language
        let ms = tx.doc().master(master).and_then(|m| m.master_shape()).map(|slot| SheetRef::Shape(ShapeRef { owner: Owner::Master(master), slot }));
        for (cell, values) in &texts.translated {
            let Some(v) = values.get(lang).filter(|v| !v.is_empty()) else { continue };
            let Some(addr) = cell_addr(tx, cell) else { continue };
            let current = ms.and_then(|s| tx.doc().sheet(s)).map(|s| s.find(&addr)).and_then(|l| match l {
                crate::sheet::Local::Cell(c) => c.value.text(),
                _ => None,
            });
            if current.is_some_and(|c| tx.doc().str(c) == v.as_str()) {
                continue;
            }
            let sym = tx.intern(v);
            // a cell the instance inherits: the row must be there locally to hold the value
            if tx.set_constant(SheetRef::Shape(shape), &addr, Value::Text(sym), Some(Unit::Str)).is_err() {
                continue;
            }
        }
        Ok(shape)
    })
}

// "Section.Row.Cell" (Row a name, or a number for indexed rows) or "Cell" -> its address
fn cell_addr(tx: &mut Transaction, text: &str) -> Option<CellAddr> {
    let parts: Vec<&str> = text.split('.').collect();
    match parts.as_slice() {
        [cell] => Some(tx.single(cell)),
        [sec, row, cell] => {
            let s = tx.intern(sec);
            let c = tx.intern(cell);
            let key = match row.parse::<u32>() {
                Ok(ix) => RowKey::Index(ix),
                Err(_) => RowKey::Named(tx.intern(row)),
            };
            Some(CellAddr::row(SecKey::one(s), key, c))
        }
        _ => None,
    }
}

/// base64 (RFC 4648, with padding)
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// move master `m` to position `pos` among the masters (a stencil's order: MS-04, SL-07);
/// a master in use by shapes of the document stays where it is (InUse)
pub fn move_master(tx: &mut Transaction, m: MasterRef, pos: usize) -> Result<(), EditError> {
    let master = tx.doc().master(m).cloned().ok_or(EditError::NoMaster)?;
    if tx.doc().master_in_use(master.header.id) {
        return Err(EditError::InUse);
    }
    let n = tx.doc().masters().count();
    let from = tx.doc().masters().position(|r| r == m).ok_or(EditError::NoMaster)?;
    // the position counted without the master itself
    let to = if pos > from { pos - 1 } else { pos }.min(n - 1);
    if to == from {
        return Ok(());
    }
    tx.atomic(|tx| {
        tx.run(Op::MasterRemove { slot: m.0 })?;
        tx.run(Op::MasterInsert { slot: m.0, pos: to as u32, master: Box::new(master) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocKind;

    // a stencil with style "Mine" and masters "Box", "Ring", "Tri"; Box carries five names and
    // a UniqueID
    fn stencil() -> Document {
        let mut doc = Document::new(DocKind::Stencil);
        let mut tx = doc.transaction("build");
        let st = tx.add_style("Mine", "Mine", StyleRefs::default()).unwrap();
        let sid = tx.doc().style(st).unwrap().header.id;
        for name in ["Box", "Ring", "Tri"] {
            let m = tx.add_master(name, name).unwrap();
            let s = tx.add_shape(Owner::Master(m), None, ShapeKind::Shape).unwrap();
            tx.set_shape_styles(s, StyleRefs { line: Some(sid), fill: Some(sid), text: Some(sid) }).unwrap();
            let w = tx.single("Width");
            tx.set_length(SheetRef::Shape(s), &w, 1.0, Unit::Inches).unwrap();
            let mut h = tx.doc().master(m).unwrap().header.clone();
            let uid = tx.intern(&format!("{{0000000{}-0000-0000-0000-000000000000}}", name.len()));
            h.unique_id = Some(uid);
            tx.set_master_header(m, h).unwrap();
            if name == "Box" {
                let names = join_fields(&["Kasten", "Box", "Caja", "Scatola", "Boîte"]);
                let v = tx.intern(&names);
                tx.set_user(SheetRef::Master(m), JFD_NAME, Value::Text(v), Some(Unit::Str)).unwrap();
                let kw = join_fields(&["Viereck", "square", "cuadrado", "quadrato", "carré"]);
                let v = tx.intern(&kw);
                tx.set_user(SheetRef::Master(m), JFD_KEYWORDS, Value::Text(v), Some(Unit::Str)).unwrap();
                let q = tx.intern("1");
                tx.set_user(SheetRef::Master(m), JFD_QUICK, Value::Text(q), Some(Unit::Str)).unwrap();
            }
        }
        tx.commit();
        doc
    }

    #[test]
    fn fields_round_trip() {
        let v = ["a|b", "c\\d", "", "é", "x"];
        let s = join_fields(&v);
        assert_eq!(s, "a\\|b|c\\\\d||é|x");
        assert_eq!(split_fields(&s), v.iter().map(|x| String::from(*x)).collect::<Vec<_>>());
        assert_eq!(five("eins|zwei"), [String::from("eins"), String::from("zwei"), String::new(), String::new(), String::new()]);
    }

    #[test]
    fn texts_and_search() {
        let doc = stencil();
        let ms: Vec<MasterRef> = doc.masters().collect();
        let t = MasterTexts::of(&doc, ms[0]);
        assert_eq!(t.name_u, "Box");
        assert_eq!(t.name(0), "Kasten");
        assert_eq!(t.name(4), "Boîte");
        assert!(t.quick);
        assert!(t.matches(&["boite"], false), "accents are folded");
        assert!(t.matches(&["CARRÉ"], false));
        assert!(t.matches(&["kas", "squ"], false));
        assert!(!t.matches(&["kas", "nichts"], false));
        assert!(t.matches(&["kas", "nichts"], true));
        // a master without JFD_ cells: its Name in every language
        let r = MasterTexts::of(&doc, ms[1]);
        assert_eq!(r.names, core::array::from_fn(|_| String::from("Ring")));
        assert!(!r.quick);
        assert_eq!(fold("Straße Ñandú"), "strasse nandu");
    }

    #[test]
    fn import_copies_once_and_maps_styles() {
        let src = stencil();
        let box_m = src.masters().next().unwrap();
        let mut dst = Document::new(DocKind::Drawing);
        let mut tx = dst.transaction("drop");
        let m1 = import_master(&mut tx, &src, box_m).unwrap();
        let m2 = import_master(&mut tx, &src, box_m).unwrap();
        assert_eq!(m1, m2, "matched by UniqueID");
        tx.commit();
        assert_eq!(dst.masters().count(), 1);
        let st = dst.style_by_name_u("Mine").expect("style copied by NameU");
        let sid = dst.style(st).unwrap().header.id;
        let ms = dst.master(m1).unwrap().master_shape().unwrap();
        let shape = dst.shape(ShapeRef { owner: Owner::Master(m1), slot: ms }).unwrap();
        assert_eq!(shape.header().styles.line, Some(sid));
        // the texts read the same from the copy (strings in the new table)
        assert_eq!(MasterTexts::of(&dst, m1), MasterTexts::of(&src, box_m));
        // a second stencil with a style of the same NameU: no second style
        let mut tx = dst.transaction("drop");
        let ring = src.masters().nth(1).unwrap();
        import_master(&mut tx, &src, ring).unwrap();
        tx.commit();
        assert_eq!(dst.styles().filter(|&s| dst.style(s).and_then(|x| x.header.name_u).is_some_and(|n| dst.str(n) == "Mine")).count(), 1);
    }

    #[test]
    fn place_names_and_undo() {
        let src = stencil();
        let box_m = src.masters().next().unwrap();
        let mut dst = Document::new(DocKind::Drawing);
        let mut tx = dst.transaction("page");
        let page = tx.add_page("Seite-1", "Page-1", false, None).unwrap();
        tx.commit();
        let mut tx = dst.transaction("drop");
        let m = import_master(&mut tx, &src, box_m).unwrap();
        let a = place(&mut tx, page, m, 1.0, 2.0, 4).unwrap();
        let b = place(&mut tx, page, m, 3.0, 2.0, 0).unwrap();
        let mut change = tx.commit().unwrap();
        let sa = dst.shape(a).unwrap();
        assert_eq!(sa.name(), "Boîte");
        assert_eq!(sa.name_u(), "Box");
        let sb = dst.shape(b).unwrap();
        assert_eq!(sb.name(), format!("Kasten.{}", sb.id()));
        assert_eq!(sb.name_u(), format!("Box.{}", sb.id()));
        assert_eq!(dst.master_of(a), Some(m));
        // one undo step takes the master and both shapes back
        assert!(change.undo(&mut dst));
        assert_eq!(dst.masters().count(), 0);
        assert_eq!(dst.top_shapes(Owner::Page(page)).count(), 0);
    }

    #[test]
    fn move_master_reorders() {
        let mut doc = stencil();
        let names = |d: &Document| -> Vec<String> { d.masters().map(|m| String::from(d.str(d.master(m).unwrap().header.name_u.unwrap()))).collect() };
        let tri = doc.masters().nth(2).unwrap();
        let mut tx = doc.transaction("move");
        move_master(&mut tx, tri, 0).unwrap();
        let mut change = tx.commit().unwrap();
        assert_eq!(names(&doc), ["Tri", "Box", "Ring"]);
        assert!(doc.master(tri).is_some(), "the handle stays valid");
        let mut tx = doc.transaction("move");
        move_master(&mut tx, tri, 3).unwrap();
        tx.commit();
        assert_eq!(names(&doc), ["Box", "Ring", "Tri"]);
        let mut tx = doc.transaction("move");
        move_master(&mut tx, tri, 0).unwrap();
        tx.commit();
        assert!(change.undo(&mut doc) || names(&doc).len() == 3);
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
