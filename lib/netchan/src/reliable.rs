// reliable -- the reliable, ordered channel beside the packets (docs/TRANSPORT.md "Zuverlässige
// Nachrichten"), without I/O
//
//   Sender::push(message, chunk_max) -> Size / Backlog     a message cut into chunks
//   Sender::fill(now, rto, resend, room, bucket, out) -> ids   the chunks a RELIABLE packet carries
//   Sender::sent(seq, ids) / delivered(seq) / lost(seq)     the fate of the packets that carried them
//   Receiver::take(payload) -> Err(protocol) / Ok             the chunks of a packet that came
//   Receiver::message() -> Option<Vec<u8>>                  the next whole message, in order
//
// A message is cut into chunks of at most `chunk_max` bytes, each with a number of its own (u16,
// wrapping) and a flag "more chunks of this message follow".  A RELIABLE packet carries one or
// more chunk records:  id u16 | flags u8 (1: more) | len u16 | len bytes.  The packet is
// acknowledged as every packet (conn.rs); a chunk is done when a packet that carried it is
// Delivered.  Over UDP a chunk goes again when a packet that carried it is Lost, or when no ack
// came within the retransmission timeout (the round-trip time, four times its variation and the
// peer's ack delay, doubled for every retry up to a second); over a WebSocket never, since the transport loses
// nothing.  The receiver keeps the chunks that came early and hands out whole messages in the
// order they were sent.
//
// Limits: at most WINDOW chunks after the oldest open one are in flight (the receiver drops what
// lies further ahead, which a sender that keeps the window never sends); the sender's queue holds
// at most `max_queued` bytes (push: Backlog); a message has at most `max_message` bytes on both
// sides (the receiver ends the connection on more), the chunks kept before their turn at most
// EARLY_MAX bytes (more: the connection ends too); the bytes a second of reliable chunks come
// from a token bucket (`reliable_bytes_per_s` of the configs), so a large message does not run
// into the peer's rate limits.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;

use crate::conn::Bucket;
use crate::Error;

/// chunks in flight after the oldest open one
pub const WINDOW: u16 = 256;
/// the bytes of a chunk record before its data
pub const CHUNK_HEAD: usize = 5;
/// the bytes of chunks a receiver keeps before their turn (a sender within the window over UDP
/// has at most WINDOW * MAX_UDP_PAYLOAD in flight; over a WebSocket chunks come in order)
pub const EARLY_MAX: usize = 512 * 1024;
/// the retransmission timeout before the first round-trip time is known (µs)
const RTO_FIRST_US: u64 = 250_000;
/// the timeout's bounds (µs)
const RTO_MIN_US: u64 = 50_000;
const RTO_MAX_US: u64 = 1_000_000;
/// the least the variation counts for (µs): a link without jitter would else time out at the
/// round-trip time itself
const RTO_VAR_MIN_US: f64 = 10_000.0;

const FLAG_MORE: u8 = 1;

/// the limits of the reliable channel of a connection
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// the largest message, sent or taken
    pub max_message: usize,
    /// bytes this side keeps queued until the peer has them (push: Backlog beyond)
    pub max_queued: usize,
    /// bytes a second of reliable chunks this side sends
    pub bytes_per_s: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_message: 1 << 20, max_queued: 4 << 20, bytes_per_s: 128 * 1024 }
    }
}

/// the retransmission timeout from the smoothed round-trip time and its variation (µs): as
/// QUIC's probe timeout, the round trip, four times the variation (at least 10 ms) and the
/// longest the peer may hold an ack back (conn::ACK_DELAY_US)
pub fn rto_us(srtt_us: f64, rttvar_us: f64) -> u64 {
    if srtt_us <= 0.0 {
        return RTO_FIRST_US;
    }
    let t = srtt_us + (4.0 * rttvar_us).max(RTO_VAR_MIN_US) + crate::conn::ACK_DELAY_US as f64;
    (t as u64).clamp(RTO_MIN_US, RTO_MAX_US)
}

struct Chunk {
    id: u16,
    more: bool,
    data: Vec<u8>,
    /// when it last went out (None: not yet, or due again)
    sent_at: Option<u64>,
    retries: u32,
    /// a packet that carried it was Lost: due at once
    lost: bool,
    acked: bool,
}

/// what a connection sends reliably
pub struct Sender {
    limits: Limits,
    next_id: u16,
    queue: VecDeque<Chunk>,
    queued: usize,
    /// packet seq -> the chunks it carried
    flights: BTreeMap<u16, Vec<u16>>,
    bucket: Bucket,
    /// chunks sent again
    pub resent: u64,
}

impl Sender {
    pub fn new(limits: Limits, now: u64) -> Sender {
        Sender { limits, next_id: 0, queue: VecDeque::new(), queued: 0, flights: BTreeMap::new(), bucket: Bucket::new(limits.bytes_per_s, now), resent: 0 }
    }

    /// queue a message, cut into chunks of at most `chunk_max` bytes (Size: empty or over
    /// max_message; Backlog: more than max_queued would wait)
    pub fn push(&mut self, msg: &[u8], chunk_max: usize) -> Result<(), Error> {
        if msg.is_empty() || msg.len() > self.limits.max_message {
            return Err(Error::Size);
        }
        if self.queued.saturating_add(msg.len()) > self.limits.max_queued {
            return Err(Error::Backlog);
        }
        let chunk_max = chunk_max.clamp(1, u16::MAX as usize);
        let n = msg.len().div_ceil(chunk_max);
        for (i, piece) in msg.chunks(chunk_max).enumerate() {
            self.queue.push_back(Chunk { id: self.next_id, more: i + 1 < n, data: Vec::from(piece), sent_at: None, retries: 0, lost: false, acked: false });
            self.next_id = self.next_id.wrapping_add(1);
        }
        self.queued += msg.len();
        Ok(())
    }

    /// bytes queued that the peer does not have yet
    pub fn queued(&self) -> usize {
        self.queued
    }

    /// whether a chunk is waiting to go out (now or after a timeout)
    pub fn pending(&self) -> bool {
        !self.queue.is_empty()
    }

    /// append the records of the chunks that are due to `out`, at most `room` bytes of them ->
    /// their ids (empty: nothing due, or the bucket is empty).  `resend`: chunks whose packet
    /// went unacknowledged for `rto` µs go again (UDP; a WebSocket loses nothing)
    pub fn fill(&mut self, now: u64, rto: u64, resend: bool, room: usize, out: &mut Vec<u8>) -> Vec<u16> {
        let mut ids = Vec::new();
        let Some(base) = self.queue.front().map(|c| c.id) else { return ids };
        let mut used = 0;
        for c in self.queue.iter_mut() {
            if c.id.wrapping_sub(base) >= WINDOW {
                break;
            }
            if c.acked {
                continue;
            }
            let again = match c.sent_at {
                None => false,
                Some(t) if resend && (c.lost || now.saturating_sub(t) >= rto.saturating_mul(1 << c.retries.min(6)).min(RTO_MAX_US)) => true,
                Some(_) => continue,
            };
            let rec = CHUNK_HEAD + c.data.len();
            if used + rec > room || !self.bucket.take(rec as u64, now) {
                break;
            }
            out.extend_from_slice(&c.id.to_le_bytes());
            out.push(if c.more { FLAG_MORE } else { 0 });
            out.extend_from_slice(&(c.data.len() as u16).to_le_bytes());
            out.extend_from_slice(&c.data);
            used += rec;
            if again {
                c.retries += 1;
                self.resent += 1;
            }
            c.sent_at = Some(now);
            c.lost = false;
            ids.push(c.id);
        }
        ids
    }

    /// the packet `seq` carried these chunks
    pub fn sent(&mut self, seq: u16, ids: Vec<u16>) {
        self.flights.insert(seq, ids);
    }

    /// the peer has packet `seq`: its chunks are done
    pub fn delivered(&mut self, seq: u16) {
        let Some(ids) = self.flights.remove(&seq) else { return };
        let Some(base) = self.queue.front().map(|c| c.id) else { return };
        for id in ids {
            let at = id.wrapping_sub(base) as usize;
            if let Some(c) = self.queue.get_mut(at).filter(|c| c.id == id) {
                c.acked = true;
            }
        }
        while self.queue.front().is_some_and(|c| c.acked) {
            if let Some(c) = self.queue.pop_front() {
                self.queued -= c.data.len();
            }
        }
    }

    /// packet `seq` did not reach the peer: its chunks are due again (UDP)
    pub fn lost(&mut self, seq: u16) {
        let Some(ids) = self.flights.remove(&seq) else { return };
        let Some(base) = self.queue.front().map(|c| c.id) else { return };
        for id in ids {
            let at = id.wrapping_sub(base) as usize;
            if let Some(c) = self.queue.get_mut(at).filter(|c| c.id == id && !c.acked) {
                c.lost = true;
            }
        }
    }
}

/// what a connection takes in reliably
pub struct Receiver {
    max_message: usize,
    /// the id of the next chunk to hand out
    next: u16,
    /// chunks that came before their turn
    early: BTreeMap<u16, (bool, Vec<u8>)>,
    early_bytes: usize,
    /// the chunks of the message being put together
    partial: Vec<u8>,
    done: VecDeque<Vec<u8>>,
}

impl Receiver {
    pub fn new(max_message: usize) -> Receiver {
        Receiver { max_message, next: 0, early: BTreeMap::new(), early_bytes: 0, partial: Vec::new(), done: VecDeque::new() }
    }

    /// the chunk records of a RELIABLE packet that came (authentic, not a duplicate) -> Err when
    /// they are malformed or a message grows past max_message (the peer breaks the protocol)
    pub fn take(&mut self, mut b: &[u8]) -> Result<(), ()> {
        while !b.is_empty() {
            if b.len() < CHUNK_HEAD {
                return Err(());
            }
            let id = u16::from_le_bytes([b[0], b[1]]);
            let flags = b[2];
            let len = u16::from_le_bytes([b[3], b[4]]) as usize;
            if flags & !FLAG_MORE != 0 || b.len() < CHUNK_HEAD + len || len == 0 {
                return Err(());
            }
            let data = &b[CHUNK_HEAD..CHUNK_HEAD + len];
            b = &b[CHUNK_HEAD + len..];
            // behind (a copy of one handed out) or beyond the window: nothing to keep
            if id.wrapping_sub(self.next) >= WINDOW {
                continue;
            }
            if let alloc::collections::btree_map::Entry::Vacant(e) = self.early.entry(id) {
                self.early_bytes += len;
                if self.early_bytes > EARLY_MAX {
                    return Err(());
                }
                e.insert((flags & FLAG_MORE != 0, Vec::from(data)));
            }
        }
        while let Some((more, data)) = self.early.remove(&self.next) {
            self.early_bytes -= data.len();
            if self.partial.len() + data.len() > self.max_message {
                return Err(());
            }
            self.partial.extend_from_slice(&data);
            self.next = self.next.wrapping_add(1);
            if !more {
                self.done.push_back(core::mem::take(&mut self.partial));
            }
        }
        Ok(())
    }

    /// the next whole message, in the order they were sent
    pub fn message(&mut self) -> Option<Vec<u8>> {
        self.done.pop_front()
    }
}
