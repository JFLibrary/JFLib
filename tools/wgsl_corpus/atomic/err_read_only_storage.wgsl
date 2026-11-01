// WGSL "Atomic Built-in Functions": atomics in a read-only storage buffer
// expect: error
@group(0) @binding(0) var<storage, read> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { _ = atomicLoad(&a); }
