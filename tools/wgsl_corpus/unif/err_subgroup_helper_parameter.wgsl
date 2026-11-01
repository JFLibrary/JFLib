// WGSL "Uniformity Analysis": a parameter deciding a subgroup function
// expect: error
// uniformity
enable subgroups;
fn f(c: u32) -> u32 { if c == 0u { return subgroupAdd(1u); } return 0u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = f(i);
}
