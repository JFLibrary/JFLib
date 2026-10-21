// swapchan -- example effect plugin "Kanäle tauschen" in Rust
//                                                 [apps/PaintCraft/plugins/swapchan/swapchan.s]
//
//   python build_rs.py plugin swapchan [--target win|linux] [--out build/x]
//   -> build/x/win/plugins/swapchan.dll, build/x/linux/plugins/swapchan.so; copy it into
//      <PaintCraft folder>/Effects
//
// One effect (Effekte menu, category Farbe) with one choice property: the colour channels are
// rotated per pixel (straight ARGB, alpha unchanged).  render is pure and thread safe: the
// host calls it for many ROIs at once from its worker threads.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct Swap;

impl Effect for Swap {
    type Aux = ();

    // dst = src with the channels rotated inside the ROI
    fn render(fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        let mode = fx.param_int(0); // slot 0: order 0..2
        for y in fx.y0..fx.y1 {
            if fx.cancelled() {
                return Err(ERR_CANCEL);
            }
            let (src, dst) = fx.rows(y);
            for (d, &p) in dst.iter_mut().zip(src) {
                let (a, r, g, b) = (p & 0xFF00_0000, (p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                *d = match mode {
                    1 => a | g << 16 | b << 8 | r, // R = G, G = B, B = R
                    2 => a | b << 16 | r << 8 | g, // R = B, G = R, B = G
                    _ => a | b << 16 | g << 8 | r, // R <-> B
                };
            }
        }
        Ok(())
    }
}

static PROPS: [FxProp; 1] = [FxProp::choice(
    0,
    c"Reihenfolge",
    "Rot und Blau tauschen\0Rot ← Grün ← Blau\0Rot ← Blau ← Grün\0\0".as_bytes(),
    0,
)];
static FX_SWAP: FxDesc = FxDesc::new::<Swap>(c"Kanäle tauschen", 0, FXC_COLOR, &PROPS);
static FX_LIST: [&FxDesc; 1] = [&FX_SWAP];
static INFO: PluginInfo = PluginInfo::new(c"Kanäle tauschen (Beispiel)", c"1.0")
    .author(c"jfguiLib")
    .url(c"plugins/swapchan")
    .effects(&FX_LIST);

// the host API table is not needed here
fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
