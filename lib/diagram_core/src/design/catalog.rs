// catalog -- JFDiagrammer's own designs (V3, TH-03): NetLab first
//
// All values are JFLib's own, from the NetLab style (styles/netlab: palette.rs, lib.rs): ink
// anthracite #1D2228 and slate #4D5560, window grey #ECEEF1, the accent Signalblau #1F5FA8 and
// the chart series ochre #C9792B, teal #3D8C84, brick #B0453F, violet #7A5C99, olive #7F8C3A;
// the typeface Inter.  Lines are thin and square-ended, effects rare.
//
//   NetLab    the house look: light tinted fills, 1 pt lines in the colour, anthracite text,
//             slate connectors with a slim filled arrow.  Variants: Signalblau, Petrol, Signalblau
//             solid (white text), Graphit.
//   Contour   black on white, for print and technical drawings: white fills, 1.25 pt outlines,
//             open arrows, no effects.  Variants: Anthracite, Signalblau, Brick, Rounded.
//   Contrast  bold surfaces: solid fills without outline, white bold text, strong dark
//             connectors with a wide arrow.  Variants: Signalblau, Petrol, Brick, Anthracite rounded.
//
// Conventions every own design keeps, so the style sheets of a new document fit all of them:
// font style 1 is dark text for light ground (the style "Text Only" uses it), connector line
// style 1 is the connector with its arrow, and the variants' quick style 4 is the quiet one
// for containers and backgrounds.

use alloc::string::String;
use alloc::vec::Vec;

use super::{Clr, Design, Effect, Fill, FontStyle, Line, Mod, Scheme, Shadow, Slot, VarStyle, Variant, EMU_PT, FIRST_ID};

/// the typeface of the NetLab style
pub const FONT: &str = "Inter";

// NetLab colours
const ANTHRACITE: u32 = 0x1D2228;
const SLATE: u32 = 0x4D5560;
const GREY: u32 = 0x6E7681;
const SILVER: u32 = 0xA0A7B0;
const WINDOW: u32 = 0xECEEF1;
const PAPER: u32 = 0xF2F3F5;
const WHITE: u32 = 0xFFFFFF;
const SIGNALBLAU: u32 = 0x1F5FA8;
const OCHRE: u32 = 0xC9792B;
const TEAL: u32 = 0x3D8C84;
const BRICK: u32 = 0xB0453F;
const VIOLET: u32 = 0x7A5C99;
const OLIVE: u32 = 0x7F8C3A;

/// the scheme number of NetLab
pub const NETLAB: u32 = FIRST_ID;
/// the scheme number of Contour
pub const CONTOUR: u32 = FIRST_ID + 1;
/// the scheme number of Contrast
pub const CONTRAST: u32 = FIRST_ID + 2;

const MM: u32 = 36_000; // EMU per millimetre

fn ph() -> Clr {
    Clr::ph()
}

fn slot(s: Slot) -> Clr {
    Clr::slot(s)
}

fn none_effects(n: usize) -> Vec<Effect> {
    (0..n).map(|_| Effect::default()).collect()
}

fn shadow(blur_pt: u32, dist_pt: u32, alpha: u32) -> Effect {
    Effect { shadow: Some(Shadow { blur: blur_pt * EMU_PT, dist: dist_pt * EMU_PT, dir: 5_400_000, color: slot(Slot::Dk1).with(Mod::Alpha(alpha * 1000)) }) }
}

fn colors(accents: [u32; 6], dk2: u32, lt2: u32) -> [u32; 12] {
    let [a1, a2, a3, a4, a5, a6] = accents;
    [ANTHRACITE, WHITE, dk2, lt2, a1, a2, a3, a4, a5, a6, SIGNALBLAU, VIOLET]
}

fn backgrounds() -> Vec<Fill> {
    alloc::vec![Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt2)), Fill::solid(ph().lighter(90))]
}

fn variant(colors: [u32; 7], embellishment: u8, styles: [VarStyle; 4]) -> Variant {
    Variant { colors, monotone: false, embellishment, styles }
}

/// NetLab: the house look
pub fn netlab() -> Design {
    let shapes = Scheme {
        name: String::from("NetLab"),
        fills: alloc::vec![
            Fill::solid(slot(Slot::Lt1)),
            Fill::solid(ph().lighter(85)),
            Fill::solid(ph().lighter(60)),
            Fill::solid(ph()),
            Fill::solid(ph().darker(25)),
            Fill::solid(slot(Slot::Lt2)),
        ],
        lines: alloc::vec![
            Line::solid(75, ph()),
            Line::solid(100, ph()),
            Line::solid(150, ph()),
            Line::solid(100, ph().darker(30)),
            Line::solid(200, ph()),
            Line::solid(100, slot(Slot::Dk2)),
        ],
        effects: {
            let mut e = none_effects(4);
            e.push(shadow(3, 1, 24));
            e.push(shadow(6, 2, 30));
            e
        },
        backgrounds: backgrounds(),
        fonts: alloc::vec![
            FontStyle::regular(slot(Slot::Dk1)),
            FontStyle::bold(slot(Slot::Dk1)),
            FontStyle::regular(ph().darker(40)),
            FontStyle::regular(slot(Slot::Lt1)),
            FontStyle::bold(slot(Slot::Lt1)),
            FontStyle::regular(slot(Slot::Dk2)),
        ],
    };
    let connectors = Scheme {
        name: String::from("NetLab"),
        fills: alloc::vec![Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt2))],
        lines: alloc::vec![
            Line::solid(100, slot(Slot::Dk2)).arrow(13, 2),
            Line::solid(100, ph()).arrow(13, 2),
            Line::solid(150, slot(Slot::Dk1)).arrow(13, 2),
        ],
        effects: none_effects(3),
        backgrounds: backgrounds(),
        fonts: alloc::vec![FontStyle::regular(slot(Slot::Dk1)), FontStyle::regular(slot(Slot::Dk2)), FontStyle::regular(slot(Slot::Lt1))],
    };
    let light = [VarStyle::new(2, 2, 1, 1), VarStyle::new(3, 4, 1, 1), VarStyle::new(4, 4, 1, 5), VarStyle::new(6, 6, 1, 1)];
    let solid = [VarStyle::new(4, 4, 1, 4), VarStyle::new(5, 5, 1, 4), VarStyle::new(2, 2, 1, 1), VarStyle::new(6, 6, 1, 1)];
    let blue = [SIGNALBLAU, TEAL, OCHRE, BRICK, VIOLET, OLIVE, SLATE];
    Design {
        name: String::from("NetLab"),
        id: NETLAB,
        colors: colors([SIGNALBLAU, OCHRE, TEAL, BRICK, VIOLET, OLIVE], SLATE, WINDOW),
        background: WHITE,
        major_font: String::from(FONT),
        minor_font: String::from(FONT),
        shapes,
        connectors,
        variants: [
            variant(blue, 2, light),
            variant([TEAL, SIGNALBLAU, OLIVE, OCHRE, VIOLET, BRICK, SLATE], 2, light),
            variant(blue, 2, solid),
            variant([SLATE, SIGNALBLAU, TEAL, OCHRE, BRICK, VIOLET, GREY], 2, light),
        ],
    }
}

/// Contour: black on white, for print
pub fn contour() -> Design {
    let shapes = Scheme {
        name: String::from("Contour"),
        fills: alloc::vec![
            Fill::solid(slot(Slot::Lt1)),
            Fill::solid(slot(Slot::Lt2)),
            Fill::solid(ph().lighter(88)),
            Fill::solid(ph()),
            Fill::none(),
            Fill::solid(slot(Slot::Dk1)),
        ],
        lines: alloc::vec![
            Line::solid(75, ph()),
            Line::solid(125, ph()),
            Line::solid(200, ph()),
            Line::solid(125, slot(Slot::Dk1)),
            Line::solid(75, ph()).pattern(2),
            Line::solid(125, ph()).rounded(5 * MM / 2),
        ],
        effects: none_effects(6),
        backgrounds: backgrounds(),
        fonts: alloc::vec![
            FontStyle::regular(slot(Slot::Dk1)),
            FontStyle::bold(slot(Slot::Dk1)),
            FontStyle::regular(ph()),
            FontStyle::regular(slot(Slot::Lt1)),
            FontStyle::bold(ph()),
            FontStyle::regular(slot(Slot::Dk2)),
        ],
    };
    let connectors = Scheme {
        name: String::from("Contour"),
        fills: alloc::vec![Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt2))],
        lines: alloc::vec![
            Line::solid(100, slot(Slot::Dk1)).arrow(1, 2),
            Line::solid(100, ph()).arrow(1, 2),
            Line::solid(150, slot(Slot::Dk1)).arrow(1, 3),
        ],
        effects: none_effects(3),
        backgrounds: backgrounds(),
        fonts: alloc::vec![FontStyle::regular(slot(Slot::Dk1)), FontStyle::regular(slot(Slot::Dk2)), FontStyle::regular(slot(Slot::Lt1))],
    };
    let ink = [VarStyle::new(1, 2, 1, 1), VarStyle::new(3, 3, 1, 2), VarStyle::new(6, 4, 1, 4), VarStyle::new(2, 5, 1, 6)];
    let coloured = [VarStyle::new(1, 2, 1, 1), VarStyle::new(3, 3, 1, 5), VarStyle::new(4, 4, 1, 4), VarStyle::new(2, 5, 1, 6)];
    let rounded = [VarStyle::new(1, 6, 1, 1), VarStyle::new(3, 6, 1, 2), VarStyle::new(6, 6, 1, 4), VarStyle::new(2, 5, 1, 6)];
    let anthracite = [ANTHRACITE, SLATE, GREY, SIGNALBLAU, BRICK, TEAL, SILVER];
    Design {
        name: String::from("Contour"),
        id: CONTOUR,
        colors: colors([SIGNALBLAU, BRICK, TEAL, OCHRE, VIOLET, GREY], SLATE, PAPER),
        background: WHITE,
        major_font: String::from(FONT),
        minor_font: String::from(FONT),
        shapes,
        connectors,
        variants: [
            variant(anthracite, 1, ink),
            variant([SIGNALBLAU, ANTHRACITE, TEAL, BRICK, OCHRE, VIOLET, SILVER], 1, coloured),
            variant([BRICK, ANTHRACITE, OCHRE, SIGNALBLAU, TEAL, VIOLET, SILVER], 1, coloured),
            variant(anthracite, 1, rounded),
        ],
    }
}

/// Contrast: bold surfaces
pub fn contrast() -> Design {
    let shapes = Scheme {
        name: String::from("Contrast"),
        fills: alloc::vec![
            Fill::solid(ph()),
            Fill::solid(ph().darker(20)),
            Fill::solid(ph().lighter(80)),
            Fill::solid(slot(Slot::Lt1)),
            Fill::solid(slot(Slot::Dk1)),
            Fill::solid(slot(Slot::Lt2)),
        ],
        lines: alloc::vec![
            Line::solid(100, ph()).pattern(0),
            Line::solid(150, ph().darker(30)),
            Line::solid(250, ph()),
            Line::solid(100, slot(Slot::Lt1)),
            Line::solid(100, ph()).pattern(0).rounded(3 * MM),
            Line::solid(100, slot(Slot::Dk2)),
        ],
        effects: {
            let mut e = none_effects(1);
            e.push(shadow(4, 2, 28));
            e.extend(none_effects(4));
            e
        },
        backgrounds: backgrounds(),
        fonts: alloc::vec![
            FontStyle::regular(slot(Slot::Dk1)),
            FontStyle::bold(slot(Slot::Lt1)),
            FontStyle::regular(slot(Slot::Lt1)),
            FontStyle::bold(slot(Slot::Dk1)),
            FontStyle::bold(ph().darker(40)),
            FontStyle::regular(slot(Slot::Dk2)),
        ],
    };
    let connectors = Scheme {
        name: String::from("Contrast"),
        fills: alloc::vec![Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt1)), Fill::solid(slot(Slot::Lt2))],
        lines: alloc::vec![
            Line::solid(150, slot(Slot::Dk1)).arrow(4, 2),
            Line::solid(150, ph()).arrow(4, 2),
            Line::solid(225, slot(Slot::Dk1)).arrow(4, 3),
        ],
        effects: none_effects(3),
        backgrounds: backgrounds(),
        fonts: alloc::vec![FontStyle::bold(slot(Slot::Dk1)), FontStyle::regular(slot(Slot::Dk2)), FontStyle::regular(slot(Slot::Lt1))],
    };
    let flat = [VarStyle::new(1, 1, 1, 2), VarStyle::new(1, 1, 1, 2), VarStyle::new(3, 2, 1, 5), VarStyle::new(6, 6, 1, 4)];
    let rounded = [VarStyle::new(1, 5, 1, 2), VarStyle::new(1, 5, 1, 2), VarStyle::new(3, 5, 1, 5), VarStyle::new(6, 6, 1, 4)];
    Design {
        name: String::from("Contrast"),
        id: CONTRAST,
        colors: colors([SIGNALBLAU, OCHRE, TEAL, BRICK, VIOLET, OLIVE], SLATE, WINDOW),
        background: WHITE,
        major_font: String::from(FONT),
        minor_font: String::from(FONT),
        shapes,
        connectors,
        variants: [
            variant([SIGNALBLAU, OCHRE, TEAL, BRICK, VIOLET, OLIVE, ANTHRACITE], 3, flat),
            variant([TEAL, SIGNALBLAU, OLIVE, OCHRE, VIOLET, BRICK, ANTHRACITE], 3, flat),
            variant([BRICK, OCHRE, SIGNALBLAU, TEAL, VIOLET, OLIVE, ANTHRACITE], 3, flat),
            variant([ANTHRACITE, SIGNALBLAU, TEAL, OCHRE, BRICK, VIOLET, SLATE], 3, rounded),
        ],
    }
}

/// all own designs, NetLab first
pub fn all() -> Vec<Design> {
    alloc::vec![netlab(), contour(), contrast()]
}

/// the own design with this scheme number
pub fn by_id(id: u32) -> Option<Design> {
    all().into_iter().find(|d| d.id == id)
}
