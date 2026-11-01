// WGSL "Subgroups": abstract arguments become i32 and f32
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAdd(1);
  let b: i32 = a;
  let c = subgroupAdd(1.0);
  let d: f32 = c;
  let e = subgroupBroadcastFirst(vec2(1, 2));
  let f: vec2i = e;
}
