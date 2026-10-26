//! Effect execution, previews and adjustment dialogs.
pub(crate) mod controls;
pub(crate) mod curves;
pub(crate) mod levels;

// fx -- PaintCraft: Korrekturen und Effekte (agent A6).  The effect runner: menu rows ->
// registered effects (fx_use_all), German names and property labels, live preview through
// the compose tool hook, tiles on the worker pool, debounced restarts, commit through the
// selection as one history entry, "<Effekt> wiederholen" (Strg+F), remembered parameters.
//
// Module contract (main.s):
//   fx_init()               fx_use_all(); command rows -> FXDESC (fx_map); plugin rows
//   fx_states()             poll the running effect; rows of registered effects enabled while
//                           an image is open; CMD_FX_REPEAT after the first effect
//   fx_frame()              the effect dialog (fxui.s / adj_curves.s / adj_levels.s) and the
//                           progress window "Rendern..."
//   fx_cmd(id, arg) -> eax  380..699, CMD_FX_OK / CMD_FX_CANCEL, UIC_TIMER (TMR_FX_DEBOUNCE),
//                           UIC_WAKE (tiles finished), UIC_WINCLOSE of the effect windows
// Public (pn_fx.inc): fx_jobs_pending, fx_params_changed, fx_name, fx_label, fx_choices,
//   fx_place_dialog, fx_progress, fx_settings_save, fx_settings_load, fx_diag_cmd,
//   fx_diag_frame, g_fx (the running effect)
//
// Runner (docs/EFFECTS.md):
//   1. tool_commit_live; copy the current layer (FXA_SRC, straight ARGB), copy the selection
//      mask (FXA_SEL); area = selection bounds or the whole layer
//   2. tiles of 256 x 256 (FXF_BANDS: full-width bands of 128 rows, FXF_WHOLE: one tile),
//      sorted centre-out from the visible part of the image
//   3. FXD_PREPARE on a worker (a job of one item), then the tiles as a background job; every
//      finished tile sets its state byte and calls plat_wake, the UI thread shows it
//      (comp_mark) on UIC_WAKE / the next frame
//   4. preview = compose hook fx_row: finished tiles lerp(layer, dst, sel_clip_row coverage),
//      unfinished ones the layer; the commit runs the same fx_row over the layer rows
//   5. a parameter change cancels the job (JOB_CANCEL), the render restarts 50 ms after the
//      last change (TMR_FX_DEBOUNCE)
//   6. OK: commit once complete (up to 1 Mpx the UI thread waits, larger images show the
//      progress window "Rendern..." with Abbrechen), one history entry named after the
//      effect; Abbrechen: job cancelled, preview removed, no entry (HIST-019)
//                                              [apps/PaintCraft/src/fx.s]
//
// Rust model:
// - FXA_DST is not one surface: every tile renders into a buffer of its own (the job's
//   `pending` slot); the UI thread moves it to `shown` when it marks the tile (fs_mark_tiles),
//   so the compose hook (worker threads, &Pn) reads only buffers the UI thread owns.  The
//   asm's FS_DST is read by the hook while a restart overwrites it (a benign race there).
// - The job data (source copy, selection copy, tile grid, state bytes, counters) is shared
//   with the background jobs through an Arc (FxData); the parameters and the prepare result
//   of one run through another (Run).
// - `sync` (tool::SYNC_FX): the UI's sync hook gets no context.  fx_sync_update publishes
//   fx_jobs_pending's answer after every state change; the restart that fx_jobs_pending
//   starts at once is started by the next fs_poll when the hook asked (FX_SYNC_ASK, set by
//   tool.rs's hook).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering::SeqCst};

use ::fx::{Aux, Dst, FxArgs, FxDesc, Params, FP_COLOR, FXF_ADJ, FXF_BANDS, FXF_NO_DIALOG, FXF_WHOLE, FX_NSLOTS};
use gfx::tmask::TMask;
use gfx::{Surface, FMT_ARGB};
use rt::sync::SpinLock;
use ui::*;

use crate::document::composite::{comp_mark, comp_set_tool, CompHook, HookKey};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_cur_layer, doc_index, doc_layer, Doc, DocId};
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_flags;
use crate::app::commands::pn_cmd_label;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const MEM_MAX: usize = 128; // remembered parameter blocks (registry index)
const PARAM_BYTES: usize = FX_NSLOTS * 8;
const SYNC_PX: u64 = 1 << 20; // up to 1 Mpx OK / immediate effects finish at once
const DEBOUNCE_MS: u32 = 50;
const PLUG_ADJ_MAX: usize = (CMD_ADJ_LAST + 1 - CMD_ADJ_PLUGIN0) as usize;
const PLUG_FX_MAX: usize = (CMD_FX_LAST + 1 - CMD_FX_PLUGIN0) as usize;
const REP_CAP: usize = 160;
const PLUG_LIST_CAP: usize = 2048; // STRLIST buffers of the plugin rows
const PLUG_NAME_MAX: usize = 48;
const PROG_CAP: usize = 64;
const DIAG_CAP: usize = 64;
const LBL_ANY: u32 = 0xFFFF; // label row for every effect (common keys)
pub(crate) const ERR_PLUGIN_FAULT: i32 = -64; // P1: plugin.inc ERR_FAULT (a plugin crashed)
const ERR_CANCEL: i32 = ::fx::ERR_CANCEL;

pub(crate) const FSM_NONE: u32 = 0; // no effect
pub(crate) const FSM_DIALOG: u32 = 1; // dialog open, live preview
pub(crate) const FSM_APPLY: u32 = 2; // OK / no dialog: commit as soon as the render is complete
pub(crate) const FSP_IDLE: u32 = 0;
pub(crate) const FSP_PREPARE: u32 = 1;
pub(crate) const FSP_RENDER: u32 = 2;
pub(crate) const FSP_DONE: u32 = 3; // every tile rendered with FS_RPARAM
pub(crate) const FSP_FAILED: u32 = 4;
pub(crate) const FSK_GENERIC: u32 = 0; // fxui.rs
pub(crate) const FSK_CURVES: u32 = 1; // adj_curves.rs
pub(crate) const FSK_LEVELS: u32 = 2; // adj_levels.rs
pub(crate) const FSK_NONE: u32 = 3; // applied at once (FXF_NO_DIALOG, Strg+F)

const S_NOMEM: &str = "Zu wenig Arbeitsspeicher für diesen Effekt.";
const S_REP_DEFAULT: &str = "Letzten Effekt wiederholen";
const S_SEC_FX: &str = "Effekte";
const S_KEY_LAST: &str = "_last";

/// tool.rs's sync hook asked (a script `sync` waits): fs_poll starts a debounced restart at
/// once, as the asm's fx_jobs_pending does
pub(crate) static FX_SYNC_ASK: AtomicBool = AtomicBool::new(false);

// German choice labels
static CH_BLEND: [&str; 14] = [
    "Normal",
    "Multiplizieren",
    "Additiv",
    "Farbig nachbelichten",
    "Farbig abwedeln",
    "Reflektieren",
    "Leuchten",
    "Überlagerung",
    "Unterschied",
    "Negierung",
    "Aufhellen",
    "Verdunkeln",
    "Bildschirm",
    "XOR",
];
static CH_EDGE: [&str; 7] = ["Abschneiden", "Umlaufend", "Spiegeln", "Primärfarbe", "Sekundärfarbe", "Transparent", "Original"];
static CH_SCHEME: [&str; 4] = ["Klassisch", "Graustufen", "Palette", "Regenbogen"];
static CH_MORPH_MODE: [&str; 4] = ["Erweitern", "Erodieren", "Öffnen", "Schließen"];
static CH_MORPH_SHAPE: [&str; 3] = ["Kreis", "Quadrat", "Raute"];
static CH_QUANT_ALG: [&str; 2] = ["Octree", "Medianschnitt"];
static CH_QUANT_ALPHA: [&str; 2] = ["Beibehalten", "Binär"];
static CH_CURVES: [&str; 2] = ["Leuchtkraft", "RGB"];

use ::fx::{
    FX_ADD_NOISE, FX_AUTO_LEVEL, FX_BLACK_WHITE, FX_BOKEH_BLUR, FX_BRIGHT_CONTRAST, FX_BULGE, FX_CLOUDS, FX_CRYSTALIZE,
    FX_CURVES, FX_DENTS, FX_DROP_SHADOW, FX_EDGE_DETECT, FX_EMBOSS, FX_EXPOSURE, FX_FRAGMENT_BLUR, FX_FROSTED_GLASS,
    FX_GAUSSIAN_BLUR, FX_GLOW, FX_HIGHLIGHTS_SHADOWS, FX_HUE_SATURATION, FX_INK_SKETCH, FX_INVERT_ALPHA,
    FX_INVERT_COLORS, FX_JULIA, FX_LEVELS, FX_MANDELBROT, FX_MEDIAN, FX_MORPHOLOGY, FX_MOTION_BLUR, FX_OIL_PAINTING,
    FX_OUTLINE, FX_PENCIL_SKETCH, FX_PIXELATE, FX_POLAR_INVERSION, FX_POSTERIZE, FX_QUANTIZE, FX_RADIAL_BLUR,
    FX_RED_EYE, FX_REDUCE_NOISE, FX_RELIEF, FX_SEPIA, FX_SHARPEN, FX_SKETCH_BLUR, FX_SOFTEN_PORTRAIT,
    FX_SQUARE_BLUR, FX_STRAIGHTEN, FX_SURFACE_BLUR, FX_TEMP_TINT, FX_TILE_REFLECTION, FX_TURBULENCE, FX_TWIST,
    FX_UNFOCUS, FX_VIGNETTE, FX_ZOOM_BLUR,
};

/// menu rows -> effects (German names, also the history entry names);
/// id 0: no kernel yet, the row stays disabled
static FX_MAP: [(u32, u32, &str); 66] = [
    (CMD_FX_CONTENT_FILL, ::fx::enhance::CONTENT_FILL, "Inhaltsbasiert füllen"),
    (CMD_FX_PORTRAIT, ::fx::enhance::PORTRAIT, "Porträt retuschieren"),
    (CMD_ADJ_BALANCE, ::fx::enhance::COLOR_BALANCE, "Farbbalance"),
    (CMD_ADJ_BW_PLUS, ::fx::enhance::BLACK_WHITE_PLUS, "Schwarzweiß erweitert"),
    (CMD_FX_DREAM, ::fx::enhance::DREAM, "Traum"),
    (CMD_FX_OIL_PLUS, ::fx::enhance::OIL_PLUS, "Ölgemälde erweitert"),
    (CMD_FX_PASTEL, ::fx::enhance::PASTEL, "Pastell"),
    (CMD_FX_FEATHER, ::fx::enhance::FEATHER, "Weiche Objektkante"),
    (CMD_FX_INNER_SHADOW, ::fx::enhance::INNER_SHADOW, "Innerer Schatten"),
    (CMD_FX_BEVEL, ::fx::enhance::BEVEL, "Abgeflachte Kante"),
    (CMD_FX_DODGE_BURN, ::fx::enhance::DODGE_BURN, "Abwedeln / Nachbelichten"),
    (CMD_ADJ_AUTO, FX_AUTO_LEVEL, "Automatisch"),
    (CMD_ADJ_BW, FX_BLACK_WHITE, "Schwarzweiß"),
    (CMD_ADJ_BRICON, FX_BRIGHT_CONTRAST, "Helligkeit / Kontrast"),
    (CMD_ADJ_CURVES, FX_CURVES, "Kurven"),
    (CMD_ADJ_EXPOSURE, FX_EXPOSURE, "Belichtung"),
    (CMD_ADJ_HISHADOW, FX_HIGHLIGHTS_SHADOWS, "Tiefen / Lichter"),
    (CMD_ADJ_HUESAT, FX_HUE_SATURATION, "Farbton / Sättigung"),
    (CMD_ADJ_INV_ALPHA, FX_INVERT_ALPHA, "Alpha umkehren"),
    (CMD_ADJ_INVERT, FX_INVERT_COLORS, "Farben umkehren"),
    (CMD_ADJ_LEVELS, FX_LEVELS, "Manuelle Anpassung"),
    (CMD_ADJ_POSTERIZE, FX_POSTERIZE, "Tontrennung"),
    (CMD_ADJ_SEPIA, FX_SEPIA, "Sepia"),
    (CMD_ADJ_TEMPTINT, FX_TEMP_TINT, "Farbtemperatur / Tönung"),
    (CMD_FX_INK_SKETCH, FX_INK_SKETCH, "Skizze nachzeichnen"),
    (CMD_FX_OIL, FX_OIL_PAINTING, "Ölgemälde"),
    (CMD_FX_PENCIL_SKETCH, FX_PENCIL_SKETCH, "Bleistiftskizze"),
    (CMD_FX_BOKEH, FX_BOKEH_BLUR, "Bokeh"),
    (CMD_FX_FRAGMENT, FX_FRAGMENT_BLUR, "Verwackelung"),
    (CMD_FX_GAUSS, FX_GAUSSIAN_BLUR, "Gaußscher Weichzeichner"),
    (CMD_FX_MEDIAN_BLUR, 0, "Medianunschärfe"),
    (CMD_FX_MOTION_BLUR, FX_MOTION_BLUR, "Bewegungsunschärfe"),
    (CMD_FX_RADIAL_BLUR, FX_RADIAL_BLUR, "Radialer Weichzeichner"),
    (CMD_FX_SKETCH_BLUR, FX_SKETCH_BLUR, "Skizzenunschärfe"),
    (CMD_FX_SQUARE_BLUR, FX_SQUARE_BLUR, "Box-Weichzeichner"),
    (CMD_FX_SURFACE_BLUR, FX_SURFACE_BLUR, "Kantenerhaltende Unschärfe"),
    (CMD_FX_UNFOCUS, FX_UNFOCUS, "Unschärfe"),
    (CMD_FX_ZOOM_BLUR, FX_ZOOM_BLUR, "Zoom-Weichzeichner"),
    (CMD_FX_QUANTIZE, FX_QUANTIZE, "Quantize"),
    (CMD_FX_BULGE, FX_BULGE, "Wölbung"),
    (CMD_FX_CRYSTALLIZE, FX_CRYSTALIZE, "Kristallisieren"),
    (CMD_FX_DENTS, FX_DENTS, "Dellen"),
    (CMD_FX_FROSTED, FX_FROSTED_GLASS, "Milchglas"),
    (CMD_FX_MORPHOLOGY, FX_MORPHOLOGY, "Morphologie"),
    (CMD_FX_PIXELATE, FX_PIXELATE, "In Pixel auflösen"),
    (CMD_FX_POLAR_INV, FX_POLAR_INVERSION, "Polarinvertierung"),
    (CMD_FX_TILE_REFL, FX_TILE_REFLECTION, "Kachelspiegelung"),
    (CMD_FX_TWIST, FX_TWIST, "Spirale"),
    (CMD_FX_ADD_NOISE, FX_ADD_NOISE, "Rauschen hinzufügen"),
    (CMD_FX_MEDIAN, FX_MEDIAN, "Median"),
    (CMD_FX_REDUCE_NOISE, FX_REDUCE_NOISE, "Rauschen reduzieren"),
    (CMD_FX_DROP_SHADOW, FX_DROP_SHADOW, "Schlagschatten"),
    (CMD_FX_GLOW, FX_GLOW, "Leuchten"),
    (CMD_FX_RED_EYE, FX_RED_EYE, "Rote Augen entfernen"),
    (CMD_FX_SHARPEN, FX_SHARPEN, "Verschärfen"),
    (CMD_FX_SOFTEN, FX_SOFTEN_PORTRAIT, "Porträt weichzeichnen"),
    (CMD_FX_STRAIGHTEN, FX_STRAIGHTEN, "Begradigen"),
    (CMD_FX_VIGNETTE, FX_VIGNETTE, "Vignette"),
    (CMD_FX_CLOUDS, FX_CLOUDS, "Wolken"),
    (CMD_FX_JULIA, FX_JULIA, "Julia-Fraktal"),
    (CMD_FX_MANDELBROT, FX_MANDELBROT, "Mandelbrot-Fraktal"),
    (CMD_FX_TURBULENCE, FX_TURBULENCE, "Turbulenz"),
    (CMD_FX_EDGE, FX_EDGE_DETECT, "Kantenerkennung"),
    (CMD_FX_EMBOSS, FX_EMBOSS, "Einprägen"),
    (CMD_FX_OUTLINE, FX_OUTLINE, "Umriss"),
    (CMD_FX_RELIEF, FX_RELIEF, "Gravur"),
];

/// the rows of fx_map in order
fn map_rows() -> impl Iterator<Item = &'static (u32, u32, &'static str)> {
    FX_MAP.iter()
}

/// property labels: effect-specific rows first, then LBL_ANY rows for common keys (see
/// docs/EFFECTS.md)
static FX_LABELS: &[(u32, &str, &str, &[&str])] = &[
    (FX_BRIGHT_CONTRAST, "brightness", "Helligkeit", &[]),
    (FX_BRIGHT_CONTRAST, "contrast", "Kontrast", &[]),
    (FX_CURVES, "mode", "Transferzuordnung", &CH_CURVES),
    (FX_EXPOSURE, "exposure", "Belichtung", &[]),
    (FX_HIGHLIGHTS_SHADOWS, "shadows", "Tiefen", &[]),
    (FX_HIGHLIGHTS_SHADOWS, "highlights", "Lichter", &[]),
    (FX_HUE_SATURATION, "hue", "Farbton", &[]),
    (FX_HUE_SATURATION, "saturation", "Sättigung", &[]),
    (FX_HUE_SATURATION, "lightness", "Helligkeit", &[]),
    (FX_POSTERIZE, "red", "Rot", &[]),
    (FX_POSTERIZE, "green", "Grün", &[]),
    (FX_POSTERIZE, "blue", "Blau", &[]),
    (FX_POSTERIZE, "linked", "Verbunden", &[]),
    (FX_TEMP_TINT, "temperature", "Temperatur", &[]),
    (FX_TEMP_TINT, "tint", "Tönung", &[]),
    (FX_INK_SKETCH, "ink_outline", "Kontur nachzeichnen", &[]),
    (FX_INK_SKETCH, "coloring", "Färbung", &[]),
    (FX_OIL_PAINTING, "brush_size", "Pinselgröße", &[]),
    (FX_OIL_PAINTING, "coarseness", "Farbvarianz", &[]),
    (FX_PENCIL_SKETCH, "pencil_tip_size", "Bleistiftdicke", &[]),
    (FX_PENCIL_SKETCH, "color_range", "Bereich", &[]),
    (FX_BOKEH_BLUR, "granularity", "Granularität", &[]),
    (FX_FRAGMENT_BLUR, "fragments", "Verwackelungseffekt", &[]),
    (FX_FRAGMENT_BLUR, "distance", "Entfernung", &[]),
    (FX_FRAGMENT_BLUR, "rotation", "Drehung", &[]),
    (FX_MOTION_BLUR, "centered", "Zentriert", &[]),
    (FX_SKETCH_BLUR, "spread", "Streuung", &[]),
    (FX_ZOOM_BLUR, "amount", "Distanz", &[]),
    (FX_QUANTIZE, "colors", "Farben", &[]),
    (FX_QUANTIZE, "algorithm", "Algorithmus:", &CH_QUANT_ALG),
    (FX_QUANTIZE, "dithering", "Dithering-Level", &[]),
    (FX_QUANTIZE, "alpha", "Transparenz:", &CH_QUANT_ALPHA),
    (FX_BULGE, "amount", "Wölbung", &[]),
    (FX_BULGE, "offset", "Mittig", &[]),
    (FX_CRYSTALIZE, "cell_size", "Zellgröße", &[]),
    (FX_DENTS, "scale", "Skalierung", &[]),
    (FX_DENTS, "refraction", "Brechung", &[]),
    (FX_DENTS, "roughness", "Detail", &[]),
    (FX_DENTS, "tension", "Turbulenz", &[]),
    (FX_FROSTED_GLASS, "scatter_radius", "Größter Streuradius", &[]),
    (FX_FROSTED_GLASS, "smoothness", "Glättung", &[]),
    (FX_MORPHOLOGY, "mode", "Modus:", &CH_MORPH_MODE),
    (FX_MORPHOLOGY, "shape", "Form:", &CH_MORPH_SHAPE),
    (FX_PIXELATE, "cell_size", "Kantenlänge", &[]),
    (FX_POLAR_INVERSION, "amount", "Skalierung", &[]),
    (FX_POLAR_INVERSION, "offset", "Versatz", &[]),
    (FX_TILE_REFLECTION, "tile_size", "Kachelgröße", &[]),
    (FX_TILE_REFLECTION, "curvature", "Biegung", &[]),
    (FX_TWIST, "amount", "Stärke / Richtung", &[]),
    (FX_TWIST, "size", "Größe", &[]),
    (FX_ADD_NOISE, "color_saturation", "Farbsättigung", &[]),
    (FX_ADD_NOISE, "coverage", "Abdeckung", &[]),
    (FX_MEDIAN, "percentile", "Perzentil", &[]),
    (FX_DROP_SHADOW, "distance", "Distanz", &[]),
    (FX_DROP_SHADOW, "blur_radius", "Schattenradius", &[]),
    (FX_DROP_SHADOW, "opacity", "Abdeckung", &[]),
    (FX_DROP_SHADOW, "shadow_only", "Nur Schatten", &[]),
    (FX_RED_EYE, "tolerance", "Toleranz", &[]),
    (FX_RED_EYE, "saturation", "Sättigung", &[]),
    (FX_SHARPEN, "amount", "Betrag", &[]),
    (FX_SOFTEN_PORTRAIT, "softness", "Weichheit", &[]),
    (FX_SOFTEN_PORTRAIT, "lighting", "Beleuchtung", &[]),
    (FX_SOFTEN_PORTRAIT, "warmth", "Wärme", &[]),
    (FX_STRAIGHTEN, "zoom", "Automatisch zoomen", &[]),
    (FX_VIGNETTE, "density", "Stärke", &[]),
    (FX_CLOUDS, "power", "Grobheit", &[]),
    (FX_JULIA, "zoom", "Vergrößern", &[]),
    (FX_MANDELBROT, "zoom", "Vergrößern", &[]),
    (FX_MANDELBROT, "invert_colors", "Farben umkehren", &[]),
    (FX_TURBULENCE, "scale", "Größe", &[]),
    (FX_TURBULENCE, "detail", "Oktaven", &[]),
    (FX_OUTLINE, "thickness", "Linienstärke", &[]),
    (LBL_ANY, "radius", "Radius", &[]),
    (LBL_ANY, "angle", "Winkel", &[]),
    (LBL_ANY, "quality", "Qualität", &[]),
    (LBL_ANY, "seed", "Zufällig anordnen", &[]),
    (LBL_ANY, "offset", "Mitte", &[]),
    (LBL_ANY, "distance", "Distanz", &[]),
    (LBL_ANY, "amount", "Betrag", &[]),
    (LBL_ANY, "intensity", "Intensität", &[]),
    (LBL_ANY, "strength", "Stärke", &[]),
    (LBL_ANY, "threshold", "Schwellenwert", &[]),
    (LBL_ANY, "brightness", "Helligkeit", &[]),
    (LBL_ANY, "contrast", "Kontrast", &[]),
    (LBL_ANY, "scale", "Skalierung", &[]),
    (LBL_ANY, "factor", "Faktor", &[]),
    (LBL_ANY, "roughness", "Grobheit", &[]),
    (LBL_ANY, "color", "Farbe", &[]),
    (LBL_ANY, "primary_color", "Primärfarbe", &[]),
    (LBL_ANY, "secondary_color", "Sekundärfarbe", &[]),
    (LBL_ANY, "blend_mode", "Mischmodus:", &CH_BLEND),
    (LBL_ANY, "edge_behavior", "Kantenverhalten:", &CH_EDGE),
    (LBL_ANY, "color_scheme", "Farbschema:", &CH_SCHEME),
];

// ================================================================== records

/// the tile grid of the running effect (FS_BX0 .. FS_THS)
#[derive(Clone, Copy, Default)]
pub(crate) struct Grid {
    /// the area to render (selection bounds or the layer), exclusive
    pub(crate) bx0: i32,
    pub(crate) by0: i32,
    pub(crate) bx1: i32,
    pub(crate) by1: i32,
    /// layer size
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// log2 tile width (8; 16 = full width), log2 tile height (8; 7 bands; 16 = whole)
    tws: u32,
    ths: u32,
    c0: u32,
    r0: u32,
    ncols: u32,
    pub(crate) nt: u32,
}

impl Grid {
    // fs_tile_rect(index) -- tile `index` (row-major over the grid) clipped to the area
    fn tile_rect(&self, index: u32) -> [i32; 4] {
        let row = index / self.ncols;
        let col = index % self.ncols + self.c0;
        let x0 = ((col << self.tws) as i32).max(self.bx0);
        let x1 = (((col << self.tws) as i32).wrapping_add(1 << self.tws)).min(self.bx1);
        let r = row + self.r0;
        let y0 = ((r << self.ths) as i32).max(self.by0);
        let y1 = (((r << self.ths) as i32).wrapping_add(1 << self.ths)).min(self.by1);
        [x0, y0, x1, y1]
    }
}

/// what the background jobs share with the UI thread (the asm's g_fx fields the workers
/// touch): source, selection copy, tile order, state bytes, counters, rendered tiles
pub(crate) struct FxData {
    desc: &'static FxDesc,
    /// FS_SRC: copy of the layer (FMT_ARGB)
    src: Surface,
    /// FS_SEL: private copy of the selection (FXA_SEL)
    sel: Option<TMask>,
    g: Grid,
    /// FS_ORDER: tile indexes in render order
    order: Vec<u32>,
    /// FS_TSTATE: bit 0 rendered, bit 1 to show (worker), bit 2 shown (UI thread)
    state: Vec<AtomicU8>,
    /// tiles rendered and not shown yet (the UI thread moves them to FxCore::shown)
    pending: Vec<SpinLock<Option<Vec<u8>>>>,
    /// FS_NDONE / FS_UNMARKED / FS_ERR
    ndone: AtomicU32,
    unmarked: AtomicU32,
    err: AtomicI32,
    /// the result of FXD_PREPARE (taken by the UI thread for the tile run)
    prep: SpinLock<Option<Aux>>,
}

/// one render: FS_RPARAM and FS_AUX
struct Run {
    params: Params,
    aux: Option<Aux>,
}

/// the part of the running effect the compose hook reads (Sync)
#[derive(Default)]
pub(crate) struct FxCore {
    /// FS_HOOK: fx_row is the compose hook of the document
    on: bool,
    pub(crate) g: Grid,
    data: Option<Arc<FxData>>,
    /// the pixels of every shown tile (its rectangle, stride (x1 - x0) * 4)
    shown: Vec<Vec<u8>>,
}

/// the running effect (g_fx, FS_* of pn_fx.inc) and the plugin rows of the menus
pub(crate) struct Fx {
    /// fx_plug_list_adj: plugin adjustment names (row k = CMD_ADJ_PLUGIN0 + k; ui.menu_list)
    pub(crate) plug_list_adj: Vec<String>,
    /// fx_plug_list_fx: plugin effect names (row k = CMD_FX_PLUGIN0 + k)
    pub(crate) plug_list_fx: Vec<String>,
    pub(crate) core: FxCore,
    job: plat::Job,
    /// FS_DESC
    pub(crate) desc: Option<&'static FxDesc>,
    /// FS_DOC (None = the document was closed under the effect)
    pub(crate) doc: Option<DocId>,
    pub(crate) layer: i32,
    pub(crate) mode: u32,
    pub(crate) phase: u32,
    pub(crate) kind: u32,
    /// FS_NAME: German name
    pub(crate) name: &'static str,
    icon: u32,
    /// FS_FXIDX: registry index (parameter memory), -1 = none
    fxidx: i32,
    restarts: u32,
    dirty: bool,
    due: bool,
    /// FS_PARAM: the dialog's parameters
    pub(crate) param: Params,
    /// FS_RPARAM: the parameters of the running render
    rparam: Params,
    /// FS_OPEN: the dialog window's open flag (fxui / curves / levels)
    pub(crate) open: Shared<i32>,
    progopen: Shared<i32>,
    prog_rect: Shared<WinRect>,
    /// g_map_desc: FXDESC per fx_map row (None = not registered)
    map_desc: Vec<Option<&'static FxDesc>>,
    plug_adj: Vec<&'static FxDesc>,
    plug_fx: Vec<&'static FxDesc>,
    used_adj: usize,
    used_fx: usize,
    last_desc: Option<&'static FxDesc>,
    last_name: Option<&'static str>,
    mem: Vec<Params>,
    memok: [bool; MEM_MAX],
    async_on: u32,
    rep_label: String,
    /// (Rust) the repeat label waits for pn_cmd_label (fx_settings_load has no Pn)
    rep_pending: bool,
    diag_txt: String,
}

impl Default for Fx {
    fn default() -> Fx {
        Fx {
            plug_list_adj: Vec::new(),
            plug_list_fx: Vec::new(),
            core: FxCore::default(),
            job: plat::Job::new(),
            desc: None,
            doc: None,
            layer: 0,
            mode: FSM_NONE,
            phase: FSP_IDLE,
            kind: 0,
            name: "",
            icon: 0,
            fxidx: -1,
            restarts: 0,
            dirty: false,
            due: false,
            param: Params::default(),
            rparam: Params::default(),
            open: Shared::new(0),
            progopen: Shared::new(0),
            prog_rect: Shared::new(WinRect::new(0, 0, 360, 0, 0)),
            map_desc: Vec::new(),
            plug_adj: Vec::new(),
            plug_fx: Vec::new(),
            used_adj: 0,
            used_fx: 0,
            last_desc: None,
            last_name: None,
            mem: vec![Params::default(); MEM_MAX],
            memok: [false; MEM_MAX],
            async_on: 0,
            rep_label: String::new(),
            rep_pending: false,
            diag_txt: String::new(),
        }
    }
}

// ================================================================== module contract

/// fx_init() -- register every built-in effect, map the menu rows
pub(crate) fn fx_init(p: &mut Pn, ui: &mut Ui) {
    ::fx::use_all();
    p.fx.map_desc = map_rows().map(|r| if r.1 != 0 { ::fx::find_id(r.1) } else { None }).collect();
    plug_rows(p);
    p.fx.fxidx = -1;
    ui.menu_list(CMD_ADJ_PLUGIN0, &p.fx.plug_list_adj);
    ui.menu_list(CMD_FX_PLUGIN0, &p.fx.plug_list_fx);
}

// plug_rows() -- registered effects without a fixed row get a plugin row (CMD_ADJ_PLUGIN0.. /
// CMD_FX_PLUGIN0..; label = the effect's key) and a string in fx_plug_list_adj / _fx (the
// rows of Mi::List CMD_ADJ_PLUGIN0 / CMD_FX_PLUGIN0 in the menus)
fn plug_rows(p: &mut Pn) {
    for i in 0..::fx::count() {
        let Some(d) = ::fx::get(i) else { continue };
        if map_row_of(&p.fx, d).is_some() {
            continue;
        }
        let adj = d.flags & FXF_ADJ != 0;
        let id = if adj {
            if p.fx.plug_adj.len() >= PLUG_ADJ_MAX {
                continue;
            }
            p.fx.plug_adj.push(d);
            CMD_ADJ_PLUGIN0 + p.fx.plug_adj.len() as u32 - 1
        } else {
            if p.fx.plug_fx.len() >= PLUG_FX_MAX {
                continue;
            }
            p.fx.plug_fx.push(d);
            CMD_FX_PLUGIN0 + p.fx.plug_fx.len() as u32 - 1
        };
        pn_cmd_label(p, id, d.name);
        // the STRLIST of the menu rows (adjustments / effects)
        let f = &mut p.fx;
        let (list, used) = if adj { (&mut f.plug_list_adj, &mut f.used_adj) } else { (&mut f.plug_list_fx, &mut f.used_fx) };
        if *used > PLUG_LIST_CAP - PLUG_NAME_MAX - 2 {
            continue;
        }
        let s = fmt_cap(String::from(d.name), PLUG_NAME_MAX);
        *used += s.len() + 1;
        list.push(s);
    }
}

// map_row_of(desc) -> fx_map row of desc, None = none
fn map_row_of(f: &Fx, d: &FxDesc) -> Option<usize> {
    f.map_desc.iter().position(|x| x.is_some_and(|x| core::ptr::eq(x, d)))
}

/// fx_states() -- poll the running effect; command states
pub(crate) fn fx_states(p: &mut Pn, ui: &mut Ui) {
    if p.fx.rep_pending {
        p.fx.rep_pending = false;
        rep_label(p);
    }
    fs_poll(p, ui);
    let doc = doc_active(&p.doc).is_some();
    for (i, r) in map_rows().enumerate() {
        let on = doc && p.fx.map_desc.get(i).is_some_and(|d| d.is_some()) && (r.0!=CMD_FX_CONTENT_FILL || p.doc.active().is_some_and(crate::document::selection::sel_active));
        pn_cmd_set(ui, r.0, if on { 0 } else { CF_DISABLED });
    }
    let on = doc && p.fx.last_desc.is_some();
    pn_cmd_set(ui, CMD_FX_REPEAT, if on { 0 } else { CF_DISABLED });
    fx_sync_update(&p.fx);
}

/// fx_frame() -- the dialog of the running effect, the progress window
pub(crate) fn fx_frame(p: &mut Pn, ui: &mut Ui) {
    if p.fx.open.get() != 0 {
        match p.fx.kind {
            FSK_CURVES => crate::effects::curves::curves_frame(p, ui),
            FSK_LEVELS => crate::effects::levels::levels_frame(p, ui),
            FSK_GENERIC => crate::effects::controls::fxui_frame(p, ui),
            _ => {}
        }
    }
    if p.fx.progopen.get() != 0 {
        prog_frame(p, ui);
    }
    fx_sync_update(&p.fx);
}

// prog_frame() -- paint.net's applying dialog: "Rendern... n%", progress bar, Abbrechen
fn prog_frame(p: &mut Pn, ui: &mut Ui) {
    let f = &p.fx;
    let open = f.progopen.clone();
    let rect = f.prog_rect.clone();
    if ui.window_begin(WINID_FXPROG, f.name, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let (done, total, _) = fx_progress(f);
        let pct = if total != 0 { done.wrapping_mul(100) / total } else { 0 };
        let t = fmt_cap(format!("Rendern... {}%", pct as i32), PROG_CAP);
        ui.next_size(-1, 0); // fixed width: the text changes between passes
        ui.text(TS_BODY, &t);
        ui.next_size(-1, 0);
        ui.progress(ID_FX_PROG, done as i32, total as i32);
        if ui.dialog_buttons(None, None, Some(crate::preferences::localization::tr("Abbrechen"))) == DLG_CLOSE {
            ui.cmd_run(CMD_FX_CANCEL, 0);
        }
    }
    ui.window_end();
}

/// fx_cmd(id, arg) -> eax 1 handled
pub(crate) fn fx_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let r = fx_cmd2(p, ui, id, arg);
    fx_sync_update(&p.fx);
    r
}

fn fx_cmd2(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        UIC_TIMER => {
            if arg != TMR_FX_DEBOUNCE {
                return false;
            }
            p.fx.due = true;
            fs_poll(p, ui);
            return true;
        }
        UIC_WAKE => {
            fs_poll(p, ui);
            return false; // UIC_WAKE reaches every module anyway
        }
        UIC_WINCLOSE => {
            if arg.wrapping_sub(WINID_FX) <= WINID_FXPROG - WINID_FX {
                fs_end(p, ui);
            }
            return false; // the library hides the window
        }
        _ => {}
    }
    if id & 0xFFFF_0000 == 0x7FFF_0000 {
        return false;
    }
    match id {
        CMD_FX_OK => fx_ok(p, ui),
        CMD_FX_CANCEL => fs_end(p, ui),
        CMD_FX_REPEAT => {
            let (d, n) = (p.fx.last_desc, p.fx.last_name.unwrap_or(""));
            fx_open(p, ui, d, n, true);
        }
        _ => {
            if let Some(k) = map_rows().position(|r| r.0 == id) {
                let d = p.fx.map_desc.get(k).copied().flatten();
                let name = map_rows().nth(k).map_or("", |r| r.2);
                fx_open(p, ui, d, name, false);
            } else if let Some(&d) = id.checked_sub(CMD_ADJ_PLUGIN0).and_then(|k| p.fx.plug_adj.get(k as usize)) {
                fx_open(p, ui, Some(d), d.name, false);
            } else if let Some(&d) = id.checked_sub(CMD_FX_PLUGIN0).and_then(|k| p.fx.plug_fx.get(k as usize)) {
                fx_open(p, ui, Some(d), d.name, false);
            }
        }
    }
    true
}

// ================================================================== starting / ending

// fx_open(desc, name, repeat) -- run an effect: with its dialog (repeat false and the effect
// has one) or at once with the remembered / default parameters
fn fx_open(p: &mut Pn, ui: &mut Ui, desc: Option<&'static FxDesc>, name: &'static str, repeat: bool) {
    let Some(desc) = desc else { return };
    if p.fx.mode != FSM_NONE {
        return;
    }
    crate::tools::tool_commit_live(p, ui);
    if !fs_begin(p, desc, name) {
        return;
    }
    fs_params_load(p, repeat);
    // the kind of dialog
    let kind = if repeat || desc.flags & FXF_NO_DIALOG != 0 {
        FSK_NONE
    } else if desc.id == FX_CURVES {
        FSK_CURVES
    } else if desc.id == FX_LEVELS {
        FSK_LEVELS
    } else if desc.props.is_empty() {
        FSK_NONE
    } else {
        FSK_GENERIC
    };
    p.fx.kind = kind;
    fs_hook_on(p);
    if kind == FSK_NONE {
        p.fx.mode = FSM_APPLY;
        fs_start(p);
        fs_apply_wait(p, ui);
        return;
    }
    p.fx.mode = FSM_DIALOG;
    p.fx.open.set(1);
    match kind {
        FSK_CURVES => crate::effects::curves::curves_open(p, ui),
        FSK_LEVELS => crate::effects::levels::levels_open(p, ui),
        _ => crate::effects::controls::fxui_open(p, ui),
    }
    fs_start(p);
    ui.repass();
}

// fx_ok() -- OK of the dialog: commit as soon as the render with the final values is complete
fn fx_ok(p: &mut Pn, ui: &mut Ui) {
    if p.fx.mode != FSM_DIALOG {
        return;
    }
    p.fx.open.set(0);
    p.fx.mode = FSM_APPLY;
    fs_apply_wait(p, ui);
}

// fs_apply_wait() -- FSM_APPLY: small areas finish on the UI thread and commit now; large
// ones show the progress window and commit from fs_poll
fn fs_apply_wait(p: &mut Pn, ui: &mut Ui) {
    let g = p.fx.core.g;
    let area = (g.bx1 - g.bx0) as u32 as u64 * (g.by1 - g.by0) as u32 as u64;
    if p.fx.async_on == 0 && area <= SYNC_PX {
        fs_finish_now(p);
    }
    fs_poll(p, ui); // complete: committed and ended here
    if p.fx.mode == FSM_APPLY {
        p.fx.progopen.set(1);
        ui.repass();
    }
}

// fs_finish_now() -- wait on the UI thread (helping the pool) until the render with the
// current parameters is complete, failed or cancelled
fn fs_finish_now(p: &mut Pn) {
    loop {
        match p.fx.phase {
            FSP_PREPARE | FSP_RENDER => {
                let _ = p.fx.job.wait();
                fs_step(p);
            }
            _ => {
                if !p.fx.dirty {
                    return;
                }
                fs_start(p);
            }
        }
    }
}

// fs_begin(desc, name) -> true -- set up g_fx for the active image's current layer: area,
// source copy, selection copy, tiles in render order
fn fs_begin(p: &mut Pn, desc: &'static FxDesc, name: &'static str) -> bool {
    let Some(doc) = doc_active(&p.doc) else {
        fs_free(&mut p.fx);
        return false;
    };
    let (mut g, sel, src) = match fs_copy(p.doc.d(doc)) {
        Ok(r) => r,
        Err(nomem) => {
            fs_free(&mut p.fx);
            if nomem {
                crate::integrations::file_io::fio_error(p, Some(name), S_NOMEM);
            }
            return false;
        }
    };
    // tile grid: 256 x 256, full-width bands of 128 rows, or one tile
    g.tws = 8;
    g.ths = 8;
    if desc.flags & FXF_BANDS != 0 {
        g.tws = 16;
        g.ths = 7;
    }
    if desc.flags & FXF_WHOLE != 0 {
        g.tws = 16;
        g.ths = 16;
    }
    g.c0 = (g.bx0 as u32) >> g.tws;
    g.ncols = ((g.bx1 - 1) as u32 >> g.tws) - g.c0 + 1;
    g.r0 = (g.by0 as u32) >> g.ths;
    let nrows = ((g.by1 - 1) as u32 >> g.ths) - g.r0 + 1;
    g.nt = g.ncols * nrows;
    let order = fs_order(p, &g);
    let nt = g.nt as usize;
    let data = FxData {
        desc,
        src,
        sel,
        g,
        order,
        state: (0..nt).map(|_| AtomicU8::new(0)).collect(),
        pending: (0..nt).map(|_| SpinLock::new(None)).collect(),
        ndone: AtomicU32::new(0),
        unmarked: AtomicU32::new(0),
        err: AtomicI32::new(0),
        prep: SpinLock::new(None),
    };
    let f = &mut p.fx;
    f.desc = Some(desc);
    f.doc = Some(doc);
    f.name = name;
    f.layer = p.doc.d(doc).cur;
    f.mode = FSM_NONE;
    f.phase = FSP_IDLE;
    f.kind = 0;
    f.restarts = 0;
    f.dirty = false;
    f.due = false;
    f.param = Params::default();
    f.open.set(0);
    f.progopen.set(0);
    f.icon = if desc.flags & FXF_ADJ != 0 { IC_SLIDERS } else { IC_FILTER };
    f.core = FxCore { on: false, g, data: Some(Arc::new(data)), shown: vec![Vec::new(); nt] };
    f.fxidx = fx_index(desc);
    true
}

// fs_copy(doc) -> (area, selection copy, source copy) | Err(true = out of memory, false =
// nothing to do) -- the first part of fs_begin
fn fs_copy(d: &Doc) -> Result<(Grid, Option<TMask>, Surface), bool> {
    let l = doc_cur_layer(d).ok_or(false)?;
    if l.sf.px.is_empty() {
        return Err(false);
    }
    let (w, h) = (l.sf.w, l.sf.h);
    let mut g = Grid { bx1: w, by1: h, w, h, ..Grid::default() };
    // area: the selection bounds (nothing selected = nothing to do)
    let mut sel = None;
    if crate::document::selection::sel_active(d) {
        let b = crate::document::selection::sel_bbox(d).ok_or(false)?;
        g.bx0 = b[0].max(0);
        g.by0 = b[1].max(0);
        g.bx1 = b[2].min(g.bx1);
        g.by1 = b[3].min(g.by1);
        if g.bx1 <= g.bx0 || g.by1 <= g.by0 {
            return Err(false);
        }
        sel = crate::document::selection::sel_mask(d).cloned();
    }
    // the source copy
    let mut src = crate::document::selection::surf_try(w, h).ok_or(true)?;
    src.fmt = FMT_ARGB;
    let (ss, ls, nb) = (src.stride as usize, l.sf.stride as usize, w as usize * 4);
    for y in 0..h as usize {
        src.px[y * ss..y * ss + nb].copy_from_slice(&l.sf.px[y * ls..y * ls + nb]);
    }
    Ok((g, sel, src))
}

// fs_order(grid) -> the tiles sorted: visible ones first, then by the distance of their centre
// from the centre of the visible part of the image (key: vis << 62 | d2 << 16 | index)
fn fs_order(p: &Pn, g: &Grid) -> Vec<u32> {
    let v = &p.view;
    let r = crate::workspace::view::view_canvas_rect(v);
    let (vx0, vy0) = crate::workspace::view::view_screen_to_doc(v, r[0], r[1]);
    let (vx1, vy1) = crate::workspace::view::view_screen_to_doc(v, r[0].wrapping_add(r[2]), r[1].wrapping_add(r[3]));
    let cx = vx1.wrapping_add(vx0);
    let cy = vy1.wrapping_add(vy0);
    let mut keys: Vec<u64> = (0..g.nt)
        .map(|i| {
            let t = g.tile_rect(i);
            let outside = !(t[2] > vx0 && t[0] < vx1 && t[3] > vy0 && t[1] < vy1);
            let dx = t[0].wrapping_add(t[2]).wrapping_sub(cx) as i64;
            let dy = t[1].wrapping_add(t[3]).wrapping_sub(cy) as i64;
            let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u64;
            (d2 << 16) | i as u64 | (outside as u64) << 62
        })
        .collect();
    keys.sort_unstable(); // (the asm's heap sort: the keys are unique)
    keys.iter().map(|k| (k & 0xFFFF) as u32).collect()
}

// fs_params_load(repeat) -- FS_PARAM = the remembered parameters of the effect, else its
// defaults; primary_color / secondary_color follow the Colors window (not for a repeat)
fn fs_params_load(p: &mut Pn, repeat: bool) {
    let Some(desc) = p.fx.desc else { return };
    let f = &mut p.fx;
    let k = f.fxidx as u32 as usize;
    if k < MEM_MAX && f.memok[k] {
        f.param = f.mem[k];
    } else {
        ::fx::defaults(desc, &mut f.param);
    }
    if repeat {
        return;
    }
    for pr in desc.props.iter() {
        if pr.typ != FP_COLOR {
            continue;
        }
        let c = if pr.name == "primary_color" {
            crate::workspace::palette::pal_primary(&p.pal)
        } else if pr.name == "secondary_color" {
            crate::workspace::palette::pal_secondary(&p.pal)
        } else {
            continue;
        };
        p.fx.param.0[pr.slot as usize & (FX_NSLOTS - 1)] = c as u64;
    }
}

// fx_index(desc) -> registry index, -1 = not registered
fn fx_index(desc: &FxDesc) -> i32 {
    (0..::fx::count()).find(|&i| ::fx::get(i).is_some_and(|d| core::ptr::eq(d, desc))).map_or(-1, |i| i as i32)
}

// fs_hook_on() -- the preview: fx_row becomes the compose hook of the image
fn fs_hook_on(p: &mut Pn) {
    let Some(doc) = p.fx.doc else { return };
    comp_set_tool(p, doc, Some(HookKey::Fx));
    p.fx.core.on = true;
}

// fs_hook_off(mark) -- remove the preview hook (mark: redraw the area)
fn fs_hook_off(p: &mut Pn, mark: bool) {
    if !p.fx.core.on {
        return;
    }
    p.fx.core.on = false;
    let Some(doc) = p.fx.doc else { return };
    comp_set_tool(p, doc, None);
    if mark {
        let g = p.fx.core.g;
        comp_mark(p, doc, g.bx0, g.by0, g.bx1, g.by1);
    }
}

// fs_end() -- stop the effect: cancel and wait for the job, remove the preview, free
// everything, close its windows (no history entry unless fs_commit ran before)
fn fs_end(p: &mut Pn, ui: &mut Ui) {
    if p.fx.mode == FSM_NONE {
        return;
    }
    if !p.fx.job.done() {
        p.fx.job.cancel();
        let _ = p.fx.job.wait();
    }
    fs_hook_off(p, true);
    match p.fx.kind {
        FSK_GENERIC => crate::effects::controls::fxui_close(p, ui),
        FSK_CURVES => crate::effects::curves::curves_close(p, ui),
        FSK_LEVELS => crate::effects::levels::levels_close(p, ui),
        _ => {}
    }
    fs_free(&mut p.fx);
    let f = &mut p.fx;
    f.mode = FSM_NONE;
    f.phase = FSP_IDLE;
    f.open.set(0);
    f.progopen.set(0);
    f.dirty = false;
    p.invalidate();
    p.repass();
}

// fs_free() -- release the buffers of g_fx (the job must be idle)
fn fs_free(f: &mut Fx) {
    f.job = plat::Job::new(); // (drops the closures holding the job data)
    f.core.data = None;
    f.core.shown = Vec::new();
    f.core.g.nt = 0;
}

// ================================================================== the render job

// fs_start() -- render FS_PARAM from the beginning (the job must be idle)
fn fs_start(p: &mut Pn) {
    let f = &mut p.fx;
    let Some(data) = f.core.data.clone() else { return };
    *data.prep.lock() = None;
    data.ndone.store(0, SeqCst);
    data.err.store(0, SeqCst);
    f.dirty = false;
    f.due = false;
    f.restarts = f.restarts.wrapping_add(1);
    f.rparam = f.param;
    let params = f.param;
    if let Some(prep) = data.desc.prepare {
        f.phase = FSP_PREPARE;
        let d = data.clone();
        f.job.set(1, move |_, ctl| {
            let a = FxArgs {
                src: &d.src,
                x0: d.g.bx0,
                y0: d.g.by0,
                x1: d.g.bx1,
                y1: d.g.by1,
                params: &params,
                aux: None,
                job: Some(ctl),
                sel: d.sel.as_ref(),
            };
            match prep(&a) {
                Ok(aux) => *d.prep.lock() = aux,
                Err(e) => d.err.store(e, SeqCst),
            }
        });
        f.job.start();
    } else {
        fs_start_tiles(f, Run { params, aux: None });
    }
    p.invalidate();
}

// fs_start_tiles(run) -- the tiles as a background job
fn fs_start_tiles(f: &mut Fx, run: Run) {
    let Some(data) = f.core.data.clone() else { return };
    f.phase = FSP_RENDER;
    let run = Arc::new(run);
    let nt = data.g.nt as u64;
    f.job.set(nt, move |i, ctl| fs_tile_job(&data, &run, i, ctl));
    f.job.start();
}

// fs_step() -- phase transitions once the job has finished (UI thread)
fn fs_step(p: &mut Pn) {
    let f = &mut p.fx;
    let Some(data) = f.core.data.clone() else { return };
    match f.phase {
        FSP_PREPARE => {
            if !f.job.done() {
                return;
            }
            let cancelled = f.job.wait().is_err();
            let e = data.err.load(SeqCst);
            if cancelled || e == ERR_CANCEL {
                f.phase = FSP_IDLE;
            } else if e != 0 {
                f.phase = FSP_FAILED;
            } else {
                let aux = data.prep.lock().take();
                fs_start_tiles(f, Run { params: f.rparam, aux });
            }
        }
        FSP_RENDER => {
            if !f.job.done() {
                return;
            }
            if data.ndone.load(SeqCst) == data.g.nt {
                f.phase = FSP_DONE;
            } else {
                let e = data.err.load(SeqCst);
                f.phase = if e == 0 || e == ERR_CANCEL { FSP_IDLE } else { FSP_FAILED };
            }
        }
        _ => return,
    }
    p.invalidate();
}

// fs_poll() -- UI thread, every frame and on UIC_WAKE: show finished tiles, advance the
// phases, restart after the debounce, commit an applied effect once complete
fn fs_poll(p: &mut Pn, ui: &mut Ui) {
    let ask = FX_SYNC_ASK.swap(false, SeqCst);
    if p.fx.mode == FSM_NONE {
        return;
    }
    let gone = p.fx.doc.is_none_or(|d| doc_index(&p.doc, d) < 0);
    if gone {
        p.fx.doc = None; // the image was closed under the effect
        p.fx.core.on = false;
        fs_end(p, ui);
        return;
    }
    // (Rust) a script `sync` waits: start a debounced restart at once (fx_jobs_pending)
    if ask && p.fx.dirty {
        let _ = p.fx.job.wait();
        fs_step(p);
        fs_start(p);
    }
    fs_mark_tiles(p);
    fs_step(p);
    let f = &p.fx;
    if f.dirty && (f.due || f.mode == FSM_APPLY) && f.job.done() {
        fs_step(p);
        fs_start(p);
    }
    if p.fx.mode != FSM_APPLY {
        return;
    }
    match p.fx.phase {
        FSP_FAILED => {
            let msg = if p.fx.core.data.as_ref().is_some_and(|d| d.err.load(SeqCst) == ERR_PLUGIN_FAULT) {
                "Das Plugin ist abgestürzt und wurde deaktiviert (siehe Einstellungen ▸ Plugin-Fehler)."
            } else {
                "Der Effekt ist fehlgeschlagen (zu wenig Arbeitsspeicher)."
            };
            let n = p.fx.name;
            crate::integrations::file_io::fio_error(p, Some(n), msg);
            fs_end(p, ui);
        }
        FSP_IDLE => {
            if !p.fx.dirty {
                fs_start(p); // an applied effect always renders to the end
            }
        }
        FSP_DONE => {
            let unmarked = p.fx.core.data.as_ref().map_or(0, |d| d.unmarked.load(SeqCst));
            if !p.fx.dirty && unmarked == 0 {
                fs_commit(p);
                fs_end(p, ui);
            }
        }
        _ => {}
    }
}

// fs_mark_tiles() -- comp_mark the tiles the workers finished since the last call (their
// pixels become the shown ones)
fn fs_mark_tiles(p: &mut Pn) {
    let Some(data) = p.fx.core.data.clone() else { return };
    if data.unmarked.load(SeqCst) == 0 {
        return;
    }
    for i in 0..data.g.nt as usize {
        if data.state[i].load(SeqCst) & 2 == 0 {
            continue;
        }
        data.state[i].fetch_and(!2, SeqCst); // not "to show" any more
        if let Some(px) = data.pending[i].lock().take() {
            p.fx.core.shown[i] = px;
        }
        data.state[i].fetch_or(4, SeqCst); // shown
        data.unmarked.fetch_sub(1, SeqCst);
        if p.fx.core.on {
            if let Some(doc) = p.fx.doc {
                let t = data.g.tile_rect(i as u32);
                comp_mark(p, doc, t[0], t[1], t[2], t[3]);
            }
        }
    }
}

// fs_tile_job(index) -- worker: render tile FS_ORDER[index]; mark it rendered and wake the UI
fn fs_tile_job(d: &FxData, run: &Run, index: usize, ctl: &plat::JobCtl) {
    if ctl.cancelled() {
        return;
    }
    let ti = d.order[index] as usize;
    let t = d.g.tile_rect(ti as u32);
    let stride = (t[2] - t[0]).max(0) as usize * 4;
    // (a whole-image tile is as big as the image)
    let Some(mut buf) = gfx::try_zeroed(stride * (t[3] - t[1]).max(0) as usize) else {
        d.err.store(::fx::ERR_NOMEM, SeqCst);
        return;
    };
    let a = FxArgs {
        src: &d.src,
        x0: t[0],
        y0: t[1],
        x1: t[2],
        y1: t[3],
        params: &run.params,
        aux: run.aux.as_deref(),
        job: Some(ctl),
        sel: d.sel.as_ref(),
    };
    let r = {
        let mut dst = Dst::strided(t, &mut buf, stride);
        (d.desc.render)(&a, &mut dst)
    };
    if r != 0 {
        if r != ERR_CANCEL {
            d.err.store(r, SeqCst);
        }
        return;
    }
    *d.pending[ti].lock() = Some(buf);
    // unmarked counts the tiles with bit 2 set: a tile rendered again before fs_mark_tiles
    // took it (a restart) is already counted
    if d.state[ti].fetch_or(3, SeqCst) & 2 == 0 {
        d.unmarked.fetch_add(1, SeqCst);
    }
    d.ndone.fetch_add(1, SeqCst);
    plat::wake();
}

// fx_seg(core, doc, x, y, n, res, cov) -> SCR_NONE (unchanged) / SCR_PART / SCR_ALL: the
// effect result of pixels x .. x+n-1 of row y (n <= 64, 64-aligned x) in res and the
// selection coverage in cov where the tile is rendered and shown (state bit 2)
fn fx_seg(c: &FxCore, d: &Doc, x: i32, y: i32, n: i32, res: &mut [u8; 256], cov: &mut [u8; 64]) -> i32 {
    let g = &c.g;
    let Some(data) = &c.data else { return SCR_NONE };
    if y < g.by0 || y >= g.by1 || x >= g.bx1 || x.wrapping_add(n) <= g.bx0 {
        return SCR_NONE;
    }
    let ti = (((x as u32) >> g.tws).wrapping_sub(g.c0)).wrapping_add((((y as u32) >> g.ths).wrapping_sub(g.r0)).wrapping_mul(g.ncols)) as usize;
    if ti >= data.state.len() || data.state[ti].load(SeqCst) & 4 == 0 {
        return SCR_NONE;
    }
    let r = crate::document::selection::sel_clip_row(d, x, y, n, cov) as i32;
    if r == SCR_NONE {
        return SCR_NONE;
    }
    // the result pixels (FS_DST: 0 outside the tile)
    let t = g.tile_rect(ti as u32);
    let buf = &c.shown[ti];
    let stride = (t[2] - t[0]).max(0) as usize * 4;
    let n = n.clamp(0, 64);
    let (a, e) = (x.max(t[0]), x.wrapping_add(n).min(t[2])); // the part inside the tile
    let ro = (y - t[1]) as usize * stride;
    if a < e && ro + (e - t[0]) as usize * 4 <= buf.len() {
        let (k0, k1) = ((a - x) as usize * 4, (e - x) as usize * 4);
        res[..k0].fill(0);
        res[k0..k1].copy_from_slice(&buf[ro + (a - t[0]) as usize * 4..ro + (e - t[0]) as usize * 4]);
        res[k1..n as usize * 4].fill(0);
    } else {
        res[..n as usize * 4].fill(0);
    }
    r
}

/// fx_row(dst, x, y, n, layer_row) -> true written / false unchanged -- the compose hook
/// (worker threads): pixels x .. x+n-1 of row y = lerp(layer_row, result, selection
/// coverage) where the tile is rendered and shown, else unchanged
fn fx_row(c: &FxCore, d: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool {
    let mut res = [0u8; 256];
    let mut cov = [0u8; 64];
    let nb = n.max(0) as usize * 4;
    match fx_seg(c, d, x, y, n, &mut res, &mut cov) {
        SCR_ALL => dst[..nb].copy_from_slice(&res[..nb]),
        SCR_PART => gfx::straight::lerp_mask_row(&mut dst[..nb], &layer_row[..nb], &res[..nb], &cov[..nb / 4]),
        _ => return false,
    }
    true
}

/// the compose hook of the running effect (fx_row; HookKey::Fx)
pub(crate) fn fx_hook(p: &Pn) -> Option<Box<dyn CompHook + '_>> {
    if !p.fx.core.on || p.fx.core.data.is_none() {
        return None;
    }
    Some(Box::new(FxHook { c: &p.fx.core }))
}

struct FxHook<'a> {
    c: &'a FxCore,
}

impl CompHook for FxHook<'_> {
    fn row(&self, doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool {
        fx_row(self.c, doc, dst, x, y, n, layer_row)
    }
}

// ================================================================== commit

// fs_commit() -> true history entry -- write the area into the layer through fx_row
fn fs_commit(p: &mut Pn) -> bool {
    let Some(doc) = p.fx.doc else { return false };
    let li = p.fx.layer;
    let g = p.fx.core.g;
    {
        let Some(l) = doc_layer(p.doc.d(doc), li) else { return false };
        if l.sf.px.is_empty() || l.sf.w != g.w || l.sf.h != g.h {
            return false;
        }
    }
    if crate::document::history::hist_px_begin(p, doc, li) < 0 {
        return false;
    }
    if fs_touch(p, doc) != 0 {
        crate::document::history::hist_px_cancel(p, doc);
        let n = p.fx.name;
        crate::integrations::file_io::fio_error(p, Some(n), S_NOMEM);
        return false;
    }
    fs_hook_off(p, false);
    // every row of the area through fx_row, in place on the layer (worker threads)
    let mut px = core::mem::take(&mut p.doc.dm(doc).layers[li as usize].sf.px);
    {
        let d = p.doc.d(doc);
        let c = &p.fx.core;
        let stride = d.layers[li as usize].sf.stride as usize;
        let rows: Vec<SpinLock<&mut [u8]>> =
            px.chunks_mut(stride).skip(g.by0 as usize).take((g.by1 - g.by0) as usize).map(SpinLock::new).collect();
        let item = |i: usize, _: &plat::JobCtl| {
            let mut row = rows[i].lock();
            let y = g.by0 + i as i32;
            let mut x = g.bx0 & !(TILE - 1);
            while x < g.bx1 {
                let n = (g.w - x).min(TILE);
                let seg = &mut row[x as usize * 4..(x + n) as usize * 4];
                let mut res = [0u8; 256];
                let mut cov = [0u8; 64];
                let nb = n as usize * 4;
                match fx_seg(c, d, x, y, n, &mut res, &mut cov) {
                    SCR_ALL => seg.copy_from_slice(&res[..nb]),
                    SCR_PART => gfx::straight::lerp_mask_into_a(seg, &res[..nb], &cov[..n as usize]),
                    _ => {}
                }
                x += TILE;
            }
        };
        let _ = plat::parallel_for(rows.len(), &item);
    }
    p.doc.dm(doc).layers[li as usize].sf.px = px;
    crate::document::model::doc_mark_px(p, doc, li, g.bx0, g.by0, g.bx1, g.by1);
    let (n, icon) = (p.fx.name, p.fx.icon);
    let r = crate::document::history::hist_px_end(p, doc, n, icon);
    fs_remember(p);
    r
}

// fs_touch() -> 0 | ERR_NOMEM -- hist_px_touch the 64 x 64 tiles of the area that the
// selection reaches
fn fs_touch(p: &mut Pn, doc: DocId) -> i32 {
    let g = p.fx.core.g;
    let data = p.fx.core.data.clone();
    let mut y = g.by0 & !(TILE - 1);
    while y < g.by1 {
        let mut x = g.bx0 & !(TILE - 1);
        while x < g.bx1 {
            let empty = data.as_ref().and_then(|d| d.sel.as_ref()).is_some_and(|m| matches!(m.span(x, y, 1).0, gfx::tmask::Span::Empty));
            if !empty {
                let r = crate::document::history::hist_px_touch(p, doc, x.max(g.bx0), y.max(g.by0), (x + TILE).min(g.bx1), (y + TILE).min(g.by1));
                if r != 0 {
                    return r;
                }
            }
            x += TILE;
        }
        y += TILE;
    }
    0
}

// fs_remember() -- the parameters of the committed effect are remembered; it becomes the
// effect of "<Effekt> wiederholen" (Strg+F)
fn fs_remember(p: &mut Pn) {
    let f = &mut p.fx;
    let k = f.fxidx as u32 as usize;
    if k < MEM_MAX {
        f.memok[k] = true;
        f.mem[k] = f.rparam;
    }
    f.last_desc = f.desc;
    f.last_name = Some(f.name);
    rep_label(p);
}

// rep_label() -- CMD_FX_REPEAT = "<Effekt> wiederholen" (paint.net Effects.RepeatMenuItem.Format)
fn rep_label(p: &mut Pn) {
    let Some(n) = p.fx.last_name else { return };
    p.fx.rep_label = fmt_cap(format!("{} wiederholen", n), REP_CAP);
    let l = p.fx.rep_label.clone();
    pn_cmd_label(p, CMD_FX_REPEAT, &l);
}

// ================================================================== interface for the dialogs

/// fx_params_changed() -- the dialog changed FS_PARAM: cancel the render, restart after 50 ms
pub(crate) fn fx_params_changed(p: &mut Pn, ui: &mut Ui) {
    let f = &mut p.fx;
    if f.mode != FSM_DIALOG {
        return;
    }
    f.dirty = true;
    f.due = false;
    f.job.cancel();
    ui.timer(TMR_FX_DEBOUNCE, DEBOUNCE_MS);
    fx_sync_update(f);
}

/// fx_jobs_pending() -> eax 1 while the effect renders or waits for its commit (the script
/// command `sync`; tool.rs chains it into its sync hook through tool::SYNC_FX).  A debounced
/// restart is started at once (scripts do not advance the clock while they sync): fs_poll
/// after FX_SYNC_ASK.
pub(crate) fn fx_jobs_pending(p: &mut Pn) -> bool {
    fx_pending(&p.fx)
}

// fx_pending(f) -- the answer of fx_jobs_pending without its side effect
fn fx_pending(f: &Fx) -> bool {
    if f.mode == FSM_NONE {
        return false;
    }
    if f.dirty || f.phase == FSP_PREPARE || f.phase == FSP_RENDER || f.mode == FSM_APPLY {
        return true;
    }
    f.core.data.as_ref().is_some_and(|d| d.unmarked.load(SeqCst) != 0)
}

// fx_sync_update() -- tool::SYNC_FX = fx_jobs_pending
fn fx_sync_update(f: &Fx) {
    crate::tools::SYNC_FX.store(fx_pending(f), SeqCst);
}

/// fx_name(desc) -> German name of an effect (paint.net 5.1), else its key
pub(crate) fn fx_name(f: &Fx, desc: &'static FxDesc) -> &'static str {
    match map_row_of(f, desc) {
        Some(k) => map_rows().nth(k).map_or(desc.name, |r| r.2),
        None => desc.name,
    }
}

// lbl_find(desc, i) -> the fx_labels row of property i of desc
fn lbl_find(desc: &FxDesc, i: usize) -> Option<&'static (u32, &'static str, &'static str, &'static [&'static str])> {
    let key = desc.props.get(i)?.name;
    FX_LABELS.iter().find(|r| (r.0 == desc.id || r.0 == LBL_ANY) && r.1 == key)
}

/// fx_label(desc, i) -> German label of property i (its key when unknown)
pub(crate) fn fx_label(desc: &'static FxDesc, i: usize) -> &'static str {
    match lbl_find(desc, i) {
        Some(r) => r.2,
        None => desc.props.get(i).map_or("", |p| p.name),
    }
}

/// fx_choices(desc, i) -> the German choice labels of property i (its keys when unknown)
pub(crate) fn fx_choices(desc: &'static FxDesc, i: usize) -> &'static [&'static str] {
    match lbl_find(desc, i) {
        Some(r) if !r.3.is_empty() => r.3,
        _ => desc.props.get(i).map_or(&[], |p| p.choices),
    }
}

/// fx_place_dialog(&winrect, w) -- a dialog window of w DIP beside the image when there is
/// room (right, else left of it), else at the right edge of the canvas; top = canvas + 16
pub(crate) fn fx_place_dialog(p: &Pn, ui: &mut Ui, r: &Shared<WinRect>, w: i32) {
    let v = &p.view;
    let c = crate::workspace::view::view_canvas_rect(v);
    let cx0 = ui.px2dip(c[0]);
    let cy0 = ui.px2dip(c[1]);
    let cx1 = ui.px2dip(c[0].wrapping_add(c[2]));
    let il = ui.px2dip(crate::workspace::view::view_doc_to_screen(v, 0, 0).0);
    let g = p.fx.core.g;
    let ir = ui.px2dip(crate::workspace::view::view_doc_to_screen(v, g.w, g.h).0);
    // right of the image
    let mut x = ir + 16;
    if x + w + 8 > cx1 {
        // left of it
        x = il - 16 - w;
        if x < cx0 + 8 {
            // over the right part of the canvas
            x = (cx1 - w - 16).max(cx0 + 8);
        }
    }
    r.set(WinRect::new(x, cy0 + 16, w, 0, WRF_POS));
}

/// fx_progress() -> (tiles rendered, tiles, FSP_* phase)
pub(crate) fn fx_progress(f: &Fx) -> (u32, u32, u32) {
    let (done, nt) = f.core.data.as_ref().map_or((0, 0), |d| (d.ndone.load(SeqCst), d.g.nt));
    (done, nt, f.phase)
}

// ================================================================== settings (persistence hooks)

/// fx_settings_save(ini) -- section "Effekte": key = effect key, value = 512 hex digits (the
/// 32 parameter slots) for every remembered effect; "_last" = key of the last effect
pub(crate) fn fx_settings_save(f: &Fx, ini: &mut base::ini::Ini) {
    let n = ::fx::count().min(MEM_MAX);
    for i in 0..n {
        if !f.memok[i] {
            continue;
        }
        let hex = hex_encode(&f.mem[i]);
        if let Some(d) = ::fx::get(i) {
            ini.set(S_SEC_FX, d.name, &hex);
        }
    }
    if let Some(d) = f.last_desc {
        ini.set(S_SEC_FX, S_KEY_LAST, d.name);
    }
}

/// fx_settings_load(ini) -- the counterpart of fx_settings_save
pub(crate) fn fx_settings_load(f: &mut Fx, ini: &base::ini::Ini) {
    let n = ::fx::count().min(MEM_MAX);
    for i in 0..n {
        let Some(d) = ::fx::get(i) else { continue };
        let Some(v) = ini.get(S_SEC_FX, d.name) else { continue };
        if v.len() != PARAM_BYTES * 2 {
            continue;
        }
        if let Some(pr) = hex_decode(v) {
            f.mem[i] = pr;
            f.memok[i] = true;
        }
    }
    let Some(k) = ini.get(S_SEC_FX, S_KEY_LAST) else { return };
    let Some(d) = ::fx::find(k) else { return };
    f.last_desc = Some(d);
    f.last_name = Some(fx_name(f, d));
    f.rep_pending = true; // rep_label (needs Pn: the next fx_states)
}

// hex_encode(params) -- PARAM_BYTES bytes -> lower-case hex digits
fn hex_encode(pr: &Params) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(PARAM_BYTES * 2);
    for w in pr.0.iter() {
        for b in w.to_le_bytes() {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
    }
    s
}

// hex_decode(text) -> params | None (bad digit)
fn hex_decode(t: &str) -> Option<Params> {
    let b = t.as_bytes();
    let nib = |c: u8| -> Option<u8> {
        let v = c.wrapping_sub(b'0');
        if v < 10 {
            return Some(v);
        }
        let v = (v | 0x20).wrapping_sub(49);
        if v < 6 { Some(v + 10) } else { None }
    };
    let mut pr = Params::default();
    for i in 0..PARAM_BYTES {
        let v = nib(b[i * 2])? << 4 | nib(b[i * 2 + 1])?;
        pr.0[i / 8] |= (v as u64) << ((i % 8) * 8);
    }
    Some(pr)
}

// ================================================================== test hooks (diag.rs block A6)
// Commands (cmd ID ARG):
//   995 0|1|2      FNV-1a 64 of the current layer (0) / the composite (1) of the active image /
//                  the preview: the layer rows through the compose hook fx_row (2), rows of
//                  w * 4 bytes -> #9990 text "hhhhhhhhhhhhhhhh"
//   996 s|v<<8     parameter slot s of the running effect = integer v (signed 24 bit), restart
//                  as if the dialog changed it
//   997 s|v<<8     parameter slot s = f64 v / 100 (signed 24 bit), restart
//   998 on         1 = OK / immediate effects always take the asynchronous path (progress
//                  window, `sync`), 0 = small images finish at once; 2 = settings round trip:
//                  fx_settings_save into a new INI, forget everything, fx_settings_load
//   999 x|y<<16    pixel (x, y) of the effect result FS_DST -> #9990 text "AARRGGBB"
// Hidden records (every frame): 9990 text probe · 9991 value FS_MODE | FS_PHASE << 4, text
// effect name · 9992 value tiles rendered, text "done/total" · 9993 value render starts ·
// 9994 text label of CMD_FX_REPEAT, value 1 = enabled · 9995 value parameter slot 0 (int),
// text slot 0 as f64 * 100 (for decimal properties)

// hash_fnv64(data, seed) -- FNV-1a 64 (seed 0 = the offset basis)
fn fnv(d: &[u8], seed: u64) -> u64 {
    let mut h = if seed == 0 { 0xCBF2_9CE4_8422_2325 } else { seed };
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

// sf_hash(sf) -- FNV-1a 64 over the rows of a surface (w * 4 bytes each)
fn sf_hash(sf: &Surface) -> u64 {
    let mut h = 0;
    let (s, nb) = (sf.stride as usize, sf.w as usize * 4);
    for y in 0..sf.h as usize {
        h = fnv(&sf.px[y * s..y * s + nb], h);
    }
    h
}

/// fx_diag_cmd(id, arg) -> eax 1
pub(crate) fn fx_diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id - CMD_A6_TEST0 {
        0 => {
            let Some(doc) = doc_active(&p.doc) else { return true };
            let h = if arg == 2 {
                fx_preview_hash(p)
            } else if arg == 0 {
                let Some(l) = doc_cur_layer(p.doc.d(doc)) else { return true };
                if l.sf.px.is_empty() {
                    return true;
                }
                sf_hash(&l.sf)
            } else {
                let sf = crate::document::composite::comp_surface(p, doc);
                if sf.px.is_empty() {
                    return true;
                }
                sf_hash(sf)
            };
            p.fx.diag_txt = format!("{:016x}", h);
        }
        1 | 2 => {
            if p.fx.mode != FSM_DIALOG {
                return true;
            }
            let s = (arg as usize) & (FX_NSLOTS - 1);
            let v = (arg as i32) >> 8;
            p.fx.param.0[s] = if id == CMD_A6_TEST0 + 1 { v as i64 as u64 } else { (v as f64 / 100.0).to_bits() };
            fx_params_changed(p, ui);
        }
        3 => {
            if arg != 2 {
                p.fx.async_on = arg;
                return true;
            }
            let mut ini = base::ini::Ini::new();
            fx_settings_save(&p.fx, &mut ini);
            let f = &mut p.fx;
            f.memok = [false; MEM_MAX];
            f.last_desc = None;
            f.last_name = None;
            pn_cmd_label(p, CMD_FX_REPEAT, S_REP_DEFAULT);
            fx_settings_load(&mut p.fx, &ini);
            if p.fx.rep_pending {
                p.fx.rep_pending = false;
                rep_label(p);
            }
        }
        4 => {
            let f = &p.fx;
            let (x, y) = ((arg & 0xFFFF) as i32, (arg >> 16) as i32);
            if f.core.data.is_none() || x as u32 >= f.core.g.w as u32 || y as u32 >= f.core.g.h as u32 {
                return true;
            }
            let v = fx_dst_px(&f.core, x, y);
            p.fx.diag_txt = format!("{:08X}", v);
        }
        _ => {}
    }
    true
}

// fx_dst_px(x, y) -- pixel (x, y) of FS_DST: the rendered tile holding it, else 0
fn fx_dst_px(c: &FxCore, x: i32, y: i32) -> u32 {
    let g = &c.g;
    if x < g.bx0 || x >= g.bx1 || y < g.by0 || y >= g.by1 {
        return 0;
    }
    let ti = (((x as u32) >> g.tws) - g.c0 + (((y as u32) >> g.ths) - g.r0) * g.ncols) as usize;
    let t = g.tile_rect(ti as u32);
    let stride = (t[2] - t[0]) as usize * 4;
    let o = (y - t[1]) as usize * stride + (x - t[0]) as usize * 4;
    let pend;
    let buf: &[u8] = if !c.shown[ti].is_empty() {
        &c.shown[ti]
    } else {
        let Some(d) = &c.data else { return 0 };
        pend = d.pending[ti].lock().clone().unwrap_or_default();
        &pend
    };
    if o + 4 > buf.len() {
        return 0;
    }
    u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
}

// fx_preview_hash() -> FNV-1a 64 of what the preview shows for the current layer: every row
// through fx_row (64-pixel segments, as compose calls it), else the layer row
fn fx_preview_hash(p: &Pn) -> u64 {
    let Some(doc) = doc_active(&p.doc) else { return 0 };
    let d = p.doc.d(doc);
    let Some(l) = doc_cur_layer(d) else { return 0 };
    if l.sf.px.is_empty() {
        return 0;
    }
    let sf = &l.sf;
    let mut h = 0u64;
    let mut seg = [0u8; 256];
    for y in 0..sf.h {
        let row = &sf.px[y as usize * sf.stride as usize..];
        let mut x = 0;
        while x < sf.w {
            let n = (sf.w - x).min(TILE);
            let nb = n as usize * 4;
            let lr = &row[x as usize * 4..x as usize * 4 + nb];
            seg[..nb].copy_from_slice(lr);
            if p.fx.core.on {
                fx_row(&p.fx.core, d, &mut seg, x, y, n, lr);
            }
            h = fnv(&seg[..nb], h);
            x += TILE;
        }
    }
    h
}

// a6_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn a6_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

/// fx_diag_frame() -- the hidden records 9990..9995
pub(crate) fn fx_diag_frame(p: &mut Pn, ui: &mut Ui) {
    let f = &p.fx;
    a6_sem(ui, ID_A6_TEST0, &f.diag_txt, 0);
    let name = if f.mode != FSM_NONE { f.name } else { "" };
    a6_sem(ui, ID_A6_TEST0 + 1, name, (f.phase << 4 | f.mode) as i32);
    let (done, nt, _) = fx_progress(f);
    let t = fmt_cap(format!("{}/{}", done, nt), DIAG_CAP);
    a6_sem(ui, ID_A6_TEST0 + 2, &t, done as i32);
    a6_sem(ui, ID_A6_TEST0 + 3, "", f.restarts as i32);
    let on = pn_cmd_flags(ui, CMD_FX_REPEAT) & CF_DISABLED == 0;
    let rep = if f.last_name.is_some() { f.rep_label.as_str() } else { "" };
    a6_sem(ui, ID_A6_TEST0 + 4, rep, on as i32);
    let t = fmt_cap(format!("{}", rt::fx::cvtd64(f.param.f(0) * 100.0)), DIAG_CAP);
    a6_sem(ui, ID_A6_TEST0 + 5, &t, f.param.0[0] as u32 as i32);
}

/// fx_src() -- FS_SRC: the copy of the layer the running effect reads (fxui's thumbnail,
/// the Levels histogram)
pub(crate) fn fx_src(f: &Fx) -> Option<&Surface> {
    f.core.data.as_deref().map(|d| &d.src)
}

/// fx_sel() -- FS_SEL: the private copy of the selection (FXA_SEL)
pub(crate) fn fx_sel(f: &Fx) -> Option<&TMask> {
    f.core.data.as_deref().and_then(|d| d.sel.as_ref())
}
