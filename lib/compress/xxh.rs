// xxh -- xxHash32 (the checksum of LZ4 frames) and xxHash64 (zstd frames); not cryptographic

const P1: u32 = 0x9E37_79B1;
const P2: u32 = 0x85EB_CA77;
const P3: u32 = 0xC2B2_AE3D;
const P4: u32 = 0x27D4_EB2F;
const P5: u32 = 0x1656_67B1;

#[inline(always)]
fn rd(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline(always)]
fn round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(P2)).rotate_left(13).wrapping_mul(P1)
}

/// xxHash32 of a byte string
pub fn xxh32(data: &[u8], seed: u32) -> u32 {
    let mut p = data;
    let mut h = if data.len() >= 16 {
        let mut v = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        while p.len() >= 16 {
            for (i, x) in v.iter_mut().enumerate() {
                *x = round(*x, rd(&p[i * 4..]));
            }
            p = &p[16..];
        }
        v[0].rotate_left(1).wrapping_add(v[1].rotate_left(7)).wrapping_add(v[2].rotate_left(12)).wrapping_add(v[3].rotate_left(18))
    } else {
        seed.wrapping_add(P5)
    };
    h = h.wrapping_add(data.len() as u32);
    while p.len() >= 4 {
        h = h.wrapping_add(rd(p).wrapping_mul(P3)).rotate_left(17).wrapping_mul(P4);
        p = &p[4..];
    }
    for &b in p {
        h = h.wrapping_add((b as u32).wrapping_mul(P5)).rotate_left(11).wrapping_mul(P1);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^= h >> 16;
    h
}

const Q1: u64 = 0x9E37_79B1_85EB_CA87;
const Q2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const Q3: u64 = 0x1656_67B1_9E37_79F9;
const Q4: u64 = 0x85EB_CA77_C2B2_AE63;
const Q5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline(always)]
fn rd8(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[inline(always)]
fn round64(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(Q2)).rotate_left(31).wrapping_mul(Q1)
}

#[inline(always)]
fn merge64(acc: u64, v: u64) -> u64 {
    (acc ^ round64(0, v)).wrapping_mul(Q1).wrapping_add(Q4)
}

/// xxHash64 of a byte string (zstd's content checksum is its low 32 bits)
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    let mut p = data;
    let mut h = if data.len() >= 32 {
        let mut v = [seed.wrapping_add(Q1).wrapping_add(Q2), seed.wrapping_add(Q2), seed, seed.wrapping_sub(Q1)];
        while p.len() >= 32 {
            for (i, x) in v.iter_mut().enumerate() {
                *x = round64(*x, rd8(&p[i * 8..]));
            }
            p = &p[32..];
        }
        let mut h = v[0].rotate_left(1).wrapping_add(v[1].rotate_left(7)).wrapping_add(v[2].rotate_left(12)).wrapping_add(v[3].rotate_left(18));
        for x in v {
            h = merge64(h, x);
        }
        h
    } else {
        seed.wrapping_add(Q5)
    };
    h = h.wrapping_add(data.len() as u64);
    while p.len() >= 8 {
        h = (h ^ round64(0, rd8(p))).rotate_left(27).wrapping_mul(Q1).wrapping_add(Q4);
        p = &p[8..];
    }
    if p.len() >= 4 {
        h = (h ^ (rd(p) as u64).wrapping_mul(Q1)).rotate_left(23).wrapping_mul(Q2).wrapping_add(Q3);
        p = &p[4..];
    }
    for &b in p {
        h = (h ^ (b as u64).wrapping_mul(Q5)).rotate_left(11).wrapping_mul(Q1);
    }
    h ^= h >> 33;
    h = h.wrapping_mul(Q2);
    h ^= h >> 29;
    h = h.wrapping_mul(Q3);
    h ^= h >> 32;
    h
}
