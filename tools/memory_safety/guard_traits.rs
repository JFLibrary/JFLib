// Compile-only witness: a guard over Cell must not satisfy Sync.
#![no_std]
use core::cell::Cell;
fn requires_sync<T: Sync>() {}
pub fn witness() { requires_sync::<rt::sync::Guard<'static, Cell<u32>>>(); }
