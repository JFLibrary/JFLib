// WGSL "Atomic Built-in Functions": atomicCompareExchangeWeak returns old_value and exchanged
// expect: valid
@group(0) @binding(0) var<storage, read_write> a: atomic<u32>;
@group(0) @binding(1) var<storage, read_write> o: u32;
@compute @workgroup_size(1) fn main() { let r = atomicCompareExchangeWeak(&a, 0u, 1u); if r.exchanged { o = r.old_value; } }
