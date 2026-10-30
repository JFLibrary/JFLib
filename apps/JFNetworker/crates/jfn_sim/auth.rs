// auth -- the stored secrets and the local accounts of an IOS-like device: how a password or a
// secret is kept in the running configuration and how it is checked, and the usernames, enable
// secret and enable password a device knows without a RADIUS server (aaa.rs adds AAA and
// RADIUS on top)
//
//   Secret                    a kept password; kind 0 plain text, 7 a reversible scheme of our
//                             own (for "service password-encryption"), 8 PBKDF2-SHA-256 with a
//                             salt (what "enable secret" and "username ... secret" store)
//   Secret::plain / type7 / pbkdf2(plain, salt)   make one; verify(input); reveal (0 and 7:
//                             the plain text); config_word / from_config (the running config)
//   UserAcct { name, privilege, secret }   a local user
//   LocalAuth                 a device's enable secret, enable password and users, and the
//                             "service password-encryption" flag; verify_enable / verify_user
//
// Decisions (documented in the README): a plain password stays readable (kind 0) unless
// "service password-encryption" is set, which rewrites kinds 0 to kind 7, a reversible
// obfuscation of our own (not a vendor's), shown as a hex string.  "enable secret" and
// "username ... secret" are hashed with PBKDF2-SHA-256 (kind 8), a 12-byte salt from the
// device's random stream and 1000 rounds, shown as "8 $8$<salt>$<hash>" (our own base64, URL
// alphabet without padding).  A salt is drawn only when a secret is typed; a configuration that
// boots keeps the stored salt and hash, so a device behaves the same across a save and a
// restart.  The number of rounds is a constant, so the work is the same on every target.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::{b64, ct_eq, pbkdf2_sha256};

/// PBKDF2-SHA-256 rounds of a kind-8 secret (a constant, so the work is the same everywhere)
pub const PBKDF2_ROUNDS: u32 = 1000;
/// the salt of a kind-8 secret (bytes from the device's random stream)
pub const SALT_LEN: usize = 12;
/// the hash kept of a kind-8 secret
pub const HASH_LEN: usize = 32;

/// the keystream of the reversible kind-7 scheme (our own, documented; not a vendor's)
const KEY7: &[u8] = b"jfnetworker-type7-keystream-v1";

/// a password as the running configuration keeps it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Secret {
    /// 0 plain text, 7 reversible, 8 PBKDF2-SHA-256
    pub kind: u8,
    /// kind 0: the plain text; kind 7: the hex of the obfuscation; kind 8: "$8$salt$hash"
    pub text: String,
}

impl Secret {
    /// a plain-text password (kind 0)
    pub fn plain(s: &str) -> Secret {
        Secret { kind: 0, text: String::from(s) }
    }

    /// the reversible kind-7 form of a plain password
    pub fn type7(plain: &str) -> Secret {
        Secret { kind: 7, text: encode7(plain.as_bytes()) }
    }

    /// a PBKDF2-SHA-256 secret of a plain password with a salt (kind 8)
    pub fn pbkdf2(plain: &str, salt: &[u8]) -> Secret {
        let salt = &salt[..salt.len().min(SALT_LEN)];
        let mut hash = [0u8; HASH_LEN];
        pbkdf2_sha256(plain.as_bytes(), salt, PBKDF2_ROUNDS, &mut hash);
        Secret { kind: 8, text: format!("$8${}${}", b64::encode_url(salt), b64::encode_url(&hash)) }
    }

    /// the plain text of a readable secret (kinds 0 and 7), None for a hash
    pub fn reveal(&self) -> Option<String> {
        match self.kind {
            0 => Some(self.text.clone()),
            7 => decode7(&self.text).and_then(|b| String::from_utf8(b).ok()),
            _ => None,
        }
    }

    /// `input` is the right password
    pub fn verify(&self, input: &str) -> bool {
        match self.kind {
            0 => ct_eq(self.text.as_bytes(), input.as_bytes()),
            7 => match decode7(&self.text) {
                Some(b) => ct_eq(&b, input.as_bytes()),
                None => false,
            },
            8 => {
                let Some((salt, hash)) = parse8(&self.text) else {
                    return false;
                };
                let mut got = [0u8; HASH_LEN];
                pbkdf2_sha256(input.as_bytes(), &salt, PBKDF2_ROUNDS, &mut got);
                ct_eq(&got[..hash.len()], &hash)
            }
            _ => false,
        }
    }

    /// kind 8: hashed (nothing to make readable even with "no service password-encryption")
    pub fn is_hash(&self) -> bool {
        self.kind == 8
    }

    /// what follows the keyword in the running configuration ("pass", "7 071a...", "$8$..")
    /// (a kind-0 secret with the implicit "0" left off, as IOS writes it)
    pub fn config_word(&self) -> String {
        match self.kind {
            0 => self.text.clone(),
            k => format!("{} {}", k, self.text),
        }
    }

    /// reads a secret back from a running configuration: an explicit kind ("0 X", "7 X", "8 X")
    /// or a bare word (kind 0)
    pub fn from_config(kind: Option<u8>, word: &str) -> Secret {
        match kind {
            Some(7) => Secret { kind: 7, text: String::from(word) },
            Some(8) => Secret { kind: 8, text: String::from(word) },
            _ => Secret::plain(word),
        }
    }

    /// the same secret rewritten for "service password-encryption" on (kind 0 becomes 7) or off
    /// (kind 7 becomes 0); a hash (kind 8) is left as it is
    pub fn with_encryption(&self, on: bool) -> Secret {
        match (self.kind, on) {
            (0, true) => Secret::type7(&self.text),
            (7, false) => match self.reveal() {
                Some(p) => Secret::plain(&p),
                None => self.clone(),
            },
            _ => self.clone(),
        }
    }
}

/// the reversible kind-7 obfuscation: a start offset into the keystream, then each byte XORed
/// with it, as hex (two digits of the start, then the bytes); our own, so it is reversible
fn encode7(data: &[u8]) -> String {
    // a fixed start keeps the output deterministic (the config stays byte-identical)
    let start = (data.len() as u8).wrapping_mul(7) % KEY7.len() as u8;
    let mut out = alloc::vec![start];
    for (i, &b) in data.iter().enumerate() {
        out.push(b ^ KEY7[(start as usize + i) % KEY7.len()]);
    }
    crypto::hex(&out)
}

fn decode7(hex: &str) -> Option<Vec<u8>> {
    let b = crypto::unhex(hex)?;
    let (&start, rest) = b.split_first()?;
    if start as usize >= KEY7.len() {
        return None;
    }
    Some(rest.iter().enumerate().map(|(i, &c)| c ^ KEY7[(start as usize + i) % KEY7.len()]).collect())
}

/// the salt and hash of a "$8$salt$hash" secret
fn parse8(s: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let rest = s.strip_prefix("$8$")?;
    let (salt, hash) = rest.split_once('$')?;
    Some((b64::decode_url(salt)?, b64::decode_url(hash)?))
}

/// a local user account
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAcct {
    pub name: String,
    /// the privilege level the user gets (0 to 15; the default is 1)
    pub privilege: u8,
    pub secret: Secret,
}

/// the local credentials of a device: enable secret and password per level, the users, and
/// whether readable passwords are obfuscated
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalAuth {
    /// "enable secret [level N]": the hashed secret per level (level 15 is the usual one)
    pub enable_secret: Vec<(u8, Secret)>,
    /// "enable password [level N]": the readable password per level
    pub enable_password: Vec<(u8, Secret)>,
    /// local users ("username ...")
    pub users: Vec<UserAcct>,
    /// "service password-encryption"
    pub password_encryption: bool,
}

impl LocalAuth {
    /// the secret that guards a level ("enable secret" wins over "enable password"); None when
    /// nothing guards it
    pub fn enable_of(&self, level: u8) -> Option<&Secret> {
        self.enable_secret.iter().find(|(l, _)| *l == level).map(|(_, s)| s).or_else(|| self.enable_password.iter().find(|(l, _)| *l == level).map(|(_, s)| s))
    }

    /// the device asks for an enable secret at `level`
    pub fn enable_guarded(&self, level: u8) -> bool {
        self.enable_of(level).is_some()
    }

    /// `input` opens `level`
    pub fn verify_enable(&self, level: u8, input: &str) -> bool {
        self.enable_of(level).is_some_and(|s| s.verify(input))
    }

    /// a user by name (case-sensitive, as IOS)
    pub fn user(&self, name: &str) -> Option<&UserAcct> {
        self.users.iter().find(|u| u.name == name)
    }

    /// the user's privilege level when `input` is their password, else None
    pub fn verify_user(&self, name: &str, input: &str) -> Option<u8> {
        let u = self.user(name)?;
        u.secret.verify(input).then_some(u.privilege)
    }

    /// sets or replaces an "enable secret [level N]"
    pub fn set_enable_secret(&mut self, level: u8, s: Option<Secret>) {
        self.enable_secret.retain(|(l, _)| *l != level);
        if let Some(s) = s {
            self.enable_secret.push((level, s));
            self.enable_secret.sort_by_key(|(l, _)| *l);
        }
    }

    /// sets or replaces an "enable password [level N]"
    pub fn set_enable_password(&mut self, level: u8, s: Option<Secret>) {
        self.enable_password.retain(|(l, _)| *l != level);
        if let Some(s) = s {
            self.enable_password.push((level, s));
            self.enable_password.sort_by_key(|(l, _)| *l);
        }
    }

    /// adds or replaces a user
    pub fn set_user(&mut self, u: UserAcct) {
        self.users.retain(|x| x.name != u.name);
        self.users.push(u);
    }

    pub fn remove_user(&mut self, name: &str) -> bool {
        let n = self.users.len();
        self.users.retain(|x| x.name != name);
        self.users.len() != n
    }

    /// turns "service password-encryption" on or off: readable passwords change kind, hashes
    /// stay (once a password is kind 7 it cannot be read back out and shown as plain... it can,
    /// since kind 7 is reversible)
    pub fn set_password_encryption(&mut self, on: bool) {
        if self.password_encryption == on {
            return;
        }
        self.password_encryption = on;
        for (_, s) in self.enable_password.iter_mut() {
            *s = s.with_encryption(on);
        }
        for u in self.users.iter_mut() {
            u.secret = u.secret.with_encryption(on);
        }
    }

    /// nothing is configured (the device opens as it always has)
    pub fn is_empty(&self) -> bool {
        self.enable_secret.is_empty() && self.enable_password.is_empty() && self.users.is_empty() && !self.password_encryption
    }

    /// the bytes it holds (keyframe::node_bytes)
    pub fn footprint(&self) -> usize {
        let s = |x: &Secret| x.text.len() + 8;
        self.enable_secret.iter().map(|(_, x)| s(x)).sum::<usize>() + self.enable_password.iter().map(|(_, x)| s(x)).sum::<usize>() + self.users.iter().map(|u| u.name.len() + s(&u.secret) + 8).sum::<usize>()
    }
}
