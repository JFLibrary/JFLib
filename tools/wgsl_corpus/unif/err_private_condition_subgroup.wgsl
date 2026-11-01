// WGSL "Uniformity Analysis": a private variable
// expect: error
// uniformity
enable subgroups;
var<private> pv: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if pv == 0u { let a = subgroupAdd(1u); }
}
