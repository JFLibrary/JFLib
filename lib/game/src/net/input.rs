// input -- what a client sends every tick, the redundancy that makes a lost datagram cost nothing, and
// the server's buffer that hands each tick its input
//
//   NetInput        one tick's input, quantised: held buttons (the game's bits), two axes, yaw, pitch and
//                   the view time for lag compensation (how far behind the tick the client's picture of
//                   the others was, in 1/256 ticks)
//   write_inputs    the newest input whole, every older one against the next newer (1 bit when it is the
//                   same); read_inputs the inverse (None when malformed)
//   InputBuffer     the server's: put what arrives (late and far-future ticks are refused), take(tick)
//                   hands the tick its input or repeats the last one (a missed input), have() says up to
//                   which tick the inputs are complete (the client resends from there), the margin says
//                   how early inputs arrive (the client steers its clock by it)
//
// Ticks are u32 and wrap; every comparison goes through tick_diff.

use alloc::vec::Vec;

use super::bits::{BitReader, BitWriter};
use super::snapshot::tick_diff;

/// inputs a packet carries at most (the newest and the 15 before it)
pub const MAX_REDUNDANT: usize = 16;
/// ticks the server's buffer looks ahead (inputs further in the future are refused)
pub const BUFFER: usize = 64;

/// one tick's input (see the module header)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetInput {
    /// the buttons held (the game's bits; edges come from comparing with the previous tick)
    pub buttons: u16,
    /// right (+) and left (-), quant::axis_q
    pub move_x: i8,
    /// forward (+) and back (-), quant::axis_q
    pub move_z: i8,
    /// the view's yaw, quant::angle_q
    pub yaw: u16,
    /// the view's pitch, quant::pitch_q
    pub pitch: i16,
    /// how far the client's picture of the other entities lagged behind this tick, in 1/256 ticks (the
    /// lag compensation's view time is tick - view / 256)
    pub view: u16,
}

fn write_one(w: &mut BitWriter, i: &NetInput) {
    w.u16(i.buttons);
    w.u8(i.move_x as u8);
    w.u8(i.move_z as u8);
    w.u16(i.yaw);
    w.u16(i.pitch as u16);
    w.u16(i.view);
}

fn read_one(r: &mut BitReader) -> NetInput {
    NetInput { buttons: r.u16(), move_x: r.u8() as i8, move_z: r.u8() as i8, yaw: r.u16(), pitch: r.u16() as i16, view: r.u16() }
}

/// write `inputs` (the newest first, at most MAX_REDUNDANT) whose newest is for tick `newest`
pub fn write_inputs(w: &mut BitWriter, newest: u32, inputs: &[NetInput]) {
    let n = inputs.len().clamp(1, MAX_REDUNDANT);
    w.u32(newest);
    w.uvar(n as u32 - 1);
    let first = inputs.first().copied().unwrap_or_default();
    write_one(w, &first);
    let mut next = first;
    for i in inputs.iter().take(n).skip(1) {
        if *i == next {
            w.bool(true);
        } else {
            w.bool(false);
            let mask = (i.buttons != next.buttons) as u32
                | ((i.move_x != next.move_x) as u32) << 1
                | ((i.move_z != next.move_z) as u32) << 2
                | ((i.yaw != next.yaw) as u32) << 3
                | ((i.pitch != next.pitch) as u32) << 4
                | ((i.view != next.view) as u32) << 5;
            w.bits(mask, 6);
            if mask & 1 != 0 {
                w.u16(i.buttons);
            }
            if mask & 2 != 0 {
                w.u8(i.move_x as u8);
            }
            if mask & 4 != 0 {
                w.u8(i.move_z as u8);
            }
            if mask & 8 != 0 {
                w.svar(i.yaw.wrapping_sub(next.yaw) as i16 as i32);
            }
            if mask & 16 != 0 {
                w.svar(i.pitch as i32 - next.pitch as i32);
            }
            if mask & 32 != 0 {
                w.svar(i.view as i32 - next.view as i32);
            }
        }
        next = *i;
    }
}

/// read what write_inputs wrote -> (the newest tick, the inputs newest first); None when malformed
pub fn read_inputs(r: &mut BitReader) -> Option<(u32, Vec<NetInput>)> {
    let newest = r.u32();
    // (saturating: a peer's 0xFFFFFFFF + 1 overflowed on 32 bits)
    let n = (r.uvar() as usize).saturating_add(1);
    if !r.ok() || n > MAX_REDUNDANT {
        return None;
    }
    let mut v = Vec::with_capacity(n);
    let first = read_one(r);
    v.push(first);
    let mut next = first;
    for _ in 1..n {
        let i = if r.bool() {
            next
        } else {
            let mask = r.bits(6);
            let mut i = next;
            if mask & 1 != 0 {
                i.buttons = r.u16();
            }
            if mask & 2 != 0 {
                i.move_x = r.u8() as i8;
            }
            if mask & 4 != 0 {
                i.move_z = r.u8() as i8;
            }
            if mask & 8 != 0 {
                i.yaw = next.yaw.wrapping_add(r.svar() as u16);
            }
            if mask & 16 != 0 {
                let p = next.pitch as i32 + r.svar();
                if !(i16::MIN as i32..=i16::MAX as i32).contains(&p) {
                    r.fail();
                }
                i.pitch = p as i16;
            }
            if mask & 32 != 0 {
                let x = next.view as i32 + r.svar();
                if !(0..=u16::MAX as i32).contains(&x) {
                    r.fail();
                }
                i.view = x as u16;
            }
            i
        };
        v.push(i);
        next = i;
    }
    if !r.ok() {
        return None;
    }
    Some((newest, v))
}

/// what an input buffer counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputStats {
    /// inputs taken in (each tick once)
    pub received: u64,
    /// inputs that came again (redundancy)
    pub repeated: u64,
    /// inputs for ticks already simulated
    pub late: u64,
    /// inputs too far in the future (refused)
    pub early: u64,
    /// ticks simulated without the client's input (the last one repeated)
    pub missed: u64,
}

/// the server's inputs of one client (see the module header)
#[derive(Clone, Debug)]
pub struct InputBuffer {
    slots: Vec<Option<(u32, NetInput)>>,
    /// the next tick take() hands out
    next: u32,
    /// the input used last (repeated for a missed tick)
    last: NetInput,
    /// the smallest margin (newest tick of a packet minus the next tick) since take_margin
    margin: Option<i32>,
    stats: InputStats,
}

impl InputBuffer {
    /// a buffer whose first tick is `next`
    pub fn new(next: u32) -> InputBuffer {
        InputBuffer { slots: alloc::vec![None; BUFFER], next, last: NetInput::default(), margin: None, stats: InputStats::default() }
    }

    /// a packet's inputs (newest first, the newest for tick `newest`)
    pub fn put_packet(&mut self, newest: u32, inputs: &[NetInput]) {
        let m = tick_diff(newest, self.next);
        self.margin = Some(self.margin.map_or(m, |x| x.min(m)));
        for (k, i) in inputs.iter().enumerate() {
            self.put(newest.wrapping_sub(k as u32), *i);
        }
    }

    /// one input for `tick`; false when refused (late, too far ahead, or already there)
    pub fn put(&mut self, tick: u32, input: NetInput) -> bool {
        let d = tick_diff(tick, self.next);
        if d < 0 {
            self.stats.late += 1;
            return false;
        }
        if d >= BUFFER as i32 {
            self.stats.early += 1;
            return false;
        }
        let k = tick as usize % BUFFER;
        if self.slots[k].is_some_and(|(t, _)| t == tick) {
            self.stats.repeated += 1;
            return false;
        }
        self.slots[k] = Some((tick, input));
        self.stats.received += 1;
        true
    }

    /// the input of `tick` (the buffer moves past it) and whether it is the client's own (false: missed,
    /// the last one repeated)
    pub fn take(&mut self, tick: u32) -> (NetInput, bool) {
        // ticks skipped over (the buffer was behind) are dropped
        if tick_diff(tick, self.next) > BUFFER as i32 {
            self.slots.iter_mut().for_each(|s| *s = None);
            self.next = tick;
        }
        while tick_diff(tick, self.next) > 0 {
            let k = self.next as usize % BUFFER;
            if self.slots[k].is_some_and(|(t, _)| t == self.next) {
                self.slots[k] = None;
            }
            self.next = self.next.wrapping_add(1);
        }
        let k = tick as usize % BUFFER;
        let got = match self.slots[k] {
            Some((t, i)) if t == tick => {
                self.slots[k] = None;
                Some(i)
            }
            _ => None,
        };
        self.next = tick.wrapping_add(1);
        match got {
            Some(i) => {
                self.last = i;
                (i, true)
            }
            None => {
                self.stats.missed += 1;
                (self.last, false)
            }
        }
    }

    /// the newest tick up to which every input from the next one on is here (next - 1 when the next is
    /// missing)
    pub fn have(&self) -> u32 {
        let mut t = self.next;
        for _ in 0..BUFFER {
            let k = t as usize % BUFFER;
            if !self.slots[k].is_some_and(|(x, _)| x == t) {
                break;
            }
            t = t.wrapping_add(1);
        }
        t.wrapping_sub(1)
    }

    /// the smallest margin since the last call (None: no packet came)
    pub fn take_margin(&mut self) -> Option<i32> {
        self.margin.take()
    }

    /// the next tick take() hands out
    pub fn next(&self) -> u32 {
        self.next
    }

    /// what it counted
    pub fn stats(&self) -> InputStats {
        self.stats
    }
}
