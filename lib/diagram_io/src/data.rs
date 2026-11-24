// data -- (JD18) the external data parts of a VSDX package and diagram_core's data::DataSets
// (DF-12)
//
//   load(doc, limits)        after reading (vsdx::read): connections.xml, recordsets.xml and the
//                            recordset parts they name become doc.data; the parts and the
//                            relationships from recordsets.xml leave doc.parts / doc.rels.  When
//                            one of them is not understood, all stay parts as before (kept as
//                            bytes, written back unchanged).
//   with_parts(doc)          for writing (vsdx::write): a copy of the document with doc.data as
//                            parts again (None when there is no data: nothing to add)
//   ado(rs) / read_ado(..)   a recordset's rows as an ADO persisted recordset ("ADO classic XML",
//                            what Visio's DataRecordset.DataAsXML gives and AddFromXML takes)
//
// The parts ([MS-VSDX] 2.3.3.2, 2.3.3.10; desktop Visio):
//   /visio/data/connections.xml    DataConnections (NextID) > DataConnection (ID, FileName,
//                                  ConnectionString, Command, FriendlyName, Timeout,
//                                  AlwaysUseConnectionFile); the main part names it
//                                  (…/visio/2010/relationships/connections)
//   /visio/data/recordsets.xml     DataRecordSets (NextID, ActiveRecordsetID, DataWindowOrder)
//                                  > DataRecordSet (attributes in data.rs) > Rel r:id,
//                                  DataColumns > DataColumn, PrimaryKey > RowKeyValue, RowMap,
//                                  RefreshConflict, AutoLinkComparison; named by the main part
//                                  (…/recordsets)
//   /visio/data/recordsetN.xml     one per DataRecordSet, named by its Rel r:id from
//                                  recordsets.xml (…/recordset): the cached rows as an ADO
//                                  persisted recordset: <xml> s:Schema > s:ElementType >
//                                  s:AttributeType (name, rs:name, s:datatype dt:type) and
//                                  rs:data > z:row with one attribute per column
// Row IDs: a Visio_RowID column when the data has one (DataAsXML adds it), else the
// RowKeyValue of the row's key, else the position (from 1).  The data is written without
// Visio_RowID: the key or the position gives the IDs back.
//
// Everything is untrusted: the parts are bounded by the ZIP limits (part size), the XML by the
// reader's depth and attribute limits; nothing recurses.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use diagram_core::data::{AutoLink, Column, Connection, DataRow, DataSets, DataValue, Key, Recordset, RowLink};
use diagram_core::{Document, Part, Relationship, Sym};

use crate::write::escape;
use crate::xml::{self, Event, Ns, Reader};

/// the content type of connections.xml
pub(crate) const CT_CONNECTIONS: &str = "application/vnd.ms-visio.connections+xml";
/// the content type of recordsets.xml
pub(crate) const CT_RECORDSETS: &str = "application/vnd.ms-visio.recordsets+xml";
/// the content type of a recordset's data part
pub(crate) const CT_RECORDSET: &str = "application/vnd.ms-visio.recordset+xml";
const REL_CONNECTIONS: &str = "http://schemas.microsoft.com/visio/2010/relationships/connections";
const REL_RECORDSETS: &str = "http://schemas.microsoft.com/visio/2010/relationships/recordsets";
const REL_RECORDSET: &str = "http://schemas.microsoft.com/visio/2010/relationships/recordset";
const NS_MAIN: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DECL: &str = "<?xml version='1.0' encoding='utf-8' ?>\r\n";
const ROW_ID: &str = "Visio_RowID";

// ---------------------------------------------------------------- a small element tree

// an element: its local name, whether it is in a namespace, its attributes and children
struct El {
    local: String,
    attrs: Vec<At>,
    kids: Vec<El>,
}

struct At {
    qname: String,
    local: String,
    ns: Ns,
    value: String,
}

impl El {
    // an attribute without namespace
    fn get(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.ns == Ns::None && a.local == name).map(|a| a.value.as_str())
    }

    fn uint(&self, name: &str) -> Result<Option<u32>, ()> {
        match self.get(name) {
            None => Ok(None),
            Some(v) => v.trim().parse().map(Some).map_err(|_| ()),
        }
    }

    fn flag(&self, name: &str) -> Result<Option<bool>, ()> {
        match self.get(name).map(str::trim) {
            None => Ok(None),
            Some("1" | "true") => Ok(Some(true)),
            Some("0" | "false") => Ok(Some(false)),
            Some(_) => Err(()),
        }
    }

    fn string(&self, name: &str) -> Option<String> {
        self.get(name).map(String::from)
    }

    // the attributes not in `known` (namespace declarations left out), as written
    fn others(&self, known: &[&str]) -> Vec<(String, String)> {
        self.attrs
            .iter()
            .filter(|a| a.ns != Ns::Decl && !(a.ns == Ns::None && known.contains(&a.local.as_str())))
            .map(|a| (a.qname.clone(), a.value.clone()))
            .collect()
    }
}

// a part's XML as a tree (element-only: text between elements is ignored); None when it is not
// well-formed or breaks the bounds
fn tree(bytes: &[u8], depth: usize, attrs: usize, nodes: usize) -> Option<El> {
    let text = xml::text_of(bytes)?;
    let mut r = Reader::new(&text, depth, attrs);
    let mut stack: Vec<El> = Vec::new();
    let mut count = 0usize;
    let mut root = None;
    loop {
        match r.next().ok()? {
            Event::Start(s) => {
                count += 1;
                if count > nodes {
                    return None;
                }
                let attrs = s.attrs.iter().map(|a| At { qname: String::from(a.qname), local: String::from(a.local), ns: a.ns, value: String::from(&*a.value) }).collect();
                stack.push(El { local: String::from(s.local), attrs, kids: Vec::new() });
            }
            Event::End { .. } => {
                let el = stack.pop()?;
                match stack.last_mut() {
                    Some(p) => p.kids.push(el),
                    None => root = Some(el),
                }
            }
            Event::Text(_) => {}
            Event::Eof => return root,
        }
    }
}

// ---------------------------------------------------------------- reading

/// doc.data from the data parts of a file just read (see the file's head); bounded by `nodes`
/// elements per part and the XML depth and attributes of `lim`
pub(crate) fn load(doc: &mut Document, lim: &crate::vsdx::Limits) {
    let find = |doc: &Document, ct: &str| doc.parts.iter().position(|p| doc.str(p.content_type) == ct);
    let (conn_at, rs_at) = (find(doc, CT_CONNECTIONS), find(doc, CT_RECORDSETS));
    if conn_at.is_none() && rs_at.is_none() {
        return;
    }
    let nodes = lim.items.max(1);
    let attrs = lim.attrs.max(16_384);
    let mut data = DataSets::default();
    if let Some(i) = conn_at {
        let Some(t) = tree(&doc.parts[i].data, lim.depth, attrs, nodes) else { return };
        if connections(&t, &mut data).is_err() {
            return;
        }
    }
    // the recordset parts by r:id (from recordsets.xml)
    let mut taken: Vec<Sym> = Vec::new();
    if let Some(i) = rs_at {
        let Some(t) = tree(&doc.parts[i].data, lim.depth, attrs, nodes) else { return };
        let source = doc.parts[i].name;
        let Ok(rel_ids) = recordsets(&t, &mut data) else { return };
        for (n, rid) in rel_ids.iter().enumerate() {
            let Some(rid) = rid else { continue };
            let target = doc.rels.iter().find(|r| same(doc, r.source, source) && doc.str(r.id) == rid && !r.external).and_then(|r| r.part);
            let Some(part) = target.and_then(|t| doc.parts.iter().find(|p| same(doc, p.name, t))) else { continue };
            let rs = &mut data.recordsets[n];
            match read_ado(&part.data, rs, lim.depth, attrs) {
                Some(rows) => rs.rows = Arc::new(rows),
                None => rs.raw = Some(Arc::new(part.data.clone())),
            }
            taken.push(part.name);
        }
        taken.push(source);
    }
    if let Some(i) = conn_at {
        taken.push(doc.parts[i].name);
    }
    // the model holds them now: the parts go (the relationships from recordsets.xml stay where
    // they are, with_parts writes them anew in their place: the same order on every write)
    let gone = |doc: &Document, s: Sym| taken.iter().any(|&t| same(doc, t, s));
    let keep: Vec<bool> = doc.parts.iter().map(|p| !gone(doc, p.name)).collect();
    let mut it = keep.iter();
    doc.parts.retain(|_| *it.next().unwrap_or(&true));
    doc.data = data;
}

// two part names the same (with or without the leading '/', ASCII case folded)
fn same(doc: &Document, a: Sym, b: Sym) -> bool {
    let (a, b) = (doc.str(a).trim_start_matches('/'), doc.str(b).trim_start_matches('/'));
    a.eq_ignore_ascii_case(b)
}

fn connections(t: &El, data: &mut DataSets) -> Result<(), ()> {
    if t.local != "DataConnections" {
        return Err(());
    }
    data.next_connection = t.uint("NextID")?.unwrap_or(1);
    data.connections_attrs = t.others(&["NextID"]);
    for c in t.kids.iter().filter(|k| k.local == "DataConnection") {
        let conn = Connection {
            id: c.uint("ID")?.ok_or(())?,
            file_name: c.string("FileName").unwrap_or_default(),
            connection_string: c.string("ConnectionString"),
            command: c.string("Command"),
            friendly_name: c.string("FriendlyName"),
            timeout: c.uint("Timeout")?,
            always_use_file: c.flag("AlwaysUseConnectionFile")?,
            attrs: c.others(&["ID", "FileName", "ConnectionString", "Command", "FriendlyName", "Timeout", "AlwaysUseConnectionFile"]),
        };
        if data.connections.iter().any(|x| x.id == conn.id) {
            return Err(());
        }
        data.connections.push(conn);
    }
    Ok(())
}

const RS_ATTRS: [&str; 13] = [
    "ID", "ConnectionID", "Command", "Options", "TimeRefreshed", "NextRowID", "Name", "RowOrder", "RefreshOverwriteAll",
    "RefreshNoReconciliationUI", "RefreshInterval", "ReplaceLinks", "Checksum",
];
const COL_ATTRS: [&str; 14] = [
    "ColumnNameID", "Name", "Label", "OrigLabel", "LangID", "Calendar", "DataType", "UnitType", "Currency", "Degree", "DisplayWidth",
    "DisplayOrder", "Mapped", "Hyperlink",
];

// the recordsets -> the r:id of each one's data part
fn recordsets(t: &El, data: &mut DataSets) -> Result<Vec<Option<String>>, ()> {
    if t.local != "DataRecordSets" {
        return Err(());
    }
    data.next_recordset = t.uint("NextID")?.unwrap_or(1);
    data.active = t.uint("ActiveRecordsetID")?;
    data.window_order = t.string("DataWindowOrder");
    data.recordsets_attrs = t.others(&["NextID", "ActiveRecordsetID", "DataWindowOrder"]);
    let mut rel_ids = Vec::new();
    for e in t.kids.iter().filter(|k| k.local == "DataRecordSet") {
        let mut rs = Recordset {
            id: e.uint("ID")?.ok_or(())?,
            connection: e.uint("ConnectionID")?,
            command: e.string("Command"),
            options: e.uint("Options")?,
            time_refreshed: e.string("TimeRefreshed"),
            next_row: e.uint("NextRowID")?.unwrap_or(1),
            name: e.string("Name").unwrap_or_default(),
            row_order: e.flag("RowOrder")?.unwrap_or(false),
            overwrite_all: e.flag("RefreshOverwriteAll")?.unwrap_or(false),
            no_reconcile_ui: e.flag("RefreshNoReconciliationUI")?,
            refresh_interval: e.uint("RefreshInterval")?.unwrap_or(0),
            replace_links: e.uint("ReplaceLinks")?,
            checksum: e.uint("Checksum")?,
            attrs: e.others(&RS_ATTRS),
            ..Recordset::default()
        };
        if data.recordsets.iter().any(|x| x.id == rs.id) {
            return Err(());
        }
        let mut rid = None;
        let mut cols_seen = false;
        for k in &e.kids {
            match k.local.as_str() {
                "Rel" => rid = k.attrs.iter().find(|a| a.ns == Ns::Rel && a.local == "id").map(|a| a.value.clone()),
                "DataColumns" => {
                    cols_seen = true;
                    rs.sort_column = k.string("SortColumn");
                    rs.sort_asc = k.flag("SortAsc")?;
                    for c in k.kids.iter().filter(|c| c.local == "DataColumn") {
                        let col = Column {
                            id: c.string("ColumnNameID").ok_or(())?,
                            name: c.string("Name").ok_or(())?,
                            label: c.string("Label").unwrap_or_default(),
                            orig_label: c.string("OrigLabel"),
                            lang: c.uint("LangID")?,
                            calendar: c.uint("Calendar")?.map(|x| x.min(u16::MAX as u32) as u16),
                            data_type: c.uint("DataType")?.unwrap_or(0).min(u16::MAX as u32) as u16,
                            unit_type: c.string("UnitType"),
                            currency: c.uint("Currency")?.map(|x| x.min(u16::MAX as u32) as u16),
                            degree: c.uint("Degree")?,
                            display_width: c.uint("DisplayWidth")?,
                            display_order: c.uint("DisplayOrder")?,
                            mapped: c.flag("Mapped")?,
                            hyperlink: c.flag("Hyperlink")?,
                            attrs: c.others(&COL_ATTRS),
                        };
                        if rs.columns.iter().any(|x| x.id == col.id) {
                            return Err(());
                        }
                        rs.columns.push(col);
                    }
                }
                "PrimaryKey" => {
                    let column = k.string("ColumnNameID").ok_or(())?;
                    let mut values = Vec::new();
                    for v in k.kids.iter().filter(|v| v.local == "RowKeyValue") {
                        values.push((v.uint("RowID")?.ok_or(())?, v.string("Value").unwrap_or_default()));
                    }
                    rs.key.push(Key { column, values: Arc::new(values) });
                }
                "RowMap" | "RefreshConflict" => {
                    let l = RowLink { row: k.uint("RowID")?.ok_or(())?, page: k.uint("PageID")?.ok_or(())?, shape: k.uint("ShapeID")?.ok_or(())? };
                    if k.local == "RowMap" { rs.links.push(l) } else { rs.conflicts.push(l) }
                }
                "AutoLinkComparison" => rs.auto_link.push(AutoLink {
                    column: k.string("ColumnName").unwrap_or_default(),
                    context: k.uint("ContextType")?.unwrap_or(0),
                    label: k.string("ContextTypeLabel"),
                }),
                // an unknown child: not understood, the parts stay as they are
                _ => return Err(()),
            }
        }
        if !cols_seen {
            return Err(());
        }
        rel_ids.push(rid);
        data.recordsets.push(rs);
    }
    Ok(rel_ids)
}

/// the rows of an ADO persisted recordset (ADO classic XML) for a recordset's columns: values
/// matched to the columns by name (rs:name, else name; ColumnNameID), typed by dt:type; row IDs
/// as the file's head says.  None when it is not such a recordset.
pub(crate) fn read_ado(bytes: &[u8], rs: &Recordset, depth: usize, attrs: usize) -> Option<Vec<DataRow>> {
    let text = xml::text_of(bytes)?;
    let mut r = Reader::new(&text, depth, attrs);
    // (attribute name in z:row, column name, dt:type, rs:dbtype)
    let mut schema: Vec<(String, String, String, String)> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut rows: Vec<Vec<(usize, String)>> = Vec::new();
    let mut root = false;
    let mut in_data = false;
    loop {
        match r.next().ok()? {
            Event::Start(s) => {
                if !root {
                    if s.local != "xml" {
                        return None;
                    }
                    root = true;
                    continue;
                }
                let attr = |name: &str, ns: bool| s.attrs.iter().find(|a| a.local == name && (a.ns != Ns::None) == ns).map(|a| String::from(&*a.value));
                match s.local {
                    "AttributeType" => {
                        let name = attr("name", false)?;
                        let col = attr("name", true).unwrap_or_else(|| name.clone());
                        schema.push((name, col, String::new(), String::new()));
                        cur = Some(schema.len() - 1);
                    }
                    "datatype" => {
                        if let Some(c) = cur.and_then(|c| schema.get_mut(c)) {
                            c.2 = attr("type", true).unwrap_or_default();
                            c.3 = attr("dbtype", true).unwrap_or_default();
                        }
                    }
                    "data" => in_data = true,
                    "row" if in_data => {
                        let mut v = Vec::new();
                        for a in &s.attrs {
                            if a.ns == Ns::Decl {
                                continue;
                            }
                            if let Some(i) = schema.iter().position(|c| c.0 == a.local) {
                                v.push((i, String::from(&*a.value)));
                            }
                        }
                        rows.push(v);
                    }
                    _ => {}
                }
            }
            Event::End { .. } => {}
            Event::Text(_) => {}
            Event::Eof => break,
        }
    }
    if !root {
        return None;
    }
    // schema column -> recordset column
    let map: Vec<Option<usize>> = schema.iter().map(|c| rs.column_by_id(&c.1)).collect();
    let id_col = schema.iter().position(|c| c.1 == ROW_ID);
    let n = rs.columns.len();
    let mut out = Vec::with_capacity(rows.len());
    for (pos, raw) in rows.into_iter().enumerate() {
        let mut values = alloc::vec![DataValue::Empty; n];
        let mut id = None;
        for (i, text) in raw {
            if Some(i) == id_col {
                id = text.trim().parse::<u32>().ok();
                continue;
            }
            if let Some(Some(c)) = map.get(i) {
                values[*c] = typed(&schema[i].2, &schema[i].3, &text);
            }
        }
        out.push((id, pos as u32 + 1, values));
    }
    // IDs: Visio_RowID, else the stored key values, else the position
    let by_key: Option<alloc::collections::BTreeMap<String, u32>> = (!rs.key.is_empty() && id_col.is_none()).then(|| {
        let mut m = alloc::collections::BTreeMap::new();
        if let Some(first) = rs.key.first() {
            for (row, _) in first.values.iter() {
                let parts: Vec<&str> = rs.key.iter().map(|k| k.values.iter().find(|(r, _)| r == row).map_or("", |(_, v)| v.as_str())).collect();
                m.entry(parts.join("\t")).or_insert(*row);
            }
        }
        m
    });
    let mut used = alloc::collections::BTreeSet::new();
    let mut next = rs.next_row.max(1);
    let rows: Vec<DataRow> = out
        .into_iter()
        .map(|(id, pos, values)| {
            let probe = DataRow { id: pos, values };
            let want = id.or_else(|| match &by_key {
                Some(m) => m.get(&rs.key_of(&probe)).copied(),
                None => Some(pos),
            });
            let id = match want {
                Some(x) if used.insert(x) => x,
                _ => loop {
                    let x = next;
                    next = next.saturating_add(1);
                    if used.insert(x) {
                        break x;
                    }
                },
            };
            DataRow { id, values: probe.values }
        })
        .collect();
    Some(rows)
}

// a z:row attribute by its column's dt:type
fn typed(dt: &str, dbtype: &str, text: &str) -> DataValue {
    let t = text.trim();
    match dt {
        "boolean" => match t.to_ascii_lowercase().as_str() {
            "true" | "1" | "-1" => DataValue::Bool(true),
            "false" | "0" => DataValue::Bool(false),
            _ => DataValue::Text(String::from(text)),
        },
        "int" | "i1" | "i2" | "i4" | "i8" | "ui1" | "ui2" | "ui4" | "ui8" | "float" | "r4" | "r8" | "number" | "fixed.14.4" => match t.parse::<f64>() {
            Ok(x) if x.is_finite() && dbtype == "currency" => DataValue::Currency(x),
            Ok(x) if x.is_finite() => DataValue::Num(x),
            _ => DataValue::Text(String::from(text)),
        },
        "dateTime" | "dateTime.tz" | "date" | "time" | "time.tz" => match iso_days(t) {
            Some(d) => DataValue::Date(d),
            None => DataValue::Text(String::from(text)),
        },
        _ => DataValue::Text(String::from(text)),
    }
}

// "2026-11-23T14:05:00" (date, optional time, fraction and zone ignored) -> days since
// 1899-12-30; a bare time "14:05:00" counts from that day
fn iso_days(s: &str) -> Option<f64> {
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, t),
        None if s.contains(':') => ("1899-12-30", s),
        None => (s, ""),
    };
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let dd: u32 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&dd) {
        return None;
    }
    let time = time.split(['Z', '+']).next().unwrap_or("");
    let mut t = time.split(':');
    let h: u32 = t.next().filter(|x| !x.is_empty()).map_or(Some(0), |x| x.parse().ok())?;
    let mi: u32 = t.next().map_or(Some(0), |x| x.parse().ok())?;
    let sec: f64 = t.next().map_or(Some(0.0), |x| x.parse().ok())?;
    if h > 24 || mi > 59 || !(0.0..61.0).contains(&sec) {
        return None;
    }
    let whole = sec as u32;
    Some(diagram_core::field::ole_days(y, m, dd, h, mi, whole) + (sec - whole as f64) / 86_400.0)
}

// ---------------------------------------------------------------- writing

/// a copy of the document with doc.data as parts (connections.xml, recordsets.xml, one data
/// part per recordset) and the relationships from recordsets.xml to them; None when the
/// document has no data
pub(crate) fn with_parts(doc: &Document) -> Option<Document> {
    let data = &doc.data;
    if data.connections.is_empty() && data.recordsets.is_empty() {
        return None;
    }
    let mut d = doc.clone();
    let taken = |d: &Document, name: &str| d.parts.iter().any(|p| d.str(p.name).trim_start_matches('/').eq_ignore_ascii_case(name.trim_start_matches('/')));
    let free = |d: &Document, first: &str, stem: &str| -> String {
        if !taken(d, first) {
            return String::from(first);
        }
        (2..).map(|n| format!("/visio/data/{stem}{n}.xml")).find(|c| !taken(d, c)).unwrap_or_default()
    };
    // the parts go before the windows part when there is one (pack writes its own windows part
    // last; read back it is a kept part, and the data parts stay before it on every write)
    let mut at = d.parts.iter().position(|p| d.str(p.content_type) == crate::vsdx::CT_WINDOWS).unwrap_or(d.parts.len());
    let mut add = |d: &mut Document, name: &str, ct: &str, rel: &str, bytes: Vec<u8>| {
        let part = Part { name: d.intern(name), content_type: d.intern(ct), rel_type: d.intern(rel), data: bytes };
        d.parts.insert(at, part);
        at += 1;
    };
    if !data.connections.is_empty() {
        let name = free(&d, "/visio/data/connections.xml", "connections");
        add(&mut d, &name, CT_CONNECTIONS, REL_CONNECTIONS, connections_xml(data).into_bytes());
    }
    if !data.recordsets.is_empty() {
        let rs_name = free(&d, "/visio/data/recordsets.xml", "recordsets");
        // the relationships from recordsets.xml: written anew where the old ones stood
        let src = d.intern(&rs_name);
        let mut rel_at = None;
        let mut i = 0;
        while i < d.rels.len() {
            if same(&d, d.rels[i].source, src) {
                d.rels.remove(i);
                rel_at.get_or_insert(i);
            } else {
                i += 1;
            }
        }
        let mut rel_at = rel_at.unwrap_or(d.rels.len());
        // the data parts first (their names decide the r:ids)
        let mut ids = Vec::new();
        for (i, rs) in data.recordsets.iter().enumerate() {
            let name = free(&d, &format!("/visio/data/recordset{}.xml", i + 1), "recordset");
            let bytes = match &rs.raw {
                Some(raw) => raw.as_ref().clone(),
                None => ado(rs).into_bytes(),
            };
            add(&mut d, &name, CT_RECORDSET, REL_RECORDSET, bytes);
            let id = format!("rId{}", i + 1);
            let target = String::from(name.rsplit('/').next().unwrap_or(&name));
            let rel = Relationship {
                source: d.intern(&rs_name),
                id: d.intern(&id),
                kind: d.intern(REL_RECORDSET),
                target: d.intern(&target),
                external: false,
                part: Some(d.intern(&name)),
            };
            d.rels.insert(rel_at, rel);
            rel_at += 1;
            ids.push(id);
        }
        add(&mut d, &rs_name, CT_RECORDSETS, REL_RECORDSETS, recordsets_xml(data, &ids).into_bytes());
    }
    Some(d)
}

fn attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("='");
    escape(out, value, true);
    out.push('\'');
}

fn opt<T: core::fmt::Display>(out: &mut String, name: &str, v: Option<T>) {
    if let Some(v) = v {
        attr(out, name, &format!("{v}"));
    }
}

fn bit(out: &mut String, name: &str, v: Option<bool>) {
    if let Some(b) = v {
        attr(out, name, if b { "1" } else { "0" });
    }
}

fn others(out: &mut String, list: &[(String, String)]) {
    for (k, v) in list {
        attr(out, k, v);
    }
}

fn connections_xml(data: &DataSets) -> String {
    let mut o = String::from(DECL);
    o.push_str("<DataConnections");
    attr(&mut o, "xmlns", NS_MAIN);
    attr(&mut o, "xmlns:r", NS_R);
    let next = data.connections.iter().map(|c| c.id + 1).max().unwrap_or(1).max(data.next_connection);
    attr(&mut o, "NextID", &format!("{next}"));
    others(&mut o, &data.connections_attrs);
    o.push('>');
    for c in &data.connections {
        o.push_str("<DataConnection");
        attr(&mut o, "ID", &format!("{}", c.id));
        attr(&mut o, "FileName", &c.file_name);
        if let Some(s) = &c.connection_string {
            attr(&mut o, "ConnectionString", s);
        }
        if let Some(s) = &c.command {
            attr(&mut o, "Command", s);
        }
        if let Some(s) = &c.friendly_name {
            attr(&mut o, "FriendlyName", s);
        }
        opt(&mut o, "Timeout", c.timeout);
        bit(&mut o, "AlwaysUseConnectionFile", c.always_use_file);
        others(&mut o, &c.attrs);
        o.push_str("/>");
    }
    o.push_str("</DataConnections>");
    o
}

fn recordsets_xml(data: &DataSets, ids: &[String]) -> String {
    let mut o = String::from(DECL);
    o.push_str("<DataRecordSets");
    attr(&mut o, "xmlns", NS_MAIN);
    attr(&mut o, "xmlns:r", NS_R);
    let next = data.recordsets.iter().map(|r| r.id + 1).max().unwrap_or(1).max(data.next_recordset);
    attr(&mut o, "NextID", &format!("{next}"));
    opt(&mut o, "ActiveRecordsetID", data.active);
    if let Some(w) = &data.window_order {
        attr(&mut o, "DataWindowOrder", w);
    }
    others(&mut o, &data.recordsets_attrs);
    o.push('>');
    for (rs, rid) in data.recordsets.iter().zip(ids) {
        o.push_str("<DataRecordSet");
        attr(&mut o, "ID", &format!("{}", rs.id));
        opt(&mut o, "ConnectionID", rs.connection);
        if let Some(s) = &rs.command {
            attr(&mut o, "Command", s);
        }
        opt(&mut o, "Options", rs.options);
        if let Some(s) = &rs.time_refreshed {
            attr(&mut o, "TimeRefreshed", s);
        }
        let next_row = rs.rows.iter().map(|r| r.id + 1).max().unwrap_or(1).max(rs.next_row);
        attr(&mut o, "NextRowID", &format!("{next_row}"));
        attr(&mut o, "Name", &rs.name);
        // RowOrder: 1 without key columns, absent or 0 with them ([MS-VSDX])
        attr(&mut o, "RowOrder", if rs.key.is_empty() { "1" } else { "0" });
        attr(&mut o, "RefreshOverwriteAll", if rs.overwrite_all { "1" } else { "0" });
        bit(&mut o, "RefreshNoReconciliationUI", rs.no_reconcile_ui);
        attr(&mut o, "RefreshInterval", &format!("{}", rs.refresh_interval));
        opt(&mut o, "ReplaceLinks", rs.replace_links);
        opt(&mut o, "Checksum", rs.checksum);
        others(&mut o, &rs.attrs);
        o.push('>');
        o.push_str("<Rel");
        attr(&mut o, "r:id", rid);
        o.push_str("/>");
        o.push_str("<DataColumns");
        if let Some(s) = &rs.sort_column {
            attr(&mut o, "SortColumn", s);
        }
        bit(&mut o, "SortAsc", rs.sort_asc);
        o.push('>');
        for c in &rs.columns {
            o.push_str("<DataColumn");
            attr(&mut o, "ColumnNameID", &c.id);
            attr(&mut o, "Name", &c.name);
            attr(&mut o, "Label", &c.label);
            if let Some(s) = &c.orig_label {
                attr(&mut o, "OrigLabel", s);
            }
            opt(&mut o, "LangID", c.lang);
            opt(&mut o, "Calendar", c.calendar);
            attr(&mut o, "DataType", &format!("{}", c.data_type));
            if let Some(s) = &c.unit_type {
                attr(&mut o, "UnitType", s);
            }
            opt(&mut o, "Currency", c.currency);
            opt(&mut o, "Degree", c.degree);
            opt(&mut o, "DisplayWidth", c.display_width);
            opt(&mut o, "DisplayOrder", c.display_order);
            bit(&mut o, "Mapped", c.mapped);
            bit(&mut o, "Hyperlink", c.hyperlink);
            others(&mut o, &c.attrs);
            o.push_str("/>");
        }
        o.push_str("</DataColumns>");
        for k in &rs.key {
            o.push_str("<PrimaryKey");
            attr(&mut o, "ColumnNameID", &k.column);
            if k.values.is_empty() {
                o.push_str("/>");
                continue;
            }
            o.push('>');
            for (row, v) in k.values.iter() {
                o.push_str("<RowKeyValue");
                attr(&mut o, "RowID", &format!("{row}"));
                attr(&mut o, "Value", v);
                o.push_str("/>");
            }
            o.push_str("</PrimaryKey>");
        }
        for (tag, list) in [("RowMap", &rs.links), ("RefreshConflict", &rs.conflicts)] {
            for l in list.iter() {
                o.push('<');
                o.push_str(tag);
                attr(&mut o, "RowID", &format!("{}", l.row));
                attr(&mut o, "PageID", &format!("{}", l.page));
                attr(&mut o, "ShapeID", &format!("{}", l.shape));
                o.push_str("/>");
            }
        }
        for a in &rs.auto_link {
            o.push_str("<AutoLinkComparison");
            attr(&mut o, "ColumnName", &a.column);
            attr(&mut o, "ContextType", &format!("{}", a.context));
            if let Some(l) = &a.label {
                attr(&mut o, "ContextTypeLabel", l);
            }
            o.push_str("/>");
        }
        o.push_str("</DataRecordSet>");
    }
    o.push_str("</DataRecordSets>");
    o
}

/// a recordset's rows as an ADO persisted recordset (ADO classic XML): one AttributeType per
/// column (c1, c2 ... with rs:name = the column's source name; types from DataType: string,
/// float, boolean, dateTime, currency), rs:data with one z:row per row, empty values left out
pub fn ado(rs: &Recordset) -> String {
    let mut o = String::from(DECL);
    o.push_str("<xml xmlns:s='uuid:BDC6E3F0-6DA3-11d1-A2A3-00AA00C14882' xmlns:dt='uuid:C2F41010-65B3-11d1-A29F-00AA00C14882' xmlns:rs='urn:schemas-microsoft-com:rowset' xmlns:z='#RowsetSchema'>\r\n");
    o.push_str("<s:Schema id='RowsetSchema'>\r\n<s:ElementType name='row' content='eltOnly' rs:updatable='true'>\r\n");
    use diagram_core::data::kind;
    for (i, c) in rs.columns.iter().enumerate() {
        o.push_str("<s:AttributeType");
        attr(&mut o, "name", &format!("c{}", i + 1));
        attr(&mut o, "rs:name", &c.id);
        attr(&mut o, "rs:number", &format!("{}", i + 1));
        o.push_str(" rs:nullable='true' rs:maydefer='true' rs:write='true'>");
        let long = rs.rows.iter().any(|r| matches!(r.values.get(i), Some(DataValue::Text(s)) if s.chars().count() > 255));
        let dt = match c.data_type {
            kind::NUMBER | kind::DURATION => "<s:datatype dt:type='float' dt:maxLength='8' rs:precision='15' rs:fixedlength='true'/>",
            kind::BOOL => "<s:datatype dt:type='boolean' dt:maxLength='2' rs:fixedlength='true'/>",
            kind::DATE => "<s:datatype dt:type='dateTime' rs:dbtype='variantdate' dt:maxLength='16' rs:fixedlength='true'/>",
            kind::CURRENCY => "<s:datatype dt:type='number' rs:dbtype='currency' dt:maxLength='8' rs:precision='19' rs:fixedlength='true'/>",
            _ if long => "<s:datatype dt:type='string' dt:maxLength='536870910' rs:long='true'/>",
            _ => "<s:datatype dt:type='string' dt:maxLength='255' rs:precision='0'/>",
        };
        o.push_str(dt);
        o.push_str("</s:AttributeType>\r\n");
    }
    o.push_str("<s:extends type='rs:rowbase'/>\r\n</s:ElementType>\r\n</s:Schema>\r\n<rs:data>\r\n");
    for r in rs.rows.iter() {
        o.push_str("<z:row");
        for (i, v) in r.values.iter().enumerate().take(rs.columns.len()) {
            let text: Cow<str> = match v {
                DataValue::Empty => continue,
                DataValue::Bool(b) => Cow::Borrowed(if *b { "True" } else { "False" }),
                DataValue::Date(d) => {
                    let (y, m, dd, h, mi, s, _) = diagram_core::field::civil(*d);
                    Cow::Owned(format!("{y:04}-{m:02}-{dd:02}T{h:02}:{mi:02}:{s:02}"))
                }
                other => Cow::Owned(other.text()),
            };
            attr(&mut o, &format!("c{}", i + 1), &text);
        }
        o.push_str("/>\r\n");
    }
    o.push_str("</rs:data>\r\n</xml>");
    o
}

#[cfg(test)]
mod tests;
