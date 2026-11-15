// t_keychar -- (E2) the keyboard layout's character of a key event (plat::key_y, KeyEvent::ch):
// what the key types with Shift, Caps Lock and AltGr but without Ctrl, Alt and the Windows /
// Super key
//
//   plattest --test keychar   Event::y's packing (location, character, dead key); Windows: the
//                             layouts loaded for the user through ToUnicodeEx (German and English
//                             (US) when loaded: Ctrl+Z, Ctrl++, AltGr+Q, a dead key, keys without
//                             a character; the thread's keyboard state stays); Linux: the
//                             fallback (US) keymap with Ctrl, Shift and Caps Lock held

use plat::{key_char, key_dead, key_location, key_y, KL_LEFT, KL_NUMPAD, KL_STANDARD};

use crate::T;

pub fn t_keychar(t: &mut T) {
    // Event::y: the location in bits 0..7, the character above it, a dead key's flag
    let y = key_y(KL_NUMPAD, '5' as u32, false);
    t.check(key_location(y) == KL_NUMPAD && key_char(y) == '5' as u32 && !key_dead(y), "keychar: y of the number block's 5");
    let y = key_y(KL_STANDARD, 0x1F600, false);
    t.check(key_char(y) == 0x1F600 && key_location(y) == KL_STANDARD && y > 0, "keychar: a character beyond the BMP, y stays positive");
    let y = key_y(KL_STANDARD, 0xB4, true);
    t.check(key_char(y) == 0xB4 && key_dead(y), "keychar: a dead key (´)");
    t.check(key_y(KL_LEFT, 0, true) == KL_LEFT as i32, "keychar: no character: the location alone (no dead flag), as before");
    t.check(key_char(key_y(KL_STANDARD, 0x11_0000, false)) == 0, "keychar: beyond U+10FFFF: none");
    os(t);
}

#[cfg(windows)]
fn os(t: &mut T) {
    use alloc::format;
    use plat::{key_char_test, key_layouts_test, MOD_ALT, MOD_ALTGR, MOD_CAPS, MOD_CTRL, MOD_SHIFT};
    const VK_Z: u32 = 0x5A;
    const VK_Q: u32 = 0x51;
    const VK_2: u32 = 0x32;
    const VK_OEM_PLUS: u32 = 0xBB;
    const VK_OEM_6: u32 = 0xDD; // German ´ (dead)
    const VK_RETURN: u32 = 0x0D;
    const VK_F5: u32 = 0x74;
    const VK_NUMPAD5: u32 = 0x65;
    let layouts = key_layouts_test();
    t.info(&format!("  (layouts: {:x?})\n", layouts));
    t.check(!layouts.is_empty(), "keychar: the user has keyboard layouts");
    // every layout: Ctrl and Alt change nothing; keys without a character have none
    for &hkl in &layouts {
        let z = key_char_test(hkl, VK_Z, 0).0;
        t.check(key_char_test(hkl, VK_Z, MOD_CTRL).0 == z && key_char_test(hkl, VK_Z, MOD_ALT).0 == z,
            &format!("keychar {:x}: Ctrl+Z, Alt+Z give Z's character", hkl & 0xFFFF));
        t.check(key_char_test(hkl, VK_RETURN, MOD_CTRL) == (0, false) && key_char_test(hkl, VK_F5, 0) == (0, false),
            &format!("keychar {:x}: Ctrl+Enter, F5: none", hkl & 0xFFFF));
    }
    if let Some(&de) = layouts.iter().find(|&&h| h & 0xFFFF == 0x0407) {
        t.check(key_char_test(de, VK_Z, MOD_CTRL) == ('z' as u32, false), "keychar de: Ctrl+Z -> z");
        t.check(key_char_test(de, VK_Z, MOD_CTRL | MOD_SHIFT) == ('Z' as u32, false), "keychar de: Ctrl+Shift+Z -> Z");
        t.check(key_char_test(de, VK_Z, MOD_CAPS) == ('Z' as u32, false), "keychar de: Caps Lock + Z -> Z");
        t.check(key_char_test(de, VK_OEM_PLUS, MOD_CTRL) == ('+' as u32, false), "keychar de: Ctrl++ -> +");
        t.check(key_char_test(de, VK_Q, MOD_ALTGR) == ('@' as u32, false), "keychar de: AltGr+Q -> @");
        t.check(key_char_test(de, VK_Z, MOD_ALTGR) == ('z' as u32, false), "keychar de: AltGr+Z (none of its own) -> z");
        t.check(key_char_test(de, VK_OEM_6, 0) == (0xB4, true), "keychar de: ´ is a dead key");
        t.check(key_char_test(de, VK_NUMPAD5, MOD_CTRL) == ('5' as u32, false), "keychar de: Ctrl + the number block's 5 -> 5");
    } else {
        t.info("  (no German layout loaded: its checks skipped)\n");
    }
    if let Some(&us) = layouts.iter().find(|&&h| h == 0x0409_0409) {
        t.check(key_char_test(us, VK_OEM_PLUS, MOD_CTRL) == ('=' as u32, false), "keychar us: Ctrl+= -> =");
        t.check(key_char_test(us, VK_2, MOD_CTRL | MOD_SHIFT) == ('@' as u32, false), "keychar us: Ctrl+Shift+2 -> @");
    } else {
        t.info("  (no English (US) layout loaded: its checks skipped)\n");
    }
}

#[cfg(target_os = "linux")]
fn os(t: &mut T) {
    // the US fallback keymap; evdev 44 = Z, 13 = the US layout's "=", 28 = Enter
    plat::xkb_use_fallback();
    plat::xkb_set_mods(0x04, 0, 0, 0); // Ctrl
    t.check(key_char(plat::xkb_key_y(44)) == 'z' as u32, "keychar xkb: Ctrl+Z -> z");
    t.check(plat::xkb_key(44).1 == 0, "keychar xkb: Ctrl+Z types no text");
    plat::xkb_set_mods(0x05, 0, 0, 0); // Ctrl + Shift
    t.check(key_char(plat::xkb_key_y(44)) == 'Z' as u32, "keychar xkb: Ctrl+Shift+Z -> Z");
    plat::xkb_set_mods(0x04, 0, 0x02, 0); // Ctrl, Caps Lock
    t.check(key_char(plat::xkb_key_y(44)) == 'Z' as u32, "keychar xkb: Ctrl + Caps Lock + Z -> Z");
    plat::xkb_set_mods(0x04, 0, 0, 0);
    t.check(key_char(plat::xkb_key_y(13)) == '=' as u32, "keychar xkb: Ctrl+= -> =");
    t.check(plat::xkb_key_y(28) == KL_STANDARD as i32, "keychar xkb: Ctrl+Enter: none");
    t.check(plat::xkb_key_y(29) == KL_LEFT as i32, "keychar xkb: the left Ctrl: its place, no character");
    plat::xkb_set_mods(0, 0, 0, 0);
}

#[cfg(not(any(windows, target_os = "linux")))]
fn os(t: &mut T) {
    t.info("  (the layout's character of this system: not checked here)\n");
}
