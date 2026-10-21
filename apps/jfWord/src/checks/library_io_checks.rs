// Native library codec/store contracts, isolated below build/ and independent of profile state.
use alloc::{format, string::String, vec};
use doc_core::{Source, SourceLibrary};
use doc_io::{decode_source_library, encode_source_library, LibraryStore, LibraryStoreError};

fn sample() -> SourceLibrary {
    let mut sources = crate::checks::sample().sources;
    sources[0].id = 17;
    sources.push(Source {
        id: 300,
        title: String::from("Überblick & Vergleich"),
        corporate: String::from("Institut <Nord>"),
        comments: String::from("Eigene Bibliothek\r\nα β 𝄞"),
        ..Default::default()
    });
    SourceLibrary {
        sources,
        next_id: 901,
    }
}

pub(crate) fn probe(path: &str, output: Option<&str>, expect_reject: bool) -> i32 {
    let result = (|| {
        let bytes = doc_io::read_file(path, 4 << 20)?;
        let library = decode_source_library(&bytes)?;
        let encoded = encode_source_library(&library)?;
        if decode_source_library(&encoded)? != library {
            return Err(doc_io::Error::Invalid);
        }
        if !expect_reject {
            if let Some(output) = output {
                if output.is_empty() || output.contains('\0') {
                    return Err(doc_io::Error::Invalid);
                }
                if !plat::write_file(output, &encoded) {
                    return Err(doc_io::Error::Io);
                }
            }
        }
        Ok::<(), doc_io::Error>(())
    })();
    match result {
        Ok(()) if !expect_reject => {
            plat::write_stdout(b"PASS source library\n");
            0
        }
        Err(error) if expect_reject => {
            plat::write_stdout(format!("PASS rejected {error:?}\n").as_bytes());
            0
        }
        result => {
            plat::write_stdout(format!("FAIL source library {result:?}\n").as_bytes());
            1
        }
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
                plat::write_stdout(format!("FAIL {}\n", $name).as_bytes());
            }
        };
    }
    let original = sample();
    let bytes = encode_source_library(&original).unwrap();
    let xml = core::str::from_utf8(&bytes).unwrap();
    check!(
        decode_source_library(&bytes) == Ok(original.clone()),
        "native library preserves sparse IDs, order, metadata and next-ID high-water"
    );
    check!(
        encode_source_library(&decode_source_library(&bytes).unwrap()) == Ok(bytes.clone()),
        "native library output is deterministic"
    );
    check!(
        decode_source_library(&encode_source_library(&SourceLibrary::default()).unwrap())
            == Ok(SourceLibrary::default()),
        "empty native library round-trip"
    );
    let exhausted = SourceLibrary {
        sources: vec![],
        next_id: 0x7fff_ffff,
    };
    check!(
        decode_source_library(&encode_source_library(&exhausted).unwrap()) == Ok(exhausted),
        "exhausted source-ID sentinel survives restart"
    );
    for broken in [
        xml.replace("urn:jfword:source-library:1", "urn:jfword:source-library:2"),
        xml.replace("version=\"1\"", "version=\"2\""),
        xml.replace("version=\"1\"", "future=\"1\""),
        xml.replace("version=\"1\"", "j:version=\"1\""),
        xml.replace("version=\"1\"", "version=\"1\" future=\"yes\""),
        xml.replace("next-id=\"901\"", "next-id=\"0\""),
        xml.replace("next-id=\"901\"", "next-id=\"300\""),
        xml.replace("next-id=\"901\"", "next-id=\"2147483648\""),
        xml.replace("next-id=\"901\"", "next-id=\"+901\""),
        xml.replace("next-id=\"901\"", "next-id=\"\""),
        xml.replace("next-id=\"901\"", ""),
        xml.replace("<b:Sources ", "<b:Sources SelectedStyle=\"ignored\" "),
        xml.replace("</b:Source>", "<b:RefOrder>1</b:RefOrder></b:Source>"),
        xml.replace("</b:Source>", "<b:Future>future</b:Future></b:Source>"),
        xml.replace("</j:SourceLibrary>", "<j:Future/></j:SourceLibrary>"),
        xml.replace("</j:SourceLibrary>", "text</j:SourceLibrary>"),
        xml.replace("jf300", "jf17"),
        xml.replace("jf300", "jf0"),
        xml.replace("jf300", "external-tag"),
    ] {
        check!(
            decode_source_library(broken.as_bytes()).is_err(),
            "native library rejects unknown or invalid state"
        );
    }
    check!(
        decode_source_library(&vec![b' '; (4 << 20) + 1]).is_err(),
        "native library rejects input before oversized allocation"
    );
    let mut invalid = original.clone();
    invalid.sources[0].title.clear();
    check!(
        encode_source_library(&invalid).is_err(),
        "invalid library model cannot be encoded"
    );
    check!(
        LibraryStore::open("").is_err(),
        "empty library path rejected"
    );
    check!(
        LibraryStore::open("build/a\0b").is_err(),
        "NUL library path rejected"
    );
    check!(
        LibraryStore::recover("").is_err(),
        "empty recovery path rejected"
    );

    // File tests run only when the normal self-test caller requests its explicit build fixtures.
    if base::Args(plat::args()).value("--fixture").is_none() {
        return (passed, failed);
    }
    check!(
        plat::mkdir("build/jfword-library-recovery-ui"),
        "isolated GUI recovery directory"
    );
    check!(
        plat::write_file(
            "build/jfword-library-recovery-ui/sources.jflib",
            b"<broken-library>"
        ),
        "isolated malformed GUI library"
    );
    check!(
        plat::write_file("build/jfword-library-recovery-ui/sources.jflib.bak", &bytes),
        "isolated valid GUI backup"
    );
    let platform = if cfg!(windows) { "win" } else { "linux" };
    let folder = format!("build/library-io-check/{platform}/{}", plat::time_us());
    let path = format!("{folder}/sources.jfsources");
    let bak = format!("{path}.bak");
    let damaged = format!("{path}.damaged");
    let mut store = LibraryStore::open(&path).unwrap();
    check!(
        store.library() == &SourceLibrary::default()
            && store.path() == path
            && plat::file_exists(&folder) == 0,
        "opening absent library creates neither file nor parent directories"
    );
    check!(
        store.save(&invalid).is_err() && plat::file_exists(&folder) == 0,
        "invalid model save leaves filesystem untouched"
    );
    check!(
        store.save(&original).is_ok(),
        "first library save creates parents and file"
    );
    check!(
        store.library() == &original
            && plat::read_file(&path) == Some(bytes.clone())
            && plat::file_exists(&bak) == 0,
        "first save publishes validated library without inventing a backup"
    );
    check!(
        LibraryStore::open(&path).unwrap().library() == &original,
        "fresh store reads persisted IDs and source metadata"
    );
    let mut stale = LibraryStore::open(&path).unwrap();
    let mut changed = original.clone();
    changed.sources[0].title = String::from("Überarbeitete Methoden");
    let changed_bytes = encode_source_library(&changed).unwrap();
    check!(store.save(&changed).is_ok(), "second library save succeeds");
    check!(
        plat::read_file(&bak) == Some(bytes.clone())
            && plat::read_file(&path) == Some(changed_bytes.clone()),
        "backup contains prior validated version, current contains new version"
    );
    check!(
        stale.save(&original) == Err(LibraryStoreError::Conflict)
            && stale.library() == &original
            && plat::read_file(&path) == Some(changed_bytes.clone()),
        "stale writer cannot overwrite another writer"
    );
    check!(
        stale.restore_backup() == Err(LibraryStoreError::Conflict),
        "stale restore cannot overwrite another writer"
    );
    check!(
        stale.reload().is_ok() && stale.library() == &changed,
        "reload advances model and conflict baseline"
    );
    check!(
        store.save(&changed).is_ok() && plat::read_file(&bak) == Some(bytes.clone()),
        "identical save retains useful previous backup"
    );
    let held = plat::FileLock::acquire(&format!("{path}.lock")).unwrap();
    check!(
        stale.save(&original) == Err(LibraryStoreError::Busy)
            && plat::read_file(&path) == Some(changed_bytes.clone()),
        "held library lock rejects simultaneous save"
    );
    check!(
        stale.restore_backup() == Err(LibraryStoreError::Busy),
        "held library lock rejects simultaneous recovery"
    );
    drop(held);
    check!(
        stale.restore_backup().is_ok()
            && stale.library() == &original
            && plat::read_file(&path) == Some(bytes.clone())
            && plat::read_file(&bak) == Some(bytes.clone())
            && plat::read_file(&damaged) == Some(changed_bytes.clone()),
        "released lock permits restore while retaining both replaced bytes and good backup"
    );
    check!(
        stale.save(&invalid).is_err()
            && stale.library() == &original
            && plat::read_file(&path) == Some(bytes.clone()),
        "invalid subsequent save preserves disk and in-memory state"
    );
    let corrupt = b"<corrupt>unclosed";
    check!(
        plat::write_file(&path, corrupt),
        "corrupt library fixture written"
    );
    check!(
        LibraryStore::open(&path).is_err(),
        "existing corrupt library never opens as empty"
    );
    check!(
        stale.reload().is_err() && stale.library() == &original,
        "failed reload preserves previously validated model"
    );
    check!(
        stale.save(&changed) == Err(LibraryStoreError::Conflict)
            && plat::read_file(&path) == Some(corrupt.to_vec()),
        "corrupt external replacement prevents stale save"
    );
    check!(
        plat::delete(&damaged) && plat::mkdir(&damaged),
        "damaged-copy failure fixture"
    );
    check!(
        matches!(LibraryStore::recover(&path), Err(LibraryStoreError::Io))
            && plat::read_file(&path) == Some(corrupt.to_vec())
            && plat::read_file(&bak) == Some(bytes.clone()),
        "failed preservation of damaged bytes aborts recovery"
    );
    check!(
        plat::delete(&damaged),
        "damaged-copy failure fixture removed"
    );
    check!(
        LibraryStore::recover(&path).is_ok_and(|restored| restored.library() == &original)
            && plat::read_file(&path) == Some(bytes.clone())
            && plat::read_file(&bak) == Some(bytes.clone())
            && plat::read_file(&damaged) == Some(corrupt.to_vec()),
        "explicit recovery validates backup and preserves corrupted current bytes"
    );
    check!(
        LibraryStore::inspect_backup(&path) == Ok(original.clone()),
        "backup inspection returns validated model without writes"
    );
    let preview = LibraryStore::prepare_recovery(&path).unwrap();
    check!(
        preview.library() == &original,
        "recovery preview exposes validated backup"
    );
    check!(
        plat::write_file(&path, corrupt),
        "recovery preview main conflict fixture"
    );
    check!(
        matches!(preview.restore(), Err(LibraryStoreError::Conflict))
            && plat::read_file(&path) == Some(corrupt.to_vec()),
        "reviewed recovery detects changed current file before writing"
    );
    let preview = LibraryStore::prepare_recovery(&path).unwrap();
    check!(
        plat::write_file(&bak, &changed_bytes),
        "recovery preview backup conflict fixture"
    );
    check!(
        matches!(preview.restore(), Err(LibraryStoreError::Conflict))
            && plat::read_file(&path) == Some(corrupt.to_vec()),
        "reviewed recovery detects changed backup before writing"
    );
    check!(
        matches!(
            LibraryStore::recover_if_backup(&path, &original),
            Err(LibraryStoreError::Conflict)
        ),
        "model-based reviewed recovery also refuses a different backup"
    );
    check!(
        plat::write_file(&bak, &bytes),
        "restore reviewed backup fixture"
    );
    check!(
        LibraryStore::prepare_recovery(&path)
            .unwrap()
            .restore()
            .is_ok()
            && plat::read_file(&path) == Some(bytes.clone()),
        "fresh recovery preview restores exact reviewed files"
    );
    check!(
        plat::write_file(&bak, corrupt),
        "corrupt backup fixture written"
    );
    check!(
        LibraryStore::recover(&path).is_err() && plat::read_file(&path) == Some(bytes.clone()),
        "invalid backup never replaces valid main file"
    );
    check!(
        plat::delete(&bak) && plat::mkdir(&bak),
        "backup failure fixture is directory"
    );
    check!(
        matches!(
            LibraryStore::prepare_recovery(&path),
            Err(LibraryStoreError::Io)
        ),
        "non-file backup is rejected before opening"
    );
    let mut store = LibraryStore::open(&path).unwrap();
    check!(
        store.save(&changed) == Err(LibraryStoreError::Io)
            && store.library() == &original
            && plat::read_file(&path) == Some(bytes.clone()),
        "backup write failure aborts main write and preserves store"
    );
    check!(plat::delete(&bak), "backup failure fixture removed");
    check!(
        store.save(&changed).is_ok(),
        "failed backup releases lock and allows later successful save"
    );
    // Atomic writes use private, uniquely named siblings. A directory at the
    // destination is a real, portable failure independent of that naming scheme.
    // Retain the original file beside it so every saved byte can be verified.
    let held = format!("{path}.held");
    check!(
        plat::rename(&path, &held) && plat::mkdir(&path),
        "main atomic write failure fixture"
    );
    let save = store.save(&original);
    let replaced = plat::write_file(&path, &bytes);
    check!(
        save == Err(LibraryStoreError::Io)
            && !replaced
            && plat::file_exists(&path) == 2
            && store.library() == &changed
            && plat::read_file(&held) == Some(changed_bytes.clone()),
        "main write failure preserves saved model and file"
    );
    check!(
        plat::delete(&path) && plat::rename(&held, &path)
            && plat::read_file(&path) == Some(changed_bytes.clone()),
        "main write failure fixture removed"
    );
    check!(
        store.save(&original).is_ok() && store.save(&changed).is_ok(),
        "failed main write releases lock and later saves preserve backup order"
    );
    check!(plat::delete(&path), "external delete fixture");
    check!(
        store.save(&original) == Err(LibraryStoreError::Conflict),
        "external deletion conflicts with prior successful snapshot"
    );
    check!(
        store.reload().is_ok() && store.library() == &SourceLibrary::default(),
        "explicit reload accepts externally deleted file as empty memory state"
    );
    check!(
        store.restore_backup().is_ok() && store.library() == &original,
        "valid backup restores missing library"
    );
    let absent_path = format!("{folder}/absent.jfsources");
    let mut absent_a = LibraryStore::open(&absent_path).unwrap();
    let mut absent_b = LibraryStore::open(&absent_path).unwrap();
    check!(
        absent_a.save(&original).is_ok()
            && absent_b.save(&changed) == Err(LibraryStoreError::Conflict),
        "two absent snapshots cannot overwrite each other's first save"
    );
    check!(
        LibraryStore::open(&folder).is_err(),
        "directory at library filename is IO error, not empty library"
    );
    let large_path = format!("{folder}/large.jfsources");
    check!(
        plat::write_file(&large_path, &vec![b' '; (4 << 20) + 1]),
        "oversized file fixture"
    );
    check!(
        LibraryStore::open(&large_path).is_err(),
        "oversized file rejected before allocation"
    );
    let watermark_path = format!("{folder}/watermark.jfsources");
    let mut watermark_store = LibraryStore::open(&watermark_path).unwrap();
    let mut advanced = original.clone();
    advanced.sources.push(Source {
        id: 950,
        title: String::from("Später ergänzte Quelle"),
        ..Default::default()
    });
    advanced.next_id = 1001;
    check!(
        watermark_store.save(&original).is_ok() && watermark_store.save(&advanced).is_ok(),
        "persistent high-water recovery fixture"
    );
    check!(
        watermark_store.save(&original).is_err() && watermark_store.library() == &advanced,
        "normal save cannot roll back persistent ID high-water mark"
    );
    let preview = LibraryStore::prepare_recovery(&watermark_path).unwrap();
    let mut planned = original.clone();
    planned.next_id = 1001;
    check!(
        preview.library() == &planned,
        "recovery preview retains greater valid current ID high-water mark"
    );
    check!(
        preview
            .restore()
            .is_ok_and(|restored| restored.library() == &planned)
            && LibraryStore::open(&watermark_path).unwrap().library() == &planned,
        "restoring old source content does not recycle IDs issued since backup"
    );
    (passed, failed)
}
