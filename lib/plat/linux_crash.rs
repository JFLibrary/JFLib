// linux_crash -- crash handling on Linux                [src/plat/linux/plat_linux.s linux_init, crash_handler]
//
// SIGSEGV, SIGBUS, SIGILL and SIGFPE run a handler on a 64 KB alternate stack (the main
// thread's; workers use their own stack): it fills a CrashInfo, prints "jfgui: crash signal
// N at rip 0x..., address 0x... (look it up in the .map file)" and ends in rt::crash::report
// (the app's hook, exit code 3).  SA_RESETHAND: a crash inside the handler ends the process.
// SIGPIPE is ignored.  CrashInfo::frames: the call chain, exact from the .eh_frame CFI
// (crash-check); the stack scan (plat-x11), filtered, when that finds nothing.

use core::fmt::Write;

use rt::crash::{CK_ACCESS, CK_BUS, CK_DIVIDE, CK_ILLEGAL, CK_OTHER, CrashInfo, StackText};
use rt::os::syscall6;

const SYS_RT_SIGACTION: usize = 13;
const SYS_SIGALTSTACK: usize = 131;
const SIGILL: i32 = 4;
const SIGBUS: i32 = 7;
const SIGFPE: i32 = 8;
const SIGSEGV: i32 = 11;
const SIGPIPE: usize = 13;
const SA_SIGINFO: u64 = 4;
const SA_ONSTACK: u64 = 0x0800_0000;
const SA_RESTORER: u64 = 0x0400_0000;
const SA_RESETHAND: u64 = 0x8000_0000;
const ALTSTACK_SIZE: usize = 65536;
const UC_GREGS: usize = 40; // ucontext: uc_flags, uc_link, uc_stack, then the registers
// gregs indexes of rax rbx rcx rdx rsi rdi rbp rsp r8 .. r15 rip (CrashInfo::regs order)
const UC_REGS: [usize; 17] = [13, 11, 14, 12, 9, 8, 10, 15, 0, 1, 2, 3, 4, 5, 6, 7, 16];
const REG_RIP: usize = 16;
const REG_ERR: usize = 19;

#[repr(C, align(16))]
struct AltStack([u8; ALTSTACK_SIZE]);
static mut ALTSTACK: AltStack = AltStack([0; ALTSTACK_SIZE]);

// return path of signal handlers (never used by the crash handler, which exits)
core::arch::global_asm!(
    ".globl jfgui_rs_sigret",
    "jfgui_rs_sigret:",
    "    mov eax, 15",
    "    syscall",
    "    hlt",
);

unsafe extern "C" {
    fn jfgui_rs_sigret();
    static __ehdr_start: u8;
}

/// the alternate stack, the handlers, SIGPIPE ignored (process start)
pub(crate) fn crash_init() {
    // SAFETY: plain kernel structures; ALTSTACK is only used by the kernel from now on.
    unsafe {
        let ss = [(&raw mut ALTSTACK) as usize, 0, ALTSTACK_SIZE];
        syscall6(SYS_SIGALTSTACK, ss.as_ptr() as usize, 0, 0, 0, 0, 0);
        let sa = [handler as *const () as usize, (SA_SIGINFO | SA_ONSTACK | SA_RESTORER | SA_RESETHAND) as usize,
                  jfgui_rs_sigret as *const () as usize, 0];
        for sig in [SIGSEGV, SIGBUS, SIGILL, SIGFPE] {
            syscall6(SYS_RT_SIGACTION, sig as usize, sa.as_ptr() as usize, 0, 8, 0, 0);
        }
        let ign = [1usize, SA_RESTORER as usize, jfgui_rs_sigret as *const () as usize, 0]; // SIG_IGN
        syscall6(SYS_RT_SIGACTION, SIGPIPE, ign.as_ptr() as usize, 0, 8, 0, 0);
    }
}

extern "C" fn handler(sig: i32, si: *const u8, uc: *const u8) -> ! {
    if rt::crash::busy() {
        rt::os::exit(3);
    }
    let mut ci = CrashInfo::empty();
    ci.code = sig as u32;
    ci.kind = match sig {
        SIGSEGV => CK_ACCESS,
        SIGBUS => CK_BUS,
        SIGILL => CK_ILLEGAL,
        SIGFPE => CK_DIVIDE,
        _ => CK_OTHER,
    };
    // SAFETY: the kernel passes a valid siginfo and ucontext.
    let si_addr = unsafe {
        let greg = |i: usize| *(uc.add(UC_GREGS + i * 8) as *const u64);
        let si_addr = *(si.add(16) as *const u64);
        if ci.kind == CK_ACCESS || ci.kind == CK_BUS {
            ci.addr = si_addr;
        }
        if ci.kind == CK_ACCESS {
            // page fault error code: bit 1 write, bit 4 instruction fetch
            let err = greg(REG_ERR);
            ci.access = if err & 16 != 0 { 8 } else { (err >> 1 & 1) as u32 };
        }
        for (i, &g) in UC_REGS.iter().enumerate() {
            ci.regs[i] = greg(g);
        }
        ci.ip = greg(REG_RIP);
        si_addr
    };
    code_range(&mut ci);
    call_chain(&mut ci); // crash-check: CFI, else the filtered stack scan
    let mut line = StackText::<160>::new();
    let _ = writeln!(line, "jfgui: crash signal {sig} at rip 0x{:x}, address 0x{si_addr:x} (look it up in the .map file)",
                     ci.ip);
    rt::os::write_stderr(line.bytes());
    rt::crash::report(&ci)
}

/// code0 / code1 = the executable PT_LOAD segment of our ELF image (base 0: the .map lists
/// absolute addresses)
fn code_range(ci: &mut CrashInfo) {
    ci.code0 = u64::MAX;
    ci.code1 = 0;
    // SAFETY: __ehdr_start is the ELF header the linker maps with the image.
    unsafe {
        let eh = &raw const __ehdr_start as usize;
        let phoff = *((eh + 32) as *const u64) as usize;
        let phentsize = *((eh + 54) as *const u16) as usize;
        let phnum = *((eh + 56) as *const u16) as usize;
        for k in 0..phnum {
            let ph = eh + phoff + k * phentsize;
            if *(ph as *const u32) == 1 && *((ph + 4) as *const u32) & 1 != 0 {
                // PT_LOAD, PF_X
                let start = *((ph + 16) as *const u64);
                ci.code0 = ci.code0.min(start);
                ci.code1 = ci.code1.max(start + *((ph + 40) as *const u64));
            }
        }
    }
}

// ---------------------------------------------------------------- the call chain (crash-check)
//
// Exact: the .eh_frame CFI of our static image (rustc emits unwind tables for every function;
// ld.lld writes .eh_frame_hdr with the sorted FDE table, found through PT_GNU_EH_FRAME).  From
// the signal context each step finds the FDE of pc (pc - 1 for return addresses), runs its
// CFA program up to there (the rules for the CFA and the saved registers), and reads the
// caller's registers and return address from the stack.  A pc without an FDE (naked
// functions such as crash_test, global_asm stubs) is linked to its caller by the first word
// above rsp (at most 1 KB; [rsp] for a leaf) that follows a call instruction calling into
// that code (calls_into); from there the CFI goes on.  Stack words are read with
// process_vm_readv (an unmapped address fails instead of faulting), the tables only inside
// the PT_LOAD segment that holds them: a corrupted stack ends the chain.  No allocation, no
// frame pointers needed.
//
// Fallback (no frame found that way: no .eh_frame_hdr, or code without CFI whose caller is
// not found in that 1 KB): the stack scan of the asm (stack_scan), filtered -- see
// scan_filtered for its limits.

const SYS_GETPID: usize = 39;
const SYS_PROCESS_VM_READV: usize = 310;
const CRASH_STACK: usize = 16384; // bytes of the crashing stack scanned for return addresses
const PT_LOAD: u32 = 1;
const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
const UNWIND_STEPS: usize = 64; // frames stepped through at most
const LEAF_WORDS: u64 = 128; // code without CFI: its return address is searched in 1 KB above rsp
const NREG: usize = 17; // DWARF columns 0..15 (rax rdx rcx rbx rsi rdi rbp rsp r8 .. r15), 16 = pc
const DW_RSP: usize = 7;
const DW_RA: usize = 16;
const STATE_DEPTH: usize = 8; // DW_CFA_remember_state nesting
// CrashInfo::regs index of each DWARF register 0..15
const DW_TO_CI: [usize; 16] = [0, 3, 2, 1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// CrashInfo::frames: the CFI chain, else the filtered stack scan
fn call_chain(ci: &mut CrashInfo) {
    ci.nframes = 0;
    let eh = eh_tables();
    if let Some(eh) = &eh
        && cfi_chain(ci, eh)
    {
        return;
    }
    stack_scan(ci, eh.as_ref());
}

// ---- reading our image and the stack

/// bounds-checked reads inside one mapped PT_LOAD segment of our image
struct Seg {
    lo: u64,
    hi: u64,
}

impl Seg {
    fn get<const N: usize>(&self, a: u64) -> Option<[u8; N]> {
        if a < self.lo || a >= self.hi || self.hi - a < N as u64 {
            return None;
        }
        let mut b = [0u8; N];
        // SAFETY: a .. a + N lies inside a PT_LOAD segment of our image (mapped, readable).
        unsafe { core::ptr::copy_nonoverlapping(a as *const u8, b.as_mut_ptr(), N) };
        Some(b)
    }
}

/// a cursor over .eh_frame / .eh_frame_hdr data
struct Cur<'a> {
    s: &'a Seg,
    p: u64,
}

impl Cur<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = self.s.get::<1>(self.p)?[0];
        self.p += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.s.get::<2>(self.p)?);
        self.p += 2;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.s.get::<4>(self.p)?);
        self.p += 4;
        Some(v)
    }
    fn u64(&mut self) -> Option<u64> {
        let v = u64::from_le_bytes(self.s.get::<8>(self.p)?);
        self.p += 8;
        Some(v)
    }
    fn uleb(&mut self) -> Option<u64> {
        let (mut v, mut sh) = (0u64, 0u32);
        loop {
            let b = self.u8()?;
            if sh < 64 {
                v |= ((b & 0x7f) as u64) << sh;
            }
            sh += 7;
            if b & 0x80 == 0 {
                return Some(v);
            }
            if sh > 70 {
                return None;
            }
        }
    }
    fn sleb(&mut self) -> Option<i64> {
        let (mut v, mut sh) = (0i64, 0u32);
        loop {
            let b = self.u8()?;
            if sh < 64 {
                v |= ((b & 0x7f) as i64) << sh;
            }
            sh += 7;
            if b & 0x80 == 0 {
                if sh < 64 && b & 0x40 != 0 {
                    v |= -1i64 << sh;
                }
                return Some(v);
            }
            if sh > 70 {
                return None;
            }
        }
    }
    /// a DW_EH_PE_* encoded pointer: absolute, pc-relative or relative to `data`
    /// (.eh_frame_hdr); an indirect one is not followed
    fn ptr(&mut self, enc: u8, data: u64) -> Option<u64> {
        let at = self.p;
        let v = match enc & 0x0f {
            0x00 | 0x04 | 0x0c => self.u64()?,
            0x01 => self.uleb()?,
            0x02 => self.u16()? as u64,
            0x03 => self.u32()? as u64,
            0x09 => self.sleb()? as u64,
            0x0a => self.u16()? as i16 as i64 as u64,
            0x0b => self.u32()? as i32 as i64 as u64,
            _ => return None,
        };
        let base = match enc & 0x70 {
            0x00 => 0,
            0x10 => at,
            0x30 => data,
            _ => return None,
        };
        Some(base.wrapping_add(v))
    }
}

/// one 8-byte word of our own memory, or None when it is not mapped (process_vm_readv)
fn peek(pid: usize, a: u64) -> Option<u64> {
    let mut v = 0u64;
    let local = [(&raw mut v) as usize, 8];
    let remote = [a as usize, 8];
    // SAFETY: reads 8 bytes of our own address space into v (the iovecs are live).
    let got = unsafe { syscall6(SYS_PROCESS_VM_READV, pid, local.as_ptr() as usize, 1, remote.as_ptr() as usize, 1, 0) };
    if got == 8 { Some(v) } else { None }
}

// ---- the unwind tables

/// .eh_frame_hdr: the sorted table (initial location, FDE), both relative to the header
struct Eh {
    seg: Seg,
    hdr: u64,
    table: u64,
    count: u64,
}

/// the FDE table of our image (PT_GNU_EH_FRAME), None without one or in another format
fn eh_tables() -> Option<Eh> {
    let (mut hdr, mut seg) = (0u64, None);
    // SAFETY: __ehdr_start is our ELF header, mapped with the program headers after it.
    unsafe {
        let eh = &raw const __ehdr_start as usize;
        let phoff = *((eh + 32) as *const u64) as usize;
        let phentsize = *((eh + 54) as *const u16) as usize;
        let phnum = *((eh + 56) as *const u16) as usize;
        for k in 0..phnum {
            let ph = eh + phoff + k * phentsize;
            if *(ph as *const u32) == PT_GNU_EH_FRAME {
                hdr = *((ph + 16) as *const u64);
            }
        }
        for k in 0..phnum {
            let ph = eh + phoff + k * phentsize;
            let (va, memsz) = (*((ph + 16) as *const u64), *((ph + 40) as *const u64));
            // PT_LOAD, PF_R, holding the header (and .eh_frame after it)
            if *(ph as *const u32) == PT_LOAD && *((ph + 4) as *const u32) & 4 != 0 && hdr >= va && hdr < va + memsz {
                seg = Some(Seg { lo: va, hi: va + memsz });
            }
        }
    }
    let seg = seg?;
    let mut c = Cur { s: &seg, p: hdr };
    let (ver, ptr_enc, count_enc, table_enc) = (c.u8()?, c.u8()?, c.u8()?, c.u8()?);
    if ver != 1 || table_enc != 0x3b {
        return None; // datarel sdata4 is what linkers write
    }
    c.ptr(ptr_enc, hdr)?; // eh_frame_ptr (the FDEs are reached through the table)
    let count = c.ptr(count_enc, hdr)?;
    let table = c.p;
    Some(Eh { seg, hdr, table, count })
}

/// a CIE: alignment factors, return address column, FDE pointer encoding, initial instructions
struct Cie {
    code_align: u64,
    data_align: i64,
    ra: u64,
    fde_enc: u8,
    aug_z: bool,
    ins: u64,
    end: u64,
}

fn parse_cie(s: &Seg, at: u64) -> Option<Cie> {
    let mut c = Cur { s, p: at };
    let len = c.u32()? as u64;
    if len == 0 || len == 0xffff_ffff {
        return None; // (64-bit lengths are not written for code this size)
    }
    let end = at + 4 + len;
    if c.u32()? != 0 {
        return None; // not a CIE
    }
    let ver = c.u8()?;
    if ver != 1 && ver != 3 && ver != 4 {
        return None;
    }
    let mut aug = [0u8; 8];
    let mut n = 0;
    loop {
        let b = c.u8()?;
        if b == 0 {
            break;
        }
        if n == aug.len() {
            return None;
        }
        aug[n] = b;
        n += 1;
    }
    if ver == 4 {
        c.u8()?; // address size
        c.u8()?; // segment selector size
    }
    let code_align = c.uleb()?;
    let data_align = c.sleb()?;
    let ra = if ver == 1 { c.u8()? as u64 } else { c.uleb()? };
    let mut fde_enc = 0u8; // absolute pointers unless 'R' says otherwise
    let aug_z = n > 0 && aug[0] == b'z';
    if aug_z {
        let alen = c.uleb()?;
        let aend = c.p + alen;
        for &a in &aug[1..n] {
            match a {
                b'R' => fde_enc = c.u8()?,
                b'P' => {
                    let e = c.u8()?;
                    c.ptr(e & 0x7f, 0)?; // the personality routine (not needed)
                }
                b'L' => {
                    c.u8()?; // LSDA encoding
                }
                b'S' | b'B' => {}
                _ => break, // the rest of the augmentation data is skipped below
            }
        }
        c.p = aend;
    } else if n > 0 {
        return None; // an augmentation without 'z' cannot be skipped
    }
    Some(Cie { code_align, data_align, ra, fde_enc, aug_z, ins: c.p, end })
}

/// an FDE: its code range, CIE and instructions
struct Fde {
    begin: u64,
    end: u64,
    cie: Cie,
    ins: u64,
    ins_end: u64,
}

/// the FDE whose code range holds pc (binary search in .eh_frame_hdr)
fn find_fde(eh: &Eh, pc: u64) -> Option<Fde> {
    let entry = |i: u64| -> Option<(u64, u64)> {
        let mut c = Cur { s: &eh.seg, p: eh.table + 8 * i };
        Some((c.ptr(0x3b, eh.hdr)?, c.ptr(0x3b, eh.hdr)?))
    };
    let (mut lo, mut hi) = (0u64, eh.count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if entry(mid)?.0 <= pc {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return None;
    }
    let (_, at) = entry(lo - 1)?;
    let mut c = Cur { s: &eh.seg, p: at };
    let len = c.u32()? as u64;
    if len == 0 || len == 0xffff_ffff {
        return None;
    }
    let ins_end = at + 4 + len;
    let id_at = c.p;
    let id = c.u32()? as u64;
    if id == 0 || id > id_at {
        return None; // a CIE, or a broken pointer
    }
    let cie = parse_cie(&eh.seg, id_at - id)?;
    let begin = c.ptr(cie.fde_enc, eh.hdr)?;
    let range = c.ptr(cie.fde_enc & 0x0f, eh.hdr)?;
    if cie.aug_z {
        let alen = c.uleb()?;
        c.p += alen;
    }
    if pc < begin || pc - begin >= range {
        return None;
    }
    Some(Fde { begin, end: begin + range, cie, ins: c.p, ins_end })
}

// ---- the CFA program

#[derive(Clone, Copy)]
enum Rule {
    Same,     // unchanged in the caller
    Undef,    // not recoverable
    At(i64),  // saved at CFA + n
    Val(i64), // the value CFA + n
    Reg(u64), // in another register
}

#[derive(Clone, Copy)]
struct Row {
    cfa_reg: u64,
    cfa_off: i64,
    r: [Rule; NREG],
}

impl Row {
    fn set(&mut self, reg: u64, rule: Rule) {
        if (reg as usize) < NREG {
            self.r[reg as usize] = rule;
        }
    }
}

/// runs CFA instructions from..to starting at code address loc until the row for pc is
/// reached; init = the CIE's row (for DW_CFA_restore); None for anything it does not know
fn run_cfa(s: &Seg, cie: &Cie, from: u64, to: u64, mut loc: u64, pc: u64, row: &mut Row, init: &Row) -> Option<()> {
    let mut saved = [*row; STATE_DEPTH];
    let mut depth = 0;
    let mut c = Cur { s, p: from };
    let da = cie.data_align;
    while c.p < to {
        let op = c.u8()?;
        let low = (op & 0x3f) as u64;
        let mut adv = None;
        match op >> 6 {
            1 => adv = Some(low), // DW_CFA_advance_loc
            2 => {
                let off = (c.uleb()? as i64).wrapping_mul(da); // DW_CFA_offset
                row.set(low, Rule::At(off));
            }
            3 => {
                if (low as usize) < NREG {
                    row.r[low as usize] = init.r[low as usize]; // DW_CFA_restore
                }
            }
            _ => match op {
                0x00 => {} // nop
                0x01 => {
                    // set_loc
                    loc = c.ptr(cie.fde_enc, 0)?;
                    if loc > pc {
                        return Some(());
                    }
                }
                0x02 => adv = Some(c.u8()? as u64),  // advance_loc1
                0x03 => adv = Some(c.u16()? as u64), // advance_loc2
                0x04 => adv = Some(c.u32()? as u64), // advance_loc4
                0x05 => {
                    // offset_extended
                    let (r, off) = (c.uleb()?, c.uleb()? as i64);
                    row.set(r, Rule::At(off.wrapping_mul(da)));
                }
                0x06 => {
                    // restore_extended
                    let r = c.uleb()? as usize;
                    if r < NREG {
                        row.r[r] = init.r[r];
                    }
                }
                0x07 => row.set(c.uleb()?, Rule::Undef), // undefined
                0x08 => row.set(c.uleb()?, Rule::Same),  // same_value
                0x09 => {
                    // register
                    let (r, r2) = (c.uleb()?, c.uleb()?);
                    row.set(r, Rule::Reg(r2));
                }
                0x0a => {
                    // remember_state
                    if depth == STATE_DEPTH {
                        return None;
                    }
                    saved[depth] = *row;
                    depth += 1;
                }
                0x0b => {
                    // restore_state
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                    *row = saved[depth];
                }
                0x0c => {
                    // def_cfa
                    row.cfa_reg = c.uleb()?;
                    row.cfa_off = c.uleb()? as i64;
                }
                0x0d => row.cfa_reg = c.uleb()?,          // def_cfa_register
                0x0e => row.cfa_off = c.uleb()? as i64,   // def_cfa_offset
                0x10 | 0x16 => {
                    // expression / val_expression: not evaluated, the register is lost
                    let r = c.uleb()?;
                    let n = c.uleb()?;
                    c.p = c.p.checked_add(n)?;
                    row.set(r, Rule::Undef);
                }
                0x11 => {
                    // offset_extended_sf
                    let (r, off) = (c.uleb()?, c.sleb()?);
                    row.set(r, Rule::At(off.wrapping_mul(da)));
                }
                0x12 => {
                    // def_cfa_sf
                    row.cfa_reg = c.uleb()?;
                    row.cfa_off = c.sleb()?.wrapping_mul(da);
                }
                0x13 => row.cfa_off = c.sleb()?.wrapping_mul(da), // def_cfa_offset_sf
                0x14 => {
                    // val_offset
                    let (r, off) = (c.uleb()?, c.uleb()? as i64);
                    row.set(r, Rule::Val(off.wrapping_mul(da)));
                }
                0x15 => {
                    // val_offset_sf
                    let (r, off) = (c.uleb()?, c.sleb()?);
                    row.set(r, Rule::Val(off.wrapping_mul(da)));
                }
                0x2e => {
                    c.uleb()?; // GNU_args_size
                }
                0x2f => {
                    // GNU_negative_offset_extended
                    let (r, off) = (c.uleb()?, c.uleb()? as i64);
                    row.set(r, Rule::At(off.wrapping_mul(da).wrapping_neg()));
                }
                _ => return None, // def_cfa_expression and unknown operations
            },
        }
        if let Some(d) = adv {
            loc = loc.wrapping_add(d.wrapping_mul(cie.code_align));
            if loc > pc {
                return Some(());
            }
        }
    }
    Some(())
}

enum Step {
    Done,  // regs hold the caller's registers
    NoFde, // pc has no unwind data
    Fail,  // unwind data it cannot follow, or the stack is unreadable
}

/// one frame up with the CFI: regs (DWARF order, [16] = pc) become the caller's; `first` =
/// the faulting frame (pc itself, not pc - 1, selects the row)
fn cfi_step(eh: &Eh, pid: usize, regs: &mut [Option<u64>; NREG], first: bool) -> Step {
    let Some(pc) = regs[DW_RA] else { return Step::Fail };
    let at = if first { pc } else { pc.wrapping_sub(1) };
    let Some(fde) = find_fde(eh, at) else { return Step::NoFde };
    let cie = &fde.cie;
    if cie.ra != DW_RA as u64 {
        return Step::Fail;
    }
    let mut row = Row { cfa_reg: DW_RSP as u64, cfa_off: 8, r: [Rule::Same; NREG] };
    let init = row;
    if run_cfa(&eh.seg, cie, cie.ins, cie.end, 0, u64::MAX, &mut row, &init).is_none() {
        return Step::Fail;
    }
    let init = row;
    if run_cfa(&eh.seg, cie, fde.ins, fde.ins_end, fde.begin, at, &mut row, &init).is_none() || at >= fde.end {
        return Step::Fail;
    }
    let Some(Some(base)) = regs.get(row.cfa_reg as usize).copied() else { return Step::Fail };
    let cfa = base.wrapping_add(row.cfa_off as u64);
    let mut next = [None; NREG];
    for (k, n) in next.iter_mut().enumerate() {
        *n = match row.r[k] {
            Rule::Same => regs[k],
            Rule::Undef => None,
            Rule::At(o) => peek(pid, cfa.wrapping_add(o as u64)),
            Rule::Val(o) => Some(cfa.wrapping_add(o as u64)),
            Rule::Reg(r) => regs.get(r as usize).copied().flatten(),
        };
    }
    next[DW_RSP] = Some(cfa);
    if next[DW_RA].is_none() {
        return Step::Fail;
    }
    *regs = next;
    Step::Done
}

/// CrashInfo::frames from the CFI; false when no frame was found
fn cfi_chain(ci: &mut CrashInfo, eh: &Eh) -> bool {
    // SAFETY: getpid has no arguments.
    let pid = unsafe { syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0) } as usize;
    let mut regs = [None; NREG];
    for (k, r) in regs.iter_mut().take(16).enumerate() {
        *r = Some(ci.regs[DW_TO_CI[k]]);
    }
    regs[DW_RA] = Some(ci.ip);
    for step in 0..UNWIND_STEPS {
        let Some(sp) = regs[DW_RSP] else { break };
        match cfi_step(eh, pid, &mut regs, step == 0) {
            Step::Done => {}
            Step::Fail => break,
            Step::NoFde => {
                // code without unwind data (naked functions, global_asm stubs): its return
                // address is the first word from rsp up whose call instruction calls into
                // this code -- [rsp] for a leaf; a stub with a frame of its own may have
                // changed registers, so only rsp and pc are known after it
                let pc = regs[DW_RA].unwrap_or(0);
                let mut found = None;
                for k in 0..LEAF_WORDS {
                    let a = sp + 8 * k;
                    let Some(v) = peek(pid, a) else { break };
                    if calls_into(ci, Some(eh), v, pc, step == 0) {
                        found = Some((a, v));
                        break;
                    }
                }
                let Some((a, ret)) = found else { break };
                if a != sp {
                    regs = [None; NREG];
                }
                regs[DW_RA] = Some(ret);
                regs[DW_RSP] = Some(a + 8);
            }
        }
        let (Some(ret), Some(nsp)) = (regs[DW_RA], regs[DW_RSP]) else { break };
        if nsp <= sp || ret < ci.code0 || ret >= ci.code1 {
            break; // no progress, or out of our code (the end of the chain)
        }
        add_frame(ci, ret);
        if ci.nframes as usize == ci.frames.len() {
            break;
        }
    }
    ci.nframes > 0
}

// ---- the fallback: the stack scan  [plat_linux.s stack_scan, add_frame, lx_is_return]

/// the call instruction right before v (a return address candidate in our code): Some(Some(
/// target)) for call rel32, Some(None) for an indirect call (FF /2 with a register or a
/// memory operand: [reg], [reg + disp8 / disp32], [rip + disp32], SIB forms)
fn call_before(ci: &CrashInfo, v: u64) -> Option<Option<u64>> {
    if v < ci.code0.wrapping_add(8) || v >= ci.code1 {
        return None;
    }
    // SAFETY: v - 8 .. v lies inside our image's code (checked above).
    let b = |k: u64| unsafe { *((v - k) as *const u8) };
    if b(5) == 0xE8 {
        // SAFETY: as above.
        let rel = unsafe { ((v - 4) as *const i32).read_unaligned() };
        return Some(Some(v.wrapping_add(rel as i64 as u64)));
    }
    // FF /2: ModRM (and SIB) decide the length of the instruction ending at v
    for len in [2u64, 3, 4, 6, 7] {
        if b(len) != 0xFF {
            continue;
        }
        let m = b(len - 1);
        if (m >> 3) & 7 != 2 {
            continue;
        }
        let (md, rm) = (m >> 6, m & 7);
        let want = match (md, rm) {
            (3, _) => 2,
            (0, 5) => 6,
            (0, 4) => {
                if len >= 3 && b(len - 2) & 7 == 5 { 7 } else { 3 } // SIB without base: disp32
            }
            (0, _) => 2,
            (1, 4) => 4,
            (1, _) => 3,
            (2, 4) => 7,
            _ => 6,
        };
        if want == len {
            return Some(None);
        }
    }
    None
}

/// v lies in our code right after a call instruction  [lx_is_return]
fn is_return(ci: &CrashInfo, v: u64) -> bool {
    call_before(ci, v).is_some()
}

fn add_frame(ci: &mut CrashInfo, v: u64) {
    let n = ci.nframes as usize;
    if n < ci.frames.len() {
        ci.frames[n] = v;
        ci.nframes += 1;
    }
}

/// the function holding code address pc: (start, end) of its FDE
fn func_of(eh: Option<&Eh>, pc: u64) -> Option<(u64, u64)> {
    find_fde(eh?, pc).map(|f| (f.begin, f.end))
}

/// v (a return address) may be the caller of the frame at pc (`first`: pc is the faulting
/// instruction, else a return address): a direct call must target the start of the function
/// holding pc -- or, in code without CFI, a place at most 64 KB before pc that no FDE
/// covers (as leaf_call of the asm); an indirect call cannot be checked and passes
fn calls_into(ci: &CrashInfo, eh: Option<&Eh>, v: u64, pc: u64, first: bool) -> bool {
    let Some(call) = call_before(ci, v) else { return false };
    let Some(target) = call else { return true };
    let at = if first { pc } else { pc.wrapping_sub(1) };
    match func_of(eh, at) {
        Some((start, _)) => target == start,
        None => {
            target <= at && at - target < 0x10000 && (eh.is_none() || func_of(eh, target).is_none())
        }
    }
}

/// up to 16 KB above the faulting rsp copied with process_vm_readv (stops at an unmapped page
/// instead of faulting); scan_filtered on it, else every return address found  [stack_scan]
fn stack_scan(ci: &mut CrashInfo, eh: Option<&Eh>) {
    let mut copy = [0u64; CRASH_STACK / 8];
    let lo = ci.regs[7] & !7; // rsp
    let local = [copy.as_mut_ptr() as usize, CRASH_STACK];
    let remote = [lo as usize, CRASH_STACK];
    // SAFETY: getpid; process_vm_readv of our own stack into the local copy (iovecs live).
    let got = unsafe {
        let pid = syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0);
        syscall6(SYS_PROCESS_VM_READV, pid as usize, local.as_ptr() as usize, 1, remote.as_ptr() as usize, 1, 0)
    };
    if got <= 0 {
        return;
    }
    let words = &copy[..(got as usize / 8).min(CRASH_STACK / 8)];
    scan_filtered(ci, eh, words);
    if ci.nframes > 0 {
        return;
    }
    for &v in words {
        if ci.nframes as usize >= ci.frames.len() {
            break;
        }
        if is_return(ci, v) {
            add_frame(ci, v);
        }
    }
}

/// the scan without frame pointers: walking up from rsp, a return address is taken when its
/// call instruction calls into the function of the frame below (calls_into), starting at the
/// faulting instruction.  Limits: a stale return address from an indirect call (dyn methods,
/// function pointers) is taken like a live one; a tail call breaks the link (the caller's
/// call targets another function), so the chain may end early; frames above 16 KB are not seen.
fn scan_filtered(ci: &mut CrashInfo, eh: Option<&Eh>, words: &[u64]) {
    let mut pc = ci.ip;
    let mut first = true;
    for &v in words {
        if ci.nframes as usize >= ci.frames.len() {
            break;
        }
        if calls_into(ci, eh, v, pc, first) {
            add_frame(ci, v);
            pc = v;
            first = false;
        }
    }
}
