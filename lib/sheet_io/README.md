# jflib-sheet-io

Spreadsheet import for [JFLib](https://jflib.org): Excel workbooks (`.xlsx`, `.xlsm`, `.xltx`,
`.xltm`, transitional and strict Office Open XML) and CSV files, both delivered as the same typed
`Table`. It reads the values a file stores, the cached result of a formula included, and never
evaluates a formula. It is plain Rust, without the standard library and without a line of
`unsafe` code.

Why a crate of its own: JFLib keeps one crate per foreign format family, like `dxf`, `gltf` and
`pdf`. jfWord's `doc_io` would pull its document model, layout and text into every user of a
spreadsheet reader; `archive` holds containers, not their content; `base::encoding` is strict
Unicode by design, while CSV files from Excel come in Windows-1252. CSV sits next to XLSX because
both deliver the same table.

## How it is used

```rust
use jflib::sheet_io::{csv, xlsx, Value};

// a workbook: the sheets and names first, the cells of a sheet when asked for
let wb = xlsx::Workbook::open(&bytes, xlsx::Limits::default())?;
for (i, s) in wb.sheets().iter().enumerate() {
    // s.name, s.state (Visible, Hidden, VeryHidden), s.kind (Worksheet, Chartsheet, Other)
}
let first = wb.sheet_table(0, true)?;                  // the used range, first row as names
let area = wb.area("'Tab 1'!$A$1:$C$10").unwrap();     // or a defined name's area
let table = wb.table(&area, true)?;
for name in wb.names() {
    // name.name, name.scope, name.formula, name.area (one rectangle), name.table (a ListObject)
}
let grid = wb.sheet(0)?;                               // sparse: get(row, col), rows(), used()
if let Value::Date(serial) = grid.get(1, 2) {
    let when = jflib::sheet_io::civil(*serial);        // an OLE Automation date
}

// a CSV file: encoding and delimiter detected, Excel's "sep=;" honoured
let file = csv::read(&bytes, &csv::Options::default())?;
let table = csv::to_table(&file, true, true);          // header row, numbers/booleans/dates typed
```

Values are `Empty`, `Number`, `Text`, `Bool`, `Date` (an OLE Automation date: days since
1899-12-30, as Visio uses them; Excel's 1904 date system and its fake 1900-02-29 are converted)
and `Error` (`#N/A`, `#DIV/0!` ...). `Value::text` gives the display form, with dates in ISO 8601.
Column names of a table are trimmed; an empty one is `F<n>` as Excel's OLE DB driver names it, a
repeated one gets a number (`Name1`).

What the XLSX reader knows: shared strings with rich runs (phonetic runs left out), `xml:space`,
Excel's `_xHHHH_` escapes, inline strings, booleans, errors, ISO dates (`t="d"`), dates by
built-in and custom number formats (`[h]:mm` is an elapsed time), rows and cells without
positions or out of order, defined names global and sheet-local (Excel's `_xlnm.*` included), the
tables (ListObjects) of the worksheets, hidden sheets and chartsheets. The workbook part is found
through the package relationship and the sheets through the workbook's relationships, never by
guessing paths. Macros of an `.xlsm` are never read. A binary `.xlsb` or an old `.xls` is
`Error::NotXlsx`.

What the CSV reader knows: RFC 4180 and what Excel writes -- quoted fields with `""`, delimiters
and line breaks inside quotes, CR LF, LF and CR records, a last line with or without a line break,
Excel's `sep=` line, UTF-8 with and without BOM, UTF-16 (Excel's "Unicode text"), Windows-1252.
The delimiter is detected among TAB, `;`, `,` and `|` (`csv::detect_delimiter` says how). It is
lenient where Excel is: a quote inside an unquoted field is kept, text after a closing quote too.
German numbers (`1.234,5`) are read when `;` is the delimiter.

## Files are untrusted input

`xlsx::Limits` bounds the ZIP container before anything is inflated (parts, sizes, the
compression ratio), the nesting of the XML, the rows, columns and cells of a sheet, the shared
strings and their bytes, and the text one sheet may hold (one long shared string in a million
cells is refused, not copied). The XML reader takes no DTD and no entity but the five predefined
ones, so no entity expansion of any kind. No size a file advertises (`<dimension>`, `count`,
`uniqueCount`) sizes an allocation; cells are stored sparsely, so a cell at `A1` and one at
`XFD1048576` are two cells, and a table of an area is refused when it would hold more than
`Limits::cells` values. `csv::CsvLimits` bounds the bytes, records, fields and the bytes of a
field. Nothing recurses; a broken file is an error, never a panic.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::sheet_io`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with the next release. The
[changelog](https://docs.rs/crate/jflib-sheet-io/latest/source/CHANGELOG.md) will say so.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#sheet_io): every
  public item of `sheet_io`, also on [docs.rs](https://docs.rs/jflib-sheet-io)

## License

MIT or Apache-2.0, at your choice.
