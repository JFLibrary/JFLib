// WGSL "Uniformity Analysis": storageBarrier also needs uniform control flow
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { storageBarrier(); } }
