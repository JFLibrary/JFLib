// plib -- plugin libraries and the crash guard: the portable part
//                                        [src/plat/plib.s; docs/modules/PLUGIN_LOADER.md]
//
//   lib_open(path) -> Lib / Err(LIBE_*)    Windows: LoadLibraryExW without DllMain / imports
//                                          (win_lib.rs); Linux: own ELF ET_DYN loader
//                                          (linux_elfload.rs)
//   lib_sym(h, name) -> address | 0        exported symbol (own export / .dynsym lookup)
//   lib_close(h)                           unload (also when a Lib is dropped)
//   lib_seg(h, addr) -> (end of the image segment holding addr (0 = outside), LIBA_*)
//                                          -- check every pointer a plugin hands over (data:
//                                          LIBA_R, code: LIBA_X) before dereferencing it
//   guard_init()                           install the OS fault handler once (VEH on
//                                          Windows, SIGSEGV / SIGBUS / SIGILL / SIGFPE /
//                                          SIGTRAP on Linux; linux_guard.rs)
//   guard_call(f, a1, a2, a3, a4, info) -> (rax, rdx)
//        calls the code at address f with the Win64 ABI (on Linux too: the plugin ABI) as
//        f(a1, a2, a3, a4); a fault inside it on this thread (also in nested host code)
//        resumes here with rax = rdx = ERR_FAULT and info (GuardInfo) filled; otherwise
//        info.kind = 0 and rax / rdx are those of f.  Nested calls are allowed.  ERR_LIMIT
//        when the thread table is full (GT_MAX threads).
//   guard_tag() -> u64                     info.tag of this thread's innermost active guard
//                                          (0 = none)
//
// guard_call pushes a guard record (rsp / rbp of its trampoline, xmm6-15, MXCSR, x87
// control word, tag) and links it as the innermost guard of the calling thread in a table
// {OS thread id -> record} (thread id = TEB ClientId on Windows, gettid on Linux).  The
// handlers find the thread's entry by its id, unlink the innermost record (a second fault
// goes to the enclosing guard), store the fault into it and resume the thread at
// jfgui_guard_fault_ip with rsp / rbp of the trampoline's body.  That code restores x87,
// MXCSR, DF and xmm6-15 and returns through the epilogue, which restores the caller's
// callee-saved registers.  NOT a sandbox: the guard catches faults, nothing else (see the
// docs: corrupt state stays corrupt, hangs are not detected).
//
// Safety contract: guard_call runs untrusted machine code at an arbitrary address; it is a
// safe function only in the sense the assembly API is: the caller hands it addresses of
// loaded plugin code (lib_sym) or of its own `extern "win64"` functions.  Pointers a plugin
// returns are checked with lib_seg before the host reads through them.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const ERR_FAULT: i32 = -64; // the guarded call faulted (plugin disabled)

pub const LIBA_R: u32 = 1; // lib_seg flags
pub const LIBA_W: u32 = 2;
pub const LIBA_X: u32 = 4;

pub const LIBE_IO: u32 = 1; // file missing / unreadable / larger than LIB_FILE_MAX
pub const LIBE_FORMAT: u32 = 2; // not an x86-64 ELF shared object / PE DLL, or damaged
pub const LIBE_DEPS: u32 = 3; // needs other libraries / undefined symbols
pub const LIBE_RELOC: u32 = 4; // unsupported relocation
pub const LIBE_NOMEM: u32 = 5;
pub const LIBE_INIT: u32 = 6; // TLS, initialisers or an interpreter (not supported)
pub const LIBE_MAP: u32 = 7; // mmap / mprotect failed
pub const LIB_FILE_MAX: u64 = 64 << 20;
pub const LH_SEGMAX: usize = 16;

pub const GK_NONE: u32 = 0;
pub const GK_ACCESS: u32 = 1; // access violation / SIGSEGV
pub const GK_ILLEGAL: u32 = 2; // illegal / privileged instruction / SIGILL
pub const GK_ARITH: u32 = 3; // division by zero, overflow, FP trap / SIGFPE
pub const GK_BUS: u32 = 4; // misalignment / SIGBUS
pub const GK_BREAK: u32 = 5; // breakpoint (int3) / SIGTRAP
pub const GK_STACK: u32 = 6; // stack overflow (Windows)
pub const GT_MAX: usize = 128;

/// one segment of a loaded image: [start, end), LIBA_*
#[derive(Clone, Copy, Default, Debug)]
pub struct LibSeg {
    pub start: usize,
    pub end: usize,
    pub flags: u32,
}

/// a loaded plugin library (LIBH)
pub struct Lib {
    pub(crate) base: usize,
    pub(crate) span: usize,
    pub(crate) os: usize, // HMODULE (Windows) / 0
    pub(crate) nseg: u32,
    #[allow(dead_code)]
    pub(crate) nsym: u32, // .dynsym entries (Linux)
    pub(crate) symtab: usize, // .dynsym (Linux) / export directory RVA (Windows)
    #[allow(dead_code)]
    pub(crate) strtab: usize, // .dynstr (Linux)
    pub(crate) strsz: usize, // its size / the export directory's size (Windows)
    pub(crate) segs: [LibSeg; LH_SEGMAX],
}

impl Lib {
    pub(crate) fn empty() -> Lib {
        Lib { base: 0, span: 0, os: 0, nseg: 0, nsym: 0, symtab: 0, strtab: 0, strsz: 0, segs: [LibSeg::default(); LH_SEGMAX] }
    }

    /// the image base
    pub fn base(&self) -> usize {
        self.base
    }

    /// bytes mapped from the base
    pub fn span(&self) -> usize {
        self.span
    }

    /// the segments (Windows: the headers, then every section)
    pub fn segs(&self) -> &[LibSeg] {
        &self.segs[..(self.nseg as usize).min(LH_SEGMAX)]
    }
}

impl Drop for Lib {
    fn drop(&mut self) {
        crate::lib_unload(self);
    }
}

/// unload a library (dropping the Lib does the same)  [plat_lib_close]
pub fn lib_close(h: Lib) {
    drop(h)
}

/// the end of the segment of h holding addr (0 = none) and its LIBA_* flags  [plat_lib_seg]
pub fn lib_seg(h: &Lib, addr: usize) -> (usize, u32) {
    for s in h.segs() {
        if addr >= s.start && addr < s.end {
            return (s.end, s.flags);
        }
    }
    (0, 0)
}

// ---------------------------------------------------------------- the guard

/// GUARDINFO: what a guarded call hit; tag is an input (guard_tag)
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GuardInfo {
    pub kind: u32, // GK_* (0 = no fault)
    pub code: u32, // NTSTATUS (Windows) / signal number (Linux)
    pub pc: u64,   // faulting instruction
    pub addr: u64, // faulting data address (access faults) or 0
    pub tag: u64,  // in: tag of this guard (guard_tag)
}

/// the guard record (GR_* of plugin.inc; the trampoline and the handlers use these offsets)
#[repr(C, align(16))]
pub(crate) struct GuardRec {
    pub(crate) prev: u64, // 0: enclosing guard of this thread
    pub(crate) rsp: u64,  // 8: rsp / rbp of the trampoline's body
    pub(crate) rbp: u64,  // 16
    pub(crate) ent: u64,  // 24: thread table entry
    pub(crate) kind: u32, // 32: GK_* (written by the fault handler)
    pub(crate) code: u32, // 36
    pub(crate) pc: u64,   // 40
    pub(crate) addr: u64, // 48
    tag: u64,             // 56
    mxcsr: u32,           // 64
    fcw: u16,             // 68
    pad: [u8; 10],
    xmm: [u8; 160], // 80: xmm6..xmm15
    rax: u64,       // 240: results of the call
    rdx: u64,       // 248
}

const _: () = assert!(core::mem::size_of::<GuardRec>() == 256);

/// one thread: OS thread id, innermost active guard record (0 = none)
#[repr(C)]
pub(crate) struct GtEnt {
    pub(crate) tid: AtomicU64,
    pub(crate) cur: AtomicU64,
}

#[allow(clippy::declare_interior_mutable_const)]
const GT0: GtEnt = GtEnt { tid: AtomicU64::new(0), cur: AtomicU64::new(0) };
static GT: [GtEnt; GT_MAX] = [GT0; GT_MAX];
static GT_N: AtomicU32 = AtomicU32::new(0);
static GUARD_ON: AtomicU32 = AtomicU32::new(0);

// The trampoline (Win64 ABI on both OSes): jfgui_guard_run(rec, f, args) -> 0 returned /
// 1 faulted.  Frame like the assembly's PROLOG 16: [rsp+0..63] outgoing arguments, [rsp+64]
// the record pointer (the fault path finds it there: only rsp / rbp are restored).
core::arch::global_asm!(
    ".globl jfgui_guard_run",
    ".globl jfgui_guard_fault_ip",
    "jfgui_guard_run:",
    "    push rbp",
    "    mov rbp, rsp",
    "    push rbx",
    "    push rsi",
    "    push rdi",
    "    push r12",
    "    push r13",
    "    push r14",
    "    push r15",
    "    sub rsp, 88",
    "    mov [rsp + 64], rcx",
    "    mov rsi, rcx",
    "    mov r12, rdx",
    "    mov r13, r8",
    "    mov [rsi + 8], rsp",
    "    mov [rsi + 16], rbp",
    "    mov dword ptr [rsi + 64], 0",
    "    mov dword ptr [rsi + 68], 0",
    "    stmxcsr dword ptr [rsi + 64]",
    "    fnstcw word ptr [rsi + 68]",
    "    movdqu [rsi + 80], xmm6",
    "    movdqu [rsi + 96], xmm7",
    "    movdqu [rsi + 112], xmm8",
    "    movdqu [rsi + 128], xmm9",
    "    movdqu [rsi + 144], xmm10",
    "    movdqu [rsi + 160], xmm11",
    "    movdqu [rsi + 176], xmm12",
    "    movdqu [rsi + 192], xmm13",
    "    movdqu [rsi + 208], xmm14",
    "    movdqu [rsi + 224], xmm15",
    "    mov rbx, [rsi + 24]",
    "    mov [rbx + 8], rsi", // armed: GT_CUR = the record
    "    mov rcx, [r13]",
    "    mov rdx, [r13 + 8]",
    "    mov r8, [r13 + 16]",
    "    mov r9, [r13 + 24]",
    "    call r12",
    // normal return: rsp is intact; rbp / xmm6-15 are restored even if f broke the ABI
    "    mov rsi, [rsp + 64]",
    "    mov rbp, [rsi + 16]",
    "    mov [rsi + 240], rax",
    "    mov [rsi + 248], rdx",
    "    mov rbx, [rsi + 24]",
    "    mov rcx, [rsi]",
    "    mov [rbx + 8], rcx", // unlinked: GT_CUR = the enclosing record
    "    call 2f",
    "    cld",
    "    xor eax, eax",
    "    jmp 3f",
    // the fault handlers resume a faulting guarded thread here (rsp, rbp of the body; the
    // record is already unlinked)
    "jfgui_guard_fault_ip:",
    "    cld",
    "    fninit",
    "    mov rsi, [rsp + 64]",
    "    fldcw word ptr [rsi + 68]",
    "    ldmxcsr dword ptr [rsi + 64]",
    "    call 2f",
    "    mov eax, 1",
    "3:",
    "    lea rsp, [rbp - 56]",
    "    pop r15",
    "    pop r14",
    "    pop r13",
    "    pop r12",
    "    pop rdi",
    "    pop rsi",
    "    pop rbx",
    "    pop rbp",
    "    ret",
    // xmm6..xmm15 from the record at rsi; leaf
    "2:",
    "    movdqu xmm6, [rsi + 80]",
    "    movdqu xmm7, [rsi + 96]",
    "    movdqu xmm8, [rsi + 112]",
    "    movdqu xmm9, [rsi + 128]",
    "    movdqu xmm10, [rsi + 144]",
    "    movdqu xmm11, [rsi + 160]",
    "    movdqu xmm12, [rsi + 176]",
    "    movdqu xmm13, [rsi + 192]",
    "    movdqu xmm14, [rsi + 208]",
    "    movdqu xmm15, [rsi + 224]",
    "    ret",
);

unsafe extern "win64" {
    fn jfgui_guard_run(rec: *mut GuardRec, f: usize, args: *const [u64; 4]) -> u64;
    fn jfgui_guard_fault_ip();
}

/// where the OS fault handlers resume a faulting guarded thread
pub(crate) fn guard_fault_ip() -> usize {
    jfgui_guard_fault_ip as *const () as usize
}

/// the entry of thread tid or None (for the OS fault handlers: no allocation, no locks)
/// [guard_lookup]
pub(crate) fn guard_lookup(tid: u64) -> Option<&'static GtEnt> {
    let n = (GT_N.load(Ordering::SeqCst) as usize).min(GT_MAX);
    GT[..n].iter().find(|e| e.tid.load(Ordering::SeqCst) == tid)
}

/// this thread's entry (claimed on first use) or None when the table is full  [guard_self]
fn guard_self() -> Option<&'static GtEnt> {
    let tid = crate::plat_tid();
    if let Some(e) = guard_lookup(tid) {
        return Some(e);
    }
    let i = GT_N.fetch_add(1, Ordering::SeqCst) as usize;
    if i >= GT_MAX {
        GT_N.store(GT_MAX as u32, Ordering::SeqCst);
        return None;
    }
    GT[i].cur.store(0, Ordering::SeqCst);
    GT[i].tid.store(tid, Ordering::SeqCst);
    Some(&GT[i])
}

/// install the fault handler (idempotent, thread safe)  [plat_guard_init]
pub fn guard_init() -> i32 {
    if GUARD_ON.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
        crate::guard_os_install();
    }
    0
}

/// info.tag of the innermost active guard of this thread (0 = none)  [plat_guard_tag]
pub fn guard_tag() -> u64 {
    let Some(e) = guard_lookup(crate::plat_tid()) else { return 0 };
    let r = e.cur.load(Ordering::SeqCst);
    if r == 0 {
        return 0;
    }
    // SAFETY: an active record lives on this thread's stack (guard_call's frame) until the
    // guarded call returns, and only this thread reads it here.
    unsafe { (*(r as *const GuardRec)).tag }
}

/// f(a1, a2, a3, a4) under the crash guard (see the file header)  [plat_guard_call]
pub fn guard_call(f: usize, a1: u64, a2: u64, a3: u64, a4: u64, info: Option<&mut GuardInfo>) -> (u64, u64) {
    if GUARD_ON.load(Ordering::SeqCst) == 0 {
        guard_init();
    }
    let Some(ent) = guard_self() else {
        let e = crate::ERR_LIMIT as i64 as u64;
        return (e, e);
    };
    if ent.cur.load(Ordering::SeqCst) == 0 {
        crate::guard_thread(); // outermost guard: per-thread setup
    }
    let mut rec = GuardRec {
        prev: ent.cur.load(Ordering::SeqCst),
        rsp: 0,
        rbp: 0,
        ent: ent as *const GtEnt as u64,
        kind: 0,
        code: 0,
        pc: 0,
        addr: 0,
        tag: info.as_ref().map_or(0, |i| i.tag),
        mxcsr: 0,
        fcw: 0,
        pad: [0; 10],
        xmm: [0; 160],
        rax: 0,
        rdx: 0,
    };
    let args = [a1, a2, a3, a4];
    // SAFETY: the trampoline links rec into this thread's entry for the call and unlinks it
    // on both paths (return: itself; fault: the handler); rec and args outlive the call.
    // What f does is the caller's contract (see the file header).
    let faulted = unsafe { jfgui_guard_run(&mut rec, f, &args) };
    let rec = &rec;
    if faulted == 0 {
        if let Some(i) = info {
            i.kind = 0;
            i.code = 0;
            i.pc = 0;
            i.addr = 0;
        }
        return (rec.rax, rec.rdx);
    }
    if let Some(i) = info {
        i.kind = rec.kind;
        i.code = rec.code;
        i.pc = rec.pc;
        i.addr = rec.addr;
    }
    let e = ERR_FAULT as i64 as u64;
    (e, e)
}

/// (for the OS fault handlers) unlink the innermost record of the faulting thread and store
/// the fault into it -> (rsp, rbp to resume with), None when the thread has no active guard
pub(crate) fn guard_hit(tid: u64, kind: u32, code: u32, pc: u64, addr: u64) -> Option<(u64, u64)> {
    let e = guard_lookup(tid)?;
    let r = e.cur.load(Ordering::SeqCst);
    if r == 0 {
        return None;
    }
    let gr = r as *mut GuardRec;
    // SAFETY: an active record of the faulting thread (its stack, above the fault); the
    // handler runs on that thread.
    unsafe {
        e.cur.store((*gr).prev, Ordering::SeqCst); // unlink: a second fault goes further out
        (*gr).kind = kind;
        (*gr).code = code;
        (*gr).pc = pc;
        (*gr).addr = addr;
        Some(((*gr).rsp, (*gr).rbp))
    }
}

/// run the Rust closure f under the crash guard -> Ok(result), Err(ERR_FAULT) when it faulted
/// (info filled), Err(ERR_LIMIT) when the thread table is full (f did not run).  A fault
/// abandons f's frames without running destructors (what they own leaks): meant for host
/// code that may touch plugin memory (the loader fuzz runs lib_open / lib_sym under it).
pub fn guard_run<R, F: FnOnce() -> R>(f: F, info: Option<&mut GuardInfo>) -> Result<R, i32> {
    struct Ctx<F, R> {
        f: Option<F>,
        r: Option<R>,
    }
    extern "win64" fn thunk<R, F: FnOnce() -> R>(p: u64) -> u64 {
        // SAFETY: p is the Ctx of guard_run below, alive for the whole guarded call.
        let c = unsafe { &mut *(p as *mut Ctx<F, R>) };
        if let Some(f) = c.f.take() {
            c.r = Some(f());
        }
        0
    }
    let mut c = Ctx { f: Some(f), r: None };
    guard_call(thunk::<R, F> as *const () as usize, &mut c as *mut Ctx<F, R> as u64, 0, 0, 0, info);
    match c.r.take() {
        Some(r) => Ok(r),
        None if c.f.is_some() => Err(crate::ERR_LIMIT), // the table was full: f did not run
        None => Err(ERR_FAULT),                         // f was taken by the faulted call
    }
}

// pn-sys: the host side of the plugin API (PaintCraft's PNAPI) gets addresses from plugin code
// -- records it passes back (SURFACE, IMGDEC, JOB), strings, buffers on its stack -- that
// may lie anywhere; these two touch such memory under the crash guard.
/// copy len bytes at any address under the crash guard -> None when addr is 0, the copy
/// faulted or the memory for the copy is missing  (pn-sys)
pub fn guard_read(addr: usize, len: usize) -> Option<alloc::vec::Vec<u8>> {
    if addr == 0 {
        return None;
    }
    let mut v = alloc::vec::Vec::new();
    if v.try_reserve_exact(len).is_err() {
        return None;
    }
    let r = guard_run(
        || {
            // SAFETY: a fault inside the copy is caught by the guard (v stays empty: its
            // length is set only after the copy).
            let s = unsafe { core::slice::from_raw_parts(addr as *const u8, len) };
            v.extend_from_slice(s);
        },
        None,
    );
    if r.is_ok() && v.len() == len { Some(v) } else { None }
}

/// write data at any address under the crash guard -> false when addr is 0 or it faulted
/// (pn-sys)
pub fn guard_write(addr: usize, data: &[u8]) -> bool {
    if addr == 0 {
        return false;
    }
    // SAFETY: a fault inside the copy is caught by the guard; the caller hands over memory
    // the plugin API documents as writable (a record or buffer the plugin passed).
    guard_run(|| unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), addr as *mut u8, data.len()) }, None).is_ok()
}

/// copy len bytes at addr of the image of h -- only when they lie in one readable segment
/// (lib_seg), else None (pointers a plugin hands over)
pub fn lib_read(h: &Lib, addr: usize, len: usize) -> Option<alloc::vec::Vec<u8>> {
    let (end, fl) = lib_seg(h, addr);
    if end == 0 || fl & LIBA_R == 0 || len > end - addr {
        return None;
    }
    // SAFETY: [addr, addr + len) lies in a readable segment of the loaded image.
    Some(unsafe { core::slice::from_raw_parts(addr as *const u8, len) }.to_vec())
}

// ---------------------------------------------------------------- test aids (plattest)
// Win64 functions to run under guard_call: 0 sum (rax = a1 + 2 a2 + 3 a3 + 4 a4, rdx = a1 ^
// a4), 1 write to [a1], 2 ud2, 3 int3, 4 integer division by zero, 5 endless recursion in
// 4 KB frames (stack overflow), 6 clobber every callee-saved register, xmm6-15, MXCSR (round
// down), the x87 control word (single precision) and DF, then write to [a1]
core::arch::global_asm!(
    ".globl jfgui_gt_code",
    "jfgui_gt_code:",
    "0:",
    "    lea rax, [rcx + rdx*2]",
    "    lea r10, [r8 + r8*2]",
    "    add rax, r10",
    "    lea rax, [rax + r9*4]",
    "    mov rdx, rcx",
    "    xor rdx, r9",
    "    ret",
    ".p2align 4",
    "1:",
    "    mov dword ptr [rcx], 0x0BADF00D",
    "    ret",
    ".p2align 4",
    "2:",
    "    ud2",
    "    ret",
    ".p2align 4",
    "3:",
    "    int3",
    "    ret",
    ".p2align 4",
    "4:",
    "    xor ecx, ecx",
    "    mov eax, 1",
    "    xor edx, edx",
    "    div ecx",
    "    ret",
    ".p2align 4",
    "5:",
    "    sub rsp, 4096",
    "    mov [rsp], rax",
    "    call 5b",
    "    add rsp, 4096",
    "    ret",
    ".p2align 4",
    "6:",
    "    mov rax, 0x5A5A5A5A5A5A5A5A",
    "    mov rbx, rax",
    "    mov rsi, rax",
    "    mov rdi, rax",
    "    mov rbp, rax",
    "    mov r12, rax",
    "    mov r13, rax",
    "    mov r14, rax",
    "    mov r15, rax",
    "    movq xmm6, rax",
    "    movq xmm7, rax",
    "    movq xmm8, rax",
    "    movq xmm9, rax",
    "    movq xmm10, rax",
    "    movq xmm11, rax",
    "    movq xmm12, rax",
    "    movq xmm13, rax",
    "    movq xmm14, rax",
    "    movq xmm15, rax",
    "    push 0x3F80",
    "    ldmxcsr dword ptr [rsp]",
    "    mov dword ptr [rsp], 0x7F",
    "    fldcw word ptr [rsp]",
    "    add rsp, 8",
    "    std",
    "    mov dword ptr [rcx], eax",
    "    cld",
    "    ret",
    ".p2align 4",
    ".globl jfgui_gt_table",
    "jfgui_gt_table:",
    "    lea rax, [rip + 0b]",
    "    lea rdx, [rip + 1b]",
    "    lea r8, [rip + 2b]",
    "    lea r9, [rip + 3b]",
    "    mov [rcx], rax",
    "    mov [rcx + 8], rdx",
    "    mov [rcx + 16], r8",
    "    mov [rcx + 24], r9",
    "    lea rax, [rip + 4b]",
    "    lea rdx, [rip + 5b]",
    "    lea r8, [rip + 6b]",
    "    mov [rcx + 32], rax",
    "    mov [rcx + 40], rdx",
    "    mov [rcx + 48], r8",
    "    ret",
    // -> rax MXCSR, rdx x87 control word | DF << 16
    ".globl jfgui_gt_state",
    "jfgui_gt_state:",
    "    sub rsp, 8",
    "    stmxcsr dword ptr [rsp]",
    "    mov eax, [rsp]",
    "    fnstcw word ptr [rsp + 4]",
    "    movzx edx, word ptr [rsp + 4]",
    "    pushfq",
    "    pop rcx",
    "    and ecx, 0x400",
    "    shl ecx, 6",
    "    or edx, ecx",
    "    shl rdx, 32",
    "    or rax, rdx",
    "    add rsp, 8",
    "    ret",
);

unsafe extern "win64" {
    fn jfgui_gt_table(t: *mut [usize; 7]);
    fn jfgui_gt_state() -> u64;
}

/// (test aid) the address of guard test function k (see above), 0 for an unknown k
pub fn guard_test_code(k: u32) -> usize {
    let mut t = [0usize; 7];
    // SAFETY: fills the 7 addresses.
    unsafe { jfgui_gt_table(&mut t) };
    t.get(k as usize).copied().unwrap_or(0)
}

/// (test aid) this thread's MXCSR, x87 control word and direction flag
pub fn guard_test_state() -> (u32, u32, bool) {
    // SAFETY: reads the control registers only.
    let v = unsafe { jfgui_gt_state() };
    let d = (v >> 32) as u32;
    (v as u32, d & 0xFFFF, d >> 16 & 1 != 0)
}
