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
// from navigator.languages, the Android host from the system's locale list.

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
#[cfg(not(windows))]
pub fn system_languages() -> Vec<String> {
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

/// the regional format (numbers, dates, currency) of the user
#[cfg(not(windows))]
pub fn system_locale() -> Option<String> {
    env_tag(&["LC_ALL", "LC_TIME", "LC_NUMERIC", "LANG"]).or_else(|| system_languages().into_iter().next())
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
#[link(name = "kernel32")]
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
