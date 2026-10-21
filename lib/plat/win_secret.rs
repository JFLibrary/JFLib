// win_secret -- secret.rs on Windows: the Credential Manager (advapi32) and DPAPI (crypt32)
//
// One generic credential (CRED_TYPE_GENERIC) per entry, target "jflib:<app>/<service>/<account>",
// user name = the account, persisted for this user on this computer (CRED_PERSIST_LOCAL_MACHINE).
// ENTERPRISE would copy the entry to every machine of a roaming profile: app tokens are often
// bound to the device, and the Linux keyring stays local too.  The Credential Manager keeps at
// most 2560 bytes (CRED_MAX_CREDENTIAL_BLOB_SIZE): larger values are encrypted with
// CryptProtectData (the user's DPAPI key, the target as additional entropy) into
// data://secrets/<target hash>-<file hash>.bin, and the credential holds an empty value plus the
// attribute JFLib_File = SHA-256 of that file.  A new file is written before the credential
// points at it; afterwards the entry's other files are removed.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{null, null_mut};

use super::{Name, SecretError};

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const CRED_MAX_BLOB: usize = 5 * 512;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_NO_SUCH_LOGON_SESSION: u32 = 1312;
const CRYPTPROTECT_UI_FORBIDDEN: u32 = 1;
const FILE_KEYWORD: &str = "JFLib_File";
const DIR: &str = "data://secrets";

#[repr(C)]
struct Attribute {
    keyword: *const u16,
    flags: u32,
    size: u32,
    value: *const u8,
}

// CREDENTIALW
#[repr(C)]
struct Credential {
    flags: u32,
    typ: u32,
    target: *const u16,
    comment: *const u16,
    written: [u32; 2],
    blob_size: u32,
    blob: *mut u8,
    persist: u32,
    attr_count: u32,
    attrs: *const Attribute,
    alias: *const u16,
    user: *const u16,
}

// DATA_BLOB
#[repr(C)]
struct Blob {
    size: u32,
    data: *mut u8,
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CredWriteW(cred: *const Credential, flags: u32) -> i32;
    fn CredReadW(target: *const u16, typ: u32, flags: u32, cred: *mut *mut Credential) -> i32;
    fn CredDeleteW(target: *const u16, typ: u32, flags: u32) -> i32;
    fn CredFree(buf: *mut c_void);
}

#[link(name = "crypt32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CryptProtectData(input: *const Blob, descr: *const u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, out: *mut Blob) -> i32;
    fn CryptUnprotectData(input: *const Blob, descr: *mut *mut u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, out: *mut Blob) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

// '%' and '/' of a name escaped, so "a/b" + "c" and "a" + "b/c" stay apart
fn esc(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

fn target(n: &Name) -> String {
    format!("jflib:{}/{}/{}", n.app, esc(n.service), esc(n.account))
}

fn os_error(call: &str, code: u32) -> SecretError {
    if code == ERROR_NO_SUCH_LOGON_SESSION {
        return SecretError::Unavailable; // no credential store in this logon session (services)
    }
    SecretError::Failed(format!("{call} failed (error {code})"))
}

// the start of the file names of one entry: 16 hex digits of the target's SHA-256
fn file_prefix(target: &str) -> String {
    format!("{}-", crypto::hex(&crypto::sha256(target.as_bytes())[..8]))
}

fn file_name(target: &str, hash: &[u8; 32]) -> String {
    format!("{}{}.bin", file_prefix(target), crypto::hex(&hash[..8]))
}

// an entry as stored: its value, and the hash of its DPAPI file
struct Stored {
    blob: Vec<u8>,
    file: Option<[u8; 32]>,
}

// wide string at p (NUL-terminated) == s
//
// SAFETY: p is null or a NUL-terminated UTF-16 string.
unsafe fn wide_is(p: *const u16, s: &str) -> bool {
    if p.is_null() {
        return false;
    }
    let mut i = 0;
    for u in s.encode_utf16() {
        // SAFETY: the string goes on at least up to its NUL, which ends the loop below.
        if unsafe { *p.add(i) } != u {
            return false;
        }
        i += 1;
    }
    // SAFETY: as above.
    unsafe { *p.add(i) == 0 }
}

fn read(target: &str) -> Result<Option<Stored>, SecretError> {
    let w = crate::to_wide(target);
    let mut p: *mut Credential = null_mut();
    // SAFETY: a NUL-terminated name and an out pointer; CredFree releases the record.
    if unsafe { CredReadW(w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) } == 0 {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        return if e == ERROR_NOT_FOUND { Ok(None) } else { Err(os_error("CredReadW", e)) };
    }
    // SAFETY: CredReadW returned a valid record; its value and attributes live until CredFree.
    let st = unsafe {
        let c = &*p;
        let mut blob = Vec::new();
        if !c.blob.is_null() && c.blob_size > 0 {
            let v = core::slice::from_raw_parts_mut(c.blob, c.blob_size as usize);
            blob.extend_from_slice(v);
            v.fill(0);
        }
        let mut file = None;
        for i in 0..c.attr_count as usize {
            let a = &*c.attrs.add(i);
            if a.size == 32 && !a.value.is_null() && wide_is(a.keyword, FILE_KEYWORD) {
                let mut h = [0u8; 32];
                h.copy_from_slice(core::slice::from_raw_parts(a.value, 32));
                file = Some(h);
            }
        }
        CredFree(p as *mut c_void);
        Stored { blob, file }
    };
    Ok(Some(st))
}

fn write(target: &str, n: &Name, value: &[u8], file: Option<&[u8; 32]>) -> Result<(), SecretError> {
    let tw = crate::to_wide(target);
    let user = crate::to_wide(n.account);
    let comment = crate::to_wide(&format!("JFLib secret of {}", n.app));
    let kw = crate::to_wide(FILE_KEYWORD);
    let attr = file.map(|h| Attribute { keyword: kw.as_ptr(), flags: 0, size: 32, value: h.as_ptr() });
    let c = Credential {
        flags: 0,
        typ: CRED_TYPE_GENERIC,
        target: tw.as_ptr(),
        comment: comment.as_ptr(),
        written: [0; 2],
        blob_size: value.len() as u32,
        blob: if value.is_empty() { null_mut() } else { value.as_ptr() as *mut u8 },
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attr_count: attr.is_some() as u32,
        attrs: attr.as_ref().map_or(null(), |a| a as *const Attribute),
        alias: null(),
        user: if n.account.is_empty() { null() } else { user.as_ptr() },
    };
    // SAFETY: the record and everything it points to live during the call, which only reads.
    if unsafe { CredWriteW(&c, 0) } == 0 {
        // SAFETY: no arguments.
        return Err(os_error("CredWriteW", unsafe { GetLastError() }));
    }
    Ok(())
}

// DPAPI: protect (true) or unprotect data, entropy bound to the entry
fn dpapi(protect: bool, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, SecretError> {
    let input = Blob { size: data.len() as u32, data: data.as_ptr() as *mut u8 };
    let ent = Blob { size: entropy.len() as u32, data: entropy.as_ptr() as *mut u8 };
    let mut out = Blob { size: 0, data: null_mut() };
    // SAFETY: the blobs describe live slices the calls only read; the output buffer is
    // allocated by the call and released with LocalFree below.
    let ok = unsafe {
        if protect {
            CryptProtectData(&input, null(), &ent, null_mut(), null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, null_mut(), &ent, null_mut(), null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        return Err(os_error(if protect { "CryptProtectData" } else { "CryptUnprotectData" }, e));
    }
    let mut v = Vec::new();
    if !out.data.is_null() {
        // SAFETY: out describes the buffer the call allocated; it is cleared, then freed.
        unsafe {
            let s = core::slice::from_raw_parts_mut(out.data, out.size as usize);
            v.extend_from_slice(s);
            s.fill(0);
            LocalFree(out.data as *mut c_void);
        }
    }
    Ok(v)
}

// remove the entry's DPAPI files except keep (an old value, a file a crash left behind)
fn sweep(target: &str, keep: Option<&[u8; 32]>) {
    let prefix = file_prefix(target);
    let keep = keep.map(|h| file_name(target, h));
    let mut old: Vec<String> = Vec::new();
    crate::dir_list(DIR, &mut |name, dir, _| {
        if !dir && name.starts_with(&prefix) && Some(name) != keep.as_deref() {
            old.push(String::from(name));
        }
    });
    for f in old {
        crate::delete(&format!("{DIR}/{f}"));
    }
}

pub fn available() -> bool {
    // a lookup of a name nobody uses: "not found" means the store is there
    let w = crate::to_wide("jflib:?");
    let mut p: *mut Credential = null_mut();
    // SAFETY: a NUL-terminated name and an out pointer.
    if unsafe { CredReadW(w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) } != 0 {
        // SAFETY: the record CredReadW returned.
        unsafe { CredFree(p as *mut c_void) };
        return true;
    }
    // SAFETY: no arguments.
    unsafe { GetLastError() != ERROR_NO_SUCH_LOGON_SESSION }
}

pub fn set(n: &Name, secret: &[u8]) -> Result<(), SecretError> {
    let t = target(n);
    if secret.len() <= CRED_MAX_BLOB {
        write(&t, n, secret, None)?;
        sweep(&t, None);
        return Ok(());
    }
    let enc = dpapi(true, secret, t.as_bytes())?;
    let hash = crypto::sha256(&enc);
    let file = format!("{DIR}/{}", file_name(&t, &hash));
    if !crate::write_file(&file, &enc) {
        return Err(SecretError::Failed(format!("cannot write {file}")));
    }
    if let Err(e) = write(&t, n, &[], Some(&hash)) {
        crate::delete(&file);
        return Err(e);
    }
    sweep(&t, Some(&hash));
    Ok(())
}

pub fn get(n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
    let t = target(n);
    let Some(st) = read(&t)? else { return Ok(None) };
    let Some(hash) = st.file else { return Ok(Some(st.blob)) };
    let file = format!("{DIR}/{}", file_name(&t, &hash));
    let Some(enc) = crate::read_file(&file) else {
        return Err(SecretError::Failed(format!("{file} is missing")));
    };
    if !crypto::ct_eq(&crypto::sha256(&enc), &hash) {
        return Err(SecretError::Failed(format!("{file} does not match its credential")));
    }
    dpapi(false, &enc, t.as_bytes()).map(Some)
}

pub fn delete(n: &Name) -> Result<bool, SecretError> {
    let t = target(n);
    let w = crate::to_wide(&t);
    // SAFETY: a NUL-terminated name.
    if unsafe { CredDeleteW(w.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        if e != ERROR_NOT_FOUND {
            return Err(os_error("CredDeleteW", e));
        }
        sweep(&t, None);
        return Ok(false);
    }
    sweep(&t, None);
    Ok(true)
}
