// event -- reliable events over the netcode's own packets: each delivered once and in order, over UDP and over a
// WebSocket (docs/NETCODE.md, "Ereignisse")
//
//   NetEvent   a tick, a channel (0 ..= 15: the game's kinds, such as the kill feed, hit confirmations, chat) and
//              up to MAX_EVENT bytes
//   Outbox     the sender's side: every event gets the next number of one sequence per direction (u16, wrapping)
//              and stays until the peer acknowledges it; every packet carries the oldest ones up to a byte budget.
//              Over a WebSocket (reliable and ordered) each goes once and is forgotten
//   Inbox      the receiver's side: the number it expects next.  A packet's run of events starts at or before it
//              (the sender forgets only what was acknowledged): the ones before are duplicates, the rest are
//              delivered in order; the acknowledgement rides in the next packets of the other direction
//   write_section / read_section   a run of events in a bit stream (after a message's header, proto.rs)
//
// Server -> client the events ride in SNAPSHOTs, client -> server in INPUTs, and each side acknowledges in its own
// packets what it got (proto.rs: the event part after the header, flagged in byte 0).  This is the rule of Quake 3's
// reliable commands: whatever the network loses, duplicates or reorders, a packet that arrives holds a run without a
// gap from what the receiver expects, so nothing waits for anything (no reordering buffer, no timer, no extra
// packet).  Delivered in one order per direction, so in order per channel as well.  The price: an event rides in
// every packet until its acknowledgement comes back -- at 30 snapshots a second and a round trip of 100 ms four or
// five times; a kill-feed line of a few bytes costs little.  Why here and not in netchan: see NETCODE.md.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use super::bits::{svar_bits, uvar_bits, BitReader, BitWriter};
use super::snapshot::tick_diff;

/// the bytes of one event at most
pub const MAX_EVENT: usize = 255;
/// channels (0 ..= 15)
pub const CHANNELS: u8 = 16;
/// events an outbox holds at most (more: the peer does not acknowledge; a server drops such a client)
pub const MAX_PENDING: usize = 512;
/// events one packet's run holds at most
pub const MAX_RUN: usize = 256;
/// packets of the other direction that carry the acknowledgement after events came (a duplicate starts it again)
pub const ACK_PACKETS: u32 = 32;

/// one event (see the module header)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetEvent {
    /// the sender's tick when it happened (the server's tick; a client's predicted tick)
    pub tick: u32,
    /// the channel: the game's kind of event (0 ..= 15)
    pub channel: u8,
    /// the game's bytes (at most MAX_EVENT)
    pub data: Vec<u8>,
}

impl NetEvent {
    /// an event of `channel` at `tick` with these bytes
    pub fn new(tick: u32, channel: u8, data: &[u8]) -> NetEvent {
        NetEvent { tick, channel, data: data.to_vec() }
    }

    /// within the limits (channel, size)
    pub fn valid(&self) -> bool {
        self.channel < CHANNELS && self.data.len() <= MAX_EVENT
    }

    /// its bits in a run whose ticks count from `base_tick`
    pub fn bits(&self, base_tick: u32) -> usize {
        4 + svar_bits(tick_diff(self.tick, base_tick)) + uvar_bits(self.data.len() as u32) + 8 * self.data.len()
    }
}

/// a - b of two event numbers (they wrap)
pub fn seq_diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

/// write a run: the number of its first event, the count, then each event (channel, tick against `base_tick`, length,
/// bytes)
pub fn write_section(w: &mut BitWriter, first: u16, events: &[NetEvent], base_tick: u32) {
    let n = events.len().clamp(1, MAX_RUN);
    w.u16(first);
    w.uvar(n as u32 - 1);
    for e in events.iter().take(n) {
        w.bits(e.channel as u32 & 15, 4);
        w.svar(tick_diff(e.tick, base_tick));
        let len = e.data.len().min(MAX_EVENT);
        w.uvar(len as u32);
        for &b in &e.data[..len] {
            w.u8(b);
        }
    }
}

/// read a run (None when malformed: a count or length out of range, the stream too short)
pub fn read_section(r: &mut BitReader, base_tick: u32) -> Option<(u16, Vec<NetEvent>)> {
    let first = r.u16();
    // (saturating: a peer's 0xFFFFFFFF + 1 overflowed on 32 bits)
    let n = (r.uvar() as usize).saturating_add(1);
    // (an event takes 16 bits at least: no allocation for a count the stream cannot hold)
    if n > MAX_RUN || n * 16 > r.bytes_left() * 8 + 8 {
        return None;
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let channel = r.bits(4) as u8;
        let tick = base_tick.wrapping_add(r.svar() as u32);
        let len = r.uvar() as usize;
        if len > MAX_EVENT || len > r.bytes_left() + 1 {
            return None;
        }
        let mut data = Vec::with_capacity(len);
        for _ in 0..len {
            data.push(r.u8());
        }
        if !r.ok() {
            return None;
        }
        v.push(NetEvent { tick, channel, data });
    }
    r.ok().then_some((first, v))
}

/// what an outbox counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutboxStats {
    /// events queued
    pub queued: u64,
    /// events refused (too long, a bad channel, the outbox full)
    pub refused: u64,
    /// events put into a packet for the first time
    pub sent: u64,
    /// ... and again (UDP: until acknowledged)
    pub resent: u64,
    /// events acknowledged (or, over a WebSocket, sent)
    pub done: u64,
    /// the bits of the runs written
    pub bits: u64,
}

/// the sender's side (see the module header)
#[derive(Clone, Debug)]
pub struct Outbox {
    q: VecDeque<(u16, NetEvent, u32)>,
    next: u16,
    reliable: bool,
    budget: usize,
    stats: OutboxStats,
}

impl Outbox {
    /// an empty outbox; `reliable`: the transport delivers (a WebSocket), each event goes once; `budget`: the bytes
    /// of the events a packet carries at most (it always carries at least one)
    pub fn new(reliable: bool, budget: usize) -> Outbox {
        Outbox { q: VecDeque::new(), next: 0, reliable, budget, stats: OutboxStats::default() }
    }

    /// queue an event -> false when refused (too long, a bad channel, MAX_PENDING waiting)
    pub fn push(&mut self, e: NetEvent) -> bool {
        if !e.valid() || self.q.len() >= MAX_PENDING {
            self.stats.refused += 1;
            return false;
        }
        self.q.push_back((self.next, e, 0));
        self.next = self.next.wrapping_add(1);
        self.stats.queued += 1;
        true
    }

    /// events waiting (not yet acknowledged; over a WebSocket: not yet sent)
    pub fn pending(&self) -> usize {
        self.q.len()
    }

    /// what it counted
    pub fn stats(&self) -> OutboxStats {
        self.stats
    }

    /// the run the next packet carries (the number of its first event and the events), None when nothing waits;
    /// ticks of the bit count against `base_tick` (the packet's tick)
    pub fn take_run(&mut self, base_tick: u32) -> Option<(u16, Vec<NetEvent>)> {
        let first = self.q.front()?.0;
        let budget = self.budget.saturating_mul(8);
        let mut bits = 16 + 8;
        let mut v = Vec::new();
        for (_, e, sends) in self.q.iter_mut() {
            let b = e.bits(base_tick);
            if !v.is_empty() && (bits + b > budget || v.len() >= MAX_RUN) {
                break;
            }
            bits += b;
            v.push(e.clone());
            if *sends == 0 {
                self.stats.sent += 1;
            } else {
                self.stats.resent += 1;
            }
            *sends += 1;
        }
        self.stats.bits += bits as u64;
        if self.reliable {
            for _ in 0..v.len() {
                self.q.pop_front();
            }
            self.stats.done += v.len() as u64;
        }
        Some((first, v))
    }

    /// the peer has every event up to `seq` (an acknowledgement of a number never sent is ignored)
    pub fn ack(&mut self, seq: u16) {
        if seq_diff(seq, self.next.wrapping_sub(1)) > 0 {
            return;
        }
        while self.q.front().is_some_and(|x| seq_diff(x.0, seq) <= 0) {
            self.q.pop_front();
            self.stats.done += 1;
        }
    }
}

/// what an inbox counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InboxStats {
    /// events delivered (once each)
    pub delivered: u64,
    /// events that came again (dropped)
    pub duplicates: u64,
    /// runs that started after the next expected number (an honest sender never sends one; dropped)
    pub gaps: u64,
    /// events dropped because the app did not take them (MAX_PENDING waiting)
    pub overflow: u64,
}

/// the receiver's side (see the module header)
#[derive(Clone, Debug, Default)]
pub struct Inbox {
    next: u16,
    got: VecDeque<NetEvent>,
    ack_left: u32,
    acks: bool,
    stats: InboxStats,
}

impl Inbox {
    /// an empty inbox expecting event 0; `acks`: acknowledge (false over a WebSocket, whose sender does not wait)
    pub fn new(acks: bool) -> Inbox {
        Inbox { acks, ..Inbox::default() }
    }

    /// a packet's run: the new events in order into the queue, duplicates dropped
    pub fn receive(&mut self, first: u16, events: Vec<NetEvent>) {
        if events.is_empty() {
            return;
        }
        if seq_diff(first, self.next) > 0 {
            self.stats.gaps += 1;
            return;
        }
        for (i, e) in events.into_iter().enumerate() {
            let seq = first.wrapping_add(i as u16);
            if seq_diff(seq, self.next) < 0 {
                self.stats.duplicates += 1;
                continue;
            }
            self.next = self.next.wrapping_add(1);
            if self.got.len() >= MAX_PENDING * 8 {
                self.stats.overflow += 1;
                continue;
            }
            self.got.push_back(e);
            self.stats.delivered += 1;
        }
        if self.acks {
            self.ack_left = ACK_PACKETS;
        }
    }

    /// the next event delivered (in order)
    pub fn take(&mut self) -> Option<NetEvent> {
        self.got.pop_front()
    }

    /// events delivered and not yet taken
    pub fn waiting(&self) -> usize {
        self.got.len()
    }

    /// the acknowledgement the next packet carries (every event up to it came), None when nothing came lately
    pub fn ack_for_packet(&mut self) -> Option<u16> {
        if self.ack_left == 0 {
            return None;
        }
        self.ack_left -= 1;
        Some(self.next.wrapping_sub(1))
    }

    /// what it counted
    pub fn stats(&self) -> InboxStats {
        self.stats
    }
}
