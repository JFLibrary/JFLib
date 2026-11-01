// WGSL "Constructible Types": zero-value constructors of constructible types
// expect: valid
struct S { a: i32, v: vec3f }
fn f() { let a = vec3f(); let b = S(); let c = array<i32, 3>(); let d = mat2x2f(); _ = a; _ = b; _ = c; _ = d; }
