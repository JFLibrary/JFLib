// vk_mem -- the device memory sub-allocator of the Vulkan backend
//
//   Allocator::new(props)                      the memory types of the adapter
//   a.alloc(f, dev, &req, kind, linear) -> Result<Alloc>    a range of a block (or a block of
//                                               its own for large requests)
//   a.free(f, dev, &alloc)                     the range back (blocks are released when empty)
//   a.destroy_all(f, dev)                      every block (device teardown)
//
// One pool per (memory type, linear / optimal): buffers and optimally tiled images never share
// a block, so bufferImageGranularity does not matter.  The blocks of a pool grow: 4 MiB of device
// memory or 1 MiB of host memory first, each new block twice the last, up to 64 MiB / 16 MiB (at
// most an eighth of the heap): a program with one small window reserves a few MiB, not 80.  A
// request above an eighth of the largest block gets a block of its own.  Each block keeps a free
// list sorted by offset (first fit, neighbours merged on free); one empty block per pool is kept
// for reuse, further empty ones are released.
// Host-visible blocks are mapped once for their lifetime (Alloc::ptr).
//
// Memory kinds: Device = DEVICE_LOCAL (preferably not host-visible: the BAR window stays
// free); Upload = HOST_VISIBLE | HOST_COHERENT, preferably not device-local and uncached
// (write-combined system memory for staging); Readback = HOST_VISIBLE, preferably
// HOST_CACHED and coherent (non-coherent ranges are invalidated before reads, vk_res.rs).

use alloc::vec::Vec;

use crate::vk_ffi as vk;
use crate::{GpuError, MemoryKind, Result};

/// a sub-allocated range: memory, offset, size, mapped address (0 = not host-visible)
#[derive(Clone, Copy)]
pub(crate) struct Alloc {
    pub mem: vk::Handle,
    pub offset: u64,
    pub size: u64,
    pub ptr: usize,
    pub coherent: bool,
    pool: u32,
    block: u32,
}

struct Block {
    mem: vk::Handle,
    size: u64,
    ptr: usize,
    free: Vec<(u64, u64)>, // (offset, size), sorted by offset
    live: u32,
    dedicated: bool,
}

struct Pool {
    type_index: u32,
    linear: bool,
    block_size: u64,
    next_block: u64,
    blocks: Vec<Option<Block>>,
}

pub(crate) struct Allocator {
    pools: Vec<Pool>,
    props: vk::PhysicalDeviceMemoryProperties,
    /// bytes of device memory allocated from the driver (all blocks)
    pub reserved: u64,
}

const MIB: u64 = 1 << 20;
const AVOID: u32 = vk::MEMORY_PROPERTY_PROTECTED_BIT | vk::MEMORY_PROPERTY_LAZILY_ALLOCATED_BIT
    | vk::MEMORY_PROPERTY_DEVICE_COHERENT_BIT_AMD;

fn align_up(v: u64, a: u64) -> u64 {
    (v + a - 1) & !(a - 1)
}

impl Allocator {
    pub fn new(props: vk::PhysicalDeviceMemoryProperties) -> Allocator {
        Allocator { pools: Vec::new(), props, reserved: 0 }
    }

    /// the memory type for kind among the allowed bits (None: no fitting type)
    pub fn pick_type(&self, bits: u32, kind: MemoryKind) -> Option<u32> {
        let (need, prefer, avoid) = match kind {
            MemoryKind::Device => (vk::MEMORY_PROPERTY_DEVICE_LOCAL_BIT, 0, vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT),
            MemoryKind::Upload => (
                vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT | vk::MEMORY_PROPERTY_HOST_COHERENT_BIT,
                0,
                vk::MEMORY_PROPERTY_DEVICE_LOCAL_BIT | vk::MEMORY_PROPERTY_HOST_CACHED_BIT,
            ),
            MemoryKind::Readback => (
                vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT,
                vk::MEMORY_PROPERTY_HOST_CACHED_BIT | vk::MEMORY_PROPERTY_HOST_COHERENT_BIT,
                vk::MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            ),
        };
        let mut best: Option<(i32, u32)> = None;
        for pass in 0..2 {
            // pass 1: Device memory falls back to any type (UMA without DEVICE_LOCAL), Upload to
            // non-coherent host memory (flushed by the writer)
            let need = if pass == 0 {
                need
            } else {
                match kind {
                    MemoryKind::Device => 0,
                    _ => vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT,
                }
            };
            for i in 0..self.props.memory_type_count.min(32) {
                let fl = self.props.memory_types[i as usize].property_flags;
                if bits & (1 << i) == 0 || fl & need != need || fl & AVOID != 0 {
                    continue;
                }
                let score = 4 * (fl & prefer).count_ones() as i32 - 8 * (fl & avoid).count_ones() as i32;
                if best.is_none_or(|(s, _)| score > s) {
                    best = Some((score, i));
                }
            }
            if best.is_some() {
                break;
            }
        }
        best.map(|(_, i)| i)
    }

    fn pool(&mut self, type_index: u32, linear: bool) -> usize {
        if let Some(i) = self.pools.iter().position(|p| p.type_index == type_index && p.linear == linear) {
            return i;
        }
        let t = self.props.memory_types[type_index as usize];
        let heap = self.props.memory_heaps[t.heap_index as usize].size;
        let host = t.property_flags & vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT != 0;
        let (first, base) = if host { (MIB, 16 * MIB) } else { (4 * MIB, 64 * MIB) };
        let block_size = base.min((heap / 8) & !(MIB - 1)).max(MIB);
        self.pools.push(Pool { type_index, linear, block_size, next_block: first.min(block_size), blocks: Vec::new() });
        self.pools.len() - 1
    }

    pub fn alloc(&mut self, f: &vk::DeviceFns, dev: vk::Device, req: &vk::MemoryRequirements, kind: MemoryKind,
                 linear: bool) -> Result<Alloc> {
        let ti = self.pick_type(req.memory_type_bits, kind).ok_or(GpuError::Unsupported("no fitting memory type"))?;
        let mt = self.props.memory_types[ti as usize];
        if req.size > self.props.memory_heaps[mt.heap_index as usize].size {
            return Err(GpuError::OutOfMemory); // larger than the heap: never asked of the driver
        }
        let fl = mt.property_flags;
        let host = fl & vk::MEMORY_PROPERTY_HOST_VISIBLE_BIT != 0;
        let coherent = fl & vk::MEMORY_PROPERTY_HOST_COHERENT_BIT != 0;
        let pi = self.pool(ti, linear);
        let align = req.alignment.max(16);
        let size = align_up(req.size.max(1), 16);
        let bs = self.pools[pi].block_size;
        // a block of its own above an eighth of the largest block: large textures and buffers
        // go back to the driver as soon as they are freed
        if size <= bs / 8 {
            let pool = &mut self.pools[pi];
            for (bi, b) in pool.blocks.iter_mut().enumerate() {
                let Some(b) = b else { continue };
                if b.dedicated {
                    continue;
                }
                if let Some(off) = take(&mut b.free, size, align) {
                    b.live += 1;
                    let ptr = if b.ptr != 0 { b.ptr + off as usize } else { 0 };
                    return Ok(Alloc { mem: b.mem, offset: off, size, ptr, coherent, pool: pi as u32, block: bi as u32 });
                }
            }
        }
        // a new block (shared, or a dedicated one of exactly this size)
        let dedicated = size > bs / 8;
        let bsize = if dedicated {
            size
        } else {
            // the pool's next size, large enough for this request with its alignment
            let pool = &mut self.pools[pi];
            let b = pool.next_block;
            pool.next_block = (b * 2).min(bs);
            b.max(size + align).min(bs)
        };
        let mem = self.raw_alloc(f, dev, bsize, ti)?;
        let mut ptr = 0usize;
        if host {
            let mut p: *mut core::ffi::c_void = core::ptr::null_mut();
            // SAFETY: mem is a fresh host-visible allocation of bsize bytes.
            let r = unsafe { (f.map_memory)(dev, mem, 0, vk::WHOLE_SIZE, 0, &mut p) };
            if r != vk::SUCCESS {
                // SAFETY: nothing uses the memory yet.
                unsafe { (f.free_memory)(dev, mem, core::ptr::null()) };
                self.reserved -= bsize;
                return Err(crate::vk_inst::err(r));
            }
            ptr = p as usize;
        }
        let mut free = Vec::new();
        if !dedicated && bsize > size {
            free.push((size, bsize - size));
        }
        let b = Block { mem, size: bsize, ptr, free, live: 1, dedicated };
        let pool = &mut self.pools[pi];
        let bi = match pool.blocks.iter().position(|b| b.is_none()) {
            Some(i) => {
                pool.blocks[i] = Some(b);
                i
            }
            None => {
                pool.blocks.push(Some(b));
                pool.blocks.len() - 1
            }
        };
        Ok(Alloc { mem, offset: 0, size, ptr, coherent, pool: pi as u32, block: bi as u32 })
    }

    fn raw_alloc(&mut self, f: &vk::DeviceFns, dev: vk::Device, size: u64, ti: u32) -> Result<vk::Handle> {
        let info = vk::MemoryAllocateInfo {
            s_type: vk::STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO,
            allocation_size: size,
            memory_type_index: ti,
            ..vk::Zero::zero()
        };
        let mut mem = 0;
        // SAFETY: plain allocation call with a valid info structure.
        let r = unsafe { (f.allocate_memory)(dev, &info, core::ptr::null(), &mut mem) };
        if r != vk::SUCCESS {
            return Err(crate::vk_inst::err(r));
        }
        self.reserved += size;
        Ok(mem)
    }

    pub fn free(&mut self, f: &vk::DeviceFns, dev: vk::Device, a: &Alloc) {
        let pool = &mut self.pools[a.pool as usize];
        let Some(b) = pool.blocks[a.block as usize].as_mut() else { return };
        b.live -= 1;
        if !b.dedicated {
            give(&mut b.free, a.offset, a.size);
        }
        if b.live > 0 {
            return;
        }
        // an empty block: keep one shared block per pool, release the rest
        let keep = !b.dedicated
            && !pool.blocks.iter().enumerate().any(|(i, o)| {
                i != a.block as usize && o.as_ref().is_some_and(|o| o.live == 0 && !o.dedicated)
            });
        if keep {
            return;
        }
        let b = pool.blocks[a.block as usize].take().unwrap_or_else(|| unreachable!());
        self.reserved -= b.size;
        // SAFETY: no range of the block is in use (live == 0); unmapped implicitly by the free.
        unsafe { (f.free_memory)(dev, b.mem, core::ptr::null()) };
    }

    pub fn destroy_all(&mut self, f: &vk::DeviceFns, dev: vk::Device) {
        for p in self.pools.drain(..) {
            for b in p.blocks.into_iter().flatten() {
                // SAFETY: device teardown: the GPU is idle and every resource destroyed.
                unsafe { (f.free_memory)(dev, b.mem, core::ptr::null()) };
            }
        }
        self.reserved = 0;
    }
}

/// first fit of size bytes at align in a sorted free list -> the offset
fn take(free: &mut Vec<(u64, u64)>, size: u64, align: u64) -> Option<u64> {
    for i in 0..free.len() {
        let (off, len) = free[i];
        let a = align_up(off, align);
        if a + size > off + len {
            continue;
        }
        let tail = off + len - (a + size);
        match (a > off, tail > 0) {
            (false, false) => {
                free.remove(i);
            }
            (false, true) => free[i] = (a + size, tail),
            (true, false) => free[i] = (off, a - off),
            (true, true) => {
                free[i] = (off, a - off);
                free.insert(i + 1, (a + size, tail));
            }
        }
        return Some(a);
    }
    None
}

/// return a range to a sorted free list, merged with its neighbours
fn give(free: &mut Vec<(u64, u64)>, off: u64, size: u64) {
    let i = free.partition_point(|&(o, _)| o < off);
    let merge_prev = i > 0 && free[i - 1].0 + free[i - 1].1 == off;
    let merge_next = i < free.len() && off + size == free[i].0;
    match (merge_prev, merge_next) {
        (true, true) => {
            free[i - 1].1 += size + free[i].1;
            free.remove(i);
        }
        (true, false) => free[i - 1].1 += size,
        (false, true) => free[i] = (off, size + free[i].1),
        (false, false) => free.insert(i, (off, size)),
    }
}
