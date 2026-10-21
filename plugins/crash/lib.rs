// crash -- TEST plugin: an effect whose render writes to address 0     [apps/PaintCraft/plugins/crash/crash.s]
//
// Only for tools/plugin_check.py / apps/PaintCraft/tests/plugins*.uis -- never copy it into a
// real Effects folder.  Expected: the host catches the access violation, disables the plugin,
// lists it under Einstellungen > Plugin-Fehler and keeps running.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct Crash;

impl Effect for Crash {
    type Aux = ();

    // an access violation at address 0 (in assembly: a null write in Rust would be undefined
    // behaviour, not a dependable fault)
    #[allow(unsafe_code)]
    fn render(_fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        // SAFETY: none -- this test plugin faults on purpose; the host's guard catches it.
        unsafe { core::arch::asm!("mov dword ptr [{0}], 0x0BADF00D", in(reg) 0usize, options(nostack)) };
        Ok(())
    }
}

static FX_CRASH: FxDesc = FxDesc::new::<Crash>(c"Absturz-Test", FXF_NO_DIALOG, FXC_STYLIZE, &[]);
static FX_LIST: [&FxDesc; 1] = [&FX_CRASH];
static INFO: PluginInfo = PluginInfo::new(c"Absturz-Test", c"0.1").author(c"jfguiLib Tests").effects(&FX_LIST);

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
