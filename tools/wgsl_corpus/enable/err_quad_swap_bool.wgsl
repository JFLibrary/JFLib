// WGSL "Subgroups": quadSwapX not on bool
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = quadSwapX(true);
}
