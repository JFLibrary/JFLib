// WGSL "Atomic Built-in Functions": no atomics in uniform buffers
// expect: error
@group(0) @binding(0) var<uniform> a: atomic<u32>;
