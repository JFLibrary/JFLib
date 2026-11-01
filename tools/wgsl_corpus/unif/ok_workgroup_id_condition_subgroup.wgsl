// WGSL "Uniformity Analysis": and workgroup_id
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(workgroup_id) w: vec3u) { if w.x == 0u { let c = subgroupAdd(1u); } }
