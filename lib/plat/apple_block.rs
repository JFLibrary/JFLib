// apple_block -- Objective-C blocks from Rust (macOS and iOS): global blocks without captured state
// that the system calls back, and calling a block the system hands over (the notification centre's
// completion handlers).
//
//   global(slot, invoke) -> block   a block literal of the runtime's global class, made once per slot
//                                   and kept for the process; invoke's first argument is the block
//   call(block, arg)                void (^)(void) or, with Some(x), void (^)(NSUInteger)

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

unsafe extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

#[repr(C)]
struct BlockDesc {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    desc: *const BlockDesc,
}

static BLOCK_DESC: BlockDesc = BlockDesc { reserved: 0, size: core::mem::size_of::<Block>() };

/// a global block calling `invoke` (made once for `slot`, kept for the process)
pub(crate) fn global(slot: &AtomicPtr<c_void>, invoke: *const c_void) -> *const c_void {
    let b = slot.load(Ordering::SeqCst);
    if !b.is_null() {
        return b;
    }
    // (the address of the runtime's global block class: taking it needs no unsafe)
    let isa = core::ptr::addr_of!(_NSConcreteGlobalBlock);
    let blk = alloc::boxed::Box::new(Block { isa, flags: 1 << 28, reserved: 0, invoke, desc: &BLOCK_DESC }); // BLOCK_IS_GLOBAL
    let p = alloc::boxed::Box::into_raw(blk) as *mut c_void;
    slot.store(p, Ordering::SeqCst);
    p
}

/// call a block the system handed us: void (^)(void) or, with Some(x), void (^)(NSUInteger)
///
/// SAFETY: b is null or a live block of that signature.
pub(crate) unsafe fn call(b: *const c_void, arg: Option<usize>) {
    if b.is_null() {
        return;
    }
    // SAFETY: (the caller's block) its invoke pointer at offset 16 of the block literal.
    unsafe {
        let invoke = *(b as *const u8).add(16).cast::<*const c_void>();
        match arg {
            None => {
                let f: extern "C" fn(*const c_void) = core::mem::transmute(invoke);
                f(b);
            }
            Some(x) => {
                let f: extern "C" fn(*const c_void, usize) = core::mem::transmute(invoke);
                f(b, x);
            }
        }
    }
}
