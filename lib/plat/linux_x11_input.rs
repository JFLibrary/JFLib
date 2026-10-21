// linux_x11_input -- X11 keyboard and pointer input                [src/plat/linux/x11_input.s]
//
//   push(type, btn, x, y, data)           evq_push with the current MOD_* bits and window
//   input_init()                          XKB: detectable auto-repeat, keymap change events;
//                                         then the keyboard mapping
//   kb_load()                             GetKeyboardMapping + GetModifierMapping
//   kb_set_keysyms(keysyms, first, count, per)   core mapping (also for tests)
//   kb_set_modmap(keycodes, per)          modifier roles (Alt, NumLock, Super, AltGr)
//   kb_build(group)                       core mapping -> the key records of linux_xkb.rs
//   ks_value(keysym) -> symbol value      keysym as an xkb.s symbol value
//   key / button / motion / enter / leave / focus   event handlers (event())
//   ime_pos(x, y, h)                      the focused text field's caret (window px, h 0 =
//                                         none): IBus focus and cursor location while a window
//                                         of ours has the keyboard (its FocusOut ends it); then
//                                         every key goes to IBus first (linux_ibus.rs) and a
//                                         pressed key it takes sends no event
//   release_buttons()                     E_UP for every held button
//
// Keys: X keycode = evdev + 8 (Xorg evdev/libinput, Xwayland).  The core mapping columns
// follow the XKB convention G1L1 G1L2 G2L1 G2L2 G1L3 G1L4 G2L3 G2L4; the group comes from
// the event state (bits 13-14, XKB clients).  Levels, Caps Lock, Num Lock, text and VK codes
// are computed by the key table of linux_xkb.rs exactly as for Wayland.  The event state is the state
// *before* the event; like Win32 (and the Wayland backend) a modifier's own key event
// already carries its new MOD_* bit.  Auto-repeat: XKB detectable auto-repeat (press, press,
// ..., release); without it a release followed by a press of the same key with the same time
// is one repeat.  Pointer: buttons 1/2/3/8/9 -> left/middle/right/X1/X2, 4/5 wheel up/down and
// 6/7 left/right (120 per notch); motion is compressed while no button is held.

use super::*;
use crate::{
    BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_X1, BTN_X2, E_DOWN, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_LEAVE,
    E_MOVE, E_TEXT, E_UP, E_WHEEL, EVQ_CAP, Event, WST_MINIMIZED,
};

const KV_KP: u32 = 0x20000000; // xkb.s symbol value flags
const KV_DEAD: u32 = 0x40000000;
const MSV_SHIFT: u32 = 0x1000000; // modifier id << 24 (xkb.s MS_*)
const MSV_CTRL: u32 = 0x2000000;
const MSV_CAPS: u32 = 0x3000000;
const MSV_ALT: u32 = 0x4000000;
const MSV_META: u32 = 0x5000000;
const MSV_SUPER: u32 = 0x6000000;
const MSV_HYPER: u32 = 0x7000000;
const MSV_L3: u32 = 0x8000000;
const MSV_MODESW: u32 = 0x9000000;
const MSV_NUM: u32 = 0xA000000;
const MSV_L5: u32 = 0xB000000;
const MSV_GROUP: u32 = 0xC000000;
pub(super) const KS_COLS: usize = 8; // keysym columns kept per keycode
const XKB_CORE_KBD: u32 = 0x100; // XkbUseCoreKbd
const EV_COMPRESS: u32 = (EVQ_CAP / 2) as u32;

// buttons 4..7 -> E_WHEEL data (i16 dy | i16 dx << 16; up / right positive)
static WHEEL_DATA: [u32; 4] = [120, 0xFF88, 0xFF880000, 120 << 16];
// X button -> BTN_* (0 = none)
static BUTTON_MAP: [u8; 10] = [0, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, 0, 0, 0, 0, BTN_X1, BTN_X2];

// function, keypad, modifier and dead keysyms (0xFE00..0xFFFF)
static KS_SPECIAL: &[(u16, u32)] = &[
    (0xFFE1, MSV_SHIFT),
    (0xFFE2, MSV_SHIFT),
    (0xFFE3, MSV_CTRL),
    (0xFFE4, MSV_CTRL),
    (0xFFE5, MSV_CAPS),
    (0xFFE6, MSV_CAPS),
    (0xFFE7, MSV_META),
    (0xFFE8, MSV_META),
    (0xFFE9, MSV_ALT),
    (0xFFEA, MSV_ALT),
    (0xFFEB, MSV_SUPER),
    (0xFFEC, MSV_SUPER),
    (0xFFED, MSV_HYPER),
    (0xFFEE, MSV_HYPER),
    (0xFE03, MSV_L3), // ISO_Level3_Shift
    (0xFE04, MSV_L3),
    (0xFE05, MSV_L3),
    (0xFF7E, MSV_MODESW), // Mode_switch
    (0xFF7F, MSV_NUM),    // Num_Lock
    (0xFE11, MSV_L5),
    (0xFE12, MSV_L5),
    (0xFE13, MSV_L5),
    (0xFE06, MSV_GROUP),
    (0xFE07, MSV_GROUP),
    (0xFE08, MSV_GROUP),
    (0xFE09, MSV_GROUP),
    (0xFE0A, MSV_GROUP),
    (0xFE0B, MSV_GROUP),
    (0xFE0C, MSV_GROUP),
    (0xFE0D, MSV_GROUP),
    (0xFE0E, MSV_GROUP),
    (0xFE0F, MSV_GROUP),
    // keypad
    (0xFF80, KV_KP | 0x20), // KP_Space
    (0xFF89, KV_KP),        // KP_Tab
    (0xFF8D, KV_KP),        // KP_Enter
    (0xFF91, KV_KP),
    (0xFF92, KV_KP),
    (0xFF93, KV_KP),
    (0xFF94, KV_KP),
    (0xFF95, KV_KP), // KP_Home
    (0xFF96, KV_KP),
    (0xFF97, KV_KP),
    (0xFF98, KV_KP),
    (0xFF99, KV_KP),
    (0xFF9A, KV_KP),
    (0xFF9B, KV_KP),
    (0xFF9C, KV_KP),
    (0xFF9D, KV_KP),
    (0xFF9E, KV_KP),
    (0xFF9F, KV_KP), // KP_Delete
    (0xFFAA, KV_KP | 0x2A),
    (0xFFAB, KV_KP | 0x2B),
    (0xFFAC, KV_KP | 0x2C), // KP_Separator
    (0xFFAD, KV_KP | 0x2D),
    (0xFFAE, KV_KP | 0x2E), // KP_Decimal
    (0xFFAF, KV_KP | 0x2F),
    (0xFFB0, KV_KP | 0x30),
    (0xFFB1, KV_KP | 0x31),
    (0xFFB2, KV_KP | 0x32),
    (0xFFB3, KV_KP | 0x33),
    (0xFFB4, KV_KP | 0x34),
    (0xFFB5, KV_KP | 0x35),
    (0xFFB6, KV_KP | 0x36),
    (0xFFB7, KV_KP | 0x37),
    (0xFFB8, KV_KP | 0x38),
    (0xFFB9, KV_KP | 0x39),
    (0xFFBD, KV_KP | 0x3D),
    // dead keys: their spacing character (no compose, like the Wayland backend)
    (0xFE50, KV_DEAD | 0x60),
    (0xFE51, KV_DEAD | 0xB4),
    (0xFE52, KV_DEAD | 0x5E),
    (0xFE53, KV_DEAD | 0x7E),
    (0xFE54, KV_DEAD | 0xAF),
    (0xFE55, KV_DEAD | 0x2D8),
    (0xFE56, KV_DEAD | 0x2D9),
    (0xFE57, KV_DEAD | 0xA8),
    (0xFE58, KV_DEAD | 0xB0),
    (0xFE59, KV_DEAD | 0x2DD),
    (0xFE5A, KV_DEAD | 0x2C7),
    (0xFE5B, KV_DEAD | 0xB8),
    (0xFE5C, KV_DEAD | 0x2DB),
    (0xFE5D, KV_DEAD | 0x37A),
    (0xFE60, KV_DEAD | 0x2E),
    (0xFE61, KV_DEAD | 0x2C0),
    (0xFE62, KV_DEAD | 0x31B),
    (0xFE63, KV_DEAD | 0x2F),
    (0xFE68, KV_DEAD | 0x5F),
    (0xFE6E, KV_DEAD | 0x2C),
    (0xFE6F, KV_DEAD | 0xA4),
    (0xFE8C, KV_DEAD | 0x3BC),
];

// legacy text keysyms (Latin-2/3/4/9, Cyrillic, Greek, publishing, technical):
// u16 keysym, u16 codepoint (generated from the ISO 8859 / KOI8-R code pages)
static KS_TEXT: &[u16] = &[
    0x01A1, 0x0104, 0x01A2, 0x02D8, 0x01A3, 0x0141, 0x01A5, 0x013D, 0x01A6, 0x015A, 0x01A9, 0x0160,
    0x01AA, 0x015E, 0x01AB, 0x0164, 0x01AC, 0x0179, 0x01AE, 0x017D, 0x01AF, 0x017B, 0x01B1, 0x0105,
    0x01B2, 0x02DB, 0x01B3, 0x0142, 0x01B5, 0x013E, 0x01B6, 0x015B, 0x01B7, 0x02C7, 0x01B9, 0x0161,
    0x01BA, 0x015F, 0x01BB, 0x0165, 0x01BC, 0x017A, 0x01BD, 0x02DD, 0x01BE, 0x017E, 0x01BF, 0x017C,
    0x01C0, 0x0154, 0x01C3, 0x0102, 0x01C5, 0x0139, 0x01C6, 0x0106, 0x01C8, 0x010C, 0x01CA, 0x0118,
    0x01CC, 0x011A, 0x01CF, 0x010E, 0x01D0, 0x0110, 0x01D1, 0x0143, 0x01D2, 0x0147, 0x01D5, 0x0150,
    0x01D8, 0x0158, 0x01D9, 0x016E, 0x01DB, 0x0170, 0x01DE, 0x0162, 0x01E0, 0x0155, 0x01E3, 0x0103,
    0x01E5, 0x013A, 0x01E6, 0x0107, 0x01E8, 0x010D, 0x01EA, 0x0119, 0x01EC, 0x011B, 0x01EF, 0x010F,
    0x01F0, 0x0111, 0x01F1, 0x0144, 0x01F2, 0x0148, 0x01F5, 0x0151, 0x01F8, 0x0159, 0x01F9, 0x016F,
    0x01FB, 0x0171, 0x01FE, 0x0163, 0x01FF, 0x02D9, 0x02A1, 0x0126, 0x02A2, 0x02D8, 0x02A6, 0x0124,
    0x02A9, 0x0130, 0x02AA, 0x015E, 0x02AB, 0x011E, 0x02AC, 0x0134, 0x02AF, 0x017B, 0x02B1, 0x0127,
    0x02B6, 0x0125, 0x02B9, 0x0131, 0x02BA, 0x015F, 0x02BB, 0x011F, 0x02BC, 0x0135, 0x02BF, 0x017C,
    0x02C5, 0x010A, 0x02C6, 0x0108, 0x02D5, 0x0120, 0x02D8, 0x011C, 0x02DD, 0x016C, 0x02DE, 0x015C,
    0x02E5, 0x010B, 0x02E6, 0x0109, 0x02F5, 0x0121, 0x02F8, 0x011D, 0x02FD, 0x016D, 0x02FE, 0x015D,
    0x02FF, 0x02D9, 0x03A1, 0x0104, 0x03A2, 0x0138, 0x03A3, 0x0156, 0x03A5, 0x0128, 0x03A6, 0x013B,
    0x03A9, 0x0160, 0x03AA, 0x0112, 0x03AB, 0x0122, 0x03AC, 0x0166, 0x03AE, 0x017D, 0x03B1, 0x0105,
    0x03B2, 0x02DB, 0x03B3, 0x0157, 0x03B5, 0x0129, 0x03B6, 0x013C, 0x03B7, 0x02C7, 0x03B9, 0x0161,
    0x03BA, 0x0113, 0x03BB, 0x0123, 0x03BC, 0x0167, 0x03BD, 0x014A, 0x03BE, 0x017E, 0x03BF, 0x014B,
    0x03C0, 0x0100, 0x03C7, 0x012E, 0x03C8, 0x010C, 0x03CA, 0x0118, 0x03CC, 0x0116, 0x03CF, 0x012A,
    0x03D0, 0x0110, 0x03D1, 0x0145, 0x03D2, 0x014C, 0x03D3, 0x0136, 0x03D9, 0x0172, 0x03DD, 0x0168,
    0x03DE, 0x016A, 0x03E0, 0x0101, 0x03E7, 0x012F, 0x03E8, 0x010D, 0x03EA, 0x0119, 0x03EC, 0x0117,
    0x03EF, 0x012B, 0x03F0, 0x0111, 0x03F1, 0x0146, 0x03F2, 0x014D, 0x03F3, 0x0137, 0x03F9, 0x0173,
    0x03FD, 0x0169, 0x03FE, 0x016B, 0x03FF, 0x02D9, 0x06A1, 0x0452, 0x06A2, 0x0453, 0x06A3, 0x0451,
    0x06A4, 0x0454, 0x06A5, 0x0455, 0x06A6, 0x0456, 0x06A7, 0x0457, 0x06A8, 0x0458, 0x06A9, 0x0459,
    0x06AA, 0x045A, 0x06AB, 0x045B, 0x06AC, 0x045C, 0x06AD, 0x0491, 0x06AE, 0x045E, 0x06AF, 0x045F,
    0x06B0, 0x2116, 0x06B1, 0x0402, 0x06B2, 0x0403, 0x06B3, 0x0401, 0x06B4, 0x0404, 0x06B5, 0x0405,
    0x06B6, 0x0406, 0x06B7, 0x0407, 0x06B8, 0x0408, 0x06B9, 0x0409, 0x06BA, 0x040A, 0x06BB, 0x040B,
    0x06BC, 0x040C, 0x06BD, 0x0490, 0x06BE, 0x040E, 0x06BF, 0x040F, 0x06C0, 0x044E, 0x06C1, 0x0430,
    0x06C2, 0x0431, 0x06C3, 0x0446, 0x06C4, 0x0434, 0x06C5, 0x0435, 0x06C6, 0x0444, 0x06C7, 0x0433,
    0x06C8, 0x0445, 0x06C9, 0x0438, 0x06CA, 0x0439, 0x06CB, 0x043A, 0x06CC, 0x043B, 0x06CD, 0x043C,
    0x06CE, 0x043D, 0x06CF, 0x043E, 0x06D0, 0x043F, 0x06D1, 0x044F, 0x06D2, 0x0440, 0x06D3, 0x0441,
    0x06D4, 0x0442, 0x06D5, 0x0443, 0x06D6, 0x0436, 0x06D7, 0x0432, 0x06D8, 0x044C, 0x06D9, 0x044B,
    0x06DA, 0x0437, 0x06DB, 0x0448, 0x06DC, 0x044D, 0x06DD, 0x0449, 0x06DE, 0x0447, 0x06DF, 0x044A,
    0x06E0, 0x042E, 0x06E1, 0x0410, 0x06E2, 0x0411, 0x06E3, 0x0426, 0x06E4, 0x0414, 0x06E5, 0x0415,
    0x06E6, 0x0424, 0x06E7, 0x0413, 0x06E8, 0x0425, 0x06E9, 0x0418, 0x06EA, 0x0419, 0x06EB, 0x041A,
    0x06EC, 0x041B, 0x06ED, 0x041C, 0x06EE, 0x041D, 0x06EF, 0x041E, 0x06F0, 0x041F, 0x06F1, 0x042F,
    0x06F2, 0x0420, 0x06F3, 0x0421, 0x06F4, 0x0422, 0x06F5, 0x0423, 0x06F6, 0x0416, 0x06F7, 0x0412,
    0x06F8, 0x042C, 0x06F9, 0x042B, 0x06FA, 0x0417, 0x06FB, 0x0428, 0x06FC, 0x042D, 0x06FD, 0x0429,
    0x06FE, 0x0427, 0x06FF, 0x042A, 0x07A1, 0x0386, 0x07A2, 0x0388, 0x07A3, 0x0389, 0x07A4, 0x038A,
    0x07A5, 0x03AA, 0x07A7, 0x038C, 0x07A8, 0x038E, 0x07A9, 0x03AB, 0x07AB, 0x038F, 0x07AE, 0x0385,
    0x07AF, 0x2015, 0x07B1, 0x03AC, 0x07B2, 0x03AD, 0x07B3, 0x03AE, 0x07B4, 0x03AF, 0x07B5, 0x03CA,
    0x07B6, 0x0390, 0x07B7, 0x03CC, 0x07B8, 0x03CD, 0x07B9, 0x03CB, 0x07BA, 0x03B0, 0x07BB, 0x03CE,
    0x07C1, 0x0391, 0x07C2, 0x0392, 0x07C3, 0x0393, 0x07C4, 0x0394, 0x07C5, 0x0395, 0x07C6, 0x0396,
    0x07C7, 0x0397, 0x07C8, 0x0398, 0x07C9, 0x0399, 0x07CA, 0x039A, 0x07CB, 0x039B, 0x07CC, 0x039C,
    0x07CD, 0x039D, 0x07CE, 0x039E, 0x07CF, 0x039F, 0x07D0, 0x03A0, 0x07D1, 0x03A1, 0x07D2, 0x03A3,
    0x07D4, 0x03A4, 0x07D5, 0x03A5, 0x07D6, 0x03A6, 0x07D7, 0x03A7, 0x07D8, 0x03A8, 0x07D9, 0x03A9,
    0x07E1, 0x03B1, 0x07E2, 0x03B2, 0x07E3, 0x03B3, 0x07E4, 0x03B4, 0x07E5, 0x03B5, 0x07E6, 0x03B6,
    0x07E7, 0x03B7, 0x07E8, 0x03B8, 0x07E9, 0x03B9, 0x07EA, 0x03BA, 0x07EB, 0x03BB, 0x07EC, 0x03BC,
    0x07ED, 0x03BD, 0x07EE, 0x03BE, 0x07EF, 0x03BF, 0x07F0, 0x03C0, 0x07F1, 0x03C1, 0x07F2, 0x03C3,
    0x07F3, 0x03C2, 0x07F4, 0x03C4, 0x07F5, 0x03C5, 0x07F6, 0x03C6, 0x07F7, 0x03C7, 0x07F8, 0x03C8,
    0x07F9, 0x03C9, 0x08BC, 0x2264, 0x08BD, 0x2260, 0x08BE, 0x2265, 0x08BF, 0x222B, 0x08C0, 0x2234,
    0x08C1, 0x221D, 0x08C2, 0x221E, 0x08C5, 0x2207, 0x08C8, 0x223C, 0x08C9, 0x2243, 0x08CD, 0x21D4,
    0x08CE, 0x21D2, 0x08CF, 0x2261, 0x08D6, 0x221A, 0x08DA, 0x2282, 0x08DB, 0x2283, 0x08DC, 0x2229,
    0x08DD, 0x222A, 0x08DE, 0x2227, 0x08DF, 0x2228, 0x08EF, 0x2202, 0x08F6, 0x0192, 0x08FB, 0x2190,
    0x08FC, 0x2191, 0x08FD, 0x2192, 0x08FE, 0x2193, 0x0AA1, 0x2003, 0x0AA2, 0x2002, 0x0AA3, 0x2004,
    0x0AA4, 0x2005, 0x0AA5, 0x2007, 0x0AA6, 0x2008, 0x0AA7, 0x2009, 0x0AA8, 0x200A, 0x0AA9, 0x2014,
    0x0AAA, 0x2013, 0x0AAE, 0x2026, 0x0AAF, 0x2025, 0x0AB0, 0x2153, 0x0AB1, 0x2154, 0x0AB2, 0x2155,
    0x0AB3, 0x2156, 0x0AB4, 0x2157, 0x0AB5, 0x2158, 0x0AB6, 0x2159, 0x0AB7, 0x215A, 0x0AB8, 0x2105,
    0x0ABB, 0x2012, 0x0AC3, 0x215B, 0x0AC4, 0x215C, 0x0AC5, 0x215D, 0x0AC6, 0x215E, 0x0AC9, 0x2122,
    0x0AD0, 0x2018, 0x0AD1, 0x2019, 0x0AD2, 0x201C, 0x0AD3, 0x201D, 0x0AD4, 0x211E, 0x0AD5, 0x2030,
    0x0AD6, 0x2032, 0x0AD7, 0x2033, 0x0AD9, 0x271D, 0x0AE6, 0x2022, 0x0AEC, 0x2663, 0x0AED, 0x2666,
    0x0AEE, 0x2665, 0x0AF0, 0x2720, 0x0AF1, 0x2020, 0x0AF2, 0x2021, 0x0AF3, 0x2713, 0x0AF4, 0x2717,
    0x0AF5, 0x266F, 0x0AF6, 0x266D, 0x0AF7, 0x2642, 0x0AF8, 0x2640, 0x0AF9, 0x260E, 0x0AFA, 0x2315,
    0x0AFB, 0x2117, 0x0AFC, 0x2038, 0x0AFD, 0x201A, 0x0AFE, 0x201E, 0x13BC, 0x0152, 0x13BD, 0x0153,
    0x13BE, 0x0178,
];

/// x11_ks_value(keysym) -> xkb.s symbol value: codepoint | KV_KP | KV_DEAD | modifier id << 24
/// (0 = no text, e.g. function keys)
pub(super) fn ks_value(ks: u32) -> u32 {
    if ks == 0 {
        return 0;
    }
    if ks.wrapping_sub(0x20) <= 0x7E - 0x20 || ks.wrapping_sub(0xA0) <= 0xFF - 0xA0 {
        return ks; // ASCII, Latin-1
    }
    if ks.wrapping_sub(0x1000100) <= 0x110FFFF - 0x1000100 {
        return ks - 0x1000000; // Unicode keysym
    }
    if ks.wrapping_sub(0x20A0) <= 0x0C {
        return ks; // currency block (EuroSign 0x20AC)
    }
    if ks < 0xFE00 {
        for p in KS_TEXT.chunks_exact(2) {
            if p[0] as u32 == ks {
                return p[1] as u32;
            }
        }
        return 0;
    }
    if ks > 0xFFFF {
        return 0;
    }
    for &(k, v) in KS_SPECIAL.iter() {
        if k as u32 == ks {
            return v;
        }
    }
    0
}

/// case_upper(value) -> (upper-case value, lower-case value); upper 0: no letter
fn case_upper(v: u32) -> (u32, u32) {
    if v & 0x1FFFFF != v {
        return (0, 0); // flags: not a letter
    }
    if v.wrapping_sub(b'a' as u32) <= 25 {
        return (v - 32, v);
    }
    if v.wrapping_sub(b'A' as u32) <= 25 {
        return (v, v + 32);
    }
    if v.wrapping_sub(0xE0) <= 0x1E {
        if v == 0xF7 {
            return (0, 0);
        }
        return (v - 32, v);
    }
    if v.wrapping_sub(0xC0) <= 0x1E && v != 0xD7 {
        return (v, v + 32);
    }
    (0, 0)
}

impl X11 {
    /// x11_push(type | btn << 8, x, y, data): queue an event with the current modifiers and
    /// the window of the event being handled (evwin)
    pub(super) fn push(&mut self, typ: u8, btn: u8, x: i32, y: i32, data: u32) {
        crate::evq_push(Event {
            typ,
            btn,
            mods: self.mods as u8,
            win: (self.evwin & (PW_MAX as i32 - 1)) as u8,
            x,
            y,
            data,
        });
    }

    // ------------------------------------------------------------------ keyboard setup

    /// x11_input_init: XKB extension (if present) and the keyboard mapping
    pub(super) fn input_init(&mut self) {
        let op = self.xkb_op;
        if op != 0 {
            let (o, seq) = self.req(op, 8); // UseExtension 1.0
            wr16(&mut self.out, o, 1);
            wr16(&mut self.out, o + 2, 0);
            if let Some(r) = self.reply(seq) {
                if r[1] != 0 {
                    self.xkb_on = true;
                    // PerClientFlags: detectable auto-repeat
                    let (o, seq) = self.req(op | XKB_PER_CLIENT_FLAGS << 8, 28);
                    wr16(&mut self.out, o, XKB_CORE_KBD);
                    wr32(&mut self.out, o + 4, 1); // change
                    wr32(&mut self.out, o + 8, 1); // value
                    if let Some(r) = self.reply(seq) {
                        self.detect_rep = rd32(&r, 12) & 1 != 0;
                    }
                    // SelectEvents: NewKeyboardNotify + MapNotify -> reload the mapping
                    let (o, _) = self.req(op | XKB_SELECT_EVENTS << 8, 16);
                    let b = &mut self.out;
                    wr16(b, o, XKB_CORE_KBD);
                    wr16(b, o + 2, 3); // affectWhich
                    wr16(b, o + 4, 0); // clear
                    wr16(b, o + 6, 3); // selectAll
                    wr16(b, o + 8, 0xFF); // affectMap
                    wr16(b, o + 10, 0xFF); // map
                }
            }
        }
        self.kb_load();
        if self.debug {
            eprint(&alloc::format!("x11: XKB {}, detectable auto-repeat {}\n", self.xkb_on as u32, self.detect_rep as u32));
        }
    }

    /// x11_kb_load: fetch the core keyboard + modifier mapping and build the tables
    pub(super) fn kb_load(&mut self) {
        let first = self.min_kc;
        let count = self.max_kc.wrapping_sub(first).wrapping_add(1);
        let (o, seq) = self.req(XR_GET_KEYBOARD_MAPPING, 8);
        self.out[o] = first as u8;
        self.out[o + 1] = count as u8;
        if let Some(r) = self.reply(seq) {
            let per = r[1] as u32; // keysyms per keycode
            let ks: alloc::vec::Vec<u32> = r[32..].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            self.kb_set_keysyms(&ks, first, count, per);
        }
        let (_, seq) = self.req(XR_GET_MODIFIER_MAPPING, 4);
        if let Some(r) = self.reply(seq) {
            let per = r[1] as u32; // keycodes per modifier
            self.kb_set_modmap(&r[32..], per);
        }
        let g = self.group;
        self.kb_build(g);
        if crate::env("jfgui_XKBDUMP").is_some() {
            crate::xkb_dump();
        }
    }

    /// x11_kb_set_keysyms(keysyms, first, count, per): keep up to 8 columns per keycode
    pub(super) fn kb_set_keysyms(&mut self, ks: &[u32], first: u32, count: u32, per: u32) {
        self.ks = [[0; KS_COLS]; 256];
        let cols = (per as usize).min(KS_COLS);
        let mut kc = first;
        let mut src = 0usize;
        for _ in 0..count {
            if kc > 255 {
                break;
            }
            for c in 0..cols {
                self.ks[kc as usize][c] = ks.get(src + c).copied().unwrap_or(0);
            }
            src += per as usize;
            kc += 1;
        }
    }

    /// x11_kb_set_modmap(keycodes, per): which ModN is Alt, NumLock, Super, AltGr -- every
    /// keysym of every key on Mod1..Mod5 counts (like Xlib)
    pub(super) fn kb_set_modmap(&mut self, kc: &[u8], per: u32) {
        self.role_alt = 0;
        self.role_num = 0;
        self.role_super = 0;
        self.role_l3 = 0;
        let per = per as usize;
        for m in 3..8usize {
            // Mod1..Mod5
            for e in 0..per {
                let k = kc.get(m * per + e).copied().unwrap_or(0) as usize;
                if k == 0 {
                    continue;
                }
                for c in 0..KS_COLS {
                    let id = (ks_value(self.ks[k][c]) >> 24) & 15;
                    let bit = 1u32 << m;
                    if id == MSV_ALT >> 24 || id == MSV_META >> 24 {
                        self.role_alt |= bit;
                    } else if id == MSV_SUPER >> 24 || id == MSV_HYPER >> 24 {
                        self.role_super |= bit;
                    } else if id == MSV_L3 >> 24 {
                        self.role_l3 |= bit;
                    } else if id == MSV_NUM >> 24 {
                        self.role_num |= bit;
                    }
                }
            }
        }
    }

    /// x11_kb_build(group): key records for group 0 (columns 0 1 4 5) or group 1 (columns 2 3
    /// 6 7; an empty group 2 falls back to group 1)
    pub(super) fn kb_build(&mut self, group: u32) {
        self.group = group;
        crate::xkb_begin();
        for kc in 8..=255usize {
            let cols = self.ks[kc];
            let base = if group != 0 && (cols[2] | cols[3]) != 0 { 2 } else { 0 };
            let (l1, l2, l3, l4) = (cols[base], cols[base + 1], cols[base + 4], cols[base + 5]);
            if l1 | l2 | l3 | l4 == 0 {
                continue;
            }
            let mut v = [ks_value(l1), ks_value(l2), ks_value(l3), ks_value(l4)];
            if v[3] == 0 {
                v[3] = v[2]; // L4 NoSymbol: same as L3
            }
            // L2 NoSymbol: the upper case of a letter, else L1 (core protocol rule)
            if l2 == 0 {
                v[1] = v[0];
                let (u, l) = case_upper(v[0]);
                if u != 0 {
                    v[1] = u;
                    v[0] = l;
                }
            }
            let nlev = if v[2] | v[3] != 0 {
                4
            } else if v[0] != v[1] || l2 != 0 {
                2
            } else {
                1
            };
            crate::xkb_set_key(kc as u32 - 8, &v, nlev);
        }
        let (a, n, s, l) = (self.role_alt, self.role_num, self.role_super, self.role_l3);
        crate::xkb_set_roles(a, n, s, l);
        self.kb_ready = true;
    }

    /// x11_state_mods(state): MOD_* bits from an event's state field (+ group switch)
    fn state_mods(&mut self, state: u32) {
        if self.xkb_on {
            let mut g = (state >> 13) & 3;
            if g > 1 {
                g = 0; // groups 3 / 4: not in 8 columns
            }
            if g != self.group {
                self.kb_build(g);
            }
        }
        self.mods = crate::xkb_set_mods(state & 0xFF, 0, 0, 0);
    }

    // ------------------------------------------------------------------ keyboard events

    /// x11_key(ev, pressed): KeyPress / KeyRelease
    pub(super) fn key(&mut self, b: &[u8], pressed: bool) {
        let t = rd32(b, XEV_TIME);
        self.time = t;
        if !self.kb_ready {
            return;
        }
        self.state_mods(rd16(b, XEV_STATE) as u32);
        let kc = b[XEV_DETAIL] as u32;
        if kc < 8 {
            return;
        }
        let ev = kc - 8;
        // like Win32, a modifier's own event already carries its new state
        let bit = crate::xkb_mod_bit(ev);
        if pressed {
            self.mods |= bit;
        } else {
            self.mods &= !bit;
        }
        let (vk, text) = crate::xkb_key(ev);
        if pressed && vk == 0x1B && self.dnd_escape() {
            return; // Esc ends the app's drag (linux_x11_dnd.rs)
        }
        let (w, m) = ((kc >> 5) as usize, 1u32 << (kc & 31));
        // an input method (IBus) sees the key first while a text field has the focus
        let ime = self.ime_on && crate::ibus_ready();
        let keyval = if text >= 0x20 && text < 0x100 {
            text
        } else if text > 0xFF {
            0x0100_0000 | text
        } else {
            self.ks[kc as usize][0]
        };
        let xstate = rd16(b, XEV_STATE) as u32;
        if pressed {
            let mut rep = 0u32;
            if self.held[w] & m != 0 {
                rep = 0x80000000; // held already: auto-repeat
            }
            self.held[w] |= m;
            if kc + 1 == self.rep_merge {
                self.rep_merge = 0;
                rep = 0x80000000;
            }
            if ime && keyval != 0 && crate::ibus_key(keyval, ev, xstate) {
                return; // its text comes as E_IME_COMMIT / E_PREEDIT
            }
            if vk != 0 {
                self.push(E_KEY_DOWN, 0, 0, 0, vk | rep);
            }
            if text != 0 {
                self.push(E_TEXT, 0, 0, 0, text);
            }
            return;
        }
        if !self.detect_rep {
            // server auto-repeat without XKB: release + press with the same time
            let p = self.in_pos;
            if self.in_len - p >= 32 {
                let n = &self.inb[p..p + 32];
                if n[0] & 0x7F == XE_KEY_PRESS && n[XEV_DETAIL] as u32 == kc && rd32(n, XEV_TIME) == t {
                    self.rep_merge = kc + 1;
                    return; // the key stays down
                }
            }
        }
        self.held[w] &= !m;
        if ime && keyval != 0 {
            crate::ibus_key(keyval, ev, xstate | crate::IBUS_RELEASE);
        }
        if vk != 0 {
            self.push(E_KEY_UP, 0, 0, 0, vk);
        }
    }

    /// ime_pos(x, y, h): the caret of the focused text field in window px (h 0: none) ->
    /// IBus focus / cursor location on the root
    pub(super) fn ime_pos(&mut self, x: i32, y: i32, h: i32) {
        // only while a window of ours has the keyboard (the UI calls this on every frame)
        let win = (0..PW_MAX).find(|&k| self.xw[k].win != 0 && self.xw[k].focus);
        match win {
            Some(i) if h > 0 => {
                let (rx, ry) = if self.xw[i].posok { (self.xw[i].rx, self.xw[i].ry) } else { (0, 0) };
                let rect = [rx + x, ry + y, h];
                if !self.ime_on || self.ime_rect != rect || self.ime_win != i as u8 {
                    if self.ime_on && self.ime_win != i as u8 {
                        crate::ibus_focus(self.ime_win, false, 0, 0, 0);
                    }
                    self.ime_on = true;
                    self.ime_rect = rect;
                    self.ime_win = i as u8;
                    crate::ibus_focus(i as u8, true, rect[0], rect[1], h);
                }
            }
            _ => self.ime_off(),
        }
    }

    // the text field or its window lost the keyboard
    fn ime_off(&mut self) {
        if self.ime_on {
            self.ime_on = false;
            crate::ibus_focus(self.ime_win, false, 0, 0, 0);
        }
    }

    /// x11_focus(ev, in): FocusIn / FocusOut of one of our windows (evwin)
    pub(super) fn focus(&mut self, b: &[u8], fin: bool) {
        let mode = b[8];
        let detail = b[1];
        let i = (self.evwin as u32 as usize) & (PW_MAX - 1);
        if mode == 1 || mode == 2 {
            return; // NotifyGrab / NotifyUngrab: keyboard grabs of the window manager
        }
        if detail > 4 {
            return; // Pointer, PointerRoot, None
        }
        if fin {
            if self.xw[i].focus {
                return;
            }
            self.xw[i].focus = true;
            self.xw[i].state &= !WST_MINIMIZED;
            let w = self.evwin;
            self.hide_owned(w, false); // windows hidden while it was minimized
            self.push(E_FOCUS_IN, 0, 0, 0, 0);
        } else {
            if detail == 2 || !self.xw[i].focus {
                return; // NotifyInferior
            }
            self.xw[i].focus = false;
            self.held = [0; 8];
            self.rep_merge = 0;
            if self.ime_win as usize == i {
                self.ime_off(); // back with the next frame after E_FOCUS_IN
            }
            self.push(E_FOCUS_OUT, 0, 0, 0, 0);
        }
    }

    // ------------------------------------------------------------------ pointer events

    /// x11_button(ev, pressed): ButtonPress / ButtonRelease
    pub(super) fn button(&mut self, b: &[u8], pressed: bool) {
        let t = rd32(b, XEV_TIME);
        self.time = t;
        self.state_mods(rd16(b, XEV_STATE) as u32);
        let (x, y) = (rds16(b, XEV_X), rds16(b, XEV_Y));
        self.mx = x;
        self.my = y;
        let d = b[XEV_DETAIL] as u32;
        if d.wrapping_sub(4) <= 3 {
            // wheel: 4 up, 5 down, 6 left, 7 right (press only)
            if pressed {
                self.push(E_WHEEL, 0, x, y, WHEEL_DATA[(d - 4) as usize]);
            }
            return;
        }
        if d > 9 {
            return;
        }
        let bt = BUTTON_MAP[d as usize];
        if bt == 0 {
            return;
        }
        let typ;
        if pressed {
            self.press_rx = rds16(b, XEV_ROOT_X);
            self.press_ry = rds16(b, XEV_ROOT_Y);
            self.press_btn = d;
            self.btn_win = self.evwin;
            self.btn |= 1 << bt;
            typ = E_DOWN;
        } else {
            if self.btn & 1 << bt == 0 {
                return; // not pressed inside our window
            }
            self.btn &= !(1 << bt);
            typ = E_UP;
            if bt == crate::BTN_LEFT {
                self.dnd_release(); // the app's drag drops (linux_x11_dnd.rs)
            }
        }
        let data = self.xi_data(t); // (PN1) pen data of the same device event
        self.push(typ, bt, x, y, data);
        // released outside after a drag: the leave was held back until now
        if !pressed && self.btn == 0 && self.leave_pending != 0 {
            let w = self.leave_pending - 1;
            self.leave_pending = 0;
            self.evwin = w;
            self.push(E_LEAVE, 0, 0, 0, 0);
        }
    }

    /// x11_motion(ev): MotionNotify; consecutive motions are merged while no button is held
    /// (painting needs every point) or while the event queue is half full
    pub(super) fn motion(&mut self, b: &[u8]) {
        let t = rd32(b, XEV_TIME);
        self.time = t;
        self.state_mods(rd16(b, XEV_STATE) as u32);
        let (x, y) = (rds16(b, XEV_X), rds16(b, XEV_Y));
        self.mx = x;
        self.my = y;
        self.dnd_motion(rds16(b, XEV_ROOT_X), rds16(b, XEV_ROOT_Y)); // the app's drag follows
        if self.btn == 0 || crate::evq_count() >= EV_COMPRESS {
            let p = self.in_pos;
            if self.in_len - p >= 32 && self.inb[p] & 0x7F == XE_MOTION {
                return; // a newer position follows
            }
        }
        let data = self.xi_data(t);
        self.push(E_MOVE, 0, x, y, data);
    }

    /// x11_enter(ev): EnterNotify -- the pointer is over our window
    pub(super) fn enter(&mut self, b: &[u8]) {
        self.time = rd32(b, XEV_TIME);
        self.state_mods(rd16(b, XEV_STATE) as u32);
        self.inside |= 1u32.wrapping_shl(self.evwin as u32);
        self.leave_pending = 0;
        let (x, y) = (rds16(b, XEV_X), rds16(b, XEV_Y));
        self.mx = x;
        self.my = y;
        self.push(E_MOVE, 0, x, y, 0);
    }

    /// x11_leave(ev): LeaveNotify.  With a button held (implicit grab) the events keep
    /// coming: E_LEAVE follows the last release.  A grab by someone else (the window manager
    /// moving the window) ends our pointer input: release the buttons.
    pub(super) fn leave(&mut self, b: &[u8]) {
        self.time = rd32(b, XEV_TIME);
        let mode = b[XEV_MODE];
        let w = self.evwin;
        let bit = 1u32.wrapping_shl(w as u32);
        if mode == 2 && self.inside & bit == 0 {
            return; // NotifyUngrab while outside
        }
        self.inside &= !bit;
        if mode == 0 && self.btn != 0 {
            self.leave_pending = w + 1;
            return;
        }
        self.leave_pending = 0;
        self.release_buttons();
        self.push(E_LEAVE, 0, 0, 0, 0);
    }

    /// x11_release_buttons: E_UP for every held button (at the last position, in the window
    /// of the press)
    pub(super) fn release_buttons(&mut self) {
        let ev = self.evwin;
        self.evwin = self.btn_win;
        for bt in 1..=5u8 {
            if self.btn & 1 << bt != 0 {
                self.btn &= !(1 << bt);
                let (x, y) = (self.mx, self.my);
                self.push(E_UP, bt, x, y, 0);
            }
        }
        self.evwin = ev;
    }
}
