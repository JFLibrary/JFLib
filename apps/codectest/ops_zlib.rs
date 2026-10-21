// ops_zlib -- codectest ops 300..319: reusable deflate / inflate states, caller-provided
// output buffers, thr_self, history-style benchmarks          [apps/codectest/src/zlib_ops.s]
//
//   op 300 deflate sequence   arg bit 0: the *_to functions into a buffer of `cap` bytes
//          (else the allocating *_st ones), bit 1: no shared state; data = items {i32 level,
//          u32 kind (0 zlib, 1 raw, 2 gzip), u32 cap (0 = deflate_bound(len)), u32 len,
//          bytes}, all with ONE deflate state -> result 0, per item {i32 result, u32 n, bytes}
//   op 301 inflate sequence   arg bit 1: no shared state; data = items {u32 kind, u32 cap,
//          u32 len, bytes}, ONE inflate state -> per item {i32 result, u32 consumed, bytes}
//   op 302 history deflate benchmark  arg = mode: 0 zlib_deflate per 16 KB tile, 1 one
//          deflate state per thread (thr_self) + zlib_deflate_to; data = {u32 tiles, u32
//          pattern, u32 level, u32 reps} -> {u64 best us, u64 first us, u64 bytes kept, u64
//          FNV-1a of the kept streams, u32 threads, u32 kept}
//   op 303 history inflate benchmark  arg = mode: 0 zlib_inflate, 1 per-thread states ->
//          {u64 best us, u64 first us, u32 errors, u32 threads}
//   op 304 thr_self           arg = items -> {u32 conflicts, u32 max slot, u32 workers,
//          u32 bad, u64 slots seen}

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};

use crate::{Ct, u32_at};
use img_core::deflate::{DeflateState, deflate_bound, deflate_raw_st, deflate_raw_to, zlib_deflate, zlib_deflate_st,
                        zlib_deflate_to};
use img_core::gzip::{gzip_decode_st, gzip_encode_st, gzip_encode_to};
use img_core::inflate::{InflateState, inflate_raw_st, zlib_inflate, zlib_inflate_st};
use img_core::{ERR_HDR, ERR_NOMEM};
use rt::sync::SpinLock;

const ZB_TILE: usize = 16384; // history tile (64 x 64 BGRA)
const ZB_ZMAX: usize = ZB_TILE - 1024; // kept when it saves 1 KB
const ZB_ZCAP: usize = ZB_ZMAX + 16; // zlib_deflate_to capacity per tile
const ZB_BATCH: usize = 256; // tiles per parallel_for
const ZB_SLOTS: usize = 65; // THR_MAX + 1

/// run op (one record via ct.record); false = unknown op
pub fn op(ct: &mut Ct, op: u32, arg: u32, data: &[u8]) -> bool {
    match op {
        300 => match defseq(arg, data) {
            Some(o) => ct.record(0, &o),
            None => ct.record(ERR_HDR, &[]),
        },
        301 => match infseq(arg, data) {
            Some(o) => ct.record(0, &o),
            None => ct.record(ERR_HDR, &[]),
        },
        302 | 303 => bench(ct, op, arg, data),
        304 => zself(ct, arg),
        _ => return false,
    }
    true
}

fn item(o: &mut Vec<u8>, res: i64, bytes: &[u8]) {
    o.extend_from_slice(&(res as i32).to_le_bytes());
    let n = if res > 0 { res as usize } else { 0 };
    o.extend_from_slice(&(n as u32).to_le_bytes());
    o.extend_from_slice(&bytes[..n]);
}

// op 300
fn defseq(arg: u32, data: &[u8]) -> Option<Vec<u8>> {
    let mut st = if arg & 2 == 0 { Some(DeflateState::new()) } else { None };
    let mut o = Vec::new();
    let mut p = 0usize;
    while data.len() - p >= 16 {
        let level = u32_at(data, p) as i32;
        let kind = u32_at(data, p + 4);
        let cap = u32_at(data, p + 8) as usize;
        let len = u32_at(data, p + 12) as usize;
        if len > data.len() - p - 16 {
            return None;
        }
        let src = &data[p + 16..p + 16 + len];
        p += 16 + len;
        let s = st.as_deref_mut();
        if arg & 1 != 0 {
            let cap = if cap == 0 { deflate_bound(len) } else { cap };
            let mut buf = vec![0u8; cap];
            let r = match kind {
                0 => zlib_deflate_to(s, src, level, &mut buf),
                1 => deflate_raw_to(s, src, level, &mut buf),
                _ => gzip_encode_to(s, src, level, &mut buf),
            };
            let res = match r {
                Ok(n) => n as i64,
                Err(e) => e as i64,
            };
            item(&mut o, res, &buf);
        } else {
            let v = match kind {
                0 => zlib_deflate_st(s, src, level),
                1 => deflate_raw_st(s, src, level),
                _ => gzip_encode_st(s, src, level),
            };
            item(&mut o, v.len() as i64, &v);
        }
    }
    if p != data.len() {
        return None;
    }
    Some(o)
}

// op 301
fn infseq(arg: u32, data: &[u8]) -> Option<Vec<u8>> {
    let mut st: Option<Box<InflateState>> = if arg & 2 == 0 { Some(InflateState::new()) } else { None };
    let mut o = Vec::new();
    let mut p = 0usize;
    while data.len() - p >= 12 {
        let kind = u32_at(data, p);
        let cap = u32_at(data, p + 4) as usize;
        let len = u32_at(data, p + 8) as usize;
        if len > data.len() - p - 12 {
            return None;
        }
        let src = &data[p + 12..p + 12 + len];
        p += 12 + len;
        let mut buf = vec![0u8; cap];
        let s = st.as_deref_mut();
        let (res, used) = match kind {
            0 => match zlib_inflate_st(s, src, &mut buf) {
                Ok(n) => (n as i64, 0),
                Err(e) => (e as i64, 0),
            },
            1 => match inflate_raw_st(s, src, &mut buf) {
                Ok((n, u)) => (n as i64, u),
                Err(e) => (e as i64, 0),
            },
            _ => match gzip_decode_st(s, src, &mut buf) {
                Ok((n, u)) => (n as i64, u),
                Err(e) => (e as i64, 0),
            },
        };
        o.extend_from_slice(&(res as i32).to_le_bytes());
        o.extend_from_slice(&(used as u32).to_le_bytes());
        if res > 0 {
            o.extend_from_slice(&buf[..res as usize]);
        }
    }
    if p != data.len() {
        return None;
    }
    Some(o)
}

// ---------------------------------------------------------------- benchmarks

// per-thread states (slot thr_self(); a busy flag against two non-worker threads sharing
// the last slot): created on first use, kept for later runs
static DST: SpinLock<Vec<Option<Box<DeflateState>>>> = SpinLock::new(Vec::new());
static IST: SpinLock<Vec<Option<Box<InflateState>>>> = SpinLock::new(Vec::new());

// zb_acquire: the slot's state (None: busy -> a temporary one)
fn take<T>(slots: &SpinLock<Vec<Option<Box<T>>>>, busy: &[AtomicU32], new: fn() -> Box<T>) -> (Option<Box<T>>, usize) {
    let k = plat::thr_self().min(ZB_SLOTS - 1);
    if busy[k].swap(1, SeqCst) != 0 {
        return (None, usize::MAX);
    }
    let mut v = slots.lock();
    if v.len() < ZB_SLOTS {
        v.resize_with(ZB_SLOTS, || None);
    }
    let s = v[k].take().unwrap_or_else(new);
    (Some(s), k)
}

fn give<T>(slots: &SpinLock<Vec<Option<Box<T>>>>, busy: &[AtomicU32], s: Option<Box<T>>, k: usize) {
    if k == usize::MAX {
        return;
    }
    slots.lock()[k] = s;
    busy[k].store(0, SeqCst);
}

static DBUSY: [AtomicU32; ZB_SLOTS] = [const { AtomicU32::new(0) }; ZB_SLOTS];
static IBUSY: [AtomicU32; ZB_SLOTS] = [const { AtomicU32::new(0) }; ZB_SLOTS];

// zb_setup: the input tiles (pattern 0 noisy gradient, 1 random, 2 smooth)
fn tiles(n: usize, pattern: u32) -> Vec<u8> {
    let mut t = vec![0u8; n * ZB_TILE];
    let mut lcg = 0x1234_5678u32;
    for (i, b) in t.iter_mut().enumerate() {
        lcg = lcg.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let mut v = lcg >> 24;
        if pattern != 1 {
            let c = (i & 3) as u32;
            if c == 3 {
                v = 255; // opaque alpha
            } else {
                let px = ((i & (ZB_TILE - 1)) >> 2) as u32;
                let tile = (i >> 14) as u32;
                v = if pattern == 2 {
                    (px & 63) * 4 + c * 16 + tile
                } else {
                    (px & 63) + (px >> 6) + tile + c * 8 + (lcg >> 29)
                };
            }
        }
        *b = v as u8;
    }
    t
}

fn fnv64(d: &[u8]) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for &b in d {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
    }
    h
}

// ops 302 / 303
fn bench(ct: &mut Ct, op: u32, mode: u32, data: &[u8]) {
    if data.len() < 16 {
        ct.record(ERR_HDR, &[]);
        return;
    }
    let n = u32_at(data, 0) as usize;
    if n == 0 || n > 65536 {
        ct.record(ERR_HDR, &[]);
        return;
    }
    let (pattern, level) = (u32_at(data, 4), u32_at(data, 8) as i32);
    let reps = u32_at(data, 12).max(1);
    let src = tiles(n, pattern);
    let threads = plat::thr_count();
    let mut best = u64::MAX;
    let mut first = 0u64;
    let mut o = Vec::new();
    if op == 302 {
        let mut pool = Vec::new();
        let mut kept = 0u32;
        for rep in 0..reps {
            let t0 = plat::time_us();
            pool.clear();
            kept = 0;
            let mut base = 0usize;
            while base < n {
                let k = (n - base).min(ZB_BATCH);
                let res: Vec<SpinLock<(Vec<u8>, i64)>> = (0..k).map(|_| SpinLock::new((Vec::new(), 0))).collect();
                let f = |i: usize, _: &plat::JobCtl| {
                    let tile = &src[(base + i) * ZB_TILE..(base + i + 1) * ZB_TILE];
                    let r = if mode == 0 {
                        let v = zlib_deflate(tile, level);
                        let l = v.len() as i64;
                        (v, l)
                    } else {
                        let (mut st, slot) = take(&DST, &DBUSY, DeflateState::new);
                        let mut buf = vec![0u8; ZB_ZCAP];
                        let r = zlib_deflate_to(st.as_deref_mut(), tile, level, &mut buf);
                        give(&DST, &DBUSY, st, slot);
                        match r {
                            Ok(l) => {
                                buf.truncate(l);
                                (buf, l as i64)
                            }
                            Err(e) => (Vec::new(), e as i64),
                        }
                    };
                    *res[i].lock() = r;
                };
                let _ = plat::parallel_for(k, &f);
                for r in &res {
                    let g = r.lock();
                    if g.1 >= 0 && g.1 as usize <= ZB_ZMAX {
                        pool.extend_from_slice(&g.0);
                        kept += 1;
                    }
                }
                base += k;
            }
            let dt = plat::time_us() - t0;
            if rep == 0 {
                first = dt;
            }
            best = best.min(dt);
        }
        o.extend_from_slice(&best.to_le_bytes());
        o.extend_from_slice(&first.to_le_bytes());
        o.extend_from_slice(&(pool.len() as u64).to_le_bytes());
        o.extend_from_slice(&fnv64(&pool).to_le_bytes());
        o.extend_from_slice(&threads.to_le_bytes());
        o.extend_from_slice(&kept.to_le_bytes());
    } else {
        // every tile compressed once (level), then decoded `reps` times
        let mut st = DeflateState::new();
        let mut z: Vec<(usize, usize)> = Vec::with_capacity(n);
        let mut zdata = vec![0u8; deflate_bound(ZB_TILE) * n];
        let mut off = 0usize;
        for t in 0..n {
            let cap = deflate_bound(ZB_TILE);
            match zlib_deflate_to(Some(&mut st), &src[t * ZB_TILE..(t + 1) * ZB_TILE], level, &mut zdata[off..off + cap]) {
                Ok(l) => {
                    z.push((off, l));
                    off += l;
                }
                Err(_) => {
                    ct.record(ERR_NOMEM, &[]);
                    return;
                }
            }
        }
        let errs = AtomicU32::new(0);
        for rep in 0..reps {
            let t0 = plat::time_us();
            errs.store(0, SeqCst);
            let f = |i: usize, _: &plat::JobCtl| {
                let (zo, zl) = z[i];
                let stream = &zdata[zo..zo + zl];
                let mut out = vec![0u8; ZB_TILE];
                let r = if mode == 0 {
                    zlib_inflate(stream, &mut out)
                } else {
                    let (mut st, slot) = take(&IST, &IBUSY, InflateState::new);
                    let r = zlib_inflate_st(st.as_deref_mut(), stream, &mut out);
                    give(&IST, &IBUSY, st, slot);
                    r
                };
                if r != Ok(ZB_TILE) || out[..] != src[i * ZB_TILE..(i + 1) * ZB_TILE] {
                    errs.fetch_add(1, SeqCst);
                }
            };
            let _ = plat::parallel_for(n, &f);
            let dt = plat::time_us() - t0;
            if rep == 0 {
                first = dt;
            }
            best = best.min(dt);
        }
        o.extend_from_slice(&best.to_le_bytes());
        o.extend_from_slice(&first.to_le_bytes());
        o.extend_from_slice(&errs.load(SeqCst).to_le_bytes());
        o.extend_from_slice(&threads.to_le_bytes());
    }
    ct.record(0, &o);
}

// op 304: every item holds slot thr_self() busy for a moment; two items on the same slot at
// once = a conflict
fn zself(ct: &mut Ct, items: u32) {
    let conf = AtomicU32::new(0);
    let bad = AtomicU32::new(0);
    let mx = AtomicU32::new(0);
    let seen = AtomicU64::new(0);
    let busy = [AtomicU64::new(0), AtomicU64::new(0)];
    let n = plat::thr_count();
    let f = |_: usize, _: &plat::JobCtl| {
        let mut s = plat::thr_self() as u32;
        if s > n {
            bad.fetch_add(1, SeqCst);
            return;
        }
        s = s.min(64);
        mx.fetch_max(s, SeqCst);
        seen.fetch_or(1u64 << s.min(63), SeqCst);
        let (w, b) = ((s >> 6) as usize, 1u64 << (s & 63));
        if busy[w].fetch_or(b, SeqCst) & b != 0 {
            conf.fetch_add(1, SeqCst);
            return;
        }
        for _ in 0..400 {
            core::hint::spin_loop();
        }
        busy[w].fetch_and(!b, SeqCst);
    };
    let _ = plat::parallel_for(items as usize, &f);
    let mut o = Vec::with_capacity(24);
    for v in [conf.load(SeqCst), mx.load(SeqCst), n, bad.load(SeqCst)] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    o.extend_from_slice(&seen.load(SeqCst).to_le_bytes());
    ct.record(0, &o);
}
