// apple_spell -- (spell.rs) the system's spell checker: macOS NSSpellChecker (its languages, the
// words the user taught it), iOS UITextChecker (its languages, the keyboard's learned words)
//
//   languages() -> tags      availableLanguages as BCP 47 tags ("en", "en_GB" -> "en", "en-GB")
//   find(lang) -> tag        the tag exactly, else the bare language, else any region of it
//   Checker::open(tag)       remembers the checker's own name of that language (iOS: a UITextChecker
//                            of its own)
//   check(word) / suggest(word, max)   macOS checkSpellingOfString:... / guessesForWordRange:...,
//                            iOS rangeOfMisspelledWordInString:... / guessesForWordRange:...
//
// The checkers are UIKit's / AppKit's: they stay on the thread that uses them (the UI thread;
// SysSpeller is not Send).  Every call runs in an autorelease pool of its own.

use alloc::string::String;
use alloc::vec::Vec;

use crate::apple::objc::{Id, NIL, NOT_FOUND, NSRange, class, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release,
                         string_of};
use crate::locale_tag;

#[cfg(target_os = "macos")]
fn checker() -> Id {
    // SAFETY: the shared spell checker (a shared object).
    unsafe { msg!(class(c"NSSpellChecker"), c"sharedSpellChecker"; Id) }
}

/// the object whose availableLanguages count: the shared checker / UITextChecker's class
fn language_source() -> Id {
    #[cfg(target_os = "macos")]
    return checker();
    #[cfg(target_os = "ios")]
    class(c"UITextChecker")
}

/// the checker's own names of its languages ("en", "en_GB", "de")
fn names() -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: a pool of ours; availableLanguages is an array of NSStrings.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let sc = language_source();
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

/// a language of the checker (its own name of it; iOS: a UITextChecker of its own, +1)
pub(super) struct Checker {
    name: String,
    #[cfg(target_os = "ios")]
    tc: Id,
}

#[cfg(target_os = "ios")]
impl Drop for Checker {
    fn drop(&mut self) {
        release(self.tc);
    }
}

impl Checker {
    pub(super) fn open(tag: &str) -> Option<Checker> {
        let name = names().into_iter().find(|n| locale_tag(n).is_some_and(|t| t.eq_ignore_ascii_case(tag)))?;
        #[cfg(target_os = "macos")]
        return Some(Checker { name });
        #[cfg(target_os = "ios")]
        {
            // SAFETY: a UITextChecker of ours (+1, released by Drop).
            let tc: Id = unsafe { msg!(msg!(class(c"UITextChecker"), c"alloc"; Id), c"init"; Id) };
            (!tc.is_null()).then_some(Checker { name, tc })
        }
    }

    /// the checker the calls go to
    fn checker(&self) -> Id {
        #[cfg(target_os = "macos")]
        return checker();
        #[cfg(target_os = "ios")]
        self.tc
    }

    /// the word is correct (no checker: true)
    #[cfg(target_os = "ios")]
    pub(super) fn check(&self, word: &str) -> bool {
        let tc = self.checker();
        if tc.is_null() || word.is_empty() {
            return true;
        }
        let (w, l) = (nsstring(word), nsstring(&self.name));
        let range = NSRange { location: 0, length: word.encode_utf16().count() };
        // SAFETY: the checker's method with two strings of ours (released after).
        let r: NSRange = unsafe {
            msg!(tc, c"rangeOfMisspelledWordInString:range:startingAt:wrap:language:", w => Id, range => NSRange, 0isize => isize,
                 false => bool, l => Id; NSRange)
        };
        release(w);
        release(l);
        r.location == NOT_FOUND || r.length == 0
    }

    /// the word is correct (no checker: true)
    #[cfg(target_os = "macos")]
    pub(super) fn check(&self, word: &str) -> bool {
        let sc = self.checker();
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
        let sc = self.checker();
        let mut out = Vec::new();
        if sc.is_null() || word.is_empty() || max == 0 {
            return out;
        }
        let (w, l) = (nsstring(word), nsstring(&self.name));
        let range = NSRange { location: 0, length: word.encode_utf16().count() };
        // SAFETY: a pool of ours; the checker's method with strings of ours; the guesses are copied.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            #[cfg(target_os = "macos")]
            let arr: Id = msg!(sc, c"guessesForWordRange:inString:language:inSpellDocumentWithTag:", range => NSRange, w => Id, l => Id,
                               0isize => isize; Id);
            #[cfg(target_os = "ios")]
            let arr: Id = msg!(sc, c"guessesForWordRange:inString:language:", range => NSRange, w => Id, l => Id; Id);
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
