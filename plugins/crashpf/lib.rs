// crashpf -- TEST plugin: an adjustment whose render runs PNAPI parallel_for with 16 items;
// item 11 divides by zero on some worker thread        [apps/PaintCraft/plugins/crashpf/crashpf.s]
//
// Only for tests (never copy it into a real Effects folder).  Expected: the item's guard
// catches the fault, the remaining items are cancelled, the plugin is disabled, render returns
// ERR_FAULT.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

struct CrashPf;

impl Effect for CrashPf {
    type Aux = ();

    // parallel_for(item, 16) -> its result
    fn render(_fx: &mut Fx<'_>, _: Option<&()>) -> Result<(), i32> {
        parallel_for(16, &|i, _job| {
            if i == 11 {
                divide_by_zero();
            }
        })
    }
}

// an integer division by zero (#DE) -- in assembly: Rust's `/` checks for zero and panics
#[allow(unsafe_code)]
fn divide_by_zero() {
    // SAFETY: none -- this test plugin faults on purpose; the host's guard catches it.
    unsafe {
        core::arch::asm!(
            "xor ecx, ecx",
            "mov eax, 1",
            "xor edx, edx",
            "div ecx",
            out("eax") _, out("ecx") _, out("edx") _,
            options(nostack, nomem),
        )
    };
}

static FX_CRASH: FxDesc =
    FxDesc::new::<CrashPf>(c"Absturz-Test (parallel)", FXF_ADJ | FXF_NO_DIALOG, FXC_ADJUST, &[]);
static FX_LIST: [&FxDesc; 1] = [&FX_CRASH];
static INFO: PluginInfo =
    PluginInfo::new(c"Absturz-Test parallel_for", c"0.1").author(c"jfguiLib Tests").effects(&FX_LIST);

// keeps the API table (parallel_for needs it)
fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
