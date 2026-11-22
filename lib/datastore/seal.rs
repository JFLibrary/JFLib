// seal -- encryption at rest: the keys of a store or journal and the sealed forms of what they
// write (XChaCha20-Poly1305, a random 24-byte nonce each time)
//
//   keys       from the caller's 32-byte key with BLAKE3 derive_key: one for snapshots, one for
//              the log, one for journals, and a key id (16 bytes, says which key sealed a file
//              without telling anything about the key)
//   snapshot   "BCE1" | key id:16 | seq:u64 | nonce:24 | sealed(the "BCS1" snapshot), the
//              header as AAD
//   log frame  the frame of lib.rs with the magic "BCW2" and as payload nonce:24 | sealed(the
//              JSON of the transaction), AAD "BCW2" | seq; the frame's checksum stays (it finds
//              torn frames without the key)
//   journal    a line "~" + base64url(nonce:24 | sealed(the JSON line)), AAD the journal's
//              name and month
// The key id and the AEAD find another key and every change of a sealed object.  What they
// cannot find: files removed or put back as a whole (an older snapshot, the end of a log cut
// off, lines of a journal deleted) by someone who can write the folder.

use alloc::string::String;
use alloc::vec::Vec;

use crypto::aead;

pub(crate) const SNAP_SEALED: &[u8; 4] = b"BCE1";
pub(crate) const WAL_SEALED: &[u8; 4] = b"BCW2";
const NONCE: usize = 24;
/// what sealing adds to a payload (nonce and tag)
pub(crate) const OVERHEAD: usize = NONCE + 16;
const SNAP_HEAD: usize = 4 + 16 + 8;

/// the keys of a store or journal
pub(crate) struct Keys {
    snap: [u8; 32],
    wal: [u8; 32],
    journal: [u8; 32],
    pub id: [u8; 16],
}

impl Drop for Keys {
    fn drop(&mut self) {
        for k in [&mut self.snap, &mut self.wal, &mut self.journal] {
            crypto::zeroize(k);
        }
    }
}

/// why a sealed snapshot did not open
pub(crate) enum Unseal {
    /// sealed with another key
    OtherKey,
    /// damaged or changed
    Damaged,
}

impl Keys {
    pub fn derive(key: &[u8; 32]) -> Keys {
        let d = |c: &str| crypto::blake3::derive_key(c, key);
        let full = d("jflib datastore 1 key id");
        let mut id = [0u8; 16];
        id.copy_from_slice(&full[..16]);
        Keys { snap: d("jflib datastore 1 snapshot"), wal: d("jflib datastore 1 log"), journal: d("jflib datastore 1 journal"), id }
    }

    pub fn seal_snapshot(&self, seq: u64, plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(SNAP_HEAD + OVERHEAD + plain.len());
        out.extend_from_slice(SNAP_SEALED);
        out.extend_from_slice(&self.id);
        out.extend_from_slice(&seq.to_le_bytes());
        let nonce: [u8; NONCE] = plat::random_bytes();
        let sealed = aead::x_seal(&self.snap, &nonce, &out[..SNAP_HEAD], plain);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        out
    }

    /// a sealed snapshot -> (the seq of its header, the plain "BCS1" bytes)
    pub fn open_snapshot(&self, b: &[u8]) -> Result<(u64, Vec<u8>), Unseal> {
        if b.len() < SNAP_HEAD + OVERHEAD || &b[..4] != SNAP_SEALED {
            return Err(Unseal::Damaged);
        }
        if !crypto::ct_eq(&b[4..20], &self.id) {
            return Err(Unseal::OtherKey);
        }
        let seq = u64::from_le_bytes([b[20], b[21], b[22], b[23], b[24], b[25], b[26], b[27]]);
        let mut nonce = [0u8; NONCE];
        nonce.copy_from_slice(&b[SNAP_HEAD..SNAP_HEAD + NONCE]);
        let plain = aead::x_open(&self.snap, &nonce, &b[..SNAP_HEAD], &b[SNAP_HEAD + NONCE..]).ok_or(Unseal::Damaged)?;
        Ok((seq, plain))
    }

    fn wal_aad(seq: u64) -> [u8; 12] {
        let mut aad = [0u8; 12];
        aad[..4].copy_from_slice(WAL_SEALED);
        aad[4..].copy_from_slice(&seq.to_le_bytes());
        aad
    }

    pub fn seal_frame(&self, seq: u64, plain: &[u8]) -> Vec<u8> {
        let nonce: [u8; NONCE] = plat::random_bytes();
        let mut out = Vec::with_capacity(OVERHEAD + plain.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&aead::x_seal(&self.wal, &nonce, &Keys::wal_aad(seq), plain));
        out
    }

    pub fn open_frame(&self, seq: u64, payload: &[u8]) -> Option<Vec<u8>> {
        if payload.len() < OVERHEAD {
            return None;
        }
        let mut nonce = [0u8; NONCE];
        nonce.copy_from_slice(&payload[..NONCE]);
        aead::x_open(&self.wal, &nonce, &Keys::wal_aad(seq), &payload[NONCE..])
    }

    fn line_aad(name: &str, month: u32) -> Vec<u8> {
        let mut aad = Vec::from(&b"jflib journal 1\0"[..]);
        aad.extend_from_slice(name.as_bytes());
        aad.push(0);
        aad.extend_from_slice(&month.to_le_bytes());
        aad
    }

    /// a journal line, sealed: "~" and base64url, no newline
    pub fn seal_line(&self, name: &str, month: u32, plain: &[u8]) -> String {
        let nonce: [u8; NONCE] = plat::random_bytes();
        let mut b = Vec::from(&nonce[..]);
        b.extend_from_slice(&aead::x_seal(&self.journal, &nonce, &Keys::line_aad(name, month), plain));
        let mut out = String::from("~");
        out.push_str(&crypto::b64::encode_url(&b));
        out
    }

    pub fn open_line(&self, name: &str, month: u32, line: &[u8]) -> Option<Vec<u8>> {
        let text = core::str::from_utf8(line.strip_prefix(b"~")?).ok()?;
        let b = crypto::b64::decode_url(text)?;
        if b.len() < OVERHEAD {
            return None;
        }
        let mut nonce = [0u8; NONCE];
        nonce.copy_from_slice(&b[..NONCE]);
        aead::x_open(&self.journal, &nonce, &Keys::line_aad(name, month), &b[NONCE..])
    }
}
