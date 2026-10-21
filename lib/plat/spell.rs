// spell -- the system's spell checker
//
//   sys_spell_find(lang) -> Some(tag)   the checker of the system knows the language: the tag
//                                    as it names it ("de-DE"); `lang` exactly, else the same
//                                    language in its main region, else any region of it
//   sys_spell_languages() -> tags    every language it knows
//   SysSpeller::open(tag) -> checker   language(), check(word), suggest(word, max)
//   hunspell_dirs() -> folders       where Hunspell dictionaries live here (DICPATH, Linux:
//                                    the XDG data folders); the spell crate reads them
//
// Windows (win_spell.rs): ISpellCheckerFactory / ISpellChecker of the Spell Checking API
// (Windows 8 and later; the languages are the installed language packs).  COM on the calling
// thread (an apartment of its own on the UI thread): a SysSpeller stays on the thread that
// opened it (it is not Send).  check() reports a word as misspelled when ISpellChecker::Check
// returns any error for it (also "replace" / "delete" actions); the user's Windows word list
// counts as known.  Linux, Web, Android: no system checker (None, empty) -- their Hunspell
// dictionaries are files the spell crate reads itself.

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

#[cfg(windows)]
#[path = "win_spell.rs"]
mod imp;

/// a checker of the system's spell checking service for one language
pub struct SysSpeller {
    #[cfg(windows)]
    inner: imp::Checker,
    tag: String,
    _thread: PhantomData<*const ()>, // stays on its thread (COM apartment)
}

/// the folders of Hunspell dictionaries on this system, in search order: DICPATH (';'
/// separated on Windows, ':' elsewhere), then on Linux $XDG_DATA_HOME/hunspell
/// (~/.local/share/hunspell) and hunspell, myspell, myspell/dicts below every XDG_DATA_DIRS
/// folder (default /usr/local/share, /usr/share); Web / Android: none.  Always readable for
/// the app's permissions (policy.rs), like the font folders.
pub fn hunspell_dirs() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(any(target_arch = "wasm32", target_os = "android")) {
        return v;
    }
    let sep = if cfg!(windows) { ';' } else { ':' };
    if let Some(p) = crate::env("DICPATH") {
        v.extend(p.split(sep).filter(|d| !d.is_empty()).map(String::from));
    }
    if cfg!(target_os = "linux") {
        match crate::env("XDG_DATA_HOME").filter(|h| !h.is_empty()) {
            Some(h) => v.push(alloc::format!("{}/hunspell", h.trim_end_matches('/'))),
            None => v.extend(crate::env("HOME").map(|h| alloc::format!("{}/.local/share/hunspell", h.trim_end_matches('/')))),
        }
        let data = crate::env("XDG_DATA_DIRS").filter(|d| !d.is_empty()).unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
        for d in data.split(':').filter(|d| !d.is_empty()) {
            let d = d.trim_end_matches('/');
            v.push(alloc::format!("{}/hunspell", d));
            v.push(alloc::format!("{}/myspell", d));
            v.push(alloc::format!("{}/myspell/dicts", d));
        }
    }
    v
}

/// the tag of a language the system's checker knows (see the module header)
pub fn sys_spell_find(lang: &str) -> Option<String> {
    #[cfg(windows)]
    {
        imp::find(lang)
    }
    #[cfg(not(windows))]
    {
        let _ = lang;
        None
    }
}

/// every language the system's checker knows
pub fn sys_spell_languages() -> Vec<String> {
    #[cfg(windows)]
    {
        imp::languages()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

impl SysSpeller {
    /// the checker for `tag` (a tag sys_spell_find gave), None when the system has none
    pub fn open(tag: &str) -> Option<SysSpeller> {
        #[cfg(windows)]
        {
            let inner = imp::Checker::open(tag)?;
            Some(SysSpeller { inner, tag: String::from(tag), _thread: PhantomData })
        }
        #[cfg(not(windows))]
        {
            let _ = tag;
            None
        }
    }

    /// the language tag
    pub fn language(&self) -> &str {
        &self.tag
    }

    /// the word is correct (an error of the service: true)
    pub fn check(&self, word: &str) -> bool {
        #[cfg(windows)]
        {
            self.inner.check(word)
        }
        #[cfg(not(windows))]
        {
            let _ = word;
            true
        }
    }

    /// corrections for the word (at most max)
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        #[cfg(windows)]
        {
            self.inner.suggest(word, max)
        }
        #[cfg(not(windows))]
        {
            let _ = (word, max);
            Vec::new()
        }
    }
}
