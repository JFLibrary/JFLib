// palette -- the BackupCraft colour tokens (T_* order of lib/ui/consts.rs)
//
// Light: cool slate greys, white cards on a #F3F5F8 window, #C9D0D9 control frames.
// Dark: deep blue-grey (#12161C window, #1C222A cards).  The accent entries are 0 here and
// filled by accent() (lib.rs).  Status colours: success green, warning amber, critical red,
// info blue, each with a tinted background for badges and info bars.

use ui::consts::T_COUNT;

pub(crate) const LIGHT: [u32; T_COUNT] = [
    0xFFF3F5F8, 0xFFECEFF3, 0xFFF8F9FB, 0xFFE1E5EB, // bg, bg alt, layer, layer stroke
    0xFFFFFFFF, 0xFFE1E5EB, // card, card stroke
    0xFFFFFFFF, 0xFFF6F8FA, 0xFFEDF0F3, 0xFFF3F5F7, // control rest / hover / pressed / disabled
    0xFFC9D0D9, 0xFFB4BDC8, 0xFF8892A0, // control stroke, bottom stroke, strong stroke
    0x00000000, 0x1018202B, 0x1C18202B, 0x00000000, // subtle rest / hover / pressed / disabled
    0xFF18202B, 0xFF4E5968, 0xFF768190, 0xFFA3ACB7, // text primary / secondary / tertiary / disabled
    0, 0, 0, 0xFFB8D2D7, // accent (accent()), accent disabled
    0, 0xFFF1F4F6, 0, // on accent (accent()), on accent disabled, accent text (accent())
    0, 0xFFFFFFFF, // focus outer (accent()), inner
    0xFFFFFFFF, 0x2418202B, 0x4D0F141A, 0xFFE4E8ED, // flyout, flyout stroke, smoke, divider
    0xFFFFFFFF, // input active
    0xFFD9383C, 0xFF2B9348, 0xFFE07B00, 0xFF1C7ED6, // critical, success, warning, info
    0xFFFDECEC, 0xFFE6F5EA, 0xFFFFF1DE, 0xFFE6F0FB, // their backgrounds
    0xFFD9383C, 0x22000000, 0x5C18202B, 0xFFE3E7EC, // close hover, shadow, scroll thumb, track
    0xFFFFFFFF, 0xFFFFFFFF, 0xFF8892A0, // knob, alt fill, strong fill
];

pub(crate) const DARK: [u32; T_COUNT] = [
    0xFF12161C, 0xFF0E1116, 0xFF171C23, 0xFF262D37,
    0xFF1C222A, 0xFF2A323D,
    0xFF232A33, 0xFF29313B, 0xFF1F252D, 0xFF1D2229,
    0xFF36404C, 0xFF3F4A57, 0xFF6B7684,
    0x00000000, 0x14E7EBF0, 0x0CE7EBF0, 0x00000000,
    0xFFE7EBF0, 0xFFAAB4C0, 0xFF808B98, 0xFF5B6572,
    0, 0, 0, 0xFF24464C,
    0, 0xFF6F7A87, 0,
    0, 0xFF000000,
    0xFF1F252E, 0xFF333C48, 0x99000000, 0xFF2A323D,
    0xFF1A2027,
    0xFFFF6B6B, 0xFF51CF66, 0xFFFFA94D, 0xFF4DABF7,
    0xFF3A1F22, 0xFF19331F, 0xFF3A2A17, 0xFF172B40,
    0xFFD9383C, 0x66000000, 0x80E7EBF0, 0xFF2C343F,
    0xFFE7EBF0, 0xFF1A2027, 0xFF808B98,
];
