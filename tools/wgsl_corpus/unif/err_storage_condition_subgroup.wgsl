// WGSL "Uniformity Analysis": nor a read_write buffer
// expect: error
// uniformity
enable subgroups;
@group(0) @binding(0) var<storage, read_write> b: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if b == 0u { let c = subgroupAdd(1u); }
}
