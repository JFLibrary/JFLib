// linux_audio -- (audio.rs) sound through a PulseAudio server (pipewire-pulse on current
// desktops, WSLg's server), one connection and thread per stream; the freedesktop sound theme
//
//   open(feed, name) -> Dev   the thread connects (linux_pulse.rs), creates a playback stream
//                             (float32le, the stream's rate and channels, tlength =
//                             Core::dev_ms, ADJUST_LATENCY, no device named: the server's
//                             default -- the server moves the stream when the default changes)
//                             and answers the server's REQUESTs from the Feed; open waits for
//                             that first answer (no server: Unavailable)
//   the loop                  poll on the socket and an eventfd (kicks): pause = CORK, start =
//                             uncork, stop() = cork + FLUSH, the end = DRAIN (its reply: Ended,
//                             corked again); GET_PLAYBACK_LATENCY every 250 ms while playing
//                             (the sink's latency + what the server holds); PLAYBACK_STREAM_MOVED:
//                             the new device's name, device_changes(); UNDERFLOW counts; a killed
//                             stream or a lost connection: State::Lost, reconnected every second
//   system_sound(kind)        org.gnome.desktop.sound from the settings portal: event-sounds off
//                             -> nothing; theme-name (else freedesktop) with its Inherits=, the
//                             names dialog-information / -warning / -error / -question, bell,
//                             message-new-instant (and fallbacks) as .disabled / .oga / .ogg /
//                             .wav in the theme's Directories= (else stereo) below
//                             $XDG_DATA_HOME/sounds and $XDG_DATA_DIRS/sounds; decoded once,
//                             played through play(); no file: None (audio.rs plays a tone)
//                             (jfgui_SOUND_THEME, jfgui_EVENT_SOUNDS=0|1 override the portal)
//
// ALSA (/dev/snd) is not used: without a sound server there is no output (Unavailable).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use rt::os::syscall6;
use rt::sync::SpinLock;

use super::{AudioError, D_LOST, D_RUNNING, Feed, SOUND_DEFAULT, SOUND_ERROR, SOUND_INFO, SOUND_NOTIFY, SOUND_QUESTION, SOUND_WARNING, Sound};
use crate::{Sem, Thread, thread_spawn};

#[path = "linux_pulse.rs"]
pub(crate) mod pulse;
use pulse::{Conn, Packet};

const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_POLL: usize = 7;
const SYS_EVENTFD2: usize = 290;

struct Ctl {
    /// kicks: state changes, closing
    efd: i32,
    closing: AtomicBool,
    /// the first open: 1 pending, 0 done, -2 no server, else the server's error
    result: AtomicI32,
    ready: Sem,
}

impl Drop for Ctl {
    fn drop(&mut self) {
        // SAFETY: our eventfd, closed once (after the thread ended).
        unsafe { syscall6(SYS_CLOSE, self.efd as usize, 0, 0, 0, 0, 0) };
    }
}

impl Ctl {
    fn kick(&self) {
        let one = 1u64;
        // SAFETY: 8 bytes into our eventfd.
        unsafe { syscall6(SYS_WRITE, self.efd as usize, &one as *const u64 as usize, 8, 0, 0, 0) };
    }

    fn take_kicks(&self) {
        let mut v = 0u64;
        // SAFETY: 8 bytes out of our (non-blocking) eventfd.
        unsafe { syscall6(SYS_READ, self.efd as usize, &mut v as *mut u64 as usize, 8, 0, 0, 0) };
    }
}

/// a running stream: its thread
pub(crate) struct Dev {
    ctl: Arc<Ctl>,
    thread: Option<Thread>,
}

const NO_SERVER: i32 = -2;

pub(crate) fn open(feed: Feed, name: &str) -> Result<Dev, AudioError> {
    // SAFETY: an eventfd, close-on-exec and non-blocking.
    let efd = unsafe { syscall6(SYS_EVENTFD2, 0, 0x80000 | 0x800, 0, 0, 0, 0) };
    if efd < 0 {
        return Err(AudioError::Failed(efd as i32));
    }
    let ready = Sem::new(0).ok_or(AudioError::Failed(-1))?;
    let ctl = Arc::new(Ctl { efd: efd as i32, closing: AtomicBool::new(false), result: AtomicI32::new(1), ready });
    let (c, name) = (ctl.clone(), String::from(name));
    let thread = thread_spawn(256 * 1024, Box::new(move || run(feed, c, name))).ok_or(AudioError::Failed(-1))?;
    ctl.ready.wait(10_000);
    match ctl.result.load(Ordering::Acquire) {
        0 => Ok(Dev { ctl, thread: Some(thread) }),
        r => {
            ctl.closing.store(true, Ordering::Release);
            ctl.kick();
            thread.join();
            Err(match r {
                NO_SERVER => AudioError::Unavailable,
                1 => AudioError::Failed(-1),
                e => AudioError::Failed(e),
            })
        }
    }
}

pub(crate) fn kick(d: &Dev) {
    d.ctl.kick();
}

pub(crate) fn close(mut d: Dev) {
    d.ctl.closing.store(true, Ordering::Release);
    d.ctl.kick();
    if let Some(t) = d.thread.take() {
        t.join();
    }
}

/// a playback stream on its connection
struct PStream {
    conn: Conn,
    channel: u32,
    /// bytes the server asked for and has not got
    requested: usize,
    corked: bool,
    drain_tag: Option<u32>,
    lat_tag: Option<u32>,
    next_lat: u64,
    killed: bool,
}

impl PStream {
    fn create(feed: &Feed, name: &str) -> Result<PStream, i32> {
        let core = &feed.core;
        let app = crate::vfs::app_id();
        let mut conn = Conn::open(&app).ok_or(NO_SERVER)?;
        let ch = core.channels;
        let bpf = 4 * ch as u32;
        let tlength = (core.rate as u64 * core.dev_ms as u64 / 1000) as u32 * bpf;
        let corked = !feed.wants_device();
        let v = conn.version;
        let positions = pulse::channel_positions(ch);
        let volume = vec![pulse::VOLUME_NORM; ch as usize];
        let rep = conn.call(pulse::CMD_CREATE_PLAYBACK_STREAM, |t| {
            t.sample_spec(pulse::FORMAT_F32LE, ch as u8, core.rate).channel_map(&positions);
            // no sink named: the default one; maxlength / prebuf / minreq: the server's
            t.u32(u32::MAX).str(None).u32(u32::MAX).bool(corked);
            t.u32(tlength).u32(u32::MAX).u32(u32::MAX).u32(0).cvolume(&volume);
            // no remap, no remix, fix format / rate / channels, don't move, variable rate
            t.bool(false).bool(false).bool(false).bool(false).bool(false).bool(false).bool(false);
            // start muted, adjust latency, the properties
            t.bool(false).bool(true).proplist(&[("media.name", name), ("application.name", &app)]);
            if v >= 14 {
                t.bool(false).bool(false); // volume set, early requests
            }
            if v >= 15 {
                t.bool(false).bool(false).bool(false); // muted set, don't inhibit suspend, fail on suspend
            }
            if v >= 17 {
                t.bool(false); // relative volume
            }
            if v >= 18 {
                t.bool(false); // passthrough
            }
            if v >= 21 {
                t.u8(0); // no formats: the sample spec
            }
        })?;
        let mut r = rep.args();
        let channel = r.u32().ok_or(-1)?;
        let index = r.u32().ok_or(-1)?;
        let requested = r.u32().ok_or(-1)? as usize;
        // maxlength, tlength, prebuf, minreq; spec, map, device index and name, suspended
        let _ = (r.u32(), r.u32(), r.u32(), r.u32(), r.sample_spec(), r.channel_map(), r.u32());
        if let Some(Some(dev)) = r.str() {
            core.set_device(&dev);
        }
        let _ = r.bool();
        if let Some(lat) = r.usec() {
            core.dev_extra_us.store(lat.min(u32::MAX as u64) as u32, Ordering::Relaxed);
        }
        core.server_id.store(index, Ordering::Relaxed);
        Ok(PStream { conn, channel, requested, corked, drain_tag: None, lat_tag: None, next_lat: 0, killed: false })
    }

    fn cork(&mut self, on: bool) {
        let ch = self.channel;
        self.conn.send(pulse::CMD_CORK_PLAYBACK_STREAM, |t| {
            t.u32(ch).bool(on);
        });
        self.corked = on;
    }

    fn simple(&mut self, cmd: u32) -> Option<u32> {
        let ch = self.channel;
        self.conn.send(cmd, |t| {
            t.u32(ch);
        })
    }

    /// the next packet there (also those that came while waiting for a reply)
    fn next(&mut self) -> Option<Packet> {
        if !self.conn.pending.is_empty() {
            return Some(self.conn.pending.remove(0));
        }
        self.conn.read(0)
    }
}

// the stream's thread
fn run(mut feed: Feed, ctl: Arc<Ctl>, name: String) {
    let core = feed.core.clone();
    let mut ps = match PStream::create(&feed, &name) {
        Ok(p) => {
            ctl.result.store(0, Ordering::Release);
            core.dev.store(D_RUNNING, Ordering::Release);
            ctl.ready.post(1);
            Some(p)
        }
        Err(e) => {
            ctl.result.store(e, Ordering::Release);
            ctl.ready.post(1);
            return;
        }
    };
    let bpf = 4 * core.channels as usize;
    let mut more = true;
    let mut buf: Vec<f32> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut retry_at = 0u64;
    loop {
        ctl.take_kicks();
        if ctl.closing.load(Ordering::Acquire) {
            if let Some(p) = ps.as_mut() {
                p.simple(pulse::CMD_DELETE_PLAYBACK_STREAM);
            }
            break;
        }
        let Some(p) = ps.as_mut() else {
            // lost: a new connection and stream every second
            if crate::time_us() >= retry_at {
                if let Ok(n) = PStream::create(&feed, &name) {
                    ps = Some(n);
                    core.changes.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    core.dev.store(D_RUNNING, Ordering::Release);
                    more = true;
                    continue;
                }
                retry_at = crate::time_us() + 1_000_000;
            }
            wait(&[ctl.efd], 1000);
            continue;
        };
        // what the server said
        while let Some(pk) = p.next() {
            let Some((cmd, tag, mut r)) = pk.command() else { continue };
            match cmd {
                pulse::CMD_REQUEST => {
                    if r.u32() == Some(p.channel) {
                        p.requested += r.u32().unwrap_or(0) as usize;
                    }
                }
                pulse::CMD_REPLY | pulse::CMD_ERROR if Some(tag) == p.drain_tag => {
                    // everything played: the end, corked until start()
                    p.drain_tag = None;
                    core.ended.store(true, Ordering::Release);
                    core.dev_queued.store(0, Ordering::Relaxed);
                    p.cork(true);
                    more = true;
                }
                pulse::CMD_REPLY if Some(tag) == p.lat_tag => {
                    p.lat_tag = None;
                    let sink = r.usec().unwrap_or(0);
                    let _ = (r.usec(), r.bool(), r.timeval(), r.timeval());
                    if let (Some(w), Some(rd)) = (r.s64(), r.s64()) {
                        let held = (w - rd).max(0) as usize / bpf;
                        core.dev_queued.store(held.min(u32::MAX as usize) as u32, Ordering::Relaxed);
                    }
                    core.dev_extra_us.store(sink.min(u32::MAX as u64) as u32, Ordering::Relaxed);
                }
                pulse::CMD_ERROR if Some(tag) == p.lat_tag => p.lat_tag = None,
                pulse::CMD_UNDERFLOW => {
                    if r.u32() == Some(p.channel) && more && !p.corked {
                        core.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
                pulse::CMD_PLAYBACK_STREAM_KILLED => {
                    if r.u32() == Some(p.channel) {
                        p.killed = true;
                    }
                }
                pulse::CMD_PLAYBACK_STREAM_MOVED => {
                    if r.u32() == Some(p.channel) {
                        let _ = r.u32();
                        if let Some(Some(dev)) = r.str() {
                            core.set_device(&dev);
                        }
                        core.changes.fetch_add(1, Ordering::Relaxed);
                    }
                }
                _ => {}
            }
        }
        if core.test_change.swap(false, Ordering::AcqRel) {
            p.conn.broken = true; // (tests) as if the server went away
        }
        if p.conn.broken || p.killed {
            ps = None;
            core.dev.store(D_LOST, Ordering::Release);
            core.dev_queued.store(0, Ordering::Relaxed);
            retry_at = crate::time_us() + 1_000_000;
            continue;
        }
        // what the app wants
        if core.take_flush_device() {
            feed.apply_flush();
            if !p.corked {
                p.cork(true);
            }
            p.simple(pulse::CMD_FLUSH_PLAYBACK_STREAM);
            p.drain_tag = None;
            more = true;
        }
        let want = feed.wants_device();
        if want && p.corked {
            p.cork(false);
        } else if !want && !p.corked && p.drain_tag.is_none() {
            p.cork(true);
        }
        // the data the server asked for
        while !p.corked && more && p.drain_tag.is_none() && p.requested >= bpf {
            let frames = (p.requested / bpf).min(8192);
            buf.clear();
            buf.resize(frames * core.channels as usize, 0.0);
            more = feed.render(&mut buf, 0);
            bytes.clear();
            bytes.extend(buf.iter().flat_map(|x| x.to_le_bytes()));
            if !p.conn.write_data(p.channel, &bytes) {
                break;
            }
            p.requested -= frames * bpf;
            if !more {
                p.drain_tag = p.simple(pulse::CMD_DRAIN_PLAYBACK_STREAM);
            }
        }
        // the latency now and then
        let now = crate::time_us();
        if !p.corked && p.lat_tag.is_none() && now >= p.next_lat {
            let ch = p.channel;
            let ns = crate::unix_time_ns();
            p.lat_tag = p.conn.send(pulse::CMD_GET_PLAYBACK_LATENCY, |t| {
                t.u32(ch).timeval((ns / 1_000_000_000) as u32, ((ns / 1000) % 1_000_000) as u32);
            });
            p.next_lat = now + 250_000;
        }
        let ms = if p.corked && p.drain_tag.is_none() { -1 } else { 250 };
        wait(&[p.conn.fd(), ctl.efd], ms);
    }
}

// poll until one of the descriptors is readable or ms passed (-1: forever)
fn wait(fds: &[i32], ms: i32) {
    let mut pfd: Vec<u64> = fds.iter().map(|&fd| fd as u32 as u64 | (1u64 << 32)).collect();
    // SAFETY: pollfd records (fd, events POLLIN, revents) for our descriptors.
    unsafe { syscall6(SYS_POLL, pfd.as_mut_ptr() as usize, pfd.len(), ms as isize as usize, 0, 0, 0) };
}

pub(crate) fn available() -> bool {
    pulse::reachable()
}

// ---------------------------------------------------------------- system sounds

// a setting of org.gnome.desktop.sound from the settings portal (300 ms at most)
fn portal_sound_setting(key: &str) -> Option<crate::linux_dbus::Value> {
    use crate::linux_dbus::{Conn as Bus, Msg, Value};
    if crate::env("jfgui_PORTAL_SETTINGS").is_some_and(|v| v.starts_with('0')) {
        return None;
    }
    let mut c = Bus::session()?;
    let m = Msg::call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "Read",
        vec![Value::str("org.gnome.desktop.sound"), Value::str(key)],
    );
    let s = c.send(&m)?;
    let mut v = c.wait_reply(s, 300)?.body.into_iter().next()?;
    while let Value::V(inner) = v {
        v = *inner;
    }
    Some(v)
}

struct SoundCfg {
    at: u64,
    on: bool,
    theme: String,
}

static CFG: SpinLock<Option<SoundCfg>> = SpinLock::new(None);
static CACHE: SpinLock<Vec<(String, Option<Sound>)>> = SpinLock::new(Vec::new());

// (event sounds on, theme name): the jfgui_* overrides, else the portal (asked at most
// every 5 s)
fn sound_cfg() -> (bool, String) {
    let valid = |t: &String| !t.is_empty() && !t.contains('/');
    let env_on = crate::env("jfgui_EVENT_SOUNDS").map(|v| !v.starts_with('0'));
    let env_theme = crate::env("jfgui_SOUND_THEME").filter(valid);
    if let (Some(on), Some(theme)) = (env_on, &env_theme) {
        return (on, theme.clone());
    }
    let now = crate::time_us();
    let cached = CFG.lock().as_ref().filter(|c| now < c.at + 5_000_000).map(|c| (c.on, c.theme.clone()));
    let (on, theme) = cached.unwrap_or_else(|| {
        let on = !matches!(portal_sound_setting("event-sounds"), Some(crate::linux_dbus::Value::B(false)));
        let theme = portal_sound_setting("theme-name")
            .and_then(|v| v.as_str().map(String::from))
            .filter(valid)
            .unwrap_or_else(|| String::from("freedesktop"));
        *CFG.lock() = Some(SoundCfg { at: now, on, theme: theme.clone() });
        (on, theme)
    });
    (env_on.unwrap_or(on), env_theme.unwrap_or(theme))
}

// the freedesktop names of a kind, the most specific first
fn names(kind: u32) -> &'static [&'static str] {
    match kind {
        SOUND_INFO => &["dialog-information"],
        SOUND_WARNING => &["dialog-warning"],
        SOUND_ERROR => &["dialog-error", "dialog-warning"],
        SOUND_QUESTION => &["dialog-question", "window-question", "dialog-information"],
        SOUND_NOTIFY => &["message-new-instant", "message"],
        SOUND_DEFAULT => &["bell-window-system", "bell"],
        _ => &[],
    }
}

// the folders sound themes live in
fn sound_dirs() -> Vec<String> {
    let mut out = Vec::new();
    match crate::env("XDG_DATA_HOME") {
        Some(d) if d.starts_with('/') => out.push(alloc::format!("{d}/sounds")),
        _ => {
            if let Some(h) = crate::env("HOME") {
                out.push(alloc::format!("{h}/.local/share/sounds"));
            }
        }
    }
    let dirs = crate::env("XDG_DATA_DIRS").filter(|d| !d.is_empty()).unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
    out.extend(dirs.split(':').filter(|d| d.starts_with('/')).map(|d| alloc::format!("{d}/sounds")));
    out
}

// a theme's index.theme: (Inherits, Directories)
fn theme_index(dirs: &[String], theme: &str) -> (Vec<String>, Vec<String>) {
    for d in dirs {
        let Some(b) = crate::read_file_raw(&alloc::format!("{d}/{theme}/index.theme")) else { continue };
        let text = String::from_utf8_lossy(&b);
        let list = |key: &str| -> Vec<String> {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key).map(|r| r.trim_start()).and_then(|r| r.strip_prefix('=')))
                .map(|v| v.split(',').map(|x| String::from(x.trim())).filter(|x| !x.is_empty() && !x.contains('/')).collect())
                .unwrap_or_default()
        };
        return (list("Inherits"), list("Directories"));
    }
    (Vec::new(), Vec::new())
}

/// the theme file for kind: Some(path) (a ".disabled" path: switched off), None: none
pub(crate) fn theme_path(kind: u32) -> Option<String> {
    let (_, theme) = sound_cfg();
    let dirs = sound_dirs();
    let mut chain = vec![theme];
    let mut i = 0;
    while i < chain.len() && chain.len() < 8 {
        let (inherits, _) = theme_index(&dirs, &chain[i]);
        for t in inherits {
            if !chain.contains(&t) {
                chain.push(t);
            }
        }
        i += 1;
    }
    if !chain.iter().any(|t| t == "freedesktop") {
        chain.push(String::from("freedesktop"));
    }
    for name in names(kind) {
        for t in &chain {
            let (_, mut subdirs) = theme_index(&dirs, t);
            if subdirs.is_empty() {
                subdirs.push(String::from("stereo"));
            }
            for d in &dirs {
                for sub in &subdirs {
                    for ext in ["disabled", "oga", "ogg", "wav"] {
                        let p = alloc::format!("{d}/{t}/{sub}/{name}.{ext}");
                        if crate::file_exists_raw(&p) == 1 {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn system_sound(kind: u32) -> Option<bool> {
    let (on, _) = sound_cfg();
    if !on {
        return Some(false);
    }
    let path = theme_path(kind)?;
    if path.ends_with(".disabled") {
        return Some(false);
    }
    let cached = CACHE.lock().iter().find(|(p, _)| *p == path).map(|(_, s)| s.clone());
    let sound = match cached {
        Some(s) => s,
        None => {
            let s = crate::read_file_raw(&path).and_then(|b| Sound::decode(&b).ok());
            let mut c = CACHE.lock();
            if c.len() >= 16 {
                c.remove(0);
            }
            c.push((path, s.clone()));
            s
        }
    };
    // a file that does not decode: the generated tone
    sound.map(|s| super::play(&s).is_ok())
}

pub(crate) fn dialog_sounds() -> bool {
    sound_cfg().0
}

// ---------------------------------------------------------------- tests

/// (tests) a playback stream as the PulseAudio server lists it
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ServerStream {
    /// the sink input (Stream::server_id)
    pub index: u32,
    pub name: String,
    pub sink: u32,
    pub rate: u32,
    pub channels: u8,
    /// what the server holds and the sink's latency (microseconds)
    pub buffer_us: u64,
    pub sink_us: u64,
    pub corked: bool,
    pub props: Vec<(String, String)>,
}

/// (tests) the server's playback streams (GET_SINK_INPUT_INFO_LIST on a connection of its own)
#[doc(hidden)]
pub fn server_streams() -> Option<Vec<ServerStream>> {
    let mut c = Conn::open("plat-audio-probe")?;
    let v = c.version;
    let rep = c.call(pulse::CMD_GET_SINK_INPUT_INFO_LIST, |_| {}).ok()?;
    let mut r = rep.args();
    let mut out = Vec::new();
    while !r.done() {
        let index = r.u32()?;
        let name = r.str()?.unwrap_or_default();
        let (_module, _client, sink) = (r.u32()?, r.u32()?, r.u32()?);
        let (_, channels, rate) = r.sample_spec()?;
        let (_map, _vol) = (r.channel_map()?, r.cvolume()?);
        let (buffer_us, sink_us) = (r.usec()?, r.usec()?);
        let (_resample, _driver) = (r.str()?, r.str()?);
        let _mute = r.bool()?;
        let props = r.proplist()?;
        let corked = if v >= 19 { r.bool()? } else { false };
        if v >= 20 {
            let _ = (r.bool()?, r.bool()?); // has volume, volume writable
        }
        if v >= 21 {
            r.format_info()?;
        }
        out.push(ServerStream { index, name, sink, rate, channels, buffer_us, sink_us, corked, props });
    }
    Some(out)
}

/// (tests) the server: (its name, its version, the default sink)
#[doc(hidden)]
pub fn server_info() -> Option<(String, String, String)> {
    let mut c = Conn::open("plat-audio-probe")?;
    let rep = c.call(pulse::CMD_GET_SERVER_INFO, |_| {}).ok()?;
    let mut r = rep.args();
    let name = r.str()?.unwrap_or_default();
    let version = r.str()?.unwrap_or_default();
    let _ = (r.str()?, r.str()?, r.sample_spec()?);
    let sink = r.str()?.unwrap_or_default();
    Some((name, version, sink))
}

/// (tests) the wire format against hand-made bytes -> (check, passed)
#[doc(hidden)]
pub fn pulse_selftest() -> Vec<(&'static str, bool)> {
    use pulse::{Rd, Ts};
    let mut out = Vec::new();
    // AUTH: version 32 and a zero cookie, tag 0
    let auth = pulse::command(pulse::CMD_AUTH, 0, |t| {
        t.u32(32).arbitrary(&[0; 256]);
    });
    let mut want = vec![0, 0, 1, 20, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    want.extend_from_slice(&[b'L', 0, 0, 0, 8, b'L', 0, 0, 0, 0, b'L', 0, 0, 0, 32, b'x', 0, 0, 1, 0]);
    want.resize(want.len() + 256, 0);
    out.push(("AUTH packet: descriptor, command, tag, version, cookie", auth == want));
    // a REQUEST from the server, split across two reads
    let mut req = Ts::default();
    req.u32(pulse::CMD_REQUEST).u32(u32::MAX).u32(3).u32(1024);
    let f = pulse::frame(u32::MAX, &req.0);
    let half = pulse::unframe(&f[..23]);
    let whole = pulse::unframe(&f);
    let parsed = whole.as_ref().ok().and_then(|o| o.as_ref()).and_then(|(p, n)| {
        let (cmd, tag, mut r) = p.command()?;
        Some((cmd, tag, r.u32()?, r.u32()?, *n))
    });
    out.push(("REQUEST: a partial packet waits, the whole one parses", matches!(half, Ok(None)) && parsed == Some((61, u32::MAX, 3, 1024, f.len()))));
    // a packet in shared memory (we never offer it) breaks the stream
    let mut shm = pulse::frame(5, &[1, 2, 3, 4]);
    shm[16] = 0x80;
    out.push(("a shared-memory block is refused", pulse::unframe(&shm).is_err()));
    // the reply of CREATE_PLAYBACK_STREAM (version 32), hand-made
    let mut rep = Ts::default();
    rep.u32(pulse::CMD_REPLY).u32(7).u32(3).u32(42).u32(7680);
    rep.u32(u32::MAX).u32(7680).u32(3844).u32(1920);
    rep.sample_spec(pulse::FORMAT_F32LE, 2, 48_000).channel_map(&[1, 2]).u32(0).str(Some("RDPSink")).bool(false);
    rep.0.push(b'U');
    rep.0.extend_from_slice(&20_000u64.to_be_bytes());
    rep.0.extend_from_slice(&[b'f', b'B', 1]);
    rep.proplist(&[]);
    let pk = pulse::Packet { channel: u32::MAX, data: rep.0.clone() };
    let ok = pk.command().and_then(|(cmd, tag, mut r)| {
        let head = (cmd, tag, r.u32()?, r.u32()?, r.u32()?);
        let attrs = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let spec = r.sample_spec()?;
        let map = r.channel_map()?;
        let dev = (r.u32()?, r.str()?, r.bool()?);
        let lat = r.usec()?;
        let fmt = r.format_info()?;
        Some(head == (2, 7, 3, 42, 7680) && attrs == (u32::MAX, 7680, 3844, 1920) && spec == (5, 2, 48_000) && map == [1, 2]
            && dev == (0, Some(String::from("RDPSink")), false) && lat == 20_000 && fmt == (1, Vec::new()) && r.done())
    });
    out.push(("CREATE_PLAYBACK_STREAM reply: channel, index, attributes, device, latency, format", ok == Some(true)));
    // properties: values with their NUL; a wrong tag is refused
    let mut pl = Ts::default();
    pl.proplist(&[("media.name", "Ton"), ("application.name", "plattest")]);
    let back = Rd::new(&pl.0).proplist();
    out.push(("proplist round trip", back == Some(vec![(String::from("media.name"), String::from("Ton")), (String::from("application.name"), String::from("plattest"))])));
    out.push(("a value of another type is refused", Rd::new(&pl.0).u32().is_none() && Rd::new(b"t\x41").str().is_none()));
    out.push(("channel maps in WAVE order", pulse::channel_positions(1) == [0] && pulse::channel_positions(2) == [1, 2] && pulse::channel_positions(6) == [1, 2, 3, 7, 5, 6]));
    // PULSE_SERVER entries
    let saved = crate::env("PULSE_SERVER");
    crate::setenv("PULSE_SERVER", Some("{0123abcd}unix:/tmp/pulse-a tcp:example.org:4713 /run/pulse-b"));
    let paths = pulse::server_paths();
    crate::setenv("PULSE_SERVER", saved.as_deref());
    out.push(("PULSE_SERVER: unix: and plain paths, tcp: skipped", paths == ["/tmp/pulse-a", "/run/pulse-b"]));
    out
}
