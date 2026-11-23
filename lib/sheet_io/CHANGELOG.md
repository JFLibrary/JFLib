# Changelog of jflib-sheet-io

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- Spreadsheet import for JFDiagrammer's data linking (JD18; FEATURES.md IM-05, IM-10, decision F6). `xlsx::Workbook` reads Office Open XML workbooks (.xlsx, .xlsm without running or reading the macros, .xltx, .xltm; transitional and strict): the workbook part through the package's officeDocument relationship and the sheets through the workbook's relationships, sheets with state and kind (worksheet, chartsheet, other), defined names global and sheet-local with their area when they are one rectangle (built-in `_xlnm.*` names marked), the tables (ListObjects) of the worksheets as names, the 1900 and 1904 date systems. `sheet` gives one worksheet as a sparse `Grid` of the stored values (a formula's cached result; formulas are never evaluated): shared strings with rich runs, phonetic runs left out, `xml:space`, `_xHHHH_` escapes, inline strings, numbers, booleans, errors, ISO dates, dates and times by built-in and custom number formats (`[h]:mm` as elapsed time), rows and cells without positions or out of order (a cell given twice: the last wins). `table` / `sheet_table` give a rectangle or the used range as a `Table` with column names (trimmed, `F<n>` for empty ones, numbered duplicates). `csv::read` reads RFC 4180 and what Excel writes (quoted delimiters and line breaks, CR LF / LF / CR, `sep=`, lenient quotes), detects the encoding (BOM, UTF-16, UTF-8, else Windows-1252) and the delimiter (TAB, `;`, `,`, `|`); `csv::to_table` types numbers (German ones with `;`), booleans (TRUE/FALSE/WAHR/FALSCH) and dates (ISO, d.m.yyyy). Common types `Value` (dates as OLE Automation dates), `CellError`, `CellRef`, `Area`, `Table`, `parse_ref`, `parse_area`, `col_name`, `quote_sheet`, `civil`, `serial`. Files are untrusted input: `xlsx::Limits` (ZIP limits with the compression ratio checked before inflating, rows, columns, cells, shared strings and their bytes, text per sheet, XML nesting) and `csv::CsvLimits` (bytes, records, fields, field bytes); no DTD, no entity expansion, no allocation sized by what a file advertises, sparse cells, no recursion. apps/codectest --sheet checks hostile workbooks and texts.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
