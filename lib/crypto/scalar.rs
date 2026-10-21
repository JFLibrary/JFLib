// scalar -- integers modulo the Ed25519 group order L = 2^252 + 2774231777737235353585193779
// 0883648493: reduction of 512-bit values and a*b + c, bit-serial with masked subtraction
// (no branches on the values; slow next to the curve arithmetic, simple to check)

const L: [u64; 4] = [0x5812_631a_5cf5_d3ed, 0x14de_f9de_a2f7_9cd6, 0, 0x1000_0000_0000_0000];

fn limbs(b: &[u8]) -> [u64; 4] {
    let mut r = [0u64; 4];
    for (i, x) in r.iter_mut().enumerate() {
        *x = crate::le64(&b[i * 8..]);
    }
    r
}

fn bytes(r: &[u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (o, x) in out.chunks_exact_mut(8).zip(r) {
        o.copy_from_slice(&x.to_le_bytes());
    }
    out
}

/// a 512-bit little-endian number (as 8 limbs) mod L
fn reduce_limbs(n: &[u64; 8]) -> [u64; 4] {
    let mut r = [0u64; 4];
    for bit in (0..512).rev() {
        // r = 2r + bit  (r < L < 2^253, so 2r + 1 < 2^254 fits)
        let b = (n[bit / 64] >> (bit % 64)) & 1;
        r[3] = (r[3] << 1) | (r[2] >> 63);
        r[2] = (r[2] << 1) | (r[1] >> 63);
        r[1] = (r[1] << 1) | (r[0] >> 63);
        r[0] = (r[0] << 1) | b;
        // t = r - L; keep t unless it borrowed
        let mut t = [0u64; 4];
        let mut borrow = 0u64;
        for i in 0..4 {
            let (d, b1) = r[i].overflowing_sub(L[i]);
            let (d, b2) = d.overflowing_sub(borrow);
            t[i] = d;
            borrow = (b1 | b2) as u64;
        }
        let keep_r = 0u64.wrapping_sub(borrow); // all ones when r < L
        for i in 0..4 {
            r[i] = (r[i] & keep_r) | (t[i] & !keep_r);
        }
    }
    r
}

/// 64 little-endian bytes mod L
pub fn reduce64(b: &[u8; 64]) -> [u8; 32] {
    let mut n = [0u64; 8];
    for (i, x) in n.iter_mut().enumerate() {
        *x = crate::le64(&b[i * 8..]);
    }
    bytes(&reduce_limbs(&n))
}

/// (a * b + c) mod L of 32-byte little-endian values
pub fn mul_add(a: &[u8; 32], b: &[u8; 32], c: &[u8; 32]) -> [u8; 32] {
    let (a, b, c) = (limbs(a), limbs(b), limbs(c));
    let mut n = [0u64; 8];
    for i in 0..4 {
        let mut carry = 0u128;
        for j in 0..4 {
            let t = a[i] as u128 * b[j] as u128 + n[i + j] as u128 + carry;
            n[i + j] = t as u64;
            carry = t >> 64;
        }
        n[i + 4] = carry as u64;
    }
    let mut carry = 0u128;
    for i in 0..8 {
        let t = n[i] as u128 + if i < 4 { c[i] as u128 } else { 0 } + carry;
        n[i] = t as u64;
        carry = t >> 64;
    }
    bytes(&reduce_limbs(&n))
}

/// whether 32 little-endian bytes are a reduced scalar (< L)
pub fn is_canonical(s: &[u8; 32]) -> bool {
    let v = limbs(s);
    for i in (0..4).rev() {
        if v[i] != L[i] {
            return v[i] < L[i];
        }
    }
    false
}
