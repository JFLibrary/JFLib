// WGSL "Uniformity Analysis": pointer contents at subgroup scope
// expect: valid
// uniformity
enable subgroups;
fn g(p: ptr<function, u32>) { if *p == 0u { let a = subgroupAdd(1u); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = subgroupBroadcastFirst(1u);
  g(&x);
}
