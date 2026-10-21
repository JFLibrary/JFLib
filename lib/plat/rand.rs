// rand -- the operating system's cryptographic random number generator
//
//   fill_random(&mut buf)        BCryptGenRandom (system-preferred RNG) / getrandom(2)
//   random_bytes::<N>() -> [u8; N]          random_u64()
//
// There is no fallback: when the system RNG fails the process stops (a key made from weak
// randomness is worse than no key).

#[cfg(windows)]
fn os_fill(buf: &mut [u8]) -> bool {
    #[link(name = "bcrypt", kind = "raw-dylib")]
    unsafe extern "system" {
        fn BCryptGenRandom(alg: usize, buf: *mut u8, len: u32, flags: u32) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    for chunk in buf.chunks_mut(1 << 30) {
        // SAFETY: the buffer is valid for chunk.len() bytes.
        let st = unsafe { BCryptGenRandom(0, chunk.as_mut_ptr(), chunk.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if st != 0 {
            return false;
        }
    }
    true
}

#[cfg(target_os = "linux")]
fn os_fill(buf: &mut [u8]) -> bool {
    const SYS_GETRANDOM: usize = 318;
    const EINTR: isize = 4;
    let mut done = 0;
    while done < buf.len() {
        let want = (buf.len() - done).min(1 << 25);
        // SAFETY: writes at most `want` bytes into the rest of the buffer.
        let r = unsafe { rt::os::syscall3(SYS_GETRANDOM, buf[done..].as_mut_ptr() as usize, want, 0) };
        if r == -EINTR {
            continue;
        }
        if r <= 0 {
            return false;
        }
        done += r as usize;
    }
    true
}

/// fill a buffer with cryptographically secure random bytes (stops the process on failure)
pub fn fill_random(buf: &mut [u8]) {
    if !os_fill(buf) {
        crate::write_stderr(b"fatal: the system random number generator failed\n");
        crate::exit(70);
    }
}

/// N random bytes
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    fill_random(&mut b);
    b
}

/// a random 64-bit number
pub fn random_u64() -> u64 {
    u64::from_le_bytes(random_bytes::<8>())
}
