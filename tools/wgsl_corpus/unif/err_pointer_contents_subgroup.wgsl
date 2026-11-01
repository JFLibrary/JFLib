// WGSL "Uniformity Analysis": non-uniform contents
// expect: error
// uniformity
enable subgroups;
fn g(p: ptr<function, u32>) { if *p == 0u { let a = subgroupAdd(1u); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  g(&x);
}
