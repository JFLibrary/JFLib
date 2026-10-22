// web_audio -- (audio.rs) WebAudio through the shared browser host (web/audio.js; loader.js
// owns the AudioContext, worker.js pumps)
//
//   open(feed, name) -> Dev   a REQUEST_AUDIO notice (OPEN: id, rate, channels, latency
//                             frames); the page makes an AudioWorkletNode and hands the worker
//                             a MessagePort to it; jfgui_audio_ready(id, device rate, ok) tells
//                             us (State::Starting until then; no WebAudio: Lost)
//   jfgui_audio_pull(id, want, played, base_us, flags) -> frames | ENDED
//                             the worker's pump (every 10 ms and each tick): want frames at the
//                             context's rate rendered into the buffer of jfgui_audio_data()
//                             (converted from the stream's rate here), which the worker copies
//                             into the worklet's SharedArrayBuffer ring (cross-origin isolated
//                             pages) or posts to it; played = frames the worklet consumed,
//                             base_us = the context's own latency, flags: 1 the context is
//                             suspended (no user gesture yet: State::Blocked), 2 all sent frames
//                             were played after the end
//   kick / close              PLAY / PAUSE / FLUSH / CLOSE notices for the worklet
//
// The fill callback runs in the pump, on the worker's thread; the browser follows its default
// output device itself (device() stays "").

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use rt::sync::SpinLock;

use super::{AudioError, D_BLOCKED, D_LOST, D_RUNNING, Feed, W_PLAY};

const OP_OPEN: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PLAY: u32 = 3;
const OP_PAUSE: u32 = 4;
const OP_FLUSH: u32 = 5;
const ENDED: u32 = 0x8000_0000;
const F_BLOCKED: u32 = 1;
const F_DRAINED: u32 = 2;

struct Slot {
    id: u32,
    /// None while the pump renders from it
    feed: Option<Feed>,
    dev_rate: u32,
    channels: usize,
    /// frames sent at the device rate
    sent: u64,
    ending: bool,
}

static SLOTS: SpinLock<Vec<Slot>> = SpinLock::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static BUF: SpinLock<Vec<f32>> = SpinLock::new(Vec::new());

/// a stream of the page
pub(crate) struct Dev {
    id: u32,
    core: alloc::sync::Arc<super::Core>,
    /// the want last told to the page
    told: AtomicU8,
}

fn notice(op: u32, id: u32, a: u32, b: u32, c: u32) {
    let mut p = Vec::with_capacity(20);
    for v in [op, id, a, b, c] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    crate::web::notice(crate::web::REQUEST_AUDIO, &p);
}

pub(crate) fn open(feed: Feed, _name: &str) -> Result<Dev, AudioError> {
    let core = feed.core.clone();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let latency = (core.rate as u64 * core.dev_ms as u64 / 1000) as u32;
    SLOTS.lock().push(Slot { id, feed: Some(feed), dev_rate: 0, channels: core.channels as usize, sent: 0, ending: false });
    notice(OP_OPEN, id, core.rate, core.channels as u32, latency);
    Ok(Dev { id, core, told: AtomicU8::new(W_PLAY) })
}

pub(crate) fn kick(d: &Dev) {
    let want = d.core.want();
    if d.core.take_flush_device() {
        notice(OP_FLUSH, d.id, 0, 0, 0);
    }
    let wants = want == W_PLAY && !d.core.ended.load(Ordering::Acquire);
    let told = d.told.swap(if wants { W_PLAY } else { want.max(1) }, Ordering::AcqRel);
    if (told == W_PLAY) != wants {
        notice(if wants { OP_PLAY } else { OP_PAUSE }, d.id, 0, 0, 0);
    }
}

pub(crate) fn close(d: Dev) {
    SLOTS.lock().retain(|s| s.id != d.id);
    notice(OP_CLOSE, d.id, 0, 0, 0);
}

/// (worker.js) the page answered OPEN: the context's rate, ok 0: no WebAudio here
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_audio_ready(id: u32, dev_rate: u32, ok: u32) -> u32 {
    let mut slots = SLOTS.lock();
    let Some(s) = slots.iter_mut().find(|s| s.id == id) else { return 0 };
    s.dev_rate = dev_rate;
    if let Some(f) = &s.feed {
        f.core.dev.store(if ok != 0 && dev_rate != 0 { D_RUNNING } else { D_LOST }, Ordering::Release);
    }
    1
}

/// (worker.js) the samples of the last jfgui_audio_pull
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_audio_data() -> usize {
    BUF.lock().as_ptr() as usize
}

/// (worker.js) render up to `want` frames at the device rate -> frames rendered, ENDED (bit
/// 31) after the stream's last frames; u32::MAX: no such stream (closed)
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_audio_pull(id: u32, want: u32, played: u32, base_us: u32, flags: u32) -> u32 {
    // take the feed out while it renders (a fill may call into plat::audio)
    let (mut feed, dev_rate, ch, sent, ending) = {
        let mut slots = SLOTS.lock();
        let Some(s) = slots.iter_mut().find(|s| s.id == id) else { return u32::MAX };
        let Some(f) = s.feed.take() else { return 0 };
        (f, s.dev_rate, s.channels, s.sent, s.ending)
    };
    let core = feed.core.clone();
    feed.apply_flush();
    // what the worklet holds, in the stream's frames
    let queued = sent.wrapping_sub(played as u64) as u32 as u64; // played wraps at 2^32
    let queued = queued * core.rate as u64 / dev_rate.max(1) as u64;
    core.dev_queued.store(queued.min(u32::MAX as u64) as u32, Ordering::Relaxed);
    core.dev_extra_us.store(base_us, Ordering::Relaxed);
    if core.dev.load(Ordering::Acquire) != D_LOST {
        core.dev.store(if flags & F_BLOCKED != 0 { D_BLOCKED } else { D_RUNNING }, Ordering::Release);
    }
    let mut n = 0u32;
    let mut end = ending;
    if ending && flags & F_DRAINED != 0 {
        core.ended.store(true, Ordering::Release);
        core.dev_queued.store(0, Ordering::Relaxed);
        end = false;
    } else if want > 0 && dev_rate > 0 && !ending && feed.wants_device() {
        let frames = want.min(16_384) as usize;
        let mut buf = BUF.lock();
        buf.clear();
        buf.resize(frames * ch, 0.0);
        if !feed.render(&mut buf, dev_rate) {
            end = true;
        }
        n = frames as u32;
    }
    let mut slots = SLOTS.lock();
    if let Some(s) = slots.iter_mut().find(|s| s.id == id) {
        s.feed = Some(feed);
        s.sent = sent + n as u64;
        s.ending = end;
        if core.ended.load(Ordering::Acquire) && end {
            s.ending = false;
        }
    }
    if end && n > 0 { n | ENDED } else { n }
}

pub(crate) fn available() -> bool {
    true
}

pub(crate) fn system_sound(_kind: u32) -> Option<bool> {
    None // a generated tone through play()
}

pub(crate) fn dialog_sounds() -> bool {
    false
}
