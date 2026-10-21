// pshapes -- example shape plugin in Rust             [apps/PaintCraft/plugins/pshapes/pshapes.s]
//
//   python build_rs.py plugin pshapes  -> build/<target>/plugins/pshapes.dll / .so
//                                            (Shapes folder)
//
// Two custom shapes for the Formen tool, given as path data (the Geometry syntax of paint.net's
// ShapeKit XAML files, the same the XAML loader of <exe>/Shapes reads).  No code besides
// pn_plugin_query.

#![no_std]
#![deny(unsafe_code)]

use plugin::*;

static SHAPES: [ShapeDesc; 2] = [
    ShapeDesc::new(c"Plugin-Stern", c"F1 M 50,0 L 61,35 98,35 68,57 79,91 50,70 21,91 32,57 2,35 39,35 Z"),
    ShapeDesc::new(c"Plugin-Haus", c"M 50,0 L 100,45 88,45 88,100 12,100 12,45 0,45 Z M 40,60 L 60,60 60,100 40,100 Z"),
];
static INFO: PluginInfo = PluginInfo::new(c"Beispielformen", c"1.0")
    .author(c"jfguiLib")
    .url(c"plugins/pshapes")
    .shapes(&SHAPES);

fn query(_api: &'static PnApi) -> Option<&'static PluginInfo> {
    Some(&INFO)
}

plugin::entry!(query);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    plugin::panic(info)
}
