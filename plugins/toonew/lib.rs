// toonew -- TEST plugin: needs PNAPI version PNAPI_VERSION + 1     [apps/PaintCraft/plugins/toonew/toonew.s]
//
// Only for tests.  Expected: rejected with "Das Plugin benötigt die Plugin-API-Version 2,
// vorhanden ist 1.", nothing registered, the start continues.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct New;

impl Effect for New {
    type Aux = ();

    fn render(_fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        Ok(())
    }
}

static FX_NEW: FxDesc = FxDesc::new::<New>(c"Zukunftseffekt", FXF_NO_DIALOG, FXC_STYLIZE, &[]);
static FX_LIST: [&FxDesc; 1] = [&FX_NEW];
static INFO: PluginInfo = PluginInfo::new(c"Zu neu", c"9.0").api_min(PNAPI_VERSION + 1).effects(&FX_LIST);

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
