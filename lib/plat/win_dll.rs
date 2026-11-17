// win_dll -- functions of Windows DLLs that are no KnownDLLs, loaded from System32 on first use
//
//   system_dll! { "dwmapi.dll" { fn DwmSetWindowAttribute(h: isize, attr: u32, v: *const u32, size: u32) -> i32 = E_FAIL; } }
//
// A program's static imports are looked up next to the program first: a dwmapi.dll, winmm.dll,
// crypt32.dll ... beside a downloaded setup.exe would be loaded into it (DLL planting).  Only the
// KnownDLLs (kernel32, user32, gdi32, advapi32, ole32, shell32, ws2_32, imm32, combase ...) always
// come from System32, so they stay raw-dylib imports.  Each function the macro makes has the
// signature of the import it replaces (calls stay as they were): the first call loads the DLL
// with LOAD_LIBRARY_SEARCH_SYSTEM32 and keeps the address; without the DLL or the export it
// returns the value after `=`.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExA(name: *const u8, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> usize;
}

/// the address of `name` in the System32 DLL `dll` (both NUL-terminated), 0 when there is none
pub(crate) fn proc_of(dll: &[u8], name: &[u8]) -> usize {
    // SAFETY: NUL-terminated names; LOAD_LIBRARY_SEARCH_SYSTEM32; both calls return 0 on failure.
    unsafe {
        let m = LoadLibraryExA(dll.as_ptr(), 0, 0x800);
        if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
    }
}

/// HRESULT E_FAIL, for the functions that return one
pub(crate) const E_FAIL: i32 = 0x8000_4005u32 as i32;

macro_rules! system_dll {
    ($dll:literal { $( $vis:vis fn $name:ident($($arg:ident: $t:ty),* $(,)?) $(-> $ret:ty)? = $fail:expr; )* }) => {
        $(
            #[allow(non_snake_case, clippy::too_many_arguments)]
            $vis unsafe fn $name($($arg: $t),*) $(-> $ret)? {
                static P: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
                let mut p = P.load(core::sync::atomic::Ordering::Relaxed);
                if p == 0 {
                    p = $crate::win_dll::proc_of(concat!($dll, "\0").as_bytes(), concat!(stringify!($name), "\0").as_bytes());
                    if p == 0 {
                        return $fail;
                    }
                    P.store(p, core::sync::atomic::Ordering::Relaxed);
                }
                // SAFETY: the export of this name in that system DLL has this signature.
                let f: unsafe extern "system" fn($($t),*) $(-> $ret)? = unsafe { core::mem::transmute(p) };
                unsafe { f($($arg),*) }
            }
        )*
    };
}
pub(crate) use system_dll;
