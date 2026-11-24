// conn -- one connection's sequence numbers and acks, without I/O (docs/TRANSPORT.md "Folgenummern")
//
//   Conn::new(token, reliable, Settings, now_us)
//   conn.write_data(payload, now, &mut out) -> seq     a DATA packet (empty payload: keep-alive)
//   conn.on_data(seq, ack, bits, payload, now)         a DATA packet that came (token checked by
//                                                      the caller)
//   conn.take_event() -> Message / Delivered / Lost    conn.rate_ok(bytes, now), keepalive_due,
//                                                      timed_out, stats, rtt_ms
//   Conn::new(..).with_keys(Keys)                      sealed (UDP): write_data and
//                                                      write_disconnect seal, open(wire, head,
//                                                      seq) checks and decrypts what came
//   conn.send_reliable(message, chunk_max)             the reliable channel (reliable.rs):
//   conn.write_reliable(now, room, &mut out) -> seq    RELIABLE packets of the chunks due,
//   conn.on_reliable(seq, ack, bits, payload, now)     what came (Event::Reliable: whole
//                                                      messages in order); with_reliable(Limits)
//
// Every DATA packet has its own sequence number (u16, wrapping) and acknowledges the newest
// number received plus the 32 before it as bits.  Unreliable (UDP): packets may come late, twice
// or not at all; a number seen before (1024 back) is dropped, older ones too; whatever arrives
// new is delivered at once, in the order it came.  A sent packet is Delivered when an ack names
// it, Lost when the acks have moved more than 32 past it without naming it.  Reliable (WebSocket):
// the transport keeps order and loses nothing, so an ack of n delivers everything up to n.
// A peer that only receives (or sends less than one packet per 32 received) would let packets
// pass the bits unacknowledged: after ACK_EVERY packets without one of its own, an unreliable
// connection owes an empty DATA packet at once (ack_due).  RELIABLE packets are acknowledged
// sooner, after two or within ACK_DELAY_US: their sender times its resends by the acks.
// The round-trip time comes from the newest ack only (bits name older packets, whose wait would
// count twice), smoothed as TCP does (RFC 6298).
// Sealed (seal.rs): each packet sent takes the next packet number (u64; its low 16 bits are the
// sequence number), the nonce of ChaCha20-Poly1305; open() takes the number nearest the largest
// authenticated one and leaves the state alone when the tag does not check (Stats::forged).  The
// receive window above then drops a replayed packet as a duplicate or stale: only after the tag.
// RELIABLE packets take numbers and acks from the same sequence as DATA; their Delivered and
// Lost come as events like those of keep-alives, and tell the reliable sender which chunks the
// peer has.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use crate::packet::{seq_newer, Packet, DATA_HEADER, DISCONNECT_HEADER, TAG};
use crate::reliable::{self, Limits, Receiver, Sender};
use crate::seal::{self, Keys};
use crate::Error;

/// how far back sequence numbers are remembered (sent and received)
pub const WINDOW: usize = 1024;
/// (unreliable) packets received without sending one, after which an ack is due (half the bits)
pub const ACK_EVERY: u32 = 16;
/// (unreliable) RELIABLE packets received without sending one, after which an ack is due: the
/// peer waits for it to free its window and to send nothing twice (as TCP's delayed ack)
pub const ACK_EVERY_RELIABLE: u32 = 2;
/// (unreliable) the longest a RELIABLE packet waits for its ack (keepalive_due then holds)
pub const ACK_DELAY_US: u64 = 10_000;

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
    /// sealed packets with this connection's token whose tag did not check (forged or damaged;
    /// nothing else was done with them)
    pub forged: u64,
    /// bytes of DATA packets sent (headers included)
    pub bytes_sent: u64,
    /// bytes of DATA packets taken in (headers included)
    pub bytes_received: u64,
    /// chunks of the reliable channel sent again (UDP: lost, or no ack within the timeout)
    pub resent: u64,
    /// bytes of reliable messages the peer does not have yet
    pub reliable_queued: u64,
    /// the smoothed round-trip time (0 before the first sample)
    pub rtt_ms: f32,
    /// its variation
    pub rtt_var_ms: f32,
}

/// what a connection reports
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// a message of the peer with its sequence number
    Message { seq: u16, data: Vec<u8> },
    /// a message of the peer's reliable channel, whole and in order
    Reliable(Vec<u8>),
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
    keys: Option<Keys>,
    // the packet number the next sealed packet takes (its low bits: next_seq)
    sent_pn: u64,
    // the largest packet number authenticated
    rx_pn: Option<u64>,
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
    // RELIABLE packets among them, and when the first came
    unacked_rel: u32,
    rel_since_us: u64,
    srtt_us: f64,
    rttvar_us: f64,
    stats: Stats,
    packets: Bucket,
    bytes: Bucket,
    events: VecDeque<Event>,
    rel_tx: Sender,
    rel_rx: Receiver,
}

impl Conn {
    /// a connection with the token both sides put into DATA (0 over a WebSocket)
    pub fn new(token: u32, reliable: bool, set: Settings, now_us: u64) -> Conn {
        Conn {
            token,
            reliable,
            set,
            keys: None,
            sent_pn: 0,
            rx_pn: None,
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
            unacked_rel: 0,
            rel_since_us: 0,
            srtt_us: 0.0,
            rttvar_us: 0.0,
            stats: Stats::default(),
            packets: Bucket::new(set.packets_per_s, now_us),
            bytes: Bucket::new(set.bytes_per_s, now_us),
            events: VecDeque::new(),
            rel_tx: Sender::new(Limits::default(), now_us),
            rel_rx: Receiver::new(Limits::default().max_message),
        }
    }

    /// the reliable channel with these limits (instead of Limits::default())
    pub fn with_reliable(mut self, limits: Limits) -> Conn {
        self.rel_tx = Sender::new(limits, self.last_send_us);
        self.rel_rx = Receiver::new(limits.max_message);
        self
    }

    /// the connection sealed with these keys (UDP after the handshake): every packet written is
    /// encrypted and authenticated, every packet taken in must pass open()
    pub fn with_keys(mut self, keys: Keys) -> Conn {
        self.keys = Some(keys);
        self
    }

    /// whether its packets are sealed
    pub fn sealed(&self) -> bool {
        self.keys.is_some()
    }

    /// the token of DATA and DISCONNECT packets
    pub fn token(&self) -> u32 {
        self.token
    }

    // the next packet number and its sequence number
    fn take_pn(&mut self) -> (u64, u16) {
        let pn = self.sent_pn;
        self.sent_pn = pn.wrapping_add(1);
        let seq = self.next_seq;
        self.next_seq = seq.wrapping_add(1);
        (pn, seq)
    }

    // seal out[start..]: the first `head` bytes authenticated, the rest encrypted, the tag after it
    fn seal_tail(&self, pn: u64, out: &mut Vec<u8>, start: usize, head: usize) {
        if let Some(k) = &self.keys {
            let (aad, body) = out[start..].split_at_mut(head);
            let tag = crypto::aead::seal_in_place(&k.send, &seal::nonce(pn), aad, body);
            out.extend_from_slice(&tag);
        }
    }

    /// check and decrypt a packet that came: `wire` the whole packet, `head` the bytes before its
    /// payload (DATA_HEADER, DISCONNECT_HEADER), `seq` its sequence number -> the payload
    /// (unsealed connections: the bytes after the header as they are).  None: too short or a
    /// wrong tag (counted as forged); the connection's state is not touched then
    pub fn open(&mut self, wire: &[u8], head: usize, seq: u16) -> Option<Vec<u8>> {
        let Some(k) = &self.keys else { return wire.get(head..).map(|b| b.to_vec()) };
        if wire.len() < head + TAG {
            self.stats.forged += 1;
            return None;
        }
        let pn = seal::full_pn(self.rx_pn, seq);
        let (body, tag) = wire[head..].split_at(wire.len() - head - TAG);
        let mut t = [0u8; TAG];
        t.copy_from_slice(tag);
        let mut plain = body.to_vec();
        if !crypto::aead::open_in_place(&k.recv, &seal::nonce(pn), &wire[..head], &mut plain, &t) {
            self.stats.forged += 1;
            return None;
        }
        // authentic: a newer number moves the reference of the next reconstruction
        self.rx_pn = Some(self.rx_pn.map_or(pn, |l| l.max(pn)));
        Some(plain)
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

    /// append a DATA packet with `payload` (empty: a keep-alive), sealed if the connection is
    /// -> its sequence number
    pub fn write_data(&mut self, payload: &[u8], now: u64, out: &mut Vec<u8>) -> u16 {
        self.write_packet(false, payload, now, out)
    }

    // the fate of a packet this side sent: the event, and the reliable chunks it carried
    fn decide(&mut self, seq: u16, delivered: bool) {
        if delivered {
            self.stats.delivered += 1;
            self.events.push_back(Event::Delivered(seq));
            self.rel_tx.delivered(seq);
        } else {
            self.stats.lost += 1;
            self.events.push_back(Event::Lost(seq));
            self.rel_tx.lost(seq);
        }
    }

    // a DATA or RELIABLE packet with the next number and the acks
    fn write_packet(&mut self, reliable: bool, payload: &[u8], now: u64, out: &mut Vec<u8>) -> u16 {
        let (pn, seq) = self.take_pn();
        let k = seq as usize % WINDOW;
        if self.sent_state[k] == WAITING {
            // a whole window without an answer: decided now
            let old = self.sent_seq[k];
            self.sent_state[k] = DONE;
            self.decide(old, self.reliable);
        }
        self.sent_seq[k] = seq;
        self.sent_time[k] = now;
        self.sent_state[k] = WAITING;
        self.advance_oldest();
        let (ack, bits) = self.ack_fields();
        let before = out.len();
        let token = self.token;
        let p = if reliable { Packet::Reliable { token, seq, ack, bits, payload } } else { Packet::Data { token, seq, ack, bits, payload } };
        p.write(out, false);
        self.seal_tail(pn, out, before, DATA_HEADER);
        self.last_send_us = now;
        self.unacked_rx = 0;
        self.unacked_rel = 0;
        self.stats.sent += 1;
        self.stats.bytes_sent += (out.len() - before) as u64;
        seq
    }

    /// append a DISCONNECT with `reason` (BYE_*), sealed if the connection is; it takes a
    /// sequence number of its own, so its seal never shares a nonce with a DATA packet
    pub fn write_disconnect(&mut self, reason: u8, out: &mut Vec<u8>) {
        let (pn, seq) = self.take_pn();
        let before = out.len();
        Packet::Disconnect { token: self.token, seq, reason, tag: &[] }.write(out, false);
        self.seal_tail(pn, out, before, DISCONNECT_HEADER);
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

    /// queue a message on the reliable channel, cut into chunks of at most `chunk_max` bytes
    /// (the room of a packet less reliable::CHUNK_HEAD); Size: empty or over Limits::max_message,
    /// Backlog: more than Limits::max_queued would wait
    pub fn send_reliable(&mut self, msg: &[u8], chunk_max: usize) -> Result<(), Error> {
        self.rel_tx.push(msg, chunk_max)
    }

    /// append a RELIABLE packet with the chunks that are due (new ones, and over UDP those whose
    /// packet was lost or went unacknowledged past the timeout), at most `room` bytes of
    /// payload -> its sequence number; None: nothing due now (or the rate of the channel spent)
    pub fn write_reliable(&mut self, now: u64, room: usize, out: &mut Vec<u8>) -> Option<u16> {
        let rto = reliable::rto_us(self.srtt_us, self.rttvar_us);
        let mut payload = Vec::new();
        let ids = self.rel_tx.fill(now, rto, !self.reliable, room, &mut payload);
        if ids.is_empty() {
            return None;
        }
        let seq = self.write_packet(true, &payload, now, out);
        self.rel_tx.sent(seq, ids);
        Some(seq)
    }

    /// whether the reliable channel has chunks the peer does not have yet
    pub fn reliable_pending(&self) -> bool {
        self.rel_tx.pending()
    }

    /// a DATA packet of the peer (its token already checked)
    pub fn on_data(&mut self, seq: u16, ack: u16, bits: u32, payload: &[u8], now: u64) {
        if self.take_in(seq, ack, bits, payload.len(), now) && !payload.is_empty() {
            self.events.push_back(Event::Message { seq, data: payload.to_vec() });
        }
    }

    /// a RELIABLE packet of the peer (its token checked, opened) -> false when its chunks break
    /// the protocol (malformed, a message over max_message): the caller ends the connection
    pub fn on_reliable(&mut self, seq: u16, ack: u16, bits: u32, payload: &[u8], now: u64) -> bool {
        if !self.take_in(seq, ack, bits, payload.len(), now) {
            return true;
        }
        if self.unacked_rel == 0 {
            self.rel_since_us = now;
        }
        self.unacked_rel += 1;
        if self.rel_rx.take(payload).is_err() {
            return false;
        }
        while let Some(m) = self.rel_rx.message() {
            self.events.push_back(Event::Reliable(m));
        }
        true
    }

    // a packet of the peer into the receive window and its acks -> whether it was new
    fn take_in(&mut self, seq: u16, ack: u16, bits: u32, payload_len: usize, now: u64) -> bool {
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
                    return false;
                }
                if self.recv_have[k] && self.recv_seq[k] == seq {
                    self.stats.duplicates += 1;
                    return false;
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
        self.stats.bytes_received += (DATA_HEADER + payload_len + if self.keys.is_some() { TAG } else { 0 }) as u64;
        self.on_ack(ack, bits, now);
        true
    }

    // the entry of a number this side sent and is still remembered
    fn mine(&self, s: u16) -> Option<usize> {
        let k = s as usize % WINDOW;
        (self.sent_state[k] != EMPTY && self.sent_seq[k] == s && !seq_newer(s, self.next_seq.wrapping_sub(1))).then_some(k)
    }

    fn deliver(&mut self, k: usize) {
        if self.sent_state[k] == WAITING {
            self.sent_state[k] = DONE;
            self.decide(self.sent_seq[k], true);
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
                self.decide(self.oldest, self.reliable);
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
    /// one of this side's, so the older ones would pass the bits unacknowledged (or
    /// ACK_EVERY_RELIABLE RELIABLE packets, whose sender waits for the acks)
    pub fn ack_due(&self) -> bool {
        !self.reliable && (self.unacked_rx >= ACK_EVERY || self.unacked_rel >= ACK_EVERY_RELIABLE)
    }

    /// whether a keep-alive is due (nothing sent for keepalive_ms; unreliable: or a RELIABLE
    /// packet waited ACK_DELAY_US for its ack)
    pub fn keepalive_due(&self, now: u64) -> bool {
        now.saturating_sub(self.last_send_us) >= self.set.keepalive_ms as u64 * 1000
            || (!self.reliable && self.unacked_rel > 0 && now.saturating_sub(self.rel_since_us) >= ACK_DELAY_US)
    }

    /// whether nothing came for timeout_ms
    pub fn timed_out(&self, now: u64) -> bool {
        now.saturating_sub(self.last_recv_us) >= self.set.timeout_ms as u64 * 1000
    }

    /// what the connection counted
    pub fn stats(&self) -> Stats {
        Stats { resent: self.rel_tx.resent, reliable_queued: self.rel_tx.queued() as u64, ..self.stats }
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
