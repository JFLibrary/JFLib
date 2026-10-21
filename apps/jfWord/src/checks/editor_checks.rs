// Native model, formatting, search and pagination contracts; includes a reviewable DOCX fixture.
use alloc::{format, string::String, vec};
use doc_core::{
    Alignment, Document, Edit, Editor, Inline, PageSettings, ParagraphFormat, ParagraphStyle,
    SearchOptions, TextFormat,
};
use doc_layout::{Measure, Metrics};

struct Fixed;
struct InvalidMetrics;
impl Measure for InvalidMetrics {
    fn width(&mut self, _: &str, _: doc_core::ResolvedFormat) -> i32 {
        100
    }
    fn metrics(&mut self, _: doc_core::ResolvedFormat) -> Metrics {
        Metrics {
            ascent: i32::MAX,
            descent: i32::MAX,
            height: 240,
        }
    }
}
impl Measure for Fixed {
    fn width(&mut self, s: &str, _: doc_core::ResolvedFormat) -> i32 {
        s.chars().count() as i32 * 100
    }
    fn metrics(&mut self, _: doc_core::ResolvedFormat) -> Metrics {
        Metrics {
            ascent: 180,
            descent: 60,
            height: 240,
        }
    }
}
fn small(text: &str) -> Document {
    let mut d = Document::default();
    d.page = PageSettings {
        width: 5000,
        height: 5000,
        top: 1000,
        bottom: 1000,
        left: 1000,
        right: 1000,
    };
    d.paragraphs[0].content = vec![Inline::Text(String::from(text))];
    d.paragraphs[0].format.line = 240;
    d
}
pub(crate) fn sample() -> Document {
    let mut e = Editor::new(Document::default()).unwrap();
    e.apply(Edit::ImportSources(crate::science_checks::sources()))
        .unwrap();
    e.apply(Edit::SetText {
        paragraph: 1,
        inline: 0,
        text: String::from("Wissenschaftliches Arbeiten"),
    })
    .unwrap();
    e.apply(Edit::SetStyle {
        paragraph: 1,
        style: ParagraphStyle::Title,
    })
    .unwrap();
    let p = e.apply(Edit::AddParagraph { after: 1 }).unwrap();
    e.apply(Edit::SetText {
        paragraph: p,
        inline: 0,
        text: String::from("Methoden und Nachvollziehbarkeit"),
    })
    .unwrap();
    e.apply(Edit::SetStyle {
        paragraph: p,
        style: ParagraphStyle::Heading1,
    })
    .unwrap();
    let p = e.apply(Edit::AddParagraph { after: p }).unwrap();
    e.apply(Edit::SetText {paragraph:p,inline:0,text:String::from("Gute Forschung verbindet klare Fragen mit prüfbaren Quellen. ")+
        &"Die Methode wird dokumentiert, Ergebnisse werden kritisch verglichen und Aussagen mit Literatur belegt. ".repeat(16)}).unwrap();
    e.apply(Edit::FormatText {
        paragraph: p,
        inline: 0,
        start: 0,
        end: 14,
        format: TextFormat {
            bold: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    e.apply(Edit::SetParagraphFormat {
        paragraph: p,
        format: ParagraphFormat {
            alignment: Alignment::Justify,
            line: 360,
            ..Default::default()
        },
    })
    .unwrap();
    e.apply(Edit::Cite {
        paragraph: p,
        source: 2,
        locator: String::from("41–44"),
    })
    .unwrap();
    let p = e.apply(Edit::AddParagraph { after: p }).unwrap();
    e.apply(Edit::SetText {
        paragraph: p,
        inline: 0,
        text: String::from("Weiterführende Analyse"),
    })
    .unwrap();
    e.apply(Edit::SetStyle {
        paragraph: p,
        style: ParagraphStyle::Heading1,
    })
    .unwrap();
    e.apply(Edit::SetParagraphFormat {
        paragraph: p,
        format: ParagraphFormat {
            page_break_before: true,
            ..Default::default()
        },
    })
    .unwrap();
    let p = e.apply(Edit::AddParagraph { after: p }).unwrap();
    e.apply(Edit::SetText {paragraph:p,inline:0,text:String::from("Reproduzierbarkeit erfordert zugängliche Daten und präzise Angaben.\nEin expliziter Zeilenumbruch bleibt Teil desselben Absatzes.")}).unwrap();
    e.apply(Edit::FormatText {
        paragraph: p,
        inline: 0,
        start: 0,
        end: 19,
        format: TextFormat {
            italic: Some(true),
            underline: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    e.apply(Edit::Cite {
        paragraph: p,
        source: 3,
        locator: String::new(),
    })
    .unwrap();
    e.apply(Edit::Cite {
        paragraph: p,
        source: 4,
        locator: String::new(),
    })
    .unwrap();
    e.apply(Edit::InsertBibliography { after: p }).unwrap();
    e.document().clone()
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr,$name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    let mut e = Editor::new(small("Äpfel Forschung ÄPFEL")).unwrap();
    let before = e.document().clone();
    check!(
        e.apply(Edit::FormatText {
            paragraph: 1,
            inline: 0,
            start: 1,
            end: 3,
            format: TextFormat {
                bold: Some(true),
                ..Default::default()
            }
        })
        .is_err()
            && *e.document() == before
            && e.revision() == 0,
        "UTF-8 format boundaries and atomic failure"
    );
    e.apply(Edit::FormatText {
        paragraph: 1,
        inline: 0,
        start: 7,
        end: 11,
        format: TextFormat {
            bold: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    check!(
        e.document().plain_text() == before.plain_text()
            && e.document().paragraphs[0].content.len() == 3,
        "format selection preserves prefix suffix and Unicode"
    );
    let matches = e
        .document()
        .find_text("Forschung", SearchOptions::default())
        .unwrap();
    check!(
        matches.len() == 1 && matches[0].start_inline == 1 && matches[0].end_inline == 2,
        "search crosses character formatting boundaries"
    );
    let snapshot = e.document().clone();
    check!(
        e.apply(Edit::ReplaceMatch {
            found: matches[0].clone(),
            expected: String::from("stale"),
            replacement: String::from("X")
        })
        .is_err()
            && *e.document() == snapshot,
        "stale search ranges rejected"
    );
    check!(
        e.apply(Edit::ReplaceAll {
            query: String::from("Forschung"),
            replacement: String::from("Analyse"),
            options: SearchOptions::default()
        }) == Ok(1)
            && e.document().plain_text() == "Äpfel Analyse ÄPFEL",
        "replacement across formatting runs"
    );
    e.undo();
    check!(
        *e.document() == snapshot,
        "replacement undo restores exact formatting"
    );
    let folded = SearchOptions {
        case_sensitive: false,
        whole_word: true,
    };
    check!(
        e.document().find_text("äpfel", folded).unwrap().len() == 2,
        "Unicode lowercase matching"
    );
    check!(
        small("cat catlike bobcat cat")
            .find_text(
                "cat",
                SearchOptions {
                    whole_word: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .len()
            == 2,
        "whole-word matching"
    );
    let mut fields = small("left");
    fields.sources = crate::science_checks::sources();
    fields.paragraphs[0].content.push(Inline::Citation {
        source: 2,
        locator: String::new(),
    });
    fields.paragraphs[0]
        .content
        .push(Inline::Text(String::from("right")));
    check!(
        fields
            .find_text("Lovelace", SearchOptions::default())
            .unwrap()
            .is_empty()
            && fields
                .find_text("leftright", SearchOptions::default())
                .unwrap()
                .is_empty(),
        "search cannot consume generated fields"
    );
    let mut many = Editor::new(small("a a a")).unwrap();
    check!(
        many.apply(Edit::ReplaceAll {
            query: String::from("a"),
            replacement: String::from("long"),
            options: SearchOptions::default()
        }) == Ok(3)
            && many.document().plain_text() == "long long long",
        "reverse replacement positions remain valid"
    );
    many.undo();
    check!(
        many.document().plain_text() == "a a a" && !many.can_undo(),
        "replace all is one undo transaction"
    );
    let rev = many.revision();
    many.apply(Edit::ReplaceAll {
        query: String::from("a"),
        replacement: String::from("a"),
        options: SearchOptions::default(),
    })
    .unwrap();
    check!(
        many.revision() == rev && many.can_redo(),
        "no-op replacement preserves redo and layout cache"
    );
    let invalid = PageSettings {
        left: u32::MAX,
        ..Default::default()
    };
    check!(
        many.apply(Edit::SetPage(invalid)).is_err(),
        "page margins validated before applying"
    );
    let layout = doc_layout::paginate(&small(""), &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 1 && layout.pages[0].lines.len() == 1,
        "empty document has one editable page line"
    );
    let layout = doc_layout::paginate(&small(&"x\n".repeat(25)), &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 3 && layout.pages.iter().map(|p| p.lines.len()).sum::<usize>() == 26,
        "explicit newlines and page count"
    );
    check!(
        layout
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .all(|l| l.top >= 1000 && l.top + l.height <= 4000),
        "lines stay within vertical margins"
    );
    check!(
        layout.paragraph_at(0, 1100) == Some(1) && layout.paragraph_at(0, 900).is_none(),
        "page hit testing respects margins"
    );
    let mut centered = small("1234567890");
    centered.paragraphs[0].format.alignment = Alignment::Center;
    let layout = doc_layout::paginate(&centered, &mut Fixed).unwrap();
    check!(
        layout.pages[0].lines[0].fragments[0].x == 2000,
        "center alignment in page coordinates"
    );
    centered.paragraphs[0].format.alignment = Alignment::Right;
    check!(
        doc_layout::paginate(&centered, &mut Fixed).unwrap().pages[0].lines[0].fragments[0].x
            == 3000,
        "right alignment"
    );
    let mut justified = small("aa bb cc dd ee ff gg hh ii jj kk ll");
    justified.paragraphs[0].format.alignment = Alignment::Justify;
    let layout = doc_layout::paginate(&justified, &mut Fixed).unwrap();
    let last = layout.pages[0].lines[0].fragments.last().unwrap();
    check!(
        last.x + last.width == 4000,
        "justified first line reaches right margin"
    );
    let layout = doc_layout::paginate(&small(&"ü".repeat(95)), &mut Fixed).unwrap();
    check!(
        layout.pages[0].lines.len() == 4
            && layout.pages[0]
                .lines
                .iter()
                .flat_map(|l| &l.fragments)
                .all(|f| f.x + f.width <= 4000),
        "long Unicode tokens split without overflow"
    );
    let mut joined = small("1234567890123456789012345 ");
    joined.paragraphs[0].content.push(Inline::with_format(
        String::from("abcdef"),
        TextFormat {
            bold: Some(true),
            ..Default::default()
        },
    ));
    joined.paragraphs[0]
        .content
        .push(Inline::Text(String::from("ghi")));
    let layout = doc_layout::paginate(&joined, &mut Fixed).unwrap();
    check!(
        layout.pages[0].lines[1]
            .fragments
            .iter()
            .map(|f| f.text.as_str())
            .collect::<alloc::vec::Vec<_>>()
            .join("")
            == "abcdefghi",
        "word wrapping ignores format boundaries"
    );
    let mut breaks = Editor::new(small("one")).unwrap();
    let id = breaks.apply(Edit::AddParagraph { after: 1 }).unwrap();
    breaks
        .apply(Edit::SetParagraphFormat {
            paragraph: id,
            format: ParagraphFormat {
                page_break_before: true,
                ..Default::default()
            },
        })
        .unwrap();
    check!(
        doc_layout::paginate(breaks.document(), &mut Fixed)
            .unwrap()
            .pages
            .len()
            == 2,
        "manual page break"
    );
    let mut first = small("first");
    first.paragraphs[0].format.page_break_before = true;
    check!(
        doc_layout::paginate(&first, &mut Fixed)
            .unwrap()
            .pages
            .len()
            == 1,
        "no leading blank page for first paragraph break"
    );
    let layout = doc_layout::paginate(&small("a\tb"), &mut Fixed).unwrap();
    check!(
        layout.pages[0].lines[0].fragments.last().unwrap().x == 1720,
        "tabs advance to a fixed 36 point stop"
    );
    let mut heading = Editor::new(small(&("x\n".repeat(8) + "x"))).unwrap();
    let title = heading.apply(Edit::AddParagraph { after: 1 }).unwrap();
    heading
        .apply(Edit::SetStyle {
            paragraph: title,
            style: ParagraphStyle::Heading1,
        })
        .unwrap();
    let following = heading.apply(Edit::AddParagraph { after: title }).unwrap();
    heading
        .apply(Edit::SetParagraphFormat {
            paragraph: following,
            format: ParagraphFormat {
                line: 480,
                ..Default::default()
            },
        })
        .unwrap();
    let layout = doc_layout::paginate(heading.document(), &mut Fixed).unwrap();
    check!(
        layout.pages.len() == 2
            && layout.pages[0].lines.len() == 9
            && layout.pages[1].lines[0].paragraph == title
            && layout.pages[1].lines[1].paragraph == following,
        "heading reserves the actual first following line height"
    );
    check!(
        matches!(
            doc_layout::paginate(&small("x"), &mut InvalidMetrics),
            Err(doc_layout::Error::Metrics)
        ),
        "invalid font metrics cannot overflow pagination"
    );
    let d = sample();
    let bytes = doc_io::encode_docx(&d).unwrap();
    check!(
        doc_io::decode_docx(&bytes) == Ok(d.clone()),
        "formatted scientific document DOCX round trip"
    );
    let mut landscape = d.clone();
    landscape.page = PageSettings {
        width: 15840,
        height: 12240,
        top: 1000,
        left: 1200,
        right: 1300,
        bottom: 1400,
    };
    check!(
        doc_io::decode_docx(&doc_io::encode_docx(&landscape).unwrap()) == Ok(landscape),
        "landscape Letter custom margin round trip"
    );
    if base::Args(plat::args()).value("--fixture").is_some() {
        check!(
            doc_io::save_docx("build/jfword-editor.docx", &d).is_ok(),
            "formatted editor fixture"
        );
    }
    (passed, failed)
}
