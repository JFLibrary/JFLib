// The stream filters (ISO 32000-1, 7.4): FlateDecode (zlib, or raw DEFLATE as some writers
// leave it), LZWDecode, ASCIIHexDecode, ASCII85Decode, RunLengthDecode, the PNG and TIFF
// predictors of the first two.  Damaged data gives what came before the damage, as Acrobat and
// pdf.js do: a content stream cut short still shows its first part.  The image filters (DCT,
// JPX, JBIG2, CCITTFax) are not decoded here; the chain stops before them.  Every filter stops
// at `limit` bytes of output.

use super::object::{Dict, Name, Object};
use super::Error;
use alloc::vec;
use alloc::vec::Vec;
use compress::stream::{Decode, Status};

/// an image filter the chain stopped before
pub(crate) fn is_image_filter(n: &[u8]) -> bool {
    matches!(n, b"DCTDecode" | b"DCT" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode" | b"CCF")
}

/// the filters of a stream dictionary (or an inline image's) with their parameters; names and
/// parameter dictionaries must be resolved already
pub(crate) fn chain(dict: &Dict) -> Vec<(Name, Dict)> {
    let f = dict.get("Filter").or_else(|| dict.get("F"));
    let p = dict.get("DecodeParms").or_else(|| dict.get("DP"));
    let parm = |i: usize| -> Dict {
        match p {
            Some(Object::Dict(d)) if i == 0 => d.clone(),
            Some(Object::Array(a)) => a.get(i).and_then(Object::as_dict).cloned().unwrap_or_default(),
            _ => Dict::new(),
        }
    };
    match f {
        Some(Object::Name(n)) => vec![(n.clone(), parm(0))],
        Some(Object::Array(a)) => a.iter().enumerate().filter_map(|(i, o)| match o {
            Object::Name(n) => Some((n.clone(), parm(i))),
            _ => None,
        }).collect(),
        _ => Vec::new(),
    }
}

/// apply `filters` to `data` -> the bytes, and the image filter the chain stopped before
pub(crate) fn apply(filters: &[(Name, Dict)], mut data: Vec<u8>, limit: usize) -> Result<(Vec<u8>, Option<(Name, Dict)>), Error> {
    for (i, (name, parms)) in filters.iter().enumerate() {
        let n = name.as_bytes();
        if is_image_filter(n) {
            return Ok((data, Some((name.clone(), parms.clone()))));
        }
        data = match n {
            b"FlateDecode" | b"Fl" => predict(flate(&data, limit)?, parms, limit)?,
            b"LZWDecode" | b"LZW" => {
                let early = parms.int("EarlyChange").unwrap_or(1) != 0;
                predict(lzw(&data, early, limit)?, parms, limit)?
            }
            b"ASCIIHexDecode" | b"AHx" => ascii_hex(&data),
            b"ASCII85Decode" | b"A85" => ascii85(&data, limit)?,
            b"RunLengthDecode" | b"RL" => run_length(&data, limit)?,
            // the crypt filter was handled when the stream was decrypted
            b"Crypt" if i == 0 => data,
            _ => return Err(Error::Unsupported("a stream filter")),
        };
    }
    Ok((data, None))
}

fn zlib_header(d: &[u8]) -> bool {
    d.len() >= 2 && d[0] & 0x0F == 8 && d[0] >> 4 <= 7 && (u16::from(d[0]) << 8 | u16::from(d[1])) % 31 == 0
}

/// FlateDecode: everything up to the end of the stream or the first damage
pub(crate) fn flate(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    let mut zl;
    let mut raw;
    let dec: &mut dyn Decode = if zlib_header(data) {
        zl = compress::zlib::Decoder::new();
        &mut zl
    } else {
        raw = compress::deflate::Decoder::new();
        &mut raw
    };
    let mut off = 0;
    let mut damaged = false;
    loop {
        match dec.decode(&data[off..], &mut buf) {
            Ok(s) => {
                off += s.read;
                if out.len() + s.written > limit {
                    return Err(Error::Limit("a stream is larger than max_stream"));
                }
                out.extend_from_slice(&buf[..s.written]);
                match s.status {
                    Status::Output => {}
                    Status::End => break,
                    Status::Input => {
                        if s.read == 0 && s.written == 0 {
                            damaged = true; // cut short
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                // the stream decoder holds back at most a little of what it read before the
                // damage; a checksum that does not match comes after all the data
                damaged = e != compress::stream::Error::Checksum;
                break;
            }
        }
    }
    if out.is_empty() && damaged && !data.is_empty() {
        return Err(Error::Damaged("FlateDecode data"));
    }
    Ok(out)
}

/// LZWDecode (7.4.4): 9 to 12 bit codes, 256 clears the table, 257 ends the data; with
/// EarlyChange the code grows one code early
pub(crate) fn lzw(data: &[u8], early: bool, limit: usize) -> Result<Vec<u8>, Error> {
    // the table: each code is (prefix code, last byte, length)
    let mut prefix = vec![0u16; 4096];
    let mut last = vec![0u8; 4096];
    let mut len = vec![0u16; 4096];
    for i in 0..256 {
        last[i] = i as u8;
        len[i] = 1;
    }
    let mut next = 258usize;
    let mut width = 9u32;
    let mut prev: Option<u16> = None;
    let mut out = Vec::new();
    let (mut acc, mut bits, mut pos) = (0u32, 0u32, 0usize);
    let mut tmp = Vec::new();
    loop {
        while bits < width {
            if pos >= data.len() {
                return Ok(out);
            }
            acc = acc << 8 | data[pos] as u32;
            pos += 1;
            bits += 8;
        }
        let code = (acc >> (bits - width)) & ((1 << width) - 1);
        bits -= width;
        let code = code as usize;
        if code == 256 {
            next = 258;
            width = 9;
            prev = None;
            continue;
        }
        if code == 257 {
            return Ok(out);
        }
        let Some(p) = prev else {
            if code > 255 {
                return Ok(out); // damaged: a code before any is defined
            }
            out.push(code as u8);
            if out.len() > limit {
                return Err(Error::Limit("a stream is larger than max_stream"));
            }
            prev = Some(code as u16);
            continue;
        };
        // the string of `code` (or of prev + its first byte, for the code being defined)
        let known = code < next;
        if !known && code != next {
            return Ok(out); // damaged
        }
        let src = if known { code } else { p as usize };
        tmp.clear();
        let mut c = src;
        loop {
            tmp.push(last[c]);
            if len[c] <= 1 {
                break;
            }
            c = prefix[c] as usize;
        }
        tmp.reverse();
        let first = tmp[0];
        if !known {
            tmp.push(first);
        }
        if out.len() + tmp.len() > limit {
            return Err(Error::Limit("a stream is larger than max_stream"));
        }
        out.extend_from_slice(&tmp);
        if next < 4096 {
            prefix[next] = p;
            last[next] = first;
            len[next] = len[p as usize].saturating_add(1);
            next += 1;
        }
        let grow = if early { next + 1 } else { next };
        if grow >= 1 << width && width < 12 {
            width += 1;
        }
        prev = Some(code as u16);
    }
}

/// ASCIIHexDecode: hex digits up to ">", white space ignored, an odd last digit is followed by 0
pub(crate) fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut half: Option<u8> = None;
    for &c in data {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b'>' => break,
            _ => continue,
        };
        match half.take() {
            Some(h) => out.push(h << 4 | v),
            None => half = Some(v),
        }
    }
    if let Some(h) = half {
        out.push(h << 4);
    }
    out
}

/// ASCII85Decode: groups of five characters ! to u, z for four zeros, ~> ends; a last group
/// of n characters gives n - 1 bytes
pub(crate) fn ascii85(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let mut group = [0u8; 5];
    let mut n = 0;
    let mut start = 0;
    // a "<~" at the start is seen in the wild
    while start < data.len() && super::lex::is_ws(data[start]) {
        start += 1;
    }
    if data[start..].starts_with(b"<~") {
        start += 2;
    }
    for &c in &data[start..] {
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group[n] = c - b'!';
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
                    out.extend_from_slice(&(v as u32).to_be_bytes());
                    n = 0;
                }
            }
            _ if super::lex::is_ws(c) => {}
            _ => break, // damaged
        }
        if out.len() > limit {
            return Err(Error::Limit("a stream is larger than max_stream"));
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
        out.extend_from_slice(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Ok(out)
}

/// RunLengthDecode: 0..127 copies n + 1 bytes, 129..255 repeats the next byte 257 - n times, 128 ends
pub(crate) fn run_length(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let n = data[i] as usize;
        i += 1;
        if n < 128 {
            let end = (i + n + 1).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else if n > 128 {
            let Some(&b) = data.get(i) else { break };
            i += 1;
            out.resize(out.len() + 257 - n, b);
        } else {
            break;
        }
        if out.len() > limit {
            return Err(Error::Limit("a stream is larger than max_stream"));
        }
    }
    Ok(out)
}

/// the predictors of FlateDecode and LZWDecode (7.4.4.4): 2 is TIFF's horizontal differencing,
/// 10 to 15 PNG's filters with a type byte before each row
pub(crate) fn predict(data: Vec<u8>, parms: &Dict, limit: usize) -> Result<Vec<u8>, Error> {
    let pred = parms.int("Predictor").unwrap_or(1);
    if pred < 2 {
        return Ok(data);
    }
    let colors = parms.int("Colors").unwrap_or(1).clamp(1, 32) as usize;
    let bpc = parms.int("BitsPerComponent").unwrap_or(8);
    let bpc = if matches!(bpc, 1 | 2 | 4 | 8 | 16) { bpc as usize } else { 8 };
    let columns = parms.int("Columns").unwrap_or(1).clamp(1, 1 << 24) as usize;
    let row = colors.saturating_mul(bpc).saturating_mul(columns).div_ceil(8);
    let bpp = (colors * bpc).div_ceil(8);
    if pred == 2 {
        return Ok(tiff(data, row, colors, bpc));
    }
    let mut out = Vec::with_capacity(data.len());
    // no row is longer than the data (a huge /Columns does not allocate)
    let mut prev = vec![0u8; row.min(data.len())];
    let mut cur = vec![0u8; row.min(data.len())];
    for chunk in data.chunks(row + 1) {
        let kind = chunk[0];
        let src = &chunk[1..];
        cur[..src.len()].copy_from_slice(src);
        for v in cur[src.len()..].iter_mut() {
            *v = 0;
        }
        for i in 0..src.len() {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = match kind {
                1 => cur[i].wrapping_add(a),
                2 => cur[i].wrapping_add(b),
                3 => cur[i].wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => {
                    let p = a as i16 + b as i16 - c as i16;
                    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
                    cur[i].wrapping_add(if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c })
                }
                _ => cur[i],
            };
        }
        out.extend_from_slice(&cur[..src.len()]);
        if out.len() > limit {
            return Err(Error::Limit("a stream is larger than max_stream"));
        }
        core::mem::swap(&mut prev, &mut cur);
    }
    Ok(out)
}

fn tiff(mut data: Vec<u8>, row: usize, colors: usize, bpc: usize) -> Vec<u8> {
    for line in data.chunks_mut(row) {
        match bpc {
            8 => {
                for i in colors..line.len() {
                    line[i] = line[i].wrapping_add(line[i - colors]);
                }
            }
            16 => {
                let step = colors * 2;
                let mut i = step;
                while i + 1 < line.len() {
                    let v = u16::from_be_bytes([line[i], line[i + 1]]).wrapping_add(u16::from_be_bytes([line[i - step], line[i - step + 1]]));
                    line[i..i + 2].copy_from_slice(&v.to_be_bytes());
                    i += 2;
                }
            }
            _ => {
                // 1, 2 or 4 bits: unpack, add, pack
                let per = 8 / bpc;
                let mask = (1u8 << bpc) - 1;
                let count = line.len() * per;
                let get = |l: &[u8], k: usize| (l[k / per] >> (8 - bpc * (k % per + 1))) & mask;
                for k in colors..count {
                    let v = get(line, k).wrapping_add(get(line, k - colors)) & mask;
                    let shift = 8 - bpc * (k % per + 1);
                    line[k / per] = line[k / per] & !(mask << shift) | v << shift;
                }
            }
        }
    }
    data
}
