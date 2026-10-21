// android_secret -- secret.rs on Android: the Android Keystore, through the Java host
// (android/GallerySecrets.java, asked synchronously like the clipboard: android_host.rs)
//
// One AES-256-GCM key per app in the AndroidKeyStore (alias "jfgui-secrets", made on the first
// set): not exportable, kept by the device's TEE where it has one, usable without user
// authentication and while the device is locked (a background job may need its token).  Each
// value is encrypted with it -- a random 12-byte IV the Keystore picks, the 16-byte tag, the
// entry's name "app\0service\0account" as additional data, so a file copied to another name
// does not decrypt -- into <noBackupFilesDir>/jfgui-secrets/<SHA-256 of the name, hex>.bin
// ("JFS1", IV, ciphertext + tag), written through AtomicFile.  no_backup: Auto Backup would
// restore the files onto a device whose Keystore lacks the key.
//
//   CALL_SECRET_AVAILABLE                          -> u32 status, message
//   CALL_SECRET_GET     app, service, account       -> u32 status, the value / message
//   CALL_SECRET_SET     app, service, account, value -> u32 status, message
//   CALL_SECRET_DELETE  app, service, account       -> u32 status, message
// status 0 done, 1 missing (get, delete), 2 failed (what Java said).  No answer (no host, a
// thread without Java) is Unavailable.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Name, SecretError};
use crate::web::android_host::{CALL_SECRET_AVAILABLE, CALL_SECRET_DELETE, CALL_SECRET_GET, CALL_SECRET_SET, R, W, call};

const DONE: u32 = 0;
const MISSING: u32 = 1;

// one question -> (status, value or message); the plaintext copies are cleared
fn ask(kind: u32, n: &Name, value: Option<&[u8]>) -> Result<(u32, Vec<u8>), SecretError> {
    let mut w = W::default().str(&n.app).str(n.service).str(n.account);
    if let Some(v) = value {
        w = w.bytes(v);
    }
    let answer = call(kind, &w.0);
    crypto::zeroize(&mut w.0);
    let mut a = answer.ok_or(SecretError::Unavailable)?;
    let mut r = R(&a);
    let got = r.u32().zip(r.bytes().map(<[u8]>::to_vec));
    crypto::zeroize(&mut a);
    got.ok_or_else(|| SecretError::Failed(String::from("the Android host gave no valid answer")))
}

fn failed(mut why: Vec<u8>) -> SecretError {
    let s = String::from_utf8_lossy(&why).into_owned();
    crypto::zeroize(&mut why);
    SecretError::Failed(format!("Android Keystore: {s}"))
}

pub fn available() -> bool {
    call(CALL_SECRET_AVAILABLE, &[]).is_some_and(|a| R(&a).u32() == Some(DONE))
}

pub fn set(n: &Name, secret: &[u8]) -> Result<(), SecretError> {
    match ask(CALL_SECRET_SET, n, Some(secret))? {
        (DONE, _) => Ok(()),
        (_, why) => Err(failed(why)),
    }
}

pub fn get(n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
    match ask(CALL_SECRET_GET, n, None)? {
        (DONE, v) => Ok(Some(v)),
        (MISSING, _) => Ok(None),
        (_, why) => Err(failed(why)),
    }
}

pub fn delete(n: &Name) -> Result<bool, SecretError> {
    match ask(CALL_SECRET_DELETE, n, None)? {
        (DONE, _) => Ok(true),
        (MISSING, _) => Ok(false),
        (_, why) => Err(failed(why)),
    }
}
