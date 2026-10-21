// pkt -- a packet buffer with headroom: headers are prepended in place
//
//   Pkt::new(headroom, payload)     the payload behind `headroom` free bytes
//   push(n) -> &mut [u8]            n more bytes at the front (the new header to fill in);
//                                   grows the headroom when it runs out (a copy, still correct)
//   pull(n) -> bool                 drops n bytes at the front (a header that was read)
//   bytes() / bytes_mut() / len()   the packet from the current front
//   extend(data) / truncate(len)    the tail
//   into_vec()                      the packet alone
//
// A segment is built once: payload first, then TCP, IP, VLAN and Ethernet headers in front of
// it, without moving the payload.

use alloc::vec::Vec;

/// room for Ethernet + 802.1Q + IPv4 with options + TCP with options (14 + 4 + 60 + 60)
pub const HEADROOM: usize = 144;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pkt {
    buf: Vec<u8>,
    start: usize,
}

impl Pkt {
    /// `payload` behind `headroom` free bytes
    pub fn new(headroom: usize, payload: &[u8]) -> Pkt {
        let mut buf = Vec::with_capacity(headroom + payload.len());
        buf.resize(headroom, 0);
        buf.extend_from_slice(payload);
        Pkt { buf, start: headroom }
    }
    /// an empty packet with the standard headroom
    pub fn empty() -> Pkt {
        Pkt::new(HEADROOM, &[])
    }
    /// a received frame (no headroom)
    pub fn from_vec(v: Vec<u8>) -> Pkt {
        Pkt { buf: v, start: 0 }
    }
    pub fn len(&self) -> usize {
        self.buf.len() - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn headroom(&self) -> usize {
        self.start
    }
    pub fn bytes(&self) -> &[u8] {
        &self.buf[self.start..]
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.start..]
    }
    /// `n` new bytes at the front, zeroed, for the caller to fill in
    pub fn push(&mut self, n: usize) -> &mut [u8] {
        if n > self.start {
            let grow = (n - self.start).max(HEADROOM);
            let mut nb = Vec::with_capacity(self.buf.len() + grow);
            nb.resize(grow, 0);
            nb.extend_from_slice(&self.buf);
            self.buf = nb;
            self.start += grow;
        }
        self.start -= n;
        let s = &mut self.buf[self.start..self.start + n];
        s.fill(0);
        s
    }
    /// drops `n` bytes at the front; false (and nothing dropped) when the packet is shorter
    pub fn pull(&mut self, n: usize) -> bool {
        if n > self.len() {
            return false;
        }
        self.start += n;
        true
    }
    pub fn extend(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }
    pub fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.buf.truncate(self.start + len);
        }
    }
    /// the packet bytes without the headroom
    pub fn into_vec(mut self) -> Vec<u8> {
        if self.start > 0 {
            self.buf.drain(..self.start);
        }
        self.buf
    }
}
