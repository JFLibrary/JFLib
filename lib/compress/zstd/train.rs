// train -- Dictionary::train: a dictionary from samples, as `zstd --train` makes them
//
// The content comes from fastCover (Liao, Petri, Moffat, Wirth: "Effective construction of
// relative Lempel-Ziv dictionaries", and zstd's lib/dictBuilder/fastcover.c): every d-byte
// string (d-mer) of the samples is counted in a table of 2^20 hashed slots; the samples are
// cut into epochs, and from each in turn the k-byte segment whose distinct d-mers are most
// frequent is taken (its d-mers count no more afterwards) and put before the segments taken
// earlier, until the content is full, so the most common strings end up nearest the data.
// k and d are chosen as zstd's optimizer does, on a smaller scale: each pair of k (64 to 2048)
// and d (6, 8) builds content from three quarters of the samples, and the one under which the
// last quarter compresses smallest wins.
//
// The tables then come from compressing every sample with that content at level 3: the
// histograms of the literals and of the three sequence codes, each symbol counted once more
// (as ZDICT does: every symbol keeps a code, so a frame may always use the tables again),
// make the Huffman code (11 bits, fewer when its description would not fit) and the FSE
// tables of the offset (accuracy 8), match length and literal length codes (9).  The ID is
// derived from the content's xxHash64 in zstd's range for unregistered dictionaries.

use alloc::vec;
use alloc::vec::Vec;

use super::dict::{Dictionary, Tab};
use super::enc::{compress_with_dict, sequences_of};
use super::fse;
use super::huf;
use super::seq::{ll_code, ml_code, of_code};
use crate::xxh::xxh64;

const F: u32 = 20;
const PRIME: u64 = 0xCF1B_BCDC_B7A5_6463;

/// the hash slot of the d-mer at p (8 bytes readable there: data is padded)
#[inline(always)]
fn slot(data: &[u8], p: usize, d: usize) -> usize {
    let v = u64::from_le_bytes(data[p..p + 8].try_into().unwrap());
    ((v << (64 - 8 * d)).wrapping_mul(PRIME) >> (64 - F)) as usize
}

/// fastCover content of at most `size` bytes from samples, segments of k bytes, d-mers
fn cover(samples: &[&[u8]], size: usize, k: usize, d: usize) -> Vec<u8> {
    let mut data: Vec<u8> = Vec::with_capacity(samples.iter().map(|s| s.len()).sum::<usize>() + 8);
    let mut freqs = vec![0u32; 1 << F];
    for s in samples {
        let at = data.len();
        data.extend_from_slice(s);
        data.extend_from_slice(&[0; 8]);
        if s.len() >= 8 {
            for p in at..at + s.len() - 7 {
                freqs[slot(&data, p, d)] += 1;
            }
        }
        data.truncate(at + s.len());
    }
    let total = data.len();
    data.extend_from_slice(&[0; 8]);
    if total < 8 || k <= d {
        return Vec::new();
    }
    let dmers = total - 7;
    let mut num = (size / k).max(1);
    let mut esize = dmers / num;
    if esize < 10 * k {
        esize = (10 * k).min(dmers);
        num = (dmers / esize).max(1);
    }
    let max_zero = (num >> 3).clamp(10, 100);
    let in_k = k - d + 1;
    let mut seg = vec![0u16; 1 << F];
    let mut out = vec![0u8; size];
    let (mut tail, mut zero_run, mut epoch) = (size, 0, 0);
    while tail > 0 {
        let b = epoch * esize;
        let e = (b + esize).min(dmers);
        epoch = (epoch + 1) % num;
        // the best segment of the epoch: distinct d-mers of a window of k bytes
        let (mut ab, mut ae, mut score) = (b, b, 0u64);
        let mut best = (b, b, 0u64);
        while ae < e {
            let i = slot(&data, ae, d);
            if seg[i] == 0 {
                score += freqs[i] as u64;
            }
            seg[i] += 1;
            ae += 1;
            if ae - ab == in_k + 1 {
                let j = slot(&data, ab, d);
                seg[j] -= 1;
                if seg[j] == 0 {
                    score -= freqs[j] as u64;
                }
                ab += 1;
            }
            if score > best.2 {
                best = (ab, ae, score);
            }
        }
        while ab < ae {
            seg[slot(&data, ab, d)] = 0;
            ab += 1;
        }
        if best.2 == 0 {
            zero_run += 1;
            if zero_run >= max_zero {
                break;
            }
            continue;
        }
        zero_run = 0;
        // without the d-mers at its ends that no longer count; they count no more anywhere
        let (mut nb, mut ne) = (best.1, best.0);
        for p in best.0..best.1 {
            if freqs[slot(&data, p, d)] != 0 {
                nb = nb.min(p);
                ne = p + 1;
            }
        }
        for p in nb..ne {
            freqs[slot(&data, p, d)] = 0;
        }
        let n = (ne - nb + d - 1).min(tail).min(total - nb);
        if n < d {
            break;
        }
        tail -= n;
        out[tail..tail + n].copy_from_slice(&data[nb..nb + n]);
    }
    out.drain(..tail);
    out
}

/// the tables of content for the samples, and the dictionary of both
fn finalize(content: Vec<u8>, samples: &[&[u8]]) -> Option<Dictionary> {
    let mut lit = [1u32; 256];
    let mut ll = [1u32; 36];
    let mut ml = [1u32; 53];
    let mut of = [1u32; 31];
    for s in samples {
        let (seqs, lits) = sequences_of(s, 3, &content);
        for &b in &lits {
            lit[b as usize] += 1;
        }
        for q in &seqs {
            ll[ll_code(q.ll) as usize] += 1;
            ml[ml_code(q.ml) as usize] += 1;
            of[(of_code(q.ov) as usize).min(30)] += 1;
        }
    }
    let code = (8..=huf::MAX_BITS).rev().find_map(|bits| {
        let c = huf::Code::build_max(&lit, bits)?;
        c.write_table(&mut Vec::new()).then_some(c)
    })?;
    let tab = |h: &[u32], log: u32| Tab::new(fse::normalize(h, log), log);
    let id = ((xxh64(&content, 0) >> 11) as u32 % ((1u32 << 31) - 32768)) + 32768;
    Dictionary::build(id, &code, tab(&of, 8)?, tab(&ml, 9)?, tab(&ll, 9)?, content)
}

/// a dictionary of at most max_size bytes (at least 256) from samples -> None without
/// samples to learn from
pub(super) fn train(samples: &[&[u8]], max_size: usize) -> Option<Dictionary> {
    let total: usize = samples.iter().map(|s| s.len()).sum();
    if max_size < 256 || total < 64 || samples.is_empty() {
        return None;
    }
    // room for the tables (they take a few hundred bytes)
    let size = (max_size - max_size.min(1024).min(max_size / 4)).min(total);
    let (learn, test): (Vec<&[u8]>, Vec<&[u8]>) = if samples.len() >= 4 {
        let cut = samples.len() * 3 / 4;
        (samples[..cut].to_vec(), samples[cut..].to_vec())
    } else {
        (samples.to_vec(), samples.to_vec())
    };
    let mut best: Option<(usize, usize, usize)> = None;
    for k in [64usize, 128, 256, 512, 1024, 2048] {
        if k > size.max(64) {
            break;
        }
        for d in [6usize, 8] {
            let c = cover(&learn, size, k, d);
            if c.is_empty() {
                continue;
            }
            let raw = Dictionary::raw(&c);
            let bytes: usize = test.iter().map(|s| compress_with_dict(s, 3, &raw).len()).sum();
            if best.is_none_or(|b| bytes < b.0) {
                best = Some((bytes, k, d));
            }
        }
    }
    let (_, k, d) = best?;
    // the content fills what the tables leave (their size known after a first round); one
    // that ends up too large loses its first, least common bytes
    let first = finalize(cover(samples, size, k, d), samples)?;
    let tables = first.to_bytes().len() - first.content.len();
    let mut content = cover(samples, (max_size - tables).min(total), k, d);
    for _ in 0..3 {
        let dict = finalize(content.clone(), samples)?;
        let n = dict.to_bytes().len();
        if n <= max_size {
            return Some(dict);
        }
        content.drain(..(n - max_size).min(content.len()));
    }
    Some(first)
}
