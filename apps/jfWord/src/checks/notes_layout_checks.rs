// Native footnote/endnote pagination invariants with deterministic font metrics.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{
    Document, Inline, Note, NoteKind, PageSettings, Paragraph, ParagraphFormat, ParagraphStyle,
    ResolvedFormat, TextFormat,
};
use doc_layout::{Layout, Measure, Metrics};

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
struct Tall;
impl Measure for Tall {
    fn width(&mut self, text: &str, _: ResolvedFormat) -> i32 {
        text.chars().count() as i32 * 100
    }
    fn metrics(&mut self, _: ResolvedFormat) -> Metrics {
        Metrics {
            ascent: 1400,
            descent: 100,
            height: 1500,
        }
    }
}
fn paragraph(id: u64, text: &str) -> Paragraph {
    Paragraph {
        id,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat {
            line: 240,
            after: 0,
            ..ParagraphFormat::default()
        },
        content: vec![Inline::Text(String::from(text))],
    }
}
fn note(id: u64, kind: NoteKind, text: &str) -> Note {
    Note {
        id,
        kind,
        paragraphs: vec![paragraph(id + 100, text)],
    }
}
fn document() -> Document {
    let mut d = Document::default();
    d.page = PageSettings {
        width: 5000,
        height: 5000,
        top: 1000,
        bottom: 1000,
        left: 1000,
        right: 1000,
    };
    d.paragraphs = vec![paragraph(1, "Text ")];
    d.paragraphs[0].content.push(Inline::NoteReference(10));
    d.notes = vec![note(10, NoteKind::Footnote, "Notiz")];
    d
}
fn all_text(layout: &Layout) -> String {
    layout
        .pages
        .iter()
        .flat_map(|p| &p.lines)
        .flat_map(|l| &l.fragments)
        .map(|f| f.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}
fn bounded(layout: &Layout) -> bool {
    let top = layout.settings.top as i32;
    let bottom = (layout.settings.height - layout.settings.bottom) as i32;
    layout.pages.iter().all(|page| {
        page.lines.iter().all(|line| {
            line.top >= top
                && line.top + line.height <= bottom
                && line.fragments.iter().all(|fragment| {
                    fragment.x >= layout.settings.left as i32
                        && fragment.x + fragment.width
                            <= (layout.settings.width - layout.settings.right) as i32
                })
        }) && page
            .lines
            .windows(2)
            .all(|pair| pair[0].top + pair[0].height <= pair[1].top)
            && page.separators.iter().all(|s| s.y >= top && s.y < bottom)
    })
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
    let mut d = document();
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 1
            && layout.pages[0].separators.len() == 1
            && layout.pages[0].lines.last().unwrap().top == 3760
            && bounded(&layout),
        "footnote at bottom with reserved separator"
    );
    check!(
        layout.paragraph_at(0, 3800) == Some(110)
            && layout.paragraph_at(0, layout.pages[0].separators[0].y) == None,
        "note paragraph hit testing excludes separator"
    );
    check!(
        layout.pages[0].lines[0]
            .fragments
            .iter()
            .any(|f| f.text == "[1]" && f.format.size_half_points == 15),
        "note anchor has smaller inherited font"
    );

    d.paragraphs[0].content[0] = Inline::Text("Zeile\n".repeat(10));
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 2
            && layout.pages[0].lines.len() == 10
            && layout.pages[0].separators.is_empty()
            && layout.pages[1].lines[0]
                .fragments
                .iter()
                .any(|f| f.text == "[1]")
            && bounded(&layout),
        "footnote anchor moves with first note line"
    );

    d = document();
    d.notes[0].paragraphs[0].content[0] = Inline::Text(
        (0..50)
            .map(|i| format!("N{i:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    let text = all_text(&layout);
    check!(
        layout.pages.len() >= 4
            && bounded(&layout)
            && layout.pages.iter().all(|page| page.separators.len() == 1)
            && layout
                .pages
                .iter()
                .flat_map(|page| &page.lines)
                .filter(|line| line.paragraph == 110)
                .count()
                == 50
            && (0..50).all(|i| text.matches(&format!("N{i:02}")).count() == 1),
        "long footnote continues across pages without omitted or duplicated lines"
    );

    d = document();
    d.paragraphs.push(paragraph(2, "Nachlauf"));
    d.notes.push(note(20, NoteKind::Footnote, "Zweite"));
    d.paragraphs[0].content.push(Inline::NoteReference(20));
    d.notes.swap(0, 1);
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    let text = all_text(&layout);
    check!(
        bounded(&layout)
            && text.find("[1] Notiz") < text.find("[2] Zweite")
            && layout.pages[0]
                .lines
                .iter()
                .filter(|l| l.paragraph == 1 || l.paragraph == 2)
                .all(|l| l.top + l.height < layout.pages[0].separators[0].y),
        "multiple same-line footnotes follow anchor order and avoid body collision"
    );

    d.notes[0].paragraphs[0].format.alignment = doc_core::Alignment::Right;
    d.notes[0].paragraphs[0].content[0] = Inline::StyledText {
        text: String::from("Zweite"),
        format: TextFormat {
            bold: Some(true),
            italic: Some(true),
            underline: Some(true),
            size_half_points: Some(30),
            ..Default::default()
        },
    };
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    check!(
        layout
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .flat_map(|l| &l.fragments)
            .any(|f| f.text == "Zweite"
                && f.format.bold
                && f.format.italic
                && f.format.underline
                && f.format.size_half_points == 30
                && f.x > 1000),
        "note body preserves character formatting and paragraph alignment"
    );

    d = document();
    d.notes[0].kind = NoteKind::Endnote;
    d.notes.push(note(20, NoteKind::Endnote, "Ende zwei"));
    d.paragraphs[0].content.push(Inline::NoteReference(20));
    d.notes.swap(0, 1);
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    let text = all_text(&layout);
    check!(
        bounded(&layout)
            && layout.pages.iter().all(|p| p.separators.is_empty())
            && text.contains("Endnoten")
            && text.contains("[E1] Notiz")
            && text.find("[E1] Notiz") < text.find("[E2] Ende zwei"),
        "endnotes append heading and independently numbered bodies in anchor order"
    );

    d.notes.push(note(30, NoteKind::Footnote, "Fuss"));
    d.paragraphs[0].content.insert(1, Inline::NoteReference(30));
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    let text = all_text(&layout);
    check!(
        bounded(&layout) && text.contains("[1] Fuss") && text.contains("[E1] Notiz"),
        "footnotes and endnotes have separate numbering and shared page bounds"
    );

    d = document();
    d.notes[0]
        .paragraphs
        .push(paragraph(111, "Neue Notizseite"));
    d.notes[0].paragraphs[1].format.page_break_before = true;
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 2 && bounded(&layout) && layout.pages[1].lines[0].paragraph == 111,
        "forced break inside footnote creates continuation page"
    );

    d = document();
    check!(
        matches!(
            doc_layout::paginate(&d, &mut Tall),
            Err(doc_layout::Error::Limit)
        ),
        "anchor and first footnote line too tall fail instead of looping"
    );
    d.notes
        .push(note(20, NoteKind::Footnote, &"Long\n".repeat(40)));
    d.paragraphs[0].content.push(Inline::NoteReference(20));
    d.notes[0].paragraphs[0].content = vec![Inline::Text("First\n".repeat(40))];
    check!(
        matches!(
            doc_layout::paginate(&d, &mut Fixed),
            Err(doc_layout::Error::Limit)
        ),
        "oversized same-line note group fails explicitly without omitted notes"
    );

    d = document();
    d.page.width = 11906;
    d.sources = crate::science_checks::sources();
    d.notes[0].paragraphs[0].content.push(Inline::Citation {
        source: d.sources[0].id,
        locator: String::from("8"),
    });
    let expected = d.citation_text(d.sources[0].id, "8");
    let layout = doc_layout::paginate(&d, &mut Fixed).unwrap();
    check!(
        bounded(&layout) && all_text(&layout).contains(&expected),
        "citations in note bodies expand through shared bibliography model"
    );

    let mut varied_ok = true;
    for seed in 0..12 {
        let mut varied = document();
        varied.page.height += seed * 60;
        varied.paragraphs.clear();
        varied.notes.clear();
        for i in 0..8 {
            let id = 20 + i;
            let mut p = paragraph(1 + i, &format!("MAIN{i:02}_ "));
            p.format.before = ((seed + i as u32) % 5 * 60) as u16;
            p.format.after = ((seed + i as u32) % 4 * 100) as u16;
            p.content.push(Inline::NoteReference(id));
            varied.paragraphs.push(p);
            let kind = if (seed + i as u32) % 3 == 0 {
                NoteKind::Endnote
            } else {
                NoteKind::Footnote
            };
            let count = 1 + (seed + i as u32 * 7) % 32;
            let text = (0..count)
                .map(|j| format!("NX{i:02}{j:02}_"))
                .collect::<Vec<_>>()
                .join("\n");
            varied.notes.push(note(id, kind, &text));
        }
        varied.notes.reverse();
        match doc_layout::paginate(&varied, &mut Fixed) {
            Ok(layout) => {
                let text = all_text(&layout);
                varied_ok &= bounded(&layout);
                for i in 0..8 {
                    varied_ok &= text.matches(&format!("MAIN{i:02}_")).count() == 1;
                    for j in 0..1 + (seed + i * 7) % 32 {
                        varied_ok &= text.matches(&format!("NX{i:02}{j:02}_")).count() == 1;
                    }
                }
            }
            Err(_) => varied_ok = false,
        }
    }
    check!(
        varied_ok,
        "mixed note sizes spacing and continuation permutations preserve all content within page bounds"
    );

    d = document();
    d.paragraphs[0].content[0] = Inline::Text("x\n".repeat(25000));
    check!(
        matches!(
            doc_layout::paginate(&d, &mut Fixed),
            Err(doc_layout::Error::Limit)
        ),
        "note-aware pagination enforces the 2048 page resource budget"
    );
    (passed, failed)
}
