// aes -- AES-128 / AES-256 (FIPS 197) in constant time
//
//   Aes::new(key) (16 or 32 bytes)    aes.encrypt4(&mut [[u8; 16]; 4])    aes.encrypt(&mut block)
//                                     aes.decrypt4(&mut [[u8; 16]; 4])    aes.decrypt(&mut block)
//
// Bitsliced: four blocks at a time in eight 64-bit words, word i holding bit i of all 64 bytes
// (bit position = row * 16 + column * 4 + block).  SubBytes inverts in GF(2^8) with
// multiplications on those bit planes (x^254 by four multiplications and seven squarings),
// then applies the affine map; ShiftRows rotates inside the 16-bit row fields, MixColumns
// rotates whole rows.  No table lookups, no branches on data or key: what GCM needs for
// TLS.  The inverse cipher (CBC of the Secret Service, plat::secret) undoes the affine map
// before the same inversion; InvMixColumns is MixColumns after a_r ^= 4 (a_r ^ a_(r+2)).

use alloc::vec::Vec;

type Planes = [u64; 8];

/// bit-planes of four blocks
fn pack(blocks: &[[u8; 16]; 4]) -> Planes {
    let mut q = [0u64; 8];
    for (blk, b) in blocks.iter().enumerate() {
        for (k, &byte) in b.iter().enumerate() {
            let pos = (k % 4) * 16 + (k / 4) * 4 + blk;
            for (i, plane) in q.iter_mut().enumerate() {
                *plane |= ((byte as u64 >> i) & 1) << pos;
            }
        }
    }
    q
}

fn unpack(q: &Planes, blocks: &mut [[u8; 16]; 4]) {
    for (blk, b) in blocks.iter_mut().enumerate() {
        for (k, byte) in b.iter_mut().enumerate() {
            let pos = (k % 4) * 16 + (k / 4) * 4 + blk;
            let mut v = 0u8;
            for (i, plane) in q.iter().enumerate() {
                v |= (((plane >> pos) & 1) as u8) << i;
            }
            *byte = v;
        }
    }
}

/// reduce a product of degree <= 14 modulo x^8 + x^4 + x^3 + x + 1
#[inline(always)]
fn reduce(c: &mut [u64; 15]) -> Planes {
    for k in (8..15).rev() {
        let t = c[k];
        c[k - 4] ^= t;
        c[k - 5] ^= t;
        c[k - 7] ^= t;
        c[k - 8] ^= t;
    }
    [c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]
}

#[inline(always)]
fn gmul(a: &Planes, b: &Planes) -> Planes {
    let mut c = [0u64; 15];
    for i in 0..8 {
        for j in 0..8 {
            c[i + j] ^= a[i] & b[j];
        }
    }
    reduce(&mut c)
}

#[inline(always)]
fn gsq(a: &Planes) -> Planes {
    let mut c = [0u64; 15];
    for i in 0..8 {
        c[2 * i] = a[i];
    }
    reduce(&mut c)
}

// x^254 = x^-1 (and 0 for 0): x^2, x^3, x^12, x^15, x^240, x^252, x^254
fn gf_inv(q: &Planes) -> Planes {
    let x2 = gsq(q);
    let x3 = gmul(&x2, q);
    let x12 = gsq(&gsq(&x3));
    let x15 = gmul(&x12, &x3);
    let x240 = gsq(&gsq(&gsq(&gsq(&x15))));
    let x252 = gmul(&x240, &x12);
    gmul(&x252, &x2)
}

fn sub_bytes(q: &mut Planes) {
    let inv = gf_inv(q);
    // affine map: b_i ^ b_(i+4) ^ b_(i+5) ^ b_(i+6) ^ b_(i+7) ^ 0x63
    for i in 0..8 {
        let mut v = inv[i] ^ inv[(i + 4) % 8] ^ inv[(i + 5) % 8] ^ inv[(i + 6) % 8] ^ inv[(i + 7) % 8];
        if 0x63 >> i & 1 == 1 {
            v = !v;
        }
        q[i] = v;
    }
}

fn inv_sub_bytes(q: &mut Planes) {
    // the inverse affine map: b_(i+2) ^ b_(i+5) ^ b_(i+7) ^ 0x05, then the inversion
    let b: Planes = core::array::from_fn(|i| {
        let v = q[(i + 2) % 8] ^ q[(i + 5) % 8] ^ q[(i + 7) % 8];
        if 0x05 >> i & 1 == 1 { !v } else { v }
    });
    *q = gf_inv(&b);
}

fn shift_rows(q: &mut Planes) {
    for x in q.iter_mut() {
        let v = *x;
        *x = (v & 0xFFFF)
            | ((v >> 4) & 0x0FFF_0000)
            | ((v << 12) & 0xF000_0000)
            | ((v >> 8) & 0x0000_00FF_0000_0000)
            | ((v << 8) & 0x0000_FF00_0000_0000)
            | ((v >> 12) & 0x000F_0000_0000_0000)
            | ((v << 4) & 0xFFF0_0000_0000_0000);
    }
}

// row r takes column c - r (the rotations of shift_rows the other way round)
fn inv_shift_rows(q: &mut Planes) {
    for x in q.iter_mut() {
        let v = *x;
        *x = (v & 0xFFFF)
            | ((v << 4) & 0xFFF0_0000)
            | ((v >> 12) & 0x000F_0000)
            | ((v >> 8) & 0x0000_00FF_0000_0000)
            | ((v << 8) & 0x0000_FF00_0000_0000)
            | ((v >> 4) & 0x0FFF_0000_0000_0000)
            | ((v << 12) & 0xF000_0000_0000_0000);
    }
}

// times x modulo x^8 + x^4 + x^3 + x + 1
fn xtime(t: &Planes) -> Planes {
    [t[7], t[0] ^ t[7], t[1], t[2] ^ t[7], t[3] ^ t[7], t[4], t[5], t[6]]
}

fn mix_columns(q: &mut Planes) {
    let s1: Planes = core::array::from_fn(|i| q[i].rotate_right(16));
    let t: Planes = core::array::from_fn(|i| q[i] ^ s1[i]);
    let x2 = xtime(&t);
    for i in 0..8 {
        q[i] = x2[i] ^ s1[i] ^ q[i].rotate_right(32) ^ q[i].rotate_right(48);
    }
}

fn inv_mix_columns(q: &mut Planes) {
    let w: Planes = core::array::from_fn(|i| q[i] ^ q[i].rotate_right(32));
    let u = xtime(&xtime(&w));
    for i in 0..8 {
        q[i] ^= u[i];
    }
    mix_columns(q);
}

fn add_key(q: &mut Planes, k: &Planes) {
    for (a, b) in q.iter_mut().zip(k) {
        *a ^= b;
    }
}

/// the S-box of four bytes (key expansion), bitsliced like the rounds
fn sub_word(w: [u8; 4]) -> [u8; 4] {
    let mut blocks = [[0u8; 16]; 4];
    blocks[0][..4].copy_from_slice(&w);
    let mut q = pack(&blocks);
    sub_bytes(&mut q);
    unpack(&q, &mut blocks);
    [blocks[0][0], blocks[0][1], blocks[0][2], blocks[0][3]]
}

pub struct Aes {
    /// the round keys, bitsliced for four blocks
    rk: Vec<Planes>,
}

impl Drop for Aes {
    fn drop(&mut self) {
        for k in self.rk.iter_mut() {
            *k = [0; 8];
            core::hint::black_box(&k);
        }
    }
}

impl Aes {
    /// a 16-byte (AES-128) or 32-byte (AES-256) key
    pub fn new(key: &[u8]) -> Option<Aes> {
        let nk = match key.len() {
            16 => 4,
            32 => 8,
            _ => return None,
        };
        let nr = nk + 6;
        let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        let mut rcon = 1u8;
        for i in nk..4 * (nr + 1) {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = sub_word([t[1], t[2], t[3], t[0]]);
                t[0] ^= rcon;
                rcon = (rcon << 1) ^ (0x1B & 0u8.wrapping_sub(rcon >> 7));
            } else if nk > 6 && i % nk == 4 {
                t = sub_word(t);
            }
            let p = w[i - nk];
            w.push([p[0] ^ t[0], p[1] ^ t[1], p[2] ^ t[2], p[3] ^ t[3]]);
        }
        let rk = (0..=nr)
            .map(|r| {
                let mut b = [0u8; 16];
                for c in 0..4 {
                    b[c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
                }
                pack(&[b; 4])
            })
            .collect();
        for x in w.iter_mut() {
            *x = [0; 4];
        }
        Some(Aes { rk })
    }

    /// encrypt four blocks in place
    pub fn encrypt4(&self, blocks: &mut [[u8; 16]; 4]) {
        let nr = self.rk.len() - 1;
        let mut q = pack(blocks);
        add_key(&mut q, &self.rk[0]);
        for r in 1..nr {
            sub_bytes(&mut q);
            shift_rows(&mut q);
            mix_columns(&mut q);
            add_key(&mut q, &self.rk[r]);
        }
        sub_bytes(&mut q);
        shift_rows(&mut q);
        add_key(&mut q, &self.rk[nr]);
        unpack(&q, blocks);
    }

    /// encrypt one block in place
    pub fn encrypt(&self, block: &mut [u8; 16]) {
        let mut b = [*block, [0; 16], [0; 16], [0; 16]];
        self.encrypt4(&mut b);
        *block = b[0];
    }

    /// decrypt four blocks in place
    pub fn decrypt4(&self, blocks: &mut [[u8; 16]; 4]) {
        let nr = self.rk.len() - 1;
        let mut q = pack(blocks);
        add_key(&mut q, &self.rk[nr]);
        for r in (1..nr).rev() {
            inv_shift_rows(&mut q);
            inv_sub_bytes(&mut q);
            add_key(&mut q, &self.rk[r]);
            inv_mix_columns(&mut q);
        }
        inv_shift_rows(&mut q);
        inv_sub_bytes(&mut q);
        add_key(&mut q, &self.rk[0]);
        unpack(&q, blocks);
    }

    /// decrypt one block in place
    pub fn decrypt(&self, block: &mut [u8; 16]) {
        let mut b = [*block, [0; 16], [0; 16], [0; 16]];
        self.decrypt4(&mut b);
        *block = b[0];
    }
}
