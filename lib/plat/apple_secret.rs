// apple_secret -- the secret store on macOS and iOS: generic passwords of the keychain (Security
// framework), service "jflib:<app>/<service>", account the caller's account ('%' and '/' of the names
// as %25 / %2F, as win_secret.rs names its targets).  macOS: the user's login keychain (a program
// without a signed keychain group cannot use the data protection keychain); iOS: the app's own.  A
// locked keychain that may not ask (an SSH session) is Dismissed; another refusal Failed with the
// OSStatus.  secret.rs checks the names and the manifest first.

use super::{Name, SecretError};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

type CfRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    static kCFTypeDictionaryValueCallBacks: [usize; 5];
    static kCFBooleanTrue: CfRef;
    fn CFDictionaryCreate(alloc: CfRef, keys: *const CfRef, values: *const CfRef, n: isize, kc: *const [usize; 6], vc: *const [usize; 5]) -> CfRef;
    fn CFStringCreateWithBytes(alloc: CfRef, bytes: *const u8, n: isize, encoding: u32, external: u8) -> CfRef;
    fn CFDataCreate(alloc: CfRef, bytes: *const u8, n: isize) -> CfRef;
    fn CFDataGetLength(d: CfRef) -> isize;
    fn CFDataGetBytePtr(d: CfRef) -> *const u8;
    fn CFRelease(r: CfRef);
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CfRef;
    static kSecClassGenericPassword: CfRef;
    static kSecAttrService: CfRef;
    static kSecAttrAccount: CfRef;
    static kSecValueData: CfRef;
    static kSecReturnData: CfRef;
    static kSecMatchLimit: CfRef;
    static kSecMatchLimitOne: CfRef;
    fn SecItemAdd(attrs: CfRef, result: *mut CfRef) -> i32;
    fn SecItemUpdate(query: CfRef, attrs: CfRef) -> i32;
    fn SecItemCopyMatching(query: CfRef, result: *mut CfRef) -> i32;
    fn SecItemDelete(query: CfRef) -> i32;
}

const UTF8: u32 = 0x0800_0100;
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_SEC_AUTH_FAILED: i32 = -25293;

/// a CoreFoundation object released when dropped
struct Cf(CfRef);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: an object this module created (or received with ownership) and releases once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf_str(s: &str) -> Cf {
    // SAFETY: the bytes live during the call, which copies them.
    Cf(unsafe { CFStringCreateWithBytes(core::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) })
}

fn esc(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

/// a dictionary of the entry's names plus extra key / value pairs
fn query(service: &Cf, account: &Cf, extra: &[(CfRef, CfRef)]) -> Cf {
    // SAFETY: the Security framework's key constants and the caller's objects live during the call;
    // the dictionary retains what it holds.
    unsafe {
        let mut keys = alloc::vec![kSecClass, kSecAttrService, kSecAttrAccount];
        let mut vals = alloc::vec![kSecClassGenericPassword, service.0, account.0];
        for (k, v) in extra {
            keys.push(*k);
            vals.push(*v);
        }
        Cf(CFDictionaryCreate(core::ptr::null(), keys.as_ptr(), vals.as_ptr(), keys.len() as isize,
                              &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks))
    }
}

fn names(n: &Name) -> (Cf, Cf) {
    (cf_str(&format!("jflib:{}/{}", esc(&n.app), esc(n.service))), cf_str(n.account))
}

fn fail(status: i32) -> SecretError {
    match status {
        ERR_SEC_INTERACTION_NOT_ALLOWED | ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED => SecretError::Dismissed,
        s => SecretError::Failed(format!("OSStatus {s}")),
    }
}

/// the keychain is always there (whether it may be used shows the first call)
pub fn available() -> bool {
    true
}

/// create or replace (update first, add when there is none)
pub fn set(n: &Name, secret: &[u8]) -> Result<(), SecretError> {
    let (service, account) = names(n);
    // SAFETY: the bytes live during the call, which copies them.
    let data = Cf(unsafe { CFDataCreate(core::ptr::null(), secret.as_ptr(), secret.len() as isize) });
    let q = query(&service, &account, &[]);
    // SAFETY: valid dictionaries; the value data is retained by them.
    unsafe {
        let attrs = Cf(CFDictionaryCreate(core::ptr::null(), &kSecValueData, &data.0, 1,
                                          &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks));
        match SecItemUpdate(q.0, attrs.0) {
            0 => return Ok(()),
            ERR_SEC_ITEM_NOT_FOUND => {}
            s => return Err(fail(s)),
        }
        let add = query(&service, &account, &[(kSecValueData, data.0)]);
        match SecItemAdd(add.0, core::ptr::null_mut()) {
            0 => Ok(()),
            ERR_SEC_DUPLICATE_ITEM => match SecItemUpdate(q.0, attrs.0) {
                0 => Ok(()),
                s => Err(fail(s)),
            },
            s => Err(fail(s)),
        }
    }
}

/// the value, None when there is none
pub fn get(n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
    let (service, account) = names(n);
    // SAFETY: the framework's constants; the result is ours to release (Cf).
    unsafe {
        let q = query(&service, &account, &[(kSecReturnData, kCFBooleanTrue), (kSecMatchLimit, kSecMatchLimitOne)]);
        let mut out: CfRef = core::ptr::null();
        match SecItemCopyMatching(q.0, &mut out) {
            0 => {
                let data = Cf(out);
                let len = CFDataGetLength(data.0).max(0) as usize;
                let p = CFDataGetBytePtr(data.0);
                Ok(Some(if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, len).to_vec() }))
            }
            ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            s => Err(fail(s)),
        }
    }
}

/// remove the entry -> true when there was one
pub fn delete(n: &Name) -> Result<bool, SecretError> {
    let (service, account) = names(n);
    let q = query(&service, &account, &[]);
    // SAFETY: a valid query dictionary.
    match unsafe { SecItemDelete(q.0) } {
        0 => Ok(true),
        ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        s => Err(fail(s)),
    }
}
