// t_x11 -- linuxtest: the X11 backend without a server (keysyms, a German core keymap,
// synthetic X events) and with one (an X error, every cursor shape, MappingNotify)
//                                         [apps/linuxtest/src/main.s: test_x11synth, test_x11err]

use alloc::format;
use alloc::vec;

use plat::x11t;
use plat::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_X1, E_DOWN, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_LEAVE, E_MOVE, E_TEXT, E_UP, E_WHEEL};

use crate::Lt;

const ANY: i32 = 0x7FFFFFFF; // expect: position not checked
const VK_SHIFT: u32 = 0x10;

// keysym, expected value, name
static KS_TESTS: [(u32, u32, &str); 12] = [
    (0x1B3, 0x142, "keysym lstroke 0x1B3 -> U+0142"),
    (0x20AC, 0x20AC, "keysym EuroSign 0x20AC -> U+20AC"),
    (0x1000259, 0x259, "keysym U0259 -> U+0259"),
    (0xFE51, 0x400000B4, "keysym dead_acute -> dead | U+00B4"),
    (0xFFB7, 0x20000037, "keysym KP_7 -> keypad | 7"),
    (0x6C1, 0x430, "keysym Cyrillic_a -> U+0430"),
    (0x7D9, 0x3A9, "keysym Greek_OMEGA -> U+03A9"),
    (0xAAA, 0x2013, "keysym endash -> U+2013"),
    (0xFF0D, 0, "keysym Return -> no text"),
    (0xFFE1, 0x1000000, "keysym Shift_L -> modifier Shift"),
    (0xFE03, 0x8000000, "keysym ISO_Level3_Shift -> modifier Level3"),
    (0xE4, 0xE4, "keysym adiaeresis -> U+00E4"),
];

// German core mapping excerpt: keycode, G1L1 G1L2 G2L1 G2L2 G1L3 G1L4 G2L3 G2L4
static X11_KEYS: [(u8, [u32; 8]); 20] = [
    (24, [0x71, 0x51, 0x71, 0x51, 0x40, 0x7D9, 0x40, 0x7D9]),
    (26, [0x65, 0x45, 0x65, 0x45, 0x20AC, 0x20AC, 0x20AC, 0x20AC]),
    (16, [0x37, 0x2F, 0x37, 0x2F, 0x7B, 0xAC6, 0x7B, 0xAC6]),
    (20, [0xDF, 0x3F, 0xDF, 0x3F, 0x5C, 0xBF, 0x5C, 0xBF]),
    (48, [0xE4, 0xC4, 0xE4, 0xC4, 0xFE52, 0xFE5A, 0, 0]),
    (29, [0x7A, 0x5A, 0x7A, 0x5A, 0x8FB, 0xA5, 0, 0]),
    (45, [0x6B, 0, 0, 0, 0, 0, 0, 0]),
    (49, [0xFE52, 0xB0, 0xFE52, 0xB0, 0x1002032, 0x1002033, 0, 0]),
    (50, [0xFFE1, 0, 0xFFE1, 0, 0, 0, 0, 0]),
    (62, [0xFFE2, 0, 0xFFE2, 0, 0, 0, 0, 0]),
    (66, [0xFFE5, 0, 0xFFE5, 0, 0, 0, 0, 0]),
    (37, [0xFFE3, 0, 0xFFE3, 0, 0, 0, 0, 0]),
    (105, [0xFFE4, 0, 0xFFE4, 0, 0, 0, 0, 0]),
    (64, [0xFFE9, 0xFFE7, 0xFFE9, 0xFFE7, 0, 0, 0, 0]),
    (77, [0xFF7F, 0, 0xFF7F, 0, 0, 0, 0, 0]),
    (133, [0xFFEB, 0, 0xFFEB, 0, 0, 0, 0, 0]),
    (108, [0xFE03, 0, 0xFE03, 0, 0, 0, 0, 0]),
    (79, [0xFF95, 0xFFB7, 0xFF95, 0xFFB7, 0, 0, 0, 0]),
    (36, [0xFF0D, 0, 0xFF0D, 0, 0, 0, 0, 0]),
    (38, [0x61, 0x41, 0x6C6, 0x6E6, 0x61, 0x41, 0x6C6, 0x6E6]),
];
// modifier map, 2 keycodes per modifier: Shift Lock Control Mod1..Mod5
static X11_MODMAP: [u8; 16] = [50, 62, 66, 0, 37, 105, 64, 0, 77, 0, 0, 0, 133, 0, 108, 0];

/// synthetic X input events (x11_event) with an increasing time
struct Synth {
    time: u32,
}

impl Synth {
    /// xev(type | detail << 8, state, x, y, extra): one input event (extra = Enter / Leave mode)
    fn xe(&mut self, typ: u8, detail: u8, state: u16, x: i32, y: i32, extra: u8) {
        let mut p = [0u8; 32];
        p[0] = typ;
        p[1] = detail;
        self.time += 1;
        p[4..8].copy_from_slice(&self.time.to_le_bytes());
        for o in [20, 24] {
            p[o..o + 2].copy_from_slice(&(x as u16).to_le_bytes());
            p[o + 2..o + 4].copy_from_slice(&(y as u16).to_le_bytes());
        }
        p[28..30].copy_from_slice(&state.to_le_bytes());
        p[30] = extra;
        x11t::event(&p);
    }

    /// XKEY: press and release a key, expecting VK (+ text)
    fn key(&mut self, t: &mut Lt, kc: u8, state: u16, vk: u32, text: u32, name: &str) {
        self.xe(2, kc, state, 0, 0, 0);
        t.expect(E_KEY_DOWN as u32, ANY, 0, vk, &format!("{name}: key down"));
        if text != 0 {
            t.expect(E_TEXT as u32, ANY, 0, text, &format!("{name}: text"));
        }
        self.xe(3, kc, state, 0, 0, 0);
        t.expect(E_KEY_UP as u32, ANY, 0, vk, &format!("{name}: key up"));
    }
}

/// xfocus(type, detail, mode): synthetic FocusIn / FocusOut
fn xfocus(typ: u8, detail: u8, mode: u8) {
    let mut p = [0u8; 32];
    p[0] = typ;
    p[1] = detail;
    p[8] = mode;
    x11t::event(&p);
}

fn tb(typ: u8, btn: u8) -> u32 {
    typ as u32 | (btn as u32) << 8
}

/// keysym conversion, a German core keyboard mapping and synthetic X11 key / pointer /
/// focus events -> E_* (no X server needed: run with --shot)
pub fn test_x11synth(t: &mut Lt) {
    plat::evq_clear();
    // numeric keysyms -> xkb symbol values
    for &(ks, want, name) in KS_TESTS.iter() {
        t.check(x11t::ks_value(ks) == want, name);
    }
    // expand the sparse key table into keycodes 8..135 x 8 columns
    let mut xks = vec![0u32; 128 * 8];
    for (kc, cols) in X11_KEYS.iter() {
        let o = (*kc as usize - 8) * 8;
        xks[o..o + 8].copy_from_slice(cols);
    }
    x11t::kb_set_keysyms(&xks, 8, 128, 8);
    x11t::kb_set_modmap(&X11_MODMAP, 2);
    x11t::kb_build(0);
    let mut s = Synth { time: 0 };
    // keys (German layout)
    s.xe(2, 24, 0, 0, 0, 0);
    t.expect(E_KEY_DOWN as u32, ANY, 0, 'Q' as u32, "x11 q: key down Q");
    t.expect(E_TEXT as u32, ANY, 0, 'q' as u32, "x11 q: text q");
    s.xe(2, 24, 0, 0, 0, 0);
    t.expect(E_KEY_DOWN as u32, ANY, 0, 0x80000051, "x11 q again while held: repeat bit");
    t.expect(E_TEXT as u32, ANY, 0, 'q' as u32, "x11 q repeat: text");
    s.xe(3, 24, 0, 0, 0, 0);
    t.expect(E_KEY_UP as u32, ANY, 0, 'Q' as u32, "x11 q release");
    s.key(t, 24, 1, 'Q' as u32, 'Q' as u32, "x11 Shift+q = Q");
    s.key(t, 24, 0x80, 'Q' as u32, '@' as u32, "x11 AltGr+q = @");
    s.key(t, 24, 0x84, 'Q' as u32, '@' as u32, "x11 Ctrl+AltGr+q = @");
    s.key(t, 24, 0x04, 'Q' as u32, 0, "x11 Ctrl+q: no text");
    s.key(t, 24, 0x02, 'Q' as u32, 'Q' as u32, "x11 Caps+q = Q");
    s.key(t, 26, 0x80, 'E' as u32, 0x20AC, "x11 AltGr+e = euro");
    s.key(t, 16, 1, '7' as u32, '/' as u32, "x11 Shift+7 = /");
    s.key(t, 16, 0x80, '7' as u32, '{' as u32, "x11 AltGr+7 = {");
    s.key(t, 20, 0x80, 0xBD, 0x5C, "x11 AltGr+sharp s = backslash");
    s.key(t, 20, 1, 0xBD, '?' as u32, "x11 Shift+sharp s = ?");
    s.key(t, 48, 0, 0xDE, 0xE4, "x11 a-umlaut");
    s.key(t, 48, 2, 0xDE, 0xC4, "x11 Caps+a-umlaut");
    s.key(t, 29, 0, 'Z' as u32, 'z' as u32, "x11 key at US-Y = z, VK Z");
    s.key(t, 45, 0, 'K' as u32, 'k' as u32, "x11 single keysym k");
    s.key(t, 45, 1, 'K' as u32, 'K' as u32, "x11 single keysym k + Shift = K (core rule)");
    s.key(t, 49, 0, 0xC0, '^' as u32, "x11 dead circumflex -> ^");
    s.key(t, 49, 1, 0xC0, 0xB0, "x11 Shift+^ = degree");
    s.key(t, 79, 0x10, 0x67, '7' as u32, "x11 NumLock KP7 = 7, VK_NUMPAD7");
    s.key(t, 79, 0, 0x24, 0, "x11 KP7 without NumLock = VK_HOME");
    s.key(t, 36, 0, 0x0D, 0, "x11 Return: VK only");
    // a second layout (XKB group 2 in state bits 13-14): Russian on the A key
    x11t::set_xkb_on(true);
    s.key(t, 38, 0x2000, 'A' as u32, 0x444, "x11 group 2: A key = Cyrillic ef, VK A");
    s.key(t, 38, 0x2001, 'A' as u32, 0x424, "x11 group 2 + Shift = Cyrillic EF");
    s.key(t, 45, 0x2000, 'K' as u32, 'k' as u32, "x11 group 2 without own symbols falls back to group 1");
    s.key(t, 38, 0, 'A' as u32, 'a' as u32, "x11 back to group 1");
    x11t::set_xkb_on(false);
    s.xe(2, 50, 0, 0, 0, 0);
    t.expect(E_KEY_DOWN as u32, ANY, 0, VK_SHIFT, "x11 Shift down");
    t.check(x11t::mods() & plat::MOD_SHIFT as u32 != 0, "x11 Shift key event already carries MOD_SHIFT (like Win32)");
    s.xe(3, 50, 1, 0, 0, 0);
    t.expect(E_KEY_UP as u32, ANY, 0, VK_SHIFT, "x11 Shift up");
    t.check(x11t::mods() & plat::MOD_SHIFT as u32 == 0, "x11 Shift release clears MOD_SHIFT");
    // pointer
    s.xe(7, 0, 0, 10, 20, 0);
    t.expect(tb(E_MOVE, 0), 10, 20, 0, "x11 enter -> E_MOVE");
    s.xe(6, 0, 0, 30, 40, 0);
    t.expect(tb(E_MOVE, 0), 30, 40, 0, "x11 motion -> E_MOVE");
    s.xe(4, 1, 0, 30, 40, 0);
    t.expect(tb(E_DOWN, BTN_LEFT), 30, 40, 0, "x11 button 1 -> E_DOWN left");
    s.xe(6, 0, 0x100, 35, -45, 0);
    t.expect(tb(E_MOVE, 0), 35, -45, 0, "x11 drag motion (negative y)");
    s.xe(8, 0, 0x100, -5, 45, 0);
    s.xe(5, 1, 0x100, -5, 45, 0);
    t.expect(tb(E_UP, BTN_LEFT), -5, 45, 0, "x11 release outside -> E_UP");
    t.expect(tb(E_LEAVE, 0), 0, 0, 0, "x11 leave during a drag comes after the release");
    s.xe(7, 0, 0, 1, 2, 0);
    t.expect(tb(E_MOVE, 0), 1, 2, 0, "x11 enter again");
    s.xe(4, 4, 0, 1, 2, 0);
    t.expect(tb(E_WHEEL, 0), 1, 2, 120, "x11 button 4 -> wheel up 120");
    s.xe(5, 4, 0, 1, 2, 0);
    s.xe(4, 5, 0, 1, 2, 0);
    t.expect(tb(E_WHEEL, 0), 1, 2, 0xFF88, "x11 button 5 -> wheel down -120");
    s.xe(4, 6, 0, 1, 2, 0);
    t.expect(tb(E_WHEEL, 0), 1, 2, 0xFF880000, "x11 button 6 -> dx -120");
    s.xe(4, 7, 0, 1, 2, 0);
    t.expect(tb(E_WHEEL, 0), 1, 2, 0x00780000, "x11 button 7 -> dx +120");
    s.xe(4, 3, 0, 1, 2, 0);
    t.expect(tb(E_DOWN, BTN_RIGHT), 1, 2, 0, "x11 button 3 -> right");
    s.xe(4, 2, 0, 1, 2, 0);
    t.expect(tb(E_DOWN, BTN_MIDDLE), 1, 2, 0, "x11 button 2 -> middle");
    s.xe(4, 8, 0, 1, 2, 0);
    t.expect(tb(E_DOWN, BTN_X1), 1, 2, 0, "x11 button 8 -> X1");
    s.xe(8, 0, 0, 1, 2, 1);
    t.expect(tb(E_UP, BTN_RIGHT), 1, 2, 0, "x11 grab leave releases right");
    t.expect(tb(E_UP, BTN_MIDDLE), 1, 2, 0, "x11 grab leave releases middle");
    t.expect(tb(E_UP, BTN_X1), 1, 2, 0, "x11 grab leave releases X1");
    t.expect(tb(E_LEAVE, 0), 0, 0, 0, "x11 grab leave -> E_LEAVE");
    // focus
    xfocus(9, 3, 0);
    t.expect(tb(E_FOCUS_IN, 0), 0, 0, 0, "x11 FocusIn -> E_FOCUS_IN");
    xfocus(9, 3, 0);
    xfocus(10, 3, 1); // NotifyGrab: ignored
    xfocus(10, 3, 0);
    t.expect(tb(E_FOCUS_OUT, 0), 0, 0, 0, "x11 FocusOut -> E_FOCUS_OUT (duplicates / grabs ignored)");
    t.check(plat::evq_count() == 0, "x11 no stray events");
}

/// a request with a bad window id: the X error is recorded, the program keeps running
/// (needs the X11 backend)
pub fn test_x11err(t: &mut Lt) {
    if plat::disp_backend() != plat::BK_X11 {
        t.check(false, "x11err needs the X11 backend (jfgui_BACKEND=x11)");
        return;
    }
    t.redraw();
    let seq = x11t::req(8, &0x7FFFFF0u32.to_le_bytes()); // MapWindow
    x11t::sync();
    t.check(x11t::err_last() as u16 == seq as u16, "bad request -> X error recorded");
    // every cursor shape (glyph cursors, the empty one, back to the arrow)
    let e0 = x11t::err_last();
    for shape in 0..plat::CUR_COUNT {
        plat::set_cursor(shape);
        x11t::sync();
    }
    plat::set_cursor(0);
    x11t::sync();
    t.check(e0 == x11t::err_last(), "every CUR_* shape without an X error");
    // MappingNotify (keyboard): the mapping is fetched again in wait
    let mut p = [0u8; 32];
    p[0] = 34;
    p[4] = 1;
    x11t::event(&p);
    plat::wait(0);
    plat::xkb_set_mods(0, 0, 0, 0);
    let (vk, text) = plat::xkb_key(16); // evdev Q
    t.check(vk == 'Q' as u32 && text == 'q' as u32, "MappingNotify -> keymap fetched again, Q still translates");
    t.redraw();
    t.pump(300, 0);
    t.check(true, "still running after the X error");
}
