// mlkem -- ML-KEM (FIPS 203), the module-lattice key encapsulation: ML-KEM-512, -768 and -1024
//
//   Params::MlKem512 / MlKem768 / MlKem1024    .ek_len() .dk_len() .ct_len() (bytes)
//   keygen(params, &seed) -> (ek, dk)          seed = d || z, 64 random bytes (FIPS 203's seed
//                                              form: a key may be kept as these 64 bytes)
//   encaps(params, ek, &m) -> Option<(ct, key)>   m: 32 random bytes; None when ek has the wrong
//                                              length or a coefficient of q or more (FIPS 203 7.2)
//   decaps(params, dk, ct) -> Option<key>      None when a length is wrong or dk's hash of its
//                                              encapsulation key does not match (7.3); a changed
//                                              ciphertext gives the implicit-rejection key
//                                              J(z || ct), which nothing tells apart from a key
//
// The secret paths (keygen's s and e, encryption's y, decapsulation's message and the choice
// between its two keys) have no branch and no table lookup on secret data, and no division:
// reductions modulo q = 3329 are Barrett's with a masked correction, and Compress divides by q
// the same way (KyberSlash, 2024: a division by q compiled to a div instruction whose time
// depends on its operand).  The matrix comes from the public seed rho, so its rejection
// sampling may branch.

use alloc::vec::Vec;

use crate::sha3::{Sha3_256, Sha3_512, Shake128, Shake256};

const N: usize = 256;
const Q: u32 = 3329;
/// floor(2^32 / q) for Barrett's reduction
const M: u64 = (1u64 << 32) / Q as u64;

/// the parameter sets of FIPS 203, table 2
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Params {
    MlKem512,
    MlKem768,
    MlKem1024,
}

struct P {
    k: usize,
    eta1: usize,
    eta2: usize,
    du: u32,
    dv: u32,
}

impl Params {
    fn p(self) -> P {
        match self {
            Params::MlKem512 => P { k: 2, eta1: 3, eta2: 2, du: 10, dv: 4 },
            Params::MlKem768 => P { k: 3, eta1: 2, eta2: 2, du: 10, dv: 4 },
            Params::MlKem1024 => P { k: 4, eta1: 2, eta2: 2, du: 11, dv: 5 },
        }
    }

    /// the encapsulation key's length: 384 k + 32
    pub fn ek_len(self) -> usize {
        384 * self.p().k + 32
    }

    /// the decapsulation key's length: 768 k + 96
    pub fn dk_len(self) -> usize {
        768 * self.p().k + 96
    }

    /// the ciphertext's length: 32 (du k + dv)
    pub fn ct_len(self) -> usize {
        let p = self.p();
        32 * (p.du as usize * p.k + p.dv as usize)
    }
}

// ---------------------------------------------------------------- arithmetic modulo q

const fn pow_q(b: u32, mut e: u32) -> u32 {
    let mut r = 1;
    let mut b = b % Q;
    while e > 0 {
        if e & 1 == 1 {
            r = r * b % Q;
        }
        b = b * b % Q;
        e >>= 1;
    }
    r
}

const fn bitrev7(x: u32) -> u32 {
    let mut r = 0;
    let mut i = 0;
    while i < 7 {
        r |= ((x >> i) & 1) << (6 - i);
        i += 1;
    }
    r
}

/// zeta^BitRev7(i) (zeta = 17, a primitive 256th root of unity modulo q): the NTT's factors
const ZETAS: [u16; 128] = {
    let mut z = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        z[i] = pow_q(17, bitrev7(i as u32)) as u16;
        i += 1;
    }
    z
};

/// zeta^(2 BitRev7(i) + 1): the base case multiplication's factors
const GAMMAS: [u16; 128] = {
    let mut g = [0u16; 128];
    let mut i = 0;
    while i < 128 {
        g[i] = pow_q(17, 2 * bitrev7(i as u32) + 1) as u16;
        i += 1;
    }
    g
};

/// r - q when r >= q, else r (r < 2 q), by mask
#[inline(always)]
fn csub(r: u32) -> u32 {
    let s = r.wrapping_sub(Q);
    let below = crate::ct_mask(((s >> 31) as u64).wrapping_neg()) as u32;
    (r & below) | (s & !below)
}

/// a mod q for any a < 2^32: floor(a M / 2^32) is floor(a / q) or one less
#[inline(always)]
fn reduce(a: u32) -> u16 {
    let t = ((a as u64 * M) >> 32) as u32;
    csub(a - t * Q) as u16
}

/// floor(y / q) for y < 2^32, without a division
#[inline(always)]
fn div_q(y: u32) -> u32 {
    let t = ((y as u64 * M) >> 32) as u32;
    let r = y - t * Q;
    let below = crate::ct_mask((r.wrapping_sub(Q) >> 31) as u64) as u32;
    t + (below ^ 1)
}

#[inline(always)]
fn fmul(a: u16, b: u16) -> u16 {
    reduce(a as u32 * b as u32)
}

#[inline(always)]
fn fadd(a: u16, b: u16) -> u16 {
    csub(a as u32 + b as u32) as u16
}

#[inline(always)]
fn fsub(a: u16, b: u16) -> u16 {
    csub(a as u32 + Q - b as u32) as u16
}

type Poly = [u16; N];

/// NTT (FIPS 203 algorithm 9), in place
fn ntt(f: &mut Poly) {
    let mut i = 1;
    let mut len = 128;
    while len >= 2 {
        for start in (0..N).step_by(2 * len) {
            let zeta = ZETAS[i];
            i += 1;
            for j in start..start + len {
                let t = fmul(zeta, f[j + len]);
                f[j + len] = fsub(f[j], t);
                f[j] = fadd(f[j], t);
            }
        }
        len /= 2;
    }
}

/// the inverse NTT (algorithm 10), in place; 3303 = 128^-1 mod q
fn ntt_inv(f: &mut Poly) {
    let mut i = 127;
    let mut len = 2;
    while len <= 128 {
        for start in (0..N).step_by(2 * len) {
            let zeta = ZETAS[i];
            i -= 1;
            for j in start..start + len {
                let t = f[j];
                f[j] = fadd(t, f[j + len]);
                f[j + len] = fmul(zeta, fsub(f[j + len], t));
            }
        }
        len *= 2;
    }
    for x in f.iter_mut() {
        *x = fmul(*x, 3303);
    }
}

/// acc += f g in the NTT domain (algorithms 11 and 12)
fn mul_acc(acc: &mut Poly, f: &Poly, g: &Poly) {
    for i in 0..128 {
        let (a0, a1, b0, b1) = (f[2 * i], f[2 * i + 1], g[2 * i], g[2 * i + 1]);
        let c0 = fadd(fmul(a0, b0), fmul(fmul(a1, b1), GAMMAS[i]));
        let c1 = fadd(fmul(a0, b1), fmul(a1, b0));
        acc[2 * i] = fadd(acc[2 * i], c0);
        acc[2 * i + 1] = fadd(acc[2 * i + 1], c1);
    }
}

fn zero_polys(p: &mut [Poly]) {
    for f in p.iter_mut() {
        for x in f.iter_mut() {
            *x = 0;
        }
    }
    core::hint::black_box(&p);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

// ---------------------------------------------------------------- encodings and sampling

/// ByteEncode_d (algorithm 5): 256 values of d bits, least significant bit first
fn encode(f: &Poly, d: u32, out: &mut [u8]) {
    let (mut acc, mut bits, mut o) = (0u64, 0u32, 0usize);
    for &a in f.iter() {
        acc |= (a as u64) << bits;
        bits += d;
        while bits >= 8 {
            out[o] = acc as u8;
            o += 1;
            acc >>= 8;
            bits -= 8;
        }
    }
}

/// ByteDecode_d (algorithm 6) without the reduction: the d-bit values as they are
fn decode(b: &[u8], d: u32) -> Poly {
    let mut f = [0u16; N];
    let (mut acc, mut bits, mut at) = (0u64, 0u32, 0usize);
    let mask = (1u64 << d) - 1;
    for x in f.iter_mut() {
        while bits < d {
            acc |= (b[at] as u64) << bits;
            at += 1;
            bits += 8;
        }
        *x = (acc & mask) as u16;
        acc >>= d;
        bits -= d;
    }
    f
}

/// ByteDecode_12 with its reduction modulo q (values below 4096 < 2 q)
fn decode12(b: &[u8]) -> Poly {
    let mut f = decode(b, 12);
    for x in f.iter_mut() {
        *x = csub(*x as u32) as u16;
    }
    f
}

/// Compress_d: round(2^d x / q) mod 2^d = floor((2^d x + 1664) / q) mod 2^d (q odd: no ties)
fn compress(f: &Poly, d: u32) -> Poly {
    let mut r = [0u16; N];
    for (o, &x) in r.iter_mut().zip(f.iter()) {
        *o = (div_q(((x as u32) << d) + 1664) & ((1 << d) - 1)) as u16;
    }
    r
}

/// Decompress_d: round(q y / 2^d)
fn decompress(f: &Poly, d: u32) -> Poly {
    let mut r = [0u16; N];
    for (o, &y) in r.iter_mut().zip(f.iter()) {
        *o = ((y as u32 * Q + (1 << (d - 1))) >> d) as u16;
    }
    r
}

/// SampleNTT (algorithm 7): rejection sampling from SHAKE128(rho || j || i); public input
fn sample_ntt(rho: &[u8], j: u8, i: u8) -> Poly {
    let mut x = Shake128::new();
    x.update(rho);
    x.update(&[j, i]);
    let mut r = x.finalize_xof();
    let mut f = [0u16; N];
    let mut n = 0;
    let mut c = [0u8; 168];
    while n < N {
        r.read(&mut c);
        for t in c.chunks_exact(3) {
            let d1 = t[0] as u32 | ((t[1] as u32 & 15) << 8);
            let d2 = (t[1] as u32 >> 4) | ((t[2] as u32) << 4);
            if d1 < Q && n < N {
                f[n] = d1 as u16;
                n += 1;
            }
            if d2 < Q && n < N {
                f[n] = d2 as u16;
                n += 1;
            }
        }
    }
    f
}

/// SamplePolyCBD_eta (algorithm 8) of PRF_eta(s, b) = SHAKE256(s || b, 64 eta): the difference
/// of two sums of eta bits, counted without branches
fn sample_cbd(s: &[u8], b: u8, eta: usize) -> Poly {
    let mut buf = [0u8; 192];
    let len = 64 * eta;
    let mut h = Shake256::new();
    h.update(s);
    h.update(&[b]);
    h.finalize_xof().read(&mut buf[..len]);
    let bit = |i: usize| ((buf[i / 8] >> (i % 8)) & 1) as u32;
    let mut f = [0u16; N];
    for (i, x) in f.iter_mut().enumerate() {
        let (mut a, mut c) = (0u32, 0u32);
        for j in 0..eta {
            a += bit(2 * i * eta + j);
            c += bit(2 * i * eta + eta + j);
        }
        *x = csub(a + Q - c) as u16;
    }
    crate::zeroize(&mut buf);
    f
}

fn g(parts: &[&[u8]]) -> [u8; 64] {
    let mut h = Sha3_512::new();
    for p in parts {
        h.update(p);
    }
    h.finalize()
}

fn h(data: &[u8]) -> [u8; 32] {
    let mut x = Sha3_256::new();
    x.update(data);
    x.finalize()
}

// ---------------------------------------------------------------- K-PKE (algorithms 13 to 15)

/// the matrix A-hat from rho (transposed: A[j][i] when `t`)
fn matrix(rho: &[u8], k: usize, t: bool) -> [[Poly; 4]; 4] {
    let mut a = [[[0u16; N]; 4]; 4];
    for (i, row) in a.iter_mut().enumerate().take(k) {
        for (j, e) in row.iter_mut().enumerate().take(k) {
            *e = if t { sample_ntt(rho, i as u8, j as u8) } else { sample_ntt(rho, j as u8, i as u8) };
        }
    }
    a
}

/// K-PKE.Encrypt of the 32-byte message m with the randomness r into ct
fn pke_encrypt(p: &P, ek: &[u8], m: &[u8; 32], r: &[u8], ct: &mut [u8]) {
    let k = p.k;
    let mut t = [[0u16; N]; 4];
    for (i, ti) in t.iter_mut().enumerate().take(k) {
        *ti = decode12(&ek[384 * i..384 * i + 384]);
    }
    let rho = &ek[384 * k..384 * k + 32];
    let at = matrix(rho, k, true);
    let mut y = [[0u16; N]; 4];
    let mut e1 = [[0u16; N]; 4];
    let mut nonce = 0u8;
    for yi in y.iter_mut().take(k) {
        *yi = sample_cbd(r, nonce, p.eta1);
        nonce += 1;
    }
    for ei in e1.iter_mut().take(k) {
        *ei = sample_cbd(r, nonce, p.eta2);
        nonce += 1;
    }
    let mut e2 = sample_cbd(r, nonce, p.eta2);
    for yi in y.iter_mut().take(k) {
        ntt(yi);
    }
    // u = NTT^-1(A^T y) + e1
    let ulen = 32 * p.du as usize;
    for i in 0..k {
        let mut u = [0u16; N];
        for j in 0..k {
            mul_acc(&mut u, &at[i][j], &y[j]);
        }
        ntt_inv(&mut u);
        for (x, e) in u.iter_mut().zip(e1[i].iter()) {
            *x = fadd(*x, *e);
        }
        encode(&compress(&u, p.du), p.du, &mut ct[ulen * i..ulen * (i + 1)]);
    }
    // v = NTT^-1(t^T y) + e2 + Decompress_1(m)
    let mut v = [0u16; N];
    for j in 0..k {
        mul_acc(&mut v, &t[j], &y[j]);
    }
    ntt_inv(&mut v);
    let mu = decompress(&decode(m, 1), 1);
    for ((x, e), u) in v.iter_mut().zip(e2.iter()).zip(mu.iter()) {
        *x = fadd(fadd(*x, *e), *u);
    }
    encode(&compress(&v, p.dv), p.dv, &mut ct[ulen * k..]);
    zero_polys(&mut y);
    zero_polys(&mut e1);
    zero_polys(core::slice::from_mut(&mut e2));
    zero_polys(core::slice::from_mut(&mut v));
}

/// K-PKE.Decrypt: the 32-byte message
fn pke_decrypt(p: &P, dk: &[u8], ct: &[u8]) -> [u8; 32] {
    let k = p.k;
    let ulen = 32 * p.du as usize;
    let mut w = [0u16; N];
    for i in 0..k {
        let mut u = decompress(&decode(&ct[ulen * i..ulen * (i + 1)], p.du), p.du);
        ntt(&mut u);
        let mut s = decode12(&dk[384 * i..384 * i + 384]);
        mul_acc(&mut w, &s, &u);
        zero_polys(core::slice::from_mut(&mut s));
    }
    ntt_inv(&mut w);
    let v = decompress(&decode(&ct[ulen * k..], p.dv), p.dv);
    for (x, y) in w.iter_mut().zip(v.iter()) {
        *x = fsub(*y, *x);
    }
    let mut m = [0u8; 32];
    let mut c = compress(&w, 1);
    encode(&c, 1, &mut m);
    zero_polys(core::slice::from_mut(&mut w));
    zero_polys(core::slice::from_mut(&mut c));
    m
}

// ---------------------------------------------------------------- ML-KEM (algorithms 16 to 21)

/// ML-KEM.KeyGen from the 64-byte seed d || z: (encapsulation key, decapsulation key)
pub fn keygen(params: Params, seed: &[u8; 64]) -> (Vec<u8>, Vec<u8>) {
    let p = params.p();
    let k = p.k;
    let (d, z) = seed.split_at(32);
    let mut rs = g(&[d, &[k as u8]]);
    let (rho, sigma) = rs.split_at(32);
    // s and e first, the matrix after them: rho is public (ek carries it), and the rejection
    // sampling that follows it is the only part of keygen whose way depends on the seed
    let mut s = [[0u16; N]; 4];
    let mut e = [[0u16; N]; 4];
    let mut nonce = 0u8;
    for si in s.iter_mut().take(k) {
        *si = sample_cbd(sigma, nonce, p.eta1);
        ntt(si);
        nonce += 1;
    }
    for ei in e.iter_mut().take(k) {
        *ei = sample_cbd(sigma, nonce, p.eta1);
        ntt(ei);
        nonce += 1;
    }
    let a = matrix(rho, k, false);
    let mut ek = alloc::vec![0u8; params.ek_len()];
    for i in 0..k {
        // t_i = sum_j A[i][j] s_j + e_i
        let mut t = e[i];
        for j in 0..k {
            mul_acc(&mut t, &a[i][j], &s[j]);
        }
        encode(&t, 12, &mut ek[384 * i..384 * i + 384]);
    }
    ek[384 * k..].copy_from_slice(rho);
    let mut dk = Vec::with_capacity(params.dk_len());
    let mut block = [0u8; 384];
    for si in s.iter().take(k) {
        encode(si, 12, &mut block);
        dk.extend_from_slice(&block);
    }
    crate::zeroize(&mut block);
    dk.extend_from_slice(&ek);
    dk.extend_from_slice(&h(&ek));
    dk.extend_from_slice(z);
    zero_polys(&mut s);
    zero_polys(&mut e);
    crate::zeroize(&mut rs);
    (ek, dk)
}

/// the modulus check of FIPS 203 7.2: every 12-bit value of ek below q (public data)
fn ek_ok(params: Params, ek: &[u8]) -> bool {
    let k = params.p().k;
    ek.len() == params.ek_len() && (0..k).all(|i| decode(&ek[384 * i..384 * i + 384], 12).iter().all(|&x| (x as u32) < Q))
}

/// ML-KEM.Encaps with the 32 random bytes m: (ciphertext, shared key)
pub fn encaps(params: Params, ek: &[u8], m: &[u8; 32]) -> Option<(Vec<u8>, [u8; 32])> {
    if !ek_ok(params, ek) {
        return None;
    }
    let p = params.p();
    let mut kr = g(&[m, &h(ek)]);
    let mut ct = alloc::vec![0u8; params.ct_len()];
    pke_encrypt(&p, ek, m, &kr[32..], &mut ct);
    let mut key = [0u8; 32];
    key.copy_from_slice(&kr[..32]);
    crate::zeroize(&mut kr);
    Some((ct, key))
}

/// ML-KEM.Decaps: the shared key (implicit rejection for a ciphertext that does not re-encrypt)
pub fn decaps(params: Params, dk: &[u8], ct: &[u8]) -> Option<[u8; 32]> {
    let p = params.p();
    let k = p.k;
    if dk.len() != params.dk_len() || ct.len() != params.ct_len() {
        return None;
    }
    let (dk_pke, rest) = dk.split_at(384 * k);
    let (ek, rest) = rest.split_at(384 * k + 32);
    let (hash, z) = rest.split_at(32);
    if h(ek) != *hash {
        return None;
    }
    let mut m = pke_decrypt(&p, dk_pke, ct);
    let mut kr = g(&[&m, hash]);
    let mut again = alloc::vec![0u8; ct.len()];
    pke_encrypt(&p, ek, &m, &kr[32..], &mut again);
    // K-bar = J(z || c)
    let mut reject = [0u8; 32];
    let mut j = Shake256::new();
    j.update(z);
    j.update(ct);
    j.finalize_xof().read(&mut reject);
    // the key of the message where the ciphertext re-encrypts, K-bar where it does not: a mask
    let diff = again.iter().zip(ct).fold(0u8, |d, (a, b)| d | (a ^ b));
    let same = crate::ct_mask((((diff as u64) | (diff as u64).wrapping_neg()) >> 63).wrapping_sub(1)) as u8;
    let mut key = [0u8; 32];
    for ((o, a), b) in key.iter_mut().zip(&kr[..32]).zip(&reject) {
        *o = (a & same) | (b & !same);
    }
    crate::zeroize(&mut m);
    crate::zeroize(&mut kr);
    crate::zeroize(&mut reject);
    crate::zeroize(&mut again);
    Some(key)
}
