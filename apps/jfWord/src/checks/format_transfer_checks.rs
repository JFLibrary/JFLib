// Character-format transfer/reset: style inheritance, UTF-8 and protected fields.
use alloc::{format, string::String, vec};
use doc_core::{Document, Edit, Editor, Inline, Paragraph, ParagraphStyle, TextColor, TextFormat};

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $label:expr) => {
            if $ok { passed += 1; } else {
                failed += 1;
                plat::write_stdout(format!("FAIL format transfer {}\n", $label).as_bytes());
            }
        };
    }
    let mut document = Document::default();
    document.paragraphs[0].style = ParagraphStyle::Heading1;
    document.paragraphs[0].content = vec![Inline::with_format(String::from("Quelle α"), TextFormat {
        bold: Some(false), italic: Some(true), color: Some(TextColor::Auto), ..Default::default()
    })];
    document.paragraphs.push(Paragraph {
        id: 2, style: ParagraphStyle::Title, format: Default::default(),
        content: vec![Inline::with_format(String::from("Ziel β"), TextFormat {
            underline: Some(true), strike: Some(true), size_half_points: Some(20),
            color: Some(TextColor::Rgb(0xaabbcc)), ..Default::default()
        })],
    });
    let story = document.story_projection(1).unwrap();
    let split = story.paragraphs[1].start;
    let sample = document.story_character_format(1, 0, split - 1).unwrap();
    check!(sample.bold == Some(false) && sample.italic == Some(true)
        && sample.size_half_points == Some(32) && sample.color == Some(TextColor::Rgb(0)), "effective inherited properties");
    check!(document.story_character_format(1, split - 2, split - 1).is_err(), "invalid UTF-8 sampling rejected");
    let mut editor = Editor::new(document.clone()).unwrap();
    editor.apply(Edit::ReplaceStoryFormat { paragraph: 1, start: split, end: story.text.len(), format: sample }).unwrap();
    check!(editor.document().paragraphs[1].content[0].format() == sample, "replace clears target overrides");
    check!(editor.document().paragraphs[1].style == ParagraphStyle::Title
        && editor.document().story_projection(1).unwrap().text == story.text, "text and paragraph style retained");
    check!(editor.document().paragraphs[1].content[0].format().resolve(ParagraphStyle::Title)
        == document.paragraphs[0].content[0].format().resolve(ParagraphStyle::Heading1), "effective appearance transfers across styles");
    let formatted = editor.document().clone();
    let decoded = doc_io::decode_docx(&doc_io::encode_docx(&formatted).unwrap()).unwrap();
    check!(decoded.paragraphs[1].content[0].format() == sample, "DOCX preserves transferred properties");
    let fragment = formatted.copy_story_range(1, split, story.text.len()).unwrap();
    let copied = doc_io::decode_document_fragment(&doc_io::encode_document_fragment(&fragment).unwrap()).unwrap();
    check!(copied.document.paragraphs[0].content[0].format() == sample, "native clipboard preserves transferred properties");
    let rtf = doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(&formatted).unwrap()).unwrap();
    check!(rtf.paragraphs[1].content[0].format().resolve(rtf.paragraphs[1].style)
        == sample.resolve(ParagraphStyle::Title), "RTF effective appearance retained");
    check!(doc_io::clipboard_html_encode(&formatted).is_ok(), "HTML export supports transferred formatting");
    editor.apply(Edit::ReplaceStoryFormat { paragraph: 1, start: 0, end: story.text.len(), format: TextFormat::default() }).unwrap();
    check!(editor.document().paragraphs.iter().flat_map(|p| &p.content).all(|i| i.format() == TextFormat::default()), "cross-paragraph reset restores inheritance");
    check!(editor.document().paragraphs[0].content[0].format().resolve(ParagraphStyle::Heading1).bold, "heading becomes bold again after reset");
    check!(editor.undo() && editor.document() == &formatted, "reset is one undo step");
    check!(editor.undo() && editor.document() == &document, "transfer is one undo step");
    check!(editor.redo() && editor.document() == &formatted, "redo restores exact transfer");

    // Every malformed scalar boundary is rejected atomically, even across paragraphs.
    for at in 0..story.text.len() {
        if story.text.is_char_boundary(at) { continue; }
        let before = editor.document().clone();
        let revision = editor.revision();
        check!(editor.apply(Edit::ReplaceStoryFormat { paragraph: 1, start: at, end: story.text.len(), format: sample }).is_err()
            && editor.document() == &before && editor.revision() == revision, "invalid range leaves history and document unchanged");
    }
    let scientific = crate::checks::sample();
    let full = scientific.story_projection(1).unwrap();
    let mut scientific_editor = Editor::new(scientific.clone()).unwrap();
    check!(scientific_editor.apply(Edit::ReplaceStoryFormat { paragraph: 1, start: 0, end: full.text.len(), format: sample }).is_err()
        && scientific_editor.document() == &scientific && !scientific_editor.can_undo(), "generated citations and bibliography reject atomically");
    let mut adjacent = Document::default();
    adjacent.paragraphs[0].content = vec![Inline::Text(String::from("A")), Inline::with_format(String::from("B"), TextFormat { bold: Some(true), ..Default::default() })];
    check!(adjacent.story_character_format(1, 1, 2).unwrap().bold == Some(true), "nonempty selection samples following run at boundary");
    check!(adjacent.story_character_format(1, 1, 1).unwrap().bold == Some(false), "collapsed selection retains caret affinity");
    (passed, failed)
}
