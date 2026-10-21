// t_xkb -- linuxtest: the XKB keymap parser and key translation (German layout, US fallback)
//                                                    [apps/linuxtest/src/main.s: test_xkb]
//
//   linuxtest --shot x.png --test xkb --keymap F [--xkbdump]
//   parse the keymap file (a German layout: tools/wlkeymap.py dumps the compositor's),
//   translate keys with modifier states, check the MOD_* bits of the real modifiers and the
//   built-in US fallback

use alloc::format;

use crate::{Lt, out};

const ANY: u32 = 0xFFFF_FFFF; // VK not checked

/// evdev, depressed, locked, expected text, expected VK (ANY = not checked), name
type Xt = (u32, u32, u32, u32, u32, &'static str);

static DE: &[Xt] = &[
    (16, 0, 0, 'q' as u32, 'Q' as u32, "de: q"),
    (16, 1, 0, 'Q' as u32, 'Q' as u32, "de: Shift+q = Q"),
    (16, 0x80, 0, '@' as u32, 'Q' as u32, "de: AltGr+q = @"),
    (16, 0x84, 0, '@' as u32, 'Q' as u32, "de: Ctrl+AltGr+q = @ (AltGr sent as Ctrl+Mod5)"),
    (16, 0x04, 0, 0, 'Q' as u32, "de: Ctrl+q = no text, VK Q"),
    (16, 0x08, 0, 0, 'Q' as u32, "de: Alt+q = no text"),
    (16, 0, 0x02, 'Q' as u32, 'Q' as u32, "de: Caps+q = Q"),
    (16, 0x80, 0x02, '@' as u32, 'Q' as u32, "de: Caps+AltGr+q = @"),
    (18, 0x80, 0, 0x20AC, 'E' as u32, "de: AltGr+e = euro"),
    (8, 0x80, 0, '{' as u32, '7' as u32, "de: AltGr+7 = {"),
    (9, 0x80, 0, '[' as u32, '8' as u32, "de: AltGr+8 = ["),
    (10, 0x80, 0, ']' as u32, '9' as u32, "de: AltGr+9 = ]"),
    (11, 0x80, 0, '}' as u32, '0' as u32, "de: AltGr+0 = }"),
    (2, 1, 0, '!' as u32, '1' as u32, "de: Shift+1 = !"),
    (12, 0, 0, 0xDF, ANY, "de: sharp s"),
    (12, 1, 0, '?' as u32, ANY, "de: Shift+sharp s = ?"),
    (12, 0x80, 0, 0x5C, ANY, "de: AltGr+sharp s = backslash"),
    (40, 0, 0, 0xE4, ANY, "de: a-umlaut"),
    (40, 1, 0, 0xC4, ANY, "de: Shift+a-umlaut"),
    (40, 0, 0x02, 0xC4, ANY, "de: Caps+a-umlaut"),
    (40, 1, 0x02, 0xE4, ANY, "de: Caps+Shift+a-umlaut"),
    (39, 0, 0, 0xF6, ANY, "de: o-umlaut"),
    (26, 0, 0, 0xFC, ANY, "de: u-umlaut"),
    (26, 1, 0, 0xDC, ANY, "de: Shift+u-umlaut"),
    (21, 0, 0, 'z' as u32, 'Z' as u32, "de: key Y position = z, VK Z"),
    (44, 0, 0, 'y' as u32, 'Y' as u32, "de: key Z position = y, VK Y"),
    (86, 0x80, 0, '|' as u32, ANY, "de: AltGr+< = |"),
    (86, 1, 0, '>' as u32, ANY, "de: Shift+< = >"),
    (27, 0, 0, '+' as u32, 0xBB, "de: + (VK_OEM_PLUS)"),
    (27, 0x80, 0, '~' as u32, 0xBB, "de: AltGr++ = ~"),
    (53, 0, 0, '-' as u32, 0xBD, "de: - (VK_OEM_MINUS)"),
    (51, 1, 0, ';' as u32, 0xBC, "de: Shift+, = ;"),
    (13, 0, 0, 0xB4, ANY, "de: dead acute -> spacing acute"),
    (41, 0, 0, '^' as u32, ANY, "de: dead circumflex -> ^"),
    (41, 1, 0, 0xB0, ANY, "de: Shift+^ = degree"),
    (71, 0, 0x10, '7' as u32, 0x67, "de: NumLock KP7 = 7, VK_NUMPAD7"),
    (71, 0, 0, 0, 0x24, "de: KP7 without NumLock = VK_HOME"),
    (83, 0, 0x10, ',' as u32, 0x6E, "de: NumLock KP decimal = , (German)"),
    (55, 0, 0, '*' as u32, 0x6A, "de: KP * (VK_MULTIPLY)"),
    (96, 0, 0, 0, 0x0D, "de: KP Enter = VK_RETURN"),
    (57, 0, 0, ' ' as u32, 0x20, "de: space"),
    (28, 0, 0, 0, 0x0D, "de: Return: VK only"),
    (1, 0, 0, 0, 0x1B, "de: Escape"),
    (14, 0, 0, 0, 0x08, "de: Backspace"),
    (15, 0, 0, 0, 0x09, "de: Tab"),
    (103, 0, 0, 0, 0x26, "de: Up"),
    (59, 0, 0, 0, 0x70, "de: F1"),
    (100, 0, 0, 0, 0x12, "de: AltGr key = VK_MENU"),
];

static US: &[Xt] = &[
    (16, 0, 0, 'q' as u32, 'Q' as u32, "us fallback: q"),
    (16, 1, 0, 'Q' as u32, 'Q' as u32, "us fallback: Shift+q"),
    (16, 0, 0x02, 'Q' as u32, 'Q' as u32, "us fallback: Caps+q"),
    (3, 1, 0, '@' as u32, '2' as u32, "us fallback: Shift+2 = @"),
    (12, 0, 0, '-' as u32, 0xBD, "us fallback: -"),
    (71, 0, 0x10, '7' as u32, 0x67, "us fallback: NumLock KP7"),
];

// depressed, locked, expected MOD_* bits, name
static MODS: &[(u32, u32, u8, &str)] = &[
    (0x80, 0, plat::MOD_ALTGR, "mods: Mod5 -> MOD_ALTGR"),
    (0x01, 0, plat::MOD_SHIFT, "mods: Shift -> MOD_SHIFT"),
    (0, 0x02, plat::MOD_CAPS, "mods: Lock -> MOD_CAPS"),
    (0, 0x10, plat::MOD_NUM, "mods: Mod2 -> MOD_NUM"),
    (0x08, 0, plat::MOD_ALT, "mods: Mod1 -> MOD_ALT"),
    (0x40, 0, plat::MOD_SUPER, "mods: Mod4 -> MOD_SUPER"),
    (0x04, 0, plat::MOD_CTRL, "mods: Control -> MOD_CTRL"),
    (0x85, 0x12, plat::MOD_ALTGR | plat::MOD_CTRL | plat::MOD_SHIFT | plat::MOD_CAPS | plat::MOD_NUM, "mods: combined"),
];

/// set modifiers, translate, compare  [run_xkb_table]
fn run_table(t: &mut Lt, tab: &[Xt]) {
    for &(evdev, dep, lock, cp, vk, name) in tab {
        plat::xkb_set_mods(dep, 0, lock, 0);
        let (v, text) = plat::xkb_key(evdev);
        let ok = text == cp && (vk == ANY || v == vk);
        t.check(ok, name);
        if !ok {
            out(&format!("     got text {text:X} vk {v:X}, expected text {cp:X} vk {vk:X}\n"));
        }
    }
}

/// parse --keymap FILE and check German layout translations
pub fn test_xkb(t: &mut Lt) {
    let Some(text) = t.args.value("--keymap").and_then(plat::read_file) else {
        t.check(false, "--keymap FILE readable");
        return;
    };
    let t0 = plat::time_us();
    let r = plat::xkb_parse(&text);
    let dt = plat::time_us() - t0;
    out(&format!("xkb_parse: {} bytes in {} us\n", text.len(), dt));
    t.check(r == 0, "xkb_parse returns 0");
    if t.args.flag("--xkbdump") {
        plat::xkb_dump();
    }
    run_table(t, DE);
    // modifier bits
    for &(dep, lock, want, name) in MODS {
        let m = plat::xkb_set_mods(dep, 0, lock, 0);
        t.check(m as u8 == want, name);
    }
    let ok = plat::xkb_is_modifier(42) && plat::xkb_is_modifier(100) && plat::xkb_is_modifier(58)
        && !plat::xkb_is_modifier(16);
    t.check(ok, "is_modifier: LShift, AltGr, Caps yes; Q no");
    // built-in US fallback
    plat::xkb_use_fallback();
    run_table(t, US);
}
