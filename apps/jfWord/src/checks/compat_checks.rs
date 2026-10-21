// DOCX preservation, OPC identity and text-patch contracts, independent of the strict model.
use alloc::{format, string::String, vec, vec::Vec};
use archive::{Entry, Limits};
use doc_core::{Document, Inline};
use doc_io::{DocxFile, DocxMode};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

fn entry(name: &str, text: &str) -> Entry {
    Entry {
        name: String::from(name),
        data: text.as_bytes().to_vec(),
    }
}
fn parts() -> Vec<Entry> {
    vec![
        entry(
            "[Content_Types].xml",
            &format!(
                "<Types xmlns=\"{CT}\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"bin\" ContentType=\"application/octet-stream\"/><Override PartName=\"/content/main.xml\" ContentType=\"{MAIN}\"/></Types>"
            ),
        ),
        entry(
            "_rels/.rels",
            &format!(
                "<Relationships xmlns=\"{REL}\"><Relationship Id=\"main\" Type=\"{R}/officeDocument\" Target=\"content/main.xml\"/></Relationships>"
            ),
        ),
        entry(
            "content/_rels/main.xml.rels",
            &format!(
                "<Relationships xmlns=\"{REL}\"><Relationship Id=\"link\" Type=\"{R}/hyperlink\" Target=\"https://example.invalid/no-fetch\" TargetMode=\"External\"/><Relationship Id=\"attachment\" Type=\"{R}/oleObject\" Target=\"../assets/unknown.bin\"/></Relationships>"
            ),
        ),
        entry(
            "content/main.xml",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body><w:p><w:pPr><w:pStyle w:val=\"Title\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Calibri\"/><w:b/></w:rPr><w:t>Originaler Text α</w:t></w:r><w:r><w:t xml:space='preserve'> bleibt erhalten.</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Zelle A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Zelle B</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:hyperlink r:id=\"link\"><w:r><w:t>Externer Link</w:t></w:r></w:hyperlink><w:fldSimple w:instr=\" DATE \"><w:r><w:t>11.10.2026</w:t></w:r></w:fldSimple><w:r><w:t/></w:r></w:p><w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>"
            ),
        ),
        Entry {
            name: String::from("assets/unknown.bin"),
            data: vec![0, 1, 2, 0xff, 0x80, 6],
        },
        entry(
            "docProps/custom.xml",
            "<custom xmlns=\"urn:vendor:metadata\"><keep value=\"unchanged\"/></custom>",
        ),
    ]
}
fn pack(parts: &[Entry]) -> Vec<u8> {
    archive::write(parts, Limits::default()).unwrap()
}
fn replace(parts: &[Entry], path: &str, before: &str, after: &str) -> Vec<Entry> {
    parts
        .iter()
        .map(|part| {
            if part.name == path {
                Entry {
                    name: part.name.clone(),
                    data: String::from_utf8(part.data.clone())
                        .unwrap()
                        .replace(before, after)
                        .into_bytes(),
                }
            } else {
                part.clone()
            }
        })
        .collect()
}
fn mutate(
    parts: &[Entry],
    path: &str,
    before: &str,
    after: &str,
) -> Result<DocxFile, doc_io::Error> {
    DocxFile::open(&pack(&replace(parts, path, before, after)))
}
/// Foreign-layout fixture for the app's explicit preserved-text reading mode.
pub(crate) fn sample() -> Vec<u8> {
    pack(&parts())
}

pub(crate) fn probe(args: &base::Args) -> Option<i32> {
    let path = args.value("--check-preserved")?;
    let rejected = args.flag("--expect-reject");
    let result = (|| -> Result<DocxFile, doc_io::Error> {
        let mut file = DocxFile::load(path)?;
        if let Some(index) = args.value("--edit-index") {
            let index = index.parse::<usize>().map_err(|_| doc_io::Error::Invalid)?;
            let expected = args
                .value("--expected-file")
                .ok_or(doc_io::Error::Invalid)?;
            let replacement = args
                .value("--replacement-file")
                .ok_or(doc_io::Error::Invalid)?;
            let expected = doc_io::read_file(expected, 65536)?;
            let replacement = doc_io::read_file(replacement, 65536)?;
            let expected = core::str::from_utf8(&expected).map_err(|_| doc_io::Error::Invalid)?;
            let replacement =
                core::str::from_utf8(&replacement).map_err(|_| doc_io::Error::Invalid)?;
            file.replace_text(index, expected, replacement)?;
        }
        if rejected {
            return Ok(file);
        }
        if let Some(path) = args.value("--output") {
            file.save(path, None)?;
        }
        Ok(file)
    })();
    Some(match result {
        Ok(file) if !rejected => {
            plat::write_stdout(
                format!(
                    "PASS DOCX {:?} parts={} runs={} paragraphs={} dirty={}\n",
                    file.report().mode,
                    file.report().part_count,
                    file.text_runs().len(),
                    file.preview().len(),
                    file.dirty()
                )
                .as_bytes(),
            );
            0
        }
        Ok(_) => {
            plat::write_stdout(b"FAIL unexpected preserved acceptance\n");
            1
        }
        Err(error) => {
            plat::write_stdout(
                format!(
                    "{} {error:?}\n",
                    if rejected { "PASS rejected" } else { "FAIL" }
                )
                .as_bytes(),
            );
            if rejected { 0 } else { 1 }
        }
    })
}

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    let original = sample();
    let mut file = DocxFile::open(&original).unwrap();
    check!(
        file.document().is_none() && file.report().mode == DocxMode::Preserved,
        "foreign model is never flattened into an editable Document"
    );
    check!(
        file.report().main_part == "content/main.xml" && file.report().part_count == 6,
        "OPC resolves nonstandard main location"
    );
    check!(
        file.preview()
            == [
                "Originaler Text α bleibt erhalten.",
                "Zelle A",
                "Zelle B",
                "Externer Link11.10.2026"
            ],
        "reading excerpt includes tables and cached fields in main-story order"
    );
    check!(
        file.report().diagnostics.iter().any(|s| s == "Tabellen")
            && file
                .report()
                .diagnostics
                .iter()
                .any(|s| s.contains("Hyperlinks")),
        "compatibility report inventories unsupported structures"
    );
    check!(
        file.encode(None) == Ok(original.clone()),
        "no-op preserved save is original-byte exact"
    );
    check!(
        file.encode(Some(&Document::default())).is_err(),
        "preserved package rejects lossy editable model"
    );
    check!(
        file.text_runs().len() == 5
            && file.text_runs()[2].paragraph == 2
            && file.text_runs()[4].text.is_empty(),
        "safe direct text runs include table cells and self-closing empty text"
    );
    check!(
        !file.dirty() && !file.can_undo() && !file.can_redo() && file.revision() == 0,
        "new preserved session is clean without history"
    );
    check!(
        file.replace_text(0, "stale", "wrong").is_err() && !file.dirty(),
        "text edit compares current expected value"
    );
    check!(
        file.replace_text(99, "", "wrong").is_err() && !file.dirty(),
        "out-of-range text edit cannot alter content"
    );
    check!(
        file.replace_text(0, "Originaler Text α", "new\nparagraph")
            .is_err(),
        "text patches cannot insert structural line breaks"
    );
    check!(
        file.replace_text(0, "Originaler Text α", "\0").is_err(),
        "invalid XML replacement is rejected"
    );
    check!(
        file.replace_text(0, "Originaler Text α", &"x".repeat(65537))
            .is_err(),
        "individual text replacement is bounded"
    );
    check!(
        file.replace_text(0, "Originaler Text α", "Originaler Text α")
            .is_ok()
            && !file.can_undo() && file.revision() == 0,
        "no-op text edit does not consume history"
    );
    check!(
        file.replace_text(0, "Originaler Text α", " Geändert <&> 中文 ")
            .is_ok()
            && file.dirty() && file.revision() == 1,
        "Unicode text edit retains explicit surrounding spaces"
    );
    check!(
        file.preview()[0] == " Geändert <&> 中文  bleibt erhalten.",
        "reading excerpt follows accepted text edit"
    );
    let changed = file.encode(None).unwrap();
    let changed_parts = archive::read(&changed, Limits::default()).unwrap();
    let initial_parts = parts();
    check!(
        changed_parts
            .iter()
            .filter(|p| p.name != "content/main.xml")
            .all(|p| initial_parts.iter().any(|q| p == q)),
        "every unrelated package payload is byte-identical after edit"
    );
    let main = changed_parts
        .iter()
        .find(|p| p.name == "content/main.xml")
        .unwrap();
    let expected = String::from_utf8(
        initial_parts
            .iter()
            .find(|p| p.name == "content/main.xml")
            .unwrap()
            .data
            .clone(),
    )
    .unwrap()
    .replace(
        "<w:t>Originaler Text α</w:t>",
        "<w:t xml:space=\"preserve\"> Geändert &lt;&amp;&gt; 中文 </w:t>",
    );
    check!(
        main.data == expected.as_bytes(),
        "only selected XML leaf and necessary whitespace flag change"
    );
    let reopened = DocxFile::open(&changed).unwrap();
    check!(
        reopened.text_runs()[0].text == " Geändert <&> 中文 "
            && reopened.encode(None) == Ok(changed.clone()),
        "modified foreign document reopens without normalizing its package"
    );
    file.mark_saved();
    check!(
        !file.dirty() && file.can_undo() && file.revision() == 1,
        "save checkpoint keeps text undo"
    );
    check!(
        file.undo() && file.dirty() && file.encode(None) == Ok(original.clone()) && file.revision() == 2,
        "undo to original restores exact original archive"
    );
    check!(
        file.redo() && !file.dirty() && file.encode(None) == Ok(changed.clone()) && file.revision() == 3,
        "redo reaches exact saved checkpoint"
    );
    check!(
        file.replace_text(4, "", "Neu").is_ok()
            && file.preview()[3] == "Externer Link11.10.2026Neu",
        "self-closing text leaf can gain text without moving its run"
    );
    check!(
        file.undo()
            && file.undo()
            && file
                .replace_text(0, "Originaler Text α", "Andere Änderung")
                .is_ok()
            && !file.can_redo(),
        "new edit after undo truncates redo branch"
    );
    for i in 0..70 {
        let expected = file.text_runs()[0].text.clone();
        file.replace_text(0, &expected, &format!("Schritt {i}"))
            .unwrap();
    }
    let mut count = 0;
    while file.undo() {
        count += 1;
    }
    check!(count == 64, "preserved text undo is bounded to 64 edits");

    let native = doc_io::encode_docx(&Document::default()).unwrap();
    let native_file = DocxFile::open(&native).unwrap();
    check!(
        native_file.report().mode == DocxMode::Editable
            && native_file.document() == Some(&Document::default())
            && native_file.text_runs().is_empty(),
        "strict native documents retain full editable model"
    );
    check!(
        native_file.encode(native_file.document()) == Ok(native.clone()),
        "unchanged editable file preserves archive bytes"
    );
    let mut model = Document::default();
    model.paragraphs[0].content = vec![Inline::Text(String::from("Vollständig bearbeitet"))];
    check!(
        native_file
            .encode(Some(&model))
            .is_ok_and(|b| doc_io::decode_docx(&b) == Ok(model)),
        "changed native model uses complete strict serializer"
    );
    let notes = doc_io::encode_docx(&crate::notes_io_checks::sample()).unwrap();
    check!(
        DocxFile::open(&notes).is_ok_and(|f| f.report().mode == DocxMode::Editable),
        "native notes remain fully editable in session loader"
    );

    for encoding in [
        base::encoding::Encoding::Utf8Bom,
        base::encoding::Encoding::Utf16Le,
        base::encoding::Encoding::Utf16Be,
    ] {
        let mut encoded_parts = parts();
        let main = encoded_parts
            .iter_mut()
            .find(|p| p.name == "content/main.xml")
            .unwrap();
        let original_xml = String::from_utf8(main.data.clone()).unwrap();
        let xml = if encoding == base::encoding::Encoding::Utf8Bom {
            original_xml
        } else {
            original_xml.replace("UTF-8", "UTF-16")
        };
        main.data = base::encoding::encode(&xml, encoding);
        let bytes = pack(&encoded_parts);
        let mut file = DocxFile::open(&bytes).unwrap();
        check!(
            file.encode(None) == Ok(bytes),
            "foreign Unicode encoding unchanged copy is byte exact"
        );
        file.replace_text(0, "Originaler Text α", "UTF Ω 😀")
            .unwrap();
        let encoded = file.encode(None).unwrap();
        let entries = archive::read(&encoded, Limits::default()).unwrap();
        let raw = &entries
            .iter()
            .find(|p| p.name == "content/main.xml")
            .unwrap()
            .data;
        let (text, actual_encoding) = base::encoding::decode(raw, 4 << 20).unwrap();
        check!(
            actual_encoding == encoding
                && text
                    == xml.replace(
                        "<w:t>Originaler Text α</w:t>",
                        "<w:t xml:space=\"preserve\">UTF Ω 😀</w:t>"
                    ),
            "text patch preserves XML encoding and unrelated lexical bytes"
        );
        check!(
            DocxFile::open(&encoded).is_ok_and(|f| f.text_runs()[0].text == "UTF Ω 😀"),
            "patched Unicode package can be reopened"
        );
    }
    let renamed = replace(&parts(), "content/main.xml", "w:", "word:");
    let renamed = replace(&renamed, "content/main.xml", "xmlns:w=", "xmlns:word=");
    check!(
        DocxFile::open(&pack(&renamed)).is_ok_and(|f| f.text_runs().len() == 5),
        "text patch eligibility is namespace-aware"
    );
    let strict = replace(
        &parts(),
        "content/main.xml",
        W,
        "http://purl.oclc.org/ooxml/wordprocessingml/main",
    );
    let strict = replace(
        &strict,
        "_rels/.rels",
        &format!("{R}/officeDocument"),
        "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
    );
    check!(
        DocxFile::open(&pack(&strict)).is_ok_and(|f| f.text_runs().len() == 5),
        "Strict OOXML can be preserved and selectively text-edited"
    );
    for (before, after) in [
        (
            "<w:body>",
            "<w:body><w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r></w:p>",
        ),
        (
            "<w:body>",
            "<w:body><w:ins><w:p><w:r><w:t>Revision</w:t></w:r></w:p></w:ins>",
        ),
        (
            "<w:body>",
            "<w:body><w:del><w:p><w:r><w:delText>Deleted</w:delText></w:r></w:p></w:del>",
        ),
    ] {
        check!(
            mutate(&parts(), "content/main.xml", before, after)
                .is_ok_and(|f| f.text_runs().is_empty() && f.encode(None).is_ok()),
            "fields and tracked revisions are preservation-only"
        );
    }
    for settings in [
        "<w:documentProtection w:enforcement=\"1\"/>",
        "<w:trackRevisions/>",
    ] {
        let mut protected = parts();
        protected.push(entry(
            "content/settings.xml",
            &format!("<w:settings xmlns:w=\"{W}\">{settings}</w:settings>"),
        ));
        check!(
            DocxFile::open(&pack(&protected)).is_ok_and(|f| f.text_runs().is_empty()),
            "document protection and tracking settings prohibit text patches"
        );
    }
    let signed = replace(
        &parts(),
        "docProps/custom.xml",
        "unchanged",
        "digital-signature",
    );
    check!(
        DocxFile::open(&pack(&signed)).is_ok_and(|f| f.text_runs().is_empty()),
        "signature-bearing package cannot be edited"
    );
    let hidden = replace(&parts(), "content/main.xml", "<w:b/>", "<w:b/><w:vanish/>");
    check!(DocxFile::open(&pack(&hidden)).is_ok_and(|f| f.text_runs().len() == 4 && f.text_runs()[0].text == " bleibt erhalten."), "hidden runs are excluded from text editing");
    let control = replace(
        &parts(),
        "content/main.xml",
        "<w:body>",
        "<w:body><w:sdt><w:sdtContent><w:p><w:r><w:t>Control</w:t></w:r></w:p></w:sdtContent></w:sdt>",
    );
    check!(
        DocxFile::open(&pack(&control))
            .is_ok_and(|f| f.text_runs().len() == 5 && f.preview()[0] == "Control"),
        "content controls remain visible but are not editable leaves"
    );
    for (name, content_type) in [
        (
            "settings.XML",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        ),
        (
            "settings",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        ),
        ("opaque.bin", "application/octet-stream"),
    ] {
        let mut protected = parts();
        protected.push(entry(&format!("content/{name}"), &format!("<w:settings xmlns:w=\"{W}\"><w:documentProtection w:enforcement=\"1\"/></w:settings>")));
        protected = replace(
            &protected,
            "[Content_Types].xml",
            "</Types>",
            &format!(
                "<Override PartName=\"/content/{name}\" ContentType=\"{content_type}\"/></Types>"
            ),
        );
        protected = replace(
            &protected,
            "content/_rels/main.xml.rels",
            "</Relationships>",
            &format!(
                "<Relationship Id=\"settings\" Type=\"{R}/settings\" Target=\"{name}\"/></Relationships>"
            ),
        );
        check!(
            DocxFile::open(&pack(&protected)).is_ok_and(|f| f.text_runs().is_empty()),
            "settings protection is resolved by content type and relationships, never suffix"
        );
    }
    let mut signed = parts();
    signed.push(entry("signatures/unusual", "<Signature xmlns=\"http://www.w3.org/2000/09/xmldsig#\"><SignatureValue>TEST</SignatureValue></Signature>"));
    signed = replace(
        &signed,
        "[Content_Types].xml",
        "</Types>",
        "<Override PartName=\"/signatures/unusual\" ContentType=\"application/vnd.openxmlformats-package.digital&#45;signature-xmlsignature+xml\"/></Types>",
    );
    signed = replace(
        &signed,
        "_rels/.rels",
        "</Relationships>",
        &format!(
            "<Relationship Id=\"signature\" Type=\"{REL}/digital&#45;signature/signature\" Target=\"signatures/unusual\"/></Relationships>"
        ),
    );
    check!(
        DocxFile::open(&pack(&signed)).is_ok_and(|f| f.text_runs().is_empty()),
        "signature detection uses parsed names and attributes with expanded XML entities"
    );
    for encoding in [
        base::encoding::Encoding::Utf16Le,
        base::encoding::Encoding::Utf16Be,
    ] {
        let mut large = parts();
        let main = large
            .iter_mut()
            .find(|part| part.name == "content/main.xml")
            .unwrap();
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?><w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>Short</w:t></w:r></w:p><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:body></w:document>",
            "x".repeat((2 << 20) - 30000)
        );
        main.data = base::encoding::encode(&xml, encoding);
        let bytes = pack(&large);
        let mut session = DocxFile::open(&bytes).unwrap();
        check!(
            session
                .replace_text(0, "Short", &"z".repeat(60000))
                .is_err()
                && !session.dirty()
                && !session.can_undo()
                && session.text_runs()[0].text == "Short"
                && session.encode(None) == Ok(bytes),
            "UTF-16 encoded XML limit rejects atomically before history, dirty or save"
        );
    }
    let misplaced = replace(
        &parts(),
        "content/main.xml",
        "<w:body>",
        "<w:body><w:t>Not a direct paragraph run</w:t>",
    );
    check!(
        DocxFile::open(&pack(&misplaced)).is_ok_and(|f| f.text_runs().len() == 5),
        "eligible text must have direct document paragraph run ancestry"
    );
    let mut many = parts();
    let main = many
        .iter_mut()
        .find(|part| part.name == "content/main.xml")
        .unwrap();
    main.data = format!(
        "<w:document xmlns:w=\"{W}\"><w:body>{}</w:body></w:document>",
        "<w:p><w:r><w:t>Zeile</w:t></w:r></w:p>".repeat(4097)
    )
    .into_bytes();
    let many_bytes = pack(&many);
    let many = DocxFile::open(&many_bytes).unwrap();
    check!(
        many.report().preview_truncated
            && many.preview().len() == 4096
            && many.encode(None) == Ok(many_bytes),
        "truncated paragraph preview preserves the complete original package"
    );
    let mut large = parts();
    let main = large
        .iter_mut()
        .find(|part| part.name == "content/main.xml")
        .unwrap();
    main.data = format!("<w:document xmlns:w=\"{W}\"><w:body><w:p><w:r><w:t>{}</w:t></w:r></w:p><w:p><w:r><w:t>Tail</w:t></w:r></w:p></w:body></w:document>",
        "λ".repeat((1 << 19) + 1)).into_bytes();
    let large_bytes = pack(&large);
    let large = DocxFile::open(&large_bytes).unwrap();
    check!(
        large.report().preview_truncated
            && large.preview().len() == 1
            && large.preview()[0].len() == 1 << 20
            && large.encode(None) == Ok(large_bytes),
        "Unicode preview byte limit never truncates the retained package"
    );

    for (path, before, after) in [
        (
            "_rels/.rels",
            "Target=\"content/main.xml\"",
            "Target=\"content/main.xml\" TargetMode=\"External\"",
        ),
        (
            "_rels/.rels",
            "Target=\"content/main.xml\"",
            "Target=\"../content/main.xml\"",
        ),
        (
            "_rels/.rels",
            "Target=\"content/main.xml\"",
            "Target=\"content/missing.xml\"",
        ),
        (
            "_rels/.rels",
            "Target=\"content/main.xml\"",
            "Target=\"content%2Fmain.xml\"",
        ),
        ("_rels/.rels", "Id=\"main\"", "Id=\"\""),
        (
            "_rels/.rels",
            "</Relationships>",
            &format!(
                "<Relationship Id=\"second\" Type=\"{R}/officeDocument\" Target=\"content/main.xml\"/></Relationships>"
            ),
        ),
        (
            "content/_rels/main.xml.rels",
            "Id=\"attachment\"",
            "Id=\"link\"",
        ),
        (
            "content/_rels/main.xml.rels",
            "TargetMode=\"External\"",
            "TargetMode=\"Unclear\"",
        ),
        (
            "[Content_Types].xml",
            MAIN,
            "application/vnd.ms-word.document.macroEnabled.main+xml",
        ),
        (
            "[Content_Types].xml",
            "application/octet-stream",
            "application/vnd.ms-office.vbaProject",
        ),
        (
            "[Content_Types].xml",
            "PartName=\"/content/main.xml\"",
            "PartName=\"/missing.xml\"",
        ),
        ("content/main.xml", "<w:body>", "<w:notBody>"),
    ] {
        check!(
            mutate(&parts(), path, before, after).is_err(),
            "invalid OPC identity, macro content or malformed main XML is rejected"
        );
    }
    let mut vba = parts();
    vba.push(Entry {
        name: String::from("word/vbaProject.bin"),
        data: vec![1],
    });
    check!(
        DocxFile::open(&pack(&vba)).is_err(),
        "disguised VBA part is rejected independently of content type"
    );
    check!(
        DocxFile::open(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]).is_err(),
        "encrypted or legacy compound Word file is refused"
    );
    check!(
        mutate(
            &parts(),
            "_rels/.rels",
            "content/main.xml",
            "/content/./main.xml"
        )
        .is_ok(),
        "absolute normalized OPC main target is resolved"
    );
    (passed, failed)
}
