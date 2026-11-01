// p256 -- Diffie-Hellman on NIST P-256 (SEC 1, RFC 8446 4.2.8.2) in constant time
//
//   public_key(&secret) -> [u8; 65]          (panics on an invalid secret) 04 || x || y of secret * G
//   shared(&secret, &peer_point) -> Option<[u8; 32]>   the x coordinate of secret * peer;
//                                            None for a point that is not on the curve
//   secret_from(random) -> Option<[u8; 32]>  a scalar in [1, n - 1] from 32 random bytes
//
// The curve as a group, for protocols that compute on points themselves (EAP-PWD: eappwd.rs):
// points are x || y (64 bytes, both big endian), scalars 32 bytes big endian
//
//   below_p(x)                               x is a coordinate: smaller than p
//   y_for_x(x, odd) -> Option<[u8; 32]>      the y of a point with this x and parity (None: no such
//                                            point; variable time, only for public or hashed x)
//   on_curve(point)                          both coordinates in range and the curve equation
//   mul_point(k, point) -> Option<point>     k * point (None: not on the curve, or the identity)
//   add_points(a, b) -> Option<point>        a + b (None: not on the curve, or the identity)
//   neg_point(point)                         -point
//   scalar_add(a, b)                         a + b mod n
//   scalar_ok(k)                             k lies in [1, n - 1]
//
// Field elements in Montgomery form with four 64-bit limbs; multiplication, addition and
// subtraction without branches on the values (masked final corrections).  Points in projective
// coordinates with the complete addition formulas of Renes, Costello and Batina (2016,
// algorithm 4, a = -3): no exceptional cases, so doubling uses the same formula.  Scalar
// multiplication by four-bit windows with a table read in full for every window.

type Fe = [u64; 4];

/// p = 2^256 - 2^224 + 2^192 + 2^96 - 1
const P: Fe = [0xFFFF_FFFF_FFFF_FFFF, 0x0000_0000_FFFF_FFFF, 0x0000_0000_0000_0000, 0xFFFF_FFFF_0000_0001];
/// the group order
const N: Fe = [0xF3B9_CAC2_FC63_2551, 0xBCE6_FAAD_A717_9E84, 0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF_0000_0000];
/// R^2 mod p (R = 2^256)
const R2: Fe = [0x0000_0000_0000_0003, 0xFFFF_FFFB_FFFF_FFFF, 0xFFFF_FFFF_FFFF_FFFE, 0x0000_0004_FFFF_FFFD];
const B: Fe = [0x3BCE_3C3E_27D2_604B, 0x651D_06B0_CC53_B0F6, 0xB3EB_BD55_7698_86BC, 0x5AC6_35D8_AA3A_93E7];
const GX: Fe = [0xF4A1_3945_D898_C296, 0x7703_7D81_2DEB_33A0, 0xF8BC_E6E5_63A4_40F2, 0x6B17_D1F2_E12C_4247];
const GY: Fe = [0xCBB6_4068_37BF_51F5, 0x2BCE_3357_6B31_5ECE, 0x8EE7_EB4A_7C0F_9E16, 0x4FE3_42E2_FE1A_7F9B];

#[inline(always)]
fn adc(a: u64, b: u64, c: u64) -> (u64, u64) {
    let t = a as u128 + b as u128 + c as u128;
    (t as u64, (t >> 64) as u64)
}

#[inline(always)]
fn sbb(a: u64, b: u64, borrow: u64) -> (u64, u64) {
    let t = (a as u128).wrapping_sub(b as u128 + borrow as u128);
    (t as u64, (t >> 127) as u64)
}

/// r = a - b; then + p when that borrowed (masked)
fn sub(a: &Fe, b: &Fe) -> Fe {
    let mut r = [0u64; 4];
    let mut br = 0;
    for i in 0..4 {
        (r[i], br) = sbb(a[i], b[i], br);
    }
    let m = 0u64.wrapping_sub(br);
    let mut c = 0;
    for i in 0..4 {
        (r[i], c) = adc(r[i], P[i] & m, c);
    }
    r
}

/// r = a + b mod p (a, b < p)
fn add(a: &Fe, b: &Fe) -> Fe {
    let mut r = [0u64; 4];
    let mut c = 0;
    for i in 0..4 {
        (r[i], c) = adc(a[i], b[i], c);
    }
    // subtract p; keep the subtraction unless it borrowed beyond the carry
    let mut s = [0u64; 4];
    let mut br = 0;
    for i in 0..4 {
        (s[i], br) = sbb(r[i], P[i], br);
    }
    let (_, br) = sbb(c, 0, br);
    let keep_r = 0u64.wrapping_sub(br);
    let mut out = [0u64; 4];
    for i in 0..4 {
        out[i] = (r[i] & keep_r) | (s[i] & !keep_r);
    }
    out
}

/// a b R^-1 mod p (CIOS; -p^-1 mod 2^64 = 1)
fn mul(a: &Fe, b: &Fe) -> Fe {
    let mut t = [0u64; 6];
    for i in 0..4 {
        let mut c = 0;
        for j in 0..4 {
            let x = t[j] as u128 + a[j] as u128 * b[i] as u128 + c as u128;
            t[j] = x as u64;
            c = (x >> 64) as u64;
        }
        let (s, c2) = adc(t[4], c, 0);
        t[4] = s;
        t[5] = c2;
        let m = t[0];
        let mut c = 0;
        for j in 0..4 {
            let x = t[j] as u128 + m as u128 * P[j] as u128 + c as u128;
            t[j] = x as u64;
            c = (x >> 64) as u64;
        }
        let (s, c2) = adc(t[4], c, 0);
        t[4] = s;
        t[5] += c2;
        t = [t[1], t[2], t[3], t[4], t[5], 0];
    }
    let r = [t[0], t[1], t[2], t[3]];
    let mut s = [0u64; 4];
    let mut br = 0;
    for i in 0..4 {
        (s[i], br) = sbb(r[i], P[i], br);
    }
    let (_, br) = sbb(t[4], 0, br);
    let keep_r = 0u64.wrapping_sub(br);
    let mut out = [0u64; 4];
    for i in 0..4 {
        out[i] = (r[i] & keep_r) | (s[i] & !keep_r);
    }
    out
}

fn to_mont(a: &Fe) -> Fe {
    mul(a, &R2)
}

fn from_mont(a: &Fe) -> Fe {
    mul(a, &[1, 0, 0, 0])
}

/// a^(p-2) (a fixed sequence of squarings and multiplications)
fn inv(a: &Fe) -> Fe {
    let mut e = P;
    e[0] -= 2;
    let mut r = to_mont(&[1, 0, 0, 0]);
    for i in (0..256).rev() {
        r = mul(&r, &r);
        if (e[i / 64] >> (i % 64)) & 1 == 1 {
            r = mul(&r, a);
        }
    }
    r
}

fn from_be(b: &[u8]) -> Fe {
    let mut r = [0u64; 4];
    for i in 0..4 {
        let mut w = [0u8; 8];
        w.copy_from_slice(&b[24 - 8 * i..32 - 8 * i]);
        r[i] = u64::from_be_bytes(w);
    }
    r
}

fn to_be(a: &Fe) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..4 {
        out[24 - 8 * i..32 - 8 * i].copy_from_slice(&a[i].to_be_bytes());
    }
    out
}

/// a < m (public comparisons only)
fn less(a: &Fe, m: &Fe) -> bool {
    for i in (0..4).rev() {
        if a[i] != m[i] {
            return a[i] < m[i];
        }
    }
    false
}

/// projective point, Montgomery coordinates; (0 : 1 : 0) is the identity
#[derive(Clone, Copy)]
struct Pt {
    x: Fe,
    y: Fe,
    z: Fe,
}

/// P + Q, complete (RCB 2016, algorithm 4)
fn padd(p: &Pt, q: &Pt, b: &Fe) -> Pt {
    let (x1, y1, z1, x2, y2, z2) = (&p.x, &p.y, &p.z, &q.x, &q.y, &q.z);
    let mut t0 = mul(x1, x2);
    let mut t1 = mul(y1, y2);
    let mut t2 = mul(z1, z2);
    let mut t3 = add(x1, y1);
    let mut t4 = add(x2, y2);
    t3 = mul(&t3, &t4);
    t4 = add(&t0, &t1);
    t3 = sub(&t3, &t4);
    t4 = add(y1, z1);
    let mut x3 = add(y2, z2);
    t4 = mul(&t4, &x3);
    x3 = add(&t1, &t2);
    t4 = sub(&t4, &x3);
    x3 = add(x1, z1);
    let mut y3 = add(x2, z2);
    x3 = mul(&x3, &y3);
    y3 = add(&t0, &t2);
    y3 = sub(&x3, &y3);
    let mut z3 = mul(b, &t2);
    x3 = sub(&y3, &z3);
    z3 = add(&x3, &x3);
    x3 = add(&x3, &z3);
    z3 = sub(&t1, &x3);
    x3 = add(&t1, &x3);
    y3 = mul(b, &y3);
    t1 = add(&t2, &t2);
    t2 = add(&t1, &t2);
    y3 = sub(&y3, &t2);
    y3 = sub(&y3, &t0);
    t1 = add(&y3, &y3);
    y3 = add(&t1, &y3);
    t1 = add(&t0, &t0);
    t0 = add(&t1, &t0);
    t0 = sub(&t0, &t2);
    t1 = mul(&t4, &y3);
    t2 = mul(&t0, &y3);
    y3 = mul(&x3, &z3);
    y3 = add(&y3, &t2);
    x3 = mul(&t3, &x3);
    x3 = sub(&x3, &t1);
    z3 = mul(&t4, &z3);
    t1 = mul(&t3, &t0);
    z3 = add(&z3, &t1);
    Pt { x: x3, y: y3, z: z3 }
}

/// k * P for a 32-byte big-endian scalar (k < n), constant time
fn scalar_mul(k: &[u8; 32], p: &Pt, b: &Fe) -> Pt {
    let one = to_mont(&[1, 0, 0, 0]);
    let id = Pt { x: [0; 4], y: one, z: [0; 4] };
    let mut table = [id; 16];
    for i in 1..16 {
        table[i] = padd(&table[i - 1], p, b);
    }
    let mut acc = id;
    for byte in k.iter() {
        for half in [byte >> 4, byte & 15] {
            for _ in 0..4 {
                acc = padd(&acc, &acc, b);
            }
            // read every entry; keep the wanted one by mask
            let mut sel = id;
            for (i, e) in table.iter().enumerate() {
                let m = 0u64.wrapping_sub(((i as u8 ^ half) as u64).wrapping_sub(1) >> 63);
                for j in 0..4 {
                    sel.x[j] = (sel.x[j] & !m) | (e.x[j] & m);
                    sel.y[j] = (sel.y[j] & !m) | (e.y[j] & m);
                    sel.z[j] = (sel.z[j] & !m) | (e.z[j] & m);
                }
            }
            acc = padd(&acc, &sel, b);
        }
    }
    acc
}

fn affine(p: &Pt) -> Option<(Fe, Fe)> {
    if p.z == [0; 4] {
        return None;
    }
    let zi = inv(&p.z);
    Some((from_mont(&mul(&p.x, &zi)), from_mont(&mul(&p.y, &zi))))
}

/// a scalar in [1, n - 1] from random bytes (None: draw again)
pub fn secret_from(random: &[u8; 32]) -> Option<[u8; 32]> {
    let k = from_be(random);
    (k != [0; 4] && less(&k, &N)).then_some(*random)
}

/// the public key of a secret scalar: 04 || x || y
pub fn public_key(secret: &[u8; 32]) -> [u8; 65] {
    assert!(secret_from(secret).is_some(), "p256 secret outside [1, n - 1]");
    let b = to_mont(&B);
    let g = Pt { x: to_mont(&GX), y: to_mont(&GY), z: to_mont(&[1, 0, 0, 0]) };
    let r = scalar_mul(secret, &g, &b);
    let mut out = [0u8; 65];
    out[0] = 4;
    if let Some((x, y)) = affine(&r) {
        out[1..33].copy_from_slice(&to_be(&x));
        out[33..].copy_from_slice(&to_be(&y));
    }
    out
}

/// the shared secret (x coordinate) with a peer's public point
pub fn shared(secret: &[u8; 32], peer: &[u8]) -> Option<[u8; 32]> {
    if peer.len() != 65 || peer[0] != 4 {
        return None;
    }
    let (x, y) = (from_be(&peer[1..33]), from_be(&peer[33..]));
    if !less(&x, &P) || !less(&y, &P) {
        return None;
    }
    let (mx, my) = (to_mont(&x), to_mont(&y));
    let b = to_mont(&B);
    // y^2 = x^3 - 3x + b
    let x3 = mul(&mul(&mx, &mx), &mx);
    let rhs = add(&sub(&x3, &add(&add(&mx, &mx), &mx)), &b);
    if mul(&my, &my) != rhs {
        return None;
    }
    let q = Pt { x: mx, y: my, z: to_mont(&[1, 0, 0, 0]) };
    let (sx, _) = affine(&scalar_mul(secret, &q, &b))?;
    Some(to_be(&sx))
}

// ---------------------------------------------------------------- the group, for protocols on points

/// x is smaller than p
pub fn below_p(x: &[u8; 32]) -> bool {
    less(&from_be(x), &P)
}

/// a^e (variable time in e, which is public here)
fn pow(a: &Fe, e: &Fe) -> Fe {
    let mut r = to_mont(&[1, 0, 0, 0]);
    for i in (0..256).rev() {
        r = mul(&r, &r);
        if (e[i / 64] >> (i % 64)) & 1 == 1 {
            r = mul(&r, a);
        }
    }
    r
}

/// x^3 - 3x + b in Montgomery form (x too)
fn curve_rhs(mx: &Fe) -> Fe {
    let x3 = mul(&mul(mx, mx), mx);
    add(&sub(&x3, &add(&add(mx, mx), mx)), &to_mont(&B))
}

/// the y of the point with this x and whose lowest bit is `odd`; None when x is out of range or
/// x^3 - 3x + b has no square root (p = 3 mod 4: the root is a^((p + 1) / 4))
pub fn y_for_x(x: &[u8; 32], odd: bool) -> Option<[u8; 32]> {
    if !below_p(x) {
        return None;
    }
    let mx = to_mont(&from_be(x));
    let rhs = curve_rhs(&mx);
    // (p + 1) / 4 = 2^254 - 2^222 + 2^190 + 2^94
    let e: Fe = [0, 0x0000_0000_4000_0000, 0x4000_0000_0000_0000, 0x3FFF_FFFF_C000_0000];
    let root = pow(&rhs, &e);
    if mul(&root, &root) != rhs {
        return None;
    }
    let y = from_mont(&root);
    let y = if (y[0] & 1 == 1) == odd { y } else { sub(&[0; 4], &y) };
    Some(to_be(&y))
}

/// x || y as a point, checked: in range and on the curve
fn point_of(p: &[u8; 64]) -> Option<Pt> {
    let (x, y) = (from_be(&p[..32]), from_be(&p[32..]));
    if !less(&x, &P) || !less(&y, &P) {
        return None;
    }
    let (mx, my) = (to_mont(&x), to_mont(&y));
    if mul(&my, &my) != curve_rhs(&mx) {
        return None;
    }
    Some(Pt { x: mx, y: my, z: to_mont(&[1, 0, 0, 0]) })
}

/// x || y of a point (None: the identity)
fn bytes_of(p: &Pt) -> Option<[u8; 64]> {
    let (x, y) = affine(p)?;
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&to_be(&x));
    out[32..].copy_from_slice(&to_be(&y));
    Some(out)
}

/// both coordinates are in range and the point is on the curve
pub fn on_curve(p: &[u8; 64]) -> bool {
    point_of(p).is_some()
}

/// k * point (k below n)
pub fn mul_point(k: &[u8; 32], p: &[u8; 64]) -> Option<[u8; 64]> {
    let q = point_of(p)?;
    bytes_of(&scalar_mul(k, &q, &to_mont(&B)))
}

/// a + b
pub fn add_points(a: &[u8; 64], b: &[u8; 64]) -> Option<[u8; 64]> {
    let (p, q) = (point_of(a)?, point_of(b)?);
    bytes_of(&padd(&p, &q, &to_mont(&B)))
}

/// -point (y becomes p - y)
pub fn neg_point(p: &[u8; 64]) -> [u8; 64] {
    let mut out = *p;
    let mut y = [0u8; 32];
    y.copy_from_slice(&p[32..]);
    let ny = sub(&[0; 4], &from_be(&y));
    out[32..].copy_from_slice(&to_be(&ny));
    out
}

/// a + b mod n (both below n)
pub fn scalar_add(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (x, y) = (from_be(a), from_be(b));
    let mut r = [0u64; 4];
    let mut c = 0;
    for i in 0..4 {
        (r[i], c) = adc(x[i], y[i], c);
    }
    let mut s = [0u64; 4];
    let mut br = 0;
    for i in 0..4 {
        (s[i], br) = sbb(r[i], N[i], br);
    }
    let (_, br) = sbb(c, 0, br);
    to_be(&if br == 1 { r } else { s })
}

/// k lies in [1, n - 1]
pub fn scalar_ok(k: &[u8; 32]) -> bool {
    secret_from(k).is_some()
}
