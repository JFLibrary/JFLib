// secret -- passwords, tokens and keys in the system's secret store instead of files
//
//   secret_available() -> bool          a store is there and the app may use it
//   secret_set(service, account, &[u8]) -> Result<(), SecretError>        create or replace
//   secret_get(service, account) -> Option<Vec<u8>>                       None: missing / error
//   secret_get_checked(service, account) -> Result<Option<Vec<u8>>, SecretError>
//   secret_delete(service, account) -> Result<bool, SecretError>          true: one was removed
//
// An entry is named by the app (vfs::app_id()), a service ("github", "smtp.example.org") and
// an account ("alice", or "" for one value per service), so apps do not reach each other's
// entries through this API.  The app needs `secrets` in its manifest (policy.rs); without it
// every call is refused (SecretError::Denied, PERMDENY secrets <service>).  Values are bytes,
// at most SECRET_MAX of them.  Without a store the calls fail: nothing is ever written to a
// plain file instead.
//
// Windows (win_secret.rs): generic credentials of the Credential Manager, target
// "jflib:<app>/<service>/<account>" ('%' and '/' of the names as %25 / %2F), persisted on this
// computer only (CRED_PERSIST_LOCAL_MACHINE: tokens do not roam to other machines with the
// profile).  Values above 2560 bytes (the Credential Manager's limit) go DPAPI-encrypted into
// data://secrets/; the credential keeps the file's SHA-256, so neither a stale nor a foreign
// file is accepted.  Linux (linux_secret.rs): items of the Secret Service on the session bus
// (gnome-keyring, KWallet, KeePassXC) in the default collection, attributes app / service /
// account; the values travel encrypted (dh-ietf1024-sha256-aes128-cbc-pkcs7; plain only when
// the service lacks it); a locked collection is unlocked through the service's prompt.
// Android (android_secret.rs): an AES-256-GCM key of the Android Keystore (one per app, made on
// the first set, not exportable, no user authentication) encrypts each value into a file of
// the app's no_backup folder.  Browser (web_secret.rs, web/secrets.js): a non-extractable
// WebCrypto AES-GCM key per app in IndexedDB encrypts the values kept there; the host decrypts
// the app's entries before it starts, plat answers from that copy and the host commits each
// change asynchronously.
//
// The OS does not separate the programs of one user: other native programs of the same user
// can read these entries (the Credential Manager, an unlocked keyring); in the browser every
// script of the page's origin can use the key.  The calls block -- on Linux until the user
// answers a prompt (at most PROMPT_SECONDS): call them from a worker thread when the UI must
// stay responsive.

use alloc::string::String;
use alloc::vec::Vec;

/// the largest value secret_set stores
pub const SECRET_MAX: usize = 64 * 1024;
/// the longest service or account name (bytes of UTF-8)
pub const SECRET_NAME_MAX: usize = 256;
/// how long a call waits for the user to answer an unlock prompt (Linux)
pub const PROMPT_SECONDS: u32 = 300;

/// why a secret call failed
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretError {
    /// no secret store here (Linux: no Secret Service on the session bus; browser: no
    /// IndexedDB or WebCrypto, e.g. a page served over plain http)
    Unavailable,
    /// the app's permissions lack `secrets`
    Denied,
    /// the user dismissed the store's prompt (unlock, confirmation), or it timed out
    Dismissed,
    /// an empty or too long service, a name with NUL, or a value above SECRET_MAX
    Invalid,
    /// the store refused or failed: what it said
    Failed(String),
}

impl core::fmt::Display for SecretError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            SecretError::Unavailable => f.write_str("no secret store is available"),
            SecretError::Denied => f.write_str("the app's permissions do not allow secrets"),
            SecretError::Dismissed => f.write_str("the secret store's prompt was dismissed"),
            SecretError::Invalid => f.write_str("invalid secret name or value"),
            SecretError::Failed(s) => write!(f, "secret store: {s}"),
        }
    }
}

/// an entry: the app and the caller's names
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "android", target_arch = "wasm32")), allow(dead_code))]
pub(crate) struct Name<'a> {
    pub(crate) app: String,
    pub(crate) service: &'a str,
    pub(crate) account: &'a str,
}

fn name<'a>(service: &'a str, account: &'a str) -> Result<Name<'a>, SecretError> {
    let bad = |s: &str| s.len() > SECRET_NAME_MAX || s.contains('\0');
    if service.is_empty() || bad(service) || bad(account) {
        return Err(SecretError::Invalid);
    }
    if !crate::policy::secrets_allowed(service) {
        return Err(SecretError::Denied);
    }
    Ok(Name { app: crate::vfs::app_id(), service, account })
}

/// whether this system has a secret store the app may use (`secrets` in its manifest)
pub fn secret_available() -> bool {
    crate::policy::secrets_allowed("") && native::available()
}

/// store secret under (service, account), replacing what was there
pub fn secret_set(service: &str, account: &str, secret: &[u8]) -> Result<(), SecretError> {
    let n = name(service, account)?;
    if secret.len() > SECRET_MAX {
        return Err(SecretError::Invalid);
    }
    native::set(&n, secret)
}

/// the secret of (service, account); None when there is none, and on any error
pub fn secret_get(service: &str, account: &str) -> Option<Vec<u8>> {
    secret_get_checked(service, account).ok().flatten()
}

/// the secret of (service, account): Ok(None) when there is none
pub fn secret_get_checked(service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
    native::get(&name(service, account)?)
}

/// remove the secret of (service, account) -> whether there was one
pub fn secret_delete(service: &str, account: &str) -> Result<bool, SecretError> {
    native::delete(&name(service, account)?)
}

#[cfg(windows)]
#[path = "win_secret.rs"]
mod native;

#[cfg(target_os = "linux")]
#[path = "linux_secret.rs"]
mod native;

#[cfg(target_os = "android")]
#[path = "android_secret.rs"]
mod native;

#[cfg(target_arch = "wasm32")]
#[path = "web_secret.rs"]
mod native;

// (web.rs: jfgui_complete) the host committed a change, or failed to
#[cfg(target_arch = "wasm32")]
pub(crate) use native::committed;

#[cfg(not(any(windows, target_os = "linux", target_os = "android", target_arch = "wasm32")))]
mod native {
    use super::{Name, SecretError};
    use alloc::vec::Vec;

    pub fn available() -> bool {
        false
    }
    pub fn set(_n: &Name, _secret: &[u8]) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }
    pub fn get(_n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
        Err(SecretError::Unavailable)
    }
    pub fn delete(_n: &Name) -> Result<bool, SecretError> {
        Err(SecretError::Unavailable)
    }
}
