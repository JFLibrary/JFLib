// lzs -- compress::stream: every decoder and encoder in pieces against the one-shot functions
//
// Input in pieces of 1 byte to all at once and output buffers of 1 byte to 1 MiB give the
// one-shot functions' data; the encoders give the same bytes as compress() where they
// promise to (DEFLATE, zlib and gzip up to 256 KiB, zstd, LZ4 and Brotli streams with their
// size), every Flush::Sync point decodes to all input before it, damaged and truncated
// streams fail exactly when the one-shot decoders do, bytes after a stream are not taken, the
// pumps keep their limits, Xxh32 / Xxh64 in pieces equal xxh32 / xxh64.  The file mode
// --stream FORMAT c|d IN OUT [LEVEL] reads and writes in pieces of 64 KiB, c1 / d1 in one
// piece (for tools/stream_check.py and tools/brotli_check.py: the gzip and zstd tools,
// libbrotli, Brotli streams of WOFF2 fonts).

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use compress::stream::{decode_all, encode_all, Decode, Encode, Error, Flush, Status};
use compress::{brotli, deflate, gzip, lz4, xxh32, xxh64, zlib, zstd, Xxh32, Xxh64};

use crate::lz::{brotli_plain, samples, BROTLI};
use crate::T;

/// xorshift, reproducible
struct R(u64);

impl R {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// decode `data` in pieces (sizes 1..=cuts, 0 = all at once) into buffers of osz bytes ->
/// (output, input taken); several streams in a row are followed while input is taken
fn dec(d: &mut dyn Decode, data: &[u8], r: &mut R, cuts: usize, osz: usize) -> Result<(Vec<u8>, usize), Error> {
    let mut out = Vec::new();
    let mut ob = vec![0u8; osz];
    let mut p = 0;
    let mut idle = 0;
    loop {
        let n = if cuts == 0 { data.len() - p } else { (r.below(cuts) + 1).min(data.len() - p) };
        let s = d.decode(&data[p..p + n], &mut ob)?;
        if s.read > n || (s.status == Status::Input && s.read != n) {
            return Err(Error::Io); // a broken promise of the decoder
        }
        out.extend_from_slice(&ob[..s.written]);
        p += s.read;
        idle = if s.read == 0 && s.written == 0 { idle + 1 } else { 0 };
        if idle > 2 {
            return Err(Error::Io); // no progress
        }
        match s.status {
            Status::End if p == data.len() || (s.read == 0 && s.written == 0) => return Ok((out, p)),
            Status::Input if p == data.len() => {
                let s = d.decode(&[], &mut ob)?;
                out.extend_from_slice(&ob[..s.written]);
                if s.status == Status::Output {
                    continue;
                }
                if s.status != Status::End {
                    d.finish()?;
                }
                return Ok((out, p));
            }
            _ => {}
        }
    }
}

/// encode in pieces, Flush::Sync after every `sync`-th piece (0: never) -> (stream, the
/// (input, stream) lengths at each sync point)
fn enc(e: &mut dyn Encode, data: &[u8], r: &mut R, cuts: usize, osz: usize, sync: usize) -> Result<(Vec<u8>, Vec<(usize, usize)>), Error> {
    let mut out = Vec::new();
    let mut ob = vec![0u8; osz];
    let (mut p, mut piece) = (0, 0);
    let mut marks = Vec::new();
    loop {
        let n = if cuts == 0 { data.len() - p } else { (r.below(cuts) + 1).min(data.len() - p) };
        let last = p + n == data.len();
        piece += 1;
        let flush = if last {
            Flush::Finish
        } else if sync > 0 && piece % sync == 0 {
            Flush::Sync
        } else {
            Flush::None
        };
        let mut q = p;
        loop {
            let s = e.encode(&data[q..p + n], &mut ob, flush)?;
            out.extend_from_slice(&ob[..s.written]);
            q += s.read;
            match s.status {
                Status::Output => {}
                Status::Input if q == p + n => break,
                Status::End if last && q == p + n => return Ok((out, marks)),
                _ => return Err(Error::Io),
            }
        }
        p += n;
        if flush == Flush::Sync {
            marks.push((p, out.len()));
        }
    }
}

// the shapes of data: lz.rs's samples (the 9 MiB one only in a few runs) and data whose
// matches reach 9 MiB back (inside a zstd window only with --long)
fn shapes() -> Vec<(&'static str, Vec<u8>)> {
    let mut v = samples();
    let mut far: Vec<u8> = (0..3u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 20) as u8).collect();
    let copy = far.clone();
    far.extend(core::iter::repeat_n(7u8, 6 << 20));
    far.extend_from_slice(&copy);
    v.push(("far 15 MiB", far));
    v
}

// (pieces, output buffer) runs; a large input only gets the coarse ones
fn cutsets(len: usize) -> &'static [(usize, usize)] {
    if len > 1 << 20 {
        &[(0, 1 << 20), (300_000, 70_000)]
    } else if len > 100_000 {
        &[(0, 1 << 20), (5000, 7), (70_000, 100_000), (100, 1)]
    } else {
        &[(0, 1 << 20), (1, 1), (3, 7), (100, 1000), (5000, 70_000)]
    }
}

fn flate_one(fmt: &str, data: &[u8], level: i32) -> Vec<u8> {
    match fmt {
        "deflate" => deflate::compress(data, level),
        "zlib" => zlib::compress(data, level),
        _ => gzip::compress(data, level),
    }
}

fn flate_dec(fmt: &str) -> Box<dyn Decode> {
    match fmt {
        "deflate" => Box::new(deflate::Decoder::new()),
        "zlib" => Box::new(zlib::Decoder::new()),
        _ => Box::new(gzip::Decoder::new()),
    }
}

fn flate_enc(fmt: &str, level: i32) -> Box<dyn Encode> {
    match fmt {
        "deflate" => Box::new(deflate::Encoder::new(level)),
        "zlib" => Box::new(zlib::Encoder::new(level)),
        _ => Box::new(gzip::Encoder::new(level)),
    }
}

fn flate_back(fmt: &str, s: &[u8], max: usize) -> Option<Vec<u8>> {
    match fmt {
        "deflate" => deflate::decompress(s, max),
        "zlib" => zlib::decompress(s, max),
        _ => gzip::decompress(s, max),
    }
}

// one damaged copy of b: a flipped bit, a cut or a random byte
fn damage(b: &[u8], r: &mut R) -> Vec<u8> {
    let mut m = b.to_vec();
    match r.below(3) {
        0 => {
            let at = r.below(m.len());
            m[at] ^= 1 << r.below(8);
        }
        1 => m.truncate(r.below(m.len())),
        _ => {
            let at = r.below(m.len());
            m[at] = r.next() as u8;
        }
    }
    m
}

pub fn run(t: &mut T) {
    let mut r = R(0x5DEE_CE66_D1CE_4E5B);
    let shapes = shapes();
    flate(t, &mut r, &shapes);
    zstd_run(t, &mut r, &shapes);
    lz4_run(t, &mut r, &shapes);
    brotli_run(t, &mut r);
    brotli_enc_run(t, &mut r, &shapes);
    pumps(t, &mut r);
    hashes(t, &mut r);
}

fn flate(t: &mut T, r: &mut R, shapes: &[(&'static str, Vec<u8>)]) {
    for (name, data) in shapes.iter().filter(|(n, _)| !n.starts_with("far")) {
        for level in [0, 1, 6, 9] {
            if data.len() > 1 << 20 && level != 6 {
                continue;
            }
            for fmt in ["deflate", "zlib", "gzip"] {
                let one = flate_one(fmt, data, level);
                let mut ok = true;
                for &(cuts, osz) in cutsets(data.len()) {
                    let got = dec(&mut *flate_dec(fmt), &one, r, cuts, osz);
                    ok &= got.as_ref().is_ok_and(|(o, used)| o == data && *used == one.len());
                }
                t.check(&format!("{fmt} decoder {name} level {level}"), ok);
                // the encoder: pieces, output buffers, sync points
                let mut same = true;
                let mut back = true;
                let mut synced = true;
                for &(cuts, osz) in cutsets(data.len()) {
                    let sync = if cuts == 5000 || cuts == 300_000 { 3 } else { 0 };
                    let Ok((st, marks)) = enc(&mut *flate_enc(fmt, level), data, r, cuts, osz, sync) else {
                        back = false;
                        continue;
                    };
                    back &= flate_back(fmt, &st, data.len()).as_deref() == Some(&data[..]);
                    if data.len() <= 256 << 10 && sync == 0 {
                        same &= st == one;
                    }
                    for &(ip, op) in marks.iter().take(4) {
                        // a decoder of the stream so far gives back everything before the point
                        let mut d = flate_dec(fmt);
                        let mut o = vec![0u8; ip + 1];
                        let (mut q, mut got) = (0, 0);
                        while let Ok(s) = d.decode(&st[q..op], &mut o[got..]) {
                            q += s.read;
                            got += s.written;
                            if s.status != Status::Output {
                                break;
                            }
                        }
                        synced &= got == ip && o[..ip] == data[..ip];
                    }
                }
                t.check(&format!("{fmt} encoder {name} level {level}"), back);
                if data.len() <= 256 << 10 {
                    t.check(&format!("{fmt} encoder {name} level {level}: the bytes of compress()"), same);
                }
                if data.len() > 100_000 {
                    t.check(&format!("{fmt} encoder {name} level {level}: Flush::Sync points"), synced);
                }
            }
        }
    }
    // larger than a piece: a little larger than compress() (each piece of 256 KiB ends a
    // block; 9 MiB that shrink 500 times: 19 439 bytes instead of 18 785)
    let text = &shapes.iter().find(|s| s.0 == "big 9 MiB").unwrap().1;
    let (st, _) = enc(&mut deflate::Encoder::new(6), text, r, 0, 1 << 20, 0).unwrap();
    let one = deflate::compress(text, 6);
    let pieces = text.len().div_ceil(256 << 10);
    t.check("deflate encoder 9 MiB: at most 2% and 32 bytes a piece more than compress()", st.len() * 100 <= one.len() * 102 + pieces * 3200);
    // damaged streams fail exactly where the one-shot decoders fail
    let base = zlib::compress(&shapes[8].1, 6);
    let gz = gzip::compress(&shapes[8].1, 6);
    let mut agree = true;
    for i in 0..3000 {
        let (fmt, src) = if i % 2 == 0 { ("zlib", &base) } else { ("gzip", &gz) };
        let m = damage(src, r);
        let a = flate_back(fmt, &m, 1 << 22);
        let b = dec(&mut *flate_dec(fmt), &m, r, 1 + i % 3000, 1 + i % 5000).map(|(o, _)| o);
        agree &= a.as_ref().ok_or(()) == b.as_ref().map_err(|_| ());
    }
    t.check("zlib and gzip decoders: 3000 damaged streams as the one-shot decoders", agree);
    // the input after a stream, several members, single()
    let one = zlib::compress(b"hallo welt", 6);
    let mut more = one.clone();
    more.extend_from_slice(b"rest");
    let mut o = [0u8; 64];
    let s = zlib::Decoder::new().decode(&more, &mut o);
    t.check("zlib decoder: the input after the stream is not taken", s.is_ok_and(|s| s.status == Status::End && s.read == one.len() && s.written == 10));
    let raw = deflate::compress(b"hallo welt", 6);
    let mut more = raw.clone();
    more.extend_from_slice(&[0xFF, 0xFF]);
    let s = deflate::Decoder::new().decode(&more, &mut o);
    t.check("deflate decoder: stops at the end of the last block", s.is_ok_and(|s| s.status == Status::End && s.read == raw.len()));
    let mut two = gzip::compress(b"hello\n", 6);
    let first = two.len();
    two.extend_from_slice(&gzip::compress(b"world\n", 9));
    t.check("gzip decoder: two members", dec(&mut gzip::Decoder::new(), &two, r, 3, 2).is_ok_and(|(o, _)| o == b"hello\nworld\n"));
    let s = gzip::Decoder::single().decode(&two, &mut o);
    t.check("gzip decoder single(): one member", s.is_ok_and(|s| s.status == Status::End && s.read == first && &o[..6] == b"hello\n"));
    let mut junk = two.clone();
    junk.extend_from_slice(b"junk");
    t.check("gzip decoder: bytes that are no member", dec(&mut gzip::Decoder::new(), &junk, r, 0, 64) == Err(Error::Corrupt));
    t.check("gzip decoder: empty input is no gzip file", gzip::Decoder::new().finish() == Err(Error::Truncated));
    // the optional header fields of a member
    let mut h = vec![0x1F, 0x8B, 8, 0x1E, 0, 0, 0, 0, 0, 3, 4, 0, b'a', b'b', b'c', b'd'];
    h.extend_from_slice(b"name.txt\0a comment\0");
    let c = compress::crc32(0, &h) as u16;
    h.extend_from_slice(&c.to_le_bytes());
    let g = gzip::compress(b"fields", 6);
    h.extend_from_slice(&g[10..]);
    t.check("gzip decoder: FEXTRA, FNAME, FCOMMENT, FHCRC", dec(&mut gzip::Decoder::new(), &h, r, 1, 1).is_ok_and(|(o, _)| o == b"fields") && gzip::decompress(&h, 100).as_deref() == Some(&b"fields"[..]));
    let n = h.len();
    h[n - 30] ^= 1; // the header CRC no longer matches
    t.check("gzip decoder: a wrong FHCRC", dec(&mut gzip::Decoder::new(), &h, r, 0, 64).is_err());
    let truncated = &one[..one.len() - 1];
    t.check("zlib decoder: a truncated stream", dec(&mut zlib::Decoder::new(), truncated, r, 0, 64) == Err(Error::Truncated));
    t.check("zlib decoder: a preset dictionary", zlib::Decoder::new().decode(&[0x78, 0xbb, 0, 0, 0, 1, 3, 0], &mut o) == Err(Error::Unsupported));
    // a reset decoder and encoder work as new ones
    let mut d = gzip::Decoder::new();
    let _ = d.decode(&gz[..gz.len() / 2], &mut vec![0u8; 1 << 20]);
    d.reset();
    t.check("gzip decoder reset()", dec(&mut d, &gz, r, 0, 1 << 20).is_ok_and(|(o, _)| o == shapes[8].1));
    let mut e = zlib::Encoder::new(6);
    let _ = e.encode(&shapes[6].1, &mut vec![0u8; 1000], Flush::None);
    e.reset();
    let small = &shapes[6].1[..100_000];
    t.check("zlib encoder reset()", enc(&mut e, small, r, 0, 1 << 20, 0).is_ok_and(|(s, _)| s == zlib::compress(small, 6)));
}

fn zstd_run(t: &mut T, r: &mut R, shapes: &[(&'static str, Vec<u8>)]) {
    for (name, data) in shapes {
        for level in [1, 3, 9] {
            if data.len() > 1 << 20 && level == 9 {
                continue;
            }
            let one = zstd::compress(data, level);
            let mut ok = true;
            for &(cuts, osz) in cutsets(data.len()) {
                ok &= dec(&mut zstd::Decoder::new(), &one, r, cuts, osz).is_ok_and(|(o, u)| o == *data && u == one.len());
            }
            t.check(&format!("zstd decoder {name} level {level}"), ok);
            let (mut same, mut back) = (true, true);
            for &(cuts, osz) in cutsets(data.len()).iter().take(3) {
                let sized = enc(&mut zstd::Encoder::new(level).with_size(data.len() as u64), data, r, cuts, osz, 0);
                same &= sized.as_ref().is_ok_and(|(s, _)| *s == one);
                let plain = enc(&mut zstd::Encoder::new(level), data, r, cuts, osz, 0);
                back &= plain.is_ok_and(|(s, _)| zstd::decompress(&s, data.len()).as_deref() == Some(&data[..]) && zstd::content_size(&s).is_none());
            }
            t.check(&format!("zstd encoder {name} level {level} with_size: the bytes of compress()"), same);
            t.check(&format!("zstd encoder {name} level {level} without a size"), back);
        }
    }
    let data = &shapes[8].1;
    let (st, marks) = enc(&mut zstd::Encoder::new(3), data, r, 20_000, 9000, 2).unwrap();
    let mut synced = !marks.is_empty();
    for &(ip, op) in &marks {
        let mut o = vec![0u8; ip + 10];
        let s = zstd::Decoder::new().decode(&st[..op], &mut o);
        synced &= s.is_ok_and(|s| s.written == ip && s.status == Status::Input) && o[..ip] == data[..ip];
    }
    t.check("zstd encoder: Flush::Sync points", synced);
    t.check("zstd encoder: a wrong size", enc(&mut zstd::Encoder::new(3).with_size(10), b"0123456789A", r, 0, 64, 0).err() == Some(Error::Size));
    t.check("zstd encoder: a short input", enc(&mut zstd::Encoder::new(3).with_size(12), b"0123456789A", r, 0, 64, 0).err() == Some(Error::Size));
    // the window: a limit, a large window with a small content
    let far = zstd::compress(&shapes.iter().find(|s| s.0 == "far 15 MiB").unwrap().1, 1);
    t.check("zstd decoder: a window above the limit", dec(&mut zstd::Decoder::new().with_window_max(1 << 20), &far, r, 0, 1 << 20).err() == Some(Error::Limit));
    // a frame that claims a window of 2^40 bytes for 3 bytes of content
    let mut tiny = zstd::compress(b"abc", 3);
    tiny.splice(4..6, [0x84u8, 0xF0, 3, 0, 0, 0]);
    let got = dec(&mut zstd::Decoder::new(), &tiny, r, 0, 64);
    t.check("zstd decoder: a huge window over a small content is no limit", got.is_ok_and(|(o, _)| o == b"abc") && zstd::decompress(&tiny, 10).as_deref() == Some(&b"abc"[..]));
    // frames, skippable frames, single()
    let mut two = zstd::compress(b"erster Teil, ", 3);
    let first = two.len();
    two.extend_from_slice(&[0x50, 0x2A, 0x4D, 0x18, 3, 0, 0, 0, 1, 2, 3]);
    two.extend_from_slice(&zstd::compress(b"zweiter Teil", 19));
    t.check("zstd decoder: two frames and a skippable one", dec(&mut zstd::Decoder::new(), &two, r, 2, 3).is_ok_and(|(o, _)| o == b"erster Teil, zweiter Teil"));
    let mut o = [0u8; 64];
    let s = zstd::Decoder::single().decode(&two, &mut o);
    t.check("zstd decoder single(): one frame", s.is_ok_and(|s| s.status == Status::End && s.read == first));
    t.check("zstd decoder: no frame is fine", zstd::Decoder::new().finish().is_ok());
    t.check("zstd decoder: a truncated frame", dec(&mut zstd::Decoder::new(), &two[..first - 1], r, 0, 64) == Err(Error::Truncated));
    let base = zstd::compress(&shapes[8].1, 3);
    let mut agree = true;
    for i in 0..2000 {
        let m = damage(&base, r);
        let a = zstd::decompress(&m, 1 << 22);
        let b = dec(&mut zstd::Decoder::new(), &m, r, 1 + i % 3000, 1 + i % 5000).map(|(o, _)| o);
        agree &= a.as_ref().ok_or(()) == b.as_ref().map_err(|_| ());
    }
    t.check("zstd decoder: 2000 damaged frames as decompress()", agree);
}

fn lz4_run(t: &mut T, r: &mut R, shapes: &[(&'static str, Vec<u8>)]) {
    for (name, data) in shapes.iter().filter(|(n, _)| !n.starts_with("far")) {
        let one = lz4::frame_compress(data);
        let mut ok = true;
        for &(cuts, osz) in cutsets(data.len()) {
            ok &= dec(&mut lz4::FrameDecoder::new(), &one, r, cuts, osz).is_ok_and(|(o, u)| o == *data && u == one.len());
        }
        t.check(&format!("lz4 frame decoder {name}"), ok);
        let (mut same, mut back) = (true, true);
        for &(cuts, osz) in cutsets(data.len()).iter().take(3) {
            let sized = enc(&mut lz4::FrameEncoder::new().with_size(data.len() as u64), data, r, cuts, osz, 0);
            same &= sized.as_ref().is_ok_and(|(s, _)| *s == one);
            let plain = enc(&mut lz4::FrameEncoder::new(), data, r, cuts, osz, 2);
            back &= plain.is_ok_and(|(s, _)| lz4::frame_decompress(&s, data.len()).as_deref() == Some(&data[..]));
        }
        t.check(&format!("lz4 frame encoder {name} with_size: the bytes of frame_compress()"), same);
        t.check(&format!("lz4 frame encoder {name} without a size, with Flush::Sync"), back);
    }
    let base = lz4::frame_compress(&shapes[8].1);
    let mut agree = true;
    for i in 0..2000 {
        let m = damage(&base, r);
        let a = lz4::frame_decompress(&m, 1 << 22);
        let b = dec(&mut lz4::FrameDecoder::new(), &m, r, 1 + i % 3000, 1 + i % 5000).map(|(o, _)| o);
        agree &= a.as_ref().ok_or(()) == b.as_ref().map_err(|_| ());
    }
    t.check("lz4 frame decoder: 2000 damaged frames as frame_decompress()", agree);
}

fn brotli_run(t: &mut T, r: &mut R) {
    for &(name, stream) in BROTLI {
        let want = brotli_plain(name);
        let mut ok = true;
        for &(cuts, osz) in cutsets(stream.len()) {
            ok &= dec(&mut brotli::Decoder::new(), stream, r, cuts, osz).is_ok_and(|(o, u)| o == want && u == stream.len());
        }
        t.check(&format!("brotli decoder {name}"), ok);
        let mut more = stream.to_vec();
        more.extend_from_slice(b"XYZ");
        t.check(&format!("brotli decoder {name}: the input after the stream is not taken"), dec(&mut brotli::Decoder::new(), &more, r, 2, 5).is_ok_and(|(o, u)| o == want && u == stream.len()));
        let mut cut_fails = true;
        for cut in 0..stream.len() {
            cut_fails &= dec(&mut brotli::Decoder::new(), &stream[..cut], r, 0, 64) == Err(Error::Truncated);
        }
        t.check(&format!("brotli decoder {name}: truncated"), cut_fails);
    }
    let mut agree = true;
    for i in 0..4000 {
        let m = damage(BROTLI[2 + i % 4].1, r);
        let a = brotli::decompress_prefix(&m, 1 << 20).ok();
        let b = dec(&mut brotli::Decoder::new(), &m, r, 1 + i % 50, 1 + i % 300);
        agree &= match (&a, &b) {
            (Some((o1, u1)), Ok((o2, u2))) => o1 == o2 && u1 == u2,
            (None, Err(_)) => true,
            _ => false,
        };
    }
    t.check("brotli decoder: 4000 damaged streams as decompress_prefix()", agree);
}

fn brotli_enc_run(t: &mut T, r: &mut R, shapes: &[(&'static str, Vec<u8>)]) {
    for (name, data) in shapes {
        for level in [1, 5, 9] {
            if data.len() > 1 << 20 && level != 1 {
                continue;
            }
            let one = brotli::compress(data, level);
            let mut ok = true;
            for &(cuts, osz) in cutsets(one.len()).iter().take(3) {
                ok &= dec(&mut brotli::Decoder::new(), &one, r, cuts, osz).is_ok_and(|(o, u)| o == *data && u == one.len());
            }
            t.check(&format!("brotli decoder {name} level {level}: our stream"), ok);
            let (mut same, mut back) = (true, true);
            for &(cuts, osz) in cutsets(data.len()).iter().take(3) {
                let sized = enc(&mut brotli::Encoder::new(level).with_size(data.len() as u64), data, r, cuts, osz, 0);
                same &= sized.as_ref().is_ok_and(|(s, _)| *s == one);
                let plain = enc(&mut brotli::Encoder::new(level), data, r, cuts, osz, 3);
                back &= plain.is_ok_and(|(s, _)| brotli::decompress(&s, data.len()).as_deref() == Some(&data[..]));
            }
            t.check(&format!("brotli encoder {name} level {level} with_size: the bytes of compress()"), same);
            t.check(&format!("brotli encoder {name} level {level} without a size, with Flush::Sync"), back);
        }
    }
    let data = &shapes[8].1;
    let (st, marks) = enc(&mut brotli::Encoder::new(5), data, r, 20_000, 9000, 2).unwrap();
    let mut synced = !marks.is_empty();
    for &(ip, op) in &marks {
        let mut o = vec![0u8; ip + 10];
        let s = brotli::Decoder::new().decode(&st[..op], &mut o);
        synced &= s.is_ok_and(|s| s.written == ip && s.status == Status::Input) && o[..ip] == data[..ip];
    }
    t.check("brotli encoder: Flush::Sync points", synced);
    // Sync with nothing new: no bytes; Sync twice in a row: the second adds none
    let mut e = brotli::Encoder::new(5);
    let mut ob = [0u8; 64];
    let a = e.encode(b"", &mut ob, Flush::Sync).unwrap();
    let b = e.encode(b"abc", &mut ob, Flush::Sync).unwrap();
    let c = e.encode(b"", &mut ob, Flush::Sync).unwrap();
    t.check("brotli encoder: Flush::Sync without new input", a.status == Status::Input && b.written > 0 && c.written == 0 && c.status == Status::Input);
    let small = enc(&mut brotli::Encoder::new(5).with_window(10), data, r, 3000, 500, 0).unwrap().0;
    t.check("brotli encoder with_window(10)", small[0] & 0x7F == 0x21 && brotli::decompress(&small, data.len()).as_deref() == Some(&data[..]));
    t.check("brotli encoder: a wrong size", enc(&mut brotli::Encoder::new(5).with_size(10), b"0123456789A", r, 0, 64, 0).err() == Some(Error::Size));
    t.check("brotli encoder: a short input", enc(&mut brotli::Encoder::new(5).with_size(12), b"0123456789A", r, 0, 64, 0).err() == Some(Error::Size));
    t.check("brotli encoder: nothing", enc(&mut brotli::Encoder::new(5), b"", r, 0, 1, 0).is_ok_and(|(s, _)| brotli::decompress(&s, 0).is_some_and(|o| o.is_empty())));
}

fn pumps(t: &mut T, r: &mut R) {
    let data = &samples()[8].1;
    let mut src = 0;
    let mut packed = Vec::new();
    let n = encode_all(
        &mut gzip::Encoder::new(6),
        |b| {
            let k = (r.below(5000) + 1).min(b.len()).min(data.len() - src);
            b[..k].copy_from_slice(&data[src..src + k]);
            src += k;
            Some(k)
        },
        |b| {
            packed.extend_from_slice(b);
            true
        },
    );
    t.check("encode_all", n == Ok(packed.len() as u64) && gzip::decompress(&packed, data.len()).as_deref() == Some(&data[..]));
    let feed = |s: &[u8], at: &mut usize, b: &mut [u8]| {
        let k = b.len().min(s.len() - *at);
        b[..k].copy_from_slice(&s[*at..*at + k]);
        *at += k;
        Some(k)
    };
    let (mut at, mut back) = (0, Vec::new());
    let n = decode_all(&mut gzip::Decoder::new(), |b| feed(&packed, &mut at, b), |b| {
        back.extend_from_slice(b);
        true
    }, u64::MAX);
    t.check("decode_all", n == Ok(data.len() as u64) && back == *data);
    let mut at = 0;
    t.check("decode_all: the limit", decode_all(&mut gzip::Decoder::new(), |b| feed(&packed, &mut at, b), |_| true, 1000) == Err(Error::Limit));
    let mut at = 0;
    t.check("decode_all: a failing writer", decode_all(&mut gzip::Decoder::new(), |b| feed(&packed, &mut at, b), |_| false, u64::MAX) == Err(Error::Io));
    t.check("decode_all: a failing reader", decode_all(&mut gzip::Decoder::new(), |_| None, |_| true, u64::MAX) == Err(Error::Io));
    let mut tail = deflate::compress(b"abc", 6);
    tail.push(0);
    let mut at = 0;
    t.check("decode_all: bytes after a stream", decode_all(&mut deflate::Decoder::new(), |b| feed(&tail, &mut at, b), |_| true, u64::MAX) == Err(Error::Corrupt));
    let mut at = 0;
    let cut = &packed[..packed.len() - 3];
    t.check("decode_all: a truncated stream", decode_all(&mut gzip::Decoder::new(), |b| feed(cut, &mut at, b), |_| true, u64::MAX) == Err(Error::Truncated));
    // a bomb of zeros in buffers of 64 KiB: bounded by the writer, not by memory
    let bomb = gzip::compress(&vec![0u8; 64 << 20], 9);
    let (mut at, mut total) = (0, 0u64);
    let n = decode_all(&mut gzip::Decoder::new(), |b| feed(&bomb, &mut at, b), |b| {
        total += b.len() as u64;
        true
    }, u64::MAX);
    t.check("decode_all: 64 MiB of zeros from a small gzip", n == Ok(64 << 20) && total == 64 << 20 && bomb.len() < 100_000);
}

fn hashes(t: &mut T, r: &mut R) {
    let data: Vec<u8> = (0..100_000).map(|_| r.next() as u8).collect();
    let mut ok = true;
    for _ in 0..50 {
        let n = r.below(data.len());
        let d = &data[..n];
        let (mut a, mut b) = (Xxh32::new(7), Xxh64::new(7));
        let mut p = 0;
        while p < n {
            let k = (r.below(70) + 1).min(n - p);
            a.update(&d[p..p + k]);
            b.update(&d[p..p + k]);
            p += k;
        }
        ok &= a.digest() == xxh32(d, 7) && b.digest() == xxh64(d, 7);
    }
    t.check("Xxh32 and Xxh64 in pieces equal xxh32 and xxh64", ok);
    t.check("Xxh32 of nothing", Xxh32::new(0).digest() == xxh32(b"", 0) && Xxh64::new(0).digest() == xxh64(b"", 0));
}

/// --stream FORMAT c|d IN OUT [LEVEL]: compress or decompress a file in pieces of 64 KiB, c1 /
/// d1 in one piece (FORMAT deflate, zlib, gzip, zstd, lz4, brotli; for tools/stream_check.py
/// and tools/brotli_check.py)
pub fn file_mode(fmt: &str, op: &str, input: &str, output: &str, level: i32) -> i32 {
    if op == "c1" {
        // the one-shot encoder
        let Some(data) = plat::read_file(input) else { return 2 };
        let out = match fmt {
            "deflate" => deflate::compress(&data, level),
            "zlib" => zlib::compress(&data, level),
            "gzip" => gzip::compress(&data, level),
            "zstd" => zstd::compress(&data, level),
            "lz4" => lz4::frame_compress(&data),
            "brotli" => brotli::compress(&data, level),
            _ => return 2,
        };
        return if plat::write_file(output, &out) { 0 } else { 1 };
    }
    if op == "d1" {
        // the one-shot decoder, to hold the streams against
        let Some(data) = plat::read_file(input) else { return 2 };
        let max = 1 << 30;
        let out = match fmt {
            "deflate" => deflate::decompress(&data, max),
            "zlib" => zlib::decompress(&data, max),
            "gzip" => gzip::decompress(&data, max),
            "zstd" => zstd::decompress(&data, max),
            "lz4" => lz4::frame_decompress(&data, max),
            "brotli" => brotli::decompress(&data, max),
            _ => return 2,
        };
        return match out {
            Some(o) if plat::write_file(output, &o) => 0,
            _ => 1,
        };
    }
    let (Some(fin), Some(fout)) = (plat::File::open(input, plat::FO_READ), plat::File::open(output, plat::FO_WRITE)) else {
        return 2;
    };
    let (mut rd, mut wr) = (0u64, 0u64);
    let read = |b: &mut [u8]| {
        let n = fin.read_at(rd, b)?;
        rd += n as u64;
        Some(n)
    };
    let write = |b: &[u8]| {
        let ok = fout.write_at(wr, b) == Some(b.len());
        wr += b.len() as u64;
        ok
    };
    let r = if op == "c" {
        let mut e: Box<dyn Encode> = match fmt {
            "deflate" => Box::new(deflate::Encoder::new(level)),
            "zlib" => Box::new(zlib::Encoder::new(level)),
            "gzip" => Box::new(gzip::Encoder::new(level)),
            "zstd" => Box::new(zstd::Encoder::new(level)),
            "lz4" => Box::new(lz4::FrameEncoder::new()),
            "brotli" => Box::new(brotli::Encoder::new(level)),
            _ => return 2,
        };
        encode_all(&mut *e, read, write)
    } else {
        let mut d: Box<dyn Decode> = match fmt {
            "deflate" => Box::new(deflate::Decoder::new()),
            "zlib" => Box::new(zlib::Decoder::new()),
            "gzip" => Box::new(gzip::Decoder::new()),
            "zstd" => Box::new(zstd::Decoder::new()),
            "lz4" => Box::new(lz4::FrameDecoder::new()),
            "brotli" => Box::new(brotli::Decoder::new()),
            _ => return 2,
        };
        decode_all(&mut *d, read, write, u64::MAX)
    };
    match r {
        Ok(_) => 0,
        Err(e) => {
            plat::write_stdout(format!("{fmt} {op}: {e}\n").as_bytes());
            1
        }
    }
}
