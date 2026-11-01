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

    // the script command `gpu` (gpu_out.rs): a SKIP once
    pub(crate) fn sc_gpu(&mut self, _app: &mut dyn App, _a: &[u8]) {
        if !self.m.gout.skipped {
            self.m.gout.skipped = true;
            plat::write_stdout(b"SKIP gpu: no GPU output on this target\n");
        }
    }
}
