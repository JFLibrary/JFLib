// apple_spell -- (spell.rs) macOS: the system's spell checker, NSSpellChecker (its languages, the
// words the user taught it)
//
//   languages() -> tags      availableLanguages as BCP 47 tags ("en", "en_GB" -> "en", "en-GB")
//   find(lang) -> tag        the tag exactly, else the bare language, else any region of it
//   Checker::open(tag)       remembers the checker's own name of that language
//   check(word) / suggest(word, max)   checkSpellingOfString:... / guessesForWordRange:...
//
// The shared checker is AppKit's: it stays on the thread that uses it (the UI thread; SysSpeller
// is not Send).  Every call runs in an autorelease pool of its own.

use alloc::string::String;
use alloc::vec::Vec;

use crate::apple::objc::{Id, NIL, NOT_FOUND, NSRange, class, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release,
                         string_of};
use crate::locale_tag;

fn checker() -> Id {
    // SAFETY: the shared spell checker (a shared object).
    unsafe { msg!(class(c"NSSpellChecker"), c"sharedSpellChecker"; Id) }
}

/// the checker's own names of its languages ("en", "en_GB", "de")
fn names() -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: a pool of ours; availableLanguages is an array of NSStrings.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let sc = checker();
        let arr: Id = if sc.is_null() { NIL } else { msg!(sc, c"availableLanguages"; Id) };
        let n: usize = if arr.is_null() { 0 } else { msg!(arr, c"count"; usize) };
        for k in 0..n.min(256) {
            out.push(string_of(msg!(arr, c"objectAtIndex:", k => usize; Id)));
        }
        objc_autoreleasePoolPop(pool);
    }
    out
}

pub(super) fn languages() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in names().iter().filter_map(|n| locale_tag(n)) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

pub(super) fn find(lang: &str) -> Option<String> {
    let want = locale_tag(lang)?;
    let base = want.split('-').next().unwrap_or("").to_ascii_lowercase();
    let tags = languages();
    tags.iter()
        .find(|t| t.eq_ignore_ascii_case(&want))
        .or_else(|| tags.iter().find(|t| t.eq_ignore_ascii_case(&base)))
        .or_else(|| tags.iter().find(|t| t.to_ascii_lowercase().starts_with(&alloc::format!("{base}-"))))
        .cloned()
}

/// a language of the checker (its own name of it)
pub(super) struct Checker {
    name: String,
}

impl Checker {
    pub(super) fn open(tag: &str) -> Option<Checker> {
        names().into_iter().find(|n| locale_tag(n).is_some_and(|t| t.eq_ignore_ascii_case(tag))).map(|name| Checker { name })
    }

    /// the word is correct (no checker: true)
    pub(super) fn check(&self, word: &str) -> bool {
        let sc = checker();
        if sc.is_null() || word.is_empty() {
            return true;
        }
        let (w, l) = (nsstring(word), nsstring(&self.name));
        // SAFETY: the checker's method with two strings of ours (released after) and no word count.
        let r: NSRange = unsafe {
            msg!(sc, c"checkSpellingOfString:startingAt:language:wrap:inSpellDocumentWithTag:wordCount:", w => Id, 0isize => isize, l => Id,
                 false => bool, 0isize => isize, core::ptr::null_mut::<isize>() => *mut isize; NSRange)
        };
        release(w);
        release(l);
        r.location == NOT_FOUND || r.length == 0
    }

    /// corrections for the word, at most max
    pub(super) fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let sc = checker();
        let mut out = Vec::new();
        if sc.is_null() || word.is_empty() || max == 0 {
            return out;
        }
        let (w, l) = (nsstring(word), nsstring(&self.name));
        let range = NSRange { location: 0, length: word.encode_utf16().count() };
        // SAFETY: a pool of ours; the checker's method with strings of ours; the guesses are copied.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let arr: Id = msg!(sc, c"guessesForWordRange:inString:language:inSpellDocumentWithTag:", range => NSRange, w => Id, l => Id,
                               0isize => isize; Id);
            let n: usize = if arr.is_null() { 0 } else { msg!(arr, c"count"; usize) };
            for k in 0..n.min(max) {
                out.push(string_of(msg!(arr, c"objectAtIndex:", k => usize; Id)));
            }
            objc_autoreleasePoolPop(pool);
        }
        release(w);
        release(l);
        out
    }
}
