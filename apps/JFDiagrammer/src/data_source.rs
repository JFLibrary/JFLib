// data_source -- (JD18) a data source read into rows: an Excel workbook (.xlsx, .xlsm) or a text
// file (.csv, .txt) through sheet_io; the import (data_import.rs) and every refresh
// (data_refresh.rs) read through here
//
//   read(source) -> Result<Loaded, ReadError>   the headers and typed rows of a sheet, a range,
//                             a named range or a text file (diagram_core::data::Source)
//
// STUB (skeleton): the import agent fills it in.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::data::{DataValue, Source};

/// what a source gave: the column names and the rows (one value per column)
#[allow(dead_code)]
pub(crate) struct Loaded {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<DataValue>>,
}

/// why a source could not be read
#[allow(dead_code)]
pub(crate) enum ReadError {
    /// the file may not be read in this run (fs.user grants only files the user chose): ask the
    /// user to choose it again
    Denied,
    /// the file is not there
    Missing,
    /// the file is no workbook / text file, or the sheet or range is not in it (the message)
    Invalid(String),
}

/// read a source
#[allow(dead_code)]
pub(crate) fn read(source: &Source) -> Result<Loaded, ReadError> {
    let _ = source;
    Err(ReadError::Invalid(String::new()))
}
