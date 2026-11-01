// WGSL "Language Extensions": requires unrestricted_pointer_parameters: a workgroup pointer parameter
// expect: valid
requires unrestricted_pointer_parameters;
var<workgroup> w: array<u32, 64>;
fn g(p: ptr<workgroup, array<u32, 64>>, i: u32) { (*p)[i] = i; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(&w, i); }
