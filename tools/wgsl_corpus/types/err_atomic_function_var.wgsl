// WGSL "Atomic Types": atomics live in storage or workgroup memory, not function memory
// expect: error
fn f() { var a: atomic<u32>; }
