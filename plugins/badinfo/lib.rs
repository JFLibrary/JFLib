// badinfo -- TEST plugin: a PLUGININFO whose effect list points outside the plugin image
//                                                    [apps/PaintCraft/plugins/badinfo/badinfo.s]
//
// Only for tests.  Expected: rejected ("Ungültige Plugin-Info (Zeiger außerhalb des
// Plugins).") without touching the pointer, the start continues.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

static INFO: PluginInfo = PluginInfo {
    nfx: 1,
    fx: 0x1000 as *const *const FxDesc, // not in the image
    ..PluginInfo::new(c"Kaputte Info", c"1.0")
};

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
