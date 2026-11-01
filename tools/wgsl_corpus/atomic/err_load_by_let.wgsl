// WGSL "Atomic Built-in Functions": an atomic cannot be loaded by a plain read
// expect: error
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { let x = a; }
