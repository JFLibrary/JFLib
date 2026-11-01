// WGSL "Subgroups": also as a struct member
// expect: valid
enable subgroups;
struct In { @builtin(subgroup_size) a: u32 }
@compute @workgroup_size(64) fn main(x: In) { }
