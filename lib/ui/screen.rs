// screen -- the monitors, global shortcuts, a colour from anywhere on the screen and pictures of
// the screen for the app (plat::screen, plat::hotkey)
//
//   monitors() -> Vec<Monitor>       every monitor, the primary one first (rect / work area in
//                                    desktop units, dpi, refresh, name); a change comes as
//                                    App::command(UIC_SETTINGS, plat::SET_MONITORS)
//   hotkey_register(id, chord, description) -> Result<(), HotkeyError>   a global shortcut
//                                    ("Ctrl+Alt+K"; plat::Chord::parse names the keys): when it
//                                    is pressed anywhere, App::command(UIC_HOTKEY, id); when the
//                                    desktop drops it later (the portal's dialog),
//                                    App::command(UIC_HOTKEY_LOST, id) and hotkey_error(id)
//                                    tells why.  Needs `input.global` in the manifest.
//   hotkey_unregister(id) -> bool    hotkeys_available() -> bool
//   pick_color() -> bool             the user clicks a pixel anywhere on the screen (Esc
//                                    cancels); then App::command(UIC_PICK, 1 picked / 0 not) and
//                                    picked_color() -> Some(0xRRGGBB); false: not possible here
//   pick_available() -> bool         a colour picker with CP_PIPETTE shows its pipette then
//   screenshot(rect) -> bool         a picture of the whole desktop (None) or a rectangle; then
//                                    App::command(UIC_SCREENSHOT, 1 / 0) and screenshot_take() ->
//                                    Option<Surface> (FMT_XRGB); needs `screen.capture`
//   screen_error() -> Option<ScreenError>   why the last pick / picture failed
//
// Headless runs (scripts, --shot) touch nothing real: shortcuts are only recorded (the script
// command `system hotkey ID` presses one), a pick waits for `system pick RRGGBB|cancel`, the
// monitors are the window (or the script's `screen` rectangle) at the run's dpi, pictures are
// Unavailable.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Surface, FMT_XRGB};
use plat::hotkey::{self, Chord, HotkeyError, HotkeyEvent};
use plat::screen::{self, Monitor, ScreenError, ScreenEvent, Shot};

use super::consts::*;
use super::Ui;

/// the state of the screen services
#[derive(Default)]
pub(crate) struct S {
    /// a pick is running; the colour picker that asked (0: the app itself)
    pub(crate) picking: bool,
    pub(crate) pick_for: u32,
    /// the last picked colour (taken by picked_color / the colour picker)
    pub(crate) picked: Option<u32>,
    /// a picked colour for the colour picker pick_for, until its next pass applies it
    pub(crate) pending: Option<(u32, u32)>,
    shot: Option<Surface>,
    error: Option<ScreenError>,
    /// why shortcuts were dropped: (id, why)
    lost: Vec<(u32, HotkeyError)>,
}

// the B, G, R, x rows of a shot (or its PNG) as a surface
fn surface_of(s: &Shot) -> Option<Surface> {
    if let Some((_, data)) = &s.file {
        let dec = img_core::ImgDec { flags: 0, frame: 0, hint: 0, maxpx: 0, maxmem: 0, cancel: None };
        return img_core::io::decode_ex(data, Some(&dec), None).ok();
    }
    if s.w <= 0 || s.h <= 0 || s.px.len() < (s.w * s.h * 4) as usize {
        return None;
    }
    let mut sf = Surface::new(s.w, s.h, FMT_XRGB);
    if sf.px.is_empty() {
        return None;
    }
    let row = (s.w * 4) as usize;
    for y in 0..s.h as usize {
        let o = y * sf.stride as usize;
        sf.px[o..o + row].copy_from_slice(&s.px[y * row..y * row + row]);
    }
    Some(sf)
}

impl Ui {
    /// every monitor, the primary one first
    pub fn monitors(&mut self) -> Vec<Monitor> {
        if !self.headless {
            return screen::monitors();
        }
        // headless: the window (or the script's virtual screen) as one monitor
        let s = self.m.win.screen;
        let r = if s[2] > 0 && s[3] > 0 { [self.dp(s[0]), self.dp(s[1]), self.dp(s[2]), self.dp(s[3])] } else { [0, 0, self.wp.rt.fb.w, self.wp.rt.fb.h] };
        alloc::vec![Monitor { id: 0, name: String::from("headless"), rect: r, work: r, dpi: self.wp.rt.dpi.max(1) as u32, refresh_mhz: 60000, primary: true }]
    }

    /// whether global shortcuts work here
    pub fn hotkeys_available(&mut self) -> bool {
        hotkey::set_dry_run(self.headless);
        hotkey::hotkey_available()
    }

    /// register chord ("Ctrl+Alt+K") as the global shortcut id; description tells the user
    /// what it does (the desktop's dialog shows it)
    pub fn hotkey_register(&mut self, id: u32, chord: &str, description: &str) -> Result<(), HotkeyError> {
        hotkey::set_dry_run(self.headless);
        let c = Chord::parse(chord).ok_or(HotkeyError::Invalid)?;
        self.m.screen.lost.retain(|e| e.0 != id);
        let r = hotkey::hotkey_register(id, c, description);
        if let Err(e) = &r {
            plat::log!(plat::LOG_INFO, "app", "global shortcut {chord}: {e}");
        }
        r
    }

    /// remove the global shortcut id -> false when it was not registered
    pub fn hotkey_unregister(&mut self, id: u32) -> bool {
        hotkey::hotkey_unregister(id)
    }

    /// why the desktop dropped the shortcut id (after UIC_HOTKEY_LOST)
    pub fn hotkey_error(&self, id: u32) -> Option<HotkeyError> {
        self.m.screen.lost.iter().find(|e| e.0 == id).map(|e| e.1.clone())
    }

    /// whether the user can pick a colour anywhere on the screen here (headless: yes, the
    /// script answers)
    pub fn pick_available(&self) -> bool {
        self.headless || screen::pick_available()
    }

    /// the user picks a pixel anywhere on the screen; UIC_PICK reports the end -> false when
    /// that is not possible here (or a pick runs)
    pub fn pick_color(&mut self) -> bool {
        self.pick_start_for(0)
    }

    // a pick for colour picker `owner` (0: the app)
    pub(crate) fn pick_start_for(&mut self, owner: u32) -> bool {
        if self.m.screen.picking {
            return false;
        }
        self.m.screen.error = None;
        if !self.headless {
            if let Err(e) = screen::pick_start() {
                self.m.screen.error = Some(e);
                return false;
            }
        }
        self.m.screen.picking = true;
        self.m.screen.pick_for = owner;
        true
    }

    /// the colour of the last pick (0xRRGGBB), once
    pub fn picked_color(&mut self) -> Option<u32> {
        self.m.screen.picked.take()
    }

    /// a picture of the whole desktop (None) or of a rectangle (desktop units); UIC_SCREENSHOT
    /// reports it -> false when that is not possible here
    pub fn screenshot(&mut self, rect: Option<[i32; 4]>) -> bool {
        self.m.screen.error = None;
        let r = if self.headless { Err(ScreenError::Unavailable) } else { screen::capture_start(rect) };
        match r {
            Ok(()) => true,
            Err(e) => {
                self.m.screen.error = Some(e);
                false
            }
        }
    }

    /// the picture of the last screenshot, once
    pub fn screenshot_take(&mut self) -> Option<Surface> {
        self.m.screen.shot.take()
    }

    /// why the last pick or picture failed (None: it worked or was not asked for)
    pub fn screen_error(&self) -> Option<ScreenError> {
        self.m.screen.error.clone()
    }

    // a pick ended: remembered for its colour picker / the app, UIC_PICK
    pub(crate) fn pick_done(&mut self, r: Result<u32, ScreenError>) {
        let s = &mut self.m.screen;
        if !s.picking {
            return;
        }
        s.picking = false;
        match r {
            Ok(c) => {
                if s.pick_for != 0 {
                    s.pending = Some((s.pick_for, c));
                } else {
                    s.picked = Some(c);
                }
            }
            Err(e) => s.error = Some(e),
        }
        let ok = self.m.screen.pending.is_some() || self.m.screen.picked.is_some();
        if self.m.screen.pick_for == 0 {
            self.cmd_run(UIC_PICK, ok as u32);
        }
        self.repass();
    }

    /// (E_WAKE) shortcuts pressed or dropped, picks and pictures finished -> App::command
    pub(crate) fn screen_poll(&mut self) {
        for e in hotkey::hotkey_take() {
            match e {
                HotkeyEvent::Pressed(id) => self.cmd_run(UIC_HOTKEY, id),
                HotkeyEvent::Refused(id, why) => {
                    plat::log!(plat::LOG_INFO, "app", "global shortcut {id} dropped: {why}");
                    self.m.screen.lost.retain(|e| e.0 != id);
                    self.m.screen.lost.push((id, why));
                    self.cmd_run(UIC_HOTKEY_LOST, id);
                }
            }
        }
        for e in screen::screen_take() {
            match e {
                ScreenEvent::Color(r) => self.pick_done(r),
                ScreenEvent::Shot(r) => {
                    let ok = match r {
                        Ok(s) => {
                            self.m.screen.shot = surface_of(&s);
                            if self.m.screen.shot.is_none() {
                                self.m.screen.error = Some(ScreenError::Failed(String::from("the picture could not be read")));
                            }
                            self.m.screen.shot.is_some()
                        }
                        Err(e) => {
                            self.m.screen.error = Some(e);
                            false
                        }
                    };
                    self.cmd_run(UIC_SCREENSHOT, ok as u32);
                }
            }
        }
    }
}
