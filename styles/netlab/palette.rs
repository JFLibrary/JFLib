// palette -- the NetLab colour tokens (T_* order of lib/ui/consts.rs)
//
// Light: a #ECEEF1 window, #F6F7F8 panels, white fields and cards, #B8BEC6 frames, ink
// #1D2228 / #4D5560.  Dark: #1B1E22 window, #23272C panels, #2B3036 fields, #3D444D frames,
// #E3E6EA text.  The accent entries are 0 here and filled by accent() (lib.rs).  Status
// colours are desaturated (success green, warning ochre, critical red, info blue), each with
// a quiet tint for badges and info bars.

use ui::consts::T_COUNT;

pub(crate) const LIGHT: [u32; T_COUNT] = [
    0xFFECEEF1, 0xFFE2E5E9, 0xFFF6F7F8, 0xFFB8BEC6, // bg, bg alt, layer (panels), layer stroke
    0xFFFFFFFF, 0xFFB8BEC6, // card, card stroke
    0xFFF8F9FA, 0xFFFFFFFF, 0xFFE3E6EA, 0xFFEEF0F2, // control rest / hover / pressed / disabled
    0xFFB8BEC6, 0xFFA9B0B9, 0xFF7D8590, // control stroke, bottom stroke, strong stroke
    0x00000000, 0x141D2228, 0x241D2228, 0x00000000, // subtle rest / hover / pressed / disabled
    0xFF1D2228, 0xFF4D5560, 0xFF6E7681, 0xFFA0A7B0, // text primary / secondary / tertiary / disabled
    0, 0, 0, 0xFFB4C6DC, // accent (accent()), accent disabled
    0, 0xFFF4F6F8, 0, // on accent (accent()), on accent disabled, accent text (accent())
    0, 0xFFFFFFFF, // focus outer (accent()), inner
    0xFFFFFFFF, 0xFFA9B0B9, 0x4D1D2228, 0xFFD3D7DC, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFC62828, 0xFF2E7D32, 0xFFA86400, 0xFF1F6FB5, // critical, success, warning, info
    0xFFF9E8E8, 0xFFE8F2E8, 0xFFF7EFE0, 0xFFE6EFF8, // their backgrounds
    0xFFC42B1C, 0x24000000, 0xFFA3AAB3, 0xFFDDE1E6, // close hover, shadow, scroll thumb, track
    0xFFFFFFFF, 0xFFFFFFFF, 0xFF7D8590, // knob, alt fill, strong fill
];

pub(crate) const DARK: [u32; T_COUNT] = [
    0xFF1B1E22, 0xFF16181B, 0xFF23272C, 0xFF3D444D,
    0xFF2B3036, 0xFF3D444D,
    0xFF2E3339, 0xFF353B42, 0xFF272B30, 0xFF25292E,
    0xFF3D444D, 0xFF3D444D, 0xFF7A838E,
    0x00000000, 0x16E3E6EA, 0x24E3E6EA, 0x00000000,
    0xFFE3E6EA, 0xFFAAB1BA, 0xFF858D97, 0xFF5B636D,
    0, 0, 0, 0xFF2F4256,
    0, 0xFF6B737D, 0,
    0, 0xFF000000,
    0xFF2B3036, 0xFF4A525C, 0x99000000, 0xFF33393F,
    0xFF2B3036,
    0xFFE06A6A, 0xFF66B36A, 0xFFD69A45, 0xFF64A0DA,
    0xFF3A2324, 0xFF1F3122, 0xFF352B1B, 0xFF1E2C3C,
    0xFFC42B1C, 0x66000000, 0xFF6A727C, 0xFF30353B,
    0xFFE3E6EA, 0xFF2B3036, 0xFFA0A8B2,
];
