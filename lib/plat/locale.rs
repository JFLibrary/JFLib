// locale -- the user's languages and regional format as BCP 47 tags ("de-DE", "en-US")
//
//   system_languages() -> Vec<String>   the UI languages in order of preference (empty: unknown)
//   system_locale() -> Option<String>   the regional format of numbers and dates (None: unknown)
//   locale_tag(s) -> Option<String>     "de_DE.UTF-8@euro" / "de-de" -> "de-DE"; "C", "POSIX"
//                                       and empty -> None
//
// Windows: GetUserPreferredUILanguages and GetUserDefaultLocaleName.  Linux, the browser and
// Android: the environment -- LANGUAGE (a ':' list, GNU), LC_ALL, LC_MESSAGES, LANG for the
// languages; LC_ALL, LC_TIME, LC_NUMERIC, LANG for the format.  The browser host sets LANGUAGE
// from navigator.languages, the Android host from the system's locale list.  macOS and iOS:
// CoreFoundation's preferred languages and current locale; LANGUAGE / LC_ALL / LC_MESSAGES and
// LC_ALL / LC_TIME / LC_NUMERIC set by hand win (LANG alone does not).

use alloc::string::String;
use alloc::vec::Vec;

/// a POSIX locale name or a BCP 47 tag as a BCP 47 tag: language lower case, a script title
/// case, a region upper case ("zh_hant_tw" -> "zh-Hant-TW"); None for "C" / "POSIX" / empty
pub fn locale_tag(s: &str) -> Option<String> {
    let s = s.split(['.', '@']).next().unwrap_or("").trim();
    if s.is_empty() || s == "C" || s == "POSIX" {
        return None;
    }
    let mut out = String::new();
    for (k, part) in s.split(['_', '-']).enumerate() {
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        if k > 0 {
            out.push('-');
        }
        match (k, part.len()) {
            (0, _) => out.push_str(&part.to_ascii_lowercase()),
            (_, 4) if part.bytes().all(|b| b.is_ascii_alphabetic()) => {
                out.push_str(&part[..1].to_ascii_uppercase());
                out.push_str(&part[1..].to_ascii_lowercase());
            }
            (_, 2) | (_, 3) => out.push_str(&part.to_ascii_uppercase()),
            _ => out.push_str(&part.to_ascii_lowercase()),
        }
    }
    let lang = out.split('-').next().unwrap_or("");
    if !(2..=3).contains(&lang.len()) || !lang.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    Some(out)
}

// the first of these variables that is set and names a locale
#[cfg(not(windows))]
fn env_tag(names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| crate::env(n).filter(|v| !v.is_empty())).and_then(|v| locale_tag(&v))
}

/// the UI languages in order of preference (empty when the system names none)
#[cfg(not(any(windows, target_vendor = "apple")))]
pub fn system_languages() -> Vec<String> {
    env_languages()
}

/// the regional format (numbers, dates, currency) of the user
#[cfg(not(any(windows, target_vendor = "apple")))]
pub fn system_locale() -> Option<String> {
    env_tag(&["LC_ALL", "LC_TIME", "LC_NUMERIC", "LANG"]).or_else(|| system_languages().into_iter().next())
}

/// Apple: the system's preferred languages (CFLocaleCopyPreferredLanguages); LANGUAGE, LC_ALL or
/// LC_MESSAGES set by hand win (LANG alone does not: Terminal sets it from the region)
#[cfg(target_vendor = "apple")]
pub fn system_languages() -> Vec<String> {
    if ["LANGUAGE", "LC_ALL", "LC_MESSAGES"].iter().any(|n| crate::env(n).is_some_and(|v| !v.is_empty())) {
        let v = env_languages();
        if !v.is_empty() {
            return v;
        }
    }
    let v = dedup(cf::preferred_languages().iter().filter_map(|s| locale_tag(s)).collect());
    if v.is_empty() { env_languages() } else { v }
}

/// Apple: LC_ALL, LC_TIME or LC_NUMERIC set by hand, else the system's current locale
/// (CFLocaleCopyCurrent: "de_DE", "en_US@rg=dezzzz" -> "en-US"), else LANG, else the first language
#[cfg(target_vendor = "apple")]
pub fn system_locale() -> Option<String> {
    env_tag(&["LC_ALL", "LC_TIME", "LC_NUMERIC"])
        .or_else(|| cf::current_locale().and_then(|s| locale_tag(&s)))
        .or_else(|| env_tag(&["LANG"]))
        .or_else(|| system_languages().into_iter().next())
}

/// the languages of LANGUAGE, LC_ALL, LC_MESSAGES and LANG (GNU gettext's rules)
#[cfg(not(windows))]
fn env_languages() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|n| crate::env(n).filter(|v| !v.is_empty()));
    // LANGUAGE does not count under the "C" locale (as with GNU gettext)
    if !raw.as_deref().is_some_and(|v| locale_tag(v).is_none()) {
        if let Some(list) = crate::env("LANGUAGE") {
            out.extend(list.split([':', ',']).filter_map(locale_tag));
        }
    }
    out.extend(raw.and_then(|v| locale_tag(&v)));
    dedup(out)
}

// Apple: CoreFoundation's locale (also without an application object)
#[cfg(target_vendor = "apple")]
mod cf {
    use alloc::string::String;
    use alloc::vec::Vec;

    type CfRef = *const core::ffi::c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> CfRef;
        fn CFLocaleCopyCurrent() -> CfRef;
        fn CFLocaleGetIdentifier(l: CfRef) -> CfRef;
        fn CFArrayGetCount(a: CfRef) -> isize;
        fn CFArrayGetValueAtIndex(a: CfRef, i: isize) -> CfRef;
        fn CFStringGetCString(s: CfRef, buf: *mut u8, n: isize, encoding: u32) -> u8;
        fn CFRelease(r: CfRef);
    }

    fn string(s: CfRef) -> Option<String> {
        if s.is_null() {
            return None;
        }
        let mut buf = [0u8; 128];
        // SAFETY: a CFString and a buffer of 128 bytes (UTF-8, NUL-terminated on success).
        if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100) } == 0 {
            return None;
        }
        let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
        core::str::from_utf8(&buf[..n]).ok().map(String::from)
    }

    /// "de-DE", "en-US", ... in the user's order
    pub fn preferred_languages() -> Vec<String> {
        let mut out = Vec::new();
        // SAFETY: a copied array of CFStrings, released after.
        unsafe {
            let a = CFLocaleCopyPreferredLanguages();
            if a.is_null() {
                return out;
            }
            for k in 0..CFArrayGetCount(a).min(32) {
                out.extend(string(CFArrayGetValueAtIndex(a, k)));
            }
            CFRelease(a);
        }
        out
    }

    /// the current locale's identifier ("de_DE")
    pub fn current_locale() -> Option<String> {
        // SAFETY: a copied locale (released after); its identifier belongs to it.
        unsafe {
            let l = CFLocaleCopyCurrent();
            if l.is_null() {
                return None;
            }
            let id = string(CFLocaleGetIdentifier(l));
            CFRelease(l);
            id
        }
    }
}

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in v {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

#[cfg(windows)]
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetUserPreferredUILanguages(flags: u32, num: *mut u32, buf: *mut u16, len: *mut u32) -> i32;
    fn GetUserDefaultLocaleName(buf: *mut u16, len: i32) -> i32;
}

/// the UI languages in order of preference (empty when the system names none)
#[cfg(windows)]
pub fn system_languages() -> Vec<String> {
    const MUI_LANGUAGE_NAME: u32 = 0x8;
    let mut buf = [0u16; 512];
    let (mut num, mut len) = (0u32, buf.len() as u32);
    // SAFETY: the buffer holds len UTF-16 units; num and len are written by the call.
    let ok = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut num, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return Vec::new();
    }
    let used = &buf[..(len as usize).min(buf.len())];
    dedup(used.split(|&c| c == 0).filter(|s| !s.is_empty()).filter_map(|s| locale_tag(&String::from_utf16_lossy(s))).collect())
}

/// the regional format (numbers, dates, currency) of the user
#[cfg(windows)]
pub fn system_locale() -> Option<String> {
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the buffer holds 85 UTF-16 units.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return system_languages().into_iter().next();
    }
    locale_tag(&String::from_utf16_lossy(&buf[..n as usize - 1]))
}
