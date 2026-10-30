// win_spell -- (spell.rs) the Windows Spell Checking API: ISpellCheckerFactory, ISpellChecker
//
//   find(lang)       IsSupported(lang); else the supported tags (get_SupportedLanguages) of
//                    the same language: its main region ("de" -> "de-DE"), else the first
//   languages()      get_SupportedLanguages (IEnumString, CoTaskMemFree for each string)
//   Checker::open    CreateSpellChecker(tag)
//   check(word)      Check(word) -> IEnumSpellingError: a first error = misspelled
//   suggest(word)    Suggest(word) -> IEnumString (the word itself left out; Windows also
//                    offers similar words for a correct one)
//
// COM objects are used through their vtable slots (as win_taskbar.rs); every object made
// here is released, every string the API hands out is freed.

use alloc::string::String;
use alloc::vec::Vec;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: usize, flags: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: usize, ctx: u32, iid: *const Guid, out: *mut usize) -> i32;
    fn CoTaskMemFree(p: *mut u16);
}

/// a COM GUID
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

static CLSID_SPELLCHECKERFACTORY: Guid = Guid(0x7AB3_6653, 0x1796, 0x484B, [0xBD, 0xFA, 0xE7, 0x4F, 0x1D, 0xB7, 0xC1, 0xDC]);
static IID_ISPELLCHECKERFACTORY: Guid = Guid(0x8E01_8A9D, 0x2415, 0x4677, [0xBF, 0x08, 0x79, 0x4E, 0xA6, 0x1F, 0x94, 0xBB]);

const CLSCTX_INPROC_SERVER: u32 = 1;
const COINIT_APARTMENTTHREADED: u32 = 2;
const S_OK: i32 = 0;

// vtable slots
const VT_RELEASE: usize = 2;
const F_SUPPORTED_LANGUAGES: usize = 3; // ISpellCheckerFactory::get_SupportedLanguages
const F_IS_SUPPORTED: usize = 4; // ::IsSupported
const F_CREATE: usize = 5; // ::CreateSpellChecker
const C_CHECK: usize = 4; // ISpellChecker::Check
const C_SUGGEST: usize = 5; // ::Suggest
const E_NEXT: usize = 3; // IEnumString::Next, IEnumSpellingError::Next

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().filter(|&c| c != 0).chain(core::iter::once(0)).collect()
}

/// the function in vtable slot `slot` of COM object `obj`
///
/// SAFETY: obj is a live COM interface pointer whose vtable has more than `slot` entries.
unsafe fn vfn(obj: usize, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

/// release a COM object
///
/// SAFETY: obj is a live COM interface pointer owned by the caller.
unsafe fn release(obj: usize) {
    unsafe {
        let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vfn(obj, VT_RELEASE));
        f(obj);
    }
}

/// COM on this thread (S_FALSE and RPC_E_CHANGED_MODE leave it usable)
fn com_init() {
    // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED); balanced by the process end.
    unsafe { CoInitializeEx(0, COINIT_APARTMENTTHREADED) };
}

/// the factory (released when dropped)
struct Factory(usize);

impl Factory {
    fn new() -> Option<Factory> {
        com_init();
        let mut p = 0usize;
        // SAFETY: CoCreateInstance with valid GUIDs and an out-pointer.
        let hr = unsafe { CoCreateInstance(&CLSID_SPELLCHECKERFACTORY, 0, CLSCTX_INPROC_SERVER, &IID_ISPELLCHECKERFACTORY, &mut p) };
        if hr < 0 || p == 0 { None } else { Some(Factory(p)) }
    }

    fn supported(&self, tag: &str) -> bool {
        let w = wide(tag);
        let mut ok = 0i32;
        // SAFETY: IsSupported(LPCWSTR, BOOL*) on the live factory.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *const u16, *mut i32) -> i32 = core::mem::transmute(vfn(self.0, F_IS_SUPPORTED));
            f(self.0, w.as_ptr(), &mut ok)
        };
        hr == S_OK && ok != 0
    }

    fn languages(&self) -> Vec<String> {
        let mut e = 0usize;
        // SAFETY: get_SupportedLanguages(IEnumString**) on the live factory.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *mut usize) -> i32 = core::mem::transmute(vfn(self.0, F_SUPPORTED_LANGUAGES));
            f(self.0, &mut e)
        };
        if hr < 0 || e == 0 {
            return Vec::new();
        }
        // SAFETY: e is the IEnumString just returned, released by enum_strings.
        unsafe { enum_strings(e, 256) }
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        // SAFETY: the factory made in new().
        unsafe { release(self.0) };
    }
}

/// a wide string the API allocated: copied, then freed (CoTaskMemFree)
///
/// SAFETY: p is NULL or a NUL-terminated string from CoTaskMemAlloc.
unsafe fn take_string(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let mut n = 0;
        while n < 4096 && *p.add(n) != 0 { // the limit first: no read past it
            n += 1;
        }
        let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
        CoTaskMemFree(p);
        s
    }
}

/// the strings of an IEnumString (at most max), which is released
///
/// SAFETY: e is a live IEnumString owned by the caller.
unsafe fn enum_strings(e: usize, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let next: unsafe extern "system" fn(usize, u32, *mut *mut u16, *mut u32) -> i32 = core::mem::transmute(vfn(e, E_NEXT));
        while out.len() < max {
            let mut p: *mut u16 = core::ptr::null_mut();
            let mut got = 0u32;
            let hr = next(e, 1, &mut p, &mut got);
            if hr != S_OK || got == 0 {
                if !p.is_null() {
                    CoTaskMemFree(p);
                }
                break;
            }
            out.push(take_string(p));
        }
        release(e);
    }
    out
}

/// every supported language
pub(super) fn languages() -> Vec<String> {
    Factory::new().map(|f| f.languages()).unwrap_or_default()
}

/// the tag the system uses for `lang` (see the module header)
pub(super) fn find(lang: &str) -> Option<String> {
    let f = Factory::new()?;
    let lang = lang.replace('_', "-");
    if !lang.is_empty() && f.supported(&lang) {
        return Some(lang);
    }
    let primary = lang.split('-').next().unwrap_or("").to_ascii_lowercase();
    if primary.is_empty() {
        return None;
    }
    let all = f.languages();
    let same: Vec<&String> = all.iter().filter(|t| t.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(&primary))).collect();
    let main = alloc::format!("{}-{}", primary, main_region(&primary)).to_ascii_lowercase();
    same.iter().find(|t| t.to_ascii_lowercase() == main).or(same.first()).map(|t| String::from(t.as_str()))
}

// the main region of a language ("en" -> "US", else the language in capitals)
fn main_region(primary: &str) -> String {
    match primary {
        "en" => String::from("US"),
        "he" => String::from("IL"),
        "el" => String::from("GR"),
        "da" => String::from("DK"),
        "sv" => String::from("SE"),
        "cs" => String::from("CZ"),
        "uk" => String::from("UA"),
        "nb" | "nn" => String::from("NO"),
        "ar" => String::from("SA"),
        "ja" => String::from("JP"),
        "ko" => String::from("KR"),
        "zh" => String::from("CN"),
        p => p.to_ascii_uppercase(),
    }
}

/// an ISpellChecker
pub(super) struct Checker(usize);

impl Checker {
    pub(super) fn open(tag: &str) -> Option<Checker> {
        let f = Factory::new()?;
        let w = wide(tag);
        let mut c = 0usize;
        // SAFETY: CreateSpellChecker(LPCWSTR, ISpellChecker**) on the live factory.
        let hr = unsafe {
            let cf: unsafe extern "system" fn(usize, *const u16, *mut usize) -> i32 = core::mem::transmute(vfn(f.0, F_CREATE));
            cf(f.0, w.as_ptr(), &mut c)
        };
        if hr < 0 || c == 0 { None } else { Some(Checker(c)) }
    }

    /// no spelling error in the word
    pub(super) fn check(&self, word: &str) -> bool {
        let w = wide(word);
        let mut e = 0usize;
        // SAFETY: Check(LPCWSTR, IEnumSpellingError**) on the live checker.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *const u16, *mut usize) -> i32 = core::mem::transmute(vfn(self.0, C_CHECK));
            f(self.0, w.as_ptr(), &mut e)
        };
        if hr < 0 || e == 0 {
            return true;
        }
        let mut err = 0usize;
        // SAFETY: IEnumSpellingError::Next(ISpellingError**) on the live enumerator; both
        // objects are released.
        unsafe {
            let next: unsafe extern "system" fn(usize, *mut usize) -> i32 = core::mem::transmute(vfn(e, E_NEXT));
            let hr = next(e, &mut err);
            if err != 0 {
                release(err);
            }
            release(e);
            !(hr == S_OK && err != 0)
        }
    }

    /// corrections (at most max)
    pub(super) fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let w = wide(word);
        let mut e = 0usize;
        // SAFETY: Suggest(LPCWSTR, IEnumString**) on the live checker.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *const u16, *mut usize) -> i32 = core::mem::transmute(vfn(self.0, C_SUGGEST));
            f(self.0, w.as_ptr(), &mut e)
        };
        if hr != S_OK || e == 0 {
            if e != 0 {
                // SAFETY: the enumerator just returned (S_FALSE: the word itself).
                unsafe { release(e) };
            }
            return Vec::new();
        }
        // SAFETY: the IEnumString just returned.
        let mut v = unsafe { enum_strings(e, max + 1) };
        v.retain(|s| s != word);
        v.truncate(max);
        v
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        // SAFETY: the checker made in open().
        unsafe { release(self.0) };
    }
}
