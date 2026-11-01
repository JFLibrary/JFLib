// WGSL "Limits": nested structs: depth 15
// expect: valid
struct S0 { a: u32 }
struct S1 { a: S0 }
struct S2 { a: S1 }
struct S3 { a: S2 }
struct S4 { a: S3 }
struct S5 { a: S4 }
struct S6 { a: S5 }
struct S7 { a: S6 }
struct S8 { a: S7 }
struct S9 { a: S8 }
struct S10 { a: S9 }
struct S11 { a: S10 }
struct S12 { a: S11 }
struct S13 { a: S12 }
struct S14 { a: S13 }
var<private> v: S14;
