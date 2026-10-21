// Rich clipboard contracts against the native, platform-independent RTF codec.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{
    Alignment, Document, Inline, Paragraph, ParagraphFormat, ParagraphStyle, TextColor, TextFormat,
};
use doc_io::{clipboard_rtf_decode as decode, clipboard_rtf_encode as encode};

fn text(bytes: &[u8]) -> Option<String> {
    decode(bytes).ok().map(|d| {
        d.paragraphs
            .iter()
            .map(|p| d.paragraph_text(p))
            .collect::<Vec<_>>()
            .join("\n")
    })
}
fn visible(d: &Document) -> Vec<(String, Vec<doc_core::ResolvedFormat>, ParagraphFormat)> {
    d.paragraphs
        .iter()
        .map(|p| {
            let formats = p
                .content
                .iter()
                .flat_map(|i| {
                    i.text()
                        .unwrap_or("")
                        .chars()
                        .map(move |_| i.format().resolve(p.style))
                })
                .collect();
            (d.paragraph_text(p), formats, p.format)
        })
        .collect()
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL clipboard codec {}\n", $name).as_bytes());
            }
        };
    }
    let mut d = Document::default();
    d.paragraphs[0].style = ParagraphStyle::Heading1;
    d.paragraphs[0].format = ParagraphFormat {
        explicit: 0,
        alignment: Alignment::Justify,
        before: 120,
        after: 240,
        line: 360,
        left: 720,
        right: 360,
        first_line: -360,
        page_break_before: true,
        tabs: Default::default(),
    };
    d.paragraphs[0].content = vec![
        Inline::Text(String::from(
            "Grüße αβ 𝄞 😀 {braces} \\ slash\tTab\nSoft break ",
        )),
        Inline::StyledText {
            text: String::from("formatted"),
            format: TextFormat {
                font_family: None,
                bold: Some(false),
                italic: Some(true),
                underline: Some(true),
                strike: Some(true),
                color: Some(TextColor::Rgb(0x8b2fb0)),
                size_half_points: Some(23),
                ..Default::default()
            },
        },
    ];
    d.paragraphs.push(Paragraph {
        id: 2,
        style: ParagraphStyle::Title,
        format: ParagraphFormat {
        explicit: 0,
            alignment: Alignment::Center,
            first_line: 240,
            ..ParagraphFormat::default()
        },
        content: vec![Inline::Text(String::from("Title"))],
    });
    d.paragraphs.push(Paragraph {
        id: 3,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content: vec![Inline::Text(String::new())],
    });
    let encoded = encode(&d).unwrap();
    let decoded = decode(&encoded);
    check!(
        decoded.as_ref().is_ok_and(|v| visible(v) == visible(&d)),
        "rendered formatting and Unicode round trip"
    );
    check!(
        encoded.is_ascii() && encoded.windows(8).any(|v| v == b"\\fi-360\\"),
        "portable ASCII RTF and hanging indent"
    );
    check!(
        encoded.windows(6).any(|v| v == b"\\line ") && encoded.windows(5).any(|v| v == b"\\par\n"),
        "soft and hard breaks have separate controls"
    );
    check!(encode(&d) == Ok(encoded.clone()), "deterministic output");
    check!(
        text(br"{\rtf1\ansi Hello\par World\par}") == Some(String::from("Hello\nWorld")),
        "final paragraph marker adds no phantom paragraph"
    );
    check!(
        text(br"{\rtf1 A\par\par}") == Some(String::from("A\n")),
        "explicit final blank paragraph retained"
    );
    check!(
        text(br"{\rtf1}") == Some(String::new()),
        "empty fragment remains editable"
    );
    check!(
        text(br"{\rtf1\ansi\ansicpg1252 \'80 \'e4 \'93Hi\'94}") == Some(String::from("€ ä “Hi”")),
        "Windows 1252 escaped bytes"
    );
    check!(
        text(b"{\\rtf1\\ansi raw \xe9}") == Some(String::from("raw é")),
        "Windows 1252 raw bytes"
    );
    check!(
        text(br"{\rtf1\uc1 \u945?\u-10188?\u-8930?}") == Some(String::from("α𝄞")),
        "signed UTF16 surrogate pair"
    );
    check!(
        text(br"{\rtf1\uc1 \u945\'61\u946\~\u947\tab X}") == Some(String::from("αβγX")),
        "fallback escaped bytes symbols and words count once"
    );
    check!(
        text(br"{\rtf1\uc2 \u945ab{\uc0\u946}\u947cd}") == Some(String::from("αβγ")),
        "unicode fallback count restores after group"
    );
    check!(
        text(br"{\rtf1\uc3 \u945?{B}C}") == Some(String::from("αBC")),
        "group boundary terminates unicode fallback"
    );
    check!(
        text(br"{\rtf1\uc0 \u945\u946\u947}") == Some(String::from("αβγ")),
        "zero fallback unicode"
    );
    check!(
        text(br"{\rtf1\ansi\ansicpg932\uc1 \u26085?\u26412?}") == Some(String::from("日本")),
        "Unicode is independent of unsupported legacy byte page"
    );
    check!(text(br"{\rtf1{\fonttbl{\f0\fcharset161 Arial;}{\f1\fcharset204 Arial;}}\f0\'e1\'e2 \f1\'df}") == Some(String::from("αβ Я")), "Greek and Cyrillic font code pages");
    check!(
        text(br"{\rtf1\ansicpg1250\'e8\'e1\'f8}") == Some(String::from("čář")),
        "Central European document code page"
    );
    check!(
        text(br"{\rtf1{\fonttbl{\f0\fcharset161\cpg1252 Arial;}}\f0\'e4}")
            == Some(String::from("ä")),
        "font code page overrides charset mapping"
    );
    check!(
        text(br"{\rtf1 A\~B\_C\-D\emdash\endash\bullet\lquote\rquote\ldblquote\rdblquote}")
            == Some(String::from("A\u{a0}B\u{2011}C\u{ad}D—–•‘’“”")),
        "semantic symbols"
    );
    check!(text(br"{\rtf1 {\fonttbl{\f0\fnil\fcharset0 Arial;}}{\info{\title Secret}}{\*\generator Word;}Visible{\*\unknown Not visible}}") == Some(String::from("Visible")), "metadata and ignorable destination text never leak");
    check!(text(br#"{\rtf1 Before {\field{\*\fldinst HYPERLINK "https://example.invalid"}{\fldrslt Visible}} after}"#) == Some(String::from("Before Visible after")), "only cached field result enters external fragment");
    check!(
        text(br"{\rtf1 A{\*\unknown\bin4 {}\\}B}") == Some(String::from("AB")),
        "binary payload in ignored metadata is counted exactly"
    );
    let scoped = decode(br"{\rtf1\b A{\b0\i B}C\plain D}").unwrap();
    let formats: Vec<_> = scoped.paragraphs[0]
        .content
        .iter()
        .map(Inline::format)
        .collect();
    check!(
        formats.len() == 4
            && formats[0].bold == Some(true)
            && formats[1].bold == Some(false)
            && formats[1].italic == Some(true)
            && formats[2].bold == Some(true)
            && formats[2].italic == Some(false)
            && formats[3].bold == Some(false),
        "nested formatting and plain reset"
    );
    let color = decode(br"{\rtf1{\colortbl;\red18\green52\blue86;}\cf1 X\cf0 Y}").unwrap();
    check!(
        color.paragraphs[0].content[0].format().color == Some(TextColor::Rgb(0x123456))
            && color.paragraphs[0].content[1].format().color == Some(TextColor::Auto),
        "color table and automatic color"
    );
    let paragraph =
        decode(br"{\rtf1\pard\qr\sb80\sa100\li720\ri240\fi-360\sl480\slmult1\pagebb Text\par}")
            .unwrap();
    check!(
        paragraph.paragraphs[0].format
            == ParagraphFormat {
        explicit: 0,
                alignment: Alignment::Right,
                before: 80,
                after: 100,
                left: 720,
                right: 240,
                first_line: -360,
                line: 480,
                page_break_before: true,
                tabs: Default::default(),
            },
        "Word paragraph control units"
    );
    let late = decode(br"{\rtf1 Text\qc\sa120\sl360\slmult1\par}").unwrap();
    check!(
        late.paragraphs[0].format.alignment == Alignment::Center
            && late.paragraphs[0].format.after == 120
            && late.paragraphs[0].format.line == 360,
        "properties after text still apply to paragraph"
    );
    for (input, label) in [
        (&br"{\rtf1 \u-10188?}"[..], "unpaired high surrogate"),
        (&br"{\rtf1 \u-8930?}"[..], "unpaired low surrogate"),
        (
            &br"{\rtf1 \u-10188?X\u-8930?}"[..],
            "surrogate interrupted by text",
        ),
        (
            &br"{\rtf1 \u-10188?\b\u-8930?}"[..],
            "surrogate with inconsistent formatting",
        ),
        (&br"{\rtf1\uc17 X}"[..], "excessive unicode fallback"),
        (&br"{\rtf1\u65536?}"[..], "out of range unicode"),
        (&br"{\rtf1\u0?}"[..], "nul unicode"),
        (&br"{\rtf1\u65535?}"[..], "non XML Unicode"),
        (&br"{\rtf1\'8z}"[..], "invalid hex escape"),
        (&br"{\rtf1\'81}"[..], "undefined Windows 1252 byte"),
        (
            &br"{\rtf1\ansi\ansicpg932\'82\'a0}"[..],
            "unsupported multibyte page",
        ),
        (
            &br"{\rtf1{\fonttbl{\f0\fcharset2 Symbol;}}\f0 a}"[..],
            "symbol font bytes cannot be misread as Latin",
        ),
        (&br"{\rtf1\cf2 X}"[..], "undefined color"),
        (&br"{\rtf1{\colortbl;\red256;}X}"[..], "out of range RGB"),
        (
            &br"{\rtf1{\colortbl;\red2}X}"[..],
            "unterminated color entry",
        ),
        (&br"{\rtf1\fs7 X}"[..], "unsupported text size"),
        (&br"{\rtf1\sa3000 X}"[..], "unsupported paragraph spacing"),
        (&br"{\rtf1\li-10 X}"[..], "negative left indent"),
        (&br"{\rtf1\li9000 X}"[..], "indent outside page"),
        (&br"{\rtf1\sl-240 X}"[..], "absolute line spacing"),
        (&br"{\rtf1 X\sl-240}"[..], "absolute spacing after text"),
        (&br"{\rtf1\sl480 X}"[..], "absolute positive spacing"),
        (&br"{\rtf1\s2 X}"[..], "unresolved named style"),
        (
            &br"{\rtf1\highlight2 X}"[..],
            "unsupported visible typography",
        ),
        (
            &br"{\rtf1\intbl X\cell\row}"[..],
            "tables require richer document model",
        ),
        (
            &br"{\rtf1 A{\pict\pngblip 0000}B}"[..],
            "visible image cannot vanish",
        ),
        (
            &br"{\rtf1 A{\*\shp hidden}B}"[..],
            "starred drawing cannot vanish",
        ),
        (
            &br"{\rtf1 A{\object data}B}"[..],
            "embedded object cannot vanish",
        ),
        (
            &br"{\rtf1 A{\footnote note}B}"[..],
            "footnote body cannot vanish",
        ),
        (
            &br"{\rtf1{\listtext 1.}Text}"[..],
            "list label cannot vanish",
        ),
        (
            &br"{\rtf1{\field{\*\fldinst DATE}}}"[..],
            "field without cached result cannot vanish",
        ),
        (
            &br"{\rtf1{\field{\fldrslt A}{\fldrslt B}}}"[..],
            "duplicate field result",
        ),
        (
            &br"{\rtf1\unknown Text}"[..],
            "unknown visible control is explicit rejection",
        ),
        (&br"{\rtf1\bin1 X}"[..], "binary visible content rejected"),
        (
            &br"{\rtf1{\*\unknown\bin100 X}}"[..],
            "truncated binary metadata",
        ),
        (&br"{\rtf1\fs99999999999999 X}"[..], "integer overflow"),
        (&br"{\rtf1\fs- X}"[..], "missing numeric parameter"),
        (&br"{\rtf1 text"[..], "unclosed group"),
        (&br"{\rtf1 text}}"[..], "extra close group"),
        (
            &br"{\rtf1 A}{\rtf1 B}"[..],
            "second document is not appended",
        ),
        (&br"{\rtf1 text}tail"[..], "trailing non whitespace"),
        (&br"{\ansi text}"[..], "missing RTF header"),
        (&br"{\rtf2 text}"[..], "unsupported version"),
    ] {
        check!(decode(input).is_err(), label);
    }
    let depth = format!("{{\\rtf1 {}X{}}}", "{".repeat(64), "}".repeat(64));
    check!(decode(depth.as_bytes()).is_err(), "bounded group nesting");
    let paragraphs = format!("{{\\rtf1 {} }}", "X\\par ".repeat(2049));
    check!(
        decode(paragraphs.as_bytes()).is_err(),
        "bounded paragraph count"
    );
    check!(
        decode(&vec![b' '; (16 << 20) + 1]).is_err(),
        "input limit before parsing"
    );
    let large = format!("{{\\rtf1 {} }}", "x".repeat((1 << 20) + 16));
    check!(
        decode(large.as_bytes()).is_ok(),
        "token budget admits model-sized plain text"
    );
    let words = format!("{{\\rtf1 {} }}", "\\b0 X\\b1 Y".repeat(4097));
    check!(decode(words.as_bytes()).is_err(), "bounded inline count");
    let mut bad = d.clone();
    bad.paragraphs[0].content = vec![Inline::Text(String::from("\0"))];
    check!(
        encode(&bad).is_err(),
        "unvalidated source document rejected"
    );
    let fields = crate::checks::sample();
    let flat = decode(&encode(&fields).unwrap()).unwrap();
    check!(
        flat.paragraphs
            .iter()
            .map(|p| flat.paragraph_text(p))
            .collect::<Vec<_>>()
            == fields
                .paragraphs
                .iter()
                .map(|p| fields.paragraph_text(p))
                .collect::<Vec<_>>()
            && flat.sources.is_empty()
            && flat
                .paragraphs
                .iter()
                .flat_map(|p| &p.content)
                .all(|n| n.text().is_some()),
        "scientific fields preserve display text externally"
    );
    let html = String::from_utf8(doc_io::clipboard_html_encode(&d).unwrap()).unwrap();
    check!(
        html.contains("text-indent:-18.00pt")
            && html.contains("line-height:1.500000")
            && html.contains("font-size:11.50pt")
            && html.contains("text-decoration:underline line-through")
            && html.contains("color:#8B2FB0"),
        "HTML explicit paragraph and character CSS"
    );
    check!(
        html.contains("Grüße αβ 𝄞 😀") && html.contains("<br>Soft break"),
        "HTML UTF8 and soft breaks"
    );
    let mut markup = Document::default();
    markup.paragraphs[0].content = vec![Inline::Text(String::from("<script>&\"'"))];
    let html = String::from_utf8(doc_io::clipboard_html_encode(&markup).unwrap()).unwrap();
    check!(
        html.contains("&lt;script&gt;&amp;&quot;&#39;") && !html.contains("<script>"),
        "HTML payload always escapes source text"
    );
    markup.paragraphs[0].content = vec![Inline::Text(String::from("A\r\nB\rC\nD"))];
    check!(
        text(&encode(&markup).unwrap()) == Some(String::from("A\nB\nC\nD")),
        "CRLF and lone CR normalize once"
    );
    markup.paragraphs[0].content.clear();
    markup.paragraphs[0].style = ParagraphStyle::Title;
    let blank_title = decode(&encode(&markup).unwrap()).unwrap();
    check!(
        blank_title.paragraphs[0].content[0]
            .format()
            .size_half_points
            == Some(48),
        "empty title paragraph retains font metrics"
    );
    // Captured from Windows RichEdit 10.0.26100, rather than emitted by this codec.
    let richedit = br"{\rtf1\ansi\ansicpg1252\deff0\nouicompat\deflang1031{\fonttbl{\f0\fnil\fcharset0 Arial;}{\f1\fnil Segoe UI;}{\f2\fnil\fcharset161 Segoe UI;}{\f3\fnil\fcharset0 Segoe UI;}}{\colortbl ;\red18\green52\blue86;}{\*\generator Riched20 10.0.26100}\viewkind4\uc1\pard\cf1\ul\b\i\strike\fs23 Bold\cf0\ulnone\b0\i0\strike0\f1\fs18  \f2\lang1032\'e1 \f3\lang1033\'e9\par\pard\qc Centered\f1\lang1031\par}";
    let imported = decode(richedit).unwrap();
    check!(
        imported.plain_text() == "Bold α é\nCentered"
            && imported.paragraphs[0].content[0]
                .format()
                .resolve(ParagraphStyle::Normal)
                == doc_core::ResolvedFormat {
                    font_family: doc_core::FontFamily::new("Arial").unwrap(),
                    bold: true,
                    italic: true,
                    underline: true,
                    strike: true,
                    size_half_points: 23,
                    color: 0xff123456,
                    highlight: doc_core::Highlight::None,
                    positioning: doc_core::TextPositioning::Baseline,
                }
            && imported.paragraphs[1].format.alignment == Alignment::Center,
        "independent Windows RichEdit fixture"
    );
    (passed, failed)
}

pub(crate) fn probe(args: &base::Args) -> Option<i32> {
    let path = args.value("--check-rtf")?;
    let result = (|| -> Result<Document, doc_io::Error> {
        let bytes = doc_io::read_file(path, 16 << 20)?;
        let document = decode(&bytes)?;
        if let Some(path) = args.value("--output") {
            if !plat::write_file(path, &encode(&document)?) {
                return Err(doc_io::Error::Io);
            }
        }
        if let Some(path) = args.value("--docx-output") {
            doc_io::save_docx(path, &document)?;
        }
        Ok(document)
    })();
    let rejected = args.flag("--expect-reject");
    let success = result.is_err() == rejected;
    match result {
        Ok(d) => plat::write_stdout(
            format!(
                "{} RTF: {} paragraphs, {} bytes of displayed text\n",
                if success { "PASS" } else { "FAIL" },
                d.paragraphs.len(),
                d.plain_text().len()
            )
            .as_bytes(),
        ),
        Err(error) => plat::write_stdout(
            format!("{} RTF: {error:?}\n", if success { "PASS" } else { "FAIL" }).as_bytes(),
        ),
    }
    Some(i32::from(!success))
}
