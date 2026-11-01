// WGSL "Atomic Built-in Functions": atomics inside a struct and an array
// expect: valid
struct S { n: atomic<u32>, m: array<atomic<i32>, 4> }
@group(0) @binding(0) var<storage, read_write> s: S;
var<workgroup> w: array<atomic<u32>, 64>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { atomicAdd(&w[i], 1u); atomicAdd(&s.n, atomicLoad(&w[i])); atomicMax(&s.m[i % 4u], 1); }
