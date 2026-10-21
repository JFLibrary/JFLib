// win_lib -- plugin libraries and the crash guard: the Windows part
//                                         [src/plat/win/win_lib.s; docs/modules/PLUGIN_LOADER.md]
//
//   lib_open(path) -> Lib / Err(LIBE_*)   LoadLibraryExW(DONT_RESOLVE_DLL_REFERENCES): the
//        image is mapped and relocated, but DllMain / TLS callbacks never run and no import
//        is bound -- a plugin with an import, delay-import or TLS directory is rejected
//        (LIBE_DEPS / LIBE_INIT); only x86-64 DLLs (LIBE_FORMAT).  Hard-error dialogs are
//        suppressed (SetThreadErrorMode).  Segments: headers + every section.
//   lib_sym(h, name) -> address | 0       own export-directory lookup (bounds checked against
//        SizeOfImage, every table / name inside a readable section -- a section without
//        MEM_READ is PAGE_NOACCESS; forwarders -> 0, so no other DLL is ever loaded)
//   (crate) lib_unload(h)                 FreeLibrary (lib_close / drop)
//   (crate) plat_tid() -> u64             thread id (TEB ClientId, gs:[0x48])
//   (crate) guard_thread()                nothing on Windows
//   (crate) guard_os_install()            AddVectoredExceptionHandler(first, guard_veh)
//
// guard_veh handles access violations, in-page errors, illegal / privileged instructions,
// integer and FP traps, misalignment, breakpoints and stack overflow -- only while the
// faulting thread has an active guard (plib.rs); everything else goes on to the next
// handler (win_crash.rs).

use alloc::vec::Vec;

use crate::{
    GK_ACCESS, GK_ARITH, GK_BREAK, GK_BUS, GK_ILLEGAL, GK_STACK, LH_SEGMAX, LIBA_R, LIBA_W, LIBA_X, LIBE_DEPS,
    LIBE_FORMAT, LIBE_INIT, LIBE_IO, LIBE_NOMEM, Lib, LibSeg, guard_fault_ip, guard_hit, lib_seg,
};

const DONT_RESOLVE_REFS: u32 = 1; // LoadLibraryExW flag
const SEM_QUIET: u32 = 0x8003; // SEM_FAILCRITICALERRORS | NOGPFAULTERRORBOX | NOOPENFILEERRORBOX
const PATHW_CAP: usize = 1024; // wchars
const IMG_SPAN_MAX: usize = 256 << 20;
// PE offsets
const PE_LFANEW: usize = 0x3C;
const OH_SIZEIMAGE: usize = 56;
const OH_SIZEHDRS: usize = 60;
const OH_NRVA: usize = 108;
const OH_DIR: usize = 112; // data directories (8 bytes each)
const DIR_EXPORT: usize = 0;
const DIR_IMPORT: usize = 1;
const DIR_TLS: usize = 9;
const DIR_DELAY: usize = 13;
const SCN_EXEC: u32 = 0x2000_0000;
const SCN_READ: u32 = 0x4000_0000;
const SCN_WRITE: u32 = 0x8000_0000;
// exception codes
const EXC_ACCESS: u32 = 0xC000_0005;
const EXC_INPAGE: u32 = 0xC000_0006;
const EXC_ILLEGAL: u32 = 0xC000_001D;
const EXC_PRIV: u32 = 0xC000_0096;
const EXC_FLT_FIRST: u32 = 0xC000_008C; // array bounds .. flt underflow
const EXC_FLT_LAST: u32 = 0xC000_0095; // .. int divide by zero, int overflow
const EXC_STACK: u32 = 0xC000_00FD;
const EXC_MISALIGN: u32 = 0x8000_0002;
const EXC_BREAK: u32 = 0x8000_0003;
// CONTEXT (x64)
const CTX_EFLAGS: usize = 0x44;
const CTX_RSP: usize = 0x98;
const CTX_RBP: usize = 0xA0;
const CTX_RIP: usize = 0xF8;

#[repr(C)]
struct ExceptionPointers {
    record: *const u8,
    context: *mut u8,
}

// (the same functions as win_crash.rs / win_api.rs declare, with this module's types)
#[allow(clashing_extern_declarations)]
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(first: u32, h: extern "system" fn(*const ExceptionPointers) -> i32) -> usize;
    fn LoadLibraryExW(name: *const u16, file: isize, flags: u32) -> isize;
    fn FreeLibrary(m: isize) -> i32;
    fn SetThreadErrorMode(mode: u32, old: *mut u32) -> i32;
    fn GetLastError() -> u32;
}

/// the id of the calling thread (TEB ClientId.UniqueThread)  [plat_tid]
pub(crate) fn plat_tid() -> u64 {
    let t: u64;
    // SAFETY: gs points to this thread's TEB on x64 Windows.
    unsafe { core::arch::asm!("mov {}, gs:[0x48]", out(reg) t, options(nostack, readonly, preserves_flags)) };
    t
}

/// per-thread guard setup (none on Windows)  [plat_guard_thread]
pub(crate) fn guard_thread() {}

/// the vectored handler, called before win_crash.rs' handlers  [guard_os_install]
pub(crate) fn guard_os_install() {
    // SAFETY: a valid handler function for the whole process lifetime.
    unsafe { AddVectoredExceptionHandler(1, guard_veh) };
}

/// -1 continue at the guard's fault address / 0 not ours  [guard_veh]
extern "system" fn guard_veh(ep: *const ExceptionPointers) -> i32 {
    // SAFETY: the system passes valid exception pointers; the record / context are read and
    // the context written at their documented x64 offsets.
    unsafe {
        let rec = (*ep).record;
        let ctx = (*ep).context;
        let code = *(rec as *const u32);
        let kind = match code {
            EXC_ACCESS | EXC_INPAGE => GK_ACCESS,
            EXC_ILLEGAL | EXC_PRIV => GK_ILLEGAL,
            EXC_STACK => GK_STACK,
            EXC_MISALIGN => GK_BUS,
            EXC_BREAK => GK_BREAK,
            EXC_FLT_FIRST..=EXC_FLT_LAST => GK_ARITH,
            _ => return 0, // EXCEPTION_CONTINUE_SEARCH
        };
        let pc = *(rec.add(0x10) as *const u64); // ExceptionAddress
        let nparams = *(rec.add(0x18) as *const u32);
        let addr = if nparams >= 2 && kind == GK_ACCESS { *(rec.add(0x28) as *const u64) } else { 0 };
        let Some((rsp, rbp)) = guard_hit(plat_tid(), kind, code, pc, addr) else { return 0 };
        *(ctx.add(CTX_RIP) as *mut u64) = guard_fault_ip() as u64;
        *(ctx.add(CTX_RSP) as *mut u64) = rsp;
        *(ctx.add(CTX_RBP) as *mut u64) = rbp;
        *(ctx.add(CTX_EFLAGS) as *mut u32) &= !0x400; // DF = 0
    }
    -1 // EXCEPTION_CONTINUE_EXECUTION
}

// ---------------------------------------------------------------- the loader

// reads of the mapped image (the headers page is always readable; everything else is
// checked against the segments before)
unsafe fn r16(a: usize) -> u32 {
    unsafe { (a as *const u16).read_unaligned() as u32 }
}

unsafe fn r32(a: usize) -> u32 {
    unsafe { (a as *const u32).read_unaligned() }
}

/// open a plugin DLL (see the file header)  [plat_lib_open]
pub fn lib_open(path: &str) -> Result<Lib, u32> {
    if !crate::policy::plugin_allowed(path) {
        return Err(LIBE_IO);
    }
    // UTF-16, '/' -> '\' (LoadLibrary needs backslashes)
    let mut w: Vec<u16> = Vec::new();
    let mut buf = [0u16; 2];
    for c in path.chars() {
        if c == '\0' {
            break;
        }
        let u = c.encode_utf16(&mut buf);
        if w.len() + u.len() > PATHW_CAP - 1 {
            break;
        }
        w.extend_from_slice(u);
    }
    if w.is_empty() {
        return Err(LIBE_IO);
    }
    for c in w.iter_mut() {
        if *c == b'/' as u16 {
            *c = b'\\' as u16;
        }
    }
    w.push(0);
    let mut old = 0u32;
    // SAFETY: a NUL-terminated wide path; error mode set and put back around the load.
    let h = unsafe {
        SetThreadErrorMode(SEM_QUIET, &mut old);
        let h = LoadLibraryExW(w.as_ptr(), 0, DONT_RESOLVE_REFS);
        let err = if h == 0 { GetLastError() } else { 0 };
        SetThreadErrorMode(old, core::ptr::null_mut());
        if h == 0 {
            return Err(match err {
                2 | 3 | 5 => LIBE_IO, // file / path not found, access denied
                8 | 14 => LIBE_NOMEM,
                _ => LIBE_FORMAT,
            });
        }
        h
    };
    let mut lib = Lib::empty();
    lib.base = h as usize;
    lib.os = h as usize; // from here on dropping lib frees the module
    // SAFETY: h is a mapped image; pe_check reads only its headers page.
    match unsafe { pe_check(&mut lib) } {
        0 => Ok(lib),
        e => Err(e),
    }
}

/// the mapped image: x86-64 DLL (PE32+), no imports / delay imports / TLS; fills the
/// segments (headers, then the sections) -> 0 / LIBE_*  [pe_check]
unsafe fn pe_check(l: &mut Lib) -> u32 {
    let b = l.base;
    unsafe {
        if r16(b) != 0x5A4D {
            return LIBE_FORMAT; // "MZ"
        }
        let lfanew = r32(b + PE_LFANEW) as usize;
        if !(64..=4096 - 24 - 240).contains(&lfanew) {
            return LIBE_FORMAT;
        }
        let nt = b + lfanew;
        if r32(nt) != 0x0000_4550 || r16(nt + 4) != 0x8664 || r16(nt + 22) & 0x2000 == 0 {
            return LIBE_FORMAT; // "PE\0\0", AMD64, IMAGE_FILE_DLL
        }
        let nsec = r16(nt + 6) as usize;
        let ohsize = r16(nt + 20) as usize;
        if ohsize < OH_DIR + 16 * 8 {
            return LIBE_FORMAT;
        }
        let oh = nt + 24;
        if r16(oh) != 0x20B {
            return LIBE_FORMAT;
        }
        let span = r32(oh + OH_SIZEIMAGE) as usize;
        if !(4096..=IMG_SPAN_MAX).contains(&span) {
            return LIBE_FORMAT;
        }
        l.span = span;
        if r32(oh + OH_NRVA) < 14 {
            return LIBE_FORMAT;
        }
        if r32(oh + OH_DIR + DIR_IMPORT * 8 + 4) != 0 || r32(oh + OH_DIR + DIR_DELAY * 8 + 4) != 0 {
            return LIBE_DEPS;
        }
        if r32(oh + OH_DIR + DIR_TLS * 8 + 4) != 0 {
            return LIBE_INIT;
        }
        l.symtab = r32(oh + OH_DIR + DIR_EXPORT * 8) as usize; // export directory RVA / size
        l.strsz = r32(oh + OH_DIR + DIR_EXPORT * 8 + 4) as usize;
        // the section table inside the headers
        let hdrs = r32(oh + OH_SIZEHDRS) as usize;
        if hdrs > span {
            return LIBE_FORMAT;
        }
        let sec0 = oh + ohsize;
        let end = sec0 - b + nsec * 40;
        if end > hdrs || end > 4096 || nsec > LH_SEGMAX - 1 {
            return LIBE_FORMAT;
        }
        // segment 0: the headers (read only)
        l.segs[0] = LibSeg { start: b, end: b + hdrs, flags: LIBA_R };
        l.nseg = 1;
        for k in 0..nsec {
            let s = sec0 + k * 40;
            let va = r32(s + 12) as usize;
            let mut vs = r32(s + 8) as usize; // VirtualSize
            if vs == 0 {
                vs = r32(s + 16) as usize; // SizeOfRawData
            }
            if va >= span {
                return LIBE_FORMAT;
            }
            let e = (va + vs).min(span);
            let ch = r32(s + 36);
            let mut fl = 0;
            if ch & SCN_READ != 0 {
                fl |= LIBA_R;
            }
            if ch & SCN_WRITE != 0 {
                fl |= LIBA_W;
            }
            if ch & SCN_EXEC != 0 {
                fl |= LIBA_X;
            }
            l.segs[l.nseg as usize] = LibSeg { start: b + va, end: b + e, flags: fl };
            l.nseg += 1;
        }
    }
    0
}

/// [addr, addr + len) lies in one readable segment of h  [sy_rd]
fn sy_rd(h: &Lib, addr: usize, len: usize) -> bool {
    let (end, fl) = lib_seg(h, addr);
    end != 0 && fl & LIBA_R != 0 && len <= end - addr
}

/// the address of the export name, 0 when missing / forwarded  [plat_lib_sym]
pub fn lib_sym(h: &Lib, name: &str) -> usize {
    let (b, span) = (h.base, h.span);
    let (edir, esize) = (h.symtab, h.strsz);
    let name = name.as_bytes();
    let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
    if edir == 0 || edir + 40 > span || !sy_rd(h, b + edir, 40) {
        return 0;
    }
    let ed = b + edir;
    // SAFETY: every read below lies in a readable segment of the mapped image (sy_rd /
    // lib_seg checked) or in the export directory checked above.
    unsafe {
        let nnames = r32(ed + 24) as usize;
        if nnames > 65536 {
            return 0;
        }
        let names = b + r32(ed + 32) as usize;
        if !sy_rd(h, names, nnames * 4) {
            return 0;
        }
        let ords = b + r32(ed + 36) as usize;
        if !sy_rd(h, ords, nnames * 2) {
            return 0;
        }
        let mut hit = None;
        'names: for i in 0..nnames {
            let rva = r32(names + i * 4) as usize;
            if rva >= span {
                continue;
            }
            let p = b + rva; // export name: readable up to its segment end
            let (end, fl) = lib_seg(h, p);
            if end == 0 || fl & LIBA_R == 0 {
                continue;
            }
            let mut k = 0;
            loop {
                if p + k >= end {
                    continue 'names;
                }
                let c = *((p + k) as *const u8);
                let want = name.get(k).copied().unwrap_or(0);
                if c != want {
                    continue 'names;
                }
                if c == 0 {
                    hit = Some(i);
                    break 'names;
                }
                k += 1;
            }
        }
        let Some(i) = hit else { return 0 };
        let ord = r16(ords + i * 2) as usize; // ordinal index
        if ord >= r32(ed + 20) as usize {
            return 0; // NumberOfFunctions
        }
        let funcs = r32(ed + 28) as usize;
        if funcs + ord * 4 + 4 > span {
            return 0;
        }
        let fp = b + funcs + ord * 4;
        if !sy_rd(h, fp, 4) {
            return 0;
        }
        let frva = r32(fp) as usize;
        if frva >= span || frva.wrapping_sub(edir) < esize {
            return 0; // outside, or inside the export directory: a forwarder
        }
        b + frva
    }
}

/// FreeLibrary (lib_close, drop)  [plat_lib_close]
pub(crate) fn lib_unload(h: &mut Lib) {
    if h.os != 0 {
        // SAFETY: the module handle LoadLibraryExW returned, freed once.
        unsafe { FreeLibrary(h.os as isize) };
        h.os = 0;
    }
}
