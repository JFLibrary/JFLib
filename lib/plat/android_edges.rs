// android_edges -- the colours at the app's edges for Android's system bars (form.rs)
//
// The UI hands over the colours along the main window's top, bottom, left and right edge after
// every present of it (host_edges); GalleryActivity reads them each frame (bridge.c nativeEdges ->
// jfgui_edge) and paints the status bar, the navigation bar and a cut-out's strip in them, with
// light or dark system icons to match: the bars follow the app, not the system's appearance (an app
// may keep its own theme).  iOS does the same in its host (apple_ios.rs edge_color).

use core::sync::atomic::{AtomicU32, Ordering};

// 0xFFRRGGBB once a frame was there; 0 = none yet (the host keeps its first colours)
static EDGES: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

/// the colours (0xRRGGBB) along the main window's top, bottom, left and right edge
pub fn host_edges(colors: [u32; 4]) {
    for (e, c) in EDGES.iter().zip(colors) {
        e.store(c | 0xFF00_0000, Ordering::Relaxed);
    }
}

/// (bridge.c) the colour of edge `side` (0 top, 1 bottom, 2 left, 3 right): 0xFFRRGGBB, 0 = none yet
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_edge(side: u32) -> u32 {
    EDGES.get(side as usize).map_or(0, |e| e.load(Ordering::Relaxed))
}
