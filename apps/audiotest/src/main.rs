// audiotest -- sound output of the library (plat::audio, ui.beep): a tone generator, a WAV /
// Ogg Vorbis file played once (the app's assets: klang.wav, klang.oga; or one the user opens),
// the system's sounds and a dialog that plays its own
//
//   python build_rs.py run audiotest
//   python build_rs.py test audiotest           tests/*.uis: the UI headless, and the device
//                                               tests (--test play / sounds, src/tests.rs)
//   build/win/audiotest.exe --test play         with the real output (SKIP without one)
//
// The generator is a fill stream (plat::audio::Stream::open_fill, src/tone.rs): frequency,
// volume and wave go to the audio thread through atomics.  The status line shows what the
// stream reports (state, device, latency, underruns), refreshed by a timer while it plays.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod selftest;
mod tests;
mod tone;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;

use plat::audio::{self, Config, Stream};
use ui::*;

const ID_WAVE: u32 = 96; // .. + 1
const ID_FREQ: u32 = 101;
const ID_VOL: u32 = 102;
const ID_PLAY: u32 = 103;
const ID_CHIME: u32 = 110;
const ID_CHIME_OGG: u32 = 113;
const ID_OPEN: u32 = 111;
const ID_FD: u32 = 112;
const ID_SOUNDS: u32 = 120; // .. + 5: the system sounds
const ID_SOUNDS_ON: u32 = 130;
const ID_DIALOG: u32 = 131;
const ID_DLG: u32 = 140;
const ID_PAGE: u32 = 150;

const CMD_QUIT: u32 = 1;
const TIMER_STATUS: u32 = 1;

static CMDS: [Command; 1] = [Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0)];
static M_FILE: [Mi; 1] = [Mi::Cmd(CMD_QUIT)];
static MENUBAR: [Mi; 1] = [Mi::Menu("&Datei", &M_FILE)];

const DESC: AppDesc = AppDesc {
    title: "Audio-Test",
    flags: AF_RESIZABLE,
    w: 720,
    h: 640,
    min_w: 420,
    min_h: 360,
    cmds: &CMDS,
    menubar: &MENUBAR,
    ..AppDesc::NONE
};

const RG_FREQ: Range = Range::new(100, 2000, 10, 0);
const RG_VOL: Range = Range::new(0, 100, 1, 0);
const SOUNDS: [(u32, &str); 6] = [
    (audio::SOUND_DEFAULT, "Standard"),
    (audio::SOUND_INFO, "Information"),
    (audio::SOUND_WARNING, "Warnung"),
    (audio::SOUND_ERROR, "Fehler"),
    (audio::SOUND_QUESTION, "Frage"),
    (audio::SOUND_NOTIFY, "Benachrichtigung"),
];

struct Audiotest {
    wave: i32,
    freq: i32,
    vol: i32,
    tone: Arc<tone::Gen>,
    stream: Option<Stream>,
    note: String,
    sound_note: String,
    sounds_on: i32,
    dlg: Shared<i32>,
    fd: Shared<i32>,
    path: String,
    selftest: Option<selftest::SelfTest>,
    started: bool,
}

impl Default for Audiotest {
    fn default() -> Self {
        Audiotest {
            wave: tone::WAVE_SINE as i32,
            freq: 440,
            vol: 20,
            tone: tone::Gen::new(440.0, 0.2, tone::WAVE_SINE),
            stream: None,
            note: String::new(),
            sound_note: String::new(),
            sounds_on: 1,
            dlg: Shared::new(0),
            fd: Shared::new(0),
            path: String::new(),
            selftest: None,
            started: false,
        }
    }
}

impl Audiotest {
    fn sync_gen(&self) {
        self.tone.set(self.freq as f32, self.vol as f32 / 100.0, self.wave as u32);
    }

    fn toggle(&mut self, ui: &mut Ui) {
        if self.stream.take().is_some() {
            self.note = String::from("Ton angehalten.");
            return;
        }
        self.sync_gen();
        let cfg = Config { rate: 48_000, channels: 2, latency_ms: 30, name: "Ton-Generator" };
        match Stream::open_fill(&cfg, tone::fill(self.tone.clone(), 48_000)) {
            Ok(s) => {
                self.stream = Some(s);
                self.note.clear();
                ui.timer(TIMER_STATUS, 200);
            }
            Err(e) => self.note = format!("Kein Ton: {e}"),
        }
    }

    fn status(&self) -> String {
        match &self.stream {
            None => String::from("Aus"),
            Some(s) => {
                let state = match s.state() {
                    audio::State::Starting => "Startet",
                    audio::State::Playing => "Spielt",
                    audio::State::Paused => "Pausiert",
                    audio::State::Stopped => "Gestoppt",
                    audio::State::Ended => "Zu Ende",
                    audio::State::Blocked => "Wartet auf einen Klick (Browser)",
                    audio::State::Lost => "Kein Gerät",
                };
                let dev = s.device();
                format!(
                    "{state} · Gerät: {} · Latenz: {:.1} ms · Aussetzer: {} · Gerätewechsel: {}",
                    if dev.is_empty() { "?" } else { &dev },
                    s.latency_ms(),
                    s.underruns(),
                    s.device_changes()
                )
            }
        }
    }

    fn play_result(&mut self, what: &str, r: Result<audio::Voice, audio::AudioError>) {
        self.note = match r {
            Ok(_) => format!("{what} wird abgespielt."),
            Err(e) => format!("{what}: {e}"),
        };
    }
}

impl App for Audiotest {
    fn frame(&mut self, ui: &mut Ui) {
        if !self.started {
            self.started = true;
            if selftest::SelfTest::wanted() {
                let mut t = selftest::SelfTest::new();
                if let Some(ms) = t.step() {
                    ui.timer(selftest::TIMER, ms);
                }
                self.selftest = Some(t);
            }
        }
        ui.set_sounds(self.sounds_on != 0);
        ui.menubar(None);
        ui.next_size(0, -1);
        ui.scroll_begin(ID_PAGE, SC_V);
        ui.box_begin(0, BOX_V | pad(20), 14);
        ui.heading(1, TS_TITLE, "Audio-Test");
        if let Some(t) = &self.selftest {
            ui.text(TS_CAPTION | TX_SECONDARY, &t.status());
        }

        // the tone generator
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
        ui.heading(2, TS_SUBTITLE, "Ton-Generator");
        ui.box_begin(0, BOX_H | BOX_WRAP, 12);
        let mut changed = ui.radio(ID_WAVE, "Sinus", &mut self.wave, tone::WAVE_SINE as i32) & EV_ACT != 0;
        changed |= ui.radio(ID_WAVE + 1, "Rechteck", &mut self.wave, tone::WAVE_SQUARE as i32) & EV_ACT != 0;
        ui.box_end();
        changed |= ui.prop_slider(ID_FREQ, "Frequenz (Hz)", &mut self.freq, &RG_FREQ, 440) & (EV_ACT | EV_COMMIT) != 0;
        changed |= ui.prop_slider(ID_VOL, "Lautstärke (%)", &mut self.vol, &RG_VOL, 20) & (EV_ACT | EV_COMMIT) != 0;
        if changed {
            self.sync_gen();
        }
        let label = if self.stream.is_some() { "Anhalten" } else { "Abspielen" };
        if ui.accent_button(ID_PLAY, label) & EV_ACT != 0 {
            self.toggle(ui);
        }
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &self.status());
        ui.box_end();

        // a sound once
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
        ui.heading(2, TS_SUBTITLE, "Klang abspielen");
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        if ui.button(ID_CHIME, "Glocke (WAV)") & EV_ACT != 0 {
            let r = audio::play_file("app://klang.wav");
            self.play_result("Glocke", r);
        }
        if ui.button(ID_CHIME_OGG, "Glocke (Ogg Vorbis)") & EV_ACT != 0 {
            let r = audio::play_file("app://klang.oga");
            self.play_result("Glocke (Vorbis)", r);
        }
        if ui.button(ID_OPEN, "Datei öffnen …") & EV_ACT != 0 {
            self.fd.set(1);
        }
        ui.box_end();
        if !self.note.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.note);
        }
        ui.box_end();

        // the system's sounds
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
        ui.heading(2, TS_SUBTITLE, "Systemklänge");
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        for (i, (kind, label)) in SOUNDS.iter().enumerate() {
            if ui.button(ID_SOUNDS + i as u32, label) & EV_ACT != 0 {
                self.sound_note = if ui.beep(*kind) { String::new() } else { format!("„{label}“: kein Klang (aus oder keiner zugeordnet).") };
            }
        }
        ui.box_end();
        if !self.sound_note.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.sound_note);
        }
        ui.switch(ID_SOUNDS_ON, "Klänge an", &mut self.sounds_on);
        if ui.button(ID_DIALOG, "Warnung im Dialog") & EV_ACT != 0 {
            self.dlg.set(1);
        }
        ui.box_end();
        ui.box_end();
        ui.scroll_end();

        let dlg = self.dlg.clone();
        if ui.dialog_begin(ID_DLG, "Achtung", &dlg) {
            ui.dialog_sound(audio::SOUND_WARNING);
            ui.text(TS_BODY | TX_WRAP, "Dieser Dialog spielt beim Öffnen den Warnklang des Systems (wo Meldungen das tun).");
            ui.dialog_buttons(Some("OK"), None, None);
        }
        ui.dialog_end();
        let fd = self.fd.clone();
        if ui.filedlg(ID_FD, FD_OPEN, &fd, &mut self.path, 4096, &["wav=WAV-Klang", "ogg=Ogg Vorbis", "oga=Ogg-Audio"]) & EV_ACT != 0 {
            let r = audio::play_file(&self.path.clone());
            let name = String::from(plat::display_name(&self.path));
            self.play_result(&name, r);
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            UIC_TIMER if arg == TIMER_STATUS => {
                if self.stream.is_some() {
                    ui.timer(TIMER_STATUS, 200);
                }
            }
            UIC_TIMER if arg == selftest::TIMER => {
                if let Some(ms) = self.selftest.as_mut().and_then(|t| t.step()) {
                    ui.timer(selftest::TIMER, ms);
                }
            }
            _ => return false,
        }
        true
    }
}

ui::main!(audiotest());

fn audiotest() -> i32 {
    if let Some(name) = plat::arg_value("--test") {
        return tests::run(&name);
    }
    ui::launch(&DESC, Audiotest::default(), &style::STYLE)
}
