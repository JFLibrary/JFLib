// cmac -- AES-CMAC (RFC 4493, NIST SP 800-38B): a message authentication code from the AES
// block cipher, as the MIC of WPA3-Personal's 4-way handshake (AKM 8) and of 802.11w use it
//
//   cmac_aes(key, data) -> Option<[u8; 16]>   key: 16 (AES-128) or 32 bytes; None otherwise

use crate::aes::Aes;

// doubling in GF(2^128) with the polynomial of the standard (the subkeys K1, K2)
fn dbl(b: &[u8; 16]) -> [u8; 16] {
    let mut o = [0u8; 16];
    let mut carry = 0u8;
    for i in (0..16).rev() {
        o[i] = (b[i] << 1) | carry;
        carry = b[i] >> 7;
    }
    if carry != 0 {
        o[15] ^= 0x87;
    }
    o
}

/// the CMAC of `data` under `key`
pub fn cmac_aes(key: &[u8], data: &[u8]) -> Option<[u8; 16]> {
    let aes = Aes::new(key)?;
    let mut l = [0u8; 16];
    aes.encrypt(&mut l);
    let k1 = dbl(&l);
    let k2 = dbl(&k1);
    let n = data.len().div_ceil(16).max(1);
    let whole = !data.is_empty() && data.len() % 16 == 0;
    let mut x = [0u8; 16];
    for i in 0..n {
        let mut m = [0u8; 16];
        let chunk = &data[i * 16..(i * 16 + 16).min(data.len())];
        m[..chunk.len()].copy_from_slice(chunk);
        if i == n - 1 {
            if whole {
                for (a, b) in m.iter_mut().zip(&k1) {
                    *a ^= b;
                }
            } else {
                m[chunk.len()] = 0x80;
                for (a, b) in m.iter_mut().zip(&k2) {
                    *a ^= b;
                }
            }
        }
        for (a, b) in x.iter_mut().zip(&m) {
            *a ^= b;
        }
        aes.encrypt(&mut x);
    }
    Some(x)
}
