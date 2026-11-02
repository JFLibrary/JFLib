// gpu_out_off -- the UI's GPU output on targets without it (gpu_out.rs: Windows only for now):
// every frame takes the platform's path, no Vulkan code is linked

use super::{App, GpuOutput, Ui};

#[derive(Default)]
pub(crate) struct S {
    skipped: bool,
}

impl Ui {
    pub(crate) fn gp_window_mode(&mut self) {}
    pub(crate) fn gp_present(&mut self, _cur: u32, _pw: i32, _n: i32) -> bool {
        false
    }
    pub(crate) fn gp_cpu_done(&mut self, _cur: u32, _pw: i32) {}
    pub(crate) fn gp_split_wanted(&self) -> bool {
        false
    }
    pub(crate) fn gp_plan_images(&mut self) -> bool {
        false
    }
    pub(crate) fn gp_plan_canvases(&mut self) -> bool {
        false
    }
    pub(crate) fn gp_canvas_wanted(&self) -> bool {
        false
    }
    pub(crate) fn gp_canvas_steady(&self, _id: u32, _ver: u32) -> bool {
        false
    }
    pub(crate) fn gp_canvas_begin(&mut self) {}
    pub(crate) fn gp_canvas_damage(&mut self, _id: u32, _r: [f64; 4]) {}
    pub(crate) fn gp_split_begin(&mut self, _n: i32) -> bool {
        false
    }
    pub(crate) fn gp_split_draw(&mut self, _n: i32) {}
    pub(crate) fn gp_frame_done(&mut self, _t0: u64, _n: i32, _split: bool) {}
    pub(crate) fn gp_settle_all(&mut self) {}
    pub(crate) fn gp_repaired(&mut self) {}
    pub(crate) fn gp_tick(&mut self) {}
    pub(crate) fn gp_deadline(&self) -> Option<u64> {
        None
    }
    pub(crate) fn gp_window_gone(&mut self, _i: u32) {}
    pub(crate) fn gp_shutdown(&mut self) {}
    pub(crate) fn gp_scripted(&self) -> bool {
        false
    }
    pub(crate) fn gp_script_pw(&mut self, _cur: u32) -> i32 {
        -1
    }

    /// what window `win` (a UI window index, 0 = main) shows through: here the platform's path
    pub fn gpu_output(&self, _win: u32) -> GpuOutput {
        GpuOutput { never: true, ..GpuOutput::default() }
    }

    /// (test aid) on Windows: the device behaves as lost from now on
    pub fn gpu_test_lose(&mut self) {}

    /// on Windows: the pixels of the GPU's images into the framebuffer
    pub fn gpu_settle(&mut self) {}

    /// (test aid) on Windows: the bytes image textures may take
    pub fn gpu_test_image_budget(&mut self, _bytes: u64) {}

    /// (test aid) on Windows: the bytes canvas tiles may take
    pub fn gpu_test_canvas_budget(&mut self, _bytes: u64) {}

    /// (test aid) on Windows: the tiles go this long (ms) after the last move
    pub fn gpu_test_canvas_unused(&mut self, _ms: u64) {}

    /// on Windows: every canvas whose view comes from tiles goes back to the CPU's picture now
    pub fn gpu_canvas_settle(&mut self) {}

    // the script command `gpu` (gpu_out.rs): a SKIP once
    pub(crate) fn sc_gpu(&mut self, _app: &mut dyn App, _a: &[u8]) {
        if !self.m.gout.skipped {
            self.m.gout.skipped = true;
            plat::write_stdout(b"SKIP gpu: no GPU output on this target\n");
        }
    }
}
