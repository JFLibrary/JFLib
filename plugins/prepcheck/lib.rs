// prepcheck -- TEST plugin (no assembly counterpart): an adjustment whose prepare step returns an
// aux, no aux or an error, for the two-register return of FXD_PREPARE (rax = aux | 0, edx = the
// error; lib/plugin/fx.rs prepare_tramp).  Only for plugins/tests/prepcheck.uis.
//
// "Ergebnis" 0: aux = 0x00FFFFFF, render inverts r, g, b; 1: no aux (Ok(None)), render copies;
// 2: prepare fails with ERR_LIMIT, the host reports it and keeps the layer.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct Prep;

impl Effect for Prep {
    type Aux = u32;
    const PREPARE: bool = true;

    fn prepare(fx: &Fx<'_>) -> Result<Option<u32>, i32> {
        match fx.param_int(0) {
            1 => Ok(None),
            2 => Err(ERR_LIMIT),
            _ => Ok(Some(0x00FF_FFFF)),
        }
    }

    fn render(fx: &mut Fx<'_>, aux: Option<&u32>) -> Result<(), i32> {
        let m = aux.copied().unwrap_or(0);
        for y in fx.y0..fx.y1 {
            let (src, dst) = fx.rows(y);
            for (d, &p) in dst.iter_mut().zip(src) {
                *d = p ^ m;
            }
        }
        Ok(())
    }
}

static PROPS: [FxProp; 1] = [FxProp::choice(0, c"Ergebnis", "aux\0kein aux\0Fehler\0\0".as_bytes(), 0)];
static FX_PREP: FxDesc = FxDesc::new::<Prep>(c"prepare-Test", FXF_ADJ, FXC_ADJUST, &PROPS);
static FX_LIST: [&FxDesc; 1] = [&FX_PREP];
static INFO: PluginInfo = PluginInfo::new(c"prepare-Test", c"0.1").author(c"jfguiLib Tests").effects(&FX_LIST);

// keeps the API table (the aux block comes from PA_ALLOC)
fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
