// Bounded native process probes for independent codec and cooperative writer tests.
use alloc::format;

pub(crate) fn run(args: &base::Args) -> Option<i32> {
    if let Some(path) = args.value("--check-library") {
        return Some(crate::library_io_checks::probe(
            path,
            args.value("--output"),
            args.flag("--expect-reject"),
        ));
    }
    let hold = args.value("--hold-library-lock");
    let save = args.value("--library-save");
    if hold.is_none() && save.is_none() {
        return None;
    }
    let delay = match args.value("--wait-ms").unwrap_or("0").parse::<u32>() {
        Ok(ms) if ms <= 5000 => ms,
        _ => return Some(2),
    };
    let ready = || -> bool {
        args.value("--ready")
            .is_none_or(|p| plat::write_file(p, b"ready"))
    };
    if let Some(path) = hold {
        let Some(_lock) = plat::FileLock::acquire(path) else {
            return Some(73);
        };
        if !ready() {
            return Some(2);
        }
        plat::sleep_ms(delay);
        return Some(0);
    }
    let result = (|| {
        let mut store = doc_io::LibraryStore::open(save.unwrap())?;
        let input = args
            .value("--library-input")
            .ok_or(doc_io::LibraryStoreError::Io)?;
        // LibraryStore::open reads with the same bounded native decoder as production.
        if plat::file_exists_checked(input) != Some(1) {
            return Err(doc_io::LibraryStoreError::Io);
        }
        let incoming = doc_io::LibraryStore::open(input)?;
        if !ready() {
            return Err(doc_io::LibraryStoreError::Io);
        }
        plat::sleep_ms(delay);
        store.save(incoming.library())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            plat::write_stdout(format!("LIBRARY {error:?}\n").as_bytes());
            match error {
                doc_io::LibraryStoreError::Busy => 73,
                doc_io::LibraryStoreError::Conflict => 74,
                _ => 2,
            }
        }
    })
}
