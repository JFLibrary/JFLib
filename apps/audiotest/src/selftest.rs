// selftest -- jfgui_SELFTEST=audio: the device test inside the running app, step by step on UI
// timers (Android: the start intent's extras, apps/audiotest/tests/android_audio.py; the app
// has no other threads there).  Results go to data://audio-selftest.txt ("ok NAME" /
// "FAIL NAME: detail" lines, "done PASSED/TOTAL" last).
//
//   0  a fill stream: a quiet 523 Hz tone, 48 kHz stereo     1  it plays, frames played, a
//   latency; pause     2  the position     3  it stayed; start: 4 s on (the script looks at the
//   audio server meanwhile)     4  it played on; stop; play() a tone; the Ogg Vorbis chime
//   decodes; the system's information sound     5  the voice ended: done

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::audio::{self, Config, Sound, State, Stream, Voice};

pub const TIMER: u32 = 7;
const RESULTS: &str = "data://audio-selftest.txt";

pub struct SelfTest {
    step: u32,
    stream: Option<Stream>,
    voice: Option<Voice>,
    mark: u64,
    log: Vec<String>,
    pub done: bool,
}

impl SelfTest {
    pub fn wanted() -> bool {
        plat::env("jfgui_SELFTEST").is_some_and(|v| v == "audio")
    }

    pub fn new() -> SelfTest {
        SelfTest { step: 0, stream: None, voice: None, mark: 0, log: Vec::new(), done: false }
    }

    pub fn status(&self) -> String {
        let passed = self.log.iter().filter(|l| l.starts_with("ok ")).count();
        format!("Selbsttest: Schritt {}, {passed} von {} bestanden{}", self.step, self.log.len(), if self.done { " – fertig" } else { "" })
    }

    /// the next step -> ms until the one after it (None: finished)
    pub fn step(&mut self) -> Option<u32> {
        let s = self.step;
        self.step += 1;
        match s {
            0 => {
                let mut i = 0u64;
                let fill = move |buf: &mut [f32]| {
                    for f in buf.chunks_exact_mut(2) {
                        let v = (0.05 * rt::x87::sin(2.0 * core::f64::consts::PI * 523.25 * i as f64 / 48_000.0)) as f32;
                        f[0] = v;
                        f[1] = v;
                        i += 1;
                    }
                    true
                };
                match Stream::open_fill(&Config { rate: 48_000, channels: 2, latency_ms: 40, name: "audiotest selftest" }, fill) {
                    Ok(st) => {
                        check(&mut self.log, true, "open a fill stream", String::new());
                        self.stream = Some(st);
                    }
                    Err(e) => check(&mut self.log, false, "open a fill stream", format!("{e}")),
                }
                Some(500)
            }
            1 => {
                if let Some(st) = &self.stream {
                    let (state, played, lat) = (st.state(), st.played(), st.latency_ms());
                    check(&mut self.log, state == State::Playing, "it plays", format!("{state:?}"));
                    check(&mut self.log, played > 4800, "frames are played", format!("{played}"));
                    check(&mut self.log, lat > 0.5 && lat < 500.0, "a latency is reported", format!("{lat:.1} ms, device {}", st.device()));
                    st.pause();
                }
                Some(300)
            }
            2 => {
                if let Some(st) = &self.stream {
                    self.mark = st.played();
                }
                Some(300)
            }
            3 => {
                if let Some(st) = &self.stream {
                    let p = st.played();
                    check(&mut self.log, st.state() == State::Paused && p == self.mark, "pause keeps the position", format!("{} -> {p}", self.mark));
                    st.start();
                }
                Some(4000)
            }
            4 => {
                if let Some(st) = &self.stream {
                    let p = st.played();
                    check(&mut self.log, p > self.mark + 4 * 48_000 / 2, "it played on after start", format!("{} -> {p}", self.mark));
                    check(&mut self.log, st.underruns() < 10, "few underruns", format!("{}", st.underruns()));
                    st.stop();
                }
                self.stream = None;
                let tone = Sound::new(48_000, 1, (0..9600).map(|i| (0.05 * rt::x87::sin(2.0 * core::f64::consts::PI * 660.0 * i as f64 / 48_000.0)) as f32).collect());
                match tone.as_ref().map(audio::play) {
                    Ok(Ok(v)) => {
                        check(&mut self.log, v.playing(), "play() starts a voice", String::new());
                        self.voice = Some(v);
                    }
                    other => check(&mut self.log, false, "play() starts a voice", format!("{:?}", other.err())),
                }
                // the chime as Ogg Vorbis: decoded here (the portable maths of this target)
                let ogg = Sound::load("app://klang.oga");
                let ok = ogg.as_ref().is_ok_and(|s| s.rate() == 22_050 && s.frames() == 13_230);
                check(&mut self.log, ok, "the Ogg Vorbis chime decodes", format!("{ogg:?}"));
                let s = audio::sound(audio::SOUND_INFO);
                check(&mut self.log, s, "the system's information sound", String::from("not played"));
                Some(1500)
            }
            _ => {
                let ended = self.voice.as_ref().is_some_and(|v| !v.playing());
                check(&mut self.log, ended, "the voice ended", String::new());
                let passed = self.log.iter().filter(|l| l.starts_with("ok ")).count();
                self.log.push(format!("done {passed}/{}", self.log.len()));
                let mut text = self.log.join("\n");
                text.push('\n');
                plat::write_file(RESULTS, text.as_bytes());
                plat::write_stdout(text.as_bytes());
                self.done = true;
                None
            }
        }
    }
}

// one result line
fn check(log: &mut Vec<String>, ok: bool, name: &str, detail: String) {
    log.push(if ok { format!("ok {name}") } else { format!("FAIL {name}: {detail}") });
}
