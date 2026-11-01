// WGSL "Uniformity Analysis": textureDimensions of a read_write storage texture is uniform
// expect: valid
// uniformity
@group(0) @binding(0) var tt: texture_storage_2d<r32uint, read_write>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if textureDimensions(tt).x == 0u { workgroupBarrier(); }
}
