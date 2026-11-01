// WGSL "Atomic Built-in Functions": an atomic is no value: a + 1u
// expect: error
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { let x = a + 1u; }
