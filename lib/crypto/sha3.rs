// sha3 -- SHA-3 and SHAKE (FIPS 202), streaming and one-shot
//
//   Sha3_224 / Sha3_256 / Sha3_384 / Sha3_512   new, update, finalize; sha3_224(data) .. sha3_512
//   Shake128 / Shake256                         new, update, finalize_xof -> ShakeReader::read(out)
//                                               (any length, read in as many calls as wanted);
//                                               shake128(data, out) / shake256(data, out)
//
// Keccak-f[1600] on 25 64-bit lanes: no tables, no branches on the data.  The state is
// overwritten when a hash is dropped (ML-KEM and HMAC put secrets through it).

use crate::Hash;

const RC: [u64; 24] = [
    0x0000_0000_0000_0001, 0x0000_0000_0000_8082, 0x8000_0000_0000_808a, 0x8000_0000_8000_8000,
    0x0000_0000_0000_808b, 0x0000_0000_8000_0001, 0x8000_0000_8000_8081, 0x8000_0000_0000_8009,
    0x0000_0000_0000_008a, 0x0000_0000_0000_0088, 0x0000_0000_8000_8009, 0x0000_0000_8000_000a,
    0x0000_0000_8000_808b, 0x8000_0000_0000_008b, 0x8000_0000_0000_8089, 0x8000_0000_0000_8003,
    0x8000_0000_0000_8002, 0x8000_0000_0000_0080, 0x0000_0000_0000_800a, 0x8000_0000_8000_000a,
    0x8000_0000_8000_8081, 0x8000_0000_0000_8080, 0x0000_0000_8000_0001, 0x8000_0000_8000_8008,
];

/// rho's rotations along pi's path from lane 1
const ROT: [u32; 24] = [1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44];
const PI: [usize; 24] = [10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1];

/// Keccak-f[1600]: theta, rho and pi, chi, iota, 24 rounds
fn keccak_f(a: &mut [u64; 25]) {
    for rc in RC {
        let mut c = [0u64; 5];
        for (x, cx) in c.iter_mut().enumerate() {
            *cx = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in (0..25).step_by(5) {
                a[y + x] ^= d;
            }
        }
        let mut t = a[1];
        for (&j, &r) in PI.iter().zip(&ROT) {
            let next = a[j];
            a[j] = t.rotate_left(r);
            t = next;
        }
        for y in (0..25).step_by(5) {
            let row = [a[y], a[y + 1], a[y + 2], a[y + 3], a[y + 4]];
            for x in 0..5 {
                a[y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
            }
        }
        a[0] ^= rc;
    }
}

/// the sponge: `rate` bytes a block (a multiple of 8), `pos` the next byte of the block
#[derive(Clone)]
struct Sponge {
    a: [u64; 25],
    rate: usize,
    pos: usize,
}

impl Drop for Sponge {
    fn drop(&mut self) {
        crate::zeroize_words(&mut self.a);
    }
}

impl Sponge {
    const fn new(rate: usize) -> Sponge {
        Sponge { a: [0; 25], rate, pos: 0 }
    }

    fn absorb(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.pos == 0 && data.len() >= self.rate {
                for (lane, c) in self.a.iter_mut().zip(data[..self.rate].chunks_exact(8)) {
                    *lane ^= crate::le64(c);
                }
                keccak_f(&mut self.a);
                data = &data[self.rate..];
                continue;
            }
            let take = (self.rate - self.pos).min(data.len());
            for &b in &data[..take] {
                self.a[self.pos / 8] ^= (b as u64) << (8 * (self.pos % 8));
                self.pos += 1;
            }
            data = &data[take..];
            if self.pos == self.rate {
                keccak_f(&mut self.a);
                self.pos = 0;
            }
        }
    }

    /// the padding: the domain's bits (SHA-3 0x06, SHAKE 0x1F), the last bit of the block;
    /// then the output is read from the start of the block
    fn pad(&mut self, domain: u8) {
        self.a[self.pos / 8] ^= (domain as u64) << (8 * (self.pos % 8));
        let last = self.rate - 1;
        self.a[last / 8] ^= 0x80 << (8 * (last % 8));
        keccak_f(&mut self.a);
        self.pos = 0;
    }

    fn squeeze(&mut self, out: &mut [u8]) {
        for o in out.iter_mut() {
            if self.pos == self.rate {
                keccak_f(&mut self.a);
                self.pos = 0;
            }
            *o = (self.a[self.pos / 8] >> (8 * (self.pos % 8))) as u8;
            self.pos += 1;
        }
    }
}

macro_rules! sha3 {
    ($name:ident, $f:ident, $rate:expr, $out:expr, $bits:expr) => {
        #[doc = concat!("SHA3-", $bits)]
        #[derive(Clone)]
        #[allow(non_camel_case_types)]
        pub struct $name(Sponge);

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $name {
            pub const fn new() -> $name {
                $name(Sponge::new($rate))
            }

            pub fn update(&mut self, data: &[u8]) {
                self.0.absorb(data)
            }

            pub fn finalize(mut self) -> [u8; $out] {
                self.0.pad(0x06);
                let mut out = [0u8; $out];
                self.0.squeeze(&mut out);
                out
            }
        }

        impl Hash for $name {
            const BLOCK: usize = $rate;
            const OUT: usize = $out;
            fn new() -> Self {
                $name::new()
            }
            fn update(&mut self, data: &[u8]) {
                self.0.absorb(data)
            }
            fn finish_into(self, out: &mut [u8]) {
                out[..$out].copy_from_slice(&self.finalize());
            }
        }

        #[doc = concat!("SHA3-", $bits, " of a byte string")]
        pub fn $f(data: &[u8]) -> [u8; $out] {
            let mut h = $name::new();
            h.update(data);
            h.finalize()
        }
    };
}

sha3!(Sha3_224, sha3_224, 144, 28, "224");
sha3!(Sha3_256, sha3_256, 136, 32, "256");
sha3!(Sha3_384, sha3_384, 104, 48, "384");
sha3!(Sha3_512, sha3_512, 72, 64, "512");

/// the output of a SHAKE: read as much as wanted, in pieces or at once (the same bytes)
#[derive(Clone)]
pub struct ShakeReader(Sponge);

impl ShakeReader {
    /// the next out.len() bytes
    pub fn read(&mut self, out: &mut [u8]) {
        self.0.squeeze(out)
    }
}

macro_rules! shake {
    ($name:ident, $f:ident, $rate:expr, $bits:expr) => {
        #[doc = concat!("SHAKE", $bits, ": update, then finalize_xof and read any length")]
        #[derive(Clone)]
        pub struct $name(Sponge);

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $name {
            pub const fn new() -> $name {
                $name(Sponge::new($rate))
            }

            pub fn update(&mut self, data: &[u8]) {
                self.0.absorb(data)
            }

            pub fn finalize_xof(mut self) -> ShakeReader {
                self.0.pad(0x1F);
                ShakeReader(self.0)
            }
        }

        #[doc = concat!("SHAKE", $bits, " of a byte string, out.len() bytes")]
        pub fn $f(data: &[u8], out: &mut [u8]) {
            let mut h = $name::new();
            h.update(data);
            h.finalize_xof().read(out)
        }
    };
}

shake!(Shake128, shake128, 168, "128");
shake!(Shake256, shake256, 136, "256");
