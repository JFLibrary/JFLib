// codec -- netio's frame codec without sockets: lengths at their boundaries, masks at every
// offset, bytes fed one at a time, control frames between fragments, the decoder's bounds

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netio::ws::{self, Decoder, Frame, Limits, Message, Role};

use crate::{Ctx, Rng};

/// every frame of `data` fed in pieces of `step` bytes -> the events, or the close code
pub fn decode_all(role: Role, lim: Limits, data: &[u8], step: usize) -> Result<Vec<Frame>, u16> {
    let mut d = Decoder::new(role, lim);
    let mut out = Vec::new();
    for piece in data.chunks(step.max(1)) {
        d.feed(piece)?;
        while let Some(f) = d.next()? {
            out.push(f);
        }
    }
    Ok(out)
}

pub fn run(quick: bool) -> Ctx {
    let mut c = Ctx::new("codec", quick);
    let mut rng = Rng::new(11);
    let lim = Limits { max_frame: 1 << 20, max_message: 4 << 20, ..Limits::default() };
    // the length forms at their boundaries, masked (client -> server) and not
    for &n in &[0usize, 1, 125, 126, 127, 65535, 65536, 70000] {
        let p = rng.bytes(n);
        for mask in [false, true] {
            let mut b = Vec::new();
            ws::encode_frame(&mut b, ws::OP_BINARY, true, &p, if mask { Some([9, 8, 7, 6]) } else { None });
            let head = 2 + if n < 126 { 0 } else if n <= 0xffff { 2 } else { 8 } + if mask { 4 } else { 0 };
            c.check(b.len() == head + n, || format!("frame of {} bytes: {} long", n, b.len()));
            let role = if mask { Role::Server } else { Role::Client };
            for step in [1usize, 7, 4096, usize::MAX] {
                if step == 1 && n > 1000 {
                    continue;
                }
                let r = decode_all(role, lim, &b, step);
                c.check(r == Ok(alloc::vec![Frame::Message(Message::Binary(p.clone()))]), || format!("{} bytes, mask {}, step {}: {:?}", n, mask, step, r.as_ref().map(|v| v.len())));
            }
            // the wrong side: a server takes no unmasked frame, a client no masked one
            let wrong = if mask { Role::Client } else { Role::Server };
            c.check(decode_all(wrong, lim, &b, usize::MAX) == Err(ws::CLOSE_PROTOCOL), || format!("{} bytes, mask {}: the wrong side refuses", n, mask));
        }
    }
    // apply_mask at every offset against the plain loop
    for off in 0..8 {
        for n in [0usize, 1, 3, 7, 8, 9, 31, 100] {
            let p = rng.bytes(n);
            let k = [0x12, 0x34, 0x56, 0x78];
            let mut a = p.clone();
            ws::apply_mask(&mut a, k, off);
            let b: Vec<u8> = p.iter().enumerate().map(|(i, x)| x ^ k[(i + off) & 3]).collect();
            c.check(a == b, || format!("apply_mask offset {} length {}", off, n));
        }
    }
    // control frames between the fragments of a message, a close at the end
    let mut b = Vec::new();
    let m = Some([1, 2, 3, 4]);
    ws::encode_frame(&mut b, ws::OP_TEXT, false, "Grü".as_bytes(), m);
    ws::encode_frame(&mut b, ws::OP_PING, true, b"1", m);
    ws::encode_frame(&mut b, ws::OP_CONT, false, "ße ".as_bytes(), m);
    ws::encode_frame(&mut b, ws::OP_PONG, true, b"2", m);
    ws::encode_frame(&mut b, ws::OP_CONT, true, "✓".as_bytes(), m);
    let mut cl = Vec::from(4000u16.to_be_bytes());
    cl.extend_from_slice("tschüss".as_bytes());
    ws::encode_frame(&mut b, ws::OP_CLOSE, true, &cl, m);
    ws::encode_frame(&mut b, ws::OP_TEXT, true, b"after the close", m);
    let want = alloc::vec![
        Frame::Ping(Vec::from(&b"1"[..])),
        Frame::Pong(Vec::from(&b"2"[..])),
        Frame::Message(Message::Text(String::from("Grüße ✓"))),
        Frame::Close(Some((4000, String::from("tschüss")))),
    ];
    for step in [1usize, 2, 5, 1000] {
        let r = decode_all(Role::Server, lim, &b, step);
        c.check(r.as_ref() == Ok(&want), || format!("controls between fragments, step {}: {:?}", step, r));
    }
    // a close without a code
    let mut b = Vec::new();
    ws::encode_frame(&mut b, ws::OP_CLOSE, true, &[], None);
    c.check(decode_all(Role::Client, lim, &b, 1) == Ok(alloc::vec![Frame::Close(None)]), || String::from("close without a code"));
    // every close code: valid ones pass, the others are a protocol error
    for code in 0u16..=5100 {
        let mut b = Vec::new();
        ws::encode_frame(&mut b, ws::OP_CLOSE, true, &code.to_be_bytes(), None);
        let r = decode_all(Role::Client, lim, &b, usize::MAX);
        let want = if ws::close_code_valid(code) { Ok(alloc::vec![Frame::Close(Some((code, String::new())))]) } else { Err(ws::CLOSE_PROTOCOL) };
        if r != want {
            c.check(false, || format!("close code {}: {:?}", code, r));
        }
    }
    c.check(ws::close_code_valid(1000) && ws::close_code_valid(4999) && !ws::close_code_valid(1005) && !ws::close_code_valid(1006) && !ws::close_code_valid(2999), || String::from("close_code_valid"));
    // the decoder's bounds: a feed may bring one frame and 64 KiB more, no further
    let small = Limits { max_frame: 1000, max_message: 3000, ..Limits::default() };
    let mut d = Decoder::new(Role::Server, small);
    let big = alloc::vec![0u8; 1000 + 14 + 64 * 1024 + 1];
    c.check(d.feed(&big) == Err(ws::CLOSE_TOO_BIG), || String::from("a feed past one frame and 64 KiB is refused"));
    let mut d = Decoder::new(Role::Server, small);
    let ok = d.feed(&big[..1000]).is_ok();
    c.check(ok && d.buffered() == 1000, || String::from("a feed within the bound"));
    c
}
