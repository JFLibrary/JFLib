// Table structure, rich cell stories, strict interchange and page geometry.
use alloc::{format, string::String, vec};
use doc_core::*;
#[path="table_merge_checks.rs"]
mod merged;
#[path="table_style_checks.rs"]
mod styles;
#[path="table_style_graph_checks.rs"]
mod style_graph;
#[path="cell_border_clipboard_checks.rs"] mod borders_clipboard;
#[path="nested_table_checks.rs"] mod nested;
#[path="floating_table_checks.rs"] mod floating;
#[path="floating_pages_checks.rs"] mod floating_pages;
#[path="row_break_checks.rs"] mod row_breaks;
#[path="row_height_checks.rs"] mod row_heights;
struct Fixed;
impl doc_layout::Measure for Fixed {
    fn width(&mut self, s: &str, _: ResolvedFormat) -> i32 {
        s.chars().count() as i32 * 100
    }
    fn metrics(&mut self, _: ResolvedFormat) -> doc_layout::Metrics {
        doc_layout::Metrics {
            ascent: 160,
            descent: 40,
            height: 220,
        }
    }
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($x:expr,$name:expr) => {
            if $x {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL tables: {}\n", $name).as_bytes());
            }
        };
    }
    let mut e = Editor::new(Document::default()).unwrap();
    let id = e
        .apply(Edit::InsertTable {
            paragraph: 1,
            offset: 0,
            rows: 2,
            columns: 3,
        })
        .unwrap();
    let t = e.document().table(id).unwrap().clone();
    let a = t.cell(0, 0).unwrap().paragraphs[0].id;
    let b = t.cell(0, 1).unwrap().paragraphs[0].id;
    check!(
        t.rows.len() == 2 && t.columns.len() == 3 && e.document().all_paragraphs().count() == 7,
        "shape and unique cell paragraphs"
    );
    check!(e.document().validate().is_ok(), "valid table document");
    check!(
        e.document().story_projection(a).unwrap().paragraphs.len() == 1,
        "cell owns one story"
    );
    check!(
        e.apply(Edit::JoinParagraphs {
            first: a,
            second: b
        })
        .is_err(),
        "cell boundaries cannot be joined"
    );
    e.apply(Edit::ReplaceStoryText {
        paragraph: a,
        start: 0,
        end: 0,
        text: String::from("Äpfel\nBirnen"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().cell_for_paragraph(a).unwrap().paragraphs.len() == 2,
        "Enter creates cell paragraphs"
    );
    check!(
        e.document().story_projection(a).unwrap().text == "Äpfel\nBirnen",
        "cell story text"
    );
    e.apply(Edit::FormatStoryText {
        paragraph: a,
        start: 0,
        end: 6,
        format: TextFormat {
            bold: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    check!(
        e.document().story_character_format(a, 0, 6).unwrap().bold == Some(true),
        "rich format in cell"
    );
    let fragment = e.document().copy_story_range(a, 0, 13).unwrap();
    check!(
        fragment.document.tables.is_empty() && fragment.document.paragraphs.len() == 2,
        "cell text clipboard preserves paragraphs"
    );
    check!(
        e.apply(Edit::ReplaceStoryFragment {
            paragraph: b,
            start: 0,
            end: 0,
            fragment
        })
        .is_ok(),
        "paste formatted cell story"
    );
    check!(
        e.document().plain_text().contains("Äpfel"),
        "word counts and text include cells"
    );
    check!(
        e.document()
            .find_text("Birnen", SearchOptions::default())
            .unwrap()
            .len()
            == 2,
        "search includes cells"
    );
    let snapshot = e.document().clone();
    let revision = e.revision();
    check!(
        e.apply(Edit::ChangeTable {
            table: id,
            change: TableChange::Columns(vec![5, 5, 5])
        })
        .is_err(),
        "too narrow cells rejected"
    );
    check!(
        e.document() == &snapshot && e.revision() == revision,
        "invalid table edit is atomic"
    );
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::InsertRow(1),
    })
    .unwrap();
    check!(
        e.document().table(id).unwrap().rows.len() == 3 && e.document().paragraph(a).is_some(),
        "insert row retains identities"
    );
    e.undo();
    check!(e.document() == &snapshot, "row insertion undo");
    e.redo();
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::InsertColumn(1),
    })
    .unwrap();
    check!(
        e.document().table(id).unwrap().columns.len() == 4,
        "insert column"
    );
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::RemoveColumn(1),
    })
    .unwrap();
    check!(
        e.document().table(id).unwrap().columns.len() == 3 && e.document().paragraph(b).is_some(),
        "remove column preserves neighbours"
    );
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::RemoveRow(1),
    })
    .unwrap();
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::Cell {
            row: 0,
            column: 0,
            fill: Some(0xc2e4f2),
            alignment: CellAlignment::Center,
        },
    })
    .unwrap();
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::RowHeight {
            row: 0,
            height: 1000,
        },
    })
    .unwrap();
    let bytes = doc_io::encode_docx(e.document()).unwrap();
    let decoded = doc_io::decode_docx(&bytes);
    check!(
        decoded.as_ref() == Ok(e.document()),
        "DOCX table and identity roundtrip"
    );
    check!(
        decoded
            .as_ref()
            .is_ok_and(|d| doc_io::encode_docx(d).is_ok_and(|b| b == bytes)),
        "DOCX bytes stable"
    );
    check!(
        doc_io::DocxFile::open(&bytes).is_ok_and(|f| f.report().mode == doc_io::DocxMode::Editable),
        "table session stays editable"
    );
    let html = doc_io::clipboard_html_encode(e.document()).unwrap();
    let html = core::str::from_utf8(&html).unwrap();
    check!(
        html.contains("<table") && html.contains("Äpfel") && html.contains("C2E4F2"),
        "HTML table export"
    );
    let rtf = doc_io::clipboard_rtf_encode(e.document()).unwrap();
    let rtf = core::str::from_utf8(&rtf).unwrap();
    check!(
        rtf.contains("\\trowd")
            && rtf.contains("\\cellx")
            && rtf.contains("\\intbl")
            && rtf.contains("Birnen"),
        "RTF table export"
    );
    let layout = doc_layout::paginate(e.document(), &mut Fixed).unwrap();
    check!(
        layout.pages[0].cells.len() == 6,
        "row layout has every cell"
    );
    check!(
        layout.pages[0].cells[0].rect[3] == 1000,
        "minimum row height"
    );
    let line = layout.pages[0]
        .lines
        .iter()
        .find(|l| l.paragraph == a)
        .unwrap();
    let cell = &layout.pages[0].cells[0];
    check!(
        line.x > cell.rect[0] && line.top > cell.rect[1],
        "cell content respects padding"
    );
    check!(
        doc_view::table_cell_at(&layout.pages[0], cell.rect[0] + 50, cell.rect[1] + 50)
            .is_some_and(|c| c.cell == cell.cell),
        "shared hit testing"
    );
    e.apply(Edit::ChangeTable {
        table: id,
        change: TableChange::Delete,
    })
    .unwrap();
    check!(
        e.document().tables.is_empty() && e.document().paragraph(a).is_none(),
        "delete removes table and cells"
    );
    e.undo();
    check!(
        e.document().paragraph(a).is_some(),
        "delete undo restores cell content"
    );
    let snapshot = e.document().clone();
    check!(
        e.apply(Edit::RemoveParagraph(t.before)).is_err() && e.document() == &snapshot,
        "table anchor cannot disappear"
    );
    // Paragraph segment offsets after a table must never splice unrelated body paragraphs.
    let mut e = Editor::new(Document::default()).unwrap();
    e.apply(Edit::ReplaceStoryText {
        paragraph: 1,
        start: 0,
        end: 0,
        text: String::from("before\nafter\nlast"),
        paragraph_breaks: true,
    })
    .unwrap();
    let after = e.document().paragraphs[1].id;
    e.apply(Edit::InsertTable {
        paragraph: after,
        offset: 0,
        rows: 1,
        columns: 1,
    })
    .unwrap();
    check!(
        e.document().story_projection(1).unwrap().text == "before",
        "table divides main text stories"
    );
    e.apply(Edit::ReplaceStoryText {
        paragraph: after,
        start: 0,
        end: 10,
        text: String::from("changed"),
        paragraph_breaks: true,
    })
    .unwrap();
    check!(
        e.document().paragraph_text(&e.document().paragraphs[0]) == "before"
            && e.document().story_projection(after).unwrap().text == "changed",
        "body edit after table uses correct indices"
    );
    // Tall cell text continues without duplicate or missing text lines across page boundaries.
    let t = e.document().tables[0].id;
    let p = e.document().tables[0].cell(0, 0).unwrap().paragraphs[0].id;
    e.apply(Edit::SetText {
        paragraph: p,
        inline: 0,
        text: "long line\n".repeat(200),
    })
    .unwrap();
    let layout = doc_layout::paginate(e.document(), &mut Fixed).unwrap();
    check!(
        layout.pages.len() > 2
            && layout
                .pages
                .iter()
                .all(|p| !p.cells.is_empty() || !p.lines.is_empty()),
        "tall row pagination"
    );
    check!(
        layout
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .filter(|l| l.paragraph == p)
            .count()
            == 201,
        "row continuations preserve every line"
    );
    check!(
        e.apply(Edit::ChangeTable {
            table: t,
            change: TableChange::RemoveColumn(0)
        })
        .is_err(),
        "last column requires table delete"
    );
    // Scientific fields in cells and body footnotes use the same source registry/layout.
    let mut d = Document::default();
    d.sources.push(Source {
        id: 50,
        title: String::from("Table source"),
        ..Default::default()
    });
    let mut e = Editor::new(d).unwrap();
    let table = e
        .apply(Edit::InsertTable {
            paragraph: 1,
            offset: 0,
            rows: 1,
            columns: 2,
        })
        .unwrap();
    let a = e
        .document()
        .table(table)
        .unwrap()
        .cell(0, 0)
        .unwrap()
        .paragraphs[0]
        .id;
    e.apply(Edit::InsertCitation {
        paragraph: a,
        inline: 0,
        offset: 0,
        items: vec![CitationItem {
            source: 50,
            locator: String::from("7"),
            ..Default::default()
        }],
    })
    .unwrap();
    check!(
        e.document().cited_source_ids() == vec![50] && e.document().source_uses(50).len() == 1,
        "cell citations participate in bibliography"
    );
    let note = e
        .apply(Edit::InsertNote {
            paragraph: 1,
            inline: 0,
            offset: 0,
            kind: NoteKind::Footnote,
            text: String::from("Footnote after table"),
        })
        .unwrap();
    let layout = doc_layout::paginate(e.document(), &mut Fixed).unwrap();
    check!(
        layout.pages.iter().any(|p| !p.cells.is_empty())
            && layout.pages.iter().any(|p| !p.separators.is_empty()),
        "table and body footnote coexist"
    );
    let encoded = doc_io::encode_docx(e.document()).unwrap();
    check!(
        doc_io::decode_docx(&encoded).is_ok_and(|d| d == *e.document() && d.note(note).is_some()),
        "table fields and note identities survive DOCX together"
    );
    let mut malformed = e.document().clone();
    malformed.tables[0].columns[0] = u32::MAX;
    check!(
        malformed.validate().is_err(),
        "overflowing column widths are rejected before arithmetic"
    );
    malformed = e.document().clone();
    malformed.tables[0].rows[0].cells[0].id = 50;
    check!(
        malformed.validate().is_err(),
        "cell/source identities cannot collide"
    );
    malformed = e.document().clone();
    malformed.tables[0].rows[0].cells[0].paragraphs[0]
        .format
        .first_line = 5000;
    check!(
        malformed.validate().is_err(),
        "cell first-line indent cannot escape its column"
    );
    let second = e
        .document()
        .table(table)
        .unwrap()
        .cell(0, 1)
        .unwrap()
        .paragraphs[0]
        .id;
    let bounds = e.document().paragraph_bounds(second);
    check!(
        bounds.0 == e.document().page.left + e.document().table(table).unwrap().columns[0] + 90,
        "second-cell text origin includes preceding columns and padding"
    );
    let mut area = e.document().page;
    area.left = bounds.0;
    area.right = area.width - bounds.1;
    let ruler = doc_view::RulerDraw {
        paragraph_only: true,
        page: area,
        paragraph: ParagraphFormat::default(),
        axis: doc_view::RulerAxis::Horizontal,
        page_start: 0,
        page_width: 794,
        dpi: 96,
        font: 0,
        palette: doc_view::RulerPalette::light(0),
    };
    check!(
        ruler
            .handle_at(ruler.pixel_at(bounds.0 as i32), 1)
            .is_none(),
        "cell ruler margins are locked"
    );
    check!(
        ruler.handle_at(ruler.pixel_at(bounds.0 as i32 + 720), 12)
            == Some(doc_view::RulerHandle::NewTab),
        "cell ruler tab positions use the cell origin"
    );
    let (p,f)=merged::run();
    let (sp,sf)=styles::run();
    let (gp,gf)=style_graph::run();
    let (bp,bf)=borders_clipboard::run();
    let (np,nf)=nested::run();
    let (mp,mf)=floating_pages::run();
    let (fp,ff)=floating::run();
    let (rp,rf)=row_breaks::run();
    let (hp,hf)=row_heights::run();
    (passed+p+sp+gp+bp+np+fp+mp+rp+hp, failed+f+sf+gf+bf+nf+ff+mf+rf+hf)
}
