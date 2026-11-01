// WGSL "Memory Layout": a uniform buffer above maxUniformBufferBindingSize (65536 bytes): checked when a bind group is made, not at pipeline creation
// expect: valid
@group(0) @binding(0) var<uniform> u: array<vec4f, 4097>;
@compute @workgroup_size(1) fn main() { _ = u[0]; }
