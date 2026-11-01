// WGSL "Uniformity Analysis": textureLoad of a read-only storage texture is uniform
// expect: valid
// uniformity
@group(0) @binding(0) var tt: texture_storage_2d<r32uint, read>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if textureLoad(tt, vec2u()).x == 0u { workgroupBarrier(); }
}
