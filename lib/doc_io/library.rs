// Native source-library persistence: strict XML, optimistic conflict detection and recovery.
// New shared functionality; no user-profile policy or application UI belongs in this layer.
use alloc::{format, string::String, vec::Vec};
use doc_core::SourceLibrary;

use crate::{bibliography, encode_sources, package::DECL, parse, read_file, unsupported, Error};

const NS: &str = "urn:jfword:source-library:1";
const MAX_FILE: usize = 4 << 20;

/// Encode the native version-one source library, preserving source IDs and their high-water mark.
pub fn encode_source_library(library: &SourceLibrary) -> Result<Vec<u8>, Error> {
    library.validate()?;
    let sources = encode_sources(&library.sources)?;
    let sources = core::str::from_utf8(&sources).map_err(|_| Error::Invalid)?;
    let sources = sources.strip_prefix(DECL).ok_or(Error::Invalid)?;
    let bytes = format!(
        "{DECL}<j:SourceLibrary xmlns:j=\"{NS}\" version=\"1\" next-id=\"{}\">{sources}</j:SourceLibrary>",
        library.next_id
    )
    .into_bytes();
    // The extra wrapper also consumes XML depth/node budget; never emit an unreadable library.
    if decode_source_library(&bytes)? != *library {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}

/// Decode the complete native source-library format; future versions and unknown data fail closed.
pub fn decode_source_library(bytes: &[u8]) -> Result<SourceLibrary, Error> {
    if bytes.len() > MAX_FILE {
        return Err(Error::Limit);
    }
    let root = parse(bytes)?;
    if !root.is(NS, "SourceLibrary")
        || !root.text.is_empty()
        || root.children.len() != 1
        || root.attrs.len() != 2
    {
        return Err(unsupported("SourceLibrary"));
    }
    for (name, _) in &root.attrs {
        if !name.ns.is_empty() || !matches!(name.local.as_str(), "version" | "next-id") {
            return Err(unsupported(&name.local));
        }
    }
    if root.attr("", "version") != Some("1") {
        return Err(unsupported("SourceLibrary version"));
    }
    let next = root.attr("", "next-id").ok_or(Error::Invalid)?;
    if next.is_empty() || !next.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid);
    }
    let sources = &root.children[0];
    // Word's standalone exchange permits document-style hints and RefOrder. A native library
    // has no such unrepresented data: rejecting it avoids silently discarding future metadata.
    if !sources.attrs.is_empty()
        || sources.children.iter().any(|source| {
            source
                .children
                .iter()
                .any(|field| field.name.local == "RefOrder")
        })
    {
        return Err(unsupported("SourceLibrary bibliography metadata"));
    }
    let library = SourceLibrary {
        sources: bibliography::read_sources(sources, true)?,
        next_id: next.parse().map_err(|_| Error::Invalid)?,
    };
    library.validate()?;
    Ok(library)
}

/// Persistent-library failures distinguish stale content, a held lock and codec rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryStoreError {
    /// A path could not be inspected, read, backed up or atomically replaced.
    Io,
    /// The cooperative lock could not be acquired (busy or unavailable).
    Busy,
    /// The file has changed since this store's successful open, reload or save.
    Conflict,
    /// The current file, backup or proposed model is outside the supported native format.
    Codec(Error),
}
impl From<Error> for LibraryStoreError {
    fn from(error: Error) -> Self {
        match error {
            Error::Io => Self::Io,
            other => Self::Codec(other),
        }
    }
}

/// A validated source library and the exact file snapshot used for optimistic concurrency.
/// Opening and reloading never write. Saving uses a sibling lock and atomic platform writes.
pub struct LibraryStore {
    path: String,
    library: SourceLibrary,
    snapshot: Option<Vec<u8>>,
}

/// A recovery preview with exact current-file and backup snapshots, safe to keep across a dialog.
/// Restoring fails if either reviewed file changes before the caller confirms the operation.
#[derive(Clone)]
pub struct LibraryRecovery {
    path: String,
    current: Option<Vec<u8>>,
    backup: Vec<u8>,
    library: SourceLibrary,
}
impl LibraryRecovery {
    /// The planned restored model, retaining the greater ID high-water mark of valid current XML.
    pub fn library(&self) -> &SourceLibrary {
        &self.library
    }

    /// Restore the reviewed backup under the library lock, preserving replaced bytes in `.damaged`.
    pub fn restore(&self) -> Result<LibraryStore, LibraryStoreError> {
        LibraryStore::restore_snapshot(&self.path, &self.current, &self.backup, &self.library)
    }
}

fn checked_path(path: &str) -> Result<(), LibraryStoreError> {
    if path.is_empty() || path.contains('\0') || path.ends_with('/') || path.ends_with('\\') {
        return Err(LibraryStoreError::Codec(Error::Invalid));
    }
    Ok(())
}

fn snapshot(path: &str) -> Result<Option<Vec<u8>>, LibraryStoreError> {
    match plat::file_exists_checked(path).ok_or(LibraryStoreError::Io)? {
        0 => Ok(None),
        1 => Ok(Some(read_file(path, MAX_FILE)?)),
        _ => Err(LibraryStoreError::Io),
    }
}

fn existing_snapshot(path: &str) -> Result<Vec<u8>, LibraryStoreError> {
    snapshot(path)?.ok_or(LibraryStoreError::Io)
}

fn prepare_parent(path: &str) -> Result<(), LibraryStoreError> {
    if let Some(at) = path.rfind(['/', '\\']) {
        // A filesystem root already exists; drive-root paths keep their separator.
        let end = if at == 0 || path.as_bytes().get(at.wrapping_sub(1)) == Some(&b':') {
            at + 1
        } else {
            at
        };
        if !plat::mkdir(&path[..end]) {
            return Err(LibraryStoreError::Io);
        }
    }
    Ok(())
}

fn write(path: &str, bytes: &[u8]) -> Result<(), LibraryStoreError> {
    if plat::write_file(path, bytes) {
        Ok(())
    } else {
        Err(LibraryStoreError::Io)
    }
}

fn preserve_high_water(library: &mut SourceLibrary, current: &Option<Vec<u8>>) {
    if let Some(bytes) = current {
        if let Ok(current) = decode_source_library(bytes) {
            library.next_id = library.next_id.max(current.next_id);
        }
    }
}

impl LibraryStore {
    /// Open an existing valid library, or an empty in-memory library when the path is absent.
    /// A malformed existing file is an error and is never replaced by an empty model.
    pub fn open(path: &str) -> Result<Self, LibraryStoreError> {
        checked_path(path)?;
        let snapshot = snapshot(path)?;
        let library = match &snapshot {
            Some(bytes) => decode_source_library(bytes)?,
            None => SourceLibrary::default(),
        };
        Ok(Self {
            path: String::from(path),
            library,
            snapshot,
        })
    }

    /// The last successfully opened, reloaded or saved library.
    pub fn library(&self) -> &SourceLibrary {
        &self.library
    }

    /// The configured native library filename.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Validate fully, acquire the sibling lock, compare the original bytes, then save atomically.
    /// An existing valid version is backed up to `.bak` before replacement; backup failure aborts.
    pub fn save(&mut self, library: &SourceLibrary) -> Result<(), LibraryStoreError> {
        let bytes = encode_source_library(library)?;
        if library.next_id < self.library.next_id {
            return Err(LibraryStoreError::Codec(Error::Model(
                doc_core::Error::Invalid,
            )));
        }
        prepare_parent(&self.path)?;
        let _lock = plat::FileLock::acquire(&format!("{}.lock", self.path))
            .ok_or(LibraryStoreError::Busy)?;
        if snapshot(&self.path)? != self.snapshot {
            return Err(LibraryStoreError::Conflict);
        }
        if self.snapshot.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        if let Some(previous) = &self.snapshot {
            write(&format!("{}.bak", self.path), previous)?;
        }
        write(&self.path, &bytes)?;
        self.library = library.clone();
        self.snapshot = Some(bytes);
        Ok(())
    }

    /// Read and validate the current file; on any failure this store keeps its previous state.
    pub fn reload(&mut self) -> Result<(), LibraryStoreError> {
        let fresh = Self::open(&self.path)?;
        self.library = fresh.library;
        self.snapshot = fresh.snapshot;
        Ok(())
    }

    /// Restore the validated `.bak` only if the current file still matches this store's snapshot.
    /// The replaced bytes are retained in `.damaged`; the good `.bak` is never overwritten.
    pub fn restore_backup(&mut self) -> Result<(), LibraryStoreError> {
        let restored = Self::restore(&self.path, &self.snapshot, None)?;
        self.library = restored.library;
        self.snapshot = restored.snapshot;
        Ok(())
    }

    /// Read a backup for review without changing the current file or creating any paths.
    pub fn inspect_backup(path: &str) -> Result<SourceLibrary, LibraryStoreError> {
        checked_path(path)?;
        Ok(decode_source_library(&existing_snapshot(&format!(
            "{path}.bak"
        ))?)?)
    }

    /// Read a valid backup and current bytes for explicit recovery, including malformed current XML.
    /// This read-only preview can later restore exactly those reviewed snapshots.
    pub fn prepare_recovery(path: &str) -> Result<LibraryRecovery, LibraryStoreError> {
        checked_path(path)?;
        let current = snapshot(path)?;
        let backup = existing_snapshot(&format!("{path}.bak"))?;
        let mut library = decode_source_library(&backup)?;
        preserve_high_water(&mut library, &current);
        Ok(LibraryRecovery {
            path: String::from(path),
            current,
            backup,
            library,
        })
    }

    /// Restore only the backup model the caller previously inspected; changes return `Conflict`.
    pub fn restore_backup_if(&mut self, expected: &SourceLibrary) -> Result<(), LibraryStoreError> {
        let restored = Self::restore(&self.path, &self.snapshot, Some(expected))?;
        self.library = restored.library;
        self.snapshot = restored.snapshot;
        Ok(())
    }

    /// Recover from a malformed current file by restoring a validated `.bak` under the same lock.
    /// Current bytes are retained in `.damaged` before replacement; neither an unreadable nor an
    /// oversized current file is overwritten because an exact comparison cannot be established.
    /// Valid current XML retains its ID high-water mark. A malformed file cannot supply that mark.
    pub fn recover(path: &str) -> Result<Self, LibraryStoreError> {
        checked_path(path)?;
        let current = snapshot(path)?;
        Self::restore(path, &current, None)
    }

    /// Recover only the backup model the caller previously inspected, preventing a stale preview.
    pub fn recover_if_backup(
        path: &str,
        expected: &SourceLibrary,
    ) -> Result<Self, LibraryStoreError> {
        checked_path(path)?;
        let current = snapshot(path)?;
        Self::restore(path, &current, Some(expected))
    }

    fn restore(
        path: &str,
        expected: &Option<Vec<u8>>,
        expected_backup: Option<&SourceLibrary>,
    ) -> Result<Self, LibraryStoreError> {
        let bytes = existing_snapshot(&format!("{path}.bak"))?;
        let mut library = decode_source_library(&bytes)?;
        if expected_backup.is_some_and(|expected| *expected != library) {
            return Err(LibraryStoreError::Conflict);
        }
        preserve_high_water(&mut library, expected);
        Self::restore_snapshot(path, expected, &bytes, &library)
    }

    fn restore_snapshot(
        path: &str,
        expected: &Option<Vec<u8>>,
        bytes: &[u8],
        library: &SourceLibrary,
    ) -> Result<Self, LibraryStoreError> {
        let restored_bytes = encode_source_library(library)?;
        let _lock =
            plat::FileLock::acquire(&format!("{path}.lock")).ok_or(LibraryStoreError::Busy)?;
        if snapshot(path)? != *expected || existing_snapshot(&format!("{path}.bak"))? != bytes {
            return Err(LibraryStoreError::Conflict);
        }
        if expected.as_deref() != Some(restored_bytes.as_slice()) {
            if let Some(previous) = expected {
                write(&format!("{path}.damaged"), previous)?;
            }
            write(path, &restored_bytes)?;
        }
        Ok(Self {
            path: String::from(path),
            library: library.clone(),
            snapshot: Some(restored_bytes),
        })
    }
}
