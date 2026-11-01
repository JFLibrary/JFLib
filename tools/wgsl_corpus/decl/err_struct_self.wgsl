// WGSL "Declaration and Scope": a struct must not contain itself
// expect: error
struct S { a: i32, s: S }
