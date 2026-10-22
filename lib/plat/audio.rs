// audio -- sound output: streams the app fills, short sounds from memory or files, the
// system's own sounds
//
//   Stream::open(&Config) -> Result<Stream, AudioError>   the app pushes samples into the
//                           stream's ring (write / write_i16; never blocks, takes what fits),
//                           the platform drains it on its audio thread
//   Stream::open_fill(&Config, fill)   fill(&mut [f32]) -> bool tops up the device's buffer
//                           instead (false: this was the last buffer)
//   s.write(&[f32]) / s.write_i16(&[i16]) -> frames taken;  s.writable(), s.queued()
//   s.start() / s.pause() / s.stop() (empties the ring) / s.finish() (ends once all is played)
//   s.set_volume(0 ..= 1), s.state() -> State, s.latency_ms(), s.played(), s.underruns(),
//   s.device(), s.device_changes()
//   play(&Sound) -> Result<Voice, AudioError>    a short sound through the shared mixer
//   play_file(path)         a WAV or Ogg Vorbis file through the VFS (app://, data://, ..)
//   Sound::wav(bytes) / ogg / decode / load(path) / tone(freq, ms)
//   sound(SOUND_*) -> bool  the system's own sound for an event (dialog_sounds(): whether the
//                           platform plays one when a message dialog opens)
//   available() -> bool     an output device / sound service exists here
//
// Samples are f32 in -1 ..= 1, interleaved frames (WAVE channel order: 1 mono, 2 left / right,
// 6 = 5.1); the device side always runs in f32 at the stream's rate (the system converts the
// rate and the channels where needed; the browser's context rate is converted here).
//
// One model everywhere: the ring.  The app writes, a platform thread drains -- this works on
// all four targets, where a callback on the audio thread does not: the browser's audio runs in
// an AudioWorklet that cannot call into the app's WASM instance, so it is fed from a buffer
// anyway.  The ring also keeps app code (allocation, locks, the UI's pace) off the real-time
// thread.  open_fill is the same stream without a ring: fill runs on the audio thread
// (Windows, Linux, Android) or in the browser worker's pump, and must not block.
//
// Latency: Config::latency_ms (default 40) is split between the device buffer (half, 10 ..= 100
// ms) and the ring (the rest; writable() keeps the ring within it); a fill stream gives all of it
// to the device buffer.  An app that writes from UI timers (16 ms frames) wants >= 50 ms.
// A stream starts playing at once (silence until data arrives); pause() first to prefill.
//
// Platforms: Windows WASAPI shared mode, event-driven on its own thread, the default device
// followed through IMMNotificationClient (win_audio.rs); Linux the PulseAudio native protocol
// (pipewire-pulse on current desktops, WSLg's server), the server moves the stream to a new
// default device itself (linux_audio.rs); the browser WebAudio, an AudioWorklet fed from a
// SharedArrayBuffer ring or posted buffers, started after a user gesture (State::Blocked
// before; web_audio.rs, web/audio.js); Android AAudio through the JNI host (android_audio.rs,
// android/bridge.c).  No device or service: AudioError::Unavailable.
//
// play() mixes into one stream of 48 kHz stereo that opens on demand and ends after 2 s of
// silence (each sound converted to 48 kHz once, while it plays).  System sounds: Windows the
// sound scheme's aliases (PlaySound: SystemAsterisk, SystemExclamation, SystemHand,
// SystemQuestion, .Default, Notification.Default), Linux the freedesktop sound theme
// (dialog-information, dialog-warning, dialog-error, dialog-question, bell, message-new-instant
// as .oga / .ogg / .wav, the theme of org.gnome.desktop.sound, only while its event-sounds
// are on), Android the notification sound and ToneGenerator beeps, the browser short generated
// tones (as Linux without a theme).  jfgui_AUDIO_DRYRUN=1 / set_dry_run: sound() only prints
// "sound: NAME".

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use rt::sync::SpinLock;

#[path = "audio_dsp.rs"]
mod dsp;
#[path = "audio_wav.rs"]
mod wav;
#[path = "audio_vorbis.rs"]
mod vorbis;

use dsp::{Resampler, Ring};

/// the default beep (Windows ".Default", freedesktop "bell")
pub const SOUND_DEFAULT: u32 = 0;
/// information (SystemAsterisk, dialog-information)
pub const SOUND_INFO: u32 = 1;
/// a warning (SystemExclamation, dialog-warning)
pub const SOUND_WARNING: u32 = 2;
/// an error (SystemHand, dialog-error)
pub const SOUND_ERROR: u32 = 3;
/// a question (SystemQuestion, dialog-question)
pub const SOUND_QUESTION: u32 = 4;
/// a notification (Notification.Default, message-new-instant)
pub const SOUND_NOTIFY: u32 = 5;

/// why audio is not possible
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// no output device or sound service here
    Unavailable,
    /// a rate or channel count out of range
    Invalid,
    /// not a sound file read here (WAV PCM / float, Ogg Vorbis) or a damaged one
    Format,
    /// the file could not be read
    Io,
    /// the system refused (its code: an HRESULT, a PulseAudio error, an AAudio result)
    Failed(i32),
}

impl core::fmt::Display for AudioError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            AudioError::Unavailable => f.write_str("no audio output here"),
            AudioError::Invalid => f.write_str("audio format out of range"),
            AudioError::Format => f.write_str("not a readable sound file"),
            AudioError::Io => f.write_str("sound file not readable"),
            AudioError::Failed(c) => write!(f, "audio failed ({c:#x})"),
        }
    }
}

/// what a stream does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// the device is being opened (browser: until its host answers)
    Starting,
    Playing,
    Paused,
    /// stop(): the ring emptied; start() plays again
    Stopped,
    /// finish() or a fill that returned false: everything was played (start() plays again)
    Ended,
    /// the browser waits for a user gesture before it plays sound
    Blocked,
    /// no device right now (the old one went away); the stream plays again when one appears
    Lost,
}

/// what a stream plays
#[derive(Clone, Copy, Debug)]
pub struct Config<'a> {
    /// frames per second (8000 ..= 192000)
    pub rate: u32,
    /// interleaved channels (1 ..= 8)
    pub channels: u16,
    /// the latency aimed for, device buffer and ring together (10 ..= 1000 ms)
    pub latency_ms: u32,
    /// the stream's name in the system's mixer ("" = the app's name)
    pub name: &'a str,
}

impl Config<'static> {
    /// rate and channels, 40 ms, the app's name
    pub const fn new(rate: u32, channels: u16) -> Config<'static> {
        Config { rate, channels, latency_ms: 40, name: "" }
    }
}

// what the app asked for (Core::want)
pub(crate) const W_PLAY: u8 = 0;
pub(crate) const W_PAUSE: u8 = 1;
pub(crate) const W_STOP: u8 = 2;
// what the device does (Core::dev)
pub(crate) const D_STARTING: u8 = 0;
pub(crate) const D_RUNNING: u8 = 1;
pub(crate) const D_BLOCKED: u8 = 2;
pub(crate) const D_LOST: u8 = 3;

/// the state a stream's app side and its platform side share
pub(crate) struct Core {
    pub(crate) rate: u32,
    pub(crate) channels: u16,
    /// the device buffer aimed for (ms)
    pub(crate) dev_ms: u32,
    ring: Option<Ring>,
    ring_frames: usize,
    want: AtomicU8,
    pub(crate) dev: AtomicU8,
    finish: AtomicBool,
    flush: AtomicBool,
    flush_mark: AtomicUsize,
    flush_dev: AtomicBool,
    rearm: AtomicBool,
    volume: AtomicU32,
    rendered: AtomicU64,
    /// frames in the device's buffer, not played yet (platform)
    pub(crate) dev_queued: AtomicU32,
    /// further latency of the device / server (platform, microseconds)
    pub(crate) dev_extra_us: AtomicU32,
    pub(crate) underruns: AtomicU32,
    pub(crate) device: SpinLock<String>,
    pub(crate) changes: AtomicU32,
    /// everything was played (platform, after draining the device)
    pub(crate) ended: AtomicBool,
    /// the server's id of the stream (PulseAudio: the sink input; tests)
    pub(crate) server_id: AtomicU32,
    /// (tests) act as if the default device changed / the connection broke
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
    pub(crate) test_change: AtomicBool,
}

impl Core {
    pub(crate) fn want(&self) -> u8 {
        self.want.load(Ordering::Acquire)
    }

    /// stop() asked to empty the device's buffer too (taken once, by the platform)
    pub(crate) fn take_flush_device(&self) -> bool {
        self.flush_dev.swap(false, Ordering::AcqRel)
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))] // the browser names no device
    pub(crate) fn set_device(&self, name: &str) {
        let mut d = self.device.lock();
        if d.as_str() != name {
            d.clear();
            d.push_str(name);
        }
    }
}

type FillFn = Box<dyn FnMut(&mut [f32]) -> bool + Send>;

// where a stream's frames come from
struct Source {
    core: Arc<Core>,
    fill: Option<FillFn>,
    had: bool,
    filled_out: bool,
}

impl Source {
    // buf: whole frames, zeros past the real ones -> real frames
    fn pull(&mut self, buf: &mut [f32]) -> usize {
        let ch = self.core.channels as usize;
        let frames = buf.len() / ch;
        if let Some(f) = &mut self.fill {
            buf.fill(0.0);
            if self.filled_out {
                return 0;
            }
            if !f(buf) {
                self.filled_out = true;
                self.core.finish.store(true, Ordering::Release);
            }
            return frames;
        }
        let Some(ring) = &self.core.ring else {
            buf.fill(0.0);
            return 0;
        };
        let n = ring.pop(buf) / ch;
        buf[n * ch..].fill(0.0);
        if n < frames && self.had && !self.core.finish.load(Ordering::Acquire) {
            self.core.underruns.fetch_add(1, Ordering::Relaxed);
        }
        self.had = n == frames;
        n
    }

    fn exhausted(&self) -> bool {
        if self.fill.is_some() {
            return self.filled_out;
        }
        self.core.finish.load(Ordering::Acquire) && self.core.ring.as_ref().is_none_or(|r| r.len() == 0)
    }
}

/// the platform side of a stream: renders the frames the device asks for
pub(crate) struct Feed {
    pub(crate) core: Arc<Core>,
    src: Source,
    rs: Option<Resampler>,
    rs_rate: u32,
    vol: f32,
    done: bool,
}

impl Feed {
    fn new(core: Arc<Core>, fill: Option<FillFn>) -> Feed {
        let src = Source { core: core.clone(), fill, had: false, filled_out: false };
        Feed { core, src, rs: None, rs_rate: 0, vol: 1.0, done: false }
    }

    /// out: whole frames of the stream's channels at the device's rate (0: the stream's) ->
    /// false: this was the last buffer (play it, drain the device, then set Core::ended)
    pub(crate) fn render(&mut self, out: &mut [f32], dev_rate: u32) -> bool {
        let core = self.core.clone();
        let ch = core.channels as usize;
        self.apply_flush();
        if core.rearm.swap(false, Ordering::AcqRel) {
            self.done = false;
            self.src.filled_out = false;
            self.src.had = false;
        }
        out.fill(0.0);
        if self.done {
            return false;
        }
        if core.want() != W_PLAY {
            return true;
        }
        let real = if dev_rate == 0 || dev_rate == core.rate {
            self.src.pull(out)
        } else {
            if self.rs_rate != dev_rate {
                self.rs = Some(Resampler::new(core.rate, dev_rate, ch));
                self.rs_rate = dev_rate;
            }
            let src = &mut self.src;
            self.rs.as_mut().map_or(0, |rs| rs.process(out, &mut |b| src.pull(b)))
        };
        core.rendered.fetch_add(real as u64, Ordering::Relaxed);
        // the volume, ramped over this buffer when it changed
        let target = f32::from_bits(core.volume.load(Ordering::Relaxed));
        let frames = out.len() / ch;
        if self.vol != target && frames > 0 {
            let d = (target - self.vol) / frames as f32;
            for (i, f) in out.chunks_exact_mut(ch).enumerate() {
                let g = self.vol + d * (i + 1) as f32;
                f.iter_mut().for_each(|x| *x *= g);
            }
            self.vol = target;
        } else if target != 1.0 {
            out.iter_mut().for_each(|x| *x *= target);
        }
        if real == 0 && self.src.exhausted() {
            self.done = true;
        }
        !self.done
    }

    /// stop() was called: drop what was queued before it (the consumer's side; render does it
    /// too)
    pub(crate) fn apply_flush(&mut self) {
        let core = &self.core;
        if core.flush.swap(false, Ordering::AcqRel) {
            if let Some(r) = &core.ring {
                r.drop_until(core.flush_mark.load(Ordering::Acquire));
            }
            if let Some(rs) = &mut self.rs {
                rs.reset();
            }
        }
    }

    /// whether the stream wants to play now (not paused / stopped, not at its end)
    #[cfg_attr(target_os = "android", allow(dead_code))] // AAudio asks by calling back
    pub(crate) fn wants_device(&self) -> bool {
        self.core.want() == W_PLAY && !self.core.ended.load(Ordering::Acquire)
    }
}

/// an output stream
pub struct Stream {
    core: Arc<Core>,
    dev: Option<native::Dev>,
}

impl Stream {
    /// a stream the app writes into (starts playing at once: silence until data arrives)
    pub fn open(cfg: &Config) -> Result<Stream, AudioError> {
        Stream::make(cfg, None)
    }

    /// a stream that calls fill to top up the device's buffer (on the platform's audio thread
    /// or the browser worker's pump; the buffer comes zeroed); fill returns false after its
    /// last buffer
    pub fn open_fill<F: FnMut(&mut [f32]) -> bool + Send + 'static>(cfg: &Config, fill: F) -> Result<Stream, AudioError> {
        Stream::make(cfg, Some(Box::new(fill)))
    }

    fn make(cfg: &Config, fill: Option<FillFn>) -> Result<Stream, AudioError> {
        if !(8000..=192_000).contains(&cfg.rate) || !(1..=8).contains(&cfg.channels) {
            return Err(AudioError::Invalid);
        }
        let lat = cfg.latency_ms.clamp(10, 1000);
        let push = fill.is_none();
        let dev_ms = if push { (lat / 2).clamp(10, 100) } else { lat.min(200) };
        let ring_frames = if push { ((cfg.rate as u64 * lat.saturating_sub(dev_ms).max(10) as u64) / 1000) as usize } else { 0 };
        let ch = cfg.channels as usize;
        let core = Arc::new(Core {
            rate: cfg.rate,
            channels: cfg.channels,
            dev_ms,
            ring: push.then(|| Ring::new(ring_frames * ch * 2)),
            ring_frames,
            want: AtomicU8::new(W_PLAY),
            dev: AtomicU8::new(D_STARTING),
            finish: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            flush_mark: AtomicUsize::new(0),
            flush_dev: AtomicBool::new(false),
            rearm: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            rendered: AtomicU64::new(0),
            dev_queued: AtomicU32::new(0),
            dev_extra_us: AtomicU32::new(0),
            underruns: AtomicU32::new(0),
            device: SpinLock::new(String::new()),
            changes: AtomicU32::new(0),
            ended: AtomicBool::new(false),
            server_id: AtomicU32::new(u32::MAX),
            test_change: AtomicBool::new(false),
        });
        let name: String = if cfg.name.is_empty() { crate::vfs::app_id() } else { cfg.name.chars().filter(|&c| c != '\0').take(128).collect() };
        let dev = native::open(Feed::new(core.clone(), fill), &name)?;
        Ok(Stream { core, dev: Some(dev) })
    }

    /// frames per second
    pub fn rate(&self) -> u32 {
        self.core.rate
    }

    pub fn channels(&self) -> u16 {
        self.core.channels
    }

    /// interleaved samples into the ring -> whole frames taken (at most writable(); 0 for a
    /// fill stream)
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let ch = self.core.channels as usize;
        let n = (samples.len() / ch).min(self.writable());
        self.push(n, |i| samples[i])
    }

    /// the same for 16-bit samples
    pub fn write_i16(&mut self, samples: &[i16]) -> usize {
        let ch = self.core.channels as usize;
        let n = (samples.len() / ch).min(self.writable());
        self.push(n, |i| samples[i] as f32 / 32768.0)
    }

    fn push(&mut self, frames: usize, f: impl Fn(usize) -> f32) -> usize {
        let Some(ring) = &self.core.ring else { return 0 };
        let ch = self.core.channels as usize;
        if frames > 0 && (self.core.ended.load(Ordering::Acquire) || self.core.finish.load(Ordering::Acquire)) {
            // writing after the end plays again
            self.core.finish.store(false, Ordering::Release);
            self.rearm();
        }
        let n = ring.push_with(frames * ch, f) / ch;
        if n > 0 {
            self.kick();
        }
        n
    }

    /// frames write() takes now
    pub fn writable(&self) -> usize {
        self.core.ring.as_ref().map_or(0, |_| self.core.ring_frames.saturating_sub(self.queued()))
    }

    /// frames waiting in the ring
    pub fn queued(&self) -> usize {
        self.core.ring.as_ref().map_or(0, |r| r.len() / self.core.channels as usize)
    }

    /// play (again): after pause(), stop() or the end
    pub fn start(&self) {
        if self.core.ended.load(Ordering::Acquire) {
            self.core.finish.store(false, Ordering::Release);
            self.rearm();
        }
        self.core.want.store(W_PLAY, Ordering::Release);
        self.kick();
    }

    /// hold: the queued frames stay
    pub fn pause(&self) {
        self.core.want.store(W_PAUSE, Ordering::Release);
        self.kick();
    }

    /// stop and drop what is queued (ring and device buffer)
    pub fn stop(&self) {
        self.core.want.store(W_STOP, Ordering::Release);
        if let Some(r) = &self.core.ring {
            self.core.flush_mark.store(r.mark(), Ordering::Release);
        }
        self.core.flush.store(true, Ordering::Release);
        self.core.flush_dev.store(true, Ordering::Release);
        self.kick();
    }

    /// no more data: the stream ends (State::Ended) once the ring is played
    pub fn finish(&self) {
        self.core.finish.store(true, Ordering::Release);
        self.kick();
    }

    fn rearm(&self) {
        self.core.ended.store(false, Ordering::Release);
        self.core.rearm.store(true, Ordering::Release);
    }

    fn kick(&self) {
        if let Some(d) = &self.dev {
            native::kick(d);
        }
    }

    /// 0 silent ..= 1 as written (applied here, ramped over one buffer)
    pub fn set_volume(&self, v: f32) {
        let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        self.core.volume.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.core.volume.load(Ordering::Relaxed))
    }

    pub fn state(&self) -> State {
        let c = &self.core;
        if c.ended.load(Ordering::Acquire) {
            return State::Ended;
        }
        match c.dev.load(Ordering::Acquire) {
            D_STARTING => State::Starting,
            D_BLOCKED => State::Blocked,
            D_LOST => State::Lost,
            _ => match c.want() {
                W_PLAY => State::Playing,
                W_PAUSE => State::Paused,
                _ => State::Stopped,
            },
        }
    }

    /// what is written now is heard after about this long (ring, device buffer, the device's
    /// own latency; ms)
    pub fn latency_ms(&self) -> f32 {
        let c = &self.core;
        let frames = self.queued() as f32 + c.dev_queued.load(Ordering::Relaxed) as f32;
        frames * 1000.0 / c.rate as f32 + c.dev_extra_us.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// frames played so far (handed to the device minus its buffer)
    pub fn played(&self) -> u64 {
        let c = &self.core;
        c.rendered.load(Ordering::Relaxed).saturating_sub(c.dev_queued.load(Ordering::Relaxed) as u64)
    }

    /// how often the ring ran dry while data was coming (and the device's own count where
    /// it reports one)
    pub fn underruns(&self) -> u32 {
        self.core.underruns.load(Ordering::Relaxed)
    }

    /// the output device's name ("" not known yet)
    pub fn device(&self) -> String {
        self.core.device.lock().clone()
    }

    /// how often the stream followed a new default device (or the server moved it)
    pub fn device_changes(&self) -> u32 {
        self.core.changes.load(Ordering::Relaxed)
    }

    /// (tests) the server's id of the stream: PulseAudio's sink input index (u32::MAX: none)
    #[doc(hidden)]
    pub fn server_id(&self) -> u32 {
        self.core.server_id.load(Ordering::Relaxed)
    }

    /// (tests) act as if the default device changed (Windows: reopen on the default device)
    /// or the server went away (Linux: the connection is dropped and made anew); the stream
    /// plays on, device_changes() counts it.  Elsewhere nothing happens.
    #[doc(hidden)]
    pub fn test_device_change(&self) {
        self.core.test_change.store(true, Ordering::Release);
        self.kick();
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(d) = self.dev.take() {
            native::close(d);
        }
    }
}

impl core::fmt::Debug for Stream {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Stream({} Hz, {} ch, {:?})", self.core.rate, self.core.channels, self.state())
    }
}

// ---------------------------------------------------------------- sounds

/// decoded PCM: interleaved f32 samples (cheap to clone: shared)
#[derive(Clone)]
pub struct Sound {
    rate: u32,
    channels: u16,
    data: Arc<[f32]>,
}

impl Sound {
    /// interleaved samples (1 ..= 8 channels, 1 kHz ..= 384 kHz)
    pub fn new(rate: u32, channels: u16, samples: Vec<f32>) -> Result<Sound, AudioError> {
        if !(1..=8).contains(&channels) || !(1000..=384_000).contains(&rate) || samples.len() % channels as usize != 0 {
            return Err(AudioError::Invalid);
        }
        Ok(Sound { rate, channels, data: samples.into() })
    }

    /// a RIFF WAVE file (PCM 8 / 16 / 24 / 32 bits, float 32 / 64)
    pub fn wav(bytes: &[u8]) -> Result<Sound, AudioError> {
        let (rate, ch, s) = wav::parse(bytes)?;
        Sound::new(rate, ch, s)
    }

    /// an Ogg Vorbis file (.ogg, .oga)
    pub fn ogg(bytes: &[u8]) -> Result<Sound, AudioError> {
        let (rate, ch, s) = vorbis::decode(bytes)?;
        Sound::new(rate, ch, s).map_err(|_| AudioError::Format)
    }

    /// WAV or Ogg Vorbis, told apart by their first bytes
    pub fn decode(bytes: &[u8]) -> Result<Sound, AudioError> {
        match bytes.get(..4) {
            Some(b"RIFF") => Sound::wav(bytes),
            Some(b"OggS") => Sound::ogg(bytes),
            _ => Err(AudioError::Format),
        }
    }

    /// a sound file through the VFS (app://, data://, a path, ..)
    pub fn load(path: &str) -> Result<Sound, AudioError> {
        Sound::decode(&crate::read_file(path).ok_or(AudioError::Io)?)
    }

    /// a sine of freq Hz, ms long, 48 kHz mono, at half scale with 5 ms fades
    pub fn tone(freq: f32, ms: u32) -> Sound {
        Sound { rate: 48_000, channels: 1, data: dsp::tone(freq, ms.min(60_000), 48_000, 0.5).into() }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.data.len() / self.channels as usize
    }

    pub fn samples(&self) -> &[f32] {
        &self.data
    }

    pub fn duration_ms(&self) -> u32 {
        (self.frames() as u64 * 1000 / self.rate as u64) as u32
    }

    /// as a 16-bit PCM WAVE file
    pub fn to_wav(&self) -> Vec<u8> {
        wav::write16(self.rate, self.channels, &self.data)
    }
}

impl core::fmt::Debug for Sound {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "Sound({} Hz, {} ch, {} frames)", self.rate, self.channels, self.frames())
    }
}

// a playing sound's controls, shared with the mixer
struct VoiceCtl {
    stop: AtomicBool,
    done: AtomicBool,
    vol: AtomicU32,
}

/// a sound play() started; dropping the handle lets it play on
pub struct Voice {
    ctl: Arc<VoiceCtl>,
}

impl Voice {
    /// end it now
    pub fn stop(&self) {
        self.ctl.stop.store(true, Ordering::Release);
    }

    /// still playing (or waiting for the device)
    pub fn playing(&self) -> bool {
        !self.ctl.done.load(Ordering::Acquire)
    }

    /// 0 ..= 1
    pub fn set_volume(&self, v: f32) {
        self.ctl.vol.store(if v.is_nan() { 0.0f32 } else { v.clamp(0.0, 1.0) }.to_bits(), Ordering::Relaxed);
    }
}

const MIX_RATE: u32 = 48_000;
const MIX_IDLE_MS: usize = 2000;

// a sound in the mixer
struct Playing {
    sound: Sound,
    pos: usize,
    rs: Option<Resampler>,
    ctl: Arc<VoiceCtl>,
    tmp: Vec<f32>,
}

impl Playing {
    // add frames of this sound (stereo out) -> false: it ended
    fn mix(&mut self, out: &mut [f32]) -> bool {
        if self.ctl.stop.load(Ordering::Acquire) {
            return false;
        }
        let vch = self.sound.channels as usize;
        let frames = out.len() / 2;
        self.tmp.clear();
        self.tmp.resize(frames * vch, 0.0);
        let Playing { sound, pos, rs, tmp, .. } = self;
        let data = &sound.data;
        let total = data.len() / vch;
        let mut take = |b: &mut [f32]| {
            let n = (b.len() / vch).min(total - *pos);
            b[..n * vch].copy_from_slice(&data[*pos * vch..(*pos + n) * vch]);
            b[n * vch..].fill(0.0);
            *pos += n;
            n
        };
        let real = match rs {
            Some(rs) => rs.process(tmp, &mut take),
            None => take(tmp),
        };
        let g = f32::from_bits(self.ctl.vol.load(Ordering::Relaxed));
        for (o, f) in out.chunks_exact_mut(2).zip(self.tmp.chunks_exact(vch)) {
            let (l, r) = if vch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
            o[0] += l * g;
            o[1] += r * g;
        }
        real > 0 || self.pos < total
    }
}

struct MixShared {
    pending: SpinLock<Vec<Playing>>,
    ending: AtomicBool,
}

struct Mixer {
    stream: Stream,
    shared: Arc<MixShared>,
}

static MIXER: SpinLock<Option<Mixer>> = SpinLock::new(None);

// the mixer's fill: the voices of play() added up; false after MIX_IDLE_MS of nothing
fn mix_fill(shared: Arc<MixShared>) -> impl FnMut(&mut [f32]) -> bool + Send {
    let mut voices: Vec<Playing> = Vec::new();
    let mut idle = 0usize;
    move |out: &mut [f32]| {
        {
            let mut p = shared.pending.lock();
            voices.append(&mut p);
            if voices.is_empty() && idle >= MIX_RATE as usize * MIX_IDLE_MS / 1000 {
                shared.ending.store(true, Ordering::Release);
                return false;
            }
        }
        voices.retain_mut(|v| {
            let more = v.mix(out);
            if !more {
                v.ctl.done.store(true, Ordering::Release);
            }
            more
        });
        for x in out.iter_mut() {
            *x = x.clamp(-1.0, 1.0);
        }
        idle = if voices.is_empty() { idle + out.len() / 2 } else { 0 };
        true
    }
}

/// play a sound once, mixed with the other sounds of play() (dropping the Voice lets it play
/// on) -> Err(Unavailable) without an output
pub fn play(s: &Sound) -> Result<Voice, AudioError> {
    let ctl = Arc::new(VoiceCtl { stop: AtomicBool::new(false), done: AtomicBool::new(false), vol: AtomicU32::new(1.0f32.to_bits()) });
    let rs = (s.rate != MIX_RATE).then(|| Resampler::new(s.rate, MIX_RATE, s.channels as usize));
    let mut p = Some(Playing { sound: s.clone(), pos: 0, rs, ctl: ctl.clone(), tmp: Vec::new() });
    let mut m = MIXER.lock();
    if let Some(mx) = &*m {
        if mx.stream.state() == State::Lost {
            return Err(AudioError::Unavailable); // no device now: no sound later
        }
        let mut pend = mx.shared.pending.lock();
        if !mx.shared.ending.load(Ordering::Acquire) {
            pend.extend(p.take());
            drop(pend);
            mx.stream.kick();
            return Ok(Voice { ctl });
        }
    }
    // none yet, or the last one ended: a new stream (the old one closes first)
    drop(m.take());
    let shared = Arc::new(MixShared { pending: SpinLock::new(Vec::new()), ending: AtomicBool::new(false) });
    if let Some(p) = p {
        shared.pending.lock().push(p);
    }
    let cfg = Config { rate: MIX_RATE, channels: 2, latency_ms: 40, name: "" };
    let stream = Stream::open_fill(&cfg, mix_fill(shared.clone()))?;
    *m = Some(Mixer { stream, shared });
    Ok(Voice { ctl })
}

/// play a WAV or Ogg Vorbis file once (see play)
pub fn play_file(path: &str) -> Result<Voice, AudioError> {
    play(&Sound::load(path)?)
}

// ---------------------------------------------------------------- system sounds

static DRY: AtomicBool = AtomicBool::new(false);

/// true: sound() only prints "sound: NAME" (headless runs; also jfgui_AUDIO_DRYRUN=1)
pub fn set_dry_run(on: bool) {
    DRY.store(on, Ordering::Relaxed);
}

fn dry_run() -> bool {
    DRY.load(Ordering::Relaxed) || crate::env("jfgui_AUDIO_DRYRUN").is_some_and(|v| v.starts_with('1'))
}

/// the name of a SOUND_* kind ("default", "information", "warning", "error", "question",
/// "notify"; "" for others)
pub fn sound_name(kind: u32) -> &'static str {
    match kind {
        SOUND_DEFAULT => "default",
        SOUND_INFO => "information",
        SOUND_WARNING => "warning",
        SOUND_ERROR => "error",
        SOUND_QUESTION => "question",
        SOUND_NOTIFY => "notify",
        _ => "",
    }
}

/// the system's sound for an event (SOUND_*) -> false: nothing played (unknown kind, sounds
/// switched off by the system, no output)
pub fn sound(kind: u32) -> bool {
    if kind > SOUND_NOTIFY {
        return false;
    }
    if dry_run() {
        crate::write_stdout(alloc::format!("sound: {}\n", sound_name(kind)).as_bytes());
        return true;
    }
    match native::system_sound(kind) {
        Some(played) => played,
        None => play(&tone_sound(kind)).is_ok(),
    }
}

/// whether message dialogs play the system's sound when they open (Windows like MessageBox;
/// Linux desktops while event sounds are on; not Android, not the browser)
pub fn dialog_sounds() -> bool {
    native::dialog_sounds()
}

/// an output device / sound service exists here (a quick probe)
pub fn available() -> bool {
    native::available()
}

// the generated sound of a kind (browser; Linux without a sound theme)
pub(crate) fn tone_sound(kind: u32) -> Sound {
    let parts: &[(f32, u32)] = match kind {
        SOUND_INFO => &[(660.0, 70), (880.0, 90)],
        SOUND_WARNING => &[(740.0, 160)],
        SOUND_ERROR => &[(330.0, 120), (262.0, 160)],
        SOUND_QUESTION => &[(880.0, 70), (660.0, 90)],
        SOUND_NOTIFY => &[(1047.0, 80), (1319.0, 110)],
        _ => &[(880.0, 90)],
    };
    let mut v = Vec::new();
    for &(f, ms) in parts {
        v.extend_from_slice(&dsp::tone(f, ms, MIX_RATE, 0.3));
    }
    Sound { rate: MIX_RATE, channels: 1, data: v.into() }
}

/// (tests) the amplitude of the sine at freq in channel ch of interleaved frames (Goertzel,
/// Hann window: a full-scale sine reads 1.0)
#[doc(hidden)]
pub fn level(samples: &[f32], channels: usize, ch: usize, rate: u32, freq: f32) -> f32 {
    dsp::level(samples, channels, ch, rate, freq)
}

/// (tests) samples converted from one rate to another by the streams' resampler
#[doc(hidden)]
pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    let ch = channels.max(1);
    let mut rs = Resampler::new(from, to, ch);
    let frames = (samples.len() / ch) as u64 * to as u64 / from.max(1) as u64;
    let mut out = alloc::vec![0.0f32; frames as usize * ch];
    let mut pos = 0;
    rs.process(&mut out, &mut |b: &mut [f32]| {
        let n = (b.len()).min(samples.len() - pos);
        b[..n].copy_from_slice(&samples[pos..pos + n]);
        b[n..].fill(0.0);
        pos += n;
        n / ch
    });
    out
}

/// (tests) the ring of a push stream: n samples written in pieces of `piece` and read in
/// pieces of `take`, by turns or (threaded, Windows / Linux) by a writer thread while this one
/// reads -> whether every sample arrived once and in order
#[doc(hidden)]
pub fn ring_check(n: usize, piece: usize, take: usize, threaded: bool) -> bool {
    let ring = Arc::new(Ring::new(1000));
    let piece = piece.max(1);
    #[cfg(any(windows, target_os = "linux"))]
    let writer = if threaded {
        let r = ring.clone();
        crate::thread_spawn(64 * 1024, Box::new(move || {
            let mut next = 0usize;
            while next < n {
                let (k, start) = (piece.min(n - next), next);
                let put = r.push_with(k, |i| (start + i) as f32);
                next += put;
                if put == 0 {
                    crate::thread_yield();
                }
            }
        }))
    } else {
        None
    };
    #[cfg(not(any(windows, target_os = "linux")))]
    let writer: Option<()> = {
        let _ = threaded;
        None
    };
    let mut got: Vec<f32> = Vec::with_capacity(n);
    let mut next = 0usize;
    let mut buf = alloc::vec![0.0f32; take.max(1)];
    let start = crate::time_us();
    while got.len() < n && crate::time_us() - start < 20_000_000 {
        if writer.is_none() && next < n {
            let (k, at) = (piece.min(n - next), next);
            next += ring.push_with(k, |i| (at + i) as f32);
        }
        let m = ring.pop(&mut buf);
        got.extend_from_slice(&buf[..m]);
    }
    #[cfg(any(windows, target_os = "linux"))]
    if let Some(w) = writer {
        w.join();
    }
    ring.len() == 0 && got.len() == n && got.iter().enumerate().all(|(i, &v)| v == i as f32)
}

#[cfg(windows)]
#[path = "win_audio.rs"]
mod native;
#[cfg(windows)]
pub use native::Loopback;

#[cfg(target_os = "linux")]
#[path = "linux_audio.rs"]
mod native;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use native::{ServerStream, pulse_selftest, server_info, server_streams};

/// (tests, Linux) the sound theme's file for a SOUND_* kind (".disabled": switched off)
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub fn theme_path(kind: u32) -> Option<String> {
    native::theme_path(kind)
}

#[cfg(target_arch = "wasm32")]
#[path = "web_audio.rs"]
mod native;

#[cfg(target_os = "android")]
#[path = "android_audio.rs"]
mod native;

#[cfg(not(any(windows, target_os = "linux", target_arch = "wasm32", target_os = "android")))]
mod native {
    use super::{AudioError, Feed};
    pub(crate) struct Dev;
    pub(crate) fn open(_feed: Feed, _name: &str) -> Result<Dev, AudioError> {
        Err(AudioError::Unavailable)
    }
    pub(crate) fn kick(_d: &Dev) {}
    pub(crate) fn close(_d: Dev) {}
    pub(crate) fn available() -> bool {
        false
    }
    pub(crate) fn system_sound(_kind: u32) -> Option<bool> {
        Some(false)
    }
    pub(crate) fn dialog_sounds() -> bool {
        false
    }
}
