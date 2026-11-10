// mtl_swap -- swapchains on Metal (AP6c, docs/APPLE.md): a CAMetalLayer on a plat window.  Not built
// yet: Surface::new says Unsupported, so the types below are never made; they keep the API whole.

use crate::mtl_dev::{Device, Instance};
use crate::*;

/// a window as a target for swapchains
#[derive(Clone)]
pub struct Surface {
    _window: i32,
}

impl Surface {
    /// (Linux: the window's size for a rebuild on a worker)
    pub fn track_size(&self) {}

    /// the surface of plat window `window`
    pub fn new(_instance: &Instance, window: i32) -> Result<Surface> {
        let _ = window;
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
}

/// a frame being drawn
pub struct Frame {
    size: (u32, u32),
}

/// what Swapchain::try_acquire found
#[must_use]
pub enum TryAcquire {
    Frame(Frame),
    NotReady,
    Stale,
    NoArea,
}

impl Frame {
    pub fn index(&self) -> u32 {
        0
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// images shown in a window
pub struct Swapchain {
    texture: Texture,
}

impl Device {
    pub fn create_swapchain(&self, _surface: &Surface, _desc: &SwapchainDesc) -> Result<Swapchain> {
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
}

impl Swapchain {
    pub fn rebuild(&mut self) -> Result<()> {
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
    pub fn try_acquire(&mut self) -> Result<TryAcquire> {
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
    pub fn acquire(&mut self) -> Result<Option<Frame>> {
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
    pub fn view(&self, _frame: &Frame) -> &TextureView {
        self.texture.view()
    }
    pub fn texture(&self, _frame: &Frame) -> &Texture {
        &self.texture
    }
    pub fn present<'a, I: IntoIterator<Item = CommandBuffer<'a>>>(&self, _frame: Frame, _cmds: I) -> Result<SubmitId> {
        Err(GpuError::Unsupported("window surfaces on Metal (AP6c)"))
    }
    pub fn prepare(&mut self) -> Result<bool> {
        Ok(true)
    }
    pub fn flush(&self) -> Result<bool> {
        Ok(true)
    }
    pub fn busy(&self) -> bool {
        false
    }
    pub fn next_look(&self, retry: u64) -> u64 {
        retry
    }
    pub fn damage(&self, _rects: Option<&[[i32; 4]]>) {}
    pub fn resize(&mut self) {}
    pub fn set_present_mode(&mut self, _m: PresentMode) {}
    pub fn present_mode(&self) -> PresentMode {
        PresentMode::Fifo
    }
    pub fn format(&self) -> Format {
        self.texture.format()
    }
    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
    pub fn image_count(&self) -> usize {
        1
    }
}
