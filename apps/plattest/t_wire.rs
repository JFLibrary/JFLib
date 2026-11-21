// t_wire -- (plat-shared) the decoders of other programs' data with a fixed-seed corpus (plat::fuzz):
//   plattest --test wirefuzz
// Valid data first (CF_UNICODETEXT, a DROPFILES block wide and ANSI, a text/uri-list, a DIB, CF_HTML; on
// Linux a D-Bus signal and Wayland events) with the answers the formats define, then random and mutated
// bytes through every decoder: nothing panics (a panic ends plattest), the lengths they report stay
// within the input, text comes out as UTF-8, an image as a BMP file.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::T;

fn wide(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn dropfiles(paths: &[&str], wide_names: bool) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [20u32, 0, 0, 0, wide_names as u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for p in paths {
        if wide_names {
            b.extend(wide(p));
            b.extend([0, 0]);
        } else {
            b.extend(p.bytes());
            b.push(0);
        }
    }
    b.extend(if wide_names { &[0u8, 0][..] } else { &[0u8][..] });
    b
}

// a 2 x 2 DIB, 32 bits, BI_RGB, with alpha
fn dib() -> Vec<u8> {
    let mut b = Vec::new();
    for v in [40u32, 2, 2] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, 16, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 128, 255, 0, 0, 255, 255, 255, 255, 0]);
    b
}

fn cf_html(fragment: &str) -> Vec<u8> {
    let pre = "<html><body><!--StartFragment-->";
    let post = "<!--EndFragment--></body></html>";
    // the header with 8-digit offsets: its length does not depend on them
    let head_len = format!("Version:0.9\r\nStartHTML:{:08}\r\nEndHTML:{:08}\r\nStartFragment:{:08}\r\nEndFragment:{:08}\r\n", 0, 0, 0, 0).len();
    let start = head_len + pre.len();
    let end = start + fragment.len();
    let total = end + post.len();
    let head = format!("Version:0.9\r\nStartHTML:{:08}\r\nEndHTML:{:08}\r\nStartFragment:{:08}\r\nEndFragment:{:08}\r\n", head_len, total, start, end);
    let mut b = head.into_bytes();
    b.extend(pre.bytes());
    b.extend(fragment.bytes());
    b.extend(post.bytes());
    b
}

// a D-Bus signal (little endian): path "/a", interface "x.y", member "M", signature "s", body "hi"
#[cfg(target_os = "linux")]
fn dbus_signal() -> Vec<u8> {
    let mut f: Vec<u8> = Vec::new();
    let mut field = |code: u8, sig: u8, val: &[u8]| {
        while f.len() % 8 != 0 {
            f.push(0);
        }
        f.extend([code, 1, sig, 0]);
        if sig == b'g' {
            f.push(val.len() as u8);
            f.extend(val);
            f.push(0);
        } else {
            f.extend((val.len() as u32).to_le_bytes());
            f.extend(val);
            f.push(0);
        }
    };
    field(1, b'o', b"/a");
    field(2, b's', b"x.y");
    field(3, b's', b"M");
    field(8, b'g', b"s");
    let mut body = Vec::new();
    body.extend(2u32.to_le_bytes());
    body.extend(b"hi\0");
    let mut m = vec![b'l', 4, 0, 1];
    m.extend((body.len() as u32).to_le_bytes());
    m.extend(1u32.to_le_bytes());
    m.extend((f.len() as u32).to_le_bytes());
    m.extend(&f);
    while m.len() % 8 != 0 {
        m.push(0);
    }
    m.extend(&body);
    m
}

// a Wayland event: object 3, opcode 1, the words of its body
#[cfg(target_os = "linux")]
fn wl_event(words: &[u32]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend(3u32.to_le_bytes());
    m.extend((1u32 | ((8 + 4 * words.len() as u32) << 16)).to_le_bytes());
    for w in words {
        m.extend(w.to_le_bytes());
    }
    m
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

// flip, drop, insert or overwrite a few bytes (lengths and counts get the values that break them)
fn mutate(r: &mut Rng, b: &mut Vec<u8>) {
    for _ in 0..1 + r.below(5) {
        let at = r.below(b.len() + 1);
        match r.below(5) {
            0 if at < b.len() => b[at] ^= 1 << r.below(8),
            1 if at < b.len() => {
                b.remove(at);
            }
            2 => b.insert(at, r.next() as u8),
            3 if at + 4 <= b.len() => {
                let v = [0u32, 1, 3, 4, 7, 8, 0x7fff_ffff, 0xffff_ffff, 0xffff, 0x1_0000][r.below(10)];
                b[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
            _ => {
                let n = r.below(b.len() + 1);
                b.truncate(n);
            }
        }
    }
}

pub fn t_wirefuzz(t: &mut T) {
    // the answers the formats define
    let mut text = wide("Hallo\r\nWelt");
    text.extend([0, 0]);
    text.extend(wide("dahinter"));
    t.check(plat::fuzz::clip_wide_text(&text).as_deref() == Some(&b"Hallo\nWelt"[..]), "CF_UNICODETEXT up to the NUL, CR LF as LF");
    let mut bad = wide("a");
    bad.extend([0x00, 0xD8, b'b', 0]); // a high surrogate, then 'b'
    t.check(plat::fuzz::clip_wide_text(&bad).as_deref() == Some("a\u{FFFD}b".as_bytes()), "a lone surrogate is U+FFFD");
    t.check(plat::fuzz::hdrop(&dropfiles(&["C:\\a", "C:\\b c"], true)) == vec![String::from("C:\\a"), String::from("C:\\b c")], "DROPFILES, wide");
    t.check(plat::fuzz::hdrop(&dropfiles(&["/x", "/y"], false)) == vec![String::from("/x"), String::from("/y")], "DROPFILES, ANSI");
    let mut off = dropfiles(&["C:\\a"], true);
    off[0..4].copy_from_slice(&1000u32.to_le_bytes());
    t.check(plat::fuzz::hdrop(&off).is_empty(), "DROPFILES with an offset outside the block: no path");
    plat::drop_reset();
    t.check(plat::drop_uri_list(b"file:///tmp/a%20b\r\n# a comment\r\nfile://localhost/x\r\nhttp://elsewhere/y\r\n") == 2, "text/uri-list: two local files");
    t.check(plat::drop_take() == vec![String::from("/tmp/a b"), String::from("/x")], "... decoded");
    let bmp = plat::clip_dib_to_bmp(&dib());
    t.check(bmp.as_ref().is_some_and(|b| b.starts_with(b"BM") && u32::from_le_bytes([b[2], b[3], b[4], b[5]]) as usize == b.len()), "a DIB as a BMP file");
    t.check(plat::clip_html_decode(&cf_html("<b>fett</b>")).is_some_and(|h| String::from_utf8_lossy(&h).contains("<b>fett</b>")), "CF_HTML: the fragment");
    #[cfg(target_os = "linux")]
    {
        let sig = dbus_signal();
        t.check(plat::fuzz::dbus(&sig) == Ok(Some(sig.len())), "a D-Bus signal decodes to its length");
        t.check(plat::fuzz::dbus(&sig[..sig.len() - 1]) == Ok(None), "... one byte short: incomplete");
        t.check(plat::fuzz::wl_args(&wl_event(&[1, 2, 3])) > 0, "a Wayland event of three words");
        let mut s = wl_event(&[6]);
        s.extend(b"seat0\0\0\0");
        s[6..8].copy_from_slice(&20u16.to_le_bytes());
        t.check(plat::fuzz::wl_args(&s) > 0, "a Wayland event with a string");
    }

    // the corpus: every decoder with random and mutated bytes
    let mut seeds: Vec<Vec<u8>> = vec![text, bad, dropfiles(&["C:\\a", "C:\\b"], true), dropfiles(&["/x"], false), dib(), cf_html("x<i>y</i>"),
                                       b"file:///a\r\nfile://localhost/b%zz\r\n".to_vec()];
    if let Some(b) = bmp {
        seeds.push(b);
    }
    #[cfg(target_os = "linux")]
    {
        seeds.push(dbus_signal());
        seeds.push(wl_event(&[1, 0xffff_fff0, 3]));
    }
    let mut r = Rng(0x7769_7265_6675_7a7a);
    let (mut cases, mut bad_len, mut bad_text, mut bad_bmp) = (0u64, 0u64, 0u64, 0u64);
    let n = if t.quiet { 20_000 } else { 100_000 };
    for k in 0..n {
        let mut b = if k % 5 == 4 {
            (0..r.below(300)).map(|_| r.next() as u8).collect()
        } else {
            seeds[r.below(seeds.len())].clone()
        };
        mutate(&mut r, &mut b);
        if let Some(s) = plat::fuzz::clip_wide_text(&b) {
            bad_text += String::from_utf8(s).is_err() as u64;
        }
        let _ = plat::fuzz::hdrop(&b);
        plat::drop_reset();
        let _ = plat::drop_uri_list(&b);
        let _ = plat::drop_take();
        if let Some(out) = plat::clip_dib_to_bmp(&b) {
            bad_bmp += !out.starts_with(b"BM") as u64;
        }
        if let Some(out) = plat::clip_bmp_fix(&b) {
            bad_bmp += !out.starts_with(b"BM") as u64;
        }
        let _ = plat::clip_html_decode(&b);
        let _ = plat::clipimg_split(&b);
        #[cfg(target_os = "linux")]
        {
            if let Ok(Some(len)) = plat::fuzz::dbus(&b) {
                bad_len += (len > b.len()) as u64;
            }
            let _ = plat::fuzz::wl_args(&b);
        }
        cases += 1;
    }
    crate::out(&format!("wirefuzz: {cases} cases (seed 0x776972656675_7a7a)\n"));
    t.check(bad_len == 0, &format!("corpus: lengths within the input ({bad_len} beyond)"));
    t.check(bad_text == 0, &format!("corpus: text is UTF-8 ({bad_text} not)"));
    t.check(bad_bmp == 0, &format!("corpus: images are BMP files ({bad_bmp} not)"));
}
