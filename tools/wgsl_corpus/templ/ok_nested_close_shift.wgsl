// WGSL "Template Lists": >> closes two nested template lists
// expect: valid
@group(0) @binding(0) var<storage> b: array<array<i32, 2>>;
alias M = array<vec2<f32>>;
