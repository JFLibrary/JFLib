// chunker -- content-defined chunking (FastCDC with normalized chunking, level 2)
//
// Boundaries depend on the content only, so an insertion early in a file moves one boundary
// and leaves the later chunks (and their deduplication) intact.  The gear table is derived
// from the repository key: chunk sizes do not reveal known files to someone who sees the
// repository without its key.

use alloc::vec::Vec;

#[derive(Clone)]
pub struct Chunker {
    gear: [u64; 256],
    pub min: usize,
    pub avg: usize,
    pub max: usize,
    mask_s: u64,
    mask_l: u64,
}

/// a mask of `bits` high bits (the gear hash accumulates towards the top)
fn mask(bits: u32) -> u64 {
    if bits == 0 { 0 } else { (!0u64) << (64 - bits.min(63)) }
}

impl Chunker {
    /// avg must be a power of two; min = avg / 4, max = avg * 4
    pub fn new(gear_key: &[u8; 32], avg: usize) -> Chunker {
        let mut bytes = [0u8; 256 * 8];
        crypto::Blake3::new_keyed(gear_key).update(b"gear table").finalize_xof(&mut bytes);
        let mut gear = [0u64; 256];
        for (i, g) in gear.iter_mut().enumerate() {
            *g = u64::from_le_bytes([bytes[i * 8], bytes[i * 8 + 1], bytes[i * 8 + 2], bytes[i * 8 + 3], bytes[i * 8 + 4], bytes[i * 8 + 5], bytes[i * 8 + 6], bytes[i * 8 + 7]]);
        }
        let avg = avg.next_power_of_two().max(4096);
        let bits = avg.trailing_zeros();
        Chunker { gear, min: avg / 4, avg, max: avg * 4, mask_s: mask(bits + 2), mask_l: mask(bits - 2) }
    }

    /// the length of the first chunk of `data` (`last`: no more data follows)
    pub fn cut(&self, data: &[u8], last: bool) -> Option<usize> {
        let n = data.len();
        if n <= self.min {
            return last.then_some(n);
        }
        let end = n.min(self.max);
        let normal = self.avg.min(end);
        let mut fp = 0u64;
        let mut i = self.min;
        while i < normal {
            fp = (fp << 1).wrapping_add(self.gear[data[i] as usize]);
            if fp & self.mask_s == 0 {
                return Some(i + 1);
            }
            i += 1;
        }
        while i < end {
            fp = (fp << 1).wrapping_add(self.gear[data[i] as usize]);
            if fp & self.mask_l == 0 {
                return Some(i + 1);
            }
            i += 1;
        }
        // no boundary: max reached, or the data ends (only a boundary when nothing follows)
        if end == self.max || last { Some(end) } else { None }
    }

    /// split a whole buffer into chunk lengths
    pub fn split(&self, data: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos < data.len() {
            let n = self.cut(&data[pos..], true).unwrap_or(data.len() - pos);
            out.push(n);
            pos += n;
        }
        out
    }
}
