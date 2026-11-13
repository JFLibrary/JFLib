// win_crash -- crash handling on Windows               [src/plat/win/plat_win.s crash_veh, crash_handler]
//
// A vectored handler takes fatal exceptions (code >= 0xC0000000) raised inside our image;
// the unhandled-exception filter takes everything nobody handled.  Both fill a CrashInfo,
// print "jfgui: crash 0x<code> at image offset 0x<offset> (look it up in the .map file)" and
// end in rt::crash::report (the app's hook, exit code 3).  Exceptions of system DLLs go on to
// their own handlers (print dialogs and the shell raise and catch some themselves).
// CrashInfo::frames: the call chain, exact from the PE unwind data (RtlVirtualUnwind,
// crash-check); the stack scan of crash_fill (plat_win.s, plat-win) when that finds nothing
// and for a stack overflow.  ARM64 (docs/ARM64.md, ARM5): its CONTEXT, unwind data and calls at the end.

use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use rt::crash::{CK_ACCESS, CK_BUS, CK_DIVIDE, CK_ILLEGAL, CK_OTHER, CK_STACK, CrashInfo, StackText};

#[repr(C)]
struct ExceptionPointers {
    record: *const u8,
    context: *const u8,
}

type Handler = extern "system" fn(*const ExceptionPointers) -> i32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AddVectoredExceptionHandler(first: u32, h: Handler) -> usize;
    fn SetUnhandledExceptionFilter(f: Handler) -> usize;
    fn GetModuleHandleW(name: *const u16) -> isize;
}

static IMG_BASE: AtomicUsize = AtomicUsize::new(0);
static IMG_SIZE: AtomicUsize = AtomicUsize::new(0);
static HANDLER_TID: AtomicUsize = AtomicUsize::new(0); // the thread in handler (crash-check)

#[cfg(target_arch = "x86_64")] const CTX_FP: usize = 0xA0; // rbp; CTX_REGS: rax rbx rcx rdx rsi rdi rbp rsp r8 .. r15 rip (CrashInfo::regs order)
#[cfg(target_arch = "x86_64")] const CTX_REGS: [usize; 17] =
    [0x78, 0x90, 0x80, 0x88, 0xA8, 0xB0, 0xA0, 0x98, 0xB8, 0xC0, 0xC8, 0xD0, 0xD8, 0xE0, 0xE8, 0xF0, 0xF8];

/// our image's range, the handlers (process start)
pub(crate) fn crash_init() {
    // SAFETY: our own module handle; the PE headers are mapped with the image.
    unsafe {
        let base = GetModuleHandleW(core::ptr::null()) as usize;
        if base != 0 {
            let nt = base + (*((base + 0x3C) as *const u32)) as usize;
            IMG_SIZE.store(*((nt + 0x50) as *const u32) as usize, Ordering::SeqCst); // SizeOfImage
            IMG_BASE.store(base, Ordering::SeqCst);
        }
        AddVectoredExceptionHandler(1, veh);
        SetUnhandledExceptionFilter(handler);
    }
}

extern "system" fn veh(ep: *const ExceptionPointers) -> i32 {
    // SAFETY: the system passes valid exception pointers.
    let (code, at) = unsafe {
        let rec = (*ep).record;
        (*(rec as *const u32), *(rec.add(16) as *const usize))
    };
    if code >= 0xC000_0000 && at.wrapping_sub(IMG_BASE.load(Ordering::SeqCst)) < IMG_SIZE.load(Ordering::SeqCst) {
        handler(ep);
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

extern "system" fn handler(ep: *const ExceptionPointers) -> i32 {
    if rt::crash::busy() {
        rt::os::exit(3); // a crash in the hook / a second thread
    }
    // crash-check: a fault inside this handler on the same thread (filling the CrashInfo,
    // before report marks the crash busy) ends the process instead of entering it again
    let tid = super::wlib::plat_tid() as usize;
    if HANDLER_TID.swap(tid, Ordering::SeqCst) == tid {
        rt::os::exit(3);
    }
    let mut ci = CrashInfo::empty();
    // SAFETY: valid exception record and context from the system.
    unsafe { fill(&mut ci, (*ep).record, (*ep).context) };
    let mut line = StackText::<160>::new();
    if let Some((name, base)) = crate::module_at(ci.ip) {
        // jxl-module: inside the loaded JPEG XL writer module
        let _ = writeln!(line, "jfgui: crash 0x{:X} in {name} at offset 0x{:x} (look it up in its .map file)", ci.code,
                         ci.ip.wrapping_sub(base));
    } else {
        let _ = writeln!(line, "jfgui: crash 0x{:X} at image offset 0x{:x} (look it up in the .map file)", ci.code,
                         ci.ip.wrapping_sub(ci.base));
    }
    rt::os::write_stderr_crash(line.bytes());
    rt::crash::report(&ci)
}

/// CrashInfo from the exception record and the context
unsafe fn fill(ci: &mut CrashInfo, rec: *const u8, ctx: *const u8) {
    unsafe {
        let code = *(rec as *const u32);
        ci.code = code;
        ci.kind = match code {
            0xC000_0005 => CK_ACCESS,
            0xC000_0006 | 0x8000_0002 => CK_BUS, // in-page error, misalignment
            0xC000_001D | 0xC000_0096 => CK_ILLEGAL,
            0xC000_0094 | 0xC000_0095 => CK_DIVIDE,
            0xC000_00FD => CK_STACK,
            _ => CK_OTHER,
        };
        if (ci.kind == CK_ACCESS || ci.kind == CK_BUS) && *(rec.add(24) as *const u32) >= 2 {
            ci.access = *(rec.add(32) as *const u64) as u32; // 0 read, 1 write, 8 execute (DEP)
            ci.addr = *(rec.add(40) as *const u64);
        }
        for (i, &off) in CTX_REGS.iter().enumerate() {
            ci.regs[i] = *(ctx.add(off) as *const u64);
        }
        ci.ip = *(ctx.add(CTX_PC) as *const u64);
        // the executable sections of our image
        let base = IMG_BASE.load(Ordering::SeqCst);
        ci.base = base as u64;
        ci.code0 = u64::MAX;
        ci.code1 = 0;
        if base != 0 {
            let nt = base + (*((base + 0x3C) as *const u32)) as usize;
            let nsec = *((nt + 6) as *const u16) as usize;
            let opt = *((nt + 20) as *const u16) as usize;
            let mut sh = nt + 24 + opt;
            for _ in 0..nsec {
                if *((sh + 36) as *const u32) & 0x2000_0000 != 0 {
                    // IMAGE_SCN_MEM_EXECUTE
                    let start = (base + *((sh + 12) as *const u32) as usize) as u64;
                    let end = start + *((sh + 8) as *const u32) as u64;
                    ci.code0 = ci.code0.min(start);
                    ci.code1 = ci.code1.max(end);
                }
                sh += 40;
            }
        }
        // the call chain: exact from the PE unwind data (crash-check), else the stack scan
        // (plat-win; also for a stack overflow, where the unwinder would need more stack)
        if ci.kind == CK_STACK || !unwind(ci, ctx) {
            frames(ci, *(ctx.add(CTX_FP) as *const u64), *(ctx.add(CTX_SP) as *const u64));
        }
    }
}

// ---------------------------------------------------------------- exact call chain (crash-check)
//
// RtlVirtualUnwind steps from the fault context through the frames the PE unwind data
// (.pdata / .xdata, which rustc emits for every function with a stack frame) describes; a pc
// without unwind data (naked functions such as crash_test, global_asm stubs) is linked to its
// caller by the first word above rsp (at most 1 KB; [rsp] for a leaf) that is the return
// address of a call into that code (leaf_link).  Each step's return address in our code
// becomes a frame; frames of system DLLs (a fault inside kernel32, a window procedure called
// by user32) are unwound but not listed.  No allocation, no frame pointers needed.
//
// The unwinder trusts the stack, so every step is checked first: rsp and the frame base
// (rsp, or the frame register - 16 * offset) lie in this thread's stack (TEB StackLimit ..
// StackBase) together with every slot the unwind codes read; leaf_link reads only inside the
// stack.  A corrupted stack ends the chain instead of faulting inside the crash handler.

#[cfg(target_arch = "x86_64")] const CONTEXT_SIZE: usize = 0x4D0;
#[cfg(target_arch = "x86_64")] const CTX_SP: usize = 0x98; // rsp
#[cfg(target_arch = "x86_64")] const CTX_PC: usize = 0xF8; // rip
#[cfg(target_arch = "x86_64")] const CTX_GPR: usize = 0x78; // rax; unwind register n is at CTX_GPR + 8 n (rax rcx rdx rbx rsp ...)
const UNWIND_STEPS: usize = 64; // frames stepped through at most (ours and system DLLs')
const LEAF_WORDS: u64 = 128; // code without unwind data: its return address is searched in 1 KB
// CONTEXT offsets of the callee-saved registers rbx rbp rsi rdi r12 .. r15
#[cfg(target_arch = "x86_64")] const CTX_NONVOL: [usize; 8] = [0x90, 0xA0, 0xA8, 0xB0, 0xD8, 0xE0, 0xE8, 0xF0];
#[cfg(target_arch = "x86_64")] const UNW_FLAG_CHAININFO: u8 = 4;

#[repr(C, align(16))]
struct Context([u8; CONTEXT_SIZE]);

impl Context {
    fn get(&self, off: usize) -> u64 {
        u64::from_le_bytes(self.0[off..off + 8].try_into().unwrap_or([0; 8]))
    }
    fn set(&mut self, off: usize, v: u64) {
        self.0[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

#[repr(C)]
#[cfg(target_arch = "x86_64")] struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlLookupFunctionEntry(pc: u64, base: *mut u64, history: *mut u8) -> *const RuntimeFunction;
    fn RtlVirtualUnwind(typ: u32, base: u64, pc: u64, fe: *const RuntimeFunction, ctx: *mut Context,
                        handler_data: *mut usize, establisher: *mut u64, ctx_ptrs: *mut u8) -> usize;
}

/// this thread's stack: StackLimit .. StackBase of the TEB (committed, readable)
#[cfg(target_arch = "x86_64")] fn stack_bounds() -> (u64, u64) {
    let (lo, hi): (u64, u64);
    // SAFETY: gs points at this thread's TEB on x64 Windows; two reads.
    unsafe {
        core::arch::asm!("mov {0}, gs:[0x10]", "mov {1}, gs:[0x08]", out(reg) lo, out(reg) hi,
                         options(nostack, readonly, preserves_flags));
    }
    (lo, hi)
}

/// the frame base RtlVirtualUnwind will use and the bytes above it that its unwind codes
/// read (pushes, allocations, saves, machine frames, the return address), over the chained
/// infos; None for unwind data it does not know
#[cfg(target_arch = "x86_64")] unsafe fn frame_extent(base: u64, fe: *const RuntimeFunction, pc: u64, ctx: &Context) -> Option<(u64, u64)> {
    unsafe {
        let mut fe = fe;
        let mut frame = ctx.get(CTX_SP);
        let mut ext = 0u64; // pushes + allocations so far
        let mut need = 0u64; // highest byte read above the frame base
        for depth in 0..8 {
            let ui = (base + (*fe).unwind as u64) as *const u8;
            let ver = *ui & 7;
            let flags = *ui >> 3;
            let n = *ui.add(2) as usize;
            let (freg, foff) = (*ui.add(3) & 15, (*ui.add(3) >> 4) as u64);
            if ver != 1 && ver != 2 {
                return None;
            }
            let code = |i: usize| -> (u8, u8, u8) { (*ui.add(4 + 2 * i), *ui.add(5 + 2 * i) & 15, *ui.add(5 + 2 * i) >> 4) };
            let slot = |i: usize| -> u64 { (ui.add(4 + 2 * i) as *const u16).read_unaligned() as u64 };
            // the primary function: pc past its prolog (or past the frame register's set-up)
            let poff = pc.wrapping_sub(base + (*fe).begin as u64);
            let mut i = 0;
            while i < n {
                let (at, op, info) = code(i);
                let applies = depth > 0 || poff >= at as u64;
                match op {
                    0 => ext += 8, // UWOP_PUSH_NONVOL
                    1 => {
                        // UWOP_ALLOC_LARGE
                        ext += if info == 0 { slot(i + 1) * 8 } else { slot(i + 1) | slot(i + 2) << 16 };
                        i += if info == 0 { 1 } else { 2 };
                    }
                    2 => ext += info as u64 * 8 + 8, // UWOP_ALLOC_SMALL
                    3 => {
                        // UWOP_SET_FPREG
                        if freg != 0 && applies {
                            frame = ctx.get(CTX_GPR + 8 * freg as usize).wrapping_sub(16 * foff);
                        }
                    }
                    4 | 5 => {
                        // UWOP_SAVE_NONVOL(_FAR)
                        let off = if op == 4 { slot(i + 1) * 8 } else { slot(i + 1) | slot(i + 2) << 16 };
                        need = need.max(off + 8);
                        i += if op == 4 { 1 } else { 2 };
                    }
                    6 => i += 1, // UWOP_EPILOG (version 2): no stack access
                    8 | 9 => {
                        // UWOP_SAVE_XMM128(_FAR)
                        let off = if op == 8 { slot(i + 1) * 16 } else { slot(i + 1) | slot(i + 2) << 16 };
                        need = need.max(off + 16);
                        i += if op == 8 { 1 } else { 2 };
                    }
                    10 => need = need.max(ext + 48), // UWOP_PUSH_MACHFRAME (+ error code)
                    _ => return None,
                }
                i += 1;
            }
            if flags & UNW_FLAG_CHAININFO == 0 {
                return Some((frame, need.max(ext + 8)));
            }
            fe = ui.add(4 + 2 * (n + (n & 1))) as *const RuntimeFunction;
        }
        None
    }
}

/// the caller of code without unwind data at pc: (stack slot, return address) of the first
/// word from sp up (LEAF_WORDS at most, inside the stack) that is the return address of a call
/// into that code -- a direct call (E8) must target at most 64 KB before pc where no unwind
/// data covers the target, an indirect call cannot be checked; a return address into another
/// module (code with unwind data) counts only at [rsp] (a leaf called by a system DLL)
#[cfg(target_arch = "x86_64")] unsafe fn leaf_link(ci: &CrashInfo, pc: u64, first: bool, sp: u64, hi: u64, img: u64, img_end: u64) -> Option<(u64, u64)> {
    let at = if first { pc } else { pc.wrapping_sub(1) };
    for k in 0..LEAF_WORDS {
        let a = sp + 8 * k;
        if a + 8 > hi {
            break;
        }
        // SAFETY: an aligned read inside the stack (sp is 8-aligned, a + 8 <= hi).
        let v = unsafe { *(a as *const u64) };
        let mut b = 0u64;
        if v >= img && v < img_end {
            // SAFETY: is_return checks the code range before reading code bytes.
            if !unsafe { is_return(ci, v) } {
                continue;
            }
            // SAFETY: v - 5 .. v lies inside our code (is_return).
            if unsafe { *((v - 5) as *const u8) } != 0xE8 {
                return Some((a, v)); // an indirect call
            }
            // SAFETY: as above.
            let t = v.wrapping_add(unsafe { ((v - 4) as *const i32).read_unaligned() } as i64 as u64);
            // SAFETY: a lookup only.
            if t <= at && at - t < 0x10000 && unsafe { RtlLookupFunctionEntry(t, &mut b, core::ptr::null_mut()) }.is_null() {
                return Some((a, v));
            }
        } else if k == 0 && !unsafe { RtlLookupFunctionEntry(v, &mut b, core::ptr::null_mut()) }.is_null() {
            return Some((a, v));
        }
    }
    None
}

/// CrashInfo::frames from the PE unwind data; false when no frame was found (the caller
/// scans the stack instead)
#[cfg(target_arch = "x86_64")] unsafe fn unwind(ci: &mut CrashInfo, fault: *const u8) -> bool {
    let (lo, hi) = stack_bounds();
    let mut ctx = Context([0; CONTEXT_SIZE]);
    // SAFETY: the system's CONTEXT record of the fault (CONTEXT_SIZE bytes).
    unsafe { core::ptr::copy_nonoverlapping(fault, ctx.0.as_mut_ptr(), CONTEXT_SIZE) };
    let img = IMG_BASE.load(Ordering::SeqCst) as u64;
    let img_end = img + IMG_SIZE.load(Ordering::SeqCst) as u64;
    ci.nframes = 0;
    for step in 0..UNWIND_STEPS {
        let pc = ctx.get(CTX_PC);
        let sp = ctx.get(CTX_SP);
        if sp & 7 != 0 || sp < lo || sp >= hi {
            break;
        }
        let mut base = 0u64;
        // SAFETY: a lookup in the loaded modules' function tables.
        let fe = unsafe { RtlLookupFunctionEntry(pc, &mut base, core::ptr::null_mut()) };
        if fe.is_null() {
            // code without unwind data: linked to its caller by leaf_link; after a stub with a
            // frame of its own the callee-saved registers are not known (0: a frame register
            // based on them fails the checks below)
            // SAFETY: reads inside lo .. hi (sp checked above).
            let Some((a, ret)) = (unsafe { leaf_link(ci, pc, step == 0, sp, hi, img, img_end) }) else { break };
            if a != sp {
                for off in CTX_NONVOL {
                    ctx.set(off, 0);
                }
            }
            ctx.set(CTX_PC, ret);
            ctx.set(CTX_SP, a + 8);
        } else {
            // SAFETY: fe / base from RtlLookupFunctionEntry: the module's unwind data.
            let Some((frame, need)) = (unsafe { frame_extent(base, fe, pc, &ctx) }) else { break };
            if frame & 7 != 0 || frame < lo || frame > hi || need > hi - frame {
                break; // the codes would read outside the stack
            }
            let (mut hd, mut ef) = (0usize, 0u64);
            // SAFETY: every slot the unwind codes read lies in the stack (checked above).
            unsafe { RtlVirtualUnwind(0, base, pc, fe, &mut ctx, &mut hd, &mut ef, core::ptr::null_mut()) };
        }
        let ret = ctx.get(CTX_PC);
        if ret == 0 || ctx.get(CTX_SP) <= sp {
            break; // the end of the chain (thread start), or no progress
        }
        if ret >= ci.code0 && ret < ci.code1 {
            add_frame(ci, ret);
            if ci.nframes as usize == ci.frames.len() {
                break;
            }
        }
    }
    ci.nframes > 0
}

// ---------------------------------------------------------------- call chain (plat-win)

#[repr(C)]
struct MemInfo {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    region: usize,
    state: u32,
    protect: u32,
    typ: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualQuery(addr: usize, mi: *mut MemInfo, n: usize) -> usize;
}

/// append a return address (ignored when full)  [add_frame]
fn add_frame(ci: &mut CrashInfo, v: u64) {
    let n = ci.nframes as usize;
    if n < ci.frames.len() {
        ci.frames[n] = v;
        ci.nframes += 1;
    }
}

/// v lies in the code range right after a call instruction (E8 rel32; FF /2 with a register,
/// [reg], [reg + disp8], [reg + disp32] or [rip + disp32] operand, with or without SIB / REX)
/// [is_return]
#[cfg(target_arch = "x86_64")] unsafe fn is_return(ci: &CrashInfo, v: u64) -> bool {
    if v < ci.code0.wrapping_add(8) || v >= ci.code1 {
        return false;
    }
    // SAFETY: v - 7 .. v lies inside our image's code (checked above).
    let b = |k: u64| unsafe { *((v - k) as *const u8) };
    if b(5) == 0xE8 {
        return true; // call rel32
    }
    if b(2) == 0xFF && (b(1) & 0xF8 == 0xD0 || b(1) & 0xF8 == 0x10) {
        return true; // call reg: FF D0+r; call [reg]: FF 10+r (not 14 / 15)
    }
    if b(3) == 0xFF && b(2) & 0xF8 == 0x50 {
        return true; // call [reg + disp8]: FF 50+r d8
    }
    if b(6) == 0xFF && (b(5) == 0x15 || b(5) & 0xF8 == 0x90) {
        return true; // call [rip + disp32] / [reg + disp32]
    }
    if b(7) == 0xFF && b(6) == 0x94 {
        return true; // call [reg + disp32] with SIB: FF 94 sib d32
    }
    b(4) == 0xFF && b(3) == 0x54 // call [reg + disp8] with SIB: FF 54 sib d8
}

/// v is a return address whose call rel32 (E8) jumps to at most 64 KB before the crash
/// address: the call of the function that crashed -- preferred over stale return addresses in
/// its frame  [leaf_call]
#[cfg(target_arch = "x86_64")] unsafe fn leaf_call(ci: &CrashInfo, v: u64) -> bool {
    // SAFETY: called after is_return(v): v - 5 .. v lies inside our code.
    unsafe {
        if *((v - 5) as *const u8) != 0xE8 {
            return false;
        }
        let target = v.wrapping_add(((v - 4) as *const i32).read_unaligned() as i64 as u64);
        target <= ci.ip && ci.ip - target < 0x10000
    }
}

/// CrashInfo::frames from the stack (inside the committed region holding rsp, at most 16 KB):
/// frames linking rbp -> [rbp] = the caller's rbp, [rbp + 8] = the return address; the first
/// return address below rbp (a leaf function's caller) comes first.  Without a chain of two
/// frames every return address on the stack is listed instead (may hold stale ones).
/// [crash_fill, the call chain]
unsafe fn frames(ci: &mut CrashInfo, rbp: u64, rsp: u64) {
    ci.nframes = 0;
    let lo = rsp & !7;
    // SAFETY: MEMORY_BASIC_INFORMATION is plain data.
    let mut mi: MemInfo = unsafe { core::mem::zeroed() };
    // the end of the committed region holding rsp (the stack above it), at most 16 KB
    // SAFETY: a query only.
    let hi = if unsafe { VirtualQuery(lo as usize, &mut mi, core::mem::size_of::<MemInfo>()) } == 0 {
        lo // nothing to scan
    } else {
        ((mi.base + mi.region) as u64).min(lo + 16384)
    };
    // SAFETY: every read below is a u64 at an 8-aligned address in lo .. hi, the committed stack.
    let at = |p: u64| unsafe { *(p as *const u64) };
    if hi > lo {
        // the leaf candidate: a call of the crashed function first, else the first return address
        let end = rbp.min(hi);
        let mut found = None;
        let mut p = lo;
        while p < end {
            // SAFETY: is_return / leaf_call only read inside our code.
            if unsafe { is_return(ci, at(p)) && leaf_call(ci, at(p)) } {
                found = Some(at(p));
                break;
            }
            p += 8;
        }
        if found.is_none() {
            p = lo;
            while p < end {
                // SAFETY: as above.
                if unsafe { is_return(ci, at(p)) } {
                    found = Some(at(p));
                    break;
                }
                p += 8;
            }
        }
        if let Some(v) = found {
            ci.frames[0] = v;
            ci.nframes = 1;
        }
        let mut f = rbp;
        while f & 7 == 0 && f >= lo && f.wrapping_add(16) <= hi && (ci.nframes as usize) < ci.frames.len() {
            let ret = at(f + 8);
            // SAFETY: as above.
            if !unsafe { is_return(ci, ret) } {
                break;
            }
            add_frame(ci, ret);
            let next = at(f);
            if next <= f {
                break;
            }
            f = next;
        }
        if ci.nframes >= 3 {
            return; // leaf + two linked frames: a real chain
        }
    }
    ci.nframes = 0;
    let mut p = lo;
    while p < hi && (ci.nframes as usize) < ci.frames.len() {
        // SAFETY: as above.
        if unsafe { is_return(ci, at(p)) } {
            add_frame(ci, at(p));
        }
        p += 8;
    }
}

// ---------------------------------------------------------------- ARM64 (docs/ARM64.md, ARM5)
//
// The same steps with the CONTEXT of ARM64 (x0 .. x28 at 8 + 8 n, Fp = x29, Lr = x30, Sp, Pc; 0x390 bytes)
// and its unwind data: a .pdata entry is 8 bytes, the start and either the RVA of an .xdata record or,
// marked in its low two bits, packed unwind data; the codes are those of Microsoft's ARM64 exception
// handling.  CrashInfo::regs holds x0 .. x15 and pc (as on Linux and Apple), the TEB is in x18.  Code
// without unwind data is linked to its caller by lr at the fault (a leaf such as crash_test or a call
// through a bad pointer), further up by the first word above sp that is the return address of a bl / blr
// into it (leaf_link).  As on x64 every step is checked first: unwind_reads_ok follows the codes from each
// place the unwinder may start at and wants every slot they read inside this thread's stack.

#[cfg(target_arch = "aarch64")] const CTX_REGS: [usize; 17] =
    [0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50, 0x58, 0x60, 0x68, 0x70, 0x78, 0x80, 0x108];
#[cfg(target_arch = "aarch64")] const CONTEXT_SIZE: usize = 0x390;
#[cfg(target_arch = "aarch64")] const CTX_X19: usize = 0xA0; // x19 .. x28 at CTX_X19 + 8 n
#[cfg(target_arch = "aarch64")] const CTX_FP: usize = 0xF0;
#[cfg(target_arch = "aarch64")] const CTX_LR: usize = 0xF8;
#[cfg(target_arch = "aarch64")] const CTX_SP: usize = 0x100;
#[cfg(target_arch = "aarch64")] const CTX_PC: usize = 0x108;
#[cfg(target_arch = "aarch64")] const CODE_BYTES: usize = 31 * 4; // an .xdata record's codes at most (Code Words < 32)

#[repr(C)]
#[cfg(target_arch = "aarch64")]
struct RuntimeFunction {
    begin: u32,
    unwind: u32, // .xdata RVA, or packed unwind data when the low two bits are 1 or 2
}

/// this thread's stack: StackLimit .. StackBase of the TEB (committed, readable)
#[cfg(target_arch = "aarch64")]
fn stack_bounds() -> (u64, u64) {
    let (lo, hi): (u64, u64);
    // SAFETY: x18 points at this thread's TEB on ARM64 Windows (only read); two reads.
    unsafe {
        core::arch::asm!("ldr {0}, [x18, #0x10]", "ldr {1}, [x18, #0x08]", out(reg) lo, out(reg) hi,
                         options(nostack, readonly, preserves_flags));
    }
    (lo, hi)
}

/// [a, a + n) lies in lo .. hi
#[cfg(target_arch = "aarch64")]
fn in_stack(a: u64, n: u64, lo: u64, hi: u64) -> bool {
    a & 7 == 0 && a >= lo && a <= hi && n <= hi - a
}

/// every slot RtlVirtualUnwind may read for this frame lies in the stack: packed unwind data reads at most
/// its frame size above sp (and above x29 when it sets up a frame pointer); an .xdata record's codes are
/// followed from every code where the unwinder may start (its prolog, an epilog, a place inside them) to
/// their end, with this context's sp and x29.  false for codes it does not know (SVE, machine frames, ...)
#[cfg(target_arch = "aarch64")]
unsafe fn unwind_reads_ok(base: u64, fe: *const RuntimeFunction, ctx: &Context, lo: u64, hi: u64) -> bool {
    let (sp0, fp) = (ctx.get(CTX_SP), ctx.get(CTX_FP));
    // SAFETY: fe and the .xdata record it names belong to a loaded image (RtlLookupFunctionEntry).
    unsafe {
        let w = (*fe).unwind;
        match w & 3 {
            0 => {}
            3 => return false, // reserved
            _ => {
                let size = (w >> 23) as u64 * 16; // FrameSize
                return in_stack(sp0, size, lo, hi) && ((w >> 21) & 3 != 3 || in_stack(fp, size, lo, hi));
            }
        }
        let x = (base + w as u64) as *const u32;
        let h = *x;
        if (h >> 18) & 3 != 0 {
            return false; // version
        }
        let (mut epilogs, mut words) = ((h >> 22) & 31, (h >> 27) & 31);
        let mut p = x.add(1);
        if epilogs == 0 && words == 0 {
            epilogs = *p & 0xFFFF;
            words = (*p >> 16) & 0xFF;
            p = p.add(1);
        }
        if (h >> 21) & 1 == 0 {
            p = p.add(epilogs as usize); // the epilog scopes (E = 0)
        }
        let n = words as usize * 4;
        if n > CODE_BYTES {
            return false; // more codes than any function of ours has: not checked
        }
        let mut code = [0u8; CODE_BYTES + 3];
        core::ptr::copy_nonoverlapping(p as *const u8, code.as_mut_ptr(), n);
        let mut start = 0;
        while start < n {
            let mut sp = sp0;
            let mut next = 0u64; // save_next codes before a pair: each adds 16 bytes to it
            let mut i = start;
            let mut first_len = 0;
            while i < n {
                let (b, b1) = (code[i], code[i + 1] as u64);
                let z6 = (b1 & 0x3F) * 8; // the offset of the two-byte saves
                let pair = 16 + 16 * next;
                let (len, ok) = match b {
                    0x00..=0x1F => {
                        sp = sp.wrapping_add((b & 0x1F) as u64 * 16); // alloc_s
                        (1, true)
                    }
                    0x20..=0x3F => {
                        let ok = in_stack(sp, pair, lo, hi); // save_r19r20_x
                        sp = sp.wrapping_add((b & 0x1F) as u64 * 8);
                        (1, ok)
                    }
                    0x40..=0x7F => (1, in_stack(sp.wrapping_add((b & 0x3F) as u64 * 8), pair, lo, hi)), // save_fplr
                    0x80..=0xBF => {
                        let ok = in_stack(sp, pair, lo, hi); // save_fplr_x
                        sp = sp.wrapping_add(((b & 0x3F) as u64 + 1) * 8);
                        (1, ok)
                    }
                    0xC0..=0xC7 => {
                        sp = sp.wrapping_add((((b & 7) as u64) << 8 | b1) * 16); // alloc_m
                        (2, true)
                    }
                    0xC8..=0xCB | 0xD6..=0xD9 => (2, in_stack(sp.wrapping_add(z6), pair, lo, hi)), // save_regp, _lrpair, _fregp
                    0xCC..=0xCF | 0xDA | 0xDB => {
                        let ok = in_stack(sp, pair, lo, hi); // save_regp_x, save_fregp_x
                        sp = sp.wrapping_add(z6 + 8);
                        (2, ok)
                    }
                    0xD0..=0xD3 | 0xDC | 0xDD => (2, in_stack(sp.wrapping_add(z6), 8, lo, hi)), // save_reg, save_freg
                    0xD4 | 0xD5 | 0xDE => {
                        let ok = in_stack(sp, 8, lo, hi); // save_reg_x, save_freg_x
                        sp = sp.wrapping_add((b1 & 0x1F) * 8 + 8);
                        (2, ok)
                    }
                    0xE0 => {
                        let (b2, b3) = (code[i + 2] as u64, code[i + 3] as u64);
                        sp = sp.wrapping_add((b1 << 16 | b2 << 8 | b3) * 16); // alloc_l
                        (4, true)
                    }
                    0xE1 => {
                        sp = fp; // set_fp: mov sp, x29
                        (1, true)
                    }
                    0xE2 => {
                        sp = fp.wrapping_sub(b1 * 8); // add_fp
                        (2, true)
                    }
                    0xE3 | 0xFC => (1, true), // nop, pac_sign_lr
                    0xE4 => break,           // end
                    0xE6 => {
                        next += 1; // save_next: the pair save after it reads 16 bytes more
                        (1, true)
                    }
                    _ => return false, // end_c (a chained scope), save_any_reg, SVE, machine frames, reserved
                };
                if !ok {
                    return false;
                }
                if b != 0xE6 {
                    next = 0;
                }
                if i == start {
                    first_len = len;
                }
                i += len;
            }
            start += first_len.max(1);
        }
        true
    }
}

/// the caller of code without unwind data at pc -> (stack slot, or 0 for lr; return address): at the fault
/// lr, when it is a return address into our code or into code with unwind data (a leaf, also one a system
/// DLL called, or a call through a bad pointer; sp stays); further up the first word from sp up (LEAF_WORDS
/// at most, inside the stack) that is the return address of a call into that code -- a bl must target at
/// most 64 KB before pc where no unwind data covers the target, a blr cannot be checked
#[cfg(target_arch = "aarch64")]
unsafe fn leaf_link(ci: &CrashInfo, ctx: &Context, pc: u64, first: bool, sp: u64, hi: u64) -> Option<(u64, u64)> {
    let mut b = 0u64;
    if first {
        let lr = ctx.get(CTX_LR);
        // SAFETY: is_return checks the code range before reading code bytes; the lookup reads nothing.
        if lr != pc && (unsafe { is_return(ci, lr) } || !unsafe { RtlLookupFunctionEntry(lr, &mut b, core::ptr::null_mut()) }.is_null()) {
            return Some((0, lr));
        }
    }
    let at = if first { pc } else { pc.wrapping_sub(4) };
    for k in 0..LEAF_WORDS {
        let a = sp + 8 * k;
        if a + 8 > hi {
            break;
        }
        // SAFETY: an aligned read inside the stack (sp is 8-aligned, a + 8 <= hi).
        let v = unsafe { *(a as *const u64) };
        // SAFETY: is_return checks the code range before reading code bytes.
        if !unsafe { is_return(ci, v) } {
            continue;
        }
        // SAFETY: v - 4 .. v lies inside our code (is_return).
        let insn = unsafe { *((v - 4) as *const u32) };
        if insn & 0xFC00_0000 != 0x9400_0000 {
            return Some((a, v)); // blr
        }
        let t = (v - 4).wrapping_add(((insn << 6) as i32 >> 4) as i64 as u64);
        // SAFETY: a lookup only.
        if t <= at && at - t < 0x10000 && unsafe { RtlLookupFunctionEntry(t, &mut b, core::ptr::null_mut()) }.is_null() {
            return Some((a, v));
        }
    }
    None
}

/// CrashInfo::frames from the unwind data; false when no frame was found (the caller scans the stack
/// instead).  A leaf keeps sp, so only the first step may end at the same sp.
#[cfg(target_arch = "aarch64")]
unsafe fn unwind(ci: &mut CrashInfo, fault: *const u8) -> bool {
    let (lo, hi) = stack_bounds();
    let mut ctx = Context([0; CONTEXT_SIZE]);
    // SAFETY: the system's CONTEXT record of the fault (CONTEXT_SIZE bytes).
    unsafe { core::ptr::copy_nonoverlapping(fault, ctx.0.as_mut_ptr(), CONTEXT_SIZE) };
    ci.nframes = 0;
    for step in 0..UNWIND_STEPS {
        let pc = ctx.get(CTX_PC);
        let sp = ctx.get(CTX_SP);
        if sp & 7 != 0 || sp < lo || sp >= hi {
            break;
        }
        let mut base = 0u64;
        // SAFETY: a lookup in the loaded modules' function tables.
        let fe = unsafe { RtlLookupFunctionEntry(pc, &mut base, core::ptr::null_mut()) };
        if fe.is_null() {
            // code without unwind data; after a stub with a frame of its own the callee-saved registers
            // and the frame pointer are not known (0: codes based on x29 then fail the checks)
            // SAFETY: reads inside lo .. hi (sp checked above).
            let Some((a, ret)) = (unsafe { leaf_link(ci, &ctx, pc, step == 0, sp, hi) }) else { break };
            if a != 0 {
                for r in 0..11 {
                    ctx.set(CTX_X19 + 8 * r, 0); // x19 .. x28, x29
                }
                ctx.set(CTX_SP, (a + 8 + 15) & !15);
            }
            ctx.set(CTX_PC, ret);
        } else {
            // SAFETY: fe / base from RtlLookupFunctionEntry: the module's unwind data.
            if !unsafe { unwind_reads_ok(base, fe, &ctx, lo, hi) } {
                break; // the codes would read outside the stack
            }
            let (mut hd, mut ef) = (0usize, 0u64);
            // SAFETY: every slot the unwind codes read lies in the stack (checked above).
            unsafe { RtlVirtualUnwind(0, base, pc, fe, &mut ctx, &mut hd, &mut ef, core::ptr::null_mut()) };
        }
        let (ret, nsp) = (ctx.get(CTX_PC), ctx.get(CTX_SP));
        if ret == 0 || nsp < sp || (nsp == sp && step > 0) {
            break; // the end of the chain (thread start), or no progress
        }
        if ret >= ci.code0 && ret < ci.code1 {
            add_frame(ci, ret);
            if ci.nframes as usize == ci.frames.len() {
                break;
            }
        }
    }
    ci.nframes > 0
}

/// v lies in the code range right after a call: bl imm26 or blr xn  [is_return]
#[cfg(target_arch = "aarch64")]
unsafe fn is_return(ci: &CrashInfo, v: u64) -> bool {
    if v & 3 != 0 || v < ci.code0.wrapping_add(4) || v >= ci.code1 {
        return false;
    }
    // SAFETY: v - 4 .. v lies inside our image's code (checked above).
    let insn = unsafe { *((v - 4) as *const u32) };
    insn & 0xFC00_0000 == 0x9400_0000 || insn & 0xFFFF_FC1F == 0xD63F_0000
}

/// v is a return address whose bl jumps to at most 64 KB before the crash address: the call of the
/// function that crashed  [leaf_call]
#[cfg(target_arch = "aarch64")]
unsafe fn leaf_call(ci: &CrashInfo, v: u64) -> bool {
    // SAFETY: called after is_return(v): v - 4 .. v lies inside our code.
    let insn = unsafe { *((v - 4) as *const u32) };
    if insn & 0xFC00_0000 != 0x9400_0000 {
        return false;
    }
    let target = (v - 4).wrapping_add(((insn << 6) as i32 >> 4) as i64 as u64);
    target <= ci.ip && ci.ip - target < 0x10000
}
