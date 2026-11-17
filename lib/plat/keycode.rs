// keycode -- (G7a) where a key sits on the keyboard, the same number on every platform: the USB
// HID usage IDs of the keyboard page (page 7), whatever the layout prints on the key
//
//   KC_*                              the usages (KC_W is the key right of Q on a US keyboard: the
//                                     Z of a French one, still KC_W)
//   key_code(scancode) -> code        Event::x of a key event on this platform -> its usage
//                                     (0: unknown); KeyEvent::code of the UI
//   key_scancode(code) -> scancode    the other way (scripts, tests); 0: this platform has none
//   key_code_set1(sc) / key_code_evdev(code) / key_code_mac(kvk)   the tables of the
//                                     platforms, on every target (tests): Windows' scan codes
//                                     of set 1 (0xE0.. for the extended keys, as Event::x),
//                                     Linux's evdev codes, the virtual key codes of macOS (kVK_*)
//   key_code_name(code) / key_code_from_name(name)   the browser's KeyboardEvent.code names
//                                     ("KeyW", "ShiftLeft"); "" / 0 unknown
//   key_label(code) -> Option<String> what the current layout prints on that key ("Z" for KC_W
//                                     on a French keyboard), None where the platform does not
//                                     say (the browser, Android, iOS) or the key has no label
//
// What Event::x of E_KEY_DOWN / E_KEY_UP holds per platform: Windows' scan code (set 1, 0xE0..
// extended), Linux's evdev code (X11 and Wayland), the usage itself on macOS, iOS and in the
// browser (their own codes are turned into usages where the event is made); Android 0.
//
// The rows follow the USB HID usage tables (keyboard page) and the mapping of the UI Events
// KeyboardEvent.code specification (W3C) to the native codes of each system.

use alloc::string::String;

/// (G7a) USB HID usages of the keyboard page: where a key sits (KeyEvent::code)
pub const KC_A: u32 = 0x04;
pub const KC_B: u32 = 0x05;
pub const KC_C: u32 = 0x06;
pub const KC_D: u32 = 0x07;
pub const KC_E: u32 = 0x08;
pub const KC_F: u32 = 0x09;
pub const KC_G: u32 = 0x0A;
pub const KC_H: u32 = 0x0B;
pub const KC_I: u32 = 0x0C;
pub const KC_J: u32 = 0x0D;
pub const KC_K: u32 = 0x0E;
pub const KC_L: u32 = 0x0F;
pub const KC_M: u32 = 0x10;
pub const KC_N: u32 = 0x11;
pub const KC_O: u32 = 0x12;
pub const KC_P: u32 = 0x13;
pub const KC_Q: u32 = 0x14;
pub const KC_R: u32 = 0x15;
pub const KC_S: u32 = 0x16;
pub const KC_T: u32 = 0x17;
pub const KC_U: u32 = 0x18;
pub const KC_V: u32 = 0x19;
pub const KC_W: u32 = 0x1A;
pub const KC_X: u32 = 0x1B;
pub const KC_Y: u32 = 0x1C;
pub const KC_Z: u32 = 0x1D;
pub const KC_1: u32 = 0x1E;
pub const KC_2: u32 = 0x1F;
pub const KC_3: u32 = 0x20;
pub const KC_4: u32 = 0x21;
pub const KC_5: u32 = 0x22;
pub const KC_6: u32 = 0x23;
pub const KC_7: u32 = 0x24;
pub const KC_8: u32 = 0x25;
pub const KC_9: u32 = 0x26;
pub const KC_0: u32 = 0x27;
pub const KC_ENTER: u32 = 0x28;
pub const KC_ESCAPE: u32 = 0x29;
pub const KC_BACKSPACE: u32 = 0x2A;
pub const KC_TAB: u32 = 0x2B;
pub const KC_SPACE: u32 = 0x2C;
pub const KC_MINUS: u32 = 0x2D;
pub const KC_EQUAL: u32 = 0x2E;
pub const KC_BRACKET_LEFT: u32 = 0x2F;
pub const KC_BRACKET_RIGHT: u32 = 0x30;
/// the US layout's backslash; on an ISO keyboard the key left of Enter (# on a German one)
pub const KC_BACKSLASH: u32 = 0x31;
pub const KC_SEMICOLON: u32 = 0x33;
pub const KC_QUOTE: u32 = 0x34;
/// left of 1 on a US keyboard (` ~), ^ on a German one
pub const KC_BACKQUOTE: u32 = 0x35;
pub const KC_COMMA: u32 = 0x36;
pub const KC_PERIOD: u32 = 0x37;
pub const KC_SLASH: u32 = 0x38;
pub const KC_CAPS_LOCK: u32 = 0x39;
pub const KC_F1: u32 = 0x3A;
pub const KC_F2: u32 = 0x3B;
pub const KC_F3: u32 = 0x3C;
pub const KC_F4: u32 = 0x3D;
pub const KC_F5: u32 = 0x3E;
pub const KC_F6: u32 = 0x3F;
pub const KC_F7: u32 = 0x40;
pub const KC_F8: u32 = 0x41;
pub const KC_F9: u32 = 0x42;
pub const KC_F10: u32 = 0x43;
pub const KC_F11: u32 = 0x44;
pub const KC_F12: u32 = 0x45;
pub const KC_PRINT_SCREEN: u32 = 0x46;
pub const KC_SCROLL_LOCK: u32 = 0x47;
pub const KC_PAUSE: u32 = 0x48;
pub const KC_INSERT: u32 = 0x49;
pub const KC_HOME: u32 = 0x4A;
pub const KC_PAGE_UP: u32 = 0x4B;
pub const KC_DELETE: u32 = 0x4C;
pub const KC_END: u32 = 0x4D;
pub const KC_PAGE_DOWN: u32 = 0x4E;
pub const KC_RIGHT: u32 = 0x4F;
pub const KC_LEFT: u32 = 0x50;
pub const KC_DOWN: u32 = 0x51;
pub const KC_UP: u32 = 0x52;
pub const KC_NUM_LOCK: u32 = 0x53;
pub const KC_NUMPAD_DIVIDE: u32 = 0x54;
pub const KC_NUMPAD_MULTIPLY: u32 = 0x55;
pub const KC_NUMPAD_SUBTRACT: u32 = 0x56;
pub const KC_NUMPAD_ADD: u32 = 0x57;
pub const KC_NUMPAD_ENTER: u32 = 0x58;
pub const KC_NUMPAD1: u32 = 0x59;
pub const KC_NUMPAD2: u32 = 0x5A;
pub const KC_NUMPAD3: u32 = 0x5B;
pub const KC_NUMPAD4: u32 = 0x5C;
pub const KC_NUMPAD5: u32 = 0x5D;
pub const KC_NUMPAD6: u32 = 0x5E;
pub const KC_NUMPAD7: u32 = 0x5F;
pub const KC_NUMPAD8: u32 = 0x60;
pub const KC_NUMPAD9: u32 = 0x61;
pub const KC_NUMPAD0: u32 = 0x62;
pub const KC_NUMPAD_DECIMAL: u32 = 0x63;
/// the ISO keyboard's key left of Z (< > on a German one)
pub const KC_INTL_BACKSLASH: u32 = 0x64;
pub const KC_CONTEXT_MENU: u32 = 0x65;
pub const KC_POWER: u32 = 0x66;
pub const KC_NUMPAD_EQUAL: u32 = 0x67;
pub const KC_F13: u32 = 0x68;
pub const KC_F14: u32 = 0x69;
pub const KC_F15: u32 = 0x6A;
pub const KC_F16: u32 = 0x6B;
pub const KC_F17: u32 = 0x6C;
pub const KC_F18: u32 = 0x6D;
pub const KC_F19: u32 = 0x6E;
pub const KC_F20: u32 = 0x6F;
pub const KC_F21: u32 = 0x70;
pub const KC_F22: u32 = 0x71;
pub const KC_F23: u32 = 0x72;
pub const KC_F24: u32 = 0x73;
pub const KC_VOLUME_MUTE: u32 = 0x7F;
pub const KC_VOLUME_UP: u32 = 0x80;
pub const KC_VOLUME_DOWN: u32 = 0x81;
pub const KC_NUMPAD_COMMA: u32 = 0x85;
pub const KC_INTL_RO: u32 = 0x87;
pub const KC_KANA_MODE: u32 = 0x88;
pub const KC_INTL_YEN: u32 = 0x89;
pub const KC_CONVERT: u32 = 0x8A;
pub const KC_NON_CONVERT: u32 = 0x8B;
pub const KC_LANG1: u32 = 0x90;
pub const KC_LANG2: u32 = 0x91;
pub const KC_CONTROL_LEFT: u32 = 0xE0;
pub const KC_SHIFT_LEFT: u32 = 0xE1;
pub const KC_ALT_LEFT: u32 = 0xE2;
/// the left Windows / Super / Command key
pub const KC_META_LEFT: u32 = 0xE3;
pub const KC_CONTROL_RIGHT: u32 = 0xE4;
pub const KC_SHIFT_RIGHT: u32 = 0xE5;
/// the right Alt (AltGr on many layouts)
pub const KC_ALT_RIGHT: u32 = 0xE6;
pub const KC_META_RIGHT: u32 = 0xE7;

// one key: its usage, Windows' scan code (set 1, 0xE0.. extended; 0 none), Linux's evdev code (0
// none), macOS' virtual key code (NO_MAC none: kVK_ANSI_A is 0) and the browser's code name
struct Row {
    u: u8,
    win: u16,
    ev: u16,
    mac: u8,
    name: &'static str,
}

const NO_MAC: u8 = 0xFF;

const fn r(u: u32, win: u16, ev: u16, mac: u8, name: &'static str) -> Row {
    Row { u: u as u8, win, ev, mac, name }
}

static ROWS: [Row; 130] = [
    r(KC_A, 0x1E, 30, 0x00, "KeyA"),
    r(KC_B, 0x30, 48, 0x0B, "KeyB"),
    r(KC_C, 0x2E, 46, 0x08, "KeyC"),
    r(KC_D, 0x20, 32, 0x02, "KeyD"),
    r(KC_E, 0x12, 18, 0x0E, "KeyE"),
    r(KC_F, 0x21, 33, 0x03, "KeyF"),
    r(KC_G, 0x22, 34, 0x05, "KeyG"),
    r(KC_H, 0x23, 35, 0x04, "KeyH"),
    r(KC_I, 0x17, 23, 0x22, "KeyI"),
    r(KC_J, 0x24, 36, 0x26, "KeyJ"),
    r(KC_K, 0x25, 37, 0x28, "KeyK"),
    r(KC_L, 0x26, 38, 0x25, "KeyL"),
    r(KC_M, 0x32, 50, 0x2E, "KeyM"),
    r(KC_N, 0x31, 49, 0x2D, "KeyN"),
    r(KC_O, 0x18, 24, 0x1F, "KeyO"),
    r(KC_P, 0x19, 25, 0x23, "KeyP"),
    r(KC_Q, 0x10, 16, 0x0C, "KeyQ"),
    r(KC_R, 0x13, 19, 0x0F, "KeyR"),
    r(KC_S, 0x1F, 31, 0x01, "KeyS"),
    r(KC_T, 0x14, 20, 0x11, "KeyT"),
    r(KC_U, 0x16, 22, 0x20, "KeyU"),
    r(KC_V, 0x2F, 47, 0x09, "KeyV"),
    r(KC_W, 0x11, 17, 0x0D, "KeyW"),
    r(KC_X, 0x2D, 45, 0x07, "KeyX"),
    r(KC_Y, 0x15, 21, 0x10, "KeyY"),
    r(KC_Z, 0x2C, 44, 0x06, "KeyZ"),
    r(KC_1, 0x02, 2, 0x12, "Digit1"),
    r(KC_2, 0x03, 3, 0x13, "Digit2"),
    r(KC_3, 0x04, 4, 0x14, "Digit3"),
    r(KC_4, 0x05, 5, 0x15, "Digit4"),
    r(KC_5, 0x06, 6, 0x17, "Digit5"),
    r(KC_6, 0x07, 7, 0x16, "Digit6"),
    r(KC_7, 0x08, 8, 0x1A, "Digit7"),
    r(KC_8, 0x09, 9, 0x1C, "Digit8"),
    r(KC_9, 0x0A, 10, 0x19, "Digit9"),
    r(KC_0, 0x0B, 11, 0x1D, "Digit0"),
    r(KC_ENTER, 0x1C, 28, 0x24, "Enter"),
    r(KC_ESCAPE, 0x01, 1, 0x35, "Escape"),
    r(KC_BACKSPACE, 0x0E, 14, 0x33, "Backspace"),
    r(KC_TAB, 0x0F, 15, 0x30, "Tab"),
    r(KC_SPACE, 0x39, 57, 0x31, "Space"),
    r(KC_MINUS, 0x0C, 12, 0x1B, "Minus"),
    r(KC_EQUAL, 0x0D, 13, 0x18, "Equal"),
    r(KC_BRACKET_LEFT, 0x1A, 26, 0x21, "BracketLeft"),
    r(KC_BRACKET_RIGHT, 0x1B, 27, 0x1E, "BracketRight"),
    r(KC_BACKSLASH, 0x2B, 43, 0x2A, "Backslash"),
    r(KC_SEMICOLON, 0x27, 39, 0x29, "Semicolon"),
    r(KC_QUOTE, 0x28, 40, 0x27, "Quote"),
    r(KC_BACKQUOTE, 0x29, 41, 0x32, "Backquote"),
    r(KC_COMMA, 0x33, 51, 0x2B, "Comma"),
    r(KC_PERIOD, 0x34, 52, 0x2F, "Period"),
    r(KC_SLASH, 0x35, 53, 0x2C, "Slash"),
    r(KC_CAPS_LOCK, 0x3A, 58, 0x39, "CapsLock"),
    r(KC_F1, 0x3B, 59, 0x7A, "F1"),
    r(KC_F2, 0x3C, 60, 0x78, "F2"),
    r(KC_F3, 0x3D, 61, 0x63, "F3"),
    r(KC_F4, 0x3E, 62, 0x76, "F4"),
    r(KC_F5, 0x3F, 63, 0x60, "F5"),
    r(KC_F6, 0x40, 64, 0x61, "F6"),
    r(KC_F7, 0x41, 65, 0x62, "F7"),
    r(KC_F8, 0x42, 66, 0x64, "F8"),
    r(KC_F9, 0x43, 67, 0x65, "F9"),
    r(KC_F10, 0x44, 68, 0x6D, "F10"),
    r(KC_F11, 0x57, 87, 0x67, "F11"),
    r(KC_F12, 0x58, 88, 0x6F, "F12"),
    r(KC_PRINT_SCREEN, 0xE037, 99, NO_MAC, "PrintScreen"),
    r(KC_SCROLL_LOCK, 0x46, 70, NO_MAC, "ScrollLock"),
    // (Windows reports Pause as 0x45 and Num Lock as 0xE045: the other way round than set 1)
    r(KC_PAUSE, 0x45, 119, NO_MAC, "Pause"),
    r(KC_INSERT, 0xE052, 110, 0x72, "Insert"), // (macOS: Help, where Insert sits)
    r(KC_HOME, 0xE047, 102, 0x73, "Home"),
    r(KC_PAGE_UP, 0xE049, 104, 0x74, "PageUp"),
    r(KC_DELETE, 0xE053, 111, 0x75, "Delete"),
    r(KC_END, 0xE04F, 107, 0x77, "End"),
    r(KC_PAGE_DOWN, 0xE051, 109, 0x79, "PageDown"),
    r(KC_RIGHT, 0xE04D, 106, 0x7C, "ArrowRight"),
    r(KC_LEFT, 0xE04B, 105, 0x7B, "ArrowLeft"),
    r(KC_DOWN, 0xE050, 108, 0x7D, "ArrowDown"),
    r(KC_UP, 0xE048, 103, 0x7E, "ArrowUp"),
    r(KC_NUM_LOCK, 0xE045, 69, 0x47, "NumLock"), // (macOS: the keypad's Clear)
    r(KC_NUMPAD_DIVIDE, 0xE035, 98, 0x4B, "NumpadDivide"),
    r(KC_NUMPAD_MULTIPLY, 0x37, 55, 0x43, "NumpadMultiply"),
    r(KC_NUMPAD_SUBTRACT, 0x4A, 74, 0x4E, "NumpadSubtract"),
    r(KC_NUMPAD_ADD, 0x4E, 78, 0x45, "NumpadAdd"),
    r(KC_NUMPAD_ENTER, 0xE01C, 96, 0x4C, "NumpadEnter"),
    r(KC_NUMPAD1, 0x4F, 79, 0x53, "Numpad1"),
    r(KC_NUMPAD2, 0x50, 80, 0x54, "Numpad2"),
    r(KC_NUMPAD3, 0x51, 81, 0x55, "Numpad3"),
    r(KC_NUMPAD4, 0x4B, 75, 0x56, "Numpad4"),
    r(KC_NUMPAD5, 0x4C, 76, 0x57, "Numpad5"),
    r(KC_NUMPAD6, 0x4D, 77, 0x58, "Numpad6"),
    r(KC_NUMPAD7, 0x47, 71, 0x59, "Numpad7"),
    r(KC_NUMPAD8, 0x48, 72, 0x5B, "Numpad8"),
    r(KC_NUMPAD9, 0x49, 73, 0x5C, "Numpad9"),
    r(KC_NUMPAD0, 0x52, 82, 0x52, "Numpad0"),
    r(KC_NUMPAD_DECIMAL, 0x53, 83, 0x41, "NumpadDecimal"),
    r(KC_INTL_BACKSLASH, 0x56, 86, 0x0A, "IntlBackslash"),
    r(KC_CONTEXT_MENU, 0xE05D, 127, 0x6E, "ContextMenu"),
    r(KC_POWER, 0xE05E, 116, NO_MAC, "Power"),
    r(KC_NUMPAD_EQUAL, 0x59, 117, 0x51, "NumpadEqual"),
    r(KC_F13, 0x64, 183, 0x69, "F13"),
    r(KC_F14, 0x65, 184, 0x6B, "F14"),
    r(KC_F15, 0x66, 185, 0x71, "F15"),
    r(KC_F16, 0x67, 186, 0x6A, "F16"),
    r(KC_F17, 0x68, 187, 0x40, "F17"),
    r(KC_F18, 0x69, 188, 0x4F, "F18"),
    r(KC_F19, 0x6A, 189, 0x50, "F19"),
    r(KC_F20, 0x6B, 190, 0x5A, "F20"),
    r(KC_F21, 0x6C, 191, NO_MAC, "F21"),
    r(KC_F22, 0x6D, 192, NO_MAC, "F22"),
    r(KC_F23, 0x6E, 193, NO_MAC, "F23"),
    r(KC_F24, 0x76, 194, NO_MAC, "F24"),
    r(KC_VOLUME_MUTE, 0xE020, 113, 0x4A, "AudioVolumeMute"),
    r(KC_VOLUME_UP, 0xE030, 115, 0x48, "AudioVolumeUp"),
    r(KC_VOLUME_DOWN, 0xE02E, 114, 0x49, "AudioVolumeDown"),
    r(KC_NUMPAD_COMMA, 0x7E, 121, 0x5F, "NumpadComma"),
    r(KC_INTL_RO, 0x73, 89, 0x5E, "IntlRo"),
    r(KC_KANA_MODE, 0x70, 93, NO_MAC, "KanaMode"),
    r(KC_INTL_YEN, 0x7D, 124, 0x5D, "IntlYen"),
    r(KC_CONVERT, 0x79, 92, NO_MAC, "Convert"),
    r(KC_NON_CONVERT, 0x7B, 94, NO_MAC, "NonConvert"),
    r(KC_LANG1, 0x72, 122, 0x68, "Lang1"),
    r(KC_LANG2, 0x71, 123, 0x66, "Lang2"),
    r(KC_CONTROL_LEFT, 0x1D, 29, 0x3B, "ControlLeft"),
    r(KC_SHIFT_LEFT, 0x2A, 42, 0x38, "ShiftLeft"),
    r(KC_ALT_LEFT, 0x38, 56, 0x3A, "AltLeft"),
    r(KC_META_LEFT, 0xE05B, 125, 0x37, "MetaLeft"),
    r(KC_CONTROL_RIGHT, 0xE01D, 97, 0x3E, "ControlRight"),
    r(KC_SHIFT_RIGHT, 0x36, 54, 0x3C, "ShiftRight"),
    r(KC_ALT_RIGHT, 0xE038, 100, 0x3D, "AltRight"),
    r(KC_META_RIGHT, 0xE05C, 126, 0x36, "MetaRight"),
];

/// (G7a) the usage of a Windows scan code of set 1 as a key event of Windows carries it in
/// Event::x (0xE0.. for the extended keys); 0 unknown.  Alt+Print (0x54) is Print, Ctrl+Pause
/// (0xE046) is Pause
pub fn key_code_set1(sc: u32) -> u32 {
    match sc {
        0 => 0,
        0x54 => KC_PRINT_SCREEN,
        0xE046 => KC_PAUSE,
        _ => ROWS.iter().find(|k| k.win as u32 == sc).map_or(0, |k| k.u as u32),
    }
}

/// (G7a) the usage of a Linux evdev key code (KEY_*, the X keycode minus 8); 0 unknown
pub fn key_code_evdev(code: u32) -> u32 {
    if code == 0 {
        return 0;
    }
    ROWS.iter().find(|k| k.ev as u32 == code).map_or(0, |k| k.u as u32)
}

/// (G7a) the usage of a macOS virtual key code (kVK_*, NSEvent.keyCode; kVK_ANSI_A is 0); 0
/// unknown
pub fn key_code_mac(kvk: u32) -> u32 {
    if kvk >= NO_MAC as u32 {
        return 0;
    }
    ROWS.iter().find(|k| k.mac as u32 == kvk).map_or(0, |k| k.u as u32)
}

/// (G7a) the browser's KeyboardEvent.code name of a usage ("KeyW", "ShiftLeft"); "" unknown
pub fn key_code_name(code: u32) -> &'static str {
    row(code).map_or("", |k| k.name)
}

/// (G7a) the usage of a KeyboardEvent.code name; 0 unknown.  Older names of Firefox ("OSLeft",
/// "OSRight", "VolumeMute", "VolumeUp", "VolumeDown") too
pub fn key_code_from_name(name: &str) -> u32 {
    match name {
        "" => 0,
        "OSLeft" => KC_META_LEFT,
        "OSRight" => KC_META_RIGHT,
        "VolumeMute" => KC_VOLUME_MUTE,
        "VolumeUp" => KC_VOLUME_UP,
        "VolumeDown" => KC_VOLUME_DOWN,
        _ => ROWS.iter().find(|k| k.name == name).map_or(0, |k| k.u as u32),
    }
}

fn row(code: u32) -> Option<&'static Row> {
    if code == 0 || code > 0xFF {
        return None;
    }
    ROWS.iter().find(|k| k.u as u32 == code)
}

/// (crate) Windows' scan code of a usage (0xE0.. extended; 0 none)
#[allow(dead_code)]
pub(crate) fn set1_of(code: u32) -> u32 {
    row(code).map_or(0, |k| k.win as u32)
}

/// (crate) Linux's evdev code of a usage (0 none)
#[allow(dead_code)]
pub(crate) fn evdev_of(code: u32) -> u32 {
    row(code).map_or(0, |k| k.ev as u32)
}

/// (crate) macOS' virtual key code of a usage (None: none)
#[allow(dead_code)]
pub(crate) fn mac_of(code: u32) -> Option<u32> {
    row(code).filter(|k| k.mac != NO_MAC).map(|k| k.mac as u32)
}

/// (G7a) the usage (KC_*) of a key event's Event::x on this platform: Windows' scan code, Linux's
/// and Android's evdev code, on macOS, iOS and in the browser the usage itself; 0 unknown
pub fn key_code(scancode: u32) -> u32 {
    #[cfg(windows)]
    return key_code_set1(scancode);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return key_code_evdev(scancode);
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    return row(scancode).map_or(0, |k| k.u as u32);
}

/// (G7a) Event::x of a key event on this platform for a usage (KC_*): what key_code turns back
/// into it; 0 where this platform has no code for it
pub fn key_scancode(code: u32) -> u32 {
    #[cfg(windows)]
    return set1_of(code);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return evdev_of(code);
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    return row(code).map_or(0, |k| k.u as u32);
}

/// (G7a) what the current keyboard layout prints on the key at usage `code`: its character
/// without Shift in upper case ("Z" for KC_W on a French keyboard, "Ü" for KC_BRACKET_LEFT on a
/// German one), else the system's name of the key where it has one (Windows: "Esc", "Enter" in
/// the system's language).  Windows (the thread's layout), Linux (the keymap of X11 or Wayland,
/// once a window has the keyboard), macOS (the current input source); None in the browser, on
/// Android and iOS and for keys the layout gives nothing
pub fn key_label(code: u32) -> Option<String> {
    if row(code).is_none() {
        return None;
    }
    #[cfg(windows)]
    return crate::win_key_label(set1_of(code));
    #[cfg(target_os = "linux")]
    return crate::xkb_key_label(evdev_of(code));
    #[cfg(target_os = "macos")]
    return mac_of(code).and_then(crate::mac_key_label);
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    return None;
}

/// (crate) a key's character as its label: letters in upper case, nothing for control characters
#[allow(dead_code)]
pub(crate) fn label_of(c: u32) -> Option<String> {
    let ch = char::from_u32(c).filter(|c| !c.is_control() && *c != ' ')?;
    let mut s = String::new();
    for u in ch.to_uppercase() {
        s.push(u);
    }
    Some(s)
}

/// (test aid, plattest --test input) every row of the table: (usage, Windows scan code, evdev
/// code, macOS key code or 0xFF, the browser's name)
pub fn key_code_rows_test() -> alloc::vec::Vec<(u32, u32, u32, u32, &'static str)> {
    ROWS.iter().map(|k| (k.u as u32, k.win as u32, k.ev as u32, k.mac as u32, k.name)).collect()
}
