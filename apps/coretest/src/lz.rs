// lz -- lib/compress: xxHash32/64 vectors, LZ4 block and frame and zstd round trips on many
// shapes of data and levels, corrupt input, and the file modes for tools/lz4_check.py and
// tools/zstd_check.py (interop with the zstd tool)

use alloc::vec::Vec;

use compress::{lz4, xxh32, xxh64, zstd};

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

fn samples() -> Vec<(&'static str, Vec<u8>)> {
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
