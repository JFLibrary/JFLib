// t_audio -- plat::audio without a device: WAV files, the ring, sample-rate conversion,
// generated tones, Ogg Vorbis, the PulseAudio wire format (Linux)
//
//   plattest --test audio   WAV: PCM 8 / 16 / 24 / 32 bits, float 32 / 64, extensible, mono,
//                           stereo, 6 channels, chunks in odd places, truncated data, and the
//                           files that must be refused; the ring in pieces and across a thread;
//                           the resampler (44.1 <-> 48 kHz, 96 -> 48 kHz without aliasing,
//                           8 -> 48 kHz without images); tones and their level; Ogg Vorbis
//                           files of apps/plattest/data/audio against their sine; Linux: the
//                           PulseAudio tagstruct and packets against hand-made bytes

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use plat::audio::{self, AudioError, Sound};

use crate::T;

// a WAVE file: fmt chunk fields, the data, extra chunks before fmt / between
fn wav(tag: u16, ch: u16, rate: u32, bits: u16, data: &[u8], extensible: bool) -> Vec<u8> {
    let align = ch * bits.div_ceil(8);
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&(if extensible { 0xFFFEu16 } else { tag }).to_le_bytes());
    fmt.extend_from_slice(&ch.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align as u32).to_le_bytes());
    fmt.extend_from_slice(&align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        fmt.extend_from_slice(&3u32.to_le_bytes());
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71]);
    }
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    chunk(&mut body, b"fmt ", &fmt);
    chunk(&mut body, b"data", data);
    let mut f = Vec::new();
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(body.len() as u32).to_le_bytes());
    f.extend_from_slice(&body);
    f
}

fn chunk(b: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    b.extend_from_slice(id);
    b.extend_from_slice(&(data.len() as u32).to_le_bytes());
    b.extend_from_slice(data);
    if data.len() % 2 == 1 {
        b.push(0);
    }
}

fn close(a: &[f32], b: &[f32], tol: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

fn t_wav(t: &mut T) {
    // the same five values in every format: 0, 0.5, -0.5, near +1, -1
    let want = [0.0f32, 0.5, -0.5, 1.0, -1.0];
    let pcm8: Vec<u8> = vec![128, 192, 64, 255, 0];
    let pcm16: Vec<u8> = [0i16, 16384, -16384, 32767, -32768].iter().flat_map(|v| v.to_le_bytes()).collect();
    let pcm24: Vec<u8> = [0i32, 0x40_0000, -0x40_0000, 0x7F_FFFF, -0x80_0000].iter().flat_map(|v| v.to_le_bytes()[..3].to_vec()).collect();
    let pcm32: Vec<u8> = [0i32, 0x4000_0000, -0x4000_0000, i32::MAX, i32::MIN].iter().flat_map(|v| v.to_le_bytes()).collect();
    let f32d: Vec<u8> = want.iter().flat_map(|v| v.to_le_bytes()).collect();
    let f64d: Vec<u8> = want.iter().flat_map(|v| (*v as f64).to_le_bytes()).collect();
    for (name, tag, bits, data, tol) in [
        ("PCM 8", 1u16, 8u16, &pcm8, 1.0 / 127.0),
        ("PCM 16", 1, 16, &pcm16, 1.0 / 32767.0),
        ("PCM 24", 1, 24, &pcm24, 1.0 / 8_388_607.0),
        ("PCM 32", 1, 32, &pcm32, 1e-6),
        ("float 32", 3, 32, &f32d, 0.0),
        ("float 64", 3, 64, &f64d, 0.0),
    ] {
        for ext in [false, true] {
            let r = Sound::wav(&wav(tag, 1, 44_100, bits, data, ext));
            let ok = r.as_ref().is_ok_and(|s| s.rate() == 44_100 && s.channels() == 1 && close(s.samples(), &want, tol));
            t.check(ok, &format!("audio wav: {name}{} mono", if ext { " (extensible)" } else { "" }));
            if !ok {
                t.info(&format!("  {:?}\n", r.map(|s| s.samples().to_vec())));
            }
        }
    }
    // stereo 16 bit: frames of (left, right); 6 channels float
    let st: Vec<u8> = [1000i16, -1000, 2000, -2000].iter().flat_map(|v| v.to_le_bytes()).collect();
    let s = Sound::wav(&wav(1, 2, 48_000, 16, &st, false));
    t.check(s.as_ref().is_ok_and(|s| s.channels() == 2 && s.frames() == 2 && (s.samples()[1] + 1000.0 / 32768.0).abs() < 1e-7), "audio wav: stereo 16 bit, interleaved");
    let six: Vec<u8> = (0..12).flat_map(|i| (i as f32 / 16.0).to_le_bytes()).collect();
    let s = Sound::wav(&wav(3, 6, 96_000, 32, &six, true));
    t.check(s.as_ref().is_ok_and(|s| s.channels() == 6 && s.frames() == 2 && s.rate() == 96_000), "audio wav: 6 channels float, 96 kHz");
    // a LIST chunk with an odd size before fmt, data before fmt
    let mut body = b"WAVE".to_vec();
    chunk(&mut body, b"LIST", b"INFOx");
    chunk(&mut body, b"data", &pcm16);
    let fmt = &wav(1, 1, 22_050, 16, &[], false)[20..36];
    chunk(&mut body, b"fmt ", fmt);
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(body.len() as u32).to_le_bytes());
    f.extend_from_slice(&body);
    t.check(Sound::wav(&f).is_ok_and(|s| s.rate() == 22_050 && s.frames() == 5), "audio wav: odd LIST chunk first, data before fmt");
    // a data chunk that claims more than the file has: the whole frames that are there
    let mut f = wav(1, 2, 8000, 16, &st, false);
    let n = f.len();
    f[n - 8 - 4..n - 8].copy_from_slice(&1000u32.to_le_bytes());
    f.truncate(n - 2);
    t.check(Sound::wav(&f).is_ok_and(|s| s.frames() == 1), "audio wav: truncated data -> the whole frames there");
    // refused
    let bad: [(&str, Vec<u8>); 8] = [
        ("empty", Vec::new()),
        ("not RIFF", b"RIFX\0\0\0\0WAVE".to_vec()),
        ("no fmt", {
            let mut b = b"RIFF\x0c\0\0\0WAVE".to_vec();
            chunk(&mut b, b"data", &pcm16);
            b
        }),
        ("no data", wav(1, 1, 8000, 16, &[], false)[..36].to_vec()),
        ("ADPCM", wav(2, 1, 8000, 4, &pcm8, false)),
        ("0 channels", wav(1, 0, 8000, 16, &pcm16, false)),
        ("block size", {
            let mut b = wav(1, 2, 8000, 16, &st, false);
            b[32] = 3;
            b
        }),
        ("rate 0", wav(1, 1, 0, 16, &pcm16, false)),
    ];
    for (name, b) in bad {
        let r = Sound::wav(&b);
        t.check(r.as_ref().err() == Some(&AudioError::Format), &format!("audio wav: {name} refused"));
    }
    t.check(Sound::decode(b"ID3\x04").err() == Some(AudioError::Format), "audio decode: unknown bytes refused");
    // round trip through to_wav (16 bit)
    let tone = Sound::tone(440.0, 50);
    let back = Sound::wav(&tone.to_wav());
    t.check(back.as_ref().is_ok_and(|b| b.rate() == 48_000 && close(b.samples(), tone.samples(), 3.0 / 65536.0)), "audio wav: to_wav round trip");
    t.check(Sound::load("mem://nichts.wav").err() == Some(AudioError::Io), "audio load: a missing file is Io");
    let p = format!("{}/ton.wav", t.base);
    t.check(plat::write_file(&p, &tone.to_wav()) && Sound::load(&p).is_ok_and(|s| s.duration_ms() == 50), "audio load: a WAV file through the VFS");
}

fn t_ring(t: &mut T) {
    for (piece, take) in [(1, 1), (7, 3), (64, 1000), (999, 17), (1024, 1024)] {
        t.check(audio::ring_check(20_000, piece, take, false), &format!("audio ring: 20000 samples in pieces of {piece}, read by {take}"));
    }
    for (piece, take) in [(3, 5), (480, 256), (1024, 7)] {
        t.check(audio::ring_check(2_000_000, piece, take, true), &format!("audio ring: 2000000 samples from a writer thread in pieces of {piece}, read by {take}"));
    }
}

// no sound server (Linux): every stream is Unavailable, nothing hangs
#[cfg(target_os = "linux")]
fn t_no_server(t: &mut T) {
    let saved = plat::env("PULSE_SERVER");
    plat::setenv("PULSE_SERVER", Some("unix:/nonexistent/plattest/pulse"));
    let start = plat::time_us();
    let r = audio::Stream::open(&audio::Config::new(48_000, 2));
    t.check(r.as_ref().err() == Some(&AudioError::Unavailable), &format!("audio: no sound server -> Unavailable ({:?})", r.as_ref().err()));
    t.check(!audio::available() && audio::play(&Sound::tone(440.0, 10)).err() == Some(AudioError::Unavailable), "audio: available() false, play() Unavailable");
    t.check(plat::time_us() - start < 2_000_000, "audio: and that at once");
    plat::setenv("PULSE_SERVER", saved.as_deref());
}

fn t_dsp(t: &mut T) {
    // tones and their level
    let s = Sound::tone(1000.0, 200);
    let l = audio::level(s.samples(), 1, 0, 48_000, 1000.0);
    t.check((l - 0.5).abs() < 0.01, &format!("audio tone: 1 kHz at half scale reads {l:.4}"));
    let off = audio::level(s.samples(), 1, 0, 48_000, 1500.0);
    t.check(off < 0.01, &format!("audio level: 1.5 kHz in a 1 kHz tone reads {off:.5}"));
    // 44.1 -> 48 kHz and back: the frequency and the level stay
    let src = Sound::tone(1000.0, 300);
    let at44 = audio::resample(src.samples(), 1, 48_000, 44_100);
    t.check((at44.len() as i64 - 13_230).abs() <= 1, &format!("audio resample: 48 -> 44.1 kHz length {}", at44.len()));
    let mid = &at44[2000..at44.len() - 2000];
    let l44 = audio::level(mid, 1, 0, 44_100, 1000.0);
    t.check((l44 - 0.5).abs() < 0.01, &format!("audio resample: 1 kHz after 48 -> 44.1 kHz reads {l44:.4}"));
    let back = audio::resample(&at44, 1, 44_100, 48_000);
    let lb = audio::level(&back[2000..back.len() - 2000], 1, 0, 48_000, 1000.0);
    t.check((lb - 0.5).abs() < 0.01, &format!("audio resample: and back to 48 kHz reads {lb:.4}"));
    // stereo stays apart
    let st: Vec<f32> = src.samples().iter().flat_map(|&v| [v, 0.0]).collect();
    let r = audio::resample(&st, 2, 48_000, 44_100);
    let (l, rr) = (audio::level(&r[4000..r.len() - 4000], 2, 0, 44_100, 1000.0), audio::level(&r[4000..r.len() - 4000], 2, 1, 44_100, 1000.0));
    t.check((l - 0.5).abs() < 0.01 && rr < 0.001, &format!("audio resample: stereo channels stay apart ({l:.4} / {rr:.5})"));
    // 96 -> 48 kHz: 30 kHz must not alias to 18 kHz, 5 kHz passes
    let hi: Vec<f32> = {
        let a = dsp_tone(30_000.0, 96_000, 9600);
        let b = dsp_tone(5000.0, 96_000, 9600);
        a.iter().zip(&b).map(|(x, y)| x + y).collect()
    };
    let down = audio::resample(&hi, 1, 96_000, 48_000);
    let part = &down[500..down.len() - 500];
    let (alias, pass) = (audio::level(part, 1, 0, 48_000, 18_000.0), audio::level(part, 1, 0, 48_000, 5000.0));
    t.check(alias < 0.005 && (pass - 0.5).abs() < 0.01, &format!("audio resample: 96 -> 48 kHz, 30 kHz removed ({alias:.5}), 5 kHz kept ({pass:.4})"));
    // 8 -> 48 kHz: no image of 1 kHz at 7 kHz / 9 kHz
    let low = dsp_tone(1000.0, 8000, 1600);
    let up = audio::resample(&low, 1, 8000, 48_000);
    let part = &up[600..up.len() - 600];
    let (img, keep) = (audio::level(part, 1, 0, 48_000, 7000.0).max(audio::level(part, 1, 0, 48_000, 9000.0)), audio::level(part, 1, 0, 48_000, 1000.0));
    t.check(img < 0.005 && (keep - 0.5).abs() < 0.01, &format!("audio resample: 8 -> 48 kHz without images ({img:.5}), 1 kHz kept ({keep:.4})"));
    // the system sounds' generated tones (browser, Linux without a theme)
    for k in [audio::SOUND_DEFAULT, audio::SOUND_INFO, audio::SOUND_WARNING, audio::SOUND_ERROR, audio::SOUND_QUESTION, audio::SOUND_NOTIFY] {
        t.check(!audio::sound_name(k).is_empty(), &format!("audio sound: kind {k} is \"{}\"", audio::sound_name(k)));
    }
    t.check(audio::sound_name(99).is_empty() && !audio::sound(99), "audio sound: an unknown kind plays nothing");
}

// a sine at half scale without fades (plain samples)
fn dsp_tone(freq: f32, rate: u32, n: usize) -> Vec<f32> {
    (0..n).map(|i| (0.5 * rt::x87::sin(2.0 * core::f64::consts::PI * freq as f64 * i as f64 / rate as f64)) as f32).collect()
}

// the PulseAudio wire format against hand-made packets (Linux)
#[cfg(target_os = "linux")]
fn t_pulse_wire(t: &mut T) {
    for (name, ok) in audio::pulse_selftest() {
        t.check(ok, &format!("audio pulse wire: {name}"));
    }
}

// the freedesktop sound theme in a scratch data folder: inheritance, Directories=, .disabled,
// the freedesktop fallback, names that fall back to others (Linux)
#[cfg(target_os = "linux")]
fn t_theme(t: &mut T) {
    let base = format!("{}/{}/data", plat::cwd(), t.base);
    let wav = Sound::tone(660.0, 40).to_wav();
    let files = [
        ("sounds/testtheme/index.theme", b"[Sound Theme]\nName=Test\nInherits=parent\nDirectories=stereo,5.1\n".to_vec()),
        ("sounds/testtheme/stereo/dialog-warning.wav", wav.clone()),
        ("sounds/testtheme/stereo/dialog-error.disabled", Vec::new()),
        ("sounds/parent/index.theme", b"[Sound Theme]\nName=Parent\nDirectories=stereo\n".to_vec()),
        ("sounds/parent/stereo/dialog-information.wav", wav.clone()),
        ("sounds/freedesktop/stereo/bell.wav", wav),
    ];
    let mut ok = true;
    for (p, b) in &files {
        let full = format!("{base}/{p}");
        ok &= plat::mkdir(&full[..full.rfind('/').unwrap_or(0)]) && plat::write_file(&full, b);
    }
    t.check(ok, "audio theme: a scratch sound theme written");
    let saved: Vec<(&str, Option<alloc::string::String>)> =
        ["XDG_DATA_HOME", "XDG_DATA_DIRS", "jfgui_SOUND_THEME", "jfgui_EVENT_SOUNDS"].iter().map(|k| (*k, plat::env(k))).collect();
    plat::setenv("XDG_DATA_HOME", Some(&format!("{base}/home-nothing")));
    plat::setenv("XDG_DATA_DIRS", Some(&base));
    plat::setenv("jfgui_SOUND_THEME", Some("testtheme"));
    plat::setenv("jfgui_EVENT_SOUNDS", Some("1"));
    let tail = |k: u32| audio::theme_path(k).map(|p| alloc::string::String::from(p.strip_prefix(&format!("{base}/sounds/")).unwrap_or(&p)));
    t.expect_s(tail(audio::SOUND_WARNING).as_deref(), Some("testtheme/stereo/dialog-warning.wav"), "audio theme: the theme's own file");
    t.expect_s(tail(audio::SOUND_INFO).as_deref(), Some("parent/stereo/dialog-information.wav"), "audio theme: from the inherited theme");
    t.expect_s(tail(audio::SOUND_ERROR).as_deref(), Some("testtheme/stereo/dialog-error.disabled"), "audio theme: .disabled is found (no sound)");
    t.expect_s(tail(audio::SOUND_DEFAULT).as_deref(), Some("freedesktop/stereo/bell.wav"), "audio theme: bell from the freedesktop fallback");
    t.expect_s(tail(audio::SOUND_QUESTION).as_deref(), Some("parent/stereo/dialog-information.wav"), "audio theme: no dialog-question -> dialog-information");
    t.expect_s(tail(audio::SOUND_NOTIFY).as_deref(), None, "audio theme: no message sound -> none (a generated tone)");
    plat::setenv("jfgui_SOUND_THEME", Some("parent"));
    t.expect_s(tail(audio::SOUND_WARNING).as_deref(), None, "audio theme: another theme, without a warning");
    plat::setenv("jfgui_EVENT_SOUNDS", Some("0"));
    t.check(!audio::dialog_sounds(), "audio theme: event sounds off -> dialogs play none");
    for (k, v) in saved {
        plat::setenv(k, v.as_deref());
    }
}

/// plattest --test audio-ref --ogg F.oga --raw F.raw: our decoding of F against another
/// decoder's (raw f32le in the same channel order, e.g. ffmpeg -c:a libvorbis -i F -f f32le):
/// the best alignment within 2048 frames, the largest and the RMS difference (development)
pub fn t_audio_ref(t: &mut T) {
    let args = base::Args(plat::args());
    let (Some(ogg), Some(raw)) = (args.value("--ogg"), args.value("--raw")) else {
        t.check(false, "audio-ref: --ogg F --raw F");
        return;
    };
    let s = plat::read_file(ogg).ok_or(AudioError::Io).and_then(|b| Sound::decode(&b));
    let r: Vec<f32> = plat::read_file(raw).unwrap_or_default().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let Ok(s) = s else {
        t.check(false, &format!("audio-ref: {ogg} decodes ({:?})", s.err()));
        return;
    };
    let ch = s.channels() as usize;
    let a = s.samples();
    // the reference in Vorbis order (libvorbis does not reorder): into WAVE order
    let order: &[usize] = match ch {
        3 => &[0, 2, 1],
        5 => &[0, 2, 1, 3, 4],
        6 => &[0, 2, 1, 5, 3, 4],
        _ => &[0, 1, 2, 3, 4, 5, 6, 7],
    };
    let r: Vec<f32> = r.chunks_exact(ch).flat_map(|f| (0..ch).map(|c| f[order[c]]).collect::<Vec<f32>>()).collect();
    let (fa, fr) = (a.len() / ch, r.len() / ch);
    let mut best = (f32::MAX, 0isize, 0.0f32);
    for off in -2048isize..=2048 {
        let mut max = 0.0f32;
        let mut sum = 0.0f64;
        let mut n = 0usize;
        for i in 0..fa {
            let j = i as isize + off;
            if j < 0 || j as usize >= fr {
                continue;
            }
            for c in 0..ch {
                let d = (a[i * ch + c] - r[j as usize * ch + c]).abs();
                max = max.max(d);
                sum += (d * d) as f64;
            }
            n += 1;
        }
        if n > fa / 2 && max < best.0 {
            best = (max, off, rt::fx::sqrtd(sum / (n * ch) as f64) as f32);
        }
    }
    t.info(&format!("  {ogg}: {} Hz, {ch} ch, ours {fa} frames, theirs {fr}; offset {}, max diff {:.2e}, rms {:.2e}\n", s.rate(), best.1, best.0, best.2));
    t.check(best.0 < 1e-3, &format!("audio-ref: {ogg} matches the reference (max {:.2e})", best.0));
}

// the Ogg Vorbis fixtures (apps/plattest/tests/audio, made by libvorbis through ffmpeg from
// generated tones): the exact length, each channel's tone at its level, in WAVE order
fn t_vorbis(t: &mut T) {
    let load = |n: &str| plat::read_file(&format!("apps/plattest/tests/audio/{n}.oga")).ok_or(AudioError::Io).and_then(|b| Sound::decode(&b));
    let lv = |s: &Sound, c: usize, f: f32| {
        let ch = s.channels() as usize;
        let x = s.samples();
        audio::level(&x[x.len() / 4 / ch * ch..x.len() * 3 / 4 / ch * ch], ch, c, s.rate(), f)
    };
    match load("sine440") {
        Ok(s) => {
            t.check(s.rate() == 44_100 && s.channels() == 1 && s.frames() == 22_050, &format!("audio vorbis: mono 44.1 kHz, 0.5 s exactly ({s:?})"));
            let (a, b) = (lv(&s, 0, 440.0), lv(&s, 0, 1000.0));
            t.check((a - 0.5).abs() < 0.02 && b < 0.005, &format!("audio vorbis: 440 Hz at 0.5 ({a:.4}), nothing at 1 kHz ({b:.5})"));
        }
        Err(e) => t.check(false, &format!("audio vorbis: sine440.oga decodes ({e})")),
    }
    match load("chord") {
        Ok(s) => {
            t.check(s.rate() == 48_000 && s.channels() == 2 && s.frames() == 28_800, &format!("audio vorbis: stereo 48 kHz, 0.6 s ({s:?})"));
            let (l, r, x) = (lv(&s, 0, 660.0), lv(&s, 1, 990.0), lv(&s, 0, 990.0));
            t.check((l - 0.4).abs() < 0.02 && (r - 0.4).abs() < 0.02 && x < 0.01, &format!("audio vorbis: coupled stereo, 660 Hz left ({l:.4}), 990 Hz right ({r:.4}), apart ({x:.5})"));
        }
        Err(e) => t.check(false, &format!("audio vorbis: chord.oga decodes ({e})")),
    }
    match load("chirp") {
        Ok(s) => {
            let x = s.samples();
            let rms = rt::fx::sqrtd(x.iter().map(|&v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64);
            t.check(s.rate() == 22_050 && s.frames() == 8820 && (rms - 0.354).abs() < 0.03, &format!("audio vorbis: lowest quality (q -1), a chirp at RMS {rms:.3}"));
        }
        Err(e) => t.check(false, &format!("audio vorbis: chirp.oga decodes ({e})")),
    }
    match load("clicks") {
        Ok(s) => {
            let x = s.samples();
            let peak = x.chunks_exact(2).fold(0.0f32, |m, f| m.max(f[0].abs()));
            // between the clicks (50 .. 90 ms of each 100 ms) the left channel is quiet
            let quiet = (0..5).all(|k| x[(k * 4410 + 2205) * 2..(k * 4410 + 3969) * 2].chunks_exact(2).all(|f| f[0].abs() < 0.02));
            let r = lv(&s, 1, 500.0);
            t.check(peak > 0.5 && quiet && (r - 0.3).abs() < 0.02, &format!("audio vorbis: short blocks around clicks (peak {peak:.3}, quiet between: {quiet}), 500 Hz right ({r:.4})"));
        }
        Err(e) => t.check(false, &format!("audio vorbis: clicks.oga decodes ({e})")),
    }
    match load("surround") {
        Ok(s) => {
            let want = [300.0, 400.0, 500.0, 60.0, 700.0, 800.0];
            let levels: Vec<f32> = (0..6).map(|c| lv(&s, c, want[c])).collect();
            let ok = s.channels() == 6 && levels.iter().enumerate().all(|(c, &l)| l > if c == 3 { 0.1 } else { 0.25 });
            t.check(ok, &format!("audio vorbis: 5.1 in WAVE order (FL FR FC LFE BL BR) {levels:.3?}"));
        }
        Err(e) => t.check(false, &format!("audio vorbis: surround.oga decodes ({e})")),
    }
    // damaged files
    if let Some(b) = plat::read_file("apps/plattest/tests/audio/clicks.oga") {
        // cut inside the last of its 100 ms pages: the pages before it decode
        let last = b.windows(4).rposition(|w| w == b"OggS").unwrap_or(0);
        let cut = Sound::decode(&b[..last + 10]);
        t.check(cut.as_ref().is_ok_and(|s| s.frames() > 4410 && s.frames() < 22_050), &format!("audio vorbis: a file cut off decodes up to its last whole page ({cut:?})"));
        let mut bad = b.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x40;
        t.check(Sound::decode(&bad).err() == Some(AudioError::Format), "audio vorbis: a damaged page (CRC) is refused");
    }
    t.check(Sound::decode(b"OggS\0\x02").err() == Some(AudioError::Format), "audio vorbis: a stub page is refused");
}

pub fn t_audio(t: &mut T) {
    t.scratch_init();
    t_wav(t);
    t_ring(t);
    t_dsp(t);
    t_vorbis(t);
    #[cfg(target_os = "linux")]
    t_pulse_wire(t);
    #[cfg(target_os = "linux")]
    t_no_server(t);
    #[cfg(target_os = "linux")]
    t_theme(t);
    t.scratch_done();
}
