// WGSL "Language Extensions": requires linear_indexing: global_invocation_index and workgroup_index built-ins
// expect: valid
requires linear_indexing;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_index) g: u32, @builtin(workgroup_index) w: u32) { o[g] = w; }
