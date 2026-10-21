// ecdsa -- ECDSA verification on NIST P-256 and P-384 (FIPS 186-4, SEC 1)
//
//   verify(curve, public_key, digest, r, s)   public key uncompressed (04 || x || y), checked
//                                             to lie on the curve; r, s big-endian integers
//
// Jacobian coordinates in the Montgomery domain (a = -3 formulas), u1 G + u2 Q by Shamir's
// trick.  Public data only: variable time.

use crate::bn::{self, Limbs, Mont};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    P256,
    P384,
}

struct Params {
    p: &'static str,
    n: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
    bytes: usize,
}

fn params(c: Curve) -> Params {
    match c {
        Curve::P256 => Params {
            p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
            n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
            b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
            gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
            gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
            bytes: 32,
        },
        Curve::P384 => Params {
            p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
            n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
            b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
            gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
            gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
            bytes: 48,
        },
    }
}

impl Curve {
    /// the size of a coordinate / scalar in bytes
    pub fn bytes(self) -> usize {
        params(self).bytes
    }
}

fn hex_limbs(h: &str, k: usize) -> Limbs {
    bn::from_be(&crate::unhex(h).unwrap_or_default(), k)
}

/// a point in Jacobian coordinates, Montgomery form; z = 0 is the point at infinity
#[derive(Clone)]
struct Pt {
    x: Limbs,
    y: Limbs,
    z: Limbs,
}

struct Field {
    m: Mont,
    one: Limbs,
}

impl Field {
    fn mul(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.mul(a, b)
    }
    fn sq(&self, a: &[u64]) -> Limbs {
        self.m.mul(a, a)
    }
    fn add(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.add(a, b)
    }
    fn sub(&self, a: &[u64], b: &[u64]) -> Limbs {
        self.m.sub(a, b)
    }
    fn small(&self, a: &[u64], k: u32) -> Limbs {
        let mut r = a.to_vec();
        for _ in 1..k {
            r = self.add(&r, a);
        }
        r
    }

    fn double(&self, p: &Pt) -> Pt {
        if bn::is_zero(&p.z) || bn::is_zero(&p.y) {
            return Pt { x: self.one.clone(), y: self.one.clone(), z: alloc::vec![0; self.one.len()] };
        }
        let delta = self.sq(&p.z);
        let gamma = self.sq(&p.y);
        let beta = self.mul(&p.x, &gamma);
        let alpha = self.small(&self.mul(&self.sub(&p.x, &delta), &self.add(&p.x, &delta)), 3);
        let x3 = self.sub(&self.sq(&alpha), &self.small(&beta, 8));
        let z3 = self.sub(&self.sub(&self.sq(&self.add(&p.y, &p.z)), &gamma), &delta);
        let y3 = self.sub(&self.mul(&alpha, &self.sub(&self.small(&beta, 4), &x3)), &self.small(&self.sq(&gamma), 8));
        Pt { x: x3, y: y3, z: z3 }
    }

    fn add_pts(&self, p: &Pt, q: &Pt) -> Pt {
        if bn::is_zero(&p.z) {
            return q.clone();
        }
        if bn::is_zero(&q.z) {
            return p.clone();
        }
        let z1z1 = self.sq(&p.z);
        let z2z2 = self.sq(&q.z);
        let u1 = self.mul(&p.x, &z2z2);
        let u2 = self.mul(&q.x, &z1z1);
        let s1 = self.mul(&self.mul(&p.y, &q.z), &z2z2);
        let s2 = self.mul(&self.mul(&q.y, &p.z), &z1z1);
        let h = self.sub(&u2, &u1);
        let r = self.small(&self.sub(&s2, &s1), 2);
        if bn::is_zero(&h) {
            return if bn::is_zero(&r) { self.double(p) } else { Pt { x: self.one.clone(), y: self.one.clone(), z: alloc::vec![0; self.one.len()] } };
        }
        let i = self.sq(&self.small(&h, 2));
        let j = self.mul(&h, &i);
        let v = self.mul(&u1, &i);
        let x3 = self.sub(&self.sub(&self.sq(&r), &j), &self.small(&v, 2));
        let y3 = self.sub(&self.mul(&r, &self.sub(&v, &x3)), &self.small(&self.mul(&s1, &j), 2));
        let z3 = self.mul(&self.sub(&self.sub(&self.sq(&self.add(&p.z, &q.z)), &z1z1), &z2z2), &h);
        Pt { x: x3, y: y3, z: z3 }
    }
}

/// verify an ECDSA signature over `digest` (the hash of the message, any length)
pub fn verify(curve: Curve, public_key: &[u8], digest: &[u8], r: &[u8], s: &[u8]) -> bool {
    let pr = params(curve);
    let nb = pr.bytes;
    let k = nb.div_ceil(8);
    if public_key.len() != 1 + 2 * nb || public_key[0] != 4 {
        return false;
    }
    let p = hex_limbs(pr.p, k);
    let n = hex_limbs(pr.n, k);
    let (Some(fm), Some(nm)) = (Mont::new(&p), Mont::new(&n)) else { return false };
    let f = Field { one: fm.to_mont(&[1]), m: fm };
    // r, s in [1, n-1]
    let strip = |x: &[u8]| -> Option<Limbs> {
        let x = &x[x.iter().position(|&b| b != 0).unwrap_or(x.len())..];
        if x.len() > nb {
            return None;
        }
        let v = bn::from_be(x, k);
        (!bn::is_zero(&v) && bn::cmp(&v, &n) == core::cmp::Ordering::Less).then_some(v)
    };
    let (Some(r), Some(s)) = (strip(r), strip(s)) else { return false };
    // the public key: coordinates below p, on the curve y^2 = x^3 - 3x + b
    let qx = bn::from_be(&public_key[1..1 + nb], k);
    let qy = bn::from_be(&public_key[1 + nb..], k);
    if bn::cmp(&qx, &p) != core::cmp::Ordering::Less || bn::cmp(&qy, &p) != core::cmp::Ordering::Less {
        return false;
    }
    let (mx, my) = (f.m.to_mont(&qx), f.m.to_mont(&qy));
    let b = f.m.to_mont(&hex_limbs(pr.b, k));
    let rhs = f.add(&f.sub(&f.mul(&f.sq(&mx), &mx), &f.small(&mx, 3)), &b);
    if f.sq(&my) != rhs {
        return false;
    }
    // e: the leftmost bits of the digest, as many as n has
    let n_bits = bn::bits(&n);
    let mut e = bn::from_be(&digest[..digest.len().min(nb)], k + 1);
    let have = digest.len().min(nb) * 8;
    if have > n_bits {
        let sh = have - n_bits;
        let mut c = 0u64;
        for x in e.iter_mut().rev() {
            let nc = *x << (64 - sh);
            *x = *x >> sh | c;
            c = nc;
        }
    }
    let e = nm.reduce_once(&e);
    // w = s^-1, u1 = e w, u2 = r w (mod n)
    let w = nm.inv_mont(&nm.to_mont(&s));
    let u1 = nm.from_mont(&nm.mul(&nm.to_mont(&e), &w));
    let u2 = nm.from_mont(&nm.mul(&nm.to_mont(&r), &w));
    let g = Pt { x: f.m.to_mont(&hex_limbs(pr.gx, k)), y: f.m.to_mont(&hex_limbs(pr.gy, k)), z: f.one.clone() };
    let q = Pt { x: mx, y: my, z: f.one.clone() };
    let gq = f.add_pts(&g, &q);
    let (b1, b2) = (bn::to_be(&u1, nb), bn::to_be(&u2, nb));
    let mut acc = Pt { x: f.one.clone(), y: f.one.clone(), z: alloc::vec![0; k] };
    for i in 0..nb * 8 {
        acc = f.double(&acc);
        let bit = |b: &[u8]| b[i / 8] >> (7 - i % 8) & 1 == 1;
        match (bit(&b1), bit(&b2)) {
            (true, true) => acc = f.add_pts(&acc, &gq),
            (true, false) => acc = f.add_pts(&acc, &g),
            (false, true) => acc = f.add_pts(&acc, &q),
            _ => {}
        }
    }
    if bn::is_zero(&acc.z) {
        return false;
    }
    // affine x = X / Z^2, then mod n
    let zinv = f.m.inv_mont(&acc.z);
    let x = f.m.from_mont(&f.mul(&acc.x, &f.sq(&zinv)));
    let xn = nm.reduce_once(&x);
    xn == r
}
