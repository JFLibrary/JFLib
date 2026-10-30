// linux_secret -- secret.rs on Linux: the Secret Service (org.freedesktop.secrets) on the
// session bus, through linux_dbus.rs; one connection and one session per call
//
//   open       OpenSession("dh-ietf1024-sha256-aes128-cbc-pkcs7", our public key):
//              Diffie-Hellman in the 1024-bit MODP group of RFC 2409 (generator 2, 128 random
//              private bytes, crypto::bn::Mont::pow_ct), the shared value as 128 bytes ->
//              HKDF-SHA256 without salt and info -> the AES-128 key; values travel AES-CBC with
//              PKCS #7 padding and a random IV.  "plain" only when the service answers
//              NotSupported (as libsecret).  Services write these numbers differently: KDE's
//              ksecretd (KWallet 6.24) sends its public value with a sign byte (129 bytes when
//              the top bit is set), refuses ours when it is shorter than 128 bytes and hashes
//              the shared value without leading zeros, libsecret / gnome-keyring pad it to 128
//              bytes.  So the peer's value is read without leading zeros, our public value
//              never starts with a zero byte (a new pair) and a shared value that does opens
//              a new session: then every way of writing them is the same.
//   set        ReadAlias("default") (missing: CreateCollection(label, "default") and its
//              prompt), CreateItem(label, attributes, secret, replace) on that collection;
//              IsLocked -> Unlock + prompt, then once more
//   get        SearchItems(app, service, account) -> unlocked / locked items; locked ones are
//              unlocked (prompt) first; Item.GetSecret(session)
//   delete     the same search, Unlock when needed, Item.Delete (+ prompt) for every match
//   prompts    AddMatch for the prompt's Completed signal, Prompt(""), wait up to
//              PROMPT_SECONDS for Completed(dismissed, result); dismissed / timeout -> Dismissed
//              (a timed-out prompt is dismissed)
// Items carry the attributes xdg:schema = org.freedesktop.Secret.Generic, app, service and
// account (secret-tool lookup app A service S account U finds them), the label
// "<app>: <service> (<account>)" and the content type text/plain for UTF-8 values.  No bus or
// no service on it: Unavailable.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crypto::aes::Aes;
use crypto::bn::{Mont, from_be, to_be};

use super::{Name, PROMPT_SECONDS, SecretError};
use crate::linux_dbus::{Conn, F_NO_REPLY, Msg, T_ERROR, T_RETURN, T_SIGNAL, Value};

const DEST: &str = "org.freedesktop.secrets";
const ROOT: &str = "/org/freedesktop/secrets";
const I_SERVICE: &str = "org.freedesktop.Secret.Service";
const I_COLLECTION: &str = "org.freedesktop.Secret.Collection";
const I_ITEM: &str = "org.freedesktop.Secret.Item";
const I_PROMPT: &str = "org.freedesktop.Secret.Prompt";
const ALGO: &str = "dh-ietf1024-sha256-aes128-cbc-pkcs7";
const E_LOCKED: &str = "org.freedesktop.Secret.Error.IsLocked";
const E_UNSUPPORTED: &str = "org.freedesktop.DBus.Error.NotSupported";
const CALL_MS: u64 = 25_000;

// RFC 2409 6.2, the second Oakley group: 2^1024 - 2^960 - 1 + 2^64 ([2^894 pi] + 129093)
const PRIME: [u8; 128] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xc9, 0x0f, 0xda, 0xa2, 0x21, 0x68, 0xc2, 0x34,
    0xc4, 0xc6, 0x62, 0x8b, 0x80, 0xdc, 0x1c, 0xd1, 0x29, 0x02, 0x4e, 0x08, 0x8a, 0x67, 0xcc, 0x74,
    0x02, 0x0b, 0xbe, 0xa6, 0x3b, 0x13, 0x9b, 0x22, 0x51, 0x4a, 0x08, 0x79, 0x8e, 0x34, 0x04, 0xdd,
    0xef, 0x95, 0x19, 0xb3, 0xcd, 0x3a, 0x43, 0x1b, 0x30, 0x2b, 0x0a, 0x6d, 0xf2, 0x5f, 0x14, 0x37,
    0x4f, 0xe1, 0x35, 0x6d, 0x6d, 0x51, 0xc2, 0x45, 0xe4, 0x85, 0xb5, 0x76, 0x62, 0x5e, 0x7e, 0xc6,
    0xf4, 0x4c, 0x42, 0xe9, 0xa6, 0x37, 0xed, 0x6b, 0x0b, 0xff, 0x5c, 0xb6, 0xf4, 0x06, 0xb7, 0xed,
    0xee, 0x38, 0x6b, 0xfb, 0x5a, 0x89, 0x9f, 0xa5, 0xae, 0x9f, 0x24, 0x11, 0x7c, 0x4b, 0x1f, 0xe6,
    0x49, 0x28, 0x66, 0x51, 0xec, 0xe6, 0x53, 0x81, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];

// ---------------------------------------------------------------- values

fn bytes(b: &[u8]) -> Value {
    Value::A(String::from("y"), b.iter().map(|&x| Value::Y(x)).collect())
}

fn as_bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::V(inner) => as_bytes(inner),
        Value::A(e, items) if e == "y" => items.iter().map(|x| if let Value::Y(b) = x { Some(*b) } else { None }).collect(),
        _ => None,
    }
}

fn as_path(v: Option<&Value>) -> String {
    match v {
        Some(Value::V(inner)) => as_path(Some(inner)),
        Some(v) => String::from(v.as_str().unwrap_or("/")),
        None => String::from("/"),
    }
}

fn paths(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::A(_, items)) => items.iter().filter_map(|x| x.as_str()).map(String::from).collect(),
        _ => Vec::new(),
    }
}

fn attributes(n: &Name, schema: bool) -> Value {
    let mut kv = vec![("app", n.app.as_str()), ("service", n.service), ("account", n.account)];
    if schema {
        kv.insert(0, ("xdg:schema", "org.freedesktop.Secret.Generic"));
    }
    let e = kv.into_iter().map(|(k, v)| Value::E(alloc::boxed::Box::new(Value::str(k)), alloc::boxed::Box::new(Value::str(v))));
    Value::A(String::from("{ss}"), e.collect())
}

fn label(n: &Name) -> String {
    if n.account.is_empty() { format!("{}: {}", n.app, n.service) } else { format!("{}: {} ({})", n.app, n.service, n.account) }
}

// an error name of the bus or the service -> SecretError ("" = no answer, a broken connection)
fn fail(e: &str) -> SecretError {
    match e {
        "" => SecretError::Failed(String::from("no answer from the Secret Service")),
        "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" => SecretError::Unavailable,
        // the bus could not start the service (activation)
        _ if e.starts_with("org.freedesktop.DBus.Error.Spawn.") => SecretError::Unavailable,
        _ => SecretError::Failed(String::from(e)),
    }
}

// a method call -> the reply's body, Err(error name; "" when no answer came)
fn call(c: &mut Conn, dest: &str, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Result<Vec<Value>, String> {
    let serial = c.send(&Msg::call(dest, path, iface, member, body)).ok_or_else(String::new)?;
    let end = crate::time_us() + CALL_MS * 1000;
    loop {
        let now = crate::time_us();
        if now >= end {
            return Err(String::new());
        }
        match c.read(((end - now) / 1000) as i32 + 1) {
            Some(m) if (m.typ == T_RETURN || m.typ == T_ERROR) && m.reply_to == serial => {
                return if m.typ == T_RETURN { Ok(m.body) } else { Err(m.error) };
            }
            Some(_) => {}
            None if c.broken => return Err(String::new()),
            None => {}
        }
    }
}

// ---------------------------------------------------------------- the session

struct Session {
    c: Conn,
    path: String,
    /// None: the plain algorithm (Aes clears its round keys when dropped)
    aes: Option<Aes>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let mut m = Msg::call(DEST, &self.path, "org.freedesktop.Secret.Session", "Close", Vec::new());
        m.flags = F_NO_REPLY;
        self.c.send(&m);
    }
}

// (private, public) of a new Diffie-Hellman key pair; the public value has 128 bytes and no
// leading zero byte (written with or without them it is the same; ksecretd refuses less)
fn dh_pair(m: &Mont) -> ([u8; 128], Vec<u8>) {
    let mut private = [0u8; 128];
    loop {
        crate::fill_random(&mut private);
        let public = to_be(&m.pow_ct(&[2], &private), 128);
        if public[0] != 0 {
            return (private, public);
        }
    }
}

/// why the service's public value gives no cipher
enum DhErr {
    /// not in 2 .. p - 2
    Invalid,
    /// the shared value starts with a zero byte: services hash it with or without it
    LeadingZero,
}

// the session's cipher from the service's public value (leading zero bytes, ksecretd's sign
// byte, are skipped)
fn dh_key(m: &Mont, private: &[u8; 128], peer: &[u8]) -> Result<Aes, DhErr> {
    let peer = &peer[peer.iter().position(|&b| b != 0).unwrap_or(peer.len())..];
    if peer.len() > 128 {
        return Err(DhErr::Invalid);
    }
    let y = from_be(peer, 16);
    let mut top = from_be(&PRIME, 16);
    top[0] -= 1; // p - 1 (p is odd)
    if crypto::bn::cmp(&y, &[1]) != core::cmp::Ordering::Greater || crypto::bn::cmp(&y, &top) != core::cmp::Ordering::Less {
        return Err(DhErr::Invalid);
    }
    let mut shared = to_be(&m.pow_ct(&y, private), 128);
    if shared[0] == 0 {
        crypto::zeroize(&mut shared);
        return Err(DhErr::LeadingZero);
    }
    let mut key = [0u8; 16];
    crypto::hkdf_sha256(&[], &shared, &[], &mut key);
    crypto::zeroize(&mut shared);
    let aes = Aes::new(&key);
    crypto::zeroize(&mut key);
    aes.ok_or(DhErr::Invalid)
}

fn open() -> Result<Session, SecretError> {
    let mut c = Conn::session().ok_or(SecretError::Unavailable)?;
    let m = Mont::new(&from_be(&PRIME, 16)).ok_or(SecretError::Invalid)?;
    let mut tries = 0;
    let s = loop {
        let (mut private, public) = dh_pair(&m);
        let r = call(&mut c, DEST, ROOT, I_SERVICE, "OpenSession", vec![Value::str(ALGO), Value::var(bytes(&public))]);
        match r {
            Ok(b) => {
                let k = b.first().and_then(as_bytes).map_or(Err(DhErr::Invalid), |peer| dh_key(&m, &private, &peer));
                crypto::zeroize(&mut private);
                let path = as_path(b.get(1));
                match k {
                    Ok(aes) => break Session { c, path, aes: Some(aes) },
                    // a shared value with a leading zero (1 in 256): close it, a new pair
                    Err(DhErr::LeadingZero) if tries < 8 => {
                        tries += 1;
                        let mut close = Msg::call(DEST, &path, "org.freedesktop.Secret.Session", "Close", Vec::new());
                        close.flags = F_NO_REPLY;
                        c.send(&close);
                    }
                    Err(_) => {
                        drop(Session { c, path, aes: None }); // closes it
                        return Err(SecretError::Failed(String::from("the Secret Service sent an invalid public key")));
                    }
                }
            }
            Err(e) if e == E_UNSUPPORTED => {
                crypto::zeroize(&mut private);
                let b = call(&mut c, DEST, ROOT, I_SERVICE, "OpenSession", vec![Value::str("plain"), Value::var(Value::str(""))]).map_err(|e| fail(&e))?;
                break Session { c, path: as_path(b.get(1)), aes: None };
            }
            Err(e) => {
                crypto::zeroize(&mut private);
                return Err(fail(&e));
            }
        }
    };
    if s.path == "/" {
        return Err(SecretError::Failed(String::from("OpenSession returned no session")));
    }
    Ok(s)
}

impl Session {
    fn call(&mut self, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Result<Vec<Value>, SecretError> {
        call(&mut self.c, DEST, path, iface, member, body).map_err(|e| fail(&e))
    }

    // (oayays): the session, the IV, the value (encrypted), the content type
    fn seal(&self, data: &[u8]) -> Value {
        let ctype = if core::str::from_utf8(data).is_ok() { "text/plain" } else { "application/octet-stream" };
        let (iv, value) = match &self.aes {
            None => (Vec::new(), data.to_vec()),
            Some(aes) => {
                let mut iv = [0u8; 16];
                crate::fill_random(&mut iv);
                let pad = 16 - data.len() % 16;
                let mut buf = data.to_vec();
                buf.resize(data.len() + pad, pad as u8);
                let mut prev = iv;
                for block in buf.chunks_mut(16) {
                    let mut b = [0u8; 16];
                    for i in 0..16 {
                        b[i] = block[i] ^ prev[i];
                    }
                    aes.encrypt(&mut b);
                    block.copy_from_slice(&b);
                    prev = b;
                }
                (iv.to_vec(), buf)
            }
        };
        Value::R(vec![Value::O(self.path.clone()), bytes(&iv), bytes(&value), Value::str(ctype)])
    }

    // the value of a (oayays) the service sent
    fn unseal(&self, secret: &Value) -> Option<Vec<u8>> {
        let Value::R(f) = secret else { return None };
        let iv = as_bytes(f.get(1)?)?;
        let mut value = as_bytes(f.get(2)?)?;
        let Some(aes) = &self.aes else { return Some(value) };
        if iv.len() != 16 || value.is_empty() || value.len() % 16 != 0 {
            return None;
        }
        let mut prev = [0u8; 16];
        prev.copy_from_slice(&iv);
        for block in value.chunks_mut(16) {
            let mut b = [0u8; 16];
            b.copy_from_slice(block);
            let cipher = b;
            aes.decrypt(&mut b);
            for i in 0..16 {
                block[i] = b[i] ^ prev[i];
            }
            prev = cipher;
        }
        let pad = *value.last()? as usize;
        if pad == 0 || pad > 16 || value[value.len() - pad..].iter().any(|&b| b as usize != pad) {
            crypto::zeroize(&mut value);
            return None;
        }
        value.truncate(value.len() - pad);
        Some(value)
    }

    // run a prompt -> its result; the user's dismissal or a timeout -> Dismissed
    fn prompt(&mut self, path: &str) -> Result<Value, SecretError> {
        let rule = format!("type='signal',interface='{I_PROMPT}',member='Completed',path='{path}'");
        call(&mut self.c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", vec![Value::str(&rule)]).map_err(|e| fail(&e))?;
        let serial = self.c.send(&Msg::call(DEST, path, I_PROMPT, "Prompt", vec![Value::str("")])).ok_or_else(|| fail(""))?;
        let end = crate::time_us() + PROMPT_SECONDS as u64 * 1_000_000;
        loop {
            let now = crate::time_us();
            if now >= end {
                let mut m = Msg::call(DEST, path, I_PROMPT, "Dismiss", Vec::new());
                m.flags = F_NO_REPLY;
                self.c.send(&m);
                return Err(SecretError::Dismissed);
            }
            match self.c.read(((end - now) / 1000).min(i32::MAX as u64) as i32 + 1) {
                Some(m) if m.typ == T_ERROR && m.reply_to == serial => return Err(fail(&m.error)),
                Some(m) if m.typ == T_SIGNAL && m.iface == I_PROMPT && m.member == "Completed" && m.path == path => {
                    if matches!(m.body.first(), Some(Value::B(true))) {
                        return Err(SecretError::Dismissed);
                    }
                    return Ok(m.body.get(1).cloned().unwrap_or(Value::B(false)));
                }
                Some(_) => {}
                None if self.c.broken => return Err(fail("")),
                None => {}
            }
        }
    }

    // a prompt path the service returned ("/": none)
    fn maybe_prompt(&mut self, path: &str) -> Result<Option<Value>, SecretError> {
        if path == "/" { Ok(None) } else { self.prompt(path).map(Some) }
    }

    fn unlock(&mut self, objects: &[String]) -> Result<(), SecretError> {
        let list = Value::A(String::from("o"), objects.iter().map(|o| Value::O(o.clone())).collect());
        let b = self.call(ROOT, I_SERVICE, "Unlock", vec![list])?;
        self.maybe_prompt(&as_path(b.get(1)))?;
        Ok(())
    }

    // the default collection, created (with its prompt) when there is none
    fn collection(&mut self) -> Result<String, SecretError> {
        let b = self.call(ROOT, I_SERVICE, "ReadAlias", vec![Value::str("default")])?;
        let p = as_path(b.first());
        if p != "/" {
            return Ok(p);
        }
        let props = Value::dict(vec![("org.freedesktop.Secret.Collection.Label", Value::str("Default keyring"))]);
        let b = self.call(ROOT, I_SERVICE, "CreateCollection", vec![props, Value::str("default")])?;
        let p = as_path(b.first());
        if p != "/" {
            return Ok(p);
        }
        let r = self.maybe_prompt(&as_path(b.get(1)))?;
        let p = as_path(r.as_ref());
        if p == "/" {
            return Err(SecretError::Failed(String::from("no default collection")));
        }
        Ok(p)
    }

    // (unlocked, locked) items of the entry
    fn search(&mut self, n: &Name) -> Result<(Vec<String>, Vec<String>), SecretError> {
        let b = self.call(ROOT, I_SERVICE, "SearchItems", vec![attributes(n, false)])?;
        Ok((paths(b.first()), paths(b.get(1))))
    }
}

// ---------------------------------------------------------------- secret.rs

pub fn available() -> bool {
    let Some(mut c) = Conn::session() else { return false };
    let mut ask = |member: &str, body: Vec<Value>| call(&mut c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", member, body);
    if matches!(ask("NameHasOwner", vec![Value::str(DEST)]).as_deref(), Ok([Value::B(true), ..])) {
        return true;
    }
    match ask("ListActivatableNames", Vec::new()) {
        Ok(b) => matches!(b.first(), Some(Value::A(_, names)) if names.iter().any(|v| v.as_str() == Some(DEST))),
        Err(_) => false,
    }
}

pub fn set(n: &Name, secret: &[u8]) -> Result<(), SecretError> {
    let mut s = open()?;
    let col = s.collection()?;
    let props = Value::dict(vec![("org.freedesktop.Secret.Item.Label", Value::str(&label(n))), ("org.freedesktop.Secret.Item.Attributes", attributes(n, true))]);
    let value = s.seal(secret);
    for again in [false, true] {
        match call(&mut s.c, DEST, &col, I_COLLECTION, "CreateItem", vec![props.clone(), value.clone(), Value::B(true)]) {
            Ok(b) => {
                s.maybe_prompt(&as_path(b.get(1)))?;
                return Ok(());
            }
            Err(e) if e == E_LOCKED && !again => s.unlock(core::slice::from_ref(&col))?,
            Err(e) => return Err(fail(&e)),
        }
    }
    Err(fail(E_LOCKED))
}

pub fn get(n: &Name) -> Result<Option<Vec<u8>>, SecretError> {
    let mut s = open()?;
    let (unlocked, locked) = s.search(n)?;
    let item = match (unlocked.first(), locked.first()) {
        (Some(i), _) => i.clone(),
        (None, Some(i)) => {
            let i = i.clone();
            s.unlock(&locked)?;
            i
        }
        (None, None) => return Ok(None),
    };
    let session = Value::O(s.path.clone());
    let b = s.call(&item, I_ITEM, "GetSecret", vec![session])?;
    let v = b.first().and_then(|v| s.unseal(v));
    v.map(Some).ok_or_else(|| SecretError::Failed(String::from("the Secret Service sent a value that does not decrypt")))
}

pub fn delete(n: &Name) -> Result<bool, SecretError> {
    let mut s = open()?;
    let (unlocked, locked) = s.search(n)?;
    if !locked.is_empty() {
        s.unlock(&locked)?;
    }
    let mut any = false;
    for item in unlocked.iter().chain(&locked) {
        let b = s.call(item, I_ITEM, "Delete", Vec::new())?;
        s.maybe_prompt(&as_path(b.first()))?;
        any = true;
    }
    Ok(any)
}
