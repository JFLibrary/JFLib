// tests -- unit tests of the text crate: the bidi layout of lines with right-to-left characters
// (bidi.rs) while shaping is off
//
//   cargo test -p jflib-text --lib
//
// The fonts: the styles' embedded "JFLib Sans" (SGF1, no Hebrew or Arabic, as in the UI) and
// doc_view's DejaVu Sans Mono (TrueType with Arabic) as the fallback font.

use crate::shaping::LINE_BIDI;
use crate::{Text, FONT_REG};

static REG: &[u8] = include_bytes!("../style_assets/assets/font_reg.bin");
static SEMI: &[u8] = include_bytes!("../style_assets/assets/font_semi.bin");
static BOLD: &[u8] = include_bytes!("../style_assets/assets/font_bold.bin");
static ICONS: &[u8] = include_bytes!("../style_assets/assets/icons.bin");
static MONO: &[u8] = include_bytes!("../doc_view/assets/DejaVuSansMono.ttf");

const SALAM: &str = "سلام";

fn text(fallback: bool) -> Text {
    let names = [["JFLib Sans", "Regular", "JFLib Sans", "", ""]; 4];
    let mut t = Text::new([REG, SEMI, BOLD, ICONS], names);
    if fallback {
        let id = t.load_mem(MONO, 0);
        assert!(id >= 0, "DejaVu Sans Mono loads");
        t.set_fallback_fonts(&[id as u32]);
    }
    t.set_font(FONT_REG, 14 << 8);
    t
}

#[test]
fn left_to_right_lines_keep_the_plain_run() {
    let mut t = text(true);
    for s in ["Backup jobs", "Größe — 12:30 (UTC)", "日本語のテキスト", "Привет, мир"] {
        let w = t.measure(s);
        let carets: alloc::vec::Vec<i32> = (0..=s.len()).filter(|&i| s.is_char_boundary(i)).map(|i| t.caret_x(s.as_bytes(), i)).collect();
        assert_ne!(t.shaping.line.kind, LINE_BIDI, "{s}: no bidi layout");
        let old = t.set_bidi(false);
        assert!(old);
        assert_eq!(t.measure(s), w, "{s}");
        let plain: alloc::vec::Vec<i32> = (0..=s.len()).filter(|&i| s.is_char_boundary(i)).map(|i| t.caret_x(s.as_bytes(), i)).collect();
        assert_eq!(carets, plain, "{s}");
        t.set_bidi(true);
    }
}

#[test]
fn arabic_is_shaped_right_to_left() {
    let mut t = text(true);
    let b = SALAM.as_bytes();
    let (start, end) = (t.caret_x(b, 0), t.caret_x(b, b.len()));
    assert_eq!(t.shaping.line.kind, LINE_BIDI);
    assert!(start > end && end == 0, "the first letter on the right: {start} {end}");
    assert!((start - t.width(SALAM)).abs() <= 1, "{start}");
    assert_eq!(t.hit(b, None, start), 0);
    // joined: the first letter is not its isolated form
    let first = t.shaping.line.glyphs.last().map(|g| g.id);
    t.caret_x("س".as_bytes(), 0);
    assert_ne!(first, t.shaping.line.glyphs.first().map(|g| g.id), "initial form of seen");
    // set_bidi(false): logical order, one glyph after the other
    t.set_bidi(false);
    assert_eq!(t.caret_x(b, 0), 0);
}

#[test]
fn a_mixed_line_runs_in_visual_order() {
    let mut t = text(true);
    let s = "abc سلام def";
    let b = s.as_bytes();
    let a0 = s.find('س').unwrap_or(0);
    let a1 = a0 + SALAM.len();
    // a left-to-right line (its first strong letter): abc, the Arabic word right to left, def
    // (at a0 and a1 the caret is where the cluster before it in visual order ends: after the
    // space before the word, left of its last letter)
    let x = |t: &mut Text, i: usize| t.caret_x(b, i);
    assert_eq!(x(&mut t, 0), 0);
    let (abc, after_first, before_last, last) = (x(&mut t, 3), x(&mut t, a0 + 2), x(&mut t, a1 - 2), x(&mut t, s.len()));
    assert!(abc < before_last && before_last < after_first && after_first < last, "{abc} {before_last} {after_first} {last}");
    // "abc " and the first Arabic letter: two places on the screen, the letter on the word's right
    let spans = t.selection_spans(b, 0, a0 + 2);
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!(spans[0].1 <= before_last && spans[1].0 == after_first && spans[1].1 > after_first, "{spans:?}");
}

#[test]
fn a_right_to_left_line_takes_the_numbers_along() {
    let mut t = text(true);
    // a right-to-left line: the number left of the word it follows, its digits in their order
    let s = "سلام 12";
    let b = s.as_bytes();
    let (word_start, one, two) = (t.caret_x(b, 0), t.caret_x(b, s.len() - 2), t.caret_x(b, s.len() - 1));
    assert!(one < two && two < word_start, "{one} {two} {word_start}");
}

#[test]
fn without_a_font_reversed_and_mirrored() {
    // no fallback font: the Arabic letters are missing, the line is still laid out right to left
    // and its parentheses are mirrored
    let mut t = text(false);
    let s = "(سلام)";
    let w = t.measure(s);
    assert_eq!(t.shaping.line.kind, LINE_BIDI);
    assert_eq!(w, t.shaping.line.width);
    let (open, close) = (t.glyph(FONT_REG, '(' as u32), t.glyph(FONT_REG, ')' as u32));
    let glyphs = &t.shaping.line.glyphs;
    assert_eq!(glyphs.first().map(|g| g.id), Some(open), "the closing parenthesis, mirrored, on the left");
    assert_eq!(glyphs.last().map(|g| g.id), Some(close));
    assert!(glyphs.windows(2).all(|g| g[0].x < g[1].x));
}

#[test]
fn bidi_lines_are_kept() {
    let mut t = text(true);
    let a = "שלום";
    let b = SALAM;
    let wa = t.measure(a);
    let wb = t.measure(b);
    assert_eq!(t.shaping.lines.len(), 1);
    assert_eq!(t.measure(a), wa);
    assert_eq!(t.shaping.line.text, a);
    assert_eq!(t.measure(b), wb);
    assert_eq!(t.shaping.lines.len(), 1, "swapped, not laid out again");
}
