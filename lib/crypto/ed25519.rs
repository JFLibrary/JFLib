// ed25519 -- Ed25519 signatures (RFC 8032), strict verification
//
//   Keypair::from_seed(&seed) -> Keypair { public }     sign(&msg) -> [u8; 64]
//   verify(&public, &msg, &sig) -> bool
//
// Verification is strict: the public key and R must be canonical points, S must be below the
// group order, and [S]B == R + [k]A must hold exactly (no cofactor), so a signature is valid
// or not the same way everywhere.  The curve constants (d, sqrt(-1), the base point) are
// computed from their definitions instead of copied as long numbers.
// Scalar multiplication: double and add-always with masked selects (no branches on bits).

use crate::field::Fe;
use crate::scalar;
use crate::sha2::Sha512;

/// a point in extended coordinates (X:Y:Z:T), x = X/Z, y = Y/Z, xy = T/Z
#[derive(Clone, Copy)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

struct Curve {
    d2: Fe,
    d: Fe,
    sqrt_m1: Fe,
    base: Point,
}

impl Curve {
    fn new() -> Curve {
        // d = -121665 / 121666
        let d = Fe::small(121665).neg().mul(&Fe::small(121666).invert());
        // sqrt(-1) = 2^((p - 1) / 4); (p - 1) / 4 = 2^253 - 5 = (2^252 - 3) * 2 + 1
        let two = Fe::small(2);
        let sqrt_m1 = two.pow_p58().square().mul(&two);
        let mut c = Curve { d2: d.add(&d), d, sqrt_m1, base: Point::IDENTITY };
        // B: y = 4/5, x even
        let y = Fe::small(4).mul(&Fe::small(5).invert());
        let mut enc = y.to_bytes();
        enc[31] &= 0x7f;
        c.base = c.decode(&enc).expect("base point");
        c
    }

    /// RFC 8032 5.1.3, strict: None for a non-canonical y, no square root, or -0
    fn decode(&self, b: &[u8; 32]) -> Option<Point> {
        let sign = b[31] >> 7;
        let mut yb = *b;
        yb[31] &= 0x7f;
        let y = Fe::from_bytes(&yb);
        if y.to_bytes() != yb {
            return None; // y >= p
        }
        let yy = y.square();
        let u = yy.sub(&Fe::ONE);
        let v = self.d.mul(&yy).add(&Fe::ONE);
        let v3 = v.square().mul(&v);
        let v7 = v3.square().mul(&v);
        let mut x = u.mul(&v3).mul(&u.mul(&v7).pow_p58());
        let vxx = v.mul(&x.square());
        if vxx.ct_equal(&u) {
            // x is a root
        } else if vxx.ct_equal(&u.neg()) {
            x = x.mul(&self.sqrt_m1);
        } else {
            return None;
        }
        if x.is_zero() && sign == 1 {
            return None;
        }
        if x.is_negative() as u8 != sign {
            x = x.neg();
        }
        Some(Point { x, y, z: Fe::ONE, t: x.mul(&y) })
    }
}

impl Point {
    const IDENTITY: Point = Point { x: Fe::ZERO, y: Fe::ONE, z: Fe::ONE, t: Fe::ZERO };

    fn add(&self, q: &Point, c: &Curve) -> Point {
        let a = self.y.sub(&self.x).mul(&q.y.sub(&q.x));
        let b = self.y.add(&self.x).mul(&q.y.add(&q.x));
        let cc = self.t.mul(&c.d2).mul(&q.t);
        let d = self.z.add(&self.z).mul(&q.z);
        let e = b.sub(&a);
        let f = d.sub(&cc);
        let g = d.add(&cc);
        let h = b.add(&a);
        Point { x: e.mul(&f), y: g.mul(&h), t: e.mul(&h), z: f.mul(&g) }
    }

    fn double(&self) -> Point {
        let a = self.x.square();
        let b = self.y.square();
        let c = self.z.square();
        let c = c.add(&c);
        let h = a.add(&b);
        let e = h.sub(&self.x.add(&self.y).square());
        let g = a.sub(&b);
        let f = c.add(&g);
        Point { x: e.mul(&f), y: g.mul(&h), t: e.mul(&h), z: f.mul(&g) }
    }

    fn neg(&self) -> Point {
        Point { x: self.x.neg(), y: self.y, z: self.z, t: self.t.neg() }
    }

    fn select(&self, q: &Point, choice: u64) -> Point {
        Point {
            x: self.x.select(&q.x, choice),
            y: self.y.select(&q.y, choice),
            z: self.z.select(&q.z, choice),
            t: self.t.select(&q.t, choice),
        }
    }

    /// [k]P for a 256-bit little-endian k (no branches on its bits)
    fn mul(&self, k: &[u8; 32], c: &Curve) -> Point {
        let mut r = Point::IDENTITY;
        for i in (0..256).rev() {
            r = r.double();
            let bit = ((k[i >> 3] >> (i & 7)) & 1) as u64;
            let s = r.add(self, c);
            r = r.select(&s, bit);
        }
        r
    }

    fn encode(&self) -> [u8; 32] {
        let zi = self.z.invert();
        let x = self.x.mul(&zi);
        let y = self.y.mul(&zi);
        let mut b = y.to_bytes();
        b[31] |= (x.is_negative() as u8) << 7;
        b
    }
}

fn hash_reduce(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha512::new();
    for p in parts {
        h.update(p);
    }
    scalar::reduce64(&h.finalize())
}

/// an Ed25519 key pair from its 32-byte seed (the private key of RFC 8032)
#[derive(Clone)]
pub struct Keypair {
    seed: [u8; 32],
    a: [u8; 32],
    prefix: [u8; 32],
    pub public: [u8; 32],
}

impl Drop for Keypair {
    fn drop(&mut self) {
        crate::zeroize(&mut self.seed);
        crate::zeroize(&mut self.a);
        crate::zeroize(&mut self.prefix);
    }
}

impl Keypair {
    pub fn from_seed(seed: &[u8; 32]) -> Keypair {
        let h = crate::sha512(seed);
        let mut a = [0u8; 32];
        a.copy_from_slice(&h[..32]);
        a[0] &= 248;
        a[31] &= 63;
        a[31] |= 64;
        let mut prefix = [0u8; 32];
        prefix.copy_from_slice(&h[32..]);
        let c = Curve::new();
        let public = c.base.mul(&a, &c).encode();
        Keypair { seed: *seed, a, prefix, public }
    }

    /// the seed (to store the key)
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let c = Curve::new();
        let r = hash_reduce(&[&self.prefix, msg]);
        let rb = c.base.mul(&r, &c).encode();
        let k = hash_reduce(&[&rb, &self.public, msg]);
        let s = scalar::mul_add(&k, &self.a, &r);
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&rb);
        sig[32..].copy_from_slice(&s);
        sig
    }
}

/// strict verification of a signature
pub fn verify(public: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let c = Curve::new();
    let Some(a) = c.decode(public) else { return false };
    let mut rb = [0u8; 32];
    rb.copy_from_slice(&sig[..32]);
    if c.decode(&rb).is_none() {
        return false;
    }
    let mut s = [0u8; 32];
    s.copy_from_slice(&sig[32..]);
    if !scalar::is_canonical(&s) {
        return false;
    }
    let k = hash_reduce(&[&rb, public, msg]);
    // [S]B - [k]A must encode to R
    let check = c.base.mul(&s, &c).add(&a.neg().mul(&k, &c), &c);
    crate::ct_eq(&check.encode(), &rb)
}

/// whether 32 bytes are a valid (canonical, on-curve) public key
pub fn valid_public_key(public: &[u8; 32]) -> bool {
    Curve::new().decode(public).is_some()
}
