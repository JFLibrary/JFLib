// the unit tests of the writers (cargo test -p jflib-sheet-io --lib): workbooks and CSV files
// written by xlsx::write and csv::write, read back with the crate's own readers; Excel's limits
// as errors; the same input gives the same bytes

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::opc::{Package, Rules};

use crate::csv::{self, Encoding, LineEnd, WriteOptions};
use crate::xlsx::{self, BookOut, Limits, SheetOut, WriteError, WriteOptions as XlsxOptions, Workbook, MAX_TEXT};
use crate::{serial, CellError, CellRef, Civil, Table, Value, MAX_COLS, MAX_ROWS};

fn text(s: &str) -> Value {
    Value::Text(String::from(s))
}

fn day(y: i32, m: u8, d: u8) -> f64 {
    serial(&Civil::date(y, m, d))
}

fn one(rows: Vec<Vec<Value>>) -> BookOut {
    BookOut { sheets: vec![SheetOut::new("Daten", rows)] }
}

fn written(book: &BookOut) -> Vec<u8> {
    xlsx::write(book, &XlsxOptions::DEFAULT).expect("write")
}

fn reopen(bytes: &[u8]) -> Workbook {
    Workbook::open(bytes, Limits::default()).expect("open")
}

fn package(bytes: &[u8]) -> Package {
    Package::read(bytes, archive::Limits::default(), Rules::DEFAULT).expect("package")
}

fn part(bytes: &[u8], name: &str) -> String {
    String::from_utf8(package(bytes).part(name).expect(name).to_vec()).expect("utf-8")
}

// the positions of `tags` in `xml` ascend (the schema's order of children)
fn in_order(xml: &str, tags: &[&str]) -> bool {
    let at: Vec<Option<usize>> = tags.iter().map(|t| xml.find(t)).collect();
    at.iter().all(Option::is_some) && at.windows(2).all(|w| w[0] < w[1])
}

// what the reader gives back for a value written (see xlsx_write's head)
fn back(v: &Value) -> Value {
    match v {
        Value::Number(n) | Value::Date(n) if !n.is_finite() => Value::Error(CellError::Num),
        Value::Text(s) if s.is_empty() => Value::Empty,
        Value::Error(e @ (CellError::Spill | CellError::Calc | CellError::GettingData | CellError::Other(_))) => text(e.text()),
        other => other.clone(),
    }
}

// ------------------------------------------------------------------ XLSX

#[test]
fn xlsx_write_values() {
    let long = "x".repeat(MAX_TEXT);
    let astral = "\u{1F600}".repeat(MAX_TEXT / 2);
    let rows = vec![
        vec![Value::Number(1.5), Value::Number(-0.0), Value::Number(1e300), Value::Number(-1e-300), Value::Number(0.1 + 0.2), Value::Number(123_456_789_012_345_680.0), Value::Number(f64::NAN), Value::Number(f64::NEG_INFINITY)],
        vec![text("Größe 日本語 \u{1F600}"), text("a\u{1}b\u{8}c\u{b}d\u{c}e\u{1f}f\u{0}"), text("Zeile 1\r\nZeile 2\nZeile 3\r"), text("\tTab\t"), text("\u{fffe}\u{ffff}"), text("  zwei Leerzeichen  "), text("&<>\"' ]]>"), text("")],
        vec![text("_x0041_"), text("_x005F_"), text("_x1234\u{1}"), text("a_x00e4_b_X0041_c_x12"), text("__x0041__x0042_"), text(&long), text(&astral), text("_x00D_ _xABCD")],
        vec![Value::Bool(true), Value::Bool(false), Value::Error(CellError::Null), Value::Error(CellError::Div0), Value::Error(CellError::Value), Value::Error(CellError::Ref), Value::Error(CellError::Name), Value::Error(CellError::Num), Value::Error(CellError::NA)],
        vec![Value::Error(CellError::Spill), Value::Error(CellError::Calc), Value::Error(CellError::GettingData), Value::Error(CellError::Other(String::from("#BUSY!"))), Value::Empty, Value::Empty, Value::Number(7.0), text("#BUSY!")],
        vec![
            Value::Date(day(2026, 11, 24)),
            Value::Date(serial(&Civil::new(2026, 11, 24, 14, 5, 30))),
            Value::Date(0.5),
            Value::Date(0.0),
            Value::Date(day(1900, 1, 1)),
            Value::Date(day(1900, 2, 28) + 0.25),
            Value::Date(day(1900, 3, 1)),
            Value::Date(serial(&Civil::new(9999, 12, 31, 23, 59, 59))),
            Value::Date(f64::INFINITY),
        ],
    ];
    let z = written(&one(rows.clone()));
    let g = reopen(&z).sheet(0).unwrap();
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            assert_eq!(*g.get(r as u32, c as u32), back(v), "{}", CellRef::new(r as u32, c as u32).a1());
        }
    }
    // dates Excel cannot show: their text
    let early = vec![vec![Value::Date(day(1899, 12, 31) + 0.5), Value::Date(day(1800, 1, 1)), Value::Date(day(10_000, 1, 1)), Value::Date(-0.25)]];
    let g = reopen(&written(&one(early))).sheet(0).unwrap();
    let texts: Vec<Value> = g.row(0).iter().map(|(_, v)| v.clone()).collect();
    assert_eq!(texts, [text("1899-12-31 12:00:00"), text("1800-01-01"), text("10000-01-01"), text("1899-12-29 18:00:00")]);
    // what the parts say: Excel's escapes, kept white space, the date styles, error cells
    let sst = part(&z, "xl/sharedStrings.xml");
    for want in ["_x0001_", "_x0000_", "_x000D_&#10;", "_x005F_x0041_", "_x005F_x005F_", "_x005F_x1234_x0001_", "_x005F_x00e4_b_X0041_c_x12", "__x005F_x0041__x005F_x0042_", "<t xml:space=\"preserve\">&#9;Tab&#9;</t>", "&amp;&lt;&gt;&quot;&apos; ]]&gt;", "_xFFFE__xFFFF_"] {
        assert!(sst.contains(want), "{want}");
    }
    // 20 text cells, the error texts among them, "" none; the same text twice is one entry
    assert!(sst.contains("count=\"20\" uniqueCount=\"19\""));
    let sheet = part(&z, "xl/worksheets/sheet1.xml");
    for want in ["<c r=\"A6\" s=\"1\"><v>46350</v>", "<c r=\"B6\" s=\"2\">", "<c r=\"C6\" s=\"3\"><v>0.5</v>", "<c r=\"E6\" s=\"1\"><v>1</v>", "<c r=\"G6\" s=\"1\"><v>61</v>", "<c r=\"D4\" t=\"e\"><v>#DIV/0!</v>", "<c r=\"A4\" t=\"b\"><v>1</v>", "<c r=\"G1\" t=\"e\"><v>#NUM!</v>", "<c r=\"B1\"><v>0</v>"] {
        assert!(sheet.contains(want), "{want}");
    }
    assert!(!sheet.contains("r=\"H2\""), "an empty text is no cell");
}

#[test]
fn xlsx_write_layout() {
    let d = day(2026, 11, 24);
    let table = Table {
        columns: vec![String::from("Name"), String::from("Wert"), String::from("Datum"), String::from("Ja")],
        rows: vec![
            vec![text("Router"), Value::Number(1.5), Value::Date(d), Value::Bool(true)],
            vec![text("Switch mit sehr langem Namen"), Value::Empty, Value::Empty, Value::Bool(false)],
            vec![Value::Empty, Value::Number(3.0), Value::Empty, Value::Empty],
        ],
    };
    let mut s = SheetOut::from_table("Bericht", &table);
    s.autofilter = true;
    s.widths = vec![None, Some(20.0), Some(0.0)];
    let z = written(&BookOut { sheets: vec![s] });
    let wb = reopen(&z);
    assert_eq!(wb.sheet_table(0, true).unwrap(), table);
    let filter = &wb.names()[0];
    assert!(filter.builtin() && filter.hidden && filter.name == "_xlnm._FilterDatabase" && filter.scope == Some(0));
    assert_eq!(filter.formula, "'Bericht'!$A$1:$D$4");
    assert_eq!(filter.area.map(|a| a.a1()), Some(String::from("A1:D4")));
    let x = part(&z, "xl/worksheets/sheet1.xml");
    for want in [
        "<dimension ref=\"A1:D4\"/>",
        "<sheetView tabSelected=\"1\" workbookViewId=\"0\"><pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/>",
        "<c r=\"A1\" s=\"4\" t=\"s\">",
        "<c r=\"C2\" s=\"1\">",
        "<col min=\"1\" max=\"1\" width=\"29.5\" customWidth=\"1\"/>",
        "<col min=\"2\" max=\"2\" width=\"20\" customWidth=\"1\"/>",
        "<col min=\"3\" max=\"3\" width=\"0\" hidden=\"1\" customWidth=\"1\"/>",
        "<col min=\"4\" max=\"4\" width=\"8.43\" customWidth=\"1\"/>",
        "<autoFilter ref=\"A1:D4\"/>",
    ] {
        assert!(x.contains(want), "{want}");
    }
    assert!(in_order(&x, &["<dimension", "<sheetViews", "<sheetFormatPr", "<cols", "<sheetData", "<autoFilter", "<pageMargins"]));
    // estimated widths: wide characters twice, the longest line, a number's General length,
    // a date and time, a long text capped at 80; equal neighbours in one <col>
    let rows = vec![
        vec![text("日本語テキスト"), text("ab\ncdefghijklmn\nij"), Value::Number(0.1 + 0.2), Value::Date(d + 0.5), text(&"y".repeat(300)), Value::Bool(true), Value::Bool(false)],
        vec![],
        vec![Value::Empty, Value::Empty, Value::Empty, Value::Empty, Value::Empty, Value::Empty, Value::Empty, Value::Number(1.0)],
    ];
    let mut s = SheetOut::new("Breiten", rows);
    s.header_rows = 2;
    let z = written(&BookOut { sheets: vec![s] });
    let x = part(&z, "xl/worksheets/sheet1.xml");
    for want in [
        "<col min=\"1\" max=\"1\" width=\"16.9\" customWidth=\"1\"/>",
        "<col min=\"2\" max=\"2\" width=\"14.7\" customWidth=\"1\"/>",
        "<col min=\"3\" max=\"3\" width=\"13.6\" customWidth=\"1\"/>",
        "<col min=\"4\" max=\"4\" width=\"19.1\" customWidth=\"1\"/>",
        "<col min=\"5\" max=\"5\" width=\"80\" customWidth=\"1\"/>",
        "<col min=\"6\" max=\"8\" width=\"8.43\" customWidth=\"1\"/>",
        "<pane ySplit=\"2\" topLeftCell=\"A3\"",
        "<row r=\"1\"><c r=\"A1\" s=\"4\" t=\"s\">",
        "<c r=\"D1\" s=\"6\">",
        "<row r=\"3\"><c r=\"H3\"><v>1</v>",
    ] {
        assert!(x.contains(want), "{want}");
    }
    assert!(!x.contains("<row r=\"2\""), "a row without values is not written");
    // ragged rows and gaps, read back as they were
    let rows = vec![vec![text("a")], vec![], vec![Value::Empty, Value::Empty, Value::Number(1.0)], vec![text("b"), Value::Empty]];
    let g = reopen(&written(&one(rows))).sheet(0).unwrap();
    assert_eq!(g.cells(), 3);
    assert_eq!((g.get(0, 0), g.get(2, 2), g.get(3, 0)), (&text("a"), &Value::Number(1.0), &text("b")));
    assert_eq!(g.used().map(|a| a.a1()), Some(String::from("A1:C4")));
    // an empty sheet
    let z = written(&one(Vec::new()));
    assert!(reopen(&z).sheet(0).unwrap().is_empty());
    let x = part(&z, "xl/worksheets/sheet1.xml");
    assert!(x.contains("<dimension ref=\"A1\"/>") && x.contains("<sheetData/>") && !x.contains("<cols>"));
    assert!(package(&z).part("xl/sharedStrings.xml").is_none(), "no texts, no shared strings part");
}

#[test]
fn xlsx_write_sheets() {
    let mut names: Vec<String> = (1..=40).map(|i| alloc::format!("Blatt {i}")).collect();
    names.extend(["O'Brien", "Größe & Maß", "日本", "<\"Zitat\">", "History ", "x".repeat(31).as_str(), "\u{1F600}".repeat(15).as_str(), "Ä".repeat(31).as_str()].map(String::from));
    let sheets: Vec<SheetOut> = names.iter().enumerate().map(|(i, n)| SheetOut::new(n, vec![vec![Value::Number(i as f64), text(n)]])).collect();
    let z = written(&BookOut { sheets });
    let wb = reopen(&z);
    let read: Vec<&str> = wb.sheets().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(read, names.iter().map(String::as_str).collect::<Vec<_>>());
    for (i, n) in names.iter().enumerate() {
        let g = wb.sheet(i).unwrap();
        assert_eq!((g.get(0, 0), g.get(0, 1)), (&Value::Number(i as f64), &text(n)));
    }
    // only the first tab is selected (two would be grouped)
    assert!(part(&z, "xl/worksheets/sheet1.xml").contains("tabSelected=\"1\""));
    assert!(!part(&z, "xl/worksheets/sheet2.xml").contains("tabSelected"));
    assert_eq!(wb.sheet_by_name("o'brien"), Some(40));
}

#[test]
fn xlsx_write_limits() {
    let err = |b: &BookOut| xlsx::write(b, &XlsxOptions::DEFAULT).err();
    assert_eq!(err(&BookOut::default()), Some(WriteError::NoSheets));
    let named = |n: &str| BookOut { sheets: vec![SheetOut::new(n, Vec::new())] };
    for bad in ["", "a[b", "a]b", "a:b", "a*b", "a?b", "a/b", "a\\b", "'a", "a'", "History", "hISTORY", "a\tb", "a\u{1}", "a\u{ffff}"] {
        assert!(matches!(err(&named(bad)), Some(WriteError::SheetName { sheet: 0, .. })), "{bad:?}");
    }
    assert!(matches!(err(&named(&"x".repeat(32))), Some(WriteError::SheetName { what: "is longer than Excel's 31 characters", .. })));
    // 16 characters past the BMP are 32 UTF-16 units, as Excel counts them
    assert!(matches!(err(&named(&"\u{1F600}".repeat(16))), Some(WriteError::SheetName { .. })));
    for twice in [["Daten", "DATEN"], ["Äpfel", "äPFEL"]] {
        let b = BookOut { sheets: twice.iter().map(|n| SheetOut::new(n, Vec::new())).collect() };
        assert!(matches!(err(&b), Some(WriteError::SheetName { sheet: 1, .. })), "{twice:?}");
    }
    // a value past the last row or column; Empty past them is fine
    let mut rows = vec![Vec::new(); MAX_ROWS as usize];
    rows.push(vec![Value::Empty]);
    assert!(err(&one(rows.clone())).is_none());
    rows.push(vec![Value::Empty, Value::Bool(true)]);
    assert_eq!(err(&one(rows)), Some(WriteError::Cell { sheet: 0, at: CellRef::new(MAX_ROWS + 1, 1), what: "a value past Excel's last row 1 048 576" }));
    let mut wide = vec![Value::Empty; MAX_COLS as usize - 1];
    wide.push(text("XFD"));
    let g = reopen(&written(&one(vec![wide.clone()]))).sheet(0).unwrap();
    assert_eq!(g.get(0, MAX_COLS - 1), &text("XFD"));
    wide.push(Value::Number(1.0));
    assert!(matches!(err(&one(vec![wide])), Some(WriteError::Cell { at: CellRef { row: 0, col: MAX_COLS }, .. })));
    // texts: 32767 characters fit, one more does not (UTF-16 units)
    assert!(err(&one(vec![vec![text(&"x".repeat(MAX_TEXT))]])).is_none());
    assert!(matches!(err(&one(vec![vec![Value::Empty, text(&"x".repeat(MAX_TEXT + 1))]])), Some(WriteError::Cell { at: CellRef { row: 0, col: 1 }, .. })));
    assert!(matches!(err(&one(vec![vec![text(&"\u{1F600}".repeat(MAX_TEXT / 2 + 1))]])), Some(WriteError::Cell { .. })));
    // settings
    let with = |f: &dyn Fn(&mut SheetOut)| {
        let mut s = SheetOut::new("S", vec![vec![text("a")]]);
        f(&mut s);
        err(&BookOut { sheets: vec![s] })
    };
    for w in [-1.0, 255.5, f64::NAN, f64::INFINITY] {
        assert!(matches!(with(&|s| s.widths = vec![None, Some(w)]), Some(WriteError::Sheet { sheet: 0, .. })), "{w}");
    }
    assert!(with(&|s| s.widths = vec![Some(255.0), Some(0.0), None]).is_none());
    assert!(matches!(with(&|s| s.widths = vec![None; MAX_COLS as usize + 1]), Some(WriteError::Sheet { .. })));
    assert!(matches!(with(&|s| s.header_rows = MAX_ROWS), Some(WriteError::Sheet { .. })));
    assert!(with(&|s| s.header_rows = MAX_ROWS - 1).is_none());
    assert_eq!(with(&|s| s.autofilter = true), Some(WriteError::Sheet { sheet: 0, what: "a filter needs a header row" }));
    // xlsx::WriteOptions
    let book = one(vec![vec![Value::Number(1.0), text("a"), Value::Empty, Value::Bool(true)]]);
    assert!(xlsx::write(&book, &XlsxOptions { cells: 3, ..XlsxOptions::DEFAULT }).is_ok());
    assert_eq!(xlsx::write(&book, &XlsxOptions { cells: 2, ..XlsxOptions::DEFAULT }).err(), Some(WriteError::Limit("cells")));
    assert_eq!(xlsx::write(&book, &XlsxOptions { bytes: 2000, ..XlsxOptions::DEFAULT }).err(), Some(WriteError::Limit("bytes")));
    assert!(xlsx::write(&book, &XlsxOptions { bytes: 6000, ..XlsxOptions::DEFAULT }).is_ok());
    // a large sheet is refused cell by cell, before its XML is complete
    let big = one((0..1000).map(|i| vec![text(&alloc::format!("{i:04}{}", "z".repeat(1000))), Value::Number(i as f64)]).collect());
    assert_eq!(xlsx::write(&big, &XlsxOptions { bytes: 100_000, ..XlsxOptions::DEFAULT }).err(), Some(WriteError::Limit("bytes")));
    assert!(WriteError::Cell { sheet: 0, at: CellRef::new(2, 1), what: "x" }.text().contains("sheet 1, cell B3"));
}

#[test]
fn xlsx_write_package() {
    let mut s = SheetOut::from_table("Daten", &Table { columns: vec![String::from("A")], rows: vec![vec![text("x")]] });
    s.autofilter = true;
    let book = BookOut { sheets: vec![s, SheetOut::new("Zwei", vec![vec![Value::Date(day(2026, 1, 1))]])] };
    let z = written(&book);
    // same book, same bytes; another value, other bytes
    assert_eq!(z, written(&book));
    let mut other = book.clone();
    other.sheets[1].rows[0][0] = Value::Date(day(2026, 1, 2));
    assert_ne!(z, written(&other));
    // the package passes OPC's checks and names every part with its content type
    let p = package(&z);
    assert_eq!(p.main_part(), Some("xl/workbook.xml"));
    let names: Vec<&str> = p.entries().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["[Content_Types].xml", "_rels/.rels", "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml", "xl/styles.xml", "xl/sharedStrings.xml", "docProps/core.xml", "docProps/app.xml"]
    );
    for (name, ct) in [
        ("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
        ("xl/worksheets/sheet2.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/styles.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"),
        ("xl/sharedStrings.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"),
        ("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
        ("docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
        ("_rels/.rels", "application/vnd.openxmlformats-package.relationships+xml"),
    ] {
        assert_eq!(p.content_type(name), Some(ct), "{name}");
    }
    let kinds: Vec<String> = p.relationships("").unwrap().into_iter().map(|r| r.kind).collect();
    assert!(kinds.iter().any(|k| k.ends_with("/core-properties")) && kinds.iter().any(|k| k.ends_with("/extended-properties")));
    for e in p.entries().iter().filter(|e| e.name.ends_with(".xml") && !e.name.starts_with('[')) {
        assert!(e.data.starts_with(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>"), "{}", e.name);
    }
    // the schema's order of children; no time anywhere
    let wb = part(&z, "xl/workbook.xml");
    assert!(in_order(&wb, &["<bookViews>", "<sheets>", "<definedNames>"]));
    assert!(wb.contains("<sheet name=\"Zwei\" sheetId=\"2\" r:id=\"rId2\"/>"));
    let styles = part(&z, "xl/styles.xml");
    assert!(in_order(&styles, &["<fonts", "<fills", "<borders", "<cellStyleXfs", "<cellXfs", "<cellStyles", "<dxfs", "<tableStyles"]));
    assert!(in_order(&styles, &["patternType=\"none\"", "patternType=\"gray125\""]));
    assert!(!part(&z, "docProps/core.xml").contains("dcterms:created"));
    let wb = reopen(&z);
    assert_eq!(wb.sheet(1).unwrap().get(0, 0), &Value::Date(day(2026, 1, 1)));
    assert!(!wb.date1904());
    // the DEFLATE level: stored and every level hold the same parts; 6 and 9 smaller than 1 (9 is
    // not always smaller than 6)
    let rows: Vec<Vec<Value>> = (0..500).map(|r| vec![text(&alloc::format!("Zeile {r}")), Value::Number(r as f64)]).collect();
    let book = one(rows);
    let packed: Vec<Vec<u8>> = [0u8, 1, 6, 9].iter().map(|&level| xlsx::write(&book, &XlsxOptions { level, ..XlsxOptions::DEFAULT }).unwrap()).collect();
    assert!(packed[0].len() > 2 * packed[1].len() && packed[1].len() >= packed[2].len() && packed[1].len() >= packed[3].len(), "{:?}", packed.iter().map(Vec::len).collect::<Vec<_>>());
    for z in &packed {
        assert_eq!(package(z).entries(), package(&packed[2]).entries());
        assert_eq!(reopen(z).sheet(0).unwrap().get(499, 0), &text("Zeile 499"));
    }
}

// ------------------------------------------------------------------ CSV

fn records(bytes: &[u8]) -> csv::Csv {
    csv::read(bytes, &csv::Options::default()).expect("read")
}

fn rec(c: &csv::Csv) -> Vec<Vec<&str>> {
    c.records.iter().map(|r| r.iter().map(String::as_str).collect()).collect()
}

#[test]
fn csv_write_values() {
    let rows = vec![
        vec![text("Name"), text("Wert"), text("Datum"), text("Ja")],
        vec![text("a,b"), Value::Number(1.5), Value::Date(day(2026, 11, 24)), Value::Bool(true)],
        vec![text("Zitat \"x\""), Value::Number(-0.0), Value::Date(serial(&Civil::new(2026, 11, 24, 14, 5, 30))), Value::Bool(false)],
        vec![text(" Rand "), Value::Number(1e21), Value::Date(0.5), Value::Error(CellError::NA)],
        vec![text("Zeile\r\nzwei"), Value::Number(f64::NAN), Value::Empty, text("")],
        vec![],
        vec![text("=1+2"), text("\tTab")],
    ];
    let b = csv::write(&rows, &WriteOptions::DEFAULT).unwrap();
    let want = "\u{feff}Name,Wert,Datum,Ja\r\n\"a,b\",1.5,2026-11-24,TRUE\r\n\"Zitat \"\"x\"\"\",0,2026-11-24 14:05:30,FALSE\r\n\" Rand \",1e21,12:00:00,#N/A\r\n\"Zeile\r\nzwei\",#NUM!,,\r\n\r\n=1+2,\"\tTab\"\r\n";
    assert_eq!(String::from_utf8(b.clone()).unwrap(), want);
    let c = records(&b);
    assert_eq!((c.delimiter, c.encoding), (b',', Encoding::Utf8Bom));
    // (a line break inside quotes reads as LF, as Excel keeps it in a cell)
    assert_eq!(rec(&c)[4], ["Zeile\nzwei", "#NUM!", "", ""]);
    let t = csv::to_table(&c, true, true);
    assert_eq!(
        t.rows[..3],
        [
            vec![text("a,b"), Value::Number(1.5), Value::Date(day(2026, 11, 24)), Value::Bool(true)],
            vec![text("Zitat \"x\""), Value::Number(0.0), Value::Date(serial(&Civil::new(2026, 11, 24, 14, 5, 30))), Value::Bool(false)],
            // a time alone stays text in to_table, an error too
            vec![text(" Rand "), Value::Number(1e21), text("12:00:00"), text("#N/A")],
        ]
    );
    // dates with milliseconds, numbers of every size read back exactly
    let nums = [0.1 + 0.2, -1e-300, 1.7976931348623157e308, 5e-324, 123_456.789, 1e-5, 1e17, -0.000_012_5];
    let b = csv::write(&[nums.iter().map(|&n| Value::Number(n)).collect(), vec![Value::Date(day(2026, 1, 2) + 0.25 + 0.25 / 86_400.0)]], &WriteOptions::DEFAULT).unwrap();
    let t = csv::to_table(&records(&b), false, true);
    assert_eq!(t.rows[0], nums.map(Value::Number));
    assert_eq!(t.rows[1][0].text(), "2026-01-02 06:00:00.250");
    // a table: its header, then its rows; an empty table is an empty file (the BOM)
    let table = Table {
        columns: vec![String::from("Name"), String::from("Zahl"), String::from("Tag"), String::from("Ok")],
        rows: vec![vec![text("x;y"), Value::Number(2.5), Value::Date(day(2026, 3, 1)), Value::Bool(false)], vec![text("z"), Value::Empty, Value::Empty, Value::Bool(true)]],
    };
    let b = csv::write_table(&table, &WriteOptions::DEFAULT).unwrap();
    assert_eq!(csv::to_table(&records(&b), true, true), table);
    assert_eq!(csv::write_table(&Table::default(), &WriteOptions::DEFAULT).unwrap(), [0xef, 0xbb, 0xbf]);
}

#[test]
fn csv_write_options() {
    // German: ; and a decimal comma, exponents included, read back as numbers
    let nums = [1234.5, -0.25, 1e-7, 1.5e17, 3.0, -2.5e-300];
    let b = csv::write(&[nums.iter().map(|&n| Value::Number(n)).collect(), vec![text("1;2"), text("a")]], &WriteOptions::GERMAN).unwrap();
    assert_eq!(String::from_utf8(b.clone()).unwrap(), "\u{feff}1234,5;-0,25;1e-07;1,5e17;3;-2,5e-300\r\n\"1;2\";a\r\n");
    let c = records(&b);
    assert_eq!(c.delimiter, b';');
    assert_eq!(csv::to_table(&c, false, true).rows[0], nums.map(Value::Number));
    // TAB and |, LF: the reader finds them again
    let rows = vec![vec![text("a b"), text("c|d"), text("e\tf")], vec![text("1"), Value::Empty, text("x")]];
    for d in [b'\t', b'|', b',', b';'] {
        let b = csv::write(&rows, &WriteOptions { delimiter: d, line_end: LineEnd::Lf, ..WriteOptions::DEFAULT }).unwrap();
        assert!(!b.contains(&b'\r'));
        let c = records(&b);
        assert_eq!((c.delimiter, rec(&c)), (d, vec![vec!["a b", "c|d", "e\tf"], vec!["1", "", "x"]]), "{}", d as char);
    }
    // encodings, each read back with its own detection
    let rows = vec![vec![text("Größe"), text("5 €"), text("Ÿ\u{81}")]];
    for enc in [Encoding::Utf8, Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Windows1252] {
        let b = csv::write(&rows, &WriteOptions { encoding: enc, ..WriteOptions::DEFAULT }).unwrap();
        let c = records(&b);
        assert_eq!((c.encoding, rec(&c)), (enc, vec![vec!["Größe", "5 €", "Ÿ\u{81}"]]), "{enc:?}");
    }
    let b = csv::write(&rows, &WriteOptions { encoding: Encoding::Windows1252, ..WriteOptions::DEFAULT }).unwrap();
    assert_eq!(b, b"Gr\xf6\xdfe,5 \x80,\x9f\x81\r\n");
    // a character Windows-1252 lacks is an error, not a "?"
    let w1252 = WriteOptions { encoding: Encoding::Windows1252, ..WriteOptions::DEFAULT };
    let rows = vec![vec![text("a")], vec![text("b"), Value::Empty, text("x日本")]];
    assert_eq!(csv::write(&rows, &w1252).err(), Some(csv::WriteError::Unmappable { record: 1, field: 2, ch: '日' }));
    assert!(matches!(csv::write(&[vec![text("\u{80}")]], &w1252).err(), Some(csv::WriteError::Unmappable { ch: '\u{80}', .. })));
    // the formula guard: texts only, quoted where the rest needs it
    let rows = vec![vec![text("=SUM(A1)"), text("+1"), text("-1"), text("@x"), text("\tx"), text("\rx"), text("a=b"), Value::Number(-5.0), text("-")]];
    let guard = WriteOptions { guard_formulas: true, line_end: LineEnd::Lf, encoding: Encoding::Utf8, ..WriteOptions::DEFAULT };
    assert_eq!(String::from_utf8(csv::write(&rows, &guard).unwrap()).unwrap(), "'=SUM(A1),'+1,'-1,'@x,'\tx,\"'\rx\",a=b,-5,'-\n");
    // options refused
    for (d, dec) in [(b'"', b'.'), (b'x', b'.'), (b'\n', b'.'), (b',', b','), (b';', b';'), (b';', b'x')] {
        let o = WriteOptions { delimiter: d, decimal: dec, ..WriteOptions::DEFAULT };
        assert_eq!(csv::write(&[], &o).err(), Some(csv::WriteError::Options), "{} {}", d as char, dec as char);
    }
    // the size limit; an exact fit passes
    let rows = vec![vec![text("abcd"), text("efgh")]];
    let exact = csv::write(&rows, &WriteOptions::DEFAULT).unwrap().len();
    assert!(csv::write(&rows, &WriteOptions { max_bytes: exact, ..WriteOptions::DEFAULT }).is_ok());
    assert_eq!(csv::write(&rows, &WriteOptions { max_bytes: exact - 1, ..WriteOptions::DEFAULT }).err(), Some(csv::WriteError::Limit("max_bytes")));
    // a first field "sep=" is quoted, or read and Excel would take it for the delimiter hint
    let b = csv::write(&[vec![text("SEP=;")], vec![text("sep=;")]], &WriteOptions::DEFAULT).unwrap();
    assert!(b.starts_with(b"\xef\xbb\xbf\"SEP=;\"\r\nsep=;\r\n"));
    let c = csv::read(&b, &csv::Options { delimiter: Some(b','), ..csv::Options::default() }).unwrap();
    assert_eq!(rec(&c), [vec!["SEP=;"], vec!["sep=;"]]);
}

#[test]
fn csv_write_round_trip() {
    // texts of every kind, ragged records, empty fields: read gives them back (CR as LF)
    let texts = ["", " ", "a", "\"", "\"\"", ",", ";", "a\nb", "a\rb", "a\r\nb", " lead", "trail\t", "Größe", "日本語", "\u{1F600}", "x,\"y\"\nz", "'quote", "#N/A"];
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for n in 1..=texts.len() {
        rows.push(texts[..n].iter().map(|s| text(s)).collect());
    }
    for d in [b',', b';', b'\t', b'|'] {
        let opts = WriteOptions { delimiter: d, ..WriteOptions::DEFAULT };
        let c = csv::read(&csv::write(&rows, &opts).unwrap(), &csv::Options { delimiter: Some(d), ..csv::Options::default() }).unwrap();
        assert_eq!(c.records.len(), rows.len());
        for (r, row) in rows.iter().enumerate() {
            let want: Vec<String> = row.iter().map(|v| v.text().replace("\r\n", "\n").replace('\r', "\n")).collect();
            // a record of one empty field is an empty line, read as one empty field
            assert_eq!(c.records[r], want, "{} record {r}", d as char);
        }
    }
}
