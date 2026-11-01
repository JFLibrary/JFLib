// WGSL "Subgroups": of quadSwapX too
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  quadSwapX(1u);
}
