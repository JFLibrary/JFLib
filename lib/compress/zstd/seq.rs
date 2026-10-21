// seq -- the sequence codes of zstd: literal length, match length and offset codes with their
// base values and extra bits, and the predefined distributions (RFC 8878 3.1.1.3.2)

pub const LL_MAX: usize = 35;
pub const ML_MAX: usize = 52;
pub const OF_MAX: usize = 31;
pub const LL_LOG_MAX: u32 = 9;
pub const ML_LOG_MAX: u32 = 9;
pub const OF_LOG_MAX: u32 = 8;

pub const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64, 128, 256, 512, 1024, 2048,
    4096, 8192, 16384, 32768, 65536,
];
pub const LL_BITS: [u8; 36] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

pub const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
    37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027, 2051, 4099, 8195, 16387, 32771, 65539,
];
pub const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4,
    4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

pub const LL_DEFAULT: [i16; 36] = [4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1, -1, -1, -1, -1];
pub const LL_DEFAULT_LOG: u32 = 6;
pub const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
pub const ML_DEFAULT_LOG: u32 = 6;
pub const OF_DEFAULT: [i16; 29] = [1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1];
pub const OF_DEFAULT_LOG: u32 = 5;

/// the code of a value in a base table (the last base not above it)
#[inline]
fn code_of(base: &[u32], v: u32) -> u8 {
    let mut lo = 0;
    let mut hi = base.len();
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if base[mid] <= v {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo as u8
}

#[inline]
pub fn ll_code(ll: u32) -> u8 {
    if ll < 16 { ll as u8 } else { code_of(&LL_BASE, ll) }
}

#[inline]
pub fn ml_code(ml: u32) -> u8 {
    if ml < 35 { (ml - 3) as u8 } else { code_of(&ML_BASE, ml) }
}

/// the offset code of an offset value (offset + 3, or 1..3 for a repeat)
#[inline]
pub fn of_code(ov: u32) -> u8 {
    (31 - ov.leading_zeros()) as u8
}
