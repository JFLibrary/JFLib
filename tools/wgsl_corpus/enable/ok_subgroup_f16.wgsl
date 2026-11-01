// WGSL "Subgroups": with enable f16 on f16 values
// expect: valid
enable f16;
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAdd(1h) + subgroupMax(vec2h(1h)).x + subgroupBroadcast(1h, 1u) + subgroupShuffle(1h, 1u) + quadSwapX(1h);
}
