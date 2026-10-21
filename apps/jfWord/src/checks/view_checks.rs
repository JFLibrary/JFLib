// Native document-view checks: theme-independent metrics/pixels and bounded geometry.
use alloc::{format, rc::Rc, string::String, vec};
use doc_core::{Document, Inline, ParagraphStyle, TextFormat};
use doc_view::{Fonts, PageDraw, PageError, PageTransform};
use ui::{Draw, Gfx, Style};

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL doc_view {}\n", $name).as_bytes());
            }
        };
    }
    let assets = style::STYLE.fonts();
    let names = style::STYLE.font_names();
    let mut ordinary = text::Text::new(assets, names);
    let mut changed_ui = text::Text::new([assets[2], assets[2], assets[0], assets[3]], names);
    ordinary.set_font(0, 13 * 256);
    changed_ui.set_font(0, 13 * 256);
    check!(
        ordinary.measure("Dokumentschrift") != changed_ui.measure("Dokumentschrift"),
        "test uses distinct GUI faces"
    );
    let normal_fonts = Fonts::new(&mut ordinary).unwrap();
    let changed_fonts = Fonts::new(&mut changed_ui).unwrap();
    let mut document = Document::default();
    document.paragraphs[0].content = vec![
        Inline::Text(String::from("Unabhängige Dokumentfonts: Grüße, αβ und Ω. ")),
        Inline::with_format(
            String::from("Fett und kursiv"),
            TextFormat {
                bold: Some(true),
                italic: Some(true),
                underline: Some(true),
                strike: Some(true),
                ..TextFormat::default()
            },
        ),
    ];
    let unchanged = document.clone();
    ordinary.set_font(text::FONT_BOLD, 19 * 256);
    changed_ui.set_font(text::FONT_REG, 31 * 256);
    let layout = Rc::new(doc_view::paginate(&document, &mut ordinary, &normal_fonts).unwrap());
    let alternate =
        Rc::new(doc_view::paginate(&document, &mut changed_ui, &changed_fonts).unwrap());
    check!(
        format!("{layout:?}") == format!("{alternate:?}"),
        "GUI font changes do not affect pagination or resolved document formatting"
    );
    check!(
        document == unchanged,
        "pagination does not mutate the document"
    );
    check!(
        ordinary.get_font() == (text::FONT_BOLD, 19 * 256)
            && changed_ui.get_font() == (text::FONT_REG, 31 * 256),
        "pagination restores GUI font state"
    );
    let page = PageDraw::new(layout.clone(), 0, normal_fonts.clone()).unwrap();
    let alternate_page = PageDraw::new(alternate, 0, changed_fonts).unwrap();
    let thumbnail = page.thumbnail(&mut ordinary, 240).unwrap();
    let other_thumbnail = alternate_page.thumbnail(&mut changed_ui, 240).unwrap();
    check!(
        thumbnail.px == other_thumbnail.px
            && thumbnail.w == other_thumbnail.w
            && thumbnail.h == other_thumbnail.h,
        "GUI fonts do not affect document pixels"
    );
    let mut canvas = gfx::Surface::new(thumbnail.w, thumbnail.h, gfx::FMT_PARGB);
    let mut dc = gfx::Dc::new(canvas.w, canvas.h);
    page.draw(
        &mut Gfx {
            dc: &mut dc,
            sf: &mut canvas,
            text: &mut ordinary,
        },
        [0, 0, thumbnail.w, thumbnail.h],
    );
    check!(
        canvas.px == thumbnail.px,
        "thumbnail and editor canvas use identical page rendering"
    );
    check!(
        ordinary.get_font() == (text::FONT_BOLD, 19 * 256),
        "render restores GUI font state"
    );
    check!(
        thumbnail
            .px
            .chunks_exact(4)
            .any(|pixel| pixel[3] == 255 && pixel[..3].iter().all(|channel| *channel < 128)),
        "thumbnail contains rendered document text"
    );
    check!(
        matches!(
            PageDraw::new(layout.clone(), usize::MAX, normal_fonts),
            Err(PageError::Page)
        ),
        "missing page is rejected"
    );
    check!(
        matches!(page.thumbnail(&mut ordinary, 0), Err(PageError::Limit))
            && matches!(page.thumbnail(&mut ordinary, 2049), Err(PageError::Limit)),
        "thumbnail allocation bounds are checked before drawing"
    );
    let format = TextFormat::default().resolve(ParagraphStyle::Normal);
    let mut preview = crate::preview::Preview::default();
    let first = preview.prepare_fonts(&mut ordinary).unwrap().get(format);
    let count = ordinary.count();
    let mut stable = true;
    for _ in 0..128 {
        preview.reset();
        let fonts = preview.prepare_fonts(&mut ordinary).unwrap();
        stable &= fonts.get(format) == first && ordinary.count() == count;
    }
    check!(
        stable,
        "document changes retain the same font registry handles"
    );
    let mut invalid_text = text::Text::new([&[]; 4], [[""; 5]; 4]);
    check!(
        Fonts::new(&mut invalid_text).is_err() && invalid_text.count() == 0,
        "invalid host font registry does not gain partial document font state"
    );
    check!(
        PageTransform::new(0, [0, 0, 100, 100]).is_none()
            && PageTransform::new(12000, [0, 0, 0, 100]).is_none(),
        "empty geometry is rejected"
    );
    for width in [79, 794, 1191, 3970] {
        let transform = PageTransform::new(11906, [17, 31, width, width * 2]).unwrap();
        let pixels = transform.to_pixels([1440, 2160]);
        let roundtrip = transform.to_page(pixels);
        let tolerance = 11906 / width + 1;
        check!(
            (roundtrip[0] - 1440).abs() <= tolerance && (roundtrip[1] - 2160).abs() <= tolerance,
            "page hit coordinates remain stable across zoom and DPI"
        );
    }
    let extreme = PageTransform::new(u32::MAX, [i32::MIN, i32::MIN, 1, 1]).unwrap();
    check!(
        extreme.to_page([i32::MAX, i32::MAX]) == [i32::MAX, i32::MAX],
        "extreme external coordinates saturate without arithmetic overflow"
    );
    check!(
        doc_view::zoom_width(11906, -1) == doc_view::zoom_width(11906, 10)
            && doc_view::zoom_width(11906, 999) == doc_view::zoom_width(11906, 500),
        "zoom limits are shared and bounded"
    );
    (passed, failed)
}
