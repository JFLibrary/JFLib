// WGSL "Uniformity Analysis": workgroup_id is uniform within the workgroup
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(workgroup_id) w: vec3u) { if w.x == 0u { workgroupBarrier(); } }
