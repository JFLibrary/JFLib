// WGSL "Uniformity Analysis": workgroupBarrier under a condition on local_invocation_id
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_id) l: vec3u) { if l.x == 0u { workgroupBarrier(); } }
