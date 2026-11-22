// lz -- lib/compress: xxHash32/64, CRC-32 and Adler-32 vectors, LZ4 block and frame, zstd,
// DEFLATE, zlib and gzip round trips on many shapes of data and levels, streams of zlib, gzip
// and libbrotlienc (Brotli), corrupt, truncated and trailing input, and the file modes for
// tools/lz4_check.py and tools/zstd_check.py (interop with the zstd tool)

use alloc::vec::Vec;

use compress::{adler32, brotli, crc32, deflate, gzip, lz4, xxh32, xxh64, zlib, zstd};

use crate::T;

/// a reproducible pseudo-random generator (xorshift)
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

pub(crate) fn samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let text: Vec<u8> = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ".iter().copied().cycle().take(200_000).collect();
    let random: Vec<u8> = (0..300_000).map(|_| r.next() as u8).collect();
    let mut mixed = Vec::new();
    for i in 0..2000 {
        if i % 3 == 0 {
            mixed.extend((0..97).map(|_| r.next() as u8));
        } else {
            mixed.extend_from_slice(&text[i * 7 % 1000..i * 7 % 1000 + 131]);
        }
    }
    let mut v = alloc::vec![
        ("empty", Vec::new()),
        ("one byte", alloc::vec![7]),
        ("12 bytes", b"abcdabcdabcd".to_vec()),
        ("13 bytes", b"abcdabcdabcda".to_vec()),
        ("zeros 1 MiB", alloc::vec![0u8; 1 << 20]),
        ("run 70000", alloc::vec![b'x'; 70_000]),
        ("text", text),
        ("random", random),
        ("mixed", mixed),
    ];
    // longer than one frame block (4 MiB)
    v.push(("big 9 MiB", (0..9 << 20).map(|i: usize| (i / 3000 % 251) as u8 ^ (i as u8 & 1)).collect()));
    v
}

pub fn run(t: &mut T) {
    t.eq("xxh32 empty", xxh32(b"", 0), 0x02CC_5D05);
    t.eq("xxh32 abc", xxh32(b"abc", 0), 0x32D1_53FF);
    t.eq("xxh32 seed", xxh32(b"", 1), 0x0B2C_B792);
    t.eq("xxh64 empty", xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
    t.eq("xxh64 abc", xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
    t.eq("lz4 empty block", lz4::compress(b""), alloc::vec![0u8]);
    for (name, data) in samples() {
        let c = lz4::compress(&data);
        t.check(&alloc::format!("lz4 bound {}", name), c.len() <= lz4::bound(data.len()));
        let d = lz4::decompress(&c, data.len());
        t.check(&alloc::format!("lz4 block {}", name), d.as_deref() == Some(&data[..]));
        let f = lz4::frame_compress(&data);
        let d = lz4::frame_decompress(&f, data.len() + 1);
        t.check(&alloc::format!("lz4 frame {}", name), d.as_deref() == Some(&data[..]));
        if name == "text" {
            t.check("lz4 text compresses", c.len() < data.len() / 10);
        }
        if name == "random" {
            t.check("lz4 random stays small", c.len() <= data.len() + data.len() / 200 + 16);
        }
    }
    // limits and corruption: never more output than allowed, no panic on any truncation
    let data = b"hello hello hello hello hello hello hello hello hello world".to_vec();
    let c = lz4::compress(&data);
    t.check("lz4 max_out", lz4::decompress(&c, data.len() - 1).is_none());
    let mut all_ok = true;
    for cut in 0..c.len() {
        if lz4::decompress(&c[..cut], 1 << 20).as_deref() == Some(&data[..]) {
            all_ok = false;
        }
    }
    t.check("lz4 truncated blocks fail", all_ok);
    t.check("lz4 offset 0", lz4::decompress(&[0x10, b'a', 0, 0, 0x50, 1, 2, 3, 4, 5], 100).is_none());
    t.check("lz4 offset past start", lz4::decompress(&[0x10, b'a', 2, 0, 0x50, 1, 2, 3, 4, 5], 100).is_none());
    let f = lz4::frame_compress(&data);
    let mut bad = f.clone();
    let n = bad.len();
    bad[n - 1] ^= 1;
    t.check("lz4 frame checksum", lz4::frame_decompress(&bad, 1000).is_none());
    let mut bad = f.clone();
    bad[5] ^= 0x40; // header checksum no longer matches
    t.check("lz4 frame header checksum", lz4::frame_decompress(&bad, 1000).is_none());
    zstd_run(t);
    let mut fuzz = Rng(42);
    let mut no_panic = 0;
    for _ in 0..2000 {
        let mut b = c.clone();
        let k = (fuzz.next() as usize) % b.len();
        b[k] = fuzz.next() as u8;
        let _ = lz4::decompress(&b, 4096);
        no_panic += 1;
    }
    t.eq("lz4 mutated blocks", no_panic, 2000);
    checksum_run(t);
    flate_run(t);
    brotli_run(t);
}

// Adler-32 byte by byte, as RFC 1950 defines it
fn adler_ref(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

fn checksum_run(t: &mut T) {
    t.eq("crc32 empty", crc32(0, b""), 0);
    t.eq("crc32 check string", crc32(0, b"123456789"), 0xCBF4_3926);
    t.eq("crc32 continued", crc32(crc32(0, b"12345"), b"6789"), 0xCBF4_3926);
    t.eq("adler32 empty", adler32(1, b""), 1);
    t.eq("adler32 Wikipedia", adler32(1, b"Wikipedia"), 0x11E6_0398);
    let mut sound = true;
    for (name, data) in samples() {
        if adler32(1, &data) != adler_ref(&data) {
            sound = false;
            t.check(&alloc::format!("adler32 {}", name), false);
        }
        // split anywhere, continued: the same value
        let k = data.len() / 3;
        if adler32(adler32(1, &data[..k]), &data[k..]) != adler_ref(&data) {
            sound = false;
            t.check(&alloc::format!("adler32 continued {}", name), false);
        }
    }
    let ff = alloc::vec![0xFFu8; 100_003];
    t.check("adler32 all 0xFF (largest sums)", adler32(1, &ff) == adler_ref(&ff));
    t.check("adler32 against the byte-by-byte definition", sound);
}

fn flate_run(t: &mut T) {
    // streams of other encoders: zlib.compress (Python), raw deflate (zlib, wbits -15), gzip -n -9
    let z: &[u8] = &[0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x01, 0x68, 0x03, 0x08, 0xb1];
    t.check("zlib of Python", zlib::decompress(z, 100).as_deref() == Some(&b"hello hello hello hello"[..]));
    t.check("zlib of Python, empty", zlib::decompress(&[0x78, 0xda, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01], 10).as_deref() == Some(&b""[..]));
    let raw: &[u8] = &[0x2b, 0x4a, 0x2c, 0x57, 0x48, 0x49, 0x4d, 0xcb, 0x49, 0x2c, 0x49, 0xd5, 0x51, 0x28, 0xc2, 0xc1, 0x01, 0x00];
    t.check("deflate of zlib", deflate::decompress(raw, 100).as_deref() == Some(&b"raw deflate, raw deflate, raw deflate"[..]));
    let gz: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0xe7, 0x02, 0x00, 0x20,
        0x30, 0x3a, 0x36, 0x06, 0x00, 0x00, 0x00,
    ];
    t.check("gzip of gzip -n", gzip::decompress(gz, 100).as_deref() == Some(&b"hello\n"[..]));
    t.eq("gzip stored size", gzip::stored_size(gz), Some(6));
    let mut two = gz.to_vec();
    two.extend_from_slice(&gzip::compress(b"world\n", 6));
    t.check("gzip two members", gzip::decompress(&two, 100).as_deref() == Some(&b"hello\nworld\n"[..]));
    t.check("gzip member consumed", gzip::decompress_member_into(None, &two, &mut [0u8; 64]) == Ok((6, gz.len())));
    t.check("gzip empty input", gzip::decompress(b"", 100).is_none());
    // a preset dictionary (FDICT) is refused
    t.check("zlib FDICT refused", zlib::decompress_into(None, &[0x78, 0xbb, 0, 0, 0, 1, 3, 0], &mut [0u8; 16]) == Err(deflate::Error::Header));
    // round trips on every shape of data, the bounds, the reusable states
    let mut dst = deflate::DeflateState::new();
    let mut ist = deflate::InflateState::new();
    for (name, data) in samples() {
        for level in [0, 1, 6, 9] {
            let d = deflate::compress(&data, level);
            let zl = zlib::compress(&data, level);
            let g = gzip::compress(&data, level);
            t.check(&alloc::format!("deflate bound {} {}", name, level), zl.len() <= deflate::bound(data.len()));
            t.check(&alloc::format!("deflate {} level {}", name, level), deflate::decompress(&d, data.len()).as_deref() == Some(&data[..]));
            t.check(&alloc::format!("zlib {} level {}", name, level), zlib::decompress(&zl, data.len()).as_deref() == Some(&data[..]));
            t.check(&alloc::format!("gzip {} level {}", name, level), gzip::decompress(&g, data.len()).as_deref() == Some(&data[..]));
            t.check(&alloc::format!("deflate state {} {}", name, level), deflate::compress_with(Some(&mut dst), &data, level) == d);
            if level == 6 {
                let mut out = alloc::vec![0u8; data.len()];
                let r = zlib::decompress_into(Some(&mut ist), &zl, &mut out);
                t.check(&alloc::format!("zlib into {}", name), r == Ok((data.len(), zl.len())) && out == data);
                if name == "text" {
                    t.check("deflate text compresses", d.len() < data.len() / 50);
                }
                if name == "random" {
                    t.check("deflate random stays small", d.len() <= data.len() + data.len() / 1000 + 64);
                }
            }
        }
    }
    // limits, trailing bytes, truncation and corruption
    let data: Vec<u8> = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ".iter().copied().cycle().take(5000).collect();
    let zl = zlib::compress(&data, 6);
    let g = gzip::compress(&data, 6);
    let d = deflate::compress(&data, 6);
    t.check("zlib max_out", zlib::decompress(&zl, data.len() - 1).is_none());
    t.check("gzip max_out", gzip::decompress(&g, data.len() - 1).is_none());
    t.check("deflate max_out", deflate::decompress(&d, data.len() - 1).is_none());
    let mut more = zl.clone();
    more.push(0);
    t.check("zlib trailing byte refused", zlib::decompress(&more, 10_000).is_none());
    let mut more = g.clone();
    more.push(0);
    t.check("gzip trailing byte refused", gzip::decompress(&more, 10_000).is_none());
    let mut more = d.clone();
    more.push(0);
    t.check("deflate trailing byte refused", deflate::decompress(&more, 10_000).is_none());
    let mut bad = zl.clone();
    let n = bad.len();
    bad[n - 1] ^= 1;
    t.check("zlib Adler-32", zlib::decompress(&bad, 10_000).is_none());
    let mut bad = g.clone();
    let n = bad.len();
    bad[n - 6] ^= 1;
    t.check("gzip CRC-32", gzip::decompress(&bad, 10_000).is_none());
    let mut all_ok = true;
    for cut in 0..zl.len() {
        if zlib::decompress(&zl[..cut], 1 << 20).is_some() || deflate::decompress(&d[..cut.min(d.len() - 1)], 1 << 20).is_some() {
            all_ok = false;
        }
    }
    for cut in 0..g.len() {
        if gzip::decompress(&g[..cut], 1 << 20).is_some() {
            all_ok = false;
        }
    }
    t.check("deflate, zlib and gzip truncated streams fail", all_ok);
    let mut fuzz = Rng(11);
    let mut sound = true;
    for k in 0..3000 {
        let mut b = if k % 2 == 0 { zl.clone() } else { g.clone() };
        for _ in 0..1 + k % 3 {
            let i = (fuzz.next() as usize) % b.len();
            b[i] ^= 1 << (fuzz.next() % 8);
        }
        let out = if k % 2 == 0 { zlib::decompress(&b, 8192) } else { gzip::decompress(&b, 8192) };
        if out.is_some_and(|o| o.len() > 8192) {
            sound = false;
        }
        let _ = deflate::decompress(&b, 8192);
    }
    t.check("zlib, gzip and deflate mutated streams", sound);
    // a zlib bomb within max_out grows its buffer, past it stops
    let bomb = zlib::compress(&alloc::vec![0u8; 8 << 20], 9);
    t.check("zlib 8 MiB of zeros", zlib::decompress(&bomb, 8 << 20).is_some_and(|o| o.len() == 8 << 20));
    t.check("zlib 8 MiB of zeros, 1 MiB allowed", zlib::decompress(&bomb, 1 << 20).is_none());
}

// streams of libbrotlienc 1.2 (BrotliEncoderCompress: quality, lgwin 22, mode generic or text)
pub(crate) static BROTLI: &[(&str, &[u8])] = &[
    ("empty", &[0x06]),
    ("hello", &[0x0b, 0x02, 0x80, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0x03]),
    ("text q1", &[
        0x8b, 0x9b, 0x00, 0x00, 0x80, 0xaa, 0xaa, 0xaa, 0xea, 0xff, 0x74, 0x65, 0xb8, 0x1b, 0xd8, 0xe5, 0x60, 0x00, 0x76,
        0x38, 0xdb, 0x59, 0xc1, 0x84, 0xcd, 0x4c, 0x40, 0x15, 0xd8, 0x0c, 0x44, 0x45, 0xc0, 0x60, 0x03, 0x6e, 0x19, 0xd8,
        0xc7, 0x81, 0x31, 0x1c, 0xcc, 0x87, 0x8d, 0x31, 0xcf, 0xd0, 0x68, 0xd8, 0x5d, 0xd8, 0xda, 0xd9, 0xeb, 0x83, 0xcf,
        0x04, 0x63, 0x26, 0xac, 0x49, 0x60, 0x34, 0x58, 0xf0, 0x53, 0x82, 0x39, 0x71, 0x38, 0x8d, 0xf1, 0x2c, 0xc5, 0x38,
        0x46, 0xe9, 0xc6, 0x55, 0xea, 0xe9, 0xaf, 0x9c, 0x91, 0x2b, 0x1e, 0x2f, 0x62,
    ]),
    ("text q11", &[
        0x1b, 0x37, 0x01, 0xf8, 0xc5, 0x5c, 0x5b, 0xaa, 0x47, 0x17, 0x4b, 0x10, 0x0c, 0xd9, 0xfc, 0xb8, 0x53, 0xc7, 0x3f,
        0xa7, 0x6c, 0x7b, 0xac, 0x8f, 0x46, 0x2c, 0x14, 0xba, 0x80, 0xdd, 0x85, 0xad, 0x9d, 0x7d, 0x7e, 0xf0, 0x2c, 0x30,
        0xb2, 0x60, 0x4d, 0x02, 0x4d, 0xd2, 0x82, 0x9f, 0x12, 0xcc, 0x89, 0xc3, 0x69, 0x8c, 0x87, 0x91, 0xc5, 0x31, 0xa6,
        0x6e, 0x5c, 0x53, 0xbd, 0xfc, 0x95, 0x33, 0x6a, 0x45,
    ]),
    ("words q11", &[
        0x1b, 0x58, 0x00, 0x88, 0x1c, 0x09, 0x76, 0xec, 0xc8, 0x3a, 0x87, 0x8c, 0x1e, 0xa9, 0xa8, 0xd0, 0x40, 0x0f, 0x96,
        0xea, 0x1e, 0x8a, 0x9a, 0xa0, 0x68, 0x0a, 0x2a, 0xec, 0x0d, 0x36, 0xe0, 0xc0, 0x21, 0x0b, 0x64, 0x7f, 0x21, 0xbf,
        0x41, 0xa7, 0x15, 0xce, 0x0c, 0x3c, 0x4e, 0xd4, 0xbc, 0x14, 0x32, 0x08, 0x77, 0xda, 0x1f, 0xbe, 0x0c, 0xb1, 0x02,
    ]),
    ("html q9 text mode", &[
        0x1b, 0x45, 0x00, 0x00, 0x44, 0x27, 0x87, 0x78, 0x90, 0xef, 0x18, 0xb4, 0x84, 0x8a, 0x0e, 0x95, 0x07, 0x5c, 0xfc,
        0xb0, 0x46, 0xa4, 0x54, 0xa3, 0x12, 0x6c, 0x8c, 0x9d, 0x63, 0x20, 0x8d, 0x27, 0x2a, 0x09, 0x52, 0xfb, 0x8d, 0xdb,
        0x65, 0x94, 0x3b, 0x0f, 0xb6, 0x14, 0x46, 0xf3, 0xfb, 0xa2, 0xfe, 0x23, 0xa8, 0x09, 0x07,
    ]),
];

pub(crate) fn brotli_plain(name: &str) -> Vec<u8> {
    let text = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ";
    match name {
        "empty" => Vec::new(),
        "hello" => b"hello".to_vec(),
        "text q1" | "text q11" => text.iter().copied().cycle().take(4 * text.len()).collect(),
        "words q11" => b"The quick brown fox jumps over the lazy dog. The quick brown fox jumps over the lazy dog!".to_vec(),
        _ => b"<html><head><title>JFLib</title></head><body>compression</body></html>".to_vec(),
    }
}

fn brotli_run(t: &mut T) {
    for &(name, stream) in BROTLI {
        let want = brotli_plain(name);
        t.check(&alloc::format!("brotli {}", name), brotli::decompress(stream, 1000).as_deref() == Some(&want[..]));
        if !want.is_empty() {
            t.check(&alloc::format!("brotli limit {}", name), brotli::decompress_prefix(stream, want.len() - 1).err() == Some(brotli::Error::Limit));
        }
        let mut more = stream.to_vec();
        more.push(0);
        t.check(&alloc::format!("brotli trailing byte {}", name), brotli::decompress(&more, 1000).is_none());
        t.check(&alloc::format!("brotli prefix {}", name), brotli::decompress_prefix(&more, 1000).is_ok_and(|(o, used)| o == want && used == stream.len()));
        let mut all_ok = true;
        for cut in 0..stream.len() {
            if brotli::decompress(&stream[..cut], 1000).is_some_and(|o| o == want) {
                all_ok = false;
            }
        }
        t.check(&alloc::format!("brotli truncated {}", name), all_ok);
    }
    let mut fuzz = Rng(13);
    let mut sound = true;
    for k in 0..4000 {
        let mut b = BROTLI[2 + k % 4].1.to_vec();
        for _ in 0..1 + k % 3 {
            let i = (fuzz.next() as usize) % b.len();
            b[i] ^= 1 << (fuzz.next() % 8);
        }
        if brotli::decompress(&b, 4096).is_some_and(|o| o.len() > 4096) {
            sound = false;
        }
    }
    t.check("brotli mutated streams", sound);
}

fn zstd_run(t: &mut T) {
    for (name, data) in samples() {
        for level in [1, 3, 9] {
            let c = zstd::compress(&data, level);
            t.check(&alloc::format!("zstd bound {} {}", name, level), c.len() <= zstd::bound(data.len()));
            t.check(&alloc::format!("zstd size {} {}", name, level), zstd::content_size(&c) == Some(data.len() as u64));
            let d = zstd::decompress(&c, data.len());
            t.check(&alloc::format!("zstd {} level {}", name, level), d.as_deref() == Some(&data[..]));
            if name == "text" {
                t.check(&alloc::format!("zstd text compresses {}", level), c.len() < data.len() / 50);
                t.check(&alloc::format!("zstd beats lz4 on text {}", level), c.len() <= lz4::compress(&data).len());
            }
            if name == "mixed" && level == 3 {
                t.check("zstd beats lz4 on mixed data", c.len() < lz4::compress(&data).len());
            }
            if name == "random" {
                t.check("zstd random stays small", c.len() <= data.len() + 64);
            }
        }
    }
    // several frames and a skippable frame in between
    let mut two = zstd::compress(b"erster Teil, ", 3);
    two.extend_from_slice(&[0x50, 0x2A, 0x4D, 0x18, 3, 0, 0, 0, 1, 2, 3]);
    two.extend_from_slice(&zstd::compress(b"zweiter Teil", 3));
    t.check("zstd concatenated frames", zstd::decompress(&two, 100).as_deref() == Some(&b"erster Teil, zweiter Teil"[..]));
    let data: Vec<u8> = b"Ein Backup ist erst dann ein Backup, wenn die Wiederherstellung funktioniert. ".iter().copied().cycle().take(5000).collect();
    let c = zstd::compress(&data, 3);
    t.check("zstd max_out", zstd::decompress(&c, data.len() - 1).is_none());
    let mut bad = c.clone();
    let n = bad.len();
    bad[n - 1] ^= 1;
    t.check("zstd checksum", zstd::decompress(&bad, 10_000).is_none());
    let mut all_ok = true;
    for cut in 0..c.len() {
        if zstd::decompress(&c[..cut], 1 << 20).is_some_and(|d| d == data) {
            all_ok = false;
        }
    }
    t.check("zstd truncated frames fail", all_ok);
    // corruption anywhere: no panic, never more than max_out
    let mut fuzz = Rng(7);
    let mut sound = true;
    for k in 0..3000 {
        let mut b = c.clone();
        let flips = 1 + k % 3;
        for _ in 0..flips {
            let i = 4 + (fuzz.next() as usize) % (b.len() - 4);
            b[i] ^= 1 << (fuzz.next() % 8);
        }
        if zstd::decompress(&b, 8192).is_some_and(|d| d.len() > 8192) {
            sound = false;
        }
    }
    t.check("zstd mutated frames", sound);
}

/// --zstd c|d IN OUT [LEVEL]: compress or decompress a file (for tools/zstd_check.py)
pub fn zstd_file_mode(op: &str, input: &str, output: &str, level: i32) -> i32 {
    let Some(data) = plat::read_file(input) else { return 2 };
    let out = match op {
        "c" => Some(zstd::compress(&data, level)),
        _ => zstd::decompress(&data, 1 << 30),
    };
    match out {
        Some(o) if plat::write_file(output, &o) => 0,
        _ => 1,
    }
}

/// --lz4 c|d IN OUT: frame-compress or -decompress a file (for tools/lz4_check.py)
pub fn file_mode(op: &str, input: &str, output: &str) -> i32 {
    let Some(data) = plat::read_file(input) else { return 2 };
    let out = match op {
        "c" => Some(lz4::frame_compress(&data)),
        _ => lz4::frame_decompress(&data, 1 << 30),
    };
    match out {
        Some(o) if plat::write_file(output, &o) => 0,
        _ => 1,
    }
}
