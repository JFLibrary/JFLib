// conn -- one connection's sequence numbers and acks, without I/O (docs/TRANSPORT.md "Folgenummern")
//
//   Conn::new(token, reliable, Settings, now_us)
//   conn.write_data(payload, now, &mut out) -> seq     a DATA packet (empty payload: keep-alive)
//   conn.on_data(seq, ack, bits, payload, now)         a DATA packet that came (token checked by
//                                                      the caller)
//   conn.take_event() -> Message / Delivered / Lost    conn.rate_ok(bytes, now), keepalive_due,
//                                                      timed_out, stats, rtt_ms
//
// Every DATA packet has its own sequence number (u16, wrapping) and acknowledges the newest
// number received plus the 32 before it as bits.  Unreliable (UDP): packets may come late, twice
// or not at all; a number seen before (1024 back) is dropped, older ones too; whatever arrives
// new is delivered at once, in the order it came.  A sent packet is Delivered when an ack names
// it, Lost when the acks have moved more than 32 past it without naming it.  Reliable (WebSocket):
// the transport keeps order and loses nothing, so an ack of n delivers everything up to n.
// A peer that only receives (or sends less than one packet per 32 received) would let packets
// pass the bits unacknowledged: after ACK_EVERY packets without one of its own, an unreliable
// connection owes an empty DATA packet at once (ack_due).
// The round-trip time comes from the newest ack only (bits name older packets, whose wait would
// count twice), smoothed as TCP does (RFC 6298).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use crate::packet::{seq_newer, Packet};

/// how far back sequence numbers are remembered (sent and received)
pub const WINDOW: usize = 1024;
/// (unreliable) packets received without sending one, after which an ack is due (half the bits)
pub const ACK_EVERY: u32 = 16;

/// the timing and limits of a connection
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// nothing received for this long ends the connection
    pub timeout_ms: u32,
    /// an empty DATA packet goes out when nothing was sent for this long (it carries the acks)
    pub keepalive_ms: u32,
    /// packets a second from the peer, on average (bursts of two seconds)
    pub packets_per_s: u32,
    /// bytes a second from the peer, on average (bursts of two seconds)
    pub bytes_per_s: u32,
}

/// what a connection counted
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// DATA packets sent (keep-alives included)
    pub sent: u64,
    /// DATA packets taken in (duplicates and stale ones not counted)
    pub received: u64,
    /// sent packets an ack named
    pub delivered: u64,
    /// sent packets the acks passed by
    pub lost: u64,
    /// packets that came a second time
    pub duplicates: u64,
    /// packets older than the window
    pub stale: u64,
    /// packets dropped for the rate limits or their size
    pub dropped: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    /// the smoothed round-trip time (0 before the first sample)
    pub rtt_ms: f32,
    /// its variation
    pub rtt_var_ms: f32,
}

/// what a connection reports
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// a message of the peer with its sequence number
    Message { seq: u16, data: Vec<u8> },
    /// the peer has a packet this side sent
    Delivered(u16),
    /// the peer's acks passed this packet by
    Lost(u16),
}

/// a token bucket: `rate` a second, two seconds' worth saved up at most
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bucket {
    rate: u64,
    level: u64,
    last_us: u64,
}

impl Bucket {
    pub(crate) fn new(rate: u32, now: u64) -> Bucket {
        let rate = rate.max(1) as u64;
        Bucket { rate, level: rate * 2_000_000, last_us: now }
    }

    pub(crate) fn take(&mut self, n: u64, now: u64) -> bool {
        let dt = now.saturating_sub(self.last_us);
        self.last_us = now.max(self.last_us);
        self.level = self.level.saturating_add(dt.saturating_mul(self.rate)).min(self.rate * 2_000_000);
        let need = n.saturating_mul(1_000_000);
        if self.level < need {
            return false;
        }
        self.level -= need;
        true
    }
}

const EMPTY: u8 = 0;
const WAITING: u8 = 1;
const DONE: u8 = 2;

/// the state of one connection (see the top of this file)
pub struct Conn {
    token: u32,
    reliable: bool,
    set: Settings,
    next_seq: u16,
    sent_seq: Vec<u16>,
    sent_time: Vec<u64>,
    sent_state: Vec<u8>,
    // the oldest sent number whose fate is open
    oldest: u16,
    recv_seq: Vec<u16>,
    recv_have: Vec<bool>,
    latest: u16,
    any: bool,
    last_send_us: u64,
    last_recv_us: u64,
    // packets received since this side last sent one
    unacked_rx: u32,
    srtt_us: f64,
    rttvar_us: f64,
    stats: Stats,
    packets: Bucket,
    bytes: Bucket,
    events: VecDeque<Event>,
}

impl Conn {
    /// a connection with the token both sides put into DATA (0 over a WebSocket)
    pub fn new(token: u32, reliable: bool, set: Settings, now_us: u64) -> Conn {
        Conn {
            token,
            reliable,
            set,
            next_seq: 0,
            sent_seq: vec![0; WINDOW],
            sent_time: vec![0; WINDOW],
            sent_state: vec![EMPTY; WINDOW],
            oldest: 0,
            recv_seq: vec![0; WINDOW],
            recv_have: vec![false; WINDOW],
            latest: 0,
            any: false,
            last_send_us: now_us,
            last_recv_us: now_us,
            unacked_rx: 0,
            srtt_us: 0.0,
            rttvar_us: 0.0,
            stats: Stats::default(),
            packets: Bucket::new(set.packets_per_s, now_us),
            bytes: Bucket::new(set.bytes_per_s, now_us),
            events: VecDeque::new(),
        }
    }

    /// the token of DATA and DISCONNECT packets
    pub fn token(&self) -> u32 {
        self.token
    }

    /// whether the transport is reliable (a WebSocket)
    pub fn reliable(&self) -> bool {
        self.reliable
    }

    /// the newest number received and the bits of the 32 before it (0xffff, 0 before any)
    pub fn ack_fields(&self) -> (u16, u32) {
        if !self.any {
            return (0xffff, 0);
        }
        let mut bits = 0u32;
        for i in 0..32u16 {
            let s = self.latest.wrapping_sub(1 + i);
            let k = s as usize % WINDOW;
            if self.recv_have[k] && self.recv_seq[k] == s {
                bits |= 1 << i;
            }
        }
        (self.latest, bits)
    }

    /// append a DATA packet with `payload` (empty: a keep-alive) -> its sequence number
    pub fn write_data(&mut self, payload: &[u8], now: u64, out: &mut Vec<u8>) -> u16 {
        let seq = self.next_seq;
        let k = seq as usize % WINDOW;
        if self.sent_state[k] == WAITING {
            // a whole window without an answer: decided now
            let old = self.sent_seq[k];
            self.sent_state[k] = DONE;
            if self.reliable {
                self.stats.delivered += 1;
                self.events.push_back(Event::Delivered(old));
            } else {
                self.stats.lost += 1;
                self.events.push_back(Event::Lost(old));
            }
        }
        self.sent_seq[k] = seq;
        self.sent_time[k] = now;
        self.sent_state[k] = WAITING;
        self.next_seq = seq.wrapping_add(1);
        self.advance_oldest();
        let (ack, bits) = self.ack_fields();
        let before = out.len();
        Packet::Data { token: self.token, seq, ack, bits, payload }.write(out, false);
        self.last_send_us = now;
        self.unacked_rx = 0;
        self.stats.sent += 1;
        self.stats.bytes_sent += (out.len() - before) as u64;
        seq
    }

    /// whether a packet of `bytes` from the peer stays within the rate limits (false: drop it,
    /// or end a reliable connection)
    pub fn rate_ok(&mut self, bytes: usize, now: u64) -> bool {
        let ok = self.packets.take(1, now) && self.bytes.take(bytes as u64, now);
        if !ok {
            self.stats.dropped += 1;
        }
        ok
    }

    /// count a packet dropped by the caller (its size)
    pub fn count_dropped(&mut self) {
        self.stats.dropped += 1;
    }

    /// a DATA packet of the peer (its token already checked)
    pub fn on_data(&mut self, seq: u16, ack: u16, bits: u32, payload: &[u8], now: u64) {
        let k = seq as usize % WINDOW;
        if self.any {
            if seq_newer(seq, self.latest) {
                let gap = seq.wrapping_sub(self.latest) as usize;
                if gap >= WINDOW {
                    self.recv_have.iter_mut().for_each(|h| *h = false);
                } else {
                    for i in 1..gap {
                        self.recv_have[self.latest.wrapping_add(i as u16) as usize % WINDOW] = false;
                    }
                }
                self.latest = seq;
            } else {
                let age = self.latest.wrapping_sub(seq) as usize;
                if age >= WINDOW {
                    self.stats.stale += 1;
                    return;
                }
                if self.recv_have[k] && self.recv_seq[k] == seq {
                    self.stats.duplicates += 1;
                    return;
                }
            }
        } else {
            self.any = true;
            self.latest = seq;
        }
        self.recv_have[k] = true;
        self.recv_seq[k] = seq;
        self.last_recv_us = now;
        self.unacked_rx = self.unacked_rx.saturating_add(1);
        self.stats.received += 1;
        self.stats.bytes_received += (crate::packet::DATA_HEADER + payload.len()) as u64;
        self.on_ack(ack, bits, now);
        if !payload.is_empty() {
            self.events.push_back(Event::Message { seq, data: payload.to_vec() });
        }
    }

    // the entry of a number this side sent and is still remembered
    fn mine(&self, s: u16) -> Option<usize> {
        let k = s as usize % WINDOW;
        (self.sent_state[k] != EMPTY && self.sent_seq[k] == s && !seq_newer(s, self.next_seq.wrapping_sub(1))).then_some(k)
    }

    fn deliver(&mut self, k: usize) {
        if self.sent_state[k] == WAITING {
            self.sent_state[k] = DONE;
            self.stats.delivered += 1;
            self.events.push_back(Event::Delivered(self.sent_seq[k]));
        }
    }

    fn on_ack(&mut self, ack: u16, bits: u32, now: u64) {
        // an ack of a number this side never sent (or forgot) says nothing
        let Some(k) = self.mine(ack) else { return };
        if self.sent_state[k] == WAITING {
            let sample = now.saturating_sub(self.sent_time[k]) as f64;
            if self.srtt_us == 0.0 {
                self.srtt_us = sample.max(1.0);
                self.rttvar_us = sample / 2.0;
            } else {
                let d = if self.srtt_us > sample { self.srtt_us - sample } else { sample - self.srtt_us };
                self.rttvar_us = 0.75 * self.rttvar_us + 0.25 * d;
                self.srtt_us = 0.875 * self.srtt_us + 0.125 * sample;
            }
            self.stats.rtt_ms = (self.srtt_us / 1000.0) as f32;
            self.stats.rtt_var_ms = (self.rttvar_us / 1000.0) as f32;
        }
        self.deliver(k);
        for i in 0..32u16 {
            if bits & (1 << i) != 0 {
                if let Some(k) = self.mine(ack.wrapping_sub(1 + i)) {
                    self.deliver(k);
                }
            }
        }
        // the open ones behind the ack: reliable delivered, else lost once the bits passed them
        let edge = if self.reliable { ack.wrapping_add(1) } else { ack.wrapping_sub(32) };
        while self.oldest != self.next_seq && seq_newer(edge, self.oldest) {
            let k = self.oldest as usize % WINDOW;
            if self.sent_state[k] == WAITING && self.sent_seq[k] == self.oldest {
                self.sent_state[k] = DONE;
                if self.reliable {
                    self.stats.delivered += 1;
                    self.events.push_back(Event::Delivered(self.oldest));
                } else {
                    self.stats.lost += 1;
                    self.events.push_back(Event::Lost(self.oldest));
                }
            }
            self.oldest = self.oldest.wrapping_add(1);
        }
        self.advance_oldest();
    }

    // past the numbers already decided
    fn advance_oldest(&mut self) {
        while self.oldest != self.next_seq {
            let k = self.oldest as usize % WINDOW;
            if self.sent_state[k] == WAITING && self.sent_seq[k] == self.oldest {
                break;
            }
            self.oldest = self.oldest.wrapping_add(1);
        }
    }

    /// the next event (messages, deliveries, losses in the order they happened)
    pub fn take_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// (unreliable) whether an empty DATA packet is due at once: ACK_EVERY packets came without
    /// one of this side's, so the older ones would pass the bits unacknowledged
    pub fn ack_due(&self) -> bool {
        !self.reliable && self.unacked_rx >= ACK_EVERY
    }

    /// whether a keep-alive is due (nothing sent for keepalive_ms)
    pub fn keepalive_due(&self, now: u64) -> bool {
        now.saturating_sub(self.last_send_us) >= self.set.keepalive_ms as u64 * 1000
    }

    /// whether nothing came for timeout_ms
    pub fn timed_out(&self, now: u64) -> bool {
        now.saturating_sub(self.last_recv_us) >= self.set.timeout_ms as u64 * 1000
    }

    /// what the connection counted
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// the smoothed round-trip time in ms (0 before the first ack)
    pub fn rtt_ms(&self) -> f32 {
        self.stats.rtt_ms
    }

    /// the sequence number the next packet gets
    pub fn next_seq(&self) -> u16 {
        self.next_seq
    }
}
