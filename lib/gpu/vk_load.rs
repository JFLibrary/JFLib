// vk_load -- the Vulkan loader library, opened at run time
//
//   Lib::open(name) -> Result<Lib>     Windows: LoadLibraryExW (name "vulkan-1.dll" from
//                                      System32 only; a path from jfgui_GPU_DLL from its own
//                                      folder), then vkGetInstanceProcAddr
//   lib.gipa(instance, name) -> Pfn    vkGetInstanceProcAddr
//
// The program does not import vulkan-1.dll: it starts on machines without Vulkan and gets
// GpuError::NoLoader there.  plat's plugin loader (win_lib.rs) maps DLLs without resolving
// their imports, which the Vulkan loader needs -- this is a normal LoadLibrary.  Error dialogs
// for missing DLLs are suppressed for the call (SetThreadErrorMode).  Other targets: NoLoader
// until their loaders exist (Linux / Android: libvulkan.so through a dynamic linker).

use crate::vk_ffi as vk;
use crate::{GpuError, Result};

type Gipa = unsafe extern "system" fn(vk::Instance, *const u8) -> vk::Pfn;

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Lib {
    #[cfg(windows)]
    module: isize,
    gipa: Gipa,
}

// SAFETY: the module handle and the function pointer are process-global and immutable.
unsafe impl Send for Lib {}
// SAFETY: as above.
unsafe impl Sync for Lib {}

impl Lib {
    pub fn gipa(&self, instance: vk::Instance, name: &[u8]) -> vk::Pfn {
        debug_assert!(name.last() == Some(&0));
        // SAFETY: name is NUL-terminated; instance is null or a live instance of this loader.
        unsafe { (self.gipa)(instance, name.as_ptr()) }
    }
}

#[cfg(windows)]
mod win {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn LoadLibraryExW(name: *const u16, file: isize, flags: u32) -> isize;
        pub fn GetProcAddress(module: isize, name: *const u8) -> usize;
        pub fn FreeLibrary(module: isize) -> i32;
        pub fn SetThreadErrorMode(mode: u32, old: *mut u32) -> i32;
    }
    pub const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
    pub const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    pub const SEM_QUIET: u32 = 0x8003; // SEM_FAILCRITICALERRORS | NOGPFAULTERRORBOX | NOOPENFILEERRORBOX
}

#[cfg(windows)]
impl Lib {
    pub fn open(name: &str) -> Result<Lib> {
        use alloc::vec::Vec;
        let path = name.contains('\\') || name.contains('/');
        let wide: Vec<u16> = name.encode_utf16().chain(core::iter::once(0)).collect();
        let flags = if path {
            win::LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | win::LOAD_LIBRARY_SEARCH_SYSTEM32
        } else {
            win::LOAD_LIBRARY_SEARCH_SYSTEM32
        };
        let mut old = 0u32;
        // SAFETY: plain Win32 calls; wide is NUL-terminated.
        let module = unsafe {
            win::SetThreadErrorMode(win::SEM_QUIET, &mut old);
            let m = win::LoadLibraryExW(wide.as_ptr(), 0, flags);
            win::SetThreadErrorMode(old, core::ptr::null_mut());
            m
        };
        if module == 0 {
            return Err(GpuError::NoLoader);
        }
        // SAFETY: a module we just loaded; the name is NUL-terminated.
        let p = unsafe { win::GetProcAddress(module, b"vkGetInstanceProcAddr\0".as_ptr()) };
        if p == 0 {
            // SAFETY: the module from above, unused.
            unsafe { win::FreeLibrary(module) };
            return Err(GpuError::NoLoader);
        }
        // SAFETY: vkGetInstanceProcAddr has this prototype (vulkan_core.h).
        let gipa = unsafe { core::mem::transmute::<usize, Gipa>(p) };
        Ok(Lib { module, gipa })
    }
}

#[cfg(windows)]
impl Drop for Lib {
    fn drop(&mut self) {
        // SAFETY: the instance that used the loader is destroyed before (InstShared field order).
        unsafe { win::FreeLibrary(self.module) };
    }
}

#[cfg(not(windows))]
impl Lib {
    pub fn open(_name: &str) -> Result<Lib> {
        Err(GpuError::NoLoader)
    }
}
