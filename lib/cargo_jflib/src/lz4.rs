// LZ4 blocks (the block format, no frame) for the SquashFS image of an AppImage: matches along
// hash chains, the longest one taken (as LZ4's high-compression mode does, without its optimal
// parse).  The format's end rules hold: the last 5 bytes are literals, no match starts in the
// last 12 bytes.

const MIN_MATCH: usize = 4;
const MAX_OFFSET: usize = 65535;
const HASH_BITS: u32 = 16;
const MAX_CHAIN: usize = 64;
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;

fn length(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

fn sequence(out: &mut Vec<u8>, literals: &[u8], m: Option<(usize, usize)>) {
    let lit = literals.len();
    let ml = m.map_or(0, |(len, _)| len - MIN_MATCH);
    out.push(((lit.min(15) as u8) << 4) | ml.min(15) as u8);
    if lit >= 15 {
        length(out, lit - 15);
    }
    out.extend_from_slice(literals);
    if let Some((_, offset)) = m {
        out.extend_from_slice(&(offset as u16).to_le_bytes());
        if ml >= 15 {
            length(out, ml - 15);
        }
    }
}

pub(crate) fn compress(src: &[u8]) -> Vec<u8> {
    let n = src.len();
    let mut out = Vec::with_capacity(n / 2 + 16);
    let mut head = vec![u32::MAX; 1 << HASH_BITS];
    let mut prev = vec![u32::MAX; MAX_OFFSET + 1];
    let hash = |i: usize| -> usize {
        let v = u32::from_le_bytes([src[i], src[i + 1], src[i + 2], src[i + 3]]);
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    };
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        let h = hash(i);
        prev[i % (MAX_OFFSET + 1)] = head[h];
        head[h] = i as u32;
    };
    let (mut i, mut anchor) = (0, 0);
    while n > MF_LIMIT && i + MF_LIMIT <= n {
        let max = n - LAST_LITERALS - i;
        let (mut best, mut dist) = (0, 0);
        let mut cand = head[hash(i)];
        let mut chain = MAX_CHAIN;
        while cand != u32::MAX && chain > 0 {
            let c = cand as usize;
            if c >= i || i - c > MAX_OFFSET {
                break;
            }
            if src[c + best.min(max - 1)] == src[i + best.min(max - 1)] {
                let len = src[c..c + max].iter().zip(&src[i..i + max]).take_while(|(a, b)| a == b).count();
                if len > best {
                    best = len;
                    dist = i - c;
                    if len == max {
                        break;
                    }
                }
            }
            let p = prev[c % (MAX_OFFSET + 1)];
            if p == u32::MAX || p as usize >= c {
                break;
            }
            cand = p;
            chain -= 1;
        }
        insert(i, &mut head, &mut prev);
        if best >= MIN_MATCH {
            sequence(&mut out, &src[anchor..i], Some((best, dist)));
            for k in i + 1..i + best {
                if k + MF_LIMIT <= n {
                    insert(k, &mut head, &mut prev);
                }
            }
            i += best;
            anchor = i;
        } else {
            i += 1;
        }
    }
    sequence(&mut out, &src[anchor..], None);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decompress(src: &[u8]) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut i = 0;
        loop {
            let token = src[i];
            i += 1;
            let mut lit = (token >> 4) as usize;
            if lit == 15 {
                loop {
                    lit += src[i] as usize;
                    i += 1;
                    if src[i - 1] != 255 {
                        break;
                    }
                }
            }
            out.extend_from_slice(&src[i..i + lit]);
            i += lit;
            if i == src.len() {
                return out;
            }
            let off = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
            i += 2;
            let mut ml = (token & 15) as usize;
            if ml == 15 {
                loop {
                    ml += src[i] as usize;
                    i += 1;
                    if src[i - 1] != 255 {
                        break;
                    }
                }
            }
            let from = out.len() - off;
            for k in 0..ml + MIN_MATCH {
                out.push(out[from + k]);
            }
        }
    }

    #[test]
    fn round_trip() {
        let mut text = Vec::new();
        let mut x = 7u32;
        for k in 0..300_000u32 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            text.push(if k % 5 < 3 { b"an AppImage of JFLib "[(k % 21) as usize] } else { (x >> 24) as u8 });
        }
        for data in [Vec::new(), b"abc".to_vec(), b"abcdabcdabcdabcdabcdabcdabcd".to_vec(), vec![7u8; 70_000], text.clone()] {
            let z = compress(&data);
            assert_eq!(decompress(&z), data);
        }
        // text with a changed byte now and then: matches of 4 bytes and more
        let mut words: Vec<u8> = b"an AppImage of JFLib carries its runtime and a SquashFS image ".repeat(5000);
        for k in (0..words.len()).step_by(97) {
            words[k] = (k % 251) as u8;
        }
        let z = compress(&words);
        assert_eq!(decompress(&z), words);
        assert!(z.len() < words.len() / 4);
    }
}
