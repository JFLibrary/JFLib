// Bounded control-channel IO primitives. Only explicitly enabled callers read stdin.

/// Read bytes from the process standard input. Zero denotes clean EOF.
/// Diagnostics use `write_stderr`; protocol responses use `write_stdout`.
#[cfg(windows)]
pub fn read_stdin(bytes: &mut [u8]) -> Result<usize, u32> {
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> isize;
        fn ReadFile(handle: isize, buffer: *mut u8, count: u32, read: *mut u32, overlapped: *mut crate::win::Overlapped) -> i32;
        fn GetLastError() -> u32;
    }
    let mut read = 0;
    // SAFETY: buffer is writable for count bytes, the synchronous call cannot outlive it.
    unsafe {
        let handle = GetStdHandle(-10i32 as u32);
        if ReadFile(handle, bytes.as_mut_ptr(), bytes.len().min(u32::MAX as usize) as u32, &mut read, core::ptr::null_mut()) != 0 {
            Ok(read as usize)
        } else {
            let error = GetLastError();
            if error == 109 { Ok(0) } else { Err(error) }
        }
    }
}

/// Read bytes from the process standard input, retrying interrupted system calls.
#[cfg(target_os = "linux")]
pub fn read_stdin(bytes: &mut [u8]) -> Result<usize, u32> {
    loop {
        // SAFETY: fd 0, a live writable slice, read cannot exceed the passed length.
        let count = unsafe { rt::os::syscall3(0, 0, bytes.as_mut_ptr() as usize, bytes.len()) };
        if count == -4 { continue; }
        return if count < 0 { Err((-count) as u32) } else { Ok(count as usize) };
    }
}
