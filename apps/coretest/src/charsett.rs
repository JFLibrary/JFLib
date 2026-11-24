// charsett -- checks of formats::encoding's single-byte character sets and WHATWG labels: every set
// (ASCII below 0x80, unassigned bytes as U+FFFD, encode undoes decode, code pages, names), vectors
// whose expected values come from Python's codecs and the WHATWG index (not from these tables), the
// labels, decode_label (byte-order marks over labels, malformed input, limits)

use alloc::string::String;
use alloc::vec::Vec;

use formats::encoding::{self, Charset, Error, Label, MultiByte};

use crate::T;

fn s(units: &[u32]) -> String {
    units.iter().map(|&u| char::from_u32(u).unwrap_or('?')).collect()
}

pub fn run(t: &mut T) {
    // every set: the shape of its table
    let mut bad = Vec::new();
    for &c in Charset::ALL {
        let all: Vec<u8> = (0..=255u8).collect();
        let text = c.decode(&all);
        let ascii_ok = text.chars().take(128).enumerate().all(|(i, ch)| ch as u32 == i as u32);
        let count_ok = text.chars().count() == 256;
        let mut round_ok = true;
        for b in 0x80..=0xFFu8 {
            match c.decode_char(b) {
                Some(ch) => round_ok &= c.encode_char(ch).and_then(|x| c.decode_char(x)) == Some(ch),
                None => round_ok &= text.chars().nth(b as usize) == Some(char::REPLACEMENT_CHARACTER),
            }
        }
        let cp_ok = c.code_page().is_none_or(|n| Charset::from_code_page(n) == Some(c));
        // a WHATWG set is named by its own name; the others by no web label
        let label_ok = if c.whatwg() { Charset::from_label(c.name()) == Some(c) } else { Charset::from_label(c.name()) != Some(c) };
        if !(ascii_ok && count_ok && round_ok && cp_ok && label_ok) {
            bad.push(c.name());
        }
    }
    t.eq("every set: ASCII, 256 characters, encode undoes decode, code page, label", bad, Vec::<&str>::new());
    t.eq("sets", Charset::ALL.len(), 45);

    // vectors (expected values from Python's codecs and the WHATWG index)
    let cases: &[(Charset, &[u8], &[u32])] = &[
        (Charset::Windows1252, &[0x80, 0x81, 0x9F, 0xE9], &[0x20AC, 0x0081, 0x0178, 0x00E9]),
        (Charset::Koi8U, &[0xAE, 0xBE, 0xA4], &[0x045E, 0x040E, 0x0454]),
        (Charset::Windows1255, &[0xCA], &[0x05BA]),
        (Charset::Windows1253, &[0xAA, 0xC1], &[0xFFFD, 0x0391]),
        (Charset::Koi8R, &[0xC1, 0xD7], &[0x0430, 0x0432]),
        (Charset::Windows874, &[0xA1, 0xDB], &[0x0E01, 0xFFFD]),
        (Charset::Ibm866, &[0x80, 0xF0], &[0x0410, 0x0401]),
        (Charset::Macintosh, &[0x80, 0xDB], &[0x00C4, 0x20AC]),
        (Charset::Iso8859_15, &[0xA4], &[0x20AC]),
        (Charset::Ibm437, &[0x80, 0xE1, 0xFE], &[0x00C7, 0x00DF, 0x25A0]),
        (Charset::Ibm850, &[0xD5, 0x9C], &[0x0131, 0x00A3]),
        (Charset::Ibm862, &[0x80, 0x9A], &[0x05D0, 0x05EA]),
        (Charset::Latin1, &[0x80, 0xFF], &[0x0080, 0x00FF]),
        (Charset::XUserDefined, &[0x41, 0x80, 0xFF], &[0x0041, 0xF780, 0xF7FF]),
    ];
    for (c, bytes, want) in cases {
        t.eq(&alloc::format!("{} decodes", c.name()), c.decode(bytes), s(want));
    }
    t.eq("ASCII stays ASCII", Charset::Ibm437.decode(b"plain text"), String::from("plain text"));

    // encode
    t.eq("windows-1252 encodes the euro sign", Charset::Windows1252.encode("€uro"), Ok(alloc::vec![0x80, b'u', b'r', b'o']));
    t.eq("IBM437 has no euro sign", Charset::Ibm437.encode("a€"), Err(Error::Unmappable('€')));
    t.eq("ISO-8859-1 exactly", (Charset::Latin1.encode("ÿ"), Charset::Latin1.encode("Ā")), (Ok(alloc::vec![0xFF]), Err(Error::Unmappable('Ā'))));
    t.eq("KOI8-R round trip", Charset::Koi8R.encode("мама").map(|b| Charset::Koi8R.decode(&b)), Ok(String::from("мама")));

    // code pages
    t.eq("code pages", [1252u16, 437, 850, 866, 874, 10000, 20866, 28591, 28592, 28599, 38598, 932].map(Charset::from_code_page), [
        Some(Charset::Windows1252), Some(Charset::Ibm437), Some(Charset::Ibm850), Some(Charset::Ibm866), Some(Charset::Windows874),
        Some(Charset::Macintosh), Some(Charset::Koi8R), Some(Charset::Latin1), Some(Charset::Iso8859_2), Some(Charset::Windows1254),
        Some(Charset::Iso8859_8I), None,
    ]);

    // the WHATWG labels
    let labels: &[(&str, Option<Label>)] = &[
        ("latin1", Some(Label::Single(Charset::Windows1252))),
        ("ISO-8859-1", Some(Label::Single(Charset::Windows1252))),
        ("us-ascii", Some(Label::Single(Charset::Windows1252))),
        (" \tUTF-8\r\n", Some(Label::Utf8)),
        ("utf-16", Some(Label::Utf16Le)),
        ("UTF-16BE", Some(Label::Utf16Be)),
        ("tis-620", Some(Label::Single(Charset::Windows874))),
        ("iso-8859-9", Some(Label::Single(Charset::Windows1254))),
        ("x-mac-ukrainian", Some(Label::Single(Charset::XMacCyrillic))),
        ("koi8-ru", Some(Label::Single(Charset::Koi8U))),
        ("logical", Some(Label::Single(Charset::Iso8859_8I))),
        ("Shift_JIS", Some(Label::Multi(MultiByte::ShiftJis))),
        ("gb2312", Some(Label::Multi(MultiByte::Gbk))),
        ("iso-2022-kr", Some(Label::Replacement)),
        ("cp437", None),
        ("", None),
        ("latin1x", None),
        ("latin 1", None),
        ("\u{2113}atin1", None),
    ];
    for (l, want) in labels {
        t.eq(&alloc::format!("label {:?}", l), encoding::label(l), *want);
    }
    let long = "x".repeat(10_000);
    t.eq("a long label", encoding::label(&long), None);

    // decode_label: the byte-order mark wins, malformed input is U+FFFD, the limits
    let d = |l: &str, b: &[u8]| encoding::decode_label(l, b, 1 << 20);
    t.eq("BOM UTF-8 over windows-1252", d("windows-1252", b"\xEF\xBB\xBFa\xC3\xA9"), Ok(String::from("aé")));
    t.eq("BOM UTF-16LE over latin1", d("latin1", b"\xFF\xFEa\0"), Ok(String::from("a")));
    t.eq("BOM with an unknown label", d("nope", b"\xFE\xFF\0a"), Ok(String::from("a")));
    t.eq("UTF-16BE with an odd byte", d("utf-16be", b"\0a\xD8"), Ok(String::from("a\u{FFFD}")));
    t.eq("UTF-16LE lone surrogate", d("utf-16le", b"\0\xD8b\0"), Ok(String::from("\u{FFFD}b")));
    t.eq("UTF-8 cut", d("utf-8", b"a\xC3"), Ok(String::from("a\u{FFFD}")));
    t.eq("koi8-r", d("koi8-r", b"\xCD\xC1\xCD\xC1"), Ok(String::from("мама")));
    t.eq("replacement", (d("iso-2022-kr", b"abc"), d("iso-2022-kr", b"")), (Ok(String::from("\u{FFFD}")), Ok(String::new())));
    t.eq("Shift_JIS by its label", d("shift_jis", b"a\x82\xA0"), Ok(String::from("aあ")));
    t.eq("an unknown label", d("nope", b"a"), Err(Error::Unsupported));
    t.eq("input over the limit", encoding::decode_label("koi8-r", &[0xC1; 10], 5), Err(Error::Limit));
    t.eq("output over the limit", encoding::decode_label("windows-1252", &[0x80; 4], 10), Err(Error::Limit));
    t.eq("output at the limit", encoding::decode_label("windows-1252", &[0x80; 4], 12).map(|x| x.len()), Ok(12));

    // the old path is the same type
    let old: base::encoding::Charset = Charset::Ibm437;
    t.eq("base::encoding::Charset", old.name(), "IBM437");

    multibyte(t);
}

// the multi-byte encodings: vectors from Python's codecs (cp932, euc_jp, iso2022_jp, gbk, gb18030, big5, cp949)
// and from the standard where it differs from them; the exhaustive comparison with encoding_rs is
// tools/encoding_check.py
fn multibyte(t: &mut T) {
    use MultiByte::*;
    for &m in MultiByte::ALL {
        t.eq(&alloc::format!("{} by its name and code page", m.name()), (MultiByte::from_label(m.name()), MultiByte::from_code_page(m.code_page())), (Some(m), Some(m)));
    }
    t.eq("multi-byte encodings", MultiByte::ALL.len(), 7);
    let enc = |m: MultiByte, s: &str| m.encode(s);
    let v = |b: &[u8]| Ok(b.to_vec());
    // Japanese
    t.eq("Shift_JIS 日本語", enc(ShiftJis, "日本語"), v(&[0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA]));
    t.eq("Shift_JIS half-width katakana, yen, overline, minus", enc(ShiftJis, "ｱ¥‾−"), v(&[0xB1, 0x5C, 0x7E, 0x81, 0x7C]));
    t.eq("Shift_JIS NEC row 13", enc(ShiftJis, "①"), v(&[0x87, 0x40]));
    t.eq("Shift_JIS writes the IBM extensions, not NEC's copies (the standard; cp932 writes EE EF)", enc(ShiftJis, "ⅰ"), v(&[0xFA, 0x40]));
    t.eq("Shift_JIS reads both", (ShiftJis.decode(&[0xFA, 0x40]), ShiftJis.decode(&[0xEE, 0xEF])), (String::from("ⅰ"), String::from("ⅰ")));
    t.eq("Shift_JIS user-defined rows as the Private Use Area", ShiftJis.decode(&[0xF0, 0x40, 0xF9, 0xFC]), s(&[0xE000, 0xE757]));
    t.eq("Shift_JIS 0x80 and a broken lead before ASCII", ShiftJis.decode(&[0x80, 0x82, 0x20, 0xBF, 0xA0, 0xFD]), s(&[0x80, 0xFFFD, 0x20, 0xFF7F, 0xFFFD, 0xFFFD]));
    t.eq("Shift_JIS has no Hangul", enc(ShiftJis, "a한"), Err(Error::Unmappable('한')));
    t.eq("EUC-JP 日本語", enc(EucJp, "日本語"), v(&[0xC6, 0xFC, 0xCB, 0xDC, 0xB8, 0xEC]));
    t.eq("EUC-JP half-width katakana", (enc(EucJp, "ｱ"), EucJp.decode(&[0x8E, 0xB1])), (v(&[0x8E, 0xB1]), String::from("ｱ")));
    t.eq("EUC-JP reads JIS X 0212", EucJp.decode(&[0x8F, 0xB0, 0xA1]), String::from("丂"));
    t.eq("EUC-JP writes JIS X 0208 only", enc(EucJp, "丂"), Err(Error::Unmappable('丂')));
    t.eq("ISO-2022-JP あ", enc(Iso2022Jp, "Aあ"), v(b"A\x1B$B$\"\x1B(B"));
    t.eq("ISO-2022-JP yen through JIS X 0201 Roman", enc(Iso2022Jp, "¥"), v(b"\x1B(J\\\x1B(B"));
    t.eq("ISO-2022-JP half-width katakana as full-width", enc(Iso2022Jp, "ｱ"), v(b"\x1B$B%\"\x1B(B"));
    t.eq("ISO-2022-JP cannot write ESC", enc(Iso2022Jp, "a\x1Bb"), Err(Error::Unmappable('\x1B')));
    t.eq("ISO-2022-JP reads its states", Iso2022Jp.decode(b"a\x1B(J\\\x1B(I1\x1B$B$\"\x1B(Bb"), String::from("a¥ｱあb"));
    t.eq("ISO-2022-JP two escapes in a row are an error", Iso2022Jp.decode(b"\x1B(J\x1B(Ba"), String::from("\u{FFFD}a"));
    t.eq("ISO-2022-JP a cut escape", (Iso2022Jp.decode(b"a\x1B"), Iso2022Jp.decode(b"a\x1B$")), (String::from("a\u{FFFD}"), String::from("a\u{FFFD}$")));
    // Chinese
    t.eq("GBK 中文", enc(Gbk, "中文"), v(&[0xD6, 0xD0, 0xCE, 0xC4]));
    t.eq("GBK writes the euro sign as 0x80, gb18030 as A2 E3", (enc(Gbk, "€"), enc(Gb18030, "€")), (v(&[0x80]), v(&[0xA2, 0xE3])));
    t.eq("gb18030 beyond the BMP", (enc(Gb18030, "😀"), Gb18030.decode(&[0x94, 0x39, 0xFC, 0x36])), (v(&[0x94, 0x39, 0xFC, 0x36]), String::from("😀")));
    t.eq("GBK has no four-byte sequences", enc(Gbk, "😀"), Err(Error::Unmappable('😀')));
    t.eq("GBK reads them", Gbk.decode(&[0x94, 0x39, 0xFC, 0x36]), String::from("😀"));
    t.eq("gb18030-2022: U+FE10 is A6 D9 (gb18030-2005: 84 31 82 36), the old U+E78D too", (enc(Gb18030, "\u{FE10}"), enc(Gb18030, "\u{E78D}"), Gb18030.decode(&[0xA6, 0xD9])),
        (v(&[0xA6, 0xD9]), v(&[0xA6, 0xD9]), String::from("\u{FE10}")));
    t.eq("gb18030 refuses U+E5E5", enc(Gb18030, "\u{E5E5}"), Err(Error::Unmappable('\u{E5E5}')));
    t.eq("gb18030 the last BMP pointer and the gap after it", (Gb18030.decode(&[0x84, 0x31, 0xA4, 0x39]), Gb18030.decode(&[0x84, 0x31, 0xA5, 0x30])), (String::from("\u{FFFF}"), String::from("\u{FFFD}")));
    t.eq("gb18030 a cut four-byte sequence: its bytes are read again", (Gb18030.decode(&[0x81, 0x30, 0x81, 0x20]), Gb18030.decode(&[0x81, 0x30, 0x81, 0x41])), (String::from("\u{FFFD}0\u{FFFD} "), String::from("\u{FFFD}0丄")));
    t.eq("Big5 中文", enc(Big5, "中文"), v(&[0xA4, 0xA4, 0xA4, 0xE5]));
    t.eq("Big5 two characters from one pair", Big5.decode(&[0x88, 0x62]), String::from("Ê\u{304}"));
    t.eq("Big5 HKSCS-2008, beyond the BMP", Big5.decode(&[0x87, 0x7A, 0x87, 0x7B]), s(&[0x3875, 0x21D53]));
    t.eq("Big5 writes the last of a doubled character (the standard; cp950 writes A2 A4)", enc(Big5, "═"), v(&[0xF9, 0xF9]));
    t.eq("Big5 does not write HKSCS's rows below A1", enc(Big5, "\u{3875}"), Err(Error::Unmappable('\u{3875}')));
    // Korean
    t.eq("EUC-KR 한국어", enc(EucKr, "한국어"), v(&[0xC7, 0xD1, 0xB1, 0xB9, 0xBE, 0xEE]));
    t.eq("EUC-KR the Unified Hangul Code", (enc(EucKr, "똠"), EucKr.decode(&[0x8C, 0x63])), (v(&[0x8C, 0x63]), String::from("똠")));
    // character references
    t.eq("Shift_JIS with character references", ShiftJis.encode_html("a😀"), b"a&#128512;".to_vec());
    t.eq("ISO-2022-JP leaves JIS X 0208 before a reference", Iso2022Jp.encode_html("あ😀"), b"\x1B$B$\"\x1B(B&#128512;".to_vec());
    t.eq("ISO-2022-JP ESC as U+FFFD", Iso2022Jp.encode_html("\x1B"), b"&#65533;".to_vec());
    // round trips of everything each one writes, in pieces
    let sample = "テキスト 文字化け ｶﾀｶﾅ 中文字符 繁體中文 한국어 텍스트 ①ⅰ¥€";
    for &m in MultiByte::ALL {
        // (what comes back as itself: Shift_JIS writes ¥ as 0x5C, which reads as a backslash)
        let text: String = sample.chars().filter(|c| m.encode(c.encode_utf8(&mut [0; 4])).is_ok_and(|b| m.decode(&b).chars().eq([*c]))).collect();
        let bytes = m.encode(&text).unwrap_or_default();
        let mut d = encoding::Decoder::new(Label::Multi(m));
        let mut back = String::new();
        for b in &bytes {
            d.decode(core::slice::from_ref(b), false, &mut back);
        }
        d.decode(&[], true, &mut back);
        t.eq(&alloc::format!("{} round trip byte by byte", m.name()), (back == text, d.malformed(), text.chars().count() > 8), (true, false, true));
    }
    // the decoder in pieces, its malformed flag, the byte-order mark rule
    let pieces = |l: Label, parts: &[&[u8]]| {
        let mut d = encoding::Decoder::new(l);
        let mut out = String::new();
        for (i, p) in parts.iter().enumerate() {
            d.decode(p, i + 1 == parts.len(), &mut out);
        }
        (out, d.malformed())
    };
    t.eq("pieces: gb18030", pieces(Label::Multi(Gb18030), &[&[0x94], &[0x39], &[], &[0xFC, 0x36]]), (String::from("😀"), false));
    t.eq("pieces: ISO-2022-JP", pieces(Label::Multi(Iso2022Jp), &[b"\x1B", b"$", b"B$", b"\"\x1B(", b"B"]), (String::from("あ"), false));
    t.eq("pieces: UTF-8 and a cut end", pieces(Label::Utf8, &[&[0xE3, 0x81], &[0x82, 0xE3]]), (String::from("あ\u{FFFD}"), true));
    t.eq("pieces: UTF-16LE with a surrogate pair", pieces(Label::Utf16Le, &[&[0x3D], &[0xD8, 0x00], &[0xDE]]), (String::from("😀"), false));
    t.eq("pieces: a lead byte at the end", pieces(Label::Multi(Big5), &[b"a", &[0xA4]]), (String::from("a\u{FFFD}"), true));
    let sniff = |l: Label, parts: &[&[u8]]| {
        let mut d = encoding::Decoder::sniffing(l);
        let mut out = String::new();
        for (i, p) in parts.iter().enumerate() {
            d.decode(p, i + 1 == parts.len(), &mut out);
        }
        (out, d.label())
    };
    t.eq("sniffing: a UTF-8 mark in pieces wins over Shift_JIS", sniff(Label::Multi(ShiftJis), &[&[0xEF, 0xBB], &[0xBF, 0xE3, 0x81, 0x82]]), (String::from("あ"), Label::Utf8));
    t.eq("sniffing: no mark", sniff(Label::Multi(ShiftJis), &[&[0x82], &[0xA0]]), (String::from("あ"), Label::Multi(ShiftJis)));
    t.eq("sniffing: the beginning of a mark at the end", sniff(Label::Multi(ShiftJis), &[&[0xEF, 0xBB]]), (String::from("\u{FFFD}"), Label::Multi(ShiftJis)));
    t.eq("sniffing: UTF-16BE", sniff(Label::Single(Charset::Windows1252), &[&[0xFE], &[0xFF, 0, 0x41]]), (String::from("A"), Label::Utf16Be));
    // labels and code pages
    t.eq("labels", ["sjis", "windows-31j", "x-euc-jp", "csiso2022jp", "gb2312", "GB18030", "big5-hkscs", "ks_c_5601-1987"].map(MultiByte::from_label),
        [Some(ShiftJis), Some(ShiftJis), Some(EucJp), Some(Iso2022Jp), Some(Gbk), Some(Gb18030), Some(Big5), Some(EucKr)]);
    t.eq("from_code_page", [65001u16, 1200, 1252, 932, 51949, 50221, 50222, 1361].map(encoding::from_code_page),
        [Some(Label::Utf8), Some(Label::Utf16Le), Some(Label::Single(Charset::Windows1252)), Some(Label::Multi(ShiftJis)), Some(Label::Multi(EucKr)), Some(Label::Multi(Iso2022Jp)), None, None]);
    t.eq("decode_label: the replacement encoding keeps no room for its input", encoding::decode_label("iso-2022-kr", &[0x41; 4096], 1 << 20).map(|s| (s.len(), s.capacity() < 64)), Ok((3, true)));
    t.eq("Label::name", [Label::Utf16Be, Label::Multi(Gb18030), Label::Single(Charset::Koi8U), Label::Replacement].map(Label::name), ["UTF-16BE", "gb18030", "KOI8-U", "replacement"]);
    t.eq("decode_label GBK", encoding::decode_label("gb2312", &[0xD6, 0xD0, 0xCE, 0xC4], 100), Ok(String::from("中文")));
    // hostile input: no panic, bounded output
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut worst = 0usize;
    for round in 0..400 {
        let mut b = Vec::new();
        for _ in 0..(round % 50) * 7 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            b.push(seed as u8);
        }
        for &m in MultiByte::ALL {
            let text = m.decode(&b);
            worst = worst.max(text.len().saturating_sub(3 * b.len()));
            let _ = m.encode(&text);
        }
    }
    t.eq("random bytes: at most three output bytes per input byte", worst, 0);
}

/// coretest --encoding OP LABEL IN OUT [SEED]: the decoders and encoders on files, for tools/encoding_check.py
///   d    decode IN at once (no byte-order mark rule) -> OUT as UTF-8
///   dc   the same in pieces of 0 to 7 bytes (sizes from SEED)
///   ds   in pieces as dc, with the byte-order mark rule (Decoder::sniffing)
///   e    encode each character of IN (UTF-8) alone -> one line each: hex bytes or "-"
///   et   encode all of IN -> OUT; an unmappable character: its code point in hex on stdout, exit 3
///   eh   encode all of IN with character references -> OUT
pub fn cli(op: &str, rest: &[String]) -> i32 {
    let (Some(label), Some(input), Some(output)) = (rest.first(), rest.get(1), rest.get(2)) else { return 2 };
    let Some(l) = encoding::label(label) else { return 2 };
    let Some(data) = plat::read_file(input) else { return 2 };
    let out: Vec<u8> = match op {
        "d" | "dc" | "ds" => {
            let mut d = if op == "ds" { encoding::Decoder::sniffing(l) } else { encoding::Decoder::new(l) };
            let mut s = String::new();
            if op == "d" {
                d.decode(&data, true, &mut s);
            } else {
                let mut seed: u64 = rest.get(3).and_then(|x| x.parse().ok()).unwrap_or(1);
                let mut at = 0;
                loop {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let end = (at + ((seed >> 33) % 8) as usize).min(data.len());
                    let last = end == data.len();
                    d.decode(&data[at..end], last, &mut s);
                    at = end;
                    if last {
                        break;
                    }
                }
            }
            s.into_bytes()
        }
        "e" | "et" | "eh" => {
            let Label::Multi(m) = l else { return 2 };
            let Ok(text) = core::str::from_utf8(&data) else { return 2 };
            match op {
                "e" => {
                    let mut o = String::with_capacity(text.len() * 3);
                    let mut buf = [0u8; 4];
                    for c in text.chars() {
                        match m.encode(c.encode_utf8(&mut buf)) {
                            Ok(b) => b.iter().for_each(|x| o.push_str(&alloc::format!("{x:02x}"))),
                            Err(_) => o.push('-'),
                        }
                        o.push('\n');
                    }
                    o.into_bytes()
                }
                "et" => match m.encode(text) {
                    Ok(b) => b,
                    Err(Error::Unmappable(c)) => {
                        plat::write_stdout(alloc::format!("{:x}\n", c as u32).as_bytes());
                        return 3;
                    }
                    Err(_) => return 1,
                },
                _ => m.encode_html(text),
            }
        }
        _ => return 2,
    };
    if plat::write_file(output, &out) { 0 } else { 1 }
}
