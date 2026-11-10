// apple_bundle -- (pkg.rs) macOS: an app bundle replaced as a whole by an update (lib/update, kind
// "app"; docs/APPLE.md AP8a)
//
//   follows(old, new) -> Ok / Err(why)   the new bundle's signature is valid (the whole bundle, strict)
//                        and it may follow the old one: a bundle signed by a team must satisfy the old
//                        one's designated requirement (the same identifier and team, Apple's anchor); an
//                        ad-hoc signed one (its requirement is its own hash) must carry the same
//                        identifier and be signed ad hoc as well or by a team
//   swap(a, b) -> bool   the two folders exchanged in one step (renamex_np, RENAME_SWAP): the app is
//                        always there, old or new

use alloc::format;
use alloc::string::String;
use core::ffi::c_void;

type CfRef = *const c_void;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecCodeInfoTeamIdentifier: CfRef;
    static kSecCodeInfoIdentifier: CfRef;
    fn SecStaticCodeCreateWithPath(path: CfRef, flags: u32, code: *mut CfRef) -> i32;
    fn SecStaticCodeCheckValidity(code: CfRef, flags: u32, requirement: CfRef) -> i32;
    fn SecCodeCopyDesignatedRequirement(code: CfRef, flags: u32, requirement: *mut CfRef) -> i32;
    fn SecCodeCopySigningInformation(code: CfRef, flags: u32, info: *mut CfRef) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(alloc: CfRef, buf: *const u8, len: isize, is_dir: u8) -> CfRef;
    fn CFDictionaryGetValue(dict: CfRef, key: CfRef) -> CfRef;
    fn CFEqual(a: CfRef, b: CfRef) -> u8;
    fn CFRelease(r: CfRef);
}
unsafe extern "C" {
    fn renamex_np(from: *const u8, to: *const u8, flags: u32) -> i32;
}

const RENAME_SWAP: u32 = 0x2;
// SecCSFlags
const CHECK_ALL_ARCHITECTURES: u32 = 1 << 0;
const CHECK_NESTED_CODE: u32 = 1 << 3;
const STRICT_VALIDATE: u32 = 1 << 4;
const SIGNING_INFORMATION: u32 = 1 << 1;

/// a CoreFoundation object of ours, released at the end
struct Cf(CfRef);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: an object this module created or copied (+1).
            unsafe { CFRelease(self.0) };
        }
    }
}

fn zero(path: &str) -> Option<alloc::vec::Vec<u8>> {
    if path.contains('\0') {
        return None;
    }
    let mut z = alloc::vec::Vec::from(path.as_bytes());
    z.push(0);
    Some(z)
}

/// the static code of the bundle at path
fn code(path: &str) -> Result<Cf, String> {
    // SAFETY: a URL of the path's bytes; SecStaticCodeCreateWithPath writes the code (+1) or fails.
    unsafe {
        let url = Cf(CFURLCreateFromFileSystemRepresentation(core::ptr::null(), path.as_ptr(), path.len() as isize, 1));
        if url.0.is_null() {
            return Err(format!("{path}: no URL"));
        }
        let mut c: CfRef = core::ptr::null();
        let st = SecStaticCodeCreateWithPath(url.0, 0, &mut c);
        if st != 0 || c.is_null() {
            return Err(format!("{path}: no signed code ({st})"));
        }
        Ok(Cf(c))
    }
}

/// the signing information of a code: (identifier, team) -- the copied dictionary is kept
fn signer(c: &Cf) -> Result<(Cf, CfRef, CfRef), String> {
    // SAFETY: the code's signing information (+1); its values live as long as the dictionary.
    unsafe {
        let mut info: CfRef = core::ptr::null();
        let st = SecCodeCopySigningInformation(c.0, SIGNING_INFORMATION, &mut info);
        if st != 0 || info.is_null() {
            return Err(format!("no signing information ({st})"));
        }
        let ident = CFDictionaryGetValue(info, kSecCodeInfoIdentifier);
        let team = CFDictionaryGetValue(info, kSecCodeInfoTeamIdentifier);
        Ok((Cf(info), ident, team))
    }
}

/// whether the bundle at `new` may replace the one at `old` (see the module header)
pub(crate) fn follows(old: &str, new: &str) -> Result<(), String> {
    let (old_code, new_code) = (code(old)?, code(new)?);
    // SAFETY: the codes made above; a requirement copied (+1) and released; dictionaries kept while read.
    unsafe {
        let mut req: CfRef = core::ptr::null();
        let (_old_info, old_ident, old_team) = signer(&old_code)?;
        let req = if old_team.is_null() {
            None // ad hoc: its requirement is its own hash, which no new version has
        } else if SecCodeCopyDesignatedRequirement(old_code.0, 0, &mut req) == 0 && !req.is_null() {
            Some(Cf(req))
        } else {
            return Err(String::from("the installed app's requirement cannot be read"));
        };
        let st = SecStaticCodeCheckValidity(new_code.0, CHECK_ALL_ARCHITECTURES | CHECK_NESTED_CODE | STRICT_VALIDATE,
                                            req.as_ref().map_or(core::ptr::null(), |r| r.0));
        if st != 0 {
            return Err(format!("the new app's signature does not check{} ({st})", if req.is_some() { " against the installed app's requirement" } else { "" }));
        }
        let (_new_info, new_ident, _) = signer(&new_code)?;
        if old_ident.is_null() || new_ident.is_null() || CFEqual(old_ident, new_ident) == 0 {
            return Err(String::from("the new app has another identifier"));
        }
    }
    Ok(())
}

/// exchange the folders a and b in one step -> it worked
pub(crate) fn swap(a: &str, b: &str) -> bool {
    let (Some(za), Some(zb)) = (zero(a), zero(b)) else { return false };
    // SAFETY: two NUL-terminated paths.
    unsafe { renamex_np(za.as_ptr(), zb.as_ptr(), RENAME_SWAP) == 0 }
}
