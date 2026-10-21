// Character properties across inheritance, safe editing, interchange and real page geometry.
use alloc::{format, rc::Rc, string::String, vec};
use doc_core::{Document, Edit, Editor, Highlight, Inline, Note, NoteKind, Paragraph,
    ParagraphStyle, StyleDefinition, TextFormat, TextPositioning};
use doc_layout::Measure;
use ui::Style;

fn run(text: &str, highlight: Highlight, positioning: TextPositioning) -> Inline {
    Inline::with_format(String::from(text), TextFormat { highlight: Some(highlight),
        positioning: Some(positioning), ..Default::default() })
}
pub(crate) fn run_checks() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => { if $ok { passed += 1; } else {
            failed += 1; plat::write_stdout(format!("FAIL character {}\n", $name).as_bytes());
        } };
    }
    for color in Highlight::ALL {
        check!(Highlight::from_name(color.name()) == Some(color), "stable palette identity");
        if let Some(rgb) = color.rgb() { check!(Highlight::from_rgb(rgb) == Some(color), "exact palette RGB"); }
        for position in [TextPositioning::Baseline, TextPositioning::Superscript, TextPositioning::Subscript] {
            let mut d = Document::default();
            d.paragraphs[0].content = vec![run("β² Ω", color, position)];
            let bytes = doc_io::encode_docx(&d).unwrap();
            check!(doc_io::decode_docx(&bytes).as_ref() == Ok(&d), "all DOCX palette/script combinations");
            let rtf = doc_io::clipboard_rtf_encode(&d).unwrap();
            let back = doc_io::clipboard_rtf_decode(&rtf).unwrap();
            let f = back.paragraphs[0].content[0].format();
            check!(f.highlight == Some(color) && f.positioning == Some(position)
                && f.size_half_points == Some(22) && back.plain_text() == d.plain_text(), "RTF preserves nominal size and scripts");
        }
    }
    check!(Highlight::from_name("chartreuse").is_none() && Highlight::from_rgb(0x123456).is_none(), "no lossy palette guessing");
    let mut d = Document::default();
    let mut base = StyleDefinition::new(ParagraphStyle::Named(1), "Markiert");
    base.text.highlight = Some(Highlight::Yellow);
    base.text.positioning = Some(TextPositioning::Superscript);
    let mut child = StyleDefinition::new(ParagraphStyle::Named(2), "Abgeleitet");
    child.based_on = Some(base.id);
    d.styles = vec![base.clone(), child];
    d.paragraphs[0].style = ParagraphStyle::Named(2);
    d.paragraphs[0].content = vec![Inline::Text(String::from("H")), run("2", Highlight::None, TextPositioning::Baseline), Inline::Text(String::from("O"))];
    let inherited = d.resolve_text(Default::default(), d.paragraphs[0].style);
    check!(inherited.highlight == Highlight::Yellow && inherited.positioning == TextPositioning::Superscript, "inherited script and highlight");
    let mut e = Editor::new(d.clone()).unwrap();
    base.text.highlight = Some(Highlight::Cyan); base.text.positioning = Some(TextPositioning::Subscript);
    e.apply(Edit::SetStyleDefinition(base)).unwrap();
    check!(e.document().story_character_format(1, 1, 2).unwrap().positioning == Some(TextPositioning::Baseline)
        && e.document().story_character_format(1, 1, 2).unwrap().highlight == Some(Highlight::None), "explicit clears survive parent edits");
    let summary = e.document().story_character_common(1, 0, 3).unwrap();
    check!(summary.highlight.is_none() && summary.positioning.is_none() && summary.bold == Some(false), "mixed selection summary");
    let before = e.document().clone();
    e.apply(Edit::FormatStoryText { paragraph: 1, start: 0, end: 3,
        format: TextFormat { positioning: Some(TextPositioning::Superscript), ..Default::default() } }).unwrap();
    check!(e.document().story_character_common(1, 0, 3).unwrap().positioning == Some(TextPositioning::Superscript)
        && e.document().story_character_common(1, 0, 3).unwrap().highlight.is_none(), "single-property patch preserves mixed highlighting");
    e.undo(); check!(e.document() == &before, "undo restores inherited and direct formatting");
    e.redo(); check!(e.document().story_character_common(1, 0, 3).unwrap().positioning == Some(TextPositioning::Superscript), "redo restores script");
    let bytes = doc_io::encode_docx(e.document()).unwrap();
    check!(doc_io::decode_docx(&bytes).as_ref() == Ok(e.document()), "styles and clears roundtrip");
    let copy = e.document().copy_story_range(1, 0, 3).unwrap();
    let private = doc_io::encode_document_fragment(&copy).unwrap();
    check!(doc_io::decode_document_fragment(&private).as_ref() == Ok(&copy), "native clipboard keeps properties and inheritance");
    let mut target = Document::default();
    target.paragraphs[0].content = vec![Inline::Text(String::from("AB"))];
    let mut target = Editor::new(target).unwrap();
    target.replace_story_fragment(1, 1, 1, copy).unwrap();
    check!(target.document().plain_text() == "AH2OB"
        && target.document().story_character_format(1, 1, 2).unwrap().highlight == Some(Highlight::Cyan)
        && target.document().story_character_format(1, 2, 3).unwrap().highlight == Some(Highlight::None)
        && target.document().story_character_format(1, 4, 5).unwrap().positioning == Some(TextPositioning::Baseline), "clipboard joined seams preserve source and destination");
    let rtf = br"{\rtf1{\colortbl;\red255\green255\blue0;}\highlight1\super A{\sub B}\nosupersub C\plain D}";
    let imported = doc_io::clipboard_rtf_decode(rtf).unwrap();
    let runs = &imported.paragraphs[0].content;
    check!(runs.len() == 4 && runs[0].format().positioning == Some(TextPositioning::Superscript)
        && runs[1].format().positioning == Some(TextPositioning::Subscript)
        && runs[2].format().positioning == Some(TextPositioning::Baseline)
        && runs[2].format().highlight == Some(Highlight::Yellow)
        && runs[3].format().highlight == Some(Highlight::None), "independent RTF group restore and plain reset");
    check!(doc_io::clipboard_rtf_decode(br"{\rtf1{\colortbl;\red1\green2\blue3;}\highlight1 X}").is_err(), "non-palette RTF color rejected");
    check!(doc_io::clipboard_rtf_decode(br"{\rtf1\super2 X}").is_err(), "invalid script toggle rejected");
    let mut sample = Document::default();
    sample.paragraphs[0].content = vec![Inline::Text(String::from("H")), run("2", Highlight::Yellow, TextPositioning::Subscript),
        Inline::Text(String::from("O + x")), run("2", Highlight::Cyan, TextPositioning::Superscript)];
    let html = String::from_utf8(doc_io::clipboard_html_encode(&sample).unwrap()).unwrap();
    check!(html.contains("background-color:#FFFF00;vertical-align:-2.20pt")
        && html.contains("background-color:#00FFFF;vertical-align:3.65pt")
        && html.contains("font-size:7.50pt"), "HTML uses shared display size and baseline shifts");
    let mut text = text::Text::new(style::STYLE.fonts(), style::STYLE.font_names());
    let fonts = doc_view::Fonts::new(&mut text).unwrap();
    for size in [8, 22, 96, 288] {
        for inline in &mut sample.paragraphs[0].content { let mut f = inline.format(); f.size_half_points = Some(size); *inline = Inline::with_format(String::from(inline.text().unwrap()), f); }
        let layout = Rc::new(doc_view::paginate(&sample, &mut text, &fonts).unwrap());
        let first = &layout.pages[0].lines[0];
        let baseline = first.fragments[0].baseline;
        let sub = &first.fragments[1];
        check!(sub.baseline > baseline && sub.format.size_half_points == size
            && sub.format.display_size_half_points() < size, "subscript retains nominal size and lowers glyph baseline");
        let sup = layout.pages.iter().flat_map(|p| &p.lines).flat_map(|l| &l.fragments).find(|f| f.format.positioning == TextPositioning::Superscript).unwrap();
        check!(sup.format.baseline_shift() < 0 && sup.format.size_half_points == size, "superscript shares zoom-independent policy");
        let mut contained = true;
        for line in layout.pages.iter().flat_map(|p| &p.lines) {
            for fragment in &line.fragments {
                let metrics = doc_view::FontMeasure::new(&mut text, &fonts).metrics(fragment.format);
                contained &= fragment.baseline - metrics.ascent >= line.top && fragment.baseline + metrics.descent <= line.top + line.height;
            }
        }
        check!(contained, "glyph bounds fit mixed-script line extents at minimum and maximum sizes");
        if size == 22 {
            let page = doc_view::PageDraw::new(layout, 0, fonts.clone()).unwrap();
            let pixels = page.thumbnail(&mut text, 900).unwrap();
            check!(pixels.px.chunks_exact(4).any(|p| p == [0, 255, 255, 255])
                && pixels.px.chunks_exact(4).any(|p| p == [255, 255, 0, 255]), "page renderer paints exact document highlight colors");
        }
    }
    // Exercise the separate note flow engine and DOCX note parts with the same properties.
    sample.notes = vec![Note { id: 5, kind: NoteKind::Footnote, paragraphs: vec![Paragraph {
        id: 6, style: ParagraphStyle::Normal, format: Default::default(),
        content: vec![run("Notiz", Highlight::Yellow, TextPositioning::Superscript)],
    }] }];
    sample.paragraphs[0].content.push(Inline::NoteReference(5));
    let layout = doc_view::paginate(&sample, &mut text, &fonts).unwrap();
    check!(layout.pages.iter().flat_map(|p| &p.lines).any(|l| l.paragraph == 6
        && l.fragments.iter().any(|f| f.format.highlight == Highlight::Yellow && f.format.positioning == TextPositioning::Superscript)), "footnote flow supports same character properties");
    let bytes = doc_io::encode_docx(&sample).unwrap();
    check!(doc_io::decode_docx(&bytes).as_ref() == Ok(&sample), "note properties survive DOCX");
    (passed, failed)
}
