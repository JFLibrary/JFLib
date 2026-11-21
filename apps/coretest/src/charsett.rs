// charsett -- checks of formats::encoding's single-byte character sets and WHATWG labels: every set
// (ASCII below 0x80, unassigned bytes as U+FFFD, encode undoes decode, code pages, names), vectors
// whose expected values come from Python's codecs and the WHATWG index (not from these tables), the
// labels, decode_label (byte-order marks over labels, malformed input, limits)

use alloc::string::String;
use alloc::vec::Vec;

use formats::encoding::{self, Charset, Error, Label};

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
        ("Shift_JIS", Some(Label::Other("Shift_JIS"))),
        ("gb2312", Some(Label::Other("GBK"))),
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
    t.eq("an encoding not decoded here", d("shift_jis", b"a"), Err(Error::Unsupported));
    t.eq("an unknown label", d("nope", b"a"), Err(Error::Unsupported));
    t.eq("input over the limit", encoding::decode_label("koi8-r", &[0xC1; 10], 5), Err(Error::Limit));
    t.eq("output over the limit", encoding::decode_label("windows-1252", &[0x80; 4], 10), Err(Error::Limit));
    t.eq("output at the limit", encoding::decode_label("windows-1252", &[0x80; 4], 12).map(|x| x.len()), Ok(12));

    // the old path is the same type
    let old: base::encoding::Charset = Charset::Ibm437;
    t.eq("base::encoding::Charset", old.name(), "IBM437");
}
