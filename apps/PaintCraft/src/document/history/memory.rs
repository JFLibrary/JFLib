// hist_mem -- PaintCraft history memory: the tile records, the 16 KB tile store, tile kernels,
// deflate on the worker pool, the spill file and the parallel entry loader
//                                                              [apps/PaintCraft/src/hist_mem.s]
//
//   hm_init()                                budget default (min(40 % of the RAM, 4 GB))
//   hm_fail() -> bool                        fault injection (hist_fail_after)
//   HistMem::tile_alloc() -> Option<Tile>    16 KB tile (hm_tile_alloc; contents undefined)
//   HistMem::tile_free(tile)                 (hm_tile_free)
//   tile_get / tile_put / tile_swap / tile_equal / tile_solid / tile_fill   tile kernels
//   HistMem::ht_load(e, i) -> 0 | ERR_*      make tile i TST_RAW
//   HistMem::ht_release(e, i)                free the payload of any state (record kept)
//   HistMem::ht_solidify(e, i, vw, vh)       TST_RAW tile with one colour -> TST_SOLID
//   HistMem::compress_entry(e) -> saved      deflate (level 1) the raw tiles (worker pool)
//   HistMem::spill_entry(e) -> 0 | -1        RAM tiles of a pixel entry to the spill file
//   HistMem::load_entry(e) -> 0 | ERR_*      every tile of a pixel entry into RAM (pool)
//   hist_budget_set / hist_budget / hist_mem_ram / hist_mem_disk / hist_temp_dir_set /
//   hist_fail_after / hist_spill_enable
//
// The asm's small-block allocator, 32-tile slabs and 1 MB deflate payload blocks are
// allocation strategies only: here an entry is a Box, a tile a boxed 16 KB array (a short
// free list keeps the allocator out of undo / redo) and a deflate payload a boxed slice.  What
// the history shows is kept exactly: the byte accounting (records HE_SIZE, raw tiles 16 KB,
// deflate payloads rounded up to 16, arrays, spill bytes) and every point where the asm asks
// hm_fail (an allocation a test can make fail).
//
// Z1: deflate / inflate run with one reusable state per pool thread (slot thr_self()) and
// deflate writes into a batch buffer of ZBATCH slots allocated once.
//
// Accounting: `ram` = RAM bytes of every entry of every document and of open captures; every
// change of a tile state updates the entry's `bytes` / `disk` and the totals together.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, Ordering};

use img_core::deflate::{zlib_deflate_to, DeflateState};
use img_core::inflate::{zlib_inflate_st, InflateState};
use img_core::{ERR_FAIL, ERR_IO, ERR_NOMEM};
use rt::sync::SpinLock;

use crate::app::constants::*;
use crate::document::history::{HistEntry, HK_PX};
use crate::app::Pn;

// ------------------------------------------------------------------ tiles (pn_hist.inc)

pub(crate) const TILE_BYTES: usize = 16384;
pub(crate) const TILE_PITCH: usize = 256;
pub(crate) const TST_RAW: u8 = 0; // 64 x 64 x 4 bytes in RAM (row pitch 256)
pub(crate) const TST_SOLID: u8 = 1; // every valid pixel = the colour
pub(crate) const TST_Z: u8 = 2; // zlib stream in RAM
pub(crate) const TST_DISK: u8 = 3; // in the spill file (zlib, or raw with HTF_STORED)
pub(crate) const HTF_STORED: u8 = 1; // the disk payload is the raw tile
pub(crate) const HTF_NOZ: u8 = 2; // deflate did not help: keep raw (spill raw)
pub(crate) const HT_SIZE: u64 = 32; // bytes of a tile record (accounting of tile arrays)
pub(crate) const HE_SIZE: u64 = 128; // bytes of an entry record (accounting)

/// a 16 KB tile: 64 rows of 256 bytes
pub(crate) type Tile = Box<[u8; TILE_BYTES]>;

/// the payload of a tile record (HT_PTR / HT_AUX)
#[derive(Default)]
pub(crate) enum Pay {
    #[default]
    None,
    Raw(Tile),
    Z(Box<[u8]>),
    Disk(u64),
}

/// a tile record of a pixel entry / capture (HT_*)
#[derive(Default)]
pub(crate) struct HTile {
    pub(crate) tx: u16,
    pub(crate) ty: u16,
    /// TST_*
    pub(crate) st: u8,
    /// HTF_*
    pub(crate) fl: u8,
    /// payload bytes (TST_Z, TST_DISK)
    pub(crate) len: u32,
    /// TST_SOLID: the colour (HT_PTR low 32 bits)
    pub(crate) color: u32,
    pub(crate) pay: Pay,
}

impl HTile {
    /// the raw tile (TST_RAW)
    pub(crate) fn raw(&self) -> &[u8; TILE_BYTES] {
        match &self.pay {
            Pay::Raw(t) => t,
            _ => panic!("history tile not raw"),
        }
    }
    pub(crate) fn raw_mut(&mut self) -> &mut [u8; TILE_BYTES] {
        match &mut self.pay {
            Pay::Raw(t) => t,
            _ => panic!("history tile not raw"),
        }
    }
}

// ------------------------------------------------------------------ constants

const ZMAX: usize = TILE_BYTES - 1024; // keep deflate output only when it saves >= 1 KB
const ZBATCH: usize = 256; // tiles per parallel_for batch
const ZC_CAP: usize = ZMAX + 16; // zlib_deflate_to capacity (larger = too big anyway)
const ZS_SLOTS: usize = plat::THR_MAX + 1; // per-thread states
const SP_EXT_MAX: usize = 4096; // free extents of the spill file
const BUDGET_MAX: u64 = 0x1_0000_0000; // 4 GB
const LD_PAR: i32 = 8; // load_entry: decode on the pool from this many tiles
// freed tiles kept for reuse: the asm carves tiles from 32-tile slabs, so a large undo / redo
// costs it one allocation per slab; a tile here is one page run of the global allocator, so a
// free list of up to 64 MB stands in for the slabs (dropped under memory pressure:
// hist_enforce over budget, hist_reclaim)
const FREE_TILES: usize = 4096;

/// a deflate payload's bytes in the accounting (the zpool rounds up to 16)
#[inline]
pub(crate) fn zlen(len: u32) -> u64 {
    (len as u64 + 15) & !15
}

// ------------------------------------------------------------------ state

/// the spill file (g_sp_h: 0 not opened, -1 failed)
#[derive(Default)]
enum SpFile {
    #[default]
    Closed,
    Failed,
    Open(plat::File),
}

/// the state of hist_mem.s (its g_* globals)
pub(crate) struct HistMem {
    /// g_hm_ram: RAM bytes of all histories (wrapping like the asm's u64)
    pub(crate) ram: u64,
    /// g_hm_disk: bytes in the spill file
    pub(crate) disk: u64,
    pub(crate) budget: u64,
    budget_def: u64,
    failn: i32,
    failing: bool,
    sp: SpFile,
    sp_dir: Option<String>,
    sp_on: bool,
    sp_end: u64,
    /// free extents of the spill file: offset, length
    sp_ext: Vec<(u64, u64)>,
    free: Vec<Tile>,
    zs: Vec<SpinLock<Option<Box<DeflateState>>>>,
    zi: Vec<SpinLock<Option<Box<InflateState>>>>,
    zc_out: Vec<SpinLock<Vec<u8>>>,
    iobuf: Vec<u8>,
}

impl Default for HistMem {
    fn default() -> HistMem {
        HistMem {
            ram: 0,
            disk: 0,
            budget: 0,
            budget_def: 0,
            failn: 0,
            failing: false,
            sp: SpFile::Closed,
            sp_dir: None,
            sp_on: false,
            sp_end: 0,
            sp_ext: Vec::new(),
            free: Vec::new(),
            zs: (0..ZS_SLOTS).map(|_| SpinLock::new(None)).collect(),
            zi: (0..ZS_SLOTS).map(|_| SpinLock::new(None)).collect(),
            zc_out: Vec::new(),
            iobuf: Vec::new(),
        }
    }
}

/// a zeroed tile
fn new_tile() -> Tile {
    match vec![0u8; TILE_BYTES].into_boxed_slice().try_into() {
        Ok(t) => t,
        Err(_) => unreachable!(),
    }
}

/// hm_init() -- budget = min(40 % of the RAM, 4 GB) (1 GB when unknown)
pub(crate) fn hm_init(p: &mut Pn) {
    let hm = &mut p.hm;
    let t = plat::mem_total();
    let b = if t == 0 { 0x4000_0000 } else { (t / 5 * 2).min(BUDGET_MAX) };
    hm.budget = b;
    hm.budget_def = b;
    hm.sp_on = true;
}

/// hm_fail() -> true when this allocation must fail (fault injection)
pub(crate) fn hm_fail(hm: &mut HistMem) -> bool {
    hm.fail()
}

impl HistMem {
    /// hm_fail()
    pub(crate) fn fail(&mut self) -> bool {
        if self.failing {
            return true;
        }
        if self.failn <= 0 {
            return false;
        }
        self.failn -= 1;
        if self.failn != 0 {
            return false;
        }
        self.failing = true;
        true
    }

    /// ACC e, d -- add a signed byte delta to the entry and to the RAM total
    #[inline]
    pub(crate) fn acc(&mut self, e: &mut HistEntry, d: i64) {
        e.bytes = e.bytes.wrapping_add(d as u64);
        self.ram = self.ram.wrapping_add(d as u64);
    }

    // -------------------------------------------------------------- tile pool

    /// hm_tile_alloc() -> a tile (contents undefined) | None
    pub(crate) fn tile_alloc(&mut self) -> Option<Tile> {
        if self.fail() {
            return None;
        }
        Some(self.free.pop().unwrap_or_else(new_tile))
    }

    /// hm_tile_free(tile)
    pub(crate) fn tile_free(&mut self, t: Tile) {
        if self.free.len() < FREE_TILES {
            self.free.push(t);
        }
    }

    /// the kept free tiles go back to the system (memory pressure)
    pub(crate) fn tiles_trim(&mut self) {
        self.free = Vec::new();
    }

    /// zp_alloc(len) -> a deflate payload | None (fault injection)
    fn zp_alloc(&mut self, len: usize) -> Option<Box<[u8]>> {
        if self.fail() {
            return None;
        }
        Some(vec![0u8; len].into_boxed_slice())
    }

    // -------------------------------------------------------------- spill file

    /// sp_write(buf) -> file offset | None
    fn sp_write(&mut self, buf: &[u8]) -> Option<u64> {
        if !self.sp_on {
            return None;
        }
        match self.sp {
            SpFile::Failed => return None,
            SpFile::Closed => match plat::File::open_temp(self.sp_dir.as_deref()) {
                Some(f) => self.sp = SpFile::Open(f),
                None => {
                    self.sp = SpFile::Failed;
                    return None;
                }
            },
            SpFile::Open(_) => {}
        }
        let len = buf.len() as u64;
        // first fit in the free extents, else append
        let off = match self.sp_ext.iter().position(|e| e.1 >= len) {
            Some(k) => {
                let o = self.sp_ext[k].0;
                self.sp_ext[k].0 += len;
                self.sp_ext[k].1 -= len;
                if self.sp_ext[k].1 == 0 {
                    self.sp_ext.swap_remove(k); // used up: the last one moves here
                }
                o
            }
            None => {
                let o = self.sp_end;
                self.sp_end += len;
                o
            }
        };
        let ok = match &self.sp {
            SpFile::Open(f) => f.write_at(off, buf) == Some(buf.len()),
            _ => false,
        };
        if ok {
            Some(off)
        } else {
            self.sp_free(off, len);
            None
        }
    }

    /// sp_read(off, buf) -> true when the whole buffer was read
    fn sp_read(&self, off: u64, buf: &mut [u8]) -> bool {
        match &self.sp {
            SpFile::Open(f) => f.read_at(off, buf) == Some(buf.len()),
            _ => false,
        }
    }

    /// sp_free(off, len) -- the file end shrinks when the extent is the last one; else the
    /// extent goes to the free list (dropped when the list is full)
    fn sp_free(&mut self, off: u64, len: u64) {
        if off.wrapping_add(len) != self.sp_end {
            if self.sp_ext.len() < SP_EXT_MAX {
                self.sp_ext.push((off, len));
            }
            return;
        }
        self.sp_end = off;
        // extents that now end at the file end shrink it further
        while let Some(k) = self.sp_ext.iter().position(|e| e.0 + e.1 == self.sp_end) {
            self.sp_end = self.sp_ext[k].0;
            self.sp_ext.swap_remove(k);
        }
    }

    /// le_free(run) -- free one run of spill file payloads (None / 0: nothing)
    fn le_free(&mut self, run: Option<(u64, u64)>) {
        if let Some((o, l)) = run {
            if l != 0 {
                self.sp_free(o, l);
            }
        }
    }

    // -------------------------------------------------------------- tile states

    /// ht_load(e, i) -> 0 | ERR_NOMEM | ERR_IO | ERR_FAIL -- bring tile i into TST_RAW state
    /// (payload of the old state released; accounting updated)
    pub(crate) fn ht_load(&mut self, e: &mut HistEntry, i: usize) -> i32 {
        let old = e.tiles[i].st;
        if old == TST_RAW {
            return 0;
        }
        let Some(mut t) = self.tile_alloc() else { return ERR_NOMEM };
        match old {
            TST_SOLID => tile_fill(&mut t[..], e.tiles[i].color),
            TST_Z => {
                let len = e.tiles[i].len;
                let ok = match &e.tiles[i].pay {
                    Pay::Z(z) => inflate_slot(&self.zi, &z[..len as usize], &mut t),
                    _ => false,
                };
                if !ok {
                    self.tile_free(t);
                    return ERR_FAIL;
                }
                self.acc(e, -(zlen(len) as i64));
            }
            TST_DISK => {
                let Pay::Disk(off) = e.tiles[i].pay else {
                    self.tile_free(t);
                    return ERR_FAIL;
                };
                let len = e.tiles[i].len;
                if e.tiles[i].fl & HTF_STORED != 0 {
                    if !self.sp_read(off, &mut t[..]) {
                        self.tile_free(t);
                        return ERR_IO;
                    }
                } else {
                    if len as usize > TILE_BYTES {
                        self.tile_free(t);
                        return ERR_FAIL;
                    }
                    let mut buf = core::mem::take(&mut self.iobuf);
                    buf.resize(TILE_BYTES, 0);
                    if !self.sp_read(off, &mut buf[..len as usize]) {
                        self.iobuf = buf;
                        self.tile_free(t);
                        return ERR_IO;
                    }
                    let ok = inflate_slot(&self.zi, &buf[..len as usize], &mut t);
                    self.iobuf = buf;
                    if !ok {
                        self.tile_free(t);
                        return ERR_FAIL;
                    }
                }
                self.sp_free(off, len as u64);
                e.disk = e.disk.wrapping_sub(len as u64);
                self.disk = self.disk.wrapping_sub(len as u64);
            }
            _ => {
                self.tile_free(t);
                return ERR_FAIL;
            }
        }
        let ht = &mut e.tiles[i];
        ht.st = TST_RAW;
        ht.fl &= !(HTF_STORED | HTF_NOZ);
        ht.len = 0;
        ht.pay = Pay::Raw(t);
        self.acc(e, TILE_BYTES as i64);
        e.nraw += 1;
        if old != TST_Z {
            e.nmem += 1; // a deflated tile already held RAM
        }
        0
    }

    /// ht_release(e, i) -- free the payload of the tile (any state); the record stays
    /// (state TST_SOLID colour 0 afterwards)
    pub(crate) fn ht_release(&mut self, e: &mut HistEntry, i: usize) {
        let ht = &mut e.tiles[i];
        let (st, fl, len) = (ht.st, ht.fl, ht.len);
        let pay = core::mem::take(&mut ht.pay);
        ht.st = TST_SOLID;
        ht.fl = 0;
        ht.len = 0;
        ht.color = 0;
        match (st, pay) {
            (TST_RAW, Pay::Raw(t)) => {
                self.tile_free(t);
                self.acc(e, -(TILE_BYTES as i64));
                e.nmem -= 1;
                if fl & HTF_NOZ == 0 {
                    e.nraw -= 1;
                }
            }
            (TST_Z, _) => {
                self.acc(e, -(zlen(len) as i64));
                e.nmem -= 1;
            }
            (TST_DISK, Pay::Disk(off)) => {
                self.sp_free(off, len as u64);
                e.disk = e.disk.wrapping_sub(len as u64);
                self.disk = self.disk.wrapping_sub(len as u64);
            }
            _ => {}
        }
    }

    /// ht_solidify(e, i, vw, vh) -- a TST_RAW tile whose valid pixels are one colour becomes
    /// TST_SOLID (its 16 KB are freed)
    pub(crate) fn ht_solidify(&mut self, e: &mut HistEntry, i: usize, vw: i32, vh: i32) {
        if e.tiles[i].st != TST_RAW {
            return;
        }
        if let Some(c) = tile_solid(&e.tiles[i].raw()[..], vw, vh) {
            self.ht_release(e, i);
            e.tiles[i].color = c;
        }
    }

    // -------------------------------------------------------------- deflate on the pool

    /// hm_compress_entry(e) -> RAM bytes saved -- deflate (level 1) every raw tile of a pixel
    /// entry on the worker pool; tiles that do not shrink by 1 KB keep TST_RAW (HTF_NOZ)
    pub(crate) fn compress_entry(&mut self, e: &mut HistEntry) -> u64 {
        let mut saved = 0u64;
        if e.kind != HK_PX || e.nraw == 0 {
            return 0;
        }
        if self.zc_out.is_empty() {
            self.zc_out = (0..ZBATCH).map(|_| SpinLock::new(vec![0u8; ZC_CAP])).collect();
        }
        let res: Vec<AtomicI64> = (0..ZBATCH).map(|_| AtomicI64::new(i64::MIN)).collect();
        let mut base = 0usize;
        while base < e.tiles.len() {
            let n = (e.tiles.len() - base).min(ZBATCH);
            for r in &res[..n] {
                r.store(i64::MIN, Ordering::Relaxed);
            }
            {
                let tiles = &e.tiles[base..base + n];
                let (zs, out, res) = (&self.zs, &self.zc_out, &res);
                let item = |i: usize, _: &plat::JobCtl| {
                    let t = &tiles[i];
                    if t.st != TST_RAW || t.fl & HTF_NOZ != 0 {
                        return;
                    }
                    let Pay::Raw(raw) = &t.pay else { return };
                    let k = plat::thr_self().min(ZS_SLOTS - 1);
                    let mut sg = zs[k].lock();
                    let st = sg.get_or_insert_with(DeflateState::new);
                    let mut o = out[i].lock();
                    let r = match zlib_deflate_to(Some(&mut **st), &raw[..], 1, &mut o[..ZC_CAP]) {
                        Ok(m) => m as i64,
                        Err(er) => er as i64,
                    };
                    res[i].store(r, Ordering::Relaxed);
                };
                let _ = plat::parallel_for(n, &item);
            }
            // adopt the results (UI thread)
            for i in 0..n {
                let r = res[i].load(Ordering::Relaxed);
                if r == i64::MIN || r == ERR_NOMEM as i64 {
                    continue; // not deflated / no state: retried later
                }
                let ti = base + i;
                if r < 0 || r as usize > ZMAX {
                    e.tiles[ti].fl |= HTF_NOZ; // (errors count as too big)
                    e.nraw -= 1;
                    continue;
                }
                let len = r as usize;
                let Some(mut z) = self.zp_alloc(len) else { continue }; // stays raw
                z.copy_from_slice(&self.zc_out[i].lock()[..len]);
                let old = core::mem::replace(&mut e.tiles[ti].pay, Pay::Z(z));
                e.tiles[ti].st = TST_Z;
                e.tiles[ti].len = len as u32;
                if let Pay::Raw(t) = old {
                    self.tile_free(t);
                }
                e.nraw -= 1;
                let d = zlen(len as u32) as i64 - TILE_BYTES as i64; // negative: bytes saved
                self.acc(e, d);
                saved = saved.wrapping_sub(d as u64);
            }
            base += n;
        }
        saved
    }

    // tile i becomes TST_DISK at off (len set): the disk bytes are counted
    fn to_disk(&mut self, e: &mut HistEntry, i: usize, off: u64) {
        let ht = &mut e.tiles[i];
        ht.st = TST_DISK;
        ht.pay = Pay::Disk(off);
        let len = ht.len as u64;
        e.disk = e.disk.wrapping_add(len);
        self.disk = self.disk.wrapping_add(len);
    }

    // a RAM tile (TST_RAW / TST_Z) leaves RAM for the spill file: its accounting
    fn ram_out(&mut self, e: &mut HistEntry, i: usize) {
        match e.tiles[i].st {
            TST_RAW => {
                if let Pay::Raw(t) = core::mem::take(&mut e.tiles[i].pay) {
                    self.tile_free(t);
                }
                self.acc(e, -(TILE_BYTES as i64));
                e.nmem -= 1;
                if e.tiles[i].fl & HTF_NOZ == 0 {
                    e.nraw -= 1;
                }
                e.tiles[i].fl = HTF_STORED;
                e.tiles[i].len = TILE_BYTES as u32;
            }
            _ => {
                let len = e.tiles[i].len;
                e.tiles[i].pay = Pay::None;
                self.acc(e, -(zlen(len) as i64));
                e.nmem -= 1;
                e.tiles[i].fl = 0;
            }
        }
    }

    /// sp_tiles(e) -> 0 | -1 -- spill_entry tile by tile (no gather buffer); -1 when the file
    /// cannot be written (tiles written so far stay there)
    fn sp_tiles(&mut self, e: &mut HistEntry) -> i32 {
        if e.kind != HK_PX {
            return 0;
        }
        for i in 0..e.tiles.len() {
            let off = match (e.tiles[i].st, &e.tiles[i].pay) {
                (TST_Z, Pay::Z(z)) => {
                    let z = z[..e.tiles[i].len as usize].to_vec();
                    self.sp_write(&z)
                }
                (TST_RAW, Pay::Raw(t)) => {
                    let t = t.to_vec();
                    self.sp_write(&t)
                }
                _ => continue,
            };
            let Some(off) = off else { return -1 };
            self.ram_out(e, i);
            self.to_disk(e, i, off);
        }
        0
    }

    /// hm_spill_entry(e) -> 0 | -1 -- move every RAM tile (TST_Z, TST_RAW) of a pixel entry
    /// to the spill file with one write (gathered in a temporary buffer); -1 when the file
    /// cannot be written (nothing changed then)
    pub(crate) fn spill_entry(&mut self, e: &mut HistEntry) -> i32 {
        if e.kind != HK_PX {
            return 0;
        }
        let mut total = 0usize;
        for t in &e.tiles {
            match t.st {
                TST_RAW => total += TILE_BYTES,
                TST_Z => total += t.len as usize,
                _ => {}
            }
        }
        if total == 0 {
            return 0;
        }
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(total).is_err() {
            return self.sp_tiles(e); // no gather buffer: tile by tile
        }
        for t in &e.tiles {
            match (&t.pay, t.st) {
                (Pay::Raw(r), TST_RAW) => buf.extend_from_slice(&r[..]),
                (Pay::Z(z), TST_Z) => buf.extend_from_slice(&z[..t.len as usize]),
                _ => {}
            }
        }
        let Some(base) = self.sp_write(&buf) else { return -1 };
        drop(buf);
        let mut run = 0u64; // every RAM tile is at base + running offset
        for i in 0..e.tiles.len() {
            if e.tiles[i].st != TST_RAW && e.tiles[i].st != TST_Z {
                continue;
            }
            self.ram_out(e, i);
            let len = e.tiles[i].len as u64;
            self.to_disk(e, i, base + run);
            run += len;
        }
        0
    }

    /// hm_load_entry(e) -> 0 | ERR_NOMEM | ERR_IO | ERR_FAIL -- every tile of a pixel entry
    /// into TST_RAW state.  From LD_PAR tiles on: destination tiles first, spill file payloads
    /// read in runs, decoding (inflate / fill / copy) on the worker pool; on failure nothing
    /// changes.
    pub(crate) fn load_entry(&mut self, e: &mut HistEntry) -> i32 {
        let mut nload = 0i32;
        let mut dbytes = 0usize;
        for t in &e.tiles {
            if t.st != TST_RAW {
                nload += 1;
                if t.st == TST_DISK {
                    dbytes += t.len as usize;
                }
            }
        }
        if nload == 0 {
            return 0;
        }
        if nload < LD_PAR {
            for i in 0..e.tiles.len() {
                let r = self.ht_load(e, i);
                if r != 0 {
                    return r;
                }
            }
            return 0;
        }
        if self.fail() {
            return ERR_NOMEM;
        }
        let n = e.tiles.len();
        // side records: the new raw tile, the scratch offset of a spilled payload
        let mut dst: Vec<Option<Tile>> = (0..n).map(|_| None).collect();
        let mut soff: Vec<usize> = vec![0; n];
        let mut scratch = vec![0u8; dbytes];
        let mut sp = 0usize;
        let mut run: Option<(u64, usize, usize)> = None; // file offset, scratch offset, length
        let mut err = 0;
        for i in 0..n {
            let st = e.tiles[i].st;
            if st == TST_RAW {
                continue;
            }
            match self.tile_alloc() {
                Some(t) => dst[i] = Some(t),
                None => {
                    err = ERR_NOMEM;
                    break;
                }
            }
            if st == TST_DISK {
                let Pay::Disk(off) = e.tiles[i].pay else { continue };
                let len = e.tiles[i].len as usize;
                soff[i] = sp;
                match run {
                    // does it continue the current run?
                    Some((ro, rs, rl)) if ro + rl as u64 == off => run = Some((ro, rs, rl + len)),
                    _ => {
                        if let Some((ro, rs, rl)) = run {
                            if rl != 0 && !self.sp_read(ro, &mut scratch[rs..rs + rl]) {
                                err = ERR_IO;
                                break;
                            }
                        }
                        run = Some((off, sp, len));
                    }
                }
                sp += len;
            }
        }
        if err == 0 {
            if let Some((ro, rs, rl)) = run {
                if rl != 0 && !self.sp_read(ro, &mut scratch[rs..rs + rl]) {
                    err = ERR_IO;
                }
            }
        }
        if err == 0 {
            // decode on the pool
            let slots: Vec<SpinLock<Option<Tile>>> = dst.drain(..).map(SpinLock::new).collect();
            let bad: Vec<AtomicI64> = (0..n).map(|_| AtomicI64::new(0)).collect();
            {
                let (tiles, zi, scr, soff, slots, bad) = (&e.tiles, &self.zi, &scratch, &soff, &slots, &bad);
                let item = |i: usize, _: &plat::JobCtl| {
                    let mut g = slots[i].lock();
                    let Some(d) = g.as_mut() else { return };
                    let t = &tiles[i];
                    let ok = match (t.st, &t.pay) {
                        (TST_SOLID, _) => {
                            tile_fill(&mut d[..], t.color);
                            true
                        }
                        (TST_DISK, _) if t.fl & HTF_STORED != 0 => {
                            d.copy_from_slice(&scr[soff[i]..soff[i] + TILE_BYTES]);
                            true
                        }
                        (TST_DISK, _) => inflate_slot(zi, &scr[soff[i]..soff[i] + t.len as usize], d),
                        (TST_Z, Pay::Z(z)) => inflate_slot(zi, &z[..t.len as usize], d),
                        _ => false,
                    };
                    if !ok {
                        bad[i].store(1, Ordering::Relaxed);
                    }
                };
                let _ = plat::parallel_for(n, &item);
            }
            dst = slots.iter().map(|s| s.lock().take()).collect();
            if (0..n).any(|i| dst[i].is_some() && bad[i].load(Ordering::Relaxed) != 0) {
                err = ERR_FAIL;
            }
        }
        if err != 0 {
            // give the destination tiles back
            for t in dst.into_iter().flatten() {
                self.tile_free(t);
            }
            return err;
        }
        // adopt: release the old payloads (spill file runs freed at once), then TST_RAW
        let mut frun: Option<(u64, u64)> = None;
        for (i, d) in dst.iter_mut().enumerate() {
            let Some(t) = d.take() else { continue };
            let old = e.tiles[i].st;
            let len = e.tiles[i].len;
            if old == TST_Z {
                self.acc(e, -(zlen(len) as i64));
            } else if old == TST_DISK {
                e.disk = e.disk.wrapping_sub(len as u64);
                self.disk = self.disk.wrapping_sub(len as u64);
                if let Pay::Disk(off) = e.tiles[i].pay {
                    match frun {
                        Some((ro, rl)) if ro + rl == off => frun = Some((ro, rl + len as u64)),
                        _ => {
                            self.le_free(frun);
                            frun = Some((off, len as u64));
                        }
                    }
                }
            }
            let ht = &mut e.tiles[i];
            ht.pay = Pay::Raw(t);
            ht.st = TST_RAW;
            ht.fl = 0;
            ht.len = 0;
            self.acc(e, TILE_BYTES as i64);
            e.nraw += 1;
            if old != TST_Z {
                e.nmem += 1;
            }
        }
        self.le_free(frun);
        0
    }
}

/// hm_inflate_tile(src, dst) -> true when the payload inflated to exactly TILE_BYTES (the
/// calling thread's inflate state)
fn inflate_slot(zi: &[SpinLock<Option<Box<InflateState>>>], src: &[u8], dst: &mut [u8; TILE_BYTES]) -> bool {
    let k = plat::thr_self().min(ZS_SLOTS - 1);
    let mut g = zi[k].lock();
    let st = g.get_or_insert_with(InflateState::new);
    matches!(zlib_inflate_st(Some(&mut **st), src, &mut dst[..]), Ok(TILE_BYTES))
}

// ------------------------------------------------------------------ tile kernels
// Rows: tile pitch 256 bytes, layer rows `stride`; vw * 4 bytes per row, vh rows (1..64).
// `px` starts at the tile's first pixel in the layer.

/// tile_get(tile, px, stride, vw, vh) -- layer region -> tile; edge tiles: padding zeroed
pub(crate) fn tile_get(tile: &mut [u8], px: &[u8], stride: i32, vw: i32, vh: i32) {
    if vw < TILE || vh < TILE {
        tile[..TILE_BYTES].fill(0);
    }
    let (n, s) = (vw as usize * 4, stride as usize);
    for r in 0..vh as usize {
        tile[r * TILE_PITCH..r * TILE_PITCH + n].copy_from_slice(&px[r * s..r * s + n]);
    }
}

/// tile_put(tile, px, stride, vw, vh) -- tile -> layer region
pub(crate) fn tile_put(tile: &[u8], px: &mut [u8], stride: i32, vw: i32, vh: i32) {
    let (n, s) = (vw as usize * 4, stride as usize);
    for r in 0..vh as usize {
        px[r * s..r * s + n].copy_from_slice(&tile[r * TILE_PITCH..r * TILE_PITCH + n]);
    }
}

/// tile_swap(tile, px, stride, vw, vh) -- exchange tile and layer region
pub(crate) fn tile_swap(tile: &mut [u8], px: &mut [u8], stride: i32, vw: i32, vh: i32) {
    let (n, s) = (vw as usize * 4, stride as usize);
    for r in 0..vh as usize {
        px[r * s..r * s + n].swap_with_slice(&mut tile[r * TILE_PITCH..r * TILE_PITCH + n]);
    }
}

/// tile_equal(tile, px, stride, vw, vh) -> true when the valid region is unchanged
pub(crate) fn tile_equal(tile: &[u8], px: &[u8], stride: i32, vw: i32, vh: i32) -> bool {
    let (n, s) = (vw as usize * 4, stride as usize);
    (0..vh as usize).all(|r| tile[r * TILE_PITCH..r * TILE_PITCH + n] == px[r * s..r * s + n])
}

// 16 pixels of one colour
#[inline]
fn pat64(c: [u8; 4]) -> [u8; 64] {
    let mut p = [0u8; 64];
    for q in p.chunks_exact_mut(4) {
        q.copy_from_slice(&c);
    }
    p
}

/// tile_solid(tile, vw, vh) -> Some(colour) when every valid pixel is the same
pub(crate) fn tile_solid(tile: &[u8], vw: i32, vh: i32) -> Option<u32> {
    let c = [tile[0], tile[1], tile[2], tile[3]];
    let pat = pat64(c);
    let n = vw as usize * 4;
    for r in 0..vh as usize {
        let row = &tile[r * TILE_PITCH..r * TILE_PITCH + n];
        let mut ch = row.chunks_exact(64);
        if !ch.all(|q| q == &pat[..]) || ch.remainder() != &pat[..ch.remainder().len()] {
            return None;
        }
    }
    Some(u32::from_le_bytes(c))
}

/// tile_fill(tile, argb) -- all 4096 pixels
pub(crate) fn tile_fill(tile: &mut [u8], argb: u32) {
    let pat = pat64(argb.to_le_bytes());
    for q in tile[..TILE_BYTES].chunks_exact_mut(64) {
        q.copy_from_slice(&pat);
    }
}

// ------------------------------------------------------------------ settings

/// hist_budget_set(bytes) -- 0 = the default
pub(crate) fn hist_budget_set(p: &mut Pn, bytes: u64) {
    p.hm.budget = if bytes == 0 { p.hm.budget_def } else { bytes };
}

/// hist_budget() -> bytes
pub(crate) fn hist_budget(hm: &HistMem) -> u64 {
    hm.budget
}

/// hist_mem_ram() -> bytes of all histories in RAM
pub(crate) fn hist_mem_ram(hm: &HistMem) -> u64 {
    hm.ram
}

/// hist_mem_disk() -> bytes of all histories in the spill file
pub(crate) fn hist_mem_disk(hm: &HistMem) -> u64 {
    hm.disk
}

/// hist_temp_dir_set(path | None) -- directory of the spill file (used when it is created)
pub(crate) fn hist_temp_dir_set(hm: &mut HistMem, path: Option<&str>) {
    hm.sp_dir = path.map(String::from);
}

/// hist_fail_after(n) -- from the n-th next history allocation on, every one fails (0 = off)
pub(crate) fn hist_fail_after(hm: &mut HistMem, n: u32) {
    hm.failn = n as i32;
    hm.failing = false;
}

/// hist_spill_enable(on) -- false makes every spill fail (tests)
pub(crate) fn hist_spill_enable(hm: &mut HistMem, on: bool) {
    hm.sp_on = on;
}
