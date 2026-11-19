// input -- named actions bound to keys by where they sit, mouse buttons, the wheel, the raw motion of
// the locked pointer and game pads; sensitivity, inversion and a factor per platform for the motion; the
// pointer lock's click and its end; the pad's dead zone, turn rate and curve; bindings and settings in a
// settings file
//
//   Source            where an input comes from: Key(KC_*: the key's place, KeyEvent::code, never its
//                     character), Mouse(MOUSE_*), WheelUp / WheelDown (a notch as a press), MouseX /
//                     MouseY (raw motion), Wheel (the turn in notches), PadButton(plat::PAD_*), PadAxis
//                     (a stick's axis), PadDir (a stick's direction as a button); name / parse for the
//                     file, label for a menu (plat::key_label: what the layout prints on the key;
//                     label_for: a pad's labels by its style)
//   ActionMap         actions (name, ActionKind, bindings and their defaults); store / load in an Ini
//   ActionKind        Button (down, pressed, released), Axis (keys with a scale each, a pad's analog
//                     values, -1 ..= 1), Motion (raw motion: an angle in radians, the wheel in notches,
//                     a stick turning at a rate)
//   MouseSettings     sensitivity, invert_x, invert_y, factor (None: the platform's,
//                     platform_motion_factor); store / load in an Ini
//   PadSettings       sensitivity (of TURN_RATE), dead_zone (radial), curve, invert_x, invert_y; store /
//                     load in the same section
//   Input             takes the events (key, button, buttons, motion, wheel, set_lock, plat_event) and
//                     the pads' snapshots (pad_frame) and answers per frame (down, pressed, released,
//                     axis, motion; end_frame) and per tick (tick() -> TickInput); the lock:
//                     take_lock_request, playing, take_pause; the pad: active_pad, pin_pad, device
//
// The pointer lock (docs/GAME.md, the user's decisions of 2026-11-18).  While the game needs the lock
// (require_lock, the default) and does not hold it, no action fires: a mouse press asks for the lock
// (take_lock_request -> the app calls ui.pointer_lock(true) in the same frame: the browser grants it only
// from a press) and is swallowed with its release, so the click that starts playing never fires.  Esc
// belongs to the lock and never reaches the game: the game only sees the lock end (set_lock with
// ui.pointer_lock_state(), or E_POINTER_LOCK) and takes that as its pause (take_pause): every held input
// is let go, the actions report their release.  A platform without a lock (Unsupported: phones) plays
// without one.
//
// Motion units.  ui.pointer_motion / plat::win_raw_motion_take report what each platform has: the
// mouse's counts (Windows, X11), libinput's unaccelerated motion normalised to 1000 dpi (Wayland),
// points after the system's acceleration (macOS), raw counts with unadjustedMovement or CSS pixels after
// the acceleration (the browser).  The reference unit is a count of a 1000-dpi mouse; the factor of a
// platform (platform_motion_factor) turns its unit into that, MouseSettings::factor overrides it.  A
// Motion action's value is then counts x factor x sensitivity x DEGREES_PER_COUNT (0.022 degrees: Quake's
// and Source's m_yaw, so sensitivities read as in those games) in radians, the sign flipped by invert_x /
// invert_y; a wheel's motion is in notches (no sensitivity).
//
// Frames and ticks.  Events come with the frames, the simulation runs in ticks.  Every action counts
// its presses and releases and sums its motion twice: for the frame (end_frame clears them: the camera
// turns per frame) and for the tick (tick() takes them: a press between two ticks reaches the next tick
// even when the frame had none, a press and a release between two ticks make one tick with down,
// pressed and released).  An Axis reads the keys held at the moment it is asked.
//
// Game pads (G7a part 2).  pad_frame takes the pads of a frame (ui.gamepads(), plat::pads()) and the
// time since the frame before.  One pad drives a single player's input: the one used last (a button
// pressed or a stick out of its dead zone; the first such pad, and any other that is used, takes over),
// or the one pin_pad names (a split screen: an Input per pad).  Its buttons press and release like keys
// (a tap between two frames is a press and a release: the snapshots carry their edges); its sticks go
// through a radial dead zone (PadSettings::dead_zone: nearer the middle is rest, the rest of the way
// scales to 0 ..= 1); an Axis adds a stick's value (or a trigger's) times the binding's scale to its
// keys', a Motion turns by TURN_RATE x sensitivity degrees a second at full deflection, the deflection
// raised to PadSettings::curve (fine aim near the middle), times the frame's time.  A pad plays without
// the pointer lock: a pad button pressed while the game does not play starts playing (pad play) and is
// swallowed with its release, as the lock's click; the mouse still asks for the lock with its first
// press, the lock taken ends pad play, its end pauses as before.  The active pad going away during pad
// play pauses.  After a pause a pad button held counts only once it was let go.  device() says what was
// used last (prompts for keys or for a pad).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::Ini;
use plat::{Gamepad, PAD_AXES, PAD_BUTTONS, PAD_MAX, PadStyle, PointerLock};
// the portable x^y, the same bits on every target (in the browser rt's x87 module is that code)
#[cfg(not(target_arch = "wasm32"))]
use rt::portable::math::pow;
#[cfg(target_arch = "wasm32")]
use rt::x87::pow;

/// the left mouse button (plat::BTN_LEFT, ui::BTN_LEFT)
pub const MOUSE_LEFT: u8 = 1;
/// the right mouse button
pub const MOUSE_RIGHT: u8 = 2;
/// the middle mouse button (the wheel pressed)
pub const MOUSE_MIDDLE: u8 = 3;
/// the first side button (back)
pub const MOUSE_X1: u8 = 4;
/// the second side button (forward)
pub const MOUSE_X2: u8 = 5;
/// the highest mouse button an input keeps
const MOUSE_MAX: u8 = 7;

/// the angle one count of the reference mouse turns at sensitivity 1 (degrees): Quake's and Source's
/// m_yaw, so that sensitivities read as in those games
pub const DEGREES_PER_COUNT: f64 = 0.022;

/// the factor of Windows' motion: the mouse's counts (Raw Input) are the reference unit
pub const FACTOR_WINDOWS: f64 = 1.0;
/// the factor of Linux' motion: X11's raw counts and Wayland's 1000-dpi normalised motion are the
/// reference unit
pub const FACTOR_LINUX: f64 = 1.0;
/// the factor of macOS' motion: points after the system's acceleration.  At a moderate speed and the
/// default tracking speed a 1000-dpi mouse makes about 0.4 points per count (the system takes a mouse for
/// 400 dpi and moves the pointer 96 points per inch at gain 1, about 1.7 at that speed): an estimate,
/// checked by hand in G7b part 2
pub const FACTOR_MACOS: f64 = 2.5;
/// the factor of the browser's motion: raw counts with unadjustedMovement (Chrome on Windows and
/// macOS), else CSS pixels after the system's acceleration, about one per count at a moderate speed
pub const FACTOR_BROWSER: f64 = 1.0;
/// the factor where the pointer cannot be locked (Android, iOS): no motion comes
pub const FACTOR_OTHER: f64 = 1.0;

/// the degrees per second a stick turns at full deflection and pad sensitivity 1 (PadSettings)
pub const TURN_RATE: f64 = 180.0;
/// a stick's direction (Source::PadDir) is held from this far on and let go below PAD_DIR_RELEASE
pub const PAD_DIR_PRESS: f64 = 0.5;
/// below this a held stick direction is let go
pub const PAD_DIR_RELEASE: f64 = 0.4;

/// the names of the sticks' axes in a settings file (Source::PadAxis)
const PAD_AXIS_NAMES: [&str; PAD_AXES] = ["PadLeftX", "PadLeftY", "PadRightX", "PadRightY"];

/// the factor that turns this platform's raw motion into counts of a 1000-dpi mouse (the FACTOR_*
/// constants)
pub const fn platform_motion_factor() -> f64 {
    if cfg!(target_arch = "wasm32") {
        FACTOR_BROWSER
    } else if cfg!(windows) {
        FACTOR_WINDOWS
    } else if cfg!(target_os = "macos") {
        FACTOR_MACOS
    } else if cfg!(target_os = "linux") {
        FACTOR_LINUX
    } else {
        FACTOR_OTHER
    }
}

/// where an input comes from
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Source {
    /// a key by where it sits: a USB HID usage, plat::KC_* (KeyEvent::code), whatever the layout prints
    Key(u32),
    /// a mouse button, MOUSE_LEFT ..= MOUSE_X2 (up to 7)
    Mouse(u8),
    /// a notch of the wheel up: a press and a release at once
    WheelUp,
    /// a notch of the wheel down
    WheelDown,
    /// the raw motion of the locked pointer to the right
    MouseX,
    /// the raw motion of the locked pointer downwards
    MouseY,
    /// the wheel's turn in notches, up positive
    Wheel,
    /// a pad's button of the standard layout (plat::PAD_A ..= plat::PAD_HOME): pressed and released; in
    /// an Axis its analog value (a trigger's 0 ..= 1) times the scale
    PadButton(u8),
    /// a pad's stick axis (plat::PAD_LX, PAD_LY, PAD_RX, PAD_RY: right and down positive) after the radial
    /// dead zone: in an Axis its value -1 ..= 1 times the scale, in a Motion it turns (looking: TURN_RATE,
    /// PadSettings)
    PadAxis(u8),
    /// one direction of a stick axis as a button (true: right / down), held from PAD_DIR_PRESS on; in an
    /// Axis its value 0 ..= 1
    PadDir(u8, bool),
}

impl Source {
    /// a motion (MouseX, MouseY, Wheel): for Motion actions; the others are pressed and released
    pub fn is_motion(self) -> bool {
        matches!(self, Source::MouseX | Source::MouseY | Source::Wheel)
    }

    /// a pad's (PadButton, PadAxis, PadDir)
    pub fn is_pad(self) -> bool {
        matches!(self, Source::PadButton(_) | Source::PadAxis(_) | Source::PadDir(..))
    }

    /// the name in a settings file, the same on every platform: a key's KeyboardEvent.code name ("KeyW",
    /// "ShiftLeft"; "Code0x99" without one), "Mouse1" .. "Mouse7", "WheelUp", "WheelDown", "MouseX",
    /// "MouseY", "Wheel"; a pad's "PadA" .. "PadHome" (plat::pad_button_name), "PadLeftX", "PadLeftY",
    /// "PadRightX", "PadRightY", a direction "PadLeftX+" (right / down) or "PadLeftX-"
    pub fn name(self) -> String {
        match self {
            Source::Key(code) => match plat::key_code_name(code) {
                "" => format!("Code0x{code:02X}"),
                n => String::from(n),
            },
            Source::Mouse(b) => format!("Mouse{b}"),
            Source::WheelUp => String::from("WheelUp"),
            Source::WheelDown => String::from("WheelDown"),
            Source::MouseX => String::from("MouseX"),
            Source::MouseY => String::from("MouseY"),
            Source::Wheel => String::from("Wheel"),
            Source::PadButton(b) => format!("Pad{}", plat::pad_button_name(b as usize)),
            Source::PadAxis(a) => String::from(PAD_AXIS_NAMES.get(a as usize).copied().unwrap_or("PadAxis?")),
            Source::PadDir(a, pos) => format!("{}{}", PAD_AXIS_NAMES.get(a as usize).copied().unwrap_or("PadAxis?"), if pos { '+' } else { '-' }),
        }
    }

    /// the source of a name (Source::name); None: unknown
    pub fn parse(s: &str) -> Option<Source> {
        match s {
            "WheelUp" => return Some(Source::WheelUp),
            "WheelDown" => return Some(Source::WheelDown),
            "MouseX" => return Some(Source::MouseX),
            "MouseY" => return Some(Source::MouseY),
            "Wheel" => return Some(Source::Wheel),
            _ => {}
        }
        if let Some(rest) = s.strip_prefix("Pad") {
            for (a, n) in PAD_AXIS_NAMES.iter().enumerate() {
                if let Some(dir) = s.strip_prefix(n) {
                    return match dir {
                        "" => Some(Source::PadAxis(a as u8)),
                        "+" => Some(Source::PadDir(a as u8, true)),
                        "-" => Some(Source::PadDir(a as u8, false)),
                        _ => None,
                    };
                }
            }
            return (0..PAD_BUTTONS).find(|&b| plat::pad_button_name(b) == rest).map(|b| Source::PadButton(b as u8));
        }
        if let Some(n) = s.strip_prefix("Mouse") {
            return match n.parse::<u8>() {
                Ok(b) if (1..=MOUSE_MAX).contains(&b) => Some(Source::Mouse(b)),
                _ => None,
            };
        }
        if let Some(h) = s.strip_prefix("Code0x") {
            return u32::from_str_radix(h, 16).ok().filter(|c| (1..=0xFF).contains(c)).map(Source::Key);
        }
        match plat::key_code_from_name(s) {
            0 => None,
            code => Some(Source::Key(code)),
        }
    }

    /// what a menu shows: for a key what the current layout prints on it (plat::key_label: "Z" for KC_W on
    /// a French keyboard), where the platform does not say (the browser, phones) the name without "Key" /
    /// "Digit"; for the mouse and the pads German texts an app translates with its catalogue
    /// (ui::i18n::t); a pad's buttons as on an Xbox pad (label_for: by the pad's style)
    pub fn label(self) -> String {
        self.label_for(PadStyle::Xbox)
    }

    /// label, a pad's buttons as a pad of that style prints them (plat::Gamepad::style): Xbox (also for
    /// pads of no known style) A B X Y, LB RB LT RT, Ansicht, Menü; PlayStation Kreuz, Kreis, Quadrat,
    /// Dreieck, L1 R1 L2 R2, Create, Options, L3 R3; Nintendo by the places B A Y X, L R ZL ZR, Minus,
    /// Plus
    pub fn label_for(self, style: PadStyle) -> String {
        match self {
            Source::Key(code) => plat::key_label(code).filter(|l| !l.is_empty()).unwrap_or_else(|| {
                let n = self.name();
                match n.strip_prefix("Key").or_else(|| n.strip_prefix("Digit")) {
                    Some(rest) if !rest.is_empty() => String::from(rest),
                    _ => n,
                }
            }),
            Source::Mouse(b) => String::from(match b {
                MOUSE_LEFT => "Linke Maustaste",
                MOUSE_RIGHT => "Rechte Maustaste",
                MOUSE_MIDDLE => "Mittlere Maustaste",
                MOUSE_X1 => "Maustaste 4",
                MOUSE_X2 => "Maustaste 5",
                _ => return format!("Maustaste {b}"),
            }),
            Source::WheelUp => String::from("Mausrad hoch"),
            Source::WheelDown => String::from("Mausrad runter"),
            Source::MouseX => String::from("Maus waagrecht"),
            Source::MouseY => String::from("Maus senkrecht"),
            Source::Wheel => String::from("Mausrad"),
            Source::PadButton(b) => String::from(pad_button_label(b as usize, style)),
            Source::PadAxis(a) => String::from(match a as usize {
                0 => "Linker Stick waagrecht",
                1 => "Linker Stick senkrecht",
                2 => "Rechter Stick waagrecht",
                3 => "Rechter Stick senkrecht",
                _ => "Stick",
            }),
            Source::PadDir(a, pos) => String::from(match (a as usize, pos) {
                (0, true) => "Linker Stick rechts",
                (0, false) => "Linker Stick links",
                (1, true) => "Linker Stick runter",
                (1, false) => "Linker Stick hoch",
                (2, true) => "Rechter Stick rechts",
                (2, false) => "Rechter Stick links",
                (3, true) => "Rechter Stick runter",
                (3, false) => "Rechter Stick hoch",
                _ => "Stick",
            }),
        }
    }
}

/// a pad button's label by the pad's style (Source::label_for)
fn pad_button_label(b: usize, style: PadStyle) -> &'static str {
    const XBOX: [&str; PAD_BUTTONS] = ["A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Ansicht", "Menü", "Linker Stick (drücken)", "Rechter Stick (drücken)", "Steuerkreuz hoch",
                                       "Steuerkreuz runter", "Steuerkreuz links", "Steuerkreuz rechts", "Xbox-Taste"];
    const PS: [&str; PAD_BUTTONS] = ["Kreuz", "Kreis", "Quadrat", "Dreieck", "L1", "R1", "L2", "R2", "Create", "Options", "L3", "R3", "Steuerkreuz hoch", "Steuerkreuz runter",
                                     "Steuerkreuz links", "Steuerkreuz rechts", "PS-Taste"];
    const NIN: [&str; PAD_BUTTONS] = ["B", "A", "Y", "X", "L", "R", "ZL", "ZR", "Minus", "Plus", "Linker Stick (drücken)", "Rechter Stick (drücken)", "Steuerkreuz hoch",
                                      "Steuerkreuz runter", "Steuerkreuz links", "Steuerkreuz rechts", "Home"];
    let t = match style {
        PadStyle::PlayStation => &PS,
        PadStyle::Nintendo => &NIN,
        _ => &XBOX,
    };
    t.get(b).copied().unwrap_or("Pad-Knopf")
}

/// a binding: a source and its scale (an Axis: +1 / -1 for the two directions; a Motion: a multiplier,
/// -1 turns it round)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bind {
    /// where the input comes from
    pub source: Source,
    /// what the source counts for
    pub scale: f64,
}

impl Bind {
    /// the source with scale 1
    pub const fn new(source: Source) -> Bind {
        Bind { source, scale: 1.0 }
    }

    /// the source with a scale
    pub const fn scaled(source: Source, scale: f64) -> Bind {
        Bind { source, scale }
    }

    /// a key by its place (plat::KC_*) with scale 1
    pub const fn key(code: u32) -> Bind {
        Bind::new(Source::Key(code))
    }

    /// the text of the binding in a settings file: the source's name, "-" before it for scale -1,
    /// "*SCALE" after it for another scale ("KeyD", "-KeyA", "MouseY*0.5")
    pub fn text(&self) -> String {
        let n = self.source.name();
        if self.scale == 1.0 {
            n
        } else if self.scale == -1.0 {
            format!("-{n}")
        } else {
            format!("{n}*{}", base::f64_format(self.scale))
        }
    }

    /// the binding of a text (Bind::text); None: not readable
    pub fn parse(s: &str) -> Option<Bind> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (name, scale) = match s.split_once('*') {
            Some((n, k)) => {
                let (v, used) = base::f64_parse(k.as_bytes());
                if used == 0 || used != k.len() || !v.is_finite() {
                    return None;
                }
                (n, v)
            }
            None => (s, 1.0),
        };
        Some(Bind { source: Source::parse(name)?, scale: if neg { -scale } else { scale } })
    }
}

/// what an action reports
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionKind {
    /// pressed and released (fire, jump, use): sources that press (keys, mouse buttons, wheel notches)
    Button,
    /// a value in -1 ..= 1 from the sources held, each with its scale (walking forward and back from W
    /// and S), a pad's sticks and triggers with their analog values times the scale
    Axis,
    /// raw motion summed (looking around): MouseX, MouseY (radians), Wheel (notches), a stick's axis
    /// (PadAxis: radians, turning at a rate)
    Motion,
}

impl ActionKind {
    /// a source this kind takes: a Button what presses (keys, mouse buttons, wheel notches, pad buttons and
    /// stick directions), an Axis those and a stick's axis, a Motion the mouse's motion, the wheel and a
    /// stick's axis
    pub fn takes(self, s: Source) -> bool {
        match self {
            ActionKind::Button => !s.is_motion() && !matches!(s, Source::PadAxis(_)),
            ActionKind::Axis => !s.is_motion(),
            ActionKind::Motion => s.is_motion() || matches!(s, Source::PadAxis(_)),
        }
    }
}

/// an action of an ActionMap (its index)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(u16);

impl ActionId {
    /// the index in the map (and in TickInput)
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug)]
struct Action {
    name: String,
    kind: ActionKind,
    binds: Vec<Bind>,
    defaults: Vec<Bind>,
}

/// the actions of a game and their bindings
#[derive(Clone, Debug, Default)]
pub struct ActionMap {
    actions: Vec<Action>,
}

/// a name an Ini key can be and a settings page can show: ASCII letters, digits, '_', '-', '.'
fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

impl ActionMap {
    /// no actions
    pub const fn new() -> ActionMap {
        ActionMap { actions: Vec::new() }
    }

    /// a new action with its default bindings (bindings of a source the kind does not take are left out).
    /// Panics for a name used before or one that is not ASCII letters, digits, '_', '-', '.' (at most 64)
    pub fn add(&mut self, name: &str, kind: ActionKind, defaults: &[Bind]) -> ActionId {
        assert!(name_ok(name), "ActionMap::add: {name:?} is no action name (letters, digits, _ - .)");
        assert!(self.find(name).is_none(), "ActionMap::add: {name:?} twice");
        assert!(self.actions.len() < u16::MAX as usize, "ActionMap::add: too many actions");
        let mut binds: Vec<Bind> = Vec::new();
        for b in defaults {
            if kind.takes(b.source) && !binds.iter().any(|x| x.source == b.source) {
                binds.push(*b);
            }
        }
        self.actions.push(Action { name: String::from(name), kind, defaults: binds.clone(), binds });
        ActionId((self.actions.len() - 1) as u16)
    }

    /// actions
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// none
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// every action in order
    pub fn ids(&self) -> impl Iterator<Item = ActionId> {
        (0..self.actions.len() as u16).map(ActionId)
    }

    /// the action of that name
    pub fn find(&self, name: &str) -> Option<ActionId> {
        self.actions.iter().position(|a| a.name == name).map(|i| ActionId(i as u16))
    }

    /// its name
    pub fn name(&self, a: ActionId) -> &str {
        &self.actions[a.index()].name
    }

    /// its kind
    pub fn kind(&self, a: ActionId) -> ActionKind {
        self.actions[a.index()].kind
    }

    /// its bindings now
    pub fn binds(&self, a: ActionId) -> &[Bind] {
        &self.actions[a.index()].binds
    }

    /// its default bindings
    pub fn defaults(&self, a: ActionId) -> &[Bind] {
        &self.actions[a.index()].defaults
    }

    /// one more binding (false: the kind does not take the source, or the action has it already)
    pub fn bind(&mut self, a: ActionId, b: Bind) -> bool {
        let act = &mut self.actions[a.index()];
        if !act.kind.takes(b.source) || act.binds.iter().any(|x| x.source == b.source) {
            return false;
        }
        act.binds.push(b);
        true
    }

    /// the binding of that source goes (false: none)
    pub fn unbind(&mut self, a: ActionId, s: Source) -> bool {
        let act = &mut self.actions[a.index()];
        let n = act.binds.len();
        act.binds.retain(|b| b.source != s);
        act.binds.len() != n
    }

    /// these bindings instead (false: one of them the kind does not take or one source twice; nothing
    /// changes then)
    pub fn set_binds(&mut self, a: ActionId, binds: &[Bind]) -> bool {
        let kind = self.actions[a.index()].kind;
        let twice = binds.iter().enumerate().any(|(i, b)| binds[..i].iter().any(|x| x.source == b.source));
        if twice || binds.iter().any(|b| !kind.takes(b.source)) {
            return false;
        }
        self.actions[a.index()].binds = binds.to_vec();
        true
    }

    /// its default bindings again
    pub fn reset(&mut self, a: ActionId) {
        let act = &mut self.actions[a.index()];
        act.binds = act.defaults.clone();
    }

    /// every action's default bindings again
    pub fn reset_all(&mut self) {
        for act in &mut self.actions {
            act.binds = act.defaults.clone();
        }
    }

    /// the actions a source drives (two actions on one key: a settings page shows the clash)
    pub fn users(&self, s: Source) -> Vec<ActionId> {
        self.ids().filter(|&a| self.binds(a).iter().any(|b| b.source == s)).collect()
    }

    /// the bindings into section `section` of a settings file: a line "bind.NAME=..." (the bindings'
    /// texts separated by blanks, "none" for none) for every action whose bindings differ from its
    /// defaults; the line of one that has its defaults goes (a new default reaches who never changed it)
    pub fn store(&self, ini: &mut Ini, section: &str) {
        for act in &self.actions {
            let key = format!("bind.{}", act.name);
            if act.binds == act.defaults {
                ini.remove(section, &key);
            } else if act.binds.is_empty() {
                ini.set(section, &key, "none");
            } else {
                let texts: Vec<String> = act.binds.iter().map(|b| b.text()).collect();
                ini.set(section, &key, &texts.join(" "));
            }
        }
    }

    /// the bindings from section `section` of a settings file (Self::store): an action without a line
    /// gets its defaults -> the bindings that could not be read (an unknown source, a source the kind does
    /// not take, a source twice): left out, the rest of the line counts
    pub fn load(&mut self, ini: &Ini, section: &str) -> usize {
        let mut bad = 0;
        for act in &mut self.actions {
            let key = format!("bind.{}", act.name);
            let Some(line) = ini.get(section, &key) else {
                act.binds = act.defaults.clone();
                continue;
            };
            let mut binds: Vec<Bind> = Vec::new();
            for tok in line.split_ascii_whitespace() {
                if tok == "none" {
                    continue;
                }
                match Bind::parse(tok) {
                    Some(b) if act.kind.takes(b.source) && !binds.iter().any(|x| x.source == b.source) => binds.push(b),
                    _ => bad += 1,
                }
            }
            act.binds = binds;
        }
        bad
    }
}

/// how the raw motion of the mouse turns into angles
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseSettings {
    /// the multiplier of DEGREES_PER_COUNT (1: 0.022 degrees a count of the reference mouse)
    pub sensitivity: f64,
    /// right turns left
    pub invert_x: bool,
    /// down looks up
    pub invert_y: bool,
    /// the factor from this platform's units to counts of a 1000-dpi mouse; None: the platform's
    /// (platform_motion_factor)
    pub factor: Option<f64>,
}

impl Default for MouseSettings {
    fn default() -> Self {
        MouseSettings::DEFAULT
    }
}

/// the sensitivity a file may hold
const SENS_MIN: f64 = 0.001;
const SENS_MAX: f64 = 100.0;

impl MouseSettings {
    /// sensitivity 1, nothing inverted, the platform's factor
    pub const DEFAULT: MouseSettings = MouseSettings { sensitivity: 1.0, invert_x: false, invert_y: false, factor: None };

    /// the factor in use (factor, else the platform's)
    pub fn motion_factor(&self) -> f64 {
        self.factor.unwrap_or(platform_motion_factor())
    }

    /// radians per raw unit of this platform along x and y (the sign of the inversion included)
    pub fn radians_per_unit(&self) -> (f64, f64) {
        let k = self.motion_factor() * self.sensitivity * DEGREES_PER_COUNT * (core::f64::consts::PI / 180.0);
        (if self.invert_x { -k } else { k }, if self.invert_y { -k } else { k })
    }

    /// into section `section` of a settings file: "sensitivity", "invert_x", "invert_y",
    /// "motion_factor", each only when it differs from the default (else its line goes)
    pub fn store(&self, ini: &mut Ini, section: &str) {
        let d = MouseSettings::DEFAULT;
        if self.sensitivity == d.sensitivity {
            ini.remove(section, "sensitivity");
        } else {
            ini.set_f64(section, "sensitivity", self.sensitivity);
        }
        for (key, v) in [("invert_x", self.invert_x), ("invert_y", self.invert_y)] {
            if v {
                ini.set(section, key, "true");
            } else {
                ini.remove(section, key);
            }
        }
        match self.factor {
            Some(f) => ini.set_f64(section, "motion_factor", f),
            None => ini.remove(section, "motion_factor"),
        };
    }

    /// from section `section` of a settings file (Self::store): what is missing or unreadable is the
    /// default, the sensitivity is kept in 0.001 ..= 100, a factor in 0.001 ..= 1000
    pub fn load(ini: &Ini, section: &str) -> MouseSettings {
        let d = MouseSettings::DEFAULT;
        let sens = ini.get_f64(section, "sensitivity", d.sensitivity);
        let factor = ini.get_f64(section, "motion_factor", f64::NAN);
        MouseSettings {
            sensitivity: if sens.is_finite() { sens.clamp(SENS_MIN, SENS_MAX) } else { d.sensitivity },
            invert_x: ini.get_int(section, "invert_x", 0) != 0,
            invert_y: ini.get_int(section, "invert_y", 0) != 0,
            factor: factor.is_finite().then(|| factor.clamp(0.001, 1000.0)),
        }
    }
}

/// how a pad's sticks turn into values and angles (the sticks of every pad of the input)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadSettings {
    /// the multiplier of TURN_RATE (1: 180 degrees a second at full deflection)
    pub sensitivity: f64,
    /// the radial dead zone of both sticks, 0 ..= 0.9: nearer the middle a stick is at rest, the rest of
    /// its way scales to 0 ..= 1 (beyond 1, a square stick's corners: 1)
    pub dead_zone: f64,
    /// the response curve of looking, 1 ..= 4: the deflection after the dead zone to this power (1
    /// linear; 2, the default, turns slowly near the middle for fine aim and fast at the end)
    pub curve: f64,
    /// right turns left
    pub invert_x: bool,
    /// down looks up
    pub invert_y: bool,
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings::DEFAULT
    }
}

impl PadSettings {
    /// sensitivity 1, dead zone 0.15, curve 2, nothing inverted
    pub const DEFAULT: PadSettings = PadSettings { sensitivity: 1.0, dead_zone: 0.15, curve: 2.0, invert_x: false, invert_y: false };

    /// a stick (x, y as the pad reports it, -1 ..= 1) after the radial dead zone: (0, 0) up to dead_zone
    /// from the middle, beyond it the way that is left scaled to 0 ..= 1 in the same direction
    pub fn stick(&self, x: f64, y: f64) -> (f64, f64) {
        if !(x.is_finite() && y.is_finite()) {
            return (0.0, 0.0);
        }
        let dz = self.dead_zone.clamp(0.0, 0.9);
        let m = physics::math::sqrt(x * x + y * y);
        if m <= dz || m == 0.0 {
            return (0.0, 0.0);
        }
        let k = (m.min(1.0) - dz) / (1.0 - dz) / m;
        (x * k, y * k)
    }

    /// a stick's turn in radians per second (x right, y down): the dead zone, the curve on the
    /// deflection, TURN_RATE x sensitivity at full deflection, the inversion's signs
    pub fn look(&self, x: f64, y: f64) -> (f64, f64) {
        let (sx, sy) = self.stick(x, y);
        let m = physics::math::sqrt(sx * sx + sy * sy);
        if m == 0.0 {
            return (0.0, 0.0);
        }
        let c = self.curve.clamp(1.0, 4.0);
        // the deflection to the power c, kept in its direction: (s / m) * m^c = s * m^(c - 1)
        let f = if c == 1.0 {
            1.0
        } else if c == 2.0 {
            m
        } else {
            pow(m, c - 1.0)
        };
        let rate = TURN_RATE * self.sensitivity * (core::f64::consts::PI / 180.0);
        let k = f * rate;
        (if self.invert_x { -sx * k } else { sx * k }, if self.invert_y { -sy * k } else { sy * k })
    }

    /// into section `section` of a settings file: "pad_sensitivity", "pad_dead_zone", "pad_curve",
    /// "pad_invert_x", "pad_invert_y", each only when it differs from the default (else its line goes)
    pub fn store(&self, ini: &mut Ini, section: &str) {
        let d = PadSettings::DEFAULT;
        for (key, v, dv) in [("pad_sensitivity", self.sensitivity, d.sensitivity), ("pad_dead_zone", self.dead_zone, d.dead_zone), ("pad_curve", self.curve, d.curve)] {
            if v == dv {
                ini.remove(section, key);
            } else {
                ini.set_f64(section, key, v);
            }
        }
        for (key, v) in [("pad_invert_x", self.invert_x), ("pad_invert_y", self.invert_y)] {
            if v {
                ini.set(section, key, "true");
            } else {
                ini.remove(section, key);
            }
        }
    }

    /// from section `section` (Self::store): what is missing or unreadable is the default; the
    /// sensitivity kept in 0.001 ..= 100, the dead zone in 0 ..= 0.9, the curve in 1 ..= 4
    pub fn load(ini: &Ini, section: &str) -> PadSettings {
        let d = PadSettings::DEFAULT;
        let get = |key: &str, dv: f64, lo: f64, hi: f64| {
            let v = ini.get_f64(section, key, dv);
            if v.is_finite() { v.clamp(lo, hi) } else { dv }
        };
        PadSettings {
            sensitivity: get("pad_sensitivity", d.sensitivity, SENS_MIN, SENS_MAX),
            dead_zone: get("pad_dead_zone", d.dead_zone, 0.0, 0.9),
            curve: get("pad_curve", d.curve, 1.0, 4.0),
            invert_x: ini.get_int(section, "pad_invert_x", 0) != 0,
            invert_y: ini.get_int(section, "pad_invert_y", 0) != 0,
        }
    }
}

/// what the player used last (Input::device): a game shows its prompts for that
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Device {
    /// the keyboard or the mouse (also before anything was used)
    #[default]
    KeyboardMouse,
    /// a game pad (Input::active_pad)
    Pad,
}

/// the pad's part of an Input
#[derive(Clone, Debug, Default)]
struct PadIn {
    settings: PadSettings,
    /// the pad that drives the actions, the one the game pinned
    active: Option<u8>,
    pin: Option<u8>,
    /// the active pad's buttons held as the input counts them (bit PAD_*), their values, its sticks after
    /// the dead zone, their turns (rad/s), its stick directions held (bit axis * 2 + positive)
    held: u32,
    values: [f64; PAD_BUTTONS],
    sticks: [f64; PAD_AXES],
    look: [f64; PAD_AXES],
    dirs: u8,
    /// buttons that count only after a release: held when the input paused, when the pad became the
    /// active one, or taken by a capture
    latched: u32,
    /// the press that started pad play: its release is swallowed too
    swallowed: u32,
    /// playing with the pad without the pointer lock
    play: bool,
    /// per slot: the buttons held and whether a stick was out of its dead zone at the last frame
    prev: [u32; PAD_MAX],
    prev_moved: u8,
    /// per slot: the directions held at the last frame (captures)
    prev_dirs: [u8; PAD_MAX],
}

/// an action in a tick (TickInput) or a frame
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionState {
    /// held at the tick, or pressed since the tick before (a tap between two ticks counts once)
    pub down: bool,
    /// pressed since the tick before
    pub pressed: bool,
    /// released since the tick before
    pub released: bool,
    /// an Axis: its value at the tick (-1 ..= 1); a Motion: the motion since the tick before; a Button:
    /// 1 when down
    pub value: f64,
}

/// the actions of one tick (Input::tick), indexed by ActionId
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickInput {
    states: Vec<ActionState>,
}

impl TickInput {
    /// the state of a (all false and 0 for an action the map did not have)
    pub fn get(&self, a: ActionId) -> ActionState {
        self.states.get(a.index()).copied().unwrap_or_default()
    }

    /// a is down
    pub fn down(&self, a: ActionId) -> bool {
        self.get(a).down
    }

    /// a was pressed since the tick before
    pub fn pressed(&self, a: ActionId) -> bool {
        self.get(a).pressed
    }

    /// a was released since the tick before
    pub fn released(&self, a: ActionId) -> bool {
        self.get(a).released
    }

    /// a's value
    pub fn value(&self, a: ActionId) -> f64 {
        self.get(a).value
    }

    /// every action's state in the map's order
    pub fn states(&self) -> &[ActionState] {
        &self.states
    }
}

/// what an action has counted since the frame and since the tick
#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    /// down: a bound source is held (Button, Axis)
    held: bool,
    frame_pressed: u32,
    frame_released: u32,
    tick_pressed: u32,
    tick_released: u32,
    frame_motion: f64,
    tick_motion: f64,
}

/// the input of a game: events in, actions out (see the module header)
#[derive(Clone, Debug)]
pub struct Input {
    map: ActionMap,
    mouse: MouseSettings,
    require_lock: bool,
    lock: PointerLock,
    /// the keys held, by usage
    keys: [u64; 4],
    /// the mouse buttons held (bit b)
    buttons: u8,
    /// buttons whose press was the lock's click: their release is the lock's too
    swallowed: u8,
    want_lock: bool,
    pause_edge: bool,
    pauses: u32,
    /// the wheel's turn not yet a whole notch (WheelUp / WheelDown)
    wheel_rest: f64,
    acc: Vec<Acc>,
    /// Some: the next source pressed is captured for a settings page instead of driving actions
    capture: Option<Option<Source>>,
    /// the game pad's state and settings
    pad: PadIn,
    device: Device,
}

impl Input {
    /// the input of these actions: mouse settings at their defaults, the lock required
    pub fn new(map: ActionMap) -> Input {
        let n = map.len();
        Input {
            map,
            mouse: MouseSettings::DEFAULT,
            require_lock: true,
            lock: PointerLock::Off,
            keys: [0; 4],
            buttons: 0,
            swallowed: 0,
            want_lock: false,
            pause_edge: false,
            pauses: 0,
            wheel_rest: 0.0,
            acc: alloc::vec![Acc::default(); n],
            capture: None,
            pad: PadIn { settings: PadSettings::DEFAULT, ..PadIn::default() },
            device: Device::KeyboardMouse,
        }
    }

    /// the actions and their bindings
    pub fn map(&self) -> &ActionMap {
        &self.map
    }

    /// change the actions or their bindings (a settings page, ActionMap::load); afterwards every action
    /// is held as its sources are (a binding that came or went presses or releases it)
    pub fn edit_map<V>(&mut self, f: impl FnOnce(&mut ActionMap) -> V) -> V {
        let v = f(&mut self.map);
        self.acc.resize(self.map.len(), Acc::default());
        self.update_held();
        v
    }

    /// the mouse settings
    pub fn mouse(&self) -> &MouseSettings {
        &self.mouse
    }

    /// other mouse settings
    pub fn set_mouse(&mut self, m: MouseSettings) {
        self.mouse = m;
    }

    /// whether the actions need the pointer lock (true, the default: a game that looks with the mouse);
    /// false: actions always fire, clicks too
    pub fn set_require_lock(&mut self, on: bool) {
        self.require_lock = on;
    }

    /// the lock is required (set_require_lock)
    pub fn require_lock(&self) -> bool {
        self.require_lock
    }

    /// the actions fire: the lock is held, or not required, or the platform has none (Unsupported), or the
    /// player plays with a pad (pad_playing)
    pub fn playing(&self) -> bool {
        self.mouse_playing() || self.pad.play
    }

    /// the mouse plays: the lock is held, not required or not there (else a press asks for it)
    fn mouse_playing(&self) -> bool {
        !self.require_lock || matches!(self.lock, PointerLock::Locked | PointerLock::Unsupported)
    }

    /// playing with a pad without the pointer lock: a pad button pressed while the game did not play
    /// started it (that press fired nothing); the lock taken or the pad gone ends it
    pub fn pad_playing(&self) -> bool {
        self.pad.play
    }

    /// the state of the lock as the input knows it (set_lock)
    pub fn lock(&self) -> PointerLock {
        self.lock
    }

    /// the state of the pointer lock now (ui.pointer_lock_state() every frame, or E_POINTER_LOCK through
    /// plat_event).  Its end (from Locked to anything else) pauses: every held input is let go, the actions
    /// report their release, take_pause says so once
    pub fn set_lock(&mut self, state: PointerLock) {
        let was = self.playing();
        self.lock = state;
        if state == PointerLock::Locked {
            self.want_lock = false;
            self.pad.play = false; // (the mouse took over)
        }
        self.play_changed(was);
    }

    /// playing started or ended since `was`: the end pauses (everything let go), the start takes what is
    /// held without a press
    fn play_changed(&mut self, was: bool) {
        if was && !self.playing() {
            self.release_all();
            self.pause_edge = true;
            self.pauses += 1;
        } else if !was && self.playing() {
            // keys held while paused count from now on, without a press (holding fire while clicking back
            // into the game does not shoot)
            for i in 0..self.acc.len() {
                let act = &self.map.actions[i];
                if act.kind != ActionKind::Motion {
                    let held = act.binds.iter().any(|b| self.source_held(b.source));
                    self.acc[i].held = held;
                }
            }
        }
    }

    /// a press asked for the lock since the last call: the app asks for it now (ui.pointer_lock(true), in
    /// the frame of the press)
    pub fn take_lock_request(&mut self) -> bool {
        core::mem::take(&mut self.want_lock)
    }

    /// the lock ended since the last call: the game pauses (shows its menu)
    pub fn take_pause(&mut self) -> bool {
        core::mem::take(&mut self.pause_edge)
    }

    /// how often the lock has ended so far
    pub fn pauses(&self) -> u32 {
        self.pauses
    }

    /// every held key and button is let go (the actions report their release); motion not yet taken goes;
    /// a pad button held counts again only after it was let go
    pub fn release_all(&mut self) {
        self.keys = [0; 4];
        self.buttons = 0;
        self.swallowed = 0;
        self.wheel_rest = 0.0;
        let p = &mut self.pad;
        p.latched |= p.held | p.swallowed;
        p.held = 0;
        p.swallowed = 0;
        p.values = [0.0; PAD_BUTTONS];
        p.sticks = [0.0; PAD_AXES];
        p.look = [0.0; PAD_AXES];
        p.dirs = 0;
        for a in &mut self.acc {
            a.frame_motion = 0.0;
            a.tick_motion = 0.0;
        }
        self.update_held();
    }

    /// the key at usage `code` (plat::KC_*, KeyEvent::code) went down or up; a down while held (the
    /// system's repeat) changes nothing
    pub fn key(&mut self, code: u32, down: bool) {
        if code == 0 || code > 0xFF {
            return;
        }
        let (w, bit) = ((code / 64) as usize, 1u64 << (code % 64));
        if down && self.capture_take(Source::Key(code)) {
            return;
        }
        let held = self.keys[w] & bit != 0;
        if down == held {
            return;
        }
        if down {
            self.device = Device::KeyboardMouse;
            self.keys[w] |= bit;
        } else {
            self.keys[w] &= !bit;
        }
        self.update_held();
    }

    /// the key at usage `code` is held
    pub fn key_held(&self, code: u32) -> bool {
        code != 0 && code <= 0xFF && self.keys[(code / 64) as usize] & (1u64 << (code % 64)) != 0
    }

    /// mouse button b (MOUSE_*) went down or up.  While the game needs the lock and does not hold it, a
    /// press asks for the lock (take_lock_request) and is swallowed with its release
    pub fn button(&mut self, b: u8, down: bool) {
        if b == 0 || b > MOUSE_MAX {
            return;
        }
        let bit = 1u8 << b;
        if down {
            if self.capture_take(Source::Mouse(b)) {
                return;
            }
            self.device = Device::KeyboardMouse;
            if !self.mouse_playing() {
                // (also while playing with a pad: the mouse plays only with the lock)
                self.swallowed |= bit;
                self.want_lock = true;
                return;
            }
            if self.buttons & bit != 0 {
                return;
            }
            self.buttons |= bit;
        } else {
            if self.swallowed & bit != 0 {
                self.swallowed &= !bit;
                return;
            }
            if self.buttons & bit == 0 {
                return;
            }
            self.buttons &= !bit;
        }
        self.update_held();
    }

    /// the buttons of a frame as ui reports them (UiInput::pressed and released: bit 1 << BTN_*): a
    /// button held before with both bits went up and down again, one not held went down and up
    pub fn buttons(&mut self, pressed: u32, released: u32) {
        for b in 1..=MOUSE_MAX {
            let bit = 1u32 << b;
            let (p, r) = (pressed & bit != 0, released & bit != 0);
            let held = self.buttons & (1 << b) != 0 || self.swallowed & (1 << b) != 0;
            match (p, r) {
                (true, true) if held => {
                    self.button(b, false);
                    self.button(b, true);
                }
                (true, true) => {
                    self.button(b, true);
                    self.button(b, false);
                }
                (true, false) => self.button(b, true),
                (false, true) => self.button(b, false),
                (false, false) => {}
            }
        }
    }

    /// the mouse button b is held (not the lock's click)
    pub fn button_held(&self, b: u8) -> bool {
        b != 0 && b <= MOUSE_MAX && self.buttons & (1 << b) != 0
    }

    /// raw motion of the locked pointer (ui.pointer_motion(), plat::win_raw_motion_take): right and down
    /// positive, in this platform's units; nothing while not playing
    pub fn motion(&mut self, dx: f64, dy: f64) {
        if !self.playing() || !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
            return;
        }
        self.device = Device::KeyboardMouse;
        let (kx, ky) = self.mouse.radians_per_unit();
        for (i, a) in self.acc.iter_mut().enumerate() {
            let act = &self.map.actions[i];
            if act.kind != ActionKind::Motion {
                continue;
            }
            let mut m = 0.0;
            for b in &act.binds {
                match b.source {
                    Source::MouseX => m += dx * kx * b.scale,
                    Source::MouseY => m += dy * ky * b.scale,
                    _ => {}
                }
            }
            a.frame_motion += m;
            a.tick_motion += m;
        }
    }

    /// the wheel turned by `notches` (up positive; ui's 120 per notch is 1): Wheel motion, and a press
    /// of WheelUp / WheelDown per whole notch
    pub fn wheel(&mut self, notches: f64) {
        if !notches.is_finite() || notches == 0.0 {
            return;
        }
        if self.capture.is_some() {
            self.capture_take(if notches > 0.0 { Source::WheelUp } else { Source::WheelDown });
            return;
        }
        if !self.playing() {
            return;
        }
        for (i, a) in self.acc.iter_mut().enumerate() {
            let act = &self.map.actions[i];
            if act.kind == ActionKind::Motion {
                let m = act.binds.iter().filter(|b| b.source == Source::Wheel).fold(0.0, |m, b| m + notches * b.scale);
                a.frame_motion += m;
                a.tick_motion += m;
            }
        }
        self.wheel_rest += notches;
        while self.wheel_rest >= 1.0 || self.wheel_rest <= -1.0 {
            let s = if self.wheel_rest > 0.0 { Source::WheelUp } else { Source::WheelDown };
            self.wheel_rest -= if self.wheel_rest > 0.0 { 1.0 } else { -1.0 };
            self.pulse(s);
        }
    }

    /// a press and a release of every action bound to s that is not held otherwise
    fn pulse(&mut self, s: Source) {
        for (i, a) in self.acc.iter_mut().enumerate() {
            let act = &self.map.actions[i];
            if act.kind == ActionKind::Button && !a.held && act.binds.iter().any(|b| b.source == s) {
                a.frame_pressed += 1;
                a.tick_pressed += 1;
                a.frame_released += 1;
                a.tick_released += 1;
            }
        }
    }

    /// a source is held now (keys, buttons, the active pad's buttons and stick directions, a stick out of
    /// its dead zone)
    fn source_held(&self, s: Source) -> bool {
        match s {
            Source::Key(c) => self.key_held(c),
            Source::Mouse(b) => self.button_held(b),
            Source::PadButton(b) => self.pad_held(b as usize),
            Source::PadDir(a, pos) => (a as usize) < PAD_AXES && self.pad.dirs & (1 << (a * 2 + pos as u8)) != 0,
            Source::PadAxis(a) => self.pad.sticks.get(a as usize).is_some_and(|&v| v != 0.0),
            _ => false,
        }
    }

    /// a source's value in an Axis (None: not held): keys and buttons 1, a pad button its analog value
    /// (a trigger's), a stick its value after the dead zone, a direction its part of it
    fn source_value(&self, s: Source) -> Option<f64> {
        match s {
            Source::PadButton(b) => {
                let b = b as usize;
                let blocked = b >= PAD_BUTTONS || (self.pad.latched | self.pad.swallowed) & (1 << b) != 0;
                let v = if blocked { 0.0 } else { self.pad.values[b] };
                (v != 0.0).then_some(v)
            }
            Source::PadAxis(a) => self.pad.sticks.get(a as usize).copied().filter(|&v| v != 0.0),
            Source::PadDir(a, pos) => {
                let v = self.pad.sticks.get(a as usize).copied().unwrap_or(0.0);
                let v = if pos { v } else { -v };
                (v > 0.0).then_some(v)
            }
            _ => self.source_held(s).then_some(1.0),
        }
    }

    /// every Button and Axis action's held state from its sources, counting the presses and releases
    fn update_held(&mut self) {
        let playing = self.playing();
        for i in 0..self.acc.len() {
            let act = &self.map.actions[i];
            if act.kind == ActionKind::Motion {
                continue;
            }
            let held = playing && act.binds.iter().any(|b| self.source_held(b.source));
            let a = &mut self.acc[i];
            if held != a.held {
                a.held = held;
                if held {
                    a.frame_pressed += 1;
                    a.tick_pressed += 1;
                } else {
                    a.frame_released += 1;
                    a.tick_released += 1;
                }
            }
        }
    }

    /// an Axis' value now: the scales of its sources held, kept in -1 ..= 1 (a Button: 1 when held)
    pub fn axis(&self, a: ActionId) -> f64 {
        if !self.playing() {
            return 0.0;
        }
        let act = &self.map.actions[a.index()];
        match act.kind {
            ActionKind::Motion => 0.0,
            ActionKind::Button => if self.acc[a.index()].held { 1.0 } else { 0.0 },
            ActionKind::Axis => {
                // (from +0.0: an empty f64 sum is -0.0, which a text would show as "-0"; a key's value 1 times
                // its scale is its scale, exactly)
                let v = act.binds.iter().fold(0.0, |v, b| match self.source_value(b.source) {
                    Some(x) => v + x * b.scale,
                    None => v,
                });
                v.clamp(-1.0, 1.0)
            }
        }
    }

    /// a is held now (a source bound to it is held, while playing)
    pub fn down(&self, a: ActionId) -> bool {
        self.acc[a.index()].held
    }

    /// a was pressed in this frame (since end_frame)
    pub fn pressed(&self, a: ActionId) -> bool {
        self.acc[a.index()].frame_pressed > 0
    }

    /// a was released in this frame
    pub fn released(&self, a: ActionId) -> bool {
        self.acc[a.index()].frame_released > 0
    }

    /// a Motion's motion in this frame (radians for the mouse, notches for the wheel)
    pub fn motion_of(&self, a: ActionId) -> f64 {
        self.acc[a.index()].frame_motion
    }

    /// the frame is over: its presses, releases and motion are cleared (the tick's stay for tick())
    pub fn end_frame(&mut self) {
        for a in &mut self.acc {
            a.frame_pressed = 0;
            a.frame_released = 0;
            a.frame_motion = 0.0;
        }
    }

    /// the actions for a tick, and the presses, releases and motion since the tick before are taken (the
    /// next tick starts from nothing)
    pub fn tick(&mut self) -> TickInput {
        let mut states = Vec::with_capacity(self.acc.len());
        for i in 0..self.acc.len() {
            let id = ActionId(i as u16);
            let kind = self.map.actions[i].kind;
            let a = self.acc[i];
            let down = a.held || a.tick_pressed > 0;
            let value = match kind {
                ActionKind::Motion => a.tick_motion,
                ActionKind::Button => if down { 1.0 } else { 0.0 },
                ActionKind::Axis => self.axis(id),
            };
            states.push(ActionState { down: kind != ActionKind::Motion && down, pressed: a.tick_pressed > 0, released: a.tick_released > 0, value });
            let a = &mut self.acc[i];
            a.tick_pressed = 0;
            a.tick_released = 0;
            a.tick_motion = 0.0;
        }
        TickInput { states }
    }

    /// the next key, mouse button or wheel notch pressed is not an input but the answer to a settings
    /// page's "press a key" (take_capture); the actions keep their state meanwhile
    pub fn start_capture(&mut self) {
        self.capture = Some(None);
    }

    /// the source pressed since start_capture (the capture ends with it); None: none yet, or no capture
    pub fn take_capture(&mut self) -> Option<Source> {
        match self.capture {
            Some(Some(s)) => {
                self.capture = None;
                Some(s)
            }
            _ => None,
        }
    }

    /// a capture is running (start_capture, nothing pressed yet)
    pub fn capturing(&self) -> bool {
        matches!(self.capture, Some(None))
    }

    /// (key, button, wheel) the press is the capture's answer
    fn capture_take(&mut self, s: Source) -> bool {
        if let Some(None) = self.capture {
            self.capture = Some(Some(s));
            return true;
        }
        false
    }

    /// an event of plat's window (a game with its own window and swapchain, without ui): keys
    /// (E_KEY_DOWN / E_KEY_UP by plat::key_code of their scancode), buttons (E_DOWN / E_UP), the wheel
    /// (E_WHEEL), the lock (E_POINTER_LOCK: data 1 Locked, else plat::win_pointer_lock_state of the
    /// window), its motion (E_RAW_MOTION: plat::win_raw_motion_take of the window), E_FOCUS_OUT lets
    /// everything go -> the event was one of these
    pub fn plat_event(&mut self, ev: &plat::Event) -> bool {
        match ev.typ {
            plat::E_KEY_DOWN => self.key(plat::key_code(ev.x as u32), true),
            plat::E_KEY_UP => self.key(plat::key_code(ev.x as u32), false),
            plat::E_DOWN => self.button(ev.btn, true),
            plat::E_UP => self.button(ev.btn, false),
            plat::E_WHEEL => self.wheel((ev.data as u16 as i16) as f64 / 120.0),
            plat::E_POINTER_LOCK => {
                let s = if ev.data == 1 {
                    PointerLock::Locked
                } else {
                    match plat::win_pointer_lock_state(ev.win as i32) {
                        PointerLock::Locked => PointerLock::Off,
                        s => s,
                    }
                };
                self.set_lock(s);
            }
            plat::E_RAW_MOTION => {
                let (dx, dy) = plat::win_raw_motion_take(ev.win as i32);
                self.motion(dx, dy);
            }
            plat::E_FOCUS_OUT => self.release_all(),
            // (the pads come with pad_frame(&plat::pads(), dt) every frame)
            plat::E_GAMEPAD => {}
            _ => return false,
        }
        true
    }

    /// the bindings, the mouse and the pad settings into section `section` of a settings file
    /// (ActionMap::store, MouseSettings::store, PadSettings::store): with base::Settings,
    /// `input.store(settings.ini_mut(), "input")`
    pub fn store(&self, ini: &mut Ini, section: &str) {
        self.map.store(ini, section);
        self.mouse.store(ini, section);
        self.pad.settings.store(ini, section);
    }

    /// the bindings, the mouse and the pad settings from section `section` (Self::store) -> the bindings
    /// that could not be read
    pub fn load(&mut self, ini: &Ini, section: &str) -> usize {
        let bad = self.map.load(ini, section);
        self.mouse = MouseSettings::load(ini, section);
        self.pad.settings = PadSettings::load(ini, section);
        self.update_held();
        bad
    }

    // ------------------------------------------------------------------ game pads

    /// the pad settings
    pub fn pad_settings(&self) -> &PadSettings {
        &self.pad.settings
    }

    /// other pad settings (from the next pad_frame on)
    pub fn set_pad_settings(&mut self, s: PadSettings) {
        self.pad.settings = s;
    }

    /// the pad that drives the actions (plat::Gamepad::index): the one used last, or the pinned one;
    /// None before any pad was used
    pub fn active_pad(&self) -> Option<u8> {
        self.pad.active
    }

    /// only this pad drives the input from now on (an Input per player in a split screen); None: the
    /// one used last again
    pub fn pin_pad(&mut self, index: Option<u8>) {
        self.pad.pin = index;
    }

    /// the pad pinned (pin_pad)
    pub fn pinned_pad(&self) -> Option<u8> {
        self.pad.pin
    }

    /// what the player used last: a key or the mouse, or a pad
    pub fn device(&self) -> Device {
        self.device
    }

    /// the active pad's button b (plat::PAD_*) is held as the input counts it (not one that waits for its
    /// release after a pause, not the press that started pad play)
    pub fn pad_held(&self, b: usize) -> bool {
        b < PAD_BUTTONS && self.pad.held & (1 << b) != 0
    }

    /// the active pad's stick axis a (plat::PAD_LX ..) after the dead zone
    pub fn pad_stick(&self, a: usize) -> f64 {
        self.pad.sticks.get(a).copied().unwrap_or(0.0)
    }

    /// the pads of this frame (ui.gamepads(), plat::pads(): the connected ones with the edges of their
    /// buttons) and the seconds since the frame before (a stick's turn is its rate times that; kept in
    /// 0 ..= 0.25).  The pad used last drives the actions (or the pinned one); a pad button pressed while
    /// the game does not play starts pad play and fires nothing, nor does its release; the active pad gone
    /// during pad play pauses.  Call it once a frame, before reading the actions, also without pads (an
    /// empty slice lets the active pad go)
    pub fn pad_frame(&mut self, pads: &[Gamepad], dt: f64) {
        let dt = if dt.is_finite() && dt > 0.0 { dt.min(0.25) } else { 0.0 };
        let settings = self.pad.settings;
        // which pad: the pinned one, else the active one while it is there; one that is used takes over
        let mut active = self.pad.active.filter(|&a| pads.iter().any(|p| p.index == a && p.connected));
        let mut used: Option<u8> = None;
        let mut captured: Option<Source> = None;
        for p in pads.iter().filter(|p| p.connected && (p.index as usize) < PAD_MAX) {
            let i = p.index as usize;
            let pressed_new = (p.pressed | p.down) & !self.pad.prev[i];
            let (l, r) = (settings.stick(p.axes[0] as f64, p.axes[1] as f64), settings.stick(p.axes[2] as f64, p.axes[3] as f64));
            let moved = l != (0.0, 0.0) || r != (0.0, 0.0);
            let was_moved = self.pad.prev_moved & (1 << i) != 0;
            // (captures: the lowest new button, else a stick's direction newly past PAD_DIR_PRESS)
            let mut dirs = 0u8;
            for (a, v) in [l.0, l.1, r.0, r.1].into_iter().enumerate() {
                if v >= PAD_DIR_PRESS {
                    dirs |= 1 << (a * 2 + 1);
                } else if v <= -PAD_DIR_PRESS {
                    dirs |= 1 << (a * 2);
                }
            }
            if captured.is_none() {
                if pressed_new != 0 {
                    captured = Some(Source::PadButton(pressed_new.trailing_zeros() as u8));
                } else if dirs & !self.pad.prev_dirs[i] != 0 {
                    let k = (dirs & !self.pad.prev_dirs[i]).trailing_zeros() as u8;
                    captured = Some(Source::PadDir(k / 2, k % 2 == 1));
                }
            }
            self.pad.prev[i] = p.pressed;
            self.pad.prev_dirs[i] = dirs;
            if moved {
                self.pad.prev_moved |= 1 << i;
            } else {
                self.pad.prev_moved &= !(1 << i);
            }
            if pressed_new != 0 || (moved && !was_moved) {
                used = Some(p.index);
                // (a button takes over from another pad; a stick only when none is active: a stick that
                // drifts out of its dead zone does not steal the input)
                if self.pad.pin.is_none() && (pressed_new != 0 || active.is_none()) {
                    active = Some(p.index);
                }
            }
        }
        // slots that went: their memory goes
        for i in 0..PAD_MAX {
            if !pads.iter().any(|p| p.connected && p.index as usize == i) {
                self.pad.prev[i] = 0;
                self.pad.prev_dirs[i] = 0;
                self.pad.prev_moved &= !(1 << i);
            }
        }
        if let Some(pin) = self.pad.pin {
            active = pads.iter().any(|p| p.connected && p.index == pin).then_some(pin);
        }
        let was_playing = self.playing();
        // the active pad changed or went: what the old one held goes (a new one's held buttons wait for
        // their release)
        if active != self.pad.active {
            let gone = self.pad.active.is_some() && active.is_none();
            self.pad.held = 0;
            self.pad.dirs = 0;
            self.pad.swallowed = 0;
            self.pad.values = [0.0; PAD_BUTTONS];
            self.pad.sticks = [0.0; PAD_AXES];
            self.pad.look = [0.0; PAD_AXES];
            self.pad.latched = active.and_then(|a| pads.iter().find(|p| p.index == a)).map_or(0, |p| p.pressed & !p.down);
            self.pad.active = active;
            if gone && self.pad.play {
                self.pad.play = false; // the pad went during pad play: the pause
            }
            self.update_held();
        }
        // a settings page's capture takes the press (it drives nothing, nor does its hold)
        if let (true, Some(s)) = (self.capturing(), captured) {
            self.capture_take(s);
            if let Source::PadButton(b) = s {
                self.pad.latched |= 1 << b;
            }
        }
        let Some(p) = active.and_then(|a| pads.iter().find(|p| p.index == a)) else {
            self.play_changed(was_playing);
            return;
        };
        if used == Some(p.index) {
            self.device = Device::Pad;
        }
        // the buttons: a latched one counts after it was let go, the press that started pad play with its
        // release
        let pd = &mut self.pad;
        pd.latched &= p.pressed & !p.up;
        pd.swallowed &= p.pressed & !p.up;
        let new_presses = p.down & !pd.latched & !pd.swallowed;
        let start = !was_playing && new_presses != 0 && self.capture.is_none();
        if start {
            pd.play = true;
            pd.swallowed |= new_presses;
        }
        let mut values = [0.0f64; PAD_BUTTONS];
        for (b, v) in values.iter_mut().enumerate() {
            *v = p.buttons[b] as f64;
        }
        pd.values = values;
        let l = settings.stick(p.axes[0] as f64, p.axes[1] as f64);
        let r = settings.stick(p.axes[2] as f64, p.axes[3] as f64);
        pd.sticks = [l.0, l.1, r.0, r.1];
        let (llx, lly) = settings.look(p.axes[0] as f64, p.axes[1] as f64);
        let (lrx, lry) = settings.look(p.axes[2] as f64, p.axes[3] as f64);
        pd.look = [llx, lly, lrx, lry];
        // the stick directions with their hysteresis
        let mut dirs = pd.dirs;
        for (a, &v) in pd.sticks.iter().enumerate() {
            for (pos, sv) in [(false, -v), (true, v)] {
                let bit = 1u8 << (a * 2 + pos as usize);
                if dirs & bit != 0 && sv < PAD_DIR_RELEASE {
                    dirs &= !bit;
                } else if dirs & bit == 0 && sv >= PAD_DIR_PRESS {
                    dirs |= bit;
                }
            }
        }
        pd.dirs = dirs;
        if start {
            // playing from now on: what is held counts without a press (play_changed), the starting press
            // is swallowed
            pd.held = p.pressed & !pd.latched & !pd.swallowed;
            self.play_changed(was_playing);
        } else {
            // the edges since the last frame: a button that went down and up (a tap) presses and releases,
            // one held that went up and down again releases and presses
            let blocked = pd.latched | pd.swallowed;
            let now = p.pressed & !blocked;
            let was = pd.held;
            let taps = p.down & p.up & !p.pressed & !was & !blocked;
            let again = p.down & p.up & p.pressed & was & !blocked;
            if again != 0 {
                pd.held = was & !again;
                self.update_held();
            }
            self.pad.held = now;
            self.update_held();
            for b in 0..PAD_BUTTONS {
                if taps & (1 << b) != 0 && self.playing() {
                    self.pulse(Source::PadButton(b as u8));
                }
            }
            self.play_changed(was_playing);
        }
        // looking: a Motion's stick axes turn by their rate times the frame's time
        if dt > 0.0 && self.playing() {
            let look = self.pad.look;
            for (i, a) in self.acc.iter_mut().enumerate() {
                let act = &self.map.actions[i];
                if act.kind != ActionKind::Motion {
                    continue;
                }
                let mut m = 0.0;
                for b in &act.binds {
                    if let Source::PadAxis(k) = b.source {
                        m += look.get(k as usize).copied().unwrap_or(0.0) * dt * b.scale;
                    }
                }
                if m != 0.0 {
                    a.frame_motion += m;
                    a.tick_motion += m;
                }
            }
        }
    }
}
