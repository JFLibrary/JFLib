// BibTeX interchange regression checks, without an external runtime or test framework.
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{Person, Source, SourceKind};

fn source() -> Source {
    Source {
        id: 1,
        title: String::from("Methoden & Modelle: 50%"),
        year: String::from("2026"),
        authors: vec![
            Person {
                family: String::from("Müller"),
                given: String::from("Anna Maria"),
                ..Default::default()
            },
            Person {
                family: String::from("李"),
                given: String::from("明"),
                ..Default::default()
            },
            Person {
                family: String::from("van Doe"),
                ..Default::default()
            },
        ],
        publisher: String::from("Forschungsverlag"),
        city: String::from("Berlin"),
        edition: String::from("2"),
        volume: String::from("4"),
        pages: String::from("17--29"),
        url: String::from("https://example.org/a_b?q=1&part=2#anchor"),
        standard_number: String::from("978-1-23456-789-0"),
        year_accessed: String::from("2024"),
        month_accessed: String::from("02"),
        day_accessed: String::from("29"),
        comments: String::from("Prüfung #1: Kosten $5, Variable a_b"),
        ..Default::default()
    }
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL BibTeX {}\n", $name).as_bytes());
            }
        };
    }
    let basic = source();
    let mut article = Source {
        id: 2,
        kind: SourceKind::JournalArticle,
        title: String::from("Offene Wissenschaft"),
        authors: vec![Person {
            family: String::from("Doe"),
            given: String::from("Jane"),
            ..Default::default()
        }],
        year: String::from("2025"),
        journal: String::from("Journal of Methods"),
        volume: String::from("12"),
        issue: String::from("3"),
        pages: String::from("1--10, 42"),
        standard_number: String::from("1234-5678"),
        ..Default::default()
    };
    let website = Source {
        id: 3,
        kind: SourceKind::Website,
        title: String::from("Forschung im Netz"),
        corporate: String::from("Research and Science, Inc."),
        website: String::from("Wissenschaftsportal"),
        url: String::from("https://example.org/a%20b?x_y=1&cost=$5#page"),
        year_accessed: String::from("2026"),
        month_accessed: String::from("10"),
        ..Default::default()
    };
    let sources = vec![basic.clone(), article.clone(), website.clone()];
    let encoded = doc_io::encode_bibtex(&sources).unwrap();
    check!(
        doc_io::decode_bibtex(&encoded) == Ok(sources.clone()),
        "complete supported model roundtrip"
    );
    check!(
        doc_io::encode_bibtex(&sources) == Ok(encoded.clone()),
        "deterministic export"
    );
    let output = core::str::from_utf8(&encoded).unwrap();
    check!(
        output.contains("\\&")
            && output.contains("\\%")
            && output.contains("\\$")
            && output.contains("\\#")
            && output.contains("\\_"),
        "safe special-character escaping"
    );
    check!(
        output.contains("author = {{Research and Science, Inc.}}")
            && output.contains("howpublished = {Wissenschaftsportal}"),
        "corporate and website output"
    );
    check!(
        output.contains("issn = {1234-5678}") && output.contains("isbn = {978-1-23456-789-0}"),
        "standard-number type mapping"
    );
    for encoding in [
        base::encoding::Encoding::Utf8Bom,
        base::encoding::Encoding::Utf16Le,
        base::encoding::Encoding::Utf16Be,
    ] {
        check!(
            doc_io::decode_bibtex(&base::encoding::encode(output, encoding)) == Ok(sources.clone()),
            "BOM encoding"
        );
    }
    let literal = b"% leading comment\n@comment{ignored {nested} content}\n@BOOK(external:key, AUTHOR=\"Lovelace, Ada and Hopper, Grace B.\", TITLE={Analytical Engine}, YEAR=1843,)\n@misc{web, title=\"Science\", author={{Science Group}}, url={https://example.org/a%20b}}";
    let decoded = doc_io::decode_bibtex(literal).unwrap();
    check!(
        decoded.len() == 2
            && decoded[0].id == 1
            && decoded[1].id == 2
            && decoded.iter().all(|s| s.external_id.is_empty()),
        "external keys remapped without fake external IDs"
    );
    check!(
        decoded[0].authors[1].given == "Grace B."
            && decoded[0].authors[1].middle.is_empty()
            && decoded[1].corporate == "Science Group",
        "author spelling and order"
    );
    check!(
        decoded[0].year == "1843" && decoded[1].kind == SourceKind::Website,
        "numeric year and web misc"
    );
    check!(
        doc_io::decode_bibtex(b"@book{k,title={A\t long\n title},note={Quote: \"yes\"}}")
            .is_ok_and(|s| s[0].title == "A long title" && s[0].comments == "Quote: \"yes\""),
        "BibTeX whitespace normalization and quotes"
    );
    check!(
        doc_io::decode_bibtex(b"@book{k,title={A},author={Doe AND Smith}}")
            .is_ok_and(|s| s[0].authors.len() == 2),
        "case-insensitive author separator"
    );
    check!(
        doc_io::decode_bibtex(b"@comment(ignored (nested) content)").is_ok_and(|s| s.is_empty())
            && doc_io::encode_bibtex(&[]).is_ok_and(|s| s.is_empty()),
        "empty bibliography and comment"
    );
    let bad = [
        ("@book{k,title={A},title={B}}", "duplicate field"),
        ("@book{k,title={A}} @book{k,title={B}}", "duplicate key"),
        (
            "@book{k,title={A},YEAR=1,year=2}",
            "case-insensitive duplicate field",
        ),
        (
            "@book{k,title={A},doi={10.1234/example}}",
            "unrepresented DOI",
        ),
        (
            "@book{k,title={A},editor={Doe, Jane}}",
            "unsupported author role",
        ),
        ("@book{k,title={A},crossref={parent}}", "cross-reference"),
        (
            "@book{k,title={A},issn={1234-5678}}",
            "wrong standard-number type",
        ),
        (
            "@article{k,title={A},howpublished={site}}",
            "wrong website-title type",
        ),
        ("@inproceedings{k,title={A}}", "unsupported entry type"),
        ("@string{foo={A}}", "string definition"),
        ("@preamble{\"anything\"}", "preamble"),
        ("@book{k,title=foo}", "macro value"),
        ("@book{k,title={A} # {B}}", "concatenation"),
        (
            "@book{k,title={The {NASA} study}}",
            "case-protection grouping",
        ),
        (
            "@book{k,title={The \\{literal\\} study}}",
            "literal braces are not dropped",
        ),
        ("@book{k,title={\\textbf{A}}}", "TeX command"),
        ("@book{k,title={\\\"a}}", "TeX accent command"),
        ("@book{k,title={Cost $5}}", "unescaped special character"),
        (
            "@book{k,title={A},author={Jane Doe}}",
            "ambiguous unseparated personal name",
        ),
        ("@book{k,title={A},author={Doe, Jr, Jane}}", "name suffix"),
        (
            "@book{k,title={A},author={{Group} and Doe, Jane}}",
            "mixed corporate and personal authors",
        ),
        ("@book{k,title={A},author={others}}", "et al marker"),
        ("@book{k,title={A},author={Doe and }}", "empty author"),
        ("@book{k,title={A},author={{}}}", "empty corporation"),
        (
            "@book{k,title={A},urldate={2023-02-29}}",
            "invalid leap day",
        ),
        (
            "@book{k,title={A},urldate={2026-04-31}}",
            "invalid month day",
        ),
        (
            "@book{k,title={A},urldate={2026-1-02}}",
            "noncanonical access date",
        ),
        ("@misc{k,title={Printed misc}}", "misc without website URL"),
        ("@book{k,year=2026}", "missing title"),
        ("@book{k,title={A}", "unfinished record"),
        ("@book{k,title=\"A}", "unbalanced quoted value"),
        ("@book{k,title={A},} ", "trailing data"),
    ];
    for (input, name) in bad {
        check!(doc_io::decode_bibtex(input.as_bytes()).is_err(), name);
    }
    let error = doc_io::decode_bibtex(b"@book{record17,title={A},hidden={B}}");
    check!(
        matches!(error, Err(doc_io::Error::Unsupported(ref text)) if text.contains("record17") && text.contains("hidden")),
        "entry and field diagnostics"
    );
    check!(
        doc_io::decode_bibtex(b"@book{ok,title={Good}} @book{bad,title={Bad},unknown={data}}")
            .is_err(),
        "invalid second record aborts entire import"
    );
    check!(
        doc_io::decode_bibtex(&[0xff, 0xfe, 0, 0]).is_err()
            && doc_io::decode_bibtex(&[0xff]).is_err(),
        "invalid or unsupported encoding"
    );
    let mut long = basic.clone();
    long.title = "a".repeat(4096);
    check!(
        doc_io::encode_bibtex(&[long.clone()]).and_then(|b| doc_io::decode_bibtex(&b))
            == Ok(vec![long.clone()]),
        "4096-byte scalar boundary"
    );
    long.title.push('a');
    check!(
        doc_io::encode_bibtex(&[long]).is_err()
            && doc_io::decode_bibtex(
                format!("@book{{k,title={{{}}}}}", "a".repeat(4097)).as_bytes()
            )
            .is_err(),
        "oversized scalar rejected"
    );
    let mut long_name = basic.clone();
    long_name.authors = vec![Person {
        family: "a".repeat(4096),
        given: "b".repeat(4096),
        ..Default::default()
    }];
    check!(
        doc_io::encode_bibtex(&[long_name.clone()]).and_then(|b| doc_io::decode_bibtex(&b))
            == Ok(vec![long_name]),
        "separate 4096-byte name component bounds"
    );
    let names = (0..65)
        .map(|n| format!("Family{n}, Given"))
        .collect::<Vec<_>>()
        .join(" and ");
    check!(
        doc_io::decode_bibtex(format!("@book{{k,title={{A}},author={{{names}}}}}").as_bytes())
            .is_err(),
        "author-count bound"
    );
    check!(
        doc_io::decode_bibtex(&vec![b' '; (4 << 20) + 1]) == Err(doc_io::Error::Limit),
        "input byte bound"
    );
    let records = (0..1025)
        .map(|n| format!("@book{{k{n},title={{A}}}}\n"))
        .collect::<String>();
    check!(
        doc_io::decode_bibtex(records.as_bytes()) == Err(doc_io::Error::Limit),
        "record count bound"
    );
    let large = (0..260)
        .map(|n| {
            format!(
                "@book{{k{n},title={{{}}},note={{{}}}}}\n",
                "a".repeat(4096),
                "b".repeat(4096)
            )
        })
        .collect::<String>();
    check!(
        doc_io::decode_bibtex(large.as_bytes()) == Err(doc_io::Error::Limit),
        "retained-model byte bound"
    );
    let nested = format!("@book{{k,title={}{}}}", "{".repeat(51), "}".repeat(51));
    check!(
        doc_io::decode_bibtex(nested.as_bytes()).is_err(),
        "bounded nested lexer"
    );
    for field in [
        "middle",
        "external_id",
        "language",
        "short_title",
        "whitespace",
        "literal_braces",
        "backslash",
        "and_name",
        "given_only",
        "date",
    ] {
        let mut unrepresentable = basic.clone();
        match field {
            "middle" => unrepresentable.authors[0].middle = String::from("B."),
            "external_id" => unrepresentable.external_id = String::from("{external}"),
            "language" => unrepresentable.language = String::from("1031"),
            "short_title" => unrepresentable.short_title = String::from("Short"),
            "whitespace" => unrepresentable.title = String::from("Two  spaces"),
            "literal_braces" => unrepresentable.title = String::from("Literal {braces}"),
            "backslash" => unrepresentable.title = String::from("Literal \\ path"),
            "and_name" => unrepresentable.authors[0].family = String::from("Research and Science"),
            "given_only" => unrepresentable.authors[0].family.clear(),
            "date" => unrepresentable.month_accessed = String::from("2"),
            _ => unreachable!(),
        }
        check!(doc_io::encode_bibtex(&[unrepresentable]).is_err(), field);
    }
    article.id = 1;
    check!(
        doc_io::encode_bibtex(&[basic.clone(), article]).is_err(),
        "duplicate model identity"
    );
    let mut invalid_id = basic;
    invalid_id.id = 0;
    check!(
        doc_io::encode_bibtex(&[invalid_id]).is_err(),
        "unallocated model identity"
    );
    (passed, failed)
}
