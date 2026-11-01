// WGSL "Subgroups": every subgroup and quad function in a compute shader
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a1 = subgroupAdd(1u) + subgroupExclusiveAdd(1u) + subgroupInclusiveAdd(1u) + subgroupMul(1u) + subgroupExclusiveMul(1u) + subgroupInclusiveMul(1u);
  let a2 = subgroupAdd(1.5f) + subgroupMul(vec2f(1.0)).x + subgroupMax(2.5f) + subgroupMin(vec3f(2.5)).y;
  let a3 = subgroupAdd(vec2i(1)).x + subgroupMax(1i) + subgroupMin(vec4i(1)).w;
  let a4 = (subgroupAnd(1u) | subgroupOr(vec2u(1u)).x) ^ subgroupXor(vec3u(1u)).z;
  let a5 = subgroupAll(true) && subgroupAny(false) && subgroupElect();
  let a6 = subgroupBallot(true).x + subgroupBroadcast(1u, 3u) + subgroupBroadcast(vec2u(1u), 3i).x + subgroupBroadcastFirst(1u);
  let a7 = subgroupShuffle(1u, 2u) + subgroupShuffle(1u, 2i) + subgroupShuffleDown(1u, 1u) + subgroupShuffleUp(1u, 1u) + subgroupShuffleXor(1u, 1u);
  let a8 = quadBroadcast(1u, 0u) + quadBroadcast(vec2u(1u), 3i).x + quadSwapDiagonal(1u) + quadSwapX(1u) + quadSwapY(vec4u(1u)).w;
}
