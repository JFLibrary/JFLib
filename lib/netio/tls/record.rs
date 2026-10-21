// record -- the TLS record layer: TLS 1.3 (RFC 8446) and, for public servers, TLS 1.2 (RFC
// 5246 with AEAD suites only: RFC 5288 AES-GCM, RFC 7905 ChaCha20-Poly1305)
//
//   plaintext record    type | 0x0303 | length | fragment          (handshake before keys,
//                                                                    alerts, CCS)
//   TLS 1.3 protected   23 | 0x0303 | length | AEAD(content ‖ type ‖ zero padding)
//                       nonce = IV xor sequence number, additional data = the record header
//   TLS 1.2 protected   type | 0x0303 | length | [explicit nonce (GCM)] AEAD(content)
//                       additional data = sequence ‖ type ‖ 0x0303 ‖ content length;
//                       GCM nonce = 4-byte salt ‖ explicit 8 bytes (we send the sequence),
//                       ChaCha20 nonce = IV xor sequence number

use alloc::boxed::Box;
use alloc::vec::Vec;

use crypto::aead;
use crypto::gcm::AesGcm;

pub const CHANGE_CIPHER_SPEC: u8 = 20;
pub const ALERT: u8 = 21;
pub const HANDSHAKE: u8 = 22;
pub const APPLICATION_DATA: u8 = 23;

/// the largest plaintext fragment
pub const MAX_PLAIN: usize = 1 << 14;
/// the largest record body on the wire (protected: plaintext + type + tag, some slack)
pub const MAX_RECORD: usize = MAX_PLAIN + 256;

/// the cipher suites (TLS 1.3, then TLS 1.2 ECDHE)
pub const TLS_AES_128_GCM_SHA256: u16 = 0x1301;
pub const TLS_CHACHA20_POLY1305_SHA256: u16 = 0x1303;
pub const ECDHE_ECDSA_AES128_GCM: u16 = 0xC02B;
pub const ECDHE_RSA_AES128_GCM: u16 = 0xC02F;
pub const ECDHE_RSA_CHACHA20: u16 = 0xCCA8;
pub const ECDHE_ECDSA_CHACHA20: u16 = 0xCCA9;

/// key and IV lengths of a suite (TLS 1.2: the IV is the implicit part)
pub fn key_iv_len(suite: u16) -> Option<(usize, usize)> {
    match suite {
        TLS_CHACHA20_POLY1305_SHA256 => Some((32, 12)),
        TLS_AES_128_GCM_SHA256 => Some((16, 12)),
        ECDHE_RSA_CHACHA20 | ECDHE_ECDSA_CHACHA20 => Some((32, 12)),
        ECDHE_ECDSA_AES128_GCM | ECDHE_RSA_AES128_GCM => Some((16, 4)),
        _ => None,
    }
}

enum Aead {
    ChaCha([u8; 32]),
    Gcm(Box<AesGcm>),
}

/// one direction's keys
pub struct Cipher {
    aead: Aead,
    iv: [u8; 12],
    seq: u64,
    /// TLS 1.2 framing
    v12: bool,
    /// TLS 1.2 GCM: 4-byte salt + explicit nonce
    explicit: bool,
}

impl Drop for Cipher {
    fn drop(&mut self) {
        if let Aead::ChaCha(k) = &mut self.aead {
            crypto::zeroize(k);
        }
    }
}

impl Cipher {
    /// TLS 1.3 with ChaCha20-Poly1305
    pub fn new(key: [u8; 32], iv: [u8; 12]) -> Cipher {
        Cipher { aead: Aead::ChaCha(key), iv, seq: 0, v12: false, explicit: false }
    }

    /// the keys of a suite (key and IV of key_iv_len); None for an unknown suite
    pub fn for_suite(suite: u16, key: &[u8], iv: &[u8], v12: bool) -> Option<Cipher> {
        let (kl, il) = key_iv_len(suite)?;
        if key.len() != kl || iv.len() != il {
            return None;
        }
        let aead = if kl == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            Aead::ChaCha(k)
        } else {
            Aead::Gcm(Box::new(AesGcm::new(key)?))
        };
        let mut i = [0u8; 12];
        i[..il].copy_from_slice(iv);
        Some(Cipher { aead, iv: i, seq: 0, v12, explicit: il == 4 })
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        if self.explicit {
            n[4..].copy_from_slice(&self.seq.to_be_bytes());
        } else {
            for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
                n[4 + i] ^= b;
            }
        }
        n
    }

    fn seal_raw(&self, nonce: &[u8; 12], aad: &[u8], body: &mut Vec<u8>) -> [u8; 16] {
        match &self.aead {
            Aead::ChaCha(k) => aead::seal_in_place(k, nonce, aad, body),
            Aead::Gcm(g) => g.seal_in_place(nonce, aad, body),
        }
    }

    fn open_raw(&self, nonce: &[u8; 12], aad: &[u8], body: &mut [u8], tag: &[u8; 16]) -> bool {
        match &self.aead {
            Aead::ChaCha(k) => aead::open_in_place(k, nonce, aad, body, tag),
            Aead::Gcm(g) => g.open_in_place(nonce, aad, body, tag),
        }
    }

    fn aad12(&self, ty: u8, len: usize) -> [u8; 13] {
        let mut a = [0u8; 13];
        a[..8].copy_from_slice(&self.seq.to_be_bytes());
        a[8] = ty;
        a[9] = 3;
        a[10] = 3;
        a[11] = (len >> 8) as u8;
        a[12] = len as u8;
        a
    }

    /// a protected record of `content` of type `ty`
    pub fn seal(&mut self, ty: u8, content: &[u8]) -> Vec<u8> {
        if self.v12 {
            let nonce = self.nonce();
            let aad = self.aad12(ty, content.len());
            let mut body = Vec::from(content);
            let tag = self.seal_raw(&nonce, &aad, &mut body);
            let ex = if self.explicit { 8 } else { 0 };
            let len = ex + body.len() + 16;
            let mut out = Vec::with_capacity(5 + len);
            out.extend_from_slice(&[ty, 3, 3, (len >> 8) as u8, len as u8]);
            if self.explicit {
                out.extend_from_slice(&nonce[4..]);
            }
            out.extend_from_slice(&body);
            out.extend_from_slice(&tag);
            self.seq += 1;
            return out;
        }
        let len = content.len() + 1 + 16;
        let mut out = Vec::with_capacity(5 + len);
        out.extend_from_slice(&[APPLICATION_DATA, 3, 3, (len >> 8) as u8, len as u8]);
        let mut body = Vec::with_capacity(len);
        body.extend_from_slice(content);
        body.push(ty);
        let hdr = [out[0], out[1], out[2], out[3], out[4]];
        let tag = self.seal_raw(&self.nonce(), &hdr, &mut body);
        self.seq += 1;
        out.extend_from_slice(&body);
        out.extend_from_slice(&tag);
        out
    }

    /// open a protected record body (header given) -> (type, content)
    pub fn open(&mut self, hdr: &[u8; 5], body: &[u8]) -> Option<(u8, Vec<u8>)> {
        if self.v12 {
            let ex = if self.explicit { 8 } else { 0 };
            if body.len() < ex + 16 || body.len() > MAX_RECORD {
                return None;
            }
            let mut nonce = self.nonce();
            if self.explicit {
                nonce[4..].copy_from_slice(&body[..8]);
            }
            let n = body.len() - 16;
            let mut tag = [0u8; 16];
            tag.copy_from_slice(&body[n..]);
            let mut plain = Vec::from(&body[ex..n]);
            let aad = self.aad12(hdr[0], plain.len());
            if !self.open_raw(&nonce, &aad, &mut plain, &tag) || plain.len() > MAX_PLAIN {
                return None;
            }
            self.seq += 1;
            return Some((hdr[0], plain));
        }
        if body.len() < 17 || body.len() > MAX_RECORD {
            return None;
        }
        let n = body.len() - 16;
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&body[n..]);
        let mut plain = Vec::from(&body[..n]);
        if !self.open_raw(&self.nonce(), hdr, &mut plain, &tag) {
            return None;
        }
        self.seq += 1;
        // strip the zero padding; the last non-zero byte is the real type
        while plain.last() == Some(&0) {
            plain.pop();
        }
        let ty = plain.pop()?;
        if plain.len() > MAX_PLAIN {
            return None;
        }
        Some((ty, plain))
    }
}

/// a plaintext record
pub fn plain(ty: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + content.len());
    // legacy_record_version 0x0301 for the first ClientHello is allowed; 0x0303 is fine too
    out.extend_from_slice(&[ty, 3, 3, (content.len() >> 8) as u8, content.len() as u8]);
    out.extend_from_slice(content);
    out
}
