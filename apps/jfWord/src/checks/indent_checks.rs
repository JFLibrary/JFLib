// First-line and hanging indents: model atomicity, OOXML and page/note geometry.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{
    Alignment, Document, Edit, Editor, Inline, Note, NoteKind, PageSettings, Paragraph,
    ParagraphFormat, ParagraphStyle, ResolvedFormat,
};
use doc_layout::{Measure, Metrics};

struct Fixed;
impl Measure for Fixed {
    fn width(&mut self, text: &str, _: ResolvedFormat) -> i32 {
        text.chars().count() as i32 * 100
    }
    fn metrics(&mut self, _: ResolvedFormat) -> Metrics {
        Metrics {
            ascent: 180,
            descent: 60,
            height: 240,
        }
    }
}
fn paragraph(id: u64, text: &str, first_line: i32) -> Paragraph {
    Paragraph {
        id,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat {
            first_line,
            left: 600,
            right: 200,
            line: 240,
            after: 0,
            ..Default::default()
        },
        content: vec![Inline::Text(String::from(text))],
    }
}
fn document(text: &str, first_line: i32) -> Document {
    let mut d = Document::default();
    d.page = PageSettings {
        width: 5000,
        height: 5000,
        top: 1000,
        bottom: 1000,
        left: 1000,
        right: 1000,
    };
    d.paragraphs = vec![paragraph(1, text, first_line)];
    d
}

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    for indent in [-1600, -600, 0, 600, 1840] {
        let d = document("Test", indent);
        check!(d.validate().is_ok(), "valid first-line extent");
        check!(
            doc_io::decode_docx(&doc_io::encode_docx(&d).unwrap()) == Ok(d.clone()),
            "signed first-line indent DOCX round trip"
        );
        let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
        check!(
            layout.pages[0].lines[0].x == 1600 + indent,
            "first-line insertion geometry"
        );
    }
    for indent in [i32::MIN, -1601, 1841, i32::MAX] {
        let mut editor = Editor::new(document("Test", 0)).unwrap();
        let before = editor.document().clone();
        let mut format = before.paragraphs[0].format;
        format.first_line = indent;
        check!(
            editor
                .apply(Edit::SetParagraphFormat {
                    paragraph: 1,
                    format
                })
                .is_err()
                && *editor.document() == before
                && !editor.can_undo(),
            "invalid first-line extent rejects atomically without overflow"
        );
    }
    let mut editor = Editor::new(document("Text", 0)).unwrap();
    let mut format = editor.document().paragraphs[0].format;
    format.first_line = -600;
    editor
        .apply(Edit::SetParagraphFormat {
            paragraph: 1,
            format,
        })
        .unwrap();
    check!(
        editor.undo()
            && editor.document().paragraphs[0].format.first_line == 0
            && editor.redo()
            && editor.document().paragraphs[0].format.first_line == -600,
        "indent change has one undo snapshot"
    );

    for indent in [-600, 600] {
        for alignment in [
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::Justify,
        ] {
            let mut d = document(&"αβγδ word ".repeat(80), indent);
            d.paragraphs[0].format.alignment = alignment;
            d.paragraphs.push(paragraph(2, "Next\nSoft break", indent));
            let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
            check!(layout.pages.len() > 1, "indented paragraph spans pages");
            let lines: Vec<_> = layout
                .pages
                .iter()
                .flat_map(|page| &page.lines)
                .filter(|line| line.paragraph == 1)
                .collect();
            check!(
                lines.iter().enumerate().all(|(index, line)| {
                    let extra = if index == 0 { indent } else { 0 };
                    let available = 2200 - extra;
                    let occupied: i32 = line.fragments.iter().map(|f| f.width).sum();
                    let free = (available - occupied).max(0);
                    let expected = 1600
                        + extra
                        + match alignment {
                            Alignment::Center => free / 2,
                            Alignment::Right => free,
                            _ => 0,
                        };
                    line.x == expected
                        && line
                            .fragments
                            .iter()
                            .all(|f| f.x >= 1600 + extra && f.x + f.width <= 3800)
                }),
                "all alignments use per-line width and first line only once across pages"
            );
            let second: Vec<_> = layout
                .pages
                .iter()
                .flat_map(|page| &page.lines)
                .filter(|line| line.paragraph == 2)
                .collect();
            check!(
                second.len() == 2 && second[0].x == 1600 + indent && second[1].x == 1600,
                "next paragraph resets indent; soft break does not"
            );
            check!(
                lines
                    .iter()
                    .flat_map(|line| &line.fragments)
                    .all(|fragment| {
                        let Some((start, end)) = fragment.source else {
                            return false;
                        };
                        d.text_projection(1).unwrap().text.get(start..end)
                            == Some(fragment.text.as_str())
                    }),
                "indented geometry retains scalar hit-test provenance"
            );
        }
        // Long tokens must reevaluate available width immediately after a line wraps.
        let d = document(&"α".repeat(160), indent);
        let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
        let lines: Vec<_> = layout.pages.iter().flat_map(|page| &page.lines).collect();
        check!(
            lines[0]
                .fragments
                .iter()
                .map(|f| f.text.chars().count())
                .sum::<usize>()
                == ((2200 - indent) / 100) as usize
                && lines[1]
                    .fragments
                    .iter()
                    .map(|f| f.text.chars().count())
                    .sum::<usize>()
                    == 22,
            "long unicode tokens use changing first-line width"
        );
    }
    let tabbed = doc_layout::paginate(&document("\tText", -600), &mut Fixed).unwrap();
    check!(
        tabbed.pages[0].lines[0].fragments[1].x == 1600,
        "hanging indent has implicit tab stop at paragraph left edge"
    );
    let tabbed = doc_layout::paginate(&document("\tText", 600), &mut Fixed).unwrap();
    check!(
        tabbed.pages[0].lines[0].fragments[1].x == 2320,
        "first-line tabs retain paragraph-relative grid"
    );

    for kind in [NoteKind::Footnote, NoteKind::Endnote] {
        let mut d = document("Anchor", 600);
        d.paragraphs[0].content.push(Inline::NoteReference(10));
        d.notes.push(Note {
            id: 10,
            kind,
            paragraphs: vec![
                paragraph(11, &"Scientific note αβ ".repeat(65), -600),
                paragraph(12, "Second note paragraph\nSoft break", 600),
            ],
        });
        check!(
            doc_io::decode_docx(&doc_io::encode_docx(&d).unwrap()) == Ok(d.clone()),
            "note paragraph indents DOCX round trip"
        );
        let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
        if kind == NoteKind::Endnote {
            let generated = layout
                .pages
                .iter()
                .enumerate()
                .find_map(|(page, p)| {
                    p.lines
                        .iter()
                        .find(|line| line.paragraph == 0)
                        .map(|line| (page, line))
                })
                .unwrap();
            check!(
                layout.paragraph_at(generated.0, generated.1.top).is_none()
                    && generated.1.fragments.iter().all(|f| f.source.is_none()),
                "generated endnote heading has no editable paragraph provenance"
            );
        }
        check!(
            layout.pages[0].lines[0].x == 2200,
            "note-aware main flow keeps first-line indent"
        );
        let note: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|page| &page.lines)
            .filter(|line| line.paragraph == 11)
            .collect();
        check!(
            note.len() > 10 && note[0].x == 1000 && note[1..].iter().all(|l| l.x == 1600),
            "note hanging indent does not restart on continuation page"
        );
        let second: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|page| &page.lines)
            .filter(|line| line.paragraph == 12)
            .collect();
        check!(
            second[0].x == 2200 && second[1..].iter().all(|l| l.x == 1600),
            "next note paragraph resets first-line indent"
        );
    }
    (passed, failed)
}
