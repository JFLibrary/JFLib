// data_source -- (JD18, IM-05, IM-10) a data source read into rows: an Excel workbook (.xlsx,
// .xlsm) or a text file (.csv, .txt) through the library's sheet_io; the import (data_import.rs)
// and every refresh (data_refresh.rs) read through here
//
//   read(source) -> Result<Loaded, ReadError>   the headers and typed rows of a sheet, a range
//                             ("Tabelle1!A1:C9"), a named range or table, or a text file
//                             (diagram_core::data::Source, from the recordset's connection)
//   read_bytes(path)          the file's bytes: Denied when this run may not read it (fs.user
//                             grants the files the user chose in this run), Missing when it is
//                             not there
//   workbook(bytes) / text(bytes)   the parsed file for the data selector's pages
//   loaded(table)             a sheet_io::Table as headers and DataValues
//   recordset_columns(data, rs) / read_columns(source, cols)   what a refresh reads: the
//                             recordset's source (or the file it moved to) with only the
//                             columns its Command names ("select `A`,`B` from ..."; the data
//                             selector writes such a list when columns were left out, as Visio
//                             does, so that Visio's refresh leaves them out too)
//   command_columns(cmd) / command_with_columns(cmd, cols) / keep_columns(loaded, cols)   the
//                             column list of a Command read and written, a table cut to it
//
// Values: numbers, texts, Booleans and dates keep their type (a date is a day number since
// 1899-12-30, as Visio keeps it); an error value of a cell becomes its text ("#N/A").  A text
// file's values are typed by sheet_io's inference (numbers, TRUE/FALSE/WAHR/FALSCH, ISO and
// d.m.yyyy dates); with ';' as delimiter German numbers ("1.234,5") count too.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::data::{DataValue, ExcelTable, Source};
use sheet_io::{csv, xlsx, Table, Value};

/// what a source gave: the column names and the rows (one value per column)
pub(crate) struct Loaded {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<DataValue>>,
}

/// why a source could not be read
pub(crate) enum ReadError {
    /// the file may not be read in this run (fs.user grants only files the user chose): ask the
    /// user to choose it again
    Denied,
    /// the file is not there
    Missing,
    /// the file is no workbook / text file, or the sheet or range is not in it (the message,
    /// English from sheet_io or German from here; shown after "Die Datenquelle ist nicht
    /// lesbar:")
    Invalid(String),
}

/// read a source
pub(crate) fn read(source: &Source) -> Result<Loaded, ReadError> {
    match source {
        Source::Excel { path, table, header } => {
            let bytes = read_bytes(path)?;
            let wb = workbook(&bytes)?;
            let t = excel_table(&wb, table, *header)?;
            Ok(loaded(t))
        }
        Source::Text { path, header } => {
            let bytes = read_bytes(path)?;
            let c = text(&bytes)?;
            Ok(loaded(csv::to_table(&c, *header, true)))
        }
        Source::Other => Err(ReadError::Invalid(String::from(ui::i18n::t("keine Excel-Arbeitsmappe und keine Textdatei")))),
    }
}

/// the bytes of a file the run may read
pub(crate) fn read_bytes(path: &str) -> Result<Vec<u8>, ReadError> {
    if !plat::policy::fs_allowed(path, plat::policy::FS_READ) {
        return Err(ReadError::Denied);
    }
    plat::read_file(path).ok_or(ReadError::Missing)
}

/// a workbook (the sheets and names, not their cells yet)
pub(crate) fn workbook(bytes: &[u8]) -> Result<xlsx::Workbook, ReadError> {
    xlsx::Workbook::open(bytes, xlsx::Limits::DEFAULT).map_err(|e| ReadError::Invalid(e.text()))
}

/// a text file's records (encoding and delimiter detected)
pub(crate) fn text(bytes: &[u8]) -> Result<csv::Csv, ReadError> {
    csv::read(bytes, &csv::Options::DEFAULT).map_err(|e| ReadError::Invalid(e.text()))
}

/// the table a connection names in a workbook: a sheet's used range, a range of a sheet, a
/// named range or table
pub(crate) fn excel_table(wb: &xlsx::Workbook, table: &ExcelTable, header: bool) -> Result<Table, ReadError> {
    let missing = |what: &str| ReadError::Invalid(ui::i18n::tf("„{}“ ist nicht in der Arbeitsmappe", &[what]));
    let t = match table {
        ExcelTable::Sheet(name) => {
            let i = wb.sheet_by_name(name).ok_or_else(|| missing(name))?;
            wb.sheet_table(i, header)
        }
        ExcelTable::Range(sheet, range) => {
            let area = wb.area(&alloc::format!("{}!{range}", sheet_io::quote_sheet(sheet))).ok_or_else(|| missing(&alloc::format!("{sheet}!{range}")))?;
            wb.table(&area, header)
        }
        ExcelTable::Name(name) => {
            let area = wb.names().iter().find(|n| n.name.eq_ignore_ascii_case(name) && !n.builtin()).and_then(|n| n.area.clone()).ok_or_else(|| missing(name))?;
            wb.table(&area, header)
        }
    };
    t.map_err(|e| ReadError::Invalid(e.text()))
}

/// a sheet_io table as the recordset's headers and values
pub(crate) fn loaded(t: Table) -> Loaded {
    let rows = t.rows.into_iter().map(|r| r.into_iter().map(value).collect()).collect();
    Loaded { headers: t.columns, rows }
}

/// the columns a recordset's Command names (None: all, "select *")
pub(crate) fn recordset_columns(data: &diagram_core::data::DataSets, rs: &diagram_core::data::Recordset) -> Option<Vec<String>> {
    let conn = rs.connection.and_then(|id| data.connections.iter().find(|c| c.id == id));
    let cmd = rs.command.as_deref().or(conn.and_then(|c| c.command.as_deref())).unwrap_or("");
    command_columns(cmd)
}

/// read a source (also one moved to another file) and keep only `cols` when given
pub(crate) fn read_columns(source: &Source, cols: &Option<Vec<String>>) -> Result<Loaded, ReadError> {
    let l = read(source)?;
    Ok(match cols {
        Some(c) => keep_columns(l, c),
        None => l,
    })
}

/// the columns a Command selects ("select `A`,`B` from `X$`", also [A] or plain names); None
/// for "select * ..." or anything that is no such list
pub(crate) fn command_columns(cmd: &str) -> Option<Vec<String>> {
    let lower = cmd.to_ascii_lowercase();
    let lead = lower.len() - lower.trim_start().len();
    if !lower[lead..].starts_with("select ") {
        return None;
    }
    let end = lower.find(" from ")?;
    let list = cmd.get(lead + 7..end)?.trim();
    if list.is_empty() || list == "*" {
        return None;
    }
    let mut out = Vec::new();
    let mut rest = list;
    while !rest.is_empty() {
        let r = rest.trim_start();
        let (name, after) = match r.chars().next()? {
            '`' => {
                let e = r[1..].find('`')? + 1;
                (&r[1..e], &r[e + 1..])
            }
            '[' => {
                let e = r.find(']')?;
                (&r[1..e], &r[e + 1..])
            }
            _ => match r.find(',') {
                Some(e) => (r[..e].trim(), &r[e..]),
                None => (r.trim(), ""),
            },
        };
        out.push(String::from(name));
        let after = after.trim_start();
        rest = match after.strip_prefix(',') {
            Some(a) => a,
            None if after.is_empty() => "",
            None => return None,
        };
    }
    (!out.is_empty()).then_some(out)
}

/// a Command ("select * from `X$`") with a column list instead of "*"; a name with '`' cannot
/// be written in it: the Command stays as it is
pub(crate) fn command_with_columns(cmd: &str, cols: &[String]) -> String {
    let lower = cmd.to_ascii_lowercase();
    let Some(at) = lower.find(" from ") else { return String::from(cmd) };
    if cols.is_empty() || cols.iter().any(|c| c.contains('`')) || !lower.trim_start().starts_with("select ") {
        return String::from(cmd);
    }
    let list: Vec<String> = cols.iter().map(|c| alloc::format!("`{c}`")).collect();
    alloc::format!("select {}{}", list.join(","), &cmd[at..])
}

/// a table cut to these columns (in this order; names compared without case, as the OLE DB
/// provider does; a column the table lacks is left out)
pub(crate) fn keep_columns(l: Loaded, cols: &[String]) -> Loaded {
    let idx: Vec<usize> = cols.iter().filter_map(|c| l.headers.iter().position(|h| h.to_lowercase() == c.to_lowercase())).collect();
    let headers = idx.iter().map(|&i| l.headers[i].clone()).collect();
    let rows = l.rows.into_iter().map(|r| idx.iter().map(|&i| r.get(i).cloned().unwrap_or_default()).collect()).collect();
    Loaded { headers, rows }
}

/// a cell value as a recordset value
pub(crate) fn value(v: Value) -> DataValue {
    match v {
        Value::Empty => DataValue::Empty,
        Value::Number(x) => DataValue::Num(x),
        Value::Text(s) => DataValue::Text(s),
        Value::Bool(b) => DataValue::Bool(b),
        Value::Date(d) => DataValue::Date(d),
        Value::Error(e) => DataValue::Text(String::from(e.text())),
    }
}
