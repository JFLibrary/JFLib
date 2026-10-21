// RIS model/codec regression contracts. New native checks without a test runtime.
use alloc::{format, string::String, vec};
use base::encoding::{self, Encoding};
use doc_core::{Document, Edit, Editor, Person, Source, SourceKind};
use doc_io::{decode_ris, encode_ris};

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL RIS {}\n", $name).as_bytes());
            }
        };
    }
    let input = "TY  - JOUR\r\nID  - external-abc\r\nT1  - Unicode α 李\r\nA1  - Müller, Anna Marie\r\nAU  - García\r\nY1  - 2026///\r\nJO  - Methods\r\nVL  - 12\r\nIS  - 3\r\nSP  - 17\r\nEP  - 29\r\nDO  - 10.1234/methods\r\nSN  - 1234-5678\r\nST  - Unicode\r\nN1  - first\r\n      second\r\nY2  - 2024/02/29/\r\nLA  - de\r\nER  -\r\n";
    let decoded = decode_ris(input.as_bytes()).unwrap();
    let s = &decoded[0];
    check!(
        s.id == 1
            && s.kind == SourceKind::JournalArticle
            && s.title == "Unicode α 李"
            && s.year == "2026",
        "type, aliases, identity and Unicode"
    );
    check!(
        s.authors
            == vec![
                Person {
                    family: String::from("Müller"),
                    given: String::from("Anna Marie"),
                    middle: String::new()
                },
                Person {
                    family: String::from("García"),
                    ..Default::default()
                }
            ],
        "ordered AU/A1 and intact given names"
    );
    check!(
        s.pages == "17-29"
            && s.journal == "Methods"
            && s.url == "https://doi.org/10.1234/methods"
            && s.comments == "first\nsecond"
            && s.external_id.is_empty(),
        "metadata mapping without GUID conflation"
    );
    check!(
        s.year_accessed == "2024" && s.month_accessed == "02" && s.day_accessed == "29",
        "access date components"
    );
    for enc in [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be] {
        check!(
            decode_ris(&encoding::encode(input, enc)) == Ok(decoded.clone()),
            "Unicode BOM decoding"
        );
    }
    let mut all = decoded.clone();
    all.push(Source {
        id: 90,
        kind: SourceKind::Book,
        title: String::from("Buch"),
        year: String::from("2020"),
        publisher: String::from("Verlag"),
        city: String::from("Berlin"),
        edition: String::from("2"),
        authors: vec![Person {
            family: String::from("Smith"),
            given: String::from("Jane"),
            middle: String::new(),
        }],
        ..Default::default()
    });
    all.push(Source {
        id: 91,
        kind: SourceKind::Website,
        title: String::from("Seite"),
        website: String::from("Portal"),
        url: String::from("https://example.org"),
        comments: String::from("first\n\nZZ  - plain text\n      spaces\n"),
        ..Default::default()
    });
    let encoded = encode_ris(&all).unwrap();
    for (i, s) in all.iter_mut().enumerate() {
        s.id = i as u64 + 1;
    }
    check!(
        decode_ris(&encoded) == Ok(all.clone()),
        "complete supported model roundtrip and escaped multiline"
    );
    check!(
        encode_ris(&all) == Ok(encoded),
        "deterministic output independent of local IDs"
    );
    let equal_doi = input.replace(
        "DO  - 10.1234/methods",
        "DO  - 10.1234/methods\r\nUR  - https://doi.org/10.1234/methods",
    );
    check!(
        decode_ris(equal_doi.as_bytes()) == Ok(decoded.clone()),
        "DO and agreeing URL"
    );
    check!(
        matches!(decode_ris(format!("{input}{input}").as_bytes()), Err(doc_io::Error::Unsupported(ref message)) if message.contains("Datensatz 2") && message.contains("ID")),
        "duplicate external reference ID rejected"
    );
    check!(
        decode_ris(b"TY  - BOOK\nBT  - Book\nER  -\n").is_ok()
            && decode_ris(b"").unwrap().is_empty(),
        "BOOK title alias and empty list"
    );
    for (tag, line) in [
        ("ZZ", "ZZ  - unknown"),
        ("TI", "TI  - duplicate"),
        ("DO/UR", "UR  - https://example.org/other"),
        ("AU", "AU  - Smith, Jane, Jr."),
        ("AU", "AU  - Corporation,"),
        ("PY/Y1", "PY  - 2026/10/10"),
        ("Y2", "Y2  - 2023/02/29"),
        ("KW", "KW  - key"),
        ("AB", "AB  - abstract"),
    ] {
        let prefix = if tag == "DO/UR" {
            "DO  - 10.1234/one\n"
        } else {
            ""
        };
        let value = format!("TY  - JOUR\nTI  - Title\n{prefix}{line}\nER  -\n");
        check!(
            matches!(decode_ris(value.as_bytes()), Err(doc_io::Error::Unsupported(ref message)) if message.contains("Datensatz 1") && message.contains(tag)),
            "specific record/tag rejection"
        );
    }
    for bad in [
        "TI  - no record\n",
        "TY  - BOOK\nTI  - incomplete\n",
        "TY  - BOOK\nTY  - BOOK\nTI  - nested\nER  -\n",
        "TY  - THES\nTI  - Thesis\nER  -\n",
        "TY  - BOOK\nTI  - A\nER  - data\n",
        "TY  - JOUR\nTI  - T\nEP  - 5\nER  -\n",
        "TY  - JOUR\nTI  - T\nSP  - 1-4\nEP  - 5\nER  -\n",
        "TY  - ELEC\nTI  - T\nVL  - 2026\nER  -\n",
        "TY  - BOOK\nTI  - T\nT2  - Series\nER  -\n",
        "TY  - BOOK\nTI  - \nER  -\n",
        "TY  - BOOK\nTI  - A\0B\nER  -\n",
        "TY  - JOUR\nTI  - T\nDO  - 10.1234/one#fragment\nER  -\n",
        "TY  - JOUR\nTI  - T\nDO  - 10.1234/<unsafe>\nER  -\n",
        "TY  - JOUR\nTI  - T\nID  - a\0b\nER  -\n",
    ] {
        check!(
            decode_ris(bad.as_bytes()).is_err(),
            "invalid framing or unrepresentable field"
        );
    }
    check!(
        decode_ris(&[0xff]).is_err() && decode_ris(&[0xff, 0xfe, 0]).is_err(),
        "malformed Unicode rejected"
    );
    let mut e = Editor::new(Document::default()).unwrap();
    let before = e.document().clone();
    let invalid_batch = format!("{input}TY  - BOOK\nTI  - B\nKW  - cannot retain\nER  -\n");
    if let Ok(sources) = decode_ris(invalid_batch.as_bytes()) {
        let _ = e.apply(Edit::ImportSources(sources));
    }
    check!(
        e.document() == &before && !e.dirty(),
        "failure in second record exposes no partial import"
    );
    let simple = Source {
        title: String::from("Title"),
        ..Default::default()
    };
    for variant in 0..7 {
        let mut s = simple.clone();
        match variant {
            0 => s.corporate = String::from("Institute"),
            1 => s.external_id = String::from("{GUID}"),
            2 => s.authors.push(Person {
                family: String::from("Smith"),
                given: String::from("Anna"),
                middle: String::from("J."),
            }),
            3 => s.year = String::from("2026/"),
            4 => s.comments = String::from("CR\rinside"),
            5 => s.authors.push(Person {
                family: String::from("Smith, Jr."),
                ..Default::default()
            }),
            _ => s.day_accessed = String::from("10"),
        }
        check!(
            encode_ris(&[simple.clone(), s]).is_err(),
            "unrepresentable export fails atomically"
        );
    }
    check!(
        decode_ris(&vec![b' '; (4 << 20) + 1]) == Err(doc_io::Error::Limit),
        "input budget"
    );
    check!(
        decode_ris(format!("TY  - BOOK\nTI  - {}\nER  -\n", "x".repeat(4097)).as_bytes())
            == Err(doc_io::Error::Limit),
        "field budget"
    );
    check!(
        decode_ris(
            format!("TY  - BOOK\nTI  - T\n{}ER  -\n", "AU  - Smith\n".repeat(65)).as_bytes()
        ) == Err(doc_io::Error::Limit),
        "author budget"
    );
    check!(
        decode_ris("TY  - BOOK\nTI  - T\nER  -\n".repeat(1025).as_bytes())
            == Err(doc_io::Error::Limit),
        "record budget"
    );
    check!(
        decode_ris(format!("TY  - BOOK\n{}ER  -\n", "TI  - T\n".repeat(4096)).as_bytes())
            == Err(doc_io::Error::Limit),
        "tag budget"
    );
    let heavy = Source {
        title: "x".repeat(4096),
        ..Default::default()
    };
    check!(
        encode_ris(&vec![heavy; 512]) == Err(doc_io::Error::Limit),
        "retained model budget"
    );
    (passed, failed)
}
