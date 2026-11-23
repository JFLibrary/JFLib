// sheet -- the document's lists, sheets, shapes and connects of the XML format (vdx/mod.rs says
// what goes where)

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{
    Cell, ColorEntry, Connect, Extra, ExtraItem, Extras, FaceName, ForeignData, ForeignPayload, Formula, Master, MasterHeader,
    Page, PageHeader, Part, Place, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, ShapeTree, Sheet, Style, StyleHeader,
    StyleRefs, Text, TextItem, Unit, Value,
};

use super::{media, Vx, R};
use crate::read::Count;
use crate::vsdx::{Error, Warning};
use crate::xml::{Event, Ns, Start};

const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

/// the single-row sections: their cells go straight into the sheet
const SINGLE: [&str; 22] = [
    "XForm", "XForm1D", "Line", "Fill", "Event", "LayerMem", "StyleProp", "Foreign", "PageProps", "TextBlock", "TextXForm", "Align",
    "Protection", "Help", "Misc", "RulerGrid", "DocProps", "Image", "Group", "Layout", "PageLayout", "PrintProps",
];

/// the sections of rows: (element, VSDX section, named rows, the row type VSDX writes)
const ROWS: [(&str, &str, bool, Option<&str>); 15] = [
    ("Char", "Character", false, None),
    ("Para", "Paragraph", false, None),
    ("Scratch", "Scratch", false, None),
    ("Connection", "Connection", false, Some("Connection")),
    ("ConnectionABCD", "Connection", false, Some("ConnectionABCD")),
    ("Field", "Field", false, None),
    ("Layer", "Layer", false, None),
    ("Reviewer", "Reviewer", false, None),
    ("Annotation", "Annotation", false, None),
    ("Control", "Control", true, None),
    ("Act", "Actions", true, None),
    ("User", "User", true, None),
    ("Prop", "Property", true, None),
    ("Hyperlink", "Hyperlink", true, None),
    ("SmartTagDef", "ActionTag", true, None),
];

/// the cells of a Geometry section itself (everything else in a Geom element is a row)
const GEOM_CELLS: [&str; 5] = ["NoFill", "NoLine", "NoShow", "NoSnap", "NoQuickDrag"];

/// cells the format still allows that VSDX has no more (gone since Visio 2003)
const GONE: [(&str, &str); 2] = [("Character", "Locale"), ("Character", "Perpendicular")];

/// the cells that hold a FaceName ID here and a font name in VSDX
const FONT_CELLS: [(&str, &str); 4] = [("Character", "Font"), ("Character", "AsianFont"), ("Character", "ComplexScriptFont"), ("Paragraph", "BulletFont")];

// the text of an element without children as copy wrote it (<a>x &amp; y</a>, <a/>), unescaped
fn plain_text(x: &str) -> String {
    let inner = match (x.find('>'), x.rfind("</")) {
        (Some(a), Some(b)) if a < b && !x[..a].ends_with('/') => &x[a + 1..b],
        _ => "",
    };
    inner.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#13;", "\r").replace("&amp;", "&")
}

// a shape read but not yet in its tree: its parent's index in the list
struct Pending {
    parent: Option<usize>,
    id: u32,
    data: ShapeData,
}

// a shape being read
struct ShapeFrame {
    idx: usize,
    id: u32,
    data: ShapeData,
    geometry: u32,
    listed: bool,
}

enum Frame {
    List,
    Shape(Box<ShapeFrame>),
}

impl<'a> Vx<'a, '_> {
    // ---- cells and sections

    // a cell element: its name, F, Unit, Err and its text (the value)
    fn cell(&mut self, s: &Start<'a>) -> R<Cell> {
        self.count(Count::Cell)?;
        let (mut f, mut u, mut e) = (None, None, None);
        for a in &s.attrs {
            match (a.ns, a.local) {
                (Ns::None, "F") => f = Some(a.value.clone()),
                (Ns::None, "Unit") => u = Some(a.value.clone()),
                (Ns::None, "Err") => e = Some(a.value.clone()),
                _ => {}
            }
        }
        let max = self.lim.formula_bytes;
        if f.as_ref().is_some_and(|x| x.len() > max) {
            return Err(Error::Limit("formula"));
        }
        let v = self.text_of(s, max, "formula")?;
        let unit = match u.as_deref() {
            Some(code) => Some(Unit::parse(code).unwrap_or_else(|| Unit::Other(self.doc.intern(code)))),
            None => None,
        };
        let name = self.sym(s.local);
        let strings = self.doc.strings_mut();
        let value = Value::parse(&v, unit, strings);
        let formula = f.as_deref().map_or(Formula::Absent, |f| Formula::parse(f, strings));
        let error = e.as_deref().map(|e| strings.intern(e));
        Ok(Cell { name, value, formula, unit, error, stale: false })
    }

    // the cells of a row or group element into `cells` (a later cell of a name replaces an
    // earlier one); `section`: the VSDX section, for the cells it no longer has and the fonts
    fn cells_into(&mut self, s: &Start<'a>, cells: &mut Vec<Cell>, section: &str) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            match self.next()? {
                Event::Start(c) if c.ns == Ns::Vdx => {
                    if GONE.contains(&(section, c.local)) {
                        self.skip(&c)?;
                        continue;
                    }
                    let mut cell = self.cell(&c)?;
                    if FONT_CELLS.contains(&(section, c.local)) {
                        if let Value::Num(id) = cell.value {
                            let name = (id >= 0.0 && id <= u32::MAX as f64).then(|| self.faces.get(&(id as u32)).cloned()).flatten();
                            if let Some(n) = name {
                                cell.value = Value::Text(self.sym(&n));
                                cell.unit = None;
                            }
                        }
                    }
                    match cells.iter_mut().find(|x| x.name == cell.name) {
                        Some(old) => *old = cell,
                        None => cells.push(cell),
                    }
                }
                Event::Start(c) => self.left_out(&c)?,
                Event::Text(_) => {}
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("an element without end")),
            }
        }
    }

    // a child of a sheet element that is a section or a single-row group -> true; false: none
    // (not read)
    fn sheet_child(&mut self, c: &Start<'a>, sheet: &mut Sheet, geometry: &mut u32) -> R<bool> {
        if c.ns != Ns::Vdx {
            return Ok(false);
        }
        if c.local == "Geom" {
            self.geom(c, sheet, geometry)?;
        } else if c.local == "Tabs" {
            self.tabs(c, sheet)?;
        } else if let Some(&(_, sec, named, kind)) = ROWS.iter().find(|r| r.0 == c.local) {
            self.row(c, sheet, sec, named, kind)?;
        } else if SINGLE.contains(&c.local) {
            let mut cells = core::mem::take(&mut sheet.cells);
            let r = self.cells_into(c, &mut cells, "");
            sheet.cells = cells;
            r?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    // the section `name` of a sheet (made when new); its position
    fn section_of(&mut self, sheet: &mut Sheet, name: &str) -> R<usize> {
        let sym = self.sym(name);
        let key = SecKey { name: sym, index: 0 };
        if let Some(i) = sheet.section_pos(key) {
            return Ok(i);
        }
        self.count(Count::Section)?;
        sheet.sections.push(Section::new(key));
        Ok(sheet.sections.len() - 1)
    }

    // a row element of a section of rows
    fn row(&mut self, s: &Start<'a>, sheet: &mut Sheet, sec: &'static str, named: bool, kind: Option<&str>) -> R<()> {
        self.count(Count::Row)?;
        let (mut ix, mut del, mut name, mut name_u, mut id) = (None, false, None, None, None);
        for a in &s.attrs {
            match (a.ns, a.local) {
                (Ns::None, "IX") => ix = Some(self.uint(&a.value, "a row IX that is no number")?),
                (Ns::None, "Del") => del = self.flag(&a.value, "a row Del that is no flag")?,
                (Ns::None, "Name") => name = Some(String::from(a.value.as_ref())),
                (Ns::None, "NameU") => name_u = Some(String::from(a.value.as_ref())),
                (Ns::None, "ID") => id = Some(self.uint(&a.value, "a row ID that is no number")?),
                _ => {}
            }
        }
        let si = self.section_of(sheet, sec)?;
        let n = sheet.sections[si].rows.len();
        // a named row: NameU, else Name, else Row_<ID> (Visio's name for it)
        let key_name = name_u.clone().or_else(|| name.clone()).or_else(|| id.map(|i| alloc::format!("Row_{i}")));
        let mut row = if named || key_name.is_some() && (name_u.is_some() || name.is_some()) {
            let k = key_name.unwrap_or_else(|| alloc::format!("Row_{}", n + 1));
            let mut r = Row::new(RowKey::Named(self.sym(&k)));
            if let (Some(l), Some(u)) = (&name, &name_u) {
                if l != u {
                    r.local_name = Some(self.sym(l));
                }
            }
            if let Some(i) = ix {
                // both: the name is the key, IX stays (as vsdx.rs keeps it)
                self.count(Count::Item)?;
                let item = ExtraItem::Attribute { name: self.sym("IX"), value: self.sym(&alloc::format!("{i}")) };
                r.extras.push(Extra { place: Place::Element, item });
            }
            r
        } else {
            Row::new(RowKey::Index(ix.unwrap_or(n as u32)))
        };
        row.del = del;
        row.kind = kind.map(|k| self.sym(k));
        let mut cells = Vec::new();
        self.cells_into(s, &mut cells, sec)?;
        row.cells = cells;
        let section = &mut sheet.sections[si];
        if section.row(row.key).is_some() {
            self.warn(Warning::LeftOut { what: String::from(s.qname) });
            return Ok(());
        }
        section.rows.push(row);
        Ok(())
    }

    // a Tabs element: the row; its tab stops are left out
    fn tabs(&mut self, s: &Start<'a>, sheet: &mut Sheet) -> R<()> {
        self.count(Count::Row)?;
        let mut ix = None;
        let mut del = false;
        for a in &s.attrs {
            match (a.ns, a.local) {
                (Ns::None, "IX") => ix = Some(self.uint(&a.value, "a row IX that is no number")?),
                (Ns::None, "Del") => del = self.flag(&a.value, "a row Del that is no flag")?,
                _ => {}
            }
        }
        let si = self.section_of(sheet, "Tabs")?;
        let n = sheet.sections[si].rows.len() as u32;
        let mut row = Row::new(RowKey::Index(ix.unwrap_or(n)));
        row.del = del;
        let mut stops = false;
        if !s.empty {
            loop {
                match self.next()? {
                    Event::Start(c) => {
                        self.skip(&c)?;
                        stops = true;
                    }
                    Event::Text(_) => {}
                    Event::End { .. } => break,
                    Event::Eof => return Err(self.bad("Tabs without end")),
                }
            }
        } else {
            self.next()?;
        }
        if stops {
            self.warn(Warning::LeftOut { what: String::from("Tab") });
        }
        let section = &mut sheet.sections[si];
        if section.row(row.key).is_none() {
            section.rows.push(row);
        }
        Ok(())
    }

    // a Geom element: a Geometry section
    fn geom(&mut self, s: &Start<'a>, sheet: &mut Sheet, geometry: &mut u32) -> R<()> {
        self.count(Count::Section)?;
        let (mut ix, mut del) = (None, false);
        for a in &s.attrs {
            match (a.ns, a.local) {
                (Ns::None, "IX") => ix = Some(self.uint(&a.value, "a Geom IX that is no number")?),
                (Ns::None, "Del") => del = self.flag(&a.value, "a Geom Del that is no flag")?,
                _ => {}
            }
        }
        let index = ix.unwrap_or(*geometry);
        *geometry = geometry.saturating_add(1).max(index.saturating_add(1));
        let mut sec = Section::new(SecKey { name: self.sym("Geometry"), index });
        sec.ix_written = true;
        sec.del = del;
        if !s.empty {
            loop {
                let c = match self.next()? {
                    Event::Start(c) => c,
                    Event::Text(_) => continue,
                    Event::End { .. } => break,
                    Event::Eof => return Err(self.bad("Geom without end")),
                };
                if c.ns != Ns::Vdx {
                    self.left_out(&c)?;
                } else if GEOM_CELLS.contains(&c.local) {
                    let cell = self.cell(&c)?;
                    match sec.cells.iter_mut().find(|x| x.name == cell.name) {
                        Some(old) => *old = cell,
                        None => sec.cells.push(cell),
                    }
                } else {
                    self.count(Count::Row)?;
                    let (mut rix, mut rdel) = (None, false);
                    for a in &c.attrs {
                        match (a.ns, a.local) {
                            (Ns::None, "IX") => rix = Some(self.uint(&a.value, "a row IX that is no number")?),
                            (Ns::None, "Del") => rdel = self.flag(&a.value, "a row Del that is no flag")?,
                            _ => {}
                        }
                    }
                    // geometry rows count from 1
                    let mut row = Row::new(RowKey::Index(rix.unwrap_or(sec.rows.len() as u32 + 1)));
                    row.del = rdel;
                    row.kind = Some(self.sym(c.local));
                    let mut cells = Vec::new();
                    self.cells_into(&c, &mut cells, "Geometry")?;
                    row.cells = cells;
                    if sec.row(row.key).is_some() {
                        self.warn(Warning::LeftOut { what: String::from(c.qname) });
                    } else {
                        sec.rows.push(row);
                    }
                }
            }
        } else {
            self.next()?;
        }
        if sheet.section(sec.key()).is_some() {
            self.warn(Warning::LeftOut { what: String::from("Geom") });
            return Ok(());
        }
        sheet.sections.push(sec);
        Ok(())
    }

    // a sheet element of its own (PageSheet, DocumentSheet, StyleSheet's body): its sections and
    // cells into `sheet`; everything else is left out
    fn sheet_body(&mut self, s: &Start<'a>, sheet: &mut Sheet) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        let mut geometry = 0u32;
        loop {
            match self.next()? {
                Event::Start(c) => {
                    if !self.sheet_child(&c, sheet, &mut geometry)? {
                        self.left_out(&c)?;
                    }
                }
                Event::Text(_) => {}
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("a sheet without end")),
            }
        }
    }

    // LineStyle, FillStyle, TextStyle of a sheet element
    fn style_refs(&self, s: &Start<'a>) -> R<StyleRefs> {
        let mut st = StyleRefs::default();
        for a in &s.attrs {
            match (a.ns, a.local) {
                (Ns::None, "LineStyle") => st.line = Some(self.uint(&a.value, "a LineStyle that is no number")?),
                (Ns::None, "FillStyle") => st.fill = Some(self.uint(&a.value, "a FillStyle that is no number")?),
                (Ns::None, "TextStyle") => st.text = Some(self.uint(&a.value, "a TextStyle that is no number")?),
                _ => {}
            }
        }
        Ok(st)
    }

    // ---- the document's lists

    pub(super) fn settings(&mut self, s: &Start<'a>) -> R<()> {
        let mut st = core::mem::take(&mut self.doc.settings);
        for a in &s.attrs {
            if a.ns != Ns::None {
                continue;
            }
            let v = &a.value;
            match a.local {
                "TopPage" => st.top_page = Some(self.uint(v, "a TopPage that is no number")?),
                "DefaultTextStyle" => st.default_text_style = Some(self.uint(v, "a DefaultTextStyle that is no number")?),
                "DefaultLineStyle" => st.default_line_style = Some(self.uint(v, "a DefaultLineStyle that is no number")?),
                "DefaultFillStyle" => st.default_fill_style = Some(self.uint(v, "a DefaultFillStyle that is no number")?),
                "DefaultGuideStyle" => st.default_guide_style = Some(self.uint(v, "a DefaultGuideStyle that is no number")?),
                _ => {}
            }
        }
        if s.empty {
            self.next()?;
        } else {
            let max = self.lim.formula_bytes;
            loop {
                let c = match self.next()? {
                    Event::Start(c) => c,
                    Event::Text(_) => continue,
                    Event::End { .. } => break,
                    Event::Eof => return Err(self.bad("DocumentSettings without end")),
                };
                if c.ns != Ns::Vdx {
                    self.left_out(&c)?;
                    continue;
                }
                // a text element is an item; one with children (SnapAngles with its SnapAngle
                // list) stays as XML at its place, as vsdx.rs keeps it
                let (x, nested) = self.copy_full(&c)?;
                let Some(x) = x else {
                    self.warn(Warning::LeftOut { what: String::from(c.qname) });
                    continue;
                };
                self.count(Count::Item)?;
                if nested {
                    let item = ExtraItem::Element(self.sym(&x));
                    st.extras.push(Extra { place: Place::AfterChild(st.items.len() as u32), item });
                } else {
                    let t = plain_text(&x);
                    if t.len() > max {
                        return Err(Error::Limit("formula"));
                    }
                    let item = (self.sym(c.local), self.sym(&t));
                    st.items.push(item);
                }
            }
        }
        self.doc.settings = st;
        Ok(())
    }

    pub(super) fn colors(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("Colors without end")),
            };
            let (mut ix, mut rgb) = (None, None);
            for a in &c.attrs {
                match (a.ns, a.local) {
                    (Ns::None, "IX") => ix = self.uint(&a.value, "").ok(),
                    (Ns::None, "RGB") => rgb = Some(String::from(a.value.as_ref())),
                    _ => {}
                }
            }
            self.skip(&c)?;
            match (Self::is(&c, "ColorEntry"), ix, rgb) {
                (true, Some(ix), Some(rgb)) => {
                    self.count(Count::Item)?;
                    let rgb = self.sym(&rgb);
                    self.doc.colors.push(ColorEntry { ix, rgb });
                }
                _ => self.warn(Warning::LeftOut { what: String::from(c.qname) }),
            }
        }
    }

    pub(super) fn face_names(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("FaceNames without end")),
            };
            self.skip(&c)?;
            if !Self::is(&c, "FaceName") {
                self.warn(Warning::LeftOut { what: String::from(c.qname) });
                continue;
            }
            let mut f = FaceName::default();
            let mut id = None;
            let mut name = None;
            for a in &c.attrs {
                if a.ns != Ns::None {
                    continue;
                }
                // VSDX's names: NameU for Name, Panose for Panos; the ID goes (fonts are named)
                let n = match a.local {
                    "ID" => {
                        id = self.uint(&a.value, "").ok();
                        continue;
                    }
                    "Name" => {
                        name = Some(String::from(a.value.as_ref()));
                        "NameU"
                    }
                    "Panos" => "Panose",
                    "UnicodeRanges" | "CharSets" | "Flags" => a.local,
                    _ => continue,
                };
                let pair = (self.sym(n), self.sym(&a.value));
                f.attrs.push(pair);
            }
            self.count(Count::Item)?;
            if let (Some(i), Some(n)) = (id, name) {
                self.faces.insert(i, n);
            }
            self.doc.faces.push(f);
        }
    }

    pub(super) fn styles(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("StyleSheets without end")),
            };
            if !Self::is(&c, "StyleSheet") {
                self.left_out(&c)?;
                continue;
            }
            self.count(Count::Object)?;
            let mut h = StyleHeader::default();
            let mut id = None;
            for a in &c.attrs {
                if a.ns != Ns::None {
                    continue;
                }
                let v = &a.value;
                match a.local {
                    "ID" => id = Some(self.uint(v, "a StyleSheet ID that is no number")?),
                    "Name" => h.name = Some(self.sym(v)),
                    "NameU" => h.name_u = Some(self.sym(v)),
                    "IsCustomName" => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                    "IsCustomNameU" => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                    _ => {}
                }
            }
            h.based_on = self.style_refs(&c)?;
            h.id = id.ok_or_else(|| self.bad("a StyleSheet without ID"))?;
            let mut st = Style { header: h, ..Style::default() };
            self.sheet_body(&c, &mut st.sheet)?;
            self.doc.insert_style(st).map_err(|_| self.bad("two styles with one ID"))?;
        }
    }

    pub(super) fn doc_sheet(&mut self, s: &Start<'a>) -> R<()> {
        let styles = self.style_refs(s)?;
        let mut attrs = Vec::new();
        for a in &s.attrs {
            if a.ns == Ns::None && matches!(a.local, "Name" | "NameU" | "IsCustomName" | "IsCustomNameU" | "UniqueID") {
                attrs.push((self.sym(a.local), self.sym(&a.value)));
            }
        }
        let mut sheet = Sheet::new();
        self.sheet_body(s, &mut sheet)?;
        let ds = self.doc.doc_sheet_mut();
        ds.sheet = sheet;
        ds.styles = styles;
        ds.attrs = attrs;
        Ok(())
    }

    // ---- masters and pages

    pub(super) fn masters(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("Masters without end")),
            };
            if !Self::is(&c, "Master") {
                self.left_out(&c)?;
                continue;
            }
            let m = self.master(&c)?;
            self.doc.insert_master(m).map_err(|_| self.bad("two masters with one ID"))?;
        }
    }

    fn master(&mut self, s: &Start<'a>) -> R<Master> {
        self.count(Count::Object)?;
        let mut h = MasterHeader::default();
        let mut id = None;
        for a in &s.attrs {
            if a.ns != Ns::None {
                continue;
            }
            let v = &a.value;
            match a.local {
                "ID" => id = Some(self.uint(v, "a Master ID that is no number")?),
                "Name" => h.name = Some(self.sym(v)),
                "NameU" => h.name_u = Some(self.sym(v)),
                "IsCustomName" => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "BaseID" => h.base_id = Some(self.sym(v)),
                "UniqueID" => h.unique_id = Some(self.sym(v)),
                // the attributes VSDX's Master has too
                "Prompt" | "IconSize" | "AlignName" | "MatchByName" | "IconUpdate" | "PatternFlags" | "Hidden" | "MasterType" => {
                    let p = (self.sym(a.local), self.sym(v));
                    h.attrs.push(p);
                }
                _ => {}
            }
        }
        h.id = id.ok_or_else(|| self.bad("a Master without ID"))?;
        let mut m = Master::new(h);
        if s.empty {
            self.next()?;
            return Ok(m);
        }
        let (mut sheet, mut shapes, mut connects, mut icon) = (false, false, false, false);
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(m),
                Event::Eof => return Err(self.bad("a Master without end")),
            };
            if Self::is(&c, "PageSheet") && !sheet {
                sheet = true;
                m.header.sheet_styles = self.style_refs(&c)?;
                let mut sh = Sheet::new();
                self.sheet_body(&c, &mut sh)?;
                m.sheet = sh;
            } else if Self::is(&c, "Icon") && !icon {
                icon = true;
                let t = self.text_of(&c, self.lim.text_bytes, "text")?;
                let t: String = t.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                if !t.is_empty() {
                    m.icon = Some(self.sym(&t));
                }
            } else if Self::is(&c, "Shapes") && !shapes {
                shapes = true;
                self.shapes(&c, &mut m.shapes)?;
            } else if Self::is(&c, "Connects") && !connects {
                connects = true;
                self.connects(&c, &mut m.connects)?;
            } else {
                self.left_out(&c)?;
            }
        }
    }

    pub(super) fn pages(&mut self, s: &Start<'a>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("Pages without end")),
            };
            if !Self::is(&c, "Page") {
                self.left_out(&c)?;
                continue;
            }
            let p = self.page(&c)?;
            self.doc.insert_page(p).map_err(|_| self.bad("two pages with one ID"))?;
        }
    }

    fn page(&mut self, s: &Start<'a>) -> R<Page> {
        self.count(Count::Object)?;
        let mut h = PageHeader::default();
        let mut id = None;
        for a in &s.attrs {
            if a.ns != Ns::None {
                continue;
            }
            let v = &a.value;
            match a.local {
                "ID" => id = Some(self.uint(v, "a Page ID that is no number")?),
                "Name" => h.name = Some(self.sym(v)),
                "NameU" => h.name_u = Some(self.sym(v)),
                "IsCustomName" => h.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" => h.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "Background" => h.background = self.flag(v, "a Background that is no flag")?,
                "BackPage" => h.back_page = Some(self.uint(v, "a BackPage that is no number")?),
                // the attributes VSDX's Page has too
                "ViewScale" | "ViewCenterX" | "ViewCenterY" | "ReviewerID" | "AssociatedPage" => {
                    let p = (self.sym(a.local), self.sym(v));
                    h.attrs.push(p);
                }
                _ => {}
            }
        }
        h.id = id.ok_or_else(|| self.bad("a Page without ID"))?;
        let mut page = Page::new(h);
        if s.empty {
            self.next()?;
            return Ok(page);
        }
        let (mut sheet, mut shapes, mut connects) = (false, false, false);
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(page),
                Event::Eof => return Err(self.bad("a Page without end")),
            };
            if Self::is(&c, "PageSheet") && !sheet {
                sheet = true;
                page.header.sheet_styles = self.style_refs(&c)?;
                let mut sh = Sheet::new();
                self.sheet_body(&c, &mut sh)?;
                page.sheet = sh;
            } else if Self::is(&c, "Shapes") && !shapes {
                shapes = true;
                self.shapes(&c, &mut page.shapes)?;
            } else if Self::is(&c, "Connects") && !connects {
                connects = true;
                self.connects(&c, &mut page.connects)?;
            } else {
                self.left_out(&c)?;
            }
        }
    }

    fn connects(&mut self, s: &Start<'a>, out: &mut Vec<Connect>) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        loop {
            let c = match self.next()? {
                Event::Start(c) => c,
                Event::Text(_) => continue,
                Event::End { .. } => return Ok(()),
                Event::Eof => return Err(self.bad("a Connects list without end")),
            };
            if !Self::is(&c, "Connect") {
                self.left_out(&c)?;
                continue;
            }
            self.count(Count::Item)?;
            let mut x = Connect { from_sheet: 0, from_cell: None, from_part: None, to_sheet: 0, to_cell: None, to_part: None, extras: Extras::new() };
            let (mut from, mut to) = (None, None);
            for a in &c.attrs {
                if a.ns != Ns::None {
                    continue;
                }
                match a.local {
                    "FromSheet" => from = Some(self.uint(&a.value, "a FromSheet that is no number")?),
                    "ToSheet" => to = Some(self.uint(&a.value, "a ToSheet that is no number")?),
                    "FromCell" => x.from_cell = Some(self.sym(&a.value)),
                    "ToCell" => x.to_cell = Some(self.sym(&a.value)),
                    "FromPart" => x.from_part = Some(self.int(&a.value, "a FromPart that is no number")?),
                    "ToPart" => x.to_part = Some(self.int(&a.value, "a ToPart that is no number")?),
                    _ => {}
                }
            }
            self.skip(&c)?;
            match (from, to) {
                (Some(f), Some(t)) => {
                    x.from_sheet = f;
                    x.to_sheet = t;
                }
                _ => return Err(self.bad("a Connect without FromSheet or ToSheet")),
            }
            out.push(x);
        }
    }

    // ---- shapes

    // a Shape's start tag into a ShapeData and its ID
    fn shape_start(&mut self, s: &Start<'a>) -> R<(u32, ShapeData)> {
        self.count(Count::Shape)?;
        let mut data = ShapeData::new(ShapeKind::Unspecified);
        let mut id = None;
        for a in &s.attrs {
            if a.ns != Ns::None {
                continue;
            }
            let v = &a.value;
            match a.local {
                "ID" => id = Some(self.uint(v, "a Shape ID that is no number")?),
                "Type" => data.header.kind = ShapeKind::parse(v, self.doc.strings_mut()),
                "Name" => data.header.name = Some(self.sym(v)),
                "NameU" => data.header.name_u = Some(self.sym(v)),
                "IsCustomName" => data.header.is_custom_name = Some(self.flag(v, "an IsCustomName that is no flag")?),
                "IsCustomNameU" => data.header.is_custom_name_u = Some(self.flag(v, "an IsCustomNameU that is no flag")?),
                "Master" => data.header.master = Some(self.uint(v, "a Master that is no number")?),
                "MasterShape" => data.header.master_shape = Some(self.uint(v, "a MasterShape that is no number")?),
                "LineStyle" => data.header.styles.line = Some(self.uint(v, "a LineStyle that is no number")?),
                "FillStyle" => data.header.styles.fill = Some(self.uint(v, "a FillStyle that is no number")?),
                "TextStyle" => data.header.styles.text = Some(self.uint(v, "a TextStyle that is no number")?),
                "Del" => data.header.del = self.flag(v, "a Shape Del that is no flag")?,
                "OriginalID" => data.header.original_id = Some(self.uint(v, "an OriginalID that is no number")?),
                "UniqueID" => data.header.unique_id = Some(self.sym(v)),
                _ => {}
            }
        }
        let id = id.ok_or_else(|| self.bad("a Shape without ID"))?;
        Ok((id, data))
    }

    // a child of a Shape other than its Shapes list
    fn shape_child(&mut self, c: &Start<'a>, f: &mut ShapeFrame) -> R<()> {
        if self.sheet_child(c, &mut f.data.sheet, &mut f.geometry)? {
            return Ok(());
        }
        let data = &mut f.data;
        if Self::is(c, "Text") && data.text.is_none() {
            let t = self.text(c)?;
            f.data.text = Some(Box::new(t));
        } else if Self::is(c, "ForeignData") && data.foreign.is_none() {
            let fd = self.foreign(c)?;
            f.data.foreign = Some(Box::new(fd));
        } else if let Some(i) = ["Data1", "Data2", "Data3"].iter().position(|n| Self::is(c, n)).filter(|&i| data.data[i].is_none()) {
            let t = self.text_of(c, self.lim.text_bytes, "text")?;
            f.data.data[i] = Some(self.sym(&t));
        } else {
            self.left_out(c)?;
        }
        Ok(())
    }

    // a Shapes list into `tree` (groups at any depth without recursion: a stack of the open
    // shapes and lists)
    fn shapes(&mut self, s: &Start<'a>, tree: &mut ShapeTree) -> R<()> {
        if s.empty {
            self.next()?;
            return Ok(());
        }
        let mut list: Vec<Pending> = Vec::new();
        let mut stack: Vec<Frame> = alloc::vec![Frame::List];
        loop {
            let ev = self.next()?;
            let depth = stack.len();
            match stack.last_mut() {
                Some(Frame::List) => match ev {
                    Event::Start(c) if Self::is(&c, "Shape") => {
                        let parent = match depth.checked_sub(2).and_then(|i| stack.get(i)) {
                            Some(Frame::Shape(g)) => Some(g.idx),
                            _ => None,
                        };
                        let (id, data) = self.shape_start(&c)?;
                        let idx = list.len();
                        list.push(Pending { parent, id, data: ShapeData::default() });
                        stack.push(Frame::Shape(Box::new(ShapeFrame { idx, id, data, geometry: 0, listed: false })));
                    }
                    Event::Start(c) => self.left_out(&c)?,
                    Event::Text(_) => {}
                    Event::End { .. } => {
                        stack.pop();
                        if stack.is_empty() {
                            break;
                        }
                    }
                    Event::Eof => return Err(self.bad("a Shapes list without end")),
                },
                Some(Frame::Shape(f)) => match ev {
                    Event::Start(c) if Self::is(&c, "Shapes") && !f.listed => {
                        f.listed = true;
                        stack.push(Frame::List);
                    }
                    Event::Start(c) => self.shape_child(&c, f)?,
                    Event::Text(_) => {}
                    Event::End { .. } => {
                        if let Some(Frame::Shape(f)) = stack.pop() {
                            let f = *f;
                            let _ = f.id;
                            list[f.idx].data = f.data;
                        }
                    }
                    Event::Eof => return Err(self.bad("a Shape without end")),
                },
                None => return Err(self.bad("a Shapes list without end")),
            }
        }
        let mut slots: Vec<u32> = Vec::with_capacity(list.len());
        for p in list {
            let parent = p.parent.and_then(|i| slots.get(i).copied());
            let slot = tree.insert(parent, p.id, p.data).map_err(|_| self.bad("two shapes with one ID"))?;
            slots.push(slot);
        }
        Ok(())
    }

    // a shape text: characters and the marks cp, pp, tp, fld (a fld with its cached text and
    // anything else as XML text without prefixes)
    fn text(&mut self, s: &Start<'a>) -> R<Text> {
        let mut text = Text::default();
        if s.empty {
            self.next()?;
            return Ok(text);
        }
        let max = self.lim.text_bytes;
        let mut size = 0usize;
        let mut buf = String::new();
        loop {
            match self.next()? {
                Event::Text(t) => {
                    size += t.len();
                    if size > max {
                        return Err(Error::Limit("text"));
                    }
                    buf.push_str(&t);
                }
                Event::Start(c) => {
                    if !buf.is_empty() {
                        self.count(Count::Item)?;
                        text.items.push(TextItem::Chars(Box::from(core::mem::take(&mut buf).as_str())));
                    }
                    let ix = match (c.ns, c.local) {
                        (Ns::Vdx, "cp" | "pp" | "tp" | "fld") => {
                            let own: Vec<_> = c.attrs.iter().filter(|a| a.ns != Ns::Decl).collect();
                            match own.as_slice() {
                                [a] if a.ns == Ns::None && a.local == "IX" => self.uint(&a.value, "").ok(),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    let (local, empty) = (c.local, c.empty);
                    let item = match (ix, local) {
                        (Some(ix), "cp" | "pp" | "tp") => {
                            // marks are empty; anything inside is dropped
                            self.skip(&c)?;
                            match local {
                                "cp" => TextItem::Cp(ix),
                                "pp" => TextItem::Pp(ix),
                                _ => TextItem::Tp(ix),
                            }
                        }
                        (Some(ix), "fld") if empty => {
                            self.next()?;
                            TextItem::Field(ix)
                        }
                        _ => match self.copy(&c)? {
                            Some(x) => {
                                size += x.len();
                                if size > max {
                                    return Err(Error::Limit("text"));
                                }
                                TextItem::Other(self.sym(&x))
                            }
                            None => {
                                self.warn(Warning::LeftOut { what: String::from(c.qname) });
                                continue;
                            }
                        },
                    };
                    self.count(Count::Item)?;
                    text.items.push(item);
                }
                Event::End { .. } => break,
                Event::Eof => return Err(self.bad("a Text without end")),
            }
        }
        if !buf.is_empty() {
            self.count(Count::Item)?;
            text.items.push(TextItem::Chars(Box::from(buf.as_str())));
        }
        Ok(text)
    }

    // ForeignData: its attributes, its base64 picture as a part of its own
    fn foreign(&mut self, s: &Start<'a>) -> R<ForeignData> {
        let mut fd = ForeignData::default();
        let mut ftype = String::new();
        for a in &s.attrs {
            if a.ns != Ns::None {
                continue;
            }
            if a.local == "ForeignType" {
                ftype = String::from(a.value.as_ref());
            }
            let p = (self.sym(a.local), self.sym(&a.value));
            fd.attrs.push(p);
        }
        let max = self.lim.zip.part_bytes;
        let t = self.text_of(s, max, "file")?;
        if t.bytes().all(|c| c.is_ascii_whitespace()) {
            return Ok(fd);
        }
        let pic = media::base64(t.as_bytes(), max).and_then(|b| media::picture(b, &ftype));
        let Some((ext, ct, data)) = pic else {
            self.warn(Warning::LeftOut { what: String::from("ForeignData") });
            return Ok(fd);
        };
        self.count(Count::Item)?;
        self.media += 1;
        let name = self.sym(&alloc::format!("/visio/media/image{}.{ext}", self.media));
        let part = Part { name, content_type: self.sym(ct), rel_type: self.sym(REL_IMAGE), data };
        self.doc.parts.push(part);
        fd.payload = ForeignPayload::Part(name);
        Ok(fd)
    }
}
