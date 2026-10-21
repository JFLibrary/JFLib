// Preserved edits across OPC stories, with independent lexical and history assertions.
use alloc::{format, string::String, vec::Vec};
use archive::{Entry, Limits};
use doc_io::{DocxFile, DocxStory};
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const TYPES: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";
fn replace(parts: &mut [Entry], path: &str, from: &str, to: &str) {
    let entry = parts.iter_mut().find(|p| p.name == path).unwrap();
    entry.data = String::from_utf8(entry.data.clone())
        .unwrap()
        .replace(from, to)
        .into_bytes();
}
fn parts() -> Vec<Entry> {
    let mut parts = archive::read(&crate::compat_checks::sample(), Limits::default()).unwrap();
    for (path, kind, root, body) in [
        (
            "stories/top.xml",
            "header",
            "hdr",
            "<w:p><w:r><w:rPr><w:b/><w:color w:val='123456'/></w:rPr><w:t>Kopf α</w:t></w:r><w:fldSimple w:instr=' DATE '><w:r><w:t>Cached date</w:t></w:r></w:fldSimple></w:p>",
        ),
        (
            "stories/bottom.xml",
            "footer",
            "ftr",
            "<w:p><w:r><w:t>Fuß Ω</w:t></w:r></w:p>",
        ),
        (
            "stories/foot.xml",
            "footnotes",
            "footnotes",
            "<w:footnote w:id='-1' w:type='separator'><w:p><w:r><w:t>Separator</w:t></w:r></w:p></w:footnote><w:footnote w:id='7'><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t>Anmerkung 中</w:t></w:r></w:p></w:footnote><w:footnote w:id='8' w:type='normal'><w:p><w:r><w:t>Zweite Note</w:t></w:r><w:sdt><w:sdtContent><w:r><w:t>Control</w:t></w:r></w:sdtContent></w:sdt></w:p></w:footnote>",
        ),
        (
            "stories/end.data",
            "endnotes",
            "endnotes",
            "<w:endnote w:id='0'><w:p><w:r><w:t>Endnote 😀</w:t></w:r></w:p></w:endnote>",
        ),
    ] {
        replace(
            &mut parts,
            "[Content_Types].xml",
            "</Types>",
            &format!("<Override PartName='/{path}' ContentType='{TYPES}.{kind}+xml'/></Types>"),
        );
        replace(
            &mut parts,
            "content/_rels/main.xml.rels",
            "</Relationships>",
            &format!(
                "<Relationship Id='{kind}' Type='{R}/{kind}' Target='../{path}'/></Relationships>"
            ),
        );
        parts.push(Entry {
            name: String::from(path),
            data: format!("<w:{root} xmlns:w='{W}'>{body}</w:{root}>").into_bytes(),
        });
    }
    replace(
        &mut parts,
        "content/_rels/main.xml.rels",
        "</Relationships>",
        &format!(
            "<Relationship Id='sharedHeader' Type='{R}/header' Target='../stories/top.xml'/></Relationships>"
        ),
    );
    parts
}
fn pack(parts: &[Entry]) -> Vec<u8> {
    archive::write(parts, Limits::default()).unwrap()
}
pub(crate) fn sample() -> Vec<u8> {
    pack(&parts())
}
pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr,$name:expr) => {
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
        file.text_runs().len() == 10,
        "all related stories discovered once, protected leaves excluded"
    );
    for (index, story, path, note) in [
        (5, DocxStory::Header, "stories/top.xml", None),
        (6, DocxStory::Footer, "stories/bottom.xml", None),
        (7, DocxStory::Footnote, "stories/foot.xml", Some(7)),
        (8, DocxStory::Footnote, "stories/foot.xml", Some(8)),
        (9, DocxStory::Endnote, "stories/end.data", Some(0)),
    ] {
        let run = &file.text_runs()[index];
        check!(
            run.index == index && run.story == story && run.part == path && run.note_id == note,
            "story metadata uses OPC path and original note identity"
        );
    }
    let preview = file.preview().to_vec();
    let mut expected = parts();
    for index in [5, 7, 6, 9, 8] {
        let run = file.text_runs()[index].clone();
        let after = format!(" Geändert {index} <&> Ω ");
        file.replace_text(index, &run.text, &after).unwrap();
        replace(
            &mut expected,
            &run.part,
            &format!("<w:t>{}</w:t>", run.text),
            &format!("<w:t xml:space=\"preserve\"> Geändert {index} &lt;&amp;&gt; Ω </w:t>"),
        );
        check!(
            file.preview() == preview && file.dirty(),
            "secondary story edit leaves main preview unchanged"
        );
    }
    let encoded = file.encode(None).unwrap();
    let actual = archive::read(&encoded, Limits::default()).unwrap();
    check!(
        actual == expected,
        "multiple story edits change only chosen lexical leaves in chosen parts"
    );
    let reopened = DocxFile::open(&encoded).unwrap();
    check!(
        reopened.text_runs() == file.text_runs(),
        "multi-story edits reopen with stable run identity"
    );
    file.mark_saved();
    for index in [8, 9, 6, 7, 5] {
        check!(
            file.undo()
                && file.text_runs()[index].text
                    == DocxFile::open(&original).unwrap().text_runs()[index].text,
            "global undo traverses edits across different stories in order"
        );
    }
    check!(
        file.encode(None).unwrap() == original && file.dirty(),
        "undo all restores original archive including compression metadata"
    );
    for _ in 0..5 {
        file.redo();
    }
    check!(
        !file.dirty() && file.encode(None).unwrap() == encoded,
        "redo reaches multi-story saved checkpoint"
    );
    file.undo();
    file.replace_text(0, "Originaler Text α", "Haupttext neu")
        .unwrap();
    check!(
        !file.can_redo() && file.preview()[0].starts_with("Haupttext neu"),
        "main-story branch clears global redo and updates preview"
    );
    file.undo();
    check!(
        file.preview() == preview,
        "main-story undo refreshes preview after secondary edits"
    );

    for (path, opening) in [("stories/top.xml", "<w:p>"), ("stories/foot.xml", "<w:p>")] {
        let mut changed = parts();
        replace(
            &mut changed,
            path,
            opening,
            "<w:p><w:r><w:fldChar w:fldCharType='begin'/></w:r>",
        );
        let session = DocxFile::open(&pack(&changed)).unwrap();
        check!(
            !session.text_runs().iter().any(|r| r.part == path)
                && session
                    .text_runs()
                    .iter()
                    .any(|r| r.story == DocxStory::Main)
                && session
                    .text_runs()
                    .iter()
                    .any(|r| r.story == DocxStory::Endnote),
            "complex fields block only their containing story part"
        );
    }
    for (path, from, to) in [
        ("[Content_Types].xml", ".header+xml", ".FAKE+xml"),
        ("content/_rels/main.xml.rels", "/header'", "/FAKE'"),
        ("stories/top.xml", "w:hdr", "w:body"),
    ] {
        let mut changed = parts();
        replace(&mut changed, path, from, to);
        let session = DocxFile::open(&pack(&changed)).unwrap();
        check!(
            !session
                .text_runs()
                .iter()
                .any(|r| r.story == DocxStory::Header)
                && session
                    .text_runs()
                    .iter()
                    .any(|r| r.story == DocxStory::Main),
            "header requires matching relationship, content type and root identity"
        );
    }
    let mut utf16 = parts();
    for entry in &mut utf16 {
        if entry.name.starts_with("stories/") {
            entry.data = base::encoding::encode(
                core::str::from_utf8(&entry.data).unwrap(),
                base::encoding::Encoding::Utf16Be,
            );
        }
    }
    let mut session = DocxFile::open(&pack(&utf16)).unwrap();
    session.replace_text(5, "Kopf α", "Unicode 😀").unwrap();
    let changed = archive::read(&session.encode(None).unwrap(), Limits::default()).unwrap();
    check!(
        changed
            .iter()
            .find(|e| e.name == "stories/top.xml")
            .unwrap()
            .data
            .starts_with(&[0xfe, 0xff])
            && changed
                .iter()
                .filter(|e| e.name != "stories/top.xml")
                .eq(utf16.iter().filter(|e| e.name != "stories/top.xml")),
        "secondary UTF-16 edit preserves BOM, encoding and every other payload"
    );
    let mut strict = parts();
    for entry in &mut strict {
        if let Ok(text) = core::str::from_utf8(&entry.data) {
            entry.data = text
                .replace(W, "http://purl.oclc.org/ooxml/wordprocessingml/main")
                .replace(R, "http://purl.oclc.org/ooxml/officeDocument/relationships")
                .into_bytes();
        }
    }
    check!(
        DocxFile::open(&pack(&strict)).unwrap().text_runs().len() == 10,
        "Strict namespace and relationships discover all eligible stories"
    );
    (passed, failed)
}
