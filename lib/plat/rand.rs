// rand -- the operating system's cryptographic random number generator
//
//   fill_random(&mut buf)        BCryptGenRandom (system-preferred RNG) / getrandom(2) / getentropy
//                                (Apple) / arc4random_buf (Android: bionic's ChaCha20 generator,
//                                seeded by the kernel, every API level)
//   random_bytes::<N>() -> [u8; N]          random_u64()
//
// Not in the browser: there the page's crypto.getRandomValues is the source.
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
    const EINTR: isize = 4;
    let mut done = 0;
    while done < buf.len() {
        let want = (buf.len() - done).min(1 << 25);
        // SAFETY: writes at most `want` bytes into the rest of the buffer.
        let r = unsafe { rt::os::syscall3(rt::os::SYS_GETRANDOM, buf[done..].as_mut_ptr() as usize, want, 0) };
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

#[cfg(target_vendor = "apple")]
fn os_fill(buf: &mut [u8]) -> bool {
    #[link(name = "System")]
    unsafe extern "C" {
        fn getentropy(buf: *mut u8, len: usize) -> i32;
    }
    // getentropy takes at most 256 bytes per call
    buf.chunks_mut(256).all(|chunk| {
        // SAFETY: the chunk is valid for its length (at most 256 bytes).
        unsafe { getentropy(chunk.as_mut_ptr(), chunk.len()) == 0 }
    })
}

#[cfg(target_os = "android")]
fn os_fill(buf: &mut [u8]) -> bool {
    #[link(name = "c")]
    unsafe extern "C" {
        fn arc4random_buf(buf: *mut u8, len: usize);
    }
    // bionic aborts the process itself when it cannot seed the generator
    // SAFETY: the buffer is valid for buf.len() bytes.
    unsafe { arc4random_buf(buf.as_mut_ptr(), buf.len()) };
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
