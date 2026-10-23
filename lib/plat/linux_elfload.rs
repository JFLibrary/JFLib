// linux_elfload -- plugin libraries on Linux: a small ELF loader without libc
//                                        [src/plat/linux/elfload.s; docs/modules/PLUGIN_LOADER.md]
//
//   lib_open(path) -> Lib / Err(LIBE_*)
//   lib_sym(h, name) -> address | 0       .dynsym lookup (count from DT_HASH nchain or the
//                                         DT_GNU_HASH chains)
//   (crate) lib_unload(h)                 munmap (lib_close / drop)
//
// Accepted: ELFCLASS64, little endian, ET_DYN, EM_X86_64, program headers of 56 bytes
// (1..64), at most LH_SEGMAX PT_LOAD segments in ascending, page-disjoint order, never
// writable AND executable, one PT_DYNAMIC; relocations R_X86_64_NONE / RELATIVE and
// R_X86_64_64 / GLOB_DAT / JUMP_SLOT against symbols the object defines itself (RELA only).
// Rejected: DT_NEEDED or undefined symbols (LIBE_DEPS), PT_INTERP / PT_TLS / DT_INIT /
// DT_INIT_ARRAY / DT_PREINIT_ARRAY (LIBE_INIT: nothing of the object runs while loading),
// DT_REL and other relocation types (LIBE_RELOC), everything malformed (LIBE_FORMAT).
// The file is untrusted input: every header value, offset, size and address is checked in
// 64-bit arithmetic against the file size or the mapped span (LIB_SPAN_MAX) before use.
// Mapping: one anonymous read-write reservation of the whole span, the segments copied in,
// relocations applied, then mprotect per segment (gaps PROT_NONE, PT_GNU_RELRO read-only).
// While loading, the mapping is accessed as a slice (every access bounds-checked on top of
// the assembly's checks); lib_sym reads the protected image like the assembly.

use alloc::vec::Vec;

use rt::os::syscall6;

use crate::{
    OsFile, LH_SEGMAX, LIB_FILE_MAX, LIBE_DEPS, LIBE_FORMAT, LIBE_INIT, LIBE_IO, LIBE_MAP, LIBE_NOMEM,
    LIBE_RELOC, Lib, LibSeg,
};

const SYS_MMAP: usize = 9;
const SYS_MPROTECT: usize = 10;
const SYS_MUNMAP: usize = 11;
const PROT_READ: u32 = 1;
const PROT_WRITE: u32 = 2;
const PROT_EXEC: u32 = 4;
const LIB_SPAN_MAX: u64 = 256 << 20;
const SYM_MAX: u64 = 65536;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_TLS: u32 = 7;
const PT_GNU_RELRO: u32 = 0x6474_E552;
const PF_X: u64 = 1;
const PF_W: u64 = 2;
const PF_R: u64 = 4;
const DT_NEEDED: u64 = 1;
const DT_PLTRELSZ: u64 = 2;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_STRSZ: u64 = 10;
const DT_SYMENT: u64 = 11;
const DT_INIT: u64 = 12;
const DT_REL: u64 = 17;
const DT_TEXTREL: u64 = 22;
const DT_RELRSZ: u64 = 35;
const DT_RELR: u64 = 36;
const DT_RELSZ: u64 = 18;
const DT_PLTREL: u64 = 20;
const DT_JMPREL: u64 = 23;
const DT_INIT_ARRAYSZ: u64 = 27;
const DT_PREINIT_ARRAYSZ: u64 = 33;
const DT_GNU_HASH: u64 = 0x6FFF_FEF5;
const R_NONE: u32 = 0;
const R_64: u32 = 1;
const R_GLOB_DAT: u32 = 6;
const R_JUMP_SLOT: u32 = 7;
const R_RELATIVE: u32 = 8;
const SHN_ABS: u32 = 0xFFF1;

fn g16(b: &[u8], o: u64) -> Option<u32> {
    let o = o as usize;
    Some(u16::from_le_bytes(b.get(o..o.checked_add(2)?)?.try_into().ok()?) as u32)
}

fn g32(b: &[u8], o: u64) -> Option<u32> {
    let o = o as usize;
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn g64(b: &[u8], o: u64) -> Option<u64> {
    let o = o as usize;
    Some(u64::from_le_bytes(b.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// one PT_LOAD: va, end (va + memsz), file offset, file size, p_flags (then PROT_* bits)
#[derive(Clone, Copy, Default)]
struct Seg {
    va: u64,
    end: u64,
    off: u64,
    fsz: u64,
    flags: u64,
}

/// the dynamic tags and the segment list (elf_map's L block)
#[derive(Default)]
struct El {
    nseg: usize,
    segs: [Seg; LH_SEGMAX],
    dynoff: u64,
    dynsz: u64,
    relro_va: u64,
    relro_sz: u64,
    strtab: u64,
    strsz: u64,
    symtab: u64,
    hash: u64,
    gnuhash: u64,
    rela: u64,
    relasz: u64,
    jmprel: u64,
    pltrelsz: u64,
    nsym: u64,
    pltrel: u64,
    syment: u64,
    relaent: u64,
    span: u64,
}

/// open a plugin ELF object (see the file header)  [plat_lib_open]
pub fn lib_open(path: &str) -> Result<Lib, u32> {
    if !crate::policy::plugin_allowed(path) {
        return Err(LIBE_IO);
    }
    let f = OsFile::open_read_nofollow(path).ok_or(LIBE_IO)?;
    let size = f.size().ok_or(LIBE_IO)?;
    if size > LIB_FILE_MAX {
        return Err(LIBE_IO);
    }
    if size < 64 {
        return Err(LIBE_FORMAT);
    }
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(size as usize).is_err() {
        return Err(LIBE_NOMEM);
    }
    buf.resize(size as usize, 0);
    if f.read_at(0, &mut buf) != Some(size as usize) {
        return Err(LIBE_IO);
    }
    drop(f);
    let mut lib = Lib::empty();
    elf_map(&buf, &mut lib)?;
    Ok(lib)
}

/// an anonymous private read-write mapping of len bytes -> its address or -errno
fn mmap_anon(len: usize) -> isize {
    // SAFETY: a new anonymous mapping (MAP_PRIVATE | MAP_ANONYMOUS, fd -1); nothing else is touched.
    unsafe { syscall6(SYS_MMAP, 0, len, (PROT_READ | PROT_WRITE) as usize, 0x22, usize::MAX, 0) }
}

/// mprotect(base + offset, len, prot) -> ok  [em_mprotect]
fn em_mprotect(base: usize, off: u64, len: u64, prot: u32) -> bool {
    // SAFETY: a page range inside our reservation.
    unsafe { syscall6(SYS_MPROTECT, base + off as usize, len as usize, prot as usize, 0, 0, 0) == 0 }
}

/// validate, map, relocate, protect; fill the Lib  [elf_map]
fn elf_map(b: &[u8], l: &mut Lib) -> Result<(), u32> {
    let size = b.len() as u64;
    let mut el = El::default();
    let fmt = LIBE_FORMAT;
    // ---- ELF header
    if b[..4] != *b"\x7FELF" || b[4] != 2 || b[5] != 1 || b[6] != 1 {
        return Err(fmt); // ELFCLASS64, little endian, version
    }
    if g16(b, 16) != Some(3) || g16(b, 18) != Some(62) || g32(b, 20) != Some(1) || g16(b, 54) != Some(56) {
        return Err(fmt); // ET_DYN, EM_X86_64, e_phentsize
    }
    let phnum = g16(b, 56).ok_or(fmt)? as u64;
    if phnum == 0 || phnum > 64 {
        return Err(fmt);
    }
    let phoff = g64(b, 32).ok_or(fmt)?;
    if phoff > size || size - phoff < phnum * 56 {
        return Err(fmt);
    }
    let mut span = 0u64; // page end of the last PT_LOAD
    // ---- program headers
    for k in 0..phnum {
        let ph = phoff + k * 56;
        let typ = g32(b, ph).ok_or(fmt)?;
        match typ {
            PT_LOAD => {
                let memsz = g64(b, ph + 40).ok_or(fmt)?;
                if memsz == 0 {
                    continue;
                }
                let filesz = g64(b, ph + 32).ok_or(fmt)?;
                let off = g64(b, ph + 8).ok_or(fmt)?;
                let va = g64(b, ph + 16).ok_or(fmt)?;
                if filesz > memsz || off > size || filesz > size - off || va > LIB_SPAN_MAX || memsz > LIB_SPAN_MAX - va {
                    return Err(fmt);
                }
                let flags = g32(b, ph + 4).ok_or(fmt)? as u64;
                if flags & (PF_W | PF_X) == PF_W | PF_X {
                    return Err(fmt); // W^X
                }
                if va & !4095 < span || el.nseg >= LH_SEGMAX {
                    return Err(fmt); // overlaps the previous segment's pages
                }
                el.segs[el.nseg] = Seg { va, end: va + memsz, off, fsz: filesz, flags };
                el.nseg += 1;
                span = (va + memsz + 4095) & !4095;
            }
            PT_DYNAMIC => {
                if el.dynsz != 0 {
                    return Err(fmt);
                }
                let off = g64(b, ph + 8).ok_or(fmt)?;
                let fsz = g64(b, ph + 32).ok_or(fmt)?;
                if off > size || fsz > size - off || fsz < 16 {
                    return Err(fmt);
                }
                el.dynoff = off;
                el.dynsz = fsz;
            }
            PT_INTERP | PT_TLS => return Err(LIBE_INIT),
            PT_GNU_RELRO => {
                el.relro_va = g64(b, ph + 16).ok_or(fmt)?;
                el.relro_sz = g64(b, ph + 40).ok_or(fmt)?;
            }
            _ => {}
        }
    }
    if el.nseg == 0 || el.dynsz == 0 {
        return Err(fmt);
    }
    // ---- map the span and copy the segments
    let base = mmap_anon(span as usize);
    if (base as usize) > (-4096isize) as usize {
        return Err(LIBE_MAP);
    }
    let base = base as usize;
    el.span = span;
    let r = elf_load(b, &mut el, base);
    if let Err(e) = r {
        // SAFETY: our reservation, unmapped once.
        unsafe { syscall6(SYS_MUNMAP, base, span as usize, 0, 0, 0, 0) };
        return Err(e);
    }
    // ---- the handle
    l.base = base;
    l.span = span as usize;
    l.os = 0;
    l.nseg = el.nseg as u32;
    l.nsym = el.nsym as u32;
    l.symtab = base + el.symtab as usize;
    l.strtab = base + el.strtab as usize;
    l.strsz = el.strsz as usize;
    for (k, s) in el.segs[..el.nseg].iter().enumerate() {
        l.segs[k] = LibSeg { start: base + s.va as usize, end: base + s.end as usize, flags: s.flags as u32 };
    }
    Ok(())
}

// the mapped part of elf_map: copy, dynamic tags, tables, relocations, protection
fn elf_load(b: &[u8], el: &mut El, base: usize) -> Result<(), u32> {
    let fmt = LIBE_FORMAT;
    let span = el.span;
    {
        // SAFETY: base is a fresh read-write anonymous mapping of span bytes, only reached
        // through this slice until the protection is set below.
        let m: &mut [u8] = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, span as usize) };
        for s in &el.segs[..el.nseg] {
            let (va, off, n) = (s.va as usize, s.off as usize, s.fsz as usize);
            m.get_mut(va..va + n).ok_or(fmt)?.copy_from_slice(b.get(off..off + n).ok_or(fmt)?);
        }
        // ---- dynamic section (read from the file copy)
        let mut p = el.dynoff;
        for _ in 0..el.dynsz >> 4 {
            let tag = g64(b, p).ok_or(fmt)?;
            let val = g64(b, p + 8).ok_or(fmt)?;
            p += 16;
            match tag {
                0 => break,
                DT_NEEDED => return Err(LIBE_DEPS),
                DT_INIT => return Err(LIBE_INIT),
                DT_INIT_ARRAYSZ | DT_PREINIT_ARRAYSZ if val != 0 => return Err(LIBE_INIT),
                DT_REL | DT_RELSZ | DT_RELRSZ if val != 0 => return Err(LIBE_RELOC),
                DT_RELR | DT_TEXTREL => return Err(LIBE_RELOC),
                DT_HASH => el.hash = val,
                DT_STRTAB => el.strtab = val,
                DT_SYMTAB => el.symtab = val,
                DT_RELA => el.rela = val,
                DT_RELASZ => el.relasz = val,
                DT_RELAENT => el.relaent = val,
                DT_STRSZ => el.strsz = val,
                DT_SYMENT => el.syment = val,
                DT_PLTRELSZ => el.pltrelsz = val,
                DT_PLTREL => el.pltrel = val,
                DT_JMPREL => el.jmprel = val,
                DT_GNU_HASH => el.gnuhash = val,
                _ => {}
            }
        }
        // ---- tables inside the span
        if el.strtab == 0 || el.strtab >= span || el.strsz > span - el.strtab {
            return Err(fmt);
        }
        if el.symtab == 0 || el.symtab >= span {
            return Err(fmt);
        }
        if (el.syment != 0 && el.syment != 24) || (el.relaent != 0 && el.relaent != 24) {
            return Err(fmt);
        }
        // ---- number of dynamic symbols: DT_HASH nchain, else the DT_GNU_HASH chains
        let nsym = if el.hash != 0 {
            if el.hash >= span || span - el.hash < 8 {
                return Err(fmt);
            }
            g32(m, el.hash + 4).ok_or(fmt)? as u64
        } else {
            gnu_count(m, el).ok_or(fmt)?
        };
        if nsym > SYM_MAX || nsym * 24 + el.symtab > span {
            return Err(fmt);
        }
        // lib_sym reads both tables after the protection is set: they must be readable
        let readable = |a: u64, n: u64| {
            el.segs[..el.nseg].iter().any(|s| s.flags & PF_R != 0 && a >= s.va && a <= s.end && n <= s.end - a)
        };
        if !readable(el.symtab, nsym * 24) || !readable(el.strtab, el.strsz) {
            return Err(fmt);
        }
        el.nsym = nsym;
        // ---- relocations
        elf_rela(m, el, base, el.rela, el.relasz)?;
        if el.pltrelsz != 0 {
            if el.pltrel != DT_RELA {
                return Err(LIBE_RELOC);
            }
            elf_rela(m, el, base, el.jmprel, el.pltrelsz)?;
        }
    }
    // ---- protection: segments, PROT_NONE gaps, RELRO read-only
    let mut cur = 0u64; // page cursor
    for k in 0..el.nseg {
        let s = el.segs[k];
        let start = s.va & !4095;
        if cur < start && !em_mprotect(base, cur, start - cur, 0) {
            return Err(LIBE_MAP);
        }
        let end = (s.end + 4095) & !4095;
        cur = end;
        let mut prot = 0;
        if s.flags & PF_R != 0 {
            prot |= PROT_READ;
        }
        if s.flags & PF_W != 0 {
            prot |= PROT_WRITE;
        }
        if s.flags & PF_X != 0 {
            prot |= PROT_EXEC;
        }
        el.segs[k].flags = prot as u64; // LIBA_* == PROT_* bits
        if !em_mprotect(base, start, end - start, prot) {
            return Err(LIBE_MAP);
        }
    }
    let (rv, rs) = (el.relro_va, el.relro_sz);
    if rs != 0 && rv < span && rs <= span - rv {
        let e = (rv + rs) & !4095; // never protect a partial page
        let s = rv & !4095;
        if e > s && !em_mprotect(base, s, e - s, PROT_READ) {
            return Err(LIBE_MAP);
        }
    }
    Ok(())
}

/// the number of dynamic symbols from DT_GNU_HASH, None when malformed / absent
/// [elf_gnu_count]
fn gnu_count(m: &[u8], el: &El) -> Option<u64> {
    let span = el.span;
    let gh = el.gnuhash;
    if gh == 0 || gh >= span || span - gh < 16 {
        return None;
    }
    let nb = g32(m, gh)? as u64; // nbuckets
    let symoff = g32(m, gh + 4)? as u64;
    let bloom = g32(m, gh + 8)? as u64;
    if nb == 0 || nb > SYM_MAX || bloom > SYM_MAX || symoff > SYM_MAX {
        return None;
    }
    let buckets = gh + 16 + bloom * 8;
    let chains = buckets + nb * 4;
    if chains > span {
        return None;
    }
    let mut maxb = 0u32;
    for i in 0..nb {
        maxb = maxb.max(g32(m, buckets + i * 4)?);
    }
    let mut d = maxb as u64;
    if d == 0 {
        return Some(symoff);
    }
    if d < symoff {
        return None;
    }
    loop {
        if d >= SYM_MAX {
            return None;
        }
        let e = chains + (d - symoff) * 4 + 4;
        if e > span {
            return None;
        }
        let v = g32(m, e - 4)?;
        d += 1;
        if v & 1 != 0 {
            return Some(d);
        }
    }
}

/// apply one RELA table of the mapped object  [elf_rela]
fn elf_rela(m: &mut [u8], el: &El, base: usize, va: u64, size: u64) -> Result<(), u32> {
    let fmt = LIBE_FORMAT;
    let span = el.span;
    if size == 0 {
        return Ok(());
    }
    if va == 0 || va > span || size > span - va || size % 24 != 0 {
        return Err(fmt);
    }
    for k in 0..size / 24 {
        let e = va + k * 24;
        let off = g64(m, e).ok_or(fmt)?; // r_offset
        let info = g64(m, e + 8).ok_or(fmt)?;
        let mut addend = g64(m, e + 16).ok_or(fmt)?;
        let typ = info as u32;
        let sym = info >> 32;
        if typ == R_NONE {
            continue;
        }
        if off >= span {
            return Err(fmt);
        }
        // the target lies inside a PT_LOAD
        if !el.segs[..el.nseg].iter().any(|s| off >= s.va && off + 8 <= s.end) {
            return Err(fmt);
        }
        let v = match typ {
            R_RELATIVE => (base as u64).wrapping_add(addend),
            R_64 | R_GLOB_DAT | R_JUMP_SLOT => {
                if typ != R_64 {
                    addend = 0; // GLOB_DAT / JUMP_SLOT: no addend
                }
                if sym == 0 || sym >= el.nsym {
                    return Err(LIBE_RELOC);
                }
                let s = el.symtab + sym * 24; // inside the span: nsym was checked
                let shndx = g16(m, s + 6).ok_or(fmt)?;
                if shndx == 0 {
                    return Err(LIBE_DEPS); // undefined: would need another object
                }
                let mut val = g64(m, s + 8).ok_or(fmt)?;
                if shndx != SHN_ABS {
                    if val >= span {
                        return Err(fmt);
                    }
                    val = val.wrapping_add(base as u64);
                }
                val.wrapping_add(addend)
            }
            _ => return Err(LIBE_RELOC),
        };
        m.get_mut(off as usize..off as usize + 8).ok_or(fmt)?.copy_from_slice(&v.to_le_bytes());
    }
    Ok(())
}

/// the address of the defined dynamic symbol name, or 0  [plat_lib_sym]
pub fn lib_sym(h: &Lib, name: &str) -> usize {
    let name = name.as_bytes();
    let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
    let (symtab, strtab, strsz) = (h.symtab, h.strtab, h.strsz);
    // SAFETY: .dynsym (nsym entries) and .dynstr (strsz bytes) lie inside the mapped span
    // (checked by elf_map); like the assembly the reads assume they are readable.
    unsafe {
        'syms: for i in 1..h.nsym as usize {
            // symbol 0 is the null symbol
            let s = symtab + i * 24;
            let shndx = (s as *const u8).add(6).cast::<u16>().read_unaligned() as u32;
            if shndx == 0 {
                continue;
            }
            let st_name = (s as *const u32).read_unaligned() as usize;
            if st_name >= strsz {
                continue;
            }
            let mut k = 0;
            loop {
                if st_name + k >= strsz {
                    continue 'syms;
                }
                let c = *((strtab + st_name + k) as *const u8);
                if c != name.get(k).copied().unwrap_or(0) {
                    continue 'syms;
                }
                if c == 0 {
                    break;
                }
                k += 1;
            }
            let v = (s as *const u8).add(8).cast::<u64>().read_unaligned() as usize;
            if shndx == SHN_ABS {
                return v;
            }
            if v >= h.span {
                return 0;
            }
            return h.base + v;
        }
    }
    0
}

/// munmap (lib_close, drop)  [plat_lib_close]
pub(crate) fn lib_unload(h: &mut Lib) {
    if h.base != 0 {
        // SAFETY: our reservation of span bytes, unmapped once.
        unsafe { syscall6(SYS_MUNMAP, h.base, h.span, 0, 0, 0, 0) };
        h.base = 0;
    }
}
