// sample -- the sample document with every feature of lib/pdf: pages of three sizes in both
// origins, fill rules, strokes (caps, joins, miter limit, dashes), curves, transforms,
// clipping, alpha, images (JPEG passed through, RGBA with a soft mask, gray scaled with and
// without interpolation), text in TrueType and CFF subsets (kerning, ligatures, Greek,
// Cyrillic, Arabic, a fallback font, stroked and invisible text), links to a URI and to pages,
// gradients (axial and radial, hard edges, varying alpha as a soft mask, turned) and hatches
// (angles, crossing, turned, in both origins), nested outlines and the document information.
// Page 5 is laid out for reader_check.py's pixel checks (see the comments there).  Fonts only
// from the repository: DejaVu
// (lib/doc_view/assets, DejaVu licence), Inter (styles/netlab/assets, SIL OFL 1.1) and the
// synthetic CFF fixtures of apps/fonttest.

use alloc::vec::Vec;

use pdf::{Date, Dest, Document, Error, FillRule, FontId, Hatch, Image, Info, LineCap, LineJoin, Origin, Rgb, Shading, TextMode, A4, LETTER};

pub const SERIF: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSerif.ttf");
pub const MONO: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSansMono.ttf");
pub const INTER: &[u8] = include_bytes!("../../../styles/netlab/assets/Inter-SemiBold.ttf");
pub const MINI_OTF: &[u8] = include_bytes!("../../fonttest/tests/fixtures/mini.otf");
pub const CID_OTF: &[u8] = include_bytes!("../../fonttest/tests/fixtures/cid.otf");

pub const URI: &str = "https://github.com/JFLibrary/JFLib";

pub struct Fonts {
    pub serif: FontId,
    pub mono: FontId,
    pub inter: FontId,
    pub mini: FontId,
    pub cid: FontId,
}

pub fn fonts(doc: &mut Document) -> Result<Fonts, Error> {
    Ok(Fonts {
        serif: doc.add_font(SERIF.to_vec(), 0)?,
        mono: doc.add_font(MONO.to_vec(), 0)?,
        inter: doc.add_font(INTER.to_vec(), 0)?,
        mini: doc.add_font(MINI_OTF.to_vec(), 0)?,
        cid: doc.add_font(CID_OTF.to_vec(), 0)?,
    })
}

/// a JPEG of w x h: a sky gradient with a sun (img_jpeg's encoder)
pub fn test_jpeg(w: i32, h: i32) -> Vec<u8> {
    let mut s = gfx::Surface::new(w, h, gfx::FMT_XRGB);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x - w * 3 / 4, y - h / 3);
            let sun = dx * dx + dy * dy < (h / 6) * (h / 6);
            let (r, g, b) = if sun {
                (255, 220, 60)
            } else if y > h * 2 / 3 {
                (40 + x * 60 / w, 120 + (y - h * 2 / 3) * 80 / h, 40)
            } else {
                (60 + y * 120 / h, 120 + y * 100 / h, 230)
            };
            let o = (y * s.stride + x * 4) as usize;
            s.px[o..o + 4].copy_from_slice(&[b as u8, g as u8, r as u8, 255]);
        }
    }
    img_jpeg::encode(&s, 85).unwrap_or_default()
}

/// RGBA w x h: hue across, alpha falling off from the centre
pub fn test_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as i32 - w as i32 / 2, y as i32 - h as i32 / 2);
            let d = ((cx * cx + cy * cy) * 255 / ((w as i32 / 2) * (w as i32 / 2))).min(255);
            px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 200, (255 - d) as u8]);
        }
    }
    px
}

fn star(c: &mut pdf::Canvas, cx: f64, cy: f64, r: f64) {
    // a five-pointed star drawn as one self-crossing path
    let pts = [(0.0, -1.0), (0.588, 0.809), (-0.951, -0.309), (0.951, -0.309), (-0.588, 0.809)];
    c.move_to(cx + pts[0].0 * r, cy + pts[0].1 * r);
    for p in &pts[1..] {
        c.line_to(cx + p.0 * r, cy + p.1 * r);
    }
    c.close();
}

fn heading(c: &mut pdf::Canvas, f: &Fonts, x: f64, y: f64, s: &str) {
    c.set_fill_color(Rgb::hex(0x1F3A60));
    c.text(f.inter, 13.0, x, y, s);
    c.set_fill_color(Rgb::BLACK);
}

fn label(c: &mut pdf::Canvas, f: &Fonts, x: f64, y: f64, s: &str) {
    c.set_fill_color(Rgb::hex(0x404040));
    c.text(f.serif, 8.0, x, y, s);
    c.set_fill_color(Rgb::BLACK);
}

pub fn document() -> Result<Document, Error> {
    let mut doc = Document::new();
    doc.set_info(Info {
        title: Some("JFLib PDF – Beispiel".into()),
        author: Some("JFLib".into()),
        subject: Some("Alle Funktionen des Vektor-PDF-Schreibers".into()),
        keywords: Some("PDF, Vektor, Schriften, Lesezeichen".into()),
        creator: Some("pdftest".into()),
        producer: None,
        created: Some(Date { year: 2026, month: 11, day: 16, hour: 12, minute: 0, second: 0, offset_minutes: 60 }),
        modified: Some(Date { year: 2026, month: 11, day: 16, hour: 12, minute: 30, second: 5, offset_minutes: 60 }),
    });
    doc.set_language("de-DE");
    let f = fonts(&mut doc)?;
    let p1 = doc.add_page(A4.0, A4.1);
    let p2 = doc.add_page(A4.0, A4.1);
    let p3 = doc.add_page(LETTER.1, LETTER.0);
    let p4 = doc.add_page(pdf::mm(100.0), pdf::mm(100.0));
    let p5 = doc.add_page(A4.0, A4.1);
    let jpeg = doc.add_image(Image::jpeg(test_jpeg(240, 150))?);
    let rgba = doc.add_image(Image::rgba(64, 64, &test_rgba(64, 64))?);
    let gray_px: Vec<u8> = (0..64).map(|i| if (i / 8 + i % 8) % 2 == 0 { 40 } else { 200 } + (i as u8 % 8) * 6).collect();
    let gray = doc.add_image(Image::gray(8, 8, &gray_px)?);
    let gray_smooth = doc.add_image(Image::gray(8, 8, &gray_px)?.interpolate(true));

    let (d_fill, d_stroke, d_curves, d_clip);
    // ------------------------------------------------------------ page 1: vector graphics
    {
        let mut c = doc.canvas(p1, Origin::TopLeft);
        c.set_fill_color(Rgb::hex(0x1F3A60));
        c.text(f.inter, 22.0, 50.0, 70.0, "JFLib PDF – Vektor-PDF-Schreiber");
        c.set_fill_color(Rgb::hex(0x404040));
        c.text(f.serif, 10.0, 50.0, 90.0, "Seite 1: Füllregeln, Konturen, Kurven, Transformationen, Clipping und Transparenz.");

        d_fill = c.dest(50.0, 110.0);
        heading(&mut c, &f, 50.0, 125.0, "1.1 Füllregeln");
        c.set_fill_color(Rgb::hex(0x3070C0));
        c.set_stroke_color(Rgb::hex(0x102040));
        c.set_line_width(1.5);
        star(&mut c, 110.0, 190.0, 50.0);
        c.fill_stroke(FillRule::NonZero);
        star(&mut c, 250.0, 190.0, 50.0);
        c.fill_stroke(FillRule::EvenOdd);
        // a ring: two circles, even-odd leaves the hole
        c.set_fill_color(Rgb::hex(0xC05030));
        c.circle(400.0, 190.0, 45.0);
        c.circle(400.0, 190.0, 25.0);
        c.fill(FillRule::EvenOdd);
        label(&mut c, &f, 80.0, 255.0, "non-zero");
        label(&mut c, &f, 225.0, 255.0, "even-odd");
        label(&mut c, &f, 370.0, 255.0, "Ring (even-odd)");

        d_stroke = c.dest(50.0, 270.0);
        heading(&mut c, &f, 50.0, 285.0, "1.2 Konturen: Kappen, Verbindungen, Gehrung, Strichmuster");
        c.set_stroke_color(Rgb::hex(0x206030));
        c.set_line_width(10.0);
        for (i, cap) in [LineCap::Butt, LineCap::Round, LineCap::Square].iter().enumerate() {
            let y = 310.0 + i as f64 * 22.0;
            c.set_line_cap(*cap);
            c.move_to(70.0, y);
            c.line_to(170.0, y);
            c.stroke();
        }
        c.set_line_cap(LineCap::Butt);
        for (i, join) in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel].iter().enumerate() {
            let x = 210.0 + i as f64 * 70.0;
            c.set_line_join(*join);
            c.move_to(x, 360.0);
            c.line_to(x + 25.0, 310.0);
            c.line_to(x + 50.0, 360.0);
            c.stroke();
        }
        // the miter limit: the same sharp corner, limit 10 (pointed) and 1.5 (bevelled)
        c.set_line_join(LineJoin::Miter);
        c.set_line_width(6.0);
        for (i, limit) in [10.0, 1.5].iter().enumerate() {
            let x = 430.0 + i as f64 * 55.0;
            c.set_miter_limit(*limit);
            c.move_to(x, 365.0);
            c.line_to(x + 12.0, 310.0);
            c.line_to(x + 24.0, 365.0);
            c.stroke();
        }
        c.set_miter_limit(10.0);
        c.set_line_width(2.0);
        c.set_stroke_color(Rgb::BLACK);
        for (i, (pat, phase)) in [(&[6.0, 3.0][..], 0.0), (&[1.0, 4.0][..], 0.0), (&[12.0, 3.0, 2.0, 3.0][..], 5.0)].iter().enumerate() {
            let y = 390.0 + i as f64 * 14.0;
            c.set_dash(pat, *phase);
            if i == 1 {
                c.set_line_cap(LineCap::Round);
            }
            c.move_to(70.0, y);
            c.line_to(540.0, y);
            c.stroke();
            c.set_line_cap(LineCap::Butt);
        }
        c.set_dash(&[], 0.0);
        label(&mut c, &f, 70.0, 380.0, "butt, round, square · miter, round, bevel · Gehrungsgrenze 10 und 1,5 · Strichmuster");

        d_curves = c.dest(50.0, 435.0);
        heading(&mut c, &f, 50.0, 450.0, "1.3 Kurven und Transformationen");
        c.set_stroke_color(Rgb::hex(0x803080));
        c.set_line_width(2.5);
        c.move_to(60.0, 560.0);
        c.curve_to(90.0, 460.0, 150.0, 620.0, 190.0, 500.0);
        c.quad_to(210.0, 460.0, 230.0, 540.0);
        c.stroke();
        c.set_fill_color(Rgb::hex(0xF0C040));
        c.ellipse(290.0, 520.0, 40.0, 25.0);
        c.fill_stroke(FillRule::NonZero);
        c.set_line_cap(LineCap::Round);
        c.arc(370.0, 520.0, 30.0, 30.0, 0.0, 4.5);
        c.stroke();
        c.set_line_cap(LineCap::Butt);
        for k in 0..6 {
            c.save();
            c.translate(470.0, 520.0);
            c.rotate(k as f64 * core::f64::consts::PI / 12.0);
            c.set_fill_color(Rgb::new(0.2 + k as f64 * 0.13, 0.4, 0.8 - k as f64 * 0.1));
            c.set_fill_alpha(0.55);
            c.rounded_rect(-30.0, -30.0, 60.0, 60.0, 8.0);
            c.fill(FillRule::NonZero);
            c.restore();
        }

        d_clip = c.dest(50.0, 590.0);
        heading(&mut c, &f, 50.0, 605.0, "1.4 Clipping und Transparenz");
        c.save();
        c.circle(120.0, 690.0, 60.0);
        c.clip(FillRule::NonZero);
        for k in 0..12 {
            c.set_fill_color(if k % 2 == 0 { Rgb::hex(0x2050A0) } else { Rgb::hex(0xE0E8F0) });
            c.rect(60.0 + k as f64 * 10.0, 620.0, 10.0, 140.0);
            c.fill(FillRule::NonZero);
        }
        c.restore();
        for (i, col) in [0xE03030u32, 0x30A040, 0x3050E0].iter().enumerate() {
            let a = i as f64 * 2.094;
            c.set_fill_color(Rgb::hex(*col));
            c.set_fill_alpha(0.5);
            c.circle(300.0 + 28.0 * gfx::xform::sincos(a).1, 690.0 + 28.0 * gfx::xform::sincos(a).0, 45.0);
            c.fill(FillRule::NonZero);
        }
        c.set_fill_alpha(1.0);
        c.set_stroke_color(Rgb::hex(0x000000));
        c.set_stroke_alpha(0.3);
        c.set_line_width(8.0);
        c.rect(420.0, 640.0, 110.0, 100.0);
        c.stroke();
        c.set_stroke_alpha(1.0);
        label(&mut c, &f, 60.0, 775.0, "Streifen in einem Kreis geclippt · drei Kreise mit Alpha 0,5 · Kontur mit Alpha 0,3");

        c.set_fill_color(Rgb::hex(0x2060C0));
        let w = c.text(f.serif, 10.0, 50.0, 805.0, "Weiter zu Bildern und Text (Seite 2)");
        c.link_dest(50.0, 795.0, w, 13.0, Dest::Fit(p2));
    }
    // ------------------------------------------------------------ page 2: images and text
    let (d_images, d_text);
    {
        let mut c = doc.canvas(p2, Origin::TopLeft);
        d_images = c.dest(50.0, 40.0);
        heading(&mut c, &f, 50.0, 60.0, "2.1 Bilder");
        c.image(jpeg, 50.0, 75.0, 240.0, 150.0);
        label(&mut c, &f, 50.0, 238.0, "JPEG, unverändert eingebettet (DCTDecode)");
        // a checkerboard under the RGBA image shows its alpha
        for y in 0..6 {
            for x in 0..6 {
                c.set_fill_color(if (x + y) % 2 == 0 { Rgb::hex(0xCCCCCC) } else { Rgb::WHITE });
                c.rect(320.0 + x as f64 * 20.0, 75.0 + y as f64 * 20.0, 20.0, 20.0);
                c.fill(FillRule::NonZero);
            }
        }
        c.image(rgba, 320.0, 75.0, 120.0, 120.0);
        label(&mut c, &f, 320.0, 210.0, "RGBA, Alpha als SMask");
        c.image(gray, 460.0, 75.0, 50.0, 50.0);
        c.image(gray_smooth, 460.0, 135.0, 50.0, 50.0);
        label(&mut c, &f, 460.0, 200.0, "8×8 grau, mit");
        label(&mut c, &f, 460.0, 210.0, "/Interpolate unten");

        d_text = c.dest(50.0, 255.0);
        heading(&mut c, &f, 50.0, 275.0, "2.2 Text mit eingebetteten Untermengen");
        let lines = [
            "Unterschneidung: AVATAR, Wave To, Tag, Yo – Ligaturen: Grafik, Pfiff, Auflage, office.",
            "Umlaute und Zeichen: Größe, Übermaß, Ärger – „Anführungen“ … 3 × 4 ≥ 10 € ¶ §",
            "Griechisch: Καλημέρα κόσμε · Kyrillisch: Здравствуй, мир",
        ];
        for (i, l) in lines.iter().enumerate() {
            c.text(f.serif, 11.0, 50.0, 300.0 + i as f64 * 16.0, l);
        }
        c.text(f.mono, 10.0, 50.0, 352.0, "fn main() { println!(\"Hallo\"); }  // DejaVu Sans Mono");
        let w = c.text(f.mono, 12.0, 50.0, 372.0, "Arabisch (von rechts): ");
        c.text(f.mono, 12.0, 50.0 + w, 372.0, "السلام عليكم");
        let w = c.text_fallback(&[f.inter, f.mono], 12.0, 50.0, 392.0, "Inter mit Ersatzschrift: مرحبا");
        label(&mut c, &f, 60.0 + w, 392.0, "(Arabisch aus DejaVu Sans Mono)");
        c.text(f.mini, 24.0, 50.0, 430.0, "AVOBFGJQ");
        c.text(f.cid, 24.0, 250.0, 430.0, "ABAB");
        label(&mut c, &f, 50.0, 445.0, "CFF-Schriften (synthetische Testschriften): mini.otf mit Kerning AV, cid.otf (CID-keyed)");
        c.set_text_mode(TextMode::Stroke);
        c.set_stroke_color(Rgb::hex(0xA02020));
        c.set_line_width(0.6);
        c.text(f.inter, 30.0, 50.0, 490.0, "Kontur-Text");
        c.set_text_mode(TextMode::FillStroke);
        c.set_fill_color(Rgb::hex(0xF0D070));
        c.text(f.inter, 30.0, 260.0, 490.0, "Füllen + Kontur");
        c.set_text_mode(TextMode::Invisible);
        c.text(f.serif, 10.0, 50.0, 510.0, "Unsichtbarer Text: suchbar und kopierbar");
        c.set_text_mode(TextMode::Fill);
        label(&mut c, &f, 50.0, 522.0, "(Hier steht unsichtbarer Text – die Suche nach „unsichtbar“ findet ihn.)");
        // rotated text keeps upright glyphs relative to its own baseline
        c.save();
        c.translate(420.0, 600.0);
        c.rotate(-0.4);
        c.set_fill_color(Rgb::hex(0x206080));
        c.text(f.serif, 14.0, 0.0, 0.0, "gedrehter Text");
        c.restore();

        heading(&mut c, &f, 50.0, 580.0, "2.3 Links");
        c.set_fill_color(Rgb::hex(0x2060C0));
        let w = c.text(f.serif, 11.0, 50.0, 605.0, URI);
        c.set_stroke_color(Rgb::hex(0x2060C0));
        c.set_line_width(0.6);
        c.move_to(50.0, 607.0);
        c.line_to(50.0 + w, 607.0);
        c.stroke();
        c.link_uri(50.0, 595.0, w, 14.0, URI);
        let w = c.text(f.serif, 11.0, 50.0, 625.0, "Zur Querformat-Seite (Seite 3)");
        c.link_dest(50.0, 615.0, w, 14.0, Dest::Fit(p3));
        let w = c.text(f.serif, 11.0, 50.0, 645.0, "Zurück zu 1.3 Kurven");
        c.link_dest(50.0, 635.0, w, 14.0, d_curves);
    }
    // ------------------------------------------------------------ page 3: Letter landscape, y up
    let d_p3;
    {
        let mut c = doc.canvas(p3, Origin::BottomLeft);
        let (w, h) = c.page_size();
        d_p3 = c.dest(40.0, h - 30.0);
        c.set_fill_color(Rgb::hex(0x1F3A60));
        c.text(f.inter, 20.0, 40.0, h - 60.0, "Seite 3: US Letter quer, Ursprung unten links");
        c.set_fill_color(Rgb::BLACK);
        c.text(f.serif, 11.0, 40.0, h - 85.0, "Hier wächst y nach oben (PDF-Seitenraum); Text steht trotzdem aufrecht.");
        // axes
        c.set_stroke_color(Rgb::hex(0x808080));
        c.set_line_width(1.0);
        c.move_to(40.0, 40.0);
        c.line_to(w - 40.0, 40.0);
        c.move_to(40.0, 40.0);
        c.line_to(40.0, h - 120.0);
        c.stroke();
        c.text(f.serif, 9.0, w - 60.0, 28.0, "x");
        c.text(f.serif, 9.0, 28.0, h - 130.0, "y");
        // a sine-like curve and rotated labels around a centre
        c.set_stroke_color(Rgb::hex(0xC03030));
        c.set_line_width(2.0);
        c.move_to(60.0, 200.0);
        for k in 1..=8 {
            let x = 60.0 + k as f64 * 80.0;
            let up = if k % 2 == 1 { 120.0 } else { -120.0 };
            c.curve_to(x - 50.0, 200.0 + up, x - 30.0, 200.0 + up, x, 200.0);
        }
        c.stroke();
        for k in 0..8 {
            c.save();
            c.translate(560.0, 380.0);
            c.rotate(k as f64 * core::f64::consts::FRAC_PI_4);
            c.set_fill_color(Rgb::new(k as f64 / 8.0, 0.3, 1.0 - k as f64 / 8.0));
            c.text(f.inter, 12.0, 20.0, -4.0, "Drehung");
            c.restore();
        }
        c.set_fill_color(Rgb::hex(0x2060C0));
        let tw = c.text(f.serif, 11.0, 40.0, 60.0, "Zurück zum Anfang (Seite 1)");
        c.link_dest(40.0, 56.0, tw, 14.0, Dest::Fit(p1));
    }
    // ------------------------------------------------------------ page 4: 100 x 100 mm
    let d_p4;
    {
        let mut c = doc.canvas(p4, Origin::TopLeft);
        let (w, h) = c.page_size();
        d_p4 = c.dest(0.0, 0.0);
        c.set_fill_color(Rgb::hex(0xEEF3F8));
        c.rect(0.0, 0.0, w, h);
        c.fill(FillRule::NonZero);
        c.set_stroke_color(Rgb::hex(0x1F3A60));
        c.set_line_width(2.0);
        c.rect(10.0, 10.0, w - 20.0, h - 20.0);
        c.stroke();
        c.set_fill_color(Rgb::hex(0x1F3A60));
        c.text(f.inter, 14.0, 22.0, 40.0, "100 × 100 mm");
        c.set_fill_color(Rgb::BLACK);
        c.text(f.serif, 9.0, 22.0, 58.0, "Jede Seite hat ihre eigene Größe.");
        c.image(jpeg, 22.0, 70.0, 120.0, 75.0);
    }
    // ------------------------------------------------------------ page 5: gradients and hatches
    let d_p5 = gradients_page(&mut doc, &f, p5);
    // ------------------------------------------------------------ outlines
    let o1 = doc.add_outline(None, "1 Vektorgrafik", Dest::Fit(p1));
    doc.add_outline(Some(o1), "1.1 Füllregeln", d_fill);
    doc.add_outline(Some(o1), "1.2 Konturen", d_stroke);
    doc.add_outline(Some(o1), "1.3 Kurven und Transformationen", d_curves);
    doc.add_outline(Some(o1), "1.4 Clipping und Transparenz", d_clip);
    let o2 = doc.add_outline(None, "2 Bilder, Text und Links", Dest::Fit(p2));
    doc.add_outline(Some(o2), "2.1 Bilder", d_images);
    let o22 = doc.add_outline(Some(o2), "2.2 Text", d_text);
    doc.add_outline(Some(o22), "Arabisch und Ersatzschrift", Dest::Xyz { page: p2, left: 50.0, top: A4.1 - 360.0, zoom: Some(2.0) });
    doc.set_outline_open(o2, false);
    doc.add_outline(None, "3 Querformat (Letter, y nach oben)", d_p3);
    doc.add_outline(None, "4 Kleine Seite (100 × 100 mm)", d_p4);
    doc.add_outline(None, "5 Verläufe und Schraffuren", d_p5);
    Ok(doc)
}

/// page 5 (A4): gradients and hatches.  reader_check.py samples pixels of it at 72 dpi (one pixel
/// per point, the top-left canvas's coordinates); keep the places it names in step.
fn gradients_page(doc: &mut Document, f: &Fonts, page: pdf::PageId) -> Dest {
    let red = Rgb::new(1.0, 0.0, 0.0);
    let blue = Rgb::new(0.0, 0.0, 1.0);
    let d;
    {
        let mut c = doc.canvas(page, Origin::TopLeft);
        d = c.dest(0.0, 0.0);
        c.set_fill_color(Rgb::hex(0x1F3A60));
        c.text(f.inter, 20.0, 50.0, 60.0, "Seite 5: Verläufe und Schraffuren");

        heading(&mut c, f, 50.0, 100.0, "5.1 Lineare Verläufe");
        // red to blue over x 50 .. 250 (checked at x 52, 150, 248, y 145)
        c.rect(50.0, 115.0, 200.0, 60.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([50.0, 145.0], [250.0, 145.0], &[(0.0, red, 1.0), (1.0, blue, 1.0)]));
        // four stops with a hard edge at x 420 (checked at x 416 and 424)
        c.rect(300.0, 115.0, 240.0, 60.0);
        let four = [(0.0, Rgb::hex(0xFFE040), 1.0), (0.5, Rgb::hex(0xF08000), 1.0), (0.5, Rgb::hex(0x20A040), 1.0), (1.0, Rgb::hex(0x104020), 1.0)];
        c.fill_shading(FillRule::NonZero, &Shading::axial([300.0, 0.0], [540.0, 0.0], &four));
        label(&mut c, f, 50.0, 188.0, "zwei Stufen, rot nach blau · vier Stufen mit harter Kante in der Mitte");

        heading(&mut c, f, 50.0, 215.0, "5.2 Transparenz und Drehung");
        // alpha 1 to 0 on the white page (checked at x 52, 150, 248, y 260)
        c.rect(50.0, 230.0, 200.0, 60.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([50.0, 0.0], [250.0, 0.0], &[(0.0, Rgb::hex(0x2050C0), 1.0), (1.0, Rgb::hex(0x2050C0), 0.0)]));
        // three stops of varying alpha over a checkerboard
        for y in 0..3 {
            for x in 0..12 {
                c.set_fill_color(if (x + y) % 2 == 0 { Rgb::hex(0xBBBBBB) } else { Rgb::WHITE });
                c.rect(300.0 + x as f64 * 20.0, 230.0 + y as f64 * 20.0, 20.0, 20.0);
                c.fill(FillRule::NonZero);
            }
        }
        c.rect(300.0, 230.0, 240.0, 60.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([300.0, 0.0], [540.0, 0.0], &[(0.0, red, 1.0), (0.5, Rgb::hex(0x20C040), 0.25), (1.0, blue, 1.0)]));
        // one alpha for all stops (ExtGState ca), turned with the canvas
        c.save();
        c.translate(150.0, 345.0);
        c.rotate(-0.25);
        c.rounded_rect(-90.0, -30.0, 180.0, 60.0, 12.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([-90.0, 0.0], [90.0, 0.0], &[(0.0, Rgb::hex(0x803080), 0.6), (0.5, Rgb::hex(0xF0C040), 0.6), (1.0, Rgb::hex(0x208080), 0.6)]));
        c.restore();
        // the canvas's fill alpha times a soft mask, turned the other way
        c.save();
        c.translate(420.0, 345.0);
        c.rotate(0.25);
        c.set_fill_alpha(0.7);
        c.rounded_rect(-100.0, -30.0, 200.0, 60.0, 30.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([0.0, -30.0], [0.0, 30.0], &[(0.0, Rgb::hex(0xC03030), 0.0), (1.0, Rgb::hex(0xC03030), 1.0)]));
        c.restore();
        label(&mut c, f, 50.0, 412.0, "Alpha 1 bis 0 · Alpha 1, 0,25, 1 · gedreht mit Alpha 0,6 · Füll-Alpha 0,7 mal Verlaufs-Alpha");

        heading(&mut c, f, 50.0, 430.0, "5.3 Radiale Verläufe");
        // white centre to dark blue at radius 60 (checked at the centre 120, 510 and at radius 57)
        c.circle(120.0, 510.0, 60.0);
        c.fill_shading(FillRule::NonZero, &Shading::radial([120.0, 510.0], 0.0, [120.0, 510.0], 60.0, &[(0.0, Rgb::WHITE, 1.0), (1.0, Rgb::hex(0x102060), 1.0)]));
        // an offset inner circle, four stops, extended to the rectangle's corners (checked at 212, 452)
        c.rect(210.0, 450.0, 140.0, 120.0);
        let rings = [(0.0, Rgb::hex(0xFFF0A0), 1.0), (0.3, Rgb::hex(0xF09030), 1.0), (0.7, Rgb::hex(0xC03020), 1.0), (1.0, Rgb::hex(0x401010), 1.0)];
        c.fill_shading(FillRule::NonZero, &Shading::radial([265.0, 495.0], 6.0, [280.0, 510.0], 55.0, &rings));
        // shade() paints the whole clip: here an ellipse
        c.save();
        c.ellipse(460.0, 510.0, 80.0, 55.0);
        c.clip(FillRule::NonZero);
        c.shade(&Shading::radial([460.0, 510.0], 10.0, [460.0, 510.0], 80.0, &[(0.0, Rgb::hex(0x30C0F0), 1.0), (1.0, Rgb::hex(0x2040A0), 1.0)]));
        c.restore();
        label(&mut c, f, 50.0, 588.0, "von der Mitte nach außen · versetzter Innenkreis, vier Stufen, fortgesetzt · shade() in einer Ellipse");

        heading(&mut c, f, 50.0, 615.0, "5.4 Schraffuren");
        // horizontal lines 3 wide every 8 through y 640.5 (checked at y 640 line, 644 gap, x 120)
        c.rect(50.0, 627.0, 140.0, 80.0);
        c.fill_hatch(FillRule::NonZero, &Hatch { origin: [0.0, 640.5], ..Hatch::new(0.0, 8.0, 3.0, Rgb::hex(0xA02020)) });
        c.set_stroke_color(Rgb::hex(0x404040));
        c.set_line_width(1.0);
        c.rect(50.0, 627.0, 140.0, 80.0);
        c.stroke();
        // two crossing families
        c.rect(210.0, 627.0, 140.0, 80.0);
        c.fill_hatch(FillRule::NonZero, &Hatch::new(core::f64::consts::FRAC_PI_4, 10.0, 1.2, Rgb::hex(0x2050C0)));
        c.rect(210.0, 627.0, 140.0, 80.0);
        c.fill_hatch(FillRule::NonZero, &Hatch::new(-core::f64::consts::FRAC_PI_4, 10.0, 1.2, Rgb::hex(0x2050C0)));
        c.rect(210.0, 627.0, 140.0, 80.0);
        c.stroke();
        // turned with the canvas, alpha 0.6, in an ellipse; a star with the even-odd rule
        c.save();
        c.translate(450.0, 667.0);
        c.rotate(0.3);
        c.ellipse(0.0, 0.0, 85.0, 38.0);
        c.fill_hatch(FillRule::NonZero, &Hatch { alpha: 0.6, ..Hatch::new(0.0, 6.0, 2.0, Rgb::hex(0x208040)) });
        c.ellipse(0.0, 0.0, 85.0, 38.0);
        c.stroke();
        c.restore();
        star(&mut c, 557.0, 667.0, 30.0);
        c.fill_hatch(FillRule::EvenOdd, &Hatch::new(1.2, 4.0, 1.0, Rgb::BLACK));
        star(&mut c, 557.0, 667.0, 30.0);
        c.set_line_width(0.5);
        c.stroke();
        label(&mut c, f, 50.0, 722.0, "waagrecht · gekreuzt (±45°) · gedreht mit Alpha 0,6 · Stern mit even-odd");
    }
    {
        // the same page with y upwards: angles turn counter-clockwise here
        let mut c = doc.canvas(page, Origin::BottomLeft);
        c.set_fill_color(Rgb::hex(0x404040));
        c.text(f.serif, 8.0, 50.0, 20.0, "Ursprung unten links: Schraffur 30° (gegen den Uhrzeigersinn), Verlauf nach oben");
        c.rect(50.0, 35.0, 200.0, 70.0);
        c.fill_hatch(FillRule::NonZero, &Hatch::new(core::f64::consts::PI / 6.0, 7.0, 1.5, Rgb::hex(0x6030A0)));
        c.set_stroke_color(Rgb::hex(0x404040));
        c.rect(50.0, 35.0, 200.0, 70.0);
        c.stroke();
        c.rect(300.0, 35.0, 240.0, 70.0);
        c.fill_shading(FillRule::NonZero, &Shading::axial([0.0, 35.0], [0.0, 105.0], &[(0.0, Rgb::hex(0x104020), 1.0), (1.0, Rgb::hex(0xE0F0E0), 1.0)]));
    }
    d
}

pub fn build() -> Result<Vec<u8>, Error> {
    document()?.write()
}
