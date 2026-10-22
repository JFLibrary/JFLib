// tests -- audiotest --test NAME: plat::audio with the system's real output (PASS / FAIL /
// SKIP lines, the exit code is the number of failures; no output device: SKIP, not FAIL)
//
//   --test play     a push stream (48 kHz stereo, a quiet 1234 Hz tone written as the ring takes
//                   it), finish -> Ended, latency, played frames, underruns, the device's name;
//                   a fill stream (44.1 kHz mono, 777 Hz) that ends itself; pause keeps the
//                   position, start goes on, stop empties the ring; volume; play() and
//                   play_file(app://klang.wav) through the mixer.  Windows: the default device's
//                   mix captured by WASAPI loopback meanwhile must contain both tones (Goertzel
//                   level against a frequency that was not played)
//   --test sounds   the system sounds: dry run lines, sound() of every kind (quiet systems
//                   play nothing: only "not refused" counts), dialog_sounds()
//   --test pulse    (Linux) the PulseAudio server ($PULSE_SERVER, WSLg's /mnt/wslg/PulseServer,
//                   pipewire-pulse): our stream among its sink inputs (index, rate, channels,
//                   media.name), corked while paused, its latency as the server reports it and
//                   as the stream does, gone after close; a system sound from a scratch theme
//                   (WAV) and the generated tone without one; no server: SKIP
//
// The tones are quiet (-26 dBFS) and short; they are audible on the speakers.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::audio::{self, Config, Sound, State, Stream};

pub struct T {
    pub passes: u32,
    pub fails: u32,
}

impl T {
    pub fn check(&mut self, ok: bool, name: &str) {
        if ok {
            self.passes += 1;
            out(&format!("PASS {name}\n"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {name}\n"));
        }
    }

    pub fn skip(&self, why: &str) {
        out(&format!("SKIP {why}\n"));
    }

    pub fn info(&self, s: &str) {
        out(&format!("  {s}\n"));
    }
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// run --test NAME -> the number of failures (2: unknown name)
pub fn run(name: &str) -> i32 {
    let mut t = T { passes: 0, fails: 0 };
    match name {
        "play" => t_play(&mut t),
        "sounds" => t_sounds(&mut t),
        "pulse" => t_pulse(&mut t),
        _ => {
            plat::write_stderr(b"usage: audiotest --test play|sounds|pulse\n");
            return 2;
        }
    }
    out(&format!("audiotest: {} passed, {} failure(s)\n", t.passes, t.fails));
    t.fails as i32
}

const AMP: f32 = 0.05;

/// a sine of freq at AMP, ms long, channels copies per frame
fn sine(freq: f32, rate: u32, ms: u32, channels: usize) -> Vec<f32> {
    let n = (rate as u64 * ms as u64 / 1000) as usize;
    let mut v = Vec::with_capacity(n * channels);
    for i in 0..n {
        let s = (AMP as f64 * rt::x87::sin(2.0 * core::f64::consts::PI * freq as f64 * i as f64 / rate as f64)) as f32;
        v.extend(core::iter::repeat_n(s, channels));
    }
    v
}

/// wait up to ms until f() holds -> whether it did
fn wait_for(ms: u32, mut f: impl FnMut() -> bool) -> bool {
    let end = plat::time_us() + ms as u64 * 1000;
    while plat::time_us() < end {
        if f() {
            return true;
        }
        plat::sleep_ms(5);
    }
    f()
}

/// the highest level of freq in 100 ms windows of channel 0 (Windows: the loopback capture)
#[cfg(windows)]
fn peak_level(data: &[f32], ch: usize, rate: u32, freq: f32) -> f32 {
    let w = (rate / 10) as usize * ch;
    let mut best = 0.0f32;
    let mut at = 0;
    while at + w <= data.len() {
        best = best.max(audio::level(&data[at..at + w], ch, 0, rate, freq));
        at += w / 2;
    }
    best
}

fn t_play(t: &mut T) {
    if !audio::available() {
        t.skip("play: no output device / sound service here");
        return;
    }
    #[cfg(windows)]
    let lb = audio::Loopback::start();
    #[cfg(windows)]
    t.check(lb.is_ok(), "play: WASAPI loopback of the default device started");

    // 1. a push stream: the tone written as the ring takes it
    let cfg = Config { rate: 48_000, channels: 2, latency_ms: 60, name: "audiotest" };
    let mut s = match Stream::open(&cfg) {
        Ok(s) => s,
        Err(e) => {
            t.check(false, &format!("play: open a push stream ({e})"));
            return;
        }
    };
    t.check(matches!(s.state(), State::Playing | State::Starting), &format!("play: a new stream plays ({:?})", s.state()));
    t.check(s.writable() > 0 && s.writable() <= 48 * 60, &format!("play: the ring takes at most the latency ({} frames)", s.writable()));
    let tone = sine(1234.0, 48_000, 700, 2);
    let frames = tone.len() / 2;
    let (mut pos, start) = (0usize, plat::time_us());
    let (mut lat_lo, mut lat_hi) = (f32::MAX, 0.0f32);
    while pos < frames && plat::time_us() - start < 5_000_000 {
        pos += s.write(&tone[pos * 2..]);
        if pos > 4800 {
            let l = s.latency_ms();
            lat_lo = lat_lo.min(l);
            lat_hi = lat_hi.max(l);
        }
        plat::sleep_ms(4);
    }
    t.check(pos == frames, &format!("play: 700 ms of 1234 Hz written as the ring took them ({pos} of {frames} frames)"));
    s.finish();
    let ended = wait_for(3000, || s.state() == State::Ended);
    t.check(ended, &format!("play: finish() -> Ended once the ring is played ({:?})", s.state()));
    let played = s.played();
    t.check(played >= frames as u64 && played < frames as u64 + 48_000, &format!("play: {played} frames played"));
    t.info(&format!("latency {lat_lo:.1} .. {lat_hi:.1} ms, underruns {}, device \"{}\"", s.underruns(), s.device()));
    t.check(lat_hi > 1.0 && lat_hi < 250.0, &format!("play: latency while writing {lat_lo:.1} .. {lat_hi:.1} ms"));
    t.check(s.underruns() <= 2, &format!("play: {} underruns while writing", s.underruns()));
    if cfg!(windows) {
        t.check(!s.device().is_empty(), &format!("play: the device has a name (\"{}\")", s.device()));
    }
    drop(s);

    // 2. a fill stream that ends itself: 500 ms of 777 Hz at 44.1 kHz mono
    let mut left = 22_050usize;
    let mut i = 0u64;
    let f = Stream::open_fill(&Config { rate: 44_100, channels: 1, latency_ms: 40, name: "audiotest fill" }, move |buf: &mut [f32]| {
        for x in buf.iter_mut() {
            if left == 0 {
                break;
            }
            *x = (AMP as f64 * rt::x87::sin(2.0 * core::f64::consts::PI * 777.0 * i as f64 / 44_100.0)) as f32;
            i += 1;
            left -= 1;
        }
        left > 0
    });
    match f {
        Ok(f) => {
            let ended = wait_for(3000, || f.state() == State::Ended);
            t.check(ended && f.played() >= 22_050, &format!("play: a fill stream that returns false ends ({:?}, {} frames)", f.state(), f.played()));
        }
        Err(e) => t.check(false, &format!("play: open a fill stream ({e})")),
    }

    // 3. pause keeps the position, start goes on, stop empties the ring
    match Stream::open(&cfg) {
        Ok(mut s) => {
            let silence = alloc::vec![0.0f32; 48_000 * 2];
            let feed = |s: &mut Stream, ms: u32| {
                let end = plat::time_us() + ms as u64 * 1000;
                while plat::time_us() < end {
                    s.write(&silence[..s.writable() * 2]);
                    plat::sleep_ms(4);
                }
            };
            feed(&mut s, 150);
            s.pause();
            plat::sleep_ms(60);
            let p0 = s.played();
            plat::sleep_ms(150);
            let p1 = s.played();
            t.check(s.state() == State::Paused && p1 == p0, &format!("play: paused, the position stays ({p0} -> {p1})"));
            t.check(s.queued() > 0, &format!("play: paused, the ring keeps its {} frames", s.queued()));
            s.start();
            feed(&mut s, 150);
            t.check(s.played() > p1 + 2400, &format!("play: started again, it goes on ({})", s.played()));
            s.stop();
            let flushed = wait_for(500, || s.queued() == 0);
            t.check(flushed && s.state() == State::Stopped, &format!("play: stop() empties the ring ({} queued, {:?})", s.queued(), s.state()));
            s.start();
            let before = s.played();
            feed(&mut s, 120);
            t.check(s.played() > before, "play: start() after stop() plays again");
            s.set_volume(0.5);
            t.check(s.volume() == 0.5, "play: set_volume(0.5)");
            s.set_volume(3.0);
            let hi = s.volume();
            s.set_volume(f32::NAN);
            t.check(hi == 1.0 && s.volume() == 0.0, "play: volume clamped to 0 ..= 1 (NaN: 0)");
        }
        Err(e) => t.check(false, &format!("play: open a third stream ({e})")),
    }

    // 4. a device change while playing (Windows: reopened on the default device; Linux: the
    // connection dropped and made anew after a second): the stream plays on to its end
    if cfg!(any(windows, target_os = "linux")) {
        match Stream::open(&cfg) {
            Ok(mut s) => {
                let tone = sine(660.0, 48_000, 1800, 2);
                let (mut pos, start, mut changed) = (0usize, plat::time_us(), false);
                while pos < tone.len() / 2 && plat::time_us() - start < 8_000_000 {
                    pos += s.write(&tone[pos * 2..]);
                    if !changed && pos > 9600 {
                        s.test_device_change();
                        changed = true;
                    }
                    plat::sleep_ms(4);
                }
                s.finish();
                let ended = wait_for(4000, || s.state() == State::Ended);
                t.check(ended && s.device_changes() == 1, &format!("play: after a device change the stream plays on to its end ({:?}, {} change(s))", s.state(), s.device_changes()));
                t.check(s.played() >= (tone.len() / 2) as u64, &format!("play: every frame was played ({} of {})", s.played(), tone.len() / 2));
            }
            Err(e) => t.check(false, &format!("play: open a stream for the device change ({e})")),
        }
    }

    // 5. the mixer: a quiet tone and the chime of the app's assets
    let quiet = Sound::new(48_000, 1, sine(1234.0, 48_000, 150, 1)).ok();
    match quiet.as_ref().map(audio::play) {
        Some(Ok(v)) => {
            t.check(v.playing(), "play: play() starts a voice");
            t.check(wait_for(2000, || !v.playing()), "play: the voice ends after its sound");
        }
        _ => t.check(false, "play: play() of a tone"),
    }
    for f in ["app://klang.wav", "app://klang.oga"] {
        match audio::play_file(f) {
            Ok(v) => {
                v.set_volume(0.15);
                t.check(wait_for(3000, || !v.playing()), &format!("play: play_file({f}) plays to its end"));
            }
            Err(e) => t.check(false, &format!("play: play_file({f}) ({e})")),
        }
    }
    let v = audio::play(&Sound::tone(440.0, 5000));
    if let Ok(v) = &v {
        v.set_volume(0.0);
        v.stop();
        t.check(wait_for(500, || !v.playing()), "play: Voice::stop ends a long sound at once");
    }

    #[cfg(windows)]
    if let Ok(lb) = lb {
        let (rate, ch, data) = lb.finish();
        let ch = ch as usize;
        t.info(&format!("loopback: {} Hz, {ch} channels, {} frames", rate, data.len() / ch.max(1)));
        if data.iter().all(|&x| x == 0.0) {
            t.skip("play: the loopback capture was silent (device muted or no engine mix)");
        } else {
            let (a, a_off) = (peak_level(&data, ch, rate, 1234.0), peak_level(&data, ch, rate, 1500.0));
            let (b, b_off) = (peak_level(&data, ch, rate, 777.0), peak_level(&data, ch, rate, 600.0));
            t.info(&format!("levels: 1234 Hz {a:.4} (1500 Hz {a_off:.4}), 777 Hz {b:.4} (600 Hz {b_off:.4})"));
            t.check(a > 0.002 && a > 8.0 * a_off, "play: the 1234 Hz push tone reached the device (loopback)");
            t.check(b > 0.002 && b > 8.0 * b_off, "play: the 777 Hz fill tone at 44.1 kHz reached the device (loopback)");
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn t_pulse(t: &mut T) {
    t.skip("pulse: Linux only");
}

#[cfg(target_os = "linux")]
fn t_pulse(t: &mut T) {
    let Some((server, version, sink)) = audio::server_info() else {
        t.skip("pulse: no PulseAudio server here ($PULSE_SERVER / $XDG_RUNTIME_DIR/pulse/native)");
        return;
    };
    t.info(&format!("server \"{server}\" {version}, default sink \"{sink}\""));
    t.check(audio::available(), "pulse: available()");
    let cfg = Config { rate: 48_000, channels: 2, latency_ms: 60, name: "audiotest pulse" };
    let mut s = match Stream::open(&cfg) {
        Ok(s) => s,
        Err(e) => {
            t.check(false, &format!("pulse: open a stream ({e})"));
            return;
        }
    };
    let id = s.server_id();
    t.check(id != u32::MAX, &format!("pulse: the server gave the stream sink input {id}"));
    t.check(s.device() == sink, &format!("pulse: the stream plays on the default sink (\"{}\")", s.device()));
    let tone = sine(880.0, 48_000, 1500, 2);
    let (mut pos, start) = (0usize, plat::time_us());
    let mut listed: Option<audio::ServerStream> = None;
    let mut lat = 0.0f32;
    while pos < tone.len() / 2 && plat::time_us() - start < 6_000_000 {
        pos += s.write(&tone[pos * 2..]);
        if listed.is_none() && pos > 24_000 {
            listed = audio::server_streams().and_then(|v| v.into_iter().find(|x| x.index == id));
            lat = s.latency_ms();
        }
        plat::sleep_ms(4);
    }
    match &listed {
        Some(x) => {
            let media = x.props.iter().find(|(k, _)| k == "media.name").map(|(_, v)| v.as_str()).unwrap_or("");
            t.info(&format!("sink input {}: \"{}\", {} Hz, {} ch, buffer {} us, sink {} us", x.index, media, x.rate, x.channels, x.buffer_us, x.sink_us));
            t.check(x.rate == 48_000 && x.channels == 2, "pulse: the server lists the stream with 48 kHz stereo");
            t.check(media == "audiotest pulse", &format!("pulse: its media.name is the stream's name (\"{media}\")"));
            t.check(!x.corked, "pulse: it is not corked while playing");
            let server_ms = (x.buffer_us + x.sink_us) as f32 / 1000.0;
            t.check(server_ms < 1000.0, &format!("pulse: the server's latency {server_ms:.1} ms"));
        }
        None => t.check(false, "pulse: the stream is among the server's sink inputs"),
    }
    t.check(lat > 1.0 && lat < 1000.0, &format!("pulse: the stream reports {lat:.1} ms latency (server + ring)"));
    s.pause();
    plat::sleep_ms(300);
    let corked = audio::server_streams().and_then(|v| v.into_iter().find(|x| x.index == id)).is_some_and(|x| x.corked);
    t.check(corked, "pulse: pause() corks the stream at the server");
    s.start();
    s.finish();
    let ended = wait_for(4000, || s.state() == State::Ended);
    t.check(ended, &format!("pulse: finish() -> Ended after DRAIN ({:?})", s.state()));
    t.check(s.played() >= (tone.len() / 2) as u64, &format!("pulse: {} frames played", s.played()));
    t.info(&format!("underruns {}, device changes {}", s.underruns(), s.device_changes()));
    drop(s);
    plat::sleep_ms(200);
    let gone = audio::server_streams().is_some_and(|v| v.iter().all(|x| x.index != id));
    t.check(gone, "pulse: the stream is gone from the server after drop");
    // a system sound from a scratch theme (a WAV), the generated tone without one
    // (in the app's cache folder: the manifest allows no other)
    let vdir = format!("cache://theme-test-{:x}", plat::time_us() & 0xFFFFFF);
    let wav = Sound::new(48_000, 1, sine(990.0, 48_000, 120, 1)).map(|s| s.to_wav()).unwrap_or_default();
    let ok = plat::mkdir(&format!("{vdir}/sounds/scratch/stereo")) && plat::write_file(&format!("{vdir}/sounds/scratch/stereo/dialog-warning.wav"), &wav);
    let dir = plat::vfs::os_path(&vdir).unwrap_or_default();
    let saved: Vec<(&str, Option<String>)> = ["XDG_DATA_DIRS", "XDG_DATA_HOME", "jfgui_SOUND_THEME", "jfgui_EVENT_SOUNDS"].iter().map(|k| (*k, plat::env(k))).collect();
    plat::setenv("XDG_DATA_DIRS", Some(&dir));
    plat::setenv("XDG_DATA_HOME", Some(&format!("{dir}/nothing")));
    plat::setenv("jfgui_SOUND_THEME", Some("scratch"));
    plat::setenv("jfgui_EVENT_SOUNDS", Some("1"));
    t.check(ok && audio::theme_path(audio::SOUND_WARNING).is_some_and(|p| p.ends_with("dialog-warning.wav")), "pulse: the scratch theme's warning");
    t.check(audio::sound(audio::SOUND_WARNING), "pulse: sound(warning) plays the theme's WAV");
    t.check(audio::theme_path(audio::SOUND_NOTIFY).is_none() && audio::sound(audio::SOUND_NOTIFY), "pulse: sound(notify) without a file plays the generated tone");
    plat::setenv("jfgui_EVENT_SOUNDS", Some("0"));
    t.check(!audio::sound(audio::SOUND_WARNING) && !audio::dialog_sounds(), "pulse: event sounds off -> no sound");
    plat::sleep_ms(600);
    for (k, v) in saved {
        plat::setenv(k, v.as_deref());
    }
    for f in ["sounds/scratch/stereo/dialog-warning.wav", "sounds/scratch/stereo", "sounds/scratch", "sounds"] {
        plat::delete(&format!("{vdir}/{f}"));
    }
    plat::delete(&vdir);
}

fn t_sounds(t: &mut T) {
    let kinds = [audio::SOUND_DEFAULT, audio::SOUND_INFO, audio::SOUND_WARNING, audio::SOUND_ERROR, audio::SOUND_QUESTION, audio::SOUND_NOTIFY];
    audio::set_dry_run(true);
    for k in kinds {
        t.check(audio::sound(k), &format!("sounds: dry run of \"{}\"", audio::sound_name(k)));
    }
    audio::set_dry_run(false);
    t.check(!audio::sound(77), "sounds: an unknown kind plays nothing");
    // Linux: as on a desktop with event sounds on (whatever this one's portal says)
    let saved = plat::env("jfgui_EVENT_SOUNDS");
    plat::setenv("jfgui_EVENT_SOUNDS", Some("1"));
    let conv = audio::dialog_sounds();
    plat::setenv("jfgui_EVENT_SOUNDS", saved.as_deref());
    t.check(conv == cfg!(any(windows, target_os = "linux")), &format!("sounds: message dialogs play a sound here: {conv}"));
    if !audio::available() {
        t.skip("sounds: no output device here");
        return;
    }
    // the real ones: a system may assign no sound to an event (false), but none may hang;
    // Windows: what played must reach the device (loopback: some energy)
    let mut played: Vec<String> = Vec::new();
    for k in [audio::SOUND_INFO, audio::SOUND_WARNING] {
        #[cfg(windows)]
        let lb = audio::Loopback::start().ok();
        let start = plat::time_us();
        let p = audio::sound(k);
        t.check(plat::time_us() - start < 2_000_000, &format!("sounds: \"{}\" returns at once", audio::sound_name(k)));
        plat::sleep_ms(1200);
        if p {
            played.push(String::from(audio::sound_name(k)));
        }
        #[cfg(windows)]
        if let Some(lb) = lb {
            let (_, _, data) = lb.finish();
            let peak = data.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            if p {
                t.check(peak > 1e-3, &format!("sounds: \"{}\" reached the device (loopback peak {peak:.4})", audio::sound_name(k)));
            } else {
                t.skip(&format!("sounds: the scheme assigns no sound to \"{}\"", audio::sound_name(k)));
            }
        }
    }
    t.info(&format!("played: {}", played.join(", ")));
}
