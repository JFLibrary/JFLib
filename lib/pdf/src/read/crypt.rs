// The standard security handler (ISO 32000-1, 7.6.3; ISO 32000-2, 7.6.4): RC4 with 40 to 128
// bit keys (revisions 2 to 4), AES-128 through crypt filters (revision 4, AESV2), AES-256
// (revision 6 of PDF 2.0, and revision 5 of Adobe's extension level 3).  The password is tried
// as the user password, then as the owner password; the empty password opens what most files
// with only permissions set.  Passwords of revisions 5 and 6 are UTF-8 (SASLprep is not
// applied: passwords with characters it changes need the form it gives), older ones
// PDFDocEncoding (characters outside Latin-1 become "?").

use super::object::{Dict, Name, Object, Ref};
use super::Error;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crypto::aes::Aes;

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53,
    0x69, 0x7A,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    None,
    Rc4,
    Aes128,
    Aes256,
}

pub(crate) struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    files: Method,
    filters: BTreeMap<Name, Method>,
    pub encrypt_metadata: bool,
    /// the /P permissions, and whether the owner password opened the file
    pub permissions: u32,
    pub owner: bool,
    pub revision: u8,
}

fn rc4(key: &[u8], data: &mut [u8]) {
    if key.is_empty() {
        return;
    }
    let mut s: [u8; 256] = core::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    for b in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(s[i as usize]);
        s.swap(i as usize, j as usize);
        *b ^= s[s[i as usize].wrapping_add(s[j as usize]) as usize];
    }
}

fn aes_cbc_decrypt(key: &[u8], iv: &[u8; 16], data: &mut [u8]) -> Option<()> {
    let aes = Aes::new(key)?;
    let mut prev = *iv;
    for block in data.chunks_exact_mut(16) {
        let mut b: [u8; 16] = block.try_into().ok()?;
        let save = b;
        aes.decrypt(&mut b);
        for k in 0..16 {
            block[k] = b[k] ^ prev[k];
        }
        prev = save;
    }
    Some(())
}

fn aes_cbc_encrypt(aes: &Aes, iv: &[u8], data: &mut [u8]) {
    let mut prev: [u8; 16] = iv.try_into().unwrap_or([0; 16]);
    for block in data.chunks_exact_mut(16) {
        for k in 0..16 {
            prev[k] ^= block[k];
        }
        aes.encrypt(&mut prev);
        block.copy_from_slice(&prev);
    }
}

/// decrypt with AES-CBC, the IV in front, PKCS#7 padding removed when it is valid
fn aes_open(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 16 {
        return Vec::new();
    }
    let iv: [u8; 16] = data[..16].try_into().unwrap_or([0; 16]);
    let mut out = data[16..16 + (data.len() - 16) / 16 * 16].to_vec();
    if aes_cbc_decrypt(key, &iv, &mut out).is_none() {
        return Vec::new();
    }
    if let Some(&p) = out.last() {
        let p = p as usize;
        if (1..=16).contains(&p) && p <= out.len() && out[out.len() - p..].iter().all(|&c| c as usize == p) {
            out.truncate(out.len() - p);
        }
    }
    out
}

fn bytes(d: &Dict, k: &str) -> Vec<u8> {
    d.get(k).and_then(Object::as_bytes).map(<[u8]>::to_vec).unwrap_or_default()
}

/// ISO 32000-2 Algorithm 2.B: the hash of revision 6 (revision 5: one SHA-256)
fn hash6(pw: &[u8], salt: &[u8], udata: &[u8], r: u8) -> [u8; 32] {
    let mut first = Vec::with_capacity(pw.len() + salt.len() + udata.len());
    first.extend_from_slice(pw);
    first.extend_from_slice(salt);
    first.extend_from_slice(udata);
    let mut k: Vec<u8> = crypto::sha256(&first).to_vec();
    if r == 6 {
        let mut round = 0u32;
        loop {
            let unit = pw.len() + k.len() + udata.len();
            let mut e = Vec::with_capacity(unit * 64);
            for _ in 0..64 {
                e.extend_from_slice(pw);
                e.extend_from_slice(&k);
                e.extend_from_slice(udata);
            }
            let Some(aes) = Aes::new(&k[..16]) else { break };
            aes_cbc_encrypt(&aes, &k[16..32], &mut e);
            let n = e[..16].iter().map(|&c| c as u32).sum::<u32>() % 3;
            k = match n {
                0 => crypto::sha256(&e).to_vec(),
                1 => crypto::sha384(&e).to_vec(),
                _ => crypto::sha512(&e).to_vec(),
            };
            round += 1;
            if round >= 64 && (*e.last().unwrap_or(&0) as u32) <= round - 32 {
                break;
            }
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    out
}

fn method_of(cf: Option<&Dict>) -> Method {
    match cf.and_then(|d| d.name("CFM")) {
        Some(b"V2") => Method::Rc4,
        Some(b"AESV2") => Method::Aes128,
        Some(b"AESV3") => Method::Aes256,
        _ => Method::None,
    }
}

/// the key of revisions 2 to 4 for a user password (Algorithm 2)
fn key_r4(pw: &[u8], o: &[u8], p: u32, id0: &[u8], r: u8, n: usize, meta: bool) -> Vec<u8> {
    let mut m = crypto::Md5::new();
    let mut padded = [0u8; 32];
    let l = pw.len().min(32);
    padded[..l].copy_from_slice(&pw[..l]);
    padded[l..].copy_from_slice(&PAD[..32 - l]);
    m.update(&padded);
    m.update(&o[..o.len().min(32)]);
    m.update(&p.to_le_bytes());
    m.update(id0);
    if r >= 4 && !meta {
        m.update(&[0xFF; 4]);
    }
    let mut h = m.finalize();
    if r >= 3 {
        for _ in 0..50 {
            h = crypto::md5(&h[..n]);
        }
    }
    h[..n].to_vec()
}

/// Algorithms 4 and 5: does `key` give /U
fn user_ok(key: &[u8], u: &[u8], id0: &[u8], r: u8) -> bool {
    if r == 2 {
        let mut x = PAD;
        rc4(key, &mut x);
        return u.len() >= 32 && x[..] == u[..32];
    }
    let mut x = crypto::md5_parts(&[&PAD[..], id0]);
    rc4(key, &mut x);
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|&c| c ^ i).collect();
        rc4(&k, &mut x);
    }
    u.len() >= 16 && x[..] == u[..16]
}

impl Crypt {
    /// the handler of an /Encrypt dictionary (references resolved), with the first /ID string
    pub fn open(enc: &Dict, id0: &[u8], password: &str) -> Result<Crypt, Error> {
        if !enc.is("Filter", "Standard") {
            return Err(Error::Unsupported("a security handler other than the standard one"));
        }
        let v = enc.int("V").unwrap_or(0);
        let r = enc.int("R").unwrap_or(2).clamp(0, 255) as u8;
        let p = enc.int("P").unwrap_or(-1) as u32;
        let meta = enc.get("EncryptMetadata").and_then(Object::as_bool).unwrap_or(true);
        let (o, u) = (bytes(enc, "O"), bytes(enc, "U"));
        let cfs = enc.get("CF").and_then(Object::as_dict);
        let named = |k: &str| -> Method {
            match enc.name(k) {
                Some(b"Identity") => Method::None,
                Some(n) => method_of(cfs.and_then(|c| c.get_bytes(n)).and_then(Object::as_dict)),
                None => Method::None,
            }
        };
        let (strings, streams, files) = match v {
            1 | 2 => (Method::Rc4, Method::Rc4, Method::Rc4),
            4 | 5 => {
                let s = named("StmF");
                let f = if enc.contains("EFF") { named("EFF") } else { s };
                (named("StrF"), s, f)
            }
            _ => return Err(Error::Unsupported("this encryption version")),
        };
        let mut filters = BTreeMap::new();
        if let Some(c) = cfs {
            for (k, d) in c.iter() {
                filters.insert(k.clone(), method_of(d.as_dict()));
            }
        }
        let mut c = Crypt { key: Vec::new(), strings, streams, files, filters, encrypt_metadata: meta, permissions: p, owner: false, revision: r };
        if r >= 5 {
            // UTF-8, at most 127 bytes
            let mut pw = password.as_bytes();
            if pw.len() > 127 {
                let mut n = 127;
                while !password.is_char_boundary(n) {
                    n -= 1;
                }
                pw = &pw[..n];
            }
            let (oe, ue) = (bytes(enc, "OE"), bytes(enc, "UE"));
            if o.len() < 48 || u.len() < 48 || oe.len() < 32 || ue.len() < 32 {
                return Err(Error::Damaged("the encryption dictionary"));
            }
            let file_key = |h: [u8; 32], wrapped: &[u8]| -> Vec<u8> {
                let mut k = wrapped[..32].to_vec();
                let _ = aes_cbc_decrypt(&h, &[0; 16], &mut k);
                k
            };
            if hash6(pw, &u[32..40], &[], r) == u[..32] {
                c.key = file_key(hash6(pw, &u[40..48], &[], r), &ue);
            } else if hash6(pw, &o[32..40], &u[..48], r) == o[..32] {
                c.key = file_key(hash6(pw, &o[40..48], &u[..48], r), &oe);
                c.owner = true;
            } else {
                return Err(Error::Password);
            }
            return Ok(c);
        }
        // revisions 2 to 4: PDFDocEncoding, Latin-1 as far as it goes
        let pw: Vec<u8> = password.chars().map(|ch| if (ch as u32) < 256 { ch as u8 } else { b'?' }).collect();
        let n = if r == 2 { 5 } else {
            let mut bits = enc.int("Length").unwrap_or(40);
            if v == 4 {
                if let Some(l) = cfs.and_then(|c| c.get("StdCF")).and_then(Object::as_dict).and_then(|d| d.int("Length")) {
                    bits = if l <= 32 { l * 8 } else { l };
                } else {
                    bits = 128;
                }
            }
            (bits / 8).clamp(5, 16) as usize
        };
        if o.len() < 32 {
            return Err(Error::Damaged("the encryption dictionary"));
        }
        let key = key_r4(&pw, &o, p, id0, r, n, meta);
        if user_ok(&key, &u, id0, r) {
            c.key = key;
            return Ok(c);
        }
        // Algorithm 7: the owner password gives the user password
        let mut padded = [0u8; 32];
        let l = pw.len().min(32);
        padded[..l].copy_from_slice(&pw[..l]);
        padded[l..].copy_from_slice(&PAD[..32 - l]);
        let mut h = crypto::md5(&padded);
        if r >= 3 {
            for _ in 0..50 {
                h = crypto::md5(&h[..n]);
            }
        }
        let okey = &h[..n];
        let mut user = o[..32].to_vec();
        if r == 2 {
            rc4(okey, &mut user);
        } else {
            for i in (0..=19u8).rev() {
                let k: Vec<u8> = okey.iter().map(|&c| c ^ i).collect();
                rc4(&k, &mut user);
            }
        }
        let key = key_r4(&user, &o, p, id0, r, n, meta);
        if user_ok(&key, &u, id0, r) {
            c.key = key;
            c.owner = true;
            return Ok(c);
        }
        Err(Error::Password)
    }

    fn object_key(&self, r: Ref, aes: bool) -> Vec<u8> {
        let mut m = crypto::Md5::new();
        m.update(&self.key);
        m.update(&r.num.to_le_bytes()[..3]);
        m.update(&r.generation.to_le_bytes());
        if aes {
            m.update(b"sAlT");
        }
        let h = m.finalize();
        h[..(self.key.len() + 5).min(16)].to_vec()
    }

    fn run(&self, method: Method, r: Ref, data: &[u8]) -> Vec<u8> {
        match method {
            Method::None => data.to_vec(),
            Method::Rc4 => {
                let mut out = data.to_vec();
                rc4(&self.object_key(r, false), &mut out);
                out
            }
            Method::Aes128 => aes_open(&self.object_key(r, true), data),
            Method::Aes256 => aes_open(&self.key, data),
        }
    }

    pub fn string(&self, r: Ref, data: &[u8]) -> Vec<u8> {
        self.run(self.strings, r, data)
    }

    /// a stream's data: `filter` is the name of a /Crypt filter of the stream, `file` an
    /// embedded file (EFF)
    pub fn stream(&self, r: Ref, data: &[u8], filter: Option<&[u8]>, file: bool) -> Vec<u8> {
        let m = match filter {
            Some(b"Identity") => Method::None,
            Some(n) => self.filters.get(n).copied().unwrap_or(self.streams),
            None if file => self.files,
            None => self.streams,
        };
        self.run(m, r, data)
    }

    /// decrypt the strings of an object loaded from the file (not those of a signature's
    /// /Contents, which are never encrypted)
    pub fn strings_in(&self, r: Ref, o: &mut Object, depth: u32) {
        if depth > super::lex::MAX_DEPTH {
            return;
        }
        match o {
            Object::String(s) => *s = self.string(r, s),
            Object::Array(a) => a.iter_mut().for_each(|v| self.strings_in(r, v, depth + 1)),
            Object::Dict(d) => self.dict_strings(r, d, depth),
            Object::Stream(s) => self.dict_strings(r, &mut s.dict, depth),
            _ => {}
        }
    }

    fn dict_strings(&self, r: Ref, d: &mut Dict, depth: u32) {
        let sig = d.contains("ByteRange") && (d.is("Type", "Sig") || d.is("Type", "DocTimeStamp") || d.contains("SubFilter"));
        for (k, v) in d.0.iter_mut() {
            if sig && k.as_bytes() == b"Contents" {
                continue;
            }
            self.strings_in(r, v, depth + 1);
        }
    }
}
