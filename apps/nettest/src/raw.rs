// raw -- a WebSocket client by hand over a TcpStream: any request bytes, frames as bytes (masked or
// not, malformed on purpose), the server's frames read back one by one

use alloc::string::String;
use alloc::vec::Vec;

use netio::http::{Headers, Reader};
use netio::ws;
use plat::net::{SocketAddr, TcpStream};

/// a connection after the upgrade request
pub struct Raw {
    pub s: TcpStream,
    buf: Vec<u8>,
}

/// a valid upgrade request for `path` with these extra header lines ("Name: value\r\n" each)
pub fn request(path: &str, extra: &str) -> String {
    alloc::format!(
        "GET {} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{}\r\n",
        path, extra
    )
}

/// send `req` as it is, read the answer's head -> (status, headers, the connection)
pub fn open(addr: &SocketAddr, req: &str) -> Option<(u16, Headers, Raw)> {
    let s = TcpStream::connect(addr, 3000).ok()?;
    s.set_read_timeout(Some(3000));
    s.set_write_timeout(Some(3000));
    s.write_all(req.as_bytes()).ok()?;
    let (status, h, rest) = {
        let mut rd = Reader::new(&s);
        let (start, h) = rd.head().ok()?;
        let status = start.split(' ').nth(1)?.parse::<u16>().ok()?;
        (status, h, rd.take_buffered())
    };
    Some((status, h, Raw { s, buf: rest }))
}

/// a connection upgraded with the plain request (None: no 101)
pub fn upgrade(addr: &SocketAddr, path: &str) -> Option<Raw> {
    match open(addr, &request(path, "")) {
        Some((101, _, r)) => Some(r),
        _ => None,
    }
}

impl Raw {
    /// write bytes as they are
    pub fn send(&self, b: &[u8]) -> bool {
        self.s.write_all(b).is_ok()
    }

    /// one frame: b0 = FIN | RSV | opcode as given, the length in its shortest form, masked with a
    /// fixed key when `mask`
    pub fn frame(&self, b0: u8, payload: &[u8], mask: bool) -> bool {
        let mut out = Vec::new();
        ws::encode_frame(&mut out, b0 & 0x0f, b0 & 0x80 != 0, payload, if mask { Some([0x37, 0xfa, 0x21, 0x3d]) } else { None });
        out[0] = b0;
        self.send(&out)
    }

    /// the next frame of the server within `ms` -> (b0, payload); None when it ended or nothing came
    pub fn read_frame(&mut self, ms: u32) -> Option<(u8, Vec<u8>)> {
        let end = plat::time_us() + ms as u64 * 1000;
        loop {
            if self.buf.len() >= 2 {
                let len7 = (self.buf[1] & 0x7f) as usize;
                let (ext, len) = match len7 {
                    126 if self.buf.len() >= 4 => (2, u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize),
                    127 if self.buf.len() >= 10 => {
                        let mut a = [0u8; 8];
                        a.copy_from_slice(&self.buf[2..10]);
                        (8, u64::from_be_bytes(a) as usize)
                    }
                    126 | 127 => (usize::MAX, 0),
                    n => (0, n),
                };
                if ext != usize::MAX && self.buf.len() >= 2 + ext + len {
                    let b0 = self.buf[0];
                    let p = self.buf[2 + ext..2 + ext + len].to_vec();
                    self.buf.drain(..2 + ext + len);
                    return Some((b0, p));
                }
            }
            let now = plat::time_us();
            if now >= end {
                return None;
            }
            if !self.s.wait_readable(((end - now) / 1000).max(1) as u32).unwrap_or(false) {
                continue;
            }
            let mut b = [0u8; 16384];
            match self.s.read(&mut b) {
                Ok(n) if n > 0 => self.buf.extend_from_slice(&b[..n]),
                _ => return None,
            }
        }
    }

    /// read frames until the server's close -> its code (None: the connection ended without one,
    /// or nothing came within `ms`)
    pub fn close_code(&mut self, ms: u32) -> Option<u16> {
        let end = plat::time_us() + ms as u64 * 1000;
        loop {
            let left = (end.saturating_sub(plat::time_us()) / 1000) as u32;
            if left == 0 {
                return None;
            }
            let (b0, p) = self.read_frame(left)?;
            if b0 & 0x0f == ws::OP_CLOSE {
                return Some(if p.len() >= 2 { u16::from_be_bytes([p[0], p[1]]) } else { ws::CLOSE_NO_STATUS });
            }
        }
    }
}
