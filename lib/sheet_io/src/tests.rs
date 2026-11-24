// the unit tests (cargo test -p jflib-sheet-io --lib): workbooks built here with archive's ZIP
// writer (nothing of Microsoft's in the repository), hostile ones among them, and CSV texts

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::{Entry, Method};

use crate::csv::{self, CsvLimits, Encoding, Options};
use crate::xlsx::{Error, Limits, SheetKind, SheetState, Workbook};
use crate::{civil, col_name, parse_area, parse_ref, quote_sheet, serial, Area, CellError, CellRef, Civil, Table, Value, MAX_COLS, MAX_ROWS};

// ------------------------------------------------------------------ workbooks built here

const R_T: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_S: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const MAIN_T: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const MAIN_S: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const PKG_R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const XLSX_MAIN: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const XLSM_MAIN: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

struct Sheet {
    name: String,
    state: &'static str,
    kind: &'static str,
    xml: String,
}

struct Book {
    strict: bool,
    main_type: &'static str,
    sheets: Vec<Sheet>,
    strings: Option<String>,
    styles: Option<String>,
    head: String,
    names: String,
    // more parts: (name, bytes)
    extra: Vec<(String, Vec<u8>)>,
    // more relationships of the workbook
    rels: String,
}

impl Book {
    fn new() -> Book {
        Book { strict: false, main_type: XLSX_MAIN, sheets: Vec::new(), strings: None, styles: None, head: String::new(), names: String::new(), extra: Vec::new(), rels: String::new() }
    }
    fn main(&self) -> &'static str {
        if self.strict { MAIN_S } else { MAIN_T }
    }
    fn r(&self) -> &'static str {
        if self.strict { R_S } else { R_T }
    }
    fn sheet(self, name: &str, rows: &str) -> Book {
        let xml = ws_ns(self.main(), self.r(), rows);
        self.sheet_with(name, "", "worksheet", xml)
    }
    fn sheet_with(mut self, name: &str, state: &'static str, kind: &'static str, xml: String) -> Book {
        self.sheets.push(Sheet { name: String::from(name), state, kind, xml });
        self
    }
    fn strings(mut self, items: &str) -> Book {
        self.strings = Some(format!("{DECL}<sst xmlns=\"{}\" count=\"999999999\" uniqueCount=\"999999999\">{items}</sst>", self.main()));
        self
    }
    fn styles(mut self, numfmts: &str, xfs: &[u32]) -> Book {
        let xf: String = xfs.iter().map(|id| format!("<xf numFmtId=\"{id}\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>")).collect();
        self.styles = Some(format!(
            "{DECL}<styleSheet xmlns=\"{}\"><numFmts count=\"1\">{numfmts}</numFmts><cellStyleXfs count=\"1\"><xf numFmtId=\"14\"/></cellStyleXfs><cellXfs count=\"{}\">{xf}</cellXfs></styleSheet>",
            self.main(),
            xfs.len()
        ));
        self
    }
    fn entries(&self) -> Vec<Entry> {
        let (main, r) = (self.main(), self.r());
        let mut types = format!(
            "{DECL}<Types xmlns=\"{CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"{}\"/>",
            self.main_type
        );
        let mut out = Vec::new();
        let mut sheets = String::new();
        let mut rels = String::new();
        for (i, s) in self.sheets.iter().enumerate() {
            let dir = if s.kind == "chartsheet" { "chartsheets" } else { "worksheets" };
            let part = format!("xl/{dir}/sheet{}.xml", i + 1);
            let state = if s.state.is_empty() { String::new() } else { format!(" state=\"{}\"", s.state) };
            sheets.push_str(&format!("<sheet name=\"{}\" sheetId=\"{}\"{state} r:id=\"rId{}\"/>", esc(&s.name), i + 1, i + 1));
            rels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"{r}/{}\" Target=\"{dir}/sheet{}.xml\"/>", i + 1, s.kind, i + 1));
            types.push_str(&format!("<Override PartName=\"/{part}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.{}+xml\"/>", s.kind));
            out.push(Entry { name: part, data: s.xml.clone().into_bytes() });
        }
        if let Some(s) = &self.strings {
            rels.push_str(&format!("<Relationship Id=\"rId100\" Type=\"{r}/sharedStrings\" Target=\"sharedStrings.xml\"/>"));
            out.push(Entry { name: String::from("xl/sharedStrings.xml"), data: s.clone().into_bytes() });
        }
        if let Some(s) = &self.styles {
            rels.push_str(&format!("<Relationship Id=\"rId101\" Type=\"{r}/styles\" Target=\"styles.xml\"/>"));
            out.push(Entry { name: String::from("xl/styles.xml"), data: s.clone().into_bytes() });
        }
        rels.push_str(&self.rels);
        let names = if self.names.is_empty() { String::new() } else { format!("<definedNames>{}</definedNames>", self.names) };
        let wb = format!("{DECL}<workbook xmlns=\"{main}\" xmlns:r=\"{r}\">{}<sheets>{sheets}</sheets>{names}<calcPr calcId=\"191029\"/></workbook>", self.head);
        out.push(Entry { name: String::from("xl/workbook.xml"), data: wb.into_bytes() });
        out.push(Entry { name: String::from("xl/_rels/workbook.xml.rels"), data: format!("{DECL}<Relationships xmlns=\"{PKG_R}\">{rels}</Relationships>").into_bytes() });
        let office = format!("{r}/officeDocument");
        out.push(Entry { name: String::from("_rels/.rels"), data: format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{office}\" Target=\"xl/workbook.xml\"/></Relationships>").into_bytes() });
        for (n, d) in &self.extra {
            out.push(Entry { name: n.clone(), data: d.clone() });
        }
        types.push_str("</Types>");
        out.insert(0, Entry { name: String::from("[Content_Types].xml"), data: types.into_bytes() });
        out
    }
    fn zip(&self) -> Vec<u8> {
        zip(&self.entries())
    }
    fn open(&self) -> Result<Workbook, Error> {
        Workbook::open(&self.zip(), Limits::default())
    }
}

fn zip(entries: &[Entry]) -> Vec<u8> {
    let big = archive::Limits { entries: 1000, part_bytes: 64 << 20, total_bytes: 64 << 20, archive_bytes: 64 << 20, ratio: 1032 };
    archive::write_with(entries, big, Method::Deflate(6)).expect("zip")
}

fn ws_ns(main: &str, r: &str, rows: &str) -> String {
    format!("{DECL}<worksheet xmlns=\"{main}\" xmlns:r=\"{r}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetData>{rows}</sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>")
}

fn ws(rows: &str) -> String {
    ws_ns(MAIN_T, R_T, rows)
}

fn text(s: &str) -> Value {
    Value::Text(String::from(s))
}

fn day(y: i32, m: u8, d: u8) -> f64 {
    serial(&Civil::date(y, m, d))
}

fn names(t: &Table) -> Vec<&str> {
    t.columns.iter().map(String::as_str).collect()
}

// ------------------------------------------------------------------ common types

#[test]
fn refs_and_areas() {
    assert_eq!(col_name(0), "A");
    assert_eq!(col_name(25), "Z");
    assert_eq!(col_name(26), "AA");
    assert_eq!(col_name(27), "AB");
    assert_eq!(col_name(16_383), "XFD");
    assert_eq!(col_name(u32::MAX), "MWLQKWV");
    assert_eq!(parse_ref("$B$3"), Some(CellRef::new(2, 1)));
    assert_eq!(parse_ref("b3"), Some(CellRef::new(2, 1)));
    assert_eq!(parse_ref("XFD1048576"), Some(CellRef::new(1_048_575, 16_383)));
    assert_eq!(parse_ref("MWLQKWV4294967296"), Some(CellRef::new(u32::MAX, u32::MAX)));
    for bad in ["", "B", "3", "B0", "$$B3", "B3$", "B-3", "MWLQKWW1", "A4294967297", "Ä1", "B 3"] {
        assert_eq!(parse_ref(bad), None, "{bad}");
    }
    assert_eq!(CellRef::new(2, 27).a1(), "AB3");
    let sheets = ["Tab 1", "O'Brien", "Daten"];
    let a = parse_area("'Tab 1'!$A$1:$C$10", &sheets).unwrap();
    assert_eq!(a, Area { sheet: Some(0), first: CellRef::new(0, 0), last: CellRef::new(9, 2) });
    assert_eq!(a.rows(), 10);
    assert_eq!(a.cols(), 3);
    assert_eq!(a.a1(), "A1:C10");
    assert_eq!(parse_area("='O''Brien'!B2", &sheets).unwrap().sheet, Some(1));
    assert_eq!(parse_area("daten!C3:A1", &sheets).unwrap(), Area { sheet: Some(2), first: CellRef::new(0, 0), last: CellRef::new(2, 2) });
    assert_eq!(parse_area("A1", &sheets).unwrap().sheet, None);
    let cols = parse_area("Daten!$A:$C", &sheets).unwrap();
    assert_eq!((cols.first, cols.last), (CellRef::new(0, 0), CellRef::new(MAX_ROWS - 1, 2)));
    let rows = parse_area("Daten!2:3", &sheets).unwrap();
    assert_eq!((rows.first, rows.last), (CellRef::new(1, 0), CellRef::new(2, MAX_COLS - 1)));
    for bad in ["Fehlt!A1", "Daten:Tab!A1", "[1]Daten!A1", "A1,B2", "SUM(A1:B2)", "#REF!", "Daten!#REF!", "A:3", "'Tab 1!A1", "A"] {
        assert_eq!(parse_area(bad, &sheets), None, "{bad}");
    }
    assert!(a.contains(CellRef::new(9, 2)) && !a.contains(CellRef::new(10, 0)));
    assert_eq!(quote_sheet("Daten"), "Daten");
    assert_eq!(quote_sheet("Tab 1"), "'Tab 1'");
    assert_eq!(quote_sheet("O'Brien"), "'O''Brien'");
    assert_eq!(quote_sheet("A1"), "'A1'");
    assert_eq!(quote_sheet("2026"), "'2026'");
}

#[test]
fn dates_and_texts() {
    let s = serial(&Civil::new(2026, 11, 23, 14, 5, 0));
    assert_eq!(civil(s), Some(Civil::new(2026, 11, 23, 14, 5, 0)));
    assert_eq!(day(1899, 12, 30), 0.0);
    assert_eq!(day(1900, 3, 1), 61.0);
    assert_eq!(day(1970, 1, 1), 25_569.0);
    assert_eq!(civil(-1.5), Some(Civil::new(1899, 12, 28, 12, 0, 0)));
    assert_eq!(civil(f64::NAN), None);
    assert_eq!(civil(1.0e9), None);
    assert_eq!(Value::Date(s).text(), "2026-11-23 14:05:00");
    assert_eq!(Value::Date(day(2026, 11, 23)).text(), "2026-11-23");
    assert_eq!(Value::Date(0.5).text(), "12:00:00");
    assert_eq!(Value::Date(0.25 + 0.5 / 86_400.0).text(), "06:00:00.500");
    assert_eq!(Value::Number(1.5).text(), "1.5");
    assert_eq!(Value::Number(3.0).text(), "3");
    assert_eq!(Value::Number(-0.0).text(), "0");
    assert_eq!(Value::Bool(true).text(), "TRUE");
    assert_eq!(Value::Error(CellError::NA).text(), "#N/A");
    assert_eq!(Value::Empty.text(), "");
    assert_eq!(CellError::parse("#div/0!"), CellError::Div0);
    assert_eq!(CellError::parse("#SPILL!"), CellError::Spill);
    assert_eq!(CellError::parse("#BUSY!"), CellError::Other(String::from("#BUSY!")));
    assert_eq!(CellError::Other(String::from("#BUSY!")).text(), "#BUSY!");
    // header names: trimmed, empty ones F<n>, repeated ones numbered (case-insensitive)
    let n = crate::column_names(vec![" Name ".into(), "".into(), "name".into(), "Name1".into(), "NAME".into(), "F2".into()]);
    assert_eq!(n, ["Name", "F2", "name1", "Name11", "NAME2", "F21"]);
    let many = crate::column_names(vec![String::from("x"); 16_384]);
    assert_eq!(many[16_383], "x16383");
}

// ------------------------------------------------------------------ XLSX

#[test]
fn workbook_basics() {
    let chart = format!("{DECL}<chartsheet xmlns=\"{MAIN_T}\" xmlns:r=\"{R_T}\"><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews></chartsheet>");
    let b = Book::new()
        .sheet("Daten", r#"<row r="1" spans="1:2"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2"><v>1.5</v></c><c r="B2" t="b"><v>1</v></c></row>"#)
        .sheet_with("Versteckt", "hidden", "worksheet", ws(r#"<row r="3"><c r="C3"><v>3</v></c></row>"#))
        .sheet_with("Ganz versteckt", "veryHidden", "worksheet", ws(""))
        .sheet_with("Diagramm", "", "chartsheet", chart)
        .strings("<si><t>Name</t></si><si><t>Wert</t></si>");
    let mut b = b;
    b.names = String::from(concat!(
        r#"<definedName name="Bereich">Daten!$A$1:$B$2</definedName>"#,
        r#"<definedName name="Lokal" localSheetId="1" hidden="1">'Versteckt'!$C$3</definedName>"#,
        r#"<definedName name="_xlnm.Print_Area" localSheetId="0">Daten!$A$1:$B$2</definedName>"#,
        r#"<definedName name="Formel">SUM(Daten!A1:A2)</definedName>"#,
        r#"<definedName name="Spalten">Daten!$A:$B</definedName>"#,
        r#"<definedName name="Ohne" localSheetId="0">$B$2</definedName>"#,
        r#"<definedName name="Weit" localSheetId="9">Daten!A1</definedName>"#,
    ));
    let wb = b.open().expect("open");
    let sheets: Vec<(&str, SheetState, SheetKind)> = wb.sheets().iter().map(|s| (s.name.as_str(), s.state, s.kind)).collect();
    assert_eq!(
        sheets,
        [
            ("Daten", SheetState::Visible, SheetKind::Worksheet),
            ("Versteckt", SheetState::Hidden, SheetKind::Worksheet),
            ("Ganz versteckt", SheetState::VeryHidden, SheetKind::Worksheet),
            ("Diagramm", SheetState::Visible, SheetKind::Chartsheet),
        ]
    );
    assert!(!wb.date1904());
    let n = wb.names();
    assert_eq!(n.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Bereich", "Lokal", "_xlnm.Print_Area", "Formel", "Spalten", "Ohne"]);
    assert_eq!(n[0].area, Some(Area { sheet: Some(0), first: CellRef::new(0, 0), last: CellRef::new(1, 1) }));
    assert_eq!((n[1].scope, n[1].hidden, n[1].area.map(|a| a.sheet)), (Some(1), true, Some(Some(1))));
    assert!(n[2].builtin() && !n[0].builtin());
    assert_eq!(n[3].area, None);
    assert_eq!(n[3].formula, "SUM(Daten!A1:A2)");
    assert_eq!(n[4].area.map(|a| a.last), Some(CellRef::new(MAX_ROWS - 1, 1)));
    assert_eq!(n[5].area.map(|a| (a.sheet, a.first)), Some((Some(0), CellRef::new(1, 1))));
    let g = wb.sheet(0).unwrap();
    assert_eq!(g.get(0, 0), &text("Name"));
    assert_eq!(g.get(1, 0), &Value::Number(1.5));
    assert_eq!(g.get(1, 1), &Value::Bool(true));
    assert_eq!(g.get(5, 5), &Value::Empty);
    assert_eq!(g.cells(), 4);
    assert_eq!(g.used(), Some(Area { sheet: Some(0), first: CellRef::new(0, 0), last: CellRef::new(1, 1) }));
    assert_eq!(wb.sheet(1).unwrap().used().map(|a| a.first), Some(CellRef::new(2, 2)));
    assert!(wb.sheet(2).unwrap().is_empty());
    assert!(wb.sheet(3).unwrap().is_empty());
    assert_eq!(wb.sheet(4).err(), Some(Error::Missing("sheet")));
    assert_eq!(wb.sheet_by_name("daten"), Some(0));
    assert_eq!(wb.sheet_by_name("GANZ VERSTECKT"), Some(2));
    assert_eq!(wb.area("'Ganz versteckt'!B2").map(|a| a.sheet), Some(Some(2)));
    let t = wb.sheet_table(0, true).unwrap();
    assert_eq!(names(&t), ["Name", "Wert"]);
    assert_eq!(t.rows, [vec![Value::Number(1.5), Value::Bool(true)]]);
    assert_eq!(t.column("wert"), Some(1));
    let t = wb.sheet_table(1, false).unwrap();
    assert_eq!((names(&t), t.rows.clone()), (vec!["F1"], vec![vec![Value::Number(3.0)]]));
    assert_eq!(wb.sheet_table(2, true).unwrap(), Table::default());
}

#[test]
fn shared_strings() {
    let b = Book::new()
        .sheet("S", &(0..10).map(|i| format!("<row r=\"{}\"><c r=\"A{}\" t=\"s\"><v>{i}</v></c></row>", i + 1, i + 1)).collect::<String>())
        .strings(concat!(
            r#"<si><r><rPr><b/><sz val="11"/></rPr><t>Fett</t></r><r><t xml:space="preserve"> und normal</t></r><rPh sb="0" eb="1"><t>ふりがな</t></rPh><phoneticPr fontId="1"/></si>"#,
            "<si><t>Zeile_x000D_\nneu</t></si>",
            r#"<si><t>_x005F_x0041_ und _x0041_ und _x00zz_</t></si>"#,
            r#"<si><t>  getrimmt  </t></si>"#,
            r#"<si><t xml:space="preserve">  bleibt  </t></si>"#,
            r#"<si><t>Smiley _xD83D__xDE00_ Ende _xD800_</t></si>"#,
            r#"<si><t>&lt;&amp;&gt; &#x20AC;&#8364;</t></si>"#,
            r#"<si><t><![CDATA[cdata <x> & ]]></t><!-- a comment --></si>"#,
            r#"<si/>"#,
            r#"<si><t>Ä😀</t></si>"#,
        ));
    let wb = b.open().expect("open");
    let g = wb.sheet(0).unwrap();
    let got: Vec<Value> = (0..10).map(|r| g.get(r, 0).clone()).collect();
    assert_eq!(
        got,
        [
            text("Fett und normal"),
            text("Zeile\r\nneu"),
            text("_x0041_ und A und _x00zz_"),
            text("getrimmt"),
            text("  bleibt  "),
            text("Smiley 😀 Ende _xD800_"),
            text("<&> €€"),
            text("cdata <x> &"), // no xml:space="preserve": trimmed, CDATA too
            Value::Empty,
            text("Ä😀"),
        ]
    );
}

#[test]
fn cell_types() {
    let b = Book::new().sheet(
        "T",
        concat!(
            r#"<row r="1">"#,
            r#"<c r="A1" t="inlineStr"><is><t>inline</t></is></c>"#,
            r#"<c r="B1" t="inlineStr"><is><r><t>rich</t></r><r><t xml:space="preserve"> inline</t></r><rPh><t>x</t></rPh></is></c>"#,
            r#"<c r="C1" t="str"><f>CONCAT("a","b")</f><v>ab</v></c>"#,
            r#"<c r="D1"><f>1+1</f><v>2</v></c>"#,
            r#"<c r="E1"><f>NOW()</f></c>"#,
            r#"<c r="F1" t="b"><v>0</v></c>"#,
            r#"<c r="G1" t="e"><v>#N/A</v></c>"#,
            r#"<c r="H1" t="e"><f>1/0</f><v>#DIV/0!</v></c>"#,
            r#"<c r="I1" t="d"><v>2026-11-23T14:05:00</v></c>"#,
            r#"<c r="J1" t="d"><v>2026-11-23</v></c>"#,
            r#"<c r="K1" t="e"><v>#BUSY!</v></c>"#,
            r#"<c r="L1"><v>-1.25E-3</v></c>"#,
            r#"<c r="M1" s="0"/>"#,
            r#"<c r="N1" t="n"><v>abc</v></c>"#,
            r#"<c r="O1" t="d"><v>14:05:00</v></c>"#,
            r#"<c r="P1" t="str"><f>""</f><v></v></c>"#,
            r#"<c r="Q1"><v> 7 </v></c>"#,
            r#"<c r="R1" t="b"><v>true</v></c>"#,
            r#"<c r="S1"><v>inf</v></c>"#,
            r#"</row>"#,
        ),
    );
    let wb = b.open().expect("open");
    let g = wb.sheet(0).unwrap();
    let row: Vec<Value> = (0..19).map(|c| g.get(0, c).clone()).collect();
    assert_eq!(
        row,
        [
            text("inline"),
            text("rich inline"),
            text("ab"),
            Value::Number(2.0),
            Value::Empty,
            Value::Bool(false),
            Value::Error(CellError::NA),
            Value::Error(CellError::Div0),
            Value::Date(serial(&Civil::new(2026, 11, 23, 14, 5, 0))),
            Value::Date(day(2026, 11, 23)),
            Value::Error(CellError::Other(String::from("#BUSY!"))),
            Value::Number(-0.00125),
            Value::Empty,
            text("abc"),
            Value::Date((14.0 * 60.0 + 5.0) / 1440.0),
            Value::Empty,
            Value::Number(7.0),
            Value::Bool(true),
            text("inf"),
        ]
    );
    // only cells with a value are kept
    assert_eq!(g.cells(), 16);
}

// number formats: (numFmtId, code)
const FORMATS: &str = concat!(
    r#"<numFmt numFmtId="164" formatCode="yyyy\-mm\-dd\ hh:mm"/>"#,
    r#"<numFmt numFmtId="165" formatCode="[h]:mm"/>"#,
    r#"<numFmt numFmtId="166" formatCode="&quot;Datum&quot;\ 0"/>"#,
    r#"<numFmt numFmtId="167" formatCode="[$-407]dd/mm/yyyy;@"/>"#,
    r#"<numFmt numFmtId="168" formatCode="0.00E+00"/>"#,
    r#"<numFmt numFmtId="169" formatCode="[Red]#,##0.00\ _€;\-#,##0.00\ _€"/>"#,
    r#"<numFmt numFmtId="170" formatCode="h:mm"/>"#,
    r#"<numFmt numFmtId="171" formatCode="mmmm"/>"#,
    r#"<numFmt numFmtId="172" formatCode="General"/>"#,
    r##"<numFmt numFmtId="173" formatCode="#,##0\ \d"/>"##,
    r#"<numFmt numFmtId="174" formatCode="0 &quot;h&quot; * 0"/>"#,
    r#"<numFmt numFmtId="175" formatCode="[$€-407] #,##0.00;[mm]"/>"#,
);
// cellXfs: s=0 General, 1 = 14 (m/d/yyyy), 2 = 164, 3 = 165 ... 13 = 175, 14 = 46 ([h]:mm:ss),
// 15 = 22 (m/d/yyyy h:mm), 16 = 20 (h:mm), 17 = 45 (mm:ss), 18 = 30 (East Asian date), 19 = 49 (@)
const XFS: [u32; 20] = [0, 14, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 46, 22, 20, 45, 30, 49];

fn date_book(date1904: bool, cells: &[(u32, f64)]) -> Workbook {
    let rows: String = cells.iter().enumerate().map(|(i, (s, v))| format!("<row r=\"{0}\"><c r=\"A{0}\" s=\"{s}\"><v>{v}</v></c></row>", i + 1)).collect();
    let mut b = Book::new().sheet("D", &rows).styles(FORMATS, &XFS);
    if date1904 {
        b.head = String::from(r#"<fileVersion appName="xl"/><workbookPr date1904="1" defaultThemeVersion="166925"/>"#);
    }
    b.open().expect("open")
}

#[test]
fn dates_1900() {
    let nov23 = day(2026, 11, 23);
    let cells = [
        (1, nov23),
        (1, 1.0),
        (1, 59.0),
        (1, 60.0),
        (1, 61.0),
        (2, nov23 + 0.586_805_555_6),
        (3, 1.5),
        (4, 7.0),
        (5, nov23),
        (6, 12345.0),
        (7, 3.0),
        (8, 0.5),
        (9, nov23),
        (10, 4.0),
        (11, 5.0),
        (12, 6.0),
        (13, 2.25),
        (14, 1.25),
        (15, nov23 + 0.5),
        (16, 0.75),
        (17, 0.01),
        (18, nov23),
        (19, 8.0),
        (99, 9.0),
        (1, 0.25),
    ];
    let wb = date_book(false, &cells);
    let g = wb.sheet(0).unwrap();
    let got: Vec<String> = (0..cells.len() as u32).map(|r| match g.get(r, 0) {
        Value::Date(_) => format!("D {}", g.get(r, 0).text()),
        v => format!("N {}", v.text()),
    }).collect();
    assert_eq!(
        got,
        [
            "D 2026-11-23",
            "D 1900-01-01",          // serial 1
            "D 1900-02-28",          // serial 59
            "D 1900-02-28",          // serial 60, Excel's 1900-02-29
            "D 1900-03-01",          // serial 61
            "D 2026-11-23 14:05:00", // yyyy\-mm\-dd\ hh:mm
            "D 1899-12-31 12:00:00", // [h]:mm keeps 1.5 days (36 hours) as it is
            "N 7",                   // "Datum" 0
            "D 2026-11-23",          // [$-407]dd/mm/yyyy;@
            "N 12345",               // 0.00E+00
            "N 3",                   // [Red]#,##0.00\ _€
            "D 12:00:00",            // h:mm
            "D 2026-11-23",          // mmmm
            "N 4",                   // General
            "N 5",                   // #,##0\ \d
            "N 6",                   // 0 "h" * 0
            "D 1900-01-01 06:00:00", // [mm] in a later section: elapsed, 2.25 kept
            "D 1899-12-31 06:00:00", // id 46 [h]:mm:ss, 1.25 kept
            "D 2026-11-23 12:00:00", // id 22
            "D 18:00:00",            // id 20 h:mm
            "D 00:14:24",            // id 45 mm:ss
            "D 2026-11-23",          // id 30, an East Asian date
            "N 8",                   // id 49 @
            "N 9",                   // a style past cellXfs
            "D 06:00:00",            // a date format on 0.25: a time of day
        ]
    );
    assert_eq!(g.get(6, 0), &Value::Date(1.5));
    assert_eq!(g.get(16, 0), &Value::Date(2.25));
}

#[test]
fn dates_1904() {
    let nov23 = day(2026, 11, 23);
    let wb = date_book(true, &[(1, 0.0), (1, nov23 - 1462.0), (16, 0.5), (16, nov23 - 1462.0 + 0.5), (2, 1.25), (3, 1.5)]);
    assert!(wb.date1904());
    let g = wb.sheet(0).unwrap();
    assert_eq!(g.get(0, 0).text(), "1904-01-01");
    assert_eq!(g.get(1, 0), &Value::Date(nov23));
    assert_eq!(g.get(2, 0), &Value::Date(0.5));
    assert_eq!(g.get(3, 0), &Value::Date(nov23 + 0.5));
    assert_eq!(g.get(4, 0).text(), "1904-01-02 06:00:00");
    assert_eq!(g.get(5, 0), &Value::Date(1.5));
}

#[test]
fn positions() {
    let b = Book::new().sheet(
        "P",
        concat!(
            r#"<row><c><v>1</v></c><c><v>2</v></c><c r="E1"><v>5</v></c><c><v>6</v></c></row>"#,
            r#"<row><c><v>10</v></c></row>"#,
            r#"<row r="5"><c r="C5"><v>3</v></c><c r="A5"><v>1</v></c><c r="B5"><v>2</v></c></row>"#,
            r#"<row><c><v>7</v></c></row>"#,
            r#"<row r="3"><c r="B3"><v>33</v></c></row>"#,
            r#"<row r="5"><c r="A5"><v>100</v></c><c r="D5"/></row>"#,
            r#"<row r="4"><c r="A4"><v>1</v></c><c r="A4"/></row>"#,
            r#"<row r="7"><c r="B7"><v>1</v></c><c r="B7"><v>2</v></c><c r="B7"><v>3</v></c></row>"#,
        ),
    );
    let wb = b.open().expect("open");
    let g = wb.sheet(0).unwrap();
    let all: Vec<(u32, Vec<(u32, f64)>)> = g.rows().map(|(r, c)| (r, c.iter().map(|(k, v)| (*k, v.number().unwrap_or(-1.0))).collect())).collect();
    assert_eq!(
        all,
        [
            (0, vec![(0, 1.0), (1, 2.0), (4, 5.0), (5, 6.0)]),
            (1, vec![(0, 10.0)]),
            (2, vec![(1, 33.0)]),
            (4, vec![(0, 100.0), (1, 2.0), (2, 3.0)]),
            (5, vec![(0, 7.0)]),
            (6, vec![(1, 3.0)]),
        ]
    );
    assert_eq!(g.get(3, 0), &Value::Empty);
    assert_eq!(g.used(), Some(Area { sheet: Some(0), first: CellRef::new(0, 0), last: CellRef::new(6, 5) }));
    assert_eq!(g.row(4).len(), 3);
}

#[test]
fn strict_workbook() {
    let mut b = Book::new();
    b.strict = true;
    let mut b = b
        .sheet("Streng", r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><v>46349</v></c></row>"#)
        .strings("<si><t>strikt</t></si>")
        .styles("", &[0, 14]);
    b.names = String::from(r#"<definedName name="N">Streng!$A$1</definedName>"#);
    let wb = b.open().expect("open");
    assert_eq!(wb.sheets()[0].kind, SheetKind::Worksheet);
    let g = wb.sheet(0).unwrap();
    assert_eq!(g.get(0, 0), &text("strikt"));
    assert_eq!(g.get(0, 1).text(), "2026-11-23");
    assert_eq!(wb.names()[0].area.map(|a| a.sheet), Some(Some(0)));
}

#[test]
fn xlsm_and_tables() {
    let mut b = Book::new().sheet(
        "Daten",
        concat!(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="C1" t="inlineStr"><is><t>name</t></is></c></row>"#,
            r#"<row r="2"><c r="A2" t="inlineStr"><is><t>a</t></is></c><c r="B2"><v>1</v></c><c r="C2"><v>2</v></c></row>"#,
            r#"<row r="3"><c r="A3" t="inlineStr"><is><t>b</t></is></c><c r="B3"><v>3</v></c></row>"#,
            r#"<row r="4"><c r="A4" t="inlineStr"><is><t>Summe</t></is></c><c r="B4"><f>SUM(B2:B3)</f><v>4</v></c></row>"#,
        ),
    );
    b.main_type = XLSM_MAIN;
    b.rels = String::from(r#"<Relationship Id="rId9" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#);
    b.extra.push((String::from("xl/vbaProject.bin"), vec![0xD0, 0xCF, 0x11, 0xE0, 1, 2, 3]));
    b.extra.push((
        String::from("xl/worksheets/_rels/sheet1.xml.rels"),
        format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{R_T}/table\" Target=\"../tables/table1.xml\"/></Relationships>").into_bytes(),
    ));
    b.extra.push((
        String::from("xl/tables/table1.xml"),
        format!("{DECL}<table xmlns=\"{MAIN_T}\" id=\"1\" name=\"Tabelle1\" displayName=\"Liste\" ref=\"A1:C4\" totalsRowCount=\"1\"><autoFilter ref=\"A1:C3\"/></table>").into_bytes(),
    ));
    let wb = b.open().expect("xlsm opens");
    let t = &wb.names()[0];
    assert_eq!((t.name.as_str(), t.table, t.scope, t.formula.as_str()), ("Liste", true, None, "Daten!A1:C4"));
    let area = t.area.unwrap();
    assert_eq!(area, Area { sheet: Some(0), first: CellRef::new(0, 0), last: CellRef::new(2, 2) });
    let tab = wb.table(&area, true).unwrap();
    assert_eq!(names(&tab), ["Name", "F2", "name1"]);
    assert_eq!(tab.rows, [vec![text("a"), Value::Number(1.0), Value::Number(2.0)], vec![text("b"), Value::Number(3.0), Value::Empty]]);
    // a whole-column reference gives the rows that hold data
    let cols = wb.table(&wb.area("Daten!$A:$B").unwrap(), true).unwrap();
    assert_eq!(cols.rows.len(), 3);
    assert_eq!(cols.columns.len(), 2);
    // an area past the used range, one without a sheet
    assert_eq!(wb.table(&wb.area("Daten!F10:G20").unwrap(), true).unwrap(), Table::default());
    assert_eq!(wb.table(&parse_area("A1:B2", &[] as &[&str]).unwrap(), true).err(), Some(Error::Missing("the area's sheet")));
    // leading empty rows and columns of an area stay
    let t = wb.table(&wb.area("Daten!B3:D9").unwrap(), false).unwrap();
    assert_eq!((names(&t), t.rows.len()), (vec!["F1", "F2"], 2));
}

// a workbook may be read on another thread and kept beside the document
#[test]
fn workbook_traits() {
    fn send_sync_clone<T: Send + Sync + Clone>() {}
    send_sync_clone::<Workbook>();
    send_sync_clone::<crate::xlsx::Grid>();
    send_sync_clone::<Table>();
    send_sync_clone::<csv::Csv>();
}

// ------------------------------------------------------------------ hostile workbooks

fn opened(b: &Book, limits: Limits) -> Result<Workbook, Error> {
    Workbook::open(&b.zip(), limits)
}

#[test]
fn hostile_zip_and_package() {
    // not a ZIP at all, an empty file, a truncated archive
    assert_eq!(Workbook::open(b"Name;Wert\n", Limits::default()).err(), Some(Error::NotXlsx));
    assert_eq!(Workbook::open(b"", Limits::default()).err(), Some(Error::NotXlsx));
    assert_eq!(Workbook::open(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1], Limits::default()).err(), Some(Error::NotXlsx));
    let z = Book::new().sheet("A", "").zip();
    assert_eq!(Workbook::open(&z[..z.len() / 2], Limits::default()).err(), Some(Error::Archive(archive::Error::Invalid)));
    // a ZIP bomb by ratio: 4 MiB of rows that DEFLATE packs more than 200 times
    let rows = "<row/>".repeat(700_000);
    let bomb = Book::new().sheet("Bombe", &rows);
    assert_eq!(opened(&bomb, Limits::default()).err(), Some(Error::Archive(archive::Error::Limit)));
    let roomy = Limits { archive: archive::Limits { ratio: 1032, ..Limits::DEFAULT.archive }, ..Limits::DEFAULT };
    assert!(opened(&bomb, roomy).unwrap().sheet(0).unwrap().is_empty());
    // archive limits: too many parts, too large a part
    let small = Limits { archive: archive::Limits { entries: 3, ..Limits::DEFAULT.archive }, ..Limits::DEFAULT };
    assert_eq!(opened(&bomb, small).err(), Some(Error::Archive(archive::Error::Limit)));
    // no officeDocument relationship: not a workbook
    let mut e = Book::new().sheet("A", "").entries();
    for x in &mut e {
        if x.name == "_rels/.rels" {
            x.data = format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{R_T}/extended-properties\" Target=\"xl/workbook.xml\"/></Relationships>").into_bytes();
        }
    }
    assert_eq!(Workbook::open(&zip(&e), Limits::default()).err(), Some(Error::NotXlsx));
    // another main part: a document
    let mut e = Book::new().sheet("A", "").entries();
    for x in &mut e {
        if x.name == "xl/workbook.xml" {
            x.data = format!("{DECL}<document xmlns=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"/>").into_bytes();
        }
    }
    assert_eq!(Workbook::open(&zip(&e), Limits::default()).err(), Some(Error::NotXlsx));
    // a relationship to a part that is not there: the package is refused
    let mut e = Book::new().sheet("A", "").entries();
    e.retain(|x| x.name != "xl/worksheets/sheet1.xml");
    assert!(matches!(Workbook::open(&zip(&e), Limits::default()).err(), Some(Error::Package(_))));
    // workbook.xml missing altogether
    let mut e = Book::new().sheet("A", "").entries();
    e.retain(|x| x.name != "xl/workbook.xml");
    assert!(matches!(Workbook::open(&zip(&e), Limits::default()).err(), Some(Error::Package(_))));
}

#[test]
fn hostile_rels() {
    // the sheet's relationship names the workbook itself (a cycle): read as a worksheet, refused
    let mut b = Book::new();
    b.sheets.push(Sheet { name: String::from("Kreis"), state: "", kind: "worksheet", xml: ws("") });
    let mut e = b.entries();
    for x in &mut e {
        if x.name == "xl/_rels/workbook.xml.rels" {
            x.data = format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{R_T}/worksheet\" Target=\"workbook.xml\"/><Relationship Id=\"rId2\" Type=\"{R_T}/worksheet\" Target=\"/xl/worksheets/sheet1.xml\"/></Relationships>").into_bytes();
        }
    }
    let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
    assert_eq!(wb.sheet(0).err(), Some(Error::Invalid("a worksheet part whose root is no <worksheet>")));
    // the shared strings' relationship names a worksheet
    let mut b = Book::new().sheet("A", r#"<row r="1"><c r="A1"><v>1</v></c></row>"#);
    b.rels = String::from(r#"<Relationship Id="rId50" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="worksheets/sheet1.xml"/>"#);
    assert_eq!(b.open().err(), Some(Error::Invalid("the shared strings part's root is no <sst>")));
    // a sheet whose r:id names no relationship, two sheets on one part, a table relationship back
    // to the workbook (left out)
    let mut b = Book::new().sheet("A", r#"<row r="1"><c r="A1"><v>1</v></c></row>"#);
    b.extra.push((
        String::from("xl/worksheets/_rels/sheet1.xml.rels"),
        format!("{DECL}<Relationships xmlns=\"{PKG_R}\"><Relationship Id=\"rId1\" Type=\"{R_T}/table\" Target=\"../workbook.xml\"/></Relationships>").into_bytes(),
    ));
    let mut e = b.entries();
    for x in &mut e {
        if x.name == "xl/workbook.xml" {
            x.data = format!("{DECL}<workbook xmlns=\"{MAIN_T}\" xmlns:r=\"{R_T}\"><sheets><sheet name=\"A\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"B\" sheetId=\"2\" r:id=\"rId1\"/><sheet name=\"C\" sheetId=\"3\" r:id=\"rId77\"/><sheet name=\"D\" sheetId=\"4\"/></sheets></workbook>").into_bytes();
        }
    }
    let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
    assert_eq!(wb.sheets().iter().map(|s| s.kind).collect::<Vec<_>>(), [SheetKind::Worksheet, SheetKind::Worksheet, SheetKind::Other, SheetKind::Other]);
    assert_eq!(wb.sheet(1).unwrap().get(0, 0), &Value::Number(1.0));
    assert!(wb.sheet(2).unwrap().is_empty() && wb.sheet(3).unwrap().is_empty());
    assert!(wb.names().is_empty());
}

#[test]
fn hostile_sheets() {
    // a huge dimension and one cell at each corner: two cells, the table of it is over the limit
    let corners = Book::new().sheet(
        "Ecken",
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row>"#,
    );
    let mut e = corners.entries();
    for x in &mut e {
        if x.name == "xl/worksheets/sheet1.xml" {
            x.data = String::from_utf8(x.data.clone()).unwrap().replace("<dimension ref=\"A1\"/>", "<dimension ref=\"A1:XFD1048576\"/>").into_bytes();
        }
    }
    let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
    let g = wb.sheet(0).unwrap();
    assert_eq!(g.cells(), 2);
    assert_eq!(g.used().map(|a| a.last), Some(CellRef::new(MAX_ROWS - 1, MAX_COLS - 1)));
    assert_eq!(wb.sheet_table(0, true).err(), Some(Error::Limit("cells")));
    assert_eq!(wb.table(&wb.area("Ecken!A1:XFD1048576").unwrap(), false).err(), Some(Error::Limit("cells")));
    // rows and columns past Excel's sheet and past smaller limits
    let past_row = Book::new().sheet("R", r#"<row r="1048577"><c><v>1</v></c></row>"#);
    assert_eq!(past_row.open().unwrap().sheet(0).err(), Some(Error::Limit("rows")));
    let past_col = Book::new().sheet("C", r#"<row r="1"><c r="XFE1"><v>1</v></c></row>"#);
    assert_eq!(past_col.open().unwrap().sheet(0).err(), Some(Error::Limit("cols")));
    let cell_row = Book::new().sheet("C", r#"<row r="1"><c r="A1048577"><v>1</v></c></row>"#);
    assert_eq!(cell_row.open().unwrap().sheet(0).err(), Some(Error::Limit("rows")));
    let many_rows = Book::new().sheet("M", &"<row><c><v>1</v></c></row>".repeat(11));
    let ten = Limits { rows: 10, ..Limits::DEFAULT };
    assert_eq!(opened(&many_rows, ten).unwrap().sheet(0).err(), Some(Error::Limit("rows")));
    let many_cols = Book::new().sheet("M", &format!("<row>{}</row>", "<c><v>1</v></c>".repeat(17)));
    let sixteen = Limits { cols: 16, ..Limits::DEFAULT };
    assert_eq!(opened(&many_cols, sixteen).unwrap().sheet(0).err(), Some(Error::Limit("cols")));
    let five = Limits { cells: 5, ..Limits::DEFAULT };
    assert_eq!(opened(&many_rows, five).unwrap().sheet(0).err(), Some(Error::Limit("cells")));
    // shared strings: their number, their bytes, one long string in many cells
    let strings = Book::new().sheet("S", "").strings("<si><t>a</t></si><si><t>b</t></si><si><t>c</t></si>");
    assert_eq!(opened(&strings, Limits { strings: 2, ..Limits::DEFAULT }).err(), Some(Error::Limit("strings")));
    assert_eq!(opened(&strings, Limits { string_bytes: 2, ..Limits::DEFAULT }).err(), Some(Error::Limit("string_bytes")));
    let long = "x".repeat(1000);
    let amplified = Book::new().sheet("S", &"<row><c t=\"s\"><v>0</v></c></row>".repeat(2000)).strings(&format!("<si><t>{long}</t></si>"));
    let mib = Limits { string_bytes: 1 << 20, ..Limits::DEFAULT };
    let wb = opened(&amplified, mib).expect("the table itself is small");
    assert_eq!(wb.sheet(0).err(), Some(Error::Limit("string_bytes")));
    // a shared-string index past the table, one that is no number
    let past = Book::new().sheet("S", r#"<row r="1"><c r="A1" t="s"><v>3</v></c></row>"#).strings("<si><t>a</t></si>");
    assert_eq!(past.open().unwrap().sheet(0).err(), Some(Error::Invalid("a shared string index past the table")));
    let nan = Book::new().sheet("S", r#"<row r="1"><c r="A1" t="s"><v>x</v></c></row>"#).strings("<si><t>a</t></si>");
    assert_eq!(nan.open().unwrap().sheet(0).err(), Some(Error::Invalid("a shared string index")));
    // bad positions
    let bad_row = Book::new().sheet("S", r#"<row r="0"><c><v>1</v></c></row>"#);
    assert_eq!(bad_row.open().unwrap().sheet(0).err(), Some(Error::Invalid("a row number")));
    let bad_ref = Book::new().sheet("S", r#"<row r="1"><c r="1A"><v>1</v></c></row>"#);
    assert_eq!(bad_ref.open().unwrap().sheet(0).err(), Some(Error::Invalid("a cell reference")));
}

#[test]
fn hostile_xml() {
    // deep nesting in a sheet and in the workbook part
    let deep = format!(r#"<row r="1"><c r="A1"><v>1</v></c></row></sheetData><extLst>{}{}</extLst><sheetData>"#, "<x>".repeat(100), "</x>".repeat(100));
    let b = Book::new().sheet("Tief", &deep);
    assert_eq!(b.open().unwrap().sheet(0).err(), Some(Error::Limit("depth")));
    let mut b = Book::new().sheet("A", "");
    b.head = format!("<extLst>{}{}</extLst>", "<x>".repeat(100), "</x>".repeat(100));
    assert_eq!(b.open().err(), Some(Error::Limit("depth")));
    // a DOCTYPE with an entity (a billion laughs needs one): refused before anything is expanded
    let mut e = Book::new().sheet("A", "").entries();
    for x in &mut e {
        if x.name == "xl/worksheets/sheet1.xml" {
            x.data = format!("<?xml version=\"1.0\"?><!DOCTYPE worksheet [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]><worksheet xmlns=\"{MAIN_T}\"><sheetData><row><c t=\"inlineStr\"><is><t>&b;</t></is></c></row></sheetData></worksheet>").into_bytes();
        }
    }
    let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
    assert!(matches!(wb.sheet(0).err(), Some(Error::Xml { what, .. }) if what.contains("DOCTYPE")));
    // an undefined entity, a broken tag, an unclosed element, wrong encoding
    for body in [
        format!("<worksheet xmlns=\"{MAIN_T}\"><sheetData><row><c t=\"inlineStr\"><is><t>&nbsp;</t></is></c></row></sheetData></worksheet>"),
        format!("<worksheet xmlns=\"{MAIN_T}\"><sheetData><row><c</row></sheetData></worksheet>"),
        format!("<worksheet xmlns=\"{MAIN_T}\"><sheetData><row>"),
        format!("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><worksheet xmlns=\"{MAIN_T}\"/>"),
    ] {
        let mut e = Book::new().sheet("A", "").entries();
        for x in &mut e {
            if x.name == "xl/worksheets/sheet1.xml" {
                x.data = body.clone().into_bytes();
            }
        }
        let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
        assert!(matches!(wb.sheet(0).err(), Some(Error::Xml { .. })), "{body}");
    }
    // UTF-16 parts read as well
    let mut e = Book::new().sheet("A", "").entries();
    for x in &mut e {
        if x.name == "xl/worksheets/sheet1.xml" {
            let s = format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?><worksheet xmlns=\"{MAIN_T}\"><sheetData><row><c t=\"inlineStr\"><is><t>sechzehn</t></is></c></row></sheetData></worksheet>");
            let mut d = vec![0xFF, 0xFE];
            for u in s.encode_utf16() {
                d.extend_from_slice(&u.to_le_bytes());
            }
            x.data = d;
        }
    }
    let wb = Workbook::open(&zip(&e), Limits::default()).expect("open");
    assert_eq!(wb.sheet(0).unwrap().get(0, 0), &text("sechzehn"));
}

// ------------------------------------------------------------------ CSV

fn rec(csv: &csv::Csv) -> Vec<Vec<&str>> {
    csv.records.iter().map(|r| r.iter().map(String::as_str).collect()).collect()
}

fn read(bytes: &[u8]) -> csv::Csv {
    csv::read(bytes, &Options::default()).expect("csv")
}

#[test]
fn csv_grammar() {
    let c = read(b"a,\"b,1\",\"c \"\"q\"\"\"\r\n\"x\r\ny\",,z\nlast,\"open");
    assert_eq!(c.delimiter, b',');
    assert_eq!(rec(&c), [vec!["a", "b,1", "c \"q\""], vec!["x\ny", "", "z"], vec!["last", "open"]]);
    // CR, LF, CR LF mixed; a final line break starts no record; an empty line is one empty field
    let c = read(b"a;b\rc;d\n\ne;f\r\n");
    assert_eq!(rec(&c), [vec!["a", "b"], vec!["c", "d"], vec![""], vec!["e", "f"]]);
    // a quote inside an unquoted field is kept, text after a closing quote too
    let c = read(b"ab\"cd,\"ef\"gh,\"i\"\"\"j\n");
    assert_eq!(rec(&c), [vec!["ab\"cd", "efgh", "i\"j"]]);
    // a delimiter at the end of the line: one more empty field; a line break in quotes (CR alone)
    let c = read(b"a,b,\n\"1\r2\",3,\n");
    assert_eq!(rec(&c), [vec!["a", "b", ""], vec!["1\n2", "3", ""]]);
    assert!(read(b"").records.is_empty());
    assert_eq!(rec(&read(b"\n")), [vec![""]]);
    assert_eq!(rec(&read(b"x")), [vec!["x"]]);
}

#[test]
fn csv_encodings() {
    // Windows-1252: umlauts, the euro sign at 0x80, an open byte as its C1 control
    let c = read(b"Gr\xf6\xdfe;Preis\nM\xe4rz;5 \x80\n\x81;\x9f\n");
    assert_eq!(c.encoding, Encoding::Windows1252);
    assert_eq!(rec(&c), [vec!["Größe", "Preis"], vec!["März", "5 €"], vec!["\u{81}", "Ÿ"]]);
    // UTF-8 with and without BOM
    let c = read("\u{feff}Größe,€\n".as_bytes());
    assert_eq!((c.encoding, rec(&c)), (Encoding::Utf8Bom, vec![vec!["Größe", "€"]]));
    let c = read("Größe,€\n".as_bytes());
    assert_eq!((c.encoding, rec(&c)), (Encoding::Utf8, vec![vec!["Größe", "€"]]));
    // UTF-16 LE with BOM and TAB (Excel's "Unicode text"), BE, LE without BOM
    let text = "Name\tWert\r\nÄpfel\t3\r\n";
    let mut le = vec![0xFF, 0xFE];
    let mut be = vec![0xFE, 0xFF];
    let mut bare = Vec::new();
    for u in text.encode_utf16() {
        le.extend_from_slice(&u.to_le_bytes());
        be.extend_from_slice(&u.to_be_bytes());
        bare.extend_from_slice(&u.to_le_bytes());
    }
    for (bytes, enc) in [(&le, Encoding::Utf16Le), (&be, Encoding::Utf16Be), (&bare, Encoding::Utf16Le)] {
        let c = read(bytes);
        assert_eq!((c.encoding, c.delimiter), (enc, b'\t'));
        assert_eq!(rec(&c), [vec!["Name", "Wert"], vec!["Äpfel", "3"]]);
    }
    // a forced encoding
    let opts = Options { encoding: Some(Encoding::Windows1252), ..Options::default() };
    assert_eq!(rec(&csv::read("ä".as_bytes(), &opts).unwrap()), [vec!["Ã¤"]]);
    let opts = Options { encoding: Some(Encoding::Utf8), ..Options::default() };
    assert_eq!(rec(&csv::read(b"\xe4", &opts).unwrap()), [vec!["\u{fffd}"]]);
    assert_eq!(csv::decode(b"\x80"), (String::from("€"), Encoding::Windows1252));
}

#[test]
fn csv_delimiters() {
    assert_eq!(csv::detect_delimiter("a;b;c\n1;2;3\n"), b';');
    assert_eq!(csv::detect_delimiter("a,b,c\n1,2,3\n"), b',');
    assert_eq!(csv::detect_delimiter("a\tb\n1\t2\n"), b'\t');
    assert_eq!(csv::detect_delimiter("a|b\n1|2\n"), b'|');
    // German numbers: ; is consistent, , is not (or less so)
    assert_eq!(csv::detect_delimiter("Name;Wert\nA;1,5\nB;2,75\n"), b';');
    assert_eq!(csv::detect_delimiter("1,5;2,5\n3,5;4,5\n"), b';');
    // quoted delimiters and line breaks do not count
    assert_eq!(csv::detect_delimiter("\"a;b\",c\n\"d\n;e\",f\n"), b',');
    // none consistent: the one in most records
    assert_eq!(csv::detect_delimiter("a;b\nc;d;e\nf,g\n"), b';');
    assert_eq!(csv::detect_delimiter("nichts\n"), b',');
    assert_eq!(csv::detect_delimiter(""), b',');
    // Excel's sep= line: honoured and removed; a delimiter of the options wins
    let c = read(b"sep=|\r\na|b;c\n");
    assert_eq!((c.delimiter, rec(&c)), (b'|', vec![vec!["a", "b;c"]]));
    let c = csv::read(b"SEP=;\na;b,c\n", &Options { delimiter: Some(b','), ..Options::default() }).unwrap();
    assert_eq!((c.delimiter, rec(&c)), (b',', vec![vec!["a;b", "c"]]));
    assert_eq!(rec(&read(b"sep=;")), Vec::<Vec<&str>>::new());
    assert_eq!(csv::detect_delimiter("sep=\t\na,b\n"), b'\t');
    assert_eq!(csv::read(b"a", &Options { delimiter: Some(b'"'), ..Options::default() }).err(), Some(csv::Error::Options));
    assert_eq!(csv::read(b"a", &Options { delimiter: Some(b'\n'), ..Options::default() }).err(), Some(csv::Error::Options));
}

#[test]
fn csv_limits() {
    let lim = |l: CsvLimits| Options { limits: l, ..Options::default() };
    let d = CsvLimits::DEFAULT;
    assert_eq!(csv::read(b"abc", &lim(CsvLimits { bytes: 2, ..d })).err(), Some(csv::Error::Limit("bytes")));
    assert_eq!(csv::read(b"a\nb\nc\n", &lim(CsvLimits { records: 2, ..d })).err(), Some(csv::Error::Limit("records")));
    assert!(csv::read(b"a\nb\n", &lim(CsvLimits { records: 2, ..d })).is_ok());
    assert_eq!(csv::read(b"a,b,c\n", &lim(CsvLimits { fields: 2, ..d })).err(), Some(csv::Error::Limit("fields")));
    assert_eq!(csv::read(b"a,b\nc,d\n", &lim(CsvLimits { cells: 3, ..d })).err(), Some(csv::Error::Limit("cells")));
    assert_eq!(csv::read(b"abcd,e\n", &lim(CsvLimits { field_bytes: 3, ..d })).err(), Some(csv::Error::Limit("field_bytes")));
    assert_eq!(csv::read(b"\"ab\"\"cd\"\n", &lim(CsvLimits { field_bytes: 3, ..d })).err(), Some(csv::Error::Limit("field_bytes")));
    // a file of delimiters only: bounded by the fields, not by memory
    let commas = vec![b','; 1 << 20];
    assert_eq!(csv::read(&commas, &Options::default()).err(), Some(csv::Error::Limit("fields")));
}

#[test]
fn csv_tables() {
    let c = read(b"Name; Wert ;;Name;Datum;Ja\nA;1,5;x;007;23.11.2026;wahr\nB;1.234,5;;-3;2026-11-23T14:05;FALSE\nC;1.5;;1e3;1.2.2026 9:05\n;;;;;\n\n");
    let t = csv::to_table(&c, true, true);
    assert_eq!(names(&t), ["Name", "Wert", "F3", "Name1", "Datum", "Ja"]);
    assert_eq!(
        t.rows,
        [
            vec![text("A"), Value::Number(1.5), text("x"), text("007"), Value::Date(day(2026, 11, 23)), Value::Bool(true)],
            vec![text("B"), Value::Number(1234.5), Value::Empty, Value::Number(-3.0), Value::Date(serial(&Civil::new(2026, 11, 23, 14, 5, 0))), Value::Bool(false)],
            vec![text("C"), Value::Number(1.5), Value::Empty, Value::Number(1000.0), Value::Date(day(2026, 2, 1) + (9.0 * 60.0 + 5.0) / 1440.0), Value::Empty],
        ]
    );
    // without inference everything is text; without a header the columns are F1 ...
    let t = csv::to_table(&c, false, false);
    assert_eq!(names(&t), ["F1", "F2", "F3", "F4", "F5", "F6"]);
    assert_eq!(t.rows[1][1], text("1,5"));
    assert_eq!(t.rows.len(), 4);
    // with , as delimiter German numbers stay text
    let c = read(b"a,b\n\"1,5\",\" 2.5 \"\n0.5,-0\n.5,+7\n1e,0x10\n");
    let t = csv::to_table(&c, true, true);
    assert_eq!(t.rows[0], [text("1,5"), Value::Number(2.5)]);
    assert_eq!(t.rows[1], [Value::Number(0.5), Value::Number(-0.0)]);
    assert_eq!(t.rows[2], [Value::Number(0.5), Value::Number(7.0)]);
    assert_eq!(t.rows[3], [text("1e"), text("0x10")]);
    // ragged records: padded to the widest, columns past the header kept when they hold data,
    // dropped when they are empty everywhere (a delimiter at the end of the data lines)
    let c = read(b"a,b\n1\n2,3,4\n");
    let t = csv::to_table(&c, true, false);
    assert_eq!(names(&t), ["a", "b", "F3"]);
    assert_eq!(t.rows, [vec![text("1"), Value::Empty, Value::Empty], vec![text("2"), text("3"), text("4")]]);
    let c = read(b"a,b\n1,2,\n3,4,\n");
    assert_eq!(names(&csv::to_table(&c, true, false)), ["a", "b"]);
    // no numbers that are not: inf, nan, hex, a lone sign, German groups that are not of three
    let c = read(b"x;y;z;w;v\ninf;nan;-;1.23,4;12.345.678,9\n");
    let t = csv::to_table(&c, true, true);
    assert_eq!(t.rows[0], [text("inf"), text("nan"), text("-"), text("1.23,4"), Value::Number(12_345_678.9)]);
    // dates that are not: day 31 of November, a two-digit year
    let c = read(b"d\n31.11.2026\n1.2.26\n2026-02-30\n");
    assert!(csv::to_table(&c, true, true).rows.iter().all(|r| matches!(r[0], Value::Text(_))));
    assert_eq!(csv::to_table(&read(b""), true, true), Table::default());
}
