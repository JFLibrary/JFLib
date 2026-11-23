// sheet_checks -- codectest --sheet: lib/sheet_io's XLSX and CSV readers against workbooks and
// texts built here (nothing of Microsoft's in the repository): hostile ones must be refused with
// the right error, quickly and without allocating what they claim; controls of the same shape
// must still read.  Exit code 1 when a check fails.
//
//   xlsx_zip       a ZIP bomb by ratio (4 MiB of rows packed more than 200 times), too many parts,
//                  a truncated archive, no ZIP at all, no officeDocument relationship
//   xlsx_sparse    a <dimension> of the whole sheet with one cell at A1 and one at XFD1048576:
//                  two cells kept; its table over Limits::cells, refused before allocating
//   xlsx_limits    rows, columns, cells, shared strings and their bytes, one long shared string
//                  in 20000 cells (string_bytes per sheet), a shared-string index past the table
//   xlsx_xml       100000 nested elements (no recursion), a DOCTYPE with entities (billion
//                  laughs), an undefined entity, an unclosed part
//   xlsx_rels      a sheet whose relationship names the workbook, shared strings that are a
//                  worksheet, a sheet without relationship
//   xlsx_large     control: 50000 rows of 8 columns (shared strings, numbers, dates) read and
//                  turned into a Table in bounded time
//   csv_limits     64 MiB of delimiters, a quote that never closes, too many records, a header of
//                  16384 equal names (no quadratic naming)
//   csv_large      control: 200000 records of German CSV in Windows-1252, typed
//
// --only NAME runs the groups whose names start with NAME.  (build_rs.py test codectest runs it
// through tests/sheet.uis; --script is ignored)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::{Entry, Method};
use sheet_io::csv::{self, CsvLimits, Options};
use sheet_io::xlsx::{Error, Limits, Workbook};
use sheet_io::Value;

/// a refusal must come within this many microseconds
const QUICK_US: u64 = 1_000_000;

struct H {
    ok: u32,
    failed: u32,
    only: Option<String>,
}

impl H {
    fn on(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|o| name.starts_with(o.as_str()))
    }
    fn check(&mut self, name: &str, cond: bool, what: String) {
        if cond {
            self.ok += 1;
            plat::write_stdout(format!("ok   {name}: {what}\n").as_bytes());
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {name}: {what}\n").as_bytes());
        }
    }
    /// f() must give `want`, within QUICK_US
    fn quick<T: PartialEq + core::fmt::Debug>(&mut self, name: &str, want: T, f: impl FnOnce() -> T) {
        let t0 = plat::time_us();
        let r = f();
        let us = plat::time_us().wrapping_sub(t0);
        let what = format!("{r:?} (want {want:?}) in {} ms", us / 1000);
        self.check(name, r == want && us < QUICK_US, what);
    }
}

// ------------------------------------------------------------------ workbooks built here

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const PKG_R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

/// a workbook of one sheet "Daten": the sheet's whole XML, the shared strings (si elements) and
/// the styles (cellXfs numFmtIds); `rels` replaces the workbook's relationships when not empty
fn book(sheet: &str, strings: &str, xfs: &[u32], rels: &str) -> Vec<Entry> {
    let types = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/></Types>"
    );
    let xf: String = xfs.iter().map(|id| format!("<xf numFmtId=\"{id}\"/>")).collect();
    let wb_rels = if rels.is_empty() {
        format!("<Relationship Id=\"rId1\" Type=\"{R}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{R}/sharedStrings\" Target=\"sharedStrings.xml\"/><Relationship Id=\"rId3\" Type=\"{R}/styles\" Target=\"styles.xml\"/>")
    } else {
        String::from(rels)
    };
    let e = |n: &str, d: String| Entry { name: String::from(n), data: d.into_bytes() };
    vec![
        e("[Content_Types].xml", types),
        e("_rels/.rels", format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{R}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")),
        e("xl/workbook.xml", format!("{DECL}<workbook xmlns=\"{MAIN}\" xmlns:r=\"{R}\"><sheets><sheet name=\"Daten\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        e("xl/_rels/workbook.xml.rels", format!("{DECL}<Relationships xmlns=\"{PKG_R}\">{wb_rels}</Relationships>")),
        e("xl/worksheets/sheet1.xml", String::from(sheet)),
        e("xl/sharedStrings.xml", format!("{DECL}<sst xmlns=\"{MAIN}\" count=\"4000000000\" uniqueCount=\"4000000000\">{strings}</sst>")),
        e("xl/styles.xml", format!("{DECL}<styleSheet xmlns=\"{MAIN}\"><cellXfs count=\"{}\">{xf}</cellXfs></styleSheet>", xfs.len())),
    ]
}

fn ws(rows: &str) -> String {
    format!("{DECL}<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{R}\"><dimension ref=\"A1:XFD1048576\"/><sheetData>{rows}</sheetData></worksheet>")
}

fn zip(entries: &[Entry]) -> Vec<u8> {
    let big = archive::Limits { entries: 1000, part_bytes: 256 << 20, total_bytes: 512 << 20, archive_bytes: 512 << 20, ratio: 1032 };
    archive::write_with(entries, big, Method::Deflate(1)).unwrap_or_default()
}

fn open(z: &[u8], limits: Limits) -> Result<Workbook, Error> {
    Workbook::open(z, limits)
}

/// the error of reading sheet 0 (None: it read)
fn sheet_err(z: &[u8], limits: Limits) -> Option<Error> {
    match open(z, limits) {
        Ok(wb) => wb.sheet(0).err(),
        Err(e) => Some(e),
    }
}

// ------------------------------------------------------------------ the groups

fn xlsx_zip(t: &mut H) {
    let rows = "<row/>".repeat(700_000);
    let bomb = zip(&book(&ws(&rows), "", &[], ""));
    t.check("xlsx_zip/bomb_size", bomb.len() < (4 << 20) / 200, format!("{} bytes for 4.2 MB of XML", bomb.len()));
    t.quick("xlsx_zip/bomb", Some(Error::Archive(archive::Error::Limit)), || open(&bomb, Limits::default()).err());
    let roomy = Limits { archive: archive::Limits { ratio: 1032, ..Limits::DEFAULT.archive }, ..Limits::DEFAULT };
    t.check("xlsx_zip/bomb_allowed", open(&bomb, roomy).is_ok_and(|wb| wb.sheet(0).is_ok_and(|g| g.is_empty())), String::from("the same file with the ratio raised: 700000 empty rows"));
    let few = Limits { archive: archive::Limits { entries: 4, ..Limits::DEFAULT.archive }, ..Limits::DEFAULT };
    t.quick("xlsx_zip/entries", Some(Error::Archive(archive::Error::Limit)), || open(&bomb, few).err());
    let small = zip(&book(&ws(""), "", &[], ""));
    t.quick("xlsx_zip/truncated", Some(Error::Archive(archive::Error::Invalid)), || open(&small[..small.len() - 30], Limits::default()).err());
    t.quick("xlsx_zip/not_zip", Some(Error::NotXlsx), || open(b"Name;Wert\r\nA;1\r\n", Limits::default()).err());
    t.quick("xlsx_zip/xls", Some(Error::NotXlsx), || open(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0], Limits::default()).err());
    let mut e = book(&ws(""), "", &[], "");
    e[1].data = format!("{DECL}<Relationships xmlns=\"{PKG_R}\"/>").into_bytes();
    t.quick("xlsx_zip/no_main", Some(Error::NotXlsx), || open(&zip(&e), Limits::default()).err());
}

fn xlsx_sparse(t: &mut H) {
    let z = zip(&book(&ws(r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row>"#), "", &[], ""));
    let t0 = plat::time_us();
    let r = open(&z, Limits::default()).and_then(|wb| wb.sheet(0).map(|g| (g.cells(), g.get(1_048_575, 16_383).clone())));
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    t.check("xlsx_sparse/corners", r == Ok((2, Value::Number(2.0))) && ms < 1000, format!("{r:?} in {ms} ms"));
    t.quick("xlsx_sparse/table", Some(Error::Limit("cells")), || open(&z, Limits::default()).ok()?.sheet_table(0, true).err());
    t.quick("xlsx_sparse/area", Some(Error::Limit("cells")), || {
        let wb = open(&z, Limits::default()).ok()?;
        wb.table(&wb.area("Daten!A:XFD")?, false).err()
    });
}

fn xlsx_limits(t: &mut H) {
    let z = |rows: &str| zip(&book(&ws(rows), "<si><t>a</t></si><si><t>b</t></si><si><t>c</t></si>", &[], ""));
    let d = Limits::DEFAULT;
    t.quick("xlsx_limits/row_excel", Some(Error::Limit("rows")), || sheet_err(&z(r#"<row r="1048577"/>"#), d));
    t.quick("xlsx_limits/col_excel", Some(Error::Limit("cols")), || sheet_err(&z(r#"<row><c r="XFE1"><v>1</v></c></row>"#), d));
    t.quick("xlsx_limits/row_u32", Some(Error::Limit("rows")), || sheet_err(&z(r#"<row r="4294967296"/>"#), d));
    let many = z(&"<row><c><v>1</v></c></row>".repeat(2000));
    t.quick("xlsx_limits/rows", Some(Error::Limit("rows")), || sheet_err(&many, Limits { rows: 1000, ..d }));
    t.quick("xlsx_limits/cells", Some(Error::Limit("cells")), || sheet_err(&many, Limits { cells: 1999, ..d }));
    t.check("xlsx_limits/cells_ok", sheet_err(&many, Limits { cells: 2000, ..d }).is_none(), String::from("2000 cells within a limit of 2000"));
    let wide = z(&format!("<row>{}</row>", "<c><v>1</v></c>".repeat(300)));
    t.quick("xlsx_limits/cols", Some(Error::Limit("cols")), || sheet_err(&wide, Limits { cols: 256, ..d }));
    t.quick("xlsx_limits/strings", Some(Error::Limit("strings")), || open(&many, Limits { strings: 2, ..d }).err());
    t.quick("xlsx_limits/string_bytes", Some(Error::Limit("string_bytes")), || open(&many, Limits { string_bytes: 2, ..d }).err());
    // one shared string of 100 KB in 20000 cells: 2 GB of text if each cell copied it unbounded
    let long = "x".repeat(100_000);
    let amp = zip(&book(&ws(&"<row><c t=\"s\"><v>0</v></c></row>".repeat(20_000)), &format!("<si><t>{long}</t></si>"), &[], ""));
    t.quick("xlsx_limits/amplified", Some(Error::Limit("string_bytes")), || sheet_err(&amp, d));
    t.quick("xlsx_limits/index", Some(Error::Invalid("a shared string index past the table")), || sheet_err(&z(r#"<row><c t="s"><v>3</v></c></row>"#), d));
    t.quick("xlsx_limits/index_huge", Some(Error::Invalid("a shared string index")), || sheet_err(&z(r#"<row><c t="s"><v>99999999999999999999999</v></c></row>"#), d));
}

fn xlsx_xml(t: &mut H) {
    let d = Limits::DEFAULT;
    let deep = zip(&book(&format!("<worksheet xmlns=\"{MAIN}\">{}{}</worksheet>", "<x>".repeat(100_000), "</x>".repeat(100_000)), "", &[], ""));
    t.quick("xlsx_xml/deep", Some(Error::Limit("depth")), || sheet_err(&deep, d));
    let laughs = format!(
        "<?xml version=\"1.0\"?><!DOCTYPE worksheet [<!ENTITY a \"aaaaaaaaaa\">{}]><worksheet xmlns=\"{MAIN}\"><sheetData><row><c t=\"inlineStr\"><is><t>&z;</t></is></c></row></sheetData></worksheet>",
        (b'b'..=b'z').map(|c| { let p = (c - 1) as char; format!("<!ENTITY {} \"&{p};&{p};&{p};&{p};&{p};&{p};&{p};&{p};&{p};&{p};\">", c as char) }).collect::<String>()
    );
    let lz = zip(&book(&laughs, "", &[], ""));
    t.quick("xlsx_xml/doctype", true, || matches!(sheet_err(&lz, d), Some(Error::Xml { what, .. }) if what.contains("DOCTYPE")));
    let undefined = zip(&book(&ws(r#"<row><c t="inlineStr"><is><t>&z;</t></is></c></row>"#), "", &[], ""));
    t.quick("xlsx_xml/entity", true, || matches!(sheet_err(&undefined, d), Some(Error::Xml { .. })));
    let unclosed = zip(&book(&format!("<worksheet xmlns=\"{MAIN}\"><sheetData><row>{}", "<c><v>1</v></c>".repeat(10_000)), "", &[], ""));
    t.quick("xlsx_xml/unclosed", true, || matches!(sheet_err(&unclosed, d), Some(Error::Xml { .. })));
    let deep_book = {
        let mut e = book(&ws(""), "", &[], "");
        e[2].data = format!("<workbook xmlns=\"{MAIN}\">{}{}</workbook>", "<x>".repeat(5000), "</x>".repeat(5000)).into_bytes();
        zip(&e)
    };
    t.quick("xlsx_xml/deep_workbook", Some(Error::Limit("depth")), || open(&deep_book, d).err());
}

fn xlsx_rels(t: &mut H) {
    let d = Limits::DEFAULT;
    let cycle = zip(&book(&ws(""), "", &[], &format!("<Relationship Id=\"rId1\" Type=\"{R}/worksheet\" Target=\"workbook.xml\"/><Relationship Id=\"rId2\" Type=\"{R}/sharedStrings\" Target=\"sharedStrings.xml\"/><Relationship Id=\"rId3\" Type=\"{R}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId4\" Type=\"{R}/worksheet\" Target=\"worksheets/sheet1.xml\"/>")));
    t.quick("xlsx_rels/cycle", Some(Error::Invalid("a worksheet part whose root is no <worksheet>")), || sheet_err(&cycle, d));
    let swapped = zip(&book(&ws(""), "", &[], &format!("<Relationship Id=\"rId1\" Type=\"{R}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{R}/sharedStrings\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId3\" Type=\"{R}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId4\" Type=\"{R}/worksheet\" Target=\"sharedStrings.xml\"/>")));
    t.quick("xlsx_rels/swapped", Some(Error::Invalid("the shared strings part's root is no <sst>")), || open(&swapped, d).err());
    let none = zip(&book(&ws(""), "", &[], &format!("<Relationship Id=\"rId9\" Type=\"{R}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{R}/sharedStrings\" Target=\"sharedStrings.xml\"/><Relationship Id=\"rId3\" Type=\"{R}/styles\" Target=\"styles.xml\"/>")));
    t.quick("xlsx_rels/unlinked", Some(true), || open(&none, d).ok().map(|wb| wb.sheets()[0].kind == sheet_io::xlsx::SheetKind::Other && wb.sheet(0).is_ok_and(|g| g.is_empty())));
}

fn xlsx_large(t: &mut H) {
    let strings: String = (0..1000).map(|i| format!("<si><t>Name {i}</t></si>")).collect();
    let mut rows = String::with_capacity(50_000 * 260);
    rows.push_str(r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="B1" t="inlineStr"><is><t>Wert</t></is></c><c r="C1" t="inlineStr"><is><t>Datum</t></is></c></row>"#);
    for r in 2..=50_000u32 {
        rows.push_str(&format!(
            "<row r=\"{r}\"><c r=\"A{r}\" t=\"s\"><v>{}</v></c><c r=\"B{r}\"><v>{}.5</v></c><c r=\"C{r}\" s=\"1\"><v>{}</v></c><c r=\"D{r}\"><v>{r}</v></c><c r=\"E{r}\" t=\"b\"><v>1</v></c><c r=\"F{r}\" t=\"str\"><f>A{r}</f><v>x</v></c><c r=\"G{r}\"><v>-1</v></c><c r=\"H{r}\"><v>1e3</v></c></row>",
            r % 1000,
            r,
            45_000 + r % 3000
        ));
    }
    let z = zip(&book(&ws(&rows), &strings, &[0, 14], ""));
    let t0 = plat::time_us();
    let r = open(&z, Limits::default()).and_then(|wb| wb.sheet_table(0, true));
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    let ok = r.as_ref().is_ok_and(|tab| {
        tab.rows.len() == 49_999
            && tab.columns.len() == 8
            && tab.columns[..3] == [String::from("Name"), String::from("Wert"), String::from("Datum")]
            && tab.rows[0][0] == Value::Text(String::from("Name 2"))
            && tab.rows[0][2].text() == "2023-03-17"
            && tab.rows[49_998][3] == Value::Number(50_000.0)
    });
    t.check("xlsx_large/table", ok && ms < 30_000, format!("{} KB, 400000 cells to a Table in {ms} ms ({:?})", z.len() / 1024, r.as_ref().err()));
}

fn csv_limits(t: &mut H) {
    let commas = vec![b';'; 64 << 20];
    t.quick("csv_limits/delimiters", Some(csv::Error::Limit("fields")), || csv::read(&commas, &Options::default()).err());
    let lines = "a;b\n".repeat(1 << 20);
    t.quick("csv_limits/records", Some(csv::Error::Limit("records")), || csv::read(lines.as_bytes(), &Options { limits: CsvLimits { records: 1000, ..CsvLimits::DEFAULT }, ..Options::DEFAULT }).err());
    t.quick("csv_limits/cells", Some(csv::Error::Limit("cells")), || csv::read(lines.as_bytes(), &Options { limits: CsvLimits { cells: 1000, ..CsvLimits::DEFAULT }, ..Options::DEFAULT }).err());
    let open_quote = format!("a;\"{}", "x".repeat(2 << 20));
    t.quick("csv_limits/open_quote", Some(csv::Error::Limit("field_bytes")), || csv::read(open_quote.as_bytes(), &Options::DEFAULT).err());
    t.quick("csv_limits/bytes", Some(csv::Error::Limit("bytes")), || csv::read(&commas, &Options { limits: CsvLimits { bytes: 1 << 20, ..CsvLimits::DEFAULT }, ..Options::DEFAULT }).err());
    let header = format!("{}\n", vec!["Name"; 16_384].join(";"));
    t.quick("csv_limits/names", Some(String::from("Name16383")), || csv::read(header.as_bytes(), &Options::DEFAULT).ok().map(|c| csv::to_table(&c, true, false).columns[16_383].clone()));
}

fn csv_large(t: &mut H) {
    let mut text: Vec<u8> = b"Name;Betrag;Datum;Aktiv\r\n".to_vec();
    for i in 0..200_000u32 {
        text.extend_from_slice(format!("Gr\u{f6}\u{df}e {i};{}.{:03},{:02};{}.{}.2026;{}\r\n", i / 1000 + 1, i % 1000, i % 100, i % 28 + 1, i % 12 + 1, if i % 2 == 0 { "WAHR" } else { "FALSCH" }).as_bytes());
    }
    // Windows-1252: the umlauts as single bytes
    let bytes: Vec<u8> = String::from_utf8(text).unwrap_or_default().chars().map(|c| c as u32 as u8).collect();
    let t0 = plat::time_us();
    let r = csv::read(&bytes, &Options::DEFAULT).map(|c| (c.encoding, c.delimiter, csv::to_table(&c, true, true)));
    let ms = plat::time_us().wrapping_sub(t0) / 1000;
    let ok = r.as_ref().is_ok_and(|(enc, d, tab)| {
        *enc == csv::Encoding::Windows1252
            && *d == b';'
            && tab.rows.len() == 200_000
            && tab.rows[1][0] == Value::Text(String::from("Größe 1"))
            && tab.rows[1234][1] == Value::Number(2234.34)
            && tab.rows[1][3] == Value::Bool(false)
            && matches!(tab.rows[5][2], Value::Date(_))
    });
    t.check("csv_large/table", ok && ms < 30_000, format!("{} KB, 200000 records typed in {ms} ms ({:?})", bytes.len() / 1024, r.as_ref().err()));
}

/// run every group -> exit code
pub fn run(only: Option<&str>) -> i32 {
    let mut t = H { ok: 0, failed: 0, only: only.map(String::from) };
    let groups: [(&str, fn(&mut H)); 8] = [
        ("xlsx_zip", xlsx_zip),
        ("xlsx_sparse", xlsx_sparse),
        ("xlsx_limits", xlsx_limits),
        ("xlsx_xml", xlsx_xml),
        ("xlsx_rels", xlsx_rels),
        ("xlsx_large", xlsx_large),
        ("csv_limits", csv_limits),
        ("csv_large", csv_large),
    ];
    for (name, f) in groups {
        if t.on(name) {
            f(&mut t);
        }
    }
    plat::write_stdout(format!("sheet: {} ok, {} failed\n", t.ok, t.failed).as_bytes());
    if t.failed == 0 { 0 } else { 1 }
}
