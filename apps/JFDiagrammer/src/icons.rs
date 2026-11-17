// icons -- JFDiagrammer's ribbon icons: own artwork in the NetLab look (icons/NAME_16.svg and
// NAME_32.svg -> mkicons.py -> icons_data.rs), drawn through ui::icon_hook at private code
// points from 0x10F200 (the JD_* constants; JD_*_L is the 32 px drawing of the same icon)
//
//   Icons::install(ui) -> Icons    every icon parsed once, the hook set
//   icons.update(ui)               the theme's colours for the hook (every frame)
//
// The art is filled layers in five roles (mkicons.py): ink = the colour the style draws the icon
// in (text, disabled, the checked colour), body / shade = the ink at 12 / 26 %, tint = the
// theme's accent, hi = the accent at 24 %.  A disabled icon draws its accent in the ink; high
// contrast draws everything in the ink without surfaces.
//
// Sizes: the art fills the em box the style asks for: the small art (16 grid) the 16 DIP box of
// small buttons, the large art (32 grid) the 32 DIP box of NetLab's large ribbon buttons (JD8).
// At 100 % and 200 % every line falls on whole pixels.

// the whole drawn set; the ribbon takes the ones it has buttons for so far
#[allow(dead_code)]
#[path = "icons_data.rs"]
mod icons_data;

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;

use ui::{Gfx, SymPalette, Symbol, Ui, SYM_BODY, SYM_HI, SYM_ROLES, SYM_SHADE, SYM_TINT, T_ACCENT, T_TEXT_DISABLED};

pub(crate) use icons_data::*;

const BODY: u32 = 31; // 12 % of the ink
const SHADE: u32 = 66; // 26 %
const HI: u32 = 61; // 24 % of the accent

#[derive(Clone, Copy, Default)]
struct Theme {
    accent: u32,
    disabled: u32,
    contrast: bool,
}

/// the hook's view of the theme
pub(crate) struct Icons {
    theme: Rc<Cell<Theme>>,
}

impl Icons {
    /// parse every icon and draw the JD_* code points from now on
    pub(crate) fn install(ui: &mut Ui) -> Icons {
        let syms: Vec<(u32, f32, Symbol)> =
            ARTS.iter().map(|&(cp, grid, art)| (cp, grid, Symbol::parse(art, [grid, grid]))).collect();
        let theme = Rc::new(Cell::new(Theme::default()));
        let seen = theme.clone();
        ui.icon_hook(Some(Rc::new(move |g: &mut Gfx, cp: u32, x: i32, y: i32, size: i32, argb: u32| {
            let Ok(i) = syms.binary_search_by_key(&cp, |s| s.0) else { return false };
            let (_, grid, sym) = &syms[i];
            let em = size as f32 / 256.0; // the em box in px: the art fills it
            // x, y are target pixels; paint wants the draw context's coordinates
            let (px, py) = ((x - g.dc.ox) as f32, (y - g.dc.oy) as f32);
            sym.paint(g, px, py, em / grid, &palette(argb, seen.get()));
            true
        })));
        let icons = Icons { theme };
        icons.update(ui);
        icons
    }

    /// the accent and the disabled colour of the current theme
    pub(crate) fn update(&self, ui: &Ui) {
        self.theme.set(Theme { accent: ui.color(T_ACCENT), disabled: ui.color(T_TEXT_DISABLED), contrast: ui.high_contrast() });
    }
}

// the colour scaled to a share of its alpha
fn share(c: u32, a: u32) -> u32 {
    (c & 0xFF_FFFF) | (((c >> 24) * a / 255) << 24)
}

// the roles for an icon drawn in `ink`
fn palette(ink: u32, t: Theme) -> SymPalette {
    let accent = if t.contrast || ink == t.disabled || t.accent == 0 { ink } else { t.accent };
    let mut c = [ink; SYM_ROLES];
    let surface = |col: u32, a: u32| if t.contrast { 0 } else { share(col, a) };
    c[SYM_BODY as usize] = surface(ink, BODY);
    c[SYM_SHADE as usize] = surface(ink, SHADE);
    c[SYM_TINT as usize] = accent;
    c[SYM_HI as usize] = surface(accent, HI);
    SymPalette { c }
}
