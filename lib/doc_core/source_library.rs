// Reusable source-library transactions and identity allocation; new native functionality.
// Storage belongs to doc_io: this model never accesses a filesystem or a document.
use crate::{Error, Id, Source};
use alloc::{collections::BTreeSet, vec::Vec};

const ID_END: Id = 0x7fff_ffff;
const MAX_SOURCES: usize = 1024;
const MAX_BYTES: usize = 2 << 20;
const HISTORY_COUNT: usize = 64;
const HISTORY_BYTES: usize = 8 << 20;

/// Independent source collection with a persisted, monotonically increasing ID watermark.
/// Public for codecs; validate at trust boundaries. IDs have no meaning in other containers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLibrary {
    /// Ordered source records, each with an identity local to this library.
    pub sources: Vec<Source>,
    /// First never-issued ID, or 0x7fff_ffff when the identity space is exhausted.
    pub next_id: Id,
}
impl Default for SourceLibrary {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            next_id: 1,
        }
    }
}
impl SourceLibrary {
    /// Find a source by its library-local identity.
    pub fn source(&self, id: Id) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }
    /// Estimated retained payload, including source, creator and string storage.
    pub fn byte_size(&self) -> usize {
        self.sources
            .iter()
            .fold(core::mem::size_of::<Self>(), |size, source| {
                size.saturating_add(source.byte_size())
            })
    }
    /// Check metadata, unique local IDs, allocation watermark and collection budgets.
    pub fn validate(&self) -> Result<(), Error> {
        if self.sources.len() > MAX_SOURCES || self.byte_size() > MAX_BYTES {
            return Err(Error::Limit);
        }
        if self.next_id == 0 || self.next_id > ID_END {
            return Err(Error::Invalid);
        }
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            if source.id == 0 || source.id >= self.next_id || !ids.insert(source.id) {
                return Err(Error::Invalid);
            }
            source.validate()?;
        }
        Ok(())
    }
    fn allocate(&mut self) -> Result<Id, Error> {
        if self.next_id >= ID_END {
            return Err(Error::Limit);
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(id)
    }
}

/// Atomic source-library operations; imported identities are always external to the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryEdit {
    /// Import records, ignoring their IDs and skipping exact metadata duplicates.
    /// Returns the final record's existing or new library ID, or zero for an empty import.
    Import(Vec<Source>),
    /// ID zero explicitly creates a source; a nonzero ID updates an existing source.
    /// Unlike import, explicit creation preserves intentionally identical records.
    Put(Source),
    /// Remove an existing library source without affecting copies in documents.
    Remove(Id),
}

/// Atomic editor with at most 64 retained history snapshots and an 8 MiB history budget.
/// Undo and redo retain the allocation watermark, including when a library is saved/reloaded.
#[derive(Clone, Debug)]
pub struct SourceLibraryEditor {
    library: SourceLibrary,
    undo: Vec<SourceLibrary>,
    redo: Vec<SourceLibrary>,
    revision: u64,
}
impl SourceLibraryEditor {
    fn trim_history(&mut self) {
        while self.undo.len() + self.redo.len() > HISTORY_COUNT
            || self
                .undo
                .iter()
                .chain(&self.redo)
                .map(SourceLibrary::byte_size)
                .sum::<usize>()
                > HISTORY_BYTES
        {
            // Old undo states are least relevant; with none left, discard the furthest redo.
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else {
                self.redo.remove(0);
            }
        }
    }
    /// Start an editing session from a validated library, without inherited history.
    pub fn new(library: SourceLibrary) -> Result<Self, Error> {
        library.validate()?;
        Ok(Self {
            library,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
        })
    }
    /// Current immutable collection, ready for a library codec or copying to a document.
    pub fn library(&self) -> &SourceLibrary {
        &self.library
    }
    /// Changes for every successful content edit, undo or redo; unchanged by no-ops/errors.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Whether a complete transaction can be undone.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    /// Whether an undone transaction can be reapplied.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Undo one transaction without making any previously issued identity reusable.
    pub fn undo(&mut self) -> bool {
        let Some(mut prior) = self.undo.pop() else {
            return false;
        };
        prior.next_id = self.library.next_id;
        self.redo.push(core::mem::replace(&mut self.library, prior));
        self.trim_history();
        self.revision = self.revision.wrapping_add(1);
        true
    }
    /// Redo one transaction, preserving the same source identities and allocation watermark.
    pub fn redo(&mut self) -> bool {
        let Some(mut next) = self.redo.pop() else {
            return false;
        };
        next.next_id = self.library.next_id;
        self.undo.push(core::mem::replace(&mut self.library, next));
        self.trim_history();
        self.revision = self.revision.wrapping_add(1);
        true
    }
    /// Validate and apply a whole transaction. Errors and no-ops preserve history/revision.
    pub fn apply(&mut self, edit: LibraryEdit) -> Result<Id, Error> {
        // Bound the input even when all records would be deduplicated on import.
        match &edit {
            LibraryEdit::Import(sources) => {
                if sources.len() > MAX_SOURCES
                    || sources.iter().fold(0usize, |size, source| {
                        size.saturating_add(source.byte_size())
                    }) > MAX_BYTES
                {
                    return Err(Error::Limit);
                }
                for source in sources {
                    source.validate()?;
                }
            }
            LibraryEdit::Put(source) => source.validate()?,
            LibraryEdit::Remove(_) => (),
        }
        let mut next = self.library.clone();
        let id = match edit {
            LibraryEdit::Import(sources) => {
                let mut affected = 0;
                for mut source in sources {
                    if let Some(existing) =
                        next.sources.iter().find(|item| item.same_content(&source))
                    {
                        affected = existing.id;
                    } else {
                        if next.sources.len() >= MAX_SOURCES {
                            return Err(Error::Limit);
                        }
                        source.id = next.allocate()?;
                        affected = source.id;
                        next.sources.push(source);
                        if next.byte_size() > MAX_BYTES {
                            return Err(Error::Limit);
                        }
                    }
                }
                affected
            }
            LibraryEdit::Put(mut source) => {
                if source.id == 0 {
                    if next.sources.len() >= MAX_SOURCES {
                        return Err(Error::Limit);
                    }
                    source.id = next.allocate()?;
                    let id = source.id;
                    next.sources.push(source);
                    id
                } else {
                    let target = next
                        .sources
                        .iter_mut()
                        .find(|item| item.id == source.id)
                        .ok_or(Error::Missing)?;
                    let id = source.id;
                    *target = source;
                    id
                }
            }
            LibraryEdit::Remove(id) => {
                let at = next
                    .sources
                    .iter()
                    .position(|source| source.id == id)
                    .ok_or(Error::Missing)?;
                next.sources.remove(at);
                id
            }
        };
        next.validate()?;
        if next != self.library {
            self.undo.push(core::mem::replace(&mut self.library, next));
            self.redo.clear();
            self.trim_history();
            self.revision = self.revision.wrapping_add(1);
        }
        Ok(id)
    }
}
