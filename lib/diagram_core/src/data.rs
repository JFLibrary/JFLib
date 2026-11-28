// data -- (JD18) external data in a document: connections, recordsets with their rows, rows
// linked to shapes (DA-04, DA-05, DF-12)
//
//   doc.data: DataSets              connections, recordsets (Document::data; diagram_io reads and
//                                   writes them as Visio's connections.xml, recordsets.xml and one
//                                   recordset part per recordset)
//   Recordset                       columns, the key (or the row order), the cached rows (shown
//                                   without file or network access), the links row -> shape,
//                                   refresh settings and conflicts as Visio keeps them
//   Recordset::from_table(..)       a new recordset from columns and rows (types guessed per column)
//   doc.data_link(shape, rs)        the row a shape is linked to; doc.data_links(shape) all of them
//   rs.shapes_of(row)               the shapes linked to a row (page ID, shape ID)
//
//   Transactions (undo like every edit; they return the Value cells written, for the formula
//   engine's recalc_cells):
//   add_recordset(tx, conn, rs)     -> the recordset's ID (IDs and a unique name handed out)
//   remove_recordset(tx, id)        the recordset, its links, its connection when no other uses it
//   set_recordset(tx, rs)           replace one (column settings, refresh interval, name ...)
//   link(tx, shape, rs, row)        bind a row to a shape: one Property row per column (Label,
//                                   Type, Value, DataLinked = 1); a shape has at most one row of
//                                   a recordset, an earlier one is replaced
//   unlink(tx, shape, rs)           the binding goes, the shape data stays with its last values
//   auto_link(tx, shapes, rs, pairs, replace)   AS-04: a column against a shape field (text,
//                                   name or a shape data field) -> the rows bound where they match
//   refresh(tx, rs, columns, rows, now)   DA-05: new data for a recordset: rows matched by key
//                                   (or by order), linked shapes updated, conflicts reported;
//                                   missing_key(doc, rs, columns): key columns the source lacks
//                                   (refresh refuses those)
//   user_edited(tx, sheet, name)    a user's change of a linked value: DataLinked = 0, so a
//                                   refresh keeps it (unless the recordset overwrites all)
//   resolve_*                       what the user decides about a conflict
//
// Visio's rules (MS-VSDX 2.2.10.2, 2.3.4.2.39): a column maps to the shape data rows whose name
// (N) equals the column's Name; a RowMap binds a row (RowID) to a shape (PageID, ShapeID);
// with PrimaryKey elements the key columns identify a row across refreshes, without them
// (RowOrder = 1) its position does; RefreshOverwriteAll = 0 updates only the Value cells whose
// row has DataLinked = 1.  Row IDs start at 1 in the order of the source (Visio's DataAsXML).
// Visio's codes stay as they are in the model: Type and DataType 0 text, 1 fixed list,
// 2 number, 3 Boolean, 4 variable list, 5 date, 6 duration, 7 currency.  New columns get Visio's
// row names ("_VisDM_" and the source name's letters and digits, ROW_PREFIX).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::names;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::units::Unit;

/// Visio's data type codes (a column's DataType, a shape data row's Type)
pub mod kind {
    /// text (string)
    pub const TEXT: u16 = 0;
    /// fixed list
    pub const FIXED_LIST: u16 = 1;
    /// number
    pub const NUMBER: u16 = 2;
    /// Boolean
    pub const BOOL: u16 = 3;
    /// variable list
    pub const VAR_LIST: u16 = 4;
    /// date and time
    pub const DATE: u16 = 5;
    /// duration
    pub const DURATION: u16 = 6;
    /// currency
    pub const CURRENCY: u16 = 7;
}

/// the document's external data: Visio's DataConnections and DataRecordSets
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DataSets {
    /// the connections (an Excel workbook, a text file ...)
    pub connections: Vec<Connection>,
    /// NextID of DataConnections
    pub next_connection: u32,
    /// the recordsets in file order (the External Data pane's tabs)
    pub recordsets: Vec<Recordset>,
    /// NextID of DataRecordSets
    pub next_recordset: u32,
    /// ActiveRecordsetID: the tab shown
    pub active: Option<u32>,
    /// DataWindowOrder as written
    pub window_order: Option<String>,
    /// unknown attributes of DataConnections
    pub connections_attrs: Vec<(String, String)>,
    /// unknown attributes of DataRecordSets
    pub recordsets_attrs: Vec<(String, String)>,
}

/// a data connection (DataConnection)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Connection {
    /// ID, unique among the connections
    pub id: u32,
    /// FileName: an .odc or data source file ("" when the connection string says it all)
    pub file_name: String,
    /// ConnectionString (an OLE DB / ODBC string; see excel_connection, text_connection)
    pub connection_string: Option<String>,
    /// Command: the query
    pub command: Option<String>,
    /// FriendlyName
    pub friendly_name: Option<String>,
    /// Timeout
    pub timeout: Option<u32>,
    /// AlwaysUseConnectionFile
    pub always_use_file: Option<bool>,
    /// unknown attributes as read
    pub attrs: Vec<(String, String)>,
}

/// a column of a recordset (DataColumn)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Column {
    /// ColumnNameID: the column's name in the source, unique in the recordset
    pub id: String,
    /// Name: the name of the shape data rows (Prop.<Name>) the column maps to, unique
    pub name: String,
    /// Label: what the user sees (the shape data label; renaming a column changes it)
    pub label: String,
    /// OrigLabel
    pub orig_label: Option<String>,
    /// LangID as written: Visio writes a culture name ("de-DE"), not the number [MS-VSDX]'s
    /// schema names
    pub lang: Option<String>,
    /// Calendar
    pub calendar: Option<u16>,
    /// DataType (kind::*)
    pub data_type: u16,
    /// UnitType
    pub unit_type: Option<String>,
    /// Currency
    pub currency: Option<u16>,
    /// Degree
    pub degree: Option<u32>,
    /// DisplayWidth
    pub display_width: Option<u32>,
    /// DisplayOrder
    pub display_order: Option<u32>,
    /// Mapped
    pub mapped: Option<bool>,
    /// Hyperlink
    pub hyperlink: Option<bool>,
    /// unknown attributes as read
    pub attrs: Vec<(String, String)>,
}

/// a cell of a recordset's row
#[derive(Clone, PartialEq, Debug, Default)]
pub enum DataValue {
    /// no value
    #[default]
    Empty,
    /// text
    Text(String),
    /// a number
    Num(f64),
    /// a truth value
    Bool(bool),
    /// a date: days since 30 December 1899 (the fraction is the time of day)
    Date(f64),
    /// an amount of money
    Currency(f64),
}

impl DataValue {
    /// the value as the External Data pane shows it and as keys and matching compare it:
    /// numbers in their shortest form, dates "2026-11-23" (with " 14:05:00" when there is a
    /// time), TRUE / FALSE
    pub fn text(&self) -> String {
        match self {
            DataValue::Empty => String::new(),
            DataValue::Text(s) => s.clone(),
            DataValue::Num(x) | DataValue::Currency(x) => crate::formula::fmt_num(*x),
            DataValue::Bool(b) => String::from(if *b { "TRUE" } else { "FALSE" }),
            DataValue::Date(d) => {
                let (y, m, dd, h, mi, s, _) = crate::field::civil(*d);
                if h == 0 && mi == 0 && s == 0 {
                    format!("{y:04}-{m:02}-{dd:02}")
                } else {
                    format!("{y:04}-{m:02}-{dd:02} {h:02}:{mi:02}:{s:02}")
                }
            }
        }
    }

    /// the data type a value of this kind has (kind::*)
    pub fn kind(&self) -> u16 {
        match self {
            DataValue::Empty | DataValue::Text(_) => kind::TEXT,
            DataValue::Num(_) => kind::NUMBER,
            DataValue::Bool(_) => kind::BOOL,
            DataValue::Date(_) => kind::DATE,
            DataValue::Currency(_) => kind::CURRENCY,
        }
    }
}

/// a row of a recordset with its Visio row ID
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DataRow {
    /// the row ID (from 1; stays the same across refreshes while the key matches)
    pub id: u32,
    /// one value per column, in the order of Recordset::columns
    pub values: Vec<DataValue>,
}

/// a row bound to a shape (RowMap; RefreshConflict has the same three numbers)
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct RowLink {
    /// RowID
    pub row: u32,
    /// PageID: the ID of the shape's page
    pub page: u32,
    /// ShapeID: the shape's ID on that page
    pub shape: u32,
}

/// a key column with the key value of every row (PrimaryKey with its RowKeyValue elements)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Key {
    /// ColumnNameID of the key column
    pub column: String,
    /// (RowID, value) per row
    pub values: Arc<Vec<(u32, String)>>,
}

/// AutoLinkComparison as read (which column was compared with which shape field)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AutoLink {
    /// ColumnName
    pub column: String,
    /// ContextType
    pub context: u32,
    /// ContextTypeLabel
    pub label: Option<String>,
}

/// a recordset (DataRecordSet): what a source delivered and how its rows are bound to shapes
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Recordset {
    /// ID, unique among the recordsets
    pub id: u32,
    /// ConnectionID (None: data without connection, e.g. pasted XML)
    pub connection: Option<u32>,
    /// Command: the query
    pub command: Option<String>,
    /// Options (bit 1: no refresh)
    pub options: Option<u32>,
    /// TimeRefreshed as written ("2026-11-23T14:05:00")
    pub time_refreshed: Option<String>,
    /// NextRowID: the next free row ID
    pub next_row: u32,
    /// Name: the tab's name in the External Data pane
    pub name: String,
    /// RowOrder: without key columns the position identifies a row
    pub row_order: bool,
    /// RefreshOverwriteAll: a refresh overwrites values the user changed too
    pub overwrite_all: bool,
    /// RefreshNoReconciliationUI
    pub no_reconcile_ui: Option<bool>,
    /// RefreshInterval in minutes (0: none)
    pub refresh_interval: u32,
    /// ReplaceLinks
    pub replace_links: Option<u32>,
    /// Checksum
    pub checksum: Option<u32>,
    /// DataColumns: the columns
    pub columns: Vec<Column>,
    /// SortColumn of DataColumns
    pub sort_column: Option<String>,
    /// SortAsc of DataColumns
    pub sort_asc: Option<bool>,
    /// the key columns with their values (empty: the row order identifies rows)
    pub key: Vec<Key>,
    /// RowMap: rows bound to shapes
    pub links: Vec<RowLink>,
    /// RefreshConflict: bound shapes whose row was not found at the last refresh
    pub conflicts: Vec<RowLink>,
    /// AutoLinkComparison as read
    pub auto_link: Vec<AutoLink>,
    /// the cached rows (the recordset part); shared so that an undo step is cheap
    pub rows: Arc<Vec<DataRow>>,
    /// the recordset part as read when it could not be understood: written back unchanged
    pub raw: Option<Arc<Vec<u8>>>,
    /// unknown attributes as read
    pub attrs: Vec<(String, String)>,
}

impl Recordset {
    /// a new recordset from a table: one column per header (types guessed from the values),
    /// row IDs from 1; `key`: the source names of the key columns (empty: the row order).
    /// IDs and the name are handed out by add_recordset.
    pub fn from_table(name: &str, headers: &[String], rows: Vec<Vec<DataValue>>, key: &[String]) -> Recordset {
        let mut columns: Vec<Column> = Vec::new();
        for (i, h) in headers.iter().enumerate() {
            let data_type = guess_kind(rows.iter().filter_map(|r| r.get(i)));
            let id = unique(h.trim(), |c| columns.iter().any(|x| x.id == c));
            let name = unique(&row_name(&id), |c| columns.iter().any(|x| x.name.eq_ignore_ascii_case(c)));
            columns.push(Column { id: id.clone(), name, label: id, data_type, ..Column::default() });
        }
        let n = columns.len();
        let rows: Vec<DataRow> = rows
            .into_iter()
            .enumerate()
            .map(|(i, mut v)| {
                v.resize(n, DataValue::Empty);
                DataRow { id: i as u32 + 1, values: v }
            })
            .collect();
        let mut rs = Recordset {
            name: String::from(name),
            next_row: rows.len() as u32 + 1,
            row_order: key.is_empty(),
            columns,
            rows: Arc::new(rows),
            ..Recordset::default()
        };
        rs.key = key.iter().filter(|k| rs.column_by_id(k).is_some()).map(|k| Key { column: k.clone(), values: Arc::new(Vec::new()) }).collect();
        rs.row_order = rs.key.is_empty();
        rs.update_key_values();
        rs
    }

    /// the column with this source name (ColumnNameID)
    pub fn column_by_id(&self, id: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    /// the column with this shape data name (Name)
    pub fn column_by_name(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// a row by ID
    pub fn row(&self, id: u32) -> Option<&DataRow> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// the shapes a row is bound to: (page ID, shape ID)
    pub fn shapes_of(&self, row: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.links.iter().filter(move |l| l.row == row).map(|l| (l.page, l.shape))
    }

    /// the row a shape is bound to
    pub fn row_of(&self, page: u32, shape: u32) -> Option<u32> {
        self.links.iter().find(|l| l.page == page && l.shape == shape).map(|l| l.row)
    }

    /// whether refreshing is switched off for it (Options bit 1)
    pub fn no_refresh(&self) -> bool {
        self.options.is_some_and(|o| o & 1 != 0)
    }

    /// a row's key as text (the key columns' values joined by TAB; the row ID without key)
    pub fn key_of(&self, row: &DataRow) -> String {
        if self.key.is_empty() {
            return format!("{}", row.id);
        }
        let mut s = String::new();
        for (i, k) in self.key.iter().enumerate() {
            if i > 0 {
                s.push('\t');
            }
            if let Some(c) = self.column_by_id(&k.column) {
                s.push_str(&row.values.get(c).map(DataValue::text).unwrap_or_default());
            }
        }
        s
    }

    /// recompute the RowKeyValue lists from the rows (after a change of rows or key columns)
    pub fn update_key_values(&mut self) {
        let rows = self.rows.clone();
        for k in &mut self.key {
            if let Some(c) = self.columns.iter().position(|x| x.id == k.column) {
                k.values = Arc::new(rows.iter().map(|r| (r.id, r.values.get(c).map(DataValue::text).unwrap_or_default())).collect());
            }
        }
    }
}

/// the prefix of the shape data rows Visio makes for a linked column ("Prop._VisDM_Ports")
pub const ROW_PREFIX: &str = "_VisDM_";

// a source name as a shape data row name the way Visio 16 names them (Visio check 2026-11-27):
// "_VisDM_" and the name's letters and digits ("IP-Adresse" -> "_VisDM_IPAdresse"); so a column
// linked here and the same column linked in Visio meet in one row
fn row_name(s: &str) -> String {
    let mut out = String::from(ROW_PREFIX);
    out.extend(s.trim().chars().filter(|c| c.is_alphanumeric()));
    if out.len() == ROW_PREFIX.len() {
        out.push('F');
    }
    out
}

// `base` itself when free, else base1, base2 ...
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = if base.is_empty() { "F" } else { base };
    if !taken(base) {
        return String::from(base);
    }
    (1..).map(|n| format!("{base}{n}")).find(|c| !taken(c)).unwrap_or_default()
}

/// the data type of a column from its values: number, date, Boolean or currency when every
/// non-empty value is one, else text
pub fn guess_kind<'a>(values: impl Iterator<Item = &'a DataValue>) -> u16 {
    let mut found: Option<u16> = None;
    for v in values {
        if matches!(v, DataValue::Empty) {
            continue;
        }
        let k = v.kind();
        match found {
            None => found = Some(k),
            Some(f) if f == k => {}
            Some(f) if (f == kind::NUMBER && k == kind::CURRENCY) || (f == kind::CURRENCY && k == kind::NUMBER) => found = Some(kind::NUMBER),
            Some(_) => return kind::TEXT,
        }
    }
    found.unwrap_or(kind::TEXT)
}

// ---------------------------------------------------------------- connections for sources

/// what an Excel connection reads: the table name of the OLE DB provider ("Tabelle1$" a sheet,
/// "Tabelle1$A1:C9" a range of a sheet, "Geräte" a named range)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExcelTable {
    /// a whole sheet (its used range)
    Sheet(String),
    /// a range of a sheet ("A1:C9")
    Range(String, String),
    /// a named range of the workbook
    Name(String),
}

impl ExcelTable {
    /// the provider's table name: "Tabelle1$", "Tabelle1$A1:C9", "Geräte"
    pub fn ole(&self) -> String {
        match self {
            ExcelTable::Sheet(s) => format!("{s}$"),
            ExcelTable::Range(s, r) => format!("{s}${}", r.replace('$', "")),
            ExcelTable::Name(n) => n.clone(),
        }
    }

    /// a provider's table name back (quotes around a sheet name removed)
    pub fn parse(table: &str) -> ExcelTable {
        let t = table.trim();
        match t.split_once('$') {
            Some((sheet, "")) => ExcelTable::Sheet(String::from(sheet.trim_matches('\''))),
            Some((sheet, range)) => ExcelTable::Range(String::from(sheet.trim_matches('\'')), String::from(range)),
            None => ExcelTable::Name(String::from(t)),
        }
    }
}

/// what a connection reads, as far as JFDiagrammer can read it itself
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// an Excel workbook: its path, the sheet, range or named range, whether the first row
    /// holds the column names
    Excel { path: String, table: ExcelTable, header: bool },
    /// a text file (CSV): its path, whether the first row holds the column names
    Text { path: String, header: bool },
    /// anything else (a database, a SharePoint list): shown from the cache, not refreshed
    Other,
}

/// the connection string and command Visio writes for an Excel workbook (the ACE OLE DB
/// provider, as Visio's data selector does; Visio refreshes through it)
pub fn excel_connection(path: &str, table: &ExcelTable, header: bool) -> (String, String) {
    let hdr = if header { "YES" } else { "NO" };
    let conn = format!(
        "Provider=Microsoft.ACE.OLEDB.12.0;User ID=Admin;Data Source={path};Mode=Read;Extended Properties=\"HDR={hdr};IMEX=1;MaxScanRows=0;Excel 12.0;\";Jet OLEDB:Engine Type=37"
    );
    (conn, format!("select * from `{}`", table.ole()))
}

/// the connection string and command for a text file (the ACE text driver; Visio's "Other OLE
/// DB" path): the folder is the data source, the file the table
pub fn text_connection(path: &str, header: bool) -> (String, String) {
    let (dir, file) = match path.rfind(['\\', '/']) {
        Some(i) => (&path[..i + 1], &path[i + 1..]),
        None => ("", path),
    };
    let hdr = if header { "YES" } else { "NO" };
    let conn = format!("Provider=Microsoft.ACE.OLEDB.12.0;Data Source={dir};Extended Properties=\"text;HDR={hdr};FMT=Delimited\"");
    (conn, format!("select * from `{file}`"))
}

/// what a recordset's connection reads (Source::Other when it is not a workbook or text file
/// JFDiagrammer can open)
pub fn source_of(data: &DataSets, rs: &Recordset) -> Source {
    let Some(conn) = rs.connection.and_then(|id| data.connections.iter().find(|c| c.id == id)) else { return Source::Other };
    let cs = conn.connection_string.as_deref().unwrap_or("");
    let command = rs.command.as_deref().or(conn.command.as_deref()).unwrap_or("");
    let props = conn_value(cs, "Extended Properties").unwrap_or_default();
    let header = !props.to_ascii_uppercase().replace(' ', "").contains("HDR=NO");
    let Some(ds) = conn_value(cs, "Data Source") else { return Source::Other };
    let object = command_object(command);
    let lower = props.to_ascii_lowercase();
    if lower.starts_with("text") || lower.contains(";text") || lower.contains("fmt=delimited") {
        let file = object.unwrap_or_default();
        let sep = if ds.ends_with(['\\', '/']) || ds.is_empty() { "" } else { "\\" };
        return Source::Text { path: format!("{ds}{sep}{file}"), header };
    }
    if lower.contains("excel") || ds.to_ascii_lowercase().ends_with(".xlsx") || ds.to_ascii_lowercase().ends_with(".xlsm") {
        let Some(obj) = object else { return Source::Other };
        return Source::Excel { path: ds, table: ExcelTable::parse(&obj), header };
    }
    Source::Other
}

// a value of a connection string: key=value;key="quoted;value" (case-insensitive key)
fn conn_value(cs: &str, key: &str) -> Option<String> {
    let mut rest = cs;
    while !rest.is_empty() {
        let (k, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let (v, next) = if let Some(q) = after.strip_prefix('"') {
            let end = q.find('"').unwrap_or(q.len());
            (&q[..end], q.get(end + 1..).unwrap_or("").trim_start_matches(|c| c != ';').trim_start_matches(';'))
        } else {
            match after.split_once(';') {
                Some((v, n)) => (v, n),
                None => (after, ""),
            }
        };
        if k.trim().eq_ignore_ascii_case(key) {
            return Some(String::from(v.trim()));
        }
        rest = next;
    }
    None
}

// the table of "select * from `Tabelle1$`" (or [Tabelle1$] / "Tabelle1$")
fn command_object(cmd: &str) -> Option<String> {
    let lower = cmd.to_ascii_lowercase();
    let at = lower.find(" from ")? + 6;
    let t = cmd[at..].trim();
    let (open, close) = match t.chars().next()? {
        '`' => ('`', '`'),
        '[' => ('[', ']'),
        '"' => ('"', '"'),
        _ => return Some(String::from(t.split_whitespace().next()?)),
    };
    let inner = &t[open.len_utf8()..];
    Some(String::from(&inner[..inner.find(close)?]))
}

// ---------------------------------------------------------------- reading the links

impl Document {
    /// the page ID and shape ID of a shape on a page (None for master shapes)
    pub fn data_ids(&self, shape: ShapeRef) -> Option<(u32, u32)> {
        let Owner::Page(p) = shape.owner else { return None };
        Some((self.page(p)?.header.id, self.shape(shape)?.id()))
    }

    /// the shape with this page ID and shape ID
    pub fn data_shape(&self, page: u32, shape: u32) -> Option<ShapeRef> {
        let p = self.page_by_id(page)?;
        self.shape_by_id(Owner::Page(p), shape)
    }

    /// the row of recordset `rs` a shape is bound to
    pub fn data_link(&self, shape: ShapeRef, rs: u32) -> Option<u32> {
        let (p, s) = self.data_ids(shape)?;
        self.data.recordsets.iter().find(|r| r.id == rs)?.row_of(p, s)
    }

    /// every binding of a shape: (recordset ID, row ID)
    pub fn data_links(&self, shape: ShapeRef) -> Vec<(u32, u32)> {
        let Some((p, s)) = self.data_ids(shape) else { return Vec::new() };
        self.data.recordsets.iter().filter_map(|r| r.row_of(p, s).map(|row| (r.id, row))).collect()
    }

    /// a recordset by ID
    pub fn recordset(&self, id: u32) -> Option<&Recordset> {
        self.data.recordsets.iter().find(|r| r.id == id)
    }
}

// ---------------------------------------------------------------- transactions

/// a Value cell a data edit wrote (for Engine::recalc_cells)
pub type Written = (SheetRef, CellAddr);

fn with_data<T>(tx: &mut Transaction, f: impl FnOnce(&mut DataSets) -> Result<T, EditError>) -> Result<T, EditError> {
    let mut d = tx.doc().data.clone();
    let r = f(&mut d)?;
    if d != tx.doc().data {
        tx.set_data(d)?;
    }
    Ok(r)
}

/// add a recordset (and its connection, if any) -> the recordset's ID.  The connection gets the
/// next connection ID, the recordset the next recordset ID and a name no other has
/// ("Daten", "Daten (2)" ...); it becomes the active one.
pub fn add_recordset(tx: &mut Transaction, conn: Option<Connection>, mut rs: Recordset) -> Result<u32, EditError> {
    with_data(tx, |d| {
        if let Some(mut c) = conn {
            c.id = d.next_connection.max(d.connections.iter().map(|x| x.id + 1).max().unwrap_or(1)).max(1);
            d.next_connection = c.id + 1;
            rs.connection = Some(c.id);
            d.connections.push(c);
        }
        rs.id = d.next_recordset.max(d.recordsets.iter().map(|x| x.id + 1).max().unwrap_or(1)).max(1);
        d.next_recordset = rs.id + 1;
        let base = if rs.name.trim().is_empty() { String::from("Daten") } else { String::from(rs.name.trim()) };
        rs.name = if d.recordsets.iter().any(|r| r.name == base) {
            (2..).map(|n| format!("{base} ({n})")).find(|c| !d.recordsets.iter().any(|r| &r.name == c)).unwrap_or(base)
        } else {
            base
        };
        let id = rs.id;
        d.recordsets.push(rs);
        d.active = Some(id);
        Ok(id)
    })
}

/// remove a recordset with its links (the shape data stays); its connection goes too when no
/// other recordset uses it
pub fn remove_recordset(tx: &mut Transaction, id: u32) -> Result<(), EditError> {
    with_data(tx, |d| {
        let pos = d.recordsets.iter().position(|r| r.id == id).ok_or(EditError::Invalid)?;
        let rs = d.recordsets.remove(pos);
        if let Some(c) = rs.connection {
            if !d.recordsets.iter().any(|r| r.connection == Some(c)) {
                d.connections.retain(|x| x.id != c);
            }
        }
        if d.active == Some(id) {
            d.active = d.recordsets.first().map(|r| r.id);
        }
        Ok(())
    })
}

/// replace a recordset (same ID): its name, columns' labels and types, refresh settings ...
pub fn set_recordset(tx: &mut Transaction, rs: Recordset) -> Result<(), EditError> {
    with_data(tx, |d| {
        let slot = d.recordsets.iter_mut().find(|r| r.id == rs.id).ok_or(EditError::Invalid)?;
        *slot = rs;
        Ok(())
    })
}

/// replace a connection (same ID)
pub fn set_connection(tx: &mut Transaction, conn: Connection) -> Result<(), EditError> {
    with_data(tx, |d| {
        let slot = d.connections.iter_mut().find(|c| c.id == conn.id).ok_or(EditError::Invalid)?;
        *slot = conn;
        Ok(())
    })
}

/// make a recordset the active one (the tab the External Data pane shows)
pub fn set_active(tx: &mut Transaction, id: u32) -> Result<(), EditError> {
    with_data(tx, |d| {
        d.active = Some(id);
        Ok(())
    })
}

// the Property rows of a column on a shape: Label, Type, Value, DataLinked
fn write_field(tx: &mut Transaction, sheet: SheetRef, col: &Column, value: &DataValue, out: &mut Vec<Written>) -> Result<(), EditError> {
    let (sk, rk) = tx.ensure_named_row(sheet, names::PROPERTY, &col.name)?;
    let label = tx.intern(&col.label);
    tx.set_constant(sheet, &CellAddr::row(sk, rk, names::LABEL), Value::Text(label), Some(Unit::Str))?;
    tx.set_constant(sheet, &CellAddr::row(sk, rk, names::TYPE), Value::Num(f64::from(col.data_type.min(7))), None)?;
    let addr = CellAddr::row(sk, rk, names::VALUE);
    set_value(tx, sheet, &addr, value)?;
    out.push((sheet, addr));
    let linked = tx.intern(DATA_LINKED);
    tx.set_constant(sheet, &CellAddr::row(sk, rk, linked), Value::Num(1.0), None)
}

/// the Property cell that says a value comes from linked data (1) or was changed by the user (0)
pub const DATA_LINKED: &str = "DataLinked";

// a data value into a Value cell, typed as Visio keeps it
fn set_value(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, value: &DataValue) -> Result<(), EditError> {
    let (v, u) = match value {
        DataValue::Empty => (Value::Text(tx.intern("")), Some(Unit::Str)),
        DataValue::Text(s) => (Value::Text(tx.intern(s)), Some(Unit::Str)),
        DataValue::Num(x) => (Value::Num(*x), None),
        DataValue::Bool(b) => (Value::Text(tx.intern(if *b { "TRUE" } else { "FALSE" })), Some(Unit::Bool)),
        DataValue::Date(x) => (Value::Num(*x), Some(Unit::Date)),
        DataValue::Currency(x) => (Value::Num(*x), Some(Unit::Currency)),
    };
    match tx.set_constant(sheet, addr, v, u) {
        // a guarded value stays (Visio leaves a GUARDed cell alone too)
        Err(EditError::Guarded) => Ok(()),
        r => r,
    }
}

// the Value of a shape's Property row as text (for matching and conflicts)
fn field_text(doc: &Document, sheet: SheetRef, name: &str) -> Option<String> {
    let row = doc.shape_data(sheet).into_iter().find(|r| r.name() == Some(name))?;
    Some(value_text(doc, row.get("Value").value(), row.get("Value").cell.and_then(|c| c.unit)))
}

fn value_text(doc: &Document, v: Value, unit: Option<Unit>) -> String {
    match (v, unit) {
        (Value::Num(x), Some(Unit::Date)) => DataValue::Date(x).text(),
        (Value::Num(x), _) => crate::formula::fmt_num(x),
        (Value::Text(s), _) => String::from(doc.str(s)),
        (Value::Absent, _) => String::new(),
    }
}

fn data_linked(doc: &Document, sheet: SheetRef, name: &str) -> bool {
    doc.shape_data(sheet).into_iter().find(|r| r.name() == Some(name)).is_some_and(|r| r.num(DATA_LINKED).unwrap_or(0.0) != 0.0)
}

/// bind a row to a shape on a page: the shape's earlier row of this recordset is replaced, one
/// shape data row per column gets the row's value -> the Value cells written
pub fn link(tx: &mut Transaction, shape: ShapeRef, rs: u32, row: u32) -> Result<Vec<Written>, EditError> {
    let (page_id, shape_id) = tx.doc().data_ids(shape).ok_or(EditError::NoShape)?;
    let set = tx.doc().recordset(rs).ok_or(EditError::Invalid)?.clone();
    let data = set.row(row).ok_or(EditError::Invalid)?.clone();
    tx.atomic(|tx| {
        with_data(tx, |d| {
            let r = d.recordsets.iter_mut().find(|r| r.id == rs).ok_or(EditError::Invalid)?;
            r.links.retain(|l| !(l.page == page_id && l.shape == shape_id));
            r.conflicts.retain(|l| !(l.page == page_id && l.shape == shape_id));
            r.links.push(RowLink { row, page: page_id, shape: shape_id });
            Ok(())
        })?;
        let mut out = Vec::new();
        let sheet = SheetRef::Shape(shape);
        for (c, col) in set.columns.iter().enumerate() {
            let v = data.values.get(c).cloned().unwrap_or_default();
            write_field(tx, sheet, col, &v, &mut out)?;
        }
        Ok(out)
    })
}

/// remove a shape's binding to recordset `rs` (None: to every recordset); the shape data
/// stays with its last values, marked as no longer linked
pub fn unlink(tx: &mut Transaction, shape: ShapeRef, rs: Option<u32>) -> Result<(), EditError> {
    let (page_id, shape_id) = tx.doc().data_ids(shape).ok_or(EditError::NoShape)?;
    let names: Vec<String> = tx
        .doc()
        .data
        .recordsets
        .iter()
        .filter(|r| rs.is_none_or(|id| r.id == id) && r.row_of(page_id, shape_id).is_some())
        .flat_map(|r| r.columns.iter().map(|c| c.name.clone()))
        .collect();
    tx.atomic(|tx| {
        with_data(tx, |d| {
            for r in d.recordsets.iter_mut().filter(|r| rs.is_none_or(|id| r.id == id)) {
                r.links.retain(|l| !(l.page == page_id && l.shape == shape_id));
                r.conflicts.retain(|l| !(l.page == page_id && l.shape == shape_id));
            }
            Ok(())
        })?;
        let sheet = SheetRef::Shape(shape);
        for n in names {
            if data_linked(tx.doc(), sheet, &n) {
                clear_linked(tx, sheet, &n)?;
            }
        }
        Ok(())
    })
}

fn clear_linked(tx: &mut Transaction, sheet: SheetRef, name: &str) -> Result<(), EditError> {
    let sk = SecKey::one(names::PROPERTY);
    let rk = RowKey::Named(tx.intern(name));
    let c = tx.intern(DATA_LINKED);
    tx.set_constant(sheet, &CellAddr::row(sk, rk, c), Value::Num(0.0), None)
}

/// a user changed the value of shape data field `name` (the Shape Data pane, a dialog): when
/// it came from linked data, it is marked as the user's (DataLinked = 0), so that a refresh
/// keeps it and reports it as a conflict instead
pub fn user_edited(tx: &mut Transaction, sheet: SheetRef, name: &str) -> Result<(), EditError> {
    if data_linked(tx.doc(), sheet, name) {
        clear_linked(tx, sheet, name)?;
    }
    Ok(())
}

/// the shape field a column is compared with by auto_link
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShapeField {
    /// the shape's text
    Text,
    /// the shape's name (Name, else NameU)
    Name,
    /// a shape data field by row name (Prop.<name>)
    Data(String),
}

/// what a shape shows for a field (None: the shape has no such field)
pub fn shape_field(doc: &Document, shape: ShapeRef, field: &ShapeField) -> Option<String> {
    let s = doc.shape(shape)?;
    match field {
        ShapeField::Text => Some(crate::field::plain(doc, shape)),
        ShapeField::Name => Some(s.name()),
        ShapeField::Data(n) => field_text(doc, SheetRef::Shape(shape), n),
    }
}

/// AS-04: bind rows to shapes where a column's value equals a shape field (all pairs must
/// match; compared as text, trimmed, ignoring case); shapes already bound to the recordset
/// keep their row unless `replace` -> the bindings made (shape, row) and the Value cells
pub fn auto_link(tx: &mut Transaction, shapes: &[ShapeRef], rs: u32, pairs: &[(usize, ShapeField)], replace: bool) -> Result<(Vec<(ShapeRef, u32)>, Vec<Written>), EditError> {
    if pairs.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let set = tx.doc().recordset(rs).ok_or(EditError::Invalid)?.clone();
    // rows by the joined text of their compared columns (the first row wins on duplicates)
    let mut by_key: BTreeMap<String, u32> = BTreeMap::new();
    for r in set.rows.iter() {
        let k: Vec<String> = pairs.iter().map(|(c, _)| norm(&r.values.get(*c).map(DataValue::text).unwrap_or_default())).collect();
        if k.iter().all(|x| x.is_empty()) {
            continue;
        }
        by_key.entry(k.join("\t")).or_insert(r.id);
    }
    let mut plan = Vec::new();
    for &s in shapes {
        if !replace && tx.doc().data_link(s, rs).is_some() {
            continue;
        }
        let k: Option<Vec<String>> = pairs.iter().map(|(_, f)| shape_field(tx.doc(), s, f).map(|t| norm(&t))).collect();
        let Some(k) = k else { continue };
        if let Some(&row) = by_key.get(&k.join("\t")) {
            plan.push((s, row));
        }
    }
    tx.atomic(|tx| {
        let mut out = Vec::new();
        for &(s, row) in &plan {
            out.extend(link(tx, s, rs, row)?);
        }
        Ok((plan.clone(), out))
    })
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// a conflict a refresh found
#[derive(Clone, PartialEq, Debug)]
pub enum Conflict {
    /// the row a shape is bound to is gone from the source (the binding stays until the user
    /// decides: resolve_unlink or delete the shape)
    RowGone { row: u32, shape: ShapeRef },
    /// several new rows have the key of one bound row; the first one was taken
    DuplicateKey { key: String, rows: u32 },
    /// the user changed a linked value and the source changed it too; the user's value stayed
    UserValue { shape: ShapeRef, column: String, user: String, data: DataValue },
}

/// what refresh did
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Refreshed {
    /// rows that are new
    pub added: usize,
    /// rows whose values changed
    pub changed: usize,
    /// rows that are gone
    pub removed: usize,
    /// shapes whose data was updated
    pub shapes: usize,
    /// what needs the user
    pub conflicts: Vec<Conflict>,
    /// the Value cells written
    pub written: Vec<Written>,
}

/// the key columns of recordset `rs` (their source names) that the source's `headers` lack:
/// refresh refuses such data (ask the user to choose the key again)
pub fn missing_key(doc: &Document, rs: u32, headers: &[String]) -> Vec<String> {
    let Some(set) = doc.recordset(rs) else { return Vec::new() };
    set.key.iter().filter(|k| !headers.iter().any(|h| h.trim() == k.column)).map(|k| k.column.clone()).collect()
}

/// DA-05: new data for recordset `rs` (the source's columns by name and its rows in source
/// order).  Rows keep their IDs while their key matches (without key: their position); new
/// rows get new IDs; columns keep their Name, label and type when the source still has them,
/// new ones are added.  Every bound shape gets the new values -- a value the user changed
/// (DataLinked = 0) only when the recordset overwrites all, else it stays and is reported.
/// Shapes whose row is gone stay bound and are reported (and kept as RefreshConflict).
/// `now` is the time to note ("2026-11-23T14:05:00").  Invalid, with nothing changed, when the
/// source lacks a key column (missing_key): matched by position instead, each bound shape got
/// another row's data without a word
pub fn refresh(tx: &mut Transaction, rs: u32, headers: &[String], rows: Vec<Vec<DataValue>>, now: &str) -> Result<Refreshed, EditError> {
    let old = tx.doc().recordset(rs).ok_or(EditError::Invalid)?.clone();
    if !missing_key(tx.doc(), rs, headers).is_empty() {
        return Err(EditError::Invalid);
    }
    let mut new = old.clone();
    // columns: kept by source name, new ones appended (types guessed), gone ones dropped
    let mut columns: Vec<Column> = Vec::new();
    for (i, h) in headers.iter().enumerate() {
        let h = h.trim();
        match old.columns.iter().find(|c| c.id == h) {
            Some(c) => columns.push(c.clone()),
            None => {
                let data_type = guess_kind(rows.iter().filter_map(|r| r.get(i)));
                let name = unique(&row_name(h), |c| old.columns.iter().chain(columns.iter()).any(|x| x.name.eq_ignore_ascii_case(c)));
                columns.push(Column { id: String::from(h), name, label: String::from(h), data_type, ..Column::default() });
            }
        }
    }
    new.columns = columns;
    new.row_order = new.key.is_empty();
    let n = new.columns.len();
    // old rows by key
    let old_key = |r: &DataRow| -> String {
        if old.key.is_empty() {
            return format!("{}", r.id);
        }
        old.key_of(r)
    };
    let mut old_by_key: BTreeMap<String, &DataRow> = BTreeMap::new();
    for r in old.rows.iter() {
        old_by_key.entry(old_key(r)).or_insert(r);
    }
    let mut res = Refreshed::default();
    let mut next = old.next_row.max(old.rows.iter().map(|r| r.id + 1).max().unwrap_or(1));
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    let mut out_rows = Vec::with_capacity(rows.len());
    for (pos, mut values) in rows.into_iter().enumerate() {
        values.resize(n, DataValue::Empty);
        let probe = DataRow { id: pos as u32 + 1, values };
        let key = if new.key.is_empty() { format!("{}", probe.id) } else { new.key_of(&probe) };
        let id = match (seen.get(&key), old_by_key.get(&key)) {
            (Some(_), _) => {
                // the same key again: a new row of its own, reported once per key
                if !res.conflicts.iter().any(|c| matches!(c, Conflict::DuplicateKey { key: k, .. } if *k == key)) {
                    res.conflicts.push(Conflict::DuplicateKey { key: key.clone(), rows: 2 });
                } else if let Some(Conflict::DuplicateKey { rows, .. }) = res.conflicts.iter_mut().find(|c| matches!(c, Conflict::DuplicateKey { key: k, .. } if *k == key)) {
                    *rows += 1;
                }
                res.added += 1;
                next += 1;
                next - 1
            }
            (None, Some(o)) => {
                if !same_values(&old, o, &new, &probe) {
                    res.changed += 1;
                }
                o.id
            }
            (None, None) => {
                res.added += 1;
                if new.key.is_empty() {
                    next = next.max(probe.id + 1);
                    probe.id
                } else {
                    next += 1;
                    next - 1
                }
            }
        };
        seen.entry(key).or_insert(id);
        out_rows.push(DataRow { id, values: probe.values });
    }
    res.removed = old.rows.iter().filter(|r| !out_rows.iter().any(|n| n.id == r.id)).count();
    new.next_row = next.max(out_rows.iter().map(|r| r.id + 1).max().unwrap_or(1));
    new.rows = Arc::new(out_rows);
    new.update_key_values();
    new.time_refreshed = Some(String::from(now));
    // Visio's checksum follows the columns (it stays when Visio refreshes the same columns):
    // other columns, and Visio computes it anew
    if new.columns.iter().map(|c| &c.id).ne(old.columns.iter().map(|c| &c.id)) {
        new.checksum = None;
    }
    new.conflicts.clear();
    // bound shapes
    let doc = tx.doc();
    let mut updates: Vec<(ShapeRef, u32)> = Vec::new();
    for l in &new.links {
        let Some(shape) = doc.data_shape(l.page, l.shape) else { continue };
        if new.row(l.row).is_some() {
            updates.push((shape, l.row));
        } else {
            res.conflicts.push(Conflict::RowGone { row: l.row, shape });
            new.conflicts.push(*l);
        }
    }
    // links of shapes that no longer exist go
    new.links.retain(|l| doc.data_shape(l.page, l.shape).is_some());
    tx.atomic(|tx| {
        set_recordset(tx, new.clone())?;
        for &(shape, row) in &updates {
            let sheet = SheetRef::Shape(shape);
            let Some(data) = new.row(row) else { continue };
            let mut touched = false;
            for (c, col) in new.columns.iter().enumerate() {
                let v = data.values.get(c).cloned().unwrap_or_default();
                let has = tx.doc().shape_data(sheet).into_iter().any(|r| r.name() == Some(col.name.as_str()));
                if has && !new.overwrite_all && !data_linked(tx.doc(), sheet, &col.name) {
                    let user = field_text(tx.doc(), sheet, &col.name).unwrap_or_default();
                    if user != v.text() {
                        res.conflicts.push(Conflict::UserValue { shape, column: col.name.clone(), user, data: v });
                    }
                    continue;
                }
                let before = field_text(tx.doc(), sheet, &col.name);
                if before.as_deref() != Some(v.text().as_str()) || !has {
                    touched = true;
                }
                write_field(tx, sheet, col, &v, &mut res.written)?;
            }
            if touched {
                res.shapes += 1;
            }
        }
        Ok(())
    })?;
    Ok(res)
}

fn same_values(a: &Recordset, ra: &DataRow, b: &Recordset, rb: &DataRow) -> bool {
    b.columns.iter().enumerate().all(|(i, c)| {
        let old = a.column_by_id(&c.id).and_then(|j| ra.values.get(j)).cloned().unwrap_or_default();
        rb.values.get(i).cloned().unwrap_or_default() == old
    })
}

/// a conflict resolved by taking the source's value over the user's (DataLinked = 1 again)
pub fn resolve_take_data(tx: &mut Transaction, shape: ShapeRef, rs: u32, column: &str) -> Result<Vec<Written>, EditError> {
    let set = tx.doc().recordset(rs).ok_or(EditError::Invalid)?.clone();
    let row = tx.doc().data_link(shape, rs).and_then(|r| set.row(r)).ok_or(EditError::Invalid)?.clone();
    let c = set.column_by_name(column).ok_or(EditError::Invalid)?;
    let mut out = Vec::new();
    write_field(tx, SheetRef::Shape(shape), &set.columns[c], &row.values.get(c).cloned().unwrap_or_default(), &mut out)?;
    Ok(out)
}

/// a conflict resolved by keeping the shape without its row: the binding goes
pub fn resolve_unlink(tx: &mut Transaction, shape: ShapeRef, rs: u32) -> Result<(), EditError> {
    unlink(tx, shape, Some(rs))
}

#[cfg(test)]
mod tests;
