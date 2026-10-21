// crashq -- TEST plugin: pn_plugin_query executes an illegal instruction (ud2)
//                                                      [apps/PaintCraft/plugins/crashq/crashq.s]
//
// Only for tests.  Expected: the start continues, the plugin is listed as crashed in
// pn_plugin_query and unloaded.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    plugin::trap()
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
