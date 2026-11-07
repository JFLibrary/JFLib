// canvas -- a canvas whose zoom and pan go through GPU textures (G3c; docs/GPU.md "Leinwände")
//
// The CPU stays the truth: a canvas records its Cmd::Custom as ever (ui.canvas, ui.cmd_custom), its
// Draw paints what the view shows.  Right after it the widget tells the library what that drawing
// is made of:
//
//   ui.canvas_gpu_wanted() -> bool     is it worth building a CanvasDesc?  (Never on other targets,
//                                      without a GPU output, in headless runs.)
//   ui.canvas_gpu(CanvasDesc { .. })   the custom command recorded last shows this content through
//                                      this view
//   ui.canvas_gpu_damage(id, r)        the content rectangle r (x0, y0, x1, y1, content units)
//                                      changed: its tiles are drawn again when they are needed
//
// While the view moves and the content stays, the content comes from tiles (TILE x TILE px GPU
// textures, drawn once by CanvasContent::draw_view, kept under a budget) that the render pass draws
// with the transform: no raster, no upload of the view.  A short time after the last change the
// view is drawn once more by the CPU: at rest the picture is byte for byte the CPU path's.  An edit
// goes the CPU's way as without a GPU and costs no tile.
//
// Everywhere else, and with CanvasGpu::Cpu, ui.canvas_gpu does nothing and the Draw paints every
// frame; the widget's code stays the same on every target.

use alloc::rc::Rc;
use alloc::vec::Vec;

use crate::cmd::{Cmd, Gfx};
use crate::Ui;

/// how a canvas may use the GPU while its view moves
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[non_exhaustive]
pub enum CanvasGpu {
    /// the CPU path in every frame, as without a GPU
    Cpu,
    /// moves by whole pixels at the same scale come from the tiles; another scale is drawn by the
    /// CPU at once.  Pixel-exact when the content's raster does not depend on its place
    Pan,
    /// also zoom: the tiles are scaled while the gesture runs (a little soft or coarse), the CPU
    /// draws the picture sharp a moment after it
    #[default]
    Zoom,
}

/// the view of a canvas: the content point (u, v) shows at the window pixel position
/// (ox + u * scale, oy + v * scale)
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct CanvasView {
    pub scale: f64,
    pub ox: f64,
    pub oy: f64,
}

/// what a canvas shows, drawable through any view (at render time)
pub trait CanvasContent {
    /// draw the content through view v into g (origin 0, the clip says what needs pixels, rect = the
    /// target's rectangle): what the canvas's Draw paints there at this view, without what lies
    /// over it, every pixel opaque
    fn draw_view(&self, g: &mut Gfx, v: CanvasView, rect: [i32; 4]);
}

/// a canvas for ui.canvas_gpu
pub struct CanvasDesc {
    /// its identity in the window (the tiles live as long as it is drawn; ui.canvas_gpu_damage)
    pub id: u32,
    /// its name for jfgui_GPU_CANVAS_CPU / jfgui_GPU_CANVAS_PAN ("image", "diagram", ...)
    pub name: &'static str,
    /// the window rectangle x, y, w, h the content fills, opaque (inside the custom command's box)
    pub rect: [i32; 4],
    pub view: CanvasView,
    /// stands for everything the content shows: another value drops its tiles
    pub ver: u32,
    pub content: Rc<dyn CanvasContent>,
    pub gpu: CanvasGpu,
    /// the raster does not depend on the place: a move by whole pixels must match byte for byte
    /// (jfgui_GPU_VERIFY fails on a difference; else it is counted, GpuOutput::canvas_diff_*)
    pub exact_pan: bool,
}

/// a canvas of the frame: its command
#[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))] // (read by gpu_out.rs)
pub(crate) struct Rec {
    pub(crate) layer: u8,
    pub(crate) index: u32,
    pub(crate) d: CanvasDesc,
}

/// the canvases recorded in the live window's frame
#[derive(Default)]
pub(crate) struct W {
    #[cfg_attr(not(any(windows, jfgui_glibc, target_os = "android", target_arch = "wasm32")), allow(dead_code))] // (read by gpu_out.rs)
    pub(crate) recs: Vec<Rec>,
}

impl Ui {
    /// is the GPU output there to use a CanvasDesc?  A widget without one saves building it
    pub fn canvas_gpu_wanted(&self) -> bool {
        self.gp_canvas_wanted()
    }

    /// the custom command recorded last shows this content through this view (module header)
    pub fn canvas_gpu(&mut self, d: CanvasDesc) {
        if d.gpu == CanvasGpu::Cpu || !self.gp_canvas_wanted() {
            return;
        }
        let l = self.wp.cmd.layer as usize;
        let list = &self.wp.cmd.layers[l];
        if let Some(Cmd::Custom { f: Some(_), .. }) = list.last() {
            let index = list.len() as u32 - 1;
            self.wp.canv.recs.push(Rec { layer: l as u8, index, d });
        }
    }

    /// the content rectangle x0, y0, x1, y1 (content units) of canvas id changed: its tiles go
    pub fn canvas_gpu_damage(&mut self, id: u32, r: [f64; 4]) {
        if self.gp_canvas_wanted() {
            self.gp_canvas_damage(id, r);
        }
    }
}
