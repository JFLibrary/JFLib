// keys -- the server's key material in <data>/keys
//
//   master.key   32 random bytes: seals the secrets in the database (credentials, job
//                passphrases) with XChaCha20-Poly1305
//   signing.key  the Ed25519 seed that signs restore points
//   recovery.pub the X25519 public key of the recovery kit (the private key is shown once)
//
// The files are owner-only.  A server that was set up refuses to start without its keys
// (it could not read its own secrets or prove its restore points).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::{aead, b64, ed25519};

pub struct Keys {
    master: [u8; 32],
    pub signing: ed25519::Keypair,
    dir: String,
}

fn load32(path: &str) -> Option<[u8; 32]> {
    let b = plat::read_file(path)?;
    let mut k = [0u8; 32];
    (b.len() == 32).then(|| {
        k.copy_from_slice(&b);
        k
    })
}

impl Keys {
    /// load the keys; create them when `create` (a server that is not set up yet)
    pub fn load(dir: &str, create: bool) -> Result<Keys, String> {
        if !plat::fs::mkdir_private(dir) {
            return Err(format!("cannot create {}", dir));
        }
        let mp = plat::fs::join(dir, "master.key");
        let sp = plat::fs::join(dir, "signing.key");
        let master = match load32(&mp) {
            Some(k) => k,
            None if create => {
                let k: [u8; 32] = plat::random_bytes();
                if !plat::fs::write_file_private(&mp, &k) {
                    return Err(format!("cannot write {}", mp));
                }
                k
            }
            None => return Err(format!("the master key {} is missing", mp)),
        };
        let seed = match load32(&sp) {
            Some(k) => k,
            None if create => {
                let k: [u8; 32] = plat::random_bytes();
                if !plat::fs::write_file_private(&sp, &k) {
                    return Err(format!("cannot write {}", sp));
                }
                k
            }
            None => return Err(format!("the signing key {} is missing", sp)),
        };
        Ok(Keys { master, signing: ed25519::Keypair::from_seed(&seed), dir: String::from(dir) })
    }

    /// seal a secret: "s1:" + base64url(nonce ‖ ciphertext ‖ tag); `aad` binds it to its
    /// place (e.g. "credential:<id>")
    pub fn seal(&self, plain: &[u8], aad: &str) -> String {
        let nonce: [u8; 24] = plat::random_bytes();
        let mut out = Vec::from(&nonce[..]);
        out.extend_from_slice(&aead::x_seal(&self.master, &nonce, aad.as_bytes(), plain));
        format!("s1:{}", b64::encode_url(&out))
    }

    pub fn unseal(&self, sealed: &str, aad: &str) -> Option<Vec<u8>> {
        let b = b64::decode_url(sealed.strip_prefix("s1:")?)?;
        if b.len() < 24 + 16 {
            return None;
        }
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&b[..24]);
        aead::x_open(&self.master, &nonce, aad.as_bytes(), &b[24..])
    }

    /// a key for a purpose, derived from the master key (BLAKE3 derive_key)
    pub fn derive(&self, context: &str) -> [u8; 32] {
        crypto::blake3::derive_key(context, &self.master)
    }

    pub fn dir(&self) -> &str {
        &self.dir
    }

    /// the recovery kit's public key (None: no kit yet)
    pub fn recovery_public(&self) -> Option<[u8; 32]> {
        load32(&plat::fs::join(&self.dir, "recovery.pub"))
    }

    pub fn set_recovery_public(&self, pk: &[u8; 32]) -> bool {
        plat::fs::write_file_private(&plat::fs::join(&self.dir, "recovery.pub"), pk)
    }
}
