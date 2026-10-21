// Real model/layout/codec and platform-independent ruler gesture contracts.
use alloc::{format, string::String, vec};
use doc_core::*;
use doc_view::{RulerAxis, RulerController, RulerDraw, RulerEdit, RulerPalette};
struct Fixed;
impl doc_layout::Measure for Fixed {
    fn width(&mut self, s: &str, _: ResolvedFormat) -> i32 { s.chars().count() as i32 * 100 }
    fn metrics(&mut self, _: ResolvedFormat) -> doc_layout::Metrics { doc_layout::Metrics { ascent: 180, descent: 60, height: 240 } }
}
fn stop(position: i32, alignment: TabAlignment, leader: TabLeader) -> TabStop { TabStop { position, alignment, leader } }
fn list(values: &[TabStop]) -> TabStops { let mut t = TabStops::default(); for v in values { t.set(*v).unwrap(); } t }
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check { ($ok:expr, $name:expr) => { if $ok { passed += 1; } else { failed += 1; plat::write_stdout(format!("FAIL tabs {}\n", $name).as_bytes()); } }; }
    let a = stop(1440, TabAlignment::Left, TabLeader::Dot);
    let b = stop(2880, TabAlignment::Right, TabLeader::Hyphen);
    let mut full = TabStops::default();
    for i in (0..64).rev() { full.set(stop(i * 90, TabAlignment::Left, TabLeader::None)).unwrap(); }
    check!(full.set(a).is_ok() && full.as_slice().len() == 64 && full.as_slice()[16] == a, "replace same position in full sorted list");
    // The replacement above changes its leader; account for that before the failed insert.
    let before = full;
    check!(full.set(stop(9000, TabAlignment::Left, TabLeader::None)) == Err(Error::Limit) && full == before, "capacity failure leaves input unchanged");
    check!(full.set(stop(31681, TabAlignment::Left, TabLeader::None)) == Err(Error::Invalid), "position bounds");
    check!(full.set(stop(100, TabAlignment::Bar, TabLeader::Dot)) == Err(Error::Invalid), "bar leader rejected");
    let mut d = Document::default();
    d.paragraphs[0].content = vec![Inline::Text(String::from("Name\t12,50"))];
    let mut base = StyleDefinition::new(ParagraphStyle::Named(1), "Tabellenzeile");
    base.paragraph.tabs = list(&[a, b]);
    let mut child = StyleDefinition::new(ParagraphStyle::Named(2), "Abgeleitet");
    child.based_on = Some(base.id);
    child.paragraph.tabs = list(&[TabStop::clear(1440), stop(4320, TabAlignment::Decimal, TabLeader::Underscore)]);
    d.styles = vec![base.clone(), child]; d.paragraphs[0].style = ParagraphStyle::Named(2);
    check!(d.resolve_paragraph(&d.paragraphs[0]).tabs.as_slice().len() == 2 && d.resolve_paragraph(&d.paragraphs[0]).tabs.as_slice()[0] == b, "cumulative inheritance and clear");
    let mut e = Editor::new(d.clone()).unwrap();
    e.apply(Edit::FormatStoryParagraphs { paragraph: 1, start: 0, end: 0, change: ParagraphChange::Tabs(TabStops::default()) }).unwrap();
    check!(e.document().resolve_paragraph(&e.document().paragraphs[0]).tabs.is_empty(), "clear all suppresses inherited stops");
    e.undo(); check!(*e.document() == d, "undo restores tab declarations");
    e.redo(); check!(e.document().resolve_paragraph(&e.document().paragraphs[0]).tabs.is_empty(), "redo clears inherited stops");
    let mut patch = ParagraphFormat::default(); patch.tabs.set(TabStop::clear(2880)).unwrap();
    let over = ParagraphFormat::default().overlay(patch);
    check!(over.tabs.as_slice() == &[TabStop::clear(2880)], "patch retains clear marker");
    let encoded = doc_io::encode_docx(&d).unwrap();
    check!(doc_io::decode_docx(&encoded) == Ok(d.clone()), "DOCX includes cumulative style tabs");
    let copy = d.copy_story_range(1, 0, d.plain_text().len()).unwrap();
    check!(doc_io::decode_document_fragment(&doc_io::encode_document_fragment(&copy).unwrap()) == Ok(copy), "native clipboard preserves tab inheritance");
    for alignment in [TabAlignment::Left, TabAlignment::Center, TabAlignment::Right, TabAlignment::Decimal, TabAlignment::Bar] {
        for leader in [TabLeader::None, TabLeader::Dot, TabLeader::Hyphen, TabLeader::Underscore, TabLeader::Heavy, TabLeader::MiddleDot] {
            if alignment == TabAlignment::Bar && leader != TabLeader::None { continue; }
            let mut sample = Document::default();
            sample.paragraphs[0].format.tabs = list(&[stop(1440, alignment, leader)]);
            sample.paragraphs[0].content = vec![Inline::Text(String::from("A\t12,5"))];
            check!(doc_io::decode_docx(&doc_io::encode_docx(&sample).unwrap()) == Ok(sample.clone()), "DOCX alignment and leader matrix");
            let back = doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(&sample).unwrap()).unwrap();
            check!(back.paragraphs[0].format.tabs == sample.paragraphs[0].format.tabs && back.plain_text() == sample.plain_text(), "RTF alignment and leader matrix");
            let layout = doc_layout::paginate(&sample, &mut Fixed).unwrap();
            let line = &layout.pages[0].lines[0];
            if alignment == TabAlignment::Bar {
                check!(line.bar_tabs == vec![sample.page.left as i32 + 1440] && line.fragments[2].x == sample.page.left as i32 + 720, "bar draws rule without consuming a tab stop");
            } else {
                let offset = match alignment { TabAlignment::Center => 200, TabAlignment::Right => 400, TabAlignment::Decimal => 200, _ => 0 };
                check!(line.fragments[2].x == sample.page.left as i32 + 1440 - offset, "aligned tab geometry");
                check!(line.fragments[1].text.is_empty() && line.fragments[1].source == Some((1, 2)) && line.fragments[1].leader == leader, "leader is not editable document text");
            }
        }
    }
    let rtf = br"{\rtf1\pard\tqr\tldot\tx1440\tx2880 A\tab B\par\pard C}";
    let back = doc_io::clipboard_rtf_decode(rtf).unwrap();
    check!(back.paragraphs[0].format.tabs.as_slice()[1].alignment == TabAlignment::Left && back.paragraphs[0].format.tabs.as_slice()[1].leader == TabLeader::None && back.paragraphs[1].format.tabs.is_empty(), "RTF next-stop and paragraph resets");
    let mut sample = Document::default();
    sample.paragraphs[0].format.tabs = list(&[stop(2880, TabAlignment::Decimal, TabLeader::Dot)]);
    sample.paragraphs[0].content = vec![Inline::Text(String::from("X\t12")), Inline::with_format(String::from("3.4"), TextFormat { bold: Some(true), ..Default::default() })];
    let layout = doc_layout::paginate(&sample, &mut Fixed).unwrap();
    let f = layout.pages[0].lines[0].fragments.iter().find(|f| f.text == "3.4").unwrap();
    check!(f.x + 100 == sample.page.left as i32 + 2880, "decimal alignment across differently formatted runs");
    let html = String::from_utf8(doc_io::clipboard_html_encode(&sample).unwrap()).unwrap();
    check!(html.contains("tab-stops: decimal dotted 144.00pt"), "Word HTML carries tab declarations");
    let mut note = sample.paragraphs[0].clone(); note.id = 3;
    sample.notes = vec![Note { id: 2, kind: NoteKind::Footnote, paragraphs: vec![note] }];
    sample.paragraphs[0].content.push(Inline::NoteReference(2));
    let layout = doc_layout::paginate(&sample, &mut Fixed).unwrap();
    let line = layout.pages.iter().flat_map(|p| &p.lines).find(|l| l.paragraph == 3).unwrap();
    check!(line.fragments.iter().any(|f| f.text == "3.4" && f.x + 100 == sample.page.left as i32 + 2880), "footnotes share tab layout and alignment");
    check!(doc_io::decode_docx(&doc_io::encode_docx(&sample).unwrap()) == Ok(sample), "note tab DOCX roundtrip");
    let mut wide = Document::default();
    let mut parent = StyleDefinition::new(ParagraphStyle::Named(3), "64 stops"); parent.paragraph.tabs = full;
    wide.styles.push(parent); wide.paragraphs[0].style = ParagraphStyle::Named(3);
    wide.paragraphs[0].format.tabs = list(&[stop(10000, TabAlignment::Left, TabLeader::None)]);
    check!(wide.validate() == Err(Error::Limit), "overflowing inheritance rejected, never truncated");
    for dpi in [96, 144, 192] {
        for width in [397, 794, 1588] {
            let draw = RulerDraw { paragraph_only: false, page: PageSettings::default(), paragraph: ParagraphFormat::default(), axis: RulerAxis::Horizontal,
                page_start: -35, page_width: width, dpi, font: 0, palette: RulerPalette::light(0xffeeeeee) };
            let x = draw.pixel_at(draw.page.left as i32 + 1440);
            let mut controller = RulerController::default();
            let mut input = ui::UiInput { x, y: 12 * dpi as i32 / 96, pressed: 1 << ui::BTN_LEFT, ..Default::default() };
            check!(controller.update(&draw, &input, 1, TabAlignment::Right).is_none(), "press does not commit");
            input.pressed = 0; input.released = 1 << ui::BTN_LEFT;
            let edit = controller.update(&draw, &input, 1, TabAlignment::Right);
            check!(matches!(edit, Some(RulerEdit::Paragraph(p)) if p.tabs.as_slice() == &[stop(1440, TabAlignment::Right, TabLeader::None)]), "click placement at all DPI and zoom combinations");
            input.released = 0; input.pressed = 1 << ui::BTN_LEFT;
            controller.update(&draw, &input, 1, TabAlignment::Left);
            input.pressed = 0; input.key = ui::VK_ESCAPE;
            check!(controller.update(&draw, &input, 1, TabAlignment::Left).is_none() && !controller.dragging(), "Escape cancels without mutation");
            input.key = 0; input.pressed = 1 << ui::BTN_LEFT;
            controller.update(&draw, &input, 1, TabAlignment::Left);
            input.pressed = 0; input.released = 1 << ui::BTN_LEFT;
            check!(controller.update(&draw, &input, 2, TabAlignment::Left).is_none(), "changed document context cancels stale drag");
        }
    }
    (passed, failed)
}
