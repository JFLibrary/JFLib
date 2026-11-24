// encoding_ref -- the reference for formats::encoding: encoding_rs (Firefox's implementation of the WHATWG
// Encoding Standard), built offline from Cargo's registry by tools/encoding_check.py.  Not part of JFLib.
//   encoding_ref index OUT                  the multi-byte indexes as JSON: name -> [code points of each pointer]
//   encoding_ref decode LABEL IN OUT        IN decoded at once, without the byte-order mark rule
//   encoding_ref decode-chunks LABEL IN OUT the same, one byte at a time
//   encoding_ref encode LABEL IN OUT        each character of IN alone: a line of hex bytes or "-"
//   encoding_ref encode-text LABEL IN OUT   all of IN; an unmappable character: its code point on stdout, exit 3
//   encoding_ref encode-html LABEL IN OUT   all of IN with character references
use encoding_rs::*;
use std::fmt::Write as _;
use std::io::Write as _;

fn dec(e: &'static Encoding, b: &[u8]) -> String {
    e.decode_without_bom_handling(b).0.into_owned()
}

// None for malformed input (U+FFFD from the index stays)
fn strict(e: &'static Encoding, b: &[u8]) -> String {
    e.decode_without_bom_handling_and_without_replacement(b).map(|c| c.into_owned()).unwrap_or_default()
}

fn esc(s: &str) -> String {
    let mut o = String::from("[");
    for (i, c) in s.chars().enumerate() {
        let _ = write!(o, "{}{}", if i > 0 { "," } else { "" }, c as u32);
    }
    o.push(']');
    o
}

fn index(out: &str) {
    let mut j = String::from("{");
    // jis0208 through Shift_JIS
    let mut v = Vec::new();
    for p in 0..11280u32 {
        if (8836..10716).contains(&p) {
            v.push(String::new());
            continue;
        }
        let (l, t) = (p / 188, p % 188);
        let lb = l + if l < 0x1F { 0x81 } else { 0xC1 };
        let tb = t + if t < 0x3F { 0x40 } else { 0x41 };
        let s = strict(SHIFT_JIS, &[lb as u8, tb as u8]);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "jis0208", &v);
    // jis0208 through EUC-JP (pointers below 8836)
    let mut v = Vec::new();
    for p in 0..8836u32 {
        let s = strict(EUC_JP, &[(p / 94 + 0xA1) as u8, (p % 94 + 0xA1) as u8]);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "jis0208_euc", &v);
    let mut v = Vec::new();
    for p in 0..8836u32 {
        let s = strict(EUC_JP, &[0x8F, (p / 94 + 0xA1) as u8, (p % 94 + 0xA1) as u8]);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "jis0212", &v);
    let mut v = Vec::new();
    for p in 0..126 * 190u32 {
        let s = strict(EUC_KR, &[(p / 190 + 0x81) as u8, (p % 190 + 0x41) as u8]);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "euc_kr", &v);
    let mut v = Vec::new();
    for p in 0..126 * 157u32 {
        let t = p % 157;
        let s = strict(BIG5, &[(p / 157 + 0x81) as u8, (t + if t < 0x3F { 0x40 } else { 0x62 }) as u8]);
        v.push(s);
    }
    push(&mut j, "big5", &v);
    let mut v = Vec::new();
    for p in 0..126 * 190u32 {
        let t = p % 190;
        let s = strict(GB18030, &[(p / 190 + 0x81) as u8, (t + if t < 0x3F { 0x40 } else { 0x41 }) as u8]);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "gb18030", &v);
    let mut v = Vec::new();
    for p in 0..39420u32 {
        let b = [(p / 12600 + 0x81) as u8, (p / 1260 % 10 + 0x30) as u8, (p / 10 % 126 + 0x81) as u8, (p % 10 + 0x30) as u8];
        let s = strict(GB18030, &b);
        v.push(if s.chars().count() == 1 { s } else { String::new() });
    }
    push(&mut j, "gb18030_four", &v);
    j.pop();
    j.push('}');
    std::fs::write(out, j).unwrap();
}

fn push(j: &mut String, name: &str, v: &[String]) {
    let _ = write!(j, "\"{name}\":[");
    for (i, s) in v.iter().enumerate() {
        if i > 0 {
            j.push(',');
        }
        j.push_str(&esc(s));
    }
    j.push_str("],");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "index" => index(&a[2]),
        "decode" => {
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let b = std::fs::read(&a[3]).unwrap();
            std::fs::write(&a[4], dec(e, &b)).unwrap();
        }
        "decode-bom" => {
            // the standard's decode: a byte-order mark wins
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let b = std::fs::read(&a[3]).unwrap();
            std::fs::write(&a[4], e.decode(&b).0.as_bytes()).unwrap();
        }
        "decode-chunks" => {
            // LABEL IN OUT: the same with a decoder fed one byte at a time
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let b = std::fs::read(&a[3]).unwrap();
            let mut d = e.new_decoder_without_bom_handling();
            let mut s = String::with_capacity(b.len() * 4 + 16);
            for (i, x) in b.iter().enumerate() {
                s.reserve(16);
                let (_r, _n, _) = d.decode_to_string(std::slice::from_ref(x), &mut s, i + 1 == b.len());
            }
            if b.is_empty() {
                let _ = d.decode_to_string(&[], &mut s, true);
            }
            std::fs::write(&a[4], s).unwrap();
        }
        "encode" => {
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let t = std::fs::read_to_string(&a[3]).unwrap();
            let mut o = std::io::BufWriter::new(std::fs::File::create(&a[4]).unwrap());
            let mut buf = [0u8; 16];
            for c in t.chars() {
                let mut enc = e.new_encoder();
                let mut s = [0u8; 4];
                let (r, _read, written) = enc.encode_from_utf8_without_replacement(c.encode_utf8(&mut s), &mut buf, true);
                match r {
                    EncoderResult::InputEmpty => {
                        for x in &buf[..written] {
                            write!(o, "{x:02x}").unwrap();
                        }
                        writeln!(o).unwrap();
                    }
                    _ => writeln!(o, "-").unwrap(),
                }
            }
            // (a failed write ends in a panic, exit code 101: never a short file with exit code 0)
            o.flush().unwrap();
        }
        "encode-html" => {
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let t = std::fs::read_to_string(&a[3]).unwrap();
            std::fs::write(&a[4], e.encode(&t).0).unwrap();
        }
        "encode-text" => {
            // LABEL IN OUT: the whole text, unmappable -> exit 3 with the code point
            let e = Encoding::for_label(a[2].as_bytes()).unwrap();
            let t = std::fs::read_to_string(&a[3]).unwrap();
            let mut enc = e.new_encoder();
            let mut out = vec![0u8; t.len() * 8 + 16];
            let (r, _read, written) = enc.encode_from_utf8_without_replacement(&t, &mut out, true);
            match r {
                EncoderResult::InputEmpty => std::fs::write(&a[4], &out[..written]).unwrap(),
                EncoderResult::Unmappable(c) => {
                    println!("{:x}", c as u32);
                    std::process::exit(3);
                }
                EncoderResult::OutputFull => panic!(),
            }
        }
        _ => panic!(),
    }
}
