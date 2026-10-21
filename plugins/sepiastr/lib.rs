// sepiastr -- example adjustment plugin "Sepia-Stärke" in Rust
//                                                 [apps/PaintCraft/plugins/sepiastr/sepiastr.s]
//
//   python build_rs.py plugin sepiastr  -> build/<target>/plugins/sepiastr.dll / .so
//                                             (copy it into <PaintCraft folder>/Effects)
//
// One adjustment (Korrekturen menu, FXF_ADJ) with an integer property "Stärke" 0..100 %:
// dst = lerp(src, sepia(src), strength); sepia tone t = (77 r + 150 g + 29 b + 128) >> 8,
// colour (t + 40, t + 20, t - 20) clamped; alpha unchanged.  Integer math only: the result is
// the same on every OS, for every ROI split, and the same as the assembly plugin's.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct Sepia;

impl Effect for Sepia {
    type Aux = ();

    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        // slot 0: strength 0..100 -> w = strength * 256 / 100 (rounded), 0..256
        let s = fx.param_int(0).clamp(0, 100) as u32;
        let w = (s * 256 + 50) / 100;
        for y in fx.y0..fx.y1 {
            if fx.cancelled() {
                return Err(ERR_CANCEL);
            }
            let (src, dst) = fx.rows(y);
            for (d, &p) in dst.iter_mut().zip(src) {
                *d = sepia(p, w);
            }
        }
        Ok(())
    }
}

// sepia(p, w) -- one straight ARGB pixel
fn sepia(p: u32, w: u32) -> u32 {
    let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
    let t = (77 * r + 150 * g + 29 * b + 128) >> 8;
    let rr = mix(r, (t + 40).min(255), w);
    let gg = mix(g, (t + 20).min(255), w);
    let bb = mix(b, t.saturating_sub(20), w);
    (p & 0xFF00_0000) | rr << 16 | gg << 8 | bb
}

// mix(src, tone, w) = (src * (256 - w) + tone * w + 128) >> 8
fn mix(src: u32, tone: u32, w: u32) -> u32 {
    (src * (256 - w) + tone * w + 128) >> 8
}

static PROPS: [FxProp; 1] = [FxProp::int(0, c"Stärke", 0.0, 100.0, 60.0)];
static FX_SEPIA: FxDesc = FxDesc::new::<Sepia>(c"Sepia-Stärke", FXF_ADJ, FXC_ADJUST, &PROPS);
static FX_LIST: [&FxDesc; 1] = [&FX_SEPIA];
static INFO: PluginInfo = PluginInfo::new(c"Sepia-Stärke (Beispiel)", c"1.0")
    .author(c"jfguiLib")
    .url(c"plugins/sepiastr")
    .effects(&FX_LIST);

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
