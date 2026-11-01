// WGSL "Uniformity Analysis": nor is a read-modify-write result
// expect: error
// uniformity
@group(0) @binding(0) var<storage, read_write> a: atomic<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if atomicAdd(&a, 1u) > 4u { workgroupBarrier(); }
}
