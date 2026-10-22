// sound -- the system's sounds from the UI (plat::audio::sound)
//
//   beep(SOUND_*) -> bool    the system's sound for an event (plat::audio::SOUND_INFO,
//                            SOUND_WARNING, SOUND_ERROR, SOUND_QUESTION, SOUND_NOTIFY,
//                            SOUND_DEFAULT) unless sounds are off -> false: nothing played
//   dialog_sound(SOUND_*)    right after dialog_begin returned true: the sound once, when the
//                            dialog has just opened, where the platform's message dialogs play
//                            one (plat::audio::dialog_sounds: Windows like MessageBox, Linux
//                            desktops while event sounds are on; not Android, not the browser)
//   set_sounds(on), sounds() the app's own switch (default on; an app setting "Klänge");
//                            jfgui_SOUNDS=0 turns them off for the process
//
// An app whose PERMISSIONS lack `audio.output` plays nothing: beep is refused like any other
// access (an ask line asks, "PERMDENY audio.output" once, plat::policy::denials), dialog_sound
// stays silent without a question or a record (plat::policy::audio_granted).
//
// Headless runs (scripts, --shot) play nothing: plat::audio's dry run prints "sound: NAME", and
// the script command `expect sound KIND|none` checks the last sound requested (auto.rs).

use plat::audio;

use super::Ui;

/// the switch and the last request (scripts)
#[derive(Default)]
pub(crate) struct S {
    off: bool,
    pub(crate) last: Option<u32>,
}

impl Ui {
    /// the system's sound for kind (plat::audio::SOUND_*) -> false: off, unknown, not
    /// permitted (`audio.output`) or not played
    pub fn beep(&mut self, kind: u32) -> bool {
        if !self.sounds() || audio::sound_name(kind).is_empty() || !plat::policy::audio_allowed() {
            return false;
        }
        self.m.sound.last = Some(kind);
        audio::set_dry_run(self.headless);
        audio::sound(kind)
    }

    /// inside an open content dialog: its sound, once when it has just opened, where message
    /// dialogs play one on this platform and the app may play sound -> whether a sound was
    /// asked for
    pub fn dialog_sound(&mut self, kind: u32) -> bool {
        let c = &mut self.wp.chrome;
        if !c.dlg_active || !c.dlg_fresh {
            return false;
        }
        c.dlg_fresh = false;
        audio::dialog_sounds() && plat::policy::audio_granted() && self.beep(kind)
    }

    /// the app's switch for its sounds (beep, dialog_sound); on by default
    pub fn set_sounds(&mut self, on: bool) {
        self.m.sound.off = !on;
    }

    /// sounds are on: the app's switch and jfgui_SOUNDS (0: off)
    pub fn sounds(&self) -> bool {
        !self.m.sound.off && !plat::env("jfgui_SOUNDS").is_some_and(|v| v.starts_with('0'))
    }
}
