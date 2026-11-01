// WGSL "Built-in Values": the compute built-ins with their types
// expect: valid
@compute @workgroup_size(4, 2) fn main(@builtin(local_invocation_id) l: vec3u, @builtin(local_invocation_index) li: u32,
    @builtin(global_invocation_id) g: vec3u, @builtin(workgroup_id) w: vec3u, @builtin(num_workgroups) n: vec3u) {}
