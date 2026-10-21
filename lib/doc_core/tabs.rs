//! Paragraph tab definitions, shared by editors, layout and document codecs.
use crate::Error;

/// Tab alignment. `Clear` removes an inherited stop at the same position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabAlignment { #[default] Left, Center, Right, Decimal, Bar, Clear }
/// Repeated visual fill between the cursor and a tab target (never document text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabLeader { #[default] None, Dot, Hyphen, Underscore, Heavy, MiddleDot }
impl TabAlignment {
    pub fn name(self) -> &'static str { match self { Self::Left => "left", Self::Center => "center", Self::Right => "right", Self::Decimal => "decimal", Self::Bar => "bar", Self::Clear => "clear" } }
    pub fn from_name(name: &str) -> Option<Self> { Some(match name { "left" => Self::Left, "center" => Self::Center, "right" => Self::Right, "decimal" => Self::Decimal, "bar" => Self::Bar, "clear" => Self::Clear, _ => return None }) }
}
impl TabLeader {
    pub fn name(self) -> &'static str { match self { Self::None => "none", Self::Dot => "dot", Self::Hyphen => "hyphen", Self::Underscore => "underscore", Self::Heavy => "heavy", Self::MiddleDot => "middleDot" } }
    pub fn from_name(name: &str) -> Option<Self> { Some(match name { "none" => Self::None, "dot" => Self::Dot, "hyphen" => Self::Hyphen, "underscore" => Self::Underscore, "heavy" => Self::Heavy, "middleDot" => Self::MiddleDot, _ => return None }) }
}
/// Position in twips from the page's left text margin, independent of paragraph indents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabStop { pub position: i32, pub alignment: TabAlignment, pub leader: TabLeader }
impl TabStop {
    pub fn clear(position: i32) -> Self { Self { position, alignment: TabAlignment::Clear, leader: TabLeader::None } }
    pub fn validate(self) -> Result<(), Error> {
        if !(-31680..=31680).contains(&self.position)
            || (matches!(self.alignment, TabAlignment::Clear | TabAlignment::Bar) && self.leader != TabLeader::None)
        { Err(Error::Invalid) } else { Ok(()) }
    }
}
/// Sorted, unique tab definitions. Bounded storage preserves the lightweight Copy
/// paragraph-format contract; overflowing inheritance is rejected by validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabStops { entries: [TabStop; 64], len: u8, overflow: bool }
impl Default for TabStops {
    fn default() -> Self { Self { entries: [TabStop::default(); 64], len: 0, overflow: false } }
}
impl TabStops {
    pub fn as_slice(&self) -> &[TabStop] { &self.entries[..usize::from(self.len)] }
    pub fn is_empty(&self) -> bool { self.len == 0 && !self.overflow }
    pub fn validate(&self) -> Result<(), Error> { if self.overflow { Err(Error::Limit) } else { Ok(()) } }
    /// Insert or replace a position. On error this collection remains unchanged.
    pub fn set(&mut self, stop: TabStop) -> Result<(), Error> {
        self.validate()?;
        stop.validate()?;
        match self.as_slice().binary_search_by_key(&stop.position, |s| s.position) {
            Ok(i) => self.entries[i] = stop,
            Err(i) => {
                let n = usize::from(self.len);
                if n == self.entries.len() { return Err(Error::Limit); }
                self.entries.copy_within(i..n, i + 1);
                self.entries[i] = stop;
                self.len += 1;
            }
        }
        Ok(())
    }
    /// Remove a definition locally; use `TabStop::clear` to suppress inherited stops.
    pub fn remove(&mut self, position: i32) {
        if let Ok(i) = self.as_slice().binary_search_by_key(&position, |s| s.position) {
            let n = usize::from(self.len);
            self.entries.copy_within(i + 1..n, i);
            self.len -= 1;
            self.entries[n - 1] = TabStop::default();
        }
    }
    /// Declarations that transform inherited effective stops into the desired list.
    /// Desired stops cannot contain clear markers. No input is changed on overflow.
    pub fn relative_to(self, inherited: Self) -> Result<Self, Error> {
        self.validate()?; inherited.validate()?;
        if self.as_slice().iter().any(|s| s.alignment == TabAlignment::Clear) { return Err(Error::Invalid); }
        let mut result = self;
        for s in inherited.as_slice() {
            if s.alignment != TabAlignment::Clear && !self.as_slice().iter().any(|t| t.position == s.position) {
                result.set(TabStop::clear(s.position))?;
            }
        }
        Ok(result)
    }
    /// Merge an override list. Resolution consumes clear markers; patch composition
    /// retains them so a later style change cannot resurrect an explicitly cleared stop.
    pub(crate) fn merge(mut self, patch: Self, resolve: bool) -> Self {
        if self.overflow || patch.overflow { self.overflow = true; return self; }
        // Process clears first, so replacing a full inherited list cannot overflow
        // merely because the removed positions sort after the inserted positions.
        if resolve { for s in patch.as_slice() { if s.alignment == TabAlignment::Clear { self.remove(s.position); } } }
        for s in patch.as_slice() {
            if resolve && s.alignment == TabAlignment::Clear { continue; }
            if self.set(*s).is_err() { self.overflow = true; break; }
        }
        self
    }
}
