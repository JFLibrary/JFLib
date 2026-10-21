// web_secret -- secret.rs in the browser: the web host's store (web/secrets.js)
//
// The host keeps one AES-GCM key (256 bits) per app as a non-extractable WebCrypto CryptoKey in
// IndexedDB (database jfgui-secrets-v1, store "keys") and the values encrypted with it (store
// "values": app, service, account, a random 12-byte IV, ciphertext + tag; the additional data
// is "app\0service\0account", so a record copied to another name does not decrypt).
// WebCrypto and IndexedDB answer only asynchronously, the API is synchronous: the host decrypts
// the app's entries before the app starts (jfgui_secrets) and plat answers get from that copy;
// set and delete change the copy at once and hand the change to the host (REQUEST_SECRET),
// which encrypts and commits the changes in order and completes them (jfgui_complete ->
// committed).  A change that does not reach IndexedDB leaves its entry Failed until it is set
// or deleted again.  Without the host's store (no IndexedDB or WebCrypto -- a page served over
// plain http from another host than localhost --, an older host) every call is Unavailable.
// Other open pages of the app see a change after they are reloaded.

use alloc::string::String;
use alloc::vec::Vec;
use rt::sync::SpinLock;

use super::{Name, SecretError};
use crate::web::{REQUEST_SECRET, input, request, wipe_input};

const OP_SET: u32 = 1;
const OP_DELETE: u32 = 2;
// jfgui_complete's status of a committed change
const COMMITTED: i32 = 2;

struct Entry {
    app: String,
    service: String,
    account: String,
    // None: deleted (the delete not committed yet)
    value: Option<Vec<u8>>,
    // the request of the last change while the host commits it (0: none)
    pending: u32,
    // the last change did not reach IndexedDB, or the stored value did not decrypt
    failed: Option<&'static str>,
}

impl Entry {
    fn is(&self, n: &Name) -> bool {
        self.app == n.app && self.service == n.service && self.account == n.account
    }
    fn replace(&mut self, value: Option<Vec<u8>>, pending: u32) {
        if let Some(mut old) = core::mem::replace(&mut self.value, value) {
            crypto::zeroize(&mut old);
        }
        self.pending = pending;
        self.failed = None;
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        if let Some(v) = &mut self.value {
            crypto::zeroize(v);
        }
    }
}

// the app's entries; None: the host has no store
static STORE: SpinLock<Option<Vec<Entry>>> = SpinLock::new(None);

struct Rd<'a>(&'a [u8]);
impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = b;
        Some(a)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Option<String> {
        Some(String::from(core::str::from_utf8(self.bytes()?).ok()?))
    }
}

fn put(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

/// (web host) the app's stored secrets, decrypted, before jfgui_start; this turns the store on.
/// Inbox: u32 count, then per entry app, service, account (UTF-8), u32 readable (0: the record
/// did not decrypt), the value; strings and bytes as u32 length + bytes.  1 = taken.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_secrets(len: u32) -> u32 {
    let Some(mut bytes) = input(len) else { return 0 };
    wipe_input();
    let mut r = Rd(&bytes);
    let mut entries = Vec::new();
    let ok = (|| {
        for _ in 0..r.u32()? {
            let (app, service, account, readable) = (r.str()?, r.str()?, r.str()?, r.u32()? != 0);
            let value = r.bytes()?;
            entries.push(Entry {
                app,
                service,
                account,
                value: readable.then(|| value.to_vec()),
                pending: 0,
                failed: (!readable).then_some("the stored value does not decrypt"),
            });
        }
        r.0.is_empty().then_some(())
    })()
    .is_some();
    crypto::zeroize(&mut bytes);
    if !ok {
        return 0;
    }
    *STORE.lock() = Some(entries);
    1
}

/// (web.rs: jfgui_complete) the host committed the change `id` (status 2) or did not
pub(crate) fn committed(id: u32, status: i32) {
    let mut g = STORE.lock();
    let Some(entries) = g.as_mut() else { return };
    // a later change of the entry decides (the host commits in order)
    let Some(i) = entries.iter().position(|e| e.pending == id) else { return };
    entries[i].pending = 0;
    if status != COMMITTED {
        entries[i].failed = Some(match status {
            -1 => "the browser refused its storage",
            -2 => "the browser's storage is full",
            _ => "the browser did not store the change",
        });
    } else if entries[i].value.is_none() {
        entries.remove(i);
    }
}

// hand a change to the host (the store's lock is held: the requests keep the order of the copy)
fn commit(op: u32, n: &Name, value: Option<&[u8]>) -> Result<u32, SecretError> {
    let mut msg = Vec::new();
    let size = 16 + n.app.len() + n.service.len() + n.account.len() + value.map_or(0, |v| 4 + v.len());
    if msg.try_reserve_exact(size).is_err() {
        return Err(SecretError::Failed(String::from("out of memory")));
    }
    msg.extend_from_slice(&op.to_le_bytes());
    put(&mut msg, n.app.as_bytes());
    put(&mut msg, n.service.as_bytes());
    put(&mut msg, n.account.as_bytes());
    if let Some(v) = value {
        put(&mut msg, v);
    }
    let id = request(REQUEST_SECRET, &msg, None);
    crypto::zeroize(&mut msg);
    id.ok_or_else(|| SecretError::Failed(String::from("too many host requests are pending")))
}

pub fn available() -> bool {
    STORE.lock().is_some()
}

pub fn set(n: &Name, secret: &[u8]) -> Result<(), SecretError> {
    let mut g = STORE.lock();
    let entries = g.as_mut().ok_or(SecretError::Unavailable)?;
    let mut value = Vec::new();
    if value.try_reserve_exact(secret.len()).is_err() {
        return Err(SecretError::Failed(String::from("out of memory")));
    }
    value.extend_from_slice(secret);
    let id = commit(OP_SET, n, Some(secret))?;
    match entries.iter_mut().find(|e| e.is(n)) {
        Some(e) => e.replace(Some(value), id),
        None => entries.push(Entry {
            app: n.app.clone(),
            service: String::from(n.service),
            account: String::from(n.account),
            value: Some(value),
            pending: id,
            failed: None,
        }),
    }
    Ok(())
}

pub fn get(n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
    let g = STORE.lock();
    let entries = g.as_ref().ok_or(SecretError::Unavailable)?;
    match entries.iter().find(|e| e.is(n)) {
        Some(Entry { failed: Some(why), .. }) => Err(SecretError::Failed(String::from(*why))),
        Some(e) => Ok(e.value.clone()),
        None => Ok(None),
    }
}

pub fn delete(n: &Name) -> Result<bool, SecretError> {
    let mut g = STORE.lock();
    let entries = g.as_mut().ok_or(SecretError::Unavailable)?;
    // also without a copy: another page of the app may have stored it meanwhile
    let id = commit(OP_DELETE, n, None)?;
    match entries.iter_mut().find(|e| e.is(n)) {
        Some(e) => {
            let had = e.value.is_some() || e.failed.is_some();
            e.replace(None, id);
            Ok(had)
        }
        None => {
            entries.push(Entry {
                app: n.app.clone(),
                service: String::from(n.service),
                account: String::from(n.account),
                value: None,
                pending: id,
                failed: None,
            });
            Ok(false)
        }
    }
}
