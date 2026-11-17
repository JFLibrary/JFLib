// dump -- a document as text with every string resolved: two documents hold the same model when
// their dumps are equal (string tables, slots and next IDs are not content and stay out)

use alloc::string::String;
use core::fmt::Write as _;

use diagram_core::{
    Cell, Connect, Document, ExtraItem, Extras, ForeignPayload, Formula, Owner, Row, RowKey, Section, ShapeKind, Sheet,
    StyleRefs, Sym, TextItem, Trigger, Unit, Value,
};

struct D<'d> {
    doc: &'d Document,
    s: String,
}

// FNV-1a of bytes kept as bytes (themes, parts): equal bytes, equal hash
fn hash(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3))
}

impl<'d> D<'d> {
    fn t(&self, s: Sym) -> &'d str {
        self.doc.str(s)
    }

    fn o(&self, s: Option<Sym>) -> String {
        s.map_or_else(|| String::from("-"), |s| alloc::format!("{:?}", self.t(s)))
    }

    fn pairs(&mut self, what: &str, list: &[(Sym, Sym)]) {
        if list.is_empty() {
            return;
        }
        let _ = write!(self.s, " {what}[");
        for &(n, v) in list {
            let _ = write!(self.s, "{:?}={:?} ", self.t(n), self.t(v));
        }
        self.s.push(']');
    }

    fn extras(&mut self, what: &str, e: &Extras) {
        if e.is_empty() {
            return;
        }
        let _ = write!(self.s, " {what}{{");
        for x in e.items() {
            let item = match x.item {
                ExtraItem::Attribute { name, value } => alloc::format!("@{:?}={:?}", self.t(name), self.t(value)),
                ExtraItem::Element(r) => alloc::format!("<{:?}", self.t(r)),
                ExtraItem::Text(r) => alloc::format!("\"{:?}", self.t(r)),
            };
            let _ = write!(self.s, "{:?} {item}; ", x.place);
        }
        self.s.push('}');
    }

    fn styles(&mut self, st: StyleRefs) {
        let _ = write!(self.s, " styles {:?}/{:?}/{:?}", st.line, st.fill, st.text);
    }

    fn cell(&mut self, c: &Cell) {
        let v = match c.value {
            Value::Absent => String::from("-"),
            Value::Num(x) => alloc::format!("{x:?}"),
            Value::Text(t) => alloc::format!("{:?}", self.t(t)),
        };
        let f = match c.formula {
            Formula::Absent => String::from("-"),
            Formula::Inherited => String::from("Inh"),
            Formula::NoFormula => String::from("NoF"),
            Formula::Expr(e) => alloc::format!("{:?}", self.t(e)),
        };
        let u = match c.unit {
            None => String::from("-"),
            Some(Unit::Other(s)) => alloc::format!("?{}", self.t(s)),
            Some(u) => String::from(u.code().unwrap_or("?")),
        };
        let _ = write!(self.s, "\n      cell {:?} v={v} f={f} u={u} e={} stale={}", self.t(c.name), self.o(c.error), c.stale);
    }

    fn triggers(&mut self, list: &[Trigger]) {
        for t in list {
            let _ = write!(self.s, "\n      trigger {:?} {:?}", self.t(t.name), self.t(t.inner));
        }
    }

    fn row(&mut self, r: &Row) {
        let key = match r.key {
            RowKey::Index(i) => alloc::format!("#{i}"),
            RowKey::Named(n) => alloc::format!("{:?}", self.t(n)),
        };
        let _ = write!(self.s, "\n     row {key} ixw={} local={} t={} del={}", r.ix_written, self.o(r.local_name), self.o(r.kind), r.del);
        self.extras("x", &r.extras);
        for c in &r.cells {
            self.cell(c);
        }
        self.triggers(&r.triggers);
    }

    fn section(&mut self, s: &Section) {
        let _ = write!(self.s, "\n    section {:?} ix={} ixw={} del={}", self.t(s.name), s.index, s.ix_written, s.del);
        self.extras("x", &s.extras);
        for c in &s.cells {
            self.cell(c);
        }
        self.triggers(&s.triggers);
        for r in &s.rows {
            self.row(r);
        }
    }

    fn sheet(&mut self, sh: &Sheet) {
        self.extras("sheetx", &sh.extras);
        for c in &sh.cells {
            self.cell(c);
        }
        self.triggers(&sh.triggers);
        for s in &sh.sections {
            self.section(s);
        }
    }

    fn connects(&mut self, list: &[Connect]) {
        for c in list {
            let _ = write!(self.s, "\n   connect {} {} {:?} -> {} {} {:?}", c.from_sheet, self.o(c.from_cell), c.from_part, c.to_sheet, self.o(c.to_cell), c.to_part);
            self.extras("x", &c.extras);
        }
    }

    fn tree(&mut self, owner: Owner) {
        let doc = self.doc;
        for r in doc.all_shapes(owner) {
            let Some(sh) = doc.shape(r) else { continue };
            let d = sh.data();
            let h = &d.header;
            let kind = match h.kind {
                ShapeKind::Other(t) => alloc::format!("other {:?}", self.t(t)),
                k => alloc::format!("{k:?}"),
            };
            let parent = sh.parent().and_then(|p| doc.shape(p)).map(|p| p.id());
            let kids: alloc::vec::Vec<u32> = sh.children().filter_map(|c| doc.shape(c)).map(|c| c.id()).collect();
            let _ = write!(
                self.s,
                "\n   shape {} in {parent:?} kids {kids:?} {kind} name {} nameu {} custom {:?}/{:?} master {:?} ms {:?} del {} orig {:?} uid {}",
                sh.id(),
                self.o(h.name),
                self.o(h.name_u),
                h.is_custom_name,
                h.is_custom_name_u,
                h.master,
                h.master_shape,
                h.del,
                h.original_id,
                self.o(h.unique_id)
            );
            self.styles(h.styles);
            self.extras("x", &d.extras);
            for (i, v) in d.data.iter().enumerate() {
                if let Some(v) = v {
                    let _ = write!(self.s, " data{} {:?}", i + 1, self.t(*v));
                }
            }
            if let Some(t) = &d.text {
                let _ = write!(self.s, "\n    text");
                self.extras("x", &t.extras);
                for item in &t.items {
                    let _ = match item {
                        TextItem::Chars(c) => write!(self.s, " {c:?}"),
                        TextItem::Other(r) => write!(self.s, " other {:?}", self.t(*r)),
                        m => write!(self.s, " {m:?}"),
                    };
                }
            }
            if let Some(f) = &d.foreign {
                let _ = write!(self.s, "\n    foreign");
                self.pairs("a", &f.attrs);
                let _ = match &f.payload {
                    ForeignPayload::None => write!(self.s, " none"),
                    ForeignPayload::Inline(b) => write!(self.s, " inline {b:?}"),
                    ForeignPayload::Part(p) => write!(self.s, " part {:?}", self.t(*p)),
                };
                self.extras("x", &f.extras);
            }
            self.sheet(&d.sheet);
        }
    }
}

/// the model of a document as text
pub fn dump(doc: &Document) -> String {
    let mut d = D { doc, s: String::new() };
    let _ = write!(d.s, "kind {:?} macros {}", doc.kind, doc.macros);
    d.pairs("attrs", &doc.attrs);
    d.extras("docx", &doc.extras);
    let st = &doc.settings;
    let _ = write!(d.s, "\nsettings top {:?} text {:?} line {:?} fill {:?} guide {:?}", st.top_page, st.default_text_style, st.default_line_style, st.default_fill_style, st.default_guide_style);
    d.pairs("a", &st.attrs);
    d.pairs("items", &st.items);
    d.extras("x", &st.extras);
    let _ = write!(d.s, "\ncolors");
    for c in &doc.colors {
        let _ = write!(d.s, " {}={:?}", c.ix, d.t(c.rgb));
    }
    d.extras("x", &doc.colors_extras);
    let _ = write!(d.s, "\nfaces");
    for f in &doc.faces {
        d.pairs("face", &f.attrs);
    }
    d.extras("x", &doc.faces_extras);
    d.extras("\nstylesx", &doc.styles_extras);
    for r in doc.styles() {
        let Some(st) = doc.style(r) else { continue };
        let h = &st.header;
        let _ = write!(d.s, "\nstyle {} name {} nameu {} custom {:?}/{:?}", h.id, d.o(h.name), d.o(h.name_u), h.is_custom_name, h.is_custom_name_u);
        d.styles(h.based_on);
        d.pairs("a", &h.attrs);
        d.extras("x", &st.extras);
        d.sheet(&st.sheet);
    }
    let ds = doc.doc_sheet();
    let _ = write!(d.s, "\ndocsheet");
    d.styles(ds.styles);
    d.pairs("a", &ds.attrs);
    d.sheet(&ds.sheet);
    d.extras("\npagesx", &doc.pages_extras);
    for r in doc.pages() {
        let Some(p) = doc.page(r) else { continue };
        let h = &p.header;
        let _ = write!(
            d.s,
            "\npage {} name {} nameu {} custom {:?}/{:?} bg {} back {:?} theme {:?} part {}",
            h.id,
            d.o(h.name),
            d.o(h.name_u),
            h.is_custom_name,
            h.is_custom_name_u,
            h.background,
            h.back_page,
            p.theme,
            d.o(p.part)
        );
        d.styles(h.sheet_styles);
        d.pairs("a", &h.attrs);
        d.extras("x", &p.extras);
        d.extras("cx", &p.contents_extras);
        d.sheet(&p.sheet);
        d.connects(&p.connects);
        d.tree(Owner::Page(r));
    }
    d.extras("\nmastersx", &doc.masters_extras);
    for r in doc.masters() {
        let Some(m) = doc.master(r) else { continue };
        let h = &m.header;
        let _ = write!(
            d.s,
            "\nmaster {} name {} nameu {} custom {:?}/{:?} base {} uid {} icon {} part {}",
            h.id,
            d.o(h.name),
            d.o(h.name_u),
            h.is_custom_name,
            h.is_custom_name_u,
            d.o(h.base_id),
            d.o(h.unique_id),
            d.o(m.icon),
            d.o(m.part)
        );
        d.styles(h.sheet_styles);
        d.pairs("a", &h.attrs);
        d.extras("x", &m.extras);
        d.extras("cx", &m.contents_extras);
        d.sheet(&m.sheet);
        d.connects(&m.connects);
        d.tree(Owner::Master(r));
    }
    for t in &doc.themes {
        let _ = write!(d.s, "\ntheme {:?} {} bytes {:016x}", d.t(t.part), t.data.len(), hash(&t.data));
    }
    for p in &doc.parts {
        let _ = write!(d.s, "\npart {:?} {:?} {:?} {} bytes {:016x}", d.t(p.name), d.t(p.content_type), d.t(p.rel_type), p.data.len(), hash(&p.data));
    }
    for r in &doc.rels {
        let _ = write!(d.s, "\nrel {:?} {:?} {:?} {:?} ext {} -> {}", d.t(r.source), d.t(r.id), d.t(r.kind), d.t(r.target), r.external, d.o(r.part));
    }
    d.s.push('\n');
    d.s
}

/// the first line where two dumps differ, both versions (for a failing check)
pub fn first_difference(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            let cut = |s: &str| s.chars().take(300).collect::<String>();
            return alloc::format!("line {}: {} | {}", i + 1, cut(x), cut(y));
        }
    }
    alloc::format!("{} vs {} lines", a.lines().count(), b.lines().count())
}
