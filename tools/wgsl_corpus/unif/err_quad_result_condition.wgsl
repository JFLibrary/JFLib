// WGSL "Uniformity Analysis": nor of a quad function
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if quadSwapX(1u) == 0u { let a = subgroupAdd(1u); }
}
