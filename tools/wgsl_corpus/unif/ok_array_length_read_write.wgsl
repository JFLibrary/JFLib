// WGSL "Uniformity Analysis": arrayLength depends on the pointer, not on the contents
// expect: valid
// uniformity
@group(0) @binding(0) var<storage, read_write> rw: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if arrayLength(&rw) > 4u { workgroupBarrier(); }
}
