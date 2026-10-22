// android_audio -- (audio.rs) AAudio through the Android host (android/audio.c); the system
// sounds through the Java host (android/GalleryAudio.java)
//
//   open(feed, name) -> Dev   jfgui_host_audio_open: a started AAudio stream (f32, the stream's
//                             rate and channels, shared, low latency, a buffer of Core::dev_ms);
//                             its data callback is jfgui_audio_render below, on AAudio's thread
//                             (the Feed lives in a box the stream points to); no output:
//                             Unavailable
//   jfgui_audio_render        renders from the Feed (converted when AAudio opened at another
//                             rate); after the end a buffer's worth of silence, then 0: AAudio
//                             stops the stream (State::Ended; start() starts it again)
//   kick / close              START / PAUSE / FLUSH (paused, the buffer dropped) / CLOSE
//   system_sound(kind)        GalleryAudio over jfgui_host_call (CALL_SOUND): the notification
//                             sound for SOUND_NOTIFY, ToneGenerator beeps for the others
//                             (STREAM_SYSTEM: they follow the system's volume and mute)
//
// A disconnected device (headphones out) makes audio.c reopen the stream on the new default
// device (device_changes()); device() is the AAudio device id as text.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{AudioError, D_RUNNING, Feed, W_PLAY};

unsafe extern "C" {
    fn jfgui_host_audio_open(feed: *mut c_void, rate: i32, channels: i32, buffer_frames: i32) -> usize;
    fn jfgui_host_audio_ctl(h: usize, op: i32) -> i32;
    fn jfgui_host_audio_info(h: usize, what: i32) -> i64;
}

const OP_START: i32 = 1;
const OP_PAUSE: i32 = 2;
const OP_FLUSH: i32 = 3;
const OP_CLOSE: i32 = 4;
const INFO_BUFFER: i32 = 2;
const INFO_XRUNS: i32 = 4;
const INFO_CHANGES: i32 = 5;
const INFO_DEVICE: i32 = 6;

const CALL_SOUND: u32 = 40;

// what the callback owns
struct Render {
    feed: Feed,
    /// frames of silence rendered after the end
    silence: usize,
    /// the device buffer's frames (an estimate of what is still to be heard)
    buffer: AtomicUsize,
    /// the stream stopped itself at its end (AAudio state stopped)
    stopped: Arc<AtomicBool>,
}

/// a stream on AAudio
pub(crate) struct Dev {
    h: usize,
    render: *mut Render,
    core: Arc<super::Core>,
    stopped: Arc<AtomicBool>,
    playing: AtomicBool,
}

// SAFETY: the handle and the box are used through audio.c, which locks; the box is touched only
// by the callback until close.
unsafe impl Send for Dev {}
unsafe impl Sync for Dev {}

/// (android/audio.c) AAudio's data callback: frames at rate into out -> 0: the end was played
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_audio_render(user: *mut c_void, out: *mut f32, frames: i32, rate: i32) -> i32 {
    // SAFETY: user is the Render box of a stream that is open (close waits for callbacks).
    let r = unsafe { &mut *(user as *mut Render) };
    let core = r.feed.core.clone();
    let ch = core.channels as usize;
    let n = frames.max(0) as usize;
    // SAFETY: AAudio hands out n frames of f32 x channels for this callback.
    let buf = unsafe { core::slice::from_raw_parts_mut(out, n * ch) };
    let was_done = r.silence > 0 || core.ended.load(Ordering::Acquire);
    let more = r.feed.render(buf, rate.max(0) as u32);
    let buffer = r.buffer.load(Ordering::Relaxed);
    core.dev_queued.store(buffer as u32, Ordering::Relaxed);
    if more {
        r.silence = 0;
        return 1;
    }
    if was_done || r.silence > 0 {
        r.silence += n;
    } else {
        r.silence = 1; // the last real frames are in this buffer
    }
    if r.silence > buffer {
        core.ended.store(true, Ordering::Release);
        core.dev_queued.store(0, Ordering::Relaxed);
        r.silence = 0;
        r.stopped.store(true, Ordering::Release);
        return 0;
    }
    1
}

pub(crate) fn open(feed: Feed, _name: &str) -> Result<Dev, AudioError> {
    let core = feed.core.clone();
    let buffer = (core.rate as u64 * core.dev_ms as u64 / 1000) as usize;
    let stopped = Arc::new(AtomicBool::new(false));
    let render = Box::into_raw(Box::new(Render { feed, silence: 0, buffer: AtomicUsize::new(buffer), stopped: stopped.clone() }));
    // SAFETY: render stays valid until close; audio.c calls back only while the stream is open.
    let h = unsafe { jfgui_host_audio_open(render as *mut c_void, core.rate as i32, core.channels as i32, buffer as i32) };
    if h == 0 {
        // SAFETY: the stream was not made: the box is still ours alone.
        drop(unsafe { Box::from_raw(render) });
        return Err(AudioError::Unavailable);
    }
    // SAFETY: our open handle.
    let real = unsafe { jfgui_host_audio_info(h, INFO_BUFFER) };
    if real > 0 {
        // SAFETY: the box lives until close; the field is atomic (the callback may run already).
        unsafe { (*render).buffer.store(real as usize, Ordering::Relaxed) };
    }
    // SAFETY: our open handle.
    let dev = unsafe { jfgui_host_audio_info(h, INFO_DEVICE) };
    core.set_device(&alloc::format!("{dev}"));
    core.dev.store(D_RUNNING, Ordering::Release);
    Ok(Dev { h, render, core, stopped, playing: AtomicBool::new(true) })
}

pub(crate) fn kick(d: &Dev) {
    let c = &d.core;
    // SAFETY: our open handle (each call locks in audio.c).
    unsafe {
        if c.take_flush_device() {
            jfgui_host_audio_ctl(d.h, OP_FLUSH);
            d.playing.store(false, Ordering::Release);
        }
        // (after its end the stream stopped itself: start() starts it again)
        let wants = c.want() == W_PLAY && !c.ended.load(Ordering::Acquire);
        if wants {
            if d.stopped.swap(false, Ordering::AcqRel) || !d.playing.load(Ordering::Acquire) {
                jfgui_host_audio_ctl(d.h, OP_START);
                d.playing.store(true, Ordering::Release);
            }
        } else if c.want() != W_PLAY && d.playing.load(Ordering::Acquire) {
            jfgui_host_audio_ctl(d.h, OP_PAUSE);
            d.playing.store(false, Ordering::Release);
        }
        c.changes.store(jfgui_host_audio_info(d.h, INFO_CHANGES).max(0) as u32, Ordering::Relaxed);
        let x = jfgui_host_audio_info(d.h, INFO_XRUNS).max(0) as u32;
        if x > 0 {
            c.underruns.fetch_max(x, Ordering::Relaxed);
        }
    }
}

pub(crate) fn close(d: Dev) {
    // SAFETY: after CLOSE no callback runs any more: the box is ours again.
    unsafe {
        jfgui_host_audio_ctl(d.h, OP_CLOSE);
        drop(Box::from_raw(d.render));
    }
}

pub(crate) fn available() -> bool {
    true
}

pub(crate) fn system_sound(kind: u32) -> Option<bool> {
    let r = crate::web::android_host::call(CALL_SOUND, &kind.to_le_bytes())?;
    Some(r.first() == Some(&1))
}

pub(crate) fn dialog_sounds() -> bool {
    false
}
