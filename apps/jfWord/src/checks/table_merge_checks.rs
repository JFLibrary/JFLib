// Behavioral grid, interchange, pagination and sizing regressions.
use super::*;
fn edit(e: &mut Editor, id: Id, change: TableChange) {
    e.apply(Edit::ChangeTable { table: id, change }).unwrap();
}
pub(super) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($v:expr,$label:expr) => {
            if $v {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL merged tables: {}\n", $label).as_bytes());
            }
        };
    }
    let mut e = Editor::new(Document::default()).unwrap();
    let id = e
        .apply(Edit::InsertTable {
            paragraph: 1,
            offset: 0,
            rows: 3,
            columns: 3,
        })
        .unwrap();
    let original = e.document().table(id).unwrap().clone();
    for (r, c, cell) in original.anchors() {
        e.apply(Edit::SetText {
            paragraph: cell.paragraphs[0].id,
            inline: 0,
            text: format!("R{r}C{c}"),
        })
        .unwrap();
    }
    let before = e.document().clone();
    edit(
        &mut e,
        id,
        TableChange::Merge {
            row: 0,
            column: 0,
            rows: 2,
            columns: 2,
        },
    );
    let merged = e.document().clone();
    let t = merged.table(id).unwrap();
    let anchor = t.cell(0, 0).unwrap();
    let cellid = anchor.id;
    let paragraphs: alloc::vec::Vec<_> = anchor.paragraphs.iter().map(|p| p.id).collect();
    check!(
        anchor.row_span == 2 && anchor.column_span == 2 && t.anchors().count() == 6,
        "rectangular span has one owner"
    );
    check!(
        t.cell(1, 1).unwrap().id == cellid && t.origin(1, 1) == Some((0, 0)),
        "covered grid coordinates resolve to owner"
    );
    check!(
        anchor
            .paragraphs
            .iter()
            .map(|p| merged.paragraph_text(p))
            .collect::<alloc::vec::Vec<_>>()
            == vec!["R0C0", "R0C1", "R1C0", "R1C1"],
        "merge retains every paragraph in reading order"
    );
    check!(
        merged.plain_text().split_whitespace().count()
            == before.plain_text().split_whitespace().count(),
        "merging does not duplicate or drop words"
    );
    check!(
        e.undo() && e.document() == &before && e.redo() && e.document() == &merged,
        "merge has exact undo and redo"
    );
    let revision = e.revision();
    check!(
        e.apply(Edit::ChangeTable {
            table: id,
            change: TableChange::Merge {
                row: 0,
                column: 1,
                rows: 2,
                columns: 2
            }
        })
        .is_err()
            && e.revision() == revision
            && e.document() == &merged,
        "partial span intersection rejected atomically"
    );
    edit(&mut e, id, TableChange::Split { row: 1, column: 1 });
    let t = e.document().table(id).unwrap();
    check!(
        t.anchors().count() == 9
            && t.cell(0, 0).unwrap().paragraphs.len() == 4
            && t.cell(1, 1).unwrap().paragraphs.len() == 1,
        "split keeps content in first cell and creates editable neighbors"
    );
    check!(
        e.document().validate().is_ok()
            && e.document().plain_text().split_whitespace().count()
                == before.plain_text().split_whitespace().count(),
        "split identities and content stay valid"
    );
    e.undo();
    edit(&mut e, id, TableChange::InsertRow(1));
    check!(
        e.document().table(id).unwrap().cell(2, 1).unwrap().row_span == 3,
        "row inside span extends it"
    );
    edit(&mut e, id, TableChange::RemoveRow(0));
    check!(
        e.document().table(id).unwrap().cell(0, 0).unwrap().id == cellid
            && e.document().table(id).unwrap().cell(0, 0).unwrap().row_span == 2,
        "deleting anchor row retains spanning cell and text"
    );
    edit(&mut e, id, TableChange::InsertColumn(1));
    check!(
        e.document()
            .table(id)
            .unwrap()
            .cell(1, 2)
            .unwrap()
            .column_span
            == 3,
        "column inside span extends it"
    );
    edit(&mut e, id, TableChange::RemoveColumn(0));
    check!(
        e.document().table(id).unwrap().cell(0, 0).unwrap().id == cellid
            && e.document()
                .table(id)
                .unwrap()
                .cell(0, 0)
                .unwrap()
                .column_span
                == 2,
        "deleting anchor column retains spanning cell identity"
    );
    check!(
        paragraphs
            .iter()
            .all(|p| e.document().paragraph(*p).is_some()),
        "structural span changes retain all merged paragraphs"
    );
    let bytes = doc_io::encode_docx(&merged).unwrap();
    let loaded = doc_io::decode_docx(&bytes).unwrap();
    check!(loaded == merged, "DOCX gridSpan/vMerge model roundtrip");
    check!(
        doc_io::encode_docx(&loaded).unwrap() == bytes,
        "DOCX merged-cell bytes are stable"
    );
    let html = String::from_utf8(doc_io::clipboard_html_encode(&merged).unwrap()).unwrap();
    check!(
        html.contains("rowspan=\"2\" colspan=\"2\"") && html.matches("<td").count() == 6,
        "HTML has one rowspan/colspan cell per anchor"
    );
    let rtf = String::from_utf8(doc_io::clipboard_rtf_encode(&merged).unwrap()).unwrap();
    check!(
        rtf.contains("\\clmgf")
            && rtf.contains("\\clmrg")
            && rtf.contains("\\clvmgf")
            && rtf.contains("\\clvmrg"),
        "RTF exports horizontal and vertical merge markers"
    );
    let layout = doc_layout::paginate(&merged, &mut Fixed).unwrap();
    let page = &layout.pages[0];
    let box_ = page.cells.iter().find(|c| c.cell == cellid).unwrap();
    check!(
        page.cells.len() == 6
            && box_.rect[2] == merged.table(id).unwrap().columns[..2].iter().sum::<u32>() as i32,
        "merged geometry uses summed grid width"
    );
    check!(
        doc_view::table_cell_at(
            page,
            box_.rect[0] + box_.rect[2] - 30,
            box_.rect[1] + box_.rect[3] - 30
        )
        .unwrap()
        .cell
            == cellid,
        "hit testing lower covered portion selects owner"
    );
    let (left, right) = merged.paragraph_bounds(paragraphs[0]);
    check!(
        right - left == box_.rect[2] as u32 - 180,
        "ruler spans the full merged text area"
    );
    let mut malformed = merged.clone();
    malformed.tables[0].rows[1].cells[1].column_span = 1;
    check!(
        malformed.validate().is_err(),
        "malformed covered slot rejected"
    );
    malformed = merged.clone();
    malformed.tables[0].rows[0].cells[0].row_span = 4;
    check!(
        malformed.validate().is_err(),
        "span outside grid rejected before layout"
    );
    let mut long = Editor::new(merged.clone()).unwrap();
    long.apply(Edit::SetText {
        paragraph: paragraphs[0],
        inline: 0,
        text: "Zeile\n".repeat(200),
    })
    .unwrap();
    let layout = doc_layout::paginate(long.document(), &mut Fixed).unwrap();
    check!(
        layout.pages.len() > 1
            && layout
                .pages
                .iter()
                .filter(|p| p.cells.iter().any(|c| c.cell == cellid))
                .count()
                > 1,
        "oversized merged band continues across pages"
    );
    check!(
        layout
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .filter(|l| l.paragraph == paragraphs[0])
            .count()
            == 201,
        "no merged text line lost or duplicated at page boundary"
    );
    let fragments: alloc::vec::Vec<_> = layout
        .pages
        .iter()
        .flat_map(|p| &p.cells)
        .filter(|c| c.cell == cellid)
        .collect();
    check!(
        fragments[..fragments.len() - 1].iter().all(|c| !c.row_end)
            && fragments.last().unwrap().row_end,
        "only the last page fragment owns the resizable row edge"
    );
    let first_page = &layout.pages[0];
    let fragment = fragments[0];
    let mut controller = doc_view::TableController::default();
    let input = ui::UiInput {
        rect: [
            0,
            0,
            long.document().page.width as i32,
            long.document().page.height as i32,
        ],
        x: fragment.rect[0] + 100,
        y: fragment.rect[1] + fragment.rect[3],
        pressed: 1 << ui::BTN_LEFT,
        ..Default::default()
    };
    check!(
        controller
            .update(long.document(), first_page, 0, &input, 0)
            .is_none()
            && !controller.active(),
        "page continuation seam cannot resize an unrelated grid row"
    );
    for p in &layout.pages {
        check!(
            p.lines
                .iter()
                .all(|l| l.top >= long.document().page.top as i32
                    && l.top + l.height
                        <= (long.document().page.height - long.document().page.bottom) as i32),
            "merged lines remain inside page bounds"
        );
    }
    let mut sizing = Editor::new(before.clone()).unwrap();
    let textp = original.cell(0, 0).unwrap().paragraphs[0].id;
    sizing
        .apply(Edit::SetText {
            paragraph: textp,
            inline: 0,
            text: String::from("Eine deutlich breitere Spalte"),
        })
        .unwrap();
    let widths = doc_layout::table_fit(
        sizing.document(),
        id,
        doc_layout::TableFit::Contents,
        &mut Fixed,
    )
    .unwrap();
    check!(
        widths[0] > widths[1]
            && widths.iter().sum::<u32>() < sizing.document().page.content_width(),
        "content fit gives long text more room without filling page unnecessarily"
    );
    edit(&mut sizing, id, TableChange::Columns(widths));
    let widths = doc_layout::table_fit(
        sizing.document(),
        id,
        doc_layout::TableFit::Page,
        &mut Fixed,
    )
    .unwrap();
    check!(
        widths.iter().sum::<u32>() == sizing.document().page.content_width(),
        "page fit consumes exactly the available width"
    );
    edit(&mut sizing, id, TableChange::Columns(widths));
    let equal = doc_layout::table_fit(
        sizing.document(),
        id,
        doc_layout::TableFit::Equal,
        &mut Fixed,
    )
    .unwrap();
    check!(
        equal.iter().max().unwrap() - equal.iter().min().unwrap() <= 1
            && equal.iter().sum::<u32>() == sizing.document().table(id).unwrap().width(),
        "equal widths retain total including integer remainder"
    );
    let widths =
        doc_layout::table_fit(&merged, id, doc_layout::TableFit::Contents, &mut Fixed).unwrap();
    let mut fit = Editor::new(merged.clone()).unwrap();
    edit(&mut fit, id, TableChange::Columns(widths));
    check!(
        doc_layout::paginate(fit.document(), &mut Fixed).is_ok(),
        "content fit honors merged grid and paragraph bounds"
    );
    let mut tabbed = Editor::new(before.clone()).unwrap();
    tabbed
        .apply(Edit::SetText {
            paragraph: textp,
            inline: 0,
            text: String::from("X\tY"),
        })
        .unwrap();
    let mut stops = TabStops::default();
    stops
        .set(TabStop {
            position: 1800,
            alignment: TabAlignment::Left,
            leader: TabLeader::None,
        })
        .unwrap();
    tabbed
        .apply(Edit::SetParagraphFormat {
            paragraph: textp,
            format: ParagraphFormat {
                left: 200,
                first_line: 300,
                tabs: stops,
                ..Default::default()
            },
        })
        .unwrap();
    let widths = doc_layout::table_fit(
        tabbed.document(),
        id,
        doc_layout::TableFit::Contents,
        &mut Fixed,
    )
    .unwrap();
    check!(
        widths[0] == 2080,
        "content fit honors custom stops without counting the first-line indent twice"
    );
    // Every rectangular merge in this grid survives insert/delete at each crossed boundary.
    for rows in 1..=3 {
        for columns in 1..=3 {
            if rows * columns == 1 {
                continue;
            }
            let mut e = Editor::new(before.clone()).unwrap();
            edit(
                &mut e,
                id,
                TableChange::Merge {
                    row: 0,
                    column: 0,
                    rows,
                    columns,
                },
            );
            for at in 0..rows {
                let snapshot = e.document().clone();
                edit(&mut e, id, TableChange::InsertRow(at));
                check!(e.document().validate().is_ok(), "row insertion span matrix");
                e.undo();
                edit(&mut e, id, TableChange::RemoveRow(at));
                check!(e.document().validate().is_ok(), "row removal span matrix");
                e.undo();
                check!(e.document() == &snapshot, "row structure undo matrix");
            }
            for at in 0..columns {
                let snapshot = e.document().clone();
                edit(&mut e, id, TableChange::InsertColumn(at));
                check!(
                    e.document().validate().is_ok(),
                    "column insertion span matrix"
                );
                e.undo();
                edit(&mut e, id, TableChange::RemoveColumn(at));
                check!(
                    e.document().validate().is_ok(),
                    "column removal span matrix"
                );
                e.undo();
                check!(e.document() == &snapshot, "column structure undo matrix");
            }
        }
    }
    (passed, failed)
}
