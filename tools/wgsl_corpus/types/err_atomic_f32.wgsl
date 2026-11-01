// WGSL "Atomic Types": atomic is only for i32 and u32
// expect: error
var<workgroup> a: atomic<f32>;
